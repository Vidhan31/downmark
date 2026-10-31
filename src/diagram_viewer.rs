//! A modal showing one Mermaid diagram, to zoom into and move around.
//!
//! The diagram is drawn from its SVG at every zoom: GPUI draws the whole
//! diagram, which stays sharp up to about twice its natural size, and past
//! that the part in view is drawn again at the exact zoom, so small text in
//! a large diagram stays crisp however far it's enlarged.

use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

use gpui_kit::{
    App, AppContext as _, Bounds, Context, CursorStyle, FocusHandle, Focusable, FontWeight, Image,
    ImageFormat, InteractiveElement as _, IntoElement, KeyBinding, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, PinchEvent, Pixels, Point, Render,
    ScrollDelta, ScrollEvent, SharedString, Size, Styled as _, Task, Window, actions,
    assets::IconName,
    base::ElementExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, Theme,
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div, img, point,
    prelude::FluentBuilder as _,
    px, size,
};

use crate::{
    actions::{CloseWindow, ZoomIn, ZoomOut, ZoomReset},
    analysis::content_hash,
    diagram::{self, DiagramTheme},
};

actions!(
    diagram_viewer,
    [
        DiagramZoomIn,
        DiagramZoomOut,
        FitDiagram,
        ActualSize,
        PanLeft,
        PanRight,
        PanUp,
        PanDown
    ]
);

const KEY_CONTEXT: &str = "DiagramViewer";
/// Space kept around the diagram when it's fitted to the window.
const MARGIN: f32 = 32.;
/// Each zoom step, by button or key.
const ZOOM_STEP: f32 = 1.25;
/// Each notch of a mouse wheel.
const WHEEL_STEP: f32 = 1.2;
const MAX_ZOOM: f32 = 16.;
/// A fitted diagram is enlarged at most this much; tiny ones get no bigger.
const MAX_FIT: f32 = 6.;
/// How far the arrow keys move the diagram.
const PAN_STEP: f32 = 64.;
/// GPUI rasterizes SVG images at twice their size, so they stay sharp up to
/// this many device pixels per diagram pixel.
const BASE_DENSITY: f32 = 2.;
/// Pause after zooming or moving before the view is drawn again sharply.
const SHARP_DELAY: Duration = Duration::from_millis(120);

pub(crate) fn bind_keys(cx: &mut App) {
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("+", DiagramZoomIn, context),
        KeyBinding::new("=", DiagramZoomIn, context),
        KeyBinding::new("-", DiagramZoomOut, context),
        KeyBinding::new("0", FitDiagram, context),
        KeyBinding::new("1", ActualSize, context),
        KeyBinding::new("left", PanLeft, context),
        KeyBinding::new("right", PanRight, context),
        KeyBinding::new("up", PanUp, context),
        KeyBinding::new("down", PanDown, context),
        crate::actions::key_binding("secondary-w", CloseWindow, context),
    ]);
}

/// Show the diagram drawn from `source` in a modal dialog.
pub(crate) fn open(source: &str, title: String, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let viewer = cx.new(|cx| DiagramViewer::new(source.to_string(), title, window, cx));
    let initial_focus = std::cell::Cell::new(true);

    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            viewer.read(cx).focus_handle(cx).focus(window, cx);
        }
        let view_size = window.viewport_size();
        let width = (view_size.width * 0.88)
            .min(px(1200.))
            .min((view_size.width - px(32.)).max(px(320.)));
        let height = (view_size.height * 0.82)
            .min(px(850.))
            .min((view_size.height - px(32.)).max(px(240.)));

        let viewer_elem = viewer.clone();
        dialog
            .w(width)
            .h(height)
            .centered()
            .close_button(false)
            .p_0()
            .content(move |content, _window, _cx| {
                content
                    .h(height)
                    .size_full()
                    .p_0()
                    .overflow_hidden()
                    .child(viewer_elem.clone())
            })
    });
}

/// A diagram drawn for the viewer.
struct Drawn {
    svg: Arc<str>,
    image: Arc<Image>,
    /// The SVG's viewBox: where the diagram's coordinates start, and its
    /// natural size in pixels.
    view_box: Bounds<f32>,
}

/// The part of the diagram in view, drawn sharply for one zoom and place.
struct Sharp {
    view: View,
    image: Arc<Image>,
}

/// What the viewport shows: the zoom (window pixels per diagram pixel) and
/// where the diagram's top left corner is, relative to the viewport's.
#[derive(Debug, Clone, Copy, PartialEq)]
struct View {
    zoom: f32,
    offset: Point<Pixels>,
    viewport: Size<Pixels>,
    scale_factor: f32,
}

pub(crate) struct DiagramViewer {
    focus_handle: FocusHandle,
    source: String,
    title: String,
    diagram: Option<Result<Arc<Drawn>, String>>,
    /// The colors the diagram was drawn with, to redraw it if the theme
    /// changes.
    theme: Option<DiagramTheme>,
    zoom: f32,
    offset: Point<Pixels>,
    /// The diagram's area of the window, once laid out.
    viewport: Bounds<Pixels>,
    /// Whether the diagram follows the window's size, until the user zooms
    /// or moves it.
    fitted: bool,
    /// Where a drag started: the pointer's position and the offset then.
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    /// The window's device pixels per pixel, as of the last frame.
    scale_factor: f32,
    sharp: Option<Sharp>,
    render_task: Option<Task<()>>,
    sharp_task: Option<Task<()>>,
    _theme_subscription: gpui_kit::Subscription,
}

impl DiagramViewer {
    fn new(source: String, title: String, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let theme_subscription = cx.observe_global_in::<Theme>(window, |this, _, cx| {
            this.draw(cx);
        });
        let mut this = Self {
            focus_handle,
            source,
            title,
            diagram: None,
            theme: None,
            zoom: 1.,
            offset: Point::default(),
            viewport: Bounds::default(),
            fitted: true,
            drag: None,
            scale_factor: window.scale_factor(),
            sharp: None,
            render_task: None,
            sharp_task: None,
            _theme_subscription: theme_subscription,
        };
        this.draw(cx);
        this
    }

    /// Lay out and draw the diagram in the current theme's colors.
    fn draw(&mut self, cx: &mut Context<'_, Self>) {
        let theme = crate::themes::diagram_theme(cx);
        if self.theme.as_ref() == Some(&theme) {
            return;
        }
        self.theme = Some(theme.clone());
        let source = self.source.clone();
        let id = format!("viewer-{:x}", content_hash(&source));
        let task = cx.background_executor().spawn(async move {
            let _permit = diagram::RENDER_SEMAPHORE.acquire().await;
            let diagram = diagram::render(&source, &theme, &id)?;
            let view_box = view_box(&diagram.svg)
                .unwrap_or_else(|| Bounds::new(point(0., 0.), size(diagram.width, diagram.height)));
            let svg: Arc<str> = diagram.svg.into();
            Ok(Arc::new(Drawn {
                image: Arc::new(Image::from_bytes(ImageFormat::Svg, svg.as_bytes().to_vec())),
                svg,
                view_box,
            }))
        });
        self.render_task = Some(cx.spawn(async move |this, cx| {
            let diagram = task.await;
            _ = this.update(cx, |this, cx| {
                let first = this.diagram.is_none();
                this.diagram = Some(diagram);
                this.sharp = None;
                if first {
                    this.fitted = true;
                }
                this.settle(cx);
                cx.notify();
            });
        }));
    }

    fn drawn(&self) -> Option<&Arc<Drawn>> {
        self.diagram.as_ref()?.as_ref().ok()
    }

    /// The zoom that shows the whole diagram in the window.
    fn fit_zoom(&self) -> Option<f32> {
        let drawn = self.drawn()?;
        let room = self.viewport.size;
        if room.width <= px(0.) || room.height <= px(0.) {
            return None;
        }
        let width = (f32::from(room.width) - 2. * MARGIN).max(1.);
        let height = (f32::from(room.height) - 2. * MARGIN).max(1.);
        Some(
            (width / drawn.view_box.size.width)
                .min(height / drawn.view_box.size.height)
                .min(MAX_FIT),
        )
    }

    fn min_zoom(&self) -> f32 {
        self.fit_zoom().unwrap_or(1.).min(1.) / 2.
    }

    /// Zoom by `factor`, keeping the diagram point under `anchor` (in window
    /// coordinates) where it is.
    fn zoom_by(&mut self, factor: f32, anchor: Option<Point<Pixels>>, cx: &mut Context<'_, Self>) {
        if self.drawn().is_none() {
            return;
        }
        let new_zoom = (self.zoom * factor).clamp(self.min_zoom(), MAX_ZOOM);
        let anchor = anchor.map_or_else(
            || {
                point(
                    self.viewport.size.width / 2.,
                    self.viewport.size.height / 2.,
                )
            },
            |anchor| anchor - self.viewport.origin,
        );
        let ratio = new_zoom / self.zoom;
        self.offset = point(
            anchor.x - (anchor.x - self.offset.x) * ratio,
            anchor.y - (anchor.y - self.offset.y) * ratio,
        );
        self.zoom = new_zoom;
        self.fitted = false;
        self.settle(cx);
    }

    fn set_zoom(&mut self, zoom: f32, cx: &mut Context<'_, Self>) {
        self.zoom_by(zoom / self.zoom, None, cx);
    }

    fn pan(&mut self, dx: f32, dy: f32, cx: &mut Context<'_, Self>) {
        self.offset = point(self.offset.x + px(dx), self.offset.y + px(dy));
        self.fitted = false;
        self.settle(cx);
    }

    fn fit(&mut self, cx: &mut Context<'_, Self>) {
        self.fitted = true;
        self.settle(cx);
    }

    /// Apply fitting, keep the diagram in view, and ask for a sharp drawing
    /// of what's shown if GPUI's own would be blurry.
    fn settle(&mut self, cx: &mut Context<'_, Self>) {
        let Some(drawn) = self.drawn().cloned() else {
            return;
        };
        if self.fitted
            && let Some(zoom) = self.fit_zoom()
        {
            self.zoom = zoom;
        }
        // A diagram smaller than the window sits in its middle; a larger one
        // can be moved until its edge meets the window's.
        let room = self.viewport.size;
        let width = px(drawn.view_box.size.width * self.zoom);
        let height = px(drawn.view_box.size.height * self.zoom);
        let place = |offset: Pixels, extent: Pixels, room: Pixels| {
            if extent <= room {
                (room - extent) / 2.
            } else {
                offset.clamp(room - extent, px(0.))
            }
        };
        self.offset = point(
            place(self.offset.x, width, room.width),
            place(self.offset.y, height, room.height),
        );
        cx.notify();
        self.schedule_sharp(drawn, cx);
    }

    fn view(&self, scale_factor: f32) -> View {
        View {
            zoom: self.zoom,
            offset: self.offset,
            viewport: self.viewport.size,
            scale_factor,
        }
    }

    fn schedule_sharp(&mut self, drawn: Arc<Drawn>, cx: &mut Context<'_, Self>) {
        let scale_factor = self.scale_factor;
        let view = self.view(scale_factor);
        let blurry = self.zoom * scale_factor > BASE_DENSITY * 1.05;
        if !blurry || self.sharp.as_ref().is_some_and(|sharp| sharp.view == view) {
            self.sharp_task = None;
            return;
        }
        self.sharp_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHARP_DELAY).await;
            let task = cx
                .background_executor()
                .spawn(async move { draw_view(&drawn, view) });
            let Some(png) = task.await else {
                return;
            };
            _ = this.update(cx, |this, cx| {
                this.sharp = Some(Sharp {
                    view,
                    image: Arc::new(Image::from_bytes(ImageFormat::Png, png)),
                });
                cx.notify();
            });
        }));
    }

    fn on_scroll(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        cx.stop_propagation();
        match event.delta {
            // A mouse wheel zooms, as in image viewers, a step per notch
            // (which systems count as one to three lines).
            ScrollDelta::Lines(lines) => self.zoom_by(
                WHEEL_STEP.powf(lines.y.clamp(-1., 1.)),
                Some(event.position),
                cx,
            ),
            ScrollDelta::Pixels(delta) if event.modifiers.secondary(cx) => {
                let factor = (f32::from(delta.y) / 200.).exp();
                self.zoom_by(factor, Some(event.position), cx);
            }
            ScrollDelta::Pixels(_) => {
                let delta = event.delta.pixel_delta(window.line_height());
                self.pan(f32::from(delta.x), f32::from(delta.y), cx);
            }
        }
    }

    fn on_pinch(&mut self, event: &PinchEvent, cx: &mut Context<'_, Self>) {
        self.zoom_by(1. + event.delta, Some(event.position), cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.focus_handle.focus(window, cx);
        if event.click_count == 2 {
            self.drag = None;
            if self.fitted {
                self.zoom_by(2., Some(event.position), cx);
            } else {
                self.fit(cx);
            }
            return;
        }
        self.drag = Some((event.position, self.offset));
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((start, offset)) = self.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            cx.notify();
            return;
        }
        self.offset = offset + (event.position - start);
        self.fitted = false;
        self.settle(cx);
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.drag = None;
        cx.notify();
    }

    fn render_header(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let percent = format!("{:.0}%", self.zoom * 100.);
        let ready = self.drawn().is_some();
        h_flex()
            .id("diagram-header")
            .w_full()
            .h(px(40.))
            .px_3()
            .justify_between()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex().items_center().gap_2().min_w_0().child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(cx.theme().foreground)
                        .truncate()
                        .child(SharedString::from(self.title.clone())),
                ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("zoom-out")
                            .accessibility_label("Zoom out")
                            .ghost()
                            .small()
                            .compact()
                            .icon(Icon::new(IconName::ZoomOut))
                            .tooltip("Zoom out")
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.zoom_by(1. / ZOOM_STEP, None, cx);
                            })),
                    )
                    .child(
                        Button::new("actual-size")
                            .accessibility_label("Actual size 100%")
                            .ghost()
                            .small()
                            .compact()
                            .label(percent)
                            .tooltip("Actual size")
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| this.set_zoom(1., cx))),
                    )
                    .child(
                        Button::new("zoom-in")
                            .accessibility_label("Zoom in")
                            .ghost()
                            .small()
                            .compact()
                            .icon(Icon::new(IconName::ZoomIn))
                            .tooltip("Zoom in")
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.zoom_by(ZOOM_STEP, None, cx);
                            })),
                    )
                    .child(
                        Button::new("fit")
                            .accessibility_label("Fit to view")
                            .ghost()
                            .small()
                            .compact()
                            .icon(Icon::new(IconName::Maximize))
                            .tooltip("Fit to view")
                            .selected(self.fitted)
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| this.fit(cx))),
                    )
                    .child(div().h_4().w_px().mx_1().bg(cx.theme().border))
                    .child(
                        Button::new("close-diagram")
                            .accessibility_label("Close diagram viewer (Esc)")
                            .ghost()
                            .small()
                            .compact()
                            .icon(Icon::new(IconName::Close))
                            .tooltip("Close")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }

    fn render_canvas(&self, window: &Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let content = match &self.diagram {
            None => div().into_any_element(),
            Some(Err(error)) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .p_8()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(Icon::new(IconName::TriangleAlert))
                .child(error.clone())
                .into_any_element(),
            Some(Ok(drawn)) => {
                let width = px(drawn.view_box.size.width * self.zoom);
                let height = px(drawn.view_box.size.height * self.zoom);
                let view = self.view(window.scale_factor());
                let sharp = self
                    .sharp
                    .as_ref()
                    .filter(|sharp| sharp.view == view)
                    .map(|sharp| Arc::clone(&sharp.image));
                div()
                    .size_full()
                    .relative()
                    .child(
                        img(Arc::clone(&drawn.image))
                            .absolute()
                            .left(self.offset.x)
                            .top(self.offset.y)
                            .w(width)
                            .h(height),
                    )
                    .when_some(sharp, |this, sharp| {
                        this.child(
                            img(sharp)
                                .absolute()
                                .left_0()
                                .top_0()
                                .w(view.viewport.width)
                                .h(view.viewport.height),
                        )
                    })
                    .into_any_element()
            }
        };
        div()
            .id("diagram-canvas")
            .flex_1()
            .min_h_0()
            .w_full()
            .relative()
            .overflow_hidden()
            .bg(cx.theme().background)
            .cursor(if self.drag.is_some() {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::OpenHand
            })
            .on_scroll(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| this.on_pinch(event, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .child(content)
            .on_prepaint(move |bounds, _, cx| {
                // Notifying mid-frame can be lost; update once it's done.
                cx.defer(move |cx| {
                    _ = entity.update(cx, |this, cx| {
                        if this.viewport != bounds {
                            this.viewport = bounds;
                            this.settle(cx);
                        }
                    });
                });
            })
    }
}

impl Focusable for DiagramViewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DiagramViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if (self.scale_factor - window.scale_factor()).abs() > f32::EPSILON {
            // Moved to a display with a different density.
            self.scale_factor = window.scale_factor();
            self.settle(cx);
        }
        v_flex()
            .id("diagram-viewer")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(
                cx.listener(|this, _: &DiagramZoomIn, _, cx| this.zoom_by(ZOOM_STEP, None, cx)),
            )
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(ZOOM_STEP, None, cx)))
            .on_action(cx.listener(|this, _: &DiagramZoomOut, _, cx| {
                this.zoom_by(1. / ZOOM_STEP, None, cx);
            }))
            .on_action(
                cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(1. / ZOOM_STEP, None, cx)),
            )
            .on_action(cx.listener(|this, _: &FitDiagram, _, cx| this.fit(cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.fit(cx)))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| this.set_zoom(1., cx)))
            .on_action(cx.listener(|this, _: &PanLeft, _, cx| this.pan(PAN_STEP, 0., cx)))
            .on_action(cx.listener(|this, _: &PanRight, _, cx| this.pan(-PAN_STEP, 0., cx)))
            .on_action(cx.listener(|this, _: &PanUp, _, cx| this.pan(0., PAN_STEP, cx)))
            .on_action(cx.listener(|this, _: &PanDown, _, cx| this.pan(0., -PAN_STEP, cx)))
            .on_action(|_: &gpui_kit::component::input::Escape, window, cx| window.close_dialog(cx))
            .on_action(|_: &gpui_kit::component::dialog::Cancel, window, cx| window.close_dialog(cx))
            .on_action(|_: &CloseWindow, window, cx| window.close_dialog(cx))
            .child(self.render_header(cx))
            .child(self.render_canvas(window, cx))
    }
}

/// The `viewBox` of an SVG's root element.
fn view_box(svg: &str) -> Option<Bounds<f32>> {
    let root = &svg[..svg.find('>')?];
    let start = root.find(" viewBox=\"")? + " viewBox=\"".len();
    let end = root[start..].find('"')? + start;
    let numbers: Vec<f32> = root[start..end]
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();
    let [x, y, width, height] = numbers[..] else {
        return None;
    };
    (width > 0. && height > 0.).then(|| Bounds::new(point(x, y), size(width, height)))
}

/// The SVG of just the part of the diagram `view` shows, sized in device
/// pixels.
fn cropped_svg(drawn: &Drawn, view: View) -> Option<String> {
    let svg = &drawn.svg;
    let root_end = svg.find('>')?;
    let mut root = svg[..root_end].to_string();
    for name in ["width", "height", "viewBox", "preserveAspectRatio"] {
        let needle = format!(" {name}=\"");
        if let Some(start) = root.find(&needle) {
            let end = root[start + needle.len()..].find('"')? + start + needle.len() + 1;
            root.replace_range(start..end, "");
        }
    }
    let width = (f32::from(view.viewport.width) * view.scale_factor).round();
    let height = (f32::from(view.viewport.height) * view.scale_factor).round();
    if width < 1. || height < 1. {
        return None;
    }
    let origin = drawn.view_box.origin;
    let x = origin.x - f32::from(view.offset.x) / view.zoom;
    let y = origin.y - f32::from(view.offset.y) / view.zoom;
    let view_width = f32::from(view.viewport.width) / view.zoom;
    let view_height = f32::from(view.viewport.height) / view.zoom;
    let root = root.replacen(
        "<svg",
        &format!(
            "<svg width=\"{width}\" height=\"{height}\" viewBox=\"{x} {y} {view_width} {view_height}\" preserveAspectRatio=\"none\""
        ),
        1,
    );
    Some(format!("{root}{}", &svg[root_end..]))
}

/// Draw the part of the diagram `view` shows at its exact zoom, as PNG.
fn draw_view(drawn: &Drawn, view: View) -> Option<Vec<u8>> {
    static OPTIONS: LazyLock<resvg::usvg::Options<'static>> = LazyLock::new(|| {
        let mut options = resvg::usvg::Options::default();
        options.fontdb_mut().load_system_fonts();
        options
    });
    let svg = cropped_svg(drawn, view)?;
    let tree = resvg::usvg::Tree::from_str(&svg, &OPTIONS).ok()?;
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(svg: &str) -> Drawn {
        Drawn {
            svg: svg.into(),
            image: Arc::new(Image::from_bytes(ImageFormat::Svg, Vec::new())),
            view_box: view_box(svg).unwrap(),
        }
    }

    #[test]
    fn reads_the_view_box() {
        let bounds = view_box(r#"<svg xmlns="x" viewBox="-8 -8 200.5 100"><g/></svg>"#).unwrap();
        assert_eq!(bounds, Bounds::new(point(-8., -8.), size(200.5, 100.)));
        assert!(view_box("<svg><g/></svg>").is_none());
    }

    #[test]
    fn crops_to_the_view() {
        let drawn = drawn(
            r#"<svg width="200" height="100" viewBox="-8 -8 200 100" xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#,
        );
        // Zoomed 4×, scrolled 80 px right and 40 down, on a 2× display.
        let view = View {
            zoom: 4.,
            offset: point(px(-80.), px(-40.)),
            viewport: size(px(400.), px(300.)),
            scale_factor: 2.,
        };
        let svg = cropped_svg(&drawn, view).unwrap();
        assert!(svg.starts_with(
            r#"<svg width="800" height="600" viewBox="12 2 100 75" preserveAspectRatio="none" xmlns"#
        ));
        assert!(svg.ends_with(r#"<rect width="10" height="10"/></svg>"#));
        let png = draw_view(&drawn, view).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
    }

    struct TestView;

    impl Render for TestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn opens_diagram_modal_and_closes(cx: &mut gpui_kit::TestAppContext) {
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
            open("flowchart TD\nA-->B", "Diagram — test".into(), window, cx);
            assert!(window.has_active_dialog(cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window_handle, |_, window, cx| {
            crate::actions::bind_keys(cx);
            window.draw(cx).clear(cx);
            let handled =
                window.dispatch_keystroke(gpui_kit::Keystroke::parse("escape").unwrap(), cx);
            assert!(handled);
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
    }

    #[test]
    fn draws_real_diagrams_sharply() {
        let diagram = diagram::render(
            "flowchart LR\n  A[Write] --> B[Ship]",
            &DiagramTheme::document(),
            "viewer-test",
        )
        .unwrap();
        let drawn = drawn(&diagram.svg);
        let view = View {
            zoom: 3.,
            offset: point(px(0.), px(0.)),
            viewport: size(px(300.), px(200.)),
            scale_factor: 1.,
        };
        assert!(draw_view(&drawn, view).is_some());
    }
}
