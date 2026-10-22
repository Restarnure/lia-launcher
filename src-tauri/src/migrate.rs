//! Перенесення з інших лаунчерів: світи, ресурспаки, налаштування й сервери
//! з теки `.minecraft` у нову збірку ЛІЇ.
use crate::error::{err, Context, Result};
use crate::tasks::Task;
use crate::util;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub path: String,
    pub exists: bool,
    pub saves: Vec<SaveInfo>,
    pub resourcepacks: usize,
    pub shaderpacks: usize,
    pub mods: usize,
    pub has_options: bool,
    pub has_servers: bool,
    /// Остання запущена версія з профілів лаунчера: (версія гри, модлоадер).
    pub last_version: Option<(String, String)>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SaveInfo {
    pub folder: String,
    pub size: u64,
}

pub fn default_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join(".minecraft")
}

fn count(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|r| r.flatten().count()).unwrap_or(0)
}

/// Версія гри й модлоадер з назви профілю: "1.20.1-forge-47.2.0", "Fabric 1.21.1", "OptiFine 1.12.2"…
pub fn parse_version_name(id: &str) -> Option<(String, String)> {
    let lower = id.to_lowercase();
    let loader = if lower.contains("neoforge") {
        "neoforge"
    } else if lower.contains("forge") {
        "forge"
    } else if lower.contains("fabric") {
        "fabric"
    } else if lower.contains("quilt") {
        "quilt"
    } else {
        "vanilla"
    };
    // «neoforge-21.1.77» — це версія NeoForge, а не гри.
    if let Some(v) = lower.strip_prefix("neoforge-") {
        return crate::loaders::neo_to_mc(v).map(|mc| (mc, "neoforge".into()));
    }
    let re = regex_lite::Regex::new(r"\b(1\.\d{1,2}(?:\.\d{1,2})?|2\d\.\d{1,2}(?:\.\d{1,2})?)\b").unwrap();
    let mc = re.captures(id)?.get(1)?.as_str().to_string();
    Some((mc, loader.to_string()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profiles {
    #[serde(default)]
    profiles: std::collections::HashMap<String, Profile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    #[serde(default)]
    last_version_id: Option<String>,
    #[serde(default)]
    last_used: Option<String>,
}

fn last_version(dir: &Path) -> Option<(String, String)> {
    let p: Profiles = util::read_json(&dir.join("launcher_profiles.json")).ok()?;
    let best = p.profiles.values().filter(|x| x.last_version_id.is_some()).max_by(|a, b| a.last_used.cmp(&b.last_used))?;
    let id = best.last_version_id.as_deref()?;
    // "latest-release" і подібне — службові назви офіційного лаунчера.
    if id.starts_with("latest-") {
        return None;
    }
    parse_version_name(id)
}

pub fn scan(path: Option<&str>) -> Scan {
    let dir = path.map(PathBuf::from).unwrap_or_else(default_dir);
    let mut saves: Vec<SaveInfo> = std::fs::read_dir(dir.join("saves"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("level.dat").exists())
        .map(|e| SaveInfo { folder: e.file_name().to_string_lossy().to_string(), size: util::dir_size(&e.path()) })
        .collect();
    saves.sort_by_key(|s| s.folder.to_lowercase());
    Scan {
        exists: dir.join("saves").exists() || dir.join("versions").exists(),
        resourcepacks: count(&dir.join("resourcepacks")),
        shaderpacks: count(&dir.join("shaderpacks")),
        mods: count(&dir.join("mods")),
        has_options: dir.join("options.txt").exists(),
        has_servers: dir.join("servers.dat").exists(),
        last_version: last_version(&dir),
        path: dir.display().to_string(),
        saves,
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct What {
    pub saves: Vec<String>,
    pub resourcepacks: bool,
    pub shaderpacks: bool,
    pub mods: bool,
    pub options: bool,
}

/// Копіює вибране в теку гри збірки (`dest`). Світи — повністю, з усіма файлами.
pub fn copy_into(src: &Path, dest: &Path, what: &What, task: &Task) -> Result<()> {
    if !src.is_dir() {
        return Err(err("invalid", format!("теки {} немає", src.display())));
    }
    let mut jobs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for s in &what.saves {
        let from = util::safe_join(&src.join("saves"), s)?;
        if from.join("level.dat").exists() {
            jobs.push((from, dest.join("saves").join(s)));
        }
    }
    for (on, name) in [(what.resourcepacks, "resourcepacks"), (what.shaderpacks, "shaderpacks"), (what.mods, "mods")] {
        if on && src.join(name).is_dir() {
            jobs.push((src.join(name), dest.join(name)));
        }
    }
    if what.options {
        for f in ["options.txt", "servers.dat", "optionsof.txt", "optionsshaders.txt"] {
            if src.join(f).is_file() {
                jobs.push((src.join(f), dest.join(f)));
            }
        }
    }
    task.stage("migrate", jobs.len() as u64, "items");
    for (from, to) in jobs {
        task.check()?;
        if from.is_dir() {
            util::copy_dir(&from, &to)?;
        } else {
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(&from, &to).ctx(|| to.display().to_string())?;
        }
        task.add(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_version_name as p;

    #[test]
    fn version_names() {
        assert_eq!(p("1.20.1-forge-47.2.0"), Some(("1.20.1".into(), "forge".into())));
        assert_eq!(p("Fabric 1.21.1"), Some(("1.21.1".into(), "fabric".into())));
        assert_eq!(p("fabric-loader-0.16.9-1.21.4"), Some(("1.21.4".into(), "fabric".into())));
        assert_eq!(p("neoforge-21.1.77"), Some(("1.21.1".into(), "neoforge".into())));
        assert_eq!(p("1.12.2-OptiFine_HD_U_G5"), Some(("1.12.2".into(), "vanilla".into())));
        assert_eq!(p("26.3"), Some(("26.3".into(), "vanilla".into())));
        assert_eq!(p("ForgeOptiFine 1.16.5"), Some(("1.16.5".into(), "forge".into())));
    }
}
