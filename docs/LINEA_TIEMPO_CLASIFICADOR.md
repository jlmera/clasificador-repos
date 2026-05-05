# Línea de tiempo — CLASIFICADOR DE REPOSITORIOS

**Proyecto**: Clasificador de Repositorios GitHub
**Autor**: Jorge Mera (jlmera)
**Periodo de construcción**: 28-Abr a 4-May 2026
**Versión final**: 1.0.0 (release el 4-May-2026)
**Repositorio raíz**: `D:\DUGOTEX\11 - IA\GitHub\`

---

## Contexto inicial

Carpeta con **94 repositorios de GitHub clonados sin organización** durante meses (creciendo a 157 al cierre de la v1.0). Sin índice, sin categorías, sin forma de buscarlos ni saber qué tenía cada uno. El reto era diseñar y construir una herramienta que **(a)** clasificara los existentes en categorías sensatas, **(b)** procesara repos nuevos llegando al `_inbox/`, **(c)** ofreciera una manera de buscarlos, **(d)** mantuviera todo sincronizado en el tiempo, y **(e)** evolucionara la taxonomía a medida que el universo creciera.

---

## Fases del proyecto

### Fase 0 — Inventario y categorización · 28-Abr 04:00–04:30

- Análisis automatizado de los READMEs y manifiestos (`package.json`, `composer.json`, `Cargo.toml`, `go.mod`, etc.) de los 94 repos iniciales.
- Diseño de **10 categorías** que cubren todo el espectro: `01-claude-code`, `02-mcp-y-conectores`, `03-agentes-y-llms`, `04-listas-curadas`, `05-self-hosted`, `06-infraestructura-core`, `07-multimedia-y-conversion`, `08-productividad`, `09-sistema-windows-linux`, `10-utilidades-dev`.
- **Hito 1**: documento maestro `PLAN_CATEGORIZACION_GITHUB.md` con la asignación completa.
- **Hito 2**: scripts PowerShell `mover_repos.ps1` para reorganizar el árbol físicamente.
- **Hito 3**: carpeta `_inbox/` para futuros clones, antes de clasificarlos.

### Fase 1 — Buscador web e índice · 28-Abr 04:30–05:30

- Generación de `repos_index.json` con metadata enriquecida por repo (descripción, lenguaje principal, stack, tags).
- Construcción de `buscador.html` autocontenido — un solo archivo navegable offline con paginación, filtros por categoría/lenguaje/stack, búsqueda full-text y enlaces directos a cada repo.
- **Hito 4**: paleta verde sage `#8FB46B` adoptada como identidad visual del proyecto (consistente con la del entorno DUGOTEX).
- **Hito 5**: wiki Obsidian generada con `build_wiki.py` siguiendo el patrón LLM Wiki / Karpathy — cada repo es un nodo con wikilinks y frontmatter YAML.

### Fase 2 — CLI Python · 28-Abr (mismo día más tarde)

- Diseño modular: `clasificador.py` con módulos `scan`, `classify`, `duplicates`, `ids`, `index`, `html`, `readme`, `wiki`, `secrets`.
- Compilación con PyInstaller `--onefile --windowed` → `clasificador.exe`.
- **Hito 6**: detección de duplicados por nombre y por URL del remote git normalizada (resuelve el caso "el mismo repo clonado dos veces con nombres distintos").
- **Hito 7**: integración con la **GitHub API** para obtener IDs numéricos y URLs canónicas (cache en `repo_ids.json` con `last_attempt` para no machacar el rate-limit anonymous de 60 req/h).
- **Hito 8**: auto-traducción de descripciones al español usando **Anthropic Claude API** (modelo `claude-sonnet-4-6`).

### Fase 3 — GUI Python con tkinter · 28-Abr (tarde-noche)

- `clasificador_gui.py` con tema verde sage, paneles de log, barra de progreso animada.
- Threading con `queue.Queue` para no bloquear el hilo de UI durante operaciones largas.
- **Hito 9**: diálogo interactivo "Resolver duplicados" — el usuario decide caso por caso si archivar, saltar, reemplazar, borrar o renombrar.
- **Hito 10**: `secret_store.py` cifra la API key de Anthropic con **Windows DPAPI** (`CryptProtectData`) — la key vive cifrada en `config.dat` junto al `.exe`, descifrable solo por el usuario+máquina que la creó.
- **Hito 11**: build a `clasificadopy.exe` (13.14 MB, modo windowed sin consola).

### Fase 4 — Refinamiento UX · 28-Abr noche

- Render de READMEs a HTML formateado dentro de cada repo (`_README.html`) con caché incremental por mtime.
- Paginación, sort numérico de la columna `#`, eliminación de footer.
- Logo SVG + header rediseñado compacto.
- **Hito 12**: traducción de READMEs completos al español con LLM (`render_readme_html_es`), también con caché incremental para no pagar tokens dos veces.
- Renombre `clasificador-gui.exe` → `clasificadopy.exe` para diferenciarlo del CLI.
- Limpieza: `_tools/` → `tools/`, `_wiki/` → `wiki/`.

### Fase 5 — Port completo a Rust · 28-Abr / 29-Abr

- Decisión de migrar de Python a Rust por velocidad de arranque, tamaño de binario y robustez.
- **Stack Rust elegido**: `eframe + egui` (GUI inmediata), `ureq` (HTTP), `pulldown-cmark` (Markdown), `walkdir`, `regex`, `chrono`, `serde`, `windows-rs 0.58` (DPAPI), `winres` (icono embebido), `image` (PNG/ICO).
- **Hito 13**: paridad funcional 1:1 con la versión Python, incluyendo schema JSON 100% compatible.
- **Hito 14**: DPAPI portable Python ↔ Rust (mismo `config.dat` se descifra desde ambos binarios).
- **Hito 15**: `clasificadors.exe` GUI compila a 7.48 MB (vs 13.14 MB de Python — **−43%**).
- **Hito 16**: arranque <200 ms (vs ~1.5 s de Python — **−87%**).
- **Hito 17**: reindex completo <2 s (vs ~30 s de Python — **−93%**).

### Fase 6 — Consolidación 1.0 · 29-Abr (día final)

- **Hito 18**: eliminación de toda mención visible de "Rust" en la UI — el usuario no debe ver el detalle de implementación.
- **Hito 19**: renombre `clasificadors.exe` → `clasificador.exe` (sin sufijo de lenguaje).
- **Hito 20**: artefactos definitivos sin sufijos `_rs`: `tools/repos_index.json`, `tools/repo_ids.json`, `buscador.html`.
- **Hito 21**: renombre `tools/` → `data/` (la carpeta ya no son herramientas, es estado persistente).
- **Hito 22**: renombre `clasificador-rs/` → `fuente/` y `_build_definitivo.bat` movido dentro con paths relativos auto-detectados.
- **Hito 23**: bandera de Colombia 🇨🇴 como icono real (PNG 60×40 embebido vía `include_bytes!` + carga lazy a `egui::TextureHandle`) en el botón "Traducir READMEs". En el HTML como SVG inline. Adopción explícita de la nacionalidad del proyecto.
- **Hito 24**: HTML con favicon embebido como data URI base64, título en mayúsculas `BUSCADOR DE REPOSITORIOS`, numeración con padding `01, 02, …`.
- **Hito 25**: reorganización de botones de la GUI en 3 filas conceptuales — **Flujo principal** (Resolver duplicados / Escanear / Aplicar) + **Mantenimiento** (Solo reindexar / API Key / Limpiar log) en una misma línea con justificación a derecha; **Enriquecer y publicar** (Refrescar IDs / Traducir / Wiki / Abrir buscador) en la siguiente.
- **Hito 26**: marquee de progreso reescrito a mano — segmento del 30% recorre el track sin titilar; en barra determinada se añade un sheen blanco translúcido del 6.25% del fill que recorre cíclicamente para indicar "estoy vivo" entre items en operaciones largas.
- **Hito 27**: animación basada en `ctx.input(|i| i.time)` (reloj absoluto del frame) en vez de acumulado — independiente del FPS y del movimiento del mouse.

### Fase 7 — Auditoría de performance · 29-Abr (final)

- Análisis sistemático de hot paths con tiempos reales medidos.
- **Hito 28**: regex compiladas como `static Lazy<Regex>` (4 en `scan.rs`, 2 en `duplicates.rs`) — antes se compilaban 488 veces por reindex, ahora 6 veces totales en toda la sesión.
- **Hito 29**: `Cargo.toml` con `lto = "fat"` + `incremental = false` — binario final pasa de 7.48 MB a 7.08 MB (**−5.4%** adicional). LTO completo entre crates con linker reordenando funciones.
- **Hito 30**: limpieza del campo huérfano `spinner_phase` y del patrón `if exists() + read_to_string()` doble syscall, reemplazado por `read_to_string()` directo.

### Fase 8 — Categorías editables · 30-Abr

- **Sub-fase 3a — Config externalizado**: la taxonomía se mueve de constantes hardcoded a `data/categorias.json`. Nuevo módulo `categories_config.rs` con auto-bootstrap desde defaults si el archivo no existe.
- **Sub-fase 3b — Reclasificación masiva**: nuevo botón "🔁 Reclasificar todo" que evalúa los 157 repos contra el config actual y muestra un diálogo con los cambios propuestos para que el usuario los apruebe selectivamente.
- **Sub-fase 3c — Editor visual**: diálogo "🏷 Categorías" con panel doble (lista + editor de keywords/topic_boosts), añadir/quitar/renombrar/reordenar. Renombrar el id dispara `fs::rename` físico de la carpeta; eliminar con repos dentro los migra al fallback antes de borrar.
- **Hito 31**: `data/categorias.json` editable + 3 nuevos diálogos modales con worker threads.

### Fase 9 — Robustez y atomicidad · 3-May

- **Trigger**: `repo_ids.json` apareció truncado a 24 KB con solo 43 de 157 repos cacheados; la última entrada cortada en `"awesome-selfhosted"...s`. Causa: race condition entre `fs::write` no atómico y Syncthing entre dos máquinas.
- **Hito 32**: nuevo módulo `atomic_io.rs` con `write_atomic_string` (write a `.tmp` + fsync + `rename` atómico vía `MoveFileExW`).
- Aplicado a 6 archivos críticos: `repo_ids.json`, `categorias.json`, `repos_index.json`, `descripciones_es.json`, `buscador.html`, `config.dat`.
- Recovery: refresh completo regeneró el cache de 157 repos en 67.7s. Nunca volvió a corromperse.

### Fase 10 — Descubridor automático de categorías · 3-May

- Nuevo módulo `topic_discovery.rs` (~324 líneas) que escanea topics oficiales de GitHub en `repo_ids.json` y propone candidatos a categoría nueva basándose en frecuencia.
- **Filtros default**: ocultar lenguajes, ocultar stacks, ocultar genéricos. Sobre 157 repos: 896 topics distintos → 94 con ≥3 ocurrencias → 69 candidatos tras filtros.
- **Detección de sinónimos**: plural↔singular (`agent`↔`agents`), sufijo `-cli` (`gemini`↔`gemini-cli`). Fusiona en un único candidato sumando los repos.
- **Hito 33**: nuevo botón "🔍 Descubrir categorías" + diálogo modal con tabla editable + botón "🔍 Simular" que ejecuta `compute_reclassification` con un config provisional para mostrar predicted counts reales.
- **Hito 34**: botón "✨ Solo verdes" que desmarca rojos (predicted=0) y amarillos (predicted<count/2), dejando solo los candidatos que realmente se van a poblar al aplicar.

### Fase 11 — Refinamientos UX · 3-May / 4-May

- **Hito 35**: botón "🔢 Compactar numeración" en el editor de categorías (renombra todas las filas a NN- consecutivo respetando IDs custom).
- **Hito 36**: botón "🧹 Borrar vacías (N)" que cuenta automáticamente categorías sin repos físicos y las elimina del config + carpeta.
- **Hito 37**: bug fix del prefijo numérico duplicado en `apply_discover_worker` (validación NN- único).
- **Hito 38**: footer del descubridor reorganizado en 2 líneas (info arriba, opciones abajo).
- **Hito 39**: log multilínea con menú contextual (right-click) para "Copiar todo el log" + "Limpiar log". Refactor a un único `Label` con `LayoutJob` rico que preserva colores por línea pero permite selección continua del mouse a través de varias líneas + Ctrl+C nativo.
- **Hito 40**: header rediseñado: eliminado el cuadrito con emoji `📂`; título alineado a la izquierda + skyline decorativo de barras verticales con altura aleatoria y degradado verde oscuro→pastel a su derecha.

### Fase 12 — Refresh inteligente con ETag · 4-May

- Implementación de `If-None-Match` (HTTP conditional GET) en `fetch_github_meta`. Cada response 200 OK guarda el header `etag` en el `RepoMeta` cacheado.
- En el siguiente refresh, los repos sin cambios upstream responden **304 Not Modified** sin body. Según docs oficiales de GitHub, los 304 NO consumen cuota del rate-limit primario.
- **Hito 41**: nuevo enum `FetchOutcome::{Updated, NotModified, Failed}` + struct `GhFetchStats` para medir `fetched/not_modified/failed/skipped`.
- **Hito 42**: log del refresh muestra "📡 GitHub API: 5 cambiados · 152 sin cambios (304) · 0 fallaron · 💡 97% no consumieron rate limit".
- En refreshes sucesivos: tiempo total **~25s** vs ~70s del primer pase. **5-10× más eficiente**.

### Fase 13 — Refactor modular de gui/app.rs · 4-May

- **Trigger**: `gui/app.rs` había crecido a 3707 líneas. Borrow-checker hostil, navegación tediosa.
- **Hito 43**: split en 5 módulos auto-contenidos:
  - `gui/types.rs` (172 líneas) — `WorkerMsg` + 5 `DialogState` + `DuplicateItem` + `CatRow`
  - `gui/helpers.rs` (140 líneas) — `validate_and_diff_cats`, `count_repos_per_category`, `open_url`
  - `gui/workers.rs` (601 líneas) — los 4 `apply_X_worker` que corren en `thread::spawn`
  - `gui/app.rs` (2826 líneas) — `ClasificadorApp` + `impl` + `update()` + `run_worker`
- Reducción: app.rs **−24% (-881 líneas)**. Sigue grande por el `update()` con renders inline (refactor a `dialogs/X.rs` queda como Oleada 3 del roadmap).

### Fase 14 — Optimizaciones finales y release v1.0.0 · 4-May

- **Hito 44**: 3 regex movidas a `Lazy<Regex>` estáticas en `apply_actions.rs` y `ids.rs` (`RE_GIT_LOG_TS` × 2, `RE_GH_URL`). Antes se compilaban 514 veces por reindex (157 repos × 2 + 200 duplicados); ahora 3 totales por sesión.
- **Hito 45**: 2 `cap.get(1).unwrap()` peligrosos reemplazados por `if let Some(...)` en parsers de timestamp git.
- **Hito 46**: dead code eliminado en `llm.rs:209-220` (cómputo de offsets que se descartaban con `let _ = ...`).
- **Hito 47**: 4 imports `unused` limpiados de `gui/app.rs` post-refactor.
- **RELEASE 1.0.0**: 7.30 MB, ~25 s refresh con ETag, 157 repos, ~25 categorías, todos los flujos atómicos a disco.

---

## Métricas finales (v1.0.0)

| Dimensión | Python (clasificadopy.exe) | Rust v1.0.0 | Mejora |
|---|---|---|---|
| Tamaño del binario | 13.14 MB | 7.30 MB | **−44%** |
| Arranque GUI | ~1.5 s | <500 ms | **−67%** |
| Reindex completo (157 repos) | ~30 s | ~1 s | **−97%** |
| Refresh GitHub IDs primer pase | ~70 s | ~70 s | igual (limitado por red) |
| Refresh GitHub IDs siguientes pases | n/a | ~25 s | **5-10× con ETag** |
| Líneas de código Rust | n/a | 7 653 | en 27 módulos |
| Dependencias en runtime | Python 3.12 + libs | ninguna (estático) | autónomo |

| Datos del catálogo (4-May 2026) | Valor |
|---|---|
| Repos catalogados | 157 |
| Categorías activas | ~25 (10 originales + 15 descubiertas/refinadas) |
| Topics distintos en cache | 896 |
| GitHub IDs resueltos | 157 / 157 (100%) |
| Repos con `topics` upstream | 124 (33 sin etiquetar por sus autores) |
| Archivos atómicos a disco | 6 (repo_ids, categorias, index, descripciones, html, config.dat) |

---

## Stack tecnológico final

**Lenguaje**: Rust 1.95 (toolchain `stable-x86_64-pc-windows-msvc`)

**Crates**:
- GUI: `eframe 0.29`, `egui 0.29`
- HTTP: `ureq 2`
- Serialización: `serde 1`, `serde_json 1`
- Filesystem: `walkdir 2`
- Regex: `regex 1`, `once_cell 1`
- Markdown: `pulldown-cmark 0.13`
- Encoding: `encoding_rs 0.8`
- Tiempo: `chrono 0.4`
- CLI: `clap 4.5`
- Imágenes: `image 0.25`
- Diálogos: `rfd 0.15`
- Windows API: `windows 0.58` (DPAPI)
- Resources: `winres 0.1` (build script)

**APIs externas**:
- GitHub API con PAT (rate-limit 5000 req/h) + `If-None-Match` para conditional GET
- Anthropic Claude API (modelo `claude-sonnet-4-6`) para traducción de READMEs

**Identidad visual**:
- Color primario: verde sage `#8FB46B`
- Color secundario: verde oscuro `#496229`
- Tipografía monospace para datos técnicos, sans-serif del sistema para UI

**Compilación de release**:
- `opt-level = 3`
- `lto = "fat"`
- `codegen-units = 1`
- `strip = true`
- `panic = "abort"`

---

## Estructura final del proyecto

```
D:\DUGOTEX\11 - IA\GitHub\
├── 01-claude-code/  …  NN-{descubierta}/      [~25 categorías con 157 repos]
├── _inbox/                                    [zona de aterrizaje]
├── _duplicados/                               [archivo opcional de duplicados]
├── data/                                      [estado persistente, atomic-write]
│   ├── categorias.json                        [taxonomía editable]
│   ├── repos_index.json                       [índice consumido por buscador.html]
│   ├── repo_ids.json                          [cache GitHub: id + topics + etag]
│   └── descripciones_es.json                  [traducciones LLM cacheadas]
├── fuente/                                    [código Rust — 7 653 líneas en src/]
│   ├── Cargo.toml, src/, templates/
│   ├── clasificador.ico, bandera_co.png
│   ├── _build_definitivo.bat                  [build incremental + verificación]
│   └── _build_limpio.bat                      [cargo clean + build desde cero]
├── wiki/                                      [vault Obsidian generado]
├── clasificador.exe                           [GUI v1.0.0, 7.30 MB]
├── buscador.html                              [buscador web, ~140 KB]
├── roadmap_clasificador.pdf                   [25 features futuras priorizadas]
├── HISTORIA_PROYECTO.txt                      [historia narrativa]
└── LINEA_TIEMPO_CLASIFICADOR.md               [este archivo]

%APPDATA%\jlmera\clasificador\
└── config.dat                                 [Anthropic key + GitHub PAT, DPAPI cifrado]
```

---

## Hitos pivotales de la v1.0.0 (en orden cronológico)

Si vas a renderizar como timeline visual, estos son los **15 puntos pivotales**:

1. **28-Abr 04:00** — Inventario inicial: 94 repos sin clasificar
2. **28-Abr 04:30** — 10 categorías definidas + reorganización física
3. **28-Abr 05:30** — Buscador web `buscador.html` operativo
4. **28-Abr (tarde)** — CLI Python con clasificador modular
5. **28-Abr (noche)** — GUI Python `clasificadopy.exe` con DPAPI, threading, diálogo de duplicados
6. **28-Abr / 29-Abr** — Port completo a Rust → `clasificadors.exe` (paridad 1:1, 87% más rápido)
7. **29-Abr (día)** — Consolidación 1.0: renombres, bandera Colombia, reorganización UI
8. **29-Abr (tarde)** — Auditoría de performance: Lazy regex, LTO fat, limpiezas
9. **30-Abr** — Categorías editables (config externalizado + reclasificación masiva + editor visual)
10. **3-May (mañana)** — Robustez: `atomic_io.rs` resuelve corrupción por race con Syncthing
11. **3-May (día)** — Descubridor automático de categorías por GitHub topics (Fase 4)
12. **3-May / 4-May** — Refinamientos UX: compactar, borrar vacías, simular, solo verdes, header skyline, log con menú contextual
13. **4-May (mañana)** — Refresh inteligente con ETag/If-None-Match: 5-10× más eficiente
14. **4-May (tarde)** — Refactor modular de gui/app.rs en 5 submódulos (-24% líneas)
15. **4-May (final)** — Optimizaciones (3 regex Lazy + dead code) → **RELEASE v1.0.0**

---

## Decisiones arquitectónicas clave

1. **HTML autocontenido** — el `buscador.html` embebe el JSON de datos vía `<script type="application/json">` y el favicon como data URI. Funciona offline, se puede compartir por correo, no depende del filesystem.
2. **Cache incremental por mtime** — los READMEs solo se renderizan/traducen si cambió el original. Operaciones repetidas son casi instantáneas.
3. **Schema JSON estable Python ↔ Rust** — `repos_index.json` con 14 campos compatibles entre versiones, así migrar entre ellas no rompe el resto del flujo.
4. **DPAPI para secretos** — la API key nunca se almacena en texto plano ni en variables de entorno. El cifrado está atado al usuario + máquina; copiar `config.dat` a otra PC no permite descifrarlo.
5. **Worker threads vía channel** — la GUI nunca se congela durante operaciones largas. El `mpsc::channel` mantiene el flujo unidireccional worker → main thread.
6. **Categorización por heurística + fallback LLM** — la clasificación principal es por keywords con pesos (rápido, sin red); el LLM solo se usa para traducir descripciones y READMEs (caro pero opcional).
7. **`panic = "abort"` en release** — sin unwinding, binario más chico y más rápido. El precio es que cualquier panic mata el proceso, lo que disciplina al código a nunca paniquear.
8. **Config editable externamente (`data/categorias.json`)** — la taxonomía se puede modificar sin recompilar. La UI valida los cambios y dispara renames físicos cuando cambia un id de categoría.
9. **Escritura atómica universal** (`atomic_io.rs`) — todos los JSONs críticos se escriben con `write-to-tmp + fsync + rename`. Garantiza consistencia frente a Syncthing, antivirus, otros procesos o crashes a media escritura. Aplicado a 6 archivos: repo_ids, categorias, repos_index, descripciones_es, buscador.html, config.dat.
10. **Descubrimiento de categorías data-driven** — `topic_discovery.rs` propone candidatos a partir de los topics oficiales de GitHub, no de listas hardcoded. La taxonomía evoluciona con el universo del usuario, no la inversa.
11. **ETag conditional GET** — todos los refreshes mandan `If-None-Match` con el etag previo. Los repos sin cambios responden 304 Not Modified sin consumir cuota del rate-limit. El ahorro real es 5-10× a partir del segundo refresh.
12. **Validación + simulación antes de aplicar** — el descubridor permite "Simular" para ver cuántos repos REALMENTE caerían en cada candidato (ejecuta `compute_reclassification` en memoria). Permite descartar categorías que se quedarían vacías sin tocar disco.
13. **Modularidad explícita en GUI** — `gui/types.rs` (estado), `gui/helpers.rs` (puro), `gui/workers.rs` (thread::spawn), `gui/app.rs` (impl + render). Cada archivo es testeable en aislamiento.

---

*Generado a partir del log de tareas y la conversación de construcción del proyecto. Cierre v1.0.0 — 4 de mayo de 2026.*
