//! Shape size dialogs (a click with a shape tool): Rectangle, Rounded Rectangle, Ellipse, Polygon,
//! Star, Line Segment, Arc and Spiral tool options, and the two grids (size, divider counts and skews, switches).
//! The Line Segment, Arc and grid dialogs are also their tools' Tool Options (a double-click on the
//! tool, [`open_options`]): OK then sets the options the tool's next drag and click use, and draws
//! nothing.

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
        form::choice(ui, d, "arcType", tl!("Type:"), (form::FIELD_W, form::FIELD_W - 12.0), &[("open", tl!("Open")), ("closed", tl!("Closed"))]);
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
        .or_else(|| OPTION_TOOLS.contains(&d.kind.as_str()).then(|| tool_options(d)));
    let (x, y) = (d.f64("x", 0.0), d.f64("y", 0.0));
    let (bx, by) = box_origin(d);
    let (id, params) = match d.kind.as_str() {
        "ellipse" => ("shape.ellipse", json!({"x": bx, "y": by, "width": d.f64("width", 100.0), "height": d.f64("height", 100.0)})),
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
            json!({"x": bx, "y": by, "width": d.f64("width", 100.0), "height": d.f64("height", 100.0), "radius": d.f64("radius", 0.0)}),
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
    Ok(result)
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

    #[test]
    fn alt_click_centres_the_typed_size_on_the_click() {
        let fields = |alt: bool| Dialog::new("rectangle", json!({"x": 100.0, "y": 50.0, "alt": alt, "width": "40 pt", "height": "20 pt"}));
        assert_eq!(box_origin(&fields(false)), (100.0, 50.0));
        assert_eq!(box_origin(&fields(true)), (80.0, 40.0));
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
