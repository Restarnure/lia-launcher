//! Вхід гри через ЛІЮ (authlib-injector). Гра входить не акаунтом ЛІЯ ID, а ігровим профілем —
//! офлайн-ніком чи ліцензійним профілем, який належить акаунту ЛІЯ ID. Ліцензійних гравців без ЛІЇ
//! перевіряємо через Mojang, тож у світ гравця ЛІЇ заходять усі.
//! Специфікація: https://github.com/yushijinhun/authlib-injector/wiki/Yggdrasil-服务端技术规范
use crate::api;
use crate::{bad, forbidden, internal, App, ApiErr, R, S};
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use md5::Digest as _;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
use rsa::signature::{SignatureEncoding, Signer};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::time::Instant;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const MOJANG_SESSION: &str = "https://sessionserver.mojang.com/session/minecraft";

/// Ключ підпису текстур. Створюється при розгортанні: `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:4096`.
pub fn load_key(path: &FsPath) -> (rsa::RsaPrivateKey, String) {
    let pem = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("немає ключа підпису {}", path.display()));
    let key = rsa::RsaPrivateKey::from_pkcs8_pem(&pem).expect("ключ підпису (PKCS#8 PEM)");
    let public = key.to_public_key().to_public_key_pem(LineEnding::LF).expect("публічний ключ");
    (key, public)
}

fn sign(app: &App, data: &str) -> String {
    let sk = SigningKey::<sha1::Sha1>::new(app.key.clone());
    B64.encode(sk.sign(data.as_bytes()).to_bytes())
}

pub fn undashed(uuid: &str) -> String {
    uuid.replace('-', "")
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn dashed(u: &str) -> Option<String> {
    uuid::Uuid::parse_str(u).ok().map(|x| x.hyphenated().to_string())
}

/// UUID офлайн-гравця — той самий, що рахує сервер Minecraft: MD5("OfflinePlayer:нік"), версія 3.
pub fn offline_uuid(name: &str) -> String {
    let mut h: [u8; 16] = md5::Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(h).hyphenated().to_string()
}

// ---------------------------------------------------------------- ігрові профілі

#[derive(Clone)]
pub struct Profile {
    pub id: i64,
    pub user: i64,
    pub name: String,
    pub uuid: String,
    /// "offline" | "mojang"
    pub kind: String,
    /// Скін з Mojang для ліцензійного профілю, якщо в ЛІЯ ID свого немає: (url, slim).
    pub mojang_skin: Option<(String, bool)>,
}

const PROFILE_COLS: &str = "id, user, name, uuid, kind, mojang_skin, mojang_slim";

fn row_profile(r: &rusqlite::Row) -> rusqlite::Result<Profile> {
    let skin: Option<String> = r.get(5)?;
    Ok(Profile { id: r.get(0)?, user: r.get(1)?, name: r.get(2)?, uuid: r.get(3)?, kind: r.get(4)?, mojang_skin: skin.map(|s| (s, r.get::<_, i64>(6).unwrap_or(0) != 0)) })
}

pub fn profile_by(app: &App, col: &str, val: &str) -> Option<Profile> {
    let col = match col {
        "name" => "name",
        "uuid" => "uuid",
        _ => return None,
    };
    app.db.lock().unwrap().query_row(&format!("SELECT {PROFILE_COLS} FROM profiles WHERE {col} = ?1"), [val], row_profile).ok()
}

/// Профіль, на який виписано ігровий токен. Ліцензійний профіль переходить до того, хто останнім довів
/// ліцензію, і токени попереднього власника тоді вже не діють.
pub fn token_profile(app: &App, token: &str) -> Option<Profile> {
    let db = app.db.lock().unwrap();
    let (pid, user): (i64, i64) = db.query_row("SELECT profile, user FROM tokens WHERE hash = ?1 AND profile IS NOT NULL", [api::token_hash(token)], |r| Ok((r.get(0)?, r.get(1)?))).ok()?;
    db.query_row(&format!("SELECT {PROFILE_COLS} FROM profiles WHERE id = ?1 AND user = ?2"), [pid, user], row_profile).ok()
}

/// Текстури профілю: у ліцензії — скін з Mojang; офлайн-нік — стандартний.
fn textures(p: &Profile) -> Value {
    let mut t = json!({});
    if let Some((url, slim)) = p.mojang_skin.clone() {
        t["SKIN"] = json!({ "url": url });
        if slim {
            t["SKIN"]["metadata"] = json!({ "model": "slim" });
        }
    }
    t
}

fn profile_json(app: &App, p: &Profile, signed: bool) -> Value {
    let value = B64.encode(json!({ "timestamp": crate::now(), "profileId": undashed(&p.uuid), "profileName": p.name, "textures": textures(p) }).to_string());
    let mut prop = json!({ "name": "textures", "value": value });
    if signed {
        prop["signature"] = json!(sign(app, prop["value"].as_str().unwrap()));
    }
    json!({ "id": undashed(&p.uuid), "name": p.name, "properties": [prop] })
}

/// Профіль від Mojang (ліцензійний гравець без ЛІЇ) — з тими самими текстурами, але підписаними нашим ключем,
/// бо гра з authlib-injector перевіряє підпис ключем ЛІЇ.
fn resign_mojang(app: &App, mut v: Value, signed: bool) -> Value {
    if let Some(props) = v["properties"].as_array_mut() {
        props.retain(|p| p["name"] == "textures");
        for p in props.iter_mut() {
            let sig = p["value"].as_str().map(|val| sign(app, val));
            match (signed, sig) {
                (true, Some(s)) => p["signature"] = json!(s),
                _ => {
                    p.as_object_mut().map(|o| o.remove("signature"));
                }
            }
        }
    }
    v
}

fn mojang_skin_of(profile: &Value) -> Option<(String, bool)> {
    let prop = profile["properties"].as_array()?.iter().find(|p| p["name"] == "textures")?;
    let tex: Value = serde_json::from_slice(&B64.decode(prop["value"].as_str()?).ok()?).ok()?;
    let skin = &tex["textures"]["SKIN"];
    Some((skin["url"].as_str()?.to_string(), skin["metadata"]["model"] == "slim"))
}

async fn mojang_get(app: &App, url: &str) -> Option<Value> {
    let r = app.http.get(url).send().await.ok()?;
    if r.status() != StatusCode::OK {
        return None;
    }
    r.json().await.ok()
}

fn valid_nick(n: &str) -> bool {
    (3..=16).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Deserialize)]
pub struct SessionReq {
    kind: String,
    name: String,
    /// Для ліцензії: serverId, з яким лаунчер щойно зробив join на сервері Mojang.
    #[serde(rename = "serverId")]
    server_id: Option<String>,
}

/// Ігрова сесія для запуску гри: закріплює нік (або ліцензійний профіль) за акаунтом ЛІЯ ID
/// і видає токен, з яким гра входить через authlib-injector.
pub async fn game_session(st: S, h: HeaderMap, Json(r): Json<SessionReq>) -> R<Json<Value>> {
    let user = api::auth(&st, &h)?;
    let profile = match r.kind.as_str() {
        "offline" => {
            if !valid_nick(&r.name) {
                return Err(bad("нік: 3–16 символів, латинські літери, цифри та _"));
            }
            match profile_by(&st, "name", &r.name) {
                Some(p) if p.user == user.id => p,
                Some(p) if p.kind == "mojang" => return Err(ApiErr(StatusCode::CONFLICT, "NickTaken", format!("нік {} уже зайнятий ліцензійним гравцем ЛІЇ", p.name))),
                Some(_) => return Err(ApiErr(StatusCode::CONFLICT, "NickTaken", "цей нік уже використовує інший гравець ЛІЇ".into())),
                None => {
                    st.limit(format!("nick:{}", user.id), 20, std::time::Duration::from_secs(3600))?;
                    // Не більше 10 ніків на акаунт — щоб не можна було скупити чужі ніки.
                    let owned: i64 = st.db.lock().unwrap().query_row("SELECT count(*) FROM profiles WHERE user = ?1 AND kind = 'offline'", [user.id], |r| r.get(0)).map_err(internal)?;
                    if owned >= 10 {
                        return Err(bad("на одному ЛІЯ ID може бути не більше 10 ніків"));
                    }
                    let uuid = offline_uuid(&r.name);
                    st.db
                        .lock()
                        .unwrap()
                        .execute("INSERT INTO profiles (user, name, uuid, kind, created) VALUES (?1, ?2, ?3, 'offline', ?4)", params![user.id, r.name, uuid, crate::now()])
                        .map_err(internal)?;
                    profile_by(&st, "uuid", &uuid).ok_or_else(crate::not_found)?
                }
            }
        }
        "mojang" => {
            // Володіння ліцензією доводимо без передачі токена Mojang: лаунчер робить join із нашим serverId,
            // а ми питаємо Mojang hasJoined.
            let sid = r.server_id.unwrap_or_default();
            let ok = st.challenges.lock().unwrap().remove(&sid).is_some_and(|(uid, at)| uid == user.id && at.elapsed().as_secs() < 120);
            if !ok || !valid_nick(&r.name) {
                return Err(forbidden("перевірку ліцензії не пройдено, спробуйте ще раз"));
            }
            let v = mojang_get(&st, &format!("{MOJANG_SESSION}/hasJoined?username={}&serverId={sid}", r.name))
                .await
                .ok_or_else(|| forbidden("Mojang не підтвердив ліцензію"))?;
            let uuid = v["id"].as_str().and_then(dashed).ok_or_else(|| forbidden("Mojang не підтвердив ліцензію"))?;
            let name = v["name"].as_str().unwrap_or(&r.name).to_string();
            let skin = mojang_skin_of(&v);
            {
                let db = st.db.lock().unwrap();
                // Ліцензія має пріоритет: офлайн-нік з тим самим іменем у когось іншого звільняється.
                db.execute("DELETE FROM profiles WHERE name = ?1 AND uuid != ?2", params![name, uuid]).map_err(internal)?;
                db.execute(
                    "INSERT INTO profiles (user, name, uuid, kind, mojang_skin, mojang_slim, created) VALUES (?1, ?2, ?3, 'mojang', ?4, ?5, ?6)
                     ON CONFLICT(uuid) DO UPDATE SET user = ?1, name = ?2, kind = 'mojang', mojang_skin = ?4, mojang_slim = ?5",
                    params![user.id, name, uuid, skin.as_ref().map(|s| s.0.clone()), skin.as_ref().is_some_and(|s| s.1) as i64, crate::now()],
                )
                .map_err(internal)?;
            }
            profile_by(&st, "uuid", &uuid).ok_or_else(crate::not_found)?
        }
        _ => return Err(bad("kind")),
    };
    let token = hex(&rand::random::<[u8; 32]>());
    {
        let db = st.db.lock().unwrap();
        db.execute("INSERT INTO tokens (hash, user, client, created, profile) VALUES (?1, ?2, '', ?3, ?4)", params![api::token_hash(&token), user.id, crate::now(), profile.id])
            .map_err(internal)?;
        // Старі ігрові токени цього профілю не потрібні — лишаємо кілька останніх (кілька ПК одночасно).
        db.execute(
            "DELETE FROM tokens WHERE profile = ?1 AND hash NOT IN (SELECT hash FROM tokens WHERE profile = ?1 ORDER BY created DESC LIMIT 5)",
            [profile.id],
        )
        .map_err(internal)?;
    }
    Ok(Json(json!({ "accessToken": token, "uuid": undashed(&profile.uuid), "name": profile.name })))
}

/// serverId для перевірки ліцензії (див. game_session).
pub async fn game_challenge(st: S, h: HeaderMap) -> R<Json<Value>> {
    let user = api::auth(&st, &h)?;
    let sid = hex(&rand::random::<[u8; 16]>());
    st.challenges.lock().unwrap().insert(sid.clone(), (user.id, Instant::now()));
    Ok(Json(json!({ "serverId": sid })))
}

// ---------------------------------------------------------------- Yggdrasil

pub async fn meta(st: S) -> Json<Value> {
    let host = st.cfg.public_url.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("").to_string();
    Json(json!({
        "meta": {
            "serverName": "ЛІЯ",
            "implementationName": "lia-server",
            "implementationVersion": env!("CARGO_PKG_VERSION"),
            "links": { "homepage": st.cfg.public_url },
        },
        // Скіни ліцензійних гравців лишаються на серверах Mojang.
        "skinDomains": [host, ".minecraft.net"],
        "signaturePublickey": st.public_pem,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenReq {
    access_token: String,
}

pub async fn validate(st: S, Json(r): Json<TokenReq>) -> R<StatusCode> {
    token_profile(&st, &r.access_token).ok_or_else(|| forbidden("Invalid token."))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn invalidate(st: S, Json(r): Json<TokenReq>) -> StatusCode {
    let _ = st.db.lock().unwrap().execute("DELETE FROM tokens WHERE hash = ?1 AND profile IS NOT NULL", [api::token_hash(&r.access_token)]);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinReq {
    access_token: String,
    selected_profile: String,
    server_id: String,
}

pub async fn join(st: S, Json(r): Json<JoinReq>) -> R<StatusCode> {
    let p = token_profile(&st, &r.access_token).ok_or_else(|| forbidden("Invalid token."))?;
    if undashed(&p.uuid) != undashed(&r.selected_profile) {
        return Err(forbidden("Invalid profile."));
    }
    if r.server_id.len() > 64 {
        return Err(bad("serverId"));
    }
    st.joins.lock().unwrap().insert(r.server_id, (p.uuid, p.name, Instant::now()));
    Ok(StatusCode::NO_CONTENT)
}

pub async fn has_joined(st: S, h: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (Some(name), Some(server)) = (q.get("username"), q.get("serverId")) else {
        return StatusCode::NO_CONTENT.into_response();
    };
    let hit = st.joins.lock().unwrap().remove(server);
    if let Some((uuid, n, at)) = hit {
        if n.eq_ignore_ascii_case(name) && at.elapsed().as_secs() < 30 {
            if let Some(p) = profile_by(&st, "uuid", &uuid) {
                return Json(profile_json(&st, &p, true)).into_response();
            }
        }
        return StatusCode::NO_CONTENT.into_response();
    }
    // Не наш гравець — може, ліцензія без ЛІЇ: питаємо Mojang (з лімітом, щоб нами не засипали Mojang).
    if st.limit(format!("mojang:{}", crate::client_ip(&h)), 120, std::time::Duration::from_secs(60)).is_err() {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let mut url = format!("{MOJANG_SESSION}/hasJoined?username={}&serverId={}", urlenc(name), urlenc(server));
    if let Some(ip) = q.get("ip") {
        url += &format!("&ip={}", urlenc(ip));
    }
    match mojang_get(&st, &url).await {
        Some(v) => Json(resign_mojang(&st, v, true)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

fn urlenc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub async fn profile(st: S, h: HeaderMap, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some(uuid) = dashed(&id) else {
        return StatusCode::NO_CONTENT.into_response();
    };
    let signed = q.get("unsigned").map(String::as_str) == Some("false");
    if let Some(p) = profile_by(&st, "uuid", &uuid) {
        return Json(profile_json(&st, &p, signed)).into_response();
    }
    // Скін ліцензійного гравця без ЛІЇ — з Mojang.
    if st.limit(format!("mojang:{}", crate::client_ip(&h)), 120, std::time::Duration::from_secs(60)).is_err() {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    match mojang_get(&st, &format!("{MOJANG_SESSION}/profile/{}", undashed(&uuid))).await {
        Some(v) => Json(resign_mojang(&st, v, signed)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn profiles_by_names(st: S, Json(names): Json<Vec<String>>) -> Json<Value> {
    let out: Vec<Value> = names.iter().take(10).filter_map(|n| profile_by(&st, "name", n)).map(|p| json!({ "id": undashed(&p.uuid), "name": p.name })).collect();
    Json(json!(out))
}

/// Ігрові профілі акаунта (для сторінки ЛІЯ ID).
pub fn profiles_of(app: &App, user: i64) -> Vec<Value> {
    let db = app.db.lock().unwrap();
    let mut q = db.prepare(&format!("SELECT {PROFILE_COLS} FROM profiles WHERE user = ?1 ORDER BY created")).unwrap();
    q.query_map([user], row_profile).unwrap().flatten().map(|p| json!({ "name": p.name, "kind": p.kind })).collect()
}


#[cfg(test)]
mod tests {
    #[test]
    fn offline_uuid_matches_minecraft() {
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch") у Java.
        assert_eq!(super::offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }
}
