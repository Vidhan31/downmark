//! Expanded table modal overlay.
//!
//! Renders a zoomed card displaying a Markdown table with interactive scrolling,
//! selectable text, link clicking, and copy-to-clipboard functionality.

use std::{path::Path, sync::Arc};

use gpui_kit::{
    App, AppContext as _, ClickEvent, ClipboardItem, FontWeight, InteractiveElement as _,
    ParentElement as _, SharedString, Styled as _, Window,
    assets::IconName,
    base::{TextView, TextViewState},
    component::{
        ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::Cancel,
        h_flex, v_flex,
    },
    div, px,
};

use crate::images;

/// Show the expanded Markdown table in a modal dialog.
#[expect(
    clippy::too_many_lines,
    reason = "Declarative GPUI modal dialog layout for table"
)]
pub(crate) fn open(
    markdown: SharedString,
    base_dir: Option<Arc<Path>>,
    markdown_extensions: gpui_kit::component::text::MarkdownExtensions,
    on_link_click: impl Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + 'static + Send + Sync,
    window: &mut Window,
    cx: &mut App,
) {
    if window.has_active_dialog(cx) {
        return;
    }
    let view = cx.new(|cx| TextViewState::markdown(&markdown, cx));
    let on_link_click = Arc::new(on_link_click);
    let focus_handle = cx.focus_handle();
    let initial_focus = std::cell::Cell::new(true);

    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            focus_handle.focus(window, cx);
        }

        let view_size = window.viewport_size();
        let width = (view_size.width * 0.88)
            .min(px(1200.))
            .min((view_size.width - px(32.)).max(px(320.)));
        let height = (view_size.height * 0.82)
            .min(px(850.))
            .min((view_size.height - px(32.)).max(px(240.)));

        let markdown_copy = markdown.clone();
        let on_link = Arc::clone(&on_link_click);
        let table_view = view.clone();
        let base_dir = base_dir.clone();
        let extensions = markdown_extensions.clone();
        let container_focus = focus_handle.clone();

        dialog
            .w(width)
            .h(height)
            .centered()
            .close_button(false)
            .p_0()
            .content(move |content, _window, cx| {
                let markdown_for_copy = markdown_copy.clone();
                let base_dir_for_img = base_dir.clone();
                let on_link_for_click = Arc::clone(&on_link);
                content
                    .h(height)
                    .size_full()
                    .p_0()
                    .child(
                        v_flex()
                            .id("expanded-table-container")
                            .track_focus(&container_focus)
                            .size_full()
                            .overflow_hidden()
                            .on_action(|_: &gpui_kit::component::input::Escape, window, cx| {
                                window.close_dialog(cx);
                            })
                            .on_action(|_: &Cancel, window, cx| {
                                window.close_dialog(cx);
                            })
                            .on_action(|_: &crate::actions::CloseWindow, window, cx| {
                                window.close_dialog(cx);
                            })
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .px_4()
                                    .py_2p5()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(cx.theme().foreground)
                                            .child("Table"),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                Button::new("copy-expanded-table")
                                                    .accessibility_label("Copy table Markdown")
                                                    .xsmall()
                                                    .ghost()
                                                    .icon(Icon::new(IconName::Copy))
                                                    .tooltip("Copy table Markdown")
                                                    .on_click(move |_, _, cx| {
                                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                                            markdown_for_copy.to_string(),
                                                        ));
                                                    }),
                                            )
                                            .child(
                                                Button::new("close-expanded-table")
                                                    .accessibility_label("Close expanded table (Esc)")
                                                    .xsmall()
                                                    .ghost()
                                                    .icon(Icon::new(IconName::Close))
                                                    .tooltip("Close (Esc)")
                                                    .on_click(|_, window, cx| {
                                                        window.close_dialog(cx);
                                                    }),
                                            ),
                                    ),
                            )
                            .child(
                                div().flex_1().min_h_0().size_full().p_4().child(
                                    TextView::new(&table_view)
                                        .size_full()
                                        .scrollable(true)
                                        .selectable(true)
                                        .markdown_extensions(extensions.clone())
                                        .image_source(move |url| images::resolve(url, base_dir_for_img.as_deref()))
                                        .on_link_click(move |url, event, window, cx| {
                                            window.close_dialog(cx);
                                            on_link_for_click(url, event, window, cx);
                                        }),
                                ),
                            ),
                    )
            })
    });
}
