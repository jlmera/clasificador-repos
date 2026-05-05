//! Helpers libres compartidos entre `app.rs` y los workers de la GUI.
//!
//! Funciones puras que no dependen del estado de la app (`ClasificadorApp`).
//! Se mantienen acá para reducir el tamaño de `app.rs` y permitir tests
//! unitarios independientes en el futuro.

use std::collections::HashMap;
use std::path::Path;

use crate::categories_config::CategoriasConfig;
use crate::gui::types::CatsEditorDialogState;

/// Valida el estado del editor de categorías y calcula el diff respecto al
/// snapshot inicial. Devuelve `Some((cfg, renames, deletes, new_ids, summary))`
/// si todo está OK; `None` si hubo errores (en cuyo caso `dlg.status` se
/// actualiza con el mensaje rojo para que la siguiente repintada lo muestre).
///
/// Vive como función libre (no método) para que el caller pueda soltar el
/// borrow de `&mut self.cats_dialog` antes de tocar otros campos de self.
pub fn validate_and_diff_cats(
    dlg: &mut CatsEditorDialogState,
) -> Option<(CategoriasConfig, Vec<(String, String)>, Vec<String>, Vec<String>, String)> {
    // ── Validación de la lista global ──
    if dlg.rows.is_empty() {
        dlg.status = "✗ La lista de categorías no puede quedar vacía. \
                      Reset a defaults o añade al menos una.".to_string();
        return None;
    }

    // ── Validación por fila (IDs) ──
    // - no vacío
    // - sin caracteres reservados de Windows (\ / : * ? " < > |) ni espacios
    // - único dentro de la lista
    let mut errors: Vec<String> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (i, row) in dlg.rows.iter().enumerate() {
        let id = row.def.id.trim();
        if id.is_empty() {
            errors.push(format!("fila #{}: id vacío", i + 1));
            continue;
        }
        if id.chars().any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                               || c.is_whitespace())
        {
            errors.push(format!("fila #{} ({}): id contiene caracteres inválidos", i + 1, id));
        }
        if let Some(prev_i) = seen.insert(id.to_string(), i) {
            errors.push(format!("id duplicado '{}' (filas #{} y #{})",
                                id, prev_i + 1, i + 1));
        }
    }
    if !errors.is_empty() {
        dlg.status = format!("✗ {} error(es): {}",
                             errors.len(), errors.join("; "));
        return None;
    }

    // ── Diff: detectar renombres, borrados y nuevos. ──
    let mut renames: Vec<(String, String)> = Vec::new(); // (old_id, new_id)
    let mut new_ids:  Vec<String>          = Vec::new();
    for row in &dlg.rows {
        if row.is_new() {
            new_ids.push(row.def.id.clone());
        } else if row.is_renamed() {
            let orig = row.original_id.as_ref().unwrap().clone();
            renames.push((orig, row.def.id.clone()));
        }
    }
    let mut deletes: Vec<String> = dlg.original_ids.iter()
        .filter(|orig| !dlg.rows.iter()
            .any(|r| r.original_id.as_deref() == Some(orig.as_str())))
        .cloned()
        .collect();
    deletes.retain(|d| !renames.iter().any(|(old, _)| old == d));

    let cfg = CategoriasConfig {
        version:    dlg.version,
        categorias: dlg.rows.iter().map(|r| r.def.clone()).collect(),
    };

    let summary_msg = format!(
        "━━━ Editor de categorías: {} renombres · {} nuevas · {} borradas ━━━",
        renames.len(), new_ids.len(), deletes.len()
    );

    dlg.status.clear();
    Some((cfg, renames, deletes, new_ids, summary_msg))
}

/// Cuenta repositorios (subdirectorios) en cada subcarpeta directa de `root`,
/// ignorando carpetas que empiezan con `_` (`_inbox`, `_duplicados`) y la
/// carpeta `data/` que aloja config + caché. La clave del HashMap es el
/// nombre de la carpeta (= id de categoría); el valor es la cantidad de
/// subdirs que hay dentro (= repos individuales).
///
/// IMPORTANTE: el resultado puede contener carpetas que NO están en el
/// config de categorías (si el usuario movió cosas a mano o quedó residuo
/// de un id viejo). Eso permite al editor detectar "carpetas huérfanas"
/// y mostrar advertencias al usuario.
pub fn count_repos_per_category(root: &Path) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let rd = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return counts,
    };
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('_') || name == "data" || name == "wiki"
           || name == "fuente" || name == "reinicios" || name.starts_with('.')
        {
            continue;
        }
        let inner = std::fs::read_dir(entry.path())
            .map(|rd2| rd2.flatten()
                          .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                          .count())
            .unwrap_or(0);
        counts.insert(name, inner);
    }
    counts
}

/// Abre una URL en el navegador default del sistema. En Windows usa
/// `cmd /C start` con `CREATE_NO_WINDOW` para no flashear una ventana de
/// consola; en otras plataformas usa `xdg-open`.
pub fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
    }
    #[cfg(not(windows))]
    { std::process::Command::new("xdg-open").arg(url).spawn()?; }
    Ok(())
}
