//! API ЛІЇ для лаунчера: друзі, статуси, збірки, стрічка. Акаунт — у account.rs.
use crate::ygg::hex;
use crate::{bad, forbidden, internal, not_found, ApiErr, App, R, S};
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::{SinkExt, StreamExt};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Більше однаково не пропустить Cloudflare (100 МБ на тіло запиту в безкоштовному плані).
pub const PACK_MAX: usize = 100 * 1000 * 1000;
const PACK_QUOTA: i64 = 1024 * 1024 * 1024;

// ------------------------------------------------------------ користувачі й токени

#[derive(Clone)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub uuid: String,
    pub bio: String,
    pub email: Option<String>,
    /// Пошту підтверджено (або пошта на сервері вимкнена — тоді підтверджувати нема чим).
    pub verified: bool,
}

const USER_COLS: &str = "id, name, uuid, bio, email, email_verified";

fn row_user(r: &rusqlite::Row) -> rusqlite::Result<User> {
    Ok(User { id: r.get(0)?, name: r.get(1)?, uuid: r.get(2)?, bio: r.get(3)?, email: r.get(4)?, verified: r.get::<_, i64>(5)? != 0 || !crate::mail::enabled() })
}

pub fn user_by(app: &App, col: &str, val: &str) -> Option<User> {
    let col = match col {
        "name" => "name",
        "uuid" => "uuid",
        _ => return None,
    };
    app.db.lock().unwrap().query_row(&format!("SELECT {USER_COLS} FROM users WHERE {col} = ?1"), [val], row_user).ok()
}

pub fn user_by_id(app: &App, id: i64) -> Option<User> {
    app.db.lock().unwrap().query_row(&format!("SELECT {USER_COLS} FROM users WHERE id = ?1"), [id], row_user).ok()
}

pub fn token_hash(token: &str) -> String {
    hex(&Sha1::digest(token.as_bytes()))
}

/// Акаунт ЛІЯ ID за токеном лаунчера. Ігрові токени (передаються в гру й видні в процесах) сюди не підходять.
/// Сесія, якою не користувались 90 днів, недійсна.
pub fn token_user(app: &App, token: &str) -> Option<User> {
    let db = app.db.lock().unwrap();
    let hash = token_hash(token);
    let (id, created, used): (i64, i64, Option<i64>) = db.query_row("SELECT user, created, last_used FROM tokens WHERE hash = ?1 AND profile IS NULL", [&hash], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok()?;
    let now = crate::now();
    let used = used.unwrap_or(created);
    if now - used > crate::account::SESSION_IDLE {
        let _ = db.execute("DELETE FROM tokens WHERE hash = ?1", [&hash]);
        return None;
    }
    if now - used > 3600_000 {
        let _ = db.execute("UPDATE tokens SET last_used = ?1 WHERE hash = ?2", params![now, hash]);
    }
    db.query_row(&format!("SELECT {USER_COLS} FROM users WHERE id = ?1"), [id], row_user).ok()
}

fn unauthorized() -> ApiErr {
    ApiErr(StatusCode::UNAUTHORIZED, "Unauthorized", "увійдіть у ЛІЯ ID знову".into())
}

/// Будь-який вхід, навіть без підтвердженої пошти (для сторінки акаунта).
pub fn auth_any(app: &App, h: &HeaderMap) -> R<User> {
    token_user(app, crate::account::token_from(h)).ok_or_else(unauthorized)
}

/// Вхід з підтвердженою поштою — для друзів, збірок, світів і гри.
pub fn auth(app: &App, h: &HeaderMap) -> R<User> {
    let u = auth_any(app, h)?;
    if !u.verified {
        return Err(ApiErr(StatusCode::FORBIDDEN, "EmailNotVerified", "спершу підтвердіть пошту ЛІЯ ID".into()));
    }
    Ok(u)
}

pub fn check_password(app: &App, name: &str, pass: &str) -> R<User> {
    let stored: Option<String> = app.db.lock().unwrap().query_row("SELECT pass FROM users WHERE name = ?1", [name], |r| r.get(0)).optional().map_err(internal)?;
    match stored {
        Some(s) if crate::account::verify_password(pass, &s) => user_by(app, "name", name).ok_or_else(not_found),
        _ => Err(forbidden("Неправильний пароль.")),
    }
}

fn public(_app: &App, u: &User) -> Value {
    json!({ "name": u.name, "uuid": u.uuid, "bio": u.bio })
}

// ------------------------------------------------------------ друзі

#[derive(PartialEq)]
enum Rel {
    None,
    Friends,
    Outgoing,
    Incoming,
}

fn relation(app: &App, me: i64, other: i64) -> Rel {
    let db = app.db.lock().unwrap();
    let get = |a: i64, b: i64| db.query_row("SELECT accepted FROM friends WHERE a = ?1 AND b = ?2", [a, b], |r| r.get::<_, i64>(0)).optional().ok().flatten();
    // accepted: 1 — друзі, 0 — запит, -1 — запит відхилено (той, хто просив, про це не дізнається
    // і не може засипати повторними запитами).
    match (get(me, other), get(other, me)) {
        (Some(1), _) | (_, Some(1)) => Rel::Friends,
        (Some(_), _) => Rel::Outgoing,
        (_, Some(0)) => Rel::Incoming,
        _ => Rel::None,
    }
}

fn friend_ids(app: &App, me: i64) -> Vec<i64> {
    let db = app.db.lock().unwrap();
    let mut q = db.prepare("SELECT CASE WHEN a = ?1 THEN b ELSE a END FROM friends WHERE (a = ?1 OR b = ?1) AND accepted = 1").unwrap();
    q.query_map([me], |r| r.get(0)).unwrap().flatten().collect()
}

/// Повідомлення в усі відкриті WebSocket-и користувача.
fn notify(app: &App, user: i64, msg: &Value) {
    if let Some(list) = app.sockets.lock().unwrap().get_mut(&user) {
        let s = msg.to_string();
        list.retain(|tx| tx.send(s.clone()).is_ok());
    }
}

fn event(app: &App, user: i64, kind: &str, pack: Option<&str>, data: Value) {
    let _ = app.db.lock().unwrap().execute("INSERT INTO events (user, kind, pack, data, created) VALUES (?1, ?2, ?3, ?4, ?5)", params![user, kind, pack, data.to_string(), crate::now()]);
}

fn friend_view(app: &App, u: &User) -> Value {
    let mut v = public(app, u);
    v["presence"] = app.presence.lock().unwrap().get(&u.id).cloned().unwrap_or(Value::Null);
    v
}

pub async fn friends(st: S, h: HeaderMap) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    let rows: Vec<(i64, i64, i64)> = {
        let db = st.db.lock().unwrap();
        let mut q = db.prepare("SELECT a, b, accepted FROM friends WHERE a = ?1 OR b = ?1").map_err(internal)?;
        let v = q.query_map([me.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(internal)?.flatten().collect();
        v
    };
    let (mut friends, mut incoming, mut outgoing) = (vec![], vec![], vec![]);
    for (a, b, acc) in rows {
        let other = if a == me.id { b } else { a };
        let Some(u) = user_by_id(&st, other) else { continue };
        match (acc, a == me.id) {
            (1, _) => friends.push(friend_view(&st, &u)),
            (_, true) => outgoing.push(public(&st, &u)),
            (0, false) => incoming.push(public(&st, &u)),
            _ => {} // запит, який я відхилив
        }
    }
    Ok(Json(json!({ "friends": friends, "incoming": incoming, "outgoing": outgoing })))
}

/// Запит у друзі; якщо зустрічний запит уже є — це прийняття.
pub async fn friend_add(st: S, h: HeaderMap, Path(name): Path<String>) -> R<StatusCode> {
    let me = auth(&st, &h)?;
    let other = user_by(&st, "name", &name).ok_or_else(not_found)?;
    if other.id == me.id {
        return Err(bad("це ви"));
    }
    match relation(&st, me.id, other.id) {
        Rel::Friends | Rel::Outgoing => {}
        Rel::Incoming => {
            st.db.lock().unwrap().execute("UPDATE friends SET accepted = 1 WHERE a = ?1 AND b = ?2", [other.id, me.id]).map_err(internal)?;
            befriended(&st, &me, &other);
        }
        Rel::None => {
            // Раніше відхилений запит від цього гравця — тепер я сам хочу дружити: просто приймаємо.
            let revived = st.db.lock().unwrap().execute("UPDATE friends SET accepted = 1 WHERE a = ?1 AND b = ?2 AND accepted = -1", [other.id, me.id]).map_err(internal)?;
            if revived > 0 {
                befriended(&st, &me, &other);
                return Ok(StatusCode::NO_CONTENT);
            }
            st.limit(format!("friend:{}", me.id), 30, Duration::from_secs(3600))?;
            st.db.lock().unwrap().execute("INSERT INTO friends (a, b, accepted, created) VALUES (?1, ?2, 0, ?3)", params![me.id, other.id, crate::now()]).map_err(internal)?;
            notify(&st, other.id, &json!({ "type": "friends", "request": me.name }));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

fn befriended(st: &App, me: &User, other: &User) {
    event(st, me.id, "friend", None, json!({ "a": me.name, "b": other.name }));
    notify(st, other.id, &json!({ "type": "friends" }));
    // Тепер видно статуси одне одного.
    for (x, y) in [(me, other), (other, me)] {
        if let Some(p) = st.presence.lock().unwrap().get(&x.id).cloned() {
            notify(st, y.id, &json!({ "type": "presence", "name": x.name, "presence": p }));
        }
    }
}

pub async fn friend_remove(st: S, h: HeaderMap, Path(name): Path<String>) -> R<StatusCode> {
    let me = auth(&st, &h)?;
    let other = user_by(&st, "name", &name).ok_or_else(not_found)?;
    if relation(&st, me.id, other.id) == Rel::Incoming {
        // Відхилити запит: лишаємо позначку, щоб той самий гравець не надсилав його знову.
        st.db.lock().unwrap().execute("UPDATE friends SET accepted = -1 WHERE a = ?1 AND b = ?2", [other.id, me.id]).map_err(internal)?;
        return Ok(StatusCode::NO_CONTENT);
    }
    st.db.lock().unwrap().execute("DELETE FROM friends WHERE (a = ?1 AND b = ?2) OR (a = ?2 AND b = ?1)", [me.id, other.id]).map_err(internal)?;
    notify(&st, other.id, &json!({ "type": "friends" }));
    Ok(StatusCode::NO_CONTENT)
}

pub async fn search_users(st: S, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    let s = q.get("q").map(|s| s.trim()).unwrap_or("");
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Ok(Json(json!([])));
    }
    let users: Vec<User> = {
        let db = st.db.lock().unwrap();
        let mut stmt = db.prepare(&format!("SELECT {USER_COLS} FROM users WHERE name LIKE ?1 ESCAPE '\\' AND id != ?2 ORDER BY length(name) LIMIT 10")).map_err(internal)?;
        let v = stmt.query_map(params![format!("{}%", s.replace('_', "\\_")), me.id], row_user).map_err(internal)?.flatten().collect();
        v
    };
    Ok(Json(json!(users.iter().map(|u| public(&st, u)).collect::<Vec<_>>())))
}

pub async fn user(st: S, h: HeaderMap, Path(name): Path<String>) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    let u = user_by(&st, "name", &name).ok_or_else(not_found)?;
    let rel = relation(&st, me.id, u.id);
    let mut v = if rel == Rel::Friends { friend_view(&st, &u) } else { public(&st, &u) };
    v["relation"] = json!(match rel {
        Rel::None => "none",
        Rel::Friends => "friends",
        Rel::Outgoing => "outgoing",
        Rel::Incoming => "incoming",
    });
    v["packs"] = json!(list_packs(&st, &me, "p.owner = ?2", Some(u.id))?);
    Ok(Json(v))
}

// ------------------------------------------------------------ статуси (WebSocket)

#[derive(Deserialize)]
pub struct WsQuery {
    token: String,
    /// Версія лаунчера: WebSocket відкриває вікно, і User-Agent там браузерний.
    v: Option<String>,
}

pub async fn ws(st: S, Query(q): Query<WsQuery>, up: WebSocketUpgrade) -> R<Response> {
    if q.v.as_deref().and_then(crate::parse_version).is_none_or(|v| v < st.cfg.min_launcher) {
        return Err(crate::outdated());
    }
    let u = token_user(&st, &q.token).filter(|u| u.verified).ok_or_else(|| forbidden("Invalid token."))?;
    // Кожне з'єднання тримає буфери; без межі один акаунт вичерпав би пам'ять тисячами з'єднань.
    if st.sockets.lock().unwrap().get(&u.id).is_some_and(|l| l.len() >= 16) {
        return Err(ApiErr(StatusCode::TOO_MANY_REQUESTS, "TooManyRequests", "забагато відкритих з'єднань".into()));
    }
    let app = st.0.clone();
    // Статус — кількасот байтів; типові для axum 64 МБ на повідомлення (16 МБ на кадр) дозволили б
    // вичерпати пам'ять кількома повідомленнями.
    Ok(up.max_message_size(64 * 1024).max_frame_size(64 * 1024).on_upgrade(move |sock| session_ws(app, u, q.token, sock)))
}

fn broadcast_presence(app: &App, u: &User) {
    let p = app.presence.lock().unwrap().get(&u.id).cloned().unwrap_or(Value::Null);
    let msg = json!({ "type": "presence", "name": u.name, "presence": p });
    for f in friend_ids(app, u.id) {
        notify(app, f, &msg);
    }
}

/// Статус від лаунчера: онлайн / у грі (яка збірка) / відкритий світ. Адресу світу ставить сервер —
/// лише якщо цей код ретранслятора справді належить гравцю.
fn clean_presence(app: &App, u: &User, v: &Value) -> Value {
    let s = |x: &Value, n: usize| x.as_str().map(|s| s.chars().take(n).collect::<String>());
    let state = match v["state"].as_str() {
        Some("playing") => "playing",
        _ => "online",
    };
    let mut out = json!({ "state": state, "since": crate::now() });
    if state == "playing" {
        let g = &v["game"];
        out["game"] = json!({
            "name": s(&g["name"], 48),
            "mcVersion": s(&g["mcVersion"], 24),
            "loader": s(&g["loader"], 16),
            "pack": s(&g["pack"], 16),
        });
    }
    if let Some(code) = v["world"]["code"].as_str() {
        if app.relay.owner(code) == Some(u.id) {
            // licensedOnly: хост грає з ліцензією без «Грати з друзями» — пірати в такий світ не зайдуть.
            out["world"] = json!({
                "code": code,
                "address": format!("{code}.{}:{}", app.cfg.play_domain, app.cfg.relay_port),
                "licensedOnly": v["world"]["licensedOnly"].as_bool().unwrap_or(false),
            });
        }
    }
    out
}

async fn session_ws(app: Arc<App>, u: User, token: String, sock: WebSocket) {
    let (mut tx, mut rx) = sock.split();
    let (out, mut inbox) = tokio::sync::mpsc::unbounded_channel::<String>();
    app.sockets.lock().unwrap().entry(u.id).or_default().push(out.clone());
    // Статуси друзів, які вже онлайн.
    for f in friend_ids(&app, u.id) {
        let p = app.presence.lock().unwrap().get(&f).cloned();
        if let (Some(p), Some(fu)) = (p, user_by_id(&app, f)) {
            let _ = out.send(json!({ "type": "presence", "name": fu.name, "presence": p }).to_string());
        }
    }
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                m = inbox.recv() => match m {
                    Some(m) => if tx.send(Message::Text(m.into())).await.is_err() { break },
                    None => break,
                },
                _ = tokio::time::sleep(Duration::from_secs(30)) => if tx.send(Message::Ping(Bytes::new())).await.is_err() { break },
            }
        }
    });
    // Сесію відкликали (вихід з усіх пристроїв, новий пароль, видалення акаунта) — статуси друзів більше не бачить.
    let mut check = tokio::time::interval(Duration::from_secs(60));
    loop {
        let m = tokio::select! {
            m = rx.next() => m,
            _ = check.tick() => {
                if token_user(&app, &token).is_none() {
                    break;
                }
                continue;
            }
        };
        let Some(Ok(m)) = m else { break };
        let Message::Text(t) = m else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
        if v["type"] == "presence" {
            let p = clean_presence(&app, &u, &v);
            // Друзям — лише справжні зміни, а не кожне повідомлення (захист від засипання трафіком).
            let same = app.presence.lock().unwrap().get(&u.id).is_some_and(|old| old["state"] == p["state"] && old["game"] == p["game"] && old["world"] == p["world"]);
            if !same {
                app.presence.lock().unwrap().insert(u.id, p);
                broadcast_presence(&app, &u);
            }
        }
    }
    writer.abort();
    let last = {
        let mut s = app.sockets.lock().unwrap();
        let list = s.entry(u.id).or_default();
        list.retain(|x| !x.same_channel(&out));
        let empty = list.is_empty();
        if empty {
            s.remove(&u.id);
        }
        empty
    };
    if last {
        app.presence.lock().unwrap().remove(&u.id);
        broadcast_presence(&app, &u);
    }
}

/// Світ гравця закрився — прибрати його зі статусу.
pub fn world_closed(app: &App, user: i64, code: &str) {
    let changed = {
        let mut p = app.presence.lock().unwrap();
        match p.get_mut(&user) {
            Some(v) if v["world"]["code"] == code => {
                v.as_object_mut().unwrap().remove("world");
                true
            }
            _ => false,
        }
    };
    if changed {
        if let Some(u) = user_by_id(app, user) {
            broadcast_presence(app, &u);
        }
    }
}

// ------------------------------------------------------------ збірки

struct PackMeta {
    name: String,
    mc: String,
    loader: String,
}

/// Читає modrinth.index.json з .mrpack: назва, версія гри, модлоадер.
fn read_mrpack(mut f: impl Read + Seek) -> R<PackMeta> {
    // Кінець zip64 може оголосити мільйони записів, і вже розбір каталогу з'їдає сотні МБ пам'яті.
    // Збірці вистачає звичайного zip (до 65535 файлів), тож записи кінця з ознаками zip64 не приймаємо.
    let len = f.seek(SeekFrom::End(0)).map_err(internal)?;
    f.seek(SeekFrom::Start(len.saturating_sub(68 * 1024))).map_err(internal)?;
    let mut tail = vec![];
    f.read_to_end(&mut tail).map_err(internal)?;
    if tail.windows(22).any(|w| w.starts_with(b"PK\x05\x06") && (w[10..12] == [0xFF; 2] || w[12..16] == [0xFF; 4] || w[16..20] == [0xFF; 4])) {
        return Err(bad("у збірці забагато файлів"));
    }
    let mut zip = zip::ZipArchive::new(f).map_err(|_| bad("це не .mrpack"))?;
    let mut s = String::new();
    zip.by_name("modrinth.index.json").map_err(|_| bad("у файлі немає modrinth.index.json"))?.take(4 * 1024 * 1024).read_to_string(&mut s).map_err(|_| bad("пошкоджений modrinth.index.json"))?;
    let v: Value = serde_json::from_str(&s).map_err(|_| bad("пошкоджений modrinth.index.json"))?;
    let deps = &v["dependencies"];
    let mc: String = deps["minecraft"].as_str().ok_or_else(|| bad("не вказано версію Minecraft"))?.chars().take(32).collect();
    let loader = ["fabric-loader", "quilt-loader", "forge", "neoforge"].iter().find(|l| deps[**l].is_string()).map(|l| l.trim_end_matches("-loader").to_string()).unwrap_or_else(|| "vanilla".into());
    let name: String = v["name"].as_str().unwrap_or("Збірка").chars().take(64).collect();
    Ok(PackMeta { name, mc, loader })
}

fn pack_visible(app: &App, me: &User, owner: i64, visibility: &str) -> bool {
    owner == me.id || visibility == "public" || relation(app, me.id, owner) == Rel::Friends
}

/// Збірки, які бачить `me`, з додатковою умовою `cond` (параметр ?2 — `arg`).
fn list_packs(app: &App, me: &User, cond: &str, arg: Option<i64>) -> R<Vec<Value>> {
    let rows: Vec<(String, i64, String, String, String, String, String, i64, i64, i64, String, bool, i64)> = {
        let db = app.db.lock().unwrap();
        let sql = format!(
            "SELECT p.id, p.owner, p.name, p.description, p.mc_version, p.loader, p.visibility, p.version, p.size, p.updated, u.name,
                    EXISTS(SELECT 1 FROM subs s WHERE s.pack = p.id AND s.user = ?1),
                    (SELECT count(*) FROM subs s WHERE s.pack = p.id)
             FROM packs p JOIN users u ON u.id = p.owner WHERE {cond} ORDER BY p.updated DESC LIMIT 100"
        );
        let mut q = db.prepare(&sql).map_err(internal)?;
        let map = |r: &rusqlite::Row| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?));
        let v = match arg {
            Some(a) => q.query_map(params![me.id, a], map),
            None => q.query_map(params![me.id], map),
        }
        .map_err(internal)?
        .flatten()
        .collect();
        v
    };
    Ok(rows
        .into_iter()
        .filter(|r| pack_visible(app, me, r.1, &r.6))
        .map(|r| {
            json!({
                "id": r.0, "owner": r.10, "mine": r.1 == me.id, "name": r.2, "description": r.3, "mcVersion": r.4, "loader": r.5,
                "visibility": r.6, "version": r.7, "size": r.8, "updated": r.9, "subscribed": r.11, "subscribers": r.12,
            })
        })
        .collect())
}

pub async fn packs(st: S, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    let list = match q.get("user") {
        Some(n) => {
            let u = user_by(&st, "name", n).ok_or_else(not_found)?;
            list_packs(&st, &me, "p.owner = ?2", Some(u.id))?
        }
        // Без фільтра — мої та друзів.
        None => {
            let mut ids = friend_ids(&st, me.id);
            ids.push(me.id);
            let set = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            list_packs(&st, &me, &format!("p.owner IN ({set})"), None)?
        }
    };
    Ok(Json(json!(list)))
}

pub async fn packs_subscribed(st: S, h: HeaderMap) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    Ok(Json(json!(list_packs(&st, &me, "EXISTS(SELECT 1 FROM subs s WHERE s.pack = p.id AND s.user = ?1)", None)?)))
}

fn pack_path(app: &App, id: &str) -> std::path::PathBuf {
    app.cfg.data.join("packs").join(format!("{id}.mrpack"))
}

fn valid_id(id: &str) -> R<()> {
    if id.len() == 12 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(not_found())
    }
}

/// (власник, видимість) збірки.
fn pack_owner(app: &App, id: &str) -> R<(i64, String)> {
    valid_id(id)?;
    app.db.lock().unwrap().query_row("SELECT owner, visibility FROM packs WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(internal)?.ok_or_else(not_found)
}

fn visibility(q: &HashMap<String, String>) -> &'static str {
    if q.get("visibility").is_some_and(|v| v == "public") {
        "public"
    } else {
        "friends"
    }
}

/// Тимчасовий файл збірки прибирається сам: і при помилці, і коли запит обірвали посеред завантаження.
struct TmpFile(std::path::PathBuf);

impl Drop for TmpFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Приймає збірку в тимчасовий файл, а не в пам'ять (буває до 100 МБ), перевіряє й ставить на місце.
/// Повертає метадані й розмір.
async fn save_pack(app: &App, me: &User, id: &str, body: Body, replacing: i64) -> R<(PackMeta, i64)> {
    let tmp = TmpFile(app.cfg.data.join("packs").join(format!("{}.tmp", hex(&rand::random::<[u8; 8]>()))));
    let mut f = tokio::fs::File::create(&tmp.0).await.map_err(internal)?;
    let mut size = 0;
    let mut s = body.into_data_stream();
    while let Some(chunk) = s.next().await {
        let chunk = chunk.map_err(|_| bad("збірку не вдалося отримати"))?;
        size += chunk.len();
        if size > PACK_MAX {
            return Err(bad("збірка завелика (до 100 МБ)"));
        }
        f.write_all(&chunk).await.map_err(internal)?;
    }
    f.flush().await.map_err(internal)?;
    drop(f);
    let meta = read_mrpack(std::fs::File::open(&tmp.0).map_err(internal)?)?;
    let used: i64 = app.db.lock().unwrap().query_row("SELECT coalesce(sum(size), 0) FROM packs WHERE owner = ?1", [me.id], |r| r.get(0)).map_err(internal)?;
    if used - replacing + size as i64 > PACK_QUOTA {
        return Err(bad("перевищено місце для збірок (1 ГБ)"));
    }
    std::fs::rename(&tmp.0, pack_path(app, id)).map_err(internal)?;
    Ok((meta, size as i64))
}

fn notify_friends(app: &App, me: &User, msg: &Value) {
    for f in friend_ids(app, me.id) {
        notify(app, f, msg);
    }
}

// Тіло — `Body`, а не `Bytes`: інакше axum читав би все тіло в пам'ять ще до перевірки входу.
pub async fn pack_publish(st: S, h: HeaderMap, Query(q): Query<HashMap<String, String>>, body: Body) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    st.limit(format!("pack:{}", me.id), 30, Duration::from_secs(3600))?;
    let id = hex(&rand::random::<[u8; 6]>());
    let (meta, size) = save_pack(&st, &me, &id, body, 0).await?;
    let desc: String = q.get("description").map(|d| d.chars().take(500).collect()).unwrap_or_default();
    let now = crate::now();
    st.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO packs (id, owner, name, description, mc_version, loader, visibility, version, size, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?9)",
            params![id, me.id, meta.name, desc, meta.mc, meta.loader, visibility(&q), size, now],
        )
        .map_err(internal)?;
    event(&st, me.id, "pack_published", Some(&id), json!({ "name": meta.name, "version": 1 }));
    notify_friends(&st, &me, &json!({ "type": "feed" }));
    pack_get_inner(&st, &me, &id)
}

pub async fn pack_update(st: S, h: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>, body: Body) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    let (owner, _) = pack_owner(&st, &id)?;
    if owner != me.id {
        return Err(forbidden("це не ваша збірка"));
    }
    st.limit(format!("pack:{}", me.id), 30, Duration::from_secs(3600))?;
    let old: i64 = st.db.lock().unwrap().query_row("SELECT size FROM packs WHERE id = ?1", [&id], |r| r.get(0)).map_err(internal)?;
    let (meta, size) = save_pack(&st, &me, &id, body, old).await?;
    let version: i64 = {
        let db = st.db.lock().unwrap();
        db.execute(
            "UPDATE packs SET name = ?1, mc_version = ?2, loader = ?3, size = ?4, version = version + 1, updated = ?5,
                              visibility = coalesce(?6, visibility), description = coalesce(?7, description) WHERE id = ?8",
            params![meta.name, meta.mc, meta.loader, size, crate::now(), q.get("visibility").map(|_| visibility(&q)), q.get("description").map(|d| d.chars().take(500).collect::<String>()), id],
        )
        .map_err(internal)?;
        db.query_row("SELECT version FROM packs WHERE id = ?1", [&id], |r| r.get(0)).map_err(internal)?
    };
    event(&st, me.id, "pack_updated", Some(&id), json!({ "name": meta.name, "version": version }));
    let subs: Vec<i64> = {
        let db = st.db.lock().unwrap();
        let mut q = db.prepare("SELECT user FROM subs WHERE pack = ?1").map_err(internal)?;
        let v = q.query_map([&id], |r| r.get(0)).map_err(internal)?.flatten().collect();
        v
    };
    let msg = json!({ "type": "pack", "id": id, "version": version });
    for s in subs {
        notify(&st, s, &msg);
    }
    notify_friends(&st, &me, &json!({ "type": "feed" }));
    pack_get_inner(&st, &me, &id)
}

pub async fn pack_delete(st: S, h: HeaderMap, Path(id): Path<String>) -> R<StatusCode> {
    let me = auth(&st, &h)?;
    let (owner, _) = pack_owner(&st, &id)?;
    if owner != me.id {
        return Err(forbidden("це не ваша збірка"));
    }
    {
        let db = st.db.lock().unwrap();
        db.execute("DELETE FROM events WHERE pack = ?1", [&id]).map_err(internal)?;
        db.execute("DELETE FROM packs WHERE id = ?1", [&id]).map_err(internal)?;
    }
    let _ = std::fs::remove_file(pack_path(&st, &id));
    Ok(StatusCode::NO_CONTENT)
}

fn pack_get_inner(app: &App, me: &User, id: &str) -> R<Json<Value>> {
    valid_id(id)?;
    let list = list_packs(app, me, &format!("p.id = '{id}'"), None)?;
    list.into_iter().next().map(Json).ok_or_else(not_found)
}

pub async fn pack_get(st: S, h: HeaderMap, Path(id): Path<String>) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    pack_get_inner(&st, &me, &id)
}

pub async fn pack_file(st: S, h: HeaderMap, Path(id): Path<String>) -> R<Response> {
    let me = auth(&st, &h)?;
    let (owner, vis) = pack_owner(&st, &id)?;
    if !pack_visible(&st, &me, owner, &vis) {
        return Err(not_found());
    }
    let f = tokio::fs::File::open(pack_path(&st, &id)).await.map_err(|_| not_found())?;
    let len = f.metadata().await.map_err(internal)?.len();
    // Потоком, а не цілим файлом у пам'яті: кілька одночасних завантажень великої збірки вичерпали б пам'ять сервера.
    let stream = futures::stream::try_unfold(f, |mut f| async move {
        let mut buf = vec![0u8; 64 * 1024];
        let n = f.read(&mut buf).await?;
        buf.truncate(n);
        Ok::<_, std::io::Error>((n > 0).then(|| (Bytes::from(buf), f)))
    });
    Ok(([(header::CONTENT_TYPE, "application/x-modrinth-modpack+zip".to_string()), (header::CONTENT_LENGTH, len.to_string())], Body::from_stream(stream)).into_response())
}

pub async fn pack_subscribe(st: S, h: HeaderMap, Path(id): Path<String>) -> R<StatusCode> {
    let me = auth(&st, &h)?;
    let (owner, vis) = pack_owner(&st, &id)?;
    if !pack_visible(&st, &me, owner, &vis) {
        return Err(not_found());
    }
    st.db.lock().unwrap().execute("INSERT OR IGNORE INTO subs (user, pack) VALUES (?1, ?2)", params![me.id, id]).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pack_unsubscribe(st: S, h: HeaderMap, Path(id): Path<String>) -> R<StatusCode> {
    let me = auth(&st, &h)?;
    valid_id(&id)?;
    st.db.lock().unwrap().execute("DELETE FROM subs WHERE user = ?1 AND pack = ?2", params![me.id, id]).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Стрічка: події друзів і оновлення збірок, на які я підписаний.
pub async fn feed(st: S, h: HeaderMap) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    let mut ids = friend_ids(&st, me.id);
    ids.push(me.id);
    let set = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let rows: Vec<(i64, String, String, Option<String>, String, i64)> = {
        let db = st.db.lock().unwrap();
        let mut q = db
            .prepare(&format!(
                "SELECT e.id, u.name, e.kind, e.pack, e.data, e.created FROM events e JOIN users u ON u.id = e.user
                 WHERE e.user IN ({set}) OR e.pack IN (SELECT pack FROM subs WHERE user = ?1)
                 ORDER BY e.id DESC LIMIT 60"
            ))
            .map_err(internal)?;
        let v = q.query_map([me.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).map_err(internal)?.flatten().collect();
        v
    };
    let mut out = vec![];
    for (id, user, kind, pack, data, created) in rows {
        // Подія збірки, якої вже не видно (стала приватною або друг видалився), — пропускаємо.
        if let Some(p) = &pack {
            match pack_owner(&st, p) {
                Ok((o, v)) if pack_visible(&st, &me, o, &v) => {}
                _ => continue,
            }
        }
        out.push(json!({ "id": id, "user": user, "kind": kind, "pack": pack, "data": serde_json::from_str::<Value>(&data).unwrap_or_default(), "created": created }));
    }
    Ok(Json(json!(out)))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    fn mrpack(mc: &str) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file("modrinth.index.json", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(serde_json::json!({ "name": "Тест", "dependencies": { "minecraft": mc, "fabric-loader": "0.16.0" } }).to_string().as_bytes()).unwrap();
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn mrpack_meta() {
        let m = super::read_mrpack(Cursor::new(mrpack("1.21.1"))).unwrap();
        assert_eq!((m.name.as_str(), m.mc.as_str(), m.loader.as_str()), ("Тест", "1.21.1", "fabric"));
        assert_eq!(super::read_mrpack(Cursor::new(mrpack(&"1".repeat(100_000)))).unwrap().mc.len(), 32);
        // Кінець архіву з ознакою zip64 (кількість записів 0xFFFF) — відмова ще до розбору каталогу.
        let mut b = mrpack("1.21.1");
        let n = b.len();
        b[n - 12..n - 10].copy_from_slice(&[0xFF, 0xFF]);
        assert_eq!(super::read_mrpack(Cursor::new(b)).err().unwrap().2, "у збірці забагато файлів");
    }
}
