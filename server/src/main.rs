//! Сервер ЛІЇ: ЛІЯ ID (сумісний з Yggdrasil для authlib-injector), скіни, друзі, статуси,
//! опубліковані збірки і ретранслятор LAN-світів.
mod account;
mod api;
mod db;
mod mail;
mod relay;
mod ygg;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub struct Config {
    /// Зовнішня адреса API, напр. https://lia.starnure.dev
    pub public_url: String,
    /// Домен ретранслятора: <код>.<play_domain>:<relay_port>
    pub play_domain: String,
    pub relay_port: u16,
    pub data: PathBuf,
    /// Старіших лаунчерів (з відомими вадами) сервер не пускає і просить оновитися.
    pub min_launcher: [u32; 3],
}

pub struct App {
    pub cfg: Config,
    // ponytail: одне з'єднання SQLite під м'ютексом; пул — коли навантаження це покаже.
    pub db: Mutex<rusqlite::Connection>,
    pub key: rsa::RsaPrivateKey,
    pub public_pem: String,
    /// serverId → (uuid, нік, коли) для join/hasJoined.
    pub joins: Mutex<HashMap<String, (String, String, Instant)>>,
    /// serverId перевірки ліцензії → (користувач, коли).
    pub challenges: Mutex<HashMap<String, (i64, Instant)>>,
    pub http: reqwest::Client,
    /// Відкриті WebSocket-и гравців: id користувача → канали.
    pub sockets: Mutex<HashMap<i64, Vec<mpsc::UnboundedSender<String>>>>,
    pub presence: Mutex<HashMap<i64, Value>>,
    pub limits: Mutex<HashMap<String, (u32, Instant)>>,
    pub relay: relay::Hub,
}

pub type S = axum::extract::State<Arc<App>>;

/// Помилка у форматі Yggdrasil — його ж читає й лаунчер.
#[derive(Debug)]
pub struct ApiErr(pub StatusCode, pub &'static str, pub String);

/// «1.1.3», «1.1.3 Java/21» → [1, 1, 3].
pub fn parse_version(v: &str) -> Option<[u32; 3]> {
    let mut it = v.split(|c: char| !c.is_ascii_digit() && c != '.').next()?.split('.').map(|p| p.parse().ok());
    Some([it.next()??, it.next()??, it.next()??])
}

pub fn outdated() -> ApiErr {
    ApiErr(StatusCode::UPGRADE_REQUIRED, "Outdated", "Ця версія ЛІЇ застаріла, оновіть лаунчер: Налаштування → «Перевірити оновлення».".into())
}

/// Лаунчер і запущена ним гра називають версію в User-Agent: «LIA-Launcher/1.1.3» (Java дописує « Java/21»).
async fn gate(axum::extract::State(app): S, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let ua = req.headers().get(axum::http::header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    if ua.strip_prefix("LIA-Launcher/").and_then(parse_version).is_some_and(|v| v < app.cfg.min_launcher) {
        return outdated().into_response();
    }
    next.run(req).await
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1, "errorMessage": self.2 }))).into_response()
    }
}

pub type R<T> = std::result::Result<T, ApiErr>;

pub fn bad(msg: impl Into<String>) -> ApiErr {
    ApiErr(StatusCode::BAD_REQUEST, "IllegalArgumentException", msg.into())
}
pub fn forbidden(msg: impl Into<String>) -> ApiErr {
    ApiErr(StatusCode::FORBIDDEN, "ForbiddenOperationException", msg.into())
}
pub fn not_found() -> ApiErr {
    ApiErr(StatusCode::NOT_FOUND, "NotFound", "не знайдено".into())
}
pub fn internal(e: impl std::fmt::Display) -> ApiErr {
    eprintln!("помилка: {e}");
    ApiErr(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", "внутрішня помилка сервера".into())
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// IP клієнта: за Cloudflare Tunnel справжня адреса — у CF-Connecting-IP.
/// Для IPv6 беремо мережу /64: вона ціла в кожного абонента, тож зміною адреси ліміти не обійти.
pub fn client_ip(h: &HeaderMap) -> String {
    let ip = h.get("cf-connecting-ip").and_then(|v| v.to_str().ok()).unwrap_or("local");
    match ip.parse() {
        Ok(std::net::IpAddr::V6(v6)) if v6.to_ipv4_mapped().is_none() => format!("{}/64", std::net::Ipv6Addr::from_bits(v6.to_bits() & (u128::MAX << 64))),
        _ => ip.to_string(),
    }
}

impl App {
    /// Не більше `max` спроб за `window` на ключ (IP, IP+нік тощо).
    pub fn limit(&self, key: String, max: u32, window: Duration) -> R<()> {
        let mut m = self.limits.lock().unwrap();
        let e = m.entry(key).or_insert((0, Instant::now()));
        if e.1.elapsed() > window {
            *e = (0, Instant::now());
        }
        e.0 += 1;
        if e.0 > max {
            return Err(ApiErr(StatusCode::TOO_MANY_REQUESTS, "TooManyRequests", "забагато спроб, зачекайте кілька хвилин".into()));
        }
        Ok(())
    }
}

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

#[tokio::main]
async fn main() {
    let cfg = Config {
        public_url: env("LIA_PUBLIC_URL", "http://127.0.0.1:8080").trim_end_matches('/').to_string(),
        play_domain: env("LIA_PLAY_DOMAIN", "play.localhost"),
        relay_port: env("LIA_RELAY_PORT", "25570").parse().expect("LIA_RELAY_PORT"),
        data: PathBuf::from(env("LIA_DATA", "data")),
        min_launcher: parse_version(&env("LIA_MIN_LAUNCHER", "1.1.4")).expect("LIA_MIN_LAUNCHER"),
    };
    std::fs::create_dir_all(cfg.data.join("packs")).expect("тека даних");
    let (key, public_pem) = ygg::load_key(&cfg.data.join("signing.pem"));
    let conn = db::open(&cfg.data.join("lia.db"));
    let relay_port = cfg.relay_port;
    let http = env("LIA_HTTP", "0.0.0.0:8080");
    let app = Arc::new(App {
        cfg,
        db: Mutex::new(conn),
        key,
        public_pem,
        joins: Mutex::new(HashMap::new()),
        challenges: Mutex::new(HashMap::new()),
        http: reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("lia-server").build().expect("http"),
        sockets: Mutex::new(HashMap::new()),
        presence: Mutex::new(HashMap::new()),
        limits: Mutex::new(HashMap::new()),
        relay: relay::Hub::default(),
    });

    // Прибирання застарілих join-ів і лічильників спроб.
    let a = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            a.joins.lock().unwrap().retain(|_, v| v.2.elapsed() < Duration::from_secs(60));
            a.challenges.lock().unwrap().retain(|_, v| v.1.elapsed() < Duration::from_secs(300));
            a.limits.lock().unwrap().retain(|_, v| v.1.elapsed() < Duration::from_secs(3600));
        }
    });

    let a = app.clone();
    tokio::spawn(async move { relay::listen(a, relay_port).await });

    let ygg = Router::new()
        .route("/", get(ygg::meta))
        .route("/authserver/validate", post(ygg::validate))
        .route("/authserver/invalidate", post(ygg::invalidate))
        .route("/sessionserver/session/minecraft/join", post(ygg::join))
        .route("/sessionserver/session/minecraft/hasJoined", get(ygg::has_joined))
        .route("/sessionserver/session/minecraft/profile/{uuid}", get(ygg::profile))
        .route("/api/profiles/minecraft", post(ygg::profiles_by_names));

    let router = Router::new()
        .nest("/yggdrasil", ygg)
        // authlib-injector звертається до кореня API і з косою рискою в кінці, і без неї.
        .route("/yggdrasil/", get(ygg::meta))
        .route("/api/register", post(account::register))
        .route("/api/login", post(account::login))
        .route("/api/logout", post(account::logout))
        .route("/api/email/verify", post(account::verify_email))
        .route("/api/email/resend", post(account::resend_code))
        .route("/api/email/change", post(account::change_email))
        .route("/api/password/forgot", post(account::forgot))
        .route("/api/password/reset", post(account::reset))
        .route("/api/sessions", get(account::sessions))
        .route("/api/sessions/others", delete(account::revoke_others))
        .route("/api/sessions/{id}", delete(account::revoke_session))
        .route("/api/game/challenge", post(ygg::game_challenge))
        .route("/api/game/session", post(ygg::game_session))
        .route("/api/me", get(account::me).delete(account::delete_account))
        .route("/api/me/password", post(account::change_password))
        .route("/api/users", get(api::search_users))
        .route("/api/users/{name}", get(api::user))
        .route("/api/friends", get(api::friends))
        .route("/api/friends/{name}", post(api::friend_add).delete(api::friend_remove))
        // Великі тіла — лише для завантаження збірок; решта запитів — з типовим лімітом axum (2 МБ).
        .route("/api/packs", get(api::packs).post(api::pack_publish).layer(DefaultBodyLimit::max(api::PACK_MAX)))
        .route("/api/packs/subscribed", get(api::packs_subscribed))
        .route("/api/packs/{id}", put(api::pack_update).layer(DefaultBodyLimit::max(api::PACK_MAX)).delete(api::pack_delete).get(api::pack_get))
        .route("/api/packs/{id}/file", get(api::pack_file))
        .route("/api/packs/{id}/subscribe", post(api::pack_subscribe).delete(api::pack_unsubscribe))
        .route("/api/feed", get(api::feed))
        .route("/api/ws", get(api::ws))
        .route("/health", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(app.clone(), gate))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(&http).await.expect("HTTP-порт");
    println!("ЛІЯ: HTTP {http}, ретранслятор :{relay_port}");
    axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert_eq!(super::parse_version("1.1.3"), Some([1, 1, 3]));
        assert_eq!(super::parse_version("1.10.0 Java/21.0.7"), Some([1, 10, 0]));
        assert!(super::parse_version("1.1.2").unwrap() < [1, 1, 3] && super::parse_version("1.2.0").unwrap() > [1, 1, 3]);
        for bad in ["e2e", "", "1.1", "x1.1.3"] {
            assert_eq!(super::parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn client_ip_v6_is_a_network() {
        let ip = |v: &str| {
            let mut h = axum::http::HeaderMap::new();
            h.insert("cf-connecting-ip", v.parse().unwrap());
            super::client_ip(&h)
        };
        assert_eq!(ip("2001:db8:1:2:aaaa::1"), ip("2001:db8:1:2:bbbb::2"));
        assert_eq!(ip("2001:db8:1:2:aaaa::1"), "2001:db8:1:2::/64");
        assert_ne!(ip("2001:db8:1:3::1"), ip("2001:db8:1:2::1"));
        assert_eq!(ip("203.0.113.7"), "203.0.113.7");
        assert_eq!(ip("::ffff:203.0.113.7"), "::ffff:203.0.113.7");
        assert_eq!(super::client_ip(&axum::http::HeaderMap::new()), "local");
    }
}
