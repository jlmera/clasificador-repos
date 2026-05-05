# Clasificador de Repositorios v1.0.0

**Primera release pública** · 4 de mayo de 2026 · Native Windows app · Rust + eframe/egui

---

## ¿Qué es esto?

Una aplicación de escritorio para Windows que **mantiene organizada una colección creciente de repositorios clonados desde GitHub**. Clasifica automáticamente cada repo en una taxonomía editable de categorías, detecta duplicados, sincroniza metadata oficial vía API, y genera un buscador HTML autocontenido para navegar la colección.

Probada en producción contra una colección de **157 repositorios** clasificados en **23 categorías**, con `repos_index.json` consumido por el buscador y vault Obsidian generado a partir del mismo índice.

## Highlights

- **Clasificación automática** con scoring híbrido `keywords + topic_boosts`, configurable vía `data/categorias.json` editable en caliente desde el editor visual integrado.
- **Detección de duplicados** por nombre y por URL del git remote (extraída de `.git/config`), con diálogo modal para resolver: archivar, borrar, renombrar o reemplazar.
- **Descubridor de categorías nuevas** que escanea topics oficiales de GitHub (REST API), filtra ruido (lenguajes, stacks, genéricos), y propone candidatos con preview de cuántos repos caerían realmente en cada uno.
- **Refresh inteligente con ETag** — usa `If-None-Match` para conditional GET. Los repos sin cambios responden 304 sin consumir cuota del rate-limit (5000 req/h con PAT).
- **Buscador HTML autocontenido** (`buscador.html`) con paginación, filtros por categoría/lenguaje/stack y búsqueda full-text — todo en un solo archivo sin dependencias externas.
- **Wiki Obsidian** generada con frontmatter YAML, wikilinks bidireccionales y página por categoría.
- **Traducción de READMEs al español** usando Anthropic Claude API (`claude-sonnet-4-6`), con caché incremental para no re-traducir lo ya hecho.
- **Cifrado de credenciales** con Windows DPAPI (`CryptProtectData`) — la API key y el PAT se atan al usuario+máquina.
- **Escritura atómica** universal en JSONs críticos (`write-to-tmp + fsync + rename`) para resistir race conditions con Syncthing/OneDrive.

## Stack

- **Lenguaje:** Rust 1.95 (toolchain `stable-x86_64-pc-windows-msvc`)
- **GUI:** eframe / egui 0.29 (immediate-mode, sin retención de estado entre frames)
- **HTTP:** ureq 2.x con `If-None-Match` para ETag
- **Serialización:** serde + serde_json
- **Win32:** windows-rs 0.58 (DPAPI cifrado, SetFileAttributesW para borrar `.git`, MoveFileExW atómico)
- **Markdown:** pulldown-cmark 0.13
- **APIs externas:** GitHub REST API, Anthropic Claude API

## Instalación

### Opción A — Binario precompilado (recomendado)

1. Descargá `clasificador.exe` del asset adjunto a esta release.
2. Colocálo en cualquier carpeta (no requiere instalador).
3. Ejecutalo. La primera vez te pedirá configurar la carpeta raíz donde tenés tus repos.

### Opción B — Compilar desde código

```cmd
git clone https://github.com/jlmera/clasificador-repos.git
cd clasificador-repos\fuente
_build_definitivo.bat
```

Requiere Rust 1.95+ y Visual Studio 2022 Build Tools.

## Configuración recomendada

Para usar todas las features:

- **GitHub PAT** (Personal Access Token) — sube el rate-limit de la API de 60 a 5000 req/h. Generalo en `https://github.com/settings/tokens` con scope `public_repo` (read-only).
- **Anthropic API key** — habilita la feature de traducción de READMEs al español (opcional).

Ambos se configuran desde el botón **⚙ API Key** en la app y se almacenan cifrados con DPAPI en `%APPDATA%\jlmera\clasificador\config.dat`.

## Documentación

- [README](https://github.com/jlmera/clasificador-repos/blob/main/README.md) — visión general y quickstart
- [HISTORIA_PROYECTO.txt](https://github.com/jlmera/clasificador-repos/blob/main/docs/HISTORIA_PROYECTO.txt) — narrativa completa del desarrollo, fase por fase
- [LINEA_TIEMPO_CLASIFICADOR.md](https://github.com/jlmera/clasificador-repos/blob/main/docs/LINEA_TIEMPO_CLASIFICADOR.md) — hitos cronológicos y métricas
- [roadmap_clasificador.pdf](https://github.com/jlmera/clasificador-repos/blob/main/docs/roadmap_clasificador.pdf) — 25 features priorizadas para v1.x y v2.0

## Compatibilidad

- ✅ Windows 10 (1809+)
- ✅ Windows 11
- ❌ Linux / macOS — usa exclusivamente APIs Win32 (DPAPI, SetFileAttributesW, MoveFileExW)

## Licencia

[MIT](https://github.com/jlmera/clasificador-repos/blob/main/LICENSE) — uso libre incluso comercial, con atribución requerida.

---

**Autor:** Jorge L. Mera ([@jlmera](https://github.com/jlmera)) — DUGOTEX
**Asistencia AI:** Claude (Anthropic)
