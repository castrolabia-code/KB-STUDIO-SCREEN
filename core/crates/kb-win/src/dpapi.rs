//! Protección de secretos en disco con DPAPI (ligada al usuario de Windows).
//! Sólo el mismo usuario en la misma PC puede descifrar.

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    unsafe {
        let v = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(blob.pbData as _)));
        v
    }
}

pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptProtectData(&input, windows::core::w!("KB STUDIO SCREEN"), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
        .map_err(|e| format!("CryptProtectData: {e}"))?;
    Ok(take(out))
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
        .map_err(|e| format!("CryptUnprotectData: {e}"))?;
    Ok(take(out))
}
