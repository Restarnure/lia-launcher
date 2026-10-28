//! Модлоадери: Fabric і Quilt — готовий профіль з їхнього meta API;
//! Forge і NeoForge — офіційний інсталятор у режимі `--installClient` (старий Forge — вручну).
use crate::ctx::Ctx;
use crate::error::{err, Context, Error, Result};
use crate::mojang::install::{self, check_id};
use crate::mojang::version::VersionJson;
use crate::mojang::{self, java};
use crate::net::{self, Dl};
use crate::tasks::Task;
use crate::util;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const QUILT_META: &str = "https://meta.quiltmc.org/v3";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const FORGE_PROMOS: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const NEO_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";
const NEO_VERSIONS: &str = "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";

#[derive(Serialize, Clone, Debug)]
pub struct LoaderVersion {
    /// Те, що зберігається в збірці й передається в `install`.
    pub id: String,
    /// Для показу (без префікса версії гри).
    pub name: String,
    pub stable: bool,
}

fn meta_base(loader: &str) -> &'static str {
    if loader == "quilt" {
        QUILT_META
    } else {
        FABRIC_META
    }
}

fn unstable_name(v: &str) -> bool {
    let v = v.to_lowercase();
    ["beta", "alpha", "pre", "rc", "snapshot"].iter().any(|t| v.contains(t))
}

/// Числове порівняння версій на кшталт "21.1.77" > "21.1.9".
fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap_or(0)).collect()
}

/// Версія NeoForge → версія гри: 21.1.x → 1.21.1, 21.0.x → 1.21, 26.3.0.x → 26.3, 26.1.2.x → 26.1.2.
pub fn neo_to_mc(v: &str) -> Option<String> {
    if v.contains('+') || v.starts_with("0.") {
        return None;
    }
    let p: Vec<&str> = v.split(['.', '-']).collect();
    let major: u32 = p.first()?.parse().ok()?;
    if major < 26 {
        let minor: u32 = p.get(1)?.parse().ok()?;
        Some(if minor == 0 { format!("1.{major}") } else { format!("1.{major}.{minor}") })
    } else {
        let minor: u32 = p.get(1)?.parse().ok()?;
        let patch: u32 = p.get(2)?.parse().ok()?;
        Some(if patch == 0 { format!("{major}.{minor}") } else { format!("{major}.{minor}.{patch}") })
    }
}

async fn forge_all(ctx: &Ctx) -> Result<Vec<String>> {
    let xml = String::from_utf8_lossy(
        &net::get_bytes(&ctx.http, &format!("{FORGE_MAVEN}/maven-metadata.xml")).await.or_else(|e| {
            std::fs::read(ctx.paths.cache_file("forge-metadata.xml")).map(Into::into).map_err(|_| e)
        })?,
    )
    .to_string();
    let _ = util::write_atomic(&ctx.paths.cache_file("forge-metadata.xml"), xml.as_bytes());
    Ok(xml.split("<version>").skip(1).filter_map(|s| s.split("</version>").next()).map(str::to_string).collect())
}

async fn neo_all(ctx: &Ctx) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct R {
        versions: Vec<String>,
    }
    let r: R = net::get_json_cached(&ctx.http, NEO_VERSIONS, &ctx.paths.cache_file("neoforge-versions.json")).await?;
    Ok(r.versions)
}

/// Версії гри, які підтримує модлоадер (для фільтра у вікні створення збірки).
pub async fn supported_games(ctx: &Ctx, loader: &str) -> Result<Vec<String>> {
    Ok(match loader {
        "fabric" | "quilt" => {
            #[derive(Deserialize)]
            struct G {
                version: String,
            }
            let url = format!("{}/versions/game", meta_base(loader));
            let g: Vec<G> = net::get_json_cached(&ctx.http, &url, &ctx.paths.cache_file(&format!("{loader}-games.json"))).await?;
            g.into_iter().map(|g| g.version).collect()
        }
        "forge" => {
            let set: BTreeSet<String> = forge_all(ctx).await?.iter().filter_map(|v| v.split('-').next().map(str::to_string)).collect();
            set.into_iter().collect()
        }
        "neoforge" => {
            let set: BTreeSet<String> = neo_all(ctx).await?.iter().filter_map(|v| neo_to_mc(v)).collect();
            set.into_iter().collect()
        }
        _ => vec![],
    })
}

pub async fn versions(ctx: &Ctx, loader: &str, mc: &str) -> Result<Vec<LoaderVersion>> {
    check_id(mc)?;
    let mut out = match loader {
        "fabric" | "quilt" => {
            #[derive(Deserialize)]
            struct E {
                loader: L,
            }
            #[derive(Deserialize)]
            struct L {
                version: String,
                #[serde(default)]
                stable: Option<bool>,
            }
            let url = format!("{}/versions/loader/{mc}", meta_base(loader));
            let list: Vec<E> = net::get_json_cached(&ctx.http, &url, &ctx.paths.cache_file(&format!("{loader}-{mc}.json"))).await?;
            list.into_iter()
                .map(|e| LoaderVersion {
                    stable: e.loader.stable.unwrap_or_else(|| !unstable_name(&e.loader.version)),
                    name: e.loader.version.clone(),
                    id: e.loader.version,
                })
                .collect::<Vec<_>>()
        }
        "forge" => {
            let promos: serde_json::Value =
                net::get_json_cached(&ctx.http, FORGE_PROMOS, &ctx.paths.cache_file("forge-promos.json")).await.unwrap_or_default();
            let rec = promos["promos"][format!("{mc}-recommended")].as_str().unwrap_or("").to_string();
            let prefix = format!("{mc}-");
            let mut list: Vec<LoaderVersion> = forge_all(ctx)
                .await?
                .into_iter()
                .filter(|v| v.starts_with(&prefix))
                .map(|v| {
                    let name = v[prefix.len()..].to_string();
                    LoaderVersion { stable: !rec.is_empty() && name == rec, name, id: v }
                })
                .collect();
            list.sort_by_key(|v| std::cmp::Reverse(version_key(&v.name)));
            if rec.is_empty() {
                if let Some(first) = list.first_mut() {
                    first.stable = true;
                }
            }
            list
        }
        "neoforge" => {
            let mut list: Vec<LoaderVersion> = neo_all(ctx)
                .await?
                .into_iter()
                .filter(|v| neo_to_mc(v).as_deref() == Some(mc))
                .map(|v| LoaderVersion { stable: !unstable_name(&v), name: v.clone(), id: v })
                .collect();
            list.sort_by_key(|v| std::cmp::Reverse(version_key(&v.name)));
            list
        }
        _ => return Err(err("invalid", format!("невідомий модлоадер {loader}"))),
    };
    if loader == "fabric" || loader == "quilt" {
        out.dedup_by(|a, b| a.id == b.id);
    }
    Ok(out)
}

/// Найкраща версія: перша стабільна, інакше найновіша.
pub async fn pick(ctx: &Ctx, loader: &str, mc: &str) -> Result<String> {
    let list = versions(ctx, loader, mc).await?;
    list.iter()
        .find(|v| v.stable)
        .or(list.first())
        .map(|v| v.id.clone())
        .ok_or_else(|| err("loader_unsupported", format!("{loader} не підтримує Minecraft {mc}")))
}

/// Встановлює модлоадер і повертає id профілю запуску в versions/.
pub async fn install(ctx: &Ctx, loader: &str, mc: &str, version: Option<&str>, task: &Arc<Task>) -> Result<String> {
    check_id(mc)?;
    let version = match version {
        Some(v) => v.to_string(),
        None => pick(ctx, loader, mc).await?,
    };
    check_id(&version)?;
    task.stage("loader", 0, "");
    match loader {
        "fabric" | "quilt" => {
            let url = format!("{}/versions/loader/{mc}/{version}/profile/json", meta_base(loader));
            let bytes = net::get_bytes(&ctx.http, &url).await?;
            let profile: VersionJson = serde_json::from_slice(&bytes).ctx(|| url.clone())?;
            check_id(&profile.id)?;
            util::write_atomic(&ctx.paths.version_json(&profile.id), &bytes)?;
            Ok(profile.id)
        }
        "forge" => run_installer(ctx, mc, &format!("{FORGE_MAVEN}/{version}/forge-{version}-installer.jar"), task).await,
        "neoforge" => run_installer(ctx, mc, &format!("{NEO_MAVEN}/{version}/neoforge-{version}-installer.jar"), task).await,
        _ => Err(err("invalid", format!("невідомий модлоадер {loader}"))),
    }
}

fn read_zip_entry(jar: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar)?)?;
    let result = match zip.by_name(name) {
        Ok(mut e) => {
            let mut buf = Vec::new();
            e.read_to_end(&mut buf)?;
            Ok(Some(buf))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    };
    result
}

async fn run_installer(ctx: &Ctx, mc: &str, url: &str, task: &Arc<Task>) -> Result<String> {
    // Ванільні JSON і client.jar — наперед, перевірені за SHA1: інсталятор їх тоді не качає.
    let vanilla = install::load_json(ctx, mc).await?;
    if let Some(c) = vanilla.downloads.as_ref().and_then(|d| d.get("client")) {
        net::download(&ctx.http, &Dl::new(&c.url, ctx.paths.version_jar(mc), c.sha1.clone(), c.size), None).await.ok();
    }
    let java = java::ensure_runtime(ctx, &vanilla.java_component(), task, false).await?;
    task.stage("loader", 0, "");

    let tmp = ctx.paths.temp().join(format!("installer-{}", util::now_ms()));
    std::fs::create_dir_all(&tmp)?;
    let jar = tmp.join("installer.jar");
    net::download(&ctx.http, &Dl::new(url, jar.clone(), None, None), None).await?;
    let result = install_from_jar(ctx, &java, &jar, &tmp, task).await;
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

async fn install_from_jar(ctx: &Ctx, java: &Path, jar: &Path, tmp: &Path, task: &Arc<Task>) -> Result<String> {
    let profile: serde_json::Value = match read_zip_entry(jar, "install_profile.json")? {
        Some(b) => serde_json::from_slice(&b)?,
        None => return Err(err("loader_broken", "в інсталяторі немає install_profile.json")),
    };

    // Старий Forge (1.5–1.12.1): профіль версії й universal.jar лежать прямо в інсталяторі.
    if let Some(info) = profile.get("versionInfo") {
        let id = profile["install"]["target"].as_str().unwrap_or_default().to_string();
        check_id(&id)?;
        let path = profile["install"]["path"].as_str().and_then(util::maven_path).ok_or_else(|| err("loader_broken", "install.path"))?;
        let file = profile["install"]["filePath"].as_str().unwrap_or_default();
        let universal = read_zip_entry(jar, file)?.ok_or_else(|| err("loader_broken", format!("немає {file}")))?;
        util::write_atomic(&ctx.paths.library(&path), &universal)?;
        util::write_json(&ctx.paths.version_json(&id), info)?;
        return Ok(id);
    }

    let id = profile["version"].as_str().unwrap_or_default().to_string();
    check_id(&id)?;
    let total = profile["libraries"].as_array().map_or(0, |a| a.len())
        + profile["processors"].as_array().map_or(0, |a| a.iter().filter(|p| p["sides"].as_array().is_none_or(|s| s.iter().any(|x| x == "client"))).count());
    task.stage("loader", total as u64, "items");

    let profiles = ctx.paths.root.join("launcher_profiles.json");
    if !profiles.exists() {
        util::write_atomic(&profiles, br#"{"profiles":{}}"#)?;
    }
    let mut cmd = tokio::process::Command::new(java);
    cmd.arg("-jar").arg(jar).arg("--installClient").arg(&ctx.paths.root).current_dir(tmp);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut child = cmd.spawn().ctx(|| java.display().to_string())?;
    // Байтами, не lines(): Java пише шляхи в cp1251 (кирилиця в імені користувача), lines() на цьому
    // повертає помилку, читання stdout зупиняється, і інсталятор назавжди блокується на повному каналі.
    let mut out = BufReader::new(child.stdout.take().unwrap()).split(b'\n');
    let stderr = child.stderr.take().unwrap();
    let err_task = tokio::spawn(async move {
        let mut s = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut BufReader::new(stderr), &mut s).await;
        crate::launch::decode(&s)
    });
    let mut tail: Vec<String> = Vec::new();
    let mut done = 0u64;
    loop {
        // Не лише в паузах виводу: під час завантаження бібліотек рядки йдуть безперервно.
        if task.cancelled() {
            let _ = child.kill().await;
            return Err(Error::Cancelled);
        }
        tokio::select! {
            line = out.next_segment() => match line {
                Ok(Some(l)) => {
                    let l = crate::launch::decode(&l).trim_end_matches('\r').to_string();
                    if l.contains("Downloading library") || l.contains("Considering library") || l.contains("MainClass:") {
                        done = (done + 1).min(total as u64);
                        task.set(done);
                    }
                    tail.push(l);
                    if tail.len() > 60 { tail.remove(0); }
                }
                _ => break,
            },
            _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
        }
    }
    let status = child.wait().await?;
    let stderr = err_task.await.unwrap_or_default();
    if !status.success() || !ctx.paths.version_json(&id).exists() {
        let detail = format!("{}\n{}", tail.join("\n"), stderr.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"));
        return Err(err("loader_install", format!("інсталятор завершився з кодом {:?}\n{}", status.code(), detail.trim())));
    }
    Ok(id)
}

/// Перевірка, що профіль модлоадера на місці (після перенесення теки даних тощо).
pub async fn ensure(ctx: &Ctx, loader: &str, mc: &str, version: Option<&str>, version_id: Option<&str>, task: &Arc<Task>) -> Result<String> {
    if let Some(id) = version_id {
        if check_id(id).is_ok() && ctx.paths.version_json(id).exists() {
            return Ok(id.to_string());
        }
    }
    if loader == "vanilla" {
        mojang::manifest(ctx, false).await?;
        return Ok(mc.to_string());
    }
    install(ctx, loader, mc, version, task).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neo_mapping() {
        assert_eq!(neo_to_mc("21.1.77").as_deref(), Some("1.21.1"));
        assert_eq!(neo_to_mc("21.0.167").as_deref(), Some("1.21"));
        assert_eq!(neo_to_mc("20.4.0-beta").as_deref(), Some("1.20.4"));
        assert_eq!(neo_to_mc("26.3.0.95").as_deref(), Some("26.3"));
        assert_eq!(neo_to_mc("26.1.2.122").as_deref(), Some("26.1.2"));
        assert_eq!(neo_to_mc("26.1.0.0-alpha.1+snapshot-1"), None);
        assert_eq!(neo_to_mc("0.25w14craftmine.3-beta"), None);
    }

    #[test]
    fn version_order() {
        let mut v = vec!["21.1.9", "21.1.77", "21.1.100-beta"];
        v.sort_by_key(|s| std::cmp::Reverse(version_key(s)));
        assert_eq!(v, vec!["21.1.100-beta", "21.1.77", "21.1.9"]);
    }
}
