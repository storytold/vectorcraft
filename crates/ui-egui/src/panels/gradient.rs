//! Gradient panel: the gradient thumbnail (drag it onto art; its menu lists the document's
//! gradient swatches and Save to Swatches), type buttons, the Fill/Stroke proxy, angle and aspect
//! ratio fields with preset menus, Reverse, the gradient slider and the selected stop's fields.
//!
//! On the slider: click below the ramp to add a stop, drag a stop to move it (Alt drags a copy;
//! Alt-dropping it on another stop swaps their colours), drag it off to remove it, double-click it
//! for the stop popover, and drag a diamond to move a midpoint (a selected diamond's midpoint
//! shows in Location). Dropping a colour swatch on the ramp adds a stop of that colour, or
//! recolours the stop it lands on; a global or spot colour (or a tint of one) stays linked, so
//! editing the swatch recolours the stop. The thumbnail menu marks the gradient swatch the
//! gradient was applied from. Hide Options leaves the thumbnail, the proxy and the slider.
//!
//! With the Stroke proxy in front, the Stroke buttons lay a linear or radial gradient within the
//! stroke (placed on the page like a fill's), along it or across it (angle and aspect ratio then
//! don't apply).
//!
//! A freeform gradient shows its section instead of the angle, aspect and slider: the Draw
//! toggle (Points or Lines: how the Gradient tool adds points) and the selected point's colour
//! (edited in the Color panel), opacity and spread, with Delete Point.

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::gradient::{
    MIN_STOPS, duplicate_stop, insert_stop, midpoint_from_pos, midpoint_pos, move_stop, remove_stop, set_midpoint, swap_stop_colors,
};
use vectorcraft_color::{Color, Freeform, FreeformMode, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{NodeId, StrokeGradientMode};

use super::{active_paint, live_run, pstate, set_pstate};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets::{self, Live, PanelDrag, menu_item};
use crate::{VectorcraftApp, icons};

/// Dragging a stop this far below the ramp removes it.
const REMOVE_DISTANCE: f32 = 28.0;
/// The Angle field's preset menu, degrees.
const ANGLE_PRESETS: [f64; 9] = [-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0];
/// The Aspect Ratio field's preset menu, percent.
const ASPECT_PRESETS: [f64; 9] = [10.0, 25.0, 50.0, 75.0, 100.0, 150.0, 200.0, 300.0, 400.0];
/// Panel-state key of Show/Hide Options.
const HIDE_OPTIONS: &str = "gradient-hide-options";
/// Panel-state key of the selected midpoint diamond.
const MID_SELECTED: &str = "grad-mid";

/// Select stop `i` (the session's selected stop, shared with the Gradient tool's annotator, the
/// Color panel and agents), or none.
fn select_stop(app: &mut VectorcraftApp, i: Option<usize>) {
    app.run("gradient.selectStop", json!({ "index": i })).ok();
}

// ---------- pure ramp math (unit-tested) ----------

/// Offset (0..1) of an x position on a ramp spanning `left..left + width`.
pub fn x_to_offset(x: f32, left: f32, width: f32) -> f32 {
    ((x - left) / width.max(1.0)).clamp(0.0, 1.0)
}

/// Stops as `paint.editGradient` JSON.
pub use vectorcraft_tools::params::stops_json;

/// A colour dropped on the ramp: recolours stop `on` (fully opaque), or adds an opaque stop of
/// that colour at `offset`, linked to `link` (a global swatch and tint) when given. Returns the
/// stops and the index of the stop that took the colour.
fn drop_color(g: &Gradient, on: Option<usize>, offset: f32, c: Color, link: Option<(String, f32)>) -> (Vec<GradientStop>, usize) {
    let (mut v, i) = match on.filter(|i| *i < g.stops.len()) {
        Some(i) => (g.stops.clone(), i),
        None => insert_stop(g, offset),
    };
    v[i].set_color(c, link);
    v[i].opacity = 1.0;
    (v, i)
}

/// The swatch link a dropped paint's params give a stop: `{swatch, tint?}` of a global or spot
/// colour or a tint swatch (a proxy's tint carries its percentage); none for other colours.
pub(crate) fn dropped_link(app: &VectorcraftApp, params: &Value) -> Option<(String, f32)> {
    let name = params.get("swatch")?.as_str()?;
    let d = &app.session.active()?.doc;
    let (base, own) = d.swatch_link(name)?;
    let tint = params.get("tint").and_then(Value::as_f64).map_or(own, |t| (t / 100.0).clamp(0.0, 1.0) as f32);
    Some((base, tint))
}

/// The stops after dragging stop `i` of `origin` to `offset`: moved, or with `copy` (Alt) a copy
/// left there, or the colours swapped with stop `over` when the copy is dropped on it. Returns
/// the stops and the stop to select.
fn dragged_stops(origin: &[GradientStop], i: usize, offset: f32, copy: bool, over: Option<usize>) -> (Vec<GradientStop>, usize) {
    match (copy, over.filter(|j| *j != i)) {
        (true, Some(j)) => (swap_stop_colors(origin, i, j), i),
        (true, None) => duplicate_stop(origin, i, offset),
        (false, _) => move_stop(origin, i, offset),
    }
}

// ---------- shared edits ----------

/// The Type row: Linear, Radial and Freeform for the paint behind the active proxy (one of them
/// lit when it is a gradient). The Gradient panel's and the Properties panel's.
pub(crate) fn type_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let kind = current(app).map(|g| g.gradient.kind);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Type:"));
        for (k, icon, tip) in [
            (GradientKind::Linear, "dc-grad-linear", tl!("Linear Gradient")),
            (GradientKind::Radial, "dc-grad-radial", tl!("Radial Gradient")),
            (GradientKind::Freeform, "dc-grad-freeform", tl!("Freeform Gradient")),
        ] {
            if widgets::icon_button(ui, icon, tip, kind == Some(k), 24.0).clicked() {
                edit(app, json!({"kind": k.label().to_lowercase()}), Live::Released);
            }
        }
    });
}

/// Set how the Gradient tool adds the points of the freeform gradient behind the active proxy.
fn set_draw_mode(app: &mut VectorcraftApp, mode: FreeformMode) {
    edit(app, json!({ "mode": mode.label().to_lowercase() }), Live::Released);
}

/// The Properties panel's Draw row for a freeform gradient: Points and Lines as radio buttons.
pub(crate) fn draw_radios(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(g) = current(app).filter(|g| g.gradient.kind == GradientKind::Freeform) else { return };
    let mode = shown_points(&g).mode;
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Draw:"));
        for (m, label) in [(FreeformMode::Points, tl!("Points")), (FreeformMode::Lines, tl!("Lines"))] {
            if widgets::radio(ui, label, mode == m, true) && mode != m {
                set_draw_mode(app, m);
            }
        }
    });
}

/// The gradient behind the active proxy.
fn current(app: &VectorcraftApp) -> Option<GradientPaint> {
    match active_paint(app) {
        Paint::Gradient(g) => Some(*g),
        _ => None,
    }
}

/// Edit the gradient behind the active proxy (`paint.editGradient` params).
fn edit(app: &mut VectorcraftApp, params: Value, phase: Live) {
    let mut p = params;
    p["stroke"] = json!(!app.session.fill_active);
    live_run(app, "Gradient", "paint.editGradient", p, phase);
}

/// Write `stops` to the gradient behind the active proxy (live while dragging) and, once
/// released, select stop `select` (the panel, the stop popover and the Color panel share this).
pub(crate) fn set_stops(app: &mut VectorcraftApp, stops: &[GradientStop], select: Option<usize>, phase: Live) {
    edit(app, json!({ "stops": stops_json(stops) }), phase);
    if phase == Live::Released
        && let Some(i) = select
        && app.session.selected_stop() != Some(i)
    {
        select_stop(app, Some(i));
    }
}

/// Save the gradient as a swatch.
fn save_to_swatches(app: &mut VectorcraftApp, g: &GradientPaint) {
    app.run("swatch.new", super::paint_params(&Paint::Gradient(Box::new(g.clone())))).ok();
}

/// The document's gradient swatches (groups included): name and gradient.
fn gradient_swatches(app: &VectorcraftApp) -> Vec<(String, Gradient)> {
    let Some(st) = app.session.active() else { return vec![] };
    let d = &st.doc;
    d.swatches
        .iter()
        .chain(d.swatch_groups.iter().flat_map(|g| g.swatches.iter()))
        .filter_map(|s| match &s.paint {
            Paint::Gradient(g) => Some((s.name.clone(), g.gradient.clone())),
            _ => None,
        })
        .collect()
}

/// The selected midpoint diamond: kept for the object it was picked on, and given up when a stop
/// is selected.
fn selected_mid(app: &VectorcraftApp, ctx: &egui::Context, stops: usize) -> Option<usize> {
    let (i, owner) = pstate::<Option<(usize, Option<NodeId>)>>(ctx, MID_SELECTED)?;
    (app.session.selected_stop().is_none() && i + 1 < stops && owner == first_id(app)).then_some(i)
}

fn first_id(app: &VectorcraftApp) -> Option<NodeId> {
    app.session.active().and_then(|d| d.selection.objects.first().copied())
}

// ---------- UI ----------

/// Drag state of the ramp: which handle is being dragged.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Drag {
    #[default]
    None,
    /// Stop `index`; `copy` (Alt held when the drag began) drags a copy.
    Stop {
        index: usize,
        copy: bool,
    },
    Mid(usize),
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let gp = current(app);
    let is_grad = gp.is_some();
    let g = gp.unwrap_or_else(|| GradientPaint::new(Gradient::default()));
    let kind = g.gradient.kind;
    let hidden: bool = pstate(ui.ctx(), HIDE_OPTIONS);
    // Header: thumbnail + its swatch menu, the type buttons and Edit Gradient.
    ui.horizontal(|ui| {
        thumbnail(app, ui, &g, is_grad);
        if hidden {
            ui.add_space(6.0);
            super::proxy(app, ui, 36.0);
            return;
        }
        ui.add_space(4.0);
        ui.vertical(|ui| {
            type_row(app, ui);
            if widgets::flat_button(ui, tl!("Edit Gradient"), 96.0).clicked() {
                app.select_tool("gradient");
            }
        });
    });
    let freeform = is_grad && kind == GradientKind::Freeform;
    if hidden {
        if !freeform {
            ui.add_space(6.0);
            ramp(app, ui, &g.gradient, is_grad);
        }
        return;
    }
    // How the gradient lies on the stroke behind the Stroke proxy (a linear or radial one).
    let stroke_mode = (!app.session.fill_active && is_grad && !freeform).then(|| app.session.shown_stroke().map(|st| st.gradient_mode)).flatten();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Stroke:"));
        for (m, icon, tip) in [
            (StrokeGradientMode::Within, "dc-grad-stroke-within", tl!("Gradient within stroke")),
            (StrokeGradientMode::Along, "dc-grad-stroke-along", tl!("Gradient along stroke")),
            (StrokeGradientMode::Across, "dc-grad-stroke-across", tl!("Gradient across stroke")),
        ] {
            let on = stroke_mode == Some(m);
            if widgets::icon_button_enabled(ui, icon, tip, on, stroke_mode.is_some(), 22.0).clicked() && !on {
                edit(app, json!({"strokeMode": m.name()}), Live::Released);
            }
        }
    });
    // Along and across, the gradient follows the path: its angle and aspect ratio don't apply.
    let placed = stroke_mode.is_none_or(|m| m == StrokeGradientMode::Within);
    if freeform {
        freeform_section(app, ui, &g);
        return;
    }
    // The proxy beside the angle and aspect ratio fields.
    ui.horizontal(|ui| {
        super::proxy(app, ui, 36.0);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                icons::icon(ui, "rotate-ccw", 15.0, if placed { t.icon } else { t.text_disabled }).on_hover_text(tl!("Angle"));
                let angle = g.geom.map_or(g.angle, |x| x.angle_deg());
                let set = ui
                    .add_enabled_ui(placed, |ui| {
                        widgets::spin_plain(ui, "grad-angle", (angle * 10.0).round() / 10.0, "°", 1, 104.0, 1.0, -180.0, &ANGLE_PRESETS)
                    })
                    .inner;
                if let Some(a) = set {
                    edit(app, json!({"angle": a}), Live::Released);
                }
                if widgets::icon_button_enabled(ui, "dc-reverse", tl!("Reverse Gradient"), false, is_grad, 22.0).clicked() {
                    edit(app, json!({"reverse": true}), Live::Released);
                }
            });
            let radial = kind == GradientKind::Radial && placed;
            ui.horizontal(|ui| {
                icons::icon(ui, "scaling", 15.0, if radial { t.icon } else { t.text_disabled }).on_hover_text(tl!("Aspect Ratio"));
                let asp = g.geom.map_or(100.0, |x| (x.aspect * 1000.0).round() / 10.0);
                let set = ui.add_enabled_ui(radial, |ui| widgets::spin_plain(ui, "grad-aspect", asp, "%", 1, 104.0, 1.0, 0.5, &ASPECT_PRESETS)).inner;
                if let Some(a) = set {
                    edit(app, json!({"aspect": a}), Live::Released);
                }
            });
        });
    });
    ui.add_space(6.0);
    ramp(app, ui, &g.gradient, is_grad);
    ui.add_space(4.0);
    stop_fields(app, ui, &g.gradient, is_grad);
    if !is_grad {
        widgets::dim_label(ui, tl!("Click the ramp or a type button to apply a gradient."));
    }
}

/// The freeform points shown: the gradient's own, else (unplaced) as many as it places, coloured
/// along the stops.
pub(crate) fn shown_points(g: &GradientPaint) -> std::borrow::Cow<'_, Freeform> {
    g.freeform_on(vectorcraft_geom::Rect::new(0.0, 0.0, 1.0, 1.0))
}

/// Run a `paint.freeform.*` command on the paint behind the active proxy.
pub(crate) fn edit_point(app: &mut VectorcraftApp, cmd: &str, mut params: Value) {
    params["stroke"] = json!(!app.session.fill_active);
    live_run(app, "Gradient", cmd, params, Live::Released);
}

/// The freeform section: the proxy beside the Draw toggle, then the selected point's colour,
/// opacity and spread, and Delete Point.
fn freeform_section(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint) {
    let f = shown_points(g);
    let mode = f.mode;
    ui.horizontal(|ui| {
        super::proxy(app, ui, 36.0);
        ui.add_space(4.0);
        widgets::dim_label(ui, tl!("Draw:"));
        for (m, icon, tip) in [
            (FreeformMode::Points, "circle", tl!("Points: clicks add free points")),
            (FreeformMode::Lines, "spline", tl!("Lines: clicks add points joined by a line")),
        ] {
            if widgets::icon_button(ui, icon, tip, mode == m, 24.0).clicked() && mode != m {
                set_draw_mode(app, m);
            }
        }
    });
    ui.add_space(6.0);
    let sel = app.session.selected_freeform_point().filter(|i| *i < f.points.len());
    let point = sel.map(|i| f.points[i]);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Color:"));
        let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        let paint = point.map_or(Paint::None, |p| Paint::solid(p.color));
        widgets::swatch_tile(ui, r, &paint, false, resp.hovered());
        resp.on_hover_text(tl!("Edit the selected point's colour in the Color panel"));
        if let Some(p) = point {
            widgets::dim_label(ui, &p.color.to_hex().to_uppercase());
        }
    });
    ui.horizontal(|ui| {
        ui.add_enabled_ui(sel.is_some(), |ui| {
            for (label, key, value) in [(tl!("Opacity:"), "opacity", point.map(|p| p.opacity)), (tl!("Spread:"), "spread", point.map(|p| p.spread))] {
                widgets::dim_label(ui, label);
                // Blank while no point is selected.
                let shown = value.map(|v| (v as f64 * 1000.0).round() / 10.0);
                if let Some(v) = widgets::mixed_field(ui, ("freeform", key), shown, "%", 1, 54.0)
                    && let Some(i) = sel
                {
                    edit_point(app, "paint.freeform.setPoint", json!({ "index": i, key: v.clamp(0.0, 100.0) / 100.0 }));
                }
            }
        });
        let can_del = sel.is_some() && f.points.len() > 1;
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Point"), false, can_del, 22.0).clicked()
            && let Some(i) = sel
        {
            edit_point(app, "paint.freeform.deletePoint", json!({ "index": i }));
        }
    });
    if sel.is_none() {
        widgets::dim_label(ui, tl!("Click the art with the Gradient tool to add or select points."));
    }
}

/// The gradient thumbnail (click: apply a gradient; drag: onto art) and its menu of gradient
/// swatches with Save to Swatches.
fn thumbnail(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint, is_grad: bool) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::click_and_drag());
    widgets::gradient_chip(ui, r, &g.gradient);
    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    let resp = resp.on_hover_text(tl!("Gradient Fill: click to apply, drag onto art"));
    if resp.clicked() && !is_grad {
        edit(app, json!({}), Live::Released);
    }
    // The dragged copy carries no placement: it fits the art it lands on.
    widgets::drag_source(ui, &resp, || PanelDrag::paint(Paint::Gradient(Box::new(g.clone()))));
    let dr = Rect::from_min_size(r.right_top() + vec2(1.0, 0.0), vec2(14.0, 40.0));
    ui.advance_cursor_after_rect(dr);
    let dresp = ui.interact(dr, ui.id().with("grad-swatches"), Sense::click());
    icons::paint(ui, "chevron-down", Rect::from_center_size(dr.center(), vec2(12.0, 12.0)), if dresp.hovered() { t.text_strong } else { t.icon });
    let dresp = dresp.on_hover_text(tl!("Gradient swatches"));
    egui::Popup::menu(&dresp).show(|ui| {
        ui.set_min_width(200.0);
        let mut chosen = None;
        let swatches = gradient_swatches(app);
        if swatches.is_empty() {
            widgets::dim_label(ui, tl!("No gradient swatches"));
        }
        for (name, grad) in &swatches {
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            // The gradient swatch the gradient was applied from.
            if g.swatch.as_ref() == Some(name) {
                ui.painter().rect_filled(row, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(row, 0.0, t.hover);
            }
            let chip = Rect::from_min_size(row.left_center() + vec2(4.0, -8.0), vec2(16.0, 16.0));
            widgets::gradient_chip(ui, chip, grad);
            ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
            ui.painter().text(chip.right_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.5), t.text);
            if resp.clicked() {
                chosen = Some(name.clone());
            }
        }
        ui.separator();
        if menu_item(ui, tl!("Save to Swatches"), is_grad, false) {
            save_to_swatches(app, g);
        }
        if let Some(name) = chosen {
            super::apply_click(app, ui, json!({ "swatch": name }));
            ui.close();
        }
    });
}

/// The selected stop's colour (click: its popover; the eyedropper samples one from the art),
/// opacity and location (a selected diamond's midpoint instead), and Delete Stop.
fn stop_fields(app: &mut VectorcraftApp, ui: &mut Ui, g: &Gradient, is_grad: bool) {
    let stops = &g.stops;
    let sel = app.session.selected_stop().filter(|i| *i < stops.len() && is_grad);
    let mid = selected_mid(app, ui.ctx(), stops.len()).filter(|_| is_grad);
    let stop = sel.map(|i| &stops[i]);
    if let (Some(i), Some(s)) = (sel, stop) {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Color:"));
            let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            widgets::swatch_tile(ui, r, &Paint::solid(s.color), false, resp.hovered());
            // A linked stop names its swatch and tint.
            let label = match &s.swatch {
                Some(n) => format!("{n} {}%", vectorcraft_color::tint_percent(s.tint)),
                None => s.color.to_hex().to_uppercase(),
            };
            widgets::dim_name(ui, &label);
            if resp.on_hover_text(tl!("Edit the stop")).clicked() {
                open_popover(app, i, r.left_bottom() + vec2(0.0, 4.0));
            }
            ui.add_space(ui.available_width() - 22.0);
            if widgets::icon_button(ui, "pipette", tl!("Eyedropper: click the art to sample the stop's colour"), false, 22.0).clicked() {
                stop_eyedropper(app);
            }
        });
    }
    ui.horizontal(|ui| {
        ui.add_enabled_ui(sel.is_some(), |ui| {
            widgets::dim_label(ui, tl!("Opacity:"));
            if let Some(v) = widgets::plain_field(ui, "grad-op", stop.map_or(100.0, |s| s.opacity as f64 * 100.0), "%", 0, 54.0)
                && let Some(i) = sel
            {
                let mut v2 = stops.clone();
                v2[i].opacity = (v / 100.0).clamp(0.0, 1.0) as f32;
                set_stops(app, &v2, Some(i), Live::Released);
            }
        });
        ui.add_enabled_ui(sel.is_some() || mid.is_some(), |ui| {
            widgets::dim_label(ui, tl!("Location:"));
            let loc = match (mid, stop) {
                (Some(m), _) => stops[m].midpoint as f64 * 100.0,
                (None, Some(s)) => s.offset as f64 * 100.0,
                _ => 0.0,
            };
            if let Some(v) = widgets::plain_field(ui, ("grad-loc", mid.is_some()), loc, "%", 1, 54.0) {
                let v = (v / 100.0) as f32;
                if let Some(m) = mid {
                    set_stops(app, &set_midpoint(stops, m, v), None, Live::Released);
                } else if let Some(i) = sel {
                    let (v2, ni) = move_stop(stops, i, v);
                    set_stops(app, &v2, Some(ni), Live::Released);
                }
            }
        });
        let can_del = sel.is_some() && stops.len() > MIN_STOPS;
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Stop"), false, can_del, 22.0).clicked()
            && let Some(i) = sel
            && let Some(v) = remove_stop(stops, i)
        {
            let next = i.min(v.len() - 1);
            set_stops(app, &v, Some(next), Live::Released);
        }
    });
}

/// The stop's eyedropper: the Eyedropper tool samples the next colour clicked on the art into the
/// selected stop, then hands back to the current tool.
fn stop_eyedropper(app: &mut VectorcraftApp) {
    let back = app.session.tool_id();
    app.select_tool("eyedropper");
    app.session.set_tool_option("stop", &json!(back));
}

/// Open the stop popover for stop `i` at screen position `at`.
fn open_popover(app: &mut VectorcraftApp, i: usize, at: Pos2) {
    select_stop(app, Some(i));
    app.ui.dialog = Some(Dialog::new("gradientStop", json!({ "index": i, "screen": [at.x, at.y], "tab": "color" })));
}

/// The gradient slider: ramp, stops below, midpoint diamonds above.
fn ramp(app: &mut VectorcraftApp, ui: &mut Ui, g: &Gradient, is_grad: bool) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (area, area_resp) = ui.allocate_exact_size(vec2(w, 50.0), Sense::hover());
    let bar = Rect::from_min_size(area.min + vec2(8.0, 10.0), vec2(w - 16.0, 18.0));
    // Checkerboard under the ramp (opacity).
    let cell = 6.0;
    let mut y = bar.top();
    let mut row = 0;
    while y < bar.bottom() {
        let mut x = bar.left();
        let mut col = row % 2;
        while x < bar.right() {
            let r = Rect::from_min_max(pos2(x, y), pos2((x + cell).min(bar.right()), (y + cell).min(bar.bottom())));
            ui.painter().rect_filled(r, 0.0, if col % 2 == 0 { Color32::WHITE } else { Color32::from_gray(204) });
            x += cell;
            col += 1;
        }
        y += cell;
        row += 1;
    }
    let n = (bar.width() / 2.0) as usize;
    for i in 0..n {
        let tt = (i as f32 + 0.5) / n as f32;
        let (c, o) = g.sample(tt);
        let [r, gg, b, a] = c.to_rgba8(o);
        let rr =
            Rect::from_min_size(pos2(bar.left() + i as f32 * bar.width() / n as f32, bar.top()), vec2(bar.width() / n as f32 + 0.5, bar.height()));
        ui.painter().rect_filled(rr, 0.0, Color32::from_rgba_unmultiplied(r, gg, b, a));
    }
    // A colour swatch held over the ramp: outline it as a drop target.
    let droppable = is_grad && area_resp.dnd_hover_payload::<PanelDrag>().is_some_and(|d| d.color().is_some());
    let edge = if droppable { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.border) };
    ui.painter().rect_stroke(bar, 0.0, edge, StrokeKind::Outside);
    let x_of = |o: f32| bar.left() + o * bar.width();
    let offset_at = |p: Pos2| x_to_offset(p.x, bar.left(), bar.width());
    let marker = |o: f32| Rect::from_min_size(pos2(x_of(o) - 6.0, bar.bottom() + 2.0), vec2(12.0, 16.0));
    let sel = app.session.selected_stop();
    let stops = &g.stops;
    let mid_sel = selected_mid(app, ui.ctx(), stops.len());
    let mut drag: Drag = pstate(ui.ctx(), "grad-drag");
    // While dragging, edits are computed against the stops as they were when the drag began (the
    // document shows the preview).
    let origin: Vec<GradientStop> = if drag == Drag::None { stops.clone() } else { pstate::<Vec<GradientStop>>(ui.ctx(), "grad-origin") };
    let origin = if origin.is_empty() { stops.clone() } else { origin };
    // The stop of `origin` other than `i` whose marker is under `p`.
    let stop_under = |p: Pos2, i: usize| origin.iter().enumerate().position(|(j, s)| j != i && marker(s.offset).contains(p));
    let mut changed: Option<(Vec<GradientStop>, Live)> = None;
    // The stop to select once `changed` is applied.
    let mut select: Option<usize> = None;
    let mut pick_mid: Option<usize> = None;
    let mut popover: Option<(usize, Pos2)> = None;
    // The ramp and the strip below it, under the stops and diamonds (interacted first): a click
    // below adds a stop, and without a gradient either applies one.
    let below = Rect::from_min_max(pos2(bar.left(), bar.bottom()), pos2(bar.right(), area.bottom()));
    let add_resp = ui.interact(below, ui.id().with("grad-add"), Sense::click());
    let bar_resp = ui.interact(bar, ui.id().with("grad-bar"), Sense::click());
    if add_resp.hovered() && is_grad {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Copy);
    }
    if !is_grad && (bar_resp.clicked() || add_resp.clicked()) {
        edit(app, json!({}), Live::Released);
    } else if add_resp.clicked()
        && let Some(p) = add_resp.interact_pointer_pos()
    {
        let (v, i) = insert_stop(g, offset_at(p));
        select = Some(i);
        changed = Some((v, Live::Released));
    }
    // Midpoint diamonds.
    for i in 0..stops.len().saturating_sub(1) {
        let Some(p) = midpoint_pos(stops, i) else { continue };
        let c = pos2(x_of(p), bar.top() - 5.0);
        let rect = Rect::from_center_size(c, vec2(10.0, 10.0));
        let resp = ui.interact(rect, ui.id().with(("grad-mid", i)), Sense::click_and_drag());
        let lit = drag == Drag::Mid(i) || mid_sel == Some(i) || resp.hovered();
        let pts = vec![c + vec2(0.0, -4.0), c + vec2(4.0, 0.0), c + vec2(0.0, 4.0), c + vec2(-4.0, 0.0)];
        ui.painter().add(egui::Shape::convex_polygon(pts, if lit { Color32::WHITE } else { t.icon }, Stroke::new(1.0, t.border)));
        if !is_grad {
            continue;
        }
        if resp.clicked() || resp.drag_started() {
            pick_mid = Some(i);
        }
        if resp.drag_started() {
            drag = Drag::Mid(i);
            set_pstate(ui.ctx(), "grad-origin", stops.clone());
        }
        if (resp.dragged() || resp.drag_stopped())
            && let Some(pp) = resp.interact_pointer_pos()
            && let Some(m) = midpoint_from_pos(&origin, i, offset_at(pp))
        {
            let phase = if resp.drag_stopped() { Live::Released } else { Live::Dragging };
            changed = Some((set_midpoint(&origin, i, m), phase));
            if resp.drag_stopped() {
                drag = Drag::None;
            }
        }
    }
    // Stops (house-shaped markers under the ramp).
    let alt = super::alt_held(ui);
    for (i, s) in stops.iter().enumerate() {
        let x = x_of(s.offset);
        let m = marker(s.offset);
        let top = m.top();
        let resp = ui.interact(m, ui.id().with(("grad-stop", i)), Sense::click_and_drag());
        let (dragging_this, copy) = match drag {
            Drag::Stop { index, copy } => (index == i, copy),
            _ => (false, false),
        };
        // A moved stop (not a copy) dragged this far below the ramp is removed on release.
        let off_ramp = |p: Pos2| !copy && p.y > bar.bottom() + REMOVE_DISTANCE && origin.len() > MIN_STOPS;
        let off = dragging_this && resp.interact_pointer_pos().is_some_and(off_ramp);
        let outline = if sel == Some(i) { t.accent } else { t.border };
        let body = vec![pos2(x, top), pos2(x + 6.0, top + 5.0), pos2(x + 6.0, top + 15.0), pos2(x - 6.0, top + 15.0), pos2(x - 6.0, top + 5.0)];
        if !off {
            ui.painter().add(egui::Shape::convex_polygon(body, if sel == Some(i) { t.text_strong } else { t.icon }, Stroke::new(1.0, outline)));
            let chip = Rect::from_min_size(pos2(x - 4.0, top + 6.0), vec2(8.0, 7.0));
            ui.painter().rect_filled(chip, 0.0, super::c32(&s.color));
        }
        if !is_grad {
            continue;
        }
        if resp.double_clicked() {
            popover = Some((i, m.left_bottom() + vec2(0.0, 4.0)));
        }
        if resp.clicked() || resp.drag_started() {
            select = Some(i);
        }
        if resp.drag_started() {
            drag = Drag::Stop { index: i, copy: alt };
            set_pstate(ui.ctx(), "grad-origin", stops.clone());
        }
        if (resp.dragged() || resp.drag_stopped())
            && dragging_this
            && let Some(pp) = resp.interact_pointer_pos()
        {
            let phase = if resp.drag_stopped() { Live::Released } else { Live::Dragging };
            if resp.drag_stopped() {
                drag = Drag::None;
            }
            if !off_ramp(pp) {
                let over = if copy { stop_under(pp, i) } else { None };
                let (v, ni) = dragged_stops(&origin, i, offset_at(pp), copy, over);
                select = Some(ni);
                changed = Some((v, phase));
            } else if phase == Live::Released {
                if let Some(v) = remove_stop(&origin, i) {
                    select = Some(i.min(v.len() - 1));
                    changed = Some((v, phase));
                }
            } else {
                // Dragged off: show the gradient without the stop moving until release.
                changed = Some((origin.clone(), phase));
            }
        }
    }
    // A colour swatch dropped on the ramp.
    if is_grad
        && let Some(d) = area_resp.dnd_release_payload::<PanelDrag>()
        && let Some(c) = d.color()
        && let Some(p) = ui.input(|i| i.pointer.latest_pos())
    {
        let link = match &*d {
            PanelDrag::Paint { params, .. } => dropped_link(app, params),
            _ => None,
        };
        let (v, i) = drop_color(g, stops.iter().position(|s| marker(s.offset).contains(p)), offset_at(p), c, link);
        select = Some(i);
        changed = Some((v, Live::Released));
    }
    set_pstate(ui.ctx(), "grad-drag", drag);
    if let Some(i) = pick_mid {
        set_pstate(ui.ctx(), MID_SELECTED, Some((i, first_id(app))));
        if sel.is_some() {
            select_stop(app, None);
        }
    } else if select.is_some() {
        set_pstate(ui.ctx(), MID_SELECTED, None::<(usize, Option<NodeId>)>);
    }
    match changed {
        Some((v, phase)) => set_stops(app, &v, select, phase),
        None => {
            if let Some(i) = select.filter(|i| sel != Some(*i)) {
                select_stop(app, Some(i));
            }
        }
    }
    if let Some((i, at)) = popover {
        open_popover(app, i, at);
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let g = current(app);
    let hidden: bool = pstate(ui.ctx(), HIDE_OPTIONS);
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), HIDE_OPTIONS, !hidden);
    }
    ui.separator();
    if menu_item(ui, tl!("Add to Swatches"), g.is_some(), false)
        && let Some(g) = &g
    {
        save_to_swatches(app, g);
    }
    if menu_item(ui, tl!("Reverse Gradient"), g.is_some(), false) {
        edit(app, json!({"reverse": true}), Live::Released);
    }
    if menu_item(ui, tl!("Reset to White, Black"), true, false) {
        set_stops(app, &Gradient::default().stops, None, Live::Released);
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton};
    use vectorcraft_engine::Session;

    use super::*;

    #[test]
    fn offsets_and_json() {
        assert_eq!(x_to_offset(50.0, 0.0, 200.0), 0.25);
        assert_eq!(x_to_offset(-5.0, 0.0, 200.0), 0.0);
        assert_eq!(x_to_offset(500.0, 0.0, 200.0), 1.0);
        let j = stops_json(&Gradient::default().stops);
        assert_eq!(j.as_array().unwrap().len(), 2);
        assert_eq!(j[1]["midpoint"], json!(0.5));
    }

    #[test]
    fn dropped_colours_recolour_or_add_an_opaque_stop() {
        let mut g = Gradient::default();
        g.stops[1].opacity = 0.5;
        let red = Color::rgb(1.0, 0.0, 0.0);
        let (v, i) = drop_color(&g, Some(1), 0.3, red, None);
        assert_eq!((v.len(), i, v[1].color, v[1].opacity), (2, 1, red, 1.0));
        let (v, i) = drop_color(&g, None, 0.3, red, None);
        assert_eq!((v.len(), i, v[1].color, v[1].offset, v[1].opacity), (3, 1, red, 0.3, 1.0));
    }

    #[test]
    fn dragged_stops_move_copy_or_swap() {
        let g = Gradient::default();
        let (v, i) = dragged_stops(&g.stops, 0, 0.4, false, Some(1));
        assert_eq!((v.len(), i, v[0].offset), (2, 0, 0.4));
        let (v, i) = dragged_stops(&g.stops, 0, 0.4, true, None);
        assert_eq!((v.len(), i, v[1].offset, v[1].color), (3, 1, 0.4, Color::WHITE));
        let (v, i) = dragged_stops(&g.stops, 0, 1.0, true, Some(1));
        assert_eq!((v.len(), i, v[0].color, v[1].color), (2, 0, Color::BLACK, Color::WHITE));
        // Dropped on itself: a copy.
        assert_eq!(dragged_stops(&g.stops, 0, 0.0, true, Some(0)).0.len(), 3);
    }

    // ---------- headless frames ----------

    /// A selected rectangle filled with `fill` (`paint.setFill` params).
    fn app(fill: Value) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        app.run("paint.setFill", fill).unwrap();
        app
    }

    /// A headless Gradient panel: frames share one context and a clock.
    struct Panel {
        ctx: egui::Context,
        time: f64,
    }

    impl Panel {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            Self { ctx, time: 0.0 }
        }

        /// One frame with `events` (and `modifiers` held); returns the texts drawn and where.
        fn frame(&mut self, app: &mut VectorcraftApp, events: Vec<Event>, modifiers: Modifiers) -> Vec<(String, Pos2)> {
            fn texts(s: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
                match s {
                    egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.rect.center().to_vec2())),
                    egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                    _ => {}
                }
            }
            self.time += 0.1;
            let mut events = events;
            events.insert(0, Event::ModifiersChanged(modifiers));
            let input = egui::RawInput {
                time: Some(self.time),
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(260.0, 600.0))),
                ..Default::default()
            };
            let mut out = self.ctx.run_ui(input, |ui| show(app, ui));
            out.textures_delta.clear();
            let mut v = vec![];
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
            v
        }

        /// The rect of the widget its parent ui keyed `key` (last frame).
        fn widget(&self, key: impl std::hash::Hash + std::fmt::Debug + Copy) -> Rect {
            self.ctx
                .viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).find(|w| w.id == w.parent_id.with(key)).map(|w| w.rect))
                .expect("widget drawn")
        }

        fn click(&mut self, app: &mut VectorcraftApp, at: Pos2) -> Vec<(String, Pos2)> {
            let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(app, vec![Event::PointerMoved(at), b(true)], Modifiers::NONE);
            self.frame(app, vec![b(false)], Modifiers::NONE);
            self.frame(app, vec![], Modifiers::NONE)
        }

        /// The `size`-square widgets right of `at` on its row (last frame), left to right.
        fn row_after(&self, at: Pos2, size: f32) -> Vec<Rect> {
            let mut v: Vec<Rect> = self.ctx.viewport(|vp| {
                vp.prev_pass
                    .widgets
                    .layers()
                    .flat_map(|(_, w)| w.iter())
                    .map(|w| w.rect)
                    .filter(|r| {
                        (r.width() - size).abs() < 0.5 && (r.height() - size).abs() < 0.5 && (r.center().y - at.y).abs() < 8.0 && r.left() > at.x
                    })
                    .collect()
            });
            v.sort_by(|a, b| a.left().total_cmp(&b.left()));
            v.dedup();
            v
        }

        /// Press at `from`, move to `to` in steps and release there, with `m` held.
        fn drag(&mut self, app: &mut VectorcraftApp, from: Pos2, to: Pos2, m: Modifiers) {
            let b = |at, pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: m };
            self.frame(app, vec![Event::PointerMoved(from), b(from, true)], m);
            for k in 1..=4 {
                self.frame(app, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))], m);
            }
            self.frame(app, vec![b(to, false)], m);
            self.frame(app, vec![], Modifiers::NONE);
        }
    }

    fn fill(app: &VectorcraftApp) -> GradientPaint {
        match crate::panels::current_paints(app).0 {
            Paint::Gradient(g) => *g,
            p => panic!("expected a gradient fill, got {p:?}"),
        }
    }

    fn hexes(app: &VectorcraftApp) -> Vec<String> {
        fill(app).gradient.stops.iter().map(|s| s.color.to_hex()).collect()
    }

    #[test]
    fn the_proxy_toggles_fill_and_stroke_and_stays_with_options_hidden() {
        let mut app = app(json!({"gradient": {}}));
        let mut p = Panel::new();
        let texts = p.frame(&mut app, vec![], Modifiers::NONE);
        assert!(texts.iter().any(|(t, _)| t == "Opacity:"));
        let stroke = p.widget("stroke-proxy");
        p.click(&mut app, stroke.right_bottom() - vec2(2.0, 2.0));
        assert!(!app.session.fill_active, "the Stroke proxy came to the front");
        set_pstate(&p.ctx, HIDE_OPTIONS, true);
        let texts = p.frame(&mut app, vec![], Modifiers::NONE);
        assert!(!texts.iter().any(|(t, _)| t == "Opacity:" || t == "Type:"), "{texts:?}");
        let fill = p.widget("fill-proxy");
        p.click(&mut app, fill.left_top() + vec2(2.0, 2.0));
        assert!(app.session.fill_active, "the proxy works with the options hidden");
        p.widget(("grad-stop", 1));
    }

    #[test]
    fn the_stroke_buttons_lay_the_gradient_within_along_or_across_the_stroke() {
        let mut app = app(json!({"gradient": {}}));
        app.run("paint.setStroke", json!({"gradient": {}, "focus": false})).unwrap();
        assert!(app.session.fill_active);
        let mode = |app: &VectorcraftApp| app.session.shown_stroke().unwrap().gradient_mode;
        let mut p = Panel::new();
        let texts = p.frame(&mut app, vec![], Modifiers::NONE);
        let label = texts.iter().find(|(t, _)| t == "Stroke:").expect("the Stroke buttons").1;
        // With the Fill proxy in front they are off.
        let buttons = p.row_after(label, 22.0);
        assert_eq!(buttons.len(), 3);
        p.click(&mut app, buttons[1].center());
        assert_eq!(mode(&app), StrokeGradientMode::Within);
        let stroke = p.widget("stroke-proxy");
        p.click(&mut app, stroke.right_bottom() - vec2(2.0, 2.0));
        assert!(!app.session.fill_active);
        for (i, want) in [(1, StrokeGradientMode::Along), (2, StrokeGradientMode::Across), (0, StrokeGradientMode::Within)] {
            p.click(&mut app, buttons[i].center());
            assert_eq!(mode(&app), want);
        }
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(mode(&app), StrokeGradientMode::Across, "one undo step per click");
    }

    #[test]
    fn a_selected_diamond_shows_and_sets_its_midpoint_in_location() {
        let stops = json!([{"offset": 0, "color": "#ffffff", "midpoint": 0.3}, {"offset": 1, "color": "#000000"}]);
        let mut app = app(json!({"gradient": {"stops": stops}}));
        app.run("gradient.selectStop", json!({"index": 1})).unwrap();
        let mut p = Panel::new();
        let texts = p.frame(&mut app, vec![], Modifiers::NONE);
        assert!(texts.iter().any(|(t, _)| t == "100%"), "the stop's location: {texts:?}");
        let diamond = p.widget(("grad-mid", 0));
        let texts = p.click(&mut app, diamond.center());
        assert_eq!(app.session.selected_stop(), None, "the diamond replaces the stop selection");
        assert!(texts.iter().any(|(t, _)| t == "30%"), "the midpoint: {texts:?}");
        // Typing a location moves the midpoint.
        let field = texts.iter().find(|(t, _)| t == "30%").unwrap().1;
        p.click(&mut app, field);
        let key = |k, modifiers| Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers };
        p.frame(&mut app, vec![key(egui::Key::A, Modifiers::COMMAND), Event::Text("60".into())], Modifiers::NONE);
        p.frame(&mut app, vec![key(egui::Key::Enter, Modifiers::NONE)], Modifiers::NONE);
        assert!((fill(&app).gradient.stops[0].midpoint - 0.6).abs() < 1e-6, "{:?}", fill(&app).gradient.stops);
        // Clicking a stop selects it again.
        let stop = p.widget(("grad-stop", 0));
        let texts = p.click(&mut app, stop.center());
        assert_eq!(app.session.selected_stop(), Some(0));
        assert!(texts.iter().any(|(t, _)| t == "0%"));
    }

    #[test]
    fn the_thumbnail_menu_applies_gradient_swatches_and_saves_one() {
        let mut app = app(json!({"color": "#336699"}));
        let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
        app.run("swatch.new", json!({"name": "Dusk", "gradient": {"stops": stops}})).unwrap();
        let mut p = Panel::new();
        p.frame(&mut app, vec![], Modifiers::NONE);
        let menu = p.widget("grad-swatches").center();
        let texts = p.click(&mut app, menu);
        let dusk = texts.iter().find(|(t, _)| t == "Dusk").unwrap_or_else(|| panic!("the menu lists the swatch: {texts:?}")).1;
        assert!(texts.iter().any(|(t, _)| t.ends_with("Save to Swatches")));
        p.click(&mut app, dusk);
        assert_eq!(hexes(&app), ["#ff0000", "#0000ff"]);
        // Save to Swatches adds the current gradient.
        let before = gradient_swatches(&app).len();
        let texts = p.click(&mut app, menu);
        let save = texts.iter().find(|(t, _)| t.ends_with("Save to Swatches")).unwrap().1;
        p.click(&mut app, save);
        assert_eq!(gradient_swatches(&app).len(), before + 1);
    }

    #[test]
    fn alt_drag_copies_a_stop_and_alt_dropping_on_a_stop_swaps_colours() {
        let mut app = app(json!({"gradient": {}}));
        let mut p = Panel::new();
        p.frame(&mut app, vec![], Modifiers::NONE);
        let (a, b) = (p.widget(("grad-stop", 0)).center(), p.widget(("grad-stop", 1)).center());
        let mid = pos2((a.x + b.x) / 2.0, a.y);
        let offsets = |app: &VectorcraftApp| fill(app).gradient.stops.iter().map(|s| (s.offset * 10.0).round() / 10.0).collect::<Vec<_>>();
        // A plain drag moves the stop.
        p.drag(&mut app, a, mid, Modifiers::NONE);
        assert_eq!(offsets(&app), [0.5, 1.0]);
        app.run("edit.undo", json!({})).unwrap();
        // Alt leaves the original and drags a copy, selected on release; one undo step.
        p.frame(&mut app, vec![], Modifiers::NONE);
        p.drag(&mut app, a, mid, Modifiers::ALT);
        assert_eq!((offsets(&app), hexes(&app)), (vec![0.0, 0.5, 1.0], vec!["#ffffff".to_string(), "#ffffff".into(), "#000000".into()]));
        assert_eq!(app.session.selected_stop(), Some(1));
        // Alt-dropping the first stop on the last swaps their colours.
        p.drag(&mut app, a, b, Modifiers::ALT);
        assert_eq!((offsets(&app), hexes(&app)), (vec![0.0, 0.5, 1.0], vec!["#000000".to_string(), "#ffffff".into(), "#ffffff".into()]));
        app.run("edit.undo", json!({})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(hexes(&app), ["#ffffff", "#000000"]);
    }

    #[test]
    fn a_dropped_swatch_adds_or_recolours_a_stop() {
        let mut app = app(json!({"gradient": {}}));
        let mut p = Panel::new();
        p.frame(&mut app, vec![], Modifiers::NONE);
        let (a, b) = (p.widget(("grad-stop", 0)).center(), p.widget(("grad-stop", 1)).center());
        let drop = |p: &mut Panel, app: &mut VectorcraftApp, at: Pos2| {
            egui::DragAndDrop::set_payload(&p.ctx, PanelDrag::paint(Paint::solid(Color::rgb(1.0, 0.0, 0.0))));
            p.frame(app, vec![Event::PointerMoved(at)], Modifiers::NONE);
            let up = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
            p.frame(app, vec![up], Modifiers::NONE);
        };
        drop(&mut p, &mut app, pos2((a.x + b.x) / 2.0, a.y - 20.0));
        assert_eq!(hexes(&app), ["#ffffff", "#ff0000", "#000000"]);
        p.frame(&mut app, vec![], Modifiers::NONE);
        let last = p.widget(("grad-stop", 2)).center();
        drop(&mut p, &mut app, last);
        assert_eq!(hexes(&app), ["#ffffff", "#ff0000", "#ff0000"]);
        // A spot swatch tile dropped on a stop links it: editing the swatch recolours the stop.
        app.run("swatch.new", json!({"name": "Ink", "color": "#cc0066", "spot": true, "focus": false})).unwrap();
        let ink = app.session.doc().unwrap().doc.swatch("Ink").unwrap().paint.clone();
        egui::DragAndDrop::set_payload(&p.ctx, PanelDrag::Paint { paint: ink, params: json!({"swatch": "Ink"}), rows: None });
        p.frame(&mut app, vec![Event::PointerMoved(a)], Modifiers::NONE);
        p.frame(
            &mut app,
            vec![Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
            Modifiers::NONE,
        );
        let st = fill(&app).gradient.stops[0].clone();
        assert_eq!((st.swatch.as_deref(), st.tint, st.color.to_hex()), (Some("Ink"), 1.0, "#cc0066".to_string()));
        app.run("swatch.edit", json!({"name": "Ink", "color": "#0066cc"})).unwrap();
        assert_eq!(hexes(&app)[0], "#0066cc");
    }

    #[test]
    fn double_clicking_a_stop_opens_the_shared_popover_beside_it() {
        let mut app = app(json!({"gradient": {}}));
        let mut p = Panel::new();
        p.frame(&mut app, vec![], Modifiers::NONE);
        let stop = p.widget(("grad-stop", 1));
        let b = |pressed| Event::PointerButton { pos: stop.center(), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        p.frame(&mut app, vec![Event::PointerMoved(stop.center()), b(true), b(false), b(true), b(false)], Modifiers::NONE);
        let d = app.ui.dialog.clone().expect("the popover opened");
        assert_eq!(
            (d.kind.as_str(), &d.fields["index"], &d.fields["screen"]),
            ("gradientStop", &json!(1), &json!([stop.left(), stop.bottom() + 4.0]))
        );
        assert_eq!(app.session.selected_stop(), Some(1));
    }

    #[test]
    fn the_stop_eyedropper_arms_the_eyedropper_for_the_stop() {
        let mut app = app(json!({"gradient": {}}));
        app.select_tool("gradient");
        app.run("gradient.selectStop", json!({"index": 0})).unwrap();
        let texts = Panel::new().frame(&mut app, vec![], Modifiers::NONE);
        assert!(texts.iter().any(|(t, _)| t == "#FFFFFF"), "the stop's colour row: {texts:?}");
        stop_eyedropper(&mut app);
        assert_eq!((app.session.tool_id(), app.session.tool_options()), ("eyedropper", json!({"stop": "gradient"})));
    }
}
