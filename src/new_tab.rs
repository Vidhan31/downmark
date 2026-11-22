use std::path::PathBuf;

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

use crate::catalog::{FolderCatalog, InstructionEntry, ProjectDocEntry, SkillEntry};

/// Events emitted by the Home page to be handled by the workspace.
#[derive(Clone, Debug)]
pub(crate) enum NewTabEvent {
    OpenFile(PathBuf),
    OpenFileInNewTab(PathBuf),
    PromptChooseFolder,
    SelectProjectFilter(Option<PathBuf>),
    RemoveProjectFolder(PathBuf),
    PromptOpenFile,
    Rescan,
}

/// The 5 distinct tabs splitting rules, skills, and docs across project and global scopes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum HubTab {
    #[default]
    ProjectSkills,
    ProjectRules,
    ProjectDocs,
    GlobalSkills,
    GlobalRules,
}

pub(crate) struct NewTabView {
    catalog: Entity<FolderCatalog>,
    filter_input: Entity<InputState>,
    filter_query: String,
    active_tab: HubTab,
    selected_item_index: usize,
    show_project_manager: bool,
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
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter in view…"));

        let subscriptions = vec![
            cx.observe(&catalog, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &filter_input,
                window,
                |this, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.filter_query = input.read(cx).text().to_string();
                        this.selected_item_index = 0;
                        cx.notify();
                    }
                },
            ),
        ];

        let has_projects = !catalog.read(cx).project_folders().is_empty();
        let active_tab = if has_projects {
            HubTab::ProjectSkills
        } else {
            HubTab::GlobalSkills
        };

        Self {
            catalog,
            filter_input,
            filter_query: String::new(),
            active_tab,
            selected_item_index: 0,
            show_project_manager: false,
            _subscriptions: subscriptions,
        }
    }

    /// Renders a sleek floating project workspace switcher popover.
    #[expect(
        clippy::too_many_lines,
        reason = "Declarative project workspace popover menu"
    )]
    fn render_project_popover(
        project_folders: &[PathBuf],
        selected_project_filter: Option<&PathBuf>,
        total_projects: usize,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let home_dir = std::env::var("HOME").unwrap_or_default();

        div()
            .id("project-popover-container")
            .absolute()
            .inset_0()
            .occlude()
            // Clicking backdrop dismisses the popover
            .child(
                div()
                    .id("project-popover-backdrop")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.show_project_manager = false;
                            cx.notify();
                        }),
                    ),
            )
            // Floating popover card anchored beneath the header workspace button
            .child(
                v_flex()
                    .id("project-popover-card")
                    .absolute()
                    .top(px(46.))
                    .left(px(84.))
                    .w(px(320.))
                    .max_h(px(380.))
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .p_2()
                    .gap_1p5()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    // Popover Header
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .px_2()
                            .py_1()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("Workspaces"),
                                    )
                                    .child(
                                        div()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded_full()
                                            .bg(theme.list_hover)
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.muted_foreground)
                                            .child(format!("{total_projects}")),
                                    ),
                            )
                            .child(
                                Button::new("close-project-popover-btn")
                                    .ghost()
                                    .small()
                                    .compact()
                                    .icon(Icon::new(IconName::Close))
                                    .tooltip("Close")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_project_manager = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    // Scrollable Workspaces List
                    .child(
                        v_flex()
                            .id("popover-workspaces-scroll")
                            .max_h(px(250.))
                            .overflow_y_scrollbar()
                            .gap_0p5()
                            // All Projects item
                            .child({
                                let is_active = selected_project_filter.is_none();
                                h_flex()
                                    .id("popover-filter-all")
                                    .role(gpui_kit::Role::Button)
                                    .aria_label("View all workspaces")
                                    .items_center()
                                    .justify_between()
                                    .px_2p5()
                                    .h_8()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(is_active, |s| {
                                        s.bg(theme.list_active).text_color(theme.foreground)
                                    })
                                    .when(!is_active, |s| s.hover(|h| h.bg(theme.list_hover)))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            this.show_project_manager = false;
                                            cx.emit(NewTabEvent::SelectProjectFilter(None));
                                        }),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                Icon::new(IconName::FolderOpen)
                                                    .size(px(14.))
                                                    .text_color(if is_active {
                                                        theme.primary
                                                    } else {
                                                        theme.muted_foreground
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(if is_active {
                                                        FontWeight::BOLD
                                                    } else {
                                                        FontWeight::MEDIUM
                                                    })
                                                    .child("All Workspaces"),
                                            ),
                                    )
                                    .when(is_active, |r| {
                                        r.child(
                                            Icon::new(IconName::Check)
                                                .size(px(13.))
                                                .text_color(theme.primary),
                                        )
                                    })
                            })
                            // Divider
                            .child(div().h_px().bg(theme.border).my_1())
                            // Individual Projects
                            .children(project_folders.iter().map(|path| {
                                let is_active = selected_project_filter == Some(path);
                                let name = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("project")
                                    .to_string();
                                let path_str = path.to_string_lossy().replace(&home_dir, "~");
                                let click_path = path.clone();
                                let remove_path = path.clone();

                                h_flex()
                                    .id(SharedString::from(format!("popover-item-{name}")))
                                    .role(gpui_kit::Role::Button)
                                    .aria_label(format!("Select workspace {name}"))
                                    .items_center()
                                    .justify_between()
                                    .px_2p5()
                                    .h_9()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(is_active, |s| {
                                        s.bg(theme.list_active).text_color(theme.foreground)
                                    })
                                    .when(!is_active, |s| s.hover(|h| h.bg(theme.list_hover)))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.show_project_manager = false;
                                            cx.emit(NewTabEvent::SelectProjectFilter(Some(
                                                click_path.clone(),
                                            )));
                                        }),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                Icon::new(IconName::Folder)
                                                    .size(px(13.))
                                                    .text_color(theme.muted_foreground),
                                            )
                                            .child(
                                                v_flex()
                                                    .gap_0()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .font_weight(if is_active {
                                                                FontWeight::BOLD
                                                            } else {
                                                                FontWeight::MEDIUM
                                                            })
                                                            .text_color(theme.foreground)
                                                            .truncate()
                                                            .child(name.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(theme.muted_foreground)
                                                            .truncate()
                                                            .child(path_str),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1p5()
                                            .when(is_active, |r| {
                                                r.child(
                                                    Icon::new(IconName::Check)
                                                        .size(px(13.))
                                                        .text_color(theme.primary),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "popover-rm-{name}"
                                                    )))
                                                    .role(gpui_kit::Role::Button)
                                                    .aria_label(format!("Remove workspace {name}"))
                                                    .size(px(18.))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded_md()
                                                    .hover(|h| {
                                                        h.bg(theme.muted_foreground.opacity(0.2))
                                                    })
                                                    .child(
                                                        Icon::new(IconName::Close)
                                                            .size(px(11.))
                                                            .text_color(theme.muted_foreground),
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(move |_this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            cx.emit(
                                                                NewTabEvent::RemoveProjectFolder(
                                                                    remove_path.clone(),
                                                                ),
                                                            );
                                                        }),
                                                    ),
                                            ),
                                    )
                            })),
                    )
                    // Footer: Add Project Folder Action
                    .child(
                        h_flex()
                            .pt_1p5()
                            .border_t_1()
                            .border_color(theme.border)
                            .child(
                                Button::new("popover-add-folder-btn")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::Plus))
                                    .child("Add Project Folder…")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_project_manager = false;
                                        cx.emit(NewTabEvent::PromptChooseFolder);
                                    })),
                            ),
                    ),
            )
    }

    /// Renders a segmented navigation tab button.
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "is_active flag toggles tab styling"
    )]
    fn render_tab_button(
        label: &str,
        count: usize,
        icon: IconName,
        tab: HubTab,
        is_active: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let bg_color = if is_active {
            theme.background
        } else {
            theme.list_hover.opacity(0.0)
        };
        let text_color = if is_active {
            theme.foreground
        } else {
            theme.muted_foreground
        };
        let border_color = if is_active {
            theme.border
        } else {
            theme.border.opacity(0.0)
        };

        h_flex()
            .id(SharedString::from(format!("tab-btn-{label}")))
            .role(gpui_kit::Role::Tab)
            .aria_selected(is_active)
            .items_center()
            .gap_1p5()
            .px_3()
            .py_1p5()
            .rounded_md()
            .text_xs()
            .cursor_pointer()
            .border_1()
            .border_color(border_color)
            .bg(bg_color)
            .text_color(text_color)
            .when(is_active, |s| s.font_weight(FontWeight::BOLD))
            .when(!is_active, |s| {
                s.hover(|h| {
                    h.bg(theme.background.opacity(0.5))
                        .text_color(theme.foreground)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.active_tab = tab;
                        this.selected_item_index = 0;
                        cx.notify();
                    }),
                )
            })
            .child(Icon::new(icon).size(px(14.)))
            .child(label.to_string())
            .child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_full()
                    .when(is_active, |b| {
                        b.bg(theme.primary.opacity(0.15)).text_color(theme.primary)
                    })
                    .when(!is_active, |b| {
                        b.bg(theme.border.opacity(0.5))
                            .text_color(theme.muted_foreground)
                    })
                    .text_xs()
                    .child(format!("{count}")),
            )
    }

    /// Renders a clean card for a Skill in the Master list pane.
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "is_selected flag toggles active row styling"
    )]
    fn render_skill_list_item(
        skill: &SkillEntry,
        idx: usize,
        is_selected: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let name = skill.name.clone();
        let source = skill.agent_source.clone();
        let comp_count = skill.companion_files.len();
        let full_path = skill.skill_file.full_path.clone();

        h_flex()
            .id(SharedString::from(format!("skill-item-{idx}")))
            .role(gpui_kit::Role::ListItem)
            .aria_selected(is_selected)
            .aria_label(format!("Skill: {name}"))
            .items_center()
            .justify_between()
            .h_12()
            .px_3()
            .rounded_lg()
            .cursor_pointer()
            .when(is_selected, |s| {
                s.bg(theme.list_active)
                    .border_1()
                    .border_color(theme.primary.opacity(0.45))
            })
            .when(!is_selected, |s| {
                s.border_1()
                    .border_color(theme.border.opacity(0.3))
                    .bg(theme.background)
                    .hover(|h| {
                        h.bg(theme.list_hover.opacity(0.5))
                            .border_color(theme.border.opacity(0.5))
                    })
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.selected_item_index = idx;
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |_this, _, _, cx| {
                    cx.emit(NewTabEvent::OpenFileInNewTab(full_path.clone()));
                }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(if is_selected {
                                FontWeight::BOLD
                            } else {
                                FontWeight::SEMIBOLD
                            })
                            .text_color(theme.foreground)
                            .truncate()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(source),
                    ),
            )
            .when(comp_count > 0, |r| {
                r.child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded_sm()
                        .bg(theme.list_hover)
                        .text_color(theme.muted_foreground)
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .flex_none()
                        .child(format!("{} docs", comp_count + 1)),
                )
            })
    }

    /// Renders the Right Detail Pane for a Skill, taking full advantage of horizontal space.
    #[expect(
        clippy::too_many_lines,
        reason = "Rich master-detail inspector layout for skills and companion files"
    )]
    fn render_skill_detail_pane(skill: &SkillEntry, cx: &Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        let skill_name = skill.name.clone();
        let description = skill.description.clone();
        let full_path = skill.skill_file.full_path.clone();
        let dir_display = skill
            .dir_path
            .to_string_lossy()
            .replace(&std::env::var("HOME").unwrap_or_default(), "~");
        let companions = skill.companion_files.clone();

        v_flex()
            .id("skill-detail-pane")
            .size_full()
            .p_6()
            .gap_6()
            .overflow_y_scrollbar()
            // Header
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_4()
                    .pb_4()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        v_flex()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .child(skill_name),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        Icon::new(IconName::FolderOpen)
                                            .size(px(13.))
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(dir_display),
                                    ),
                            ),
                    )
                    .child({
                        let p = full_path.clone();
                        let p_middle = full_path.clone();
                        Button::new("open-skill-md-btn")
                            .primary()
                            .icon(Icon::new(IconName::FileText))
                            .child("Open SKILL.md")
                            .on_click(cx.listener(move |_this, _, _, cx| {
                                cx.emit(NewTabEvent::OpenFile(p.clone()));
                            }))
                            .on_mouse_up(
                                MouseButton::Middle,
                                cx.listener(move |_this, _, _, cx| {
                                    cx.emit(NewTabEvent::OpenFileInNewTab(p_middle.clone()));
                                }),
                            )
                    }),
            )
            // Description / Overview (editorial typography, no heavy card)
            .when(!description.is_empty(), |pane| {
                pane.child(
                    v_flex()
                        .gap_1p5()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.muted_foreground)
                                .child("OVERVIEW"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .line_height(px(22.))
                                .text_color(theme.foreground.opacity(0.9))
                                .child(description),
                        ),
                )
            })
            // Package Documents Table
            .child(
                v_flex()
                    .gap_2p5()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.muted_foreground)
                                    .child("PACKAGE DOCUMENTS"),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .bg(theme.list_hover)
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{} files", companions.len() + 1)),
                            ),
                    )
                    .child(
                        v_flex()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme.border)
                            .overflow_hidden()
                            // Table Header
                            .child(
                                h_flex()
                                    .items_center()
                                    .h_8()
                                    .px_3()
                                    .bg(theme.list_hover.opacity(0.35))
                                    .border_b_1()
                                    .border_color(theme.border)
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("DOCUMENT"),
                                    )
                                    .child(
                                        div()
                                            .w(px(160.))
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("ROLE / TYPE"),
                                    )
                                    .child(
                                        div()
                                            .w(px(110.))
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("MODIFIED"),
                                    )
                                    .child(
                                        div()
                                            .w(px(80.))
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("SIZE"),
                                    ),
                            )
                            // Row for SKILL.md entrypoint
                            .child({
                                let p = full_path;
                                let p_middle = p.clone();
                                let size = skill.skill_file.formatted_size();
                                let mod_str = skill.skill_file.formatted_modified();
                                h_flex()
                                    .id("skill-entrypoint-row")
                                    .role(gpui_kit::Role::ListItem)
                                    .items_center()
                                    .h_10()
                                    .px_3()
                                    .border_b_1()
                                    .border_color(theme.border.opacity(0.4))
                                    .cursor_pointer()
                                    .hover(|h| h.bg(theme.list_hover))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |_this,
                                                  event: &gpui_kit::MouseDownEvent,
                                                  _,
                                                  cx| {
                                                if event.modifiers.control
                                                    || event.modifiers.platform
                                                {
                                                    cx.emit(NewTabEvent::OpenFileInNewTab(
                                                        p.clone(),
                                                    ));
                                                } else {
                                                    cx.emit(NewTabEvent::OpenFile(p.clone()));
                                                }
                                            },
                                        ),
                                    )
                                    .on_mouse_up(
                                        MouseButton::Middle,
                                        cx.listener(move |_this, _, _, cx| {
                                            cx.emit(NewTabEvent::OpenFileInNewTab(
                                                p_middle.clone(),
                                            ));
                                        }),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                Icon::new(IconName::FileText)
                                                    .size(px(14.))
                                                    .text_color(theme.primary),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(theme.foreground)
                                                    .child("SKILL.md"),
                                            ),
                                    )
                                    .child(
                                        h_flex().w(px(160.)).flex_none().child(
                                            div()
                                                .px_2()
                                                .py_0p5()
                                                .rounded_full()
                                                .bg(theme.primary.opacity(0.12))
                                                .text_color(theme.primary)
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child("Entrypoint"),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .w(px(110.))
                                            .flex_none()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(mod_str),
                                    )
                                    .child(
                                        div()
                                            .w(px(80.))
                                            .flex_none()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(size),
                                    )
                            })
                            // Companion Rows
                            .children(companions.into_iter().enumerate().map(|(ix, comp)| {
                                let p = comp.full_path.clone();
                                let p_middle = p.clone();
                                let rel = comp.relative_path.clone();
                                let name = comp.file_name.clone();
                                let size = comp.formatted_size();
                                let mod_str = comp.formatted_modified();
                                h_flex()
                                    .id(SharedString::from(format!("comp-row-{ix}")))
                                    .role(gpui_kit::Role::ListItem)
                                    .items_center()
                                    .h_10()
                                    .px_3()
                                    .border_b_1()
                                    .border_color(theme.border.opacity(0.4))
                                    .cursor_pointer()
                                    .hover(|h| h.bg(theme.list_hover))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |_this,
                                                  event: &gpui_kit::MouseDownEvent,
                                                  _,
                                                  cx| {
                                                if event.modifiers.control
                                                    || event.modifiers.platform
                                                {
                                                    cx.emit(NewTabEvent::OpenFileInNewTab(
                                                        p.clone(),
                                                    ));
                                                } else {
                                                    cx.emit(NewTabEvent::OpenFile(p.clone()));
                                                }
                                            },
                                        ),
                                    )
                                    .on_mouse_up(
                                        MouseButton::Middle,
                                        cx.listener(move |_this, _, _, cx| {
                                            cx.emit(NewTabEvent::OpenFileInNewTab(
                                                p_middle.clone(),
                                            ));
                                        }),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                Icon::new(IconName::FileText)
                                                    .size(px(13.))
                                                    .text_color(theme.muted_foreground),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(theme.foreground)
                                                    .truncate()
                                                    .child(name),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(160.))
                                            .flex_none()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .truncate()
                                            .child(if rel.is_empty() {
                                                "Companion doc".to_string()
                                            } else {
                                                rel
                                            }),
                                    )
                                    .child(
                                        div()
                                            .w(px(110.))
                                            .flex_none()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(mod_str),
                                    )
                                    .child(
                                        div()
                                            .w(px(80.))
                                            .flex_none()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(size),
                                    )
                            })),
                    ),
            )
    }

    /// Renders a full-width high-density table for Rules (both Global and Project),
    /// eliminating excessive UI ceremony, giant empty boxes, and redundant sidebars.
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "Global vs Project rule rendering flag"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "Renders full table view with header, empty states, and row elements"
    )]
    fn render_rules_table(
        rules: &[InstructionEntry],
        is_global: bool,
        total_projects: usize,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        if rules.is_empty() {
            let (title, msg) = if is_global {
                (
                    "No Global Rules Detected",
                    "No standard AI agent rule files were found in ~/.codex/, ~/.gemini/, ~/.claude/, or ~/.config/.",
                )
            } else if total_projects == 0 {
                (
                    "No Project Folders Chosen",
                    "Click 'Add Project' above to select your workspace or project folders.",
                )
            } else {
                (
                    "No Project Rule Files Found",
                    "Add AGENTS.md, CLAUDE.md, or .cursorrules to your project.",
                )
            };
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Bot)
                        .size_8()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(msg),
                );
        }

        v_flex()
            .size_full()
            // Table Header
            .child(
                h_flex()
                    .items_center()
                    .h_8()
                    .px_3()
                    .bg(theme.list_hover.opacity(0.35))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .w(px(220.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.muted_foreground)
                            .child(if is_global {
                                "AGENT / PROVIDER"
                            } else {
                                "PROJECT"
                            }),
                    )
                    .child(
                        div()
                            .w(px(160.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.muted_foreground)
                            .child("RULE FILE"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.muted_foreground)
                            .child("FILE PATH"),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.muted_foreground)
                            .child("MODIFIED"),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.muted_foreground)
                            .child("SIZE"),
                    )
                    .child(div().w(px(70.)).flex_none()),
            )
            // Table Rows
            .child(
                v_flex().flex_1().overflow_y_scrollbar().children(
                    rules
                        .iter()
                        .enumerate()
                        .map(|(ix, rule)| Self::render_rule_table_row(rule, is_global, ix, cx)),
                ),
            )
    }

    /// Renders a dense, responsive row for the Rules table.
    #[expect(
        clippy::fn_params_excessive_bools,
        clippy::too_many_lines,
        reason = "Declarative GPUI table row styling and column elements"
    )]
    fn render_rule_table_row(
        rule: &InstructionEntry,
        is_global: bool,
        idx: usize,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let full_path = rule.file.full_path.clone();
        let full_path_middle = full_path.clone();
        let name = rule.name.clone();
        let source = rule.agent_source.clone();
        let size_str = rule.file.formatted_size();
        let mod_str = rule.file.formatted_modified();

        let path_display = if is_global {
            full_path
                .to_string_lossy()
                .replace(&std::env::var("HOME").unwrap_or_default(), "~")
        } else {
            rule.file.relative_path.clone()
        };

        h_flex()
            .id(SharedString::from(format!("rule-row-{idx}")))
            .role(gpui_kit::Role::ListItem)
            .aria_label(format!("Rule: {name} ({source})"))
            .items_center()
            .h_10()
            .px_3()
            .border_b_1()
            .border_color(theme.border.opacity(0.4))
            .hover(|h| h.bg(theme.list_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, event: &gpui_kit::MouseDownEvent, _, cx| {
                    if event.modifiers.control || event.modifiers.platform {
                        cx.emit(NewTabEvent::OpenFileInNewTab(full_path.clone()));
                    } else {
                        cx.emit(NewTabEvent::OpenFile(full_path.clone()));
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |_this, _, _, cx| {
                    cx.emit(NewTabEvent::OpenFileInNewTab(full_path_middle.clone()));
                }),
            )
            // Column 1: Agent Provider or Project
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .w(px(220.))
                    .flex_none()
                    .child(
                        Icon::new(if is_global {
                            IconName::Bot
                        } else {
                            IconName::FolderOpen
                        })
                        .size(px(14.))
                        .text_color(theme.primary),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.foreground)
                            .truncate()
                            .child(source),
                    ),
            )
            // Column 2: Rule File
            .child(
                h_flex()
                    .items_center()
                    .gap_1p5()
                    .w(px(160.))
                    .flex_none()
                    .child(
                        Icon::new(IconName::FileText)
                            .size(px(13.))
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.foreground)
                            .child(name),
                    ),
            )
            // Column 3: Full / Relative Path
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(path_display),
            )
            // Column 4: Modified
            .child(
                div()
                    .w(px(110.))
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(mod_str),
            )
            // Column 5: Size
            .child(
                div()
                    .w(px(80.))
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(size_str),
            )
            // Column 6: Action
            .child({
                let p = rule.file.full_path.clone();
                let p_middle = p.clone();
                div().w(px(70.)).flex_none().flex().justify_end().child(
                    Button::new(SharedString::from(format!("open-rule-btn-{idx}")))
                        .ghost()
                        .small()
                        .child("Open")
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.emit(NewTabEvent::OpenFile(p.clone()));
                        }))
                        .on_mouse_up(
                            MouseButton::Middle,
                            cx.listener(move |_this, _, _, cx| {
                                cx.emit(NewTabEvent::OpenFileInNewTab(p_middle.clone()));
                            }),
                        ),
                )
            })
    }

    /// Renders a dense, responsive row for the Project Documentation table.
    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI table row for documentation"
    )]
    fn render_doc_table_row(
        doc: &ProjectDocEntry,
        idx: usize,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let full_path = doc.file.full_path.clone();
        let full_path_middle = full_path.clone();
        let name = doc.file.file_name.clone();
        let rel_path = doc.file.relative_path.clone();
        let size_str = doc.file.formatted_size();
        let mod_str = doc.file.formatted_modified();

        let is_priority = name.eq_ignore_ascii_case("README.md")
            || name.eq_ignore_ascii_case("CHANGELOG.md")
            || name.eq_ignore_ascii_case("CONTRIBUTING.md");

        h_flex()
            .id(SharedString::from(format!("doc-row-{idx}")))
            .role(gpui_kit::Role::ListItem)
            .aria_label(format!("Doc: {name}"))
            .items_center()
            .h_9()
            .px_3()
            .border_b_1()
            .border_color(theme.border.opacity(0.4))
            .hover(|h| h.bg(theme.list_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, event: &gpui_kit::MouseDownEvent, _, cx| {
                    if event.modifiers.control || event.modifiers.platform {
                        cx.emit(NewTabEvent::OpenFileInNewTab(full_path.clone()));
                    } else {
                        cx.emit(NewTabEvent::OpenFile(full_path.clone()));
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |_this, _, _, cx| {
                    cx.emit(NewTabEvent::OpenFileInNewTab(full_path_middle.clone()));
                }),
            )
            // Column 1: Name
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .w(px(280.))
                    .flex_none()
                    .child(
                        Icon::new(IconName::FileText)
                            .size(px(14.))
                            .text_color(if is_priority {
                                theme.primary
                            } else {
                                theme.muted_foreground
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(if is_priority {
                                FontWeight::BOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(theme.foreground)
                            .truncate()
                            .child(name),
                    ),
            )
            // Column 2: Location & Project
            .child(
                h_flex()
                    .flex_1()
                    .items_center()
                    .gap_1p5()
                    .truncate()
                    .when_some(doc.project_name.as_ref(), |row, proj| {
                        row.child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded_sm()
                                .bg(theme.primary.opacity(0.12))
                                .text_color(theme.primary)
                                .text_xs()
                                .flex_none()
                                .child(proj.clone()),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(rel_path),
                    ),
            )
            // Column 3: Modified
            .child(
                div()
                    .w(px(110.))
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(mod_str),
            )
            // Column 4: Size
            .child(
                div()
                    .w(px(80.))
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(size_str),
            )
            // Column 5: Action
            .child({
                let p = doc.file.full_path.clone();
                let p_middle = p.clone();
                div().w(px(60.)).flex_none().flex().justify_end().child(
                    Button::new(SharedString::from(format!("open-doc-btn-{idx}")))
                        .ghost()
                        .small()
                        .child("Open")
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.emit(NewTabEvent::OpenFile(p.clone()));
                        }))
                        .on_mouse_up(
                            MouseButton::Middle,
                            cx.listener(move |_this, _, _, cx| {
                                cx.emit(NewTabEvent::OpenFileInNewTab(p_middle.clone()));
                            }),
                        ),
                )
            })
    }
}

impl Render for NewTabView {
    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI layout for responsive multi-pane AI workflow hub"
    )]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        let catalog = self.catalog.read(cx);

        let project_folders = catalog.project_folders().to_vec();
        let selected_project_filter = catalog.selected_project_filter().cloned();
        let total_projects = project_folders.len();

        let filtered_hub = catalog.filter_hub(&self.filter_query);

        // Clamp selection indices
        let cur_skill_idx = self.selected_item_index;

        // Current active items based on selected tab for keyboard navigation
        let active_tab_paths: Vec<PathBuf> = match self.active_tab {
            HubTab::ProjectSkills => filtered_hub
                .project_skills
                .iter()
                .map(|s| s.skill_file.full_path.clone())
                .collect(),
            HubTab::ProjectRules => filtered_hub
                .project_instructions
                .iter()
                .map(|i| i.file.full_path.clone())
                .collect(),
            HubTab::ProjectDocs => filtered_hub
                .project_docs
                .iter()
                .map(|d| d.file.full_path.clone())
                .collect(),
            HubTab::GlobalSkills => filtered_hub
                .global_skills
                .iter()
                .map(|s| s.skill_file.full_path.clone())
                .collect(),
            HubTab::GlobalRules => filtered_hub
                .global_instructions
                .iter()
                .map(|i| i.file.full_path.clone())
                .collect(),
        };

        let nav_count = active_tab_paths.len();

        v_flex()
            .id("new-tab-page")
            .size_full()
            .relative()
            .bg(theme.background)
            .px_6()
            .py_4()
            .gap_3()
            .overflow_hidden()
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "down" => {
                        if nav_count > 0 {
                            this.selected_item_index = (this.selected_item_index + 1).min(nav_count - 1);
                            cx.notify();
                        }
                    }
                    "up" => {
                        if this.selected_item_index > 0 {
                            this.selected_item_index -= 1;
                            cx.notify();
                        } else {
                            this.filter_input.update(cx, |input, cx| input.focus(window, cx));
                        }
                    }
                    "enter" => {
                        if let Some(path) = active_tab_paths.get(this.selected_item_index).cloned() {
                            if event.keystroke.modifiers.secondary(cx) || event.keystroke.modifiers.shift {
                                cx.emit(NewTabEvent::OpenFileInNewTab(path));
                            } else {
                                cx.emit(NewTabEvent::OpenFile(path));
                            }
                        }
                    }
                    "escape" => {
                        if this.show_project_manager {
                            this.show_project_manager = false;
                            cx.notify();
                        } else if !this.filter_query.is_empty() {
                            this.filter_query.clear();
                            this.filter_input
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            }))
            // 1. Sleek Header Bar: Clean Title, Workspace Picker, and Actions
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .h_9()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2p5()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .child("Home"),
                            )
                            // Workspace Selector (or Add Folder if none)
                            .when(total_projects > 0, |header| {
                                let (label, is_filtered) = if let Some(ref filter) = selected_project_filter {
                                    (
                                        filter
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("Project")
                                            .to_string(),
                                        true,
                                    )
                                } else {
                                    (format!("All Projects ({total_projects})"), false)
                                };

                                header.child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            Button::new("workspace-picker-btn")
                                                .ghost()
                                                .small()
                                                .icon(Icon::new(IconName::FolderOpen))
                                                .child(label)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.show_project_manager = !this.show_project_manager;
                                                    cx.notify();
                                                })),
                                        )
                                        .when(is_filtered, |row| {
                                            row.child(
                                                Button::new("clear-project-filter-btn")
                                                    .ghost()
                                                    .small()
                                                    .compact()
                                                    .icon(Icon::new(IconName::Close))
                                                    .tooltip("Show all workspaces")
                                                    .on_click(cx.listener(|_this, _, _, cx| {
                                                        cx.emit(NewTabEvent::SelectProjectFilter(None));
                                                    })),
                                            )
                                        }),
                                )
                            })
                            .when(total_projects == 0, |header| {
                                header.child(
                                    Button::new("empty-add-project-btn")
                                        .ghost()
                                        .small()
                                        .icon(Icon::new(IconName::Plus))
                                        .child("Add Project Folder")
                                        .on_click(cx.listener(|_this, _, _, cx| {
                                            cx.emit(NewTabEvent::PromptChooseFolder);
                                        })),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Button::new("new-tab-open-file-btn")
                                    .accessibility_label("Open Markdown file")
                                    .ghost()
                                    .small()
                                    .icon(Icon::new(IconName::FileText))
                                    .child("Open File")
                                    .on_click(cx.listener(|_this, _, _, cx| {
                                        cx.emit(NewTabEvent::PromptOpenFile);
                                    })),
                            )
                            .child(
                                Button::new("new-tab-rescan-btn")
                                    .accessibility_label("Rescan paths")
                                    .ghost()
                                    .small()
                                    .compact()
                                    .icon(Icon::new(IconName::RotateCw))
                                    .tooltip("Rescan workspaces and global paths")
                                    .on_click(cx.listener(|_this, _, _, cx| {
                                        cx.emit(NewTabEvent::Rescan);
                                    })),
                            ),
                    ),
            )
            // 2. Segmented Navigation Tabs + Inline Filter Bar
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .p_1()
                            .rounded_lg()
                            .bg(theme.list_hover.opacity(0.35))
                            .border_1()
                            .border_color(theme.border.opacity(0.5))
                            .child(Self::render_tab_button(
                                "Project Skills",
                                filtered_hub.project_skills.len(),
                                IconName::Sparkles,
                                HubTab::ProjectSkills,
                                self.active_tab == HubTab::ProjectSkills,
                                cx,
                            ))
                            .child(Self::render_tab_button(
                                "Project Rules",
                                filtered_hub.project_instructions.len(),
                                IconName::Bot,
                                HubTab::ProjectRules,
                                self.active_tab == HubTab::ProjectRules,
                                cx,
                            ))
                            .child(Self::render_tab_button(
                                "Project Docs",
                                filtered_hub.project_docs.len(),
                                IconName::BookOpen,
                                HubTab::ProjectDocs,
                                self.active_tab == HubTab::ProjectDocs,
                                cx,
                            ))
                            .child(
                                div()
                                    .h_3p5()
                                    .w(px(1.))
                                    .mx_1()
                                    .bg(theme.border),
                            )
                            .child(Self::render_tab_button(
                                "Global Skills",
                                filtered_hub.global_skills.len(),
                                IconName::Sparkles,
                                HubTab::GlobalSkills,
                                self.active_tab == HubTab::GlobalSkills,
                                cx,
                            ))
                            .child(Self::render_tab_button(
                                "Global Rules",
                                filtered_hub.global_instructions.len(),
                                IconName::Bot,
                                HubTab::GlobalRules,
                                self.active_tab == HubTab::GlobalRules,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .w(px(240.))
                            .child(Input::new(&self.filter_input).cleanable(true)),
                    ),
            )
            // 4. Main Multi-Pane Content Area (Viewport Fitted, Master-Detail or Dense Table)
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .rounded_xl()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .overflow_hidden()
                    // Case A: Project Skills (Master-Detail)
                    .when(self.active_tab == HubTab::ProjectSkills, |pane| {
                        let skills = &filtered_hub.project_skills;
                        if skills.is_empty() {
                            let (title, msg) = if total_projects == 0 {
                                ("No Project Folders Chosen", "Click 'Add Project' above to select your workspace or project folders.")
                            } else {
                                ("No Project Skills Found", "Add skills to .agents/skills/ in your project to see them here.")
                            };
                            return pane.child(
                                v_flex()
                                    .size_full()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .child(Icon::new(IconName::Sparkles).size_8().text_color(theme.muted_foreground))
                                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(theme.foreground).child(title))
                                    .child(div().text_xs().text_color(theme.muted_foreground).child(msg)),
                            );
                        }
                        let selected_skill = skills.get(cur_skill_idx.min(skills.len() - 1));

                        pane.child(
                            h_flex()
                                .size_full()
                                // Left Master List
                                .child(
                                    v_flex()
                                        .w(px(380.))
                                        .h_full()
                                        .border_r_1()
                                        .border_color(theme.border)
                                        .overflow_y_scrollbar()
                                        .p_2p5()
                                        .gap_2()
                                        .children(skills.iter().enumerate().map(|(ix, skill)| {
                                            Self::render_skill_list_item(skill, ix, ix == cur_skill_idx, cx)
                                        })),
                                )
                                // Right Detail Pane
                                .child(
                                    div()
                                        .flex_1()
                                        .h_full()
                                        .min_w_0()
                                        .when_some(selected_skill, |detail, skill| {
                                            detail.child(Self::render_skill_detail_pane(skill, cx))
                                        }),
                                ),
                        )
                    })
                    // Case B: Project Rules (Full-Width High-Density Table)
                    .when(self.active_tab == HubTab::ProjectRules, |pane| {
                        pane.child(Self::render_rules_table(
                            &filtered_hub.project_instructions,
                            false,
                            total_projects,
                            cx,
                        ))
                    })
                    // Case C: Global Skills (Master-Detail)
                    .when(self.active_tab == HubTab::GlobalSkills, |pane| {
                        let skills = &filtered_hub.global_skills;
                        if skills.is_empty() {
                            return pane.child(
                                v_flex()
                                    .size_full()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .child(Icon::new(IconName::Sparkles).size_8().text_color(theme.muted_foreground))
                                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(theme.foreground).child("No Global Skills Detected"))
                                    .child(div().text_xs().text_color(theme.muted_foreground).child("Standard paths like ~/.agents/skills or ~/.gemini/config/skills were not found.")),
                            );
                        }
                        let selected_skill = skills.get(cur_skill_idx.min(skills.len() - 1));

                        pane.child(
                            h_flex()
                                .size_full()
                                // Left Master List
                                .child(
                                    v_flex()
                                        .w(px(380.))
                                        .h_full()
                                        .border_r_1()
                                        .border_color(theme.border)
                                        .overflow_y_scrollbar()
                                        .p_2p5()
                                        .gap_2()
                                        .children(skills.iter().enumerate().map(|(ix, skill)| {
                                            Self::render_skill_list_item(skill, ix, ix == cur_skill_idx, cx)
                                        })),
                                )
                                // Right Detail Pane
                                .child(
                                    div()
                                        .flex_1()
                                        .h_full()
                                        .min_w_0()
                                        .when_some(selected_skill, |detail, skill| {
                                            detail.child(Self::render_skill_detail_pane(skill, cx))
                                        }),
                                ),
                        )
                    })
                    // Case D: Global Rules (Full-Width High-Density Table)
                    .when(self.active_tab == HubTab::GlobalRules, |pane| {
                        pane.child(Self::render_rules_table(
                            &filtered_hub.global_instructions,
                            true,
                            total_projects,
                            cx,
                        ))
                    })
                    // Case E: Project Docs (Full-Width High-Density Table)
                    .when(self.active_tab == HubTab::ProjectDocs, |pane| {
                        let docs = &filtered_hub.project_docs;
                        if docs.is_empty() {
                            let (title, msg) = if total_projects == 0 {
                                ("No Project Folders Chosen", "Click 'Add Project' above to select your workspace or project folders.")
                            } else {
                                ("No Project Documentation Found", "No general Markdown files (README, specs, guides) were found in chosen project folders.")
                            };
                            return pane.child(
                                v_flex()
                                    .size_full()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .child(Icon::new(IconName::BookOpen).size_8().text_color(theme.muted_foreground))
                                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(theme.foreground).child(title))
                                    .child(div().text_xs().text_color(theme.muted_foreground).child(msg)),
                            );
                        }

                        pane.child(
                            v_flex()
                                .size_full()
                                .// Table Header
                                child(
                                    h_flex()
                                        .items_center()
                                        .h_8()
                                        .px_3()
                                        .bg(theme.list_hover.opacity(0.35))
                                        .border_b_1()
                                        .border_color(theme.border)
                                        .child(
                                            div()
                                                .w(px(280.))
                                                .flex_none()
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.muted_foreground)
                                                .child("FILE NAME"),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.muted_foreground)
                                                .child("PROJECT & PATH"),
                                        )
                                        .child(
                                            div()
                                                .w(px(110.))
                                                .flex_none()
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.muted_foreground)
                                                .child("MODIFIED"),
                                        )
                                        .child(
                                            div()
                                                .w(px(80.))
                                                .flex_none()
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.muted_foreground)
                                                .child("SIZE"),
                                        )
                                        .child(
                                            div()
                                                .w(px(60.))
                                                .flex_none()
                                                .flex()
                                                .justify_end()
                                                .text_xs()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme.muted_foreground)
                                                .child("ACTION"),
                                        ),
                                )
                                // Table Body Rows
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .overflow_y_scrollbar()
                                        .children(docs.iter().enumerate().map(|(ix, doc)| {
                                            Self::render_doc_table_row(doc, ix, cx)
                                        })),
                                ),
                        )
                    }),
            )
            .when(self.show_project_manager, |this| {
                this.child(Self::render_project_popover(
                    &project_folders,
                    selected_project_filter.as_ref(),
                    total_projects,
                    cx,
                ))
            })
    }
}
