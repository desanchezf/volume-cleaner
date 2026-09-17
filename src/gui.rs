use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::config::{Extensions, FileCategory};
use crate::filesystem::{self, display_path, Entry};
use crate::platform;

enum ScanEvent {
    Progress(f32, String),
    Done(Vec<Entry>),
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
    Size,
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
}

impl VolumeCleanerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_rust_theme(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        Self {
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
        }
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

    // Funcion pick folder de la GUI
    fn pick_folder(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.folder = Some(path);
            self.status = "Folder selected".to_string();
        }
    }

    // Funcion scan de la GUI
    fn start_scan(&mut self, ctx: egui::Context) {
        if self.scan_rx.is_some() {
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

        let dir = dir.to_string();
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.scan_progress = 0.0;
        self.status = "Scanning…".to_string();

        std::thread::spawn(move || {
            let files = filesystem::scan_directory(&dir, &extensions, |fraction, status| {
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
            self.scan_progress = 1.0;
            self.status = format!("Scanned {count} files");
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
    fn ensure_image_loaded(&mut self, ctx: &egui::Context, path: &Path) {
        if self.loaded_image_path.as_deref() == Some(path) {
            return;
        }
        self.loaded_image_path = Some(path.to_path_buf());
        self.current_image_uri = match std::fs::read(path) {
            Ok(bytes) => {
                let uri = format!("bytes://{}", path.display());
                ctx.include_bytes(uri.clone(), bytes);
                Some(uri)
            }
            Err(_) => None,
        };
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        let scanning = self.scan_rx.is_some();
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
        ui.separator();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Duplicates, "Remove duplicates");
            ui.selectable_value(&mut self.tab, Tab::Review, "Review files");
        });
    }

    fn ui_duplicates(&mut self, ui: &mut egui::Ui) {
        ui.label("Byte-identical copies. Keep or delete each row; nothing is removed until you confirm later.");

        ui.horizontal(|ui| {
            ui.label("Sort by:");
            for (key, label) in [(SortKey::Name, "Name"), (SortKey::Size, "Size")] {
                if ui.selectable_label(self.sort_key == key, label).clicked() {
                    if self.sort_key == key {
                        self.sort_ascending = !self.sort_ascending;
                    } else {
                        self.sort_key = key;
                        self.sort_ascending = true;
                    }
                }
            }
            ui.weak(if self.sort_ascending { "▲" } else { "▼" });
        });

        let mut duped: Vec<usize> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.is_duped)
            .map(|(i, _)| i)
            .collect();

        match self.sort_key {
            SortKey::Name => duped.sort_by(|&a, &b| self.files[a].path.cmp(&self.files[b].path)),
            SortKey::Size => duped.sort_by_key(|&i| self.files[i].size),
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
            .column(Column::auto().at_least(56.0))
            .column(Column::auto())
            .column(Column::auto())
            .column(Column::auto().at_least(48.0))
            .header(22.0, |mut header| {
                header.col(|ui| {
                    ui.strong("path");
                });
                header.col(|ui| {
                    ui.strong("ext");
                });
                header.col(|ui| {
                    ui.strong("bytes");
                });
                header.col(|ui| {
                    ui.strong("keep");
                });
                header.col(|ui| {
                    ui.strong("delete");
                });
                header.col(|ui| {
                    ui.strong("marked");
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

                    row.col(|ui| {
                        ui.add(egui::Label::new(path).truncate())
                            .on_hover_text(self.files[index].path.display().to_string());
                    });
                    row.col(|ui| {
                        ui.label(ext);
                    });
                    row.col(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(size.to_string());
                        });
                    });
                    row.col(|ui| {
                        if ui.button("Keep").clicked() {
                            self.files[index].marked_for_deletion = false;
                        }
                    });
                    row.col(|ui| {
                        if ui.button("Delete").clicked() {
                            self.files[index].marked_for_deletion = true;
                        }
                    });
                    row.col(|ui| {
                        if marked {
                            ui.colored_label(egui::Color32::from_rgb(200, 80, 80), "yes");
                        } else {
                            ui.weak("no");
                        }
                    });
                });
            });
    }

    fn ui_review(&mut self, ui: &mut egui::Ui) {
        ui.label("One file at a time. Keep or delete, then next.");
        if self.files.is_empty() {
            ui.weak("Scan a folder first.");
            return;
        }
        if self.review_index >= self.files.len() {
            self.review_index = self.files.len() - 1;
        }
        let total = self.files.len();
        let index = self.review_index;
        ui.label(format!("{}/{}", index + 1, total));
        ui.label(display_path(&self.files[index].path));
        let ext = self.files[index].extension.clone();
        ui.label(if ext.is_empty() {
            "extension: -".to_string()
        } else {
            format!("extension: {ext}")
        });

        ui.separator();
        let path = self.files[index].path.clone();
        match self.presets.category_for(&ext) {
            FileCategory::Image => self.ui_image_preview(ui, &path),
            FileCategory::Documents if is_plain_text_extension(&ext) => {
                self.ui_text_preview(ui, &path)
            }
            _ => self.ui_reveal_fallback(ui, &path),
        }
        ui.separator();

        ui.horizontal(|ui| {
            if ui.button("Keep").clicked() {
                self.files[index].marked_for_deletion = false;
                self.review_index = (index + 1).min(total - 1);
            }
            if ui.button("Delete").clicked() {
                self.files[index].marked_for_deletion = true;
                self.review_index = (index + 1).min(total - 1);
            }
        });
    }

    fn ui_image_preview(&mut self, ui: &mut egui::Ui, path: &Path) {
        self.ensure_image_loaded(ui.ctx(), path);
        match &self.current_image_uri {
            Some(uri) => {
                ui.add(
                    egui::Image::new(uri.clone())
                        .max_height(320.0)
                        .max_width(ui.available_width())
                        .shrink_to_fit(),
                );
            }
            None => {
                ui.colored_label(egui::Color32::from_rgb(200, 80, 80), "Could not load image");
            }
        }
    }

    fn ui_text_preview(&self, ui: &mut egui::Ui, path: &Path) {
        const MAX_PREVIEW_BYTES: u64 = 200 * 1024;
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size > MAX_PREVIEW_BYTES {
            ui.weak("File too large to preview as text.");
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(content) => {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
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
            ui.label("Are you sure you want to exit Volume Cleaner?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.show_exit_dialog = false;
                }
                if ui.button("Exit").clicked() {
                    self.show_exit_dialog = false;
                    self.exit_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }

    fn ui_footer(&self, ui: &mut egui::Ui) {
        ui.add(
            egui::ProgressBar::new(self.scan_progress)
                .desired_width(ui.available_width())
                .desired_height(8.0)
                .corner_radius(egui::CornerRadius::same(4))
                .fill(egui::Color32::from_rgb(247, 76, 0))
                .animate(self.scan_rx.is_some() && self.scan_progress < 1.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(&self.status);
            ui.strong("Status:");
        });
    }
}

fn is_plain_text_extension(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "txt" | "md")
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

        if !self.exit_confirmed && ui.ctx().input(|i| i.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_exit_dialog = true;
        }
        self.ui_exit_dialog(ui.ctx());

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
                match self.tab {
                    Tab::Duplicates => self.ui_duplicates(ui),
                    Tab::Review => self.ui_review(ui),
                }
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
