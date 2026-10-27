//! ЛІЯ ID і соцмережа: вхід (Yggdrasil-сумісний сервер ЛІЇ), authlib-injector для гри,
//! ретранслятор LAN-світів з боку хоста, експорт збірки в .mrpack для публікації друзям.
use crate::accounts::Account;
use crate::ctx::Ctx;
use crate::error::{err, Context, Error, Result};
use crate::instances::Instance;
use crate::net::{self, Dl};
use crate::{modrinth, util};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest as _;
use std::collections::HashMap;
use std::io::{Read, Write};
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

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LiaTokens {
    pub access_token: String,
    pub client_token: String,
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
        403 if msg.contains("Invalid token") => err("lia_auth", "увійдіть у ЛІЯ ID знову"),
        _ => err("lia", msg),
    })
}

fn account_from(v: &Value) -> Result<Account> {
    let p = &v["selectedProfile"];
    let (Some(id), Some(name)) = (p["id"].as_str(), p["name"].as_str()) else {
        return Err(err("lia", "сервер ЛІЇ повернув неочікувану відповідь"));
    };
    Ok(Account {
        id: format!("lia-{id}"),
        kind: "lia".into(),
        name: name.into(),
        uuid: id.into(),
        added: util::now_ms(),
        ms: None,
        lia: Some(LiaTokens { access_token: v["accessToken"].as_str().unwrap_or("").into(), client_token: v["clientToken"].as_str().unwrap_or("").into() }),
    })
}

fn client_token() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

pub async fn login(http: &reqwest::Client, name: &str, password: &str) -> Result<Account> {
    let v = call(http, reqwest::Method::POST, "/yggdrasil/authserver/authenticate", None, Some(json!({ "username": name.trim(), "password": password, "clientToken": client_token(), "requestUser": false }))).await?;
    account_from(&v)
}

pub async fn register(http: &reqwest::Client, name: &str, password: &str) -> Result<Account> {
    let v = call(http, reqwest::Method::POST, "/api/register", None, Some(json!({ "name": name.trim(), "password": password, "clientToken": client_token() }))).await?;
    account_from(&v)
}

/// Перевіряє токен перед грою; протермінований оновлює. `true` — токен змінився.
pub async fn ensure_fresh(http: &reqwest::Client, acc: &mut Account) -> Result<bool> {
    let Some(t) = acc.lia.clone() else { return Ok(false) };
    let body = json!({ "accessToken": t.access_token, "clientToken": t.client_token });
    match call(http, reqwest::Method::POST, "/yggdrasil/authserver/validate", None, Some(body.clone())).await {
        Ok(_) => Ok(false),
        Err(Error::Coded { code: "lia_auth", .. }) => {
            let v = call(http, reqwest::Method::POST, "/yggdrasil/authserver/refresh", None, Some(body)).await?;
            let fresh = account_from(&v)?;
            acc.name = fresh.name;
            acc.lia = fresh.lia;
            Ok(true)
        }
        Err(e) => Err(e),
    }
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
    task: tokio::task::JoinHandle<()>,
}

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
    if let Some(s) = shares().lock().unwrap().get(instance) {
        return Ok((s.code.clone(), s.address.clone()));
    }
    let s = relay_conn(json!({ "kind": "host", "token": token })).await.ctx(|| "ретранслятор ЛІЇ".into())?;
    let (r, w) = s.into_split();
    let mut lines = BufReader::new(r).lines();
    let hello: Value = match tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line()).await {
        Ok(Ok(Some(l))) => serde_json::from_str(&l)?,
        _ => return Err(err("lia_relay", "ретранслятор не відповів (можливо, вже відкрито 3 світи)")),
    };
    let code = hello["code"].as_str().unwrap_or("").to_string();
    let address = hello["address"].as_str().unwrap_or("").to_string();
    let task = tokio::spawn(async move {
        let _w = w; // тримаємо з'єднання відкритим
        while let Ok(Some(l)) = lines.next_line().await {
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
    shares().lock().unwrap().insert(instance.to_string(), Share { code: code.clone(), address: address.clone(), task });
    Ok((code, address))
}

pub fn unshare(instance: &str) {
    if let Some(s) = shares().lock().unwrap().remove(instance) {
        s.task.abort();
    }
}

// ---------------------------------------------------------------- експорт збірки

const CONTENT_DIRS: &[&str] = &["mods", "resourcepacks", "shaderpacks"];

fn loader_dep(inst: &Instance) -> Result<Option<(&'static str, String)>> {
    let key = match inst.loader.as_str() {
        "vanilla" => return Ok(None),
        "fabric" => "fabric-loader",
        "quilt" => "quilt-loader",
        "forge" => "forge",
        "neoforge" => "neoforge",
        _ => return Ok(None),
    };
    let v = inst.loader_version.clone().ok_or_else(|| err("lia", "спершу запустіть збірку один раз — тоді відома версія модлоадера"))?;
    // У .mrpack Forge вказують без префікса версії гри.
    let v = v.strip_prefix(&format!("{}-", inst.mc_version)).map(str::to_string).unwrap_or(v);
    Ok(Some((key, v)))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Збирає .mrpack: моди з Modrinth — посиланнями, решта (свої моди, config/) — у overrides.
pub async fn export_mrpack(ctx: &Ctx, inst: &Instance) -> Result<Vec<u8>> {
    let game = ctx.paths.game_dir(&inst.id);
    let mut content: Vec<(String, PathBuf)> = vec![];
    for d in CONTENT_DIRS {
        for e in std::fs::read_dir(game.join(d)).into_iter().flatten().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            // Вимкнені (.disabled) і теки ресурспаків не публікуємо — лише робочі файли.
            if p.is_file() && !name.ends_with(".disabled") {
                content.push((format!("{d}/{name}"), p));
            }
        }
    }
    let mut overrides: Vec<(String, PathBuf)> = vec![];
    let mut cfg = vec![];
    walk(&game.join("config"), &mut cfg);
    for p in cfg {
        if let Ok(rel) = p.strip_prefix(&game) {
            overrides.push((rel.to_string_lossy().replace('\\', "/"), p));
        }
    }
    let hashes: Vec<(String, String, String, u64)> = content
        .iter()
        .map(|(rel, p)| {
            let b = std::fs::read(p).ctx(|| p.display().to_string())?;
            Ok((rel.clone(), hex::encode(sha1::Sha1::digest(&b)), hex::encode(sha2::Sha512::digest(&b)), b.len() as u64))
        })
        .collect::<Result<_>>()?;
    let known = modrinth::identify(ctx, &hashes.iter().map(|h| h.1.clone()).collect::<Vec<_>>()).await.unwrap_or_default();
    let mut files = vec![];
    for ((rel, sha1, sha512, size), (_, path)) in hashes.into_iter().zip(content.iter()) {
        let url = known.get(&sha1).and_then(|(ver, _)| ver["files"].as_array()?.iter().find(|f| f["hashes"]["sha1"] == sha1.as_str())?["url"].as_str().map(str::to_string));
        match url {
            Some(url) => files.push(json!({ "path": rel, "hashes": { "sha1": sha1, "sha512": sha512 }, "downloads": [url], "fileSize": size })),
            None => overrides.push((rel, path.clone())),
        }
    }
    let mut deps = json!({ "minecraft": inst.mc_version });
    if let Some((k, v)) = loader_dep(inst)? {
        deps[k] = json!(v);
    }
    let index = json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": util::now_ms().to_string(),
        "name": inst.name,
        "files": files,
        "dependencies": deps,
    });
    tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("modrinth.index.json", opts)?;
        zip.write_all(serde_json::to_string_pretty(&index)?.as_bytes())?;
        for (rel, p) in overrides {
            let mut buf = vec![];
            std::fs::File::open(&p).ctx(|| p.display().to_string())?.read_to_end(&mut buf)?;
            zip.start_file(format!("overrides/{rel}"), opts)?;
            zip.write_all(&buf)?;
        }
        Ok(zip.finish()?.into_inner())
    })
    .await
    .map_err(|e| err("io", e.to_string()))?
}

/// Ідентифікатор збірки ЛІЇ, з якої встановлено цю збірку (поле modpack.projectId = "lia:<id>").
pub fn pack_of(inst: &Instance) -> Option<String> {
    inst.modpack.as_ref()?.project_id.strip_prefix("lia:").map(str::to_string)
}
