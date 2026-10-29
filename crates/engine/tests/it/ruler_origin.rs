//! #1212: ruler coordinates are saved metadata, independent of the artwork's coordinates.

use serde_json::{Value, json};
use vectorcraft_doc::RulerMode;
use vectorcraft_engine::Session;
use vectorcraft_format::{SaveOptions, load, save, save_with};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 125})).unwrap();
    s
}

fn origin(s: &mut Session) -> Value {
    s.execute("document.rulerOrigin", &json!({})).unwrap()
}

#[test]
fn changing_origin_preserves_artwork_and_selection_and_is_one_undo_step() {
    let mut s = session();
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(origin(&mut s), json!({"x": 0.0, "y": 0.0}));
    let p = json!({"x": 125.5, "y": -30.25});
    s.execute("document.setRulerOrigin", &p).unwrap();
    assert_eq!(origin(&mut s), p);
    assert_eq!(s.doc().unwrap().selection, selection);
    let mut changed = s.execute("document.json", &json!({})).unwrap();
    changed.as_object_mut().unwrap().remove("artboard_ruler_origins");
    assert_eq!(changed, serde_json::to_value(&*before).unwrap(), "only the origin changed");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let revision = s.doc().unwrap().revision;
    s.execute("document.setRulerOrigin", &p).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "same origin is a no-op");
    assert_eq!(s.doc().unwrap().revision, revision);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(&*s.doc().unwrap().doc, &*before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(origin(&mut s), p);
}

#[test]
fn reset_uses_the_requested_artboards_top_left_and_ignores_noops() {
    let mut s = session();
    // The one document zero point of global rulers (artboard rulers keep one per artboard).
    s.execute("view.globalRulers", &json!({"global": true})).unwrap();
    s.execute("artboard.new", &json!({"x": 700, "y": -250, "width": 200, "height": 150})).unwrap();
    s.execute("document.setRulerOrigin", &json!({"x": 25, "y": 50})).unwrap();
    let boards = s.doc().unwrap().doc.artboards.clone();
    s.execute("document.resetRulerOrigin", &json!({"artboard": 1})).unwrap();
    assert_eq!(origin(&mut s), json!({"x": 700.0, "y": -250.0}));
    assert_eq!(s.doc().unwrap().doc.artboards, boards);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("document.resetRulerOrigin", &json!({"artboard": 1})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    for p in [json!({"artboard": 9}), json!({"artboard": -1}), json!({"artboard": "first"})] {
        assert!(s.execute("document.resetRulerOrigin", &p).is_err());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
    s.execute("document.resetRulerOrigin", &json!({})).unwrap();
    assert_eq!(origin(&mut s), json!({"x": 0.0, "y": 0.0}));
}

#[test]
fn native_versions_and_compression_keep_the_origin_and_old_files_default_to_zero() {
    let mut s = session();
    let legacy = String::from_utf8(save(&s.doc().unwrap().doc, false)).unwrap().replacen("\"format\":\"vectorcraft\"", "\"format\":\"drawcraft\"", 1);
    assert!(serde_json::to_value(load(legacy.as_bytes()).unwrap()).unwrap().get("ruler_origin").is_none());
    let p = json!({"x": -12.5, "y": 27.75});
    s.execute("document.setRulerOrigin", &p).unwrap();
    let doc = &s.doc().unwrap().doc;
    for (version, compress) in [(1, false), (2, false), (3, false), (4, false), (4, true)] {
        let bytes = save_with(doc, &SaveOptions { version, compress, ..Default::default() }).unwrap();
        let back = load(&bytes).unwrap();
        assert_eq!(back.ruler_origin_for(0), doc.ruler_origin_for(0), "v{version}, compression {compress}");
        assert_eq!((back.rulers, &back.artboard_ruler_origins), (doc.rulers, &doc.artboard_ruler_origins), "v{version}, compression {compress}");
        assert_eq!(back.layers, doc.layers);
        assert_eq!(back.guides, doc.guides);
    }
}

#[test]
fn origin_rejects_invalid_coordinates_without_changing_absolute_command_coordinates() {
    let mut s = session();
    s.execute("document.setRulerOrigin", &json!({"x": 200, "y": 150})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for p in
        [json!({"x": 4_000_001, "y": 0}), json!({"x": 0, "y": -4_000_001}), json!({"x": "10", "y": 0}), json!({"x": 0}), json!({"x": null, "y": 0})]
    {
        assert!(s.execute("document.setRulerOrigin", &p).is_err(), "{p}");
        assert_eq!(&*s.doc().unwrap().doc, &*before);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
    s.execute("object.setBounds", &json!({"x": 80, "y": 40, "reference": 0})).unwrap();
    let bounds = s.doc().unwrap().doc.bounds_of(&s.doc().unwrap().selection.objects, false).unwrap();
    assert_eq!((bounds.x0, bounds.y0), (80.0, 40.0), "engine coordinates stay absolute");
    assert_eq!(origin(&mut s), json!({"x": 200.0, "y": 150.0}));
}

// View › Rulers › Change to Global / Artboard Rulers. Behaviour from Adobe's public help:
// https://helpx.adobe.com/illustrator/desktop/measure-and-align/grids-and-guides/about-rulers.html
// (global rulers count from the first artboard's top-left corner and stay there; artboard rulers,
// the default, from the active artboard's, and each artboard can have its own origin) and
// https://helpx.adobe.com/illustrator/desktop/measure-and-align/grids-and-guides/use-rulers.html
// (View > Rulers > Change to Artboard Rulers / Change to Global Rulers; double-click the corner
// where the rulers meet to restore the default origin).

fn two_boards() -> Session {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 700, "y": -250, "width": 200, "height": 150})).unwrap();
    s
}

fn origin_of(s: &mut Session, artboard: usize) -> Value {
    s.execute("document.rulerOrigin", &json!({"artboard": artboard})).unwrap()
}

#[test]
fn new_documents_use_artboard_rulers_that_count_from_the_active_artboard() {
    let mut s = two_boards();
    assert_eq!(s.doc().unwrap().doc.rulers, RulerMode::Artboard);
    assert_eq!(origin_of(&mut s, 0), json!({"x": 0.0, "y": 0.0}));
    assert_eq!(origin_of(&mut s, 1), json!({"x": 700.0, "y": -250.0}), "zero at the second artboard's top-left corner");
    // Moving an artboard moves its zero point with it.
    s.execute("artboard.setProps", &json!({"index": 1, "x": 650, "y": -200})).unwrap();
    assert_eq!(origin_of(&mut s, 1), json!({"x": 650.0, "y": -200.0}));
    assert!(s.execute("document.rulerOrigin", &json!({"artboard": 2})).is_err());
}

#[test]
fn global_rulers_count_from_one_zero_point_whichever_artboard_is_active() {
    let mut s = two_boards();
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("view.globalRulers", &json!({})).unwrap(), json!({"global": true}));
    assert_eq!(s.doc().unwrap().doc.rulers, RulerMode::Global);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    for i in [0, 1] {
        assert_eq!(origin_of(&mut s, i), json!({"x": 0.0, "y": 0.0}), "artboard {i}");
    }
    // Setting it explicitly to what it is is a no-op; a bad value changes nothing.
    s.execute("view.globalRulers", &json!({"global": true})).unwrap();
    assert!(s.execute("view.globalRulers", &json!({"global": "yes"})).is_err());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.rulers, RulerMode::Artboard);
    assert_eq!(origin_of(&mut s, 1), json!({"x": 700.0, "y": -250.0}));
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.execute("view.globalRulers", &json!({})).unwrap(), json!({"global": false}), "toggles back");
}

#[test]
fn each_artboard_keeps_its_own_origin_apart_from_the_global_one() {
    let mut s = two_boards();
    s.execute("document.setRulerOrigin", &json!({"x": 720, "y": -230, "artboard": 1})).unwrap();
    assert_eq!(origin_of(&mut s, 1), json!({"x": 720.0, "y": -230.0}));
    assert_eq!(origin_of(&mut s, 0), json!({"x": 0.0, "y": 0.0}), "the other artboard keeps its corner");
    // The origin is kept relative to its artboard: it follows a move.
    s.execute("artboard.setProps", &json!({"index": 1, "x": 600, "y": -250})).unwrap();
    assert_eq!(origin_of(&mut s, 1), json!({"x": 620.0, "y": -230.0}));
    // Global rulers have their own zero point, untouched by the artboard's.
    s.execute("view.globalRulers", &json!({"global": true})).unwrap();
    s.execute("document.setRulerOrigin", &json!({"x": 10, "y": 20})).unwrap();
    assert_eq!(origin_of(&mut s, 1), json!({"x": 10.0, "y": 20.0}));
    s.execute("view.globalRulers", &json!({"global": false})).unwrap();
    assert_eq!(origin_of(&mut s, 1), json!({"x": 620.0, "y": -230.0}));
    assert_eq!(origin_of(&mut s, 0), json!({"x": 0.0, "y": 0.0}));
    // Double-clicking the corner restores that artboard's top-left corner, in one undo step.
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("document.resetRulerOrigin", &json!({"artboard": 1})).unwrap();
    assert_eq!(origin_of(&mut s, 1), json!({"x": 600.0, "y": -250.0}));
    assert!(s.doc().unwrap().doc.artboard_ruler_origins.is_empty());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("document.resetRulerOrigin", &json!({"artboard": 1})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "already there: no-op");
    assert!(s.execute("document.setRulerOrigin", &json!({"x": 1, "y": 1, "artboard": 5})).is_err());
}

#[test]
fn the_ruler_mode_is_saved_and_files_from_before_it_measure_globally() {
    let mut s = two_boards();
    s.execute("document.setRulerOrigin", &json!({"x": 750, "y": -200, "artboard": 1})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let back = load(&save(&doc, false)).unwrap();
    assert_eq!((back.rulers, back.ruler_origin_for(1)), (RulerMode::Artboard, doc.ruler_origin_for(1)));
    s.execute("view.globalRulers", &json!({"global": true})).unwrap();
    let back = load(&save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(back.rulers, RulerMode::Global);
    // A file written before the choice existed has no `rulers` key: its rulers measured globally.
    let mut old = serde_json::to_value(&*doc).unwrap();
    old.as_object_mut().unwrap().remove("rulers");
    let old: vectorcraft_doc::Document = serde_json::from_value(old).unwrap();
    assert_eq!(old.rulers, RulerMode::Global);
    assert_eq!(old.ruler_origin_for(1), old.ruler_origin_or_zero());
}

#[test]
fn stored_origins_out_of_range_fall_back_to_the_artboard_corner() {
    let mut s = two_boards();
    let mut doc = s.doc().unwrap().doc.clone();
    let id = doc.artboards[1].id;
    for bad in [vectorcraft_geom::Vec2::new(f64::NAN, 0.0), vectorcraft_geom::Vec2::new(0.0, 9.0e6)] {
        std::sync::Arc::make_mut(&mut doc).artboard_ruler_origins.insert(id, bad);
        assert_eq!(doc.ruler_origin_for(1), vectorcraft_geom::Point::new(700.0, -250.0));
    }
    // An index past the artboards uses the first one; no artboard at all, the global zero point.
    assert_eq!(doc.ruler_origin_for(9), vectorcraft_geom::Point::ZERO);
    std::sync::Arc::make_mut(&mut doc).artboards.clear();
    assert_eq!(doc.ruler_origin_for(0), doc.ruler_origin_or_zero());
    let _ = origin_of(&mut s, 0);
}
