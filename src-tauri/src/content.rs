//! Встановлений вміст збірки: моди, ресурспаки, шейдери, світи, скріншоти.
use crate::error::{err, Context, Result};
use crate::paths::Paths;
use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Mods,
    ResourcePacks,
    Shaders,
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind> {
        match s {
            "mods" => Ok(Kind::Mods),
            "resourcepacks" => Ok(Kind::ResourcePacks),
            "shaderpacks" => Ok(Kind::Shaders),
            _ => Err(err("invalid", format!("невідомий тип вмісту {s}"))),
        }
    }

    pub fn folder(self) -> &'static str {
        match self {
            Kind::Mods => "mods",
            Kind::ResourcePacks => "resourcepacks",
            Kind::Shaders => "shaderpacks",
        }
    }

    fn accepts(self, name: &str) -> bool {
        let n = name.to_lowercase();
        let n = n.strip_suffix(".disabled").unwrap_or(&n);
        match self {
            Kind::Mods => n.ends_with(".jar"),
            _ => n.ends_with(".zip"),
        }
    }
}

/// Що відомо про файл з Modrinth (щоб показати назву, іконку й шукати оновлення).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub project_id: String,
    pub version_id: String,
    pub version_number: String,
    pub title: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Index {
    #[serde(default)]
    files: HashMap<String, Meta>,
}

fn base_name(file: &str) -> &str {
    file.strip_suffix(".disabled").unwrap_or(file)
}

impl Index {
    fn path(paths: &Paths, id: &str) -> PathBuf {
        paths.instance_dir(id).join("content.json")
    }

    pub fn load(paths: &Paths, id: &str) -> Index {
        util::read_json(&Self::path(paths, id)).unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths, id: &str) -> Result<()> {
        util::write_json(&Self::path(paths, id), self)
    }

    fn key(kind: Kind, file: &str) -> String {
        format!("{}/{}", kind.folder(), base_name(file))
    }

    pub fn get(&self, kind: Kind, file: &str) -> Option<&Meta> {
        self.files.get(&Self::key(kind, file))
    }

    pub fn set(&mut self, kind: Kind, file: &str, meta: Meta) {
        self.files.insert(Self::key(kind, file), meta);
    }

    pub fn remove(&mut self, kind: Kind, file: &str) {
        self.files.remove(&Self::key(kind, file));
    }

    pub fn project_ids(&self) -> HashSet<String> {
        self.files.values().map(|m| m.project_id.clone()).collect()
    }

    /// Імена файлів (без теки) цього проєкту.
    pub fn files_of(&self, project_id: &str) -> Vec<String> {
        self.files
            .iter()
            .filter(|(_, m)| m.project_id == project_id)
            .filter_map(|(k, _)| k.split_once('/').map(|(_, f)| f.to_string()))
            .collect()
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub file: String,
    pub name: String,
    pub enabled: bool,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
    pub project_id: Option<String>,
    pub version: Option<String>,
    pub icon_url: Option<String>,
    pub description: Option<String>,
}

fn modified_ms(m: &std::fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

fn zip_text(path: &Path, names: &[&str]) -> Option<(String, String)> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    for n in names {
        if let Ok(mut e) = zip.by_name(n) {
            if e.size() > 512 * 1024 {
                continue;
            }
            let mut s = String::new();
            if e.read_to_string(&mut s).is_ok() {
                return Some((n.to_string(), s));
            }
        }
    }
    None
}

/// Назва й версія мода з його власного опису (fabric.mod.json, mods.toml, mcmod.info).
fn jar_meta(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let files = ["fabric.mod.json", "quilt.mod.json", "META-INF/neoforge.mods.toml", "META-INF/mods.toml", "mcmod.info"];
    let Some((name, text)) = zip_text(path, &files) else { return (None, None, None) };
    let s = |v: &serde_json::Value| v.as_str().map(str::to_string).filter(|s| !s.is_empty());
    match name.as_str() {
        "fabric.mod.json" => {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            (s(&v["name"]), s(&v["version"]), s(&v["description"]))
        }
        "quilt.mod.json" => {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let q = &v["quilt_loader"];
            (s(&q["metadata"]["name"]), s(&q["version"]), s(&q["metadata"]["description"]))
        }
        "mcmod.info" => {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let m = if v.is_array() { &v[0] } else { &v["modList"][0] };
            (s(&m["name"]), s(&m["version"]), s(&m["description"]))
        }
        _ => {
            let v: toml::Value = toml::from_str(&text).unwrap_or(toml::Value::Table(Default::default()));
            let m = v.get("mods").and_then(|m| m.as_array()).and_then(|a| a.first());
            let get = |k: &str| m.and_then(|m| m.get(k)).and_then(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
            let version = get("version").filter(|v| !v.contains("${"));
            (get("displayName"), version, get("description"))
        }
    }
}

pub fn list(paths: &Paths, inst_id: &str, kind: Kind) -> Vec<Item> {
    let dir = paths.game_dir(inst_id).join(kind.folder());
    let index = Index::load(paths, inst_id);
    let mut out: Vec<Item> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().to_string();
            let meta = e.metadata().ok()?;
            let is_dir = meta.is_dir();
            if !(is_dir && kind != Kind::Mods) && !kind.accepts(&file) {
                return None;
            }
            let enabled = !file.ends_with(".disabled");
            let known = index.get(kind, &file);
            let (jar_name, jar_version, jar_desc) = if known.is_none() && kind == Kind::Mods { jar_meta(&e.path()) } else { (None, None, None) };
            let stem = base_name(&file).trim_end_matches(".jar").trim_end_matches(".zip").to_string();
            Some(Item {
                name: known.map(|m| m.title.clone()).filter(|t| !t.is_empty()).or(jar_name).unwrap_or(stem),
                version: known.map(|m| m.version_number.clone()).filter(|v| !v.is_empty()).or(jar_version),
                project_id: known.map(|m| m.project_id.clone()),
                icon_url: known.and_then(|m| m.icon_url.clone()),
                description: jar_desc,
                size: if is_dir { 0 } else { meta.len() },
                modified: modified_ms(&meta),
                enabled,
                is_dir,
                file,
            })
        })
        .collect();
    out.sort_by_key(|i| i.name.to_lowercase());
    out
}

fn item_path(paths: &Paths, inst_id: &str, kind: Kind, file: &str) -> Result<PathBuf> {
    let dir = paths.game_dir(inst_id).join(kind.folder());
    let p = util::safe_join(&dir, file)?;
    if p.parent() != Some(dir.as_path()) {
        return Err(err("invalid", format!("неприпустиме ім'я файлу {file}")));
    }
    Ok(p)
}

/// Вмикає/вимикає (суфікс .disabled — так роблять усі лаунчери). Повертає нове ім'я.
pub fn toggle(paths: &Paths, inst_id: &str, kind: Kind, file: &str) -> Result<String> {
    let from = item_path(paths, inst_id, kind, file)?;
    let new_name = match file.strip_suffix(".disabled") {
        Some(base) => base.to_string(),
        None => format!("{file}.disabled"),
    };
    let to = item_path(paths, inst_id, kind, &new_name)?;
    std::fs::rename(&from, &to).ctx(|| from.display().to_string())?;
    Ok(new_name)
}

pub fn remove(paths: &Paths, inst_id: &str, kind: Kind, file: &str) -> Result<()> {
    let p = item_path(paths, inst_id, kind, file)?;
    if trash::delete(&p).is_err() {
        if p.is_dir() {
            std::fs::remove_dir_all(&p)?;
        } else {
            std::fs::remove_file(&p)?;
        }
    }
    let mut index = Index::load(paths, inst_id);
    index.remove(kind, file);
    index.save(paths, inst_id)
}

/// Копіює файли, перетягнуті у вікно або вибрані в діалозі.
pub fn add_files(paths: &Paths, inst_id: &str, kind: Kind, files: &[PathBuf]) -> Result<usize> {
    let dir = paths.game_dir(inst_id).join(kind.folder());
    std::fs::create_dir_all(&dir)?;
    let mut n = 0;
    for f in files {
        let Some(name) = f.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
        if !kind.accepts(&name) || !f.is_file() {
            continue;
        }
        let dest = dir.join(&name);
        std::fs::copy(f, &dest).ctx(|| dest.display().to_string())?;
        n += 1;
    }
    Ok(n)
}

/// SHA1 файлів (для пошуку на Modrinth). Теки пропускаються.
pub fn hashes(paths: &Paths, inst_id: &str, kind: Kind, files: &[String]) -> HashMap<String, String> {
    let dir = paths.game_dir(inst_id).join(kind.folder());
    files
        .iter()
        .filter_map(|f| {
            let p = dir.join(f);
            p.is_file().then(|| util::sha1_file(&p).ok().map(|h| (h, f.clone()))).flatten()
        })
        .collect()
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct World {
    pub folder: String,
    pub icon: Option<String>,
    pub modified: i64,
    pub size: u64,
}

pub fn worlds(paths: &Paths, inst_id: &str) -> Vec<World> {
    let dir = paths.game_dir(inst_id).join("saves");
    let mut out: Vec<World> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("level.dat").exists())
        .map(|e| {
            let p = e.path();
            let icon = p.join("icon.png");
            World {
                folder: e.file_name().to_string_lossy().to_string(),
                icon: icon.exists().then(|| icon.display().to_string()),
                modified: std::fs::metadata(p.join("level.dat")).map(|m| modified_ms(&m)).unwrap_or(0),
                size: util::dir_size(&p),
            }
        })
        .collect();
    out.sort_by_key(|w| std::cmp::Reverse(w.modified));
    out
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Shot {
    pub path: String,
    pub name: String,
    pub modified: i64,
}

pub fn screenshots(paths: &Paths, inst_id: &str) -> Vec<Shot> {
    let dir = paths.game_dir(inst_id).join("screenshots");
    let mut out: Vec<Shot> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().to_lowercase().ends_with(".png"))
        .map(|e| Shot {
            path: e.path().display().to_string(),
            name: e.file_name().to_string_lossy().to_string(),
            modified: e.metadata().map(|m| modified_ms(&m)).unwrap_or(0),
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.modified));
    out
}

/// Шлях усередині теки гри (для «відкрити теку», видалення світу тощо).
pub fn game_path(paths: &Paths, inst_id: &str, rel: &str) -> Result<PathBuf> {
    let base = paths.game_dir(inst_id);
    if rel.is_empty() {
        return Ok(base);
    }
    util::safe_join(&base, rel)
}
