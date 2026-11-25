//! Free Distort's dialog body (Effect › Distort & Transform › Free Distort…): a box in the
//! proportions of the first selected object's bounds with a handle on each of its four corners, as
//! the transform box's Distort mode draws them. Dragging a handle moves that corner; the keys of
//! the box move the picked one (an arrow a hundredth of the box side, ten times as far with Shift).
//! Reset puts every corner back on the box. The corners are the effect's `corners` field, which the
//! effect dialog previews and applies.

use craft::behavior::CanvasView;
use craft::behavior::geometry::quad::{BoxRules, BoxState, TransformMode};
use craft::behavior::kurbo::{Point, Rect as DocRect};
use craft::draw::ui::transform::TransformBox;
use craft::ui::buttons::Button;
use craft::ui::canvas::Canvas;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Picture;
use craft::ui::layout_components::Stack;
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param};
use craft::ui::primitives::{ContentPainterCx, PaintShape};
use craft::ui::runtime::env::Env;
use egui::{Response, Sense, vec2};
use serde_json::{Value, json};

use super::effect_adjust::json_value;
// PLAN: assumed `EffectCx` is declared in `effect_params.rs` (ef.C3, effects-w4).
use super::effect_params::EffectCx;
use crate::VectorcraftApp;

/// The Free Distort effect.
pub(crate) const FREE_DISTORT: &str = "distort.freeDistort";

/// The corners on the box: top left, top right, bottom right and bottom left, in unit box
/// coordinates.
pub(super) const IDENTITY: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// The box's side in document units: one unit is a hundredth of the side, the step of an arrow key.
const SIDE: f64 = 100.0;
/// The space between the box and the edge of the pad in document units: the share of its side
/// the box always left (40 of 180 points).
const MARGIN: f64 = SIDE * 40.0 / 180.0;
/// How many cells the grid across the distorted box has along each side.
const GRID: u32 = 4;

/// The corners in `fields["corners"]`. A missing or malformed corner reads as its place on the
/// box, as the effect reads it.
pub(super) fn corners(fields: &DialogFields) -> [[f64; 2]; 4] {
    let mut c = IDENTITY;
    if let Some(Value::Array(list)) = json_value(fields, "corners") {
        for (slot, v) in c.iter_mut().zip(&list) {
            if let Some(a) = v.as_array()
                && let (Some(x), Some(y)) = (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64))
                && x.is_finite()
                && y.is_finite()
            {
                *slot = [x, y];
            }
        }
    }
    c
}

/// The width of the first selected object's bounds over their height, limited to 1/8 to 8; 1 when
/// nothing is selected or the bounds are empty.
pub(super) fn aspect(app: &VectorcraftApp) -> f32 {
    let bounds = app.session.active().and_then(|st| st.selection.objects.first().and_then(|id| st.doc.node(*id))).and_then(|n| n.geometric_bounds());
    match bounds {
        Some(b) if b.width() > 0.0 || b.height() > 0.0 => (b.width().max(1e-9) / b.height().max(1e-9)).clamp(0.125, 8.0) as f32,
        _ => 1.0,
    }
}

/// A coordinate kept to four decimals (a hundredth of a percent of the box).
fn round(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

/// The row of the Free Distort dialog: the box with its handles and Reset.
pub(super) fn params(cx: &EffectCx, _fields: &DialogFields) -> Vec<Param> {
    let aspect = cx.aspect;
    vec![Param::custom("corners", tl!("Free Distort"), move |ui, fields, _| pad(ui, fields, aspect)).full_width()]
}

/// A corner of the unit box as a document point.
fn doc_point([x, y]: [f64; 2]) -> Point {
    Point::new(x * SIDE, y * SIDE)
}

/// The corners the box's `now` moves `before` to. A corner the box did not move stays as it was, so
/// a click that moved nothing changes nothing. A moved corner stays inside the document `shown`,
/// or, when it lies outside already, goes no farther out.
fn moved(before: [[f64; 2]; 4], given: [Point; 4], now: [Point; 4], shown: DocRect) -> [[f64; 2]; 4] {
    let (lo, hi) = ([shown.x0 / SIDE, shown.y0 / SIDE], [shown.x1 / SIDE, shown.y1 / SIDE]);
    let mut c = before;
    for (i, corner) in c.iter_mut().enumerate() {
        let (Some(&was), Some(&to)) = (given.get(i), now.get(i)) else { continue };
        if (to - was).hypot() <= 1e-9 {
            continue;
        }
        let target = [to.x / SIDE, to.y / SIDE];
        for (k, v) in corner.iter_mut().enumerate() {
            // `max` then `min`, as the bounds may be reversed by a degenerate view.
            *v = round(target[k].max(lo[k].min(*v)).min(hi[k].max(*v)));
        }
    }
    c
}

/// The pad: the box at rest, the distorted box with a grid across it and the handles, drawn through
/// `view`.
fn paint(cx: &ContentPainterCx<'_>, view: &CanvasView, quad: [Point; 4]) {
    let screen = |p: Point| view.to_screen(p);
    let q = quad.map(screen);
    let at = |u: f32, v: f32| q[0].lerp(q[1], u).lerp(q[3].lerp(q[2], u), v);
    cx.outline(&PaintShape::Polygon(IDENTITY.map(|c| screen(doc_point(c))).to_vec()));
    for k in 1..GRID {
        let f = k as f32 / GRID as f32;
        cx.outline(&PaintShape::Path(vec![at(f, 0.0), at(f, 1.0)]));
        cx.outline(&PaintShape::Path(vec![at(0.0, f), at(1.0, f)]));
    }
    cx.outline(&PaintShape::Polygon(q.to_vec()));
}

/// The box, the handles and Reset. Changes go to `fields["corners"]`.
fn pad(ui: &mut egui::Ui, fields: &mut DialogFields, aspect: f64) -> Response {
    let before = corners(fields);
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    let (least_width, height) = {
        let plot = &Env::of(ui).tokens.size.plot;
        (plot.width, plot.height)
    };
    let mut after = before;
    let mut reset = false;
    let shown = Stack::new().fill_width().show(ui, |ui, _| {
        let (room, _) = ui.allocate_exact_size(vec2(ui.available_width().max(least_width), height), Sense::hover());
        // The document is the box with a margin round it; its units are `aspect` times as wide as tall.
        let extent = SIDE + 2.0 * MARGIN;
        let zoom = (f64::from(room.width()) / (extent * aspect)).min(f64::from(room.height()) / extent);
        let view = CanvasView::new(room, Point::new(SIDE / 2.0, SIDE / 2.0), zoom).aspect(aspect);
        let quad = before.map(doc_point);
        let mut state = BoxState::from_rect(DocRect::new(0.0, 0.0, SIDE, SIDE));
        state.quad = quad;
        let canvas = Canvas::new(tl!("Free Distort")).view(view).show_at(ui, room, |ui, body| {
            Picture::painter("", ThumbAspect::SQUARE, move |cx, _| paint(cx, &view, quad)).show_at(ui, body.rect);
            TransformBox::new(tl!("Free Distort"), Bind::given(state), DocRect::new(0.0, 0.0, SIDE, SIDE), 0.0)
                .rules(BoxRules::in_mode(TransformMode::Distort))
                .reference_point(false)
                .inside_moves(false)
                .session_commit(false)
                .show_at(ui, body.rect)
        });
        if let Some(output) = canvas.inner
            && let Some(edit) = output.edit.value()
        {
            after = moved(before, quad, edit.quad, view.visible());
        }
        reset = Button::new(tl!("Reset")).enabled(after != IDENTITY).show(ui).clicked();
    });
    if reset {
        after = IDENTITY;
    }
    if after != before {
        fields.set("corners", FieldValue::Json(json!(after)));
    }
    shown.output
}
