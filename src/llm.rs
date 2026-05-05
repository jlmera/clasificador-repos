//! Llamadas a Anthropic Claude API: traducción de README al español.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::scan::read_text_safe;

// Regex multilínea: captura cualquier tag HTML <…> aunque sus atributos
// estén distribuidos en varias líneas. La flag (?s) hace que "." matchee
// también newlines. Resuelve casos tipo:
//     <picture>
//       <source
//         media="..."
//         srcset="..."
//       >
//       <img
//         src="..."
//         alt="..."
//       >
//     </picture>
// Antes, el filtro "línea empieza con '<'" descartaba <picture>, <source y
// <img/> pero las líneas con atributos sueltos se filtraban por el blob.
static RE_HTML_MULTILINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<[^>]*>").unwrap()
});

// Patrones de atributos HTML sueltos que sobrevivirían al strip multilínea
// si la tag estaba mal cerrada en el .md (raro pero pasa).
static RE_ATTR_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)^\s*(src|alt|href|width|height|media|srcset|style|class|id|target|rel|loading|fetchpriority|crossorigin|sizes|type|name)\s*=\s*"#
    ).unwrap()
});

/// Extrae una descripción corta (≤240 chars) en español a partir del
/// `_README_es.md` ya traducido. Salta headers, badges, imágenes,
/// tablas de idiomas, líneas dominadas por HTML/markdown decorativo
/// y otras líneas no narrativas. Devuelve `None` si no encuentra
/// contenido útil.
///
/// Es la fuente para `data/descripciones_es.json` cuando se procesan
/// repos vía "Traducir READMEs". Reusa la traducción ya pagada — no
/// consume tokens.
pub fn extract_short_desc_from_md(md_path: &Path) -> Option<String> {
    if !md_path.exists() { return None; }
    let raw = read_text_safe(md_path, 8000);

    // PASO 1 — Strip global de HTML multilínea ANTES de iterar líneas.
    let stripped = RE_HTML_MULTILINE.replace_all(&raw, " ").to_string();

    // PASO 2 — Decodificar entidades HTML básicas. README de proyectos web
    // suele tener &nbsp; (no-break-space) para indentar visualmente, &amp;
    // donde se quería un &, etc. Sin esto el blob final se ve "&nbsp;&nbsp;…".
    let txt = stripped
        .replace("&nbsp;",  " ")
        .replace("&amp;",   "&")
        .replace("&lt;",    "<")
        .replace("&gt;",    ">")
        .replace("&quot;",  "\"")
        .replace("&#39;",   "'")
        .replace("&apos;",  "'");

    let mut useful: Vec<String> = Vec::new();
    let mut total_chars = 0usize;

    for line in txt.lines() {
        let mut ls = line.trim();
        if ls.is_empty() { continue; }

        // PASO 3 — Despojar marcador de blockquote markdown al inicio.
        // Una línea como "> 🧪 NUEVO: …" sí tiene contenido narrativo,
        // solo lleva el ">" decorativo. La conservamos sin él.
        if let Some(rest) = ls.strip_prefix("> ") {
            ls = rest.trim();
        } else if let Some(rest) = ls.strip_prefix('>') {
            // ">..." sin espacio: solo despojamos si lo siguiente es alfanum
            // (sino es cierre de tag suelto o similar — descartamos abajo).
            if rest.chars().next().map_or(false, |c| c.is_alphanumeric()) {
                ls = rest.trim();
            }
        }
        if ls.is_empty() { continue; }

        // ── FILTROS DE DESCARTE ─────────────────────────────────────
        if ls.starts_with('#') { continue; }                  // header
        if ls.starts_with('<') { continue; }                  // tag HTML residual
        if ls.starts_with("![") { continue; }                 // imagen markdown
        if ls.starts_with("[!") { continue; }                 // linked-badge: [![…](…)](…)
        if ls.starts_with("```") { continue; }                // bloque código
        if ls.starts_with('|') { continue; }                  // fila de tabla markdown
        // Línea decorativa pura
        if ls.chars().all(|c| matches!(c, '#'|'='|'-'|'*'|' '|'_')) { continue; }
        // Badges shields.io
        if ls.contains("shields.io") || ls.contains("img.shields") { continue; }
        // Tabla de idiomas con banderas múltiples
        let flags = ls.chars()
            .filter(|c| matches!(*c, '\u{1F1E6}'..='\u{1F1FF}'))
            .count();
        if flags >= 3 { continue; }
        // Item suelto de lista de idiomas: 1 bandera + línea corta
        // (típicamente "🇨🇳 中文 •" como entrada individual del listado).
        let visible = ls.chars().filter(|c| !c.is_whitespace()).count();
        if flags >= 2 && visible < 30 { continue; }
        // Línea con muchas barras "|" sin ser narrativa: lista de idiomas
        // (English | Español | Deutsch | …) o fila de tabla markdown
        // sin pipe inicial (raro pero pasa). 4+ pipes en una sola línea
        // es prácticamente garantizado uno de esos casos.
        if ls.matches('|').count() >= 4 { continue; }

        // ── Casos de "una línea por idioma" (post strip de <a>/<p>) ──
        // Cuando el .md original tenía:
        //   <p>
        //     <a href="...">English</a> |
        //     <a href="...">简体中文</a> |
        //   </p>
        // mi strip multilínea deja líneas tipo "English |" / "简体中文 |".
        // Cada una tiene visible<25 y termina en separador.
        if (ls.ends_with('|') || ls.ends_with('•') || ls.ends_with('｜')
            || ls.ends_with('·') || ls.ends_with('/'))
            && visible < 30
        {
            continue;
        }
        // Línea con muy poco contenido visible (≤12 chars sin contar espacios).
        // Captura el ÚLTIMO idioma del listado (que viene sin separador final)
        // y otros residuos como "中文", "한국어", "EN", "日本語" sueltos.
        // Una descripción real con tan pocos chars no aporta nada útil.
        if visible <= 12 { continue; }
        // Línea corta con cualquier separador: "简体中文 | English",
        // "Language / 语言 / 語言 / Dil", "中文 / English / 한국어".
        if visible < 50 && (
            ls.contains(" | ") || ls.contains(" / ") || ls.contains(" • ")
            || ls.contains(" ｜ ") || ls.contains(" · ")
        ) {
            // Heurística A: 2+ separadores del mismo tipo → navegación de idiomas.
            let pipes   = ls.matches(" | ").count() + ls.matches(" ｜ ").count();
            let slashes = ls.matches(" / ").count();
            let dots    = ls.matches(" • ").count() + ls.matches(" · ").count();
            if pipes >= 2 || slashes >= 2 || dots >= 2 {
                continue;
            }
            // Heurística B: 1 separador + ambos lados son etiquetas cortas
            // (≤2 palabras o solo CJK) → probable "X | Y" tipo "简体中文 | English".
            //
            // OJO: separar SOLO por el caracter del separador detectado.
            // Si la línea contiene un path como "[简体中文](./README.zh-CN.md)|"
            // y splitearamos por '|' Y '/' a la vez, parts.len() saldría >2
            // por culpa del '/' del path y la heurística fallaría.
            if pipes == 1 || slashes == 1 || dots == 1 {
                let sep_str: &str = if pipes == 1 { " | " }
                    else if slashes == 1 { " / " }
                    else if ls.contains(" • ") { " • " }
                    else { " · " };
                let parts: Vec<&str> = ls.split(sep_str).collect();
                let all_short = parts.iter().all(|p| {
                    let t = p.trim();
                    !t.is_empty() && t.split_whitespace().count() <= 2
                });
                if all_short && parts.len() == 2 {
                    continue;
                }
            }
        }
        // Cabecera explícita "Language / Idioma / Lang …"
        let lower_kw = ls.to_lowercase();
        if lower_kw.starts_with("language ")
            || lower_kw.starts_with("language:")
            || lower_kw.starts_with("language/")
            || lower_kw.starts_with("idioma ")
            || lower_kw.starts_with("idioma:")
            || lower_kw.starts_with("lang ")
            || lower_kw.starts_with("lang:")
            || lower_kw.starts_with("lang/")
            || lower_kw.starts_with("readme in")
            || lower_kw.starts_with("read this")
        {
            continue;
        }
        // Item de TOC
        let lower = ls.to_lowercase();
        if lower.starts_with("- [") || lower.starts_with("* [") {
            continue;
        }
        // Atributo HTML huérfano (src=, alt=, srcset=, …)
        if RE_ATTR_LINE.is_match(ls) { continue; }
        // ">" solitario o muy corto: cierre fragmentado de tag
        if ls.starts_with('>') && visible < 30 {
            continue;
        }
        // Línea que es mayoritariamente links/imágenes: 3+ patrones "]("
        if ls.matches("](").count() >= 3 { continue; }
        // Línea que es UN SOLO link como contenido principal:
        //   "[texto](url)"   (link simple solo)
        //   "[[texto]](url)" (wiki-style link, tipo whisper)
        //   "![alt](url)"    (ya capturado arriba con starts_with)
        // Si la línea entera es básicamente [(...)](url), no narrativa.
        if ls.matches("](").count() == 1
            && (ls.starts_with("[[") || ls.starts_with('['))
            && ls.ends_with(')')
        {
            // Línea que es exactamente UN link `[texto](url)` sin texto
            // narrativo extra. Descartamos: el README ya tiene el link en
            // su propia línea, no aporta a la descripción corta.
            // (Cálculos de offsets eliminados — eran dead code de un refactor anterior.)
            continue;
        }

        let line_chars = ls.chars().count();
        useful.push(ls.to_string());
        total_chars += line_chars + 1;
        if total_chars > 240 { break; }
    }

    if useful.is_empty() { return None; }

    // ── Limpieza del blob: parser char-by-char que strippea
    //    HTML inline + markdown residual. Sin regex para no
    //    incurrir en costo de compilación por llamada.
    let blob = useful.join(" ");
    let mut clean = String::with_capacity(blob.len());
    let mut chars = blob.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Tag HTML <…> — descarta hasta el siguiente '>'.
            '<' => {
                for ch in chars.by_ref() {
                    if ch == '>' { break; }
                }
                // No emitimos nada — la tag entera desaparece.
            }
            // Link markdown [texto](url) — conservamos solo "texto".
            '[' => {
                let mut text = String::new();
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == ']' { closed = true; break; }
                    text.push(ch);
                }
                if closed && chars.peek() == Some(&'(') {
                    chars.next();
                    for ch in chars.by_ref() {
                        if ch == ')' { break; }
                    }
                    clean.push_str(&text);
                } else {
                    // No es link, restauramos lo que copiamos.
                    clean.push('[');
                    clean.push_str(&text);
                    if closed { clean.push(']'); }
                }
            }
            // Marcadores de formato markdown que no aportan texto.
            '`' | '*' | '_' => {}
            // Backslash al final de línea (line break markdown) o sueltos.
            '\\' => {}
            _ => clean.push(c),
        }
    }

    // Colapsar whitespace y truncar a 240 chars.
    let collapsed: String = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let final_text: String = collapsed.chars().take(240).collect();
    let trimmed = final_text.trim();
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_MODEL:   &str = "claude-sonnet-4-6";

const README_VIEWER_TEMPLATE: &str = include_str!("../templates/readme_viewer.html");

#[derive(Serialize)]
struct AnthropicReq<'a> {
    model:      &'a str,
    max_tokens: u32,
    messages:   Vec<AnthropicMsg<'a>>,
}
#[derive(Serialize)]
struct AnthropicMsg<'a> { role: &'a str, content: &'a str }

#[derive(Deserialize)]
struct AnthropicResp { content: Vec<AnthropicContent> }
#[derive(Deserialize)]
struct AnthropicContent { text: String }

/// Pide a Claude que traduzca el markdown al español preservando el formato.
/// Devuelve el markdown traducido o un error con el motivo específico.
pub fn translate_readme_md(md_content: &str, repo_name: &str, api_key: &str) -> Result<String> {
    let max_chars = 12_000;
    let original_chars = md_content.chars().count();
    let truncated_orig = original_chars > max_chars;
    let content: String = md_content.chars().take(max_chars).collect();

    let prompt = format!(
        "Traduce al ESPAÑOL el siguiente README de un repositorio GitHub. \
        Reglas estrictas:\n\
        1) PRESERVA exactamente el formato Markdown (#, ##, ```, listas, tablas, \
        links [texto](url), imágenes ![alt](url), badges, HTML).\n\
        2) NO traduzcas: nombres de comandos, código, paths, URLs, identificadores, \
        nombres propios, marcas, versiones, licencias técnicas (MIT, Apache, GPL).\n\
        3) NO añadas notas, prefacios ni comentarios. Devuelve SOLO el markdown traducido.\n\
        4) Mantén el tono técnico original.\n\
        5) Si encuentras código de bloque (```...```), déjalo SIN MODIFICAR.\n\n\
        Repositorio: {}\n\n\
        README ORIGINAL:\n\
        ----- INICIO -----\n\
        {}\n\
        ----- FIN -----\n",
        repo_name, content
    );

    let req = AnthropicReq {
        model:      ANTHROPIC_MODEL,
        max_tokens: 8000,
        messages:   vec![AnthropicMsg { role: "user", content: &prompt }],
    };

    let req_json = serde_json::to_value(&req)
        .map_err(|e| anyhow!("serializando request: {}", e))?;

    // ureq::Error tiene dos variantes: Status (HTTP no-2xx con body) y Transport (red).
    let resp = match ureq::post(ANTHROPIC_API_URL)
        .set("x-api-key", api_key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(240))
        .send_json(req_json)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            // Intentar extraer el body de error de Anthropic (suele venir como
            // {"type":"error","error":{"type":"...","message":"..."}}).
            let body = r.into_string().unwrap_or_else(|_| "<sin body>".into());
            let detail: String = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string()))
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(anyhow!("HTTP {} · {}", code, detail));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(anyhow!("red/timeout: {}", t));
        }
    };

    let status = resp.status();
    if status != 200 {
        return Err(anyhow!("HTTP {} inesperado", status));
    }
    let body: AnthropicResp = resp.into_json()
        .map_err(|e| anyhow!("body no es JSON Anthropic válido: {}", e))?;
    let first = body.content.first()
        .ok_or_else(|| anyhow!("respuesta vacía (content[]={})", body.content.len()))?;
    let mut text = first.text.trim().to_string();
    if text.is_empty() {
        return Err(anyhow!(
            "Claude devolvió texto vacío (input {} chars, posible truncamiento)",
            original_chars));
    }

    if truncated_orig {
        text.push_str(
            "\n\n---\n\n*(Traducción truncada — el README original es muy largo. \
             Usa el botón 📖 para ver la versión completa en idioma original.)*\n"
        );
    }
    Ok(text)
}

/// Traduce al español una línea corta de descripción (típicamente el campo
/// `description` de la GitHub API). Optimizada para latencia: timeout 30 s,
/// max_tokens 256 (≈800 chars), 1 sola request HTTP.
///
/// Coste por llamada: ~50-150 tokens (~US$0.001). Para 146 repos: <US$0.20.
pub fn translate_short_desc(text_en: &str, api_key: &str) -> Result<String> {
    let trimmed = text_en.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("descripción de entrada vacía"));
    }

    let prompt = format!(
        "Traduce al ESPAÑOL esta descripción corta de un repositorio GitHub.\n\
        Reglas:\n\
        - Devuelve SOLO la traducción, sin comillas, sin notas, sin prefacios.\n\
        - Una sola línea, máximo 240 caracteres.\n\
        - Conserva nombres propios, marcas y términos técnicos en su forma original.\n\
        - No añadas formato Markdown ni HTML.\n\n\
        Original:\n{}\n",
        trimmed
    );

    let req = AnthropicReq {
        model:      ANTHROPIC_MODEL,
        max_tokens: 256,
        messages:   vec![AnthropicMsg { role: "user", content: &prompt }],
    };
    let req_json = serde_json::to_value(&req)
        .map_err(|e| anyhow!("serializando: {}", e))?;

    let resp = match ureq::post(ANTHROPIC_API_URL)
        .set("x-api-key", api_key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(30))
        .send_json(req_json)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_else(|_| "<sin body>".into());
            let preview: String = body.chars().take(160).collect();
            return Err(anyhow!("HTTP {} · {}", code, preview));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(anyhow!("red: {}", t));
        }
    };

    if resp.status() != 200 {
        return Err(anyhow!("HTTP {} inesperado", resp.status()));
    }
    let body: AnthropicResp = resp.into_json()
        .map_err(|e| anyhow!("body no es JSON Anthropic: {}", e))?;
    let first = body.content.first()
        .ok_or_else(|| anyhow!("respuesta vacía (content[]={})", body.content.len()))?;
    let mut text = first.text.trim().to_string();
    if text.is_empty() {
        return Err(anyhow!("Claude devolvió texto vacío"));
    }
    // Truncar a 240 chars manteniendo boundaries UTF-8.
    if text.chars().count() > 240 {
        text = text.chars().take(240).collect();
    }
    Ok(text)
}

fn find_readme(repo: &Path) -> Option<PathBuf> {
    for c in &["README.md","README.MD","Readme.md","readme.md","README","README.txt"] {
        let p = repo.join(c);
        if p.exists() { return Some(p); }
    }
    None
}

/// Genera `<repo>/_README_es.html`.
/// - Cachea el markdown traducido en `<repo>/_README_es.md`.
/// - Si `allow_api_call=false` y no hay caché, devuelve `None` (no llama LLM).
pub fn render_readme_html_es(
    repo: &Path,
    categoria: &str,
    root: &Path,
    api_key: &str,
    allow_api_call: bool,
    force: bool,
) -> Result<Option<PathBuf>> {
    let Some(readme) = find_readme(repo) else { return Ok(None); };
    let md_es_cache = repo.join("_README_es.md");
    let out         = repo.join("_README_es.html");

    // Skip si _README_es.html ya es más nuevo que el original
    if !force && out.exists() {
        if let (Ok(h), Ok(m)) = (fs::metadata(&out), fs::metadata(&readme)) {
            if let (Ok(ht), Ok(mt)) = (h.modified(), m.modified()) {
                if ht >= mt { return Ok(Some(out)); }
            }
        }
    }

    // Reusar caché md_es si es más nueva que original
    let mut md_es: Option<String> = None;
    if !force && md_es_cache.exists() {
        if let (Ok(c), Ok(m)) = (fs::metadata(&md_es_cache), fs::metadata(&readme)) {
            if let (Ok(ct), Ok(mt)) = (c.modified(), m.modified()) {
                if ct >= mt {
                    md_es = Some(read_text_safe(&md_es_cache, 999_999));
                }
            }
        }
    }

    if md_es.is_none() {
        if !allow_api_call {
            return Ok(None);
        }
        let original = read_text_safe(&readme, 999_999);
        let name = repo.file_name().map(|s| s.to_string_lossy().into_owned())
                     .unwrap_or_default();
        // Propagar el error específico de la API hacia arriba para que el
        // handler de la GUI muestre el motivo real (HTTP 4xx, timeout, etc).
        let translated = translate_readme_md(&original, &name, api_key)?;
        let _ = fs::write(&md_es_cache, &translated);
        md_es = Some(translated);
    }

    let md_es_text = md_es.unwrap_or_default();
    let body = markdown_to_html(&md_es_text);

    let title = repo.file_name().map(|s| s.to_string_lossy().into_owned())
                  .unwrap_or_default();
    let back_url = format!(
        "file:///{}",
        root.join("buscador.html").to_string_lossy().replace('\\', "/")
    );
    let html = README_VIEWER_TEMPLATE
        .replace("__TITLE__", &format!("{} 🇨🇴", html_escape(&title)))
        .replace("__CAT__",   &html_escape(categoria))
        .replace("__BACK__",  &back_url)
        .replace("__BODY__",  &body);
    fs::write(&out, html)?;
    Ok(Some(out))
}

// Mismas regex que readme.rs para abrir/cerrar bloques HTML que envuelven
// markdown. Insertando blank lines forzamos a pulldown-cmark a procesar
// el contenido en lugar de tratarlo como HTML opaco.
static RE_BLOCK_HTML_OPEN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?im)^(\s*<(?:div|center|p|section|article|main|header|footer|nav|aside)(?:\s+[^>]*)?>)\s*$"
    ).unwrap()
});
static RE_BLOCK_HTML_CLOSE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?im)^(\s*</(?:div|center|p|section|article|main|header|footer|nav|aside)>)\s*$"
    ).unwrap()
});

fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    // Pre-procesar bloques HTML para que el markdown interno se renderice.
    let pre1 = RE_BLOCK_HTML_OPEN.replace_all(md, "$1\n");
    let pre2 = RE_BLOCK_HTML_CLOSE.replace_all(&pre1, "\n$1");

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_GFM);
    let parser = Parser::new_ext(&pre2, opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&#39;")
}

/// Pequeña prueba de conectividad: pide un mensaje corto para validar la API key.
pub fn test_api_key(api_key: &str) -> Result<String> {
    let prompt = "Responde solo con la palabra OK.";
    let req = AnthropicReq {
        model: ANTHROPIC_MODEL,
        max_tokens: 16,
        messages: vec![AnthropicMsg { role: "user", content: prompt }],
    };
    let resp = ureq::post(ANTHROPIC_API_URL)
        .set("x-api-key", api_key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(15))
        .send_json(serde_json::to_value(&req)?);
    match resp {
        Ok(r) if r.status() == 200 => {
            let body: AnthropicResp = r.into_json()?;
            Ok(body.content.first().map(|c| c.text.clone()).unwrap_or_else(|| "(vacío)".into()))
        }
        Ok(r) => Err(anyhow!("HTTP {}", r.status())),
        Err(e) => Err(anyhow!("Red: {}", e)),
    }
}
