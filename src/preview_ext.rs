//! Preview extensions beyond `CommonMark` and GFM: GitHub alerts, math and
//! Mermaid diagrams.

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash as _, Hasher as _},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use gpui_kit::assets::IconName;
use gpui_kit::{
    App, FontWeight, Global, Hsla, Image, ImageFormat, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, RenderImage, Rgba, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, SvgRenderer, WeakEntity,
    Window,
    base::{TextView, TextViewState},
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        text::{
            InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode,
            MarkdownParseContext, MarkdownPlugin, markdown_ast::Node,
        },
        v_flex,
    },
    div, img,
    prelude::FluentBuilder as _,
    px,
};

use crate::{
    actions::{ViewDiagram, ViewTable},
    analysis::{content_hash, plain_text},
    diagram::{self, DiagramTheme},
    math::{self, MathStyle},
};

/// The parser configuration and plugins the preview renders with.
pub(crate) fn markdown_extensions(
    preview: &gpui_kit::Entity<TextViewState>,
    base_dir: Arc<Mutex<Option<PathBuf>>>,
) -> MarkdownExtensions {
    let cache = MathCache::new(preview);
    let diagrams = DiagramCache::new(preview);
    let with_math = |extensions: MarkdownExtensions| {
        extensions
            .plugin(BlockMathPlugin {
                cache: cache.clone(),
            })
            .plugin(InlineMathPlugin {
                cache: cache.clone(),
            })
            .plugin(MermaidPlugin {
                cache: diagrams.clone(),
            })
    };
    let table = TableBlockPlugin {
        base_dir,
        body_extensions: with_math(
            MarkdownExtensions::default()
                .frontmatter()
                .plugin(gpui_kit::component::text::FrontmatterPlugin::new()),
        ),
    };
    with_math(
        MarkdownExtensions::default()
            .frontmatter()
            .plugin(gpui_kit::component::text::FrontmatterPlugin::new())
            .block_surface("note", alert_surface(AlertKind::Note))
            .block_surface("tip", alert_surface(AlertKind::Tip))
            .block_surface("important", alert_surface(AlertKind::Important))
            .block_surface("warning", alert_surface(AlertKind::Warning))
            .block_surface("caution", alert_surface(AlertKind::Caution))
            .plugin(table),
    )
}

/// Extensions for the expanded table overlay dialog (without `TableBlockPlugin` to avoid nested buttons).
pub(crate) fn table_overlay_extensions(
    preview: &gpui_kit::Entity<TextViewState>,
) -> MarkdownExtensions {
    let cache = MathCache::new(preview);
    let diagrams = DiagramCache::new(preview);
    let with_math = |extensions: MarkdownExtensions| {
        extensions
            .plugin(BlockMathPlugin {
                cache: cache.clone(),
            })
            .plugin(InlineMathPlugin {
                cache: cache.clone(),
            })
            .plugin(MermaidPlugin {
                cache: diagrams.clone(),
            })
    };
    with_math(
        MarkdownExtensions::default()
            .frontmatter()
            .plugin(gpui_kit::component::text::FrontmatterPlugin::new())
            .block_surface("note", alert_surface(AlertKind::Note))
            .block_surface("tip", alert_surface(AlertKind::Tip))
            .block_surface("important", alert_surface(AlertKind::Important))
            .block_surface("warning", alert_surface(AlertKind::Warning))
            .block_surface("caution", alert_surface(AlertKind::Caution)),
    )
}

/// The five GitHub alert types, e.g. `> [!NOTE]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl AlertKind {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Tip => "Tip",
            Self::Important => "Important",
            Self::Warning => "Warning",
            Self::Caution => "Caution",
        }
    }

    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::Note => IconName::Info,
            Self::Tip => IconName::Lightbulb,
            Self::Important => IconName::MessageSquareWarning,
            Self::Warning => IconName::TriangleAlert,
            Self::Caution => IconName::OctagonAlert,
        }
    }

    fn color(self, cx: &App) -> Hsla {
        let theme = cx.theme();
        match self {
            Self::Note => theme.info,
            Self::Tip => theme.success,
            Self::Important => theme.magenta,
            Self::Warning => theme.warning,
            Self::Caution => theme.danger,
        }
    }
}

fn alert_surface(
    kind: AlertKind,
) -> impl Fn(&mut Window, &mut App) -> gpui_kit::Div + Send + Sync + 'static {
    move |_, cx| {
        let color = kind.color(cx);

        v_flex()
            .w_full()
            .gap_1()
            .px_4()
            .py_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(color.opacity(0.35))
            .bg(color.opacity(0.06))
            .child(
                h_flex()
                    .id(SharedString::from(format!("alert-title-{}", kind.title())))
                    .role(gpui_kit::Role::Note)
                    .aria_label(kind.title())
                    .gap_2()
                    .text_color(color)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(Icon::new(kind.icon()).small())
                    .child(kind.title()),
            )
    }
}

#[derive(Clone)]
pub(crate) struct ActiveTableScrollHandle(pub(crate) gpui_kit::ScrollHandle);
impl gpui_kit::Global for ActiveTableScrollHandle {}

/// Calculates the estimated display width in pixels for a Markdown table
/// so it can be rendered at full width in horizontal scroll containers.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "Table width approximation from character count"
)]
pub(crate) fn table_content_width(markdown: &str) -> f32 {
    let mut col_max_chars: Vec<usize> = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        // Skip delimiter row like |---|:---|---:|
        let is_sep = trimmed
            .chars()
            .all(|c| c == '|' || c == '-' || c == ':' || c.is_whitespace());
        if is_sep {
            continue;
        }
        let cells: Vec<&str> = trimmed.trim_matches('|').split('|').collect();
        if cells.len() > col_max_chars.len() {
            col_max_chars.resize(cells.len(), 0);
        }
        for (i, cell) in cells.iter().enumerate() {
            let trimmed_cell = cell.trim();
            let cell_width = trimmed_cell
                .split("<br>")
                .map(|segment| unicode_width::UnicodeWidthStr::width(segment.trim()))
                .max()
                .unwrap_or(0);
            col_max_chars[i] = col_max_chars[i].max(cell_width);
        }
    }
    if col_max_chars.is_empty() {
        return 0.0;
    }
    let total: f32 = col_max_chars
        .iter()
        .map(|&chars| {
            let text_px = ((chars as f32) * 9.5).clamp(60.0, 600.0);
            text_px + 33.0
        })
        .sum::<f32>()
        + 2.0;
    total
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableBlockData {
    markdown: String,
}

struct TableBlockPlugin {
    base_dir: Arc<Mutex<Option<PathBuf>>>,
    body_extensions: MarkdownExtensions,
}

impl MarkdownPlugin for TableBlockPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "table-block"
    }

    fn parse(&self, node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let Node::Table(table) = node else {
            return None;
        };
        let source = cx.node_source(node)?;
        let text = plain_text(&table.children);
        Some(
            MarkdownNode::new(
                "table-block",
                TableBlockData {
                    markdown: source.to_string(),
                },
            )
            .text(text)
            .markdown(source),
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Declarative layout for table scroller and controls"
    )]
    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let data = node
            .data::<TableBlockData>()
            .expect("table block node data");
        let markdown = SharedString::from(data.markdown.clone());
        let hash = content_hash(&data.markdown);
        let id = SharedString::from(format!("table-{hash}"));
        let markdown_expand = markdown.clone();
        let markdown_copy = markdown;

        let table_width = table_content_width(&data.markdown);

        let scroll_handle = window
            .use_keyed_state(format!("{id}-scroller"), cx, |_, _| {
                gpui_kit::ScrollHandle::new()
            })
            .read(cx)
            .clone();

        let table_focus = window
            .use_keyed_state(format!("{id}-focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();

        if cx.try_global::<ActiveTableScrollHandle>().is_none() {
            cx.set_global(ActiveTableScrollHandle(scroll_handle.clone()));
        }

        let scroll_handle_for_keys = scroll_handle.clone();
        let scroll_handle_for_wheel = scroll_handle.clone();
        let scroll_handle_for_scrollbar = scroll_handle.clone();
        let scroll_handle_for_active = scroll_handle.clone();
        let scroll_handle_for_click = scroll_handle.clone();
        let focus_for_click = table_focus.clone();

        let table_view_style =
            crate::themes::preview_style(cx).with_table(gpui_kit::StyleRefinement::default());

        let base_dir = self.base_dir.lock().ok().and_then(|g| g.clone());
        let base_dir_for_img = base_dir.clone();
        let base_dir_for_links = base_dir;

        v_flex()
            .id(id.clone())
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .justify_end()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("{id}-expand")))
                            .accessibility_label("Expand table")
                            .xsmall()
                            .ghost()
                            .icon(Icon::new(IconName::Maximize))
                            .label("Expand table")
                            .tooltip("View full table across window width (Esc to close)")
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(ViewTable(markdown_expand.clone())),
                                    cx,
                                );
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{id}-copy")))
                            .accessibility_label("Copy table Markdown")
                            .xsmall()
                            .ghost()
                            .icon(Icon::new(IconName::Copy))
                            .label("Copy table")
                            .tooltip("Copy table Markdown")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    markdown_copy.to_string(),
                                ));
                            }),
                    ),
            )
            .child(
                div()
                    .id(format!("{id}-wrapper"))
                    .w_full()
                    .relative()
                    .track_focus(&table_focus)
                    .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                        window.focus(&focus_for_click, cx);
                        cx.set_global(ActiveTableScrollHandle(scroll_handle_for_click.clone()));
                    })
                    .on_mouse_move(move |_, _, cx| {
                        cx.set_global(ActiveTableScrollHandle(scroll_handle_for_active.clone()));
                    })
                    .capture_key_down(move |event, window, cx| {
                        let key = event.keystroke.key.to_lowercase();
                        let mut offset = scroll_handle_for_keys.offset();
                        let max = scroll_handle_for_keys.max_offset();
                        match key.as_str() {
                            "left" | "arrowleft" | "h" => {
                                offset.x = (offset.x + px(120.)).min(px(0.));
                                scroll_handle_for_keys.set_offset(offset);
                                window.refresh();
                                cx.stop_propagation();
                            }
                            "right" | "arrowright" | "l" => {
                                offset.x = (offset.x - px(120.)).max(-max.x);
                                scroll_handle_for_keys.set_offset(offset);
                                window.refresh();
                                cx.stop_propagation();
                            }
                            "home" => {
                                offset.x = px(0.);
                                scroll_handle_for_keys.set_offset(offset);
                                window.refresh();
                                cx.stop_propagation();
                            }
                            "end" => {
                                offset.x = -max.x;
                                scroll_handle_for_keys.set_offset(offset);
                                window.refresh();
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    })
                    .child(
                        div()
                            .id(format!("{id}-scroll-viewport"))
                            .w_full()
                            .overflow_x_scroll()
                            .restrict_scroll_to_axis()
                            .track_scroll(&scroll_handle)
                            .on_scroll(move |event, window, _cx| {
                                let delta = event.delta.pixel_delta(px(16.));
                                let h_delta = if event.modifiers.shift {
                                    if delta.x == px(0.) { delta.y } else { delta.x }
                                } else {
                                    delta.x
                                };
                                if h_delta != px(0.) {
                                    let mut offset = scroll_handle_for_wheel.offset();
                                    let max = scroll_handle_for_wheel.max_offset();
                                    offset.x = (offset.x + h_delta).clamp(-max.x, px(0.));
                                    scroll_handle_for_wheel.set_offset(offset);
                                    window.refresh();
                                }
                            })
                            .child(
                                div()
                                    .min_w_full()
                                    .when(table_width > 0.0, |this| this.w(px(table_width)))
                                    .child(
                                        TextView::markdown(
                                            SharedString::from(format!("{id}-view")),
                                            data.markdown.clone(),
                                        )
                                        .style(table_view_style)
                                        .markdown_extensions(self.body_extensions.clone())
                                        .selectable(true)
                                        .on_link_click(crate::links::on_click)
                                        .image_source(move |url| {
                                            crate::images::resolve(url, base_dir_for_img.as_deref())
                                        })
                                        .link_tooltip(move |url, window, cx| {
                                            crate::links::build_tooltip(
                                                url,
                                                base_dir_for_links.as_deref(),
                                                window,
                                                cx,
                                            )
                                        })
                                        .w_full(),
                                    ),
                            ),
                    )
                    .child(
                        gpui_kit::base::Scrollbar::horizontal(&scroll_handle_for_scrollbar)
                            .viewport_from_layout(),
                    ),
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Formula {
    tex: String,
    style: MathStyle,
}

fn formula_node(tex: &str, style: MathStyle, source: &str) -> MarkdownNode {
    let name = match style {
        MathStyle::Inline => "inline-math",
        MathStyle::Display => "math",
    };
    MarkdownNode::new(
        name,
        Formula {
            tex: tex.to_string(),
            style,
        },
    )
    .text(tex.trim().to_string())
    .accessibility_label(format!("Formula: {}", tex.trim()))
    .markdown(source.to_string())
}

/// A rendered formula ready to paint.
struct PreparedMath {
    image: Arc<Image>,
    width: f32,
    height: f32,
    baseline: f32,
}

enum MathEntry {
    Pending,
    Ready(Arc<PreparedMath>),
    Failed(SharedString),
}

/// LaTeX source, style, font size (as bits) and RGBA color.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MathKey {
    tex: String,
    style: MathStyle,
    font_size_bits: u32,
    color_rgba: u32,
}

/// Formulas rendered for the preview, keyed by source, style, size and
/// color. Rendering runs on a background thread; the preview is asked to
/// lay out again once a formula is ready.
#[derive(Clone)]
struct MathCache {
    entries: Arc<Mutex<HashMap<MathKey, MathEntry>>>,
    preview: WeakEntity<TextViewState>,
}

/// Computer Modern has a smaller x-height than interface sans-serif fonts;
/// scale formulas so they read at the same size as the text around them.
pub(crate) const INLINE_MATH_SCALE: f32 = 1.15;
/// Display formulas are set a little larger again, as in print.
pub(crate) const DISPLAY_MATH_SCALE: f32 = 1.3;

/// Formulas kept before the cache starts over.
const MATH_CACHE_LIMIT: usize = 512;

enum Lookup {
    Ready(Arc<PreparedMath>),
    Waiting,
    Failed(SharedString),
}

impl MathCache {
    fn new(preview: &gpui_kit::Entity<TextViewState>) -> Self {
        Self {
            entries: Arc::default(),
            preview: preview.downgrade(),
        }
    }

    /// The prepared formula, starting its rendering when it is new.
    fn get(&self, formula: &Formula, font_size: f32, color: Hsla, cx: &mut App) -> Lookup {
        let rgba = Rgba::from(color);
        let key = MathKey {
            tex: formula.tex.clone(),
            style: formula.style,
            font_size_bits: font_size.to_bits(),
            color_rgba: u32::from(rgba),
        };
        let mut entries = self.entries.lock().expect("math cache poisoned");
        match entries.get(&key) {
            Some(MathEntry::Ready(math)) => return Lookup::Ready(Arc::clone(math)),
            Some(MathEntry::Pending) => return Lookup::Waiting,
            Some(MathEntry::Failed(error)) => return Lookup::Failed(error.clone()),
            None => {}
        }
        if entries.len() >= MATH_CACHE_LIMIT {
            entries.retain(|_, entry| matches!(entry, MathEntry::Pending));
        }
        entries.insert(key.clone(), MathEntry::Pending);
        drop(entries);

        let color = format!("#{:08x}", u32::from(rgba));
        let (tex, style) = (formula.tex.clone(), formula.style);
        let task = cx
            .background_executor()
            .spawn(async move { math::render(&tex, style, font_size, &color) });
        let cache = self.clone();
        cx.spawn(async move |cx| {
            let entry = match task.await {
                Ok(svg) => {
                    // Layout snaps sizes to whole pixels; round up so the
                    // formula is never clipped and its baseline, which sits
                    // at the very bottom of formulas without descenders,
                    // stays inside the box the inline layout validates.
                    let height = svg.height.ceil();
                    MathEntry::Ready(Arc::new(PreparedMath {
                        image: Arc::new(Image::from_bytes(ImageFormat::Svg, svg.svg.into_bytes())),
                        width: svg.width.ceil(),
                        height,
                        baseline: svg.baseline.clamp(0., height),
                    }))
                }
                Err(error) => MathEntry::Failed(error.into()),
            };
            cache
                .entries
                .lock()
                .expect("math cache poisoned")
                .insert(key, entry);
            _ = cache
                .preview
                .update(cx, gpui_kit::base::TextViewState::invalidate_inline_layout);
        })
        .detach();
        Lookup::Waiting
    }
}

fn math_image(math: &PreparedMath) -> gpui_kit::Img {
    img(Arc::clone(&math.image))
        .object_fit(ObjectFit::Contain)
        .flex_shrink_0()
        .w(px(math.width))
        .h(px(math.height))
}

/// The formula's source, shown while it renders or when it cannot.
fn math_source(tex: &str, color: Hsla, cx: &App) -> gpui_kit::Div {
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(color)
        .child(tex.trim().to_string())
}

/// Display math: `$$` fences, or a paragraph that is only `$$ … $$`.
struct BlockMathPlugin {
    cache: MathCache,
}

pub(crate) fn display_math_source(source: &str) -> Option<&str> {
    let body = source.trim().strip_prefix("$$")?.strip_suffix("$$")?.trim();
    (!body.is_empty()).then_some(body)
}

impl MarkdownPlugin for BlockMathPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "math"
    }

    fn parse(&self, node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        match node {
            Node::Math(math) => Some(formula_node(
                &math.value,
                MathStyle::Display,
                cx.node_source(node).unwrap_or(&math.value),
            )),
            Node::Paragraph(_) => {
                let source = cx.node_source(node)?;
                let tex = display_math_source(source)?;
                Some(formula_node(tex, MathStyle::Display, source))
            }
            _ => None,
        }
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let formula = node.data::<Formula>().expect("math node data");
        let font_size = f32::from(window.text_style().font_size.to_pixels(window.rem_size()));
        let font_size = (font_size * DISPLAY_MATH_SCALE).round();
        let foreground = cx.theme().foreground;

        let content = match self.cache.get(formula, font_size, foreground, cx) {
            Lookup::Ready(math) => math_image(&math).into_any_element(),
            Lookup::Waiting => {
                math_source(&formula.tex, cx.theme().muted_foreground, cx).into_any_element()
            }
            Lookup::Failed(error) => v_flex()
                .items_center()
                .gap_1()
                .child(math_source(&formula.tex, cx.theme().danger, cx))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(error),
                )
                .into_any_element(),
        };

        div()
            .id(SharedString::from(format!(
                "math-{}",
                content_hash(&formula.tex)
            )))
            .w_full()
            .flex()
            .justify_center()
            .py_1()
            .overflow_x_scroll()
            .child(content)
    }
}

/// Inline math: `$…$`.
struct InlineMathPlugin {
    cache: MathCache,
}

/// Whether an inline `$…$` span is math. Like GitHub, prose such as
/// "costs $5 and $10" is not: the content may not start or end with
/// whitespace, and the closing dollar may not be followed by a digit.
pub(crate) fn is_inline_math(value: &str, following: Option<char>) -> bool {
    !value.is_empty()
        && !value.starts_with(char::is_whitespace)
        && !value.ends_with(char::is_whitespace)
        && !following.is_some_and(|ch| ch.is_ascii_digit())
}

impl MarkdownPlugin for InlineMathPlugin {
    fn name(&self) -> &'static str {
        "inline-math"
    }

    fn parse(&self, node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let Node::InlineMath(math) = node else {
            return None;
        };
        let end = node.position()?.end.offset;
        let following = cx.source().get(end..).and_then(|rest| rest.chars().next());
        if !is_inline_math(&math.value, following) {
            return None;
        }
        let source = cx.node_source(node).unwrap_or(&math.value);
        Some(formula_node(&math.value, MathStyle::Inline, source))
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let formula = node.data::<Formula>()?;
        let font_size = (f32::from(context.font_size()) * INLINE_MATH_SCALE).round();
        let color = context.text_style().color;
        let lookup = self.cache.get(formula, font_size, color, cx);
        match lookup {
            Lookup::Ready(math) => {
                Some(InlineElement::new(math_image(&math)).with_baseline(px(math.baseline)))
            }
            // The preview shows the node's text until the formula is ready.
            Lookup::Waiting | Lookup::Failed(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MermaidSource(String);

/// A rendered diagram ready to paint.
pub(crate) struct PreparedDiagram {
    image: Arc<RenderImage>,
    width: f32,
    height: f32,
}

pub(crate) enum DiagramEntry {
    Pending,
    Ready(Arc<PreparedDiagram>),
    Failed(SharedString),
}

enum DiagramLookup {
    Ready(Arc<PreparedDiagram>),
    Waiting,
    Failed(SharedString),
}

/// Diagrams kept before the cache starts over.
const DIAGRAM_CACHE_LIMIT: usize = 128;
/// Diagrams are laid out for this text size and scaled with the preview's.
const DIAGRAM_FONT_SIZE: f32 = 16.;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DiagramKey {
    pub(crate) source: SharedString,
    pub(crate) theme_hash: u64,
    pub(crate) scale_factor_bits: u32,
}

impl DiagramKey {
    pub(crate) fn new(source: impl Into<SharedString>, theme_hash: u64, scale_factor: f32) -> Self {
        Self {
            source: source.into(),
            theme_hash,
            scale_factor_bits: scale_factor.to_bits(),
        }
    }
}

/// Application-wide cache of rendered diagrams, managed through GPUI's Global context.
#[derive(Clone, Default)]
pub(crate) struct SharedDiagramCache {
    entries: Arc<Mutex<HashMap<DiagramKey, DiagramEntry>>>,
}

impl Global for SharedDiagramCache {}

impl SharedDiagramCache {
    /// Retrieve the application's shared diagram cache, or a default instance if not registered.
    pub(crate) fn global(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    /// Removes cached diagrams that are not referenced in any currently open documents,
    /// returning the evicted rendered images so their window GPU textures can be dropped.
    pub(crate) fn prune_unreferenced(&self, active_contents: &[&str]) -> Vec<Arc<RenderImage>> {
        let mut evicted_images = Vec::new();
        if let Ok(mut entries) = self.entries.lock() {
            if active_contents.is_empty() {
                for (_, entry) in entries.drain() {
                    if let DiagramEntry::Ready(diagram) = entry {
                        evicted_images.push(Arc::clone(&diagram.image));
                    }
                }
            } else {
                entries.retain(|key, entry| {
                    let keep = active_contents
                        .iter()
                        .any(|content| content.contains(key.source.as_ref()));
                    if !keep && let DiagramEntry::Ready(diagram) = entry {
                        evicted_images.push(Arc::clone(&diagram.image));
                    }
                    keep
                });
            }
        }
        evicted_images
    }

    /// Clears all cached diagrams.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Utility for manually invalidating the shared diagram cache"
        )
    )]
    pub(crate) fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_for_test(&self, key: DiagramKey, entry: DiagramEntry) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(key, entry);
        }
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, key: &DiagramKey) -> bool {
        self.entries
            .lock()
            .is_ok_and(|entries| entries.contains_key(key))
    }
}

/// Diagrams rendered for the preview, keyed by source and theme, rendered
/// on a background thread with bounded concurrency.
#[derive(Clone)]
struct DiagramCache {
    preview: WeakEntity<TextViewState>,
}

impl DiagramCache {
    fn new(preview: &gpui_kit::Entity<TextViewState>) -> Self {
        Self {
            preview: preview.downgrade(),
        }
    }

    fn get(
        &self,
        source: &str,
        theme: DiagramTheme,
        scale_factor: f32,
        svg_renderer: SvgRenderer,
        cx: &mut App,
    ) -> DiagramLookup {
        let theme_hash = {
            let mut hasher = DefaultHasher::new();
            theme.hash(&mut hasher);
            hasher.finish()
        };
        let key = DiagramKey::new(source, theme_hash, scale_factor);
        let shared_cache = SharedDiagramCache::global(cx);
        let mut entries = shared_cache.entries.lock().expect("diagram cache poisoned");
        match entries.get(&key) {
            Some(DiagramEntry::Ready(diagram)) => return DiagramLookup::Ready(Arc::clone(diagram)),
            Some(DiagramEntry::Pending) => return DiagramLookup::Waiting,
            Some(DiagramEntry::Failed(error)) => return DiagramLookup::Failed(error.clone()),
            None => {}
        }
        if entries.len() >= DIAGRAM_CACHE_LIMIT {
            entries.retain(|_, entry| matches!(entry, DiagramEntry::Pending));
        }
        entries.insert(key.clone(), DiagramEntry::Pending);
        drop(entries);

        let source_str = source.to_string();
        let id = format!("mermaid-{:x}", content_hash(source));
        let task: gpui_kit::Task<Result<(diagram::Diagram, Arc<RenderImage>), String>> =
            cx.background_executor().spawn(async move {
                let _permit = diagram::RENDER_SEMAPHORE.acquire().await;
                let diagram = diagram::render(&source_str, &theme, &id)?;
                let image = svg_renderer
                    .render_single_frame(diagram.svg.as_bytes(), scale_factor)
                    .map_err(|e| e.to_string())?;
                Ok((diagram, image))
            });
        let preview = self.preview.clone();
        let cache_clone = shared_cache;
        cx.spawn(async move |cx| {
            let entry = match task.await {
                Ok((diagram, image)) => DiagramEntry::Ready(Arc::new(PreparedDiagram {
                    image,
                    width: diagram.width,
                    height: diagram.height,
                })),
                Err(error) => DiagramEntry::Failed(error.into()),
            };
            if let Ok(mut entries) = cache_clone.entries.lock() {
                entries.insert(key, entry);
            }
            _ = preview.update(cx, gpui_kit::base::TextViewState::invalidate_inline_layout);
        })
        .detach();
        DiagramLookup::Waiting
    }
}

/// Fenced code blocks marked `mermaid` render as diagrams.
struct MermaidPlugin {
    cache: DiagramCache,
}

impl MarkdownPlugin for MermaidPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "mermaid"
    }

    fn parse(&self, node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let Node::Code(code) = node else {
            return None;
        };
        if !diagram::is_mermaid(code.lang.as_deref()) {
            return None;
        }
        Some(
            MarkdownNode::new("mermaid", MermaidSource(code.value.clone()))
                .text(code.value.clone())
                .accessibility_label("Mermaid diagram")
                .markdown(cx.node_source(node).unwrap_or(&code.value).to_string()),
        )
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = &node.data::<MermaidSource>().expect("mermaid node data").0;
        let id = SharedString::from(format!("mermaid-{}", content_hash(source)));
        let scale = f32::from(window.rem_size()) / DIAGRAM_FONT_SIZE;
        let theme = crate::themes::diagram_theme(cx);
        let scale_factor = window.scale_factor();
        let svg_renderer = cx.svg_renderer();

        let content = match self
            .cache
            .get(source, theme, scale_factor, svg_renderer, cx)
        {
            DiagramLookup::Ready(diagram) => {
                let width = diagram.width * scale;
                let source = SharedString::from(source.clone());
                let open = move |window: &mut Window, cx: &mut App| {
                    window.dispatch_action(Box::new(ViewDiagram(source.clone())), cx);
                };
                let open_from_button = open.clone();
                h_flex()
                    .w_full()
                    .justify_center()
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-view")))
                            .group("diagram")
                            .relative()
                            .w_full()
                            .max_w(px(width))
                            .on_click(move |event, window, cx| {
                                if !event.is_right_click() {
                                    open(window, cx);
                                }
                            })
                            .child(
                                img(Arc::clone(&diagram.image))
                                    .object_fit(ObjectFit::Contain)
                                    .w_full()
                                    .aspect_ratio(diagram.width / diagram.height),
                            )
                            .child(
                                // Shown on hover, so it's clear a click opens the
                                // diagram larger.
                                div()
                                    .absolute()
                                    .top_1()
                                    .right_1()
                                    .invisible()
                                    .group_hover("diagram", gpui_kit::Styled::visible)
                                    .child(
                                        Button::new(SharedString::from(format!("{id}-expand")))
                                            .accessibility_label(
                                                "Open Mermaid diagram in full viewer",
                                            )
                                            .icon(Icon::new(IconName::Maximize))
                                            .small()
                                            .outline()
                                            .tooltip("Open larger")
                                            .on_click(move |_, window, cx| {
                                                cx.stop_propagation();
                                                open_from_button(window, cx);
                                            }),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
            DiagramLookup::Waiting => div()
                .h(px(48.))
                .w_full()
                .rounded(cx.theme().radius)
                .bg(cx.theme().muted.opacity(0.5))
                .into_any_element(),
            DiagramLookup::Failed(error) => v_flex()
                .w_full()
                .gap_2()
                .p_3()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().danger.opacity(0.4))
                .bg(cx.theme().danger.opacity(0.05))
                .child(
                    h_flex()
                        .items_start()
                        .gap_2()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(Icon::new(IconName::TriangleAlert).small().mt_0p5())
                        .child(div().flex_1().min_w_0().child(error)),
                )
                .child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(source.trim_end().to_string()),
                )
                .into_any_element(),
        };
        div().id(id).w_full().py_2().child(content)
    }
}

fn collect_reference_nodes(
    node: &markdown::mdast::Node,
    source: &str,
    defs: &mut std::collections::HashMap<String, (String, Option<String>)>,
    refs: &mut Vec<(std::ops::Range<usize>, String, String)>,
) {
    use markdown::mdast::Node;

    match node {
        Node::Definition(def) => {
            defs.entry(def.identifier.clone())
                .or_insert_with(|| (def.url.clone(), def.title.clone()));
        }
        Node::LinkReference(link) => {
            if let Some(pos) = link.position.as_ref() {
                let range = pos.start.offset..pos.end.offset;
                let label = if let Some(last) = link.children.last().and_then(|c| c.position()) {
                    let end = (last.end.offset + 1).min(range.end);
                    source.get(range.start..end).unwrap_or("")
                } else if let Some(first_close) = source[range.clone()].find(']') {
                    &source[range.start..=range.start + first_close]
                } else {
                    ""
                };
                if !label.is_empty() {
                    refs.push((range, label.to_string(), link.identifier.clone()));
                }
            }
        }
        Node::ImageReference(img) => {
            if let Some(pos) = img.position.as_ref() {
                let range = pos.start.offset..pos.end.offset;
                let label = if let Some(first_close) = source[range.clone()].find(']') {
                    &source[range.start..=range.start + first_close]
                } else {
                    ""
                };
                if !label.is_empty() {
                    refs.push((range, label.to_string(), img.identifier.clone()));
                }
            }
        }
        _ => {}
    }

    if let Some(children) = node.children() {
        for child in children {
            collect_reference_nodes(child, source, defs, refs);
        }
    }
}

/// Resolves Markdown reference-style links (`[text][ref]`, `[ref][]`, `[ref]`)
/// and image references (`![alt][ref]`) into standard inline links (`[text](url "title")`).
///
/// This avoids upstream parser limitations where reference definition metadata is
/// detached from link targets in preview mode.
pub(crate) fn resolve_reference_links(source: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    use std::collections::HashMap;

    use markdown::mdast::Node;

    if !source.contains("]:") {
        return Cow::Borrowed(source);
    }

    let Ok(Node::Root(root)) =
        markdown::to_mdast(source, &crate::analysis::preview_parse_options())
    else {
        return Cow::Borrowed(source);
    };

    let mut defs: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut refs: Vec<(std::ops::Range<usize>, String, String)> = Vec::new();

    collect_reference_nodes(&Node::Root(root), source, &mut defs, &mut refs);

    if defs.is_empty() || refs.is_empty() {
        return Cow::Borrowed(source);
    }

    refs.sort_by_key(|(range, _, _)| std::cmp::Reverse(range.start));

    let mut out = source.to_string();
    for (range, label, identifier) in refs {
        let normalized = identifier.trim().to_lowercase();
        if let Some((url, title)) = defs
            .iter()
            .find(|(k, _)| k.trim().to_lowercase() == normalized)
            .map(|(_, v)| v)
        {
            let replacement = if let Some(title) = title {
                format!("{label}({url} \"{title}\")")
            } else {
                format!("{label}({url})")
            };
            if range.end <= out.len()
                && out.is_char_boundary(range.start)
                && out.is_char_boundary(range.end)
            {
                out.replace_range(range, &replacement);
            }
        }
    }

    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_link_parse() {
        let md = "- **Reference-style links:**\n  You can read more about [CommonMark][commonmark-ref] and [GFM Specification][gfm-ref].\n  Collapsed: [commonmark-ref][].\n  Shortcut: [commonmark-ref].\n  Image: ![Sample][commonmark-ref].\n\n[commonmark-ref]: https://spec.commonmark.org \"CommonMark Specification\"\n[gfm-ref]: https://github.github.com/gfm/ \"GitHub Flavored Markdown Spec\"";
        let resolved = resolve_reference_links(md);
        assert!(
            resolved
                .contains("[CommonMark](https://spec.commonmark.org \"CommonMark Specification\")")
        );
        assert!(resolved.contains(
            "[GFM Specification](https://github.github.com/gfm/ \"GitHub Flavored Markdown Spec\")"
        ));
        assert!(resolved.contains(
            "[commonmark-ref](https://spec.commonmark.org \"CommonMark Specification\")"
        ));
        assert!(
            resolved
                .contains("![Sample](https://spec.commonmark.org \"CommonMark Specification\")")
        );
    }

    #[gpui_kit::test]
    fn parses_and_renders_github_alerts(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let md = "> [!WARNING]\n> Back up first.\n>\n> - Really.";
        let (window_handle, _) = cx.update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                let entity = cx.new(|cx| TextViewState::markdown(md, cx));
                let extensions = markdown_extensions(&entity, Arc::new(Mutex::new(None)));
                cx.new(|_| {
                    struct AlertTestRoot {
                        text: SharedString,
                        extensions: MarkdownExtensions,
                    }
                    impl gpui_kit::Render for AlertTestRoot {
                        fn render(
                            &mut self,
                            _: &mut Window,
                            _: &mut gpui_kit::Context<'_, Self>,
                        ) -> impl IntoElement {
                            TextView::markdown("alert-test", self.text.clone())
                                .markdown_extensions(self.extensions.clone())
                        }
                    }
                    AlertTestRoot {
                        text: md.into(),
                        extensions,
                    }
                })
            })
            .unwrap()
        });
        cx.update_window(window_handle, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
    }

    #[test]
    fn tells_math_from_prices() {
        assert!(is_inline_math("x^2", Some(' ')));
        assert!(!is_inline_math("5 and ", Some('1')));
        assert!(!is_inline_math(" x", None));
        assert!(!is_inline_math("x", Some('0')));
        assert_eq!(display_math_source("$$ a + b $$"), Some("a + b"));
        assert_eq!(display_math_source("$$ $$"), None);
    }

    #[test]
    fn test_diagram_prune_unreferenced() {
        let cache = SharedDiagramCache::default();

        let key_a = DiagramKey::new("graph TD\nA-->B", 1, 1.0);
        let key_b = DiagramKey::new("graph LR\nC-->D", 1, 1.0);
        let key_c = DiagramKey::new("sequenceDiagram\nAlice->>Bob: Hello", 1, 1.0);

        cache.insert_for_test(key_a.clone(), DiagramEntry::Failed("test".into()));
        cache.insert_for_test(key_b.clone(), DiagramEntry::Failed("test".into()));
        cache.insert_for_test(key_c.clone(), DiagramEntry::Failed("test".into()));

        assert!(cache.contains_key(&key_a));
        assert!(cache.contains_key(&key_b));
        assert!(cache.contains_key(&key_c));

        let open_tab_contents =
            vec!["# Document with diagram\n\n```mermaid\ngraph LR\nC-->D\n```\n"];
        let _ = cache.prune_unreferenced(&open_tab_contents);

        assert!(
            !cache.contains_key(&key_a),
            "Unreferenced diagram A must be pruned"
        );
        assert!(
            cache.contains_key(&key_b),
            "Referenced diagram B must be retained"
        );
        assert!(
            !cache.contains_key(&key_c),
            "Unreferenced diagram C must be pruned"
        );

        let _ = cache.prune_unreferenced(&[]);
        assert!(
            !cache.contains_key(&key_b),
            "All diagrams must be pruned when no tabs are open"
        );
    }

    #[test]
    fn test_diagram_cache_clear() {
        let cache = SharedDiagramCache::default();

        let key = DiagramKey::new("graph TD\nA-->B", 1, 1.0);
        cache.insert_for_test(key.clone(), DiagramEntry::Failed("test".into()));
        assert!(cache.contains_key(&key));

        cache.clear();
        assert!(!cache.contains_key(&key));
    }

    #[gpui_kit::test]
    fn parses_and_renders_table_with_top_actions(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        cx.update(gpui_kit::init);
        let md = "| H1 | H2 |\n|---|---|\n| C1 | C2 |";
        let (window_handle, _) = cx.update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                let entity = cx.new(|cx| TextViewState::markdown(md, cx));
                let extensions = markdown_extensions(&entity, Arc::new(Mutex::new(None)));
                cx.new(|_| {
                    struct TableTestRoot {
                        text: SharedString,
                        extensions: MarkdownExtensions,
                    }
                    impl gpui_kit::Render for TableTestRoot {
                        fn render(
                            &mut self,
                            _: &mut Window,
                            _: &mut gpui_kit::Context<'_, Self>,
                        ) -> impl IntoElement {
                            TextView::markdown("table-test", self.text.clone())
                                .markdown_extensions(self.extensions.clone())
                        }
                    }
                    TableTestRoot {
                        text: md.into(),
                        extensions,
                    }
                })
            })
            .unwrap()
        });
        cx.update_window(window_handle, |_, window, cx| {
            use gpui_kit::component::WindowExt as _;
            window.draw(cx).clear(cx);
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
    }

    #[test]
    fn table_content_width_calculates_dimensions() {
        assert_eq!(table_content_width(""), 0.0);
        assert_eq!(table_content_width("Just plain text\nwithout tables"), 0.0);

        let small = "| A | B |\n|---|---|\n| 1 | 2 |";
        let width_small = table_content_width(small);
        assert!(width_small > 0.0);

        let wide = "| Col 1 | Col 2 | Col 3 | Col 4 |\n|---|---|---|---|\n| Very long text with multiple words that should expand the column | Second column | Third | Fourth |";
        let width_wide = table_content_width(wide);
        assert!(width_wide > width_small);

        let with_breaks = "| A |\n|---|\n| First line<br>Second longer line of text |";
        assert!(table_content_width(with_breaks) > 0.0);
    }
}
