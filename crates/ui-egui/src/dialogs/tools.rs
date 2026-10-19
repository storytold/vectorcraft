//! Dialogs a tool click opens: shape sizes, transform values, graph size and data, flare options,
//! artboard options and the gradient stop popover.

use serde_json::{Value, json};

use super::shapes::grid_pin_to_anchor;
use crate::VectorcraftApp;
use crate::state::Dialog;

/// A click with a shape tool opens its size dialog.
pub fn open_tool_dialog(app: &mut VectorcraftApp, kind: &str, p: Value) {
    let x = p.get("x").and_then(Value::as_f64).unwrap_or(0.0);
    let y = p.get("y").and_then(Value::as_f64).unwrap_or(0.0);
    let d = match kind {
        "rectangle" | "ellipse" => Dialog::new(kind, json!({"x": x, "y": y, "width": "100 pt", "height": "100 pt"})),
        "roundedRectangle" => Dialog::new(kind, json!({"x": x, "y": y, "width": "100 pt", "height": "100 pt", "radius": "12 pt"})),
        "polygon" => Dialog::new(kind, json!({"x": x, "y": y, "radius": "50 pt", "sides": 6})),
        "star" => Dialog::new(kind, json!({"x": x, "y": y, "radius1": "50 pt", "radius2": "25 pt", "points": 5})),
        "lineSegment" => Dialog::new(kind, json!({"x": x, "y": y, "length": "100 pt", "angle": 0})),
        // The grids: the tool's stored options (divider counts and skews, switches) at the default size.
        "rectangularGrid" | "polarGrid" => {
            let mut obj = match app.session.tool_options_of(kind) {
                Value::Object(o) => o,
                _ => serde_json::Map::new(),
            };
            for (key, value) in [("__anchorX", json!(x)), ("__anchorY", json!(y)), ("width", json!("100 pt")), ("height", json!("100 pt"))] {
                obj.insert(key.into(), value);
            }
            obj.insert("reference".into(), json!(0));
            let mut d = Dialog::new(kind, Value::Object(obj));
            grid_pin_to_anchor(&mut d);
            d
        }
        // Graph tool click: the graph's size.
        "graph" => Dialog::new(
            "command",
            json!({"__command": "graph.create", "__label": "Graph", "type": p.get("type").cloned().unwrap_or(json!("column")), "x": x, "y": y, "width": 200, "height": 150}),
        ),
        "graphData" => {
            // After drawing a graph: Graph Data, its worksheet.
            if let Err(e) = super::graph_data::open(app) {
                app.status(e);
            }
            return;
        }
        // A Flare tool click: the Flare Tool Options, with the tool's values; OK draws a flare there.
        "flare" => {
            super::flare_options::open(app, Some(vectorcraft_geom::Point::new(x, y)));
            return;
        }
        "rotate" | "reflect" | "scale" | "shear" | "artboardOptions" => {
            let mut base = match kind {
                "rotate" => json!({"angle": 0}),
                "reflect" => json!({"axis": "vertical", "angle": 90}),
                "scale" => json!({"sx": 100, "sy": 100, "uniform": true}),
                "shear" => json!({"angle": 0, "axis": "horizontal"}),
                _ => json!({}),
            };
            if let (Some(b), Some(o)) = (base.as_object_mut(), p.as_object()) {
                for (k, v) in o {
                    b.insert(k.clone(), v.clone());
                }
            }
            Dialog::new(kind, base)
        }
        // Double-clicking a stop on the gradient annotator: its popover, next to the stop's chip.
        "gradientStop" => Dialog::new(kind, json!({"index": p.get("index").cloned().unwrap_or(json!(0)), "x": x, "y": y, "tab": "color"})),
        // Double-clicking a width point with the Width tool: Width Point Edit.
        super::width_point::KIND => {
            if let Err(e) = super::width_point::open(app, &p) {
                app.status(e);
            }
            return;
        }
        // The Blend tool's double-click and Alt-click: Blend Options.
        super::blend_options::KIND => {
            if let Err(e) = super::blend_options::open(app) {
                app.status(e);
            }
            return;
        }
        // Double-clicking a plane widget of the perspective grid: that plane's options.
        super::perspective_plane::KIND => {
            if let Err(e) = super::perspective_plane::open(app, &p) {
                app.status(e);
            }
            return;
        }
        // Double-clicking a Live Corners widget: Corners.
        super::corners::KIND => {
            if let Err(e) = super::corners::open(app, &p) {
                app.status(e);
            }
            return;
        }
        // Double-clicking a slice with the Slice Selection tool: Slice Options.
        super::slices::OPTIONS => {
            if let Err(e) = super::slices::open_options(app) {
                app.status(e);
            }
            return;
        }
        _ => return,
    };
    app.ui.dialog = Some(d);
}
