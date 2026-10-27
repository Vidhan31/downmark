//! Search overlay UI component.
//!
//! Renders the floating search bar for searching within the Markdown preview,
//! including query input, match counter badge, next/prev buttons, and dismiss action.

use std::sync::Arc;

use gpui_kit::{
    App, Entity, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

pub(crate) fn render_search_overlay(
    search_input: &Entity<InputState>,
    match_display: &str,
    total_matches: usize,
    on_prev: impl Fn(&Window, &mut App) + 'static + Send + Sync,
    on_next: impl Fn(&Window, &mut App) + 'static + Send + Sync,
    on_close: impl Fn(&mut Window, &mut App) + 'static + Send + Sync,
    cx: &App,
) -> impl IntoElement {
    let on_close = Arc::new(on_close);
    let on_close_esc = Arc::clone(&on_close);
    let on_close_btn = Arc::clone(&on_close);
    let match_display = match_display.to_string();

    div()
        .id("preview-search-overlay")
        .role(gpui_kit::Role::Search)
        .aria_label("Search in preview")
        .absolute()
        .top_3()
        .right_6()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .capture_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                on_close_esc(window, cx);
            }
        })
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .bg(cx.theme().background)
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .shadow_lg()
                .child(
                    div()
                        .w(px(200.))
                        .child(Input::new(search_input).appearance(false).cleanable(true)),
                )
                .when(!match_display.is_empty(), |el| {
                    el.child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if total_matches > 0 {
                                cx.theme().muted_foreground
                            } else {
                                cx.theme().danger
                            })
                            .child(match_display),
                    )
                })
                .child(div().h(px(16.)).w(px(1.)).bg(cx.theme().border))
                .child(
                    Button::new("preview-search-prev")
                        .accessibility_label("Previous search match (Shift+Enter)")
                        .ghost()
                        .compact()
                        .xsmall()
                        .icon(Icon::new(IconName::ChevronUp))
                        .tooltip("Previous match (Shift+Enter)")
                        .disabled(total_matches == 0)
                        .on_click(move |_, window, cx| {
                            on_prev(window, cx);
                        }),
                )
                .child(
                    Button::new("preview-search-next")
                        .accessibility_label("Next search match (Enter)")
                        .ghost()
                        .compact()
                        .xsmall()
                        .icon(Icon::new(IconName::ChevronDown))
                        .tooltip("Next match (Enter)")
                        .disabled(total_matches == 0)
                        .on_click(move |_, window, cx| {
                            on_next(window, cx);
                        }),
                )
                .child(
                    Button::new("preview-search-close")
                        .accessibility_label("Close search (Esc)")
                        .ghost()
                        .compact()
                        .xsmall()
                        .icon(Icon::new(IconName::Close))
                        .tooltip("Close search (Esc)")
                        .on_click(move |_, window, cx| {
                            on_close_btn(window, cx);
                        }),
                ),
        )
}
