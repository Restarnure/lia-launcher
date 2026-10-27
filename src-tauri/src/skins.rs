use crate::error::{err, Result};
use crate::net;
use crate::paths::Paths;
use base64::Engine;
use std::io::Read;

pub async fn data_url(http: &reqwest::Client, url: &str) -> Result<String> {
    // Профіль Mojang віддає текстури за http:// — тягнемо їх тим самим шляхом, але за https.
    let url = url.replacen("http://", "https://", 1);
    let u = url::Url::parse(&url).map_err(|e| err("invalid", e.to_string()))?;
    if u.scheme() != "https" || !u.host_str().is_some_and(|h| h.ends_with("minecraft.net") || h.ends_with("mojang.com")) {
        return Err(err("invalid", "скін не з серверів Mojang"));
    }
    let bytes = net::get_bytes(http, &url).await?;
    Ok(png_url(&bytes))
}

pub fn png_url(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Скін Стіва з уже встановленої гри — не тягнемо чужі текстури в інсталятор.
pub fn default_skin(paths: &Paths) -> Option<Vec<u8>> {
    let mut jars: Vec<_> = std::fs::read_dir(paths.versions())
        .ok()?
        .flatten()
        .map(|e| e.path().join(format!("{}.jar", e.file_name().to_string_lossy())))
        .filter(|p| p.exists())
        .collect();
    jars.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    for jar in jars {
        let Ok(f) = std::fs::File::open(&jar) else { continue };
        let Ok(mut zip) = zip::ZipArchive::new(f) else { continue };
        for name in ["assets/minecraft/textures/entity/player/wide/steve.png", "assets/minecraft/textures/entity/steve.png"] {
            if let Ok(mut e) = zip.by_name(name) {
                let mut buf = Vec::new();
                if e.read_to_end(&mut buf).is_ok() {
                    return Some(buf);
                }
            }
        }
    }
    None
}
