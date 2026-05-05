//! Mover repos, eliminar carpetas con read-only (Win), acciones sobre duplicados.
//! Equivalente a `move_repo`, `_safe_rmtree`, `resolve_duplicate_action` Python.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;

// Regex compilada UNA SOLA VEZ. last_commit_date se llama por cada
// duplicado en gather_compare_info → con 200 duplicados eran 200
// compilaciones de regex. Con Lazy es una sola para toda la sesión.
static RE_GIT_LOG_TS: Lazy<Regex> = Lazy::new(||
    Regex::new(r"\s(\d{10})\s[\+\-]\d{4}\s").unwrap()
);

/// Mueve un repo a `<root>/<categoria>/<nombre>`. Falla si ya existe destino.
pub fn move_repo(src: &Path, root: &Path, categoria: &str) -> Result<PathBuf> {
    let dst_dir = root.join(categoria);
    fs::create_dir_all(&dst_dir)?;
    let name = src.file_name().context("repo sin nombre")?;
    let dst = dst_dir.join(name);
    if dst.exists() {
        return Err(anyhow!("ya existe destino: {}", dst.display()));
    }
    fs::rename(src, &dst)
        .with_context(|| format!("rename {} → {}", src.display(), dst.display()))?;
    Ok(dst)
}

/// Borra recursivo limpiando atributo read-only (típico en
/// `.git/objects/pack/*.idx` en Windows). Equivalente al `_safe_rmtree` Python.
///
/// Adicionalmente reintenta hasta 3 veces con back-off ante errores típicos
/// de Windows que son transitorios:
///   - `ERROR_SHARING_VIOLATION (32)` — antivirus escaneando, Explorer
///      mostrando preview, git daemon de fondo, Syncthing copiando.
///   - `ERROR_ACCESS_DENIED (5)` — flag read-only que perdió la carrera con
///      un escaneo o que aplicaba a directorio padre.
///   - `ERROR_DIR_NOT_EMPTY (145)` — race entre el listado del directorio
///      y la eliminación efectiva de su contenido.
///
/// Si tras 3 intentos sigue fallando, propaga el error con el contexto
/// completo (path + último error del SO).
pub fn safe_rmtree(path: &Path) -> Result<()> {
    /// Limpia TODOS los atributos NTFS que podrían bloquear el delete:
    /// READONLY, SYSTEM, HIDDEN, ARCHIVE. Lo hace usando Win32 directo
    /// (`SetFileAttributesW(path, FILE_ATTRIBUTE_NORMAL)`) porque la API
    /// estándar de Rust (`fs::Permissions::set_readonly`) solo toca el
    /// flag READONLY y deja los demás intactos — y los `.git/objects/pack/*.idx`
    /// y `.pack` muchas veces tienen SYSTEM, generando ERROR_ACCESS_DENIED
    /// (os error 5) al intentar borrar.
    ///
    /// En non-Windows usamos el comportamiento original (solo readonly)
    /// porque los demás atributos no aplican.
    #[cfg(windows)]
    fn unset_readonly(p: &Path) -> std::io::Result<()> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_NORMAL,
        };
        // SetFileAttributesW requiere wide-string null-terminated.
        let wide: Vec<u16> = p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: wide es null-terminated y vivo durante la llamada.
        let result = unsafe {
            SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_NORMAL)
        };
        if let Err(e) = result {
            // Si falla, no es fatal — algunos archivos no permiten cambiar
            // attribs (link a path remoto, NTFS junction roto, etc.).
            // Devolvemos el error para que el caller decida; pero el rmtree
            // ya hace `let _ = unset_readonly(p)` ignorándolo, así que
            // intentaremos remove_file de todos modos.
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("SetFileAttributesW falló: {:?}", e),
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    fn unset_readonly(p: &Path) -> std::io::Result<()> {
        let meta = fs::metadata(p)?;
        let mut perms = meta.permissions();
        if perms.readonly() {
            #[allow(deprecated)]
            perms.set_readonly(false);
            fs::set_permissions(p, perms)?;
        }
        Ok(())
    }
    fn impl_(p: &Path) -> std::io::Result<()> {
        if p.is_dir() {
            // procesar contenido
            let entries: Vec<_> = match fs::read_dir(p) {
                Ok(rd) => rd.flatten().collect(),
                Err(e) => return Err(e),
            };
            for entry in entries {
                let p2 = entry.path();
                if entry.file_type()?.is_dir() {
                    impl_(&p2)?;
                } else {
                    let _ = unset_readonly(&p2);
                    fs::remove_file(&p2)?;
                }
            }
            let _ = unset_readonly(p);
            fs::remove_dir(p)?;
        } else {
            let _ = unset_readonly(p);
            fs::remove_file(p)?;
        }
        Ok(())
    }

    // Retry loop: 3 intentos con back-off 100ms · 400ms · 1000ms.
    // Cubre los casos más comunes de "archivo en uso transitoriamente"
    // en Windows sin colgar al usuario más de 1.5s por repo problemático.
    const DELAYS_MS: &[u64] = &[100, 400, 1000];
    let mut last_err: Option<std::io::Error> = None;
    for (attempt, delay) in DELAYS_MS.iter().copied().enumerate() {
        match impl_(path) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt < DELAYS_MS.len() - 1 {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                }
            }
        }
    }

    // Fallback Windows: si fs::remove_file/dir falló 3 veces, intentar con
    // `cmd /C rmdir /S /Q`. El shell de Windows usa un código path de Win32
    // distinto al de Rust (a veces `RemoveDirectoryW` con flags diferentes,
    // y maneja DACLs heredadas de manera más permisiva). Si el problema era
    // permisos NTFS extendidos (no atributos read-only/system/hidden), este
    // fallback funciona donde el método nativo falla.
    //
    // Solo se ejecuta si el path sigue existiendo después del retry loop
    // (puede que el último intento haya borrado parcialmente).
    #[cfg(windows)]
    if path.exists() {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let path_str = path.to_string_lossy().to_string();
        let result = std::process::Command::new("cmd")
            .args(["/C", "rmdir", "/S", "/Q", &path_str])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match result {
            Ok(out) if out.status.success() && !path.exists() => {
                // cmd rmdir tuvo éxito.
                return Ok(());
            }
            Ok(out) => {
                // cmd corrió pero falló — adjuntamos su stderr al error.
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let combined = format!(
                    "fs::remove falló y cmd /C rmdir también falló: {}",
                    if stderr.is_empty() { "(sin output)".to_string() } else { stderr }
                );
                return Err(anyhow::anyhow!("{}", combined))
                    .with_context(|| format!("rmtree {}", path.display()));
            }
            Err(e) => {
                // ni siquiera pudimos invocar cmd — propagamos error original.
                let _ = e;
            }
        }
    }

    Err(last_err.expect("loop ejecuta al menos 1 intento"))
        .with_context(|| format!(
            "rmtree {} (3 intentos nativos + fallback cmd rmdir fallidos — \
             probablemente proceso reteniendo file handle, antivirus, o Syncthing)",
            path.display()
        ))
}

#[derive(Debug, Clone)]
pub enum DupAction {
    Archive,
    Skip,
    Delete,
    Replace,
    Rename(String),
}

impl DupAction {
    pub fn label(&self) -> &'static str {
        match self {
            DupAction::Archive    => "archive_new",
            DupAction::Skip       => "skip",
            DupAction::Delete     => "delete_new",
            DupAction::Replace    => "replace_old",
            DupAction::Rename(_)  => "rename_new",
        }
    }
}

/// Aplica una acción sobre un duplicado. Devuelve mensaje de resultado.
pub fn apply_dup_action(
    action: &DupAction,
    new_path: &Path,
    old_path: &Path,
) -> Result<String> {
    match action {
        DupAction::Archive => {
            let archive = new_path.parent()
                .context("sin parent")?
                .join("_duplicados");
            fs::create_dir_all(&archive)?;
            let name = new_path.file_name().context("sin nombre")?;
            let mut dst = archive.join(name);
            if dst.exists() {
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                dst = archive.join(format!("{}__{}", name.to_string_lossy(), stamp));
            }
            fs::rename(new_path, &dst)?;
            Ok(format!("Archivado en: {}", dst.display()))
        }
        DupAction::Skip => Ok("Saltado (sin cambios)".into()),
        DupAction::Delete => {
            safe_rmtree(new_path)?;
            Ok(format!("Borrado: {}", new_path.display()))
        }
        DupAction::Replace => {
            let parent = old_path.parent().context("viejo sin parent")?;
            safe_rmtree(old_path)?;
            let name = new_path.file_name().context("nuevo sin nombre")?;
            let dst = parent.join(name);
            fs::rename(new_path, &dst)?;
            Ok(format!("Reemplazado: {}", old_path.display()))
        }
        DupAction::Rename(new_name) => {
            if new_name.trim().is_empty() {
                return Err(anyhow!("nuevo nombre vacío"));
            }
            let parent = new_path.parent().context("nuevo sin parent")?;
            let dst = parent.join(new_name);
            if dst.exists() {
                return Err(anyhow!("ya existe: {}", dst.display()));
            }
            fs::rename(new_path, &dst)?;
            Ok(format!("Renombrado a: {}", dst.display()))
        }
    }
}

/// Información comparativa para mostrar lado a lado en el diálogo.
#[derive(Debug, Clone)]
pub struct CompareInfo {
    pub name: String,
    pub path: PathBuf,
    pub url: String,
    pub last_commit: String,
    pub size_mb: f64,
}

pub fn gather_compare_info(repo: &Path) -> CompareInfo {
    let url = crate::duplicates::get_git_remote_url(repo)
        .unwrap_or_else(|| "(sin remote)".to_string());
    let last_commit = last_commit_date(repo);
    let size_mb = quick_size_mb(repo);
    CompareInfo {
        name: repo.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        path: repo.to_path_buf(),
        url,
        last_commit,
        size_mb,
    }
}

fn last_commit_date(repo: &Path) -> String {
    let head = repo.join(".git").join("logs").join("HEAD");
    if let Ok(text) = fs::read_to_string(&head) {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        if let Some(last) = lines.last() {
            // Regex pre-compilada en RE_GIT_LOG_TS (top del archivo).
            if let Some(cap) = RE_GIT_LOG_TS.captures(last) {
                if let Some(ts_match) = cap.get(1) {
                    if let Ok(ts) = ts_match.as_str().parse::<i64>() {
                        if let Some(d) = chrono::DateTime::from_timestamp(ts, 0) {
                            return d.date_naive().to_string();
                        }
                    }
                }
            }
        }
    }
    "—".into()
}

fn quick_size_mb(repo: &Path) -> f64 {
    let mut total: u64 = 0;
    let mut count = 0usize;
    let walker = walkdir::WalkDir::new(repo)
        .max_depth(2)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !crate::categories::EXCLUDE_DIRS.iter().any(|d| *d == n)
        });
    for entry in walker.flatten() {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total = total.saturating_add(meta.len());
                count += 1;
                if count > 500 { break; }
            }
        }
    }
    (total as f64) / 1_048_576.0
}
