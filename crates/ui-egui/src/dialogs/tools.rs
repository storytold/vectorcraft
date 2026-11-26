//! Dialogs a tool click opens: shape sizes, transform values, graph size and data, flare options,
//! artboard options and the gradient stop popover.

use serde_json::{Value, json};
use vectorcraft_geom::Point;

use super::shapes::Click;
use super::{artboard_options, command, flare_options, gradient_stop, shapes, transform};
use crate::VectorcraftApp;

/// Sends an opener's failure to the status line.
pub(super) fn reported<T>(app: &mut VectorcraftApp, result: Result<T, String>) {
    if let Err(e) = result {
        app.status(e);
    }
}

/// The click a tool request carries: `None` unless `x` and `y` are finite numbers.
fn click_of(p: &Value) -> Option<Click> {
    let coordinate = |key: &str| p.get(key).and_then(Value::as_f64).filter(|n| n.is_finite());
    let alt = p.get("alt").and_then(Value::as_bool).unwrap_or(false);
    Some(Click { at: Point::new(coordinate("x")?, coordinate("y")?), alt })
}

/// A click with a shape tool opens its size dialog.
pub fn open_tool_dialog(app: &mut VectorcraftApp, kind: &str, p: Value) {
    match kind {
        "rectangle" | "ellipse" | "roundedRectangle" | "polygon" | "star" | "lineSegment" | "spiral" | "arc" | "rectangularGrid" | "polarGrid" => {
            shapes::open(app, kind, click_of(&p));
        }
        // Graph tool click: the graph's size.
        "graph" => {
            let at = click_of(&p).map_or(Point::new(0.0, 0.0), |c| c.at);
            let graph_type = p.get("type").cloned().unwrap_or(json!("column"));
            let params = [("type", graph_type), ("x", json!(at.x)), ("y", json!(at.y)), ("width", json!(200)), ("height", json!(150))];
            let params = params.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            let r = command::open(app, "graph.create", "Graph", params);
            reported(app, r);
        }
        "graphData" => {
            // After drawing a graph: Graph Data, its worksheet.
            let r = super::graph_data::open(app);
            reported(app, r);
        }
        // A Flare tool click: the Flare Tool Options, with the tool's values; OK draws a flare there.
        "flare" => {
            flare_options::open(app, click_of(&p).map(|c| c.at));
        }
        "rotate" | "reflect" | "scale" | "shear" => {
            if let Some(k) = transform::Kind::of_tool(kind) {
                transform::open(app, k, &p);
            }
        }
        "artboardOptions" => {
            let r = artboard_options::open(app);
            reported(app, r);
        }
        // Double-clicking a stop on the gradient annotator: its popover, next to the stop's chip.
        "gradientStop" => {
            let index = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let coordinate = |key: &str| p.get(key).and_then(Value::as_f64).unwrap_or(0.0);
            gradient_stop::open_at(app, index, gradient_stop::Anchor::Chip(Point::new(coordinate("x"), coordinate("y"))));
        }
        // Image Trace's Ignore Color eyedropper updates the panel, not the document's paint.
        "imageTraceIgnoreColor" => crate::panels::image_trace::apply_ignore_color_sample(app, &p),
        // Brush Options › Key Color eyedropper: update the open dialog, do not replace it.
        "brushKeyColor" => {
            if let Some(c) = p.get("color") {
                super::brush_art_options::apply_key_color_sample(app, c);
            }
        }
        // Double-clicking a width point with the Width tool: Width Point Edit.
        super::width_point::KIND => {
            let r = super::width_point::open(app, &p);
            reported(app, r);
        }
        // The Blend tool's double-click and Alt-click: Blend Options.
        super::blend_options::KIND => {
            let r = super::blend_options::open(app);
            reported(app, r);
        }
        // Double-clicking a plane widget of the perspective grid: that plane's options.
        super::perspective_plane::KIND => {
            let r = super::perspective_plane::open(app, &p);
            reported(app, r);
        }
        // Double-clicking a Live Corners widget: Corners.
        super::corners::KIND => {
            let r = super::corners::open(app, &p);
            reported(app, r);
        }
        // Double-clicking a slice with the Slice Selection tool: Slice Options.
        super::slices::OPTIONS => {
            let r = super::slices::open_options(app);
            reported(app, r);
        }
        // A click or double-click that would edit type set in fonts that aren't available.
        vectorcraft_tools::text::MISSING_FONT_EDIT => super::confirm::ask_missing_font_edit(app, &p),
        _ => {}
    }
}
