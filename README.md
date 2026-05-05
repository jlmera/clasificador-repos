# Clasificador de Repositorios

Aplicación nativa Windows para mantener organizada una colección creciente de repositorios clonados desde GitHub. Clasifica automáticamente cada repo en categorías curadas usando una mezcla de heurística sobre nombre/descripción/manifests + topics oficiales de la API de GitHub. Genera un buscador HTML autocontenido para navegar la colección, traduce READMEs al español con Claude, y exporta una wiki Obsidian.

**Versión actual:** 1.0.0 (release 4-may-2026) · **Stack:** Rust + eframe/egui + windows-rs

---

## Capturas

```
┌─ CLASIFICADOR DE REPOSITORIOS ─────────  ▒▒▓▓▓▓▓▓▓▓▓▓▓▓ ─┐
│ Status: Listo. Selecciona una acción.                     │
├───────────────────────────────────────────────────────────┤
│ [🔍 Escanear] [✅ Aplicar]                                │
│ [🧹 Limpiar log] [🔄 Solo reindexar] [🔁 Reclasificar]    │
│ [🔍 Descubrir] [🏷 Categorías] [⚙ API Key]                │
│ [🆔 Refrescar GitHub IDs] [🇨🇴 Traducir READMEs]          │
│ [📚 Wiki Obsidian] [🌐 Abrir buscador]                    │
├───────────────────────────────────────────────────────────┤
│ === Acción: scan ===                                      │
│   • langgraph  [Python, stack=python]                     │
│       → 03-agentes-y-llms  conf=0.86                      │
│   ...                                                     │
└───────────────────────────────────────────────────────────┘
```

---

## Features principales

- **Clasificación automática** con scoring por keywords + topic_boosts (config externalizado en `data/categorias.json`).
- **Detección de duplicados** por nombre y por URL del git remote, con diálogo modal para resolver: archivar, borrar, renombrar o reemplazar.
- **Descubridor de categorías nuevas** que escanea topics oficiales de GitHub (campo `topics` de la REST API), filtra lenguajes/stacks/genéricos y propone candidatos con preview de cuántos repos caerían realmente en cada uno.
- **Editor visual de categorías** para añadir, eliminar, renombrar (renombra carpetas físicas), reordenar y editar keywords/topic_boosts con DragValue.
- **Refresh inteligente con ETag** — usa `If-None-Match` para conditional GET. Los repos sin cambios responden 304 sin consumir cuota del rate-limit.
- **Buscador HTML autocontenido** (`buscador.html`) con paginación, filtros por categoría/lenguaje/stack y búsqueda full-text.
- **Wiki Obsidian** generada con frontmatter YAML, wikilinks y página por categoría.
- **Traducción de READMEs** al español usando Anthropic Claude API (con caché incremental).
- **Cifrado de credenciales** con Windows DPAPI (`CryptProtectData`) — la API key y el PAT se atan al usuario+máquina.
- **Escritura atómica** universal en JSONs críticos (`write-to-tmp + fsync + rename`) para resistir race conditions con Syncthing.

---

## Instalación

### Pre-requisitos

- Windows 10 / 11
- Rust 1.95+ con toolchain `stable-x86_64-pc-windows-msvc` (instalar desde [rustup.rs](https://rustup.rs/))
- Visual Studio 2022 Build Tools (para `cl.exe` + Windows SDK)
- (Opcional) Personal Access Token de GitHub para evitar el rate-limit anonymous (60 req/h)
- (Opcional) Anthropic API key para usar las features de traducción

### Compilar desde fuente

```cmd
git clone https://github.com/jlmera/clasificador-repos.git
cd clasificador-repos
_build_definitivo.bat
```

El binario `clasificador.exe` queda en la carpeta padre. Tarda ~50 segundos la primera vez (compila ~200 dependencias).

Si cargo deja un fingerprint inconsistente y el binario no se regenera, usá:

```cmd
_build_limpio.bat
```

Esto hace `cargo clean` + build desde cero (~2-3 minutos).

---

## Configuración inicial

1. **Lanzá `clasificador.exe`.** En la primera ejecución pide configurar la carpeta raíz (donde están tus repos clonados).
2. **Click en `⚙ API Key`** y pegá:
   - **Anthropic API key** (sk-ant-...) si querés traducciones — opcional.
   - **GitHub PAT** (ghp_... o github_pat_...) para subir el rate-limit a 5000 req/h — recomendado.
3. **Click en `🆔 Refrescar GitHub IDs`** para poblar el cache con metadata oficial (id + descripción + topics) de cada repo. La primera vez tarda ~70s para 150 repos; las siguientes ~25s gracias al ETag.
4. **Click en `🌐 Abrir buscador`** para ver el resultado en el navegador.

---

## Arquitectura

### Estructura de archivos

```
fuente/
├── Cargo.toml          # Definición del crate + 2 binarios
├── src/
│   ├── main.rs         # Bin 1: GUI eframe
│   ├── bin/cli.rs      # Bin 2: CLI sin GUI
│   ├── lib.rs          # Re-exports de los módulos
│   ├── scan.rs         # Análisis de repos individuales
│   ├── classify.rs     # Heurística de scoring
│   ├── ids.rs          # Cache GitHub con ETag flow
│   ├── index.rs        # Reindex completo + repos_index.json
│   ├── html.rs         # Generador de buscador.html
│   ├── readme.rs       # Render de README a HTML
│   ├── reclassify.rs   # Análisis y aplicación de reclassify
│   ├── topic_discovery.rs  # Descubridor de categorías
│   ├── categories.rs   # Defaults hardcoded (fallback)
│   ├── categories_config.rs  # JSON externalizado editable
│   ├── apply_actions.rs    # Ops sobre duplicados + safe_rmtree Win32
│   ├── duplicates.rs   # Detección por nombre y por URL git
│   ├── moves.rs        # find_inbox_repos + helpers
│   ├── paths.rs        # Paths convencionales
│   ├── secrets.rs      # DPAPI cifrado
│   ├── llm.rs          # Anthropic Claude API
│   ├── wiki.rs         # Generador Obsidian vault
│   ├── atomic_io.rs    # Write-atomic con fsync + rename
│   └── gui/
│       ├── mod.rs
│       ├── theme.rs    # Paleta sage
│       ├── types.rs    # DialogStates + WorkerMsg + CatRow
│       ├── helpers.rs  # Funciones puras
│       ├── workers.rs  # Los 4 apply_X_worker en thread::spawn
│       └── app.rs      # ClasificadorApp + update()
├── templates/
│   └── buscador.html   # Template del entregable
├── docs/
│   ├── HISTORIA_PROYECTO.txt          # Historia narrativa completa
│   ├── LINEA_TIEMPO_CLASIFICADOR.md   # Hitos cronológicos + métricas
│   └── roadmap_clasificador.pdf       # 25 features futuras (Oleadas 1-6)
├── _build_definitivo.bat
└── _build_limpio.bat
```

### Datos persistidos en disco (todos atomic-write)

| Archivo | Propósito |
|---|---|
| `<root>/data/categorias.json` | Taxonomía editable (id + keywords + topic_boosts) |
| `<root>/data/repo_ids.json` | Cache de la API GitHub (id, topics, description, etag) |
| `<root>/data/repos_index.json` | Índice consumido por `buscador.html` |
| `<root>/data/descripciones_es.json` | Traducciones LLM cacheadas |
| `<root>/buscador.html` | Buscador web autocontenido |
| `%APPDATA%/jlmera/clasificador/config.dat` | API keys cifradas con DPAPI |

---

## Documentación

- **[`docs/HISTORIA_PROYECTO.txt`](docs/HISTORIA_PROYECTO.txt)** — relato completo del desarrollo, fase por fase, con decisiones y trade-offs.
- **[`docs/LINEA_TIEMPO_CLASIFICADOR.md`](docs/LINEA_TIEMPO_CLASIFICADOR.md)** — hitos cronológicos, métricas finales y decisiones arquitectónicas.
- **[`docs/roadmap_clasificador.pdf`](docs/roadmap_clasificador.pdf)** — 25 features priorizadas en 6 oleadas para versiones futuras.

---

## Stack tecnológico

- **Lenguaje:** Rust 1.95 (toolchain `stable-x86_64-pc-windows-msvc`)
- **GUI:** eframe / egui 0.29
- **HTTP:** ureq 2.x con `If-None-Match` para ETag
- **Serialización:** serde + serde_json
- **Filesystem:** walkdir, std::fs::rename atómico
- **Markdown:** pulldown-cmark 0.13 (render README a HTML)
- **Win32:** windows-rs 0.58 (DPAPI cifrado, SetFileAttributesW para borrar `.git`, MoveFileExW atómico)
- **Regex:** regex 1.x con `Lazy<Regex>` para hot paths
- **APIs externas:** GitHub REST API, Anthropic Claude API (`claude-sonnet-4-6`)

---

## Licencia

Este proyecto se distribuye bajo la licencia [MIT](LICENSE) — uso libre incluso comercial, con atribución requerida.

---

## Autor

**Jorge L. Mera** (jlmera) — DUGOTEX
Asistencia AI: Claude (Anthropic)
