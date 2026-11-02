use super::color::{Live, pointer_phase};
use super::dropdowns::combo;
use super::labels::dim_label;
use super::numeric::{range_field, slider};
use super::{BlendMode, Rect, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, icons, id_with, vec2};

/// Whether the blend-mode list draws a separator above `BlendMode::ALL[i]`: where a
/// [`BlendMode::group`] starts.
pub fn blend_separator_before(i: usize) -> bool {
    i > 0 && i < BlendMode::ALL.len() && BlendMode::ALL[i].group() != BlendMode::ALL[i - 1].group()
}

/// The blend-mode dropdown, its groups separated (Normal | darken | lighten | contrast | inversion
/// | component modes). `None` (objects that differ) shows blank. Returns the chosen mode.
pub fn blend_dropdown(ui: &mut Ui, id: impl std::hash::Hash, current: Option<BlendMode>, width: f32) -> Option<BlendMode> {
    combo(ui, id, current.map_or("", |m| tl!(m.label())), width, |ui| {
        let mut chosen = None;
        for (i, m) in BlendMode::ALL.into_iter().enumerate() {
            if blend_separator_before(i) {
                ui.separator();
            }
            if ui.selectable_label(Some(m) == current, tl!(m.label())).clicked() {
                chosen = Some(m);
            }
        }
        chosen
    })
}

/// The blend mode an effect parameter names (`"mode": "multiply"`), which editors show as a
/// [`blend_param_dropdown`].
pub fn blend_param(key: &str, value: &serde_json::Value) -> Option<BlendMode> {
    value.as_str().filter(|_| key == "mode").and_then(BlendMode::parse)
}

/// [`blend_dropdown`] for a blend-mode parameter; returns the chosen mode's parameter value.
pub fn blend_param_dropdown(ui: &mut Ui, id: impl std::hash::Hash, current: BlendMode) -> Option<serde_json::Value> {
    blend_dropdown(ui, id, Some(current), 120.0).map(|m| serde_json::Value::String(m.label().to_ascii_lowercase()))
}

/// An edit made with [`opacity_blend`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransparencyEdit {
    Blend(BlendMode),
    /// Opacity in percent, with the slider's drag phase (`Released` for a typed value).
    Opacity(f64, Live),
}

/// The transparency controls shared by the Transparency panel and the Appearance panel's Opacity
/// popups: the grouped blend-mode dropdown, the opacity percent field and its slider popup.
/// `opacity` (0..1) and `blend` are `None` where the objects differ (shown blank).
pub fn opacity_blend(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy,
    opacity: Option<f32>,
    blend: Option<BlendMode>,
    enabled: bool,
) -> Option<TransparencyEdit> {
    let t = Tokens::get(ui.ctx());
    let mut edit = None;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            if let Some(m) = blend_dropdown(ui, (id, "blend"), blend, 104.0) {
                edit = Some(TransparencyEdit::Blend(m));
            }
        });
        dim_label(ui, "Opacity:");
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_enabled_ui(enabled, |ui| {
            if let Some(o) = range_field(ui, (id, "opacity"), opacity.map(|o| o as f64 * 100.0), 0.0..=100.0, "%", 0, 50.0) {
                edit = Some(TransparencyEdit::Opacity(o, Live::Released));
            }
        });
        let (r, resp) = ui.allocate_exact_size(vec2(18.0, 26.0), if enabled { Sense::click() } else { Sense::hover() });
        ui.painter().rect_stroke(r, 2, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
        icons::paint(ui, "chevron-right", r.shrink2(vec2(3.0, 7.0)), if enabled { t.icon } else { t.text_disabled });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled && ui.is_enabled(), tl!("Opacity")));
        egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let mut o = opacity.unwrap_or(1.0) * 100.0;
            let r = slider(ui, egui::Slider::new(&mut o, 0.0..=100.0).show_value(false));
            let phase = if r.drag_stopped() || (r.changed() && !r.dragged()) {
                Live::Released
            } else if r.changed() {
                Live::Dragging
            } else {
                return;
            };
            edit = Some(TransparencyEdit::Opacity(o.round() as f64, phase));
        });
    });
    edit
}

/// The 3×3 reference point locator: nine squares, the eight outer ones joined by a line around
/// the box (the centre one stands alone), the current one filled. Returns a new index when clicked.
pub fn reference_point(ui: &mut Ui, current: usize) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    // 5 pt squares with 4 pt gaps; the origin is snapped to whole points so the 1 pt lines through
    // the squares' centres stay crisp.
    let (sq, step) = (5.0, 9.0);
    let size = 2.0 * step + sq;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let o = rect.min.round();
    let centre = |i: usize| o + Vec2::new(sq / 2.0 + (i % 3) as f32 * step, sq / 2.0 + (i / 3) as f32 * step);
    let line = Stroke::new(1.0, t.text_dim);
    ui.painter().rect_stroke(Rect::from_two_pos(centre(0), centre(8)), 0.0, line, StrokeKind::Middle);
    let mut out = None;
    // Screen readers name the squares left to right and top to bottom.
    let names = [
        tl!("Top Left"),
        tl!("Top"),
        tl!("Top Right"),
        tl!("Left"),
        tl!("Center"),
        tl!("Right"),
        tl!("Bottom Left"),
        tl!("Bottom"),
        tl!("Bottom Right"),
    ];
    for (i, name) in names.into_iter().enumerate() {
        let r = Rect::from_center_size(centre(i), Vec2::splat(sq));
        let resp = ui.interact(r.expand(2.0), ui.scope_id().with(("refpt", i)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), i == current, name));
        if resp.clicked() {
            out = Some(i);
        }
        if i == current {
            ui.painter().rect_filled(r, 0.0, t.text);
        } else {
            ui.painter().rect_filled(r, 0.0, t.panel);
            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, if resp.hovered() { t.text } else { t.text_dim }), StrokeKind::Inside);
        }
    }
    out
}

/// An angle dial `size` points across: a circle with a hand from its centre at `angle` (degrees,
/// counter-clockwise from 3 o'clock). Pressing or dragging points the hand at the pointer, in whole
/// degrees (-179..180), Shift in 45° steps. Returns the new angle.
pub fn angle_dial(ui: &mut Ui, id: impl std::hash::Hash, angle: f64, size: f32) -> Option<f64> {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let resp = ui.interact(rect, id_with(ui.scope_id(), id), Sense::click_and_drag());
    let (c, r) = (rect.center(), size / 2.0 - 1.0);
    let ring = if resp.hovered() || resp.dragged() { t.text } else { t.input_border };
    ui.painter().circle(c, r, t.input, Stroke::new(1.0, ring));
    let a = (angle as f32).to_radians();
    ui.painter().line_segment([c, c + vec2(a.cos(), -a.sin()) * (r - 3.0)], Stroke::new(1.5, t.text));
    ui.painter().circle_filled(c, 2.0, t.text);
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), angle, tl!("Angle")));
    let (p, _) = pointer_phase(&resp)?;
    let v = p - c;
    if v.length() < 2.0 {
        return None;
    }
    let step = if ui.input(|i| i.modifiers.shift) { 45.0 } else { 1.0 };
    let deg = ((-v.y).atan2(v.x).to_degrees() as f64 / step).round() * step;
    // atan2 gives -180..180: -180 shows as 180.
    let deg = if deg <= -180.0 { deg + 360.0 } else { deg };
    (deg != angle).then_some(deg)
}
