//! GUI con eframe + egui (port del clasificador_gui.py de tkinter).
//! Tema verde sage idéntico al Python actual.
//!
//! Estructura modular (refactorizado en B5.2):
//!   - `theme`   — paleta de colores constante
//!   - `types`   — DialogStates + WorkerMsg + DuplicateItem + CatRow
//!   - `helpers` — funciones libres reutilizables (validate, count_repos, open_url)
//!   - `workers` — los 4 apply_X_worker que corren en thread::spawn
//!   - `app`     — ClasificadorApp + impl + update() (render principal)

pub mod theme;
pub mod types;
pub mod helpers;
pub mod workers;
pub mod app;

pub use app::ClasificadorApp;
