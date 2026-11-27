use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Represents a discovered Markdown file inside a tracked folder or skill.
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

/// Represents an AI agent instruction / rules file (e.g. AGENTS.md, CLAUDE.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstructionEntry {
    pub(crate) name: String,
    pub(crate) agent_source: String,
    pub(crate) file: ScannedMarkdownFile,
    pub(crate) is_global: bool,
    pub(crate) project_name: Option<String>,
    pub(crate) project_path: Option<PathBuf>,
}

/// Represents a discovered AI skill entity with its entrypoint and companion files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillEntry {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) dir_path: PathBuf,
    pub(crate) skill_file: ScannedMarkdownFile,
    pub(crate) companion_files: Vec<ScannedMarkdownFile>,
    pub(crate) agent_source: String,
    pub(crate) is_global: bool,
    pub(crate) project_name: Option<String>,
    pub(crate) project_path: Option<PathBuf>,
}

/// Represents a general project documentation Markdown file (README, specs, architecture, etc.).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectDocEntry {
    pub(crate) file: ScannedMarkdownFile,
    pub(crate) project_name: Option<String>,
    pub(crate) project_path: Option<PathBuf>,
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
    #[expect(dead_code, reason = "Formatting helper for workspace file entry")]
    pub(crate) fn formatted_size(&self) -> String {
        self.file.formatted_size()
    }

    #[expect(dead_code, reason = "Formatting helper for workspace file entry")]
    pub(crate) fn formatted_modified(&self) -> String {
        self.file.formatted_modified()
    }
}

/// Filtered view of the AI Agent Workflow Hub.
#[derive(Clone, Debug, Default)]
pub(crate) struct FilteredHubView {
    pub(crate) project_instructions: Vec<InstructionEntry>,
    pub(crate) project_skills: Vec<SkillEntry>,
    pub(crate) project_docs: Vec<ProjectDocEntry>,
    pub(crate) global_instructions: Vec<InstructionEntry>,
    pub(crate) global_skills: Vec<SkillEntry>,
    #[expect(dead_code, reason = "Cached match count for future badge rendering")]
    pub(crate) total_matches: usize,
}

/// Predefined standard AI agent configuration.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PredefinedAgentDef {
    pub(crate) name: &'static str,
    pub(crate) instruction_paths: &'static [&'static str],
    pub(crate) skill_paths: &'static [&'static str],
    pub(crate) fallback_skills: &'static [&'static str],
}

pub(crate) const PREDEFINED_AGENTS: &[PredefinedAgentDef] = &[
    PredefinedAgentDef {
        name: "Google Antigravity",
        instruction_paths: &[
            ".gemini/AGENTS.md",
            ".gemini/config/AGENTS.md",
            ".gemini/GEMINI.md",
        ],
        skill_paths: &[
            ".gemini/config/skills",
            ".gemini/skills",
            ".gemini/antigravity-cli/skills",
            ".gemini/antigravity/skills",
        ],
        fallback_skills: &[],
    },
    PredefinedAgentDef {
        name: "Claude Code",
        instruction_paths: &[".claude/CLAUDE.md", ".claude/rules/*.md"],
        skill_paths: &[".claude/skills"],
        fallback_skills: &[],
    },
    PredefinedAgentDef {
        name: "OpenAI Codex",
        instruction_paths: &[".codex/AGENTS.md", ".agents/AGENTS.md"],
        skill_paths: &[".agents/skills", ".codex/skills"],
        fallback_skills: &["/etc/codex/skills"],
    },
    PredefinedAgentDef {
        name: "Cursor",
        instruction_paths: &[],
        skill_paths: &[".cursor/skills"],
        fallback_skills: &[".agents/skills"],
    },
    PredefinedAgentDef {
        name: "OpenCode",
        instruction_paths: &[".config/opencode/AGENTS.md"],
        skill_paths: &[".config/opencode/skills"],
        fallback_skills: &[".agents/skills", ".claude/skills"],
    },
    PredefinedAgentDef {
        name: "Windsurf",
        instruction_paths: &[".codeium/windsurf/memories/global_rules.md"],
        skill_paths: &[".codeium/windsurf/skills"],
        fallback_skills: &[],
    },
    PredefinedAgentDef {
        name: "GitHub Copilot",
        instruction_paths: &[
            ".copilot/copilot-instructions.md",
            ".copilot/instructions/*.instructions.md",
        ],
        skill_paths: &[".copilot/skills"],
        fallback_skills: &[".agents/skills", ".claude/skills"],
    },
    PredefinedAgentDef {
        name: "Amp",
        instruction_paths: &[".config/amp/AGENTS.md", ".config/AGENTS.md"],
        skill_paths: &[".config/agents/skills", ".config/amp/skills"],
        fallback_skills: &[".agents/skills", ".claude/skills"],
    },
    PredefinedAgentDef {
        name: "Devin",
        instruction_paths: &[".config/devin/AGENTS.md", ".devin/global_rules.md"],
        skill_paths: &[".config/devin/skills"],
        fallback_skills: &[".agents/skills"],
    },
    PredefinedAgentDef {
        name: "Kiro",
        instruction_paths: &[".kiro/steering/AGENTS.md", ".kiro/steering/*.md"],
        skill_paths: &[".kiro/skills"],
        fallback_skills: &[],
    },
];

/// Helper to get user's home directory across Linux, macOS, and Windows.
pub(crate) fn user_home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// Internal work item for directory queue traversal.
struct DirScanTask {
    dir_path: PathBuf,
    rel_prefix: String,
    depth: usize,
}

/// Fast check for noisy compiler/build/VCS directories that should be skipped.
/// Note: Hidden directories like `.agents`, `.claude`, `.gemini`, `.cursor` are NOT skipped.
#[inline]
pub(crate) fn is_noise_or_hidden_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".cache"
            | ".vscode"
            | ".idea"
            | ".cargo"
            | ".rustup"
            | "node_modules"
            | "target"
            | "build"
            | "dist"
            | "vendor"
            | "__pycache__"
            | "out"
    )
}

pub(crate) use crate::document::is_markdown_file_name;

/// Dot-prefixed rule filenames tracked by both `scan_project` and the catalog watcher.
const SPECIAL_RULE_FILENAMES: &[&str] = &[".cursorrules", ".copilot-instructions.md"];

/// Project-relative skill container directories scanned and watched for skill packages.
pub(crate) const PROJECT_SKILL_CONTAINER_RELS: &[&str] = &[
    ".agents/skills",
    ".claude/skills",
    ".cursor/skills",
    ".gemini/skills",
    ".config/opencode/skills",
    "skills",
];

/// Returns `true` if `path` has a filename tracked by the Home hub catalog.
pub(crate) fn is_catalog_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| is_markdown_file_name(name) || SPECIAL_RULE_FILENAMES.contains(&name))
}

/// Global AI agent directories to watch, split into existing directories and
/// missing targets whose nearest existing ancestor should be watched instead.
#[derive(Debug, Default)]
pub(crate) struct GlobalWatchDirs {
    pub(crate) instruction_dirs: Vec<PathBuf>,
    pub(crate) skill_containers: Vec<PathBuf>,
    pub(crate) missing_targets: Vec<PathBuf>,
}

/// Resolves an agent path template relative to `home` (or as an absolute `/...` path).
fn resolve_agent_path(path_spec: &str, home: Option<&Path>) -> PathBuf {
    if let Some(stripped) = path_spec.strip_prefix('/') {
        PathBuf::from(format!("/{stripped}"))
    } else if let Some(h) = home {
        h.join(path_spec)
    } else {
        PathBuf::from(path_spec)
    }
}

/// Collects global AI agent directories to watch for changes.
pub(crate) fn global_watch_dirs() -> GlobalWatchDirs {
    let home = user_home_dir();
    let mut seen_dirs = HashSet::new();
    let mut seen_missing = HashSet::new();
    let mut result = GlobalWatchDirs::default();

    for agent in PREDEFINED_AGENTS {
        for &inst_path in agent.instruction_paths {
            let candidate = if inst_path.contains('*') {
                let rel_dir = inst_path.trim_end_matches("*.md").trim_end_matches('/');
                home.as_deref().map(|h| h.join(rel_dir))
            } else {
                let full = resolve_agent_path(inst_path, home.as_deref());
                full.parent()
                    .filter(|parent| home.as_deref() != Some(*parent))
                    .map(Path::to_path_buf)
            };
            if let Some(dir) = candidate {
                if dir.is_dir() {
                    if seen_dirs.insert(dir.clone()) {
                        result.instruction_dirs.push(dir);
                    }
                } else if seen_missing.insert(dir.clone()) {
                    result.missing_targets.push(dir);
                }
            }
        }

        for &skill_rel in agent.skill_paths.iter().chain(agent.fallback_skills) {
            let full = resolve_agent_path(skill_rel, home.as_deref());
            if full.is_dir() {
                if seen_dirs.insert(full.clone()) {
                    result.skill_containers.push(full);
                }
            } else if seen_missing.insert(full.clone()) {
                result.missing_targets.push(full);
            }
        }
    }

    result
}

#[inline]
fn cmp_ignore_ascii_case(a: &str, b: &str) -> std::cmp::Ordering {
    a.chars()
        .map(|c| c.to_ascii_lowercase())
        .cmp(b.chars().map(|c| c.to_ascii_lowercase()))
}

/// Parses name and description metadata from a `SKILL.md` file.
pub(crate) fn parse_skill_metadata(skill_file_path: &Path, default_name: &str) -> (String, String) {
    let mut name = default_name.to_string();
    let mut description = String::new();

    let Ok(content) = fs::read_to_string(skill_file_path) else {
        return (name, description);
    };

    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---")
        && let Some(end_idx) = rest.find("\n---")
    {
        let frontmatter = &rest[..end_idx];
        let mut current_key = "";
        let mut desc_lines = Vec::new();

        for line in frontmatter.lines() {
            let trimmed_line = line.trim();
            if let Some(val) = trimmed_line.strip_prefix("name:") {
                current_key = "name";
                let val = val.trim().trim_matches('"').trim_matches('\'');
                if !val.is_empty() {
                    name = val.to_string();
                }
            } else if let Some(val) = trimmed_line.strip_prefix("description:") {
                current_key = "description";
                let val = val.trim().trim_matches('"').trim_matches('\'');
                if val != ">-" && val != "|" && !val.is_empty() {
                    desc_lines.push(val.to_string());
                }
            } else if current_key == "description" {
                if trimmed_line.starts_with("---")
                    || (line.contains(':') && !line.starts_with(' ') && !line.starts_with('\t'))
                {
                    current_key = "";
                } else if !trimmed_line.is_empty() {
                    desc_lines.push(trimmed_line.to_string());
                }
            }
        }

        if !desc_lines.is_empty() {
            description = desc_lines.join(" ");
        }
    }

    if description.is_empty() {
        for line in content.lines() {
            let l = line.trim();
            if l.starts_with('#') {
                if name == default_name {
                    let heading_text = l.trim_start_matches('#').trim();
                    if !heading_text.is_empty() {
                        name = heading_text.to_string();
                    }
                }
            } else if !l.is_empty() && !l.starts_with("---") {
                description = l.to_string();
                break;
            }
        }
    }

    (name, description)
}

/// Scans a single skill directory for its `SKILL.md` and companion Markdown files.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "is_global flag distinguishes user-level from project-level skills"
)]
pub(crate) fn scan_skill_dir(
    skill_dir: &Path,
    agent_source: &str,
    is_global: bool,
    project_name: Option<String>,
    project_path: Option<PathBuf>,
) -> Option<SkillEntry> {
    if !skill_dir.is_dir() {
        return None;
    }
    let dir_name = skill_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("skill")
        .to_string();

    let mut skill_file = None;
    let mut companion_files = Vec::new();
    let mut queue = vec![(skill_dir.to_path_buf(), String::new(), 0)];

    while let Some((dir, rel_prefix, depth)) = queue.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let Ok(file_name) = entry.file_name().into_string() else {
                continue;
            };
            let Ok(ft) = entry.file_type() else {
                continue;
            };

            if ft.is_dir() {
                if depth < 4 && !is_noise_or_hidden_dir(&file_name) {
                    let next_dir = dir.join(&file_name);
                    let next_rel = if rel_prefix.is_empty() {
                        format!("{file_name}/")
                    } else {
                        format!("{rel_prefix}{file_name}/")
                    };
                    queue.push((next_dir, next_rel, depth + 1));
                }
            } else if ft.is_file() || ft.is_symlink() {
                let full_path = dir.join(&file_name);
                let meta = if ft.is_symlink() {
                    fs::metadata(&full_path)
                } else {
                    entry.metadata()
                };
                let Ok(meta) = meta else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let rel_path = format!("{rel_prefix}{file_name}");
                let (file_size, modified) = (meta.len(), meta.modified().ok());

                let scanned = ScannedMarkdownFile {
                    relative_path: rel_path,
                    full_path,
                    file_name: file_name.clone(),
                    file_size,
                    modified,
                };

                if (file_name.eq_ignore_ascii_case("SKILL.md")
                    || file_name.eq_ignore_ascii_case("skill.md"))
                    && rel_prefix.is_empty()
                {
                    skill_file = Some(scanned);
                } else if is_markdown_file_name(&file_name) {
                    companion_files.push(scanned);
                }
            }
        }
    }

    let skill_file = skill_file?;
    let (name, description) = parse_skill_metadata(&skill_file.full_path, &dir_name);
    companion_files.sort_by(|a, b| cmp_ignore_ascii_case(&a.relative_path, &b.relative_path));

    Some(SkillEntry {
        name,
        description,
        dir_path: skill_dir.to_path_buf(),
        skill_file,
        companion_files,
        agent_source: agent_source.to_string(),
        is_global,
        project_name,
        project_path,
    })
}

/// Scans a directory containing multiple skill subdirectories.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "is_global flag distinguishes user-level from project-level skills"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Takes project metadata for discovered skills"
)]
pub(crate) fn scan_skills_container(
    container: &Path,
    agent_source: &str,
    is_global: bool,
    seen_dirs: &mut HashSet<PathBuf>,
    project_name: Option<String>,
    project_path: Option<PathBuf>,
) -> Vec<SkillEntry> {
    let mut skills = Vec::new();
    let Ok(entries) = fs::read_dir(container) else {
        return skills;
    };

    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() || (ft.is_symlink() && entry.path().is_dir()) {
            let path = entry.path();
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen_dirs.insert(canonical)
                && let Some(skill) = scan_skill_dir(
                    &path,
                    agent_source,
                    is_global,
                    project_name.clone(),
                    project_path.clone(),
                )
            {
                skills.push(skill);
            }
        }
    }

    skills.sort_by(|a, b| cmp_ignore_ascii_case(&a.name, &b.name));
    skills
}

/// Scans a single instruction file.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "is_global flag distinguishes user-level from project-level instruction files"
)]
pub(crate) fn scan_instruction_file(
    file_path: &Path,
    agent_source: &str,
    is_global: bool,
    project_name: Option<String>,
    project_path: Option<PathBuf>,
) -> Option<InstructionEntry> {
    if !file_path.is_file() {
        return None;
    }

    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("AGENTS.md")
        .to_string();

    let (file_size, modified) = match fs::metadata(file_path) {
        Ok(m) => (m.len(), m.modified().ok()),
        Err(_) => (0, None),
    };

    Some(InstructionEntry {
        name: file_name.clone(),
        agent_source: agent_source.to_string(),
        file: ScannedMarkdownFile {
            relative_path: file_name.clone(),
            full_path: file_path.to_path_buf(),
            file_name,
            file_size,
            modified,
        },
        is_global,
        project_name,
        project_path,
    })
}

/// Background-scannable snapshot of all Home hub catalog state.
#[derive(Clone, Debug, Default)]
pub(crate) struct CatalogSnapshot {
    pub(crate) project_folders: Vec<PathBuf>,
    pub(crate) project_instructions: Vec<InstructionEntry>,
    pub(crate) project_skills: Vec<SkillEntry>,
    pub(crate) project_docs: Vec<ProjectDocEntry>,
    pub(crate) global_instructions: Vec<InstructionEntry>,
    pub(crate) global_skills: Vec<SkillEntry>,
    pub(crate) groups: Vec<ScannedFolderGroup>,
    pub(crate) sorted_modified_cache: Vec<WorkspaceFileEntry>,
    pub(crate) all_files_cache: Vec<WorkspaceFileEntry>,
}

/// Manages tracked user directories, AI Agent skills, rules, and Markdown discovery.
#[derive(Clone, Debug, Default)]
pub(crate) struct FolderCatalog {
    pub(crate) project_folders: Vec<PathBuf>,
    pub(crate) selected_project_filter: Option<PathBuf>,

    pub(crate) project_instructions: Vec<InstructionEntry>,
    pub(crate) project_skills: Vec<SkillEntry>,
    pub(crate) project_docs: Vec<ProjectDocEntry>,

    pub(crate) global_instructions: Vec<InstructionEntry>,
    pub(crate) global_skills: Vec<SkillEntry>,

    folders: Vec<PathBuf>,
    groups: Vec<ScannedFolderGroup>,
    sorted_modified_cache: Vec<WorkspaceFileEntry>,
    all_files_cache: Vec<WorkspaceFileEntry>,
}

impl FolderCatalog {
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Takes ownership of project folders"
    )]
    pub(crate) fn new(
        project_folders: Vec<PathBuf>,
        selected_project_filter: Option<PathBuf>,
    ) -> Self {
        let mut canonical_projects = Vec::new();
        for p in &project_folders {
            if p.is_dir() {
                let canonical = p.canonicalize().unwrap_or_else(|_| p.clone());
                if !canonical_projects.contains(&canonical) {
                    canonical_projects.push(canonical);
                }
            }
        }

        let filter = selected_project_filter.and_then(|f| {
            let canonical = f.canonicalize().unwrap_or_else(|_| f.clone());
            if canonical_projects.contains(&canonical) {
                Some(canonical)
            } else {
                None
            }
        });

        let folders = canonical_projects.clone();

        let mut catalog = Self {
            project_folders: canonical_projects,
            selected_project_filter: filter,
            project_instructions: Vec::new(),
            project_skills: Vec::new(),
            project_docs: Vec::new(),
            global_instructions: Vec::new(),
            global_skills: Vec::new(),
            folders,
            groups: Vec::new(),
            sorted_modified_cache: Vec::new(),
            all_files_cache: Vec::new(),
        };
        catalog.rescan();
        catalog
    }

    /// Convenience constructor for backwards compatibility and tests.
    #[expect(
        dead_code,
        reason = "Convenience constructor for tests and external callers"
    )]
    pub(crate) fn from_folders(folders: Vec<PathBuf>) -> Self {
        Self::new(folders, None)
    }

    pub(crate) fn project_folders(&self) -> &[PathBuf] {
        &self.project_folders
    }

    pub(crate) fn selected_project_filter(&self) -> Option<&PathBuf> {
        self.selected_project_filter.as_ref()
    }

    #[expect(dead_code, reason = "Legacy compatibility accessor")]
    pub(crate) fn active_project(&self) -> Option<&PathBuf> {
        self.selected_project_filter
            .as_ref()
            .or(self.project_folders.first())
    }

    #[expect(dead_code, reason = "Legacy compatibility accessor")]
    pub(crate) fn recent_projects(&self) -> &[PathBuf] {
        &self.project_folders
    }

    pub(crate) fn add_project_folder(&mut self, folder: PathBuf) {
        let canonical = match folder.canonicalize() {
            Ok(c) => c,
            Err(_) => folder,
        };
        if !canonical.is_dir() {
            return;
        }
        if !self.project_folders.contains(&canonical) {
            self.project_folders.push(canonical.clone());
        }
        if !self.folders.contains(&canonical) {
            self.folders.push(canonical.clone());
        }
        self.selected_project_filter = Some(canonical);
        self.rescan();
    }

    pub(crate) fn remove_project_folder(&mut self, path: &Path) {
        self.project_folders.retain(|p| p != path);
        self.folders.retain(|p| p != path);
        if self.selected_project_filter.as_deref() == Some(path) {
            self.selected_project_filter = None;
        }
        self.rescan();
    }

    pub(crate) fn set_selected_project_filter(&mut self, filter: Option<PathBuf>) {
        // Project folders are stored canonicalized; normalize the filter to match (e.g. Windows \\?\ / 8.3 paths).
        self.selected_project_filter = filter.map(|f| f.canonicalize().unwrap_or(f));
    }

    #[expect(dead_code, reason = "Legacy adapter for setting active project")]
    pub(crate) fn set_active_project(&mut self, project: Option<PathBuf>) {
        if let Some(p) = project {
            self.add_project_folder(p);
        } else {
            self.selected_project_filter = None;
        }
    }

    #[expect(dead_code, reason = "Legacy adapter for removing project")]
    pub(crate) fn remove_recent_project(&mut self, path: &Path) {
        self.remove_project_folder(path);
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Accessor for project instructions in tests")
    )]
    pub(crate) fn project_instructions(&self) -> &[InstructionEntry] {
        &self.project_instructions
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Accessor for project skills in tests")
    )]
    pub(crate) fn project_skills(&self) -> &[SkillEntry] {
        &self.project_skills
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Accessor for project docs in tests")
    )]
    pub(crate) fn project_docs(&self) -> &[ProjectDocEntry] {
        &self.project_docs
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Accessor for global instructions")
    )]
    pub(crate) fn global_instructions(&self) -> &[InstructionEntry] {
        &self.global_instructions
    }

    #[expect(dead_code, reason = "Accessor for global skills")]
    pub(crate) fn global_skills(&self) -> &[SkillEntry] {
        &self.global_skills
    }

    pub(crate) fn folders(&self) -> &[PathBuf] {
        &self.folders
    }

    #[expect(dead_code, reason = "Accessor for groups")]
    pub(crate) fn groups(&self) -> &[ScannedFolderGroup] {
        &self.groups
    }

    pub(crate) fn total_files(&self) -> usize {
        self.all_files_cache.len()
    }

    #[expect(dead_code, reason = "Accessor for sorted modified files")]
    pub(crate) fn sorted_modified_files(&self) -> &[WorkspaceFileEntry] {
        &self.sorted_modified_cache
    }

    #[expect(dead_code, reason = "Accessor for all cached files")]
    pub(crate) fn all_files(&self) -> &[WorkspaceFileEntry] {
        &self.all_files_cache
    }

    /// Adds a directory to tracked folders if it exists and is not already tracked.
    #[expect(dead_code, reason = "API for programmatic folder addition")]
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
        if !self.project_folders.contains(&canonical) {
            self.project_folders.push(canonical);
        }
        self.rescan();
        true
    }

    /// Removes a folder from tracked folders.
    #[expect(dead_code, reason = "API for programmatic folder removal")]
    pub(crate) fn remove_folder(&mut self, path: &Path) -> bool {
        let initial_len = self.folders.len();
        self.folders.retain(|f| f != path);
        self.project_folders.retain(|f| f != path);
        if self.selected_project_filter.as_deref() == Some(path) {
            self.selected_project_filter = None;
        }
        if self.folders.len() < initial_len {
            self.rescan();
            true
        } else {
            false
        }
    }

    /// Rescans both project scope and global AI agent scope, pruning missing folders.
    pub(crate) fn rescan(&mut self) {
        let snapshot = Self::scan_snapshot(self.project_folders.clone());
        self.apply_snapshot(snapshot);
    }

    /// Scans global and project scopes off the UI thread, pruning missing folders.
    pub(crate) fn scan_snapshot(mut project_folders: Vec<PathBuf>) -> CatalogSnapshot {
        project_folders.retain(|dir| crate::settings::path_exists_as_dir_or_unconfirmed(dir));

        let (global_instructions, global_skills) = Self::scan_global();
        let (project_instructions, project_skills, project_docs, groups) =
            Self::scan_project(&project_folders);
        let (sorted_modified_cache, all_files_cache) = Self::build_caches(&groups);

        CatalogSnapshot {
            project_folders,
            project_instructions,
            project_skills,
            project_docs,
            global_instructions,
            global_skills,
            groups,
            sorted_modified_cache,
            all_files_cache,
        }
    }

    /// Applies a previously scanned [`CatalogSnapshot`] onto this catalog.
    pub(crate) fn apply_snapshot(&mut self, snapshot: CatalogSnapshot) {
        self.project_folders = snapshot.project_folders;
        self.folders = self.project_folders.clone();
        self.selected_project_filter = self
            .selected_project_filter
            .take()
            .filter(|filter| self.project_folders.contains(filter));
        self.project_instructions = snapshot.project_instructions;
        self.project_skills = snapshot.project_skills;
        self.project_docs = snapshot.project_docs;
        self.global_instructions = snapshot.global_instructions;
        self.global_skills = snapshot.global_skills;
        self.groups = snapshot.groups;
        self.sorted_modified_cache = snapshot.sorted_modified_cache;
        self.all_files_cache = snapshot.all_files_cache;
    }

    fn scan_global() -> (Vec<InstructionEntry>, Vec<SkillEntry>) {
        let mut instructions = Vec::new();
        let mut skills = Vec::new();
        let mut seen_inst = HashSet::new();
        let mut seen_skills = HashSet::new();

        let home = user_home_dir();

        // Predefined agent environments (at most one rule file per provider)
        for agent in PREDEFINED_AGENTS {
            'inst_loop: for &inst_path in agent.instruction_paths {
                if inst_path.contains('*') {
                    if let Some(ref h) = home {
                        let rel_dir = inst_path.trim_end_matches("*.md").trim_end_matches('/');
                        let dir = h.join(rel_dir);
                        if dir.is_dir()
                            && let Ok(entries) = fs::read_dir(&dir)
                        {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.is_file()
                                    && is_markdown_file_name(&entry.file_name().to_string_lossy())
                                    && let Ok(c) = path.canonicalize()
                                    && seen_inst.insert(c)
                                    && let Some(inst) =
                                        scan_instruction_file(&path, agent.name, true, None, None)
                                {
                                    instructions.push(inst);
                                    break 'inst_loop;
                                }
                            }
                        }
                    }
                } else {
                    let full = resolve_agent_path(inst_path, home.as_deref());

                    if full.is_file()
                        && let Ok(c) = full.canonicalize()
                        && seen_inst.insert(c)
                        && let Some(inst) =
                            scan_instruction_file(&full, agent.name, true, None, None)
                    {
                        instructions.push(inst);
                        break 'inst_loop;
                    }
                }
            }

            for &skill_rel in agent.skill_paths.iter().chain(agent.fallback_skills) {
                let full = resolve_agent_path(skill_rel, home.as_deref());

                if full.is_dir() {
                    let discovered = scan_skills_container(
                        &full,
                        agent.name,
                        true,
                        &mut seen_skills,
                        None,
                        None,
                    );
                    skills.extend(discovered);
                }
            }
        }

        instructions.sort_by(|a, b| cmp_ignore_ascii_case(&a.name, &b.name));
        skills.sort_by(|a, b| cmp_ignore_ascii_case(&a.name, &b.name));

        (instructions, skills)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Comprehensive discovery across candidate project instruction files, skills, and docs"
    )]
    fn scan_project(
        project_folders: &[PathBuf],
    ) -> (
        Vec<InstructionEntry>,
        Vec<SkillEntry>,
        Vec<ProjectDocEntry>,
        Vec<ScannedFolderGroup>,
    ) {
        if project_folders.is_empty() {
            return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        }

        let mut all_instructions = Vec::new();
        let mut all_skills = Vec::new();
        let mut all_docs = Vec::new();
        let mut groups = Vec::with_capacity(project_folders.len());

        let candidate_rel_instructions = [
            "AGENTS.md",
            "AGENT.md",
            "CLAUDE.md",
            "GEMINI.md",
            "copilot-instructions.md",
            ".agents/AGENTS.md",
            ".claude/CLAUDE.md",
            ".gemini/AGENTS.md",
            ".config/opencode/AGENTS.md",
            ".devin/global_rules.md",
        ];

        let candidate_skill_containers = [
            (PROJECT_SKILL_CONTAINER_RELS[0], ".agents"),
            (PROJECT_SKILL_CONTAINER_RELS[1], ".claude"),
            (PROJECT_SKILL_CONTAINER_RELS[2], ".cursor"),
            (PROJECT_SKILL_CONTAINER_RELS[3], ".gemini"),
            (PROJECT_SKILL_CONTAINER_RELS[4], "opencode"),
            (PROJECT_SKILL_CONTAINER_RELS[5], "skills"),
        ];

        for project in project_folders {
            if !project.is_dir() {
                continue;
            }

            let project_name = project
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string();

            let mut seen_inst = HashSet::new();

            // 1. Project Instruction Files
            for &rel in SPECIAL_RULE_FILENAMES
                .iter()
                .chain(candidate_rel_instructions.iter())
            {
                let path = project.join(rel);
                if path.is_file()
                    && let Ok(c) = path.canonicalize()
                    && seen_inst.insert(c)
                {
                    let source = if rel.contains('/') {
                        format!(
                            "{project_name} · {}",
                            rel.split('/').next().unwrap_or("subfolder")
                        )
                    } else {
                        project_name.clone()
                    };
                    if let Some(inst) = scan_instruction_file(
                        &path,
                        &source,
                        false,
                        Some(project_name.clone()),
                        Some(project.clone()),
                    ) {
                        all_instructions.push(inst);
                    }
                }
            }

            // Subdir rules like .claude/rules/*.md
            let rules_dir = project.join(".claude").join("rules");
            if rules_dir.is_dir()
                && let Ok(entries) = fs::read_dir(&rules_dir)
            {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && is_markdown_file_name(&entry.file_name().to_string_lossy())
                        && let Ok(c) = path.canonicalize()
                        && seen_inst.insert(c)
                        && let Some(inst) = scan_instruction_file(
                            &path,
                            &format!("{project_name} · claude-rule"),
                            false,
                            Some(project_name.clone()),
                            Some(project.clone()),
                        )
                    {
                        all_instructions.push(inst);
                    }
                }
            }

            // 2. Project Skills
            let mut seen_skills = HashSet::new();
            for (rel, label) in &candidate_skill_containers {
                let path = project.join(rel);
                if path.is_dir() {
                    let source = format!("{project_name} · {label}");
                    let discovered = scan_skills_container(
                        &path,
                        &source,
                        false,
                        &mut seen_skills,
                        Some(project_name.clone()),
                        Some(project.clone()),
                    );
                    all_skills.extend(discovered);
                }
            }

            // 3. Project Documentation (all other Markdown files)
            let scanned_group = Self::scan_folder(project);
            let mut claimed_paths = HashSet::new();
            for inst in &all_instructions {
                if let Ok(c) = inst.file.full_path.canonicalize() {
                    claimed_paths.insert(c);
                }
            }
            for skill in &all_skills {
                if let Ok(c) = skill.skill_file.full_path.canonicalize() {
                    claimed_paths.insert(c);
                }
                for comp in &skill.companion_files {
                    if let Ok(c) = comp.full_path.canonicalize() {
                        claimed_paths.insert(c);
                    }
                }
            }

            for file in &scanned_group.files {
                let canonical = file
                    .full_path
                    .canonicalize()
                    .unwrap_or_else(|_| file.full_path.clone());
                if !claimed_paths.contains(&canonical) {
                    all_docs.push(ProjectDocEntry {
                        file: file.clone(),
                        project_name: Some(project_name.clone()),
                        project_path: Some(project.clone()),
                    });
                }
            }
            groups.push(scanned_group);
        }

        all_instructions.sort_by(|a, b| cmp_ignore_ascii_case(&a.name, &b.name));
        all_skills.sort_by(|a, b| cmp_ignore_ascii_case(&a.name, &b.name));

        // Sort project docs: prioritize prominent root files
        all_docs.sort_by(|a, b| {
            let a_priority = doc_priority(&a.file.file_name);
            let b_priority = doc_priority(&b.file.file_name);
            b_priority
                .cmp(&a_priority)
                .then_with(|| b.file.modified.cmp(&a.file.modified))
                .then_with(|| cmp_ignore_ascii_case(&a.file.relative_path, &b.file.relative_path))
        });

        (all_instructions, all_skills, all_docs, groups)
    }

    /// Scans a single directory iteratively for Markdown files using a worklist queue.
    pub(crate) fn scan_folder(folder: &Path) -> ScannedFolderGroup {
        const MAX_DEPTH: usize = 8;

        let folder_name = folder
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Project")
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
                    let meta = if file_type.is_symlink() {
                        fs::metadata(&full_path)
                    } else {
                        entry.metadata()
                    };
                    let Ok(meta) = meta else {
                        continue;
                    };
                    if !meta.is_file() {
                        continue;
                    }
                    let relative_path = if task.rel_prefix.is_empty() {
                        file_name.clone()
                    } else {
                        format!("{}{}", task.rel_prefix, file_name)
                    };
                    let (file_size, modified) = (meta.len(), meta.modified().ok());

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
    #[expect(dead_code, reason = "Utility for sorting folder groups")]
    pub(crate) fn sort_groups(groups: &mut [ScannedFolderGroup]) {
        groups.sort_by(|a, b| {
            let a_mod = a.files.first().and_then(|f| f.modified);
            let b_mod = b.files.first().and_then(|f| f.modified);
            b_mod
                .cmp(&a_mod)
                .then_with(|| cmp_ignore_ascii_case(&a.folder_name, &b.folder_name))
        });
    }

    /// Builds sorted and flat file caches from scanned folder groups.
    fn build_caches(
        groups: &[ScannedFolderGroup],
    ) -> (Vec<WorkspaceFileEntry>, Vec<WorkspaceFileEntry>) {
        let mut entries = Vec::new();
        for group in groups {
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

        entries.sort_by(|a, b| cmp_ignore_ascii_case(&a.file.relative_path, &b.file.relative_path));
        (by_modified, entries)
    }

    /// Resolves recent accessed file paths into `WorkspaceFileEntry` list.
    #[expect(
        dead_code,
        reason = "Legacy recent accessed file resolver kept for reference"
    )]
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

    /// Filters the entire hub across project & global instructions, skills, and docs.
    #[expect(
        clippy::too_many_lines,
        reason = "Aggregates and filters 5 distinct hub categories and sorts results"
    )]
    pub(crate) fn filter_hub(&self, query: &str) -> FilteredHubView {
        let q = query.trim().to_lowercase();
        let selected_filter = self.selected_project_filter.as_ref();

        let p_inst: Vec<_> = self
            .project_instructions
            .iter()
            .filter(|i| {
                if let Some(filter) = selected_filter
                    && i.project_path.as_ref() != Some(filter)
                {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                i.name.to_lowercase().contains(&q)
                    || i.agent_source.to_lowercase().contains(&q)
                    || i.file.relative_path.to_lowercase().contains(&q)
                    || i.project_name
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
            })
            .cloned()
            .collect();

        let p_skills: Vec<_> = self
            .project_skills
            .iter()
            .filter(|s| {
                if let Some(filter) = selected_filter
                    && s.project_path.as_ref() != Some(filter)
                {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                s.name.to_lowercase().contains(&q)
                    || s.description.to_lowercase().contains(&q)
                    || s.agent_source.to_lowercase().contains(&q)
                    || s.project_name
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
                    || s.companion_files
                        .iter()
                        .any(|c| c.file_name.to_lowercase().contains(&q))
            })
            .cloned()
            .collect();

        let p_docs: Vec<_> = self
            .project_docs
            .iter()
            .filter(|d| {
                if let Some(filter) = selected_filter
                    && d.project_path.as_ref() != Some(filter)
                {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                d.file.file_name.to_lowercase().contains(&q)
                    || d.file.relative_path.to_lowercase().contains(&q)
                    || d.project_name
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
            })
            .cloned()
            .collect();

        let g_inst: Vec<_> = self
            .global_instructions
            .iter()
            .filter(|i| {
                if q.is_empty() {
                    return true;
                }
                i.name.to_lowercase().contains(&q)
                    || i.agent_source.to_lowercase().contains(&q)
                    || i.file.relative_path.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();

        let g_skills: Vec<_> = self
            .global_skills
            .iter()
            .filter(|s| {
                if q.is_empty() {
                    return true;
                }
                s.name.to_lowercase().contains(&q)
                    || s.description.to_lowercase().contains(&q)
                    || s.agent_source.to_lowercase().contains(&q)
                    || s.companion_files
                        .iter()
                        .any(|c| c.file_name.to_lowercase().contains(&q))
            })
            .cloned()
            .collect();

        let total = p_inst.len() + p_skills.len() + p_docs.len() + g_inst.len() + g_skills.len();

        FilteredHubView {
            project_instructions: p_inst,
            project_skills: p_skills,
            project_docs: p_docs,
            global_instructions: g_inst,
            global_skills: g_skills,
            total_matches: total,
        }
    }

    /// Filters files across folders by search query (backwards compatibility).
    #[expect(dead_code, reason = "Legacy filter method preserved for compatibility")]
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

fn doc_priority(name: &str) -> u8 {
    let upper = name.to_ascii_uppercase();
    if upper.starts_with("README") {
        3
    } else if upper.starts_with("CHANGELOG")
        || upper.starts_with("CONTRIBUTING")
        || upper.starts_with("ARCHITECTURE")
        || upper.starts_with("SECURITY")
        || upper.starts_with("ROADMAP")
        || upper.starts_with("SPEC")
    {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        // Crucial: .agents and .claude are NOT considered noise
        assert!(!is_noise_or_hidden_dir(".agents"));
        assert!(!is_noise_or_hidden_dir(".claude"));
        assert!(!is_noise_or_hidden_dir(".gemini"));
        assert!(!is_noise_or_hidden_dir(".cursor"));
        assert!(!is_noise_or_hidden_dir("src"));
        assert!(!is_noise_or_hidden_dir("docs"));
    }

    #[test]
    fn test_parse_skill_metadata() {
        let temp_dir =
            std::env::temp_dir().join(format!("downmark_test_meta_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let skill_file = temp_dir.join("SKILL.md");
        fs::write(
            &skill_file,
            "---\nname: my-skill\ndescription: >-\n  A great skill for coding\n---\n\n# Fallback Header\nBody\n",
        )
        .unwrap();

        let (name, desc) = parse_skill_metadata(&skill_file, "default-dir");
        assert_eq!(name, "my-skill");
        assert_eq!(desc, "A great skill for coding");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_scan_skill_dir_with_companions() {
        let temp_dir =
            std::env::temp_dir().join(format!("downmark_test_skill_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let skill_path = temp_dir.join("pr");
        fs::create_dir_all(skill_path.join("references")).unwrap();

        fs::write(
            skill_path.join("SKILL.md"),
            "---\nname: pr-skill\ndescription: Pull request helper\n---\n",
        )
        .unwrap();
        fs::write(skill_path.join("CREDITS.md"), "# Credits\n").unwrap();
        fs::write(skill_path.join("references").join("rules.md"), "# Rules\n").unwrap();

        let skill = scan_skill_dir(&skill_path, "OpenAI Codex", true, None, None).unwrap();
        assert_eq!(skill.name, "pr-skill");
        assert_eq!(skill.description, "Pull request helper");
        assert_eq!(skill.companion_files.len(), 2);
        assert!(
            skill
                .companion_files
                .iter()
                .any(|c| c.file_name == "CREDITS.md")
        );
        assert!(
            skill
                .companion_files
                .iter()
                .any(|c| c.file_name == "rules.md")
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_project_scope_categorization() {
        let temp_dir =
            std::env::temp_dir().join(format!("downmark_test_proj_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let skills_dir = temp_dir.join(".agents").join("skills").join("test-skill");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::create_dir_all(temp_dir.join("docs")).unwrap();

        fs::write(temp_dir.join("AGENTS.md"), "# Agent instructions\n").unwrap();
        fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: test-skill\n---\n# Test\n",
        )
        .unwrap();
        fs::write(temp_dir.join("README.md"), "# My Project\n").unwrap();
        fs::write(
            temp_dir.join("docs").join("architecture.md"),
            "# Architecture\n",
        )
        .unwrap();

        let catalog = FolderCatalog::new(vec![temp_dir.clone()], None);

        assert_eq!(catalog.project_instructions().len(), 1);
        assert_eq!(catalog.project_instructions()[0].name, "AGENTS.md");

        assert_eq!(catalog.project_skills().len(), 1);
        assert_eq!(catalog.project_skills()[0].name, "test-skill");

        assert_eq!(catalog.project_docs().len(), 2);
        assert_eq!(catalog.project_docs()[0].file.file_name, "README.md");

        let filtered = catalog.filter_hub("architecture");
        assert_eq!(filtered.project_docs.len(), 1);
        assert_eq!(filtered.project_docs[0].file.file_name, "architecture.md");

        let filtered_unique = catalog.filter_hub("test-skill");
        assert_eq!(filtered_unique.project_skills.len(), 1);
        assert_eq!(filtered_unique.project_skills[0].name, "test-skill");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_multiple_project_folders() {
        let base_dir =
            std::env::temp_dir().join(format!("downmark_test_multi_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base_dir);
        let proj_a = base_dir.join("project_a");
        let proj_b = base_dir.join("project_b");
        fs::create_dir_all(&proj_a).unwrap();
        fs::create_dir_all(&proj_b).unwrap();

        fs::write(proj_a.join("AGENTS.md"), "# Project A Rules\n").unwrap();
        fs::write(proj_a.join("README.md"), "# Project A\n").unwrap();

        fs::write(proj_b.join("CLAUDE.md"), "# Project B Rules\n").unwrap();
        fs::write(proj_b.join("GUIDE.md"), "# Project B Guide\n").unwrap();

        let mut catalog = FolderCatalog::new(vec![proj_a.clone(), proj_b.clone()], None);

        // When filter is None, both projects are aggregated
        assert_eq!(catalog.project_instructions().len(), 2);
        assert_eq!(catalog.project_docs().len(), 2);

        // When filtered to project_a
        catalog.set_selected_project_filter(Some(proj_a));
        let filtered_a = catalog.filter_hub("");
        assert_eq!(filtered_a.project_instructions.len(), 1);
        assert_eq!(filtered_a.project_instructions[0].name, "AGENTS.md");
        assert_eq!(filtered_a.project_docs.len(), 1);
        assert_eq!(filtered_a.project_docs[0].file.file_name, "README.md");

        // When filtered to project_b
        catalog.set_selected_project_filter(Some(proj_b));
        let filtered_b = catalog.filter_hub("");
        assert_eq!(filtered_b.project_instructions.len(), 1);
        assert_eq!(filtered_b.project_instructions[0].name, "CLAUDE.md");
        assert_eq!(filtered_b.project_docs.len(), 1);
        assert_eq!(filtered_b.project_docs[0].file.file_name, "GUIDE.md");

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn test_global_instructions_single_per_provider() {
        let catalog = FolderCatalog::new(vec![], None);
        let globals = catalog.global_instructions();

        let mut seen_providers = HashSet::new();
        for rule in globals {
            assert!(
                !rule.agent_source.contains("Shared"),
                "Must not contain 'Shared': {}",
                rule.agent_source
            );
            assert!(
                seen_providers.insert(rule.agent_source.clone()),
                "Provider '{}' must only have at most one rule file",
                rule.agent_source
            );
        }
    }

    #[test]
    fn test_rescan_prunes_deleted_project_folders_files_and_broken_symlinks() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let base_dir = std::env::temp_dir().join(format!(
            "downmark_test_rescan_prune_{}_{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base_dir);
        let proj_a = base_dir.join("proj_a");
        let proj_b = base_dir.join("proj_b");
        fs::create_dir_all(&proj_a).unwrap();
        fs::create_dir_all(&proj_b).unwrap();

        let keep_md = proj_a.join("README.md");
        let remove_md = proj_a.join("old.md");
        let target_md = base_dir.join("target.md");
        let link_md = proj_a.join("linked.md");
        fs::write(&keep_md, "# Keep\n").unwrap();
        fs::write(&remove_md, "# Old\n").unwrap();
        fs::write(&target_md, "# Target\n").unwrap();
        fs::write(proj_b.join("NOTES.md"), "# Notes\n").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target_md, &link_md).unwrap();
        #[cfg(windows)]
        let _ = fs::write(&link_md, "# Target\n");

        let mut catalog = FolderCatalog::new(vec![proj_a, proj_b.clone()], Some(proj_b.clone()));
        assert_eq!(catalog.project_folders().len(), 2);
        assert_eq!(catalog.project_docs().len(), 4);

        // Delete one markdown file, break the symlink target, and delete proj_b.
        fs::remove_file(&remove_md).unwrap();
        fs::remove_file(&target_md).unwrap();
        #[cfg(windows)]
        let _ = fs::remove_file(&link_md);
        fs::remove_dir_all(&proj_b).unwrap();

        catalog.rescan();

        assert_eq!(catalog.project_folders().len(), 1);
        assert_eq!(catalog.selected_project_filter(), None);
        assert_eq!(catalog.project_docs().len(), 1);
        assert_eq!(catalog.project_docs()[0].file.file_name, "README.md");

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn test_is_catalog_file_matches_special_dot_rules() {
        assert!(is_catalog_file(Path::new("/tmp/proj/.cursorrules")));
        assert!(is_catalog_file(Path::new(
            "/tmp/proj/.copilot-instructions.md"
        )));
        assert!(is_catalog_file(Path::new("/tmp/proj/AGENTS.md")));
        assert!(!is_catalog_file(Path::new("/tmp/proj/.gitignore")));
        assert!(!is_catalog_file(Path::new("/tmp/proj/main.rs")));
    }

    #[test]
    fn test_apply_snapshot_preserves_current_filter_when_still_present() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let base_dir = std::env::temp_dir().join(format!(
            "downmark_test_snapshot_filter_{}_{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base_dir);
        let proj_a = base_dir.join("proj_a");
        let proj_b = base_dir.join("proj_b");
        fs::create_dir_all(&proj_a).unwrap();
        fs::create_dir_all(&proj_b).unwrap();

        let mut catalog =
            FolderCatalog::new(vec![proj_a.clone(), proj_b.clone()], Some(proj_a.clone()));
        let snapshot = FolderCatalog::scan_snapshot(vec![proj_a, proj_b.clone()]);
        // Simulate user switching filter while background scan was in flight.
        catalog.set_selected_project_filter(Some(proj_b.clone()));
        catalog.apply_snapshot(snapshot);
        assert_eq!(catalog.selected_project_filter(), Some(&proj_b));

        let _ = fs::remove_dir_all(&base_dir);
    }
}
