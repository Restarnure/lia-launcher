//! Опубліковані збірки. Версія збірки це маніфест, повний список файлів, які отримає гравець.
//! Моди з Modrinth і файли базового модпака йдуть посиланнями, на сервер вантажиться лише своє,
//! по файлу за раз. Так немає межі Cloudflare на запит (100 МБ), а оновлення качає лише змінене.
use crate::api::{auth, event, friend_ids, notify, pack_get_inner, pack_owner, pack_visible, User};
use crate::ygg::hex;
use crate::{bad, forbidden, internal, not_found, App, R, S};
use axum::body::{Body, Bytes};
use axum::extract::Path;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Один файл. Більше однаково не пропустить Cloudflare (100 МБ на запит у безкоштовному плані).
pub const BLOB_MAX: usize = 95 * 1000 * 1000;
/// Маніфест і список хешів: тисячі файлів по сотні байтів.
pub const MANIFEST_MAX: usize = 8 * 1024 * 1024;
const QUOTA: i64 = 1024 * 1024 * 1024;
const FILES_MAX: usize = 50_000;
/// Файл без посилань живе добу: його могли щойно завантажити для версії, яку ще не опубліковано.
const ORPHAN_TTL: i64 = 24 * 3600 * 1000;
/// Звідки гравець може качати файли збірки (як і в лаунчері, за специфікацією .mrpack).
const HOSTS: &[&str] = &["cdn.modrinth.com", "github.com", "raw.githubusercontent.com", "gitlab.com"];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: String,
    pub mc: String,
    pub loader: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
    /// Модпак з Modrinth, з якого зроблено збірку. Файли з позначкою `base` лежать у його архіві.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Base>,
    /// Мова й увімкнені ресурспаки автора.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Options>,
    pub files: Vec<PackFile>,
    #[serde(default)]
    pub version: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Base {
    pub project_id: String,
    pub version_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_name: String,
    pub url: String,
    pub sha1: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(default)]
    pub resource_packs: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackFile {
    pub path: String,
    pub sha1: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub base: bool,
}

impl PackFile {
    /// Файл лежить на сервері ЛІЇ, а не за посиланням чи в базовому модпаку.
    fn stored(&self) -> bool {
        self.url.is_none() && !self.base
    }
}

pub fn manifest_path(app: &App, id: &str) -> PathBuf {
    app.cfg.data.join("packs").join(format!("{id}.json"))
}

pub fn blob_dir(app: &App, owner: i64) -> PathBuf {
    app.cfg.data.join("blobs").join(owner.to_string())
}

fn is_sha1(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Відносний шлях усередині теки гри: без «..», абсолютних шляхів і символів, яких не приймає Windows.
fn safe_path(p: &str) -> bool {
    !p.is_empty() && p.len() <= 512 && p.split('/').all(|c| !c.is_empty() && c != "." && c != ".." && !c.ends_with(['.', ' ']) && !c.chars().any(|ch| ch.is_control() || "\\:*?\"<>|".contains(ch)))
}

fn allowed_url(u: &str, hosts: &[&str]) -> bool {
    let Some(rest) = u.strip_prefix("https://") else { return false };
    u.len() <= 1024 && hosts.contains(&rest.split(['/', '?', '#']).next().unwrap_or(""))
}

fn short(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

/// Перевіряє маніфест від лаунчера. Файли, що мають лежати на сервері, мусять уже бути завантажені власником.
fn validate(app: &App, owner: i64, mut m: Manifest) -> R<Manifest> {
    m.name = short(&m.name, 64);
    m.mc = short(&m.mc, 32);
    if m.name.is_empty() || m.mc.is_empty() || !m.mc.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_ ".contains(&b)) {
        return Err(bad("у збірці не вказано назву чи версію гри"));
    }
    if !["vanilla", "fabric", "quilt", "forge", "neoforge"].contains(&m.loader.as_str()) {
        return Err(bad("невідомий модлоадер"));
    }
    m.loader_version = m.loader_version.map(|v| short(&v, 64)).filter(|v| !v.is_empty());
    if let Some(b) = &mut m.base {
        let id_ok = |s: &str| !s.is_empty() && s.len() <= 16 && s.bytes().all(|c| c.is_ascii_alphanumeric());
        if !id_ok(&b.project_id) || !id_ok(&b.version_id) || !allowed_url(&b.url, &["cdn.modrinth.com"]) || !is_sha1(&b.sha1) {
            return Err(bad("базовий модпак вказано неправильно"));
        }
        b.name = short(&b.name, 128);
        b.version_name = short(&b.version_name, 64);
        b.icon_url = b.icon_url.take().filter(|u| allowed_url(u, &["cdn.modrinth.com"]));
    }
    if let Some(o) = &mut m.options {
        o.lang = o.lang.take().filter(|l| l.len() <= 16 && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'));
        if o.resource_packs.len() > 256 || o.resource_packs.iter().any(|p| p.len() > 256 || p.chars().any(char::is_control)) {
            return Err(bad("забагато ресурспаків"));
        }
    }
    if m.files.len() > FILES_MAX {
        return Err(bad("у збірці забагато файлів"));
    }
    // Windows не розрізняє регістр: два шляхи, що відрізняються лише ним, ведуть до того самого файлу.
    let mut seen = HashSet::new();
    let db = app.db.lock().unwrap();
    let mut q = db.prepare("SELECT size FROM blobs WHERE owner = ?1 AND sha1 = ?2").map_err(internal)?;
    for f in &mut m.files {
        if !safe_path(&f.path) || !seen.insert(f.path.to_lowercase()) {
            return Err(bad(format!("неприпустимий шлях у збірці: {}", short(&f.path, 100))));
        }
        if !is_sha1(&f.sha1) {
            return Err(bad(format!("неправильний хеш файлу {}", f.path)));
        }
        if let Some(u) = &f.url {
            if !allowed_url(u, HOSTS) {
                return Err(bad(format!("файл {} з недозволеного джерела", f.path)));
            }
            f.base = false;
        } else if f.base {
            if m.base.is_none() {
                return Err(bad("у збірці немає базового модпака"));
            }
        } else {
            // Розмір беремо свій: його порахував сервер, коли приймав файл.
            f.size = q.query_row(params![owner, f.sha1], |r| r.get::<_, i64>(0)).optional().map_err(internal)?.ok_or_else(|| bad(format!("файл {} не завантажено", f.path)))? as u64;
        }
    }
    Ok(m)
}

fn write_manifest(app: &App, id: &str, m: &Manifest) -> R<()> {
    let path = manifest_path(app, id);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(m).map_err(internal)?).map_err(internal)?;
    std::fs::rename(&tmp, &path).map_err(internal)
}

fn set_files(db: &rusqlite::Connection, id: &str, m: &Manifest) -> R<()> {
    db.execute("DELETE FROM pack_files WHERE pack = ?1", [id]).map_err(internal)?;
    let mut q = db.prepare("INSERT OR IGNORE INTO pack_files (pack, sha1) VALUES (?1, ?2)").map_err(internal)?;
    for f in m.files.iter().filter(|f| f.stored()) {
        q.execute(params![id, f.sha1]).map_err(internal)?;
    }
    Ok(())
}

/// Скільки займе збірка в гравця.
fn total(m: &Manifest) -> i64 {
    m.files.iter().map(|f| f.size as i64).sum()
}

/// Прибирає файли власника, на які вже не посилається жодна його збірка.
fn gc(app: &App, owner: i64) {
    let dead: Vec<String> = {
        let db = app.db.lock().unwrap();
        let Ok(mut q) = db.prepare(
            "SELECT b.sha1 FROM blobs b WHERE b.owner = ?1 AND b.created < ?2
             AND NOT EXISTS (SELECT 1 FROM pack_files f JOIN packs p ON p.id = f.pack WHERE p.owner = ?1 AND f.sha1 = b.sha1)",
        ) else {
            return;
        };
        let v = q.query_map(params![owner, crate::now() - ORPHAN_TTL], |r| r.get(0)).map(|r| r.flatten().collect()).unwrap_or_default();
        v
    };
    for sha in dead {
        let _ = std::fs::remove_file(blob_dir(app, owner).join(&sha));
        let _ = app.db.lock().unwrap().execute("DELETE FROM blobs WHERE owner = ?1 AND sha1 = ?2", params![owner, sha]);
    }
}

fn notify_friends(app: &App, me: &User, msg: &Value) {
    for f in friend_ids(app, me.id) {
        notify(app, f, msg);
    }
}

// ------------------------------------------------------------ файли автора

#[derive(Deserialize)]
pub struct Hashes {
    hashes: Vec<String>,
}

/// Яких файлів ще немає на сервері. Наявні «освіжаємо», щоб прибирання не забрало їх до публікації.
pub async fn blobs_missing(st: S, h: HeaderMap, Json(b): Json<Hashes>) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    if b.hashes.len() > FILES_MAX {
        return Err(bad("забагато файлів"));
    }
    let db = st.db.lock().unwrap();
    let mut touch = db.prepare("UPDATE blobs SET created = ?3 WHERE owner = ?1 AND sha1 = ?2").map_err(internal)?;
    let now = crate::now();
    let mut missing = vec![];
    for s in b.hashes.iter().filter(|s| is_sha1(s)) {
        if touch.execute(params![me.id, s, now]).map_err(internal)? == 0 {
            missing.push(s);
        }
    }
    Ok(Json(json!({ "missing": missing })))
}

/// Тимчасовий файл прибирається сам: і при помилці, і коли запит обірвали посеред завантаження.
struct TmpFile(PathBuf);

impl Drop for TmpFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Приймає один файл у тимчасовий файл (не в пам'ять), звіряє SHA-1 і ставить на місце.
pub async fn blob_put(st: S, h: HeaderMap, Path(sha): Path<String>, body: Body) -> R<StatusCode> {
    let me = auth(&st, &h)?;
    if !is_sha1(&sha) {
        return Err(bad("неправильний хеш файлу"));
    }
    st.limit(format!("blob:{}", me.id), 20_000, Duration::from_secs(3600))?;
    let dir = blob_dir(&st, me.id);
    tokio::fs::create_dir_all(&dir).await.map_err(internal)?;
    let tmp = TmpFile(dir.join(format!("{sha}.{}.tmp", hex(&rand::random::<[u8; 6]>()))));
    let mut f = tokio::fs::File::create(&tmp.0).await.map_err(internal)?;
    let mut hasher = Sha1::new();
    let mut size = 0;
    let mut s = body.into_data_stream();
    while let Some(chunk) = s.next().await {
        let chunk = chunk.map_err(|_| bad("файл не вдалося отримати"))?;
        size += chunk.len();
        if size > BLOB_MAX {
            return Err(bad("файл завеликий (до 95 МБ)"));
        }
        hasher.update(&chunk);
        f.write_all(&chunk).await.map_err(internal)?;
    }
    f.flush().await.map_err(internal)?;
    drop(f);
    if hex(&hasher.finalize()) != sha {
        return Err(bad("файл пошкодився дорогою, спробуйте ще раз"));
    }
    let db = st.db.lock().unwrap();
    let used: i64 = db.query_row("SELECT coalesce(sum(size), 0) FROM blobs WHERE owner = ?1 AND sha1 != ?2", params![me.id, sha], |r| r.get(0)).map_err(internal)?;
    if used + size as i64 > QUOTA {
        return Err(bad("перевищено місце для збірок (1 ГБ)"));
    }
    std::fs::rename(&tmp.0, dir.join(&sha)).map_err(internal)?;
    db.execute("INSERT OR REPLACE INTO blobs (owner, sha1, size, created) VALUES (?1, ?2, ?3, ?4)", params![me.id, sha, size as i64, crate::now()]).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------ збірки

#[derive(Deserialize)]
pub struct Publish {
    manifest: Manifest,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

fn visibility(v: Option<&str>) -> &'static str {
    if v == Some("public") {
        "public"
    } else {
        "friends"
    }
}

pub async fn pack_publish(st: S, h: HeaderMap, Json(p): Json<Publish>) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    st.limit(format!("pack:{}", me.id), 30, Duration::from_secs(3600))?;
    let mut m = validate(&st, me.id, p.manifest)?;
    m.version = 1;
    let id = hex(&rand::random::<[u8; 6]>());
    write_manifest(&st, &id, &m)?;
    let desc = p.description.map(|d| short(&d, 500)).unwrap_or_default();
    let now = crate::now();
    {
        let db = st.db.lock().unwrap();
        db.execute(
            "INSERT INTO packs (id, owner, name, description, mc_version, loader, visibility, version, size, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?9)",
            params![id, me.id, m.name, desc, m.mc, m.loader, visibility(p.visibility.as_deref()), total(&m), now],
        )
        .map_err(internal)?;
        set_files(&db, &id, &m)?;
    }
    event(&st, me.id, "pack_published", Some(&id), json!({ "name": m.name, "version": 1 }));
    notify_friends(&st, &me, &json!({ "type": "feed" }));
    gc(&st, me.id);
    pack_get_inner(&st, &me, &id)
}

pub async fn pack_update(st: S, h: HeaderMap, Path(id): Path<String>, Json(p): Json<Publish>) -> R<Json<Value>> {
    let me = auth(&st, &h)?;
    let (owner, _) = pack_owner(&st, &id)?;
    if owner != me.id {
        return Err(forbidden("це не ваша збірка"));
    }
    st.limit(format!("pack:{}", me.id), 30, Duration::from_secs(3600))?;
    let mut m = validate(&st, me.id, p.manifest)?;
    {
        let db = st.db.lock().unwrap();
        m.version = db.query_row("SELECT version FROM packs WHERE id = ?1", [&id], |r| r.get::<_, i64>(0)).map_err(internal)? + 1;
        write_manifest(&st, &id, &m)?;
        db.execute(
            "UPDATE packs SET name = ?1, mc_version = ?2, loader = ?3, size = ?4, version = ?5, updated = ?6,
                              visibility = coalesce(?7, visibility), description = coalesce(?8, description) WHERE id = ?9",
            params![m.name, m.mc, m.loader, total(&m), m.version, crate::now(), p.visibility.as_deref().map(|v| visibility(Some(v))), p.description.map(|d| short(&d, 500)), id],
        )
        .map_err(internal)?;
        set_files(&db, &id, &m)?;
    }
    event(&st, me.id, "pack_updated", Some(&id), json!({ "name": m.name, "version": m.version }));
    let subs: Vec<i64> = {
        let db = st.db.lock().unwrap();
        let mut q = db.prepare("SELECT user FROM subs WHERE pack = ?1").map_err(internal)?;
        let v = q.query_map([&id], |r| r.get(0)).map_err(internal)?.flatten().collect();
        v
    };
    let msg = json!({ "type": "pack", "id": id, "version": m.version });
    for s in subs {
        notify(&st, s, &msg);
    }
    notify_friends(&st, &me, &json!({ "type": "feed" }));
    gc(&st, me.id);
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
    let _ = std::fs::remove_file(manifest_path(&st, &id));
    gc(&st, me.id);
    Ok(StatusCode::NO_CONTENT)
}

/// Видима збірка та її власник.
fn visible_pack(app: &App, me: &User, id: &str) -> R<i64> {
    let (owner, vis) = pack_owner(app, id)?;
    if !pack_visible(app, me, owner, &vis) {
        return Err(not_found());
    }
    Ok(owner)
}

pub async fn pack_manifest(st: S, h: HeaderMap, Path(id): Path<String>) -> R<Response> {
    let me = auth(&st, &h)?;
    visible_pack(&st, &me, &id)?;
    let bytes = tokio::fs::read(manifest_path(&st, &id)).await.map_err(|_| not_found())?;
    Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response())
}

/// Файл збірки. Віддаємо лише ті, що є в поточній версії: чужі файли автора за хешем не дістати.
pub async fn pack_blob(st: S, h: HeaderMap, Path((id, sha)): Path<(String, String)>) -> R<Response> {
    let me = auth(&st, &h)?;
    let owner = visible_pack(&st, &me, &id)?;
    if !is_sha1(&sha) || !st.db.lock().unwrap().prepare("SELECT 1 FROM pack_files WHERE pack = ?1 AND sha1 = ?2").map_err(internal)?.exists(params![id, sha]).map_err(internal)? {
        return Err(not_found());
    }
    let f = tokio::fs::File::open(blob_dir(&st, owner).join(&sha)).await.map_err(|_| not_found())?;
    let len = f.metadata().await.map_err(internal)?.len();
    // Потоком, а не цілим файлом у пам'яті: кілька одночасних завантажень вичерпали б пам'ять сервера.
    let stream = futures::stream::try_unfold(f, |mut f| async move {
        let mut buf = vec![0u8; 64 * 1024];
        let n = f.read(&mut buf).await?;
        buf.truncate(n);
        Ok::<_, std::io::Error>((n > 0).then(|| (Bytes::from(buf), f)))
    });
    Ok(([(header::CONTENT_TYPE, "application/octet-stream".to_string()), (header::CONTENT_LENGTH, len.to_string())], Body::from_stream(stream)).into_response())
}

// ------------------------------------------------------------ перенесення збірок до 1.1.5

/// До 1.1.5 збірка лежала одним .mrpack. Розкладаємо такі на маніфест і файли автора.
pub fn migrate(app: &App) {
    let rows: Vec<(String, i64, i64)> = {
        let db = app.db.lock().unwrap();
        let mut q = db.prepare("SELECT id, owner, version FROM packs").expect("збірки");
        let v = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).expect("збірки").flatten().collect();
        v
    };
    for (id, owner, version) in rows {
        let old = app.cfg.data.join("packs").join(format!("{id}.mrpack"));
        if !old.exists() {
            continue;
        }
        let r = from_mrpack(app, owner, &old).and_then(|mut m| {
            m.version = version;
            write_manifest(app, &id, &m).map_err(|e| e.2)?;
            let db = app.db.lock().unwrap();
            set_files(&db, &id, &m).map_err(|e| e.2)?;
            db.execute("UPDATE packs SET size = ?1 WHERE id = ?2", params![total(&m), id]).map_err(|e| e.to_string())?;
            Ok(())
        });
        match r {
            Ok(()) => {
                let _ = std::fs::remove_file(&old);
                println!("збірку {id} перенесено в новий формат");
            }
            Err(e) => eprintln!("збірку {id} не вдалося перенести: {e}"),
        }
    }
}

fn from_mrpack(app: &App, owner: i64, path: &std::path::Path) -> Result<Manifest, String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut s = String::new();
    zip.by_name("modrinth.index.json").map_err(|e| e.to_string())?.take(4 * 1024 * 1024).read_to_string(&mut s).map_err(|e| e.to_string())?;
    let index: Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    let deps = &index["dependencies"];
    let mc = deps["minecraft"].as_str().ok_or("не вказано версію гри")?.to_string();
    let (loader, loader_version) = [("fabric-loader", "fabric"), ("quilt-loader", "quilt"), ("neoforge", "neoforge"), ("forge", "forge")]
        .iter()
        .find_map(|(k, l)| deps[*k].as_str().map(|v| (l.to_string(), Some(v.to_string()))))
        .unwrap_or(("vanilla".into(), None));
    // Лаунчер зберігає версію Forge з префіксом версії гри, а .mrpack без нього.
    let loader_version = match (loader.as_str(), loader_version) {
        ("forge", Some(v)) if !v.starts_with(&format!("{mc}-")) => Some(format!("{mc}-{v}")),
        (_, v) => v,
    };
    let mut files: Vec<PackFile> = vec![];
    for f in index["files"].as_array().into_iter().flatten() {
        if f["env"]["client"] == "unsupported" {
            continue;
        }
        let url = f["downloads"].as_array().into_iter().flatten().filter_map(Value::as_str).find(|u| allowed_url(u, HOSTS));
        let (Some(p), Some(sha), Some(url)) = (f["path"].as_str(), f["hashes"]["sha1"].as_str(), url) else { continue };
        files.push(PackFile { path: p.to_string(), sha1: sha.to_lowercase(), size: f["fileSize"].as_u64().unwrap_or(0), url: Some(url.to_string()), base: false });
    }
    let dir = blob_dir(app, owner);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // client-overrides перекривають overrides (специфікація .mrpack).
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = e.name().map_err(|e| e.to_string())?.to_string();
            let Some(rel) = name.strip_prefix(prefix).filter(|r| !r.is_empty() && safe_path(r)).map(str::to_string) else { continue };
            if e.is_dir() {
                continue;
            }
            let mut buf = vec![];
            (&mut e).take(BLOB_MAX as u64 + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            if buf.len() > BLOB_MAX {
                return Err(format!("файл {rel} завеликий"));
            }
            let sha = hex(&Sha1::digest(&buf));
            std::fs::write(dir.join(&sha), &buf).map_err(|e| e.to_string())?;
            app.db
                .lock()
                .unwrap()
                .execute("INSERT OR IGNORE INTO blobs (owner, sha1, size, created) VALUES (?1, ?2, ?3, ?4)", params![owner, sha, buf.len() as i64, crate::now()])
                .map_err(|e| e.to_string())?;
            files.retain(|f| f.path != rel);
            files.push(PackFile { path: rel, sha1: sha, size: buf.len() as u64, url: None, base: false });
        }
    }
    let name = short(index["name"].as_str().unwrap_or("Збірка"), 64);
    Ok(Manifest { name, mc, loader, loader_version, base: None, options: None, files, version: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        for ok in ["mods/a.jar", "config/sub/x.toml", "resourcepacks/Русифікатор.zip", "options.txt"] {
            assert!(safe_path(ok), "{ok}");
        }
        for bad in ["", "/etc/passwd", "../x", "mods/../../x", "mods//a", "C:/x", "mods\\a.jar", "a/.", "mods/a.", "mods/a ", "x\0y"] {
            assert!(!safe_path(bad), "{bad}");
        }
    }

    #[test]
    fn urls_and_hashes() {
        assert!(allowed_url("https://cdn.modrinth.com/data/x/y.jar", HOSTS));
        for bad in ["http://cdn.modrinth.com/x", "https://cdn.modrinth.com.evil.io/x", "https://user@cdn.modrinth.com/x", "https://cdn.modrinth.com:8443/x", "https://example.com/x"] {
            assert!(!allowed_url(bad, HOSTS), "{bad}");
        }
        assert!(is_sha1("da39a3ee5e6b4b0d3255bfef95601890afd80709"));
        assert!(!is_sha1("DA39A3EE5E6B4B0D3255BFEF95601890AFD80709"));
        assert!(!is_sha1("da39a3ee"));
    }

    #[test]
    fn manifest_shape() {
        let m: Manifest = serde_json::from_value(json!({
            "name": "Тест", "mc": "1.20.1", "loader": "fabric", "loaderVersion": "0.16.10",
            "options": { "lang": "uk_ua", "resourcePacks": ["vanilla", "file/ru.zip"] },
            "files": [{ "path": "mods/a.jar", "sha1": "da39a3ee5e6b4b0d3255bfef95601890afd80709", "size": 0, "url": "https://cdn.modrinth.com/a.jar" },
                      { "path": "config/a.toml", "sha1": "da39a3ee5e6b4b0d3255bfef95601890afd80709", "size": 0 }]
        }))
        .unwrap();
        assert!(!m.files[0].stored() && m.files[1].stored());
        let back = serde_json::to_value(&m).unwrap();
        assert!(back["files"][1].get("base").is_none() && back["files"][1].get("url").is_none());
        assert_eq!(back["options"]["resourcePacks"][1], "file/ru.zip");
    }
}
