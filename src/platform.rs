use std::path::Path;

/// Abre el gestor de archivos del sistema mostrando (o, cuando el SO lo
/// permite, seleccionando) el archivo indicado.
pub fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg("/select,")
            .arg(path)
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }

    #[cfg(target_os = "linux")]
    {
        // xdg-open no soporta "seleccionar archivo", solo abrir una carpeta.
        let target = path.parent().unwrap_or(path);
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

/// Convierte una imagen HEIC/HEIF a JPEG para poder previsualizarla, ya que
/// el decodificador de imágenes de egui no soporta HEIC. Solo en macOS, con
/// `sips` (incluido en el sistema). El resultado se guarda en la caché de la
/// aplicación y se reutiliza. Devuelve `None` en otros sistemas o si falla.
pub fn heic_preview(path: &Path) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use sha2::{Digest, Sha256};
        let cache = dirs::cache_dir()?.join("volume-cleaner").join("heic");
        std::fs::create_dir_all(&cache).ok()?;
        let key: String = Sha256::digest(path.to_string_lossy().as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let out = cache.join(format!("{key}.jpg"));
        if out.exists() {
            return Some(out);
        }
        let status = std::process::Command::new("sips")
            .args(["-s", "format", "jpeg", "-Z", "2048"])
            .arg(path)
            .arg("--out")
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        return if status.success() && out.exists() { Some(out) } else { None };
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}
