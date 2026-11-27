use std::path::{Path, PathBuf};

use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled as _, Subscription, Window,
    assets::IconName,
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        resizable::{h_resizable, resizable_panel},
        scroll::ScrollableElement as _,
        tab::{Tab, TabBar},
        v_flex,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

use crate::catalog::{FolderCatalog, InstructionEntry, ProjectDocEntry, SkillEntry};

/// Ordered list of category tabs in the Home hub.
const HUB_TABS: [HubTab; 5] = [
    HubTab::ProjectSkills,
    HubTab::ProjectRules,
    HubTab::ProjectDocs,
    HubTab::GlobalSkills,
    HubTab::GlobalRules,
];

/// Default width of the skills sidebar in pixels.
const SKILLS_SIDEBAR_WIDTH_PX: f32 = 268.;
/// Minimum width when dragging the skills sidebar splitter.
const SKILLS_SIDEBAR_MIN_WIDTH_PX: f32 = 210.;
/// Maximum width when dragging the skills sidebar splitter.
const SKILLS_SIDEBAR_MAX_WIDTH_PX: f32 = 400.;

/// Events emitted by the Home page to be handled by the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
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

impl HubTab {
    fn from_index(index: usize) -> Option<Self> {
        HUB_TABS.get(index).copied()
    }

    fn index(self) -> usize {
        HUB_TABS
            .iter()
            .position(|tab| *tab == self)
            .unwrap_or_default()
    }
}

/// Whether an empty category view is due to an active filter or genuinely no entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmptyStateKind {
    NoFilterMatches,
    GenuinelyEmpty,
}

/// Visual selection state for tabs and sidebar rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionState {
    Selected,
    Unselected,
}

impl SelectionState {
    fn from_eq<T: PartialEq>(current: &T, target: &T) -> Self {
        if current == target {
            Self::Selected
        } else {
            Self::Unselected
        }
    }

    fn is_selected(self) -> bool {
        matches!(self, Self::Selected)
    }
}

/// Contiguous section of skills sharing the same `agent_source`, keeping
/// display indices aligned with keyboard navigation.
struct SkillGroup<'a> {
    source: &'a str,
    items: Vec<(usize, &'a SkillEntry)>,
}

/// Groups `skills` by `agent_source` (sorted case-insensitively) and returns
/// both the flat display-ordered slice and the grouped sections with matching
/// flat indices `0..len`.
fn group_skills_by_source<'a>(
    skills: &'a [SkillEntry],
) -> (Vec<&'a SkillEntry>, Vec<SkillGroup<'a>>) {
    let mut sources: Vec<&'a str> = Vec::new();
    for skill in skills {
        let src = skill.agent_source.as_str();
        if !sources
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(src))
        {
            sources.push(src);
        }
    }
    sources.sort_by_key(|a| a.to_ascii_lowercase());

    let mut ordered = Vec::with_capacity(skills.len());
    let mut groups = Vec::with_capacity(sources.len());

    for source in sources {
        let mut items = Vec::new();
        for skill in skills {
            if skill.agent_source.eq_ignore_ascii_case(source) {
                let idx = ordered.len();
                ordered.push(skill);
                items.push((idx, skill));
            }
        }
        if !items.is_empty() {
            groups.push(SkillGroup { source, items });
        }
    }

    (ordered, groups)
}

fn abbreviate_home_path(path: &Path) -> String {
    if let Some(home) = crate::catalog::user_home_dir()
        && let Ok(rel) = path.strip_prefix(&home)
    {
        if rel.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rel.display());
    }
    path.display().to_string()
}

pub(crate) struct NewTabView {
    catalog: Entity<FolderCatalog>,
    filter_input: Entity<InputState>,
    filter_query: String,
    active_tab: HubTab,
    selected_item_index: usize,
    table_scroll_handle: ScrollHandle,
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
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter…"));

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
            table_scroll_handle: ScrollHandle::new(),
            show_project_manager: false,
            _subscriptions: subscriptions,
        }
    }

    fn emit_open_file_for_click(event: &ClickEvent, path: PathBuf, cx: &mut Context<'_, Self>) {
        let modifiers = event.modifiers();
        if modifiers.control || modifiers.platform {
            cx.emit(NewTabEvent::OpenFileInNewTab(path));
        } else {
            cx.emit(NewTabEvent::OpenFile(path));
        }
    }

    /// Binds left-click (respecting Ctrl/Cmd to open in a new tab) and middle-click
    /// handlers to a row or item so actions fire only after press and release on the same element.
    fn with_open_file_clicks<E: StatefulInteractiveElement>(
        element: E,
        path: PathBuf,
        cx: &Context<'_, Self>,
    ) -> E {
        let path_middle = path.clone();
        element
            .on_click(cx.listener(move |_this, event: &ClickEvent, _, cx| {
                Self::emit_open_file_for_click(event, path.clone(), cx);
            }))
            .on_aux_click(cx.listener(move |_this, event: &ClickEvent, _, cx| {
                if event.is_middle_click() {
                    cx.emit(NewTabEvent::OpenFileInNewTab(path_middle.clone()));
                }
            }))
    }

    /// Wraps an "Open" button with left-click (respecting Ctrl/Cmd to open in a new tab)
    /// and middle-click handlers that wait for release on the button and stop propagation
    /// to any enclosing clickable row.
    fn render_open_file_button(
        button: Button,
        wrap_id: impl Into<ElementId>,
        path: PathBuf,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let path_left = path.clone();
        let path_middle = path;
        div()
            .id(wrap_id)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
            .on_aux_click(cx.listener(move |_this, event: &ClickEvent, _, cx| {
                if event.is_middle_click() {
                    cx.stop_propagation();
                    cx.emit(NewTabEvent::OpenFileInNewTab(path_middle.clone()));
                }
            }))
            .child(
                button.on_click(cx.listener(move |_this, event: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    Self::emit_open_file_for_click(event, path_left.clone(), cx);
                })),
            )
    }

    /// Renders the floating workspace switcher popover anchored to the top toolbar.
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

        div()
            .id("project-popover-container")
            .absolute()
            .inset_0()
            .occlude()
            .child(
                div()
                    .id("project-popover-backdrop")
                    .debug_selector(|| "project-popover-backdrop".to_string())
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
            .child(
                v_flex()
                    .id("project-popover-card")
                    .absolute()
                    .top(px(42.))
                    .left(px(76.))
                    .w(px(320.))
                    .max_h(px(380.))
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .shadow_lg()
                    .p_2()
                    .gap_1p5()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
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
                                            .text_xs()
                                            .text_color(theme.muted_foreground.opacity(0.75))
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
                    .child(
                        v_flex()
                            .id("popover-workspaces-scroll")
                            .max_h(px(250.))
                            .overflow_y_scrollbar()
                            .gap_0p5()
                            .child({
                                let is_active = selected_project_filter.is_none();
                                h_flex()
                                    .id("popover-filter-all")
                                    .debug_selector(|| "popover-filter-all".to_string())
                                    .role(gpui_kit::Role::Button)
                                    .aria_label("View all workspaces")
                                    .items_center()
                                    .justify_between()
                                    .px_2p5()
                                    .h_8()
                                    .rounded(theme.radius)
                                    .cursor_pointer()
                                    .when(is_active, |s| {
                                        s.bg(theme.list_active).text_color(theme.foreground)
                                    })
                                    .when(!is_active, |s| s.hover(|h| h.bg(theme.list_hover)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_project_manager = false;
                                        cx.emit(NewTabEvent::SelectProjectFilter(None));
                                    }))
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
                                                        FontWeight::SEMIBOLD
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
                            .child(div().h_px().bg(theme.border).my_1())
                            .children(project_folders.iter().map(|path| {
                                let is_active = selected_project_filter == Some(path);
                                let name = path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("project")
                                    .to_string();
                                let path_str = abbreviate_home_path(path);
                                let click_path = path.clone();
                                let remove_path = path.clone();
                                let row_id = format!("popover-item-{}", path.to_string_lossy());
                                let item_selector = format!("popover-item-{name}");
                                let rm_selector = format!("popover-rm-{name}");

                                h_flex()
                                    .id(SharedString::from(row_id))
                                    .debug_selector(move || item_selector)
                                    .role(gpui_kit::Role::Button)
                                    .aria_label(format!("Select workspace {name}"))
                                    .items_center()
                                    .justify_between()
                                    .px_2p5()
                                    .h_9()
                                    .rounded(theme.radius)
                                    .cursor_pointer()
                                    .when(is_active, |s| {
                                        s.bg(theme.list_active).text_color(theme.foreground)
                                    })
                                    .when(!is_active, |s| s.hover(|h| h.bg(theme.list_hover)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.show_project_manager = false;
                                        cx.emit(NewTabEvent::SelectProjectFilter(Some(
                                            click_path.clone(),
                                        )));
                                    }))
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
                                                                FontWeight::SEMIBOLD
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
                                                    .debug_selector(move || rm_selector)
                                                    .role(gpui_kit::Role::Button)
                                                    .aria_label(format!("Remove workspace {name}"))
                                                    .size(px(18.))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(theme.radius)
                                                    .hover(|h| {
                                                        h.bg(theme.muted_foreground.opacity(0.2))
                                                    })
                                                    .child(
                                                        Icon::new(IconName::Close)
                                                            .size(px(11.))
                                                            .text_color(theme.muted_foreground),
                                                    )
                                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                        cx.stop_propagation();
                                                    })
                                                    .on_click(cx.listener(
                                                        move |_this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            cx.emit(
                                                                NewTabEvent::RemoveProjectFolder(
                                                                    remove_path.clone(),
                                                                ),
                                                            );
                                                        },
                                                    )),
                                            ),
                                    )
                            })),
                    )
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

    /// Renders a centered empty-state message with icon, title, and guidance.
    fn render_empty_state(
        icon: IconName,
        title: &'static str,
        message: &'static str,
        cx: &Context<'_, Self>,
    ) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(Icon::new(icon).size_8().text_color(theme.muted_foreground))
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
                    .child(message),
            )
            .into_any_element()
    }

    fn render_no_filter_matches_state(
        icon: IconName,
        cx: &Context<'_, Self>,
    ) -> gpui_kit::AnyElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2p5()
            .child(Icon::new(icon).size_8().text_color(theme.muted_foreground))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .child("No Matching Items"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No entries in this category match the current filter."),
            )
            .child(
                Button::new("clear-filter-btn")
                    .debug_selector(|| "clear-filter-btn".to_string())
                    .ghost()
                    .small()
                    .child("Clear Filter")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.filter_query.clear();
                        this.filter_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.selected_item_index = 0;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Builds a native underline category tab with icon, label, and plain muted count.
    fn build_category_tab(
        label: &'static str,
        count: usize,
        icon: IconName,
        selection: SelectionState,
        cx: &Context<'_, Self>,
    ) -> Tab {
        let theme = cx.theme();
        let is_active = selection.is_selected();
        let icon_color = if is_active {
            theme.primary
        } else {
            theme.muted_foreground
        };
        let label_color = if is_active {
            theme.foreground
        } else {
            theme.muted_foreground
        };
        let count_color = theme
            .muted_foreground
            .opacity(if is_active { 0.9 } else { 0.65 });

        Tab::new().aria_label(label).px_1p5().child(
            h_flex()
                .items_center()
                .gap_1p5()
                .child(Icon::new(icon).xsmall().text_color(icon_color))
                .child(
                    div()
                        .text_color(label_color)
                        .when(is_active, |d| d.font_weight(FontWeight::MEDIUM))
                        .child(label),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(count_color)
                        .child(format!("{count}")),
                ),
        )
    }

    /// Renders a borderless, desktop-native source-list row for a Skill.
    fn render_skill_sidebar_item(
        skill: &SkillEntry,
        idx: usize,
        selection: SelectionState,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let is_selected = selection.is_selected();
        let name = skill.name.clone();
        let source = skill.agent_source.clone();
        let comp_count = skill.companion_files.len();
        let full_path = skill.skill_file.full_path.clone();

        h_flex()
            .id(SharedString::from(format!("skill-item-{idx}")))
            .debug_selector(move || format!("skill-item-{idx}"))
            .role(gpui_kit::Role::ListItem)
            .aria_selected(is_selected)
            .aria_label(format!("Skill: {name} ({source})"))
            .items_center()
            .h_8()
            .px_2p5()
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(is_selected, |s| {
                s.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .when(!is_selected, |s| {
                s.text_color(theme.sidebar_foreground)
                    .hover(|h| h.bg(theme.sidebar_accent.opacity(0.55)))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_item_index = idx;
                cx.notify();
            }))
            .on_aux_click(cx.listener(move |_this, event: &ClickEvent, _, cx| {
                if event.is_middle_click() {
                    cx.emit(NewTabEvent::OpenFileInNewTab(full_path.clone()));
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .when(is_selected, |d| d.font_weight(FontWeight::MEDIUM))
                    .truncate()
                    .child(name),
            )
            .when(comp_count > 0, |r| {
                r.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground.opacity(0.8))
                        .child(format!("{} docs", comp_count + 1)),
                )
            })
    }

    /// Renders a single source-grouped section inside the skills sidebar.
    fn render_skill_sidebar_group(
        group_ix: usize,
        group: SkillGroup<'_>,
        selected_idx: usize,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let group_count = group.items.len();
        let header_label = group.source.to_uppercase();

        v_flex()
            .gap_0p5()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .px_2p5()
                    .when(group_ix == 0, gpui_kit::Styled::pt_1)
                    .when(group_ix > 0, gpui_kit::Styled::pt_3)
                    .pb_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground.opacity(0.8))
                            .truncate()
                            .child(header_label),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground.opacity(0.65))
                            .child(format!("{group_count}")),
                    ),
            )
            .children(group.items.into_iter().map(|(ix, skill)| {
                Self::render_skill_sidebar_item(
                    skill,
                    ix,
                    SelectionState::from_eq(&ix, &selected_idx),
                    cx,
                )
            }))
    }

    /// Renders the resizable source-list sidebar and right detail pane for Skills.
    #[expect(
        clippy::too_many_arguments,
        reason = "Master-detail skills view with precomputed grouping and empty-state context"
    )]
    fn render_skills_master_detail(
        &self,
        id: &'static str,
        ordered: &[&SkillEntry],
        groups: Vec<SkillGroup<'_>>,
        empty_kind: EmptyStateKind,
        empty_title: &'static str,
        empty_msg: &'static str,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        if ordered.is_empty() {
            if empty_kind == EmptyStateKind::NoFilterMatches {
                return Self::render_no_filter_matches_state(IconName::Sparkles, cx);
            }
            return Self::render_empty_state(IconName::Sparkles, empty_title, empty_msg, cx);
        }

        let selected_idx = self
            .selected_item_index
            .min(ordered.len().saturating_sub(1));
        let selected_skill = ordered.get(selected_idx).copied();

        h_resizable(id)
            .child(
                resizable_panel()
                    .size(px(SKILLS_SIDEBAR_WIDTH_PX))
                    .size_range(px(SKILLS_SIDEBAR_MIN_WIDTH_PX)..px(SKILLS_SIDEBAR_MAX_WIDTH_PX))
                    .fixed()
                    .child(
                        v_flex()
                            .id(SharedString::from(format!("{id}-sidebar")))
                            .size_full()
                            .bg(theme.sidebar)
                            .overflow_y_scrollbar()
                            .px_2()
                            .py_2()
                            .gap_0p5()
                            .children(groups.into_iter().enumerate().map(|(group_ix, group)| {
                                Self::render_skill_sidebar_group(group_ix, group, selected_idx, cx)
                            })),
                    ),
            )
            .child(
                resizable_panel()
                    .size_range(px(320.)..gpui_kit::Pixels::MAX)
                    .child(
                        div()
                            .size_full()
                            .min_w_0()
                            .bg(theme.background)
                            .when_some(selected_skill, |detail, skill| {
                                detail.child(Self::render_skill_detail_pane(skill, cx))
                            }),
                    ),
            )
            .into_any_element()
    }

    /// Renders the Right Detail Pane for a Skill with clean desktop hierarchy.
    #[expect(
        clippy::too_many_lines,
        reason = "Rich master-detail inspector layout for skills and companion files"
    )]
    fn render_skill_detail_pane(skill: &SkillEntry, cx: &Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        let skill_name = skill.name.clone();
        let agent_source = skill.agent_source.clone();
        let description = skill.description.clone();
        let full_path = skill.skill_file.full_path.clone();
        let dir_display = abbreviate_home_path(&skill.dir_path);
        let companions = skill.companion_files.clone();
        let total_docs = companions.len() + 1;
        let files_count_label = if total_docs == 1 {
            "1 file".to_string()
        } else {
            format!("{total_docs} files")
        };

        v_flex()
            .id("skill-detail-pane")
            .size_full()
            .p_6()
            .gap_5()
            .overflow_y_scrollbar()
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
                            .gap_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.foreground)
                                    .truncate()
                                    .child(skill_name),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        Icon::new(IconName::FolderOpen)
                                            .size(px(13.))
                                            .flex_none()
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .truncate()
                                            .child(format!("{agent_source} · {dir_display}")),
                                    ),
                            ),
                    )
                    .child({
                        let button = Button::new("open-skill-md-btn")
                            .debug_selector(|| "open-skill-md-btn".to_string())
                            .primary()
                            .icon(Icon::new(IconName::FileText))
                            .child("Open SKILL.md");
                        Self::render_open_file_button(
                            button,
                            "open-skill-md-btn-wrap",
                            full_path.clone(),
                            cx,
                        )
                    }),
            )
            .when(!description.is_empty(), |pane| {
                pane.child(
                    v_flex()
                        .gap_1p5()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
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
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.muted_foreground)
                                    .child("PACKAGE DOCUMENTS"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground.opacity(0.75))
                                    .child(files_count_label),
                            ),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .child(
                                h_flex()
                                    .items_center()
                                    .h_8()
                                    .px_3()
                                    .bg(theme.sidebar)
                                    .border_y_1()
                                    .border_color(theme.border)
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("DOCUMENT"),
                                    )
                                    .child(
                                        div()
                                            .w(px(160.))
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("ROLE / TYPE"),
                                    )
                                    .child(
                                        div()
                                            .w(px(110.))
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("MODIFIED"),
                                    )
                                    .child(
                                        div()
                                            .w(px(80.))
                                            .flex_none()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.muted_foreground)
                                            .child("SIZE"),
                                    ),
                            )
                            .child({
                                let size = skill.skill_file.formatted_size();
                                let mod_str = skill.skill_file.formatted_modified();
                                Self::with_open_file_clicks(
                                    h_flex()
                                        .id("skill-entrypoint-row")
                                        .debug_selector(|| "skill-entrypoint-row".to_string())
                                        .role(gpui_kit::Role::ListItem)
                                        .items_center()
                                        .h_9()
                                        .px_3()
                                        .border_b_1()
                                        .border_color(theme.border.opacity(0.4))
                                        .cursor_pointer()
                                        .hover(|h| h.bg(theme.list_hover)),
                                    full_path,
                                    cx,
                                )
                                .child(
                                    h_flex().items_center().flex_1().min_w_0().child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.foreground)
                                            .child("SKILL.md"),
                                    ),
                                )
                                .child(
                                    div()
                                        .w(px(160.))
                                        .flex_none()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme.primary)
                                        .child("Entrypoint"),
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
                            .children(companions.into_iter().enumerate().map(|(ix, comp)| {
                                let p = comp.full_path.clone();
                                let rel = comp.relative_path.clone();
                                let name = comp.file_name.clone();
                                let size = comp.formatted_size();
                                let mod_str = comp.formatted_modified();
                                Self::with_open_file_clicks(
                                    h_flex()
                                        .id(SharedString::from(format!("comp-row-{ix}")))
                                        .debug_selector(move || format!("comp-row-{ix}"))
                                        .role(gpui_kit::Role::ListItem)
                                        .items_center()
                                        .h_9()
                                        .px_3()
                                        .border_b_1()
                                        .border_color(theme.border.opacity(0.4))
                                        .cursor_pointer()
                                        .hover(|h| h.bg(theme.list_hover)),
                                    p,
                                    cx,
                                )
                                .child(
                                    h_flex().items_center().flex_1().min_w_0().child(
                                        div()
                                            .text_sm()
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

    /// Renders a full-width high-density table for Rules (both Global and Project).
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "Global vs Project rule rendering flag"
    )]
    fn render_rules_table(
        rules: &[InstructionEntry],
        is_global: bool,
        total_projects: usize,
        empty_kind: EmptyStateKind,
        selected_idx: usize,
        scroll_handle: &ScrollHandle,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        if rules.is_empty() {
            if empty_kind == EmptyStateKind::NoFilterMatches {
                return Self::render_no_filter_matches_state(IconName::Bot, cx);
            }
            let (title, msg) = if is_global {
                (
                    "No Global Rules Detected",
                    "No standard AI agent rule files were found in ~/.codex/, ~/.gemini/, ~/.claude/, or ~/.config/.",
                )
            } else if total_projects == 0 {
                (
                    "No Project Folders Chosen",
                    "Click 'Add Project Folder' above to select your workspace or project folders.",
                )
            } else {
                (
                    "No Project Rule Files Found",
                    "Add AGENTS.md, CLAUDE.md, or .cursorrules to your project.",
                )
            };
            return Self::render_empty_state(IconName::Bot, title, msg, cx);
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .items_center()
                    .h_8()
                    .px_4()
                    .bg(theme.sidebar)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .w(px(220.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
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
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("RULE FILE"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("FILE PATH"),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("MODIFIED"),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("SIZE"),
                    )
                    .child(div().w(px(70.)).flex_none()),
            )
            .child(
                v_flex()
                    .id(if is_global {
                        "global-rules-scroll"
                    } else {
                        "project-rules-scroll"
                    })
                    .flex_1()
                    .track_scroll(scroll_handle)
                    .overflow_y_scrollbar()
                    .children(rules.iter().enumerate().map(|(ix, rule)| {
                        Self::render_rule_table_row(rule, is_global, ix, ix == selected_idx, cx)
                    })),
            )
            .into_any_element()
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
        is_selected: bool,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let full_path = rule.file.full_path.clone();
        let name = rule.name.clone();
        let source = rule.agent_source.clone();
        let size_str = rule.file.formatted_size();
        let mod_str = rule.file.formatted_modified();

        let path_display = if is_global {
            abbreviate_home_path(&full_path)
        } else {
            rule.file.relative_path.clone()
        };

        Self::with_open_file_clicks(
            h_flex()
                .id(SharedString::from(format!("rule-row-{idx}")))
                .debug_selector(move || format!("rule-row-{idx}"))
                .role(gpui_kit::Role::ListItem)
                .aria_selected(is_selected)
                .aria_label(format!("Rule: {name} ({source})"))
                .items_center()
                .h_9()
                .px_4()
                .border_b_1()
                .border_color(theme.border.opacity(0.4))
                .cursor_pointer()
                .when(is_selected, |s| s.bg(theme.list_active))
                .when(!is_selected, |s| s.hover(|h| h.bg(theme.list_hover))),
            full_path,
            cx,
        )
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
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .truncate()
                        .child(source),
                ),
        )
        .child(
            h_flex().items_center().w(px(160.)).flex_none().child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .child(name),
            ),
        )
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .truncate()
                .child(path_display),
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
                .child(size_str),
        )
        .child({
            let p = rule.file.full_path.clone();
            let button = Button::new(SharedString::from(format!("open-rule-btn-{idx}")))
                .debug_selector(move || format!("open-rule-btn-{idx}"))
                .ghost()
                .small()
                .child("Open");
            div()
                .w(px(70.))
                .flex_none()
                .flex()
                .justify_end()
                .child(Self::render_open_file_button(
                    button,
                    SharedString::from(format!("open-rule-btn-wrap-{idx}")),
                    p,
                    cx,
                ))
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
        selection: SelectionState,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let is_selected = selection.is_selected();
        let theme = cx.theme();
        let full_path = doc.file.full_path.clone();
        let name = doc.file.file_name.clone();
        let rel_path = doc.file.relative_path.clone();
        let size_str = doc.file.formatted_size();
        let mod_str = doc.file.formatted_modified();

        let is_priority = name.eq_ignore_ascii_case("README.md")
            || name.eq_ignore_ascii_case("CHANGELOG.md")
            || name.eq_ignore_ascii_case("CONTRIBUTING.md");

        Self::with_open_file_clicks(
            h_flex()
                .id(SharedString::from(format!("doc-row-{idx}")))
                .debug_selector(move || format!("doc-row-{idx}"))
                .role(gpui_kit::Role::ListItem)
                .aria_selected(is_selected)
                .aria_label(format!("Doc: {name}"))
                .items_center()
                .h_9()
                .px_4()
                .border_b_1()
                .border_color(theme.border.opacity(0.4))
                .cursor_pointer()
                .when(is_selected, |s| s.bg(theme.list_active))
                .when(!is_selected, |s| s.hover(|h| h.bg(theme.list_hover))),
            full_path,
            cx,
        )
        .child(
            h_flex().items_center().w(px(280.)).flex_none().child(
                div()
                    .text_xs()
                    .font_weight(if is_priority {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(theme.foreground)
                    .truncate()
                    .child(name),
            ),
        )
        .child(
            h_flex()
                .flex_1()
                .items_center()
                .gap_1p5()
                .truncate()
                .when_some(doc.project_name.as_ref(), |row, proj| {
                    row.child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.primary)
                            .flex_none()
                            .child(proj.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground.opacity(0.6))
                            .flex_none()
                            .child("·"),
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
                .child(size_str),
        )
        .child({
            let p = doc.file.full_path.clone();
            let button = Button::new(SharedString::from(format!("open-doc-btn-{idx}")))
                .debug_selector(move || format!("open-doc-btn-{idx}"))
                .ghost()
                .small()
                .child("Open");
            div()
                .w(px(60.))
                .flex_none()
                .flex()
                .justify_end()
                .child(Self::render_open_file_button(
                    button,
                    SharedString::from(format!("open-doc-btn-wrap-{idx}")),
                    p,
                    cx,
                ))
        })
    }

    /// Renders the Project Documentation table view.
    fn render_project_docs_table(
        docs: &[ProjectDocEntry],
        total_projects: usize,
        empty_kind: EmptyStateKind,
        selected_idx: usize,
        scroll_handle: &ScrollHandle,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        if docs.is_empty() {
            if empty_kind == EmptyStateKind::NoFilterMatches {
                return Self::render_no_filter_matches_state(IconName::BookOpen, cx);
            }
            let (title, msg) = if total_projects == 0 {
                (
                    "No Project Folders Chosen",
                    "Click 'Add Project Folder' above to select your workspace or project folders.",
                )
            } else {
                (
                    "No Project Documentation Found",
                    "No general Markdown files (README, specs, guides) were found in chosen project folders.",
                )
            };
            return Self::render_empty_state(IconName::BookOpen, title, msg, cx);
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .items_center()
                    .h_8()
                    .px_4()
                    .bg(theme.sidebar)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .w(px(280.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("FILE NAME"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("PROJECT & PATH"),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("MODIFIED"),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
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
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted_foreground)
                            .child("ACTION"),
                    ),
            )
            .child(
                v_flex()
                    .id("project-docs-scroll")
                    .flex_1()
                    .track_scroll(scroll_handle)
                    .overflow_y_scrollbar()
                    .children(docs.iter().enumerate().map(|(ix, doc)| {
                        Self::render_doc_table_row(
                            doc,
                            ix,
                            SelectionState::from_eq(&ix, &selected_idx),
                            cx,
                        )
                    })),
            )
            .into_any_element()
    }
}

impl Render for NewTabView {
    #[expect(
        clippy::too_many_lines,
        reason = "Declarative GPUI layout for native master-detail and table hub"
    )]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        let catalog = self.catalog.read(cx);

        let project_folders = catalog.project_folders().to_vec();
        let selected_project_filter = catalog.selected_project_filter().cloned();
        let total_projects = project_folders.len();

        let has_filter = !self.filter_query.is_empty();
        let unfiltered_hub = has_filter.then(|| catalog.filter_hub(""));
        let filtered_hub = catalog.filter_hub(&self.filter_query);

        let mut grouped_skills = match self.active_tab {
            HubTab::ProjectSkills => Some(group_skills_by_source(&filtered_hub.project_skills)),
            HubTab::GlobalSkills => Some(group_skills_by_source(&filtered_hub.global_skills)),
            _ => None,
        };

        let active_tab_paths: Vec<PathBuf> = match self.active_tab {
            HubTab::ProjectSkills | HubTab::GlobalSkills => {
                grouped_skills.as_ref().map_or_default(|(ordered, _)| {
                    ordered
                        .iter()
                        .map(|s| s.skill_file.full_path.clone())
                        .collect()
                })
            }
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
            HubTab::GlobalRules => filtered_hub
                .global_instructions
                .iter()
                .map(|i| i.file.full_path.clone())
                .collect(),
        };

        let nav_count = active_tab_paths.len();
        self.selected_item_index = self.selected_item_index.min(nav_count.saturating_sub(1));
        let selected_item_index = self.selected_item_index;

        v_flex()
            .id("new-tab-page")
            .size_full()
            .relative()
            .bg(theme.background)
            .gap_0()
            .overflow_hidden()
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if this.show_project_manager {
                    if event.keystroke.key.as_str() == "escape" {
                        this.show_project_manager = false;
                        cx.notify();
                    }
                    return;
                }
                match event.keystroke.key.as_str() {
                    "down" => {
                        if nav_count > 0 {
                            this.selected_item_index =
                                (this.selected_item_index + 1).min(nav_count - 1);
                            if matches!(
                                this.active_tab,
                                HubTab::ProjectRules | HubTab::GlobalRules | HubTab::ProjectDocs
                            ) {
                                this.table_scroll_handle
                                    .scroll_to_item(this.selected_item_index);
                            }
                            cx.notify();
                        }
                    }
                    "up" => {
                        if this.selected_item_index > 0 {
                            this.selected_item_index -= 1;
                            if matches!(
                                this.active_tab,
                                HubTab::ProjectRules | HubTab::GlobalRules | HubTab::ProjectDocs
                            ) {
                                this.table_scroll_handle
                                    .scroll_to_item(this.selected_item_index);
                            }
                            cx.notify();
                        } else {
                            this.filter_input
                                .update(cx, |input, cx| input.focus(window, cx));
                        }
                    }
                    "enter" => {
                        if let Some(path) = active_tab_paths.get(this.selected_item_index).cloned()
                        {
                            if event.keystroke.modifiers.secondary(cx)
                                || event.keystroke.modifiers.shift
                            {
                                cx.emit(NewTabEvent::OpenFileInNewTab(path));
                            } else {
                                cx.emit(NewTabEvent::OpenFile(path));
                            }
                        }
                    }
                    "escape" if !this.filter_query.is_empty() => {
                        this.filter_query.clear();
                        this.filter_input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            // Unified Top Chrome: Window Sub-Toolbar + Native Underline Category Strip
            .child(
                v_flex()
                    .w_full()
                    .bg(theme.title_bar)
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(42.))
                            .px_4()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.foreground)
                                            .child("Home"),
                                    )
                                    .child(div().h_3p5().w_px().bg(theme.border))
                                    .when(total_projects > 0, |header| {
                                        let (label, is_filtered) =
                                            if let Some(ref filter) = selected_project_filter {
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
                                                            this.show_project_manager =
                                                                !this.show_project_manager;
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
                                                            .on_click(cx.listener(
                                                                |_this, _, _, cx| {
                                                                    cx.emit(
                                                                        NewTabEvent::SelectProjectFilter(
                                                                            None,
                                                                        ),
                                                                    );
                                                                },
                                                            )),
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
                                    .gap_2()
                                    .child(
                                        div().w(px(220.)).child(
                                            Input::new(&self.filter_input)
                                                .small()
                                                .prefix(
                                                    Icon::new(IconName::Search)
                                                        .xsmall()
                                                        .text_color(theme.muted_foreground),
                                                )
                                                .cleanable(true),
                                        ),
                                    )
                                    .child(div().h_3p5().w_px().bg(theme.border))
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
                    .child(
                        TabBar::new("hub-category-tabs")
                            .underline()
                            .w_full()
                            .px_2p5()
                            .selected_index(self.active_tab.index())
                            .last_empty_space(div())
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                if let Some(tab) = HubTab::from_index(*ix)
                                    && this.active_tab != tab
                                {
                                    this.active_tab = tab;
                                    this.selected_item_index = 0;
                                    cx.notify();
                                }
                            }))
                            .child(Self::build_category_tab(
                                "Project Skills",
                                filtered_hub.project_skills.len(),
                                IconName::Sparkles,
                                SelectionState::from_eq(&self.active_tab, &HubTab::ProjectSkills),
                                cx,
                            ))
                            .child(Self::build_category_tab(
                                "Project Rules",
                                filtered_hub.project_instructions.len(),
                                IconName::Bot,
                                SelectionState::from_eq(&self.active_tab, &HubTab::ProjectRules),
                                cx,
                            ))
                            .child(Self::build_category_tab(
                                "Project Docs",
                                filtered_hub.project_docs.len(),
                                IconName::BookOpen,
                                SelectionState::from_eq(&self.active_tab, &HubTab::ProjectDocs),
                                cx,
                            ))
                            .child(Self::build_category_tab(
                                "Global Skills",
                                filtered_hub.global_skills.len(),
                                IconName::Sparkles,
                                SelectionState::from_eq(&self.active_tab, &HubTab::GlobalSkills),
                                cx,
                            ))
                            .child(Self::build_category_tab(
                                "Global Rules",
                                filtered_hub.global_instructions.len(),
                                IconName::Bot,
                                SelectionState::from_eq(&self.active_tab, &HubTab::GlobalRules),
                                cx,
                            )),
                    ),
            )
            // Edge-to-edge Main Content Area (Master-Detail or Dense Table)
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .bg(theme.background)
                    .overflow_hidden()
                    .when(self.active_tab == HubTab::ProjectSkills, |pane| {
                        let (title, msg) = if total_projects == 0 {
                            (
                                "No Project Folders Chosen",
                                "Click 'Add Project Folder' above to select your workspace or project folders.",
                            )
                        } else {
                            (
                                "No Project Skills Found",
                                "Add skills to .agents/skills/ in your project to see them here.",
                            )
                        };
                        let (ordered, groups) = grouped_skills.take().unwrap_or_default();
                        let empty_kind = if has_filter
                            && unfiltered_hub
                                .as_ref()
                                .is_some_and(|h| !h.project_skills.is_empty())
                        {
                            EmptyStateKind::NoFilterMatches
                        } else {
                            EmptyStateKind::GenuinelyEmpty
                        };
                        pane.child(self.render_skills_master_detail(
                            "project-skills-split",
                            &ordered,
                            groups,
                            empty_kind,
                            title,
                            msg,
                            cx,
                        ))
                    })
                    .when(self.active_tab == HubTab::ProjectRules, |pane| {
                        let empty_kind = if has_filter
                            && unfiltered_hub
                                .as_ref()
                                .is_some_and(|h| !h.project_instructions.is_empty())
                        {
                            EmptyStateKind::NoFilterMatches
                        } else {
                            EmptyStateKind::GenuinelyEmpty
                        };
                        pane.child(Self::render_rules_table(
                            &filtered_hub.project_instructions,
                            false,
                            total_projects,
                            empty_kind,
                            selected_item_index,
                            &self.table_scroll_handle,
                            cx,
                        ))
                    })
                    .when(self.active_tab == HubTab::GlobalSkills, |pane| {
                        let (ordered, groups) = grouped_skills.take().unwrap_or_default();
                        let empty_kind = if has_filter
                            && unfiltered_hub
                                .as_ref()
                                .is_some_and(|h| !h.global_skills.is_empty())
                        {
                            EmptyStateKind::NoFilterMatches
                        } else {
                            EmptyStateKind::GenuinelyEmpty
                        };
                        pane.child(self.render_skills_master_detail(
                            "global-skills-split",
                            &ordered,
                            groups,
                            empty_kind,
                            "No Global Skills Detected",
                            "Standard paths like ~/.agents/skills or ~/.gemini/config/skills were not found.",
                            cx,
                        ))
                    })
                    .when(self.active_tab == HubTab::GlobalRules, |pane| {
                        let empty_kind = if has_filter
                            && unfiltered_hub
                                .as_ref()
                                .is_some_and(|h| !h.global_instructions.is_empty())
                        {
                            EmptyStateKind::NoFilterMatches
                        } else {
                            EmptyStateKind::GenuinelyEmpty
                        };
                        pane.child(Self::render_rules_table(
                            &filtered_hub.global_instructions,
                            true,
                            total_projects,
                            empty_kind,
                            selected_item_index,
                            &self.table_scroll_handle,
                            cx,
                        ))
                    })
                    .when(self.active_tab == HubTab::ProjectDocs, |pane| {
                        let empty_kind = if has_filter
                            && unfiltered_hub
                                .as_ref()
                                .is_some_and(|h| !h.project_docs.is_empty())
                        {
                            EmptyStateKind::NoFilterMatches
                        } else {
                            EmptyStateKind::GenuinelyEmpty
                        };
                        pane.child(Self::render_project_docs_table(
                            &filtered_hub.project_docs,
                            total_projects,
                            empty_kind,
                            selected_item_index,
                            &self.table_scroll_handle,
                            cx,
                        ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ScannedMarkdownFile;

    fn sample_skill(name: &str, source: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_string(),
            description: String::new(),
            dir_path: PathBuf::from(format!("/tmp/{name}")),
            skill_file: ScannedMarkdownFile {
                relative_path: "SKILL.md".to_string(),
                full_path: PathBuf::from(format!("/tmp/{name}/SKILL.md")),
                file_name: "SKILL.md".to_string(),
                file_size: 128,
                modified: None,
            },
            companion_files: Vec::new(),
            agent_source: source.to_string(),
            is_global: true,
            project_name: None,
            project_path: None,
        }
    }

    #[test]
    fn hub_tab_index_round_trips() {
        for (expected_ix, tab) in HUB_TABS.iter().enumerate() {
            assert_eq!(tab.index(), expected_ix);
            assert_eq!(HubTab::from_index(expected_ix), Some(*tab));
        }
        assert_eq!(HubTab::from_index(HUB_TABS.len()), None);
    }

    #[test]
    fn skill_grouping_aligns_indices_across_sources() {
        let skills = vec![
            sample_skill("alpha", "OpenAI Codex"),
            sample_skill("beta", "Claude Code"),
            sample_skill("gamma", "OpenAI Codex"),
        ];
        let (ordered, groups) = group_skills_by_source(&skills);
        assert_eq!(ordered.len(), 3);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].source, "Claude Code");
        assert_eq!(groups[0].items[0].0, 0);
        assert_eq!(groups[0].items[0].1.name, "beta");
        assert_eq!(groups[1].source, "OpenAI Codex");
        assert_eq!(groups[1].items[0].0, 1);
        assert_eq!(groups[1].items[0].1.name, "alpha");
        assert_eq!(groups[1].items[1].0, 2);
        assert_eq!(groups[1].items[1].1.name, "gamma");
        assert_eq!(ordered[1].name, "alpha");
    }

    #[gpui_kit::test]
    fn new_tab_view_renders_and_switches_categories(cx: &mut gpui_kit::TestAppContext) {
        let (view, visual_cx) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            let catalog = cx.new(|_| FolderCatalog::new(Vec::new(), None));
            NewTabView::new(catalog, window, cx)
        });
        visual_cx.run_until_parked();

        view.update(visual_cx, |this, cx| {
            assert_eq!(this.active_tab, HubTab::GlobalSkills);
            this.active_tab = HubTab::GlobalRules;
            cx.notify();
        });
        visual_cx.run_until_parked();

        view.read_with(visual_cx, |this, _| {
            assert_eq!(this.active_tab, HubTab::GlobalRules);
        });
    }

    type RecordedEvents = std::rc::Rc<std::cell::RefCell<Vec<NewTabEvent>>>;

    fn open_test_hub(
        cx: &mut gpui_kit::TestAppContext,
        initial_tab: HubTab,
    ) -> (
        Entity<NewTabView>,
        &mut gpui_kit::VisualTestContext,
        RecordedEvents,
    ) {
        use std::{cell::RefCell, rc::Rc};

        let events: RecordedEvents = Rc::new(RefCell::new(Vec::new()));
        let events_sub = Rc::clone(&events);

        let (view, visual_cx) = cx.add_window_view(|window, cx| {
            gpui_kit::init(cx);
            let mut cat = FolderCatalog::new(Vec::new(), None);
            let proj = PathBuf::from("/tmp/downmark-test-proj");
            cat.project_folders = vec![proj.clone()];

            let mut skill_a = sample_skill("alpha", "Claude Code");
            skill_a.is_global = false;
            skill_a.companion_files.push(ScannedMarkdownFile {
                relative_path: "references/guide.md".to_string(),
                full_path: PathBuf::from("/tmp/alpha/references/guide.md"),
                file_name: "guide.md".to_string(),
                file_size: 64,
                modified: None,
            });
            let mut skill_b = sample_skill("beta", "Claude Code");
            skill_b.is_global = false;
            cat.project_skills = vec![skill_a, skill_b];

            cat.project_instructions = vec![InstructionEntry {
                name: "AGENTS.md".to_string(),
                agent_source: "TestProj".to_string(),
                file: ScannedMarkdownFile {
                    relative_path: "AGENTS.md".to_string(),
                    full_path: PathBuf::from("/tmp/downmark-test-proj/AGENTS.md"),
                    file_name: "AGENTS.md".to_string(),
                    file_size: 42,
                    modified: None,
                },
                is_global: false,
                project_name: Some("TestProj".to_string()),
                project_path: Some(proj.clone()),
            }];

            cat.project_docs = vec![ProjectDocEntry {
                file: ScannedMarkdownFile {
                    relative_path: "README.md".to_string(),
                    full_path: PathBuf::from("/tmp/downmark-test-proj/README.md"),
                    file_name: "README.md".to_string(),
                    file_size: 99,
                    modified: None,
                },
                project_name: Some("TestProj".to_string()),
                project_path: Some(proj),
            }];

            let catalog = cx.new(|_| cat);
            let mut view = NewTabView::new(catalog, window, cx);
            view.active_tab = initial_tab;
            view
        });
        visual_cx.update(|_, cx| {
            cx.subscribe(&view, move |_, event: &NewTabEvent, _| {
                events_sub.borrow_mut().push(event.clone());
            })
            .detach();
        });
        visual_cx.run_until_parked();
        (view, visual_cx, events)
    }

    #[gpui_kit::test]
    fn skill_sidebar_item_requires_full_click(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::Modifiers;

        let (view, visual_cx, events) = open_test_hub(cx, HubTab::ProjectSkills);
        let skill0 = visual_cx
            .debug_bounds("skill-item-0")
            .expect("skill-item-0 bounds");
        let skill1 = visual_cx
            .debug_bounds("skill-item-1")
            .expect("skill-item-1 bounds");

        visual_cx.simulate_mouse_move(skill1.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(skill1.center(), MouseButton::Left, Modifiers::default());
        view.read_with(visual_cx, |this, _| {
            assert_eq!(
                this.selected_item_index, 0,
                "pressing mouse down on skill-item-1 must not select it before mouse up"
            );
        });
        visual_cx.simulate_mouse_move(
            skill0.center(),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(skill0.center(), MouseButton::Left, Modifiers::default());
        view.read_with(visual_cx, |this, _| {
            assert_eq!(this.selected_item_index, 0);
        });

        visual_cx.simulate_mouse_move(skill1.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(skill1.center(), MouseButton::Left, Modifiers::default());
        visual_cx.simulate_mouse_up(skill1.center(), MouseButton::Left, Modifiers::default());
        view.read_with(visual_cx, |this, _| {
            assert_eq!(this.selected_item_index, 1, "full click selects skill 1");
        });

        // Middle-click must require matching mousedown + mouseup on the same item.
        visual_cx.simulate_mouse_move(skill1.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(skill1.center(), MouseButton::Middle, Modifiers::default());
        visual_cx.simulate_mouse_move(
            skill0.center(),
            Some(MouseButton::Middle),
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(skill0.center(), MouseButton::Middle, Modifiers::default());
        assert!(events.borrow().is_empty());

        visual_cx.simulate_mouse_move(skill0.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(skill0.center(), MouseButton::Middle, Modifiers::default());
        assert!(events.borrow().is_empty());
        visual_cx.simulate_mouse_up(skill0.center(), MouseButton::Middle, Modifiers::default());
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFileInNewTab(PathBuf::from(
                "/tmp/alpha/SKILL.md"
            ))]
        );
    }

    #[gpui_kit::test]
    fn skill_detail_rows_and_button_require_full_click(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::Modifiers;

        let (_view, visual_cx, events) = open_test_hub(cx, HubTab::ProjectSkills);
        let entrypoint = visual_cx
            .debug_bounds("skill-entrypoint-row")
            .expect("skill-entrypoint-row bounds");
        let comp0 = visual_cx
            .debug_bounds("comp-row-0")
            .expect("comp-row-0 bounds");
        let open_skill_btn = visual_cx
            .debug_bounds("open-skill-md-btn")
            .expect("open-skill-md-btn bounds");

        visual_cx.simulate_mouse_move(entrypoint.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(entrypoint.center(), MouseButton::Left, Modifiers::default());
        assert!(events.borrow().is_empty());
        visual_cx.simulate_mouse_move(
            comp0.center(),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(comp0.center(), MouseButton::Left, Modifiers::default());
        assert!(events.borrow().is_empty());

        visual_cx.simulate_mouse_move(entrypoint.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(entrypoint.center(), MouseButton::Left, Modifiers::default());
        visual_cx.simulate_mouse_up(entrypoint.center(), MouseButton::Left, Modifiers::default());
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFile(PathBuf::from("/tmp/alpha/SKILL.md"))]
        );
        events.borrow_mut().clear();

        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        visual_cx.simulate_mouse_move(comp0.center(), None, ctrl);
        visual_cx.simulate_mouse_down(comp0.center(), MouseButton::Left, ctrl);
        assert!(events.borrow().is_empty());
        visual_cx.simulate_mouse_up(comp0.center(), MouseButton::Left, ctrl);
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFileInNewTab(PathBuf::from(
                "/tmp/alpha/references/guide.md"
            ))]
        );
        events.borrow_mut().clear();

        // Open SKILL.md button middle-click requires matching mousedown + mouseup on the button.
        visual_cx.simulate_mouse_move(entrypoint.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(
            entrypoint.center(),
            MouseButton::Middle,
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_move(
            open_skill_btn.center(),
            Some(MouseButton::Middle),
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(
            open_skill_btn.center(),
            MouseButton::Middle,
            Modifiers::default(),
        );
        assert!(events.borrow().is_empty());
        visual_cx.simulate_mouse_down(
            open_skill_btn.center(),
            MouseButton::Middle,
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(
            open_skill_btn.center(),
            MouseButton::Middle,
            Modifiers::default(),
        );
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFileInNewTab(PathBuf::from(
                "/tmp/alpha/SKILL.md"
            ))]
        );
    }

    #[gpui_kit::test]
    fn rules_and_docs_rows_and_buttons_require_full_click(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{Modifiers, point, px};

        let (view, visual_cx, events) = open_test_hub(cx, HubTab::ProjectRules);
        let rule_row = visual_cx
            .debug_bounds("rule-row-0")
            .expect("rule-row-0 bounds");
        let open_rule_btn = visual_cx
            .debug_bounds("open-rule-btn-0")
            .expect("open-rule-btn-0 bounds");
        let rule_row_left = point(rule_row.left() + px(24.), rule_row.center().y);

        visual_cx.simulate_mouse_move(rule_row_left, None, Modifiers::default());
        visual_cx.simulate_mouse_down(rule_row_left, MouseButton::Left, Modifiers::default());
        assert!(events.borrow().is_empty());
        visual_cx.simulate_mouse_up(rule_row_left, MouseButton::Left, Modifiers::default());
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFile(PathBuf::from(
                "/tmp/downmark-test-proj/AGENTS.md"
            ))]
        );
        events.borrow_mut().clear();

        // Clicking nested Open button emits OpenFile once without triggering parent row.
        visual_cx.simulate_mouse_move(open_rule_btn.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(
            open_rule_btn.center(),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(
            open_rule_btn.center(),
            MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFile(PathBuf::from(
                "/tmp/downmark-test-proj/AGENTS.md"
            ))]
        );
        events.borrow_mut().clear();

        visual_cx.simulate_mouse_down(
            open_rule_btn.center(),
            MouseButton::Middle,
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(
            open_rule_btn.center(),
            MouseButton::Middle,
            Modifiers::default(),
        );
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFileInNewTab(PathBuf::from(
                "/tmp/downmark-test-proj/AGENTS.md"
            ))]
        );
        events.borrow_mut().clear();

        // Docs row and nested Open button.
        view.update(visual_cx, |this, cx| {
            this.active_tab = HubTab::ProjectDocs;
            cx.notify();
        });
        visual_cx.run_until_parked();

        let doc_row = visual_cx
            .debug_bounds("doc-row-0")
            .expect("doc-row-0 bounds");
        let open_doc_btn = visual_cx
            .debug_bounds("open-doc-btn-0")
            .expect("open-doc-btn-0 bounds");
        let doc_row_left = point(doc_row.left() + px(24.), doc_row.center().y);

        visual_cx.simulate_mouse_move(doc_row_left, None, Modifiers::default());
        visual_cx.simulate_mouse_down(doc_row_left, MouseButton::Left, Modifiers::default());
        assert!(events.borrow().is_empty());
        visual_cx.simulate_mouse_up(doc_row_left, MouseButton::Left, Modifiers::default());
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFile(PathBuf::from(
                "/tmp/downmark-test-proj/README.md"
            ))]
        );
        events.borrow_mut().clear();

        visual_cx.simulate_mouse_move(open_doc_btn.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(
            open_doc_btn.center(),
            MouseButton::Left,
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(
            open_doc_btn.center(),
            MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::OpenFile(PathBuf::from(
                "/tmp/downmark-test-proj/README.md"
            ))]
        );
    }

    #[gpui_kit::test]
    fn workspace_popover_items_require_full_click_and_backdrop_dismisses_on_press(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{Modifiers, point, px};

        let (view, visual_cx, events) = open_test_hub(cx, HubTab::ProjectSkills);
        view.update(visual_cx, |this, cx| {
            this.show_project_manager = true;
            cx.notify();
        });
        visual_cx.run_until_parked();

        let filter_all = visual_cx
            .debug_bounds("popover-filter-all")
            .expect("popover-filter-all bounds");
        let popover_item = visual_cx
            .debug_bounds("popover-item-downmark-test-proj")
            .expect("popover-item bounds");
        let popover_rm = visual_cx
            .debug_bounds("popover-rm-downmark-test-proj")
            .expect("popover-rm bounds");
        let popover_item_left = point(popover_item.left() + px(24.), popover_item.center().y);

        // Press down on remove button does not remove; dragging onto row cancels both.
        visual_cx.simulate_mouse_move(popover_rm.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(popover_rm.center(), MouseButton::Left, Modifiers::default());
        assert!(events.borrow().is_empty());
        visual_cx.simulate_mouse_move(
            popover_item_left,
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        visual_cx.simulate_mouse_up(popover_item_left, MouseButton::Left, Modifiers::default());
        assert!(events.borrow().is_empty());

        // Full click on remove button emits only RemoveProjectFolder.
        visual_cx.simulate_mouse_move(popover_rm.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(popover_rm.center(), MouseButton::Left, Modifiers::default());
        visual_cx.simulate_mouse_up(popover_rm.center(), MouseButton::Left, Modifiers::default());
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::RemoveProjectFolder(PathBuf::from(
                "/tmp/downmark-test-proj"
            ))]
        );
        events.borrow_mut().clear();

        // Popover item waits for mouseup.
        visual_cx.simulate_mouse_move(popover_item_left, None, Modifiers::default());
        visual_cx.simulate_mouse_down(popover_item_left, MouseButton::Left, Modifiers::default());
        view.read_with(visual_cx, |this, _| {
            assert!(this.show_project_manager);
        });
        visual_cx.simulate_mouse_up(popover_item_left, MouseButton::Left, Modifiers::default());
        view.read_with(visual_cx, |this, _| {
            assert!(!this.show_project_manager);
        });
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::SelectProjectFilter(Some(PathBuf::from(
                "/tmp/downmark-test-proj"
            )))]
        );
        events.borrow_mut().clear();

        // Filter-all item waits for mouseup; backdrop dismisses on press.
        view.update(visual_cx, |this, cx| {
            this.show_project_manager = true;
            cx.notify();
        });
        visual_cx.run_until_parked();

        visual_cx.simulate_mouse_move(filter_all.center(), None, Modifiers::default());
        visual_cx.simulate_mouse_down(filter_all.center(), MouseButton::Left, Modifiers::default());
        view.read_with(visual_cx, |this, _| {
            assert!(this.show_project_manager);
        });
        visual_cx.simulate_mouse_up(filter_all.center(), MouseButton::Left, Modifiers::default());
        view.read_with(visual_cx, |this, _| {
            assert!(!this.show_project_manager);
        });
        assert_eq!(
            events.borrow().as_slice(),
            &[NewTabEvent::SelectProjectFilter(None)]
        );

        view.update(visual_cx, |this, cx| {
            this.show_project_manager = true;
            cx.notify();
        });
        visual_cx.run_until_parked();
        let backdrop = visual_cx
            .debug_bounds("project-popover-backdrop")
            .expect("backdrop bounds");
        let outside_card = point(backdrop.right() - px(20.), backdrop.bottom() - px(20.));
        visual_cx.simulate_mouse_move(outside_card, None, Modifiers::default());
        visual_cx.simulate_mouse_down(outside_card, MouseButton::Left, Modifiers::default());
        view.read_with(visual_cx, |this, _| {
            assert!(!this.show_project_manager);
        });
    }

    #[gpui_kit::test]
    fn open_button_modifier_click_emits_single_open_in_new_tab_event(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::Modifiers;

        let (_view, visual_cx, events) = open_test_hub(cx, HubTab::ProjectDocs);
        let open_doc_btn = visual_cx
            .debug_bounds("open-doc-btn-0")
            .expect("open-doc-btn-0 bounds");

        for modifiers in [Modifiers::control(), Modifiers::command()] {
            events.borrow_mut().clear();
            visual_cx.simulate_mouse_move(open_doc_btn.center(), None, modifiers);
            visual_cx.simulate_mouse_down(open_doc_btn.center(), MouseButton::Left, modifiers);
            visual_cx.simulate_mouse_up(open_doc_btn.center(), MouseButton::Left, modifiers);
            assert_eq!(
                events.borrow().as_slice(),
                &[NewTabEvent::OpenFileInNewTab(PathBuf::from(
                    "/tmp/downmark-test-proj/README.md"
                ))]
            );
        }
    }
}
