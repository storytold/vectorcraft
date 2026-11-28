//! Tests for the Fold Preview's model: a blank drawn in a document folded into its 3D shape.

use serde_json::{Value, json};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    s
}

fn path(s: &mut Session, d: &str) -> u64 {
    s.execute("path.create", &json!({ "d": d })).unwrap()["id"].as_u64().unwrap()
}

fn role(s: &mut Session, role: &str, ids: &[u64]) {
    s.execute("addon.packaging.setRole", &json!({ "role": role, "ids": ids })).unwrap();
}

fn fold(s: &mut Session, p: Value) -> Value {
    s.execute("addon.packaging.fold", &p).unwrap()
}

/// A tray blank: a 100 pt base (x 140 to 240, y 140 to 240) with a 40 pt wall on each side →
/// (outline, creases).
fn tray(s: &mut Session) -> (u64, Vec<u64>) {
    let outline = path(s, "M140 100 L240 100 L240 140 L280 140 L280 240 L240 240 L240 280 L140 280 L140 240 L100 240 L100 140 L140 140 Z");
    let creases = vec![path(s, "M140 140 L240 140"), path(s, "M240 140 L240 240"), path(s, "M240 240 L140 240"), path(s, "M140 240 L140 140")];
    role(s, "cut", &[outline]);
    role(s, "crease", &creases);
    s.execute("select.none", &json!({})).unwrap();
    (outline, creases)
}

fn vector(v: &Value) -> [f64; 3] {
    let a = v.as_array().unwrap();
    [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()]
}

/// The middle of the bounds of the points of SVG path data `d` (absolute moves and lines only).
fn middle(d: &str) -> [f64; 2] {
    let n: Vec<f64> = d.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter_map(|t| t.parse().ok()).collect();
    let span = |k: usize| n.iter().skip(k).step_by(2).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &t| (lo.min(t), hi.max(t)));
    let ((x0, x1), (y0, y1)) = (span(0), span(1));
    [(x0 + x1) / 2.0, (y0 + y1) / 2.0]
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn a_tray_folds_its_walls_up_round_the_base() {
    let mut s = session();
    tray(&mut s);
    let steps = s.doc().unwrap().history.undo.len();
    let r = fold(&mut s, json!({}));
    assert_eq!((r["ok"].as_bool(), r["blank"].as_u64(), r["blanks"].as_u64()), (Some(true), Some(0), Some(1)), "{r}");
    let panels = r["panels"].as_array().unwrap();
    assert_eq!(panels.len(), 5);
    let base = r["base"].as_u64().unwrap() as usize;
    let frame = &panels[base]["frame"];
    assert!(
        near(vector(&frame["x"]), [1.0, 0.0, 0.0]) && near(vector(&frame["z"]), [0.0, 0.0, 1.0]) && near(vector(&frame["origin"]), [0.0; 3]),
        "{frame}"
    );
    assert!(panels[base]["parent"].is_null());
    assert!(panels[base]["outline"].as_str().unwrap().starts_with('M'), "SVG path data");
    for (i, p) in panels.iter().enumerate().filter(|(i, _)| *i != base) {
        assert_eq!((p["parent"].as_u64(), p["depth"].as_u64()), (Some(base as u64), Some(1)), "{p}");
        // Each wall stands square to the base, below it (away from the print), its print facing out:
        // away from the base's middle (190, 190), toward the side the wall lies on.
        let [cx, cy] = middle(p["outline"].as_str().unwrap());
        let (x, y, o) = (vector(&p["frame"]["x"]), vector(&p["frame"]["y"]), vector(&p["frame"]["origin"]));
        let placed: [f64; 3] = std::array::from_fn(|k| o[k] + x[k] * cx - y[k] * cy);
        let (dx, dy) = (cx - 190.0, 190.0 - cy);
        let out = if dx.abs() > dy.abs() { [dx.signum(), 0.0, 0.0] } else { [0.0, dy.signum(), 0.0] };
        assert!(near(vector(&p["frame"]["z"]), out), "wall {i}: {p}");
        assert!((placed[2] + 20.0).abs() < 1e-9, "wall {i}'s middle is 20 pt below the base: {placed:?}");
    }
    let hinges = r["hinges"].as_array().unwrap();
    assert_eq!(hinges.len(), 4);
    assert!(hinges.iter().all(|h| h["kind"] == "tree" && h["angle"] == 90.0 && h["closes"] == true), "{r}");
    assert_eq!(r["collisions"], json!([]));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "a fold records no undo step");

    // Flat, every frame is the flat blank's own.
    let r = fold(&mut s, json!({ "amount": 0 }));
    for p in r["panels"].as_array().unwrap() {
        assert!(near(vector(&p["frame"]["z"]), [0.0, 0.0, 1.0]) && near(vector(&p["frame"]["origin"]), [0.0; 3]), "{p}");
    }
}

#[test]
fn the_base_is_the_panel_under_a_point() {
    let mut s = session();
    tray(&mut s);
    // The left wall: x 100 to 140, y 140 to 240.
    let r = fold(&mut s, json!({ "base": [120, 190] }));
    let base = r["base"].as_u64().unwrap() as usize;
    let panels = r["panels"].as_array().unwrap();
    assert!(near(vector(&panels[base]["frame"]["z"]), [0.0, 0.0, 1.0]));
    // The tray's base is now a child of the wall, and the other walls its children.
    let depths: Vec<u64> = panels.iter().map(|p| p["depth"].as_u64().unwrap()).collect();
    let mut sorted = depths.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, [0, 1, 2, 2, 2], "{depths:?}");
    let e = s.execute("addon.packaging.fold", &json!({ "base": [10, 10] })).unwrap_err().to_string();
    assert!(e.contains("`base` is in no panel"), "{e}");
}

#[test]
fn bad_values_are_refused() {
    let mut s = session();
    tray(&mut s);
    for (p, want) in [
        (json!({ "amount": 1.5 }), "`amount` must be from 0 to 1"),
        (json!({ "amount": -0.1 }), "`amount` must be from 0 to 1"),
        (json!({ "amount": "1" }), "`amount` must be a number"),
        (json!({ "base": [1] }), "`base` must be a point"),
        (json!({ "base": "middle" }), "`base` must be a point"),
        (json!({ "tolerance": 50 }), "`tolerance` must be from"),
    ] {
        let e = s.execute("addon.packaging.fold", &p).unwrap_err().to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
}

#[test]
fn without_a_blank_nothing_is_folded_and_the_issues_say_why() {
    let mut s = session();
    let r = fold(&mut s, json!({}));
    assert_eq!((r["ok"].as_bool(), r["blank"].is_null(), r["blanks"].as_u64()), (Some(false), true, Some(0)), "{r}");
    assert_eq!(r["panels"], json!([]));
    assert_eq!(r["issues"][0]["code"], "noCut");
}

#[test]
fn the_print_leaves_out_fold_lines_and_layers_that_do_not_print() {
    let mut s = session();
    let (outline, creases) = tray(&mut s);
    let art = path(&mut s, "M150 150 L200 150 L200 200 Z");
    let notes = s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.setProps", &json!({"id": notes, "printable": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let hidden: Vec<u64> = cmd::print_hidden(d, false).into_iter().map(|id| id.0).collect();
    for id in creases.iter().chain([&outline, &notes]) {
        assert!(hidden.contains(id), "{id} in {hidden:?}");
    }
    assert!(!hidden.contains(&art));
    let shown: Vec<u64> = cmd::print_hidden(d, true).into_iter().map(|id| id.0).collect();
    assert_eq!(shown, vec![notes], "with the fold lines shown, only the layer is left out");
    let (blanks, issues) = cmd::fold_source(d);
    assert_eq!((blanks.len(), blanks[0].panels.len()), (1, 5));
    assert!(issues.is_empty(), "{issues:?}");
}
