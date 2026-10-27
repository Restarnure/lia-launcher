use crate::error::Result;
use crate::paths::Paths;
use crate::util;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Client ID застосунку Azure для входу Microsoft, вшитий у збірку (схвалений Mojang).
/// Задається змінною LIA_MS_CLIENT_ID під час збірки або тут, у BUILT_IN.
const BUILT_IN: &str = "d3e68979-4edc-432d-b092-70ab0060e19e";
pub const DEFAULT_MS_CLIENT_ID: &str = match option_env!("LIA_MS_CLIENT_ID") {
    Some(id) => id,
    None => BUILT_IN,
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
    /// Масштаб інтерфейсу лаунчера (1.0 = 100%).
    pub ui_scale: f64,
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
            ms_client_id: String::new(),
            selected_account: None,
            last_instance: None,
            show_featured: true,
            show_snapshots: false,
            ui_scale: 1.0,
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
        self.ui_scale = if self.ui_scale.is_finite() { self.ui_scale.clamp(0.75, 1.5) } else { 1.0 };
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

    /// Client ID для входу: свій із налаштувань (для розробки) або вшитий у збірку.
    pub fn client_id(&self) -> String {
        if self.ms_client_id.is_empty() {
            DEFAULT_MS_CLIENT_ID.to_string()
        } else {
            self.ms_client_id.clone()
        }
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
