use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_doc::{CharStyle, ColumnDesign, ColumnType, Fractions, GraphKind, GraphSpec, NodeKind};

use crate::{NodeId, Session};

#[test]
fn fmt_value_does_not_saturate_beyond_i64() {
    assert_eq!(super::fmt_value(1e19), "10000000000000000000");
    assert_eq!(super::fmt_value(2e19), "20000000000000000000");
    assert_eq!(super::fmt_value(-2e19), "-20000000000000000000");
    assert_eq!(super::fmt_value(42.0), "42");
    assert_eq!(super::fmt_value(-0.0), "0");
    assert_eq!(super::fmt_value(2.5), "2.5");
}

fn graph(s: &mut Session, ty: &str) -> NodeId {
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6";
    NodeId(
        s.execute("graph.create", &json!({"type": ty, "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"].as_u64().unwrap(),
    )
}

fn group<'a>(n: &'a vectorcraft_doc::Node, name: &str) -> &'a vectorcraft_doc::Node {
    n.children().unwrap().iter().find(|c| c.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("no {name}"))
}

/// The generated group stored for series `index`, not a sibling that happens to share its name.
fn series(n: &vectorcraft_doc::Node, index: u32) -> &vectorcraft_doc::Node {
    n.children().unwrap().iter().find(|c| c.extras().series_index == Some(index)).unwrap_or_else(|| panic!("no series {index}"))
}

#[test]
fn every_graph_type_generates_series_groups() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    for ty in ["column", "stackedColumn", "bar", "stackedBar", "line", "area", "scatter", "pie", "radar"] {
        let id = graph(&mut s, ty);
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        assert!(n.extras().graph.is_some(), "{ty}");
        if ty != "scatter" {
            assert!(!group(&n, "2024").children().unwrap().is_empty(), "{ty}");
            assert!(!group(&n, "2025").children().unwrap().is_empty(), "{ty}");
        }
        assert!(n.geometric_bounds().unwrap().width() > 100.0, "{ty}");
    }
}

#[test]
fn column_heights_follow_data_and_data_edits_regenerate_in_place() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let heights = |s: &Session| -> Vec<f64> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        // Bars only (the legend swatch is the 8 × 8 square at the end).
        let g = group(&n, "2025");
        let ch = g.children().unwrap();
        ch[..ch.len() - 1].iter().map(|c| c.geometric_bounds().unwrap().height()).collect()
    };
    let h = heights(&s);
    assert!((h[2] / h[0] - 3.0).abs() < 1e-6, "{h:?}");
    // Move the graph, then change the data: it stays where it was moved.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
    let x0 = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    let v = s.execute("graph.setData", &json!({})).unwrap();
    assert!(v["csv"].as_str().unwrap().contains("Q2,5,4"));
    s.execute("graph.setData", &json!({"csv": ",2024,2025\nQ1,3,2\nQ2,5,8\nQ3,-1,6"})).unwrap();
    let h2 = heights(&s);
    assert!((h2[1] / h2[0] - 4.0).abs() < 1e-6, "{h2:?}");
    assert!((s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0 - x0).abs() < 1.0);
    // Type change keeps the data; undo restores the columns.
    s.execute("graph.setType", &json!({"type": "pie"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_ref().unwrap().kind, vectorcraft_doc::GraphKind::Pie);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["type"], json!("column"));
    // Selecting a bar inside still targets the graph.
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let bar = group(&n, "2024").children().unwrap()[0].id;
    s.execute("select.set", &json!({"ids": [bar.0]})).unwrap();
    assert!(s.execute("graph.setData", &json!({})).is_ok());
    assert!(matches!(n.kind, NodeKind::Group { .. }));
}

/// The x of every marker of series `index`, left to right.
fn marker_xs(s: &Session, id: NodeId, index: u32) -> Vec<f64> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let mut xs: Vec<f64> = series(&n, index)
        .children()
        .unwrap()
        .iter()
        .map(|c| c.geometric_bounds().unwrap())
        .filter(|b| (b.width() - 5.0).abs() < 1e-6)
        .map(|b| b.center().x)
        .collect();
    xs.sort_by(f64::total_cmp);
    xs
}

#[test]
fn edge_to_edge_lines_off_puts_line_points_at_category_centres() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // 300 pt wide, three categories: centres at 150, 250, 350 (off by default); edges at 100 and 400.
    let id = graph(&mut s, "line");
    assert_eq!(marker_xs(&s, id, 0), vec![150.0, 250.0, 350.0]);
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["edgeToEdge"], json!(false));
    s.execute("graph.setType", &json!({"edgeToEdge": true})).unwrap();
    assert_eq!(marker_xs(&s, id, 0), vec![100.0, 250.0, 400.0]);
    // A graph saved before the option existed keeps its edge-to-edge lines.
    let old: GraphSpec = serde_json::from_value(json!({"kind": "line"})).unwrap();
    assert!(old.edge_to_edge);
}

#[test]
fn radar_graphs_follow_mark_and_connect_data_points() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "radar");
    let parts = |s: &Session| series(&s.doc().unwrap().doc.node(id).unwrap().clone(), 0).children().unwrap().len();
    // The ring, three markers and the legend swatch.
    assert_eq!(parts(&s), 5);
    s.execute("graph.setType", &json!({"markPoints": false})).unwrap();
    assert_eq!(parts(&s), 2);
    s.execute("graph.setType", &json!({"markPoints": true, "connectPoints": false})).unwrap();
    assert_eq!(parts(&s), 4);
}

#[test]
fn an_overridden_value_axis_is_used_exactly() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let r = s.execute("graph.create", &json!({"type": "column", "x": 20, "y": 20, "width": 170, "height": 74, "series": ["a"], "categories": ["x", "y"], "rows": [[4839], [10488]]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let tallest = |s: &Session| -> f64 {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let bars = series(&n, 0).children().unwrap();
        bars.iter().map(|c| c.geometric_bounds().unwrap().height()).fold(0.0, f64::max)
    };
    // Calculated: a nice axis (0 to 12500 here), so the 10488 column falls short of 10488 / 12000 of the plot.
    assert!((tallest(&s) - 74.0 * 10488.0 / 12000.0).abs() > 1.0);
    s.execute("graph.setType", &json!({"axisMin": 0, "axisMax": 12000})).unwrap();
    assert!((tallest(&s) - 74.0 * 10488.0 / 12000.0).abs() < 1e-6, "{}", tallest(&s));
    let o = s.execute("graph.setType", &json!({})).unwrap();
    assert_eq!((o["axisMin"].as_f64(), o["axisMax"].as_f64()), (Some(0.0), Some(12000.0)));
    // One bound only keeps the calculated (nice) axis around it.
    s.execute("graph.setType", &json!({"axisMax": null})).unwrap();
    assert!((tallest(&s) - 74.0 * 10488.0 / 12000.0).abs() > 1.0);
}

#[test]
fn a_scatter_x_axis_that_overflows_does_not_hang() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // Finite x values whose rounded axis end is infinite: this used to add ticks forever. Now the
    // command returns: points that far off the plot can't be drawn, which is an error.
    let csv = ",y1,x1\nP,1,1.7e308\nQ,2,1.5e308";
    let refused = |r: crate::Result<serde_json::Value>| assert!(r.unwrap_err().to_string().contains("exceed the canvas"));
    refused(s.execute("graph.create", &json!({"type": "scatter", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})));
    // The same data through Graph Data, on an ordinary scatter graph: it stays as it was.
    let axes = |s: &Session, id: NodeId| group(s.doc().unwrap().doc.node(id).unwrap(), "Axes").children().unwrap().len();
    let id = make(&mut s, "scatter", ",y1,x1\nP,1,10\nQ,2,20");
    let before = axes(&s, id);
    refused(s.execute("graph.setData", &json!({"id": id.0, "csv": csv})));
    assert_eq!(axes(&s, id), before);
}

#[test]
fn tick_values_end_on_the_maximum_and_never_run_away() {
    // Counted, not accumulated: an offset range keeps its last tick.
    let t = super::tick_values(1e10, 1e10 + 1.0, 0.01);
    assert_eq!(t.len(), 101);
    assert!((t[100] - (1e10 + 1.0)).abs() < 1e-3);
    // A range too wide for f64, or a step too small to move, gives no ticks or a capped list.
    assert!(super::tick_values(-1.7e308, 1.7e308, f64::INFINITY).is_empty());
    assert!(super::tick_values(1e20, 1e20 + 16384.0, 163.84).len() <= 1001);
    assert!(super::tick_values(0.0, 1.0, 0.0).is_empty());
    // An override whose span overflows falls back to the calculated axis instead of hanging.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    graph(&mut s, "column");
    s.execute("graph.setType", &json!({"axisMin": -1.7e308, "axisMax": 1.7e308})).unwrap();
}

#[test]
fn group_selection_paint_becomes_the_series_colour_and_survives_regeneration() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    for ty in GraphKind::ALL {
        let id = graph(&mut s, ty.id());
        let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
        s.execute("paint.setFill", &json!({"ids": [series_id.0], "color": "#ff0000"})).unwrap();
        let spec = s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_deref().unwrap().clone();
        let restored: GraphSpec = serde_json::from_value(serde_json::to_value(&spec).unwrap()).unwrap();
        assert_eq!(spec, restored);
        let red = Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(spec.series_paints.first().and_then(|p| p.fill.clone()), Some(red.clone()), "{ty:?}");
        let selected_group = group(s.doc().unwrap().doc.node(id).unwrap(), "2024");
        assert!(selected_group.children().unwrap().iter().all(|n| n.appearance.fill_paint() == red), "{ty:?}");
        s.execute("graph.setData", &json!({"rows": [[2, 3], [4, 5]]})).unwrap();
        s.execute("graph.setType", &json!({"type": "line"})).unwrap();
        let spec = s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_deref().unwrap();
        assert_eq!(spec.series_paints.first().and_then(|p| p.fill.clone()), Some(red.clone()), "{ty:?}");
        let series = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap();
        assert!(series.iter().any(|n| n.appearance.fill_paint() == red), "{ty:?}");
    }
}

#[test]
fn painting_a_series_is_one_undoable_edit() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
    s.execute("paint.setFill", &json!({"ids": [series_id.0], "color": "#e8573f"})).unwrap();
    assert_eq!(
        s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_ref().unwrap().series_paints[0]
            .fill
            .as_ref()
            .and_then(Paint::color)
            .unwrap()
            .to_hex(),
        "#e8573f"
    );
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_ref().unwrap().series_paints.is_empty());
    assert_eq!(
        group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0].appearance.fill_paint().color(),
        Some(super::default_series_color(0))
    );
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(
        s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_ref().unwrap().series_paints[0]
            .fill
            .as_ref()
            .and_then(Paint::color)
            .unwrap()
            .to_hex(),
        "#e8573f"
    );
}

/// A fill that a series' marks or a value axis number have already is one undo step when the graph
/// keeps no fill for them (as after a command that paints only the art): the art stays as it was
/// and the graph keeps the fill. Given again, the fill records no undo step.
#[test]
fn a_fill_the_art_has_already_is_kept_by_the_graph_in_one_step() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
    let label = text_id(&s, id, "Axes", "4");
    for target in [series_id, label] {
        let fill = json!({"ids": [target.0], "color": "#e8573f"});
        s.execute("paint.setFill", &fill).unwrap();
        // The graph keeps nothing, as after a command that paints only the art.
        s.edit("Forget", |d, _| {
            let g = d.node_mut(id).unwrap().extras_if_any().unwrap().graph.as_deref_mut().unwrap();
            g.series_paints.clear();
            g.label_styles = vectorcraft_doc::LabelStyles::default();
            Ok(())
        })
        .unwrap();
        let art = s.doc().unwrap().doc.node(target).unwrap().clone();
        let steps = s.doc().unwrap().history.undo.len();
        s.execute("paint.setFill", &fill).unwrap();
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{target:?}");
        assert_eq!(s.doc().unwrap().doc.node(target).unwrap(), &art, "the art is as it was");
        let g = spec(&s, id);
        let kept = if target == series_id { g.series_paints[0].fill.clone() } else { g.label_styles.values.map(|st| st.fill) };
        assert_eq!(kept.and_then(|p| p.color()).map(|c| c.to_hex()), Some("#e8573f".into()), "{target:?}");
        s.execute("paint.setFill", &fill).unwrap();
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "nothing left to keep: {target:?}");
    }
}

#[test]
fn painting_one_bar_sets_the_colour_used_by_the_whole_series_on_rebuild() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let bar = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0].id;
    s.execute("paint.setFill", &json!({"ids": [bar.0], "color": "#12ab34"})).unwrap();
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let node = s.doc().unwrap().doc.node(id).unwrap();
    let green = vectorcraft_color::Color::rgb(18.0 / 255.0, 171.0 / 255.0, 52.0 / 255.0);
    assert_eq!(node.extras().graph.as_ref().unwrap().series_paints[0].fill.as_ref().and_then(Paint::color), Some(green));
    assert!(group(node, "2024").children().unwrap().iter().all(|n| n.appearance.fill_paint().color() == Some(green)));
}

/// A series index read from a file past the graph's series is not a series: painting its art
/// paints the art alone and stores nothing (no allocation sized by the file).
#[test]
fn a_series_index_past_the_graphs_series_is_ignored() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
    s.edit("Junk", |d, _| {
        d.node_mut(series_id).unwrap().extras_mut().series_index = Some(u32::MAX);
        Ok(())
    })
    .unwrap();
    let bar = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0].id;
    s.execute("paint.setFill", &json!({"ids": [bar.0, series_id.0], "color": "#12ab34"})).unwrap();
    let node = s.doc().unwrap().doc.node(id).unwrap();
    assert!(node.extras().graph.as_ref().unwrap().series_paints.is_empty());
    assert_eq!(group(node, "2024").children().unwrap()[0].appearance.fill_paint().color().map(|c| c.to_hex()), Some("#12ab34".into()));
}

#[test]
fn painting_a_line_series_stroke_sets_its_persistent_colour() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "line");
    let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
    s.execute("paint.setStroke", &json!({"ids": [series_id.0], "color": "#3751c8"})).unwrap();
    let blue = vectorcraft_color::Color::rgb(55.0 / 255.0, 81.0 / 255.0, 200.0 / 255.0);
    assert_eq!(
        s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_ref().unwrap().series_paints[0].stroke.as_ref().and_then(Paint::color),
        Some(blue)
    );
    assert!(s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_ref().unwrap().series_paints[0].fill.is_none());
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(
        group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap().iter().any(|n| n.appearance.stroke_paint().color() == Some(blue))
    );
}

#[test]
fn series_colours_survive_native_save_open_and_export() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let first = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
    let second = group(s.doc().unwrap().doc.node(id).unwrap(), "2025").id;
    s.execute("paint.setFill", &json!({"ids": [first.0], "color": "#f00"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [second.0], "color": "#008080"})).unwrap();
    let dir = vectorcraft_testkit::temp_dir("graph-palette");
    let path = dir.join("palette.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let paints = &s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_ref().unwrap().series_paints;
    assert_eq!(paints.iter().map(|p| p.fill.as_ref().and_then(Paint::color).unwrap().to_hex()).collect::<Vec<_>>(), vec!["#ff0000", "#008080"]);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[3, 4, 5]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    let swatch = group(n, "2025").children().unwrap().last().unwrap();
    assert_eq!(swatch.appearance.fill_paint().color().unwrap().to_hex(), "#008080");
    let third = group(n, "Series 3").children().unwrap().first().unwrap();
    assert_eq!(third.appearance.fill_paint().color().unwrap().to_hex(), "#cccccc");
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    let svg = svg["text"].as_str().unwrap();
    assert!(svg.contains("#ff0000") && svg.contains("#008080"));
    let png = dir.join("palette.png");
    s.execute("document.export", &json!({"format": "png", "path": png})).unwrap();
    let pixels = image::open(&png).unwrap().to_rgba8();
    assert!(pixels.pixels().any(|p| p.0 == [255, 0, 0, 255]));
    assert!(pixels.pixels().any(|p| p.0 == [0, 128, 128, 255]));
    let old: GraphSpec = serde_json::from_value(json!({"rows": [[1.0]]})).unwrap();
    assert!(old.series_paints.is_empty(), "older files keep the greyscale default");
}

#[test]
fn a_column_stroke_stays_a_stroke_and_a_marker_fill_does_not_recolour_its_line() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
    s.execute("paint.setStroke", &json!({"ids": [series_id.0], "color": "#0000ff"})).unwrap();
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let spec = s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_deref().unwrap().clone();
    let blue = Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0));
    assert_eq!(spec.series_paints[0].stroke, Some(blue.clone()));
    assert!(spec.series_paints[0].fill.is_none());
    let bar = &group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0];
    assert_eq!(bar.appearance.fill_paint().color(), Some(super::default_series_color(0)));
    assert_eq!(bar.appearance.stroke_paint(), blue);

    let id = graph(&mut s, "line");
    let marker = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[1].id;
    s.execute("paint.setFill", &json!({"ids": [marker.0], "color": "#ff0000"})).unwrap();
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let spec = s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_deref().unwrap().clone();
    assert!(spec.series_paints[0].stroke.is_none());
    let series = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap();
    assert_eq!(series[0].appearance.stroke_paint().color(), Some(super::default_series_color(0)));
    assert_eq!(series[1].appearance.fill_paint().color(), Some(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)));
}

#[test]
fn a_gradient_fill_survives_regeneration() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
    s.execute(
        "paint.setFill",
        &json!({"ids": [series_id.0], "gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
    )
    .unwrap();
    s.execute("graph.setData", &json!({"rows": [[2, 3], [4, 5]]})).unwrap();
    let spec = s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_deref().unwrap().clone();
    assert!(matches!(spec.series_paints[0].fill, Some(Paint::Gradient(_))));
    let bar = &group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0];
    assert!(matches!(bar.appearance.fill_paint(), Paint::Gradient(_)));
}

#[test]
fn a_series_named_legend_keeps_its_own_index_and_an_empty_series_is_omitted() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let csv = ",Legend,Axes\nQ1,3,2\nQ2,5,4";
    let id = NodeId(s.execute("graph.create", &json!({"type": "column", "x": 0, "y": 0, "csv": csv})).unwrap()["id"].as_u64().unwrap());
    let legend_id = {
        let n = s.doc().unwrap().doc.node(id).unwrap();
        let legend = series(n, 0);
        assert_eq!(legend.name.as_deref(), Some("Legend"));
        assert!(n.children().unwrap().iter().any(|c| c.name.as_deref() == Some("Legend") && c.extras().series_index.is_none()));
        assert!(n.children().unwrap().iter().any(|c| c.name.as_deref() == Some("Axes") && c.extras().series_index.is_none()));
        legend.id
    };
    s.execute("paint.setFill", &json!({"ids": [legend_id.0], "color": "#ff0000"})).unwrap();
    s.execute("graph.setData", &json!({"csv": csv})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    assert_eq!(n.extras().graph.as_ref().unwrap().series_paints[0].fill.as_ref().and_then(Paint::color).unwrap().to_hex(), "#ff0000");
    assert!(n.extras().graph.as_ref().unwrap().series_paints.get(1).is_none_or(|p| p.fill.is_none()));
    assert_eq!(series(n, 0).children().unwrap()[0].appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");
    assert_eq!(series(n, 1).name.as_deref(), Some("Axes"));
    assert_ne!(series(n, 1).children().unwrap()[0].appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");

    let id = graph(&mut s, "pie");
    s.execute("graph.setType", &json!({"id": id.0, "legend": false})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",Keep,Drop\nQ1,4,0\nQ2,2,0"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    assert!(n.children().unwrap().iter().any(|c| c.name.as_deref() == Some("Keep")));
    assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Drop")));
    let keep = group(n, "Keep").id;
    s.execute("paint.setFill", &json!({"ids": [keep.0], "color": "#00ff00"})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[8, 0], [1, 0]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    assert_eq!(group(n, "Keep").extras().series_index, Some(0));
    assert_eq!(n.extras().graph.as_ref().unwrap().series_paints[0].fill.as_ref().and_then(Paint::color).unwrap().to_hex(), "#00ff00");
    assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Drop")));
}

#[test]
fn csv_parsing() {
    let (s, c, r) = super::parse_csv("\t2024\t2025\nQ1\t1\t2\nQ2\t3\t4\n");
    assert_eq!(s, vec!["2024", "2025"]);
    assert_eq!(c, vec!["Q1", "Q2"]);
    assert_eq!(r, vec![vec![Some(1.0), Some(2.0)], vec![Some(3.0), Some(4.0)]]);
    let (s, c, r) = super::parse_csv("1,2\n3,4");
    assert!(s.is_empty() && c.is_empty());
    assert_eq!(r.len(), 2);
    assert_eq!(super::nice_axis(0.0, 7.0, 0), (0.0, 8.0, 2.0));
}

#[test]
fn numeric_csv_cells_round_trip_without_rounding_or_saturation() {
    let mut g = GraphSpec::default();
    g.set_cells(vec![vec![Some(1.2345), Some(-1.2345)], vec![Some(1e-6), Some(-1e-6)], vec![Some(1e20), Some(-1e20)], vec![Some(0.0), None]]);
    let csv = super::to_csv(&g);
    let (_, _, cells) = super::parse_csv(&csv);
    assert_eq!(cells, g.cells(), "{csv}");
}

#[test]
fn labels_with_commas_quotes_and_newlines_round_trip_through_csv() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let series = json!(["Revenue, USD", "Say \"hi\"", "a\tb"]);
    let categories = json!(["Doe, Jane", "2024", " padded ", "two\nlines"]);
    let rows = json!([[1, 2, 3], [4, null, 6], [7, 8, 9], [10, 11, 12]]);
    let r = s
        .execute(
            "graph.create",
            &json!({"type": "column", "x": 20, "y": 20, "width": 300, "height": 200, "series": series, "categories": categories, "rows": rows}),
        )
        .unwrap();
    let id = r["id"].as_u64().unwrap();
    let before = s.execute("graph.setData", &json!({"id": id})).unwrap();
    assert_eq!(before["series"], series);
    assert_eq!(before["categories"], categories);
    s.execute("graph.setData", &json!({"id": id, "csv": before["csv"]})).unwrap();
    let after = s.execute("graph.setData", &json!({"id": id})).unwrap();
    assert_eq!(after["series"], before["series"], "{}", before["csv"]);
    assert_eq!(after["categories"], before["categories"], "{}", before["csv"]);
    assert_eq!(after["rows"], before["rows"], "{}", before["csv"]);
}

#[test]
fn empty_cells_are_blank_and_quoted_numbers_are_labels() {
    // Quoted years label the rows; empty and non-numeric cells are blank values, not zeros.
    let (s, c, r) = super::parse_csv(",a,b\n\"2023\",1,\n\"2024\",,x\n\"2025\",3,4");
    assert_eq!(s, vec!["a", "b"]);
    assert_eq!(c, vec!["2023", "2024", "2025"]);
    assert_eq!(r, vec![vec![Some(1.0), None], vec![None, None], vec![Some(3.0), Some(4.0)]]);
    // Back to CSV: blanks stay empty and numeric labels keep their quotes, so it reads back the same.
    let mut g = GraphSpec { series: s, categories: c, ..GraphSpec::default() };
    g.set_cells(r);
    assert_eq!(g.blanks, vec![[0, 1], [1, 0], [1, 1]]);
    let csv = super::to_csv(&g);
    assert!(csv.contains("\"2024\",,"), "{csv}");
    let (s2, c2, r2) = super::parse_csv(&csv);
    assert_eq!((s2, c2, r2), (g.series.clone(), g.categories.clone(), g.cells()));
}

#[test]
fn a_row_with_an_empty_label_keeps_its_values_in_place() {
    // The label column is the first column of every row once any row has a label.
    let (_, c, r) = super::parse_csv(",a,b\nQ1,1,2\n,3,4");
    assert_eq!(c, vec!["Q1", ""]);
    assert_eq!(r, vec![vec![Some(1.0), Some(2.0)], vec![Some(3.0), Some(4.0)]]);
    // And a data window round trip keeps it (the empty label is written as "").
    let mut g = GraphSpec { series: vec!["a".into(), "b".into()], categories: c, ..GraphSpec::default() };
    g.set_cells(r);
    assert_eq!(super::parse_csv(&super::to_csv(&g)).2, g.cells());
    // Without series or labels, a first row starting with a blank is still data after a round trip.
    let mut g = GraphSpec::default();
    g.set_cells(vec![vec![None, Some(2.0)], vec![Some(3.0), Some(4.0)]]);
    let (s, c, r) = super::parse_csv(&super::to_csv(&g));
    assert!(s.is_empty() && c.is_empty());
    assert_eq!(r, g.cells());
    // A label with spaces around a number stays a label, spaces and all.
    let mut g = GraphSpec { categories: vec!["2024 ".into()], ..GraphSpec::default() };
    g.set_cells(vec![vec![Some(1.0)]]);
    assert_eq!(super::parse_csv(&super::to_csv(&g)).1, vec!["2024 "]);
}

#[test]
fn blanks_read_back_as_null_and_year_header_rows_still_read_as_headers() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let r = s.execute("graph.create", &json!({"type": "line", "x": 0, "y": 0, "width": 300, "height": 200, "rows": [[1, null], [2, 3]]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    assert_eq!(s.execute("graph.setData", &json!({"id": id.0})).unwrap()["rows"], json!([[1.0, null], [2.0, 3.0]]));
    // A label followed by empty cells is a header with no series names, as before blanks existed.
    let (s, c, r) = super::parse_csv("Year,,\n2023,1,2");
    assert!(s.is_empty());
    assert_eq!(c, Vec::<String>::new());
    assert_eq!(r, vec![vec![Some(2023.0), Some(1.0), Some(2.0)]]);
}

#[test]
fn a_huge_ragged_grid_is_capped() {
    let g = GraphSpec { rows: vec![vec![0.0; vectorcraft_doc::MAX_GRAPH_SERIES + 10]; 3], ..GraphSpec::default() };
    assert!(g.cells().iter().all(|r| r.len() == vectorcraft_doc::MAX_GRAPH_SERIES));
}

#[test]
fn a_label_only_row_stays_with_blank_values() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let r = s.execute("graph.create", &json!({"type": "column", "x": 0, "y": 0, "width": 300, "height": 200, "csv": ",a\nQ1,1\nQ2\nQ3,3"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let spec = s.doc().unwrap().doc.node(id).unwrap().extras().graph.clone().unwrap();
    assert_eq!(spec.categories, vec!["Q1", "Q2", "Q3"]);
    assert_eq!(spec.cells(), vec![vec![Some(1.0)], vec![None], vec![Some(3.0)]]);
}

#[test]
fn blank_cells_leave_out_their_columns_and_break_lines() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let rows = json!([[3, 1], [null, 2], [5, null], [4, 4]]);
    let r = s
        .execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 400, "height": 200, "series": ["a", "b"], "rows": rows}))
        .unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let count = |s: &Session, index: u32| series(&s.doc().unwrap().doc.node(id).unwrap().clone(), index).children().unwrap().len();
    // Three columns each (one blank), plus the legend swatch.
    assert_eq!((count(&s, 0), count(&s, 1)), (4, 4));
    s.execute("graph.setType", &json!({"type": "stackedColumn"})).unwrap();
    assert_eq!((count(&s, 0), count(&s, 1)), (4, 4));
    // Line: series a is 3, blank, 5, 4. The lone first point has nothing to connect to, so one line (5 to 4), three
    // markers and the legend swatch.
    s.execute("graph.setType", &json!({"type": "line"})).unwrap();
    assert_eq!(count(&s, 0), 1 + 3 + 1);
    // Series b is 1, 2, blank, 4: one line (1 to 2), three markers and the swatch.
    assert_eq!(count(&s, 1), 1 + 3 + 1);
    // A data window round trip keeps the blanks.
    let csv = s.execute("graph.setData", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    assert_eq!(csv.lines().nth(2), Some(",2"), "{csv}");
    let spec = s.doc().unwrap().doc.node(id).unwrap().extras().graph.clone().unwrap();
    assert_eq!(spec.cells()[1], vec![None, Some(2.0)]);
    // In the file a blank is a 0 placeholder plus its place in `blanks`, so a version without blanks reads
    // numbers (as zeros); a graph without blanks writes no `blanks` at all.
    let v = serde_json::to_value(&spec).unwrap();
    assert_eq!(v["rows"][1], json!([0.0, 2.0]));
    assert_eq!(v["blanks"], json!([[1, 0], [2, 1]]));
    assert!(serde_json::to_value(GraphSpec::default()).unwrap().get("blanks").is_none());
}

fn spec(s: &Session, id: NodeId) -> GraphSpec {
    s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_deref().unwrap().clone()
}

/// Bounds of a series' marks, without its legend swatch (the last child).
fn marks(s: &Session, id: NodeId, index: u32) -> Vec<vectorcraft_geom::Rect> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let ch = series(&n, index).children().unwrap().to_vec();
    ch[..ch.len() - 1].iter().map(|c| c.geometric_bounds().unwrap()).collect()
}

#[test]
fn a_series_given_its_own_type_is_drawn_as_that_type_in_front_of_the_columns() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let one_column = marks(&s, id, 0)[0].width();
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.kind, g.series_kind(0), g.series_kind(1)), (GraphKind::Column, GraphKind::Column, GraphKind::Line));
    // The columns left take the whole cluster; the line runs through the category centres, one marker a value.
    let cols = marks(&s, id, 0);
    assert_eq!(cols.len(), 3);
    assert!((cols[0].width() - 2.0 * one_column).abs() < 1e-6, "{} vs {one_column}", cols[0].width());
    let line = marks(&s, id, 1);
    assert_eq!(line.len(), 4, "a line and three markers");
    assert!((line[1].center().x - cols[0].center().x).abs() < 1e-6);
    // The line series is drawn after (in front of) the column series.
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let order: Vec<u32> = n.children().unwrap().iter().filter_map(|c| c.extras().series_index).collect();
    assert_eq!(order, [0, 1]);
    s.execute("graph.setType", &json!({"seriesIndexes": [0], "type": "line"})).unwrap();
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "column"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let order: Vec<u32> = n.children().unwrap().iter().filter_map(|c| c.extras().series_index).collect();
    assert_eq!(order, [1, 0]);
    // Back to the graph's own type: nothing is stored for that series.
    assert_eq!(spec(&s, id).series_kinds, [Some(GraphKind::Line)]);
    // One undo per change.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_kinds, [Some(GraphKind::Line), Some(GraphKind::Line)]);
}

#[test]
fn group_selecting_a_series_retypes_only_that_series() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let grp = series(&n, 1).id;
    s.execute("select.set", &json!({"ids": [grp.0]})).unwrap();
    // The dialog opens on the series' type, and its OK (every option sent back) keeps it.
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["type"], "column");
    s.execute("graph.setType", &json!({"type": "area"})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.kind, g.series_kind(1)), (GraphKind::Column, GraphKind::Area));
    let mut all = s.execute("graph.setType", &json!({})).unwrap();
    assert_eq!(all["type"], "area");
    all["ticks"] = json!(4);
    s.execute("graph.setType", &all).unwrap();
    assert_eq!(spec(&s, id).series_kind(1), GraphKind::Area);
    // The series stays selected through each change.
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [grp]);
    // A series and something outside it selected, or an explicit `id`: the whole graph.
    s.execute("select.set", &json!({"ids": [grp.0, id.0]})).unwrap();
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["type"], "column");
    s.execute("select.set", &json!({"ids": [grp.0]})).unwrap();
    assert_eq!(s.execute("graph.setType", &json!({"id": id.0})).unwrap()["type"], "column");
    // The whole graph selected: the graph's type changes, the series keeps its own.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setType", &json!({"type": "line"})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.kind, g.series_kind(0), g.series_kind(1)), (GraphKind::Line, GraphKind::Line, GraphKind::Area));
}

#[test]
fn types_that_do_not_share_a_value_axis_are_refused_or_ignored() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    for ty in ["pie", "radar", "scatter", "bar"] {
        assert!(s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": ty})).is_err(), "{ty}");
    }
    for bad in [json!([2]), json!([]), json!([-1]), json!(["x"]), json!([1e30]), json!({}), json!(1)] {
        assert!(s.execute("graph.setType", &json!({"seriesIndexes": bad, "type": "line"})).is_err(), "{bad}");
    }
    assert!(s.execute("graph.setType", &json!({"seriesIndexes": [1]})).is_err(), "no type");
    assert!(spec(&s, id).series_kinds.is_empty());
    // A series type the graph's new type can't take is drawn as the graph's type, and kept for when it can.
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
    s.execute("graph.setType", &json!({"type": "pie"})).unwrap();
    let g = spec(&s, id);
    assert_eq!(g.series_kind(1), GraphKind::Pie);
    s.execute("graph.setType", &json!({"type": "column"})).unwrap();
    assert_eq!(spec(&s, id).series_kind(1), GraphKind::Line);
}

#[test]
fn stacked_columns_take_one_slot_and_the_value_axis_covers_their_total() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",a,b,c\nQ1,10,20,30\nQ2,10,20,30";
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "stackedColumn", "x": 0, "y": 0, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("graph.setType", &json!({"seriesIndexes": [2], "type": "line"})).unwrap();
    let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
    assert!((a[0].x0 - b[0].x0).abs() < 1e-6 && (a[0].y0 - b[0].y1).abs() < 1e-6, "b sits on a");
    // a + b stack to 30, the line's highest value is 30: the axis ends at 30 and the stack reaches the top.
    assert!((b[0].y0 - 0.0).abs() < 1e-6, "{:?}", b[0]);
    // With a plain column too, the stack is the cluster's last slot, beside it.
    s.execute("graph.setType", &json!({"seriesIndexes": [0], "type": "column"})).unwrap();
    let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
    assert!((a[0].width() - b[0].width()).abs() < 1e-6 && b[0].x0 > a[0].x1, "{:?} {:?}", a[0], b[0]);
}

#[test]
fn areas_mixed_with_columns_put_their_points_at_the_category_centres() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("graph.setType", &json!({"seriesIndexes": [0], "type": "area"})).unwrap();
    let (area, cols) = (marks(&s, id, 0)[0], marks(&s, id, 1));
    assert!((area.x0 - cols[0].center().x).abs() < 1e-6 && (area.x1 - cols[2].center().x).abs() < 1e-6, "{area:?}");
    // The area is drawn behind the columns.
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let order: Vec<u32> = n.children().unwrap().iter().filter_map(|c| c.extras().series_index).collect();
    assert_eq!(order, [0, 1]);
    // An area graph alone still spans the plot.
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "area"})).unwrap();
    assert!((marks(&s, id, 0)[0].x0 - 100.0).abs() < 1e-6);
}

#[test]
fn bar_graphs_take_stacked_bar_series() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",a,b,c\nQ1,10,20,-5\nQ2,10,20,-5";
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "bar", "x": 0, "y": 0, "width": 300, "height": 200, "csv": csv})).unwrap()["id"].as_u64().unwrap(),
    );
    assert!(s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "column"})).is_err());
    s.execute("graph.setType", &json!({"seriesIndexes": [1, 2], "type": "stackedBar"})).unwrap();
    let (a, b, c) = (marks(&s, id, 0), marks(&s, id, 1), marks(&s, id, 2));
    // b and c stack along x in the cluster's second slot, below a's bar; c, negative, runs left of the zero line.
    assert!((a[0].height() - b[0].height()).abs() < 1e-6 && b[0].y0 > a[0].y1, "{:?} {:?}", a[0], b[0]);
    assert!((b[0].y0 - c[0].y0).abs() < 1e-6 && (c[0].x1 - b[0].x0).abs() < 1e-6, "{:?} {:?}", b[0], c[0]);
}

#[test]
fn series_types_survive_save_and_open_and_older_files_have_none() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
    let path = vectorcraft_testkit::temp_dir("graph-kinds").join("kinds.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!(spec(&s, id).series_kinds, [None, Some(GraphKind::Line)]);
    let g: GraphSpec = serde_json::from_value(json!({"kind": "line", "rows": [[1.0, 2.0]]})).unwrap();
    assert!(g.series_kinds.is_empty());
    assert!(serde_json::to_value(&g).unwrap().get("seriesKinds").is_none());
    // A stored type the graph can't take, or past its series, is drawn as the graph's type.
    let g: GraphSpec = serde_json::from_value(json!({"kind": "pie", "seriesKinds": ["line", null, "column"], "rows": [[1.0]]})).unwrap();
    assert_eq!((g.series_kind(0), g.series_kind(5)), (GraphKind::Pie, GraphKind::Pie));
}

/// The value axis labels: text and bounds of the texts in the Axes group that read as numbers.
fn value_labels(s: &Session, id: NodeId) -> Vec<(String, vectorcraft_geom::Rect)> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    group(&n, "Axes")
        .children()
        .unwrap()
        .iter()
        .filter_map(|c| match &c.kind {
            NodeKind::Text(t) => Some((t.plain_text(), c.geometric_bounds().unwrap())),
            _ => None,
        })
        .filter(|(t, _)| t.parse::<f64>().is_ok())
        .collect()
}

fn two_scale_graph(s: &mut Session) -> NodeId {
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",a,b\nQ1,10,1000\nQ2,5,500";
    NodeId(
        s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    )
}

#[test]
fn the_value_axis_goes_on_the_left_the_right_or_both_sides() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    let left: Vec<String> = value_labels(&s, id).into_iter().inspect(|(_, b)| assert!(b.x1 < 100.0, "{b:?}")).map(|(t, _)| t).collect();
    assert_eq!(left, ["0", "200", "400", "600", "800", "1000"]);
    s.execute("graph.setType", &json!({"valueAxis": "right"})).unwrap();
    let right = value_labels(&s, id);
    assert!(right.iter().all(|(_, b)| b.x0 > 400.0), "{right:?}");
    assert_eq!(right.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), left);
    // Both sides, one scale: the same labels on each side, and the legend moves past the right ones.
    s.execute("graph.setType", &json!({"valueAxis": "both"})).unwrap();
    let both = value_labels(&s, id);
    assert_eq!(both.len(), 12);
    let edge = both.iter().map(|(_, b)| b.x1).fold(0.0, f64::max);
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let swatch = series(&n, 0).children().unwrap().last().unwrap().geometric_bounds().unwrap();
    assert!(swatch.x0 > edge, "{swatch:?} vs {edge}");
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["valueAxis"], "both");
    assert!(s.execute("graph.setType", &json!({"valueAxis": "top"})).is_err());
}

#[test]
fn separate_scales_give_the_right_axis_series_a_scale_of_their_own() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
    // Assigned, but drawn on one scale until separate scales are on with both sides.
    assert_eq!(spec(&s, id).right_series, [1]);
    s.execute("graph.setType", &json!({"separateScales": true})).unwrap();
    assert!((marks(&s, id, 0)[0].height() - 2.0).abs() < 1e-6, "a's 10 of 1000");
    s.execute("graph.setType", &json!({"valueAxis": "both"})).unwrap();
    // a's 10 and b's 1000 both reach the top: 0..10 on the left, 0..1000 on the right.
    let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
    assert!((a[0].y0 - 100.0).abs() < 1e-6 && (b[0].y0 - 100.0).abs() < 1e-6, "{:?} {:?}", a[0], b[0]);
    let labels = value_labels(&s, id);
    let side = |right: bool| labels.iter().filter(|(_, b)| (b.x0 > 400.0) == right).map(|(t, _)| t.clone()).collect::<Vec<_>>();
    assert_eq!(side(false), ["0", "2", "4", "6", "8", "10"]);
    assert_eq!(side(true), ["0", "200", "400", "600", "800", "1000"]);
    // The right axis' own tick values.
    s.execute("graph.setType", &json!({"rightAxisMin": 0, "rightAxisMax": 2000, "rightTicks": 4})).unwrap();
    assert!((marks(&s, id, 1)[0].y0 - 200.0).abs() < 1e-6, "1000 of 2000");
    assert!((marks(&s, id, 0)[0].y0 - 100.0).abs() < 1e-6, "the left axis is unchanged");
    let v = s.execute("graph.setType", &json!({})).unwrap();
    assert_eq!((v["separateScales"].clone(), v["rightTicks"].clone(), v["rightAxisMax"].clone()), (json!(true), json!(4), json!(2000.0)));
    // Stacked series stack on their own axis, each axis' stack in a slot of its own.
    s.execute("graph.setType", &json!({"type": "stackedColumn"})).unwrap();
    let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
    assert!((a[0].y1 - 300.0).abs() < 1e-6 && (b[0].y1 - 300.0).abs() < 1e-6, "{:?} {:?}", a[0], b[0]);
    assert!(b[0].x0 >= a[0].x1 - 1e-6 && (a[0].width() - b[0].width()).abs() < 1e-6, "side by side: {:?} {:?}", a[0], b[0]);
    // A line on the right axis follows the right scale.
    s.execute("graph.setType", &json!({"type": "column"})).unwrap();
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
    let line = marks(&s, id, 1);
    assert!((line[1].center().y - 200.0).abs() < 1e-6 && (line[2].center().y - 250.0).abs() < 1e-6, "1000 and 500 of 2000: {line:?}");
}

#[test]
fn group_selected_series_go_on_the_chosen_axis() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
    s.execute("select.set", &json!({"ids": [grp.0]})).unwrap();
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["valueAxis"], "left");
    assert!(s.execute("graph.setType", &json!({"valueAxis": "both"})).is_err(), "a series takes one axis");
    s.execute("graph.setType", &json!({"valueAxis": "right"})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.right_series.clone(), g.value_axis), (vec![1], vectorcraft_doc::ValueAxisSide::Left));
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["valueAxis"], "right");
    s.execute("graph.setType", &json!({"valueAxis": "left"})).unwrap();
    assert!(spec(&s, id).right_series.is_empty());
}

#[test]
fn bar_graphs_keep_their_value_axis_along_the_bottom() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "bar");
    let before = (value_labels(&s, id), marks(&s, id, 0));
    s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true, "seriesIndexes": [1]})).unwrap_err();
    s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true})).unwrap();
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
    assert_eq!((value_labels(&s, id), marks(&s, id, 0)), before);
}

#[test]
fn value_axis_options_survive_save_and_open_and_are_left_out_when_unset() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true, "rightAxisMin": 0, "rightAxisMax": 2000})).unwrap();
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
    let path = vectorcraft_testkit::temp_dir("graph-axes").join("axes.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let g = spec(&s, id);
    assert_eq!(
        (g.value_axis, g.separate_scales, g.right_series, g.right_axis_max),
        (vectorcraft_doc::ValueAxisSide::Both, true, vec![1], Some(2000.0))
    );
    let v = serde_json::to_value(GraphSpec::default()).unwrap();
    for k in ["valueAxis", "separateScales", "rightSeries", "rightTicks", "rightAxisMin", "rightAxisMax"] {
        assert!(v.get(k).is_none(), "{k}");
    }
}

#[test]
fn the_dialog_round_trip_keeps_series_on_both_axes_where_they_are() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",a,b,c\nQ1,10,1000,3\nQ2,5,500,4";
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
    for side in ["both", "left"] {
        s.execute("graph.setType", &json!({"valueAxis": side, "separateScales": true})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        s.execute("select.set", &json!({"ids": [series(&n, 0).id.0, series(&n, 1).id.0]})).unwrap();
        let fields = s.execute("graph.setType", &json!({})).unwrap();
        assert!(fields["valueAxis"].is_null(), "{side}: {fields}");
        s.execute("graph.setType", &fields).unwrap();
        let g = spec(&s, id);
        assert_eq!((g.right_series, g.value_axis.id()), (vec![1], side), "{side}");
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    }
}

#[test]
fn an_axis_with_no_series_shows_the_other_scale() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true})).unwrap();
    let labels = |s: &Session| {
        let l = value_labels(s, id);
        let side = |right: bool| l.iter().filter(|(_, b)| (b.x0 > 400.0) == right).map(|(t, _)| t.clone()).collect::<Vec<_>>();
        (side(false), side(true))
    };
    let (left, right) = labels(&s);
    assert_eq!(left, right);
    assert_eq!(right.last().unwrap(), "1000");
    // Every series on the right: the left axis shows the right scale.
    s.execute("graph.setType", &json!({"seriesIndexes": [0, 1], "valueAxis": "right", "rightAxisMin": 0, "rightAxisMax": 2000})).unwrap();
    let (left, right) = labels(&s);
    assert_eq!(left, right);
    assert_eq!(right.last().unwrap(), "2000");
}

#[test]
fn right_axis_columns_start_at_the_right_scales_zero() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",a,b\nQ1,10,-500\nQ2,5,500";
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true, "seriesIndexes": [1]})).unwrap_err();
    s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true})).unwrap();
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
    // Right scale -600..600 (zero at y 200), left 0..10 (zero at y 300): b's columns meet at the right zero.
    let b = marks(&s, id, 1);
    assert!((b[0].y0 - 200.0).abs() < 1e-6 && (b[1].y1 - 200.0).abs() < 1e-6, "{b:?}");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let at_200 = group(&n, "Axes")
        .children()
        .unwrap()
        .iter()
        .filter(|c| matches!(c.kind, NodeKind::Path { .. }))
        .filter_map(|c| c.geometric_bounds())
        .any(|bb| bb.height() < 1e-6 && (bb.y0 - 200.0).abs() < 1e-6 && (bb.width() - 300.0).abs() < 1e-6);
    assert!(at_200, "a zero line for the right scale");
}

#[test]
fn data_without_a_right_axis_series_drops_its_assignment() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
    s.execute("graph.setData", &json!({"csv": ",a\nQ1,10\nQ2,5"})).unwrap();
    assert!(spec(&s, id).right_series.is_empty());
    s.execute("graph.setData", &json!({"csv": ",a,b\nQ1,10,1000\nQ2,5,500"})).unwrap();
    assert!(spec(&s, id).right_series.is_empty(), "a new series starts on the left");
}

/// Bounds of the straight lines in the Axes group.
fn axis_lines(s: &Session, id: NodeId) -> Vec<vectorcraft_geom::Rect> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    group(&n, "Axes").children().unwrap().iter().filter(|c| matches!(c.kind, NodeKind::Path { .. })).filter_map(|c| c.geometric_bounds()).collect()
}

fn axis_texts(s: &Session, id: NodeId) -> Vec<String> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    group(&n, "Axes")
        .children()
        .unwrap()
        .iter()
        .filter_map(|c| match &c.kind {
            NodeKind::Text(t) => Some(t.plain_text()),
            _ => None,
        })
        .collect()
}

#[test]
fn value_axis_tick_marks_take_a_length_and_a_count_per_division() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    let short = |s: &Session, x0: f64| axis_lines(s, id).iter().filter(|b| (b.x0 - x0).abs() < 1e-6 && (b.width() - 4.0).abs() < 1e-6).count();
    let across = |s: &Session| axis_lines(s, id).iter().filter(|b| b.height() < 1e-6 && (b.width() - 300.0).abs() < 1e-6).count();
    // Short (the default): one at each of the six labels; the zero line runs across.
    assert_eq!((short(&s, 96.0), across(&s)), (6, 1));
    s.execute("graph.setType", &json!({"tickLength": "full"})).unwrap();
    // Across the plot at 200…1000; the one at 0 would be the zero line again.
    assert_eq!((short(&s, 96.0), across(&s)), (0, 6));
    s.execute("graph.setType", &json!({"tickLength": "none"})).unwrap();
    assert_eq!((short(&s, 96.0), across(&s)), (0, 1));
    // Two per division: one more halfway between labels, none past the last label.
    s.execute("graph.setType", &json!({"tickLength": "short", "tickMarks": 2})).unwrap();
    assert_eq!(short(&s, 96.0), 11);
    assert!(axis_lines(&s, id).iter().any(|b| (b.x0 - 96.0).abs() < 1e-6 && (b.y0 - 280.0).abs() < 1e-6), "100 of 0..1000 sits at y 280");
    assert_eq!(axis_texts(&s, id).iter().filter(|t| t.parse::<f64>().is_ok()).count(), 6, "labels only at divisions");
    // The right axis has its own.
    s.execute("graph.setType", &json!({"valueAxis": "both", "rightTickLength": "none"})).unwrap();
    assert_eq!((short(&s, 96.0), short(&s, 400.0)), (11, 0));
    s.execute("graph.setType", &json!({"rightTickLength": "short", "rightTickMarks": 4})).unwrap();
    assert_eq!(short(&s, 400.0), 21);
    let v = s.execute("graph.setType", &json!({})).unwrap();
    assert_eq!((v["tickLength"].clone(), v["tickMarks"].clone(), v["rightTickMarks"].clone()), (json!("short"), json!(2), json!(4)));
    assert!(s.execute("graph.setType", &json!({"tickLength": "long"})).is_err());
    s.execute("graph.setType", &json!({"tickMarks": 1_000_000})).unwrap();
    assert_eq!(spec(&s, id).tick_marks, 20);
}

#[test]
fn category_tick_marks_go_at_or_between_the_labels() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    let below = |s: &Session| {
        let mut xs: Vec<f64> = axis_lines(s, id)
            .iter()
            .filter(|b| b.width() < 1e-6 && (b.y0 - 300.0).abs() < 1e-6 && (b.height() - 4.0).abs() < 1e-6)
            .map(|b| b.x0)
            .collect();
        xs.sort_by(f64::total_cmp);
        xs
    };
    // None by default, as before.
    assert!(below(&s).is_empty());
    s.execute("graph.setType", &json!({"categoryTickLength": "short"})).unwrap();
    assert_eq!(below(&s), [175.0, 325.0]);
    s.execute("graph.setType", &json!({"ticksBetweenLabels": true})).unwrap();
    assert_eq!(below(&s), [100.0, 250.0, 400.0]);
    s.execute("graph.setType", &json!({"categoryTickMarks": 2})).unwrap();
    assert_eq!(below(&s), [100.0, 175.0, 250.0, 325.0, 400.0]);
    s.execute("graph.setType", &json!({"categoryTickLength": "full"})).unwrap();
    let full = axis_lines(&s, id).iter().filter(|b| b.width() < 1e-6 && (b.height() - 200.0).abs() < 1e-6 && b.x0 > 100.0 + 1e-6).count();
    assert_eq!(full, 4, "inside the plot: 175, 250, 325 and 400");
    // Bar graphs: the category axis is the left one.
    let bar = graph(&mut s, "bar");
    s.execute("graph.setType", &json!({"id": bar.0, "categoryTickLength": "short", "ticksBetweenLabels": true})).unwrap();
    let left = axis_lines(&s, bar).iter().filter(|b| b.height() < 1e-6 && (b.x0 - 96.0).abs() < 1e-6 && (b.width() - 4.0).abs() < 1e-6).count();
    assert_eq!(left, 4, "three categories, four edges");
}

#[test]
fn value_axis_numbers_take_a_prefix_and_a_suffix() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    s.execute("graph.setType", &json!({"prefix": "$", "suffix": " M", "valueAxis": "both", "separateScales": true})).unwrap();
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right", "rightSuffix": "%"})).unwrap();
    let texts = axis_texts(&s, id);
    assert!(texts.contains(&"$10 M".to_string()) && texts.contains(&"$0 M".to_string()), "{texts:?}");
    assert!(texts.contains(&"1000%".to_string()) && !texts.contains(&"$1000 M".to_string()), "{texts:?}");
    // One short line: control characters dropped, the length capped.
    s.execute("graph.setType", &json!({"prefix": format!("a\nb{}", "x".repeat(500))})).unwrap();
    let p = spec(&s, id).prefix;
    assert!(p.starts_with("ab") && p.chars().count() == 64, "{p}");
    let v = s.execute("graph.setType", &json!({})).unwrap();
    assert_eq!((v["suffix"].clone(), v["rightSuffix"].clone()), (json!(" M"), json!("%")));
}

#[test]
fn tick_and_label_options_survive_save_and_open_and_are_left_out_when_unset() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    s.execute(
        "graph.setType",
        &json!({"tickLength": "full", "tickMarks": 3, "categoryTickLength": "short", "ticksBetweenLabels": true, "suffix": "%"}),
    )
    .unwrap();
    let path = vectorcraft_testkit::temp_dir("graph-ticks").join("ticks.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let g = spec(&s, id);
    assert_eq!(
        (g.tick_length, g.tick_marks, g.category_tick_length, g.ticks_between_labels, g.suffix.as_str()),
        (vectorcraft_doc::TickLength::Full, 3, vectorcraft_doc::TickLength::Short, true, "%")
    );
    let v = serde_json::to_value(GraphSpec::default()).unwrap();
    for k in [
        "tickLength",
        "tickMarks",
        "rightTickLength",
        "rightTickMarks",
        "categoryTickLength",
        "categoryTickMarks",
        "ticksBetweenLabels",
        "prefix",
        "suffix",
        "rightPrefix",
        "rightSuffix",
    ] {
        assert!(v.get(k).is_none(), "{k}");
    }
    // A file's huge values draw within the caps.
    let g: GraphSpec = serde_json::from_value(json!({"rows": [[1.0]], "tickMarks": 1_000_000_000u64, "prefix": "y".repeat(10_000)})).unwrap();
    let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
    let art = super::generate(&mut d, &g);
    let axes = art.iter().find(|n| n.name.as_deref() == Some("Axes")).unwrap().children().unwrap().to_vec();
    assert!(axes.len() < 200, "{} axis parts", axes.len());
    for t in axes.iter().filter_map(|n| match &n.kind {
        NodeKind::Text(t) => Some(t.plain_text()),
        _ => None,
    }) {
        assert!(t.chars().count() < 80, "{} characters", t.chars().count());
    }
}

#[test]
fn between_labels_ticks_on_an_edge_to_edge_graph_fall_halfway_between_its_labels() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "area");
    s.execute("graph.setType", &json!({"categoryTickLength": "short", "ticksBetweenLabels": true})).unwrap();
    // Labels at 100, 250 and 400 (the plot's edges and middle): ticks at 175 and 325.
    let mut xs: Vec<f64> = axis_lines(&s, id)
        .iter()
        .filter(|b| b.width() < 1e-6 && (b.y0 - 300.0).abs() < 1e-6 && (b.height() - 4.0).abs() < 1e-6)
        .map(|b| b.x0)
        .collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs, [175.0, 325.0]);
}

#[test]
fn bar_graph_value_ticks_run_up_the_plot_and_long_labels_push_the_legend() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "bar");
    let legend_x = |s: &Session| {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        series(&n, 0).children().unwrap().last().unwrap().geometric_bounds().unwrap().x0
    };
    let before = legend_x(&s);
    assert!((before - 414.0).abs() < 1e-6, "a short last label leaves the legend where it was: {before}");
    s.execute("graph.setType", &json!({"tickLength": "full", "tickMarks": 2})).unwrap();
    let up: Vec<_> = axis_lines(&s, id).into_iter().filter(|b| b.width() < 1e-6 && (b.height() - 200.0).abs() < 1e-6).collect();
    // -2..6 in 2s: five labels and four halves; the one at 0 is the zero line already, the left end the axis.
    assert_eq!(up.len(), 9, "{up:?}");
    s.execute("graph.setType", &json!({"suffix": " million tonnes"})).unwrap();
    let last = value_label_edge(&s, id);
    assert!(legend_x(&s) >= last + 6.0 - 1e-6, "{} vs {last}", legend_x(&s));
}

/// The right end of the Axes group's texts.
fn value_label_edge(s: &Session, id: NodeId) -> f64 {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    group(&n, "Axes")
        .children()
        .unwrap()
        .iter()
        .filter(|c| matches!(c.kind, NodeKind::Text(_)))
        .filter_map(|c| c.geometric_bounds())
        .map(|b| b.x1)
        .fold(f64::MIN, f64::max)
}

#[test]
fn counts_come_back_from_the_dialog_as_floats_and_are_capped() {
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    s.execute("graph.setType", &json!({"tickMarks": 3.0, "ticks": 4.0, "categoryTickMarks": 2.6})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.tick_marks, g.ticks, g.category_tick_marks), (3, 4, 3));
    s.execute("graph.setType", &json!({"categoryTickMarks": 0, "rightTickMarks": 1e300, "rightTicks": -2.0})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.category_tick_marks, g.right_tick_marks, g.right_ticks), (1, 20, 0));
    // A file's values come back as drawn.
    let mut fields = s.execute("graph.setType", &json!({})).unwrap();
    assert_eq!(fields["rightTickMarks"], 20);
    fields["tickLength"] = json!("full");
    s.execute("graph.setType", &fields).unwrap();
    assert_eq!(spec(&s, id).tick_length, vectorcraft_doc::TickLength::Full);
}

#[test]
fn a_long_category_axis_is_ticked_all_the_way_within_the_cap() {
    let rows: Vec<Vec<f64>> = (0..5000).map(|i| vec![i as f64]).collect();
    let g = GraphSpec { rows, category_tick_length: vectorcraft_doc::TickLength::Short, category_tick_marks: 20, ..GraphSpec::default() };
    let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
    let art = super::generate(&mut d, &g);
    let axes = art.iter().find(|n| n.name.as_deref() == Some("Axes")).unwrap();
    let ticks: Vec<f64> = axes
        .children()
        .unwrap()
        .iter()
        .filter_map(|c| c.geometric_bounds())
        .filter(|b| b.width() < 1e-9 && (b.height() - 4.0).abs() < 1e-9)
        .map(|b| b.x0)
        .collect();
    assert!(ticks.len() <= 10_000 && ticks.len() >= 5000, "{}", ticks.len());
    assert!(ticks.iter().copied().fold(f64::MIN, f64::max) > 199.0, "the last category is ticked");
}

/// A graph design: a 10 × 10 rectangle at the back and a 20-wide ellipse over it, saved as `name`.
fn save_design(s: &mut Session, name: &str) {
    let r = s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let e = s.execute("shape.ellipse", &json!({"x": 595, "y": 600, "width": 20, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [r, e]})).unwrap();
    s.execute("graph.design", &json!({"new": name})).unwrap();
}

/// The marks of series `index` (legend swatch last) and their bounds.
fn series_parts(s: &Session, id: NodeId, index: u32) -> Vec<vectorcraft_doc::Node> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    series(&n, index).children().unwrap().iter().map(|c| (**c).clone()).collect()
}

#[test]
fn a_marker_design_draws_each_data_point_sized_by_its_backmost_object() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Pill");
    assert_eq!(s.execute("graph.design", &json!({})).unwrap()["designs"], json!(["Pill"]));
    let id = graph(&mut s, "line");
    let before = series_parts(&s, id, 1);
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
    s.execute("select.set", &json!({"ids": [grp.0]})).unwrap();
    s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
    let after = series_parts(&s, id, 1);
    assert_eq!(after.len(), before.len(), "a line, three markers and the swatch");
    for (old, new) in before.iter().zip(&after).skip(1) {
        let (o, n) = (old.geometric_bounds().unwrap(), new.geometric_bounds().unwrap());
        assert!(matches!(new.kind, NodeKind::Group { .. }), "the design's art");
        // The 10-point rectangle fills the marker's square, the 20-wide ellipse twice as wide.
        assert!((n.center() - o.center()).hypot() < 1e-6, "{o:?} {n:?}");
        assert!((n.width() - 2.0 * o.width()).abs() < 1e-6 && (n.height() - o.height()).abs() < 1e-6, "{o:?} {n:?}");
    }
    // The series stays selected; the other series keeps its squares; the query names the design.
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [grp]);
    assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    assert_eq!(s.execute("graph.marker", &json!({})).unwrap()["design"], "Pill");
    assert_eq!(spec(&s, id).series_markers, [None, Some("Pill".to_string())]);
    // Back to the default square.
    s.execute("graph.marker", &json!({"design": null})).unwrap();
    assert!(spec(&s, id).series_markers.is_empty());
    assert!(series_parts(&s, id, 1).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    assert!(s.execute("graph.marker", &json!({"design": "Nope"})).is_err());
    assert!(s.execute("graph.marker", &json!({"design": 3})).is_err());
}

#[test]
fn marker_designs_apply_to_markers_not_to_columns() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Pill");
    let id = graph(&mut s, "column");
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
    // Every series (none picked): the line series' markers and swatch; the columns and their swatch stay.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
    assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    let line = series_parts(&s, id, 1);
    assert!(line.iter().skip(1).all(|n| matches!(n.kind, NodeKind::Group { .. })), "markers and swatch");
    let swatch = line.last().unwrap().geometric_bounds().unwrap();
    assert!((swatch.width() - 16.0).abs() < 1e-6 && (swatch.height() - 8.0).abs() < 1e-6, "{swatch:?}");
    // Scatter and radar graphs draw them too.
    for ty in ["scatter", "radar"] {
        let g = graph(&mut s, ty);
        s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
        s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
        assert!(series_parts(&s, g, 0).iter().any(|n| matches!(n.kind, NodeKind::Group { .. })), "{ty}");
    }
}

#[test]
fn graph_designs_are_saved_pasted_and_deleted() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    assert!(s.execute("graph.design", &json!({"new": "Pill"})).is_err(), "nothing selected");
    save_design(&mut s, "Pill");
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    assert!(s.execute("graph.design", &json!({"new": "Pill"})).is_err(), "the name is taken");
    assert!(s.execute("graph.design", &json!({"new": "  "})).is_err(), "no name");
    // Paste Design: a copy of the art, selected, with ids of its own.
    let pasted = NodeId(s.execute("graph.design", &json!({"paste": "Pill"})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [pasted]);
    let art = s.doc().unwrap().doc.graph_designs[0].art.clone();
    let copy = s.doc().unwrap().doc.node(pasted).unwrap().clone();
    assert_ne!(copy.id, art.id);
    assert_eq!(copy.geometric_bounds(), art.geometric_bounds());
    assert!(s.execute("graph.design", &json!({"paste": "Nope"})).is_err());
    // Deleting a design draws its graphs' default markers again; undo brings both back.
    let id = graph(&mut s, "line");
    s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
    s.execute("graph.design", &json!({"delete": "Pill"})).unwrap();
    assert!(s.doc().unwrap().doc.graph_designs.is_empty());
    assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(series_parts(&s, id, 0).iter().skip(1).all(|n| matches!(n.kind, NodeKind::Group { .. })));
    assert!(s.execute("graph.design", &json!({"delete": "Nope"})).is_err());
}

#[test]
fn designs_and_markers_survive_save_and_open() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Pill");
    let id = graph(&mut s, "line");
    s.execute("graph.marker", &json!({"seriesIndexes": [0], "design": "Pill"})).unwrap();
    let path = vectorcraft_testkit::temp_dir("graph-designs").join("designs.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!(s.execute("graph.design", &json!({})).unwrap()["designs"], json!(["Pill"]));
    assert_eq!(spec(&s, id).series_markers, [Some("Pill".to_string())]);
    assert!(serde_json::to_value(GraphSpec::default()).unwrap().get("seriesMarkers").is_none());
    // A design whose art has no area draws the default square.
    let g = GraphSpec { kind: GraphKind::Line, series_markers: vec![Some("Dot".into())], ..GraphSpec::default() };
    let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
    let dot = vectorcraft_doc::Node::group(d.alloc_id(), vec![]);
    d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Dot".into(), art: std::sync::Arc::new(dot) });
    let art = super::generate(&mut d, &g);
    let s0 = art.iter().find(|n| n.extras().series_index == Some(0)).unwrap();
    assert!(s0.children().unwrap().iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
}

#[test]
fn a_graph_saved_as_a_design_is_plain_art_even_as_its_own_marker() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Pill");
    let id = graph(&mut s, "line");
    s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.design", &json!({"new": "Chart"})).unwrap();
    let art = s.doc().unwrap().doc.graph_designs[1].art.clone();
    let mut graphs = 0;
    art.walk(&mut |n| graphs += usize::from(n.extras().graph.is_some() || n.extras().series_index.is_some()));
    assert_eq!(graphs, 0, "no graph inside a design");
    // The graph drawn with itself as a marker, then the design it used deleted: still one graph to edit.
    s.execute("graph.marker", &json!({"id": id.0, "seriesIndexes": [1], "design": "Chart"})).unwrap();
    s.execute("graph.design", &json!({"delete": "Pill"})).unwrap();
    let mut found = 0;
    s.doc().unwrap().doc.walk(|n| found += usize::from(n.extras().graph.is_some()));
    assert_eq!(found, 1);
    // A deleted design reads as none in the query, so the dialog opens on the default marker.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.marker", &json!({"seriesIndexes": [0], "design": null})).unwrap();
    let v = s.execute("graph.marker", &json!({"seriesIndexes": [1, 0]})).unwrap();
    assert!(v["design"].is_null(), "{v}");
}

#[test]
fn designs_too_big_for_a_graph_are_refused_and_drawn_as_squares() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // 1,500 objects at each of 200 points is past the budget.
    let ids: Vec<u64> = (0..1500)
        .map(|i| s.execute("shape.rectangle", &json!({"x": i % 40, "y": i / 40, "width": 1, "height": 1})).unwrap()["id"].as_u64().unwrap())
        .collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    s.execute("graph.design", &json!({"new": "Big"})).unwrap();
    let rows: Vec<Vec<f64>> = (0..200).map(|i| vec![i as f64]).collect();
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "line", "x": 0, "y": 0, "width": 300, "height": 200, "rows": rows})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    assert!(s.execute("graph.marker", &json!({"id": id.0, "design": "Big"})).is_err());
    assert!(spec(&s, id).series_markers.is_empty());
    // A file asking for it anyway draws the default squares.
    let mut g = spec(&s, id);
    g.series_markers = vec![Some("Big".into())];
    let mut d = (*s.doc().unwrap().doc).clone();
    let art = super::generate(&mut d, &g);
    let s0 = art.iter().find(|n| n.extras().series_index == Some(0)).unwrap();
    assert!(s0.children().unwrap().iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    // Too many objects for one design.
    let ids: Vec<u64> = (0..2001)
        .map(|i| s.execute("shape.rectangle", &json!({"x": i % 40, "y": i / 40, "width": 1, "height": 1})).unwrap()["id"].as_u64().unwrap())
        .collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    assert!(s.execute("graph.design", &json!({"new": "Huge"})).is_err());
}

#[test]
fn a_files_designs_are_tidied() {
    let mut d = vectorcraft_doc::Document::new(100.0, 100.0);
    let art = |d: &mut vectorcraft_doc::Document| std::sync::Arc::new(vectorcraft_doc::Node::group(d.alloc_id(), vec![]));
    let mut graphy = vectorcraft_doc::Node::group(d.alloc_id(), vec![]);
    graphy.extras_mut().graph = Some(Box::new(GraphSpec::default()));
    for name in ["  A\u{7}  ", "A", "", "   ", &"x".repeat(100)] {
        let a = art(&mut d);
        d.graph_designs.push(vectorcraft_doc::GraphDesign { name: name.into(), art: a });
    }
    d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "G".into(), art: std::sync::Arc::new(graphy) });
    d.tidy_graph_designs();
    let names: Vec<_> = d.graph_designs.iter().map(|x| x.name.clone()).collect();
    assert_eq!(names, ["A".to_string(), "x".repeat(64), "G".to_string()]);
    assert!(d.graph_designs[2].art.extras().graph.is_none());
}

#[test]
fn painting_a_series_drawn_with_a_design_leaves_the_design_alone() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Pill");
    let id = graph(&mut s, "line");
    s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 0).id;
    s.execute("paint.setFill", &json!({"ids": [grp.0], "color": "#f00"})).unwrap();
    // The design's paths keep the design's paint, before and after the graph is drawn again.
    let design_fill = |s: &Session| {
        let parts = series_parts(s, id, 0);
        let mut fills = vec![];
        parts[1].walk(&mut |n| {
            if let NodeKind::Path { .. } = n.kind {
                fills.push(n.appearance.fill_paint().color().map(|c| c.to_hex()));
            }
        });
        fills
    };
    let painted = design_fill(&s);
    assert!(!painted.contains(&Some("#ff0000".into())), "{painted:?}");
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    assert_eq!(design_fill(&s), painted);
    // A design of one path too.
    let r = s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"ids": [r], "color": "#00f"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("graph.design", &json!({"new": "Square"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.marker", &json!({"design": "Square"})).unwrap();
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 0).id;
    s.execute("paint.setFill", &json!({"ids": [grp.0], "color": "#f00"})).unwrap();
    assert_eq!(design_fill(&s), [Some("#0000ff".to_string())]);
}

#[test]
fn graph_design_params_are_names() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    for p in [json!({"new": 5}), json!({"paste": null}), json!({"delete": ["a"]})] {
        assert!(s.execute("graph.design", &p).is_err(), "{p}");
    }
}

#[test]
fn document_wide_passes_see_design_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"ids": [r], "color": "#123456"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("graph.design", &json!({"new": "Blue"})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let mut seen = false;
    s.doc().unwrap().doc.visit_paints(&mut |p| seen |= p.color().is_some_and(|c| c.to_hex() == "#123456"));
    assert!(seen, "the colour is used by the design");
}

#[test]
fn new_design_rename_delete_and_select_unused() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    // New Design names it "New Design", numbered when that's taken.
    assert_eq!(s.execute("graph.design", &json!({"new": true})).unwrap()["name"], "New Design");
    assert_eq!(s.execute("graph.design", &json!({"new": true})).unwrap()["name"], "New Design 2");
    let id = graph(&mut s, "line");
    s.execute("graph.marker", &json!({"id": id.0, "design": "New Design"})).unwrap();
    let v = s.execute("graph.design", &json!({"selectUnused": true})).unwrap();
    assert_eq!(v["unused"], json!(["New Design 2"]));
    // Renaming a design the graph draws with: the graph follows, still drawing it.
    s.execute("graph.design", &json!({"rename": "New Design", "to": "Dot"})).unwrap();
    assert_eq!(spec(&s, id).series_markers, [Some("Dot".to_string()), Some("Dot".to_string())]);
    assert!(s.execute("graph.design", &json!({"rename": "Dot", "to": "New Design 2"})).is_err(), "taken");
    assert!(s.execute("graph.design", &json!({"rename": "Nope", "to": "X"})).is_err());
    // Delete several at once.
    s.execute("graph.design", &json!({"delete": ["Dot", "New Design 2"]})).unwrap();
    assert_eq!(s.execute("graph.design", &json!({})).unwrap()["designs"], json!([]));
    assert!(s.execute("graph.design", &json!({"delete": ["Nope"]})).is_err());
}

/// A column design: a `w` × `h` rectangle at the back, a `w`-wide ellipse over its top `top` points and a rectangle
/// over the rest, with a horizontal guide line between them when `guide`, saved as `name`.
fn save_column_design(s: &mut Session, name: &str, (w, h, top): (f64, f64, f64), guide: bool) {
    let shape = |s: &mut Session, kind: &str, x: f64, y: f64, w: f64, h: f64| {
        s.execute(kind, &json!({"x": 600.0 + x, "y": 600.0 + y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap()
    };
    let mut ids = vec![shape(s, "shape.rectangle", 0.0, 0.0, w, h), shape(s, "shape.ellipse", 0.0, 0.0, w, top)];
    ids.push(shape(s, "shape.rectangle", w / 4.0, top, w / 2.0, h - top));
    if guide {
        let line =
            s.execute("shape.line", &json!({"x1": 600, "y1": 600.0 + top, "x2": 600.0 + w, "y2": 600.0 + top})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [line]})).unwrap();
        s.execute("view.guides.make", &json!({})).unwrap();
        ids.push(line);
    }
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    s.execute("graph.design", &json!({"new": name})).unwrap();
}

/// A column graph of one series with `values`, its value axis 0 to 50 over 200 points (4 points a unit), the zero
/// line at y = 300.
fn one_series(s: &mut Session, ty: &str, values: &[f64]) -> NodeId {
    let rows: Vec<Vec<f64>> = values.iter().map(|v| vec![*v]).collect();
    let p = json!({"type": ty, "x": 100, "y": 100, "width": 300, "height": 200, "series": ["A"], "rows": rows});
    let id = NodeId(s.execute("graph.create", &p).unwrap()["id"].as_u64().unwrap());
    s.execute("graph.setType", &json!({"id": id.0, "axisMin": 0, "axisMax": 50})).unwrap();
    id
}

/// The bounds of the backmost object (the frame) of each copy of a design inside `n`, a column drawn with it.
fn frames(n: &vectorcraft_doc::Node) -> Vec<vectorcraft_geom::Rect> {
    n.children().unwrap().iter().filter_map(|c| c.children()?.first()?.geometric_bounds()).collect()
}

fn near(a: vectorcraft_geom::Rect, b: [f64; 4]) -> bool {
    [a.x0 - b[0], a.y0 - b[1], a.x1 - b[2], a.y1 - b[3]].iter().all(|d| d.abs() < 1e-6)
}

#[test]
fn a_repeating_column_design_stacks_a_copy_per_ten_and_chops_or_scales_the_last() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_column_design(&mut s, "Coin", (10.0, 10.0, 4.0), false);
    let id = one_series(&mut s, "column", &[35.0]);
    let plain = series_parts(&s, id, 0)[0].geometric_bounds().unwrap();
    assert!(near(plain, [plain.x0, 160.0, plain.x1, 300.0]), "{plain:?}");
    s.execute("graph.column", &json!({"id": id.0, "design": "Coin", "columnType": "repeating", "each": 10})).unwrap();
    // 35 at 10 a copy: three copies of 40 points up from the zero line, and a fourth cut off at the value.
    let column = series_parts(&s, id, 0)[0].clone();
    let parts = column.children().unwrap();
    assert_eq!(parts.len(), 4, "3.5 designs");
    let at = |y0: f64, y1: f64| [plain.x0, y0, plain.x1, y1];
    let copies = frames(&column);
    assert!(near(copies[0], at(260.0, 300.0)) && near(copies[1], at(220.0, 260.0)) && near(copies[2], at(180.0, 220.0)), "{copies:?}");
    let NodeKind::Group { clip: true, children } = &parts[3].kind else { panic!("the last copy is clipped: {:?}", parts[3].kind) };
    assert!(near(frames(&parts[3])[0], at(140.0, 180.0)), "a whole copy");
    let (clip, copy) = (children[0].geometric_bounds().unwrap(), vectorcraft_render::painted_bounds(&s.doc().unwrap().doc, &children[1]).unwrap());
    assert_eq!(clip.y0, 160.0, "cut off at the value");
    // Its sides and its foot, strokes included, are not cut.
    assert!(clip.x0 <= copy.x0 && clip.x1 >= copy.x1 && clip.y1 >= copy.y1 && copy.x0 < plain.x0, "{clip:?} {copy:?}");
    // Nor is the brush art of a brushed path inside a group of the design.
    let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
    let square = |x0: f64, x1: f64| vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x0, 0.0, x1, 10.0));
    let frame = vectorcraft_doc::Node::path(d.alloc_id(), square(0.0, 10.0), vectorcraft_doc::Appearance::basic(Paint::None, Paint::None, 0.0));
    let mut look = vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(vectorcraft_color::Color::BLACK), 1.0);
    for item in &mut look.items {
        if let vectorcraft_doc::AppearanceItem::Stroke(st) = item {
            st.brush = Some("Wide".into());
        }
    }
    let brushed = vectorcraft_doc::Node::path(d.alloc_id(), square(4.0, 6.0), look);
    let inner = vectorcraft_doc::Node::group(d.alloc_id(), vec![std::sync::Arc::new(brushed)]);
    let art = vectorcraft_doc::Node::group(d.alloc_id(), vec![std::sync::Arc::new(frame), std::sync::Arc::new(inner)]);
    d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Brushed".into(), art: std::sync::Arc::new(art) });
    let options = ColumnDesign { design: "Brushed".into(), column_type: ColumnType::Repeating, ..ColumnDesign::default() };
    let g =
        GraphSpec { rows: vec![vec![35.0]], axis_min: Some(0.0), axis_max: Some(50.0), series_columns: vec![Some(options)], ..GraphSpec::default() };
    let out = super::generate(&mut d, &g);
    let column = out.iter().find(|n| n.extras().series_index == Some(0)).unwrap().children().unwrap()[0].clone();
    let last = column.children().unwrap().last().unwrap().clone();
    let NodeKind::Group { clip: true, children } = &last.kind else { panic!("the last copy is clipped: {:?}", last.kind) };
    let clip = children[0].geometric_bounds().unwrap();
    let mut reach = None;
    children[1].walk(&mut |n| {
        if vectorcraft_brush::has_brush(n) {
            reach = vectorcraft_render::painted_bounds(&d, n);
        }
    });
    let reach = reach.unwrap();
    assert!(clip.x0 <= reach.x0 && clip.x1 >= reach.x1 && clip.y1 >= reach.y1, "{clip:?} {reach:?}");
    // Scale: the last copy half as tall instead.
    s.execute("graph.column", &json!({"id": id.0, "design": "Coin", "fractions": "scale"})).unwrap();
    let column = series_parts(&s, id, 0)[0].clone();
    let copies = frames(&column);
    assert_eq!(copies.len(), 4);
    assert!(near(copies[3], at(160.0, 180.0)), "{copies:?}");
    let c = &spec(&s, id).series_columns[0];
    assert_eq!((c.as_ref().map(|c| (c.column_type, c.each, c.fractions))), Some((ColumnType::Repeating, 10.0, Fractions::Scale)));
    // A bar graph stacks them along its bars, and a negative value stacks them the other way from the zero line.
    let bars = one_series(&mut s, "bar", &[-15.0]);
    s.execute("graph.setType", &json!({"id": bars.0, "axisMin": -50, "axisMax": 50})).unwrap();
    s.execute("graph.column", &json!({"id": bars.0, "design": "Coin", "columnType": "repeating", "each": 10, "fractions": "scale"})).unwrap();
    let bar = series_parts(&s, bars, 0)[0].clone();
    let copies = frames(&bar);
    // The value axis runs from -50 to 50 over 300 points: 3 points a unit, the zero line at x = 250.
    let (zero, unit) = (250.0, 3.0);
    assert!((copies[0].x1 - zero).abs() < 1e-6 && (copies[0].width() - 10.0 * unit).abs() < 1e-6, "{copies:?}");
    assert!((copies[1].x1 - copies[0].x0).abs() < 1e-6 && (copies[1].width() - 5.0 * unit).abs() < 1e-6, "{copies:?}");
}

#[test]
fn a_sliding_column_design_keeps_the_part_above_its_guide_and_stretches_the_rest() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // 10 wide and 20 tall, the guide 5 points from the top.
    save_column_design(&mut s, "Pencil", (10.0, 20.0, 5.0), true);
    let id = one_series(&mut s, "column", &[35.0, 20.0]);
    s.execute("graph.setType", &json!({"id": id.0, "columnWidth": 20})).unwrap();
    s.execute("graph.column", &json!({"id": id.0, "design": "Pencil", "columnType": "sliding"})).unwrap();
    let parts = series_parts(&s, id, 0);
    for (column, value_end) in parts.iter().zip([160.0, 220.0]) {
        // The columns are 24 points wide (2.4 times the design): two copies, each clipped at the guide, which lands
        // 12 points (2.4 times 5) below the value end.
        let halves = column.children().unwrap();
        assert_eq!(halves.len(), 2, "{halves:?}");
        let half = |i: usize| {
            let NodeKind::Group { clip: true, children } = &halves[i].kind else { panic!("a clip group: {:?}", halves[i].kind) };
            (children[0].geometric_bounds().unwrap(), children[1].children().unwrap().to_vec())
        };
        let ((above, kept), (below, stretched)) = (half(0), half(1));
        let at = value_end + 12.0;
        assert!((above.y1 - at).abs() < 1e-6 && (below.y0 - at).abs() < 1e-6, "{above:?} {below:?}");
        assert_eq!(kept.len(), 3, "the guide is not drawn");
        // Above the guide the design keeps its proportions: the 10 × 5 ellipse is 24 × 12 at the value end.
        let top = kept[1].geometric_bounds().unwrap();
        assert!((top.y0 - value_end).abs() < 1e-6 && (top.width() - 24.0).abs() < 1e-6 && (top.height() - 12.0).abs() < 1e-6, "{top:?}");
        // Below it the design is stretched down to the zero line.
        let (frame, body) = (stretched[0].geometric_bounds().unwrap(), stretched[2].geometric_bounds().unwrap());
        assert!((frame.width() - 24.0).abs() < 1e-6 && (frame.y1 - 300.0).abs() < 1e-6, "{frame:?}");
        assert!((body.y0 - at).abs() < 1e-6 && (body.y1 - 300.0).abs() < 1e-6, "{body:?}");
    }
    // Vertically Scaled stretches the whole design, the top part included.
    s.execute("graph.column", &json!({"id": id.0, "design": "Pencil", "columnType": "vertical"})).unwrap();
    let art = series_parts(&s, id, 0)[0].children().unwrap()[0].children().unwrap().to_vec();
    assert!((art[1].geometric_bounds().unwrap().height() - 140.0 / 4.0).abs() < 1e-6);
    // Without a guide a sliding design is scaled vertically too.
    save_column_design(&mut s, "Plain", (10.0, 20.0, 5.0), false);
    s.execute("graph.column", &json!({"id": id.0, "design": "Plain", "columnType": "sliding"})).unwrap();
    let art = series_parts(&s, id, 0)[0].children().unwrap()[0].children().unwrap().to_vec();
    assert!((art[1].geometric_bounds().unwrap().height() - 140.0 / 4.0).abs() < 1e-6);
    // Guides that are not a horizontal line, a 2 × 2 square and a vertical line 0.1 points long, made guides before
    // the line, are passed over.
    let id_of = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let mut ids = vec![
        id_of(s.execute("shape.rectangle", &json!({"x": 700, "y": 600, "width": 10, "height": 20})).unwrap()),
        id_of(s.execute("shape.ellipse", &json!({"x": 700, "y": 600, "width": 10, "height": 5})).unwrap()),
    ];
    let square = id_of(s.execute("shape.rectangle", &json!({"x": 704, "y": 611, "width": 2, "height": 2})).unwrap());
    let tick = id_of(s.execute("shape.line", &json!({"x1": 702, "y1": 615, "x2": 702, "y2": 615.1})).unwrap());
    let line = id_of(s.execute("shape.line", &json!({"x1": 700, "y1": 605, "x2": 710, "y2": 605})).unwrap());
    s.execute("select.set", &json!({"ids": [square, tick, line]})).unwrap();
    s.execute("view.guides.make", &json!({})).unwrap();
    ids.extend([square, tick, line]);
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    s.execute("graph.design", &json!({"new": "Boxed"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"id": id.0, "design": "Boxed", "columnType": "sliding"})).unwrap();
    let halves = series_parts(&s, id, 0)[0].children().unwrap().to_vec();
    let above = halves[0].children().unwrap()[0].geometric_bounds().unwrap();
    assert!((above.y1 - (160.0 + 12.0)).abs() < 1e-6, "cut at the line, not at the square or the short vertical line: {above:?}");
}

#[test]
fn uniformly_scaled_column_designs_keep_their_proportions_and_cast_no_shadow() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_column_design(&mut s, "Coin", (10.0, 10.0, 4.0), false);
    let id = one_series(&mut s, "column", &[35.0, 20.0]);
    s.execute("graph.setType", &json!({"id": id.0, "dropShadow": true})).unwrap();
    let plain = series_parts(&s, id, 0)[0].geometric_bounds().unwrap();
    s.execute("graph.column", &json!({"id": id.0, "design": "Coin", "columnType": "uniform"})).unwrap();
    for (column, length) in series_parts(&s, id, 0).iter().zip([140.0, 80.0]) {
        let frame = frames(column)[0];
        assert!((frame.width() - length).abs() < 1e-6 && (frame.height() - length).abs() < 1e-6, "{frame:?}");
        assert!((frame.center().x - column.geometric_bounds().unwrap().center().x).abs() < 1e-6);
    }
    assert!((frames(&series_parts(&s, id, 0)[0])[0].center().x - plain.center().x).abs() < 1e-6, "centered on the column");
    // A design twice as wide as it is tall reaches past the plot, and the legend starts right of it.
    save_column_design(&mut s, "Strip", (20.0, 10.0, 4.0), false);
    let wide = one_series(&mut s, "column", &[20.0, 35.0]);
    s.execute("graph.column", &json!({"id": wide.0, "design": "Strip", "columnType": "uniform"})).unwrap();
    let parts = series_parts(&s, wide, 0);
    let (reach, legend) = (frames(&parts[1])[0], parts.last().unwrap().geometric_bounds().unwrap());
    assert!(reach.x1 > 400.0 && legend.x0 > reach.x1, "{reach:?} {legend:?}");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Drop Shadow")), "no shadow for designed columns");
    // The legend swatch shows the design, its frame filling the 8-point square.
    let swatch = series_parts(&s, id, 0).last().unwrap().clone();
    let frame = swatch.children().unwrap()[0].geometric_bounds().unwrap();
    assert!((frame.width() - 8.0).abs() < 1e-6 && (frame.height() - 8.0).abs() < 1e-6, "{frame:?}");
    // Painting the series leaves the design's paint alone.
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 0).id;
    s.execute("paint.setFill", &json!({"ids": [grp.0], "color": "#f00"})).unwrap();
    let mut red = false;
    s.doc().unwrap().doc.node(id).unwrap().walk(&mut |n| red |= n.appearance.fill_paint().color().is_some_and(|c| c.to_hex() == "#ff0000"));
    assert!(!red);
}

#[test]
fn column_designs_survive_save_and_open_and_follow_their_design() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_column_design(&mut s, "Coin", (10.0, 10.0, 4.0), false);
    let id = graph(&mut s, "stackedColumn");
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
    // Every series takes the design; only the stacked column series draws it.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let p = json!({"id": id.0, "design": "Coin", "columnType": "repeating", "each": 2.5, "fractions": "scale"});
    s.execute("graph.column", &p).unwrap();
    assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Group { .. })), "columns and swatch");
    assert!(series_parts(&s, id, 1).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })), "the line series");
    let v = s.execute("graph.column", &json!({"id": id.0, "seriesIndexes": [0]})).unwrap();
    assert_eq!(v, json!({"designs": ["Coin"], "design": "Coin", "columnType": "repeating", "each": 2.5, "fractions": "scale"}));
    let path = vectorcraft_testkit::temp_dir("graph-column-designs").join("columns.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let options = ColumnDesign { design: "Coin".into(), column_type: ColumnType::Repeating, each: 2.5, fractions: Fractions::Scale };
    assert_eq!(spec(&s, id).series_columns, [Some(options.clone()), Some(options)]);
    assert!(serde_json::to_value(GraphSpec::default()).unwrap().get("seriesColumns").is_none());
    // Renamed, the graph draws it still; in use, it isn't unused; deleted, the columns are plain again.
    s.execute("graph.design", &json!({"rename": "Coin", "to": "Token"})).unwrap();
    assert!(spec(&s, id).series_columns.iter().flatten().all(|c| c.design == "Token"));
    assert_eq!(s.execute("graph.design", &json!({"selectUnused": true})).unwrap()["unused"], json!([]));
    s.execute("graph.design", &json!({"delete": "Token"})).unwrap();
    assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    assert!(s.execute("graph.column", &json!({"id": id.0})).unwrap()["design"].is_null());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Group { .. })));
    // Null goes back to plain columns.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"id": id.0, "design": null})).unwrap();
    assert!(spec(&s, id).series_columns.is_empty());
}

#[test]
fn graph_column_params_are_checked_and_designs_too_big_are_drawn_plain() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_column_design(&mut s, "Coin", (10.0, 10.0, 4.0), false);
    let id = one_series(&mut s, "column", &[35.0]);
    for p in [
        json!({"design": "Nope"}),
        json!({"design": 3}),
        json!({"design": "Coin", "columnType": "diagonal"}),
        json!({"design": "Coin", "each": 0}),
        json!({"design": "Coin", "each": -1}),
        json!({"design": "Coin", "each": "10"}),
        json!({"design": "Coin", "fractions": "round"}),
    ] {
        let mut p = p;
        p["id"] = json!(id.0);
        assert!(s.execute("graph.column", &p).is_err(), "{p}");
    }
    assert!(spec(&s, id).series_columns.is_empty());
    // 350,000 copies of a three-object design are past the budget: refused, and a file asking for it draws plain columns.
    let p = json!({"id": id.0, "design": "Coin", "columnType": "repeating", "each": 0.0001});
    assert!(s.execute("graph.column", &p).is_err());
    let mut g = spec(&s, id);
    let options = ColumnDesign { design: "Coin".into(), column_type: ColumnType::Repeating, each: 0.0001, fractions: Fractions::Chop };
    g.series_columns = vec![Some(options.clone())];
    let mut d = (*s.doc().unwrap().doc).clone();
    let art = super::generate(&mut d, &g);
    let s0 = art.iter().find(|n| n.extras().series_index == Some(0)).unwrap();
    assert!(s0.children().unwrap().iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    // Columns of 0 on a value axis from 10,000 to 10,001 run 2,000,000 points from the clamped zero line: a copy each
    // 0.1 is past the budget too.
    let far = one_series(&mut s, "column", &[0.0, 0.0]);
    s.execute("graph.setType", &json!({"id": far.0, "axisMin": 10_000, "axisMax": 10_001})).unwrap();
    assert!(s.execute("graph.column", &json!({"id": far.0, "design": "Coin", "columnType": "repeating", "each": 0.1})).is_err());
    assert!(spec(&s, far).series_columns.is_empty());
    // A file's unusable value per copy draws the design vertically scaled, and so does a copy longer than the canvas:
    // 35 at 1,000,000,000 a copy, 4,000,000,000 points a copy.
    for (each, fractions) in [(-1.0, Fractions::Chop), (1e9, Fractions::Scale), (1e9, Fractions::Chop)] {
        g.series_columns = vec![Some(ColumnDesign { each, fractions, ..options.clone() })];
        let art = super::generate(&mut d, &g);
        let s0 = art.iter().find(|n| n.extras().series_index == Some(0)).unwrap();
        let column = s0.children().unwrap()[0].clone();
        assert_eq!(frames(&column).len(), 1, "{each} {fractions:?}");
        assert!((frames(&column)[0].height() - 140.0).abs() < 1e-6, "{each} {fractions:?}");
        assert!(!matches!(column.children().unwrap()[0].kind, NodeKind::Group { clip: true, .. }), "{each} {fractions:?}: not clipped");
    }
}

/// A column design with totals: a 10 × 10 rectangle at the back, type "%31" set from 2 points above its top, and a
/// group of type "Votes" inside it and type "%02 and %00" set from 4 points below it, with a horizontal guide line 5
/// points from the top when `guide`, saved as `name`. Returns the points the two totals are set from.
fn save_totals_design(s: &mut Session, name: &str, guide: bool) -> [vectorcraft_geom::Point; 2] {
    let id_of = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let frame = id_of(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap());
    let above = id_of(s.execute("text.create", &json!({"x": 600, "y": 598, "text": "%31", "size": 4})).unwrap());
    let votes = id_of(s.execute("text.create", &json!({"x": 601, "y": 606, "text": "Votes", "size": 3})).unwrap());
    let below = id_of(s.execute("text.create", &json!({"x": 600, "y": 614, "text": "%02 and %00", "size": 4})).unwrap());
    let origin = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.xf * vectorcraft_geom::Point::ZERO,
        k => panic!("type: {k:?}"),
    };
    let points = [origin(s, above), origin(s, below)];
    s.execute("select.set", &json!({"ids": [votes, below]})).unwrap();
    let inner = id_of(s.execute("object.group", &json!({})).unwrap());
    let mut ids = vec![frame, above, inner];
    if guide {
        let line = id_of(s.execute("shape.line", &json!({"x1": 600, "y1": 605, "x2": 610, "y2": 605})).unwrap());
        s.execute("select.set", &json!({"ids": [line]})).unwrap();
        s.execute("view.guides.make", &json!({})).unwrap();
        ids.push(line);
    }
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    s.execute("graph.design", &json!({"new": name})).unwrap();
    points
}

/// The text of each piece of type inside `n`, in paint order.
fn texts_in(n: &vectorcraft_doc::Node) -> Vec<String> {
    let mut out = vec![];
    n.walk(&mut |c| {
        if let NodeKind::Text(t) = &c.kind {
            out.push(t.plain_text());
        }
    });
    out
}

/// The type among `nodes`, each as its text and the point it is set from; each must be type at its own size.
fn totals_of(nodes: &[std::sync::Arc<vectorcraft_doc::Node>]) -> Vec<(String, vectorcraft_geom::Point)> {
    nodes
        .iter()
        .map(|n| match &n.kind {
            NodeKind::Text(t) => {
                let [a, b, c, d, x, y] = t.xf.as_coeffs();
                assert_eq!([a, b, c, d], [1.0, 0.0, 0.0, 1.0], "a total keeps its size: {:?}", t.xf);
                (t.plain_text(), vectorcraft_geom::Point::new(x, y))
            }
            k => panic!("a total: {k:?}"),
        })
        .collect()
}

fn close(a: vectorcraft_geom::Point, b: vectorcraft_geom::Point) -> bool {
    (a - b).hypot() < 1e-6
}

#[test]
fn column_design_totals_show_each_columns_value_at_their_own_size() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let [above, below] = save_totals_design(&mut s, "Ballot", false);
    let id = one_series(&mut s, "column", &[35.0, 7.256]);
    s.execute("graph.column", &json!({"id": id.0, "design": "Ballot"})).unwrap();
    let check = |s: &Session, want: [[&str; 2]; 2]| {
        let parts = series_parts(s, id, 0);
        for (column, want) in parts.iter().zip(want) {
            let children = column.children().unwrap();
            // The design's copy keeps the type without a total, stretched with the design, and leaves the totals out.
            assert_eq!(texts_in(&children[0]), ["Votes"]);
            let frame = frames(column)[0];
            let (sx, sy) = (frame.width() / 10.0, frame.height() / 10.0);
            let at = |p: vectorcraft_geom::Point| vectorcraft_geom::Point::new(frame.x0 + (p.x - 600.0) * sx, frame.y0 + (p.y - 600.0) * sy);
            let totals = totals_of(&children[1..]);
            assert_eq!(totals.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), want);
            // Each set from where the design's copy puts the point it was set from.
            assert!(close(totals[0].1, at(above)) && close(totals[1].1, at(below)), "{totals:?} {frame:?}");
        }
        // The legend swatch keeps the type without a total and leaves the totals out.
        assert_eq!(texts_in(parts.last().unwrap()), ["Votes"]);
    };
    check(&s, [["035.0", "35.00 and 35"], ["007.3", "7.26 and 7"]]);
    // New data draws the new values.
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[20], [0.004]]})).unwrap();
    check(&s, [["020.0", "20.00 and 20"], ["000.0", "0.00 and 0"]]);
    // The design in the Graph Design list is left as it was.
    assert_eq!(texts_in(&s.doc().unwrap().doc.graph_designs[0].art), ["%31", "Votes", "%02 and %00"]);
    // A bar below zero shows its minus sign, its totals moved with the design along the bar.
    let bars = one_series(&mut s, "bar", &[-15.0]);
    s.execute("graph.setType", &json!({"id": bars.0, "axisMin": -50, "axisMax": 50})).unwrap();
    s.execute("graph.column", &json!({"id": bars.0, "design": "Ballot"})).unwrap();
    let bar = series_parts(&s, bars, 0)[0].clone();
    let frame = frames(&bar)[0];
    let (sx, sy) = (frame.width() / 10.0, frame.height() / 10.0);
    let at = |p: vectorcraft_geom::Point| vectorcraft_geom::Point::new(frame.x0 + (p.x - 600.0) * sx, frame.y0 + (p.y - 600.0) * sy);
    let totals = totals_of(&bar.children().unwrap()[1..]);
    assert_eq!(totals.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["-015.0", "-15.00 and -15"]);
    assert!((frame.x1 - 250.0).abs() < 1e-6 && close(totals[0].1, at(above)) && close(totals[1].1, at(below)), "{totals:?} {frame:?}");
    // A design that is a single piece of type with a total shows the value alone, and its legend swatch is the
    // default square.
    let one = s.execute("text.create", &json!({"x": 600, "y": 700, "text": "%00", "size": 4})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [one]})).unwrap();
    s.execute("graph.design", &json!({"new": "Count"})).unwrap();
    let count = one_series(&mut s, "column", &[35.0]);
    s.execute("graph.column", &json!({"id": count.0, "design": "Count"})).unwrap();
    let parts = series_parts(&s, count, 0);
    assert_eq!(texts_in(&parts[0]), ["35"]);
    assert_eq!(totals_of(&parts[0].children().unwrap()[1..]).len(), 1);
    assert!(matches!(parts.last().unwrap().kind, NodeKind::Path { .. }), "the default swatch: {:?}", parts.last().unwrap().kind);
    // Repeating, with a last copy to chop, it still shows the value.
    s.execute("graph.column", &json!({"id": count.0, "design": "Count", "columnType": "repeating", "each": 10})).unwrap();
    let column = series_parts(&s, count, 0)[0].clone();
    assert!(matches!(column.kind, NodeKind::Group { .. }) && texts_in(&column) == ["35"], "{:?}", column.kind);
}

#[test]
fn a_repeating_design_shows_its_totals_once_at_the_value_end_and_a_sliding_one_beside_its_part() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let [above, below] = save_totals_design(&mut s, "Ballot", true);
    let id = one_series(&mut s, "column", &[35.0]);
    let plain = series_parts(&s, id, 0)[0].geometric_bounds().unwrap();
    s.execute("graph.column", &json!({"id": id.0, "design": "Ballot", "columnType": "repeating", "each": 10})).unwrap();
    let column = series_parts(&s, id, 0)[0].clone();
    let children = column.children().unwrap();
    // Three whole copies and the one cut off at the value, none with a total; then the two totals, not cut.
    assert_eq!(children.len(), 6, "{children:?}");
    assert!(matches!(children[3].kind, NodeKind::Group { clip: true, .. }), "the last copy is clipped");
    assert!(children[..4].iter().all(|c| texts_in(c) == ["Votes"]));
    // Set as in a whole copy that ends at the value: 40 points tall from y 160, as wide as the column.
    let sx = plain.width() / 10.0;
    let at = |p: vectorcraft_geom::Point| vectorcraft_geom::Point::new(plain.x0 + (p.x - 600.0) * sx, 160.0 + (p.y - 600.0) * 4.0);
    let totals = totals_of(&children[4..]);
    assert_eq!(totals.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["035.0", "35.00 and 35"]);
    assert!(close(totals[0].1, at(above)) && close(totals[1].1, at(below)), "{totals:?}");
    // Sliding: the total above the guide moves with the part kept in proportion at the value end, the one below the
    // design with the part stretched down to the zero line.
    s.execute("graph.column", &json!({"id": id.0, "design": "Ballot", "columnType": "sliding"})).unwrap();
    let column = series_parts(&s, id, 0)[0].clone();
    let children = column.children().unwrap();
    assert_eq!(children.len(), 4, "two halves and two totals: {children:?}");
    let k = (140.0 - 5.0 * sx) / 5.0;
    let totals = totals_of(&children[2..]);
    let kept = vectorcraft_geom::Point::new(plain.x0 + (above.x - 600.0) * sx, 160.0 + (above.y - 600.0) * sx);
    let stretched = vectorcraft_geom::Point::new(plain.x0 + (below.x - 600.0) * sx, 300.0 + (below.y - 610.0) * k);
    assert!(close(totals[0].1, kept) && close(totals[1].1, stretched), "{totals:?} {kept:?} {stretched:?}");
    // A total's pattern fill moves with it.
    let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
    let square = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
    let frame = vectorcraft_doc::Node::path(d.alloc_id(), square, vectorcraft_doc::Appearance::basic(Paint::None, Paint::None, 0.0));
    let origin = vectorcraft_geom::Point::new(0.0, -2.0);
    let t = vectorcraft_doc::TextObject::point(origin, "%00", CharStyle::default());
    let mut total = vectorcraft_doc::Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    let dots = Paint::Pattern { pattern: "Dots".into(), xf: vectorcraft_geom::Affine::IDENTITY };
    total.appearance = vectorcraft_doc::Appearance::basic(dots, Paint::None, 0.0).into();
    let art = vectorcraft_doc::Node::group(d.alloc_id(), vec![std::sync::Arc::new(frame), std::sync::Arc::new(total)]);
    d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Dotted".into(), art: std::sync::Arc::new(art) });
    let options = ColumnDesign { design: "Dotted".into(), ..ColumnDesign::default() };
    let g =
        GraphSpec { rows: vec![vec![35.0]], axis_min: Some(0.0), axis_max: Some(50.0), series_columns: vec![Some(options)], ..GraphSpec::default() };
    let out = super::generate(&mut d, &g);
    let column = out.iter().find(|n| n.extras().series_index == Some(0)).unwrap().children().unwrap()[0].clone();
    let (text, at) = totals_of(&column.children().unwrap()[1..]).remove(0);
    assert_eq!(text, "35");
    let Paint::Pattern { xf, .. } = column.children().unwrap()[1].appearance.fill_paint() else { panic!("a pattern fill") };
    assert!((xf * origin - at).hypot() < 1e-6 && (at - origin).hypot() > 1.0, "{xf:?} {at:?}");
}

#[test]
fn totals_inside_a_group_with_a_look_of_its_own_show_the_value_where_they_are() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id_of = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let frame = id_of(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap());
    let total = id_of(s.execute("text.create", &json!({"x": 600, "y": 605, "text": "%00", "size": 4})).unwrap());
    s.execute("select.set", &json!({"ids": [total]})).unwrap();
    let faded = id_of(s.execute("object.group", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [faded]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.execute("select.set", &json!({"ids": [frame, faded]})).unwrap();
    s.execute("graph.design", &json!({"new": "Faded"})).unwrap();
    let id = one_series(&mut s, "column", &[35.0]);
    s.execute("graph.column", &json!({"id": id.0, "design": "Faded", "columnType": "repeating", "each": 10})).unwrap();
    // The total stays in its half-opaque group: each of the four copies shows the value there.
    let parts = series_parts(&s, id, 0);
    let copies = parts[0].children().unwrap();
    assert_eq!(copies.len(), 4, "{copies:?}");
    let mut faded_groups = 0;
    for c in copies {
        assert_eq!(texts_in(c), ["35"]);
        c.walk(&mut |n| faded_groups += usize::from(matches!(n.kind, NodeKind::Group { .. }) && (n.opacity - 0.5).abs() < 1e-6));
    }
    assert_eq!(faded_groups, 4);
    // The legend swatch shows the design without it.
    let swatch = parts.last().unwrap();
    assert!(matches!(swatch.kind, NodeKind::Group { .. }) && texts_in(swatch).is_empty(), "{:?}", swatch.kind);
    // Type with a total that clips other art clips it with the value in each copy, and the legend swatch leaves out
    // what it clips.
    let frame = id_of(s.execute("shape.rectangle", &json!({"x": 600, "y": 650, "width": 10, "height": 10})).unwrap());
    let inside = id_of(s.execute("shape.rectangle", &json!({"x": 600, "y": 650, "width": 10, "height": 10})).unwrap());
    let mask = id_of(s.execute("text.create", &json!({"x": 600, "y": 658, "text": "%00", "size": 8})).unwrap());
    s.execute("select.set", &json!({"ids": [inside, mask]})).unwrap();
    let clipped = id_of(s.execute("object.clippingMask.make", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [frame, clipped]})).unwrap();
    s.execute("graph.design", &json!({"new": "Masked"})).unwrap();
    let masked = one_series(&mut s, "column", &[35.0]);
    s.execute("graph.column", &json!({"id": masked.0, "design": "Masked"})).unwrap();
    let parts = series_parts(&s, masked, 0);
    let clip_groups = |n: &vectorcraft_doc::Node| {
        let mut out = vec![];
        n.walk(&mut |c| {
            if let NodeKind::Group { clip: true, children } = &c.kind {
                out.push(children.first().map(|m| texts_in(m)).unwrap_or_default());
            }
        });
        out
    };
    assert_eq!(clip_groups(&parts[0]), [["35"]], "the copy's clipping type shows the value");
    let swatch = parts.last().unwrap();
    let mut drawn = 0;
    swatch.walk(&mut |n| drawn += usize::from(!matches!(n.kind, NodeKind::Group { .. })));
    assert!(clip_groups(swatch).is_empty() && drawn == 1, "only the frame: {swatch:?}");
    // Totals that are a blend's key objects stay in the blend and show the value there, and the legend swatch leaves
    // them out.
    let frame = id_of(s.execute("shape.rectangle", &json!({"x": 600, "y": 680, "width": 10, "height": 10})).unwrap());
    let first = id_of(s.execute("text.create", &json!({"x": 600, "y": 685, "text": "%00", "size": 4})).unwrap());
    let second = id_of(s.execute("text.create", &json!({"x": 606, "y": 685, "text": "%00", "size": 4})).unwrap());
    s.execute("select.set", &json!({"ids": [first, second]})).unwrap();
    let blend = id_of(s.execute("object.blend.make", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [frame, blend]})).unwrap();
    s.execute("graph.design", &json!({"new": "Blended"})).unwrap();
    let blended = one_series(&mut s, "column", &[35.0]);
    s.execute("graph.column", &json!({"id": blended.0, "design": "Blended"})).unwrap();
    let parts = series_parts(&s, blended, 0);
    let mut blends = vec![];
    parts[0].walk(&mut |n| {
        if matches!(n.kind, NodeKind::Blend { .. }) {
            blends.push(texts_in(n));
        }
    });
    assert_eq!(blends, [["35", "35"]], "{:?}", parts[0]);
    assert!(texts_in(parts.last().unwrap()).is_empty(), "{:?}", parts.last().unwrap());
    // A design that is that blend alone gets the default legend swatch.
    s.execute("select.set", &json!({"ids": [blend]})).unwrap();
    s.execute("graph.design", &json!({"new": "Keys"})).unwrap();
    let keys = one_series(&mut s, "column", &[35.0]);
    s.execute("graph.column", &json!({"id": keys.0, "design": "Keys"})).unwrap();
    let parts = series_parts(&s, keys, 0);
    assert_eq!(texts_in(&parts[0]), ["35", "35"]);
    assert!(matches!(parts.last().unwrap().kind, NodeKind::Path { .. }), "the default swatch: {:?}", parts.last().unwrap().kind);
    // A design of a total inside two plain groups shows the value, and its legend swatch is the default square.
    let total = id_of(s.execute("text.create", &json!({"x": 700, "y": 700, "text": "%00", "size": 4})).unwrap());
    s.execute("select.set", &json!({"ids": [total]})).unwrap();
    let inner = id_of(s.execute("object.group", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [inner]})).unwrap();
    let outer = id_of(s.execute("object.group", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [outer]})).unwrap();
    s.execute("graph.design", &json!({"new": "Nested"})).unwrap();
    let nested = one_series(&mut s, "column", &[35.0]);
    s.execute("graph.column", &json!({"id": nested.0, "design": "Nested"})).unwrap();
    let parts = series_parts(&s, nested, 0);
    assert_eq!(texts_in(&parts[0]), ["35"]);
    assert!(matches!(parts.last().unwrap().kind, NodeKind::Path { .. }), "the default swatch: {:?}", parts.last().unwrap().kind);
}

#[test]
fn totals_take_their_places_from_the_two_digits_after_the_percent_sign() {
    for (v, before, after, want) in [
        (122.0, 3, 0, "122"),
        (122.0, 0, 0, "122"),
        (5.0, 3, 1, "005.0"),
        (1234.0, 2, 0, "1234"),
        (2.5, 0, 0, "3"),
        (-2.5, 0, 0, "-3"),
        (0.25, 0, 0, "0"),
        (0.25, 0, 2, "0.25"),
        (1.005, 0, 2, "1.01"),
        (0.1 + 0.2, 0, 2, "0.30"),
        (9.995, 0, 2, "10.00"),
        (99.96, 0, 1, "100.0"),
        (-0.004, 0, 2, "0.00"),
        (-7.25, 2, 1, "-07.3"),
        (7.0, 0, 9, "7.000000000"),
        (1e21, 0, 0, "1000000000000000000000"),
    ] {
        assert_eq!(super::total_text(v, before, after), want, "{v} %{before}{after}");
    }
    // Each `%` and two digits, left to right; any other `%` stays.
    let style = CharStyle::default;
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "%31 and %%12, %1x, 50%", style());
    assert!(super::fill_totals(&mut t, 7.25));
    assert_eq!(t.plain_text(), "007.3 and %7.25, %1x, 50%");
    // A total split between two runs takes the style of its `%`.
    let (small, big) = (CharStyle { size: 10.0, ..style() }, CharStyle { size: 20.0, ..style() });
    t.runs = vec![vectorcraft_doc::TextRun::new("a %3", small), vectorcraft_doc::TextRun::new("1 b", big)];
    assert!(super::fill_totals(&mut t, 7.25));
    assert_eq!(t.runs.iter().map(|r| (r.text.as_str(), r.style.size)).collect::<Vec<_>>(), [("a 007.3", 10.0), (" b", 20.0)]);
    // Type without a total is left as it is.
    let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "Votes: 50%", style());
    assert!(!super::fill_totals(&mut t, 7.25));
    assert_eq!(t.plain_text(), "Votes: 50%");
}

fn make(s: &mut Session, ty: &str, csv: &str) -> NodeId {
    NodeId(
        s.execute("graph.create", &json!({"type": ty, "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"].as_u64().unwrap(),
    )
}

/// Where each wedge of series `index` starts: its angle from 12 o'clock, clockwise, in degrees (the legend
/// swatch, last, left out).
fn wedge_starts(s: &Session, id: NodeId, index: u32) -> Vec<f64> {
    let parts = series_parts(s, id, index);
    parts[..parts.len() - 1]
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Path { path, .. } => {
                let bp = path.to_bezpath();
                let pts: Vec<_> = bp.elements().iter().filter_map(|e| e.end_point()).collect();
                let (c, p) = (pts.first()?, pts.get(1)?);
                let deg = (p.x - c.x).atan2(-(p.y - c.y)).to_degrees();
                Some(if deg < -1e-6 { deg + 360.0 } else { deg })
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_bar_in_a_label_breaks_the_line() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "column", ",Total|Sales|2023,b\nQ1|first,3,2\nQ2,5,4");
    let texts = axis_texts(&s, id);
    assert!(texts.contains(&"Q1\nfirst".to_string()), "{texts:?}");
    // The legend: a label of three lines, and the next row below all three.
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let legend: Vec<_> = group(&n, "Legend").children().unwrap().iter().map(|c| (**c).clone()).collect();
    let NodeKind::Text(t) = &legend[0].kind else { panic!() };
    assert_eq!(t.plain_text(), "Total\nSales\n2023");
    let (a, b) =
        (series_parts(&s, id, 0).last().unwrap().geometric_bounds().unwrap(), series_parts(&s, id, 1).last().unwrap().geometric_bounds().unwrap());
    assert!((b.y0 - a.y0 - 3.0 * 9.0 * 1.8).abs() < 1e-6, "{a:?} {b:?}");
}

#[test]
fn transpose_swaps_rows_and_columns_and_switch_xy_swaps_scatter_pairs() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "column", ",a,b\nQ1,1,2\nQ2,3,");
    let before = spec(&s, id);
    s.execute("graph.setData", &json!({"transpose": true})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.series.clone(), g.categories.clone()), (vec!["Q1".to_string(), "Q2".into()], vec!["a".to_string(), "b".into()]));
    assert_eq!(g.cells(), [[Some(1.0), Some(3.0)], [Some(2.0), None]]);
    s.execute("graph.setData", &json!({"transpose": true})).unwrap();
    assert_eq!(spec(&s, id).cells(), before.cells());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).cells(), g.cells());
    assert!(s.execute("graph.setData", &json!({"switchXY": true})).is_err(), "scatter graphs only");
    let sc = make(&mut s, "scatter", ",y1,x1,y2,x2\nP,1,10,2,20");
    s.execute("graph.setData", &json!({"switchXY": true})).unwrap();
    let g = spec(&s, sc);
    assert_eq!(g.cells(), [[Some(10.0), Some(1.0), Some(20.0), Some(2.0)]]);
    assert_eq!(g.series, ["x1", "y1", "x2", "y2"]);
}

#[test]
fn pie_wedges_sort_largest_first_in_each_pie_or_in_the_first_pies_order() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "pie", ",a,b,c\nP1,1,3,2\nP2,3,1,2");
    // None (data order): a starts at 12 o'clock in both pies.
    assert_eq!(wedge_starts(&s, id, 0).iter().map(|d| d.round()).collect::<Vec<_>>(), [0.0, 0.0]);
    s.execute("graph.setType", &json!({"pieSort": "all"})).unwrap();
    // P1: b (3) first; P2: a (3) first.
    let (a, b) = (wedge_starts(&s, id, 0), wedge_starts(&s, id, 1));
    assert!(b[0].abs() < 1e-6 && a[1].abs() < 1e-6, "{a:?} {b:?}");
    s.execute("graph.setType", &json!({"pieSort": "first"})).unwrap();
    // P1's order (b, c, a) in both: b first in P2 too.
    let b = wedge_starts(&s, id, 1);
    assert!(b.iter().all(|d| d.abs() < 1e-6), "{b:?}");
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["pieSort"], "first");
    assert!(s.execute("graph.setType", &json!({"pieSort": "random"})).is_err());
}

#[test]
fn pies_sized_by_their_totals_side_by_side_or_stacked() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "pie", ",a,b\nSmall,5,5\nBig,20,20");
    let pie = |s: &Session, c: usize| {
        let wedges = |i: u32| {
            let parts = series_parts(s, id, i);
            parts[..parts.len() - 1].to_vec()
        };
        let rects: Vec<_> = (0..2).flat_map(wedges).filter_map(|n| n.geometric_bounds()).collect();
        rects.into_iter().skip(c).step_by(2).reduce(|a, b| a.union(b)).unwrap()
    };
    let even = (pie(&s, 0), pie(&s, 1));
    assert!((even.0.width() - even.1.width()).abs() < 1e-6);
    s.execute("graph.setType", &json!({"piePosition": "ratio"})).unwrap();
    let (small, big) = (pie(&s, 0), pie(&s, 1));
    // Areas in proportion to the totals: 10 and 40, so radii 1 : 2.
    assert!((big.width() / small.width() - 2.0).abs() < 1e-6, "{small:?} {big:?}");
    assert!((big.width() - even.1.width()).abs() < 1e-6, "the biggest keeps the full size");
    s.execute("graph.setType", &json!({"piePosition": "stacked"})).unwrap();
    // Drawn largest first, so the big pie's wedges come first in each series.
    let (big, small) = (pie(&s, 0), pie(&s, 1));
    assert!((small.center() - big.center()).hypot() < 1e-6 && (big.width() / small.width() - 2.0).abs() < 1e-6);
    // The big pie is drawn first, under the small one.
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let wedges: Vec<f64> = series(&n, 0).children().unwrap().iter().filter_map(|c| c.geometric_bounds()).map(|b| b.width()).collect();
    assert!(wedges[0] > wedges[1], "{wedges:?}");
}

#[test]
fn legends_in_wedges_put_the_series_labels_inside_the_pie() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "pie", ",North,South|East\nP,1,1");
    s.execute("graph.setType", &json!({"pieLegend": "wedges"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let pie =
        series(&n, 0).children().unwrap()[0].geometric_bounds().unwrap().union(series(&n, 1).children().unwrap()[0].geometric_bounds().unwrap());
    let labels: Vec<_> = group(&n, "Legend").children().unwrap().iter().map(|c| (**c).clone()).collect();
    assert_eq!(labels.len(), 2);
    for l in &labels {
        let b = l.geometric_bounds().unwrap();
        assert!(pie.contains(b.center()), "{b:?} inside {pie:?}");
    }
    // No swatches beside the pie: each series group holds only its wedge.
    assert_eq!((series(&n, 0).children().unwrap().len(), series(&n, 1).children().unwrap().len()), (1, 1));
    let v = s.execute("graph.setType", &json!({})).unwrap();
    assert_eq!((v["pieLegend"].clone(), v["piePosition"].clone()), (json!("wedges"), json!("even")));
    assert!(v.get("legend").is_none(), "a pie's Legend stands for the checkbox");
    // The labels are drawn over the wedges, and on the black first wedge in white.
    let order: Vec<_> = n.children().unwrap().iter().map(|c| c.name.clone().unwrap_or_default()).collect();
    assert_eq!(order.last().map(String::as_str), Some("Legend"), "{order:?}");
    let colour = |l: &vectorcraft_doc::Node| match &l.kind {
        NodeKind::Text(t) => t.runs[0].style.fill.color().unwrap().to_hex(),
        _ => panic!(),
    };
    assert_eq!((colour(&labels[0]), colour(&labels[1])), ("#ffffff".to_string(), "#000000".to_string()));
    // No Legend wins over a legend checkbox sent along (as the dialog does), and Standard comes back without wedges.
    s.execute("graph.setType", &json!({"pieLegend": "none", "legend": true})).unwrap();
    assert!(!spec(&s, id).legend);
    s.execute("graph.setType", &json!({"legend": true})).unwrap();
    assert!(!spec(&s, id).pie_legend_in_wedges);
    assert!(s.execute("graph.setType", &json!({"pieLegend": "inside"})).is_err());
    let path = vectorcraft_testkit::temp_dir("graph-pie").join("pie.vectorcraft");
    s.execute("graph.setType", &json!({"pieLegend": "wedges", "piePosition": "stacked", "pieSort": "all"})).unwrap();
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.pie_legend_in_wedges, g.pie_position, g.pie_sort), (true, vectorcraft_doc::PiePosition::Stacked, vectorcraft_doc::PieSort::All));
    let v = serde_json::to_value(GraphSpec::default()).unwrap();
    for k in ["pieLegendInWedges", "piePosition", "pieSort"] {
        assert!(v.get(k).is_none(), "{k}");
    }
}

#[test]
fn stacked_pies_are_rings_so_none_covers_another() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "pie", ",a,b\nSmall,5,5\nBig,20,20\nSame,20,20");
    s.execute("graph.setType", &json!({"piePosition": "stacked"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    // Series b's wedges: of the two equal big pies one is left out, the other a ring outside the small pie
    // (clear of the centre), and the small pie's whole.
    let b: Vec<_> = series(&n, 1).children().unwrap().iter().map(|c| (**c).clone()).collect();
    assert_eq!(b.len(), 3, "two wedges and the swatch");
    let small = b[1].geometric_bounds().unwrap();
    let NodeKind::Path { path, .. } = &b[0].kind else { panic!() };
    let c = small.center();
    let near: Vec<f64> = path.to_bezpath().elements().iter().filter_map(|e| e.end_point()).map(|p| (p - c).hypot()).collect();
    let min = near.iter().copied().fold(f64::MAX, f64::min);
    assert!(min > 1.0, "the ring keeps clear of the centre: {near:?}");
    // The names are drawn over the pies: after every series group.
    let kids = n.children().unwrap();
    let last_series = kids.iter().rposition(|c| c.extras().series_index.is_some()).unwrap();
    let names: Vec<usize> = kids.iter().enumerate().filter(|(_, c)| matches!(c.kind, NodeKind::Text(_))).map(|(i, _)| i).collect();
    assert_eq!(names.len(), 2, "of two equal pies, one has no ring and no name");
    assert!(names.iter().all(|i| *i > last_series), "{names:?} after {last_series}");
}

#[test]
fn transpose_keeps_within_the_series_cap_and_drops_stale_axis_series() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let rows: Vec<Vec<f64>> = (0..300).map(|i| vec![i as f64]).collect();
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "column", "x": 0, "y": 0, "width": 300, "height": 200, "rows": rows})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    assert!(s.execute("graph.setData", &json!({"transpose": true})).is_err(), "300 categories can't become series");
    assert_eq!(spec(&s, id).cells().len(), 300);
    let g = make(&mut s, "column", ",a,b,c\nQ1,1,2,3");
    s.execute("graph.setType", &json!({"seriesIndexes": [2], "valueAxis": "right"})).unwrap();
    s.execute("graph.setData", &json!({"transpose": true})).unwrap();
    assert!(spec(&s, g).right_series.is_empty(), "one series left");
    // An all-blank series becomes no category.
    let h = make(&mut s, "column", ",a,b\nQ1,1,\nQ2,2,");
    s.execute("graph.setData", &json!({"transpose": true})).unwrap();
    assert_eq!(spec(&s, h).categories, ["a", "b"], "b keeps its label");
    assert!(s.execute("graph.setData", &json!({"id": h.0, "transpose": false})).unwrap().get("csv").is_some(), "no change: the query");
}

#[test]
fn switch_xy_moves_the_labels_with_their_columns() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "scatter", ",y1,x1,y2\nP,1,10,2,20");
    s.execute("graph.setData", &json!({"switchXY": true})).unwrap();
    let g = spec(&s, id);
    assert_eq!(g.series, ["x1", "y1", "", "y2"]);
    assert_eq!(g.cells(), [[Some(10.0), Some(1.0), Some(20.0), Some(2.0)]]);
}

#[test]
fn a_label_of_only_bars_is_blank_and_non_pie_graphs_list_no_pie_options() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "column", ",||,b\nQ1,1,2");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert_eq!(group(&n, "Legend").children().unwrap().len(), 1, "only b");
    let v = s.execute("graph.setType", &json!({})).unwrap();
    assert!(v.get("pieLegend").is_none() && v.get("legend").is_some());
    // Bar and radar labels with a `|` centre their lines on the category.
    for ty in ["bar", "radar"] {
        let g = make(&mut s, ty, ",a\nOne|Two|Three,1\nQ2,2\nQ3,3");
        assert!(axis_texts(&s, g).contains(&"One\nTwo\nThree".to_string()), "{ty}");
    }
}

#[test]
fn drop_shadows_sit_behind_the_columns_lines_and_wedges() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let kids = n.children().unwrap();
    let at = kids.iter().position(|c| c.name.as_deref() == Some("Drop Shadow")).unwrap();
    assert!(kids.iter().position(|c| c.extras().series_index.is_some()).unwrap() > at, "behind the series");
    let shadow = &kids[at];
    assert!((shadow.opacity - 0.35).abs() < 1e-6);
    let copies: Vec<_> = shadow.children().unwrap().iter().filter_map(|c| c.geometric_bounds()).collect();
    let cols: Vec<_> = (0..2).flat_map(|i| marks(&s, id, i)).collect();
    assert_eq!(copies.len(), cols.len(), "one per column, none for the swatches");
    for c in &cols {
        assert!(copies.iter().any(|b| (b.x0 - c.x0 - 2.0).abs() < 1e-6 && (b.y0 - c.y0 - 2.0).abs() < 1e-6), "{c:?}");
    }
    let black = shadow.children().unwrap().iter().all(|c| c.appearance.fill_paint().color().map(|c| c.to_hex()) == Some("#000000".into()));
    assert!(black);
    // Lines cast one too (not their markers); pies one per wedge.
    let line = graph(&mut s, "line");
    s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
    let n = s.doc().unwrap().doc.node(line).unwrap().clone();
    assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), 2, "one per series line");
    let pie = graph(&mut s, "pie");
    s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
    let n = s.doc().unwrap().doc.node(pie).unwrap().clone();
    assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), 5, "the wedges with a value above 0");
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["dropShadow"], true);
    s.execute("graph.setType", &json!({"dropShadow": false})).unwrap();
    let n = s.doc().unwrap().doc.node(pie).unwrap().clone();
    assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Drop Shadow")));
}

#[test]
fn the_legend_across_top_runs_in_rows_above_the_plot() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("graph.setType", &json!({"legendAcrossTop": true})).unwrap();
    let swatch = |s: &Session, i: u32| series_parts(s, id, i).last().unwrap().geometric_bounds().unwrap();
    let (a, b) = (swatch(&s, 0), swatch(&s, 1));
    assert!(a.y1 <= 100.0 - 8.0 + 1e-6 && (a.y0 - b.y0).abs() < 1e-6 && b.x0 > a.x1 && (a.x0 - 100.0).abs() < 1e-6, "{a:?} {b:?}");
    assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["legendAcrossTop"], true);
    // Long labels wrap into rows within the plot's width, the last row just above it.
    let labels: Vec<String> = (0..6).map(|i| format!("A rather long series name {i}")).collect();
    let csv = format!(",{}\nQ1,1,2,3,4,5,6", labels.join(","));
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let ys: Vec<f64> = (0..6).map(|i| swatch(&s, i).y0).collect();
    assert!(ys.windows(2).any(|w| w[1] > w[0]), "a second row: {ys:?}");
    assert!(ys.iter().all(|y| *y + 8.0 <= 100.0 - 8.0 + 1e-6), "{ys:?}");
    let xs: Vec<f64> = (0..6).map(|i| swatch(&s, i).x1).collect();
    assert!(xs.iter().all(|x| *x <= 400.0), "{xs:?}");
    let path = vectorcraft_testkit::temp_dir("graph-top").join("top.vectorcraft");
    s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let g = spec(&s, id);
    assert!(g.legend_across_top && g.drop_shadow);
    let v = serde_json::to_value(GraphSpec::default()).unwrap();
    assert!(v.get("dropShadow").is_none() && v.get("legendAcrossTop").is_none());
}

#[test]
fn each_layer_keeps_its_shadows_just_behind_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "column", ",a,b,c\nQ1,3,2,1\nQ2,5,,2\nQ3,4,6,3");
    s.execute("graph.setType", &json!({"seriesIndexes": [0], "type": "area"})).unwrap();
    s.execute("graph.setType", &json!({"seriesIndexes": [2], "type": "line", "dropShadow": true})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let order: Vec<String> = n
        .children()
        .unwrap()
        .iter()
        .map(|c| c.extras().series_index.map_or_else(|| c.name.clone().unwrap_or_default(), |i| format!("s{i}")))
        .collect();
    // Areas cast none; the columns' shadows sit between the area and the columns, the line's between the columns
    // and the line.
    let at = |k: &str| order.iter().position(|o| o == k).unwrap();
    let shadows: Vec<usize> = order.iter().enumerate().filter(|(_, o)| *o == "Drop Shadow").map(|(i, _)| i).collect();
    assert_eq!(shadows.len(), 2, "{order:?}");
    assert!(at("s0") < shadows[0] && shadows[0] < at("s1") && at("s1") < shadows[1] && shadows[1] < at("s2"), "{order:?}");
    // b's blank leaves two columns; a line with a blank casts a shadow per run.
    let line = make(&mut s, "line", ",a\nQ1,1\nQ2,\nQ3,3\nQ4,4");
    s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
    let n = s.doc().unwrap().doc.node(line).unwrap().clone();
    assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), 1, "Q1 alone draws no line; Q3–Q4 one");
    // Scatter and radar lines cast one too; a stacked segment of 0 doesn't.
    for (ty, want) in [("scatter", 1), ("radar", 2)] {
        let g = graph(&mut s, ty);
        s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
        let n = s.doc().unwrap().doc.node(g).unwrap().clone();
        assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), want, "{ty}");
    }
    let st = make(&mut s, "stackedColumn", ",a,b\nQ1,0,2");
    s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
    let n = s.doc().unwrap().doc.node(st).unwrap().clone();
    assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), 1);
}

#[test]
fn the_legend_across_top_keeps_above_labels_over_the_plot() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "radar", ",a,b\nTop|of|chart,1,2\nQ2,2,3\nQ3,3,1");
    s.execute("graph.setType", &json!({"legendAcrossTop": true})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let axes_top = group(&n, "Axes").children().unwrap().iter().filter_map(|c| c.geometric_bounds()).map(|b| b.y0).fold(f64::MAX, f64::min);
    let legend_bottom = group(&n, "Legend").children().unwrap().iter().filter_map(|c| c.geometric_bounds()).map(|b| b.y1).fold(f64::MIN, f64::max);
    assert!(legend_bottom < axes_top, "{legend_bottom} above {axes_top}");
    // No legend: nothing across the top either.
    s.execute("graph.setType", &json!({"legend": false})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Legend")));
    // Labels of a row other than the last stay within the plot's width.
    let labels: Vec<String> = (0..6).map(|i| format!("A rather long series name {i}")).collect();
    let col = make(&mut s, "column", &format!(",{}\nQ1,1,2,3,4,5,6", labels.join(",")));
    s.execute("graph.setType", &json!({"legendAcrossTop": true})).unwrap();
    let n = s.doc().unwrap().doc.node(col).unwrap().clone();
    let texts: Vec<_> = group(&n, "Legend").children().unwrap().iter().filter_map(|c| c.geometric_bounds()).collect();
    let last_row = texts.iter().map(|b| b.y0).fold(f64::MIN, f64::max);
    assert!(texts.iter().filter(|b| b.y0 < last_row - 1e-6).all(|b| b.x1 <= 400.0 + 1e-6), "{texts:?}");
}

#[test]
fn widths_past_100_overlap_and_the_first_row_or_column_goes_in_front() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("graph.setType", &json!({"columnWidth": 250, "clusterWidth": 300})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.column_width, g.cluster_width), (250.0, 300.0));
    let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
    assert!(a[0].x1 > b[0].x0, "the columns of a cluster overlap: {:?} {:?}", a[0], b[0]);
    assert!(b[0].x1 > a[1].x0, "and so do the clusters: {:?} {:?}", b[0], a[1]);
    assert_eq!(s.execute("graph.setType", &json!({"clusterWidth": 5000})).map(|_| spec(&s, id).cluster_width).unwrap(), 1000.0);
    let order = |s: &Session| -> Vec<u32> {
        s.doc().unwrap().doc.node(id).unwrap().children().unwrap().iter().filter_map(|c| c.extras().series_index).collect()
    };
    assert_eq!(order(&s), [0, 1]);
    s.execute("graph.setType", &json!({"firstColumnInFront": true})).unwrap();
    assert_eq!(order(&s), [1, 0], "the first series drawn last, on top");
    // First Row in Front: each series' first category drawn last.
    let xs = |s: &Session| marks(s, id, 0).iter().map(|b| b.x0).collect::<Vec<_>>();
    let before = xs(&s);
    assert!(before.windows(2).all(|w| w[0] < w[1]));
    s.execute("graph.setType", &json!({"firstRowInFront": true})).unwrap();
    let after = xs(&s);
    assert!(after.windows(2).all(|w| w[0] > w[1]), "{after:?}");
    let v = s.execute("graph.setType", &json!({})).unwrap();
    assert_eq!((v["firstRowInFront"].clone(), v["firstColumnInFront"].clone()), (json!(true), json!(true)));
}

#[test]
fn filled_lines_are_bands_in_the_series_colour() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "line", ",a\nQ1,2\nQ2,2\nQ3,2");
    let thin = marks(&s, id, 0)[0];
    assert!(thin.height() < 1e-6, "a flat 1 pt line");
    s.execute("graph.setType", &json!({"filledLines": true})).unwrap();
    let parts = series_parts(&s, id, 0);
    let band = parts[0].geometric_bounds().unwrap();
    assert!((band.height() - 3.0).abs() < 1e-6 && (band.width() - thin.width()).abs() < 1e-6, "{band:?}");
    assert_eq!(parts[0].appearance.fill_paint().color().unwrap().to_hex(), "#000000", "the first series' colour");
    assert_eq!(parts[0].appearance.stroke_width(), 0.25);
    // A series colour set with Group Selection fills the band.
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 0).id;
    s.execute("paint.setFill", &json!({"ids": [grp.0], "color": "#ff0000"})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[2], [3], [2]]})).unwrap();
    assert_eq!(series_parts(&s, id, 0)[0].appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");
    let path = vectorcraft_testkit::temp_dir("graph-filled").join("filled.vectorcraft");
    s.execute("graph.setType", &json!({"firstRowInFront": true, "firstColumnInFront": true})).unwrap();
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let g = spec(&s, id);
    assert!(g.filled_lines && g.first_row_in_front && g.first_column_in_front);
    let v = serde_json::to_value(GraphSpec::default()).unwrap();
    for k in ["filledLines", "firstRowInFront", "firstColumnInFront"] {
        assert!(v.get(k).is_none(), "{k}");
    }
}

#[test]
fn filled_lines_keep_the_line_colour_and_fall_back_to_thin_lines() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "line", ",a\nQ1,2\nQ2,3\nQ3,2");
    // A colour given to the thin line (its stroke) fills the band.
    let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 0).id;
    s.execute("paint.setStroke", &json!({"ids": [grp.0], "color": "#00ff00"})).unwrap();
    s.execute("graph.setType", &json!({"id": id.0, "filledLines": true})).unwrap();
    let band = &series_parts(&s, id, 0)[0];
    assert_eq!(band.appearance.fill_paint().color().unwrap().to_hex(), "#00ff00");
    // A radar series of zeros has every point at the centre: no band to make, so a thin line.
    let radar = make(&mut s, "radar", ",a\nQ1,0\nQ2,0\nQ3,0");
    s.execute("graph.setType", &json!({"id": radar.0, "filledLines": true})).unwrap();
    let line = &series_parts(&s, radar, 0)[0];
    assert!(line.appearance.fill_paint().is_none(), "a stroked line");
    // Past the budget, thin lines.
    let rows: Vec<Vec<f64>> = (0..8_000).map(|i| vec![(i % 7) as f64, 1.0, 2.0]).collect();
    let big = NodeId(
        s.execute("graph.create", &json!({"type": "line", "x": 0, "y": 0, "width": 300, "height": 200, "rows": rows})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("graph.setType", &json!({"id": big.0, "filledLines": true, "markPoints": false})).unwrap();
    assert!(series_parts(&s, big, 0)[0].appearance.fill_paint().is_none());
}

#[test]
fn overlapping_columns_keep_clear_of_the_legend_and_options_follow_the_graph_type() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("graph.setType", &json!({"columnWidth": 1000, "clusterWidth": 1000})).unwrap();
    let reach = (0..2).flat_map(|i| marks(&s, id, i)).map(|b| b.x1).fold(f64::MIN, f64::max);
    assert!(reach > 400.0, "past the plot: {reach}");
    let swatch = series_parts(&s, id, 0).last().unwrap().geometric_bounds().unwrap();
    assert!(swatch.x0 >= reach + 14.0 - 1e-6, "{swatch:?} after {reach}");
    let v = s.execute("graph.setType", &json!({})).unwrap();
    assert!(v.get("firstRowInFront").is_some() && v.get("filledLines").is_none());
    let line = graph(&mut s, "line");
    let v = s.execute("graph.setType", &json!({"id": line.0})).unwrap();
    assert!(v.get("firstRowInFront").is_none() && v.get("filledLines").is_some());
}

/// The Axes and Legend texts with their size and fill, after the graph is drawn again.
fn label_looks(s: &mut Session, id: NodeId) -> Vec<(String, f64, String)> {
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let mut out = vec![];
    for g in ["Axes", "Legend"] {
        for c in group(&n, g).children().unwrap() {
            if let NodeKind::Text(t) = &c.kind {
                let st = &t.runs[0].style;
                out.push((t.plain_text(), st.size, st.fill.color().map(|c| c.to_hex()).unwrap_or_default()));
            }
        }
    }
    out
}

fn text_id(s: &Session, id: NodeId, grp: &str, text: &str) -> NodeId {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    group(&n, grp).children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == text)).unwrap().id
}

#[test]
fn formatting_the_whole_graphs_text_survives_it_being_drawn_again() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 12})).unwrap();
    let looks = label_looks(&mut s, id);
    assert!(looks.iter().all(|(_, size, _)| (*size - 12.0).abs() < 1e-6), "{looks:?}");
    let g = spec(&s, id);
    assert!(g.label_styles.values.is_some() && g.label_styles.categories.is_some() && g.label_styles.legend.is_some());
    // One undo takes the data change back, the next the formatting and the kept styles together.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(spec(&s, id).label_styles.values.is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).label_styles, vectorcraft_doc::LabelStyles::default());
}

#[test]
fn formatting_one_label_keeps_that_kind_of_label() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    // A legend label (Group Selection), recoloured with the Character panel's fill.
    let l = text_id(&s, id, "Legend", "2025");
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    s.execute("text.setStyle", &json!({"fill": "#ff0000"})).unwrap();
    // A category label, resized.
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("select.set", &json!({"ids": [c.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 14})).unwrap();
    // A value axis number, painted with the Appearance fill.
    let v = text_id(&s, id, "Axes", "4");
    s.execute("paint.setFill", &json!({"ids": [v.0], "color": "#0000ff"})).unwrap();
    let looks = label_looks(&mut s, id);
    let look = |t: &str| looks.iter().find(|(x, _, _)| x == t).cloned().unwrap();
    assert_eq!(look("2024"), ("2024".into(), 9.0, "#ff0000".into()), "every legend label");
    assert_eq!(look("Q1").1, 14.0, "every category label");
    assert_eq!(look("Q1").2, "#000000");
    assert_eq!(look("0").2, "#0000ff", "every value axis number");
    assert_eq!(look("0").1, 9.0);
    // Kept through save and open; nothing written when unset.
    let path = vectorcraft_testkit::temp_dir("graph-labels").join("labels.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!(spec(&s, id).label_styles.categories.as_ref().unwrap().size, 14.0);
    assert!(serde_json::to_value(GraphSpec::default()).unwrap().get("labelStyles").is_none());
}

#[test]
fn pie_names_and_wedge_labels_keep_their_style() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "pie", ",a,b\nP1,1,2");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let name = n.children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == "P1")).unwrap().id;
    s.execute("select.set", &json!({"ids": [name.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 16})).unwrap();
    assert_eq!(spec(&s, id).label_styles.categories.as_ref().unwrap().size, 16.0);
    s.execute("graph.setType", &json!({"id": id.0, "pieLegend": "wedges"})).unwrap();
    let w = text_id(&s, id, "Legend", "a");
    s.execute("select.set", &json!({"ids": [w.0]})).unwrap();
    s.execute("text.setStyle", &json!({"fill": "#00ff00"})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[3, 1]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let fills: Vec<String> = group(&n, "Legend")
        .children()
        .unwrap()
        .iter()
        .filter_map(|c| match &c.kind {
            NodeKind::Text(t) => t.runs[0].style.fill.color().map(|c| c.to_hex()),
            _ => None,
        })
        .collect();
    assert_eq!(fills, ["#00ff00", "#00ff00"], "the kept fill, also on the dark wedge");
}

#[test]
fn labels_are_told_apart_by_where_they_come_not_by_their_text() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // Categories named like the value axis numbers.
    let id = make(&mut s, "column", ",a\n2,1\n4,3\n6,5");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let texts: Vec<(NodeId, String)> = group(&n, "Axes")
        .children()
        .unwrap()
        .iter()
        .filter_map(|c| match &c.kind {
            NodeKind::Text(t) => Some((c.id, t.plain_text())),
            _ => None,
        })
        .collect();
    // The value axis "2" comes before the category "2".
    let value_two = texts.iter().find(|(_, t)| t == "2").unwrap().0;
    s.execute("select.set", &json!({"ids": [value_two.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 7})).unwrap();
    let g = spec(&s, id);
    assert!(g.label_styles.values.is_some() && g.label_styles.categories.is_none(), "{:?}", g.label_styles);
    // A scatter graph's numbers are all values.
    let sc = graph(&mut s, "scatter");
    s.execute("select.set", &json!({"ids": [sc.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 8})).unwrap();
    let g = spec(&s, sc);
    assert!(g.label_styles.values.is_some() && g.label_styles.categories.is_none());
    // Part of a label's text: nothing kept.
    let col = graph(&mut s, "column");
    let q = text_id(&s, col, "Axes", "Q2");
    s.execute("select.set", &json!({"ids": [q.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 20, "start": 0, "end": 1})).unwrap();
    assert!(spec(&s, col).label_styles.categories.is_none());
    // Only the change is kept: a size given to the whole graph leaves the label colours as they were.
    let pie = make(&mut s, "pie", ",a,b\nP,1,1");
    s.execute("graph.setType", &json!({"id": pie.0, "pieLegend": "wedges"})).unwrap();
    s.execute("select.set", &json!({"ids": [pie.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 11})).unwrap();
    let legend = spec(&s, pie).label_styles.legend.unwrap();
    assert_eq!((legend.size, legend.fill.color().unwrap().to_hex()), (11.0, "#000000".into()));
    s.execute("graph.setData", &json!({"id": pie.0, "rows": [[2, 1]]})).unwrap();
    let n = s.doc().unwrap().doc.node(pie).unwrap().clone();
    let fill = match &group(&n, "Legend").children().unwrap()[0].kind {
        NodeKind::Text(t) => t.runs[0].style.fill.color().unwrap().to_hex(),
        _ => panic!(),
    };
    assert_eq!(fill, "#ffffff", "still white on the black wedge");
}

#[test]
fn character_formats_are_kept_and_bigger_labels_space_out() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.setFormat", &json!({"underline": true, "baselineShift": 1})).unwrap();
    let g = spec(&s, id);
    let st = g.label_styles.legend.unwrap();
    assert!(st.underline && (st.baseline_shift - 1.0).abs() < 1e-9);
    // 20 pt legend labels: the rows move apart so they don't overlap.
    s.execute("text.setStyle", &json!({"size": 20})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",a,b,c\nQ1,1,2,3"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let mut rows: Vec<_> = group(&n, "Legend").children().unwrap().iter().filter_map(|c| c.geometric_bounds()).collect();
    rows.sort_by(|a, b| a.y0.total_cmp(&b.y0));
    assert!(rows.windows(2).all(|w| w[1].y0 >= w[0].y1 - 1e-6), "{rows:?}");
    // A file's absurd style is kept within the Character limits.
    let mut g = spec(&s, id);
    g.label_styles.values = Some(CharStyle { size: 1e300, tracking: f64::INFINITY, ..CharStyle::default() });
    let mut d = (*s.doc().unwrap().doc).clone();
    let art = super::generate(&mut d, &g);
    let axes = art.iter().find(|n| n.name.as_deref() == Some("Axes")).unwrap();
    for t in axes.children().unwrap() {
        if let NodeKind::Text(t) = &t.kind {
            assert!(t.runs[0].style.size <= 1296.0 && t.runs[0].style.tracking.is_finite());
        }
    }
}
