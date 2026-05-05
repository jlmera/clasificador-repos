//! Generador de wiki Obsidian — patrón LLM-Wiki / Karpathy.
//! Equivalente a `generate_obsidian_wiki()` Python.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::index::RepoEntry;

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

pub fn generate_obsidian_wiki(root: &Path, data: &[RepoEntry]) -> Result<PathBuf> {
    let wiki = root.join("wiki");
    fs::create_dir_all(wiki.join("categorias"))?;
    fs::create_dir_all(wiki.join("repos"))?;

    // Una página por repo
    for r in data {
        let title = &r.name;
        let page = wiki.join("repos").join(format!("{}.md", slug(title)));
        let stack_str = if r.stack.is_empty() { "—".to_string() } else { r.stack.join(", ") };
        let url_md = match &r.url_github {
            Some(u) => format!("[{0}]({0})", u),
            None    => "—".to_string(),
        };
        let warn = if r.necesita_traduccion { " ⚠️ (necesita traducción)" } else { "" };

        let stack_json = serde_json::to_string(&r.stack).unwrap_or_else(|_| "[]".into());
        let tags_json  = serde_json::to_string(&r.tags ).unwrap_or_else(|_| "[]".into());
        let id_str = r.id.map(|i| i.to_string()).unwrap_or_else(|| "null".into());

        let content = format!(
"---
title: {name}
id: {id}
consecutivo: {cons}
categoria: {cat}
ruta: {ruta}
lenguaje_principal: {lang}
stack: {stack_json}
tags: {tags_json}
fecha_agregado: {fecha}
url_github: {url}
---

# {name}{warn}

> **Resumen:** {desc}

## Categoría
[[categorias/{cat}|{cat}]]

## Stack detectado
- Lenguaje principal: **{lang}**
- Stack: {stack_str}

## Identificadores
- Consecutivo local: **{cons}**
- GitHub ID: **{id_label}**
- Fecha agregado: **{fecha}**
- URL: {url_md}

## Ruta local
`{ruta}`

## Tags
{tags_hash}
",
            name      = title,
            id        = id_str,
            cons      = r.consecutivo,
            cat       = r.categoria,
            ruta      = r.ruta,
            lang      = r.lenguaje_principal,
            stack_json= stack_json,
            tags_json = tags_json,
            fecha     = r.fecha_agregado,
            url       = r.url_github.clone().unwrap_or_default(),
            warn      = warn,
            desc      = if r.descripcion.is_empty() { "(sin descripción)".to_string() } else { r.descripcion.clone() },
            stack_str = stack_str,
            id_label  = match r.id { Some(i) => i.to_string(), None => "—".into() },
            url_md    = url_md,
            tags_hash = r.tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" "),
        );
        fs::write(&page, content)?;
    }

    // Página por categoría
    let mut by_cat: BTreeMap<&str, Vec<&RepoEntry>> = BTreeMap::new();
    for r in data {
        by_cat.entry(r.categoria.as_str()).or_default().push(r);
    }
    for (cat, repos) in &by_cat {
        let page = wiki.join("categorias").join(format!("{}.md", cat));
        let mut lines = vec![
            format!("# {}", cat),
            String::new(),
            format!("Total repos: **{}**", repos.len()),
            String::new(),
            "| # | Repo | Lenguaje | Descripción |".to_string(),
            "|---|---|---|---|".to_string(),
        ];
        let mut sorted = repos.clone();
        sorted.sort_by_key(|r| r.consecutivo);
        for r in sorted {
            let desc = r.descripcion.chars().take(120).collect::<String>();
            lines.push(format!(
                "| {} | [[repos/{}|{}]] | {} | {} |",
                r.consecutivo, slug(&r.name), r.name, r.lenguaje_principal, desc
            ));
        }
        fs::write(&page, lines.join("\n") + "\n")?;
    }

    // Index global
    let mut idx_lines = vec![
        "# Índice de repositorios".to_string(),
        String::new(),
        format!("Total: **{}** repos · Última actualización: {}",
            data.len(), chrono::Local::now().date_naive()),
        String::new(),
        "## Por categoría".to_string(),
        String::new(),
    ];
    for (cat, repos) in &by_cat {
        idx_lines.push(format!("### [[categorias/{0}|{0}]] ({1})", cat, repos.len()));
        let mut sorted = repos.clone();
        sorted.sort_by_key(|r| r.consecutivo);
        for r in sorted {
            let warn = if r.necesita_traduccion { " ⚠️" } else { "" };
            let desc = r.descripcion.chars().take(120).collect::<String>();
            idx_lines.push(format!(
                "- `#{:>3}`  [[repos/{}|{}]] — {}{}",
                r.consecutivo, slug(&r.name), r.name, desc, warn
            ));
        }
        idx_lines.push(String::new());
    }
    let index_md = wiki.join("index.md");
    fs::write(&index_md, idx_lines.join("\n") + "\n")?;

    // Tags
    let mut tag_idx: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for r in data {
        for t in &r.tags {
            tag_idx.entry(t.as_str()).or_default().push(r.name.as_str());
        }
    }
    let mut tag_lines = vec!["# Índice de tags".to_string(), String::new()];
    for (tag, names) in &tag_idx {
        tag_lines.push(format!("## #{} ({})", tag, names.len()));
        let mut sorted_names = names.clone();
        sorted_names.sort();
        for n in sorted_names {
            tag_lines.push(format!("- [[repos/{}|{}]]", slug(n), n));
        }
        tag_lines.push(String::new());
    }
    fs::write(wiki.join("tags.md"), tag_lines.join("\n") + "\n")?;

    Ok(index_md)
}
