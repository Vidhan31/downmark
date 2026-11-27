//! Filesystem watching for open documents, recent files, and workspace folders.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use notify::{
    Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RemoveKind},
};

use crate::catalog::{PROJECT_SKILL_CONTAINER_RELS, is_catalog_file, is_noise_or_hidden_dir};

const MAX_PROJECT_WATCH_DEPTH: usize = 8;
const MAX_GLOBAL_SKILL_WATCH_DEPTH: usize = 5;
const MAX_SKILL_PACKAGE_DEPTH: usize = 4;
const MAX_QUEUED_PATHS: usize = 512;

/// Whether global AI agent folders should be included in filesystem watching.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum GlobalWatch {
    #[default]
    Include,
    Skip,
}

/// Paths the workspace wants to monitor for external disk changes.
#[derive(Debug, Clone, Default)]
pub(crate) struct WatchTargets {
    pub(crate) open_files: Vec<PathBuf>,
    pub(crate) recent_files: Vec<PathBuf>,
    pub(crate) project_folders: Vec<PathBuf>,
    pub(crate) global_watch: GlobalWatch,
}

/// Classified summary of filesystem changes drained after debouncing.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Four independent dispatch flags for watcher subsystems"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WatchDispatch {
    pub(crate) open_files: bool,
    pub(crate) recent_files: bool,
    pub(crate) catalog: bool,
    pub(crate) resync_watches: bool,
}

impl WatchDispatch {
    pub(crate) fn any(self) -> bool {
        self.open_files || self.recent_files || self.catalog || self.resync_watches
    }
}

/// Directory sets derived from [`WatchTargets`] for non-recursive OS watches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WatchDirSet {
    pub(crate) file_parents: HashSet<PathBuf>,
    pub(crate) catalog_dirs: HashSet<PathBuf>,
    pub(crate) skill_containers: HashSet<PathBuf>,
    pub(crate) skill_container_links: HashSet<PathBuf>,
    pub(crate) catalog_ancestors: HashMap<PathBuf, HashSet<PathBuf>>,
    pub(crate) file_ancestors: HashMap<PathBuf, HashSet<PathBuf>>,
    pub(crate) external_catalog_targets: HashMap<PathBuf, HashSet<PathBuf>>,
    pub(crate) incomplete: bool,
}

impl WatchDirSet {
    fn all_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.file_parents
            .iter()
            .chain(self.catalog_dirs.iter())
            .chain(self.catalog_ancestors.keys())
            .chain(self.file_ancestors.keys())
            .chain(self.external_catalog_targets.keys())
    }
}

/// Summary of newly registered watches after [`DiskWatcher::apply_sync`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WatchSyncOutcome {
    pub(crate) added_file_parents: bool,
    pub(crate) added_catalog_dirs: bool,
}

#[derive(Debug, Default)]
struct RawWatchState {
    events: Vec<(EventKind, Vec<PathBuf>)>,
    needs_rescan: bool,
}

#[derive(Debug, Default)]
struct ActiveWatchFilter {
    open_files: HashSet<PathBuf>,
    recent_files: HashSet<PathBuf>,
    file_parents: HashSet<PathBuf>,
    catalog_dirs: HashSet<PathBuf>,
    skill_containers: HashSet<PathBuf>,
    skill_container_links: HashSet<PathBuf>,
    catalog_ancestors: HashMap<PathBuf, HashSet<PathBuf>>,
    file_ancestors: HashMap<PathBuf, HashSet<PathBuf>>,
    external_catalog_targets: HashMap<PathBuf, HashSet<PathBuf>>,
}

/// Watches open documents (via their parent directories) and workspace folders
/// using the platform's native filesystem notification backend.
#[derive(Debug)]
pub(crate) struct DiskWatcher {
    watcher: Option<RecommendedWatcher>,
    watched_dirs: HashSet<PathBuf>,
    filter: Arc<Mutex<ActiveWatchFilter>>,
    pending: Arc<Mutex<RawWatchState>>,
    wakeup_tx: smol::channel::Sender<()>,
    degraded: bool,
}

impl DiskWatcher {
    /// Creates a new watcher and returns `(watcher, wakeup_rx)`.
    ///
    /// If initializing the OS watcher fails (for example, due to system watch limits),
    /// the returned `DiskWatcher` enters degraded mode (`is_degraded() == true`) so
    /// the caller can fall back to polling.
    pub(crate) fn new() -> (Self, smol::channel::Receiver<()>) {
        let (tx, rx) = smol::channel::bounded(1);
        let pending = Arc::new(Mutex::new(RawWatchState::default()));
        let filter = Arc::new(Mutex::new(ActiveWatchFilter::default()));

        let pending_cb = Arc::clone(&pending);
        let cb_tx = tx.clone();
        let watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    if matches!(event.kind, EventKind::Access(_)) {
                        return;
                    }
                    let mut state = pending_cb.lock().expect("watch state lock poisoned");
                    if event.need_rescan() || state.events.len() >= MAX_QUEUED_PATHS {
                        state.needs_rescan = true;
                    } else {
                        state.events.push((event.kind, event.paths));
                    }
                    drop(state);
                    let _ = cb_tx.try_send(());
                } else {
                    let mut state = pending_cb.lock().expect("watch state lock poisoned");
                    state.needs_rescan = true;
                    drop(state);
                    let _ = cb_tx.try_send(());
                }
            },
            Config::default().with_follow_symlinks(false),
        );

        let (watcher, degraded) = match watcher {
            Ok(w) => (Some(w), false),
            Err(_) => (None, true),
        };

        (
            Self {
                watcher,
                watched_dirs: HashSet::new(),
                filter,
                pending,
                wakeup_tx: tx,
                degraded,
            },
            rx,
        )
    }

    pub(crate) fn is_degraded(&self) -> bool {
        self.degraded
    }

    /// Returns `true` if `file_path`'s parent directory currently has an active OS watch.
    pub(crate) fn covers_file_parent(&self, file_path: &Path) -> bool {
        let Some(parent) = file_path.parent() else {
            return false;
        };
        if self.watched_dirs.contains(parent) {
            return true;
        }
        parent
            .canonicalize()
            .is_ok_and(|canon| self.watched_dirs.contains(&canon))
    }

    /// Updates watched directories and path filters using precomputed [`WatchDirSet`].
    pub(crate) fn apply_sync(
        &mut self,
        targets: &WatchTargets,
        dir_set: &WatchDirSet,
    ) -> WatchSyncOutcome {
        let newly_covered_file_parent = {
            let mut filter = self.filter.lock().expect("watch filter lock poisoned");
            let prev_file_parents = std::mem::take(&mut filter.file_parents);
            let newly_covered = dir_set
                .file_parents
                .iter()
                .any(|dir| !prev_file_parents.contains(dir) && self.watched_dirs.contains(dir));
            filter.open_files = expand_path_variants(&targets.open_files);
            filter.recent_files = expand_path_variants(&targets.recent_files);
            filter.file_parents.clone_from(&dir_set.file_parents);
            filter.catalog_dirs.clone_from(&dir_set.catalog_dirs);
            filter
                .skill_containers
                .clone_from(&dir_set.skill_containers);
            filter
                .skill_container_links
                .clone_from(&dir_set.skill_container_links);
            filter
                .catalog_ancestors
                .clone_from(&dir_set.catalog_ancestors);
            filter.file_ancestors.clone_from(&dir_set.file_ancestors);
            filter
                .external_catalog_targets
                .clone_from(&dir_set.external_catalog_targets);
            newly_covered
        };

        let mut outcome = WatchSyncOutcome {
            added_file_parents: newly_covered_file_parent,
            ..WatchSyncOutcome::default()
        };
        let Some(watcher) = self.watcher.as_mut() else {
            self.degraded = true;
            return outcome;
        };

        let desired: HashSet<PathBuf> = dir_set.all_dirs().cloned().collect();

        let to_remove: Vec<PathBuf> = self
            .watched_dirs
            .iter()
            .filter(|dir| !desired.contains(*dir))
            .cloned()
            .collect();
        for dir in to_remove {
            let _ = watcher.unwatch(&dir);
            self.watched_dirs.remove(&dir);
        }

        let mut watch_error = false;
        for dir in desired {
            if self.watched_dirs.contains(&dir) {
                continue;
            }
            if watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
                if dir_set.file_parents.contains(&dir) {
                    outcome.added_file_parents = true;
                }
                if dir_set.catalog_dirs.contains(&dir) {
                    outcome.added_catalog_dirs = true;
                }
                self.watched_dirs.insert(dir);
            } else {
                watch_error = true;
            }
        }
        let was_degraded = self.degraded;
        self.degraded = watch_error || dir_set.incomplete;
        if !was_degraded && self.degraded {
            let _ = self.wakeup_tx.try_send(());
        }
        outcome
    }

    /// Drains all queued OS events and classifies which subsystems need updating.
    pub(crate) fn drain_dispatch(&mut self) -> WatchDispatch {
        let (events, needs_rescan) = {
            let mut state = self.pending.lock().expect("watch state lock poisoned");
            let events = std::mem::take(&mut state.events);
            let needs_rescan = std::mem::take(&mut state.needs_rescan);
            (events, needs_rescan)
        };

        if needs_rescan {
            self.invalidate_all_watched_dirs();
            let filter = self.filter.lock().expect("watch filter lock poisoned");
            return WatchDispatch {
                open_files: !filter.open_files.is_empty(),
                recent_files: !filter.recent_files.is_empty(),
                catalog: !filter.catalog_dirs.is_empty() || !filter.catalog_ancestors.is_empty(),
                resync_watches: true,
            };
        }

        let mut invalidated_any = false;
        for (kind, paths) in &events {
            if is_dir_invalidation_event(*kind) {
                for path in paths {
                    invalidated_any |= self.invalidate_watched_dir(path);
                }
            }
        }

        let filter = self.filter.lock().expect("watch filter lock poisoned");
        let mut dispatch = WatchDispatch {
            resync_watches: invalidated_any,
            ..Default::default()
        };
        for (kind, paths) in events {
            let step = classify_event(kind, &paths, &filter);
            dispatch.open_files |= step.open_files;
            dispatch.recent_files |= step.recent_files;
            dispatch.catalog |= step.catalog;
            dispatch.resync_watches |= step.resync_watches;
            if dispatch.open_files
                && dispatch.recent_files
                && dispatch.catalog
                && dispatch.resync_watches
            {
                break;
            }
        }
        dispatch
    }

    fn invalidate_all_watched_dirs(&mut self) {
        if let Some(watcher) = self.watcher.as_mut() {
            for dir in self.watched_dirs.drain() {
                let _ = watcher.unwatch(&dir);
            }
        } else {
            self.watched_dirs.clear();
        }
    }

    fn invalidate_watched_dir(&mut self, path: &Path) -> bool {
        let canon = canonical_or_owned(path);
        let invalidated: Vec<PathBuf> = self
            .watched_dirs
            .iter()
            .filter(|dir| {
                *dir == path || dir.starts_with(path) || *dir == &canon || dir.starts_with(&canon)
            })
            .cloned()
            .collect();
        if invalidated.is_empty() {
            return false;
        }
        for dir in &invalidated {
            self.watched_dirs.remove(dir);
            if let Some(watcher) = self.watcher.as_mut() {
                let _ = watcher.unwatch(dir);
            }
        }
        true
    }
}

/// Computes all directories that should be watched non-recursively for `targets`.
pub(crate) fn collect_watch_dirs(targets: &WatchTargets) -> WatchDirSet {
    let mut set = WatchDirSet::default();
    let mut explored_remaining: HashMap<PathBuf, usize> = HashMap::new();

    for path in targets.open_files.iter().chain(targets.recent_files.iter()) {
        insert_parent_dir(&mut set.file_parents, &mut set.file_ancestors, path);
    }

    for project in &targets.project_folders {
        if project.is_dir() {
            let root = canonical_or_owned(project);
            walk_non_noise_dirs(
                &root,
                MAX_PROJECT_WATCH_DEPTH,
                &mut set.catalog_dirs,
                &mut set.external_catalog_targets,
                &mut set.catalog_ancestors,
                &mut explored_remaining,
                &mut set.incomplete,
            );
            for rel in PROJECT_SKILL_CONTAINER_RELS {
                let container = root.join(rel);
                if container.is_dir() {
                    let canon_container = canonical_or_owned(&container);
                    set.skill_containers.insert(canon_container.clone());
                    if canon_container != container {
                        set.skill_container_links.insert(container);
                    }
                    walk_skills_container_dirs(
                        &canon_container,
                        &mut set.catalog_dirs,
                        &mut set.external_catalog_targets,
                        &mut set.catalog_ancestors,
                        &mut explored_remaining,
                        &mut set.incomplete,
                    );
                }
            }
            let rules_dirs = [
                (root.join(".claude").join("rules"), 1),
                (root.join(".github").join("instructions"), 4),
            ];
            for (rules_dir, max_depth) in rules_dirs {
                register_project_rule_dir(
                    &root,
                    &rules_dir,
                    max_depth,
                    &mut set,
                    &mut explored_remaining,
                );
            }
            register_missing_ancestor(&root, &mut set.catalog_ancestors);
            if project != &root {
                register_missing_ancestor(project, &mut set.catalog_ancestors);
            }
        } else {
            register_missing_ancestor(project, &mut set.catalog_ancestors);
        }
    }

    if targets.global_watch == GlobalWatch::Include {
        let global = crate::catalog::global_watch_dirs();
        for dir in global.instruction_dirs {
            if dir.is_dir() {
                let root = canonical_or_owned(&dir);
                walk_non_noise_dirs(
                    &root,
                    MAX_GLOBAL_SKILL_WATCH_DEPTH,
                    &mut set.catalog_dirs,
                    &mut set.external_catalog_targets,
                    &mut set.catalog_ancestors,
                    &mut explored_remaining,
                    &mut set.incomplete,
                );
            }
        }
        for dir in global.skill_containers {
            if dir.is_dir() {
                let canon_container = canonical_or_owned(&dir);
                set.skill_containers.insert(canon_container.clone());
                if canon_container != dir {
                    set.skill_container_links.insert(dir);
                }
                walk_skills_container_dirs(
                    &canon_container,
                    &mut set.catalog_dirs,
                    &mut set.external_catalog_targets,
                    &mut set.catalog_ancestors,
                    &mut explored_remaining,
                    &mut set.incomplete,
                );
            }
        }
        for missing in global.missing_targets {
            register_missing_ancestor(&missing, &mut set.catalog_ancestors);
        }
    }

    set
}

fn register_project_rule_dir(
    project_root: &Path,
    rules_dir: &Path,
    max_depth: usize,
    set: &mut WatchDirSet,
    explored_remaining: &mut HashMap<PathBuf, usize>,
) {
    if rules_dir.is_dir() {
        let canon_rules = canonical_or_owned(rules_dir);
        if canon_rules != rules_dir {
            set.skill_container_links.insert(rules_dir.to_path_buf());
            walk_non_noise_dirs(
                &canon_rules,
                max_depth,
                &mut set.catalog_dirs,
                &mut set.external_catalog_targets,
                &mut set.catalog_ancestors,
                explored_remaining,
                &mut set.incomplete,
            );
        }
    } else if std::fs::symlink_metadata(rules_dir).is_ok_and(|meta| meta.file_type().is_symlink())
        && !rules_dir.exists()
        && let Ok(raw_target) = std::fs::read_link(rules_dir)
    {
        let parent = rules_dir.parent().unwrap_or(project_root);
        let dest = resolve_dangling_target(parent, &raw_target);
        register_missing_ancestor(&dest, &mut set.catalog_ancestors);
        set.skill_container_links.insert(rules_dir.to_path_buf());
    }
}

fn insert_parent_dir(
    out: &mut HashSet<PathBuf>,
    ancestors: &mut HashMap<PathBuf, HashSet<PathBuf>>,
    file_path: &Path,
) {
    let mut found = false;
    if let Some(parent) = file_path.parent()
        && parent.is_dir()
    {
        let canon_parent = canonical_or_owned(parent);
        out.insert(canon_parent.clone());
        register_missing_ancestor(&canon_parent, ancestors);
        if parent != canon_parent {
            register_missing_ancestor(parent, ancestors);
        }
        found = true;
    }
    if let Ok(canonical_file) = file_path.canonicalize()
        && let Some(parent) = canonical_file.parent()
        && parent.is_dir()
    {
        out.insert(parent.to_path_buf());
        register_missing_ancestor(parent, ancestors);
        found = true;
    }
    if !found && let Some(parent) = file_path.parent() {
        register_missing_ancestor(parent, ancestors);
    }
}

fn register_missing_ancestor(target: &Path, ancestors: &mut HashMap<PathBuf, HashSet<PathBuf>>) {
    for ancestor in target.ancestors().skip(1) {
        if ancestor.is_dir() {
            let canon_ancestor = canonical_or_owned(ancestor);
            let resolved_target = target
                .strip_prefix(ancestor)
                .map_or_else(|_| target.to_path_buf(), |rel| canon_ancestor.join(rel));
            ancestors
                .entry(canon_ancestor)
                .or_default()
                .insert(resolved_target);
            break;
        }
    }
}

fn expand_path_variants(paths: &[PathBuf]) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    for path in paths {
        out.insert(path.clone());
        if let Ok(canonical) = path.canonicalize() {
            out.insert(canonical);
        } else if let Some(parent) = path.parent()
            && let Some(name) = path.file_name()
            && let Ok(canon_parent) = parent.canonicalize()
        {
            out.insert(canon_parent.join(name));
        }
    }
    out
}

fn canonical_or_owned(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_dangling_target(base_dir: &Path, raw_target: &Path) -> PathBuf {
    resolve_dangling_target_inner(base_dir, raw_target, 16)
}

fn resolve_dangling_target_inner(
    base_dir: &Path,
    raw_target: &Path,
    remaining_hops: usize,
) -> PathBuf {
    use std::path::Component;

    let mut cur = if raw_target.is_absolute() {
        PathBuf::new()
    } else {
        canonical_or_owned(base_dir)
    };
    let mut resolve_existing = true;

    for comp in raw_target.components() {
        match comp {
            Component::Prefix(prefix) => {
                cur.push(prefix.as_os_str());
            }
            Component::RootDir => {
                cur.push(Component::RootDir.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolve_existing {
                    cur.push(comp.as_os_str());
                } else if !cur.pop() {
                    cur.push(comp.as_os_str());
                    resolve_existing = false;
                } else {
                    cur = canonical_or_owned(&cur);
                }
            }
            Component::Normal(part) => {
                cur.push(part);
                if resolve_existing {
                    if let Ok(canon) = cur.canonicalize() {
                        cur = canon;
                    } else if remaining_hops > 0
                        && let Ok(link_target) = std::fs::read_link(&cur)
                    {
                        let link_base = cur.parent().unwrap_or(base_dir);
                        cur = resolve_dangling_target_inner(
                            link_base,
                            &link_target,
                            remaining_hops - 1,
                        );
                        resolve_existing = cur.exists();
                    } else {
                        resolve_existing = false;
                    }
                }
            }
        }
    }
    cur
}

fn walk_non_noise_dirs(
    root: &Path,
    max_depth: usize,
    out: &mut HashSet<PathBuf>,
    external_targets: &mut HashMap<PathBuf, HashSet<PathBuf>>,
    catalog_ancestors: &mut HashMap<PathBuf, HashSet<PathBuf>>,
    explored_remaining: &mut HashMap<PathBuf, usize>,
    incomplete: &mut bool,
) {
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    while let Some((dir, depth)) = stack.pop() {
        let remaining = max_depth.saturating_sub(depth);
        if let Some(&prev_remaining) = explored_remaining.get(&dir)
            && prev_remaining >= remaining
        {
            continue;
        }
        explored_remaining.insert(dir.clone(), remaining);
        out.insert(dir.clone());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            *incomplete = true;
            continue;
        };
        for entry_res in entries {
            let Ok(entry) = entry_res else {
                *incomplete = true;
                continue;
            };
            let Ok(ft) = entry.file_type() else {
                *incomplete = true;
                continue;
            };
            if ft.is_symlink() {
                let entry_path = entry.path();
                if is_catalog_file(&entry_path) {
                    if let Ok(canon_target) = entry_path.canonicalize() {
                        if canon_target.is_file()
                            && let Some(parent) = canon_target.parent()
                        {
                            external_targets
                                .entry(parent.to_path_buf())
                                .or_default()
                                .insert(canon_target);
                        }
                    } else if let Ok(raw_target) = std::fs::read_link(&entry_path) {
                        let dest = resolve_dangling_target(&dir, &raw_target);
                        register_missing_ancestor(&dest, catalog_ancestors);
                    }
                }
                continue;
            }
            if depth >= max_depth {
                continue;
            }
            if !ft.is_dir() {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if is_noise_or_hidden_dir(&name) {
                continue;
            }
            stack.push((dir.join(name), depth + 1));
        }
    }
}

/// Walks a skill container directory, descending into skill packages (including
/// `build` and symlinked skill directories) and deduplicating by canonical path.
fn walk_skills_container_dirs(
    container: &Path,
    out: &mut HashSet<PathBuf>,
    external_targets: &mut HashMap<PathBuf, HashSet<PathBuf>>,
    catalog_ancestors: &mut HashMap<PathBuf, HashSet<PathBuf>>,
    explored_remaining: &mut HashMap<PathBuf, usize>,
    incomplete: &mut bool,
) {
    out.insert(container.to_path_buf());
    let Ok(entries) = std::fs::read_dir(container) else {
        *incomplete = true;
        return;
    };
    for entry_res in entries {
        let Ok(entry) = entry_res else {
            *incomplete = true;
            continue;
        };
        let Ok(ft) = entry.file_type() else {
            *incomplete = true;
            continue;
        };
        let path = entry.path();
        if ft.is_dir() || (ft.is_symlink() && path.is_dir()) {
            let canon = canonical_or_owned(&path);
            walk_non_noise_dirs(
                &canon,
                MAX_SKILL_PACKAGE_DEPTH,
                out,
                external_targets,
                catalog_ancestors,
                explored_remaining,
                incomplete,
            );
        } else if ft.is_symlink()
            && !path.exists()
            && let Ok(raw_target) = std::fs::read_link(&path)
        {
            let dest = resolve_dangling_target(container, &raw_target);
            register_missing_ancestor(&dest, catalog_ancestors);
        }
    }
}

fn is_dir_invalidation_event(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(CreateKind::Folder | CreateKind::Any | CreateKind::Other)
            | EventKind::Remove(RemoveKind::Folder | RemoveKind::Any | RemoveKind::Other)
            | EventKind::Modify(ModifyKind::Name(_) | ModifyKind::Any | ModifyKind::Other)
            | EventKind::Any
            | EventKind::Other
    )
}

fn classify_event(kind: EventKind, paths: &[PathBuf], filter: &ActiveWatchFilter) -> WatchDispatch {
    let mut dispatch = WatchDispatch::default();

    if paths.is_empty() {
        if matches!(kind, EventKind::Any | EventKind::Other) {
            dispatch.open_files = !filter.open_files.is_empty();
            dispatch.recent_files = !filter.recent_files.is_empty();
            dispatch.catalog =
                !filter.catalog_dirs.is_empty() || !filter.catalog_ancestors.is_empty();
        }
        return dispatch;
    }

    let is_dir_or_generic_event = is_dir_invalidation_event(kind)
        || matches!(
            kind,
            EventKind::Create(CreateKind::Folder | CreateKind::Any | CreateKind::Other)
        );

    for path in paths {
        if filter.open_files.contains(path) {
            dispatch.open_files = true;
        }
        if filter.recent_files.contains(path) {
            dispatch.recent_files = true;
        }

        if is_dir_or_generic_event {
            if filter.file_parents.contains(path) {
                dispatch.open_files = !filter.open_files.is_empty();
                dispatch.recent_files = !filter.recent_files.is_empty();
            }
            if filter.catalog_dirs.contains(path) {
                dispatch.catalog = true;
            }
        }

        if matches_ancestor_target(path, &filter.catalog_ancestors) {
            dispatch.catalog = true;
        }
        if matches_ancestor_target(path, &filter.file_ancestors) {
            dispatch.open_files = !filter.open_files.is_empty();
            dispatch.recent_files = !filter.recent_files.is_empty();
            dispatch.resync_watches = true;
        }

        if !dispatch.catalog && is_catalog_relevant_path(kind, path, filter) {
            dispatch.catalog = true;
        }
    }

    dispatch
}

fn matches_ancestor_target(path: &Path, ancestors: &HashMap<PathBuf, HashSet<PathBuf>>) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(targets) = ancestors.get(parent) else {
        return false;
    };
    targets
        .iter()
        .any(|target| target == path || target.starts_with(path))
}

fn is_catalog_relevant_path(kind: EventKind, path: &Path, filter: &ActiveWatchFilter) -> bool {
    if filter.skill_container_links.contains(path)
        && matches!(
            kind,
            EventKind::Create(_)
                | EventKind::Remove(_)
                | EventKind::Modify(ModifyKind::Name(_) | ModifyKind::Any | ModifyKind::Other)
                | EventKind::Any
                | EventKind::Other
        )
    {
        return true;
    }

    if (is_dir_invalidation_event(kind) && filter.external_catalog_targets.contains_key(path))
        || path.parent().is_some_and(|parent| {
            filter
                .external_catalog_targets
                .get(parent)
                .is_some_and(|targets| targets.contains(path))
        })
    {
        return true;
    }

    if path
        .parent()
        .is_some_and(|parent| filter.skill_containers.contains(parent))
        && matches!(
            kind,
            EventKind::Create(_)
                | EventKind::Remove(_)
                | EventKind::Modify(ModifyKind::Name(_) | ModifyKind::Any | ModifyKind::Other)
                | EventKind::Any
                | EventKind::Other
        )
    {
        return true;
    }

    let inside_catalog = path
        .parent()
        .is_some_and(|parent| filter.catalog_dirs.contains(parent))
        || filter.catalog_dirs.contains(path);
    if !inside_catalog {
        return false;
    }

    if is_catalog_file(path) {
        return true;
    }

    let is_noise = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(is_noise_or_hidden_dir);
    if is_noise {
        return false;
    }

    match kind {
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder) => true,
        EventKind::Create(CreateKind::Any | CreateKind::Other)
        | EventKind::Modify(ModifyKind::Name(_) | ModifyKind::Any | ModifyKind::Other)
        | EventKind::Remove(RemoveKind::Any | RemoveKind::Other)
        | EventKind::Any
        | EventKind::Other => path.is_dir() || filter.catalog_dirs.contains(path),
        EventKind::Create(CreateKind::File)
        | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_))
        | EventKind::Remove(RemoveKind::File)
        | EventKind::Access(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, thread, time::Duration};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "downmark_watcher_{label}_{}_{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn collect_watch_dirs_skips_noise_and_keeps_agent_dirs() {
        let root = unique_temp_dir("collect");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("docs/guides")).unwrap();
        fs::create_dir_all(root.join(".agents/skills/my-skill")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();

        let open_dir = unique_temp_dir("open_parent");
        fs::create_dir_all(&open_dir).unwrap();
        let open_file = open_dir.join("note.md");
        fs::write(&open_file, "# Note\n").unwrap();

        let targets = WatchTargets {
            open_files: vec![open_file],
            recent_files: Vec::new(),
            project_folders: vec![root.clone()],
            global_watch: GlobalWatch::Skip,
        };

        let dirs = collect_watch_dirs(&targets);
        let canon_root = root.canonicalize().unwrap();
        let canon_open_dir = open_dir.canonicalize().unwrap();

        assert!(dirs.file_parents.contains(&canon_open_dir));
        assert!(dirs.catalog_dirs.contains(&canon_root));
        assert!(dirs.catalog_dirs.contains(&canon_root.join("docs/guides")));
        assert!(
            dirs.catalog_dirs
                .contains(&canon_root.join(".agents/skills/my-skill"))
        );
        assert!(!dirs.catalog_dirs.contains(&canon_root.join("target")));
        assert!(!dirs.catalog_dirs.contains(&canon_root.join("target/debug")));
        assert!(!dirs.catalog_dirs.contains(&canon_root.join(".git")));
        assert!(!dirs.catalog_dirs.contains(&canon_root.join("node_modules")));

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&open_dir);
    }

    #[test]
    fn watcher_detects_open_file_edits_and_ignores_non_markdown_project_files() {
        let root = unique_temp_dir("events");
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("proj");
        let open_dir = root.join("open");
        fs::create_dir_all(proj.join("src")).unwrap();
        fs::create_dir_all(&open_dir).unwrap();

        let open_file = open_dir.join("note.txt");
        fs::write(&open_file, "initial").unwrap();
        let rs_file = proj.join("src/main.rs");
        fs::write(&rs_file, "fn main() {}").unwrap();

        let (mut watcher, rx) = DiskWatcher::new();
        assert!(!watcher.is_degraded());

        let targets = WatchTargets {
            open_files: vec![open_file.clone()],
            recent_files: Vec::new(),
            project_folders: vec![proj.clone()],
            global_watch: GlobalWatch::Skip,
        };
        let dirs = collect_watch_dirs(&targets);
        watcher.apply_sync(&targets, &dirs);

        // Give OS watcher a moment to register inotify/kqueue/FSEvents watches.
        thread::sleep(Duration::from_millis(50));

        // Modifying a non-markdown source file inside project should not trigger catalog rescan.
        fs::write(&rs_file, "fn main() { println!(\"hi\"); }").unwrap();
        thread::sleep(Duration::from_millis(120));
        while rx.try_recv().is_ok() {}
        let rs_dispatch = watcher.drain_dispatch();
        assert!(!rs_dispatch.catalog);
        assert!(!rs_dispatch.open_files);

        // External write to open file should trigger open_files dispatch.
        fs::write(&open_file, "updated").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut saw_open = false;
        while std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(40));
            while rx.try_recv().is_ok() {}
            if watcher.drain_dispatch().open_files {
                saw_open = true;
                break;
            }
        }
        assert!(saw_open, "expected open_files change notification");

        // Creating a markdown file in project should trigger catalog dispatch.
        let md_file = proj.join("README.md");
        fs::write(&md_file, "# Hello\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut saw_catalog = false;
        while std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(40));
            while rx.try_recv().is_ok() {}
            if watcher.drain_dispatch().catalog {
                saw_catalog = true;
                break;
            }
        }
        assert!(saw_catalog, "expected catalog change notification");

        // Atomic rename replacement over open file and deletion of markdown file should both notify.
        let tmp_swap = open_dir.join(".note.txt.tmp");
        fs::write(&tmp_swap, "atomic replace").unwrap();
        fs::rename(&tmp_swap, &open_file).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut saw_atomic_open = false;
        while std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(40));
            while rx.try_recv().is_ok() {}
            if watcher.drain_dispatch().open_files {
                saw_atomic_open = true;
                break;
            }
        }
        assert!(
            saw_atomic_open,
            "expected atomic rename open_files notification"
        );

        fs::remove_file(&md_file).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut saw_remove = false;
        while std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(40));
            while rx.try_recv().is_ok() {}
            if watcher.drain_dispatch().catalog {
                saw_remove = true;
                break;
            }
        }
        assert!(saw_remove, "expected markdown removal catalog notification");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_watch_dirs_includes_skills_build_and_skill_symlinks_and_skips_project_build() {
        let root = unique_temp_dir("skills_build");
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("proj");
        let external_skill = root.join("external_skill");
        fs::create_dir_all(proj.join("build")).unwrap();
        fs::create_dir_all(proj.join("skills/build/refs")).unwrap();
        fs::create_dir_all(&external_skill).unwrap();
        fs::write(proj.join("skills/build/SKILL.md"), "# Build skill\n").unwrap();
        fs::write(external_skill.join("SKILL.md"), "# External\n").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&external_skill, proj.join("skills/linked-skill")).unwrap();

        let targets = WatchTargets {
            open_files: Vec::new(),
            recent_files: Vec::new(),
            project_folders: vec![proj.clone()],
            global_watch: GlobalWatch::Skip,
        };
        let dirs = collect_watch_dirs(&targets);
        let canon_proj = proj.canonicalize().unwrap();

        assert!(!dirs.catalog_dirs.contains(&canon_proj.join("build")));
        assert!(dirs.catalog_dirs.contains(&canon_proj.join("skills/build")));
        assert!(
            dirs.catalog_dirs
                .contains(&canon_proj.join("skills/build/refs"))
        );
        #[cfg(unix)]
        {
            let canon_ext = external_skill.canonicalize().unwrap();
            assert!(dirs.catalog_dirs.contains(&canon_ext));
        }

        // Removing a non-Markdown skill symlink in skills/ must classify as catalog change.
        let mut filter = ActiveWatchFilter::default();
        filter.skill_containers.clone_from(&dirs.skill_containers);
        filter.catalog_dirs.clone_from(&dirs.catalog_dirs);
        let dispatch = classify_event(
            EventKind::Remove(RemoveKind::File),
            &[canon_proj.join("skills/linked-skill")],
            &filter,
        );
        assert!(dispatch.catalog);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ancestor_watch_filters_unrelated_home_events_and_matches_target() {
        let root = unique_temp_dir("ancestor");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let canon_root = root.canonicalize().unwrap();

        let missing_codex = root.join(".codex/skills");
        let mut ancestors = HashMap::new();
        register_missing_ancestor(&missing_codex, &mut ancestors);
        assert!(ancestors.contains_key(&canon_root));

        let filter = ActiveWatchFilter {
            catalog_ancestors: ancestors,
            ..Default::default()
        };

        let unrelated = classify_event(
            EventKind::Create(CreateKind::Folder),
            &[canon_root.join("Downloads")],
            &filter,
        );
        assert!(!unrelated.catalog);

        let created_codex = classify_event(
            EventKind::Create(CreateKind::Folder),
            &[canon_root.join(".codex")],
            &filter,
        );
        assert!(created_codex.catalog);

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn walk_non_noise_dirs_marks_incomplete_on_unreadable_dir() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = unique_temp_dir("incomplete");
        let _ = fs::remove_dir_all(&root);
        let unreadable = root.join("secret");
        fs::create_dir_all(&unreadable).unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let actually_denied = matches!(
            fs::read_dir(&unreadable),
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied
        );
        if actually_denied {
            let targets = WatchTargets {
                open_files: Vec::new(),
                recent_files: Vec::new(),
                project_folders: vec![root.clone()],
                global_watch: GlobalWatch::Skip,
            };
            let dirs = collect_watch_dirs(&targets);
            assert!(dirs.incomplete);

            let (mut watcher, _rx) = DiskWatcher::new();
            watcher.apply_sync(&targets, &dirs);
            assert!(watcher.is_degraded());
        }

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o755)).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn collect_watch_dirs_records_file_symlink_at_max_depth_and_dangling_symlink_target() {
        let root = unique_temp_dir("symlink_depth_dangling");
        let _ = fs::remove_dir_all(&root);

        // Build a nested directory at depth == MAX_PROJECT_WATCH_DEPTH (8).
        let mut deep = root.join("proj");
        for i in 1..=MAX_PROJECT_WATCH_DEPTH {
            deep = deep.join(format!("d{i}"));
        }
        let beyond = deep.join("beyond");
        fs::create_dir_all(&beyond).unwrap();

        let ext_dir = root.join("external");
        fs::create_dir_all(&ext_dir).unwrap();
        let ext_file = ext_dir.join("note.md");
        fs::write(&ext_file, "# External\n").unwrap();

        // File symlink at depth == MAX_PROJECT_WATCH_DEPTH should be recorded; `beyond` should not.
        std::os::unix::fs::symlink(&ext_file, deep.join("linked.md")).unwrap();

        // Dangling catalog symlink at project root pointing into `missing_sub/ghost.md`.
        let ghost_target = ext_dir.join("missing_sub/ghost.md");
        std::os::unix::fs::symlink(&ghost_target, root.join("proj/dangling.md")).unwrap();

        let targets = WatchTargets {
            open_files: Vec::new(),
            recent_files: Vec::new(),
            project_folders: vec![root.join("proj")],
            global_watch: GlobalWatch::Skip,
        };
        let dirs = collect_watch_dirs(&targets);

        let canon_ext_dir = ext_dir.canonicalize().unwrap();
        let canon_ext_file = ext_file.canonicalize().unwrap();
        assert!(
            dirs.external_catalog_targets
                .get(&canon_ext_dir)
                .is_some_and(|targets| targets.contains(&canon_ext_file))
        );
        assert!(!dirs.catalog_dirs.contains(&beyond.canonicalize().unwrap()));
        assert!(
            dirs.catalog_ancestors.get(&canon_ext_dir).is_some_and(
                |targets| targets.contains(&canon_ext_dir.join("missing_sub/ghost.md"))
            )
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn deleted_and_recreated_watched_dir_invalidates_and_reregisters() {
        let root = unique_temp_dir("reregister");
        let _ = fs::remove_dir_all(&root);
        let open_dir = root.join("open");
        fs::create_dir_all(&open_dir).unwrap();
        let open_file = open_dir.join("note.md");
        fs::write(&open_file, "v1").unwrap();

        let (mut watcher, rx) = DiskWatcher::new();
        assert!(!watcher.is_degraded());

        let targets = WatchTargets {
            open_files: vec![open_file.clone()],
            recent_files: Vec::new(),
            project_folders: Vec::new(),
            global_watch: GlobalWatch::Skip,
        };
        let dirs = collect_watch_dirs(&targets);
        watcher.apply_sync(&targets, &dirs);
        thread::sleep(Duration::from_millis(50));

        // Delete and immediately recreate the watched directory before drain_dispatch runs.
        fs::remove_dir_all(&open_dir).unwrap();
        fs::create_dir_all(&open_dir).unwrap();
        fs::write(&open_file, "v2").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut saw_resync = false;
        while std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(40));
            while rx.try_recv().is_ok() {}
            let dispatch = watcher.drain_dispatch();
            if dispatch.resync_watches {
                saw_resync = true;
                break;
            }
        }
        assert!(saw_resync, "expected resync_watches after dir replacement");

        // Re-syncing must re-register the replacement directory's new inode watch.
        let dirs = collect_watch_dirs(&targets);
        let outcome = watcher.apply_sync(&targets, &dirs);
        assert!(outcome.added_file_parents);
        thread::sleep(Duration::from_millis(50));

        // Edits in the replacement directory must now be observed.
        fs::write(&open_file, "v3").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut saw_edit = false;
        while std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(40));
            while rx.try_recv().is_ok() {}
            if watcher.drain_dispatch().open_files {
                saw_edit = true;
                break;
            }
        }
        assert!(saw_edit, "expected edit notification after re-registration");

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_with_symlink_and_parent_dir_resolves_target_ancestor() {
        let root = unique_temp_dir("dangling_sym_dotdot");
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("proj");
        let external = root.join("external");
        let actual_dir = external.join("actual_dir");
        fs::create_dir_all(&proj).unwrap();
        fs::create_dir_all(&actual_dir).unwrap();

        std::os::unix::fs::symlink("../external/actual_dir", proj.join("sym_dir")).unwrap();
        std::os::unix::fs::symlink("sym_dir/../missing_sub/ghost.md", proj.join("dangling.md"))
            .unwrap();

        let targets = WatchTargets {
            open_files: Vec::new(),
            recent_files: Vec::new(),
            project_folders: vec![proj.clone()],
            global_watch: GlobalWatch::Skip,
        };
        let dirs = collect_watch_dirs(&targets);
        let canon_external = external.canonicalize().unwrap();
        let canon_proj = proj.canonicalize().unwrap();

        assert!(
            dirs.catalog_ancestors.get(&canon_external).is_some_and(
                |targets| targets.contains(&canon_external.join("missing_sub/ghost.md"))
            ),
            "expected symlink + .. to resolve into external/missing_sub/ghost.md"
        );
        assert!(
            !dirs
                .catalog_ancestors
                .get(&canon_proj)
                .is_some_and(|targets| targets.contains(&canon_proj.join("missing_sub/ghost.md"))),
            "must not resolve symlink + .. lexically under proj/"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_with_missing_prefix_before_parent_dir_keeps_unresolved_component() {
        let root = unique_temp_dir("dangling_sym_missing_dotdot");
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("proj");
        let external = root.join("external");
        fs::create_dir_all(&proj).unwrap();
        fs::create_dir_all(&external).unwrap();

        let canon_external = external.canonicalize().unwrap();
        std::os::unix::fs::symlink(
            canon_external.join("missing/../ghost.md"),
            proj.join("guide.md"),
        )
        .unwrap();

        let targets = WatchTargets {
            open_files: Vec::new(),
            recent_files: Vec::new(),
            project_folders: vec![proj],
            global_watch: GlobalWatch::Skip,
        };
        let dirs = collect_watch_dirs(&targets);

        assert!(
            dirs.catalog_ancestors.get(&canon_external).is_some_and(
                |targets| targets.contains(&canon_external.join("missing/../ghost.md"))
            ),
            "expected catalog ancestor to preserve missing/../ghost.md"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn collect_watch_dirs_handles_symlinked_instructions_and_dangling_rules_dir() {
        let root = unique_temp_dir("rules_dirs_symlinks");
        let _ = fs::remove_dir_all(&root);
        let proj = root.join("proj");
        let ext_inst = root.join("ext_instructions");
        let ext_missing_rules = root.join("ext_missing_rules");
        fs::create_dir_all(proj.join(".github")).unwrap();
        fs::create_dir_all(proj.join(".claude")).unwrap();
        fs::create_dir_all(&ext_inst).unwrap();

        std::os::unix::fs::symlink(&ext_inst, proj.join(".github/instructions")).unwrap();
        std::os::unix::fs::symlink(&ext_missing_rules, proj.join(".claude/rules")).unwrap();

        let targets = WatchTargets {
            open_files: Vec::new(),
            recent_files: Vec::new(),
            project_folders: vec![proj],
            global_watch: GlobalWatch::Skip,
        };
        let dirs = collect_watch_dirs(&targets);

        let canon_ext_inst = ext_inst.canonicalize().unwrap();
        let canon_root = root.canonicalize().unwrap();
        assert!(
            dirs.catalog_dirs.contains(&canon_ext_inst),
            "expected symlinked .github/instructions target to be watched"
        );
        assert!(
            dirs.catalog_ancestors
                .get(&canon_root)
                .is_some_and(|targets| targets.contains(&canon_root.join("ext_missing_rules"))),
            "expected dangling .claude/rules target ancestor to be registered"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
