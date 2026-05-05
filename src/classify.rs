//! Clasificación heurística por keywords con pesos.
//! Equivalente a `classify_heuristic()` Python.

use std::collections::HashMap;

use crate::categories_config::CategoriasConfig;
use crate::scan::RepoScan;

#[derive(Debug, Clone)]
pub struct Classification {
    pub categoria: String,
    pub confidence: f64,        // normalizada 0..1
    pub scores: HashMap<String, u64>,
}

/// Datos opcionales de GitHub API para refinar la clasificación.
/// Se obtienen del cache `repo_ids.json` cuando hay PAT configurado.
#[derive(Debug, Default, Clone)]
pub struct GhClassifyHints<'a> {
    /// Descripción oficial del repo (campo `description` de GitHub API).
    /// Si está, se prefiere sobre `scan.descripcion_en` para el blob de
    /// matching de keywords (es más limpia y precisa).
    pub description_en: Option<&'a str>,
    /// Topics oficiales del repo (campo `topics` de GitHub API).
    /// Cada topic se compara case-insensitive contra TOPIC_BOOSTS y
    /// suma su peso a la categoría correspondiente.
    pub topics: &'a [String],
}

/// Devuelve (categoría, confianza, scores).
/// Lógica:
///   - score(cat) = Σ peso×ocurrencias de cfg.keywords[cat] sobre blob
///   - + boosts por stack detectado (docker, rust, go) sobre cats hardcoded
///     a IDs `05-…`, `09-…`, `10-…`, `06-…` (estos NO entran al config
///     porque son consecuencia del stack, no del nombre/descripción)
///   - + boosts por cfg.topic_boosts[cat] si hay topics de GitHub
///   - confianza = (top/total) × (0.5 + 0.5 × margin)
///       margin = (top - second) / max(top, 1)
pub fn classify_heuristic(
    scan: &RepoScan,
    gh: Option<&GhClassifyHints>,
    cfg: &CategoriasConfig,
) -> Classification {
    // Blob de texto: nombre + descripción + primeros 1500 chars del README.
    // Si tenemos description_en de GitHub, va al frente del blob (mayor
    // probabilidad de matchear primero). El descripcion_en del scan se
    // mantiene también: ambas suman.
    let gh_desc_lower = gh
        .and_then(|g| g.description_en)
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let blob_lower = format!(
        "{} {} {} {}",
        gh_desc_lower,
        scan.name.to_lowercase(),
        scan.descripcion_en.to_lowercase(),
        truncated_lower(&scan.readme_full, 1500)
    );

    // Inicializar scores en 0 para todas las categorías del config.
    let mut scores: HashMap<String, u64> = cfg.categorias.iter()
        .map(|c| (c.id.clone(), 0u64))
        .collect();

    // 1) Score por keywords del config.
    for cat in &cfg.categorias {
        let mut total = 0u64;
        for (kw, weight) in &cat.keywords {
            let n = count_substring(&blob_lower, kw) as u64;
            if n > 0 {
                total += n * (*weight as u64);
            }
        }
        scores.insert(cat.id.clone(), total);
    }

    // 2) Boost por stack (manifest files detectados en scan.stack).
    // Los IDs aquí están hardcoded por convención: si el usuario renombra
    // estas categorías en el config, el boost no se aplica (no rompe nada,
    // solo deja de ayudar a esos casos).
    if scan.stack.iter().any(|s| s == "docker") {
        *scores.entry("05-self-hosted".to_string()).or_insert(0) += 1;
    }
    if scan.stack.iter().any(|s| s == "rust") {
        *scores.entry("09-sistema-windows-linux".to_string()).or_insert(0) += 1;
        *scores.entry("10-utilidades-dev".to_string()).or_insert(0) += 1;
    }
    if scan.stack.iter().any(|s| s == "go") {
        *scores.entry("06-infraestructura-core".to_string()).or_insert(0) += 1;
    }

    // 3) Boost por GitHub topics. Match exacto contra cada (topic, peso)
    // del config. Topics ya vienen lowercase desde fetch_github_meta.
    if let Some(g) = gh {
        for topic in g.topics {
            for cat in &cfg.categorias {
                for (t_match, weight) in &cat.topic_boosts {
                    if topic == t_match {
                        *scores.entry(cat.id.clone()).or_insert(0) += *weight as u64;
                    }
                }
            }
        }
    }

    let total: u64 = scores.values().sum();
    if total == 0 {
        // Fallback: la última categoría del config (típicamente "10-utilidades-dev").
        // Si el usuario reordenó, se respeta su orden.
        return Classification {
            categoria: cfg.fallback_id(),
            confidence: 0.0,
            scores,
        };
    }

    // Encontrar top y second
    let mut sorted: Vec<(&String, &u64)> = scores.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    let best_cat = sorted[0].0.clone();
    let top = *sorted[0].1;
    let second = sorted.get(1).map(|t| *t.1).unwrap_or(0);

    let raw = top as f64 / total as f64;
    let margin = (top.saturating_sub(second)) as f64 / (top.max(1) as f64);
    let confidence = (raw * (0.5 + 0.5 * margin) * 1000.0).round() / 1000.0;

    Classification {
        categoria: best_cat,
        confidence,
        scores,
    }
}

fn count_substring(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() { return 0; }
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        count += 1;
        start += pos + needle.len();
    }
    count
}

fn truncated_lower(s: &str, max: usize) -> String {
    let truncated: String = s.chars().take(max).collect();
    truncated.to_lowercase()
}
