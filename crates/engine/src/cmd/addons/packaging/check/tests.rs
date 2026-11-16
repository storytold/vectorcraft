//! Tests for Check Fold Lines: the blanks, panels, hinges and issues of fold lines drawn in a
//! document.

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

fn check(s: &mut Session, p: Value) -> Value {
    s.execute("addon.packaging.check", &p).unwrap()
}

fn codes(r: &Value) -> Vec<&str> {
    r["issues"].as_array().unwrap().iter().map(|i| i["code"].as_str().unwrap()).collect()
}

/// A tray blank: a 100 pt base with a 40 pt wall on each side, its outline a cut line and the
/// base's four sides creases → (outline, creases).
fn tray(s: &mut Session) -> (u64, Vec<u64>) {
    let outline = path(s, "M140 100 L240 100 L240 140 L280 140 L280 240 L240 240 L240 280 L140 280 L140 240 L100 240 L100 140 L140 140 Z");
    let creases = vec![path(s, "M140 140 L240 140"), path(s, "M240 140 L240 240"), path(s, "M240 240 L140 240"), path(s, "M140 240 L140 140")];
    role(s, "cut", &[outline]);
    role(s, "crease", &creases);
    s.execute("select.none", &json!({})).unwrap();
    (outline, creases)
}

#[test]
fn a_tray_checks_clean_with_five_panels_and_four_hinges() {
    let mut s = session();
    tray(&mut s);
    let steps = s.doc().unwrap().history.undo.len();
    let r = check(&mut s, json!({}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!((r["blanks"].as_u64(), r["panels"].as_u64(), r["hinges"].as_u64(), r["holes"].as_u64()), (Some(1), Some(5), Some(4), Some(0)), "{r}");
    assert!(codes(&r).is_empty(), "{r}");
    assert_eq!(r["scope"], "document");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "a check records no undo step");
}

#[test]
fn the_selection_or_ids_limit_the_check_to_their_lines() {
    let mut s = session();
    let (outline, creases) = tray(&mut s);
    s.execute("select.set", &json!({ "ids": [creases[0]] })).unwrap();
    let r = check(&mut s, json!({}));
    assert_eq!((r["scope"].as_str(), r["ok"].as_bool()), (Some("selection"), Some(false)));
    assert_eq!(codes(&r), ["noCut"]);
    assert_eq!(r["issues"][0]["message"], "No cut lines are in the selection.");
    let r = check(&mut s, json!({ "ids": [outline] }));
    assert_eq!((r["scope"].as_str(), r["ok"].as_bool(), r["panels"].as_u64()), (Some("ids"), Some(true), Some(1)), "{r}");
    let r = check(&mut s, json!({ "ids": [creases[1]] }));
    assert_eq!((r["scope"].as_str(), codes(&r)), (Some("ids"), vec!["noCut"]));
    assert_eq!(r["issues"][0]["message"], "No cut lines are among the given objects.");
    // No fold lines at all in the document.
    let mut s = session();
    let r = check(&mut s, json!({}));
    assert_eq!(codes(&r), ["noCut"]);
    assert_eq!(r["issues"][0]["message"], "No cut lines are in the document.");
}

#[test]
fn issues_name_their_lines_and_select_selects_them() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    // A slit from the outline that stops in the middle of the base.
    let slit = path(&mut s, "M190 100 L190 190");
    role(&mut s, "cut", &[slit]);
    s.execute("select.none", &json!({})).unwrap();
    let r = check(&mut s, json!({}));
    assert_eq!(r["ok"], false);
    let loose = r["issues"].as_array().unwrap().iter().find(|i| i["code"] == "looseEnd").cloned().unwrap();
    assert_eq!((loose["severity"].as_str(), loose["ids"].clone()), (Some("error"), json!([slit])), "{r}");
    assert_eq!(loose["at"], json!([190.0, 190.0]));
    assert!(s.doc().unwrap().selection.objects.is_empty(), "without select, the selection stays");
    let r = check(&mut s, json!({ "select": true }));
    let named: std::collections::BTreeSet<u64> =
        r["issues"].as_array().unwrap().iter().flat_map(|i| i["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap())).collect();
    let selected: std::collections::BTreeSet<u64> = s.doc().unwrap().selection.objects.iter().map(|id| id.0).collect();
    assert!(selected.contains(&slit) && selected == named, "{selected:?} {named:?}");
    assert!(!selected.contains(&creases[1]));
}

#[test]
fn a_clean_check_with_select_leaves_the_selection_alone() {
    let mut s = session();
    let (outline, _) = tray(&mut s);
    s.execute("select.set", &json!({ "ids": [outline] })).unwrap();
    let r = check(&mut s, json!({ "ids": [outline], "select": true }));
    assert!(codes(&r).is_empty(), "{r}");
    assert_eq!(s.doc().unwrap().selection.objects.iter().map(|id| id.0).collect::<Vec<_>>(), [outline]);
}

#[test]
fn tolerance_is_checked_and_joins_gaps_up_to_it() {
    let mut s = session();
    tray(&mut s);
    for bad in [json!(0.005), json!(10.5), json!("x"), json!(true)] {
        let e = s.execute("addon.packaging.check", &json!({ "tolerance": bad })).unwrap_err().to_string();
        assert!(e.contains("tolerance"), "{e}");
    }
    let e = s.execute("addon.packaging.check", &json!({ "select": 1 })).unwrap_err().to_string();
    assert!(e.contains("select"), "{e}");
    for edge in [json!(0.01), json!(10), json!("2 mm")] {
        assert_eq!(check(&mut s, json!({ "tolerance": edge }))["ok"], true);
    }
    // A slit that stops 0.5 pt short of a crease: a gap at a tolerance of 1 pt, a loose end at 0.1 pt.
    let mut s = session();
    tray(&mut s);
    let slit = path(&mut s, "M190 100 L190 139.5");
    role(&mut s, "cut", &[slit]);
    s.execute("select.none", &json!({})).unwrap();
    assert!(codes(&check(&mut s, json!({ "tolerance": 0.1 }))).contains(&"looseEnd"));
    let near = check(&mut s, json!({ "tolerance": 1 }));
    assert!(!codes(&near).contains(&"looseEnd"), "{near}");
}

#[test]
fn hidden_lines_are_left_out_and_lines_on_a_non_printing_layer_are_noted() {
    let mut s = session();
    let (_, creases) = tray(&mut s);
    // A hidden crease is not checked while nothing is selected: the tray has three hinges and a
    // panel joined by none.
    s.execute("layer.setProps", &json!({ "ids": [creases[0]], "visible": false })).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let r = check(&mut s, json!({}));
    assert_eq!((r["panels"].as_u64(), r["hinges"].as_u64()), (Some(4), Some(3)), "{r}");
    // The same in a selected group, and with the group named by `ids`: the hidden member is left
    // out too.
    let mut s = session();
    let (outline, creases) = tray(&mut s);
    s.execute("select.set", &json!({ "ids": [outline, creases[0], creases[1], creases[2], creases[3]] })).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.setProps", &json!({ "ids": [creases[0]], "visible": false })).unwrap();
    s.execute("select.set", &json!({ "ids": [group] })).unwrap();
    for p in [json!({}), json!({ "ids": [group] })] {
        let r = check(&mut s, p);
        assert_eq!((r["panels"].as_u64(), r["hinges"].as_u64()), (Some(4), Some(3)), "{r}");
    }
    // A cut line on a layer that does not print.
    let mut s = session();
    s.execute("layer.new", &json!({ "printable": false })).unwrap();
    let (outline, _) = tray(&mut s);
    let r = check(&mut s, json!({}));
    assert_eq!(r["ok"], true, "a warning only");
    let note = r["issues"].as_array().unwrap().iter().find(|i| i["code"] == "nonPrinting").cloned().unwrap();
    assert_eq!(note["severity"], "warning");
    assert!(note["ids"].as_array().unwrap().contains(&json!(outline)), "{note}");
    assert_eq!(note["message"], "A fold line is on a non-printing layer.");
}

#[test]
fn a_compound_path_and_a_grouped_line_are_checked() {
    let mut s = session();
    let (outline, creases) = tray(&mut s);
    // A window: a closed cut line in the base, made a compound path with nothing else.
    let window = path(&mut s, "M170 170 L210 170 L210 210 L170 210 Z");
    s.execute("select.set", &json!({ "ids": [window] })).unwrap();
    let compound = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64();
    let window = compound.unwrap_or(window);
    role(&mut s, "cut", &[window]);
    s.execute("select.set", &json!({ "ids": [outline, creases[0], creases[1], creases[2], creases[3], window] })).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let r = check(&mut s, json!({}));
    assert_eq!(r["scope"], "selection");
    assert_eq!((r["ok"].as_bool(), r["panels"].as_u64(), r["holes"].as_u64()), (Some(true), Some(5), Some(1)), "{r}");
}
