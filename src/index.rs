//! rebuild_index — recorre las categorías, escanea cada repo y construye
//! la lista enriquecida que se serializa a `tools/repos_index.json`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::atomic_io::write_atomic_string;
use crate::categories::TAG_KEYWORDS;
use crate::categories_config::load_or_default as load_cats_cfg;
use crate::ids::{
    assign_repo_metadata_with_stats, load_repo_ids, save_repo_ids, GhFetchStats,
};
use crate::paths::{descripciones_path, index_json_path, repo_ids_path};
use crate::readme::render_readme_html;
use crate::scan::scan_repo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub id: Option<i64>,
    pub consecutivo: u32,
    pub name: String,
    pub categoria: String,
    pub ruta: String,
    pub descripcion: String,
    pub lenguaje_principal: String,
    pub stack: Vec<String>,
    pub tags: Vec<String>,
    pub fecha_agregado: String,
    pub url_github: Option<String>,
    pub readme_html: Option<String>,
    pub readme_es_html: Option<String>,
    pub necesita_traduccion: bool,
}

pub fn load_descripciones_es(root: &Path) -> serde_json::Map<String, serde_json::Value> {
    let p = descripciones_path(root);
    if !p.exists() { return serde_json::Map::new(); }
    let text = match fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => return serde_json::Map::new(),
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn derive_tags(name: &str, desc: &str, lang: &str, stack: &[String], category: &str) -> Vec<String> {
    let mut tags: BTreeSet<String> = BTreeSet::new();
    tags.insert(category.to_string());
    tags.insert(format!("lang/{}", lang.to_lowercase().replace(' ', "-")));
    for s in stack {
        tags.insert(format!("stack/{}", s));
    }
    let blob = format!("{} {}", name, desc).to_lowercase();
    for (k, t) in TAG_KEYWORDS {
        if blob.contains(k) {
            tags.insert((*t).to_string());
        }
    }
    tags.into_iter().collect()
}

/// Opciones de rebuild_index.
#[derive(Debug, Clone, Default)]
pub struct RebuildOpts {
    /// Permite consultar api.github.com para obtener IDs.
    pub allow_github: bool,
    /// Si true, reintenta IDs aunque el cache tenga last_attempt reciente (<24h).
    pub force_github_retry: bool,
    /// PAT opcional para autenticar requests a GitHub API (5000 req/h en
    /// vez de 60). El handler de la GUI lo carga desde el config cifrado.
    pub github_pat: Option<String>,
}

/// Wrapper de compatibilidad: rebuild_index sin stats. Internamente usa
/// rebuild_index_with_stats descartando las métricas. Mantiene la API
/// existente para callers que no necesitan tracking del flow ETag.
pub fn rebuild_index(root: &Path, opts: RebuildOpts) -> Result<Vec<RepoEntry>> {
    let (data, _stats) = rebuild_index_with_stats(root, opts)?;
    Ok(data)
}

/// Variante de rebuild_index que retorna también las estadísticas del flow
/// de ETag conditional GET: cuántos fetched, not_modified, failed, skipped.
/// Útil para el botón "Refrescar GitHub IDs" que quiere mostrar al usuario
/// el ahorro de requests gracias al cache de etags.
pub fn rebuild_index_with_stats(
    root: &Path,
    opts: RebuildOpts,
) -> Result<(Vec<RepoEntry>, GhFetchStats)> {
    let descripciones_es = load_descripciones_es(root);
    let cache_path = repo_ids_path(root);
    let mut ids_cache = load_repo_ids(&cache_path);

    let cur_max = ids_cache.values().map(|m| m.consecutivo).max().unwrap_or(0);
    let mut next_local = cur_max + 1;

    let mut out: Vec<RepoEntry> = Vec::new();
    let mut stats = GhFetchStats::default();

    // Cargar la lista de categorías desde el config (data/categorias.json),
    // con fallback a los hardcoded si no existe el archivo.
    let cats_cfg = load_cats_cfg(root);

    for cat in &cats_cfg.categorias {
        let cat = cat.id.as_str();
        let cat_dir = root.join(cat);
        if !cat_dir.is_dir() { continue; }
        let mut repos: Vec<_> = match fs::read_dir(&cat_dir) {
            Ok(rd) => rd.flatten()
                        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                        .collect(),
            Err(_) => continue,
        };
        repos.sort_by_key(|e| e.file_name());

        for entry in repos {
            let repo_path = entry.path();
            let scan = match scan_repo(&repo_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let desc = descripciones_es.get(&scan.name)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| scan.descripcion_en.clone());

            let meta = assign_repo_metadata_with_stats(
                &repo_path, &scan.name, &mut ids_cache, &mut next_local,
                opts.allow_github, opts.force_github_retry,
                opts.github_pat.as_deref(),
                &mut stats,
            );

            let tags = derive_tags(
                &scan.name, &desc, &scan.lenguaje_principal, &scan.stack, cat,
            );

            // Render del README en idioma original (incremental)
            let readme_html = render_readme_html(&repo_path, cat, root, false)
                .map(|p| p.to_string_lossy().into_owned());

            // README ES: solo si la caché _README_es.md ya existe (no llama al LLM)
            let readme_es_html = readme_es_from_cache(&repo_path);

            out.push(RepoEntry {
                id: meta.id,
                consecutivo: meta.consecutivo,
                name: scan.name.clone(),
                categoria: cat.to_string(),
                ruta: scan.path.clone(),
                descripcion: desc,
                lenguaje_principal: scan.lenguaje_principal.clone(),
                stack: scan.stack.clone(),
                tags,
                fecha_agregado: meta.fecha_agregado.clone(),
                url_github: meta.url_github.clone(),
                readme_html,
                readme_es_html,
                necesita_traduccion: !descripciones_es.contains_key(&scan.name),
            });
        }
    }

    save_repo_ids(&cache_path, &ids_cache)?;
    out.sort_by_key(|r| r.consecutivo);

    let json_out = index_json_path(root);
    let s = serde_json::to_string_pretty(&out)?;
    // Escritura ATÓMICA: write-to-tmp + fsync + rename. Igual que repo_ids.json,
    // este archivo es leído por buscador.html y otros consumidores; una
    // corrupción rompe el flujo entero hasta el siguiente reindex.
    write_atomic_string(&json_out, &s)?;
    Ok((out, stats))
}

/// Si existe `_README_es.html` o se puede generar desde `_README_es.md`,
/// devuelve la ruta. Caso contrario None (no traduce — eso es del botón LLM).
fn readme_es_from_cache(repo: &Path) -> Option<String> {
    let html = repo.join("_README_es.html");
    if html.exists() {
        return Some(html.to_string_lossy().into_owned());
    }
    None
}
