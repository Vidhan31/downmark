//! Expanded table modal overlay.
//!
//! Renders a full-width modal displaying a Markdown table with interactive scrolling,
//! selectable text, link clicking, and keyboard navigation.

use std::{path::Path, sync::Arc};

use gpui_kit::{
    App, ClickEvent, InteractiveElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
    base::TextView,
    component::{WindowExt as _, dialog::Cancel, v_flex},
    div,
    prelude::FluentBuilder as _,
    px,
};

use crate::{images, preview_ext};

/// Show the expanded Markdown table in a modal dialog spanning the full app width.
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
    let on_link_click = Arc::new(on_link_click);
    let focus_handle = cx.focus_handle();
    let scroll_handle = gpui_kit::ScrollHandle::new();

    window.open_dialog(cx, move |dialog, window, cx| {
        if !focus_handle.is_focused(window) && !focus_handle.contains_focused(window, cx) {
            focus_handle.focus(window, cx);
        }

        let view_size = window.viewport_size();
        let width = view_size.width;
        let height = (view_size.height * 0.88)
            .min(px(900.))
            .min((view_size.height - px(32.)).max(px(240.)));

        let markdown_view = markdown.clone();
        let on_link = Arc::clone(&on_link_click);
        let base_dir = base_dir.clone();
        let extensions = markdown_extensions.clone();
        let container_focus = focus_handle.clone();
        let table_width = preview_ext::table_content_width(&markdown);
        let scroll_handle = scroll_handle.clone();

        dialog
            .w(width)
            .h(height)
            .centered()
            .close_button(false)
            .p_0()
            .content(move |content, _window, cx| {
                let markdown_for_view = markdown_view.clone();
                let base_dir_for_img = base_dir.clone();
                let on_link_for_click = Arc::clone(&on_link);
                let scroll_handle_for_keys = scroll_handle.clone();
                let scroll_handle_for_wheel = scroll_handle.clone();
                let scroll_handle_for_scrollbar_h = scroll_handle.clone();
                let scroll_handle_for_scrollbar_v = scroll_handle.clone();
                let focus_for_click = container_focus.clone();
                content.h(height).size_full().p_0().child(
                    v_flex()
                        .id("expanded-table-container")
                        .track_focus(&container_focus)
                        .size_full()
                        .overflow_hidden()
                        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                            window.focus(&focus_for_click, cx);
                        })
                        .on_action(|_: &gpui_kit::component::input::Escape, window, cx| {
                            window.close_dialog(cx);
                        })
                        .on_action(|_: &Cancel, window, cx| {
                            window.close_dialog(cx);
                        })
                        .on_action(|_: &crate::actions::CloseWindow, window, cx| {
                            window.close_dialog(cx);
                        })
                        .capture_key_down(move |event, window, cx| {
                            let key = event.keystroke.key.to_lowercase();
                            let mut offset = scroll_handle_for_keys.offset();
                            let max = scroll_handle_for_keys.max_offset();
                            match key.as_str() {
                                "left" | "arrowleft" | "h" => {
                                    offset.x = (offset.x + px(120.)).min(px(0.));
                                    scroll_handle_for_keys.set_offset(offset);
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                                "right" | "arrowright" | "l" => {
                                    offset.x = (offset.x - px(120.)).max(-max.x);
                                    scroll_handle_for_keys.set_offset(offset);
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                                "up" | "arrowup" | "k" => {
                                    offset.y = (offset.y + px(80.)).min(px(0.));
                                    scroll_handle_for_keys.set_offset(offset);
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                                "down" | "arrowdown" | "j" => {
                                    offset.y = (offset.y - px(80.)).max(-max.y);
                                    scroll_handle_for_keys.set_offset(offset);
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                                "home" => {
                                    offset.x = px(0.);
                                    offset.y = px(0.);
                                    scroll_handle_for_keys.set_offset(offset);
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                                "end" => {
                                    offset.x = -max.x;
                                    offset.y = -max.y;
                                    scroll_handle_for_keys.set_offset(offset);
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                                "pageup" => {
                                    offset.y = (offset.y + px(300.)).min(px(0.));
                                    scroll_handle_for_keys.set_offset(offset);
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                                "pagedown" => {
                                    offset.y = (offset.y - px(300.)).max(-max.y);
                                    scroll_handle_for_keys.set_offset(offset);
                                    window.refresh();
                                    cx.stop_propagation();
                                }
                                _ => {}
                            }
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .size_full()
                                .relative()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .id("table-overlay-scroll-viewport")
                                        .size_full()
                                        .overflow_x_scroll()
                                        .overflow_y_scroll()
                                        .track_scroll(&scroll_handle)
                                        .on_scroll(move |event, window, _cx| {
                                            let delta = event.delta.pixel_delta(px(16.));
                                            let mut offset = scroll_handle_for_wheel.offset();
                                            let max = scroll_handle_for_wheel.max_offset();
                                            if event.modifiers.shift {
                                                let h_delta = if delta.x == px(0.) {
                                                    delta.y
                                                } else {
                                                    delta.x
                                                };
                                                if h_delta != px(0.) {
                                                    offset.x =
                                                        (offset.x + h_delta).clamp(-max.x, px(0.));
                                                    scroll_handle_for_wheel.set_offset(offset);
                                                    window.refresh();
                                                }
                                            } else {
                                                let mut changed = false;
                                                if delta.x != px(0.) {
                                                    offset.x =
                                                        (offset.x + delta.x).clamp(-max.x, px(0.));
                                                    changed = true;
                                                }
                                                if delta.y != px(0.) {
                                                    offset.y =
                                                        (offset.y + delta.y).clamp(-max.y, px(0.));
                                                    changed = true;
                                                }
                                                if changed {
                                                    scroll_handle_for_wheel.set_offset(offset);
                                                    window.refresh();
                                                }
                                            }
                                        })
                                        .p_4()
                                        .child({
                                            let table_view_style = crate::themes::preview_style(cx)
                                                .with_table(gpui_kit::StyleRefinement::default());
                                            div()
                                                .min_w_full()
                                                .when(table_width > 0.0, |this| {
                                                    this.w(px(table_width))
                                                })
                                                .child(
                                                    TextView::markdown(
                                                        "table-overlay-view",
                                                        markdown_for_view,
                                                    )
                                                    .selectable(true)
                                                    .style(table_view_style)
                                                    .markdown_extensions(extensions.clone())
                                                    .image_source(move |url| {
                                                        images::resolve(
                                                            url,
                                                            base_dir_for_img.as_deref(),
                                                        )
                                                    })
                                                    .on_link_click(move |url, event, window, cx| {
                                                        window.close_dialog(cx);
                                                        on_link_for_click(url, event, window, cx);
                                                    })
                                                    .w_full(),
                                                )
                                        }),
                                )
                                .child(
                                    gpui_kit::base::Scrollbar::horizontal(
                                        &scroll_handle_for_scrollbar_h,
                                    )
                                    .viewport_from_layout(),
                                )
                                .child(
                                    gpui_kit::base::Scrollbar::vertical(
                                        &scroll_handle_for_scrollbar_v,
                                    )
                                    .viewport_from_layout(),
                                ),
                        ),
                )
            })
    });
}
