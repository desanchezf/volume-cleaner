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
