use crate::error::{err, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use sha1::{Digest, Sha1};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).ctx(|| path.display().to_string())?;
    serde_json::from_slice(&bytes).ctx(|| path.display().to_string())
}

/// Запис через тимчасовий файл і перейменування: обрив посеред запису не лишає зіпсований JSON.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ctx(|| dir.display().to_string())?;
    }
    // Свій тимчасовий файл на кожен виклик: два одночасні записи одного файлу не змішуються.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{}-{n}", std::process::id()));
    std::fs::write(&tmp, bytes).ctx(|| tmp.display().to_string())?;
    std::fs::rename(&tmp, path).ctx(|| path.display().to_string())
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// `group:artifact:version[:classifier][@ext]` → `group/path/artifact/version/artifact-version[-classifier].ext`
pub fn maven_path(coord: &str) -> Option<String> {
    let (coord, ext) = match coord.split_once('@') {
        Some((c, e)) => (c, e),
        None => (coord, "jar"),
    };
    let parts: Vec<&str> = coord.split(':').collect();
    if parts.len() < 3 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let (group, artifact, version) = (parts[0], parts[1], parts[2]);
    let classifier = parts.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    Some(format!("{}/{artifact}/{version}/{artifact}-{version}{classifier}.{ext}", group.replace('.', "/")))
}

/// Відносний шлях з чужих даних (модпак, архів) не має виходити за межі `base`.
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let rel = rel.replace('\\', "/");
    let p = Path::new(&rel);
    let mut out = base.to_path_buf();
    for c in p.components() {
        match c {
            // Windows відкидає крапки й пробіли в кінці: «...» означає саму теку.
            Component::Normal(s) if !s.to_string_lossy().trim_end_matches(['.', ' ']).is_empty() => out.push(s),
            Component::CurDir => {}
            _ => return Err(err("invalid", format!("небезпечний шлях у файлі: {rel}"))),
        }
    }
    if out == base {
        return Err(err("invalid", format!("порожній шлях: {rel}")));
    }
    Ok(out)
}

/// Назва папки з довільної назви: латиниця, цифри, дефіс. Українські літери транслітеруються.
pub fn slugify(name: &str) -> String {
    let mut s = String::new();
    for ch in name.to_lowercase().chars() {
        let t = match ch {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "h", 'ґ' => "g", 'д' => "d", 'е' => "e", 'є' => "ie",
            'ж' => "zh", 'з' => "z", 'и' => "y", 'і' => "i", 'ї' => "i", 'й' => "i", 'к' => "k", 'л' => "l",
            'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u",
            'ф' => "f", 'х' => "kh", 'ц' => "ts", 'ч' => "ch", 'ш' => "sh", 'щ' => "shch", 'ь' => "", 'ю' => "iu",
            'я' => "ia", 'ё' => "e", 'ы' => "y", 'э' => "e", 'ъ' => "",
            c if c.is_ascii_alphanumeric() => {
                s.push(c);
                continue;
            }
            _ => "-",
        };
        s.push_str(t);
    }
    let mut out = String::new();
    for part in s.split('-').filter(|p| !p.is_empty()) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(part);
    }
    out.truncate(40);
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "zbirka".into() } else { out }
}

pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else { continue };
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                Err(_) => {}
            }
        }
    }
    total
}

pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        // Замок світу запущеної гри не копіюється (і не потрібен копії).
        if e.file_name() == "session.lock" {
            continue;
        }
        // is_dir() по шляху йде за посиланням: тека-junction (спільні mods/saves) копіюється вмістом.
        if e.path().is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else {
            std::fs::copy(e.path(), &dest).ctx(|| dest.display().to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven() {
        assert_eq!(maven_path("org.lwjgl:lwjgl:3.3.3").unwrap(), "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar");
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3:natives-windows").unwrap(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
        );
        assert_eq!(
            maven_path("de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412@zip").unwrap(),
            "de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412.zip"
        );
        assert!(maven_path("broken").is_none());
    }

    #[test]
    fn safe_paths() {
        let base = Path::new("C:/base");
        assert!(safe_join(base, "mods/a.jar").is_ok());
        assert!(safe_join(base, "../evil.jar").is_err());
        assert!(safe_join(base, "mods/../../evil").is_err());
        assert!(safe_join(base, "C:/Windows/evil").is_err());
        assert!(safe_join(base, "/etc/passwd").is_err());
        assert!(safe_join(base, "").is_err());
        assert!(safe_join(base, "...").is_err());
        assert!(safe_join(base, "mods/. .").is_err());
        assert!(safe_join(base, "mods/a.").is_ok());
    }

    /// Кілька записів одного файлу одночасно: кожен має вдатися, а файл — бути цілим.
    #[test]
    fn atomic_concurrent() {
        let path = std::env::temp_dir().join(format!("lia-atomic-{}.json", std::process::id()));
        let ok = std::thread::scope(|s| {
            let hs: Vec<_> = (0..8usize)
                .map(|i| {
                    let path = &path;
                    s.spawn(move || (0..30).all(|_| write_atomic(path, &vec![b'a' + i as u8; 1000 + i * 100]).is_ok()))
                })
                .collect();
            hs.into_iter().all(|h| h.join().unwrap())
        });
        let data = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(ok);
        assert!(data.iter().all(|&b| b == data[0]) && data.len() == 1000 + (data[0] - b'a') as usize * 100);
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Мій світ 1.21"), "mii-svit-1-21");
        assert_eq!(slugify("Fabric + Sodium!"), "fabric-sodium");
        assert_eq!(slugify("!!!"), "zbirka");
        assert_eq!(slugify("Щастя Їжака"), "shchastia-izhaka");
    }
}
