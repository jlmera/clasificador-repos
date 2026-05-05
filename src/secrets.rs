//! secrets.rs — almacén cifrado de credenciales (Anthropic API key + GitHub PAT).
//!
//! Usa Windows DPAPI (CryptProtectData / CryptUnprotectData) para cifrar un
//! único `config.dat` junto al .exe. El payload es un JSON con campos opcionales
//! para que se puedan agregar más secretos sin romper compat.
//!
//! Compat: si el `config.dat` heredado de v1.0 contenía solo el string de la
//! Anthropic key (sin JSON), `load_secrets` lo detecta y lo trata como
//! `anthropic_key`. La próxima llamada a `save_secrets` migra a JSON.

use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::atomic_io::write_atomic;

#[cfg(windows)]
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};

const FILE_NAME: &str = "config.dat";

/// Carpeta donde está el .exe.
fn exe_dir() -> PathBuf {
    if let Ok(p) = std::env::current_exe() {
        if let Some(parent) = p.parent() {
            return parent.to_path_buf();
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    exe_dir().join(FILE_NAME)
}

// ── DPAPI low-level ─────────────────────────────────────────────────

#[cfg(windows)]
fn dpapi_encrypt(plaintext: &[u8]) -> Result<Vec<u8>> {
    use windows::core::PCWSTR;
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let desc = "clasificador-secrets".encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    unsafe {
        CryptProtectData(
            &mut input,
            PCWSTR(desc.as_ptr()),
            None,
            None,
            None,
            0,
            &mut output,
        )
        .map_err(|e| anyhow!("CryptProtectData falló: {}", e))?;
        let out_slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        Ok(out_slice.to_vec())
    }
}

#[cfg(windows)]
fn dpapi_decrypt(ciphertext: &[u8]) -> Result<Vec<u8>> {
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &mut input,
            None,
            None,
            None,
            None,
            0,
            &mut output,
        )
        .map_err(|e| anyhow!("CryptUnprotectData falló: {}", e))?;
        let out_slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        Ok(out_slice.to_vec())
    }
}

#[cfg(not(windows))]
fn dpapi_encrypt(_p: &[u8]) -> Result<Vec<u8>> { Err(anyhow!("DPAPI solo en Windows")) }
#[cfg(not(windows))]
fn dpapi_decrypt(_c: &[u8]) -> Result<Vec<u8>> { Err(anyhow!("DPAPI solo en Windows")) }

// ── Estructura JSON cifrada ─────────────────────────────────────────

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StoredSecrets {
    /// Anthropic API key (sk-ant-…) para traducción de READMEs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub anthropic_key: String,
    /// GitHub Personal Access Token (github_pat_… o ghp_…) para
    /// elevar el rate-limit de la API GitHub de 60 → 5000 req/h.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub github_pat: String,
}

/// Carga el JSON cifrado. Si `config.dat` está en formato legacy (string
/// plano = solo la Anthropic key), lo migra transparentemente.
pub fn load_secrets() -> StoredSecrets {
    let p = config_path();
    if !p.exists() { return StoredSecrets::default(); }
    let cipher = match fs::read(&p) {
        Ok(b) => b,
        Err(_) => return StoredSecrets::default(),
    };
    let plain = match dpapi_decrypt(&cipher) {
        Ok(p) => p,
        Err(_) => return StoredSecrets::default(),
    };
    let s = match String::from_utf8(plain) {
        Ok(s) => s,
        Err(_) => return StoredSecrets::default(),
    };
    // Intentar JSON primero; si falla, asumir legacy = string anthropic.
    if let Ok(parsed) = serde_json::from_str::<StoredSecrets>(&s) {
        parsed
    } else {
        StoredSecrets {
            anthropic_key: s.trim().to_string(),
            github_pat: String::new(),
        }
    }
}

/// Cifra y guarda. Si los dos campos están vacíos, borra el archivo en su lugar.
pub fn save_secrets(s: &StoredSecrets) -> Result<PathBuf> {
    let p = config_path();
    if s.anthropic_key.is_empty() && s.github_pat.is_empty() {
        if p.exists() { let _ = fs::remove_file(&p); }
        return Ok(p);
    }
    let json = serde_json::to_string(s)
        .map_err(|e| anyhow!("serializando secrets: {}", e))?;
    let cipher = dpapi_encrypt(json.as_bytes())?;
    // Escritura atómica: si el proceso crashea a mitad, el .tmp queda
    // pero config.dat (con tu API key + PAT) sigue intacto.
    write_atomic(&p, &cipher)?;
    Ok(p)
}

// ── API alto nivel: Anthropic key ───────────────────────────────────

pub fn save_api_key(api_key: &str) -> Result<PathBuf> {
    let key = api_key.trim().to_string();
    if key.is_empty() {
        return Err(anyhow!("La Anthropic API key está vacía"));
    }
    let mut s = load_secrets();
    s.anthropic_key = key;
    save_secrets(&s)
}

pub fn load_api_key() -> Option<String> {
    let s = load_secrets();
    if s.anthropic_key.is_empty() { None } else { Some(s.anthropic_key) }
}

pub fn delete_api_key() -> bool {
    let mut s = load_secrets();
    if s.anthropic_key.is_empty() { return false; }
    s.anthropic_key.clear();
    save_secrets(&s).is_ok()
}

pub fn has_api_key() -> bool {
    !load_secrets().anthropic_key.is_empty()
}

// ── API alto nivel: GitHub PAT ──────────────────────────────────────

pub fn save_github_pat(pat: &str) -> Result<PathBuf> {
    let pat = pat.trim().to_string();
    let mut s = load_secrets();
    s.github_pat = pat;  // permitir vaciar (string vacío = borrar)
    save_secrets(&s)
}

pub fn load_github_pat() -> Option<String> {
    let s = load_secrets();
    if s.github_pat.is_empty() { None } else { Some(s.github_pat) }
}

pub fn has_github_pat() -> bool {
    !load_secrets().github_pat.is_empty()
}
