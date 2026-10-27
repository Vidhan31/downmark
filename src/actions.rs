//! Commands and their key bindings.
//!
//! Every command is an Action so the title bar, the menu bar, the editor's
//! context menu and the keyboard all dispatch the same thing.

use gpui_kit::{Action, App, KeyBinding, SharedString, actions};
use serde::Deserialize;

use crate::settings::{Appearance, Layout};

actions!(
    downmark,
    [
        NewFile,
        NewTab,
        Open,
        Save,
        SaveAs,
        RevealInFolder,
        CloseDocument,
        CloseWindow,
        NextTab,
        PreviousTab,
        Quit,
        About,
        OpenMarkdownGuide,
        ToggleScrollSync,
        ToggleSoftWrap,
        ToggleLineNumbers,
        ToggleAppearance,
        ToggleTabs,
        ToggleFocusMode,
        ToggleRestoreSession,
        OpenSettings,
        TidyTable,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        GoToHeading,
        Bold,
        Italic,
        Strikethrough,
        InlineCode,
        InsertLink,
        CodeBlock,
        Quote,
        BulletList,
        NumberedList,
        TaskList,
        FindInPreview,
        TogglePaneFocus,
        PreviewScrollDown,
        PreviewScrollUp,
        PreviewPageDown,
        PreviewPageUp,
        PreviewScrollTop,
        PreviewScrollBottom,
    ]
);

/// Show the editor, the preview, or both.
#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = downmark, no_json)]
pub(crate) struct SetLayout(pub(crate) Layout);

/// Turn the current lines into a heading of the given level.
#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = downmark, no_json)]
pub(crate) struct Heading(pub(crate) u8);

/// Follow the system appearance, or force light or dark.
#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = downmark, no_json)]
pub(crate) struct SetAppearance(pub(crate) Appearance);

/// Use the named theme for its appearance (light or dark).
#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = downmark, no_json)]
pub(crate) struct SetTheme(pub(crate) SharedString);

/// Open an entry of the recent files list.
#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = downmark, no_json)]
pub(crate) struct OpenRecent(pub(crate) usize);

/// Show the open document at this index.
#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = downmark, no_json)]
pub(crate) struct ActivateTab(pub(crate) usize);

/// Follow a link clicked in the preview. `alternate` opens a linked
/// document the other way from the setting (a new tab or the same one).
#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = downmark, no_json)]
pub(crate) struct FollowLink {
    pub(crate) url: SharedString,
    pub(crate) alternate: bool,
}

/// Show the Mermaid diagram with this source in its own window.
#[derive(Action, Clone, PartialEq, Deserialize)]
#[action(namespace = downmark, no_json)]
pub(crate) struct ViewDiagram(pub(crate) SharedString);

/// The key context of the workspace view.
pub(crate) const WORKSPACE_CONTEXT: &str = "Workspace";

/// The key context of a markdown document view.
pub(crate) const DOCUMENT_CONTEXT: &str = "Document";

/// The key context of the markdown preview pane.
pub(crate) const PREVIEW_CONTEXT: &str = "Preview";

/// Constructs a key binding, resolving `secondary-` to `cmd-` on macOS and
/// `ctrl-` on other platforms.
pub(crate) fn key_binding<A: Action>(
    keystrokes: &str,
    action: A,
    context: Option<&'static str>,
) -> KeyBinding {
    #[cfg(target_os = "macos")]
    let resolved = keystrokes.replace("secondary-", "cmd-");
    #[cfg(not(target_os = "macos"))]
    let resolved = keystrokes.replace("secondary-", "ctrl-");

    KeyBinding::new(Box::leak(resolved.into_boxed_str()), action, context)
}

pub(crate) fn bind_keys(cx: &mut App) {
    let workspace = Some(WORKSPACE_CONTEXT);
    let document = Some(DOCUMENT_CONTEXT);
    let preview = Some(PREVIEW_CONTEXT);
    // `secondary` is Command on macOS and Control elsewhere.
    cx.bind_keys([
        key_binding("secondary-n", NewFile, None),
        key_binding("secondary-t", NewTab, None),
        key_binding("secondary-o", Open, None),
        key_binding("secondary-s", Save, workspace),
        key_binding("secondary-shift-s", SaveAs, workspace),
        key_binding("secondary-w", CloseDocument, workspace),
        key_binding("secondary-shift-w", CloseWindow, workspace),
        key_binding("ctrl-tab", NextTab, workspace),
        key_binding("ctrl-shift-tab", PreviousTab, workspace),
        #[cfg(target_os = "macos")]
        key_binding("cmd-shift-]", NextTab, workspace),
        #[cfg(target_os = "macos")]
        key_binding("cmd-shift-[", PreviousTab, workspace),
        #[cfg(not(target_os = "macos"))]
        key_binding("ctrl-pagedown", NextTab, workspace),
        #[cfg(not(target_os = "macos"))]
        key_binding("ctrl-pageup", PreviousTab, workspace),
        key_binding("secondary-shift-f", ToggleFocusMode, workspace),
        key_binding("secondary-,", OpenSettings, None),
        key_binding("secondary-q", Quit, None),
        key_binding("secondary-1", SetLayout(Layout::Editor), workspace),
        key_binding("secondary-2", SetLayout(Layout::Split), workspace),
        key_binding("secondary-3", SetLayout(Layout::Preview), workspace),
        // The last binding for an action is the one menus display.
        key_binding("secondary-+", ZoomIn, None),
        key_binding("secondary-=", ZoomIn, None),
        key_binding("secondary--", ZoomOut, None),
        key_binding("secondary-0", ZoomReset, None),
        key_binding("secondary-shift-l", ToggleAppearance, None),
        key_binding("secondary-shift-o", GoToHeading, workspace),
        key_binding("secondary-b", Bold, document),
        key_binding("secondary-i", Italic, document),
        key_binding("secondary-shift-x", Strikethrough, document),
        key_binding("secondary-e", InlineCode, document),
        key_binding("secondary-k", InsertLink, document),
        key_binding("secondary-shift-c", CodeBlock, document),
        key_binding("secondary-shift-.", Quote, document),
        key_binding("secondary-shift-8", BulletList, document),
        key_binding("secondary-shift-7", NumberedList, document),
        key_binding("secondary-shift-9", TaskList, document),
        key_binding("secondary-alt-1", Heading(1), document),
        key_binding("secondary-alt-2", Heading(2), document),
        key_binding("secondary-alt-3", Heading(3), document),
        key_binding("secondary-alt-4", Heading(4), document),
        key_binding("secondary-alt-5", Heading(5), document),
        key_binding("secondary-alt-6", Heading(6), document),
        key_binding("secondary-f", gpui_kit::component::input::Search, workspace),
        key_binding("secondary-alt-f", FindInPreview, document),
        key_binding("f6", TogglePaneFocus, workspace),
        key_binding("secondary-alt-p", TogglePaneFocus, workspace),
        key_binding("down", PreviewScrollDown, preview),
        key_binding("up", PreviewScrollUp, preview),
        key_binding("pagedown", PreviewPageDown, preview),
        key_binding("pageup", PreviewPageUp, preview),
        key_binding("space", PreviewPageDown, preview),
        key_binding("shift-space", PreviewPageUp, preview),
        key_binding("home", PreviewScrollTop, preview),
        key_binding("end", PreviewScrollBottom, preview),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn binds_all_keys_without_error(cx: &mut TestAppContext) {
        cx.update(|cx| {
            bind_keys(cx);
            crate::diagram_viewer::bind_keys(cx);
        });
    }
}
