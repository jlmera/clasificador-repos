//! Reclasificación masiva: aplica el algoritmo `classify_heuristic` a TODOS
//! los repos ya categorizados y devuelve los cambios propuestos.
//!
//! Se usa cuando el usuario edita `data/categorias.json` y quiere que los
//! repos viejos se reacomoden según las nuevas reglas, no solo los nuevos
//! del `_inbox`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::categories_config::{load_or_default as load_cats_cfg, CategoriasConfig};
use crate::classify::{classify_heuristic, GhClassifyHints};
use crate::ids::{load_repo_ids, RepoMeta};
use crate::paths::repo_ids_path;
use crate::scan::scan_repo;

/// Una propuesta de cambio de categoría para un repo individual.
#[derive(Debug, Clone)]
pub struct ReclassifyChange {
    pub name:          String,
    pub current_cat:   String,
    pub proposed_cat:  String,
    pub current_path:  PathBuf,
    pub proposed_path: PathBuf,
    pub confidence:    f64,
    /// true si el usuario quiere que se aplique este cambio. Default true.
    pub selected:      bool,
}

/// Recorre todas las carpetas de categorías existentes en `root`, ejecuta
/// `classify_heuristic` sobre cada repo y devuelve las propuestas donde
/// el resultado difiere de la carpeta actual del repo.
///
/// Los hints de GitHub (`description_en` + `topics`) se leen del cache
/// `repo_ids.json` — no se hace ninguna llamada de red. Si querés que la
/// reclasificación tenga datos frescos de GitHub, ejecutá primero
/// "🆔 Refrescar GitHub IDs".
pub fn compute_reclassification(root: &Path) -> Result<Vec<ReclassifyChange>> {
    let cats_cfg = load_cats_cfg(root);
    compute_reclassification_with_cfg(root, &cats_cfg)
}

/// Igual a `compute_reclassification` pero acepta el config como parámetro.
/// Útil para SIMULAR el efecto de un config provisional (con categorías
/// añadidas o modificadas en memoria) sin escribir nada a disco. Lo usa
/// el descubridor para predecir cuántos repos caerían en cada categoría
/// candidata antes de crearla.
pub fn compute_reclassification_with_cfg(
    root: &Path,
    cats_cfg: &CategoriasConfig,
) -> Result<Vec<ReclassifyChange>> {
    let ids_cache = load_repo_ids(&repo_ids_path(root));
    let mut changes = Vec::new();

    for cat_def in &cats_cfg.categorias {
        let current_cat_id = cat_def.id.as_str();
        let cat_dir = root.join(current_cat_id);
        if !cat_dir.is_dir() { continue; }

        let entries: Vec<_> = match fs::read_dir(&cat_dir) {
            Ok(rd) => rd.flatten()
                        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                        .collect(),
            Err(_) => continue,
        };

        for entry in entries {
            let repo_path = entry.path();
            let scan = match scan_repo(&repo_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Hints de GitHub desde cache (si los hay).
            let hints = build_hints_from_cache(ids_cache.get(&scan.name));
            let cls = classify_heuristic(&scan, Some(&hints), cats_cfg);

            if cls.categoria != current_cat_id {
                let proposed_path = root.join(&cls.categoria).join(&scan.name);
                changes.push(ReclassifyChange {
                    name:          scan.name.clone(),
                    current_cat:   current_cat_id.to_string(),
                    proposed_cat:  cls.categoria.clone(),
                    current_path:  repo_path.clone(),
                    proposed_path,
                    confidence:    cls.confidence,
                    selected:      true,
                });
            }
        }
    }

    // Orden: por categoría destino y dentro por confianza desc.
    changes.sort_by(|a, b| {
        a.proposed_cat.cmp(&b.proposed_cat)
            .then(b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
    });
    Ok(changes)
}

/// Helper para construir GhClassifyHints a partir de un RepoMeta opcional.
/// Wraps the borrowed strings/slice with proper lifetimes.
fn build_hints_from_cache(cached: Option<&RepoMeta>) -> GhClassifyHints<'_> {
    GhClassifyHints {
        description_en: cached.and_then(|c| c.description_en.as_deref()),
        topics:         cached.map(|c| c.topics.as_slice()).unwrap_or(&[]),
    }
}

/// Aplica los cambios cuyo `selected = true`. Crea la carpeta destino si
/// no existe (cuando se agregaron categorías nuevas al config). Devuelve
/// (movidos_ok, errores).
pub fn apply_reclassification(changes: &[ReclassifyChange]) -> Result<(usize, usize)> {
    let mut ok = 0usize;
    let mut errs = 0usize;
    for change in changes {
        if !change.selected { continue; }
        // Crear carpeta destino (la categoría puede no existir aún).
        if let Some(parent) = change.proposed_path.parent() {
            if let Err(_) = fs::create_dir_all(parent) {
                errs += 1;
                continue;
            }
        }
        // Si el destino ya existe (raro pero posible si hay un repo con
        // mismo nombre en otra categoría), saltamos para no sobreescribir.
        if change.proposed_path.exists() {
            errs += 1;
            continue;
        }
        match fs::rename(&change.current_path, &change.proposed_path) {
            Ok(_)  => ok += 1,
            Err(_) => errs += 1,
        }
    }
    Ok((ok, errs))
}
