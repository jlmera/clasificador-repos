//! Descubridor de candidatos a categoría a partir de los topics de GitHub.
//!
//! ## Idea
//!
//! El usuario tiene N repos cacheados en `repo_ids.json` con sus topics
//! oficiales (campo `topics` de la GitHub API). Algunos topics aparecen
//! en muchos repos y NO están cubiertos por las categorías actuales —
//! esos son candidatos naturales a convertirse en una categoría nueva.
//!
//! ## Flujo
//!
//! 1. Contar todos los topics del cache.
//! 2. Filtrar:
//!    - Los YA cubiertos por algún `topic_boosts` del config.
//!    - (Opcional) lenguajes — ya viven como `lang/X` tags.
//!    - (Opcional) stacks técnicos — ya viven como `stack/X` tags.
//!    - (Opcional) topics genéricos sin valor categorial.
//! 3. Quedarse con los que tienen `>= threshold` apariciones.
//! 4. Detectar sinónimos (plural ↔ singular, sufijo `-cli`) y fusionarlos
//!    en un solo candidato sumando los repos de ambos.
//! 5. Devolver lista ordenada por count descendente.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::categories_config::CategoriasConfig;
use crate::ids::IdsCache;

/// Lenguajes de programación comunes que NO deberían ser categoría:
/// ya viven en el repo como tag `lang/python`, `lang/rust`, etc.
const LANGUAGES: &[&str] = &[
    "python", "rust", "go", "golang", "javascript", "typescript",
    "java", "kotlin", "swift", "ruby", "php", "c", "cpp", "c++",
    "csharp", "c#", "scala", "haskell", "elixir", "erlang", "clojure",
    "perl", "lua", "dart", "r", "julia", "matlab", "shell", "bash",
    "powershell", "html", "css", "sass", "scss", "vue", "svelte",
    "ocaml", "fsharp", "f#", "nim", "zig", "crystal", "v",
];

/// Stacks técnicos / dependencias que NO deberían ser categoría:
/// ya viven como tag `stack/X` o son tan ubicuos que no segmentan
/// nada útil (un repo puede usar Docker para distribuirse sin que
/// "Docker" sea su tema central).
const STACKS: &[&str] = &[
    "docker", "kubernetes", "k8s", "podman", "compose",
    "react", "nextjs", "next-js", "vue", "vuejs", "angular", "svelte",
    "nodejs", "node", "deno", "bun", "express",
    "postgresql", "postgres", "mysql", "mariadb", "sqlite", "mongodb",
    "redis", "memcached",
    "fastapi", "flask", "django", "rails", "laravel", "spring",
    "tailwind", "tailwindcss", "bootstrap",
    "webpack", "vite", "rollup",
];

/// Topics tan genéricos que no aportan como categoría: o aplican a
/// casi todo (`ai`, `open-source`) o son etiquetas de evento/meta
/// (`hacktoberfest`) o adjetivos amorfos (`framework`, `automation`).
const GENERIC_TOPICS: &[&str] = &[
    "ai", "artificial-intelligence",
    "open-source", "opensource", "free-software",
    "hacktoberfest",
    "framework", "library", "tool", "tools", "utility", "utilities",
    "automation", "scripting",
    "awesome", "awesome-list",
    "github", "gitlab",
    "hello-world", "tutorial", "tutorials", "demo", "example",
    "boilerplate", "starter", "template", "templates",
    "project", "projects", "showcase",
    "english",
    "markdown", // 99% de los READMEs son markdown — etiqueta meta sin valor
];

/// Un candidato a categoría nueva.
#[derive(Debug, Clone)]
pub struct TopicCandidate {
    /// Topic principal (canónico) que va a dar nombre a la categoría.
    pub topic: String,
    /// Sinónimos detectados que se fusionaron en este candidato
    /// (ej. si `topic = "agents"`, podría tener `["agent"]` aquí).
    /// Vacío si no se fusionó nada.
    pub merged: Vec<String>,
    /// Total de repos que tienen `topic` O alguno de los `merged`.
    pub count: usize,
    /// Nombres de los repos. Sin duplicados aunque un repo tenga
    /// varios sinónimos. Ordenados alfabéticamente.
    pub repos: Vec<String>,
    /// ID propuesto para la categoría: `NN-{topic}` con NN auto.
    /// El usuario puede editarlo en la UI antes de crear.
    pub id_propuesto: String,
    /// Marca de selección en la UI. Default false (más conservador
    /// que reclasificar de movida).
    pub selected: bool,
    /// Predicción de cuántos repos REALMENTE caerían en esta categoría
    /// si se aplicara, calculada simulando classify_heuristic con un
    /// cfg provisional. `None` = aún no se simuló.
    /// Esta predicción puede ser MENOR que `count` porque otros buckets
    /// pueden ganar el score (ej. un repo con topic "chatgpt" pero
    /// también "claude-code" + "anthropic" puntúa más alto en
    /// 01-claude-code y se queda allí).
    pub predicted: Option<usize>,
}

/// Filtros aplicables al descubrimiento.
#[derive(Debug, Clone, Copy)]
pub struct DiscoverFilters {
    pub threshold: usize,
    pub hide_languages: bool,
    pub hide_stacks:    bool,
    pub hide_generic:   bool,
}

impl Default for DiscoverFilters {
    fn default() -> Self {
        Self {
            threshold:      3,
            hide_languages: true,
            hide_stacks:    true,
            hide_generic:   true,
        }
    }
}

/// Devuelve los topics ya cubiertos por algún `topic_boost` del config.
/// En lowercase, listos para comparar contra los topics de la API
/// (que también vienen en lowercase desde `fetch_github_meta`).
pub fn topics_covered(cfg: &CategoriasConfig) -> HashSet<String> {
    let mut s = HashSet::new();
    for cat in &cfg.categorias {
        for (t, _w) in &cat.topic_boosts {
            s.insert(t.to_lowercase());
        }
    }
    s
}

/// Núcleo: escanea repo_ids y devuelve la lista de candidatos ordenada
/// por count desc.
///
/// `next_consecutivo` es el próximo número correlativo a usar para
/// generar `id_propuesto = "NN-{topic}"`. Se incrementa por cada
/// candidato. Típicamente el caller pasa `cfg.categorias.len() + 1`
/// para que los nuevos vengan después de los existentes.
pub fn discover_candidates(
    repo_ids:        &IdsCache,
    cubiertos:       &HashSet<String>,
    filters:         DiscoverFilters,
    next_consecutivo: usize,
) -> Vec<TopicCandidate> {
    // 1) Contar topics + memorizar repos.
    //    BTreeMap para iteración determinística (mismo orden frame a frame).
    let mut counts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (repo_name, meta) in repo_ids {
        for raw in &meta.topics {
            let t = raw.to_lowercase().trim().to_string();
            if t.is_empty() { continue; }
            counts.entry(t).or_default().insert(repo_name.clone());
        }
    }

    // 2) Filtrar por umbral + cobertura + listas negras.
    let langs:   HashSet<&'static str> = LANGUAGES.iter().copied().collect();
    let stacks:  HashSet<&'static str> = STACKS.iter().copied().collect();
    let generic: HashSet<&'static str> = GENERIC_TOPICS.iter().copied().collect();

    let candidate_topics: Vec<(String, BTreeSet<String>)> = counts.into_iter()
        .filter(|(t, repos)| {
            if repos.len() < filters.threshold       { return false; }
            if cubiertos.contains(t)                 { return false; }
            if filters.hide_languages && langs.contains(t.as_str())   { return false; }
            if filters.hide_stacks    && stacks.contains(t.as_str())  { return false; }
            if filters.hide_generic   && generic.contains(t.as_str()) { return false; }
            true
        })
        .collect();

    // 3) Detectar sinónimos y fusionar.
    //    Construimos un set de topics ya consumidos (incluidos como sinónimo
    //    de otro) para no procesarlos dos veces.
    let topic_names: BTreeSet<String> = candidate_topics.iter()
        .map(|(t, _)| t.clone())
        .collect();

    let mut consumed: HashSet<String> = HashSet::new();
    let mut candidates: Vec<TopicCandidate> = Vec::new();
    let mut seq = next_consecutivo;

    // Recorremos de mayor a menor count → el "principal" siempre es el
    // más popular del grupo de sinónimos (más estable a futuro).
    let mut sorted = candidate_topics.clone();
    sorted.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));

    for (topic, repos) in sorted {
        if consumed.contains(&topic) { continue; }

        let synonyms = find_synonyms(&topic, &topic_names);
        let mut merged_names: Vec<String> = Vec::new();
        let mut all_repos: BTreeSet<String> = repos.clone();
        for syn in &synonyms {
            if syn == &topic { continue; }
            if let Some((_, syn_repos)) = candidate_topics.iter().find(|(t, _)| t == syn) {
                merged_names.push(syn.clone());
                all_repos.extend(syn_repos.iter().cloned());
                consumed.insert(syn.clone());
            }
        }
        consumed.insert(topic.clone());

        let id_propuesto = format!("{:02}-{}", seq, sanitize_id(&topic));
        seq += 1;

        candidates.push(TopicCandidate {
            topic:        topic.clone(),
            merged:       merged_names,
            count:        all_repos.len(),
            repos:        all_repos.into_iter().collect(),
            id_propuesto,
            selected:     false,
            predicted:    None,
        });
    }

    // Re-sortear por count desc (la fusión pudo cambiar el ranking).
    candidates.sort_by(|a, b| b.count.cmp(&a.count).then(a.topic.cmp(&b.topic)));
    candidates
}

/// Dado un topic y el universo de candidatos, devuelve los que son
/// equivalentes triviales:
///   - plural ↔ singular: `agent` ↔ `agents`, `skill` ↔ `skills`
///                        `class` ↔ `classes`, `box` ↔ `boxes`
///   - sufijo `-cli`: `gemini` ↔ `gemini-cli`
///
/// La lista DEVUELTA incluye `topic` mismo. Si no hay sinónimos en el
/// universo, devuelve `[topic]` (length 1).
pub fn find_synonyms(topic: &str, universe: &BTreeSet<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![topic.to_string()];

    // Generar variantes potenciales y testear si están en el universo.
    let mut variants: Vec<String> = Vec::new();

    // Plural ↔ singular (heurística simple, suficiente en ~95% de los casos).
    if let Some(stem) = topic.strip_suffix("ies") {
        variants.push(format!("{}y", stem));         // agencies → agency
    }
    if let Some(stem) = topic.strip_suffix("es") {
        variants.push(stem.to_string());             // boxes → box, classes → class
    }
    if let Some(stem) = topic.strip_suffix('s') {
        variants.push(stem.to_string());             // agents → agent
    }
    if !topic.ends_with('s') && !topic.ends_with("ss") {
        variants.push(format!("{}s", topic));        // agent → agents
    }
    if let Some(stem) = topic.strip_suffix('y') {
        variants.push(format!("{}ies", stem));       // agency → agencies
    }

    // Sufijo -cli.
    if let Some(stem) = topic.strip_suffix("-cli") {
        variants.push(stem.to_string());             // gemini-cli → gemini
    } else {
        variants.push(format!("{}-cli", topic));     // gemini → gemini-cli
    }

    for v in variants {
        if v == topic { continue; }
        if universe.contains(&v) && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// Limpia un topic para usarlo como segmento de id de carpeta.
/// - Lowercase
/// - Espacios y `_` → `-`
/// - Quita caracteres no alfanuméricos / no `-`
/// - Colapsa `--` a `-`
fn sanitize_id(topic: &str) -> String {
    let mut s = String::with_capacity(topic.len());
    let mut prev_dash = false;
    for ch in topic.chars() {
        let c = ch.to_ascii_lowercase();
        let mapped = match c {
            'a'..='z' | '0'..='9' => c,
            ' ' | '_' | '-' | '.' => '-',
            _ => continue,
        };
        if mapped == '-' && prev_dash { continue; }
        s.push(mapped);
        prev_dash = mapped == '-';
    }
    // Trim leading/trailing dashes.
    s.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synonym_plural_basic() {
        let universe: BTreeSet<String> = ["agent", "agents", "skill", "skills"]
            .iter().map(|s| s.to_string()).collect();
        let mut found = find_synonyms("agents", &universe);
        found.sort();
        assert_eq!(found, vec!["agent".to_string(), "agents".to_string()]);
    }

    #[test]
    fn synonym_cli_suffix() {
        let universe: BTreeSet<String> = ["gemini", "gemini-cli"]
            .iter().map(|s| s.to_string()).collect();
        let mut found = find_synonyms("gemini", &universe);
        found.sort();
        assert_eq!(found, vec!["gemini".to_string(), "gemini-cli".to_string()]);
    }

    #[test]
    fn sanitize_basic() {
        assert_eq!(sanitize_id("Claude Code"), "claude-code");
        assert_eq!(sanitize_id("AI__Coding"),  "ai-coding");
        assert_eq!(sanitize_id("--weird--"),   "weird");
    }
}
