use crate::ctx::Ctx;
use crate::error::{err, Context, Result};
use crate::net::{self, Dl};
use crate::tasks::Task;
use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RUNTIMES_URL: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86") => "windows-x86",
        ("windows", "aarch64") => "windows-arm64",
        ("windows", _) => "windows-x64",
        ("macos", "aarch64") => "mac-os-arm64",
        ("macos", _) => "mac-os",
        ("linux", "x86") => "linux-i386",
        _ => "linux",
    }
}

#[derive(Deserialize)]
struct RuntimeEntry {
    manifest: Ref,
    version: RuntimeVersion,
}

#[derive(Deserialize)]
struct Ref {
    sha1: String,
    url: String,
}

#[derive(Deserialize)]
struct RuntimeVersion {
    name: String,
}

#[derive(Deserialize)]
struct RuntimeManifest {
    files: BTreeMap<String, RuntimeFile>,
}

#[derive(Deserialize)]
struct RuntimeFile {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    executable: bool,
    #[serde(default)]
    downloads: Option<RuntimeDownloads>,
}

#[derive(Deserialize)]
struct RuntimeDownloads {
    raw: Option<RawFile>,
}

#[derive(Deserialize)]
struct RawFile {
    sha1: String,
    size: u64,
    url: String,
}

#[derive(Serialize, Deserialize)]
struct Marker {
    manifest_sha1: String,
    version: String,
}

/// Саме java.exe, а не javaw.exe: javaw при помилці старту JVM показує модальне вікно
/// й висить. java.exe з CREATE_NO_WINDOW теж без консолі, але помилки йдуть у лог.
pub fn java_exe(home: &Path) -> PathBuf {
    if cfg!(windows) {
        home.join("bin").join("java.exe")
    } else if cfg!(target_os = "macos") {
        home.join("jre.bundle").join("Contents").join("Home").join("bin").join("java")
    } else {
        home.join("bin").join("java")
    }
}

/// Офіційна Java від Mojang для компонента з version JSON (`java-runtime-delta` = 21 тощо).
pub async fn ensure_runtime(ctx: &Ctx, component: &str, task: &Arc<Task>, verify: bool) -> Result<PathBuf> {
    super::install::check_id(component)?;
    let home = ctx.paths.runtimes().join(component);
    let marker_path = home.join(".lia-runtime.json");
    let exe = java_exe(&home);
    if !verify && exe.exists() && marker_path.exists() {
        return Ok(exe);
    }
    task.stage("java", 0, "");
    let index: HashMap<String, HashMap<String, Vec<RuntimeEntry>>> =
        net::get_json_cached(&ctx.http, RUNTIMES_URL, &ctx.paths.cache_file("java-runtimes.json")).await?;
    let entry = index
        .get(platform())
        .and_then(|p| p.get(component))
        .and_then(|v| v.first())
        .ok_or_else(|| err("java_unavailable", format!("Mojang не постачає Java «{component}» для {}", platform())))?;
    let manifest: RuntimeManifest = net::get_json(&ctx.http, &entry.manifest.url).await?;

    let mut dls = Vec::new();
    let mut executables = Vec::new();
    for (rel, f) in &manifest.files {
        let path = util::safe_join(&home, rel)?;
        match f.kind.as_str() {
            "directory" => std::fs::create_dir_all(&path).ctx(|| path.display().to_string())?,
            "file" => {
                if let Some(raw) = f.downloads.as_ref().and_then(|d| d.raw.as_ref()) {
                    dls.push(Dl::new(&raw.url, path.clone(), Some(raw.sha1.clone()), Some(raw.size)));
                    if f.executable {
                        executables.push(path);
                    }
                }
            }
            _ => {} // посилання є лише в збірках для macOS/Linux; java працює й без них
        }
    }
    net::download_all(&ctx.http, dls, task, "java", ctx.concurrency, verify).await?;

    #[cfg(unix)]
    for p in &executables {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = executables;

    if !exe.exists() {
        return Err(err("java_broken", format!("після встановлення немає {}", exe.display())));
    }
    util::write_json(&marker_path, &Marker { manifest_sha1: entry.manifest.sha1.clone(), version: entry.version.name.clone() })?;
    Ok(exe)
}

/// Для шляху, який вказав користувач: javaw.exe → java.exe поруч (якщо є).
pub fn console_variant(java: &Path) -> PathBuf {
    let is_javaw = java.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("javaw.exe"));
    let cli = java.with_file_name("java.exe");
    if is_javaw && cli.exists() {
        cli
    } else {
        java.to_path_buf()
    }
}

/// Версія довільної Java (для поля «свій шлях до Java» у налаштуваннях).
pub async fn probe(java: &Path) -> Result<String> {
    // javaw нічого не друкує в консоль — для перевірки беремо java поруч.
    let cli = java.with_file_name(if cfg!(windows) { "java.exe" } else { "java" });
    let bin = if cli.exists() { cli } else { java.to_path_buf() };
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("-version");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = cmd.output().await.ctx(|| bin.display().to_string())?;
    let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or("").trim().to_string();
    if first.is_empty() {
        return Err(err("java_broken", format!("{} не відповідає на -version", bin.display())));
    }
    Ok(first)
}

/// Мажорна версія з рядка `java version "1.8.0_51"` / `openjdk version "21.0.7"`.
pub fn parse_major(line: &str) -> Option<u32> {
    let v = line.split('"').nth(1)?;
    let mut parts = v.split(['.', '_', '-', '+']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn majors() {
        assert_eq!(super::parse_major(r#"java version "1.8.0_51""#), Some(8));
        assert_eq!(super::parse_major(r#"openjdk version "21.0.7" 2025-04-15"#), Some(21));
        assert_eq!(super::parse_major(r#"java version "25.0.4" 2026-07-21 LTS"#), Some(25));
        assert_eq!(super::parse_major("garbage"), None);
    }
}
