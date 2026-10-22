use crate::error::Result;
use crate::paths::Paths;
use crate::util;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Client ID застосунку Azure для входу Microsoft. Порожній = вхід Microsoft вимкнено,
/// доки не вказано свій ID у налаштуваннях (ID має бути схвалений Mojang).
pub const DEFAULT_MS_CLIENT_ID: &str = match option_env!("LIA_MS_CLIENT_ID") {
    Some(id) => id,
    None => "",
};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub language: String,
    /// Тека для версій, бібліотек і збірок. `None` = поруч із налаштуваннями.
    pub data_dir: Option<String>,
    pub memory_mb: u32,
    pub java_path: Option<String>,
    pub jvm_args: String,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    /// "keep" | "minimize" | "close"
    pub on_launch: String,
    pub concurrency: u32,
    pub ms_client_id: String,
    pub selected_account: Option<String>,
    pub last_instance: Option<String>,
    pub show_featured: bool,
    pub show_snapshots: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: "uk".into(),
            data_dir: None,
            memory_mb: default_memory_mb(),
            java_path: None,
            jvm_args: String::new(),
            width: 1280,
            height: 720,
            fullscreen: false,
            on_launch: "minimize".into(),
            concurrency: 16,
            ms_client_id: DEFAULT_MS_CLIENT_ID.into(),
            selected_account: None,
            last_instance: None,
            show_featured: true,
            show_snapshots: false,
        }
    }
}

pub fn total_memory_mb() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.total_memory() / 1024 / 1024
}

fn default_memory_mb() -> u32 {
    match total_memory_mb() {
        t if t >= 15_500 => 4096,
        t if t >= 7_500 => 3072,
        _ => 2048,
    }
}

impl Settings {
    fn file() -> PathBuf {
        Paths::config_home().join("launcher.json")
    }

    pub fn load() -> Settings {
        let mut s: Settings = util::read_json(&Self::file()).unwrap_or_default();
        s.sanitize();
        s
    }

    pub fn save(&self) -> Result<()> {
        util::write_json(&Self::file(), self)
    }

    pub fn sanitize(&mut self) {
        let total = total_memory_mb().max(2048) as u32;
        self.memory_mb = self.memory_mb.clamp(512, total);
        self.concurrency = self.concurrency.clamp(1, 64);
        self.width = self.width.clamp(320, 7680);
        self.height = self.height.clamp(240, 4320);
        if !["keep", "minimize", "close"].contains(&self.on_launch.as_str()) {
            self.on_launch = "minimize".into();
        }
        if !["uk", "en"].contains(&self.language.as_str()) {
            self.language = "uk".into();
        }
        self.data_dir = self.data_dir.take().filter(|d| !d.trim().is_empty());
        self.java_path = self.java_path.take().filter(|d| !d.trim().is_empty());
        self.ms_client_id = self.ms_client_id.trim().to_string();
    }

    pub fn data_root(&self) -> PathBuf {
        if let Some(env) = std::env::var_os("LIA_DATA") {
            return PathBuf::from(env);
        }
        match &self.data_dir {
            Some(d) => PathBuf::from(d),
            None => Paths::config_home(),
        }
    }
}
