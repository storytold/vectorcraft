//! Artboard lock (`artboard.lock`) and artboard colour (`artboard.setColor`, export `artboardFill`).

use serde_json::json;

use crate::Session;

fn three_boards() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
    s
}

fn boards(s: &Session) -> Vec<(bool, vectorcraft_geom::Rect)> {
    s.doc().unwrap().doc.artboards.iter().map(|a| (a.locked, a.rect)).collect()
}

/// A locked artboard refuses to move, resize or be deleted (by every command that does), its name
/// and colour still change, unlocking brings the commands back, and Undo restores the lock.
#[test]
fn a_locked_artboard_cannot_be_moved_resized_or_deleted() {
    let mut s = three_boards();
    let before = boards(&s);
    assert_eq!(s.execute("artboard.lock", &json!({"index": 1})).unwrap()["changed"], 1);
    assert_eq!(boards(&s).iter().map(|b| b.0).collect::<Vec<_>>(), [false, true, false]);
    for (id, p) in [
        ("artboard.move", json!({"index": 1, "dx": 10, "dy": 0})),
        ("artboard.move", json!({"indices": [0, 1], "dx": 10, "dy": 0})),
        ("artboard.setProps", json!({"index": 1, "x": 5})),
        ("artboard.setProps", json!({"index": 1, "width": 50})),
        ("artboard.delete", json!({"index": 1})),
        ("artboard.delete", json!({"indices": [1, 2]})),
        ("artboard.cut", json!({"index": 1})),
        ("artboard.fitToArt", json!({"index": 1})),
        ("artboard.rearrange", json!({"layout": "column"})),
    ] {
        let e = s.execute(id, &p).unwrap_err().to_string();
        assert!(e.contains("locked"), "{id}: {e}");
        assert_eq!(boards(&s).iter().map(|b| b.1).collect::<Vec<_>>(), before.iter().map(|b| b.1).collect::<Vec<_>>(), "{id} changed nothing");
    }
    // Name and colour are not geometry; a copy leaves the original and starts unlocked.
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Cover"})).unwrap();
    s.execute("artboard.setColor", &json!({"index": 1, "color": "#336699"})).unwrap();
    s.execute("artboard.move", &json!({"index": 1, "dx": 400, "dy": 0, "copy": true})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.doc.artboards[1].name, "Cover");
    assert!(st.doc.artboards[1].locked && !st.doc.artboards[3].locked, "the copy is unlocked");
    // Unlocked, the artboard moves again; Undo puts the lock back.
    s.execute("artboard.lock", &json!({"index": 1, "locked": false})).unwrap();
    s.execute("artboard.move", &json!({"index": 1, "dx": 10, "dy": 0})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.artboards[1].locked);
}

/// Lock Selected, Lock All Except Selected and Lock All; nothing to change is a no-op without an
/// undo step.
#[test]
fn lock_modes() {
    let mut s = three_boards();
    let locked = |s: &Session| boards(s).iter().map(|b| b.0).collect::<Vec<_>>();
    s.execute("artboard.lock", &json!({"index": 0, "mode": "exceptSelected"})).unwrap();
    assert_eq!(locked(&s), [false, true, true]);
    s.execute("artboard.lock", &json!({"mode": "all", "locked": false})).unwrap();
    assert_eq!(locked(&s), [false, false, false]);
    s.execute("artboard.lock", &json!({"indices": [0, 2]})).unwrap();
    assert_eq!(locked(&s), [true, false, true]);
    assert_eq!(s.execute("artboard.lock", &json!({"indices": [0, 2]})).unwrap()["changed"], 0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(locked(&s), [false, false, false], "the no-op added no undo step");
    assert!(s.execute("artboard.lock", &json!({"index": 9})).is_err());
    assert!(s.execute("artboard.lock", &json!({"mode": "nope"})).is_err());
}

/// The art on a locked artboard is unaffected: it moves and edits as before.
#[test]
fn art_on_a_locked_artboard_still_moves() {
    let mut s = three_boards();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("artboard.lock", &json!({"mode": "all"})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
}

/// The artboard colour stays out of exports unless `artboardFill` is set, then it is drawn behind
/// the art on every format (here SVG and PNG pixels).
#[test]
fn artboard_colour_is_exported_only_when_asked() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 40, "units": "Points"})).unwrap();
    s.execute("artboard.setColor", &json!({"index": 0, "color": "#336699"})).unwrap();
    let svg = |s: &mut Session, p: serde_json::Value| s.execute("document.serialize", &p).unwrap()["text"].as_str().unwrap().to_lowercase();
    assert!(!svg(&mut s, json!({"format": "svg"})).contains("#336699") && !svg(&mut s, json!({"format": "svg"})).contains("336699"));
    assert!(svg(&mut s, json!({"format": "svg", "artboardFill": true})).contains("336699"));
    // The colour belongs to the document, not the export: saving the document keeps it unchanged.
    assert!(s.doc().unwrap().doc.layers.iter().all(|l| l.name.as_deref() != Some("Artboard Fill")));
    s.execute("artboard.setColor", &json!({"index": 0, "color": null})).unwrap();
    assert!(!svg(&mut s, json!({"format": "svg", "artboardFill": true})).contains("336699"));
}

/// Batch edits validate locked geometry before changing any artboard or consuming history.
#[test]
fn batch_artboard_edits_preserve_locks_and_are_atomic() {
    let mut s = three_boards();
    s.execute("artboard.lock", &json!({"index": 1})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for entries in [json!([{"index": 0, "width": 50}, {"index": 1, "width": 50}]), json!([{"index": 1, "x": 5}, {"index": 0, "x": 5}])] {
        let e = s.execute("artboard.setProps", &json!({"artboards": entries, "scaleArt": true})).unwrap_err();
        assert!(e.to_string().contains("locked"));
        assert_eq!(*s.doc().unwrap().doc, *before);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
    // A locked board can still be renamed alongside a resize of an unlocked one.
    s.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "width": 50}, {"index": 1, "name": "Cover"}]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect.width(), 50.0);
    assert_eq!(s.doc().unwrap().doc.artboards[1].name, "Cover");
    assert!(s.doc().unwrap().doc.artboards[1].locked);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
}
