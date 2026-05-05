//! Rutas auxiliares: data/, wiki/, etc.
//! Definitivo: la carpeta de datos persistentes se llama `data/` para
//! reflejar que ya no son "tools" auxiliares sino el estado del clasificador.

use std::path::{Path, PathBuf};

/// Sub-carpeta `data/` donde viven los archivos de estado (índices, cachés).
/// Antes se llamaba `tools/`; renombrada en v1.0 al consolidar la versión Rust.
pub fn data_dir(root: &Path) -> PathBuf {
    let p = root.join("data");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// Ruta del archivo de descripciones en español.
pub fn descripciones_path(root: &Path) -> PathBuf {
    data_dir(root).join("descripciones_es.json")
}

/// Ruta del JSON principal del índice.
pub fn index_json_path(root: &Path) -> PathBuf {
    data_dir(root).join("repos_index.json")
}

/// Cache de IDs y consecutivos por repo.
pub fn repo_ids_path(root: &Path) -> PathBuf {
    data_dir(root).join("repo_ids.json")
}

/// Buscador HTML generado.
pub fn buscador_html_path(root: &Path) -> PathBuf {
    root.join("buscador.html")
}
