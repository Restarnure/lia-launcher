//! Опитування сервера за протоколом Server List Ping (онлайн, MOTD, іконка, пінг)
//! і налаштування закріпленого сервера (Shotam).
use crate::ctx::Ctx;
use crate::error::{err, Result};
use crate::instances::LOADERS;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn varint(mut v: i32, out: &mut Vec<u8>) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v = ((v as u32) >> 7) as i32;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

async fn read_varint(s: &mut TcpStream) -> Result<i32> {
    let mut v: i32 = 0;
    for i in 0..5 {
        let b = s.read_u8().await?;
        v |= ((b & 0x7f) as i32) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(err("ping", "завеликий VarInt"))
}

fn packet(id: i32, body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::new();
    varint(id, &mut inner);
    inner.extend_from_slice(body);
    let mut out = Vec::new();
    varint(inner.len() as i32, &mut out);
    out.extend(inner);
    out
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub online: i64,
    pub max: i64,
    pub version: String,
    pub motd: serde_json::Value,
    pub favicon: Option<String>,
    pub latency_ms: u64,
    pub players: Vec<String>,
}

async fn ping_inner(host: &str, port: u16) -> Result<Status> {
    let mut s = TcpStream::connect((host, port)).await?;
    s.set_nodelay(true)?;
    let mut hs = Vec::new();
    varint(767, &mut hs);
    varint(host.len() as i32, &mut hs);
    hs.extend_from_slice(host.as_bytes());
    hs.extend_from_slice(&port.to_be_bytes());
    varint(1, &mut hs);
    s.write_all(&packet(0, &hs)).await?;
    s.write_all(&packet(0, &[])).await?;

    let _len = read_varint(&mut s).await?;
    let _id = read_varint(&mut s).await?;
    let n = read_varint(&mut s).await?;
    if !(0..=1 << 21).contains(&n) {
        return Err(err("ping", "некоректна відповідь сервера"));
    }
    let mut buf = vec![0u8; n as usize];
    s.read_exact(&mut buf).await?;
    let v: serde_json::Value = serde_json::from_slice(&buf)?;

    let t = Instant::now();
    s.write_all(&packet(1, &1234i64.to_be_bytes())).await?;
    let latency = match tokio::time::timeout(Duration::from_secs(2), async {
        read_varint(&mut s).await?;
        read_varint(&mut s).await?;
        s.read_i64().await?;
        Ok::<_, crate::error::Error>(())
    })
    .await
    {
        Ok(Ok(())) => t.elapsed().as_millis() as u64,
        _ => 0,
    };
    Ok(Status {
        online: v["players"]["online"].as_i64().unwrap_or(0),
        max: v["players"]["max"].as_i64().unwrap_or(0),
        version: v["version"]["name"].as_str().unwrap_or("").to_string(),
        motd: v["description"].clone(),
        favicon: v["favicon"].as_str().filter(|f| f.starts_with("data:image/png;base64,")).map(str::to_string),
        latency_ms: latency,
        players: v["players"]["sample"].as_array().map(|a| a.iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect()).unwrap_or_default(),
    })
}

pub async fn ping(host: &str, port: u16) -> Result<Status> {
    match tokio::time::timeout(Duration::from_secs(6), ping_inner(host, port)).await {
        Ok(r) => r,
        Err(_) => Err(err("ping_timeout", format!("{host}:{port} не відповідає"))),
    }
}

// ---------------------------------------------------------------- закріплений сервер

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Featured {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub mc_version: String,
    pub loader: String,
    /// Модрінт-слаги модів, що ставляться в збірку сервера.
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub description_uk: String,
    #[serde(default)]
    pub description_en: String,
}

/// Вбудовані налаштування. Сервер може перевизначити їх файлом https://shotam.today/launcher.json
/// (той самий формат) — тоді нова версія гри не вимагає оновлення лаунчера.
pub fn builtin() -> Featured {
    Featured {
        id: "shotam".into(),
        name: "Shotam".into(),
        address: "shotam.today".into(),
        port: 25565,
        mc_version: "1.21.11".into(),
        loader: "fabric".into(),
        mods: vec!["simple-voice-chat".into()],
        website: Some("https://shotam.today".into()),
        description_uk: "Український сервер виживання з власними плагінами, косметикою й голосовим чатом.".into(),
        description_en: "Ukrainian survival server with custom plugins, cosmetics and voice chat.".into(),
    }
}

fn valid(f: &Featured) -> bool {
    let host_ok = !f.address.is_empty() && f.address.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    let mods_ok = f.mods.iter().all(|m| !m.is_empty() && m.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    // Назва стає назвою збірки: з недопустимою кнопка «Грати» не створила б її.
    let name_ok = crate::instances::validate_name(&f.name).is_ok();
    host_ok && mods_ok && name_ok && crate::mojang::install::check_id(&f.mc_version).is_ok() && LOADERS.contains(&f.loader.as_str()) && f.id == "shotam"
}

pub async fn featured(ctx: &Ctx) -> Featured {
    let remote = async {
        let resp = ctx.http.get("https://shotam.today/launcher.json").timeout(Duration::from_secs(4)).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json::<Featured>().await.ok().filter(valid)
    };
    remote.await.unwrap_or_else(builtin)
}

// ---------------------------------------------------------------- servers.dat

fn nbt_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// Запис сервера в списку: тіло TAG_Compound без байта типу.
fn server_entry(name: &str, ip: &str) -> Vec<u8> {
    let mut b = Vec::new();
    for (k, v) in [("name", name), ("ip", ip)] {
        b.push(8);
        nbt_str(&mut b, k);
        nbt_str(&mut b, v);
    }
    b.push(0);
    b
}

/// Список серверів гри (нестиснений NBT) з одним сервером.
pub fn servers_dat(name: &str, ip: &str) -> Vec<u8> {
    let mut b = vec![10u8];
    nbt_str(&mut b, "");
    b.push(9);
    nbt_str(&mut b, "servers");
    b.push(10);
    b.extend_from_slice(&1i32.to_be_bytes());
    b.extend(server_entry(name, ip));
    b.push(0); // кінець кореня
    b
}

/// Додає сервер на початок наявного списку. Гра пише корінь з єдиним тегом «servers» першим,
/// тож розбирати решту записів не треба. Незнайомий формат — None (файл не чіпаємо).
fn prepend_server(dat: &[u8], name: &str, ip: &str) -> Option<Vec<u8>> {
    if dat.len() < 19 || dat[..6] != [10, 0, 0, 9, 0, 7] || &dat[6..13] != b"servers" {
        return None;
    }
    let count = i32::from_be_bytes(dat[14..18].try_into().ok()?);
    // Порожній список гра пише з типом TAG_End.
    if !(dat[13] == 10 || (dat[13] == 0 && count == 0)) {
        return None;
    }
    let mut out = dat[..13].to_vec();
    out.push(10);
    out.extend_from_slice(&(count + 1).to_be_bytes());
    out.extend(server_entry(name, ip));
    out.extend_from_slice(&dat[18..]);
    Some(out)
}

/// Закріплений сервер у списку «Мультиплеєр» збірки: створює список або дописує сервер, якщо його там немає.
pub fn seed_servers(game_dir: &std::path::Path, f: &Featured) {
    if !f.name.is_ascii() {
        return;
    }
    let path = game_dir.join("servers.dat");
    let ip = if f.port == 25565 { f.address.clone() } else { format!("{}:{}", f.address, f.port) };
    let data = match std::fs::read(&path) {
        Ok(dat) if dat.windows(f.address.len()).any(|w| w == f.address.as_bytes()) => return,
        Ok(dat) => match prepend_server(&dat, &f.name, &ip) {
            Some(d) => d,
            None => return,
        },
        Err(_) => servers_dat(&f.name, &ip),
    };
    let _ = std::fs::create_dir_all(game_dir);
    let _ = crate::util::write_atomic(&path, &data);
}

#[cfg(test)]
mod tests {
    #[test]
    fn servers_nbt() {
        let b = super::servers_dat("Shotam", "shotam.today");
        // TAG_Compound "" → TAG_List "servers" of 1 TAG_Compound
        assert_eq!(&b[..3], &[10, 0, 0]);
        assert_eq!(&b[3..13], &[9, 0, 7, b's', b'e', b'r', b'v', b'e', b'r', b's']);
        assert_eq!(&b[13..18], &[10, 0, 0, 0, 1]);
        assert!(b.windows(12).any(|w| w == b"shotam.today"));
        assert_eq!(&b[b.len() - 2..], &[0, 0]);

        // Дописування на початок: лічильник +1, старий запис і хвіст кореня на місці.
        let two = super::prepend_server(&super::servers_dat("Old", "old.net"), "Shotam", "shotam.today").unwrap();
        assert_eq!(&two[13..18], &[10, 0, 0, 0, 2]);
        let shotam = two.windows(12).position(|w| w == b"shotam.today").unwrap();
        let old = two.windows(7).position(|w| w == b"old.net").unwrap();
        assert!(shotam < old);
        assert_eq!(&two[two.len() - 2..], &[0, 0]);
        // Порожній список (тип TAG_End) стає списком записів.
        let empty = [10, 0, 0, 9, 0, 7, b's', b'e', b'r', b'v', b'e', b'r', b's', 0, 0, 0, 0, 0, 0];
        assert_eq!(&super::prepend_server(&empty, "S", "s.net").unwrap()[13..18], &[10, 0, 0, 0, 1]);
        assert!(super::prepend_server(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "S", "s").is_none());
    }

    #[test]
    fn varints() {
        let mut v = Vec::new();
        super::varint(300, &mut v);
        assert_eq!(v, vec![0xac, 0x02]);
        v.clear();
        super::varint(-1, &mut v);
        assert_eq!(v, vec![0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(super::valid(&super::builtin()));
        let long = super::Featured { name: "S".repeat(49), ..super::builtin() };
        assert!(!super::valid(&long));
    }
}
