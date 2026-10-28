//! Збірки ЛІЇ. Версія збірки це маніфест, повний список файлів, які отримає гравець (server/src/packs.rs).
//! Моди з Modrinth і незмінені файли базового модпака йдуть посиланнями, на сервер ЛІЇ вантажиться
//! лише своє. Гравець при оновленні качає тільки ті файли, що в нього відрізняються.
use crate::ctx::Ctx;
use crate::error::{err, Context, Result};
use crate::instances::{self, Instance, ModpackRef, NewInstance};
use crate::net::{self, Dl};
use crate::tasks::Task;
use crate::{lia, loaders, modrinth, util};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Digest as _;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// Один файл на сервері ЛІЇ (більше не пропустить Cloudflare).
const BLOB_MAX: u64 = 95 * 1000 * 1000;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: String,
    pub mc: String,
    pub loader: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Base>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Options>,
    pub files: Vec<PackFile>,
    #[serde(default)]
    pub version: i64,
}

/// Модпак з Modrinth, з якого зроблено збірку.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
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

/// Мова й увімкнені ресурспаки автора (з його options.txt).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(default)]
    pub resource_packs: Vec<String>,
}

/// Файл збірки: за посиланням (`url`), з архіву базового модпака (`base`) або з сервера ЛІЇ.
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

// ---------------------------------------------------------------- що публікується

/// Теки й файли, які не публікуються ніколи: світи, журнали, кеші, особисте.
const NEVER: &[&str] = &[
    "saves", "logs", "crash-reports", "screenshots", "backups", ".cache", ".fabric", ".mixin.out", "modernfix", "debug", "natives", "options.txt", "usercache.json",
    "usernamecache.json", "realms_persistence.json", "command_history.txt", "hotbar.nbt", "servers.dat_old",
];
/// Теки, увімкнені за замовчуванням.
const DEFAULT_ON: &[&str] = &["mods", "resourcepacks", "shaderpacks", "config", "defaultconfigs", "datapacks", "kubejs", "scripts", "global_packs", "openloader"];
/// Теки вмісту: тут файли можуть знайтися на Modrinth, а назви на кшталт «token» нічого не означають.
const CONTENT: &[&str] = &["mods", "resourcepacks", "shaderpacks", "datapacks"];
/// Налаштування гравця: пишуться, лише якщо їх ще немає, і ніколи не видаляються.
const KEEP: &[&str] = &["options.txt", "servers.dat", "optionsof.txt", "optionsshaders.txt"];

fn top(rel: &str) -> &str {
    rel.split('/').next().unwrap_or(rel)
}

/// Файли конфігів, які не можна публікувати: деякі моди тримають там токени й облікові дані.
fn private_config(rel: &str) -> bool {
    let n = rel.to_lowercase();
    ["token", "secret", "password", "passwd", "credential", "session", "account", "auth", ".key", "apikey", "api_key", "webhook"].iter().any(|w| n.contains(w))
}

fn never(rel: &str) -> bool {
    let t = top(rel);
    NEVER.contains(&t) || t.starts_with("mods.lia-backup") || rel.ends_with(".part") || (!CONTENT.contains(&t) && private_config(rel))
}

/// Вибір автора: шлях публікується, якщо найглибший запис, що його покриває, є в `include`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

fn under(rel: &str, dir: &str) -> bool {
    rel == dir || rel.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

impl Selection {
    pub fn picks(&self, rel: &str) -> bool {
        let deepest = |l: &[String]| l.iter().filter(|p| under(rel, p)).map(|p| p.len()).max();
        match (deepest(&self.include), deepest(&self.exclude)) {
            (Some(i), Some(e)) => i > e,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

/// Налаштування публікації збірки, у теці збірки.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Publishing {
    #[serde(default)]
    pack_id: Option<String>,
    #[serde(default)]
    selection: Option<Selection>,
    #[serde(default = "yes")]
    share_options: bool,
}

fn yes() -> bool {
    true
}

fn publishing_path(ctx: &Ctx, inst: &Instance) -> PathBuf {
    ctx.paths.instance_dir(&inst.id).join("lia-publish.json")
}

// ---------------------------------------------------------------- хеші

/// Шлях → (розмір, час зміни, SHA-1).
type Hashes = Mutex<HashMap<PathBuf, (u64, SystemTime, String)>>;

fn hash_cache() -> &'static Hashes {
    static C: OnceLock<Hashes> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// SHA-1 файлу. Незмінений (розмір і час зміни ті самі) файл удруге не читається.
fn sha1_cached(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    let key = (m.len(), m.modified().ok()?);
    if let Some(c) = hash_cache().lock().unwrap().get(p).filter(|c| (c.0, c.1) == key) {
        return Some(c.2.clone());
    }
    let h = util::sha1_file(p).ok()?;
    hash_cache().lock().unwrap().insert(p.to_path_buf(), (key.0, key.1, h.clone()));
    Some(h)
}

/// Хеші багатьох файлів паралельно на всіх ядрах.
async fn hash_all(paths: Vec<PathBuf>) -> Vec<Option<String>> {
    tokio::task::spawn_blocking(move || {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
        let chunk = paths.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            let hs: Vec<_> = paths.chunks(chunk).map(|part| (part.len(), s.spawn(move || part.iter().map(|p| sha1_cached(p)).collect::<Vec<_>>()))).collect();
            hs.into_iter().flat_map(|(n, h)| h.join().unwrap_or_else(|_| vec![None; n])).collect()
        })
    })
    .await
    .unwrap_or_default()
}

// ---------------------------------------------------------------- базовий модпак

/// Файл базового модпака: `url` для файлів з індексу, а без нього вміст лежить в архіві.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BaseFile {
    pub path: String,
    pub sha1: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

fn base_cache(ctx: &Ctx, version_id: &str) -> PathBuf {
    ctx.paths.cache().join("modpacks").join(format!("{version_id}.json"))
}

/// Файли модпака з архіву .mrpack: з індексу (посилання) і з overrides (вміст в архіві).
fn read_base(mrpack: &Path) -> Result<Vec<BaseFile>> {
    let index = modrinth::read_index(mrpack)?;
    let mut out: BTreeMap<String, BaseFile> = BTreeMap::new();
    for f in index.files {
        if f.env.as_ref().and_then(|e| e.get("client")).is_some_and(|c| c == "unsupported") {
            continue;
        }
        let (Some(sha1), Some(url)) = (f.hashes.get("sha1"), f.downloads.iter().find(|u| modrinth::allowed_host(u))) else { continue };
        out.insert(f.path.clone(), BaseFile { path: f.path, sha1: sha1.to_lowercase(), size: f.file_size.unwrap_or(0), url: Some(url.clone()) });
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(mrpack).ctx(|| mrpack.display().to_string())?)?;
    // client-overrides перекривають overrides (специфікація .mrpack).
    for prefix in ["overrides/", "client-overrides/"] {
        for i in 0..zip.len() {
            let mut e = zip.by_index(i)?;
            let name = e.name()?.to_string();
            let Some(rel) = name.strip_prefix(prefix).filter(|r| !r.is_empty() && !e.is_dir()) else { continue };
            let mut h = sha1::Sha1::new();
            let size = std::io::copy(&mut e, &mut HashWriter(&mut h))?;
            out.insert(rel.to_string(), BaseFile { path: rel.to_string(), sha1: hex::encode(h.finalize()), size, url: None });
        }
    }
    Ok(out.into_values().collect())
}

struct HashWriter<'a>(&'a mut sha1::Sha1);

impl Write for HashWriter<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.update(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Запам'ятовує список файлів модпака `version_id` з уже завантаженого архіву.
pub async fn cache_base(ctx: &Ctx, version_id: &str, mrpack: &Path) -> Result<Vec<BaseFile>> {
    let p = mrpack.to_path_buf();
    let files = tokio::task::spawn_blocking(move || read_base(&p)).await.map_err(|e| err("io", e.to_string()))??;
    util::write_json(&base_cache(ctx, version_id), &files)?;
    Ok(files)
}

/// Завантажує архів базового модпака в тимчасовий файл.
async fn download_base(ctx: &Ctx, base: &Base, task: &Arc<Task>) -> Result<PathBuf> {
    let tmp = ctx.paths.temp().join(format!("base-{}-{}.mrpack", base.version_id, util::now_ms()));
    task.stage("modpack", 0, "");
    net::download(&ctx.http, &Dl::new(&base.url, tmp.clone(), Some(base.sha1.clone()), None), Some(task)).await?;
    Ok(tmp)
}

async fn base_files(ctx: &Ctx, base: &Base, task: &Arc<Task>) -> Result<Vec<BaseFile>> {
    if let Ok(v) = util::read_json(&base_cache(ctx, &base.version_id)) {
        return Ok(v);
    }
    let tmp = download_base(ctx, base, task).await?;
    let r = cache_base(ctx, &base.version_id, &tmp).await;
    let _ = std::fs::remove_file(&tmp);
    r
}

/// Модпак з Modrinth, з якого встановлено збірку. `None`, якщо його немає (збірка з нуля чи з файлу) або Modrinth недоступний.
async fn base_of(ctx: &Ctx, inst: &Instance) -> Option<Base> {
    let m = inst.modpack.as_ref().filter(|m| !m.project_id.is_empty() && !m.project_id.starts_with("lia:"))?;
    let ver = modrinth::version(ctx, &m.version_id).await.ok()?;
    let f = modrinth::primary_file(&ver)?;
    Some(Base {
        project_id: m.project_id.clone(),
        version_id: m.version_id.clone(),
        name: m.name.clone(),
        version_name: m.version_name.clone(),
        url: f["url"].as_str()?.to_string(),
        sha1: f["hashes"]["sha1"].as_str()?.to_string(),
        icon_url: m.icon_url.clone(),
    })
}

// ---------------------------------------------------------------- публікація

/// Звідки гравець візьме файл.
#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Такий самий, як у базовому модпаку.
    Base,
    /// Знайдено на Modrinth.
    Link,
    /// Треба завантажити на сервер ЛІЇ.
    Upload,
    /// Уже є на сервері ЛІЇ.
    Stored,
    /// Завеликий для сервера ЛІЇ.
    Big,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlanFile {
    pub path: String,
    pub size: u64,
    pub kind: Kind,
    #[serde(skip)]
    sha1: String,
    #[serde(skip)]
    url: Option<String>,
}

/// Що є в теці гри і звідки кожен файл отримає гравець. Інтерфейс показує це деревом з галочками.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub base: Option<Base>,
    pub files: Vec<PlanFile>,
    pub selection: Selection,
    pub pack_id: Option<String>,
    pub share_options: bool,
    pub options: Option<Options>,
    #[serde(skip)]
    base_files: Vec<BaseFile>,
}

/// Усі файли теки гри, крім тих, що не публікуються ніколи. Ярлики пропускаємо: вони могли б вивести за межі теки.
fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf, u64)>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(t) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        if t.is_symlink() || never(&r) {
            continue;
        }
        if t.is_dir() {
            walk(&e.path(), &r, out);
        } else if let Ok(m) = e.metadata() {
            out.push((r, e.path(), m.len()));
        }
    }
}

fn identify_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Посилання Modrinth для файлів за SHA-1. Відповіді запам'ятовуються до перезапуску лаунчера.
async fn links(ctx: &Ctx, hashes: Vec<String>) -> HashMap<String, String> {
    let unknown: Vec<String> = {
        let c = identify_cache().lock().unwrap();
        hashes.iter().filter(|h| !c.contains_key(*h)).cloned().collect()
    };
    for part in unknown.chunks(500) {
        let Ok(found) = modrinth::identify(ctx, part).await else { continue };
        let mut c = identify_cache().lock().unwrap();
        for h in part {
            let url = found.get(h).and_then(|(ver, _)| ver["files"].as_array()?.iter().find(|f| f["hashes"]["sha1"] == h.as_str())?["url"].as_str().map(str::to_string));
            c.insert(h.clone(), url.filter(|u| modrinth::allowed_host(u)));
        }
    }
    let c = identify_cache().lock().unwrap();
    hashes.into_iter().filter_map(|h| Some((h.clone(), c.get(&h)?.clone()?))).collect()
}

/// Яких файлів ще немає на сервері ЛІЇ.
async fn missing_blobs(ctx: &Ctx, token: &str, hashes: Vec<String>) -> Result<HashSet<String>> {
    if hashes.is_empty() {
        return Ok(HashSet::new());
    }
    let v = lia::call(&ctx.http, reqwest::Method::POST, "/api/blobs/missing", Some(token), Some(json!({ "hashes": hashes }))).await?;
    Ok(v["missing"].as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)).collect())
}

/// options.txt: `ключ:значення` по рядку.
fn option<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
}

fn read_options(game: &Path) -> Option<Options> {
    let text = std::fs::read_to_string(game.join("options.txt")).ok()?;
    let lang = option(&text, "lang").map(str::to_string).filter(|l| !l.is_empty());
    let resource_packs = option(&text, "resourcePacks").and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default();
    Some(Options { lang, resource_packs })
}

pub async fn plan(ctx: &Ctx, inst: &Instance, token: &str, task: &Arc<Task>) -> Result<Plan> {
    let game = ctx.paths.game_dir(&inst.id);
    let mut local = vec![];
    walk(&game, "", &mut local);
    local.sort_by(|a, b| a.0.cmp(&b.0));
    let base = base_of(ctx, inst).await;
    let base_files = match &base {
        Some(b) => base_files(ctx, b, task).await?,
        None => vec![],
    };
    task.stage("pack_scan", 0, "");
    let hashes = hash_all(local.iter().map(|l| l.1.clone()).collect()).await;
    let by_path: HashMap<&str, &BaseFile> = base_files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut files = vec![];
    for ((rel, _, size), sha1) in local.iter().zip(hashes) {
        let Some(sha1) = sha1 else { continue };
        let same = by_path.get(rel.as_str()).filter(|b| b.sha1 == sha1);
        let kind = if same.is_some() { Kind::Base } else { Kind::Upload };
        files.push(PlanFile { path: rel.clone(), size: *size, kind, url: same.and_then(|b| b.url.clone()), sha1 });
    }
    // Свої .jar і .zip шукаємо на Modrinth, щоб гравець качав їх звідти.
    let ask: Vec<String> = files.iter().filter(|f| f.kind == Kind::Upload && CONTENT.contains(&top(&f.path)) && (f.path.ends_with(".jar") || f.path.ends_with(".zip"))).map(|f| f.sha1.clone()).collect();
    let found = links(ctx, ask).await;
    for f in files.iter_mut().filter(|f| f.kind == Kind::Upload) {
        if let Some(u) = found.get(&f.sha1) {
            f.kind = Kind::Link;
            f.url = Some(u.clone());
        } else if f.size > BLOB_MAX {
            f.kind = Kind::Big;
        }
    }
    let missing = missing_blobs(ctx, token, files.iter().filter(|f| f.kind == Kind::Upload).map(|f| f.sha1.clone()).collect()).await?;
    for f in files.iter_mut().filter(|f| f.kind == Kind::Upload && !missing.contains(&f.sha1)) {
        f.kind = Kind::Stored;
    }
    let saved: Option<Publishing> = util::read_json(&publishing_path(ctx, inst)).ok();
    let selection = saved.as_ref().and_then(|s| s.selection.clone()).unwrap_or_else(|| {
        let mut include: Vec<String> = DEFAULT_ON.iter().map(|s| s.to_string()).collect();
        include.extend(base_files.iter().map(|f| top(&f.path).to_string()).filter(|t| !never(t)));
        include.sort();
        include.dedup();
        Selection { include, exclude: vec![] }
    });
    Ok(Plan {
        base,
        files,
        selection,
        pack_id: saved.as_ref().and_then(|s| s.pack_id.clone()),
        share_options: saved.is_none_or(|s| s.share_options),
        options: read_options(&game),
        base_files,
    })
}

/// Версія модлоадера у форматі лаунчера. «Остання» не записується в збірку, тож беремо її з профілю запуску.
fn loader_version(inst: &Instance) -> Result<Option<String>> {
    if inst.loader == "vanilla" {
        return Ok(None);
    }
    let v = inst
        .loader_version
        .clone()
        .or_else(|| inst.version_id.as_deref().and_then(|id| loader_version_of(&inst.loader, &inst.mc_version, id)))
        .ok_or_else(|| err("lia", "спершу запустіть збірку один раз, тоді стане відома версія модлоадера"))?;
    // Forge лаунчер зберігає з префіксом версії гри.
    Ok(Some(if inst.loader == "forge" && !v.starts_with(&format!("{}-", inst.mc_version)) { format!("{}-{v}", inst.mc_version) } else { v }))
}

/// Версія модлоадера з id профілю запуску: fabric-loader-0.16.10-1.21.4, quilt-loader-…,
/// 1.20.1-forge-47.2.0, neoforge-21.1.77.
fn loader_version_of(loader: &str, mc: &str, id: &str) -> Option<String> {
    let v = match loader {
        "fabric" => id.strip_prefix("fabric-loader-")?.strip_suffix(&format!("-{mc}"))?,
        "quilt" => id.strip_prefix("quilt-loader-")?.strip_suffix(&format!("-{mc}"))?,
        "forge" => id.strip_prefix(&format!("{mc}-forge-"))?,
        "neoforge" => id.strip_prefix("neoforge-")?,
        _ => return None,
    };
    (!v.is_empty()).then(|| v.to_string())
}

/// Маніфест з плану й вибору автора. Файли бази в невибраних теках лишаються такими, як у модпаку:
/// невибрана тека означає «мої зміни тут не діляться», а не «видалити».
pub fn manifest(plan: &Plan, inst: &Instance, sel: &Selection, share_options: bool) -> Result<Manifest> {
    let mut files = vec![];
    let mut taken = HashSet::new();
    for f in plan.files.iter().filter(|f| sel.picks(&f.path)) {
        if f.kind == Kind::Big {
            return Err(err("lia", format!("файл {} більший за 95 МБ, зніміть з нього галочку", f.path)));
        }
        taken.insert(f.path.clone());
        let base = f.kind == Kind::Base && f.url.is_none();
        files.push(PackFile { path: f.path.clone(), sha1: f.sha1.clone(), size: f.size, url: f.url.clone(), base });
    }
    for b in &plan.base_files {
        // Файл бази у вибраній теці, якого в автора немає, автор видалив.
        if taken.contains(&b.path) || (sel.picks(&b.path) && !never(&b.path)) {
            continue;
        }
        files.push(PackFile { path: b.path.clone(), sha1: b.sha1.clone(), size: b.size, url: b.url.clone(), base: b.url.is_none() });
    }
    Ok(Manifest {
        name: inst.name.clone(),
        mc: inst.mc_version.clone(),
        loader: inst.loader.clone(),
        loader_version: loader_version(inst)?,
        base: plan.base.clone(),
        options: plan.options.clone().filter(|_| share_options),
        files,
        version: 0,
    })
}

async fn upload(ctx: &Ctx, token: &str, sha1: &str, path: &Path) -> Result<()> {
    let f = tokio::fs::File::open(path).await.ctx(|| path.display().to_string())?;
    let len = f.metadata().await?.len();
    let resp = ctx.http.put(format!("{}/api/blobs/{sha1}", lia::api())).bearer_auth(token).header(reqwest::header::CONTENT_LENGTH, len).body(reqwest::Body::from(f)).send().await?;
    lia::parse(resp).await.map(|_| ())
}

/// Публікує збірку (або нову версію вже опублікованої `pack_id`): вантажить свої файли й маніфест.
#[allow(clippy::too_many_arguments)]
pub async fn publish(ctx: &Ctx, inst: &Instance, token: &str, visibility: &str, description: &str, pack_id: Option<String>, sel: Selection, share_options: bool, task: &Arc<Task>) -> Result<Value> {
    let p = plan(ctx, inst, token, task).await?;
    let m = manifest(&p, inst, &sel, share_options)?;
    let game = ctx.paths.game_dir(&inst.id);
    let mut seen = HashSet::new();
    // Свої дані на кожен файл: async-блоки з позиченими посиланнями не проходять у команду Tauri.
    let todo: Vec<(String, String, u64)> = p.files.iter().filter(|f| f.kind == Kind::Upload && sel.picks(&f.path) && seen.insert(f.sha1.clone())).map(|f| (f.path.clone(), f.sha1.clone(), f.size)).collect();
    task.stage("pack_upload", todo.iter().map(|f| f.2).sum(), "bytes");
    let mut ups = futures::stream::iter(todo)
        .map(|(rel, sha1, size)| {
            let (path, task) = (game.join(&rel), task.clone());
            async move {
                task.check()?;
                upload(ctx, token, &sha1, &path).await.map_err(|e| err("lia", format!("{rel}: {e}")))?;
                task.add(size);
                Ok::<_, crate::error::Error>(())
            }
        })
        .buffer_unordered(6);
    while let Some(r) = ups.next().await {
        r?;
    }
    drop(ups);
    let body = json!({ "manifest": m, "visibility": visibility, "description": description });
    let pack = match &pack_id {
        Some(id) => lia::call(&ctx.http, reqwest::Method::PUT, &format!("/api/packs/{id}"), Some(token), Some(body)).await?,
        None => lia::call(&ctx.http, reqwest::Method::POST, "/api/packs", Some(token), Some(body)).await?,
    };
    let saved = Publishing { pack_id: pack["id"].as_str().map(str::to_string), selection: Some(sel), share_options };
    util::write_json(&publishing_path(ctx, inst), &saved)?;
    Ok(pack)
}

// ---------------------------------------------------------------- гравець

/// Що встановлено з якої версії збірки, у теці збірки.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub version: i64,
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub options: Option<Options>,
}

fn state_path(ctx: &Ctx, inst: &Instance) -> PathBuf {
    ctx.paths.instance_dir(&inst.id).join("lia-pack.json")
}

pub async fn fetch_manifest(ctx: &Ctx, token: &str, id: &str) -> Result<Manifest> {
    let v = lia::call(&ctx.http, reqwest::Method::GET, &format!("/api/packs/{id}/manifest"), Some(token), None).await?;
    Ok(serde_json::from_value(v)?)
}

/// HTTP-клієнт для файлів збірки на сервері ЛІЇ. Окремий, щоб токен не пішов на Modrinth чи GitHub.
fn lia_http(token: &str) -> Result<reqwest::Client> {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::AUTHORIZATION, format!("Bearer {token}").parse().map_err(|_| err("lia_auth", "увійдіть у ЛІЯ ID знову"))?);
    Ok(reqwest::Client::builder()
        .user_agent(net::USER_AGENT)
        .default_headers(h)
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(45))
        .build()?)
}

fn same(p: &Path, f: &PackFile) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.len() == f.size) && sha1_cached(p).is_some_and(|h| h.eq_ignore_ascii_case(&f.sha1))
}

/// Куди класти файл. Мод чи ресурспак, який гравець вимкнув (.disabled), лишається вимкненим.
fn dest_of(game: &Path, rel: &str) -> Result<PathBuf> {
    let dest = util::safe_join(game, rel)?;
    let off = dest.with_file_name(format!("{}.disabled", dest.file_name().unwrap_or_default().to_string_lossy()));
    Ok(if CONTENT.contains(&top(rel)) && !dest.exists() && off.exists() { off } else { dest })
}

/// Дістає файли з архіву базового модпака й звіряє їх SHA-1.
fn extract_base(mrpack: &Path, need: &[(PackFile, PathBuf)]) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(mrpack).ctx(|| mrpack.display().to_string())?)?;
    for (f, dest) in need {
        // client-overrides перекривають overrides.
        let name = [format!("client-overrides/{}", f.path), format!("overrides/{}", f.path)].into_iter().find(|n| zip.index_for_name(n).is_some());
        let mut e = zip.by_name(name.as_deref().ok_or_else(|| err("invalid_modpack", format!("у модпаку немає файлу {}", f.path)))?)?;
        let mut buf = vec![];
        (&mut e).take(f.size + 1).read_to_end(&mut buf)?;
        if hex::encode(sha1::Sha1::digest(&buf)) != f.sha1 {
            return Err(err("integrity", format!("{}: файл у модпаку не збігається зі збіркою", f.path)));
        }
        util::write_atomic(dest, &buf)?;
    }
    Ok(())
}

/// Допис у options.txt гравця: мова й ресурспаки автора. Під час встановлення як в автора,
/// при оновленні вмикаються лише нові ресурспаки, а мова змінюється, якщо гравець її не міняв.
fn apply_options(game: &Path, new: Option<&Options>, prev: Option<&Options>, fresh: bool) -> Result<()> {
    let Some(new) = new else { return Ok(()) };
    let path = game.join("options.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let existed = path.exists();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    fn set(lines: &mut Vec<String>, key: &str, value: String) {
        let line = format!("{key}:{value}");
        match lines.iter_mut().find(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':'))) {
            Some(l) => *l = line,
            None => lines.push(line),
        }
    }
    let fresh = fresh || !existed;
    if !new.resource_packs.is_empty() {
        let mine: Vec<String> = option(&text, "resourcePacks").and_then(|v| serde_json::from_str(v).ok()).unwrap_or_else(|| vec!["vanilla".into()]);
        let packs = if fresh {
            new.resource_packs.clone()
        } else {
            let old = prev.map(|p| p.resource_packs.as_slice()).unwrap_or_default();
            let mut m = mine.clone();
            m.extend(new.resource_packs.iter().filter(|p| !old.contains(p) && !mine.contains(p)).cloned());
            m
        };
        if packs != mine || fresh {
            set(&mut lines, "resourcePacks", serde_json::to_string(&packs)?);
        }
    }
    if let Some(lang) = &new.lang {
        let cur = option(&text, "lang");
        if fresh || cur.is_none() || (cur != Some(lang.as_str()) && cur == prev.and_then(|p| p.lang.as_deref())) {
            set(&mut lines, "lang", lang.clone());
        }
    }
    // Без цього 1.20+ на першому запуску стоїть на вітанні «Доступність» і не заходить на сервер.
    if !existed {
        set(&mut lines, "onboardAccessibility", "false".into());
    }
    let out = lines.join("\n") + "\n";
    if out != text {
        util::write_atomic(&path, out.as_bytes())?;
    }
    Ok(())
}

/// Приводить теку гри до маніфесту: качає лише те, що відрізняється, і прибирає файли попередньої версії,
/// яких більше немає. Файли, які гравець додав сам, лишаються.
async fn sync(ctx: &Ctx, inst: &mut Instance, pack: &str, m: &Manifest, prev: Option<&State>, token: &str, task: &Arc<Task>) -> Result<()> {
    let game = ctx.paths.game_dir(&inst.id);
    let (mut public, mut stored, mut from_base) = (vec![], vec![], vec![]);
    for f in &m.files {
        let dest = dest_of(&game, &f.path)?;
        if KEEP.contains(&f.path.as_str()) && dest.exists() {
            continue;
        }
        if let Some(url) = &f.url {
            if !modrinth::allowed_host(url) {
                return Err(err("invalid_modpack", format!("файл {} з недозволеного джерела", f.path)));
            }
            public.push(Dl::new(url, dest, Some(f.sha1.clone()), Some(f.size)));
        } else if f.base {
            from_base.push((f.clone(), dest));
        } else {
            stored.push(Dl::new(format!("{}/api/packs/{pack}/blobs/{}", lia::api(), f.sha1), dest, Some(f.sha1.clone()), Some(f.size)));
        }
    }
    net::download_all(&ctx.http, public, task, "modpack_files", ctx.concurrency, true).await?;
    net::download_all(&lia_http(token)?, stored, task, "pack_files", ctx.concurrency.min(8), true).await?;
    let need: Vec<(PackFile, PathBuf)> = tokio::task::spawn_blocking(move || from_base.into_iter().filter(|(f, d)| !same(d, f)).collect()).await.map_err(|e| err("io", e.to_string()))?;
    if !need.is_empty() {
        let base = m.base.as_ref().ok_or_else(|| err("invalid_modpack", "у збірці немає базового модпака"))?;
        let tmp = download_base(ctx, base, task).await?;
        task.stage("modpack_overrides", 0, "");
        let t = tmp.clone();
        let r = tokio::task::spawn_blocking(move || extract_base(&t, &need)).await.map_err(|e| err("io", e.to_string()));
        let _ = std::fs::remove_file(&tmp);
        r??;
    }

    // Прибираємо наприкінці: якщо завантаження обірветься, старі файли лишаться на місці.
    let now: HashSet<String> = m.files.iter().map(|f| f.path.to_lowercase()).collect();
    let gone: Vec<String> = match prev {
        Some(s) => s.files.keys().filter(|p| !now.contains(&p.to_lowercase()) && !KEEP.contains(&p.as_str())).cloned().collect(),
        // Збірка, встановлена до 1.1.5: тоді збірка замінювала теку mods повністю.
        None => std::fs::read_dir(game.join("mods"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| format!("mods/{}", e.file_name().to_string_lossy()))
            .filter(|p| !now.contains(&p.to_lowercase()) && !now.contains(&p.trim_end_matches(".disabled").to_lowercase()))
            .collect(),
    };
    for p in gone {
        if let Ok(d) = util::safe_join(&game, &p) {
            let _ = std::fs::remove_file(&d);
            let _ = std::fs::remove_file(d.with_file_name(format!("{}.disabled", d.file_name().unwrap_or_default().to_string_lossy())));
        }
    }
    apply_options(&game, m.options.as_ref(), prev.and_then(|s| s.options.as_ref()), prev.is_none() && inst.modpack.is_none())?;

    let changed = inst.mc_version != m.mc || inst.loader != m.loader || inst.loader_version != m.loader_version;
    inst.mc_version = m.mc.clone();
    inst.loader = m.loader.clone();
    inst.loader_version = m.loader_version.clone();
    if m.loader == "vanilla" {
        inst.version_id = None;
    } else if changed || inst.version_id.is_none() {
        inst.version_id = Some(loaders::install(ctx, &m.loader, &m.mc, m.loader_version.as_deref(), task).await?);
    }
    instances::save(&ctx.paths, inst)?;
    let state = State { version: m.version, files: m.files.iter().map(|f| (f.path.clone(), f.sha1.clone())).collect(), options: m.options.clone() };
    util::write_json(&state_path(ctx, inst), &state)
}

fn pack_ref(id: &str, m: &Manifest, owner: &str) -> ModpackRef {
    ModpackRef {
        project_id: format!("lia:{id}"),
        version_id: m.version.to_string(),
        name: m.name.clone(),
        version_name: format!("v{} · {owner}", m.version),
        icon_url: m.base.as_ref().and_then(|b| b.icon_url.clone()),
    }
}

/// Встановлює збірку друга як нову збірку.
pub async fn install(ctx: &Ctx, token: &str, id: &str, owner: &str, task: &Arc<Task>) -> Result<Instance> {
    let m = fetch_manifest(ctx, token, id).await?;
    let mut inst = instances::create(
        &ctx.paths,
        NewInstance { name: m.name.chars().take(48).collect(), mc_version: m.mc.clone(), loader: m.loader.clone(), loader_version: m.loader_version.clone(), icon: Some("block:bookshelf".into()) },
    )?;
    let r: Result<()> = async {
        sync(ctx, &mut inst, id, &m, None, token, task).await?;
        inst.modpack = Some(pack_ref(id, &m, owner));
        instances::save(&ctx.paths, &inst)?;
        modrinth::set_icon(ctx, &mut inst, m.base.as_ref().and_then(|b| b.icon_url.clone())).await
    }
    .await;
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(ctx.paths.instance_dir(&inst.id));
        return Err(e);
    }
    Ok(inst)
}

/// Оновлює збірку, встановлену зі збірки друга. Якщо оновлення обірветься, кнопка «Оновити» лишиться,
/// і повтор докачає решту: стан збереженої версії пишеться лише наприкінці.
pub async fn update(ctx: &Ctx, token: &str, inst: &mut Instance, owner: &str, task: &Arc<Task>) -> Result<()> {
    let id = lia::pack_of(inst).ok_or_else(|| err("invalid", "ця збірка не з ЛІЇ"))?;
    let m = fetch_manifest(ctx, token, &id).await?;
    let prev: Option<State> = util::read_json(&state_path(ctx, inst)).ok();
    sync(ctx, inst, &id, &m, prev.as_ref(), token, task).await?;
    inst.modpack = Some(pack_ref(&id, &m, owner));
    instances::save(&ctx.paths, inst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_versions_from_profile() {
        assert_eq!(loader_version_of("fabric", "1.21.4", "fabric-loader-0.16.10-1.21.4").as_deref(), Some("0.16.10"));
        assert_eq!(loader_version_of("quilt", "1.20.1", "quilt-loader-0.26.0-1.20.1").as_deref(), Some("0.26.0"));
        assert_eq!(loader_version_of("forge", "1.20.1", "1.20.1-forge-47.2.0").as_deref(), Some("47.2.0"));
        assert_eq!(loader_version_of("neoforge", "1.21.1", "neoforge-21.1.77").as_deref(), Some("21.1.77"));
        assert_eq!(loader_version_of("fabric", "1.21.4", "1.21.4"), None);
        let forge = Instance { loader: "forge".into(), mc_version: "1.20.1".into(), version_id: Some("1.20.1-forge-47.2.0".into()), ..Default::default() };
        assert_eq!(loader_version(&forge).unwrap().as_deref(), Some("1.20.1-47.2.0"));
    }

    #[test]
    fn never_published() {
        for p in ["saves/w/level.dat", "logs/latest.log", "options.txt", "config/discord/token.txt", "mods.lia-backup-1/a.jar", "modernfix/x"] {
            assert!(never(p), "{p}");
        }
        for p in ["mods/authme.jar", "config/sodium-options.json", "resourcepacks/ru", "kubejs/server_scripts/a.js", "servers.dat"] {
            assert!(!never(p), "{p}");
        }
    }

    #[test]
    fn selection() {
        let s = Selection { include: vec!["config".into(), "config/a/b".into(), "mods".into()], exclude: vec!["config/a".into()] };
        assert!(s.picks("config/x.toml"));
        assert!(!s.picks("config/a/y.toml"));
        assert!(s.picks("config/a/b/z.toml"));
        assert!(s.picks("mods"));
        assert!(!s.picks("modsextra/a.jar"));
        assert!(!s.picks("kubejs/a.js"));
    }

    fn plan_with(files: Vec<PlanFile>, base_files: Vec<BaseFile>) -> Plan {
        Plan { base: Some(Base { project_id: "p".into(), version_id: "v".into(), ..Default::default() }), files, selection: Selection::default(), pack_id: None, share_options: true, options: None, base_files }
    }

    fn pf(path: &str, kind: Kind, url: Option<&str>) -> PlanFile {
        PlanFile { path: path.into(), size: 1, kind, sha1: format!("{path}-local"), url: url.map(str::to_string) }
    }

    fn bf(path: &str, url: Option<&str>) -> BaseFile {
        BaseFile { path: path.into(), sha1: format!("{path}-base"), size: 2, url: url.map(str::to_string) }
    }

    #[test]
    fn manifest_from_plan() {
        let inst = Instance { name: "Тест".into(), mc_version: "1.20.1".into(), loader: "fabric".into(), loader_version: Some("0.16.10".into()), ..Default::default() };
        let plan = plan_with(
            vec![
                pf("mods/a.jar", Kind::Base, Some("https://cdn.modrinth.com/a.jar")),
                pf("config/same.toml", Kind::Base, None),
                pf("config/mine.toml", Kind::Upload, None),
                pf("resourcepacks/ru.zip", Kind::Link, Some("https://cdn.modrinth.com/ru.zip")),
                pf("shaderpacks/x.zip", Kind::Upload, None),
            ],
            vec![
                bf("mods/a.jar", Some("https://cdn.modrinth.com/a.jar")),
                bf("mods/removed.jar", Some("https://cdn.modrinth.com/r.jar")),
                bf("config/same.toml", None),
                bf("config/mine.toml", None),
                bf("shaderpacks/base.zip", None),
                bf("options.txt", None),
            ],
        );
        let sel = Selection { include: vec!["mods".into(), "config".into(), "resourcepacks".into()], exclude: vec![] };
        let m = manifest(&plan, &inst, &sel, true).unwrap();
        let get = |p: &str| m.files.iter().find(|f| f.path == p);
        assert!(get("mods/a.jar").unwrap().url.is_some());
        assert!(get("mods/removed.jar").is_none(), "видалений автором файл бази");
        assert!(get("config/same.toml").unwrap().base);
        assert_eq!(get("config/mine.toml").unwrap().sha1, "config/mine.toml-local");
        assert!(get("resourcepacks/ru.zip").unwrap().url.is_some());
        // Тека shaderpacks не вибрана: своє не йде, а файл модпака лишається.
        assert!(get("shaderpacks/x.zip").is_none());
        assert!(get("shaderpacks/base.zip").unwrap().base);
        assert!(get("options.txt").unwrap().base, "options.txt модпака потрібен на першому встановленні");
        assert_eq!(m.loader_version.as_deref(), Some("0.16.10"));

        let big = plan_with(vec![pf("config/huge.bin", Kind::Big, None)], vec![]);
        assert!(manifest(&big, &inst, &sel, true).is_err());
    }

    #[test]
    fn options_merge() {
        let dir = std::env::temp_dir().join(format!("lia-options-{}", util::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let read = || std::fs::read_to_string(dir.join("options.txt")).unwrap();
        let v1 = Options { lang: Some("uk_ua".into()), resource_packs: vec!["vanilla".into(), "file/a.zip".into()] };
        apply_options(&dir, Some(&v1), None, true).unwrap();
        let t = read();
        assert!(t.contains("lang:uk_ua") && t.contains(r#"resourcePacks:["vanilla","file/a.zip"]"#) && t.contains("onboardAccessibility:false"), "{t}");

        // Гравець вимкнув a.zip і змінив мову. Автор додав русифікатор і змінив мову.
        std::fs::write(dir.join("options.txt"), "fov:0.5\nlang:en_us\nresourcePacks:[\"vanilla\"]\n").unwrap();
        let v2 = Options { lang: Some("ru_ru".into()), resource_packs: vec!["vanilla".into(), "file/a.zip".into(), "file/ru.zip".into()] };
        apply_options(&dir, Some(&v2), Some(&v1), false).unwrap();
        let t = read();
        assert!(t.contains(r#"resourcePacks:["vanilla","file/ru.zip"]"#), "{t}");
        assert!(t.contains("lang:en_us") && t.contains("fov:0.5"), "{t}");

        // Мову, яку гравець не міняв, оновлюємо разом з автором.
        std::fs::write(dir.join("options.txt"), "lang:uk_ua\n").unwrap();
        apply_options(&dir, Some(&v2), Some(&v1), false).unwrap();
        assert!(read().contains("lang:ru_ru"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base_from_mrpack() {
        let dir = std::env::temp_dir().join(format!("lia-base-{}", util::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.mrpack");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("modrinth.index.json", o).unwrap();
        z.write_all(
            json!({ "formatVersion": 1, "game": "minecraft", "versionId": "1", "name": "B", "dependencies": { "minecraft": "1.20.1" },
                "files": [{ "path": "mods/a.jar", "hashes": { "sha1": "AB" }, "downloads": ["https://cdn.modrinth.com/a.jar"], "fileSize": 3 },
                          { "path": "mods/server.jar", "hashes": { "sha1": "cd" }, "env": { "client": "unsupported" }, "downloads": ["https://cdn.modrinth.com/s.jar"] }] })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        z.start_file("overrides/config/x.toml", o).unwrap();
        z.write_all(b"x").unwrap();
        z.start_file("client-overrides/config/x.toml", o).unwrap();
        z.write_all(b"y").unwrap();
        z.finish().unwrap();
        let files = read_base(&path).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!((files[1].path.as_str(), files[1].sha1.as_str()), ("mods/a.jar", "ab"));
        assert_eq!(files[0].sha1, hex::encode(sha1::Sha1::digest(b"y")));
        let need = vec![(PackFile { path: "config/x.toml".into(), sha1: files[0].sha1.clone(), size: 1, url: None, base: true }, dir.join("game/config/x.toml"))];
        extract_base(&path, &need).unwrap();
        assert_eq!(std::fs::read(dir.join("game/config/x.toml")).unwrap(), b"y");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
