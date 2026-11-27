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
            show_project_manager: false,
            _subscriptions: subscriptions,
        }
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
        let home_dir = std::env::var("HOME").unwrap_or_default();

        div()
            .id("project-popover-container")
            .absolute()
            .inset_0()
            .occlude()
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
                                    .rounded(theme.radius)
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
    fn render_skills_master_detail(
        &self,
        id: &'static str,
        skills: &[SkillEntry],
        empty_title: &'static str,
        empty_msg: &'static str,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        if skills.is_empty() {
            return Self::render_empty_state(IconName::Sparkles, empty_title, empty_msg, cx);
        }

        let (ordered, groups) = group_skills_by_source(skills);
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
        let dir_display = skill
            .dir_path
            .to_string_lossy()
            .replace(&std::env::var("HOME").unwrap_or_default(), "~");
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
                                let p = full_path;
                                let p_middle = p.clone();
                                let size = skill.skill_file.formatted_size();
                                let mod_str = skill.skill_file.formatted_modified();
                                h_flex()
                                    .id("skill-entrypoint-row")
                                    .role(gpui_kit::Role::ListItem)
                                    .items_center()
                                    .h_9()
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
                                let p_middle = p.clone();
                                let rel = comp.relative_path.clone();
                                let name = comp.file_name.clone();
                                let size = comp.formatted_size();
                                let mod_str = comp.formatted_modified();
                                h_flex()
                                    .id(SharedString::from(format!("comp-row-{ix}")))
                                    .role(gpui_kit::Role::ListItem)
                                    .items_center()
                                    .h_9()
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
                    "Click 'Add Project Folder' above to select your workspace or project folders.",
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
            .h_9()
            .px_4()
            .border_b_1()
            .border_color(theme.border.opacity(0.4))
            .cursor_pointer()
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
            .px_4()
            .border_b_1()
            .border_color(theme.border.opacity(0.4))
            .cursor_pointer()
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

    /// Renders the Project Documentation table view.
    fn render_project_docs_table(
        docs: &[ProjectDocEntry],
        total_projects: usize,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        if docs.is_empty() {
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
                v_flex().flex_1().overflow_y_scrollbar().children(
                    docs.iter()
                        .enumerate()
                        .map(|(ix, doc)| Self::render_doc_table_row(doc, ix, cx)),
                ),
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

        let filtered_hub = catalog.filter_hub(&self.filter_query);

        let active_tab_paths: Vec<PathBuf> = match self.active_tab {
            HubTab::ProjectSkills => group_skills_by_source(&filtered_hub.project_skills)
                .0
                .into_iter()
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
            HubTab::GlobalSkills => group_skills_by_source(&filtered_hub.global_skills)
                .0
                .into_iter()
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
            .gap_0()
            .overflow_hidden()
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "down" => {
                        if nav_count > 0 {
                            this.selected_item_index =
                                (this.selected_item_index + 1).min(nav_count - 1);
                            cx.notify();
                        }
                    }
                    "up" => {
                        if this.selected_item_index > 0 {
                            this.selected_item_index -= 1;
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
                        pane.child(self.render_skills_master_detail(
                            "project-skills-split",
                            &filtered_hub.project_skills,
                            title,
                            msg,
                            cx,
                        ))
                    })
                    .when(self.active_tab == HubTab::ProjectRules, |pane| {
                        pane.child(Self::render_rules_table(
                            &filtered_hub.project_instructions,
                            false,
                            total_projects,
                            cx,
                        ))
                    })
                    .when(self.active_tab == HubTab::GlobalSkills, |pane| {
                        pane.child(self.render_skills_master_detail(
                            "global-skills-split",
                            &filtered_hub.global_skills,
                            "No Global Skills Detected",
                            "Standard paths like ~/.agents/skills or ~/.gemini/config/skills were not found.",
                            cx,
                        ))
                    })
                    .when(self.active_tab == HubTab::GlobalRules, |pane| {
                        pane.child(Self::render_rules_table(
                            &filtered_hub.global_instructions,
                            true,
                            total_projects,
                            cx,
                        ))
                    })
                    .when(self.active_tab == HubTab::ProjectDocs, |pane| {
                        pane.child(Self::render_project_docs_table(
                            &filtered_hub.project_docs,
                            total_projects,
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
}
