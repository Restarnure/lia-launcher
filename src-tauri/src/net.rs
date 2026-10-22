use crate::error::{err, Context, Error, Result};
use crate::tasks::Task;
use crate::util;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub const USER_AGENT: &str = concat!("LIA-Launcher/", env!("CARGO_PKG_VERSION"));

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(45))
        .pool_max_idle_per_host(32)
        .build()
        .expect("http client")
}

async fn checked(resp: reqwest::Response, url: &str) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    let body: String = body.chars().take(300).collect();
    Err(err("http", format!("{url} → HTTP {}: {body}", status.as_u16())))
}

pub async fn get_bytes(http: &reqwest::Client, url: &str) -> Result<bytes::Bytes> {
    let resp = http.get(url).send().await.ctx(|| url.to_string())?;
    Ok(checked(resp, url).await?.bytes().await.ctx(|| url.to_string())?)
}

pub async fn get_json<T: DeserializeOwned>(http: &reqwest::Client, url: &str) -> Result<T> {
    let bytes = get_bytes(http, url).await?;
    serde_json::from_slice(&bytes).ctx(|| url.to_string())
}

/// Спершу мережа, при невдачі — збережена копія. Так лаунчер працює без інтернету.
pub async fn get_json_cached<T: DeserializeOwned>(http: &reqwest::Client, url: &str, cache: &Path) -> Result<T> {
    let net = async {
        let bytes = get_bytes(http, url).await?;
        let v: T = serde_json::from_slice(&bytes).ctx(|| url.to_string())?;
        let _ = util::write_atomic(cache, &bytes);
        Ok::<T, Error>(v)
    }
    .await;
    match net {
        Ok(v) => Ok(v),
        Err(e) if cache.exists() => util::read_json(cache).map_err(|_| e),
        Err(e) => Err(e),
    }
}

#[derive(Clone, Debug)]
pub struct Dl {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

impl Dl {
    pub fn new(url: impl Into<String>, path: PathBuf, sha1: Option<String>, size: Option<u64>) -> Dl {
        Dl { url: url.into(), path, sha1, size }
    }

    fn is_valid(&self, verify_hash: bool) -> bool {
        let Ok(meta) = std::fs::metadata(&self.path) else { return false };
        if let Some(size) = self.size {
            if meta.len() != size {
                return false;
            }
        }
        if verify_hash {
            if let Some(sha) = &self.sha1 {
                return util::sha1_file(&self.path).map(|h| h.eq_ignore_ascii_case(sha)).unwrap_or(false);
            }
        }
        true
    }
}

/// Один файл: потоково, з SHA1 на льоту, через `.part` і з повторами.
pub async fn download(http: &reqwest::Client, dl: &Dl, task: Option<&Task>) -> Result<()> {
    let mut last = None;
    for attempt in 0..4u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400 * 2u64.pow(attempt))).await;
        }
        match try_download(http, dl, task).await {
            Ok(()) => return Ok(()),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e @ Error::Coded { code: "http", .. }) if e.to_string().contains("HTTP 404") => {
                return Err(e);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap())
}

async fn try_download(http: &reqwest::Client, dl: &Dl, task: Option<&Task>) -> Result<()> {
    let resp = http.get(&dl.url).send().await.ctx(|| dl.url.clone())?;
    let resp = checked(resp, &dl.url).await?;
    if let Some(dir) = dl.path.parent() {
        tokio::fs::create_dir_all(dir).await.ctx(|| dir.display().to_string())?;
    }
    let name = dl.path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dl.path.with_file_name(format!("{name}.part"));
    let mut file = tokio::fs::File::create(&tmp).await.ctx(|| tmp.display().to_string())?;
    let mut hasher = Sha1::new();
    let mut written = 0u64;
    let mut stream = resp.bytes_stream();
    let result: Result<()> = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.ctx(|| dl.url.clone())?;
            if let Some(t) = task {
                t.check()?;
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            if let Some(t) = task {
                t.add(chunk.len() as u64);
            }
        }
        file.flush().await?;
        Ok(())
    }
    .await;
    drop(file);
    let verify = || -> Result<()> {
        result?;
        if let Some(size) = dl.size {
            if size != written {
                return Err(err("integrity", format!("{}: розмір {written} замість {size}", dl.url)));
            }
        }
        if let Some(sha) = &dl.sha1 {
            let got = hex::encode(hasher.clone().finalize());
            if !got.eq_ignore_ascii_case(sha) {
                return Err(err("integrity", format!("{}: SHA1 {got} замість {sha}", dl.url)));
            }
        }
        Ok(())
    };
    if let Err(e) = verify() {
        let _ = tokio::fs::remove_file(&tmp).await;
        if let Some(t) = task {
            t.sub(written);
        }
        return Err(e);
    }
    tokio::fs::rename(&tmp, &dl.path).await.ctx(|| dl.path.display().to_string())?;
    Ok(())
}

/// Які з файлів треба качати. Перевірка хешу (режим «полагодити») — паралельно на всіх ядрах.
async fn missing(items: Vec<Dl>, verify_hash: bool) -> Vec<Dl> {
    tokio::task::spawn_blocking(move || {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
        let chunk = items.len().div_ceil(threads).max(1);
        std::thread::scope(|s| {
            let handles: Vec<_> = items
                .chunks(chunk)
                .map(|part| s.spawn(move || part.iter().filter(|d| !d.is_valid(verify_hash)).cloned().collect::<Vec<_>>()))
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        })
    })
    .await
    .unwrap_or_default()
}

/// Масове завантаження з прогресом у `task` (етап `stage`). Повертає кількість завантажених файлів.
pub async fn download_all(
    http: &reqwest::Client,
    items: Vec<Dl>,
    task: &Arc<Task>,
    stage: &str,
    concurrency: usize,
    verify_hash: bool,
) -> Result<usize> {
    let mut seen = HashSet::new();
    let items: Vec<Dl> = items.into_iter().filter(|d| seen.insert(d.path.clone())).collect();
    if verify_hash && items.len() > 1 {
        task.stage("verify", 0, "");
    }
    let needed = missing(items, verify_hash).await;
    if needed.is_empty() {
        return Ok(0);
    }
    let bytes_mode = needed.iter().all(|d| d.size.is_some());
    let total = if bytes_mode { needed.iter().map(|d| d.size.unwrap_or(0)).sum() } else { needed.len() as u64 };
    task.stage(stage, total, if bytes_mode { "bytes" } else { "items" });
    let count = needed.len();
    let mut results = futures::stream::iter(needed)
        .map(|dl| {
            let task = task.clone();
            async move {
                task.check()?;
                let r = download(http, &dl, bytes_mode.then_some(&*task)).await;
                if !bytes_mode {
                    task.add(1);
                }
                r
            }
        })
        .buffer_unordered(concurrency.max(1));
    while let Some(r) = results.next().await {
        r?;
    }
    Ok(count)
}
