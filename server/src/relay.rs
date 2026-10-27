//! Ретранслятор LAN-світів. Один TCP-порт для всіх:
//! - гравець Minecraft підключається на `<код>.<домен>:<порт>`, код береться з handshake;
//! - лаунчер хоста тримає керівне з'єднання, а на кожного гравця відкриває окреме з'єднання даних.
//! Службові з'єднання починаються з байта 0x00 і "LIA" — у Minecraft пакет нульової довжини неможливий.
use crate::{api, App};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

struct Host {
    user: i64,
    tx: mpsc::UnboundedSender<String>,
}

#[derive(Default)]
pub struct Hub {
    hosts: Mutex<HashMap<String, Host>>,
    /// Гравці, що чекають на з'єднання даних від хоста: id → (сокет, уже прочитані байти).
    pending: Mutex<HashMap<String, (TcpStream, Vec<u8>)>>,
}

impl Hub {
    pub fn owner(&self, code: &str) -> Option<i64> {
        self.hosts.lock().unwrap().get(code).map(|h| h.user)
    }
}

pub async fn listen(app: Arc<App>, port: u16) {
    let l = TcpListener::bind(("0.0.0.0", port)).await.expect("порт ретранслятора");
    loop {
        let Ok((s, _)) = l.accept().await else { continue };
        let _ = s.set_nodelay(true);
        let app = app.clone();
        tokio::spawn(async move {
            if let Ok(Ok(Some(rest))) = tokio::time::timeout(Duration::from_secs(15), route(&app, s)).await {
                rest.await;
            }
        });
    }
}

type Cont = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Розбирає початок з'єднання (з тайм-аутом) і повертає довгу частину роботи, яка вже без тайм-ауту.
async fn route(app: &Arc<App>, mut s: TcpStream) -> std::io::Result<Option<Cont>> {
    let first = s.read_u8().await?;
    if first == 0 {
        let mut magic = [0u8; 3];
        s.read_exact(&mut magic).await?;
        if &magic != b"LIA" {
            return Ok(None);
        }
        let mut r = BufReader::new(s);
        let mut line = String::new();
        (&mut r).take(4096).read_line(&mut line).await?;
        let v: Value = serde_json::from_str(&line).unwrap_or_default();
        let s = r.into_inner();
        return Ok(match v["kind"].as_str() {
            Some("host") => host(app.clone(), s, v["token"].as_str().unwrap_or("")),
            Some("data") => data(app.clone(), s, v["id"].as_str().unwrap_or("")),
            _ => None,
        });
    }
    if first == 0xFE {
        return Ok(None); // старий пінг до 1.7
    }
    player(app, s, first).await
}

async fn varint(s: &mut TcpStream, raw: &mut Vec<u8>, first: Option<u8>) -> std::io::Result<i32> {
    let mut out = 0i32;
    for i in 0..5 {
        let b = match (i, first) {
            (0, Some(b)) => b,
            _ => s.read_u8().await?,
        };
        raw.push(b);
        out |= ((b & 0x7F) as i32) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(out);
        }
    }
    Err(std::io::ErrorKind::InvalidData.into())
}

fn read_var(buf: &[u8], pos: &mut usize) -> Option<i32> {
    let mut out = 0i32;
    for i in 0..5 {
        let b = *buf.get(*pos)?;
        *pos += 1;
        out |= ((b & 0x7F) as i32) << (7 * i);
        if b & 0x80 == 0 {
            return Some(out);
        }
    }
    None
}

fn write_var(out: &mut Vec<u8>, mut v: i32) {
    loop {
        let b = (v & 0x7F) as u8;
        v = ((v as u32) >> 7) as i32;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// Код світу з адреси в handshake: "ab12cd.play.lia.starnure.dev" → "ab12cd".
/// Forge дописує "\0FML…\0", деякі клієнти — крапку в кінці.
pub fn code_from_host(host: &str, domain: &str) -> Option<String> {
    let host = host.split('\0').next()?.trim_end_matches('.').to_ascii_lowercase();
    let code = host.strip_suffix(&format!(".{domain}"))?;
    (code.len() == 6 && code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())).then(|| code.to_string())
}

async fn player(app: &Arc<App>, mut s: TcpStream, first: u8) -> std::io::Result<Option<Cont>> {
    let mut raw = vec![];
    let len = varint(&mut s, &mut raw, Some(first)).await?;
    if !(1..=1024).contains(&len) {
        return Ok(None);
    }
    let mut pkt = vec![0u8; len as usize];
    s.read_exact(&mut pkt).await?;
    raw.extend_from_slice(&pkt);
    let mut p = 0;
    if read_var(&pkt, &mut p) != Some(0) {
        return Ok(None);
    }
    read_var(&pkt, &mut p);
    let Some(hl) = read_var(&pkt, &mut p) else { return Ok(None) };
    let Some(host) = pkt.get(p..p + hl as usize).and_then(|b| std::str::from_utf8(b).ok()) else { return Ok(None) };
    p += hl as usize + 2;
    let next = read_var(&pkt, &mut p).unwrap_or(0);
    let code = code_from_host(host, &app.cfg.play_domain);
    let tx = code.as_ref().and_then(|c| app.relay.hosts.lock().unwrap().get(c).map(|h| h.tx.clone()));
    let Some(tx) = tx else {
        // Світу немає: на вході пояснюємо гравцю, а не просто рвемо з'єднання.
        if next == 2 {
            let msg = json!({ "text": "Цей світ уже закрито або адреса неправильна." }).to_string();
            let mut body = vec![0u8];
            write_var(&mut body, msg.len() as i32);
            body.extend_from_slice(msg.as_bytes());
            let mut out = vec![];
            write_var(&mut out, body.len() as i32);
            out.extend_from_slice(&body);
            let _ = s.write_all(&out).await;
        }
        return Ok(None);
    };
    let id = crate::ygg::hex(&rand::random::<[u8; 12]>());
    app.relay.pending.lock().unwrap().insert(id.clone(), (s, raw));
    if tx.send(json!({ "open": id }).to_string()).is_err() {
        app.relay.pending.lock().unwrap().remove(&id);
        return Ok(None);
    }
    let app = app.clone();
    Ok(Some(Box::pin(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        app.relay.pending.lock().unwrap().remove(&id);
    })))
}

fn data(app: Arc<App>, mut host: TcpStream, id: &str) -> Option<Cont> {
    let (mut player, raw) = app.relay.pending.lock().unwrap().remove(id)?;
    Some(Box::pin(async move {
        if host.write_all(&raw).await.is_ok() {
            let _ = tokio::io::copy_bidirectional(&mut player, &mut host).await;
        }
    }))
}

fn host(app: Arc<App>, s: TcpStream, token: &str) -> Option<Cont> {
    let user = api::token_user(&app, token)?;
    // ponytail: без ліміту гравців на світ; додати, якщо хтось почне роздавати світ на сотні людей.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let code = {
        let mut hosts = app.relay.hosts.lock().unwrap();
        if hosts.values().filter(|h| h.user == user.id).count() >= 3 {
            return None;
        }
        let code = loop {
            let c: String = rand::random::<[u8; 6]>().iter().map(|b| b"abcdefghijkmnpqrstuvwxyz23456789"[(*b % 32) as usize] as char).collect();
            if !hosts.contains_key(&c) {
                break c;
            }
        };
        hosts.insert(code.clone(), Host { user: user.id, tx: tx.clone() });
        code
    };
    Some(Box::pin(async move {
        let (r, mut w) = s.into_split();
        let hello = json!({ "code": code, "address": format!("{code}.{}:{}", app.cfg.play_domain, app.cfg.relay_port) }).to_string() + "\n";
        if w.write_all(hello.as_bytes()).await.is_ok() {
            let mut lines = BufReader::new(r).lines();
            let mut ping = tokio::time::interval(Duration::from_secs(20));
            loop {
                tokio::select! {
                    m = rx.recv() => match m {
                        Some(m) => if w.write_all((m + "\n").as_bytes()).await.is_err() { break },
                        None => break,
                    },
                    l = lines.next_line() => match l { Ok(Some(_)) => {}, _ => break },
                    _ = ping.tick() => if w.write_all(b"{\"ping\":1}\n").await.is_err() { break },
                }
            }
        }
        app.relay.hosts.lock().unwrap().remove(&code);
        api::world_closed(&app, user.id, &code);
    }))
}

#[cfg(test)]
mod tests {
    use super::code_from_host;

    #[test]
    fn codes() {
        let d = "play.lia.starnure.dev";
        assert_eq!(code_from_host("ab12cd.play.lia.starnure.dev", d).as_deref(), Some("ab12cd"));
        assert_eq!(code_from_host("AB12CD.play.lia.starnure.dev.", d).as_deref(), Some("ab12cd"));
        assert_eq!(code_from_host("ab12cd.play.lia.starnure.dev\0FML3\0", d).as_deref(), Some("ab12cd"));
        assert_eq!(code_from_host("play.lia.starnure.dev", d), None);
        assert_eq!(code_from_host("x.ab12cd.play.lia.starnure.dev", d), None);
    }
}
