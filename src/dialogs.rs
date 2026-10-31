//! Application alert and confirmation modal dialogs.

use std::{cell::Cell, path::PathBuf, rc::Rc};

use gpui_kit::{
    App, ParentElement as _, Styled as _, Window,
    component::{
        ActiveTheme as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        dialog::DialogFooter,
        v_flex,
    },
    div,
    prelude::FluentBuilder as _,
};

use crate::themes::AppSettings;

/// Show the version, a one-line description and the license.
pub(crate) fn open_about(window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, |alert, _, cx| {
        let muted = cx.theme().muted_foreground;
        alert
            .title("Downmark")
            .description(
                v_flex()
                    .gap_2()
                    .child(concat!(
                        "Version ",
                        env!("CARGO_PKG_VERSION"),
                        ". A fast, native Markdown viewer built with GPUI Kit."
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(if cfg!(feature = "mimalloc") {
                                "Memory allocator: mimalloc v0.1"
                            } else {
                                "Memory allocator: System Default"
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Licensed under the Apache License, Version 2.0."),
                    ),
            )
            .ok_text("OK")
    });
}

/// Ask before opening a web link (or an email address, or another app's
/// link) outside Downmark, as a document's links can lead anywhere.
pub(crate) fn confirm_external_link(url: String, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let web = url.starts_with("http://") || url.starts_with("https://");
    let dont_ask = Rc::new(Cell::new(false));
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let (url, open_url) = (url.clone(), url.clone());
        let (checkbox, remember) = (Rc::clone(&dont_ask), Rc::clone(&dont_ask));
        alert
            .title(if web {
                "Open this link in your browser?"
            } else {
                "Open this link?"
            })
            .description(
                v_flex()
                    .gap_3()
                    .child(
                        div()
                            .px_2()
                            .py_1p5()
                            .rounded(cx.theme().radius)
                            .bg(cx.theme().muted)
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(url.chars().take(400).collect::<String>()),
                    )
                    .when(web, |this| {
                        this.child(
                            Checkbox::new("dont-ask")
                                .label("Don’t ask again for web links")
                                .checked(remember.get())
                                .on_click(move |checked, window, _| {
                                    checkbox.set(*checked);
                                    window.refresh();
                                }),
                        )
                    }),
            )
            .footer(
                DialogFooter::new()
                    .justify_end()
                    .child(
                        Button::new("cancel")
                            .label("Cancel")
                            .outline()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("open")
                            .label(if web { "Open in browser" } else { "Open" })
                            .primary()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                if remember.get() {
                                    AppSettings::update(cx, |settings| {
                                        settings.confirm_web_links = false;
                                    });
                                }
                                cx.open_url(&open_url);
                            }),
                    ),
            )
    });
}

/// Ask before opening a linked file Downmark doesn't show itself (or a
/// folder) with the app the system uses for it.
pub(crate) fn confirm_open_file(path: PathBuf, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let folder = path.is_dir();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let path = path.clone();
        alert
            .title(format!("Open “{name}”?"))
            .description(if folder {
                "The folder will open in your file manager."
            } else {
                "Downmark can’t show this file, so it will open in the app your system uses for it."
            })
            .footer(
                DialogFooter::new()
                    .justify_end()
                    .child(
                        Button::new("cancel")
                            .label("Cancel")
                            .outline()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("open").label("Open").primary().on_click(
                        move |_, window, cx| {
                            window.close_dialog(cx);
                            cx.open_with_system(&path);
                        },
                    )),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext as _, IntoElement};

    struct TestView;
    impl gpui_kit::Render for TestView {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui_kit::Context<'_, Self>,
        ) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn opens_about_dialog(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let (window_handle, _) = cx.update(|cx| {
            cx.set_global(crate::themes::AppSettings(
                crate::settings::Settings::default(),
            ));
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_window, cx| {
                cx.new(|_| TestView)
            })
            .unwrap()
        });
        cx.update_window(window_handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
            open_about(window, cx);
            assert!(window.has_active_dialog(cx));
            window.close_dialog(cx);
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn opens_confirm_external_link_dialog(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let (window_handle, _) = cx.update(|cx| {
            cx.set_global(crate::themes::AppSettings(
                crate::settings::Settings::default(),
            ));
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_window, cx| {
                cx.new(|_| TestView)
            })
            .unwrap()
        });
        cx.update_window(window_handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
            confirm_external_link("https://example.com".into(), window, cx);
            assert!(window.has_active_dialog(cx));
            window.close_dialog(cx);
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn opens_confirm_open_file_dialog(cx: &mut gpui_kit::TestAppContext) {
        cx.update(gpui_kit::init);
        let (window_handle, _) = cx.update(|cx| {
            cx.set_global(crate::themes::AppSettings(
                crate::settings::Settings::default(),
            ));
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_window, cx| {
                cx.new(|_| TestView)
            })
            .unwrap()
        });
        cx.update_window(window_handle, |_, window, cx| {
            assert!(!window.has_active_dialog(cx));
            confirm_open_file(PathBuf::from("test.pdf"), window, cx);
            assert!(window.has_active_dialog(cx));
            window.close_dialog(cx);
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
    }
}
