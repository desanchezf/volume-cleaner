mod filesystem;
mod gui;
mod config;



fn main() {
    let args: Vec<String> = std::env::args().collect();

    // argv: [binario, dir, ...resto]
    // cargo run -- fotos image          →  fotos + grupo "image"
    // cargo run -- fotos jpg png webp   →  fotos + extensiones custom
    if args.len() < 3 {
        eprintln!(
            "Usage:\n  {} <dir> <image|video|audio|documents>\n  {} <dir> <ext> [<ext>...]",
            args[0], args[0]
        );
        return;
    }

    let directory = &args[1];
    let rest = &args[2..];

    let presets = config::Extensions::default();
    let extensions = if rest.len() == 1 {
        let key = rest[0].as_str();
        let predefined = presets.get_extensios(key);
        if predefined.is_empty() {
            // un solo token que no es grupo → extensión custom (p.ej. "psd")
            vec![key.to_string()]
        } else {
            predefined
        }
    } else {
        rest.to_vec()
    };

    let files_by_extensions = filesystem::scan_directory(directory, &extensions);
    let files_checked = filesystem::check_files(&files_by_extensions);

    let print_marked_as_deleted = true;
    let print_marked_as_duped = true;

    filesystem::print_files(&files_checked, print_marked_as_deleted, print_marked_as_duped);


}