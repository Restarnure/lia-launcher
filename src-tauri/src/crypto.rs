//! Шифрування файлу акаунтів ключем поточного користувача Windows (DPAPI):
//! скопійований на інший ПК чи іншому користувачу файл токенів не розшифрується.
use crate::error::{err, Result};

#[cfg(windows)]
fn dpapi(data: &[u8], protect: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = if protect {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        };
        if ok == 0 || output.pbData.is_null() {
            return Err(err("crypto", "DPAPI не змогла обробити дані акаунтів"));
        }
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as _);
        Ok(out)
    }
}

const MAGIC: &[u8] = b"LIA1";

pub fn seal(data: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        let mut out = MAGIC.to_vec();
        out.extend(dpapi(data, true)?);
        Ok(out)
    }
    #[cfg(not(windows))]
    Ok(data.to_vec())
}

pub fn open(data: &[u8]) -> Result<Vec<u8>> {
    match data.strip_prefix(MAGIC) {
        #[cfg(windows)]
        Some(sealed) => dpapi(sealed, false),
        #[cfg(not(windows))]
        Some(_) => Err(err("crypto", "файл акаунтів зашифровано на Windows")),
        None => Ok(data.to_vec()),
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn roundtrip() {
        let secret = b"refresh-token-123";
        let sealed = super::seal(secret).unwrap();
        assert!(!sealed.windows(secret.len()).any(|w| w == secret));
        assert_eq!(super::open(&sealed).unwrap(), secret);
    }
}
