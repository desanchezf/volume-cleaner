# Volume Cleaner

A multi-platform photo/video/document/file/etc. cleaner — built to prevent **digital Diogenes syndrome**: the quiet pile-up of duplicates, forgotten exports, and files you will never open again.

A Rust + egui GUI application to scan storage volumes, preview media and documents, detect duplicates, and decide what stays so storage does not become a landfill.

## Features

- **Multi-platform support**: Runs on ARM (Apple Silicon), x64, x86, and x32 architectures across Windows, Linux, and macOS.
- **Recursive file scanning**: List all files recursively from selected directories or volumes.
- **File type filtering**: Group and filter by extensions:
  - 🖼️ Images (jpg, jpeg, png, webp, gif, bmp, etc.)
  - 🎥 Videos (mp4, mkv, mov, avi, webm, etc.)
  - 📄 Documents (pdf, txt, md, etc.)
  - 🎵 Audio (mp3, flac, wav, aac, etc.)
- **Custom extensions**: Users can specify additional custom file extensions to include in the scan (e.g., `.docx`, `.psd`, `.raw`).
- **Unsupported format handling**: For file types without built-in preview support (e.g., `.docx`), the application offers a **"Reveal in Explorer/Finder"** option to open the file's location in the system file manager, allowing the user to open it with the default application.
- **Duplicate detection**: Calculate file hashes (e.g., SHA-256) to identify duplicates. Only one copy of each unique hash is retained.
- **Tinder-like review**: Navigate files sequentially and mark for deletion or retention using keyboard shortcuts (e.g., `Y`/`N`, arrow keys).
- **Safe cleanup workflow**:
  - Copy retained files to a new output directory.
  - Delete originals from the source after confirmation.
- **Preview capabilities**:
  - Images: Full-resolution preview.
  - Videos: In-app playback.
  - Text/code files: Syntax-highlighted viewer.
  - PDFs: Page-by-page preview (rendered as images).
  - Unsupported formats: "Reveal in Explorer/Finder" to locate and open externally.
- **Future roadmap**:
  - Cloud storage cleanup (iCloud, Google Photos).
  - Advanced duplicate strategies (metadata, perceptual hashing for images).
  - Batch operations and undo support.

## Tech Stack

- **Language**: Rust
- **GUI Framework**: egui (via eframe)
- **Image handling**: `image` crate
- **Video playback**: `egui-video` / `egui-sharkplayer`
- **PDF rendering**: PDF crate + texture display in egui
- **Text/code viewing**: `egui_code_editor`
- **File hashing**: `sha2` or similar for duplicate detection
- **Cross-platform**: Built to run natively on all major OS and CPU architectures

## Why Rust + egui?

Volume Cleaner spends most of its time on I/O-heavy work — recursive scans, hashing large files, copying or deleting them — and the rest on a tight review loop: preview a file, decide keep or delete, move on. Rust plus egui (via eframe) cover both sides in one native binary.

- **Performance without a runtime**: Hashing and scanning large volumes stay fast and predictable. There is no garbage collector pausing the UI while a multi-gigabyte file is hashed.
- **Memory safety for dangerous operations**: Cleanup can permanently delete files. Rust's type system and ownership model reduce the class of bugs (use-after-free, data races, unchecked buffer issues) that are especially costly in a tool that mutates the filesystem.
- **Safe concurrency**: Directory traversal and SHA-256 hashing parallelize naturally across cores, without the usual race-condition tax.
- **One language, one process**: UI state, file I/O, hashing, and previews live in the same Rust binary. No Electron/JS bridge, no separate backend, no FFI tax between the scan pipeline and the review screen.
- **Immediate-mode fits the workflow**: The Tinder-like review is a tight loop (show file → shortcut → next). Immediate-mode UI redraws from current state each frame, which maps cleanly to "one candidate at a time".
- **Native and lightweight**: One codebase produces small, standalone executables for Windows, Linux, and macOS — including ARM (Apple Silicon) and x86/x32. egui renders natively on every target, without Qt or Electron.
- **Custom viewers in Rust**: Image, video, PDF, and syntax-highlighted text widgets paint into egui textures. Unsupported formats fall back to "Reveal in Explorer/Finder" without a second toolkit.

## Architecture

The application is organized into modular components for maintainability and extensibility:

```
volume-cleaner/
├── src/
│   ├── main.rs              # Application entry point
│   ├── app.rs               # Main egui app state and UI logic
│   ├── scanner/             # File system scanning and filtering
│   │   ├── mod.rs
│   │   ├── directory.rs     # Recursive directory traversal
│   │   └── filter.rs        # Extension-based file type filtering
│   ├── hasher/              # Duplicate detection via file hashing
│   │   ├── mod.rs
│   │   └── sha256.rs        # SHA-256 hash computation
│   ├── viewer/              # File preview components
│   │   ├── mod.rs
│   │   ├── image.rs         # Image preview widget
│   │   ├── video.rs         # Video playback widget
│   │   ├── text.rs          # Text/code viewer with syntax highlighting
│   │   ├── pdf.rs           # PDF page renderer
│   │   └── unsupported.rs   # Handler for unsupported formats (Reveal in Explorer/Finder)
│   ├── cleaner/             # File cleanup operations
│   │   ├── mod.rs
│   │   ├── copy.rs          # Copy files to output directory
│   │   └── delete.rs        # Delete source files safely
│   └── utils/               # Shared utilities
│   │   ├── mod.rs
│   │   └── path.rs          # Path manipulation helpers
│   │   └── platform.rs      # Cross-platform utilities (open file location, etc.)
├── Cargo.toml
└── README.md
```

### Module Responsibilities

- **`scanner`**: Recursively traverse directories, filter files by extension groups (images, videos, documents, audio), and build a list of candidate files. Supports user-defined custom extensions.
- **`hasher`**: Compute file hashes (SHA-256) for duplicate detection. Group files by hash and retain only one representative per unique hash.
- **`viewer`**: Provide egui widgets for previewing different file types:
  - Images: Load and display full-resolution.
  - Videos: Embed video player widget.
  - Text/code: Syntax-highlighted editor in read-only mode.
  - PDFs: Render pages as images for display.
  - Unsupported formats: Display "Format not supported" message with "Reveal in Explorer/Finder" button to open file location in system file manager.
- **`cleaner`**: Execute the cleanup workflow:
  - Copy retained files to the output directory, preserving directory structure.
  - Delete source files after user confirmation, with error handling and logging.
- **`utils`**: Helper functions for path manipulation, error handling, and common operations.
- **`platform`**: Cross-platform utilities for opening file locations (Finder on macOS, Explorer on Windows, file manager on Linux).

## How It Works

1. **Select source volume/directory**: Choose the folder or drive to scan.
2. **Configure filters**: Select which file types to include (images, videos, documents, audio) and add custom extensions.
3. **Scan and hash**: Recursively list files and compute hashes for duplicate detection.
4. **Review files**:
   - Navigate one file at a time.
   - Preview content (image, video, text, PDF).
   - For unsupported formats (e.g., `.docx`), see "Format not supported" with option to reveal in system file manager.
   - Press `Y` to keep, `N` to mark for deletion.
5. **Execute cleanup**:
   - Copy kept files to the output directory.
   - Optionally delete originals after confirmation.

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Y` / `→` | Keep file |
| `N` / `←` | Mark for deletion |
| `Space` | Toggle selection |
| `Enter` | Confirm and proceed to next step |
| `Esc` | Cancel / go back |
| `R` | Reveal file in Explorer/Finder (for unsupported formats) |

*(Shortcuts are configurable and will be documented in the app.)*

## System Requirements

- **Operating Systems**: Windows 10+, macOS 10.15+, Linux (modern distributions).
- **Architectures**: ARM64 (Apple Silicon), x64, x86, x32.
- **Dependencies**:
  - FFmpeg (required for video playback via `egui-video`). Install via:
    - macOS: `brew install ffmpeg`
    - Linux: `sudo apt install ffmpeg` or equivalent
    - Windows: Download from [ffmpeg.org](https://ffmpeg.org/download.html) and add to PATH
- **Memory**: Minimum 512 MB RAM recommended for large file sets.
- **Disk Space**: Sufficient space for output directory (at least equal to size of files to retain).

## Installation

```bash
git clone https://github.com/your-username/volume-cleaner
cd volume-cleaner
cargo build --release
```

Run the binary from `target/release/volume-cleaner` (or `volume-cleaner.exe` on Windows).

### Pre-built Binaries

Pre-compiled binaries for common platforms will be available in the [Releases](https://github.com/your-username/volume-cleaner/releases) section.

## Roadmap

- [ ] Cloud integration (iCloud, Google Photos)
- [ ] Perceptual hashing for near-duplicate images
- [ ] Undo/restore deleted files
- [ ] Batch selection and filters
- [ ] Progress indicators and statistics
- [ ] Dark/light theme toggle
- [ ] Operation log export (JSON/CSV)

## Contributing

Contributions are welcome! Please open an issue or submit a PR for bugs, features, or improvements.

## License

MIT

---

**Note**: This tool modifies files on your system. Always back up important data before running cleanup operations. The application provides a confirmation step before deleting any files, and logs all operations for review.