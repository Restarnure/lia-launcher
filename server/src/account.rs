//! Акаунт ЛІЯ ID: реєстрація з підтвердженням пошти, вхід із захистом від підбору, відновлення
//! й зміна пароля, зміна пошти, пристрої (сесії), видалення акаунта.
use crate::api::{self, User};
use crate::ygg::hex;
use crate::{bad, forbidden, internal, mail, not_found, ApiErr, App, R, S};
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::time::Duration;

const CODE_TTL: i64 = 15 * 60_000;
const LOCK_AFTER: i64 = 8;
const LOCK_FOR: i64 = 15 * 60_000;
/// Сесія лаунчера без використання довше — недійсна.
pub const SESSION_IDLE: i64 = 90 * 24 * 3600_000;

// ---------------------------------------------------------------- паролі

/// Найпоширеніші паролі (з публічних списків витоків) — їх не приймаємо.
const COMMON: &[&str] = &[
    "12345678", "123456789", "1234567890", "password", "password1", "qwertyui", "qwerty123", "11111111", "00000000", "abcdefgh", "iloveyou", "minecraft",
    "minecraft1", "1q2w3e4r", "1qaz2wsx", "zaq12wsx", "qwertyuiop", "asdfghjk", "88888888", "12341234", "123123123", "87654321", "passw0rd", "superman",
    "football", "baseball", "sunshine", "princess", "dragon12", "11223344", "йцукенгш", "пароль123",
];

pub fn hash_password(pass: &str) -> R<String> {
    let salt: [u8; 16] = rand::random();
    let mut out = [0u8; 32];
    argon2::Argon2::default().hash_password_into(pass.as_bytes(), &salt, &mut out).map_err(internal)?;
    Ok(format!("argon2:{}:{}", hex(&salt), hex(&out)))
}

/// Порівняння за сталий час — щоб час відповіді не підказував збіг.
fn eq_ct(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn verify_password(pass: &str, stored: &str) -> bool {
    let mut it = stored.split(':');
    let (Some("argon2"), Some(salt), Some(hash)) = (it.next(), it.next(), it.next()) else {
        return false;
    };
    let salt: Vec<u8> = (0..salt.len()).step_by(2).filter_map(|i| u8::from_str_radix(salt.get(i..i + 2)?, 16).ok()).collect();
    let mut out = [0u8; 32];
    argon2::Argon2::default().hash_password_into(pass.as_bytes(), &salt, &mut out).is_ok() && eq_ct(hex(&out).as_bytes(), hash.as_bytes())
}

pub fn valid_password(p: &str, name: &str) -> R<()> {
    let n = p.chars().count();
    if !(8..=128).contains(&n) {
        return Err(bad("пароль: від 8 до 128 символів"));
    }
    let low = p.to_lowercase();
    if low.contains(&name.to_lowercase()) {
        return Err(bad("пароль не може містити нік"));
    }
    if COMMON.contains(&low.as_str()) || p.chars().all(|c| Some(c) == p.chars().next()) {
        return Err(bad("цей пароль надто простий, його вгадають першим"));
    }
    Ok(())
}

pub fn valid_name(n: &str) -> bool {
    (3..=16).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Ніки, що вводили б в оману (видавали себе за ЛІЮ чи адміністрацію).
fn reserved(n: &str) -> bool {
    let n = n.to_lowercase();
    ["admin", "administrator", "lia", "lia_id", "liaid", "support", "moderator", "system", "starnure", "mojang", "microsoft"].contains(&n.as_str())
}

pub fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else { return false };
    e.len() <= 254 && !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !e.chars().any(|c| c.is_whitespace() || c == '<' || c == '>') && !domain.contains('@')
}

// ---------------------------------------------------------------- коди з пошти

fn code_hash(user: i64, code: &str) -> String {
    hex(&Sha1::digest(format!("{user}:{code}").as_bytes()))
}

/// Новий 6-значний код для дії `purpose` (verify / email / reset). Старий код тієї ж дії анулюється.
fn new_code(app: &App, user: i64, purpose: &str, email: &str) -> R<String> {
    let code = format!("{:06}", rand::random::<u32>() % 1_000_000);
    app.db
        .lock()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO codes (user, purpose, hash, email, expires, attempts) VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![user, purpose, code_hash(user, &code), email, crate::now() + CODE_TTL],
        )
        .map_err(internal)?;
    Ok(code)
}

/// Перевіряє код; повертає пошту, до якої він був надісланий. Не більше 5 спроб на код.
fn take_code(app: &App, user: i64, purpose: &str, code: &str) -> R<String> {
    let db = app.db.lock().unwrap();
    let row: Option<(String, String, i64, i64)> = db
        .query_row("SELECT hash, email, expires, attempts FROM codes WHERE user = ?1 AND purpose = ?2", params![user, purpose], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .optional()
        .map_err(internal)?;
    let Some((hash, email, expires, attempts)) = row else {
        return Err(bad("код неправильний або застарів, надішліть новий"));
    };
    if expires < crate::now() || attempts >= 5 {
        db.execute("DELETE FROM codes WHERE user = ?1 AND purpose = ?2", params![user, purpose]).map_err(internal)?;
        return Err(bad("код застарів або забагато спроб, надішліть новий"));
    }
    if !eq_ct(hash.as_bytes(), code_hash(user, code.trim()).as_bytes()) {
        db.execute("UPDATE codes SET attempts = attempts + 1 WHERE user = ?1 AND purpose = ?2", params![user, purpose]).map_err(internal)?;
        return Err(bad("неправильний код"));
    }
    db.execute("DELETE FROM codes WHERE user = ?1 AND purpose = ?2", params![user, purpose]).map_err(internal)?;
    Ok(email)
}

// ---------------------------------------------------------------- сесії

pub fn token_from(h: &HeaderMap) -> &str {
    h.get(axum::http::header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("")
}

fn device_name(d: Option<&str>) -> String {
    let d: String = d.unwrap_or("").chars().filter(|c| !c.is_control()).take(48).collect();
    if d.trim().is_empty() {
        "Невідомий пристрій".into()
    } else {
        d.trim().to_string()
    }
}

/// Нова сесія лаунчера (токен акаунта, без ігрового профілю).
fn new_session(app: &App, u: &User, device: &str, ip: &str) -> R<Value> {
    let token = hex(&rand::random::<[u8; 32]>());
    let now = crate::now();
    app.db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO tokens (hash, user, client, created, device, ip, last_used) VALUES (?1, ?2, '', ?3, ?4, ?5, ?3)",
            params![api::token_hash(&token), u.id, now, device, ip],
        )
        .map_err(internal)?;
    Ok(json!({ "accessToken": token, "name": u.name, "emailVerified": u.verified }))
}

// ---------------------------------------------------------------- реєстрація й вхід

#[derive(Deserialize)]
pub struct RegisterReq {
    name: String,
    email: String,
    password: String,
    device: Option<String>,
}

pub async fn register(st: S, h: HeaderMap, Json(r): Json<RegisterReq>) -> R<Json<Value>> {
    if !valid_name(&r.name) {
        return Err(bad("нік: 3–16 символів, латинські літери, цифри та _"));
    }
    if reserved(&r.name) {
        return Err(bad("цей нік зарезервовано"));
    }
    let email = r.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(bad("перевірте адресу пошти"));
    }
    valid_password(&r.password, &r.name)?;
    let ip = crate::client_ip(&h);
    st.limit(format!("reg:{ip}"), 5, Duration::from_secs(3600))?;
    let pass = hash_password(&r.password)?;
    let uuid = uuid::Uuid::new_v4().hyphenated().to_string();
    let verified = !mail::enabled();
    let res = st.db.lock().unwrap().execute(
        "INSERT INTO users (name, uuid, pass, created, email, email_verified) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![r.name, uuid, pass, crate::now(), email, verified as i64],
    );
    if let Err(e) = res {
        if matches!(e, rusqlite::Error::SqliteFailure(ref f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation) {
            let taken_email = st.db.lock().unwrap().query_row("SELECT 1 FROM users WHERE email = ?1", [&email], |_| Ok(())).is_ok();
            return Err(ApiErr(StatusCode::CONFLICT, "Taken", if taken_email { "ця пошта вже прив'язана до іншого ЛІЯ ID" } else { "цей нік уже зайнятий" }.into()));
        }
        return Err(internal(e));
    }
    let u = api::user_by(&st, "uuid", &uuid).ok_or_else(not_found)?;
    if !verified {
        let code = new_code(&st, u.id, "verify", &email)?;
        mail::code(&st, &email, "Підтвердження пошти ЛІЯ ID", &format!("Вітаємо в ЛІЇ, {}! Введіть цей код у лаунчері, щоб підтвердити пошту.", u.name), &code);
    }
    Ok(Json(new_session(&st, &u, &device_name(r.device.as_deref()), &ip)?))
}

#[derive(Deserialize)]
pub struct LoginReq {
    /// Нік або пошта.
    name: String,
    password: String,
    device: Option<String>,
}

/// Хеш, з яким звіряємо пароль неіснуючого акаунта, — щоб час відповіді був однаковий.
const DUMMY: &str = "argon2:00000000000000000000000000000000:0000000000000000000000000000000000000000000000000000000000000000";

pub async fn login(st: S, h: HeaderMap, Json(r): Json<LoginReq>) -> R<Json<Value>> {
    let ip = crate::client_ip(&h);
    st.limit(format!("auth:{ip}"), 20, Duration::from_secs(600))?;
    let login = r.name.trim();
    let row: Option<(i64, String, i64)> = st
        .db
        .lock()
        .unwrap()
        .query_row("SELECT id, pass, locked_until FROM users WHERE name = ?1 OR email = ?2", params![login, login.to_lowercase()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()
        .map_err(internal)?;
    let now = crate::now();
    let Some((id, pass, locked_until)) = row else {
        verify_password(&r.password, DUMMY);
        return Err(forbidden("Неправильний нік або пароль."));
    };
    if locked_until > now {
        let mins = (locked_until - now) / 60_000 + 1;
        return Err(ApiErr(StatusCode::TOO_MANY_REQUESTS, "Locked", format!("забагато невдалих спроб, вхід закрито ще на {mins} хв")));
    }
    // Спроба рахується до перевірки пароля й атомарно в базі: паралельні запити не дадуть більше LOCK_AFTER спроб.
    let n: Option<i64> = st
        .db
        .lock()
        .unwrap()
        .query_row("UPDATE users SET failed = failed + 1 WHERE id = ?1 AND locked_until <= ?2 RETURNING failed", params![id, now], |r| r.get(0))
        .optional()
        .map_err(internal)?;
    let n = n.filter(|n| *n <= LOCK_AFTER).ok_or_else(|| ApiErr(StatusCode::TOO_MANY_REQUESTS, "Locked", "забагато невдалих спроб, вхід закрито на 15 хв".into()))?;
    if !verify_password(&r.password, &pass) {
        if n == LOCK_AFTER {
            let db = st.db.lock().unwrap();
            db.execute("UPDATE users SET failed = 0, locked_until = ?1 WHERE id = ?2", params![now + LOCK_FOR, id]).map_err(internal)?;
            drop(db);
            // Лист не частіше разу на добу (або після успішного входу), бо інакше будь-хто, знаючи нік,
            // засипав би людину листами й вичерпав ліміт поштового сервісу.
            if let Some(u) = api::user_by_id(&st, id).filter(|u| u.verified && locked_until < now - 24 * 3600_000) {
                if let Some(e) = &u.email {
                    mail::send(&st, e, "Хтось намагається увійти у ваш ЛІЯ ID", &[format!("{}, у ваш ЛІЯ ID кілька разів ввели неправильний пароль. Вхід закрито на 15 хвилин.", u.name), "Якщо це були не ви, змініть пароль у лаунчері.".into()]);
                }
            }
        }
        return Err(forbidden("Неправильний нік або пароль."));
    }
    st.db.lock().unwrap().execute("UPDATE users SET failed = 0, locked_until = 0 WHERE id = ?1", [id]).map_err(internal)?;
    let u = api::user_by_id(&st, id).ok_or_else(not_found)?;
    let device = device_name(r.device.as_deref());
    // Лист про вхід з пристрою, з якого раніше не входили.
    let known: bool = st.db.lock().unwrap().query_row("SELECT 1 FROM tokens WHERE user = ?1 AND device = ?2 AND profile IS NULL", params![id, device], |_| Ok(())).is_ok();
    if !known && u.verified {
        if let Some(e) = &u.email {
            mail::send(&st, e, "Новий вхід у ЛІЯ ID", &[format!("{}, у ваш ЛІЯ ID увійшли з пристрою «{device}».", u.name), "Якщо це були не ви, змініть пароль і вийдіть з усіх пристроїв у лаунчері (Акаунти → ЛІЯ ID).".into()]);
        }
    }
    Ok(Json(new_session(&st, &u, &device, &ip)?))
}

// ---------------------------------------------------------------- пошта

pub async fn me(st: S, h: HeaderMap) -> R<Json<Value>> {
    let u = api::auth_any(&st, &h)?;
    let created: i64 = st.db.lock().unwrap().query_row("SELECT created FROM users WHERE id = ?1", [u.id], |r| r.get(0)).map_err(internal)?;
    Ok(Json(json!({
        "name": u.name,
        "uuid": u.uuid,
        "email": u.email,
        "emailVerified": u.verified,
        "mailEnabled": mail::enabled(),
        "created": created,
        "profiles": crate::ygg::profiles_of(&st, u.id),
    })))
}

#[derive(Deserialize)]
pub struct CodeReq {
    code: String,
}

/// Підтвердження пошти: і першої (після реєстрації), і нової (після зміни).
pub async fn verify_email(st: S, h: HeaderMap, Json(r): Json<CodeReq>) -> R<Json<Value>> {
    let u = api::auth_any(&st, &h)?;
    st.limit(format!("code:{}", u.id), 20, Duration::from_secs(3600))?;
    if let Ok(email) = take_code(&st, u.id, "email", &r.code) {
        let old = u.email.clone();
        let res = st.db.lock().unwrap().execute("UPDATE users SET email = ?1, email_verified = 1 WHERE id = ?2", params![email, u.id]);
        if res.is_err() {
            return Err(ApiErr(StatusCode::CONFLICT, "Taken", "ця пошта вже прив'язана до іншого ЛІЯ ID".into()));
        }
        if let Some(o) = old.filter(|o| *o != email) {
            mail::send(&st, &o, "Пошту ЛІЯ ID змінено", &[format!("{}, пошту вашого ЛІЯ ID змінено на {email}.", u.name), "Якщо це були не ви, відновіть доступ через «Забули пароль?» і напишіть нам.".into()]);
        }
    } else {
        take_code(&st, u.id, "verify", &r.code)?;
        st.db.lock().unwrap().execute("UPDATE users SET email_verified = 1 WHERE id = ?1", [u.id]).map_err(internal)?;
    }
    Ok(Json(json!({ "emailVerified": true })))
}

pub async fn resend_code(st: S, h: HeaderMap) -> R<StatusCode> {
    let u = api::auth_any(&st, &h)?;
    st.limit(format!("mail:{}", u.id), 5, Duration::from_secs(3600))?;
    // Спершу незавершена зміна пошти, інакше — підтвердження поточної.
    let pending: Option<String> = st.db.lock().unwrap().query_row("SELECT email FROM codes WHERE user = ?1 AND purpose = 'email'", [u.id], |r| r.get(0)).optional().map_err(internal)?;
    match (pending, &u.email) {
        (Some(e), _) => {
            let code = new_code(&st, u.id, "email", &e)?;
            mail::code(&st, &e, "Нова пошта ЛІЯ ID", &format!("{}, підтвердіть нову пошту для ЛІЯ ID.", u.name), &code);
        }
        (None, Some(e)) if !u.verified => {
            let code = new_code(&st, u.id, "verify", e)?;
            mail::code(&st, e, "Підтвердження пошти ЛІЯ ID", &format!("{}, введіть цей код у лаунчері.", u.name), &code);
        }
        _ => return Err(bad("нема чого підтверджувати")),
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct EmailReq {
    email: String,
    password: String,
}

/// Нова пошта (або перша — для акаунтів без пошти): код на нову адресу, змінюється після підтвердження.
pub async fn change_email(st: S, h: HeaderMap, Json(r): Json<EmailReq>) -> R<StatusCode> {
    let u = api::auth_any(&st, &h)?;
    st.limit(format!("mail:{}", u.id), 5, Duration::from_secs(3600))?;
    api::check_password(&st, &u.name, &r.password)?;
    let email = r.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(bad("перевірте адресу пошти"));
    }
    let taken = st.db.lock().unwrap().query_row("SELECT 1 FROM users WHERE email = ?1 AND id != ?2", params![email, u.id], |_| Ok(())).is_ok();
    if taken {
        return Err(ApiErr(StatusCode::CONFLICT, "Taken", "ця пошта вже прив'язана до іншого ЛІЯ ID".into()));
    }
    if !mail::enabled() {
        st.db.lock().unwrap().execute("UPDATE users SET email = ?1, email_verified = 1 WHERE id = ?2", params![email, u.id]).map_err(internal)?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let code = new_code(&st, u.id, "email", &email)?;
    mail::code(&st, &email, "Нова пошта ЛІЯ ID", &format!("{}, підтвердіть цю пошту для ЛІЯ ID.", u.name), &code);
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- пароль

#[derive(Deserialize)]
pub struct ForgotReq {
    login: String,
}

/// Код для скидання пароля. Відповідь однакова, чи є такий акаунт, — щоб не підказувати чужі ніки й пошти.
pub async fn forgot(st: S, h: HeaderMap, Json(r): Json<ForgotReq>) -> R<StatusCode> {
    st.limit(format!("forgot:{}", crate::client_ip(&h)), 10, Duration::from_secs(3600))?;
    if !mail::enabled() {
        return Err(ApiErr(StatusCode::SERVICE_UNAVAILABLE, "NoMail", "відновлення через пошту поки не працює".into()));
    }
    let login = r.login.trim();
    let u = st.db.lock().unwrap().query_row("SELECT id FROM users WHERE (name = ?1 OR email = ?2) AND email_verified = 1", params![login, login.to_lowercase()], |r| r.get::<_, i64>(0)).optional().map_err(internal)?;
    if let Some(u) = u.and_then(|id| api::user_by_id(&st, id)) {
        if st.limit(format!("mail:{}", u.id), 5, Duration::from_secs(3600)).is_ok() {
            if let Some(e) = &u.email {
                let code = new_code(&st, u.id, "reset", e)?;
                mail::code(&st, e, "Відновлення пароля ЛІЯ ID", &format!("{}, хтось попросив скинути пароль вашого ЛІЯ ID. Якщо це ви, введіть код у лаунчері.", u.name), &code);
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ResetReq {
    login: String,
    code: String,
    password: String,
}

pub async fn reset(st: S, h: HeaderMap, Json(r): Json<ResetReq>) -> R<StatusCode> {
    st.limit(format!("reset:{}", crate::client_ip(&h)), 20, Duration::from_secs(3600))?;
    let login = r.login.trim();
    // Пароль перевіряємо до коду (щоб слабкий пароль не «спалив» правильний код) і до пошуку акаунта
    // (щоб відповідь не підказувала, чи такий акаунт є).
    valid_password(&r.password, login)?;
    let id = st.db.lock().unwrap().query_row("SELECT id FROM users WHERE name = ?1 OR email = ?2", params![login, login.to_lowercase()], |r| r.get::<_, i64>(0)).optional().map_err(internal)?;
    let u = id.and_then(|id| api::user_by_id(&st, id)).ok_or_else(|| bad("код неправильний або застарів, надішліть новий"))?;
    take_code(&st, u.id, "reset", &r.code)?;
    valid_password(&r.password, &u.name)?;
    let pass = hash_password(&r.password)?;
    {
        let db = st.db.lock().unwrap();
        db.execute("UPDATE users SET pass = ?1, failed = 0, locked_until = 0 WHERE id = ?2", params![pass, u.id]).map_err(internal)?;
        // Новий пароль — вихід з усіх пристроїв і з гри.
        db.execute("DELETE FROM tokens WHERE user = ?1", [u.id]).map_err(internal)?;
    }
    if let Some(e) = &u.email {
        mail::send(&st, e, "Пароль ЛІЯ ID змінено", &[format!("{}, пароль вашого ЛІЯ ID скинуто через пошту. Усі пристрої вийшли з акаунта.", u.name)]);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PasswordReq {
    old: String,
    new: String,
}

pub async fn change_password(st: S, h: HeaderMap, Json(r): Json<PasswordReq>) -> R<StatusCode> {
    let u = api::auth_any(&st, &h)?;
    st.limit(format!("auth:{}", crate::client_ip(&h)), 20, Duration::from_secs(600))?;
    api::check_password(&st, &u.name, &r.old)?;
    valid_password(&r.new, &u.name)?;
    let pass = hash_password(&r.new)?;
    {
        let db = st.db.lock().unwrap();
        db.execute("UPDATE users SET pass = ?1 WHERE id = ?2", params![pass, u.id]).map_err(internal)?;
        // Інші пристрої виходять; цей лишається.
        db.execute("DELETE FROM tokens WHERE user = ?1 AND hash != ?2", params![u.id, api::token_hash(token_from(&h))]).map_err(internal)?;
    }
    if let Some(e) = u.email.as_ref().filter(|_| u.verified) {
        mail::send(&st, e, "Пароль ЛІЯ ID змінено", &[format!("{}, пароль вашого ЛІЯ ID змінено. Інші пристрої вийшли з акаунта.", u.name), "Якщо це були не ви, відновіть доступ через «Забули пароль?».".into()]);
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- пристрої

pub async fn sessions(st: S, h: HeaderMap) -> R<Json<Value>> {
    let u = api::auth_any(&st, &h)?;
    let me = api::token_hash(token_from(&h));
    let db = st.db.lock().unwrap();
    let mut q = db.prepare("SELECT hash, device, created, last_used FROM tokens WHERE user = ?1 AND profile IS NULL ORDER BY last_used DESC").map_err(internal)?;
    let list: Vec<Value> = q
        .query_map([u.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<i64>>(3)?)))
        .map_err(internal)?
        .flatten()
        .map(|(hash, device, created, used)| {
            json!({ "id": &hash[..12], "device": device.unwrap_or_else(|| "Невідомий пристрій".into()), "created": created, "lastUsed": used.unwrap_or(created), "current": hash == me })
        })
        .collect();
    Ok(Json(json!(list)))
}

pub async fn revoke_session(st: S, h: HeaderMap, Path(id): Path<String>) -> R<StatusCode> {
    let u = api::auth_any(&st, &h)?;
    if id.len() != 12 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(not_found());
    }
    st.db.lock().unwrap().execute("DELETE FROM tokens WHERE user = ?1 AND profile IS NULL AND substr(hash, 1, 12) = ?2", params![u.id, id]).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Вийти з усіх інших пристроїв (і з ігрових сесій — гра попросить увійти знову).
pub async fn revoke_others(st: S, h: HeaderMap) -> R<StatusCode> {
    let u = api::auth_any(&st, &h)?;
    st.db.lock().unwrap().execute("DELETE FROM tokens WHERE user = ?1 AND hash != ?2", params![u.id, api::token_hash(token_from(&h))]).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn logout(st: S, h: HeaderMap) -> StatusCode {
    let _ = st.db.lock().unwrap().execute("DELETE FROM tokens WHERE hash = ?1 AND profile IS NULL", [api::token_hash(token_from(&h))]);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct DeleteReq {
    password: String,
}

/// Видалити акаунт назавжди: друзі, збірки, ігрові профілі, сесії.
pub async fn delete_account(st: S, h: HeaderMap, Json(r): Json<DeleteReq>) -> R<StatusCode> {
    let u = api::auth_any(&st, &h)?;
    st.limit(format!("auth:{}", crate::client_ip(&h)), 20, Duration::from_secs(600))?;
    api::check_password(&st, &u.name, &r.password)?;
    let packs: Vec<String> = {
        let db = st.db.lock().unwrap();
        let mut q = db.prepare("SELECT id FROM packs WHERE owner = ?1").map_err(internal)?;
        let v = q.query_map([u.id], |r| r.get(0)).map_err(internal)?.flatten().collect();
        v
    };
    st.db.lock().unwrap().execute("DELETE FROM users WHERE id = ?1", [u.id]).map_err(internal)?;
    for p in packs {
        let _ = std::fs::remove_file(st.cfg.data.join("packs").join(format!("{p}.mrpack")));
    }
    st.presence.lock().unwrap().remove(&u.id);
    if let Some(e) = u.email.as_ref().filter(|_| u.verified) {
        mail::send(&st, e, "ЛІЯ ID видалено", &[format!("{}, ваш ЛІЯ ID і всі його дані видалено.", u.name)]);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords() {
        assert!(valid_password("short", "Bob").is_err());
        assert!(valid_password("minecraft", "Bob").is_err());
        assert!(valid_password("xxBobxx123", "bob").is_err());
        assert!(valid_password("aaaaaaaaaa", "Bob").is_err());
        assert!(valid_password("зелений-кіт-42", "Bob").is_ok());
        let h = hash_password("зелений-кіт-42").unwrap();
        assert!(verify_password("зелений-кіт-42", &h));
        assert!(!verify_password("зелений-кіт-43", &h));
        assert!(!verify_password("x", DUMMY));
    }

    #[test]
    fn emails() {
        assert!(valid_email("a@b.ua"));
        assert!(!valid_email("a@b"));
        assert!(!valid_email("a b@c.ua"));
        assert!(!valid_email("@c.ua"));
        assert!(!valid_email("a@@c.ua"));
    }
}
