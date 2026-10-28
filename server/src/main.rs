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
pub fn client_ip(h: &HeaderMap) -> String {
    h.get("cf-connecting-ip").and_then(|v| v.to_str().ok()).unwrap_or("local").to_string()
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
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(&http).await.expect("HTTP-порт");
    println!("ЛІЯ: HTTP {http}, ретранслятор :{relay_port}");
    axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();
}
