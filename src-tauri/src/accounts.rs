use crate::crypto;
use crate::error::{err, Context, Result};
use crate::paths::Paths;
use crate::util;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Digest as _;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MsTokens {
    pub refresh_token: String,
    pub access_token: String,
    /// Коли спливає токен Minecraft (мс).
    pub expires_at: i64,
    #[serde(default)]
    pub xuid: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// "microsoft" | "offline" | "lia" (ЛІЯ ID)
    pub kind: String,
    pub name: String,
    /// UUID без дефісів.
    pub uuid: String,
    pub added: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<MsTokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lia: Option<crate::lia::LiaTokens>,
}

/// Те, що бачить інтерфейс: без токенів.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub uuid: String,
    pub added: i64,
}

impl Account {
    pub fn view(&self) -> AccountView {
        AccountView { id: self.id.clone(), kind: self.kind.clone(), name: self.name.clone(), uuid: self.uuid.clone(), added: self.added }
    }
}

fn store_path() -> std::path::PathBuf {
    Paths::config_home().join("accounts.dat")
}

pub fn load() -> Vec<Account> {
    std::fs::read(store_path())
        .ok()
        .and_then(|b| crypto::open(&b).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(accounts: &[Account]) -> Result<()> {
    let sealed = crypto::seal(&serde_json::to_vec(accounts)?)?;
    util::write_atomic(&store_path(), &sealed)
}

// ---------------------------------------------------------------- офлайн

/// Нік як у Minecraft: 3–16 символів, латиниця, цифри, підкреслення.
pub fn validate_nick(name: &str) -> Result<String> {
    let name = name.trim();
    let ok = (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(err("invalid_nick", "нік: 3–16 символів, латиниця, цифри або _"));
    }
    Ok(name.to_string())
}

/// UUID, який сервер в офлайн-режимі видає гравцю з цим ніком (`UUID.nameUUIDFromBytes`).
pub fn offline_uuid(name: &str) -> String {
    let mut h: [u8; 16] = md5::Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    hex::encode(h)
}

pub fn offline(name: &str) -> Result<Account> {
    let name = validate_nick(name)?;
    let uuid = offline_uuid(&name);
    Ok(Account { id: format!("offline-{uuid}"), kind: "offline".into(), name, uuid, added: util::now_ms(), ms: None, lia: None })
}

pub fn dashed(uuid: &str) -> String {
    if uuid.len() != 32 {
        return uuid.to_string();
    }
    format!("{}-{}-{}-{}-{}", &uuid[..8], &uuid[8..12], &uuid[12..16], &uuid[16..20], &uuid[20..])
}

// ---------------------------------------------------------------- Microsoft

pub const MS_REDIRECT: &str = "https://login.microsoftonline.com/common/oauth2/nativeclient";
const MS_AUTHORIZE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const MS_TOKEN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";

pub struct Pkce {
    pub verifier: String,
    pub url: String,
}

/// `lang` — мова сторінки входу Microsoft ("uk" | "en"), інакше вона береться з мови Windows.
pub fn authorize_url(client_id: &str, lang: &str) -> Pkce {
    use rand::RngExt;
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::rng();
    let verifier: String = (0..64).map(|_| ALPHA[rng.random_range(0..ALPHA.len())] as char).collect();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse(MS_AUTHORIZE).unwrap();
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", MS_REDIRECT)
        .append_pair("scope", SCOPE)
        .append_pair("prompt", "select_account")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("ui_locales", if lang == "en" { "en-US" } else { "uk-UA" })
        .append_pair("mkt", if lang == "en" { "en-US" } else { "uk-UA" });
    Pkce { verifier, url: url.to_string() }
}

#[derive(Deserialize)]
struct OAuth {
    access_token: String,
    refresh_token: String,
}

async fn oauth(http: &reqwest::Client, form: &[(&str, &str)]) -> Result<OAuth> {
    let resp = http.post(MS_TOKEN).form(form).send().await.ctx(|| "login.microsoftonline.com".into())?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    if !status.is_success() {
        let code = body["error"].as_str().unwrap_or("");
        let desc = body["error_description"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
        return Err(if code == "invalid_grant" { err("ms_relogin", desc) } else { err("ms_oauth", format!("{code}: {desc}")) });
    }
    Ok(serde_json::from_value(body)?)
}

pub async fn exchange_code(http: &reqwest::Client, client_id: &str, code: &str, verifier: &str) -> Result<(String, String)> {
    let o = oauth(
        http,
        &[
            ("client_id", client_id),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", MS_REDIRECT),
            ("code_verifier", verifier),
            ("scope", SCOPE),
        ],
    )
    .await?;
    Ok((o.access_token, o.refresh_token))
}

async fn refresh_ms(http: &reqwest::Client, client_id: &str, refresh: &str) -> Result<(String, String)> {
    let o = oauth(http, &[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", refresh), ("scope", SCOPE)]).await?;
    Ok((o.access_token, o.refresh_token))
}

async fn post_json(http: &reqwest::Client, url: &str, body: serde_json::Value) -> Result<(u16, serde_json::Value)> {
    let resp = http.post(url).header("Accept", "application/json").json(&body).send().await.ctx(|| url.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))))
}

/// Xbox Live → XSTS → токен Minecraft. Повертає (токен, секунд дії, xuid).
pub async fn minecraft_token(http: &reqwest::Client, ms_access: &str) -> Result<(String, i64, String)> {
    let (s, xbl) = post_json(
        http,
        "https://user.auth.xboxlive.com/user/authenticate",
        json!({"Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={ms_access}")},
               "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT"}),
    )
    .await?;
    let xbl_token = xbl["Token"].as_str().filter(|_| s == 200).ok_or_else(|| err("ms_xbox", format!("Xbox Live: HTTP {s}")))?.to_string();

    let xsts = |rp: &'static str| {
        let t = xbl_token.clone();
        async move {
            post_json(
                http,
                "https://xsts.auth.xboxlive.com/xsts/authorize",
                json!({"Properties": {"SandboxId": "RETAIL", "UserTokens": [t]}, "RelyingParty": rp, "TokenType": "JWT"}),
            )
            .await
        }
    };
    let (s, x) = xsts("rp://api.minecraftservices.com/").await?;
    if s != 200 {
        let code = x["XErr"].as_u64().unwrap_or(0);
        return Err(match code {
            2148916233 => err("ms_no_xbox", "у акаунта Microsoft немає профілю Xbox"),
            2148916235 => err("ms_region", "Xbox Live недоступний у вашій країні"),
            2148916236 | 2148916237 => err("ms_adult_verify", "потрібне підтвердження віку на xbox.com"),
            2148916238 => err("ms_child", "дитячий акаунт: дорослий має додати його до сім'ї Microsoft"),
            _ => err("ms_xbox", format!("XSTS: HTTP {s}, XErr {code}")),
        });
    }
    let uhs = x["DisplayClaims"]["xui"][0]["uhs"].as_str().unwrap_or("").to_string();
    let xsts_token = x["Token"].as_str().unwrap_or("").to_string();

    let (s, mc) = post_json(
        http,
        "https://api.minecraftservices.com/authentication/login_with_xbox",
        json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") }),
    )
    .await?;
    if s == 403 || s == 401 {
        return Err(err("ms_app_not_approved", format!("Minecraft Services відхилив вхід (HTTP {s}): {}", mc)));
    }
    let token = mc["access_token"].as_str().filter(|_| s == 200).ok_or_else(|| err("ms_mc", format!("Minecraft Services: HTTP {s}")))?.to_string();
    let expires = mc["expires_in"].as_i64().unwrap_or(86_400);

    // xuid потрібен грі лише для телеметрії/скарг на чат — не критично, якщо не вдалось.
    let xuid = match xsts("http://xboxlive.com").await {
        Ok((200, v)) => v["DisplayClaims"]["xui"][0]["xid"].as_str().unwrap_or("0").to_string(),
        _ => "0".into(),
    };
    Ok((token, expires, xuid))
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<Texture>,
    #[serde(default)]
    pub capes: Vec<Texture>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Texture {
    pub id: String,
    pub state: String,
    pub url: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
}

pub async fn profile(http: &reqwest::Client, mc_token: &str) -> Result<Profile> {
    let resp = http.get("https://api.minecraftservices.com/minecraft/profile").bearer_auth(mc_token).send().await?;
    match resp.status().as_u16() {
        200 => Ok(resp.json().await?),
        404 => Err(err("ms_no_game", "на цьому акаунті немає Minecraft: Java Edition")),
        401 => Err(err("ms_relogin", "сесія Minecraft застаріла")),
        s => Err(err("ms_mc", format!("профіль Minecraft: HTTP {s}"))),
    }
}

/// Повний вхід за кодом авторизації з вікна Microsoft.
pub async fn login_with_code(http: &reqwest::Client, client_id: &str, code: &str, verifier: &str) -> Result<Account> {
    let (ms_access, refresh) = exchange_code(http, client_id, code, verifier).await?;
    finish_login(http, ms_access, refresh).await
}

async fn finish_login(http: &reqwest::Client, ms_access: String, refresh: String) -> Result<Account> {
    let (token, expires, xuid) = minecraft_token(http, &ms_access).await?;
    let p = profile(http, &token).await?;
    Ok(Account {
        id: format!("ms-{}", p.id),
        kind: "microsoft".into(),
        name: p.name,
        uuid: p.id,
        added: util::now_ms(),
        ms: Some(MsTokens { refresh_token: refresh, access_token: token, expires_at: util::now_ms() + expires * 1000, xuid }),
        lia: None,
    })
}

/// Оновлює токени, якщо до кінця їхньої дії менше 10 хвилин. Повертає true, якщо щось змінилось.
pub async fn ensure_fresh(http: &reqwest::Client, client_id: &str, acc: &mut Account) -> Result<bool> {
    let Some(ms) = &acc.ms else { return Ok(false) };
    if ms.expires_at - util::now_ms() > 10 * 60_000 {
        return Ok(false);
    }
    if client_id.is_empty() {
        return Err(err("ms_no_client_id", "не вказано Client ID для входу Microsoft"));
    }
    let (ms_access, refresh) = refresh_ms(http, client_id, &ms.refresh_token).await?;
    let fresh = finish_login(http, ms_access, refresh).await?;
    acc.name = fresh.name;
    acc.ms = fresh.ms;
    Ok(true)
}

/// Завантажує скін на акаунт. Повертає оновлений профіль (з новою адресою текстури).
pub async fn upload_skin(http: &reqwest::Client, mc_token: &str, png: Vec<u8>, slim: bool) -> Result<Profile> {
    let part = reqwest::multipart::Part::bytes(png).file_name("skin.png").mime_str("image/png")?;
    let form = reqwest::multipart::Form::new().text("variant", if slim { "slim" } else { "classic" }).part("file", part);
    let resp = http.post("https://api.minecraftservices.com/minecraft/profile/skins").bearer_auth(mc_token).multipart(form).send().await?;
    let s = resp.status().as_u16();
    if s == 429 {
        return Err(err("skin_rate", "забагато змін скіна поспіль — спробуйте за хвилину"));
    }
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(err("skin_upload", format!("HTTP {s}: {}", body.chars().take(200).collect::<String>())));
    }
    match resp.json::<Profile>().await {
        Ok(p) if !p.skins.is_empty() => Ok(p),
        _ => profile(http, mc_token).await,
    }
}

pub async fn set_cape(http: &reqwest::Client, mc_token: &str, cape_id: Option<&str>) -> Result<()> {
    let url = "https://api.minecraftservices.com/minecraft/profile/capes/active";
    let resp = match cape_id {
        Some(id) => http.put(url).bearer_auth(mc_token).json(&json!({ "capeId": id })).send().await?,
        None => http.delete(url).bearer_auth(mc_token).send().await?,
    };
    if !resp.status().is_success() {
        return Err(err("cape", format!("HTTP {}", resp.status().as_u16())));
    }
    Ok(())
}

/// Скін гравця за ніком через публічний API Mojang (для офлайн-акаунтів: так їх
/// показують сервери зі SkinsRestorer). Повертає (url, slim).
pub async fn skin_by_name(http: &reqwest::Client, name: &str) -> Result<Option<(String, bool)>> {
    let resp = http.get(format!("https://api.mojang.com/users/profiles/minecraft/{name}")).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let v: serde_json::Value = resp.json().await?;
    let Some(id) = v["id"].as_str() else { return Ok(None) };
    let p: serde_json::Value = crate::net::get_json(http, &format!("https://sessionserver.mojang.com/session/minecraft/profile/{id}")).await?;
    let Some(b64) = p["properties"].as_array().and_then(|a| a.iter().find(|x| x["name"] == "textures")).and_then(|x| x["value"].as_str()) else {
        return Ok(None);
    };
    let decoded = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| err("skin", e.to_string()))?;
    let t: serde_json::Value = serde_json::from_slice(&decoded)?;
    let skin = &t["textures"]["SKIN"];
    Ok(skin["url"].as_str().map(|u| (u.replace("http://", "https://"), skin["metadata"]["model"] == "slim")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java() {
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(dashed(&offline_uuid("Notch")), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }

    #[test]
    fn nicks() {
        assert!(validate_nick("Steve_2").is_ok());
        assert!(validate_nick("ab").is_err());
        assert!(validate_nick("Стів").is_err());
        assert!(validate_nick("a_very_long_name_17").is_err());
    }

    #[test]
    fn pkce_url() {
        let p = authorize_url("abc", "uk");
        assert_eq!(p.verifier.len(), 64);
        assert!(p.url.contains("client_id=abc"));
        assert!(p.url.contains("code_challenge_method=S256"));
        assert!(p.url.contains("nativeclient"));
        assert!(p.url.contains("ui_locales=uk-UA"));
    }
}
