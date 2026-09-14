use std::fs;
use sha2::{Sha256, Digest};
use walkdir::WalkDir;

pub fn scan_directory(start_dir: str, extension: &Vec) -> <Vec<PathBuf>{
    // Return all files from start dir

}

pub fn scan_duped_files(start_dir: str, extension: &Vec) -> <Vec<PathBuf>> {
    // Return all duped files from start_dir

}

pub fn check_files(files_vector: <Vec<PathBuff>>) -> <Vec<PathBuf>>{

    // Comprobamos el tamaño del fichero 
    // Comprobamos el hash

}

fn calculate_hash(file_path: &str) -> str { 
    // Return a file hash
    
}

fn calculate_filesize(file_path: &str) -> {
    // Return a file size (bytes)

}

pub fn create_output_dir() -> Result<T, E>{
    
    Ok(()); 
}

pub fn delete_dirs(dir: str) -> bool{

}

