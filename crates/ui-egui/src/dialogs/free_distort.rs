//! Free Distort's dialog body (Effect › Distort & Transform › Free Distort…): a box in the
//! proportions of the first selected object's bounds with a handle on each of its four corners.
//! Dragging a handle moves that corner. A handle focused by a click or Tab moves with the arrow
//! keys, and ten times as far with Shift. Reset puts every corner back on the box. The corners are
//! the effect's `corners` field, which the effect dialog previews and applies.

use egui::{Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2, vec2};
use serde_json::{Value, json};

use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The Free Distort effect.
pub(crate) const FREE_DISTORT: &str = "distort.freeDistort";

/// The corners on the box: top left, top right, bottom right and bottom left, in unit box
/// coordinates.
pub(super) const IDENTITY: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// The area the handles move in.
const PAD: Vec2 = vec2(260.0, 200.0);
/// The space between the box and the edges of the area.
const MARGIN: f32 = 40.0;
/// The drawn size of a handle.
const HANDLE: f32 = 9.0;
/// The size of the square that takes a press on a handle.
const GRAB: f32 = 16.0;
/// How far an arrow key moves a corner, as a share of the box's side.
const STEP: f64 = 0.01;

/// The handles' names for screen readers, in the order of the corners.
fn names() -> [&'static str; 4] {
    [tl!("Top Left"), tl!("Top Right"), tl!("Bottom Right"), tl!("Bottom Left")]
}

/// The corners in `d.fields["corners"]`. A missing or malformed corner reads as its place on the
/// box, as the effect reads it.
pub(super) fn corners(d: &Dialog) -> [[f64; 2]; 4] {
    let mut c = IDENTITY;
    if let Some(list) = d.fields.get("corners").and_then(Value::as_array) {
        for (slot, v) in c.iter_mut().zip(list) {
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
fn aspect(app: &VectorcraftApp) -> f32 {
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

/// The box, the handles and Reset. Changes go to `d.fields["corners"]`.
pub(super) fn fields(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let before = corners(d);
    let mut c = before;
    let (pad, _) = ui.allocate_exact_size(PAD, Sense::hover());
    let inner = pad.shrink(MARGIN);
    let aspect = aspect(app);
    let size = if aspect >= inner.width() / inner.height() {
        vec2(inner.width(), inner.width() / aspect)
    } else {
        vec2(inner.height() * aspect, inner.height())
    };
    let bx = Rect::from_center_size(inner.center(), size);
    let to_screen = |[x, y]: [f64; 2]| Pos2::new(bx.left() + x as f32 * bx.width(), bx.top() + y as f32 * bx.height());
    let to_unit = |p: Pos2| [f64::from((p.x - bx.left()) / bx.width()), f64::from((p.y - bx.top()) / bx.height())];
    let (lo, hi) = (to_unit(pad.min), to_unit(pad.max));
    let names = names();
    // Per handle: whether it is held or hovered, and whether it has the keyboard focus.
    let mut states = [(false, false); 4];
    for (i, corner) in c.iter_mut().enumerate() {
        let id = ui.id().with(("free-distort-corner", i));
        let at = pad.clamp(to_screen(*corner));
        let resp = ui.interact(Rect::from_center_size(at, Vec2::splat(GRAB)), id, Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), names[i]));
        if resp.drag_started() || resp.clicked() {
            resp.request_focus();
        }
        if resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            // The offset from the press to the handle's center, kept for the drag.
            ui.data_mut(|m| m.insert_temp(id, at - origin));
        }
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let grip: Vec2 = ui.data(|m| m.get_temp(id)).unwrap_or_default();
            *corner = to_unit(pad.clamp(p + grip)).map(round);
        }
        if resp.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
            });
            for (key, dx, dy) in [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)] {
                // Shift first: a key without modifiers also matches the key with Shift.
                for (modifiers, step) in [(Modifiers::SHIFT, STEP * 10.0), (Modifiers::NONE, STEP)] {
                    if ui.input_mut(|i| i.consume_key(modifiers, key)) {
                        // A corner stays inside the area, or, when it lies outside, no farther out.
                        let [x, y] = &mut *corner;
                        for (v, d, lo, hi) in [(x, dx, lo[0], hi[0]), (y, dy, lo[1], hi[1])] {
                            *v = round((*v + d * step).clamp(lo.min(*v), hi.max(*v)));
                        }
                    }
                }
            }
        }
        states[i] = (resp.dragged() || resp.hovered(), resp.has_focus());
        if resp.dragged() || resp.hovered() {
            ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
    }
    ui.add_space(4.0);
    let reset = ui
        .add_enabled_ui(c != IDENTITY, |ui| {
            let r = widgets::flat_button(ui, tl!("Reset"), 64.0);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Reset")));
            r
        })
        .inner;
    if reset.clicked() {
        c = IDENTITY;
    }
    paint(ui, pad, bx, c.map(to_screen), states);
    if c != before {
        d.fields.insert("corners".into(), json!(c));
    }
}

/// The pad: the box at rest, the distorted box with a grid across it, and the handles (filled in
/// the accent color while focused).
fn paint(ui: &egui::Ui, pad: Rect, bx: Rect, q: [Pos2; 4], states: [(bool, bool); 4]) {
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter_at(pad);
    painter.rect_filled(pad, 2.0, t.input);
    painter.rect_stroke(bx, 0.0, Stroke::new(1.0, t.text_dim.gamma_multiply(0.5)), egui::StrokeKind::Middle);
    let at = |u: f32, v: f32| q[0].lerp(q[1], u).lerp(q[3].lerp(q[2], u), v);
    let faint = Stroke::new(1.0, t.text_dim.gamma_multiply(0.6));
    for k in 1..4 {
        let f = k as f32 / 4.0;
        painter.line_segment([at(f, 0.0), at(f, 1.0)], faint);
        painter.line_segment([at(0.0, f), at(1.0, f)], faint);
    }
    painter.add(egui::Shape::closed_line(q.to_vec(), Stroke::new(1.5, t.text)));
    for (p, (active, focused)) in q.into_iter().zip(states) {
        let r = Rect::from_center_size(pad.clamp(p), Vec2::splat(HANDLE));
        painter.rect_filled(r, 1.0, if focused { t.accent } else { t.panel });
        painter.rect_stroke(r, 1.0, Stroke::new(if active { 2.0 } else { 1.0 }, t.text), egui::StrokeKind::Middle);
        if focused {
            painter.rect_stroke(r.expand(3.0), 2.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);
        }
    }
    painter.rect_stroke(pad, 2.0, Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
}
