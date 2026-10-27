//! Transform panel: reference point, X/Y/W/H with the constrain link, rotate and shear, flips,
//! live-shape properties and the Scale Corners / Scale Strokes & Effects options. While the
//! Artboard tool is in use it shows and edits the active artboard's X/Y/W/H instead (a new W or H
//! resizes every selected artboard), and with only ruler guides selected their X (vertical guides)
//! and Y (horizontal ones).

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{LiveShape, Node, NodeKind, OrientedBox};
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_geom::{Point, Rect, Vec2};

use super::{corner_radius_row, first_selected, pstate, set_pstate};
use crate::dialogs::corners::kind_label as corner_kind;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

pub const ANGLE_PRESETS: [f64; 9] = [-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0];
/// The shear angle's presets: steep enough to be useful, short of the degenerate ±90°.
pub const SHEAR_PRESETS: [f64; 9] = [-60.0, -45.0, -30.0, -15.0, 0.0, 15.0, 30.0, 45.0, 60.0];

/// Proportional size: the other dimension when one changes with the link on.
pub fn constrained(w: f64, h: f64, new_w: Option<f64>, new_h: Option<f64>) -> (f64, f64) {
    match (new_w, new_h) {
        (Some(nw), _) if w.abs() > 1e-9 => (nw, h * nw / w),
        (_, Some(nh)) if h.abs() > 1e-9 => (w * nh / h, nh),
        (a, b) => (a.unwrap_or(w), b.unwrap_or(h)),
    }
}

/// The artboard the Artboard tool has active, while that tool is in use.
fn tool_artboard(app: &VectorcraftApp) -> Option<(usize, Rect)> {
    if app.session.tool_id() != "artboard" {
        return None;
    }
    let i = usize::try_from(app.session.tool_options().get("active")?.as_u64()?).ok()?;
    app.session.active()?.doc.artboards.get(i).map(|a| (i, a.rect))
}

/// Where an artboard goes when the panel sets its X, Y, W or H: the reference point `refi` moves to
/// the new X/Y or stays put while the size changes (both sides together when `link` is on).
pub fn artboard_rect(r: Rect, refi: usize, x: Option<f64>, y: Option<f64>, w: Option<f64>, h: Option<f64>, link: bool) -> Rect {
    let rp = vectorcraft_geom::reference_point(r, refi);
    let (w0, h0) = (r.width(), r.height());
    let (nw, nh) = if link { constrained(w0, h0, w, h) } else { (w.unwrap_or(w0), h.unwrap_or(h0)) };
    // Where the reference point sits across the box (0, ½ or 1 of each side).
    let fx = if w0 > 1e-9 { (rp.x - r.x0) / w0 } else { 0.0 };
    let fy = if h0 > 1e-9 { (rp.y - r.y0) / h0 } else { 0.0 };
    let (x0, y0) = (x.unwrap_or(rp.x) - fx * nw, y.unwrap_or(rp.y) - fy * nh);
    Rect::new(x0, y0, x0 + nw, y0 + nh)
}

/// The reference point the Transform panel and the Control bar share (0 to 8, the center by default).
pub fn reference_index(ui: &Ui) -> usize {
    ui.data(|d| d.get_temp(egui::Id::unique("refpt"))).unwrap_or(4)
}

/// Where the Transform panel and the Properties panel's Transform section measure the selection
/// from and turn, shear and flip it about ([`panel_reference`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelReference {
    /// The reference point (0 to 8) of the selection's box that `object.setBounds` keeps in place
    /// or moves to a new X or Y, and that the locator shows.
    pub index: usize,
    /// The point on the page: that reference point, or a symbol instance's registration point.
    pub point: Point,
    /// `point` less the box's reference point `index` (zero unless the registration point lies on
    /// none of the box's nine points, as on a sheared instance).
    shift: Vec2,
}

impl PanelReference {
    /// The `x` and `y` of `object.setBounds` that put [`Self::point`] at `x`, `y` on the page.
    pub fn bounds_xy(&self, x: f64, y: f64) -> (f64, f64) {
        (x - self.shift.x, y - self.shift.y)
    }

    /// The `origin` param of the rotations, shears and reflections: [`Self::point`].
    pub fn origin(&self) -> serde_json::Value {
        json!([self.point.x, self.point.y])
    }
}

/// The reference of box `bx` of the selection for the Transform panel and the Properties panel's
/// Transform section: the shared reference point ([`reference_index`]); with Use Registration Point
/// for Symbol on and one symbol instance selected, its symbol's registration point (Symbol Options
/// › Registration) where the instance's transform puts it, with the box's reference point nearest
/// it: after a flip the art's top-left corner is the box's top-right one, after a rotation the
/// box turns with the art.
pub fn panel_reference(app: &VectorcraftApp, ui: &Ui, bx: &OrientedBox) -> PanelReference {
    let Some(q) = symbol_registration(app) else {
        let index = reference_index(ui);
        return PanelReference { index, point: bx.reference_point(index), shift: Vec2::ZERO };
    };
    // The nearest of the box's nine points, measured in the box's own (turned) frame.
    let (local, r) = (bx.to_local(q), bx.rect);
    let third = |v: f64, start: f64, len: f64| if len > 1e-9 { ((v - start) / len * 2.0).round().clamp(0.0, 2.0) as usize } else { 1 };
    let index = third(local.y, r.y0, r.height()) * 3 + third(local.x, r.x0, r.width());
    PanelReference { index, point: q, shift: q - bx.reference_point(index) }
}

/// Where the registration point of the one selected symbol instance is on the page, while Use
/// Registration Point for Symbol is on: the point of the symbol's art box on the 9-point grid,
/// placed by the instance's transform.
fn symbol_registration(app: &VectorcraftApp) -> Option<Point> {
    if !app.session.prefs.use_registration_point_for_symbol {
        return None;
    }
    let st = app.session.active()?;
    let [id] = st.selection.objects.as_slice() else { return None };
    let NodeKind::SymbolInstance { symbol, xf } = &st.doc.node(*id)?.kind else { return None };
    let own = Node::new(*id, NodeKind::SymbolInstance { symbol: symbol.clone(), xf: vectorcraft_geom::Affine::IDENTITY });
    let art = own.geometric_bounds_with(&st.doc.symbol_boxes())?;
    Some(*xf * vectorcraft_geom::reference_point(art, usize::from(st.doc.symbol_registration(symbol).0)))
}

/// The Transform panel menu's choices for what the panel transforms: the value of the
/// `transformPanelPatterns` preference each sets, and its label.
const PATTERN_MODES: [(&str, &str); 3] = [("object", "Transform Object Only"), ("pattern", "Transform Pattern Only"), ("both", "Transform Both")];

/// Run transform command `cmd` from the Transform panel or the Properties panel's Transform
/// section with `params` and the `objects` and `patterns` switches of the panel menu's Transform
/// Object Only (the object; its pattern fills and strokes stay as they are), Transform Pattern
/// Only (the pattern tiles alone) or Transform Both. Until one is chosen (`default`, or any other
/// value) neither is passed: patterns follow General › Transform Pattern Tiles. Errors are
/// dropped, as the panels' fields have always done.
pub fn run_transform(app: &mut VectorcraftApp, cmd: &str, mut params: serde_json::Value) {
    let switches = match app.session.prefs.transform_panel_patterns.as_str() {
        "object" => Some((true, false)),
        "pattern" => Some((false, true)),
        "both" => Some((true, true)),
        _ => None,
    };
    if let Some((objects, patterns)) = switches {
        params["objects"] = json!(objects);
        params["patterns"] = json!(patterns);
    }
    app.run(cmd, params).ok();
}

/// The panel menu's checked choice of [`PATTERN_MODES`]: the one chosen, else (`default`) what
/// General › Transform Pattern Tiles makes the panel do, Transform Both when it is on and
/// Transform Object Only when it is off.
fn pattern_mode(app: &VectorcraftApp) -> &'static str {
    let prefs = &app.session.prefs;
    PATTERN_MODES.iter().map(|(v, _)| *v).find(|v| *v == prefs.transform_panel_patterns).unwrap_or(if prefs.transform_pattern_tiles {
        "both"
    } else {
        "object"
    })
}

/// The shared reference point's locator; a click on it chooses another point.
pub fn reference_locator(ui: &mut Ui, refi: usize) {
    if let Some(i) = widgets::reference_point(ui, refi) {
        ui.data_mut(|d| d.insert_temp(egui::Id::unique("refpt"), i));
    }
}

/// Set artboard `index` to a new X, Y, W or H from reference point `refi` ([`artboard_rect`]) with
/// `artboard.setProps`, passing the Artboard tool's Move and Scale Artwork with Artboard options as
/// `moveArt` and `scaleArt`; an error is shown in the status bar. A new X or Y moves artboard
/// `index` alone; a new W or H resizes every selected artboard about its own reference point
/// ([`crate::panels::artboards::resized_with`]), in one undo step.
pub fn set_artboard(app: &mut VectorcraftApp, index: usize, refi: usize, xywh: [Option<f64>; 4], link: bool) {
    let [x, y, w, h] = xywh;
    let scale_art = crate::panels::artboards::scale_art(app);
    let move_art = crate::panels::artboards::move_art(app);
    let indices = if x.is_none() && y.is_none() { crate::panels::artboards::resized_with(app, index) } else { vec![index] };
    let common = json!({"scaleArt": scale_art, "moveArt": move_art});
    let r = crate::panels::artboards::set_props(app, &indices, common, |i, r| {
        let n = artboard_rect(r, refi, x, y, w, h, link);
        Some(json!({"index": i, "x": n.x0, "y": n.y0, "width": n.width(), "height": n.height()}))
    });
    if let Err(e) = r {
        app.status(e);
    }
}

/// The Artboard tool's view of the panel: the active artboard's position and size ([`set_artboard`]).
fn artboard_fields(app: &mut VectorcraftApp, ui: &mut Ui, index: usize, r: Rect) {
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    let refi = reference_index(ui);
    let link = proportional(app, ui);
    let rp = vectorcraft_geom::reference_point(r, refi);
    let set = |app: &mut VectorcraftApp, x, y, w, h| set_artboard(app, index, refi, [x, y, w, h], link);
    ui.horizontal(|ui| {
        reference_locator(ui, refi);
        ui.add_space(4.0);
        egui::Grid::new("xfp-ab-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
            widgets::dim_label(ui, "X:");
            if let Some(v) = widgets::num_field_named(ui, "xfp-ab-x", Some(crate::i18n::t("X:")), Some(rp.x - origin.x), units, 80.0) {
                set(app, Some(v + origin.x), None, None, None);
            }
            widgets::dim_label(ui, "W:");
            if let Some(v) = widgets::size_field_named(ui, "xfp-ab-w", Some(tl!("W:")), Some(r.width()), units, 80.0) {
                set(app, None, None, Some(v), None);
            }
            ui.end_row();
            widgets::dim_label(ui, "Y:");
            if let Some(v) = widgets::num_field_named(ui, "xfp-ab-y", Some(crate::i18n::t("Y:")), Some(rp.y - origin.y), units, 80.0) {
                set(app, None, Some(v + origin.y), None, None);
            }
            widgets::dim_label(ui, "H:");
            if let Some(v) = widgets::size_field_named(ui, "xfp-ab-h", Some(tl!("H:")), Some(r.height()), units, 80.0) {
                set(app, None, None, None, Some(v));
            }
            ui.end_row();
        });
        constrain_link(app, ui);
    });
}

/// With ruler guides selected and no art: the X of the vertical ones and the Y of the horizontal
/// ones (the least of each; none where no such guide is selected).
fn selected_guides(app: &VectorcraftApp) -> Option<(Option<f64>, Option<f64>)> {
    let st = app.session.active()?;
    if !st.selection.objects.is_empty() || st.selection.guides.is_empty() {
        return None;
    }
    let least = |vertical: bool| {
        st.selection.guides.iter().filter_map(|i| st.doc.guides.get(*i)).filter(|g| g.vertical == vertical).map(|g| g.pos).min_by(f64::total_cmp)
    };
    Some((least(true), least(false)))
}

/// The selected ruler guides' view of the panel (#1597): X moves the vertical ones and Y the
/// horizontal ones, together.
fn guide_fields(app: &mut VectorcraftApp, ui: &mut Ui, (x, y): (Option<f64>, Option<f64>)) {
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    egui::Grid::new("xfp-guide-grid").num_columns(2).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
        for (label, id, pos, origin, by) in [("X:", "xfp-guide-x", x, origin.x, "dx"), ("Y:", "xfp-guide-y", y, origin.y, "dy")] {
            widgets::dim_label(ui, label);
            ui.add_enabled_ui(pos.is_some(), |ui| {
                if let (Some(v), Some(pos)) = (widgets::num_field_named(ui, id, Some(tl!(label)), pos.map(|p| p - origin), units, 80.0), pos)
                    && let Err(e) = app.run("guide.move", json!({ by: v + origin - pos }))
                {
                    app.status(e);
                }
            });
            ui.end_row();
        }
    });
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        widgets::dim_label(ui, tl!("No document"));
        return;
    }
    if let Some((i, r)) = tool_artboard(app) {
        artboard_fields(app, ui, i, r);
        return;
    }
    if let Some(guides) = selected_guides(app) {
        guide_fields(app, ui, guides);
        return;
    }
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    // The bounding box, rotated with rotated objects: W/H are its own sides, X/Y its reference
    // point on the page.
    let bx = app.selection_box();
    let bounds = bx.map(|b| b.rect);
    let reference = bx.map(|b| panel_reference(app, ui, &b));
    let refi = reference.map_or_else(|| reference_index(ui), |r| r.index);
    // `object.setBounds`'s x and y that put the reference (the registration point) at x, y.
    let to_box = |x: f64, y: f64| reference.map_or((x, y), |r| r.bounds_xy(x, y));
    let link = proportional(app, ui);
    let copy = alt_enter_copies(app, ui);
    let has = bounds.is_some();
    let rp = reference.map(|r| r.point);
    ui.horizontal(|ui| {
        if let Some(i) = widgets::reference_point(ui, refi) {
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("refpt"), i));
        }
        ui.add_space(4.0);
        ui.add_enabled_ui(has, |ui| {
            egui::Grid::new("xfp-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
                widgets::dim_label(ui, "X:");
                if let Some(v) = widgets::num_field_named(ui, "xfp-x", Some(crate::i18n::t("X:")), rp.map(|p| p.x - origin.x), units, 80.0) {
                    run_transform(app, "object.setBounds", json!({"x": to_box(v + origin.x, 0.0).0, "reference": refi, "copy": copy}));
                }
                widgets::dim_label(ui, "W:");
                if let Some(v) = widgets::size_field_named(ui, "xfp-w", Some(tl!("W:")), bounds.map(|b| b.width()), units, 80.0) {
                    run_transform(app, "object.setBounds", json!({"width": v, "reference": refi, "proportional": link, "copy": copy}));
                }
                ui.end_row();
                widgets::dim_label(ui, "Y:");
                if let Some(v) = widgets::num_field_named(ui, "xfp-y", Some(crate::i18n::t("Y:")), rp.map(|p| p.y - origin.y), units, 80.0) {
                    run_transform(app, "object.setBounds", json!({"y": to_box(0.0, v + origin.y).1, "reference": refi, "copy": copy}));
                }
                widgets::dim_label(ui, "H:");
                if let Some(v) = widgets::size_field_named(ui, "xfp-h", Some(tl!("H:")), bounds.map(|b| b.height()), units, 80.0) {
                    run_transform(app, "object.setBounds", json!({"height": v, "reference": refi, "proportional": link, "copy": copy}));
                }
                ui.end_row();
            });
        });
        constrain_link(app, ui);
    });
    ui.add_space(4.0);
    let origin = reference.map(|r| r.origin());
    ui.horizontal(|ui| {
        ui.add_enabled_ui(has, |ui| {
            let icon = icons::icon(ui, "rotate-ccw", 16.0, t.icon).on_hover_text(tl!("Rotate"));
            crate::scrub::note_label(ui, icon.rect);
            // The bounding box's angle: a new value turns the selection to it.
            let angle = bx.map_or(0.0, |b| b.angle);
            if let Some(a) = widgets::spin_plain_named(ui, "xfp-rot", Some(tl!("Rotate")), angle, "°", 2, 96.0, 15.0, -360.0, &ANGLE_PRESETS)
                && a != angle
            {
                run_transform(app, "object.rotate", json!({"angle": a, "absolute": true, "origin": origin, "copy": copy}));
            }
            ui.add_space(4.0);
            let icon = icons::icon(ui, "dc-shear", 16.0, t.icon).on_hover_text(tl!("Shear"));
            crate::scrub::note_label(ui, icon.rect);
            // Shear is relative: the field rests at 0° and a new value shears by it.
            if let Some(a) = widgets::spin_plain_named(ui, "xfp-shear", Some(tl!("Shear")), 0.0, "°", 2, 96.0, 15.0, -89.0, &SHEAR_PRESETS)
                && a != 0.0
                && a.abs() < 90.0
            {
                run_transform(app, "object.shear", json!({"angle": a, "axis": "horizontal", "origin": origin, "copy": copy}));
            }
        });
    });
    ui.horizontal(|ui| {
        // Lucide's flip icons are named for their mirror line: flip-vertical-2 draws a vertical
        // line (the left-right flip), flip-horizontal-2 a horizontal one (top-bottom).
        if widgets::icon_button_enabled(ui, "flip-vertical-2", tl!("Flip Horizontal"), false, has, 24.0).clicked() {
            run_transform(app, "object.reflect", json!({"axis": "vertical", "origin": origin}));
        }
        if widgets::icon_button_enabled(ui, "flip-horizontal-2", tl!("Flip Vertical"), false, has, 24.0).clicked() {
            run_transform(app, "object.reflect", json!({"axis": "horizontal", "origin": origin}));
        }
    });
    // Live shape properties (a path with live corners is no shape).
    if let Some(n) = first_selected(app)
        && let NodeKind::Path { live: Some(live), .. } = &n.kind
        && !matches!(**live, vectorcraft_doc::LiveShape::Path { .. })
    {
        widgets::divider(ui);
        match &**live {
            vectorcraft_doc::LiveShape::Rectangle { .. } => {
                widgets::subheader(ui, tl!("Rectangle Properties:"));
                corner_radius_row(app, ui, &n, "xfp-radius");
            }
            vectorcraft_doc::LiveShape::Polygon { .. } => {
                widgets::subheader(ui, tl!("Polygon Properties:"));
                polygon_rows(app, ui, &n, live, "xfp-polygon");
            }
            vectorcraft_doc::LiveShape::Ellipse { pie, .. } => {
                widgets::subheader(ui, tl!("Ellipse Properties:"));
                pie_rows(app, ui, *pie, "xfp-pie");
            }
            _ => {
                widgets::subheader(ui, tl!("Shape Properties:"));
                widgets::dim_label(ui, tl!(n.kind_label()));
            }
        }
    }
    if pstate::<bool>(ui.ctx(), "xf-hide-options") {
        return;
    }
    widgets::divider(ui);
    let (sc, ss) = (app.session.prefs.scale_corners, app.session.prefs.scale_strokes);
    if widgets::check(ui, tl!("Scale Corners"), sc, true) {
        set_pref(app, "scaleCorners", !sc);
    }
    if widgets::check(ui, tl!("Scale Strokes & Effects"), ss, true) {
        set_pref(app, "scaleStrokes", !ss);
    }
}

/// An ellipse's Pie Start and End Angle (degrees, counterclockwise from 3 o'clock; 0 to 360 is the
/// whole ellipse) and Invert Pie, which shows the other part of it (Transform and Properties panels).
pub(crate) fn pie_rows(app: &mut VectorcraftApp, ui: &mut Ui, pie: (f64, f64), id: &str) {
    for (label, key, v) in [(tl!("Pie Start Angle:"), "pieStart", pie.0), (tl!("Pie End Angle:"), "pieEnd", pie.1)] {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, label);
            if let Some(a) = widgets::plain_field(ui, (id, key), v, "°", 1, 60.0) {
                app.run("object.setLiveShape", json!({ key: a })).ok();
            }
        });
    }
    if widgets::flat_button(ui, tl!("Invert Pie"), 90.0).clicked() {
        app.run("object.setLiveShape", json!({"invertPie": true})).ok();
    }
}

/// A live polygon's Polygon Properties (Transform and Properties panels): its side count, angle
/// (counterclockwise), corner type and radius, radius (centre to vertex) and side length, and Make
/// Sides Equal, which an uneven scale enables.
pub(crate) fn polygon_rows(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, live: &LiveShape, id: &str) {
    let LiveShape::Polygon { sides, .. } = live else { return };
    let (units, fw) = (app.session.general_unit(), super::field_width(ui));
    let radius = live.polygon_radius();
    let side = radius.map(|r| 2.0 * r * (std::f64::consts::PI / f64::from((*sides).max(3))).sin());
    let kind = super::corner_style(app, n).1;
    let kinds: Vec<&str> = CornerKind::ALL.iter().map(|k| corner_kind(*k)).collect();
    // The `object.setLiveShape` param an edited field sets.
    let mut set = None;
    egui::Grid::new((id, "grid")).num_columns(2).spacing([6.0, 6.0]).show(ui, |ui| {
        widgets::dim_label(ui, tl!("Sides:"));
        if let Some(s) = widgets::range_field_named(ui, (id, "sides"), Some(tl!("Sides:")), f64::from(*sides), 3.0..=f64::INFINITY, "", 0, fw) {
            set = Some(("sides", json!(s as u64)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Angle:"));
        if let Some(a) = widgets::plain_field_named(ui, (id, "angle"), Some(tl!("Angle:")), live.polygon_angle().unwrap_or(0.0), "°", 1, fw) {
            set = Some(("polygonAngle", json!(a)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Corner:"));
        if let Some(k) = widgets::dropdown_names_named(ui, (id, "kind"), Some(tl!("Corner:")), kind.map_or("", corner_kind), &kinds, fw)
            .and_then(|i| CornerKind::ALL.get(i))
        {
            set = Some(("kind", json!(k)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Corner Radius:"));
        super::corner_radius_field(app, ui, n, (id, "cornerRadius"), fw);
        ui.end_row();
        widgets::dim_label(ui, tl!("Radius:"));
        if let Some(r) = widgets::num_field_named(ui, (id, "radius"), Some(tl!("Radius:")), radius, units, fw) {
            set = Some(("polygonRadius", json!(r)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Side Length:"));
        if let Some(l) = widgets::num_field_named(ui, (id, "side"), Some(tl!("Side Length:")), side, units, fw) {
            set = Some(("sideLength", json!(l)));
        }
        ui.end_row();
    });
    if ui.add_enabled_ui(!live.polygon_sides_equal(), |ui| widgets::flat_button(ui, tl!("Make Sides Equal"), 130.0)).inner.clicked() {
        set = Some(("makeSidesEqual", json!(true)));
    }
    if let Some((key, v)) = set {
        app.run("object.setLiveShape", json!({ key: v })).ok();
    }
}

/// The Control bar's underlined "Transform" link: a click toggles the Transform panel in a
/// popover under it, as the Stroke link does with the Stroke panel.
pub fn link(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let resp = ui.link(egui::RichText::new(tl!("Transform")).size(12.0).color(t.text).underline()).on_hover_text(tl!("Transform options"));
    widgets::popover(&resp, resp.clicked(), |ui| {
        ui.set_width(300.0);
        show(app, ui);
    });
}

/// The link between W and H (Transform panel, Properties panel, Control bar): one toggle, the
/// `constrainProportions` preference, which the size fields pass on as `proportional`.
pub fn constrain_link(app: &mut VectorcraftApp, ui: &mut Ui) {
    let on = app.session.prefs.constrain_proportions;
    if widgets::icon_button(ui, if on { "link" } else { "link-2-off" }, tl!("Constrain Width and Height Proportions"), on, 22.0).clicked() {
        set_pref(app, "constrainProportions", !on);
    }
}

/// The `copy` param of a command that a field of the Transform panel, the Properties panel's
/// Transform section or the Control bar runs: true when Alt+Enter (Option+Return) applied the value
/// and no anchors are selected with Direct Selection.
pub fn alt_enter_copies(app: &VectorcraftApp, ui: &Ui) -> bool {
    widgets::enter_modifiers(ui).is_some_and(|m| m.alt) && app.session.active().is_some_and(|st| st.selection.anchors.is_empty())
}

/// The `proportional` param of the W and H fields of the Transform panel (the artboard's too), the
/// Properties panel's Transform section and the Control bar: the Constrain Width and Height
/// Proportions link, or true when Ctrl/Cmd+Enter applied the value.
pub fn proportional(app: &VectorcraftApp, ui: &Ui) -> bool {
    app.session.prefs.constrain_proportions || widgets::enter_modifiers(ui).is_some_and(|m| m.command)
}

/// Toggle a boolean preference (through `prefs.set`, as agents do).
pub fn set_pref(app: &mut VectorcraftApp, key: &str, on: bool) {
    if let Err(e) = app.run("prefs.set", json!({ "key": key, "value": on })) {
        app.status(e);
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "xf-hide-options");
    let has = super::selection_len(app) > 0;
    // The flips mirror across the panel's reference point, as its flip buttons do.
    let origin = app.selection_box().map(|b| panel_reference(app, ui, &b).origin());
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "xf-hide-options", !hidden);
    }
    ui.separator();
    if menu_item(ui, tl!("Flip Horizontal"), has, false) {
        run_transform(app, "object.reflect", json!({"axis": "vertical", "origin": origin}));
    }
    if menu_item(ui, tl!("Flip Vertical"), has, false) {
        run_transform(app, "object.reflect", json!({"axis": "horizontal", "origin": origin}));
    }
    ui.separator();
    let ss = app.session.prefs.scale_strokes;
    if menu_item(ui, tl!("Scale Strokes & Effects"), true, ss) {
        set_pref(app, "scaleStrokes", !ss);
    }
    ui.separator();
    // What the panel transforms (Transform Object Only / Pattern Only / Both), one checked.
    let current = pattern_mode(app);
    for (value, label) in PATTERN_MODES {
        if menu_item(ui, label, true, value == current)
            && let Err(e) = app.run("prefs.set", json!({"key": "transformPanelPatterns", "value": value}))
        {
            app.status(e);
        }
    }
    ui.separator();
    let reg = app.session.prefs.use_registration_point_for_symbol;
    if menu_item(ui, tl!("Use Registration Point for Symbol"), true, reg) {
        set_pref(app, "useRegistrationPointForSymbol", !reg);
    }
}

#[cfg(test)]
mod tests;
