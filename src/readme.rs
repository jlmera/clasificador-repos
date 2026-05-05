//! Render de README.md → _README.html.
//! La traducción al español la maneja `llm.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use pulldown_cmark::{html, Options, Parser};
use regex::Regex;

use crate::scan::read_text_safe;

// Regex para insertar líneas en blanco al inicio/cierre de bloques HTML.
// CommonMark: si una línea arranca con <div>, <center>, <p>… SIN línea en
// blanco siguiente, todo el bloque es HTML opaco y NO se procesa el markdown
// adentro. Por eso en READMEs tipo:
//     <div align="center">
//     # Coolify
//     </div>
// el "# Coolify" sale como texto literal. Pre-procesamos insertando blank
// lines para que pulldown-cmark sepa que puede entrar a procesar.
static RE_BLOCK_HTML_OPEN: Lazy<Regex> = Lazy::new(|| {
    // Captura la línea entera "<div align="center">" o "<p>" o "<center>".
    Regex::new(
        r"(?im)^(\s*<(?:div|center|p|section|article|main|header|footer|nav|aside)(?:\s+[^>]*)?>)\s*$"
    ).unwrap()
});
static RE_BLOCK_HTML_CLOSE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?im)^(\s*</(?:div|center|p|section|article|main|header|footer|nav|aside)>)\s*$"
    ).unwrap()
});

const README_TEMPLATE: &str = include_str!("../templates/readme_viewer.html");

/// Renderiza el README.md del repo a `<repo>/_README.html`.
/// Devuelve la ruta o None si no hay README.
/// Incremental: si el HTML es más nuevo que el .md, no regenera.
pub fn render_readme_html(repo: &Path, categoria: &str, root: &Path, force: bool) -> Option<PathBuf> {
    let readme_path = find_readme(repo);
    let out = repo.join("_README.html");

    let Some(readme) = readme_path else {
        // Sin README: generar HTML "no readme" si no existe
        if force || !out.exists() {
            let body = format!(
                r#"<div class="no-readme">📭 Este repositorio no tiene README.<br>
<small>Carpeta: <code>{}</code></small></div>"#,
                html_escape(&repo.to_string_lossy())
            );
            let html_doc = build_html(repo.file_name()?.to_string_lossy().as_ref(), categoria, root, &body);
            let _ = fs::write(&out, html_doc);
            return Some(out);
        }
        return out.exists().then_some(out);
    };

    // Incremental: skip si el HTML es más nuevo
    if !force && out.exists() {
        if let (Ok(html_meta), Ok(md_meta)) = (fs::metadata(&out), fs::metadata(&readme)) {
            if let (Ok(h), Ok(m)) = (html_meta.modified(), md_meta.modified()) {
                if h >= m { return Some(out); }
            }
        }
    }

    let content = read_text_safe(&readme, 999_999);
    let body = markdown_to_html(&content);
    let title = repo.file_name()?.to_string_lossy().to_string();
    let html_doc = build_html(&title, categoria, root, &body);
    fs::write(&out, html_doc).ok()?;
    Some(out)
}

fn find_readme(repo: &Path) -> Option<PathBuf> {
    let candidates = ["README.md","README.MD","Readme.md","readme.md","README","README.txt"];
    for c in candidates {
        let p = repo.join(c);
        if p.exists() { return Some(p); }
    }
    None
}

fn markdown_to_html(md: &str) -> String {
    // Pre-procesar: insertar línea en blanco después de tags de apertura
    // <div>/<center>/<p>… y antes de cierres correspondientes, así pulldown
    // procesa el markdown contenido en lugar de tratarlo como HTML opaco.
    let pre1 = RE_BLOCK_HTML_OPEN.replace_all(md,  "$1\n");
    let pre2 = RE_BLOCK_HTML_CLOSE.replace_all(&pre1, "\n$1");

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_GFM);
    let parser = Parser::new_ext(&pre2, opts);
    let mut html_out = String::new();
    html::push_html(&mut html_out, parser);
    html_out
}

fn build_html(title: &str, categoria: &str, root: &Path, body: &str) -> String {
    let back = format!(
        "file:///{}",
        root.join("buscador.html").to_string_lossy().replace('\\', "/")
    );
    README_TEMPLATE
        .replace("__TITLE__", &html_escape(title))
        .replace("__CAT__", &html_escape(categoria))
        .replace("__BACK__", &back)
        .replace("__BODY__", body)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&#39;")
}
