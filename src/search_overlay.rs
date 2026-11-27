//! Search overlay UI component.
//!
//! Renders the floating search bar for searching within the Markdown preview,
//! including query input, match options (case, whole word, regex), match counter
//! badge, next/prev buttons, and dismiss action.

use std::sync::Arc;

use gpui_kit::{
    App, Entity, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, RenderOnce, StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

use crate::search::{SearchOption, SearchOptions};

type WindowAction = Arc<dyn Fn(&Window, &mut App) + Send + Sync>;
type WindowMutAction = Arc<dyn Fn(&mut Window, &mut App) + Send + Sync>;
type OptionToggleAction = Arc<dyn Fn(SearchOption, &mut Window, &mut App) + Send + Sync>;

#[cfg(target_os = "macos")]
fn option_shortcut_matches(event: &KeyDownEvent) -> bool {
    let mods = &event.keystroke.modifiers;
    mods.platform && mods.alt && !mods.control && !mods.shift
}

#[cfg(not(target_os = "macos"))]
fn option_shortcut_matches(event: &KeyDownEvent) -> bool {
    let mods = &event.keystroke.modifiers;
    mods.alt && !mods.control && !mods.platform && !mods.shift
}

fn option_for_key(key: &str) -> Option<SearchOption> {
    match key {
        "c" => Some(SearchOption::CaseSensitive),
        "w" => Some(SearchOption::WholeWord),
        "r" => Some(SearchOption::Regex),
        _ => None,
    }
}

#[derive(IntoElement)]
pub(crate) struct SearchOverlay {
    search_input: Entity<InputState>,
    match_display: String,
    total_matches: usize,
    options: SearchOptions,
    error: Option<String>,
    on_toggle_option: Option<OptionToggleAction>,
    on_prev: WindowAction,
    on_next: WindowAction,
    on_close: WindowMutAction,
}

impl SearchOverlay {
    pub(crate) fn new(
        search_input: &Entity<InputState>,
        match_display: &str,
        total_matches: usize,
        on_prev: impl Fn(&Window, &mut App) + 'static + Send + Sync,
        on_next: impl Fn(&Window, &mut App) + 'static + Send + Sync,
        on_close: impl Fn(&mut Window, &mut App) + 'static + Send + Sync,
    ) -> Self {
        Self {
            search_input: search_input.clone(),
            match_display: match_display.to_string(),
            total_matches,
            options: SearchOptions::default(),
            error: None,
            on_toggle_option: None,
            on_prev: Arc::new(on_prev),
            on_next: Arc::new(on_next),
            on_close: Arc::new(on_close),
        }
    }

    pub(crate) fn options(mut self, options: SearchOptions) -> Self {
        self.options = options;
        self
    }

    pub(crate) fn error(mut self, error: Option<&str>) -> Self {
        self.error = error.map(str::to_string);
        self
    }

    pub(crate) fn on_toggle_option(
        mut self,
        handler: impl Fn(SearchOption, &mut Window, &mut App) + 'static + Send + Sync,
    ) -> Self {
        self.on_toggle_option = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for SearchOverlay {
    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI layout for floating search bar and option toggles"
    )]
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let on_close_esc = Arc::clone(&self.on_close);
        let on_close_btn = Arc::clone(&self.on_close);
        let on_toggle_key = self.on_toggle_option.clone();
        let on_toggle_btn = self.on_toggle_option;
        let on_prev = self.on_prev;
        let on_next = self.on_next;
        let match_display = self.match_display;
        let total_matches = self.total_matches;
        let options = self.options;
        let error = self.error;
        let has_error = error.is_some();
        let search_input = self.search_input;

        #[cfg(target_os = "macos")]
        let toggle_items = [
            (
                "preview-search-case",
                IconName::CaseSensitive,
                "Match case",
                "Match case (⌥⌘C)",
                options.case_sensitive,
                SearchOption::CaseSensitive,
            ),
            (
                "preview-search-word",
                IconName::WholeWord,
                "Match whole word",
                "Match whole word (⌥⌘W)",
                options.whole_word,
                SearchOption::WholeWord,
            ),
            (
                "preview-search-regex",
                IconName::Regex,
                "Use regular expression",
                "Use regular expression (⌥⌘R)",
                options.regex,
                SearchOption::Regex,
            ),
        ];

        #[cfg(not(target_os = "macos"))]
        let toggle_items = [
            (
                "preview-search-case",
                IconName::CaseSensitive,
                "Match case",
                "Match case (Alt+C)",
                options.case_sensitive,
                SearchOption::CaseSensitive,
            ),
            (
                "preview-search-word",
                IconName::WholeWord,
                "Match whole word",
                "Match whole word (Alt+W)",
                options.whole_word,
                SearchOption::WholeWord,
            ),
            (
                "preview-search-regex",
                IconName::Regex,
                "Use regular expression",
                "Use regular expression (Alt+R)",
                options.regex,
                SearchOption::Regex,
            ),
        ];

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
                    cx.stop_propagation();
                    return;
                }
                if option_shortcut_matches(event)
                    && let Some(option) = option_for_key(&event.keystroke.key.to_lowercase())
                    && let Some(ref on_toggle) = on_toggle_key
                {
                    on_toggle(option, window, cx);
                    cx.stop_propagation();
                }
            })
            .child(
                v_flex()
                    .bg(cx.theme().background)
                    .rounded_lg()
                    .border_1()
                    .border_color(if has_error {
                        cx.theme().danger
                    } else {
                        cx.theme().border
                    })
                    .shadow_lg()
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .px_2p5()
                            .py_1p5()
                            .child(
                                div().w(px(164.)).child(
                                    Input::new(&search_input).appearance(false).cleanable(true),
                                ),
                            )
                            .child(h_flex().items_center().gap_0p5().children(
                                toggle_items.into_iter().map(
                                    |(id, icon, label, tooltip, active, option)| {
                                        let on_toggle = on_toggle_btn.clone();
                                        Button::new(id)
                                            .accessibility_label(label)
                                            .ghost()
                                            .compact()
                                            .xsmall()
                                            .selected(active)
                                            .toggled(active)
                                            .icon(Icon::new(icon))
                                            .tooltip(tooltip)
                                            .on_click(move |_, window, cx| {
                                                if let Some(ref on_toggle) = on_toggle {
                                                    on_toggle(option, window, cx);
                                                }
                                            })
                                    },
                                ),
                            ))
                            .when(!match_display.is_empty(), |el| {
                                el.child(
                                    div()
                                        .px_1()
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
                            .child(div().h_4().w_px().bg(cx.theme().border))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_0p5()
                                    .child(
                                        Button::new("preview-search-prev")
                                            .accessibility_label(
                                                "Previous search match (Shift+Enter)",
                                            )
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
                            ),
                    )
                    .when_some(error, |el, err| {
                        el.child(
                            h_flex()
                                .items_center()
                                .gap_1p5()
                                .px_2p5()
                                .py_1()
                                .border_t_1()
                                .border_color(cx.theme().danger.opacity(0.25))
                                .bg(cx.theme().danger.opacity(0.06))
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(Icon::new(IconName::TriangleAlert).xsmall())
                                .child(div().min_w_0().flex_1().child(err)),
                        )
                    }),
            )
    }
}
