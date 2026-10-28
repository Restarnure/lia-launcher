use crate::error::{err, Context, Result};
use crate::paths::Paths;
use crate::util;
use serde::{Deserialize, Serialize};

pub const LOADERS: &[&str] = &["vanilla", "fabric", "quilt", "forge", "neoforge"];

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModpackRef {
    pub project_id: String,
    pub version_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version_name: String,
    #[serde(default)]
    pub icon_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Instance {
    /// Назва теки в `instances/` (не зберігається у файлі).
    #[serde(skip_deserializing)]
    pub id: String,
    pub name: String,
    /// "block:grass" тощо, або "custom" (icon.png у теці збірки).
    pub icon: String,
    pub mc_version: String,
    pub loader: String,
    pub loader_version: Option<String>,
    /// Профіль запуску в versions/ після встановлення модлоадера.
    pub version_id: Option<String>,
    pub memory_mb: Option<u32>,
    pub java_path: Option<String>,
    pub jvm_args: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fullscreen: Option<bool>,
    pub created: i64,
    pub last_played: Option<i64>,
    /// Секунди в грі.
    pub play_time: u64,
    pub featured: Option<String>,
    pub modpack: Option<ModpackRef>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NewInstance {
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

fn file(paths: &Paths, id: &str) -> std::path::PathBuf {
    paths.instance_dir(id).join("instance.json")
}

fn check(id: &str) -> Result<()> {
    // Windows відкидає крапки й пробіли в кінці: «...» — це сама тека instances.
    if id.is_empty() || id.contains(['/', '\\', ':']) || id.ends_with(['.', ' ']) {
        return Err(err("invalid", format!("неприпустимий ідентифікатор збірки: {id}")));
    }
    Ok(())
}

pub fn list(paths: &Paths) -> Vec<Instance> {
    let mut out: Vec<Instance> = std::fs::read_dir(paths.instances())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            let mut i: Instance = util::read_json(&e.path().join("instance.json")).ok()?;
            i.id = id;
            Some(i)
        })
        .collect();
    out.sort_by(|a, b| b.last_played.unwrap_or(b.created).cmp(&a.last_played.unwrap_or(a.created)));
    out
}

pub fn get(paths: &Paths, id: &str) -> Result<Instance> {
    check(id)?;
    let mut i: Instance = util::read_json(&file(paths, id)).map_err(|_| err("instance_not_found", format!("збірку {id} не знайдено")))?;
    i.id = id.to_string();
    Ok(i)
}

pub fn save(paths: &Paths, inst: &Instance) -> Result<()> {
    check(&inst.id)?;
    util::write_json(&file(paths, &inst.id), inst)
}

pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 48 {
        return Err(err("invalid_name", "назва має містити від 1 до 48 символів"));
    }
    Ok(name.to_string())
}

/// Вільна назва теки — тека одразу й створюється: дві збірки з однаковою назвою, що створюються
/// одночасно (імпорт модпаків), не потраплять в одну теку.
fn free_id(paths: &Paths, name: &str) -> Result<String> {
    std::fs::create_dir_all(paths.instances()).ctx(|| paths.instances().display().to_string())?;
    let base = util::slugify(name);
    let mut id = base.clone();
    let mut n = 2;
    loop {
        match std::fs::create_dir(paths.instance_dir(&id)) {
            Ok(()) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).ctx(|| paths.instance_dir(&id).display().to_string()),
        }
        id = format!("{base}-{n}");
        n += 1;
    }
}

pub fn create(paths: &Paths, spec: NewInstance) -> Result<Instance> {
    let name = validate_name(&spec.name)?;
    if !LOADERS.contains(&spec.loader.as_str()) {
        return Err(err("invalid", format!("невідомий модлоадер {}", spec.loader)));
    }
    crate::mojang::install::check_id(&spec.mc_version)?;
    let id = free_id(paths, &name)?;
    std::fs::create_dir_all(paths.game_dir(&id)).ctx(|| paths.game_dir(&id).display().to_string())?;
    let inst = Instance {
        id,
        name,
        icon: spec.icon.unwrap_or_else(|| "block:grass".into()),
        mc_version: spec.mc_version.clone(),
        loader: spec.loader.clone(),
        loader_version: spec.loader_version.filter(|_| spec.loader != "vanilla"),
        version_id: (spec.loader == "vanilla").then_some(spec.mc_version),
        created: util::now_ms(),
        ..Default::default()
    };
    save(paths, &inst)?;
    Ok(inst)
}

/// Видалення у кошик: світи гравця не зникають безповоротно через випадковий клік.
pub fn delete(paths: &Paths, id: &str) -> Result<()> {
    check(id)?;
    let dir = paths.instance_dir(id);
    if !dir.exists() {
        return Ok(());
    }
    if trash::delete(&dir).is_err() {
        std::fs::remove_dir_all(&dir).ctx(|| dir.display().to_string())?;
    }
    Ok(())
}

pub fn duplicate(paths: &Paths, id: &str, copy_suffix: &str) -> Result<Instance> {
    let src = get(paths, id)?;
    let mut name = format!("{} {copy_suffix}", src.name);
    if name.chars().count() > 48 {
        name = name.chars().take(48).collect();
    }
    let new_id = free_id(paths, &name)?;
    if let Err(e) = util::copy_dir(&paths.instance_dir(id), &paths.instance_dir(&new_id)) {
        let _ = std::fs::remove_dir_all(paths.instance_dir(&new_id));
        return Err(e);
    }
    // Копія — звичайна збірка: кнопка закріпленого сервера й далі веде в оригінал.
    let inst = Instance { id: new_id, name, created: util::now_ms(), last_played: None, play_time: 0, featured: None, ..src };
    save(paths, &inst)?;
    Ok(inst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_duplicates() {
        for bad in ["", ".", "..", "...", "a.", "a ", "a/b", "a\\b", "c:x"] {
            assert!(check(bad).is_err(), "{bad}");
        }
        assert!(check("fabric-1-21").is_ok());

        let root = std::env::temp_dir().join(format!("lia-inst-{}", std::process::id()));
        let paths = Paths::new(root.clone());
        // Однакова назва — різні теки, навіть якщо instance.json ще не записано.
        assert_eq!(free_id(&paths, "Shotam").unwrap(), "shotam");
        assert_eq!(free_id(&paths, "Shotam").unwrap(), "shotam-2");

        let mut src = create(&paths, NewInstance { name: "Shotam".into(), mc_version: "1.21.11".into(), loader: "fabric".into(), loader_version: None, icon: None }).unwrap();
        assert_eq!(src.id, "shotam-3");
        src.featured = Some("shotam".into());
        save(&paths, &src).unwrap();
        let copy = duplicate(&paths, &src.id, "(копія)").unwrap();
        assert_eq!(copy.featured, None);
        assert_eq!(get(&paths, &copy.id).unwrap().featured, None);
        assert_eq!(get(&paths, &src.id).unwrap().featured.as_deref(), Some("shotam"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
