//! Transform panel: reference point, X/Y/W/H with the constrain link, rotate and shear, flips,
//! live-shape properties and the Scale Corners / Scale Strokes & Effects / Align to Pixel Grid
//! options. While the Artboard tool is in use it shows and edits the active artboard's X/Y/W/H
//! instead (a new W or H resizes every selected artboard), and with only ruler guides selected
//! their X (vertical guides) and Y (horizontal ones).

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{LiveShape, Node, NodeKind, OrientedBox};
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_geom::{Point, Rect, Vec2};

use super::{corner_radius_row, first_selected, pstate, set_pstate};
use crate::caches::PixelAligned;
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
    /// Whether a symbol instance's registration point decides the reference (Use Registration
    /// Point for Symbol), so the locator shows it and offers no other point.
    pub fixed: bool,
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
        return PanelReference { index, point: bx.reference_point(index), fixed: false, shift: Vec2::ZERO };
    };
    // The nearest of the box's nine points, measured in the box's own (turned) frame.
    let (local, r) = (bx.to_local(q), bx.rect);
    let third = |v: f64, start: f64, len: f64| if len > 1e-9 { ((v - start) / len * 2.0).round().clamp(0.0, 2.0) as usize } else { 1 };
    let index = third(local.y, r.y0, r.height()) * 3 + third(local.x, r.x0, r.width());
    PanelReference { index, point: q, fixed: true, shift: q - bx.reference_point(index) }
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
pub fn run_transform(app: &mut VectorcraftApp, cmd: &str, params: serde_json::Value) {
    try_transform(app, cmd, params).ok();
}

/// [`run_transform`] that returns the command's result, for a field that shows its error.
fn try_transform(app: &mut VectorcraftApp, cmd: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let params = with_pattern_switches(app, params);
    app.run(cmd, params)
}

/// `params` with the `objects` and `patterns` switches [`run_transform`] adds.
pub fn with_pattern_switches(app: &VectorcraftApp, mut params: serde_json::Value) -> serde_json::Value {
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
    params
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

/// The locator for a selection's reference `reference` ([`panel_reference`]): a click on it chooses
/// another point, except while a symbol instance's registration point decides the reference (Use
/// Registration Point for Symbol). The locator then shows that point read-only and says why: a
/// click's choice could not move the instance, and would wait to move the next ordinary selection
/// instead.
pub fn selection_reference_locator(ui: &mut Ui, reference: &PanelReference) {
    if !reference.fixed {
        return reference_locator(ui, reference.index);
    }
    let why = tl!("Use Registration Point for Symbol: the registration point is the reference point");
    ui.add_enabled_ui(false, |ui| {
        widgets::reference_point(ui, reference.index);
    })
    .response
    .on_disabled_hover_text(why);
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

/// The Artboard tool's view of the panel, and the Properties panel's Transform section in Edit
/// Artboards: the reference point, artboard `index`'s position and size at `r` in fields
/// `fw` wide ([`set_artboard`]), and the Constrain Width and Height Proportions link.
pub(crate) fn artboard_fields(app: &mut VectorcraftApp, ui: &mut Ui, index: usize, r: Rect, fw: f32) {
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
            if let Some(v) = widgets::num_field_named(ui, "xfp-ab-x", Some(crate::i18n::t("X:")), Some(rp.x - origin.x), units, fw) {
                set(app, Some(v + origin.x), None, None, None);
            }
            widgets::dim_label(ui, "W:");
            if let Some(v) = widgets::size_field_named(ui, "xfp-ab-w", Some(tl!("W:")), Some(r.width()), units, fw) {
                set(app, None, None, Some(v), None);
            }
            ui.end_row();
            widgets::dim_label(ui, "Y:");
            if let Some(v) = widgets::num_field_named(ui, "xfp-ab-y", Some(crate::i18n::t("Y:")), Some(rp.y - origin.y), units, fw) {
                set(app, None, Some(v + origin.y), None, None);
            }
            widgets::dim_label(ui, "H:");
            if let Some(v) = widgets::size_field_named(ui, "xfp-ab-h", Some(tl!("H:")), Some(r.height()), units, fw) {
                set(app, None, None, None, Some(v));
            }
            ui.end_row();
        });
        constrain_link(app, ui);
    });
}

/// With ruler guides selected and no art: the X of the vertical ones and the Y of the horizontal
/// ones (the least of each; none where no such guide is selected).
pub(crate) fn selected_guides(app: &VectorcraftApp) -> Option<(Option<f64>, Option<f64>)> {
    let st = app.session.active()?;
    if !st.selection.objects.is_empty() || st.selection.guides.is_empty() {
        return None;
    }
    let least = |vertical: bool| {
        st.selection.guides.iter().filter_map(|i| st.doc.ruler_guide(*i)).filter(|g| g.vertical == vertical).map(|g| g.pos).min_by(f64::total_cmp)
    };
    Some((least(true), least(false)))
}

/// The selected ruler guides' view of the panel (#1597), also the Properties panel's Transform
/// section for them: X moves the vertical ones and Y the horizontal ones, together. The fields
/// are `width` wide.
pub(crate) fn guide_fields(app: &mut VectorcraftApp, ui: &mut Ui, (x, y): (Option<f64>, Option<f64>), width: f32) {
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    egui::Grid::new("xfp-guide-grid").num_columns(2).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
        for (label, id, pos, origin, by) in [("X:", "xfp-guide-x", x, origin.x, "dx"), ("Y:", "xfp-guide-y", y, origin.y, "dy")] {
            widgets::dim_label(ui, label);
            ui.add_enabled_ui(pos.is_some(), |ui| {
                if let (Some(v), Some(pos)) = (widgets::num_field_named(ui, id, Some(tl!(label)), pos.map(|p| p - origin), units, width), pos)
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
        artboard_fields(app, ui, i, r, 80.0);
        return;
    }
    if let Some(guides) = selected_guides(app) {
        guide_fields(app, ui, guides, 80.0);
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
        match &reference {
            Some(r) => selection_reference_locator(ui, r),
            None => reference_locator(ui, refi),
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
            // Shear is relative: the field rests at 0° and a new value from -359° to 359° shears
            // by it. The field has no minimum, so `object.shear` checks every value: it refuses
            // ±90°, ±270° and a value outside -359° to 359°, and the status bar shows its error.
            if let Some(a) =
                widgets::spin_plain_named(ui, "xfp-shear", Some(tl!("Shear")), 0.0, "°", 2, 96.0, 15.0, f64::NEG_INFINITY, &SHEAR_PRESETS)
                && a != 0.0
                && let Err(e) = try_transform(app, "object.shear", json!({"angle": a, "axis": "horizontal", "origin": origin, "copy": copy}))
            {
                app.status(e);
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
        if !live_shape_rows(app, ui, &n, live, "xfp", true) {
            widgets::subheader(ui, tl!("Shape Properties:"));
            widgets::dim_label(ui, tl!(n.kind_label()));
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
    pixel_grid_check(app, ui);
}

/// The selected objects' Align to Pixel Grid: on or off when they agree, `None` when they differ,
/// with whether every one can be aligned (a path that is not a guide and not inside a compound path,
/// or a compound path). `None` with nothing selected.
fn pixel_aligned_state(app: &VectorcraftApp) -> Option<PixelAligned> {
    let st = app.session.active()?;
    let nodes: Vec<&Node> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).collect();
    let first = nodes.first()?.extras().pixel_aligned;
    let value = nodes.iter().all(|n| n.extras().pixel_aligned == first).then_some(first);
    let in_compound = |n: &Node| st.doc.parent_of(n.id).and_then(|p| st.doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
    let alignable = nodes.iter().all(|n| matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) && !in_compound(n));
    Some((value, alignable))
}

/// [`pixel_aligned_state`], measured once per (document uid, revision)
/// ([`crate::caches::CanvasCache::pixel_aligned`]): the panel reads it every frame, and Select All
/// can select hundreds of thousands of paths.
fn pixel_aligned_shown(app: &mut VectorcraftApp) -> Option<PixelAligned> {
    let st = app.session.active()?;
    let key = (st.uid, st.revision);
    if let Some((k, state)) = app.canvas.pixel_aligned
        && k == key
    {
        return state;
    }
    let state = pixel_aligned_state(app);
    app.canvas.pixel_aligned = Some((key, state));
    state
}

/// The Align to Pixel Grid checkbox (`object.setPixelAligned`): a dash when the selected objects
/// differ, and unavailable unless every selected object can be aligned. A click turns it on for
/// them all, or off when it is on for them all.
fn pixel_grid_check(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (value, alignable) = pixel_aligned_shown(app).unwrap_or((Some(false), false));
    if !widgets::check3(ui, tl!("Align to Pixel Grid"), value, alignable) {
        return;
    }
    let ids = pixel_aligned_ids(app);
    if let Err(e) = app.run("object.setPixelAligned", json!({"ids": ids, "on": value != Some(true)})) {
        app.status(e);
    }
}

/// The ids of the selected objects that are in the document, for `object.setPixelAligned`: read
/// when the Align to Pixel Grid checkbox is clicked.
fn pixel_aligned_ids(app: &VectorcraftApp) -> Vec<u64> {
    app.session.active().map(|st| st.selection.objects.iter().filter(|id| st.doc.node(**id).is_some()).map(|id| id.0).collect()).unwrap_or_default()
}

/// A live rectangle's, polygon's, ellipse's or line's properties (Transform and Properties panels,
/// and for the first three the Contextual Task Bar), their widget ids under `id`. `headed` adds
/// the Rectangle and Polygon Properties headings; an ellipse's and a line's heading always show.
/// Returns false, drawing nothing, for other live shapes, and for a line unless every selected
/// object is one.
pub(crate) fn live_shape_rows(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node, live: &LiveShape, id: &str, headed: bool) -> bool {
    match live {
        LiveShape::Rectangle { .. } => {
            if headed {
                widgets::subheader(ui, tl!("Rectangle Properties:"));
            }
            let fw = super::field_width(ui);
            shape_angle_row(app, ui, live, &format!("{id}-rectangle"), "rectangleAngle", fw);
            corner_radius_row(app, ui, n, &format!("{id}-radius"));
        }
        LiveShape::Polygon { .. } => {
            if headed {
                widgets::subheader(ui, tl!("Polygon Properties:"));
            }
            polygon_rows(app, ui, n, live, &format!("{id}-polygon"));
        }
        LiveShape::Ellipse { pie, .. } => {
            widgets::subheader(ui, tl!("Ellipse Properties:"));
            shape_angle_row(app, ui, live, &format!("{id}-ellipse"), "ellipseAngle", 60.0);
            pie_rows(app, ui, *pie, &format!("{id}-pie"));
        }
        LiveShape::Line { .. } => return line_rows(app, ui, live, &format!("{id}-line")),
        _ => return false,
    }
    true
}

/// A live rectangle's or ellipse's Angle row (Rectangle and Ellipse Angle, counterclockwise): the
/// angle the selected shapes of its kind share, blank when they differ. `key` is the
/// `object.setLiveShape` param an edit sets, which turns each of them about its own center.
fn shape_angle_row(app: &mut VectorcraftApp, ui: &mut Ui, live: &LiveShape, id: &str, key: &str, width: f32) {
    let angle = shared_shape_angle(app, ui.ctx(), live);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Angle:"));
        if let Some(a) = widgets::mixed_field_named(ui, (id, "angle"), Some(tl!("Angle:")), angle, "°", 1, width) {
            app.run("object.setLiveShape", json!({ key: a })).ok();
        }
    });
}

/// The selected live shapes of `live`'s kind, the document uid and revision they were read at, and
/// the angle they share.
type SharedAngle = Option<((u64, u64, std::mem::Discriminant<LiveShape>), Option<f64>)>;

/// The angle ([`LiveShape::shape_angle`]) every selected live shape of `live`'s kind shares, `None`
/// when they differ or none is selected. Measured once per (document uid, revision) and kind: the
/// panels read it every frame, and Select All can select hundreds of thousands of shapes.
fn shared_shape_angle(app: &VectorcraftApp, ctx: &egui::Context, live: &LiveShape) -> Option<f64> {
    let st = app.session.active()?;
    let key = (st.uid, st.revision, std::mem::discriminant(live));
    if let Some((k, angle)) = pstate::<SharedAngle>(ctx, "shape-angle")
        && k == key
    {
        return angle;
    }
    let mut angles = st
        .selection
        .objects
        .iter()
        .filter_map(|id| st.doc.node(*id)?.live_shape())
        .filter(|l| std::mem::discriminant(*l) == key.2)
        .filter_map(LiveShape::shape_angle);
    // With no shape of its kind selected, the panel shows `live`'s own angle.
    let angle = match angles.next() {
        None => live.shape_angle(),
        Some(first) => angles
            .all(|a| {
                let d = (a - first).rem_euclid(360.0);
                d.min(360.0 - d) < 1e-9
            })
            .then_some(first),
    };
    set_pstate::<SharedAngle>(ctx, "shape-angle", Some((key, angle)));
    angle
}

/// Whether every selected object is a live line (and one is selected).
fn only_lines_selected(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let line = |n: &Node| matches!(&n.kind, NodeKind::Path { live: Some(l), .. } if matches!(**l, LiveShape::Line { .. }));
    !st.selection.objects.is_empty() && st.selection.objects.iter().all(|o| st.doc.node(*o).is_some_and(line))
}

/// A live line's Line Properties (Transform and Properties panels), under its heading: its length,
/// end to end, and its angle (counterclockwise from 3 o'clock, from its first end to its second). A new length keeps each selected line's first end in place, and a
/// new angle turns each one about its midpoint. Returns false, drawing nothing, unless every
/// selected object is a live line.
pub(crate) fn line_rows(app: &mut VectorcraftApp, ui: &mut Ui, live: &LiveShape, id: &str) -> bool {
    if !only_lines_selected(app) {
        return false;
    }
    let (units, fw) = (app.session.general_unit(), super::field_width(ui));
    widgets::subheader(ui, tl!("Line Properties:"));
    // The `object.setLiveShape` params the edited fields set, all sent in one call.
    let mut set = serde_json::Map::new();
    egui::Grid::new((id, "grid")).num_columns(2).spacing([6.0, 6.0]).show(ui, |ui| {
        widgets::dim_label(ui, tl!("Line Length:"));
        if let Some(l) = widgets::num_field_named(ui, (id, "length"), Some(tl!("Line Length:")), live.line_length(), units, fw) {
            set.insert("lineLength".into(), json!(l));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Line Angle:"));
        if let Some(a) = widgets::plain_field_named(ui, (id, "angle"), Some(tl!("Line Angle:")), live.line_angle().unwrap_or(0.0), "°", 1, fw) {
            set.insert("lineAngle".into(), json!(a));
        }
        ui.end_row();
    });
    if !set.is_empty()
        && let Err(e) = app.run("object.setLiveShape", serde_json::Value::Object(set))
    {
        app.status(e);
    }
    true
}

/// An ellipse's Pie Start and End Angle (degrees, counterclockwise from 3 o'clock; 0 to 360 is the
/// whole ellipse), the Constrain Pie Angle link beside them, and Invert Pie, which shows the other
/// part of it (Transform and Properties panels). With the link on, a typed angle moves the other
/// one by as much, so each selected ellipse keeps its span.
pub(crate) fn pie_rows(app: &mut VectorcraftApp, ui: &mut Ui, pie: (f64, f64), id: &str) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            for (label, key, v) in [(tl!("Pie Start Angle:"), "pieStart", pie.0), (tl!("Pie End Angle:"), "pieEnd", pie.1)] {
                ui.horizontal(|ui| {
                    widgets::dim_label(ui, label);
                    if let Some(a) = widgets::plain_field(ui, (id, key), v, "°", 1, 60.0) {
                        let params = if app.ui.constrain_pie_angle { json!({ key: a, "constrainPie": true }) } else { json!({ key: a }) };
                        app.run("object.setLiveShape", params).ok();
                    }
                });
            }
        });
        pie_link(app, ui);
    });
    if widgets::flat_button(ui, tl!("Invert Pie"), 90.0).clicked() {
        app.run("object.setLiveShape", json!({"invertPie": true})).ok();
    }
}

/// The Constrain Pie Angle link between Pie Start and End Angle: one toggle for the Transform and
/// Properties panels, kept while the app runs and off at launch. It is drawn after the fields, so
/// Tab reaches them first; with the keyboard on it, Space or Enter toggles it.
fn pie_link(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (_, at) = ui.allocate_exact_size(egui::Vec2::splat(CONSTRAIN_LINK_SIZE), egui::Sense::click());
    let on = app.ui.constrain_pie_angle;
    let tip = tl!("Constrain Pie Angle");
    let keyed = at.has_focus()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space) | i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if widgets::paint_icon_button(ui, at, if on { "link" } else { "link-2-off" }, tip, on).clicked() || keyed {
        app.ui.constrain_pie_angle = !on;
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
    let angle = shared_shape_angle(app, ui.ctx(), live);
    // The `object.setLiveShape` param an edited field sets.
    let mut set = None;
    egui::Grid::new((id, "grid")).num_columns(2).spacing([6.0, 6.0]).show(ui, |ui| {
        widgets::dim_label(ui, tl!("Sides:"));
        if let Some(s) = widgets::range_field_named(ui, (id, "sides"), Some(tl!("Sides:")), f64::from(*sides), 3.0..=1000.0, "", 0, fw) {
            set = Some(("sides", json!(s as u64)));
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Angle:"));
        if let Some(a) = widgets::mixed_field_named(ui, (id, "angle"), Some(tl!("Angle:")), angle, "°", 1, fw) {
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
    let resp = ui.link(link_text(ui)).on_hover_text(tl!("Transform options"));
    popover(app, &resp);
}

/// The Transform panel in a popover under `resp`, which a click on it toggles: the Control bar's
/// Transform [`link`], and its X:, Y:, W: and H: labels.
pub fn popover(app: &mut VectorcraftApp, resp: &egui::Response) {
    widgets::popover(resp, resp.clicked(), |ui| {
        ui.set_width(300.0);
        show(app, ui);
    });
}

/// The Transform link's text, as [`link`] draws it and [`link_width`] measures it.
fn link_text(ui: &Ui) -> egui::RichText {
    egui::RichText::new(tl!("Transform")).size(12.0).color(Tokens::get(ui.ctx()).text).underline()
}

/// The width of the Transform [`link`] in `ui`: its text, and the point that the underline's stroke
/// and the text's placement on the pixel grid can add at its right end.
pub fn link_width(ui: &Ui) -> f32 {
    widgets::text_width(ui, link_text(ui)) + 1.0
}

/// The width and height of [`constrain_link`]'s button.
pub const CONSTRAIN_LINK_SIZE: f32 = 22.0;

/// The link between W and H (Transform panel, Properties panel, Control bar): one toggle, the
/// `constrainProportions` preference, which the size fields pass on as `proportional`. It is drawn
/// after the fields it sits between, so Tab reaches the fields first and the link after them.
pub fn constrain_link(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (_, resp) = ui.allocate_exact_size(egui::Vec2::splat(CONSTRAIN_LINK_SIZE), egui::Sense::click());
    constrain_link_at(app, ui, resp);
}

/// [`constrain_link`]'s toggle, made over `at`: the Control bar keeps the link's place between W
/// and H but makes it after H, so Tab goes from W straight to H and finds the link after it. With
/// the keyboard on it, Space or Enter toggles it, as a dialog's button takes them.
pub fn constrain_link_at(app: &mut VectorcraftApp, ui: &mut Ui, at: egui::Response) {
    let on = app.session.prefs.constrain_proportions;
    let tip = tl!("Constrain Width and Height Proportions");
    let keyed = at.has_focus()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space) | i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if widgets::paint_icon_button(ui, at, if on { "link" } else { "link-2-off" }, tip, on).clicked() || keyed {
        set_pref(app, "constrainProportions", !on);
    }
}

/// The room the Control bar keeps for [`constrain_link_at`] between its W and H fields.
pub fn constrain_link_room(ui: &mut Ui) -> egui::Rect {
    ui.allocate_exact_size(egui::Vec2::splat(CONSTRAIN_LINK_SIZE), egui::Sense::hover()).0
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

/// Show on Shape Creation (`showOnShapeCreation`): after a shape was drawn
/// ([`vectorcraft_engine::Session::take_shape_drawn`]), the Transform panel shows as Window ›
/// Transform shows it, unless it can be seen already. In Presentation Mode, or with the option off,
/// the panels stay as they are. Called once per frame, after the canvas and the dialogs; a frame
/// that shows the panel asks `ctx` for another, which draws it.
pub fn show_on_shape_creation(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let drawn = app.session.take_shape_drawn();
    if !drawn || !app.session.prefs.show_on_shape_creation || app.ui.screen_mode >= 3 || crate::panel_docking::visible(&app.ui, "transform") {
        return;
    }
    match app.run("window.panel", json!({ "panel": "transform" })) {
        Ok(_) => ctx.request_repaint(),
        Err(e) => app.status(e),
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
    let on_creation = app.session.prefs.show_on_shape_creation;
    if menu_item(ui, tl!("Show on Shape Creation"), true, on_creation) {
        set_pref(app, "showOnShapeCreation", !on_creation);
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
    // The panel's values and the bounding box measure point type by its glyphs' ink.
    let glyphs = app.session.prefs.type_glyph_bounds;
    if menu_item(ui, tl!("Measure Type by Glyph Bounds"), true, glyphs) {
        set_pref(app, "typeGlyphBounds", !glyphs);
    }
    // The active document's Align New Objects to Pixel Grid (`document.setup`).
    let align_new = app.session.active().map(|st| st.doc.setup.align_new_objects_to_pixel_grid);
    if menu_item(ui, tl!("Align New Objects to Pixel Grid"), align_new.is_some(), align_new == Some(true))
        && let Err(e) = app.run("document.setup", json!({"alignNewObjectsToPixelGrid": align_new != Some(true)}))
    {
        app.status(e);
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
