//! API Yggdrasil у форматі authlib-injector: гра з цим агентом входить і перевіряє гравців через ЛІЯ ID.
//! Специфікація: https://github.com/yushijinhun/authlib-injector/wiki/Yggdrasil-服务端技术规范
use crate::api::{self, User};
use crate::{bad, forbidden, internal, App, R, S};
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
use rsa::signature::{SignatureEncoding, Signer};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::time::Instant;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

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

/// Профіль гравця з властивістю textures (підписаною, якщо просять).
pub fn profile_json(app: &App, u: &User, signed: bool) -> Value {
    let mut textures = json!({});
    if let Some(skin) = &u.skin {
        let mut s = json!({ "url": app.texture_url(skin) });
        if u.slim {
            s["metadata"] = json!({ "model": "slim" });
        }
        textures["SKIN"] = s;
    }
    let value = B64.encode(
        json!({
            "timestamp": crate::now(),
            "profileId": undashed(&u.uuid),
            "profileName": u.name,
            "textures": textures,
        })
        .to_string(),
    );
    let mut prop = json!({ "name": "textures", "value": value });
    if signed {
        prop["signature"] = json!(sign(app, prop["value"].as_str().unwrap()));
    }
    json!({
        "id": undashed(&u.uuid),
        "name": u.name,
        "properties": [prop, { "name": "uploadableTextures", "value": "skin" }],
    })
}

fn short(u: &User) -> Value {
    json!({ "id": undashed(&u.uuid), "name": u.name })
}

pub async fn meta(st: S) -> Json<Value> {
    let host = st.cfg.public_url.split("://").nth(1).unwrap_or("").split(['/', ':']).next().unwrap_or("").to_string();
    Json(json!({
        "meta": {
            "serverName": "ЛІЯ ID",
            "implementationName": "lia-server",
            "implementationVersion": env!("CARGO_PKG_VERSION"),
            "links": { "homepage": st.cfg.public_url },
            "feature.non_email_login": true,
        },
        "skinDomains": [host],
        "signaturePublickey": st.public_pem,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthReq {
    username: String,
    password: String,
    client_token: Option<String>,
    #[serde(default)]
    request_user: bool,
}

/// Видача токена. Його ж лаунчер використовує для API ЛІЇ (Bearer).
pub fn session(app: &App, u: &User, client: Option<String>, request_user: bool) -> R<Value> {
    let client = client.filter(|c| !c.is_empty() && c.len() <= 64).unwrap_or_else(|| hex(&rand::random::<[u8; 16]>()));
    let token = hex(&rand::random::<[u8; 32]>());
    app.db
        .lock()
        .unwrap()
        .execute("INSERT INTO tokens (hash, user, client, created) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![api::token_hash(&token), u.id, client, crate::now()])
        .map_err(internal)?;
    let mut out = json!({
        "accessToken": token,
        "clientToken": client,
        "availableProfiles": [short(u)],
        "selectedProfile": short(u),
    });
    if request_user {
        out["user"] = json!({ "id": undashed(&u.uuid), "properties": [] });
    }
    Ok(out)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub async fn authenticate(st: S, headers: HeaderMap, Json(r): Json<AuthReq>) -> R<Json<Value>> {
    let ip = crate::client_ip(&headers);
    st.limit(format!("auth:{ip}"), 20, std::time::Duration::from_secs(600))?;
    let u = api::check_password(&st, &r.username, &r.password)?;
    Ok(Json(session(&st, &u, r.client_token, r.request_user)?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenReq {
    access_token: String,
    client_token: Option<String>,
    #[serde(default)]
    request_user: bool,
}

pub async fn refresh(st: S, Json(r): Json<TokenReq>) -> R<Json<Value>> {
    let u = api::token_user(&st, &r.access_token, r.client_token.as_deref()).ok_or_else(|| forbidden("Invalid token."))?;
    let client = st.db.lock().unwrap().query_row("SELECT client FROM tokens WHERE hash = ?1", [api::token_hash(&r.access_token)], |r| r.get::<_, String>(0)).ok();
    revoke(&st, &r.access_token);
    Ok(Json(session(&st, &u, client, r.request_user)?))
}

pub async fn validate(st: S, Json(r): Json<TokenReq>) -> R<StatusCode> {
    api::token_user(&st, &r.access_token, r.client_token.as_deref()).ok_or_else(|| forbidden("Invalid token."))?;
    Ok(StatusCode::NO_CONTENT)
}

fn revoke(app: &App, token: &str) {
    let _ = app.db.lock().unwrap().execute("DELETE FROM tokens WHERE hash = ?1", [api::token_hash(token)]);
}

pub async fn invalidate(st: S, Json(r): Json<TokenReq>) -> StatusCode {
    revoke(&st, &r.access_token);
    StatusCode::NO_CONTENT
}

pub async fn signout(st: S, headers: HeaderMap, Json(r): Json<AuthReq>) -> R<StatusCode> {
    st.limit(format!("auth:{}", crate::client_ip(&headers)), 20, std::time::Duration::from_secs(600))?;
    let u = api::check_password(&st, &r.username, &r.password)?;
    st.db.lock().unwrap().execute("DELETE FROM tokens WHERE user = ?1", [u.id]).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinReq {
    access_token: String,
    selected_profile: String,
    server_id: String,
}

pub async fn join(st: S, Json(r): Json<JoinReq>) -> R<StatusCode> {
    let u = api::token_user(&st, &r.access_token, None).ok_or_else(|| forbidden("Invalid token."))?;
    if undashed(&u.uuid) != undashed(&r.selected_profile) {
        return Err(forbidden("Invalid profile."));
    }
    if r.server_id.len() > 64 {
        return Err(bad("serverId"));
    }
    st.joins.lock().unwrap().insert(r.server_id, (u.uuid, u.name, Instant::now()));
    Ok(StatusCode::NO_CONTENT)
}

pub async fn has_joined(st: S, Query(q): Query<HashMap<String, String>>) -> Response {
    let (Some(name), Some(server)) = (q.get("username"), q.get("serverId")) else {
        return StatusCode::NO_CONTENT.into_response();
    };
    let hit = st.joins.lock().unwrap().remove(server);
    match hit {
        Some((uuid, n, at)) if n.eq_ignore_ascii_case(name) && at.elapsed().as_secs() < 30 => match api::user_by(&st, "uuid", &uuid) {
            Some(u) => Json(profile_json(&st, &u, true)).into_response(),
            None => StatusCode::NO_CONTENT.into_response(),
        },
        _ => StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn profile(st: S, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let Ok(uuid) = uuid::Uuid::parse_str(&id) else {
        return StatusCode::NO_CONTENT.into_response();
    };
    match api::user_by(&st, "uuid", &uuid.hyphenated().to_string()) {
        Some(u) => Json(profile_json(&st, &u, q.get("unsigned").map(String::as_str) == Some("false"))).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

pub async fn profiles_by_names(st: S, Json(names): Json<Vec<String>>) -> Json<Value> {
    let out: Vec<Value> = names.iter().take(10).filter_map(|n| api::user_by(&st, "name", n)).map(|u| short(&u)).collect();
    Json(json!(out))
}
