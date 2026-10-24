//! Type › Typst… (`type.typst.insert`, `type.typst.edit`, `type.typst.info`) with the `typst`
//! program: each test is skipped when it isn't installed.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::Rect;

use super::*;

/// Is the `typst` program there to run?
fn have_typst() -> bool {
    let program = std::env::var(cmd::typst::PROGRAM_VAR).unwrap_or_else(|_| "typst".into());
    let found = std::process::Command::new(program).arg("--version").output().is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("typst isn't installed: skipped");
    }
    found
}

fn session() -> Session {
    let mut s = Session::new();
    s.prefs.enable_typst = true;
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn bounds(s: &Session, id: u64) -> Rect {
    let d = &s.doc().unwrap().doc;
    d.node(NodeId(id)).unwrap().geometric_bounds_with(&d.symbol_boxes()).unwrap()
}

fn id_of(r: &Value) -> u64 {
    r["id"].as_u64().unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn nothing_runs_until_enable_typst_is_on() {
    let mut s = session();
    s.prefs.enable_typst = false;
    let e = s.execute("type.typst.insert", &json!({"source": "x"})).unwrap_err();
    assert_eq!(e.to_string(), "Typst is off: turn on Enable Typst in Preferences › Type");
    // Off by default, and the preference turns it on.
    assert!(!Prefs::default().enable_typst);
    s.execute("prefs.set", &json!({"key": "enableTypst", "value": true})).unwrap();
    assert!(s.prefs.enable_typst);
}

#[test]
fn confined_automation_cant_turn_typst_on() {
    let dir = std::env::temp_dir().join(format!("vectorcraft-typst-confined-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let roots = crate::file_access::AutomationRoots::new(Some(&dir), None).unwrap();
    let mut s = session();
    s.prefs.enable_typst = false;
    crate::file_access::confine(roots.as_ref(), || {
        let e = s.execute("prefs.set", &json!({"key": "enableTypst", "value": true})).unwrap_err();
        assert!(e.to_string().contains("Enable Typst"), "{e}");
        let mut p = s.prefs.clone();
        p.enable_typst = true;
        s.apply_prefs(p);
        assert!(!s.prefs.enable_typst);
        // Once the user turned it on, automation may turn it off.
        s.prefs.enable_typst = true;
        s.execute("prefs.set", &json!({"key": "enableTypst", "value": false})).unwrap();
        assert!(!s.prefs.enable_typst);
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn insert_places_paths_that_keep_their_source() {
    if !have_typst() {
        return;
    }
    let mut s = session();
    let r = s.execute("type.typst.insert", &json!({"source": "$ x^2 + y^2 = z^2 $", "at": [100, 80]})).unwrap();
    let id = id_of(&r);
    let d = &s.doc().unwrap().doc;
    let g = d.node(NodeId(id)).unwrap();
    assert!(matches!(g.kind, NodeKind::Group { .. }));
    assert_eq!(g.kind_label(), "Typst");
    // Glyphs are plain paths: no symbols join the document.
    assert!(d.symbols.is_empty());
    let mut paths = 0;
    g.walk(&mut |n| {
        assert!(!matches!(n.kind, NodeKind::SymbolInstance { .. }));
        paths += matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) as usize;
    });
    assert!(paths >= 9, "x 2 + y 2 = z 2: {paths} paths");
    assert_eq!(s.doc().unwrap().selection.objects, vec![NodeId(id)]);
    let b = bounds(&s, id);
    // Centred on `at` (the page is; the ink lies a little off its centre).
    assert!((b.center().x - 100.0).abs() < b.width() / 4.0 && (b.center().y - 80.0).abs() < b.height(), "{b:?}");
    let info = s.execute("type.typst.info", &json!({})).unwrap();
    assert_eq!(info["source"], "$ x^2 + y^2 = z^2 $");
    assert_eq!(info["fontSize"], 11.0);
    // One undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(NodeId(id)).is_none());
}

#[test]
fn a_bigger_font_size_typesets_bigger() {
    if !have_typst() {
        return;
    }
    let mut s = session();
    let a = id_of(&s.execute("type.typst.insert", &json!({"source": "Hello"})).unwrap());
    let b = id_of(&s.execute("type.typst.insert", &json!({"source": "Hello", "fontSize": 22})).unwrap());
    let (a, b) = (bounds(&s, a).width(), bounds(&s, b).width());
    assert!((b / a - 2.0).abs() < 0.05, "{a} → {b}");
}

#[test]
fn edit_keeps_the_objects_place_scale_and_rotation() {
    if !have_typst() {
        return;
    }
    let mut s = session();
    let id = id_of(&s.execute("type.typst.insert", &json!({"source": "$ a $", "at": [200, 150]})).unwrap());
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    s.execute("object.move", &json!({"dx": 30, "dy": 10})).unwrap();
    let before = bounds(&s, id);
    let r = s.execute("type.typst.edit", &json!({"source": "$ a + b + c $"})).unwrap();
    let new = id_of(&r);
    let after = bounds(&s, new);
    // The centre stays; the art is twice the size it typesets at.
    assert!(close(after.center().y, before.center().y) && (after.center().x - before.center().x).abs() < 0.5, "{before:?} → {after:?}");
    let mut fresh = session();
    let plain = id_of(&fresh.execute("type.typst.insert", &json!({"source": "$ a + b + c $"})).unwrap());
    let natural = bounds(&fresh, plain);
    assert!((after.height() / natural.height() - 2.0).abs() < 0.01, "{natural:?} → {after:?}");
    // What isn't given stays: the edit is selected, and its info is the new source.
    assert_eq!(s.doc().unwrap().selection.objects, vec![NodeId(new)]);
    assert_eq!(s.execute("type.typst.info", &json!({})).unwrap()["source"], "$ a + b + c $");
    // Rotated, the edit stays rotated.
    s.execute("object.rotate", &json!({"angle": 90})).unwrap();
    let tall = bounds(&s, new);
    let r = s.execute("type.typst.edit", &json!({"fontSize": 11})).unwrap();
    let b = bounds(&s, id_of(&r));
    assert!(b.height() > b.width() && close(b.height(), tall.height()), "{tall:?} → {b:?}");
}

#[test]
fn the_source_is_saved_with_the_document() {
    if !have_typst() {
        return;
    }
    let mut s = session();
    let id = id_of(&s.execute("type.typst.insert", &json!({"source": "#emph[hi]", "preamble": "#set text(fill: red)", "fontSize": 14})).unwrap());
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let doc = vectorcraft_format::load(&bytes).unwrap();
    let t = doc.node(NodeId(id)).unwrap().extras().typst.clone().unwrap();
    assert_eq!((t.source.as_str(), t.preamble.as_str(), t.font_size), ("#emph[hi]", "#set text(fill: red)", 14.0));
}

#[test]
fn typst_errors_name_the_line_of_the_source() {
    if !have_typst() {
        return;
    }
    let mut s = session();
    let e = s.execute("type.typst.insert", &json!({"source": "ok\n$ x^ $", "preamble": "#let a = 1"})).unwrap_err().to_string();
    assert!(e.contains("line 2:"), "{e}");
    let e = s.execute("type.typst.insert", &json!({"source": "ok", "preamble": "#nope"})).unwrap_err().to_string();
    assert!(e.contains("preamble line 1:"), "{e}");
    // Nothing was added.
    assert!(s.execute("type.typst.info", &json!({})).is_err());
}

#[test]
fn bad_params_fail_before_typesetting() {
    let mut s = session();
    for p in [json!({}), json!({"source": "  "}), json!({"source": 3}), json!({"source": "a", "fontSize": 0}), json!({"source": "a", "at": [1]})] {
        assert!(s.execute("type.typst.insert", &p).is_err(), "{p}");
    }
    // Edit and info need a Typst object.
    assert!(s.execute("type.typst.edit", &json!({"source": "a"})).is_err());
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    assert!(s.execute("type.typst.info", &json!({})).is_err());
}

/// The node `id` of the active document, to change as a file could have.
fn node_mut(s: &mut Session, id: u64) -> &mut vectorcraft_doc::Node {
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(NodeId(id)).unwrap()
}

#[test]
fn edit_keeps_what_the_group_itself_has() {
    if !have_typst() {
        return;
    }
    let mut s = session();
    let id = id_of(&s.execute("type.typst.insert", &json!({"source": "$ a $"})).unwrap());
    {
        let n = node_mut(&mut s, id);
        n.name = Some("Formula".into());
        n.opacity = 0.5;
        n.isolate = true;
        n.extras_mut().attrs = Some(Box::new(vectorcraft_doc::ObjectAttributes { note: "eq. 3".into(), ..Default::default() }));
        n.extras_mut().graphic_style = Some(7);
    }
    let new = id_of(&s.execute("type.typst.edit", &json!({"source": "$ b $"})).unwrap());
    let d = &s.doc().unwrap().doc;
    let n = d.node(NodeId(new)).unwrap();
    assert_eq!((n.name.as_deref(), n.opacity, n.isolate), (Some("Formula"), 0.5, true));
    assert_eq!(n.extras().attrs.as_ref().map(|a| a.note.as_str()), Some("eq. 3"));
    assert_eq!(n.extras().graphic_style, Some(7));
    assert_eq!(n.extras().typst.as_ref().map(|t| t.source.as_str()), Some("$ b $"));
}

#[test]
fn a_files_spec_is_checked_before_typesetting() {
    if !have_typst() {
        return;
    }
    let mut s = session();
    let id = id_of(&s.execute("type.typst.insert", &json!({"source": "$ a $"})).unwrap());
    // A size Typst can't use is typeset at the default.
    node_mut(&mut s, id).extras_mut().typst.as_mut().unwrap().font_size = f64::NAN;
    let new = id_of(&s.execute("type.typst.edit", &json!({})).unwrap());
    assert_eq!(s.execute("type.typst.info", &json!({})).unwrap()["fontSize"], cmd::typst::DEFAULT_FONT_SIZE);
    // A source too long to typeset is refused, unless the edit replaces it.
    node_mut(&mut s, new).extras_mut().typst.as_mut().unwrap().source = "a".repeat(vectorcraft_doc::typst::MAX_TYPST_SOURCE + 1);
    let e = s.execute("type.typst.edit", &json!({})).unwrap_err().to_string();
    assert!(e.contains("longer than"), "{e}");
    s.execute("type.typst.edit", &json!({"source": "$ c $"})).unwrap();
}

#[test]
fn confined_automation_keeps_typst_out_of_folders_it_may_not_read() {
    if !have_typst() {
        return;
    }
    let base = std::env::temp_dir().join(format!("vectorcraft-typst-roots-{}", std::process::id()));
    let (docs, allowed) = (base.join("docs"), base.join("allowed"));
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::write(docs.join("secret.txt"), "secret").unwrap();
    let mut s = session();
    s.doc_mut().unwrap().path = Some(docs.join("figure.vectorcraft").to_string_lossy().into_owned());
    let read = json!({"source": "#read(\"secret.txt\")"});
    // Unconfined (the person at the keyboard), the document's folder is Typst's root.
    s.execute("type.typst.insert", &read).unwrap();
    // Confined to another folder, it isn't.
    let roots = crate::file_access::AutomationRoots::new(Some(&allowed), None).unwrap();
    let r = crate::file_access::confine(roots.as_ref(), || s.execute("type.typst.insert", &read));
    assert!(r.is_err(), "{r:?}");
    // Nor confined to a folder that holds it: a link in it could lead anywhere.
    let roots = crate::file_access::AutomationRoots::new(Some(&base), None).unwrap();
    let r = crate::file_access::confine(roots.as_ref(), || s.execute("type.typst.insert", &read));
    assert!(r.is_err(), "{r:?}");
    let _ = std::fs::remove_dir_all(&base);
}
