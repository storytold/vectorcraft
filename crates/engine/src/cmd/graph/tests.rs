use serde_json::json;
use vectorcraft_color::{BlendMode, Paint};
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

#[test]
fn fmt_value_tiny_negative_is_zero() {
    assert_eq!(super::fmt_value(-0.004), "0");
    assert_eq!(super::fmt_value(-0.0049), "0");
    assert_eq!(super::fmt_value(-0.006), "-0.01");
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

/// An opacity set on the fill or stroke item of one series mark is kept by the graph: every path of that series has
/// it after the graph is drawn again, the other series stay opaque, 100% forgets it, and undo takes it back.
#[test]
fn an_item_opacity_on_a_series_mark_survives_regeneration() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let opacities = |s: &Session, index: u32| -> Vec<(f32, f32)> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let marks = series(&n, index).children().unwrap().to_vec();
        marks.iter().map(|m| (m.appearance.fill().unwrap().opacity, m.appearance.stroke().unwrap().opacity)).collect()
    };
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.execute("transparency.set", &json!({"ids": [bar.0], "item": 0, "opacity": 40})).unwrap();
    s.execute("transparency.set", &json!({"ids": [bar.0], "item": 1, "opacity": 60})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.stroke_opacity), (Some(0.4), Some(0.6)));
    let restored: GraphSpec = serde_json::from_value(serde_json::to_value(spec(&s, id)).unwrap()).unwrap();
    assert_eq!(restored, spec(&s, id));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(opacities(&s, 0).iter().all(|o| *o == (0.4, 0.6)), "{:?}", opacities(&s, 0));
    assert!(opacities(&s, 1).iter().all(|o| *o == (1.0, 1.0)), "{:?}", opacities(&s, 1));
    // Back to 100%: the graph keeps no opacity for the fill.
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.execute("transparency.set", &json!({"ids": [bar.0], "item": 0, "opacity": 100})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, None);
    s.execute("graph.setType", &json!({"type": "bar"})).unwrap();
    assert!(opacities(&s, 0).iter().all(|o| *o == (1.0, 0.6)), "{:?}", opacities(&s, 0));
    // Undo goes back through the type change, the 100% and the data edit, then takes the stroke opacity away with its
    // edit.
    for _ in 0..4 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.stroke_opacity), (Some(0.4), None));
}

/// Clear Appearance and Reduce to Basic Appearance on a series mark take its fill and stroke opacity away, and the
/// graph keeps none, so the series stays opaque when it is drawn again.
#[test]
fn clearing_a_series_marks_appearance_forgets_its_kept_opacity() {
    for cmd in ["appearance.clear", "appearance.reduceToBasic"] {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
        s.execute("transparency.set", &json!({"ids": [bar.0], "item": 0, "opacity": 40})).unwrap();
        s.execute("transparency.set", &json!({"ids": [bar.0], "item": 1, "opacity": 60})).unwrap();
        s.execute(cmd, &json!({"ids": [bar.0]})).unwrap();
        let kept = spec(&s, id).series_paints[0].clone();
        assert_eq!((kept.fill_opacity, kept.stroke_opacity), (None, None), "{cmd}");
        s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        assert!(series(&n, 0).children().unwrap().iter().all(|m| m.appearance.fill().unwrap().opacity == 1.0), "{cmd}");
    }
}

/// Removing the fill whose opacity the graph keeps takes the kept opacity away; an edit that leaves a mark opaque
/// and Normal (an opacity of 100% alone) keeps nothing for its series.
#[test]
fn removing_a_series_marks_fill_forgets_its_kept_opacity() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.execute("transparency.set", &json!({"ids": [bar.0], "item": 0, "opacity": 100})).unwrap();
    assert!(spec(&s, id).series_paints.is_empty());
    s.execute("transparency.set", &json!({"ids": [bar.0], "item": 0, "opacity": 40})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, Some(0.4));
    s.execute("appearance.removeItem", &json!({"ids": [bar.0], "index": 0})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, None);
    // An opaque fill added over the kept one takes it away too.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("appearance.addFill", &json!({"ids": [bar.0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, None);
}

/// A row or a row opacity is not given to a graph design's copy inside a series, and the graph keeps no opacity for the
/// series: the graph draws the design again from the design itself.
#[test]
fn an_opacity_on_a_design_copy_in_a_series_is_not_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    save_design(&mut s, "Square");
    s.execute("graph.column", &json!({"id": id.0, "design": "Square"})).unwrap();
    let copy = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].clone();
    assert!(matches!(copy.kind, NodeKind::Group { .. }));
    assert!(s.execute("appearance.addFill", &json!({"ids": [copy.id.0]})).is_err());
    assert!(s.execute("transparency.set", &json!({"ids": [copy.id.0], "item": 0, "opacity": 40})).is_err());
    assert!(spec(&s, id).series_paints.first().is_none_or(|p| p.fill_opacity.is_none()));
}

/// A column graph whose series 0 draws the design "Square": the graph, series 0's first copy of the design, a path
/// inside that copy, and the first mark of series 1, drawn plain.
fn design_art() -> (Session, NodeId, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    save_design(&mut s, "Square");
    s.execute("graph.column", &json!({"id": id.0, "seriesIndexes": [0], "design": "Square"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let copy = series(&n, 0).children().unwrap()[0].clone();
    assert!(matches!(copy.kind, NodeKind::Group { .. }));
    let mut art = vec![];
    copy.walk(&mut |m| {
        if matches!(m.kind, NodeKind::Path { .. }) {
            art.push(m.id);
        }
    });
    let mark = series(&n, 1).children().unwrap()[0].clone();
    assert!(matches!(mark.kind, NodeKind::Path { .. }));
    (s, id, copy.id, art[0], mark.id)
}

/// `cmd` with `p` is refused as an edit of graph design art, and the document keeps its undo history.
fn refused_on_design_art(s: &mut Session, cmd: &str, p: serde_json::Value) {
    let steps = s.doc().unwrap().history.undo.len();
    let e = s.execute(cmd, &p).unwrap_err().to_string();
    assert!(e.contains("drawn again from the design"), "{cmd}: {e}");
    // `appearance.addEffect` is an alias of `effect.apply`, whose errors name `effect.apply`.
    let named = if cmd == "appearance.addEffect" { "effect.apply" } else { cmd };
    assert!(e.contains(&format!("`{named}`")), "the error names the command that ran: {e}");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{cmd}");
}

/// The look of object `id`: its appearance, opacity, blend mode, isolation, knockout and overprint.
fn look(s: &Session, id: NodeId) -> String {
    format!("{:?}", s.doc().unwrap().doc.node(id).unwrap())
}

/// An edit of a graph design's art in a series lasts only until the graph is drawn again: the graph draws the art again
/// from the design.
#[test]
fn an_edit_of_a_graph_designs_art_is_lost_when_the_graph_is_drawn_again() {
    let (mut s, id, _, art, _) = design_art();
    let path = |s: &Session| {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let mut out = None;
        series(&n, 0).children().unwrap()[0].walk(&mut |m| {
            if out.is_none() && matches!(m.kind, NodeKind::Path { .. }) {
                out = Some(m.clone());
            }
        });
        out.unwrap()
    };
    assert_eq!(path(&s).id, art);
    s.execute("object.setProps", &json!({"id": art.0, "opacity": 50})).unwrap();
    assert!((path(&s).opacity - 0.5).abs() < 1e-6);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"csv": ",2024,2025\nQ1,3,2\nQ2,5,8\nQ3,-1,6"})).unwrap();
    assert!((path(&s).opacity - 1.0).abs() < 1e-6);
}

/// The paint, stroke, gradient and overprint commands leave the art of a graph design in a series as the design draws
/// it, and edit the other objects they are given.
#[test]
fn paint_commands_leave_a_graph_designs_art_out() {
    let (mut s, graph, copy, art, mark) = design_art();
    let fill = top_row(&s, art, true);
    let commands = [
        ("stroke.set", json!({"weight": 5})),
        ("stroke.weightUp", json!({})),
        ("stroke.setAdvanced", json!({"arrowScale": [200, 200]})),
        ("paint.editGradient", json!({"kind": "linear"})),
        ("paint.setGradientGeom", json!({"start": [0, 0], "end": [50, 50]})),
        ("object.setOverprint", json!({"fill": true})),
        ("paint.swap", json!({})),
        ("paint.default", json!({})),
        ("paint.invert", json!({"item": fill, "stroke": false})),
        ("paint.invert", json!({})),
    ];
    let (copy_was, art_was) = (look(&s, copy), look(&s, art));
    for (cmd, p) in &commands {
        for ids in [json!([copy.0]), json!([art.0])] {
            let steps = s.doc().unwrap().history.undo.len();
            let mut q = p.clone();
            q["ids"] = ids;
            let _ = s.execute(cmd, &q);
            assert_eq!((look(&s, copy), look(&s, art)), (copy_was.clone(), art_was.clone()), "{cmd}");
            assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{cmd}");
        }
        let mark_was = look(&s, mark);
        let mut q = p.clone();
        q["ids"] = json!([art.0, mark.0]);
        s.execute(cmd, &q).unwrap();
        assert_eq!(look(&s, art), art_was, "{cmd}");
        assert_ne!(look(&s, mark), mark_was, "{cmd}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
    // Selected, without ids.
    s.execute("select.set", &json!({"ids": [art.0]})).unwrap();
    s.execute("stroke.set", &json!({"weight": 5})).unwrap();
    assert_eq!(look(&s, art), art_was);
    // Given the whole graph, Invert recolors the marks and leaves the art.
    let mark_was = look(&s, mark);
    s.execute("paint.invert", &json!({"ids": [graph.0]})).unwrap();
    assert_eq!(look(&s, art), art_was);
    assert_ne!(look(&s, mark), mark_was);
    // A freeform gradient point is refused on the art alone.
    refused_on_design_art(&mut s, "paint.freeform.addPoint", json!({"ids": [art.0], "at": [0, 0]}));
}

/// `attributes.info` reads the overprint of a graph design's art in a series, which the overprint edits leave out.
#[test]
fn attributes_info_reads_a_graph_designs_art() {
    let (mut s, _, _, art, _) = design_art();
    let info = s.execute("attributes.info", &json!({"ids": [art.0]})).unwrap();
    assert_eq!(info["ids"], json!([art.0]));
    assert_eq!(info["overprintFill"], json!(false), "{info}");
    assert_eq!(info["overprintStroke"], json!(false), "{info}");
}

/// With only the art of a graph design selected, `stroke.setAdvanced` and the stroke mode of `paint.editGradient` act
/// as edits of a selection, not of the stroke the next object drawn takes.
#[test]
fn stroke_edits_of_selected_design_art_leave_the_new_art_stroke() {
    let (mut s, _, _, art, _) = design_art();
    s.execute("select.set", &json!({"ids": [art.0]})).unwrap();
    let was = format!("{:?}", s.new_art_stroke());
    s.execute("stroke.setAdvanced", &json!({"arrowScale": [200, 200], "swapArrows": true})).unwrap();
    assert_eq!(format!("{:?}", s.new_art_stroke()), was, "stroke.setAdvanced");
    let mode = s.new_art_stroke().gradient_mode;
    s.execute("paint.editGradient", &json!({"kind": "linear", "stroke": true, "strokeMode": "along"})).unwrap();
    assert_eq!(s.new_art_stroke().gradient_mode, mode, "paint.editGradient");
}

/// With only the art of a graph design given or selected, the gradient, swap, default and recolor commands leave the
/// paint for new art and the remembered colors as they are.
#[test]
fn paint_edits_of_design_art_alone_leave_the_defaults() {
    let (mut s, _, _, art, _) = design_art();
    let fill = top_row(&s, art, true);
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 3})).unwrap();
    let defaults = |s: &Session| format!("{:?}", (&s.paint, s.last_solid, &s.last_gradient, &s.recent_colors));
    let commands = [
        ("paint.editGradient", json!({"kind": "linear"})),
        ("paint.editGradient", json!({"kind": "radial", "stroke": true})),
        ("paint.swap", json!({})),
        ("paint.default", json!({})),
        ("paint.invert", json!({"item": fill, "stroke": false})),
        ("paint.complement", json!({"item": fill, "stroke": false})),
        ("paint.invert", json!({})),
        ("paint.complement", json!({"stroke": true})),
        ("stroke.set", json!({"weight": 7})),
        ("stroke.weightUp", json!({})),
    ];
    for (cmd, p) in &commands {
        let was = defaults(&s);
        let mut q = p.clone();
        q["ids"] = json!([art.0]);
        s.execute(cmd, &q).unwrap();
        assert_eq!(defaults(&s), was, "{cmd}, given");
        // Selecting the art makes the defaults take its paint.
        s.execute("select.set", &json!({"ids": [art.0]})).unwrap();
        let was = defaults(&s);
        s.execute(cmd, p).unwrap();
        assert_eq!(defaults(&s), was, "{cmd}, selected");
        s.execute("select.set", &json!({"ids": []})).unwrap();
    }
    // A bad `item`, `aligned` or `target` is still an error with only design art given.
    for bad in [
        json!({"ids": [art.0], "item": "top"}),
        json!({"ids": [art.0], "item": fill, "aligned": "yes"}),
        json!({"ids": [art.0], "item": fill, "target": "nowhere"}),
        json!({"ids": [art.0], "target": "nowhere"}),
    ] {
        assert!(s.execute("paint.invert", &bad).is_err(), "{bad}");
        assert!(s.execute("paint.complement", &bad).is_err(), "{bad}");
    }
    // So is an `item` the art has no row for, in each command that leaves the art out.
    let steps = s.doc().unwrap().history.undo.len();
    for (cmd, p) in [
        ("paint.invert", json!({})),
        ("stroke.set", json!({"weight": 2})),
        ("stroke.weightUp", json!({})),
        ("stroke.setAdvanced", json!({"swapArrows": true})),
        ("paint.editGradient", json!({"kind": "linear"})),
        ("paint.setGradientGeom", json!({"start": [0, 0], "end": [50, 50]})),
        ("object.setOverprint", json!({"fill": true})),
    ] {
        let mut q = p.clone();
        q["ids"] = json!([art.0]);
        q["item"] = json!(99);
        assert!(s.execute(cmd, &q).is_err(), "{cmd}");
    }
    // The art's stroke row named without `stroke` is a stroke row: the gradient commands succeed and change nothing.
    let stroke_row = top_row(&s, art, false);
    for p in [json!({"kind": "linear"}), json!({"start": [0, 0], "end": [50, 50]})] {
        let cmd = if p.get("kind").is_some() { "paint.editGradient" } else { "paint.setGradientGeom" };
        let mut q = p.clone();
        q["ids"] = json!([art.0]);
        q["item"] = json!(stroke_row);
        s.execute(cmd, &q).unwrap();
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
    // With nothing selected, the swap still swaps the defaults.
    let was = defaults(&s);
    s.execute("paint.swap", &json!({})).unwrap();
    assert_ne!(defaults(&s), was);
}

/// A spread or a negative aspect for the art of a graph design alone changes nothing, whatever kind of gradient the
/// default fill is; a bad value is still an error.
#[test]
fn gradient_edits_of_design_art_alone_skip_the_default_kind_checks() {
    let (mut s, _, _, art, _) = design_art();
    for (default, p) in [("freeform", json!({"spread": "repeat"})), ("linear", json!({"aspect": -50}))] {
        s.execute("select.set", &json!({"ids": []})).unwrap();
        s.execute("paint.editGradient", &json!({"kind": default})).unwrap();
        let (was, art_was, steps) = (format!("{:?}", s.paint), look(&s, art), s.doc().unwrap().history.undo.len());
        let mut q = p.clone();
        q["ids"] = json!([art.0]);
        s.execute("paint.editGradient", &q).unwrap_or_else(|e| panic!("{q}: {e}"));
        assert_eq!(format!("{:?}", s.paint), was, "{default}");
        assert_eq!(look(&s, art), art_was, "{default}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{default}");
    }
    assert!(s.execute("paint.editGradient", &json!({"ids": [art.0], "spread": "sideways"})).is_err());
}

/// Invert given a group around a graph recolors the group's own fill and the series marks, and leaves the art of a
/// graph design as it is. With `target: "contents"` the group's own fill stays as it is.
#[test]
fn invert_of_a_group_recolors_its_own_fill_and_leaves_design_art() {
    let (mut s, graph, _, art, mark) = design_art();
    s.execute("select.set", &json!({"ids": [graph.0]})).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("appearance.addFill", &json!({"ids": [group.0]})).unwrap();
    let row = top_row(&s, group, true);
    s.execute("paint.setFill", &json!({"ids": [group.0], "item": row, "color": "#ff0000"})).unwrap();
    let own = |s: &Session| format!("{:?}", s.doc().unwrap().doc.node(group).unwrap().appearance.items);
    let (own_was, art_was, mark_was) = (own(&s), look(&s, art), look(&s, mark));
    s.execute("paint.invert", &json!({"ids": [group.0]})).unwrap();
    assert_ne!(own(&s), own_was, "the group's own fill");
    assert_ne!(look(&s, mark), mark_was);
    assert_eq!(look(&s, art), art_was);
    // The color remembered is the group's own new fill, which the Fill proxy shows.
    let Some(Paint::Solid { color, .. }) = s.doc().unwrap().doc.node(group).unwrap().proxy_paint_only(false, None).cloned() else {
        panic!("the group's fill is not a solid color")
    };
    assert_eq!((s.last_solid, s.recent_colors.first().copied()), (color, Some(color)));
    let (own_was, mark_was) = (own(&s), look(&s, mark));
    s.execute("paint.invert", &json!({"ids": [group.0], "target": "contents"})).unwrap();
    assert_eq!(own(&s), own_was, "target: contents");
    assert_ne!(look(&s, mark), mark_was);
    assert_eq!(look(&s, art), art_was);
}

/// Invert and Complement given a series group that holds only a graph design's art recolor the group's own fill and
/// remember that color, as the last color and the newest recent color. With several such groups the color remembered
/// is that of the first group whose fill changed.
#[test]
fn recoloring_a_group_of_design_art_alone_remembers_its_own_fill() {
    // A fill of its own for `group`, painted `hex` (the color of a rectangle painted with it).
    let paint_own = |s: &mut Session, group: NodeId, hex: &str| {
        s.execute("appearance.addFill", &json!({"ids": [group.0]})).unwrap();
        let t = NodeId(s.execute("shape.rectangle", &json!({"x": 700, "y": 700, "width": 5, "height": 5})).unwrap()["id"].as_u64().unwrap());
        s.execute("paint.setFill", &json!({"ids": [t.0], "color": hex})).unwrap();
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let paint = d.node(t).unwrap().proxy_paint_only(false, None).unwrap().clone();
        d.node_mut(group).unwrap().appearance.fill_at_mut(None).unwrap().paint = paint;
    };
    // Series 0's group with its own fill painted `hex`.
    let painted_series = |hex: &str| {
        let (mut s, graph, _, art, _) = design_art();
        let group = series(s.doc().unwrap().doc.node(graph).unwrap(), 0).id;
        paint_own(&mut s, group, hex);
        (s, group, art)
    };
    for cmd in ["paint.invert", "paint.complement"] {
        let (mut s, group, art) = painted_series("#ff0000");
        let fill = |s: &Session| s.doc().unwrap().doc.node(group).unwrap().proxy_paint_only(false, None).unwrap().clone();
        let (art_was, fill_was) = (look(&s, art), fill(&s));
        s.execute(cmd, &json!({"ids": [group.0]})).unwrap();
        assert_eq!(look(&s, art), art_was, "{cmd}");
        assert_ne!(fill(&s), fill_was, "{cmd}: the group's fill is recolored");
        let Paint::Solid { color, .. } = fill(&s) else { panic!("{cmd}: the group's fill is not a solid color") };
        assert_eq!((s.last_solid, s.recent_colors.first().copied()), (color, Some(color)), "{cmd}");
    }
    // A gray fill is its own complement: nothing changes, and the remembered colors stay as they were.
    let (mut s, group, _) = painted_series("#808080");
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    let remembered = |s: &Session| format!("{:?}", (s.last_solid, &s.recent_colors));
    let was = remembered(&s);
    assert_eq!(s.execute("paint.complement", &json!({"ids": [group.0]})).unwrap()["changed"], json!(0));
    assert_eq!(remembered(&s), was);
    // Both series groups hold only the design's art: a gray one first and a red one, given together.
    let (mut s, graph, _, _, _) = design_art();
    s.execute("graph.column", &json!({"id": graph.0, "seriesIndexes": [1], "design": "Square"})).unwrap();
    let groups = [0, 1].map(|i| series(s.doc().unwrap().doc.node(graph).unwrap(), i).id);
    paint_own(&mut s, groups[0], "#808080");
    paint_own(&mut s, groups[1], "#ff0000");
    s.execute("paint.complement", &json!({"ids": [groups[0].0, groups[1].0]})).unwrap();
    let Some(Paint::Solid { color, .. }) = s.doc().unwrap().doc.node(groups[1]).unwrap().proxy_paint_only(false, None).cloned() else {
        panic!("the red group's fill is not a solid color")
    };
    assert_eq!((s.last_solid, s.recent_colors.first().copied()), (color, Some(color)));
}

/// `transparency.set` without `item` refuses the art of a graph design in a series, and given it with other objects
/// sets only theirs.
#[test]
fn object_transparency_leaves_a_graph_designs_art_out() {
    let (mut s, _, copy, art, mark) = design_art();
    refused_on_design_art(&mut s, "transparency.set", json!({"ids": [art.0], "opacity": 40}));
    refused_on_design_art(&mut s, "transparency.set", json!({"ids": [copy.0], "isolate": true}));
    s.execute("transparency.set", &json!({"ids": [art.0, mark.0], "opacity": 40, "blend": "multiply"})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!((d.node(art).unwrap().opacity - 1.0).abs() < 1e-6);
    assert!((d.node(mark).unwrap().opacity - 0.4).abs() < 1e-6);
}

/// With the art of a graph design selected first, the Appearance panel's active row (a row of that art) stands for no
/// row of the other objects selected: an effect goes on each of them as a whole.
#[test]
fn an_active_row_of_a_graph_designs_art_is_not_applied_to_other_objects() {
    let (mut s, _, _, art, mark) = design_art();
    let stroke = top_row(&s, art, false);
    s.execute("select.set", &json!({"ids": [art.0, mark.0]})).unwrap();
    assert_eq!(s.doc().unwrap().selection.subjects().first(), Some(&art));
    s.execute("appearance.setActiveItem", &json!({"index": stroke})).unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow"})).unwrap();
    let n = s.doc().unwrap().doc.node(mark).unwrap().clone();
    assert_eq!(n.appearance.effects.len(), 1);
    assert!(n.appearance.items.iter().all(|it| it.effects().is_empty()));
    assert!(s.doc().unwrap().doc.node(art).unwrap().appearance.effects.is_empty());
}

/// Expand Appearance leaves the art of a graph design in a series as the design draws it.
#[test]
fn expand_appearance_leaves_a_graph_designs_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let r = s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].clone();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [r]})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("graph.design", &json!({"new": "Shadow"})).unwrap();
    s.execute("graph.column", &json!({"id": id.0, "seriesIndexes": [0], "design": "Shadow"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let mut art = vec![];
    series(&n, 0).children().unwrap()[0].walk(&mut |m| {
        if !m.appearance.effects.is_empty() {
            art.push(m.id);
        }
    });
    let art = art[0];
    let other = s.execute("shape.rectangle", &json!({"x": 700, "y": 700, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [other]})).unwrap();
    let art_was = look(&s, art);
    // Given alone, by id or selected, it leaves Expand Appearance disabled.
    for p in [json!({"ids": [art.0]}), json!({})] {
        s.execute("select.set", &json!({"ids": [art.0]})).unwrap();
        assert!(s.execute("effect.expandAppearance", &p).unwrap_err().to_string().contains("not available"));
        assert_eq!(look(&s, art), art_was);
    }
    // Inside a selected graph or series group, the art stays as the design draws it: with nothing else to expand there,
    // Expand Appearance is disabled, and with a mark's effect, the mark expands alone.
    let series_group = series(&n, 0).id;
    for root in [id, series_group] {
        s.execute("select.set", &json!({"ids": [root.0]})).unwrap();
        assert!(s.execute("effect.expandAppearance", &json!({})).unwrap_err().to_string().contains("not available"));
        assert_eq!(look(&s, art), art_was);
    }
    let mark = series(&n, 1).children().unwrap()[0].id;
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [mark.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let expanded = s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(expanded["ids"], json!([mark.0]));
    assert_eq!(look(&s, art), art_was);
    s.execute("edit.undo", &json!({})).unwrap();
    // Selected with another object, the art is left as it is and the other object expands.
    s.execute("select.set", &json!({"ids": [art.0, other]})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(look(&s, art), art_was);
    assert!(s.doc().unwrap().doc.node(NodeId(other)).is_none_or(|m| m.appearance.effects.is_empty()));
}

/// `effect` with `params`, as `effect.apply` gives it to an object.
fn an_effect(s: &mut Session, effect: &str, params: serde_json::Value) -> vectorcraft_doc::Effect {
    let r = s.execute("shape.rectangle", &json!({"x": 700, "y": 20, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("effect.apply", &json!({"effect": effect, "ids": [r], "params": params})).unwrap();
    s.doc().unwrap().doc.node(NodeId(r)).unwrap().appearance.effects[0].clone()
}

/// Expand Appearance of a graph neither lists the art of a design in a series whose own effect it leaves, nor adds an
/// image of that effect to the document.
#[test]
fn expand_appearance_lists_no_design_art_and_rasterizes_none() {
    let (mut s, id, copy, _, mark) = design_art();
    let shadow = an_effect(&mut s, "stylize.dropShadow", json!({}));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(copy).unwrap().appearance.effects.push(shadow);
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [mark.0]})).unwrap();
    let copy_was = look(&s, copy);
    let images = |s: &Session| s.doc().unwrap().doc.images.len();
    let before = images(&s);
    s.execute("select.set", &json!({"ids": [mark.0]})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let per_mark = images(&s) - before;
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let expanded = s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(expanded["ids"], json!([mark.0]));
    assert_eq!(look(&s, copy), copy_was);
    assert_eq!(images(&s) - before, per_mark, "only the mark's shadow becomes an image");
}

/// A geometry effect of a series group that Expand Appearance bakes moves the art of a design in that series with the
/// marks.
#[test]
fn expand_appearance_moves_design_art_with_its_series_groups_effect() {
    let (mut s, id, copy, _, _) = design_art();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let group = series(&n, 0).id;
    let shift = an_effect(&mut s, "distort.transform", json!({"moveH": 50}));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(group).unwrap().appearance.effects.push(shift);
    let left = |s: &Session| {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        series(&n, 0).children().unwrap()[0].geometric_bounds().unwrap().x0
    };
    let was = left(&s);
    assert_eq!(series(&n, 0).children().unwrap()[0].id, copy);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert!((left(&s) - was - 50.0).abs() < 1e-6, "{} then {}", was, left(&s));
}

/// The copies that a Transform effect of a series group makes of a design's art keep their own effects when Expand
/// Appearance bakes the series group's effect, as the art they copy does.
#[test]
fn expand_appearance_leaves_the_effects_of_design_art_copies() {
    let (mut s, id, copy, _, _) = design_art();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let group = series(&n, 0).id;
    let shadow = an_effect(&mut s, "stylize.dropShadow", json!({}));
    let copies = an_effect(&mut s, "distort.transform", json!({"moveH": 50, "copies": 2}));
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    doc.node_mut(copy).unwrap().appearance.effects.push(shadow);
    doc.node_mut(group).unwrap().appearance.effects.push(copies);
    let shadows = |s: &Session| {
        let mut count = 0;
        s.doc().unwrap().doc.node(id).unwrap().walk(&mut |m| count += m.appearance.effects.iter().filter(|e| e.id == "stylize.dropShadow").count());
        count
    };
    let images = |s: &Session| s.doc().unwrap().doc.images.len();
    let before = images(&s);
    assert_eq!(shadows(&s), 1);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(shadows(&s), 3, "the art and its two copies keep their shadows");
    assert_eq!(images(&s), before, "no shadow becomes an image");
}

/// The commands that give rows, effects or graphic styles refuse the art of a graph design inside a series, given by
/// id (the design's copy or a path inside it) or selected: the graph draws that art again from the design.
#[test]
fn appearance_commands_refuse_a_graph_designs_art_in_a_series() {
    let (mut s, _, copy, art, _) = design_art();
    let (rect, _) = blue_style(&mut s, "Blue", false);
    let graphic = s.execute("shape.rectangle", &json!({"x": 700, "y": 700, "width": 10, "height": 10})).unwrap()["id"].clone();
    s.execute("threeD.addGraphic", &json!({"name": "Mark", "ids": [graphic]})).unwrap();
    let fill = top_row(&s, art, true);
    for ids in [json!([copy.0]), json!([art.0])] {
        refused_on_design_art(
            &mut s,
            "graphicStyle.addFromLibrary",
            json!({"library": "painted-sketched", "name": "Wet Ink", "apply": true, "ids": ids}),
        );
        for cmd in ["appearance.addFill", "appearance.addStroke", "appearance.clear", "appearance.reduceToBasic", "appearance.showAllHidden"] {
            refused_on_design_art(&mut s, cmd, json!({"ids": ids}));
        }
        refused_on_design_art(&mut s, "appearance.moveItem", json!({"ids": ids, "from": "contents", "to": 0}));
        refused_on_design_art(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": ids}));
        refused_on_design_art(&mut s, "appearance.addEffect", json!({"effect": "stylize.dropShadow", "ids": ids}));
        refused_on_design_art(&mut s, "effect.apply", json!({"effect": "threeD.extrude", "item": null, "ids": ids}));
        refused_on_design_art(&mut s, "effect.move", json!({"ids": ids, "from": 0, "to": 0}));
        refused_on_design_art(&mut s, "effect.remove", json!({"ids": ids, "index": 0}));
        refused_on_design_art(&mut s, "threeD.mapGraphic", json!({"ids": ids, "symbol": "Mark"}));
        refused_on_design_art(&mut s, "graphicStyle.apply", json!({"name": "Blue", "ids": ids}));
        refused_on_design_art(&mut s, "graphicStyle.breakLink", json!({"ids": ids}));
    }
    for cmd in ["appearance.removeItem", "appearance.duplicateItem"] {
        refused_on_design_art(&mut s, cmd, json!({"ids": [art.0], "index": fill}));
    }
    refused_on_design_art(&mut s, "appearance.setItem", json!({"ids": [art.0], "index": fill, "color": "#00ff00"}));
    refused_on_design_art(&mut s, "appearance.moveItem", json!({"ids": [art.0], "from": fill, "to": 0}));
    refused_on_design_art(&mut s, "transparency.set", json!({"ids": [art.0], "item": fill, "opacity": 40}));
    refused_on_design_art(&mut s, "appearance.copyFrom", json!({"source": rect.0, "ids": [art.0]}));
    refused_on_design_art(&mut s, "appearance.transfer", json!({"source": rect.0, "target": art.0, "copy": true}));
    refused_on_design_art(&mut s, "appearance.transfer", json!({"source": art.0, "target": rect.0}));
    // Selected, and through the contents of the design's copy.
    s.execute("select.set", &json!({"ids": [art.0]})).unwrap();
    refused_on_design_art(&mut s, "appearance.addFill", json!({}));
    refused_on_design_art(&mut s, "graphicStyle.addFromLibrary", json!({"library": "painted-sketched", "name": "Wet Ink", "apply": true}));
    refused_on_design_art(&mut s, "appearance.addStroke", json!({"ids": [copy.0], "target": "contents"}));
    // A Pathfinder effect does not group the selected paths of the design's copy.
    let mut paths = vec![];
    s.doc().unwrap().doc.node(copy).unwrap().walk(&mut |m| {
        if matches!(m.kind, NodeKind::Path { .. }) {
            paths.push(m.id.0);
        }
    });
    assert_eq!(paths.len(), 2);
    s.execute("select.set", &json!({"ids": paths})).unwrap();
    refused_on_design_art(&mut s, "effect.apply", json!({"effect": "pathfinder.add"}));
    // Read as a source, the art gives its look.
    s.execute("appearance.transfer", &json!({"source": art.0, "target": rect.0, "copy": true})).unwrap();
}

/// Given the art of a graph design in a series with other objects, the commands edit the other objects only.
#[test]
fn appearance_commands_edit_the_objects_given_with_a_graph_designs_art() {
    let (mut s, id, copy, art, mark) = design_art();
    let (rect, _) = blue_style(&mut s, "Blue", false);
    let rows = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().appearance.clone();
    let (copy_was, art_was, mark_was) = (rows(&s, copy), rows(&s, art), rows(&s, mark));
    s.execute("appearance.addFill", &json!({"ids": [copy.0, mark.0]})).unwrap();
    assert_eq!((rows(&s, copy), rows(&s, art)), (copy_was.clone(), art_was.clone()));
    assert_eq!(rows(&s, mark).items.len(), mark_was.items.len() + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [art.0, mark.0]})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_eq!(rows(&s, mark).effects.len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": rect.0, "ids": [art.0, mark.0]})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_ne!(rows(&s, mark), mark_was);
    s.execute("edit.undo", &json!({})).unwrap();
    // The graph's contents: series 1's marks take the fill, the design's art does not.
    s.execute("appearance.addFill", &json!({"ids": [id.0], "target": "contents"})).unwrap();
    assert_eq!((rows(&s, copy), rows(&s, art)), (copy_was, art_was));
    assert_eq!(rows(&s, mark).items.len(), mark_was.items.len() + 1);
}

/// `design_art()` with the design's art painted with a red and a blue fill and series 1's first mark with a blue fill and
/// a black stroke, so that row 1 of the art (its blue fill) is row 0 of the mark: the graph, the art and the mark.
fn aligned_design_art() -> (Session, NodeId, NodeId, NodeId) {
    let (mut s, id, _, art, mark) = design_art();
    let (red, blue) = (Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)), Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0)));
    let mark_look = vectorcraft_doc::Appearance::basic(blue, Paint::solid(vectorcraft_color::Color::BLACK), 1.0);
    let mut art_look = mark_look.clone();
    art_look.items = vec![art_look.items[0].clone(), art_look.items[0].clone()];
    *art_look.items[0].paint_mut() = red;
    s.edit("Looks", |d, _| {
        d.node_mut(art).unwrap().appearance = vectorcraft_doc::Shared::new(art_look.clone());
        d.node_mut(mark).unwrap().appearance = vectorcraft_doc::Shared::new(mark_look.clone());
        Ok(())
    })
    .unwrap();
    (s, id, art, mark)
}

/// Aligned rows given with the art of a graph design in a series first are rows of that art's stack, which the commands
/// leave as it is: each other object edits the row equal to the given one, never the row at the same index.
#[test]
fn aligned_rows_of_a_graph_designs_art_edit_the_equal_rows_of_the_other_objects() {
    let rows = |s: &Session, id: NodeId| (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    let green = Paint::solid(vectorcraft_color::Color::rgb(0.0, 1.0, 0.0));
    let (mut s, _, art, mark) = aligned_design_art();
    let (art_was, mark_was) = (rows(&s, art), rows(&s, mark));
    s.execute("appearance.removeItem", &json!({"ids": [art.0, mark.0], "index": 1, "aligned": true})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_eq!(rows(&s, mark).items, mark_was.items[1..], "the mark loses its blue fill and keeps its stroke");
    s.execute("edit.undo", &json!({})).unwrap();

    s.execute("appearance.setItem", &json!({"ids": [art.0, mark.0], "index": 1, "aligned": true, "color": "#00ff00"})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_eq!((rows(&s, mark).items[0].paint(), &rows(&s, mark).items[1]), (&green, &mark_was.items[1]));
    s.execute("edit.undo", &json!({})).unwrap();

    s.execute("appearance.duplicateItem", &json!({"ids": [art.0, mark.0], "index": 1, "aligned": true})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    let fills: Vec<bool> = rows(&s, mark).items.iter().map(|it| it.is_fill()).collect();
    assert_eq!(fills, [true, true, false], "the mark's blue fill is copied, not its stroke");
    s.execute("edit.undo", &json!({})).unwrap();

    s.execute("transparency.set", &json!({"ids": [art.0, mark.0], "item": 1, "aligned": true, "opacity": 40})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_eq!((rows(&s, mark).items[0].opacity(), rows(&s, mark).items[1].opacity()), (0.4, 1.0));

    // A row of the art past the end of the mark's stack.
    let (mut s, _, art, mark) = aligned_design_art();
    s.edit("Rows", |d, _| {
        let ap = &mut d.node_mut(art).unwrap().appearance;
        let red = ap.items[0].clone();
        ap.items.insert(0, red);
        Ok(())
    })
    .unwrap();
    s.execute("transparency.set", &json!({"ids": [art.0, mark.0], "item": 2, "aligned": true, "opacity": 40})).unwrap();
    assert_eq!((rows(&s, mark).items[0].opacity(), rows(&s, mark).items[1].opacity()), (0.4, 1.0));

    // A row of the art (its red fill) that the mark has no equal of: transparency.set is an error, and appearance.setItem
    // and appearance.removeItem leave the document as it is.
    let (art_was, mark_was) = (rows(&s, art), rows(&s, mark));
    let steps = s.doc().unwrap().history.undo.len();
    let e = s.execute("transparency.set", &json!({"ids": [art.0, mark.0], "item": 0, "aligned": true, "opacity": 40})).unwrap_err();
    assert!(e.to_string().contains("no appearance item 0"), "{e}");
    s.execute("appearance.setItem", &json!({"ids": [art.0, mark.0], "index": 0, "aligned": true, "color": "#00ff00"})).unwrap();
    s.execute("appearance.removeItem", &json!({"ids": [art.0, mark.0], "index": 0, "aligned": true})).unwrap();
    assert_eq!((rows(&s, art), rows(&s, mark)), (art_was, mark_was));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "no undo step is recorded");
}

/// Aligned effects given with the art of a graph design in a series first go on, and come off, the rows equal to the art's
/// row in the other objects; an object without an equal row is left as it is.
#[test]
fn aligned_effects_of_a_graph_designs_art_edit_the_equal_rows_of_the_other_objects() {
    let rows = |s: &Session, id: NodeId| (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    let effects = |s: &Session, id: NodeId| rows(s, id).items.iter().map(|it| it.effects().len()).collect::<Vec<_>>();
    let (mut s, id, art, mark) = aligned_design_art();
    // A second mark of series 1 with a green fill, which no row of the art equals, and the same black stroke.
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let other = series(&n, 1).children().unwrap()[1].id;
    let mut other_look = rows(&s, mark);
    *other_look.items[0].paint_mut() = Paint::solid(vectorcraft_color::Color::rgb(0.0, 1.0, 0.0));
    s.edit("Look", |d, _| {
        d.node_mut(other).unwrap().appearance = vectorcraft_doc::Shared::new(other_look.clone());
        Ok(())
    })
    .unwrap();
    let (art_was, other_was) = (rows(&s, art), rows(&s, other));
    let r = s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [art.0, mark.0, other.0], "item": 1, "aligned": true})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_eq!(effects(&s, mark), [1, 0], "the shadow goes on the mark's blue fill");
    assert_eq!(rows(&s, other), other_was, "the other mark has no blue fill");
    assert_eq!((&r["ids"], &r["item"], &r["index"]), (&json!([mark.0]), &json!(0), &json!(0)), "the reply names the mark's row");
    s.execute("edit.undo", &json!({})).unwrap();
    // A 3D effect is checked on the rows it goes on only.
    s.execute("effect.apply", &json!({"effect": "threeD.extrude", "ids": [art.0, mark.0, other.0], "item": 1, "aligned": true})).unwrap();
    assert_eq!((rows(&s, art), effects(&s, mark), rows(&s, other)), (art_was.clone(), vec![1, 0], other_was.clone()));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("appearance.addEffect", &json!({"effect": "stylize.dropShadow", "ids": [art.0, mark.0, other.0], "item": 1, "aligned": true})).unwrap();
    assert_eq!((rows(&s, art), effects(&s, mark), rows(&s, other)), (art_was, vec![1, 0], other_was));

    // The art's blue fill with the same shadow as the mark's: removing it from the art's row takes it off the mark's.
    s.edit("Shadow", |d, _| {
        let row = d.node(mark).unwrap().appearance.items[0].clone();
        d.node_mut(art).unwrap().appearance.items[1] = row;
        Ok(())
    })
    .unwrap();
    let art_was = rows(&s, art);
    // The effect dialog reads the shadow from the art's row and passes the art on first.
    let (target, fx) = s.appearance_effect_target(&json!({"ids": [art.0, mark.0], "item": 1, "aligned": true})).unwrap();
    assert_eq!((fx.len(), &target["ids"], &target["item"]), (1, &json!([art.0, mark.0]), &json!(1)));
    // Edits of the art's row effect change the mark's equal row.
    s.execute("effect.setParams", &json!({"ids": [art.0, mark.0], "item": 1, "index": 0, "visible": false, "aligned": true})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert!(!rows(&s, mark).items[0].effects()[0].visible, "the mark's shadow is hidden");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("effect.duplicate", &json!({"ids": [art.0, mark.0], "item": 1, "index": 0, "aligned": true})).unwrap();
    assert_eq!((rows(&s, art), effects(&s, mark)), (art_was.clone(), vec![2, 0]));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("effect.remove", &json!({"ids": [art.0, mark.0], "item": 1, "index": 0, "aligned": true})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_eq!(effects(&s, mark), [0, 0]);

    // An object-level effect of the art that the mark does not have: a new object-level effect still goes on the mark.
    s.edit("Effect", |d, _| {
        let shadow = d.node(art).unwrap().appearance.items[1].effects()[0].clone();
        d.node_mut(art).unwrap().appearance.effects.push(shadow);
        Ok(())
    })
    .unwrap();
    let art_was = rows(&s, art);
    s.execute("effect.apply", &json!({"effect": "distort.roughen", "ids": [art.0, mark.0], "item": null, "aligned": true})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_eq!(rows(&s, mark).effects.len(), 1);
    // The art's object-level shadow is not the mark's roughen: the effect dialog reads the art's, and removing it leaves
    // the mark as it is.
    let (_, fx) = s.appearance_effect_target(&json!({"ids": [art.0, mark.0], "item": null, "aligned": true})).unwrap();
    assert_eq!(fx.iter().map(|e| e.id.to_string()).collect::<Vec<_>>(), ["stylize.dropShadow"]);
    let mark_was = rows(&s, mark);
    assert!(s.execute("effect.remove", &json!({"ids": [art.0, mark.0], "item": null, "index": 0, "aligned": true})).is_err());
    assert_eq!((rows(&s, art), rows(&s, mark)), (art_was.clone(), mark_was));
    // An object-level effect index past the end of the art's list names no effect of the mark either.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [mark.0], "item": null})).unwrap();
    let mark_was = rows(&s, mark);
    assert!(s.execute("effect.remove", &json!({"ids": [art.0, mark.0], "item": null, "index": 1, "aligned": true})).is_err());
    assert_eq!((rows(&s, art), rows(&s, mark)), (art_was.clone(), mark_was));
    // The mark with the art's object-level shadow after its roughen: removing the art's effect 0 takes off the mark's 1.
    s.edit("Shadow", |d, _| {
        let shadow = d.node(art).unwrap().appearance.effects[0].clone();
        d.node_mut(mark).unwrap().appearance.effects[1] = shadow;
        Ok(())
    })
    .unwrap();
    s.execute("effect.remove", &json!({"ids": [art.0, mark.0], "item": null, "index": 0, "aligned": true})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    assert_eq!(rows(&s, mark).effects.iter().map(|e| e.id.to_string()).collect::<Vec<_>>(), ["distort.roughen"]);
}

/// Aligned rows of a graph design's art and of another object whose stack differs are not one place in both stacks, so
/// they are not reordered, and `to` is refused for their copies; with the same stacks, the copies go to `to`.
#[test]
fn aligned_rows_of_a_graph_designs_art_and_a_different_stack_are_not_moved() {
    let rows = |s: &Session, id: NodeId| (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    let (mut s, _, art, mark) = aligned_design_art();
    let mark_was = rows(&s, mark);
    assert!(s.execute("appearance.moveItem", &json!({"ids": [art.0, mark.0], "from": 1, "to": 0, "aligned": true})).is_err());
    assert_eq!(rows(&s, mark), mark_was);
    assert!(s.execute("appearance.duplicateItem", &json!({"ids": [art.0, mark.0], "index": 1, "to": 0, "aligned": true})).is_err());
    assert_eq!(rows(&s, mark), mark_was);
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [mark.0], "item": null})).unwrap();
    s.execute("effect.apply", &json!({"effect": "distort.roughen", "ids": [mark.0], "item": null})).unwrap();
    let mark_was = rows(&s, mark);
    assert!(s.execute("effect.move", &json!({"ids": [art.0, mark.0], "from": 1, "to": 0, "aligned": true})).is_err());
    assert_eq!(rows(&s, mark), mark_was);

    // The mark with the art's stack.
    let (mut s, _, art, mark) = aligned_design_art();
    let art_was = rows(&s, art);
    s.edit("Look", |d, _| {
        d.node_mut(mark).unwrap().appearance = vectorcraft_doc::Shared::new(art_was.clone());
        Ok(())
    })
    .unwrap();
    assert!(s.execute("appearance.duplicateItem", &json!({"ids": [art.0, mark.0], "indices": [1, 9], "to": 0, "aligned": true})).is_err());
    assert_eq!(rows(&s, mark), art_was, "a row that does not exist is refused");
    s.execute("appearance.duplicateItem", &json!({"ids": [art.0, mark.0], "index": 1, "to": 0, "aligned": true})).unwrap();
    assert_eq!(rows(&s, art), art_was);
    let paints: Vec<_> = rows(&s, mark).items.iter().map(|it| it.paint().clone()).collect();
    assert_eq!(paints, [art_was.items[1].paint().clone(), art_was.items[0].paint().clone(), art_was.items[1].paint().clone()]);
}

/// An `item` given with `aligned: true` and the art of a graph design in a series first is a row of the art's stack in the
/// paint and stroke commands: each other object edits the row equal to it, and a row the art does not have as a fill or
/// stroke as edited, or that no other object has, is an error that leaves the document and its undo history as they are.
#[test]
fn aligned_paint_rows_of_a_graph_designs_art_edit_the_equal_rows_of_the_other_objects() {
    let rows = |s: &Session, id: NodeId| (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    let green = Paint::solid(vectorcraft_color::Color::rgb(0.0, 1.0, 0.0));
    // The art's row 1 is the blue fill the mark has as row 0, and its row 2 the black stroke the mark has as row 1.
    let (mut s, _, art, mark) = aligned_design_art();
    let black = rows(&s, mark).items[1].clone();
    s.edit("Stroke", |d, _| {
        d.node_mut(art).unwrap().appearance.items.push(black.clone());
        Ok(())
    })
    .unwrap();
    let (art_was, mark_was) = (rows(&s, art), rows(&s, mark));
    let fill = |s: &mut Session, cmd: &str, p: serde_json::Value| {
        s.execute(cmd, &p).unwrap();
        let now = rows(s, mark);
        assert_eq!(rows(s, art), art_was, "{cmd}");
        assert_ne!(now.items[0], mark_was.items[0], "{cmd} edits the mark's blue fill");
        assert_eq!(now.items[1], mark_was.items[1], "{cmd} leaves the mark's stroke");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(rows(s, mark), mark_was, "{cmd} is one undo step");
        now
    };
    let ids = [art.0, mark.0];
    let now = fill(&mut s, "paint.setFill", json!({"ids": ids, "item": 1, "aligned": true, "color": "#00ff00"}));
    assert_eq!(now.items[0].paint(), &green);
    let now = fill(&mut s, "paint.invert", json!({"ids": ids, "item": 1, "aligned": true}));
    assert_eq!(Some(s.last_solid), now.items[0].paint().color(), "the inverted color is the last color, not the art's");
    let now = fill(&mut s, "paint.lastColor", json!({"ids": ids, "item": 1, "aligned": true, "stroke": false}));
    assert_eq!(Some(s.last_solid), now.items[0].paint().color());
    // Without `aligned`, the art is left out and the item is a row of the mark.
    let now = fill(&mut s, "paint.invert", json!({"ids": ids, "item": 0}));
    assert_eq!(Some(s.last_solid), now.items[0].paint().color(), "the mark's inverted color is the last color, not the art's");
    s.last_solid = vectorcraft_color::Color::rgb(0.5, 0.5, 0.5);
    let now = fill(&mut s, "paint.complement", json!({"ids": ids, "item": 0}));
    assert_eq!(Some(s.last_solid), now.items[0].paint().color(), "the mark's complement is the last color, not the art's");
    let now = fill(&mut s, "paint.editGradient", json!({"ids": ids, "item": 1, "aligned": true, "kind": "linear"}));
    let Paint::Gradient(g) = now.items[0].paint() else { panic!("{:?}", now.items[0].paint()) };
    assert_eq!(&s.last_gradient, &**g, "the mark's new gradient is the last gradient");
    let now = fill(&mut s, "paint.lastGradient", json!({"ids": ids, "item": 1, "aligned": true, "stroke": false}));
    assert!(matches!(now.items[0].paint(), Paint::Gradient(_)));

    let stroke = |s: &mut Session, cmd: &str, p: serde_json::Value| {
        s.execute(cmd, &p).unwrap();
        let now = rows(s, mark);
        assert_eq!(rows(s, art), art_was, "{cmd}");
        assert_eq!(now.items[0], mark_was.items[0], "{cmd} leaves the mark's fill");
        assert_ne!(now.items[1], mark_was.items[1], "{cmd} edits the mark's stroke");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(rows(s, mark), mark_was, "{cmd} is one undo step");
        now
    };
    let now = stroke(&mut s, "paint.setStroke", json!({"ids": ids, "item": 2, "aligned": true, "color": "#00ff00"}));
    assert_eq!(now.items[1].paint(), &green);
    let now = stroke(&mut s, "stroke.set", json!({"ids": ids, "item": 2, "aligned": true, "weight": 5}));
    assert_eq!(now.stroke_at(Some(1)).unwrap().width, 5.0);
    let now = stroke(&mut s, "stroke.setAdvanced", json!({"ids": ids, "item": 2, "aligned": true, "arrowScale": [200, 200]}));
    assert_eq!(now.stroke_at(Some(1)).unwrap().arrow_scale, (200.0, 200.0));
    // Without `stroke`, the kind of the art's row is edited.
    let now = stroke(&mut s, "paint.editGradient", json!({"ids": ids, "item": 2, "aligned": true, "kind": "linear"}));
    assert!(matches!(now.items[1].paint(), Paint::Gradient(_)));
    fill(&mut s, "paint.complement", json!({"ids": ids, "item": 1, "aligned": true, "stroke": false}));

    let steps = s.doc().unwrap().history.undo.len();
    for (cmd, p, said) in [
        ("stroke.set", json!({"ids": ids, "item": 1, "aligned": true, "weight": 5}), "appearance item 1 is not a stroke"),
        ("paint.setFill", json!({"ids": ids, "item": 2, "aligned": true, "color": "#00ff00"}), "appearance item 2 is not a fill"),
        ("paint.setFill", json!({"ids": ids, "item": 0, "aligned": true, "color": "#00ff00"}), "no appearance item 0"),
        ("paint.setFill", json!({"ids": ids, "item": 9, "aligned": true, "color": "#00ff00"}), "no appearance item 9"),
        // The art alone: no other object has the row.
        ("paint.setFill", json!({"ids": [art.0], "item": 9, "aligned": true, "color": "#00ff00"}), "no appearance item 9"),
        ("paint.setFill", json!({"ids": [art.0], "item": 1, "aligned": true, "color": "#00ff00"}), "no appearance item 1"),
        ("stroke.set", json!({"ids": [art.0], "item": 1, "aligned": true, "weight": 5}), "appearance item 1 is not a stroke"),
        ("paint.editGradient", json!({"ids": [art.0], "item": 1, "aligned": true, "kind": "linear"}), "no appearance item 1"),
        // The weight steps take no row: they step each object's active or top stroke.
        ("stroke.weightUp", json!({"ids": ids, "item": 2}), "unknown parameter `item`"),
        ("stroke.weightUp", json!({"ids": ids, "aligned": true}), "unknown parameter `aligned`"),
        ("stroke.weightDown", json!({"ids": ids, "item": 2}), "unknown parameter `item`"),
        ("stroke.weightDown", json!({"ids": ids, "aligned": true}), "unknown parameter `aligned`"),
    ] {
        let defaults = format!("{:?}", (&s.paint.fill, &s.paint.stroke, &s.last_gradient));
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains(said), "{cmd}: {e}");
        assert_eq!((rows(&s, art), rows(&s, mark)), (art_was.clone(), mark_was.clone()), "{cmd}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{cmd}");
        assert_eq!(format!("{:?}", (&s.paint.fill, &s.paint.stroke, &s.last_gradient)), defaults, "{cmd}");
    }

    // An object with no row equal to the art's is left out of what the commands report as edited.
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let rect = *s.doc().unwrap().selection.subjects().first().unwrap();
    s.execute("paint.setFill", &json!({"ids": [rect.0], "color": "#ff00ff"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#ff00ff"})).unwrap();
    let rect_was = rows(&s, rect);
    let ids = [art.0, mark.0, rect.0];
    for (cmd, p, key, said) in [
        ("paint.setFill", json!({"ids": ids, "item": 1, "aligned": true, "color": "#00ff00"}), "objects", json!(1)),
        ("stroke.set", json!({"ids": ids, "item": 2, "aligned": true, "weight": 5}), "objects", json!(1)),
        ("paint.editGradient", json!({"ids": ids, "item": 1, "aligned": true, "kind": "linear"}), "ids", json!([mark.0])),
        ("paint.setGradientGeom", json!({"ids": ids, "item": 1, "aligned": true, "start": [0, 0], "end": [10, 40]}), "ids", json!([mark.0])),
    ] {
        if cmd == "paint.setGradientGeom" {
            s.execute("paint.editGradient", &json!({"ids": [mark.0], "item": 0, "kind": "linear"})).unwrap();
            let gradient = rows(&s, mark).items[0].clone();
            s.edit("Gradient", |d, _| {
                d.node_mut(art).unwrap().appearance.items[1] = gradient.clone();
                Ok(())
            })
            .unwrap();
        }
        let r = s.execute(cmd, &p).unwrap();
        assert_eq!(r[key], said, "{cmd}: {r}");
        assert_eq!(rows(&s, rect), rect_was, "{cmd}");
        s.execute("edit.undo", &json!({})).unwrap();
    }

    // A gradient spanning the selection spans only the objects that take it: a distant object without an equal row does
    // not move it.
    s.execute("shape.rectangle", &json!({"x": 5000, "y": 5000, "width": 10, "height": 10})).unwrap();
    let far = *s.doc().unwrap().selection.subjects().first().unwrap();
    s.execute("paint.setFill", &json!({"ids": [far.0], "color": "#ff00ff"})).unwrap();
    let far_was = rows(&s, far);
    let gradient = json!({"stops": [{"offset": 0, "color": "#000000"}, {"offset": 1, "color": "#ffffff"}]});
    let mut placed = Vec::new();
    for ids in [vec![art.0, mark.0], vec![art.0, mark.0, far.0]] {
        s.execute("paint.setFill", &json!({"ids": ids, "item": 1, "aligned": true, "gradient": gradient, "span": "selection"})).unwrap();
        placed.push(rows(&s, mark).items[0].clone());
        assert_eq!(rows(&s, far), far_was);
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert!(matches!(placed[0].paint(), Paint::Gradient(_)));
    assert_eq!(placed[1], placed[0]);
}

/// The gradient and freeform gradient commands read an `item` given with `aligned: true` and the art of a graph design in
/// a series first as a row of the art's stack, as the paint commands do.
#[test]
fn aligned_gradient_rows_of_a_graph_designs_art_edit_the_equal_rows_of_the_other_objects() {
    let rows = |s: &Session, id: NodeId| (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    for kind in ["linear", "freeform"] {
        let (mut s, _, art, mark) = aligned_design_art();
        // The mark's fill and the art's equal row become the same gradient.
        s.execute("paint.editGradient", &json!({"ids": [mark.0], "item": 0, "kind": kind})).unwrap();
        let gradient = rows(&s, mark).items[0].clone();
        // A yellow fill above the mark's gradient is the mark's row 1, as the gradient is the art's row 1.
        let mut yellow = gradient.clone();
        *yellow.paint_mut() = Paint::solid(vectorcraft_color::Color::rgb(1.0, 1.0, 0.0));
        s.edit("Gradient", |d, _| {
            d.node_mut(art).unwrap().appearance.items[1] = gradient.clone();
            d.node_mut(mark).unwrap().appearance.items.insert(1, yellow.clone());
            Ok(())
        })
        .unwrap();
        let ids = [art.0, mark.0];
        let edits = if kind == "linear" {
            vec![
                ("paint.setGradientGeom", json!({"ids": ids, "item": 1, "aligned": true, "start": [0, 0], "end": [10, 40]})),
                ("paint.editGradient", json!({"ids": ids, "item": 1, "aligned": true, "kind": "radial"})),
            ]
        } else {
            vec![("paint.freeform.addPoint", json!({"ids": ids, "item": 1, "aligned": true, "at": [5, 5]}))]
        };
        for (cmd, p) in edits {
            let (art_was, mark_was) = (rows(&s, art), rows(&s, mark));
            s.execute(cmd, &p).unwrap();
            let now = rows(&s, mark);
            assert_eq!(rows(&s, art), art_was, "{cmd}");
            assert_ne!(now.items[0], mark_was.items[0], "{cmd} edits the mark's gradient fill");
            assert_eq!(now.items[1..], mark_was.items[1..], "{cmd}");
            s.execute("edit.undo", &json!({})).unwrap();
            // The edited gradient is the last gradient (paint.setGradientGeom does not set it).
            if cmd != "paint.setGradientGeom" {
                let Paint::Gradient(g) = now.items[0].paint() else { panic!("{cmd}") };
                assert_eq!(&s.last_gradient, &**g, "{cmd}: the mark's edited gradient is the last gradient");
            }
        }
    }
}

/// The Contents row of a graph design's copy and of a group whose stack differs is not moved with `aligned: true`, as
/// the rows are not.
#[test]
fn aligned_contents_row_of_a_graph_designs_copy_and_a_different_stack_is_not_moved() {
    let rows = |s: &Session, id: NodeId| (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    let (mut s, _, copy, _, _) = design_art();
    s.execute("shape.rectangle", &json!({"x": 700, "y": 700, "width": 10, "height": 10})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("appearance.addFill", &json!({"ids": [g.0]})).unwrap();
    s.execute("appearance.addFill", &json!({"ids": [g.0]})).unwrap();
    assert_ne!(rows(&s, copy), rows(&s, g));
    let (copy_was, g_was) = (rows(&s, copy), rows(&s, g));
    let steps = s.doc().unwrap().history.undo.len();
    let e = s.execute("appearance.moveItem", &json!({"ids": [copy.0, g.0], "from": "contents", "to": 1, "aligned": true})).unwrap_err();
    assert!(e.to_string().contains("appearances differ"), "{e}");
    assert_eq!((rows(&s, copy), rows(&s, g)), (copy_was, g_was));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
}

/// Invert with an `item` remembers the paint of an object it recolored, not that of a graph design's art given first,
/// which it leaves out.
#[test]
fn invert_of_an_item_remembers_the_recolored_paint_not_the_design_arts() {
    let (mut s, _, art, mark) = aligned_design_art();
    s.execute("paint.invert", &json!({"ids": [art.0, mark.0], "item": 0, "stroke": false})).unwrap();
    let Paint::Solid { color, .. } = s.doc().unwrap().doc.node(mark).unwrap().appearance.items[0].paint().clone() else {
        panic!("the mark's fill stays a solid color");
    };
    assert_ne!(color, vectorcraft_color::Color::rgb(0.0, 0.0, 1.0), "the mark's blue fill is inverted");
    assert_eq!(s.last_solid, color, "the mark's inverted fill is remembered, not the art's red one");
}

/// With a graph design's art given first and `aligned: true`, `effect.remove` and `effect.duplicate` with `indices` are an
/// error and change nothing when one of the indices names no effect of the art.
#[test]
fn aligned_effect_indices_past_the_end_of_a_graph_designs_art_are_refused() {
    let rows = |s: &Session, id: NodeId| (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    let (mut s, _, art, mark) = aligned_design_art();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [mark.0], "item": null})).unwrap();
    s.edit("Shadow", |d, _| {
        let shadow = d.node(mark).unwrap().appearance.effects[0].clone();
        d.node_mut(art).unwrap().appearance.effects.push(shadow);
        Ok(())
    })
    .unwrap();
    let (art_was, mark_was) = (rows(&s, art), rows(&s, mark));
    let steps = s.doc().unwrap().history.undo.len();
    for cmd in ["effect.remove", "effect.duplicate"] {
        let e = s.execute(cmd, &json!({"ids": [art.0, mark.0], "item": null, "indices": [0, 9], "aligned": true})).unwrap_err();
        assert!(e.to_string().contains("no effects at indices 0, 9"), "{cmd}: {e}");
        assert_eq!((rows(&s, art), rows(&s, mark)), (art_was.clone(), mark_was.clone()), "{cmd}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{cmd}");
    }
    // With index 0 alone, which the art has, the mark's equal effect is removed.
    s.execute("effect.remove", &json!({"ids": [art.0, mark.0], "item": null, "indices": [0], "aligned": true})).unwrap();
    assert!(rows(&s, mark).effects.is_empty());
}

/// With a graph design's art given first and `aligned: true`, `effect.remove` and `effect.duplicate` with `indices` are an
/// error and change nothing when one of the art's effects is one no other object has, as with that index given alone.
#[test]
fn aligned_effect_indices_of_a_graph_designs_art_that_no_object_has_are_refused() {
    let rows = |s: &Session, id: NodeId| (*s.doc().unwrap().doc.node(id).unwrap().appearance).clone();
    let (mut s, _, art, mark) = aligned_design_art();
    // The art has a shadow and a roughen, the mark the shadow only.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [mark.0], "item": null})).unwrap();
    s.execute("effect.apply", &json!({"effect": "distort.roughen", "ids": [mark.0], "item": null})).unwrap();
    s.edit("Effects", |d, _| {
        let effects = d.node(mark).unwrap().appearance.effects.clone();
        d.node_mut(art).unwrap().appearance.effects = effects;
        d.node_mut(mark).unwrap().appearance.effects.pop();
        Ok(())
    })
    .unwrap();
    let (art_was, mark_was) = (rows(&s, art), rows(&s, mark));
    let steps = s.doc().unwrap().history.undo.len();
    for cmd in ["effect.remove", "effect.duplicate"] {
        assert!(s.execute(cmd, &json!({"ids": [art.0, mark.0], "item": null, "index": 1, "aligned": true})).is_err(), "{cmd}");
        let e = s.execute(cmd, &json!({"ids": [art.0, mark.0], "item": null, "indices": [0, 1], "aligned": true})).unwrap_err();
        assert!(e.to_string().contains("no effects at indices 0, 1"), "{cmd}: {e}");
        assert_eq!((rows(&s, art), rows(&s, mark)), (art_was.clone(), mark_was.clone()), "{cmd}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{cmd}");
    }
    // With the roughen on the mark too, both of its effects are removed.
    s.edit("Roughen", |d, _| {
        let roughen = d.node(art).unwrap().appearance.effects[1].clone();
        d.node_mut(mark).unwrap().appearance.effects.push(roughen);
        Ok(())
    })
    .unwrap();
    s.execute("effect.remove", &json!({"ids": [art.0, mark.0], "item": null, "indices": [0, 1], "aligned": true})).unwrap();
    assert!(rows(&s, mark).effects.is_empty());
}

/// A blur on a series group makes the whole group an image, which draws the design's art with the series' other art:
/// Expand Appearance does not put the art back beside the image.
#[test]
fn expanding_a_blurred_series_group_takes_a_graph_designs_art_into_the_image() {
    let (mut s, id, copy, _, _) = design_art();
    let series_group = series(s.doc().unwrap().doc.node(id).unwrap(), 0).id;
    s.execute("effect.apply", &json!({"effect": "blur.gaussian", "ids": [series_group.0]})).unwrap();
    s.execute("effect.expandAppearance", &json!({"ids": [series_group.0]})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(matches!(d.node(series_group).unwrap().kind, NodeKind::Image(_)));
    assert!(d.node(copy).is_none());
}

/// The paint commands leave the art of a graph design in a series out, a copy of the design given by id with an item
/// included.
#[test]
fn paint_targets_leave_a_graph_designs_copy_out() {
    let (s, _, copy, art, mark) = design_art();
    let d = &s.doc().unwrap().doc;
    assert_eq!(super::paint_targets(d, &[copy, art, mark]), [mark]);
}

/// The art of a marker design in series 1 of a line graph: the design's copy and a path inside it.
fn marker_design_art() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Pill");
    let id = graph(&mut s, "line");
    s.execute("graph.marker", &json!({"id": id.0, "seriesIndexes": [1], "design": "Pill"})).unwrap();
    let marker = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[1].clone();
    assert!(matches!(marker.kind, NodeKind::Group { .. }), "the design's art");
    let mut art = vec![];
    marker.walk(&mut |m| {
        if matches!(m.kind, NodeKind::Path { .. }) {
            art.push(m.id);
        }
    });
    (s, marker.id, art[0])
}

/// The ids `graphicStyle.list` reports as linked to style `name`.
fn linked_to(s: &mut Session, name: &str) -> Vec<u64> {
    let list = s.execute("graphicStyle.list", &json!({})).unwrap();
    let style = list["styles"].as_array().unwrap().iter().find(|g| g["name"] == name).unwrap().clone();
    let mut ids: Vec<u64> = style["linked"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    ids.sort_unstable();
    ids
}

/// Graphic styles and the art of a column or marker design in a series (the design's copy and a path inside it):
/// `graphicStyle.new` makes a style from the art without linking it; `graphicStyle.redefine` leaves the art's look as
/// it was and unlinks it, whether the art is linked (as in a document saved before) or the source, while the other
/// linked objects follow the new look.
#[test]
fn graphic_styles_leave_a_graph_designs_art_unlinked() {
    let make = |marker: bool| {
        if marker {
            marker_design_art()
        } else {
            let (s, _, copy, art, _) = design_art();
            (s, copy, art)
        }
    };
    let rows = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().appearance.clone();
    let link = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().extras().graphic_style;
    let link_directly = |s: &mut Session, ids: &[NodeId], style: u32| {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        for id in ids {
            d.node_mut(*id).unwrap().extras_mut().graphic_style = Some(style);
        }
    };
    let rect = |s: &mut Session| {
        NodeId(s.execute("shape.rectangle", &json!({"x": 700, "y": 700, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap())
    };
    for marker in [false, true] {
        for source in [0, 1] {
            let at = format!("marker {marker}, source {source}");
            // New: the style is made from the art, which stays unlinked.
            let (mut s, copy, art) = make(marker);
            let source = [copy, art][source];
            s.execute("graphicStyle.new", &json!({"id": source.0, "name": "Art"})).unwrap();
            assert_eq!(link(&s, source), None, "{at}");
            assert_eq!(linked_to(&mut s, "Art"), Vec::<u64>::new(), "{at}");
            // Redefine from another object: linked art keeps its look and is unlinked; a linked rectangle follows.
            let follower = rect(&mut s);
            s.execute("graphicStyle.apply", &json!({"name": "Art", "ids": [follower.0]})).unwrap();
            let style = link(&s, follower).unwrap();
            link_directly(&mut s, &[copy, art], style);
            let (copy_was, art_was) = (rows(&s, copy), rows(&s, art));
            let (blue_rect, blue) = blue_style(&mut s, "Blue", false);
            s.execute("graphicStyle.redefine", &json!({"name": "Art", "id": blue_rect.0})).unwrap();
            assert_eq!((rows(&s, copy), rows(&s, art)), (copy_was.clone(), art_was.clone()), "{at}");
            assert_eq!((link(&s, copy), link(&s, art)), (None, None), "{at}");
            assert_eq!(rows(&s, follower).fill_paint(), blue, "{at}");
            assert_eq!(linked_to(&mut s, "Art"), [follower.0.min(blue_rect.0), follower.0.max(blue_rect.0)], "{at}");
            // Redefine from the art itself: the art is unlinked and the linked rectangle takes the art's look.
            link_directly(&mut s, &[copy, art], style);
            s.execute("graphicStyle.redefine", &json!({"name": "Art", "id": source.0})).unwrap();
            assert_eq!((rows(&s, copy), rows(&s, art)), (copy_was, art_was), "{at}");
            assert_eq!((link(&s, copy), link(&s, art)), (None, None), "{at}");
            assert_eq!(linked_to(&mut s, "Art"), [follower.0.min(blue_rect.0), follower.0.max(blue_rect.0)], "{at}");
        }
    }
}

/// The art of a marker design in a series is refused too.
#[test]
fn effect_commands_refuse_a_marker_designs_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Pill");
    let id = graph(&mut s, "line");
    s.execute("graph.marker", &json!({"id": id.0, "seriesIndexes": [1], "design": "Pill"})).unwrap();
    let marker = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[1].clone();
    assert!(matches!(marker.kind, NodeKind::Group { .. }), "the design's art");
    refused_on_design_art(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [marker.id.0]}));
}

/// A series' kept opacity goes on its generated marks only: a column design made of one path keeps its own opacity.
#[test]
fn a_column_design_keeps_its_own_opacity_under_a_series_opacity() {
    let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
    let mut look = vectorcraft_doc::Appearance::basic(Paint::solid(vectorcraft_color::Color::BLACK), Paint::None, 0.0);
    look.fill_mut().unwrap().opacity = 0.5;
    let rect = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
    let art = vectorcraft_doc::Node::path(d.alloc_id(), rect, look);
    d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Square".into(), art: std::sync::Arc::new(art) });
    let options = ColumnDesign { design: "Square".into(), column_type: ColumnType::Vertical, ..ColumnDesign::default() };
    let paints = vec![vectorcraft_doc::SeriesPaint { fill_opacity: Some(0.2), ..Default::default() }];
    let g = GraphSpec {
        rows: vec![vec![35.0]],
        axis_min: Some(0.0),
        axis_max: Some(50.0),
        series_columns: vec![Some(options)],
        series_paints: paints,
        ..GraphSpec::default()
    };
    let out = super::generate(&mut d, &g);
    let group = out.iter().find(|n| n.extras().series_index == Some(0)).unwrap();
    let mut fills = vec![];
    group.walk(&mut |n| {
        if matches!(n.kind, NodeKind::Path { .. }) {
            fills.push(n.appearance.fill().unwrap().opacity);
        }
    });
    assert!(!fills.is_empty() && fills.iter().all(|o| *o == 0.5), "{fills:?}");
}

/// An opacity set on an item that the graph does not draw again (a second fill added to a mark) is not kept.
#[test]
fn an_opacity_on_an_added_item_of_a_series_mark_is_not_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.execute("appearance.addFill", &json!({"ids": [bar.0]})).unwrap();
    let items = s.doc().unwrap().doc.node(bar).unwrap().appearance.items.clone();
    let lower = items.iter().position(vectorcraft_doc::AppearanceItem::is_fill).unwrap();
    assert!(items.iter().rposition(vectorcraft_doc::AppearanceItem::is_fill).unwrap() != lower);
    s.execute("transparency.set", &json!({"ids": [bar.0], "item": lower, "opacity": 40})).unwrap();
    assert!(spec(&s, id).series_paints.first().is_none_or(|p| p.fill_opacity.is_none()));
}

/// The fill positions (lowest, topmost) in a mark's appearance stack.
fn fill_rows(s: &Session, mark: NodeId) -> (usize, usize) {
    let items = s.doc().unwrap().doc.node(mark).unwrap().appearance.items.clone();
    let lower = items.iter().position(vectorcraft_doc::AppearanceItem::is_fill).unwrap();
    let upper = items.iter().rposition(vectorcraft_doc::AppearanceItem::is_fill).unwrap();
    assert!(lower < upper);
    (lower, upper)
}

/// Moving a translucent fill to the top of a series mark's stack, or placing a copy of it there, makes the graph keep
/// its opacity.
#[test]
fn moving_or_duplicating_a_series_marks_fill_to_the_top_keeps_its_opacity() {
    for cmd in ["appearance.moveItem", "appearance.duplicateItem"] {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
        s.execute("appearance.addFill", &json!({"ids": [bar.0]})).unwrap();
        let (lower, upper) = fill_rows(&s, bar);
        s.execute("transparency.set", &json!({"ids": [bar.0], "item": lower, "opacity": 40})).unwrap();
        assert!(spec(&s, id).series_paints.first().is_none_or(|p| p.fill_opacity.is_none()), "{cmd}");
        let p = if cmd == "appearance.moveItem" {
            json!({"ids": [bar.0], "from": lower, "to": upper})
        } else {
            json!({"ids": [bar.0], "index": lower, "to": upper + 1})
        };
        s.execute(cmd, &p).unwrap();
        let top = fill_rows(&s, bar).1;
        assert_eq!(s.doc().unwrap().doc.node(bar).unwrap().appearance.items[top].opacity(), 0.4, "{cmd}");
        assert_eq!(spec(&s, id).series_paints[0].fill_opacity, Some(0.4), "{cmd}");
    }
}

/// Moving or copying a translucent mark's appearance onto a mark of another series (the Layers panel's target
/// circle, or the Eyedropper) makes the graph keep the opacity for that series; a move leaves the source series with
/// none.
#[test]
fn moving_or_copying_an_appearance_onto_a_series_mark_keeps_its_opacity() {
    for (cmd, source_keeps) in [("move", None), ("copy", Some(0.4)), ("eyedropper", Some(0.4))] {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let (from, to) = (series(&n, 0).children().unwrap()[0].id, series(&n, 1).children().unwrap()[0].id);
        s.execute("transparency.set", &json!({"ids": [from.0], "item": 0, "opacity": 40})).unwrap();
        match cmd {
            "eyedropper" => s.execute("appearance.copyFrom", &json!({"source": from.0, "ids": [to.0]})).unwrap(),
            _ => s.execute("appearance.transfer", &json!({"source": from.0, "target": to.0, "copy": cmd == "copy"})).unwrap(),
        };
        let paints = spec(&s, id).series_paints;
        assert_eq!(paints.get(1).and_then(|p| p.fill_opacity), Some(0.4), "{cmd}");
        assert_eq!(paints[0].fill_opacity, source_keeps, "{cmd}");
    }
}

/// A line graph keeps a stroke opacity set on a series' line, and puts it on the line drawn again.
#[test]
fn a_stroke_opacity_on_a_series_line_survives_regeneration() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "line");
    let lines = |s: &Session| -> Vec<vectorcraft_doc::Node> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let marks = series(&n, 0).children().unwrap().to_vec();
        marks.iter().filter(|m| m.appearance.stroke().is_some_and(|st| st.width > 0.0)).map(|m| (**m).clone()).collect()
    };
    let line = lines(&s).first().unwrap().clone();
    let row = line.appearance.items.iter().rposition(|it| !it.is_fill()).unwrap();
    s.execute("transparency.set", &json!({"ids": [line.id.0], "item": row, "opacity": 60})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke_opacity, Some(0.6));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4], [2, 5]]})).unwrap();
    let after = lines(&s);
    assert!(!after.is_empty() && after.iter().all(|m| m.appearance.stroke().unwrap().opacity == 0.6));
}

/// Each mark of series `index`: its object opacity and blend mode, and the blend modes of its top fill and stroke.
fn transparency_of(s: &Session, id: NodeId, index: u32) -> Vec<(f32, BlendMode, BlendMode, BlendMode)> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let marks = series(&n, index).children().unwrap().to_vec();
    marks.iter().map(|m| (m.opacity, m.blend, m.appearance.fill().unwrap().blend, m.appearance.stroke().unwrap().blend)).collect()
}

/// An object opacity and blend mode set on one series mark with no fill or stroke row picked are kept by the graph:
/// every mark of that series has them after the graph is drawn again, through save and open, the other series stay
/// opaque and Normal, 100% and Normal forget them, and undo takes them back.
#[test]
fn an_object_opacity_and_blend_mode_on_a_series_mark_survive_regeneration() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("transparency.set", &json!({"ids": [marks[0].0], "opacity": 40, "blend": "multiply"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (Some(0.4), Some(BlendMode::Multiply)));
    let restored: GraphSpec = serde_json::from_value(serde_json::to_value(spec(&s, id)).unwrap()).unwrap();
    assert_eq!(restored, spec(&s, id));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let normal = BlendMode::Normal;
    assert!(transparency_of(&s, id, 0).iter().all(|t| *t == (0.4, BlendMode::Multiply, normal, normal)), "{:?}", transparency_of(&s, id, 0));
    assert!(transparency_of(&s, id, 1).iter().all(|t| *t == (1.0, normal, normal, normal)), "{:?}", transparency_of(&s, id, 1));
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.execute("transparency.set", &json!({"ids": [bar.0], "opacity": 100, "blend": "normal"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (None, None));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(spec(&s, id).series_paints.is_empty(), "undo takes back the edit and what the graph stored");
}

/// `object.setProps` with the opacity a mark already shows replaces the opacity another mark of the series left on the
/// graph, in one undo step, and an opacity on a graph design's copy in a series is not kept.
#[test]
fn an_object_opacity_a_series_mark_already_shows_replaces_the_kept_opacity() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("object.setProps", &json!({"id": marks[0].0, "opacity": 50})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.5));
    s.execute("object.setProps", &json!({"id": marks[1].0, "opacity": 100})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, None);
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(transparency_of(&s, id, 0).iter().all(|t| t.0 == 1.0), "{:?}", transparency_of(&s, id, 0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.5));
    // Setting it again where the graph already stores it leaves the document and its history as they are.
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("object.setProps", &json!({"id": marks[0].0, "opacity": 50})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);

    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    save_design(&mut s, "Square");
    s.execute("graph.column", &json!({"id": id.0, "design": "Square"})).unwrap();
    let copy = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].clone();
    assert!(matches!(copy.kind, NodeKind::Group { .. }));
    s.execute("object.setProps", &json!({"id": copy.id.0, "opacity": 50})).unwrap();
    assert!(spec(&s, id).series_paints.iter().all(|p| p.opacity.is_none()));
}

/// Randomize Opacity on marks of one series stores the opacity of the frontmost of them on the graph in the same undo
/// step, and every mark of the series has it when the graph is drawn again.
#[test]
fn a_random_opacity_on_series_marks_survives_regeneration() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("addon.arrange.randomizeOpacity", &json!({"ids": [marks[1].0, marks[0].0], "min": 30, "max": 60, "seed": 7})).unwrap();
    let front = s.doc().unwrap().doc.node(marks[1]).unwrap().opacity;
    assert!(front < 1.0);
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(front));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(transparency_of(&s, id, 0).iter().all(|t| t.0 == front), "{:?}", transparency_of(&s, id, 0));
    assert!(transparency_of(&s, id, 1).iter().all(|t| t.0 == 1.0), "{:?}", transparency_of(&s, id, 1));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(spec(&s, id).series_paints.is_empty(), "undo takes back the opacities and what the graph stored");
}

/// Sort Objects giving series marks new opacities stores the opacity of the frontmost of them on the graph, in place of
/// the one another mark left there.
#[test]
fn an_opacity_sort_objects_gives_series_marks_survives_regeneration() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("object.setProps", &json!({"id": marks[2].0, "opacity": 30})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.3));
    // From left to right the three columns get 30%, 65% and 100%: the frontmost, the rightmost, is fully opaque.
    let ids = [marks[0].0, marks[1].0, marks[2].0];
    s.execute("addon.arrange.sort", &json!({"ids": ids, "by": "x", "apply": "opacity", "distribute": true})).unwrap();
    let opacities: Vec<f32> = marks[..3].iter().map(|m| s.doc().unwrap().doc.node(*m).unwrap().opacity).collect();
    assert_eq!(opacities, [0.3, 0.65, 1.0]);
    assert_eq!(spec(&s, id).series_paints[0].opacity, None);
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(transparency_of(&s, id, 0).iter().all(|t| t.0 == 1.0), "{:?}", transparency_of(&s, id, 0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.3), "undo takes back the sort and what the graph stored");
}

/// Sort Objects stores the opacity of the frontmost series mark whose opacity it changes: a mark in front that keeps its
/// opacity leaves the stored value to the marks it changed.
#[test]
fn an_opacity_sort_objects_stores_the_frontmost_mark_it_changes() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("object.setProps", &json!({"id": marks[0].0, "opacity": 60})).unwrap();
    s.execute("object.setProps", &json!({"id": marks[1].0, "opacity": 30})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.3));
    // From left to right the columns trade 60%, 30% and 100% for 30%, 60% and 100%: the frontmost keeps its 100%.
    let ids = [marks[0].0, marks[1].0, marks[2].0];
    s.execute("addon.arrange.sort", &json!({"ids": ids, "by": "x", "apply": "opacity"})).unwrap();
    let opacities: Vec<f32> = marks[..3].iter().map(|m| s.doc().unwrap().doc.node(*m).unwrap().opacity).collect();
    assert_eq!(opacities, [0.3, 0.6, 1.0]);
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.6));
}

/// A Sort Objects that gives series marks the opacities they already have changes nothing: the value the graph stores
/// stays and no undo step is made, even when the evenly stepped percentage differs from a mark's in the last digits.
#[test]
fn an_opacity_sort_objects_that_changes_no_mark_keeps_the_stored_opacity() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("object.setProps", &json!({"id": marks[1].0, "opacity": 65})).unwrap();
    s.execute("object.setProps", &json!({"id": marks[0].0, "opacity": 30})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.3));
    // From left to right the columns have 30%, 65% and 100%, the values an even step from 30% to 100% gives them.
    let undo = s.doc().unwrap().history.undo.len();
    let ids = [marks[0].0, marks[1].0, marks[2].0];
    s.execute("addon.arrange.sort", &json!({"ids": ids, "by": "x", "apply": "opacity", "distribute": true})).unwrap();
    let opacities: Vec<f32> = marks[..3].iter().map(|m| s.doc().unwrap().doc.node(*m).unwrap().opacity).collect();
    assert_eq!(opacities, [0.3, 0.65, 1.0]);
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.3));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "no undo step");
}

/// An object opacity, blend mode, fill, stroke and weight that Global Edit gives a mark of another series from the mark
/// that was edited are stored on the graph for that series in the same undo step, and every mark of the series has them
/// when the graph is drawn again.
#[test]
fn what_global_edit_gives_series_marks_survives_regeneration() {
    let (mut s, id, _, _) = column_marks();
    // Equal values: the first column of each series has the same shape.
    s.execute("graph.setData", &json!({"rows": [[3, 3], [5, 2]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let (first, other) = (series(&n, 0).children().unwrap()[0].id, series(&n, 1).children().unwrap()[0].id);
    s.execute("object.isolate", &json!({"id": id.0})).unwrap();
    s.execute("select.set", &json!({"ids": [first.0]})).unwrap();
    let r = s.execute("select.globalEdit", &json!({})).unwrap();
    assert!(r["ids"].as_array().unwrap().contains(&json!(other.0)), "{r}");
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("transparency.set", &json!({"ids": [first.0], "opacity": 40, "blend": "multiply"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [first.0], "color": "#e8573f"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [first.0], "color": "#1f4e9c"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [first.0], "weight": 5})).unwrap();
    let mark = s.doc().unwrap().doc.node(other).unwrap().clone();
    assert_eq!((mark.opacity, mark.blend), (0.4, BlendMode::Multiply), "Global Edit reaches the other series' mark");
    let kept = spec(&s, id).series_paints[1].clone();
    assert_eq!((kept.opacity, kept.blend), (Some(0.4), Some(BlendMode::Multiply)));
    assert_eq!(kept.fill, Some(mark.appearance.fill_paint()));
    assert_eq!((kept.stroke, kept.stroke_width), (Some(mark.appearance.stroke_paint()), Some(5.0)));
    s.execute("object.exitIsolation", &json!({})).unwrap();
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let normal = BlendMode::Normal;
    assert!(transparency_of(&s, id, 1).iter().all(|t| *t == (0.4, BlendMode::Multiply, normal, normal)), "{:?}", transparency_of(&s, id, 1));
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(series(&n, 1).children().unwrap().iter().all(|m| m.appearance.fill_paint() == mark.appearance.fill_paint()));
    while s.doc().unwrap().history.undo.len() > steps {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert!(spec(&s, id).series_paints.iter().all(|p| *p == vectorcraft_doc::SeriesPaint::default()), "{:?}", spec(&s, id).series_paints);
}

/// With a whole series group picked and Global Edit on, the other series groups of the same form take the edit of its
/// marks, and the graph stores it for their series too.
#[test]
fn what_global_edit_gives_the_marks_of_another_series_group_survives_regeneration() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    // Series without names draw no legend, so each series group holds only its columns, laid out like the other's.
    let csv = ",,\nQ1,3,3\nQ2,5,5";
    let made = s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let (first, other) = (series(&n, 0).clone(), series(&n, 1).id);
    s.execute("object.isolate", &json!({"id": id.0})).unwrap();
    s.execute("select.set", &json!({"ids": [first.id.0]})).unwrap();
    let r = s.execute("select.globalEdit", &json!({})).unwrap();
    assert!(r["ids"].as_array().unwrap().contains(&json!(other.0)), "{r}");
    let marks: Vec<u64> = first.children().unwrap().iter().map(|m| m.id.0).collect();
    s.execute("paint.setFill", &json!({"ids": marks, "color": "#e8573f"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let given = series(&n, 1).children().unwrap()[0].appearance.fill_paint();
    assert_eq!(given, series(&n, 0).children().unwrap()[0].appearance.fill_paint(), "Global Edit reaches the other group");
    assert_eq!(spec(&s, id).series_paints[1].fill, Some(given.clone()));
    s.execute("object.exitIsolation", &json!({})).unwrap();
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(series(&n, 1).children().unwrap().iter().all(|m| m.appearance.fill_paint() == given));
}

/// With a whole series group picked and Global Edit on, the opacity and blend mode set on that group reach the other
/// series groups of the same form, and the graph stores them for those groups too.
#[test]
fn the_transparency_global_edit_gives_another_series_group_survives_regeneration() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    // Series without names draw no legend, so each series group holds only its columns, laid out like the other's.
    let csv = ",,\nQ1,3,3\nQ2,5,5";
    let made = s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let (first, other) = (series(&n, 0).id, series(&n, 1).id);
    s.execute("object.isolate", &json!({"id": id.0})).unwrap();
    s.execute("select.set", &json!({"ids": [first.0]})).unwrap();
    let r = s.execute("select.globalEdit", &json!({})).unwrap();
    assert!(r["ids"].as_array().unwrap().contains(&json!(other.0)), "{r}");
    s.execute("transparency.set", &json!({"ids": [first.0], "opacity": 40, "blend": "multiply"})).unwrap();
    let g = s.doc().unwrap().doc.node(other).unwrap().clone();
    assert_eq!((g.opacity, g.blend), (0.4, BlendMode::Multiply), "Global Edit reaches the other group");
    let kept = spec(&s, id).series_paints[1].clone();
    assert_eq!((kept.group_opacity, kept.group_blend), (Some(0.4), Some(BlendMode::Multiply)));
    s.execute("object.exitIsolation", &json!({})).unwrap();
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert_eq!(group_transparency(&s, id, 1).0, (0.4, BlendMode::Multiply));
}

/// A blend mode set on the top fill or stroke row of one series mark is kept by the graph and drawn on every mark of
/// the series again; Normal forgets it.
#[test]
fn a_row_blend_mode_on_a_series_mark_survives_regeneration() {
    let (mut s, id, _, marks) = column_marks();
    let (fill, stroke) = (top_row(&s, marks[0], true), top_row(&s, marks[0], false));
    s.execute("transparency.set", &json!({"ids": [marks[0].0], "item": fill, "blend": "screen"})).unwrap();
    s.execute("transparency.set", &json!({"ids": [marks[1].0], "item": stroke, "blend": "darken"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_blend, kept.stroke_blend), (Some(BlendMode::Screen), Some(BlendMode::Darken)));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let normal = BlendMode::Normal;
    assert!(transparency_of(&s, id, 0).iter().all(|t| *t == (1.0, normal, BlendMode::Screen, BlendMode::Darken)), "{:?}", transparency_of(&s, id, 0));
    assert!(transparency_of(&s, id, 1).iter().all(|t| *t == (1.0, normal, normal, normal)), "{:?}", transparency_of(&s, id, 1));
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.execute("transparency.set", &json!({"ids": [bar.0], "item": top_row(&s, bar, true), "blend": "normal"})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_blend, None);
}

/// The graph stores an object's or a row's opacity and blend mode as one pair from the mark just edited: setting only
/// one half on another mark replaces both, either way round, and the redraw and undo follow the stored pair.
#[test]
fn setting_half_of_a_series_marks_transparency_stores_that_marks_whole_pair() {
    let normal = BlendMode::Normal;
    let (mut s, id, _, marks) = column_marks();
    s.execute("transparency.set", &json!({"ids": [marks[0].0], "blend": "multiply"})).unwrap();
    s.execute("object.setProps", &json!({"id": marks[1].0, "opacity": 50})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (Some(0.5), None));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(transparency_of(&s, id, 0).iter().all(|t| (t.0, t.1) == (0.5, normal)), "{:?}", transparency_of(&s, id, 0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (None, Some(BlendMode::Multiply)));

    let (mut s, id, _, marks) = column_marks();
    s.execute("object.setProps", &json!({"id": marks[0].0, "opacity": 30})).unwrap();
    s.execute("transparency.set", &json!({"ids": [marks[1].0], "blend": "screen"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (None, Some(BlendMode::Screen)));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(transparency_of(&s, id, 0).iter().all(|t| (t.0, t.1) == (1.0, BlendMode::Screen)), "{:?}", transparency_of(&s, id, 0));

    // A fill row: a blend alone on one mark replaces the opacity another mark's row left, and the other way round.
    let (mut s, id, _, marks) = column_marks();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "opacity": 40})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": fill, "blend": "darken"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.fill_blend), (None, Some(BlendMode::Darken)));
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "opacity": 60})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.fill_blend), (Some(0.6), None));
    s.execute("edit.undo", &json!({})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.fill_blend), (None, Some(BlendMode::Darken)));
}

/// Clear Appearance on a series mark sets its object opacity back to 100% and Normal, and the graph keeps neither.
#[test]
fn clearing_a_series_marks_appearance_forgets_its_kept_object_opacity() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("transparency.set", &json!({"ids": [marks[0].0], "opacity": 40, "blend": "multiply"})).unwrap();
    s.execute("appearance.clear", &json!({"ids": [marks[1].0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (None, None));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.4));
}

/// An appearance moved or copied onto a mark of another series takes the source's object opacity and blend mode, and
/// the graph keeps them for the target's series; a move leaves the source's series opaque and Normal.
#[test]
fn moving_an_appearance_onto_a_series_mark_keeps_its_object_opacity() {
    for copy in [false, true] {
        let (mut s, id, _, marks) = column_marks();
        let to = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
        s.execute("transparency.set", &json!({"ids": [marks[0].0], "opacity": 40, "blend": "multiply"})).unwrap();
        s.execute("appearance.transfer", &json!({"source": marks[0].0, "target": to.0, "copy": copy})).unwrap();
        let paints = spec(&s, id).series_paints;
        assert_eq!((paints[1].opacity, paints[1].blend), (Some(0.4), Some(BlendMode::Multiply)), "copy {copy}");
        let source = if copy { (Some(0.4), Some(BlendMode::Multiply)) } else { (None, None) };
        assert_eq!((paints[0].opacity, paints[0].blend), source, "copy {copy}");
    }
}

/// An Eyedropper pick of the object's transparency onto a series mark makes the graph keep the picked opacity and blend
/// mode for that series, even an opaque Normal pick onto a mark that already shows it.
#[test]
fn an_eyedropper_pick_of_object_transparency_onto_a_series_mark_is_kept() {
    let (mut s, id, _, marks) = column_marks();
    let to = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    s.execute("transparency.set", &json!({"ids": [marks[0].0], "opacity": 40, "blend": "multiply"})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": marks[0].0, "ids": [to.0]})).unwrap();
    let kept = spec(&s, id).series_paints[1].clone();
    assert_eq!((kept.opacity, kept.blend), (Some(0.4), Some(BlendMode::Multiply)));
    // An opaque Normal source picked onto another opaque mark of series 0 replaces what series 0 stores.
    let opaque = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[1].id;
    s.execute("appearance.copyFrom", &json!({"source": opaque.0, "ids": [marks[1].0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (None, None));
}

/// A graphic style made from an object at 50% Multiply, applied to a series mark, makes the graph keep the style's object
/// opacity and blend mode; redefining the style updates them for a mark still linked to it.
#[test]
fn a_graphic_styles_object_opacity_on_a_series_mark_is_kept() {
    let (mut s, id, _, marks) = column_marks();
    let (rect, _) = blue_style(&mut s, "Blue", true);
    s.execute("object.setProps", &json!({"id": rect.0, "opacity": 50, "blend": "multiply"})).unwrap();
    s.execute("graphicStyle.new", &json!({"id": rect.0, "name": "Faded"})).unwrap();
    s.execute("graphicStyle.apply", &json!({"name": "Faded", "ids": [marks[0].0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (Some(0.5), Some(BlendMode::Multiply)));
    s.execute("object.setProps", &json!({"id": rect.0, "opacity": 100, "blend": "normal"})).unwrap();
    s.execute("graphicStyle.redefine", &json!({"name": "Faded", "id": rect.0})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.opacity, kept.blend), (None, None));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, Some(0.5), "one undo takes back the redefinition and what the graph stored");

    // An opaque Normal style applied to an opaque mark replaces the opacity another mark of the series left.
    let (mut s, id, _, marks) = column_marks();
    s.execute("object.setProps", &json!({"id": marks[0].0, "opacity": 40})).unwrap();
    blue_style(&mut s, "Blue", true);
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [marks[1].0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].opacity, None);
}

/// The colors of series `index`'s marks: each mark's top fill and stroke paint and its stroke weight.
fn marks_of(s: &Session, id: NodeId, index: u32) -> Vec<(Paint, Paint, f64)> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let marks = series(&n, index).children().unwrap().to_vec();
    marks.iter().map(|m| (m.appearance.fill_paint(), m.appearance.stroke_paint(), m.appearance.stroke_width())).collect()
}

/// Set the top fill of series 0's first mark to the color `hex` with `appearance.setItem`, and return that paint.
fn hex_paint(s: &mut Session, id: NodeId, hex: &str) -> Paint {
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    let row = s.doc().unwrap().doc.node(bar).unwrap().appearance.items.iter().rposition(vectorcraft_doc::AppearanceItem::is_fill).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": row, "color": hex})).unwrap();
    s.doc().unwrap().doc.node(bar).unwrap().appearance.fill_paint()
}

/// A color or weight set with `appearance.setItem` on the topmost fill or stroke of one series mark is kept by the
/// graph: every mark of that series has it after the graph is drawn again, and undo takes it back with its edit. A row
/// under the topmost one is not kept, and a change to one kind of row keeps nothing for the other.
#[test]
fn an_appearance_item_paint_on_a_series_mark_survives_regeneration() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let blue = hex_paint(&mut s, id, "#0000ff");
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.stroke, kept.stroke_width), (Some(blue.clone()), None, None));
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    let row = s.doc().unwrap().doc.node(bar).unwrap().appearance.items.iter().rposition(|it| !it.is_fill()).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": row, "color": "#ff0000", "weight": 3})).unwrap();
    let red = s.doc().unwrap().doc.node(bar).unwrap().appearance.stroke_paint();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.stroke, kept.stroke_width), (Some(red.clone()), Some(3.0)));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| *m == (blue.clone(), red.clone(), 3.0)), "{:?}", marks_of(&s, id, 0));
    assert!(marks_of(&s, id, 1).iter().all(|m| m.0 != blue));
    // Undo goes back through the data edit, then takes the stroke paint away with its edit.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.stroke, kept.stroke_width), (Some(blue.clone()), None, None));
    // The graph does not keep a color set on a fill under the topmost one.
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.execute("appearance.addFill", &json!({"ids": [bar.0]})).unwrap();
    let (lower, _) = fill_rows(&s, bar);
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": lower, "color": "#00ff00"})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(blue));
}

/// A weight set on a stroke row, with no color, is kept by the graph with the row's paint.
#[test]
fn an_appearance_item_weight_on_a_series_mark_survives_regeneration() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].clone();
    let row = bar.appearance.items.iter().rposition(|it| !it.is_fill()).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [bar.id.0], "index": row, "color": "#ff0000"})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [bar.id.0], "index": row, "weight": 4})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke_width, Some(4.0));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| m.2 == 4.0), "{:?}", marks_of(&s, id, 0));
}

/// A column graph with series 0 kept blue at 40% opacity: the graph, the paint, the first marks of series 0 and 1.
fn blue_source() -> (Session, NodeId, Paint, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let blue = hex_paint(&mut s, id, "#0000ff");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let (from, to) = (series(&n, 0).children().unwrap()[0].id, series(&n, 1).children().unwrap()[0].id);
    s.execute("transparency.set", &json!({"ids": [from.0], "item": 0, "opacity": 40})).unwrap();
    (s, id, blue, from, to)
}

/// After the graph is drawn again, every mark of series 1 is `blue` at 40%; undo goes back through the data edit and the
/// command before it, which restores the paints the graph kept in `was`.
fn series_1_is_blue_at_40_then_undo(s: &mut Session, id: NodeId, blue: &Paint, was: &[vectorcraft_doc::SeriesPaint]) {
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    for m in series(&n, 1).children().unwrap() {
        let f = m.appearance.fill().unwrap();
        assert_eq!((f.paint.clone(), f.opacity), (blue.clone(), 0.4));
    }
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(s, id).series_paints, was);
}

/// The topmost fill (`true`) or stroke (`false`) row of `mark`.
fn top_row(s: &Session, mark: NodeId, fill: bool) -> usize {
    s.doc().unwrap().doc.node(mark).unwrap().appearance.items.iter().rposition(|it| it.is_fill() == fill).unwrap()
}

/// A new column graph, the fill of series 0's marks and those marks' ids.
fn column_marks() -> (Session, NodeId, Paint, Vec<NodeId>) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let marks = series(&n, 0).children().unwrap().to_vec();
    (s, id, marks[0].appearance.fill_paint(), marks.iter().map(|m| m.id).collect())
}

/// An appearance moved onto a mark of another series makes the graph keep its paint with its opacity for that series,
/// and leaves the source series without paint.
#[test]
fn moving_an_appearance_onto_a_series_mark_keeps_its_paint() {
    let (mut s, id, blue, from, to) = blue_source();
    let was = spec(&s, id).series_paints;
    s.execute("appearance.transfer", &json!({"source": from.0, "target": to.0})).unwrap();
    let paints = spec(&s, id).series_paints;
    assert_eq!((paints[0].fill.clone(), paints[1].fill.clone()), (Some(Paint::None), Some(blue.clone())));
    series_1_is_blue_at_40_then_undo(&mut s, id, &blue, &was);
}

/// An appearance copied onto a mark of another series makes the graph keep its paint with its opacity for that series.
#[test]
fn copying_an_appearance_onto_a_series_mark_keeps_its_paint() {
    let (mut s, id, blue, from, to) = blue_source();
    let was = spec(&s, id).series_paints;
    s.execute("appearance.transfer", &json!({"source": from.0, "target": to.0, "copy": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[1].fill, Some(blue.clone()));
    series_1_is_blue_at_40_then_undo(&mut s, id, &blue, &was);
}

/// An Eyedropper pick onto a mark of another series makes the graph keep its paint with its opacity for that series.
#[test]
fn an_eyedropper_pick_onto_a_series_mark_keeps_its_paint() {
    let (mut s, id, blue, from, to) = blue_source();
    let was = spec(&s, id).series_paints;
    s.execute("appearance.copyFrom", &json!({"source": from.0, "ids": [to.0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[1].fill, Some(blue.clone()));
    series_1_is_blue_at_40_then_undo(&mut s, id, &blue, &was);
}

/// A reversed Eyedropper (Alt-click) copies onto the clicked mark, and the graph keeps the paint for the clicked series.
#[test]
fn a_reversed_eyedropper_pick_keeps_its_paint_on_the_clicked_series() {
    let (mut s, id, blue, from, to) = blue_source();
    let was = spec(&s, id).series_paints;
    s.execute("appearance.copyFrom", &json!({"source": to.0, "ids": [from.0], "reverse": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[1].fill, Some(blue.clone()));
    series_1_is_blue_at_40_then_undo(&mut s, id, &blue, &was);
}

/// A reversed Eyedropper with nothing selected copies what it picked up last onto the clicked mark.
#[test]
fn a_reversed_eyedropper_pick_of_the_last_sample_keeps_its_paint() {
    let (mut s, id, blue, from, to) = blue_source();
    s.execute("appearance.copyFrom", &json!({"source": from.0, "ids": []})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": to.0, "reverse": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[1].fill, Some(blue));
}

/// Clear Appearance on a series mark leaves it with no fill and no stroke, and the series stays that way when the graph
/// is drawn again. Undo brings the kept paint back.
#[test]
fn clearing_a_series_marks_appearance_updates_its_kept_paint() {
    let (mut s, id, _, marks) = column_marks();
    let row = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": row, "color": "#ff0000", "weight": 2})).unwrap();
    let was = spec(&s, id).series_paints;
    s.execute("appearance.clear", &json!({"ids": [marks[0].0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.stroke, kept.stroke_width), (Some(Paint::None), Some(Paint::None), Some(0.0)));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| *m == (Paint::None, Paint::None, 0.0)), "{:?}", marks_of(&s, id, 0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);
}

/// A mark with `blue` under a second fill: the graph, the paint, the mark and its (lower, upper) fill rows, with the
/// upper row green and kept.
fn two_fills() -> (Session, NodeId, Paint, NodeId, usize, usize) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "column");
    let blue = hex_paint(&mut s, id, "#0000ff");
    let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.execute("appearance.addFill", &json!({"ids": [bar.0]})).unwrap();
    let (lower, upper) = fill_rows(&s, bar);
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": upper, "color": "#00ff00"})).unwrap();
    assert_ne!(spec(&s, id).series_paints[0].fill, Some(blue.clone()));
    (s, id, blue, bar, lower, upper)
}

/// `cmd` makes the graph keep `blue`, and undo brings back the paints it kept before.
fn top_row_command_keeps_blue(s: &mut Session, id: NodeId, blue: &Paint, cmd: &str, p: &serde_json::Value) {
    let was = spec(s, id).series_paints;
    s.execute(cmd, p).unwrap();
    assert_eq!(spec(s, id).series_paints[0].fill.as_ref(), Some(blue), "{cmd}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(s, id).series_paints, was, "{cmd}");
}

/// Reduce to Basic Appearance with the lower fill as the active row leaves that fill, and the graph keeps its paint.
#[test]
fn reducing_a_series_marks_appearance_keeps_the_row_it_leaves() {
    let (mut s, id, blue, bar, lower, _) = two_fills();
    s.execute("select.set", &json!({"ids": [bar.0]})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": lower})).unwrap();
    top_row_command_keeps_blue(&mut s, id, &blue, "appearance.reduceToBasic", &json!({}));
}

/// Removing a mark's top row makes the graph keep the paint now on top.
#[test]
fn removing_a_series_marks_top_row_keeps_the_paint_now_on_top() {
    let (mut s, id, blue, bar, _, upper) = two_fills();
    top_row_command_keeps_blue(&mut s, id, &blue, "appearance.removeItem", &json!({"ids": [bar.0], "index": upper}));
}

/// Moving a lower row above the others makes the graph keep its paint.
#[test]
fn moving_a_row_to_a_series_marks_top_keeps_its_paint() {
    let (mut s, id, blue, bar, lower, upper) = two_fills();
    top_row_command_keeps_blue(&mut s, id, &blue, "appearance.moveItem", &json!({"ids": [bar.0], "from": lower, "to": upper}));
}

/// Moving a block of rows above the others makes the graph keep the paint now on top.
#[test]
fn moving_a_block_of_rows_to_a_series_marks_top_keeps_its_paint() {
    let (mut s, id, blue, bar, lower, upper) = two_fills();
    top_row_command_keeps_blue(&mut s, id, &blue, "appearance.moveItem", &json!({"ids": [bar.0], "indices": [lower], "to": upper}));
}

/// A copy of a lower row placed on top makes the graph keep its paint.
#[test]
fn a_copy_placed_on_a_series_marks_top_keeps_its_paint() {
    let (mut s, id, blue, bar, lower, upper) = two_fills();
    top_row_command_keeps_blue(&mut s, id, &blue, "appearance.duplicateItem", &json!({"ids": [bar.0], "index": lower, "to": upper + 1}));
}

/// The mark's only stroke row removed: the series draws without a stroke; a new row comes back as its paint.
#[test]
fn removing_and_adding_a_series_marks_only_stroke_updates_its_kept_paint() {
    let (mut s, id, _, marks) = column_marks();
    let bar = marks[0];
    let row = top_row(&s, bar, false);
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": row, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.removeItem", &json!({"ids": [bar.0], "index": row})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.stroke, kept.stroke_width), (None, Some(Paint::None), Some(0.0)));
    let was = spec(&s, id).series_paints;
    s.execute("appearance.addStroke", &json!({"ids": [bar.0]})).unwrap();
    let stroke = s.doc().unwrap().doc.node(bar).unwrap().appearance.stroke_paint();
    assert!(!stroke.is_none());
    assert_eq!(spec(&s, id).series_paints[0].stroke, Some(stroke));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);
}

/// Hiding the top row makes the graph keep the visible row under it, and its opacity; hiding the only one keeps no paint;
/// Show All Hidden Attributes brings the top row back. A hidden row on top of a moved appearance is not kept.
#[test]
fn a_hidden_top_row_is_not_what_the_graph_keeps() {
    let (mut s, id, blue, bar, lower, upper) = two_fills();
    let green = s.doc().unwrap().doc.node(bar).unwrap().appearance.fill_paint();
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": lower, "opacity": 50})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, None, "the row under the top one");
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": upper, "visible": false})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.fill_opacity), (Some(blue.clone()), Some(0.5)));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| m.0 == blue), "{:?}", marks_of(&s, id, 0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("appearance.showAllHidden", &json!({"ids": [bar.0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.fill_opacity), (Some(green), None));

    // A transfer of [blue, hidden green] onto another series keeps the blue the source shows.
    let (mut s, id, blue, bar, _, upper) = two_fills();
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": upper, "visible": false})).unwrap();
    let to = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    s.execute("appearance.transfer", &json!({"source": bar.0, "target": to.0, "copy": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[1].fill, Some(blue));

    let (mut s, id, _, marks) = column_marks();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": top_row(&s, marks[0], true), "visible": false})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(Paint::None));
}

/// Hiding a top row and Show All Hidden Attributes each undo in one step, with what the graph stores.
#[test]
fn hiding_and_showing_a_series_marks_top_row_undo_with_the_stored_paint() {
    let (mut s, id, _, bar, _, upper) = two_fills();
    let was = spec(&s, id).series_paints;
    s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": upper, "visible": false})).unwrap();
    let hidden = spec(&s, id).series_paints;
    assert_ne!(hidden, was);
    s.execute("appearance.showAllHidden", &json!({"ids": [bar.0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, hidden);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);
}

/// An opaque Eyedropper pick onto a mark replaces the opacity another mark of the series stored, even though the mark
/// itself stays opaque: the series is drawn with the picked color at 100%.
#[test]
fn an_opaque_pick_replaces_the_opacity_the_series_stores() {
    let (mut s, id, _, marks) = column_marks();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "opacity": 40})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, Some(0.4));
    let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    s.execute("appearance.setItem", &json!({"ids": [source.0], "index": top_row(&s, source, true), "color": "#0000ff"})).unwrap();
    let was = spec(&s, id).series_paints;
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, None);
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(series(&n, 0).children().unwrap().iter().all(|m| m.appearance.fill().unwrap().opacity == 1.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);

    // A pick of the opacity alone replaces it too, and a pick that leaves opacity out keeps it.
    let (mut s, id, _, marks) = column_marks();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "opacity": 40})).unwrap();
    let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    let colors_only = json!({"appearance": {"fill": {"color": true, "transparency": false}, "stroke": {"color": false}}});
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0], "pickUp": colors_only})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, Some(0.4));
    let opacity_only = json!({"appearance": {"fill": {"color": false, "transparency": true}, "stroke": {"color": false}}});
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0], "pickUp": opacity_only})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, None);
}

/// A column graph whose series 0 stores the 40% Multiply fill set on its first mark, and a mark of series 1 whose top fill
/// row has no paint and is opaque and Normal: the session, the graph, series 0's marks and that source.
fn marks_and_unpainted_source() -> (Session, NodeId, Vec<NodeId>, NodeId) {
    let (mut s, id, _, marks) = column_marks();
    let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    s.execute("appearance.setItem", &json!({"ids": [source.0], "index": top_row(&s, source, true), "none": true})).unwrap();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "opacity": 40, "blend": "multiply"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.fill_blend), (Some(0.4), Some(BlendMode::Multiply)));
    (s, id, marks, source)
}

/// An Eyedropper pick from a fill row without paint onto a mark whose fill had paint replaces the fill opacity and blend
/// mode another mark of the series stored: the series is drawn opaque and Normal again.
#[test]
fn an_eyedropper_pick_that_unpaints_a_series_fill_clears_its_stored_transparency() {
    let (mut s, id, marks, source) = marks_and_unpainted_source();
    let mark = s.doc().unwrap().doc.node(marks[1]).unwrap().appearance.fill().unwrap().clone();
    assert_eq!((mark.paint.is_none(), mark.opacity, mark.blend), (false, 1.0, BlendMode::Normal), "the target's fill row");
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0]})).unwrap();
    let picked = s.doc().unwrap().doc.node(marks[1]).unwrap().appearance.fill().unwrap().clone();
    assert_eq!((picked.paint.is_none(), picked.opacity, picked.blend), (true, 1.0, BlendMode::Normal));
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.fill_blend), (None, None));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let fills: Vec<_> = series(&n, 0).children().unwrap().iter().filter_map(|m| m.appearance.fill().map(|f| (f.opacity, f.blend))).collect();
    assert!(fills.iter().all(|f| *f == (1.0, BlendMode::Normal)), "{fills:?}");
}

/// An Eyedropper pick from a fill row without paint onto a mark whose fill had no paint before the pick either leaves the
/// fill opacity and blend mode another mark of the series stored.
#[test]
fn an_eyedropper_pick_of_an_already_unpainted_row_leaves_series_transparency() {
    let (mut s, id, marks, source) = marks_and_unpainted_source();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": top_row(&s, marks[1], true), "none": true})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.fill_blend), (Some(0.4), Some(BlendMode::Multiply)), "a color set to None writes no opacity");
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill_opacity, kept.fill_blend), (Some(0.4), Some(BlendMode::Multiply)));
}

/// A whole Eyedropper pick from a source with no fill row, or no stroke row, removes that row from the mark. Where the
/// mark's row had paint, the pick replaces the opacity and blend mode another mark of the series stored for that kind
/// of row. Where the mark's row had no paint, the stored values stay.
#[test]
fn a_whole_eyedropper_pick_that_removes_a_painted_row_clears_its_stored_transparency() {
    let stored = |s: &Session, id: NodeId, fill: bool| {
        let kept = spec(s, id).series_paints[0].clone();
        if fill { (kept.fill_opacity, kept.fill_blend) } else { (kept.stroke_opacity, kept.stroke_blend) }
    };
    for (fill, painted) in [(true, true), (false, true), (false, false)] {
        let (mut s, id, _, marks) = column_marks();
        let row = top_row(&s, marks[0], fill);
        if painted {
            s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": row, "color": "#ff0000"})).unwrap();
        }
        s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": row, "color": "#ff0000", "opacity": 40, "blend": "multiply"})).unwrap();
        assert_eq!(stored(&s, id, fill), (Some(0.4), Some(BlendMode::Multiply)), "fill {fill}, painted {painted}");
        let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
        s.execute("appearance.removeItem", &json!({"ids": [source.0], "index": top_row(&s, source, fill)})).unwrap();
        let was = spec(&s, id).series_paints;
        s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0]})).unwrap();
        let mark = s.doc().unwrap().doc.node(marks[1]).unwrap().clone();
        assert!(!mark.appearance.items.iter().any(|it| it.is_fill() == fill), "the pick removed the row: fill {fill}");
        let want = if painted { (None, None) } else { (Some(0.4), Some(BlendMode::Multiply)) };
        assert_eq!(stored(&s, id, fill), want, "fill {fill}, painted {painted}");
        if painted {
            let after = spec(&s, id).series_paints;
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(spec(&s, id).series_paints, was);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(spec(&s, id).series_paints, after);
        }
    }
}

/// An appearance moved between two marks of one series leaves the series with the target's paint, not the source's
/// empty one, and the series still shows when the graph is drawn again.
#[test]
fn moving_an_appearance_within_a_series_keeps_the_targets_paint() {
    let (mut s, id, shown, marks) = column_marks();
    s.execute("appearance.transfer", &json!({"source": marks[0].0, "target": marks[1].0})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(shown.clone()));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| m.0 == shown), "{:?}", marks_of(&s, id, 0));
}

/// A row shown again with the color the mark already shows replaces the paint another mark of the series stored.
#[test]
fn showing_a_row_with_the_color_a_mark_shows_replaces_the_kept_paint() {
    let (mut s, id, shown, marks) = column_marks();
    s.execute("appearance.addFill", &json!({"ids": [marks[0].0]})).unwrap();
    let (_, upper) = fill_rows(&s, marks[0]);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": upper, "visible": false})).unwrap();
    let other = top_row(&s, marks[1], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": other, "color": "#ff0000"})).unwrap();
    assert_ne!(spec(&s, id).series_paints[0].fill, Some(shown.clone()));
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": upper, "visible": true, "color": "#000000"})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.fill_paint()));
}

/// A color or weight set again to what a mark already shows and an Eyedropper pick that leaves the mark as it was replace
/// the paint another mark of the series stored, in an undo step of their own. Clear Appearance on a clear mark leaves the
/// stored paint and the undo history as they are.
#[test]
fn writing_the_paint_a_series_mark_already_shows_replaces_the_kept_paint() {
    let (mut s, id, shown, marks) = column_marks();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "color": "#ff0000"})).unwrap();
    assert_ne!(spec(&s, id).series_paints[0].fill, Some(shown.clone()));
    let was = spec(&s, id).series_paints;
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": fill, "color": "#000000"})).unwrap();
    let again = s.doc().unwrap().doc.node(marks[1]).unwrap().appearance.fill_paint();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(again));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);

    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[0], false);
    for (mark, weight) in [(marks[1], 2), (marks[0], 5), (marks[1], 2)] {
        s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": stroke, "color": "#ff0000", "weight": weight})).unwrap();
    }
    assert_eq!(spec(&s, id).series_paints[0].stroke_width, Some(2.0));
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "weight": 5})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "weight": 2})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke_width, Some(2.0));

    let (mut s, id, shown, marks) = column_marks();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "color": "#ff0000"})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": marks[2].0, "ids": [marks[1].0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(shown));

    let (mut s, id, _, marks) = column_marks();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.clear", &json!({"ids": [marks[1].0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "color": "#ff0000"})).unwrap();
    let was = spec(&s, id).series_paints;
    let shown = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.fill_paint();
    s.execute("appearance.clear", &json!({"ids": [marks[1].0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_ne!(s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.fill_paint(), shown, "the undo took back the color, not a clear");
}

/// A line graph with a marker: the graph, the first thin line, a marker.
fn line_and_marker() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "line");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let marks = series(&n, 0).children().unwrap().to_vec();
    let line = marks.iter().find(|m| m.appearance.stroke_width() > 0.0).unwrap().id;
    let marker = marks.iter().find(|m| m.appearance.stroke_width() == 0.0 && m.appearance.stroke().is_some()).unwrap().id;
    (s, id, line, marker)
}

/// The first thin line of series 0, and every other path of the series (its markers and legend swatch).
fn line_parts(s: &Session, id: NodeId) -> (vectorcraft_doc::Node, Vec<vectorcraft_doc::Node>) {
    let parts = series_parts(s, id, 0);
    let (lines, others): (Vec<_>, Vec<_>) =
        parts.into_iter().partition(|m| m.appearance.fill_paint().is_none() && m.appearance.stroke_width() == 1.0);
    (lines[0].clone(), others)
}

/// On a line graph, a stroke set on the thin line goes on the line after a redraw; a fill set on the thin line goes on
/// the markers and the legend swatch, and the line stays without a fill.
#[test]
fn a_line_series_draws_its_kept_stroke_on_the_line_and_its_kept_fill_on_the_markers() {
    let (mut s, id, line, _) = line_and_marker();
    s.execute("appearance.setItem", &json!({"ids": [line.0], "index": top_row(&s, line, false), "color": "#ff0000"})).unwrap();
    let red = s.doc().unwrap().doc.node(line).unwrap().appearance.stroke_paint();
    s.execute("appearance.setItem", &json!({"ids": [line.0], "index": top_row(&s, line, true), "color": "#0000ff"})).unwrap();
    let blue = s.doc().unwrap().doc.node(line).unwrap().appearance.fill_paint();
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let (drawn, others) = line_parts(&s, id);
    assert_eq!((drawn.appearance.fill_paint(), drawn.appearance.stroke_paint()), (Paint::None, red));
    assert!(others.iter().all(|m| m.appearance.fill_paint() == blue), "{others:?}");
}

/// Clear Appearance on a thin line leaves the fill the graph stores for the markers; on a marker that shows no stroke, it
/// leaves the stroke the graph stores for the line. An Eyedropper pick of a stroke-only source onto the line and a copy
/// of one line onto another leave the stored fill too.
#[test]
fn a_row_left_without_paint_leaves_the_paint_the_series_stores() {
    let (mut s, id, line, marker) = line_and_marker();
    s.execute("appearance.setItem", &json!({"ids": [marker.0], "index": top_row(&s, marker, true), "color": "#0000ff"})).unwrap();
    let blue = spec(&s, id).series_paints[0].fill.clone().unwrap();
    s.execute("appearance.clear", &json!({"ids": [line.0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill.as_ref(), kept.stroke.as_ref()), (Some(&blue), Some(&Paint::None)));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    let parts = series_parts(&s, id, 0);
    assert!(parts.iter().filter(|m| !m.appearance.fill_paint().is_none()).all(|m| m.appearance.fill_paint() == blue));
    assert!(parts.iter().any(|m| m.appearance.fill_paint() == blue), "the markers keep their fill");

    let (mut s, id, line, marker) = line_and_marker();
    s.execute("appearance.setItem", &json!({"ids": [line.0], "index": top_row(&s, line, false), "color": "#ff0000"})).unwrap();
    let red = s.doc().unwrap().doc.node(line).unwrap().appearance.stroke_paint();
    s.execute("appearance.clear", &json!({"ids": [marker.0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke, Some(red.clone()), "the marker showed no stroke");
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert_eq!(line_parts(&s, id).0.appearance.stroke_paint(), red);

    let (mut s, id, line, marker) = line_and_marker();
    s.execute("appearance.setItem", &json!({"ids": [marker.0], "index": top_row(&s, marker, true), "color": "#0000ff"})).unwrap();
    let blue = spec(&s, id).series_paints[0].fill.clone().unwrap();
    let other = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    s.execute("appearance.copyFrom", &json!({"source": other.0, "ids": [line.0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(blue.clone()), "the source's line has no fill");
    s.execute("appearance.transfer", &json!({"source": other.0, "target": line.0, "copy": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(blue));
}

/// A line graph with Filled Lines on: the session, the graph and series 0's band.
fn band_graph() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "line", ",a\nQ1,2\nQ2,3\nQ3,2");
    s.execute("graph.setType", &json!({"id": id.0, "filledLines": true})).unwrap();
    let band = series_parts(&s, id, 0)[0].id;
    (s, id, band)
}

/// The band's fill, outline paint and outline weight after the graph is drawn again.
fn redrawn_band(s: &mut Session, id: NodeId) -> (Paint, Paint, f64) {
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[2], [3], [4]]})).unwrap();
    let band = series_parts(s, id, 0)[0].clone();
    (band.appearance.fill_paint(), band.appearance.stroke_paint(), band.appearance.stroke_width())
}

/// A fill set on a Filled Lines band is kept, and the band keeps its thin black outline.
#[test]
fn a_filled_line_band_keeps_a_fill_set_on_it() {
    let (mut s, id, band) = band_graph();
    s.execute("appearance.setItem", &json!({"ids": [band.0], "index": top_row(&s, band, true), "color": "#0000ff"})).unwrap();
    let blue = s.doc().unwrap().doc.node(band).unwrap().appearance.fill_paint();
    assert_eq!(redrawn_band(&mut s, id), (blue, Paint::solid(vectorcraft_color::Color::BLACK), 0.25));
}

/// A stroke set on a Filled Lines band is not kept: the band keeps its color and its thin black outline.
#[test]
fn a_stroke_set_on_a_filled_line_band_does_not_repaint_it() {
    let (mut s, id, band) = band_graph();
    let was = series_parts(&s, id, 0)[0].appearance.fill_paint();
    let row = top_row(&s, band, false);
    s.execute("appearance.setItem", &json!({"ids": [band.0], "index": row, "color": "#ff0000", "weight": 3})).unwrap();
    assert!(spec(&s, id).series_paints.first().is_none_or(|p| p.stroke.is_none()));
    assert_eq!(redrawn_band(&mut s, id), (was, Paint::solid(vectorcraft_color::Color::BLACK), 0.25));
}

/// An opacity and blend mode set on a Filled Lines band's stroke row are kept and drawn on its outline, which keeps its
/// black paint and 0.25 pt weight.
#[test]
fn a_filled_line_bands_outline_keeps_a_stroke_opacity_and_blend_mode() {
    let (mut s, id, band) = band_graph();
    let row = top_row(&s, band, false);
    s.execute("appearance.setItem", &json!({"ids": [band.0], "index": row, "opacity": 50, "blend": "multiply"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.stroke, kept.stroke_opacity, kept.stroke_blend), (None, Some(0.5), Some(BlendMode::Multiply)));
    let (_, outline, width) = redrawn_band(&mut s, id);
    assert_eq!((outline, width), (Paint::solid(vectorcraft_color::Color::BLACK), 0.25));
    let band = series_parts(&s, id, 0)[0].clone();
    let st = band.appearance.stroke().unwrap();
    assert_eq!((st.opacity, st.blend), (0.5, BlendMode::Multiply));
}

/// A graphic style without a stroke applied to a marker whose stroke had paint replaces the stroke opacity and blend mode
/// the series' Filled Lines band left: the band's outline is drawn opaque and Normal again. A marker that had no stroke
/// before the style leaves them as they are.
#[test]
fn a_style_that_removes_a_marks_stroke_replaces_the_kept_stroke_opacity() {
    for stroked in [true, false] {
        let (mut s, id, band) = band_graph();
        let marker = series_parts(&s, id, 0)[1].id;
        if stroked {
            let row = top_row(&s, marker, false);
            s.execute("appearance.setItem", &json!({"ids": [marker.0], "index": row, "color": "#ff0000"})).unwrap();
        }
        let row = top_row(&s, band, false);
        s.execute("appearance.setItem", &json!({"ids": [band.0], "index": row, "opacity": 40, "blend": "multiply"})).unwrap();
        assert_eq!(spec(&s, id).series_paints[0].stroke_opacity, Some(0.4));
        blue_style(&mut s, "Blue", false);
        s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [marker.0]})).unwrap();
        let kept = spec(&s, id).series_paints[0].clone();
        let want = if stroked { (None, None) } else { (Some(0.4), Some(BlendMode::Multiply)) };
        assert_eq!((kept.stroke_opacity, kept.stroke_blend), want, "stroked {stroked}");
        redrawn_band(&mut s, id);
        let st = series_parts(&s, id, 0)[0].appearance.stroke().unwrap().clone();
        assert_eq!((Some(st.opacity).filter(|o| *o < 1.0), Some(st.blend).filter(|b| *b != BlendMode::Normal)), want, "stroked {stroked}");
    }
}

/// An Eyedropper pick from a stroke row without paint onto a marker whose stroke had paint replaces the stroke opacity and
/// blend mode the series' Filled Lines band left, in the pick's undo step: the band's outline is drawn opaque and Normal
/// again. A pick onto a marker whose stroke had no paint before the pick leaves them as they are.
#[test]
fn an_eyedropper_pick_that_unpaints_a_series_stroke_clears_its_stored_transparency() {
    for stroked in [true, false] {
        let (mut s, id, band) = band_graph();
        let parts = series_parts(&s, id, 0);
        let (marker, source) = (parts[1].id, parts[2].id);
        let st = parts[2].appearance.stroke().unwrap();
        assert_eq!((st.paint.is_none(), st.opacity, st.blend), (true, 1.0, BlendMode::Normal), "the source's stroke row");
        if stroked {
            let row = top_row(&s, marker, false);
            s.execute("appearance.setItem", &json!({"ids": [marker.0], "index": row, "color": "#ff0000"})).unwrap();
        }
        let row = top_row(&s, band, false);
        s.execute("appearance.setItem", &json!({"ids": [band.0], "index": row, "opacity": 40, "blend": "multiply"})).unwrap();
        let was = spec(&s, id).series_paints;
        assert_eq!((was[0].stroke_opacity, was[0].stroke_blend), (Some(0.4), Some(BlendMode::Multiply)));
        s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marker.0]})).unwrap();
        let picked = s.doc().unwrap().doc.node(marker).unwrap().appearance.stroke().unwrap().clone();
        assert_eq!((picked.paint.is_none(), picked.opacity, picked.blend), (true, 1.0, BlendMode::Normal), "stroked {stroked}");
        let kept = spec(&s, id).series_paints[0].clone();
        let want = if stroked { (None, None) } else { (Some(0.4), Some(BlendMode::Multiply)) };
        assert_eq!((kept.stroke_opacity, kept.stroke_blend), want, "stroked {stroked}");
        if stroked {
            let after = spec(&s, id).series_paints;
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(spec(&s, id).series_paints, was);
            assert!(!s.doc().unwrap().doc.node(marker).unwrap().appearance.stroke_paint().is_none(), "the undo took back the pick");
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(spec(&s, id).series_paints, after);
        }
        redrawn_band(&mut s, id);
        let st = series_parts(&s, id, 0)[0].appearance.stroke().unwrap().clone();
        assert_eq!((Some(st.opacity).filter(|o| *o < 1.0), Some(st.blend).filter(|b| *b != BlendMode::Normal)), want, "stroked {stroked}");
    }
}

/// A Filled Lines band's stroke row removed with the Appearance panel or set to None with `paint.setStroke` leaves the
/// band its color: an empty stored stroke is not a color for the band.
#[test]
fn removing_a_filled_line_bands_stroke_leaves_its_color() {
    let (mut s, id, band) = band_graph();
    let was = series_parts(&s, id, 0)[0].appearance.fill_paint();
    s.execute("appearance.removeItem", &json!({"ids": [band.0], "index": top_row(&s, band, false)})).unwrap();
    assert_eq!(redrawn_band(&mut s, id).0, was);

    let (mut s, id, band) = band_graph();
    let was = series_parts(&s, id, 0)[0].appearance.fill_paint();
    s.execute("paint.setStroke", &json!({"ids": [band.0], "none": true})).unwrap();
    assert!(spec(&s, id).series_paints.first().is_none_or(|p| p.stroke.is_none()));
    assert_eq!(redrawn_band(&mut s, id).0, was);
}

/// With Connect Data Points off a Filled Lines graph draws no bands, so a stroke set on a marker is kept.
#[test]
fn a_stroke_on_a_marker_is_kept_when_filled_lines_draws_no_bands() {
    let (mut s, id, _) = band_graph();
    s.execute("graph.setType", &json!({"id": id.0, "connectPoints": false})).unwrap();
    let marker = series_parts(&s, id, 0)[0].id;
    s.execute("appearance.setItem", &json!({"ids": [marker.0], "index": top_row(&s, marker, false), "color": "#ff0000"})).unwrap();
    assert!(spec(&s, id).series_paints[0].stroke.as_ref().is_some_and(|p| !p.is_none()));
}

/// A Filled Lines graph of `ty` and `csv` draws no bands, so a stroke set on its first mark is stored and stays on it after
/// the graph is drawn again.
fn stroke_is_kept_without_bands(ty: &str, csv: &str) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, ty, csv);
    s.execute("graph.setType", &json!({"id": id.0, "filledLines": true})).unwrap();
    let mark = series_parts(&s, id, 0)[0].id;
    s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, false), "color": "#ff0000", "weight": 2})).unwrap();
    let red = s.doc().unwrap().doc.node(mark).unwrap().appearance.stroke_paint();
    assert_eq!(spec(&s, id).series_paints[0].stroke, Some(red.clone()));
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let drawn = &series_parts(&s, id, 0)[0];
    assert_eq!((drawn.appearance.stroke_paint(), drawn.appearance.stroke_width()), (red, 2.0));
}

#[test]
fn a_stroke_is_kept_on_a_one_row_line_series() {
    stroke_is_kept_without_bands("line", ",a\nQ1,2");
}

#[test]
fn a_stroke_is_kept_on_a_radar_series_of_zeros() {
    stroke_is_kept_without_bands("radar", ",a\nQ1,0\nQ2,0\nQ3,0");
}

#[test]
fn a_stroke_is_kept_on_a_scatter_series_of_one_repeated_point() {
    stroke_is_kept_without_bands("scatter", "a,b\n1,2\n1,2");
}

/// A radar series whose values all fall below `axisMin` is plotted at the center and draws no band, so a stroke set on its
/// marks is kept. A radar series with a value above `axisMin` draws a band and stores no stroke.
#[test]
fn a_radar_series_draws_a_band_only_for_values_away_from_the_center() {
    for (csv, kept) in [(",a\nQ1,1\nQ2,2\nQ3,3", true), (",a\nQ1,1\nQ2,15\nQ3,3", false)] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "radar", csv);
        s.execute("graph.setType", &json!({"id": id.0, "filledLines": true, "axisMin": 10, "axisMax": 20})).unwrap();
        let mark = series_parts(&s, id, 0)[0].id;
        s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, false), "color": "#ff0000", "weight": 2})).unwrap();
        assert_eq!(spec(&s, id).series_paints.first().is_some_and(|p| p.stroke.is_some()), kept, "{csv}");
    }
}

/// Past the Filled Lines budget the graph draws thin lines, and a stroke set on one is kept.
#[test]
fn a_stroke_on_a_thin_line_past_the_filled_lines_budget_is_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let rows: Vec<Vec<f64>> = (0..8_000).map(|i| vec![(i % 7) as f64, 1.0, 2.0]).collect();
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "line", "x": 0, "y": 0, "width": 300, "height": 200, "rows": rows})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("graph.setType", &json!({"id": id.0, "filledLines": true, "markPoints": false})).unwrap();
    let line = series_parts(&s, id, 0)[0].id;
    s.execute("appearance.setItem", &json!({"ids": [line.0], "index": top_row(&s, line, false), "color": "#ff0000"})).unwrap();
    assert!(spec(&s, id).series_paints[0].stroke.as_ref().is_some_and(|p| !p.is_none()));
}

/// A weight of 0 set again on a painted stroke replaces the weight another mark of the series left on the graph.
#[test]
fn a_zero_weight_on_a_painted_stroke_replaces_the_kept_weight() {
    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 0})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#ff0000", "weight": 5})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke_width, Some(5.0));
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "weight": 0})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke_width, Some(0.0));
}

/// An Eyedropper `append` of a source with no stroke leaves the stroke the series keeps.
#[test]
fn appending_a_fill_only_appearance_leaves_the_kept_stroke() {
    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#ff0000", "weight": 5})).unwrap();
    let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    s.execute("appearance.removeItem", &json!({"ids": [source.0], "index": top_row(&s, source, false)})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0], "append": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke_width, Some(5.0));
    // A pick of the weight alone from that source leaves the mark and what the series keeps as they were.
    let pick = json!({"appearance": {"fill": {"color": false}, "stroke": {"color": false, "weight": true}}});
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0], "pickUp": pick})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke_width, Some(5.0));
}

/// An Eyedropper `append` of a source with only a hidden fill leaves the fill the series keeps.
#[test]
fn appending_a_hidden_fill_leaves_the_kept_fill() {
    let (mut s, id, _, marks) = column_marks();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "color": "#ff0000"})).unwrap();
    let kept = spec(&s, id).series_paints[0].fill.clone();
    let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    s.execute("appearance.setItem", &json!({"ids": [source.0], "index": top_row(&s, source, true), "visible": false})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0], "append": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, kept);
}

/// An Eyedropper pick of the weight alone reads a hidden stroke too, and that weight replaces the one the series keeps.
#[test]
fn picking_the_weight_of_a_hidden_stroke_replaces_the_kept_weight() {
    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#ff0000", "weight": 5})).unwrap();
    let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    let row = top_row(&s, source, false);
    s.execute("appearance.setItem", &json!({"ids": [source.0], "index": row, "color": "#00ff00", "weight": 2, "visible": false})).unwrap();
    let pick = json!({"appearance": {"fill": {"color": false}, "stroke": {"color": false, "weight": true}}});
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0], "pickUp": pick})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke_width, Some(2.0));
}

/// An Eyedropper pick of some attributes edits the mark's topmost stroke. When that row is hidden, the series keeps what
/// it stores instead of taking the visible row under it.
#[test]
fn a_partial_pick_onto_a_hidden_top_row_leaves_the_kept_stroke() {
    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.addStroke", &json!({"ids": [marks[1].0]})).unwrap();
    let hidden = top_row(&s, marks[1], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": hidden, "visible": false})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#0000ff", "weight": 5})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    let row = top_row(&s, source, false);
    s.execute("appearance.setItem", &json!({"ids": [source.0], "index": row, "color": "#00ff00", "weight": 3})).unwrap();
    let pick = json!({"appearance": {"fill": {"color": false}, "stroke": {"color": true, "weight": true}}});
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0], "pickUp": pick})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0], kept);
}

/// An Eyedropper `append` writes a visible row under the source's hidden top row, and the series stores that row's paint
/// even when the mark already shows the same paint.
#[test]
fn appending_a_visible_row_under_a_hidden_one_replaces_the_kept_stroke() {
    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#0000ff", "weight": 5})).unwrap();
    let source = series(s.doc().unwrap().doc.node(id).unwrap(), 1).children().unwrap()[0].id;
    let row = top_row(&s, source, false);
    s.execute("appearance.setItem", &json!({"ids": [source.0], "index": row, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.addStroke", &json!({"ids": [source.0]})).unwrap();
    let hidden = top_row(&s, source, false);
    s.execute("appearance.setItem", &json!({"ids": [source.0], "index": hidden, "visible": false})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": source.0, "ids": [marks[1].0], "append": true})).unwrap();
    let kept = &spec(&s, id).series_paints[0];
    assert_eq!(kept.stroke, Some(Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0))));
    assert_eq!(kept.stroke_width, Some(2.0));
}

/// A stroke added on top of a mark replaces the stroke another mark of the series left on the graph, even when the new
/// row shows the paint the mark already showed.
#[test]
fn adding_a_stroke_replaces_the_kept_stroke_with_the_marks_own() {
    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#0000ff", "weight": 5})).unwrap();
    s.execute("appearance.addStroke", &json!({"ids": [marks[1].0]})).unwrap();
    let kept = &spec(&s, id).series_paints[0];
    assert_eq!(kept.stroke, Some(Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0))));
    assert_eq!(kept.stroke_width, Some(2.0));
}

/// A fill added right above a mark's active top fill replaces the fill another mark of the series left on the graph.
#[test]
fn adding_a_fill_above_the_active_top_fill_replaces_the_kept_fill() {
    let (mut s, id, was, marks) = column_marks();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "color": "#0000ff"})).unwrap();
    s.execute("select.set", &json!({"ids": [marks[1].0]})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": fill})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(was));
}

/// A copy of a mark's top stroke replaces the stroke another mark of the series left on the graph.
#[test]
fn duplicating_a_marks_top_stroke_replaces_the_kept_stroke() {
    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#0000ff", "weight": 5})).unwrap();
    for to in [false, true] {
        s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#0000ff", "weight": 5})).unwrap();
        let top = top_row(&s, marks[1], false);
        let p = if to { json!({"ids": [marks[1].0], "index": top, "to": top + 1}) } else { json!({"ids": [marks[1].0], "index": top}) };
        s.execute("appearance.duplicateItem", &p).unwrap();
        let kept = &spec(&s, id).series_paints[0];
        assert_eq!(kept.stroke, Some(Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0))), "{p}");
        assert_eq!(kept.stroke_width, Some(2.0), "{p}");
    }
}

/// A weight set on a stroke without paint (a line graph's marker) leaves the paint the series keeps for its line.
#[test]
fn a_weight_on_a_stroke_without_paint_leaves_the_kept_stroke() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "line");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let marks = series(&n, 0).children().unwrap().to_vec();
    let line = marks.iter().find(|m| m.appearance.stroke_width() > 0.0).unwrap().id;
    let marker = marks.iter().find(|m| m.appearance.stroke_width() == 0.0 && m.appearance.stroke().is_some()).unwrap().id;
    s.execute("appearance.setItem", &json!({"ids": [line.0], "index": top_row(&s, line, false), "color": "#ff0000"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert!(kept.stroke.as_ref().is_some_and(|p| !p.is_none()));
    s.execute("appearance.setItem", &json!({"ids": [marker.0], "index": top_row(&s, marker, false), "weight": 4})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0], kept);
}

/// The graph does not keep a paint set on a row of a graph design's copy inside a series. A stroke edit on a line keeps
/// no fill, and a blend mode alone leaves what the graph keeps as it was.
#[test]
fn a_paint_on_a_design_copy_or_the_other_kind_of_row_is_not_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    save_design(&mut s, "Square");
    s.execute("graph.column", &json!({"id": id.0, "design": "Square"})).unwrap();
    let copy = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].clone();
    assert!(matches!(copy.kind, NodeKind::Group { .. }));
    assert!(s.execute("appearance.addFill", &json!({"ids": [copy.id.0]})).is_err());
    assert!(s.execute("appearance.setItem", &json!({"ids": [copy.id.0], "index": 0, "color": "#0000ff"})).is_err());
    assert!(spec(&s, id).series_paints.first().is_none_or(|p| p.fill.is_none()));
    // A path inside the design's copy is a member of the series too, and its paint is not kept either.
    let copy = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].clone();
    let mut paths = vec![];
    copy.walk(&mut |n| {
        if matches!(n.kind, NodeKind::Path { .. }) {
            paths.push(n.id);
        }
    });
    let art = paths[0];
    assert!(s.execute("appearance.setItem", &json!({"ids": [art.0], "index": top_row(&s, art, true), "color": "#00ff00"})).is_err());
    assert!(spec(&s, id).series_paints.first().is_none_or(|p| p.fill.is_none()));

    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "line");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let line = series(&n, 0).children().unwrap().iter().find(|m| m.appearance.stroke().is_some_and(|st| st.width > 0.0)).unwrap().clone();
    let row = line.appearance.items.iter().rposition(|it| !it.is_fill()).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [line.id.0], "index": row, "color": "#0000ff"})).unwrap();
    assert!(spec(&s, id).series_paints[0].fill.is_none(), "a stroke edit on a line keeps no fill");

    // A blend mode on another mark of the series leaves the stored paint as that mark's own paint differs from it, and
    // stores only the blend mode.
    let (mut s, id, _, marks) = column_marks();
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": fill, "color": "#ff0000"})).unwrap();
    let mut kept = spec(&s, id).series_paints;
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": fill, "blend": "multiply"})).unwrap();
    kept[0].fill_blend = Some(vectorcraft_color::BlendMode::Multiply);
    assert_eq!(spec(&s, id).series_paints, kept);
}

/// A graphic style `name` made from a new rectangle with a blue fill at 50% opacity, and without a stroke row unless
/// `stroke`: the rectangle and the blue paint.
fn blue_style(s: &mut Session, name: &str, stroke: bool) -> (NodeId, Paint) {
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("appearance.setItem", &json!({"ids": [r.0], "index": top_row(s, r, true), "color": "#0000ff", "opacity": 50})).unwrap();
    if !stroke {
        s.execute("appearance.removeItem", &json!({"ids": [r.0], "index": top_row(s, r, false)})).unwrap();
    }
    s.execute("graphicStyle.new", &json!({"id": r.0, "name": name})).unwrap();
    let blue = s.doc().unwrap().doc.node(r).unwrap().appearance.fill_paint();
    (r, blue)
}

/// Series 0's marks with a red 2 pt stroke stored from `marks[1]` and a 40% fill opacity stored from `marks[2]`.
fn stored_red_stroke() -> (Session, NodeId, Vec<NodeId>) {
    let (mut s, id, _, marks) = column_marks();
    let stroke = top_row(&s, marks[1], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[2].0], "index": top_row(&s, marks[2], true), "opacity": 40})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.stroke_width, kept.fill_opacity), (Some(2.0), Some(0.4)));
    (s, id, marks)
}

/// After the graph is drawn again, every mark of series 0 has `fill` at `opacity` and `stroke` at `weight`; two undos (the
/// data edit, then the command) bring back the paints the graph stored in `was`.
fn series_0_is(s: &mut Session, id: NodeId, (fill, opacity, stroke, weight): (&Paint, f32, &Paint, f64), was: &[vectorcraft_doc::SeriesPaint]) {
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    for m in series(&n, 0).children().unwrap() {
        let f = m.appearance.fill().unwrap();
        assert_eq!((&f.paint, f.opacity), (fill, opacity));
        assert_eq!((&m.appearance.stroke_paint(), m.appearance.stroke_width()), (stroke, weight));
    }
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(s, id).series_paints, was);
}

/// A graphic style applied to a series mark is stored on the graph, its fill opacity included. A style without a stroke,
/// applied to a mark that shows one, leaves the series without a stroke.
#[test]
fn a_graphic_style_applied_to_a_series_mark_is_kept() {
    let (mut s, id, marks) = stored_red_stroke();
    let (_, blue) = blue_style(&mut s, "Blue", false);
    let was = spec(&s, id).series_paints;
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [marks[1].0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.fill_opacity, kept.stroke), (Some(blue.clone()), Some(0.5), Some(Paint::None)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was, "one undo takes back the style and what the graph stored");
    s.execute("edit.redo", &json!({})).unwrap();
    series_0_is(&mut s, id, (&blue, 0.5, &Paint::None, 0.0), &was);
}

/// A graphic style added on top of a series mark's appearance (Alt-click) stores the kinds of row the style shows: a style
/// with only a fill leaves the stroke the series stores.
#[test]
fn a_graphic_style_added_to_a_series_mark_keeps_only_its_own_kinds() {
    let (mut s, id, marks) = stored_red_stroke();
    let stroke = top_row(&s, marks[0], false);
    let red = spec(&s, id).series_paints[0].stroke.clone().unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#00ff00", "weight": 3})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    let (_, blue) = blue_style(&mut s, "Blue", false);
    let was = spec(&s, id).series_paints;
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [marks[0].0], "add": true})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.fill_opacity), (Some(blue.clone()), Some(0.5)));
    assert_eq!((kept.stroke.clone(), kept.stroke_width), (Some(red.clone()), Some(2.0)));
    series_0_is(&mut s, id, (&blue, 0.5, &red, 2.0), &was);
}

/// A graphic style whose stroke row is hidden, added on top of a series mark (Alt-click), stores its fill and leaves the
/// stroke the series stores, although the mark shows a stroke of its own.
#[test]
fn a_graphic_style_with_a_hidden_stroke_added_to_a_series_mark_keeps_the_stored_stroke() {
    let (mut s, id, marks) = stored_red_stroke();
    let stroke = top_row(&s, marks[0], false);
    let red = spec(&s, id).series_paints[0].stroke.clone().unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#00ff00", "weight": 3})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": stroke, "color": "#ff0000", "weight": 2})).unwrap();
    let (rect, blue) = blue_style(&mut s, "Blue", true);
    s.execute("appearance.setItem", &json!({"ids": [rect.0], "index": top_row(&s, rect, false), "visible": false})).unwrap();
    s.execute("graphicStyle.new", &json!({"id": rect.0, "name": "Hidden stroke"})).unwrap();
    let was = spec(&s, id).series_paints;
    s.execute("graphicStyle.apply", &json!({"name": "Hidden stroke", "ids": [marks[0].0], "add": true})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.fill_opacity), (Some(blue.clone()), Some(0.5)));
    assert_eq!((kept.stroke.clone(), kept.stroke_width), (Some(red.clone()), Some(2.0)));
    series_0_is(&mut s, id, (&blue, 0.5, &red, 2.0), &was);
}

/// Redefine Graphic Style restyles the series marks still linked to the style, and the graph stores the new look.
#[test]
fn redefining_a_graphic_style_updates_a_linked_series_mark() {
    let (mut s, id, _, marks) = column_marks();
    let (rect, _) = blue_style(&mut s, "Blue", true);
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [marks[0].0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [rect.0], "index": top_row(&s, rect, true), "color": "#00ff00", "opacity": 100})).unwrap();
    let green = s.doc().unwrap().doc.node(rect).unwrap().appearance.fill_paint();
    let was = spec(&s, id).series_paints;
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "id": rect.0})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.fill_paint(), green, "the mark follows the style");
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill, kept.fill_opacity), (Some(green.clone()), None));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was, "one undo takes back the redefinition and what the graph stored");
    s.execute("edit.redo", &json!({})).unwrap();
    let (stroke, weight) = {
        let r = s.doc().unwrap().doc.node(rect).unwrap().clone();
        (r.appearance.stroke_paint(), r.appearance.stroke_width())
    };
    series_0_is(&mut s, id, (&green, 1.0, &stroke, weight), &was);
}

/// Redefine Graphic Style made from a series mark does not edit that mark, so the graph keeps the paint another mark of the
/// series stored after it: the graph drawn again shows the same marks with or without the redefinition, and undo brings
/// back the stored paint.
#[test]
fn redefining_a_graphic_style_from_a_series_mark_keeps_the_paint_another_mark_stored() {
    let (mut s, id, _, marks) = column_marks();
    blue_style(&mut s, "Blue", true);
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [marks[1].0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": top_row(&s, marks[1], true), "color": "#00ff00"})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": top_row(&s, marks[0], true), "color": "#ff0000"})).unwrap();
    let red = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.fill_paint();
    let was = spec(&s, id).series_paints;
    assert_eq!(was[0].fill, Some(red.clone()));
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    let without = marks_of(&s, id, 0);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "id": marks[1].0})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was, "the source mark was not edited");
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    assert_eq!(marks_of(&s, id, 0), without);
    assert!(without.iter().all(|m| m.0 == red));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);
}

/// Series 0's group, and whether it is as the graph draws it: the group has no rows of its own and each path directly
/// inside it has rows.
fn series_0_group(s: &Session, id: NodeId) -> (NodeId, bool) {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let g = series(&n, 0);
    let drawn = g.appearance.items.is_empty() && g.children().unwrap().iter().all(|m| !m.appearance.items.is_empty());
    (g.id, drawn)
}

/// `appearance.setItem` given a series group's id edits the paths directly inside it, so the graph stores the color and
/// opacity and every mark has them after the graph is drawn again. The group keeps no rows of its own.
#[test]
fn an_appearance_item_set_on_a_series_group_edits_its_marks() {
    let (mut s, id, _, marks) = column_marks();
    let (group, drawn) = series_0_group(&s, id);
    assert!(drawn);
    let was = spec(&s, id).series_paints;
    let row = top_row(&s, marks[0], true);
    s.execute("appearance.setItem", &json!({"ids": [group.0], "index": row, "color": "#0000ff", "opacity": 40})).unwrap();
    assert!(series_0_group(&s, id).1, "the group has no rows of its own");
    let blue = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.fill_paint();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill.clone(), kept.fill_opacity), (Some(blue.clone()), Some(0.4)));
    let (stroke, weight) = {
        let m = s.doc().unwrap().doc.node(marks[0]).unwrap().clone();
        (m.appearance.stroke_paint(), m.appearance.stroke_width())
    };
    series_0_is(&mut s, id, (&blue, 0.4, &stroke, weight), &was);
}

/// The other Appearance commands given a series group's id, picked with Group Selection or given in `ids`, edit the
/// paths directly inside it, and the graph stores what they change: Add New Fill, Clear Appearance, Move Appearance onto
/// the group, and an opacity set on a row with `transparency.set`.
#[test]
fn appearance_commands_given_a_series_group_edit_its_marks() {
    let (mut s, id, marks) = stored_red_stroke();
    let (group, _) = series_0_group(&s, id);
    s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    assert!(series_0_group(&s, id).1, "Add New Fill on the selected group");
    let rows = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.items.iter().filter(|it| it.is_fill()).count();
    assert_eq!(rows, 2, "each mark has a new fill row");
    s.execute("edit.undo", &json!({})).unwrap();

    let row = top_row(&s, marks[0], true);
    s.execute("transparency.set", &json!({"ids": [group.0], "item": row, "opacity": 30})).unwrap();
    assert!(series_0_group(&s, id).1);
    assert_eq!(spec(&s, id).series_paints[0].fill_opacity, Some(0.3));
    s.execute("edit.undo", &json!({})).unwrap();

    s.execute("appearance.clear", &json!({"ids": [group.0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(Paint::None));
    s.execute("edit.undo", &json!({})).unwrap();

    let (rect, blue) = blue_style(&mut s, "Blue", false);
    s.execute("object.setProps", &json!({"ids": [group.0], "opacity": 50})).unwrap();
    let was = spec(&s, id).series_paints;
    s.execute("appearance.transfer", &json!({"source": rect.0, "target": group.0, "copy": true})).unwrap();
    assert!(series_0_group(&s, id).1);
    assert_eq!(s.doc().unwrap().doc.node(group).unwrap().opacity, 1.0, "the appearance replaces the group's own opacity");
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill.clone(), kept.fill_opacity, kept.stroke), (Some(blue.clone()), Some(0.5), Some(Paint::None)));
    series_0_is(&mut s, id, (&blue, 0.5, &Paint::None, 0.0), &was);
}

/// Moving the Contents row of a series group as the graph draws it, with no rows of its own, moves the group's own
/// row: the paths inside it have none.
#[test]
fn moving_a_series_groups_contents_row_moves_its_own_row() {
    let (mut s, id, _, _) = column_marks();
    let (group, drawn) = series_0_group(&s, id);
    assert!(drawn);
    let out = s.execute("appearance.moveItem", &json!({"ids": [group.0], "from": "contents", "to": 0})).unwrap();
    assert_eq!(out["contents"], json!(0));
}

/// Add New Stroke, Duplicate Item, Reorder Appearance Item and Remove Item given a series group's id edit the rows of
/// the paths directly inside it, and the group keeps no rows of its own.
#[test]
fn row_commands_given_a_series_group_edit_its_marks_rows() {
    let (mut s, id, _, marks) = column_marks();
    let (group, _) = series_0_group(&s, id);
    let count = |s: &Session, fill: bool| -> Vec<usize> {
        let d = &s.doc().unwrap().doc;
        marks.iter().map(|m| d.node(*m).unwrap().appearance.items.iter().filter(|it| it.is_fill() == fill).count()).collect()
    };
    let (fills, strokes) = (count(&s, true), count(&s, false));
    s.execute("appearance.addStroke", &json!({"ids": [group.0]})).unwrap();
    assert_eq!(count(&s, false), strokes.iter().map(|n| n + 1).collect::<Vec<_>>(), "each mark has a new stroke row");
    let fill = top_row(&s, marks[0], true);
    s.execute("appearance.duplicateItem", &json!({"ids": [group.0], "index": fill})).unwrap();
    assert_eq!(count(&s, true), fills.iter().map(|n| n + 1).collect::<Vec<_>>(), "each mark has a copy of its fill row");
    let rows = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.items.len();
    let out = s.execute("appearance.moveItem", &json!({"ids": [group.0], "from": 0, "to": rows - 1})).unwrap();
    assert_eq!(out["index"], json!(rows - 1));
    s.execute("appearance.removeItem", &json!({"ids": [group.0], "index": top_row(&s, marks[0], true)})).unwrap();
    assert_eq!(count(&s, true), fills);
    assert!(series_0_group(&s, id).1, "the group has no rows of its own");
}

/// Reduce to Basic Appearance given a series group's id reduces the paths directly inside it and drops an effect the
/// group holds.
#[test]
fn reducing_a_series_group_reduces_its_marks_and_drops_its_own_effect() {
    let (mut s, id, _, marks) = column_marks();
    let (group, _) = series_0_group(&s, id);
    s.execute("appearance.addFill", &json!({"ids": [group.0]})).unwrap();
    s.execute("effect.apply", &json!({"ids": [group.0], "effect": "distort.twist"})).unwrap();
    s.execute("appearance.reduceToBasic", &json!({"ids": [group.0]})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(group).unwrap().appearance.effects.is_empty(), "the group's effect is dropped");
    assert!(marks.iter().all(|m| d.node(*m).unwrap().appearance.items.iter().filter(|it| it.is_fill()).count() == 1));
    assert!(series_0_group(&s, id).1);
}

/// A graphic style added on top (Alt-click) given a series group's id adds its rows to the paths directly inside the
/// group and unlinks a group that held a style link.
#[test]
fn a_graphic_style_added_to_a_series_group_adds_rows_to_its_marks_and_unlinks_the_group() {
    let (mut s, id, _, marks) = column_marks();
    let (group, _) = series_0_group(&s, id);
    blue_style(&mut s, "Blue", false);
    let style = s.doc().unwrap().doc.graphic_style("Blue").unwrap().id;
    s.edit("Link", |d, _| {
        d.node_mut(group).unwrap().extras_mut().graphic_style = Some(style);
        Ok(())
    })
    .unwrap();
    let fills = |s: &Session, m: NodeId| s.doc().unwrap().doc.node(m).unwrap().appearance.items.iter().filter(|it| it.is_fill()).count();
    let was = fills(&s, marks[0]);
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [group.0], "add": true})).unwrap();
    assert!(marks.iter().all(|m| fills(&s, *m) == was + 1), "each mark has the style's fill on top");
    assert!(series_0_group(&s, id).1, "the group takes no rows of the style");
    assert_eq!(s.doc().unwrap().doc.node(group).unwrap().extras().graphic_style, None, "the group is unlinked");
}

/// Move Appearance from one of a series group's paths onto the group gives the other paths its look and leaves that
/// path as it was. When the path is the only one directly in the group, the command is refused and the group takes no
/// rows.
#[test]
fn moving_an_appearance_from_a_mark_onto_its_own_series_group_keeps_the_mark() {
    let (mut s, id, _, marks) = column_marks();
    let (group, _) = series_0_group(&s, id);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": top_row(&s, marks[0], true), "color": "#00ff00"})).unwrap();
    let green = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.fill_paint();
    s.execute("appearance.transfer", &json!({"source": marks[0].0, "target": group.0})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(marks.iter().all(|m| d.node(*m).unwrap().appearance.fill_paint() == green), "every mark, the source too, is green");
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(green));

    s.edit("One path", |d, _| {
        d.children_mut(Some(group)).unwrap().truncate(1);
        Ok(())
    })
    .unwrap();
    let only = s.doc().unwrap().doc.node(group).unwrap().children().unwrap()[0].id;
    let err = s.execute("appearance.transfer", &json!({"source": only.0, "target": group.0})).unwrap_err();
    assert!(err.to_string().contains("only path"), "{err}");
    assert!(series_0_group(&s, id).1, "the group has no rows of its own");
}

/// A series group that holds an opacity and an effect of its own is read itself as a source, as any group is: a new
/// graphic style and Move Appearance take the group's opacity and effect, and the group, not a path, is linked.
#[test]
fn a_series_group_with_a_look_of_its_own_is_read_itself_as_a_source() {
    let (mut s, id, _, marks) = column_marks();
    let (group, _) = series_0_group(&s, id);
    s.execute("transparency.set", &json!({"ids": [group.0], "opacity": 50, "item": null})).unwrap();
    s.execute("effect.apply", &json!({"ids": [group.0], "effect": "distort.twist"})).unwrap();
    s.execute("graphicStyle.new", &json!({"id": group.0, "name": "Series"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let style = d.graphic_style("Series").unwrap();
    assert_eq!((style.opacity, style.appearance.effects.len()), (0.5, 1));
    assert_eq!(d.node(group).unwrap().extras().graphic_style, Some(style.id));
    assert!(marks.iter().all(|m| d.node(*m).unwrap().extras().graphic_style.is_none()));

    let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("appearance.transfer", &json!({"source": group.0, "target": rect.0, "copy": true})).unwrap();
    let r = s.doc().unwrap().doc.node(rect).unwrap().clone();
    assert_eq!((r.opacity, r.appearance.effects.len()), (0.5, 1));
}

/// Clear Appearance and Show All Hidden Attributes given a series group's id also reach an effect the group holds, and
/// Break Link to Graphic Style unlinks the paths a style applied to the group linked.
#[test]
fn whole_appearance_commands_given_a_series_group_edit_the_group_and_its_marks() {
    let (mut s, id, marks) = stored_red_stroke();
    let (group, _) = series_0_group(&s, id);
    let effects = |s: &Session| s.doc().unwrap().doc.node(group).unwrap().appearance.effects.clone();
    s.execute("effect.apply", &json!({"ids": [group.0], "effect": "distort.twist"})).unwrap();
    s.execute("effect.setParams", &json!({"ids": [group.0], "index": 0, "visible": false, "item": null})).unwrap();
    assert!(!effects(&s)[0].visible);
    let shown = s.execute("appearance.showAllHidden", &json!({"ids": [group.0]})).unwrap();
    assert_eq!(shown["ids"], json!([group.0]));
    assert!(effects(&s)[0].visible);
    s.execute("appearance.clear", &json!({"ids": [group.0]})).unwrap();
    assert!(effects(&s).is_empty(), "the group's effect is cleared with its marks' rows");
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(Paint::None));
    s.execute("edit.undo", &json!({})).unwrap();

    blue_style(&mut s, "Blue", false);
    s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
    s.execute("graphicStyle.apply", &json!({"name": "Blue"})).unwrap();
    assert_eq!(s.selection_graphic_style().map(|(g, linked)| (g.name.clone(), linked)), Some(("Blue".into(), true)));
    s.execute("graphicStyle.breakLink", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(marks.iter().all(|m| d.node(*m).unwrap().extras().graphic_style.is_none()), "each mark is unlinked");
    assert!(s.selection_graphic_style().is_none());
}

/// Redefine Graphic Style on a selected series group whose paths a style links takes the look of the topmost of them.
#[test]
fn redefining_a_graphic_style_from_a_selected_series_group_reads_its_topmost_mark() {
    let (mut s, id, marks) = stored_red_stroke();
    let (group, _) = series_0_group(&s, id);
    blue_style(&mut s, "Blue", false);
    s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
    s.execute("graphicStyle.apply", &json!({"name": "Blue"})).unwrap();
    let top = *marks.last().unwrap();
    s.execute("appearance.setItem", &json!({"ids": [top.0], "index": top_row(&s, top, true), "color": "#00ff00"})).unwrap();
    let green = s.doc().unwrap().doc.node(top).unwrap().appearance.fill_paint();
    s.execute("graphicStyle.redefine", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.graphic_style("Blue").unwrap().appearance.fill_paint(), green);
    assert!(marks.iter().all(|m| d.node(*m).unwrap().appearance.fill_paint() == green), "the marks still linked follow");
}

/// A series group as the graph draws it, read as a source, gives the look of the topmost path directly inside it: New
/// Graphic Style keeps that path's fill and links it, Move Appearance copies it, and a move clears every path of the
/// group.
#[test]
fn a_series_group_read_as_a_source_gives_its_topmost_marks_look() {
    let (mut s, id, marks) = stored_red_stroke();
    let (group, _) = series_0_group(&s, id);
    let top = *marks.last().unwrap();
    s.execute("appearance.setItem", &json!({"ids": [group.0], "index": top_row(&s, marks[0], true), "color": "#0000ff", "opacity": 30})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": top_row(&s, marks[0], true), "color": "#00ff00"})).unwrap();
    let look = s.doc().unwrap().doc.node(top).unwrap().appearance.clone();
    assert_ne!(look.fill_paint(), s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.fill_paint());
    s.execute("graphicStyle.new", &json!({"id": group.0, "name": "Series"})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.graphic_style("Series").unwrap().appearance.fill_paint(), look.fill_paint());
    assert_eq!(d.node(top).unwrap().extras().graphic_style, Some(d.graphic_style("Series").unwrap().id));

    let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("appearance.transfer", &json!({"source": group.0, "target": rect.0})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.node(rect).unwrap().appearance.fill_paint(), look.fill_paint());
    assert!(marks.iter().all(|m| d.node(*m).unwrap().appearance.fill_paint() == Paint::None), "the move clears each mark");
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(Paint::None));
}

/// A series group that has a row of its own, as a document saved earlier can hold, is edited itself: the Appearance
/// panel lists that row and edits it by its index.
#[test]
fn a_series_group_with_a_row_of_its_own_is_edited_itself() {
    let (mut s, id, _, marks) = column_marks();
    let (group, _) = series_0_group(&s, id);
    s.edit("Row", |d, _| {
        let g = d.node_mut(group).unwrap();
        g.appearance = vectorcraft_doc::Appearance::basic(Paint::None, Paint::None, 1.0).into();
        Ok(())
    })
    .unwrap();
    let was = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.clone();
    s.execute("appearance.setItem", &json!({"ids": [group.0], "index": top_row(&s, group, true), "color": "#0000ff"})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_ne!(d.node(group).unwrap().appearance.fill_paint(), Paint::None);
    assert_eq!(d.node(marks[0]).unwrap().appearance, was, "the marks keep their rows");
}

/// A graphic style applied to a series group's id styles and links the paths directly inside it, and the graph stores
/// the style's fill and stroke.
#[test]
fn a_graphic_style_applied_to_a_series_group_styles_its_marks() {
    let (mut s, id, marks) = stored_red_stroke();
    let (group, _) = series_0_group(&s, id);
    let (_, blue) = blue_style(&mut s, "Blue", false);
    let own = json!({"ids": [group.0], "opacity": 50, "blend": "multiply", "isolate": true, "knockout": "on", "item": null});
    s.execute("transparency.set", &own).unwrap();
    s.execute("effect.apply", &json!({"ids": [group.0], "effect": "distort.twist"})).unwrap();
    let was = spec(&s, id).series_paints;
    let g = s.doc().unwrap().doc.node(group).unwrap().clone();
    assert!(g.isolate && g.knockout != Default::default() && g.blend != vectorcraft_color::BlendMode::Normal);
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [group.0]})).unwrap();
    assert!(series_0_group(&s, id).1, "the group takes no rows of the style");
    let g = s.doc().unwrap().doc.node(group).unwrap().clone();
    assert_eq!((g.opacity, g.isolate), (1.0, false), "the style replaces the group's own opacity and isolation");
    assert_eq!((g.blend, g.knockout), (vectorcraft_color::BlendMode::Normal, Default::default()), "and its blend mode and knockout");
    assert!(g.appearance.effects.is_empty(), "and its effects");
    let style = s.doc().unwrap().doc.graphic_style("Blue").unwrap().id;
    let d = &s.doc().unwrap().doc;
    assert!(marks.iter().all(|m| d.node(*m).unwrap().extras().graphic_style == Some(style)), "each mark is linked to the style");
    assert_eq!(d.node(group).unwrap().extras().graphic_style, None);
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill.clone(), kept.fill_opacity, kept.stroke), (Some(blue.clone()), Some(0.5), Some(Paint::None)));
    series_0_is(&mut s, id, (&blue, 0.5, &Paint::None, 0.0), &was);
}

/// Redefine Graphic Style on a series group linked to the style (as a file can hold it) restyles the paths directly
/// inside the group and links them, and the graph stores the new look; the group goes back to no rows of its own.
#[test]
fn redefining_a_graphic_style_restyles_the_marks_of_a_linked_series_group() {
    let (mut s, id, marks) = stored_red_stroke();
    let (group, _) = series_0_group(&s, id);
    let (rect, _) = blue_style(&mut s, "Blue", false);
    let style = s.doc().unwrap().doc.graphic_style("Blue").unwrap().clone();
    s.edit("Link", |d, _| {
        let g = d.node_mut(group).unwrap();
        g.appearance = style.appearance.clone().into();
        g.extras_mut().graphic_style = Some(style.id);
        Ok(())
    })
    .unwrap();
    s.execute("appearance.setItem", &json!({"ids": [rect.0], "index": top_row(&s, rect, true), "color": "#00ff00", "opacity": 100})).unwrap();
    let green = s.doc().unwrap().doc.node(rect).unwrap().appearance.fill_paint();
    let was = spec(&s, id).series_paints;
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "id": rect.0})).unwrap();
    assert!(series_0_group(&s, id).1, "the group has no rows of its own");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.node(group).unwrap().extras().graphic_style, None);
    assert!(marks.iter().all(|m| d.node(*m).unwrap().extras().graphic_style == Some(style.id)));
    assert!(marks.iter().all(|m| d.node(*m).unwrap().appearance.fill_paint() == green));
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill.clone(), kept.fill_opacity, kept.stroke), (Some(green.clone()), None, Some(Paint::None)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was, "one undo takes back the redefinition and what the graph stored");
    s.execute("edit.redo", &json!({})).unwrap();
    series_0_is(&mut s, id, (&green, 1.0, &Paint::None, 0.0), &was);
}

/// Redefine Graphic Style on a linked series group leaves a path of the group that is linked to the style but no
/// longer has its look as it is, unlinked, and restyles the group's other paths.
#[test]
fn redefining_a_graphic_style_keeps_an_edited_mark_of_a_linked_series_group() {
    let (mut s, id, _, marks) = column_marks();
    let (group, _) = series_0_group(&s, id);
    let (rect, _) = blue_style(&mut s, "Blue", false);
    let style = s.doc().unwrap().doc.graphic_style("Blue").unwrap().clone();
    s.edit("Link", |d, _| {
        let g = d.node_mut(group).unwrap();
        g.appearance = style.appearance.clone().into();
        g.extras_mut().graphic_style = Some(style.id);
        d.node_mut(marks[0]).unwrap().extras_mut().graphic_style = Some(style.id);
        Ok(())
    })
    .unwrap();
    let edited = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.clone();
    s.execute("appearance.setItem", &json!({"ids": [rect.0], "index": top_row(&s, rect, true), "color": "#00ff00", "opacity": 100})).unwrap();
    let green = s.doc().unwrap().doc.node(rect).unwrap().appearance.fill_paint();
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "id": rect.0})).unwrap();
    let d = &s.doc().unwrap().doc;
    let first = d.node(marks[0]).unwrap();
    assert_eq!((&first.appearance, first.extras().graphic_style), (&edited, None), "the edited mark keeps its look, unlinked");
    assert!(marks[1..].iter().all(|m| d.node(*m).unwrap().appearance.fill_paint() == green));
    assert!(marks[1..].iter().all(|m| d.node(*m).unwrap().extras().graphic_style == Some(style.id)));
}

/// A column graph whose series 0 group is linked to style "Blue" (as a file can hold it) and whose first path directly
/// inside that group is linked to style "Green" and has its look: the session, the graph, the group, series 0's paths,
/// and the ids of "Blue" and "Green".
fn linked_group_with_a_green_mark() -> (Session, NodeId, NodeId, Vec<NodeId>, u32, u32) {
    let (mut s, id, _, marks) = column_marks();
    let (group, _) = series_0_group(&s, id);
    let (blue, green) = blue_and_green_styles(&mut s);
    link_group_and_member(&mut s, group, &blue, marks[0], &green);
    (s, id, group, marks, blue.id, green.id)
}

/// Graphic styles "Blue" (a blue fill at 50% opacity) and "Green" (a green fill at full opacity), made from one new
/// rectangle that is linked to neither.
fn blue_and_green_styles(s: &mut Session) -> (vectorcraft_doc::GraphicStyle, vectorcraft_doc::GraphicStyle) {
    let (rect, _) = blue_style(s, "Blue", false);
    s.execute("appearance.setItem", &json!({"ids": [rect.0], "index": top_row(s, rect, true), "color": "#00ff00", "opacity": 100})).unwrap();
    s.execute("graphicStyle.new", &json!({"id": rect.0, "name": "Green"})).unwrap();
    let d = &s.doc().unwrap().doc;
    (d.graphic_style("Blue").unwrap().clone(), d.graphic_style("Green").unwrap().clone())
}

/// Links `group` to `outer` with its look and `member` to `inner` with its look, as a file can hold them.
fn link_group_and_member(
    s: &mut Session,
    group: NodeId,
    outer: &vectorcraft_doc::GraphicStyle,
    member: NodeId,
    inner: &vectorcraft_doc::GraphicStyle,
) {
    s.edit("Link", |d, _| {
        let g = d.node_mut(group).unwrap();
        g.appearance = outer.appearance.clone().into();
        g.extras_mut().graphic_style = Some(outer.id);
        let m = d.node_mut(member).unwrap();
        m.appearance = inner.appearance.clone().into();
        m.extras_mut().graphic_style = Some(inner.id);
        Ok(())
    })
    .unwrap();
}

/// Asserts that `d` holds what `was` holds for a redefinition of style "Blue": its definition, the link of `group`, the
/// look and link of each of `members`, and everything graph `id` stores (series paints and label rows included).
fn assert_same_redefine_state(
    d: &vectorcraft_doc::Document,
    was: &vectorcraft_doc::Document,
    id: NodeId,
    group: NodeId,
    members: &[NodeId],
    when: &str,
) {
    assert_eq!(d.graphic_style("Blue"), was.graphic_style("Blue"), "{when}: the definition of Blue");
    assert_eq!(d.node(group).unwrap().extras().graphic_style, was.node(group).unwrap().extras().graphic_style, "{when}: the group's link");
    for m in members {
        let (a, b) = (d.node(*m).unwrap(), was.node(*m).unwrap());
        assert_eq!((&a.appearance, a.extras().graphic_style), (&b.appearance, b.extras().graphic_style), "{when}: member {m:?}");
    }
    assert_eq!(d.node(id).unwrap().extras().graph, was.node(id).unwrap().extras().graph, "{when}: what the graph stores");
}

/// Redefine Graphic Style with another style's look (`from`) on a linked series group leaves a path of the group that is
/// linked to that other style linked to it, and restyles and links the group's other paths; one undo takes the
/// redefinition back and redo makes it again.
#[test]
fn redefining_a_graphic_style_from_another_style_keeps_a_mark_linked_to_it() {
    let (mut s, id, group, marks, blue, green) = linked_group_with_a_green_mark();
    let own = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.clone();
    let look = s.doc().unwrap().doc.graphic_style("Green").unwrap().appearance.fill_paint();
    let was = s.doc().unwrap().doc.clone();
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "from": "Green"})).unwrap();
    let after = s.doc().unwrap().doc.clone();
    let first = after.node(marks[0]).unwrap();
    assert_eq!((&first.appearance, first.extras().graphic_style), (&own, Some(green)), "the mark keeps its own style");
    assert!(marks[1..].iter().all(|m| after.node(*m).unwrap().appearance.fill_paint() == look));
    assert!(marks[1..].iter().all(|m| after.node(*m).unwrap().extras().graphic_style == Some(blue)));
    assert_eq!(after.node(group).unwrap().extras().graphic_style, None);
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(look), "the graph stores the new look");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_same_redefine_state(&s.doc().unwrap().doc, &was, id, group, &marks, "after one undo");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_same_redefine_state(&s.doc().unwrap().doc, &after, id, group, &marks, "after redo");
}

/// Redefine Graphic Style from an object on a linked series group leaves a path of the group that is linked to another
/// style as it is, restyles and links the group's other paths; one undo takes the redefinition back and redo makes it
/// again.
#[test]
fn redefining_a_graphic_style_from_an_object_keeps_a_mark_linked_to_another_style() {
    let (mut s, id, group, marks, blue, green) = linked_group_with_a_green_mark();
    let own = s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.clone();
    let red = NodeId(s.execute("shape.rectangle", &json!({"x": 700, "y": 700, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("appearance.setItem", &json!({"ids": [red.0], "index": top_row(&s, red, true), "color": "#ff0000", "opacity": 100})).unwrap();
    let look = s.doc().unwrap().doc.node(red).unwrap().appearance.fill_paint();
    let was = s.doc().unwrap().doc.clone();
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "id": red.0})).unwrap();
    let after = s.doc().unwrap().doc.clone();
    let first = after.node(marks[0]).unwrap();
    assert_eq!((&first.appearance, first.extras().graphic_style), (&own, Some(green)), "the mark keeps its own style");
    assert!(marks[1..].iter().all(|m| after.node(*m).unwrap().appearance.fill_paint() == look));
    assert!(marks[1..].iter().all(|m| after.node(*m).unwrap().extras().graphic_style == Some(blue)));
    assert_eq!(after.node(group).unwrap().extras().graphic_style, None);
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(look), "the graph stores the new look");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_same_redefine_state(&s.doc().unwrap().doc, &was, id, group, &marks, "after one undo");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_same_redefine_state(&s.doc().unwrap().doc, &after, id, group, &marks, "after redo");
}

/// Redefine Graphic Style on a graph's Legend group linked to the style leaves a label of the group that is linked to
/// another style with its look and its link, restyles and links the group's other label, and clears and unlinks the
/// group; one undo takes the redefinition back and redo makes it again.
#[test]
fn redefining_a_graphic_style_keeps_a_legend_label_linked_to_another_style() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let legend = group(s.doc().unwrap().doc.node(id).unwrap(), "Legend").id;
    let labels = [text_id(&s, id, "Legend", "2024"), text_id(&s, id, "Legend", "2025")];
    let (blue, green) = blue_and_green_styles(&mut s);
    link_group_and_member(&mut s, legend, &blue, labels[0], &green);
    let own = s.doc().unwrap().doc.node(labels[0]).unwrap().appearance.clone();
    let red = NodeId(s.execute("shape.rectangle", &json!({"x": 700, "y": 700, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("appearance.setItem", &json!({"ids": [red.0], "index": top_row(&s, red, true), "color": "#ff0000", "opacity": 100})).unwrap();
    let look = s.doc().unwrap().doc.node(red).unwrap().appearance.fill_paint();
    let was = s.doc().unwrap().doc.clone();
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "id": red.0})).unwrap();
    let after = s.doc().unwrap().doc.clone();
    let kept = after.node(labels[0]).unwrap();
    assert_eq!((&kept.appearance, kept.extras().graphic_style), (&own, Some(green.id)), "the label keeps its own style");
    let other = after.node(labels[1]).unwrap();
    assert_eq!((other.appearance.fill_paint(), other.extras().graphic_style), (look, Some(blue.id)), "the other label follows");
    let g = after.node(legend).unwrap();
    assert_eq!((g.appearance.items.is_empty(), g.extras().graphic_style), (true, None), "the group is cleared and unlinked");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_same_redefine_state(&s.doc().unwrap().doc, &was, id, legend, &labels, "after one undo");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_same_redefine_state(&s.doc().unwrap().doc, &after, id, legend, &labels, "after redo");
}

/// Series `index`'s group: its object opacity and blend mode, and those of each path directly inside it.
fn group_transparency(s: &Session, id: NodeId, index: u32) -> ((f32, BlendMode), Vec<(f32, BlendMode)>) {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let g = series(&n, index);
    ((g.opacity, g.blend), g.children().unwrap().iter().map(|m| (m.opacity, m.blend)).collect())
}

/// An object opacity and blend mode set on a selected series group are kept by the graph for the group: the series group
/// has them after the graph is drawn again, the graph's data reads them back unchanged after it is written to JSON, its
/// paths and the other series stay opaque and Normal, setting them again records no step, 100% and Normal forget them,
/// and undo takes them back.
#[test]
fn an_opacity_and_blend_mode_on_a_series_group_survive_regeneration() {
    let (mut s, id, _, _) = column_marks();
    let (group, _) = series_0_group(&s, id);
    s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 40, "blend": "multiply"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.group_opacity, kept.group_blend), (Some(0.4), Some(BlendMode::Multiply)));
    assert_eq!((kept.opacity, kept.blend), (None, None), "the paths keep their own");
    let restored: GraphSpec = serde_json::from_value(serde_json::to_value(spec(&s, id)).unwrap()).unwrap();
    assert_eq!(restored, spec(&s, id));
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    let normal = (1.0, BlendMode::Normal);
    let (own, paths) = group_transparency(&s, id, 0);
    assert_eq!(own, (0.4, BlendMode::Multiply));
    assert!(!paths.is_empty() && paths.iter().all(|t| *t == normal), "{paths:?}");
    let (own, paths) = group_transparency(&s, id, 1);
    assert_eq!(own, normal);
    assert!(paths.iter().all(|t| *t == normal), "{paths:?}");

    let (group, _) = series_0_group(&s, id);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("object.setProps", &json!({"id": group.0, "opacity": 40, "blend": "multiply"})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "the values the group and the graph have already");
    s.execute("object.setProps", &json!({"id": group.0, "opacity": 100, "blend": "normal"})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.group_opacity, kept.group_blend), (None, None));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"rows": [[2, 2], [3, 4]]})).unwrap();
    assert_eq!(group_transparency(&s, id, 0).0, normal);
    for _ in 0..4 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert!(spec(&s, id).series_paints.is_empty(), "undo takes back the edit and what the graph stored");
}

/// Commands that clear a series group's own opacity and blend mode also remove what the graph keeps for it: Clear
/// Appearance on the group, a graphic style applied to it and Move Appearance onto it.
#[test]
fn clearing_a_series_groups_opacity_forgets_what_the_graph_keeps() {
    let kept = |s: &Session, id: NodeId| {
        let slot = spec(s, id).series_paints[0].clone();
        (slot.group_opacity, slot.group_blend)
    };
    let faded = (Some(0.5), Some(BlendMode::Screen));
    let (mut s, id, _, _) = column_marks();
    let (rect, _) = blue_style(&mut s, "Blue", false);
    let (group, _) = series_0_group(&s, id);
    s.execute("object.setProps", &json!({"id": group.0, "opacity": 50, "blend": "screen"})).unwrap();
    assert_eq!(kept(&s, id), faded);
    let commands = [
        ("appearance.clear", json!({"ids": [group.0]})),
        ("graphicStyle.apply", json!({"name": "Blue", "ids": [group.0]})),
        ("appearance.transfer", json!({"source": rect.0, "target": group.0, "copy": true})),
    ];
    for (cmd, params) in commands {
        s.execute(cmd, &params).unwrap();
        assert_eq!(kept(&s, id), (None, None), "{cmd}");
        s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
        assert_eq!(group_transparency(&s, id, 0).0, (1.0, BlendMode::Normal), "{cmd}");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(kept(&s, id), faded, "{cmd}");
    }
}

/// Redefining a graphic style a series group is linked to clears the group's own opacity and blend mode, and removes
/// what the graph keeps for them.
#[test]
fn redefining_a_style_linked_to_a_series_group_forgets_its_kept_opacity() {
    let (mut s, id, _, _) = column_marks();
    let (group, _) = series_0_group(&s, id);
    let (rect, _) = blue_style(&mut s, "Blue", false);
    let style = s.doc().unwrap().doc.graphic_style("Blue").unwrap().clone();
    s.edit("Link", |d, _| {
        let i = d.graphic_style_index("Blue").unwrap();
        (d.graphic_styles[i].opacity, d.graphic_styles[i].blend) = (0.5, BlendMode::Screen);
        let g = d.node_mut(group).unwrap();
        g.appearance = style.appearance.clone().into();
        (g.opacity, g.blend) = (0.5, BlendMode::Screen);
        g.extras_mut().graphic_style = Some(style.id);
        Ok(())
    })
    .unwrap();
    s.execute("object.setProps", &json!({"id": group.0, "opacity": 50, "blend": "screen"})).unwrap();
    let slot = spec(&s, id).series_paints[0].clone();
    assert_eq!((slot.group_opacity, slot.group_blend), (Some(0.5), Some(BlendMode::Screen)));
    assert_eq!(s.doc().unwrap().doc.node(group).unwrap().extras().graphic_style, Some(style.id), "still linked");
    s.execute("appearance.setItem", &json!({"ids": [rect.0], "index": top_row(&s, rect, true), "color": "#00ff00"})).unwrap();
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "id": rect.0})).unwrap();
    let slot = spec(&s, id).series_paints.first().cloned().unwrap_or_default();
    assert_eq!((slot.group_opacity, slot.group_blend), (None, None));
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    assert_eq!(group_transparency(&s, id, 0).0, (1.0, BlendMode::Normal));
}

/// Isolate Blending, a knockout state and Knockout Shape set on a selected series group are kept by the graph for the
/// group, through the graph being drawn again; the other series' group stays as the graph draws it, and turning them off
/// forgets them.
#[test]
fn isolation_and_knockout_on_a_series_group_survive_regeneration() {
    let (mut s, id, _, _) = column_marks();
    let (group, _) = series_0_group(&s, id);
    s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
    s.execute("transparency.set", &json!({"isolate": true, "knockout": "off", "knockoutShape": true})).unwrap();
    let slot = spec(&s, id).series_paints[0].clone();
    assert_eq!((slot.group_isolate, slot.group_knockout, slot.group_knockout_shape), (true, Some(vectorcraft_doc::Knockout::Off), true));
    assert_eq!((slot.group_opacity, slot.group_blend, slot.opacity, slot.blend), (None, None, None, None));
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("object.setProps", &json!({"id": group.0, "isolate": true})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "the values the group and the graph have already");
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let (g, other) = (series(&n, 0), series(&n, 1));
    assert_eq!((g.isolate, g.knockout, g.knockout_shape), (true, vectorcraft_doc::Knockout::Off, true));
    assert_eq!((other.isolate, other.knockout, other.knockout_shape), (false, vectorcraft_doc::Knockout::Neutral, false));
    let (group, _) = series_0_group(&s, id);
    s.execute("object.setProps", &json!({"id": group.0, "isolate": false, "knockout": "neutral", "knockoutShape": false})).unwrap();
    assert!(spec(&s, id).series_paints.iter().all(|p| *p == vectorcraft_doc::SeriesPaint::default()), "nothing is kept");
}

/// Randomize Opacity and Sort Objects given a series group keep the opacity they give it for the group.
#[test]
fn randomize_and_sort_opacity_on_a_series_group_are_kept() {
    let (mut s, id, _, _) = column_marks();
    let (group, _) = series_0_group(&s, id);
    s.execute("addon.arrange.randomizeOpacity", &json!({"ids": [group.0], "min": 30, "max": 60, "seed": 7})).unwrap();
    let given = s.doc().unwrap().doc.node(group).unwrap().opacity;
    assert!(given < 1.0);
    assert_eq!(spec(&s, id).series_paints[0].group_opacity, Some(given));
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    assert_eq!(group_transparency(&s, id, 0).0, (given, BlendMode::Normal));

    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let (first, second) = (series(&n, 0).id, series(&n, 1).id);
    s.execute("object.setProps", &json!({"id": second.0, "opacity": 80})).unwrap();
    s.execute("addon.arrange.sort", &json!({"ids": [first.0, second.0], "by": "x", "apply": "opacity", "order": "descending"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let (a, b) = (d.node(first).unwrap().opacity, d.node(second).unwrap().opacity);
    assert_eq!((a, b), (0.8, given), "the opacities trade places");
    let kept = &spec(&s, id).series_paints;
    assert_eq!((kept[0].group_opacity, kept.get(1).and_then(|k| k.group_opacity)), ((a < 1.0).then_some(a), (b < 1.0).then_some(b)));
}

/// Default Fill and Stroke on a series mark stores a white fill and a 1 pt black stroke, both opaque, on the graph.
#[test]
fn default_fill_and_stroke_on_a_series_mark_is_kept() {
    let (mut s, id, marks) = stored_red_stroke();
    let was = spec(&s, id).series_paints;
    s.execute("paint.default", &json!({"ids": [marks[0].0]})).unwrap();
    let (white, black) = (Paint::solid(vectorcraft_color::Color::WHITE), Paint::solid(vectorcraft_color::Color::BLACK));
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill.clone(), kept.fill_opacity), (Some(white.clone()), None));
    assert_eq!((kept.stroke.clone(), kept.stroke_width), (Some(black.clone()), Some(1.0)));
    series_0_is(&mut s, id, (&white, 1.0, &black, 1.0), &was);
}

/// Swap Fill and Stroke on a series mark stores the swapped paints and the stroke weight, and leaves the opacity the series
/// stores.
#[test]
fn swapping_a_series_marks_fill_and_stroke_is_kept() {
    let (mut s, id, marks) = stored_red_stroke();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#00ff00", "weight": 3})).unwrap();
    let mark = s.doc().unwrap().doc.node(marks[0]).unwrap().clone();
    let (gray, green) = (mark.appearance.fill_paint(), mark.appearance.stroke_paint());
    let was = spec(&s, id).series_paints;
    s.execute("paint.swap", &json!({"ids": [marks[0].0]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert_eq!((kept.fill.clone(), kept.fill_opacity), (Some(green.clone()), Some(0.4)));
    assert_eq!((kept.stroke.clone(), kept.stroke_width), (Some(gray.clone()), Some(3.0)));
    series_0_is(&mut s, id, (&green, 0.4, &gray, 3.0), &was);
}

/// Swap Fill and Stroke writes the topmost fill even when it is hidden: the visible fill under it does not replace the
/// fill the series stores.
#[test]
fn swapping_onto_a_hidden_top_fill_leaves_the_stored_fill() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("appearance.addFill", &json!({"ids": [marks[0].0]})).unwrap();
    let (_, upper) = fill_rows(&s, marks[0]);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": upper, "visible": false})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": top_row(&s, marks[1], true), "color": "#ff0000"})).unwrap();
    let red = spec(&s, id).series_paints[0].fill.clone();
    assert_eq!(red, Some(s.doc().unwrap().doc.node(marks[1]).unwrap().appearance.fill_paint()));
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#00ff00", "weight": 3})).unwrap();
    s.execute("paint.swap", &json!({"ids": [marks[0].0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, red);
}

/// A column graph whose series 0 stores a red fill and a red 2 pt stroke set on its second mark, while the topmost fill
/// (over a visible one) and the green stroke of its first mark are hidden: the graph and the first mark.
fn hidden_top_rows() -> (Session, NodeId, NodeId) {
    let (mut s, id, _, marks) = column_marks();
    s.execute("appearance.addFill", &json!({"ids": [marks[0].0]})).unwrap();
    let (_, upper) = fill_rows(&s, marks[0]);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": upper, "visible": false})).unwrap();
    let stroke = top_row(&s, marks[0], false);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": stroke, "color": "#00ff00", "weight": 3, "visible": false})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": top_row(&s, marks[1], true), "color": "#ff0000"})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": top_row(&s, marks[1], false), "color": "#ff0000", "weight": 2})).unwrap();
    (s, id, marks[0])
}

/// `paint.setFill`, `paint.setStroke`, `paint.none`, Apply Fill to Stroke and Apply Stroke to Fill write the topmost row
/// of the kind even when it is hidden. The mark shows what it showed before, and the series keeps the paint it stored.
#[test]
fn a_paint_set_on_a_hidden_top_row_leaves_the_stored_paint() {
    let cases = [
        ("paint.setFill", json!({"color": "#0000ff"})),
        ("paint.setFill", json!({"none": true})),
        ("paint.setStroke", json!({"color": "#0000ff"})),
        ("paint.setStroke", json!({"none": true})),
        ("paint.none", json!({})),
        ("addon.color.fillToStroke", json!({})),
        ("addon.color.strokeToFill", json!({})),
    ];
    every_case(cases, |(cmd, params)| {
        let (mut s, id, mark) = hidden_top_rows();
        let was = spec(&s, id).series_paints;
        let red = was[0].fill.clone().unwrap();
        assert_eq!((was[0].stroke.clone(), was[0].stroke_width), (Some(red.clone()), Some(2.0)));
        let mut p = params.clone();
        if *cmd == "paint.none" {
            s.execute("select.set", &json!({"ids": [mark.0]})).unwrap();
        } else {
            p["ids"] = json!([mark.0]);
        }
        let before = s.doc().unwrap().doc.node(mark).unwrap().clone();
        s.execute(cmd, &p).unwrap();
        assert_ne!(*s.doc().unwrap().doc.node(mark).unwrap(), before, "the command wrote the hidden row");
        assert_eq!(spec(&s, id).series_paints, was);
        s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
        assert!(marks_of(&s, id, 0).iter().all(|m| *m == (red.clone(), red.clone(), 2.0)), "{:?}", marks_of(&s, id, 0));
    });
}

/// `paint.setFill` with `item` on the visible fill under a hidden topmost one changes what the mark shows, and the series
/// stores it. On a fill under a visible topmost one, the series keeps the paint of the topmost one.
#[test]
fn a_paint_set_on_a_row_under_the_top_one_is_stored_only_when_it_shows() {
    let (mut s, id, mark) = hidden_top_rows();
    let (lower, _) = fill_rows(&s, mark);
    s.execute("paint.setFill", &json!({"ids": [mark.0], "item": lower, "color": "#0000ff"})).unwrap();
    let blue = s.doc().unwrap().doc.node(mark).unwrap().appearance.paint_at(Some(lower), true).cloned();
    assert!(blue.as_ref().is_some_and(|p| !p.is_none()));
    assert_eq!(spec(&s, id).series_paints[0].fill, blue);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_ne!(spec(&s, id).series_paints[0].fill, blue);

    // The first mark has a green fill over another one, and the series stores the red fill of the second mark.
    let (mut s, id, _, marks) = column_marks();
    s.execute("appearance.addFill", &json!({"ids": [marks[0].0]})).unwrap();
    let (lower, upper) = fill_rows(&s, marks[0]);
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": upper, "color": "#00ff00"})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": top_row(&s, marks[1], true), "color": "#ff0000"})).unwrap();
    let red = Some(fill_of(&s, marks[1]));
    assert_ne!(red, Some(fill_of(&s, marks[0])));
    assert_eq!(spec(&s, id).series_paints[0].fill, red);
    s.execute("paint.setFill", &json!({"ids": [marks[0].0], "item": lower, "color": "#0000ff"})).unwrap();
    assert_ne!(s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.paint_at(Some(lower), true), red.as_ref());
    assert_eq!(spec(&s, id).series_paints[0].fill, red);
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| Some(&m.0) == red.as_ref()), "{:?}", marks_of(&s, id, 0));
}

/// A library gradient applied to a selected mark goes to the topmost fill as `paint.setFill` puts it there: the series
/// stores it from a visible row, and keeps its stored paint when that row is hidden.
#[test]
fn a_library_gradient_on_a_series_mark_is_stored_only_from_a_row_it_shows() {
    for hidden in [false, true] {
        let (mut s, id, mark) = hidden_top_rows();
        if !hidden {
            s.execute("appearance.showAllHidden", &json!({"ids": [mark.0]})).unwrap();
        }
        let lib = s.execute("library.create", &json!({})).unwrap()["id"].as_str().unwrap().to_string();
        let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 20})).unwrap()["id"].clone();
        let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
        s.execute("paint.setFill", &json!({"gradient": {"kind": "linear", "stops": stops}, "ids": [rect]})).unwrap();
        s.execute("select.set", &json!({"ids": [rect]})).unwrap();
        s.execute("library.add", &json!({"library": lib, "kind": "fillGradient"})).unwrap();
        s.execute("select.set", &json!({"ids": [mark.0]})).unwrap();
        let was = spec(&s, id).series_paints;
        s.execute("library.use", &json!({"library": lib, "kind": "fillGradient", "item": "Gradient"})).unwrap();
        let fill = fill_of(&s, mark);
        assert!(matches!(fill, Paint::Gradient(_)), "{hidden}");
        let now = spec(&s, id).series_paints;
        if hidden {
            assert_eq!(now, was);
        } else {
            assert_eq!(now[0].fill, Some(fill));
        }
    }
}

/// A mark's fill and stroke rows already have the paint a command writes, while the series stores the paint of another
/// mark. `paint.setFill`, `paint.setStroke`, Apply Fill to Stroke and Apply Stroke to Fill leave the mark as it was, and
/// the series stores the mark's paint for the kind of row the command wrote, and only for that kind.
#[test]
fn writing_the_paint_a_mark_already_has_stores_it_for_the_series() {
    let cases = [
        ("paint.setFill", json!({"color": "#ff0000"}), true),
        ("paint.setStroke", json!({"color": "#ff0000"}), false),
        ("addon.color.fillToStroke", json!({}), false),
        ("addon.color.strokeToFill", json!({}), true),
        ("addon.color.fillToStroke", json!({"skipNone": true}), false),
    ];
    every_case(cases, |(cmd, params, fill)| {
        let (mut s, id, _, marks) = column_marks();
        for (mark, hex, weight) in [(marks[0], "#ff0000", 3), (marks[1], "#0000ff", 2)] {
            s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, true), "color": hex})).unwrap();
            s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, false), "color": hex, "weight": weight})).unwrap();
        }
        let was = spec(&s, id).series_paints;
        let (red, blue) = (fill_of(&s, marks[0]), fill_of(&s, marks[1]));
        assert_eq!((was[0].fill.clone(), was[0].stroke.clone(), was[0].stroke_width), (Some(blue.clone()), Some(blue.clone()), Some(2.0)));
        let before = s.doc().unwrap().doc.node(marks[0]).unwrap().clone();
        let mut p = params.clone();
        p["ids"] = json!([marks[0].0]);
        s.execute(cmd, &p).unwrap();
        assert_eq!(*s.doc().unwrap().doc.node(marks[0]).unwrap(), before, "the mark is as it was");
        let now = spec(&s, id).series_paints;
        let expected = if *fill { (Some(red), Some(blue), Some(2.0)) } else { (Some(blue), Some(red), Some(3.0)) };
        assert_eq!((now[0].fill.clone(), now[0].stroke.clone(), now[0].stroke_width), expected);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(spec(&s, id).series_paints, was);
    });
}

/// Apply Fill to Stroke and Apply Stroke to Fill on a mark store the paint they give its topmost visible row, and every
/// mark of the series has it after the graph is drawn again. With `skipNone`, a mark without the source paint keeps what
/// the series stores.
#[test]
fn applying_one_paint_kind_to_the_other_on_a_series_mark_is_kept() {
    let (mut s, id, marks) = stored_red_stroke();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": top_row(&s, marks[0], true), "color": "#0000ff"})).unwrap();
    let blue = fill_of(&s, marks[0]);
    s.execute("addon.color.fillToStroke", &json!({"ids": [marks[0].0]})).unwrap();
    let kept = spec(&s, id).series_paints;
    assert_eq!((kept[0].fill.clone(), kept[0].stroke.clone()), (Some(blue.clone()), Some(blue.clone())));
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| (&m.0, &m.1) == (&blue, &blue)), "{:?}", marks_of(&s, id, 0));

    let (mut s, id, marks) = stored_red_stroke();
    let red = spec(&s, id).series_paints[0].stroke.clone();
    s.execute("addon.color.strokeToFill", &json!({"ids": [marks[1].0]})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].fill, red);
    s.execute("paint.setFill", &json!({"ids": [marks[0].0], "none": true})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[1].0], "index": top_row(&s, marks[1], true), "color": "#0000ff"})).unwrap();
    let was = spec(&s, id).series_paints;
    s.execute("addon.color.fillToStroke", &json!({"ids": [marks[0].0], "skipNone": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints, was);
}

/// When one paint command writes several marks of a series with different paints, the frontmost mark sets what the
/// graph stores, whatever order the ids are given in and whatever the marks' ids are.
#[test]
fn a_paint_command_on_several_marks_of_a_series_stores_the_frontmost_ones_paint() {
    every_case([[0, 1, 2], [2, 1, 0], [1, 2, 0]], |order| {
        let (mut s, id, _, marks) = column_marks();
        // The mark made last goes to the back, so the marks' ids are not in paint order.
        s.execute("select.set", &json!({"ids": [marks[2].0]})).unwrap();
        s.execute("object.arrange.sendToBack", &json!({})).unwrap();
        let drawn: Vec<NodeId> = series_parts(&s, id, 0).iter().map(|m| m.id).filter(|m| marks[..3].contains(m)).collect();
        assert_eq!(drawn, [marks[2], marks[0], marks[1]]);
        for (mark, hex) in [(marks[0], "#ff0000"), (marks[1], "#00ff00"), (marks[2], "#0000ff")] {
            s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, true), "color": hex})).unwrap();
        }
        s.execute("addon.color.fillToStroke", &json!({"ids": order.map(|i| marks[i].0)})).unwrap();
        assert_eq!(spec(&s, id).series_paints[0].stroke, Some(fill_of(&s, marks[1])), "{order:?}");
    });

    // The frontmost mark loses its stroke and a mark under it takes one: the series stores the frontmost mark's.
    let (mut s, id, marks) = stored_red_stroke();
    s.execute("appearance.setItem", &json!({"ids": [marks[2].0], "index": top_row(&s, marks[2], false), "color": "#00ff00", "weight": 2})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [marks[2].0], "none": true})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": top_row(&s, marks[0], true), "color": "#0000ff"})).unwrap();
    s.execute("addon.color.fillToStroke", &json!({"ids": [marks[0].0, marks[2].0]})).unwrap();
    assert!(!s.doc().unwrap().doc.node(marks[0]).unwrap().appearance.stroke_paint().is_none());
    assert_eq!(spec(&s, id).series_paints[0].stroke, Some(Paint::None));
}

/// Apply Fill to Stroke on two marks whose strokes already have their fills, while the series stores the front mark's
/// stroke: the back mark's stroke is taken first and the front mark's after it, so the graph ends up storing what it
/// stored before. The document and its undo history stay as they are.
#[test]
fn a_paint_command_that_leaves_a_series_stored_paint_as_it_was_records_no_undo_step() {
    let (mut s, id, _, marks) = column_marks();
    for (mark, hex, weight) in [(marks[0], "#ff0000", 3), (marks[1], "#0000ff", 2)] {
        s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, true), "color": hex})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, false), "color": hex, "weight": weight})).unwrap();
    }
    let drawn: Vec<NodeId> = series_parts(&s, id, 0).iter().map(|m| m.id).filter(|m| marks[..2].contains(m)).collect();
    assert_eq!(drawn, [marks[0], marks[1]], "the second mark is in front");
    let was = s.doc().unwrap().doc.clone();
    let blue = fill_of(&s, marks[1]);
    assert_eq!(spec(&s, id).series_paints[0].stroke, Some(blue));
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("addon.color.fillToStroke", &json!({"ids": [marks[1].0, marks[0].0]})).unwrap();
    assert_eq!(s.doc().unwrap().doc, was);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

/// A stroke set on a Filled Lines band with `paint.setStroke` or Apply Fill to Stroke is not stored: the band keeps its
/// color and its thin black outline when the graph is drawn again. A fill set with `paint.setFill` is stored.
#[test]
fn a_paint_command_stroke_on_a_filled_line_band_does_not_repaint_it() {
    every_case([("paint.setStroke", json!({"color": "#ff0000"})), ("addon.color.fillToStroke", json!({}))], |(cmd, params)| {
        let (mut s, id, band) = band_graph();
        let was = series_parts(&s, id, 0)[0].appearance.fill_paint();
        let mut p = params.clone();
        p["ids"] = json!([band.0]);
        s.execute(cmd, &p).unwrap();
        assert!(!s.doc().unwrap().doc.node(band).unwrap().appearance.stroke_paint().is_none());
        assert!(spec(&s, id).series_paints.first().is_none_or(|p| p.stroke.is_none()));
        assert_eq!(redrawn_band(&mut s, id), (was, Paint::solid(vectorcraft_color::Color::BLACK), 0.25));
    });
    let (mut s, id, band) = band_graph();
    s.execute("paint.setFill", &json!({"ids": [band.0], "color": "#0000ff"})).unwrap();
    let blue = s.doc().unwrap().doc.node(band).unwrap().appearance.fill_paint();
    assert_eq!(redrawn_band(&mut s, id), (blue, Paint::solid(vectorcraft_color::Color::BLACK), 0.25));
}

/// `paint.setStroke` with None on a marker that shows no stroke changes nothing, and the line keeps the stroke the series
/// stores. On the line itself it stores the empty stroke.
#[test]
fn removing_the_stroke_of_a_mark_without_one_leaves_the_stored_stroke() {
    let (mut s, id, line, marker) = line_and_marker();
    s.execute("paint.setStroke", &json!({"ids": [line.0], "color": "#ff0000"})).unwrap();
    let red = s.doc().unwrap().doc.node(line).unwrap().appearance.stroke_paint();
    assert_eq!(spec(&s, id).series_paints[0].stroke, Some(red.clone()));
    s.execute("paint.setStroke", &json!({"ids": [marker.0], "none": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke, Some(red.clone()));
    s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
    assert_eq!(line_parts(&s, id).0.appearance.stroke_paint(), red);

    let line = line_parts(&s, id).0.id;
    s.execute("paint.setStroke", &json!({"ids": [line.0], "none": true})).unwrap();
    assert_eq!(spec(&s, id).series_paints[0].stroke, Some(Paint::None));
}

/// Run `case` on each of `cases`, and fail naming every case that failed, not only the first one.
fn every_case<T: std::fmt::Debug>(cases: impl IntoIterator<Item = T>, case: impl Fn(&T)) {
    let failed: Vec<T> = cases.into_iter().filter(|c| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| case(c))).is_err()).collect();
    assert!(failed.is_empty(), "failed cases: {failed:?}");
}

/// The top fill paint of `mark`.
fn fill_of(s: &Session, mark: NodeId) -> Paint {
    s.doc().unwrap().doc.node(mark).unwrap().appearance.fill_paint()
}

/// A stroke weight set with `stroke.set`, or stepped with `stroke.weightUp` or `stroke.weightDown`, on a series mark is
/// stored on the graph with the mark's stroke paint, and undo takes it back with its edit.
#[test]
fn a_stroke_weight_set_on_a_series_mark_is_kept() {
    let cases = [("stroke.set", json!({"weight": 7}), 7.0), ("stroke.weightUp", json!({}), 3.0), ("stroke.weightDown", json!({}), 1.0)];
    every_case(cases, |(cmd, params, weight)| {
        let (mut s, id, marks) = stored_red_stroke();
        let was = spec(&s, id).series_paints;
        let mut p = params.clone();
        p["ids"] = json!([marks[1].0]);
        s.execute(cmd, &p).unwrap();
        let mark = s.doc().unwrap().doc.node(marks[1]).unwrap().clone();
        let (fill, stroke) = (mark.appearance.fill_paint(), mark.appearance.stroke_paint());
        let kept = spec(&s, id).series_paints[0].clone();
        assert_eq!((kept.stroke.clone(), kept.stroke_width), (Some(stroke.clone()), Some(*weight)), "{cmd}");
        series_0_is(&mut s, id, (&fill, 0.4, &stroke, *weight), &was);
    });
}

/// A gradient made on a series mark's fill with `paint.editGradient` is stored on the graph, every mark of the series has
/// that fill after the graph is drawn again, and undo takes it back with its edit.
#[test]
fn a_gradient_edit_on_a_series_mark_is_kept() {
    let (mut s, id, marks) = stored_red_stroke();
    let was = spec(&s, id).series_paints;
    s.execute("paint.editGradient", &json!({"ids": [marks[0].0], "kind": "linear"})).unwrap();
    let fill = fill_of(&s, marks[0]);
    assert!(matches!(fill, Paint::Gradient(_)), "{fill:?}");
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(fill.clone()));
    let red = was[0].stroke.clone().unwrap();
    series_0_is(&mut s, id, (&fill, 0.4, &red, 2.0), &was);
}

/// Where gradient `paint` sits on box `b`: its vector's start and end, then each freeform point, as fractions of the box.
fn gradient_place(paint: &Paint, b: vectorcraft_geom::Rect) -> Vec<f64> {
    let Paint::Gradient(g) = paint else { panic!("not a gradient: {paint:?}") };
    let points = g.geom.iter().flat_map(|geom| [geom.start, geom.end]).chain(g.freeform.iter().flat_map(|f| f.points.iter().map(|p| p.at)));
    points.flat_map(|p| [(p.x - b.x0) / b.width(), (p.y - b.y0) / b.height()]).collect()
}

/// The box a gradient on `mark`'s fill (`stroke` false) or stroke is placed on.
fn paint_box_of(mark: &vectorcraft_doc::Node, stroke: bool) -> vectorcraft_geom::Rect {
    let b = mark.geometric_bounds().unwrap();
    if stroke { vectorcraft_doc::appearance::stroke_paint_bounds(b, mark.appearance.stroke_width()) } else { b }
}

/// The fill (`stroke` false) or stroke of every path of series 0, the legend swatch included, has a gradient at `place` on
/// its own box, and the gradients are not all at one place in the document. Returns how many paths were checked.
fn series_0_gradients_at(s: &Session, id: NodeId, stroke: bool, place: &[f64]) -> usize {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let paths: Vec<_> = series(&n, 0).children().unwrap().iter().filter(|m| matches!(m.kind, NodeKind::Path { .. })).cloned().collect();
    let paints: Vec<Paint> = paths.iter().map(|m| if stroke { m.appearance.stroke_paint() } else { m.appearance.fill_paint() }).collect();
    for (m, paint) in paths.iter().zip(&paints) {
        let at = gradient_place(paint, paint_box_of(m, stroke));
        assert_eq!(at.len(), place.len(), "{at:?}");
        assert!(at.iter().zip(place).all(|(a, b)| (a - b).abs() < 1e-9), "{at:?} is not {place:?}");
    }
    assert!(paints.windows(2).any(|w| w[0] != w[1]), "every mark has the gradient at one place: {paints:?}");
    paths.len()
}

/// A gradient placed on a series mark's fill with `paint.setGradientGeom` is stored relative to the mark's box, and every
/// mark of the series has it at the same place on its own box after the graph is drawn again, also after a data change
/// moves and resizes the marks and after the graph is moved.
#[test]
fn a_gradient_placed_on_a_series_mark_takes_the_same_place_on_every_mark() {
    let (mut s, id, _, marks) = column_marks();
    // `paint.setFill` stores the gradient before it is placed.
    s.execute("paint.setFill", &json!({"ids": [marks[0].0], "gradient": {"kind": "linear"}})).unwrap();
    let fill = fill_of(&s, marks[0]);
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(fill.clone()));
    s.execute("paint.setGradientGeom", &json!({"ids": [marks[0].0], "start": [0, 0], "end": [10, 40]})).unwrap();
    let placed = fill_of(&s, marks[0]);
    assert_ne!(placed, fill, "the command places the gradient");
    let mark = s.doc().unwrap().doc.node(marks[0]).unwrap().clone();
    let place = gradient_place(&placed, paint_box_of(&mark, false));
    let kept = spec(&s, id).series_paints[0].clone();
    assert!(kept.fill_unit_box);
    assert_eq!(gradient_place(kept.fill.as_ref().unwrap(), vectorcraft_doc::GraphicStyle::UNIT_BOX), place);
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4], [6, 1]]})).unwrap();
    assert!(series_0_gradients_at(&s, id, false, &place) >= 4);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 50, "dy": 30})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[2, 2], [5, 4], [1, 1]]})).unwrap();
    series_0_gradients_at(&s, id, false, &place);
}

/// A gradient placed on a series mark's stroke is kept relative to the mark's stroke box, and every mark of the series has
/// it at the same place on its own stroke box after the graph is drawn again.
#[test]
fn a_gradient_placed_on_a_series_marks_stroke_takes_the_same_place_on_every_mark() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("paint.setStroke", &json!({"ids": [marks[1].0], "gradient": {"kind": "radial"}})).unwrap();
    s.execute("stroke.set", &json!({"ids": [marks[1].0], "weight": 4})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [marks[1].0], "stroke": true, "start": [150, 200], "end": [170, 260]})).unwrap();
    let mark = s.doc().unwrap().doc.node(marks[1]).unwrap().clone();
    let place = gradient_place(&mark.appearance.stroke_paint(), paint_box_of(&mark, true));
    assert!(spec(&s, id).series_paints[0].stroke_unit_box);
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4], [6, 1]]})).unwrap();
    series_0_gradients_at(&s, id, true, &place);
}

/// A placed gradient picked up with the Eyedropper from art outside the graph is kept relative to the series mark it is
/// dropped on, and every mark of the series has it at the same place on its own box.
#[test]
fn a_placed_gradient_dropped_on_a_series_mark_takes_the_same_place_on_every_mark() {
    let (mut s, id, _, marks) = column_marks();
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [r.0], "gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [r.0], "start": [610, 600], "end": [630, 620]})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": r.0, "ids": [marks[2].0]})).unwrap();
    let mark = s.doc().unwrap().doc.node(marks[2]).unwrap().clone();
    let place = gradient_place(&mark.appearance.fill_paint(), paint_box_of(&mark, false));
    assert!(spec(&s, id).series_paints[0].fill_unit_box);
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4], [6, 1]]})).unwrap();
    series_0_gradients_at(&s, id, false, &place);
}

/// A graphic style with a placed gradient applied to a series mark is kept relative to that mark's box, and every mark of
/// the series has the gradient at the same place on its own box.
#[test]
fn a_graphic_style_gradient_applied_to_a_series_mark_takes_the_same_place_on_every_mark() {
    let (mut s, id, _, marks) = column_marks();
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [r.0], "gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [r.0], "start": [610, 600], "end": [630, 620]})).unwrap();
    s.execute("graphicStyle.new", &json!({"id": r.0, "name": "Placed"})).unwrap();
    s.execute("graphicStyle.apply", &json!({"name": "Placed", "ids": [marks[2].0]})).unwrap();
    let mark = s.doc().unwrap().doc.node(marks[2]).unwrap().clone();
    let place = gradient_place(&mark.appearance.fill_paint(), paint_box_of(&mark, false));
    assert!(spec(&s, id).series_paints[0].fill_unit_box);
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4], [6, 1]]})).unwrap();
    series_0_gradients_at(&s, id, false, &place);
}

/// A point recolored with `paint.freeform.setPoint` in a freeform gradient on a series mark's fill is stored on the graph,
/// and every mark of the series has the gradient's points at the same place on its own box after the graph is drawn again.
#[test]
fn a_freeform_gradient_edit_on_a_series_mark_is_kept() {
    let (mut s, id, _, marks) = column_marks();
    let points = json!([{"at": [20, 20], "color": "#53aadf"}, {"at": [50, 50], "color": "#ff8000"}]);
    // `paint.setFill` stores the gradient before its point is recolored.
    s.execute("paint.setFill", &json!({"ids": [marks[0].0], "gradient": {"kind": "freeform", "freeform": {"points": points}}})).unwrap();
    s.execute("paint.freeform.setPoint", &json!({"ids": [marks[0].0], "index": 1, "color": "#00ff00"})).unwrap();
    let edited = fill_of(&s, marks[0]);
    let Paint::Gradient(g) = &edited else { panic!("{edited:?}") };
    assert_eq!(g.freeform.as_ref().unwrap().points[1].color.to_hex(), "#00ff00");
    let mark = s.doc().unwrap().doc.node(marks[0]).unwrap().clone();
    let place = gradient_place(&edited, paint_box_of(&mark, false));
    let kept = spec(&s, id).series_paints[0].clone();
    assert!(kept.fill_unit_box);
    assert_eq!(gradient_place(kept.fill.as_ref().unwrap(), vectorcraft_doc::GraphicStyle::UNIT_BOX), place);
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    series_0_gradients_at(&s, id, false, &place);
}

/// A gradient a graph kept in document coordinates (a file saved before gradients were kept relative to the unit box) is
/// drawn on every mark as it was kept.
#[test]
fn a_gradient_kept_in_document_coordinates_is_drawn_as_kept() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("paint.setFill", &json!({"ids": [marks[0].0], "gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [marks[0].0], "start": [0, 0], "end": [10, 40]})).unwrap();
    let placed = fill_of(&s, marks[0]);
    s.edit("Kept in document coordinates", |d, _| {
        let mut extras = d.node_mut(id).unwrap().extras_mut();
        let slot = &mut extras.graph.as_mut().unwrap().series_paints[0];
        (slot.fill, slot.fill_unit_box) = (Some(placed.clone()), false);
        Ok(())
    })
    .unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| m.0 == placed), "{:?}", marks_of(&s, id, 0));
    // The same gradient kept for the fill relative to the unit box leaves the stroke kept in document coordinates as it is.
    s.edit("Kept fill and stroke", |d, _| {
        let mut extras = d.node_mut(id).unwrap().extras_mut();
        let slot = &mut extras.graph.as_mut().unwrap().series_paints[0];
        (slot.fill, slot.fill_unit_box) = (Some(placed.clone()), true);
        (slot.stroke, slot.stroke_unit_box, slot.stroke_width) = (Some(placed.clone()), false, Some(2.0));
        Ok(())
    })
    .unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[2, 2], [3, 4]]})).unwrap();
    let drawn = marks_of(&s, id, 0);
    assert!(drawn.iter().all(|m| m.1 == placed), "{drawn:?}");
    series_0_gradients_at(&s, id, false, &gradient_place(&placed, vectorcraft_doc::GraphicStyle::UNIT_BOX));
}

/// A gradient placed on a series path with no height (a flat line's fill) is kept in document coordinates, and every
/// path of the series that takes the kept fill has it at that place in the document.
#[test]
fn a_gradient_placed_on_a_series_path_with_no_height_is_kept_in_document_coordinates() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "line");
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[3, 2], [3, 4], [3, 6]]})).unwrap();
    let flat = |s: &Session| {
        let parts = series_parts(s, id, 0);
        parts.into_iter().find(|m| m.appearance.fill_paint().is_none() && m.geometric_bounds().is_some_and(|b| b.height() < 1e-9)).unwrap().id
    };
    let line = flat(&s);
    s.execute("paint.setFill", &json!({"ids": [line.0], "gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [line.0], "start": [150, 120], "end": [300, 280]})).unwrap();
    let placed = fill_of(&s, line);
    let kept = spec(&s, id).series_paints[0].clone();
    assert!(!kept.fill_unit_box);
    assert_eq!(kept.fill, Some(placed.clone()));
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[3, 1], [3, 5], [3, 2]]})).unwrap();
    let fills: Vec<Paint> = series_parts(&s, id, 0).iter().map(|m| m.appearance.fill_paint()).filter(|p| matches!(p, Paint::Gradient(_))).collect();
    assert!(fills.len() >= 2 && fills.iter().all(|p| *p == placed), "{fills:?}");
}

/// Writing a series mark's stroke weight as it is, after the graph drew the mark's stroke from a gradient the series
/// keeps relative to the unit box, leaves the kept gradient, the document and its undo history as they are, on every mark.
#[test]
fn rewriting_a_series_marks_drawn_gradient_stroke_records_no_undo_step() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("paint.setStroke", &json!({"ids": [marks[1].0], "gradient": {"kind": "radial"}})).unwrap();
    s.execute("stroke.set", &json!({"ids": [marks[1].0], "weight": 4})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [marks[1].0], "stroke": true, "start": [150, 200], "end": [170, 260]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1.3, 2], [3.7, 4], [6.1, 1]]})).unwrap();
    let was = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let drawn: Vec<_> = series_parts(&s, id, 0).into_iter().filter(|m| matches!(m.appearance.stroke_paint(), Paint::Gradient(_))).collect();
    assert!(drawn.len() >= 4, "{}", drawn.len());
    for mark in drawn {
        let row = top_row(&s, mark.id, false);
        s.execute("appearance.setItem", &json!({"ids": [mark.id.0], "index": row, "weight": mark.appearance.stroke_width()})).unwrap();
        assert!(s.doc().unwrap().doc == was, "writing mark {:?} changed the document", mark.id);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
    // A mark moved with its gradient shows the kept gradient on its moved box, up to rounding.
    let kept = spec(&s, id).series_paints[0].clone();
    for (i, mark) in series_parts(&s, id, 0).into_iter().filter(|m| matches!(m.appearance.stroke_paint(), Paint::Gradient(_))).enumerate() {
        s.execute("select.set", &json!({"ids": [mark.id.0]})).unwrap();
        s.execute("object.move", &json!({"dx": 13.37 + i as f64 * 0.731, "dy": -7.19})).unwrap();
        let moved = s.doc().unwrap().doc.node(mark.id).unwrap().geometric_bounds().unwrap();
        assert!((moved.x0 - mark.geometric_bounds().unwrap().x0 - 13.37 - i as f64 * 0.731).abs() < 1e-9, "mark {:?} did not move", mark.id);
        let undo = s.doc().unwrap().history.undo.len();
        let row = top_row(&s, mark.id, false);
        s.execute("appearance.setItem", &json!({"ids": [mark.id.0], "index": row, "weight": mark.appearance.stroke_width()})).unwrap();
        assert_eq!(spec(&s, id).series_paints[0], kept, "writing moved mark {:?} changed the kept stroke", mark.id);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
}

/// Writing the fill of a series mark with no height (a column for a value of 0) as it is, after the graph drew the mark's
/// fill from a gradient the series keeps relative to the unit box, leaves the kept gradient and its unit-box flag as they
/// are.
#[test]
fn rewriting_a_drawn_gradient_fill_on_a_series_mark_with_no_height_keeps_the_kept_gradient() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("paint.setFill", &json!({"ids": [marks[0].0], "gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [marks[0].0], "start": [0, 0], "end": [10, 40]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert!(kept.fill_unit_box);
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[0, 2], [3, 4], [6, 1]]})).unwrap();
    let flat = series_parts(&s, id, 0)
        .into_iter()
        .find(|m| matches!(m.appearance.fill_paint(), Paint::Gradient(_)) && m.geometric_bounds().is_some_and(|b| b.height() < 1e-9))
        .unwrap()
        .id;
    let changed = s.edit("Write the fill as it is", |d, _| Ok(super::capture_series_top_paints(d, &[(flat, true)]))).unwrap();
    assert!(!changed);
    assert_eq!(spec(&s, id).series_paints[0], kept);
}

/// Writing a series mark's stroke weight as it is, after the graph drew the mark's stroke from a gradient the series keeps
/// in document coordinates (a file saved before the unit box), leaves the kept gradient and its unit-box flag as they are
/// and records no undo step.
#[test]
fn rewriting_a_series_marks_stroke_drawn_from_a_gradient_kept_in_document_coordinates_keeps_it() {
    let (mut s, id, _, marks) = column_marks();
    s.execute("paint.setStroke", &json!({"ids": [marks[1].0], "gradient": {"kind": "radial"}})).unwrap();
    s.execute("stroke.set", &json!({"ids": [marks[1].0], "weight": 2})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [marks[1].0], "stroke": true, "start": [150, 200], "end": [170, 260]})).unwrap();
    let placed = s.doc().unwrap().doc.node(marks[1]).unwrap().appearance.stroke_paint();
    s.edit("Kept in document coordinates", |d, _| {
        let mut extras = d.node_mut(id).unwrap().extras_mut();
        let slot = &mut extras.graph.as_mut().unwrap().series_paints[0];
        (slot.stroke, slot.stroke_unit_box, slot.stroke_width) = (Some(placed.clone()), false, Some(2.0));
        Ok(())
    })
    .unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4], [6, 1]]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    let undo = s.doc().unwrap().history.undo.len();
    let drawn: Vec<_> = series_parts(&s, id, 0).into_iter().filter(|m| m.appearance.stroke_paint() == placed).collect();
    assert!(drawn.len() >= 3, "{}", drawn.len());
    for mark in drawn {
        let row = top_row(&s, mark.id, false);
        s.execute("appearance.setItem", &json!({"ids": [mark.id.0], "index": row, "weight": mark.appearance.stroke_width()})).unwrap();
        assert_eq!(spec(&s, id).series_paints[0], kept, "writing mark {:?} changed the kept stroke", mark.id);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
}

/// A gradient placed on a line series' stroke is kept relative to the line's stroke box, and with Draw Filled Lines the
/// band, which takes the kept stroke as its fill, has it at the same place on its own box.
#[test]
fn a_gradient_kept_for_a_lines_stroke_takes_the_same_place_on_its_filled_line() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "line");
    let line = series_parts(&s, id, 0).into_iter().find(|m| m.appearance.fill_paint().is_none()).unwrap().id;
    s.execute("paint.setStroke", &json!({"ids": [line.0], "gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"ids": [line.0], "stroke": true, "start": [150, 120], "end": [300, 280]})).unwrap();
    let kept = spec(&s, id).series_paints[0].clone();
    assert!(kept.stroke_unit_box && kept.fill.is_none());
    let place = gradient_place(kept.stroke.as_ref().unwrap(), vectorcraft_doc::GraphicStyle::UNIT_BOX);
    s.execute("graph.setType", &json!({"id": id.0, "filledLines": true})).unwrap();
    let band = series_parts(&s, id, 0).into_iter().find(|m| matches!(m.appearance.fill_paint(), Paint::Gradient(_))).unwrap();
    let at = gradient_place(&band.appearance.fill_paint(), band.geometric_bounds().unwrap());
    assert!(at.iter().zip(&place).all(|(a, b)| (a - b).abs() < 1e-9), "{at:?} is not {place:?}");
}

/// A series' kept unit-box flags are read from and written to a file, and a file without them reads as document
/// coordinates.
#[test]
fn a_series_paints_unit_box_flags_round_trip() {
    let kept = vectorcraft_doc::SeriesPaint { fill_unit_box: true, stroke_unit_box: true, ..Default::default() };
    let text = serde_json::to_string(&kept).unwrap();
    assert_eq!(text, r#"{"fillUnitBox":true,"strokeUnitBox":true}"#);
    assert_eq!(serde_json::from_str::<vectorcraft_doc::SeriesPaint>(&text).unwrap(), kept);
    let old: vectorcraft_doc::SeriesPaint = serde_json::from_str("{}").unwrap();
    assert!(!old.fill_unit_box && !old.stroke_unit_box);
}

/// A series mark's fill recolored with Invert or Complement (the proxy, or one appearance row), an Edit Colors command,
/// a color filter or Recolor Artwork is stored on the graph, and undo takes it back with its edit.
#[test]
fn a_recolored_series_mark_is_kept() {
    let cases = [
        ("paint.invert", json!({}), false),
        ("paint.complement", json!({}), false),
        ("paint.invert", json!({}), true),
        ("paint.complement", json!({}), true),
        ("edit.colors.invert", json!({}), false),
        ("edit.colors.toGrayscale", json!({}), false),
        ("addon.color.filter", json!({"effect": "addon.color.invert"}), false),
        ("recolor.apply", json!({"map": {"#3366cc": "#00ff00"}}), false),
    ];
    every_case(cases, |(cmd, params, item)| {
        let (mut s, id, marks) = stored_red_stroke();
        // Not the gray the series starts in: Complement leaves a gray as it is.
        s.execute("appearance.setItem", &json!({"ids": [marks[0].0], "index": top_row(&s, marks[0], true), "color": "#3366cc"})).unwrap();
        let was = spec(&s, id).series_paints;
        let mut p = params.clone();
        if *cmd == "recolor.apply" {
            // Recolor Artwork works on the selection.
            s.execute("select.set", &json!({"ids": [marks[0].0]})).unwrap();
        } else {
            p["ids"] = json!([marks[0].0]);
        }
        if *item {
            p["item"] = json!(top_row(&s, marks[0], true));
        }
        let was_fill = fill_of(&s, marks[0]);
        s.execute(cmd, &p).unwrap();
        let fill = fill_of(&s, marks[0]);
        assert_ne!(fill, was_fill, "{cmd} recolors the fill");
        assert_eq!(spec(&s, id).series_paints[0].fill, Some(fill.clone()), "{cmd} item {item}");
        let red = was[0].stroke.clone().unwrap();
        series_0_is(&mut s, id, (&fill, 0.4, &red, 2.0), &was);
    });
}

/// The ink limit of Print Ink Fixes on a series mark's fill is stored on the graph.
#[test]
fn an_ink_limit_on_a_series_mark_is_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"colorMode": "cmyk"})).unwrap();
    let id = graph(&mut s, "column");
    let mark = series(&s.doc().unwrap().doc.node(id).unwrap().clone(), 0).children().unwrap()[0].id;
    // `paint.setFill` stores the heavy color before the limit lowers it.
    s.execute("paint.setFill", &json!({"ids": [mark.0], "color": {"c": 100, "m": 100, "y": 100, "k": 100}})).unwrap();
    let heavy = fill_of(&s, mark);
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(heavy.clone()));
    s.execute("addon.color.printFixes", &json!({"ids": [mark.0], "inkLimit": 300})).unwrap();
    let fill = fill_of(&s, mark);
    assert_ne!(fill, heavy, "the limit lowers the ink");
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(fill.clone()));
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
    assert!(marks_of(&s, id, 0).iter().all(|m| m.0 == fill), "{:?}", marks_of(&s, id, 0));
}

/// A recolor of a whole graph, or of one series group, stores the new fill of each series it reaches, and the marks have
/// it after the graph is drawn again.
#[test]
fn recoloring_a_graph_or_a_series_group_stores_its_marks_fills() {
    let cases = [("edit.colors.invert", true), ("edit.colors.invert", false), ("paint.invert", true), ("paint.invert", false)];
    every_case(cases, |(cmd, whole)| {
        let (mut s, id, gray, _) = column_marks();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let (group, other) = (series(&n, 0).id, series(&n, 1).children().unwrap()[0].clone());
        let target = if *whole { id } else { group };
        s.execute(cmd, &json!({"ids": [target.0]})).unwrap();
        let fill = marks_of(&s, id, 0).last().unwrap().0.clone();
        assert_ne!(fill, gray, "{cmd} recolors the series");
        let kept = spec(&s, id).series_paints;
        assert_eq!(kept[0].fill, Some(fill.clone()), "{cmd} whole {whole}");
        let other_now = fill_of(&s, other.id);
        if *whole {
            assert_ne!(other_now, other.appearance.fill_paint());
            assert_eq!(kept[1].fill, Some(other_now.clone()));
        } else {
            assert_eq!(other_now, other.appearance.fill_paint(), "the other series is left as it was");
            assert!(kept.get(1).is_none_or(|p| p.fill.is_none()), "{kept:?}");
        }
        s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
        assert!(marks_of(&s, id, 0).iter().all(|m| m.0 == fill), "{:?}", marks_of(&s, id, 0));
        assert!(marks_of(&s, id, 1).iter().all(|m| m.0 == other_now), "{:?}", marks_of(&s, id, 1));
    });
}

/// A recolor that changes several marks of one series to different fills stores the frontmost mark's fill, whatever
/// order the ids are given in.
#[test]
fn recoloring_several_marks_of_a_series_stores_the_frontmost_ones_fill() {
    let cases =
        [("edit.colors.invert", false), ("paint.invert", true)].into_iter().flat_map(|c| [[0, 1, 2], [2, 1, 0], [1, 2, 0]].map(|o| (c.0, c.1, o)));
    every_case(cases, |(cmd, item, order)| {
        let (mut s, id, _, marks) = column_marks();
        for (mark, hex) in [(marks[0], "#ff0000"), (marks[1], "#00ff00"), (marks[2], "#0000ff")] {
            s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, true), "color": hex})).unwrap();
        }
        let mut p = json!({"ids": order.map(|i| marks[i].0)});
        if *item {
            p["item"] = json!(top_row(&s, marks[0], true));
        }
        s.execute(cmd, &p).unwrap();
        assert_eq!(spec(&s, id).series_paints[0].fill, Some(fill_of(&s, marks[2])), "{cmd} {order:?}");
    });
}

/// Blend Horizontally across a series' three marks stores the fill it gives the middle one on the graph.
#[test]
fn blending_a_series_marks_fills_is_kept() {
    let (mut s, id, marks) = stored_red_stroke();
    for (mark, hex) in [(marks[0], "#ff0000"), (marks[2], "#0000ff")] {
        s.execute("appearance.setItem", &json!({"ids": [mark.0], "index": top_row(&s, mark, true), "color": hex})).unwrap();
    }
    let was = spec(&s, id).series_paints;
    s.execute("select.set", &json!({"ids": [marks[0].0, marks[1].0, marks[2].0]})).unwrap();
    s.execute("edit.colors.blendHorizontally", &json!({})).unwrap();
    let mid = s.doc().unwrap().doc.node(marks[1]).unwrap().appearance.fill_paint();
    assert_ne!(Some(&mid), was[0].fill.as_ref());
    assert_eq!(spec(&s, id).series_paints[0].fill, Some(mid.clone()));
    let red = was[0].stroke.clone().unwrap();
    series_0_is(&mut s, id, (&mid, 0.4, &red, 2.0), &was);
}

/// A kept opacity read from a file outside 0..1 is held to that range, and one that is not a number is left out.
#[test]
fn a_kept_series_opacity_from_a_file_is_held_to_its_range() {
    let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
    let paints = vec![vectorcraft_doc::SeriesPaint { fill_opacity: Some(-1.0), stroke_opacity: Some(f32::NAN), ..Default::default() }];
    let paints = [paints, vec![vectorcraft_doc::SeriesPaint { fill_opacity: Some(40.0), ..Default::default() }]].concat();
    let g = GraphSpec { rows: vec![vec![35.0, 20.0]], axis_min: Some(0.0), axis_max: Some(50.0), series_paints: paints, ..GraphSpec::default() };
    let out = super::generate(&mut d, &g);
    for (index, want) in [(0, 0.0), (1, 1.0)] {
        let group = out.iter().find(|n| n.extras().series_index == Some(index)).unwrap();
        let marks: Vec<_> = group.children().unwrap().iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).collect();
        assert!(!marks.is_empty());
        for m in marks {
            assert_eq!(m.appearance.fill().unwrap().opacity, want);
            assert!(m.appearance.stroke().is_none_or(|st| st.opacity == 1.0));
        }
    }
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
    let dir = vectorcraft_testkit::TempDir::new("graph-palette");
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
    let dir = vectorcraft_testkit::TempDir::new("graph-kinds");
    let path = dir.join("kinds.vectorcraft");
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

/// The Graph Type dialog's subject: the selected graph, the series picked in it, and the tick values each value axis
/// is drawn with while they are calculated, which are the ones the graph's value labels show.
#[test]
fn graph_type_subject_gives_the_picked_series_and_the_calculated_tick_values() {
    let mut s = Session::new();
    assert!(super::graph_type_subject(&s).is_err(), "no document");
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    assert!(super::graph_type_subject(&s).is_err(), "no graph selected");
    // Q1 3, 2; Q2 5, 4; Q3 -1, 6.
    let id = graph(&mut s, "column");
    let subject = super::graph_type_subject(&s).unwrap();
    assert_eq!((subject.id, subject.series), (id, vec![]));
    assert_eq!(subject.calculated, [(-2.0, 6.0, 4); 2]);
    let shown = |s: &Session| value_labels(s, id).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
    assert_eq!(shown(&s), ["-2", "0", "2", "4", "6"]);
    // Overriding with them draws the same axis, and the calculated values stay the data's.
    s.execute("graph.setType", &json!({"axisMin": -2, "axisMax": 6, "ticks": 4})).unwrap();
    assert_eq!(shown(&s), ["-2", "0", "2", "4", "6"]);
    s.execute("graph.setType", &json!({"axisMin": 0, "axisMax": 100, "ticks": 0})).unwrap();
    assert_eq!(super::graph_type_subject(&s).unwrap().calculated[0], (-2.0, 6.0, 4));
    // A single bound is calculated with, as the axis is drawn.
    s.execute("graph.setType", &json!({"axisMin": -10, "axisMax": null})).unwrap();
    let (lo, hi, _) = super::graph_type_subject(&s).unwrap().calculated[0];
    let labels = shown(&s);
    assert!(lo <= -10.0, "{lo}");
    assert_eq!((labels.first().and_then(|t| t.parse::<f64>().ok()), labels.last().and_then(|t| t.parse::<f64>().ok())), (Some(lo), Some(hi)));
    s.execute("graph.setType", &json!({"axisMin": null, "axisMax": null})).unwrap();
    // Stacked columns stack their positive and their negative values: Q2 reaches 9, Q3 from -1 to 6.
    s.execute("graph.setType", &json!({"type": "stackedColumn"})).unwrap();
    assert_eq!(super::graph_type_subject(&s).unwrap().calculated[0], (-2.0, 10.0, 6));
    assert_eq!(shown(&s), ["-2", "0", "2", "4", "6", "8", "10"]);
    // A radar graph's axis starts at 0, and a pie graph has none.
    s.execute("graph.setType", &json!({"type": "radar"})).unwrap();
    assert_eq!(super::graph_type_subject(&s).unwrap().calculated, [(0.0, 6.0, 3); 2]);
    s.execute("graph.setType", &json!({"type": "pie"})).unwrap();
    assert_eq!(super::graph_type_subject(&s).unwrap().calculated, [(0.0, 1.0, 5); 2]);
    // Series picked with Group Selection.
    s.execute("graph.setType", &json!({"type": "column"})).unwrap();
    let picked = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
    s.execute("select.set", &json!({"ids": [picked.0]})).unwrap();
    let subject = super::graph_type_subject(&s).unwrap();
    assert_eq!((subject.id, subject.series), (id, vec![1]));
    // Separate Scales: the right axis has the tick values of its own series.
    let mut s = Session::new();
    let id = two_scale_graph(&mut s);
    assert_eq!(super::graph_type_subject(&s).unwrap().calculated, [(0.0, 1000.0, 5); 2]);
    s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
    s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true})).unwrap();
    assert_eq!(super::graph_type_subject(&s).unwrap().calculated, [(0.0, 10.0, 5), (0.0, 1000.0, 5)]);
    assert_eq!(spec(&s, id).right_series, [1]);
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
    let dir = vectorcraft_testkit::TempDir::new("graph-axes");
    let path = dir.join("axes.vectorcraft");
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
    let dir = vectorcraft_testkit::TempDir::new("graph-ticks");
    let path = dir.join("ticks.vectorcraft");
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
    s.execute("graph.setType", &json!({"categoryTickMarks": 0, "rightTickMarks": 1e300})).unwrap();
    let g = spec(&s, id);
    assert_eq!((g.category_tick_marks, g.right_tick_marks, g.right_ticks), (1, 20, 0));
    // A negative count is refused.
    s.execute("graph.setType", &json!({"rightTicks": -2.0})).unwrap_err();
    assert_eq!(spec(&s, id).right_ticks, 0);
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
    // A name listed many times deletes once (its graphs regenerate once).
    s.execute("graph.design", &json!({"delete": vec!["Pill"; 1000]})).unwrap();
    assert!(s.doc().unwrap().doc.graph_designs.is_empty());
    assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
}

/// Graph design "Coin" in the active document: a 10 × 10 rectangle filled with global swatch "Gold".
fn save_coin_design(s: &mut Session) {
    s.execute("swatch.new", &json!({"name": "Gold", "color": "#d4a017", "global": true})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Gold", "ids": [r]})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("graph.design", &json!({"new": "Coin"})).unwrap();
}

/// Load a saved document holding [`save_coin_design`]'s design as a swatch library (Other
/// Library…) → its id.
fn load_coin_library(s: &mut Session) -> String {
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut src);
    let bytes = vectorcraft_format::save_file(&src.doc().unwrap().doc);
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "Coins.vectorcraft"});
    let r = s.execute("swatch.library.load", &p).unwrap();
    assert_eq!(r["graphDesigns"], 1);
    r["library"].as_str().unwrap().to_string()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Whether some object of `n` is filled with a colour linked to swatch `name`.
fn paints_with_swatch(n: &vectorcraft_doc::Node, name: &str) -> bool {
    let mut found = false;
    n.walk(&mut |m| found |= m.appearance.items.iter().any(|it| matches!(it.paint(), Paint::Solid { swatch: Some(w), .. } if w == name)));
    found
}

#[test]
fn designs_of_a_document_loaded_as_a_swatch_library_come_in_when_applied() {
    let mut s = Session::new();
    let lib = load_coin_library(&mut s);
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    // The Column dialog's list: none of the document's, Coin from the library.
    let q = s.execute("graph.column", &json!({})).unwrap();
    assert_eq!((q["designs"].clone(), q["libraries"][0]["library"].clone()), (json!([]), json!(lib)));
    assert_eq!(q["libraries"][0]["designs"], json!(["Coin"]));
    assert!(s.execute("graph.column", &json!({"design": "Coin"})).is_err(), "not the document's");
    let steps = undo_len(&s);
    s.execute("graph.column", &json!({"design": "Coin", "library": lib})).unwrap();
    assert_eq!(undo_len(&s), steps + 1, "one undo step");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.graph_designs.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Coin"]);
    assert!(d.swatch("Gold").is_some_and(|w| w.global), "the swatch the design's art uses comes along");
    assert!(paints_with_swatch(&d.graph_designs[0].art, "Gold"));
    assert!(paints_with_swatch(series(d.node(id).unwrap(), 0), "Gold"), "the columns draw the design");
    // Undo takes the design, its swatch and the columns' design out together.
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.graph_designs.is_empty() && d.swatch("Gold").is_none() && spec(&s, id).series_columns.is_empty());
    // Marker takes them too.
    let line = graph(&mut s, "line");
    s.execute("select.set", &json!({"ids": [line.0]})).unwrap();
    s.execute("graph.marker", &json!({"design": "Coin", "library": lib})).unwrap();
    assert_eq!(spec(&s, line).series_markers, [Some("Coin".to_string()), Some("Coin".to_string())]);
    assert!(s.doc().unwrap().doc.swatch("Gold").is_some());
    // An unknown library or design, and a library that is not a string, are refused.
    for p in [json!({"design": "Coin", "library": "Nope"}), json!({"design": "Nope", "library": lib}), json!({"design": "Coin", "library": 3})] {
        assert!(s.execute("graph.column", &p).is_err(), "{p}");
    }
}

#[test]
fn a_library_design_applied_to_a_moved_graph_that_names_it_keeps_the_graph_in_place() {
    let mut s = Session::new();
    let lib = load_coin_library(&mut s);
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // A library's name works in any case, as swatch library names do.
    assert_eq!(s.execute("graph.design", &json!({"library": "COINS"})).unwrap(), json!({"designs": ["Coin"]}));
    for (kind, cmd) in [("line", "graph.marker"), ("column", "graph.column")] {
        // The graph names design "Coin", which the document no longer has, and is moved after.
        save_design(&mut s, "Coin");
        let id = graph(&mut s, kind);
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute(cmd, &json!({"design": "Coin"})).unwrap();
        s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
        let rect = spec(&s, id).rect;
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("object.move", &json!({"dx": 40, "dy": -10})).unwrap();
        s.execute(cmd, &json!({"id": id.0, "design": "Coin", "library": lib})).unwrap();
        let now = spec(&s, id).rect;
        assert!((now.x0 - (rect.x0 + 40.0)).abs() < 1e-6 && (now.y0 - (rect.y0 - 10.0)).abs() < 1e-6, "{cmd}: {now:?}");
        assert!((now.width() - rect.width()).abs() < 1e-6 && (now.height() - rect.height()).abs() < 1e-6, "{cmd}: {now:?}");
        s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    }
}

#[test]
fn a_design_the_document_has_under_the_same_name_stays_as_it_is() {
    let mut s = Session::new();
    let lib = load_coin_library(&mut s);
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    let own = s.doc().unwrap().doc.graph_designs[0].art.clone();
    assert_eq!(s.execute("graph.design", &json!({"library": lib})).unwrap(), json!({"designs": ["Coin"]}));
    let steps = undo_len(&s);
    let r = s.execute("graph.design", &json!({"library": lib, "add": ["Coin", "Coin"]})).unwrap();
    assert_eq!(r, json!({"added": [], "existing": ["Coin"]}));
    assert_eq!(undo_len(&s), steps, "nothing changed: no undo step");
    // Applying it with the library draws with the document's own design.
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin", "library": lib})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.graph_designs.len() == 1 && std::sync::Arc::ptr_eq(&d.graph_designs[0].art, &own));
    assert!(d.swatch("Gold").is_none());
    assert!(s.execute("graph.design", &json!({"add": "Coin"})).is_err(), "no library");
    assert!(s.execute("graph.design", &json!({"library": lib, "add": "Nope"})).is_err());
    assert!(s.execute("graph.design", &json!({"library": lib, "add": 4})).is_err());
    // The document without it takes it as one undo step.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let r = s.execute("graph.design", &json!({"library": "Coins", "add": "Coin"})).unwrap();
    assert_eq!(r, json!({"added": ["Coin"], "existing": []}));
    assert!(s.doc().unwrap().doc.swatch("Gold").is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.graph_designs.is_empty());
}

#[test]
fn a_pasted_graph_brings_the_designs_its_series_draw_with() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.graph_designs.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Coin"]);
    assert!(paints_with_swatch(&d.graph_designs[0].art, "Gold") && d.swatch("Gold").is_some());
    // A data edit redraws the pasted graph with the design.
    let pasted = s.doc().unwrap().selection.objects.to_vec()[0];
    s.execute("graph.setData", &json!({"id": pasted.0, "csv": ",A\nQ1,4\nQ2,6"})).unwrap();
    assert!(paints_with_swatch(series(s.doc().unwrap().doc.node(pasted).unwrap(), 0), "Gold"));
    // A document with its own design of that name keeps it.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    let own = s.doc().unwrap().doc.graph_designs[0].art.clone();
    s.execute("edit.paste", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.graph_designs.len() == 1 && std::sync::Arc::ptr_eq(&d.graph_designs[0].art, &own));
    // The pasted graph is drawn with that design straight away.
    let pasted = s.doc().unwrap().selection.objects.to_vec()[0];
    // The document's design is a rectangle and an ellipse, the pasted one a rectangle alone: each
    // column and the legend swatch hold two paths.
    let paths = |n: &vectorcraft_doc::Node| {
        let mut count = 0;
        n.walk(&mut |m| count += usize::from(matches!(m.kind, NodeKind::Path { .. })));
        count
    };
    let columns: Vec<usize> = series(d.node(pasted).unwrap(), 0).children().unwrap().iter().map(|n| paths(n)).collect();
    assert_eq!(columns, [2, 2, 2, 2], "drawn with the document's design");
}

#[test]
fn a_design_the_document_keeps_brings_none_of_the_pasted_designs_resources() {
    // A column graph whose spec names marker design "Coin", which only line series draw: the
    // graph's own art does not use the design's swatch.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.marker", &json!({"design": "Coin"})).unwrap();
    assert!(spec(&s, id).series_markers.iter().any(|m| m.as_deref() == Some("Coin")));
    assert!(!paints_with_swatch(s.doc().unwrap().doc.node(id).unwrap(), "Gold"));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    // A document with its own "Coin" keeps it and takes nothing the pasted design's art uses.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    s.execute("edit.paste", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.swatch("Gold").is_none());
    // A document without one takes the design and its swatch.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.swatch("Gold").is_some());
}

#[test]
fn a_pasted_graph_naming_a_deleted_design_draws_with_the_documents_design_of_that_name() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    assert!(spec(&s, id).series_columns.iter().flatten().any(|c| c.design == "Coin"), "the graph still names it");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    s.execute("edit.paste", &json!({})).unwrap();
    let pasted = s.doc().unwrap().selection.objects.to_vec()[0];
    let paths = |n: &vectorcraft_doc::Node| {
        let mut count = 0;
        n.walk(&mut |m| count += usize::from(matches!(m.kind, NodeKind::Path { .. })));
        count
    };
    let d = &s.doc().unwrap().doc;
    let columns: Vec<usize> = series(d.node(pasted).unwrap(), 0).children().unwrap().iter().map(|n| paths(n)).collect();
    assert_eq!(columns, [2, 2, 2, 2], "drawn with the document's design");
}

#[test]
fn a_pasted_graph_keeps_its_art_when_only_a_resource_its_design_does_not_use_comes_in_renamed() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#ff0000", "global": true})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Ink", "ids": [r]})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    // An edit of the graph's art that drawing the graph again would not keep.
    let mark = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.edit("Name", |d, _| {
        if let Some(n) = d.node_mut(mark) {
            n.name = Some("Mine".into());
        }
        Ok(())
    })
    .unwrap();
    // A copy of the document holds the same design; its "Ink" is another color, so the pasted
    // rectangle's swatch takes the document's color, or with "add" comes in renamed.
    let saved = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&s.doc().unwrap().doc));
    s.execute("select.set", &json!({"ids": [id.0, r]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("document.open", &json!({"name": "b.vectorcraft", "dataBase64": saved})).unwrap();
    s.execute("swatch.edit", &json!({"name": "Ink", "color": "#0000ff"})).unwrap();
    for conflict in ["merge", "add"] {
        let r = s.execute("edit.paste", &json!({"swatchConflict": conflict})).unwrap();
        let ink_renamed = r["renamed"].as_array().is_some_and(|a| a.iter().any(|x| x["from"] == "Ink"));
        let expected = if conflict == "merge" { (json!(1), false) } else { (json!(0), true) };
        assert_eq!((r["merged"].clone(), ink_renamed), expected, "{conflict}: {r}");
        let st = s.doc().unwrap();
        let d = &st.doc;
        let pasted = st.selection.objects.iter().copied().find(|i| d.node(*i).is_some_and(|n| n.extras().graph.is_some())).unwrap();
        let names: Vec<Option<String>> = series(d.node(pasted).unwrap(), 0).children().unwrap().iter().map(|n| n.name.clone()).collect();
        assert_eq!(names[0].as_deref(), Some("Mine"), "{conflict}: not drawn again: {names:?}");
    }
}

#[test]
fn a_pasted_graph_is_drawn_again_when_its_designs_svg_filter_comes_in_under_another_name() {
    const SHIFT: &str = r#"<filter id="Shift"><feOffset dx="3" dy="3"/></filter>"#;
    const SHIFT_MORE: &str = r#"<filter id="Shift"><feOffset dx="9" dy="9"/></filter>"#;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    s.execute("svgFilter.define", &json!({"xml": SHIFT})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("svgFilter.apply", &json!({"name": "Shift", "ids": [r]})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("graph.design", &json!({"new": "Coin"})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    let mark = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.edit("Name", |d, _| {
        if let Some(n) = d.node_mut(mark) {
            n.name = Some("Mine".into());
        }
        Ok(())
    })
    .unwrap();
    // A copy of the document holds the same design, but its "Shift" filter draws otherwise, so the
    // pasted graph's filter keeps its own definition under another name.
    let saved = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&s.doc().unwrap().doc));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("document.open", &json!({"name": "b.vectorcraft", "dataBase64": saved})).unwrap();
    s.execute("svgFilter.define", &json!({"xml": SHIFT_MORE, "replace": "Shift"})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let st = s.doc().unwrap();
    let pasted = st.selection.objects.to_vec()[0];
    let names: Vec<Option<String>> = series(st.doc.node(pasted).unwrap(), 0).children().unwrap().iter().map(|n| n.name.clone()).collect();
    assert_ne!(names[0].as_deref(), Some("Mine"), "drawn again with the document's design: {names:?}");
}

#[test]
fn a_symbol_pasted_with_a_graph_inside_draws_it_with_the_documents_design_of_that_name() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let instance = s.execute("symbol.new", &json!({"name": "Chart"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [instance]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    // A document whose "Coin" is a rectangle and an ellipse: the symbol's graph draws with it.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    s.execute("edit.paste", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let art = &d.symbols.iter().find(|x| x.name == "Chart").expect("the symbol came in").art;
    let mut graphs = vec![];
    art.walk(&mut |m| {
        if m.extras().graph.is_some() {
            graphs.push(m.clone());
        }
    });
    let paths = |n: &vectorcraft_doc::Node| {
        let mut count = 0;
        n.walk(&mut |m| count += usize::from(matches!(m.kind, NodeKind::Path { .. })));
        count
    };
    let columns: Vec<usize> = series(&graphs[0], 0).children().unwrap().iter().map(|n| paths(n)).collect();
    assert_eq!(columns, [2, 2, 2, 2], "drawn with the document's design");
}

#[test]
fn a_design_brought_in_from_a_library_redraws_the_graphs_in_symbols_that_name_it() {
    let mut s = Session::new();
    let lib = load_coin_library(&mut s);
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // A graph that names "Coin", which the document no longer has, kept as a symbol's art.
    save_design(&mut s, "Coin");
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    s.execute("symbol.new", &json!({"name": "Chart", "ids": [id.0]})).unwrap();
    s.execute("graph.design", &json!({"library": lib, "add": "Coin"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let art = &d.symbols.iter().find(|x| x.name == "Chart").unwrap().art;
    let mut graphs = vec![];
    art.walk(&mut |m| {
        if m.extras().graph.is_some() {
            graphs.push(m.clone());
        }
    });
    assert!(paints_with_swatch(series(&graphs[0], 0), "Gold"), "drawn with the library's design");
}

#[test]
fn a_design_brought_in_from_a_library_leaves_a_scatter_graph_that_names_it_for_a_slot_it_does_not_plot() {
    let mut s = Session::new();
    let lib = load_coin_library(&mut s);
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // Four data columns plot two series, so marker slot 2 is never drawn.
    let id = make(&mut s, "scatter", ",y1,x1,y2,x2\nP,1,10,2,20\nQ,3,11,4,22");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    assert!(spec(&s, id).mark_points, "Mark Data Points is on");
    s.execute("graph.marker", &json!({"design": "Coin", "library": lib, "seriesIndexes": [2]})).unwrap();
    s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    assert_eq!(spec(&s, id).series_markers.get(2).cloned().flatten().as_deref(), Some("Coin"));
    // An edit of the graph's art that drawing the graph again would not keep.
    let mark = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.edit("Name", |d, _| {
        if let Some(n) = d.node_mut(mark) {
            n.name = Some("Mine".into());
        }
        Ok(())
    })
    .unwrap();
    s.execute("graph.design", &json!({"library": lib, "add": "Coin"})).unwrap();
    let names: Vec<Option<String>> = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap().iter().map(|n| n.name.clone()).collect();
    assert_eq!(names[0].as_deref(), Some("Mine"), "not drawn again: {names:?}");
}

#[test]
fn a_symbol_a_library_design_brings_draws_its_graph_with_the_documents_design_it_keeps() {
    // The library's design "A" holds an instance of symbol "Chart", whose graph draws columns with
    // the library's "B": a rectangle and an ellipse.
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut src, "B");
    let id = graph(&mut src, "column");
    src.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    src.execute("graph.column", &json!({"design": "B"})).unwrap();
    let instance = src.execute("symbol.new", &json!({"name": "Chart", "ids": [id.0]})).unwrap()["id"].as_u64().unwrap();
    src.execute("select.set", &json!({"ids": [instance]})).unwrap();
    src.execute("graph.design", &json!({"new": "A"})).unwrap();
    let bytes = vectorcraft_format::save_file(&src.doc().unwrap().doc);
    let mut s = Session::new();
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "Lib.vectorcraft"});
    let lib = s.execute("swatch.library.load", &p).unwrap()["library"].as_str().unwrap().to_string();
    // The document's own "B" is a single rectangle, and stays.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("graph.design", &json!({"new": "B"})).unwrap();
    s.execute("graph.design", &json!({"library": lib, "add": "A"})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.graph_designs.len(), 2, "A came in and B stayed");
    let art = &d.symbols.iter().find(|x| x.name == "Chart").expect("the symbol came in").art;
    let mut graphs = vec![];
    art.walk(&mut |m| {
        if m.extras().graph.is_some() {
            graphs.push(m.clone());
        }
    });
    let paths = |n: &vectorcraft_doc::Node| {
        let mut count = 0;
        n.walk(&mut |m| count += usize::from(matches!(m.kind, NodeKind::Path { .. })));
        count
    };
    let columns: Vec<usize> = series(&graphs[0], 0).children().unwrap().iter().map(|n| paths(n)).collect();
    assert_eq!(columns, [1, 1, 1, 1], "drawn with the document's B");
}

#[test]
fn a_design_brought_in_from_a_library_redraws_a_graph_in_an_opacity_mask() {
    let mut s = Session::new();
    let lib = load_coin_library(&mut s);
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap()["id"].as_u64().unwrap();
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    s.execute("select.set", &json!({"ids": [r, id.0]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    s.execute("graph.design", &json!({"library": lib, "add": "Coin"})).unwrap();
    let mut graphs = vec![];
    vectorcraft_doc::swatches::walk_with_masks(s.doc().unwrap().doc.node(NodeId(r)).unwrap(), &mut |m| {
        if m.extras().graph.is_some() {
            graphs.push(m.clone());
        }
    });
    assert!(paints_with_swatch(series(&graphs[0], 0), "Gold"), "drawn with the library's design");
}

#[test]
fn an_instance_drops_its_changes_to_graph_parts_a_design_change_redraws() {
    use vectorcraft_doc::symbol_overrides::SymbolOverride;
    let mut s = Session::new();
    let lib = load_coin_library(&mut s);
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    let instance = s.execute("symbol.new", &json!({"name": "Chart", "ids": [id.0], "type": "dynamic"})).unwrap()["id"].as_u64().unwrap();
    let instance = NodeId(instance);
    // A change to the graph itself, and one to a part of it that drawing it again replaces.
    let changes = vec![
        SymbolOverride { path: vec![], opacity: Some(0.5), ..Default::default() },
        SymbolOverride { path: vec![0], opacity: Some(0.25), ..Default::default() },
    ];
    s.edit("Change", |d, _| {
        if let Some(n) = d.node_mut(instance) {
            n.extras_mut().symbol_overrides = changes;
        }
        Ok(())
    })
    .unwrap();
    s.execute("graph.design", &json!({"library": lib, "add": "Coin"})).unwrap();
    let kept = s.doc().unwrap().doc.node(instance).unwrap().extras().symbol_overrides.clone();
    assert_eq!(kept.iter().map(|o| o.path.clone()).collect::<Vec<_>>(), [Vec::<u32>::new()], "{kept:?}");
}

#[test]
fn a_graph_in_an_opacity_mask_draws_with_the_documents_design_when_pasted() {
    let paths = |n: &vectorcraft_doc::Node| {
        let mut count = 0;
        n.walk(&mut |m| count += usize::from(matches!(m.kind, NodeKind::Path { .. })));
        count
    };
    let mask_graph = |n: &vectorcraft_doc::Node| {
        let mut graphs = vec![];
        vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| {
            if m.extras().graph.is_some() {
                graphs.push(m.clone());
            }
        });
        graphs.into_iter().next().expect("the masked graph")
    };
    // In a symbol's art and among the pasted objects.
    for in_symbol in [true, false] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        save_coin_design(&mut s);
        let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap()["id"].as_u64().unwrap();
        let id = graph(&mut s, "column");
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
        s.execute("select.set", &json!({"ids": [r, id.0]})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        let copied = if in_symbol { s.execute("symbol.new", &json!({"name": "Masked", "ids": [r]})).unwrap()["id"].as_u64().unwrap() } else { r };
        s.execute("select.set", &json!({"ids": [copied]})).unwrap();
        s.execute("edit.copy", &json!({})).unwrap();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        save_design(&mut s, "Coin");
        s.execute("edit.paste", &json!({})).unwrap();
        let st = s.doc().unwrap();
        let g = if in_symbol {
            mask_graph(&st.doc.symbols.iter().find(|x| x.name == "Masked").expect("the symbol came in").art)
        } else {
            mask_graph(st.doc.node(st.selection.objects.to_vec()[0]).unwrap())
        };
        let columns: Vec<usize> = series(&g, 0).children().unwrap().iter().map(|n| paths(n)).collect();
        assert_eq!(columns, [2, 2, 2, 2], "in a symbol: {in_symbol}");
    }
}

#[test]
fn a_graph_pasted_back_after_its_design_was_made_again_draws_with_the_new_design() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    // Made again under the same name as a rectangle and an ellipse.
    s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    save_design(&mut s, "Coin");
    s.execute("edit.paste", &json!({})).unwrap();
    let pasted = s.doc().unwrap().selection.objects.to_vec()[0];
    let paths = |n: &vectorcraft_doc::Node| {
        let mut count = 0;
        n.walk(&mut |m| count += usize::from(matches!(m.kind, NodeKind::Path { .. })));
        count
    };
    let d = &s.doc().unwrap().doc;
    let columns: Vec<usize> = series(d.node(pasted).unwrap(), 0).children().unwrap().iter().map(|n| paths(n)).collect();
    assert_eq!(columns, [2, 2, 2, 2], "drawn with the design the document has now");
}

/// The graphs in `n`'s subtree, those in opacity-mask art included.
fn graphs_in(n: &vectorcraft_doc::Node) -> Vec<vectorcraft_doc::Node> {
    let mut graphs = vec![];
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| {
        if m.extras().graph.is_some() {
            graphs.push(m.clone());
        }
    });
    graphs
}

/// Load a saved document holding [`save_coin_design`]'s "Coin" and design "Chart", an instance of
/// symbol "Plot" whose column graph names "Coin" as its columns (`columns`) or only as a marker
/// its columns do not draw, as a swatch library → its id.
fn load_chart_library(s: &mut Session, columns: bool) -> String {
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut src);
    let id = graph(&mut src, "column");
    src.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    src.execute(if columns { "graph.column" } else { "graph.marker" }, &json!({"design": "Coin"})).unwrap();
    let instance = src.execute("symbol.new", &json!({"name": "Plot", "ids": [id.0]})).unwrap()["id"].as_u64().unwrap();
    src.execute("select.set", &json!({"ids": [instance]})).unwrap();
    src.execute("graph.design", &json!({"new": "Chart"})).unwrap();
    let bytes = vectorcraft_format::save_file(&src.doc().unwrap().doc);
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "Charts.vectorcraft"});
    s.execute("swatch.library.load", &p).unwrap()["library"].as_str().unwrap().to_string()
}

#[test]
fn a_library_design_brings_none_of_the_resources_of_a_design_the_document_keeps() {
    let mut s = Session::new();
    let lib = load_chart_library(&mut s, false);
    // The document keeps its own "Coin": nothing the library's "Coin" uses comes in.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    s.execute("graph.design", &json!({"library": lib, "add": "Chart"})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.symbols.iter().any(|x| x.name == "Plot"), "the design's symbol came in");
    assert!(d.swatch("Gold").is_none());
    // A document without one takes the design and its swatch.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    s.execute("graph.design", &json!({"library": lib, "add": "Chart"})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.graph_designs.iter().any(|x| x.name == "Coin") && d.swatch("Gold").is_some());
}

#[test]
fn a_design_a_library_design_brings_along_redraws_the_graphs_that_name_it() {
    let mut s = Session::new();
    let lib = load_chart_library(&mut s, true);
    // A graph that names "Coin", which the document no longer has.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    let r = s.execute("graph.design", &json!({"library": lib, "add": "Chart"})).unwrap();
    assert_eq!(r, json!({"added": ["Chart"], "existing": []}));
    let d = &s.doc().unwrap().doc;
    assert!(paints_with_swatch(series(d.node(id).unwrap(), 0), "Gold"), "drawn with the library's Coin");
}

#[test]
fn a_paste_that_adds_a_design_redraws_the_documents_graphs_that_name_it_and_not_the_pasted_ones() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    // An edit of the copied graph's art that drawing it again would not keep.
    let mark = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.edit("Name", |d, _| {
        if let Some(n) = d.node_mut(mark) {
            n.name = Some("Mine".into());
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    // A document whose graph names "Coin", which it no longer has.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    let own = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [own.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert!(paints_with_swatch(series(st.doc.node(own).unwrap(), 0), "Gold"), "drawn with the pasted Coin");
    let pasted = st.selection.objects.to_vec()[0];
    let first = series(st.doc.node(pasted).unwrap(), 0).children().unwrap()[0].name.clone();
    assert_eq!(first.as_deref(), Some("Mine"), "the pasted graph is not drawn again");
}

#[test]
fn a_paste_that_adds_a_marker_design_a_graph_does_not_show_leaves_that_graph_as_it_is() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    // A line graph whose series name "Coin", which the document no longer has, as a marker that
    // Mark Data Points does not show, with no legend or with a legend that has no entry for them.
    for (legend, csv) in [(false, ",2024,2025\nQ1,3,2\nQ2,5,4"), (true, ",,\nQ1,3,2\nQ2,5,4")] {
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        save_design(&mut s, "Coin");
        let p = json!({"type": "line", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv});
        let own = NodeId(s.execute("graph.create", &p).unwrap()["id"].as_u64().unwrap());
        s.execute("select.set", &json!({"ids": [own.0]})).unwrap();
        s.execute("graph.marker", &json!({"design": "Coin"})).unwrap();
        s.execute("graph.setType", &json!({"id": own.0, "markPoints": false, "legend": legend})).unwrap();
        s.execute("graph.design", &json!({"delete": "Coin"})).unwrap();
        let mark = series(s.doc().unwrap().doc.node(own).unwrap(), 0).children().unwrap()[0].id;
        s.edit("Name", |d, _| {
            if let Some(n) = d.node_mut(mark) {
                n.name = Some("Mine".into());
            }
            Ok(())
        })
        .unwrap();
        s.execute("edit.paste", &json!({})).unwrap();
        let d = &s.doc().unwrap().doc;
        assert!(d.graph_designs.iter().any(|x| x.name == "Coin"), "legend {legend}: the paste added Coin");
        let first = series(d.node(own).unwrap(), 0).children().unwrap()[0].name.clone();
        assert_eq!(first.as_deref(), Some("Mine"), "legend {legend}: not drawn again");
    }
}

#[test]
fn a_symbol_pasted_twice_with_a_graph_the_document_draws_again_is_added_once() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    let instance = s.execute("symbol.new", &json!({"name": "Chart", "ids": [id.0]})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [instance]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    // A document whose "Coin" is other art: the first paste draws the symbol's graph with it.
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    // The paths of each column of the symbol's graph: 2 for the document's Coin, 1 for the copied one.
    let columns = |s: &Session| {
        let art = s.doc().unwrap().doc.symbols.iter().find(|x| x.name == "Chart").unwrap().art.clone();
        let graph = graphs_in(&art).remove(0);
        let paths = |n: &vectorcraft_doc::Node| {
            let mut count = 0;
            n.walk(&mut |m| count += usize::from(matches!(m.kind, NodeKind::Path { .. })));
            count
        };
        series(&graph, 0).children().unwrap().iter().map(|n| paths(n)).collect::<Vec<usize>>()
    };
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(columns(&s), [2, 2, 2, 2], "the first paste draws with the document's Coin");
    s.execute("edit.paste", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let names: Vec<&str> = d.symbols.iter().map(|x| x.name.as_str()).filter(|n| n.starts_with("Chart")).collect();
    assert_eq!(names, ["Chart"]);
    assert_eq!(columns(&s), [2, 2, 2, 2], "drawn with the document's Coin");
}

#[test]
fn a_symbol_pasted_twice_whose_designs_swatch_takes_the_documents_color_is_added_once() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    // A copy of the document holds the same "Coin"; its "Gold" is another color.
    let saved = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&s.doc().unwrap().doc));
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    let instance = s.execute("symbol.new", &json!({"name": "Chart", "ids": [id.0]})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [instance]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    for conflict in ["merge", "add"] {
        s.execute("document.open", &json!({"name": format!("{conflict}.vectorcraft"), "dataBase64": saved})).unwrap();
        s.execute("swatch.edit", &json!({"name": "Gold", "color": "#0000ff"})).unwrap();
        let r = s.execute("edit.paste", &json!({"swatchConflict": conflict})).unwrap();
        let gold_renamed = r["renamed"].as_array().is_some_and(|a| a.iter().any(|x| x["from"] == "Gold"));
        let expected = if conflict == "merge" { (json!(1), false) } else { (json!(0), true) };
        assert_eq!((r["merged"].clone(), gold_renamed), expected, "{conflict}: {r}");
        s.execute("edit.paste", &json!({"swatchConflict": conflict})).unwrap();
        let d = &s.doc().unwrap().doc;
        let names: Vec<&str> = d.symbols.iter().map(|x| x.name.as_str()).filter(|n| n.starts_with("Chart")).collect();
        assert_eq!(names, ["Chart"], "{conflict}");
    }
}

#[test]
fn a_pasted_graph_that_only_names_a_kept_design_as_an_undrawn_marker_keeps_its_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    // A column graph whose series name "Coin" as a marker, which columns do not draw.
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.marker", &json!({"design": "Coin"})).unwrap();
    let mark = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
    s.edit("Name", |d, _| {
        if let Some(n) = d.node_mut(mark) {
            n.name = Some("Mine".into());
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    s.execute("edit.paste", &json!({})).unwrap();
    let st = s.doc().unwrap();
    let pasted = st.selection.objects.to_vec()[0];
    let first = series(st.doc.node(pasted).unwrap(), 0).children().unwrap()[0].name.clone();
    assert_eq!(first.as_deref(), Some("Mine"), "not drawn again");
}

#[test]
fn renaming_a_design_renames_it_in_the_graphs_of_symbols_and_opacity_masks() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_coin_design(&mut s);
    let in_symbol = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [in_symbol.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("symbol.new", &json!({"name": "Chart", "ids": [in_symbol.0]})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap()["id"].as_u64().unwrap();
    let masked = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [masked.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("select.set", &json!({"ids": [r, masked.0]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    let before = graphs_in(&s.doc().unwrap().doc.symbols.iter().find(|x| x.name == "Chart").unwrap().art);
    s.execute("graph.design", &json!({"rename": "Coin", "to": "Medal"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let symbol = graphs_in(&d.symbols.iter().find(|x| x.name == "Chart").unwrap().art);
    let mask = graphs_in(d.node(NodeId(r)).unwrap());
    for (where_, g) in [("symbol", &symbol[0]), ("mask", &mask[0])] {
        let designs: Vec<String> = g.extras().graph.as_deref().unwrap().series_columns.iter().flatten().map(|c| c.design.clone()).collect();
        assert!(!designs.is_empty() && designs.iter().all(|n| n == "Medal"), "{where_}: {designs:?}");
    }
    assert_eq!(symbol[0].children(), before[0].children(), "the symbol's graph keeps its art");
}

#[test]
fn a_design_only_a_graph_in_a_symbol_or_a_mask_draws_with_is_not_unused() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Coin");
    save_design(&mut s, "Pill");
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Coin"})).unwrap();
    s.execute("symbol.new", &json!({"name": "Chart", "ids": [id.0]})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap()["id"].as_u64().unwrap();
    let masked = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [masked.0]})).unwrap();
    s.execute("graph.column", &json!({"design": "Pill"})).unwrap();
    s.execute("select.set", &json!({"ids": [r, masked.0]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    assert_eq!(s.execute("graph.design", &json!({})).unwrap()["unused"], json!([]));
}

#[test]
fn a_library_copied_to_user_defined_keeps_its_graph_designs_under_the_loaded_id() {
    let dir = vectorcraft_testkit::TempDir::new("graph-design-library-user");
    let mut s = Session::new();
    s.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let loaded = load_coin_library(&mut s);
    // The User Defined copy is a swatch file: it holds the swatches alone.
    let copy = s.execute("swatch.library.copyToUser", &json!({"library": loaded})).unwrap()["library"].as_str().unwrap().to_string();
    assert_ne!(copy, loaded);
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let libraries = s.execute("graph.design", &json!({})).unwrap()["libraries"].clone();
    assert_eq!(libraries, json!([{"library": loaded, "name": libraries[0]["name"], "designs": ["Coin"]}]));
    assert_eq!(s.execute("graph.design", &json!({"library": loaded})).unwrap(), json!({"designs": ["Coin"]}));
    assert!(s.execute("graph.design", &json!({"library": copy})).is_err());
}

#[test]
fn a_swatch_library_file_over_the_size_limit_is_refused_before_it_is_read() {
    let dir = vectorcraft_testkit::TempDir::new("graph-design-library-size");
    let path = dir.join("big.vectorcraft");
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(((256 << 20) + 1) as u64).unwrap();
    let mut s = Session::new();
    let e = s.execute("swatch.library.load", &json!({"path": path})).unwrap_err().to_string();
    assert!(e.contains("larger than 256 MB"), "{e}");
}

#[test]
fn a_document_with_graph_designs_and_no_swatches_loads_for_its_designs() {
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut src, "Pill");
    let mut d = (*src.doc().unwrap().doc).clone();
    d.swatches.clear();
    d.swatch_groups.clear();
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d)), "name": "Pills.vectorcraft"});
    let mut s = Session::new();
    let r = s.execute("swatch.library.load", &p).unwrap();
    assert_eq!((r["count"].clone(), r["graphDesigns"].clone()), (json!(0), json!(1)));
    // Without graph designs it has nothing to offer.
    d.graph_designs.clear();
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d)), "name": "Bare.vectorcraft"});
    let e = s.execute("swatch.library.load", &p).unwrap_err().to_string();
    assert!(e.contains("has no swatches of its own"), "{e}");
}

#[test]
fn a_design_library_keeps_the_sources_color_for_a_default_swatch_name() {
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    // A default swatch the source made global and gave another color, used by a design.
    let name = src.doc().unwrap().doc.swatches.iter().find(|w| !w.name.starts_with('[')).unwrap().name.clone();
    let mine = Paint::solid(vectorcraft_color::Color::rgb(0.1, 0.7, 0.3));
    let (n, p) = (name.clone(), mine.clone());
    src.edit("Change", move |d, _| {
        if let Some(w) = d.swatches.iter_mut().find(|w| w.name == n) {
            w.paint = p;
            w.global = true;
        }
        Ok(())
    })
    .unwrap();
    let r = src.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    src.execute("paint.setFill", &json!({"swatch": name, "ids": [r]})).unwrap();
    src.execute("select.set", &json!({"ids": [r]})).unwrap();
    src.execute("graph.design", &json!({"new": "Green"})).unwrap();
    let lib = crate::cmd::clipboard::resources::graph_design_library(&src.doc().unwrap().doc).unwrap();
    let named: Vec<_> = lib.swatches.iter().filter(|w| w.name == name).collect();
    assert_eq!(named.len(), 1, "one swatch named {name}");
    assert_eq!(named[0].paint, mine);
    // And one named like a swatch in a default group.
    let grouped = src.doc().unwrap().doc.swatch_groups.iter().flat_map(|g| g.swatches.iter()).next().unwrap().name.clone();
    let (n, p) = (grouped.clone(), mine.clone());
    src.edit("Change", move |d, _| {
        if let Some(w) = d.swatch_groups.iter_mut().flat_map(|g| g.swatches.iter_mut()).find(|w| w.name == n) {
            w.paint = p;
            w.global = true;
        }
        Ok(())
    })
    .unwrap();
    let r = src.execute("shape.rectangle", &json!({"x": 650, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    src.execute("paint.setFill", &json!({"swatch": grouped, "ids": [r]})).unwrap();
    src.execute("select.set", &json!({"ids": [r]})).unwrap();
    src.execute("graph.design", &json!({"new": "Grouped"})).unwrap();
    let lib = crate::cmd::clipboard::resources::graph_design_library(&src.doc().unwrap().doc).unwrap();
    let all: Vec<_> = lib.swatches.iter().chain(lib.swatch_groups.iter().flat_map(|g| g.swatches.iter())).filter(|w| w.name == grouped).collect();
    assert_eq!(all.len(), 1, "one swatch named {grouped}");
}

#[test]
fn inline_swatch_library_data_over_the_size_limit_is_refused_before_it_is_read() {
    // With a limit of 3 bytes: text and base64 of 3 bytes pass, and of 4 bytes are refused.
    let too_big = |p: serde_json::Value| crate::cmd::swatchlib::library_too_big(&p, "swatch.library.load", 3).unwrap();
    assert!(!too_big(json!({"data": "abc"})));
    assert!(too_big(json!({"data": "abcd"})));
    let b64 = |n: usize| vectorcraft_format::base64_encode(&vec![7u8; n]);
    assert!(!too_big(json!({"dataBase64": b64(3)})));
    assert!(too_big(json!({"dataBase64": b64(4)})));
}

#[test]
fn a_swatch_library_file_is_read_no_further_than_the_size_limit() {
    let dir = vectorcraft_testkit::TempDir::new("graph-design-library-bound");
    let path = dir.join("grown.gpl");
    std::fs::write(&path, [b'a'; 9]).unwrap();
    let mut s = Session::new();
    let p = json!({"path": path});
    let parsed = |_: &[u8], _: &str| Err(crate::EngineError::Other("parsed".into()));
    // A file that grew past the limit after the size check is refused when it is read.
    let e = s.swatch_libraries.load_within(&p, "swatch.library.load", Some(8), parsed).unwrap_err().to_string();
    assert!(e.contains("larger than"), "{e}");
    let e = s.swatch_libraries.load_within(&p, "swatch.library.load", Some(9), parsed).unwrap_err().to_string();
    assert!(e.contains("parsed"), "{e}");
}

#[test]
fn reading_the_head_of_a_swatch_library_file_stops_at_the_limit() {
    let dir = vectorcraft_testkit::TempDir::new("graph-design-library-head");
    let path = dir.join("long.gpl");
    std::fs::write(&path, vec![b'a'; 1 << 20]).unwrap();
    // The read the limit check relies on returns the bytes up to one past the limit, not the file.
    let head = crate::cmd::fileio::read_file_head(&path.to_string_lossy(), 9).unwrap();
    assert_eq!(head.len(), 9);
}

#[test]
fn a_library_design_brings_its_symbols_with_their_symbol_options() {
    // The source design is an instance of symbol "Corner" with 9-slice scaling, a top-left
    // registration point and the Graphic export type.
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap();
    src.execute("select.all", &json!({})).unwrap();
    let opts = json!({"name": "Corner", "nineSlice": true, "registration": 0, "exportType": "graphic"});
    let inst = src.execute("symbol.new", &opts).unwrap()["id"].as_u64().unwrap();
    src.execute("select.set", &json!({"ids": [inst]})).unwrap();
    src.execute("graph.design", &json!({"new": "Corner Mark"})).unwrap();
    let bytes = vectorcraft_format::save_file(&src.doc().unwrap().doc);
    let mut s = Session::new();
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "Corners.vectorcraft"});
    let lib = s.execute("swatch.library.load", &p).unwrap()["library"].as_str().unwrap().to_string();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    s.execute("graph.design", &json!({"library": lib, "add": "Corner Mark"})).unwrap();
    let (from, to) = (&src.doc().unwrap().doc, &s.doc().unwrap().doc);
    assert!(from.symbol_nine_slice("Corner").is_some());
    assert_eq!(to.symbol_nine_slice("Corner"), from.symbol_nine_slice("Corner"));
    assert_eq!(to.symbol_registration("Corner"), from.symbol_registration("Corner"));
    assert_eq!(to.symbol_export_type("Corner"), from.symbol_export_type("Corner"));
}

#[test]
fn designs_and_markers_survive_save_and_open() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    save_design(&mut s, "Pill");
    let id = graph(&mut s, "line");
    s.execute("graph.marker", &json!({"seriesIndexes": [0], "design": "Pill"})).unwrap();
    let dir = vectorcraft_testkit::TempDir::new("graph-designs");
    let path = dir.join("designs.vectorcraft");
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
    let dir = vectorcraft_testkit::TempDir::new("graph-column-designs");
    let path = dir.join("columns.vectorcraft");
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
    let dir = vectorcraft_testkit::TempDir::new("graph-pie");
    let path = dir.join("pie.vectorcraft");
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
    let dir = vectorcraft_testkit::TempDir::new("graph-top");
    let path = dir.join("top.vectorcraft");
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
    let dir = vectorcraft_testkit::TempDir::new("graph-filled");
    let path = dir.join("filled.vectorcraft");
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

/// Line Width sets how wide Draw Filled Lines draws the bands and is kept through save and open. A width outside 0.01
/// to 1000 pt is refused, the same width again records no undo step, and the default 3 pt is stored as no width.
#[test]
fn filled_lines_take_a_line_width() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "line", ",a\nQ1,2\nQ2,2\nQ3,2");
    s.execute("graph.setType", &json!({"id": id.0, "filledLines": true})).unwrap();
    let height = |s: &Session| series_parts(s, id, 0)[0].geometric_bounds().unwrap().height();
    assert!((height(&s) - 3.0).abs() < 1e-6, "3 pt by default: {}", height(&s));
    assert_eq!(s.execute("graph.setType", &json!({"id": id.0})).unwrap()["lineWidth"], json!(3.0));
    s.execute("graph.setType", &json!({"id": id.0, "lineWidth": 8})).unwrap();
    assert!((height(&s) - 8.0).abs() < 1e-6, "{}", height(&s));
    assert_eq!(s.execute("graph.setType", &json!({"id": id.0})).unwrap()["lineWidth"], json!(8.0));
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let before = steps(&s);
    for p in [json!({"id": id.0, "lineWidth": 8}), json!({"id": id.0, "lineWidth": 8.0, "filledLines": true}), json!({"id": id.0, "lineWidth": null})]
    {
        s.execute("graph.setType", &p).unwrap();
        assert_eq!(steps(&s), before, "nothing changed: {p}");
    }
    for w in [json!(0), json!(-2), json!(0.005), json!(1000.5)] {
        let e = s.execute("graph.setType", &json!({"id": id.0, "lineWidth": w})).unwrap_err().to_string();
        assert!(e.contains(&format!("`lineWidth` must be from 0.01 to 1000 pt, not {}", w.as_f64().unwrap())), "{e}");
        assert_eq!((steps(&s), spec(&s, id).line_width), (before, Some(8.0)), "{w}");
    }
    s.execute("graph.setType", &json!({"id": id.0, "lineWidth": 0.01})).unwrap();
    assert_eq!(spec(&s, id).line_width, Some(0.01));
    s.execute("graph.setType", &json!({"id": id.0, "lineWidth": 1000})).unwrap();
    assert_eq!(spec(&s, id).line_width, Some(1000.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).line_width, Some(8.0));
    // Kept through save and open.
    let dir = vectorcraft_testkit::TempDir::new("graph-line-width");
    let path = dir.join("width.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!(spec(&s, id).line_width, Some(8.0));
    assert!((height(&s) - 8.0).abs() < 1e-6, "{}", height(&s));
    // The default width is stored as no width, so the file keeps no `lineWidth`.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setType", &json!({"id": id.0, "lineWidth": 3})).unwrap();
    assert_eq!(spec(&s, id).line_width, None);
    assert!(serde_json::to_value(spec(&s, id)).unwrap().get("lineWidth").is_none());
    assert!((height(&s) - 3.0).abs() < 1e-6, "{}", height(&s));
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

/// An anti-aliasing method given to one category label is kept for every category label when the graph is drawn
/// again, through save and open, and undoes with the change.
#[test]
fn a_graph_labels_anti_aliasing_method_survives_it_being_drawn_again() {
    use vectorcraft_doc::text::TypeAntiAlias;
    let methods = |s: &Session, id: NodeId| -> Vec<(String, TypeAntiAlias)> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let mut out = vec![];
        for g in ["Axes", "Legend"] {
            for c in group(&n, g).children().unwrap() {
                if let NodeKind::Text(t) = &c.kind {
                    out.push((t.plain_text(), t.anti_alias));
                }
            }
        }
        out
    };
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("text.setFormat", &json!({"ids": [c.0], "antiAlias": "strong"})).unwrap();
    label_looks(&mut s, id);
    let drawn = methods(&s, id);
    let method = |t: &str| drawn.iter().find(|(x, _)| x == t).map(|(_, m)| *m).unwrap();
    assert_eq!((method("Q1"), method("Q3")), (TypeAntiAlias::Strong, TypeAntiAlias::Strong), "every category label: {drawn:?}");
    assert_eq!((method("0"), method("2024")), (TypeAntiAlias::Sharp, TypeAntiAlias::Sharp), "other labels: {drawn:?}");
    // One undo takes the data change back, the next the method and the kept method together.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, id).label_styles.categories_anti_alias, None);
    s.execute("edit.redo", &json!({})).unwrap();
    let dir = vectorcraft_testkit::TempDir::new("graph-label-anti-alias");
    let path = dir.join("labels.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!(spec(&s, id).label_styles.categories_anti_alias, Some(TypeAntiAlias::Strong));
    assert!(serde_json::to_value(GraphSpec::default()).unwrap().get("labelStyles").is_none());
}

/// A stroke given to all of a graph's text without a weight is kept at 1 pt, as `paint.setStroke` keeps it.
#[test]
fn a_stroke_given_to_the_whole_graphs_text_is_kept_at_1_pt() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.setStyle", &json!({"stroke": "#ff0000"})).unwrap();
    let legend = spec(&s, id).label_styles.legend.unwrap();
    assert_eq!((legend.stroke.color().map(|c| c.to_hex()), legend.stroke_width), (Some("#ff0000".into()), 1.0));
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
    let dir = vectorcraft_testkit::TempDir::new("graph-labels");
    let path = dir.join("labels.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    assert_eq!(spec(&s, id).label_styles.categories.as_ref().unwrap().size, 14.0);
    assert!(serde_json::to_value(GraphSpec::default()).unwrap().get("labelStyles").is_none());
}

/// Character and paragraph edits that resize a graph's labels leave the plot where it is when the graph is drawn
/// again, and a move or scale of the graph made after them is still followed.
#[test]
fn restyling_a_graphs_labels_leaves_the_plot_in_place() {
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6";
    let redraw = |s: &mut Session, id: NodeId| s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = spec(&s, id).rect;
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 14})).unwrap();
    redraw(&mut s, id);
    assert_eq!(spec(&s, id).rect, rect, "after a size change to every label");
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("text.setFormat", &json!({"ids": [c.0], "leftIndent": 20})).unwrap();
    redraw(&mut s, id);
    assert_eq!(spec(&s, id).rect, rect, "after an indent on the category labels");
    let l = text_id(&s, id, "Legend", "2025");
    s.execute("text.setStyle", &json!({"ids": [l.0], "size": 4})).unwrap();
    redraw(&mut s, id);
    assert_eq!(spec(&s, id).rect, rect, "after a smaller size on the legend labels");
    // A move and a scale of the whole graph still carry the plot with them.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 40, "dy": -10})).unwrap();
    redraw(&mut s, id);
    let moved = spec(&s, id).rect;
    assert!((moved.x0 - (rect.x0 + 40.0)).abs() < 1e-6 && (moved.y0 - (rect.y0 - 10.0)).abs() < 1e-6, "{moved:?}");
    assert!((moved.width() - rect.width()).abs() < 1e-6 && (moved.height() - rect.height()).abs() < 1e-6, "{moved:?}");
    s.execute("object.scale", &json!({"sx": 200, "sy": 100, "origin": [moved.x0, moved.y0]})).unwrap();
    redraw(&mut s, id);
    let scaled = spec(&s, id).rect;
    assert!((scaled.width() - 2.0 * rect.width()).abs() < 1e-6 && (scaled.height() - rect.height()).abs() < 1e-6, "{scaled:?}");
    assert!((scaled.x0 - moved.x0).abs() < 1e-6 && (scaled.y0 - moved.y0).abs() < 1e-6, "{scaled:?}");
}

/// Moving one label of a graph on its own leaves the plot where it is when the graph is drawn again.
#[test]
fn moving_one_label_leaves_the_plot_in_place() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = spec(&s, id).rect;
    for (grp, text) in [("Legend", "2025"), ("Axes", "Q2")] {
        let l = text_id(&s, id, grp, text);
        s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
        s.execute("object.move", &json!({"dx": 120, "dy": 60})).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
        assert_eq!(spec(&s, id).rect, rect, "after moving {text}");
    }
}

/// Label edits leave the plot in place in graphs whose legend swatches follow the size of the labels at the next
/// drawing: a legend across the top, a legend after a right value axis, and legend labels of several lines. A move of
/// the graph made after them is still followed.
#[test]
fn restyling_the_labels_of_a_graph_whose_legend_follows_them_leaves_the_plot_in_place() {
    let csv = ",North|East|Coast,South|West,Inland|Hills|Lakes|Plains\nQ1,3,2,4\nQ2,5,4,1\nQ3,-1,6,2";
    for options in [json!({"legendAcrossTop": true}), json!({"valueAxis": "right"}), json!({"valueAxis": "both"}), json!({})] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.setType", &options).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
        let rect = spec(&s, id).rect;
        for size in [16, 5] {
            s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
            s.execute("text.setStyle", &json!({"size": size})).unwrap();
            for _ in 0..2 {
                s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
                assert_eq!(spec(&s, id).rect, rect, "{options}: labels at {size} pt");
            }
        }
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("object.move", &json!({"dx": 30, "dy": 20})).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
        let moved = spec(&s, id).rect;
        assert!((moved.x0 - (rect.x0 + 30.0)).abs() < 1e-6 && (moved.y0 - (rect.y0 + 20.0)).abs() < 1e-6, "{options}: {moved:?}");
        assert!((moved.width() - rect.width()).abs() < 1e-6 && (moved.height() - rect.height()).abs() < 1e-6, "{options}: {moved:?}");
    }
}

/// Outlining a graph's labels leaves the plot where it is when the graph is drawn again, and a move of the graph made
/// before them is still followed.
#[test]
fn outlining_a_graphs_labels_leaves_the_plot_in_place() {
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6";
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = spec(&s, id).rect;
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    s.execute("select.set", &json!({"ids": [text_id(&s, id, "Axes", "Q2").0, text_id(&s, id, "Legend", "2025").0]})).unwrap();
    s.execute("type.createOutlines", &json!({})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    assert_eq!(spec(&s, id).rect, rect, "after outlining labels");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 30, "dy": 15})).unwrap();
    s.execute("select.set", &json!({"ids": [text_id(&s, id, "Axes", "Q1").0, text_id(&s, id, "Legend", "2024").0]})).unwrap();
    s.execute("type.createOutlines", &json!({})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let moved = spec(&s, id).rect;
    assert!((moved.x0 - (rect.x0 + 30.0)).abs() < 1e-6 && (moved.y0 - (rect.y0 + 15.0)).abs() < 1e-6, "{moved:?}");
    assert!((moved.width() - rect.width()).abs() < 1e-6 && (moved.height() - rect.height()).abs() < 1e-6, "{moved:?}");
    // A scale of the graph made before outlining is still followed.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.scale", &json!({"sx": 150, "sy": 50, "origin": [moved.x0, moved.y0]})).unwrap();
    s.execute("select.set", &json!({"ids": [text_id(&s, id, "Axes", "Q3").0]})).unwrap();
    s.execute("type.createOutlines", &json!({})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let scaled = spec(&s, id).rect;
    assert!((scaled.x0 - moved.x0).abs() < 1e-6 && (scaled.y0 - moved.y0).abs() < 1e-6, "{scaled:?}");
    assert!((scaled.width() - 1.5 * rect.width()).abs() < 1e-6 && (scaled.height() - 0.5 * rect.height()).abs() < 1e-6, "{scaled:?}");
}

/// Outline Stroke on a graph's strokes leaves the plot where it is when the graph is drawn again, and a move of the
/// graph made before it is still followed.
#[test]
fn outlining_a_graphs_strokes_leaves_the_plot_in_place() {
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6";
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = spec(&s, id).rect;
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 20, "dy": 10})).unwrap();
    s.execute("object.path.outlineStroke", &json!({})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let moved = spec(&s, id).rect;
    assert!((moved.x0 - (rect.x0 + 20.0)).abs() < 1e-6 && (moved.y0 - (rect.y0 + 10.0)).abs() < 1e-6, "{moved:?}");
    assert!((moved.width() - rect.width()).abs() < 1e-6 && (moved.height() - rect.height()).abs() < 1e-6, "{moved:?}");
}

/// A graph left with only its labels, or with its labels and one vertical line, leaves the plot where it is when it is
/// drawn again after a label edit.
#[test]
fn a_graph_left_with_only_its_labels_leaves_the_plot_in_place() {
    for keep_a_line in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let rect = spec(&s, id).rect;
        let (mut art, mut line) = (vec![], None);
        s.doc().unwrap().doc.node(id).unwrap().walk(&mut |n| {
            if n.children().is_none() && !matches!(n.kind, NodeKind::Text(_)) {
                let vertical = n.geometric_bounds().is_some_and(|b| b.width() < 1e-9 && b.height() > 1.0);
                if keep_a_line && vertical && line.is_none() {
                    line = n.geometric_bounds();
                } else {
                    art.push(n.id.0);
                }
            }
        });
        assert!(!art.is_empty() && line.is_some() == keep_a_line);
        s.execute("select.set", &json!({"ids": art})).unwrap();
        s.execute("edit.clear", &json!({})).unwrap();
        let left = s.doc().unwrap().doc.node(id).unwrap().clone();
        assert_eq!(vectorcraft_doc::graph::graph_art_without_type(left.children().unwrap()), line, "{:#?}", left.children());
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("text.setStyle", &json!({"size": 20})).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
        assert_eq!(spec(&s, id).rect, rect, "keeping a line: {keep_a_line}");
    }
}

/// A graph saved without the bounds of its art without type, left with one vertical line, keeps its plot's size when
/// it is drawn again.
#[test]
fn a_graph_saved_with_only_its_whole_bounds_and_left_with_a_line_keeps_its_plot() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = spec(&s, id).rect;
    let mut old = spec(&s, id);
    old.placed_art = None;
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().extras_mut().graph = Some(Box::new(old));
    let (mut art, mut line) = (vec![], None);
    s.doc().unwrap().doc.node(id).unwrap().walk(&mut |n| {
        if n.children().is_none() {
            let vertical = !matches!(n.kind, NodeKind::Text(_)) && n.geometric_bounds().is_some_and(|b| b.width() < 1e-9 && b.height() > 1.0);
            if vertical && line.is_none() {
                line = Some(n.id);
            } else {
                art.push(n.id.0);
            }
        }
    });
    assert!(line.is_some());
    s.execute("select.set", &json!({"ids": art})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    assert_eq!(spec(&s, id).rect, rect);
}

/// The bounds a graph measures moves on take a clipping group holding type at the bounds of its clip, so art outside the
/// clip does not change them; a clipping group whose clip is type is measured on its other art.
#[test]
fn a_clipping_group_holding_type_is_measured_on_its_clip() {
    use std::sync::Arc;
    use vectorcraft_doc::graph::graph_art_without_type as measure;
    use vectorcraft_geom::Affine;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let label: Arc<vectorcraft_doc::Node> =
        group(&n, "Legend").children().unwrap().iter().find(|c| matches!(c.kind, NodeKind::Text(_))).unwrap().clone();
    let mut paths = vec![];
    n.walk(&mut |c| {
        if matches!(c.kind, NodeKind::Path { .. }) {
            paths.push(c.clone());
        }
    });
    let (clip, art) = (Arc::new(paths[0].clone()), paths[1].clone());
    let clipped = |dx: f64, first: Arc<vectorcraft_doc::Node>, second: Arc<vectorcraft_doc::Node>| {
        let mut far = art.clone();
        far.transform(Affine::translate((dx, 0.0)), false);
        let mut g = vectorcraft_doc::Node::group(NodeId(999_999), vec![first, second, Arc::new(far)]);
        if let NodeKind::Group { clip, .. } = &mut g.kind {
            *clip = true;
        }
        measure(&[Arc::new(g)])
    };
    assert_eq!(clipped(500.0, clip.clone(), label.clone()), clip.geometric_bounds());
    assert_eq!(clipped(500.0, clip.clone(), label.clone()), clipped(900.0, clip.clone(), label.clone()));
    // A clip that is type leaves the other art to measure.
    assert_ne!(clipped(500.0, label.clone(), clip.clone()), clipped(900.0, label.clone(), clip.clone()));
    // A clip that is a clipping group holding type in its clipped art is measured on its own clip.
    let mut inner = vectorcraft_doc::Node::group(NodeId(999_998), vec![clip.clone(), label.clone()]);
    if let NodeKind::Group { clip, .. } = &mut inner.kind {
        *clip = true;
    }
    let inner = Arc::new(inner);
    assert_eq!(clipped(500.0, inner.clone(), label.clone()), clip.geometric_bounds());
    assert_eq!(clipped(500.0, inner.clone(), label.clone()), clipped(900.0, inner.clone(), label.clone()));
    // A blend holding type is measured whole, as it is drawn.
    let blend = vectorcraft_doc::Node::new(NodeId(999_997), NodeKind::Blend { children: vec![clip.clone(), label.clone()], spec: Box::default() });
    assert_eq!(measure(&[Arc::new(blend.clone())]), blend.geometric_bounds());
}

/// A graph whose kept bounds of its art without type have no width follows a move with the bounds of all of its art.
#[test]
fn a_graph_whose_art_without_type_kept_no_width_still_follows_a_move() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = spec(&s, id).rect;
    let mut flat = spec(&s, id);
    let art = flat.placed_art.unwrap();
    flat.placed_art = Some(vectorcraft_geom::Rect::new(art.x0, art.y0, art.x0, art.y1));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().extras_mut().graph = Some(Box::new(flat));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 25, "dy": 5})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    let moved = spec(&s, id).rect;
    assert!((moved.x0 - (rect.x0 + 25.0)).abs() < 1e-6 && (moved.y0 - (rect.y0 + 5.0)).abs() < 1e-6, "{moved:?}");
}

/// A graph saved before it kept the bounds of its art without type follows a move made since with its old bounds.
#[test]
fn a_graph_saved_with_only_its_whole_bounds_still_follows_a_move() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = spec(&s, id).rect;
    let mut old = serde_json::to_value(spec(&s, id)).unwrap();
    old.as_object_mut().unwrap().remove("placedArt");
    let old: GraphSpec = serde_json::from_value(old).unwrap();
    assert_eq!(old.placed_art, None);
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().extras_mut().graph = Some(Box::new(old));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 25, "dy": 5})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    let moved = spec(&s, id).rect;
    assert!((moved.x0 - (rect.x0 + 25.0)).abs() < 1e-6 && (moved.y0 - (rect.y0 + 5.0)).abs() < 1e-6, "{moved:?}");
    assert!(spec(&s, id).placed_art.is_some(), "the next drawing keeps the bounds of the art without type");
}

/// Each Axes and Legend text after the graph is drawn again, with its object rows (fill or stroke, color, opacity, and
/// for a stroke its weight) and its object opacity.
type RowLooks = Vec<(bool, String, f32, f64)>;
fn redrawn_label_rows(s: &mut Session, id: NodeId) -> Vec<(String, RowLooks, f32)> {
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let mut out = vec![];
    for g in ["Axes", "Legend"] {
        for c in group(&n, g).children().unwrap() {
            if let NodeKind::Text(t) = &c.kind {
                let rows = c
                    .appearance
                    .items
                    .iter()
                    .map(|it| {
                        let w = match it {
                            vectorcraft_doc::AppearanceItem::Stroke(st) => st.width,
                            vectorcraft_doc::AppearanceItem::Fill(_) => 0.0,
                        };
                        (it.is_fill(), it.paint().color().map(|c| c.to_hex()).unwrap_or_default(), it.opacity(), w)
                    })
                    .collect();
                out.push((t.plain_text(), rows, c.opacity));
            }
        }
    }
    out
}

fn rows_of(looks: &[(String, RowLooks, f32)], text: &str) -> (RowLooks, f32) {
    looks.iter().find(|(t, _, _)| t == text).map(|(_, r, o)| (r.clone(), *o)).unwrap_or_else(|| panic!("no label {text}"))
}

/// Fill and stroke rows given to one label with the Appearance commands are kept for every label of its kind when the
/// graph is drawn again, through save and open, and undo with the change.
#[test]
fn appearance_rows_on_one_label_are_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    // A category label: a red fill row at 50%.
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [c.0], "index": 0, "color": "#ff0000", "opacity": 50})).unwrap();
    // A value axis number: a 2 pt blue stroke row.
    let v = text_id(&s, id, "Axes", "4");
    s.execute("appearance.addStroke", &json!({"ids": [v.0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [v.0], "index": 0, "color": "#0000ff", "weight": 2})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "Q1"), (vec![(true, "#ff0000".into(), 0.5, 0.0)], 1.0), "every category label: {looks:?}");
    assert_eq!(rows_of(&looks, "0"), (vec![(false, "#0000ff".into(), 1.0, 2.0)], 1.0), "every value axis number: {looks:?}");
    assert_eq!(rows_of(&looks, "2024"), (vec![], 1.0), "the legend keeps none: {looks:?}");
    // One undo takes the data change back, the next the stroke's color and weight together with what the graph keeps.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let kept = spec(&s, id).label_styles.values_rows.unwrap();
    assert_ne!(kept.paint.stroke.and_then(|p| p.color()).map(|c| c.to_hex()), Some("#0000ff".into()));
    assert_eq!(kept.paint.stroke_width, Some(1.0));
    s.execute("edit.redo", &json!({})).unwrap();
    let path = vectorcraft_testkit::temp_dir("graph-label-rows").join("labels.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let g = spec(&s, id);
    assert_eq!(g.label_styles.categories_rows.unwrap().paint.fill_opacity, Some(0.5));
    assert_eq!(g.label_styles.values_rows.unwrap().paint.stroke_width, Some(2.0));
    assert!(serde_json::to_value(vectorcraft_doc::LabelStyles::default()).unwrap().as_object().unwrap().is_empty());
}

/// Removing a label's rows, clearing its appearance or hiding its row leaves its kind with what the label then shows.
#[test]
fn removing_a_labels_rows_changes_what_its_kind_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [c.0], "index": 0, "color": "#ff0000"})).unwrap();
    s.execute("appearance.removeItem", &json!({"ids": [c.0], "index": 0})).unwrap();
    assert_eq!(spec(&s, id).label_styles.categories_rows, None, "no row left");
    s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [c.0], "index": 0, "color": "#ff0000", "visible": false})).unwrap();
    assert_eq!(spec(&s, id).label_styles.categories_rows, None, "a hidden row shows nothing");
    s.execute("appearance.showAllHidden", &json!({"ids": [c.0]})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "Q3").0, vec![(true, "#ff0000".into(), 1.0, 0.0)], "{looks:?}");
    let c = text_id(&s, id, "Axes", "Q1");
    s.execute("appearance.clear", &json!({"ids": [c.0]})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "Q3").0, vec![(true, String::new(), 1.0, 0.0), (false, String::new(), 1.0, 1.0)], "{looks:?}");
}

/// A fill row painted with `paint.setFill {item}` is kept with the label's rows, and its characters keep their own fill.
#[test]
fn painting_a_labels_row_keeps_the_row_and_not_its_characters() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let l = text_id(&s, id, "Legend", "2025");
    s.execute("appearance.addFill", &json!({"ids": [l.0]})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [l.0], "item": 0, "color": "#00ff00"})).unwrap();
    let g = spec(&s, id);
    assert_eq!(g.label_styles.legend_rows.and_then(|r| r.paint.fill).and_then(|p| p.color()).map(|c| c.to_hex()), Some("#00ff00".into()));
    assert_eq!(g.label_styles.legend, None, "the characters keep the default fill");
    let looks = label_looks(&mut s, id);
    assert!(looks.iter().find(|(t, _, _)| t == "2024").is_some_and(|(_, _, fill)| fill == "#000000"), "{looks:?}");
}

/// Moving or copying an appearance onto a label (Layers panel drag) keeps the source's rows and object opacity for the
/// label's kind.
#[test]
fn an_appearance_moved_onto_a_label_is_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [r.0], "color": "#12ab34"})).unwrap();
    s.execute("transparency.set", &json!({"ids": [r.0], "opacity": 40})).unwrap();
    let v = text_id(&s, id, "Axes", "2");
    s.execute("appearance.transfer", &json!({"source": r.0, "target": v.0, "copy": true})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    let (rows, opacity) = rows_of(&looks, "6");
    assert_eq!((rows.first().map(|r| r.1.clone()), opacity), (Some("#12ab34".into()), 0.4), "{looks:?}");
}

/// The Eyedropper onto a label keeps the characters' style, the paragraph attributes and the appended rows it gave the
/// label for every label of its kind.
#[test]
fn the_eyedropper_on_a_label_is_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let src = NodeId(s.execute("text.create", &json!({"x": 10, "y": 10, "text": "x", "size": 14})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [src.0], "color": "#00ff00"})).unwrap();
    let v = text_id(&s, id, "Axes", "4");
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("appearance.copyFrom", &json!({"source": src.0, "ids": [v.0]})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one step");
    let looks = label_looks(&mut s, id);
    assert!(looks.iter().any(|(t, size, fill)| t == "0" && *size == 14.0 && fill == "#00ff00"), "every value axis number: {looks:?}");
    assert!(looks.iter().any(|(t, size, fill)| t == "Q1" && *size == 9.0 && fill == "#000000"), "other labels: {looks:?}");
    // Appended rows (Shift+Alt-click) go with the label's rows.
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [r.0], "color": "#abcdef"})).unwrap();
    let c = text_id(&s, id, "Axes", "Q1");
    s.execute("appearance.copyFrom", &json!({"source": r.0, "ids": [c.0], "append": true})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert!(rows_of(&looks, "Q3").0.iter().any(|(fill, hex, _, _)| *fill && hex == "#abcdef"), "{looks:?}");
}

/// Kept rows read from a saved graph's label styles are held to the ranges the controls keep to, and a place for the
/// Characters row past the kept rows puts every row below the characters.
#[test]
fn kept_label_rows_from_a_file_stay_in_range() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let paint = |hex: &str| serde_json::to_value(Paint::solid(vectorcraft_color::Color::from_hex(hex).unwrap())).unwrap();
    let saved = json!({"legendRows": {
        "fill": paint("#ff0000"), "fillOpacity": 7.0,
        "stroke": paint("#0000ff"), "strokeWidth": 1e9, "strokeOpacity": -2.0,
        "opacity": -3.0, "below": 9,
    }});
    let styles: vectorcraft_doc::LabelStyles = serde_json::from_value(saved).unwrap();
    s.edit("rows", |d, _| {
        d.node_mut(id).unwrap().extras_mut().graph.as_deref_mut().unwrap().label_styles = styles;
        Ok(())
    })
    .unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "2024"), (vec![(true, "#ff0000".into(), 1.0, 0.0), (false, "#0000ff".into(), 0.0, 1000.0)], 0.0), "{looks:?}");
    assert_eq!(label_node(&s, id, "Legend", "2024").appearance.contents_at(), 2);
}

/// Label `text` of group `grp` of graph `id`.
fn label_node(s: &Session, id: NodeId, grp: &str, text: &str) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(text_id(s, id, grp, text)).unwrap().clone()
}

/// An edit that leaves a label showing what it showed still sets what its kind keeps.
#[test]
fn a_label_already_showing_a_row_still_sets_what_its_kind_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [c.0], "index": 0, "color": "#ff0000"})).unwrap();
    redrawn_label_rows(&mut s, id);
    let a = text_id(&s, id, "Axes", "Q1");
    s.execute("appearance.setItem", &json!({"ids": [a.0], "index": 0, "color": "#0000ff"})).unwrap();
    // Q3 shows the red the graph drew it with.
    let b = text_id(&s, id, "Axes", "Q3");
    s.execute("appearance.setItem", &json!({"ids": [b.0], "index": 0, "color": "#ff0000"})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "Q1").0, vec![(true, "#ff0000".into(), 1.0, 0.0)], "{looks:?}");
}

/// Paint given to a label's characters leaves the rows its kind keeps as they are.
#[test]
fn paint_on_a_labels_characters_leaves_the_rows_its_kind_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [c.0], "index": 0, "color": "#ff0000"})).unwrap();
    redrawn_label_rows(&mut s, id);
    let a = text_id(&s, id, "Axes", "Q1");
    s.execute("appearance.setItem", &json!({"ids": [a.0], "index": 0, "color": "#0000ff"})).unwrap();
    // Q3 still shows red; its characters take green.
    let b = text_id(&s, id, "Axes", "Q3");
    s.execute("paint.setFill", &json!({"ids": [b.0], "color": "#00ff00"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [b.0], "color": "#00ff00"})).unwrap();
    // The Eyedropper from type, transparency included, onto its characters.
    let src = NodeId(s.execute("text.create", &json!({"x": 10, "y": 10, "text": "x", "size": 14})).unwrap()["id"].as_u64().unwrap());
    s.execute("appearance.copyFrom", &json!({"source": src.0, "ids": [b.0]})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "Q1").0, vec![(true, "#0000ff".into(), 1.0, 0.0)], "{looks:?}");
}

/// When labels of one kind are edited together, the last one in the order of `ids` sets what the kind keeps.
#[test]
fn the_last_label_of_a_kind_edited_together_sets_what_it_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let (a, b) = (text_id(&s, id, "Axes", "Q1"), text_id(&s, id, "Axes", "Q2"));
    for (l, color) in [(a, "#ff0000"), (b, "#0000ff")] {
        s.execute("appearance.addFill", &json!({"ids": [l.0]})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [l.0], "index": 0, "color": color})).unwrap();
    }
    let kept = |s: &Session| spec(s, id).label_styles.categories_rows.unwrap().paint.fill.unwrap().color().unwrap().to_hex();
    s.execute("appearance.addFill", &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!(kept(&s), "#0000ff");
    s.execute("appearance.addFill", &json!({"ids": [b.0, a.0]})).unwrap();
    assert_eq!(kept(&s), "#ff0000");
}

/// A column graph, a 10 pt square with one red fill row, and the category labels Q1, Q2 and Q3: Q1 and Q3 have one red
/// fill row, Q2 a hidden red fill row under a green one, and Q1's row is then painted blue, which the kind keeps.
/// Returns the graph, the square and the three labels.
fn labels_with_hidden_and_shown_rows(s: &mut Session) -> (NodeId, NodeId, [NodeId; 3]) {
    use vectorcraft_doc::{AppearanceItem, FillLayer};
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(s, "column");
    let square = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    let labels = ["Q1", "Q2", "Q3"].map(|t| text_id(s, id, "Axes", t));
    let fill = |hex: &str, visible: bool| {
        AppearanceItem::Fill(FillLayer { visible, ..FillLayer::new(Paint::solid(vectorcraft_color::Color::from_hex(hex).unwrap())) })
    };
    s.edit("Rows", |d, _| {
        for (l, items) in [
            (square, vec![fill("#ff0000", true)]),
            (labels[0], vec![fill("#ff0000", true)]),
            (labels[1], vec![fill("#ff0000", false), fill("#00ff00", true)]),
            (labels[2], vec![fill("#ff0000", true)]),
        ] {
            d.node_mut(l).unwrap().appearance.items = items;
        }
        Ok(())
    })
    .unwrap();
    s.execute("appearance.setItem", &json!({"ids": [labels[0].0], "index": 0, "color": "#0000ff"})).unwrap();
    assert_eq!(kept_category_fill(s, id), Some("#0000ff".into()));
    (id, square, labels)
}

/// The fill of the category label rows graph `id` keeps, as a hex color.
fn kept_category_fill(s: &Session, id: NodeId) -> Option<String> {
    spec(s, id).label_styles.categories_rows.and_then(|r| r.paint.fill).and_then(|p| p.color()).map(|c| c.to_hex())
}

/// Row edits on several labels at once, with a plain object among them, keep for the kind what the last label whose shown
/// row the edit wrote shows, also when that row comes out as it was; a label whose hidden row the edit wrote is left out.
/// One undo takes it back. Covers `appearance.setItem`, `appearance.duplicateItem` and `paint.setFill` with `item`.
#[test]
fn row_edits_on_several_labels_keep_the_last_written_shown_row() {
    let edits = [
        ("appearance.setItem", json!({"index": 0, "color": "#ff0000"})),
        ("appearance.duplicateItem", json!({"index": 0})),
        ("paint.setFill", json!({"item": 0, "color": "#ff0000"})),
    ];
    for (cmd, mut p) in edits {
        let mut s = Session::new();
        let (id, square, [_, q2, q3]) = labels_with_hidden_and_shown_rows(&mut s);
        // Q3 shows red before and after the edit; Q2 shows green over the hidden row the edit writes.
        p["ids"] = json!([q3.0, square.0, q2.0]);
        s.execute(cmd, &p).unwrap();
        assert_eq!(kept_category_fill(&s, id), Some("#ff0000".into()), "{cmd}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(kept_category_fill(&s, id), Some("#0000ff".into()), "{cmd}: undo");
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(kept_category_fill(&s, id), Some("#ff0000".into()), "{cmd}: redo");
        s.execute("edit.undo", &json!({})).unwrap();
        // Only the hidden row of a label: the kind keeps blue.
        let steps = s.doc().unwrap().history.undo.len();
        p["ids"] = json!([square.0, q2.0]);
        s.execute(cmd, &p).unwrap();
        assert_eq!(kept_category_fill(&s, id), Some("#0000ff".into()), "{cmd}: hidden row only");
        while s.doc().unwrap().history.undo.len() > steps {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        // Q2's shown green row, written as it was: the kind keeps green.
        p["ids"] = json!([q2.0]);
        p[if cmd == "paint.setFill" { "item" } else { "index" }] = json!(1);
        if cmd != "appearance.duplicateItem" {
            p["color"] = json!("#00ff00");
        }
        s.execute(cmd, &p).unwrap();
        assert_eq!(kept_category_fill(&s, id), Some("#00ff00".into()), "{cmd}: the shown row written as it was");
    }
}

/// A label's object opacity and blend mode set without a row (`transparency.set` without `item`) are kept for its kind.
#[test]
fn a_labels_object_opacity_and_blend_mode_are_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    // Another value axis number sets the kind's fill row; the label below has none, and the kind keeps the row.
    let u = text_id(&s, id, "Axes", "2");
    s.execute("appearance.addFill", &json!({"ids": [u.0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [u.0], "index": 0, "color": "#0000ff"})).unwrap();
    let v = text_id(&s, id, "Axes", "4");
    s.execute("transparency.set", &json!({"ids": [v.0], "opacity": 50, "blend": "multiply"})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "0"), (vec![(true, "#0000ff".into(), 1.0, 0.0)], 0.5), "{looks:?}");
    assert_eq!(label_node(&s, id, "Axes", "0").blend, BlendMode::Multiply);
    assert_eq!(label_node(&s, id, "Axes", "Q1").blend, BlendMode::Normal);
    // Another label of the kind sets them again.
    let w = text_id(&s, id, "Axes", "2");
    s.execute("transparency.set", &json!({"ids": [w.0], "opacity": 50, "blend": "screen"})).unwrap();
    assert_eq!(label_node(&s, id, "Axes", "0").blend, BlendMode::Multiply);
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "0").1, 0.5, "{looks:?}");
    assert_eq!(label_node(&s, id, "Axes", "0").blend, BlendMode::Screen);
}

/// Stroke options given to a label's stroke row, a fill row painted as the Appearance panel's active row, and a row's
/// blend mode are kept for the label's kind.
#[test]
fn stroke_options_and_the_active_row_on_a_label_are_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let v = text_id(&s, id, "Axes", "4");
    s.execute("appearance.addStroke", &json!({"ids": [v.0]})).unwrap();
    s.execute("stroke.set", &json!({"ids": [v.0], "item": 0, "weight": 3})).unwrap();
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [c.0]})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": 0})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#00ffff"})).unwrap();
    s.execute("transparency.set", &json!({"item": 0, "blend": "multiply"})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert_eq!(rows_of(&looks, "0").0.first().map(|r| r.3), Some(3.0), "{looks:?}");
    assert_eq!(rows_of(&looks, "Q1").0.first().map(|r| r.1.clone()), Some("#00ffff".into()), "{looks:?}");
    let q1 = label_node(&s, id, "Axes", "Q1");
    assert_eq!(q1.appearance.items.first().map(|it| it.blend()), Some(BlendMode::Multiply));
    let NodeKind::Text(t) = &q1.kind else { panic!("not type") };
    assert_eq!(t.runs[0].style.fill.color().map(|c| c.to_hex()), Some("#000000".into()), "the characters keep their fill");
}

/// A row below the Characters row stays below the characters when the graph is drawn again.
#[test]
fn a_kept_row_keeps_its_place_below_the_characters() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("appearance.addStroke", &json!({"ids": [c.0]})).unwrap();
    s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
    // The stroke below the characters, the fill above them.
    s.execute("appearance.moveItem", &json!({"ids": [c.0], "from": "contents", "to": 1})).unwrap();
    let look = |ap: &vectorcraft_doc::Appearance| (ap.contents_at(), ap.items.iter().map(|it| it.is_fill()).collect::<Vec<_>>());
    assert_eq!(look(&label_node(&s, id, "Axes", "Q2").appearance), (1, vec![false, true]));
    redrawn_label_rows(&mut s, id);
    assert_eq!(look(&label_node(&s, id, "Axes", "Q1").appearance), (1, vec![false, true]));
}

/// The Eyedropper keeps for a label's kind only the attributes it applied: a fill the label had of its own stays out.
#[test]
fn the_eyedropper_keeps_only_what_it_applied_for_a_labels_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let src = NodeId(s.execute("text.create", &json!({"x": 10, "y": 10, "text": "x", "size": 14})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [src.0], "color": "#00ff00"})).unwrap();
    let v = text_id(&s, id, "Axes", "4");
    // A fill of the label's own, as the generator gives labels on dark pie wedges.
    s.edit("own fill", |d, _| {
        if let NodeKind::Text(t) = &mut d.node_mut(v).unwrap().kind {
            t.runs.iter_mut().for_each(|r| r.style.fill = Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0)));
        }
        Ok(())
    })
    .unwrap();
    // A pick of transparency alone leaves the kind's character style unset.
    let transparency = json!({"character": false, "paragraph": false, "appearance": {"fill": false, "stroke": false}});
    s.execute("appearance.copyFrom", &json!({"source": src.0, "ids": [v.0], "apply": transparency})).unwrap();
    assert_eq!(spec(&s, id).label_styles.values, None);
    s.execute("appearance.copyFrom", &json!({"source": src.0, "ids": [v.0], "apply": {"appearance": false}})).unwrap();
    let looks = label_looks(&mut s, id);
    assert!(looks.iter().any(|(t, size, fill)| t == "0" && *size == 14.0 && fill == "#000000"), "{looks:?}");
}

/// Paint given to all of a label's text highlighted with the Type tool is kept for its kind, as for the whole label.
#[test]
fn paint_on_a_labels_highlighted_text_is_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let v = text_id(&s, id, "Axes", "4");
    s.select_tool("type", crate::ViewInfo::default()).unwrap();
    s.set_tool_option("textSelection", &json!({"id": v.0, "start": 0, "end": 1}));
    s.execute("paint.setFill", &json!({"color": "#ff8800"})).unwrap();
    s.select_tool("selection", crate::ViewInfo::default()).unwrap();
    let looks = label_looks(&mut s, id);
    assert!(looks.iter().any(|(t, _, fill)| t == "0" && fill == "#ff8800"), "{looks:?}");
}

/// Paint given to the whole graph without a row is kept for the characters of every kind of label.
#[test]
fn paint_on_the_whole_graph_is_kept_for_every_kind_of_label() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff00ff"})).unwrap();
    let looks = label_looks(&mut s, id);
    for t in ["0", "Q1", "2024"] {
        assert!(looks.iter().any(|(x, _, fill)| x == t && fill == "#ff00ff"), "{t}: {looks:?}");
    }
}

/// A gradient placed on one label's characters or on its row is kept without that placement, so it fits each label of
/// the kind when the graph is drawn again.
#[test]
fn a_placed_gradient_on_a_label_is_kept_to_fit_each_label() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let gradient = json!({"start": [30, 60], "end": [90, 60]});
    let v = text_id(&s, id, "Axes", "4");
    s.execute("paint.setFill", &json!({"ids": [v.0], "gradient": gradient})).unwrap();
    let c = text_id(&s, id, "Axes", "Q2");
    s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [c.0], "item": 0, "gradient": gradient})).unwrap();
    let placed = |p: &Paint| matches!(p, Paint::Gradient(g) if g.geom.is_some());
    let g = spec(&s, id);
    assert!(g.label_styles.values.is_some_and(|st| matches!(st.fill, Paint::Gradient(_)) && !placed(&st.fill)));
    assert!(g.label_styles.categories_rows.and_then(|r| r.paint.fill).is_some_and(|f| matches!(f, Paint::Gradient(_)) && !placed(&f)));
}

/// A graphic style applied to a label is kept with the label's rows for its kind.
#[test]
fn a_graphic_style_on_a_label_is_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let r = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [r.0], "color": "#ff8000"})).unwrap();
    s.execute("graphicStyle.new", &json!({"id": r.0, "name": "Orange"})).unwrap();
    let v = text_id(&s, id, "Axes", "4");
    s.execute("graphicStyle.apply", &json!({"name": "Orange", "ids": [v.0]})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert!(rows_of(&looks, "0").0.iter().any(|(fill, hex, _, _)| *fill && hex == "#ff8000"), "{looks:?}");
    // A label still linked to the style follows a redefinition, and its kind keeps the new look.
    let v = text_id(&s, id, "Axes", "4");
    s.execute("graphicStyle.apply", &json!({"name": "Orange", "ids": [v.0]})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [r.0], "color": "#0080ff"})).unwrap();
    s.execute("graphicStyle.redefine", &json!({"name": "Orange", "id": r.0})).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    assert!(rows_of(&looks, "0").0.iter().any(|(fill, hex, _, _)| *fill && hex == "#0080ff"), "{looks:?}");
}

/// The character style of the first run of label `text` in group `grp`.
fn label_run_style(s: &Session, id: NodeId, grp: &str, text: &str) -> CharStyle {
    let NodeKind::Text(t) = label_node(s, id, grp, text).kind else { panic!("{text} is not type") };
    t.runs[0].style.clone()
}

/// A recolor of the whole graph (Invert or Complement on the proxy, an Edit Colors command, a color filter or Recolor
/// Artwork) is kept for the characters and the rows of every kind of label when the graph is drawn again, and undo takes
/// it back with its edit.
#[test]
fn recoloring_a_graph_keeps_its_labels_paint_for_every_kind() {
    let cases = [
        ("paint.invert", json!({})),
        ("paint.complement", json!({})),
        ("edit.colors.invert", json!({})),
        ("edit.colors.toGrayscale", json!({})),
        ("addon.color.filter", json!({"effect": "addon.color.invert"})),
        ("recolor.apply", json!({"map": {"#3366cc": "#00ff00", "#ff8000": "#8000ff"}})),
    ];
    every_case(cases, |(cmd, params)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        // Characters of every label in a color that every case changes, and an orange fill row on the category labels.
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("paint.setFill", &json!({"color": "#3366cc"})).unwrap();
        let c = text_id(&s, id, "Axes", "Q2");
        s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [c.0], "index": 0, "color": "#ff8000"})).unwrap();
        let was = spec(&s, id).label_styles;
        let mut p = params.clone();
        if *cmd == "recolor.apply" {
            // Recolor Artwork works on the selection.
            s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        } else {
            p["ids"] = json!([id.0]);
        }
        s.execute(cmd, &p).unwrap();
        let hex = |p: &Paint| p.color().map(|c| c.to_hex()).unwrap_or_default();
        let characters = hex(&label_run_style(&s, id, "Axes", "4").fill);
        assert_ne!(characters, "#3366cc", "{cmd} recolors the characters");
        let row = label_node(&s, id, "Axes", "Q2").appearance.items[0].paint().clone();
        assert_ne!(hex(&row), "#ff8000", "{cmd} recolors the row");
        let looks = label_looks(&mut s, id);
        for t in ["0", "Q1", "2024"] {
            assert!(looks.iter().any(|(x, _, fill)| x == t && *fill == characters), "{cmd}: {t}: {looks:?}");
        }
        let rows = redrawn_label_rows(&mut s, id);
        assert_eq!(rows_of(&rows, "Q1").0.first().map(|r| r.1.clone()), Some(hex(&row)), "{cmd}: {rows:?}");
        // Undo takes back the recolor and what the graph keeps (the two redraws are undone first).
        for _ in 0..3 {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(spec(&s, id).label_styles, was, "{cmd}");
    });
}

/// A recolor of one label changes what the graph keeps for that kind of label only.
#[test]
fn recoloring_one_label_keeps_its_paint_for_its_kind_only() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let v = text_id(&s, id, "Axes", "4");
    s.execute("edit.colors.invert", &json!({"ids": [v.0]})).unwrap();
    let looks = label_looks(&mut s, id);
    assert!(looks.iter().any(|(t, _, fill)| t == "0" && fill == "#ffffff"), "{looks:?}");
    assert!(looks.iter().any(|(t, _, fill)| t == "Q1" && fill == "#000000"), "{looks:?}");
    assert!(looks.iter().any(|(t, _, fill)| t == "2024" && fill == "#000000"), "{looks:?}");
    let g = spec(&s, id).label_styles;
    assert!(g.categories.is_none() && g.legend.is_none(), "{g:?}");
}

/// A recolor that leaves a label's first characters as they were and gives the rest the same fill is kept for its kind.
#[test]
fn a_recolor_that_evens_out_a_labels_characters_is_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let q2 = text_id(&s, id, "Axes", "Q2");
    s.select_tool("type", crate::ViewInfo::default()).unwrap();
    for (start, end, hex) in [(0, 1, "#ff0000"), (1, 2, "#0000ff")] {
        s.set_tool_option("textSelection", &json!({"id": q2.0, "start": start, "end": end}));
        s.execute("paint.setFill", &json!({"color": hex})).unwrap();
    }
    s.select_tool("selection", crate::ViewInfo::default()).unwrap();
    assert_eq!(spec(&s, id).label_styles.categories, None, "part of a label's text is not kept");
    s.execute("select.set", &json!({"ids": [q2.0]})).unwrap();
    s.execute("recolor.apply", &json!({"map": {"#0000ff": "#ff0000"}})).unwrap();
    let looks = label_looks(&mut s, id);
    assert!(looks.iter().any(|(t, _, fill)| t == "Q1" && fill == "#ff0000"), "{looks:?}");
    assert!(looks.iter().any(|(t, _, fill)| t == "0" && fill == "#000000"), "{looks:?}");
}

/// Stroke options set with `stroke.set` and a gradient made with `paint.editGradient` and placed with
/// `paint.setGradientGeom` on a label's characters are kept for every label of its kind, the gradient without its
/// placement.
#[test]
fn stroke_and_gradient_edits_on_a_labels_characters_are_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let category = label_run_style(&s, id, "Axes", "Q1");
    let v = text_id(&s, id, "Axes", "4");
    s.execute("paint.setStroke", &json!({"ids": [v.0], "color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [v.0], "weight": 3, "cap": "round"})).unwrap();
    s.execute("paint.editGradient", &json!({"ids": [v.0], "kind": "linear", "stroke": false})).unwrap();
    let placement = json!({"ids": [v.0], "stroke": false, "start": [100, 100], "end": [200, 100], "aspect": 50});
    s.execute("paint.setGradientGeom", &placement).unwrap();
    let fill = label_run_style(&s, id, "Axes", "4").fill;
    assert!(matches!(&fill, Paint::Gradient(g) if g.geom.is_some()), "the edited label's gradient is placed: {fill:?}");
    label_looks(&mut s, id);
    let st = label_run_style(&s, id, "Axes", "0");
    assert_eq!((st.stroke_width, st.stroke_cap), (3.0, vectorcraft_doc::LineCap::Round));
    assert!(matches!(&st.fill, Paint::Gradient(g) if g.geom.is_none()), "{:?}", st.fill);
    assert_eq!(label_run_style(&s, id, "Axes", "Q1"), category, "the category labels are left as they were");
}

/// `stroke.set`, `stroke.weightUp` and `stroke.weightDown` given the whole graph are kept for every kind of label.
#[test]
fn stroke_weights_set_on_the_whole_graph_are_kept_for_its_labels() {
    let cases = [("stroke.set", json!({"weight": 2}), 2.0), ("stroke.weightUp", json!({}), 2.0), ("stroke.weightDown", json!({}), 0.75)];
    every_case(cases, |(cmd, params, weight)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        s.execute("paint.setStroke", &json!({"ids": [id.0], "color": "#ff0000"})).unwrap();
        let mut p = params.clone();
        p["ids"] = json!([id.0]);
        s.execute(cmd, &p).unwrap();
        assert_eq!(label_run_style(&s, id, "Axes", "4").stroke_width, *weight, "{cmd}");
        label_looks(&mut s, id);
        for (grp, t) in [("Axes", "0"), ("Axes", "Q1"), ("Legend", "2024")] {
            assert_eq!(label_run_style(&s, id, grp, t).stroke_width, *weight, "{cmd}: {t}");
        }
    });
}

/// A stroke weight set on a scaled label is kept as the label shows it, since the graph draws its labels unscaled.
#[test]
fn a_stroke_weight_on_a_scaled_label_is_kept_as_it_shows() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let v = text_id(&s, id, "Axes", "4");
    s.execute("select.set", &json!({"ids": [v.0]})).unwrap();
    s.execute("object.scale", &json!({"sx": 200, "sy": 200})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [v.0], "color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [v.0], "weight": 3})).unwrap();
    assert_eq!(label_run_style(&s, id, "Axes", "4").stroke_width, 1.5, "drawn at twice its size");
    label_looks(&mut s, id);
    assert_eq!(label_run_style(&s, id, "Axes", "0").stroke_width, 3.0);
}

/// When the characters of two labels of one kind are recolored differently, the frontmost one sets what the kind
/// keeps, whatever order `ids` are in.
#[test]
fn the_frontmost_label_of_a_kind_sets_what_its_recolor_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    // Legend labels: the axes tell value and category labels apart by their order, so these are moved instead.
    let (a, b) = (text_id(&s, id, "Legend", "2024"), text_id(&s, id, "Legend", "2025"));
    s.execute("paint.setFill", &json!({"ids": [a.0], "color": "#ff0000"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [b.0], "color": "#0000ff"})).unwrap();
    // The label made first goes in front, so neither the order of `ids` nor the order the labels were made in decides.
    let (front, back) = if a < b { (a, b) } else { (b, a) };
    s.execute("select.set", &json!({"ids": [front.0]})).unwrap();
    s.execute("object.arrange.bringToFront", &json!({})).unwrap();
    let path = |n: NodeId| s.doc().unwrap().doc.index_path(n);
    assert!(path(front) > path(back), "{:?} {:?}", path(front), path(back));
    let front_fill = if front == a { "#00ffff" } else { "#ffff00" };
    s.execute("edit.colors.invert", &json!({"ids": [front.0, back.0]})).unwrap();
    let kept = spec(&s, id).label_styles.legend.unwrap().fill;
    assert_eq!(kept.color().map(|c| c.to_hex()).as_deref(), Some(front_fill));
}

/// Apply Nearest Swatch and the ink limit of Print Ink Fixes on a label are kept for its kind.
#[test]
fn nearest_swatch_and_ink_limit_on_a_label_are_kept_for_its_kind() {
    let cases = [
        ("addon.color.applyNearestSwatch", json!({"library": "web-safe-216"})),
        ("addon.color.printFixes", json!({"scope": "selection", "inkLimit": 200})),
    ];
    every_case(cases, |(cmd, params)| {
        let mut s = Session::new();
        // Print Ink Fixes works on CMYK documents.
        s.execute("file.new", &json!({"width": 800, "height": 800, "colorMode": "cmyk"})).unwrap();
        let id = graph(&mut s, "column");
        let v = text_id(&s, id, "Axes", "4");
        s.execute("paint.setFill", &json!({"ids": [v.0], "color": {"c": 90, "m": 80, "y": 70, "k": 60}})).unwrap();
        let was = label_run_style(&s, id, "Axes", "4").fill;
        let mut p = params.clone();
        p["ids"] = json!([v.0]);
        s.execute(cmd, &p).unwrap();
        let now = label_run_style(&s, id, "Axes", "4").fill;
        assert_ne!(now, was, "{cmd} changes the fill");
        label_looks(&mut s, id);
        assert_eq!(label_run_style(&s, id, "Axes", "0").fill, now, "{cmd}");
    });
}

/// Swap Fill and Stroke and Default Fill and Stroke on a label are kept for its kind.
#[test]
fn swap_and_default_fill_and_stroke_on_a_label_are_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let v = text_id(&s, id, "Axes", "4");
    s.execute("paint.setStroke", &json!({"ids": [v.0], "color": "#ff0000"})).unwrap();
    s.execute("paint.swap", &json!({"ids": [v.0]})).unwrap();
    let swapped = label_run_style(&s, id, "Axes", "4");
    label_looks(&mut s, id);
    let st = label_run_style(&s, id, "Axes", "0");
    assert_eq!((&st.fill, &st.stroke), (&swapped.fill, &swapped.stroke));
    assert_eq!(st.fill.color().map(|c| c.to_hex()).as_deref(), Some("#ff0000"));
    s.execute("paint.default", &json!({"ids": [text_id(&s, id, "Axes", "4").0]})).unwrap();
    label_looks(&mut s, id);
    let st = label_run_style(&s, id, "Axes", "0");
    assert_eq!((st.fill.color().map(|c| c.to_hex()).as_deref(), st.stroke.is_none()), (Some("#000000"), true));
}

/// On a pie with Legends in Wedges, a recolor of the whole graph leaves the legend labels to the graph's black or white.
#[test]
fn a_recolor_leaves_wedge_legend_labels_their_contrast() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "pie", ",a,b\nP1,1,2");
    s.execute("graph.setType", &json!({"id": id.0, "pieLegend": "wedges"})).unwrap();
    s.execute("edit.colors.invert", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(spec(&s, id).label_styles.legend, None);
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2]]})).unwrap();
    // The first series' black wedge is white now, so its label is black.
    let NodeKind::Text(t) = label_node(&s, id, "Legend", "a").kind else { panic!("a is not type") };
    assert_eq!(t.runs[0].style.fill.color().map(|c| c.to_hex()).as_deref(), Some("#000000"));
    // A recolor of one legend label gives the kind that label's fill.
    s.execute("edit.colors.invert", &json!({"ids": [text_id(&s, id, "Legend", "a").0]})).unwrap();
    let kept = spec(&s, id).label_styles.legend.map(|st| st.fill);
    assert_eq!(kept.and_then(|f| f.color()).map(|c| c.to_hex()).as_deref(), Some("#ffffff"));
}

/// A recolor of the whole graph that turns every wedge legend label the same color (one series) still leaves them to
/// the graph's black or white, so a label stays readable when its wedge is painted light again.
#[test]
fn a_recolor_of_a_one_series_pie_leaves_its_wedge_label_its_contrast() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "pie", ",a\nP1,1");
    s.execute("graph.setType", &json!({"id": id.0, "pieLegend": "wedges"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [series_0_group(&s, id).0.0], "color": "#ffffff"})).unwrap();
    // The white wedge turns black and its black label white.
    s.execute("edit.colors.invert", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(spec(&s, id).label_styles.legend, None);
    s.execute("paint.setFill", &json!({"ids": [series_0_group(&s, id).0.0], "color": "#ffffff"})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1]]})).unwrap();
    let NodeKind::Text(t) = label_node(&s, id, "Legend", "a").kind else { panic!("a is not type") };
    assert_eq!(t.runs[0].style.fill.color().map(|c| c.to_hex()).as_deref(), Some("#000000"));
}

/// The lines in graph `id`'s Axes group (axis lines, tick marks and a radar graph's rings), in paint order.
fn line_nodes(s: &Session, id: NodeId) -> Vec<vectorcraft_doc::Node> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    group(&n, "Axes").children().unwrap().iter().filter(|c| !matches!(c.kind, NodeKind::Text(_))).map(|c| (**c).clone()).collect()
}

/// How line `n` looks: its stroke color (empty without one), stroke weight and object opacity.
fn line_look(n: &vectorcraft_doc::Node) -> (String, f64, f32) {
    let stroke = n.appearance.stroke_paint().color().map(|c| c.to_hex()).unwrap_or_default();
    (stroke, n.appearance.stroke_width(), n.opacity)
}

/// The looks of graph `id`'s lines after it is drawn again.
fn redrawn_line_looks(s: &mut Session, id: NodeId) -> Vec<(String, f64, f32)> {
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    line_nodes(s, id).iter().map(line_look).collect()
}

/// A stroke, a weight, no stroke or an opacity given to one axis line is kept for every axis line and tick mark when the
/// graph is drawn again, and undo takes it back with its edit.
#[test]
fn paint_set_on_one_axis_line_is_kept_for_every_line() {
    let cases = [
        ("paint.setStroke", json!({"color": "#ff0000"}), ("#ff0000", 0.5, 1.0)),
        ("stroke.set", json!({"weight": 2}), ("#000000", 2.0, 1.0)),
        ("paint.setStroke", json!({"none": true}), ("", 0.0, 1.0)),
        ("appearance.clear", json!({}), ("", 0.0, 1.0)),
        ("transparency.set", json!({"opacity": 40}), ("#000000", 0.5, 0.4)),
    ];
    every_case(cases, |(cmd, params, (hex, width, opacity))| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        assert!(lines.len() > 2, "{lines:?}");
        let mut p = params.clone();
        p["ids"] = json!([lines[0].id.0]);
        s.execute(cmd, &p).unwrap();
        let looks = redrawn_line_looks(&mut s, id);
        let want = (hex.to_string(), *width, *opacity);
        assert!(looks.iter().all(|l| (&l.0, (l.1 - want.1).abs() < 1e-9, (l.2 - want.2).abs() < 1e-6) == (&want.0, true, true)), "{cmd}: {looks:?}");
        // Undo takes back the redraw, then the edit and what the graph keeps.
        for _ in 0..2 {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(spec(&s, id).label_styles.axis_rows, None, "{cmd}");
    });
}

/// A recolor of the whole graph, or Swap Fill and Stroke on its axis lines, is kept for its axis lines, and the labels'
/// paint stays its own.
#[test]
fn recoloring_a_graph_keeps_its_axis_lines_paint() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("edit.colors.invert", &json!({"ids": [id.0]})).unwrap();
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| l.0 == "#ffffff"), "inverted black lines are white");
    let ids: Vec<u64> = line_nodes(&s, id).iter().map(|n| n.id.0).collect();
    s.execute("paint.swap", &json!({"ids": ids})).unwrap();
    let kept = spec(&s, id).label_styles.axis_rows.unwrap();
    assert_eq!((kept.paint.fill.and_then(|p| p.color()).map(|c| c.to_hex()), kept.paint.stroke), (Some("#ffffff".into()), Some(Paint::None)));
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| l.0.is_empty()), "the lines have no stroke after the swap");
    // The value labels keep their inverted fill, not the lines' swapped paint.
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let label_paint: Vec<(String, bool)> = group(&n, "Axes")
        .children()
        .unwrap()
        .iter()
        .filter_map(|c| match &c.kind {
            NodeKind::Text(t) => Some((t.runs[0].style.fill.color().map(|c| c.to_hex()).unwrap_or_default(), t.runs[0].style.stroke.is_none())),
            _ => None,
        })
        .collect();
    assert!(!label_paint.is_empty() && label_paint.iter().all(|l| *l == ("#ffffff".to_string(), true)), "{label_paint:?}");
}

/// Swap Fill and Stroke on an axis line keeps the swapped paint for every axis line and leaves the fill opacity the axis
/// lines keep from another line, though the line swapped still shows 100%.
#[test]
fn swapping_an_axis_lines_paint_keeps_the_fill_opacity_its_kind_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let lines = line_nodes(&s, id);
    let (first, second) = (lines[0].id, lines[1].id);
    s.execute("paint.setFill", &json!({"ids": [first.0], "color": "#00ff00"})).unwrap();
    s.execute("transparency.set", &json!({"ids": [first.0], "item": top_row(&s, first, true), "opacity": 40})).unwrap();
    let shown = s.doc().unwrap().doc.node(second).unwrap().appearance.fill().map(|f| f.opacity);
    assert!(shown.is_none_or(|o| o == 1.0), "the other lines show 100% until the graph is drawn again: {shown:?}");
    s.execute("paint.swap", &json!({"ids": [second.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    for n in line_nodes(&s, id) {
        let fill = n.appearance.fill().map(|f| (f.paint.color().map(|c| c.to_hex()), f.opacity));
        assert!(fill.as_ref().is_some_and(|(c, o)| c.as_deref() == Some("#000000") && (*o - 0.4).abs() < 1e-6), "{fill:?}");
        assert!(n.appearance.stroke_paint().is_none(), "the swapped line had no fill, so the lines have no stroke");
    }
}

/// A stroke given the whole graph, or its Axes group, is kept for its axis lines.
#[test]
fn a_stroke_given_the_whole_graph_or_its_axes_group_is_kept_for_its_axis_lines() {
    for axes in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let target = if axes { group(s.doc().unwrap().doc.node(id).unwrap(), "Axes").id } else { id };
        s.execute("paint.setStroke", &json!({"ids": [target.0], "color": "#ff0000"})).unwrap();
        assert!(redrawn_line_looks(&mut s, id).iter().all(|l| l.0 == "#ff0000"), "axes: {axes}");
    }
}

/// A radar graph's rings keep what is set on one of them, apart from its spokes.
#[test]
fn a_radar_graphs_rings_keep_their_paint_apart_from_its_spokes() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "radar");
    let lines = line_nodes(&s, id);
    // Three categories: three spokes, then the rings.
    assert_eq!(line_look(&lines[3]), ("#999999".into(), 0.25, 1.0));
    s.execute("paint.setStroke", &json!({"ids": [lines[3].id.0], "color": "#00ff00"})).unwrap();
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks[..3].iter().all(|l| l.0 == "#000000" && l.1 == 0.5), "{looks:?}");
    assert!(looks[3..].iter().all(|l| l.0 == "#00ff00" && l.1 == 0.25), "{looks:?}");
    s.execute("paint.setStroke", &json!({"ids": [line_nodes(&s, id)[0].id.0], "color": "#0000ff"})).unwrap();
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks[..3].iter().all(|l| l.0 == "#0000ff") && looks[3..].iter().all(|l| l.0 == "#00ff00"), "{looks:?}");
}

/// When an edit gives axis lines different paint, the frontmost one sets what they keep, whatever order `ids` are in.
#[test]
fn the_frontmost_axis_line_sets_what_its_recolor_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let lines = line_nodes(&s, id);
    let (back, front) = (lines[0].id, lines[lines.len() - 1].id);
    s.execute("paint.setStroke", &json!({"ids": [front.0], "color": "#ff0000"})).unwrap();
    s.execute("edit.colors.invert", &json!({"ids": [front.0, back.0]})).unwrap();
    let kept = spec(&s, id).label_styles.axis_rows.unwrap().paint.stroke.and_then(|p| p.color()).map(|c| c.to_hex());
    assert_eq!(kept.as_deref(), Some("#00ffff"), "the inverted red of the frontmost line");
}

/// An appearance moved from an axis line to one behind it is kept for every axis line, not the cleared source's.
#[test]
fn an_appearance_moved_to_an_axis_line_behind_is_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let lines = line_nodes(&s, id);
    let (back, front) = (lines[0].id, lines[lines.len() - 1].id);
    s.execute("paint.setStroke", &json!({"ids": [front.0], "color": "#ff0000"})).unwrap();
    s.execute("appearance.transfer", &json!({"source": front.0, "target": back.0})).unwrap();
    let kept = spec(&s, id).label_styles.axis_rows.unwrap().paint.stroke.and_then(|p| p.color()).map(|c| c.to_hex());
    assert_eq!(kept.as_deref(), Some("#ff0000"), "the moved red, not the cleared source");
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| l.0 == "#ff0000"));
}

/// A radar graph's spoke brought in front of its rings is still a spoke: what is set on it is kept for the spokes.
#[test]
fn a_radar_spoke_moved_in_front_of_the_rings_keeps_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "radar");
    let spoke = line_nodes(&s, id)[0].id;
    s.execute("object.arrange.bringToFront", &json!({"ids": [spoke.0]})).unwrap();
    assert_eq!(line_nodes(&s, id).last().map(|n| n.id), Some(spoke), "the spoke is in front of the rings");
    s.execute("paint.setStroke", &json!({"ids": [spoke.0], "color": "#0000ff"})).unwrap();
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks[..3].iter().all(|l| l.0 == "#0000ff") && looks[3..].iter().all(|l| l.0 == "#999999"), "{looks:?}");
}

/// A graphic style linked to an axis line passes a redefinition on to every axis line.
#[test]
fn a_redefined_graphic_style_on_an_axis_line_is_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#ff0000"})).unwrap();
    s.execute("graphicStyle.new", &json!({"id": rect.0, "name": "Axis"})).unwrap();
    let line = line_nodes(&s, id)[0].id;
    s.execute("graphicStyle.apply", &json!({"name": "Axis", "ids": [line.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#00ff00"})).unwrap();
    s.execute("graphicStyle.redefine", &json!({"name": "Axis", "id": rect.0})).unwrap();
    let kept = spec(&s, id).label_styles.axis_rows.unwrap().paint.stroke.and_then(|p| p.color()).map(|c| c.to_hex());
    assert_eq!(kept.as_deref(), Some("#00ff00"));
}

/// An Eyedropper pick and Apply Stroke to Fill on an axis line are kept for every axis line.
#[test]
fn an_eyedropper_pick_and_stroke_to_fill_on_an_axis_line_are_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#ff0000"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [rect.0], "none": true})).unwrap();
    let line = line_nodes(&s, id)[0].id;
    s.execute("appearance.copyFrom", &json!({"source": rect.0, "ids": [line.0]})).unwrap();
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| l.0 == "#ff0000" && l.1 == 1.0), "the picked 1 pt red stroke");
    let line = line_nodes(&s, id)[1].id;
    s.execute("addon.color.strokeToFill", &json!({"ids": [line.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    let fills: Vec<String> = line_nodes(&s, id).iter().map(|n| n.appearance.fill_paint().color().map(|c| c.to_hex()).unwrap_or_default()).collect();
    assert!(fills.iter().all(|f| f == "#ff0000"), "{fills:?}");
}

/// Setting a line to the paint it is drawn with by default keeps nothing, and is no edit.
#[test]
fn axis_lines_given_their_default_paint_keep_nothing() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let line = line_nodes(&s, id)[0].id;
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("paint.setStroke", &json!({"ids": [line.0], "color": "#000000"})).unwrap();
    assert_eq!(spec(&s, id).label_styles.axis_rows, None);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "nothing changed");
}

/// `object.setProps` and `transparency.set` without an `item` keep only a line's object opacity: the stroke its kind
/// keeps from another line stays, though the line they are given still shows the default stroke.
#[test]
fn an_object_opacity_on_an_axis_line_keeps_the_stroke_its_kind_keeps() {
    for (cmd, params) in [("object.setProps", json!({"opacity": 50})), ("transparency.set", json!({"opacity": 50}))] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
        assert_eq!(line_look(&line_nodes(&s, id)[1]).0, "#000000", "the other lines show black until the graph is drawn again");
        let mut p = params.clone();
        p["ids"] = json!([lines[1].id.0]);
        s.execute(cmd, &p).unwrap();
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| l.0 == "#ff0000" && (l.2 - 0.5).abs() < 1e-6), "{cmd}: {looks:?}");
    }
}

/// A stroke painted on an axis line after another axis line's stroke was set to None is kept for every axis line with
/// the painted line's weight, not the zero weight the unpainted stroke left on its kind.
#[test]
fn a_stroke_painted_after_an_axis_line_lost_its_stroke_keeps_a_weight() {
    let cases = ["paint.setStroke", "addon.color.fillToStroke", "appearance.setItem"];
    every_case(cases, |cmd| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        let weight = line_look(&lines[1]).1;
        assert!(weight > 0.0, "{cmd}: the line has a stroke weight: {weight}");
        s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "none": true})).unwrap();
        let b = lines[1].id.0;
        match *cmd {
            "paint.setStroke" => s.execute(cmd, &json!({"ids": [b], "color": "#ff0000"})).unwrap(),
            "addon.color.fillToStroke" => {
                s.execute("paint.setFill", &json!({"ids": [b], "color": "#ff0000"})).unwrap();
                s.execute(cmd, &json!({"ids": [b]})).unwrap()
            }
            _ => s.execute(cmd, &json!({"ids": [b], "index": top_row(&s, lines[1].id, false), "color": "#ff0000"})).unwrap(),
        };
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| l.0 == "#ff0000" && (l.1 - weight).abs() < 1e-9), "{cmd}: {looks:?}");
    });
}

/// A zero stroke weight set together with a stroke color on an axis line is kept for every axis line as set, also when
/// a stroke is painted on another axis line before the graph is drawn again.
#[test]
fn a_zero_weight_set_with_a_stroke_color_on_an_axis_line_is_kept() {
    every_case([false, true], |repaint| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "none": true})).unwrap();
        let at = top_row(&s, lines[1].id, false);
        s.execute("appearance.setItem", &json!({"ids": [lines[1].id.0], "index": at, "color": "#ff0000", "weight": 0})).unwrap();
        let color = if *repaint {
            s.execute("paint.setStroke", &json!({"ids": [lines[2].id.0], "color": "#0000ff"})).unwrap();
            "#0000ff"
        } else {
            "#ff0000"
        };
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| l.0 == color && l.1 == 0.0), "repaint {repaint}: {looks:?}");
    });
}

/// A reorder that leaves an axis line's rows as they were keeps what its kind keeps from another line and is no edit.
#[test]
fn a_reorder_that_changes_nothing_on_an_axis_line_keeps_the_stroke_its_kind_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let lines = line_nodes(&s, id);
    s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
    assert_eq!(line_look(&line_nodes(&s, id)[1]).0, "#000000", "the other lines show black until the graph is drawn again");
    let stroke = top_row(&s, lines[1].id, false);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("appearance.moveItem", &json!({"ids": [lines[1].id.0], "from": stroke, "to": stroke})).unwrap();
    s.execute("appearance.moveItem", &json!({"ids": [lines[1].id.0], "indices": [stroke], "to": stroke})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "nothing changed");
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks.iter().all(|l| l.0 == "#ff0000"), "{looks:?}");
}

/// An Eyedropper pick that applies only the object's transparency to an axis line keeps the stroke its kind keeps, though
/// the line it is applied to still shows the default stroke.
#[test]
fn a_transparency_only_pick_on_an_axis_line_keeps_the_stroke_its_kind_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("transparency.set", &json!({"ids": [rect.0], "opacity": 50})).unwrap();
    let lines = line_nodes(&s, id);
    s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
    assert_eq!(line_look(&line_nodes(&s, id)[1]).0, "#000000", "the other lines show black until the graph is drawn again");
    let fill = json!({"color": false, "transparency": false, "overprint": false});
    let stroke = json!({"color": false, "transparency": false, "overprint": false, "weight": false, "cap": false, "join": false, "miter": false, "dash": false});
    let apply = json!({"appearance": {"transparency": true, "fill": fill, "stroke": stroke}, "character": false, "paragraph": false});
    s.execute("appearance.copyFrom", &json!({"source": rect.0, "ids": [lines[1].id.0], "apply": apply})).unwrap();
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks.iter().all(|l| l.0 == "#ff0000" && (l.2 - 0.5).abs() < 1e-6), "{looks:?}");
}

/// The Eyedropper Options that apply every fill and stroke attribute but not the object's transparency.
fn every_row_attribute_without_transparency() -> serde_json::Value {
    let fill = json!({"color": true, "transparency": true, "overprint": true});
    let stroke =
        json!({"color": true, "transparency": true, "overprint": true, "weight": true, "cap": true, "join": true, "miter": true, "dash": true});
    json!({"appearance": {"transparency": false, "fill": fill, "stroke": stroke}, "character": false, "paragraph": false})
}

/// An Eyedropper pick of every fill and stroke attribute without the object's transparency, with or without `append`,
/// keeps the object opacity the axis lines keep from another line, though the line it is applied to still shows 100%.
#[test]
fn a_pick_without_transparency_on_an_axis_line_keeps_the_opacity_its_kind_keeps() {
    for append in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
        s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#ff0000"})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [rect.0], "none": true})).unwrap();
        let lines = line_nodes(&s, id);
        s.execute("transparency.set", &json!({"ids": [lines[0].id.0], "opacity": 40})).unwrap();
        assert_eq!(line_look(&line_nodes(&s, id)[1]).2, 1.0, "append {append}: the other lines show 100% until the graph is drawn again");
        let pick = json!({"source": rect.0, "ids": [lines[1].id.0], "apply": every_row_attribute_without_transparency(), "append": append});
        s.execute("appearance.copyFrom", &pick).unwrap();
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| l.0 == "#ff0000" && (l.2 - 0.4).abs() < 1e-6), "append {append}: {looks:?}");
    }
}

/// A graphic style added on top of a graph label's or axis line's appearance keeps the object opacity its kind keeps
/// from another label or line, though the one it is added to still shows 100%.
#[test]
fn a_graphic_style_added_to_a_label_or_axis_line_keeps_the_opacity_its_kind_keeps() {
    for label in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
        s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#ff0000"})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [rect.0], "none": true})).unwrap();
        s.execute("graphicStyle.new", &json!({"id": rect.0, "name": "Red"})).unwrap();
        let (first, second) = if label {
            (text_id(&s, id, "Axes", "Q1"), text_id(&s, id, "Axes", "Q2"))
        } else {
            let lines = line_nodes(&s, id);
            (lines[0].id, lines[1].id)
        };
        s.execute("transparency.set", &json!({"ids": [first.0], "opacity": 40})).unwrap();
        assert_eq!(s.doc().unwrap().doc.node(second).unwrap().opacity, 1.0, "label {label}: 100% until the graph is drawn again");
        s.execute("graphicStyle.apply", &json!({"name": "Red", "ids": [second.0], "add": true})).unwrap();
        if label {
            let looks = redrawn_label_rows(&mut s, id);
            for q in ["Q1", "Q2", "Q3"] {
                let (rows, opacity) = rows_of(&looks, q);
                assert!((opacity - 0.4).abs() < 1e-6 && rows.iter().any(|r| !r.0 && r.1 == "#ff0000"), "{q}: {rows:?} {opacity}");
            }
        } else {
            let looks = redrawn_line_looks(&mut s, id);
            assert!(looks.iter().all(|l| l.0 == "#ff0000" && (l.2 - 0.4).abs() < 1e-6), "{looks:?}");
        }
    }
}

/// A fill-only graphic style added on top of a graph label or axis line, or a fill-only source appended to one with the
/// Eyedropper, keeps the stroke its kind keeps from another label or line, though the one it is added to still shows its
/// own.
#[test]
fn a_fill_added_to_a_label_or_axis_line_keeps_the_stroke_its_kind_keeps() {
    let cases = [(false, false), (false, true), (true, false), (true, true)];
    every_case(cases, |&(label, append)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let (rect, _) = blue_style(&mut s, "Blue", false);
        let (first, second) = if label {
            (text_id(&s, id, "Axes", "Q1"), text_id(&s, id, "Axes", "Q2"))
        } else {
            let lines = line_nodes(&s, id);
            (lines[0].id, lines[1].id)
        };
        if label {
            s.execute("appearance.addStroke", &json!({"ids": [first.0]})).unwrap();
            s.execute("appearance.setItem", &json!({"ids": [first.0], "index": top_row(&s, first, false), "color": "#ff0000"})).unwrap();
        } else {
            s.execute("paint.setStroke", &json!({"ids": [first.0], "color": "#ff0000"})).unwrap();
        }
        if append {
            let pick = json!({"source": rect.0, "ids": [second.0], "apply": every_row_attribute_without_transparency(), "append": true});
            s.execute("appearance.copyFrom", &pick).unwrap();
        } else {
            s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [second.0], "add": true})).unwrap();
        }
        if label {
            let looks = redrawn_label_rows(&mut s, id);
            for q in ["Q1", "Q2", "Q3"] {
                let (rows, _) = rows_of(&looks, q);
                let shows = |fill: bool, hex: &str| rows.iter().any(|r| r.0 == fill && r.1 == hex);
                assert!(shows(false, "#ff0000") && shows(true, "#0000ff"), "{q}: {rows:?}");
            }
        } else {
            let looks = redrawn_line_looks(&mut s, id);
            assert!(looks.iter().all(|l| l.0 == "#ff0000"), "{looks:?}");
        }
    });
}

/// A stroke a label's kind keeps below the characters stays below them when a fill-only graphic style is added on top of
/// another label of the kind, or a fill-only source appended to it, and the new fill paints above the characters.
#[test]
fn a_fill_added_to_a_label_keeps_the_kept_stroke_below_the_characters() {
    every_case([false, true], |&append| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let (rect, _) = blue_style(&mut s, "Blue", false);
        let (q1, q2) = (text_id(&s, id, "Axes", "Q1"), text_id(&s, id, "Axes", "Q2"));
        s.execute("appearance.addStroke", &json!({"ids": [q1.0]})).unwrap();
        s.execute("appearance.moveItem", &json!({"ids": [q1.0], "from": "contents", "to": 1})).unwrap();
        if append {
            let pick = json!({"source": rect.0, "ids": [q2.0], "apply": every_row_attribute_without_transparency(), "append": true});
            s.execute("appearance.copyFrom", &pick).unwrap();
        } else {
            s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [q2.0], "add": true})).unwrap();
        }
        redrawn_label_rows(&mut s, id);
        let look = |ap: &vectorcraft_doc::Appearance| (ap.contents_at(), ap.items.iter().map(|it| it.is_fill()).collect::<Vec<_>>());
        for q in ["Q1", "Q2", "Q3"] {
            assert_eq!(look(&label_node(&s, id, "Axes", q).appearance), (1, vec![false, true]), "append {append}: {q}");
        }
    });
}

/// A Contents (Characters) row move that leaves a graph label as it was keeps the rows its kind keeps from another label
/// and is no edit.
#[test]
fn a_contents_move_that_changes_nothing_on_a_label_keeps_the_rows_its_kind_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let (q1, q2) = (text_id(&s, id, "Axes", "Q1"), text_id(&s, id, "Axes", "Q2"));
    s.execute("appearance.addStroke", &json!({"ids": [q1.0]})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [q1.0], "index": top_row(&s, q1, false), "color": "#ff0000"})).unwrap();
    let at = s.doc().unwrap().doc.node(q2).unwrap().appearance.contents_at();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("appearance.moveItem", &json!({"ids": [q2.0], "from": "contents", "to": at})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "nothing changed");
    let looks = redrawn_label_rows(&mut s, id);
    for q in ["Q1", "Q2", "Q3"] {
        let (rows, _) = rows_of(&looks, q);
        assert!(rows.iter().any(|r| !r.0 && r.1 == "#ff0000"), "{q}: {rows:?}");
    }
}

/// An Eyedropper pick with `append` and without the object's transparency on a graph label keeps the object opacity its
/// kind keeps from another label, though the label it is applied to still shows 100%.
#[test]
fn an_appended_pick_without_transparency_on_a_label_keeps_the_opacity_its_kind_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#ff0000"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [rect.0], "none": true})).unwrap();
    s.execute("transparency.set", &json!({"ids": [text_id(&s, id, "Axes", "Q1").0], "opacity": 40})).unwrap();
    let q2 = text_id(&s, id, "Axes", "Q2");
    assert_eq!(s.doc().unwrap().doc.node(q2).unwrap().opacity, 1.0, "Q2 shows 100% until the graph is drawn again");
    let pick = json!({"source": rect.0, "ids": [q2.0], "apply": every_row_attribute_without_transparency(), "append": true});
    s.execute("appearance.copyFrom", &pick).unwrap();
    let looks = redrawn_label_rows(&mut s, id);
    for q in ["Q1", "Q2", "Q3"] {
        let (rows, opacity) = rows_of(&looks, q);
        assert!((opacity - 0.4).abs() < 1e-6, "{q}: {opacity}");
        assert!(rows.iter().any(|r| !r.0 && r.1 == "#ff0000"), "{q}: {rows:?}");
    }
}

/// An edit of some attributes on an axis line that still shows the default stroke keeps what it changes, and the stroke
/// its kind keeps from another line stays: a cap is not kept, so it changes nothing the kind keeps.
#[test]
fn an_edit_of_some_attributes_on_an_axis_line_keeps_the_rest_its_kind_keeps() {
    let cases = [
        ("stroke.set", json!({"cap": "round"}), ""),
        ("paint.setFill", json!({"color": "#0000ff"}), "#0000ff"),
        ("appearance.setItem", json!({"index": 0, "color": "#00ff00"}), "#00ff00"),
    ];
    for (cmd, params, fill) in cases {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
        assert_eq!(line_look(&line_nodes(&s, id)[1]).0, "#000000", "{cmd}: the other lines show black until the graph is drawn again");
        let mut p = params.clone();
        p["ids"] = json!([lines[1].id.0]);
        s.execute(cmd, &p).unwrap();
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| *l == ("#ff0000".into(), 0.5, 1.0)), "{cmd}: {looks:?}");
        let fills: Vec<String> =
            line_nodes(&s, id).iter().map(|n| n.appearance.fill_paint().color().map(|c| c.to_hex()).unwrap_or_default()).collect();
        assert!(fills.iter().all(|f| f == fill), "{cmd}: {fills:?}");
    }
}

/// An opacity and a stroke weight given to a radar graph's ring are kept for its rings, and its spokes keep their own.
#[test]
fn transparency_and_stroke_weight_on_a_radar_ring_are_kept_for_its_rings() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "radar");
    let ring = line_nodes(&s, id)[3].id;
    s.execute("transparency.set", &json!({"ids": [ring.0], "opacity": 40})).unwrap();
    s.execute("stroke.set", &json!({"ids": [ring.0], "weight": 1})).unwrap();
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks[..3].iter().all(|l| *l == ("#000000".into(), 0.5, 1.0)), "{looks:?}");
    assert!(looks[3..].iter().all(|l| l.0 == "#999999" && l.1 == 1.0 && (l.2 - 0.4).abs() < 1e-6), "{looks:?}");
}

/// A placed gradient on an axis line's stroke is kept for every axis line without its placement.
#[test]
fn a_gradient_on_an_axis_line_is_kept_without_its_placement() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let line = line_nodes(&s, id)[0].id;
    s.execute("paint.editGradient", &json!({"ids": [line.0], "kind": "linear", "stroke": true})).unwrap();
    let placement = json!({"ids": [line.0], "stroke": true, "start": [100, 100], "end": [200, 100], "aspect": 50});
    s.execute("paint.setGradientGeom", &placement).unwrap();
    let shown = line_nodes(&s, id)[0].appearance.stroke_paint().clone();
    assert!(matches!(&shown, Paint::Gradient(g) if g.geom.is_some()), "the line's gradient is placed: {shown:?}");
    let kept = spec(&s, id).label_styles.axis_rows.unwrap().paint.stroke;
    assert!(matches!(&kept, Some(Paint::Gradient(g)) if g.geom.is_none() && g.freeform.is_none()), "{kept:?}");
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    assert!(line_nodes(&s, id).iter().all(|n| matches!(n.appearance.stroke_paint(), Paint::Gradient(_))));
}

/// Default Fill and Stroke and Apply Fill to Stroke on an axis line are kept for every axis line.
#[test]
fn default_fill_and_stroke_and_fill_to_stroke_on_an_axis_line_are_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let line = line_nodes(&s, id)[0].id;
    s.execute("paint.setStroke", &json!({"ids": [line.0], "color": "#ff0000"})).unwrap();
    s.execute("paint.default", &json!({"ids": [line.0]})).unwrap();
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| *l == ("#000000".into(), 1.0, 1.0)), "the 1 pt black default stroke");
    let fills: Vec<String> = line_nodes(&s, id).iter().map(|n| n.appearance.fill_paint().color().map(|c| c.to_hex()).unwrap_or_default()).collect();
    assert!(fills.iter().all(|f| f == "#ffffff"), "{fills:?}");
    let line = line_nodes(&s, id)[1].id;
    s.execute("paint.setFill", &json!({"ids": [line.0], "color": "#0000ff"})).unwrap();
    s.execute("addon.color.fillToStroke", &json!({"ids": [line.0]})).unwrap();
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| l.0 == "#0000ff"), "the blue fill on the stroke");
}

/// The Appearance commands and `graphicStyle.apply` given a graph's Axes group or Legend group edit the lines and labels
/// directly in it, as given a series group they edit its paths: every line and label of each kind keeps what they set
/// when the graph is drawn again, and the groups keep no rows of their own.
#[test]
fn appearance_commands_given_the_axes_or_legend_group_edit_its_lines_and_labels() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let part = |s: &Session, name: &str| group(s.doc().unwrap().doc.node(id).unwrap(), name).clone();
    s.execute("appearance.clear", &json!({"ids": [part(&s, "Axes").id.0]})).unwrap();
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| l.0.is_empty()), "the cleared lines have no stroke");
    let (_, blue) = blue_style(&mut s, "Blue", false);
    for name in ["Axes", "Legend"] {
        s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [part(&s, name).id.0]})).unwrap();
    }
    let kept = spec(&s, id).label_styles;
    let fill = |rows: Option<vectorcraft_doc::LabelRows>| rows.and_then(|r| r.paint.fill);
    assert_eq!(fill(kept.axis_rows), Some(blue.clone()), "axis lines");
    assert_eq!(fill(kept.values_rows), Some(blue.clone()), "value labels");
    assert_eq!(fill(kept.legend_rows), Some(blue.clone()), "legend labels");
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    for name in ["Axes", "Legend"] {
        let g = part(&s, name);
        assert!(g.appearance.items.is_empty(), "{name} keeps no rows of its own");
        let members: Vec<Paint> = g.children().unwrap().iter().map(|c| c.appearance.fill_paint()).collect();
        assert!(!members.is_empty() && members.iter().all(|p| *p == blue), "{name}: {members:?}");
    }
}

/// A row index given the Axes group, whose lines and labels have different rows, is refused unless it is `aligned`, a row
/// of the backmost member's stack: then the members that share that row change, the labels keep theirs, and the graph
/// keeps the change when it is drawn again.
#[test]
fn a_row_index_given_the_axes_group_is_a_row_of_its_backmost_member_with_aligned() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let axes = group(s.doc().unwrap().doc.node(id).unwrap(), "Axes").clone();
    let back = axes.children().unwrap()[0].id;
    assert!(line_nodes(&s, id).iter().any(|l| l.id == back), "the backmost member is a line");
    let stroke = top_row(&s, back, false);
    for (cmd, p) in [
        ("appearance.setItem", json!({"ids": [axes.id.0], "index": stroke, "color": "#ff0000"})),
        ("appearance.removeItem", json!({"ids": [axes.id.0], "index": stroke})),
        ("appearance.duplicateItem", json!({"ids": [axes.id.0], "index": stroke})),
        ("appearance.moveItem", json!({"ids": [axes.id.0], "from": stroke, "to": 0})),
        ("transparency.set", json!({"ids": [axes.id.0], "item": stroke, "opacity": 40})),
    ] {
        let err = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(err.contains("aligned"), "{cmd}: {err}");
    }
    let labels = |s: &Session| -> Vec<vectorcraft_doc::Appearance> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        group(&n, "Axes").children().unwrap().iter().filter(|c| matches!(c.kind, NodeKind::Text(_))).map(|c| (*c.appearance).clone()).collect()
    };
    let was = labels(&s);
    s.execute("appearance.setItem", &json!({"ids": [axes.id.0], "index": stroke, "color": "#ff0000", "aligned": true})).unwrap();
    s.execute("transparency.set", &json!({"ids": [axes.id.0], "item": stroke, "opacity": 40, "aligned": true})).unwrap();
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks.iter().all(|l| l.0 == "#ff0000"), "{looks:?}");
    let rows = spec(&s, id).label_styles.axis_rows.unwrap();
    assert_eq!(rows.paint.stroke.and_then(|p| p.color()).map(|c| c.to_hex()).as_deref(), Some("#ff0000"));
    let now = labels(&s);
    assert!(!now.is_empty() && now.iter().all(|a| *a == was[0]), "the labels keep their rows");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let l = &group(&n, "Axes").children().unwrap()[0];
    assert!((l.appearance.items[top_row(&s, l.id, false)].opacity() - 0.4).abs() < 1e-6, "the stroke row's opacity is kept");
}

/// An aligned row edit given the Axes group leaves alone what a kind of label keeps from a label without that row: the
/// category labels keep the orange fill row of one of them through an edit of the lines' stroke rows.
#[test]
fn an_aligned_row_edit_given_the_axes_group_keeps_the_rows_of_labels_it_did_not_reach() {
    let cases = [("appearance.setItem", json!({"color": "#0000ff"})), ("appearance.duplicateItem", json!({})), ("appearance.removeItem", json!({}))];
    every_case(cases, |(cmd, params)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let c = text_id(&s, id, "Axes", "Q2");
        s.execute("appearance.addFill", &json!({"ids": [c.0]})).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [c.0], "index": 0, "color": "#ff8000"})).unwrap();
        let kept = spec(&s, id).label_styles.categories_rows;
        assert!(kept.is_some(), "the category labels keep the orange row");
        let axes = group(s.doc().unwrap().doc.node(id).unwrap(), "Axes").clone();
        let back = axes.children().unwrap()[0].id;
        let mut p = params.clone();
        p["ids"] = json!([axes.id.0]);
        p["index"] = json!(top_row(&s, back, false));
        p["aligned"] = json!(true);
        s.execute(cmd, &p).unwrap();
        assert_eq!(spec(&s, id).label_styles.categories_rows, kept, "{cmd}");
    });
}

/// An edit of a hidden row of an axis line leaves what its kind keeps from another line: the line's visible rows did not
/// change.
#[test]
fn an_edit_of_a_hidden_row_of_an_axis_line_keeps_what_its_kind_keeps() {
    let cases = [
        ("appearance.setItem", json!({"color": "#0000ff"})),
        ("appearance.duplicateItem", json!({})),
        ("appearance.removeItem", json!({})),
        ("transparency.set", json!({"opacity": 40})),
        ("paint.setStroke", json!({"color": "#0000ff"})),
    ];
    every_case(cases, |(cmd, params)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        let line = lines[1].id;
        s.execute("appearance.addStroke", &json!({"ids": [line.0]})).unwrap();
        let hidden = top_row(&s, line, false);
        s.execute("appearance.setItem", &json!({"ids": [line.0], "index": hidden, "visible": false})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
        let kept = spec(&s, id).label_styles.axis_rows;
        let mut p = params.clone();
        p["ids"] = json!([line.0]);
        p[if cmd.starts_with("appearance.") { "index" } else { "item" }] = json!(hidden);
        s.execute(cmd, &p).unwrap();
        assert_eq!(spec(&s, id).label_styles.axis_rows, kept, "{cmd}");
    });
}

/// An Eyedropper pick of only the object's opacity, or of only stroke options the graph does not keep, on an axis line
/// keeps the stroke its kind keeps from another line, though the line it is given still shows the default stroke.
#[test]
fn an_eyedropper_pick_of_only_the_opacity_keeps_the_stroke_its_kind_keeps() {
    let fill = json!({"color": false, "transparency": false, "overprint": false});
    let stroke = |dash: bool| json!({"color": false, "transparency": false, "overprint": false, "weight": false, "cap": false, "join": false, "miter": false, "dash": dash});
    let cases = [
        (json!({"appearance": {"transparency": true, "fill": fill, "stroke": stroke(false)}}), 0.5),
        (json!({"appearance": {"transparency": false, "fill": fill, "stroke": stroke(true)}}), 1.0),
    ];
    every_case(cases, |(pick, opacity)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
        s.execute("object.setProps", &json!({"ids": [rect.0], "opacity": 50})).unwrap();
        s.execute("stroke.set", &json!({"ids": [rect.0], "dash": [4, 2]})).unwrap();
        let lines = line_nodes(&s, id);
        s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
        s.execute("appearance.copyFrom", &json!({"source": rect.0, "ids": [lines[1].id.0], "pickUp": pick})).unwrap();
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| l.0 == "#ff0000" && (l.2 - *opacity).abs() < 1e-6), "{pick}: {looks:?}");
    });
}

/// Default Fill and Stroke on a label or an axis line that already shows the default paint is kept for its kind, in
/// place of the paint the kind keeps from another label or line.
#[test]
fn default_fill_and_stroke_on_a_label_or_line_that_shows_it_is_kept_for_its_kind() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("paint.setFill", &json!({"ids": [text_id(&s, id, "Axes", "4").0], "color": "#ff0000"})).unwrap();
    let hex = |st: &CharStyle| st.fill.color().map(|c| c.to_hex());
    assert_eq!(hex(&label_run_style(&s, id, "Axes", "0")).as_deref(), Some("#000000"), "the other labels show black until the graph is drawn again");
    s.execute("paint.default", &json!({"ids": [text_id(&s, id, "Axes", "0").0]})).unwrap();
    label_looks(&mut s, id);
    let st = label_run_style(&s, id, "Axes", "4");
    assert_eq!((hex(&st).as_deref(), st.stroke.is_none()), (Some("#000000"), true));
    let lines = line_nodes(&s, id);
    s.execute("paint.default", &json!({"ids": [lines[0].id.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [lines[1].id.0], "color": "#ff0000"})).unwrap();
    s.execute("paint.default", &json!({"ids": [lines[0].id.0]})).unwrap();
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| *l == ("#000000".into(), 1.0, 1.0)), "the 1 pt black default stroke");
}

/// Default Fill and Stroke on a label of a kind that keeps no style, which already shows the default paint, is no edit.
#[test]
fn default_fill_and_stroke_on_a_label_that_shows_it_is_no_edit() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("paint.default", &json!({"ids": [text_id(&s, id, "Axes", "4").0]})).unwrap();
    assert_eq!(spec(&s, id).label_styles.values, None);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "nothing changed");
}

/// Redefine Graphic Style on an Axes group linked to the style with rows of its own (as a file can hold it) restyles and
/// links the lines and labels in it, and clears and unlinks the group; Break Link given the group unlinks them. A style
/// applied through the group links its lines and labels, and the graph keeps their look, not the link, when it is drawn
/// again.
#[test]
fn graphic_style_links_given_the_axes_group() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let axes = group(s.doc().unwrap().doc.node(id).unwrap(), "Axes").id;
    let (rect, _) = blue_style(&mut s, "Blue", false);
    let style = s.doc().unwrap().doc.graphic_style("Blue").unwrap().clone();
    s.edit("Link", |d, _| {
        let g = d.node_mut(axes).unwrap();
        g.appearance = style.appearance.clone().into();
        g.extras_mut().graphic_style = Some(style.id);
        Ok(())
    })
    .unwrap();
    s.execute("appearance.setItem", &json!({"ids": [rect.0], "index": top_row(&s, rect, true), "color": "#00ff00", "opacity": 100})).unwrap();
    let green = s.doc().unwrap().doc.node(rect).unwrap().appearance.fill_paint();
    s.execute("graphicStyle.redefine", &json!({"name": "Blue", "id": rect.0})).unwrap();
    let axes_now = |s: &Session| group(s.doc().unwrap().doc.node(id).unwrap(), "Axes").clone();
    let members = |s: &Session| -> Vec<(Paint, Option<u32>)> {
        axes_now(s).children().unwrap().iter().map(|c| (c.appearance.fill_paint(), c.extras().graphic_style)).collect()
    };
    let g = axes_now(&s);
    assert!(g.appearance.items.is_empty() && g.extras().graphic_style.is_none(), "the group is cleared and unlinked");
    assert!(members(&s).iter().all(|m| *m == (green.clone(), Some(style.id))), "{:?}", members(&s));
    s.execute("graphicStyle.breakLink", &json!({"ids": [axes_now(&s).id.0]})).unwrap();
    assert!(members(&s).iter().all(|m| m.1.is_none()), "unlinked");
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [axes_now(&s).id.0]})).unwrap();
    assert!(members(&s).iter().all(|m| m.1 == Some(style.id)), "linked");
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    assert!(members(&s).iter().all(|m| *m == (green.clone(), None)), "{:?}", members(&s));
}

/// The Appearance commands given a radar graph's Axes group reach its spokes and rings, which keep the edit apart.
#[test]
fn a_style_given_a_radar_graphs_axes_group_is_kept_for_spokes_and_rings() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "radar");
    let axes = group(s.doc().unwrap().doc.node(id).unwrap(), "Axes").id;
    let (_, blue) = blue_style(&mut s, "Blue", false);
    s.execute("graphicStyle.apply", &json!({"name": "Blue", "ids": [axes.0]})).unwrap();
    let kept = spec(&s, id).label_styles;
    let fill = |rows: Option<vectorcraft_doc::LabelRows>| rows.and_then(|r| r.paint.fill);
    assert_eq!(fill(kept.axis_rows), Some(blue.clone()), "spokes");
    assert_eq!(fill(kept.ring_rows), Some(blue.clone()), "rings");
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks.iter().all(|l| l.0.is_empty()), "the style has no stroke: {looks:?}");
}

/// Default Fill and Stroke on an axis line that already shows the default paint is kept for every axis line, in place of
/// the stroke the kind keeps from another line, and leaves the object opacity the kind keeps from another line.
#[test]
fn default_fill_and_stroke_on_an_axis_line_that_shows_it_is_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let lines = line_nodes(&s, id);
    s.execute("paint.default", &json!({"ids": [lines[0].id.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [lines[1].id.0], "color": "#ff0000"})).unwrap();
    s.execute("object.setProps", &json!({"ids": [lines[1].id.0], "opacity": 50})).unwrap();
    s.execute("paint.default", &json!({"ids": [lines[0].id.0]})).unwrap();
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks.iter().all(|l| (l.0.as_str(), l.1) == ("#000000", 1.0)), "the 1 pt black default stroke: {looks:?}");
    assert!(looks.iter().all(|l| (l.2 - 0.5).abs() < 1e-6), "the kind keeps its 50% opacity: {looks:?}");
}

/// A stroke paint written to an axis line that already shows it is kept for every axis line, in place of the stroke the
/// kind keeps from another line.
#[test]
fn a_stroke_written_to_an_axis_line_that_shows_it_is_kept() {
    let cases = [("paint.setStroke", json!({"color": "#000000"})), ("addon.color.fillToStroke", json!({}))];
    every_case(cases, |(cmd, params)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        s.execute("paint.setFill", &json!({"ids": [lines[1].id.0], "color": "#000000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
        assert_eq!(line_look(&line_nodes(&s, id)[1]).0, "#000000", "{cmd}: the other lines show black until the graph is drawn again");
        let mut p = params.clone();
        p["ids"] = json!([lines[1].id.0]);
        s.execute(cmd, &p).unwrap();
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| l.0 == "#000000"), "{cmd}: {looks:?}");
    });
}

/// A stroke added to an axis line whose stroke shows the paint the new one takes is kept for every axis line, in place of
/// the stroke the kind keeps from another line.
#[test]
fn a_stroke_added_to_an_axis_line_is_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let lines = line_nodes(&s, id);
    s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
    assert_eq!(line_look(&line_nodes(&s, id)[1]).0, "#000000", "the other lines show black until the graph is drawn again");
    s.execute("appearance.addStroke", &json!({"ids": [lines[1].id.0]})).unwrap();
    let looks = redrawn_line_looks(&mut s, id);
    assert!(looks.iter().all(|l| (l.0.as_str(), l.1) == ("#000000", 1.0)), "the added 1 pt black stroke: {looks:?}");
}

/// An Eyedropper pick of the stroke color, weight or transparency alone onto an axis line that already shows what it
/// writes is kept for every axis line, in place of what the kind keeps from another line.
#[test]
fn an_eyedropper_pick_an_axis_line_already_shows_is_kept() {
    let stroke_row = |n: &vectorcraft_doc::Node| n.appearance.items.iter().rposition(|it| !it.is_fill()).unwrap();
    let cases = [("color", json!({"color": "#ff0000"})), ("weight", json!({"weight": 2})), ("transparency", json!({"opacity": 40}))];
    every_case(cases, |(attr, edit)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap());
        s.execute("paint.setStroke", &json!({"ids": [rect.0], "color": "#000000"})).unwrap();
        s.execute("stroke.set", &json!({"ids": [rect.0], "weight": 0.5})).unwrap();
        let lines = line_nodes(&s, id);
        let mut p = edit.clone();
        p["ids"] = json!([lines[0].id.0]);
        match *attr {
            "color" => s.execute("paint.setStroke", &p),
            "weight" => s.execute("stroke.set", &p),
            _ => {
                p["index"] = json!(stroke_row(&lines[0]));
                s.execute("appearance.setItem", &p)
            }
        }
        .unwrap();
        let shown = line_nodes(&s, id)[1].clone();
        assert_eq!(line_look(&shown), ("#000000".into(), 0.5, 1.0), "{attr}: the other lines show the default until the graph is drawn again");
        let fill = json!({"color": false, "transparency": false, "overprint": false});
        let mut stroke = json!({"color": false, "transparency": false, "overprint": false, "weight": false, "cap": false, "join": false, "miter": false, "dash": false});
        stroke[*attr] = json!(true);
        let apply = json!({"appearance": {"transparency": false, "fill": fill, "stroke": stroke}, "character": false, "paragraph": false});
        s.execute("appearance.copyFrom", &json!({"source": rect.0, "ids": [lines[1].id.0], "apply": apply})).unwrap();
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| *l == ("#000000".into(), 0.5, 1.0)), "{attr}: {looks:?}");
        let opacities: Vec<f32> = line_nodes(&s, id).iter().map(|n| n.appearance.items[stroke_row(n)].opacity()).collect();
        assert!(opacities.iter().all(|o| *o == 1.0), "{attr}: {opacities:?}");
    });
}

/// A stroke color, weight or opacity set with `appearance.setItem`, or a weight set with `stroke.set`, on an axis line
/// that already shows it is kept for every axis line, in place of what the kind keeps from another line.
#[test]
fn a_row_attribute_set_on_an_axis_line_that_shows_it_is_kept() {
    let stroke_row = |n: &vectorcraft_doc::Node| n.appearance.items.iter().rposition(|it| !it.is_fill()).unwrap();
    let cases = [
        ("appearance.setItem", json!({"color": "#ff0000"}), json!({"color": "#000000"})),
        ("appearance.setItem", json!({"weight": 2}), json!({"weight": 0.5})),
        ("appearance.setItem", json!({"opacity": 40}), json!({"opacity": 100})),
        ("stroke.set", json!({"weight": 2}), json!({"weight": 0.5})),
    ];
    every_case(cases, |(cmd, kept, shown)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        for (line, values) in [(&lines[0], kept), (&lines[1], shown)] {
            let mut p = values.clone();
            p["ids"] = json!([line.id.0]);
            if *cmd == "appearance.setItem" {
                p["index"] = json!(stroke_row(line));
            }
            s.execute(cmd, &p).unwrap();
        }
        let looks = redrawn_line_looks(&mut s, id);
        assert!(looks.iter().all(|l| *l == ("#000000".into(), 0.5, 1.0)), "{cmd} {shown}: {looks:?}");
        let opacities: Vec<f32> = line_nodes(&s, id).iter().map(|n| n.appearance.items[stroke_row(n)].opacity()).collect();
        assert!(opacities.iter().all(|o| *o == 1.0), "{cmd} {shown}: {opacities:?}");
    });
}

/// `appearance.setItem` of a row's opacity alone on an axis line keeps the blend mode the kind keeps from another line,
/// and of its blend mode alone the opacity; a null weight or opacity writes nothing and adds no undo step.
#[test]
fn a_row_attribute_set_on_an_axis_line_keeps_the_others_its_kind_keeps() {
    let stroke_row = |n: &vectorcraft_doc::Node| n.appearance.items.iter().rposition(|it| !it.is_fill()).unwrap();
    let row = |n: &vectorcraft_doc::Node| {
        let it = &n.appearance.items[stroke_row(n)];
        (it.opacity(), it.blend())
    };
    let cases = [
        (json!({"blend": "multiply"}), json!({"opacity": 100}), (1.0, BlendMode::Multiply)),
        (json!({"opacity": 40}), json!({"blend": "normal"}), (0.4, BlendMode::Normal)),
    ];
    every_case(cases, |(kept, shown, want)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let lines = line_nodes(&s, id);
        for (line, values) in [(&lines[0], kept), (&lines[1], shown)] {
            let mut p = values.clone();
            p["ids"] = json!([line.id.0]);
            p["index"] = json!(stroke_row(line));
            s.execute("appearance.setItem", &p).unwrap();
        }
        redrawn_line_looks(&mut s, id);
        let rows: Vec<(f32, BlendMode)> = line_nodes(&s, id).iter().map(row).collect();
        assert!(rows.iter().all(|r| *r == *want), "{shown}: {rows:?}");
    });
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let lines = line_nodes(&s, id);
    s.execute("paint.setStroke", &json!({"ids": [lines[0].id.0], "color": "#ff0000"})).unwrap();
    let steps = s.doc().unwrap().history.undo.len();
    for key in ["weight", "opacity"] {
        let mut p = json!({"ids": [lines[1].id.0], "index": stroke_row(&lines[1])});
        p[key] = serde_json::Value::Null;
        s.execute("appearance.setItem", &p).unwrap();
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "a null value adds no undo step");
    assert!(redrawn_line_looks(&mut s, id).iter().all(|l| l.0 == "#ff0000"));
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

/// On a pie with Legends in Wedges, a black fill given to the white legend label on the dark wedge is kept for every
/// legend label: the graph draws it black on the dark wedge again, also after a native save and open.
#[test]
fn a_black_fill_set_on_a_wedge_legend_label_stays_black_on_dark_wedges() {
    let cases = [("paint.setFill", json!({"color": "#000000"})), ("text.setStyle", json!({"fill": "#000000"})), ("edit.colors.invert", json!({}))];
    every_case(cases, |(cmd, params)| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "pie", ",a,b\nP1,1,2");
        s.execute("graph.setType", &json!({"id": id.0, "pieLegend": "wedges"})).unwrap();
        let legend_fill = |s: &Session, text: &str| {
            let NodeKind::Text(t) = label_node(s, id, "Legend", text).kind else { panic!("{text} is not type") };
            t.runs[0].style.fill.color().map(|c| c.to_hex()).unwrap_or_default()
        };
        assert_eq!(legend_fill(&s, "a"), "#ffffff", "white on the black wedge");
        let mut p = params.clone();
        p["ids"] = json!([text_id(&s, id, "Legend", "a").0]);
        s.execute(cmd, &p).unwrap();
        assert!(spec(&s, id).label_styles.legend_fill_set, "{cmd}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(!spec(&s, id).label_styles.legend_fill_set, "{cmd}: undone");
        assert_eq!(legend_fill(&s, "a"), "#ffffff", "{cmd}: white again after the undo");
        s.execute("edit.redo", &json!({})).unwrap();
        assert!(spec(&s, id).label_styles.legend_fill_set, "{cmd}: redone");
        s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2]]})).unwrap();
        assert_eq!((legend_fill(&s, "a"), legend_fill(&s, "b")), ("#000000".into(), "#000000".into()), "{cmd}: black on both wedges");
        let dir = vectorcraft_testkit::TempDir::new("graph-wedge-legend");
        let path = dir.join("pie.vectorcraft");
        s.execute("document.save", &json!({"path": path})).unwrap();
        s.execute("document.open", &json!({"path": path})).unwrap();
        assert!(spec(&s, id).label_styles.legend_fill_set, "{cmd}: kept through save and open");
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "rows": [[2, 1]]})).unwrap();
        assert_eq!(legend_fill(&s, "a"), "#000000", "{cmd}: still black after open");
    });
}

/// A size given to the legend labels of a pie with Legends in Wedges keeps the default black fill with it. A color mode
/// change leaves that fill black, so the legend label on the dark wedge is still drawn white. A legend fill that is not
/// black, in a file saved without the mark, is marked as set by the change.
#[test]
fn a_color_mode_change_keeps_the_default_wedge_legend_fill() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = make(&mut s, "pie", ",a,b\nP1,1,2");
    s.execute("graph.setType", &json!({"id": id.0, "pieLegend": "wedges"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 11})).unwrap();
    s.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let styles = spec(&s, id).label_styles;
    assert!(!styles.legend_fill_set && !styles.legend_fill_is_own(), "{styles:?}");
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2]]})).unwrap();
    let NodeKind::Text(t) = label_node(&s, id, "Legend", "a").kind else { panic!("a is not type") };
    assert_eq!(t.runs[0].style.fill.color().map(|c| c.to_hex()), Some("#ffffff".into()), "white on the black wedge");
    let red = Paint::solid(vectorcraft_color::Color::from_hex("#ff0000").unwrap());
    let styles = vectorcraft_doc::LabelStyles { legend: Some(vectorcraft_doc::CharStyle { fill: red, ..Default::default() }), ..Default::default() };
    assert!(!styles.legend_fill_set && styles.legend_fill_is_own(), "{styles:?}");
    s.edit("legacy", |d, _| {
        d.node_mut(id).unwrap().extras_mut().graph.as_deref_mut().unwrap().label_styles = styles;
        Ok(())
    })
    .unwrap();
    s.execute("file.documentColorMode", &json!({"mode": "rgb"})).unwrap();
    assert!(spec(&s, id).label_styles.legend_fill_set, "the converted red is still the labels' own");
}

/// The default black legend fill of a graph that keeps its legend beside the marks stays the default through a color
/// mode change, so turning on Legends in Wedges later still draws the labels on dark wedges white.
#[test]
fn a_color_mode_change_keeps_the_default_legend_fill_of_other_graphs() {
    // A pie made here keeps its legend beside the pies.
    for kind in ["column", "pie"] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, kind, ",a,b\nP1,1,2");
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("text.setStyle", &json!({"size": 11})).unwrap();
        let black = Paint::solid(vectorcraft_color::Color::BLACK);
        let before = spec(&s, id).label_styles;
        assert_eq!(before.legend.as_ref().map(|st| st.fill.clone()), Some(black.clone()), "{kind}: {before:?}");
        s.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
        let after = spec(&s, id).label_styles;
        assert_eq!(after.legend.as_ref().map(|st| st.fill.clone()), Some(black), "{kind}: {after:?}");
        assert!(!after.legend_fill_set && !after.legend_fill_is_own(), "{kind}: still the default: {after:?}");
    }
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

#[test]
fn csv_leading_byte_order_mark_is_ignored() {
    assert_eq!(super::parse_csv("\u{feff}1,2\n3,4"), super::parse_csv("1,2\n3,4"));
}

#[test]
fn csv_round_trip_keeps_blank_row_in_single_column_graph() {
    let mut g = GraphSpec::default();
    g.set_cells(vec![vec![Some(1.0)], vec![None], vec![Some(3.0)]]);
    let (_, c, r) = super::parse_csv(&g.to_csv());
    assert!(c.is_empty());
    assert_eq!(r, g.cells());
}

/// Each generated label's text, paragraph attributes and alignment.
fn label_paras(s: &Session, id: NodeId) -> Vec<(String, vectorcraft_doc::ParaStyle)> {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let mut out = vec![];
    for name in ["Axes", "Legend"] {
        for c in group(&n, name).children().unwrap() {
            if let NodeKind::Text(t) = &c.kind {
                out.push((t.plain_text(), t.para.clone()));
            }
        }
    }
    out
}

#[test]
fn paragraph_formats_are_kept_when_the_graph_is_drawn_again() {
    use vectorcraft_doc::{Composer, ParaDirection, text::Kashida};
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    let aligned: Vec<_> = label_paras(&s, id).into_iter().map(|(t, p)| (t, p.justify)).collect();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.setFormat", &json!({"composer": "singleLine", "kashida": "long", "direction": "rightToLeft", "wordSpacing": [70, 90, 120]}))
        .unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    let paras = label_paras(&s, id);
    assert!(!paras.is_empty());
    for (text, p) in &paras {
        assert_eq!(p.composer, Composer::SingleLine, "{text}");
        assert_eq!(p.kashida, Kashida::Long, "{text}");
        assert_eq!(p.direction, Some(ParaDirection::RightToLeft), "{text}");
        assert_eq!(p.justification.word_spacing, [70.0, 90.0, 120.0], "{text}");
    }
    // The graph still aligns each label to its place.
    assert_eq!(paras.iter().map(|(t, p)| (t.clone(), p.justify)).collect::<Vec<_>>(), aligned);
    // One kind of label: only that kind keeps it, also through save and open.
    let col = graph(&mut s, "column");
    let v = text_id(&s, col, "Axes", "4");
    s.execute("select.set", &json!({"ids": [v.0]})).unwrap();
    s.execute("text.setFormat", &json!({"composer": "singleLine"})).unwrap();
    let dir = vectorcraft_testkit::TempDir::new("graph-label-paras");
    let path = dir.join("labels.vectorcraft");
    s.execute("document.save", &json!({"path": path})).unwrap();
    s.execute("document.open", &json!({"path": path})).unwrap();
    s.execute("select.set", &json!({"ids": [col.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": col.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    let paras = label_paras(&s, col);
    let composer = |t: &str| paras.iter().find(|(x, _)| x == t).unwrap().1.composer;
    assert_eq!(composer("0"), Composer::SingleLine, "every value axis number");
    assert_eq!(composer("Q1"), Composer::default(), "category labels as they were");
    assert_eq!(composer("2024"), Composer::default(), "legend labels as they were");
    // Part of a label's text: nothing kept.
    let q = text_id(&s, col, "Axes", "Q2");
    s.execute("select.set", &json!({"ids": [q.0]})).unwrap();
    s.execute("text.setFormat", &json!({"kashida": "short", "start": 0, "end": 1})).unwrap();
    s.execute("graph.setData", &json!({"id": col.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    assert!(label_paras(&s, col).iter().all(|(_, p)| p.kashida == Kashida::default()));
    // Spacing that would be out of order in the kept attributes is refused, even when it is in order on the label.
    let v = text_id(&s, col, "Axes", "4");
    s.execute("select.set", &json!({"ids": [col.0]})).unwrap();
    s.execute("text.setFormat", &json!({"wordSpacing": [60, 70, 80]})).unwrap();
    s.execute("select.set", &json!({"ids": [v.0]})).unwrap();
    s.execute("text.setFormat", &json!({"wordSpacing": [80, 100, 133], "start": 0, "end": 1})).unwrap();
    assert!(s.execute("text.setFormat", &json!({"wordSpacing": {"min": 90}})).is_err());
    let kept = spec(&s, col).label_styles.values_para.unwrap();
    assert_eq!(kept.justification.word_spacing, [60.0, 70.0, 80.0]);
}

#[test]
fn a_legend_across_top_spaces_its_labels_by_their_kept_paragraph_settings() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let id = graph(&mut s, "column");
    s.execute("graph.setType", &json!({"legendAcrossTop": true})).unwrap();
    let l = text_id(&s, id, "Legend", "2024");
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    s.execute("text.setFormat", &json!({"leftIndent": 40})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let label = group(&n, "Legend").children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == "2024")).unwrap();
    let label = label.geometric_bounds().unwrap();
    let next = series_parts(&s, id, 1).last().unwrap().geometric_bounds().unwrap();
    assert!(next.x0 >= label.x1, "the next swatch starts after the indented label: {label:?} {next:?}");
}

#[test]
fn a_centered_label_with_kept_leading_stays_centered_when_the_graph_is_drawn_again() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",2024,2025\nQ|1,3,2\nQ2,5,4\nQ3,-1,6";
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "bar", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let middle = |s: &Session| {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let label = group(&n, "Axes").children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == "Q\n1")).unwrap();
        let b = label.geometric_bounds().unwrap();
        (b.y0 + b.y1) / 2.0
    };
    let before = middle(&s);
    let l = text_id(&s, id, "Axes", "Q\n1");
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    s.execute("text.setFormat", &json!({"autoLeading": 400, "spaceBefore": 6})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let after = middle(&s);
    assert!((after - before).abs() < 0.01, "the two-line category label keeps its middle: {before} {after}");
}

#[test]
fn a_centered_label_with_an_empty_middle_line_and_kept_leading_stays_centered() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",2024,2025\nQ||1,3,2\nQ2,5,4\nQ3,-1,6";
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "bar", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let middle = |s: &Session| {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let label =
            group(&n, "Axes").children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == "Q\n\n1")).unwrap();
        let b = label.geometric_bounds().unwrap();
        (b.y0 + b.y1) / 2.0
    };
    let before = middle(&s);
    let l = text_id(&s, id, "Axes", "Q\n\n1");
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    s.execute("text.setFormat", &json!({"autoLeading": 400})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let after = middle(&s);
    assert!((after - before).abs() < 0.01, "the three-line category label keeps its middle: {before} {after}");
}

#[test]
fn a_centered_label_with_a_line_break_in_its_text_is_set_as_with_a_bar() {
    // A quoted CSV cell keeps its line breaks, so they reach the label as well as the `|` it is written with.
    for (bar, quoted, text) in [("Q|1", "\"Q\n1\"", "Q\n1"), ("Q|||1", "\"Q\n||1\"", "Q\n\n\n1")] {
        let label_middle = |s: &Session, id: NodeId| {
            let n = s.doc().unwrap().doc.node(id).unwrap().clone();
            let label =
                group(&n, "Axes").children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == text)).unwrap();
            let b = label.geometric_bounds().unwrap();
            (b.y0 + b.y1) / 2.0
        };
        let drawn = |cat: &str, leading: bool| {
            let mut s = Session::new();
            s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
            let csv = format!(",2024,2025\n{cat},3,2\nQ2,5,4\nQ3,-1,6");
            let id = NodeId(
                s.execute("graph.create", &json!({"type": "bar", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                    .as_u64()
                    .unwrap(),
            );
            if leading {
                let l = text_id(&s, id, "Axes", text);
                s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
                s.execute("text.setFormat", &json!({"autoLeading": 400})).unwrap();
                s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
                s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
            }
            label_middle(&s, id)
        };
        for leading in [false, true] {
            let (by_bar, by_break) = (drawn(bar, leading), drawn(quoted, leading));
            assert!((by_bar - by_break).abs() < 0.01, "{text:?} sits where {bar:?} does (kept leading: {leading}): {by_bar} {by_break}");
        }
        assert!((drawn(quoted, true) - drawn(quoted, false)).abs() < 0.01, "{text:?} keeps its middle with kept leading");
    }
}

#[test]
fn a_legend_across_top_measures_its_labels_with_system_layout() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    s.execute("type.systemLayout", &json!({"on": true})).unwrap();
    // Each `i` at 2 pt advances a whole point with System Layout, far more than its own width.
    let name = "i".repeat(60);
    let csv = format!(",{name},2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6");
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("graph.setType", &json!({"legendAcrossTop": true})).unwrap();
    let l = text_id(&s, id, "Legend", &name);
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 2})).unwrap();
    s.execute("text.setFormat", &json!({"leftIndent": 1})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let label = group(&n, "Legend").children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == name)).unwrap();
    let label = label.geometric_bounds().unwrap();
    let next = series_parts(&s, id, 1).last().unwrap().geometric_bounds().unwrap();
    assert!(next.x0 >= label.x1, "the next swatch starts after the label set with System Layout: {label:?} {next:?}");
}

#[test]
fn a_legend_label_with_a_kept_negative_indent_stays_clear_of_its_swatch() {
    for across_top in [true, false] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        s.execute("graph.setType", &json!({"legendAcrossTop": across_top})).unwrap();
        let l = text_id(&s, id, "Legend", "2025");
        s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
        s.execute("text.setFormat", &json!({"leftIndent": -40})).unwrap();
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "csv": ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6"})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let bounds = |text: &str| {
            let label =
                group(&n, "Legend").children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == text)).unwrap();
            label.geometric_bounds().unwrap()
        };
        let (first, label) = (bounds("2024"), bounds("2025"));
        let swatch = series_parts(&s, id, 1).last().unwrap().geometric_bounds().unwrap();
        assert!(label.x0 >= swatch.x1, "the label starts after its swatch (across top: {across_top}): {swatch:?} {label:?}");
        if across_top {
            assert!(swatch.x0 >= first.x1, "the swatch starts after the label before it: {first:?} {swatch:?}");
        }
    }
}

#[test]
fn a_legend_makes_room_for_the_kept_leading_of_its_labels() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",20|24,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6";
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let l = text_id(&s, id, "Legend", "20\n24");
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    s.execute("text.setFormat", &json!({"autoLeading": 400})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let bounds = |text: &str| {
        let label = group(&n, "Legend").children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == text)).unwrap();
        label.geometric_bounds().unwrap()
    };
    let (first, next) = (bounds("20\n24"), bounds("2025"));
    assert!(next.y0 >= first.y1, "the next legend label starts below the first one: {first:?} {next:?}");
}

#[test]
fn a_legend_label_with_kept_negative_space_before_stays_below_the_entry_above_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",2024,20|25\nQ1,3,2\nQ2,5,4\nQ3,-1,6";
    let id = NodeId(
        s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let l = text_id(&s, id, "Legend", "20\n25");
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    s.execute("text.setFormat", &json!({"spaceBefore": -60})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let bounds = |text: &str| {
        let label = group(&n, "Legend").children().unwrap().iter().find(|c| matches!(&c.kind, NodeKind::Text(t) if t.plain_text() == text)).unwrap();
        label.geometric_bounds().unwrap()
    };
    let (first, next) = (bounds("2024"), bounds("20\n25"));
    assert!(next.y0 >= first.y1, "the second legend label starts below the first one: {first:?} {next:?}");
}

/// A graph command given a value of the wrong type refuses it, naming the key, and leaves the graph and the undo
/// history as they were; a null axis bound still goes back to the calculated value.
#[test]
fn graph_commands_refuse_params_of_the_wrong_type() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = graph(&mut s, "line");
    s.execute("graph.setType", &json!({"id": id.0, "axisMin": 0, "axisMax": 10})).unwrap();
    let spec = |s: &Session| s.doc().unwrap().doc.node(id).unwrap().extras().graph.as_deref().cloned().unwrap();
    let undo_len = |s: &Session| s.doc().unwrap().history.undo.len();
    let (before, steps) = (spec(&s), undo_len(&s));
    let refused = |s: &mut Session, cmd: &str, p: serde_json::Value, msg: &str| {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains(msg), "{cmd} {p}: {e}");
    };
    for (key, v, msg) in [
        ("axisMin", json!("10"), "`axisMin` must be a number or null, not \"10\""),
        ("axisMax", json!(true), "`axisMax` must be a number or null"),
        ("rightAxisMin", json!([1]), "`rightAxisMin` must be a number or null"),
        ("rightAxisMax", json!({}), "`rightAxisMax` must be a number or null"),
        ("columnWidth", json!("50"), "`columnWidth` must be a number, not \"50\""),
        ("clusterWidth", json!(false), "`clusterWidth` must be a number"),
        ("ticks", json!("5"), "`ticks` must be a number, not \"5\""),
        ("ticks", json!(-2), "`ticks` must be 0 or more"),
        ("rightTicks", json!("5"), "`rightTicks` must be a number"),
        ("tickMarks", json!("2"), "`tickMarks` must be a number"),
        ("rightTickMarks", json!(-1), "`rightTickMarks` must be 0 or more"),
        ("categoryTickMarks", json!(true), "`categoryTickMarks` must be a number"),
        ("legend", json!("false"), "`legend` must be true or false"),
        ("markPoints", json!(0), "`markPoints` must be true or false"),
        ("connectPoints", json!("no"), "`connectPoints` must be true or false"),
        ("edgeToEdge", json!(1), "`edgeToEdge` must be true or false"),
        ("separateScales", json!("true"), "`separateScales` must be true or false"),
        ("ticksBetweenLabels", json!(1), "`ticksBetweenLabels` must be true or false"),
        ("firstRowInFront", json!("yes"), "`firstRowInFront` must be true or false"),
        ("firstColumnInFront", json!(0), "`firstColumnInFront` must be true or false"),
        ("filledLines", json!("on"), "`filledLines` must be true or false"),
        ("lineWidth", json!("4"), "`lineWidth` must be a number, not \"4\""),
        ("dropShadow", json!(1), "`dropShadow` must be true or false"),
        ("legendAcrossTop", json!("x"), "`legendAcrossTop` must be true or false"),
        ("type", json!(5), "`type` must be a string"),
        ("valueAxis", json!(true), "`valueAxis` must be a string"),
        ("tickLength", json!(1), "`tickLength` must be a string"),
        ("rightTickLength", json!(1), "`rightTickLength` must be a string"),
        ("categoryTickLength", json!(1), "`categoryTickLength` must be a string"),
        ("prefix", json!(1), "`prefix` must be a string"),
        ("suffix", json!(true), "`suffix` must be a string"),
        ("rightPrefix", json!([]), "`rightPrefix` must be a string"),
        ("rightSuffix", json!(2), "`rightSuffix` must be a string"),
        ("pieLegend", json!(1), "`pieLegend` must be a string"),
        ("piePosition", json!(1), "`piePosition` must be a string"),
        ("pieSort", json!(1), "`pieSort` must be a string"),
    ] {
        refused(&mut s, "graph.setType", json!({"id": id.0, key: v}), msg);
    }
    for (key, v, msg) in [
        ("csv", json!(5), "`csv` must be a string"),
        ("series", json!("a"), "`series` must be a list, not \"a\""),
        ("categories", json!({}), "`categories` must be a list"),
        ("rows", json!("1,2"), "`rows` must be a list"),
        ("rows", json!([1, 2]), "`rows` must be lists of numbers or nulls, not 1"),
        ("rows", json!([[1, "2"]]), "`rows` must be lists of numbers or nulls"),
        ("transpose", json!("true"), "`transpose` must be true or false"),
        ("switchXY", json!(1), "`switchXY` must be true or false"),
    ] {
        refused(&mut s, "graph.setData", json!({"id": id.0, key: v}), msg);
    }
    assert_eq!(spec(&s), before);
    assert_eq!(undo_len(&s), steps);
    let count = |s: &Session| s.doc().unwrap().doc.node_count();
    let objects = count(&s);
    for (key, v, msg) in [
        ("type", json!(1), "`type` must be a string"),
        ("width", json!("300"), "`width` must be a number, not \"300\""),
        ("height", json!(true), "`height` must be a number"),
        ("csv", json!(1), "`csv` must be a string"),
        ("rows", json!([[true]]), "`rows` must be lists of numbers or nulls"),
    ] {
        let mut p = json!({"type": "column", "x": 0, "y": 0, "width": 100, "height": 100});
        p[key] = v;
        refused(&mut s, "graph.create", p, msg);
    }
    assert_eq!((count(&s), undo_len(&s)), (objects, steps));
    // A null is a key left out, except for an axis bound, where it goes back to the calculated value.
    s.execute("graph.setType", &json!({"id": id.0, "legend": null, "ticks": null, "prefix": null, "axisMin": null})).unwrap();
    assert_eq!(spec(&s).axis_min, None);
    assert_eq!(spec(&s).axis_max, Some(10.0));
    s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, null], [2, 3]], "series": null, "transpose": null})).unwrap();
    assert_eq!(spec(&s).cells(), vec![vec![Some(1.0), None], vec![Some(2.0), Some(3.0)]]);
}

/// A `.ai` in PostScript form whose graph's records read whole (version 7 specification, appendix A)
/// opens as a graph whose data can be edited: Graph Data redraws it on its plot. Saving it as `.ai`,
/// in either form, writes its records, and the file opens again with the graph's new data.
#[test]
fn a_graph_opened_from_ai_keeps_its_data_for_editing() {
    let graph = "%_Gs\n%_100 200 300 100 Gb\n%_5 0 0 14 0 1 0 0 6 44 Gy\n%_10 2 0 0 90 80 0 Gd\n\
        %_1 () Ga\n%_0 13 0 0 0 0 0 () GA\n%_2 () Ga\n%_0 14 0 0 0 0 0 () GA\n%_4 () Ga\n%_0 14 0 0 0 0 0 () GA\n\
        %_2 2 1 1 Gz\n%_() Gc (East) Gc (Q1) Gc 5 Gc GC\n\
        u\nu\n0.6 0 1 0 k\n110 100 m\n110 150 L\n130 150 L\n130 100 L\nf\nU\n%_2 1 0 0 Go\n\
        u\n0 0 0 1 K\n100 200 m\n100 100 L\n300 100 L\nS\nU\nU\n%_GS\n";
    let text = format!(
        "%!PS-Adobe-3.0\n%%Creator: a script\n%%BoundingBox: 0 0 400 400\n%AI5_FileFormat 3\n%AI3_Cropmarks: 0 0 400 400\n%%EndComments\n\
         %%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n%AI5_BeginLayer\n1 1 1 1 0 0 0 79 128 255 Lb\n(Graph) Ln\n0 A\n{graph}LB\n\
         %AI5_EndLayer--\n%%PageTrailer\n%%Trailer\n%%EOF\n"
    );
    let dir = vectorcraft_testkit::temp_dir("graph-ai-open");
    let path = dir.join("graph.ai");
    std::fs::write(&path, text).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let mut found = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if n.extras().graph.is_some() {
            found.push(n.id);
        }
    });
    assert_eq!(found.len(), 1);
    let id = found[0];
    let before = spec(&s, id);
    assert_eq!(
        (before.series.as_slice(), before.categories.as_slice(), before.rows.as_slice()),
        (&["East".to_string()][..], &["Q1".to_string()][..], &[vec![5.0]][..])
    );
    // The fill of the series' first mark.
    let series_fill = |s: &Session| {
        let mut fill = None;
        s.doc().unwrap().doc.walk(|n| {
            if n.extras().series_index == Some(0) && fill.is_none() {
                let first = n.children().and_then(|c| c.first().cloned());
                fill = first.and_then(|m| m.appearance.items.iter().find(|i| i.is_fill()).map(|i| i.paint().clone()));
            }
        });
        fill
    };
    let painted = series_fill(&s);
    assert!(painted.is_some());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",East\nQ1,8\nQ2,2"})).unwrap();
    let after = spec(&s, id);
    assert_eq!(after.rows, [[8.0], [2.0]]);
    assert_eq!(after.rect, before.rect, "the graph is redrawn on its plot");
    assert_eq!(series_fill(&s), painted, "the series keeps the file's paint");
    for (name, version) in [("v7.ai", Some("7")), ("pdf.ai", None)] {
        let mut p = json!({"path": dir.join(name), "format": "ai"});
        if let Some(v) = version {
            p["aiVersion"] = json!(v);
        }
        let r = s.execute("file.saveAs", &p).unwrap();
        let notes = r["warnings"].as_array().cloned().unwrap_or_default();
        assert!(!notes.iter().any(|w| w.as_str().is_some_and(|w| w.starts_with("graphs are written as their art"))), "{name}: {r}");
        let mut again = Session::new();
        again.execute("document.open", &json!({"path": dir.join(name)})).unwrap();
        let mut graphs = vec![];
        again.doc().unwrap().doc.walk(|n| graphs.extend(n.extras().graph.as_deref().cloned()));
        assert_eq!(graphs.len(), 1, "{name}");
        let g = &graphs[0];
        assert_eq!(
            (g.series.as_slice(), g.categories.as_slice(), g.rows.as_slice()),
            (&after.series[..], &after.categories[..], &after.rows[..]),
            "{name}"
        );
        assert_eq!(g.kind, after.kind, "{name}");
    }
}

/// A graph the graph tools draw, of each type, saved as `.ai` in either form, opens again as the same
/// graph exactly when the save gives no warning about graphs; a column graph does.
#[test]
fn a_drawn_graph_saved_as_ai_opens_as_a_graph_unless_the_save_warns() {
    let dir = vectorcraft_testkit::temp_dir("graph-ai-save");
    let mut live = vec![];
    for ty in ["column", "stackedColumn", "bar", "stackedBar", "line", "area", "scatter", "pie", "radar"] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,1,6";
        let id = s.execute("graph.create", &json!({"type": ty, "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let written = spec(&s, NodeId(id));
        for (name, version) in [("v7.ai", Some("7")), ("pdf.ai", None)] {
            let path = dir.join(format!("{ty}-{name}"));
            let mut p = json!({"path": path, "format": "ai"});
            if let Some(v) = version {
                p["aiVersion"] = json!(v);
            }
            let r = s.execute("file.saveAs", &p).unwrap();
            let notes = r["warnings"].as_array().cloned().unwrap_or_default();
            let warned = notes.iter().any(|w| w.as_str().is_some_and(|w| w.starts_with("graphs are written as their art")));
            let mut again = Session::new();
            again.execute("document.open", &json!({"path": path})).unwrap();
            let mut graphs = vec![];
            again.doc().unwrap().doc.walk(|n| graphs.extend(n.extras().graph.as_deref().cloned()));
            assert_eq!(graphs.len(), usize::from(!warned), "{ty} {name}: {r}");
            if let Some(g) = graphs.first() {
                assert_eq!((g.kind, g.cells()), (written.kind, written.cells()), "{ty} {name}");
                assert_eq!((g.series.as_slice(), g.categories.as_slice()), (&written.series[..], &written.categories[..]), "{ty} {name}");
                live.push(format!("{ty} {name}"));
            }
        }
    }
    for ty in ["column", "stackedColumn", "area"] {
        assert!(live.contains(&format!("{ty} v7.ai")) && live.contains(&format!("{ty} pdf.ai")), "{ty}: {live:?}");
    }
}

/// A line graph the graph tools draw with Draw Filled Lines, saved as `.ai` in either form, opens again as the same graph
/// with its filled lines and Line Width, the default 3 pt included, and without a warning about graphs. A Line Width past
/// the records' 100 pt is saved as the graph's art, with the warning.
#[test]
fn a_drawn_graph_with_filled_lines_saved_as_ai_opens_with_them() {
    let dir = vectorcraft_testkit::temp_dir("graph-ai-filled");
    for (width, live) in [(None, true), (Some(8.0), true), (Some(150.0), false)] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,1,6";
        let id = s.execute("graph.create", &json!({"type": "line", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let mut options = json!({"id": id, "filledLines": true});
        if let Some(w) = width {
            options["lineWidth"] = json!(w);
        }
        s.execute("graph.setType", &options).unwrap();
        let written = spec(&s, NodeId(id));
        assert_eq!((written.filled_lines, written.line_width), (true, width));
        for (name, version) in [("v7.ai", Some("7")), ("pdf.ai", None)] {
            let path = dir.join(format!("{width:?}-{name}"));
            let mut p = json!({"path": path, "format": "ai"});
            if let Some(v) = version {
                p["aiVersion"] = json!(v);
            }
            let r = s.execute("file.saveAs", &p).unwrap();
            let notes = r["warnings"].as_array().cloned().unwrap_or_default();
            let warned = notes.iter().any(|w| w.as_str().is_some_and(|w| w.starts_with("graphs are written as their art")));
            assert_eq!(warned, !live, "{width:?} {name}: {r}");
            let mut again = Session::new();
            again.execute("document.open", &json!({"path": path})).unwrap();
            let mut graphs = vec![];
            again.doc().unwrap().doc.walk(|n| graphs.extend(n.extras().graph.as_deref().map(|g| (n.id, g.clone()))));
            assert_eq!(graphs.len(), usize::from(live), "{width:?} {name}");
            if let Some((gid, g)) = graphs.first() {
                assert_eq!((g.kind, g.cells(), g.filled_lines, g.line_width), (written.kind, written.cells(), true, width), "{width:?} {name}");
                assert_eq!(g.filled_line_width(), written.filled_line_width(), "{width:?} {name}");
                // With Draw Filled Lines turned off, each series draws its thin 1 pt line again.
                again.execute("select.set", &json!({"ids": [gid.0]})).unwrap();
                again.execute("graph.setType", &json!({"id": gid.0, "filledLines": false})).unwrap();
                for series in 0..2 {
                    let line = series_parts(&again, *gid, series)
                        .into_iter()
                        .any(|p| p.appearance.fill_paint().is_none() && !p.appearance.stroke_paint().is_none() && p.appearance.stroke_width() == 1.0);
                    assert!(line, "{width:?} {name}: series {series} draws no thin line");
                }
            }
        }
    }
}

/// A default legend swatch has the stroke paint and weight of its series' marks, beside the plot or across the top: the
/// thin white line of stacked columns and bars, areas and pie wedges, and none on columns and bars. A stroke color kept
/// on the series, with or without a weight of its own, is on the swatch too.
#[test]
fn a_legend_swatch_has_the_stroke_of_its_series_marks() {
    let white = |w: f64| (Paint::solid(vectorcraft_color::Color::WHITE), w);
    let cases = [
        ("stackedColumn", white(0.25)),
        ("stackedBar", white(0.25)),
        ("area", white(0.25)),
        ("pie", white(0.5)),
        ("column", (Paint::None, 0.0)),
        ("bar", (Paint::None, 0.0)),
    ];
    for (ty, stroke) in cases {
        for across_top in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
            let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,1,6";
            let id = NodeId(
                s.execute("graph.create", &json!({"type": ty, "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                    .as_u64()
                    .unwrap(),
            );
            s.execute("graph.setType", &json!({"id": id.0, "legendAcrossTop": across_top})).unwrap();
            for i in 0..2 {
                let marks = marks_of(&s, id, i);
                assert!(marks.len() > 1, "{ty} {i}: marks and a swatch");
                assert!(marks.iter().all(|m| (m.1.clone(), m.2) == stroke), "{ty} across top {across_top}, series {i}: {marks:?}");
            }
            // A stroke color kept on series 0 without a weight keeps the default weight, then with one.
            let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].clone();
            let row = bar.appearance.items.iter().rposition(|it| !it.is_fill());
            let Some(row) = row else { continue };
            s.execute("appearance.setItem", &json!({"ids": [bar.id.0], "index": row, "color": "#ff0000"})).unwrap();
            s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
            let marks = marks_of(&s, id, 0);
            assert!(marks.iter().all(|m| *m == marks[0]) && marks[0].1 != stroke.0, "{ty} across top {across_top}, red: {marks:?}");
            let bar = series(s.doc().unwrap().doc.node(id).unwrap(), 0).children().unwrap()[0].id;
            s.execute("appearance.setItem", &json!({"ids": [bar.0], "index": row, "weight": 3})).unwrap();
            s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
            let marks = marks_of(&s, id, 0);
            assert!(marks.iter().all(|m| *m == marks[0]) && marks[0].2 == 3.0, "{ty} across top {across_top}, 3 pt: {marks:?}");
        }
    }
}

/// A stacked column graph the graph tools draw, saved as `.ai` in either form, opens again as the same graph without a
/// warning, and its data can be edited there.
#[test]
fn a_drawn_stacked_column_graph_saved_as_ai_opens_as_a_graph_that_takes_new_data() {
    let dir = vectorcraft_testkit::temp_dir("graph-ai-stacked");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,1,6";
    let id = s.execute("graph.create", &json!({"type": "stackedColumn", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let written = spec(&s, NodeId(id));
    for (name, version) in [("v7.ai", Some("7")), ("pdf.ai", None)] {
        let path = dir.join(name);
        let mut p = json!({"path": path, "format": "ai"});
        if let Some(v) = version {
            p["aiVersion"] = json!(v);
        }
        let r = s.execute("file.saveAs", &p).unwrap();
        let notes = r["warnings"].as_array().cloned().unwrap_or_default();
        assert!(!notes.iter().any(|w| w.as_str().is_some_and(|w| w.starts_with("graphs are written as their art"))), "{name}: {r}");
        let mut again = Session::new();
        again.execute("document.open", &json!({"path": path})).unwrap();
        let mut found = vec![];
        again.doc().unwrap().doc.walk(|n| {
            if n.extras().graph.is_some() {
                found.push(n.id);
            }
        });
        assert_eq!(found.len(), 1, "{name}");
        let g = spec(&again, found[0]);
        assert_eq!((g.kind, g.cells()), (written.kind, written.cells()), "{name}");
        assert_eq!((g.series.as_slice(), g.categories.as_slice()), (&written.series[..], &written.categories[..]), "{name}");
        again.execute("select.set", &json!({"ids": [found[0].0]})).unwrap();
        again.execute("graph.setData", &json!({"id": found[0].0, "csv": ",2024,2025\nQ1,8,1\nQ2,2,7"})).unwrap();
        let after = spec(&again, found[0]);
        assert_eq!((after.kind, after.rows.as_slice()), (written.kind, &[vec![8.0, 1.0], vec![2.0, 7.0]][..]), "{name}");
        // Each series keeps one fill and the thin white stroke on its marks and its swatch.
        for i in 0..2 {
            let marks = marks_of(&again, found[0], i);
            let white = Paint::solid(vectorcraft_color::Color::WHITE);
            assert!(marks.len() > 1 && marks.iter().all(|m| *m == marks[0] && (&m.1, m.2) == (&white, 0.25)), "{name} series {i}: {marks:?}");
        }
    }
}

/// A graph opened from a `.ai` keeps the bounds of its art without type, so moving one of its labels on its own
/// leaves the plot where it is at the first redraw.
#[test]
fn a_graph_opened_from_ai_measures_moves_without_its_labels() {
    let graph = "%_Gs\n%_100 200 300 100 Gb\n%_5 0 0 14 0 1 0 0 6 44 Gy\n%_10 2 0 0 90 80 0 Gd\n\
        %_1 () Ga\n%_0 13 0 0 0 0 0 () GA\n%_2 () Ga\n%_0 14 0 0 0 0 0 () GA\n%_4 () Ga\n%_0 14 0 0 0 0 0 () GA\n\
        %_2 2 1 1 Gz\n%_() Gc (East) Gc (Q1) Gc 5 Gc GC\n\
        u\nu\n0.6 0 1 0 k\n110 100 m\n110 150 L\n130 150 L\n130 100 L\nf\nU\n%_2 1 0 0 Go\n\
        u\n0 0 0 1 K\n100 200 m\n100 100 L\n300 100 L\nS\nU\n\
        0 To\n1 0 0 1 100 220 0 Tp\nTP\n0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n(Sales) Tx\nTO\nU\n%_GS\n";
    let text = format!(
        "%!PS-Adobe-3.0\n%%Creator: a script\n%%BoundingBox: 0 0 400 400\n%AI5_FileFormat 3\n%AI3_Cropmarks: 0 0 400 400\n%%EndComments\n\
         %%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n%AI5_BeginLayer\n1 1 1 1 0 0 0 79 128 255 Lb\n(Graph) Ln\n0 A\n{graph}LB\n\
         %AI5_EndLayer--\n%%PageTrailer\n%%Trailer\n%%EOF\n"
    );
    let dir = vectorcraft_testkit::temp_dir("graph-ai-label");
    let path = dir.join("graph.ai");
    std::fs::write(&path, text).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({"path": path})).unwrap();
    let (mut graphs, mut labels) = (vec![], vec![]);
    s.doc().unwrap().doc.walk(|n| {
        if n.extras().graph.is_some() {
            graphs.push(n.id);
        }
        if matches!(&n.kind, NodeKind::Text(t) if t.plain_text() == "Sales") {
            labels.push(n.id);
        }
    });
    assert_eq!((graphs.len(), labels.len()), (1, 1));
    let (id, label) = (graphs[0], labels[0]);
    let before = spec(&s, id);
    s.execute("select.set", &json!({"ids": [label.0]})).unwrap();
    s.execute("text.setStyle", &json!({"size": 30})).unwrap();
    s.execute("object.move", &json!({"dx": 80, "dy": -40})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",East\nQ1,8\nQ2,2"})).unwrap();
    let rect = spec(&s, id).rect;
    assert_eq!(rect, before.rect, "after restyling and moving a label");
    // A move of the whole opened graph is still followed.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 15, "dy": 25})).unwrap();
    s.execute("graph.setData", &json!({"id": id.0, "csv": ",East\nQ1,8\nQ2,2"})).unwrap();
    let moved = spec(&s, id).rect;
    assert!((moved.x0 - (rect.x0 + 15.0)).abs() < 1e-6 && (moved.y0 - (rect.y0 + 25.0)).abs() < 1e-6, "{moved:?}");
    assert!((moved.width() - rect.width()).abs() < 1e-6 && (moved.height() - rect.height()).abs() < 1e-6, "{moved:?}");
}
