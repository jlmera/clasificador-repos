//! Configuración editable de categorías + keywords + topic boosts.
//!
//! Vive en `data/categorias.json`. Se carga al inicio de cada operación
//! que necesita clasificar o iterar categorías. Si el archivo no existe,
//! se crea con los valores `const` hardcoded de `categories.rs` como
//! semilla — así el usuario tiene un punto de partida para editar.
//!
//! Schema (JSON):
//! ```
//! {
//!   "version": 1,
//!   "categorias": [
//!     {
//!       "id": "01-claude-code",
//!       "keywords":     [["claude code", 5], ["anthropic", 3], …],
//!       "topic_boosts": [["claude-code", 10], ["mcp-server", 8], …]
//!     },
//!     …
//!   ]
//! }
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::atomic_io::write_atomic_string;
use crate::categories::{CATEGORIAS, KEYWORDS, TOPIC_BOOSTS};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoriaDef {
    /// ID único = nombre de carpeta física (ej. "01-claude-code").
    /// Cambiar el id implica renombrar la carpeta — la app no lo hace
    /// automáticamente todavía (ver Fase 3b/3c).
    pub id: String,
    /// Keywords con peso (1-10) que se matchean contra el blob de texto
    /// del repo (nombre + descripción + primeros 1500 chars del README).
    #[serde(default)]
    pub keywords: Vec<(String, u32)>,
    /// Topics oficiales de GitHub (lowercase) con peso (1-15).
    /// Match exacto contra cada elemento del array `topics` de la API.
    #[serde(default)]
    pub topic_boosts: Vec<(String, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoriasConfig {
    /// Versión del schema. Bumpeamos si cambia la estructura interna.
    pub version: u32,
    pub categorias: Vec<CategoriaDef>,
}

impl CategoriasConfig {
    /// Lista de IDs de categorías en orden, útil para iterar carpetas.
    pub fn ids(&self) -> Vec<String> {
        self.categorias.iter().map(|c| c.id.clone()).collect()
    }

    /// Categoría usada como fallback cuando ningún score > 0.
    /// Por convención usamos la última de la lista (típicamente
    /// "10-utilidades-dev"). Si la lista está vacía, retorna "_sin_categoria".
    pub fn fallback_id(&self) -> String {
        self.categorias.last()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| "_sin_categoria".to_string())
    }

    /// Construye una config por defecto convirtiendo los const hardcoded
    /// (`CATEGORIAS` + `KEYWORDS` + `TOPIC_BOOSTS`) en estructuras editables.
    pub fn from_hardcoded() -> Self {
        let mut categorias = Vec::with_capacity(CATEGORIAS.len());
        for cat_id in CATEGORIAS {
            let keywords: Vec<(String, u32)> = KEYWORDS.iter()
                .find(|(c, _)| *c == *cat_id)
                .map(|(_, kws)| kws.iter().map(|(k, w)| (k.to_string(), *w)).collect())
                .unwrap_or_default();
            let topic_boosts: Vec<(String, u32)> = TOPIC_BOOSTS.iter()
                .filter(|(_, c, _)| *c == *cat_id)
                .map(|(t, _, w)| (t.to_string(), *w))
                .collect();
            categorias.push(CategoriaDef {
                id: (*cat_id).to_string(),
                keywords,
                topic_boosts,
            });
        }
        Self { version: 1, categorias }
    }
}

/// Path absoluto del archivo de config: <root>/data/categorias.json
pub fn config_path(root: &Path) -> PathBuf {
    root.join("data").join("categorias.json")
}

/// Carga el config desde disco. Si el archivo no existe o falla el
/// parseo, retorna los defaults hardcoded Y los persiste — así el
/// archivo queda creado para que el usuario lo pueda editar.
pub fn load_or_default(root: &Path) -> CategoriasConfig {
    let p = config_path(root);
    if p.exists() {
        if let Ok(text) = fs::read_to_string(&p) {
            if let Ok(cfg) = serde_json::from_str::<CategoriasConfig>(&text) {
                return cfg;
            }
        }
    }
    let cfg = CategoriasConfig::from_hardcoded();
    let _ = save(root, &cfg);
    cfg
}

/// Persiste el config a disco como JSON pretty, de forma ATÓMICA
/// (write-to-tmp + fsync + rename) para evitar corrupciones por escrituras
/// concurrentes (Syncthing, antivirus, otra instancia de la app).
pub fn save(root: &Path, cfg: &CategoriasConfig) -> Result<()> {
    let p = config_path(root);
    let json = serde_json::to_string_pretty(cfg)?;
    write_atomic_string(&p, &json)?;
    Ok(())
}
