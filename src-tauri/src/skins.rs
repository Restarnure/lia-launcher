use crate::error::{err, Result};
use crate::net;
use crate::paths::Paths;
use base64::Engine;
use std::io::Read;

pub async fn data_url(http: &reqwest::Client, url: &str) -> Result<String> {
    // Профіль Mojang віддає текстури за http:// — тягнемо їх тим самим шляхом, але за https.
    let url = url.replacen("http://", "https://", 1);
    let u = url::Url::parse(&url).map_err(|e| err("invalid", e.to_string()))?;
    let mojang = |h: &str| ["minecraft.net", "mojang.com"].iter().any(|d| h == *d || h.strip_suffix(d).is_some_and(|s| s.ends_with('.')));
    if u.scheme() != "https" || !u.host_str().is_some_and(mojang) {
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

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn only_mojang_hosts() {
        // Проксі, що одразу рве з'єднання: дозволена адреса доходить до мережі й падає там, а не на перевірці.
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = proxy.local_addr().unwrap().port();
        std::thread::spawn(move || proxy.incoming().for_each(drop));
        let http = reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).unwrap()).build().unwrap();
        for bad in ["https://evilminecraft.net/x.png", "https://minecraft.net.evil.com/x.png", "https://notmojang.com/x.png", "file:///C:/x.png"] {
            assert_eq!(super::data_url(&http, bad).await.unwrap_err().code(), "invalid", "{bad}");
        }
        for ok in ["http://textures.minecraft.net/texture/ab", "https://minecraft.net/x.png", "https://s.mojang.com/x.png"] {
            assert_ne!(super::data_url(&http, ok).await.unwrap_err().code(), "invalid", "{ok}");
        }
    }
}
