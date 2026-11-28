//! The tools' overlay list on the canvas: the marks a tool draws (`ToolMarks`), the marquees, the settled
//! shared outlines and the Distance Guides (framework canvas controls), and the hover tips (`HoverTips`).
//!
//! `show` runs once per pass from `tool_adapter::run`, after the tool host took the pass's input, so the list
//! it shows is the one the tool just updated. None of these controls takes a press.

use craft::behavior::{CanvasView, HitLevel, HitPriority, HitShape, PartKey, Sense};
use craft::draw::ui::{HandleStyle, PathOptionsSettings};
use craft::ui::canvas::{CanvasSpace, canvas_space, hud};
use craft::ui::canvas_controls::{Marquee, MarqueeLook, Measure, SelectionOutline, SmartGuides, path_runs};
use craft::ui::contract::{Common, Component, Cx, Input, MeasureCx, PaintUi, PartsCx, Role, Show as _};
use craft::ui::primitives::{Dashes, DataWidth, Draw, PaintShape, StrokeWidth, TextRun};
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, DataColor, Metrics, StyleColor, StyleCx, TypeRole, UiTokens};
use egui::{Color32, CornerRadius, Pos2, Rect as ScreenRect, Response, Vec2};
use vectorcraft_tools::Overlay;

use super::artboards::names_an_artboard;
use super::{VectorcraftApp, tool_adapter};

/// The part key of a hover tip's disc.
const TIP: PartKey = PartKey::new("tip");
/// The radius of a gradient stop's chip, of its rim and of its selection ring, in points: a chip keeps one size at
/// every zoom.
const CHIP: f32 = 5.0;
const CHIP_RIM: f32 = 6.0;
const CHIP_RING: f32 = 8.0;
/// Where the Selection tool's rule badge sits from the pointer, in points.
const BADGE_OFFSET: Vec2 = Vec2::new(14.0, 14.0);

/// Shows `overlays` (the session's list, read after the tool host ran) over `extra` (the thread and gap overlays),
/// on the canvas of `space`. The extras lie under the tool's own marks.
pub(super) fn show(app: &VectorcraftApp, ui: &mut egui::Ui, space: &CanvasSpace, overlays: &[Overlay], extra: &[Overlay]) {
    let rect = space.view.rect;
    let options = super::selection::path_options(&app.session.prefs);
    let quiet = tool_adapter::in_gesture(app, ui.ctx()) || tool_adapter::view_tool_held(app);
    let lists = [extra, overlays];
    for (i, o) in extra.iter().chain(overlays).enumerate() {
        ui.push_id(("tool-control", i), |ui| show_control(ui, rect, o));
    }
    for (i, list) in lists.into_iter().enumerate() {
        ui.push_id(("tool-marks", i), |ui| {
            ToolMarks { common: Common::new(), overlays: list, options }.show_at(ui, rect);
        });
    }
    ui.push_id("tool-tips", |ui| {
        HoverTips { common: Common::new(), lists, quiet }.show_at(ui, rect);
    });
}

/// The framework control `o` stands for, if it is one: a marquee, a settled outline or a distance.
fn show_control(ui: &mut egui::Ui, rect: ScreenRect, o: &Overlay) {
    match o {
        Overlay::Marquee(r) => {
            Marquee::rect(name(false), *r).look(MarqueeLook::Ants).show_at(ui, rect);
        }
        Overlay::SelectionMarquee { rect: r, enclosed, .. } => {
            let look = if *enclosed { MarqueeLook::Contrast(DataColor::new(Color32::WHITE)) } else { MarqueeLook::Ants };
            Marquee::rect(name(*enclosed), *r).look(look).show_at(ui, rect);
        }
        Overlay::SharedPath { path, color, width, dash } => {
            let paths = std::slice::from_ref(&**path);
            // The outline has no width prop, so `width` only keys the revision.
            let revision = super::selection::outline_revision(paths, None, (*color, width.to_bits(), dash.map(f64::to_bits)));
            let outline = if dash.is_some() { SelectionOutline::dashed(Label::Empty, paths) } else { SelectionOutline::solid(Label::Empty, paths) };
            outline.color(data(*color)).revision(revision).show_at(ui, rect);
        }
        Overlay::Distance { .. } => {
            if let Some((measure, color)) = distance_measure(o) {
                SmartGuides::new(&[], std::slice::from_ref(&measure)).color(color).show_at(ui, rect);
            }
        }
        _ => {}
    }
}

/// The accessible name of a selection marquee: the rule it selects by.
fn name(enclosed: bool) -> Label {
    Label::from(badge_rule(enclosed).0)
}

/// The measure and the colour a [`Overlay::Distance`] is shown with; `None` for any other overlay.
fn distance_measure(o: &Overlay) -> Option<(Measure, DataColor)> {
    let Overlay::Distance { a, b, text, color } = o else { return None };
    Some((Measure::new(*a, *b).labelled(text.clone()), data(*color)))
}

/// The rule a selection marquee selects by and the one `E` switches to, as the badge names them.
fn badge_rule(enclosed: bool) -> (String, String) {
    let (current, other) = if enclosed { (tl!("Fully Enclosed"), tl!("Touching")) } else { (tl!("Touching"), tl!("Fully Enclosed")) };
    (current.to_string(), other.to_string())
}

/// Where the rule badge of a marquee whose pointer is at `pointer` is placed: beside it, kept inside the canvas.
fn badge_at(view: &CanvasView, pointer: vectorcraft_geom::Point) -> Pos2 {
    let wanted = view.to_screen(pointer) + BADGE_OFFSET;
    let rect = view.rect;
    Pos2::new(wanted.x.clamp(rect.min.x, rect.max.x), wanted.y.clamp(rect.min.y, rect.max.y))
}

fn data(c: [u8; 3]) -> DataColor {
    DataColor::new(Color32::from_rgb(c[0], c[1], c[2]))
}

fn data_rgba(c: [u8; 4]) -> DataColor {
    DataColor::new(Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
}

/// The style of the tool marks.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ToolMarksStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// A measure pill's fill and the rim of a stop chip.
    dark: StyleColor,
    /// Text on the pill.
    ink: StyleColor,
    /// The selection ring of a stop chip.
    accent: StyleColor,
    /// Around a measure's text.
    pad: Vec2,
    radius: CornerRadius,
    /// A direction handle's end radius when the preferences give no anchor size.
    handle: f32,
}

/// The marks a tool draws over the document: paths, lines, anchors, handles, labels, highlights, measures and stop
/// chips ([`paint`]), and the Selection tool's rule badge.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control, content_painter)]
struct ToolMarks<'a> {
    #[common]
    common: Common,
    overlays: &'a [Overlay],
    options: PathOptionsSettings,
}

impl Component for ToolMarks<'_> {
    const ROLE: Role = Role::None;
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = ToolMarksStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ToolMarksStyleKey) -> ToolMarksStyle {
        let tokens = cx.tokens;
        ToolMarksStyle {
            metrics: Metrics::new(),
            dark: cx.color(tokens.color.overlay.halo),
            ink: cx.color(tokens.color.text.on_strong),
            accent: cx.color(tokens.color.accent.solid),
            pad: Vec2::new(tokens.space.pad.control, tokens.space.gap.tight),
            radius: UiTokens::corner(tokens.shape.radius.control),
            handle: tokens.space.gap.tight,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let _painter = ui.content_painter(cx);
        paint(&ui.draw(), &space.view, self.overlays, &self.options, cx.style);
        for o in self.overlays {
            if let Overlay::SelectionMarquee { enclosed, pointer, .. } = o {
                let (current, other) = badge_rule(*enclosed);
                hud(cx, badge_at(&space.view, *pointer), &[(Label::from(current), format!("E: {other}"))]);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Paints the marks of `overlays` through `draw` in `view`; the marquees, shared paths and distances are controls
/// of their own and paint nothing here.
// PLAN: contract cv.S14 gives `paint(draw, view, overlays, options)`; the pill, ink and ring colours come from the theme, so the style goes too.
fn paint(draw: &Draw<'_>, view: &CanvasView, overlays: &[Overlay], options: &PathOptionsSettings, style: &ToolMarksStyle) {
    for o in overlays {
        match o {
            Overlay::Path { path, color, width, dashed } => {
                for (points, closed) in path_runs(view, path) {
                    stroke_run(draw, points, closed, *dashed, DataWidth::points(*width), data(*color));
                }
            }
            Overlay::Line { a, b, color, dashed: true } => {
                let pattern = draw.dashes();
                let margin = pattern.dash + pattern.gap + draw.width(StrokeWidth::Thin);
                if let Some((run, from)) = visible_run(view, *a, *b, view.rect.expand(margin)) {
                    draw.dashed(&run, dashes_from(pattern, from), StrokeWidth::Thin, data(*color));
                }
            }
            Overlay::Line { a, b, color, dashed: false } => {
                stroke_run(draw, vec![view.to_screen(*a), view.to_screen(*b)], false, false, StrokeWidth::Thin, data(*color));
            }
            Overlay::GridLine { a, b, color } => {
                stroke_run(draw, vec![view.to_screen(*a), view.to_screen(*b)], false, false, StrokeWidth::Thin, data_rgba(*color));
            }
            Overlay::Anchor { p, color, filled, size } => {
                let size = options.anchor_size.unwrap_or(*size);
                let shape = PaintShape::Rect(ScreenRect::from_center_size(view.to_screen(*p), Vec2::splat(size)), CornerRadius::ZERO);
                marker(draw, &shape, data(*color), *filled);
            }
            Overlay::CurvePoint { p, color, filled, size } => {
                let size = options.anchor_size.unwrap_or(*size);
                marker(draw, &PaintShape::Circle(view.to_screen(*p), size / 2.0), data(*color), *filled);
            }
            Overlay::Handle { p, color } => handle_end(draw, view.to_screen(*p), data(*color), options, style),
            // The pasteboard names an artboard, so its label is not drawn twice.
            Overlay::Label { text, .. } if names_an_artboard(text) => {}
            Overlay::Label { p, text, color } => {
                let run = TextRun::new(crate::panels::label_or_name(text, true).to_string()).role(TypeRole::Caption).data_color(data(*color));
                let layout = draw.layout(&run, f32::INFINITY);
                draw.galley(view.to_screen(*p) + Vec2::new(8.0, -14.0), &layout);
            }
            Overlay::Highlight { quad, color } => {
                draw.data_fill(&PaintShape::Polygon(quad.iter().map(|q| view.to_screen(*q)).collect()), data_rgba(*color));
            }
            Overlay::Measure { p, text } => {
                let layout = draw.layout(&TextRun::new(text.as_str()).role(TypeRole::Caption).color(style.ink), f32::INFINITY);
                let at = view.to_screen(*p) + BADGE_OFFSET;
                draw.fill(&PaintShape::Rect(ScreenRect::from_min_size(at, layout.size() + 2.0 * style.pad), style.radius), style.dark);
                draw.galley(at + style.pad, &layout);
            }
            Overlay::Swatch { p, color, selected } => {
                // A white disc under the colour shows its opacity; a dark rim keeps it readable on any art.
                let c = view.to_screen(*p);
                draw.data_fill(&PaintShape::Circle(c, CHIP_RIM), DataColor::new(Color32::WHITE));
                draw.data_fill(&PaintShape::Circle(c, CHIP), data_rgba(*color));
                draw.stroke(&PaintShape::Circle(c, CHIP_RIM), StrokeWidth::Thin, style.dark);
                if *selected {
                    draw.stroke(&PaintShape::Circle(c, CHIP_RING), StrokeWidth::Medium, style.accent);
                }
            }
            // Controls of their own, or a tip: nothing to paint here.
            Overlay::Marquee(_)
            | Overlay::SelectionMarquee { .. }
            | Overlay::SharedPath { .. }
            | Overlay::Distance { .. }
            | Overlay::HoverText { .. } => {}
        }
    }
}

/// Strokes `points`, closed back to the first when `closed`, solid or in the theme's dashes.
fn stroke_run(
    draw: &Draw<'_>,
    mut points: Vec<Pos2>,
    closed: bool,
    dashed: bool,
    width: impl Into<craft::ui::primitives::DataStrokeWidth>,
    color: DataColor,
) {
    if closed {
        points.extend(points.first().copied());
    }
    if dashed {
        draw.dashed(&points, draw.dashes(), width, color);
    } else {
        draw.stroke(&PaintShape::Path(points), width, color);
    }
}

/// The part inside `clip` of the screen line that `view` draws from document point `a` to `b`, with how far along the
/// whole line that part starts, in points. The line is clipped in `f64`, so a line millions of points long keeps its
/// part on the canvas. None when no part of it is inside `clip`, or when an end does not map to a finite point.
fn visible_run(view: &CanvasView, a: vectorcraft_geom::Point, b: vectorcraft_geom::Point, clip: ScreenRect) -> Option<([Pos2; 2], f64)> {
    let affine = view.affine();
    let (a, b) = (affine * a, affine * b);
    if !(a.is_finite() && b.is_finite()) {
        return None;
    }
    let screen = |p: vectorcraft_geom::Point| Pos2::new(p.x as f32, p.y as f32);
    // Halved, so ends near the largest `f64` do not overflow.
    let half = b.to_vec2() * 0.5 - a.to_vec2() * 0.5;
    let length = half.hypot();
    if length == 0.0 {
        let c = screen(a);
        return clip.contains(c).then_some(([c, c], 0.0));
    }
    let u = half / length;
    // The run is measured from the point of the line nearest the middle of `clip`, so the clipped ends keep their
    // precision when the line's ends are far off.
    // That point is found from the line's distance to the middle, taken from the cross product of its ends, so a line
    // whose ends are far off and not on one diagonal keeps its place on the canvas.
    let c = vectorcraft_geom::Point::new(f64::from(clip.center().x), f64::from(clip.center().y));
    let offset = (half.cross(c.to_vec2()) + 2.0 * exact_cross(a.to_vec2() * 0.5, b.to_vec2() * 0.5)) / length;
    let origin = if offset.is_finite() { c - vectorcraft_geom::Vec2::new(-u.y, u.x) * offset } else { a + u * (c - a).dot(u) };
    let (mut s0, mut s1) = ((a - origin).dot(u), (b - origin).dot(u));
    let start = s0;
    let axes = [(origin.x, u.x, f64::from(clip.left()), f64::from(clip.right())), (origin.y, u.y, f64::from(clip.top()), f64::from(clip.bottom()))];
    for (at, step, lo, hi) in axes {
        if step == 0.0 {
            if at < lo || at > hi {
                return None;
            }
        } else {
            let (enter, leave) = ((lo - at) / step, (hi - at) / step);
            s0 = s0.max(enter.min(leave));
            s1 = s1.min(enter.max(leave));
        }
    }
    if s0 > s1 {
        return None;
    }
    Some(([screen(origin + u * s0), screen(origin + u * s1)], s0 - start))
}

/// `p.x * q.y - p.y * q.x`, with the rounding error of the second product added back (Kahan's method), so two far-off
/// points whose products nearly cancel keep the small difference between them.
fn exact_cross(p: vectorcraft_geom::Vec2, q: vectorcraft_geom::Vec2) -> f64 {
    let w = p.y * q.x;
    let error = (-p.y).mul_add(q.x, w);
    p.x.mul_add(q.y, -w) + error
}

/// `pattern` for a run that starts `from` points along its line, so its dashes fall where the whole line has them.
fn dashes_from(pattern: Dashes, from: f64) -> Dashes {
    let period = f64::from(pattern.dash) + f64::from(pattern.gap);
    if !(period.is_finite() && period > 0.0 && from.is_finite()) {
        return pattern;
    }
    let phase = (f64::from(pattern.phase) - from).rem_euclid(period);
    pattern.phase(phase as f32)
}

/// An anchor or curve point: filled in `color` when selected, else a white body with a `color` edge.
fn marker(draw: &Draw<'_>, shape: &PaintShape, color: DataColor, filled: bool) {
    draw.data_fill(shape, if filled { color } else { DataColor::new(Color32::WHITE) });
    if !filled {
        draw.stroke(shape, StrokeWidth::Thin, color);
    }
}

/// A direction handle's end at `c`: a solid dot, a hollow one or a larger solid one.
fn handle_end(draw: &Draw<'_>, c: Pos2, color: DataColor, options: &PathOptionsSettings, style: &ToolMarksStyle) {
    let r = options.anchor_size.map_or(style.handle, |size| size / 2.0);
    match options.handles {
        HandleStyle::Hollow => marker(draw, &PaintShape::Circle(c, r), color, false),
        HandleStyle::Large => draw.data_fill(&PaintShape::Circle(c, r * 1.5), color),
        _ => draw.data_fill(&PaintShape::Circle(c, r), color),
    }
}

/// The tooltip discs of the tools' [`Overlay::HoverText`] regions on the screen under `view`, with their English
/// text; none while `quiet`. A disc without a finite, positive size is dropped.
fn tip_regions(lists: &[&[Overlay]], view: &CanvasView, quiet: bool) -> Vec<(HitShape, &'static str)> {
    if quiet {
        return vec![];
    }
    let discs = lists.iter().flat_map(|list| list.iter());
    discs
        .filter_map(|o| {
            let Overlay::HoverText { center, radius, text } = o else { return None };
            let (c, r) = (view.to_screen(*center), (*radius * view.zoom) as f32);
            (r.is_finite() && r > 0.0 && c.is_finite()).then(|| (HitShape::circle(c, r), *text))
        })
        .collect()
}

/// The style of the hover tips: none of its own.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct HoverTipsStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The tooltips of the tool's [`Overlay::HoverText`] regions. Each region is a hover-only part, so the tool keeps its
/// clicks and drags, and the framework shows its text after the usual delay and not during a press. The parts lie
/// under the artboard names of the pasteboard, which a press takes first, and are not declared while a gesture runs
/// or Space lends a view tool, where a click does something else.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(canvas_control)]
struct HoverTips<'a> {
    #[common]
    common: Common,
    lists: [&'a [Overlay]; 2],
    quiet: bool,
}

impl Component for HoverTips<'_> {
    const ROLE: Role = Role::None;
    const STOPS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = HoverTipsStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: HoverTipsStyleKey) -> HoverTipsStyle {
        HoverTipsStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn parts(&mut self, _: ScreenRect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        for (i, (shape, text)) in tip_regions(&self.lists, &space.view, self.quiet).into_iter().enumerate() {
            cx.part(TIP.at(i as u64), shape)
                .sense(Sense::HOVER)
                .priority(HitPriority::new(HitLevel::Background, 1))
                .tooltip(Label::from(crate::i18n::t(text).to_string()))
                .pointer_only("a hint for the pointer; the tool's own marks carry the same facts");
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[cfg(test)]
mod tests;
