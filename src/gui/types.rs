//! Tipos compartidos entre `app.rs` y los workers de la GUI.
//!
//! Aquí viven los `DialogState`s de cada modal, el enum `WorkerMsg` que
//! usan los worker threads para comunicarse con el main thread, y los
//! wrappers como `DuplicateItem` y `CatRow` que solo tienen sentido en el
//! contexto de la GUI (no se persisten a disco — eso lo hacen los structs
//! del crate base como `CategoriaDef`).

use std::collections::HashMap;
use std::path::PathBuf;

use egui::Color32;

use crate::apply_actions::{CompareInfo, DupAction};
use crate::categories_config::CategoriaDef;
use crate::reclassify::ReclassifyChange;
use crate::topic_discovery::{DiscoverFilters, TopicCandidate};

/// Mensajes que el worker thread envía al main thread vía `mpsc::channel`.
/// El main thread los recoge en `poll_worker()` cada frame y actualiza el
/// log, status, progress o abre diálogos según corresponda.
pub enum WorkerMsg {
    Log(String, Color32),
    Status(String),
    Progress(Option<(usize, usize)>), // None = indeterminate
    OpenDuplicatesDialog(Vec<DuplicateItem>),
    OpenReclassifyDialog(Vec<ReclassifyChange>),
    Done,
}

// ─────────────────────────────────────────────────────────────────
//  RESOLVER DUPLICADOS
// ─────────────────────────────────────────────────────────────────

/// Estado de un duplicado pendiente de resolver (visible en el diálogo).
pub struct DuplicateItem {
    pub new_path: PathBuf,
    pub old_path: PathBuf,
    pub motivo:   String,
    pub new_info: CompareInfo,
    pub old_info: CompareInfo,
    pub action:   DupAction,
    pub new_name: String,
}

/// Estado del diálogo modal "Resolver duplicados".
pub struct DuplicatesDialogState {
    pub items: Vec<DuplicateItem>,
    pub open:  bool,
}

// ─────────────────────────────────────────────────────────────────
//  CREDENCIALES
// ─────────────────────────────────────────────────────────────────

/// Estado del diálogo modal "Configurar credenciales" (API Key + GitHub PAT).
pub struct ApiKeyDialogState {
    pub key:      String,   // Anthropic API key (sk-ant-…)
    pub pat:      String,   // GitHub Personal Access Token (ghp_… o github_pat_…)
    pub show:     bool,     // mostrar Anthropic key en plano
    pub show_pat: bool,     // mostrar PAT en plano
    pub status:   String,
}

// ─────────────────────────────────────────────────────────────────
//  RECLASIFICAR
// ─────────────────────────────────────────────────────────────────

/// Estado del diálogo de reclasificación masiva.
pub struct ReclassDialogState {
    pub changes: Vec<ReclassifyChange>,
    pub open:    bool,
}

// ─────────────────────────────────────────────────────────────────
//  EDITOR DE CATEGORÍAS (Fase 3c)
// ─────────────────────────────────────────────────────────────────

/// Wrapper de edición sobre `CategoriaDef`. Mantiene un `original_id`
/// para detectar renombres al guardar — si `original_id != Some(id)`,
/// la categoría fue renombrada (o creada nueva si `original_id = None`).
///
/// El motivo de no modificar `CategoriaDef` directamente: ese struct se
/// serializa a JSON y no queremos contaminar el formato persistido con
/// metadata transitoria de la GUI.
#[derive(Debug, Clone)]
pub struct CatRow {
    /// ID original al abrir el diálogo. None = categoría nueva añadida en esta sesión.
    pub original_id: Option<String>,
    /// Definición editable (id, keywords, topic_boosts).
    pub def: CategoriaDef,
}

impl CatRow {
    /// Construye una fila a partir de una categoría YA EXISTENTE en el config.
    pub fn from_existing(def: CategoriaDef) -> Self {
        Self { original_id: Some(def.id.clone()), def }
    }
    /// Crea una fila para una categoría NUEVA, con id propuesto y listas vacías.
    pub fn new_blank(id: impl Into<String>) -> Self {
        Self {
            original_id: None,
            def: CategoriaDef {
                id:           id.into(),
                keywords:     Vec::new(),
                topic_boosts: Vec::new(),
            },
        }
    }
    /// True si la fila representa una categoría completamente nueva (no estaba al abrir).
    pub fn is_new(&self) -> bool { self.original_id.is_none() }
    /// True si la fila tiene un id distinto al original (=> hay que renombrar carpeta).
    pub fn is_renamed(&self) -> bool {
        match &self.original_id {
            Some(orig) => orig != &self.def.id,
            None => false, // las nuevas no se consideran "renombradas".
        }
    }
}

/// Estado del diálogo "Editor de categorías".
///
/// `version` se preserva del config original (no la mostramos en la GUI).
/// `repo_counts` se calcula al abrir el diálogo (snapshot del disco) y
/// se usa solo para mostrar (N repos) por cada fila + decidir si un
/// borrado dispara migración a fallback.
pub struct CatsEditorDialogState {
    pub version:      u32,
    /// Snapshot de IDs presentes en el config al abrir el diálogo.
    /// Se usa al guardar para detectar borrados: `original_ids − rows.original_id`
    /// es el conjunto de categorías que el usuario eliminó durante la edición.
    pub original_ids: Vec<String>,
    pub rows:         Vec<CatRow>,
    pub repo_counts:  HashMap<String, usize>,
    pub selected_idx: Option<usize>,
    pub status:       String,
    pub open:         bool,
}

// ─────────────────────────────────────────────────────────────────
//  DESCUBRIR CATEGORÍAS (Fase 4)
// ─────────────────────────────────────────────────────────────────

/// Estado del diálogo "🔍 Descubrir categorías".
///
/// Vive cargado en memoria mientras el usuario revisa candidatos. La
/// lista de candidatos `candidates` ya tiene las fusiones de sinónimos
/// aplicadas (singular/plural, -cli) — cada uno representa una futura
/// categoría con su id propuesto editable.
///
/// Cambiar `filters` regenera la lista usando el snapshot original
/// (`source_repo_ids`) sin volver a leer el disco.
pub struct DiscoverDialogState {
    /// Snapshot inmutable del repo_ids cache al abrir el diálogo. Se
    /// reusa cuando el usuario mueve el slider del umbral o toca un
    /// checkbox de filtro — no queremos releer el archivo cada frame.
    pub source_repo_ids: crate::ids::IdsCache,
    /// Topics que YA están cubiertos por el config actual al abrir
    /// el diálogo. Se preserva sin importar las ediciones en otros
    /// diálogos durante esta sesión.
    pub source_cubiertos: std::collections::HashSet<String>,
    /// Próximo número correlativo para auto-generar `id_propuesto`.
    /// Típicamente = `cfg.categorias.len() + 1`.
    pub next_consecutivo: usize,
    /// Filtros activos (umbral + 3 hides). Cambiarlos regenera la lista.
    pub filters: DiscoverFilters,
    /// Lista actualmente mostrada. Se reemplaza al cambiar filtros.
    pub candidates: Vec<TopicCandidate>,
    /// Mensaje de status / errores en el footer.
    pub status: String,
    pub open: bool,
}
