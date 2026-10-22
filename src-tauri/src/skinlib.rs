//! Бібліотека скінів (як у Modrinth): збережені PNG, модель, який скін застосовано до акаунта.
use crate::error::{err, Context, Result};
use crate::paths::Paths;
use crate::{skins, util};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Skin {
    pub id: String,
    pub name: String,
    pub slim: bool,
    pub added: i64,
    /// Звідки скін: "file", "nick:<нік>", "account".
    #[serde(default)]
    pub source: String,
}

/// Який скін бібліотеки стоїть на акаунті й за якою адресою текстури Mojang його видно.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub skin: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Library {
    #[serde(default)]
    skins: Vec<Skin>,
    #[serde(default)]
    applied: HashMap<String, Applied>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkinView {
    #[serde(flatten)]
    pub skin: Skin,
    pub data: String,
}

fn dir() -> PathBuf {
    Paths::config_home().join("skins")
}

fn load() -> Library {
    util::read_json(&dir().join("skins.json")).unwrap_or_default()
}

fn save(lib: &Library) -> Result<()> {
    util::write_json(&dir().join("skins.json"), lib)
}

fn check_id(id: &str) -> Result<()> {
    if id.len() != 16 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(err("invalid", "неприпустимий ідентифікатор скіна"));
    }
    Ok(())
}

/// Перевірка PNG скіна (64×64 або старий 64×32) і чи це тонка модель (Alex):
/// у тонких руках зони (50,16 2×4) і (54,20 2×12) порожні — прозорі або суцільно чорні.
pub fn inspect(bytes: &[u8]) -> Result<bool> {
    let bad = |e: String| err("invalid_skin", e);
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| bad(e.to_string()))?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w != 64 || (h != 64 && h != 32) {
        return Err(bad(format!("розмір {w}×{h}, а скін має бути 64×64")));
    }
    if h == 32 {
        return Ok(false);
    }
    let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| bad("завеликий файл".into()))?];
    let frame = reader.next_frame(&mut buf).map_err(|e| bad(e.to_string()))?;
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::GrayscaleAlpha => 2,
        // Без альфа-каналу прозорих пікселів немає — класична модель.
        _ => return Ok(false),
    };
    let px = |x: u32, y: u32| {
        let i = ((y * w + x) as usize) * channels;
        let a = buf[i + channels - 1];
        let black = buf[i..i + channels - 1].iter().all(|&c| c == 0);
        (a, black)
    };
    let area = |x0: u32, y0: u32, ww: u32, hh: u32| (y0..y0 + hh).flat_map(move |y| (x0..x0 + ww).map(move |x| (x, y)));
    let cells: Vec<(u8, bool)> = area(50, 16, 2, 4).chain(area(54, 20, 2, 12)).map(|(x, y)| px(x, y)).collect();
    Ok(cells.iter().all(|(a, _)| *a == 0) || cells.iter().all(|(a, black)| *a == 255 && *black))
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))[..16].to_string()
}

pub fn list() -> Vec<SkinView> {
    let lib = load();
    let mut out: Vec<SkinView> = lib
        .skins
        .into_iter()
        .filter_map(|s| {
            let bytes = std::fs::read(dir().join(format!("{}.png", s.id))).ok()?;
            Some(SkinView { data: skins::png_url(&bytes), skin: s })
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.skin.added));
    out
}

/// Додає PNG у бібліотеку (той самий файл двічі не дублюється). Модель визначається сама.
pub fn add(bytes: &[u8], name: &str, slim: Option<bool>, source: &str) -> Result<SkinView> {
    let detected = inspect(bytes)?;
    let id = hash(bytes);
    let mut lib = load();
    if let Some(s) = lib.skins.iter().find(|s| s.id == id) {
        return Ok(SkinView { skin: s.clone(), data: skins::png_url(bytes) });
    }
    util::write_atomic(&dir().join(format!("{id}.png")), bytes)?;
    let name: String = name.trim().chars().take(32).collect();
    let skin = Skin { id, name: if name.is_empty() { "Скін".into() } else { name }, slim: slim.unwrap_or(detected), added: util::now_ms(), source: source.into() };
    lib.skins.push(skin.clone());
    save(&lib)?;
    Ok(SkinView { skin, data: skins::png_url(bytes) })
}

pub fn update(id: &str, name: Option<String>, slim: Option<bool>) -> Result<()> {
    check_id(id)?;
    let mut lib = load();
    let s = lib.skins.iter_mut().find(|s| s.id == id).ok_or_else(|| err("invalid", "скін не знайдено"))?;
    if let Some(n) = name {
        let n: String = n.trim().chars().take(32).collect();
        if !n.is_empty() {
            s.name = n;
        }
    }
    if let Some(v) = slim {
        s.slim = v;
    }
    save(&lib)
}

pub fn remove(id: &str) -> Result<()> {
    check_id(id)?;
    let mut lib = load();
    lib.skins.retain(|s| s.id != id);
    lib.applied.retain(|_, a| a.skin != id);
    save(&lib)?;
    let _ = std::fs::remove_file(dir().join(format!("{id}.png")));
    Ok(())
}

pub fn bytes_of(id: &str) -> Result<Vec<u8>> {
    check_id(id)?;
    let p = dir().join(format!("{id}.png"));
    std::fs::read(&p).ctx(|| p.display().to_string())
}

pub fn applied() -> HashMap<String, Applied> {
    load().applied
}

pub fn set_applied(account_id: &str, skin_id: &str, url: &str) -> Result<()> {
    let mut lib = load();
    lib.applied.insert(account_id.to_string(), Applied { skin: skin_id.to_string(), url: url.to_string() });
    save(&lib)
}

/// Стандартні скіни (Стів, Алекс та інші) з уже встановленої гри — у інсталятор їх не кладемо.
pub fn defaults(paths: &Paths) -> Vec<SkinView> {
    let mut jars: Vec<_> = std::fs::read_dir(paths.versions())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join(format!("{}.jar", e.file_name().to_string_lossy())))
        .filter(|p| p.exists())
        .collect();
    jars.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    for jar in jars.into_iter().take(6) {
        let Ok(f) = std::fs::File::open(&jar) else { continue };
        let Ok(mut zip) = zip::ZipArchive::new(f) else { continue };
        let names: Vec<String> = zip
            .file_names()
            .filter_map(|n| n.ok().map(|c| c.to_string()))
            .filter(|n| n.starts_with("assets/minecraft/textures/entity/player/") && n.ends_with(".png"))
            .collect();
        let mut out = Vec::new();
        for n in names {
            let rest = &n["assets/minecraft/textures/entity/player/".len()..];
            let Some((model, file)) = rest.split_once('/') else { continue };
            let mut buf = Vec::new();
            if zip.by_name(&n).map(|mut e| e.read_to_end(&mut buf)).is_err() {
                continue;
            }
            let base = file.trim_end_matches(".png");
            let mut name = base.to_string();
            if let Some(c) = name.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            out.push(SkinView {
                skin: Skin { id: format!("default:{model}/{base}"), name, slim: model == "slim", added: 0, source: "default".into() },
                data: skins::png_url(&buf),
            });
        }
        if !out.is_empty() {
            out.sort_by(|a, b| a.skin.name.cmp(&b.skin.name).then(a.skin.slim.cmp(&b.skin.slim)));
            return out;
        }
    }
    Vec::new()
}

/// PNG стандартного скіна за id "default:<модель>/<ім'я>".
pub fn default_bytes(paths: &Paths, id: &str) -> Result<Vec<u8>> {
    let rest = id.strip_prefix("default:").ok_or_else(|| err("invalid", "не стандартний скін"))?;
    if rest.contains("..") || rest.chars().filter(|c| *c == '/').count() != 1 {
        return Err(err("invalid", "неприпустимий скін"));
    }
    defaults(paths)
        .into_iter()
        .find(|s| s.skin.id == id)
        .and_then(|s| {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.decode(s.data.trim_start_matches("data:image/png;base64,")).ok()
        })
        .ok_or_else(|| err("invalid", format!("стандартного скіна {rest} немає у встановленій грі")))
}

#[cfg(test)]
mod tests {
    use super::inspect;

    fn png(w: u32, h: u32, slim: bool) -> Vec<u8> {
        let mut px = vec![200u8; (w * h * 4) as usize];
        if slim {
            for (x0, y0, ww, hh) in [(50u32, 16u32, 2u32, 4u32), (54, 20, 2, 12)] {
                for y in y0..y0 + hh {
                    for x in x0..x0 + ww {
                        px[((y * w + x) * 4 + 3) as usize] = 0;
                    }
                }
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&px).unwrap();
        }
        out
    }

    #[test]
    fn detects_model_and_size() {
        assert!(!inspect(&png(64, 64, false)).unwrap());
        assert!(inspect(&png(64, 64, true)).unwrap());
        assert!(!inspect(&png(64, 32, false)).unwrap());
        assert!(inspect(&png(128, 128, false)).is_err());
        assert!(inspect(b"not a png").is_err());
    }
}
