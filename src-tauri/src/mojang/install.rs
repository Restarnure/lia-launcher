use super::rules::Env;
use super::version::VersionJson;
use crate::ctx::Ctx;
use crate::error::{err, Context, Result};
use crate::net::{self, Dl};
use crate::tasks::Task;
use crate::util;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RESOURCES_URL: &str = "https://resources.download.minecraft.net";

/// Ідентифікатор версії стає частиною шляху — не пускаємо роздільники й `..`.
pub fn check_id(id: &str) -> Result<()> {
    let bad = id.is_empty() || id == "." || id == ".." || id.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'));
    if bad {
        return Err(err("invalid", format!("неприпустимий ідентифікатор версії: {id}")));
    }
    Ok(())
}

/// JSON версії: з диска, а для ванільних — з мережі за маніфестом Mojang
/// (і повторно, якщо Mojang оновив файл, наприклад після виправлення безпеки).
pub async fn load_json(ctx: &Ctx, id: &str) -> Result<VersionJson> {
    check_id(id)?;
    let path = ctx.paths.version_json(id);
    let local = util::read_json::<VersionJson>(&path).ok();
    let manifest = super::manifest(ctx, false).await;
    let entry = manifest.as_ref().ok().and_then(|m| m.versions.iter().find(|v| v.id == id).cloned());
    let fresh = match (&local, &entry) {
        (Some(_), Some(e)) => match &e.sha1 {
            Some(sha) => util::sha1_file(&path).is_ok_and(|h| h.eq_ignore_ascii_case(sha)),
            None => true,
        },
        (Some(_), None) => true,
        (None, _) => false,
    };
    if fresh {
        return Ok(local.unwrap());
    }
    let Some(entry) = entry else {
        return Err(match manifest {
            Err(e) => e,
            Ok(_) => err("version_not_found", format!("версію {id} не знайдено")),
        });
    };
    match net::download(&ctx.http, &Dl::new(&entry.url, path.clone(), entry.sha1.clone(), None), None).await {
        Ok(()) => util::read_json(&path),
        Err(e) => local.ok_or(e),
    }
}

/// Повний профіль з урахуванням `inheritsFrom` (Fabric → ванільна версія тощо).
pub async fn resolve(ctx: &Ctx, id: &str) -> Result<VersionJson> {
    let mut chain = vec![load_json(ctx, id).await?];
    while let Some(parent) = chain.last().unwrap().inherits_from.clone() {
        if chain.len() > 8 {
            return Err(err("invalid", format!("занадто глибоке наслідування версії {id}")));
        }
        chain.push(load_json(ctx, &parent).await?);
    }
    let mut merged = chain.pop().unwrap();
    while let Some(child) = chain.pop() {
        merged = child.merge_onto(merged);
    }
    Ok(merged)
}

#[derive(Deserialize)]
struct AssetIndex {
    #[serde(default)]
    objects: HashMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    is_virtual: bool,
    #[serde(default)]
    map_to_resources: bool,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

pub struct Prepared {
    pub json: VersionJson,
    pub classpath: Vec<PathBuf>,
    /// Старі нативні jar-и та шаблони виключень для розпаковки.
    pub natives: Vec<(PathBuf, Vec<String>)>,
    pub asset_index: String,
    /// Куди вказує `${game_assets}` для старих версій (None = звичайна тека assets).
    pub legacy_assets: Option<LegacyAssets>,
    pub log_config: Option<(PathBuf, String)>,
}

pub enum LegacyAssets {
    /// `assets/virtual/<id>` — версії 1.6–1.7.2.
    Virtual(PathBuf),
    /// Копія в `<гра>/resources` — версії до 1.6.
    Resources(Vec<(String, PathBuf)>),
}

/// Готує все для запуску: client.jar, бібліотеки, нативні бібліотеки, ресурси, конфіг логування.
/// `verify` — перевірити SHA1 кожного файлу (режим «полагодити»).
pub async fn prepare(ctx: &Ctx, id: &str, task: &Arc<Task>, verify: bool) -> Result<Prepared> {
    task.stage("version", 0, "");
    let env = Env::current();
    let json = resolve(ctx, id).await?;
    let jar_id = json.jar.clone().unwrap_or_else(|| json.id.clone());
    check_id(&jar_id)?;

    let mut dls = Vec::new();
    let mut classpath = Vec::new();
    let mut natives = Vec::new();
    let mut missing_local = Vec::new();

    for lib in json.libraries.iter().filter(|l| l.allowed(&env)) {
        if let Some(a) = lib.artifact() {
            let path = ctx.paths.library(&a.path);
            match a.url {
                Some(url) => dls.push(Dl::new(url, path.clone(), a.sha1, a.size)),
                None if !path.exists() => missing_local.push(lib.name.clone()),
                None => {}
            }
            classpath.push(path);
        }
        if let Some(n) = lib.native(&env) {
            let path = ctx.paths.library(&n.path);
            if let Some(url) = n.url {
                dls.push(Dl::new(url, path.clone(), n.sha1, n.size));
            }
            let exclude = lib.extract.as_ref().map(|e| e.exclude.clone()).unwrap_or_default();
            natives.push((path, exclude));
        }
    }
    if !missing_local.is_empty() {
        return Err(err(
            "loader_broken",
            format!("бракує файлів модлоадера ({}). Перевстановіть модлоадер у налаштуваннях збірки.", missing_local.join(", ")),
        ));
    }

    let client_jar = ctx.paths.version_jar(&jar_id);
    match json.downloads.as_ref().and_then(|d| d.get("client")) {
        Some(c) => dls.push(Dl::new(&c.url, client_jar.clone(), c.sha1.clone(), c.size)),
        None if !client_jar.exists() => return Err(err("invalid", format!("у профілі {id} немає client.jar"))),
        None => {}
    }
    // Як в офіційному лаунчері: клієнт лежить під іменем профілю, що запускається.
    // NeoForge/Forge 1.17+ виключають з модулів саме `${version_name}.jar` (-DignoreList).
    let launch_jar = ctx.paths.version_jar(&json.id);
    classpath.push(launch_jar.clone());

    let log_config = json.logging.as_ref().and_then(|l| l.client.as_ref()).map(|c| {
        let path = ctx.paths.log_config(&c.file.id);
        dls.push(Dl::new(&c.file.url, path.clone(), c.file.sha1.clone(), c.file.size));
        (path, c.argument.clone())
    });

    let index_ref = json.asset_index.clone().ok_or_else(|| err("invalid", format!("у профілі {id} немає assetIndex")))?;
    check_id(&index_ref.id)?;
    let index_path = ctx.paths.asset_index(&index_ref.id);
    let index_dl = Dl::new(&index_ref.url, index_path.clone(), index_ref.sha1.clone(), index_ref.size);
    net::download_all(&ctx.http, vec![index_dl], task, "version", 1, verify).await?;
    let index: AssetIndex = util::read_json(&index_path)?;
    for obj in index.objects.values() {
        if obj.hash.len() < 2 {
            continue;
        }
        let url = format!("{RESOURCES_URL}/{}/{}", &obj.hash[..2], obj.hash);
        dls.push(Dl::new(url, ctx.paths.asset_object(&obj.hash), Some(obj.hash.clone()), Some(obj.size)));
    }

    task.check()?;
    net::download_all(&ctx.http, dls, task, "files", ctx.concurrency, verify).await?;
    if launch_jar != client_jar {
        link_jar(&client_jar, &launch_jar)?;
    }

    let legacy_assets = if index.is_virtual {
        let dir = ctx.paths.assets().join("virtual").join(&index_ref.id);
        let pairs: Vec<(String, PathBuf)> =
            index.objects.iter().map(|(name, o)| (name.clone(), ctx.paths.asset_object(&o.hash))).collect();
        task.stage("assets_copy", pairs.len() as u64, "items");
        copy_assets(&pairs, &dir, task)?;
        Some(LegacyAssets::Virtual(dir))
    } else if index.map_to_resources {
        Some(LegacyAssets::Resources(index.objects.iter().map(|(n, o)| (n.clone(), ctx.paths.asset_object(&o.hash))).collect()))
    } else {
        None
    };

    Ok(Prepared { json, classpath, natives, asset_index: index_ref.id, legacy_assets, log_config })
}

/// Жорстке посилання (без зайвих 25+ МБ), а якщо файлова система не вміє — копія.
fn link_jar(src: &Path, dest: &Path) -> Result<()> {
    let same = std::fs::metadata(dest).ok().zip(std::fs::metadata(src).ok()).is_some_and(|(a, b)| a.len() == b.len());
    if same {
        return Ok(());
    }
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    let _ = std::fs::remove_file(dest);
    if std::fs::hard_link(src, dest).is_err() {
        std::fs::copy(src, dest).ctx(|| dest.display().to_string())?;
    }
    Ok(())
}

/// Копіює об'єкти ресурсів під їхніми справжніми іменами (для версій до 1.7.3).
pub fn copy_assets(pairs: &[(String, PathBuf)], dir: &Path, task: &Task) -> Result<()> {
    for (name, src) in pairs {
        let dest = util::safe_join(dir, name)?;
        let same = std::fs::metadata(&dest).ok().zip(std::fs::metadata(src).ok()).is_some_and(|(a, b)| a.len() == b.len());
        if !same {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(src, &dest).ctx(|| dest.display().to_string())?;
        }
        task.add(1);
    }
    Ok(())
}

/// Розпаковує старі нативні бібліотеки (.dll/.so) у теку запуску.
pub fn extract_natives(natives: &[(PathBuf, Vec<String>)], dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for (jar, exclude) in natives {
        let file = std::fs::File::open(jar).ctx(|| jar.display().to_string())?;
        let mut zip = zip::ZipArchive::new(file).ctx(|| jar.display().to_string())?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let name = entry.name()?.to_string();
            if entry.is_dir() || exclude.iter().any(|ex| name.starts_with(ex.as_str())) {
                continue;
            }
            let Ok(dest) = util::safe_join(dir, &name) else { continue };
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut out = std::fs::File::create(&dest).ctx(|| dest.display().to_string())?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}
