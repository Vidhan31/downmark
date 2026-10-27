//! Expanded table modal overlay.
//!
//! Renders a zoomed card displaying a Markdown table with interactive scrolling,
//! selectable text, link clicking, and copy-to-clipboard functionality.

use std::{path::Path, sync::Arc};

use gpui_kit::{
    App, AppContext as _, ClickEvent, ClipboardItem, Entity, FontWeight, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, ScrollEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::{TextView, TextViewState},
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div, rems,
};

use crate::images;

pub(crate) struct ExpandedTable {
    pub(crate) markdown: SharedString,
    pub(crate) view: Entity<TextViewState>,
}

impl ExpandedTable {
    pub(crate) fn new(markdown: SharedString, cx: &mut App) -> Self {
        let view = cx.new(|cx| TextViewState::markdown(&markdown, cx));
        Self { markdown, view }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI layout for expanded table overlay"
    )]
    pub(crate) fn render(
        &self,
        base_dir: Option<Arc<Path>>,
        markdown_extensions: gpui_kit::component::text::MarkdownExtensions,
        on_close: impl Fn(&mut Window, &mut App) + 'static + Send + Sync,
        on_link_click: impl Fn(&SharedString, &ClickEvent, &mut Window, &mut App)
        + 'static
        + Send
        + Sync,
        cx: &App,
    ) -> impl IntoElement {
        let markdown_copy = self.markdown.clone();
        let on_close = Arc::new(on_close);
        let on_close_click = Arc::clone(&on_close);
        let on_close_backdrop = Arc::clone(&on_close);

        div()
            .id("expanded-table-overlay")
            .occlude()
            .absolute()
            .inset_0()
            .bg(cx.theme().background.opacity(0.75))
            .flex()
            .items_center()
            .justify_center()
            .p_8()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                on_close_backdrop(window, cx);
            })
            .on_scroll(|_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                v_flex()
                    .id("expanded-table-card")
                    .role(gpui_kit::Role::Dialog)
                    .aria_label("Expanded table")
                    .occlude()
                    .w_full()
                    .h(rems(42.))
                    .max_h_full()
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_xl()
                    .shadow_2xl()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_scroll({
                        let view = self.view.clone();
                        move |event: &ScrollEvent, window, cx| {
                            cx.stop_propagation();
                            let list = view.read(cx).list_state().clone();
                            if list.viewport_bounds().contains(&event.position) {
                                return;
                            }
                            let delta = event.delta.pixel_delta(window.line_height());
                            list.scroll_by(-delta.y);
                            view.update(cx, |_, cx| cx.notify());
                        }
                    })
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .px_5()
                            .pt_3p5()
                            .pb_2()
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
                                                    markdown_copy.to_string(),
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
                                            .on_click(move |_, window, cx| {
                                                on_close_click(window, cx);
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        div().flex_1().min_h_0().size_full().px_5().pb_5().child(
                            TextView::new(&self.view)
                                .size_full()
                                .scrollable(true)
                                .selectable(true)
                                .markdown_extensions(markdown_extensions)
                                .image_source(move |url| images::resolve(url, base_dir.as_deref()))
                                .on_link_click(move |url, event, window, cx| {
                                    on_link_click(url, event, window, cx);
                                }),
                        ),
                    ),
            )
    }
}
