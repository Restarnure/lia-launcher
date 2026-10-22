pub mod install;
pub mod java;
pub mod rules;
pub mod version;

use crate::ctx::Ctx;
use crate::error::Result;
use crate::net;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Manifest {
    pub latest: Latest,
    pub versions: Vec<ManifestVersion>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ManifestVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    pub release_time: String,
}

static CACHE: LazyLock<Mutex<Option<(Instant, Arc<Manifest>)>>> = LazyLock::new(Default::default);

/// Список версій Mojang. У пам'яті 10 хвилин, на диску — для роботи без мережі.
pub async fn manifest(ctx: &Ctx, refresh: bool) -> Result<Arc<Manifest>> {
    if !refresh {
        if let Some((at, m)) = CACHE.lock().unwrap().as_ref() {
            if at.elapsed() < Duration::from_secs(600) {
                return Ok(m.clone());
            }
        }
    }
    let m: Manifest = net::get_json_cached(&ctx.http, MANIFEST_URL, &ctx.paths.cache_file("version_manifest_v2.json")).await?;
    let m = Arc::new(m);
    *CACHE.lock().unwrap() = Some((Instant::now(), m.clone()));
    Ok(m)
}
