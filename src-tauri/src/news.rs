//! Нотатки до оновлень Minecraft: Java Edition з офіційного джерела лаунчера Mojang.
use crate::ctx::Ctx;
use crate::error::{err, Result};
use crate::net;
use serde::Serialize;

const BASE: &str = "https://launchercontent.mojang.com";

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub version: String,
    pub kind: String,
    pub image: Option<String>,
    pub content_path: String,
    pub date: String,
    pub short_text: String,
}

pub async fn patch_notes(ctx: &Ctx) -> Result<Vec<Entry>> {
    let v: serde_json::Value = net::get_json_cached(&ctx.http, &format!("{BASE}/v2/javaPatchNotes.json"), &ctx.paths.cache_file("patch-notes.json")).await?;
    let s = |x: &serde_json::Value| x.as_str().unwrap_or("").to_string();
    Ok(v["entries"]
        .as_array()
        .map(|a| {
            a.iter()
                .take(40)
                .map(|e| Entry {
                    id: s(&e["id"]),
                    title: s(&e["title"]),
                    version: s(&e["version"]),
                    kind: s(&e["type"]),
                    image: e["image"]["url"].as_str().map(|u| format!("{BASE}{u}")),
                    content_path: s(&e["contentPath"]),
                    date: s(&e["date"]),
                    short_text: s(&e["shortText"]),
                })
                .collect()
        })
        .unwrap_or_default())
}

/// HTML нотаток (фронтенд очищує його DOMPurify перед показом).
pub async fn patch_body(ctx: &Ctx, content_path: &str) -> Result<String> {
    let ok = content_path.starts_with("javaPatchNotes/")
        && content_path.ends_with(".json")
        && !content_path.contains("..")
        && content_path.chars().all(|c| c.is_ascii_alphanumeric() || "/-_.".contains(c));
    if !ok {
        return Err(err("invalid", "неприпустимий шлях нотаток"));
    }
    let v: serde_json::Value = net::get_json(&ctx.http, &format!("{BASE}/v2/{content_path}")).await?;
    Ok(v["body"].as_str().unwrap_or("").to_string())
}
