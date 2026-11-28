//! The Liquify tools: their options (Brush Affects, Simplify, Use Pressure Pen, Show Brush Size),
//! Alt-drag brush sizing, holding still, what they leave alone and incremental strokes.

use serde_json::{Value, json};
use vectorcraft_geom::{PathData, Point};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::new(kind, x, y)
}

/// Down at the first point, drags through the middle ones, up at the last.
fn stroke(s: &mut Session, pts: &[(f64, f64)], pressure: f32) {
    let v = ViewInfo::default();
    for (i, &(x, y)) in pts.iter().enumerate() {
        let kind = match i {
            0 => PointerKind::Down,
            _ if i + 1 == pts.len() => PointerKind::Up,
            _ => PointerKind::Drag,
        };
        s.pointer(&PointerEvent { pressure, ..ev(kind, x, y) }, v).unwrap();
    }
}

fn last_journal(s: &Session) -> (String, Value) {
    s.journal.last().unwrap().clone()
}

#[test]
fn brush_affects_reach_the_command() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    s.select_tool("scallop", ViewInfo::default()).unwrap();
    s.set_tool_option("affectAnchors", &json!(false));
    s.set_tool_option("affectOut", &json!(false));
    stroke(&mut s, &[(300.0, 150.0), (300.0, 200.0), (300.0, 250.0)], 1.0);
    let (cmd, p) = last_journal(&s);
    assert_eq!((cmd.as_str(), &p["affectAnchors"], &p["affectIn"], &p["affectOut"]), ("object.liquify", &json!(false), &json!(true), &json!(false)));
    // Replaying it gives the same path, and the boxes change the result.
    let mut t = session();
    let b = rect(&mut t, 100.0, 100.0, 200.0, 200.0);
    t.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    t.execute(&cmd, &p).unwrap();
    assert_eq!(path(&t, b), path(&s, a));
    let mut all = p.clone();
    for k in ["affectAnchors", "affectIn", "affectOut"] {
        all[k] = json!(true);
    }
    t.execute("edit.undo", &json!({})).unwrap();
    t.execute(&cmd, &all).unwrap();
    assert_ne!(path(&t, b), path(&s, a));
}

#[test]
fn simplify_can_be_turned_off() {
    let run = |on: bool| {
        let mut s = session();
        let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
        // Pucker centred on an edge pulls its new anchors along the edge: they stay flat.
        let p = json!({"tool": "pucker", "points": [[300, 200]], "intensity": 1, "detail": 10, "simplify": 100, "simplifyOn": on});
        s.execute("object.liquify", &p).unwrap();
        path(&s, a).anchor_count()
    };
    assert!(run(false) > run(true), "{} vs {}", run(false), run(true));
}

#[test]
fn pen_pressure_is_the_intensity_with_use_pressure_pen() {
    let run = |pressures: [f64; 2], use_pressure: bool| {
        let mut s = session();
        let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
        let pts = json!([[260, 150, pressures[0]], [260, 250, pressures[1]]]);
        s.execute("object.liquify", &json!({"tool": "pucker", "points": pts, "width": 120, "height": 120, "usePressure": use_pressure})).unwrap();
        path(&s, a)
    };
    let square = {
        let mut s = session();
        let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
        path(&s, a)
    };
    // No pressure: nothing moves; more pressure moves more; pressure is ignored without the option.
    assert_eq!(run([0.0, 0.0], true), square);
    let x = |p: &PathData| p.anchors().map(|(_, _, an)| an.p.x).fold(f64::MAX, |m, v| if v > 200.0 { m.min(v) } else { m });
    assert!(x(&run([1.0, 1.0], true)) < x(&run([0.3, 0.3], true)));
    assert_eq!(run([0.0, 0.0], false), run([1.0, 1.0], false));

    // The tool sends each sample's pressure while the option is on.
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    s.select_tool("bloat", ViewInfo::default()).unwrap();
    stroke(&mut s, &[(300.0, 150.0), (300.0, 200.0), (300.0, 210.0)], 0.4);
    assert_eq!(last_journal(&s).1["points"][0].as_array().unwrap().len(), 2);
    s.set_tool_option("usePressure", &json!(true));
    stroke(&mut s, &[(300.0, 150.0), (300.0, 200.0), (300.0, 210.0)], 0.4);
    let (_, p) = last_journal(&s);
    assert_eq!((p["usePressure"].as_bool(), p["points"][1][2].as_f64().map(|f| (f * 1e6).round() / 1e6)), (Some(true), Some(0.4)));
}

#[test]
fn alt_drag_sizes_the_brush_from_its_size() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("warp", v).unwrap();
    s.set_tool_option("height", &json!(60));
    let alt = Mods { alt: true, ..Mods::default() };
    let shift_alt = Mods { shift: true, ..alt };
    let size = |s: &Session| (s.tool_options()["width"].as_f64().unwrap(), s.tool_options()["height"].as_f64().unwrap());
    s.pointer(&ev(PointerKind::Down, 50.0, 50.0).with_mods(alt), v).unwrap();
    // The first sample barely moves: the brush stays about as it was (it used to collapse to 1 pt).
    s.pointer(&ev(PointerKind::Drag, 51.0, 50.0).with_mods(alt), v).unwrap();
    assert_eq!(size(&s), (102.0, 60.0));
    s.pointer(&ev(PointerKind::Drag, 60.0, 45.0).with_mods(alt), v).unwrap();
    assert_eq!(size(&s), (120.0, 50.0));
    s.pointer(&ev(PointerKind::Up, 60.0, 45.0).with_mods(alt), v).unwrap();
    assert_eq!(s.journal.iter().filter(|(c, _)| c == "object.liquify").count(), 0, "sizing doesn't liquify");
    // Shift keeps the proportions.
    s.pointer(&ev(PointerKind::Down, 0.0, 0.0).with_mods(shift_alt), v).unwrap();
    s.pointer(&ev(PointerKind::Drag, 60.0, 5.0).with_mods(shift_alt), v).unwrap();
    assert_eq!(size(&s), (240.0, 100.0));
    s.pointer(&ev(PointerKind::Up, 60.0, 5.0).with_mods(shift_alt), v).unwrap();
    // The size is kept: the other Liquify tools share it.
    s.select_tool("pucker", v).unwrap();
    assert_eq!(size(&s), (240.0, 100.0));
}

#[test]
fn show_brush_size_hides_the_outline() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("twirl", v).unwrap();
    s.pointer(&ev(PointerKind::Move, 200.0, 200.0), v).unwrap();
    assert_eq!(s.overlays(v).len(), 1);
    s.set_tool_option("showBrush", &json!(false));
    assert!(s.overlays(v).is_empty());
    assert_eq!(s.tool_options()["showBrush"], json!(false));
    // Sizing the brush shows it anyway.
    s.pointer(&ev(PointerKind::Down, 200.0, 200.0).with_mods(Mods { alt: true, ..Mods::default() }), v).unwrap();
    assert_eq!(s.overlays(v).len(), 1);
}

// ---------- holding still ----------

/// Twirl pressed at (300, 200) on the square's right edge, held for `holds` (seconds per tick),
/// released.
fn held_twirl(holds: &[f64]) -> (Session, NodeId) {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    let v = ViewInfo::default();
    s.select_tool("twirl", v).unwrap();
    s.pointer(&ev(PointerKind::Down, 300.0, 200.0), v).unwrap();
    for dt in holds {
        assert!(s.tool_wants_ticks());
        s.tool_tick(*dt, v).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, 300.0, 200.0), v).unwrap();
    (s, a)
}

#[test]
fn holding_still_keeps_twirling_scaled_by_time() {
    let (once, a) = held_twirl(&[]);
    let (held, _) = held_twirl(&[0.35]);
    let (ticked, _) = held_twirl(&[0.1, 0.1, 0.1, 0.05]);
    // A tenth of a second held is one more dab where the brush is: 3 after 0.35 s.
    let (_, p) = last_journal(&held);
    assert_eq!(p["points"], json!([[300.0, 200.0], [300.0, 200.0], [300.0, 200.0], [300.0, 200.0]]));
    assert_eq!(path(&held, a), path(&ticked, a), "the same time held in other ticks");
    // Longer holds twirl further: the edge's anchors swing further off x = 300.
    let c = Point::new(300.0, 200.0);
    let turned =
        |s: &Session| path(s, a).anchors().filter(|(_, _, an)| an.p.distance(c) < 50.0).map(|(_, _, an)| (an.p.x - 300.0).abs()).fold(0.0, f64::max);
    let (longer, _) = held_twirl(&[1.0]);
    assert!(turned(&longer) > turned(&held) && turned(&held) > turned(&once), "{} {} {}", turned(&longer), turned(&held), turned(&once));
    // One undo step whose journal entry replays the same.
    let mut t = session();
    rect(&mut t, 100.0, 100.0, 200.0, 200.0);
    t.execute("object.liquify", &p).unwrap();
    assert_eq!(path(&t, a), path(&held, a));
}

#[test]
fn only_twirl_pucker_and_bloat_keep_going() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    let v = ViewInfo::default();
    for (tool, holds) in [("warp", false), ("scallop", false), ("crystallize", false), ("wrinkle", false), ("pucker", true), ("bloat", true)] {
        s.select_tool(tool, v).unwrap();
        assert!(!s.tool_wants_ticks(), "{tool} idle");
        s.pointer(&ev(PointerKind::Down, 300.0, 200.0), v).unwrap();
        assert_eq!(s.tool_wants_ticks(), holds, "{tool}");
        s.tool_tick(0.5, v).unwrap();
        s.pointer(&ev(PointerKind::Up, 300.0, 200.0), v).unwrap();
        let n = last_journal(&s).1["points"].as_array().unwrap().len();
        assert_eq!(n, if holds { 6 } else { 1 }, "{tool}");
    }
    // Repeated points are no extra dabs for the tools that don't keep going.
    let warp = |pts: Value| {
        let mut t = session();
        let a = rect(&mut t, 100.0, 100.0, 200.0, 200.0);
        t.execute("object.liquify", &json!({"tool": "warp", "points": pts})).unwrap();
        path(&t, a)
    };
    assert_eq!(warp(json!([[300, 200], [300, 200], [320, 200]])), warp(json!([[300, 200], [320, 200]])));
}

// ---------- what Liquify leaves alone ----------

fn node_json(s: &Session, id: u64) -> Value {
    serde_json::to_value(s.doc().unwrap().doc.node(NodeId(id)).unwrap()).unwrap()
}

#[test]
fn type_symbols_graphs_and_live_contents_are_left_alone_and_reported() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let text = id(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Liquid"})).unwrap());
    let graph = id(s.execute("graph.create", &json!({"type": "column", "x": 300, "y": 60, "width": 120, "height": 80, "rows": [[1, 2]]})).unwrap());
    let sq = rect(&mut s, 450.0, 60.0, 60.0, 60.0);
    s.execute("select.set", &json!({"ids": [sq.0]})).unwrap();
    let inst = id(s.execute("symbol.new", &json!({"name": "Dot"})).unwrap());
    let a = rect(&mut s, 100.0, 300.0, 60.0, 60.0);
    let b = rect(&mut s, 200.0, 300.0, 60.0, 60.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let blend = id(s.execute("object.blend.make", &json!({"steps": 2})).unwrap());
    let c = rect(&mut s, 400.0, 300.0, 80.0, 60.0);
    s.execute("select.set", &json!({"ids": [c.0]})).unwrap();
    let env = id(s.execute("object.envelope.makeWithWarp", &json!({})).unwrap());
    let r = rect(&mut s, 560.0, 300.0, 40.0, 40.0);
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let rep = id(s.execute("object.repeat.radial", &json!({"instances": 4, "radius": 60})).unwrap());
    let plain = rect(&mut s, 100.0, 450.0, 600.0, 60.0);
    let all = [text, graph, inst, blend, env, rep];
    let before: Vec<Value> = all.iter().map(|i| node_json(&s, *i)).collect();
    // Nothing selected: the brush sweeps the whole drawing.
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let pts: Vec<Value> = (0..=16).flat_map(|i| [json!([80 + i * 40, 100]), json!([80 + i * 40, 330]), json!([80 + i * 40, 480])]).collect();
    let out = s.execute("object.liquify", &json!({"tool": "bloat", "points": pts, "width": 160, "height": 160, "intensity": 1})).unwrap();
    for (i, id) in all.iter().enumerate() {
        assert_eq!(node_json(&s, *id), before[i], "object {id} unchanged");
    }
    assert!(out["ids"].as_array().unwrap().contains(&json!(plain.0)));
    let mut skipped: Vec<u64> = out["skipped"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    skipped.sort();
    let mut want = all.to_vec();
    want.sort();
    assert_eq!(skipped, want);
    let w = out["warning"].as_str().unwrap();
    for kind in ["type", "symbols", "graphs", "blends", "envelopes", "repeats"] {
        assert!(w.contains(kind), "{kind} in {w}");
    }
    // Selected, they are left alone too (and a path inside a blend is the blend's).
    s.execute("select.set", &json!({"ids": [text, blend, plain.0]})).unwrap();
    let out =
        s.execute("object.liquify", &json!({"tool": "pucker", "points": [[130, 100], [130, 330], [130, 480]], "width": 200, "height": 200})).unwrap();
    assert_eq!(out["ids"], json!([plain.0]));
    assert_eq!(node_json(&s, blend), before[3]);
    let inside = s.execute("object.liquify", &json!({"tool": "pucker", "ids": [a.0], "points": [[130, 330]]})).unwrap();
    assert_eq!((&inside["ids"], &inside["skipped"]), (&json!([]), &json!([blend])));
}

#[test]
fn guides_are_left_alone_in_selection_mode_and_the_tool_reports_skips() {
    let mut s = session();
    let g = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    s.execute("view.guides.make", &json!({})).unwrap();
    let guide = path(&s, g);
    let a = rect(&mut s, 120.0, 120.0, 160.0, 160.0);
    let t = s.execute("text.create", &json!({"x": 300, "y": 200, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [g.0, a.0, t]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("bloat", v).unwrap();
    let ui = s.pointer(&ev(PointerKind::Down, 290.0, 200.0), v).unwrap();
    assert!(matches!(ui.first(), Some(UiRequest::Status(m)) if m.contains("type")), "{ui:?}");
    s.pointer(&ev(PointerKind::Up, 290.0, 200.0), v).unwrap();
    assert_eq!(path(&s, g), guide);
    assert_ne!(path(&s, a).bounds().unwrap(), vectorcraft_geom::Rect::new(120.0, 120.0, 280.0, 280.0));
}

// ---------- incremental strokes ----------

/// Every prefix of `pts` previewed in one interaction gives what the whole prefix gives at once.
fn previews_match_whole_strokes(params: Value, pts: &[Value], select: bool) {
    let setup = || {
        let mut s = session();
        let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
        let b = rect(&mut s, 330.0, 120.0, 120.0, 160.0);
        if select {
            s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        } else {
            s.execute("select.set", &json!({"ids": []})).unwrap();
        }
        (s, a, b)
    };
    let (mut live, a, b) = setup();
    live.begin_interaction("Liquify").unwrap();
    for k in 1..=pts.len() {
        let mut p = params.clone();
        p["points"] = json!(pts[..k]);
        let r = live.preview("object.liquify", &p).unwrap();
        let (mut whole, ..) = setup();
        let w = whole.execute("object.liquify", &p).unwrap();
        assert_eq!(r["ids"], w["ids"], "prefix {k}");
        assert_eq!((path(&live, a), path(&live, b)), (path(&whole, a), path(&whole, b)), "prefix {k} of {params}");
    }
    assert!(live.liquify_stroke.is_some(), "the drag goes on from the last preview");
    live.commit_interaction().unwrap();
}

#[test]
fn a_stroke_applied_as_it_grows_equals_the_whole_stroke() {
    let path_pts: Vec<Value> = (0..14).map(|i| json!([250 + i * 12, 130 + (i * 37) % 90])).collect();
    previews_match_whole_strokes(json!({"tool": "warp", "width": 70, "height": 50, "angle": 20, "intensity": 0.8}), &path_pts, false);
    previews_match_whole_strokes(json!({"tool": "crystallize", "width": 60, "height": 60, "complexity": 4}), &path_pts, true);
    // Holds (repeated points) and pressure.
    let mut held: Vec<Value> = path_pts.iter().take(5).cloned().collect();
    held.extend([json!([298, 200, 0.4]), json!([298, 200, 0.4]), json!([298, 200, 0.9]), json!([310, 210, 1])]);
    previews_match_whole_strokes(json!({"tool": "pucker", "usePressure": true}), &held, false);
    previews_match_whole_strokes(json!({"tool": "twirl", "rate": 120}), &held, true);
}

#[test]
fn a_changed_brush_or_another_stroke_starts_afresh() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    s.begin_interaction("Liquify").unwrap();
    s.preview("object.liquify", &json!({"tool": "bloat", "points": [[300, 150], [300, 200]]})).unwrap();
    // Not a continuation (other points, another size): applied to the snapshot, as a fresh call is.
    s.preview("object.liquify", &json!({"tool": "bloat", "points": [[300, 250]], "width": 60})).unwrap();
    let mut t = session();
    rect(&mut t, 100.0, 100.0, 200.0, 200.0);
    t.execute("object.liquify", &json!({"tool": "bloat", "points": [[300, 250]], "width": 60})).unwrap();
    assert_eq!(path(&s, a), path(&t, a));
}
