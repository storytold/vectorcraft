//! Tests for Set Fold Angle: a fold line's own angle, where it is kept, and how the fold uses it.

use serde_json::{Value, json};

use super::{KEY, fold_angles, own_angle};
use crate::*;

const SET: &str = "addon.packaging.setFoldAngle";

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

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn err(s: &mut Session, p: Value) -> String {
    s.execute(SET, &p).unwrap_err().to_string()
}

/// The text object `id`'s data holds under the fold angle key.
fn stored(s: &Session, id: u64) -> Option<String> {
    let d = &s.doc().unwrap().doc;
    let n = d.node(NodeId(id)).unwrap();
    n.extras().attrs.as_deref().and_then(|a| a.data.iter().find(|(k, _)| k == KEY).map(|(_, v)| v.clone()))
}

/// A tray blank: a 100 pt base (x 140 to 240, y 140 to 240) with a 40 pt wall on each side →
/// (outline, creases: top, right, bottom, left).
fn tray(s: &mut Session) -> (u64, Vec<u64>) {
    let outline = path(s, "M140 100 L240 100 L240 140 L280 140 L280 240 L240 240 L240 280 L140 280 L140 240 L100 240 L100 140 L140 140 Z");
    let creases = vec![path(s, "M140 140 L240 140"), path(s, "M240 140 L240 240"), path(s, "M240 240 L140 240"), path(s, "M140 240 L140 140")];
    role(s, "cut", &[outline]);
    role(s, "crease", &creases);
    s.execute("select.none", &json!({})).unwrap();
    (outline, creases)
}

/// Whether panel `p` of a fold is the tray's top wall (y 100 to 140): the middle of the bounds of
/// its outline's points (absolute moves and lines only) is above the base.
fn top_wall(p: &Value) -> bool {
    let d = p["outline"].as_str().unwrap();
    let n: Vec<f64> = d.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter_map(|t| t.parse().ok()).collect();
    let ys = n.iter().skip(1).step_by(2).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &t| (lo.min(t), hi.max(t)));
    (ys.0 + ys.1) / 2.0 < 140.0
}

fn vector(v: &Value) -> [f64; 3] {
    let a = v.as_array().unwrap();
    [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()]
}

#[test]
fn set_fold_angle_keeps_the_angle_on_the_line_and_reset_takes_it_away() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    let before = steps(&s);
    let r = s.execute(SET, &json!({"angle": 45, "ids": [creases[0]]})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64(), &r["ids"]), (Some(1), Some(0), &json!([creases[0]])), "{r}");
    assert_eq!(stored(&s, creases[0]).as_deref(), Some("45"));
    assert_eq!(steps(&s), before + 1, "one undo step");
    // The same angle again changes nothing and records no undo step.
    let r = s.execute(SET, &json!({"angle": 45.0, "ids": [creases[0]]})).unwrap();
    assert_eq!(r["changed"].as_u64(), Some(0));
    assert_eq!(steps(&s), before + 1);
    // An angle is kept to the millionth of a degree, so one closer to 45 than that is 45.
    let r = s.execute(SET, &json!({"angle": 45.00000001, "ids": [creases[0]]})).unwrap();
    assert_eq!((r["changed"].as_u64(), steps(&s)), (Some(0), before + 1), "{r}");
    s.execute(SET, &json!({"angle": 179.99999998, "ids": [creases[3]]})).unwrap();
    assert_eq!(stored(&s, creases[3]).as_deref(), Some("180"));
    s.execute("edit.undo", &json!({})).unwrap();
    // A number as text, and the selection when ids is left out; -0 is kept as 0.
    s.execute("select.set", &json!({"ids": [creases[1], creases[2]]})).unwrap();
    let r = s.execute(SET, &json!({"angle": "-0"})).unwrap();
    assert_eq!(r["changed"].as_u64(), Some(2), "{r}");
    assert_eq!(stored(&s, creases[1]).as_deref(), Some("0"));
    let r = s.execute(SET, &json!({"reset": true, "ids": [creases[0], creases[1], creases[3]]})).unwrap();
    assert_eq!(r["changed"].as_u64(), Some(2), "the line without an angle of its own is unchanged: {r}");
    assert_eq!((stored(&s, creases[0]), stored(&s, creases[1])), (None, None));
    // Nothing left to reset: no undo step.
    let at = steps(&s);
    let r = s.execute(SET, &json!({"reset": true, "ids": [creases[0]]})).unwrap();
    assert_eq!((r["changed"].as_u64(), steps(&s)), (Some(0), at));
    // Undo brings the angle back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(stored(&s, creases[0]).as_deref(), Some("45"));
}

#[test]
fn bad_angles_and_targets_are_refused() {
    let mut s = session();
    let (outline, creases) = tray(&mut s);
    let plain = path(&mut s, "M10 10 L50 50");
    let before = steps(&s);
    let ids = json!([creases[0]]);
    assert!(err(&mut s, json!({"ids": ids})).contains("missing `angle`"));
    assert!(err(&mut s, json!({"angle": 30, "reset": true, "ids": ids})).contains("not both"));
    assert!(err(&mut s, json!({"angle": 180.5, "ids": ids})).contains("from -180 to 180"));
    assert!(err(&mut s, json!({"angle": -181, "ids": ids})).contains("from -180 to 180"));
    assert!(err(&mut s, json!({"angle": "wide", "ids": ids})).contains("must be a number"));
    assert!(err(&mut s, json!({"angle": true, "ids": ids})).contains("must be a number"));
    assert!(err(&mut s, json!({"reset": "yes", "ids": ids})).contains("true or false"));
    // A cut line and a path without a role do not fold.
    assert!(err(&mut s, json!({"angle": 30, "ids": [outline, plain]})).contains("crease, partial cut or perforation"));
    // An id of no object is named as such, alone or beside a crease.
    assert!(err(&mut s, json!({"angle": 30, "ids": [999_999]})).contains("no such object id 999999"));
    assert!(err(&mut s, json!({"angle": 30, "ids": [creases[0], 999_999]})).contains("no such object id 999999"));
    assert_eq!(steps(&s), before, "no refusal records an undo step");
    // The ends of the range are allowed; cut lines and plain paths among fold lines are skipped.
    let r = s.execute(SET, &json!({"angle": -180, "ids": [creases[0], outline, plain]})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64(), &r["ids"]), (Some(1), Some(2), &json!([creases[0]])), "{r}");
    s.execute(SET, &json!({"angle": 180, "ids": [creases[0]]})).unwrap();
    assert_eq!(stored(&s, creases[0]).as_deref(), Some("180"));
    assert_eq!(stored(&s, outline), None);
}

#[test]
fn set_fold_angle_is_dimmed_without_a_line_it_can_change() {
    let mut s = session();
    let (outline, creases) = tray(&mut s);
    let enabled = |s: &Session| (find_command(SET).unwrap().enabled)(s).is_ok();
    assert!(!enabled(&s), "nothing selected");
    s.execute("select.set", &json!({"ids": [outline]})).unwrap();
    assert!(!enabled(&s), "a cut line does not fold");
    assert!(err(&mut s, json!({"angle": 30})).contains("crease, partial cut or perforation"));
    s.execute("select.set", &json!({"ids": [creases[0]]})).unwrap();
    assert!(enabled(&s));
    s.execute("object.lock", &json!({})).unwrap();
    assert_eq!(fold_angles(&s.doc().unwrap().doc, &[NodeId(creases[0])]).lines, 0, "a locked crease can't be changed");
    // A group holding a crease is enough.
    s.execute("select.set", &json!({"ids": [creases[1], creases[2]]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    assert!(enabled(&s));
    // A layer selected with a crease in it: the crease is inside the layer, which is left out, as
    // Set Fold Angle itself leaves them out.
    let layer = s.doc().unwrap().doc.layer_of(NodeId(creases[3])).unwrap();
    s.execute("select.set", &json!({"ids": [layer.0, creases[3]]})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects.len(), 2, "both are selected");
    assert!(err(&mut s, json!({"angle": 30})).contains("crease, partial cut or perforation"));
    assert!(!enabled(&s), "dimmed as the command refuses it");
    let r = s.execute("addon.packaging.roles", &json!({})).unwrap();
    assert_eq!((r["foldLines"].as_u64(), &r["foldAngle"], r["ownAngles"].as_u64()), (Some(0), &Value::Null, Some(0)), "{r}");
    // With nothing selected the counts are the document's, but Set Fold Angle takes no line.
    s.execute("select.set", &json!({"ids": []})).unwrap();
    assert!(s.execute(SET, &json!({"angle": 30})).is_err());
    let r = s.execute("addon.packaging.roles", &json!({})).unwrap();
    assert_eq!(r["scope"].as_str(), Some("document"), "{r}");
    assert_eq!(r["roles"][1]["count"].as_u64(), Some(4), "{r}");
    assert_eq!((r["foldLines"].as_u64(), &r["foldAngle"], r["ownAngles"].as_u64()), (Some(0), &Value::Null, Some(0)), "{r}");
}

#[test]
fn a_crease_named_in_ids_is_set_after_the_menu_found_none_in_the_selection() {
    let mut s = session();
    let (outline, creases) = tray(&mut s);
    s.execute("select.set", &json!({"ids": [outline]})).unwrap();
    assert!((find_command(SET).unwrap().enabled)(&s).is_err(), "a cut line does not fold");
    let r = s.execute(SET, &json!({"angle": 30, "ids": [creases[0]]})).unwrap();
    assert_eq!(r["changed"].as_u64(), Some(1), "{r}");
    assert!((find_command(SET).unwrap().enabled)(&s).is_err(), "the selection is still the cut line");
}

#[test]
fn the_selection_s_fold_angles_count_a_crease_and_its_selected_group_once() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    s.execute(SET, &json!({"angle": 45, "ids": [creases[0]]})).unwrap();
    s.execute("select.set", &json!({"ids": [creases[0], creases[1]]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let group = s.doc().unwrap().selection.objects[0].0;
    s.execute("select.set", &json!({"ids": [group, creases[0]]})).unwrap();
    let f = crate::cmd::selection_fold_angles(&s);
    assert_eq!((f.lines, f.angle, f.own), (2, None, 1), "{f:?}");
}

#[test]
fn a_locked_crease_is_skipped() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    s.execute("select.set", &json!({"ids": [creases[1]]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let r = s.execute(SET, &json!({"angle": 30, "ids": [creases[0], creases[1]]})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64()), (Some(1), Some(1)), "{r}");
    assert_eq!(stored(&s, creases[1]), None);
}

#[test]
fn a_locked_path_of_a_compound_crease_named_by_id_is_skipped() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    let (a, b) = (path(&mut s, "M300 300 L400 300"), path(&mut s, "M300 320 L400 320"));
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    role(&mut s, "crease", &[compound]);
    s.edit("Test", |d, _| {
        if let Some(n) = d.node_mut(NodeId(a)) {
            n.locked = true;
        }
        Ok(())
    })
    .unwrap();
    assert!(err(&mut s, json!({"angle": 30, "ids": [a]})).contains("crease, partial cut or perforation"));
    // Named twice, it is skipped once.
    let r = s.execute(SET, &json!({"angle": 30, "ids": [a, a, creases[0]]})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64(), &r["ids"]), (Some(1), Some(1), &json!([creases[0]])), "{r}");
    assert_eq!(stored(&s, compound), None);
    assert_eq!(fold_angles(&s.doc().unwrap().doc, &[NodeId(a)]).lines, 0);
    let r = s.execute("addon.packaging.roles", &json!({"ids": [a]})).unwrap();
    assert_eq!((r["foldLines"].as_u64(), &r["foldAngle"]), (Some(0), &Value::Null), "{r}");
    // The same with the locked path selected.
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    assert!((find_command(SET).unwrap().enabled)(&s).is_err(), "dimmed");
    assert!(err(&mut s, json!({"angle": 30})).contains("crease, partial cut or perforation"));
    assert_eq!(crate::cmd::selection_fold_angles(&s).lines, 0);
    let r = s.execute("addon.packaging.roles", &json!({})).unwrap();
    assert_eq!((r["foldLines"].as_u64(), &r["foldAngle"]), (Some(0), &Value::Null), "{r}");
    assert_eq!(stored(&s, compound), None);
    // With its compound, given or selected, the locked path is part of the compound, not skipped.
    let r = s.execute(SET, &json!({"angle": 20, "ids": [compound, a]})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64()), (Some(1), Some(0)), "{r}");
    s.execute("select.set", &json!({"ids": [compound, a]})).unwrap();
    let r = s.execute(SET, &json!({"angle": 25})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64()), (Some(1), Some(0)), "{r}");
    s.execute(SET, &json!({"reset": true, "ids": [compound]})).unwrap();
    // The compound named by id takes the angle; its other path, not locked, names it too.
    let r = s.execute(SET, &json!({"angle": 30, "ids": [b]})).unwrap();
    assert_eq!((r["changed"].as_u64(), &r["ids"]), (Some(1), &json!([compound])), "{r}");
    assert_eq!(stored(&s, compound).as_deref(), Some("30"));
}

#[test]
fn a_damaged_angle_reads_as_none_of_its_own() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    let mut n = s.doc().unwrap().doc.node(NodeId(creases[0])).cloned().unwrap();
    for (text, want) in [("45", Some(45.0)), (" -30.5 ", Some(-30.5)), ("wide", None), ("NaN", None), ("inf", None), ("181", None), ("", None)] {
        n.edit_attrs(|a| {
            a.data.retain(|(k, _)| k != KEY);
            a.data.push((KEY.into(), text.into()));
        });
        assert_eq!(own_angle(&n), want, "{text:?}");
    }
}

#[test]
fn an_angle_stored_as_other_text_for_the_same_number_is_unchanged() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    let id = NodeId(creases[0]);
    s.edit("Test", |d, _| {
        if let Some(n) = d.node_mut(id) {
            n.edit_attrs(|a| a.data.push((KEY.into(), "45.0".into())));
        }
        Ok(())
    })
    .unwrap();
    let before = steps(&s);
    let r = s.execute(SET, &json!({"angle": 45, "ids": [creases[0]]})).unwrap();
    assert_eq!((r["changed"].as_u64(), steps(&s)), (Some(0), before), "{r}");
    assert_eq!(stored(&s, creases[0]).as_deref(), Some("45.0"));
    s.execute(SET, &json!({"reset": true, "ids": [creases[0]]})).unwrap();
    assert_eq!(stored(&s, creases[0]), None);
    // A reset also removes text that is not an angle.
    s.edit("Test", |d, _| {
        if let Some(n) = d.node_mut(id) {
            n.edit_attrs(|a| a.data.push((KEY.into(), "wide".into())));
        }
        Ok(())
    })
    .unwrap();
    let r = s.execute(SET, &json!({"reset": true, "ids": [creases[0]]})).unwrap();
    assert_eq!((r["changed"].as_u64(), stored(&s, creases[0])), (Some(1), None), "{r}");
}

#[test]
fn a_line_keeps_its_own_angle_through_a_role_change() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    s.execute(SET, &json!({"angle": 45, "ids": [creases[0]]})).unwrap();
    role(&mut s, "partialCut", &[creases[0]]);
    let r = s.execute("addon.packaging.roles", &json!({"ids": [creases[0]]})).unwrap();
    assert_eq!((r["foldAngle"].as_f64(), r["ownAngles"].as_u64()), (Some(45.0), Some(1)), "{r}");
    role(&mut s, "none", &[creases[0]]);
    assert_eq!(stored(&s, creases[0]).as_deref(), Some("45"));
    role(&mut s, "crease", &[creases[0]]);
    let r = s.execute("addon.packaging.roles", &json!({"ids": [creases[0]]})).unwrap();
    assert_eq!((r["foldAngle"].as_f64(), r["ownAngles"].as_u64()), (Some(45.0), Some(1)), "{r}");
}

#[test]
fn native_save_and_preserve_editing_keep_the_angle_and_svg_leaves_it_out() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    s.execute(SET, &json!({"angle": 120, "ids": [creases[0]]})).unwrap();
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = svg["text"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| String::from_utf8_lossy(&vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).into_owned());
    assert!(svg.contains("<svg"), "{svg}");
    assert!(!svg.contains("fold-angle"), "{svg}");
    // An SVG with Preserve Editing reopens as the document it carries, the angle with it.
    let editing = s.execute("document.serialize", &json!({"format": "svg", "preserveEditing": true})).unwrap();
    let editing = editing["text"].as_str().unwrap().to_string();
    let mut t = Session::new();
    t.execute("document.open", &json!({"name": "box.svg", "dataBase64": vectorcraft_format::base64_encode(editing.as_bytes())})).unwrap();
    assert_eq!(stored(&t, creases[0]).as_deref(), Some("120"));
    let saved = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap();
    s.execute("document.open", &json!({"name": "box.vectorcraft", "dataBase64": saved["dataBase64"]})).unwrap();
    assert_eq!(stored(&s, creases[0]).as_deref(), Some("120"));
    let r = s.execute("addon.packaging.fold", &json!({})).unwrap();
    let angles: Vec<f64> = r["hinges"].as_array().unwrap().iter().map(|h| h["angle"].as_f64().unwrap()).collect();
    assert!(angles.contains(&120.0), "{r}");
}

#[test]
fn a_tray_wall_with_its_own_angle_stands_at_that_angle() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    // The top wall (y 100 to 140) folds at 45°.
    s.execute(SET, &json!({"angle": 45, "ids": [creases[0]]})).unwrap();
    let r = s.execute("addon.packaging.fold", &json!({})).unwrap();
    let base = r["base"].as_u64().unwrap() as usize;
    let mut angles: Vec<f64> = r["hinges"].as_array().unwrap().iter().map(|h| h["angle"].as_f64().unwrap()).collect();
    angles.sort_by(f64::total_cmp);
    assert_eq!(angles, [45.0, 90.0, 90.0, 90.0]);
    for (i, p) in r["panels"].as_array().unwrap().iter().enumerate().filter(|(i, _)| *i != base) {
        let z = vector(&p["frame"]["z"]);
        let top = top_wall(p);
        let tilt = if top { std::f64::consts::FRAC_1_SQRT_2 } else { 0.0 };
        assert!((z[2] - tilt).abs() < 1e-9, "panel {i}: {p}");
    }
}

#[test]
fn a_negative_angle_folds_a_crease_toward_the_print() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    s.execute(SET, &json!({"angle": -90, "ids": [creases[0]]})).unwrap();
    let r = s.execute("addon.packaging.fold", &json!({})).unwrap();
    let base = r["base"].as_u64().unwrap() as usize;
    let mut up = 0;
    for p in r["panels"].as_array().unwrap().iter().enumerate().filter(|(i, _)| *i != base).map(|(_, p)| p) {
        // The top wall folds toward the print, so its printed side faces back over the base
        // (toward -y in the y-up frame); the other walls stand square to the base as before.
        let top = top_wall(p);
        let z = vector(&p["frame"]["z"]);
        if top {
            assert!((z[1] + 1.0).abs() < 1e-9, "the top wall's print faces the base: {p}");
            up += 1;
        } else {
            assert!(z[2].abs() < 1e-9, "{p}");
        }
    }
    assert_eq!(up, 1);
}

#[test]
fn roles_reports_the_angle_the_selected_fold_lines_share() {
    let mut s = session();
    let (outline, creases) = tray(&mut s);
    let r = s.execute("addon.packaging.roles", &json!({"ids": [creases[0], creases[1]]})).unwrap();
    assert_eq!((r["foldAngle"].as_f64(), r["ownAngles"].as_u64()), (Some(90.0), Some(0)), "the role's angle: {r}");
    s.execute(SET, &json!({"angle": 30, "ids": [creases[0]]})).unwrap();
    let r = s.execute("addon.packaging.roles", &json!({"ids": [creases[0], creases[1]]})).unwrap();
    assert_eq!((r["foldAngle"].clone(), r["ownAngles"].as_u64()), (Value::Null, Some(1)), "they differ: {r}");
    let r = s.execute("addon.packaging.roles", &json!({"ids": [creases[0], outline]})).unwrap();
    assert_eq!((r["foldAngle"].as_f64(), r["ownAngles"].as_u64()), (Some(30.0), Some(1)), "a cut line has no angle: {r}");
    let r = s.execute("addon.packaging.roles", &json!({"ids": [outline]})).unwrap();
    assert!(r["foldAngle"].is_null(), "{r}");
    // A partial cut folds at -90° unless it has an angle of its own.
    role(&mut s, "partialCut", &[creases[2]]);
    let r = s.execute("addon.packaging.roles", &json!({"ids": [creases[2]]})).unwrap();
    assert_eq!((r["foldAngle"].as_f64(), r["foldLines"].as_u64()), (Some(-90.0), Some(1)), "{r}");
}

#[test]
fn roles_leaves_locked_fold_lines_out_of_the_angle_as_set_fold_angle_does() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    s.execute(SET, &json!({"angle": 45, "ids": [creases[1]]})).unwrap();
    s.execute("select.set", &json!({"ids": [creases[0], creases[1]]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let group = s.doc().unwrap().selection.objects[0].0;
    s.execute("select.set", &json!({"ids": [creases[1]]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let r = s.execute("addon.packaging.roles", &json!({"ids": [group]})).unwrap();
    assert_eq!(r["roles"][1]["count"].as_u64(), Some(2), "both are counted: {r}");
    assert_eq!((r["foldLines"].as_u64(), r["foldAngle"].as_f64(), r["ownAngles"].as_u64()), (Some(1), Some(90.0), Some(0)), "{r}");
    // So Reset has nothing to change, as the panel shows it.
    let r = s.execute(SET, &json!({"reset": true, "ids": [group]})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64()), (Some(0), Some(1)), "{r}");
    let (lines, angle, own) = {
        let f = fold_angles(&s.doc().unwrap().doc, &[NodeId(group)]);
        (f.lines, f.angle, f.own)
    };
    assert_eq!((lines, angle, own), (1, Some(90.0), 0));
}
