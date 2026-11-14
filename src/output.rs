// Modo "directorio de salida": los archivos que se conservan se copian a
// `<salida>/<categoría>/<ruta relativa>` y solo después de verificar la copia
// se manda el original a la papelera. Los marcados para borrar van a la
// papelera directamente. Si el proceso se interrumpe a mitad de un archivo,
// el original sigue intacto en el origen (como mucho queda un `.part`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::FileCategory;
use crate::filesystem;

/// Carpeta de primer nivel dentro de la salida para cada categoría.
pub fn category_dir(category: FileCategory) -> &'static str {
    match category {
        FileCategory::Image => "image",
        FileCategory::Video => "video",
        FileCategory::Audio => "audio",
        FileCategory::Documents => "documents",
        FileCategory::Custom => "other",
    }
}

/// `<output>/<categoría>/<ruta relativa a input_root>`.
pub fn destination_for(
    output: &Path,
    input_root: &Path,
    path: &Path,
    category: FileCategory,
) -> Option<PathBuf> {
    let relative = path.strip_prefix(input_root).ok()?;
    Some(output.join(category_dir(category)).join(relative))
}

/// Espacio libre en el volumen de `path`, si se puede averiguar.
pub fn available_space(path: &Path) -> Option<u64> {
    fs4::available_space(path).ok()
}

/// Comprueba que la carpeta de salida es válida para la carpeta de entrada:
/// no puede ser la misma ni estar dentro de ella (se volvería a escanear y
/// las rutas relativas se anidarían).
pub fn validate_output(input: &Path, output: &Path) -> Result<(), String> {
    let input = input.canonicalize().map_err(|e| format!("Input folder: {e}"))?;
    let output = match output.canonicalize() {
        Ok(path) => path,
        Err(_) => return Err("The output folder does not exist".to_string()),
    };
    if output.starts_with(&input) {
        return Err("The output folder cannot be the input folder or inside it".to_string());
    }
    Ok(())
}

pub enum JobKind {
    /// Copiar verificado a `dest` y después mandar el original a la papelera.
    Move { dest: PathBuf },
    /// Mandar a la papelera. Si hay `expected_hash` (duplicados), se vuelve
    /// a calcular justo antes y solo se borra si el contenido no ha cambiado
    /// desde el escaneo.
    Trash { expected_hash: Option<String> },
}

pub struct Job {
    pub source: PathBuf,
    pub kind: JobKind,
}

pub enum Outcome {
    /// Copiado y verificado en esta ruta; el original está en la papelera.
    Moved(PathBuf),
    Trashed,
    /// No se ha tocado a propósito (p. ej. cambió desde el escaneo).
    Skipped(String),
    Failed(String),
}

/// Manda un archivo a la papelera, comprobando antes su hash si se indica.
pub fn trash_checked(path: &Path, expected_hash: Option<&str>) -> Outcome {
    if let Some(expected) = expected_hash.filter(|h| !h.is_empty()) {
        if filesystem::calculate_hash(path) != expected {
            return Outcome::Skipped("changed since the scan".to_string());
        }
    }
    match trash::delete(path) {
        Ok(()) => Outcome::Trashed,
        Err(e) => Outcome::Failed(format!("trash: {e}")),
    }
}

/// Ejecuta los trabajos en orden. Se detiene antes del siguiente archivo si
/// `cancel` se activa. Devuelve el resultado de cada trabajo procesado.
pub fn run_batch(
    jobs: Vec<Job>,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f32, &str),
) -> Vec<(PathBuf, Outcome)> {
    let total = jobs.len().max(1);
    let mut results = Vec::with_capacity(jobs.len());
    for (done, job) in jobs.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let name = job
            .source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        on_progress(done as f32 / total as f32, &format!("{}/{total} {name}", done + 1));

        let outcome = match job.kind {
            JobKind::Trash { expected_hash } => trash_checked(&job.source, expected_hash.as_deref()),
            JobKind::Move { dest } => match copy_verified(&job.source, &dest) {
                Ok(final_dest) => match trash::delete(&job.source) {
                    Ok(()) => Outcome::Moved(final_dest),
                    Err(e) => Outcome::Failed(format!("copied, but trashing original failed: {e}")),
                },
                Err(e) => Outcome::Failed(e),
            },
        };
        results.push((job.source, outcome));
    }
    on_progress(1.0, "Done");
    results
}

/// Copia `source` a `dest` de forma segura:
/// 1. copia a `<dest>.part`,
/// 2. compara el SHA-256 de la copia con el del original,
/// 3. renombra la copia a su nombre final.
///
/// Nunca sobrescribe: si `dest` ya existe con el mismo contenido (p. ej. de
/// una ejecución anterior interrumpida) se reutiliza; si tiene otro contenido
/// se usa `nombre (1).ext`, `nombre (2).ext`, … Devuelve la ruta final.
fn copy_verified(source: &Path, dest: &Path) -> Result<PathBuf, String> {
    let source_hash = filesystem::calculate_hash(source);
    if source_hash.is_empty() {
        return Err("could not read source".to_string());
    }

    let mut target = dest.to_path_buf();
    let mut n = 1;
    while target.exists() {
        if filesystem::calculate_hash(&target) == source_hash {
            return Ok(target);
        }
        target = numbered(dest, n);
        n += 1;
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create folder: {e}"))?;
    }
    let mut part = target.clone().into_os_string();
    part.push(".part");
    let part = PathBuf::from(part);

    std::fs::copy(source, &part).map_err(|e| format!("copy: {e}"))?;
    if let Ok(file) = std::fs::File::open(&part) {
        let _ = file.sync_all();
    }
    if filesystem::calculate_hash(&part) != source_hash {
        let _ = std::fs::remove_file(&part);
        return Err("copy verification failed (hash mismatch)".to_string());
    }
    std::fs::rename(&part, &target).map_err(|e| format!("rename: {e}"))?;
    Ok(target)
}

/// `foto.jpg` → `foto (n).jpg`; `archivo` → `archivo (n)`.
fn numbered(path: &Path, n: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vc-output-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn destination_keeps_relative_path_under_category() {
        let dest = destination_for(
            Path::new("/out"),
            Path::new("/in"),
            Path::new("/in/2019/Trip/IMG.jpg"),
            FileCategory::Image,
        );
        assert_eq!(dest, Some(PathBuf::from("/out/image/2019/Trip/IMG.jpg")));
    }

    #[test]
    fn numbered_names() {
        assert_eq!(numbered(Path::new("/o/a.jpg"), 1), PathBuf::from("/o/a (1).jpg"));
        assert_eq!(numbered(Path::new("/o/a"), 2), PathBuf::from("/o/a (2)"));
    }

    #[test]
    fn copy_verified_never_overwrites_and_reuses_identical() {
        let dir = scratch("copy");
        let src = dir.join("src.txt");
        std::fs::write(&src, b"hello").unwrap();

        // Destino nuevo: copia y crea carpetas.
        let dest = dir.join("out/image/a/src.txt");
        assert_eq!(copy_verified(&src, &dest).unwrap(), dest);
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        assert!(!dir.join("out/image/a/src.txt.part").exists());

        // Mismo contenido ya presente: se reutiliza.
        assert_eq!(copy_verified(&src, &dest).unwrap(), dest);

        // Contenido distinto en el destino: no se sobrescribe.
        std::fs::write(&dest, b"other").unwrap();
        let renamed = copy_verified(&src, &dest).unwrap();
        assert_eq!(renamed, dir.join("out/image/a/src (1).txt"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"other");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_inside_input_is_rejected() {
        let dir = scratch("validate");
        std::fs::create_dir_all(dir.join("in/out")).unwrap();
        std::fs::create_dir_all(dir.join("out")).unwrap();
        assert!(validate_output(&dir.join("in"), &dir.join("in")).is_err());
        assert!(validate_output(&dir.join("in"), &dir.join("in/out")).is_err());
        assert!(validate_output(&dir.join("in"), &dir.join("out")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
