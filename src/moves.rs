//! Helpers de filesystem: detección de "es repo", listado del inbox.
//! Equivalente a `is_repo_dir`, `find_inbox_repos`.

use std::path::{Path, PathBuf};

pub fn is_repo_dir(path: &Path) -> bool {
    if !path.is_dir() { return false; }
    let name = match path.file_name() {
        Some(n) => n.to_string_lossy().to_string(),
        None => return false,
    };
    if name.starts_with('_') || name.starts_with('.') { return false; }
    if path.join(".git").exists() { return true; }
    let manifests = [
        "README.md", "Readme.md", "readme.md", "README", "README.txt",
        "package.json", "Cargo.toml", "go.mod",
        "pyproject.toml", "composer.json", "Dockerfile",
    ];
    manifests.iter().any(|f| path.join(f).exists())
}

pub fn find_inbox_repos(inbox: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !inbox.exists() { return out; }
    if let Ok(rd) = std::fs::read_dir(inbox) {
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let p = entry.path();
            if is_repo_dir(&p) {
                out.push(p);
            }
        }
    }
    out
}
