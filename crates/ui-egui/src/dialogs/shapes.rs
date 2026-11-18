//! Shape size dialogs (a click with a shape tool): Rectangle, Rounded Rectangle, Ellipse, Polygon,
//! Star, Line Segment, Arc and Spiral tool options, and the two grids (size, divider counts and skews, switches).
//! The Line Segment, Arc and grid dialogs are also their tools' Tool Options (a double-click on the
//! tool, [`open_options`]): OK then sets the options the tool's next drag and click use, and draws
//! nothing. Arc Segment Tool Options also shows a picture of the arc its fields describe.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_geom::{Point, Rect};

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::panels::transform::artboard_rect;
use crate::state::Dialog;
use crate::widgets;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| title(d).into(),
    body,
    confirm,
    inputs: |kind| if kind == "rectangle" { &["radius"] } else { &[] },
    ..DialogSpec::FORM
};

/// The field that marks a dialog opened as its tool's Tool Options rather than by a click.
const TOOL_OPTIONS: &str = "__toolOptions";

/// The tools whose Tool Options are this dialog.
pub(crate) const OPTION_TOOLS: [&str; 4] = ["rectangularGrid", "polarGrid", "arc", "lineSegment"];

fn title(d: &Dialog) -> &'static str {
    let options = d.bool(TOOL_OPTIONS);
    match d.kind.as_str() {
        "rectangle" => tl!("Rectangle"),
        "roundedRectangle" => tl!("Rounded Rectangle"),
        "ellipse" => tl!("Ellipse"),
        "polygon" => tl!("Polygon"),
        "star" => tl!("Star"),
        "spiral" => tl!("Spiral"),
        "rectangularGrid" if options => tl!("Rectangular Grid Tool Options"),
        "rectangularGrid" => tl!("Rectangular Grid"),
        "polarGrid" if options => tl!("Polar Grid Tool Options"),
        "polarGrid" => tl!("Polar Grid"),
        "arc" => tl!("Arc Segment Tool Options"),
        _ => tl!("Line Segment Tool Options"),
    }
}

/// Open `tool`'s Tool Options (one of [`OPTION_TOOLS`]), as a double-click on the tool does
/// (<https://helpx.adobe.com/illustrator/using/tool-techniques/rectangular-grid-tool.html>,
/// <https://helpx.adobe.com/illustrator/using/tool-techniques/arc-tool.html>): the click dialog's
/// fields at the tool's options, with no click to draw at.
pub(crate) fn open_options(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    if !OPTION_TOOLS.contains(&tool) {
        return Err(format!("the {tool} tool has no options"));
    }
    super::open_tool_dialog(app, tool, json!({}));
    let d = app.ui.dialog.as_mut().filter(|d| d.kind == tool).ok_or_else(|| format!("the {tool} tool has no options"))?;
    for key in ["x", "y", "__anchorX", "__anchorY"] {
        d.fields.remove(key);
    }
    d.fields.insert(TOOL_OPTIONS.into(), json!(true));
    Ok(json!({ "dialog": tool }))
}

/// What OK sets as the options of `d`'s tool ([`OPTION_TOOLS`]), from its fields: a grid's are its
/// switches as they are, its typed fields as numbers, its Default Size and Reference Point.
fn tool_options(d: &Dialog) -> Value {
    match d.kind.as_str() {
        "lineSegment" => {
            let (length, angle) = line_length_angle(d);
            json!({"length": length, "angle": angle, "fillLine": d.bool("fillLine")})
        }
        "arc" => arc_options(d),
        kind => {
            let (mut o, (counts, skews)) = (form::params(d), grid_keys(kind));
            if let Some(m) = o.as_object_mut() {
                m.remove("x");
                m.remove("y");
            }
            for (count, skew) in counts.into_iter().zip(skews) {
                o[count] = json!((d.f64(count, 5.0) as u64).min(999));
                o[skew] = json!(d.f64(skew, 0.0));
            }
            o["width"] = json!(d.f64("width", 100.0));
            o["height"] = json!(d.f64("height", 100.0));
            o["reference"] = json!(grid_reference(d));
            o
        }
    }
}

/// The Line Segment dialog's length and angle (degrees), a negative length turned into the same
/// line drawn the other way: -80 pt at 0° is 80 pt at 180°. The line a click draws and the options
/// OK keeps both use it, so the next line goes the way this one went.
fn line_length_angle(d: &Dialog) -> (f64, f64) {
    let (length, angle) = (d.f64("length", 100.0), d.f64("angle", 0.0));
    if length < 0.0 { (-length, (angle + 180.0).rem_euclid(360.0)) } else { (length, angle) }
}

/// The Arc tool's options from its dialog's fields.
fn arc_options(d: &Dialog) -> Value {
    json!({
        "closed": d.str("arcType").eq_ignore_ascii_case("closed"),
        "lengthX": d.f64("lengthX", 100.0),
        "lengthY": d.f64("lengthY", 100.0),
        "reference": arc_reference(d),
        "baseAlong": d.str("baseAlong"),
        "slope": d.f64("slope", 50.0),
        "fillArc": d.bool("fillArc"),
    })
}

/// The grids' divider counts and skews: (rows / concentric, columns / radial).
fn grid_keys(kind: &str) -> ([&'static str; 2], [&'static str; 2]) {
    if kind == "polarGrid" { (["concentric", "radial"], ["concentricSkew", "radialSkew"]) } else { (["rows", "columns"], ["rowSkew", "columnSkew"]) }
}

/// The 9-point locator index for a grid dialog (`reference`, default top-left).
fn grid_reference(d: &Dialog) -> usize {
    d.fields.get("reference").and_then(|v| v.as_u64()).map(|n| (n as usize).min(8)).unwrap_or(0)
}

/// A grid dialog's bounding box from its top-left `x`/`y` and size fields.
fn grid_placed(d: &Dialog) -> Rect {
    let (x, y) = (d.f64("x", 0.0), d.f64("y", 0.0));
    Rect::new(x, y, x + d.f64("width", 100.0), y + d.f64("height", 100.0))
}

fn set_grid_placed(d: &mut Dialog, r: Rect) {
    d.fields.insert("x".into(), json!(r.x0));
    d.fields.insert("y".into(), json!(r.y0));
    d.fields.insert("width".into(), json!(r.width()));
    d.fields.insert("height".into(), json!(r.height()));
}

/// Arc segment dialog: keep reference `refi` on the canvas click (`__anchorX`/`__anchorY`).
pub(crate) fn arc_pin_to_anchor(d: &mut Dialog) {
    let anchor = Point::new(d.f64("__anchorX", d.f64("x", 0.0)), d.f64("__anchorY", d.f64("y", 0.0)));
    let refi = d.fields.get("reference").and_then(|v| v.as_u64()).map(|n| (n as usize).min(8)).unwrap_or(0);
    let w = d.f64("lengthX", 100.0).abs().max(1e-9);
    let h = d.f64("lengthY", 100.0).abs().max(1e-9);
    let rp = vectorcraft_geom::reference_point(Rect::new(0.0, 0.0, w, h), refi);
    d.fields.insert("x".into(), json!(anchor.x - rp.x));
    d.fields.insert("y".into(), json!(anchor.y - rp.y));
}

fn arc_reference(d: &Dialog) -> usize {
    d.fields.get("reference").and_then(|v| v.as_u64()).map(|n| (n as usize).min(8)).unwrap_or(0)
}

/// Top-left `x`/`y` so reference point `refi` sits on the canvas click (`__anchorX`/`__anchorY`).
pub(crate) fn grid_pin_to_anchor(d: &mut Dialog) {
    let anchor = Point::new(d.f64("__anchorX", d.f64("x", 0.0)), d.f64("__anchorY", d.f64("y", 0.0)));
    let refi = grid_reference(d);
    let (w, h) = (d.f64("width", 100.0), d.f64("height", 100.0));
    let rp = vectorcraft_geom::reference_point(Rect::new(0.0, 0.0, w, h), refi);
    d.fields.insert("x".into(), json!(anchor.x - rp.x));
    d.fields.insert("y".into(), json!(anchor.y - rp.y));
}

/// The form's fields, then a grid's switches.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    if matches!(d.kind.as_str(), "rectangularGrid" | "polarGrid") {
        ui.horizontal(|ui| {
            let refi = grid_reference(d);
            if let Some(i) = widgets::reference_point(ui, refi) {
                d.fields.insert("reference".into(), json!(i));
                grid_pin_to_anchor(d);
            }
        });
        ui.add_space(4.0);
        let refi = grid_reference(d);
        let (was, w0, h0) = (grid_placed(d), d.f64("width", 100.0), d.f64("height", 100.0));
        form::grid_then(ui, d, unit, |_, d| {
            let (w, h) = (d.f64("width", 100.0), d.f64("height", 100.0));
            if (w - w0).abs() > 1e-9 || (h - h0).abs() > 1e-9 {
                set_grid_placed(d, artboard_rect(was, refi, None, None, Some(w), Some(h), false));
            }
        });
    } else if d.kind == "arc" {
        arc_form(ui, d, unit);
        if d.bool(TOOL_OPTIONS) {
            ui.add_space(8.0);
            arc_preview(ui, d, &app.session);
        }
    } else if LINKED.contains(&d.kind.as_str()) {
        linked_size_form(ui, d, unit);
    } else {
        form::grid(ui, d, unit);
    }
    if d.kind == "spiral" {
        ui.add_space(8.0);
        form::check(ui, d, "clockwise", tl!("Clockwise"));
        return false;
    }
    let own = match d.kind.as_str() {
        "rectangularGrid" => ("frame", tl!("Use Outside Rectangle As Frame")),
        "polarGrid" => ("compound", tl!("Create Compound Path From Ellipses")),
        "lineSegment" => {
            ui.add_space(8.0);
            form::check(ui, d, "fillLine", tl!("Fill Line"));
            return false;
        }
        _ => return false,
    };
    ui.add_space(8.0);
    for (key, label) in [own, ("fill", tl!("Fill Grid"))] {
        form::check(ui, d, key, label);
    }
    false
}

fn arc_form(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    ui.horizontal(|ui| {
        let refi = arc_reference(d);
        if let Some(i) = widgets::reference_point(ui, refi) {
            d.fields.insert("reference".into(), json!(i));
            arc_pin_to_anchor(d);
        }
    });
    ui.add_space(4.0);
    let rail = crate::theme::Tokens::get(ui.ctx()).input_border;
    egui::Grid::new("arc-dlg").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        let lx0 = d.f64("lengthX", 100.0);
        form::length_field(ui, d, "lengthX", tl!("Length X-Axis:"), unit);
        if d.f64("lengthX", 100.0) != lx0 {
            arc_pin_to_anchor(d);
        }
        let ly0 = d.f64("lengthY", 100.0);
        form::length_field(ui, d, "lengthY", tl!("Length Y-Axis:"), unit);
        if d.f64("lengthY", 100.0) != ly0 {
            arc_pin_to_anchor(d);
        }
        form::choice(
            ui,
            d,
            "arcType",
            tl!("Type:"),
            (form::FIELD_W, form::FIELD_W - 12.0),
            &[
                ("open", crate::i18n::tr_ctx(crate::i18n::current(), "shape", "Open")),
                ("closed", crate::i18n::tr_ctx(crate::i18n::current(), "shape", "Closed")),
            ],
        );
        form::choice(
            ui,
            d,
            "baseAlong",
            tl!("Base Along:"),
            (form::FIELD_W, form::FIELD_W - 12.0),
            &[
                ("x", crate::i18n::tr_ctx(crate::i18n::current(), "axis", "X Axis")),
                ("y", crate::i18n::tr_ctx(crate::i18n::current(), "axis", "Y Axis")),
            ],
        );
        form::row_label(ui, tl!("Slope:"));
        ui.vertical(|ui| {
            form::slider_field_named(ui, d, "slope", tl!("Slope:"), -100.0..=100.0, "", &|_| rail);
            form::slider_ends(ui, form::SLIDER_LABEL, (tl!("Concave"), tl!("Convex")));
        });
        ui.end_row();
    });
    ui.add_space(8.0);
    form::check(ui, d, "fillArc", tl!("Fill Arc"));
}

/// The size of the arc picture in Arc Segment Tool Options, and its margin around the arc.
const ARC_PREVIEW: egui::Vec2 = egui::vec2(160.0, 120.0);
const ARC_PREVIEW_MARGIN: f32 = 12.0;

/// The arc the Arc tool draws with the options OK sets from `d` ([`arc_options`], kept as the tool
/// keeps them: a slope from -100 to 100, a Base Along of Y or else X), as `shape.arc` draws it
/// with its reference point at the origin and General › Constrain Angle `constrain` (degrees)
/// turning it about that point. `None` when a length or the slope is not finite.
fn arc_preview_path(d: &Dialog, constrain: f64) -> Option<vectorcraft_geom::path::PathData> {
    let o = arc_options(d);
    let num = |key: &str| o[key].as_f64().filter(|v| v.is_finite());
    let (lx, ly, slope) = (num("lengthX")?, num("lengthY")?, num("slope")?.clamp(-100.0, 100.0));
    let base_x = !o["baseAlong"].as_str().is_some_and(|b| b.eq_ignore_ascii_case("y"));
    let closed = o["closed"].as_bool().unwrap_or(false);
    let path = vectorcraft_geom::shapes::arc_segment(Point::ZERO, arc_reference(d), lx, ly, base_x, slope, closed);
    Some(match vectorcraft_geom::constrain_rotation(constrain) {
        Some(rotation) => path.transformed(vectorcraft_geom::rotate_deg(rotation)),
        None => path,
    })
}

/// The arc in the dialog's picture, in screen points: its outline's points, whether it is closed,
/// and where its reference point sits.
struct ArcPreview {
    outline: Vec<egui::Pos2>,
    closed: bool,
    reference: egui::Pos2,
}

/// The arc of [`arc_preview_path`] fitted into `rect` with [`ARC_PREVIEW_MARGIN`] around it,
/// keeping its proportions. `None` when the fields describe no arc or the fitted points are not
/// finite.
fn arc_preview_shape(d: &Dialog, constrain: f64, rect: egui::Rect) -> Option<ArcPreview> {
    let path = arc_preview_path(d, constrain)?;
    let b = path.bounds()?.union_pt(Point::ZERO);
    let (room_w, room_h) = (f64::from(rect.width() - 2.0 * ARC_PREVIEW_MARGIN), f64::from(rect.height() - 2.0 * ARC_PREVIEW_MARGIN));
    // A side far thinner than the other (a length of 0) does not limit the scale.
    let extent = b.width().max(b.height());
    let fit = |side: f64, room: f64| if side > extent * 1e-6 { room / side } else { f64::INFINITY };
    let scale = fit(b.width(), room_w).min(fit(b.height(), room_h));
    if !(scale.is_finite() && scale > 0.0 && room_w > 0.0 && room_h > 0.0) {
        return None;
    }
    let c = rect.center();
    let to_screen = vectorcraft_geom::Affine::translate((f64::from(c.x), f64::from(c.y)))
        * vectorcraft_geom::Affine::scale(scale)
        * vectorcraft_geom::Affine::translate(-b.center().to_vec2());
    let pos = |p: Point| egui::pos2(p.x as f32, p.y as f32);
    let mut outline = Vec::new();
    vectorcraft_geom::flatten(path.transformed(to_screen).to_bezpath(), 0.25, |el| match el {
        vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) => outline.push(pos(p)),
        _ => {}
    });
    // A closed arc's last segment ends where it starts; the outline keeps its corner as the last point.
    let closed = path.is_closed();
    if closed && outline.len() > 2 && outline.first() == outline.last() {
        outline.pop();
    }
    let reference = pos(to_screen * Point::ZERO);
    let finite = |p: &egui::Pos2| p.x.is_finite() && p.y.is_finite();
    (outline.len() >= 2 && outline.iter().all(finite) && finite(&reference)).then_some(ArcPreview { outline, closed, reference })
}

/// A paint as one color for the arc picture: a solid color as it is, a gradient or pattern gray.
fn arc_preview_color(paint: &vectorcraft_color::Paint) -> Option<egui::Color32> {
    match paint {
        vectorcraft_color::Paint::None => None,
        vectorcraft_color::Paint::Solid { color, tint, .. } => Some(crate::panels::c32(&color.tinted(*tint))),
        _ => Some(egui::Color32::from_gray(160)),
    }
}

/// The arc Arc Segment Tool Options sets, drawn below its fields on white and redrawn on every
/// frame from the fields as they are then: in the current stroke (black when there is none, as
/// `shape.arc` draws it), filled with the current fill when Fill Arc is on, turned by General ›
/// Constrain Angle, with a mark on its reference point.
fn arc_preview(ui: &mut egui::Ui, d: &Dialog, session: &vectorcraft_engine::Session) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(ARC_PREVIEW, egui::Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tl!("Preview")));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    let Some(arc) = arc_preview_shape(d, session.prefs.constrain_angle, rect) else { return };
    let fill = if d.bool("fillArc") { arc_preview_color(&session.paint.fill) } else { None };
    if let Some(fill) = fill {
        // A closed arc is seen whole from its corner (its last point), and an open arc closed by
        // its chord from its end: each fills as a fan of triangles from there.
        let mut mesh = egui::Mesh::default();
        for p in &arc.outline {
            mesh.colored_vertex(*p, fill);
        }
        let last = arc.outline.len() as u32 - 1;
        for i in 0..last.saturating_sub(1) {
            mesh.add_triangle(last, i, i + 1);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    let stroke = arc_preview_color(&session.paint.stroke).unwrap_or(egui::Color32::BLACK);
    let stroke = egui::Stroke::new(1.5, stroke);
    painter.add(if arc.closed { egui::Shape::closed_line(arc.outline, stroke) } else { egui::Shape::line(arc.outline, stroke) });
    painter.circle(arc.reference, 3.0, egui::Color32::WHITE, egui::Stroke::new(1.5, t.accent));
}

/// The click dialogs whose Width and Height share a Constrain Width and Height Proportions link
/// (<https://krankykids.com/cheatsheets/illustrator_cs6/shape_tool_rectangle.html>,
/// <https://www.krankykids.com/cheatsheets/illustrator_cs6/shape_tool_ellipse.html>): the same
/// link as the Transform panel's ([`crate::panels::transform::constrain_link`]), drawn the same way.
const LINKED: [&str; 3] = ["rectangle", "roundedRectangle", "ellipse"];

/// The dialog field holding the link's state (UI-only, as every `__` field is).
const LINK: &str = "__constrain";

/// The dialog field holding the link state the dialog last applied ([`link_changed`]): a change of
/// [`LINK`], by its button or from outside (`ui.dialog.set`), shows as a difference from it.
const APPLIED: &str = "__constrainApplied";

/// The dialog field holding height ÷ width while the link is on: the proportions it keeps.
const RATIO: &str = "__ratio";

/// Where the link's state waits for the next of these dialogs (egui memory, this run only).
const LINK_STATE: &str = "shape-size-link";

/// Width and Height (then a rounded rectangle's radius) with the Constrain Width and Height
/// Proportions link beside the two size rows. While it is on, typing one side sets the other, so
/// the shape keeps the proportions it had when the link was turned on. The next of these dialogs
/// opens with the link as this one left it.
fn linked_size_form(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    if !d.fields.contains_key(LINK) {
        d.fields.insert(LINK.into(), json!(crate::panels::pstate::<bool>(ui.ctx(), LINK_STATE)));
    }
    sync_link(ui.ctx(), d);
    const BUTTON: f32 = 22.0;
    let mut rows: Option<egui::Rect> = None;
    egui::Grid::new("shape-size-dlg").num_columns(3).spacing([10.0, 8.0]).show(ui, |ui| {
        for (key, label) in [("width", "Width:"), ("height", "Height:")] {
            let row = if key == "height" && d.kind == "ellipse" {
                let (row, pressed) = height_label_button(ui, label);
                if pressed {
                    copy_width_to_height(d);
                }
                row
            } else {
                form::row_label(ui, label)
            };
            if ui.label_inputs_by(row.id, |ui| form::length(ui, d, key, unit, form::FIELD_W)) {
                follow_link(d, key);
            }
            // The link's column: the button is drawn over both rows' cells once the grid is laid out.
            let (_, cell) = ui.allocate_space(egui::vec2(BUTTON, BUTTON));
            rows = Some(rows.map_or(cell, |r| r.union(cell)));
            ui.end_row();
        }
        if d.kind == "roundedRectangle" {
            form::length_field(ui, d, "radius", &form::humanized("radius"), unit);
        }
        if d.kind == "ellipse" {
            form::field(ui, d, "anchors", "Anchors:");
        }
    });
    let Some(rows) = rows else { return };
    let on = d.bool(LINK);
    let rect = egui::Rect::from_center_size(rows.center(), egui::vec2(BUTTON, BUTTON));
    let resp = ui.interact(rect, ui.scope_id().with("shape-size-link"), egui::Sense::click());
    let icon = if on { "link" } else { "link-2-off" };
    if widgets::paint_icon_button(ui, resp, icon, "Constrain Width and Height Proportions", on).clicked() {
        // Enter on the focused button turns the link on or off; it does not also press OK.
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        d.fields.insert(LINK.into(), json!(!on));
        sync_link(ui.ctx(), d);
    }
}

/// Apply a change of the link's state, however it was made ([`link_changed`]), and keep the new
/// state for the next of these dialogs.
fn sync_link(ctx: &egui::Context, d: &mut Dialog) {
    if let Some(on) = link_changed(d) {
        crate::panels::set_pstate(ctx, LINK_STATE, on);
    }
}

/// When the link's state differs from the one the dialog last applied (or none was applied yet),
/// apply it and return it: turned on, the link keeps the proportions of the sizes the dialog shows
/// then; turned off, it forgets them. A side of zero when it turns on leaves nothing to keep, so
/// typing changes only the side typed until the link is turned off and on again.
fn link_changed(d: &mut Dialog) -> Option<bool> {
    let on = d.bool(LINK);
    if d.fields.get(APPLIED).and_then(Value::as_bool) == Some(on) {
        return None;
    }
    d.fields.insert(APPLIED.into(), json!(on));
    let ratio = d.f64("height", 100.0) / d.f64("width", 100.0);
    if on && ratio.is_finite() && ratio != 0.0 {
        d.fields.insert(RATIO.into(), json!(ratio));
    } else {
        d.fields.remove(RATIO);
    }
    Some(on)
}

/// After the side `typed` ("width" or "height") was typed with the link on, set the other side
/// from the proportions the link keeps.
fn follow_link(d: &mut Dialog, typed: &str) {
    let ratio = d.f64(RATIO, 0.0);
    if !d.bool(LINK) || !ratio.is_finite() || ratio == 0.0 {
        return;
    }
    let (other, v) = if typed == "width" { ("height", d.f64("width", 100.0) * ratio) } else { ("width", d.f64("height", 100.0) / ratio) };
    if v.is_finite() {
        d.fields.insert(other.into(), json!(v));
    }
}

/// The Ellipse dialog's Height label: a button drawn as the other rows' labels are. Whether it was
/// pressed: a click, or Enter or Space while it has the keyboard focus; that Enter does not also
/// press OK. A click whose press took the focus from a field that then refused its text (Width
/// left empty or not a distance) does not count, as such a click on OK does not. Screen readers
/// read it as the button "Height:" (which also names the Height field) with the description "Copy
/// Width to Height".
fn height_label_button(ui: &mut egui::Ui, label: &str) -> (egui::Response, bool) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let text = egui::RichText::new(tl!(label)).color(t.text_dim);
    let resp = ui.add(egui::Button::new(text).frame(false)).on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = resp.on_hover_text(tl!("Copy Width to Height"));
    let press = resp.id.with("press-pass");
    if resp.contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
        let pass = ui.ctx().cumulative_pass_nr();
        ui.data_mut(|m| m.insert_temp(press, pass));
    }
    if !resp.clicked() {
        // A press let go without a click (dragged off the button) leaves no press behind.
        if ui.input(|i| i.pointer.any_released()) {
            ui.data_mut(|m| m.remove::<u64>(press));
        }
        return (resp, false);
    }
    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let (pressed_at, refused_at, refused_now) = ui.data_mut(|m| {
        let pressed_at = m.get_temp::<u64>(press);
        m.remove::<u64>(press);
        (pressed_at, m.get_temp::<u64>(widgets::dialog_reject_pass()), m.get_temp::<bool>(widgets::dialog_reject_commit_flag()))
    });
    let refused = refused_now == Some(true) || matches!((refused_at, pressed_at), (Some(r), Some(p)) if r >= p);
    (resp, !refused)
}

/// Height takes the Width the dialog holds (help page "Draw ellipses",
/// <https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf#page=140>).
/// A Width that is missing, empty, not a distance or not finite leaves Height as it is. With the
/// link on, the link keeps the circle from then on (proportions 1 : 1), or keeps no proportions
/// when Width is 0; the link stays on.
fn copy_width_to_height(d: &mut Dialog) {
    let width = match d.fields.get("width") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => Unit::Points.parse(s),
        _ => None,
    };
    let Some(width) = width.filter(|w| w.is_finite()) else { return };
    d.fields.insert("height".into(), json!(width));
    if d.bool(LINK) {
        if width != 0.0 {
            d.fields.insert(RATIO.into(), json!(1.0));
        } else {
            d.fields.remove(RATIO);
        }
    }
}

/// The Ellipse dialog's Anchors: the nearest whole number from 2 to 1000 (4, a live ellipse, when
/// left empty or not a number).
fn ellipse_anchors(d: &Dialog) -> u64 {
    let n = d.f64("anchors", 4.0);
    if n.is_finite() { n.round().clamp(2.0, 1000.0) as u64 } else { 4 }
}

/// The top-left corner of a rectangle or ellipse: the click itself, or with Alt held (draw from centre) the click minus half the size.
fn box_origin(d: &Dialog) -> (f64, f64) {
    let (x, y) = (d.f64("x", 0.0), d.f64("y", 0.0));
    if d.fields.get("alt").and_then(Value::as_bool).unwrap_or(false) {
        (x - d.f64("width", 100.0) / 2.0, y - d.f64("height", 100.0) / 2.0)
    } else {
        (x, y)
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    // Tool Options: the options the tool's next drag and click use; nothing is drawn.
    if d.bool(TOOL_OPTIONS) {
        let r = app.run("tool.setOption", json!({"tool": d.kind, "values": tool_options(d)}));
        app.ui.dialog = None;
        return r;
    }
    let count = match d.kind.as_str() {
        "polygon" => Some(("sides", 6.0, 3.0)),
        "star" => Some(("points", 5.0, 2.0)),
        _ => None,
    };
    let options = count
        .map(|(key, default, min)| {
            let mut values = json!({});
            values[key] = json!(d.f64(key, default).clamp(min, 1000.0) as u64);
            values
        })
        .or_else(|| (d.kind == "ellipse").then(|| json!({"anchors": ellipse_anchors(d)})))
        .or_else(|| OPTION_TOOLS.contains(&d.kind.as_str()).then(|| tool_options(d)));
    let corner_radius = if d.kind == "roundedRectangle" { Some(rounded_radius(app, d)?) } else { None };
    let (x, y) = (d.f64("x", 0.0), d.f64("y", 0.0));
    let (bx, by) = box_origin(d);
    let (id, params) = match d.kind.as_str() {
        "ellipse" => (
            "shape.ellipse",
            json!({"x": bx, "y": by, "width": d.f64("width", 100.0), "height": d.f64("height", 100.0), "anchors": ellipse_anchors(d)}),
        ),
        "polygon" => ("shape.polygon", json!({"cx": x, "cy": y, "radius": d.f64("radius", 50.0), "sides": d.f64("sides", 6.0) as u64})),
        "star" => (
            "shape.star",
            json!({"cx": x, "cy": y, "radius1": d.f64("radius1", 50.0), "radius2": d.f64("radius2", 25.0), "points": d.f64("points", 5.0) as u64}),
        ),
        "spiral" => (
            "shape.spiral",
            json!({
                "cx": x,
                "cy": y,
                "radius": d.f64("radius", 50.0).max(0.01),
                "decay": d.f64("decay", 80.0).clamp(5.0, 150.0),
                "segments": (d.f64("segments", 10.0) as u64).clamp(2, 1000),
                "clockwise": d.bool("clockwise"),
            }),
        ),
        kind @ ("rectangularGrid" | "polarGrid") => {
            // The switches as they are; the typed fields as numbers.
            let (mut p, (counts, skews)) = (form::params(d), grid_keys(kind));
            let r = grid_placed(d);
            for (k, v) in [("x", r.x0), ("y", r.y0), ("width", r.width()), ("height", r.height())] {
                p[k] = json!(v);
            }
            for (count, skew) in counts.into_iter().zip(skews) {
                p[count] = json!((d.f64(count, 5.0) as u64).min(999));
                p[skew] = json!(d.f64(skew, 0.0));
            }
            (if kind == "polarGrid" { "shape.polarGrid" } else { "shape.rectangularGrid" }, p)
        }
        "lineSegment" => {
            let (l, a) = line_length_angle(d);
            let a = a.to_radians();
            let mut p = json!({"x1": x, "y1": y, "x2": x + l * a.cos(), "y2": y - l * a.sin()});
            if d.bool("fillLine") {
                p["fill"] = json!(true);
            }
            ("shape.line", p)
        }
        "arc" => {
            let anchor = Point::new(d.f64("__anchorX", x), d.f64("__anchorY", y));
            (
                "shape.arc",
                json!({
                    "x": anchor.x,
                    "y": anchor.y,
                    "lengthX": d.f64("lengthX", 100.0),
                    "lengthY": d.f64("lengthY", 100.0),
                    "reference": arc_reference(d),
                    "baseAlong": d.str("baseAlong"),
                    "slope": d.f64("slope", 50.0),
                    "closed": d.str("arcType").eq_ignore_ascii_case("closed"),
                    "fillArc": d.bool("fillArc"),
                }),
            )
        }
        _ => (
            "shape.rectangle",
            json!({
                "x": bx,
                "y": by,
                "width": d.f64("width", 100.0),
                "height": d.f64("height", 100.0),
                "radius": corner_radius.unwrap_or_else(|| d.f64("radius", 0.0)),
            }),
        ),
    };
    // On the active plane while the perspective grid shows, sized in plane units from the click;
    // else along General › Constrain Angle, turned about the point the dialog places at the click
    // (a grid's reference point, the other shapes' top-left corner or centre).
    let options = options.or_else(|| (d.kind == "spiral").then(|| params.clone()));
    let result = match vectorcraft_engine::perspective_click(&app.session, id, &params, vectorcraft_geom::Point::new(x, y)) {
        Some((id, params)) => run_and_close(app, &id, params),
        None => {
            let pivot = if matches!(d.kind.as_str(), "rectangularGrid" | "polarGrid") {
                vectorcraft_geom::reference_point(grid_placed(d), grid_reference(d))
            } else {
                vectorcraft_geom::Point::new(x, y)
            };
            let params = vectorcraft_engine::constrain_click(&app.session, id, params, pivot);
            run_and_close(app, id, params)
        }
    }?;
    if let Some(options) = options {
        app.run("tool.setOption", json!({"tool": d.kind, "values": options}))?;
    }
    // The radius of the rounded rectangle drawn is General › Corner Radius from then on.
    if let Some(r) = corner_radius.filter(|r| *r != app.session.prefs.corner_radius) {
        app.run("prefs.set", json!({"key": "cornerRadius", "value": r}))?;
    }
    Ok(result)
}

/// The Rounded Rectangle dialog's radius in points, checked as General › Corner Radius checks it
/// (an error leaves the dialog open and draws nothing). Without the field, the preference.
fn rounded_radius(app: &VectorcraftApp, d: &Dialog) -> Result<f64, String> {
    let r = if d.fields.contains_key("radius") { d.f64("radius", f64::NAN) } else { app.session.prefs.corner_radius };
    vectorcraft_engine::cmd::prefscmds::validate("cornerRadius", &json!(r))?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn spiral_click_dialog_uses_and_updates_tool_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("tool.select", json!({"tool": "spiral"})).unwrap();
        app.run("tool.setOption", json!({"tool": "spiral", "values": {"decay": 72.0, "segments": 14, "clockwise": false}})).unwrap();

        super::super::open_tool_dialog(&mut app, "spiral", json!({"x": 40.0, "y": 60.0}));
        let d = app.ui.dialog.as_ref().expect("Spiral dialog");
        assert_eq!(d.kind, "spiral");
        assert_eq!((d.f64("x", 0.0), d.f64("y", 0.0), d.f64("radius", 0.0)), (40.0, 60.0, 50.0));
        assert_eq!((d.f64("decay", 0.0), d.f64("segments", 0.0), d.bool("clockwise")), (72.0, 14.0, false));

        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("radius".into(), json!(35.0));
        d.fields.insert("decay".into(), json!(65.0));
        d.fields.insert("segments".into(), json!(12));
        d.fields.insert("clockwise".into(), json!(true));
        super::super::confirm(&mut app).unwrap();

        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 1);
        assert_eq!(app.session.tool_options_of("spiral"), json!({"decay": 65.0, "segments": 12, "clockwise": true}));
    }

    /// With General › Constrain Angle at 90°, the Spiral dialog's spiral starts straight up from the
    /// click, and the tool's options keep no turn.
    #[test]
    fn spiral_click_dialog_starts_along_the_constrain_angle() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("prefs.set", json!({"key": "constrainAngle", "value": 90})).unwrap();
        app.run("tool.select", json!({"tool": "spiral"})).unwrap();
        super::super::open_tool_dialog(&mut app, "spiral", json!({"x": 40.0, "y": 60.0}));
        app.ui.dialog.as_mut().expect("Spiral dialog").fields.insert("radius".into(), json!(35.0));
        super::super::confirm(&mut app).unwrap();

        let st = app.session.doc().unwrap();
        let id = *st.selection.objects.first().unwrap();
        let path = st.doc.node(id).unwrap().path_data().unwrap();
        let outer = path.subpaths[0].anchors.last().unwrap().p;
        assert_eq!(outer, vectorcraft_geom::Point::new(40.0, 25.0));
        assert!(app.session.tool_options_of("spiral").get("rotation").is_none());
    }

    #[test]
    fn alt_click_centres_the_typed_size_on_the_click() {
        let fields = |alt: bool| Dialog::new("rectangle", json!({"x": 100.0, "y": 50.0, "alt": alt, "width": "40 pt", "height": "20 pt"}));
        assert_eq!(box_origin(&fields(false)), (100.0, 50.0));
        assert_eq!(box_origin(&fields(true)), (80.0, 40.0));
    }

    fn arc_dialog(fields: Value) -> Dialog {
        let mut d =
            Dialog::new("arc", json!({"lengthX": 100.0, "lengthY": 60.0, "reference": 0, "baseAlong": "x", "slope": 50.0, "arcType": "open"}));
        for (k, v) in fields.as_object().into_iter().flatten() {
            d.fields.insert(k.clone(), v.clone());
        }
        d.fields.insert(TOOL_OPTIONS.into(), json!(true));
        d
    }

    fn anchor_points(p: &vectorcraft_geom::path::PathData) -> Vec<(f64, f64, f64, f64, f64, f64)> {
        p.anchors().map(|(_, _, a)| (a.p.x, a.p.y, a.h_in.x, a.h_in.y, a.h_out.x, a.h_out.y)).collect()
    }

    /// The preview is the arc `shape.arc` draws from the same values at a click on the origin, for
    /// each Type, Base Along, reference point and slope, and turned by General › Constrain Angle.
    #[test]
    fn the_arc_preview_is_the_arc_shape_arc_draws() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        for constrain in [0.0, 30.0] {
            app.run("prefs.set", json!({"key": "constrainAngle", "value": constrain})).unwrap();
            for (closed, base, reference, slope) in
                [(false, "x", 0, 50.0), (true, "x", 4, 100.0), (false, "y", 8, -100.0), (true, "y", 2, 0.0), (false, "x", 6, -40.0)]
            {
                let d =
                    arc_dialog(json!({"arcType": if closed { "closed" } else { "open" }, "baseAlong": base, "reference": reference, "slope": slope}));
                let preview = arc_preview_path(&d, app.session.prefs.constrain_angle).expect("an arc");
                let params = json!({"x": 0.0, "y": 0.0, "lengthX": 100.0, "lengthY": 60.0, "reference": reference, "baseAlong": base, "slope": slope, "closed": closed});
                let params = vectorcraft_engine::constrain_click(&app.session, "shape.arc", params, Point::ZERO);
                app.run("shape.arc", params).unwrap();
                let doc = &app.session.doc().unwrap().doc;
                let drawn = doc.layers[0].children().unwrap().last().unwrap().path_data().unwrap().clone();
                let (a, b) = (anchor_points(&preview), anchor_points(&drawn));
                assert_eq!(a.len(), b.len(), "{constrain} {closed} {base} {reference} {slope}");
                for (p, q) in a.iter().zip(&b) {
                    let close = [(p.0, q.0), (p.1, q.1), (p.2, q.2), (p.3, q.3), (p.4, q.4), (p.5, q.5)].iter().all(|(u, v)| (u - v).abs() < 1e-6);
                    assert!(close, "{constrain} {closed} {base} {reference} {slope}: {a:?} {b:?}");
                }
                assert_eq!(preview.is_closed(), closed);
            }
        }
    }

    /// The picture follows Slope, Type, Base Along and the reference point, keeps the arc inside
    /// the frame, and stays finite for a length of 0 or a huge one.
    #[test]
    fn the_arc_preview_follows_the_fields_and_fits_its_frame() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), ARC_PREVIEW);
        let shape = |fields: Value| arc_preview_shape(&arc_dialog(fields), 0.0, rect).expect("an arc");
        let inside = |s: &ArcPreview| {
            let room = rect.shrink(ARC_PREVIEW_MARGIN - 0.01);
            s.outline.iter().chain([&s.reference]).all(|p| room.contains(*p))
        };
        let base = shape(json!({}));
        assert!(inside(&base), "{:?}", base.outline);
        // Slope 0 is a straight line: two points.
        assert_eq!(shape(json!({"slope": 0.0})).outline.len(), 2);
        for changed in [json!({"slope": -50.0}), json!({"arcType": "closed"}), json!({"baseAlong": "y"}), json!({"lengthY": "30 pt"})] {
            let s = shape(changed.clone());
            assert!(inside(&s), "{changed}");
            assert_ne!(s.outline, base.outline, "{changed}");
        }
        assert!(shape(json!({"arcType": "closed"})).closed && !base.closed);
        // The reference mark sits on each of the nine points of the arc's box.
        let marks: Vec<egui::Pos2> = (0..9).map(|i| shape(json!({"reference": i})).reference).collect();
        assert!((marks[4] - rect.center()).length() < 1e-3, "{marks:?}");
        for row in 0..3 {
            assert!(marks[row * 3].x < marks[row * 3 + 1].x && marks[row * 3 + 1].x < marks[row * 3 + 2].x, "{marks:?}");
            assert!(marks[row].y < marks[row + 3].y && marks[row + 3].y < marks[row + 6].y, "{marks:?}");
        }
        for lengths in [json!({"lengthX": 0.0}), json!({"lengthY": 0.0}), json!({"lengthX": 1e300, "lengthY": 1e-300}), json!({"lengthX": -80.0})] {
            assert!(inside(&shape(lengths.clone())), "{lengths}");
        }
    }

    /// For values the Arc tool does not keep as typed, the picture shows the arc of the options OK
    /// sets: the same picture as a dialog holding those options.
    #[test]
    fn the_arc_preview_shows_the_options_ok_sets() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        for typed in [
            json!({"slope": 140.0}),
            json!({"slope": -250.0}),
            json!({"slope": "steep"}),
            json!({"lengthX": "wide"}),
            json!({"lengthY": null}),
            json!({"baseAlong": "z"}),
            json!({"reference": 12}),
        ] {
            let d = arc_dialog(typed.clone());
            let shown = arc_preview_path(&d, 0.0).expect("an arc");
            app.ui.dialog = Some(d);
            super::super::confirm(&mut app).unwrap();
            let kept = app.session.tool_options_of("arc");
            let arc_type = if kept["closed"].as_bool() == Some(true) { "closed" } else { "open" };
            let keys = ["lengthX", "lengthY", "slope", "baseAlong", "reference"];
            let mut fields: serde_json::Map<String, Value> = keys.iter().map(|k| (k.to_string(), kept[*k].clone())).collect();
            fields.insert("arcType".into(), json!(arc_type));
            let want = arc_preview_path(&arc_dialog(Value::Object(fields)), 0.0).expect("an arc");
            assert_eq!(anchor_points(&shown), anchor_points(&want), "{typed}: kept {kept}");
        }
    }

    /// Double-clicking the Arc tool shows the picture in Arc Segment Tool Options; a click on the
    /// canvas does not. Changing the options and pressing OK draws nothing and adds no undo step.
    #[test]
    fn arc_tool_options_show_a_preview_and_ok_draws_nothing() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("tool.select", json!({"tool": "arc"})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        open_options(&mut app, "arc").unwrap();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(5);
        h.check_accessibility();
        let picture = h.get_by_role_and_label(Role::Image, "Preview").rect();
        let fill_arc = h.get_by_label("Fill Arc").rect();
        assert!(picture.top() > fill_arc.center().y, "the picture is below Fill Arc: {picture:?} {fill_arc:?}");
        h.state_mut().ui.dialog.as_mut().unwrap().fields.insert("slope".into(), json!(-30.0));
        h.run_steps(2);
        super::super::confirm(h.state_mut()).unwrap();
        h.run_steps(2);
        let app = h.state();
        assert!(app.ui.dialog.is_none());
        assert!(app.session.doc().unwrap().doc.layers[0].children().unwrap().is_empty());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        assert_eq!(app.session.tool_options_of("arc")["slope"], json!(-30.0));
        drop(h);

        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        super::super::open_tool_dialog(&mut app, "arc", json!({"x": 40.0, "y": 60.0}));
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(5);
        assert!(h.query_by_role_and_label(Role::Image, "Preview").is_none());
    }

    /// The Rounded Rectangle click dialog opens on General › Corner Radius, and OK makes the radius
    /// typed (in any unit) that preference: the next dialog and the tool's next drag use it.
    #[test]
    fn rounded_rectangle_click_dialog_opens_on_and_sets_the_corner_radius() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("tool.select", json!({"tool": "roundedRectangle"})).unwrap();
        app.run("prefs.set", json!({"key": "cornerRadius", "value": 20})).unwrap();

        super::super::open_tool_dialog(&mut app, "roundedRectangle", json!({"x": 40.0, "y": 60.0}));
        let d = app.ui.dialog.as_mut().expect("Rounded Rectangle dialog");
        assert_eq!((d.kind.as_str(), d.f64("radius", 0.0)), ("roundedRectangle", 20.0));
        d.fields.insert("radius".into(), json!("0.5 in"));
        super::super::confirm(&mut app).unwrap();

        assert!(app.ui.dialog.is_none());
        let n = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].clone();
        assert_eq!(crate::panels::corner_radius(&app, &n), Some(36.0));
        assert_eq!(app.session.prefs.corner_radius, 36.0);
        assert_eq!(app.session.tool_options_of("roundedRectangle")["cornerRadius"], json!(36.0));
        super::super::open_tool_dialog(&mut app, "roundedRectangle", json!({"x": 40.0, "y": 60.0}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("radius", 0.0), 36.0);
    }

    /// A radius General › Corner Radius would refuse leaves the dialog open, draws nothing and
    /// keeps the preference.
    #[test]
    fn rounded_rectangle_click_dialog_refuses_a_radius_out_of_range() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let before = app.session.prefs.corner_radius;
        for bad in [json!(-1), json!(1297), json!("19 in"), json!("wide")] {
            super::super::open_tool_dialog(&mut app, "roundedRectangle", json!({"x": 40.0, "y": 60.0}));
            app.ui.dialog.as_mut().unwrap().fields.insert("radius".into(), bad.clone());
            assert!(super::super::confirm(&mut app).is_err(), "{bad}");
            assert!(app.ui.dialog.is_some(), "{bad}");
            assert!(app.session.doc().unwrap().doc.layers[0].children().unwrap().is_empty(), "{bad}");
            assert_eq!(app.session.prefs.corner_radius, before, "{bad}");
            app.ui.dialog = None;
        }
    }

    /// Turn the link on or off as its button or `ui.dialog.set` does, then apply it as the form
    /// does on its next frame; what [`link_changed`] returned.
    fn turn(d: &mut Dialog, on: bool) -> Option<bool> {
        d.fields.insert(LINK.into(), json!(on));
        link_changed(d)
    }

    fn type_size(d: &mut Dialog, typed: &str, value: f64) -> (f64, f64) {
        d.fields.insert(typed.into(), json!(value));
        follow_link(d, typed);
        // The form applies the link's state again on every frame: an unchanged state changes nothing.
        assert_eq!(link_changed(d), None);
        (d.f64("width", -1.0), d.f64("height", -1.0))
    }

    #[test]
    fn the_link_keeps_the_proportions_it_had_when_turned_on() {
        let mut d = Dialog::new("rectangle", json!({"x": 0.0, "y": 0.0, "width": "40 pt", "height": "20 pt"}));
        // Off: a typed side changes only itself.
        assert_eq!(turn(&mut d, false), Some(false));
        assert_eq!(type_size(&mut d, "width", 80.0), (80.0, 20.0));
        // On at 80 × 20: each side follows the other at 4 : 1, also through a zero and back.
        assert_eq!(turn(&mut d, true), Some(true));
        for (typed, value, expected) in
            [("width", 100.0, (100.0, 25.0)), ("height", 50.0, (200.0, 50.0)), ("width", 0.0, (0.0, 0.0)), ("width", 40.0, (40.0, 10.0))]
        {
            assert_eq!(type_size(&mut d, typed, value), expected, "{typed} = {value}");
        }
    }

    /// Turned off and on again from outside the dialog (`ui.dialog.set`), the link forgets the old
    /// proportions and keeps the ones the dialog shows when it comes back on.
    #[test]
    fn the_link_follows_changes_made_from_outside_the_dialog() {
        let mut d = Dialog::new("rectangle", json!({"width": 80.0, "height": 20.0}));
        assert_eq!(turn(&mut d, true), Some(true));
        assert_eq!(turn(&mut d, false), Some(false));
        assert!(!d.fields.contains_key(RATIO));
        assert_eq!(type_size(&mut d, "width", 30.0), (30.0, 20.0));
        assert_eq!(type_size(&mut d, "height", 60.0), (30.0, 60.0));
        assert_eq!(turn(&mut d, true), Some(true));
        assert_eq!(type_size(&mut d, "width", 10.0), (10.0, 20.0));
        // Set to what it already is: nothing to apply, the proportions stay.
        assert_eq!(turn(&mut d, true), None);
        assert_eq!(type_size(&mut d, "height", 40.0), (20.0, 40.0));
    }

    /// A side of zero when the link turns on leaves nothing to keep (no NaN): typing changes only
    /// the side typed, even once both sides are sizes again, until the link is turned on again.
    #[test]
    fn a_link_turned_on_at_a_zero_side_stays_unconstrained_until_turned_on_again() {
        let mut d = Dialog::new("ellipse", json!({"width": 0.0, "height": 30.0}));
        assert_eq!(turn(&mut d, true), Some(true));
        assert!(!d.fields.contains_key(RATIO));
        assert_eq!(type_size(&mut d, "width", 50.0), (50.0, 30.0));
        assert_eq!(type_size(&mut d, "height", 60.0), (50.0, 60.0));
        assert!(!d.fields.contains_key(RATIO));
        assert_eq!((turn(&mut d, false), turn(&mut d, true)), (Some(false), Some(true)));
        assert_eq!(type_size(&mut d, "width", 100.0), (100.0, 120.0));
    }

    /// The real dialog: Width comes before Height, the link is a named button that turns on with a
    /// click, and the next Rectangle dialog opens with it on.
    #[test]
    fn the_rectangle_dialog_shows_width_then_height_and_a_link_button() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        const LINK_NAME: &str = "Constrain Width and Height Proportions";
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        super::super::open_tool_dialog(&mut app, "rectangle", json!({"x": 10.0, "y": 10.0}));
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(5);
        h.check_accessibility();
        let (w, ht) = (h.get_by_role_and_label(Role::TextInput, "Width:").rect(), h.get_by_role_and_label(Role::TextInput, "Height:").rect());
        assert!(w.center().y < ht.center().y, "Width is above Height: {w:?} {ht:?}");
        assert!(!h.state().ui.dialog.as_ref().unwrap().bool(LINK));
        let link = h.get_by_role_and_label(Role::Button, LINK_NAME).rect();
        assert!(link.left() > w.right() && link.top() < ht.center().y && link.bottom() > w.center().y, "the link sits beside both rows: {link:?}");
        h.get_by_role_and_label(Role::Button, LINK_NAME).click();
        h.run_steps(2);
        let d = h.state().ui.dialog.as_ref().unwrap();
        assert!(d.bool(LINK));
        assert_eq!(d.f64(RATIO, 0.0), 1.0);
        // Cancel, then click again: the link is still on.
        let reopen = |h: &mut egui_kittest::Harness<'_, VectorcraftApp>| {
            h.state_mut().ui.dialog = None;
            h.run_steps(2);
            super::super::open_tool_dialog(h.state_mut(), "rectangle", json!({"x": 10.0, "y": 10.0}));
            h.run_steps(2);
            h.state().ui.dialog.as_ref().unwrap().bool(LINK)
        };
        assert!(reopen(&mut h));
        // Turned off from outside the dialog (`ui.dialog.set`): the next dialog opens with it off.
        h.state_mut().ui.dialog.as_mut().unwrap().fields.insert(LINK.into(), json!(false));
        h.run_steps(2);
        assert!(!h.state().ui.dialog.as_ref().unwrap().fields.contains_key(RATIO));
        assert!(!reopen(&mut h));
    }

    /// With the link on, typing a width over a 40 × 20 rectangle's and pressing Enter draws it
    /// 80 × 40.
    #[test]
    fn typing_a_width_with_the_link_on_draws_the_shape_in_proportion() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
        super::super::open_tool_dialog(&mut app, "rectangle", json!({"x": 10.0, "y": 10.0}));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("width".into(), json!(40.0));
        d.fields.insert("height".into(), json!(20.0));
        d.fields.insert(LINK.into(), json!(true));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
            out.textures_delta.clear();
        };
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&mut app, vec![]);
        frame(&mut app, vec![egui::Event::Text("80 pt".into())]);
        frame(&mut app, vec![enter]);
        assert!(app.ui.dialog.is_none());
        let doc = &app.session.doc().unwrap().doc;
        let b = doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
        assert!((b.width() - 80.0).abs() < 1e-6 && (b.height() - 40.0).abs() < 1e-6, "{b:?}");
    }

    /// The Ellipse dialog opens with Anchors 4, which draws a live ellipse; OK with 6 draws a plain
    /// path of 6 anchors, and the next Ellipse dialog opens on 6.
    #[test]
    fn the_ellipse_dialog_draws_and_keeps_its_anchor_count() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("tool.select", json!({"tool": "ellipse"})).unwrap();
        let last = |app: &VectorcraftApp| {
            let doc = &app.session.doc().unwrap().doc;
            doc.layers[0].children().unwrap().last().cloned().expect("an ellipse")
        };

        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 10.0}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("anchors", 0.0), 4.0);
        super::super::confirm(&mut app).unwrap();
        let n = last(&app);
        assert!(matches!(n.live_shape(), Some(vectorcraft_doc::LiveShape::Ellipse { .. })), "{:?}", n.live_shape());

        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 150.0, "y": 150.0}));
        app.ui.dialog.as_mut().unwrap().fields.insert("anchors".into(), json!("6"));
        super::super::confirm(&mut app).unwrap();
        let n = last(&app);
        assert!(n.live_shape().is_none());
        assert_eq!(n.path_data().unwrap().anchor_count(), 6);
        let b = n.geometric_bounds().unwrap();
        assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 100.0).abs() < 1e-6, "{b:?}");

        // Kept as the Ellipse tool's `anchors` option for the next click, after another tool too.
        assert_eq!(app.session.tool_options_of("ellipse")["anchors"], json!(6));
        app.run("tool.select", json!({"tool": "rectangle"})).unwrap();
        app.run("tool.select", json!({"tool": "ellipse"})).unwrap();
        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 150.0}));
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("anchors", 0.0), 6.0);
        app.ui.dialog = None;
        // `tool.setOption` keeps it from 2 to 1000, and the dialog opens on what it keeps.
        for (set, want) in [(json!(1), 2.0), (json!(2000), 1000.0), (json!(7), 7.0)] {
            app.run("tool.setOption", json!({"tool": "ellipse", "values": {"anchors": set}})).unwrap();
            super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 150.0}));
            assert_eq!(app.ui.dialog.as_ref().unwrap().f64("anchors", 0.0), want, "{set}");
            app.ui.dialog = None;
        }
    }

    /// The dialog's Anchors field is named, under Height, and typing into it sets the count.
    #[test]
    fn the_ellipse_dialog_shows_an_anchors_field() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 10.0}));
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(5);
        h.check_accessibility();
        let height = h.get_by_role_and_label(Role::TextInput, "Height:").rect();
        let anchors = h.get_by_role_and_label(Role::TextInput, "Anchors:").rect();
        assert!(anchors.center().y > height.center().y, "Anchors is below Height: {anchors:?} {height:?}");
        assert_eq!(ellipse_anchors(h.state().ui.dialog.as_ref().unwrap()), 4);
    }

    /// A harness drawing `app`'s dialog, with the app's fonts installed on its first frame.
    fn dialog_harness(app: VectorcraftApp) -> egui_kittest::Harness<'static, VectorcraftApp> {
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(5);
        h
    }

    /// The dialog's Width and Height in points.
    fn dialog_size(h: &egui_kittest::Harness<'_, VectorcraftApp>) -> (f64, f64) {
        let d = h.state().ui.dialog.as_ref().expect("the dialog is open");
        (d.f64("width", -1.0), d.f64("height", -1.0))
    }

    /// Replaces the text of the field named `name` with `text` and leaves it with Tab.
    fn retype(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, name: &str, text: &str) {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let all = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        h.get_by_role_and_label(Role::TextInput, name).focus();
        h.run_steps(2);
        h.key_press_modifiers(all, egui::Key::A);
        h.get_by_role_and_label(Role::TextInput, name).type_text(text);
        h.run_steps(2);
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
    }

    /// In the Ellipse dialog, a click on the word Height copies Width into Height, also a Width
    /// typed and not yet committed; the fields keep their names. Cancel discards it; OK draws the
    /// circle as one undo step.
    #[test]
    fn clicking_height_in_the_ellipse_dialog_copies_width_into_height() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 10.0}));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("width".into(), json!(80.0));
        d.fields.insert("height".into(), json!(30.0));
        let mut h = dialog_harness(app);
        h.check_accessibility();
        let button = h.get_by_role_and_label(Role::Button, "Height:");
        assert_eq!(button.accesskit_node().description().as_deref(), Some("Copy Width to Height"));
        h.get_by_role_and_label(Role::Button, "Height:").click();
        h.run_steps(2);
        assert_eq!(dialog_size(&h), (80.0, 80.0));
        let d = h.state().ui.dialog.as_ref().unwrap();
        assert!(!d.bool(LINK) && !d.fields.contains_key(RATIO), "the link stays off");
        // The value fields keep their names.
        h.get_by_role_and_label(Role::TextInput, "Width:");
        h.get_by_role_and_label(Role::TextInput, "Height:");

        // A Width typed and not committed with Enter or Tab is the one copied.
        let all = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
        h.get_by_role_and_label(Role::TextInput, "Width:").focus();
        h.run_steps(2);
        h.key_press_modifiers(all, egui::Key::A);
        h.get_by_role_and_label(Role::TextInput, "Width:").type_text("50 pt");
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, "Height:").click();
        h.run_steps(3);
        assert_eq!(dialog_size(&h), (50.0, 50.0));

        // A Width the field refuses when the click takes its focus (not a distance, or empty)
        // is not copied: Height keeps its value.
        h.state_mut().ui.dialog.as_mut().unwrap().fields.insert("height".into(), json!(30.0));
        h.run_steps(2);
        for typed in ["wide", ""] {
            h.get_by_role_and_label(Role::TextInput, "Width:").focus();
            h.run_steps(2);
            h.key_press_modifiers(all, egui::Key::A);
            if typed.is_empty() {
                h.key_press(egui::Key::Backspace);
            } else {
                h.get_by_role_and_label(Role::TextInput, "Width:").type_text(typed);
            }
            h.run_steps(2);
            h.get_by_role_and_label(Role::Button, "Height:").click();
            h.run_steps(3);
            assert_eq!(dialog_size(&h), (50.0, 30.0), "{typed:?}");
        }
        // The field shows its value again, and the next click copies it.
        h.get_by_role_and_label(Role::Button, "Height:").click();
        h.run_steps(3);
        assert_eq!(dialog_size(&h), (50.0, 50.0));

        // A Height typed and not committed does not outlast the copy: a distance is replaced by
        // Width, and a Height the field refuses keeps the copy from happening, as it keeps OK from
        // confirming.
        for (typed, after) in [("70 pt", (50.0, 50.0)), ("tall", (50.0, 30.0))] {
            h.state_mut().ui.dialog.as_mut().unwrap().fields.insert("height".into(), json!(30.0));
            h.run_steps(2);
            h.get_by_role_and_label(Role::TextInput, "Height:").focus();
            h.run_steps(2);
            h.key_press_modifiers(all, egui::Key::A);
            h.get_by_role_and_label(Role::TextInput, "Height:").type_text(typed);
            h.run_steps(2);
            h.get_by_role_and_label(Role::Button, "Height:").click();
            h.run_steps(3);
            assert_eq!(dialog_size(&h), after, "{typed:?}");
        }

        // Cancel: nothing is drawn.
        let undo = |h: &egui_kittest::Harness<'_, VectorcraftApp>| h.state().session.active().unwrap().history.undo.len();
        let before = undo(&h);
        crate::dialogs::cancel(h.state_mut());
        h.run_steps(2);
        assert!(h.state().session.doc().unwrap().doc.layers[0].children().unwrap().is_empty());
        assert_eq!(undo(&h), before);

        // OK: one circle, one undo step.
        super::super::open_tool_dialog(h.state_mut(), "ellipse", json!({"x": 10.0, "y": 10.0}));
        h.state_mut().ui.dialog.as_mut().unwrap().fields.insert("width".into(), json!(64.0));
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, "Height:").click();
        h.run_steps(2);
        super::super::confirm(h.state_mut()).unwrap();
        assert!(h.state().ui.dialog.is_none());
        assert_eq!(undo(&h), before + 1);
        let doc = &h.state().session.doc().unwrap().doc;
        let b = doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
        assert!((b.width() - 64.0).abs() < 1e-6 && (b.height() - 64.0).abs() < 1e-6, "{b:?}");
    }

    /// The Height button works from the keyboard: Space and Enter copy Width into Height, and
    /// that Enter does not press OK. With the link on, the link stays on and keeps the circle when
    /// Width is typed next; a Width of 0 is copied and leaves the link no proportions to keep.
    #[test]
    fn the_ellipse_height_button_copies_width_from_the_keyboard_and_keeps_the_link_round() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        super::super::open_tool_dialog(&mut app, "ellipse", json!({"x": 10.0, "y": 10.0}));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("width".into(), json!(80.0));
        d.fields.insert("height".into(), json!(20.0));
        d.fields.insert(LINK.into(), json!(true));
        let mut h = dialog_harness(app);
        assert_eq!(h.state().ui.dialog.as_ref().unwrap().f64(RATIO, 0.0), 0.25);
        h.get_by_role_and_label(Role::Button, "Height:").focus();
        h.run_steps(2);
        h.key_press(egui::Key::Space);
        h.run_steps(2);
        assert_eq!(dialog_size(&h), (80.0, 80.0));
        let d = h.state().ui.dialog.as_ref().unwrap();
        assert!(d.bool(LINK), "the link stays on");
        assert_eq!(d.f64(RATIO, 0.0), 1.0);
        // The link now keeps the circle.
        retype(&mut h, "Width:", "40 pt");
        assert_eq!(dialog_size(&h), (40.0, 40.0));

        // Enter on the button copies, and the dialog stays open.
        h.state_mut().ui.dialog.as_mut().unwrap().fields.insert("width".into(), json!(0.0));
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, "Height:").focus();
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert!(h.state().ui.dialog.is_some(), "Enter on the Height button does not press OK");
        assert_eq!(dialog_size(&h), (0.0, 0.0));
        let d = h.state().ui.dialog.as_ref().unwrap();
        assert!(d.bool(LINK) && !d.fields.contains_key(RATIO), "no proportions to keep at 0");
        assert!(h.state().session.doc().unwrap().doc.layers[0].children().unwrap().is_empty());
    }

    /// Enter on the focused link button turns the link on and leaves the dialog open, as Space does.
    #[test]
    fn enter_on_the_link_button_turns_the_link_on_without_pressing_ok() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        const LINK_NAME: &str = "Constrain Width and Height Proportions";
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        super::super::open_tool_dialog(&mut app, "rectangle", json!({"x": 10.0, "y": 10.0}));
        let mut h = dialog_harness(app);
        assert!(!h.state().ui.dialog.as_ref().unwrap().bool(LINK));
        h.get_by_role_and_label(Role::Button, LINK_NAME).focus();
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let d = h.state().ui.dialog.as_ref().expect("Enter on the link button does not press OK");
        assert!(d.bool(LINK));
        assert!(h.state().session.doc().unwrap().doc.layers[0].children().unwrap().is_empty());
    }

    /// Rectangle and Rounded Rectangle keep plain Width and Height labels.
    #[test]
    fn only_the_ellipse_dialog_has_the_height_button() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        for kind in ["rectangle", "roundedRectangle"] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
            super::super::open_tool_dialog(&mut app, kind, json!({"x": 10.0, "y": 10.0}));
            let h = dialog_harness(app);
            h.get_by_role_and_label(Role::TextInput, "Height:");
            assert!(h.query_by_role_and_label(Role::Button, "Height:").is_none(), "{kind}");
        }
    }

    /// A Width that is missing, empty, not a distance or not finite leaves Height as it is; a
    /// Width with a unit is copied in points, and 0 is copied.
    #[test]
    fn copying_width_takes_only_a_width_that_reads() {
        for width in [None, Some(json!("")), Some(json!("wide")), Some(json!("inf")), Some(json!(null)), Some(json!(true))] {
            let mut d = Dialog::new("ellipse", json!({"height": 30.0}));
            if let Some(w) = width.clone() {
                d.fields.insert("width".into(), w);
            }
            copy_width_to_height(&mut d);
            assert_eq!(d.fields.get("height"), Some(&json!(30.0)), "{width:?}");
        }
        for (width, want) in [(json!("1 in"), 72.0), (json!(0), 0.0), (json!(12.5), 12.5)] {
            let mut d = Dialog::new("ellipse", json!({"width": width.clone(), "height": 30.0}));
            copy_width_to_height(&mut d);
            assert_eq!(d.f64("height", -1.0), want, "{width}");
        }
    }

    #[test]
    fn ellipse_anchors_take_the_nearest_whole_number_from_2_to_1000() {
        let d = |v: Value| Dialog::new("ellipse", json!({ "anchors": v }));
        for (v, want) in [(json!(6), 6), (json!("7"), 7), (json!(5.6), 6), (json!(1), 2), (json!(0), 2), (json!(-3), 2), (json!(5000), 1000)] {
            assert_eq!(ellipse_anchors(&d(v.clone())), want, "{v}");
        }
        assert_eq!(ellipse_anchors(&Dialog::new("ellipse", json!({}))), 4);
    }

    #[test]
    fn a_negative_line_length_draws_and_keeps_the_same_direction() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("tool.select", json!({"tool": "lineSegment"})).unwrap();
        let line_bounds = |app: &VectorcraftApp, i: usize| {
            let doc = &app.session.doc().unwrap().doc;
            let n = &doc.layers[0].children().unwrap()[i];
            n.path_data().and_then(|p| p.bounds()).expect("line bounds")
        };

        super::super::open_tool_dialog(&mut app, "lineSegment", json!({"x": 100.0, "y": 60.0}));
        let d = app.ui.dialog.as_mut().expect("Line Segment dialog");
        d.fields.insert("length".into(), json!(-80.0));
        d.fields.insert("angle".into(), json!(0.0));
        super::super::confirm(&mut app).unwrap();
        let first = line_bounds(&app, 0);
        assert!((first.x0 - 20.0).abs() < 1e-6 && (first.x1 - 100.0).abs() < 1e-6, "{first:?}");
        let kept = app.session.tool_options_of("lineSegment");
        assert_eq!((kept["length"].as_f64(), kept["angle"].as_f64()), (Some(80.0), Some(180.0)));

        // The next click opens at what was kept and draws the same way.
        super::super::open_tool_dialog(&mut app, "lineSegment", json!({"x": 100.0, "y": 120.0}));
        super::super::confirm(&mut app).unwrap();
        let second = line_bounds(&app, 1);
        assert!((second.x0 - 20.0).abs() < 1e-6 && (second.x1 - 100.0).abs() < 1e-6, "{second:?}");
    }
}
