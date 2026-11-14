use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use egui_extras::{Column, Size, StripBuilder, TableBuilder};

use crate::config::{Extensions, FileCategory};
use crate::filesystem::{self, display_path, Entry};
use crate::history::{self, EventKind, Recorder, SessionLog};
use crate::output::{self, Job, JobKind, Outcome};
use crate::platform;

enum ScanEvent {
    Progress(f32, String),
    Done(Vec<Entry>),
}

enum ApplyEvent {
    Progress(f32, String),
    Done(Vec<(PathBuf, Outcome)>),
}

/// Lo que haría "Apply" con el estado actual, para mostrarlo antes de ejecutar.
struct ApplyPlan {
    jobs: Vec<Job>,
    move_count: usize,
    move_bytes: u64,
    trash_count: usize,
    trash_bytes: u64,
    untouched: usize,
}

/// Dark Material surfaces with Rust orange accents (replacing the old lilac).
fn apply_rust_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    let surface = egui::Color32::from_rgb(28, 27, 31);
    let surface_high = egui::Color32::from_rgb(43, 41, 48);
    let primary = egui::Color32::from_rgb(247, 76, 0);
    let primary_container = egui::Color32::from_rgb(154, 52, 18);
    let on_surface = egui::Color32::from_rgb(230, 225, 229);
    let outline = egui::Color32::from_rgb(121, 116, 126);
    let radius = egui::CornerRadius::same(8);

    visuals.dark_mode = true;
    visuals.panel_fill = surface;
    visuals.window_fill = surface;
    visuals.window_corner_radius = egui::CornerRadius::same(12);
    visuals.menu_corner_radius = radius;
    visuals.faint_bg_color = egui::Color32::from_rgb(36, 34, 40);
    visuals.extreme_bg_color = egui::Color32::from_rgb(20, 19, 23);
    visuals.hyperlink_color = primary;
    visuals.warn_fg_color = egui::Color32::from_rgb(255, 180, 171);
    visuals.error_fg_color = egui::Color32::from_rgb(255, 180, 171);
    visuals.selection.bg_fill = primary_container;
    visuals.selection.stroke = egui::Stroke::new(1.0, primary);
    visuals.override_text_color = Some(on_surface);
    visuals.window_stroke = egui::Stroke::new(1.0, outline);
    visuals.striped = true;

    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
        widget.fg_stroke = egui::Stroke::new(1.0, on_surface);
    }

    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, outline);
    visuals.widgets.inactive.weak_bg_fill = surface_high;
    visuals.widgets.inactive.bg_fill = surface_high;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(72, 42, 28);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(72, 42, 28);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, primary);
    visuals.widgets.active.weak_bg_fill = primary_container;
    visuals.widgets.active.bg_fill = primary_container;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, primary);
    visuals.widgets.open.weak_bg_fill = surface_high;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, primary);

    ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Light, visuals);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            style.spacing.button_padding = egui::vec2(14.0, 8.0);
            style.spacing.interact_size.y = 28.0;
        });
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Duplicates,
    Review,
    History,
}

/// Orden de recorrido en la pestaña Review files.
#[derive(Clone, Copy, PartialEq)]
enum ReviewSort {
    ScanOrder,
    Newest,
    Oldest,
    Largest,
    Folder,
}

impl ReviewSort {
    const ALL: [ReviewSort; 5] = [
        ReviewSort::ScanOrder,
        ReviewSort::Newest,
        ReviewSort::Oldest,
        ReviewSort::Largest,
        ReviewSort::Folder,
    ];

    fn label(self) -> &'static str {
        match self {
            ReviewSort::ScanOrder => "Scan order",
            ReviewSort::Newest => "Newest first",
            ReviewSort::Oldest => "Oldest first",
            ReviewSort::Largest => "Largest first",
            ReviewSort::Folder => "By folder",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FilterKind {
    Image,
    Video,
    Audio,
    Documents,
    Custom,
}

#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    Name,
    Extension,
    Size,
    Status,
}

impl FilterKind {
    fn label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Documents => "documents",
            Self::Custom => "custom",
        }
    }

    fn preset_key(self) -> Option<&'static str> {
        match self {
            Self::Custom => None,
            other => Some(other.label()),
        }
    }

    fn from_label(label: &str) -> Self {
        match label {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "documents" => Self::Documents,
            _ => Self::Custom,
        }
    }
}

pub struct VolumeCleanerApp {
    status: String,
    tab: Tab,
    folder: Option<PathBuf>,
    filter: FilterKind,
    custom_extensions: String,
    presets: Extensions,
    files: Vec<Entry>,
    sort_key: SortKey,
    sort_ascending: bool,
    review_index: usize,
    scan_progress: f32,
    scan_rx: Option<Receiver<ScanEvent>>,
    loaded_image_path: Option<PathBuf>,
    current_image_uri: Option<String>,
    show_exit_dialog: bool,
    exit_confirmed: bool,
    show_summary_dialog: bool,
    summary_confirm_delete: bool,
    show_delete_duplicates_dialog: bool,
    review_actions_width: f32,
    exit_dialog_actions_width: f32,
    summary_actions_width: f32,
    delete_duplicates_actions_width: f32,
    /// `false`: se trabaja sobre el propio volumen (borrar = papelera).
    /// `true`: los archivos que se conservan se mueven a `output_folder`.
    use_output_folder: bool,
    output_folder: Option<PathBuf>,
    show_apply_dialog: bool,
    apply_actions_width: f32,
    apply_rx: Option<Receiver<ApplyEvent>>,
    apply_cancel: Option<Arc<AtomicBool>>,
    review_sort: ReviewSort,
    /// Carpetas que se saltan al escanear (texto editable, ver DEFAULT_EXCLUDES).
    excludes: String,
    /// Sesión en curso: se guarda en el historial a medida que ocurren cosas.
    recorder: Recorder,
    /// Sesiones guardadas, cargadas al abrir la pestaña Sessions.
    history: Vec<SessionLog>,
    history_selected: Option<String>,
    previous_tab: Tab,
}

impl VolumeCleanerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_rust_theme(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut app = Self {
            status: "-".to_string(),
            tab: Tab::Duplicates,
            folder: None,
            filter: FilterKind::Image,
            custom_extensions: String::new(),
            presets: Extensions::default(),
            files: Vec::new(),
            sort_key: SortKey::Name,
            sort_ascending: true,
            review_index: 0,
            scan_progress: 0.0,
            scan_rx: None,
            loaded_image_path: None,
            current_image_uri: None,
            show_exit_dialog: false,
            exit_confirmed: false,
            show_summary_dialog: false,
            summary_confirm_delete: false,
            show_delete_duplicates_dialog: false,
            review_actions_width: 0.0,
            exit_dialog_actions_width: 0.0,
            summary_actions_width: 0.0,
            delete_duplicates_actions_width: 0.0,
            use_output_folder: false,
            output_folder: None,
            show_apply_dialog: false,
            apply_actions_width: 0.0,
            apply_rx: None,
            apply_cancel: None,
            review_sort: ReviewSort::ScanOrder,
            excludes: filesystem::DEFAULT_EXCLUDES.to_string(),
            recorder: Recorder::new(),
            history: Vec::new(),
            history_selected: None,
            previous_tab: Tab::Duplicates,
        };
        filesystem::migrate_legacy_state();
        if let Some(session) = filesystem::load_last_session() {
            app.folder = Some(session.folder);
            app.filter = FilterKind::from_label(&session.filter_label);
            app.custom_extensions = session.custom_extensions;
            app.output_folder = session.output_folder;
            app.use_output_folder = session.use_output_folder;
            app.excludes = session.excludes;
            app.status = "Restored last session — press Scan volume to resume".to_string();
        }
        app
    }

    // Selecciona las extensiones para el scan
    fn extensions_for_scan(&self) -> Vec<String> {
        if let Some(key) = self.filter.preset_key() {
            return self.presets.get_extensios(key);
        }
        self.custom_extensions
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_start_matches('.').to_string())
            .collect()
    }

    // Guarda en disco las decisiones de revisión (keep/delete) y la posición
    // actual, para poder retomarlas si se cierra y reabre la aplicación.
    fn persist_review_state(&self) {
        if let Some(folder) = &self.folder {
            let _ = filesystem::save_review_state(folder, &self.files, self.review_index);
        }
    }

    fn save_session(&self) {
        if let Some(folder) = &self.folder {
            filesystem::save_last_session(
                folder,
                self.filter.label(),
                &self.custom_extensions,
                self.use_output_folder,
                self.output_folder.as_deref(),
                &self.excludes,
            );
        }
    }

    fn pick_output_folder(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.output_folder = Some(path);
            self.status = "Output folder selected".to_string();
            self.save_session();
        }
    }

    fn busy(&self) -> bool {
        self.scan_rx.is_some() || self.apply_rx.is_some()
    }

    // Funcion pick folder de la GUI
    fn pick_folder(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.folder = Some(path);
            self.status = "Folder selected".to_string();
        }
    }

    // Funcion scan de la GUI
    fn start_scan(&mut self, ctx: egui::Context) {
        if self.busy() {
            return;
        }
        let Some(folder) = &self.folder else {
            self.status = "Choose a folder first".to_string();
            return;
        };
        let Some(dir) = folder.to_str() else {
            self.status = "Folder path is not valid UTF-8".to_string();
            return;
        };
        let extensions = self.extensions_for_scan();
        if extensions.is_empty() {
            self.status = "No extensions to scan".to_string();
            return;
        }

        self.save_session();

        let excludes = filesystem::parse_excludes(&self.excludes);
        let dir = dir.to_string();
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.scan_progress = 0.0;
        self.status = "Scanning…".to_string();

        std::thread::spawn(move || {
            let files = filesystem::scan_directory(&dir, &extensions, &excludes, |fraction, status| {
                let _ = tx.send(ScanEvent::Progress(fraction, status.to_string()));
                ctx.request_repaint();
            });
            let files = filesystem::check_files(&files, |fraction, status| {
                let _ = tx.send(ScanEvent::Progress(fraction, status.to_string()));
                ctx.request_repaint();
            });
            let _ = tx.send(ScanEvent::Done(files));
            ctx.request_repaint();
        });
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.scan_rx else {
            return;
        };

        let mut events = Vec::new();
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        let mut done_files = None;
        for event in events {
            match event {
                ScanEvent::Progress(fraction, status) => {
                    self.scan_progress = fraction;
                    self.status = status;
                }
                ScanEvent::Done(files) => {
                    done_files = Some(files);
                }
            }
        }

        if let Some(files) = done_files {
            let count = files.len();
            self.files = files;
            self.review_index = 0;
            if let Some(folder) = self.folder.clone() {
                if let Some(state) = filesystem::load_review_state(&folder) {
                    filesystem::apply_review_state(&mut self.files, &state);
                    self.review_index = state.review_index.min(self.files.len().saturating_sub(1));
                }
            }
            self.scan_progress = 1.0;
            self.status = format!("Scanned {count} files");
            if let Some(folder) = self.folder.clone() {
                let duplicates = self.files.iter().filter(|f| f.is_duped).count();
                self.recorder.record(EventKind::Scan {
                    folder,
                    filter: self.filter.label().to_string(),
                    files: count,
                    duplicates,
                });
            }
            self.scan_rx = None;
            return;
        }

        if disconnected {
            self.status = "Scan interrupted".to_string();
            self.scan_rx = None;
            return;
        }

        ctx.request_repaint();
    }

    // Carga los bytes de la imagen actual en el contexto de egui, solo si
    // ha cambiado respecto al archivo mostrado en el frame anterior.
    fn ensure_image_loaded(&mut self, ctx: &egui::Context, path: &Path, ext: &str) {
        if self.loaded_image_path.as_deref() == Some(path) {
            return;
        }
        // Libera la imagen anterior para que la memoria no crezca con cada foto.
        if let Some(previous) = self.current_image_uri.take() {
            ctx.forget_image(&previous);
        }
        self.loaded_image_path = Some(path.to_path_buf());
        // HEIC/HEIF no lo decodifica egui: se previsualiza una conversión a JPEG.
        let source = if is_heic_extension(ext) {
            match platform::heic_preview(path) {
                Some(converted) => converted,
                None => {
                    self.current_image_uri = None;
                    return;
                }
            }
        } else {
            path.to_path_buf()
        };
        self.current_image_uri = match std::fs::read(&source) {
            Ok(bytes) => {
                let uri = format!("bytes://{}", path.display());
                ctx.include_bytes(uri.clone(), bytes);
                Some(uri)
            }
            Err(_) => None,
        };
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        let scanning = self.busy();
        let mut start_scan = false;

        ui.horizontal(|ui| {
            ui.add_enabled_ui(!scanning, |ui| {
                if ui.button("Browse…").clicked() {
                    self.pick_folder();
                }
            });
            match &self.folder {
                Some(path) => {
                    let shown = display_path(path);
                    ui.label(&shown).on_hover_text(path.display().to_string());
                }
                None => {
                    ui.weak("No folder selected");
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!scanning, egui::Button::new("Scan volume"))
                    .clicked()
                {
                    start_scan = true;
                }
                ui.add_enabled_ui(!scanning, |ui| {
                    if self.filter == FilterKind::Custom {
                        ui.text_edit_singleline(&mut self.custom_extensions)
                            .on_hover_text("jpg png webp  or  jpg,png,webp");
                    }
                    egui::ComboBox::from_id_salt("filter_kind")
                        .selected_text(self.filter.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.filter, FilterKind::Image, "image");
                            ui.selectable_value(&mut self.filter, FilterKind::Video, "video");
                            ui.selectable_value(&mut self.filter, FilterKind::Audio, "audio");
                            ui.selectable_value(&mut self.filter, FilterKind::Documents, "documents");
                            ui.selectable_value(&mut self.filter, FilterKind::Custom, "custom");
                        });
                    ui.label("Extensions:");
                });
            });
        });
        if start_scan {
            self.start_scan(ui.ctx().clone());
        }

        let mut excludes_changed = false;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!scanning, |ui| {
                ui.label("Skip folders:");
                excludes_changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut self.excludes)
                            .desired_width(420.0)
                            .hint_text(filesystem::DEFAULT_EXCLUDES),
                    )
                    .on_hover_text("Folder names skipped while scanning, separated by commas. `.*` skips every hidden folder.")
                    .lost_focus();
                if ui.button("Reset").clicked() {
                    self.excludes = filesystem::DEFAULT_EXCLUDES.to_string();
                    excludes_changed = true;
                }
            });
        });
        if excludes_changed {
            self.save_session();
        }

        let mut mode_changed = false;
        let mut pick_output = false;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!scanning, |ui| {
                ui.label("Output:");
                mode_changed |= ui
                    .radio_value(&mut self.use_output_folder, false, "Work on the volume itself")
                    .changed();
                mode_changed |= ui
                    .radio_value(&mut self.use_output_folder, true, "Move kept files to a folder")
                    .changed();
                if self.use_output_folder {
                    if ui.button("Browse output…").clicked() {
                        pick_output = true;
                    }
                    match &self.output_folder {
                        Some(path) => {
                            ui.label(display_path(path))
                                .on_hover_text(path.display().to_string());
                        }
                        None => {
                            ui.weak("No output folder selected");
                        }
                    }
                }
            });

            if self.use_output_folder {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(cancel) = &self.apply_cancel {
                        if ui.button("Stop").on_hover_text("Finish the current file and stop").clicked() {
                            cancel.store(true, Ordering::Relaxed);
                        }
                    } else if ui
                        .add_enabled(
                            !scanning && self.output_folder.is_some() && !self.files.is_empty(),
                            egui::Button::new("Apply…"),
                        )
                        .on_hover_text("Move kept files to the output folder and send files marked for deletion to the Trash")
                        .clicked()
                    {
                        self.show_apply_dialog = true;
                    }
                });
            }
        });
        if mode_changed {
            self.save_session();
        }
        if pick_output {
            self.pick_output_folder();
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Duplicates, "Remove duplicates");
            ui.selectable_value(&mut self.tab, Tab::Review, "Review files");
            ui.selectable_value(&mut self.tab, Tab::History, "Sessions");
        });
    }

    fn ui_duplicates(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.weak("Click a column header to sort");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let marked_count = self.duplicates_to_delete().len();
                if ui
                    .add_enabled(marked_count > 0, egui::Button::new("Delete duplicates"))
                    .clicked()
                {
                    self.show_delete_duplicates_dialog = true;
                }
                if ui
                    .button("Keep only 1")
                    .on_hover_text(
                        "Mark every duplicate for deletion except the first copy of each group",
                    )
                    .clicked()
                {
                    self.mark_all_but_one_duplicate();
                }
            });
        });

        let mut duped: Vec<usize> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.is_duped)
            .map(|(i, _)| i)
            .collect();

        let files = &self.files;
        let by_path = |a: usize, b: usize| files[a].path.cmp(&files[b].path);
        match self.sort_key {
            SortKey::Name => duped.sort_by(|&a, &b| by_path(a, b)),
            SortKey::Extension => duped.sort_by(|&a, &b| {
                files[a]
                    .extension
                    .to_ascii_lowercase()
                    .cmp(&files[b].extension.to_ascii_lowercase())
                    .then_with(|| by_path(a, b))
            }),
            SortKey::Size => {
                duped.sort_by(|&a, &b| files[a].size.cmp(&files[b].size).then_with(|| by_path(a, b)))
            }
            SortKey::Status => duped.sort_by(|&a, &b| {
                status_rank(&files[a])
                    .cmp(&status_rank(&files[b]))
                    .then_with(|| by_path(a, b))
            }),
        }
        if !self.sort_ascending {
            duped.reverse();
        }

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .column(Column::remainder().at_least(160.0).clip(true).resizable(false))
            .column(Column::auto().at_least(36.0))
            .column(Column::auto().at_least(72.0))
            .column(Column::auto().at_least(72.0))
            .column(Column::auto())
            .column(Column::auto().at_least(56.0))
            .header(22.0, |mut header| {
                let (key, asc) = (&mut self.sort_key, &mut self.sort_ascending);
                header.col(|ui| {
                    sort_header(ui, "Path", SortKey::Name, key, asc);
                });
                header.col(|ui| {
                    sort_header(ui, "Ext", SortKey::Extension, key, asc);
                });
                header.col(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        sort_header(ui, "Size", SortKey::Size, key, asc);
                    });
                });
                header.col(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        sort_header(ui, "Bytes", SortKey::Size, key, asc);
                    });
                });
                header.col(|ui| {
                    ui.strong("Action");
                });
                header.col(|ui| {
                    sort_header(ui, "Status", SortKey::Status, key, asc);
                });
            })
            .body(|body| {
                body.rows(24.0, duped.len(), |mut row| {
                    let index = duped[row.index()];
                    let path = display_path(&self.files[index].path);
                    let ext = if self.files[index].extension.is_empty() {
                        "-".to_string()
                    } else {
                        self.files[index].extension.clone()
                    };
                    let size = self.files[index].size;
                    let marked = self.files[index].marked_for_deletion;
                    let reviewed = self.files[index].reviewed;

                    row.col(|ui| {
                        ui.add(egui::Label::new(path).truncate())
                            .on_hover_text(self.files[index].path.display().to_string());
                    });
                    row.col(|ui| {
                        ui.label(ext);
                    });
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(format_size(size));
                        });
                    });
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(format_bytes(size));
                        });
                    });
                    row.col(|ui| {
                        if ui.button("Keep").clicked() {
                            self.set_decision(index, false);
                            self.persist_review_state();
                        }
                        if ui.button("Delete").clicked() {
                            self.set_decision(index, true);
                            self.persist_review_state();
                        }
                    });
                    row.col(|ui| {
                        if marked {
                            ui.colored_label(egui::Color32::from_rgb(200, 80, 80), "Delete");
                        } else if reviewed {
                            ui.colored_label(egui::Color32::from_rgb(120, 190, 120), "Keep");
                        } else {
                            ui.weak("Pending");
                        }
                    });
                });
            });
    }

    // Orden en que se recorren los archivos en la pestaña Review files.
    // `review_index` sigue siendo un índice de `self.files`; esto solo
    // decide cuál es el anterior y el siguiente.
    fn review_order(&self) -> Vec<usize> {
        let files = &self.files;
        let mut order: Vec<usize> = (0..files.len()).collect();
        match self.review_sort {
            ReviewSort::ScanOrder => {}
            ReviewSort::Newest => order.sort_by(|&a, &b| files[b].modified.cmp(&files[a].modified)),
            ReviewSort::Oldest => order.sort_by(|&a, &b| files[a].modified.cmp(&files[b].modified)),
            ReviewSort::Largest => order.sort_by(|&a, &b| files[b].size.cmp(&files[a].size)),
            ReviewSort::Folder => order.sort_by(|&a, &b| files[a].path.cmp(&files[b].path)),
        }
        order
    }

    // Marca un archivo como keep/delete y lo apunta en el historial.
    fn set_decision(&mut self, index: usize, delete: bool) {
        self.files[index].marked_for_deletion = delete;
        self.files[index].reviewed = true;
        self.recorder.decision(delete, 1);
    }

    fn ui_review(&mut self, ui: &mut egui::Ui) {
        if self.files.is_empty() {
            ui.weak("Scan a folder first.");
            return;
        }
        if self.review_index >= self.files.len() {
            self.review_index = self.files.len() - 1;
        }
        let order = self.review_order();
        let total = order.len();
        let index = self.review_index;
        let position = order.iter().position(|&i| i == index).unwrap_or(0);
        let pending = self.files.iter().filter(|f| !f.reviewed).count();
        // Siguiente pendiente en el orden actual, empezando después del
        // archivo actual y dando la vuelta al final.
        let next_pending = (1..=total)
            .map(|step| order[(position + step) % total])
            .find(|&i| !self.files[i].reviewed && i != index);

        let mut sort_changed = false;
        ui.horizontal(|ui| {
            ui.label(format!("File {}/{}", position + 1, total));
            ui.weak(format!("· {pending} pending"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Summary").clicked() {
                    self.show_summary_dialog = true;
                }
                let before = self.review_sort;
                egui::ComboBox::from_id_salt("review_sort")
                    .selected_text(self.review_sort.label())
                    .show_ui(ui, |ui| {
                        for option in ReviewSort::ALL {
                            ui.selectable_value(&mut self.review_sort, option, option.label());
                        }
                    });
                ui.label("Order:");
                sort_changed = before != self.review_sort;
            });
        });
        if sort_changed {
            // Al cambiar el orden, empieza por el primero del nuevo orden.
            let order = self.review_order();
            self.review_index = order.first().copied().unwrap_or(0);
            self.persist_review_state();
            return;
        }

        let file = &self.files[index];
        let path = file.path.clone();
        let ext = file.extension.clone();
        let is_duped = file.is_duped;
        let status = if !file.reviewed {
            ("Pending", egui::Color32::GRAY)
        } else if file.marked_for_deletion {
            ("Delete", egui::Color32::from_rgb(200, 80, 80))
        } else {
            ("Keep", egui::Color32::from_rgb(120, 190, 120))
        };
        ui.horizontal(|ui| {
            ui.label("Decision:");
            ui.colored_label(status.1, status.0);
            if is_duped {
                ui.colored_label(
                    egui::Color32::from_rgb(240, 170, 90),
                    "⚠ Duplicate — deleting it is handled in the Remove duplicates tab",
                );
            }
        });
        ui.label(format!("Path: {}", display_path(&path)));
        ui.label(format!(
            "Extension: {}",
            if ext.is_empty() { "-" } else { &ext }
        ));
        if let Ok(metadata) = std::fs::metadata(&path) {
            ui.label(format!("Size: {}", format_size(metadata.len())));
            if let Ok(modified) = metadata.modified() {
                ui.label(format!("Modified: {}", format_system_time(modified)));
            }
        }

        ui.separator();
        // Deja hueco por debajo para la línea de dimensiones, el separador y
        // los botones, y usa el resto de la altura disponible para la
        // previsualización, para que escale con el tamaño de la ventana.
        const RESERVED_BELOW_PREVIEW: f32 = 110.0;
        let preview_max_height = (ui.available_height() - RESERVED_BELOW_PREVIEW).max(120.0);
        ui.vertical_centered(|ui| match self.presets.category_for(&ext) {
            FileCategory::Image => self.ui_image_preview(ui, &path, &ext, preview_max_height),
            FileCategory::Documents if is_plain_text_extension(&ext) => {
                self.ui_text_preview(ui, &path, preview_max_height)
            }
            _ => self.ui_reveal_fallback(ui, &path),
        });
        ui.separator();

        let no_focus = ui.ctx().memory(|mem| mem.focused().is_none());
        let keep_pressed = no_focus && ui.input(|i| i.key_pressed(egui::Key::K));
        let delete_pressed = no_focus && ui.input(|i| i.key_pressed(egui::Key::D));
        let prev_pressed = no_focus && ui.input(|i| i.key_pressed(egui::Key::ArrowLeft));
        let next_pressed = no_focus && ui.input(|i| i.key_pressed(egui::Key::ArrowRight));
        let pending_pressed = no_focus && ui.input(|i| i.key_pressed(egui::Key::N));

        let prev = position.checked_sub(1).map(|p| order[p]);
        let next = order.get(position + 1).copied();
        let mut keep_clicked = false;
        let mut delete_clicked = false;
        let mut prev_clicked = false;
        let mut next_clicked = false;
        let mut pending_clicked = false;
        let mut folder_action: Option<bool> = None;

        // Pendientes en la misma carpeta (no recursivo) que el archivo actual.
        let folder = path.parent().map(Path::to_path_buf);
        let folder_pending: Vec<usize> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.reviewed && f.path.parent() == folder.as_deref())
            .map(|(i, _)| i)
            .collect();

        centered_horizontal(ui, &mut self.review_actions_width, |ui| {
            // ⏴/⏵/⏭ están en la fuente de iconos por defecto de egui (← → no).
            if ui
                .add_enabled(prev.is_some(), egui::Button::new("⏴"))
                .on_hover_text("Previous file (Left arrow key) — keeps its current decision")
                .clicked()
            {
                prev_clicked = true;
            }
            if ui.button("Keep (K)").clicked() {
                keep_clicked = true;
            }
            if ui.button("Delete (D)").clicked() {
                delete_clicked = true;
            }
            if ui
                .add_enabled(next.is_some(), egui::Button::new("⏵"))
                .on_hover_text("Next file (Right arrow key) — keeps its current decision")
                .clicked()
            {
                next_clicked = true;
            }
            if ui
                .add_enabled(next_pending.is_some(), egui::Button::new("⏭ Next pending"))
                .on_hover_text("Jump to the next file without a decision (N)")
                .clicked()
            {
                pending_clicked = true;
            }
            ui.add_enabled_ui(!folder_pending.is_empty(), |ui| {
                ui.menu_button(format!("Folder ({} pending)", folder_pending.len()), |ui| {
                    ui.weak(folder.as_deref().map(display_path).unwrap_or_default());
                    if ui.button("Keep all pending in this folder").clicked() {
                        folder_action = Some(false);
                        ui.close();
                    }
                    if ui.button("Delete all pending in this folder").clicked() {
                        folder_action = Some(true);
                        ui.close();
                    }
                });
            });
        });

        // Navegar sin decidir: no cambia keep/delete, solo la posición.
        let target = if prev_clicked || prev_pressed {
            prev
        } else if next_clicked || next_pressed {
            next
        } else if pending_clicked || pending_pressed {
            next_pending
        } else {
            None
        };
        if let Some(target) = target {
            self.review_index = target;
            self.persist_review_state();
            return;
        }

        if let Some(delete) = folder_action {
            for &i in &folder_pending {
                self.files[i].marked_for_deletion = delete;
                self.files[i].reviewed = true;
            }
            self.recorder.decision(delete, folder_pending.len());
            self.status = format!(
                "Marked {} file(s) in this folder as {}",
                folder_pending.len(),
                if delete { "delete" } else { "keep" },
            );
            self.persist_review_state();
            return;
        }

        if keep_clicked || keep_pressed || delete_clicked || delete_pressed {
            let delete = delete_clicked || delete_pressed;
            self.set_decision(index, delete);
            if let Some(next) = next {
                self.review_index = next;
            }
            self.persist_review_state();
        }
    }

    fn ui_image_preview(&mut self, ui: &mut egui::Ui, path: &Path, ext: &str, max_height: f32) {
        self.ensure_image_loaded(ui.ctx(), path, ext);
        match &self.current_image_uri {
            Some(uri) => {
                let dimensions = match ui
                    .ctx()
                    .try_load_image(uri, egui::load::SizeHint::default())
                {
                    Ok(egui::load::ImagePoll::Ready { image }) => Some(image.size),
                    _ => None,
                };
                ui.add(
                    egui::Image::new(uri.clone())
                        .max_height(max_height)
                        .max_width(ui.available_width())
                        .shrink_to_fit(),
                );
                if is_heic_extension(ext) {
                    ui.weak("HEIC preview (converted, may be downscaled)");
                } else if let Some([width, height]) = dimensions {
                    ui.label(format!("Dimensions: {width}x{height}"));
                }
            }
            None if is_heic_extension(ext) => {
                ui.weak("HEIC preview is only available on macOS.");
                if ui.button("Reveal in file manager").clicked() {
                    platform::reveal_in_file_manager(path);
                }
            }
            None => {
                ui.colored_label(egui::Color32::from_rgb(200, 80, 80), "Could not load image");
            }
        }
    }

    fn ui_text_preview(&self, ui: &mut egui::Ui, path: &Path, max_height: f32) {
        const MAX_PREVIEW_BYTES: u64 = 200 * 1024;
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size > MAX_PREVIEW_BYTES {
            ui.weak("File too large to preview as text.");
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(content) => {
                egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(content).monospace()).wrap());
                });
            }
            Err(_) => {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 80, 80),
                    "Could not read file as text",
                );
            }
        }
    }

    fn ui_reveal_fallback(&self, ui: &mut egui::Ui, path: &Path) {
        ui.weak("No preview available for this file type.");
        if ui.button("Reveal in file manager").clicked() {
            platform::reveal_in_file_manager(path);
        }
    }

    fn ui_exit_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_exit_dialog {
            return;
        }
        egui::Modal::new(egui::Id::new("exit_confirm")).show(ctx, |ui| {
            ui.set_width(280.0);
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                ui.label("Are you sure you want to exit Volume Cleaner?");
            });
            ui.add_space(16.0);
            let mut cancel_clicked = false;
            let mut exit_clicked = false;
            centered_horizontal(ui, &mut self.exit_dialog_actions_width, |ui| {
                if ui.button("Cancel").clicked() {
                    cancel_clicked = true;
                }
                if ui.button("Exit").clicked() {
                    exit_clicked = true;
                }
            });
            if cancel_clicked {
                self.show_exit_dialog = false;
            }
            if exit_clicked {
                self.show_exit_dialog = false;
                self.exit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.add_space(4.0);
        });
    }

    fn ui_summary_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_summary_dialog {
            return;
        }

        let to_delete: Vec<usize> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.reviewed && file.marked_for_deletion && !file.is_duped)
            .map(|(i, _)| i)
            .collect();
        let total_bytes: u64 = to_delete.iter().map(|&i| self.files[i].size).sum();

        egui::Modal::new(egui::Id::new("summary_dialog")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                if self.summary_confirm_delete {
                    ui.strong(format!(
                        "Move {} file(s) to the Trash — {}?",
                        to_delete.len(),
                        format_size(total_bytes),
                    ));
                    ui.weak("You can restore them from the system Trash / Recycle Bin.");
                } else {
                    ui.strong(format!(
                        "{} file(s) marked for deletion — {}",
                        to_delete.len(),
                        format_size(total_bytes),
                    ));
                    ui.weak("Reviewed files only; byte-identical duplicates are handled in the Remove duplicates tab.");
                }
            });
            ui.add_space(8.0);
            ui.separator();

            if to_delete.is_empty() {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| ui.weak("Nothing marked for deletion yet."));
                ui.add_space(8.0);
            } else {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for &i in &to_delete {
                        ui.label(display_path(&self.files[i].path))
                            .on_hover_text(self.files[i].path.display().to_string());
                    }
                });
            }

            ui.separator();
            ui.add_space(8.0);
            let confirming = self.summary_confirm_delete;
            let mut close_clicked = false;
            let mut delete_clicked = false;
            let mut cancel_clicked = false;
            let mut confirm_clicked = false;
            centered_horizontal(ui, &mut self.summary_actions_width, |ui| {
                if confirming {
                    if ui.button("Cancel").clicked() {
                        cancel_clicked = true;
                    }
                    if ui.button("Move to Trash").clicked() {
                        confirm_clicked = true;
                    }
                } else {
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                    if ui
                        .add_enabled(
                            !to_delete.is_empty(),
                            egui::Button::new(format!("Delete {} file(s)", to_delete.len())),
                        )
                        .clicked()
                    {
                        delete_clicked = true;
                    }
                }
            });
            if close_clicked {
                self.show_summary_dialog = false;
            }
            if delete_clicked {
                self.summary_confirm_delete = true;
            }
            if cancel_clicked {
                self.summary_confirm_delete = false;
            }
            if confirm_clicked {
                self.summary_confirm_delete = false;
                self.show_summary_dialog = false;
                self.delete_marked_reviewed();
            }
            ui.add_space(4.0);
        });
    }

    fn ui_delete_duplicates_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_delete_duplicates_dialog {
            return;
        }

        let to_delete = self.duplicates_to_delete();
        let marked_count = self
            .files
            .iter()
            .filter(|file| file.is_duped && file.marked_for_deletion)
            .count();
        let spared = marked_count - to_delete.len();
        let total_bytes: u64 = to_delete.iter().map(|&i| self.files[i].size).sum();

        egui::Modal::new(egui::Id::new("delete_duplicates_confirm")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                ui.strong(format!(
                    "Move {} duplicate file(s) to the Trash — {}?",
                    to_delete.len(),
                    format_size(total_bytes),
                ));
                ui.weak("You can restore them from the system Trash / Recycle Bin.");
                if spared > 0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 180, 171),
                        format!(
                            "{spared} file(s) will be kept: every copy of their group was marked, so the first copy is preserved.",
                        ),
                    );
                }
            });
            ui.add_space(8.0);
            ui.separator();

            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for &i in &to_delete {
                    ui.label(display_path(&self.files[i].path))
                        .on_hover_text(self.files[i].path.display().to_string());
                }
            });

            ui.separator();
            ui.add_space(8.0);
            let mut cancel_clicked = false;
            let mut confirm_clicked = false;
            centered_horizontal(ui, &mut self.delete_duplicates_actions_width, |ui| {
                if ui.button("Cancel").clicked() {
                    cancel_clicked = true;
                }
                if ui.button("Move to Trash").clicked() {
                    confirm_clicked = true;
                }
            });
            if cancel_clicked {
                self.show_delete_duplicates_dialog = false;
            }
            if confirm_clicked {
                self.show_delete_duplicates_dialog = false;
                self.delete_marked_duplicates();
            }
            ui.add_space(4.0);
        });
    }

    // Agrupa los duplicados por hash, en orden de escaneo: el primero de
    // cada grupo es la copia "original".
    fn duplicate_groups(&self) -> Vec<Vec<usize>> {
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, file) in self.files.iter().enumerate() {
            if !file.is_duped {
                continue;
            }
            let group = groups.entry(file.hash.clone()).or_insert_with(|| {
                order.push(file.hash.clone());
                Vec::new()
            });
            group.push(i);
        }
        order.into_iter().filter_map(|hash| groups.remove(&hash)).collect()
    }

    // Duplicados marcados para borrar, garantizando que de cada grupo de
    // archivos idénticos siempre quede al menos una copia: si están todas
    // marcadas, se conserva la primera.
    fn duplicates_to_delete(&self) -> Vec<usize> {
        let mut result = Vec::new();
        for group in self.duplicate_groups() {
            let marked: Vec<usize> = group
                .iter()
                .copied()
                .filter(|&i| self.files[i].marked_for_deletion)
                .collect();
            if marked.len() == group.len() {
                result.extend(marked.into_iter().skip(1));
            } else {
                result.extend(marked);
            }
        }
        result.sort_unstable();
        result
    }

    // Marca para borrar todas las copias de cada grupo excepto la primera,
    // que queda marcada para conservar.
    fn mark_all_but_one_duplicate(&mut self) {
        let (mut kept, mut deleted) = (0, 0);
        for group in self.duplicate_groups() {
            for (n, &i) in group.iter().enumerate() {
                self.files[i].marked_for_deletion = n > 0;
                self.files[i].reviewed = true;
                if n > 0 { deleted += 1 } else { kept += 1 }
            }
        }
        self.recorder.decision(false, kept);
        self.recorder.decision(true, deleted);
        self.persist_review_state();
    }

    // Manda a la papelera los duplicados marcados (respetando una copia por grupo).
    fn delete_marked_duplicates(&mut self) {
        let indices = self.duplicates_to_delete();
        self.trash_files(&indices, "duplicate file(s)", "Delete duplicates");
    }

    // Manda a la papelera los archivos revisados (no duplicados) marcados
    // para borrar desde la pestaña Review files.
    fn delete_marked_reviewed(&mut self) {
        let indices: Vec<usize> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.reviewed && file.marked_for_deletion && !file.is_duped)
            .map(|(i, _)| i)
            .collect();
        self.trash_files(&indices, "reviewed file(s)", "Summary");
    }

    // Mueve a la papelera del sistema los archivos indicados y actualiza el
    // estado en memoria y persistido para que dejen de aparecer.
    fn trash_files(&mut self, indices: &[usize], what: &str, origin: &str) {
        let mut deleted_paths = Vec::new();
        let mut deleted_bytes = 0u64;
        let mut failed = 0usize;
        let mut skipped = 0usize;
        let mut events = Vec::new();

        for &i in indices {
            let file = &self.files[i];
            // Los duplicados se vuelven a comprobar: si cambiaron desde el
            // escaneo ya no son copia de nada y no se borran.
            let expected = file.is_duped.then_some(file.hash.as_str());
            match output::trash_checked(&file.path, expected) {
                Outcome::Trashed => {
                    deleted_paths.push(file.path.clone());
                    deleted_bytes += file.size;
                    events.push(EventKind::Trashed {
                        path: file.path.clone(),
                        size: file.size,
                        origin: origin.to_string(),
                    });
                }
                Outcome::Skipped(reason) => {
                    skipped += 1;
                    events.push(EventKind::Skipped {
                        path: file.path.clone(),
                        reason,
                        origin: origin.to_string(),
                    });
                }
                Outcome::Failed(reason) => {
                    failed += 1;
                    events.push(EventKind::Failed {
                        path: file.path.clone(),
                        reason,
                        origin: origin.to_string(),
                    });
                }
                // trash_checked nunca mueve archivos.
                Outcome::Moved(_) => {}
            }
        }
        self.recorder.record_many(events);

        let deleted_set: HashSet<PathBuf> = deleted_paths.into_iter().collect();
        let deleted_count = deleted_set.len();
        self.remove_processed(&deleted_set);

        let mut status = format!(
            "Moved {deleted_count} {what} to the Trash ({})",
            format_size(deleted_bytes),
        );
        if skipped > 0 {
            status.push_str(&format!("; {skipped} skipped (changed since the scan)"));
        }
        if failed > 0 {
            status.push_str(&format!("; {failed} failed"));
        }
        self.status = status;
    }

    // Quita de la lista los archivos que ya no están en el origen y actualiza
    // el estado en memoria y persistido para que dejen de aparecer.
    fn remove_processed(&mut self, deleted_set: &HashSet<PathBuf>) {
        // Mantiene la posición de revisión sobre el mismo archivo: resta los
        // borrados que estaban antes del índice actual.
        let removed_before = self.files[..self.review_index.min(self.files.len())]
            .iter()
            .filter(|file| deleted_set.contains(&file.path))
            .count();
        self.review_index -= removed_before;
        self.files.retain(|file| !deleted_set.contains(&file.path));
        // Un grupo que se queda con una sola copia ya no es duplicado.
        for group in self.duplicate_groups() {
            if group.len() < 2 {
                for i in group {
                    self.files[i].is_duped = false;
                    self.files[i].marked_for_deletion = false;
                }
            }
        }
        if !self.files.is_empty() {
            self.review_index = self.review_index.min(self.files.len() - 1);
        } else {
            self.review_index = 0;
        }
        self.persist_review_state();
    }

    // Calcula qué haría "Apply" en modo carpeta de salida:
    // - a la papelera: duplicados sobrantes (siempre queda una copia por
    //   grupo) y archivos revisados marcados para borrar;
    // - a la salida: el resto de archivos revisados (keep, y la copia que se
    //   salva de cada grupo de duplicados);
    // - sin revisar: no se tocan.
    // Primero se mueven los que se conservan y después se borran los demás.
    fn plan_apply(&self) -> Option<ApplyPlan> {
        let input = self.folder.as_deref()?;
        let output_dir = self.output_folder.as_deref()?;

        let mut to_trash: HashSet<usize> = self.duplicates_to_delete().into_iter().collect();
        to_trash.extend(
            self.files
                .iter()
                .enumerate()
                .filter(|(_, f)| f.reviewed && f.marked_for_deletion && !f.is_duped)
                .map(|(i, _)| i),
        );

        let mut plan = ApplyPlan {
            jobs: Vec::new(),
            move_count: 0,
            move_bytes: 0,
            trash_count: 0,
            trash_bytes: 0,
            untouched: 0,
        };
        let mut trash_jobs = Vec::new();
        for (i, file) in self.files.iter().enumerate() {
            if to_trash.contains(&i) {
                plan.trash_count += 1;
                plan.trash_bytes += file.size;
                trash_jobs.push(Job {
                    source: file.path.clone(),
                    kind: JobKind::Trash {
                        expected_hash: file.is_duped.then(|| file.hash.clone()),
                    },
                });
            } else if file.reviewed {
                let category = self.presets.category_for(&file.extension);
                match output::destination_for(output_dir, input, &file.path, category) {
                    Some(dest) => {
                        plan.move_count += 1;
                        plan.move_bytes += file.size;
                        plan.jobs.push(Job { source: file.path.clone(), kind: JobKind::Move { dest } });
                    }
                    None => plan.untouched += 1,
                }
            } else {
                plan.untouched += 1;
            }
        }
        plan.jobs.extend(trash_jobs);
        Some(plan)
    }

    fn start_apply(&mut self, ctx: egui::Context) {
        if self.busy() {
            return;
        }
        let Some(plan) = self.plan_apply() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.apply_rx = Some(rx);
        self.apply_cancel = Some(cancel.clone());
        self.scan_progress = 0.0;
        self.status = "Applying…".to_string();

        std::thread::spawn(move || {
            let results = output::run_batch(plan.jobs, &cancel, |fraction, status| {
                let _ = tx.send(ApplyEvent::Progress(fraction, status.to_string()));
                ctx.request_repaint();
            });
            let _ = tx.send(ApplyEvent::Done(results));
            ctx.request_repaint();
        });
    }

    fn poll_apply(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.apply_rx else {
            return;
        };
        let mut done = None;
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(ApplyEvent::Progress(fraction, status)) => {
                    self.scan_progress = fraction;
                    self.status = status;
                }
                Ok(ApplyEvent::Done(results)) => done = Some(results),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if let Some(results) = done {
            let stopped = self
                .apply_cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed));
            let sizes: HashMap<&Path, u64> =
                self.files.iter().map(|f| (f.path.as_path(), f.size)).collect();
            let mut processed = HashSet::new();
            let mut events = Vec::new();
            let (mut moved, mut trashed, mut skipped, mut failed) = (0usize, 0usize, 0usize, 0usize);
            let mut first_error = None;
            for (path, outcome) in results {
                let size = sizes.get(path.as_path()).copied().unwrap_or(0);
                match outcome {
                    Outcome::Moved(dest) => {
                        moved += 1;
                        events.push(EventKind::Moved { path: path.clone(), dest, size });
                        processed.insert(path);
                    }
                    Outcome::Trashed => {
                        trashed += 1;
                        events.push(EventKind::Trashed {
                            path: path.clone(),
                            size,
                            origin: "Apply".to_string(),
                        });
                        processed.insert(path);
                    }
                    Outcome::Skipped(reason) => {
                        skipped += 1;
                        events.push(EventKind::Skipped { path, reason, origin: "Apply".to_string() });
                    }
                    Outcome::Failed(reason) => {
                        failed += 1;
                        first_error.get_or_insert_with(|| format!("{}: {reason}", display_path(&path)));
                        events.push(EventKind::Failed { path, reason, origin: "Apply".to_string() });
                    }
                }
            }
            drop(sizes);
            self.recorder.record_many(events);
            self.remove_processed(&processed);
            self.apply_rx = None;
            self.apply_cancel = None;
            self.scan_progress = 1.0;
            let mut status = format!("Moved {moved} file(s) to the output folder, {trashed} to the Trash");
            if skipped > 0 {
                status.push_str(&format!("; {skipped} skipped (changed since the scan)"));
            }
            if failed > 0 {
                status.push_str(&format!("; {failed} failed (e.g. {})", first_error.unwrap_or_default()));
            }
            if stopped {
                status.push_str(" — stopped");
            }
            self.status = status;
            return;
        }

        if disconnected {
            self.status = "Apply interrupted".to_string();
            self.apply_rx = None;
            self.apply_cancel = None;
            return;
        }
        ctx.request_repaint();
    }

    fn ui_apply_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_apply_dialog {
            return;
        }
        let Some(plan) = self.plan_apply() else {
            self.show_apply_dialog = false;
            return;
        };
        let validation = match (&self.folder, &self.output_folder) {
            (Some(input), Some(out)) => output::validate_output(input, out),
            _ => Err("Choose an input and an output folder first".to_string()),
        };
        let output_shown = self.output_folder.as_deref().map(display_path).unwrap_or_default();
        let free_space = self.output_folder.as_deref().and_then(output::available_space);
        let validation = validation.and_then(|()| match free_space {
            Some(free) if free < plan.move_bytes => Err(format!(
                "Not enough free space in the output folder: {} needed, {} available",
                format_size(plan.move_bytes),
                format_size(free),
            )),
            _ => Ok(()),
        });

        egui::Modal::new(egui::Id::new("apply_dialog")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.add_space(4.0);
            ui.vertical_centered(|ui| {
                ui.strong("Apply decisions");
                ui.weak(format!("Output: {output_shown}"));
            });
            ui.add_space(8.0);
            ui.separator();
            ui.label(format!(
                "• Move {} kept file(s) — {} to output/<category>/<relative path>",
                plan.move_count,
                format_size(plan.move_bytes),
            ));
            ui.label(format!(
                "• Send {} file(s) marked for deletion to the Trash — {}",
                plan.trash_count,
                format_size(plan.trash_bytes),
            ));
            ui.label(format!("• Leave {} unreviewed file(s) untouched", plan.untouched));
            if let Some(free) = free_space {
                ui.weak(format!("Free space in the output volume: {}", format_size(free)));
            }
            ui.add_space(6.0);
            ui.weak("Each kept file is copied, verified with SHA-256, and only then is the original sent to the Trash. If the process stops midway, the original stays in place.");
            if let Err(reason) = &validation {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::from_rgb(255, 180, 171), reason);
            }
            ui.separator();
            ui.add_space(8.0);

            let can_apply = validation.is_ok() && !plan.jobs.is_empty();
            let mut cancel_clicked = false;
            let mut apply_clicked = false;
            centered_horizontal(ui, &mut self.apply_actions_width, |ui| {
                if ui.button("Cancel").clicked() {
                    cancel_clicked = true;
                }
                if ui.add_enabled(can_apply, egui::Button::new("Apply")).clicked() {
                    apply_clicked = true;
                }
            });
            if cancel_clicked {
                self.show_apply_dialog = false;
            }
            if apply_clicked {
                self.show_apply_dialog = false;
                self.start_apply(ctx.clone());
            }
            ui.add_space(4.0);
        });
    }

    fn reload_history(&mut self) {
        self.history = history::load_sessions();
        let still_there = self
            .history_selected
            .as_ref()
            .is_some_and(|id| self.history.iter().any(|s| &s.id == id));
        if !still_there {
            self.history_selected = self.history.first().map(|s| s.id.clone());
        }
    }

    fn ui_history(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.weak(format!(
                "{} session(s) — one per app run in which something was scanned, decided, moved or deleted",
                self.history.len(),
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Refresh").clicked() {
                    refresh = true;
                }
            });
        });
        if refresh {
            self.reload_history();
        }
        ui.add_space(6.0);
        if self.history.is_empty() {
            ui.weak("No sessions yet. A session is saved as soon as you scan, decide, move or delete something.");
            return;
        }

        let mut select: Option<String> = None;
        let mut export: Option<String> = None;
        let mut reveal: Option<String> = None;
        let current_id = self.recorder.log.id.clone();

        StripBuilder::new(ui)
            .size(Size::exact(320.0))
            .size(Size::remainder())
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("history_list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for session in &self.history {
                                let totals = session.totals();
                                let selected = self.history_selected.as_deref() == Some(session.id.as_str());
                                let mut title = history::format_time(session.started);
                                if session.id == current_id {
                                    title.push_str("  (this session)");
                                }
                                let folders = session
                                    .folders()
                                    .iter()
                                    .map(|f| display_path(f))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                let mut line = format!(
                                    "{} moved · {} trashed",
                                    totals.moved, totals.trashed,
                                );
                                if totals.failed > 0 {
                                    line.push_str(&format!(" · {} failed", totals.failed));
                                }
                                let text = if folders.is_empty() {
                                    format!("{title}\n{line}")
                                } else {
                                    format!("{title}\n{folders}\n{line}")
                                };
                                if ui.selectable_label(selected, text).clicked() {
                                    select = Some(session.id.clone());
                                }
                                ui.add_space(4.0);
                            }
                        });
                });
                strip.cell(|ui| {
                    let Some(id) = self.history_selected.clone() else {
                        ui.weak("Select a session.");
                        return;
                    };
                    // La sesión en curso se muestra con los datos en memoria.
                    let session = if id == current_id {
                        Some(&self.recorder.log)
                    } else {
                        self.history.iter().find(|s| s.id == id)
                    };
                    let Some(session) = session else {
                        ui.weak("Select a session.");
                        return;
                    };
                    let totals = session.totals();

                    ui.horizontal(|ui| {
                        ui.strong(format!(
                            "Session {} — {}",
                            history::format_time(session.started),
                            history::format_time_with_seconds(session.ended),
                        ));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Show file").on_hover_text("Reveal the session's JSON file").clicked() {
                                reveal = Some(id.clone());
                            }
                            if ui
                                .add_enabled(!session.events.is_empty(), egui::Button::new("Export CSV…"))
                                .clicked()
                            {
                                export = Some(id.clone());
                            }
                        });
                    });
                    egui::Grid::new("history_totals").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
                        ui.weak("Folders scanned");
                        let folders = session.folders();
                        ui.label(if folders.is_empty() {
                            "-".to_string()
                        } else {
                            folders.iter().map(|f| display_path(f)).collect::<Vec<_>>().join(", ")
                        });
                        ui.end_row();
                        ui.weak("Decisions");
                        ui.label(format!(
                            "{} keep · {} delete",
                            session.keep_decisions, session.delete_decisions,
                        ));
                        ui.end_row();
                        ui.weak("Moved to output");
                        ui.label(format!("{} file(s) · {}", totals.moved, format_size(totals.moved_bytes)));
                        ui.end_row();
                        ui.weak("Sent to the Trash");
                        ui.label(format!("{} file(s) · {}", totals.trashed, format_size(totals.trashed_bytes)));
                        ui.end_row();
                        if totals.skipped > 0 || totals.failed > 0 {
                            ui.weak("Skipped / failed");
                            ui.label(format!("{} / {}", totals.skipped, totals.failed));
                            ui.end_row();
                        }
                    });
                    ui.separator();

                    if session.events.is_empty() {
                        ui.weak("Only review decisions in this session — no files were scanned, moved or deleted.");
                        return;
                    }
                    let events = &session.events;
                    TableBuilder::new(ui)
                        .id_salt("history_events")
                        .striped(true)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .auto_shrink([false, false])
                        .column(Column::auto().at_least(64.0))
                        .column(Column::auto().at_least(64.0))
                        .column(Column::initial(320.0).at_least(120.0).clip(true))
                        .column(Column::remainder().at_least(120.0).clip(true))
                        .column(Column::auto().at_least(64.0))
                        .header(22.0, |mut header| {
                            for title in ["Time", "Action", "File", "Destination / detail", "Size"] {
                                header.col(|ui| {
                                    ui.strong(title);
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(22.0, events.len(), |mut row| {
                                let event = &events[row.index()];
                                let (action, file, detail, size) = history::describe(event);
                                row.col(|ui| {
                                    ui.label(history::format_time_with_seconds(event.time));
                                });
                                row.col(|ui| {
                                    let color = match action {
                                        "Moved" => egui::Color32::from_rgb(120, 190, 120),
                                        "Trashed" => egui::Color32::from_rgb(200, 80, 80),
                                        "Failed" | "Skipped" => egui::Color32::from_rgb(240, 170, 90),
                                        _ => ui.visuals().text_color(),
                                    };
                                    ui.colored_label(color, action);
                                });
                                row.col(|ui| {
                                    ui.add(egui::Label::new(&file).truncate()).on_hover_text(&file);
                                });
                                row.col(|ui| {
                                    ui.add(egui::Label::new(&detail).truncate()).on_hover_text(&detail);
                                });
                                row.col(|ui| {
                                    ui.label(size.map(format_size).unwrap_or_default());
                                });
                            });
                        });
                });
            });

        if let Some(id) = select {
            self.history_selected = Some(id);
        }
        if let Some(id) = reveal {
            if let Some(path) = history::session_file(&id) {
                platform::reveal_in_file_manager(&path);
            }
        }
        if let Some(id) = export {
            let session = if id == current_id {
                Some(self.recorder.log.clone())
            } else {
                self.history.iter().find(|s| s.id == id).cloned()
            };
            if let Some(session) = session {
                let picked = rfd::FileDialog::new()
                    .set_file_name(format!("volume-cleaner-session-{id}.csv"))
                    .add_filter("CSV", &["csv"])
                    .save_file();
                if let Some(path) = picked {
                    self.status = match history::export_csv(&session, &path) {
                        Ok(()) => format!("Exported session to {}", display_path(&path)),
                        Err(e) => format!("Could not export CSV: {e}"),
                    };
                }
            }
        }
    }

    fn ui_footer(&self, ui: &mut egui::Ui) {
        ui.add(
            egui::ProgressBar::new(self.scan_progress)
                .desired_width(ui.available_width())
                .desired_height(8.0)
                .corner_radius(egui::CornerRadius::same(4))
                .fill(egui::Color32::from_rgb(247, 76, 0))
                .animate(self.busy() && self.scan_progress < 1.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(&self.status);
            ui.strong("Status:");
        });
    }
}

// egui's `horizontal()` always reserves the full available width for its
// row, so nesting it in `vertical_centered` doesn't actually center a short
// row of buttons. Instead, remember the row's rendered width from the last
// frame and use it to compute a leading offset that centers it this frame.
fn centered_horizontal(
    ui: &mut egui::Ui,
    remembered_width: &mut f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        let offset = ((ui.available_width() - *remembered_width) / 2.0).max(0.0);
        ui.add_space(offset);
        let inner = ui.horizontal(add_contents);
        *remembered_width = inner.response.rect.width();
    });
}

// Cabecera de columna clicable: el primer clic ordena ascendente por esa
// columna y los siguientes alternan el sentido. Muestra ⏶/⏷ en la activa
// (▲/▼ no existen en la fuente proporcional por defecto de egui).
fn sort_header(
    ui: &mut egui::Ui,
    label: &str,
    column: SortKey,
    sort_key: &mut SortKey,
    ascending: &mut bool,
) {
    let active = *sort_key == column;
    let text = if active {
        format!("{label} {}", if *ascending { "⏶" } else { "⏷" })
    } else {
        label.to_string()
    };
    let response = ui
        .add(egui::Button::new(egui::RichText::new(text).strong()).frame(false))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Sort by {}", label.to_lowercase()));
    if response.clicked() {
        if active {
            *ascending = !*ascending;
        } else {
            *sort_key = column;
            *ascending = true;
        }
    }
}

// Orden de la columna Status: pendientes, luego keep, luego delete.
fn status_rank(file: &Entry) -> u8 {
    if file.marked_for_deletion {
        2
    } else if file.reviewed {
        1
    } else {
        0
    }
}

fn is_heic_extension(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "heic" | "heif")
}

fn is_plain_text_extension(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "txt" | "md")
}

// 7070 → "7.070", con separador de miles para que se lea de un vistazo.
fn format_bytes(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    out
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

// Fecha en hora local (antes se mostraba en UTC).
fn format_system_time(time: std::time::SystemTime) -> String {
    let local: chrono::DateTime<chrono::Local> = time.into();
    local.format("%Y-%m-%d %H:%M").to_string()
}

fn panel_frame(ui: &egui::Ui, inner: egui::Margin, outer: egui::Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().panel_fill)
        .inner_margin(inner)
        .outer_margin(outer)
}

impl eframe::App for VolumeCleanerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_scan(ui.ctx());
        self.poll_apply(ui.ctx());

        if !self.exit_confirmed && ui.ctx().input(|i| i.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_exit_dialog = true;
        }
        self.ui_exit_dialog(ui.ctx());
        self.ui_summary_dialog(ui.ctx());
        self.ui_delete_duplicates_dialog(ui.ctx());
        self.ui_apply_dialog(ui.ctx());

        egui::Panel::top("header")
            .show_separator_line(false)
            .frame(panel_frame(
                ui,
                egui::Margin {
                    left: 50,
                    right: 50,
                    top: 50,
                    bottom: 50,
                },
                egui::Margin::ZERO,
            ))
            .show(ui, |ui| {
                self.ui_header(ui);
            });
        egui::Panel::bottom("footer")
            .show_separator_line(false)
            .frame(panel_frame(
                ui,
                egui::Margin {
                    left: 50,
                    right: 50,
                    top: 50,
                    bottom: 50,
                },
                egui::Margin::ZERO,
            ))
            .show(ui, |ui| {
                self.ui_footer(ui);
            });
        egui::CentralPanel::default()
            .frame(panel_frame(
                ui,
                egui::Margin {
                    left: 50,
                    right: 50,
                    top: 0,
                    bottom: 0,
                },
                egui::Margin::ZERO,
            ))
            .show(ui, |ui| {
                let applying = self.apply_rx.is_some();
                if self.tab == Tab::History && self.previous_tab != Tab::History {
                    self.reload_history();
                }
                self.previous_tab = self.tab;
                ui.add_enabled_ui(!applying, |ui| match self.tab {
                    Tab::Duplicates => self.ui_duplicates(ui),
                    Tab::Review => self.ui_review(ui),
                    Tab::History => self.ui_history(ui),
                });
            });
    }
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Volume Cleaner",
        options,
        Box::new(|cc| Ok(Box::new(VolumeCleanerApp::new(cc)))),
    )
}
