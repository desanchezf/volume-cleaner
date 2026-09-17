use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use sha2::{Sha256, Digest};
use walkdir::WalkDir;

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub size: u64,
    pub hash: String,
    pub extension: String,
    pub is_duped: bool,
    pub marked_for_deletion: bool,
}

pub fn scan_directory(
    start_dir: &str,
    extensions: &[String],
    mut on_progress: impl FnMut(f32, &str),
) -> Vec<Entry> {
    let mut files = Vec::<Entry>::new();

    for entry in WalkDir::new(start_dir) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue, // sin permiso / enlace roto: saltar
        };
        // Sacamos el path y comprobamos si es un archivo
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Comprobamos si la extensión es permitida
        let allowed = match path.extension().and_then(|e| e.to_str()) {
            // Sin extensión: solo si el usuario pidió el token especial "none"
            None => extensions
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case("none")),
            Some(ext) => extensions
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(ext)),
        };

        if allowed {
            let size = match entry.metadata() {
                Ok(meta) => meta.len(),
                Err(_) => continue,
            };

            files.push(Entry {
                path: path.to_path_buf(),
                size,
                hash: String::new(),
                extension: path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string(),
                is_duped: false,
                marked_for_deletion: false,
            });

            if files.len() % 25 == 0 {
                let found = files.len();
                let fraction = 0.45 * (found as f32) / (found as f32 + 400.0);
                on_progress(fraction, &format!("Found {found} files"));
            }
        }
    }

    on_progress(0.5, &format!("Found {} files, checking duplicates…", files.len()));
    files
}

pub fn check_files(
    files_vector: &[Entry],
    mut on_progress: impl FnMut(f32, &str),
) -> Vec<Entry> {
    let mut files = files_vector.to_vec();

    let mut grouped_by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, file) in files.iter().enumerate() {
        grouped_by_size.entry(file.size).or_default().push(i);
    }

    // Dentro de cada grupo de mismo tamaño, descarta primero comparando solo
    // los primeros bytes: si ya difieren ahí, nos ahorramos leer el archivo
    // entero para el hash completo.
    let mut to_hash: Vec<usize> = Vec::new();
    for indices in grouped_by_size.values() {
        if indices.len() < 2 {
            continue;
        }

        let mut grouped_by_partial: HashMap<String, Vec<usize>> = HashMap::new();
        for &i in indices {
            let partial = calculate_partial_hash(&files[i].path);
            grouped_by_partial.entry(partial).or_default().push(i);
        }

        for partial_indices in grouped_by_partial.values() {
            if partial_indices.len() >= 2 {
                to_hash.extend(partial_indices);
            }
        }
    }

    let mut grouped_by_hash: HashMap<String, Vec<usize>> = HashMap::new();
    let total = to_hash.len().max(1);
    for (done, &i) in to_hash.iter().enumerate() {
        let hash = calculate_hash(&files[i].path);
        files[i].hash = hash.clone();
        grouped_by_hash.entry(hash).or_default().push(i);
        if done % 4 == 0 || done + 1 == to_hash.len() {
            let fraction = 0.5 + 0.5 * ((done + 1) as f32) / (total as f32);
            on_progress(fraction, &format!("Hashing {}/{total}", done + 1));
        }
    }

    for indices in grouped_by_hash.values() {
        if indices.len() >= 2 {
            for &i in indices {
                files[i].is_duped = true;
            }
        }
    }

    files
}

fn calculate_hash(file_path: &Path) -> String {
    let mut file = match File::open(file_path) {
        Ok(file) => file,
        Err(_) => return String::new(),
    };
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(_) => return String::new(),
        }
    }
    hex_encode(&hasher.finalize())
}

// Cuántos bytes iniciales se leen para el descarte rápido antes del hash completo.
const PARTIAL_HASH_BYTES: usize = 4096;

fn calculate_partial_hash(file_path: &Path) -> String {
    let file = match File::open(file_path) {
        Ok(file) => file,
        Err(_) => return String::new(),
    };
    let mut buffer = Vec::with_capacity(PARTIAL_HASH_BYTES);
    if file.take(PARTIAL_HASH_BYTES as u64).read_to_end(&mut buffer).is_err() {
        return String::new();
    }
    hex_encode(&Sha256::digest(&buffer))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push_str(&format!("{:02x}", byte));
    }
    hex
}


// Funciones auxiliares
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub(crate) fn display_path(path: &Path) -> String {
    if let Some(home) = home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            let rest = rest.display().to_string().replace('\\', "/");
            return format!("~/{rest}");
        }
    }
    path.display().to_string()
}
