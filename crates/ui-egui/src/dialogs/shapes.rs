//! Shape size dialogs (a click with a shape tool): Rectangle, Rounded Rectangle, Ellipse, Polygon,
//! Star, Line Segment Tool Options and the two grids (size, divider counts and skews, switches).

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect};

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::panels::transform::artboard_rect;
use crate::state::Dialog;
use crate::widgets;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |d| title(&d.kind).into(), body, confirm, ..DialogSpec::FORM };

fn title(kind: &str) -> &'static str {
    match kind {
        "rectangle" => tl!("Rectangle"),
        "roundedRectangle" => tl!("Rounded Rectangle"),
        "ellipse" => tl!("Ellipse"),
        "polygon" => tl!("Polygon"),
        "star" => tl!("Star"),
        "rectangularGrid" => tl!("Rectangular Grid"),
        "polarGrid" => tl!("Polar Grid"),
        _ => tl!("Line Segment Tool Options"),
    }
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
    } else {
        form::grid(ui, d, unit);
    }
    let own = match d.kind.as_str() {
        "rectangularGrid" => ("frame", tl!("Use Outside Rectangle As Frame")),
        "polarGrid" => ("compound", tl!("Create Compound Path From Ellipses")),
        _ => return false,
    };
    ui.add_space(8.0);
    for (key, label) in [own, ("fill", tl!("Fill Grid"))] {
        form::check(ui, d, key, label);
    }
    false
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
    let count = match d.kind.as_str() {
        "polygon" => Some(("sides", 6.0, 3.0)),
        "star" => Some(("points", 5.0, 2.0)),
        _ => None,
    };
    let options = count.map(|(key, default, min)| {
        let mut values = json!({});
        values[key] = json!(d.f64(key, default).clamp(min, 1000.0) as u64);
        values
    });
    let (x, y) = (d.f64("x", 0.0), d.f64("y", 0.0));
    let (bx, by) = box_origin(d);
    let (id, params) = match d.kind.as_str() {
        "ellipse" => ("shape.ellipse", json!({"x": bx, "y": by, "width": d.f64("width", 100.0), "height": d.f64("height", 100.0)})),
        "polygon" => ("shape.polygon", json!({"cx": x, "cy": y, "radius": d.f64("radius", 50.0), "sides": d.f64("sides", 6.0) as u64})),
        "star" => (
            "shape.star",
            json!({"cx": x, "cy": y, "radius1": d.f64("radius1", 50.0), "radius2": d.f64("radius2", 25.0), "points": d.f64("points", 5.0) as u64}),
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
            let (l, a) = (d.f64("length", 100.0), d.f64("angle", 0.0).to_radians());
            ("shape.line", json!({"x1": x, "y1": y, "x2": x + l * a.cos(), "y2": y - l * a.sin()}))
        }
        _ => (
            "shape.rectangle",
            json!({"x": bx, "y": by, "width": d.f64("width", 100.0), "height": d.f64("height", 100.0), "radius": d.f64("radius", 0.0)}),
        ),
    };
    // On the active plane while the perspective grid shows, sized in plane units from the click.
    let options = options.or_else(|| matches!(d.kind.as_str(), "rectangularGrid" | "polarGrid").then(|| params.clone()));
    let result = match vectorcraft_engine::perspective_click(&app.session, id, &params, vectorcraft_geom::Point::new(x, y)) {
        Some((id, params)) => run_and_close(app, &id, params),
        None => run_and_close(app, id, params),
    }?;
    if let Some(options) = options {
        app.run("tool.setOption", json!({"tool": d.kind, "values": options}))?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_click_centres_the_typed_size_on_the_click() {
        let fields = |alt: bool| Dialog::new("rectangle", json!({"x": 100.0, "y": 50.0, "alt": alt, "width": "40 pt", "height": "20 pt"}));
        assert_eq!(box_origin(&fields(false)), (100.0, 50.0));
        assert_eq!(box_origin(&fields(true)), (80.0, 40.0));
    }
}
