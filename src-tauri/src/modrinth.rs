//! Modrinth: пошук, встановлення модів/ресурспаків/шейдерів із залежностями, модпаки .mrpack.
use crate::content::{self, Kind};
use crate::ctx::Ctx;
use crate::error::{err, Context, Result};
use crate::instances::{self, Instance, ModpackRef, NewInstance};
use crate::loaders;
use crate::net::{self, Dl};
use crate::tasks::Task;
use crate::util;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

const API: &str = "https://api.modrinth.com/v2";

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    #[serde(default)]
    pub query: String,
    pub project_type: String,
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: Option<u32>,
}

pub async fn search(ctx: &Ctx, p: &SearchParams) -> Result<Value> {
    let mut facets: Vec<Vec<String>> = vec![vec![format!("project_type:{}", p.project_type)]];
    if let Some(v) = p.game_version.as_ref().filter(|v| !v.is_empty()) {
        facets.push(vec![format!("versions:{v}")]);
    }
    if let Some(l) = p.loader.as_ref().filter(|l| !l.is_empty() && l.as_str() != "vanilla") {
        // Quilt запускає більшість модів Fabric.
        let mut any = vec![format!("categories:{l}")];
        if l == "quilt" {
            any.push("categories:fabric".into());
        }
        facets.push(any);
    }
    for c in &p.categories {
        facets.push(vec![format!("categories:{c}")]);
    }
    let limit = p.limit.unwrap_or(20).clamp(1, 100).to_string();
    let offset = p.offset.to_string();
    let facets = serde_json::to_string(&facets)?;
    let sort = p.sort.clone().unwrap_or_else(|| "relevance".into());
    let resp = ctx
        .http
        .get(format!("{API}/search"))
        .query(&[("query", p.query.as_str()), ("facets", &facets), ("index", &sort), ("limit", &limit), ("offset", &offset)])
        .send()
        .await
        .ctx(|| "api.modrinth.com".into())?;
    if !resp.status().is_success() {
        return Err(err("http", format!("Modrinth: HTTP {}", resp.status().as_u16())));
    }
    Ok(resp.json().await?)
}

fn check_slug(id: &str) -> Result<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(err("invalid", format!("неприпустимий ідентифікатор Modrinth: {id}")));
    }
    Ok(())
}

pub async fn project(ctx: &Ctx, id: &str) -> Result<Value> {
    check_slug(id)?;
    net::get_json(&ctx.http, &format!("{API}/project/{id}")).await
}

pub async fn project_versions(ctx: &Ctx, id: &str, loaders: &[String], game_versions: &[String]) -> Result<Vec<Value>> {
    check_slug(id)?;
    let mut q: Vec<(&str, String)> = Vec::new();
    if !loaders.is_empty() {
        q.push(("loaders", serde_json::to_string(loaders)?));
    }
    if !game_versions.is_empty() {
        q.push(("game_versions", serde_json::to_string(game_versions)?));
    }
    let resp = ctx.http.get(format!("{API}/project/{id}/version")).query(&q).send().await?;
    if !resp.status().is_success() {
        return Err(err("http", format!("Modrinth: HTTP {}", resp.status().as_u16())));
    }
    Ok(resp.json().await?)
}

pub async fn version(ctx: &Ctx, id: &str) -> Result<Value> {
    check_slug(id)?;
    net::get_json(&ctx.http, &format!("{API}/version/{id}")).await
}

/// Модлоадери, файли яких підходять збірці.
fn loaders_for(inst: &Instance, kind: Kind) -> Vec<String> {
    match kind {
        Kind::Mods => match inst.loader.as_str() {
            "quilt" => vec!["quilt".into(), "fabric".into()],
            l => vec![l.to_string()],
        },
        Kind::Shaders => vec![],
        Kind::ResourcePacks => vec![],
    }
}

fn kind_of(project_type: &str) -> Result<Kind> {
    match project_type {
        "mod" => Ok(Kind::Mods),
        "resourcepack" => Ok(Kind::ResourcePacks),
        "shader" => Ok(Kind::Shaders),
        other => Err(err("invalid", format!("тип «{other}» не встановлюється в збірку"))),
    }
}

/// Найкраща версія проєкту для збірки: спершу release, потім beta/alpha.
async fn best_version(ctx: &Ctx, project_id: &str, inst: &Instance, kind: Kind) -> Result<Option<Value>> {
    let loaders = loaders_for(inst, kind);
    let mut list = project_versions(ctx, project_id, &loaders, std::slice::from_ref(&inst.mc_version)).await?;
    if list.is_empty() && kind == Kind::Shaders {
        // Шейдери часто не позначають кожну версію гри.
        list = project_versions(ctx, project_id, &[], &[]).await?;
    }
    let pick = list.iter().find(|v| v["version_type"] == "release").or(list.first()).cloned();
    Ok(pick)
}

fn primary_file(v: &Value) -> Option<&Value> {
    let files = v["files"].as_array()?;
    files.iter().find(|f| f["primary"] == true).or(files.first())
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub project_id: String,
    pub title: String,
    pub file: String,
}

/// Встановлює проєкт (і обов'язкові залежності модів) у збірку.
pub async fn install(ctx: &Ctx, inst: &Instance, project_id: &str, version_id: Option<&str>, task: &Arc<Task>) -> Result<Vec<Installed>> {
    let proj = project(ctx, project_id).await?;
    let kind = kind_of(proj["project_type"].as_str().unwrap_or(""))?;
    let root_version = match version_id {
        Some(v) => version(ctx, v).await?,
        None => best_version(ctx, proj["id"].as_str().unwrap_or(project_id), inst, kind)
            .await?
            .ok_or_else(|| err("no_compatible_version", format!("{} не має версії для {} {}", proj["title"].as_str().unwrap_or(project_id), inst.loader, inst.mc_version)))?,
    };

    let mut index = content::Index::load(&ctx.paths, &inst.id);
    let installed_projects: HashSet<String> = index.project_ids();
    let mut queue = vec![(root_version, proj)];
    let mut visited = HashSet::new();
    let mut plan: Vec<(Value, Value)> = Vec::new();
    while let Some((ver, p)) = queue.pop() {
        let pid = ver["project_id"].as_str().unwrap_or("").to_string();
        if !visited.insert(pid.clone()) {
            continue;
        }
        if kind == Kind::Mods {
            for dep in ver["dependencies"].as_array().cloned().unwrap_or_default() {
                if dep["dependency_type"] != "required" {
                    continue;
                }
                let dep_pid = dep["project_id"].as_str().map(str::to_string);
                if dep_pid.as_ref().is_some_and(|d| installed_projects.contains(d) || visited.contains(d)) {
                    continue;
                }
                let dep_ver = match (dep["version_id"].as_str(), &dep_pid) {
                    (Some(vid), _) => Some(version(ctx, vid).await?),
                    (None, Some(pid)) => best_version(ctx, pid, inst, Kind::Mods).await?,
                    _ => None,
                };
                if let Some(dv) = dep_ver {
                    let dp = project(ctx, dv["project_id"].as_str().unwrap_or("")).await?;
                    queue.push((dv, dp));
                }
            }
        }
        plan.push((ver, p));
    }

    let folder = ctx.paths.game_dir(&inst.id).join(kind.folder());
    let mut dls = Vec::new();
    let mut result = Vec::new();
    for (ver, p) in &plan {
        let f = primary_file(ver).ok_or_else(|| err("invalid", "у версії немає файлів"))?;
        let filename = f["filename"].as_str().unwrap_or("");
        let path = util::safe_join(&folder, filename)?;
        if path.parent() != Some(folder.as_path()) {
            return Err(err("invalid", format!("підозріла назва файлу: {filename}")));
        }
        dls.push(Dl::new(f["url"].as_str().unwrap_or(""), path, f["hashes"]["sha1"].as_str().map(str::to_string), f["size"].as_u64()));
        let pid = ver["project_id"].as_str().unwrap_or("").to_string();
        // Інша версія того самого проєкту — прибираємо, щоб не було дублікатів.
        for old in index.files_of(&pid) {
            if old != filename {
                let _ = std::fs::remove_file(folder.join(&old));
                let _ = std::fs::remove_file(folder.join(format!("{old}.disabled")));
                index.remove(kind, &old);
            }
        }
        index.set(
            kind,
            filename,
            content::Meta {
                project_id: pid.clone(),
                version_id: ver["id"].as_str().unwrap_or("").into(),
                version_number: ver["version_number"].as_str().unwrap_or("").into(),
                title: p["title"].as_str().unwrap_or("").into(),
                icon_url: p["icon_url"].as_str().map(str::to_string),
                author: None,
            },
        );
        result.push(Installed { project_id: pid, title: p["title"].as_str().unwrap_or("").into(), file: filename.into() });
    }
    net::download_all(&ctx.http, dls, task, "content", ctx.concurrency, true).await?;
    index.save(&ctx.paths, &inst.id)?;
    Ok(result)
}

// ---------------------------------------------------------------- модпаки

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrIndex {
    name: String,
    #[serde(default)]
    version_id: String,
    files: Vec<MrFile>,
    dependencies: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrFile {
    path: String,
    hashes: HashMap<String, String>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
    downloads: Vec<String>,
    #[serde(default)]
    file_size: Option<u64>,
}

/// Хости, з яких модпак може тягнути файли (специфікація формату .mrpack).
fn allowed_host(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    u.scheme() == "https" && matches!(u.host_str(), Some("cdn.modrinth.com" | "github.com" | "raw.githubusercontent.com" | "gitlab.com"))
}

/// Встановлення модпака з Modrinth: нова збірка.
pub async fn install_modpack(ctx: &Ctx, version_id: &str, task: &Arc<Task>) -> Result<Instance> {
    let ver = version(ctx, version_id).await?;
    let proj = project(ctx, ver["project_id"].as_str().unwrap_or("")).await?;
    let f = primary_file(&ver).ok_or_else(|| err("invalid", "у модпака немає файлу"))?;
    let tmp = ctx.paths.temp().join(format!("modpack-{}.mrpack", util::now_ms()));
    task.stage("modpack", 0, "");
    net::download(&ctx.http, &Dl::new(f["url"].as_str().unwrap_or(""), tmp.clone(), f["hashes"]["sha1"].as_str().map(str::to_string), f["size"].as_u64()), Some(task)).await?;
    let modpack = ModpackRef {
        project_id: proj["id"].as_str().unwrap_or("").into(),
        version_id: version_id.into(),
        name: proj["title"].as_str().unwrap_or("").into(),
        version_name: ver["version_number"].as_str().unwrap_or("").into(),
        icon_url: proj["icon_url"].as_str().map(str::to_string),
    };
    let icon_url = modpack.icon_url.clone();
    let r = import_mrpack(ctx, &tmp, Some(modpack), task).await;
    let _ = std::fs::remove_file(&tmp);
    let mut inst = r?;
    // Іконка модпака замість стандартного блока (не критично, якщо не вийшло).
    if let Some(url) = icon_url.filter(|u| u.starts_with("https://cdn.modrinth.com/")) {
        if let Ok(bytes) = net::get_bytes(&ctx.http, &url).await {
            if util::write_atomic(&ctx.paths.instance_dir(&inst.id).join("icon.png"), &bytes).is_ok() {
                inst.icon = format!("custom:{}", util::now_ms());
                instances::save(&ctx.paths, &inst)?;
            }
        }
    }
    Ok(inst)
}

/// Імпорт файлу .mrpack: створює збірку, качає файли, розпаковує overrides, ставить модлоадер.
pub async fn import_mrpack(ctx: &Ctx, path: &Path, modpack: Option<ModpackRef>, task: &Arc<Task>) -> Result<Instance> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ctx(|| path.display().to_string())?)?;
    let index: MrIndex = {
        let mut e = zip.by_name("modrinth.index.json").map_err(|_| err("invalid_modpack", "це не модпак Modrinth (немає modrinth.index.json)"))?;
        let mut s = String::new();
        e.read_to_string(&mut s)?;
        serde_json::from_str(&s)?
    };
    let mc = index.dependencies.get("minecraft").cloned().ok_or_else(|| err("invalid_modpack", "у модпаку не вказано версію гри"))?;
    let (loader, loader_version) = [("fabric-loader", "fabric"), ("quilt-loader", "quilt"), ("neoforge", "neoforge"), ("forge", "forge")]
        .iter()
        .find_map(|(key, l)| index.dependencies.get(*key).map(|v| (l.to_string(), Some(v.clone()))))
        .unwrap_or(("vanilla".into(), None));
    // Forge у профілі Modrinth вказаний без префікса версії гри.
    let loader_version = match (loader.as_str(), loader_version) {
        ("forge", Some(v)) if !v.starts_with(&format!("{mc}-")) => Some(format!("{mc}-{v}")),
        (_, v) => v,
    };

    let name = modpack.as_ref().map(|m| m.name.clone()).filter(|n| !n.is_empty()).unwrap_or(index.name.clone());
    let mut inst = instances::create(
        &ctx.paths,
        NewInstance { name: name.chars().take(48).collect(), mc_version: mc.clone(), loader: loader.clone(), loader_version: loader_version.clone(), icon: Some("block:bookshelf".into()) },
    )?;
    let result: Result<()> = async {
        let game = ctx.paths.game_dir(&inst.id);
        let mut dls = Vec::new();
        for f in &index.files {
            if f.env.as_ref().and_then(|e| e.get("client")).is_some_and(|c| c == "unsupported") {
                continue;
            }
            let dest = util::safe_join(&game, &f.path)?;
            let url = f.downloads.iter().find(|u| allowed_host(u)).ok_or_else(|| err("invalid_modpack", format!("файл {} з недозволеного джерела", f.path)))?;
            dls.push(Dl::new(url, dest, f.hashes.get("sha1").cloned(), f.file_size));
        }
        net::download_all(&ctx.http, dls, task, "modpack_files", ctx.concurrency, true).await?;

        task.stage("modpack_overrides", 0, "");
        for i in 0..zip.len() {
            let mut e = zip.by_index(i)?;
            let name = e.name()?.to_string();
            let rel = name.strip_prefix("overrides/").or_else(|| name.strip_prefix("client-overrides/"));
            let Some(rel) = rel.filter(|r| !r.is_empty()) else { continue };
            let dest = util::safe_join(&game, rel)?;
            if e.is_dir() {
                std::fs::create_dir_all(&dest)?;
                continue;
            }
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut out = std::fs::File::create(&dest).ctx(|| dest.display().to_string())?;
            std::io::copy(&mut e, &mut out)?;
        }

        if loader != "vanilla" {
            inst.version_id = Some(loaders::install(ctx, &loader, &mc, loader_version.as_deref(), task).await?);
        }
        let mut m = modpack.clone().unwrap_or_default();
        if m.version_name.is_empty() {
            m.version_name = index.version_id.clone();
        }
        if m.name.is_empty() {
            m.name = index.name.clone();
        }
        inst.modpack = Some(m);
        instances::save(&ctx.paths, &inst)
    }
    .await;
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(ctx.paths.instance_dir(&inst.id));
        return Err(e);
    }
    Ok(inst)
}

/// Ідентифікація файлів за SHA1 (моди, додані вручну) — назви й іконки з Modrinth.
pub async fn identify(ctx: &Ctx, hashes: &[String]) -> Result<HashMap<String, (Value, Value)>> {
    if hashes.is_empty() {
        return Ok(HashMap::new());
    }
    let resp = ctx.http.post(format!("{API}/version_files")).json(&json!({ "hashes": hashes, "algorithm": "sha1" })).send().await?;
    let versions: HashMap<String, Value> = resp.json().await.unwrap_or_default();
    let ids: Vec<String> = versions.values().filter_map(|v| v["project_id"].as_str().map(str::to_string)).collect::<HashSet<_>>().into_iter().collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let projects: Vec<Value> = ctx.http.get(format!("{API}/projects")).query(&[("ids", serde_json::to_string(&ids)?)]).send().await?.json().await?;
    let by_id: HashMap<String, Value> = projects.into_iter().filter_map(|p| Some((p["id"].as_str()?.to_string(), p))).collect();
    Ok(versions
        .into_iter()
        .filter_map(|(h, v)| {
            let p = by_id.get(v["project_id"].as_str()?)?.clone();
            Some((h, (v, p)))
        })
        .collect())
}

/// Нові версії для встановлених файлів: hash → версія Modrinth.
pub async fn updates(ctx: &Ctx, hashes: &[String], inst: &Instance, kind: Kind) -> Result<HashMap<String, Value>> {
    if hashes.is_empty() {
        return Ok(HashMap::new());
    }
    let mut body = json!({ "hashes": hashes, "algorithm": "sha1", "game_versions": [inst.mc_version] });
    let loaders = loaders_for(inst, kind);
    if !loaders.is_empty() {
        body["loaders"] = json!(loaders);
    }
    let resp = ctx.http.post(format!("{API}/version_files/update")).json(&body).send().await?;
    let latest: HashMap<String, Value> = resp.json().await.unwrap_or_default();
    // Стабільному моду бету не пропонуємо: Modrinth повертає найновішу версію будь-якого типу.
    let current: HashMap<String, Value> = ctx
        .http
        .post(format!("{API}/version_files"))
        .json(&json!({ "hashes": hashes, "algorithm": "sha1" }))
        .send()
        .await?
        .json()
        .await
        .unwrap_or_default();
    let mut out = HashMap::new();
    for (hash, new) in latest {
        let cur = current.get(&hash);
        let cur_release = cur.is_none_or(|c| c["version_type"] == "release");
        if new["version_type"] == "release" || !cur_release {
            out.insert(hash, new);
            continue;
        }
        // Найновіша — бета, а встановлено стабільну: шукаємо найновіший стабільний реліз.
        let pid = new["project_id"].as_str().unwrap_or("");
        let Ok(list) = project_versions(ctx, pid, &loaders, std::slice::from_ref(&inst.mc_version)).await else { continue };
        if let Some(rel) = list.into_iter().find(|v| v["version_type"] == "release") {
            let newer = cur.is_none_or(|c| rel["id"] != c["id"] && rel["date_published"].as_str() > c["date_published"].as_str());
            if newer {
                out.insert(hash, rel);
            }
        }
    }
    Ok(out)
}
