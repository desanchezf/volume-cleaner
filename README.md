# Volume Cleaner

A multi-platform photo/video/document/file/etc. cleaner — built to prevent **digital Diogenes syndrome**: the quiet pile-up of duplicates, forgotten exports, and files you will never open again.

A Rust + egui GUI application to scan storage volumes, preview files, detect duplicates, and decide what stays so storage does not become a landfill.

## Features

- **Multi-platform support**: Runs on ARM (Apple Silicon), x64, x86, and x32 architectures across Windows, Linux, and macOS.
- **Home-relative paths**: Listed paths replace the user home directory with `~` (`~/Documents/gifs/foto.jpg`). This is display-only; stored paths stay absolute. Home is taken from `HOME` on macOS/Linux and `USERPROFILE` on Windows. Separators are normalized to `/` in the `~` form so the same listing reads well on all three platforms.
- **Recursive file scanning**: List all files recursively from a selected directory or volume, with live progress feedback.
- **File type filtering**: Group and filter by extensions:
  - 🖼️ Images (jpg, jpeg, png, webp, gif, bmp)
  - 🎥 Videos (mp4, mkv, mov, avi, webm)
  - 📄 Documents (pdf, txt, md, doc, docx, odt)
  - 🎵 Audio (mp3, flac, wav, aac, ogg, m4a)
  - ✏️ Custom: user-specified extensions (e.g., `psd, raw` or `.psd .raw`)
- **Duplicate detection**: Files are first grouped by size, then by a partial hash of their first bytes, and only real candidates get a full SHA-256 hash — avoiding hashing every large file up front. The **original** is just the first occurrence found during the scan (depth-first, unsorted); the review GUI shows the duplicate flag, and the user marks keep or delete per row. A "Borrar duplicados" button then permanently deletes every duplicate currently marked for deletion, but only after a confirmation modal lists exactly which files will be removed and their combined size — nothing is deleted without that explicit confirmation.
- **Tinder-like review**: Navigate matched files one at a time and mark for deletion or retention using the on-screen buttons or the `K` (keep) / `D` (delete) keyboard shortcuts. Each decision immediately advances to the next file.
- **Resumable review sessions**: Review decisions (`keep`/`delete`) and your position in the queue are persisted to disk per scanned folder, and the last folder/filter used is remembered too — so closing and reopening the app, then scanning the same folder again, resumes right where you left off. See [Review state persistence](#review-state-persistence).
- **Preview capabilities**:
  - Images: full-resolution preview that scales with the window, plus size, modified date, and pixel dimensions.
  - Plain text (`.txt`, `.md`): scrollable monospace preview.
  - Other formats: "Reveal in file manager" button to locate and open externally (Finder/Explorer/`xdg-open`).
- **Exit confirmation**: Closing the window asks for confirmation before quitting.
- **Future roadmap**: see [Roadmap](#roadmap) below — cleanup/copy workflow, video/PDF/syntax-highlighted previews, cloud storage cleanup, perceptual hashing.

## Tech Stack

- **Language**: Rust
- **GUI Framework**: [egui](https://github.com/emilk/egui) (via `eframe`), with a custom dark "Rust orange" Material-inspired theme
- **Image decoding/display**: `egui_extras` (image loaders, WebP/GIF support), backed by the `image` crate
- **File hashing**: `sha2` (SHA-256) for duplicate detection
- **Directory traversal**: `walkdir`
- **Native folder picker**: `rfd`
- **Cross-platform**: Built to run natively on all major OS and CPU architectures

Video playback, PDF rendering, and syntax-highlighted text are **not implemented yet** — they're tracked in the [Roadmap](#roadmap) and will bring their own dependencies when they land.

## Why Rust + egui?

Volume Cleaner spends most of its time on I/O-heavy work — recursive scans, hashing large files, copying or deleting them — and the rest on a tight review loop: preview a file, decide keep or delete, move on. Rust plus egui (via eframe) cover both sides in one native binary.

- **Performance without a runtime**: Hashing and scanning large volumes stay fast and predictable. There is no garbage collector pausing the UI while a multi-gigabyte file is hashed.
- **Memory safety for dangerous operations**: Cleanup can permanently delete files. Rust's type system and ownership model reduce the class of bugs (use-after-free, data races, unchecked buffer issues) that are especially costly in a tool that mutates the filesystem.
- **Safe concurrency**: The scan and hashing pipeline runs on a background thread and streams progress back to the UI over a channel, keeping the window responsive.
- **One language, one process**: UI state, file I/O, hashing, and previews live in the same Rust binary. No Electron/JS bridge, no separate backend, no FFI tax between the scan pipeline and the review screen.
- **Immediate-mode fits the workflow**: The Tinder-like review is a tight loop (show file → shortcut → next). Immediate-mode UI redraws from current state each frame, which maps cleanly to "one candidate at a time".
- **Native and lightweight**: One codebase produces small, standalone executables for Windows, Linux, and macOS — including ARM (Apple Silicon) and x86/x32. egui renders natively on every target, without Qt or Electron.

## Architecture

The application is currently organized as a small set of flat modules:

```
volume-cleaner/
├── src/
│   ├── main.rs          # Application entry point — wires up the modules and runs the GUI
│   ├── gui.rs            # eframe::App implementation: theme, tabs, panels, review/duplicates screens
│   ├── filesystem.rs      # Recursive scanning, size/partial-hash/full-hash duplicate detection,
│   │                       # home-relative path display, and review-state/session persistence
│   ├── config.rs          # `Extensions` presets (image/video/audio/documents) and category lookup
│   └── platform.rs        # Cross-platform "reveal in file manager" (Finder/Explorer/xdg-open)
├── Cargo.toml
└── README.md
```

### Module Responsibilities

- **`gui`**: Owns all `eframe::App` state and rendering — the header (folder picker, extension filter, scan button), the "Remove duplicates" table (sort by name/size, per-row keep/delete, byte counts), the "Review files" one-at-a-time screen (metadata, preview, centered keep/delete controls, keyboard shortcuts), the exit-confirmation modal, and the footer progress bar/status line. Runs the scan on a background thread and polls it via an `mpsc` channel so the UI stays responsive.
- **`filesystem`**: Recursively walks a directory (`walkdir`), filters by extension, and produces `Entry` values (path, size, extension). Detects duplicates by grouping same-size files, discarding non-matches early via a partial hash of the first bytes, and only computing a full SHA-256 over files that still collide. Also owns home-relative path display and the on-disk persistence of review decisions and the last-used session (see below).
- **`config`**: Defines the built-in extension presets and maps an extension to a `FileCategory` so the review screen can pick the right preview widget.
- **`platform`**: Opens the OS file manager at a given path (`open -R` on macOS, `explorer /select,` on Windows, `xdg-open` on the parent directory on Linux).

## How It Works

1. **Select source volume/directory**: Click "Browse…" to choose a folder (or let it be pre-filled from your last session).
2. **Configure filters**: Pick a preset (image/video/audio/documents) or "custom" with your own extensions, then click "Scan volume".
3. **Scan and hash**: Files are listed recursively and duplicates are detected via partial-then-full hashing, with progress shown in the footer.
4. **Review files**:
   - **Remove duplicates** tab: byte-identical copies only, sortable by name or size, keep/delete per row.
   - **Review files** tab: every scanned file, one at a time, with preview and metadata; press `K` to keep or `D` to delete (or click the buttons) to advance.
   - Unsupported formats show "Reveal in file manager" instead of a preview.
5. **Resume later**: Your keep/delete decisions and review position are saved automatically as you go — see below.

## Review state persistence

Two small files are written under a `session/` directory created next to where the app is run from (i.e. relative to the current working directory):

- `session/last_session.tsv` — the last scanned folder, extension filter, and custom extensions, so they're pre-filled the next time you open the app.
- `session/state/<hash-of-folder-path>.tsv` — one file per scanned folder, recording every file's keep/delete decision and the current review index.

Both are plain tab-separated text (no external database or serialization crate). State is applied by matching absolute file paths after a fresh scan, so to resume you still need to scan the same folder again — the app does not auto-scan on launch. Nothing here is delete-related: it only remembers your *decisions*, not an actual cleanup log. The `session/` directory is git-ignored since it holds per-user, per-machine runtime state.

## Keyboard Shortcuts

| Key | Action | Where |
|-----|--------|-------|
| `K` | Keep the current file and move to the next | Review files tab |
| `D` | Mark the current file for deletion and move to the next | Review files tab |

Shortcuts are ignored while a text field (e.g. the custom-extensions box) has keyboard focus.

## System Requirements

- **Operating Systems**: Windows 10+, macOS 10.15+, Linux (modern distributions).
- **Architectures**: ARM64 (Apple Silicon), x64, x86, x32.
- **Memory**: Minimum 512 MB RAM recommended for large file sets.
- **Disk Space**: No extra space is required today, since files are deleted in place and nothing is copied yet — plan for extra space once the copy-to-output-directory workflow (see Roadmap) lands.

### Home directory (`~`)

The `~` prefix in the UI is a **display** convention, not a rewrite of `Entry.path`.

| | macOS / Linux | Windows |
|--|----------------|---------|
| Home variable | `HOME` (`/Users/you`) | `USERPROFILE` (`C:\Users\you`) |
| `~` on screen | yes | yes (`~/Documents/...`) |

Matching uses `Path::strip_prefix`, so Windows backslashes still count as home. On Windows the comparison is case-sensitive: if a path is `c:\...` and `USERPROFILE` is `C:\...`, the substitution may be skipped.

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

- [ ] Copy-then-delete cleanup workflow for the "Review files" tab: copy retained files to an output directory before deleting originals from the source (today, marking a file in the Review tab only sets a persisted flag; the Duplicates tab already deletes for real, behind a confirmation modal).
- [ ] Video playback preview (e.g. `egui-video`).
- [ ] PDF page-by-page preview.
- [ ] Syntax-highlighted preview for code files.
- [ ] Cloud integration (iCloud, Google Photos).
- [ ] Perceptual hashing for near-duplicate images.
- [ ] Undo/restore deleted files.
- [ ] Batch selection and filters.
- [ ] Light theme toggle (currently dark-only).
- [ ] Operation log export (JSON/CSV).

## Contributing

Contributions are welcome! Please open an issue or submit a PR for bugs, features, or improvements.

## License

MIT

---

**Note**: This tool modifies files on your system. Always back up important data before deleting duplicates. The "Borrar duplicados" button permanently removes files from disk (no trash/recycle bin) after you confirm a modal listing exactly what will be deleted. The "Review files" tab, by contrast, only records keep/delete decisions for now — nothing there is copied or deleted automatically yet.
