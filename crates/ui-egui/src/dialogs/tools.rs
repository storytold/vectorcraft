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
            let Some(document) = app.session.active().map(|doc| doc.uid) else {
                app.status("Open a document before drawing a graph");
                return;
            };
            let at = click_of(&p).map_or(Point::new(0.0, 0.0), |c| c.at);
            let graph_type = p.get("type").cloned().unwrap_or(json!("column"));
            let params = [
                ("type", graph_type),
                ("x", json!(at.x)),
                ("y", json!(at.y)),
                ("width", json!(200)),
                ("height", json!(150)),
                ("__doc", json!(document)),
            ];
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

#[cfg(test)]
mod tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    use super::*;

    fn confirm_graph(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        let host = app.dialogs().unwrap();
        for _ in 0..2 {
            host.with(|h| h.confirm_by_id(&ctx, dialog_id("graph.create")));
            super::super::host::pump(app, &ctx);
        }
    }

    #[test]
    fn graph_size_without_a_document_keeps_the_source_status() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        open_tool_dialog(&mut app, "graph", json!({"x": 40.0, "y": 30.0}));

        assert_eq!(app.ui.status, "Open a document before drawing a graph");
        assert!(!app.any_dialog_open());
    }

    #[test]
    fn graph_size_does_not_draw_in_a_different_document() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let opening = app.session.doc().unwrap().doc.clone();
        let click = json!({"x": 40.0, "y": 30.0, "type": "column"});
        open_tool_dialog(&mut app, "graph", click.clone());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let other = app.session.doc().unwrap().doc.clone();

        confirm_graph(&mut app);

        assert_eq!(app.session.doc().unwrap().doc, other, "confirmation leaves the other document unchanged");
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, opening);
        open_tool_dialog(&mut app, "graph", click);
        confirm_graph(&mut app);
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(doc.layers[0].children().unwrap().len(), 1);
    }

    #[test]
    fn graph_size_does_not_draw_after_file_revert() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        let dir = vectorcraft_testkit::TempDir::new("graph-click-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        let click = json!({"x": 40.0, "y": 30.0, "type": "column"});
        open_tool_dialog(&mut app, "graph", click.clone());

        app.run("shape.rectangle", json!({"x": 200, "y": 120, "width": 20, "height": 20})).unwrap();
        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let restored = app.session.doc().unwrap().doc.clone();
        confirm_graph(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, restored, "OK must not draw on the restored drawing");
        assert!(app.any_dialog_open(), "the refused dialog stays open");

        let ctx = app.ui_ctx.clone().unwrap();
        app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, dialog_id("graph.create")));
        super::super::host::pump(&mut app, &ctx);
        open_tool_dialog(&mut app, "graph", click);
        confirm_graph(&mut app);
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 1, "a click after the revert draws");
    }
}
