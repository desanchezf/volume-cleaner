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
    /// Whether the user has already made a keep/delete decision for this file.
    /// Persisted so a review session can resume where it left off.
    pub reviewed: bool,
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
                reviewed: false,
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

// --- Persistencia del estado de revisión ---
//
// Guarda, por carpeta escaneada, qué archivos ya se han revisado y su
// decisión (keep/delete), junto con el índice de revisión actual, para que
// cerrar y reabrir la aplicación permita retomar la revisión donde se dejó.

fn app_state_dir() -> Option<PathBuf> {
    Some(PathBuf::from("session"))
}

fn review_state_file_for(folder: &Path) -> Option<PathBuf> {
    let dir = app_state_dir()?.join("state");
    let key = hex_encode(&Sha256::digest(folder.to_string_lossy().as_bytes()));
    Some(dir.join(format!("{key}.tsv")))
}

pub struct ReviewState {
    pub decisions: HashMap<PathBuf, (bool, bool)>, // (marked_for_deletion, reviewed)
    pub review_index: usize,
}

pub fn save_review_state(folder: &Path, files: &[Entry], review_index: usize) -> std::io::Result<()> {
    let Some(path) = review_state_file_for(folder) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut content = format!("folder\t{}\n", folder.display());
    content.push_str(&format!("review_index\t{review_index}\n"));
    for file in files {
        content.push_str(&format!(
            "{}\t{}\t{}\n",
            file.marked_for_deletion as u8,
            file.reviewed as u8,
            file.path.display(),
        ));
    }
    std::fs::write(path, content)
}

pub fn load_review_state(folder: &Path) -> Option<ReviewState> {
    let path = review_state_file_for(folder)?;
    let content = std::fs::read_to_string(path).ok()?;

    let mut decisions = HashMap::new();
    let mut review_index = 0usize;
    for line in content.lines() {
        let mut parts = line.splitn(3, '\t');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("review_index"), Some(value), None) => {
                review_index = value.parse().unwrap_or(0);
            }
            (Some("folder"), Some(_), None) => {}
            (Some(marked), Some(reviewed), Some(path_str)) => {
                decisions.insert(PathBuf::from(path_str), (marked == "1", reviewed == "1"));
            }
            _ => {}
        }
    }

    Some(ReviewState { decisions, review_index })
}

pub fn apply_review_state(files: &mut [Entry], state: &ReviewState) {
    for file in files.iter_mut() {
        if let Some(&(marked, reviewed)) = state.decisions.get(&file.path) {
            file.marked_for_deletion = marked;
            file.reviewed = reviewed;
        }
    }
}

// --- Persistencia de la última sesión (carpeta y filtro) ---
//
// Permite que, al reabrir la aplicación, se recuerde qué carpeta y filtro se
// usaron por última vez, para no tener que volver a navegar hasta ella antes
// de reanudar la revisión.

pub struct LastSession {
    pub folder: PathBuf,
    pub filter_label: String,
    pub custom_extensions: String,
}

fn last_session_file() -> Option<PathBuf> {
    app_state_dir().map(|dir| dir.join("last_session.tsv"))
}

pub fn save_last_session(folder: &Path, filter_label: &str, custom_extensions: &str) {
    let Some(path) = last_session_file() else {
        return;
    };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let content = format!(
        "folder\t{}\nfilter\t{filter_label}\ncustom\t{custom_extensions}\n",
        folder.display(),
    );
    let _ = std::fs::write(path, content);
}

pub fn load_last_session() -> Option<LastSession> {
    let path = last_session_file()?;
    let content = std::fs::read_to_string(path).ok()?;

    let mut folder = None;
    let mut filter_label = String::new();
    let mut custom_extensions = String::new();
    for line in content.lines() {
        if let Some((key, value)) = line.split_once('\t') {
            match key {
                "folder" => folder = Some(PathBuf::from(value)),
                "filter" => filter_label = value.to_string(),
                "custom" => custom_extensions = value.to_string(),
                _ => {}
            }
        }
    }

    Some(LastSession {
        folder: folder?,
        filter_label,
        custom_extensions,
    })
}
