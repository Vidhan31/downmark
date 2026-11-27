//! User preferences persisted between sessions.

use std::path::{Path, PathBuf};

use gpui_kit::{App, Global};
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
    Split,
    #[default]
    Preview,
}

impl Layout {
    pub(crate) fn shows_editor(self) -> bool {
        self != Layout::Preview
    }

    pub(crate) fn shows_preview(self) -> bool {
        self != Layout::Editor
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
    /// Which panes the workspace shows. Not persisted so opening files
    /// always starts in preview mode.
    #[serde(skip)]
    pub(crate) layout: Layout,
    /// Distraction-free glance mode hiding chrome. Not persisted so normal launches start in standard view.
    #[serde(skip)]
    pub(crate) glance_mode: bool,
    pub(crate) soft_wrap: bool,
    pub(crate) line_numbers: bool,
    /// Keep the preview scrolled to the part of the document being edited.
    pub(crate) scroll_sync: bool,
    pub(crate) recent_files: Vec<PathBuf>,
    /// Folders tracked for discovering Markdown documents on the new tab page.
    pub(crate) tracked_folders: Vec<PathBuf>,
    /// Project folders chosen by the user for the AI workflow hub.
    #[serde(default)]
    pub(crate) project_folders: Vec<PathBuf>,
    /// Currently selected project filter in the UI (None means All Projects).
    #[serde(default)]
    pub(crate) selected_project_filter: Option<PathBuf>,
    /// Active project folder currently selected for the AI workflow hub (kept for backwards compatibility).
    #[serde(default)]
    pub(crate) active_project: Option<PathBuf>,
    /// Recently opened project folders for quick switching with minimal clicks.
    #[serde(default)]
    pub(crate) recent_projects: Vec<PathBuf>,

    /// Open documents in tabs instead of replacing the current one.
    pub(crate) tabs: bool,
    /// Open single external Markdown documents in distraction-free Glance Mode.
    pub(crate) glance_on_external: bool,
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
            glance_mode: false,
            soft_wrap: true,
            line_numbers: true,
            scroll_sync: false,
            recent_files: Vec::new(),
            tracked_folders: Vec::new(),
            project_folders: Vec::new(),
            selected_project_filter: None,
            active_project: None,
            recent_projects: Vec::new(),
            tabs: true,
            glance_on_external: true,
            restore_session: true,
            image_folder: "assets".to_string(),
            open_links_in: LinkPlacement::NewTab,
            confirm_web_links: true,
        }
    }
}

impl Settings {
    /// Whether persistent user settings match, ignoring runtime transient display state.
    pub(crate) fn persistent_eq(&self, other: &Self) -> bool {
        self.appearance == other.appearance
            && self.light_theme == other.light_theme
            && self.dark_theme == other.dark_theme
            && (self.font_size - other.font_size).abs() <= f32::EPSILON
            && self.font_family == other.font_family
            && self.mono_font_family == other.mono_font_family
            && self.soft_wrap == other.soft_wrap
            && self.line_numbers == other.line_numbers
            && self.scroll_sync == other.scroll_sync
            && self.recent_files == other.recent_files
            && self.tracked_folders == other.tracked_folders
            && self.project_folders == other.project_folders
            && self.selected_project_filter == other.selected_project_filter
            && self.active_project == other.active_project
            && self.recent_projects == other.recent_projects
            && self.tabs == other.tabs
            && self.glance_on_external == other.glance_on_external
            && self.restore_session == other.restore_session
            && self.image_folder == other.image_folder
            && self.open_links_in == other.open_links_in
            && self.confirm_web_links == other.confirm_web_links
    }
}

/// The user's preferences, shared by every window.
pub(crate) struct AppSettings(pub(crate) Settings);

impl Global for AppSettings {}

impl AppSettings {
    pub(crate) fn get(cx: &App) -> &Settings {
        &cx.global::<AppSettings>().0
    }

    /// Change the settings, persist them when needed, and re-apply appearance if changed.
    pub(crate) fn update(cx: &mut App, f: impl FnOnce(&mut Settings)) {
        let mut settings = Self::get(cx).clone();
        f(&mut settings);
        settings.font_size = clamp_font_size(settings.font_size);
        if &settings == Self::get(cx) {
            return;
        }
        let appearance_changed = {
            let current = Self::get(cx);
            current.appearance != settings.appearance
                || current.light_theme != settings.light_theme
                || current.dark_theme != settings.dark_theme
                || (current.font_size - settings.font_size).abs() > f32::EPSILON
                || current.font_family != settings.font_family
                || current.mono_font_family != settings.mono_font_family
        };
        let persistent_changed = !settings.persistent_eq(Self::get(cx));
        if persistent_changed {
            settings.save();
        }
        cx.set_global(AppSettings(settings));
        if appearance_changed {
            crate::themes::apply(None, cx);
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
        settings.layout = Layout::Preview;
        settings.glance_mode = false;
        settings.recent_files.truncate(RECENT_LIMIT);
        settings.prune_missing();
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
        if cfg!(test) {
            return;
        }
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

    /// Remove file and folder entries that are confirmed absent or of the wrong type.
    ///
    /// Returns `true` if any stored entry was removed or cleared.
    pub(crate) fn prune_missing(&mut self) -> bool {
        let prev_projects = self.project_folders.len();
        self.project_folders
            .retain(|dir| path_exists_as_dir_or_unconfirmed(dir));
        let projects_changed = self.project_folders.len() != prev_projects;
        self.prune_non_project_entries() || projects_changed
    }

    /// Remove missing non-project entries and reconcile `project_folders` against
    /// a freshly scanned snapshot's `valid_projects`.
    pub(crate) fn prune_missing_with_projects(&mut self, valid_projects: &[PathBuf]) -> bool {
        let prev_projects = self.project_folders.len();
        self.project_folders.retain(|dir| {
            let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            valid_projects.contains(&canonical)
        });
        let projects_changed = self.project_folders.len() != prev_projects;
        self.prune_non_project_entries() || projects_changed
    }

    fn prune_non_project_entries(&mut self) -> bool {
        let prev_recent = self.recent_files.len();
        let prev_tracked = self.tracked_folders.len();
        let prev_recent_projects = self.recent_projects.len();
        let prev_filter = self.selected_project_filter.clone();
        let prev_active = self.active_project.clone();

        self.recent_files
            .retain(|path| path_exists_as_file_or_unconfirmed(path));
        self.tracked_folders
            .retain(|dir| path_exists_as_dir_or_unconfirmed(dir));
        self.recent_projects
            .retain(|dir| path_exists_as_dir_or_unconfirmed(dir));
        if let Some(ref filter) = self.selected_project_filter
            && (!path_exists_as_dir_or_unconfirmed(filter)
                || !self.project_folders.contains(filter))
        {
            self.selected_project_filter = None;
        }
        if let Some(ref active) = self.active_project
            && !path_exists_as_dir_or_unconfirmed(active)
        {
            self.active_project = None;
        }

        self.recent_files.len() != prev_recent
            || self.tracked_folders.len() != prev_tracked
            || self.recent_projects.len() != prev_recent_projects
            || self.selected_project_filter != prev_filter
            || self.active_project != prev_active
    }
}

/// Returns `true` if `path` is a regular file or if metadata lookup failed for a
/// non-absence reason (such as permission denied or a transient I/O error).
pub(crate) fn path_exists_as_file_or_unconfirmed(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file(),
        Err(err) => !is_confirmed_absent(&err),
    }
}

/// Returns `true` if `path` is a directory or if metadata lookup failed for a
/// non-absence reason (such as permission denied or a transient I/O error).
pub(crate) fn path_exists_as_dir_or_unconfirmed(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_dir(),
        Err(err) => !is_confirmed_absent(&err),
    }
}

fn is_confirmed_absent(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
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
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.layout, Layout::Preview);
        assert_eq!(Layout::default(), Layout::Preview);
        assert!(!settings.scroll_sync);
        assert!(settings.soft_wrap);
        assert_eq!(settings.font_size, DEFAULT_FONT_SIZE);
        assert_eq!(settings.font_family, None);
        assert_eq!(settings.mono_font_family, None);
        assert!(settings.tabs);
        assert!(settings.glance_on_external);
        assert!(!settings.glance_mode);
        assert!(settings.restore_session);
        assert_eq!(settings.open_links_in, LinkPlacement::NewTab);
        assert!(settings.confirm_web_links);
    }

    #[test]
    fn layout_is_not_persisted_and_defaults_to_preview() {
        let settings: Settings = serde_json::from_str(r#"{ "layout": "split" }"#).unwrap();
        assert_eq!(settings.layout, Layout::Preview);
        let serialized = serde_json::to_string(&settings).unwrap();
        assert!(!serialized.contains("layout"));
    }

    #[test]
    fn persistent_eq_ignores_layout_and_glance_mode() {
        let mut a = Settings::default();
        let mut b = Settings::default();
        assert!(a.persistent_eq(&b));

        a.layout = Layout::Editor;
        a.glance_mode = true;
        assert!(a.persistent_eq(&b));

        b.font_size += 2.0;
        assert!(!a.persistent_eq(&b));
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

    #[test]
    fn prune_missing_removes_deleted_files_and_folders() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "downmark_settings_prune_{}_{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let proj = dir.join("proj");
        std::fs::create_dir_all(&proj).unwrap();

        let existing_file = proj.join("keep.md");
        std::fs::write(&existing_file, "# Keep\n").unwrap();
        let missing_file = proj.join("deleted.md");
        let missing_dir = dir.join("deleted_proj");

        let mut settings = Settings {
            recent_files: vec![existing_file.clone(), missing_file],
            tracked_folders: vec![proj.clone(), missing_dir.clone()],
            project_folders: vec![proj.clone(), missing_dir.clone()],
            recent_projects: vec![proj.clone(), missing_dir.clone()],
            selected_project_filter: Some(missing_dir.clone()),
            active_project: Some(missing_dir),
            ..Settings::default()
        };

        assert!(settings.prune_missing());
        assert_eq!(settings.recent_files, vec![existing_file]);
        assert_eq!(settings.tracked_folders, vec![proj.clone()]);
        assert_eq!(settings.project_folders, vec![proj.clone()]);
        assert_eq!(settings.recent_projects, vec![proj]);
        assert_eq!(settings.selected_project_filter, None);
        assert_eq!(settings.active_project, None);
        assert!(!settings.prune_missing());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn prune_missing_preserves_entries_on_permission_denied() {
        use std::os::unix::fs::PermissionsExt as _;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "downmark_settings_perm_{}_{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let locked = dir.join("locked");
        let inner_dir = locked.join("proj");
        std::fs::create_dir_all(&inner_dir).unwrap();
        let inner_file = inner_dir.join("note.md");
        std::fs::write(&inner_file, "# Note\n").unwrap();

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let mut settings = Settings {
            recent_files: vec![inner_file.clone()],
            tracked_folders: vec![inner_dir.clone()],
            project_folders: vec![inner_dir.clone()],
            recent_projects: vec![inner_dir.clone()],
            selected_project_filter: Some(inner_dir.clone()),
            active_project: Some(inner_dir.clone()),
            ..Settings::default()
        };

        let changed = settings.prune_missing();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(!changed);
        assert_eq!(settings.recent_files, vec![inner_file]);
        assert_eq!(settings.project_folders, vec![inner_dir.clone()]);
        assert_eq!(settings.selected_project_filter, Some(inner_dir.clone()));
        assert_eq!(settings.active_project, Some(inner_dir));
    }
}
