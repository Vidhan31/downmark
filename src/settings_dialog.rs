//! The application settings dialog.

use std::cell::Cell;

use gpui_kit::assets::IconName;
use gpui_kit::{
    App, AppContext as _, FontWeight, IntoElement, ParentElement as _, Styled as _, Window,
    component::{
        ActiveTheme as _, Icon, IndexPath, Sizable as _, WindowExt as _,
        button::Button,
        h_flex,
        notification::Notification,
        radio::RadioGroup,
        select::{SearchableVec, Select, SelectEvent, SelectState},
        switch::Switch,
        v_flex,
    },
    div, px,
};

use crate::{
    default_app,
    fonts::{DEFAULT_FONT_LABEL, FontRegistry},
    settings::LinkPlacement,
    themes::AppSettings,
};

/// The settings dialog: Downmark's optional features and how it behaves.
#[expect(
    clippy::too_many_lines,
    reason = "Full settings dialog layout, tabs, and event handlers"
)]
pub(crate) fn open_settings(window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    // Ask the system afresh each time, as the user may have changed the
    // default app elsewhere.
    default_app::refresh(cx);

    let all_fonts = FontRegistry::all_fonts();
    let code_fonts = FontRegistry::code_fonts();

    let settings = AppSettings::get(cx).clone();
    let current_normal = settings
        .font_family
        .as_deref()
        .unwrap_or(DEFAULT_FONT_LABEL);
    let current_code = settings
        .mono_font_family
        .as_deref()
        .unwrap_or(DEFAULT_FONT_LABEL);

    let normal_idx = all_fonts
        .iter()
        .position(|f| f == current_normal)
        .map(IndexPath::new);
    let code_idx = code_fonts
        .iter()
        .position(|f| f == current_code)
        .map(IndexPath::new);

    let normal_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(all_fonts), normal_idx, window, cx).searchable(true)
    });
    let code_select = cx.new(|cx| {
        SelectState::new(SearchableVec::new(code_fonts), code_idx, window, cx).searchable(true)
    });

    let normal_sub = cx.subscribe(
        &normal_select,
        move |_select, event: &SelectEvent<_>, cx: &mut App| match event {
            SelectEvent::Confirm(val) => {
                let font = val
                    .clone()
                    .filter(|f| f != DEFAULT_FONT_LABEL && !f.trim().is_empty());
                AppSettings::update(cx, |settings| settings.font_family = font);
            }
        },
    );
    normal_sub.detach();

    let code_sub = cx.subscribe(
        &code_select,
        move |_select, event: &SelectEvent<_>, cx: &mut App| match event {
            SelectEvent::Confirm(val) => {
                let font = val
                    .clone()
                    .filter(|f| f != DEFAULT_FONT_LABEL && !f.trim().is_empty());
                AppSettings::update(cx, |settings| settings.mono_font_family = font);
            }
        },
    );
    code_sub.detach();

    let fonts_synced = Cell::new(FontRegistry::is_loaded());

    window.open_dialog(cx, move |dialog, window, cx| {
        if FontRegistry::is_loaded() && !fonts_synced.get() {
            fonts_synced.set(true);
            let all = FontRegistry::all_fonts();
            let code = FontRegistry::code_fonts();
            normal_select.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(all), window, cx);
            });
            code_select.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(code), window, cx);
            });
        }
        let settings = AppSettings::get(cx).clone();
        let default_status = default_app::status(cx);
        let row = |id: &'static str,
                   title: &'static str,
                   description: &'static str,
                   checked: bool,
                   update: fn(&mut crate::settings::Settings, bool),
                   cx: &App| {
            h_flex()
                .gap_4()
                .py_2()
                .justify_between()
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(description),
                        ),
                )
                .child(
                    Switch::new(id)
                        .checked(checked)
                        .on_click(move |checked, _, cx| {
                            let checked = *checked;
                            AppSettings::update(cx, |settings| update(settings, checked));
                        }),
                )
        };
        let section = |title: &'static str| {
            div()
                .pt_3()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(title)
        };
        dialog.title("Settings").w(px(560.)).child(
            v_flex()
                .child(section("WORKSPACE"))
                .child(row(
                    "tabs",
                    "Tabs",
                    "Open documents side by side in tabs instead of one at a time.",
                    settings.tabs,
                    |settings, on| settings.tabs = on,
                    cx,
                ))
                .child(row(
                    "restore-session",
                    "Reopen last session",
                    "Start with the documents, folder and window you had open.",
                    settings.restore_session,
                    |settings, on| settings.restore_session = on,
                    cx,
                ))
                .child(section("WRITING"))
                .child(row(
                    "scroll-sync",
                    "Sync scrolling",
                    "Keep the preview on the part of the document being edited.",
                    settings.scroll_sync,
                    |settings, on| settings.scroll_sync = on,
                    cx,
                ))
                .child(row(
                    "soft-wrap",
                    "Wrap lines",
                    "Wrap long lines in the editor instead of scrolling sideways.",
                    settings.soft_wrap,
                    |settings, on| settings.soft_wrap = on,
                    cx,
                ))
                .child(row(
                    "line-numbers",
                    "Line numbers",
                    "Number the lines in the editor.",
                    settings.line_numbers,
                    |settings, on| settings.line_numbers = on,
                    cx,
                ))
                .child(section("TYPOGRAPHY"))
                .child(
                    h_flex()
                        .gap_4()
                        .py_2()
                        .justify_between()
                        .items_center()
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Document font"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Font for document reading and interface text."),
                                ),
                        )
                        .child(
                            div()
                                .w(px(220.))
                                .child(Select::new(&normal_select).placeholder(DEFAULT_FONT_LABEL)),
                        ),
                )
                .child(
                    h_flex()
                        .gap_4()
                        .py_2()
                        .justify_between()
                        .items_center()
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Code font"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            "Monospace font for editor, code blocks and tables.",
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .w(px(220.))
                                .child(Select::new(&code_select).placeholder(DEFAULT_FONT_LABEL)),
                        ),
                )
                .child(section("LINKS"))
                .child(
                    h_flex()
                        .gap_4()
                        .py_2()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child("Open linked documents in"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(if cfg!(target_os = "macos") {
                                            "Command-click a link to do the opposite."
                                        } else {
                                            "Ctrl-click a link to do the opposite."
                                        }),
                                ),
                        )
                        .child(
                            RadioGroup::horizontal("open-links-in")
                                .flex_none()
                                .child("A new tab")
                                .child("The same tab")
                                .selected_index(Some(match settings.open_links_in {
                                    LinkPlacement::NewTab => 0,
                                    LinkPlacement::SameTab => 1,
                                }))
                                .on_click(|ix, _, cx| {
                                    let placement = if *ix == 0 {
                                        LinkPlacement::NewTab
                                    } else {
                                        LinkPlacement::SameTab
                                    };
                                    AppSettings::update(cx, |settings| {
                                        settings.open_links_in = placement;
                                    });
                                }),
                        ),
                )
                .child(row(
                    "confirm-web-links",
                    "Ask before opening web links",
                    "Show where a link leads before it opens in your browser.",
                    settings.confirm_web_links,
                    |settings, on| settings.confirm_web_links = on,
                    cx,
                ))
                .child(section("FILES"))
                .child(default_app_row(default_status, cx)),
        )
    });
}

/// The settings row that makes Downmark the app for Markdown files.
fn default_app_row(status: default_app::Status, cx: &App) -> impl IntoElement {
    let is_default = status == default_app::Status::Default;
    let description = if is_default {
        "Markdown files you open from your desktop open in Downmark."
    } else if default_app::CHOSEN_IN_SYSTEM_SETTINGS {
        "Choose Downmark for Markdown files in Windows Settings."
    } else {
        "Open Markdown files from your desktop in Downmark."
    };
    let action = if is_default {
        h_flex()
            .gap_1()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::Check).small())
            .child("Default")
            .into_any_element()
    } else {
        let label = if default_app::CHOSEN_IN_SYSTEM_SETTINGS {
            "Open Settings…"
        } else {
            "Make Default"
        };
        Button::new("make-default")
            .label(label)
            .small()
            .outline()
            .on_click(|_, window, cx| {
                let note = match default_app::make_default(cx) {
                    Ok(default_app::Outcome::Done) => {
                        Notification::success("Downmark now opens Markdown files.")
                    }
                    Ok(default_app::Outcome::SettingsOpened) => Notification::info(
                        "In Settings, set Downmark as the default for .md files.",
                    ),
                    Err(message) => Notification::error(message),
                };
                window.push_notification(note, cx);
                window.refresh();
            })
            .into_any_element()
    };
    h_flex()
        .gap_4()
        .py_2()
        .justify_between()
        .child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Default app for Markdown"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                ),
        )
        .child(action)
}
