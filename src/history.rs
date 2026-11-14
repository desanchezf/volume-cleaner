// Historial de sesiones: cada ejecución de la aplicación es una sesión, que
// se guarda como JSON en `<carpeta de datos>/sessions/<id>.json` en cuanto
// ocurre algo (escaneo, decisión, archivo movido o enviado a la papelera).
// Abrir y cerrar la aplicación sin hacer nada no deja ningún archivo.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};

use crate::filesystem;

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionLog {
    /// Identificador y nombre del archivo, p. ej. `20261114-153012`.
    pub id: String,
    /// Segundos Unix.
    pub started: i64,
    pub ended: i64,
    #[serde(default)]
    pub keep_decisions: usize,
    #[serde(default)]
    pub delete_decisions: usize,
    #[serde(default)]
    pub events: Vec<Event>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Event {
    pub time: i64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    Scan {
        folder: PathBuf,
        filter: String,
        files: usize,
        duplicates: usize,
    },
    Moved {
        path: PathBuf,
        dest: PathBuf,
        size: u64,
    },
    Trashed {
        path: PathBuf,
        size: u64,
        /// Desde dónde se borró: "Delete duplicates", "Summary" o "Apply".
        origin: String,
    },
    Skipped {
        path: PathBuf,
        reason: String,
        origin: String,
    },
    Failed {
        path: PathBuf,
        reason: String,
        origin: String,
    },
}

/// Totales de una sesión, para la lista y la cabecera del detalle.
#[derive(Default)]
pub struct Totals {
    pub scans: usize,
    pub moved: usize,
    pub moved_bytes: u64,
    pub trashed: usize,
    pub trashed_bytes: u64,
    pub skipped: usize,
    pub failed: usize,
}

impl SessionLog {
    pub fn totals(&self) -> Totals {
        let mut t = Totals::default();
        for event in &self.events {
            match &event.kind {
                EventKind::Scan { .. } => t.scans += 1,
                EventKind::Moved { size, .. } => {
                    t.moved += 1;
                    t.moved_bytes += size;
                }
                EventKind::Trashed { size, .. } => {
                    t.trashed += 1;
                    t.trashed_bytes += size;
                }
                EventKind::Skipped { .. } => t.skipped += 1,
                EventKind::Failed { .. } => t.failed += 1,
            }
        }
        t
    }

    /// Carpetas escaneadas en la sesión, sin repetir.
    pub fn folders(&self) -> Vec<&Path> {
        let mut folders: Vec<&Path> = Vec::new();
        for event in &self.events {
            if let EventKind::Scan { folder, .. } = &event.kind {
                if !folders.contains(&folder.as_path()) {
                    folders.push(folder);
                }
            }
        }
        folders
    }
}

fn sessions_dir() -> Option<PathBuf> {
    filesystem::app_state_dir().map(|dir| dir.join("sessions"))
}

pub fn session_file(id: &str) -> Option<PathBuf> {
    sessions_dir().map(|dir| dir.join(format!("{id}.json")))
}

fn now() -> i64 {
    Local::now().timestamp()
}

/// Lleva la sesión en curso y la guarda en disco tras cada cambio.
pub struct Recorder {
    pub log: SessionLog,
}

impl Recorder {
    pub fn new() -> Self {
        let started = Local::now();
        Self {
            log: SessionLog {
                id: started.format("%Y%m%d-%H%M%S").to_string(),
                started: started.timestamp(),
                ended: started.timestamp(),
                keep_decisions: 0,
                delete_decisions: 0,
                events: Vec::new(),
            },
        }
    }

    pub fn record(&mut self, kind: EventKind) {
        self.record_many(std::iter::once(kind));
    }

    pub fn record_many(&mut self, kinds: impl IntoIterator<Item = EventKind>) {
        let time = now();
        self.log
            .events
            .extend(kinds.into_iter().map(|kind| Event { time, kind }));
        self.save();
    }

    pub fn decision(&mut self, delete: bool, count: usize) {
        if count == 0 {
            return;
        }
        if delete {
            self.log.delete_decisions += count;
        } else {
            self.log.keep_decisions += count;
        }
        self.save();
    }

    fn save(&mut self) {
        self.log.ended = now();
        let Some(path) = session_file(&self.log.id) else {
            return;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.log) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Todas las sesiones guardadas, de la más reciente a la más antigua.
pub fn load_sessions() -> Vec<SessionLog> {
    let Some(dir) = sessions_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<SessionLog> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|content| serde_json::from_str(&content).ok())
        .collect();
    sessions.sort_by(|a, b| b.started.cmp(&a.started));
    sessions
}

/// `2026-11-14 15:30` en hora local.
pub fn format_time(secs: i64) -> String {
    match Local.timestamp_opt(secs, 0).single() {
        Some(time) => time.format("%Y-%m-%d %H:%M").to_string(),
        None => "-".to_string(),
    }
}

pub fn format_time_with_seconds(secs: i64) -> String {
    match Local.timestamp_opt(secs, 0).single() {
        Some(time) => time.format("%H:%M:%S").to_string(),
        None => "-".to_string(),
    }
}

/// Fila de un evento para la tabla y el CSV: (acción, archivo, destino/detalle, tamaño).
pub fn describe(event: &Event) -> (&'static str, String, String, Option<u64>) {
    match &event.kind {
        EventKind::Scan { folder, filter, files, duplicates } => (
            "Scan",
            filesystem::display_path(folder),
            format!("{filter}: {files} files, {duplicates} duplicates"),
            None,
        ),
        EventKind::Moved { path, dest, size } => (
            "Moved",
            filesystem::display_path(path),
            filesystem::display_path(dest),
            Some(*size),
        ),
        EventKind::Trashed { path, size, origin } => (
            "Trashed",
            filesystem::display_path(path),
            origin.clone(),
            Some(*size),
        ),
        EventKind::Skipped { path, reason, origin } => (
            "Skipped",
            filesystem::display_path(path),
            format!("{origin}: {reason}"),
            None,
        ),
        EventKind::Failed { path, reason, origin } => (
            "Failed",
            filesystem::display_path(path),
            format!("{origin}: {reason}"),
            None,
        ),
    }
}

/// Exporta una sesión a CSV: `time,action,path,destination_or_detail,size_bytes`.
/// Las rutas se escriben completas (no con `~`).
pub fn export_csv(log: &SessionLog, path: &Path) -> std::io::Result<()> {
    let mut out = String::from("time,action,path,destination_or_detail,size_bytes\n");
    for event in &log.events {
        let time = DateTime::from_timestamp(event.time, 0)
            .map(|t| t.with_timezone(&Local).to_rfc3339())
            .unwrap_or_default();
        let (action, file, detail, size) = match &event.kind {
            EventKind::Scan { folder, filter, files, duplicates } => (
                "scan",
                folder.display().to_string(),
                format!("{filter}: {files} files, {duplicates} duplicates"),
                None,
            ),
            EventKind::Moved { path, dest, size } => {
                ("moved", path.display().to_string(), dest.display().to_string(), Some(*size))
            }
            EventKind::Trashed { path, size, origin } => {
                ("trashed", path.display().to_string(), origin.clone(), Some(*size))
            }
            EventKind::Skipped { path, reason, origin } => {
                ("skipped", path.display().to_string(), format!("{origin}: {reason}"), None)
            }
            EventKind::Failed { path, reason, origin } => {
                ("failed", path.display().to_string(), format!("{origin}: {reason}"), None)
            }
        };
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_field(&time),
            action,
            csv_field(&file),
            csv_field(&detail),
            size.map(|s| s.to_string()).unwrap_or_default(),
        ));
    }
    std::fs::write(path, out)
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn session_round_trips_through_json() {
        let log = SessionLog {
            id: "20261114-153012".to_string(),
            started: 1,
            ended: 2,
            keep_decisions: 3,
            delete_decisions: 4,
            events: vec![
                Event {
                    time: 1,
                    kind: EventKind::Moved {
                        path: "/in/a.jpg".into(),
                        dest: "/out/image/a.jpg".into(),
                        size: 10,
                    },
                },
                Event {
                    time: 2,
                    kind: EventKind::Trashed {
                        path: "/in/b.jpg".into(),
                        size: 5,
                        origin: "Summary".into(),
                    },
                },
            ],
        };
        let json = serde_json::to_string(&log).unwrap();
        let back: SessionLog = serde_json::from_str(&json).unwrap();
        let totals = back.totals();
        assert_eq!((totals.moved, totals.moved_bytes), (1, 10));
        assert_eq!((totals.trashed, totals.trashed_bytes), (1, 5));
        assert_eq!(back.delete_decisions, 4);
    }
}
