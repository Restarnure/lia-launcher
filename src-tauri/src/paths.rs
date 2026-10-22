use std::path::{Path, PathBuf};

/// Розкладка даних повторює `.minecraft` (versions/, libraries/, assets/), тож інсталятори
/// Forge/NeoForge пишуть у неї напряму, а файли спільні для всіх збірок.
#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new(root: PathBuf) -> Self {
        Paths { root }
    }

    /// Де лежать налаштування й акаунти. Тека даних гри може бути деінде (налаштування).
    pub fn config_home() -> PathBuf {
        if let Some(p) = std::env::var_os("LIA_HOME") {
            return PathBuf::from(p);
        }
        dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(".lia")
    }

    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }
    pub fn version_dir(&self, id: &str) -> PathBuf {
        self.versions().join(id)
    }
    pub fn version_json(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.json"))
    }
    pub fn version_jar(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.jar"))
    }
    pub fn libraries(&self) -> PathBuf {
        self.root.join("libraries")
    }
    pub fn library(&self, rel: &str) -> PathBuf {
        self.libraries().join(rel)
    }
    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }
    pub fn asset_index(&self, id: &str) -> PathBuf {
        self.assets().join("indexes").join(format!("{id}.json"))
    }
    pub fn asset_object(&self, hash: &str) -> PathBuf {
        self.assets().join("objects").join(&hash[..2]).join(hash)
    }
    pub fn log_config(&self, id: &str) -> PathBuf {
        self.assets().join("log_configs").join(id)
    }
    pub fn runtimes(&self) -> PathBuf {
        self.root.join("runtimes")
    }
    pub fn instances(&self) -> PathBuf {
        self.root.join("instances")
    }
    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.instances().join(id)
    }
    pub fn game_dir(&self, id: &str) -> PathBuf {
        self.instance_dir(id).join(".minecraft")
    }
    pub fn natives_root(&self) -> PathBuf {
        self.root.join("natives")
    }
    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }
    pub fn cache_file(&self, name: &str) -> PathBuf {
        self.cache().join(name)
    }
    pub fn temp(&self) -> PathBuf {
        self.root.join("temp")
    }

    pub fn contains(&self, p: &Path) -> bool {
        p.starts_with(&self.root)
    }
}
