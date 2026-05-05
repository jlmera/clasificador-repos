//! clasificador — Clasificador de repositorios (versión definitiva)
//!
//! Estructura modular. Cada repo en `D:\DUGOTEX\11 - IA\GitHub\` se escanea,
//! se clasifica por heurística + LLM (opcional) y se indexa en
//! `tools/repos_index.json`.

pub mod categories;
pub mod categories_config;
pub mod scan;
pub mod classify;
pub mod ids;
pub mod duplicates;
pub mod index;
pub mod html;
pub mod readme;
pub mod paths;
pub mod moves;

pub mod apply_actions;
pub mod reclassify;
pub mod secrets;
pub mod llm;
pub mod wiki;
pub mod atomic_io;
pub mod topic_discovery;
pub mod gui;
