//! User preferences persisted between sessions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const RECENT_LIMIT: usize = 10;
pub(crate) const DEFAULT_FONT_SIZE: f32 = 16.;
pub(crate) const MIN_FONT_SIZE: f32 = 12.;
pub(crate) const MAX_FONT_SIZE: f32 = 32.;

/// Where a document opened from a link in the preview goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LinkPlacement {
    /// A tab of its own, keeping the linking document open.
    #[default]
    NewTab,
    /// In place of the linking document, like a browser.
    SameTab,
}

/// Light or dark appearance, or follow the system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// Which panes the workspace shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Layout {
    Editor,
    #[default]
    Split,
    Preview,
}

impl Layout {
    pub(crate) fn shows_editor(self) -> bool {
        self != Layout::Preview
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings struct stores multiple independent user preference flags"
)]
pub(crate) struct Settings {
    pub(crate) appearance: Appearance,
    /// Theme used in light appearance; `None` is the built-in default.
    pub(crate) light_theme: Option<String>,
    /// Theme used in dark appearance; `None` is the built-in default.
    pub(crate) dark_theme: Option<String>,
    /// Base interface size in pixels; everything scales from it.
    pub(crate) font_size: f32,
    /// Preferred font family for document reading and interface; `None` is the system default.
    pub(crate) font_family: Option<String>,
    /// Preferred monospace font family for editor and code blocks; `None` is the system default.
    pub(crate) mono_font_family: Option<String>,
    pub(crate) layout: Layout,
    pub(crate) soft_wrap: bool,
    pub(crate) line_numbers: bool,
    /// Keep the preview scrolled to the part of the document being edited.
    pub(crate) scroll_sync: bool,
    pub(crate) recent_files: Vec<PathBuf>,
    /// Folders tracked for discovering Markdown documents on the new tab page.
    pub(crate) tracked_folders: Vec<PathBuf>,

    /// Open documents in tabs instead of replacing the current one.
    pub(crate) tabs: bool,
    /// Reopen the documents, folder and window of the last session.
    pub(crate) restore_session: bool,
    /// Folder, next to the document, that pasted and dropped images are
    /// saved into.
    pub(crate) image_folder: String,
    /// Where documents opened from links go.
    pub(crate) open_links_in: LinkPlacement,
    /// Ask before opening a web link in the browser.
    pub(crate) confirm_web_links: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            light_theme: None,
            dark_theme: None,
            font_size: DEFAULT_FONT_SIZE,
            font_family: None,
            mono_font_family: None,
            layout: Layout::Preview,
            soft_wrap: true,
            line_numbers: true,
            scroll_sync: true,
            recent_files: Vec::new(),
            tracked_folders: Vec::new(),
            tabs: true,
            restore_session: true,
            image_folder: "assets".to_string(),
            open_links_in: LinkPlacement::NewTab,
            confirm_web_links: true,
        }
    }
}

impl Settings {
    /// Load settings, falling back to defaults when the file is missing or
    /// unreadable. Out-of-range values are clamped.
    pub(crate) fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let mut settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|json| serde_json::from_str::<Settings>(&json).ok())
            .unwrap_or_default();
        settings.font_size = clamp_font_size(settings.font_size);
        settings.recent_files.truncate(RECENT_LIMIT);
        settings.tracked_folders.retain(|dir| dir.is_dir());
        let folder = settings.image_folder.trim().trim_matches(['/', '\\']);
        settings.image_folder = if folder.is_empty() || folder.contains("..") {
            "assets".to_string()
        } else {
            folder.to_string()
        };
        settings
    }

    /// Persist settings. Failures are not fatal: preferences simply are not
    /// remembered.
    pub(crate) fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Move `path` to the top of the recent files list.
    pub(crate) fn push_recent(&mut self, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.recent_files.retain(|recent| recent != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(RECENT_LIMIT);
    }
}

pub(crate) fn clamp_font_size(size: f32) -> f32 {
    if size.is_finite() {
        size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
    } else {
        DEFAULT_FONT_SIZE
    }
}

fn settings_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("settings.json"))
}

/// Where Downmark keeps its settings, session and recovered documents.
pub(crate) fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("DOWNMARK_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Downmark"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/Downmark"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|dir| dir.join("downmark"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let settings: Settings = serde_json::from_str(r#"{ "layout": "preview" }"#).unwrap();
        assert_eq!(settings.layout, Layout::Preview);
        assert!(settings.soft_wrap);
        assert_eq!(settings.font_size, DEFAULT_FONT_SIZE);
        assert_eq!(settings.font_family, None);
        assert_eq!(settings.mono_font_family, None);
        assert!(settings.tabs);
        assert!(settings.restore_session);
        assert_eq!(settings.open_links_in, LinkPlacement::NewTab);
        assert!(settings.confirm_web_links);
    }

    #[test]
    fn recent_files_are_unique_and_bounded() {
        let mut settings = Settings::default();
        for ix in 0..12 {
            settings.push_recent(Path::new(&format!("/tmp/{ix}.md")));
        }
        settings.push_recent(Path::new("/tmp/5.md"));
        assert_eq!(settings.recent_files.len(), RECENT_LIMIT);
        assert_eq!(settings.recent_files[0], PathBuf::from("/tmp/5.md"));
        assert_eq!(
            settings
                .recent_files
                .iter()
                .filter(|path| path.ends_with("5.md"))
                .count(),
            1
        );
    }

    #[test]
    fn font_size_is_clamped() {
        assert_eq!(MAX_FONT_SIZE, 32.0);
        assert_eq!(clamp_font_size(100.), 32.0);
        assert_eq!(clamp_font_size(30.), 30.0);
        assert_eq!(clamp_font_size(10.), MIN_FONT_SIZE);
        assert_eq!(clamp_font_size(f32::NAN), DEFAULT_FONT_SIZE);
    }

    #[test]
    fn font_family_round_trips() {
        let json = r#"{ "font_family": "Inter", "mono_font_family": "JetBrains Mono" }"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.font_family.as_deref(), Some("Inter"));
        assert_eq!(settings.mono_font_family.as_deref(), Some("JetBrains Mono"));
    }
}
