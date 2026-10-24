//! Остання версія і картинка для шапки головної — з нотаток до оновлень у джерелі лаунчера Mojang.
use crate::ctx::Ctx;
use crate::error::Result;
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

