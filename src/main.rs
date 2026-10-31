//! Downmark — a fast, native Markdown editor and previewer.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "mimalloc")]
unsafe extern "C" {
    fn mi_collect(force: bool);
}

#[cfg(all(not(feature = "mimalloc"), target_os = "linux"))]
unsafe extern "C" {
    fn malloc_trim(pad: usize) -> i32;
}

/// Trims allocator free lists and decommits unused memory back to the operating system.
pub(crate) fn trim_allocator_memory() {
    #[cfg(feature = "mimalloc")]
    // SAFETY: `mi_collect` is provided by the mimalloc C library to reclaim cached allocations.
    // Calling it with `force = true` is thread-safe and does not violate memory safety invariants.
    unsafe {
        mi_collect(true);
    }
    #[cfg(all(not(feature = "mimalloc"), target_os = "linux"))]
    // SAFETY: `malloc_trim` is a standard GNU libc extension to release free heap memory to the OS.
    // Passing 0 padding is well-defined and operates on glibc internal memory arenas safely.
    unsafe {
        malloc_trim(0);
    }
}

mod actions;
mod analysis;
mod catalog;
mod default_app;
mod diagram;
mod diagram_viewer;
mod dialogs;
mod document;
mod document_view;
mod fonts;
mod format;
mod images;
mod links;
mod math;
mod menus;
mod new_tab;
mod preview_ext;
mod scroll_sync;
mod search;
mod search_overlay;
mod session;
mod settings;
mod settings_dialog;
mod smart_edit;
mod table_overlay;
mod themes;
mod typst_world;
mod workspace;

use std::{path::PathBuf, sync::Arc};

use futures::StreamExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Global, WeakEntity, WindowBounds, WindowKind,
    WindowOptions, component::TitleBar, px, size,
};

use crate::{
    actions::{
        About, OpenMarkdownGuide, OpenSettings, Quit, SetAppearance, SetTheme, ToggleGlanceMode,
    },
    settings::Settings,
    workspace::{GlanceStart, Workspace},
};

const MARKDOWN_GUIDE_URL: &str = "https://commonmark.org/help/";
const DEFAULT_WINDOW_WIDTH: f32 = 860.;
const DEFAULT_WINDOW_HEIGHT: f32 = 720.;

/// The window's workspace, for files the system asks Downmark to open.
struct MainWindow {
    handle: AnyWindowHandle,
    workspace: WeakEntity<Workspace>,
}

impl Global for MainWindow {}

#[expect(
    clippy::too_many_lines,
    reason = "Application startup, arguments parsing, global actions and lifecycle setup"
)]
fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let glance = if args.iter().any(|arg| {
        let s = arg.to_string_lossy();
        s == "--glance" || s == "-g"
    }) {
        GlanceStart::Glance
    } else {
        GlanceStart::Normal
    };
    let paths: Vec<PathBuf> = args
        .into_iter()
        .filter(|arg| !arg.to_string_lossy().starts_with('-'))
        .map(PathBuf::from)
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect();

    // macOS delivers "Open with" and double-clicked files as URLs, outside
    // the app's context; they are handed over through a channel.
    let (open_tx, mut open_rx) = futures::channel::mpsc::unbounded::<Vec<PathBuf>>();
    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    application.on_open_urls(move |urls| {
        let paths: Vec<PathBuf> = urls
            .iter()
            .filter_map(|url| url::Url::parse(url).ok()?.to_file_path().ok())
            .collect();
        if !paths.is_empty() {
            let _ = open_tx.unbounded_send(paths);
        }
    });

    application.run(move |cx| {
        gpui_kit::init(cx);
        fonts::FontRegistry::init(cx);
        match reqwest_client::ReqwestClient::user_agent(concat!(
            "Downmark/",
            env!("CARGO_PKG_VERSION")
        )) {
            Ok(client) => cx.set_http_client(Arc::new(client)),
            Err(err) => {
                #[expect(
                    clippy::print_stderr,
                    reason = "CLI error reporting before window init"
                )]
                {
                    eprintln!("downmark: remote images are unavailable: {err}");
                }
            }
        }

        themes::init(Settings::load(), cx);
        cx.set_global(preview_ext::SharedDiagramCache::default());
        // Bind keys before building menus so the menus show the shortcuts.
        actions::bind_keys(cx);
        diagram_viewer::bind_keys(cx);
        let app_menu_bar = menus::init(cx);

        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        cx.on_action(|_: &OpenMarkdownGuide, cx: &mut App| cx.open_url(MARKDOWN_GUIDE_URL));
        cx.on_action(|action: &SetAppearance, cx: &mut App| {
            let appearance = action.0;
            themes::AppSettings::update(cx, |settings| settings.appearance = appearance);
        });
        cx.on_action(|action: &SetTheme, cx: &mut App| themes::select_theme(&action.0, cx));
        cx.on_action(|_: &About, cx: &mut App| with_window(cx, dialogs::open_about));
        cx.on_action(|_: &OpenSettings, cx: &mut App| {
            with_window(cx, settings_dialog::open_settings);
        });
        cx.on_action(|_: &ToggleGlanceMode, cx: &mut App| {
            if let Some(main) = cx.try_global::<MainWindow>() {
                let workspace = main.workspace.clone();
                _ = main.handle.update(cx, |_, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.toggle_glance_mode(window, cx);
                        });
                    }
                });
            }
        });
        cx.on_action(|_: &gpui_kit::component::input::Escape, cx: &mut App| {
            if themes::AppSettings::get(cx).glance_mode
                && let Some(main) = cx.try_global::<MainWindow>()
            {
                let workspace = main.workspace.clone();
                _ = main.handle.update(cx, |_, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.handle_escape(window, cx);
                        });
                    }
                });
            }
        });

        // Downmark is a single-window app: closing the window ends it on
        // every platform, as the menu offers no way to open a new one.
        cx.on_window_closed(|cx, _| {
            let main_open = cx.try_global::<MainWindow>().is_some_and(|main| {
                cx.windows()
                    .iter()
                    .any(|window| window.window_id() == main.handle.window_id())
            });
            if !main_open {
                cx.quit();
            }
        })
        .detach();

        let menu_bar = app_menu_bar.clone();
        cx.spawn(async move |cx| {
            while let Some(paths) = open_rx.next().await {
                let menu_bar = menu_bar.clone();
                cx.update(|cx| open_paths(paths, GlanceStart::Normal, menu_bar, cx));
            }
        })
        .detach();

        open_window(paths, glance, app_menu_bar, cx);
        cx.activate(true);
    });
}

/// Open files in the window, or in a new one if it was closed.
fn open_paths(
    paths: Vec<PathBuf>,
    glance: GlanceStart,
    app_menu_bar: gpui_kit::Entity<gpui_kit::component::menu::AppMenuBar>,
    cx: &mut App,
) {
    if let Some(main) = cx.try_global::<MainWindow>() {
        let (handle, workspace) = (main.handle, main.workspace.clone());
        let opened = handle.update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.open_external(paths.clone(), glance, window, cx);
            })
        });
        if matches!(opened, Ok(Ok(()))) {
            _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }
    }
    open_window(paths, glance, app_menu_bar, cx);
}

fn open_window(
    paths: Vec<PathBuf>,
    glance: GlanceStart,
    app_menu_bar: gpui_kit::Entity<gpui_kit::component::menu::AppMenuBar>,
    cx: &mut App,
) {
    let window_bounds = workspace::restored_bounds(cx).unwrap_or_else(|| {
        let mut window_size = size(px(DEFAULT_WINDOW_WIDTH), px(DEFAULT_WINDOW_HEIGHT));
        if let Some(display) = cx.primary_display() {
            let display_size = display.bounds().size;
            window_size.width = window_size.width.min(display_size.width * 0.9);
            window_size.height = window_size.height.min(display_size.height * 0.9);
        }
        WindowBounds::Windowed(Bounds::centered(None, window_size, cx))
    });
    let options = WindowOptions {
        window_bounds: Some(window_bounds),
        window_min_size: Some(size(px(560.), px(360.))),
        kind: WindowKind::Normal,
        app_id: Some("io.github.Vidhan31.Downmark".into()),
        #[cfg(target_os = "linux")]
        window_background: gpui_kit::WindowBackgroundAppearance::Opaque,
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui_kit::WindowDecorations::Client),
        ..TitleBar::window_options()
    };

    let result = gpui_kit::open_window(options, cx, move |window, cx| {
        themes::apply(Some(window), cx);
        let workspace = cx.new(|cx| Workspace::new(paths, glance, app_menu_bar, window, cx));
        cx.set_global(MainWindow {
            handle: window.window_handle(),
            workspace: workspace.downgrade(),
        });
        workspace
    });
    if let Err(err) = result {
        #[expect(clippy::print_stderr, reason = "Fatal window initialization error")]
        {
            eprintln!("downmark: could not open a window: {err}");
        }
        cx.quit();
    }
}

/// Run `f` in the main window, where dialogs like Settings belong.
fn with_window(cx: &mut App, f: impl FnOnce(&mut gpui_kit::Window, &mut App)) {
    let Some(window) = cx
        .try_global::<MainWindow>()
        .map(|main| main.handle)
        .filter(|main| cx.windows().contains(main))
        .or_else(|| cx.active_window())
        .or_else(|| cx.windows().into_iter().next())
    else {
        return;
    };
    _ = window.update(cx, |_, window, cx| f(window, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_allocator_memory() {
        trim_allocator_memory();
    }
}
