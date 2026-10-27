//! What survives quitting and crashing: the last session (open documents,
//! folder and window) and unsaved changes written aside while editing.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    document::{self, LineEnding},
    settings::config_dir,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Session {
    pub(crate) documents: Vec<SessionDocument>,
    /// Index of the document that was showing.
    pub(crate) active: usize,
    pub(crate) window: Option<WindowPlacement>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SessionDocument {
    pub(crate) path: Option<PathBuf>,
    /// Caret position, a byte offset.
    pub(crate) cursor: usize,
    /// Recovery file holding unsaved changes, if there were any.
    pub(crate) recovery: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowPlacement {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) maximized: bool,
}

impl Session {
    /// The last session, or `None` on first run.
    pub(crate) fn load() -> Option<Self> {
        Self::load_from(&session_path()?)
    }

    pub(crate) fn load_from(path: &Path) -> Option<Self> {
        let json = std::fs::read_to_string(path).ok()?;
        Some(serde_json::from_str(&json).unwrap_or_default())
    }

    pub(crate) fn save(&self) {
        if let Some(path) = session_path() {
            self.save_to(&path);
        }
    }

    pub(crate) fn save_to(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn session_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("session.json"))
}

/// Unsaved changes to one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Recovery {
    /// Where the document is saved, if it ever was.
    pub(crate) path: Option<PathBuf>,
    pub(crate) text: String,
    pub(crate) crlf: bool,
}

fn recovery_dir() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("recovery"))
}

fn recovery_path_in(dir: &Path, id: &str) -> Option<PathBuf> {
    // Ids are generated here; refuse anything that could leave the folder.
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-') {
        return None;
    }
    Some(dir.join(format!("{id}.json")))
}

fn recovery_path(id: &str) -> Option<PathBuf> {
    recovery_dir().and_then(|dir| recovery_path_in(&dir, id))
}

/// A new, unique recovery id.
pub(crate) fn new_recovery_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "{nanos:x}-{:x}-{:x}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn write_recovery_to(path: &Path, recovery: &Recovery) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string(recovery).map_err(std::io::Error::other)?;
    document::save(path, &json, LineEnding::Lf)
}

pub(crate) fn write_recovery(id: &str, recovery: &Recovery) -> std::io::Result<()> {
    let path = recovery_path(id).ok_or_else(|| std::io::Error::other("no config folder"))?;
    write_recovery_to(&path, recovery)
}

pub(crate) fn read_recovery_from(path: &Path) -> Option<Recovery> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

pub(crate) fn read_recovery(id: &str) -> Option<Recovery> {
    read_recovery_from(&recovery_path(id)?)
}

pub(crate) fn remove_recovery(id: &str) {
    if let Some(path) = recovery_path(id) {
        let _ = std::fs::remove_file(path);
    }
}

pub(crate) fn orphaned_recoveries_in(
    dir: &Path,
    known: &HashSet<String>,
) -> Vec<(String, Recovery)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, Recovery)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_stem()?.to_str()?.to_string();
            (path.extension()? == "json" && !known.contains(&id))
                .then(|| read_recovery_from(&path).map(|recovery| (id, recovery)))
                .flatten()
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Recovery files no session document refers to: left by a crash before
/// the session was written, or by another window.
pub(crate) fn orphaned_recoveries(known: &HashSet<String>) -> Vec<(String, Recovery)> {
    let Some(dir) = recovery_dir() else {
        return Vec::new();
    };
    orphaned_recoveries_in(&dir, known)
}

/// Modification time and length of a file, to notice changes made by other
/// programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiskStamp {
    pub(crate) modified: Option<SystemTime>,
    pub(crate) len: u64,
}

impl DiskStamp {
    pub(crate) fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_sessions_and_recovery() {
        let dir = std::env::temp_dir().join(format!("downmark-session-{}", new_recovery_id()));
        let session_file = dir.join("session.json");
        let recovery_dir = dir.join("recovery");

        assert_eq!(Session::load_from(&session_file), None);
        let session = Session {
            documents: vec![SessionDocument {
                path: Some(PathBuf::from("/tmp/a.md")),
                cursor: 12,
                recovery: None,
            }],
            active: 0,
            window: Some(WindowPlacement {
                x: 10.,
                y: 20.,
                width: 800.,
                height: 600.,
                maximized: false,
            }),
        };
        session.save_to(&session_file);
        assert_eq!(Session::load_from(&session_file), Some(session));

        let id = new_recovery_id();
        assert_ne!(id, new_recovery_id());
        let recovery = Recovery {
            path: None,
            text: "unsaved".into(),
            crlf: false,
        };
        let recovery_file = recovery_path_in(&recovery_dir, &id).unwrap();
        write_recovery_to(&recovery_file, &recovery).unwrap();
        assert_eq!(read_recovery_from(&recovery_file), Some(recovery.clone()));
        assert_eq!(
            orphaned_recoveries_in(&recovery_dir, &HashSet::new()),
            vec![(id.clone(), recovery)]
        );
        assert_eq!(
            orphaned_recoveries_in(&recovery_dir, &HashSet::from([id])),
            vec![]
        );
        let _ = std::fs::remove_file(&recovery_file);
        assert_eq!(read_recovery_from(&recovery_file), None);
        assert!(recovery_path_in(&recovery_dir, "../escape").is_none());

        let _ = std::fs::remove_dir_all(dir);
    }
}
