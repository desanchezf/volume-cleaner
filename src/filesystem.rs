use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use sha2::{Sha256, Digest};
use walkdir::WalkDir;

#[derive(Clone)]
pub struct Entry {
    path: PathBuf,
    size: u64,
    hash: String,
    extension: String,
    is_duped: bool,
    marked_for_deletion: bool,
}

pub fn scan_directory(start_dir: &str, extensions: &Vec<String>) -> Vec<Entry> {
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
        }
    }

    files
}

pub fn check_files(files_vector: &[Entry]) -> Vec<Entry> {
    let mut files = files_vector.to_vec();

    let mut grouped_by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, file) in files.iter().enumerate() {
        grouped_by_size.entry(file.size).or_default().push(i);
    }

    let mut grouped_by_hash: HashMap<String, Vec<usize>> = HashMap::new();
    for indices in grouped_by_size.values() {
        if indices.len() < 2 {
            continue;
        }
        for &i in indices {
            let hash = calculate_hash(&files[i].path);
            files[i].hash = hash.clone();
            grouped_by_hash.entry(hash).or_default().push(i);
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
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
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

fn display_path(path: &Path) -> String {
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

pub fn print_files(files: &[Entry], print_marked_as_deleted: bool, print_marked_as_duped: bool) {
    let visible: Vec<&Entry> = files
        .iter()
        .filter(|file| {
            if !print_marked_as_deleted && !print_marked_as_duped {
                return true;
            }
            (print_marked_as_duped && file.is_duped)
                || (print_marked_as_deleted && file.marked_for_deletion)
        })
        .collect();

    let paths: Vec<String> = visible.iter().map(|f| display_path(&f.path)).collect();
    let path_w = paths
        .iter()
        .map(|p| p.len())
        .max()
        .unwrap_or(4)
        .max("path".len());
    let hash_w = visible
        .iter()
        .map(|f| f.hash.len().max(1))
        .max()
        .unwrap_or(4)
        .max("hash".len());
    let ext_w = visible
        .iter()
        .map(|f| f.extension.len().max(1))
        .max()
        .unwrap_or(9)
        .max("extension".len());

    println!(
        "{:<path_w$}  {:>14}  {:<hash_w$}  {:<ext_w$}  {:<9}  {}",
        "path",
        "Peso en Bytes",
        "hash",
        "extension",
        "duplicado",
        "a borrar",
        path_w = path_w,
        hash_w = hash_w,
        ext_w = ext_w,
    );
    println!(
        "{:-<path_w$}  {:-<14}  {:-<hash_w$}  {:-<ext_w$}  {:-<9}  {:-<8}",
        "",
        "",
        "",
        "",
        "",
        "",
        path_w = path_w,
        hash_w = hash_w,
        ext_w = ext_w,
    );

    for file in visible {
        let hash = if file.hash.is_empty() {
            "-"
        } else {
            file.hash.as_str()
        };
        let extension = if file.extension.is_empty() {
            "-"
        } else {
            file.extension.as_str()
        };
        let duplicated = if file.is_duped { "sí" } else { "no" };
        let to_delete = if file.marked_for_deletion { "sí" } else { "no" };

        println!(
            "{:<path_w$}  {:>14}  {:<hash_w$}  {:<ext_w$}  {:<9}  {}",
            display_path(&file.path),
            file.size,
            hash,
            extension,
            duplicated,
            to_delete,
            path_w = path_w,
            hash_w = hash_w,
            ext_w = ext_w,
        );
    }
}


pub fn mark_for_deletion(files: &mut [Entry]) {
    let mut kept_hashes = HashSet::new();

    for file in files.iter_mut() {
        if !file.is_duped || file.hash.is_empty() {
            continue;
        }
        // Original = primera ocurrencia de este hash en el orden de WalkDir (el Vec del scan).
        if kept_hashes.contains(&file.hash) {
            file.marked_for_deletion = true;
        } else {
            kept_hashes.insert(file.hash.clone());
        }
    }
}