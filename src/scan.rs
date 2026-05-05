//! Scan de un repositorio: lee README + manifests + extensiones.
//! Equivalente a `scan_repo()` y helpers del clasificador.py Python.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::categories::{lang_for_ext, EXCLUDE_DIRS};

// Regex compiladas una sola vez al primer uso. Antes se compilaban por cada
// llamada a clean_readme_summary (4 × 122 repos = 488 compilaciones por reindex).
static RE_HTML:  Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]+>").unwrap());
static RE_BADGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[!\[[^\]]*\]\([^)]+\)\]\([^)]+\)").unwrap());
static RE_LINK:  Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap());
static RE_WS:    Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoScan {
    pub name: String,
    pub path: String,
    pub descripcion_en: String,
    pub readme_full: String,
    pub lenguaje_principal: String,
    pub lenguajes_secundarios: Vec<String>,
    pub stack: Vec<String>,
}

/// Lee un archivo de texto probando varias codificaciones.
pub fn read_text_safe(path: &Path, max_chars: usize) -> String {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    // Intento 1: UTF-8 (con BOM tolerable)
    let stripped: &[u8] = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        &bytes[..]
    };
    if let Ok(s) = std::str::from_utf8(stripped) {
        return truncate_chars(s, max_chars);
    }
    // Fallback: cp1252 / latin-1 (encoding_rs)
    let (cow, _, had_errors) = encoding_rs::WINDOWS_1252.decode(&bytes);
    let _ = had_errors;
    truncate_chars(&cow, max_chars)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars { break; }
        out.push(c);
    }
    out
}

/// Detecta el lenguaje principal y secundarios contando extensiones
/// (top-level + 1 nivel, excluyendo node_modules, .git, etc.).
pub fn detect_language(repo: &Path) -> (String, Vec<String>) {
    let mut counts: HashMap<&'static str, u64> = HashMap::new();
    let walker = walkdir::WalkDir::new(repo)
        .max_depth(3)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 { return true; }
            let name = e.file_name().to_string_lossy();
            !EXCLUDE_DIRS.iter().any(|d| *d == name)
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() { continue; }
        let p = entry.path();
        let ext = match p.extension() {
            Some(e) => format!(".{}", e.to_string_lossy()),
            None => continue,
        };
        if let Some(lang) = lang_for_ext(&ext) {
            *counts.entry(lang).or_insert(0) += 1;
        }
    }

    if counts.is_empty() {
        return ("desconocido".to_string(), vec![]);
    }
    let mut ordered: Vec<(&str, u64)> = counts.into_iter().collect();
    ordered.sort_by(|a, b| b.1.cmp(&a.1));
    let primary = ordered[0].0.to_string();
    let secondaries: Vec<String> = ordered.iter()
        .skip(1)
        .filter(|(n, _)| *n != "Markdown" && *n != primary)
        .take(4)
        .map(|(n, _)| n.to_string())
        .collect();
    (primary, secondaries)
}

/// Detecta tecnologías presentes (docker, rust, go, node, python, php, github actions).
pub fn detect_stack(repo: &Path) -> Vec<String> {
    let mut stack = Vec::new();
    let has = |p: &str| repo.join(p).exists();
    if has("Dockerfile") || has("docker-compose.yml") || has("docker-compose.yaml") {
        stack.push("docker".into());
    }
    if has("Cargo.toml") { stack.push("rust".into()); }
    if has("go.mod")     { stack.push("go".into()); }
    if has("package.json") { stack.push("node".into()); }
    if has("requirements.txt") || has("pyproject.toml") {
        stack.push("python".into());
    }
    if has("composer.json") { stack.push("php".into()); }
    stack
}

/// Extrae una descripción corta: primero `package.json` o `composer.json`,
/// si no, las primeras líneas no decorativas del README.
pub fn extract_description(repo: &Path) -> String {
    // 1) package.json / composer.json
    // Una sola syscall: read_to_string falla limpio si el archivo no existe.
    for (fname, key) in &[("package.json", "description"), ("composer.json", "description")] {
        if let Ok(text) = fs::read_to_string(repo.join(fname)) {
            if let Ok(j) = serde_json::from_str::<Json>(&text) {
                if let Some(s) = j.get(*key).and_then(|v| v.as_str()) {
                    return s.trim().to_string();
                }
            }
        }
    }

    // 2) README → primeras líneas significativas
    for cand in &["README.md","README.MD","Readme.md","readme.md","README","README.txt"] {
        let p = repo.join(cand);
        if p.exists() {
            let txt = read_text_safe(&p, 4000);
            return clean_readme_summary(&txt);
        }
    }
    String::new()
}

fn clean_readme_summary(txt: &str) -> String {
    let mut useful: Vec<&str> = Vec::new();
    for line in txt.lines() {
        let ls = line.trim();
        if ls.is_empty() { continue; }
        // Saltar líneas decorativas
        if ls.chars().all(|c| matches!(c, '#' | '=' | '-' | '*' | ' ')) { continue; }
        if ls.starts_with("![") || ls.starts_with("<img")
            || ls.starts_with("<p ") || ls.starts_with("<div") {
            continue;
        }
        if ls.contains("shields.io") || ls.contains("img.shields") { continue; }
        // Saltar líneas tipo "🇺🇸 English | 🇨🇳 中文 | 🇯🇵 日本語…" — son
        // navegación multi-idioma de READMEs, no contenido real del proyecto.
        // Los regional indicator symbols viven en U+1F1E6..U+1F1FF y forman
        // banderas cuando se combinan en pares.
        let flag_count = ls.chars().filter(|c| {
            matches!(*c, '\u{1F1E6}'..='\u{1F1FF}')
        }).count();
        if flag_count >= 3 { continue; }
        useful.push(ls);
        let total: usize = useful.iter().map(|s| s.len() + 1).sum();
        if total > 280 { break; }
    }
    let blob = useful.join(" ");
    // Limpieza markdown/HTML usando las regex estáticas (compiladas 1 vez).
    let blob = RE_HTML.replace_all(&blob, "");
    let blob = RE_BADGE.replace_all(&blob, "");
    let blob = RE_LINK.replace_all(&blob, "$1");
    let blob = RE_WS.replace_all(&blob, " ").to_string();
    let blob = blob.trim();
    blob.chars().take(280).collect()
}

/// Lee hasta `max_chars` del README para enviarle al LLM (idiom_full).
pub fn read_full_readme(repo: &Path, max_chars: usize) -> String {
    for cand in &["README.md","README.MD","Readme.md","readme.md","README"] {
        let p = repo.join(cand);
        if p.exists() {
            return read_text_safe(&p, max_chars);
        }
    }
    String::new()
}

/// Snapshot completo del repo (equivalente a Python `scan_repo`).
pub fn scan_repo(repo: &Path) -> Result<RepoScan> {
    let (primary, secondaries) = detect_language(repo);
    Ok(RepoScan {
        name: repo.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        path: repo.to_string_lossy().into_owned(),
        descripcion_en: extract_description(repo),
        readme_full: read_full_readme(repo, 3500),
        lenguaje_principal: primary,
        lenguajes_secundarios: secondaries,
        stack: detect_stack(repo),
    })
}
