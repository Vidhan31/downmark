//! One open document: its text and where it lives, the editor and preview
//! showing it, and everything that follows its edits (statistics, scroll
//! sync, crash recovery).

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gpui_kit::{
    App, AppContext as _, ClipboardEntry, ClipboardItem, Context, Entity, EventEmitter,
    Focusable as _, HighlightStyle, InteractiveElement as _, IntoElement, KeyDownEvent, ListOffset,
    MouseButton, MouseMoveEvent, ParentElement as _, Pixels, Render, RetainAllImageCache,
    ScrollEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window,
    assets::IconName,
    base::{TextView, TextViewState},
    component::{
        ActiveTheme as _, ElementExt as _, Icon, RopeExt as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        clipboard::Clipboard,
        h_flex,
        input::{
            Copy, Cut, Editor, EditorState, Enter, Escape, IndentInline, InputEvent, InputState,
            OutdentInline, Paste, Search, SelectAll, TabSize, TextDecoration,
            TextDecorationCollection,
        },
        native_menu::NativeMenu,
        resizable::{h_resizable, resizable_panel},
        v_flex,
    },
    div, point,
    prelude::FluentBuilder as _,
    px, rems,
};

use crate::{
    actions::{
        Bold, BulletList, CodeBlock, DOCUMENT_CONTEXT, FindInPreview, Heading, InlineCode,
        InsertLink, Italic, NumberedList, PREVIEW_CONTEXT, PreviewPageDown, PreviewPageUp,
        PreviewScrollBottom, PreviewScrollDown, PreviewScrollTop, PreviewScrollUp, Quote,
        Strikethrough, TaskList, TidyTable,
    },
    analysis::{Analysis, content_hash},
    document::{LineEnding, display_name},
    format::{self, Edit},
    images, preview_ext,
    scroll_sync::{ScrollDriver, ScrollSynchronizer},
    search::DocumentSearch,
    search_overlay,
    session::{self, DiskStamp, Recovery},
    settings::Layout,
    smart_edit,
    table_overlay::ExpandedTable,
    themes::AppSettings,
};

/// Widest the text column grows before extra width becomes margin.
const READABLE_WIDTH_REMS: f32 = 46.;
/// How long typing must pause before statistics and the outline refresh.
const ANALYSIS_DEBOUNCE: Duration = Duration::from_millis(120);

/// Policy controlling whether preview and analysis recomputation is debounced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DebouncePolicy {
    Immediate,
    Debounced,
}

/// How long typing must pause before the preview updates while typing in the editor.
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(250);
/// How long typing must pause before unsaved changes are written aside.
const RECOVERY_DEBOUNCE: Duration = Duration::from_secs(1);
/// How much of the text outside the current paragraph fades in focus mode.
const FOCUS_FADE: f32 = 0.7;

/// Something the workspace shows or acts on changed.
pub(crate) enum DocumentEvent {
    /// A message for the user, such as a failed image paste.
    Notify(SharedString),
}

#[derive(Debug, Clone, Default)]
struct SmoothScrollController {
    pending_delta: f32,
    is_animating: bool,
}

impl SmoothScrollController {
    fn new() -> Self {
        Self::default()
    }

    fn add_scroll(&mut self, distance: f32) {
        self.pending_delta = (self.pending_delta + distance).clamp(-1600.0, 1600.0);
    }

    fn reset(&mut self) {
        self.pending_delta = 0.0;
        self.is_animating = false;
    }

    fn calculate_step(&mut self) -> Option<f32> {
        let pending = self.pending_delta;
        if pending.abs() < 0.5 {
            let final_step = if pending.abs() > 0.001 {
                Some(pending)
            } else {
                None
            };
            self.pending_delta = 0.0;
            self.is_animating = false;
            return final_step;
        }

        // Exponential decay interpolation (~22% of remaining distance per 13.3ms frame).
        let fraction = 0.22;
        let mut step = pending * fraction;
        if step.abs() < 1.0 {
            step = if pending > 0.0 {
                1.0f32.min(pending)
            } else {
                (-1.0f32).max(pending)
            };
        }

        self.pending_delta -= step;
        Some(step)
    }
}

pub(crate) struct DocumentView {
    editor: Entity<EditorState>,
    preview: Entity<TextViewState>,
    image_cache: Entity<RetainAllImageCache>,
    /// Parser configuration for the preview; built once so the preview can
    /// tell that it is unchanged between frames.
    markdown_extensions: gpui_kit::component::text::MarkdownExtensions,
    search: DocumentSearch,
    search_input: Entity<InputState>,
    search_open: bool,

    path: Option<PathBuf>,
    line_ending: LineEnding,
    /// The text last read from or written to disk.
    saved_hash: u64,
    dirty: bool,
    /// The file as last read or written, to notice other programs' changes.
    disk: Option<DiskStamp>,
    /// The current source, shared with the preview and background analysis.
    source: SharedString,
    revision: usize,
    analysis: Arc<Analysis>,
    analysis_task: Option<Task<()>>,
    preview_task: Option<Task<()>>,
    /// A heading to show once the outline is known, from following a link.
    pending_anchor: Option<String>,

    recovery_id: String,
    recovery_written: bool,
    recovery_task: Option<Task<()>>,

    /// The caret as of the editor's last change, for code that runs while
    /// the editor is busy (its context menu).
    last_cursor: usize,
    focus_decorations: Option<TextDecorationCollection>,
    /// The paragraph left undimmed in focus mode.
    focus_block: Option<Range<usize>>,

    smooth_scroll: SmoothScrollController,

    scroll_sync: ScrollSynchronizer,
    /// Measured widths used to center the text column.
    editor_width: Pixels,
    preview_width: Pixels,
    last_window_width: Pixels,
    preview_style: gpui_kit::base::TextViewStyle,
    expanded_table: Option<ExpandedTable>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DocumentEvent> for DocumentView {}

impl DocumentView {
    /// A document showing `text`, saved at `path` when it has one.
    pub(crate) fn new(
        text: String,
        path: Option<PathBuf>,
        line_ending: LineEnding,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let settings = AppSettings::get(cx).clone();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .line_number(settings.line_numbers)
                .soft_wrap(settings.soft_wrap)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .indent_guides(false)
                .searchable(true)
                .placeholder("Start writing…")
        });
        let preview = cx.new(|cx| TextViewState::markdown("", cx));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in preview…"));

        let subscriptions = vec![
            cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_text_changed(DebouncePolicy::Debounced, cx);
                }
            }),
            cx.subscribe_in(
                &search_input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => {
                        this.on_search_query_changed(cx);
                    }
                    InputEvent::PressEnter { shift, .. } => {
                        if *shift {
                            this.previous_search_match(cx);
                        } else {
                            this.next_search_match(cx);
                        }
                    }
                    _ => {}
                },
            ),
            cx.observe(&editor, |this, editor, cx| {
                this.last_cursor = editor.read(cx).cursor();
                this.sync_preview(false, cx);
                this.update_focus_dimming(cx);
            }),
            // A re-parse can rebuild the preview's rows, which resets its
            // scroll position; realign once the new rows exist.
            cx.observe(&preview, |this, preview, cx| {
                let rows = preview.read(cx).list_state().item_count();
                if rows != this.scroll_sync.synced_preview_rows() {
                    this.sync_preview(true, cx);
                }
            }),
            cx.observe_global_in::<AppSettings>(window, |this, window, cx| {
                this.apply_settings(window, cx);
                this.sync_preview(true, cx);
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, _, cx| {
                this.preview_style = make_preview_style(cx);
                cx.notify();
            }),
        ];

        let preview_style = make_preview_style(cx);

        let mut this = Self {
            editor,
            markdown_extensions: preview_ext::markdown_extensions(&preview),
            preview,
            image_cache: RetainAllImageCache::new(cx),
            search: DocumentSearch::new(),
            search_input,
            search_open: false,
            path: None,
            line_ending: LineEnding::Lf,
            saved_hash: content_hash(""),
            dirty: false,
            disk: None,
            source: SharedString::default(),
            revision: 0,
            analysis: Arc::default(),
            analysis_task: None,
            preview_task: None,
            pending_anchor: None,
            recovery_id: session::new_recovery_id(),
            recovery_written: false,
            recovery_task: None,
            last_cursor: 0,
            focus_decorations: None,
            focus_block: None,
            smooth_scroll: SmoothScrollController::new(),
            scroll_sync: ScrollSynchronizer::new(),
            editor_width: px(0.),
            preview_width: px(0.),
            last_window_width: px(0.),
            preview_style,
            expanded_table: None,
            _subscriptions: subscriptions,
        };
        this.load_text(text, path, line_ending, window, cx);
        this
    }

    pub(crate) fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub(crate) fn preview(&self) -> &Entity<TextViewState> {
        &self.preview
    }

    pub(crate) fn is_preview_focused(&self, window: &Window, cx: &App) -> bool {
        self.preview.read(cx).focus_handle().is_focused(window)
    }

    #[expect(dead_code, reason = "helper for editor focus check")]
    pub(crate) fn is_editor_focused(&self, window: &Window, cx: &App) -> bool {
        self.editor.read(cx).focus_handle(cx).is_focused(window)
    }

    pub(crate) fn focus_preview(&self, window: &mut Window, cx: &mut App) {
        let handle = self.preview.read(cx).focus_handle().clone();
        window.focus(&handle, cx);
    }

    pub(crate) fn focus_editor(&self, window: &mut Window, cx: &mut App) {
        let handle = self.editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    pub(crate) fn toggle_pane_focus(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.is_preview_focused(window, cx) {
            self.focus_editor(window, cx);
        } else {
            self.focus_preview(window, cx);
        }
    }

    fn schedule_smooth_scroll_tick(window: &mut Window, cx: &mut Context<'_, Self>) {
        let view = cx.entity();
        window.on_next_frame(move |window, cx| {
            view.update(cx, |this, cx| {
                this.tick_smooth_scroll(window, cx);
            });
        });
    }

    fn tick_smooth_scroll(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.smooth_scroll.is_animating {
            return;
        }

        if let Some(step) = self.smooth_scroll.calculate_step() {
            let list = self.preview.read(cx).list_state().clone();
            list.scroll_by(px(step));
            self.preview.update(cx, |_, cx| cx.notify());
            self.sync_editor(cx);
        }

        if self.smooth_scroll.is_animating {
            Self::schedule_smooth_scroll_tick(window, cx);
        }
    }

    fn preview_scroll_down(
        &mut self,
        _: &PreviewScrollDown,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_scroll_driver(ScrollDriver::Preview);
        let delta = (window.line_height() * 3.).as_f32();
        self.smooth_scroll.add_scroll(delta);
        if !self.smooth_scroll.is_animating {
            self.smooth_scroll.is_animating = true;
            Self::schedule_smooth_scroll_tick(window, cx);
        }
    }

    fn preview_scroll_up(
        &mut self,
        _: &PreviewScrollUp,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_scroll_driver(ScrollDriver::Preview);
        let delta = (-window.line_height() * 3.).as_f32();
        self.smooth_scroll.add_scroll(delta);
        if !self.smooth_scroll.is_animating {
            self.smooth_scroll.is_animating = true;
            Self::schedule_smooth_scroll_tick(window, cx);
        }
    }

    fn preview_page_down(
        &mut self,
        _: &PreviewPageDown,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_scroll_driver(ScrollDriver::Preview);
        let list = self.preview.read(cx).list_state().clone();
        let height = list.viewport_bounds().size.height;
        let delta = if height > px(0.) {
            (height * 0.85).as_f32()
        } else {
            400.0
        };
        self.smooth_scroll.add_scroll(delta);
        if !self.smooth_scroll.is_animating {
            self.smooth_scroll.is_animating = true;
            Self::schedule_smooth_scroll_tick(window, cx);
        }
    }

    fn preview_page_up(
        &mut self,
        _: &PreviewPageUp,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_scroll_driver(ScrollDriver::Preview);
        let list = self.preview.read(cx).list_state().clone();
        let height = list.viewport_bounds().size.height;
        let delta = if height > px(0.) {
            (-height * 0.85).as_f32()
        } else {
            -400.0
        };
        self.smooth_scroll.add_scroll(delta);
        if !self.smooth_scroll.is_animating {
            self.smooth_scroll.is_animating = true;
            Self::schedule_smooth_scroll_tick(window, cx);
        }
    }

    fn preview_scroll_top(
        &mut self,
        _: &PreviewScrollTop,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.smooth_scroll.reset();
        self.set_scroll_driver(ScrollDriver::Preview);
        let list = self.preview.read(cx).list_state().clone();
        list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        self.preview.update(cx, |_, cx| cx.notify());
        self.sync_editor(cx);
    }

    fn preview_scroll_bottom(
        &mut self,
        _: &PreviewScrollBottom,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.smooth_scroll.reset();
        self.set_scroll_driver(ScrollDriver::Preview);
        let list = self.preview.read(cx).list_state().clone();
        list.scroll_to_end();
        self.preview.update(cx, |_, cx| cx.notify());
        self.sync_editor(cx);
    }

    pub(crate) fn source(&self) -> &SharedString {
        &self.source
    }

    pub(crate) fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn analysis(&self) -> &Arc<Analysis> {
        &self.analysis
    }

    pub(crate) fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub(crate) fn disk_stamp(&self) -> Option<DiskStamp> {
        self.disk
    }

    pub(crate) fn saved_hash(&self) -> u64 {
        self.saved_hash
    }

    pub(crate) fn is_search_open(&self) -> bool {
        self.search_open
    }

    pub(crate) fn open_search(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.flush_preview(cx);
        if AppSettings::get(cx).layout == Layout::Editor {
            AppSettings::update(cx, |settings| settings.layout = Layout::Split);
        }
        self.search_open = true;
        self.search_input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.on_search_query_changed(cx);
        cx.notify();
    }

    pub(crate) fn close_search(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.search_open = false;
        self.clear_search(cx);
        let preview_focus = self.preview.read(cx).focus_handle().clone();
        window.focus(&preview_focus, cx);
        cx.notify();
    }

    #[expect(dead_code, reason = "Convenience toggle for search overlay")]
    pub(crate) fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.search_open {
            self.close_search(window, cx);
        } else {
            self.open_search(window, cx);
        }
    }

    pub(crate) fn clear_search(&mut self, cx: &mut Context<'_, Self>) {
        self.search.clear_query();
        self.preview.update(cx, |state, cx| {
            state.clear_range_highlights(cx);
        });
    }

    fn on_search_query_changed(&mut self, cx: &mut Context<'_, Self>) {
        let query = self.search_input.read(cx).value().to_string();
        let rendered_text = self.preview.read(cx).rendered_text();
        self.search.update_query(&query, rendered_text.as_str());

        self.update_search_highlights(cx);
        if let Some(target_range) = self.search.current_match().cloned() {
            self.scroll_to_match(&target_range, cx);
        }
        cx.notify();
    }

    fn update_search_highlights(&mut self, cx: &mut Context<'_, Self>) {
        let (color, current_color) = (cx.theme().warning.opacity(0.35), cx.theme().warning);
        let highlights = self.search.build_highlights(color, current_color);
        self.preview.update(cx, |state, cx| {
            _ = state.set_range_highlights(highlights, cx);
        });
    }

    pub(crate) fn next_search_match(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(target_range) = self.search.next_match().cloned() {
            self.update_search_highlights(cx);
            self.scroll_to_match(&target_range, cx);
            cx.notify();
        }
    }

    pub(crate) fn previous_search_match(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(target_range) = self.search.previous_match().cloned() {
            self.update_search_highlights(cx);
            self.scroll_to_match(&target_range, cx);
            cx.notify();
        }
    }

    fn scroll_to_match(&mut self, target_range: &Range<usize>, cx: &mut Context<'_, Self>) {
        self.set_scroll_driver(ScrollDriver::Preview);
        self.preview.update(cx, |state, cx| {
            _ = state.reveal_range(target_range.clone(), cx);
            cx.notify();
        });
    }

    #[cfg(test)]
    pub(crate) fn is_expanded_table_open(&self) -> bool {
        self.expanded_table.is_some()
    }

    pub(crate) fn open_expanded_table(
        &mut self,
        fallback_markdown: &str,
        span: Option<Range<usize>>,
        cx: &mut Context<'_, Self>,
    ) {
        let text = span
            .and_then(|range| self.source.get(range))
            .unwrap_or(fallback_markdown);
        let markdown = SharedString::from(text.to_string());
        self.expanded_table = Some(ExpandedTable::new(markdown, cx));
        cx.notify();
    }

    pub(crate) fn close_expanded_table(&mut self, cx: &mut Context<'_, Self>) {
        if self.expanded_table.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn recovery_id(&self) -> Option<&str> {
        self.recovery_written.then_some(self.recovery_id.as_str())
    }

    pub(crate) fn name(&self) -> String {
        display_name(self.path.as_deref())
    }

    pub(crate) fn dir(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
    }

    /// An untitled document nobody has typed into, which opening a file may
    /// replace without asking.
    pub(crate) fn is_blank(&self) -> bool {
        self.path.is_none() && !self.dirty
    }

    pub(crate) fn cursor(&self, cx: &App) -> usize {
        self.editor.read(cx).cursor()
    }

    pub(crate) fn set_cursor(
        &mut self,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.editor.update(cx, |editor, cx| {
            let offset = smart_edit::floor_char_boundary(&editor.value(), offset);
            editor.set_selected_range(offset..offset, cx);
            let position = editor.text().offset_to_position(offset);
            editor.set_cursor_position(position, window, cx);
        });
    }

    /// Replace the document with `text` read from `path`.
    fn load_text(
        &mut self,
        text: String,
        path: Option<PathBuf>,
        line_ending: LineEnding,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.saved_hash = content_hash(&text);
        self.disk = path.as_deref().and_then(DiskStamp::of);
        self.path = path;
        self.line_ending = line_ending;
        self.dirty = false;
        self.clear_search(cx);
        self.search_open = false;
        self.scroll_sync.reset_editor_scroll();
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        // `set_value` does not emit a change event.
        self.on_text_changed(DebouncePolicy::Immediate, cx);
        self.dirty = false;
    }

    /// Restore unsaved text recovered after a crash; it stays unsaved.
    pub(crate) fn restore_unsaved(
        &mut self,
        id: String,
        recovery: Recovery,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let saved = recovery
            .path
            .as_deref()
            .and_then(|path| crate::document::load(path).ok());
        self.path = recovery.path;
        self.disk = self.path.as_deref().and_then(DiskStamp::of);
        self.line_ending = if recovery.crlf {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        self.saved_hash = saved.map_or(content_hash(""), |saved| content_hash(&saved.text));
        self.recovery_id = id;
        self.recovery_written = true;
        self.editor
            .update(cx, |editor, cx| editor.set_value(recovery.text, window, cx));
        self.on_text_changed(DebouncePolicy::Immediate, cx);
    }

    /// Show `text`, which another program saved to the document's file,
    /// keeping the caret where it was.
    pub(crate) fn reload(
        &mut self,
        text: String,
        line_ending: LineEnding,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let cursor = self.cursor(cx);
        let path = self.path.clone();
        self.load_text(text, path, line_ending, window, cx);
        self.set_cursor(cursor, window, cx);
        self.discard_recovery();
    }

    /// Accept the file on disk as it is now without reloading it.
    pub(crate) fn set_disk_stamp(&mut self, stamp: Option<DiskStamp>) {
        self.disk = stamp;
    }

    /// The file was deleted or moved away: the text only exists here now.
    pub(crate) fn mark_missing(&mut self, cx: &mut Context<'_, Self>) {
        self.disk = None;
        self.saved_hash = content_hash("\u{0}missing");
        self.dirty = true;
        self.schedule_recovery(cx);
        cx.notify();
    }

    /// Immediately update the preview with the latest source if an update is pending.
    pub(crate) fn flush_preview(&mut self, cx: &mut Context<'_, Self>) {
        if self.preview_task.is_some() {
            self.schedule_preview(DebouncePolicy::Immediate, cx);
        }
    }

    fn on_text_changed(&mut self, policy: DebouncePolicy, cx: &mut Context<'_, Self>) {
        let source = self.editor.read(cx).value();
        self.source = source;
        // Assume an edit until the analysis can tell the text matches disk.
        self.dirty = true;
        self.set_scroll_driver(ScrollDriver::Editor);
        self.revision += 1;
        self.focus_block = None;
        self.expanded_table = None;
        self.search.invalidate_cache();
        self.schedule_preview(policy, cx);
        self.schedule_analysis(policy, cx);
        cx.notify();
    }

    /// Update the preview with the current editor source text. When `policy`
    /// is Debounced, updates are delayed until typing pauses so rapid keystrokes don't
    /// re-parse and re-layout markdown on every frame.
    fn schedule_preview(&mut self, policy: DebouncePolicy, cx: &mut Context<'_, Self>) {
        let revision = self.revision;
        let source = self.source.clone();
        if policy == DebouncePolicy::Debounced {
            self.preview_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
                _ = this.update(cx, |this, cx| {
                    if this.revision != revision {
                        return;
                    }
                    this.preview
                        .update(cx, |preview, cx| preview.set_text(&source, cx));
                    if this.search_open && !this.search.query().is_empty() {
                        this.on_search_query_changed(cx);
                    }
                    this.sync_preview(true, cx);
                    this.preview_task = None;
                    cx.notify();
                });
            }));
        } else {
            self.preview_task = None;
            self.preview
                .update(cx, |preview, cx| preview.set_text(&source, cx));
            if self.search_open && !self.search.query().is_empty() {
                self.on_search_query_changed(cx);
            }
            self.sync_preview(true, cx);
        }
    }

    /// Recompute statistics, the outline and the block index off the UI
    /// thread once typing pauses. Stale results are discarded.
    fn schedule_analysis(&mut self, policy: DebouncePolicy, cx: &mut Context<'_, Self>) {
        let revision = self.revision;
        let source = self.source.clone();
        self.analysis_task = Some(cx.spawn(async move |this, cx| {
            if policy == DebouncePolicy::Debounced {
                cx.background_executor().timer(ANALYSIS_DEBOUNCE).await;
            }
            let analysis = cx
                .background_executor()
                .spawn(async move { Analysis::of(&source) })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                this.dirty = analysis.content_hash != this.saved_hash;
                this.analysis = Arc::new(analysis);
                this.analysis_task = None;
                if this.dirty {
                    this.schedule_recovery(cx);
                } else {
                    this.discard_recovery();
                }
                this.sync_preview(true, cx);
                cx.notify();
            });
        }));
    }

    /// Write the current text to `path`, which becomes the document's.
    pub(crate) fn write_to(
        &mut self,
        path: &Path,
        cx: &mut Context<'_, Self>,
    ) -> std::io::Result<()> {
        self.flush_preview(cx);
        let text = self.editor.read(cx).value();
        crate::document::save(path, &text, self.line_ending)?;
        self.saved_hash = content_hash(&text);
        self.dirty = self.source.as_ref() != text.as_ref();
        self.path = Some(path.to_path_buf());
        self.disk = DiskStamp::of(path);
        if !self.dirty {
            self.discard_recovery();
        }
        cx.notify();
        Ok(())
    }

    /// Write unsaved changes aside once typing pauses, so a crash can't lose
    /// them.
    fn schedule_recovery(&mut self, cx: &mut Context<'_, Self>) {
        let revision = self.revision;
        self.recovery_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RECOVERY_DEBOUNCE).await;
            let Ok(Some((id, recovery))) = this.read_with(cx, |this, _| {
                (this.dirty && this.revision == revision).then(|| {
                    (
                        this.recovery_id.clone(),
                        Recovery {
                            path: this.path.clone(),
                            text: this.source.to_string(),
                            crlf: this.line_ending == LineEnding::CrLf,
                        },
                    )
                })
            }) else {
                return;
            };
            let written = cx
                .background_executor()
                .spawn(async move { session::write_recovery(&id, &recovery).is_ok() })
                .await;
            _ = this.update(cx, |this, _| {
                this.recovery_written |= written;
                this.recovery_task = None;
            });
        }));
    }

    /// Forget the unsaved changes written aside: they were saved or thrown
    /// away.
    pub(crate) fn discard_recovery(&mut self) {
        self.recovery_task = None;
        if self.recovery_written {
            session::remove_recovery(&self.recovery_id);
            self.recovery_written = false;
        }
    }

    fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let settings = AppSettings::get(cx).clone();
        self.editor.update(cx, |editor, cx| {
            editor.set_soft_wrap(settings.soft_wrap, window, cx);
            editor.set_line_number(settings.line_numbers && !settings.focus_mode, window, cx);
            editor.set_folding(!settings.focus_mode, window, cx);
        });
        self.preview_style = make_preview_style(cx);
        if settings.layout == Layout::Preview {
            self.flush_preview(cx);
        }
        self.focus_block = None;
        self.update_focus_dimming(cx);
    }

    /// The editor's context menu: standard edit commands.
    fn context_menu(menu: NativeMenu, _cx: &App) -> NativeMenu {
        menu.menu("Cut", Box::new(Cut))
            .menu("Copy", Box::new(Copy))
            .menu("Paste", Box::new(Paste))
            .separator()
            .menu("Select all", Box::new(SelectAll))
    }

    /// In focus mode, fade everything but the paragraph with the caret.
    fn update_focus_dimming(&mut self, cx: &mut Context<'_, Self>) {
        let focus_mode = AppSettings::get(cx).focus_mode;
        if !focus_mode {
            if let Some(decorations) = self.focus_decorations.take() {
                decorations.clear(cx);
            }
            self.focus_block = None;
            return;
        }
        let cursor = self.cursor(cx);
        if self
            .focus_block
            .as_ref()
            .is_some_and(|block| block.start <= cursor && cursor <= block.end)
        {
            return;
        }
        let block = smart_edit::paragraph_at(&self.source, cursor);
        let len = self.source.len();
        let fade = HighlightStyle {
            fade_out: Some(FOCUS_FADE),
            ..HighlightStyle::default()
        };
        let decorations: Vec<TextDecoration> = [0..block.start, block.end..len]
            .into_iter()
            .filter(|range| !range.is_empty())
            .map(|range| TextDecoration::new(range, fade))
            .collect();
        self.focus_block = Some(block);
        if let Some(collection) = &self.focus_decorations {
            collection.set(decorations, cx);
        } else {
            let collection = self.editor.update(cx, |editor, cx| {
                editor.create_decorations_collection(decorations, cx)
            });
            self.focus_decorations = Some(collection);
        }
    }

    fn shows_editor(cx: &App) -> bool {
        let settings = AppSettings::get(cx);
        settings.focus_mode || settings.layout.shows_editor()
    }

    fn apply(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.set_selected_range(edit.range.clone(), cx);
            editor.replace(edit.text, window, cx);
            editor.set_selected_range(edit.selection, cx);
        });
    }

    /// Run a formatting command on the selection.
    pub(crate) fn apply_edit(
        &mut self,
        build: impl FnOnce(&str, Range<usize>) -> Edit,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !Self::shows_editor(cx) {
            return;
        }
        let text = self.editor.read(cx).value().to_string();
        let edit = build(&text, self.editor.read(cx).selected_range());
        self.apply(edit, window, cx);
        self.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
    }

    pub(crate) fn inline(
        &mut self,
        style: format::Inline,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.apply_edit(
            |text, selection| format::toggle_inline(text, selection, style),
            window,
            cx,
        );
    }

    pub(crate) fn block(
        &mut self,
        style: format::Block,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.apply_edit(
            |text, selection| format::toggle_block(text, selection, style),
            window,
            cx,
        );
    }

    pub(crate) fn tidy_table(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let text = self.editor.read(cx).value().to_string();
        let selection = self.editor.read(cx).selected_range();
        match smart_edit::tidy_table(&text, selection) {
            Some(edit) => self.apply(edit, window, cx),
            None => cx.emit(DocumentEvent::Notify(
                "Put the caret in a table to tidy it.".into(),
            )),
        }
    }

    /// Enter, Tab and Shift-Tab in the editor: continue lists, nest items,
    /// move between table cells. Returns whether the key was handled.
    fn smart_key(
        &mut self,
        assist: impl FnOnce(&str, Range<usize>) -> Option<Edit>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !self.editor.focus_handle(cx).is_focused(window) {
            return false;
        }
        let editor = self.editor.read(cx);
        let text = editor.value().to_string();
        let Some(edit) = assist(&text, editor.selected_range()) else {
            return false;
        };
        self.apply(edit, window, cx);
        true
    }

    /// Paste images as files next to the document, and URLs over selected
    /// text as links. Returns whether the paste was handled.
    fn paste(
        &mut self,
        item: &ClipboardItem,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    let bytes = image.bytes.clone();
                    let extension = image.format.extension();
                    self.insert_images(
                        vec![images::ImageSource::Bytes(bytes, extension)],
                        window,
                        cx,
                    );
                    return true;
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    let images: Vec<_> = paths
                        .paths()
                        .iter()
                        .filter(|path| images::is_image_path(path))
                        .map(|path| images::ImageSource::File(path.clone()))
                        .collect();
                    if !images.is_empty() {
                        self.insert_images(images, window, cx);
                        return true;
                    }
                }
                ClipboardEntry::String(string) => {
                    let text = self.editor.read(cx).value().to_string();
                    let selection = self.editor.read(cx).selected_range();
                    if let Some(edit) = smart_edit::paste_link(&text, selection, string.text()) {
                        self.apply(edit, window, cx);
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Insert dropped image files at the caret.
    pub(crate) fn drop_images(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let images = paths
            .iter()
            .filter(|path| images::is_image_path(path))
            .map(|path| images::ImageSource::File(path.clone()))
            .collect();
        self.insert_images(images, window, cx);
    }

    /// Store images in the document's image folder (files already beside
    /// the document are linked where they are) and link them at the caret.
    fn insert_images(
        &mut self,
        images: Vec<images::ImageSource>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(dir) = self.dir() else {
            cx.emit(DocumentEvent::Notify(
                "Save the document first, so images can be stored next to it.".into(),
            ));
            return;
        };
        let folder = AppSettings::get(cx).image_folder.clone();
        let stem = self
            .path
            .as_ref()
            .and_then(|path| path.file_stem())
            .map_or_else(
                || "image".to_string(),
                |stem| stem.to_string_lossy().to_string(),
            );
        let mut links = Vec::new();
        for image in images {
            match images::store_image(&image, &dir, &folder, &stem) {
                Ok((relative, alt)) => links.push((relative, alt)),
                Err(err) => {
                    cx.emit(DocumentEvent::Notify(
                        format!("Couldn’t add the image. {err}").into(),
                    ));
                }
            }
        }
        if links.is_empty() {
            return;
        }
        let text = self.editor.read(cx).value().to_string();
        let caret = self.editor.read(cx).selected_range();
        let mut markdown = String::new();
        for (ix, (relative, alt)) in links.iter().enumerate() {
            if ix > 0 {
                markdown.push('\n');
            }
            let offset = if ix == 0 { caret.start } else { 0 };
            markdown.push_str(&smart_edit::image_markdown(
                if ix == 0 { &text } else { "" },
                offset,
                relative,
                alt,
            ));
        }
        let end = caret.start + markdown.len();
        self.apply(
            Edit {
                range: caret,
                text: markdown,
                selection: end..end,
            },
            window,
            cx,
        );
        self.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
    }

    /// Show the heading a link's `#anchor` names, once the outline is known.
    pub(crate) fn reveal_anchor(
        &mut self,
        anchor: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.analysis_task.is_some() {
            self.pending_anchor = Some(anchor.to_string());
            return;
        }
        match crate::links::find_heading(&self.analysis.outline, anchor) {
            Some(ix) => self.reveal_heading(ix, window, cx),
            None => cx.emit(DocumentEvent::Notify(
                format!("“{}” has no heading “#{anchor}”.", self.name()).into(),
            )),
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::as_conversions,
        reason = "Line number fits u32 editor position and f32 block interpolation"
    )]
    pub(crate) fn reveal_heading(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(heading) = self.analysis.outline.get(ix).cloned() else {
            return;
        };
        if Self::shows_editor(cx) {
            self.set_scroll_driver(ScrollDriver::Editor);
            self.editor.update(cx, |editor, cx| {
                editor.set_cursor_position(
                    gpui_kit::component::input::Position::new(heading.line as u32, 0),
                    window,
                    cx,
                );
            });
            // Revealing the caret scrolls just far enough to show it, often
            // leaving the heading at the bottom edge. Once that frame is laid
            // out, scroll the heading to the top of the editor instead.
            let editor = self.editor.clone();
            window.on_next_frame(move |_, cx| {
                editor.update(cx, |editor, cx| {
                    let Some((caret, _)) = editor.cursor_layout() else {
                        return;
                    };
                    // Caret bounds are laid out before scrolling, so they
                    // are content positions measured from the input's top.
                    let offset = editor.scroll_offset();
                    let target = (editor.input_bounds().top() - caret.top()).min(px(0.));
                    editor.set_scroll_offset(point(offset.x, target), cx);
                });
            });
        }
        if let Some((block, _)) = self.analysis.blocks.block_at_line(heading.line as f32) {
            self.preview.update(cx, |preview, cx| {
                preview.list_state().scroll_to(ListOffset {
                    item_ix: block,
                    offset_in_item: px(0.),
                });
                cx.notify();
            });
        }
    }

    fn set_scroll_driver(&mut self, driver: ScrollDriver) {
        self.scroll_sync.set_driver(driver);
    }

    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "force flag indicates re-layout realignment"
    )]
    fn sync_preview(&mut self, force: bool, cx: &mut Context<'_, Self>) {
        self.scroll_sync
            .sync_preview(&self.editor, &self.preview, &self.analysis, force, cx);
    }

    fn sync_editor(&mut self, cx: &mut Context<'_, Self>) {
        self.scroll_sync
            .sync_editor(&self.preview, &self.editor, &self.analysis, cx);
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI layout for editor pane"
    )]
    fn render_editor(&self, window: &Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let document = cx.weak_entity();
        let inset = column_inset(self.pane_width(self.editor_width, window, cx), cx);
        let focus_mode = AppSettings::get(cx).focus_mode;
        let (menu_document, paste_document) = (document.clone(), document.clone());
        div()
            .id("editor-pane")
            .size_full()
            .bg(cx.theme().background)
            .font_family(cx.theme().mono_font_family.clone())
            // The margins beside the text column belong to the editor too:
            // the editor handles the wheel over its text, this handles the
            // rest so the whole pane scrolls.
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| {
                this.set_scroll_driver(ScrollDriver::Editor);
            }))
            .capture_key_down(cx.listener(|this, _: &KeyDownEvent, _, _| {
                this.set_scroll_driver(ScrollDriver::Editor);
            }))
            .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                if !action.secondary
                    && !action.shift
                    && this.smart_key(smart_edit::enter, window, cx)
                {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                let handled = this.smart_key(
                    |text, selection| {
                        smart_edit::table_tab(
                            text,
                            selection.clone(),
                            smart_edit::TabDirection::Forward,
                        )
                        .or_else(|| smart_edit::indent(text, selection))
                    },
                    window,
                    cx,
                );
                if handled {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &OutdentInline, window, cx| {
                let handled = this.smart_key(
                    |text, selection| {
                        smart_edit::table_tab(
                            text,
                            selection.clone(),
                            smart_edit::TabDirection::Backward,
                        )
                        .or_else(|| smart_edit::outdent(text, selection))
                    },
                    window,
                    cx,
                );
                if handled {
                    cx.stop_propagation();
                }
            }))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                if this.expanded_table.is_some() {
                    return;
                }
                this.set_scroll_driver(ScrollDriver::Editor);
                this.editor.update(cx, |editor, cx| {
                    let line_height = editor.line_height().unwrap_or(window.line_height());
                    let delta = event.delta.pixel_delta(line_height);
                    let offset = editor.scroll_offset();
                    editor.set_scroll_offset(point(offset.x, offset.y + delta.y), cx);
                });
            }))
            .child(
                Editor::new(&self.editor)
                    .h_full()
                    .text_size(rems(if focus_mode { 1. } else { 0.875 }))
                    .line_height(rems(if focus_mode { 1.75 } else { 1.5 }))
                    .border_0()
                    .rounded_none()
                    .pl(inset)
                    .pr(inset)
                    .pt(if focus_mode { rems(4.) } else { rems(1.) })
                    .aria_label("Markdown source")
                    .context_menu(move |menu, _, cx| {
                        menu_document
                            .read_with(cx, |_, cx| Self::context_menu(NativeMenu::new(), cx))
                            .unwrap_or(menu)
                    })
                    .on_paste(move |item, window, cx| {
                        paste_document
                            .update(cx, |this, cx| this.paste(item, window, cx))
                            .unwrap_or(false)
                    }),
            )
            .on_prepaint(move |bounds, _, cx| {
                let layout = AppSettings::get(cx).layout;
                cx.defer(move |cx| {
                    _ = document.update(cx, |this, cx| {
                        let changed = (this.editor_width - bounds.size.width).abs() >= px(1.);
                        this.editor_width = bounds.size.width;
                        if changed && layout == Layout::Split {
                            cx.notify();
                        }
                    });
                });
            })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI layout for preview pane"
    )]
    fn render_preview(&self, window: &Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let document = cx.weak_entity();
        let reveal_doc = cx.weak_entity();
        let base_dir = self.dir().map(Arc::<Path>::from);
        let inset = column_inset(self.pane_width(self.preview_width, window, cx), cx);
        let is_empty = self.source.trim().is_empty();

        div()
            .id("preview-pane")
            .key_context(PREVIEW_CONTEXT)
            .role(gpui_kit::Role::Document)
            .aria_label("Markdown preview")
            .size_full()
            .relative()
            .bg(cx.theme().background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    let preview_focus = this.preview.read(cx).focus_handle().clone();
                    window.focus(&preview_focus, cx);
                }),
            )
            // Let the margins beside the text column scroll the preview; the
            // list scrolls itself when the pointer is over its viewport.
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, _| {
                if this.expanded_table.is_some() {
                    return;
                }
                this.set_scroll_driver(ScrollDriver::Preview);
            }))
            .capture_key_down(cx.listener(|this, _: &KeyDownEvent, _, _| {
                if this.expanded_table.is_some() {
                    return;
                }
                this.set_scroll_driver(ScrollDriver::Preview);
            }))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                if this.expanded_table.is_some() {
                    return;
                }
                this.set_scroll_driver(ScrollDriver::Preview);
                let list = this.preview.read(cx).list_state().clone();
                if list.viewport_bounds().contains(&event.position) {
                    return;
                }
                let delta = event.delta.pixel_delta(window.line_height());
                list.scroll_by(-delta.y);
                this.preview.update(cx, |_, cx| cx.notify());
            }))
            .child(
                gpui_kit::image_cache(self.image_cache.clone())
                    .size_full()
                    .child(
                        TextView::new(&self.preview)
                            .size_full()
                            .scrollable(true)
                            .selectable(true)
                            .style(self.preview_style.clone())
                            .markdown_extensions(self.markdown_extensions.clone())
                            .pl(inset)
                            .pr(inset)
                            .py_8()
                            .image_source(move |url| images::resolve(url, base_dir.as_deref()))
                            .code_block_actions(|code_block, _, _| {
                                Clipboard::new("copy-code").value(code_block.code())
                            })
                            .table_actions({
                                let document = document.clone();
                                move |table_data, _window, cx| {
                                    let document = document.clone();
                                    let markdown = table_data.markdown.clone();
                                    let span = table_data.span.clone();
                                    let btn_id = SharedString::from(format!(
                                        "expand-table-{}",
                                        span.as_ref().map_or(0, |s| s.start)
                                    ));
                                    let row_count = table_data.rows.len();
                                    let is_long = row_count > 6;

                                    h_flex()
                                        .items_center()
                                        .when(is_long, |el| el.justify_between().w_full().px_1())
                                        .when(!is_long, gpui_kit::Styled::justify_end)
                                        .gap_1()
                                        .when(is_long, |el| {
                                            el.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(format!("{row_count} rows total (preview shortened)")),
                                            )
                                        })
                                        .child(
                                            Button::new(btn_id)
                                                .xsmall()
                                                .ghost()
                                                .icon(Icon::new(IconName::Maximize))
                                                .label("Expand table")
                                                .tooltip("View full table across window width (Esc to close)")
                                                .on_click(move |_, _window, cx| {
                                                    _ = document.update(cx, |this, cx| {
                                                        this.open_expanded_table(
                                                            &markdown,
                                                            span.clone(),
                                                            cx,
                                                        );
                                                    });
                                                }),
                                        )
                                        .into_any_element()
                                }
                            })
                            .on_link_click(crate::links::on_click)
                            .on_reveal(move |line, _, cx| {
                                _ = reveal_doc.update(cx, |this, cx| {
                                    let list = this.preview.read(cx).list_state().clone();
                                    let viewport = list.viewport_bounds();
                                    if viewport.size.height <= px(0.) {
                                        return;
                                    }
                                    if line.top() < viewport.top() {
                                        list.scroll_by(viewport.top() - line.top() + px(24.));
                                        this.preview.update(cx, |_, cx| cx.notify());
                                    } else if line.bottom() > viewport.bottom() {
                                        list.scroll_by(viewport.bottom() - line.bottom() - px(24.));
                                        this.preview.update(cx, |_, cx| cx.notify());
                                    }
                                });
                            }),
                    ),
            )
            .when(is_empty, |this| {
                this.child(
                    v_flex()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            div()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(cx.theme().foreground)
                                .child("Nothing to preview"),
                        )
                        .child("Markdown you write appears here."),
                )
            })
            .when(self.search_open, |this| {
                let match_display = self.search.match_display();
                let total_matches = self.search.total_matches();
                let doc = cx.weak_entity();
                let doc_prev = doc.clone();
                let doc_next = doc.clone();
                let doc_close = doc;
                this.child(search_overlay::render_search_overlay(
                    &self.search_input,
                    &match_display,
                    total_matches,
                    move |_, cx| {
                        _ = doc_prev.update(cx, DocumentView::previous_search_match);
                    },
                    move |_, cx| {
                        _ = doc_next.update(cx, DocumentView::next_search_match);
                    },
                    move |window, cx| {
                        _ = doc_close.update(cx, |this, cx| this.close_search(window, cx));
                    },
                    cx,
                ))
            })
            .on_prepaint(move |bounds, _, cx| {
                let layout = AppSettings::get(cx).layout;
                cx.defer(move |cx| {
                    _ = document.update(cx, |this, cx| {
                        if this.expanded_table.is_some() {
                            return;
                        }
                        let changed = (this.preview_width - bounds.size.width).abs() >= px(1.);
                        this.preview_width = bounds.size.width;
                        if changed && layout == Layout::Split {
                            cx.notify();
                        }
                        this.sync_editor(cx);
                    });
                });
            })
    }

    /// A pane fills the view unless the editor and preview share it, in
    /// which case its last measured width is used (scaled proportionally if
    /// the window resized).
    fn pane_width(&self, measured: Pixels, window: &Window, cx: &App) -> Pixels {
        let settings = AppSettings::get(cx);
        let window_width = window.viewport_size().width;
        match settings.layout {
            Layout::Split if !settings.focus_mode => {
                if measured > px(0.) && self.last_window_width > px(0.) {
                    measured * (window_width / self.last_window_width)
                } else if measured > px(0.) {
                    measured
                } else {
                    window_width / 2.
                }
            }
            _ => window_width,
        }
    }
}

impl Render for DocumentView {
    #[expect(
        clippy::too_many_lines,
        reason = "Document view layout and action registration"
    )]
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.last_window_width = window.viewport_size().width;

        if self.analysis_task.is_none()
            && let Some(anchor) = self.pending_anchor.take()
        {
            let document = cx.entity();
            window.defer(cx, move |window, cx| {
                document.update(cx, |this, cx| this.reveal_anchor(&anchor, window, cx));
            });
        }
        let settings = AppSettings::get(cx);
        let layout = if settings.focus_mode {
            Layout::Editor
        } else {
            settings.layout
        };
        let content = match layout {
            Layout::Editor => self.render_editor(window, cx).into_any_element(),
            Layout::Preview => self.render_preview(window, cx).into_any_element(),
            Layout::Split => h_resizable("document-split")
                .child(
                    resizable_panel()
                        .size_range(rems(16.).to_pixels(window.rem_size())..Pixels::MAX)
                        .child(self.render_editor(window, cx)),
                )
                .child(
                    resizable_panel()
                        .size_range(rems(16.).to_pixels(window.rem_size())..Pixels::MAX)
                        .child(self.render_preview(window, cx)),
                )
                .into_any_element(),
        };

        div()
            .key_context(DOCUMENT_CONTEXT)
            .size_full()
            .relative()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.expanded_table.is_some() {
                    match event.keystroke.key.as_str() {
                        "escape" => this.close_expanded_table(cx),
                        "down" => {
                            if let Some(table) = &this.expanded_table {
                                table.view.read(cx).list_state().scroll_by(px(60.));
                                table.view.update(cx, |_, cx| cx.notify());
                            }
                        }
                        "up" => {
                            if let Some(table) = &this.expanded_table {
                                table.view.read(cx).list_state().scroll_by(-px(60.));
                                table.view.update(cx, |_, cx| cx.notify());
                            }
                        }
                        "pagedown" | "space" => {
                            if let Some(table) = &this.expanded_table {
                                let list = table.view.read(cx).list_state().clone();
                                let h = list.viewport_bounds().size.height;
                                let d = if h > px(0.) { h * 0.85 } else { px(300.) };
                                list.scroll_by(d);
                                table.view.update(cx, |_, cx| cx.notify());
                            }
                        }
                        "pageup" => {
                            if let Some(table) = &this.expanded_table {
                                let list = table.view.read(cx).list_state().clone();
                                let h = list.viewport_bounds().size.height;
                                let d = if h > px(0.) { h * 0.85 } else { px(300.) };
                                list.scroll_by(-d);
                                table.view.update(cx, |_, cx| cx.notify());
                            }
                        }
                        "home" => {
                            if let Some(table) = &this.expanded_table {
                                table.view.read(cx).list_state().scroll_to(ListOffset {
                                    item_ix: 0,
                                    offset_in_item: px(0.),
                                });
                                table.view.update(cx, |_, cx| cx.notify());
                            }
                        }
                        "end" => {
                            if let Some(table) = &this.expanded_table {
                                table.view.read(cx).list_state().scroll_to_end();
                                table.view.update(cx, |_, cx| cx.notify());
                            }
                        }
                        _ => {}
                    }
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &TidyTable, window, cx| {
                this.tidy_table(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Bold, window, cx| {
                this.inline(format::Inline::Bold, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Italic, window, cx| {
                this.inline(format::Inline::Italic, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Strikethrough, window, cx| {
                this.inline(format::Inline::Strikethrough, window, cx);
            }))
            .on_action(cx.listener(|this, _: &InlineCode, window, cx| {
                this.inline(format::Inline::Code, window, cx);
            }))
            .on_action(cx.listener(|this, _: &InsertLink, window, cx| {
                this.apply_edit(format::insert_link, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CodeBlock, window, cx| {
                this.apply_edit(format::insert_code_block, window, cx);
            }))
            .on_action(cx.listener(|this, action: &Heading, window, cx| {
                this.block(format::Block::Heading(action.0), window, cx);
            }))
            .on_action(cx.listener(|this, _: &Quote, window, cx| {
                this.block(format::Block::Quote, window, cx);
            }))
            .on_action(cx.listener(|this, _: &BulletList, window, cx| {
                this.block(format::Block::BulletList, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NumberedList, window, cx| {
                this.block(format::Block::NumberedList, window, cx);
            }))
            .on_action(cx.listener(|this, _: &TaskList, window, cx| {
                this.block(format::Block::TaskList, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Search, window, cx| {
                let settings = AppSettings::get(cx);
                if settings.layout == Layout::Preview
                    || this.preview.read(cx).focus_handle().is_focused(window)
                    || this.search_open
                {
                    this.open_search(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &FindInPreview, window, cx| {
                this.open_search(window, cx);
            }))
            .on_action(cx.listener(Self::preview_scroll_down))
            .on_action(cx.listener(Self::preview_scroll_up))
            .on_action(cx.listener(Self::preview_page_down))
            .on_action(cx.listener(Self::preview_page_up))
            .on_action(cx.listener(Self::preview_scroll_top))
            .on_action(cx.listener(Self::preview_scroll_bottom))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.expanded_table.is_some() {
                    this.close_expanded_table(cx);
                } else if this.search_open {
                    this.close_search(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .child(content)
            .when_some(self.expanded_table.as_ref(), |this, table| {
                let base_dir = self.dir().map(Arc::<Path>::from);
                let extensions = self.markdown_extensions.clone();
                let doc = cx.weak_entity();
                let doc_close = doc.clone();
                let doc_link = doc;
                this.child(table.render(
                    base_dir,
                    extensions,
                    move |_, cx| {
                        _ = doc_close.update(cx, DocumentView::close_expanded_table);
                    },
                    move |url, event, window, cx| {
                        _ = doc_link.update(cx, DocumentView::close_expanded_table);
                        crate::links::on_click(url, event, window, cx);
                    },
                    cx,
                ))
            })
            .into_any_element()
    }
}

fn make_preview_style(cx: &App) -> gpui_kit::base::TextViewStyle {
    gpui_kit::base::TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
        .with_table(gpui_kit::StyleRefinement::default().max_h(px(280.)))
}

/// Horizontal inset that keeps text within a readable column, centered in a
/// pane of `width`, but never tighter than the standard pane padding.
fn column_inset(width: Pixels, cx: &App) -> Pixels {
    let rem = cx.theme().font_size;
    let min = rem * 1.5;
    let column = rem * READABLE_WIDTH_REMS;
    ((width - column) / 2.).max(min)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[gpui_kit::test]
    fn expands_and_closes_table_overlay(cx: &mut gpui_kit::TestAppContext) {
        let (doc, _) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            cx.set_global(AppSettings(crate::settings::Settings::default()));
            let table_md = "| A | B |\n|---|---|\n| 1 | 2 |";
            let mut view =
                DocumentView::new(table_md.to_string(), None, LineEnding::Lf, window, cx);
            assert!(!view.is_expanded_table_open());
            view.open_expanded_table(table_md, Some(0..table_md.len()), cx);
            assert!(view.is_expanded_table_open());
            assert_eq!(view.expanded_table.as_ref().unwrap().markdown, table_md);
            view.close_expanded_table(cx);
            assert!(!view.is_expanded_table_open());
            view
        });
        let _ = doc;
    }

    #[gpui_kit::test]
    fn pane_width_handles_resizing(cx: &mut gpui_kit::TestAppContext) {
        let (doc, _) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            cx.set_global(AppSettings(crate::settings::Settings {
                layout: Layout::Preview,
                ..Default::default()
            }));
            let mut view = DocumentView::new("hello".to_string(), None, LineEnding::Lf, window, cx);
            let viewport_w = window.viewport_size().width;
            assert_eq!(view.pane_width(px(100.), window, cx), viewport_w);

            cx.set_global(AppSettings(crate::settings::Settings {
                layout: Layout::Split,
                ..Default::default()
            }));
            view.last_window_width = px(1000.);
            let scaled = view.pane_width(px(500.), window, cx);
            let expected = px(500.) * (viewport_w / px(1000.));
            assert_eq!(scaled, expected);
            view
        });
        let _ = doc;
    }

    #[gpui_kit::test]
    fn live_editor_preview_debouncing(cx: &mut gpui_kit::TestAppContext) {
        let (doc, _) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            cx.set_global(AppSettings(crate::settings::Settings::default()));
            let mut view =
                DocumentView::new("initial text".to_string(), None, LineEnding::Lf, window, cx);
            assert_eq!(view.source().as_ref(), "initial text");
            assert_eq!(
                view.preview.read(cx).rendered_text().as_str().trim_end(),
                "initial text"
            );
            assert!(view.preview_task.is_none());

            view.editor.update(cx, |editor, cx| {
                editor.set_value("updated during typing".to_string(), window, cx);
            });
            view.on_text_changed(DebouncePolicy::Debounced, cx);

            assert_eq!(view.source().as_ref(), "updated during typing");
            assert_eq!(
                view.preview.read(cx).rendered_text().as_str().trim_end(),
                "initial text"
            );
            assert!(view.preview_task.is_some());

            view.flush_preview(cx);
            assert_eq!(
                view.preview.read(cx).rendered_text().as_str().trim_end(),
                "updated during typing"
            );
            assert!(view.preview_task.is_none());

            view.editor.update(cx, |editor, cx| {
                editor.set_value("final text".to_string(), window, cx);
            });
            view.on_text_changed(DebouncePolicy::Debounced, cx);
            assert_eq!(
                view.preview.read(cx).rendered_text().as_str().trim_end(),
                "updated during typing"
            );
            assert!(view.preview_task.is_some());

            view
        });

        cx.background_executor.advance_clock(PREVIEW_DEBOUNCE);
        cx.run_until_parked();

        doc.read_with(cx, |view, cx| {
            assert_eq!(
                view.preview.read(cx).rendered_text().as_str().trim_end(),
                "final text"
            );
            assert!(view.preview_task.is_none());
        });
    }

    #[gpui_kit::test]
    fn preview_keyboard_scroll_actions(cx: &mut gpui_kit::TestAppContext) {
        let (doc, _) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            cx.set_global(AppSettings(crate::settings::Settings::default()));
            let long_text = (0..100).fold(String::new(), |mut acc, i| {
                let _ = writeln!(acc, "Line {i}");
                acc
            });
            let mut view = DocumentView::new(long_text, None, LineEnding::Lf, window, cx);

            view.preview_scroll_down(&crate::actions::PreviewScrollDown, window, cx);
            assert_eq!(view.scroll_sync.driver(), ScrollDriver::Preview);
            assert!(view.smooth_scroll.pending_delta > 0.0);

            view.smooth_scroll.reset();
            view.preview_scroll_up(&crate::actions::PreviewScrollUp, window, cx);
            assert_eq!(view.scroll_sync.driver(), ScrollDriver::Preview);
            assert!(view.smooth_scroll.pending_delta < 0.0);

            view.smooth_scroll.reset();
            view.preview_page_down(&crate::actions::PreviewPageDown, window, cx);
            assert_eq!(view.scroll_sync.driver(), ScrollDriver::Preview);
            assert!(view.smooth_scroll.pending_delta > 0.0);

            view.smooth_scroll.reset();
            view.preview_page_up(&crate::actions::PreviewPageUp, window, cx);
            assert_eq!(view.scroll_sync.driver(), ScrollDriver::Preview);
            assert!(view.smooth_scroll.pending_delta < 0.0);

            view.preview_scroll_bottom(&crate::actions::PreviewScrollBottom, window, cx);
            assert_eq!(view.scroll_sync.driver(), ScrollDriver::Preview);

            view.preview_scroll_top(&crate::actions::PreviewScrollTop, window, cx);
            assert_eq!(view.scroll_sync.driver(), ScrollDriver::Preview);

            view
        });
        let _ = doc;
    }
}
