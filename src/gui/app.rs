//! App principal de la GUI con scan / apply / resolver-duplicados implementados.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use egui::{Color32, RichText, ScrollArea};

use crate::apply_actions::{
    gather_compare_info, move_repo, DupAction,
};
use crate::atomic_io::write_atomic_string;
use crate::classify::{classify_heuristic, GhClassifyHints};
use crate::duplicates::{find_duplicate, get_git_remote_url, index_existing_repos, DupReason};
use crate::gui::helpers::{count_repos_per_category, open_url, validate_and_diff_cats};
use crate::gui::theme;
use crate::gui::types::{
    ApiKeyDialogState, CatRow, CatsEditorDialogState, DiscoverDialogState,
    DuplicateItem, DuplicatesDialogState, ReclassDialogState, WorkerMsg,
};
use crate::gui::workers::{
    apply_cats_worker, apply_discover_worker, apply_duplicates_worker,
    apply_reclass_worker,
};
use crate::html::generate_html;
use crate::index::{rebuild_index, RebuildOpts};
use crate::categories_config::{
    load_or_default as load_cats_cfg,
    CategoriaDef, CategoriasConfig,
};
use crate::ids::{
    fetch_github_meta, load_repo_ids, parse_github_owner_repo, test_github_pat,
};
use crate::index::load_descripciones_es;
use crate::llm::{extract_short_desc_from_md, render_readme_html_es, test_api_key, translate_short_desc};
use crate::moves::find_inbox_repos;
use crate::paths::{buscador_html_path, descripciones_path, repo_ids_path};
use crate::reclassify::{compute_reclassification, ReclassifyChange};
use crate::topic_discovery::{
    discover_candidates, topics_covered, DiscoverFilters, TopicCandidate,
};
use crate::scan::scan_repo;
use crate::secrets::{
    config_path, delete_api_key, has_api_key, has_github_pat,
    load_api_key, load_github_pat, save_api_key, save_github_pat,
};
use crate::wiki::generate_obsidian_wiki;

/// PNG de la bandera de Colombia (60x40, ~150 bytes), embebido al compilar.
/// Se renderiza en el botón "Traducir READMEs" porque egui no compone los
/// regional indicators del emoji 🇨🇴 con la fuente NotoEmoji-Regular default
/// (lo dibuja como las dos letras "CO").
const BANDERA_CO_PNG: &[u8] = include_bytes!("../../bandera_co.png");

pub struct ClasificadorApp {
    root: String,
    threshold: f32,
    use_llm: bool,
    log_lines: Vec<(String, Color32)>,
    status: String,
    progress: Option<(usize, usize)>,
    busy: bool,
    rx: Option<Receiver<WorkerMsg>>,
    dups_dialog:     Option<DuplicatesDialogState>,
    reclass_dialog:  Option<ReclassDialogState>,
    api_dialog:      Option<ApiKeyDialogState>,
    cats_dialog:     Option<CatsEditorDialogState>,
    discover_dialog: Option<DiscoverDialogState>,
    /// Textura cacheada de la bandera de Colombia. Se carga lazily en el
    /// primer frame y se reusa el resto de la sesión.
    bandera_co_tex: Option<egui::TextureHandle>,
}

impl Default for ClasificadorApp {
    fn default() -> Self {
        Self {
            root: r"D:\DUGOTEX\11 - IA\GitHub".to_string(),
            threshold: 0.6,
            use_llm: has_api_key(),
            log_lines: vec![(
                "Listo. Pulsa una acción para empezar.".to_string(),
                theme::MUTED,
            )],
            status: "Listo. Selecciona una acción.".to_string(),
            progress: None,
            busy: false,
            rx: None,
            dups_dialog:     None,
            reclass_dialog:  None,
            api_dialog:      None,
            cats_dialog:     None,
            discover_dialog: None,
            bandera_co_tex:  None,
        }
    }
}

impl ClasificadorApp {
    fn append_log(&mut self, text: impl Into<String>, color: Color32) {
        self.log_lines.push((text.into(), color));
        if self.log_lines.len() > 1500 {
            let drop = self.log_lines.len() - 1500;
            self.log_lines.drain(0..drop);
        }
    }

    fn start_action(&mut self, action: &'static str) {
        if self.busy {
            self.append_log("⚠ Ya hay una tarea corriendo.", theme::WARN);
            return;
        }
        let (tx, rx) = channel::<WorkerMsg>();
        self.rx = Some(rx);
        self.busy = true;
        self.progress = None;
        self.status = format!("Ejecutando: {}", action);
        let root = PathBuf::from(self.root.clone());
        let allow_net = self.use_llm;
        let threshold = self.threshold as f64;

        thread::spawn(move || run_worker(action, root, allow_net, threshold, tx));
    }

    /// Variante de start_action para aplicar reclasificación: además del worker
    /// se le pasa la lista de cambios ya seleccionados por el usuario en el
    /// diálogo. Cierra el diálogo inmediatamente y dispara el trabajo en
    /// background — la GUI no se congela y el log muestra el avance.
    fn start_apply_reclass(&mut self, changes: Vec<ReclassifyChange>) {
        if self.busy {
            self.append_log("⚠ Ya hay una tarea corriendo.", theme::WARN);
            return;
        }
        let count_sel = changes.iter().filter(|c| c.selected).count();
        let (tx, rx) = channel::<WorkerMsg>();
        self.rx = Some(rx);
        self.busy = true;
        self.progress = None;
        self.status = format!("Aplicando {} reclasificaciones…", count_sel);
        let root = PathBuf::from(self.root.clone());

        // Mensaje INSTANTÁNEO al log para que el usuario vea actividad
        // antes incluso de que arranque el worker. Sin esto el log se
        // mantiene mudo durante los ~50ms de spawn del thread.
        self.append_log(
            format!("━━━ Iniciando reclasificación de {} repos ━━━", count_sel),
            theme::ACCENT_H,
        );

        thread::spawn(move || apply_reclass_worker(root, changes, tx));
    }

    /// Variante de start_action para aplicar las decisiones del diálogo de
    /// duplicados. Misma filosofía que `start_apply_reclass`:
    /// 1) Cierra el diálogo inmediatamente (el caller hace `dups_dialog.take()`).
    /// 2) Imprime un log instantáneo para que el usuario vea actividad.
    /// 3) Dispara el trabajo (mover/borrar repos + reindex + regenerar HTML)
    ///    en un worker thread para no congelar la GUI — crítico cuando hay
    ///    repos grandes (gigas) cuya operación de borrado tarda varios segundos.
    fn start_apply_duplicates(&mut self, items: Vec<DuplicateItem>) {
        if self.busy {
            self.append_log("⚠ Ya hay una tarea corriendo.", theme::WARN);
            return;
        }
        let count = items.len();
        let (tx, rx) = channel::<WorkerMsg>();
        self.rx = Some(rx);
        self.busy = true;
        self.progress = None;
        self.status = format!("Aplicando {} decisiones de duplicados…", count);
        let root = PathBuf::from(self.root.clone());

        // Mensaje INSTANTÁNEO al log para feedback visual antes incluso del
        // spawn del thread. Sin esto el usuario no percibe el click en repos
        // pesados (la barra indeterminada tarda ~1 frame en pintarse).
        self.append_log("", theme::FG);
        self.append_log(
            format!("━━━ Aplicando {} decisiones ━━━", count),
            theme::ACCENT_H,
        );

        thread::spawn(move || apply_duplicates_worker(root, items, tx));
    }

    fn poll_worker(&mut self) {
        let mut messages: Vec<WorkerMsg> = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(msg) = rx.try_recv() {
                messages.push(msg);
            }
        }
        let mut done = false;
        for msg in messages {
            match msg {
                WorkerMsg::Log(t, c)               => self.append_log(t, c),
                WorkerMsg::Status(s)               => self.status = s,
                WorkerMsg::Progress(p)             => self.progress = p,
                WorkerMsg::OpenDuplicatesDialog(d) => {
                    self.dups_dialog = Some(DuplicatesDialogState { items: d, open: true });
                }
                WorkerMsg::OpenReclassifyDialog(c) => {
                    self.reclass_dialog = Some(ReclassDialogState { changes: c, open: true });
                }
                WorkerMsg::Done                    => done = true,
            }
        }
        if done {
            self.busy = false;
            self.rx = None;
        }
    }

    /// Valida el estado del editor de categorías y, si está OK, dispara el
    /// worker que persiste cambios (renombres + borrados + escritura de JSON
    /// + reindex). Si hay errores de validación, deja el status del diálogo
    /// en rojo y NO cierra. El usuario puede corregir y volver a pulsar Guardar.
    ///
    /// La validación + diff vive en `validate_and_diff_cats` (función libre)
    /// para evitar conflictos del borrow checker entre `&mut self.cats_dialog`
    /// y los demás campos de `self` que necesito tocar después.
    fn try_save_cats_editor(&mut self) {
        // 1) Validación + diff dentro del scope del borrow mutable de cats_dialog.
        //    El bloque devuelve los datos extraídos (Option) y suelta el borrow.
        let computed = match self.cats_dialog.as_mut() {
            Some(dlg) => validate_and_diff_cats(dlg),
            None      => return,
        };
        let (cfg, renames, deletes, new_ids, summary_msg) = match computed {
            Some(tup) => tup,
            None      => return, // validación falló, status ya está actualizado
        };
        // 2) Borrow ya soltado — podemos tocar self libremente.
        let root = PathBuf::from(self.root.clone());
        self.cats_dialog = None;
        self.append_log("", theme::FG);
        self.append_log(summary_msg, theme::ACCENT_H);
        self.start_apply_cats_changes(root, cfg, renames, deletes, new_ids);
    }

    /// Spawn del worker que aplica cambios del editor de categorías.
    /// Sigue el mismo patrón que `start_apply_reclass` / `start_apply_duplicates`.
    fn start_apply_cats_changes(
        &mut self,
        root:    PathBuf,
        cfg:     CategoriasConfig,
        renames: Vec<(String, String)>,
        deletes: Vec<String>,
        new_ids: Vec<String>,
    ) {
        if self.busy {
            self.append_log("⚠ Ya hay una tarea corriendo.", theme::WARN);
            return;
        }
        let (tx, rx) = channel::<WorkerMsg>();
        self.rx = Some(rx);
        self.busy = true;
        self.progress = None;
        self.status = "Aplicando cambios del editor de categorías…".to_string();
        thread::spawn(move ||
            apply_cats_worker(root, cfg, renames, deletes, new_ids, tx)
        );
    }

    /// Abre el diálogo "Editor de categorías" con un snapshot del config
    /// actual + el conteo físico de repos por carpeta. La copia es mutable;
    /// los cambios solo se persisten al pulsar 💾 Guardar.
    fn open_cats_editor(&mut self) {
        let root = std::path::PathBuf::from(&self.root);
        let cfg = load_cats_cfg(&root);
        let counts = count_repos_per_category(&root);
        let original_ids: Vec<String> = cfg.categorias.iter()
            .map(|c| c.id.clone())
            .collect();
        let rows: Vec<CatRow> = cfg.categorias.into_iter()
            .map(CatRow::from_existing)
            .collect();
        let selected_idx = if rows.is_empty() { None } else { Some(0) };
        self.cats_dialog = Some(CatsEditorDialogState {
            version:     cfg.version,
            original_ids,
            rows,
            repo_counts: counts,
            selected_idx,
            status:      String::new(),
            open:        true,
        });
    }

    /// Spawn del worker que aplica los candidatos seleccionados del
    /// descubridor: persiste las nuevas categorías al config + crea
    /// las carpetas + reclasifica todo el árbol.
    fn start_apply_discover(&mut self, chosen: Vec<TopicCandidate>) {
        if self.busy {
            self.append_log("⚠ Ya hay una tarea corriendo.", theme::WARN);
            return;
        }
        let count = chosen.len();
        if count == 0 { return; }
        let (tx, rx) = channel::<WorkerMsg>();
        self.rx = Some(rx);
        self.busy = true;
        self.progress = None;
        self.status = format!("Creando {} categorías + reclasificando…", count);
        let root = PathBuf::from(self.root.clone());

        // Mensaje instantáneo al log para feedback inmediato.
        self.append_log("", theme::FG);
        self.append_log(
            format!("━━━ Descubridor: creando {} categorías nuevas ━━━", count),
            theme::ACCENT_H,
        );

        thread::spawn(move || apply_discover_worker(root, chosen, tx));
    }

    /// Abre el diálogo "🔍 Descubrir categorías" cargando un snapshot
    /// del cache de repos (`repo_ids.json`) y del config actual de
    /// categorías. Si el cache está vacío o no existe, avisa al usuario
    /// con un log y no abre el diálogo.
    fn open_discover_dialog(&mut self) {
        let root = std::path::PathBuf::from(&self.root);
        let repo_ids = load_repo_ids(&repo_ids_path(&root));
        if repo_ids.is_empty() {
            self.append_log(
                "✗ data/repo_ids.json está vacío o no existe. \
                 Ejecutá '🆔 Refrescar GitHub IDs' primero para que la \
                 GUI tenga datos de topics.",
                theme::ERR,
            );
            return;
        }
        let cfg = load_cats_cfg(&root);
        let cubiertos = topics_covered(&cfg);
        let next_consecutivo = cfg.categorias.len() + 1;
        let filters = DiscoverFilters::default();
        let candidates = discover_candidates(
            &repo_ids, &cubiertos, filters, next_consecutivo,
        );
        let status = format!(
            "Encontrados {} candidatos · {} repos cacheados · {} topics ya cubiertos",
            candidates.len(), repo_ids.len(), cubiertos.len()
        );
        self.discover_dialog = Some(DiscoverDialogState {
            source_repo_ids:  repo_ids,
            source_cubiertos: cubiertos,
            next_consecutivo,
            filters,
            candidates,
            status,
            open: true,
        });
    }

    fn open_buscador(&mut self) {
        let path = buscador_html_path(&PathBuf::from(&self.root));
        if !path.exists() {
            self.append_log(
                format!("✗ No existe {}. Ejecuta 'Solo reindexar' primero.", path.display()),
                theme::ERR,
            );
            return;
        }
        let url = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
        match open_url(&url) {
            Ok(_)  => self.append_log(format!("🌐 Abierto: {}", path.display()), theme::OK),
            Err(e) => self.append_log(format!("✗ No se pudo abrir: {}", e), theme::ERR),
        }
    }

}

// Nota: validate_and_diff_cats, count_repos_per_category y open_url
// vivían acá hasta el refactor B5.2. Ahora viven en gui/helpers.rs.

fn run_worker(
    action: &'static str,
    root: PathBuf,
    allow_net: bool,
    threshold: f64,
    tx: Sender<WorkerMsg>,
) {
    macro_rules! log {
        ($t:expr, $c:expr) => {{
            let _ = tx.send(WorkerMsg::Log(($t).to_string(), $c));
        }};
    }
    macro_rules! status {
        ($t:expr) => {{
            let _ = tx.send(WorkerMsg::Status(($t).to_string()));
        }};
    }
    macro_rules! progress {
        ($p:expr) => {{
            let _ = tx.send(WorkerMsg::Progress($p));
        }};
    }

    log!(format!("=== Acción: {} ===", action), theme::ACCENT_H);
    log!(format!("  root: {}", root.display()), theme::MUTED);
    log!(format!("  threshold: {:.2}   LLM: {}", threshold,
                 if allow_net { "sí" } else { "no" }), theme::MUTED);

    match action {
        "reindex" => {
            status!("Re-escaneando árbol…");
            progress!(None);
            let t0 = Instant::now();
            match rebuild_index(&root, RebuildOpts {
                allow_github: false,
                force_github_retry: false,
                github_pat: None,
            }) {
                Ok(data) => {
                    log!(format!("  ✓ {} repos indexados", data.len()), theme::OK);
                    if let Ok(p) = generate_html(&root, &data) {
                        log!(format!("  ✓ HTML: {}", p.display()), theme::OK);
                    }
                    log!(format!("✓ Reindex completo en {:.1}s",
                                  t0.elapsed().as_secs_f64()), theme::ACCENT_H);
                    status!(format!("OK · {} repos", data.len()));
                }
                Err(e) => { log!(format!("✗ Error: {}", e), theme::ERR); status!("Error"); }
            }
        }
        "scan" | "apply" => {
            let inbox = root.join("_inbox");
            if !inbox.exists() {
                log!(format!("⚠ No existe {}", inbox.display()), theme::WARN);
                status!("Sin _inbox"); let _ = tx.send(WorkerMsg::Done); return;
            }
            status!("Escaneando _inbox…");
            let candidates = find_inbox_repos(&inbox);
            if candidates.is_empty() {
                log!("⚠ _inbox vacío. Nada que clasificar.", theme::WARN);
                status!("Inbox vacío"); let _ = tx.send(WorkerMsg::Done); return;
            }
            log!(format!("Encontrados {} repo(s) candidatos", candidates.len()), theme::ACCENT_H);
            log!("Indexando repos ya categorizados…", theme::MUTED);
            let existing = index_existing_repos(&root);
            log!(format!("  → {} en árbol", existing.by_name.len()), theme::MUTED);

            // Pre-cargar PAT + cache + config de categorías para clasificar.
            // El config viene de data/categorias.json (fallback hardcoded).
            let scan_pat = load_github_pat();
            let scan_ids_cache = load_repo_ids(&repo_ids_path(&root));
            let scan_cats_cfg = load_cats_cfg(&root);
            log!(format!("  Categorías activas: {} (en data/categorias.json)",
                          scan_cats_cfg.categorias.len()), theme::MUTED);
            if scan_pat.is_some() {
                log!("  PAT activo: se consultarán topics+description de GitHub para cada candidato.",
                     theme::MUTED);
            } else {
                log!("  Sin PAT: clasificación solo con heurística local + manifests.",
                     theme::MUTED);
            }

            // Estructura: cada candidato → propuesta (cat, conf, dup?)
            struct Prop { repo: PathBuf, name: String, cat: String, conf: f64,
                          dup: Option<DupReason> }
            let mut proposals: Vec<Prop> = Vec::new();

            progress!(Some((0, candidates.len())));
            for (i, repo) in candidates.iter().enumerate() {
                status!(format!("Analizando {}/{} · {}",
                                i+1, candidates.len(),
                                repo.file_name().unwrap_or_default().to_string_lossy()));
                let dup = find_duplicate(repo, &existing);
                match scan_repo(repo) {
                    Ok(scan) => {
                        // Resolver hints de GitHub para este repo:
                        //   1) Si está en cache (raro para inbox, pero posible) → usar.
                        //   2) Si no, intentar fetch directo SOLO si hay PAT.
                        //   3) Sin PAT → no consultamos (rate-limit anonymous 60/h
                        //      es muy frágil para procesar varios candidatos).
                        let cached = scan_ids_cache.get(&scan.name);
                        let mut owned_topics: Vec<String> = Vec::new();
                        let mut owned_desc: Option<String> = None;
                        if let Some(c) = cached {
                            owned_topics = c.topics.clone();
                            owned_desc   = c.description_en.clone();
                        }
                        if owned_topics.is_empty() && owned_desc.is_none() {
                            if let Some(pat) = scan_pat.as_deref() {
                                if let Some(raw) = get_git_remote_url(repo) {
                                    if let Some((owner, slug)) = parse_github_owner_repo(&raw) {
                                        // En el scan del _inbox no hay etag previo
                                        // (es un repo nuevo) → prev_etag = None.
                                        if let crate::ids::FetchOutcome::Updated(gh) =
                                            fetch_github_meta(&owner, &slug, Some(pat), None)
                                        {
                                            owned_topics = gh.topics;
                                            owned_desc   = gh.description;
                                        }
                                    }
                                }
                            }
                        }
                        let hints = GhClassifyHints {
                            description_en: owned_desc.as_deref(),
                            topics:         &owned_topics,
                        };
                        let cls = classify_heuristic(&scan, Some(&hints), &scan_cats_cfg);
                        let cat_color = if cls.confidence >= threshold {
                            theme::OK
                        } else if cls.confidence >= 0.3 { theme::WARN } else { theme::ERR };
                        if let Some(d) = &dup {
                            let (motivo, ruta) = match d {
                                DupReason::MismoNombre(p) => ("mismo_nombre", p.clone()),
                                DupReason::MismaUrl(p)    => ("misma_url", p.clone()),
                            };
                            log!(format!("  🚫 {}  DUPLICADO ({}) → {}",
                                          scan.name, motivo, ruta.display()),
                                 theme::ERR);
                        } else {
                            log!(format!("  • {}  [{}, stack={}]",
                                          scan.name, scan.lenguaje_principal,
                                          if scan.stack.is_empty() { "—".to_string() }
                                          else { scan.stack.join(",") }),
                                 theme::FG);
                            log!(format!("      → {}  conf={:.2}",
                                          cls.categoria, cls.confidence),
                                 cat_color);
                            // OJO: `&str[..N]` panica si N cae dentro de un
                            // caracter multi-byte (emoji, CJK, acento extendido).
                            // chars().take(N) cuenta caracteres, nunca falla.
                            let preview: String = if scan.descripcion_en.chars().count() > 200 {
                                let head: String = scan.descripcion_en.chars().take(200).collect();
                                format!("{}…", head)
                            } else {
                                scan.descripcion_en.clone()
                            };
                            log!(format!("      {}", preview), theme::MUTED);
                        }
                        proposals.push(Prop {
                            repo: repo.clone(),
                            name: scan.name.clone(),
                            cat:  cls.categoria,
                            conf: cls.confidence,
                            dup,
                        });
                    }
                    Err(e) => log!(format!("  ✗ scan {} falló: {}",
                                            repo.display(), e), theme::ERR),
                }
                progress!(Some((i+1, candidates.len())));
            }

            if action == "scan" {
                let dups = proposals.iter().filter(|p| p.dup.is_some()).count();
                let baja = proposals.iter().filter(|p| p.dup.is_none() && p.conf < threshold).count();
                let ok = proposals.len() - dups - baja;
                log!("", theme::FG);
                log!(format!("Resumen dry-run: {} repos analizados",
                              proposals.len()), theme::ACCENT_H);
                log!(format!("  ✓ listos para mover: {}", ok),    theme::OK);
                log!(format!("  ⏭ baja confianza:    {}", baja),  theme::WARN);
                log!(format!("  🚫 duplicados:        {}", dups),  theme::ERR);
                log!("Para aplicar usa el botón 'Aplicar (mover)'.", theme::MUTED);
                status!(format!("Dry-run · {} listos · {} dudosos · {} dup", ok, baja, dups));
                let _ = tx.send(WorkerMsg::Done);
                return;
            }

            // ── APPLY ────────────────────────────────────────────────
            log!("", theme::FG);
            log!("Aplicando movimientos…", theme::ACCENT_H);
            let mut moved = 0; let mut skip = 0; let mut dup_n = 0; let mut err_n = 0;
            for p in &proposals {
                if p.dup.is_some() {
                    log!(format!("  🚫 DUP    {}", p.name), theme::ERR);
                    dup_n += 1; continue;
                }
                if p.conf < threshold {
                    log!(format!("  ⏭ SKIP    {}  (conf {:.2})", p.name, p.conf), theme::WARN);
                    skip += 1; continue;
                }
                match move_repo(&p.repo, &root, &p.cat) {
                    Ok(_) => { log!(format!("  ✓ MOVED  {}  →  {}/", p.name, p.cat), theme::OK); moved += 1; }
                    Err(e) => { log!(format!("  ✗ ERROR  {}: {}", p.name, e), theme::ERR); err_n += 1; }
                }
            }

            // Reindex automático tras Apply, consultando GitHub para los repos nuevos.
            // No forzamos reintento (force_github_retry=false) → solo consulta los repos
            // sin ID en cache; los que tienen ID se mantienen sin nueva llamada.
            // Rate-limit anonymous: 60 req/h.
            status!("Reindexando + consultando GitHub IDs…");
            progress!(None);
            log!("Re-escaneando árbol y resolviendo GitHub IDs nuevos…", theme::MUTED);
            log!("  (los repos que ya tienen ID NO se vuelven a consultar)", theme::MUTED);
            // Cargar el PAT cifrado para usarlo en las requests HTTP. Si no
            // hay PAT, queda en None → modo anonymous con rate-limit 60/h.
            let pat_apply = load_github_pat();
            if pat_apply.is_some() {
                log!("  Autenticando con PAT (rate-limit 5000/h).", theme::MUTED);
            }
            let t0 = Instant::now();
            match rebuild_index(&root, RebuildOpts {
                allow_github: true,
                force_github_retry: false,
                github_pat: pat_apply,
            }) {
                Ok(data) => {
                    let _ = generate_html(&root, &data);
                    let con_id = data.iter().filter(|r| r.id.is_some()).count();
                    let sin_id = data.len() - con_id;
                    log!(format!("  ✓ {} repos · {} con ID · {} sin ID  ({:.1}s)",
                                  data.len(), con_id, sin_id, t0.elapsed().as_secs_f64()),
                         theme::OK);
                    if sin_id > 0 {
                        log!(format!("  ⚠ {} sin ID (rate-limit, repo privado o no-GitHub) — \
                                       reintento manual con 'Refrescar GitHub IDs'", sin_id),
                             theme::WARN);
                    }
                }
                Err(e) => log!(format!("  ✗ reindex: {}", e), theme::ERR),
            }
            log!("", theme::FG);
            log!(format!("Resumen: ✓{} movidos · ⏭{} saltados · 🚫{} dup · ✗{} errores",
                         moved, skip, dup_n, err_n), theme::ACCENT_H);
            log!("  Traducción de READMEs y Wiki Obsidian → botones aparte.", theme::MUTED);
            status!(format!("Apply · {} movidos · {} dup", moved, dup_n));
        }
        "resolve_dups" => {
            let inbox = root.join("_inbox");
            if !inbox.exists() {
                log!(format!("⚠ No existe {}", inbox.display()), theme::WARN);
                status!("Sin _inbox"); let _ = tx.send(WorkerMsg::Done); return;
            }
            status!("Detectando duplicados…");
            let existing = index_existing_repos(&root);
            let candidates = find_inbox_repos(&inbox);
            let mut items: Vec<DuplicateItem> = Vec::new();
            for repo in candidates {
                if let Some(d) = find_duplicate(&repo, &existing) {
                    let (motivo, old_path) = match d {
                        DupReason::MismoNombre(p) => ("mismo_nombre".to_string(), p),
                        DupReason::MismaUrl(p)    => ("misma_url".to_string(), p),
                    };
                    let new_info = gather_compare_info(&repo);
                    let old_info = gather_compare_info(&old_path);
                    let new_name_default = format!("{}-fork", new_info.name);
                    items.push(DuplicateItem {
                        new_path: repo,
                        old_path,
                        motivo,
                        new_info,
                        old_info,
                        action: DupAction::Archive,
                        new_name: new_name_default,
                    });
                }
            }
            if items.is_empty() {
                log!("✓ No hay duplicados en _inbox.", theme::OK);
                status!("Sin duplicados"); let _ = tx.send(WorkerMsg::Done); return;
            }
            log!(format!("Encontrados {} duplicado(s). Abriendo diálogo…", items.len()),
                 theme::ACCENT_H);
            let _ = tx.send(WorkerMsg::OpenDuplicatesDialog(items));
            status!("Diálogo de duplicados abierto");
        }
        "refresh_github" => {
            status!("Refrescando GitHub: id + description + topics…");
            log!("Re-consultando TODOS los repos a GitHub API con If-None-Match.", theme::MUTED);
            log!("  Cada repo manda su ETag previo; si no cambió, GitHub responde 304", theme::MUTED);
            log!("  (no consume rate limit). Solo se baja metadata real para los repos", theme::MUTED);
            log!("  cuyo upstream cambió desde el último refresh.", theme::MUTED);
            // Cargar PAT (si existe) — eleva rate-limit a 5000/h.
            let pat_refresh = load_github_pat();
            if pat_refresh.is_some() {
                log!("  ✓ Autenticando con PAT (rate-limit 5000/h).", theme::ACCENT_H);
            } else {
                log!("  ⚠ Sin PAT: la GitHub API anonymous tiene 60 req/hora.", theme::WARN);
                log!("    Con 146 repos saturarás el límite — configura un PAT en ⚙ API Key.", theme::MUTED);
            }
            progress!(None);
            let t0 = Instant::now();
            match crate::index::rebuild_index_with_stats(&root, RebuildOpts {
                allow_github: true,
                force_github_retry: true,
                github_pat: pat_refresh,
            }) {
                Ok((data, gh_stats)) => {
                    let con_id = data.iter().filter(|r| r.id.is_some()).count();
                    let sin_id = data.len() - con_id;
                    log!(format!("  ✓ {} repos · {} con ID · {} sin ID",
                                  data.len(), con_id, sin_id), theme::OK);
                    // ── Stats del flow ETag ──
                    log!(format!(
                        "  📡 GitHub API: {} cambiados · {} sin cambios (304) · {} fallaron · {} skipped",
                        gh_stats.fetched, gh_stats.not_modified,
                        gh_stats.failed, gh_stats.skipped
                    ), theme::ACCENT_H);
                    if gh_stats.not_modified > 0 {
                        let pct = (gh_stats.not_modified as f32
                            / (gh_stats.fetched + gh_stats.not_modified).max(1) as f32) * 100.0;
                        log!(format!(
                            "  💡 {:.0}% de los requests volvieron 304 → no consumieron \
                             rate limit. Cada vez que refresques los repos estables se \
                             saltean automáticamente.",
                            pct
                        ), theme::MUTED);
                    }
                    if let Ok(p) = generate_html(&root, &data) {
                        log!(format!("  ✓ HTML: {}", p.display()), theme::OK);
                    }
                    if sin_id > 0 {
                        log!(format!("  ⚠ {} repos quedan sin ID (rate limit, repos privados o no-GitHub)",
                                      sin_id), theme::WARN);
                        log!("  Esos repos no se reintentarán durante las próximas 24h.", theme::MUTED);
                    }
                    log!(format!("✓ Refresh completo en {:.1}s",
                                  t0.elapsed().as_secs_f64()), theme::ACCENT_H);
                    status!(format!("OK · {} cambiados · {} sin cambios · {} con ID",
                                    gh_stats.fetched, gh_stats.not_modified, con_id));
                }
                Err(e) => { log!(format!("✗ Error: {}", e), theme::ERR); status!("Error"); }
            }
        }
        "reclassify" => {
            // Re-aplicar el algoritmo de clasificación a TODOS los repos
            // ya categorizados, según el config actual de data/categorias.json.
            // Solo computa: muestra dialog y deja al usuario decidir qué aplicar.
            status!("Reclasificando repos categorizados…");
            log!("Reclasificación masiva con el config actual de data/categorias.json", theme::ACCENT_H);
            log!("  Hints de GitHub (description+topics) se leen del cache.", theme::MUTED);
            log!("  Para datos frescos, ejecuta primero '🆔 Refrescar GitHub IDs'.", theme::MUTED);
            progress!(None);
            let t0 = Instant::now();
            match compute_reclassification(&root) {
                Ok(changes) => {
                    log!(format!("  ✓ Análisis completo en {:.1}s", t0.elapsed().as_secs_f64()),
                         theme::OK);
                    if changes.is_empty() {
                        log!("  ✓ Sin cambios — todos los repos ya están en su categoría óptima.",
                             theme::ACCENT_H);
                        status!("Sin cambios");
                    } else {
                        log!(format!("  → {} repo(s) cambiarían de categoría — abriendo diálogo…",
                                      changes.len()), theme::ACCENT_H);
                        let _ = tx.send(WorkerMsg::OpenReclassifyDialog(changes));
                        status!(format!("Diálogo abierto · revisar cambios"));
                    }
                }
                Err(e) => {
                    log!(format!("  ✗ Error: {}", e), theme::ERR);
                    status!("Error en reclasificación");
                }
            }
        }
        "wiki" => {
            status!("Generando wiki Obsidian…");
            progress!(None);
            log!("Recargando índice y generando wiki…", theme::MUTED);
            match rebuild_index(&root, RebuildOpts {
                allow_github: false,
                force_github_retry: false,
                github_pat: None,
            }) {
                Ok(data) => {
                    log!(format!("  → {} repos en data", data.len()), theme::OK);
                    match generate_obsidian_wiki(&root, &data) {
                        Ok(p) => {
                            log!(format!("  ✓ Wiki generada en: {}",
                                          p.parent().map(|p| p.display().to_string())
                                            .unwrap_or_default()), theme::OK);
                            log!(format!("  Abre {} en Obsidian para navegarla.",
                                          p.display()), theme::MUTED);
                            status!(format!("Wiki OK · {} repos", data.len()));
                        }
                        Err(e) => { log!(format!("✗ Wiki: {}", e), theme::ERR); }
                    }
                }
                Err(e) => { log!(format!("✗ index: {}", e), theme::ERR); }
            }
        }
        "translate" => {
            let api_key = load_api_key().unwrap_or_default();
            if api_key.is_empty() {
                log!("✗ No hay API key configurada. Pulsa ⚙ API Key primero.", theme::ERR);
                status!("Sin API key");
                let _ = tx.send(WorkerMsg::Done);
                return;
            }
            status!("Recopilando lista de repos…");
            log!("Recorriendo categorías para listar repos…", theme::MUTED);
            let mut repos: Vec<(PathBuf, String)> = Vec::new();
            // Categorías desde el config editable (data/categorias.json).
            let translate_cats_cfg = load_cats_cfg(&root);
            for cat_def in &translate_cats_cfg.categorias {
                let cat = cat_def.id.as_str();
                let cat_dir = root.join(cat);
                if !cat_dir.is_dir() { continue; }
                if let Ok(rd) = std::fs::read_dir(&cat_dir) {
                    let mut sub: Vec<_> = rd.flatten()
                        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                        .collect();
                    sub.sort_by_key(|e| e.file_name());
                    for entry in sub {
                        repos.push((entry.path(), cat.to_string()));
                    }
                }
            }
            if repos.is_empty() {
                log!("No hay repos categorizados.", theme::WARN);
                status!("Sin repos");
                let _ = tx.send(WorkerMsg::Done); return;
            }
            log!(format!("Total a procesar: {} repos", repos.len()), theme::ACCENT_H);
            log!("Caché incremental: solo se re-traduce si el README cambió.",
                 theme::MUTED);
            log!("Solo se loguean los repos que se traducen; los cacheados se cuentan al final.",
                 theme::MUTED);
            log!("", theme::FG);
            // Cargar el dict actual de descripciones ES — vamos a actualizarlo
            // sobre la marcha y guardarlo al final. Así cada repo procesado
            // queda con su descripción corta en español en data/descripciones_es.json.
            let mut descripciones_es = load_descripciones_es(&root);
            let mut desc_actualizadas = 0u32;

            // ── PASADA 1: descripciones cortas desde GitHub API → traducir ──
            // Para cada repo cuyo `description_en` ya está cacheado en
            // `repo_ids.json` (la trae fetch_github_meta) y NO tiene aún
            // entrada en descripciones_es, pedimos a Claude una traducción
            // de UNA línea. Es la fuente más confiable de descripción:
            // viene del autor del repo, no parseada del README.
            let ids_cache = load_repo_ids(&repo_ids_path(&root));
            let pendientes_gh: Vec<(String, String)> = ids_cache.iter()
                .filter_map(|(name, m)| {
                    if descripciones_es.contains_key(name) { return None; }
                    m.description_en.as_ref().map(|en| (name.clone(), en.clone()))
                })
                .collect();
            if !pendientes_gh.is_empty() {
                log!(format!("Pasada 1/2 — Traduciendo {} descripciones desde GitHub API…",
                              pendientes_gh.len()), theme::ACCENT_H);
                let mut gh_ok = 0u32; let mut gh_err = 0u32;
                for (nombre, en) in &pendientes_gh {
                    log!(format!("  → desc {}", nombre), theme::MUTED);
                    match translate_short_desc(en, &api_key) {
                        Ok(es) => {
                            descripciones_es.insert(
                                nombre.clone(),
                                serde_json::Value::String(es),
                            );
                            desc_actualizadas += 1;
                            gh_ok += 1;
                        }
                        Err(e) => {
                            log!(format!("    ✗ {} (fallback al README luego): {}",
                                          nombre, e), theme::WARN);
                            gh_err += 1;
                        }
                    }
                }
                log!(format!("  ✓ {} descripciones traducidas desde GitHub · {} fallos",
                              gh_ok, gh_err), theme::OK);
                log!("", theme::FG);
            } else {
                log!("Pasada 1/2 — descripciones GitHub API: nada que traducir.",
                     theme::MUTED);
            }

            log!(format!("Pasada 2/2 — Procesando READMEs completos ({} repos)…",
                          repos.len()), theme::ACCENT_H);
            // Conteos separados: nuevos (paga API), cacheados (gratis), saltados, errores.
            let mut nuevos = 0; let mut cacheados = 0; let mut sk = 0; let mut errs = 0;
            progress!(Some((0, repos.len())));
            for (i, (repo, cat)) in repos.iter().enumerate() {
                let name = repo.file_name().map(|s| s.to_string_lossy().into_owned())
                              .unwrap_or_default();
                status!(format!("Traduciendo {}/{} · {}", i+1, repos.len(), name));
                let cache_md = repo.join("_README_es.md");
                let readme_orig = ["README.md","README.MD","Readme.md","readme.md","README"]
                    .iter()
                    .map(|c| repo.join(c))
                    .find(|p| p.exists());
                let Some(readme_orig) = readme_orig else {
                    log!(format!("  ⏭ {}  (sin README)", name), theme::MUTED);
                    sk += 1; progress!(Some((i+1, repos.len()))); continue;
                };
                // ¿Caché vigente? Si sí, render_readme_html_es no llama al LLM.
                let mut cache_fresh = false;
                if let (Ok(c), Ok(m)) = (std::fs::metadata(&cache_md),
                                         std::fs::metadata(&readme_orig)) {
                    if let (Ok(ct), Ok(mt)) = (c.modified(), m.modified()) {
                        if ct >= mt { cache_fresh = true; }
                    }
                }
                // Log "→ Traduciendo X..." ANTES de la llamada para los repos
                // que sí van a pegarle al LLM. Útil porque entre esa línea y
                // el "✓ traducido" pasan 30-240 s de espera real.
                if !cache_fresh {
                    log!(format!("  → Traduciendo {}…", name), theme::MUTED);
                }
                match render_readme_html_es(repo, cat, &root, &api_key, true, false) {
                    Ok(Some(_p)) => {
                        if cache_fresh {
                            // No logueamos cada caché reusada para no saturar
                            // el panel; el conteo va al resumen final.
                            cacheados += 1;
                        } else {
                            log!(format!("  ✓ {}  → traducido y guardado", name), theme::OK);
                            nuevos += 1;
                        }
                        // Sincronizar descripción corta ES desde el .md cacheado.
                        // No consume tokens: solo lee el archivo local y extrae
                        // las primeras líneas significativas. Inserta o reemplaza.
                        if let Some(short) = extract_short_desc_from_md(&cache_md) {
                            let prev = descripciones_es.get(&name).and_then(|v| v.as_str()).map(|s| s.to_string());
                            if prev.as_deref() != Some(short.as_str()) {
                                descripciones_es.insert(
                                    name.clone(),
                                    serde_json::Value::String(short),
                                );
                                desc_actualizadas += 1;
                            }
                        }
                    }
                    Ok(None) => {
                        log!(format!("  ✗ {}  → la API no devolvió traducción", name), theme::ERR);
                        errs += 1;
                    }
                    Err(e) => {
                        log!(format!("  ✗ {}  → ERROR: {}", name, e), theme::ERR);
                        errs += 1;
                    }
                }
                progress!(Some((i+1, repos.len())));
            }
            log!("", theme::FG);
            // Persistir el dict de descripciones ES si hubo cambios. Esto
            // alimenta repos_index.json en el siguiente rebuild.
            if desc_actualizadas > 0 {
                let path = descripciones_path(&root);
                match serde_json::to_string_pretty(&descripciones_es) {
                    Ok(j) => {
                        // Escritura atómica para evitar corrupción por
                        // race condition con Syncthing (ver atomic_io.rs).
                        if let Err(e) = write_atomic_string(&path, &j) {
                            log!(format!("  ✗ No se pudo guardar descripciones_es.json: {}", e), theme::ERR);
                        } else {
                            log!(format!("  ✓ {} descripciones ES sincronizadas en data/descripciones_es.json",
                                          desc_actualizadas), theme::OK);
                        }
                    }
                    Err(e) => {
                        log!(format!("  ✗ Serializando descripciones: {}", e), theme::ERR);
                    }
                }
            }
            log!("Regenerando índice y buscador.html…", theme::MUTED);
            progress!(None);
            if let Ok(data) = rebuild_index(&root, RebuildOpts {
                allow_github: false,
                force_github_retry: false,
                github_pat: None,
            }) {
                let _ = generate_html(&root, &data);
            }
            log!("", theme::FG);
            let ok = nuevos + cacheados;
            log!(format!(
                    "━━━ Traducciones: {} OK ({} nuevos · {} cache) · {} sin README · {} errores · {} desc ES sync ━━━",
                    ok, nuevos, cacheados, sk, errs, desc_actualizadas), theme::ACCENT_H);
            status!(format!("OK · {} nuevos · {} cache · {} desc ES",
                            nuevos, cacheados, desc_actualizadas));
        }
        _ => log!(format!("Acción '{}' aún no implementada", action), theme::WARN),
    }
    let _ = tx.send(WorkerMsg::Done);
}

// Nota: los 4 apply_X_worker (reclass, duplicates, cats, discover) y
// open_url se movieron a `gui/workers.rs` y `gui/helpers.rs` durante el
// refactor B5.2. Acá quedó solo run_worker (el grande de scan/apply/etc.)
// y el impl ClasificadorApp con los métodos.

impl eframe::App for ClasificadorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Carga lazy de la bandera de Colombia (PNG embebido) la primera vez.
        // Decodificamos a RGBA con `image` y subimos al GPU como TextureHandle.
        if self.bandera_co_tex.is_none() {
            if let Ok(img) = image::load_from_memory(BANDERA_CO_PNG) {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize], &rgba.into_raw());
                self.bandera_co_tex = Some(
                    ctx.load_texture("bandera_co", color_image, Default::default())
                );
            }
        }

        if self.busy {
            // Pintar a ~60fps mientras hay un worker corriendo, así la
            // animación del sheen / marquee se ve fluida sin necesidad de
            // mover el mouse. 16ms ≈ 60fps. El costo en CPU es despreciable
            // (la app está esperando a Anthropic / GitHub la mayor parte
            // del tiempo, no haciendo trabajo intensivo).
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        self.poll_worker();

        // ── LÍNEA SEPARADORA SUPERIOR ────────────────────────────
        // Banda verde fina pegada a la barra de título de Windows
        egui::TopBottomPanel::top("top_separator")
            .exact_height(3.0)
            .frame(egui::Frame::default().fill(theme::ACCENT))
            .show_separator_line(false)
            .show(ctx, |_ui| {});

        // ── HEADER ────────────────────────────────────────────────
        egui::TopBottomPanel::top("header")
            .exact_height(64.0)
            .frame(egui::Frame::default().fill(theme::PANEL).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // ── TÍTULO a la IZQUIERDA ─────────────────────
                    ui.vertical(|ui| {
                        // CLASIFICADOR en verde + negrita, " DE REPOSITORIOS" en color normal
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.label(RichText::new("CLASIFICADOR")
                                     .size(16.0).strong().color(theme::ACCENT_H));
                            ui.label(RichText::new(" DE REPOSITORIOS")
                                     .size(16.0).strong().color(theme::FG));
                        });
                        ui.label(RichText::new(&self.status)
                                 .size(11.5).color(theme::MUTED));
                    });

                    // ── SKYLINE de barras verticales a la DERECHA del título ──
                    // Histograma decorativo: barras verticales de altura
                    // pseudo-aleatoria pegadas a la base del header. Evoca
                    // datos clasificándose en buckets de tamaños distintos.
                    //
                    // Degradado de COLOR (no de alpha): todas las barras son
                    // sólidas (alpha 255), pero el color va del verde fuerte
                    // ACCENT_H (cerca del título) al verde claro pastel
                    // (borde derecho). Nunca se vuelve blanco — el extremo
                    // derecho sigue siendo perceptiblemente verde.
                    //
                    // PRNG inline (SplitMix64) con seed fijo → mismo skyline
                    // cada frame, sin flickering.
                    ui.add_space(20.0);
                    let avail_w = ui.available_width();
                    if avail_w > 30.0 {
                        let header_h = 40.0;
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(avail_w, header_h),
                            egui::Sense::hover(),
                        );
                        const BAR_W:      f32 = 5.0;
                        const BAR_GAP:    f32 = 2.0;
                        const HEIGHT_MIN: f32 = 0.40;
                        const HEIGHT_MAX: f32 = 1.00;
                        const SEED:       u64 = 0xC0FFEE_BABE_F00D;
                        const ROUND:      f32 = 1.0;
                        // Color de inicio (cerca del título): verde oscuro fuerte.
                        // ACCENT_H del tema = #496229 (RGB 73, 98, 41).
                        const C_START: (u8, u8, u8) = (0x49, 0x62, 0x29);
                        // Color de fin (borde derecho): verde claro pastel.
                        // Más claro que ACCENT (sage) pero NO blanco.
                        // RGB ≈ (200, 220, 170) lee como verde menta suave.
                        const C_END:   (u8, u8, u8) = (200, 220, 170);

                        // SplitMix64 inline — determinístico y rápido.
                        let mut state: u64 = SEED;
                        #[inline]
                        fn next_f32(state: &mut u64) -> f32 {
                            *state = state.wrapping_add(0x9E3779B97F4A7C15);
                            let mut z = *state;
                            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
                            z ^= z >> 31;
                            ((z >> 40) as u32) as f32 / ((1u32 << 24) as f32)
                        }

                        let painter = ui.painter();
                        let avail_h = rect.height();
                        let n_bars = ((avail_w / (BAR_W + BAR_GAP)) as usize).max(1);
                        let baseline_y = rect.max.y;

                        for i in 0..n_bars {
                            let r = next_f32(&mut state);
                            let h_ratio = HEIGHT_MIN + (HEIGHT_MAX - HEIGHT_MIN) * r;
                            let bh = avail_h * h_ratio;

                            // Posición normalizada en [0, 1]: 0 = pegado al
                            // título, 1 = borde derecho.
                            let t = if n_bars > 1 {
                                i as f32 / (n_bars - 1) as f32
                            } else { 0.0 };

                            // Interpolación lineal de color (RGB) entre
                            // C_START (oscuro) y C_END (verde claro pastel).
                            let r_v = C_START.0 as f32
                                + (C_END.0 as f32 - C_START.0 as f32) * t;
                            let g_v = C_START.1 as f32
                                + (C_END.1 as f32 - C_START.1 as f32) * t;
                            let b_v = C_START.2 as f32
                                + (C_END.2 as f32 - C_START.2 as f32) * t;
                            let color = Color32::from_rgb(r_v as u8, g_v as u8, b_v as u8);

                            let bx = rect.min.x + i as f32 * (BAR_W + BAR_GAP);
                            let bar_rect = egui::Rect::from_min_max(
                                egui::pos2(bx, baseline_y - bh),
                                egui::pos2(bx + BAR_W, baseline_y),
                            );
                            painter.rect_filled(bar_rect, ROUND, color);
                        }
                    }
                });
            });

        // ── STATUS BAR ──────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar")
            .frame(egui::Frame::default().fill(theme::PANEL).inner_margin(8.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).size(11.5).color(theme::MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("v1.0 · jlmera").size(11.5).color(theme::MUTED));
                    });
                });
            });

        // ── CENTRAL ─────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::BG).inner_margin(14.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Carpeta raíz:");
                    // Marco verde oscuro alrededor del TextEdit para destacar
                    // que es la entrada principal de configuración.
                    egui::Frame::none()
                        .stroke(egui::Stroke::new(1.5, theme::ACCENT_H))
                        .rounding(4.0)
                        .inner_margin(egui::Margin::symmetric(4.0, 2.0))
                        .fill(theme::BG)
                        .show(ui, |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.root)
                                   .desired_width(360.0)
                                   .interactive(!self.busy)
                                   .frame(false));
                        });
                    if ui.add_enabled(!self.busy,
                                       egui::Button::new("…")
                                           .min_size(egui::vec2(28.0, 0.0)))
                          .on_hover_text("Seleccionar carpeta raíz")
                          .clicked()
                    {
                        let initial = std::path::PathBuf::from(&self.root);
                        let mut dlg = rfd::FileDialog::new()
                            .set_title("Selecciona la carpeta raíz GitHub");
                        if initial.exists() {
                            dlg = dlg.set_directory(&initial);
                        }
                        if let Some(picked) = dlg.pick_folder() {
                            self.root = picked.to_string_lossy().into_owned();
                            self.append_log(
                                format!("📂 Carpeta raíz: {}", self.root),
                                theme::ACCENT_H,
                            );
                        }
                    }
                    ui.add_space(14.0);
                    ui.label("Umbral:");
                    // Slider de 0.0 a 0.95: el 0.0 funciona como "escape hatch"
                    // para mover repos con confianza cero (típicamente READMEs
                    // que el filtro heurístico no puede clasificar).
                    ui.add(egui::Slider::new(&mut self.threshold, 0.0..=0.95).fixed_decimals(2));
                    ui.add_space(14.0);
                    ui.checkbox(&mut self.use_llm, "Usar LLM (Anthropic)");
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Fila 1 (combinada): FLUJO PRINCIPAL (izq) + MANTENIMIENTO (der) ──
                ui.horizontal(|ui| {
                    // FLUJO PRINCIPAL — pegado a la margen izquierda
                    ui.add_enabled_ui(!self.busy, |ui| {
                        if ui.button("🪞  Resolver duplicados").clicked() { self.start_action("resolve_dups"); }
                        if ui.button("🔍  Escanear sin aplicar").clicked() { self.start_action("scan"); }
                        let apply = egui::Button::new(
                            RichText::new("✅  Aplicar (mover)").color(Color32::WHITE)
                        ).fill(theme::ACCENT);
                        if ui.add(apply).clicked() { self.start_action("apply"); }
                    });

                    // MANTENIMIENTO — alineado a la margen derecha.
                    // En right_to_left los widgets se agregan en ORDEN INVERSO
                    // al orden visual deseado. Para conseguir visualmente:
                    //   [🧹 Limpiar log] [🔄 Solo reindexar] [🔁 Reclasificar] [🔍 Descubrir] [🏷 Categorías] [⚙ API Key]
                    // se agregan al revés: API Key → Categorías → Descubrir → Reclasificar → Reindexar → Limpiar.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // ⚙ API Key — gated por busy. Queda visualmente a la DERECHA.
                        if ui.add_enabled(!self.busy,
                            egui::Button::new("⚙  API Key")
                        ).clicked() {
                            self.api_dialog = Some(ApiKeyDialogState {
                                key:      load_api_key().unwrap_or_default(),
                                pat:      load_github_pat().unwrap_or_default(),
                                show:     false,
                                show_pat: false,
                                status: match (has_api_key(), has_github_pat()) {
                                    (true,  true)  => "✓ Anthropic key + GitHub PAT configurados.".to_string(),
                                    (true,  false) => "✓ Anthropic key configurada · sin PAT (rate-limit GitHub 60/h).".to_string(),
                                    (false, true)  => "⚠ Falta Anthropic key · GitHub PAT presente.".to_string(),
                                    (false, false) => "⚠ Aún no hay credenciales configuradas.".to_string(),
                                },
                            });
                        }
                        // 🏷 Categorías — abre el editor (Fase 3c). Gated por busy.
                        // Queda visualmente entre Descubrir y API Key.
                        if ui.add_enabled(!self.busy,
                            egui::Button::new("🏷  Categorías")
                        ).clicked() {
                            self.open_cats_editor();
                        }
                        // 🔍 Descubrir categorías — fase 4. Gated por busy.
                        // Lee repo_ids.json y propone topics frecuentes
                        // que no están cubiertos por las categorías actuales.
                        if ui.add_enabled(!self.busy,
                            egui::Button::new("🔍  Descubrir categorías")
                        ).clicked() {
                            self.open_discover_dialog();
                        }
                        // 🔁 Reclasificar todo — gated por busy.
                        if ui.add_enabled(!self.busy,
                            egui::Button::new("🔁  Reclasificar todo")
                        ).clicked() {
                            self.start_action("reclassify");
                        }
                        // 🔄 Solo reindexar — gated por busy.
                        if ui.add_enabled(!self.busy,
                            egui::Button::new("🔄  Solo reindexar")
                        ).clicked() {
                            self.start_action("reindex");
                        }
                        // 🧹 Limpiar log — SIEMPRE clickeable (no gated). Queda
                        // visualmente a la IZQUIERDA del grupo.
                        if ui.button("🧹  Limpiar log").clicked() { self.log_lines.clear(); }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                // ── Fila 2: ENRIQUECER Y PUBLICAR ────────────────────────
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("🆔  Refrescar GitHub IDs").clicked() { self.start_action("refresh_github"); }
                        // Botón con la bandera de Colombia como icono real (PNG embebido),
                        // no como emoji — egui no compone los regional indicators correctamente.
                        let btn_translate = if let Some(tex) = self.bandera_co_tex.as_ref() {
                            let img = egui::Image::new((tex.id(), egui::vec2(20.0, 14.0)));
                            egui::Button::image_and_text(img, "  Traducir READMEs")
                        } else {
                            egui::Button::new("Traducir READMEs")
                        };
                        if ui.add(btn_translate).clicked() { self.start_action("translate"); }
                        if ui.button("📚  Wiki Obsidian").clicked() { self.start_action("wiki"); }
                        if ui.button("🌐  Abrir buscador").clicked() { self.open_buscador(); }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                let prog = match self.progress {
                    Some((c, t)) if t > 0 => Some(c as f32 / t as f32),
                    Some(_) => Some(0.0),
                    None if self.busy => None,
                    None => Some(0.0),
                };
                let pb_text = match self.progress {
                    Some((c, t)) => format!("{} / {}", c, t),
                    None if self.busy => "…".to_string(),
                    None => "—".to_string(),
                };
                ui.horizontal(|ui| {
                    let total_width = ui.available_width() - 90.0;
                    let radius = 4.0;
                    // Tiempo absoluto del frame — independiente del FPS y del
                    // movimiento del mouse. Sustituye al spinner_phase acumulado
                    // (que se aceleraba con repaints frecuentes).
                    let time = ui.ctx().input(|i| i.time) as f32;

                    let (rect, _resp) = ui.allocate_exact_size(
                        egui::vec2(total_width, 16.0),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter();
                    painter.rect_filled(rect, radius, theme::PANEL2);
                    painter.rect_stroke(
                        rect, radius, egui::Stroke::new(1.0, theme::BORDER));

                    match prog {
                        // ── Determinado: fill verde + sheen pequeño que recorre ──
                        Some(p) => {
                            let p = p.clamp(0.0, 1.0);
                            let filled_w = rect.width() * p;
                            if filled_w > 0.0 {
                                let filled_rect = egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(filled_w, rect.height()),
                                );
                                painter.rect_filled(filled_rect, radius, theme::ACCENT);

                                // Sheen reducido al 50%: 12.5% del fill, mínimo 10 px.
                                if self.busy && filled_w > 8.0 {
                                    let sheen_w = (filled_w * 0.0625).max(8.0);
                                    // 0.20 ciclos/seg → ~5 s por recorrido completo.
                                    let phase   = (time * 0.20).rem_euclid(1.0);
                                    let travel  = filled_w + sheen_w;
                                    let sheen_x = rect.min.x + phase * travel - sheen_w;

                                    let s_left  = sheen_x.max(rect.min.x);
                                    let s_right = (sheen_x + sheen_w).min(rect.min.x + filled_w);
                                    if s_right > s_left {
                                        let sheen_rect = egui::Rect::from_min_max(
                                            egui::pos2(s_left,  rect.min.y + 1.0),
                                            egui::pos2(s_right, rect.max.y - 1.0),
                                        );
                                        let sheen_color = egui::Color32::from_rgba_unmultiplied(
                                            255, 255, 255, 70);
                                        painter.rect_filled(sheen_rect, radius, sheen_color);
                                    }
                                }
                            }
                        }
                        // ── Indeterminado: marquee del 30% del track recorriendo I→D
                        None => {
                            let seg_w  = rect.width() * 0.30;
                            // 0.20 ciclos/seg, igual ritmo que el sheen.
                            let phase  = (time * 0.20).rem_euclid(1.0);
                            let travel = rect.width() + seg_w;
                            let seg_x  = rect.min.x + phase * travel - seg_w;

                            let seg_left  = seg_x.max(rect.min.x);
                            let seg_right = (seg_x + seg_w).min(rect.max.x);
                            if seg_right > seg_left {
                                let seg_rect = egui::Rect::from_min_max(
                                    egui::pos2(seg_left,  rect.min.y),
                                    egui::pos2(seg_right, rect.max.y),
                                );
                                painter.rect_filled(seg_rect, radius, theme::ACCENT);
                            }
                        }
                    }
                    ui.label(RichText::new(pb_text).size(12.0).monospace().color(theme::MUTED));
                });
                ui.add_space(8.0);

                // Capturar dimensiones ANTES de pintar — el Frame por defecto
                // se ajusta al contenido; lo forzamos al ancho del padre.
                // .max(0.0) protege contra layouts degenerados donde la altura
                // restante quedó negativa por margenes acumulados.
                let log_height = ui.available_height().max(0.0);
                let log_width  = ui.available_width().max(0.0);

                // Acciones diferidas del menú contextual del log. Capturamos
                // los clicks en flags y los aplicamos DESPUÉS del closure
                // para no chocar con el borrow mutable de self.log_lines.
                let mut want_copy_all = false;
                let mut want_clear    = false;

                let frame_resp = egui::Frame::default().fill(theme::PANEL)
                    .stroke(egui::Stroke::new(1.0, theme::BORDER))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        // Reservar ancho mínimo del Frame interno.
                        ui.set_min_width((log_width - 14.0).max(0.0));
                        ScrollArea::vertical()
                            .id_salt("log_scroll")
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .max_height(log_height)
                            .show(ui, |ui| {
                                ui.set_min_width((log_width - 26.0).max(0.0));

                                // Render con un ÚNICO Label rico (LayoutJob) en
                                // vez de un Label por línea. Ventajas:
                                //   - selección continua del mouse a través de
                                //     varias líneas (no se puede con N Labels)
                                //   - Ctrl+C nativo copia exactamente lo marcado
                                //   - menú contextual aplica al área entera
                                //   - colores por línea siguen preservados
                                let mut job = egui::text::LayoutJob::default();
                                let font = egui::FontId::monospace(11.0);
                                for (i, (line, color)) in self.log_lines.iter().enumerate() {
                                    if i > 0 {
                                        job.append("\n", 0.0, egui::TextFormat {
                                            font_id: font.clone(),
                                            ..Default::default()
                                        });
                                    }
                                    job.append(line, 0.0, egui::TextFormat {
                                        font_id: font.clone(),
                                        color:   *color,
                                        ..Default::default()
                                    });
                                }
                                // Render natural del Label (left-aligned,
                                // rect = bounding box del galley = línea más
                                // larga × cantidad de líneas).
                                //
                                // Por qué esto resuelve el menú en horizontal
                                // sin centrar el texto: aunque el rect del
                                // Label NO ocupa todo el ancho del ScrollArea,
                                // contiene a TODAS las líneas — incluyendo las
                                // cortas, que en su línea quedan rodeadas de
                                // bbox horizontal. Right-click sobre la zona
                                // vacía a la derecha de una línea corta cae
                                // dentro del rect del Label (porque otra línea
                                // del mismo galley es más larga) → menú aparece.
                                //
                                // El único caso edge es cuando TODAS las líneas
                                // son cortas: ahí el bbox queda estrecho. Para
                                // ese caso lo cubrimos con fill_right_resp más
                                // abajo.
                                let label_resp = ui.add(
                                    egui::Label::new(job).selectable(true)
                                );

                                // Menú contextual #1: sobre el área del TEXTO.
                                // Cubre las líneas escritas Y el espacio a la derecha
                                // de cada línea (gracias al add_sized de arriba).
                                // El menú se cierra y dispara las acciones via flags
                                // `want_*`, que se aplican fuera del closure (más abajo).
                                label_resp.context_menu(|ui| {
                                    if ui.button("📋  Copiar todo el log").clicked() {
                                        want_copy_all = true;
                                        ui.close_menu();
                                    }
                                    ui.label(RichText::new(
                                        "(o marcá texto + Ctrl+C para copiar solo lo seleccionado)"
                                    ).color(theme::MUTED).size(10.0).italics());
                                    ui.separator();
                                    if ui.button(
                                        RichText::new("🧹  Limpiar log").color(theme::WARN)
                                    ).clicked() {
                                        want_clear = true;
                                        ui.close_menu();
                                    }
                                });

                                // Menú contextual #2 — área a la DERECHA del Label.
                                // Caso edge: cuando el galley es estrecho (todas
                                // las líneas son cortas), el rect del Label no
                                // llega al borde derecho del ScrollArea. Alocamos
                                // un rect Sense::click que cubre exactamente esa
                                // franja: misma altura del Label, ancho hasta el
                                // borde derecho del ui.
                                let label_rect = label_resp.rect;
                                let scroll_max = ui.max_rect();
                                let right_w = scroll_max.max.x - label_rect.max.x;
                                if right_w > 0.5 {
                                    let right_rect = egui::Rect::from_min_size(
                                        egui::pos2(label_rect.max.x, label_rect.min.y),
                                        egui::vec2(right_w, label_rect.height()),
                                    );
                                    let right_resp = ui.interact(
                                        right_rect,
                                        ui.id().with("log_right_fill"),
                                        egui::Sense::click(),
                                    );
                                    right_resp.context_menu(|ui| {
                                        if ui.button("📋  Copiar todo el log").clicked() {
                                            want_copy_all = true;
                                            ui.close_menu();
                                        }
                                        ui.label(RichText::new(
                                            "(o marcá texto + Ctrl+C para copiar solo lo seleccionado)"
                                        ).color(theme::MUTED).size(10.0).italics());
                                        ui.separator();
                                        if ui.button(
                                            RichText::new("🧹  Limpiar log").color(theme::WARN)
                                        ).clicked() {
                                            want_clear = true;
                                            ui.close_menu();
                                        }
                                    });
                                }

                                // Menú contextual #3 — área DEBAJO del Label.
                                // Cubre el espacio vertical sobrante hasta el
                                // borde inferior del ScrollArea. auto_shrink=false
                                // garantiza que ese espacio existe aunque el log
                                // tenga pocas líneas.
                                let fill_h = ui.available_height();
                                if fill_h > 0.5 {
                                    let fill_w = ui.available_width();
                                    let fill_resp = ui.allocate_response(
                                        egui::vec2(fill_w, fill_h),
                                        egui::Sense::click(),
                                    );
                                    fill_resp.context_menu(|ui| {
                                        if ui.button("📋  Copiar todo el log").clicked() {
                                            want_copy_all = true;
                                            ui.close_menu();
                                        }
                                        ui.label(RichText::new(
                                            "(o marcá texto + Ctrl+C para copiar solo lo seleccionado)"
                                        ).color(theme::MUTED).size(10.0).italics());
                                        ui.separator();
                                        if ui.button(
                                            RichText::new("🧹  Limpiar log").color(theme::WARN)
                                        ).clicked() {
                                            want_clear = true;
                                            ui.close_menu();
                                        }
                                    });
                                }
                            });
                    }).response;
                let _ = frame_resp; // shadowed-warning-quiet

                // Aplicar las acciones diferidas — ya fuera del borrow del closure.
                if want_copy_all {
                    let all_text: String = self.log_lines.iter()
                        .map(|(l, _)| l.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ctx.copy_text(all_text);
                    self.append_log("📋 Log copiado al portapapeles", theme::OK);
                }
                if want_clear {
                    self.log_lines.clear();
                }
            });

        // ── DIÁLOGO RESOLVER DUPLICADOS ──────────────────────────
        let mut should_apply = false;
        let mut should_close = false;
        if let Some(dlg) = self.dups_dialog.as_mut() {
            let mut open = dlg.open;
            // Calcular altura máxima del Window respecto a la ventana actual:
            // 80% del alto disponible, con tope de 720 px para no quedarse infinito
            // si la ventana del SO es enorme. Garantiza que la sección de botones
            // (Bulk + Aplicar/Cancelar) siempre quede visible debajo del scroll.
            let screen_h     = ctx.screen_rect().height();
            let win_max_h    = (screen_h * 0.80).min(720.0).max(400.0);
            // El ScrollArea interno se queda con todo menos lo que ocupan
            // el header de instrucciones (~50 px) y la fila de botones (~70 px).
            let scroll_max_h = (win_max_h - 140.0).max(200.0);

            egui::Window::new(format!("🚫 Resolver duplicados ({})", dlg.items.len()))
                .open(&mut open)
                .default_size([960.0, win_max_h])
                .min_width(720.0)
                .max_height(win_max_h)
                .show(ctx, |ui| {
                    ui.label(RichText::new(
                        "Por cada duplicado, elige una acción. La RECOMENDADA es 'Archivar' \
                         (no destruye nada). Cuando termines pulsa 'Aplicar decisiones'."
                    ).color(theme::MUTED).size(11.5));
                    ui.separator();

                    ScrollArea::vertical()
                        .max_height(scroll_max_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                        for (idx, item) in dlg.items.iter_mut().enumerate() {
                            egui::Frame::default()
                                .fill(theme::PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER))
                                .inner_margin(10.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("#{}", idx+1))
                                                 .monospace().strong().color(theme::MUTED));
                                        ui.label(RichText::new(&item.new_info.name)
                                                 .size(13.0).strong().color(theme::FG));
                                        ui.label(RichText::new(format!("· motivo: {}", item.motivo))
                                                 .color(theme::ERR).size(10.5));
                                    });
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        // Columna NUEVO
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new("🆕 NUEVO (en _inbox)")
                                                     .strong().color(theme::ACCENT_H).size(10.5));
                                            ui.label(RichText::new(format!("path: {}", item.new_info.path.display()))
                                                     .monospace().size(9.5).color(theme::FG));
                                            ui.label(RichText::new(format!("url:  {}", item.new_info.url))
                                                     .monospace().size(9.5).color(theme::FG));
                                            ui.label(RichText::new(format!("último: {}", item.new_info.last_commit))
                                                     .monospace().size(9.5).color(theme::FG));
                                            ui.label(RichText::new(format!("tamaño: ~{:.1} MB", item.new_info.size_mb))
                                                     .monospace().size(9.5).color(theme::FG));
                                        });
                                        ui.add_space(20.0);
                                        // Columna EXISTENTE
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new("📁 EXISTENTE (categorizado)")
                                                     .strong().color(theme::OK).size(10.5));
                                            ui.label(RichText::new(format!("path: {}", item.old_info.path.display()))
                                                     .monospace().size(9.5).color(theme::FG));
                                            ui.label(RichText::new(format!("url:  {}", item.old_info.url))
                                                     .monospace().size(9.5).color(theme::FG));
                                            ui.label(RichText::new(format!("último: {}", item.old_info.last_commit))
                                                     .monospace().size(9.5).color(theme::FG));
                                            ui.label(RichText::new(format!("tamaño: ~{:.1} MB", item.old_info.size_mb))
                                                     .monospace().size(9.5).color(theme::FG));
                                        });
                                    });
                                    ui.add_space(6.0);
                                    ui.label(RichText::new("¿Qué hago con el repo nuevo?")
                                             .strong().size(11.0));
                                    let mut sel = match &item.action {
                                        DupAction::Archive   => 0,
                                        DupAction::Skip      => 1,
                                        DupAction::Rename(_) => 2,
                                        DupAction::Replace   => 3,
                                        DupAction::Delete    => 4,
                                    };
                                    let labels = [
                                        "📦 Archivar (recomendado)",
                                        "⏭ Dejar en _inbox",
                                        "✏ Renombrar y reprocesar",
                                        "🔄 Reemplazar viejo (⚠)",
                                        "🗑 Borrar nuevo (⚠ irreversible)",
                                    ];
                                    ui.horizontal_wrapped(|ui| {
                                        for (i, label) in labels.iter().enumerate() {
                                            if ui.selectable_label(sel == i, *label).clicked() {
                                                sel = i;
                                            }
                                        }
                                    });
                                    item.action = match sel {
                                        0 => DupAction::Archive,
                                        1 => DupAction::Skip,
                                        2 => DupAction::Rename(item.new_name.clone()),
                                        3 => DupAction::Replace,
                                        4 => DupAction::Delete,
                                        _ => DupAction::Archive,
                                    };
                                    if matches!(item.action, DupAction::Rename(_)) {
                                        ui.horizontal(|ui| {
                                            ui.label("Nombre:");
                                            ui.add(egui::TextEdit::singleline(&mut item.new_name)
                                                   .desired_width(280.0));
                                        });
                                        item.action = DupAction::Rename(item.new_name.clone());
                                    }
                                });
                            ui.add_space(6.0);
                        }
                    });

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Bulk: ").color(theme::MUTED));
                        if ui.button("📦 Archivar todos").clicked() {
                            for it in &mut dlg.items { it.action = DupAction::Archive; }
                        }
                        if ui.button("⏭ Saltar todos").clicked() {
                            for it in &mut dlg.items { it.action = DupAction::Skip; }
                        }
                        if ui.add(egui::Button::new(
                                RichText::new("🔄 Reemplazar todos").color(theme::WARN)
                            )).clicked() {
                            for it in &mut dlg.items { it.action = DupAction::Replace; }
                        }
                        if ui.add(egui::Button::new(
                                RichText::new("🗑 Borrar todos").color(theme::ERR)
                            )).clicked() {
                            for it in &mut dlg.items { it.action = DupAction::Delete; }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let apply = egui::Button::new(
                                RichText::new("✅  Aplicar decisiones").color(Color32::WHITE)
                            ).fill(theme::ACCENT);
                            if ui.add(apply).clicked() {
                                should_apply = true;
                            }
                            if ui.button("Cancelar").clicked() {
                                should_close = true;
                            }
                        });
                    });
                });
            if !open { should_close = true; }
            dlg.open = open;
        }
        if should_apply {
            // 1) Cerrar el diálogo INMEDIATAMENTE (sensación de "click → algo pasó").
            // 2) Lanzar el trabajo pesado (mover/borrar repos + reindex + html)
            //    en un worker thread para no congelar la GUI. La barra
            //    indeterminada se anima sola y el log refleja el avance.
            //    Antes esto corría sincrónico en el main thread y daba la
            //    sensación de "no pasó nada" cuando había repos pesados.
            if let Some(dlg) = self.dups_dialog.take() {
                self.start_apply_duplicates(dlg.items);
            }
        } else if should_close {
            self.dups_dialog = None;
        }

        // ── DIÁLOGO RECLASIFICAR ─────────────────────────────────
        let mut should_apply_reclass = false;
        let mut should_close_reclass = false;
        if let Some(dlg) = self.reclass_dialog.as_mut() {
            let mut open = dlg.open;
            let screen_h     = ctx.screen_rect().height();
            let win_max_h    = (screen_h * 0.80).min(720.0).max(400.0);
            let scroll_max_h = (win_max_h - 160.0).max(200.0);

            egui::Window::new(format!("🔁 Reclasificar repos ({} cambios propuestos)",
                                       dlg.changes.len()))
                .open(&mut open)
                .default_size([900.0, win_max_h])
                .min_width(700.0)
                .max_height(win_max_h)
                .show(ctx, |ui| {
                    ui.label(RichText::new(
                        "Estos repos cambiarían de categoría según el algoritmo actual y \
                         data/categorias.json. Marca/desmarca cada uno y pulsa 'Aplicar' \
                         cuando estés conforme. Los movimientos son físicos en disco — \
                         las carpetas destino se crean si no existen."
                    ).color(theme::MUTED).size(11.0));
                    ui.separator();

                    // Header de la tabla
                    ui.horizontal(|ui| {
                        ui.add_sized([28.0,  18.0], egui::Label::new(RichText::new("✓").strong()));
                        ui.add_sized([240.0, 18.0], egui::Label::new(RichText::new("Repo").strong()));
                        ui.add_sized([200.0, 18.0], egui::Label::new(RichText::new("Actual").strong()));
                        ui.add_sized([18.0,  18.0], egui::Label::new(RichText::new("→").strong().color(theme::MUTED)));
                        ui.add_sized([200.0, 18.0], egui::Label::new(RichText::new("Propuesta").strong()));
                        ui.add_sized([60.0,  18.0], egui::Label::new(RichText::new("Conf.").strong()));
                    });
                    ui.separator();

                    ScrollArea::vertical()
                        .max_height(scroll_max_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for change in dlg.changes.iter_mut() {
                                ui.horizontal(|ui| {
                                    ui.add_sized([28.0, 18.0],
                                        egui::Checkbox::new(&mut change.selected, ""));
                                    ui.add_sized([240.0, 18.0], egui::Label::new(
                                        RichText::new(&change.name).color(theme::FG)));
                                    ui.add_sized([200.0, 18.0], egui::Label::new(
                                        RichText::new(&change.current_cat)
                                            .monospace().size(11.0).color(theme::MUTED)));
                                    ui.add_sized([18.0, 18.0], egui::Label::new(
                                        RichText::new("→").color(theme::ACCENT_H)));
                                    ui.add_sized([200.0, 18.0], egui::Label::new(
                                        RichText::new(&change.proposed_cat)
                                            .monospace().size(11.0).color(theme::ACCENT_H)));
                                    let conf_color = if change.confidence >= 0.6 { theme::OK }
                                        else if change.confidence >= 0.3 { theme::WARN }
                                        else { theme::ERR };
                                    ui.add_sized([60.0, 18.0], egui::Label::new(
                                        RichText::new(format!("{:.2}", change.confidence))
                                            .monospace().size(11.0).color(conf_color)));
                                });
                            }
                        });

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Bulk:").color(theme::MUTED));
                        if ui.button("✓ Marcar todos").clicked() {
                            for c in &mut dlg.changes { c.selected = true; }
                        }
                        if ui.button("☐ Desmarcar todos").clicked() {
                            for c in &mut dlg.changes { c.selected = false; }
                        }
                        let count_sel = dlg.changes.iter().filter(|c| c.selected).count();
                        ui.label(RichText::new(format!("· {} de {} seleccionados",
                                                        count_sel, dlg.changes.len()))
                                 .color(theme::MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let apply = egui::Button::new(
                                RichText::new(format!("✅  Aplicar {}", count_sel))
                                    .color(Color32::WHITE)
                            ).fill(theme::ACCENT);
                            if ui.add_enabled(count_sel > 0, apply).clicked() {
                                should_apply_reclass = true;
                            }
                            if ui.button("Cancelar").clicked() {
                                should_close_reclass = true;
                            }
                        });
                    });
                });
            if !open { should_close_reclass = true; }
            dlg.open = open;
        }
        if should_apply_reclass {
            // 1) Cerrar el diálogo INMEDIATAMENTE (sensación de "click → algo pasó").
            // 2) Lanzar el trabajo pesado (movimientos + reindex + html) en un
            //    worker thread para no congelar la GUI. La barra indeterminada
            //    se anima sola y el log refleja el avance.
            if let Some(dlg) = self.reclass_dialog.take() {
                self.start_apply_reclass(dlg.changes);
            }
        } else if should_close_reclass {
            self.reclass_dialog = None;
        }

        // ── DIÁLOGO EDITOR DE CATEGORÍAS (Fase 3c) ────────────────
        // Acciones que se quieren disparar al final del frame, fuera
        // del borrow de `self.cats_dialog`. Cada una se aplica si el
        // usuario clickeó el botón correspondiente; al final del bloque
        // resolvemos en orden: guardar > reset > cerrar.
        let mut cats_should_save     = false;
        let mut cats_should_close    = false;
        let mut cats_should_reset    = false;
        let mut cats_should_compact  = false; // 🔢 Compactar numeración
        let mut cats_should_purge    = false; // 🧹 Borrar vacías
        let mut cats_move_up: Option<usize>  = None;
        let mut cats_move_down: Option<usize> = None;
        let mut cats_delete: Option<usize> = None;
        let mut cats_add_new = false;

        if let Some(dlg) = self.cats_dialog.as_mut() {
            let mut open = dlg.open;
            let screen_h     = ctx.screen_rect().height();
            let win_max_h    = (screen_h * 0.85).min(760.0).max(420.0);

            egui::Window::new("🏷 Editor de categorías")
                .open(&mut open)
                .default_size([1100.0, win_max_h])
                .min_width(900.0)
                .min_height(420.0)
                .max_height(win_max_h)
                .show(ctx, |ui| {
                    // ── Header explicativo ─────────────────────────
                    ui.label(RichText::new(
                        "Edita las categorías que el clasificador usa. \
                         Renombrar el ID renombra la carpeta física en disco. \
                         Borrar una categoría con repos los mueve antes a la \
                         última de la lista (= fallback). Los cambios solo se \
                         persisten al pulsar 💾 Guardar."
                    ).color(theme::MUTED).size(11.0));
                    ui.separator();

                    // ── Layout 2 columnas: lista (izq) + editor (der) ──
                    let list_w   = 360.0_f32;
                    let footer_h = 44.0_f32;
                    let body_h   = ui.available_height() - footer_h;

                    ui.horizontal(|ui| {
                        // ─────── COLUMNA IZQUIERDA: LISTA DE CATEGORÍAS ───────
                        ui.allocate_ui_with_layout(
                            egui::vec2(list_w, body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.label(RichText::new(format!(
                                    "Categorías ({})", dlg.rows.len()
                                )).strong().color(theme::FG));
                                ui.add_space(2.0);
                                // Captura clicks en filas para aplicar la selección
                                // DESPUÉS del loop — escribir dlg.selected_idx
                                // mientras iteramos dlg.rows daría conflicto de borrows.
                                let mut new_selection: Option<usize> = None;
                                ScrollArea::vertical()
                                    .id_salt("cats_list_scroll")
                                    .max_height(body_h - 36.0)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let last_idx = dlg.rows.len().saturating_sub(1);
                                        // Tamaños fijos para que cada celda esté alineada
                                        // verticalmente entre filas. Sin esto, los botones
                                        // toman el ancho del carácter (▲ ≠ ▼ ≠ ✕) y bailan.
                                        const NAME_W: f32 = 240.0;
                                        const BTN_W:  f32 = 26.0;
                                        const ROW_H:  f32 = 22.0;
                                        for (i, row) in dlg.rows.iter().enumerate() {
                                            // Para mostrar el conteo: usamos el id ORIGINAL
                                            // si existe (= la carpeta física al abrir el diálogo);
                                            // así renombrar no hace "saltar" el contador a 0
                                            // hasta que se guarde y la carpeta exista.
                                            let lookup_id = row.original_id.as_deref()
                                                .unwrap_or(row.def.id.as_str());
                                            let n_repos = dlg.repo_counts
                                                .get(lookup_id).copied().unwrap_or(0);
                                            let is_selected = dlg.selected_idx == Some(i);
                                            let is_fallback = i == last_idx && !dlg.rows.is_empty();

                                            ui.horizontal(|ui| {
                                                // ── Label principal: id + (N) ──
                                                // Formato consistente: SIEMPRE 'id (N)' sin
                                                // prefijos. El color y el sufijo "· fallback"
                                                // marcan el estado, no caracteres en la izquierda
                                                // (eso desalinea las columnas).
                                                let label = format!("{}  ({})", row.def.id, n_repos);
                                                let mut text = RichText::new(label).monospace().size(12.0);
                                                if is_selected      { text = text.color(theme::ACCENT_H).strong(); }
                                                else if is_fallback { text = text.color(theme::ACCENT).strong(); }
                                                else if row.is_new() { text = text.color(theme::OK); }
                                                else if row.is_renamed() { text = text.color(theme::WARN); }
                                                else                { text = text.color(theme::FG); }
                                                if ui.add_sized([NAME_W, ROW_H],
                                                    egui::SelectableLabel::new(is_selected, text)
                                                ).clicked() {
                                                    new_selection = Some(i);
                                                }

                                                // ── ▲ Subir ──
                                                let up_resp = ui.add_enabled(i > 0,
                                                    egui::Button::new(RichText::new("▲").size(13.0))
                                                        .min_size(egui::vec2(BTN_W, ROW_H))
                                                );
                                                if up_resp.on_hover_text("Subir un puesto").clicked() {
                                                    cats_move_up = Some(i);
                                                }

                                                // ── ▼ Bajar ──
                                                let down_resp = ui.add_enabled(i + 1 < dlg.rows.len(),
                                                    egui::Button::new(RichText::new("▼").size(13.0))
                                                        .min_size(egui::vec2(BTN_W, ROW_H))
                                                );
                                                if down_resp.on_hover_text("Bajar un puesto").clicked() {
                                                    cats_move_down = Some(i);
                                                }

                                                // ── ✕ Borrar ──
                                                let can_delete = dlg.rows.len() > 1;
                                                let del_resp = ui.add_enabled(can_delete,
                                                    egui::Button::new(
                                                        RichText::new("✕").size(13.0).color(theme::ERR)
                                                    ).min_size(egui::vec2(BTN_W, ROW_H))
                                                );
                                                let del_tooltip = if !can_delete {
                                                    "No se puede borrar — debe quedar al menos 1 categoría"
                                                        .to_string()
                                                } else if n_repos > 0 {
                                                    format!("Borrar categoría · sus {} repo(s) se mueven a fallback",
                                                            n_repos)
                                                } else {
                                                    "Borrar categoría (carpeta vacía)".to_string()
                                                };
                                                if del_resp.on_hover_text(del_tooltip).clicked() {
                                                    cats_delete = Some(i);
                                                }

                                                // ── Marca de "fallback" al final de la fila ──
                                                // Mucho más legible que la estrellita ★ pegada al id.
                                                if is_fallback {
                                                    ui.label(
                                                        RichText::new("· fallback")
                                                            .color(theme::ACCENT)
                                                            .italics()
                                                            .size(10.0)
                                                    ).on_hover_text(
                                                        "Categoría de respaldo: aquí caen los repos \
                                                         cuando ningún score supera el umbral. \
                                                         Para cambiarla, usá ▲▼ y poné otra al final."
                                                    );
                                                }
                                            });
                                        }
                                    });
                                // Aplicar la selección capturada en el loop.
                                if let Some(i) = new_selection {
                                    dlg.selected_idx = Some(i);
                                }
                                ui.add_space(4.0);
                                ui.separator();
                                if ui.add_sized([list_w - 8.0, 24.0],
                                    egui::Button::new(
                                        RichText::new("+ Nueva categoría").color(theme::ACCENT_H)
                                    )
                                ).clicked() {
                                    cats_add_new = true;
                                }
                            });

                        ui.separator();

                        // ─────── COLUMNA DERECHA: EDITOR DE LA CATEGORÍA SELECCIONADA ───────
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let sel = dlg.selected_idx;
                                let row_count = dlg.rows.len();
                                if let Some(idx) = sel.filter(|&i| i < row_count) {
                                    let row = &mut dlg.rows[idx];

                                    // ── ID (editable también para existentes) ──
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("ID:").strong().color(theme::FG));
                                        ui.add_sized([240.0, 22.0],
                                            egui::TextEdit::singleline(&mut row.def.id)
                                                .font(egui::TextStyle::Monospace));
                                        if let Some(orig) = &row.original_id {
                                            if &row.def.id != orig {
                                                ui.label(RichText::new(
                                                    format!("(antes: {})", orig)
                                                ).color(theme::WARN).size(10.0));
                                            }
                                        } else {
                                            ui.label(RichText::new("(nueva)")
                                                .color(theme::OK).size(10.0));
                                        }
                                    });
                                    ui.label(RichText::new(
                                        "El ID es el nombre de la carpeta en disco. \
                                         Convención: 'NN-nombre-corto' en kebab-case."
                                    ).color(theme::MUTED).size(10.0));
                                    ui.add_space(6.0);

                                    // Sección scrollable con ambas tablas.
                                    ScrollArea::vertical()
                                        .id_salt("cats_editor_scroll")
                                        .max_height(body_h - 90.0)
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            // ── KEYWORDS ──
                                            ui.label(RichText::new("Keywords (peso 1-10)")
                                                .strong().color(theme::ACCENT_H));
                                            ui.label(RichText::new(
                                                "Se matchean contra nombre + descripción + primeros \
                                                 1500 chars del README, en lowercase."
                                            ).color(theme::MUTED).size(10.0));
                                            ui.add_space(2.0);
                                            let mut kw_to_remove: Option<usize> = None;
                                            for (j, (kw, w)) in row.def.keywords.iter_mut().enumerate() {
                                                ui.horizontal(|ui| {
                                                    ui.add_sized([320.0, 20.0],
                                                        egui::TextEdit::singleline(kw)
                                                            .hint_text("keyword en lowercase"));
                                                    ui.add(egui::DragValue::new(w)
                                                        .range(1..=10)
                                                        .speed(0.1));
                                                    if ui.button(RichText::new("✕").color(theme::ERR))
                                                        .clicked()
                                                    {
                                                        kw_to_remove = Some(j);
                                                    }
                                                });
                                            }
                                            if let Some(j) = kw_to_remove {
                                                row.def.keywords.remove(j);
                                            }
                                            if ui.button(RichText::new("+ Añadir keyword")
                                                .color(theme::ACCENT_H)).clicked()
                                            {
                                                row.def.keywords.push((String::new(), 3));
                                            }

                                            ui.add_space(12.0);
                                            ui.separator();
                                            ui.add_space(6.0);

                                            // ── TOPIC BOOSTS ──
                                            ui.label(RichText::new("Topic boosts de GitHub (peso 1-15)")
                                                .strong().color(theme::ACCENT_H));
                                            ui.label(RichText::new(
                                                "Match exacto contra cada elemento del array `topics` \
                                                 que devuelve la GitHub API. Pesos altos porque son \
                                                 metadata oficial del autor."
                                            ).color(theme::MUTED).size(10.0));
                                            ui.add_space(2.0);
                                            let mut tb_to_remove: Option<usize> = None;
                                            for (j, (t, w)) in row.def.topic_boosts.iter_mut().enumerate() {
                                                ui.horizontal(|ui| {
                                                    ui.add_sized([320.0, 20.0],
                                                        egui::TextEdit::singleline(t)
                                                            .hint_text("topic-en-kebab-case"));
                                                    ui.add(egui::DragValue::new(w)
                                                        .range(1..=15)
                                                        .speed(0.1));
                                                    if ui.button(RichText::new("✕").color(theme::ERR))
                                                        .clicked()
                                                    {
                                                        tb_to_remove = Some(j);
                                                    }
                                                });
                                            }
                                            if let Some(j) = tb_to_remove {
                                                row.def.topic_boosts.remove(j);
                                            }
                                            if ui.button(RichText::new("+ Añadir topic")
                                                .color(theme::ACCENT_H)).clicked()
                                            {
                                                row.def.topic_boosts.push((String::new(), 8));
                                            }
                                        });
                                } else {
                                    ui.label(RichText::new(
                                        "Selecciona una categoría a la izquierda para editarla."
                                    ).color(theme::MUTED).italics());
                                }
                            });
                    });

                    // ── Footer: utilidades + status + acciones ──
                    ui.separator();
                    ui.horizontal(|ui| {
                        // 🔢 Compactar numeración: renombra todas las categorías
                        // para que su prefijo NN sea consecutivo (01, 02, 03...).
                        // Útil tras varias pasadas del descubridor que dejaron
                        // numerales saltados (ej. 22 + 22 + 14 + 33 + 40).
                        if ui.button(
                            RichText::new("🔢  Compactar").color(theme::ACCENT_H)
                        ).on_hover_text(
                            "Renumera todas las categorías para que el prefijo \
                             NN- sea consecutivo según el orden actual de la lista. \
                             No persiste hasta que pulses Guardar — se aplican \
                             como renames físicos de carpetas."
                        ).clicked() {
                            cats_should_compact = true;
                        }
                        // 🧹 Borrar vacías: elimina categorías con 0 repos en disco.
                        let n_vacias = dlg.rows.iter()
                            .filter(|r| {
                                let lookup = r.original_id.as_deref()
                                    .unwrap_or(r.def.id.as_str());
                                dlg.repo_counts.get(lookup).copied().unwrap_or(0) == 0
                                && !r.is_new() // las recién añadidas obviamente están vacías
                            })
                            .count();
                        let purge_btn = egui::Button::new(
                            RichText::new(format!("🧹  Borrar vacías ({})", n_vacias))
                                .color(if n_vacias > 0 { theme::WARN } else { theme::MUTED })
                        );
                        if ui.add_enabled(n_vacias > 0, purge_btn)
                            .on_hover_text(
                                "Elimina del config las categorías que NO tienen \
                                 repos en disco (no incluye las recién añadidas \
                                 en esta sesión). Se persiste al pulsar Guardar."
                            ).clicked() {
                            cats_should_purge = true;
                        }
                        // 🔄 Reset a defaults
                        if ui.button(
                            RichText::new("🔄  Reset").color(theme::WARN)
                        ).on_hover_text(
                            "Reemplaza la lista actual por las categorías hardcoded \
                             (CATEGORIAS + KEYWORDS + TOPIC_BOOSTS de categories.rs). \
                             No persiste hasta que pulses Guardar."
                        ).clicked() {
                            cats_should_reset = true;
                        }
                        // Status / errores en el medio.
                        if !dlg.status.is_empty() {
                            ui.add_space(8.0);
                            let color = if dlg.status.starts_with('✗') { theme::ERR }
                                        else if dlg.status.starts_with('⚠') { theme::WARN }
                                        else { theme::OK };
                            ui.label(RichText::new(&dlg.status).color(color).size(11.0));
                        }
                        // Botones de acción a la derecha.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let save = egui::Button::new(
                                RichText::new("💾  Guardar").color(Color32::WHITE)
                            ).fill(theme::ACCENT);
                            if ui.add(save).clicked() {
                                cats_should_save = true;
                            }
                            if ui.button("Cancelar").clicked() {
                                cats_should_close = true;
                            }
                        });
                    });
                });
            if !open { cats_should_close = true; }
            dlg.open = open;
        }

        // ── Resolver acciones del editor (fuera del borrow mutable) ──
        // Reordenar / añadir / borrar tocan dlg.rows así que se aplican aquí.
        if let Some(dlg) = self.cats_dialog.as_mut() {
            if let Some(i) = cats_move_up {
                if i > 0 {
                    dlg.rows.swap(i, i - 1);
                    if dlg.selected_idx == Some(i)     { dlg.selected_idx = Some(i - 1); }
                    else if dlg.selected_idx == Some(i - 1) { dlg.selected_idx = Some(i); }
                }
            }
            if let Some(i) = cats_move_down {
                if i + 1 < dlg.rows.len() {
                    dlg.rows.swap(i, i + 1);
                    if dlg.selected_idx == Some(i)     { dlg.selected_idx = Some(i + 1); }
                    else if dlg.selected_idx == Some(i + 1) { dlg.selected_idx = Some(i); }
                }
            }
            if let Some(i) = cats_delete {
                if i < dlg.rows.len() {
                    dlg.rows.remove(i);
                    // Reajustar selected_idx para que siga apuntando a algo válido.
                    dlg.selected_idx = if dlg.rows.is_empty() {
                        None
                    } else {
                        Some(i.min(dlg.rows.len() - 1))
                    };
                    dlg.status.clear();
                }
            }
            if cats_add_new {
                // Generar un id provisional con sufijo numérico para que sea único.
                let mut n = dlg.rows.len() + 1;
                let mut new_id = format!("{:02}-nueva-categoria", n);
                while dlg.rows.iter().any(|r| r.def.id == new_id) {
                    n += 1;
                    new_id = format!("{:02}-nueva-categoria", n);
                }
                dlg.rows.push(CatRow::new_blank(new_id));
                dlg.selected_idx = Some(dlg.rows.len() - 1);
                dlg.status.clear();
            }
            if cats_should_reset {
                let defaults = CategoriasConfig::from_hardcoded();
                dlg.rows = defaults.categorias.into_iter()
                    .map(CatRow::from_existing)
                    .collect();
                // Marcar TODAS como "nuevas" para que el diff las reescriba al guardar.
                // En la práctica esto es indeseable: si las nuevas tienen el mismo id
                // que las viejas, NO queremos renombrar carpetas. Por eso preservamos
                // original_id si coincide con algún id existente del snapshot inicial.
                // Reset = solo cambia keywords/topics, mantiene los ids tal cual.
                dlg.selected_idx = if dlg.rows.is_empty() { None } else { Some(0) };
                dlg.status = "✓ Listas restauradas a defaults · pulsa Guardar para persistir".to_string();
            }
            if cats_should_compact {
                // Renumerar TODAS las filas en orden visual: 01, 02, 03, …
                // Si una fila tiene id `NN-resto`, reemplaza el prefijo NN.
                // Si NO tiene formato `NN-…` (algún id manual del usuario,
                // ej. `mi-experimento`), lo deja tal cual SIN tocar — no
                // queremos vandalizar nombres custom.
                let n = dlg.rows.len();
                let re = regex::Regex::new(r"^\d{2}-").expect("regex const");
                let mut renamed = 0usize;
                let mut skipped = 0usize;
                for (i, row) in dlg.rows.iter_mut().enumerate() {
                    let nn = (i + 1) as u32;
                    let id = row.def.id.clone();
                    if let Some(m) = re.find(&id) {
                        let rest = &id[m.end()..];
                        let new_id = format!("{:02}-{}", nn, rest);
                        if new_id != id {
                            row.def.id = new_id;
                            renamed += 1;
                        }
                    } else {
                        skipped += 1;
                    }
                }
                dlg.status = if renamed == 0 {
                    format!("✓ Numeración ya estaba consecutiva ({} filas sin tocar)", skipped)
                } else if skipped == 0 {
                    format!("✓ {} renombres propuestos · revisá y pulsá Guardar para aplicar", renamed)
                } else {
                    format!("✓ {} renombres propuestos · {} sin formato NN- (no se tocan) · pulsá Guardar",
                            renamed, skipped)
                };
                let _ = n;
            }
            if cats_should_purge {
                // Quita del state las filas con 0 repos en disco. NO toca las
                // que son nuevas en esta sesión (sino estarías borrando lo
                // que el usuario acaba de añadir manualmente).
                let mut removed_ids: Vec<String> = Vec::new();
                let counts_clone = dlg.repo_counts.clone();
                dlg.rows.retain(|r| {
                    let lookup = r.original_id.as_deref().unwrap_or(r.def.id.as_str());
                    let n_repos = counts_clone.get(lookup).copied().unwrap_or(0);
                    let keep = n_repos > 0 || r.is_new();
                    if !keep {
                        removed_ids.push(r.def.id.clone());
                    }
                    keep
                });
                // Reajustar selected_idx.
                dlg.selected_idx = if dlg.rows.is_empty() {
                    None
                } else {
                    Some(0)
                };
                dlg.status = if removed_ids.is_empty() {
                    "✓ No hay categorías vacías que borrar".to_string()
                } else {
                    format!("✓ {} categoría(s) marcada(s) para borrar: {} · pulsá Guardar para confirmar",
                            removed_ids.len(),
                            removed_ids.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                                + if removed_ids.len() > 5 { ", …" } else { "" })
                };
            }
        }

        // ── Guardar / Cancelar el editor de categorías ──
        if cats_should_save {
            self.try_save_cats_editor();
        } else if cats_should_close {
            self.cats_dialog = None;
        }

        // ── DIÁLOGO DESCUBRIDOR DE CATEGORÍAS (Fase 4) ────────────
        // Patrón idéntico a los demás: capturar acciones en flags,
        // resolverlas al final del frame fuera del borrow mutable.
        let mut disc_should_apply  = false;
        let mut disc_should_close  = false;
        let mut disc_should_simulate = false; // 🔍 Simular: predicted counts
        let mut disc_keep_only_green = false; // ✨ Mantener solo las verdes (post-simulación)
        let mut disc_filters_changed = false;
        let mut disc_select_all:    Option<bool> = None; // Some(true)=marcar, Some(false)=desmarcar

        if let Some(dlg) = self.discover_dialog.as_mut() {
            let mut open = dlg.open;
            let screen_h  = ctx.screen_rect().height();
            let win_max_h = (screen_h * 0.85).min(760.0).max(420.0);

            egui::Window::new("🔍 Descubrir categorías nuevas")
                .open(&mut open)
                .default_size([1100.0, win_max_h])
                .min_width(900.0)
                .min_height(420.0)
                .max_height(win_max_h)
                .show(ctx, |ui| {
                    // ── Header explicativo ──
                    ui.label(RichText::new(
                        "Topics oficiales de GitHub (campo 'topics' del repo) que aparecen \
                         en muchos repos tuyos pero AÚN no son una categoría. Marcá los \
                         que quieras convertir en categoría y se crean en bulk + se \
                         reclasifican los repos automáticamente."
                    ).color(theme::MUTED).size(11.0));
                    ui.separator();

                    // ── Filtros ──
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Umbral mínimo:").color(theme::FG));
                        let mut threshold = dlg.filters.threshold as i32;
                        if ui.add(egui::DragValue::new(&mut threshold)
                            .range(1..=15).speed(0.1)
                        ).changed() {
                            dlg.filters.threshold = threshold.max(1) as usize;
                            disc_filters_changed = true;
                        }
                        ui.add_space(12.0);
                        if ui.checkbox(&mut dlg.filters.hide_languages,
                            "Ocultar lenguajes"
                        ).on_hover_text(
                            "python, rust, typescript, etc. — ya viven como tag 'lang/X'"
                        ).changed() {
                            disc_filters_changed = true;
                        }
                        if ui.checkbox(&mut dlg.filters.hide_stacks,
                            "Ocultar stacks"
                        ).on_hover_text(
                            "docker, postgresql, react, etc. — ya viven como tag 'stack/X'"
                        ).changed() {
                            disc_filters_changed = true;
                        }
                        if ui.checkbox(&mut dlg.filters.hide_generic,
                            "Ocultar genéricos"
                        ).on_hover_text(
                            "ai, open-source, hacktoberfest, framework, automation, etc."
                        ).changed() {
                            disc_filters_changed = true;
                        }
                    });
                    ui.separator();

                    // ── Header de la tabla ──
                    ui.horizontal(|ui| {
                        ui.add_sized([24.0,  18.0], egui::Label::new(RichText::new("✓").strong()));
                        ui.add_sized([46.0,  18.0], egui::Label::new(RichText::new("Repos").strong()))
                            .on_hover_text("Cantidad de repos con este topic exacto (incluye sinónimos fusionados).");
                        ui.add_sized([56.0,  18.0], egui::Label::new(RichText::new("Predicted").strong()))
                            .on_hover_text(
                                "Cuántos repos REALMENTE caerían en esta categoría según el algoritmo \
                                 de classify_heuristic (cuenta SOLO los marcados). \
                                 'Repos' es el universo potencial; 'Predicted' es lo que realmente se mueve. \
                                 Se calcula con el botón '🔍 Simular' del footer.");
                        ui.add_sized([180.0, 18.0], egui::Label::new(RichText::new("Topic").strong()));
                        ui.add_sized([220.0, 18.0], egui::Label::new(RichText::new("ID propuesto").strong()));
                        ui.add_sized([260.0, 18.0], egui::Label::new(RichText::new("Sinónimos / ejemplo").strong()));
                    });
                    ui.separator();

                    // ── Lista scrollable ──
                    let scroll_max_h = ui.available_height() - 80.0;
                    ScrollArea::vertical()
                        .id_salt("discover_scroll")
                        .max_height(scroll_max_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for cand in dlg.candidates.iter_mut() {
                                ui.horizontal(|ui| {
                                    ui.add_sized([24.0, 22.0],
                                        egui::Checkbox::new(&mut cand.selected, ""));
                                    let count_color = if cand.count >= 10 { theme::ACCENT_H }
                                                       else if cand.count >= 5 { theme::ACCENT }
                                                       else { theme::MUTED };
                                    ui.add_sized([46.0, 22.0], egui::Label::new(
                                        RichText::new(format!("{}", cand.count))
                                            .monospace().strong().color(count_color)));
                                    // Predicted: cuántos repos REALMENTE se moverían.
                                    let pred_text = match cand.predicted {
                                        Some(0) => "0 ⚠".to_string(),
                                        Some(n) => format!("{}", n),
                                        None    => "—".to_string(),
                                    };
                                    let pred_color = match cand.predicted {
                                        Some(0) => theme::ERR,
                                        Some(n) if n < cand.count / 2 => theme::WARN,
                                        Some(_) => theme::OK,
                                        None    => theme::MUTED,
                                    };
                                    ui.add_sized([56.0, 22.0], egui::Label::new(
                                        RichText::new(pred_text)
                                            .monospace().strong().color(pred_color)));
                                    ui.add_sized([180.0, 22.0], egui::Label::new(
                                        RichText::new(&cand.topic).color(theme::FG).size(12.0)));
                                    // ID propuesto editable.
                                    ui.add_sized([220.0, 22.0],
                                        egui::TextEdit::singleline(&mut cand.id_propuesto)
                                            .font(egui::TextStyle::Monospace));
                                    // Sinónimos fusionados + sample de repos.
                                    let extra = if cand.merged.is_empty() {
                                        format!("ej: {}", cand.repos.iter().take(2)
                                                .cloned().collect::<Vec<_>>().join(", "))
                                    } else {
                                        format!("+ {}  ·  ej: {}",
                                                cand.merged.join(", "),
                                                cand.repos.iter().take(2)
                                                    .cloned().collect::<Vec<_>>().join(", "))
                                    };
                                    ui.add_sized([260.0, 22.0], egui::Label::new(
                                        RichText::new(extra).color(theme::MUTED).size(10.5)));
                                });
                            }
                        });

                    // ── Footer en DOS LÍNEAS ──
                    //   L1: info de estado (counts + status del último cómputo)
                    //   L2: opciones (bulk a la izq · acciones principales a la der)
                    let count_sel = dlg.candidates.iter().filter(|c| c.selected).count();
                    let total = dlg.candidates.len();
                    let any_simulated = dlg.candidates.iter()
                        .any(|c| c.predicted.is_some());
                    let n_green: usize = dlg.candidates.iter()
                        .filter(|c| matches!(c.predicted, Some(p) if p > 0 && p >= c.count / 2))
                        .count();

                    ui.separator();

                    // ── L1: INFO ──────────────────────────────────────
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!(
                            "📊  {} candidato(s) visible(s)  ·  {} seleccionado(s)",
                            total, count_sel
                        )).color(theme::FG).size(11.5).strong());
                        if any_simulated {
                            ui.label(RichText::new(format!(
                                "·  🟢 {} verde(s) tras simular",
                                n_green
                            )).color(theme::OK).size(11.5));
                        }
                        if !dlg.status.is_empty() {
                            ui.add_space(10.0);
                            let color = if dlg.status.starts_with('✗') { theme::ERR }
                                        else if dlg.status.starts_with('⚠') { theme::WARN }
                                        else if dlg.status.starts_with('✓')
                                             || dlg.status.starts_with('✨') { theme::OK }
                                        else { theme::MUTED };
                            ui.label(RichText::new(&dlg.status).color(color).size(11.0));
                        }
                    });

                    // ── L2: OPCIONES ──────────────────────────────────
                    ui.horizontal(|ui| {
                        // Bulk a la izquierda
                        ui.label(RichText::new("Bulk:").color(theme::MUTED).size(11.0));
                        if ui.button("✓ Marcar todos").clicked() {
                            disc_select_all = Some(true);
                        }
                        if ui.button("☐ Desmarcar todos").clicked() {
                            disc_select_all = Some(false);
                        }
                        // ✨ Solo verdes — habilitado solo si ya se simuló al menos una.
                        // "Verde" = predicted >= count/2 (mismo criterio que el color
                        // del badge). Desmarca rojas, amarillas y las no simuladas.
                        let green_btn = egui::Button::new(
                            RichText::new(format!("✨ Solo verdes ({})", n_green))
                                .color(if any_simulated { theme::OK } else { theme::MUTED })
                        );
                        if ui.add_enabled(any_simulated, green_btn)
                            .on_hover_text(
                                "Mantiene seleccionadas SOLO las categorías cuyo \
                                 'Predicted' es verde (al menos la mitad del universo \
                                 del topic). Desmarca rojas (vacías), amarillas (poco \
                                 rendimiento) y las no simuladas. Disponible solo \
                                 después de pulsar 🔍 Simular."
                            ).clicked() {
                            disc_keep_only_green = true;
                        }
                        // Acciones principales a la derecha (en right_to_left
                        // se agregan en orden inverso al visual:
                        //   visual: [🔍 Simular] [Cancelar] [💾 Crear y reclasificar]
                        //   código: 💾 → Cancelar → 🔍).
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let apply = egui::Button::new(
                                RichText::new(format!("💾  Crear {} y reclasificar", count_sel))
                                    .color(Color32::WHITE)
                            ).fill(theme::ACCENT);
                            if ui.add_enabled(count_sel > 0, apply).clicked() {
                                disc_should_apply = true;
                            }
                            if ui.button("Cancelar").clicked() {
                                disc_should_close = true;
                            }
                            let sim_btn = egui::Button::new(
                                RichText::new("🔍  Simular").color(theme::ACCENT_H));
                            if ui.add_enabled(count_sel > 0, sim_btn)
                                .on_hover_text(
                                    "Ejecuta el clasificador con un config provisional \
                                     que incluye SOLO los candidatos marcados y muestra \
                                     en la columna 'Predicted' cuántos repos efectivamente \
                                     caerían en cada uno. Los que queden en 0 NO conviene crear."
                                ).clicked() {
                                disc_should_simulate = true;
                            }
                        });
                    });
                });
            if !open { disc_should_close = true; }
            dlg.open = open;
        }

        // ── Resolver acciones diferidas del descubridor ──
        if let Some(dlg) = self.discover_dialog.as_mut() {
            if let Some(value) = disc_select_all {
                for c in &mut dlg.candidates { c.selected = value; }
            }
            if disc_keep_only_green {
                // Mismo criterio que el color del badge:
                //   Some(0)               → ROJO  (descartar)
                //   Some(n) si n < ct/2   → AMBAR (descartar)
                //   Some(n) (n >= ct/2)   → VERDE (mantener)
                //   None                  → no simulado (descartar — falta data)
                let mut kept = 0usize;
                let mut dropped = 0usize;
                for c in dlg.candidates.iter_mut() {
                    let is_green = matches!(c.predicted, Some(p) if p > 0 && p >= c.count / 2);
                    if c.selected {
                        if is_green { kept += 1; }
                        else        { c.selected = false; dropped += 1; }
                    }
                }
                dlg.status = format!(
                    "✨ Mantenidas {} categoría(s) verde(s) · descartadas {} (rojas/ámbar/sin simular)",
                    kept, dropped,
                );
            }
            if disc_filters_changed {
                // Re-ejecutar el descubridor con los nuevos filtros, preservando
                // las selecciones del usuario para los topics que sigan visibles.
                let prev_selected: std::collections::HashSet<String> = dlg.candidates.iter()
                    .filter(|c| c.selected)
                    .map(|c| c.topic.clone())
                    .collect();
                let mut new_cands = discover_candidates(
                    &dlg.source_repo_ids,
                    &dlg.source_cubiertos,
                    dlg.filters,
                    dlg.next_consecutivo,
                );
                for c in &mut new_cands {
                    if prev_selected.contains(&c.topic) { c.selected = true; }
                }
                dlg.status = format!("Filtros actualizados · {} candidatos visibles",
                                     new_cands.len());
                dlg.candidates = new_cands;
            }
        }

        // ── Simular: pobla `predicted` por candidato seleccionado ──
        // Lo hacemos en el main thread porque tarda <1s con 157 repos
        // (el cuello de botella es scan_repo, ya cacheado en filesystem).
        if disc_should_simulate {
            if let Some(dlg) = self.discover_dialog.as_mut() {
                let root = std::path::PathBuf::from(&self.root);
                let cfg_actual = load_cats_cfg(&root);

                // Snapshot OWNED de los marcados para evitar conflicto de borrows
                // cuando después escribamos `cand.predicted`. Tupla (id, topic, merged).
                let chosen_data: Vec<(String, String, Vec<String>)> = dlg.candidates.iter()
                    .filter(|c| c.selected)
                    .map(|c| (
                        c.id_propuesto.trim().to_string(),
                        c.topic.clone(),
                        c.merged.clone(),
                    ))
                    .collect();
                let total_chosen = chosen_data.len();

                // Construir cfg provisional inyectando los candidatos marcados.
                let mut cfg_sim = cfg_actual.clone();
                let mut nuevas: Vec<CategoriaDef> = Vec::new();
                for (id, topic, merged) in &chosen_data {
                    let mut boosts: Vec<(String, u32)> = vec![(topic.clone(), 12)];
                    for syn in merged { boosts.push((syn.clone(), 10)); }
                    nuevas.push(CategoriaDef {
                        id:           id.clone(),
                        keywords:     Vec::new(),
                        topic_boosts: boosts,
                    });
                }
                if cfg_sim.categorias.is_empty() {
                    cfg_sim.categorias.extend(nuevas.iter().cloned());
                } else {
                    let last_idx = cfg_sim.categorias.len() - 1;
                    for (offset, n) in nuevas.iter().enumerate() {
                        cfg_sim.categorias.insert(last_idx + offset, n.clone());
                    }
                }

                // Ejecutar la simulación con el cfg in-memory.
                let new_ids: std::collections::HashSet<String> = nuevas.iter()
                    .map(|n| n.id.clone()).collect();
                match crate::reclassify::compute_reclassification_with_cfg(&root, &cfg_sim) {
                    Ok(changes) => {
                        // Contar cuántos repos se moverían a cada nueva categoría.
                        let mut counts: std::collections::HashMap<String, usize> =
                            std::collections::HashMap::new();
                        for ch in &changes {
                            if new_ids.contains(&ch.proposed_cat) {
                                *counts.entry(ch.proposed_cat.clone()).or_insert(0) += 1;
                            }
                        }
                        let total_predicted: usize = counts.values().sum();
                        // Asignar `predicted` a cada candidato (ya no hay conflicto
                        // de borrows porque chosen_data es owned).
                        for cand in dlg.candidates.iter_mut() {
                            if cand.selected {
                                let id = cand.id_propuesto.trim().to_string();
                                cand.predicted = Some(counts.get(&id).copied().unwrap_or(0));
                            } else {
                                cand.predicted = None;
                            }
                        }
                        let zeros = chosen_data.iter().filter(|(id, _, _)| {
                            counts.get(id).copied().unwrap_or(0) == 0
                        }).count();
                        dlg.status = if zeros > 0 {
                            format!("⚠ Simulación: {} repos se moverían en total · \
                                     {} de {} categorías quedarían vacías (badge rojo)",
                                    total_predicted, zeros, total_chosen)
                        } else {
                            format!("✓ Simulación: {} repos se moverían en total · \
                                     todas las {} categorías marcadas tendrían contenido",
                                    total_predicted, total_chosen)
                        };
                    }
                    Err(e) => {
                        dlg.status = format!("✗ Error simulando: {}", e);
                    }
                }
            }
        }

        // ── Aplicar / Cerrar ──
        if disc_should_apply {
            // Cerrar diálogo INMEDIATAMENTE y disparar worker.
            if let Some(dlg) = self.discover_dialog.take() {
                let chosen: Vec<TopicCandidate> = dlg.candidates.into_iter()
                    .filter(|c| c.selected)
                    .collect();
                self.start_apply_discover(chosen);
            }
        } else if disc_should_close {
            self.discover_dialog = None;
        }

        // ── DIÁLOGO CREDENCIALES ─────────────────────────────────
        let mut close_api = false;
        if let Some(dlg) = self.api_dialog.as_mut() {
            let mut open = true;
            egui::Window::new("🔐 Configurar credenciales")
                .open(&mut open)
                .default_size([620.0, 380.0])
                .show(ctx, |ui| {
                    ui.label(RichText::new(
                        "Se guardan cifradas con Windows DPAPI (solo este usuario en este PC \
                         puede descifrarlas)."
                    ).color(theme::MUTED).size(11.0));
                    ui.label(RichText::new(format!("Archivo: {}", config_path().display()))
                             .color(theme::MUTED).monospace().size(10.0));
                    ui.add_space(10.0);

                    // ── Anthropic API key ──────────────────────────
                    ui.label(RichText::new("Anthropic API key").strong().color(theme::ACCENT_H));
                    ui.horizontal(|ui| {
                        ui.label("Key:");
                        let edit = egui::TextEdit::singleline(&mut dlg.key)
                            .password(!dlg.show)
                            .desired_width(420.0);
                        ui.add(edit);
                        ui.checkbox(&mut dlg.show, "👁");
                    });
                    ui.label(RichText::new(
                        "Empieza con sk-ant-… · obtén una en https://console.anthropic.com/settings/keys"
                    ).color(theme::MUTED).size(10.0));
                    ui.add_space(10.0);

                    // ── GitHub PAT ─────────────────────────────────
                    ui.label(RichText::new("GitHub Personal Access Token (opcional)")
                             .strong().color(theme::ACCENT_H));
                    ui.horizontal(|ui| {
                        ui.label("PAT:");
                        let edit = egui::TextEdit::singleline(&mut dlg.pat)
                            .password(!dlg.show_pat)
                            .desired_width(420.0);
                        ui.add(edit);
                        ui.checkbox(&mut dlg.show_pat, "👁");
                    });
                    ui.label(RichText::new(
                        "Eleva el rate-limit de GitHub API de 60 → 5000 req/h. \
                         Crea uno fine-grained en https://github.com/settings/tokens?type=beta \
                         con permiso 'Metadata: Read-only' sobre repos públicos."
                    ).color(theme::MUTED).size(10.0));
                    ui.add_space(8.0);

                    // ── Status ─────────────────────────────────────
                    ui.label(RichText::new(&dlg.status).color(theme::ACCENT_H).size(11.0));
                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(
                            RichText::new("🗑  Borrar todo").color(theme::ERR)
                        )).clicked() {
                            // Limpia tanto Anthropic como PAT del config.dat.
                            let _ = save_github_pat("");
                            let _ = delete_api_key();
                            dlg.key.clear();
                            dlg.pat.clear();
                            dlg.status = "✓ Credenciales eliminadas.".to_string();
                            self.use_llm = false;
                        }
                        if ui.button("🔌  Probar Anthropic").clicked() {
                            let k = dlg.key.trim().to_string();
                            if k.is_empty() {
                                dlg.status = "⚠ Pega primero una Anthropic key.".to_string();
                            } else {
                                // test_api_key es bloqueante (~1-15s); por simplicidad
                                // mantenemos la llamada sincrónica.
                                match test_api_key(&k) {
                                    Ok(reply) => {
                                        let preview: String = reply.chars().take(60).collect();
                                        dlg.status = format!("✓ Anthropic OK · \"{}\"", preview);
                                    }
                                    Err(e) => {
                                        dlg.status = format!("✗ Anthropic: {}", e);
                                    }
                                }
                            }
                        }
                        if ui.button("🔌  Probar GitHub").clicked() {
                            let p = dlg.pat.trim().to_string();
                            if p.is_empty() {
                                dlg.status = "⚠ Pega primero un GitHub PAT.".to_string();
                            } else {
                                // Llamada sincrónica a /rate_limit (~1-3s).
                                match test_github_pat(&p) {
                                    Ok(info) => {
                                        dlg.status = format!("✓ GitHub OK · {}", info);
                                    }
                                    Err(e) => {
                                        dlg.status = format!("✗ GitHub: {}", e);
                                    }
                                }
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let save = egui::Button::new(
                                RichText::new("💾  Guardar").color(Color32::WHITE)
                            ).fill(theme::ACCENT);
                            if ui.add(save).clicked() {
                                let k = dlg.key.trim().to_string();
                                let pat = dlg.pat.trim().to_string();
                                if k.is_empty() && pat.is_empty() {
                                    dlg.status = "⚠ Pega al menos una credencial.".to_string();
                                } else {
                                    let mut errs: Vec<String> = Vec::new();
                                    if !k.is_empty() {
                                        if let Err(e) = save_api_key(&k) {
                                            errs.push(format!("Anthropic: {}", e));
                                        } else {
                                            self.use_llm = true;
                                        }
                                    }
                                    // PAT: guardar siempre (incluso vacío vacía la entrada)
                                    if let Err(e) = save_github_pat(&pat) {
                                        errs.push(format!("PAT: {}", e));
                                    }
                                    if errs.is_empty() {
                                        close_api = true;
                                    } else {
                                        dlg.status = format!("✗ {}", errs.join(" · "));
                                    }
                                }
                            }
                            if ui.button("Cancelar").clicked() {
                                close_api = true;
                            }
                        });
                    });
                });
            if !open { close_api = true; }
        }
        if close_api {
            // Breadcrumb resumen al log principal.
            match (has_api_key(), has_github_pat()) {
                (true,  true)  => self.append_log("✓ Credenciales guardadas (Anthropic + GitHub PAT)", theme::OK),
                (true,  false) => self.append_log("✓ Anthropic key guardada · sin GitHub PAT", theme::OK),
                (false, true)  => self.append_log("✓ GitHub PAT guardado · sin Anthropic key", theme::OK),
                (false, false) => {}
            }
            self.api_dialog = None;
        }
    }
}
