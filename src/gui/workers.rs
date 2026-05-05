//! Workers de aplicación de cada diálogo modal.
//!
//! Cada uno corre en su propio `thread::spawn` y comunica progreso al
//! main thread mediante `WorkerMsg` (mpsc::channel). No tienen estado
//! propio: reciben los datos a procesar como parámetros y solo producen
//! mensajes hacia el main thread.
//!
//! El worker grande `run_worker` (scan/apply/reindex/etc.) sigue en
//! `app.rs` por ahora — moverlo es trabajo de una iteración futura.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::apply_actions::apply_dup_action;
use crate::categories_config::{
    load_or_default as load_cats_cfg, save as save_cats_cfg,
    CategoriaDef, CategoriasConfig,
};
use crate::gui::theme;
use crate::gui::types::{DuplicateItem, WorkerMsg};
use crate::html::generate_html;
use crate::index::{rebuild_index, RebuildOpts};
use crate::reclassify::{apply_reclassification, compute_reclassification, ReclassifyChange};
use crate::topic_discovery::TopicCandidate;

// ─────────────────────────────────────────────────────────────────
//  RECLASIFICACIÓN MASIVA (Fase 3b)
// ─────────────────────────────────────────────────────────────────

/// Worker dedicado a aplicar la reclasificación + reindex + regenerar HTML.
/// Vive en su propio thread para que la GUI siga respondiendo y la barra
/// indeterminada anime correctamente durante los movimientos físicos.
pub fn apply_reclass_worker(
    root: PathBuf,
    changes: Vec<ReclassifyChange>,
    tx: Sender<WorkerMsg>,
) {
    let count_sel = changes.iter().filter(|c| c.selected).count();
    let _ = tx.send(WorkerMsg::Status(format!("Moviendo {} repos…", count_sel)));
    let _ = tx.send(WorkerMsg::Progress(None));

    match apply_reclassification(&changes) {
        Ok((moved, errs)) => {
            let color = if errs == 0 { theme::OK } else { theme::WARN };
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✓ {} repos movidos · {} errores", moved, errs),
                color,
            ));
            let _ = tx.send(WorkerMsg::Status("Regenerando índice y buscador.html…".to_string()));
            match rebuild_index(&root, RebuildOpts {
                allow_github: false,
                force_github_retry: false,
                github_pat: None,
            }) {
                Ok(data) => {
                    let _ = generate_html(&root, &data);
                    let _ = tx.send(WorkerMsg::Log(
                        format!("  ✓ índice y buscador.html regenerados ({} repos)",
                                data.len()),
                        theme::OK,
                    ));
                    let _ = tx.send(WorkerMsg::Status(
                        format!("OK · {} reclasificados", moved)));
                }
                Err(e) => {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("  ✗ Error reindexando: {}", e), theme::ERR));
                    let _ = tx.send(WorkerMsg::Status("Error en reindex".to_string()));
                }
            }
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✗ Error aplicando: {}", e), theme::ERR));
            let _ = tx.send(WorkerMsg::Status("Error".to_string()));
        }
    }
    let _ = tx.send(WorkerMsg::Done);
}

// ─────────────────────────────────────────────────────────────────
//  RESOLVER DUPLICADOS
// ─────────────────────────────────────────────────────────────────

/// Worker dedicado a aplicar las decisiones del diálogo de duplicados.
/// Vive en su propio thread porque las acciones físicas (mover/borrar
/// repos completos del disco) pueden tardar varios segundos por repo
/// pesado, y bloquear la GUI da la sensación de "no pasó nada" que el
/// usuario quería evitar. Al terminar, si hubo algún cambio en disco,
/// re-genera el índice y el buscador.html.
pub fn apply_duplicates_worker(
    root: PathBuf,
    items: Vec<DuplicateItem>,
    tx: Sender<WorkerMsg>,
) {
    let total = items.len();
    let _ = tx.send(WorkerMsg::Status(format!("Procesando {} duplicados…", total)));
    let _ = tx.send(WorkerMsg::Progress(None));

    let mut ok = 0usize;
    let mut sk = 0usize;
    let mut err = 0usize;

    for (i, it) in items.into_iter().enumerate() {
        // Status por item para que el usuario vea sobre qué repo va.
        // Crítico cuando uno de los repos pesa varios GB y la operación
        // (rename + rmtree del .git) se demora segundos.
        let _ = tx.send(WorkerMsg::Status(
            format!("Duplicado {}/{} · {}", i + 1, total, it.new_info.name)));
        let res = apply_dup_action(&it.action, &it.new_path, &it.old_path);
        let label = it.action.label();
        match res {
            Ok(_msg) if label == "skip" => {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  [skip] {}", it.new_info.name), theme::MUTED));
                sk += 1;
            }
            Ok(msg) => {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  [{}] {}  →  {}", label, it.new_info.name, msg),
                    theme::OK));
                ok += 1;
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::Log(
                    // {:#} de anyhow imprime contexto + cadena completa de causas.
                    format!("  [{}] {}  →  ERROR: {:#}", label, it.new_info.name, e),
                    theme::ERR));
                err += 1;
            }
        }
    }
    let _ = tx.send(WorkerMsg::Log(
        format!("━━━ {} aplicados · {} saltados · {} errores ━━━", ok, sk, err),
        theme::ACCENT_H));

    if ok > 0 {
        let _ = tx.send(WorkerMsg::Status(
            "Regenerando índice y buscador.html…".to_string()));
        match rebuild_index(&root, RebuildOpts {
            allow_github: false,
            force_github_retry: false,
            github_pat: None,
        }) {
            Ok(data) => {
                let _ = generate_html(&root, &data);
                let _ = tx.send(WorkerMsg::Log(
                    format!("  ✓ índice y buscador.html regenerados ({} repos)",
                            data.len()),
                    theme::OK));
                let _ = tx.send(WorkerMsg::Status(format!("OK · {} aplicados", ok)));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  ✗ Error reindexando: {}", e), theme::ERR));
                let _ = tx.send(WorkerMsg::Status("Error en reindex".to_string()));
            }
        }
    } else {
        let _ = tx.send(WorkerMsg::Status(
            format!("Listo · {} saltados · {} errores", sk, err)));
    }
    let _ = tx.send(WorkerMsg::Done);
}

// ─────────────────────────────────────────────────────────────────
//  EDITOR DE CATEGORÍAS (Fase 3c)
// ─────────────────────────────────────────────────────────────────

/// Worker dedicado a aplicar los cambios del editor de categorías.
///
/// Orden de operaciones (intencional):
///   1. **Migrar repos** de las categorías borradas al fallback.
///   2. **Renombrar carpetas** (fs::rename).
///   3. **Crear carpetas nuevas** (mkdir vacío).
///   4. **Persistir el config** a `data/categorias.json`.
///   5. **Reindex + regenerar HTML**.
pub fn apply_cats_worker(
    root:    PathBuf,
    cfg:     CategoriasConfig,
    renames: Vec<(String, String)>,
    deletes: Vec<String>,
    new_ids: Vec<String>,
    tx:      Sender<WorkerMsg>,
) {
    let _ = tx.send(WorkerMsg::Status("Aplicando cambios del editor…".to_string()));
    let _ = tx.send(WorkerMsg::Progress(None));

    // ── 1. Migración de repos de categorías borradas al fallback ──
    let fallback_id = cfg.fallback_id();
    if !deletes.is_empty() {
        let _ = tx.send(WorkerMsg::Log(
            format!("→ Migrando repos de {} categoría(s) borrada(s) a '{}'",
                    deletes.len(), fallback_id),
            theme::ACCENT_H));
    }
    for del in &deletes {
        let src_dir = root.join(del);
        if !src_dir.is_dir() {
            let _ = tx.send(WorkerMsg::Log(
                format!("  · {} (sin carpeta en disco, nada que migrar)", del),
                theme::MUTED));
            continue;
        }
        let dst_dir = root.join(&fallback_id);
        if let Err(e) = std::fs::create_dir_all(&dst_dir) {
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✗ no pude crear destino '{}': {}", dst_dir.display(), e),
                theme::ERR));
            continue;
        }
        let entries = match std::fs::read_dir(&src_dir) {
            Ok(rd) => rd.flatten().collect::<Vec<_>>(),
            Err(e) => {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  ✗ leyendo '{}': {}", src_dir.display(), e), theme::ERR));
                continue;
            }
        };
        let mut moved = 0usize;
        for entry in entries {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue; }
            let name = entry.file_name();
            let dst  = dst_dir.join(&name);
            if dst.exists() {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  ⚠ '{}' ya existe en destino, salteado", name.to_string_lossy()),
                    theme::WARN));
                continue;
            }
            match std::fs::rename(entry.path(), &dst) {
                Ok(_)  => moved += 1,
                Err(e) => {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("  ✗ moviendo '{}': {}", name.to_string_lossy(), e),
                        theme::ERR));
                }
            }
        }
        match std::fs::remove_dir(&src_dir) {
            Ok(_) => {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  ✓ {} → {} ({} repo[s] movidos · carpeta eliminada)",
                            del, fallback_id, moved),
                    theme::OK));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  ⚠ {} → {} ({} repo[s] movidos · carpeta NO eliminada: {})",
                            del, fallback_id, moved, e),
                    theme::WARN));
            }
        }
    }

    // ── 2. Renombres de carpetas ──
    if !renames.is_empty() {
        let _ = tx.send(WorkerMsg::Log(
            format!("→ Renombrando {} carpeta(s) de categoría", renames.len()),
            theme::ACCENT_H));
    }
    for (old_id, new_id) in &renames {
        let src = root.join(old_id);
        let dst = root.join(new_id);
        if !src.is_dir() {
            let _ = tx.send(WorkerMsg::Log(
                format!("  · {} → {} (sin carpeta física, solo cambia el config)",
                        old_id, new_id),
                theme::MUTED));
            continue;
        }
        if dst.exists() {
            let _ = tx.send(WorkerMsg::Log(
                format!("  ⚠ destino '{}' ya existe — fusionando contenidos",
                        dst.display()),
                theme::WARN));
            let entries = match std::fs::read_dir(&src) {
                Ok(rd) => rd.flatten().collect::<Vec<_>>(),
                Err(e) => {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("  ✗ leyendo '{}': {}", src.display(), e), theme::ERR));
                    continue;
                }
            };
            let mut merged = 0usize;
            for entry in entries {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue; }
                let name = entry.file_name();
                let to   = dst.join(&name);
                if to.exists() {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("    ⚠ '{}' colisiona en destino, salteado",
                                name.to_string_lossy()),
                        theme::WARN));
                    continue;
                }
                if let Err(e) = std::fs::rename(entry.path(), &to) {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("    ✗ '{}': {}", name.to_string_lossy(), e),
                        theme::ERR));
                } else {
                    merged += 1;
                }
            }
            let _ = std::fs::remove_dir(&src);
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✓ {} → {} (fusión · {} repo[s] movidos)", old_id, new_id, merged),
                theme::OK));
        } else {
            match std::fs::rename(&src, &dst) {
                Ok(_) => {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("  ✓ {} → {}", old_id, new_id), theme::OK));
                }
                Err(e) => {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("  ✗ {} → {}: {}", old_id, new_id, e), theme::ERR));
                }
            }
        }
    }

    // ── 3. Crear carpetas nuevas vacías ──
    if !new_ids.is_empty() {
        let _ = tx.send(WorkerMsg::Log(
            format!("→ Creando {} carpeta(s) nueva(s)", new_ids.len()),
            theme::ACCENT_H));
    }
    for nid in &new_ids {
        let dir = root.join(nid);
        if dir.exists() {
            let _ = tx.send(WorkerMsg::Log(
                format!("  · {} (ya existía en disco)", nid), theme::MUTED));
        } else {
            match std::fs::create_dir_all(&dir) {
                Ok(_)  => {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("  ✓ {}/", nid), theme::OK));
                }
                Err(e) => {
                    let _ = tx.send(WorkerMsg::Log(
                        format!("  ✗ {}: {}", nid, e), theme::ERR));
                }
            }
        }
    }

    // ── 4. Persistir el config a data/categorias.json ──
    let _ = tx.send(WorkerMsg::Status("Guardando data/categorias.json…".to_string()));
    match save_cats_cfg(&root, &cfg) {
        Ok(_) => {
            let _ = tx.send(WorkerMsg::Log(
                "  ✓ data/categorias.json actualizado".to_string(), theme::OK));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✗ guardando config: {}", e), theme::ERR));
            let _ = tx.send(WorkerMsg::Status("Error guardando config".to_string()));
            let _ = tx.send(WorkerMsg::Done);
            return;
        }
    }

    // ── 5. Reindex + regenerar HTML ──
    let _ = tx.send(WorkerMsg::Status("Regenerando índice y buscador.html…".to_string()));
    match rebuild_index(&root, RebuildOpts {
        allow_github: false,
        force_github_retry: false,
        github_pat: None,
    }) {
        Ok(data) => {
            let _ = generate_html(&root, &data);
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✓ índice y buscador.html regenerados ({} repos)", data.len()),
                theme::OK));
            let _ = tx.send(WorkerMsg::Status(
                format!("OK · {} renombres · {} nuevas · {} borradas",
                        renames.len(), new_ids.len(), deletes.len())));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✗ reindex: {}", e), theme::ERR));
            let _ = tx.send(WorkerMsg::Status("Error en reindex".to_string()));
        }
    }
    let _ = tx.send(WorkerMsg::Done);
}

// ─────────────────────────────────────────────────────────────────
//  DESCUBRIDOR DE CATEGORÍAS (Fase 4)
// ─────────────────────────────────────────────────────────────────

/// Worker que aplica los candidatos seleccionados del descubridor:
/// recarga config, valida ids, crea CategoriaDefs, persiste, crea
/// carpetas, reclasifica todo el árbol, reindex + HTML.
pub fn apply_discover_worker(
    root:   PathBuf,
    chosen: Vec<TopicCandidate>,
    tx:     Sender<WorkerMsg>,
) {
    let _ = tx.send(WorkerMsg::Status(
        format!("Creando {} categorías nuevas…", chosen.len())));
    let _ = tx.send(WorkerMsg::Progress(None));

    // 1) Recargar el config (puede haber cambiado).
    let mut cfg = load_cats_cfg(&root);
    let existing_ids: std::collections::HashSet<String> = cfg.categorias.iter()
        .map(|c| c.id.clone())
        .collect();

    // 1.5) Renumerar IDs propuestos para que sean CONSECUTIVOS según el
    //      orden de selección. Si el usuario editó manualmente a algo sin
    //      prefijo numérico, respetamos su elección.
    let next_n_start: u32 = existing_ids.iter()
        .filter_map(|id| {
            let s = id.as_str();
            if s.len() >= 3 && s.as_bytes().get(2) == Some(&b'-') {
                s[..2].parse::<u32>().ok()
            } else { None }
        })
        .max().unwrap_or(0) + 1;
    let re_num_prefix = regex::Regex::new(r"^\d{2}-").expect("regex const");
    let mut chosen: Vec<TopicCandidate> = chosen;
    let mut next_n = next_n_start;
    for c in chosen.iter_mut() {
        let id = c.id_propuesto.trim().to_string();
        if re_num_prefix.is_match(&id) {
            c.id_propuesto = format!("{:02}-{}", next_n, &id[3..]);
            next_n += 1;
        }
    }

    // 2) Validar (id completo + prefijo NN únicos).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_prefixes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let existing_prefixes: std::collections::HashSet<String> = existing_ids.iter()
        .filter_map(|id| id.split('-').next()
            .filter(|s| s.len()==2 && s.chars().all(|c| c.is_ascii_digit()))
            .map(|s| s.to_string()))
        .collect();
    let mut errors: Vec<String> = Vec::new();
    for c in &chosen {
        let id = c.id_propuesto.trim();
        if id.is_empty() {
            errors.push(format!("topic '{}': id propuesto vacío", c.topic));
            continue;
        }
        if id.chars().any(|ch| matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                                 || ch.is_whitespace())
        {
            errors.push(format!("id '{}' contiene caracteres inválidos", id));
        }
        if existing_ids.contains(id) {
            errors.push(format!("id '{}' ya existe en el config", id));
        }
        if !seen.insert(id.to_string()) {
            errors.push(format!("id '{}' propuesto dos veces en el set seleccionado", id));
        }
        if let Some(prefix) = id.split('-').next()
            .filter(|s| s.len()==2 && s.chars().all(|c| c.is_ascii_digit()))
            .map(|s| s.to_string())
        {
            if existing_prefixes.contains(&prefix) {
                errors.push(format!("prefijo numérico '{}-' de '{}' ya existe en otra categoría", prefix, id));
            }
            if !seen_prefixes.insert(prefix.clone()) {
                errors.push(format!("prefijo numérico '{}-' propuesto dos veces en el set", prefix));
            }
        }
    }
    if !errors.is_empty() {
        for e in &errors {
            let _ = tx.send(WorkerMsg::Log(format!("  ✗ {}", e), theme::ERR));
        }
        let _ = tx.send(WorkerMsg::Log(
            "✗ Validación falló — no se aplicó nada. Editá los IDs en conflicto y volvé a intentar."
                .to_string(),
            theme::ERR));
        let _ = tx.send(WorkerMsg::Status("Error de validación".to_string()));
        let _ = tx.send(WorkerMsg::Done);
        return;
    }

    // 3) Construir las CategoriaDef nuevas.
    let mut nuevas: Vec<CategoriaDef> = Vec::with_capacity(chosen.len());
    for c in &chosen {
        let mut boosts: Vec<(String, u32)> = Vec::new();
        boosts.push((c.topic.clone(), 12));
        for syn in &c.merged {
            boosts.push((syn.clone(), 10));
        }
        nuevas.push(CategoriaDef {
            id:           c.id_propuesto.trim().to_string(),
            keywords:     Vec::new(),
            topic_boosts: boosts,
        });
    }

    // 4) Insertar ANTES de la última para preservar el fallback.
    if cfg.categorias.is_empty() {
        cfg.categorias.extend(nuevas.iter().cloned());
    } else {
        let last_idx = cfg.categorias.len() - 1;
        for (offset, n) in nuevas.iter().enumerate() {
            cfg.categorias.insert(last_idx + offset, n.clone());
        }
    }
    let _ = tx.send(WorkerMsg::Log(
        format!("→ {} categorías insertadas en el config (antes del fallback)",
                nuevas.len()),
        theme::ACCENT_H));

    // 5) Persistir el config + crear carpetas vacías.
    let _ = tx.send(WorkerMsg::Status("Guardando data/categorias.json…".to_string()));
    if let Err(e) = save_cats_cfg(&root, &cfg) {
        let _ = tx.send(WorkerMsg::Log(
            format!("  ✗ Error guardando config: {}", e), theme::ERR));
        let _ = tx.send(WorkerMsg::Status("Error".to_string()));
        let _ = tx.send(WorkerMsg::Done);
        return;
    }
    for n in &nuevas {
        let dir = root.join(&n.id);
        match std::fs::create_dir_all(&dir) {
            Ok(_)  => {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  ✓ {}/  (topics: {})",
                            n.id,
                            n.topic_boosts.iter().map(|(t,_)| t.as_str())
                                .collect::<Vec<_>>().join(", ")),
                    theme::OK));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::Log(
                    format!("  ✗ creando {}: {}", n.id, e), theme::ERR));
            }
        }
    }

    // 6) Reclasificar con el nuevo set.
    let _ = tx.send(WorkerMsg::Status(
        "Recomputando clasificación con las nuevas categorías…".to_string()));
    match compute_reclassification(&root) {
        Ok(mut changes) => {
            for ch in &mut changes { ch.selected = true; }
            let total = changes.len();
            if total == 0 {
                let _ = tx.send(WorkerMsg::Log(
                    "  · ningún repo cambia de categoría (los nuevos topics no \
                     coinciden con repos existentes — extraño, revisá el match)".to_string(),
                    theme::WARN));
            } else {
                let _ = tx.send(WorkerMsg::Log(
                    format!("→ {} repo(s) cambian de categoría con el nuevo set", total),
                    theme::ACCENT_H));
                match apply_reclassification(&changes) {
                    Ok((moved, errs)) => {
                        let color = if errs == 0 { theme::OK } else { theme::WARN };
                        let _ = tx.send(WorkerMsg::Log(
                            format!("  ✓ {} repos movidos · {} errores", moved, errs),
                            color));
                    }
                    Err(e) => {
                        let _ = tx.send(WorkerMsg::Log(
                            format!("  ✗ Error aplicando reclasificación: {}", e),
                            theme::ERR));
                    }
                }
            }
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✗ Error computando reclasificación: {}", e), theme::ERR));
        }
    }

    // 7) Reindex + HTML.
    let _ = tx.send(WorkerMsg::Status(
        "Regenerando índice y buscador.html…".to_string()));
    match rebuild_index(&root, RebuildOpts {
        allow_github: false,
        force_github_retry: false,
        github_pat: None,
    }) {
        Ok(data) => {
            let _ = generate_html(&root, &data);
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✓ índice y buscador.html regenerados ({} repos · {} categorías)",
                        data.len(), cfg.categorias.len()),
                theme::OK));
            let _ = tx.send(WorkerMsg::Status(
                format!("OK · {} categorías nuevas creadas", chosen.len())));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::Log(
                format!("  ✗ Error reindexando: {}", e), theme::ERR));
            let _ = tx.send(WorkerMsg::Status("Error en reindex".to_string()));
        }
    }
    let _ = tx.send(WorkerMsg::Done);
}
