//! Detección de duplicados por nombre y por URL del remote git.
//! Equivalente a `index_existing_repos()` y `find_duplicate()` Python.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;

use crate::categories_config::load_or_default as load_cats_cfg;
use crate::scan::read_text_safe;

// Regex compiladas una sola vez. Se invocan por cada repo en index_existing_repos
// y find_duplicate; sin Lazy se compilarían 244 veces por reindex completo.
static RE_REMOTE_URL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?si)\[remote\s+"origin"\][^\[]*?url\s*=\s*([^\s\n]+)"#).unwrap()
});
static RE_GIT_AT: Lazy<Regex> = Lazy::new(|| Regex::new(r"^git@([^:]+):").unwrap());

#[derive(Debug, Default)]
pub struct ExistingIndex {
    pub by_name: HashMap<String, PathBuf>,
    pub by_url:  HashMap<String, PathBuf>,
}

/// Recorre todas las categorías (según el config) y construye índices
/// por nombre y por URL del remote.
pub fn index_existing_repos(root: &Path) -> ExistingIndex {
    let mut idx = ExistingIndex::default();
    let cfg = load_cats_cfg(root);
    for cat in &cfg.categorias {
        let cat_dir = root.join(&cat.id);
        if !cat_dir.is_dir() { continue; }
        if let Ok(rd) = std::fs::read_dir(&cat_dir) {
            for entry in rd.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    let p = entry.path();
                    idx.by_name.insert(name, p.clone());
                    if let Some(url) = get_git_remote_url(&p) {
                        idx.by_url.insert(url, p);
                    }
                }
            }
        }
    }
    idx
}

#[derive(Debug, Clone)]
pub enum DupReason {
    MismoNombre(PathBuf),
    MismaUrl(PathBuf),
}

pub fn find_duplicate(repo: &Path, existing: &ExistingIndex) -> Option<DupReason> {
    let name = repo.file_name()?.to_string_lossy().to_lowercase();
    if let Some(p) = existing.by_name.get(&name) {
        return Some(DupReason::MismoNombre(p.clone()));
    }
    if let Some(url) = get_git_remote_url(repo) {
        if let Some(p) = existing.by_url.get(&url) {
            return Some(DupReason::MismaUrl(p.clone()));
        }
    }
    None
}

/// Lee `.git/config` y devuelve la URL del remote 'origin' normalizada en lowercase.
pub fn get_git_remote_url(repo: &Path) -> Option<String> {
    let cfg = repo.join(".git").join("config");
    if !cfg.exists() { return None; }
    let txt = read_text_safe(&cfg, 4000);
    let cap = RE_REMOTE_URL.captures(&txt)?;
    let mut url = cap.get(1)?.as_str().trim().to_lowercase();
    // Normalización: .git, https://, git@host:owner/repo
    if url.ends_with(".git") {
        url.truncate(url.len() - 4);
    }
    if let Some(rest) = url.strip_prefix("https://") {
        url = rest.to_string();
    } else if let Some(rest) = url.strip_prefix("http://") {
        url = rest.to_string();
    }
    url = RE_GIT_AT.replace(&url, "$1/").to_string();
    Some(url)
}
