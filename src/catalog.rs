use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Represents a discovered Markdown file inside a tracked folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScannedMarkdownFile {
    pub(crate) relative_path: String,
    pub(crate) full_path: PathBuf,
    pub(crate) file_name: String,
    pub(crate) file_size: u64,
    pub(crate) modified: Option<SystemTime>,
}

impl ScannedMarkdownFile {
    #[expect(
        clippy::cast_precision_loss,
        clippy::as_conversions,
        reason = "File sizes fit safely within f64 precision for human-readable display"
    )]
    pub(crate) fn formatted_size(&self) -> String {
        if self.file_size < 1024 {
            format!("{} B", self.file_size)
        } else if self.file_size < 1024 * 1024 {
            format!("{:.1} KB", self.file_size as f64 / 1024.0)
        } else {
            format!("{:.1} MB", self.file_size as f64 / (1024.0 * 1024.0))
        }
    }

    pub(crate) fn formatted_modified(&self) -> String {
        let Some(modified) = self.modified else {
            return String::new();
        };

        let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
            return "Just now".to_string();
        };

        let secs = elapsed.as_secs();
        if secs < 60 {
            "Just now".to_string()
        } else if secs < 3600 {
            let mins = secs / 60;
            format!("{mins}m ago")
        } else if secs < 86400 {
            let hours = secs / 3600;
            format!("{hours}h ago")
        } else if secs < 86400 * 7 {
            let days = secs / 86400;
            format!("{days}d ago")
        } else if secs < 86400 * 30 {
            let weeks = secs / (86400 * 7);
            format!("{weeks}w ago")
        } else if secs < 86400 * 365 {
            let months = secs / (86400 * 30);
            format!("{months}mo ago")
        } else {
            let years = secs / (86400 * 365);
            format!("{years}y ago")
        }
    }
}

/// Represents a tracked folder and all Markdown files found inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScannedFolderGroup {
    pub(crate) folder_path: PathBuf,
    pub(crate) folder_name: String,
    pub(crate) files: Vec<ScannedMarkdownFile>,
}

/// Represents a flat entry of a Markdown file associated with its source folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceFileEntry {
    pub(crate) folder_path: PathBuf,
    pub(crate) folder_name: String,
    pub(crate) file: ScannedMarkdownFile,
}

impl WorkspaceFileEntry {
    pub(crate) fn formatted_size(&self) -> String {
        self.file.formatted_size()
    }

    pub(crate) fn formatted_modified(&self) -> String {
        self.file.formatted_modified()
    }
}

/// Manages tracked user directories, persistence, and file discovery.
#[derive(Clone, Debug, Default)]
pub(crate) struct FolderCatalog {
    folders: Vec<PathBuf>,
    groups: Vec<ScannedFolderGroup>,
    sorted_modified_cache: Vec<WorkspaceFileEntry>,
    all_files_cache: Vec<WorkspaceFileEntry>,
}

/// Internal work item for directory queue traversal.
struct DirScanTask {
    dir_path: PathBuf,
    rel_prefix: String,
    depth: usize,
}

/// Fast check for noisy or hidden directories that should be skipped without allocating.
#[inline]
pub(crate) fn is_noise_or_hidden_dir(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    matches!(
        name,
        "node_modules"
            | "target"
            | "build"
            | "dist"
            | "vendor"
            | "__pycache__"
            | "out"
            | ".cache"
            | ".vscode"
            | ".idea"
    )
}

/// Fast check to determine whether a file name represents a Markdown document.
#[inline]
pub(crate) fn is_markdown_file_name(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }
    let bytes = name.as_bytes();
    let len = bytes.len();
    if len >= 3
        && bytes[len - 3] == b'.'
        && bytes[len - 2].eq_ignore_ascii_case(&b'm')
        && bytes[len - 1].eq_ignore_ascii_case(&b'd')
    {
        return true;
    }
    if len >= 9 && bytes[len - 9] == b'.' {
        let ext = &name[len - 8..];
        return ext.eq_ignore_ascii_case("markdown");
    }
    false
}

#[inline]
fn cmp_ignore_ascii_case(a: &str, b: &str) -> std::cmp::Ordering {
    a.chars()
        .map(|c| c.to_ascii_lowercase())
        .cmp(b.chars().map(|c| c.to_ascii_lowercase()))
}

impl FolderCatalog {
    pub(crate) fn new(folders: Vec<PathBuf>) -> Self {
        let mut catalog = Self {
            folders,
            groups: Vec::new(),
            sorted_modified_cache: Vec::new(),
            all_files_cache: Vec::new(),
        };
        catalog.rescan();
        catalog
    }

    pub(crate) fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    pub(crate) fn groups(&self) -> &[ScannedFolderGroup] {
        &self.groups
    }

    pub(crate) fn total_files(&self) -> usize {
        self.all_files_cache.len()
    }

    pub(crate) fn sorted_modified_files(&self) -> &[WorkspaceFileEntry] {
        &self.sorted_modified_cache
    }

    pub(crate) fn all_files(&self) -> &[WorkspaceFileEntry] {
        &self.all_files_cache
    }

    /// Adds a directory to tracked folders if it exists and is not already tracked.
    pub(crate) fn add_folder(&mut self, path: PathBuf) -> bool {
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            Err(_) => path,
        };

        if !canonical.is_dir() {
            return false;
        }

        if self.folders.iter().any(|f| f == &canonical) {
            return false;
        }

        self.folders.push(canonical.clone());
        let group = Self::scan_folder(&canonical);
        self.groups.push(group);
        Self::sort_groups(&mut self.groups);
        self.rebuild_caches();
        true
    }

    /// Removes a folder from tracked folders.
    pub(crate) fn remove_folder(&mut self, path: &Path) -> bool {
        let initial_len = self.folders.len();
        self.folders.retain(|f| f != path);
        if self.folders.len() < initial_len {
            self.groups.retain(|g| g.folder_path != path);
            self.rebuild_caches();
            true
        } else {
            false
        }
    }

    /// Rescans all tracked folders on disk for Markdown files.
    pub(crate) fn rescan(&mut self) {
        if self.folders.is_empty() {
            self.groups = Vec::new();
            self.rebuild_caches();
            return;
        }

        if self.folders.len() == 1 {
            if self.folders[0].is_dir() {
                self.groups = vec![Self::scan_folder(&self.folders[0])];
            } else {
                self.groups = Vec::new();
            }
            self.rebuild_caches();
            return;
        }

        let valid_folders: Vec<&Path> = self
            .folders
            .iter()
            .map(PathBuf::as_path)
            .filter(|f| f.is_dir())
            .collect();
        let mut results: Vec<Option<ScannedFolderGroup>> = vec![None; valid_folders.len()];

        std::thread::scope(|s| {
            let mut handles = Vec::with_capacity(valid_folders.len());
            for (i, folder) in valid_folders.iter().enumerate() {
                handles.push(s.spawn(move || (i, Self::scan_folder(folder))));
            }

            for handle in handles {
                if let Ok((i, group)) = handle.join() {
                    results[i] = Some(group);
                }
            }
        });

        self.groups = results.into_iter().flatten().collect();
        Self::sort_groups(&mut self.groups);
        self.rebuild_caches();
    }

    /// Scans a single directory iteratively for Markdown files using a worklist queue.
    pub(crate) fn scan_folder(folder: &Path) -> ScannedFolderGroup {
        const MAX_DEPTH: usize = 8;

        let folder_name = folder
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Folder")
            .to_string();

        let mut files = Vec::new();
        let mut queue = Vec::new();

        queue.push(DirScanTask {
            dir_path: folder.to_path_buf(),
            rel_prefix: String::new(),
            depth: 0,
        });

        while let Some(task) = queue.pop() {
            let Ok(entries) = fs::read_dir(&task.dir_path) else {
                continue;
            };

            for entry in entries.flatten() {
                let Ok(file_name) = entry.file_name().into_string() else {
                    continue;
                };

                let Ok(file_type) = entry.file_type() else {
                    continue;
                };

                if file_type.is_dir() {
                    if task.depth < MAX_DEPTH && !is_noise_or_hidden_dir(&file_name) {
                        let next_dir = task.dir_path.join(&file_name);
                        let next_prefix = if task.rel_prefix.is_empty() {
                            format!("{file_name}/")
                        } else {
                            format!("{}{file_name}/", task.rel_prefix)
                        };
                        queue.push(DirScanTask {
                            dir_path: next_dir,
                            rel_prefix: next_prefix,
                            depth: task.depth + 1,
                        });
                    }
                } else if (file_type.is_file() || file_type.is_symlink())
                    && is_markdown_file_name(&file_name)
                {
                    let full_path = task.dir_path.join(&file_name);
                    let relative_path = if task.rel_prefix.is_empty() {
                        file_name.clone()
                    } else {
                        format!("{}{}", task.rel_prefix, file_name)
                    };
                    let (file_size, modified) = match entry.metadata() {
                        Ok(m) => (m.len(), m.modified().ok()),
                        Err(_) => (0, None),
                    };

                    files.push(ScannedMarkdownFile {
                        relative_path,
                        full_path,
                        file_name,
                        file_size,
                        modified,
                    });
                }
            }
        }

        files.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| cmp_ignore_ascii_case(&a.relative_path, &b.relative_path))
        });

        ScannedFolderGroup {
            folder_path: folder.to_path_buf(),
            folder_name,
            files,
        }
    }

    /// Sorts folder groups by the most recently modified file in each group (descending).
    pub(crate) fn sort_groups(groups: &mut [ScannedFolderGroup]) {
        groups.sort_by(|a, b| {
            let a_mod = a.files.first().and_then(|f| f.modified);
            let b_mod = b.files.first().and_then(|f| f.modified);
            b_mod
                .cmp(&a_mod)
                .then_with(|| cmp_ignore_ascii_case(&a.folder_name, &b.folder_name))
        });
    }

    /// Rebuilds cached entries for fast UI rendering.
    pub(crate) fn rebuild_caches(&mut self) {
        let mut entries = Vec::new();
        for group in &self.groups {
            for file in &group.files {
                entries.push(WorkspaceFileEntry {
                    folder_path: group.folder_path.clone(),
                    folder_name: group.folder_name.clone(),
                    file: file.clone(),
                });
            }
        }

        let mut by_modified = entries.clone();
        by_modified.sort_by(|a, b| {
            b.file
                .modified
                .cmp(&a.file.modified)
                .then_with(|| cmp_ignore_ascii_case(&a.file.relative_path, &b.file.relative_path))
        });
        self.sorted_modified_cache = by_modified;

        entries.sort_by(|a, b| cmp_ignore_ascii_case(&a.file.relative_path, &b.file.relative_path));
        self.all_files_cache = entries;
    }

    /// Resolves recent accessed file paths (e.g. from app settings) into `WorkspaceFileEntry` list.
    pub(crate) fn get_recent_accessed_entries(
        &self,
        recent_paths: &[PathBuf],
    ) -> Vec<WorkspaceFileEntry> {
        let mut entries = Vec::new();
        for path in recent_paths {
            if !path.exists() || !path.is_file() {
                continue;
            }

            let mut found = None;
            for group in &self.groups {
                if let Some(file) = group.files.iter().find(|f| &f.full_path == path) {
                    found = Some(WorkspaceFileEntry {
                        folder_path: group.folder_path.clone(),
                        folder_name: group.folder_name.clone(),
                        file: file.clone(),
                    });
                    break;
                }
            }

            if let Some(entry) = found {
                entries.push(entry);
            } else {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("document.md")
                    .to_string();
                let parent_dir = path.parent().unwrap_or(path).to_path_buf();
                let folder_name = parent_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Document")
                    .to_string();
                let (file_size, modified) = match fs::metadata(path) {
                    Ok(m) => (m.len(), m.modified().ok()),
                    Err(_) => (0, None),
                };

                entries.push(WorkspaceFileEntry {
                    folder_path: parent_dir,
                    folder_name,
                    file: ScannedMarkdownFile {
                        relative_path: file_name.clone(),
                        full_path: path.clone(),
                        file_name,
                        file_size,
                        modified,
                    },
                });
            }
        }
        entries
    }

    /// Filters all files across folders by search query and optional folder filter.
    pub(crate) fn filter_files(
        &self,
        query: &str,
        selected_folder: Option<&Path>,
    ) -> (Vec<WorkspaceFileEntry>, usize) {
        let trimmed = query.trim().to_lowercase();
        if trimmed.is_empty() && selected_folder.is_none() {
            let all = self.all_files().to_vec();
            let count = all.len();
            return (all, count);
        }

        let filtered: Vec<WorkspaceFileEntry> = self
            .all_files_cache
            .iter()
            .filter(|entry| {
                if selected_folder.is_some_and(|target_dir| entry.folder_path != target_dir) {
                    return false;
                }
                if trimmed.is_empty() {
                    true
                } else {
                    entry.file.file_name.to_lowercase().contains(&trimmed)
                        || entry.file.relative_path.to_lowercase().contains(&trimmed)
                        || entry.folder_name.to_lowercase().contains(&trimmed)
                }
            })
            .cloned()
            .collect();

        let count = filtered.len();
        (filtered, count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn test_formatted_size() {
        let f1 = ScannedMarkdownFile {
            relative_path: "test.md".into(),
            full_path: PathBuf::from("/test.md"),
            file_name: "test.md".into(),
            file_size: 512,
            modified: None,
        };
        assert_eq!(f1.formatted_size(), "512 B");

        let f2 = ScannedMarkdownFile {
            relative_path: "test.md".into(),
            full_path: PathBuf::from("/test.md"),
            file_name: "test.md".into(),
            file_size: 2048,
            modified: None,
        };
        assert_eq!(f2.formatted_size(), "2.0 KB");

        let f3 = ScannedMarkdownFile {
            relative_path: "test.md".into(),
            full_path: PathBuf::from("/test.md"),
            file_name: "test.md".into(),
            file_size: 3 * 1024 * 1024,
            modified: None,
        };
        assert_eq!(f3.formatted_size(), "3.0 MB");
    }

    #[test]
    fn test_is_markdown_file_name() {
        assert!(is_markdown_file_name("README.md"));
        assert!(is_markdown_file_name("DOCUMENT.MD"));
        assert!(is_markdown_file_name("notes.markdown"));
        assert!(is_markdown_file_name("ARTICLE.MARKDOWN"));

        assert!(!is_markdown_file_name(".hidden.md"));
        assert!(!is_markdown_file_name(".git"));
        assert!(!is_markdown_file_name("image.png"));
        assert!(!is_markdown_file_name("script.js"));
        assert!(!is_markdown_file_name("md"));
        assert!(!is_markdown_file_name(".md"));
    }

    #[test]
    fn test_is_noise_or_hidden_dir() {
        assert!(is_noise_or_hidden_dir(".git"));
        assert!(is_noise_or_hidden_dir(".cache"));
        assert!(is_noise_or_hidden_dir(".vscode"));
        assert!(is_noise_or_hidden_dir("node_modules"));
        assert!(is_noise_or_hidden_dir("target"));
        assert!(is_noise_or_hidden_dir("dist"));
        assert!(is_noise_or_hidden_dir("build"));

        assert!(!is_noise_or_hidden_dir("src"));
        assert!(!is_noise_or_hidden_dir("docs"));
    }

    #[test]
    fn test_recursive_scan_and_filter() {
        let temp_dir =
            std::env::temp_dir().join(format!("downmark_test_scan_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(temp_dir.join("sub")).unwrap();
        fs::create_dir_all(temp_dir.join("node_modules")).unwrap();
        fs::create_dir_all(temp_dir.join(".git")).unwrap();

        File::create(temp_dir.join("root.md"))
            .unwrap()
            .write_all(b"# Root")
            .unwrap();
        File::create(temp_dir.join("sub").join("nested.markdown"))
            .unwrap()
            .write_all(b"# Nested")
            .unwrap();
        File::create(temp_dir.join("ignored.txt"))
            .unwrap()
            .write_all(b"text")
            .unwrap();
        File::create(temp_dir.join("node_modules").join("pkg.md"))
            .unwrap()
            .write_all(b"noise")
            .unwrap();
        File::create(temp_dir.join(".git").join("COMMIT_EDITMSG.md"))
            .unwrap()
            .write_all(b"hidden")
            .unwrap();

        let catalog = FolderCatalog::new(vec![temp_dir.clone()]);
        assert_eq!(catalog.total_files(), 2);

        let (matched, count) = catalog.filter_files("nested", None);
        assert_eq!(count, 1);
        assert_eq!(matched[0].file.file_name, "nested.markdown");

        let (_, count_none) = catalog.filter_files("nonexistent", None);
        assert_eq!(count_none, 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sort_by_modified_timestamp() {
        let now = SystemTime::now();
        let mut files = [
            ScannedMarkdownFile {
                relative_path: "old.md".into(),
                full_path: PathBuf::from("/old.md"),
                file_name: "old.md".into(),
                file_size: 100,
                modified: Some(now - Duration::from_secs(3600)),
            },
            ScannedMarkdownFile {
                relative_path: "newest.md".into(),
                full_path: PathBuf::from("/newest.md"),
                file_name: "newest.md".into(),
                file_size: 200,
                modified: Some(now),
            },
        ];

        files.sort_by_key(|b| std::cmp::Reverse(b.modified));
        assert_eq!(files[0].file_name, "newest.md");
        assert_eq!(files[1].file_name, "old.md");
    }

    #[test]
    fn test_formatted_modified() {
        let now = SystemTime::now();
        let f_recent = ScannedMarkdownFile {
            relative_path: "f.md".into(),
            full_path: PathBuf::from("/f.md"),
            file_name: "f.md".into(),
            file_size: 100,
            modified: Some(now - Duration::from_secs(10)),
        };
        assert_eq!(f_recent.formatted_modified(), "Just now");

        let f_mins = ScannedMarkdownFile {
            relative_path: "f.md".into(),
            full_path: PathBuf::from("/f.md"),
            file_name: "f.md".into(),
            file_size: 100,
            modified: Some(now - Duration::from_secs(120)),
        };
        assert_eq!(f_mins.formatted_modified(), "2m ago");

        let f_hours = ScannedMarkdownFile {
            relative_path: "f.md".into(),
            full_path: PathBuf::from("/f.md"),
            file_name: "f.md".into(),
            file_size: 100,
            modified: Some(now - Duration::from_secs(7200)),
        };
        assert_eq!(f_hours.formatted_modified(), "2h ago");

        let f_none = ScannedMarkdownFile {
            relative_path: "f.md".into(),
            full_path: PathBuf::from("/f.md"),
            file_name: "f.md".into(),
            file_size: 100,
            modified: None,
        };
        assert_eq!(f_none.formatted_modified(), "");
    }

    #[test]
    fn test_add_and_remove_folder() {
        let temp_dir =
            std::env::temp_dir().join(format!("downmark_test_add_rem_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        File::create(temp_dir.join("test.md"))
            .unwrap()
            .write_all(b"# Doc")
            .unwrap();

        let mut catalog = FolderCatalog::new(Vec::new());
        assert_eq!(catalog.folders().len(), 0);
        assert_eq!(catalog.total_files(), 0);

        let added = catalog.add_folder(temp_dir.clone());
        assert!(added);
        assert_eq!(catalog.folders().len(), 1);
        assert_eq!(catalog.total_files(), 1);

        assert!(!catalog.add_folder(temp_dir.clone()));

        let removed = catalog.remove_folder(&temp_dir.canonicalize().unwrap());
        assert!(removed);
        assert_eq!(catalog.folders().len(), 0);
        assert_eq!(catalog.total_files(), 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
