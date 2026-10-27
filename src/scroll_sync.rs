//! Bidirectional scroll synchronization between Markdown editor and preview.
//!
//! Tracks which pane is actively driving (the editor while typing/scrolling or
//! the preview under pointer interaction) and synchronizes position using AST
//! block-to-line mappings without triggering infinite feedback loops.

use gpui_kit::{
    App, Entity, ListOffset, Pixels,
    base::TextViewState,
    component::{RopeExt as _, input::EditorState},
    point, px,
};

use crate::{analysis::Analysis, settings::Layout, themes::AppSettings};

/// Estimated jumps allowed before giving up on landing a target exactly.
pub(crate) const MAX_EDITOR_SCROLL_ATTEMPTS: u8 = 4;

/// Which pane the other one follows while scroll sync is on.
///
/// The pane under the pointer, or the editor while typing, drives: its
/// scrolling moves the other pane, and scrolling it causes (the follower's
/// programmatic moves) never feed back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScrollDriver {
    #[default]
    Editor,
    Preview,
}

/// A source position the editor is being scrolled to.
///
/// Lines off screen have no layout to measure, so the editor first jumps to
/// an estimate; once the line is laid out, the next pass lands on it exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EditorScrollTarget {
    /// Zero-based source line, with the fraction through it.
    pub(crate) line: f32,
    pub(crate) attempts: u8,
}

/// Manages bidirectional scroll synchronization state between the editor and preview panes.
#[derive(Debug, Default)]
pub(crate) struct ScrollSynchronizer {
    pub(crate) driver: ScrollDriver,
    /// Editor scroll position the preview was last aligned with.
    pub(crate) synced_editor_scroll: Option<(usize, Pixels)>,
    /// Preview scroll position the editor was last aligned with.
    pub(crate) synced_preview_scroll: Option<ListOffset>,
    pub(crate) editor_scroll_target: Option<EditorScrollTarget>,
    /// Row count of the preview when it was last aligned.
    pub(crate) synced_preview_rows: usize,
}

impl ScrollSynchronizer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "driver getter for scroll sync"))]
    pub(crate) fn driver(&self) -> ScrollDriver {
        self.driver
    }

    pub(crate) fn set_driver(&mut self, driver: ScrollDriver) {
        if self.driver != driver {
            self.driver = driver;
            // A pending jump belongs to the pane that stopped driving.
            self.editor_scroll_target = None;
        }
    }

    pub(crate) fn synced_preview_rows(&self) -> usize {
        self.synced_preview_rows
    }

    pub(crate) fn reset_editor_scroll(&mut self) {
        self.synced_editor_scroll = None;
    }

    pub(crate) fn syncs_scrolling(cx: &App) -> bool {
        let settings = AppSettings::get(cx);
        settings.scroll_sync && settings.layout == Layout::Split && !settings.focus_mode
    }

    /// Scroll the preview to the block at the top of the editor.
    ///
    /// Runs when the editor scrolls or the document's structure changes. While
    /// the preview drives, it only records where the editor is, so the
    /// editor's catching up is not mistaken for the user scrolling it.
    #[expect(
        clippy::fn_params_excessive_bools,
        clippy::cast_precision_loss,
        clippy::as_conversions,
        reason = "Scroll interpolation and line offset conversions"
    )]
    pub(crate) fn sync_preview(
        &mut self,
        editor: &Entity<EditorState>,
        preview: &Entity<TextViewState>,
        analysis: &Analysis,
        force: bool,
        cx: &mut App,
    ) {
        if !Self::syncs_scrolling(cx) {
            return;
        }
        let (rows, offset_y) = {
            let ed = editor.read(cx);
            let Some(rows) = ed.visible_row_range() else {
                return;
            };
            (rows, ed.scroll_offset().y)
        };
        let position = (rows.start, offset_y);
        if self.driver != ScrollDriver::Editor {
            self.synced_editor_scroll = Some(position);
            return;
        }
        if !force && self.synced_editor_scroll == Some(position) {
            return;
        }
        self.synced_editor_scroll = Some(position);

        let list = preview.read(cx).list_state().clone();
        let count = list.item_count();
        self.synced_preview_rows = count;
        if count == 0 {
            return;
        }

        if offset_y >= px(-0.5) {
            list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        } else if analysis.blocks.len() == count
            && let Some((ix, fraction)) = analysis.blocks.block_at_line(rows.start as f32)
        {
            // Rows above the scroll top report no bounds, so land on the row
            // first, then measure it for the offset inside.
            list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
            let height = list
                .bounds_for_item(ix)
                .map_or(px(0.), |bounds| bounds.size.height);
            list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: height * fraction,
            });
        } else {
            // The structure is still being analyzed: approximate by position.
            let lines = analysis.stats.lines.max(1) as f32;
            let fraction = (rows.start as f32 / lines).clamp(0., 1.);
            let max = list.max_offset_for_scrollbar().y;
            list.set_offset_from_scrollbar(point(px(0.), -(max * fraction)));
        }
        self.synced_preview_scroll = Some(list.logical_scroll_top());
        preview.update(cx, |_, cx| cx.notify());
    }

    /// Scroll the editor to the source of the block at the top of the
    /// preview. Runs after every frame the preview paints, so it catches
    /// wheel, trackpad, scrollbar and keyboard scrolling alike.
    #[expect(
        clippy::cast_precision_loss,
        clippy::as_conversions,
        reason = "Scroll interpolation and line offset conversions"
    )]
    pub(crate) fn sync_editor(
        &mut self,
        preview: &Entity<TextViewState>,
        editor: &Entity<EditorState>,
        analysis: &Analysis,
        cx: &mut App,
    ) {
        if !Self::syncs_scrolling(cx) || self.driver != ScrollDriver::Preview {
            return;
        }
        let (list, top) = {
            let pr = preview.read(cx);
            let list = pr.list_state().clone();
            let top = list.logical_scroll_top();
            (list, top)
        };
        let moved = self.synced_preview_scroll.is_none_or(|synced| {
            synced.item_ix != top.item_ix
                || (synced.offset_in_item - top.offset_in_item).abs() > px(0.5)
        });
        if moved {
            self.synced_preview_scroll = Some(top);
            let blocks = &analysis.blocks;
            let line = if top.item_ix == 0 && top.offset_in_item <= px(0.5) {
                Some(0.)
            } else if blocks.len() == list.item_count() {
                // The top row is at the scroll top, so its bounds are known.
                let height = list
                    .bounds_for_item(top.item_ix)
                    .map_or(px(0.), |bounds| bounds.size.height);
                let fraction = if height > px(0.) {
                    top.offset_in_item / height
                } else {
                    0.
                };
                blocks.line_at_block(top.item_ix, fraction)
            } else {
                let max = list.max_offset_for_scrollbar().y;
                let scrolled = -list.scroll_px_offset_for_scrollbar().y;
                let fraction = if max > px(0.) { scrolled / max } else { 0. };
                Some(fraction.clamp(0., 1.) * analysis.stats.lines as f32)
            };
            self.editor_scroll_target = line.map(|line| EditorScrollTarget { line, attempts: 0 });
        }
        self.scroll_editor_to_target(editor, cx);
    }

    /// Move the editor toward [`Self::editor_scroll_target`]: exactly when
    /// the target line is laid out, by estimate otherwise.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "Scroll interpolation and line offset conversions"
    )]
    pub(crate) fn scroll_editor_to_target(&mut self, editor: &Entity<EditorState>, cx: &mut App) {
        let Some(mut target) = self.editor_scroll_target else {
            return;
        };
        let mut landed = false;
        editor.update(cx, |editor, cx| {
            let Some(rows) = editor.visible_row_range() else {
                return;
            };
            let text = editor.text();
            let last_line = text.lines_len().saturating_sub(1);
            let line = (target.line.max(0.).floor() as usize).min(last_line);
            let within_line = (target.line - line as f32).clamp(0., 1.);
            let line_range = text.line_start_offset(line)..text.line_end_offset(line);
            let offset = editor.scroll_offset();

            let desired = if let (Some(line_bounds), Some(text_bounds)) =
                (editor.range_to_bounds(&line_range), editor.text_bounds())
            {
                // Content position of the line: measured from the text
                // origin, which moves with the scroll offset.
                landed = true;
                let top = line_bounds.top() - text_bounds.top();
                -(top + line_bounds.size.height * within_line)
            } else {
                // Off screen: estimate from the lines visible now.
                let visible_lines = rows.len().max(1) as f32;
                let per_line = editor.input_bounds().size.height / visible_lines;
                offset.y - per_line * (line as f32 + within_line - rows.start as f32)
            };
            let desired = desired.min(px(0.));
            if (desired - offset.y).abs() > px(0.5) {
                editor.set_scroll_offset(point(offset.x, desired), cx);
            }
        });
        target.attempts += 1;
        self.editor_scroll_target =
            (!landed && target.attempts < MAX_EDITOR_SCROLL_ATTEMPTS).then_some(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_switching_clears_pending_target() {
        let mut sync = ScrollSynchronizer::new();
        assert_eq!(sync.driver(), ScrollDriver::Editor);

        sync.editor_scroll_target = Some(EditorScrollTarget {
            line: 42.0,
            attempts: 1,
        });

        sync.set_driver(ScrollDriver::Preview);
        assert_eq!(sync.driver(), ScrollDriver::Preview);
        assert!(sync.editor_scroll_target.is_none());

        sync.editor_scroll_target = Some(EditorScrollTarget {
            line: 10.0,
            attempts: 0,
        });
        sync.set_driver(ScrollDriver::Preview); // No change
        assert!(sync.editor_scroll_target.is_some());

        sync.set_driver(ScrollDriver::Editor);
        assert_eq!(sync.driver(), ScrollDriver::Editor);
        assert!(sync.editor_scroll_target.is_none());
    }

    #[test]
    fn test_editor_scroll_target_attempts_exhaustion() {
        let mut target = EditorScrollTarget {
            line: 15.5,
            attempts: 3,
        };
        target.attempts += 1;
        let next = (target.attempts < MAX_EDITOR_SCROLL_ATTEMPTS).then_some(target);
        assert!(next.is_none());
    }
}
