//! Stroke panel: weight spinner + presets, cap / corner / align toggles, miter limit, dashed line
//! with three dash/gap pairs, arrowheads with rendered previews, arrow scale, width profiles.

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ArrowAlign, Arrowhead, Dash, Document, LineCap, LineJoin, Node, StrokeAlign, StrokeLayer, Unit, WidthProfile};
use vectorcraft_engine::inspect::StrokeMixed;

use super::{character, current_stroke, pstate, set_pstate, stroke_mixed};
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

/// Stroke weight dropdown presets in points, from the ladder of `unit` (Units > Stroke): each
/// unit has its own ladder of round values in that unit, so the dropdown reads `0.25 mm`, not the
/// `0.088 mm` a converted pt ladder gives. Feet, yards and meters, which no stroke is measured
/// in, keep the pt ladder.
pub fn weight_presets(unit: Unit) -> [f64; 22] {
    let native = match unit {
        Unit::Millimeters => MM_PRESETS,
        Unit::Centimeters => CM_PRESETS,
        Unit::Inches => IN_PRESETS,
        Unit::Pixels => PX_PRESETS,
        Unit::Picas => return PC_PRESETS,
        Unit::Points | Unit::FeetInches | Unit::Feet | Unit::Meters | Unit::Yards => return PT_PRESETS,
    };
    native.map(|v| unit.to_pt(v))
}

const PT_PRESETS: [f64; 22] =
    [0.25, 0.5, 0.75, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0];
const MM_PRESETS: [f64; 22] = [0.1, 0.25, 0.35, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 15.0, 20.0, 25.0, 30.0];
const CM_PRESETS: [f64; 22] = [0.01, 0.02, 0.03, 0.05, 0.06, 0.07, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0];
const IN_PRESETS: [f64; 22] =
    [0.0078, 0.0156, 0.0313, 0.0625, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0];
const PX_PRESETS: [f64; 22] =
    [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 30.0, 40.0];
/// The pica ladder (0p1 … 5p) in points, so every entry is an exact pt weight. The dropdown shows
/// them as decimal picas (`0.083 p`) until [`Unit::number`] writes `0p1` notation.
const PC_PRESETS: [f64; 22] =
    [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 15.0, 18.0, 21.0, 24.0, 27.0, 30.0, 33.0, 36.0, 48.0, 60.0];

/// A profile's points as its silhouette: none (the plain bar) for the uniform stroke.
fn silhouette(points: &[(f64, f64, f64)]) -> Option<&[(f64, f64, f64)]> {
    (points != WidthProfile::PRESETS[0].points).then_some(points)
}

/// The Arrowheads Align buttons: (alignment, `stroke.set` value, icon, tooltip).
const ARROW_ALIGN: [(ArrowAlign, &str, &str, &str); 2] = [
    (ArrowAlign::Extend, "extend", "dc-arrow-extend", "Tip extends past the end point"),
    (ArrowAlign::Tip, "tip", "dc-arrow-tip", "Tip on the end point"),
];

/// Which Align button is on, and whether they are usable (only a stroke with a head can be aligned).
fn arrow_align_state(st: Option<&StrokeLayer>) -> (ArrowAlign, bool) {
    st.map_or((ArrowAlign::default(), false), |s| (s.arrow_align, s.start_arrow.is_some() || s.end_arrow.is_some()))
}

/// Dash pattern → the panel's six dash/gap fields (None = empty field).
pub fn dash_fields(pattern: &[f64]) -> [Option<f64>; 6] {
    let mut out = [None; 6];
    for (i, v) in pattern.iter().take(6).enumerate() {
        out[i] = Some(*v);
    }
    out
}

/// The dash/gap fields and alignment the Dashed Line section shows: the stroke's dashes, or with
/// dashes off the last ones turned off (`last`), at first 12 pt dashes fitted to corners.
fn dash_state(dash: Option<&Dash>, last: Option<([Option<f64>; 6], bool)>) -> ([Option<f64>; 6], bool) {
    match dash {
        Some(d) => (dash_fields(&d.pattern), d.align_corners),
        None => last.unwrap_or(([Some(12.0), None, None, None, None, None], true)),
    }
}

/// Six dash/gap fields → a dash pattern: stops at the first empty dash; a dash without a gap
/// repeats its length as the gap (Illustrator's behaviour).
pub fn dash_pattern(fields: &[Option<f64>; 6]) -> Vec<f64> {
    let mut out = vec![];
    for pair in 0..3 {
        let Some(d) = fields[pair * 2] else { break };
        let g = fields[pair * 2 + 1].unwrap_or(d);
        out.push(d.max(0.0));
        out.push(g.max(0.0));
    }
    if out.iter().all(|v| *v == 0.0) {
        out.clear();
    }
    out
}

fn arrow_label(a: Option<Arrowhead>) -> &'static str {
    a.map_or("None", Arrowhead::label)
}

/// Apply Stroke panel options (`stroke.set` params): while the Type tool edits text, to the
/// selected characters' stroke.
fn set(app: &mut VectorcraftApp, p: Value) {
    if character::text_editing(app).is_some() {
        character::range_style(app, json!({ "strokeOptions": p }));
    } else {
        app.run("stroke.set", p).ok();
    }
}

/// The weight the Stroke panel, Control bar and Properties panel show: blank where the selected
/// objects differ, the default for new art without a stroke.
pub(crate) fn shown_weight(app: &VectorcraftApp, st: Option<&StrokeLayer>, mixed: &StrokeMixed) -> Option<f64> {
    (!mixed.weight).then(|| st.map_or(app.session.paint.stroke_width, |s| s.width))
}

/// The weight spinner (Stroke panel, Control bar): Units > Stroke, presets, blank when mixed.
pub(crate) fn weight_field(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, weight: Option<f64>, width: f32) {
    let unit = app.session.stroke_unit();
    if let Some(w) = widgets::spin_field(ui, id, weight, unit, width, 1.0, 0.0, &weight_presets(unit)) {
        set(app, json!({"weight": w}));
    }
}

/// The underlined Stroke link (`label`: the Control bar's "Stroke:", the Properties panel's
/// "Stroke"): a click toggles the Stroke panel in a popover under it.
pub(crate) fn link(app: &mut VectorcraftApp, ui: &mut Ui, label: &str) {
    let t = Tokens::get(ui.ctx());
    let resp = ui.link(egui::RichText::new(label).size(12.0).color(t.text).underline()).on_hover_text(tl!("Stroke options"));
    widgets::popover(&resp, resp.clicked(), |ui| {
        ui.set_width(260.0);
        show(app, ui);
    });
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let st = current_stroke(app);
    let mixed = stroke_mixed(app, ui.ctx());
    let unit = app.session.stroke_unit();
    let weight = shown_weight(app, st.as_ref(), &mixed);
    let hidden: bool = pstate(ui.ctx(), "stroke-hide-options");
    let label_w = 64.0;
    let row_label = |ui: &mut Ui, s: &str| {
        let l = ui.add_sized(vec2(label_w, 24.0), egui::Label::new(egui::RichText::new(s).size(12.5).color(t.text)).halign(egui::Align::RIGHT));
        crate::scrub::note_label(ui, l.rect);
    };
    ui.horizontal(|ui| {
        row_label(ui, tl!("Weight:"));
        weight_field(app, ui, "stroke-weight", weight, 120.0);
    });
    if hidden {
        return;
    }
    // Mixed values light no button.
    let cap = st.as_ref().map(|s| s.cap).filter(|_| !mixed.cap);
    let join = st.as_ref().map(|s| s.join).filter(|_| !mixed.join);
    let align = st.as_ref().map(|s| s.align).filter(|_| !mixed.align);
    ui.horizontal(|ui| {
        row_label(ui, tl!("Cap:"));
        for (v, icon, tip, name) in [
            (LineCap::Butt, "dc-cap-butt", tl!("Butt Cap"), "butt"),
            (LineCap::Round, "dc-cap-round", tl!("Round Cap"), "round"),
            (LineCap::Square, "dc-cap-square", tl!("Projecting Cap"), "square"),
        ] {
            if widgets::icon_button(ui, icon, tip, cap == Some(v), 24.0).clicked() {
                set(app, json!({"cap": name}));
            }
        }
    });
    ui.horizontal(|ui| {
        row_label(ui, tl!("Corner:"));
        for (v, icon, tip, name) in [
            (LineJoin::Miter, "dc-join-miter", tl!("Miter Join"), "miter"),
            (LineJoin::Round, "dc-join-round", tl!("Round Join"), "round"),
            (LineJoin::Bevel, "dc-join-bevel", tl!("Bevel Join"), "bevel"),
        ] {
            if widgets::icon_button(ui, icon, tip, join == Some(v), 24.0).clicked() {
                set(app, json!({"join": name}));
            }
        }
        widgets::dim_label(ui, tl!("Limit:"));
        let lim = st.as_ref().map(|s| s.miter_limit).unwrap_or(10.0);
        if !matches!(join, Some(LineJoin::Round | LineJoin::Bevel)) {
            if let Some(v) = widgets::mixed_field(ui, "stroke-limit", (!mixed.miter_limit).then_some(lim), " x", 0, 50.0) {
                set(app, json!({"miterLimit": v.clamp(1.0, 500.0)}));
            }
        } else {
            ui.label(egui::RichText::new(format!("{lim:.0} x")).color(t.text_disabled));
        }
    });
    ui.horizontal(|ui| {
        row_label(ui, tl!("Align Stroke:"));
        for (v, icon, tip, name) in [
            (StrokeAlign::Center, "dc-stroke-center", tl!("Align Stroke to Center"), "center"),
            (StrokeAlign::Inside, "dc-stroke-inside", tl!("Align Stroke to Inside"), "inside"),
            (StrokeAlign::Outside, "dc-stroke-outside", tl!("Align Stroke to Outside"), "outside"),
        ] {
            // Only closed paths take a stroke inside or outside (not open paths, not type).
            let enabled = v == StrokeAlign::Center || mixed.can_align;
            if widgets::icon_button_enabled(ui, icon, tip, align == Some(v), enabled, 24.0).clicked() {
                set(app, json!({"align": name}));
            }
        }
    });
    widgets::divider(ui);
    // Dashed line.
    let dash = st.as_ref().and_then(|s| s.dash.as_ref());
    let (fields, align_corners) = dash_state(dash, pstate(ui.ctx(), "stroke-dash-last"));
    let shown = if mixed.dash { [None; 6] } else { fields };
    ui.horizontal(|ui| {
        if widgets::check(ui, tl!("Dashed Line"), dash.is_some(), st.is_some()) {
            if dash.is_some() {
                set_pstate(ui.ctx(), "stroke-dash-last", Some((fields, align_corners)));
                set(app, json!({"dash": null}));
            } else {
                set(app, json!({"dash": dash_pattern(&fields), "alignDashes": align_corners}));
            }
        }
        ui.add_space((ui.available_width() - 56.0).max(0.0));
        let on = dash.is_some();
        if widgets::icon_button_enabled(ui, "dc-dash-exact", tl!("Exact dash lengths"), on && !align_corners, on, 24.0).clicked() {
            set(app, json!({"alignDashes": false}));
        }
        if widgets::icon_button_enabled(ui, "dc-dash-align", tl!("Fit dashes to corners and ends"), on && align_corners, on, 24.0).clicked() {
            set(app, json!({"alignDashes": true}));
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let fw = ((ui.available_width() - 15.0) / 6.0).clamp(28.0, 40.0);
        let mut nf = shown;
        let mut changed = false;
        for (i, f) in shown.iter().enumerate() {
            let r = ui.add_enabled_ui(dash.is_some(), |ui| widgets::opt_field(ui, ("dash", i), *f, unit, fw)).inner;
            if let Some(x) = r {
                nf[i] = x.map(|v| v.max(0.0));
                changed = true;
            }
        }
        if changed && dash.is_some() {
            // The command keeps the dash offset and alignment.
            set(app, json!({"dash": dash_pattern(&nf)}));
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let fw = ((ui.available_width() - 15.0) / 6.0).clamp(28.0, 40.0);
        for lbl in [tl!("dash"), tl!("gap"), tl!("dash"), tl!("gap"), tl!("dash"), tl!("gap")] {
            ui.add_sized(vec2(fw, 12.0), egui::Label::new(egui::RichText::new(lbl).size(10.5).color(t.text_dim)));
        }
    });
    widgets::divider(ui);
    // Arrowheads.
    let (sa, ea) = (st.as_ref().and_then(|s| s.start_arrow), st.as_ref().and_then(|s| s.end_arrow));
    ui.horizontal(|ui| {
        row_label(ui, tl!("Arrowheads:"));
        if let Some(a) = arrow_dropdown(ui, "arrow-start", sa, true) {
            set(app, json!({"startArrow": a.map(|a| format!("{a:?}"))}));
        }
        if let Some(a) = arrow_dropdown(ui, "arrow-end", ea, false) {
            set(app, json!({"endArrow": a.map(|a| format!("{a:?}"))}));
        }
        if widgets::icon_button_enabled(ui, "arrow-left-right", tl!("Swap start and end arrowheads"), false, st.is_some(), 22.0).clicked() {
            app.run("stroke.setAdvanced", json!({"swapArrows": true})).ok();
        }
    });
    let scale = st.as_ref().map(|s| s.arrow_scale).unwrap_or((100.0, 100.0));
    let linked: bool = pstate(ui.ctx(), "arrow-scale-link");
    // Scale and Align only mean something for a stroke with a head.
    let (arrow_align, has_head) = arrow_align_state(st.as_ref());
    ui.horizontal(|ui| {
        row_label(ui, tl!("Scale:"));
        let mut ns = None;
        ui.add_enabled_ui(has_head, |ui| {
            if let Some(v) = widgets::plain_field(ui, "arrow-scale-s", scale.0, "%", 0, 56.0) {
                ns = Some((v, if linked { v } else { scale.1 }));
            }
            if let Some(v) = widgets::plain_field(ui, "arrow-scale-e", scale.1, "%", 0, 56.0) {
                ns = Some((if linked { v } else { scale.0 }, v));
            }
        });
        if widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tl!("Link start and end arrowhead scales"), linked, 22.0).clicked() {
            set_pstate(ui.ctx(), "arrow-scale-link", !linked);
        }
        if let Some((a, b)) = ns {
            app.run("stroke.setAdvanced", json!({"arrowScale": [a, b]})).ok();
        }
    });
    ui.horizontal(|ui| {
        row_label(ui, tl!("Align:"));
        for (v, name, icon, tip) in ARROW_ALIGN {
            if widgets::icon_button_enabled(ui, icon, tip, arrow_align == v, has_head, 22.0).clicked() {
                set(app, json!({"arrowAlign": name}));
            }
        }
    });
    widgets::divider(ui);
    // Profile.
    ui.horizontal(|ui| {
        row_label(ui, tl!("Profile:"));
        if let Some(id) = profile_dropdown(app, ui, st.as_ref().and_then(|s| s.profile.as_ref())) {
            set(app, json!({"profile": id}));
        }
        let can_flip = st.as_ref().is_some_and(|s| s.profile.is_some());
        if widgets::icon_button_enabled(ui, "flip-horizontal-2", tl!("Flip Along"), false, can_flip, 22.0).clicked() {
            app.run("stroke.setAdvanced", json!({"flipProfile": "along"})).ok();
        }
        if widgets::icon_button_enabled(ui, "flip-vertical-2", tl!("Flip Across"), false, can_flip, 22.0).clicked() {
            app.run("stroke.setAdvanced", json!({"flipProfile": "across"})).ok();
        }
    });
}

/// The document an arrowhead preview renders: a `w`×`h` pt line with head `a` at its left
/// (`start`) or right end, its tip on the end point, drawn white (the preview is tinted).
pub(super) fn arrow_doc(a: Option<Arrowhead>, start: bool, w: f64, h: f64) -> Option<Document> {
    let mut doc = Document::new(w, h);
    let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::WHITE), 1.5);
    let st = ap.stroke_mut()?;
    // Heads of weight 2.5 pt: 10 pt long and wide.
    st.arrow_scale = (500.0 / 3.0, 500.0 / 3.0);
    st.arrow_align = ArrowAlign::Tip;
    if start {
        st.start_arrow = a
    } else {
        st.end_arrow = a
    }
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((3.0, h / 2.0));
    bp.line_to((w - 3.0, h / 2.0));
    let (id, l) = (doc.alloc_id(), doc.layers[0].id);
    doc.insert(Some(l), 0, Node::path(id, vectorcraft_geom::PathData::from_bezpath(&bp), ap)).ok()?;
    Some(doc)
}

/// Draw an arrowhead preview (the head as the canvas draws it) in `r`: a line with the head at the
/// left (`start`) or right end.
fn paint_arrow(ui: &Ui, r: Rect, a: Option<Arrowhead>, start: bool, color: Color32) {
    if let Some(tex) = widgets::doc_preview(ui, &format!("arrow:{a:?}:{start}"), r.size(), |w, h| arrow_doc(a, start, w, h)) {
        ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
    }
}

/// Dropdown whose button and items show drawn arrowhead previews. Returns Some(choice).
fn arrow_dropdown(ui: &mut Ui, id: &str, cur: Option<Arrowhead>, start: bool) -> Option<Option<Arrowhead>> {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(64.0, 24.0), Sense::click());
    ui.painter().rect_filled(r, 2, t.input);
    ui.painter().rect_stroke(r, 2, Stroke::new(1.0, if resp.hovered() { t.text } else { t.input_border }), StrokeKind::Inside);
    let body = Rect::from_min_max(r.min + vec2(3.0, 0.0), pos2(r.right() - 16.0, r.bottom()));
    paint_arrow(ui, body, cur, start, t.text_strong);
    icons::paint(ui, "chevron-down", Rect::from_center_size(pos2(r.right() - 8.0, r.center().y), vec2(10.0, 10.0)), t.icon);
    let resp = resp.on_hover_text(if start { tl!("Start arrowhead") } else { tl!("End arrowhead") });
    let mut out = None;
    egui::Popup::menu(&resp).id(egui::Id::new(("arrow-pop", id))).show(|ui| {
        ui.set_min_width(170.0);
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for a in std::iter::once(None).chain(Arrowhead::ALL.map(Some)) {
                let (row, rr) = ui.allocate_exact_size(vec2(170.0, 22.0), Sense::click());
                if a == cur {
                    ui.painter().rect_filled(row, 0.0, t.row_selected);
                } else if rr.hovered() {
                    ui.painter().rect_filled(row, 0.0, t.hover);
                }
                paint_arrow(ui, Rect::from_min_size(row.min + vec2(4.0, 0.0), vec2(56.0, 22.0)), a, start, t.text_strong);
                ui.painter().text(
                    row.left_center() + vec2(66.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    tl!(arrow_label(a)),
                    egui::FontId::proportional(11.5),
                    t.text,
                );
                if rr.clicked() {
                    out = Some(a);
                    ui.close();
                }
            }
        });
    });
    out
}

/// Draw a width profile's silhouette (its points; None: the plain bar) into `r`. A profile made
/// with the Width tool can be many times the stroke's weight: the silhouette is scaled to fit `r`
/// by its widest point, and clipped to it (#909).
pub fn paint_profile(ui: &Ui, r: Rect, points: Option<&[(f64, f64, f64)]>, color: Color32) {
    let n = 32;
    let half = r.height() / 2.0 - 1.0;
    let widths: Vec<(f64, f64)> = (0..=n).map(|i| points.map_or((0.35, 0.35), |p| WidthProfile::at_points(p, i as f64 / n as f64))).collect();
    let widest = widths.iter().map(|(l, rr)| l.max(*rr)).filter(|w| w.is_finite()).fold(1.0, f64::max);
    let mut top = vec![];
    let mut bot = vec![];
    for (i, (l, rr)) in widths.iter().enumerate() {
        let x = r.left() + i as f32 / n as f32 * r.width();
        let side = |w: f64| if w.is_finite() { (w / widest).clamp(0.0, 1.0) as f32 * half } else { 0.0 };
        top.push(pos2(x, r.center().y - side(*l)));
        bot.push(pos2(x, r.center().y + side(*rr)));
    }
    let mut mesh = egui::Mesh::default();
    for i in 0..=n {
        mesh.colored_vertex(top[i], color);
        mesh.colored_vertex(bot[i], color);
    }
    for i in 0..n as u32 {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    ui.painter().with_clip_rect(r).add(egui::Shape::mesh(mesh));
}

/// The Profile dropdown (Stroke panel, Control bar): the stroke's own silhouette on the button
/// (a custom profile too), the built-in profiles and then the saved ones in the list. Returns the
/// id or name picked.
pub(crate) fn profile_dropdown(app: &VectorcraftApp, ui: &mut Ui, cur: Option<&WidthProfile>) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(100.0, 24.0), Sense::click());
    ui.painter().rect_filled(r, 2, t.input);
    ui.painter().rect_stroke(r, 2, Stroke::new(1.0, if resp.hovered() { t.text } else { t.input_border }), StrokeKind::Inside);
    let body = Rect::from_min_max(r.min + vec2(6.0, 5.0), pos2(r.right() - 20.0, r.bottom() - 5.0));
    paint_profile(ui, body, cur.and_then(|p| silhouette(&p.points)), t.text_strong);
    icons::paint(ui, "chevron-down", Rect::from_center_size(pos2(r.right() - 9.0, r.center().y), vec2(10.0, 10.0)), t.icon);
    let resp = resp.on_hover_text(tl!("Variable Width Profile"));
    let mut out = None;
    egui::Popup::menu(&resp).show(|ui| {
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            // A divider between the built-in profiles and the saved ones.
            let mut divide = true;
            for e in app.session.profile_entries() {
                if !e.built_in && std::mem::take(&mut divide) {
                    ui.separator();
                }
                let (row, rr) = ui.allocate_exact_size(vec2(170.0, 26.0), Sense::click());
                if e.matches(cur) {
                    ui.painter().rect_filled(row, 0.0, t.row_selected);
                } else if rr.hovered() {
                    ui.painter().rect_filled(row, 0.0, t.hover);
                }
                paint_profile(ui, Rect::from_min_size(row.min + vec2(6.0, 6.0), vec2(70.0, 14.0)), silhouette(e.points), t.text_strong);
                ui.painter().text(
                    row.left_center() + vec2(84.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    // A saved profile's name is the user's.
                    super::label_or_name(e.label, e.built_in),
                    egui::FontId::proportional(11.5),
                    t.text,
                );
                if rr.clicked() {
                    out = Some(e.id.to_string());
                    ui.close();
                }
            }
        });
    });
    out
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "stroke-hide-options");
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "stroke-hide-options", !hidden);
    }
    ui.separator();
    if menu_item(ui, tl!("Add to Profiles…"), crate::menus::enabled(app, "stroke.widthProfile.add"), false) {
        let name = app.session.next_profile_name();
        let p = json!({"command": "stroke.widthProfile.add", "label": "Variable Width Profile", "params": {"name": name}});
        app.run("ui.paramDialog", p).ok();
    }
    // Deletes the selected stroke's saved profile.
    let shown = current_stroke(app).and_then(|s| s.profile);
    let saved = shown.as_ref().and_then(|p| app.session.profile_entry(Some(p))).is_some_and(|e| !e.built_in);
    if menu_item(ui, tl!("Delete Profile"), saved, false) {
        app.run("stroke.widthProfile.delete", json!({})).ok();
    }
    if menu_item(ui, tl!("Reset Profiles"), crate::menus::enabled(app, "stroke.widthProfile.reset"), false) {
        app.run("stroke.widthProfile.reset", json!({})).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_fields_roundtrip() {
        let f = dash_fields(&[12.0, 6.0, 2.0, 3.0]);
        assert_eq!(f, [Some(12.0), Some(6.0), Some(2.0), Some(3.0), None, None]);
        assert_eq!(dash_pattern(&f), vec![12.0, 6.0, 2.0, 3.0]);
    }

    #[test]
    fn dash_without_gap_repeats_dash() {
        assert_eq!(dash_pattern(&[Some(5.0), None, None, Some(9.0), None, None]), vec![5.0, 5.0]);
        assert!(dash_pattern(&[None; 6]).is_empty());
        assert!(dash_pattern(&[Some(0.0), Some(0.0), None, None, None, None]).is_empty());
    }

    /// #909: a profile many times the stroke's weight (made with the Width tool) draws inside its
    /// box, scaled by its widest point, and the plain bar keeps its look.
    #[test]
    fn wide_profiles_fit_their_preview() {
        let ctx = egui::Context::default();
        let r = Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(80.0, 16.0));
        let drawn = |points: Option<&[(f64, f64, f64)]>| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| paint_profile(ui, r, points, Color32::WHITE));
            out.textures_delta.clear();
            out.shapes
                .into_iter()
                .filter_map(|c| match c.shape {
                    egui::Shape::Mesh(m) => Some((c.clip_rect, m.vertices.iter().map(|v| v.pos).collect::<Vec<_>>())),
                    _ => None,
                })
                .next()
                .unwrap()
        };
        let (clip, wide) = drawn(Some(&[(0.0, 1.0, 1.0), (0.5, 10.0, 10.0), (1.0, 1.0, 1.0)]));
        assert!(clip.min.y >= r.min.y - 0.5 && clip.max.y <= r.max.y + 0.5, "{clip:?}");
        assert!(wide.iter().all(|p| p.y >= r.top() && p.y <= r.bottom()), "{wide:?}");
        let middle = wide.iter().map(|p| (p.y - r.center().y).abs()).fold(0.0, f32::max);
        assert!((middle - (r.height() / 2.0 - 1.0)).abs() < 1e-3, "the widest point fills the box ({middle})");
        let (_, plain) = drawn(None);
        assert!(plain.iter().all(|p| (p.y - r.center().y).abs() <= 0.35 * (r.height() / 2.0 - 1.0) + 1e-3));
    }

    #[test]
    fn profile_silhouettes_and_arrow_labels() {
        assert!(silhouette(WidthProfile::PRESETS[0].points).is_none(), "uniform draws the plain bar");
        let lens = WidthProfile::lens();
        assert_eq!(silhouette(&lens.points), Some(lens.points.as_slice()));
        assert_eq!(arrow_label(Some(Arrowhead::CircleOpen)), "Circle (open)");
    }

    #[test]
    fn arrow_align_buttons_need_a_head() {
        let mut st = StrokeLayer::new(vectorcraft_color::Paint::None, 1.0);
        assert_eq!(arrow_align_state(None), (ArrowAlign::Extend, false));
        assert_eq!(arrow_align_state(Some(&st)), (ArrowAlign::Extend, false));
        st.start_arrow = Some(Arrowhead::Bar);
        st.arrow_align = ArrowAlign::Tip;
        assert_eq!(arrow_align_state(Some(&st)), (ArrowAlign::Tip, true));
        for (_, _, icon, _) in ARROW_ALIGN {
            assert!(icons::exists(icon), "{icon}");
        }
    }

    /// Run the panel and its ≡ menu for one headless frame.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(app, ui);
            menu(app, ui);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn panel_draws_for_lines_with_and_without_heads() {
        use vectorcraft_engine::Session;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let r = |app: &mut VectorcraftApp, id: &str, p: Value| app.session.execute(id, &p).unwrap();
        r(&mut app, "file.new", json!({"width": 100, "height": 100}));
        frame(&mut app);
        r(&mut app, "shape.line", json!({"x1": 10, "y1": 50, "x2": 90, "y2": 50}));
        frame(&mut app);
        for p in [json!({"endArrow": "ArrowOpen", "arrowAlign": "tip"}), json!({"profile": "lens"}), json!({"dash": [0, 6], "cap": "round"})] {
            r(&mut app, "stroke.set", p);
            frame(&mut app);
        }
        let st = current_stroke(&app).unwrap();
        assert_eq!((st.end_arrow, st.arrow_align), (Some(Arrowhead::ArrowOpen), ArrowAlign::Tip));
        assert_eq!(WidthProfile::id_of(st.profile.as_ref()), "lens");
    }

    #[test]
    fn while_typing_the_panel_strokes_the_selected_characters() {
        use vectorcraft_doc::NodeKind;
        use vectorcraft_engine::{Session, ViewInfo};
        use vectorcraft_tools::{PointerEvent, PointerKind};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let v = ViewInfo::default();
        app.session.select_tool("type", v).unwrap();
        for k in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(k, 50.0, 200.0), v).unwrap();
        }
        app.session.tool_text("Make this bold", v).unwrap();
        app.session.set_tool_option("select", &json!({"start": 5, "end": 9}));
        set(&mut app, json!({"weight": 4, "join": "round"}));
        let id = app.session.active().unwrap().selection.objects[0];
        let NodeKind::Text(t) = &app.session.active().unwrap().doc.node(id).unwrap().kind else { panic!("type") };
        let joins: Vec<(&str, LineJoin, f64)> = t.runs.iter().map(|r| (r.text.as_str(), r.style.stroke_join, r.style.stroke_width)).collect();
        assert_eq!(joins, [("Make ", LineJoin::Miter, 0.0), ("this", LineJoin::Round, 4.0), (" bold", LineJoin::Miter, 0.0)]);
        // The panel shows the selected characters' stroke.
        assert_eq!(current_stroke(&app).map(|s| (s.join, s.width)), Some((LineJoin::Round, 4.0)));
        frame(&mut app);
    }
}
