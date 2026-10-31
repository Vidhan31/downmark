use std::path::{Path, PathBuf};

use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    assets::IconName,
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        scroll::ScrollableElement as _,
        v_flex,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

use crate::{
    catalog::{FolderCatalog, WorkspaceFileEntry},
    themes::AppSettings,
};

/// Events emitted by the Home page to be handled by the document view or workspace.
#[derive(Clone, Debug)]
pub(crate) enum NewTabEvent {
    OpenFile(PathBuf),
    OpenFileInNewTab(PathBuf),
    PromptChooseFolder,
    PromptOpenFile,
    RemoveFolder(PathBuf),
    Rescan,
}

pub(crate) struct NewTabView {
    catalog: Entity<FolderCatalog>,
    filter_input: Entity<InputState>,
    filter_query: String,
    selected_folder: Option<PathBuf>,
    selected_card_index: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<NewTabEvent> for NewTabView {}

impl Focusable for NewTabView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter_input.read(cx).focus_handle(cx)
    }
}

impl NewTabView {
    pub(crate) fn new(
        catalog: Entity<FolderCatalog>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter all markdown files…"));

        let subscriptions = vec![
            cx.observe(&catalog, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &filter_input,
                window,
                |this, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.filter_query = input.read(cx).text().to_string();
                        cx.notify();
                    }
                },
            ),
        ];

        Self {
            catalog,
            filter_input,
            filter_query: String::new(),
            selected_folder: None,
            selected_card_index: None,
            _subscriptions: subscriptions,
        }
    }

    #[expect(
        clippy::too_many_lines,
        clippy::fn_params_excessive_bools,
        reason = "rich file card with metadata, actions, keyboard selection, and theme styling"
    )]
    fn render_file_card(
        entry: &WorkspaceFileEntry,
        idx: usize,
        is_selected: bool,
        prefix: &str,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let full_path = entry.file.full_path.clone();
        let size_str = entry.formatted_size();
        let mod_str = entry.formatted_modified();
        let folder_name = entry.folder_name.clone();
        let file_name = entry.file.file_name.clone();
        let is_nested = entry.file.relative_path != file_name;
        let display_folder = if is_nested {
            if let Some(parent) = Path::new(&entry.file.relative_path).parent() {
                if parent.as_os_str().is_empty() {
                    folder_name
                } else {
                    format!("{folder_name} / {}", parent.display())
                }
            } else {
                folder_name
            }
        } else {
            folder_name
        };

        let theme = cx.theme();
        let path_left = full_path.clone();
        let path_mid = full_path;
        let border_color = if is_selected {
            theme.ring
        } else {
            theme.border
        };
        let bg_color = if is_selected {
            theme.list_active
        } else {
            theme.background
        };

        h_flex()
            .id(SharedString::from(format!("{prefix}-{idx}-card")))
            .role(gpui_kit::Role::ListItem)
            .aria_selected(is_selected)
            .aria_label(format!(
                "File: {file_name}, in {display_folder}, size {size_str}, modified {mod_str}"
            ))
            .flex_1()
            .min_w(px(280.))
            .h_11()
            .px_3()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(border_color)
            .bg(bg_color)
            .hover(|s| s.bg(theme.list_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui_kit::MouseDownEvent, _, cx| {
                    this.selected_card_index = Some(idx);
                    if event.modifiers.control || event.modifiers.platform {
                        cx.emit(NewTabEvent::OpenFileInNewTab(path_left.clone()));
                    } else {
                        cx.emit(NewTabEvent::OpenFile(path_left.clone()));
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |_this, _, _, cx| {
                    cx.emit(NewTabEvent::OpenFileInNewTab(path_mid.clone()));
                }),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(theme.primary)
                    .child(Icon::new(IconName::FileText).small()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.foreground)
                            .truncate()
                            .child(file_name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(display_folder),
                    ),
            )
            .child(
                v_flex()
                    .flex_none()
                    .items_end()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.muted_foreground)
                            .child(mod_str),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground.opacity(0.8))
                            .child(size_str),
                    ),
            )
    }

    fn render_file_grid(
        entries: &[WorkspaceFileEntry],
        prefix: &str,
        selected_index: Option<usize>,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        v_flex()
            .gap_2()
            .children(entries.chunks(2).enumerate().map(|(row_idx, chunk)| {
                let first = &chunk[0];
                let second = chunk.get(1);
                let idx_first = row_idx * 2;
                let idx_second = row_idx * 2 + 1;

                h_flex()
                    .id(SharedString::from(format!("{prefix}-row-{row_idx}")))
                    .gap_2()
                    .child(Self::render_file_card(
                        first,
                        idx_first,
                        selected_index == Some(idx_first),
                        prefix,
                        cx,
                    ))
                    .when_some(second, |row, second_entry| {
                        row.child(Self::render_file_card(
                            second_entry,
                            idx_second,
                            selected_index == Some(idx_second),
                            prefix,
                            cx,
                        ))
                    })
                    .when(second.is_none(), |row| {
                        row.child(div().flex_1().min_w(px(280.)))
                    })
            }))
    }
}

impl Render for NewTabView {
    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI layout for Home page"
    )]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        let catalog = self.catalog.read(cx);
        let tracked_folders = catalog.folders();
        let total_folders = tracked_folders.len();
        let total_files = catalog.total_files();

        let recent_paths = &AppSettings::get(cx).recent_files;
        let recent_entries: Vec<WorkspaceFileEntry> = catalog
            .get_recent_accessed_entries(recent_paths)
            .into_iter()
            .take(6)
            .collect();

        let last_modified: Vec<WorkspaceFileEntry> = catalog
            .sorted_modified_files()
            .iter()
            .take(8)
            .cloned()
            .collect();

        let (filtered_files, matched_count) =
            catalog.filter_files(&self.filter_query, self.selected_folder.as_deref());

        let has_filter = !self.filter_query.trim().is_empty() || self.selected_folder.is_some();
        let filtered_count = filtered_files.len();
        let filtered_paths: Vec<PathBuf> = filtered_files
            .iter()
            .map(|e| e.file.full_path.clone())
            .collect();

        v_flex()
            .id("new-tab-page")
            .size_full()
            .overflow_y_scrollbar()
            .bg(theme.background)
            .px_8()
            .py_8()
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "down" => {
                        if filtered_count > 0 {
                            this.selected_card_index = Some(match this.selected_card_index {
                                None => 0,
                                Some(ix) => (ix + 2).min(filtered_count - 1),
                            });
                            cx.notify();
                        }
                    }
                    "up" => {
                        if let Some(ix) = this.selected_card_index {
                            if ix < 2 {
                                this.selected_card_index = None;
                                this.filter_input.update(cx, |input, cx| input.focus(window, cx));
                            } else {
                                this.selected_card_index = Some(ix - 2);
                            }
                            cx.notify();
                        }
                    }
                    "right" => {
                        if filtered_count > 0 {
                            this.selected_card_index = Some(match this.selected_card_index {
                                None => 0,
                                Some(ix) => (ix + 1).min(filtered_count - 1),
                            });
                            cx.notify();
                        }
                    }
                    "left" => {
                        if let Some(ix) = this.selected_card_index {
                            if ix == 0 {
                                this.selected_card_index = None;
                                this.filter_input.update(cx, |input, cx| input.focus(window, cx));
                            } else {
                                this.selected_card_index = Some(ix - 1);
                            }
                            cx.notify();
                        }
                    }
                    "enter" => {
                        let target_path = if let Some(ix) = this.selected_card_index {
                            filtered_paths.get(ix).cloned()
                        } else if !filtered_paths.is_empty() {
                            filtered_paths.first().cloned()
                        } else {
                            None
                        };
                        if let Some(path) = target_path {
                            if event.keystroke.modifiers.secondary(cx) || event.keystroke.modifiers.shift {
                                cx.emit(NewTabEvent::OpenFileInNewTab(path));
                            } else {
                                cx.emit(NewTabEvent::OpenFile(path));
                            }
                        }
                    }
                    "escape" => {
                        if this.selected_card_index.is_some() {
                            this.selected_card_index = None;
                            this.filter_input.update(cx, |input, cx| input.focus(window, cx));
                            cx.notify();
                        } else if !this.filter_query.is_empty() {
                            this.filter_query.clear();
                            this.filter_input.update(cx, |input, cx| input.set_value("", window, cx));
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            }))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(980.))
                    .mx_auto()
                    .gap_7()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .pb_4()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .size(px(38.))
                                            .rounded_lg()
                                            .bg(theme.primary.opacity(0.12))
                                            .text_color(theme.primary)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(Icon::new(IconName::House).size(px(20.))),
                                    )
                                    .child(
                                        v_flex()
                                            .child(
                                                div()
                                                    .text_xl()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(theme.foreground)
                                                    .child("Home"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child("Choose a folder to scan for Markdown files or open a document"),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Button::new("new-tab-open-file-btn")
                                            .accessibility_label("Open Markdown file")
                                            .outline()
                                            .small()
                                            .icon(Icon::new(IconName::FileText))
                                            .child("Open File")
                                            .on_click(cx.listener(|_this, _, _, cx| {
                                                cx.emit(NewTabEvent::PromptOpenFile);
                                             })),
                                    )
                                    .child(
                                        Button::new("new-tab-choose-folder-btn")
                                            .accessibility_label("Choose folder to scan for Markdown files")
                                            .primary()
                                            .small()
                                            .icon(Icon::new(IconName::FolderOpen))
                                            .child("Choose Folder")
                                            .on_click(cx.listener(|_this, _, _, cx| {
                                                cx.emit(NewTabEvent::PromptChooseFolder);
                                            })),
                                    )
                                    .when(total_folders > 0, |header| {
                                        header.child(
                                            Button::new("new-tab-rescan-btn")
                                                .accessibility_label("Rescan tracked folders")
                                                .ghost()
                                                .small()
                                                .icon(Icon::new(IconName::RotateCw))
                                                .tooltip("Rescan tracked folders")
                                                .on_click(cx.listener(|_this, _, _, cx| {
                                                    cx.emit(NewTabEvent::Rescan);
                                                })),
                                        )
                                    }),
                            ),
                    )
                    .when(!recent_entries.is_empty(), |page| {
                        page.child(
                            v_flex()
                                .gap_3()
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Icon::new(IconName::Clock)
                                                .small()
                                                .text_color(theme.primary),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(theme.foreground)
                                                .child("Last Accessed Files in App"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .child(format!("({} recent)", recent_entries.len())),
                                        ),
                                )
                                .child(Self::render_file_grid(&recent_entries, "recent", None, cx)),
                        )
                    })
                    .when(total_folders == 0, |page| {
                        page.child(
                            v_flex()
                                .items_center()
                                .justify_center()
                                .py_12()
                                .px_6()
                                .rounded_xl()
                                .border_1()
                                .border_dashed()
                                .border_color(theme.border)
                                .bg(theme.list_hover.opacity(0.3))
                                .gap_3()
                                .child(
                                    div()
                                        .size(px(52.))
                                        .rounded_full()
                                        .bg(theme.primary.opacity(0.12))
                                        .text_color(theme.primary)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(Icon::new(IconName::FolderOpen).size(px(26.))),
                                )
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme.foreground)
                                        .child("No Folder Chosen Yet"),
                                )
                                .child(
                                    div()
                                        .max_w(px(460.))
                                        .text_center()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child("Choose a folder containing Markdown files. We will filter out noisy directories like node_modules and .git, and display all your documents here."),
                                )
                                .child(
                                    div()
                                        .pt_2()
                                        .child(
                                            Button::new("empty-state-choose-folder-btn")
                                                .primary()
                                                .icon(Icon::new(IconName::FolderOpen))
                                                .child("Choose Folder to Scan")
                                                .on_click(cx.listener(|_this, _, _, cx| {
                                                    cx.emit(NewTabEvent::PromptChooseFolder);
                                                })),
                                        ),
                                ),
                        )
                    })
                    .when(total_folders > 0, |page| {
                        page.child(
                            v_flex()
                                .gap_7()
                                .when(!last_modified.is_empty() && !has_filter, |s2| {
                                    s2.child(
                                        v_flex()
                                            .gap_3()
                                            .child(
                                                h_flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        Icon::new(IconName::Calendar)
                                                            .small()
                                                            .text_color(theme.primary),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(theme.foreground)
                                                            .child("Last Modified Files"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(theme.muted_foreground)
                                                            .child("— Recently edited in tracked folders"),
                                                    ),
                                            )
                                            .child(Self::render_file_grid(&last_modified, "modified", None, cx)),
                                    )
                                })
                                .child(
                                    v_flex()
                                        .gap_4()
                                        .child(
                                            h_flex()
                                                .justify_between()
                                                .items_center()
                                                .child(
                                                    h_flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .child(
                                                            Icon::new(IconName::FileText)
                                                                .small()
                                                                .text_color(theme.primary),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_sm()
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_color(theme.foreground)
                                                                .child("List of All Files"),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(theme.muted_foreground)
                                                                .child(if has_filter {
                                                                    format!("({matched_count} of {total_files} files)")
                                                                } else {
                                                                    format!("({total_files} markdown files in {total_folders} folder{})", if total_folders > 1 { "s" } else { "" })
                                                                }),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_3()
                                                .items_center()
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .child(Input::new(&self.filter_input).cleanable(true)),
                                                ),
                                        )
                                        .when(total_folders > 1, |list| {
                                            list.child(
                                                h_flex()
                                                    .gap_1p5()
                                                    .items_center()
                                                    .flex_wrap()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(theme.muted_foreground)
                                                            .mr_1()
                                                            .child("Folder:"),
                                                    )
                                                    .child({
                                                        let is_all = self.selected_folder.is_none();
                                                        div()
                                                            .px_2p5()
                                                            .py_0p5()
                                                            .rounded_full()
                                                            .border_1()
                                                            .text_xs()
                                                            .when(is_all, |s| {
                                                                s.bg(theme.primary)
                                                                    .border_color(theme.primary)
                                                                    .text_color(theme.primary_foreground)
                                                                    .font_weight(FontWeight::MEDIUM)
                                                            })
                                                            .when(!is_all, |s| {
                                                                s.border_color(theme.border)
                                                                    .text_color(theme.muted_foreground)
                                                                    .hover(|h| h.bg(theme.list_hover).text_color(theme.foreground))
                                                            })
                                                            .on_mouse_down(
                                                                MouseButton::Left,
                                                                cx.listener(|this, _, _, cx| {
                                                                    this.selected_folder = None;
                                                                    cx.notify();
                                                                }),
                                                            )
                                                            .child(format!("All ({total_files})"))
                                                    })
                                                    .children(catalog.groups().iter().map(|group| {
                                                        let path = group.folder_path.clone();
                                                        let is_selected = self.selected_folder.as_ref() == Some(&path);
                                                        let count = group.files.len();
                                                        let name = group.folder_name.clone();

                                                        let click_path = path.clone();
                                                        let remove_path = path;
                                                        h_flex()
                                                            .items_center()
                                                            .gap_1()
                                                            .px_2p5()
                                                            .py_0p5()
                                                            .rounded_full()
                                                            .border_1()
                                                            .text_xs()
                                                            .when(is_selected, |s| {
                                                                s.bg(theme.primary)
                                                                    .border_color(theme.primary)
                                                                    .text_color(theme.primary_foreground)
                                                                    .font_weight(FontWeight::MEDIUM)
                                                            })
                                                            .when(!is_selected, |s| {
                                                                s.border_color(theme.border)
                                                                    .text_color(theme.muted_foreground)
                                                                    .hover(|h| h.bg(theme.list_hover).text_color(theme.foreground))
                                                            })
                                                            .on_mouse_down(
                                                                MouseButton::Left,
                                                                cx.listener(move |this, _, _, cx| {
                                                                    if this.selected_folder.as_ref() == Some(&click_path) {
                                                                        this.selected_folder = None;
                                                                    } else {
                                                                        this.selected_folder = Some(click_path.clone());
                                                                    }
                                                                    cx.notify();
                                                                }),
                                                            )
                                                            .child(format!("{name} ({count})"))
                                                            .when(is_selected, |chip| {
                                                                chip.child(
                                                                    div()
                                                                        .id(SharedString::from(format!("remove-folder-{name}")))
                                                                        .role(gpui_kit::Role::Button)
                                                                        .aria_label(format!("Remove folder {name}"))
                                                                        .size(px(14.))
                                                                        .flex()
                                                                        .items_center()
                                                                        .justify_center()
                                                                        .rounded_full()
                                                                        .hover(|h| h.bg(theme.primary_foreground.opacity(0.25)))
                                                                        .child(Icon::new(IconName::Close).size(px(8.)))
                                                                        .on_mouse_down(
                                                                            MouseButton::Left,
                                                                            cx.listener(move |_this, _, _, cx| {
                                                                                cx.stop_propagation();
                                                                                cx.emit(NewTabEvent::RemoveFolder(remove_path.clone()));
                                                                            }),
                                                                        ),
                                                                )
                                                            })
                                                    })),
                                            )
                                        })
                                        .when(!filtered_files.is_empty(), |list| {
                                            list.child(Self::render_file_grid(&filtered_files, "all", self.selected_card_index, cx))
                                        })
                                        .when(filtered_files.is_empty(), |list| {
                                            list.child(
                                                v_flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .py_8()
                                                    .gap_2()
                                                    .child(
                                                        Icon::new(IconName::Search)
                                                            .size_6()
                                                            .text_color(theme.muted_foreground),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_color(theme.muted_foreground)
                                                            .child(if has_filter {
                                                                format!("No markdown files found matching \"{}\"", self.filter_query.trim())
                                                            } else {
                                                                "No markdown files found in tracked folders".to_string()
                                                            }),
                                                    ),
                                            )
                                        }),
                                ),
                        )
                    }),
            )
    }
}
