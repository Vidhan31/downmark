//! The window: title bar, open documents (as tabs when enabled), the file
//! and outline sidebars, and the status bar. It routes commands to the
//! document showing, asks before unsaved changes are lost, notices files
//! changed by other programs, and remembers the session.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gpui_kit::{
    Anchor, App, AppContext as _, Bounds, Context, Entity, EntityId, ExternalPaths, FocusHandle, Focusable,
    FontWeight, Global, InteractiveElement as _, IntoElement, Menu, MouseButton, OwnedMenu,
    OwnedMenuItem, ParentElement as _, PathPromptOptions, Render, ScrollEvent, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window,
    WindowBounds,
    assets::IconName,
    component::{
        ActiveTheme as _, Icon, IndexPath, Sizable as _, TitleBar, WindowExt as _,
        button::{Button, ButtonVariants as _},
        command::{Command, CommandItem, CommandState},
        dialog::DialogFooter,
        h_flex,
        input::{Escape, Search},
        menu::{AppMenuBar, DropdownMenu as _, PopupMenu},
        notification::Notification,
        status_bar::StatusBar,
        tooltip::Tooltip,
        v_flex,
    },
    div, point,
    prelude::FluentBuilder as _,
    px, rems,
};

use crate::{
    actions::{
        About, ActivateTab, CloseDocument, CloseWindow, FindInPreview, FollowLink, GoToHeading,
        NewFile, NewTab, NextTab, Open, OpenRecent, OpenSettings, PreviousTab, Quit,
        RevealInFolder, Save, SaveAs, SetLayout, ToggleAppearance,
        ToggleLineNumbers, TogglePaneFocus, ToggleRestoreSession, ToggleScrollSync, ToggleSoftWrap,
        ToggleTabs, ViewDiagram, WORKSPACE_CONTEXT, ZoomIn, ZoomOut, ZoomReset,
    },
    analysis::{Analysis, content_hash},
    catalog::FolderCatalog,
    diagram_viewer, dialogs,
    document::{self, LineEnding, display_name, is_markdown_path},
    document_view::{DocumentEvent, DocumentView},
    links::{self, LinkTarget},
    new_tab::{NewTabEvent, NewTabView},
    preview_ext,
    session::{self, DiskStamp, Session, SessionDocument, WindowPlacement},
    settings::{Appearance, Layout, LinkPlacement, clamp_font_size},
    settings_dialog,
    themes::{self, AppSettings},
};

/// URL of the link currently hovered or focused in the preview, for status bar display.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct HoveredLink(pub(crate) Option<SharedString>);

impl Global for HoveredLink {}

/// Shown when Downmark starts for the first time.
const WELCOME: &str = include_str!("welcome.md");
/// How often open files and the file sidebar are checked for changes made
/// by other programs.
#[cfg_attr(
    test,
    expect(
        dead_code,
        reason = "periodic timer only loops in live runtime rather than test executor"
    )
)]
const WATCH_INTERVAL: Duration = Duration::from_secs(2);
const WINDOW_CONTROL_WIDTH_PX: f32 = 34.;
const TITLE_BAR_MENU_AND_GAP_WIDTH_PX: f32 = 36.;
const NEW_TAB_BUTTON_WIDTH_PX: f32 = 32.;
const TAB_WIDTH_PX: f32 = 180.;
const TAB_GAP_PX: f32 = 4.;
const MACOS_TITLE_BAR_LEFT_PADDING_PX: f32 = 80.;
const TITLE_BAR_LEFT_PADDING_PX: f32 = 12.;

/// What to do once unsaved changes have been saved or discarded.
#[derive(Clone)]
enum AfterConfirm {
    NewFile,
    OpenDialog,
    OpenPath(PathBuf),
    /// Open a linked document in place of the current one, at a heading.
    OpenLink(PathBuf, Option<String>),
    CloseDocument(EntityId),
    CloseWindow,
    Quit,
}

/// Where a document being opened goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// A new tab when tabs are on, otherwise in place of the current
    /// document.
    Default,
    NewTab,
    ReplaceActive,
}

/// What the watcher found for one open file.
enum DiskChange {
    Missing,
    Changed {
        text: String,
        line_ending: LineEnding,
        stamp: Option<DiskStamp>,
    },
    Touched(Option<DiskStamp>),
}

#[derive(Clone)]
pub(crate) enum WorkspaceTab {
    Document(Entity<DocumentView>),
    NewTab(Entity<NewTabView>),
}

impl WorkspaceTab {
    pub(crate) fn entity_id(&self) -> EntityId {
        match self {
            Self::Document(doc) => doc.entity_id(),
            Self::NewTab(new_tab) => new_tab.entity_id(),
        }
    }

    pub(crate) fn name(&self, cx: &App) -> SharedString {
        match self {
            Self::Document(doc) => doc.read(cx).name().into(),
            Self::NewTab(_) => SharedString::from("Home"),
        }
    }

    pub(crate) fn path<'a>(&'a self, cx: &'a App) -> Option<&'a PathBuf> {
        match self {
            Self::Document(doc) => doc.read(cx).path(),
            Self::NewTab(_) => None,
        }
    }

    pub(crate) fn is_dirty(&self, cx: &App) -> bool {
        match self {
            Self::Document(doc) => doc.read(cx).is_dirty(),
            Self::NewTab(_) => false,
        }
    }

    pub(crate) fn is_blank(&self, cx: &App) -> bool {
        match self {
            Self::Document(doc) => doc.read(cx).is_blank(),
            Self::NewTab(_) => true,
        }
    }

    pub(crate) fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Document(doc) => {
                let doc = doc.read(cx);
                if AppSettings::get(cx).layout.shows_editor() {
                    doc.editor().focus_handle(cx)
                } else {
                    doc.preview().read(cx).focus_handle().clone()
                }
            }
            Self::NewTab(new_tab) => new_tab.read(cx).focus_handle(cx),
        }
    }
}

pub(crate) struct Workspace {
    focus_handle: FocusHandle,
    tabs_scroll_handle: ScrollHandle,
    tabs: Vec<WorkspaceTab>,
    active: usize,
    headings: Entity<CommandState>,
    _app_menu_bar: Entity<AppMenuBar>,

    window_title: String,
    closing: bool,
    /// Documents whose unsaved changes the user chose to discard while
    /// closing.
    discarded: HashSet<EntityId>,
    /// Files changed on disk that are waiting for the user to decide.
    pending_disk_prompt: HashSet<EntityId>,
    last_session: Option<Session>,
    catalog: Entity<FolderCatalog>,
    _watch_task: Task<()>,
    tab_subscriptions: Vec<(EntityId, Vec<Subscription>)>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// A workspace restoring the last session (when enabled) and opening
    /// `paths`.
    pub(crate) fn new(
        paths: Vec<PathBuf>,
        app_menu_bar: Entity<AppMenuBar>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let settings = AppSettings::get(cx).clone();
        let headings = cx.new(|cx| CommandState::new(window, cx));

        let subscriptions = vec![
            cx.observe_global_in::<AppSettings>(window, |this, window, cx| {
                this.on_settings_changed(window, cx);
                cx.notify();
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                if AppSettings::get(cx).appearance == Appearance::System {
                    themes::apply(Some(window), cx);
                }
            }),
            cx.observe_window_bounds(window, |this, window, cx| this.save_session(window, cx)),
        ];

        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| this.request_close(window, cx))
                .unwrap_or(true)
        });

        let watch_task = cx.spawn_in(window, async move |this, cx| {
            #[cfg(not(test))]
            loop {
                cx.background_executor().timer(WATCH_INTERVAL).await;
                if this.update_in(cx, Self::check_disk).is_err() {
                    break;
                }
            }
            #[cfg(test)]
            let _ = this.update_in(cx, Self::check_disk);
        });

        let catalog = cx.new(|_| FolderCatalog::new(settings.tracked_folders.clone()));

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            tabs_scroll_handle: ScrollHandle::new(),
            tabs: Vec::new(),
            active: 0,
            headings,
            _app_menu_bar: app_menu_bar,
            window_title: String::new(),
            closing: false,
            discarded: HashSet::new(),
            pending_disk_prompt: HashSet::new(),
            last_session: None,
            catalog,
            _watch_task: watch_task,
            tab_subscriptions: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.restore(paths, &settings, window, cx);
        this
    }

    /// Reopen the last session's documents and anything recovered after a
    /// crash, then the files asked for.
    fn restore(
        &mut self,
        paths: Vec<PathBuf>,
        settings: &crate::settings::Settings,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let previous = Session::load();
        #[cfg_attr(
            test,
            expect(unused_variables, reason = "tests do not use welcome document")
        )]
        let first_run = previous.is_none();
        let previous = previous.unwrap_or_default();
        let mut active = None;
        let mut referenced = HashSet::new();

        for (ix, entry) in previous.documents.iter().enumerate() {
            let recovery = entry.recovery.as_deref().and_then(|id| {
                referenced.insert(id.to_string());
                session::read_recovery(id).map(|recovery| (id.to_string(), recovery))
            });
            // Unsaved changes always come back; saved documents only when
            // the session is restored (just the active one without tabs).
            let wanted = settings.restore_session
                && paths.is_empty()
                && (settings.tabs || ix == previous.active);
            let doc = match recovery {
                Some((id, recovery)) => Some(Self::restored_document(id, recovery, window, cx)),
                None if wanted => entry.path.as_ref().and_then(|path| {
                    let loaded = document::load(path).ok()?;
                    Some(cx.new(|cx| {
                        DocumentView::new(
                            loaded.text,
                            Some(path.clone()),
                            loaded.line_ending,
                            window,
                            cx,
                        )
                    }))
                }),
                None => None,
            };
            if let Some(doc) = doc {
                let cursor = entry.cursor;
                doc.update(cx, |doc, cx| doc.set_cursor(cursor, window, cx));
                if ix == previous.active {
                    active = Some(self.tabs.len());
                }
                self.add_tab(WorkspaceTab::Document(doc), false, window, cx);
            }
        }

        let orphans = session::orphaned_recoveries(&referenced);
        let recovered = orphans.len() + self.tabs.iter().filter(|tab| tab.is_dirty(cx)).count();
        for (id, recovery) in orphans {
            let doc = Self::restored_document(id, recovery, window, cx);
            self.add_tab(WorkspaceTab::Document(doc), false, window, cx);
        }
        if recovered > 0 {
            let message = if recovered == 1 {
                "Recovered unsaved changes from the last session.".to_string()
            } else {
                format!("Recovered unsaved changes to {recovered} documents.")
            };
            // The window can show notifications once it has finished opening.
            window.defer(cx, move |window, cx| {
                window.push_notification(Notification::info(message), cx);
            });
        }

        self.last_session = Some(previous);

        // Files being opened replace this untouched placeholder.
        if self.tabs.is_empty() {
            #[cfg(not(test))]
            let is_first = first_run;
            #[cfg(test)]
            let is_first = false;

            let tab = if is_first && paths.is_empty() {
                let doc = cx.new(|cx| {
                    DocumentView::new(WELCOME.to_string(), None, LineEnding::Lf, window, cx)
                });
                WorkspaceTab::Document(doc)
            } else {
                let new_tab = cx.new(|cx| NewTabView::new(self.catalog.clone(), window, cx));
                WorkspaceTab::NewTab(new_tab)
            };
            self.add_tab(tab, false, window, cx);
        }
        self.active = active.unwrap_or(0).min(self.tabs.len().saturating_sub(1));
        if !paths.is_empty() {
            self.open_paths(paths, window, cx);
        }
        self.focus_active(window, cx);
    }

    fn restored_document(
        id: String,
        recovery: session::Recovery,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<DocumentView> {
        cx.new(|cx| {
            let mut doc = DocumentView::new(String::new(), None, LineEnding::Lf, window, cx);
            doc.restore_unsaved(id, recovery, window, cx);
            doc
        })
    }

    fn active_tab(&self) -> &WorkspaceTab {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }

    fn active_document(&self) -> Option<Entity<DocumentView>> {
        match self.active_tab() {
            WorkspaceTab::Document(doc) => Some(doc.clone()),
            WorkspaceTab::NewTab(_) => None,
        }
    }

    fn tab_index(&self, id: EntityId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.entity_id() == id)
    }

    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "Internal tab activation flag"
    )]
    fn add_tab(
        &mut self,
        tab: WorkspaceTab,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let subscriptions = match &tab {
            WorkspaceTab::Document(doc) => vec![
                cx.observe(doc, |_, _, cx| cx.notify()),
                cx.subscribe_in(doc, window, |_, _, event, window, cx| match event {
                    DocumentEvent::Notify(message) => {
                        window.push_notification(Notification::info(message.clone()), cx);
                    }
                }),
            ],
            WorkspaceTab::NewTab(new_tab) => vec![
                cx.observe(new_tab, |_, _, cx| cx.notify()),
                cx.subscribe_in(
                    new_tab,
                    window,
                    |this, _, event: &NewTabEvent, window, cx| match event {
                        NewTabEvent::OpenFile(path) => {
                            this.load_path(path.clone(), Placement::Default, None, window, cx);
                        }
                        NewTabEvent::OpenFileInNewTab(path) => {
                            this.load_path(path.clone(), Placement::NewTab, None, window, cx);
                        }
                        NewTabEvent::PromptChooseFolder => {
                            this.prompt_choose_folder(window, cx);
                        }
                        NewTabEvent::PromptOpenFile => {
                            this.prompt_open(window, cx);
                        }
                        NewTabEvent::RemoveFolder(path) => {
                            this.remove_tracked_folder(path, window, cx);
                        }
                        NewTabEvent::Rescan => {
                            this.rescan_catalog(window, cx);
                        }
                    },
                ),
            ],
        };
        self.tab_subscriptions
            .push((tab.entity_id(), subscriptions));
        self.tabs.push(tab);
        if activate {
            self.activate(self.tabs.len() - 1, window, cx);
        }
    }

    fn remove_tab(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(ix) = self.tab_index(id) else {
            return;
        };
        let tab = self.tabs.remove(ix);
        if let WorkspaceTab::Document(doc) = &tab {
            doc.update(cx, |doc, _| doc.discard_recovery());
        }
        self.tab_subscriptions
            .retain(|(subscribed, _)| *subscribed != id);
        if self.tabs.is_empty() {
            let new_tab = cx.new(|cx| NewTabView::new(self.catalog.clone(), window, cx));
            self.add_tab(WorkspaceTab::NewTab(new_tab), false, window, cx);
        }
        if self.active > ix || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.activate(self.active, window, cx);
        self.on_document_closed(window, cx);
    }

    /// Put `tab` in place of the active tab.
    fn replace_active(
        &mut self,
        tab: WorkspaceTab,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let old = self.active_tab().entity_id();
        let position = self.active;
        self.add_tab(tab, false, window, cx);
        let new = self.tabs.pop().expect("just added");
        self.tabs.insert(position, new);
        if let Some(ix) = self.tab_index(old) {
            let removed = self.tabs.remove(ix);
            if let WorkspaceTab::Document(doc) = &removed {
                doc.update(cx, |doc, _| doc.discard_recovery());
            }
            self.tab_subscriptions
                .retain(|(subscribed, _)| *subscribed != old);
        }
        self.activate(position, window, cx);
        self.on_document_closed(window, cx);
    }

    /// Prune unreferenced diagram caches and trim allocator arenas when documents close.
    fn on_document_closed(&self, window: &mut Window, cx: &App) {
        let docs: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                WorkspaceTab::Document(doc) => Some(doc.read(cx)),
                WorkspaceTab::NewTab(_) => None,
            })
            .collect();
        let active_refs: Vec<&str> = docs.iter().map(|doc| doc.source().as_ref()).collect();
        let evicted = preview_ext::SharedDiagramCache::global(cx).prune_unreferenced(&active_refs);
        for image in evicted {
            _ = window.drop_image(image);
        }
        crate::trim_allocator_memory();
    }

    fn scroll_tabs_by(&mut self, dx: gpui_kit::Pixels) {
        let offset = self.tabs_scroll_handle.offset();
        let max = self.tabs_scroll_handle.max_offset().x;
        let new_x = if max > px(0.) {
            (offset.x + dx).clamp(-max, px(0.))
        } else {
            (offset.x + dx).min(px(0.))
        };
        self.tabs_scroll_handle.set_offset(point(new_x, offset.y));
    }

    fn scroll_tab_into_view(&mut self, ix: usize) {
        self.tabs_scroll_handle.scroll_to_item(ix);
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.active = ix.min(self.tabs.len() - 1);
        self.scroll_tab_into_view(self.active);
        self.focus_active(window, cx);
        self.save_session(window, cx);
        cx.notify();
    }

    fn focus_active(&self, window: &mut Window, cx: &mut App) {
        let focus = self.active_tab().focus_handle(cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));
    }

    fn active_tab_focus_handle(&self, cx: &App) -> FocusHandle {
        self.active_tab().focus_handle(cx)
    }

    /// Open `paths`: as tabs when tabs are on, otherwise in place of the
    /// current document (asking about unsaved changes first).
    fn open_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<'_, Self>) {
        let tabs = AppSettings::get(cx).tabs;
        for path in paths {
            let path = path.canonicalize().unwrap_or(path);
            if let Some(ix) = self.tabs.iter().position(|tab| tab.path(cx) == Some(&path)) {
                self.activate(ix, window, cx);
                continue;
            }
            let active = self.active_tab();
            if !tabs
                && active.is_dirty(cx)
                && let WorkspaceTab::Document(doc) = active
            {
                self.confirm_unsaved(doc.clone(), AfterConfirm::OpenPath(path), window, cx);
                return;
            }
            self.load_path(path, Placement::Default, None, window, cx);
        }
    }

    /// Read `path` in the background and show it where `placement` says,
    /// scrolled to the heading `anchor` names, if any.
    #[expect(clippy::unused_self, reason = "GPUI view method consistency")]
    fn load_path(
        &mut self,
        path: PathBuf,
        placement: Placement,
        anchor: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let read_path = path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { document::load(&read_path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            _ = this.update_in(cx, |this, window, cx| match result {
                Ok(loaded) => {
                    AppSettings::update(cx, |settings| settings.push_recent(&path));
                    let doc = cx.new(|cx| {
                        DocumentView::new(
                            loaded.text,
                            Some(path.clone()),
                            loaded.line_ending,
                            window,
                            cx,
                        )
                    });
                    let active = this.active_tab();
                    let replace = active.is_blank(cx)
                        || match placement {
                            Placement::Default => !AppSettings::get(cx).tabs,
                            Placement::NewTab => false,
                            Placement::ReplaceActive => true,
                        };
                    let tab = WorkspaceTab::Document(doc.clone());
                    if replace {
                        this.replace_active(tab, window, cx);
                    } else {
                        this.add_tab(tab, true, window, cx);
                    }
                    if let Some(anchor) = anchor {
                        this.reveal_anchor(&doc, &anchor, window, cx);
                    }
                }
                Err(err) => {
                    AppSettings::update(cx, |settings| {
                        settings.recent_files.retain(|recent| recent != &path);
                    });
                    this.notify_error(
                        format!("Couldn’t open “{}”. {err}", display_name(Some(&path))),
                        window,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn new_document(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let doc = cx.new(|cx| DocumentView::new(String::new(), None, LineEnding::Lf, window, cx));
        let tab = WorkspaceTab::Document(doc);
        if AppSettings::get(cx).tabs {
            self.add_tab(tab, true, window, cx);
        } else {
            self.replace_active(tab, window, cx);
        }
    }

    fn new_tab_action(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.new_tab(true, window, cx);
    }

    #[expect(clippy::fn_params_excessive_bools, reason = "Tab activation flag")]
    fn new_tab(&mut self, activate: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let new_tab = cx.new(|cx| NewTabView::new(self.catalog.clone(), window, cx));
        let tab = WorkspaceTab::NewTab(new_tab);
        if AppSettings::get(cx).tabs {
            self.add_tab(tab, activate, window, cx);
        } else {
            self.replace_active(tab, window, cx);
        }
    }

    #[expect(
        clippy::unused_self,
        reason = "Spawns async task with weak entity handle"
    )]
    fn prompt_choose_folder(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose Folder to Scan for Markdown Files".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            _ = this.update_in(cx, |this, _window, cx| {
                for path in paths {
                    let path = path.canonicalize().unwrap_or(path);
                    if path.is_dir() {
                        AppSettings::update(cx, |settings| {
                            if !settings.tracked_folders.contains(&path) {
                                settings.tracked_folders.push(path.clone());
                            }
                        });
                        this.catalog.update(cx, |catalog, cx| {
                            catalog.add_folder(path);
                            cx.notify();
                        });
                    }
                }
            });
        })
        .detach();
    }

    fn remove_tracked_folder(
        &mut self,
        path: &Path,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let path = path.to_path_buf();
        AppSettings::update(cx, |settings| {
            settings.tracked_folders.retain(|f| f != &path);
        });
        self.catalog.update(cx, |catalog, cx| {
            catalog.remove_folder(&path);
            cx.notify();
        });
    }

    fn rescan_catalog(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let catalog = self.catalog.clone();
        cx.spawn_in(window, async move |_this, cx| {
            catalog.update(cx, |catalog, cx| {
                catalog.rescan();
                cx.notify();
            });
        })
        .detach();
    }

    #[expect(clippy::unused_self, reason = "Workspace notification method")]
    fn notify_error(&self, message: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
        window.push_notification(Notification::error(message), cx);
    }

    fn update_window_title(&mut self, window: &mut Window, cx: &App) {
        let title = match self.active_tab() {
            WorkspaceTab::NewTab(_) => "Home — Downmark".to_string(),
            WorkspaceTab::Document(doc) => {
                let doc = doc.read(cx);
                format!(
                    "{}{} — Downmark",
                    doc.name(),
                    if doc.is_dirty() { " (edited)" } else { "" }
                )
            }
        };
        if title != self.window_title {
            window.set_window_title(&title);
            window.set_window_edited(self.tabs.iter().any(|tab| tab.is_dirty(cx)));
            self.window_title = title;
        }
    }

    fn on_settings_changed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.save_session(window, cx);
    }

    #[expect(
        clippy::unused_self,
        reason = "Convenience method for workspace setting toggles"
    )]
    fn toggle(&mut self, update: impl FnOnce(&mut crate::settings::Settings), cx: &mut App) {
        AppSettings::update(cx, update);
    }

    /// Remember the open documents, folder and window, when anything about
    /// them changed.
    fn save_session(&mut self, window: &mut Window, cx: &mut App) {
        if self.closing && self.last_session.is_none() {
            return;
        }
        let documents = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                WorkspaceTab::Document(doc) => {
                    let doc = doc.read(cx);
                    let recovery = doc
                        .is_dirty()
                        .then(|| doc.recovery_id().map(str::to_string))
                        .flatten();
                    (doc.path().is_some() || recovery.is_some()).then(|| SessionDocument {
                        path: doc.path().cloned(),
                        cursor: doc.cursor(cx),
                        recovery,
                    })
                }
                WorkspaceTab::NewTab(_) => None,
            })
            .collect::<Vec<_>>();
        let active_id = self.active_tab().entity_id();
        let active = self
            .tabs
            .iter()
            .filter(|tab| match tab {
                WorkspaceTab::Document(doc) => {
                    let doc = doc.read(cx);
                    doc.path().is_some() || doc.recovery_id().is_some()
                }
                WorkspaceTab::NewTab(_) => false,
            })
            .position(|tab| tab.entity_id() == active_id)
            .unwrap_or(0);
        let (bounds, maximized) = match window.window_bounds() {
            WindowBounds::Windowed(bounds) => (bounds, false),
            WindowBounds::Maximized(bounds) | WindowBounds::Fullscreen(bounds) => (bounds, true),
        };
        let session = Session {
            documents,
            active,
            window: Some(WindowPlacement {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                width: f32::from(bounds.size.width),
                height: f32::from(bounds.size.height),
                maximized,
            }),
        };
        if self.last_session.as_ref() != Some(&session) {
            session.save();
            self.last_session = Some(session);
        }
    }

    /// Look for open files other programs changed, moved or deleted.
    fn check_disk(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.save_session(window, cx);

        let targets: Vec<(EntityId, PathBuf, Option<DiskStamp>, u64)> = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                WorkspaceTab::Document(doc) => {
                    let id = doc.entity_id();
                    let doc = doc.read(cx);
                    let path = doc.path()?.clone();
                    (!self.pending_disk_prompt.contains(&id))
                        .then(|| (id, path, doc.disk_stamp(), doc.saved_hash()))
                }
                WorkspaceTab::NewTab(_) => None,
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let task = cx.background_executor().spawn(async move {
            targets
                .into_iter()
                .filter_map(|(id, path, known, saved_hash)| {
                    let stamp = DiskStamp::of(&path);
                    if stamp == known {
                        return None;
                    }
                    let change = match stamp {
                        None if known.is_some() => DiskChange::Missing,
                        None => return None,
                        Some(_) => match document::load(&path) {
                            Ok(loaded) if content_hash(&loaded.text) == saved_hash => {
                                DiskChange::Touched(stamp)
                            }
                            Ok(loaded) => DiskChange::Changed {
                                text: loaded.text,
                                line_ending: loaded.line_ending,
                                stamp,
                            },
                            Err(_) => return None,
                        },
                    };
                    Some((id, change))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let changes = task.await;
            _ = this.update_in(cx, |this, window, cx| {
                for (id, change) in changes {
                    this.apply_disk_change(id, change, window, cx);
                }
            });
        })
        .detach();
    }

    fn apply_disk_change(
        &mut self,
        id: EntityId,
        change: DiskChange,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(doc) = self.tab_index(id).and_then(|ix| match &self.tabs[ix] {
            WorkspaceTab::Document(doc) => Some(doc.clone()),
            WorkspaceTab::NewTab(_) => None,
        }) else {
            return;
        };
        let name = doc.read(cx).name();
        match change {
            DiskChange::Touched(stamp) => doc.update(cx, |doc, _| doc.set_disk_stamp(stamp)),
            DiskChange::Missing => {
                doc.update(cx, DocumentView::mark_missing);
                window.push_notification(
                    Notification::warning(format!(
                        "“{name}” was moved or deleted. Save it to keep your text."
                    )),
                    cx,
                );
            }
            DiskChange::Changed {
                text,
                line_ending,
                stamp,
            } => {
                if !doc.read(cx).is_dirty() {
                    doc.update(cx, |doc, cx| doc.reload(text, line_ending, window, cx));
                    return;
                }
                if window.has_active_dialog(cx) {
                    // Ask once the current dialog is answered.
                    return;
                }
                doc.update(cx, |doc, _| doc.set_disk_stamp(stamp));
                self.pending_disk_prompt.insert(id);
                if let Some(ix) = self.tab_index(id) {
                    self.activate(ix, window, cx);
                }
                let workspace = cx.weak_entity();
                let text = Arc::new(text);
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let (keep, reload) = (workspace.clone(), workspace.clone());
                    let (reload_doc, text) = (doc.clone(), Arc::clone(&text));
                    alert
                        .title(format!("“{name}” changed on disk"))
                        .description(
                            "Another program saved this file while you have unsaved changes here.",
                        )
                        .footer(
                            DialogFooter::new()
                                .justify_end()
                                .child(
                                    Button::new("reload")
                                        .label("Use the file on disk")
                                        .outline()
                                        .on_click(move |_, window, cx| {
                                            window.close_dialog(cx);
                                            let text = text.as_ref().clone();
                                            reload_doc.update(cx, |doc, cx| {
                                                doc.reload(text, line_ending, window, cx);
                                            });
                                            _ = reload.update(cx, |this, cx| {
                                                this.pending_disk_prompt.remove(&id);
                                                this.focus_active(window, cx);
                                            });
                                        }),
                                )
                                .child(
                                    Button::new("keep")
                                        .label("Keep my changes")
                                        .primary()
                                        .on_click(move |_, window, cx| {
                                            window.close_dialog(cx);
                                            _ = keep.update(cx, |this, cx| {
                                                this.pending_disk_prompt.remove(&id);
                                                this.focus_active(window, cx);
                                            });
                                        }),
                                ),
                        )
                });
            }
        }
    }

    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<'_, Self>) {
        if AppSettings::get(cx).tabs {
            self.new_document(window, cx);
        } else if let Some(doc) = self.active_document() {
            self.confirm_unsaved(doc, AfterConfirm::NewFile, window, cx);
        } else {
            self.new_document(window, cx);
        }
    }

    fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<'_, Self>) {
        if AppSettings::get(cx).tabs {
            self.prompt_open(window, cx);
        } else if let Some(doc) = self.active_document() {
            self.confirm_unsaved(doc, AfterConfirm::OpenDialog, window, cx);
        } else {
            self.prompt_open(window, cx);
        }
    }

    fn open_recent(
        &mut self,
        action: &OpenRecent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(path) = AppSettings::get(cx).recent_files.get(action.0).cloned() else {
            return;
        };
        self.open_paths(vec![path], window, cx);
    }

    fn close_document(
        &mut self,
        _: &CloseDocument,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.tabs.len() == 1 && matches!(self.tabs[0], WorkspaceTab::NewTab(_)) {
            self.confirm_close(AfterConfirm::CloseWindow, window, cx);
            return;
        }
        match self.active_tab().clone() {
            WorkspaceTab::Document(doc) => {
                let id = doc.entity_id();
                self.confirm_unsaved(doc, AfterConfirm::CloseDocument(id), window, cx);
            }
            WorkspaceTab::NewTab(new_tab) => {
                let id = new_tab.entity_id();
                self.remove_tab(id, window, cx);
            }
        }
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.confirm_close(AfterConfirm::CloseWindow, window, cx);
    }

    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.confirm_close(AfterConfirm::Quit, window, cx);
    }

    /// Called by the platform before the window closes.
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if self.closing {
            return true;
        }
        let unsaved = self
            .tabs
            .iter()
            .any(|tab| tab.is_dirty(cx) && !self.discarded.contains(&tab.entity_id()));
        if unsaved {
            self.confirm_close(AfterConfirm::CloseWindow, window, cx);
            return false;
        }
        self.save_session(window, cx);
        self.closing = true;
        true
    }

    /// Close the window or quit once every document with unsaved changes
    /// has been saved or discarded, asking about each in turn.
    fn confirm_close(
        &mut self,
        next: AfterConfirm,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let pending = self.tabs.iter().find_map(|tab| match tab {
            WorkspaceTab::Document(doc)
                if doc.read(cx).is_dirty() && !self.discarded.contains(&doc.entity_id()) =>
            {
                Some(doc.clone())
            }
            _ => None,
        });
        match pending {
            Some(doc) => {
                if let Some(ix) = self.tab_index(doc.entity_id()) {
                    self.activate(ix, window, cx);
                }
                self.confirm_unsaved(doc, next, window, cx);
            }
            None => self.run_after_confirm(next, window, cx),
        }
    }

    /// Run `next` now, or once the user has saved or discarded `doc`'s
    /// changes.
    fn confirm_unsaved(
        &mut self,
        doc: Entity<DocumentView>,
        next: AfterConfirm,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !doc.read(cx).is_dirty() {
            self.run_after_confirm(next, window, cx);
            return;
        }
        if window.has_active_dialog(cx) {
            return;
        }

        let name = doc.read(cx).name();
        let workspace = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (save, discard, cancel) = (workspace.clone(), workspace.clone(), workspace.clone());
            let (save_next, discard_next) = (next.clone(), next.clone());
            let (save_doc, discard_doc) = (doc.clone(), doc.clone());
            alert
                .title(format!("Save changes to “{name}”?"))
                .description("Your changes will be lost if you don’t save them.")
                .footer(
                    DialogFooter::new()
                        .justify_end()
                        .child(
                            Button::new("discard")
                                .label("Don’t save")
                                .outline()
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let next = discard_next.clone();
                                    let id = discard_doc.entity_id();
                                    discard_doc.update(cx, |doc, _| doc.discard_recovery());
                                    _ = discard.update(cx, |this, cx| {
                                        this.discarded.insert(id);
                                        this.focus_active(window, cx);
                                        this.run_after_confirm(next, window, cx);
                                    });
                                }),
                        )
                        .child(div().flex_1())
                        .child(Button::new("cancel").label("Cancel").outline().on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                _ = cancel.update(cx, |this, cx| {
                                    this.discarded.clear();
                                    this.focus_active(window, cx);
                                });
                            },
                        ))
                        .child(Button::new("save").label("Save").primary().on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let next = save_next.clone();
                                let doc = save_doc.clone();
                                _ = save.update(cx, |this, cx| {
                                    this.focus_active(window, cx);
                                    this.save_document(doc, Some(next), window, cx);
                                });
                            },
                        )),
                )
        });
    }

    fn run_after_confirm(
        &mut self,
        next: AfterConfirm,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match next {
            AfterConfirm::NewFile => self.new_document(window, cx),
            AfterConfirm::OpenDialog => self.prompt_open(window, cx),
            AfterConfirm::OpenPath(path) => {
                self.load_path(path, Placement::Default, None, window, cx);
            }
            AfterConfirm::OpenLink(path, anchor) => {
                self.load_path(path, Placement::ReplaceActive, anchor, window, cx);
            }
            AfterConfirm::CloseDocument(id) => {
                self.discarded.remove(&id);
                self.remove_tab(id, window, cx);
            }
            AfterConfirm::CloseWindow | AfterConfirm::Quit => {
                // More documents may still be waiting to be asked about.
                let pending = self
                    .tabs
                    .iter()
                    .any(|tab| tab.is_dirty(cx) && !self.discarded.contains(&tab.entity_id()));
                if pending {
                    self.confirm_close(next, window, cx);
                    return;
                }
                self.save_session(window, cx);
                self.closing = true;
                if matches!(next, AfterConfirm::Quit) {
                    cx.quit();
                } else {
                    window.remove_window();
                }
            }
        }
    }

    /// Follow a link clicked in the active document's preview.
    fn follow_link(
        &mut self,
        action: &FollowLink,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(doc) = self.active_document() else {
            return;
        };
        let base_dir = doc.read(cx).dir();
        let Some(target) = links::classify(&action.url, base_dir.as_deref()) else {
            return;
        };
        match target {
            LinkTarget::Heading(anchor) => self.reveal_anchor(&doc, &anchor, window, cx),
            LinkTarget::External(url) => {
                if AppSettings::get(cx).confirm_web_links {
                    dialogs::confirm_external_link(url, window, cx);
                } else {
                    cx.open_url(&url);
                }
            }
            LinkTarget::NeedsSavedDocument => self.notify_error(
                "Save this document first, so Downmark knows where the files it links to are.",
                window,
                cx,
            ),
            LinkTarget::File { path, anchor } => {
                if !path.exists() {
                    // Name the file as the link does, where it can.
                    let shown = base_dir
                        .as_deref()
                        .and_then(|dir| path.strip_prefix(dir).ok())
                        .unwrap_or(&path);
                    self.notify_error(format!("Couldn’t find “{}”.", shown.display()), window, cx);
                } else if path.is_file() && is_markdown_path(&path) {
                    let mut new_tab = AppSettings::get(cx).open_links_in == LinkPlacement::NewTab;
                    if action.alternate {
                        new_tab = !new_tab;
                    }
                    self.open_linked_document(path, anchor, new_tab, window, cx);
                } else {
                    dialogs::confirm_open_file(path, window, cx);
                }
            }
        }
    }

    /// Show a diagram of the active document in a modal.
    fn view_diagram(
        &mut self,
        action: &ViewDiagram,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(doc) = self.active_document() else {
            return;
        };
        let name = doc.read(cx).name();
        diagram_viewer::open(&action.0, format!("Diagram — {name}"), window, cx);
    }

    /// Show the Markdown file `path` (in a new tab, or in place of the
    /// active document), scrolled to `anchor`.
    #[expect(clippy::fn_params_excessive_bools, reason = "Link tab placement flag")]
    fn open_linked_document(
        &mut self,
        path: PathBuf,
        anchor: Option<String>,
        new_tab: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let path = path.canonicalize().unwrap_or(path);
        if let Some(ix) = self.tabs.iter().position(|tab| tab.path(cx) == Some(&path)) {
            self.activate(ix, window, cx);
            if let Some(anchor) = anchor
                && let WorkspaceTab::Document(doc) = &self.tabs[ix]
            {
                let doc = doc.clone();
                self.reveal_anchor(&doc, &anchor, window, cx);
            }
            return;
        }
        if new_tab {
            self.load_path(path, Placement::NewTab, anchor, window, cx);
        } else if let Some(doc) = self.active_document() {
            self.confirm_unsaved(doc, AfterConfirm::OpenLink(path, anchor), window, cx);
        } else {
            self.load_path(path, Placement::Default, anchor, window, cx);
        }
    }

    /// Scroll `doc` to the heading `anchor` names, once it is laid out.
    #[expect(clippy::unused_self, reason = "GPUI view method consistency")]
    fn reveal_anchor(
        &mut self,
        doc: &Entity<DocumentView>,
        anchor: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let doc = doc.clone();
        let anchor = anchor.to_string();
        window.defer(cx, move |window, cx| {
            doc.update(cx, |doc, cx| doc.reveal_anchor(&anchor, window, cx));
        });
    }

    #[expect(
        clippy::unused_self,
        reason = "Spawns async task with weak entity handle"
    )]
    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: AppSettings::get(cx).tabs,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            _ = this.update_in(cx, |this, window, cx| {
                for path in paths {
                    this.load_path(path, Placement::Default, None, window, cx);
                }
            });
        })
        .detach();
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(doc) = self.active_document() else {
            return;
        };
        self.save_document(doc, None, window, cx);
    }

    fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(doc) = self.active_document() else {
            return;
        };
        self.prompt_save_path(doc, None, window, cx);
    }

    /// Save `doc`, asking for a location first when it has none, then
    /// continue with `next`.
    fn save_document(
        &mut self,
        doc: Entity<DocumentView>,
        next: Option<AfterConfirm>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match doc.read(cx).path().cloned() {
            Some(path) => {
                if self.write(&doc, &path, window, cx)
                    && let Some(next) = next
                {
                    self.run_after_confirm(next, window, cx);
                }
            }
            None => self.prompt_save_path(doc, next, window, cx),
        }
    }

    #[expect(
        clippy::unused_self,
        reason = "Spawns async task with weak entity handle"
    )]
    fn prompt_save_path(
        &mut self,
        doc: Entity<DocumentView>,
        next: Option<AfterConfirm>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (dir, name) = {
            let doc = doc.read(cx);
            let dir = default_dir(doc.dir());
            let name = match doc.path() {
                Some(_) => doc.name(),
                None => suggested_file_name(doc.analysis(), "md"),
            };
            (dir, name)
        };
        let chosen = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            _ = this.update_in(cx, |this, window, cx| {
                if this.write(&doc, &path, window, cx) {
                    AppSettings::update(cx, |settings| settings.push_recent(&path));
                    if let Some(next) = next {
                        this.run_after_confirm(next, window, cx);
                    }
                }
            });
        })
        .detach();
    }

    /// Write `doc` to `path`; reports failures to the user.
    fn write(
        &mut self,
        doc: &Entity<DocumentView>,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        match doc.update(cx, |doc, cx| doc.write_to(path, cx)) {
            Ok(()) => {
                self.save_session(window, cx);
                true
            }
            Err(err) => {
                self.notify_error(
                    format!("Couldn’t save “{}”. {err}", display_name(Some(path))),
                    window,
                    cx,
                );
                false
            }
        }
    }

    #[expect(clippy::unused_self, reason = "GPUI action handler")]
    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<'_, Self>) {
        settings_dialog::open_settings(window, cx);
    }

    fn reveal_in_folder(
        &mut self,
        _: &RevealInFolder,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(doc) = self.active_document() else {
            return;
        };
        match doc.read(cx).path().cloned() {
            Some(path) => cx.reveal_path(&path),
            None => window.push_notification(
                Notification::info("Save the document to give it a location."),
                cx,
            ),
        }
    }

    fn on_drop_paths(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let documents: Vec<PathBuf> = paths
            .paths()
            .iter()
            .filter(|path| is_markdown_path(path))
            .cloned()
            .collect();
        if !documents.is_empty() {
            self.open_paths(documents, window, cx);
            return;
        }
        if AppSettings::get(cx).layout.shows_editor() && let Some(doc) = self.active_document() {
            let paths = paths.paths().to_vec();
            doc.update(cx, |doc, cx| doc.drop_images(&paths, window, cx));
            return;
        }
        self.notify_error("Drop a Markdown or text file to open it.", window, cx);
    }

    fn set_layout(&mut self, action: &SetLayout, window: &mut Window, cx: &mut Context<'_, Self>) {
        let layout = action.0;
        AppSettings::update(cx, |settings| {
            settings.layout = layout;
        });
        self.focus_active(window, cx);
    }

    fn toggle_pane_focus(
        &mut self,
        _: &TogglePaneFocus,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(doc) = self.active_document() else {
            return;
        };
        let settings = AppSettings::get(cx);
        if settings.layout == Layout::Split {
            doc.update(cx, |this, cx| {
                this.toggle_pane_focus(window, cx);
            });
        } else if settings.layout == Layout::Editor {
            self.set_layout(&SetLayout(Layout::Split), window, cx);
            doc.update(cx, |this, cx| {
                this.focus_preview(window, cx);
            });
        } else if settings.layout == Layout::Preview {
            self.set_layout(&SetLayout(Layout::Split), window, cx);
            doc.update(cx, |this, cx| {
                this.focus_editor(window, cx);
            });
        }
    }

    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "Explicit closure required for window dialog closing"
    )]
    fn go_to_heading(&mut self, _: &GoToHeading, window: &mut Window, cx: &mut Context<'_, Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let Some(doc) = self.active_document() else {
            return;
        };
        self.headings
            .update(cx, |state, cx| state.set_query("", window, cx));
        let headings = self.headings.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let focus = headings.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| focus.focus(window, cx));
            let outline = doc.read(cx).analysis().outline.clone();
            let items = outline.iter().map(|heading| {
                CommandItem::new()
                    .label(heading.text.clone())
                    .keywords([heading.text.clone()])
                    .icon(heading_icon(heading.level))
            });
            let confirm = doc.clone();
            dialog.close_button(false).p_0().child(
                Command::new(&headings)
                    .bordered(false)
                    .placeholder("Go to heading")
                    .max_h(rems(24.))
                    .items(items)
                    .empty(|_, _, _| "No headings in this document")
                    .on_confirm(move |index: IndexPath, window, cx| {
                        window.close_dialog(cx);
                        confirm.update(cx, |doc, cx| doc.reveal_heading(index.row, window, cx));
                    })
                    .on_cancel(|window, cx| window.close_dialog(cx)),
            )
        });
    }

    #[expect(clippy::unused_self, reason = "GPUI action handler")]
    fn zoom(&mut self, delta: f32, cx: &mut Context<'_, Self>) {
        AppSettings::update(cx, |settings| {
            settings.font_size = if delta == 0. {
                crate::settings::DEFAULT_FONT_SIZE
            } else {
                clamp_font_size(settings.font_size + delta)
            };
        });
    }

    #[expect(clippy::unused_self, reason = "GPUI action handler")]
    fn toggle_appearance(
        &mut self,
        _: &ToggleAppearance,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let dark = cx.theme().is_dark();
        AppSettings::update(cx, |settings| {
            settings.appearance = if dark {
                Appearance::Light
            } else {
                Appearance::Dark
            };
        });
    }

    fn render_title_bar(&self, window: &Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let workspace = cx.weak_entity();
        let supported_controls = window.window_controls();
        let mut controls_width = px(0.);
        if !cfg!(target_os = "macos") && !cfg!(target_family = "wasm") {
            controls_width += px(WINDOW_CONTROL_WIDTH_PX);
            if supported_controls.minimize {
                controls_width += px(WINDOW_CONTROL_WIDTH_PX);
            }
            if supported_controls.maximize {
                controls_width += px(WINDOW_CONTROL_WIDTH_PX);
            }
        }

        let left_padding = if cfg!(target_os = "macos") && !window.is_fullscreen() {
            px(MACOS_TITLE_BAR_LEFT_PADDING_PX)
        } else {
            px(TITLE_BAR_LEFT_PADDING_PX)
        };
        // Maximized/fullscreen inner bounds report the restore size. Use live
        // bounds there, and inner bounds for windowed layouts with decorations.
        let window_width = if window.is_maximized() || window.is_fullscreen() {
            window.bounds().size.width
        } else {
            window.inner_window_bounds().get_bounds().size.width
        };
        let available_tabs_width = window_width
            - left_padding
            - px(TITLE_BAR_MENU_AND_GAP_WIDTH_PX)
            - controls_width
            - px(NEW_TAB_BUTTON_WIDTH_PX);
        let available_tabs_width = if available_tabs_width > px(0.) {
            available_tabs_width
        } else {
            px(0.)
        };
        let single_tab_width = px(TAB_WIDTH_PX);
        let tab_count = u16::try_from(self.tabs.len()).unwrap_or(u16::MAX);
        let tab_count_f32 = f32::from(tab_count);
        let tab_gaps_f32 = f32::from(tab_count.saturating_sub(1));
        let tabs_content_width = single_tab_width * tab_count_f32 + px(TAB_GAP_PX) * tab_gaps_f32;
        let visible_tabs_width = available_tabs_width.min(tabs_content_width);

        TitleBar::new()
            .w_full()
            .on_close_window(move |_, window, cx| {
                _ = workspace.update(cx, |this, cx| {
                    this.confirm_close(AfterConfirm::CloseWindow, window, cx);
                });
            })
            .child(
                h_flex()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .max_w_full()
                    .overflow_hidden()
                    .items_center()
                    .gap_1()
                    .child(self.render_app_menu_button(cx))
                    .child(self.render_tabs(
                        visible_tabs_width + px(NEW_TAB_BUTTON_WIDTH_PX),
                        single_tab_width,
                        cx,
                    )),
            )
    }

    fn render_app_menu_button(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let action_context = self.active_tab_focus_handle(cx);
        Button::new("title-bar-menu")
            .accessibility_label("Application menu")
            .ghost()
            .small()
            .compact()
            .icon(Icon::new(IconName::Menu))
            .tooltip("Menu")
            .dropdown_menu(move |menu, window, cx| {
                let action_context = action_context.clone();
                let mut menu = menu.action_context(action_context.clone());

                let menus: Vec<OwnedMenu> = crate::menus::build(cx)
                    .into_iter()
                    .map(Menu::owned)
                    .collect();
                for app_menu in menus {
                    let items = app_menu.items;
                    let ac = action_context.clone();
                    menu = menu.submenu(app_menu.name, window, cx, move |sub, window, cx| {
                        let sub = sub.action_context(ac.clone());
                        populate_popup_from_menu_items(sub, items.clone(), &ac, window, cx)
                    });
                }

                menu
            })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Declarative tab bar layout and styling"
    )]
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "GPUI component style builder"
    )]
    fn render_tabs(
        &self,
        width: gpui_kit::Pixels,
        tab_width: gpui_kit::Pixels,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .id("tabs-wrapper")
            .h_full()
            .items_center()
            .w(width)
            .flex_none()
            .min_w_0()
            .max_w_full()
            .overflow_hidden()
            .relative()
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height());
                let scroll_dx = if delta.x.abs() > delta.y.abs() {
                    delta.x
                } else {
                    delta.y
                };
                this.scroll_tabs_by(scroll_dx);
                cx.notify();
            }))
            .child(
                h_flex()
                    .id("tabs")
                    .role(gpui_kit::Role::TabList)
                    .aria_label("Document tabs")
                    .h_full()
                    .flex_1()
                    .items_center()
                    .gap(px(TAB_GAP_PX))
                    .min_w_0()
                    .max_w_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.tabs_scroll_handle)
                    .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                        let active = ix == self.active;
                        let dirty = tab.is_dirty(cx);
                        let tab_name = tab.name(cx);
                        let tab_path = tab.path(cx).cloned();

                        let tab_tooltip = format_tab_tooltip(&tab_name, tab_path.as_deref(), dirty);

                        let tab_icon = match tab {
                            WorkspaceTab::Document(_) => IconName::FileText,
                            WorkspaceTab::NewTab(_) => IconName::House,
                        };

                        h_flex()
                            .id(("tab", ix))
                            .group("tab")
                            .role(gpui_kit::Role::Tab)
                            .aria_selected(active)
                            .aria_label(format!("Tab: {tab_name}"))
                            .h(px(28.))
                            .w(tab_width)
                            .flex_none()
                            .px_2()
                            .pr(px(18.))
                            .gap_1p5()
                            .items_center()
                            .rounded_md()
                            .text_sm()
                            .cursor_pointer()
                            .relative()
                            .overflow_hidden()
                            .when(active, |this| {
                                this.bg(theme.tab_active)
                                    .text_color(theme.tab_active_foreground)
                                    .font_weight(FontWeight::MEDIUM)
                                    .border_1()
                                    .border_color(theme.border)
                                    .shadow_sm()
                            })
                            .when(!active, |this| {
                                this.bg(theme.transparent)
                                    .text_color(theme.muted_foreground)
                                    .border_1()
                                    .border_color(theme.transparent)
                                    .hover(|style| {
                                        style.bg(theme.list_hover).text_color(theme.foreground)
                                    })
                            })
                            .tooltip(move |window, cx| {
                                Tooltip::new(tab_tooltip.clone()).build(window, cx)
                            })
                            .child(
                                Icon::new(tab_icon)
                                    .xsmall()
                                    .flex_none()
                                    .text_color(if active {
                                        theme.primary
                                    } else {
                                        theme.muted_foreground
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(tab_name.clone()),
                            )
                            .child(
                                div()
                                    .id(("workspace-tab-close", ix))
                                    .role(gpui_kit::Role::Button)
                                    .aria_label(format!("Close tab {tab_name}"))
                                    .tooltip(|window, cx| {
                                        Tooltip::new(if cfg!(target_os = "macos") {
                                            "Close tab (Cmd+W)"
                                        } else {
                                            "Close tab (Ctrl+W)"
                                        })
                                        .build(window, cx)
                                    })
                                    .absolute()
                                    .right_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(px(22.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .hover(|style| style.bg(theme.list_active))
                                    .child(div().when(dirty, |this| {
                                        this.child(
                                            div().size(px(7.)).rounded_full().bg(theme.primary),
                                        )
                                        .group_hover("tab", |style| style.invisible())
                                    }))
                                    .child(
                                        div()
                                            .absolute()
                                            .invisible()
                                            .when(active && !dirty, |this| this.visible())
                                            .group_hover("tab", |style| style.visible())
                                            .child(Icon::new(IconName::Close).xsmall()),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            match this.tabs[ix].clone() {
                                                WorkspaceTab::Document(doc) => {
                                                    let id = doc.entity_id();
                                                    this.confirm_unsaved(
                                                        doc,
                                                        AfterConfirm::CloseDocument(id),
                                                        window,
                                                        cx,
                                                    );
                                                }
                                                WorkspaceTab::NewTab(new_tab) => {
                                                    let id = new_tab.entity_id();
                                                    this.remove_tab(id, window, cx);
                                                }
                                            }
                                        }),
                                    ),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.activate(ix, window, cx);
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Middle,
                                cx.listener(move |this, _, window, cx| {
                                    match this.tabs[ix].clone() {
                                        WorkspaceTab::Document(doc) => {
                                            let id = doc.entity_id();
                                            this.confirm_unsaved(
                                                doc,
                                                AfterConfirm::CloseDocument(id),
                                                window,
                                                cx,
                                            );
                                        }
                                        WorkspaceTab::NewTab(new_tab) => {
                                            let id = new_tab.entity_id();
                                            this.remove_tab(id, window, cx);
                                        }
                                    }
                                }),
                            )
                    })),
            )
            .child(
                div()
                    .id("new-tab")
                    .role(gpui_kit::Role::Button)
                    .aria_label(if cfg!(target_os = "macos") {
                        "New Tab (Cmd+T)"
                    } else {
                        "New Tab (Ctrl+T)"
                    })
                    .tooltip(|window, cx| {
                        Tooltip::new(if cfg!(target_os = "macos") {
                            "New Tab (Cmd+T)"
                        } else {
                            "New Tab (Ctrl+T)"
                        })
                        .build(window, cx)
                    })
                    .cursor_pointer()
                    .flex_none()
                    .size(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .text_color(theme.muted_foreground)
                    .hover(|style| style.bg(theme.list_hover).text_color(theme.foreground))
                    .active(|style| style.bg(theme.list_active))
                    .child(Icon::new(IconName::Plus).xsmall())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.new_tab(true, window, cx);
                        }),
                    ),
            )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "status bar with catalog/document stats, hover target, line/column info"
    )]
    fn render_status_bar(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let (stats, editor_position, editor_selection, line_ending) = match self.active_tab() {
            WorkspaceTab::NewTab(_) => {
                let catalog = self.catalog.read(cx);
                let total_files = catalog.total_files();
                let total_folders = catalog.folders().len();
                let muted = cx.theme().muted_foreground;
                let item = |text: String| div().text_color(muted).child(text);
                return div()
                    .id("workspace-status-bar")
                    .w_full()
                    .role(gpui_kit::Role::Status)
                    .aria_label("Status bar")
                    .child(
                        StatusBar::new()
                            .left(h_flex().gap_4().child(item(
                                if total_folders == 0 {
                                    "Home — No tracked folders".to_string()
                                } else {
                                    format!(
                                        "Home — {} markdown files across {} folder{}",
                                        total_files,
                                        total_folders,
                                        if total_folders > 1 { "s" } else { "" }
                                    )
                                },
                            )))
                            .right(self.render_layout_button(cx)),
                    );
            }
            WorkspaceTab::Document(doc) => {
                let doc = doc.read(cx);
                let editor = doc.editor().read(cx);
                (
                    doc.analysis().stats,
                    editor.cursor_position(),
                    editor.selected_range(),
                    doc.line_ending(),
                )
            }
        };
        let settings = AppSettings::get(cx);
        let muted = cx.theme().muted_foreground;
        let item = |text: String| div().text_color(muted).child(text);
        let hovered_link = cx.try_global::<HoveredLink>().and_then(|h| h.0.clone());

        let left_section = if let Some(target) = hovered_link {
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Icon::new(IconName::Link)
                        .xsmall()
                        .text_color(cx.theme().primary),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .truncate()
                        .child(target),
                )
        } else {
            h_flex()
                .gap_4()
                .child(item(format!(
                    "{} {}",
                    group_thousands(stats.words),
                    if stats.words == 1 { "word" } else { "words" }
                )))
                .child(item(format!(
                    "{} {}",
                    group_thousands(stats.characters),
                    if stats.characters == 1 {
                        "character"
                    } else {
                        "characters"
                    }
                )))
                .when(stats.words > 0, |this| {
                    this.child(item(format!("{} min read", stats.reading_minutes())))
                })
        };

        div()
            .id("workspace-status-bar")
            .w_full()
            .role(gpui_kit::Role::Status)
            .aria_label("Status bar")
            .child(
                StatusBar::new().left(left_section).right(
                    h_flex()
                        .gap_4()
                        .items_center()
                        .when(settings.layout.shows_editor(), |this| {
                            this.child(item(if editor_selection.is_empty() {
                                format!(
                                    "Ln {}, Col {}",
                                    editor_position.line + 1,
                                    editor_position.character + 1
                                )
                            } else {
                                format!(
                                    "Ln {}, Col {} ({} selected)",
                                    editor_position.line + 1,
                                    editor_position.character + 1,
                                    group_thousands(editor_selection.len())
                                )
                            }))
                        })
                        .child(item(line_ending.label().to_string()))
                        .child(item("Markdown".to_string()))
                        .child(self.render_layout_button(cx)),
                ),
            )
    }

    fn render_layout_button(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let layout = AppSettings::get(cx).layout;
        let action_context = self.active_tab_focus_handle(cx);

        let icon = match layout {
            Layout::Preview => IconName::Eye,
            Layout::Split => IconName::Columns2,
            Layout::Editor => IconName::PenLine,
        };

        let tooltip = match layout {
            Layout::Preview => "Layout: Preview",
            Layout::Split => "Layout: Editor and preview",
            Layout::Editor => "Layout: Editor",
        };

        Button::new("layout-toggle-button")
            .accessibility_label("Switch layout")
            .ghost()
            .small()
            .compact()
            .icon(Icon::new(icon))
            .tooltip(tooltip)
            .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _window, cx| {
                let action_context = action_context.clone();
                let menu = menu.action_context(action_context);
                let current_layout = AppSettings::get(cx).layout;

                menu.menu_with_check(
                    "Editor",
                    current_layout == Layout::Editor,
                    Box::new(SetLayout(Layout::Editor)),
                )
                .menu_with_check(
                    "Editor and preview",
                    current_layout == Layout::Split,
                    Box::new(SetLayout(Layout::Split)),
                )
                .menu_with_check(
                    "Preview",
                    current_layout == Layout::Preview,
                    Box::new(SetLayout(Layout::Preview)),
                )
            })
    }
}

fn default_dir(dir: Option<PathBuf>) -> PathBuf {
    dir.or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn heading_icon(level: u8) -> Icon {
    Icon::new(match level {
        1 => IconName::Heading1,
        2 => IconName::Heading2,
        3 => IconName::Heading3,
        _ => IconName::Heading,
    })
}

/// A file name from the first heading, for documents never saved.
fn suggested_file_name(analysis: &Analysis, extension: &str) -> String {
    let stem: String = analysis
        .outline
        .first()
        .map(|heading| {
            heading
                .text
                .chars()
                .filter(|ch| !matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
                .take(60)
                .collect::<String>()
                .trim()
                .to_string()
        })
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "Untitled".to_string());
    format!("{stem}.{extension}")
}

fn group_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, ch) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI workspace action dispatch and layout tree"
    )]
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.update_window_title(window, cx);
        // Keyboard commands need a focused element inside the workspace; if
        // focus was lost (e.g. its element left the layout), take it back when active.
        #[cfg(not(test))]
        if window.is_window_active() && window.focused(cx).is_none() {
            let focus = self.focus_handle.clone();
            window.defer(cx, move |window, cx| {
                if window.is_window_active() && window.focused(cx).is_none() {
                    focus.focus(window, cx);
                }
            });
        }
        let main = match self.active_tab() {
            WorkspaceTab::Document(doc) => div().size_full().child(doc.clone()),
            WorkspaceTab::NewTab(new_tab) => div().size_full().child(new_tab.clone()),
        };

        v_flex()
            .id("workspace")
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::new_tab_action))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::open_recent))
            .on_action(cx.listener(Self::follow_link))
            .on_action(cx.listener(Self::view_diagram))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::reveal_in_folder))
            .on_action(cx.listener(Self::close_document))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::set_layout))
            .on_action(cx.listener(Self::go_to_heading))
            .on_action(cx.listener(Self::toggle_appearance))
            .on_action(cx.listener(Self::toggle_pane_focus))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                let next = (this.active + 1) % this.tabs.len();
                this.activate(next, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| {
                let count = this.tabs.len();
                this.activate((this.active + count - 1) % count, window, cx);
            }))
            .on_action(cx.listener(|this, action: &ActivateTab, window, cx| {
                this.activate(action.0, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindInPreview, window, cx| {
                if let Some(doc) = this.active_document() {
                    doc.update(cx, |doc, cx| {
                        doc.open_search(window, cx);
                    });
                }
            }))
            .on_action(cx.listener(|this, _: &Search, window, cx| {
                let is_preview = AppSettings::get(cx).layout == Layout::Preview;
                if let Some(doc_entity) = this.active_document() {
                    let active_doc = doc_entity.read(cx);
                    let preview_focused = active_doc
                        .preview()
                        .read(cx)
                        .focus_handle()
                        .is_focused(window);
                    let search_open = active_doc.is_search_open();
                    if is_preview || preview_focused || search_open {
                        doc_entity.update(cx, |doc, cx| {
                            doc.open_search(window, cx);
                        });
                    } else {
                        cx.propagate();
                    }
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                let search_open = this
                    .active_document()
                    .is_some_and(|doc| doc.read(cx).is_search_open());
                if search_open {
                    if let Some(doc) = this.active_document() {
                        doc.update(cx, |doc, cx| {
                            doc.close_search(window, cx);
                        });
                    }
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleScrollSync, _, cx| {
                this.toggle(|settings| settings.scroll_sync = !settings.scroll_sync, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSoftWrap, _, cx| {
                this.toggle(|settings| settings.soft_wrap = !settings.soft_wrap, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleLineNumbers, _, cx| {
                this.toggle(
                    |settings| settings.line_numbers = !settings.line_numbers,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ToggleTabs, _, cx| {
                this.toggle(|settings| settings.tabs = !settings.tabs, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleRestoreSession, _, cx| {
                this.toggle(
                    |settings| settings.restore_session = !settings.restore_session,
                    cx,
                );
            }))
            .on_action(|_: &About, window, cx| dialogs::open_about(window, cx))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom(1., cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom(-1., cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom(0., cx)))
            .on_drop(cx.listener(Self::on_drop_paths))
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().drop_target))
            .child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_title_bar(window, cx)),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().child(main))
            .child(self.render_status_bar(cx))
    }
}

impl Workspace {
    /// Open files handed over by the system while running (macOS "Open
    /// with", a second launch).
    pub(crate) fn open_external(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let documents: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_file()).collect();
        if !documents.is_empty() {
            self.open_paths(documents, window, cx);
        }
    }
}

fn populate_popup_from_menu_items(
    mut popup: PopupMenu,
    items: Vec<OwnedMenuItem>,
    focus_handle: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<'_, PopupMenu>,
) -> PopupMenu {
    for item in items {
        match item {
            OwnedMenuItem::Separator => {
                popup = popup.separator();
            }
            OwnedMenuItem::Action {
                name,
                action,
                checked,
                disabled,
                ..
            } => {
                popup = popup.menu_with_check_and_disabled(name, checked, action, disabled);
            }
            OwnedMenuItem::Submenu(sub) => {
                let sub_items = sub.items;
                let fh = focus_handle.clone();
                popup = popup.submenu(sub.name, window, cx, move |menu, window, cx| {
                    let menu = menu.action_context(fh.clone());
                    populate_popup_from_menu_items(menu, sub_items.clone(), &fh, window, cx)
                });
            }
            OwnedMenuItem::SystemMenu(_) => {}
        }
    }
    popup
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "Tab dirty state and tooltip formatting"
)]
fn format_tab_tooltip(name: &str, path: Option<&Path>, dirty: bool) -> SharedString {
    if dirty {
        if let Some(path) = path {
            SharedString::from(format!("{name} (Edited)\n{}", path.display()))
        } else {
            SharedString::from(format!("{name} (Edited)"))
        }
    } else if let Some(path) = path {
        SharedString::from(format!("{name}\n{}", path.display()))
    } else {
        SharedString::from(name.to_string())
    }
}

/// Where the last session's window was, if it still fits a display.
pub(crate) fn restored_bounds(cx: &App) -> Option<WindowBounds> {
    let placement = Session::load()?.window?;
    if !AppSettings::get(cx).restore_session {
        return None;
    }
    let bounds = Bounds::new(
        gpui_kit::point(px(placement.x), px(placement.y)),
        gpui_kit::size(
            px(placement.width.max(560.)),
            px(placement.height.max(360.)),
        ),
    );
    let visible = cx.displays().iter().any(|display| {
        let area = display.bounds();
        area.intersects(&bounds)
    });
    let bounds = if placement.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    };
    visible.then_some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_thousands() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_000), "1,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn suggests_file_name_from_first_heading() {
        let analysis = Analysis::of("# Q3: Plans/Goals?\n\nBody");
        assert_eq!(suggested_file_name(&analysis, "md"), "Q3 PlansGoals.md");
        assert_eq!(
            suggested_file_name(&Analysis::default(), "html"),
            "Untitled.html"
        );
    }

    #[test]
    fn tab_tooltip_formatting() {
        assert_eq!(format_tab_tooltip("Home", None, false).as_ref(), "Home");
        assert_eq!(
            format_tab_tooltip("Home", None, true).as_ref(),
            "Home (Edited)"
        );
        let path = Path::new("/path/to/document.md");
        assert_eq!(
            format_tab_tooltip("document.md", Some(path), false).as_ref(),
            "document.md\n/path/to/document.md"
        );
        assert_eq!(
            format_tab_tooltip("document.md", Some(path), true).as_ref(),
            "document.md (Edited)\n/path/to/document.md"
        );
    }

    #[gpui_kit::test]
    fn workspace_tab_properties(cx: &mut gpui_kit::TestAppContext) {
        let (doc_view, _) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            cx.set_global(AppSettings(crate::settings::Settings::default()));
            DocumentView::new(String::new(), None, LineEnding::Lf, window, cx)
        });

        cx.update(|cx| {
            let doc_tab = WorkspaceTab::Document(doc_view.clone());
            assert_eq!(doc_tab.name(cx), "Untitled");
            assert_eq!(doc_tab.path(cx), None);
            assert!(!doc_tab.is_dirty(cx));
            assert!(doc_tab.is_blank(cx));
        });

        let (new_tab_view, _) = cx.add_window_view(|window, cx| {
            let catalog = cx.new(|_| FolderCatalog::new(Vec::new()));
            NewTabView::new(catalog, window, cx)
        });

        cx.update(|cx| {
            let new_tab = WorkspaceTab::NewTab(new_tab_view);
            assert_eq!(new_tab.name(cx), "Home");
            assert_eq!(new_tab.path(cx), None);
            assert!(!new_tab.is_dirty(cx));
            assert!(new_tab.is_blank(cx));
        });
    }

    #[gpui_kit::test]
    fn creating_tabs_adds_each_tab(cx: &mut gpui_kit::TestAppContext) {
        let (workspace, _) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            let settings = crate::settings::Settings {
                tabs: true,
                restore_session: false,
                ..Default::default()
            };
            cx.set_global(AppSettings(settings));
            let app_menu_bar = AppMenuBar::new(cx);
            let mut ws = Workspace::new(Vec::new(), app_menu_bar, window, cx);
            let initial_tab_count = ws.tabs.len();
            for _ in 0..15 {
                ws.new_tab(false, window, cx);
            }
            assert_eq!(ws.tabs.len(), initial_tab_count + 15);
            ws
        });
        let _ = workspace;
    }

    #[gpui_kit::test]
    fn tabs_scroll_methods_behave_correctly(cx: &mut gpui_kit::TestAppContext) {
        let (workspace, _) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            let settings = crate::settings::Settings {
                tabs: true,
                restore_session: false,
                ..Default::default()
            };
            cx.set_global(AppSettings(settings));
            let app_menu_bar = AppMenuBar::new(cx);
            let mut ws = Workspace::new(Vec::new(), app_menu_bar, window, cx);
            for _ in 0..10 {
                ws.new_tab(false, window, cx);
            }

            assert_eq!(ws.tabs_scroll_handle.offset().x, px(0.));
            ws.scroll_tabs_by(-px(180.));
            assert_eq!(ws.tabs_scroll_handle.offset().x, -px(180.));
            ws.scroll_tabs_by(px(180.));
            assert_eq!(ws.tabs_scroll_handle.offset().x, px(0.));

            ws.scroll_tab_into_view(5);
            ws
        });
        let _ = workspace;
    }

    #[gpui_kit::test]
    fn new_tab_activates_newly_created_tab(cx: &mut gpui_kit::TestAppContext) {
        let (workspace, _) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            let settings = crate::settings::Settings {
                tabs: true,
                restore_session: false,
                ..Default::default()
            };
            cx.set_global(AppSettings(settings));
            let app_menu_bar = AppMenuBar::new(cx);
            let mut ws = Workspace::new(Vec::new(), app_menu_bar, window, cx);
            assert_eq!(ws.tabs.len(), 1);
            assert_eq!(ws.active, 0);

            ws.new_tab(true, window, cx);
            assert_eq!(ws.tabs.len(), 2);
            assert_eq!(ws.active, 1);
            assert!(matches!(ws.tabs[1], WorkspaceTab::NewTab(_)));
            ws
        });
        let _ = workspace;
    }
}
