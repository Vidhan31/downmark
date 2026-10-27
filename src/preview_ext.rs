//! Preview extensions beyond `CommonMark` and GFM: GitHub alerts, math and
//! Mermaid diagrams.

use std::{
    collections::HashMap,
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
        button::Button,
        h_flex,
        text::{
            InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode,
            MarkdownParseContext, MarkdownPlugin, markdown_ast::Node,
        },
        tooltip::Tooltip,
        v_flex,
    },
    div, img,
    prelude::FluentBuilder as _,
    px,
};

use crate::{
    actions::ViewDiagram,
    analysis::content_hash,
    diagram::{self, DiagramTheme},
    math::{self, MathStyle},
};

/// The parser configuration and plugins the preview renders with.
pub(crate) fn markdown_extensions(preview: &gpui_kit::Entity<TextViewState>) -> MarkdownExtensions {
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
            .plugin(LinkTooltipPlugin)
    };
    // Alert bodies are rendered by their own nested view, which needs math too.
    let alert = AlertPlugin {
        body_extensions: with_math(MarkdownExtensions::default()),
    };
    with_math(
        MarkdownExtensions::default()
            .frontmatter()
            .plugin(gpui_kit::component::text::FrontmatterPlugin::new())
            .plugin(alert),
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
    fn from_marker(marker: &str) -> Option<Self> {
        let name = marker.strip_prefix("[!")?.strip_suffix(']')?;
        Some(match name.to_ascii_uppercase().as_str() {
            "NOTE" => Self::Note,
            "TIP" => Self::Tip,
            "IMPORTANT" => Self::Important,
            "WARNING" => Self::Warning,
            "CAUTION" => Self::Caution,
            _ => return None,
        })
    }

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

#[derive(Debug, Clone, PartialEq)]
struct Alert {
    kind: AlertKind,
    /// The alert's Markdown content, without the quote markers.
    body: SharedString,
}

/// Split a blockquote's source into an alert, if its first line is an alert
/// marker and nothing else, as GitHub requires.
pub(crate) fn parse_alert(blockquote_source: &str) -> Option<(AlertKind, String)> {
    let mut lines = blockquote_source.lines().map(|line| {
        let line = line.trim_start();
        match line.strip_prefix('>') {
            Some(rest) => rest.strip_prefix(' ').unwrap_or(rest),
            // A lazy continuation line.
            None => line,
        }
    });
    let kind = AlertKind::from_marker(lines.next()?.trim())?;
    let body = lines.collect::<Vec<_>>().join("\n");
    Some((kind, body.trim().to_string()))
}

struct AlertPlugin {
    body_extensions: MarkdownExtensions,
}

impl MarkdownPlugin for AlertPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "github-alert"
    }

    fn parse(&self, node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let Node::Blockquote(_) = node else {
            return None;
        };
        let source = cx.node_source(node)?;
        let (kind, body) = parse_alert(source)?;
        Some(
            MarkdownNode::new(
                "github-alert",
                Alert {
                    kind,
                    body: body.clone().into(),
                },
            )
            .text(format!("{}\n{body}", kind.title()))
            .markdown(source),
        )
    }

    fn render(&self, node: &MarkdownNode, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let alert = node.data::<Alert>().expect("alert node data");
        let color = alert.kind.color(cx);
        let id = SharedString::from(format!("alert-{}", content_hash(&alert.body)));

        v_flex()
            .id(id.clone())
            .role(gpui_kit::Role::Note)
            .aria_label(alert.kind.title())
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
                    .gap_2()
                    .text_color(color)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(Icon::new(alert.kind.icon()).small())
                    .child(alert.kind.title()),
            )
            .when(!alert.body.is_empty(), |this| {
                this.child(
                    TextView::markdown(id, alert.body.clone())
                        .markdown_extensions(self.body_extensions.clone())
                        .on_link_click(crate::links::on_click)
                        .w_full(),
                )
            })
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
        let theme_hash = content_hash(&format!("{theme:?}"));
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
                            .cursor_pointer()
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

/// Extracts plain text from inline markdown AST nodes.
fn plain_link_text(children: &[Node]) -> String {
    let mut out = String::new();
    for child in children {
        match child {
            Node::Text(t) => out.push_str(&t.value),
            Node::InlineCode(c) => out.push_str(&c.value),
            Node::InlineMath(m) => out.push_str(&m.value),
            other => {
                if let Some(sub) = other.children() {
                    out.push_str(&plain_link_text(sub));
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkData {
    url: String,
    title: Option<String>,
    text: String,
}

/// Renders markdown hyperlinks with interactive hover tooltips, status bar target
/// notifications, and accessible screen reader information.
struct LinkTooltipPlugin;

impl MarkdownPlugin for LinkTooltipPlugin {
    fn name(&self) -> &'static str {
        "link-tooltip"
    }

    fn parse(&self, node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let Node::Link(link) = node else {
            return None;
        };
        let text = plain_link_text(&link.children);
        // Only claim links that fit cleanly inline without wrapping across multiple paragraphs
        if text.is_empty() || text.len() > 80 || text.contains('\n') {
            return None;
        }
        let source = cx.node_source(node).unwrap_or(&link.url);
        Some(
            MarkdownNode::new(
                "link-tooltip",
                LinkData {
                    url: link.url.clone(),
                    title: link.title.clone(),
                    text: text.clone(),
                },
            )
            .text(text.clone())
            .accessibility_label(format!("Link: {text}, destination: {}", link.url))
            .markdown(source),
        )
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        _: &InlineRenderContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let data = node.data::<LinkData>()?;
        let url = SharedString::from(data.url.clone());
        let url_click = url.clone();
        let url_aux = url.clone();
        let url_hover = url.clone();
        let text = data.text.clone();
        let target = crate::links::classify(&data.url, None);
        let tooltip_text =
            crate::links::format_target_tooltip(target.as_ref(), &data.url, data.title.as_deref());
        let hash = content_hash(&format!("{}:{}", data.url, text));
        let id = SharedString::from(format!("a11y-link-{hash}"));

        let el = div()
            .id(id)
            .cursor_pointer()
            .text_color(cx.theme().primary)
            .hover(gpui_kit::Styled::underline)
            .role(gpui_kit::Role::Link)
            .aria_label(format!("Link: {text}, leads to {url}"))
            .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
            .on_hover(move |hovered, _, cx| {
                if *hovered {
                    cx.set_global(crate::workspace::HoveredLink(Some(url_hover.clone())));
                } else {
                    cx.set_global(crate::workspace::HoveredLink(None));
                }
                cx.refresh_windows();
            })
            .on_click(move |event, window, cx| {
                crate::links::on_click(&url_click, event, window, cx);
            })
            .on_aux_click(move |event, window, cx| {
                crate::links::on_click(&url_aux, event, window, cx);
            })
            .child(text);

        Some(InlineElement::new(el))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_alerts() {
        let (kind, body) = parse_alert("> [!WARNING]\n> Back up first.\n>\n> - Really.").unwrap();
        assert_eq!(kind, AlertKind::Warning);
        assert_eq!(body, "Back up first.\n\n- Really.");

        assert_eq!(
            parse_alert("> [!tip]").unwrap(),
            (AlertKind::Tip, String::new())
        );
        // The marker must stand alone on the first line.
        assert!(parse_alert("> [!NOTE] inline text").is_none());
        assert!(parse_alert("> Just a quote").is_none());
        assert!(parse_alert("> [!UNKNOWN]\n> text").is_none());
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

        // Tab 1 contains key_b. No tabs contain key_a or key_c.
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

        // When all tabs are closed (empty list), everything is pruned
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
}
