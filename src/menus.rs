//! The application menus.
//!
//! macOS shows them in the system menu bar; Linux and Windows show the same
//! menus in the window's title bar through `AppMenuBar`. Check marks follow
//! the settings and theme, so the menus are rebuilt whenever either changes.

use gpui_kit::{
    App, Entity, Menu, MenuItem, SharedString,
    component::{
        GlobalState, Theme, ThemeMode,
        input::{Copy, Cut, Paste, Redo, Replace, Search, SelectAll, Undo},
        menu::AppMenuBar,
    },
};

use crate::{
    actions::{
        About, Bold, BulletList, CloseDocument, CloseWindow, CodeBlock, FindInPreview, GoToHeading,
        Heading, InlineCode, InsertLink, Italic, NewFile, NewTab, NumberedList, Open,
        OpenMarkdownGuide, OpenRecent, OpenSettings, Quit, Quote, RevealInFolder, Save, SaveAs,
        SetAppearance, SetLayout, SetTheme, Strikethrough, TaskList, TidyTable, ToggleGlanceMode,
        ToggleLineNumbers, ToggleScrollSync, ToggleSoftWrap, ToggleTabs, ZoomIn, ZoomOut,
        ZoomReset,
    },
    document::display_name,
    settings::{Appearance, Layout},
    themes::{AppSettings, theme_names},
};

pub(crate) fn init(cx: &mut App) -> Entity<AppMenuBar> {
    let app_menu_bar = AppMenuBar::new(cx);
    update(&app_menu_bar, cx);

    cx.observe_global::<AppSettings>({
        let app_menu_bar = app_menu_bar.clone();
        move |cx| update(&app_menu_bar, cx)
    })
    .detach();
    cx.observe_global::<Theme>({
        let app_menu_bar = app_menu_bar.clone();
        move |cx| update(&app_menu_bar, cx)
    })
    .detach();

    app_menu_bar
}

fn update(app_menu_bar: &Entity<AppMenuBar>, cx: &mut App) {
    // `gpui_kit::Menu` does not implement `Clone` and is consumed by value,
    // so build is evaluated once for the system menu bar and once for the in-window bar.
    cx.set_menus(build(cx));
    let menus = build(cx).into_iter().map(Menu::owned).collect();
    GlobalState::global_mut(cx).set_app_menus(menus);
    app_menu_bar.update(cx, AppMenuBar::reload);
}

/// The most entries (separators included) a menu shows in full; GPUI Kit
/// turns longer ones into short scrolling lists.
#[cfg(test)]
const MAX_MENU_ITEMS: usize = 20;

fn submenu(name: &str, items: Vec<MenuItem>) -> MenuItem {
    MenuItem::Submenu(Menu {
        name: SharedString::from(name.to_string()),
        items,
        disabled: false,
    })
}

fn menu(name: &str, items: Vec<MenuItem>) -> Menu {
    Menu {
        name: SharedString::from(name.to_string()),
        items,
        disabled: false,
    }
}

#[expect(clippy::too_many_lines, reason = "Declarative application menu tree")]
pub(crate) fn build(cx: &App) -> Vec<Menu> {
    let settings = AppSettings::get(cx);
    let macos = cfg!(target_os = "macos");

    let recent: Vec<MenuItem> = if settings.recent_files.is_empty() {
        vec![MenuItem::action("No recent files", OpenRecent(0)).disabled(true)]
    } else {
        settings
            .recent_files
            .iter()
            .enumerate()
            .map(|(ix, path)| {
                let name = display_name(Some(path));
                let folder = path
                    .parent()
                    .and_then(|dir| dir.file_name())
                    .map(|dir| format!("  —  {}", dir.to_string_lossy()))
                    .unwrap_or_default();
                MenuItem::action(format!("{name}{folder}"), OpenRecent(ix))
            })
            .collect()
    };

    let mut file = vec![
        MenuItem::action("New", NewFile),
        MenuItem::action("New Tab", NewTab),
        MenuItem::action("Open…", Open),
        submenu("Open recent", recent),
        MenuItem::separator(),
        MenuItem::action("Save", Save),
        MenuItem::action("Save as…", SaveAs),
        MenuItem::separator(),
        MenuItem::action(
            if macos {
                "Reveal in Finder"
            } else {
                "Show in folder"
            },
            RevealInFolder,
        ),
        MenuItem::separator(),
        MenuItem::action("Close", CloseDocument),
        MenuItem::action("Close window", CloseWindow),
    ];
    if !macos {
        file.push(MenuItem::action("Quit", Quit));
    }

    let mut edit = vec![
        MenuItem::action("Undo", Undo),
        MenuItem::action("Redo", Redo),
        MenuItem::separator(),
        MenuItem::action("Cut", Cut),
        MenuItem::action("Copy", Copy),
        MenuItem::action("Paste", Paste),
        MenuItem::action("Select all", SelectAll),
        MenuItem::separator(),
        MenuItem::action("Find…", Search),
        MenuItem::action("Find in preview…", FindInPreview),
        MenuItem::action("Replace…", Replace),
    ];
    if !macos {
        edit.push(MenuItem::separator());
        edit.push(MenuItem::action("Settings…", OpenSettings));
    }

    let format = vec![
        MenuItem::action("Bold", Bold),
        MenuItem::action("Italic", Italic),
        MenuItem::action("Strikethrough", Strikethrough),
        MenuItem::action("Inline code", InlineCode),
        MenuItem::action("Link", InsertLink),
        MenuItem::separator(),
        submenu(
            "Heading",
            (1..=6)
                .map(|level| MenuItem::action(format!("Heading {level}"), Heading(level)))
                .collect(),
        ),
        MenuItem::action("Quote", Quote),
        MenuItem::action("Bulleted list", BulletList),
        MenuItem::action("Numbered list", NumberedList),
        MenuItem::action("Task list", TaskList),
        MenuItem::action("Code block", CodeBlock),
        MenuItem::separator(),
        MenuItem::action("Tidy table", TidyTable),
    ];

    let appearance = settings.appearance;
    let theme = Theme::global(cx);
    let theme_items = |mode: ThemeMode| {
        let selected = if mode.is_dark() {
            &theme.dark_theme.name
        } else {
            &theme.light_theme.name
        };
        theme_names(mode, cx)
            .into_iter()
            .map(|name| {
                let checked = &name == selected;
                MenuItem::action(name.clone(), SetTheme(name)).checked(checked)
            })
            .collect::<Vec<_>>()
    };

    let view = vec![
        MenuItem::action("Editor", SetLayout(Layout::Editor))
            .checked(settings.layout == Layout::Editor),
        MenuItem::action("Editor and preview", SetLayout(Layout::Split))
            .checked(settings.layout == Layout::Split),
        MenuItem::action("Preview", SetLayout(Layout::Preview))
            .checked(settings.layout == Layout::Preview),
        MenuItem::separator(),
        MenuItem::action("Glance mode", ToggleGlanceMode).checked(settings.glance_mode),
        MenuItem::separator(),
        MenuItem::action("Tabs", ToggleTabs).checked(settings.tabs),
        MenuItem::action("Go to heading…", GoToHeading),
        MenuItem::separator(),
        MenuItem::action("Sync scrolling", ToggleScrollSync).checked(settings.scroll_sync),
        MenuItem::action("Wrap lines", ToggleSoftWrap).checked(settings.soft_wrap),
        MenuItem::action("Line numbers", ToggleLineNumbers).checked(settings.line_numbers),
        MenuItem::separator(),
        MenuItem::action("Zoom in", ZoomIn),
        MenuItem::action("Zoom out", ZoomOut),
        MenuItem::action("Actual size", ZoomReset),
        MenuItem::separator(),
        // Themes live in the Appearance submenu: menus with more than
        // MAX_MENU_ITEMS entries turn into small scrolling lists.
        submenu(
            "Appearance",
            vec![
                MenuItem::action("System", SetAppearance(Appearance::System))
                    .checked(appearance == Appearance::System),
                MenuItem::action("Light", SetAppearance(Appearance::Light))
                    .checked(appearance == Appearance::Light),
                MenuItem::action("Dark", SetAppearance(Appearance::Dark))
                    .checked(appearance == Appearance::Dark),
                MenuItem::separator(),
                submenu("Light theme", theme_items(ThemeMode::Light)),
                submenu("Dark theme", theme_items(ThemeMode::Dark)),
            ],
        ),
    ];

    let mut help = vec![MenuItem::action("Markdown guide", OpenMarkdownGuide)];
    if !macos {
        help.push(MenuItem::separator());
        help.push(MenuItem::action("About Downmark", About));
    }

    let mut menus = Vec::new();
    if macos {
        menus.push(menu(
            "Downmark",
            vec![
                MenuItem::action("About Downmark", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit Downmark", Quit),
            ],
        ));
    }
    menus.extend([
        menu("File", file),
        menu("Edit", edit),
        menu("Format", format),
        menu("View", view),
        menu("Help", help),
    ]);
    menus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[gpui_kit::test]
    fn menus_reflect_settings(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(AppSettings(Settings {
                layout: Layout::Preview,
                soft_wrap: false,
                ..Settings::default()
            }));
        });
        cx.read(|cx| {
            let menus = build(cx);
            let view = menus.iter().find(|menu| menu.name == "View").unwrap();
            let checked: Vec<_> = view
                .items
                .iter()
                .filter(|item| item.is_checked())
                .map(|item| match item {
                    MenuItem::Action { name, .. } => name.to_string(),
                    _ => String::new(),
                })
                .collect();
            assert_eq!(checked, ["Preview", "Tabs", "Line numbers"]);
        });

        cx.update(|cx| {
            AppSettings::update(cx, |settings| settings.glance_mode = true);
        });
        cx.read(|cx| {
            let menus = build(cx);
            let view = menus.iter().find(|menu| menu.name == "View").unwrap();
            let checked: Vec<_> = view
                .items
                .iter()
                .filter(|item| item.is_checked())
                .map(|item| match item {
                    MenuItem::Action { name, .. } => name.to_string(),
                    _ => String::new(),
                })
                .collect();
            assert_eq!(checked, ["Preview", "Glance mode", "Tabs", "Line numbers"]);
        });
    }

    #[gpui_kit::test]
    fn no_menu_is_long_enough_to_scroll(cx: &mut gpui_kit::TestAppContext) {
        fn check(name: &str, items: &[MenuItem]) {
            assert!(
                items.len() <= MAX_MENU_ITEMS,
                "{name} has {} items; menus with more than {MAX_MENU_ITEMS} scroll",
                items.len()
            );
            for item in items {
                if let MenuItem::Submenu(submenu) = item {
                    check(&submenu.name, &submenu.items);
                }
            }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::themes::init(Settings::default(), cx);
            cx.set_global(AppSettings(Settings::default()));
        });
        cx.read(|cx| {
            for menu in build(cx) {
                check(&menu.name, &menu.items);
            }
        });
    }
}
