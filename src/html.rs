//! Generador del buscador.html con datos JSON embebidos.

use std::path::Path;

use anyhow::Result;

use crate::atomic_io::write_atomic_string;
use crate::index::RepoEntry;
use crate::paths::buscador_html_path;

const HTML_TEMPLATE: &str = include_str!("../templates/buscador.html");

pub fn generate_html(root: &Path, data: &[RepoEntry]) -> Result<std::path::PathBuf> {
    let json_str = serde_json::to_string_pretty(data)?;
    let html = HTML_TEMPLATE.replace("__DATA__", &json_str);
    let out = buscador_html_path(root);
    // Escritura atómica: el archivo lo abre el navegador en otra sesión
    // (Syncthing en la otra máquina) y un truncamiento parcial deja el
    // buscador roto hasta el siguiente reindex.
    write_atomic_string(&out, &html)?;
    Ok(out)
}
