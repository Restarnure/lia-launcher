//! ЛІЯ ID і соцмережа. ЛІЯ ID — акаунт лаунчера (друзі, збірки, скін для гравців ЛІЇ), окремо від
//! ігрових акаунтів. Гра входить через authlib-injector ігровим профілем: офлайн-ніком (завжди, коли
//! є ЛІЯ ID) або ліцензією (кнопка «Грати з друзями»). Тут же ретранслятор LAN-світів з боку хоста.
//! Збірки живуть у packs.rs.
use crate::accounts::Account;
use crate::crypto;
use crate::ctx::Ctx;
use crate::error::{err, Context, Result};
use crate::instances::Instance;
use crate::net::{self, Dl};
use crate::paths::Paths;
use crate::util;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest as _;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Сервер ЛІЇ. Для розробки — змінна LIA_API (напр. http://127.0.0.1:18080).
pub fn api() -> String {
    std::env::var("LIA_API").unwrap_or_else(|_| "https://lia.starnure.dev".into())
}

/// Ретранслятор: будь-яке ім'я під wildcard-доменом веде на нього.
fn relay() -> String {
    std::env::var("LIA_RELAY").unwrap_or_else(|_| "relay.play.lia.starnure.dev:25570".into())
}

/// Токени акаунтів ЛІЯ ID з 1.1.0, коли ЛІЯ ID був ігровим акаунтом. Лише для переносу в сесію.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LiaTokens {
    pub access_token: String,
    #[serde(default)]
    pub client_token: String,
}

/// Вхід у ЛІЯ ID на цьому ПК. Зберігається зашифрованим, як і акаунти.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub name: String,
    pub token: String,
}

fn session_path() -> PathBuf {
    Paths::config_home().join("lia.dat")
}

pub fn session() -> Option<Session> {
    let b = std::fs::read(session_path()).ok()?;
    serde_json::from_slice(&crypto::open(&b).ok()?).ok()
}

pub fn set_session(s: Option<&Session>) -> Result<()> {
    match s {
        Some(s) => util::write_atomic(&session_path(), &crypto::seal(&serde_json::to_vec(s)?)?),
        None => {
            let _ = std::fs::remove_file(session_path());
            Ok(())
        }
    }
}

/// 1.1.0 → 1.1.1: акаунти «ЛІЯ ID» прибираються з ігрових, їхній токен стає сесією лаунчера.
/// `true` — список акаунтів змінився.
pub fn migrate(accounts: &mut Vec<Account>) -> bool {
    let Some(i) = accounts.iter().position(|a| a.kind == "lia") else { return false };
    if session().is_none() {
        if let Some(t) = &accounts[i].lia {
            let _ = set_session(Some(&Session { name: accounts[i].name.clone(), token: t.access_token.clone() }));
        }
    }
    accounts.retain(|a| a.kind != "lia");
    true
}

/// Запит до API ЛІЇ. Помилки сервера ({errorMessage}) — людським текстом.
pub async fn call(http: &reqwest::Client, method: reqwest::Method, path: &str, token: Option<&str>, body: Option<Value>) -> Result<Value> {
    let url = format!("{}{path}", api());
    let mut req = http.request(method, &url);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    parse(req.send().await.ctx(|| url.clone())?).await
}

pub async fn parse(resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if status.is_success() {
        return Ok(if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) });
    }
    let msg = serde_json::from_slice::<Value>(&bytes).ok().and_then(|v| v["errorMessage"].as_str().map(str::to_string)).unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
    Err(match status.as_u16() {
        401 => err("lia_auth", "увійдіть у ЛІЯ ID знову"),
        // Ліміт тіла запиту на сервері чи в Cloudflare. Відповідь тоді не JSON, і «HTTP 413» нічого не пояснює.
        413 => err("lia", "збірка завелика для публікації (до 100 МБ)"),
        // Сервер більше не пускає цю версію лаунчера.
        426 => err("outdated", msg),
        _ => err("lia", msg),
    })
}

/// Вхід або реєстрація ЛІЯ ID. Зберігає сесію викликач (`set_session`).
/// `email` — лише для реєстрації. Сервер бачить назву ПК (для списку пристроїв і листа про новий вхід).
pub async fn login(http: &reqwest::Client, name: &str, password: &str, email: Option<&str>) -> Result<Session> {
    let device = std::env::var("COMPUTERNAME").map(|c| format!("ЛІЯ · {c}")).unwrap_or_else(|_| "ЛІЯ".into());
    let (path, body) = match email {
        Some(e) => ("/api/register", json!({ "name": name.trim(), "email": e.trim(), "password": password, "device": device })),
        None => ("/api/login", json!({ "name": name.trim(), "password": password, "device": device })),
    };
    let v = call(http, reqwest::Method::POST, path, None, Some(body)).await?;
    Ok(Session { name: v["name"].as_str().unwrap_or(name).to_string(), token: v["accessToken"].as_str().unwrap_or("").to_string() })
}

/// Ігровий вхід через ЛІЮ для запуску гри: (токен гри, UUID без дефісів, нік).
/// Офлайн-нік закріплюється за ЛІЯ ID; ліцензію доводимо join-ом на сервері Mojang з serverId від ЛІЇ —
/// токен Mojang нікуди, крім Mojang, не йде.
pub async fn game_session(http: &reqwest::Client, s: &Session, account: &Account) -> Result<(String, String, String)> {
    let body = match (&account.kind[..], &account.ms) {
        ("offline", _) => json!({ "kind": "offline", "name": account.name }),
        ("microsoft", Some(ms)) => {
            let c = call(http, reqwest::Method::POST, "/api/game/challenge", Some(&s.token), None).await?;
            let sid = c["serverId"].as_str().unwrap_or("").to_string();
            let resp = http
                .post("https://sessionserver.mojang.com/session/minecraft/join")
                .json(&json!({ "accessToken": ms.access_token, "selectedProfile": account.uuid, "serverId": sid }))
                .send()
                .await?;
            if !resp.status().is_success() {
                return Err(err("lia", "Mojang не прийняв вхід. Перезайдіть в акаунт Microsoft"));
            }
            json!({ "kind": "mojang", "name": account.name, "serverId": sid })
        }
        _ => return Err(err("invalid", "цей акаунт не може грати через ЛІЮ")),
    };
    let v = call(http, reqwest::Method::POST, "/api/game/session", Some(&s.token), Some(body)).await?;
    let get = |k: &str| v[k].as_str().unwrap_or("").to_string();
    Ok((get("accessToken"), get("uuid"), get("name")))
}

/// Збірки, запущені з входом через ЛІЮ (у їхні LAN-світи можуть зайти й пірати).
fn lia_games() -> &'static Mutex<std::collections::HashSet<String>> {
    static S: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

pub fn set_lia_game(instance: &str, on: bool) {
    let mut s = lia_games().lock().unwrap();
    if on {
        s.insert(instance.to_string());
    } else {
        s.remove(instance);
    }
}

pub fn is_lia_game(instance: &str) -> bool {
    lia_games().lock().unwrap().contains(instance)
}

// ---------------------------------------------------------------- authlib-injector

const INJECTOR_VERSION: &str = "1.2.8";
const INJECTOR_SHA256: &str = "9c7f4343e6c82034958ffb48c14a2cb0c85928be7283103ce17da00c6d5a7b10";

fn sha256_file(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    Some(hex::encode(sha2::Sha256::digest(&bytes)))
}

/// Java-агент, що перенаправляє вхід гри на ЛІЯ ID. Завантажується один раз і перевіряється за SHA-256.
pub async fn injector(ctx: &Ctx) -> Result<PathBuf> {
    let path = ctx.paths.libraries().join("moe/yushi/authlib-injector").join(format!("authlib-injector-{INJECTOR_VERSION}.jar"));
    if sha256_file(&path).as_deref() == Some(INJECTOR_SHA256) {
        return Ok(path);
    }
    let url = format!("https://github.com/yushijinhun/authlib-injector/releases/download/v{INJECTOR_VERSION}/authlib-injector-{INJECTOR_VERSION}.jar");
    net::download(&ctx.http, &Dl::new(url, path.clone(), None, None), None).await?;
    if sha256_file(&path).as_deref() != Some(INJECTOR_SHA256) {
        let _ = std::fs::remove_file(&path);
        return Err(err("integrity", "authlib-injector: контрольна сума не збігається"));
    }
    Ok(path)
}

/// Аргументи JVM для гри з ЛІЯ ID. Свій http.agent — бо Cloudflare відсікає голий «Java/1.8…»,
/// а саме так представляються authlib-injector і сервер гри на Java 8.
pub fn jvm_args(jar: &Path) -> [String; 2] {
    [format!("-javaagent:{}={}/yggdrasil", jar.display(), api()), format!("-Dhttp.agent={}", net::USER_AGENT)]
}

// ---------------------------------------------------------------- ретранслятор (хост)

struct Share {
    code: String,
    address: String,
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

/// Ретранслятор шле ping кожні 20 с. Якщо тиша довша, з'єднання мертве (зник інтернет, ПК спав).
const RELAY_SILENCE: std::time::Duration = std::time::Duration::from_secs(60);

fn shares() -> &'static Mutex<HashMap<String, Share>> {
    static S: OnceLock<Mutex<HashMap<String, Share>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

async fn relay_conn(hello: Value) -> std::io::Result<TcpStream> {
    let mut s = TcpStream::connect(relay()).await?;
    s.set_nodelay(true)?;
    s.write_all(b"\x00LIA").await?;
    s.write_all((hello.to_string() + "\n").as_bytes()).await?;
    Ok(s)
}

/// Відкриває LAN-світ (порт `port` на цьому ПК) друзям через ретранслятор. Повертає (код, адреса).
/// `on_end` викликається, коли з'єднання з ретранслятором обірвалося.
pub async fn share(instance: &str, token: &str, port: u16, on_end: impl FnOnce() + Send + 'static) -> Result<(String, String)> {
    // Уже відкрито: та сама адреса. Якщо світ на іншому порту (перевідкрили) чи з'єднання обірвалось, відкриваємо заново.
    let same = |s: &Share| s.port == port && !s.task.is_finished();
    if let Some(s) = shares().lock().unwrap().get(instance).filter(|s| same(s)) {
        return Ok((s.code.clone(), s.address.clone()));
    }
    // Версія: старіших лаунчерів ретранслятор не пускає.
    let s = relay_conn(json!({ "kind": "host", "token": token, "v": env!("CARGO_PKG_VERSION") })).await.ctx(|| "ретранслятор ЛІЇ".into())?;
    let (r, w) = s.into_split();
    let mut lines = BufReader::new(r).lines();
    let hello: Value = match tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line()).await {
        Ok(Ok(Some(l))) => serde_json::from_str(&l)?,
        _ => return Err(err("lia_relay", "ретранслятор не відповів (можливо, вже відкрито 3 світи)")),
    };
    let code = hello["code"].as_str().unwrap_or("").to_string();
    let address = hello["address"].as_str().unwrap_or("").to_string();
    let mut map = shares().lock().unwrap();
    // Паралельний виклик для того самого світу встиг першим. Лишаємо його, а наше з'єднання закриється.
    if let Some(s) = map.get(instance).filter(|s| same(s)) {
        return Ok((s.code.clone(), s.address.clone()));
    }
    let task = tokio::spawn(async move {
        let _w = w; // тримаємо з'єднання відкритим
        while let Ok(Ok(Some(l))) = tokio::time::timeout(RELAY_SILENCE, lines.next_line()).await {
            let Ok(v) = serde_json::from_str::<Value>(&l) else { continue };
            if let Some(id) = v["open"].as_str() {
                let id = id.to_string();
                tokio::spawn(async move {
                    let (Ok(mut remote), Ok(mut local)) = (relay_conn(json!({ "kind": "data", "id": id })).await, TcpStream::connect(("127.0.0.1", port)).await) else { return };
                    let _ = local.set_nodelay(true);
                    let _ = tokio::io::copy_bidirectional(&mut remote, &mut local).await;
                });
            }
        }
        on_end();
    });
    // Старий світ цієї збірки (інший порт) закриваємо, інакше він займав би місце на ретрансляторі.
    if let Some(old) = map.insert(instance.to_string(), Share { code: code.clone(), address: address.clone(), port, task }) {
        old.task.abort();
    }
    Ok((code, address))
}

pub fn unshare(instance: &str) {
    if let Some(s) = shares().lock().unwrap().remove(instance) {
        s.task.abort();
    }
}

/// Ідентифікатор збірки ЛІЇ, з якої встановлено цю збірку (поле modpack.projectId = "lia:<id>").
pub fn pack_of(inst: &Instance) -> Option<String> {
    inst.modpack.as_ref()?.project_id.strip_prefix("lia:").map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Службове з'єднання лаунчера до підробленого ретранслятора: (привітання, сокет).
    async fn hello(relay: &tokio::net::TcpListener) -> (Value, BufReader<TcpStream>) {
        use tokio::io::AsyncReadExt;
        let (mut s, _) = tokio::time::timeout(std::time::Duration::from_secs(5), relay.accept()).await.expect("лаунчер не підключився").unwrap();
        let mut magic = [0u8; 4];
        s.read_exact(&mut magic).await.unwrap();
        assert_eq!(&magic, b"\0LIA");
        let mut r = BufReader::new(s);
        let mut line = String::new();
        r.read_line(&mut line).await.unwrap();
        (serde_json::from_str(&line).unwrap(), r)
    }

    // Багатопотоковий runtime: фонові задачі ретранслятора працюють, поки тест чекає.
    #[tokio::test(flavor = "multi_thread")]
    async fn relay_share() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::io::AsyncReadExt;
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::env::set_var("LIA_RELAY", relay.local_addr().unwrap().to_string());
        // «LAN-світ» хоста: луна.
        let lan = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = lan.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = lan.accept().await {
                tokio::spawn(async move {
                    let (mut r, mut w) = s.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });
        let r = &relay;
        let open = |code: &'static str| async move {
            let (h, mut ctl) = hello(r).await;
            assert_eq!(h, json!({ "kind": "host", "token": "tok", "v": env!("CARGO_PKG_VERSION") }));
            ctl.get_mut().write_all(format!("{{\"code\":\"{code}\",\"address\":\"{code}.play.test:25570\"}}\n").as_bytes()).await.unwrap();
            ctl
        };

        let first = tokio::spawn(share("relay-test", "tok", port, || {}));
        let mut ctl = open("aaaaaa").await;
        assert_eq!(first.await.unwrap().unwrap().0, "aaaaaa");
        // Той самий світ дає ту саму адресу без нового з'єднання.
        assert_eq!(share("relay-test", "tok", port, || {}).await.unwrap().0, "aaaaaa");
        assert!(tokio::time::timeout(Duration::from_millis(200), relay.accept()).await.is_err());

        // Гравець: ретранслятор просить з'єднання даних, байти йдуть у світ і назад.
        ctl.get_mut().write_all(b"{\"ping\":1}\n{\"open\":\"p1\"}\n").await.unwrap();
        let (h, mut data) = hello(&relay).await;
        assert_eq!(h, json!({ "kind": "data", "id": "p1" }));
        data.get_mut().write_all(b"handshake").await.unwrap();
        let mut buf = [0u8; 9];
        data.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"handshake");

        // Світ перевідкрили на іншому порту: новий код, старе з'єднання закрито.
        let ended = Arc::new(AtomicBool::new(false));
        let e = ended.clone();
        let second = tokio::spawn(share("relay-test", "tok", port + 1, move || e.store(true, Ordering::SeqCst)));
        let ctl2 = open("bbbbbb").await;
        assert_eq!(second.await.unwrap().unwrap().0, "bbbbbb");
        let mut b = [0u8; 64];
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), ctl.read(&mut b)).await.unwrap().unwrap(), 0);

        // Ретранслятор обірвав з'єднання: on_end, і наступний виклик відкриває світ заново, а не віддає мертву адресу.
        drop(ctl2);
        let t0 = std::time::Instant::now();
        // on_end задача викликає наостанок, але завершується трохи пізніше.
        while !ended.load(Ordering::SeqCst) || !shares().lock().unwrap().get("relay-test").is_some_and(|s| s.task.is_finished()) {
            assert!(t0.elapsed() < Duration::from_secs(5), "on_end не викликано");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let third = tokio::spawn(share("relay-test", "tok", port + 1, || {}));
        let _ctl3 = open("cccccc").await;
        assert_eq!(third.await.unwrap().unwrap().0, "cccccc");
        unshare("relay-test");
    }
}
