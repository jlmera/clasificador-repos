//! IDs de GitHub + consecutivos locales + fechas.
//! Equivalente a `assign_repo_metadata`, `fetch_github_id`, etc.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::atomic_io::write_atomic_string;
use crate::duplicates::get_git_remote_url;
use crate::scan::read_text_safe;

// Regex compiladas una sola vez. Antes vivían en línea adentro de
// `parse_github_owner_repo` y `get_repo_creation_date`, recompilando en
// cada call → 314 compilaciones inútiles por reindex (157 repos × 2).
static RE_GH_URL:    Lazy<Regex> = Lazy::new(||
    Regex::new(r"^github\.com[/:]([^/]+)/([^/]+)$").unwrap()
);
static RE_GIT_LOG_TS: Lazy<Regex> = Lazy::new(||
    Regex::new(r"\s(\d{10})\s[\+\-]\d{4}\s").unwrap()
);

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RepoMeta {
    pub id: Option<i64>,            // GitHub repo id, o None
    pub consecutivo: u32,
    pub fecha_agregado: String,     // ISO date YYYY-MM-DD
    pub url_github: Option<String>,
    /// Descripción oficial del repo según GitHub API (campo `description`
    /// del JSON `/repos/{owner}/{repo}`). Una línea, idioma original.
    /// Fuente canónica para traducir a `descripciones_es.json`.
    /// `#[serde(default)]` mantiene compat con `repo_ids.json` viejos.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_en: Option<String>,
    /// Topics oficiales del repo (campo `topics` de la API GitHub).
    /// Lista de strings tipo "self-hosted", "mcp-server", "rag".
    /// Se usan como señal fuerte en `classify_heuristic` mediante
    /// el mapping `TOPIC_BOOSTS`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<String>,
    /// Último intento de obtener el ID de GitHub (RFC3339).
    /// Si está poblado y no ha pasado 24h, no se reintenta automáticamente.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt: Option<String>,
    /// ETag del último response 200 OK de la API. Se manda en el header
    /// `If-None-Match` de la siguiente request; si GitHub considera que
    /// el repo no cambió, responde 304 Not Modified — y según docs de
    /// GitHub, ESE 304 NO consume cuota del rate-limit. Permite hacer
    /// refresh del cache sin "gastar" requests cuando los repos están
    /// estables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

/// Resultado de un fetch a la API de GitHub: id + descripción + topics + etag.
#[derive(Debug, Clone, Default)]
pub struct GhMeta {
    pub id: i64,
    pub description: Option<String>,
    pub topics: Vec<String>,
    /// ETag del response actual (header `etag`). Se persiste en RepoMeta
    /// para usarlo en futuros If-None-Match.
    pub etag: Option<String>,
}

/// Posibles resultados de `fetch_github_meta`. Distingue entre el caso
/// "200 OK con datos nuevos" y "304 Not Modified" (el segundo no implica
/// error: simplemente confirma que el cache sigue válido).
#[derive(Debug, Clone)]
pub enum FetchOutcome {
    /// 200 OK: GitHub devolvió metadata nueva. Reemplaza el cache.
    Updated(GhMeta),
    /// 304 Not Modified: el etag previo coincide; el repo no cambió.
    /// Mantener el cache tal cual está. Sin consumo de rate-limit.
    NotModified,
    /// Error de red, 404, 5xx, etc. No se sabe si cambió o no.
    Failed,
}

/// Estadísticas acumuladas de un batch de fetch_github_meta.
/// Útil para el log del flujo "Refrescar GitHub IDs" para mostrar
/// cuántos repos fueron consultados y cuántos no requerían refresh.
#[derive(Debug, Clone, Default)]
pub struct GhFetchStats {
    /// Repos cuya respuesta fue 200 OK con metadata nueva.
    pub fetched: u32,
    /// Repos donde el etag previo coincidió → 304 Not Modified.
    /// Estos NO cuentan contra el rate-limit de GitHub.
    pub not_modified: u32,
    /// Repos donde el fetch falló (red, 404, repo privado/movido, etc.).
    pub failed: u32,
    /// Repos que no se intentaron (sin red, ya tenían id sin force, etc.).
    pub skipped: u32,
}

pub type IdsCache = BTreeMap<String, RepoMeta>;

pub fn load_repo_ids(path: &Path) -> IdsCache {
    if !path.exists() { return IdsCache::new(); }
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return IdsCache::new(),
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_repo_ids(path: &Path, cache: &IdsCache) -> Result<()> {
    let s = serde_json::to_string_pretty(cache)?;
    // Escritura ATÓMICA (write-to-tmp + fsync + rename) para evitar
    // corrupción cuando Syncthing/antivirus/otra instancia accede al
    // archivo a mitad de escritura. Crítico para repo_ids.json:
    // las pérdidas son silenciosas (load_repo_ids retorna mapa vacío
    // ante parse error) y borran los consecutivos del usuario.
    write_atomic_string(path, &s)?;
    Ok(())
}

/// Parsea una URL git (lowercase, sin protocolo, sin .git) a (owner, repo).
pub fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let cap = RE_GH_URL.captures(url)?;
    Some((cap.get(1)?.as_str().to_string(), cap.get(2)?.as_str().to_string()))
}

const GITHUB_API: &str = "https://api.github.com/repos";

/// Prueba el PAT contra `/rate_limit` (no requiere scopes especiales).
/// Devuelve un string con el rate-limit (5000/h con PAT, 60/h anonymous).
/// Útil para que el usuario verifique que su token funciona sin gastar
/// llamadas reales contra repos.
pub fn test_github_pat(pat: &str) -> Result<String> {
    let pat = pat.trim();
    if pat.is_empty() {
        return Err(anyhow::anyhow!("PAT vacío"));
    }
    let resp = match ureq::get("https://api.github.com/rate_limit")
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .set("User-Agent", "clasificador/1.0")
        .set("Authorization", &format!("Bearer {}", pat))
        .timeout(std::time::Duration::from_secs(15))
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_else(|_| "<sin body>".into());
            let preview: String = body.chars().take(200).collect();
            return Err(anyhow::anyhow!("HTTP {} · {}", code, preview));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(anyhow::anyhow!("red: {}", t));
        }
    };

    let body: Json = resp.into_json()
        .map_err(|e| anyhow::anyhow!("body no es JSON Anthropic válido: {}", e))?;

    // Estructura de la respuesta:
    //   { "resources": { "core": { "limit": N, "remaining": M, "reset": ... } } }
    let core = body.get("resources").and_then(|r| r.get("core"));
    let limit = core.and_then(|c| c.get("limit"))
        .and_then(|l| l.as_u64()).unwrap_or(0);
    let remaining = core.and_then(|c| c.get("remaining"))
        .and_then(|l| l.as_u64()).unwrap_or(0);

    if limit < 1000 {
        // Si el limit es 60, GitHub está respondiendo como anonymous —
        // el header Authorization no se está aceptando (PAT inválido o
        // mal formateado).
        return Err(anyhow::anyhow!(
            "PAT no aceptado (rate limit anonymous: {}/h). Verifica el token.",
            limit
        ));
    }
    Ok(format!("rate limit {}/h · disponibles ahora: {}", limit, remaining))
}

/// Llama a api.github.com/repos/{owner}/{repo} y devuelve id + descripción + topics + etag.
///
/// Si `pat` es `Some`, se envía en `Authorization: Bearer <pat>`, elevando el
/// rate-limit de 60 → 5000 req/h.
///
/// Si `prev_etag` es `Some`, se envía como `If-None-Match`. GitHub responderá
/// con `304 Not Modified` (sin body) cuando el repo no cambió respecto a la
/// última versión cacheada — y según la docs oficial de GitHub, los responses
/// 304 NO consumen cuota del rate-limit. Esto permite refrescar el cache de
/// 157 repos consumiendo solo los pocos que realmente cambiaron.
pub fn fetch_github_meta(
    owner:     &str,
    repo:      &str,
    pat:       Option<&str>,
    prev_etag: Option<&str>,
) -> FetchOutcome {
    let url = format!("{}/{}/{}", GITHUB_API, owner, repo);
    let mut req = ureq::get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .set("User-Agent", "clasificador/1.0")
        .timeout(std::time::Duration::from_secs(8));
    if let Some(token) = pat {
        let token = token.trim();
        if !token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", token));
        }
    }
    if let Some(et) = prev_etag {
        let et = et.trim();
        if !et.is_empty() {
            // GitHub espera el etag tal cual lo devolvió (incluye comillas y
            // posible prefijo W/). Lo mandamos sin transformar.
            req = req.set("If-None-Match", et);
        }
    }
    // ureq 2.x trata 304 como "Status error" porque no es 2xx. Capturamos
    // ese caso explícitamente y lo mapeamos a NotModified.
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(304, _)) => return FetchOutcome::NotModified,
        Err(_) => return FetchOutcome::Failed,
    };
    if resp.status() == 304 {
        return FetchOutcome::NotModified;
    }
    if resp.status() != 200 {
        return FetchOutcome::Failed;
    }
    // Capturar el etag ANTES de consumir el body (into_json mueve la response).
    let new_etag = resp.header("etag").map(|s| s.to_string());

    let j: Json = match resp.into_json() {
        Ok(j)  => j,
        Err(_) => return FetchOutcome::Failed,
    };
    let id = match j.get("id").and_then(|v| v.as_i64()) {
        Some(i) => i,
        None    => return FetchOutcome::Failed,
    };
    let description = j.get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let topics: Vec<String> = j.get("topics")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter()
            .filter_map(|x| x.as_str())
            .map(|s| s.to_lowercase())
            .collect())
        .unwrap_or_default();
    FetchOutcome::Updated(GhMeta { id, description, topics, etag: new_etag })
}

/// Fecha del primer commit ISO o ctime de la carpeta.
pub fn get_repo_creation_date(repo: &Path) -> String {
    let head = repo.join(".git").join("logs").join("HEAD");
    if head.exists() {
        let txt = read_text_safe(&head, 5000);
        if let Some(first) = txt.lines().next() {
            // Línea: <oldsha> <newsha> <author> <ts> <tz>\t<msg>
            // Regex pre-compilada en RE_GIT_LOG_TS (top del archivo).
            if let Some(cap) = RE_GIT_LOG_TS.captures(first) {
                if let Some(ts_match) = cap.get(1) {
                    if let Ok(ts) = ts_match.as_str().parse::<i64>() {
                        if let Some(d) = chrono::DateTime::from_timestamp(ts, 0) {
                            return d.date_naive().to_string();
                        }
                    }
                }
            }
        }
    }
    // Fallback: fecha de modificación de la carpeta.
    // SystemTime → DateTime<Utc> nunca falla en la práctica, así que usamos From.
    if let Ok(meta) = fs::metadata(repo) {
        if let Ok(modified) = meta.modified() {
            let d: chrono::DateTime<chrono::Utc> = modified.into();
            return d.date_naive().to_string();
        }
    }
    chrono::Local::now().date_naive().to_string()
}

/// ¿Pasaron 24h desde `last_attempt`? Si no, conservar (no machacar la API).
fn should_retry_github(last_attempt: Option<&str>) -> bool {
    let Some(ts) = last_attempt else { return true; };
    let parsed = chrono::DateTime::parse_from_rfc3339(ts);
    let Ok(dt) = parsed else { return true; };
    let elapsed = chrono::Utc::now().signed_duration_since(dt.with_timezone(&chrono::Utc));
    elapsed.num_hours() >= 24
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Wrapper de compatibilidad sin stats. Internamente usa la versión
/// _with_stats con un acumulador descartable. Mantiene la API existente
/// para callers que no necesitan métricas.
pub fn assign_repo_metadata(
    repo: &Path,
    name: &str,
    cache: &mut IdsCache,
    next_local: &mut u32,
    allow_network: bool,
    force_github_retry: bool,
    github_pat: Option<&str>,
) -> RepoMeta {
    let mut sink = GhFetchStats::default();
    assign_repo_metadata_with_stats(
        repo, name, cache, next_local,
        allow_network, force_github_retry, github_pat, &mut sink,
    )
}

/// Asigna o reutiliza id/consecutivo/fecha/url para un repo, con flow de
/// ETag conditional GET y acumulación de estadísticas.
///
/// - `allow_network`: si false, NUNCA llama a GitHub.
/// - `force_github_retry`: si true, ignora `last_attempt` y reintenta sí o sí.
/// - `github_pat`: token opcional para autenticar (5000 req/h).
/// - `stats`: acumulador de fetched/not_modified/failed/skipped.
///
/// Si el cache tiene un `etag` previo, se manda como `If-None-Match`. Si
/// GitHub responde 304, se contabiliza como NotModified y NO se actualiza
/// la metadata (queda como estaba).
pub fn assign_repo_metadata_with_stats(
    repo: &Path,
    name: &str,
    cache: &mut IdsCache,
    next_local: &mut u32,
    allow_network: bool,
    force_github_retry: bool,
    github_pat: Option<&str>,
    stats: &mut GhFetchStats,
) -> RepoMeta {
    if let Some(cached) = cache.get(name).cloned() {
        // Si tiene id Y NO estamos forzando refresh, retornamos como antes.
        // Con `force_github_retry = true` (botón "Refrescar GitHub IDs") sí
        // re-consultamos aunque ya tenga id — así actualizamos topics y
        // description_en para repos viejos sin esos campos.
        if cached.id.is_some() && !force_github_retry {
            stats.skipped += 1;
            return cached;
        }
        // Sin red → retornamos lo que tengamos en cache.
        if !allow_network {
            stats.skipped += 1;
            return cached;
        }
        // Si NO se está forzando y el último intento fue hace <24h → respetar.
        if !force_github_retry
            && cached.id.is_none()
            && !should_retry_github(cached.last_attempt.as_deref())
        {
            stats.skipped += 1;
            return cached;
        }
        // Reintentar GitHub con If-None-Match si tenemos etag previo.
        let url_github = cached.url_github.clone();
        let owner_repo = if let Some(u) = &url_github {
            let stripped = u.strip_prefix("https://").unwrap_or(u);
            parse_github_owner_repo(stripped)
        } else if let Some(raw) = get_git_remote_url(repo) {
            parse_github_owner_repo(&raw)
        } else { None };
        if let Some((owner, slug)) = owner_repo {
            // Pasar el etag previo SOLO si force_github_retry NO está activo.
            // Cuando el usuario pulsa "Refrescar GitHub IDs" probablemente
            // quiere data fresca de verdad; mandar If-None-Match haría que
            // la mayoría devuelva 304 y se vea como "no se hizo nada". Pero
            // ese ES el punto del feature — ahorrar requests. Mantener el
            // etag mejora performance; el usuario verá en el log "X cambiados,
            // Y sin cambios" y entenderá.
            let prev_etag = cached.etag.as_deref();
            match fetch_github_meta(&owner, &slug, github_pat, prev_etag) {
                FetchOutcome::Updated(gh) => {
                    stats.fetched += 1;
                    // Para topics: si la API devuelve una lista vacía,
                    // mantenemos los cacheados (mismo criterio que antes).
                    let new_topics = if gh.topics.is_empty() {
                        cached.topics.clone()
                    } else {
                        gh.topics.clone()
                    };
                    // Si el nuevo response no trajo etag (raro pero posible),
                    // mantenemos el viejo para no perder oportunidad de 304.
                    let new_etag = gh.etag.or(cached.etag.clone());
                    let updated = RepoMeta {
                        id: Some(gh.id),
                        url_github: Some(format!("https://github.com/{}/{}", owner, slug)),
                        description_en: gh.description.or(cached.description_en.clone()),
                        topics: new_topics,
                        last_attempt: None,
                        etag: new_etag,
                        ..cached.clone()
                    };
                    cache.insert(name.to_string(), updated.clone());
                    return updated;
                }
                FetchOutcome::NotModified => {
                    stats.not_modified += 1;
                    // El cache sigue válido. Limpiamos last_attempt para
                    // marcar "verificado, no hay cambios" (no fue un fallo).
                    let updated = RepoMeta {
                        last_attempt: None,
                        ..cached.clone()
                    };
                    cache.insert(name.to_string(), updated.clone());
                    return updated;
                }
                FetchOutcome::Failed => {
                    // Caemos al flujo de fallo de red más abajo.
                }
            }
        }
        // Falló el fetch — guardar timestamp del intento.
        stats.failed += 1;
        let updated = RepoMeta {
            last_attempt: Some(now_rfc3339()),
            ..cached.clone()
        };
        cache.insert(name.to_string(), updated.clone());
        return updated;
    }

    // ── Repo nuevo (no estaba en cache) ──
    let url_normalized = get_git_remote_url(repo);
    let mut url_github = None;
    let mut gh_id = None;
    let mut description_en: Option<String> = None;
    let mut topics: Vec<String> = Vec::new();
    let mut last_attempt = None;
    let mut etag = None;
    if let Some(u) = &url_normalized {
        if let Some((owner, slug)) = parse_github_owner_repo(u) {
            url_github = Some(format!("https://github.com/{}/{}", owner, slug));
            if allow_network {
                // No hay etag previo (es nuevo) → fetch normal con prev_etag = None.
                match fetch_github_meta(&owner, &slug, github_pat, None) {
                    FetchOutcome::Updated(gh) => {
                        stats.fetched += 1;
                        gh_id = Some(gh.id);
                        description_en = gh.description;
                        topics = gh.topics;
                        etag = gh.etag;
                    }
                    FetchOutcome::NotModified => {
                        // Imposible para un repo nuevo (sin etag previo) pero
                        // por completitud, contar como skipped.
                        stats.skipped += 1;
                    }
                    FetchOutcome::Failed => {
                        stats.failed += 1;
                        last_attempt = Some(now_rfc3339());
                    }
                }
            } else {
                stats.skipped += 1;
            }
        } else {
            stats.skipped += 1;
        }
    } else {
        stats.skipped += 1;
    }

    let info = RepoMeta {
        id: gh_id,
        consecutivo: *next_local,
        fecha_agregado: get_repo_creation_date(repo),
        url_github,
        description_en,
        topics,
        last_attempt,
        etag,
    };
    *next_local += 1;
    cache.insert(name.to_string(), info.clone());
    info
}
