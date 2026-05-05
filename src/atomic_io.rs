//! Escritura atómica de archivos: write-to-tmp + fsync + rename.
//!
//! ## Motivación
//!
//! La función estándar `std::fs::write(path, data)` truncar+abre+escribe el
//! archivo final en su lugar. Si el proceso es interrumpido a la mitad
//! (kill, crash, agotamiento de disco) o si OTRO proceso accede al archivo
//! mientras estamos escribiendo (Syncthing sincronizando entre máquinas,
//! antivirus escaneando, otra instancia de la app), el archivo queda
//! corrupto/truncado.
//!
//! El patrón `write_atomic` resuelve esto:
//!   1. Escribir todo el contenido a `<path>.tmp`
//!   2. fsync el archivo temporal (garantía de que está en disco)
//!   3. `rename(tmp, real)` — operación atómica a nivel filesystem
//!
//! En Windows `fs::rename` usa `MoveFileExW` con `MOVEFILE_REPLACE_EXISTING`,
//! que es atómico siempre que origen+destino estén en el mismo volumen.
//! En Linux/macOS es atómico por POSIX.
//!
//! Si Syncthing intercepta a mitad de proceso, ve el `.tmp` (que no le
//! interesa si está en exclusiones) o el archivo final completo. Nunca un
//! estado intermedio.
//!
//! ## Recomendación adicional para usuarios con Syncthing
//!
//! Agregar a las exclusiones del folder sincronizado:
//!   `*.tmp`
//! Esto evita que los archivos temporales se sincronicen y se confundan
//! con los reales en la otra máquina.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Escribe `contents` a `path` de forma atómica:
///   1. Escribe a `<path>.tmp`
///   2. fsync para empujar al disco físico
///   3. Renombra `.tmp` → destino (atómico a nivel FS)
///
/// Retorna error si cualquiera de los pasos falla. Si el `rename` final
/// falla, el `.tmp` queda en disco como evidencia para debug.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    // Crear directorio padre si no existe.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creando directorio padre de {}", path.display()))?;
    }

    let tmp = tmp_path(path);

    // Escribir + fsync en un scope para que File se cierre antes del rename.
    {
        let mut f = File::create(&tmp)
            .with_context(|| format!("creando archivo temporal {}", tmp.display()))?;
        f.write_all(contents)
            .with_context(|| format!("escribiendo en {}", tmp.display()))?;
        // sync_all garantiza que los datos llegaron al disco físico antes
        // de continuar. Sin esto, un crash entre write_all y rename podría
        // dejar el .tmp con bytes en buffer del OS, no en el disco.
        f.sync_all()
            .with_context(|| format!("fsync de {}", tmp.display()))?;
    }

    // Rename atómico. En Windows MoveFileExW(MOVEFILE_REPLACE_EXISTING) hace
    // esto correctamente desde Rust 1.45.
    fs::rename(&tmp, path)
        .with_context(|| format!("renombrando {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Variante para `String` (la mayoría de nuestros JSON).
pub fn write_atomic_string(path: &Path, contents: &str) -> Result<()> {
    write_atomic(path, contents.as_bytes())
}

/// Path del archivo temporal asociado: `foo.json` → `foo.json.tmp`.
/// Mantiene el mismo directorio para que el rename sea intra-volume.
fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.to_path_buf();
    let mut name = path.file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("file"));
    name.push(".tmp");
    tmp.set_file_name(name);
    tmp
}
