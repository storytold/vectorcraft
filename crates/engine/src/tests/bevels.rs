//! Custom bevels (`cmd/bevels.rs`): bevels read from the bevels file's symbols, listed, set on
//! Extrude & Bevel and Extrude & Bevel (Classic) by name, reloaded when the file is saved, and kept
//! with the documents that use them.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, Symbol};
use vectorcraft_geom::{Anchor, PathData, Point, SubPath};
use vectorcraft_render::effects;

use crate::cmd::bevels::{self, bevels_of};
use crate::*;

fn line(pts: &[(f64, f64)], closed: bool) -> Node {
    let pts: Vec<Point> = pts.iter().map(|&(x, y)| Point::new(x, y)).collect();
    Node::path(NodeId(0), PathData::single(SubPath::polyline(&pts, closed)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0))
}

fn add_symbol(d: &mut Document, name: &str, art: Node) {
    let art = d.reid(&art);
    d.symbols.push(Symbol { name: name.into(), art: Arc::new(art), dynamic: false });
}

/// A folder of its own for test `name`, emptied.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-bevels-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
}

#[test]
fn each_open_path_symbol_is_a_bevel_from_its_first_anchor_to_its_last() {
    let mut d = Document::new(100.0, 100.0);
    add_symbol(&mut d, "Notch", line(&[(0.0, 0.0), (20.0, 60.0), (70.0, 40.0), (100.0, 100.0)], false));
    // The same path mirrored across the page and drawn from the other side gives the same points.
    add_symbol(&mut d, "Mirrored", line(&[(300.0, 50.0), (280.0, 110.0), (230.0, 90.0), (200.0, 150.0)], false));
    // In a group of one, at another size.
    let small = line(&[(0.0, 0.0), (2.0, 6.0), (7.0, 4.0), (10.0, 10.0)], false);
    add_symbol(&mut d, "Grouped", Node::group(NodeId(0), vec![Arc::new(small)]));
    // Points past the ends are held to them.
    add_symbol(&mut d, "Overshoot", line(&[(0.0, 0.0), (-20.0, 50.0), (100.0, 120.0), (100.0, 100.0)], false));
    add_symbol(&mut d, "Closed", line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true));
    let two = Node::group(NodeId(0), vec![Arc::new(line(&[(0.0, 0.0), (1.0, 1.0)], false)), Arc::new(line(&[(0.0, 0.0), (1.0, 1.0)], false))]);
    add_symbol(&mut d, "Two paths", two);
    add_symbol(&mut d, "Upright", line(&[(0.0, 0.0), (5.0, 50.0), (0.0, 100.0)], false));
    let dense: Vec<(f64, f64)> = (0..=bevels::MAX_ANCHORS).map(|i| (i as f64, i as f64)).collect();
    add_symbol(&mut d, "Dense", line(&dense, false));
    add_symbol(&mut d, "Notch", line(&[(0.0, 0.0), (1.0, 1.0)], false));
    let read = bevels_of(&d);
    let names: Vec<&str> = read.bevels.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Notch", "Mirrored", "Grouped", "Overshoot"]);
    let notch = [(0.0, 0.0), (0.2, 0.6), (0.7, 0.4), (1.0, 1.0)];
    for b in &read.bevels[..3] {
        assert_eq!(b.points.len(), 4, "{}", b.name);
        assert!(b.points.iter().zip(notch).all(|(&a, b)| close(a, b)), "{}: {:?}", b.name, b.points);
    }
    assert!(close(read.bevels[3].points[1], (0.0, 0.5)) && close(read.bevels[3].points[2], (1.0, 1.0)), "{:?}", read.bevels[3].points);
    let skipped: Vec<(&str, &str)> = read.skipped.iter().map(|(n, r)| (n.as_str(), r.as_str())).collect();
    assert_eq!(
        skipped,
        [
            ("Closed", "its art is not one open path"),
            ("Two paths", "its art is not one open path"),
            ("Upright", "its ends are not apart both across and down"),
            ("Dense", "its path has too many anchors for a bevel"),
            ("Notch", "another symbol has this name"),
        ]
    );
}

/// A curve is cut into straight lines, and a path of many points keeps the ones that matter, at
/// most 64 in all, ends included.
#[test]
fn curves_and_long_paths_fit_in_the_points_a_bevel_holds() {
    let mut d = Document::new(100.0, 100.0);
    // A quarter circle from (0, 0) to (100, 100), bulging up and right.
    let k = 0.5523 * 100.0;
    let mut a = Anchor::corner(Point::new(0.0, 0.0));
    a.h_out = Point::new(k, 0.0);
    let mut b = Anchor::corner(Point::new(100.0, 100.0));
    b.h_in = Point::new(100.0, 100.0 - k);
    let arc = Node::path(NodeId(0), PathData::single(SubPath::new(vec![a, b], false)), Appearance::default());
    add_symbol(&mut d, "Arc", arc);
    let wavy: Vec<(f64, f64)> = (0..=3000).map(|i| (i as f64 / 30.0, i as f64 / 30.0 + 5.0 * (i as f64 / 50.0).sin())).collect();
    add_symbol(&mut d, "Wavy", line(&wavy, false));
    let read = bevels_of(&d);
    assert!(read.skipped.is_empty(), "{:?}", read.skipped);
    for b in &read.bevels {
        assert!((3..=effects::BevelProfile::MAX_POINTS).contains(&b.points.len()), "{}: {}", b.name, b.points.len());
        assert!(close(b.points[0], (0.0, 0.0)) && close(*b.points.last().unwrap(), (1.0, 1.0)), "{}", b.name);
        assert!(effects::BevelProfile::new(&b.points).is_some(), "{}", b.name);
    }
    // The arc's points lie on the circle: across first, then down.
    let arc = &read.bevels[0];
    assert!(arc.points.iter().all(|&(x, y)| (((x - 0.0).powi(2) + (y - 1.0).powi(2)).sqrt() - 1.0).abs() < 0.01), "{:?}", arc.points);
}

/// A path of many tall curves, cut into more straight lines than a bevel's path may take, is
/// skipped as too detailed.
#[test]
fn a_path_cut_into_too_many_lines_is_skipped() {
    let mut d = Document::new(100.0, 100.0);
    let anchors: Vec<Anchor> = (0..4000)
        .map(|i| {
            let p = Point::new(f64::from(i), f64::from(i) / 4.0);
            let swing = if i % 2 == 0 { 1000.0 } else { -1000.0 };
            let mut a = Anchor::corner(p);
            a.h_in = Point::new(p.x - 0.5, p.y - swing);
            a.h_out = Point::new(p.x + 0.5, p.y + swing);
            a
        })
        .collect();
    let wild = Node::path(NodeId(0), PathData::single(SubPath::new(anchors, false)), Appearance::default());
    add_symbol(&mut d, "Wild", wild);
    let read = bevels_of(&d);
    assert!(read.bevels.is_empty());
    assert_eq!(read.skipped, [("Wild".to_string(), "its path is too detailed for a bevel".to_string())]);
}

/// Simplifying the paths of one file measures at most a fixed number of point-to-line distances:
/// a path that needs more than are left is skipped as too detailed, and a short one still reads.
#[test]
fn simplifying_the_bevels_of_a_file_has_a_work_limit() {
    let mut d = Document::new(100.0, 100.0);
    let wavy: Vec<(f64, f64)> = (0..=3000).map(|i| (i as f64 / 30.0, i as f64 / 30.0 + 5.0 * (i as f64 / 50.0).sin())).collect();
    add_symbol(&mut d, "Wavy", line(&wavy, false));
    add_symbol(&mut d, "Notch", line(&[(0.0, 0.0), (20.0, 60.0), (70.0, 40.0), (100.0, 100.0)], false));
    let read = bevels::bevels_within(&d, 5_000);
    let names: Vec<&str> = read.bevels.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Notch"]);
    assert_eq!(read.skipped, [("Wavy".to_string(), "its path is too detailed for a bevel".to_string())]);
    assert_eq!(bevels_of(&d).bevels.len(), 2);
}

fn rect(s: &mut Session, x: u32) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap())
}

fn params(s: &Session, id: NodeId) -> Value {
    s.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params.clone()
}

#[test]
fn the_bevels_file_is_made_opened_and_reloaded_when_saved() {
    let dir = folder("edit");
    let file = dir.join(bevels::FILE_NAME).to_string_lossy().to_string();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = rect(&mut s, 20);
    s.attach_bevels_file(file.clone());
    let list = s.execute("threeD.bevels.list", &json!({})).unwrap();
    assert_eq!((list["bevels"].as_array().unwrap().len(), list["file"].as_str()), (0, Some(file.as_str())));
    let apply = json!({"effect": effects::EXTRUDE, "ids": [id.0], "item": null, "params": {"bevel": {"custom": "Example Bevel"}}});
    assert!(s.execute("effect.apply", &apply).unwrap_err().to_string().contains("no custom bevel Example Bevel"));
    let r = s.execute("threeD.bevels.edit", &json!({})).unwrap();
    assert_eq!(r["created"], true);
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(file.as_str()), "the file is the active document");
    let list = s.execute("threeD.bevels.list", &json!({})).unwrap();
    assert_eq!(list["bevels"][0]["name"], "Example Bevel");
    assert_eq!(list["bevels"][0]["points"].as_array().unwrap().len(), 4);
    // A second bevel drawn in the file is listed once the file is saved.
    let st = s.doc_mut().unwrap();
    add_symbol(Arc::make_mut(&mut st.doc), "Steps", line(&[(0.0, 0.0), (0.0, 50.0), (50.0, 50.0), (50.0, 100.0), (100.0, 100.0)], false));
    assert_eq!(s.execute("threeD.bevels.list", &json!({})).unwrap()["bevels"].as_array().unwrap().len(), 1);
    s.execute("document.save", &json!({})).unwrap();
    let list = s.execute("threeD.bevels.list", &json!({})).unwrap();
    let names: Vec<&str> = list["bevels"].as_array().unwrap().iter().filter_map(|b| b["name"].as_str()).collect();
    assert_eq!(names, ["Example Bevel", "Steps"]);
    assert_eq!(s.execute("threeD.bevels.reload", &json!({})).unwrap()["count"], 2);
    // Editing again brings the open file to the front, without making it anew.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(s.execute("threeD.bevels.edit", &json!({})).unwrap()["created"], false);
    assert_eq!(s.documents().len(), 2);
    // The rectangle takes the bevel with a copy of its points, in both Extrude & Bevels.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    s.execute("effect.apply", &json!({"effect": effects::EXTRUDE, "ids": [id.0], "item": null, "params": {"bevel": {"custom": "Steps"}}})).unwrap();
    let p = params(&s, id);
    assert_eq!((p["bevel"].as_str(), p[effects::BEVEL_NAME].as_str()), (Some("custom"), Some("Steps")));
    assert_eq!(p[effects::BEVEL_PROFILE], json!([[0.0, 0.0], [0.0, 0.5], [0.5, 0.5], [0.5, 1.0], [1.0, 1.0]]));
    let set = |bevel: Value| json!({"ids": [id.0], "index": 0, "item": null, "params": {"bevel": bevel}});
    s.execute("effect.setParams", &set(json!({"custom": "Example Bevel"}))).unwrap();
    assert_eq!(params(&s, id)[effects::BEVEL_NAME], "Example Bevel");
    for bad in [json!({"custom": 3}), json!({"custom": "Nope"}), json!({"name": "Steps"})] {
        let undo = s.doc().unwrap().history.undo.len();
        assert!(s.execute("effect.setParams", &set(bad.clone())).is_err(), "{bad}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{bad}: no undo step");
    }
    let classic = rect(&mut s, 200);
    s.execute(
        "effect.apply",
        &json!({"effect": effects::CLASSIC_EXTRUDE, "ids": [classic.0], "item": null, "params": {"bevel": {"custom": "Steps"}}}),
    )
    .unwrap();
    assert_eq!(params(&s, classic)[effects::BEVEL_NAME], "Steps");
    // A built-in shape again draws that shape.
    s.execute("effect.setParams", &set(json!("rounded"))).unwrap();
    assert!(matches!(effects::extrude_options(&params(&s, id)).bevel.map(|b| b.shape.id()), Some("rounded")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `effect.apply` and `effect.setParams` given a custom bevel by name record its points as
/// `savedBevel`, so a replay draws that bevel after the bevels file no longer has it.
#[test]
fn a_replay_draws_the_custom_bevel_the_command_recorded() {
    let dir = folder("replay");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let id = rect(&mut s, 20);
    s.attach_bevels_file(dir.join(bevels::FILE_NAME).to_string_lossy().to_string());
    s.execute("threeD.bevels.edit", &json!({})).unwrap();
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    let apply = json!({"effect": effects::EXTRUDE, "ids": [id.0], "item": null, "params": {"bevel": {"custom": "Example Bevel"}}});
    s.execute("effect.apply", &apply).unwrap();
    let applied = params(&s, id);
    let (cmd, recorded) = s.journal.last().unwrap().clone();
    assert_eq!(cmd, "effect.apply");
    assert_eq!(recorded[bevels::SAVED_BEVEL], applied[effects::BEVEL_PROFILE]);
    let set = json!({"ids": [id.0], "index": 0, "item": null, "params": {"bevel": {"custom": "Example Bevel"}, "bevelWidth": 9}});
    s.execute("effect.setParams", &set).unwrap();
    let edited = params(&s, id);
    let (_, set_recorded) = s.journal.last().unwrap().clone();
    assert_eq!(set_recorded[bevels::SAVED_BEVEL], applied[effects::BEVEL_PROFILE]);
    // The bevels file no longer has the bevel: by name it is refused, and the replays draw it.
    s.attach_bevels_file(dir.join("Other.vectorcraft").to_string_lossy().to_string());
    assert!(s.execute("effect.apply", &apply).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).unwrap().appearance.effects.is_empty());
    s.replay("effect.apply", &recorded).unwrap();
    assert_eq!(params(&s, id), applied);
    s.replay("effect.setParams", &set_recorded).unwrap();
    assert_eq!(params(&s, id), edited);
    // Points given with the name are used in place of the file's.
    let notch = json!([[0.0, 0.0], [0.2, 0.6], [0.7, 0.4], [1.0, 1.0]]);
    let mut given = set.clone();
    given["savedBevel"] = notch.clone();
    s.execute("effect.setParams", &given).unwrap();
    assert_eq!((&params(&s, id)[effects::BEVEL_PROFILE], &params(&s, id)[effects::BEVEL_NAME]), (&notch, &json!("Example Bevel")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_document_keeps_its_custom_bevels_without_the_bevels_file() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let id = rect(&mut s, 100);
    let notch = json!([[0, 0], [0.2, 0.6], [0.7, 0.4], [1, 1]]);
    let custom = json!({"bevel": "custom", "bevelName": "Notch", "bevelProfile": notch, "bevelWidth": 12, "bevelHeight": 12});
    s.execute("effect.apply", &json!({"effect": effects::EXTRUDE, "ids": [id.0], "item": null, "params": custom})).unwrap();
    let ink = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    let with_bevel = ink(&s);
    // Saved and opened in a session that has no bevels file: the bevel is drawn the same.
    let saved = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "a.vectorcraft", "dataBase64": saved})).unwrap();
    assert_eq!(params(&o, id)[effects::BEVEL_PROFILE], params(&s, id)[effects::BEVEL_PROFILE]);
    assert_eq!(ink(&o).pixels, with_bevel.pixels);
    // The classic chamfer draws differently.
    let set = json!({"ids": [id.0], "index": 0, "item": null, "params": {"bevel": "classic"}});
    o.execute("effect.setParams", &set).unwrap();
    assert_ne!(ink(&o).pixels, with_bevel.pixels);
    o.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(ink(&o).pixels, with_bevel.pixels);
    // Points that do not make a bevel are refused before the document changes.
    let bad = json!({"ids": [id.0], "index": 0, "item": null, "params": {"bevelProfile": [[0, 0], [0.5, 2], [1, 1]]}});
    assert!(o.execute("effect.setParams", &bad).unwrap_err().to_string().contains("bevelProfile must be"));
}

#[test]
fn both_extrudes_take_a_custom_bevels_points_and_refuse_a_name_that_is_not_a_string() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let notch = json!([[0, 0], [0.2, 0.6], [0.7, 0.4], [1, 1]]);
    for effect in [effects::EXTRUDE, effects::CLASSIC_EXTRUDE] {
        let id = rect(&mut s, 100);
        let apply = |name: Value| json!({"effect": effect, "ids": [id.0], "item": null, "params": {"bevel": "custom", "bevelName": name, "bevelProfile": notch}});
        let before = s.doc().unwrap().doc.clone();
        let e = s.execute("effect.apply", &apply(json!(3))).unwrap_err().to_string();
        assert!(e.contains("bevelName must be a string"), "{effect}: {e}");
        assert_eq!(s.doc().unwrap().doc, before, "{effect}");
        s.execute("effect.apply", &apply(json!("Notch"))).unwrap();
        assert_eq!((&params(&s, id)[effects::BEVEL_PROFILE], &params(&s, id)[effects::BEVEL_NAME]), (&notch, &json!("Notch")), "{effect}");
    }
}

#[test]
fn without_a_bevels_file_editing_is_refused_and_none_are_listed() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let list = s.execute("threeD.bevels.list", &json!({})).unwrap();
    assert_eq!((list["bevels"].as_array().unwrap().len(), &list["file"]), (0, &Value::Null));
    assert!(s.execute("threeD.bevels.edit", &json!({})).is_err());
    assert!(s.execute("threeD.bevels.reload", &json!({})).is_err());
    // A file that is not a document lists no bevels and says why.
    let dir = folder("unreadable");
    let file = dir.join(bevels::FILE_NAME);
    std::fs::write(&file, b"not a document").unwrap();
    s.attach_bevels_file(file.to_string_lossy().to_string());
    let list = s.execute("threeD.bevels.list", &json!({})).unwrap();
    assert!(list["error"].is_string(), "{list}");
    assert!(s.execute("threeD.bevels.reload", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Automation roots that grant reading `read` alone.
fn read_roots(read: &[&std::path::Path]) -> Arc<crate::file_access::AutomationRoots> {
    crate::file_access::AutomationRoots::new(read, &[]).unwrap().unwrap()
}

#[test]
fn an_agent_outside_the_bevels_files_folder_sees_its_file_name_alone_and_cannot_reload_it() {
    let dir = folder("roots-list");
    let (prefs, other) = (dir.join("prefs"), dir.join("other"));
    std::fs::create_dir_all(&prefs).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let file = prefs.join(bevels::FILE_NAME);
    std::fs::write(&file, b"not a drawing").unwrap();
    let (file, prefs_text) = (file.to_string_lossy().to_string(), prefs.to_string_lossy().to_string());
    let mut s = Session::new();
    s.attach_bevels_file(file.clone());
    let (outside, inside) = (read_roots(&[&other]), read_roots(&[&prefs]));
    let list = crate::file_access::confine(Some(&outside), || s.execute("threeD.bevels.list", &json!({}))).unwrap();
    assert_eq!(list["file"], Value::Null, "{list}");
    let error = list["error"].as_str().unwrap();
    assert!(error.starts_with(bevels::FILE_NAME) && !error.contains(&prefs_text), "{error}");
    let list = crate::file_access::confine(Some(&inside), || s.execute("threeD.bevels.list", &json!({}))).unwrap();
    assert_eq!(list["file"].as_str(), Some(file.as_str()));
    assert!(list["error"].as_str().unwrap().starts_with(&file), "{list}");
    // A good file now: an agent outside the folder cannot have it read.
    std::fs::write(&file, vectorcraft_format::save_file(&bevels::starter())).unwrap();
    let e = crate::file_access::confine(Some(&outside), || s.execute("threeD.bevels.reload", &json!({}))).unwrap_err().to_string();
    assert!(e.contains("outside the read root") && e.ends_with(bevels::FILE_NAME) && !e.contains(&prefs_text), "{e}");
    let list = s.execute("threeD.bevels.list", &json!({})).unwrap();
    assert!(list["bevels"].as_array().unwrap().is_empty() && list["error"].is_string(), "not read again: {list}");
    let r = crate::file_access::confine(Some(&inside), || s.execute("threeD.bevels.reload", &json!({}))).unwrap();
    assert_eq!(r["count"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_agent_edits_the_bevels_file_only_inside_its_read_roots() {
    let dir = folder("roots-edit");
    let (prefs, other) = (dir.join("prefs"), dir.join("other"));
    std::fs::create_dir_all(&other).unwrap();
    let file = prefs.join(bevels::FILE_NAME).to_string_lossy().to_string();
    let prefs_text = prefs.to_string_lossy().to_string();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.attach_bevels_file(file.clone());
    let first_doc = s.doc().unwrap().uid;
    let outside = read_roots(&[&other]);
    let edit = |s: &mut Session, roots: &Arc<crate::file_access::AutomationRoots>| {
        crate::file_access::confine(Some(roots), || s.execute("threeD.bevels.edit", &json!({})))
    };
    // Refused before the folder or the file is made, or anything is opened.
    let e = edit(&mut s, &outside).unwrap_err().to_string();
    assert!(e.ends_with(bevels::FILE_NAME) && !e.contains(&prefs_text), "{e}");
    assert!(!prefs.exists(), "no folder made");
    assert_eq!((s.documents().len(), s.doc().unwrap().uid), (1, first_doc));
    // From the 3D and Materials panel (no roots), the file is made and opened.
    assert_eq!(s.execute("threeD.bevels.edit", &json!({})).unwrap()["created"], true);
    assert_eq!(s.documents().len(), 2);
    // Open already: still not brought to the front for an agent outside the folder.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert!(edit(&mut s, &outside).is_err());
    assert_eq!((s.documents().len(), s.doc().unwrap().uid), (2, first_doc));
    // Inside the read roots: opened under them, with its path.
    let inside = read_roots(&[&prefs]);
    let mut agent = Session::new();
    agent.attach_bevels_file(file.clone());
    let r = edit(&mut agent, &inside).unwrap();
    assert_eq!((r["path"].as_str(), &r["created"]), (Some(file.as_str()), &json!(false)));
    let st = agent.doc().unwrap();
    assert_eq!(st.path.as_deref(), Some(file.as_str()));
    assert!(st.opened_by.as_ref().is_some_and(|r| Arc::ptr_eq(r, &inside)), "opened under the agent's roots");
    // A missing file inside the roots is still made first.
    let fresh = dir.join("fresh").join(bevels::FILE_NAME).to_string_lossy().to_string();
    let mut agent = Session::new();
    agent.attach_bevels_file(fresh.clone());
    let r = edit(&mut agent, &read_roots(&[&dir])).unwrap();
    assert_eq!(r["created"], true);
    assert!(std::path::Path::new(&fresh).is_file());
    assert_eq!(agent.doc().unwrap().path.as_deref(), Some(fresh.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}
