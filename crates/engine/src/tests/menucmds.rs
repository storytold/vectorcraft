//! Menu long-tail commands (Object/Edit/Select/Type/View/File), driven through `Session::execute`.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ColorMode, LiveShape, Node, NodeKind, TextKind};
use vectorcraft_geom::{Rect, shapes};

use crate::tests_large_edits::{layer, run_soon, square, undo_gives_back};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn text(s: &mut Session, t: &str) -> NodeId {
    id_of(&s.execute("text.create", &json!({"x": 50, "y": 50, "text": t})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    node(s, id).geometric_bounds().unwrap()
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

/// A placed 64 × 32 px PNG whose header reads but whose pixel data is cut short
/// ([`cut_pixels`]): the renderer can't decode it and draws nothing.
pub(crate) fn place_undecodable_png(s: &mut Session) -> NodeId {
    place_undecodable_png_sized(s, 64, 32)
}

/// Places a PNG like [`place_undecodable_png`] does, at `w` × `h` px. PNGs of different sizes are
/// different images, each stored under its own key.
pub(crate) fn place_undecodable_png_sized(s: &mut Session, w: u32, h: u32) -> NodeId {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let r = s.execute("file.place", &json!({"name": "cut.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    cut_pixels(s, id);
    id
}

/// Cut the pixel data of the PNG that placed image `id` shows short, right after its first `IDAT`
/// chunk's type, as a document saved with a damaged image holds it: the header still reads, but
/// the renderer can't decode the image. Place refuses such a file, so tests cut a placed one. The
/// image is stored under the key of its new bytes.
pub(crate) fn cut_pixels(s: &mut Session, id: NodeId) {
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let Some(NodeKind::Image(im)) = doc.node(id).map(|n| &n.kind) else { panic!("{id:?} is not an image") };
    let old = im.key.clone();
    let mut png = doc.images[&old].bytes.to_vec();
    let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
    png.truncate(idat + 8);
    let cut = vectorcraft_doc::ImageBlob::new("image/png", png);
    let key = cut.content_key();
    doc.images.insert(key.clone(), cut);
    if let Some(Node { kind: NodeKind::Image(im), .. }) = doc.node_mut(id) {
        im.key = key;
    }
    let mut shown = false;
    doc.visit_images(|_, im| shown |= im.key == old);
    if !shown {
        doc.images.remove(&old);
    }
}

/// Whether reply `r` warns that an image couldn't be decoded, as raster exports do.
pub(crate) fn warns_undecoded(r: &Value) -> bool {
    r["warning"].as_str() == Some("an image couldn't be decoded and is missing from the picture")
}

fn fill(s: &mut Session, id: NodeId, hex: &str) {
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": hex})).unwrap();
}

fn fill_of(s: &Session, id: NodeId) -> Color {
    node(s, id).appearance.fill_paint().color().unwrap()
}

fn plain(s: &Session, id: NodeId) -> String {
    match &node(s, id).kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn selected(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().selection.objects.clone()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// `cmd` with `p` fails with the error for `key`, which `p` gives as something other than a number.
fn refuses_non_number(s: &mut Session, cmd: &str, p: &Value, key: &str) {
    let r = s.execute(cmd, p);
    let want = format!("`{key}` must be a number, not {}", p[key]);
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
}

/// `cmd` with `p` fails with the error for `key`, which `p` gives as something other than true or false.
fn refuses_non_boolean(s: &mut Session, cmd: &str, p: &Value, key: &str) {
    let r = s.execute(cmd, p);
    let want = format!("`{key}` must be true or false");
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
}

// ---------- Object → Lock / Hide ----------

#[test]
fn lock_all_artwork_above_only_overlapping_objects_above() {
    let mut s = session();
    let below = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    let over = rect(&mut s, 30.0, 30.0, 50.0, 50.0);
    let far = rect(&mut s, 400.0, 400.0, 20.0, 20.0);
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    let r = s.execute("object.lock.above", &json!({})).unwrap();
    assert_eq!(r["count"], 1);
    assert!(node(&s, over).locked);
    assert!(!node(&s, below).locked && !node(&s, far).locked && !node(&s, a).locked);
    assert_eq!(undo_len(&s), n + 1);
}

#[test]
fn hide_all_artwork_above() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    let over = rect(&mut s, 30.0, 30.0, 50.0, 50.0);
    sel(&mut s, &[a]);
    s.execute("object.hide.above", &json!({})).unwrap();
    assert!(!node(&s, over).visible);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, over).visible);
}

fn key(s: &Session) -> Option<NodeId> {
    s.doc().unwrap().selection.key
}

/// Lock › All Artwork Above deselects the selected objects it locks: here the key object, in a
/// group above the other selected object. Undo selects it again as the key object.
#[test]
fn lock_all_artwork_above_deselects_the_objects_it_locks() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    sel(&mut s, &[b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    sel(&mut s, &[a, b]);
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    s.execute("object.lock.above", &json!({})).unwrap();
    assert!(node(&s, g).locked);
    assert_eq!((selected(&s), key(&s)), (vec![a], None));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!node(&s, g).locked);
    assert_eq!((selected(&s), key(&s)), (vec![a, b], Some(b)));
}

/// Hide › All Artwork Above deselects the selected objects it hides (here one in a sublayer above)
/// and selected objects that were hidden before. The one object left can't be the key object, so
/// the key goes too; undo brings both back.
#[test]
fn hide_all_artwork_above_deselects_the_objects_it_hides() {
    let mut s = session();
    let hidden = rect(&mut s, 400.0, 400.0, 20.0, 20.0);
    s.execute("layer.setProps", &json!({"id": hidden.0, "visible": false})).unwrap();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let sub = id_of(&s.execute("layer.newSublayer", &json!({})).unwrap());
    let b = rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    sel(&mut s, &[hidden, a, b]);
    s.execute("select.key", &json!({"id": a.0})).unwrap();
    s.execute("object.hide.above", &json!({})).unwrap();
    assert!(!node(&s, sub).visible);
    assert_eq!((selected(&s), key(&s)), (vec![a], None));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((selected(&s), key(&s)), (vec![hidden, a, b], Some(a)));
}

/// Lock › All Artwork Above and Hide › All Artwork Above take each object that overlaps a selected
/// object below it in the same layer, once, whether it is locked or hidden already or not. An object
/// that overlaps only a selected object above it, the group around a selected object, the selected
/// objects and an object that lies over a selected object of another layer stay as they are.
#[test]
fn lock_and_hide_all_artwork_above_take_what_overlaps_a_selected_object_below_in_its_layer() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    // `b` overlaps only `c`, which lies above it.
    let b = rect(&mut s, 250.0, 0.0, 100.0, 100.0);
    let c = rect(&mut s, 90.0, 0.0, 200.0, 100.0);
    // `d` overlaps `a`, `e` overlaps `c`, `f` overlaps both, `g` overlaps neither, and `k` overlaps `a` and is locked.
    let d = rect(&mut s, 20.0, 50.0, 30.0, 30.0);
    let e = rect(&mut s, 200.0, 50.0, 30.0, 30.0);
    let f = rect(&mut s, 60.0, 10.0, 50.0, 20.0);
    let g = rect(&mut s, 500.0, 500.0, 20.0, 20.0);
    let k = rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    s.execute("layer.setProps", &json!({"id": k.0, "locked": true})).unwrap();
    // `m` is in a group, and `n` overlaps it.
    let m = rect(&mut s, 800.0, 0.0, 50.0, 50.0);
    sel(&mut s, &[m]);
    let group = id_of(&s.execute("object.group", &json!({})).unwrap());
    let n = rect(&mut s, 820.0, 20.0, 50.0, 50.0);
    // In another layer, `h` lies where `a` lies, and `j` overlaps `i`.
    s.execute("layer.new", &json!({})).unwrap();
    let h = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let i = rect(&mut s, 600.0, 0.0, 100.0, 100.0);
    let j = rect(&mut s, 650.0, 50.0, 100.0, 100.0);
    let (taken, kept) = ([d, e, f, k, n, j], [a, b, c, g, group, m, h, i]);
    for (cmd, lock) in [("object.lock.above", true), ("object.hide.above", false)] {
        let done = |s: &Session, id: NodeId| if lock { node(s, id).locked } else { !node(s, id).visible };
        sel(&mut s, &[a, c, m, i]);
        let steps = undo_len(&s);
        assert_eq!(s.execute(cmd, &json!({})).unwrap(), json!({"count": 6}), "{cmd}");
        assert!(taken.iter().all(|id| done(&s, *id)), "{cmd}");
        assert!(!kept.iter().any(|id| done(&s, *id)), "{cmd}");
        assert_eq!((undo_len(&s), selected(&s)), (steps + 1, vec![a, c, m, i]), "{cmd}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

/// A session with 1,000 tiles of 100 x 200 pt in rows of 40 under the 50,000 squares of [`square`],
/// 50 on each tile, all of it selected; and the ids of the tiles and of the squares.
fn tiles_under_squares() -> (Session, Vec<NodeId>, Vec<NodeId>) {
    let tile = |t: usize| {
        let (x, y) = ((t % 40) as f64 * 100.0, (t / 40) as f64 * 200.0);
        Rect::new(x, y, x + 100.0, y + 200.0)
    };
    let s = crate::tests_large_edits::session(51_000, |d, k| {
        if k < 1000 { Node::path(d.alloc_id(), shapes::rectangle(tile(k)), Appearance::default_art()) } else { square(d, k - 1000) }
    });
    let mut tiles = layer(&s, 0);
    let squares = tiles.split_off(1000);
    (s, tiles, squares)
}

/// With 1,000 tiles selected under 50,000 squares, Lock › All Artwork Above locks every square in one
/// undo step and Hide › All Artwork Above hides every square, each within 20 s, and the tiles stay
/// selected.
#[test]
fn lock_and_hide_all_artwork_above_1000_tiles_under_50000_objects_finish_within_20_s() {
    let (mut s, tiles, squares) = tiles_under_squares();
    sel(&mut s, &tiles);
    let before = s.doc().unwrap().doc.clone();
    let (mut s, r) = run_soon(s, "object.lock.above", json!({}));
    assert_eq!(r, json!({"count": 50_000}));
    let d = &s.doc().unwrap().doc;
    assert!(squares.iter().all(|id| d.node(*id).is_some_and(|n| n.locked)), "every square is locked");
    assert!(tiles.iter().all(|id| d.node(*id).is_some_and(|n| !n.locked)), "no tile is locked");
    assert!(selected(&s) == tiles, "the tiles stay selected");
    undo_gives_back(&mut s, &before);
    let (s, r) = run_soon(s, "object.hide.above", json!({}));
    assert_eq!(r, json!({"count": 50_000}));
    let d = &s.doc().unwrap().doc;
    assert!(squares.iter().all(|id| d.node(*id).is_some_and(|n| !n.visible)), "every square is hidden");
    assert!(tiles.iter().all(|id| d.node(*id).is_some_and(|n| n.visible)), "no tile is hidden");
    assert!(selected(&s) == tiles, "the tiles stay selected");
}

/// With all 51,000 objects of [`tiles_under_squares`] selected, Lock › All Artwork Above and Hide ›
/// All Artwork Above each find nothing to lock or hide within 20 s and record no undo step.
#[test]
fn lock_and_hide_all_artwork_above_with_51000_objects_selected_finish_within_20_s() {
    let (s, tiles, squares) = tiles_under_squares();
    let (steps, all) = (undo_len(&s), [tiles, squares].concat());
    assert!(selected(&s) == all, "everything is selected");
    let (s, r) = run_soon(s, "object.lock.above", json!({}));
    assert_eq!(r, json!({"count": 0}));
    let (s, r) = run_soon(s, "object.hide.above", json!({}));
    assert_eq!(r, json!({"count": 0}));
    assert_eq!(undo_len(&s), steps);
    assert!(selected(&s) == all, "everything stays selected");
}

#[test]
fn lock_and_hide_other_layers() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let l2 = id_of(&s.execute("layer.new", &json!({})).unwrap_or(json!({"id": 0})));
    let l1 = s.doc().unwrap().doc.layers[0].id;
    assert_ne!(l1, l2);
    sel(&mut s, &[a]);
    s.execute("object.lock.otherLayers", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(l2).unwrap().locked);
    assert!(!d.node(l1).unwrap().locked);
    s.execute("object.hide.otherLayers", &json!({})).unwrap();
    assert!(!s.doc().unwrap().doc.node(l2).unwrap().visible);
}

/// Whether `id` is visible and whether it is locked.
fn shown_and_locked(s: &Session, id: NodeId) -> (bool, bool) {
    let n = node(s, id);
    (n.visible, n.locked)
}

/// Each of `commands` is off in isolation mode: the command list reports it disabled, its error
/// names isolation mode, and so does the error of a call that gives the selection as `ids` (for a
/// command that takes them).
fn off_in_isolation(s: &mut Session, commands: &[&str]) {
    let ids: Vec<u64> = selected(s).iter().map(|i| i.0).collect();
    for c in commands {
        let spec = find_command(c).unwrap();
        assert_eq!(spec.info(s).disabled_reason.as_deref(), Some("exit isolation mode first"), "{c}");
        let given = cmd::params::takes_key(spec.params, "ids").then(|| json!({"ids": ids}));
        for p in std::iter::once(json!({})).chain(given) {
            let e = s.execute(c, &p).unwrap_err().to_string();
            assert!(e.contains("exit isolation mode first"), "{c} {p}: {e}");
        }
    }
}

/// In isolation mode, Lock › Selection and Hide › Selection are off and change nothing, in an
/// isolated group and in an isolated compound path with one of its paths selected. After Exit
/// Isolation Mode they lock and hide again.
#[test]
fn lock_and_hide_selection_are_off_in_isolation_mode() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let outer = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let hole = rect(&mut s, 120.0, 120.0, 20.0, 20.0);
    sel(&mut s, &[outer, hole]);
    let compound = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    let ids = [a, b, g, outer, hole, compound];
    let commands = ["object.lock", "object.hide"];
    for (isolated, member) in [(g, a), (compound, hole)] {
        s.execute("object.isolate", &json!({"id": isolated.0})).unwrap();
        sel(&mut s, &[member]);
        off_in_isolation(&mut s, &commands);
        for id in ids {
            assert_eq!(shown_and_locked(&s, id), (true, false), "{id:?} in isolation of {isolated:?}");
        }
        assert_eq!(selected(&s), [member]);
        s.execute("object.exitIsolation", &json!({})).unwrap();
    }
    sel(&mut s, &[a]);
    s.execute("object.lock", &json!({})).unwrap();
    sel(&mut s, &[outer]);
    s.execute("object.hide", &json!({})).unwrap();
    assert_eq!(shown_and_locked(&s, a), (true, true));
    assert_eq!(shown_and_locked(&s, compound), (false, false));
}

/// In isolation mode, Unlock All and Show All are off: the locked and the hidden objects outside
/// and inside the isolated group stay as they are. Pattern editing mode and opacity mask editing
/// are not isolation mode for them.
#[test]
fn unlock_all_and_show_all_are_off_in_isolation_mode() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let c = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b, c]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let locked = rect(&mut s, 100.0, 0.0, 10.0, 10.0);
    let hidden = rect(&mut s, 120.0, 0.0, 10.0, 10.0);
    s.execute("layer.setProps", &json!({"id": locked.0, "locked": true})).unwrap();
    s.execute("layer.setProps", &json!({"id": hidden.0, "visible": false})).unwrap();
    s.execute("layer.setProps", &json!({"id": b.0, "locked": true})).unwrap();
    s.execute("layer.setProps", &json!({"id": c.0, "visible": false})).unwrap();
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    sel(&mut s, &[a]);
    off_in_isolation(&mut s, &["object.unlockAll", "object.showAll"]);
    for (id, want) in [(locked, (true, true)), (hidden, (false, false)), (b, (true, true)), (c, (false, false))] {
        assert_eq!(shown_and_locked(&s, id), want, "{id:?}");
    }
    s.execute("object.exitIsolation", &json!({})).unwrap();
    s.execute("object.unlockAll", &json!({})).unwrap();
    s.execute("object.showAll", &json!({})).unwrap();
    for id in [locked, hidden, b, c] {
        assert_eq!(shown_and_locked(&s, id), (true, false), "{id:?}");
    }

    // Pattern editing mode: the tile's art hides and shows again.
    let r = rect(&mut s, 300.0, 300.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    let tile = selected(&s);
    s.execute("object.hide", &json!({})).unwrap();
    assert!(!node(&s, tile[0]).visible, "hidden in pattern editing mode");
    s.execute("object.showAll", &json!({})).unwrap();
    assert!(node(&s, tile[0]).visible, "shown in pattern editing mode");
    // An object isolated inside pattern editing mode is isolation mode again.
    s.execute("object.isolate", &json!({"id": tile[0].0})).unwrap();
    sel(&mut s, &[tile[0]]);
    off_in_isolation(&mut s, &["object.unlockAll", "object.showAll"]);
    s.execute("object.pattern.cancel", &json!({})).unwrap();

    // Opacity mask editing: the mask's art locks and unlocks again.
    let obj = rect(&mut s, 0.0, 300.0, 200.0, 200.0);
    let m = rect(&mut s, 0.0, 300.0, 100.0, 200.0);
    sel(&mut s, &[obj, m]);
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    s.execute("transparency.editOpacityMask", &json!({})).unwrap();
    let mask = selected(&s);
    s.execute("object.lock", &json!({})).unwrap();
    assert!(node(&s, mask[0]).locked, "locked in opacity mask editing");
    s.execute("object.unlockAll", &json!({})).unwrap();
    assert!(!node(&s, mask[0]).locked, "unlocked in opacity mask editing");
}

/// In isolation mode, Lock › All Artwork Above, Lock › Other Layers, Hide › All Artwork Above and
/// Hide › Other Layers are off: the art above and the other layer stay as they are.
#[test]
fn lock_and_hide_above_and_other_layers_are_off_in_isolation_mode() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let over = rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    let l2 = id_of(&s.execute("layer.new", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    sel(&mut s, &[a]);
    off_in_isolation(&mut s, &["object.lock.above", "object.lock.otherLayers", "object.hide.above", "object.hide.otherLayers"]);
    for id in [a, b, g, over, l2] {
        assert_eq!(shown_and_locked(&s, id), (true, false), "{id:?}");
    }
    s.execute("object.exitIsolation", &json!({})).unwrap();
    sel(&mut s, &[g]);
    assert_eq!(s.execute("object.lock.above", &json!({})).unwrap(), json!({"count": 1}));
    s.execute("object.hide.otherLayers", &json!({})).unwrap();
    assert_eq!(shown_and_locked(&s, over), (true, true));
    assert_eq!(shown_and_locked(&s, l2), (false, false));
}

/// How many steps there are to undo and to redo, and whether the document has unsaved changes.
fn history(s: &Session) -> (usize, usize, bool) {
    let st = s.doc().unwrap();
    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
}

/// Select `ids` with a move of them left to redo, and mark the document saved.
fn select_with_a_step_to_redo(s: &mut Session, ids: &[NodeId]) {
    sel(s, ids);
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
}

/// Lock › All Artwork Above and Hide › All Artwork Above with no object above the selection, or with
/// every one locked (Lock) or hidden (Hide) already, and Lock › Other Layers and Hide › Other Layers
/// with no other layer, or with every one locked or hidden already, record no undo step: the step to
/// redo stays and the document stays unmodified. All Artwork Above still deselects a selected object
/// that is hidden.
#[test]
fn lock_and_hide_with_nothing_to_lock_or_hide_record_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let over = rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    // Nothing is above `over`, and the document has one layer.
    select_with_a_step_to_redo(&mut s, &[over]);
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
    for c in ["object.lock.above", "object.hide.above", "object.lock.otherLayers", "object.hide.otherLayers"] {
        assert_eq!(s.execute(c, &json!({})).unwrap(), json!({"count": 0}), "{c}");
        assert_eq!(history(&s), before, "{c}");
        assert_eq!(selected(&s), [over], "{c}");
    }
    // Lock › All Artwork Above locks `over` above `a` in one undo step, and then it has nothing left
    // to lock. Hide › All Artwork Above then hides `over`, and then it has nothing left to hide.
    for c in ["object.lock.above", "object.hide.above"] {
        select_with_a_step_to_redo(&mut s, &[a]);
        let undo = history(&s).0;
        assert_eq!(s.execute(c, &json!({})).unwrap(), json!({"count": 1}), "{c}");
        assert_eq!(history(&s), (undo + 1, 0, true), "{c}");
        select_with_a_step_to_redo(&mut s, &[a]);
        let before = history(&s);
        assert_eq!(s.execute(c, &json!({})).unwrap(), json!({"count": 1}), "{c} again");
        assert_eq!(history(&s), before, "{c} again");
    }
    // A hidden object selected beside `a` is deselected, still without an undo step.
    let hidden = rect(&mut s, 400.0, 400.0, 20.0, 20.0);
    s.execute("layer.setProps", &json!({"id": hidden.0, "visible": false})).unwrap();
    select_with_a_step_to_redo(&mut s, &[hidden, a]);
    let before = history(&s);
    assert_eq!(s.execute("object.hide.above", &json!({})).unwrap(), json!({"count": 1}));
    assert_eq!((history(&s), selected(&s)), (before, vec![a]));
    // Other Layers locks, then hides, a second layer in one undo step each, and then it has nothing
    // left to lock or hide.
    s.execute("layer.new", &json!({})).unwrap();
    for c in ["object.lock.otherLayers", "object.hide.otherLayers"] {
        select_with_a_step_to_redo(&mut s, &[a]);
        let undo = history(&s).0;
        assert_eq!(s.execute(c, &json!({})).unwrap(), json!({"count": 1}), "{c}");
        assert_eq!(history(&s), (undo + 1, 0, true), "{c}");
        select_with_a_step_to_redo(&mut s, &[a]);
        let before = history(&s);
        assert_eq!(s.execute(c, &json!({})).unwrap(), json!({"count": 1}), "{c} again");
        assert_eq!(history(&s), before, "{c} again");
    }
}

/// Hide › Unselected Artwork hides, in one undo step, each object that is not selected and holds no selected object:
/// here `b`, the member `n` of the group that holds the selected `m`, and `p` on another layer. The selected objects,
/// that group, the clip group that holds the selected `c2` with all its members, a locked object, the art on a locked
/// layer and an object hidden before stay as they are, and so does the selection. Show All shows them again.
#[test]
fn hide_unselected_artwork_hides_every_object_that_is_not_selected() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let m = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    let n = rect(&mut s, 60.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[m, n]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let c1 = rect(&mut s, 80.0, 0.0, 10.0, 10.0);
    let c2 = rect(&mut s, 85.0, 5.0, 10.0, 10.0);
    let c3 = rect(&mut s, 80.0, 0.0, 30.0, 30.0);
    sel(&mut s, &[c1, c2, c3]);
    let clip = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let k = rect(&mut s, 120.0, 0.0, 10.0, 10.0);
    s.execute("layer.setProps", &json!({"id": k.0, "locked": true})).unwrap();
    let h = rect(&mut s, 140.0, 0.0, 10.0, 10.0);
    s.execute("layer.setProps", &json!({"id": h.0, "visible": false})).unwrap();
    s.execute("layer.new", &json!({})).unwrap();
    let p = rect(&mut s, 0.0, 100.0, 10.0, 10.0);
    let locked_layer = id_of(&s.execute("layer.new", &json!({})).unwrap());
    let q = rect(&mut s, 20.0, 100.0, 10.0, 10.0);
    s.execute("layer.setProps", &json!({"id": locked_layer.0, "locked": true})).unwrap();
    let shown = |s: &Session, ids: &[NodeId]| ids.iter().map(|id| node(s, *id).visible).collect::<Vec<_>>();
    sel(&mut s, &[a, m, c2]);
    let steps = undo_len(&s);
    assert_eq!(s.execute("object.hide.unselected", &json!({})).unwrap(), json!({"count": 3}));
    assert_eq!(shown(&s, &[b, n, p, h]), [false; 4]);
    assert_eq!(shown(&s, &[a, m, g, c1, c2, c3, clip, k, q]), [true; 9]);
    assert!(node(&s, k).locked);
    assert_eq!((undo_len(&s), selected(&s)), (steps + 1, vec![a, m, c2]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(shown(&s, &[b, n, p]), [true; 3]);
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("object.showAll", &json!({})).unwrap();
    assert_eq!(shown(&s, &[b, n, p, h]), [true; 4]);
}

/// Hide › Unselected Artwork leaves the art on a locked artboard as it is, as Select All leaves it unselected.
#[test]
fn hide_unselected_artwork_leaves_the_art_on_a_locked_artboard() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0, 10.0), rect(&mut s, 20.0, 0.0, 10.0, 10.0));
    let on_locked = rect(&mut s, 2050.0, 50.0, 10.0, 10.0);
    s.execute("artboard.new", &json!({"x": 2000, "y": 0, "width": 400, "height": 400})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    sel(&mut s, &[a]);
    assert_eq!(s.execute("object.hide.unselected", &json!({})).unwrap(), json!({"count": 1}));
    assert_eq!([a, b, on_locked].map(|id| node(&s, id).visible), [true, false, true]);
}

/// In isolation mode Hide › Unselected Artwork is disabled: the menu item is off, and the command returns an error that
/// names isolation mode and changes nothing. While Lock Guides is on it leaves the guides as they are. With nothing
/// left to hide it records no undo step and returns a count of 0. A compound path that holds a selected path stays as
/// it is.
#[test]
fn hide_unselected_artwork_keeps_locked_guides_and_is_disabled_in_isolation_mode() {
    let mut s = session();
    let guide = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[guide]);
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    let out = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let m = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    let n = rect(&mut s, 60.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[m, n]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let hide = |s: &mut Session| s.execute("object.hide.unselected", &json!({})).unwrap();
    let shown = |s: &Session, ids: &[NodeId]| ids.iter().map(|id| node(s, *id).visible).collect::<Vec<_>>();
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    select_with_a_step_to_redo(&mut s, &[m]);
    let before = history(&s);
    let info = find_command("object.hide.unselected").unwrap().info(&s);
    assert!(!info.enabled && info.disabled_reason.as_deref() == Some("exit isolation mode first"), "{info:?}");
    let e = s.execute("object.hide.unselected", &json!({})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..)) && e.to_string().contains("isolation mode"), "{e}");
    assert_eq!(shown(&s, &[m, n, out, guide]), [true; 4]);
    assert_eq!((history(&s), selected(&s)), (before, vec![m]));
    s.execute("object.exitIsolation", &json!({})).unwrap();
    sel(&mut s, &[m]);
    assert!(find_command("object.hide.unselected").unwrap().info(&s).enabled);
    assert_eq!(hide(&mut s), json!({"count": 2}));
    assert_eq!(shown(&s, &[n, out, guide]), [false, false, true]);
    select_with_a_step_to_redo(&mut s, &[m]);
    let before = history(&s);
    assert_eq!(hide(&mut s), json!({"count": 0}));
    assert_eq!(history(&s), before);
    // With Lock Guides off, the guide path is hidden like other art.
    s.execute("view.guides.lock", &json!({"locked": false})).unwrap();
    sel(&mut s, &[m]);
    assert_eq!(hide(&mut s), json!({"count": 1}));
    assert_eq!(shown(&s, &[guide]), [false]);
    // A compound path that holds the selected path stays as it is with its paths.
    let outer = rect(&mut s, 0.0, 200.0, 100.0, 100.0);
    let hole = rect(&mut s, 40.0, 240.0, 20.0, 20.0);
    sel(&mut s, &[outer, hole]);
    let c = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    sel(&mut s, &[hole]);
    assert_eq!(hide(&mut s), json!({"count": 1}));
    assert_eq!(shown(&s, &[g, c, outer, hole]), [false, true, true, true]);
}

/// While Lock Guides is on, Hide › Unselected Artwork keeps a group that holds a guide path visible and hides its other
/// members one by one. With a layer selected, the objects on that layer stay visible and the rest are hidden.
#[test]
fn hide_unselected_artwork_keeps_groups_with_locked_guides_and_the_selected_layers_art() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let guide = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    let x = rect(&mut s, 60.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[guide, x]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    sel(&mut s, &[guide]);
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    let other = id_of(&s.execute("layer.new", &json!({})).unwrap());
    let p = rect(&mut s, 0.0, 100.0, 10.0, 10.0);
    let hide = |s: &mut Session| s.execute("object.hide.unselected", &json!({})).unwrap();
    let shown = |s: &Session, ids: &[NodeId]| ids.iter().map(|id| node(s, *id).visible).collect::<Vec<_>>();
    sel(&mut s, &[a]);
    assert_eq!(hide(&mut s), json!({"count": 3}));
    assert_eq!(shown(&s, &[a, b, g, guide, x, p]), [true, false, true, true, false, false]);
    s.execute("object.showAll", &json!({})).unwrap();
    sel(&mut s, &[other]);
    assert_eq!(hide(&mut s), json!({"count": 3}));
    assert_eq!(shown(&s, &[a, b, g, guide, x, p]), [false, false, true, true, false, true]);
    assert_eq!(selected(&s), [other]);
}

// ---------- Transform Each ----------

#[test]
fn transform_each_scales_each_about_its_own_center() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a, b]);
    let n = undo_len(&s);
    s.execute("object.transformEach", &json!({"scaleH": 50, "scaleV": 50})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(25.0, 25.0, 75.0, 75.0));
    assert_eq!(bounds(&s, b), Rect::new(225.0, 25.0, 275.0, 75.0));
    assert_eq!(undo_len(&s), n + 1);
}

#[test]
fn transform_each_move_reference_and_copy() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    // Top-left reference: scaling keeps the top-left corner.
    let r = s.execute("object.transformEach", &json!({"scaleH": 50, "scaleV": 50, "moveH": 10, "moveV": 5, "reference": 0, "copy": true})).unwrap();
    let copy = NodeId(r["ids"][0].as_u64().unwrap());
    assert_ne!(copy, a);
    assert_eq!(bounds(&s, a), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert_eq!(bounds(&s, copy), Rect::new(10.0, 5.0, 60.0, 55.0));
}

#[test]
fn transform_each_random_is_deterministic_with_seed() {
    let run = || {
        let mut s = session();
        let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        sel(&mut s, &[a]);
        s.execute("object.transformEach", &json!({"moveH": 50, "random": true, "seed": 7})).unwrap();
        bounds(&s, a)
    };
    let (x, y) = (run(), run());
    assert_eq!(x, y);
    assert!(x.x0.abs() <= 50.0 && x.width() == 100.0);
}

/// A `reference` or `seed` that is not a whole number (a fraction, a negative number or a string) is an error naming
/// the key, and the object stays as it was; a `reference` of 0.0 is the top-left point.
#[test]
fn transform_each_refuses_a_reference_or_seed_that_is_not_a_whole_number() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let before = (undo_len(&s), bounds(&s, a));
    for (p, msg) in [
        (json!({"scaleH": 50, "reference": 2.5}), "`reference` must be a whole number"),
        (json!({"scaleH": 50, "reference": -1}), "`reference` must be a whole number"),
        (json!({"scaleH": 50, "reference": "0"}), "`reference` must be a number, not \"0\""),
        (json!({"moveH": 50, "random": true, "seed": 7.5}), "`seed` must be a whole number"),
        (json!({"moveH": 50, "random": true, "seed": -7}), "`seed` must be a whole number"),
        (json!({"moveH": 50, "random": true, "seed": "7"}), "`seed` must be a number, not \"7\""),
    ] {
        let r = s.execute("object.transformEach", &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg: m, .. }) if m == msg), "{p}: {r:?}");
        assert_eq!((undo_len(&s), bounds(&s, a)), before, "{p}");
    }
    s.execute("object.transformEach", &json!({"scaleH": 50, "scaleV": 50, "reference": 0.0})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(0.0, 0.0, 50.0, 50.0));
}

/// Transform Each refuses a scale, move or rotation given as text or as another value that is not a
/// number, and the object stays as it was; numbers for the scales and the horizontal move transform it.
#[test]
fn transform_each_refuses_a_number_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let before = (undo_len(&s), bounds(&s, a));
    for (key, p) in [
        ("scaleH", json!({"scaleH": "50"})),
        ("scaleV", json!({"scaleH": 50, "scaleV": "50%"})),
        ("moveH", json!({"moveH": "10"})),
        ("moveV", json!({"moveH": 10, "moveV": [5]})),
        ("rotate", json!({"rotate": "45"})),
        ("rotate", json!({"moveH": 10, "rotate": true})),
    ] {
        refuses_non_number(&mut s, "object.transformEach", &p, key);
        assert_eq!((undo_len(&s), bounds(&s, a)), before, "{p}");
    }
    s.execute("object.transformEach", &json!({"scaleH": 50, "scaleV": 50, "moveH": 10, "moveV": null})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(35.0, 25.0, 85.0, 75.0));
}

/// Transform Each refuses Reflect X, Reflect Y, Random, Copy, Scale Strokes & Effects, Scale
/// Corners, Transform Objects or Transform Patterns given as text or as another value that is not
/// true or false, and the object stays as it was and is not copied; true, false and null still work.
#[test]
fn transform_each_refuses_a_boolean_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let count = |s: &Session| s.doc().unwrap().doc.layers[0].children().unwrap().len();
    let before = (undo_len(&s), bounds(&s, a), count(&s));
    for (key, p) in [
        ("reflectX", json!({"reflectX": "true"})),
        ("reflectY", json!({"moveH": 10, "reflectY": 1})),
        ("random", json!({"moveH": 10, "random": "false"})),
        ("copy", json!({"copy": "true"})),
        ("copy", json!({"moveH": 10, "copy": [true]})),
        ("strokes", json!({"scaleH": 50, "scaleV": 50, "strokes": "false"})),
        ("corners", json!({"scaleH": 50, "corners": 1})),
        ("objects", json!({"moveH": 10, "objects": "false"})),
        ("patterns", json!({"moveH": 10, "patterns": [true]})),
    ] {
        refuses_non_boolean(&mut s, "object.transformEach", &p, key);
        assert_eq!((undo_len(&s), bounds(&s, a), count(&s)), before, "{p}");
    }
    let p = json!({"moveH": 10, "reflectX": true, "random": false, "copy": null, "objects": null, "patterns": false});
    s.execute("object.transformEach", &p).unwrap();
    assert_eq!((bounds(&s, a), count(&s)), (Rect::new(10.0, 0.0, 110.0, 100.0), before.2));
}

/// With Scale Strokes & Effects on, Transform Each asked to scale a stroked rectangle with
/// `strokes` given as text, such as "false", fails: the saved document, the undo history and what
/// Transform Again repeats stay as they were, and Transform Again still repeats the Transform Each
/// before it. `strokes: false` scales the rectangle and keeps its stroke weight; `strokes: true`
/// and a null (the preference) scale both.
#[test]
fn transform_each_refuses_strokes_given_as_text_and_keeps_transform_again() {
    let mut s = session();
    s.prefs.scale_strokes = true;
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    s.execute("object.transformEach", &json!({"moveH": 10})).unwrap();
    let saved = |s: &Session| vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let repeat = |s: &Session| s.doc().unwrap().last_transform.clone();
    let before = (saved(&s), history(&s), repeat(&s));
    for strokes in [json!("false"), json!("true"), json!(0)] {
        let p = json!({"scaleH": 200, "scaleV": 200, "strokes": strokes});
        refuses_non_boolean(&mut s, "object.transformEach", &p, "strokes");
        assert!((saved(&s), history(&s), repeat(&s)) == before, "{p}");
    }
    let weight = |s: &Session| node(s, a).appearance.stroke_width();
    s.execute("object.transformAgain", &json!({})).unwrap();
    assert_eq!((bounds(&s, a), weight(&s)), (Rect::new(20.0, 0.0, 120.0, 100.0), 4.0), "the move before it repeats");
    for (strokes, w) in [(json!(false), 4.0), (json!(true), 8.0), (Value::Null, 8.0)] {
        let p = json!({"scaleH": 200, "scaleV": 200, "strokes": strokes});
        s.execute("object.transformEach", &p).unwrap();
        assert_eq!((bounds(&s, a), weight(&s)), (Rect::new(-30.0, -50.0, 170.0, 150.0), w), "{p}");
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

/// #1910: Transform Each's Reflect X flips left-right; Reflect Y flips top-bottom (same as
/// Effect › Distort › Transform and Illustrator's Transform Each checkboxes).
#[test]
fn transform_each_reflect_x_flips_left_right_reflect_y_flips_top_bottom() {
    let mut s = session();
    // A right-pointing triangle: tip at x=100, base on the left.
    let id =
        id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 50}, {"x": 0, "y": 100}], "closed": true})).unwrap());
    sel(&mut s, &[id]);
    s.execute("object.transformEach", &json!({"reflectX": true})).unwrap();
    let tip_x = match &node(&s, id).kind {
        // The tip is the only vertex at y=50; Reflect X about the centre maps (100, 50) → (0, 50).
        NodeKind::Path { path, .. } => path.subpaths[0].anchors.iter().find(|a| (a.p.y - 50.0).abs() < 1e-6).map(|a| a.p.x).unwrap_or(f64::NAN),
        _ => panic!("expected a path"),
    };
    assert!((tip_x - 0.0).abs() < 1e-6, "reflectX must move the tip from x=100 to x=0, got {tip_x}");

    s.execute("edit.undo", &json!({})).unwrap();
    // A down-pointing tip at (50, 100); Reflect Y about the centre (50, 50) maps it to (50, 0).
    let id2 =
        id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 50, "y": 100}], "closed": true})).unwrap());
    sel(&mut s, &[id2]);
    s.execute("object.transformEach", &json!({"reflectY": true})).unwrap();
    let tip_y = match &node(&s, id2).kind {
        NodeKind::Path { path, .. } => path.subpaths[0].anchors.iter().find(|a| (a.p.x - 50.0).abs() < 1e-6).map(|a| a.p.y).unwrap_or(f64::NAN),
        _ => panic!("expected a path"),
    };
    assert!((tip_y - 0.0).abs() < 1e-6, "reflectY must move the tip from y=100 to y=0, got {tip_y}");
}

/// Transform Each with its default values, or with any values that move nothing (both scales 100%,
/// no move, rotation or reflection, Random or not), records no undo step and replies with the
/// objects. With only a layer selected there is no object to transform: with Copy or without, it
/// records no undo step and keeps the selection. With Copy and objects selected, the default values
/// copy them in one undo step.
#[test]
fn transform_each_that_changes_nothing_records_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    select_with_a_step_to_redo(&mut s, &[a, b]);
    let before = history(&s);
    let still = json!({"scaleH": 100, "scaleV": 100, "moveH": 0, "moveV": 0, "rotate": 0, "reflectX": false, "reflectY": false, "random": true});
    for p in [json!({}), still, json!({"reference": 0, "strokes": true, "corners": true, "patterns": true})] {
        assert_eq!(s.execute("object.transformEach", &p).unwrap(), json!({"ids": [a.0, b.0]}), "{p}");
        assert_eq!(history(&s), before, "{p}");
        assert_eq!(s.journal.last().unwrap().0, "object.transformEach");
    }
    let layer = s.doc().unwrap().doc.layers[0].id;
    sel(&mut s, &[layer]);
    for p in [json!({"moveH": 10}), json!({"moveH": 10, "copy": true})] {
        assert_eq!(s.execute("object.transformEach", &p).unwrap(), json!({"ids": []}), "{p}");
        assert_eq!(history(&s), before, "{p}");
        assert_eq!(selected(&s), [layer], "{p}");
    }
    sel(&mut s, &[a, b]);
    let r = s.execute("object.transformEach", &json!({"copy": true})).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 2);
    assert_eq!(history(&s), (before.0 + 1, 0, true));
}

/// #1732: Transform Again (⌘D) repeats a Transform Each — the same transform on the objects
/// again, another copy when the Transform Each made copies — rather than an earlier transform.
#[test]
fn transform_again_repeats_transform_each() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    // An earlier Transform must not be what Transform Again repeats.
    s.execute("object.transform", &json!({"matrix": [1, 0, 0, 1, 5, 5]})).unwrap();
    s.execute("object.transformEach", &json!({"moveH": 50})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(55.0, 5.0, 155.0, 105.0));
    s.execute("object.transformAgain", &json!({})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(105.0, 5.0, 205.0, 105.0), "50 pt more, not 5");
    s.execute("object.transformAgain", &json!({})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(155.0, 5.0, 255.0, 105.0), "again and again");
    // With Copy, every Transform Again makes another copy, further on each time.
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let r = s.execute("object.transformEach", &json!({"moveH": 50, "copy": true})).unwrap();
    let first = NodeId(r["ids"][0].as_u64().unwrap());
    assert_eq!(bounds(&s, first), Rect::new(50.0, 0.0, 150.0, 100.0));
    let r = s.execute("object.transformAgain", &json!({})).unwrap();
    let second = NodeId(r["ids"][0].as_u64().unwrap());
    assert_ne!(second, first);
    assert_eq!(bounds(&s, second), Rect::new(100.0, 0.0, 200.0, 100.0), "another copy, 50 pt further");
    assert_eq!(bounds(&s, a), Rect::new(0.0, 0.0, 100.0, 100.0), "the original stays");
    assert_eq!(bounds(&s, first), Rect::new(50.0, 0.0, 150.0, 100.0), "and the first copy too");
}

/// #1732: the Transform Each dialog previews through an interaction; OK commits it, and
/// Transform Again repeats what the commit recorded. A preview that changed nothing doesn't take
/// the record over (it records no undo step either).
#[test]
fn transform_again_repeats_a_committed_transform_each_preview() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    s.begin_interaction("Transform Each").unwrap();
    s.preview("object.transformEach", &json!({"moveH": 50})).unwrap();
    s.commit_interaction().unwrap();
    assert_eq!(bounds(&s, a), Rect::new(50.0, 0.0, 150.0, 100.0));
    s.execute("object.transformAgain", &json!({})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(100.0, 0.0, 200.0, 100.0), "the dialog's Transform Each repeats");
    s.begin_interaction("Transform Each").unwrap();
    s.preview("object.transformEach", &json!({})).unwrap();
    s.commit_interaction().unwrap();
    s.execute("object.transformAgain", &json!({})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(150.0, 0.0, 250.0, 100.0), "still the one that changed something");
}

#[test]
fn reset_bounding_box_is_a_noop() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 0);
    assert_eq!(undo_len(&s), n);
}

// ---------- Expand / Rasterize / Crop / Trim marks ----------

#[test]
fn expand_live_shape_and_text_in_one_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let t = text(&mut s, "Hi");
    sel(&mut s, &[a, t]);
    let n = undo_len(&s);
    // Stroke off: the rectangle stays one path (its stroke isn't outlined).
    s.execute("object.expand", &json!({"stroke": false})).unwrap();
    assert!(matches!(node(&s, a).kind, NodeKind::Path { live: None, .. }));
    assert!(s.doc().unwrap().doc.node(t).is_none(), "text became outlines");
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Expand");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(node(&s, a).kind, NodeKind::Path { live: Some(_), .. }));
    assert!(s.doc().unwrap().doc.node(t).is_some());
}

#[test]
fn expand_text_outlines_its_stroke_too() {
    let mut s = session();
    let t = text(&mut s, "Hi");
    sel(&mut s, &[t]);
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    let info = s.execute("object.expand.info", &json!({})).unwrap();
    assert_eq!(info["stroke"], json!(true), "stroked type has a stroke to expand");
    let n = undo_len(&s);
    s.execute("object.expand", &json!({})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let d = &s.doc().unwrap().doc;
    let (mut paths, mut stroked) = (0, 0);
    for id in &s.doc().unwrap().selection.objects {
        d.node(*id).unwrap().walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
                paths += 1;
                stroked += usize::from(!c.appearance.stroke_paint().is_none());
            }
        });
    }
    assert!(paths > 0);
    assert_eq!(stroked, 0, "every glyph's stroke became a filled outline ({paths} paths)");
}

#[test]
fn expand_nothing_errors() {
    let mut s = session();
    let p = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 10, "y": 10}]})).unwrap());
    sel(&mut s, &[p]);
    // A plain path: nothing to expand for object/fill.
    let r = s.execute("object.expand", &json!({"stroke": false}));
    assert!(r.is_err());
}

#[test]
fn rasterize_replaces_selection_with_image() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 40.0, 30.0);
    fill(&mut s, a, "#ff0000");
    sel(&mut s, &[a]);
    let vb = s.doc().unwrap().doc.bounds_of(&[a], true).unwrap();
    let r = s.execute("object.rasterize", &json!({"ppi": 144, "padding": 0})).unwrap();
    let img = id_of(&r);
    assert!(s.doc().unwrap().doc.node(a).is_none());
    let n = node(&s, img);
    let NodeKind::Image(im) = &n.kind else { panic!() };
    // Visual bounds (stroke included) at 2 px/pt.
    assert_eq!((im.width as f64, im.height as f64), ((vb.width() * 2.0).ceil(), (vb.height() * 2.0).ceil()));
    assert!(s.doc().unwrap().doc.images.contains_key(&im.key));
    let b = n.geometric_bounds().unwrap();
    assert!(close(b.x0, vb.x0) && close(b.y0, vb.y0) && b.width() >= vb.width());
    assert_eq!(selected(&s), vec![img]);
}

/// Rasterize refuses a resolution given as text or as another value that is not a number, and the
/// selected art stays as it was.
#[test]
fn rasterize_refuses_a_ppi_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 40.0, 30.0);
    sel(&mut s, &[a]);
    let before = (undo_len(&s), node(&s, a));
    for p in [json!({"ppi": "144"}), json!({"ppi": "300 ppi", "padding": 0}), json!({"ppi": [72]})] {
        refuses_non_number(&mut s, "object.rasterize", &p, "ppi");
        assert_eq!((undo_len(&s), node(&s, a)), before, "{p}");
    }
    let img = id_of(&s.execute("object.rasterize", &json!({"ppi": 144, "padding": 0})).unwrap());
    assert!(matches!(node(&s, img).kind, NodeKind::Image(_)));
}

#[test]
fn rasterize_says_when_an_image_could_not_be_decoded() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 40.0, 30.0);
    let cut = place_undecodable_png(&mut s);
    sel(&mut s, &[a, cut]);
    let r = s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
    assert!(warns_undecoded(&r), "{r}");
    // The image still replaces the selection.
    assert!(matches!(node(&s, id_of(&r)).kind, NodeKind::Image(_)));
    assert!(s.doc().unwrap().doc.node(a).is_none() && s.doc().unwrap().doc.node(cut).is_none());
}

#[test]
fn crop_image_says_when_the_image_could_not_be_decoded() {
    let mut s = session();
    let cut = place_undecodable_png(&mut s);
    sel(&mut s, &[cut]);
    let b = bounds(&s, cut);
    let r = s.execute("object.cropImage", &json!({"rect": [b.x0, b.y0, b.width() / 2.0, b.height()]})).unwrap();
    assert!(warns_undecoded(&r), "{r}");
}

#[test]
fn crop_image_to_rect() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let img = id_of(&s.execute("object.rasterize", &json!({"ppi": 72})).unwrap());
    let r = s.execute("object.cropImage", &json!({"rect": [10, 10, 50, 40]})).unwrap();
    assert_eq!((r["width"].as_u64().unwrap(), r["height"].as_u64().unwrap()), (50, 40));
    let b = bounds(&s, img);
    assert!(close(b.x0, 10.0) && close(b.y0, 10.0) && close(b.width(), 50.0) && close(b.height(), 40.0), "{b:?}");
    // Image fully inside the artboard: default crop has nothing to do.
    assert!(s.execute("object.cropImage", &json!({})).is_err());
}

/// Crop Image refuses a `rect` that is not an array, such as text, and the image stays as it was; a
/// null `rect` crops to the artboard as a missing one does.
#[test]
fn crop_image_refuses_a_rect_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, -20.0, -20.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let img = id_of(&s.execute("object.rasterize", &json!({"ppi": 72})).unwrap());
    let before = (undo_len(&s), node(&s, img));
    for p in [json!({"rect": "0, 0, 20, 20"}), json!({"rect": {"x": 0, "y": 0, "width": 20, "height": 20}}), json!({"rect": 20})] {
        let r = s.execute("object.cropImage", &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == "rect must be [x, y, width, height]"), "{p}: {r:?}");
        assert_eq!((undo_len(&s), node(&s, img)), before, "{p}");
    }
    s.execute("object.cropImage", &json!({"rect": null})).unwrap();
    let b = bounds(&s, img);
    assert!(close(b.x0, 0.0) && close(b.y0, 0.0), "{b:?}");
}

/// Crop Image refuses a `trim` given as text or as another value that is not true or false, and the
/// image stays as it was; a null `trim` crops to `rect` as a missing one does.
#[test]
fn crop_image_refuses_a_trim_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, -20.0, -20.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let img = id_of(&s.execute("object.rasterize", &json!({"ppi": 72})).unwrap());
    let before = (undo_len(&s), node(&s, img));
    for p in [json!({"trim": "true"}), json!({"trim": "false", "rect": [10, 10, 50, 40]}), json!({"trim": 1})] {
        refuses_non_boolean(&mut s, "object.cropImage", &p, "trim");
        assert_eq!((undo_len(&s), node(&s, img)), before, "{p}");
    }
    s.execute("object.cropImage", &json!({"trim": null, "rect": [10, 10, 50, 40]})).unwrap();
    let b = bounds(&s, img);
    assert!(close(b.x0, 10.0) && close(b.y0, 10.0) && close(b.width(), 50.0) && close(b.height(), 40.0), "{b:?}");
}

#[test]
fn crop_image_disabled_without_image() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    assert!(matches!(s.execute("object.cropImage", &json!({})), Err(EngineError::Disabled(..))));
}

#[test]
fn mask_image_clips_it_to_its_outline_and_selects_the_clipping_path() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 100.0, 50.0);
    sel(&mut s, &[a]);
    let img = id_of(&s.execute("object.rasterize", &json!({"ppi": 72})).unwrap());
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let ib = bounds(&s, img);
    let r = s.execute("object.maskImage", &json!({})).unwrap();
    let (g, p) = (id_of(&r), NodeId(r["path"].as_u64().unwrap()));
    let group = node(&s, g);
    assert!(matches!(group.kind, NodeKind::Group { clip: true, .. }));
    assert_eq!(group.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [p, img]);
    // The clipping path follows the rotated image's outline, so nothing is cut off yet.
    assert!(matches!(node(&s, p).kind, NodeKind::Path { clipping: true, .. }));
    let pb = bounds(&s, p);
    assert!(close(pb.x0, ib.x0) && close(pb.y0, ib.y0) && close(pb.x1, ib.x1) && close(pb.y1, ib.y1), "{pb:?} {ib:?}");
    assert_eq!(s.doc().unwrap().selection.objects, [p]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(g).is_none() && s.doc().unwrap().doc.node(img).is_some());
    sel(&mut s, &[]);
    assert!(matches!(s.execute("object.maskImage", &json!({})), Err(EngineError::Disabled(..))));
}

#[test]
fn trim_marks_around_selection() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    fill(&mut s, a, "#00ff00");
    s.execute("stroke.set", &json!({"ids": [a.0], "weight": 0})).unwrap();
    sel(&mut s, &[a]);
    let g = id_of(&s.execute("object.createTrimMarks", &json!({"offset": 10, "length": 20})).unwrap());
    let n = node(&s, g);
    assert_eq!(n.children().unwrap().len(), 8);
    assert_eq!(n.name.as_deref(), Some("Trim Marks"));
    let b = n.geometric_bounds().unwrap();
    let vb = s.doc().unwrap().doc.bounds_of(&[a], true).unwrap();
    assert!(close(b.x0, vb.x0 - 30.0) && close(b.y1, vb.y1 + 30.0), "{b:?} {vb:?}");
}

/// Create Trim Marks refuses an offset, length, bleed or weight given as text or as another value
/// that is not a number, around the selection and around the artboards, and makes no marks.
#[test]
fn trim_marks_refuse_a_number_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    let calls = [
        ("offset", json!({"offset": "10"})),
        ("length", json!({"length": "20 pt"})),
        ("bleed", json!({"style": "japanese", "bleed": "3"})),
        ("weight", json!({"weight": true})),
        ("offset", json!({"allArtboards": true, "offset": {"value": 10}})),
    ];
    for ids in [vec![a], vec![]] {
        sel(&mut s, &ids);
        let before = (undo_len(&s), s.doc().unwrap().doc.layers[0].children().unwrap().len());
        for (key, p) in &calls {
            refuses_non_number(&mut s, "object.createTrimMarks", p, key);
            assert_eq!((undo_len(&s), s.doc().unwrap().doc.layers[0].children().unwrap().len()), before, "{p}");
        }
    }
    s.execute("object.createTrimMarks", &json!({"offset": 10, "length": 20, "bleed": null})).unwrap();
}

/// Create Trim Marks refuses an `allArtboards` that is not true or false and a `style` that is not
/// text, around the selection and around the artboards, and makes no marks; null takes the default.
#[test]
fn trim_marks_refuse_a_boolean_or_style_of_the_wrong_kind() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    let style = "`style` must be roman or japanese";
    let calls = [
        (json!({"allArtboards": "true"}), "`allArtboards` must be true or false"),
        (json!({"allArtboards": 0}), "`allArtboards` must be true or false"),
        (json!({"style": 5}), style),
        (json!({"allArtboards": true, "style": true}), style),
        (json!({"style": ["roman"]}), style),
    ];
    for ids in [vec![a], vec![]] {
        sel(&mut s, &ids);
        let before = (undo_len(&s), s.doc().unwrap().doc.layers[0].children().unwrap().len());
        for (p, msg) in &calls {
            let r = s.execute("object.createTrimMarks", p);
            assert!(matches!(&r, Err(EngineError::BadParams { msg: m, .. }) if m == msg), "{p}: {r:?}");
            assert_eq!((undo_len(&s), s.doc().unwrap().doc.layers[0].children().unwrap().len()), before, "{p}");
        }
    }
    s.execute("object.createTrimMarks", &json!({"allArtboards": null, "style": null})).unwrap();
}

// ---------- Convert to Shape ----------

#[test]
fn convert_rectangle_path_to_live_shape() {
    let mut s = session();
    let p = id_of(
        &s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 40, "y": 0}, {"x": 40, "y": 20}, {"x": 0, "y": 20}], "closed": true}))
            .unwrap(),
    );
    sel(&mut s, &[p]);
    assert_eq!(s.execute("object.shape.convertToShape", &json!({})).unwrap()["converted"], 1);
    let n = node(&s, p);
    let (Some(LiveShape::Rectangle { w, h, .. }), NodeKind::Path { path, .. }) = (n.live_shape(), &n.kind) else { panic!() };
    assert!(close(*w, 40.0) && close(*h, 20.0));
    assert_eq!(path.bounds(), Some(Rect::new(0.0, 0.0, 40.0, 20.0)));
}

#[test]
fn convert_rotated_rectangle_and_ellipse() {
    let mut s = session();
    let p = id_of(
        &s.execute(
            "path.create",
            &json!({"anchors": [{"x": 50, "y": 0}, {"x": 100, "y": 50}, {"x": 50, "y": 100}, {"x": 0, "y": 50}], "closed": true}),
        )
        .unwrap(),
    );
    let e = id_of(&s.execute("shape.ellipse", &json!({"x": 0, "y": 200, "width": 80, "height": 40})).unwrap());
    sel(&mut s, &[e]);
    s.execute("object.expandShape", &json!({})).unwrap();
    sel(&mut s, &[p, e]);
    assert_eq!(s.execute("object.shape.convertToShape", &json!({})).unwrap()["converted"], 2);
    let b = bounds(&s, p);
    assert!(close(b.x0, 0.0) && close(b.x1, 100.0) && close(b.y1, 100.0), "{b:?}");
    assert!(matches!(node(&s, e).live_shape(), Some(LiveShape::Ellipse { .. })));
}

#[test]
fn convert_to_shape_rejects_other_paths() {
    let mut s = session();
    let p = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 40, "y": 0}, {"x": 10, "y": 20}], "closed": true})).unwrap());
    sel(&mut s, &[p]);
    assert!(s.execute("object.shape.convertToShape", &json!({})).is_err());
}

// ---------- Blend ----------

#[test]
fn blend_make_release_options() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 30.0, 30.0);
    fill(&mut s, a, "#000000");
    fill(&mut s, b, "#ffffff");
    sel(&mut s, &[a, b]);
    let n = undo_len(&s);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3})).unwrap());
    assert_eq!(undo_len(&s), n + 1);
    let gn = node(&s, g);
    assert!(matches!(gn.kind, NodeKind::Blend { .. }), "live blend");
    let ch = vectorcraft_doc::live::expand_live(&gn);
    assert_eq!(ch.len(), 5);
    // Middle step: halfway in position, size and colour.
    let mid = &ch[2];
    let mb = mid.geometric_bounds().unwrap();
    assert!(close(mb.x0, 50.0) && close(mb.width(), 20.0), "{mb:?}");
    let c = mid.appearance.fill_paint().color().unwrap().to_rgb();
    assert!((c[0] - 0.5).abs() < 1e-3);
    // Options: 1 step.
    s.execute("object.blend.options", &json!({"steps": 1})).unwrap();
    assert_eq!(vectorcraft_doc::live::expand_live(&node(&s, g)).len(), 3);
    // Release: keys only, back in the layer.
    let r = s.execute("object.blend.release", &json!({})).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 2);
    assert!(s.doc().unwrap().doc.node(g).is_none());
    assert!(s.doc().unwrap().doc.node(a).is_some() && s.doc().unwrap().doc.node(b).is_some());
}

#[test]
fn blend_expand_and_reverse() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 2})).unwrap());
    s.execute("object.blend.reverseFrontToBack", &json!({})).unwrap();
    assert_eq!(node(&s, g).children().unwrap().last().unwrap().id, a);
    s.execute("object.blend.reverseSpine", &json!({})).unwrap();
    assert!(close(bounds(&s, a).x0, 100.0));
    s.execute("object.blend.expand", &json!({})).unwrap();
    assert!(matches!(node(&s, g).kind, NodeKind::Group { .. }));
    assert!(s.execute("object.blend.release", &json!({})).is_err(), "no longer a blend");
}

// ---------- Artboards ----------

#[test]
fn convert_to_artboards_and_rearrange() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 200.0, 100.0);
    sel(&mut s, &[a]);
    assert_eq!(s.execute("artboard.convertToArtboards", &json!({})).unwrap()["artboards"], 1);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards.len(), 2);
    assert_eq!(d.artboards[1].rect, Rect::new(900.0, 0.0, 1100.0, 100.0));
    assert!(d.node(a).is_none());
    // Art on artboard 2 moves with it.
    let b = rect(&mut s, 950.0, 25.0, 10.0, 10.0);
    s.execute("artboard.rearrange", &json!({"columns": 1, "spacing": 50})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards[1].rect, Rect::new(0.0, 650.0, 200.0, 750.0));
    assert_eq!(bounds(&s, b), Rect::new(50.0, 675.0, 60.0, 685.0));
}

/// Convert to Artboards names an artboard one past the highest artboard id the document has, also
/// after a delete of the last artboard or an undo of a new one.
#[test]
fn convert_to_artboards_names_by_the_artboards_the_document_has() {
    let name_after = |change: &dyn Fn(&mut Session)| {
        let mut s = session();
        s.execute("artboard.new", &json!({})).unwrap();
        s.execute("artboard.new", &json!({})).unwrap();
        change(&mut s);
        let a = rect(&mut s, 3000.0, 0.0, 200.0, 100.0);
        sel(&mut s, &[a]);
        s.execute("artboard.convertToArtboards", &json!({})).unwrap();
        s.doc().unwrap().doc.artboards.last().map(|b| b.name.clone()).unwrap()
    };
    assert_eq!(name_after(&|s| _ = s.execute("artboard.delete", &json!({"index": 2})).unwrap()), "Artboard 3");
    assert_eq!(name_after(&|s| _ = s.execute("edit.undo", &json!({})).unwrap()), "Artboard 3");
}

/// Rearrange All Artboards refuses `columns` that are not a whole number (a fraction, a negative number or a string),
/// and the artboards stay where they were; 1.0, as the Rearrange All Artboards dialog sends a count, is one column.
#[test]
fn rearrange_refuses_columns_that_are_not_a_whole_number() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 300})).unwrap();
    let boards = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let before = (boards(&s), undo_len(&s));
    for (p, msg) in [
        (json!({"columns": 1.5}), "`columns` must be a whole number"),
        (json!({"columns": -1}), "`columns` must be a whole number"),
        (json!({"columns": "1"}), "`columns` must be a number, not \"1\""),
    ] {
        let r = s.execute("artboard.rearrange", &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg: m, .. }) if m == msg), "{p}: {r:?}");
        assert_eq!((boards(&s), undo_len(&s)), before, "{p}");
    }
    let r = s.execute("artboard.rearrange", &json!({"columns": 1.0, "spacing": 50})).unwrap();
    assert_eq!((r["rows"].clone(), r["columns"].clone()), (json!(2), json!(1)));
}

/// Rearrange All Artboards refuses a spacing given as text or as another value that is not a number,
/// and the artboards stay where they were.
#[test]
fn rearrange_refuses_a_spacing_given_as_text() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 300})).unwrap();
    let boards = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let before = (boards(&s), undo_len(&s));
    for p in [json!({"spacing": "50"}), json!({"columns": 1, "spacing": "50 pt"}), json!({"layout": "row", "spacing": false})] {
        refuses_non_number(&mut s, "artboard.rearrange", &p, "spacing");
        assert_eq!((boards(&s), undo_len(&s)), before, "{p}");
    }
    s.execute("artboard.rearrange", &json!({"columns": 1, "spacing": 50})).unwrap();
    assert_eq!(boards(&s)[1], Rect::new(0.0, 650.0, 400.0, 950.0));
}

/// Rearrange All Artboards refuses `byColumn`, `moveArtwork` or `patterns` given as text or as
/// another value that is not true or false, and the artboards and the art stay where they were;
/// `moveArtwork: false` leaves the art in place.
#[test]
fn rearrange_refuses_a_boolean_given_as_text() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 300})).unwrap();
    let b = rect(&mut s, 1050.0, 25.0, 10.0, 10.0);
    let boards = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let before = (boards(&s), undo_len(&s), bounds(&s, b));
    for (key, p) in [
        ("byColumn", json!({"byColumn": "true"})),
        ("byColumn", json!({"layout": "row", "byColumn": 1})),
        ("moveArtwork", json!({"moveArtwork": "false"})),
        ("moveArtwork", json!({"columns": 1, "moveArtwork": 0})),
        ("patterns", json!({"patterns": "true"})),
    ] {
        refuses_non_boolean(&mut s, "artboard.rearrange", &p, key);
        assert_eq!((boards(&s), undo_len(&s), bounds(&s, b)), before, "{p}");
    }
    s.execute("artboard.rearrange", &json!({"columns": 1, "spacing": 50, "byColumn": null, "moveArtwork": false})).unwrap();
    assert_eq!((boards(&s)[1], bounds(&s, b)), (Rect::new(0.0, 650.0, 400.0, 950.0), before.2));
}

/// Fit to Artwork Bounds fits each artboard it is given to the art on it, in one undo step: an
/// object partly on an artboard counts for each artboard it overlaps, art flush with an edge from
/// outside or on no artboard doesn't count, and an artboard with no art stays as it is.
#[test]
fn fit_to_artwork_bounds_fits_each_artboard_to_its_own_art() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 300})).unwrap();
    s.execute("artboard.new", &json!({"x": 0, "y": 1000, "width": 300, "height": 300})).unwrap();
    rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    rect(&mut s, 1100.0, 50.0, 100.0, 100.0);
    // Across the gap between artboards 1 and 2.
    rect(&mut s, 700.0, 250.0, 350.0, 30.0);
    // Flush with artboard 1's right edge, and on no artboard.
    rect(&mut s, 800.0, 500.0, 100.0, 50.0);
    rect(&mut s, 3000.0, 3000.0, 10.0, 10.0);
    s.execute("select.all", &json!({})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let boards = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let was = boards(&s);
    let n = undo_len(&s);
    s.execute("artboard.fitToArt", &json!({"indices": [0, 1, 2]})).unwrap();
    assert_eq!(boards(&s), [Rect::new(100.0, 100.0, 1050.0, 280.0), Rect::new(700.0, 50.0, 1200.0, 280.0), was[2]]);
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(boards(&s), was);
    assert!(s.execute("artboard.fitToArt", &json!({"index": 2})).is_err(), "no art on artboard 3");
    assert_eq!(boards(&s), was);
    s.execute("artboard.fitToArt", &json!({})).unwrap();
    assert_eq!(boards(&s), [Rect::new(100.0, 100.0, 1050.0, 280.0), was[1], was[2]], "the first artboard by default");
}

/// Artboard Options and Fit to Selected Art fail with `no such artboard` on an `index` that names
/// no artboard (past the last, negative, fractional, a string or null), and the artboards stay as
/// they were. Artboard Options needs an `index`; Fit to Selected Art without one fits the first
/// artboard.
#[test]
fn artboard_options_and_fit_to_selected_art_refuse_an_index_that_names_no_artboard() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 300})).unwrap();
    rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("select.all", &json!({})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let boards = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let (was, n) = (boards(&s), undo_len(&s));
    for index in [json!(99), json!(-1), json!(1.5), json!("1"), Value::Null] {
        for (cmd, p) in [("artboard.setProps", json!({"index": index, "width": 300})), ("artboard.fitToSelection", json!({"index": index}))] {
            assert_eq!(s.execute(cmd, &p).unwrap_err().to_string(), "no such artboard", "{cmd} {p}");
        }
    }
    let e = s.execute("artboard.setProps", &json!({"width": 300})).unwrap_err().to_string();
    assert_eq!(e, "invalid parameters for `artboard.setProps`: missing index");
    assert_eq!((boards(&s), undo_len(&s)), (was.clone(), n), "the artboards and the undo steps as they were");
    s.execute("artboard.fitToSelection", &json!({})).unwrap();
    assert_eq!(boards(&s), [Rect::new(100.0, 100.0, 150.0, 150.0), was[1]], "the first artboard by default");
}

// ---------- Edit Colors ----------

#[test]
fn invert_and_convert_colors() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    fill(&mut s, a, "#ff0000");
    sel(&mut s, &[a]);
    s.execute("edit.colors.invert", &json!({"stroke": false})).unwrap();
    assert_eq!(fill_of(&s, a).to_hex(), "#00ffff");
    s.execute("edit.colors.toCMYK", &json!({})).unwrap();
    assert!(matches!(fill_of(&s, a), Color::Cmyk { .. }));
    s.execute("edit.colors.toGrayscale", &json!({})).unwrap();
    assert!(matches!(fill_of(&s, a), Color::Gray { .. }));
    s.execute("edit.colors.toRGB", &json!({})).unwrap();
    assert!(matches!(fill_of(&s, a), Color::Rgb { .. }));
}

#[test]
fn saturate_and_adjust_balance() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [a.0], "color": [0.8, 0.4, 0.4]})).unwrap();
    sel(&mut s, &[a]);
    s.execute("edit.colors.saturate", &json!({"intensity": -100})).unwrap();
    let [_, sat, _] = fill_of(&s, a).to_hsb();
    assert!(sat < 1e-4);
    s.execute("edit.colors.adjustBalance", &json!({"r": 10, "g": -10})).unwrap();
    let [r, g, _] = fill_of(&s, a).to_rgb();
    assert!((r - 0.9).abs() < 1e-3 && (g - 0.7).abs() < 1e-3, "{r} {g}");
    assert!(s.execute("edit.colors.adjustBalance", &json!({})).is_err());
}

#[test]
fn blend_colors_horizontally() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..3).map(|i| rect(&mut s, 200.0 - i as f64 * 100.0, 0.0, 10.0, 10.0)).collect();
    fill(&mut s, ids[2], "#000000"); // leftmost
    fill(&mut s, ids[0], "#ffffff"); // rightmost
    fill(&mut s, ids[1], "#ff0000");
    sel(&mut s, &ids);
    s.execute("edit.colors.blendHorizontally", &json!({})).unwrap();
    assert_eq!(fill_of(&s, ids[1]).to_hex(), "#808080");
    sel(&mut s, &ids[..2]);
    assert!(s.execute("edit.colors.blendVertically", &json!({})).is_err(), "needs three");
}

#[test]
fn blend_front_to_back_uses_stacking() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..3).map(|_| rect(&mut s, 0.0, 0.0, 10.0, 10.0)).collect();
    fill(&mut s, ids[0], "#000000");
    fill(&mut s, ids[2], "#ffffff");
    sel(&mut s, &ids);
    s.execute("edit.colors.blendFrontToBack", &json!({})).unwrap();
    assert_eq!(fill_of(&s, ids[1]).to_hex(), "#808080");
}

// ---------- Paste without formatting / Find ----------

#[test]
fn paste_without_formatting_resets_text_style() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Bold", "size": 40})).unwrap());
    sel(&mut s, &[t]);
    s.execute("edit.copy", &json!({})).unwrap();
    let n = undo_len(&s);
    let r = s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap();
    let p = NodeId(r["ids"][0].as_u64().unwrap());
    let NodeKind::Text(tt) = &node(&s, p).kind else { panic!() };
    assert_eq!(tt.plain_text(), "Bold");
    assert_eq!(tt.first_style().size, 12.0);
    assert_eq!(undo_len(&s), n + 1);
    // Clipboard keeps its formatting.
    let NodeKind::Text(ct) = &s.clipboard.nodes[0].kind else { panic!() };
    assert_eq!(ct.first_style().size, 40.0);
}

#[test]
fn find_and_replace_options() {
    let mut s = session();
    let a = text(&mut s, "Cat cat catalog");
    let b = text(&mut s, "the CAT");
    let r = s.execute("edit.findReplace", &json!({"find": "cat", "replace": "dog", "wholeWord": true})).unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!(plain(&s, a), "dog dog catalog");
    assert_eq!(plain(&s, b), "the dog");
    let r = s.execute("edit.findReplace", &json!({"find": "Dog", "replace": "x", "matchCase": true})).unwrap();
    assert_eq!(r["count"], 0);
    assert!(s.execute("edit.findReplace", &json!({"find": ""})).is_err());
}

/// Find and Replace with no ids leaves the type on a locked artboard as it is.
#[test]
fn find_and_replace_leaves_type_on_a_locked_artboard() {
    let mut s = session();
    let a = text(&mut s, "cat");
    let b = id_of(&s.execute("text.create", &json!({"x": 2050, "y": 50, "text": "cat"})).unwrap());
    s.execute("artboard.new", &json!({"x": 2000, "y": 0, "width": 400, "height": 400})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    let r = s.execute("edit.findReplace", &json!({"find": "cat", "replace": "dog"})).unwrap();
    assert_eq!((r["count"].as_u64(), plain(&s, a), plain(&s, b)), (Some(1), "dog".to_string(), "cat".to_string()));
}

/// Find and Replace that replaces each match with the text it is (`CAT` found in `cat`, ignoring
/// case, and replaced by `cat`) records no undo step: the step to redo stays and the document stays
/// unmodified. It still replies with the count. A replacement that changes the text is one undo step.
#[test]
fn find_and_replace_that_changes_nothing_records_no_undo_step() {
    let mut s = session();
    let a = text(&mut s, "cat cat");
    select_with_a_step_to_redo(&mut s, &[a]);
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
    assert_eq!(s.execute("edit.findReplace", &json!({"find": "CAT", "replace": "cat"})).unwrap(), json!({"count": 2}));
    assert_eq!((history(&s), plain(&s, a)), (before, "cat cat".to_string()));
    assert_eq!(s.execute("edit.findReplace", &json!({"find": "cat", "replace": "dog"})).unwrap(), json!({"count": 2}));
    assert_eq!((history(&s), plain(&s, a)), ((before.0 + 1, 0, true), "dog dog".to_string()));
}

#[test]
fn find_next_cycles_through_text_objects() {
    let mut s = session();
    let a = text(&mut s, "apple");
    let _ = text(&mut s, "banana");
    let c = text(&mut s, "pineapple");
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(id_of(&s.execute("edit.findNext", &json!({"find": "APPLE"})).unwrap()), a);
    assert_eq!(id_of(&s.execute("edit.findNext", &json!({"find": "apple"})).unwrap()), c);
    assert_eq!(id_of(&s.execute("edit.findNext", &json!({"find": "apple"})).unwrap()), a);
    assert!(s.execute("edit.findNext", &json!({"find": "zzz"})).unwrap()["id"].is_null());
}

// ---------- Select ----------

#[test]
fn select_same_font_size_and_fill() {
    let mut s = session();
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 20, "text": "a", "size": 20})).unwrap());
    let b = id_of(&s.execute("text.create", &json!({"x": 0, "y": 60, "text": "b", "size": 20})).unwrap());
    let c = id_of(&s.execute("text.create", &json!({"x": 0, "y": 90, "text": "c", "size": 30, "font": "Inter"})).unwrap());
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.same.fontSize", &json!({})).unwrap()["count"], 2);
    assert_eq!(selected(&s), vec![a, b]);
    sel(&mut s, &[c]);
    assert_eq!(s.execute("select.same.fontFamily", &json!({})).unwrap()["count"], 1);
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.same.textFillColor", &json!({})).unwrap()["count"], 3);
    assert_eq!(s.execute("select.same.fontFamilyStyle", &json!({})).unwrap()["count"], 2);
    let r = rect(&mut s, 0.0, 0.0, 5.0, 5.0);
    sel(&mut s, &[r]);
    assert!(s.execute("select.same.fontSize", &json!({})).is_err());
}

/// #1737: Select > Same reads type's fill and stroke from its characters (the first run), so type
/// matches type and paths both ways.
#[test]
fn select_same_paint_reads_type_characters() {
    let mut s = session();
    let t1 = text(&mut s, "a");
    let t2 = text(&mut s, "b");
    let t3 = text(&mut s, "c");
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let q = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[t1, t2, r]);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    sel(&mut s, &[t3, q]);
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    sel(&mut s, &[t1, r]);
    s.execute("paint.setStroke", &json!({"color": "#00ff00"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 3})).unwrap();
    sel(&mut s, &[t3]);
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    let q_stroke = node(&s, q).appearance;
    assert_eq!((q_stroke.stroke_paint(), q_stroke.stroke_width()), (Paint::solid(Color::BLACK), 1.0));

    let same = |s: &mut Session, from: NodeId, cmd: &str| {
        sel(s, &[from]);
        s.execute(cmd, &json!({})).unwrap();
        let mut v = selected(s);
        v.sort();
        v
    };
    assert_eq!(same(&mut s, t1, "select.same.fillColor"), vec![t1, t2, r]);
    assert_eq!(same(&mut s, q, "select.same.fillColor"), vec![t3, q]);
    assert_eq!(same(&mut s, r, "select.same.strokeColor"), vec![t1, r]);
    assert_eq!(same(&mut s, t3, "select.same.strokeColor"), vec![t3, q]);
    assert_eq!(same(&mut s, q, "select.same.strokeWeight"), vec![t3, q]);
    assert_eq!(same(&mut s, t1, "select.same.fillAndStroke"), vec![t1, r]);
}

/// Select > Same matches a compound path on its own paint, as a whole: never its member paths.
/// When a member path is selected first, the other objects are compared with its compound path.
#[test]
fn select_same_matches_compound_paths_on_their_own_paint() {
    let mut s = session();
    let [a, b] = [0.0, 20.0].map(|x| rect(&mut s, x, 0.0, 10.0, 10.0));
    let outer = rect(&mut s, 100.0, 0.0, 50.0, 50.0);
    let inner = rect(&mut s, 110.0, 10.0, 20.0, 20.0);
    let r = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    for id in [a, b, outer, inner] {
        fill(&mut s, id, "#000000");
    }
    fill(&mut s, r, "#ff0000");
    sel(&mut s, &[outer, inner]);
    let c = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    let same = |s: &mut Session, from: NodeId, cmd: &str| {
        sel(s, &[from]);
        s.execute(cmd, &json!({})).unwrap();
        let mut v = selected(s);
        v.sort();
        v
    };
    for from in [a, c, outer] {
        assert_eq!(same(&mut s, from, "select.same.fillColor"), vec![a, b, c], "from {from:?}");
    }
    assert_eq!(same(&mut s, inner, "select.same.shapeType"), vec![c]);
    fill(&mut s, c, "#ff0000");
    assert_eq!(same(&mut s, a, "select.same.fillColor"), vec![a, b]);
    assert_eq!(same(&mut s, r, "select.same.fillColor"), vec![r, c]);
    // Isolated in the compound path, its member paths are the objects to match.
    s.execute("object.isolate", &json!({"id": c.0})).unwrap();
    assert_eq!(same(&mut s, outer, "select.same.shapeType"), vec![outer, inner]);
}

/// Select by Color needs no object to start from: it matches fills and strokes as
/// `document.inspect` reports them, `#rrggbb`.
#[test]
fn select_color_matches_fill_and_stroke_without_a_selection() {
    let mut s = session();
    let [a, b, c] = [0.0, 20.0, 40.0].map(|x| rect(&mut s, x, 0.0, 10.0, 10.0));
    let t = text(&mut s, "a");
    fill(&mut s, a, "#000000");
    fill(&mut s, t, "#000000");
    // A grey as a PDF stores it reads as #fefefe.
    s.execute("paint.setFill", &json!({"ids": [b.0], "color": [0.996, 0.996, 0.996]})).unwrap();
    fill(&mut s, c, "#ff0000");
    s.execute("paint.setStroke", &json!({"ids": [a.0, c.0], "color": "#0000ff"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let pick = |s: &mut Session, p: Value| {
        s.execute("select.color", &p).unwrap();
        let mut v = selected(s);
        v.sort();
        v
    };
    assert_eq!(pick(&mut s, json!({"fill": "#000000"})), vec![a, t]);
    assert_eq!(pick(&mut s, json!({"fill": "#fefefe"})), vec![b]);
    assert_eq!(pick(&mut s, json!({"stroke": "#0000ff"})), vec![a, c]);
    assert_eq!(pick(&mut s, json!({"fill": "#000000", "stroke": "#0000ff"})), vec![a]);
    // Select > Reselect repeats it.
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.reselect", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a]);
    // A compound path reads as its own paint, never its members'.
    let outer = rect(&mut s, 100.0, 0.0, 50.0, 50.0);
    let inner = rect(&mut s, 110.0, 10.0, 20.0, 20.0);
    fill(&mut s, outer, "#000000");
    fill(&mut s, inner, "#000000");
    sel(&mut s, &[outer, inner]);
    let compound = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    assert_eq!(pick(&mut s, json!({"fill": "#000000"})), vec![a, t, compound]);
    fill(&mut s, compound, "#ff0000");
    assert_eq!(pick(&mut s, json!({"fill": "#000000"})), vec![a, t]);
    assert_eq!(pick(&mut s, json!({"fill": "#ff0000"})), vec![c, compound]);
    // Type without characters reads as the default character fill, black.
    let empty = text(&mut s, "x");
    if let Some(NodeKind::Text(t)) = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(empty).map(|n| &mut n.kind) {
        t.runs.clear();
    }
    assert_eq!(pick(&mut s, json!({"fill": "#000000"})), vec![a, t, empty]);
    assert!(s.execute("select.color", &json!({})).is_err());
    assert!(s.execute("select.color", &json!({"fill": "not a colour"})).is_err());
}

#[test]
fn select_direction_handles_selects_all_anchors() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.object.directionHandles", &json!({})).unwrap()["anchors"], 4);
    assert_eq!(s.doc().unwrap().selection.anchors.get(&a).unwrap().len(), 4);
}

#[test]
fn direction_handles_leave_out_hidden_and_locked_paths() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let hidden = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let locked = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, hidden, locked]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.setProps", &json!({"ids": [hidden.0], "visible": false})).unwrap();
    s.execute("object.setProps", &json!({"ids": [locked.0], "locked": true})).unwrap();
    sel(&mut s, &[g]);
    assert_eq!(s.execute("select.object.directionHandles", &json!({})).unwrap()["anchors"], 4);
    assert_eq!(selected(&s), vec![a]);
}

#[test]
fn select_point_and_area_text() {
    let mut s = session();
    let p = text(&mut s, "point");
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 0, "text": "area", "area": {"width": 100, "height": 50}})).unwrap());
    let _ = rect(&mut s, 0.0, 0.0, 5.0, 5.0);
    assert_eq!(s.execute("select.object.pointText", &json!({})).unwrap()["count"], 1);
    assert_eq!(selected(&s), vec![p]);
    s.execute("select.object.areaText", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a]);
    assert_eq!(s.execute("select.object.brushStrokes", &json!({})).unwrap()["count"], 0);
}

/// Select › Object › Bristle Brush Strokes goes by the brush's type: a bristle brush counts
/// whatever it is named, and a brush of another type with "Bristle" in its name does not.
#[test]
fn bristle_brush_strokes_select_by_brush_type() {
    let mut s = session();
    let round = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let soft = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let nib = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    let _plain = rect(&mut s, 60.0, 0.0, 10.0, 10.0);
    s.execute("brush.apply", &json!({"name": "Bristle Round", "ids": [round.0]})).unwrap();
    s.execute("brush.apply", &json!({"name": "Bristle Flat Fan", "ids": [soft.0]})).unwrap();
    s.execute("brush.options", &json!({"name": "Bristle Flat Fan", "newName": "Soft"})).unwrap();
    s.execute("brush.new", &json!({"type": "calligraphic", "name": "Bristle-like"})).unwrap();
    s.execute("brush.apply", &json!({"name": "Bristle-like", "ids": [nib.0]})).unwrap();
    let brush = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).and_then(|n| n.appearance.stroke()).and_then(|st| st.brush.clone());
    assert_eq!(brush(&s, soft).as_deref(), Some("Soft"));
    assert_eq!(brush(&s, nib).as_deref(), Some("Bristle-like"));
    assert_eq!(s.execute("select.object.bristleBrushStrokes", &json!({})).unwrap()["count"], 2);
    assert_eq!(selected(&s), vec![round, soft]);
}

/// Select › Object and Select › Same leave out hidden and locked objects, and the objects in hidden
/// or locked groups and sublayers, as Select All does.
#[test]
fn select_menu_commands_leave_out_hidden_and_locked_art() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let shown = text(&mut s, "shown");
    let hidden = text(&mut s, "hidden");
    let locked = text(&mut s, "locked");
    let in_hidden_group = text(&mut s, "in a hidden group");
    let in_locked_group = text(&mut s, "in a locked group");
    let open = id_of(&s.execute("shape.line", &json!({"x1": 0, "y1": 0, "x2": 10, "y2": 10})).unwrap());
    let hidden_open = id_of(&s.execute("shape.line", &json!({"x1": 0, "y1": 20, "x2": 10, "y2": 30})).unwrap());
    sel(&mut s, &[in_hidden_group]);
    let hidden_group = id_of(&s.execute("object.group", &json!({})).unwrap());
    sel(&mut s, &[in_locked_group]);
    let locked_group = id_of(&s.execute("object.group", &json!({})).unwrap());
    let hidden_sub = id_of(&s.execute("layer.newSublayer", &json!({"parent": layer.0})).unwrap());
    let in_hidden_sub = text(&mut s, "in a hidden sublayer");
    let locked_sub = id_of(&s.execute("layer.newSublayer", &json!({"parent": layer.0})).unwrap());
    let in_locked_sub = text(&mut s, "in a locked sublayer");
    assert_eq!(s.doc().unwrap().doc.parent_of(in_hidden_sub), Some(hidden_sub));
    assert_eq!(s.doc().unwrap().doc.parent_of(in_locked_sub), Some(locked_sub));
    s.execute("object.setProps", &json!({"ids": [hidden.0, hidden_open.0, hidden_group.0], "visible": false})).unwrap();
    s.execute("object.setProps", &json!({"ids": [locked.0, locked_group.0], "locked": true})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [hidden_sub.0], "visible": false})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [locked_sub.0], "locked": true})).unwrap();
    assert_eq!(s.execute("select.object.textObjects", &json!({})).unwrap()["count"], 1);
    assert_eq!(selected(&s), vec![shown]);
    assert_eq!(s.execute("select.object.pointText", &json!({})).unwrap()["count"], 1);
    assert_eq!(selected(&s), vec![shown]);
    assert_eq!(s.execute("select.same.fontSize", &json!({})).unwrap()["count"], 1);
    assert_eq!(selected(&s), vec![shown]);
    s.execute("select.object.openPaths", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![open]);
}

/// Next Object Above and Below pass over the hidden and locked objects and the sublayers beside the
/// selected object.
#[test]
fn next_object_above_and_below_pass_over_what_cant_be_selected() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let hidden = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let locked = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    let sub = id_of(&s.execute("layer.newSublayer", &json!({"parent": layer.0})).unwrap());
    s.execute("layer.setCurrent", &json!({"id": layer.0})).unwrap();
    let b = rect(&mut s, 60.0, 0.0, 10.0, 10.0);
    let order: Vec<NodeId> = s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(order, vec![a, hidden, locked, sub, b]);
    s.execute("object.setProps", &json!({"ids": [hidden.0], "visible": false})).unwrap();
    s.execute("object.setProps", &json!({"ids": [locked.0], "locked": true})).unwrap();
    sel(&mut s, &[a]);
    s.execute("select.nextAbove", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![b]);
    s.execute("select.nextBelow", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a]);
    // Nothing that can be selected below it: the selection stays.
    s.execute("select.nextBelow", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a]);
}

/// A keyboard shortcut runs `object.isolate` with no params; the first selected object is isolated (#2991).
#[test]
fn object_isolate_without_id_uses_the_selection() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    let _b = rect(&mut s, 100.0, 0.0, 50.0, 50.0);
    sel(&mut s, &[a]);
    s.execute("object.isolate", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().isolation, Some(a));
}

/// In isolation mode Select › Same and Select › Object select only objects in the isolated group.
#[test]
fn select_menu_commands_stay_inside_the_isolated_group() {
    let mut s = session();
    let outside = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let _text_outside = text(&mut s, "outside");
    let a = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    let text_inside = text(&mut s, "inside");
    for id in [outside, a, b] {
        fill(&mut s, id, "#ff0000");
    }
    sel(&mut s, &[a, b, text_inside]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    sel(&mut s, &[a]);
    s.execute("select.same.fillColor", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a, b]);
    s.execute("select.object.textObjects", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![text_inside]);
    s.execute("select.object.pointText", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![text_inside]);
    s.execute("select.same.fontSize", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![text_inside]);
    // All on Same Layers takes the objects of the isolated group.
    sel(&mut s, &[a]);
    s.execute("select.object.allOnSameLayers", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a, b, text_inside]);
}

/// In isolation mode the Layers panel's target circle and selection column can't select an object
/// outside the isolated group, so it can't be moved either (#2286).
#[test]
fn layers_panel_selects_nothing_outside_the_isolated_group() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    let b = rect(&mut s, 100.0, 0.0, 50.0, 50.0);
    let outside = rect(&mut s, 300.0, 0.0, 50.0, 50.0);
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let before = bounds(&s, outside);
    let _ = s.execute("layer.target", &json!({"id": outside.0}));
    assert!(!selected(&s).contains(&outside));
    let _ = s.execute("layer.target", &json!({"id": outside.0, "add": true}));
    assert!(!selected(&s).contains(&outside));
    let _ = s.execute("layer.selectAll", &json!({"id": outside.0}));
    assert!(!selected(&s).contains(&outside));
    let _ = s.execute("layer.selectAll", &json!({"id": outside.0, "add": true}));
    assert!(!selected(&s).contains(&outside));
    let _ = s.execute("object.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!(bounds(&s, outside), before);
    // Objects inside the group can still be targeted and selected.
    s.execute("layer.target", &json!({"id": a.0})).unwrap();
    assert_eq!(selected(&s), vec![a]);
    s.execute("layer.selectAll", &json!({"id": b.0})).unwrap();
    assert_eq!(selected(&s), vec![b]);
}

/// In isolation mode the selection column of the isolated layer's own row selects the art on it.
#[test]
fn layers_panel_selects_the_art_of_the_isolated_layer() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    let b = rect(&mut s, 100.0, 0.0, 50.0, 50.0);
    let layer = s.active().unwrap().doc.layers[0].id;
    s.execute("object.isolate", &json!({"id": layer.0})).unwrap();
    s.execute("layer.selectAll", &json!({"id": layer.0})).unwrap();
    let mut got = selected(&s);
    got.sort_by_key(|i| i.0);
    assert_eq!(got, vec![a, b]);
}

/// In isolation mode a recalled selection leaves out the objects outside the isolated group, and
/// Next Object Above selects nothing outside it.
#[test]
fn recalled_selections_and_next_object_stay_inside_the_isolated_group() {
    let mut s = session();
    let outside = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let _above_outside = rect(&mut s, 0.0, 20.0, 10.0, 10.0);
    let a = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[outside, a]);
    s.execute("select.save", &json!({"name": "Both"})).unwrap();
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("select.recall", &json!({"name": "Both"})).unwrap();
    assert_eq!(selected(&s), vec![a]);
    // An object outside the group (selected by id) has no neighbor in it: the selection stays.
    sel(&mut s, &[outside]);
    s.execute("select.nextAbove", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![outside]);
}

/// In isolation mode Open Paths and All on Same Layers select nothing when the isolated group, or
/// the layer it is on, is hidden or locked.
#[test]
fn select_menu_commands_leave_out_an_isolated_group_hidden_or_locked_since() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let a = id_of(&s.execute("shape.line", &json!({"x1": 0, "y1": 0, "x2": 10, "y2": 10})).unwrap());
    let b = id_of(&s.execute("shape.line", &json!({"x1": 0, "y1": 20, "x2": 10, "y2": 30})).unwrap());
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("select.object.openPaths", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a, b]);
    s.execute("layer.setProps", &json!({"ids": [layer.0], "visible": false})).unwrap();
    s.execute("select.object.openPaths", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![]);
    sel(&mut s, &[a]);
    s.execute("select.object.allOnSameLayers", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![]);
    s.execute("layer.setProps", &json!({"ids": [layer.0], "visible": true})).unwrap();
    s.execute("object.setProps", &json!({"ids": [g.0], "locked": true})).unwrap();
    s.execute("select.object.openPaths", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![]);
}

/// The selection after command `c` runs with nothing selected.
fn select_from_none(s: &mut Session, c: &str) -> Vec<NodeId> {
    s.execute("select.none", &json!({})).unwrap();
    s.execute(c, &json!({})).unwrap();
    selected(s)
}

/// In isolation mode Select All, All on Active Artboard and Inverse select nothing while the
/// isolated group is hidden or locked, or the layer it is on is hidden or a template.
#[test]
fn select_all_leaves_out_an_isolated_group_hidden_or_locked_since() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    let commands = ["select.all", "select.allOnArtboard", "select.inverse"];
    for c in commands {
        assert_eq!(select_from_none(&mut s, c), vec![a, b], "{c}");
    }
    let changes = [
        ("object.setProps", json!({"ids": [g.0], "visible": false}), json!({"ids": [g.0], "visible": true})),
        ("object.setProps", json!({"ids": [g.0], "locked": true}), json!({"ids": [g.0], "locked": false})),
        ("layer.setProps", json!({"ids": [layer.0], "visible": false}), json!({"ids": [layer.0], "visible": true})),
        // A template layer that has been unlocked again.
        ("layer.setProps", json!({"ids": [layer.0], "template": true, "locked": false}), json!({"ids": [layer.0], "template": false})),
    ];
    for (cmd, on, off) in changes {
        s.execute(cmd, &on).unwrap();
        for c in commands {
            assert_eq!(select_from_none(&mut s, c), vec![], "{c} after {cmd} {on}");
        }
        s.execute(cmd, &off).unwrap();
        for c in commands {
            assert_eq!(select_from_none(&mut s, c), vec![a, b], "{c} after {cmd} {off}");
        }
    }
}

/// Pattern editing and opacity mask editing isolate a new layer: Select All, All on Active
/// Artboard and Inverse select the art on it.
#[test]
fn select_all_in_pattern_and_mask_editing_selects_the_art_being_edited() {
    let mut s = session();
    let commands = ["select.all", "select.allOnArtboard", "select.inverse"];
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    let tile = selected(&s);
    assert_eq!(tile.len(), 1);
    for c in commands {
        assert_eq!(select_from_none(&mut s, c), tile, "{c} in pattern editing");
    }
    s.execute("object.pattern.done", &json!({})).unwrap();
    let obj = rect(&mut s, 0.0, 0.0, 200.0, 200.0);
    let m = rect(&mut s, 0.0, 0.0, 100.0, 200.0);
    sel(&mut s, &[obj, m]);
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    s.execute("transparency.editOpacityMask", &json!({})).unwrap();
    let mask = selected(&s);
    assert_eq!(mask.len(), 1);
    for c in commands {
        assert_eq!(select_from_none(&mut s, c), mask, "{c} in opacity mask editing");
    }
}

/// All on Active Artboard takes a line on the artboard's right or bottom edge, as it takes one on
/// the left or top edge. A line on the edge two artboards share is on both.
#[test]
fn select_all_on_artboard_takes_lines_on_its_right_and_bottom_edges() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50, "artboards": 2, "artboardLayout": {"layout": "row", "spacing": 0}})).unwrap();
    let mut line = |x1: f64, y1: f64, x2: f64, y2: f64| id_of(&s.execute("shape.line", &json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2})).unwrap());
    let (bottom, right, shared) = (line(10.0, 50.0, 40.0, 50.0), line(200.0, 10.0, 200.0, 40.0), line(100.0, 10.0, 100.0, 40.0));
    line(200.5, 10.0, 200.5, 40.0);
    let mut on = |i: u64| {
        s.execute("select.allOnArtboard", &json!({"artboard": i})).unwrap();
        selected(&s)
    };
    assert_eq!(on(0), vec![bottom, shared]);
    assert_eq!(on(1), vec![right, shared]);
}

/// All on Active Artboard refuses artboard 2^32 and selects nothing. A 32-bit build read it as
/// artboard 0.
#[test]
fn select_all_on_artboard_refuses_an_artboard_past_u32() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("select.allOnArtboard", &json!({"artboard": 1u64 << 32})).is_err());
    assert!(selected(&s).is_empty());
}

#[test]
fn save_and_recall_selection() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    assert_eq!(s.execute("select.save", &json!({})).unwrap()["name"], "Selection 1");
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute("select.recall", &json!({"name": "Selection 1"})).unwrap()["count"], 2);
    s.execute("select.editSaved", &json!({"name": "Selection 1", "newName": "Pair"})).unwrap();
    assert_eq!(s.execute("select.savedList", &json!({})).unwrap(), json!(["Pair"]));
    s.execute("select.editSaved", &json!({"name": "Pair", "delete": true})).unwrap();
    assert!(s.execute("select.recall", &json!({"name": "Pair"})).is_err());
}

#[test]
fn update_selection_replaces_the_recalled_saved_selection() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let c = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    s.execute("select.save", &json!({"name": "A"})).unwrap();
    s.execute("select.save", &json!({"name": "Other"})).unwrap();
    // Nothing recalled yet: Update Selection is off.
    assert!(s.execute("select.update", &json!({})).is_err());
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.recall", &json!({"name": "A"})).unwrap();
    s.execute("select.add", &json!({"ids": [c.0]})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "A");
    assert!(s.doc().unwrap().is_dirty());
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute("select.recall", &json!({"name": "A"})).unwrap()["count"], 3);
    assert_eq!(selected(&s), vec![a, b, c]);
    // Only the recalled one changes.
    s.execute("select.recall", &json!({"name": "Other"})).unwrap();
    assert_eq!(selected(&s), vec![a, b]);
    // It travels with the file.
    let loaded = vectorcraft_format::load(&vectorcraft_format::save_file(&s.doc().unwrap().doc)).unwrap();
    assert_eq!(loaded.saved_selections.iter().map(|x| x.objects.len()).collect::<Vec<_>>(), [3, 2]);
    // It is one undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.recall", &json!({"name": "A"})).unwrap();
    assert_eq!(selected(&s), vec![a, b]);
    // Nothing selected: Update Selection is off.
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("select.update", &json!({})).is_err());
    // It can run again without another recall.
    sel(&mut s, &[c]);
    s.execute("select.update", &json!({})).unwrap();
    sel(&mut s, &[b]);
    s.execute("select.update", &json!({})).unwrap();
    s.execute("select.recall", &json!({"name": "A"})).unwrap();
    assert_eq!(selected(&s), vec![b]);
}

#[test]
fn update_selection_follows_a_rename_and_stops_after_a_delete() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    s.execute("select.save", &json!({"name": "One"})).unwrap();
    s.execute("select.recall", &json!({"name": "One"})).unwrap();
    s.execute("select.editSaved", &json!({"name": "One", "newName": "Uno"})).unwrap();
    sel(&mut s, &[a, b]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "Uno");
    s.execute("select.recall", &json!({"name": "Uno"})).unwrap();
    assert_eq!(selected(&s), vec![a, b]);
    // A saved selection of the same name saved after the delete is not the recalled one.
    s.execute("select.editSaved", &json!({"name": "Uno", "delete": true})).unwrap();
    s.execute("select.save", &json!({"name": "Uno"})).unwrap();
    assert!(s.execute("select.update", &json!({})).is_err());
}

#[test]
fn update_selection_changes_the_recalled_saved_selection_after_undo_and_redo() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let c = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    let recall = |s: &mut Session, name: &str| {
        s.execute("select.recall", &json!({"name": name})).unwrap();
        selected(s)
    };
    sel(&mut s, &[a]);
    s.execute("select.save", &json!({"name": "A"})).unwrap();
    sel(&mut s, &[b]);
    s.execute("select.save", &json!({"name": "B"})).unwrap();
    recall(&mut s, "A");
    // Swap the two names, then undo the swap: Update Selection still changes the one recalled.
    let swap = json!({"edits": [{"name": "A", "newName": "B"}, {"name": "B", "newName": "A"}]});
    s.execute("select.editSaved", &swap).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[a, c]);
    s.execute("select.update", &json!({})).unwrap();
    assert_eq!((recall(&mut s, "A"), recall(&mut s, "B")), (vec![a, c], vec![b]));
    // Undo and redo a rename of the recalled one.
    recall(&mut s, "A");
    s.execute("select.editSaved", &json!({"name": "A", "newName": "First"})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    sel(&mut s, &[c]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "First");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[c]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "A");
    assert_eq!(recall(&mut s, "A"), vec![c]);
    // Undo and redo an Edit Selection… that renamed it and deleted the other one.
    recall(&mut s, "A");
    let edits = json!({"edits": [{"name": "A", "newName": "First"}, {"name": "B", "delete": true}]});
    s.execute("select.editSaved", &edits).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "First");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "A");
    assert_eq!((recall(&mut s, "A"), recall(&mut s, "B")), (vec![a], vec![b]));
    // A cancelled undo group that renamed it.
    recall(&mut s, "B");
    s.execute("edit.undoGroup.begin", &json!({})).unwrap();
    s.execute("select.editSaved", &json!({"name": "B", "newName": "Second"})).unwrap();
    s.execute("edit.undoGroup.end", &json!({"cancel": true})).unwrap();
    sel(&mut s, &[a, b]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "B");
    assert_eq!(recall(&mut s, "B"), vec![a, b]);
    // Undo an Update Selection that gave it the other one's objects.
    recall(&mut s, "A");
    sel(&mut s, &[a, b]);
    s.execute("select.update", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[c]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "A");
    assert_eq!((recall(&mut s, "A"), recall(&mut s, "B")), (vec![c], vec![a, b]));
    // Save Selection made after the recall, and undone.
    recall(&mut s, "A");
    s.execute("select.save", &json!({"name": "C"})).unwrap();
    sel(&mut s, &[b]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "A");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.update", &json!({})).unwrap()["name"], "A");
}

#[test]
fn edit_selection_needs_a_saved_selection_and_a_free_name() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    // Edit Selection… is off until something is saved.
    assert!(s.execute("select.editSaved", &json!({"name": "Selection 1", "newName": "A"})).is_err());
    s.execute("select.save", &json!({"name": "One"})).unwrap();
    s.execute("select.save", &json!({"name": "Two"})).unwrap();
    // A name already saved can't be taken; both selections keep theirs.
    assert!(s.execute("select.editSaved", &json!({"name": "One", "newName": " Two "})).is_err());
    assert_eq!(s.execute("select.savedList", &json!({})).unwrap(), json!(["One", "Two"]));
    // Renaming to its own name, or a blank name, changes nothing; a free name renames.
    s.execute("select.editSaved", &json!({"name": "One", "newName": "One"})).unwrap();
    s.execute("select.editSaved", &json!({"name": "One", "newName": "  "})).unwrap();
    s.execute("select.editSaved", &json!({"name": "One", "newName": " Uno "})).unwrap();
    assert_eq!(s.execute("select.savedList", &json!({})).unwrap(), json!(["Uno", "Two"]));
}

#[test]
fn saved_selections_live_in_the_document() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let list = |s: &mut Session| s.execute("select.savedList", &json!({})).unwrap();
    sel(&mut s, &[a]);
    s.execute("select.save", &json!({"name": "A"})).unwrap();
    sel(&mut s, &[a, b]);
    s.execute("select.save", &json!({"name": "Both"})).unwrap();
    // They travel with the file.
    let loaded = vectorcraft_format::load(&vectorcraft_format::save_file(&s.doc().unwrap().doc)).unwrap();
    assert_eq!(loaded.saved_selections.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["A", "Both"]);
    // Saving is an undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(list(&mut s), json!(["A"]));
    // An existing name is replaced by the current selection.
    sel(&mut s, &[b]);
    s.execute("select.save", &json!({"name": "A"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute("select.recall", &json!({"name": "A"})).unwrap()["count"], 1);
    assert_eq!(selected(&s), vec![b]);
    // Objects deleted since are left out of a recall.
    sel(&mut s, &[a, b]);
    s.execute("select.save", &json!({"name": "Pair"})).unwrap();
    sel(&mut s, &[a]);
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(s.execute("select.recall", &json!({"name": "Pair"})).unwrap()["count"], 1);
}

/// A recall leaves out the objects hidden or locked since the save, themselves or through their
/// group or sublayer.
#[test]
fn recalled_selections_leave_out_objects_hidden_or_locked_since() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let hidden = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let locked = rect(&mut s, 40.0, 0.0, 10.0, 10.0);
    let in_group = rect(&mut s, 60.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[in_group]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let sub = id_of(&s.execute("layer.newSublayer", &json!({"parent": layer.0})).unwrap());
    let in_sub = rect(&mut s, 80.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, hidden, locked, in_group, in_sub]);
    s.execute("select.save", &json!({"name": "All"})).unwrap();
    s.execute("object.setProps", &json!({"ids": [hidden.0, g.0], "visible": false})).unwrap();
    s.execute("object.setProps", &json!({"ids": [locked.0], "locked": true})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [sub.0], "locked": true})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute("select.recall", &json!({"name": "All"})).unwrap()["count"], 1);
    assert_eq!(selected(&s), vec![a]);
}

#[test]
fn edit_selection_applies_several_edits_as_one_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    for n in ["A", "B", "C"] {
        s.execute("select.save", &json!({ "name": n })).unwrap();
    }
    let list = |s: &mut Session| s.execute("select.savedList", &json!({})).unwrap();
    // Edits name the selections as they are, so two can swap names; a delete goes with them.
    s.execute("select.editSaved", &json!({"edits": [{"name": "A", "newName": "B"}, {"name": "B", "newName": "A"}, {"name": "C", "delete": true}]}))
        .unwrap();
    assert_eq!(list(&mut s), json!(["B", "A"]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(list(&mut s), json!(["A", "B", "C"]));
    // Two ending with one name, or an unknown name, change nothing.
    assert!(s.execute("select.editSaved", &json!({"edits": [{"name": "A", "newName": "B"}]})).is_err());
    assert!(s.execute("select.editSaved", &json!({"edits": [{"name": "Z", "delete": true}]})).is_err());
    assert_eq!(list(&mut s), json!(["A", "B", "C"]));
}

#[test]
fn a_document_keeps_at_most_25_saved_selections() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    for i in 0..25 {
        s.execute("select.save", &json!({ "name": format!("S{i}") })).unwrap();
    }
    assert!(s.execute("select.save", &json!({"name": "One too many"})).is_err());
    // A name already saved is replaced even when the list is full.
    s.execute("select.save", &json!({"name": "S3"})).unwrap();
    // Unnamed saves take the first free "Selection N".
    s.execute("select.editSaved", &json!({"name": "S0", "delete": true})).unwrap();
    assert_eq!(s.execute("select.save", &json!({})).unwrap()["name"], "Selection 1");
}

// ---------- Type ----------

#[test]
fn change_case_variants() {
    let mut s = session();
    let t = text(&mut s, "hello wORLD. again here");
    sel(&mut s, &[t]);
    s.execute("type.changeCase", &json!({"case": "upper"})).unwrap();
    assert_eq!(plain(&s, t), "HELLO WORLD. AGAIN HERE");
    s.execute("type.changeCase", &json!({"case": "lower"})).unwrap();
    assert_eq!(plain(&s, t), "hello world. again here");
    s.execute("type.changeCase", &json!({"case": "title"})).unwrap();
    assert_eq!(plain(&s, t), "Hello World. Again Here");
    s.execute("type.changeCase", &json!({"case": "sentence"})).unwrap();
    assert_eq!(plain(&s, t), "Hello world. Again here");
    assert!(s.execute("type.changeCase", &json!({"case": "weird"})).is_err());
}

/// Georgian has no word capitals: title and sentence case leave Mkhedruli lower case (and bring
/// Mtavruli back to it) instead of capitalizing it; upper case still gives Mtavruli.
#[test]
fn change_case_leaves_georgian_uncapitalized() {
    let mut s = session();
    let t = text(&mut s, "ფენის სახელი. ახალი Layer");
    sel(&mut s, &[t]);
    s.execute("type.changeCase", &json!({"case": "title"})).unwrap();
    assert_eq!(plain(&s, t), "ფენის სახელი. ახალი Layer");
    s.execute("type.changeCase", &json!({"case": "sentence"})).unwrap();
    assert_eq!(plain(&s, t), "ფენის სახელი. ახალი layer");
    s.execute("type.changeCase", &json!({"case": "upper"})).unwrap();
    assert_eq!(plain(&s, t), "ᲤᲔᲜᲘᲡ ᲡᲐᲮᲔᲚᲘ. ᲐᲮᲐᲚᲘ LAYER");
    s.execute("type.changeCase", &json!({"case": "title"})).unwrap();
    assert_eq!(plain(&s, t), "ფენის სახელი. ახალი Layer");
}

/// Change Case is one undo step, and Change Case to the case the text has already records no undo
/// step: the step to redo stays and the document stays unmodified. It still replies with the
/// objects.
#[test]
fn change_case_to_the_case_the_text_has_records_no_undo_step() {
    let mut s = session();
    let t = text(&mut s, "hello wORLD. again here");
    for case in ["upper", "lower", "title", "sentence"] {
        sel(&mut s, &[t]);
        let undo = history(&s).0;
        s.execute("type.changeCase", &json!({"case": case})).unwrap();
        assert_eq!(history(&s), (undo + 1, 0, true), "{case}");
        select_with_a_step_to_redo(&mut s, &[t]);
        let before = (history(&s), plain(&s, t));
        assert_eq!(s.execute("type.changeCase", &json!({"case": case})).unwrap(), json!({"ids": [t.0]}), "{case} again");
        assert_eq!((history(&s), plain(&s, t)), before, "{case} again");
    }
}

#[test]
fn smart_punctuation_quotes_dashes_ellipsis() {
    let mut s = session();
    let t = text(&mut s, "\"Hi\" it's 1--2 and 3---4...");
    sel(&mut s, &[t]);
    assert_eq!(s.execute("type.smartPunctuation", &json!({})).unwrap()["changed"], 1);
    assert_eq!(plain(&s, t), "“Hi” it’s 1–2 and 3—4…");
}

/// Smart Punctuation on type with nothing left to change, or on a selection with no type in it,
/// records no undo step and replies `changed: 0`.
#[test]
fn smart_punctuation_with_nothing_to_change_records_no_undo_step() {
    let mut s = session();
    let t = text(&mut s, "\"Hi\" it's 1--2 and 3---4...");
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[t]);
    assert_eq!(s.execute("type.smartPunctuation", &json!({})).unwrap()["changed"], 1);
    for id in [t, r] {
        select_with_a_step_to_redo(&mut s, &[id]);
        let before = history(&s);
        assert_eq!(s.execute("type.smartPunctuation", &json!({})).unwrap(), json!({"changed": 0}), "{id:?}");
        assert_eq!(history(&s), before, "{id:?}");
    }
    assert_eq!(plain(&s, t), "“Hi” it’s 1–2 and 3—4…");
}

#[test]
fn convert_point_area_roundtrip() {
    let mut s = session();
    let t = text(&mut s, "Some words");
    sel(&mut s, &[t]);
    s.execute("type.convertToAreaType", &json!({})).unwrap();
    let NodeKind::Text(tt) = &node(&s, t).kind else { panic!() };
    assert!(matches!(tt.kind, TextKind::Area { .. }));
    assert_eq!(tt.plain_text(), "Some words");
    s.execute("type.convertToPointType", &json!({})).unwrap();
    let NodeKind::Text(tt) = &node(&s, t).kind else { panic!() };
    assert!(matches!(tt.kind, TextKind::Point));
    assert_eq!(tt.plain_text(), "Some words");
}

/// Convert To Area Type with no point type selected, and Convert To Point Type with no area type
/// selected, record no undo step and reply with the objects. With point type and area type
/// selected, Convert To Area Type converts the point type in one undo step, and Convert To Point
/// Type then converts both in another.
#[test]
fn converting_type_to_the_kind_it_is_records_no_undo_step() {
    let mut s = session();
    let p = text(&mut s, "Point");
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 100, "text": "Area", "area": {"width": 100, "height": 50}})).unwrap());
    for (c, id) in [("type.convertToAreaType", a), ("type.convertToPointType", p)] {
        select_with_a_step_to_redo(&mut s, &[id]);
        let before = history(&s);
        assert_eq!(s.execute(c, &json!({})).unwrap(), json!({"ids": [id.0]}), "{c}");
        assert_eq!(history(&s), before, "{c}");
    }
    for c in ["type.convertToAreaType", "type.convertToPointType"] {
        select_with_a_step_to_redo(&mut s, &[p, a]);
        let undo = history(&s).0;
        s.execute(c, &json!({})).unwrap();
        assert_eq!(history(&s), (undo + 1, 0, true), "{c}");
    }
}

#[test]
fn area_to_point_turns_wraps_into_breaks() {
    let mut s = session();
    let a = id_of(
        &s.execute("text.create", &json!({"x": 0, "y": 0, "text": "one two three four five six", "area": {"width": 40, "height": 200}})).unwrap(),
    );
    sel(&mut s, &[a]);
    s.execute("type.convertToPointType", &json!({})).unwrap();
    let txt = plain(&s, a);
    assert!(txt.contains('\n'), "{txt:?}");
    assert_eq!(txt.replace('\n', " "), "one two three four five six");
}

#[test]
fn placeholder_and_insert_characters() {
    let mut s = session();
    let t = text(&mut s, "x");
    sel(&mut s, &[t]);
    s.execute("type.fillPlaceholder", &json!({})).unwrap();
    assert!(plain(&s, t).len() > 10);
    s.execute("text.setText", &json!({"text": "A"})).unwrap();
    s.execute("type.insert", &json!({"char": "emDash"})).unwrap();
    s.execute("type.insert", &json!({"char": "copyright"})).unwrap();
    s.execute("type.insert", &json!({"text": "z"})).unwrap();
    assert_eq!(plain(&s, t), "A—©z");
    assert!(s.execute("type.insert", &json!({"char": "nope"})).is_err());
}

#[test]
fn area_placeholder_fills_frame() {
    let mut s = session();
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 0, "text": "", "area": {"width": 300, "height": 300}})).unwrap());
    sel(&mut s, &[a]);
    s.execute("type.fillPlaceholder", &json!({})).unwrap();
    assert!(plain(&s, a).len() > 400);
}

/// Fill with Placeholder Text on type that holds its placeholder text already (point type, and area
/// type filled to its frame), and `type.insert` with an empty `text`, record no undo step and reply
/// with the objects. Inserting text is one undo step.
#[test]
fn placeholder_and_insert_that_change_nothing_record_no_undo_step() {
    let mut s = session();
    let t = text(&mut s, "x");
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 100, "text": "", "area": {"width": 300, "height": 300}})).unwrap());
    sel(&mut s, &[t, a]);
    s.execute("type.fillPlaceholder", &json!({})).unwrap();
    select_with_a_step_to_redo(&mut s, &[t, a]);
    let before = (history(&s), plain(&s, t), plain(&s, a));
    for (c, p) in [("type.fillPlaceholder", json!({})), ("type.insert", json!({"text": ""}))] {
        assert_eq!(s.execute(c, &p).unwrap(), json!({"ids": [t.0, a.0]}), "{c}");
        assert_eq!((history(&s), plain(&s, t), plain(&s, a)), before, "{c}");
    }
    s.execute("type.insert", &json!({"text": "z"})).unwrap();
    assert_eq!(history(&s), (before.0.0 + 1, 0, true));
}

#[test]
fn font_and_size_menu_items_use_set_style() {
    let mut s = session();
    let t = text(&mut s, "x");
    sel(&mut s, &[t]);
    s.execute("text.setStyle", &json!({"size": 36})).unwrap();
    s.execute("text.setStyle", &json!({"font": "Inter"})).unwrap();
    let NodeKind::Text(tt) = &node(&s, t).kind else { panic!() };
    assert_eq!(tt.first_style().size, 36.0);
    assert_eq!(tt.first_style().font_family, "Inter");
}

// ---------- View → Guides ----------

#[test]
fn make_release_clear_guides() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    assert_eq!(s.execute("view.guides.make", &json!({})).unwrap()["count"], 1);
    assert!(matches!(node(&s, a).kind, NodeKind::Path { guide: true, .. }));
    assert!(selected(&s).is_empty());
    assert_eq!(s.execute("view.guides.release", &json!({})).unwrap()["count"], 1);
    assert!(matches!(node(&s, a).kind, NodeKind::Path { guide: false, .. }));
    sel(&mut s, &[a]);
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    let n = undo_len(&s);
    assert_eq!(s.execute("view.guides.clear", &json!({})).unwrap()["count"], 2);
    assert!(s.doc().unwrap().doc.guides.is_empty());
    assert!(s.doc().unwrap().doc.node(a).is_none());
    assert_eq!(undo_len(&s), n + 1);
}

/// The anchor positions of `id`, which must be a path of one open subpath, and whether it is still a guide.
fn line_ends(s: &Session, id: NodeId) -> (Vec<(f64, f64)>, bool) {
    let NodeKind::Path { path, guide, .. } = &node(s, id).kind else { panic!("not a path") };
    let [sub] = &path.subpaths[..] else { panic!("not one subpath") };
    assert!(!sub.closed);
    (sub.anchors.iter().map(|a| (a.p.x, a.p.y)).collect(), *guide)
}

/// Release Guides turns ruler guides into paths with no fill or stroke, each in one undo step. An
/// artboard guide becomes a line across its artboard, and a canvas guide becomes a line across the
/// bounds of all the artboards.
#[test]
fn release_guides_turns_ruler_guides_into_paths() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    let g = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[g]);
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 50, "artboard": 0})).unwrap();
    let d = &s.doc().unwrap().doc;
    let (ab, all) = (d.artboards[0].rect, d.artboards[0].rect.union(d.artboards[1].rect));
    assert!(all.x1 > ab.x1);
    let layer = s.doc().unwrap().current_layer();
    // The selected artboard guide goes, and a selected path across its artboard takes its place.
    s.execute("guide.select", &json!({"indexes": [1]})).unwrap();
    let n = undo_len(&s);
    assert_eq!(s.execute("view.guides.release", &json!({})).unwrap()["count"], 1);
    assert_eq!((guides(&s), undo_len(&s)), (vec![(false, 300.0)], n + 1));
    let [line] = selected(&s)[..] else { panic!("one path selected") };
    assert_eq!(line_ends(&s, line), (vec![(ab.x0, 50.0), (ab.x1, 50.0)], false));
    assert_eq!(node(&s, line).appearance, Appearance::basic(Paint::None, Paint::None, 0.0));
    assert_eq!(s.doc().unwrap().doc.parent_of(line), layer);
    assert!(matches!(node(&s, g).kind, NodeKind::Path { guide: true, .. }), "the guide path stays a guide");
    // Undo brings the guide back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s), vec![(false, 300.0), (false, 50.0)]);
    assert!(s.doc().unwrap().doc.node(line).is_none());
    // index releases the canvas guide as a line across the artboards.
    assert_eq!(s.execute("view.guides.release", &json!({"index": 0})).unwrap()["count"], 1);
    assert_eq!(guides(&s), vec![(false, 50.0)]);
    let [line] = selected(&s)[..] else { panic!("one path selected") };
    assert_eq!(line_ends(&s, line), (vec![(all.x0, 300.0), (all.x1, 300.0)], false));
    // ids releases those guide paths.
    assert_eq!(s.execute("view.guides.release", &json!({"ids": [g.0]})).unwrap()["count"], 1);
    assert!(matches!(node(&s, g).kind, NodeKind::Path { guide: false, .. }));
    assert_eq!(selected(&s), vec![g]);
    // Locked guides are not released by index or ids.
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    for p in [json!({"index": 0}), json!({"ids": [g.0]})] {
        assert!(matches!(s.execute("view.guides.release", &p), Err(EngineError::Other(m)) if m == "guides are locked"), "{p}");
    }
    assert_eq!(guides(&s).len(), 1);
}

#[test]
fn ruler_guides_and_lock() {
    let mut s = session();
    assert_eq!(s.execute("guide.add", &json!({"vertical": false, "pos": 50})).unwrap()["index"], 0);
    s.execute("guide.move", &json!({"index": 0, "pos": 70})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[0].pos, 70.0);
    assert_eq!(s.execute("view.guides.lock", &json!({})).unwrap()["locked"], true);
    assert!(s.guides_locked());
    assert!(matches!(s.execute("guide.remove", &json!({"index": 0})), Err(EngineError::Disabled(..))));
    s.execute("view.guides.lock", &json!({})).unwrap();
    s.execute("guide.remove", &json!({"index": 0})).unwrap();
    assert!(s.doc().unwrap().doc.guides.is_empty());
    assert!(s.execute("guide.remove", &json!({"index": 0})).is_err());
}

/// #1534: Lock Guides locks the paths made into guides too: they can't be selected (so neither
/// moved, reshaped nor deleted), by a click, a marquee or Select All, and locking lets them go.
#[test]
fn locked_guides_lock_guide_paths() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0, 10.0), rect(&mut s, 20.0, 20.0, 10.0, 10.0));
    sel(&mut s, &[a]);
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    sel(&mut s, &[a, b]);
    assert_eq!(selected(&s), vec![b]);
    s.execute("select.all", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![b]);
    s.execute("view.guides.lock", &json!({"locked": false})).unwrap();
    sel(&mut s, &[a, b]);
    assert_eq!(selected(&s), vec![a, b]);
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    assert_eq!(selected(&s), vec![b]);
}

/// A rectangle at (`x`, 0), 10 pt wide, made into a guide; nothing is selected after.
fn guide_path(s: &mut Session, x: f64) -> NodeId {
    let g = rect(s, x, 0.0, 10.0, 10.0);
    sel(s, &[g]);
    s.execute("view.guides.make", &json!({})).unwrap();
    g
}

/// Exit Isolation Mode selects the isolated art, except a guide path while Lock Guides is on, so
/// a move after it leaves the guide where it is.
#[test]
fn exit_isolation_leaves_a_locked_guide_path_deselected() {
    let mut s = session();
    let g = guide_path(&mut s, 0.0);
    // A locked guide path can't be isolated; one isolated before Lock Guides went on stays isolated.
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    assert!(s.execute("object.isolate", &json!({"id": g.0})).is_err());
    s.execute("view.guides.lock", &json!({"locked": false})).unwrap();
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert!(selected(&s).is_empty());
    let _ = s.execute("object.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!(bounds(&s, g).x0, 0.0);
    // Lock Guides off: the guide is selected.
    s.execute("view.guides.lock", &json!({"locked": false})).unwrap();
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![g]);
}

/// Undo and Redo bring back a selection without the guides, ruler guides or guide paths, that
/// Lock Guides has locked since, so a move after them leaves the guides where they are.
#[test]
fn undo_and_redo_leave_locked_guides_deselected() {
    let mut s = session();
    let g = guide_path(&mut s, 0.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    let select_both = |s: &mut Session| {
        sel(s, &[g]);
        s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    };
    select_both(&mut s);
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert_eq!((bounds(&s, g).x0, guides(&s)), (10.0, vec![(true, 110.0)]));
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((selected(&s), selected_guides(&s)), (vec![], vec![]), "after Undo");
    let _ = s.execute("object.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!((bounds(&s, g).x0, guides(&s)), (0.0, vec![(true, 100.0)]));
    // Undone with both selected, redone once they are locked.
    s.execute("view.guides.lock", &json!({"locked": false})).unwrap();
    select_both(&mut s);
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!((selected(&s), selected_guides(&s)), (vec![], vec![]), "after Redo");
    let _ = s.execute("object.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!((bounds(&s, g).x0, guides(&s)), (10.0, vec![(true, 110.0)]));
}

/// Unlock All, Show All and Paste select the art they unlock, show or paste, except guide paths
/// while Lock Guides is on.
#[test]
fn unlock_all_show_all_and_paste_leave_locked_guide_paths_deselected() {
    let mut s = session();
    let (g, h) = (guide_path(&mut s, 0.0), guide_path(&mut s, 20.0));
    sel(&mut s, &[g]);
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    sel(&mut s, &[h]);
    s.execute("object.hide", &json!({})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    assert_eq!(s.execute("object.unlockAll", &json!({})).unwrap()["count"], 1);
    assert!(selected(&s).is_empty(), "after Unlock All");
    assert_eq!(s.execute("object.showAll", &json!({})).unwrap()["count"], 1);
    assert!(selected(&s).is_empty(), "after Show All");
    s.execute("edit.paste", &json!({})).unwrap();
    assert!(selected(&s).is_empty(), "after Paste");
    // Lock Guides off: the pasted guide is selected.
    s.execute("view.guides.lock", &json!({"locked": false})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let [copy] = selected(&s)[..] else { panic!("the pasted guide is selected") };
    assert!(crate::is_guide_path(&s.doc().unwrap().doc, copy));
}

/// Lock Guides deselects the guides of every open document, not only the active one's.
#[test]
fn lock_guides_deselects_the_guides_of_every_open_document() {
    let mut s = session();
    let g = guide_path(&mut s, 0.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    sel(&mut s, &[g]);
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!((selected(&s), selected_guides(&s)), (vec![], vec![]));
}

/// A failed batch and a canceled undo group bring back the selection from before them, without
/// the guide paths that Lock Guides locked inside them.
#[test]
fn a_failed_batch_and_a_canceled_undo_group_leave_locked_guides_deselected() {
    let mut s = session();
    let g = guide_path(&mut s, 0.0);
    sel(&mut s, &[g]);
    let steps = json!([{"command": "view.guides.lock", "params": {"locked": true}}, {"command": "object.isolate", "params": {}}]);
    assert!(s.execute("command.batch", &json!({"commands": steps})).is_err());
    assert!(s.guides_locked());
    assert!(selected(&s).is_empty(), "after the failed batch");
    s.execute("view.guides.lock", &json!({"locked": false})).unwrap();
    sel(&mut s, &[g]);
    s.execute("edit.undoGroup.begin", &json!({})).unwrap();
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    s.execute("edit.undoGroup.end", &json!({"cancel": true})).unwrap();
    assert_eq!(bounds(&s, g).x0, 0.0);
    assert!(selected(&s).is_empty(), "after the canceled undo group");
}

/// Canceling a drag or a dialog's preview brings back the selection from before it, without the
/// guide paths that Lock Guides locked during it.
#[test]
fn a_canceled_interaction_leaves_locked_guides_deselected() {
    let mut s = session();
    let g = guide_path(&mut s, 0.0);
    sel(&mut s, &[g]);
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    s.cancel_interaction().unwrap();
    assert_eq!(bounds(&s, g).x0, 0.0);
    assert!(selected(&s).is_empty());
}

/// A drag preview that fails puts back the selection from before the drag, without the guide
/// paths that Lock Guides locked during it, so a move after the drag moves nothing.
#[test]
fn a_failed_preview_leaves_locked_guides_deselected() {
    let mut s = session();
    let g = guide_path(&mut s, 0.0);
    sel(&mut s, &[g]);
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    assert!(s.preview("object.move", &json!({"dx": "far", "dy": 0})).is_err());
    assert!(selected(&s).is_empty());
    s.commit_interaction().unwrap();
    let _ = s.execute("object.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!(bounds(&s, g).x0, 0.0);
}

fn guides(s: &Session) -> Vec<(bool, f64)> {
    s.doc().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos)).collect()
}

fn selected_guides(s: &Session) -> Vec<usize> {
    s.doc().unwrap().selection.guides.clone()
}

/// #414: ruler guides are selected, moved, copied, nudged and deleted like art, over commands.
#[test]
fn ruler_guides_select_move_and_delete() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    for (vertical, pos) in [(true, 100.0), (false, 50.0), (true, 300.0)] {
        s.execute("guide.add", &json!({"vertical": vertical, "pos": pos})).unwrap();
    }
    sel(&mut s, &[a]);
    // Selecting guides deselects the art; toggle adds and removes.
    assert_eq!(s.execute("guide.select", &json!({"indexes": [0]})).unwrap()["selected"], json!([0]));
    assert!(selected(&s).is_empty());
    assert_eq!(s.execute("guide.select", &json!({"indexes": [1], "toggle": true})).unwrap()["selected"], json!([0, 1]));
    let layer = s.doc().unwrap().current_layer().unwrap().0;
    assert_eq!(
        s.execute("guide.list", &json!({})).unwrap()[1],
        json!({"index": 1, "vertical": false, "pos": 50.0, "selected": true, "layer": layer, "shown": true, "editable": true})
    );
    assert!(s.execute("guide.select", &json!({"indexes": [7]})).is_err());
    // A move takes each selected guide along its own axis, in one undo step.
    let n = undo_len(&s);
    s.execute("guide.move", &json!({"dx": 10, "dy": -5})).unwrap();
    assert_eq!(guides(&s), [(true, 110.0), (false, 45.0), (true, 300.0)]);
    assert_eq!(undo_len(&s), n + 1);
    // Alt: copies, which are then the selection.
    s.execute("guide.move", &json!({"dx": 20, "copy": true})).unwrap();
    assert_eq!(guides(&s).len(), 5);
    assert_eq!(&guides(&s)[3..], [(true, 130.0), (false, 45.0)]);
    assert_eq!(selected_guides(&s), [3, 4]);
    // The arrow keys nudge them.
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0, "big": true})).unwrap();
    assert_eq!(guides(&s)[3], (true, 140.0));
    // Delete (edit.clear) deletes the selected guides; the others keep their places.
    s.execute("guide.select", &json!({"indexes": [1, 3]})).unwrap();
    assert_eq!(s.execute("guide.list", &json!({})).unwrap().as_array().unwrap().len(), 5);
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(guides(&s), [(true, 110.0), (true, 300.0), (false, 45.0)]);
    assert!(selected_guides(&s).is_empty());
    assert!(s.doc().unwrap().doc.node(a).is_some(), "the art stays");
    // Removing one keeps the rest of the selection pointing at the same guides.
    s.execute("guide.select", &json!({"indexes": [1, 2]})).unwrap();
    assert_eq!(s.execute("guide.remove", &json!({"index": 0})).unwrap()["count"], 1);
    assert_eq!((guides(&s), selected_guides(&s)), (vec![(true, 300.0), (false, 45.0)], vec![0, 1]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((guides(&s).len(), selected_guides(&s)), (3, vec![1, 2]));
    // Out of range positions are refused.
    assert!(s.execute("guide.move", &json!({"index": 0, "pos": 1e300})).is_err());
    assert!(s.execute("guide.add", &json!({"vertical": true, "pos": -1e300})).is_err());
    // Locking deselects them, and they can't be picked again until unlocked.
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    assert!(selected_guides(&s).is_empty());
    assert!(matches!(s.execute("guide.select", &json!({"indexes": [0]})), Err(EngineError::Disabled(..))));
    assert!(matches!(s.execute("edit.clear", &json!({})), Err(EngineError::Disabled(..))));
}

/// Shift selects ruler guides and art together, in either order; Delete takes both in one step.
#[test]
fn guides_and_art_are_shift_selected_together() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 40.0, 20.0);
    let b = rect(&mut s, 70.0, 50.0, 60.0, 30.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    // Art first, then a Shift-click on a guide: the art stays selected.
    sel(&mut s, &[a]);
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    assert_eq!((selected(&s), selected_guides(&s)), (vec![a], vec![0]));
    // A guide first, then Shift-added art: the guide stays selected.
    s.execute("guide.select", &json!({"indexes": [1]})).unwrap();
    s.execute("select.add", &json!({"ids": [b.0]})).unwrap();
    assert_eq!((selected(&s), selected_guides(&s)), (vec![b], vec![1]));
    // A new selection drops them.
    sel(&mut s, &[a]);
    assert!(selected_guides(&s).is_empty());
    // Delete takes the art and the guides, in one step; undo brings both back.
    s.execute("guide.select", &json!({"indexes": [1], "toggle": true})).unwrap();
    let n = undo_len(&s);
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!((guides(&s), undo_len(&s)), (vec![(true, 200.0)], n + 1));
    assert!(s.doc().unwrap().doc.node(a).is_none() && s.doc().unwrap().doc.node(b).is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s).len(), 2);
    // Ids given: only those go, the guides stay.
    s.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    s.execute("select.add", &json!({"ids": [b.0]})).unwrap();
    s.execute("edit.clear", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(guides(&s).len(), 2);
}

/// When ruler guides and art are selected together, moving the art moves the guides as well;
/// copy leaves both in place and moves the copies; single undo reverts both.
#[test]
fn guides_and_art_move_together_when_selected() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 40.0, 20.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();

    // Select art and both guides.
    sel(&mut s, &[a]);
    s.execute("guide.select", &json!({"indexes": [0, 1], "toggle": true})).unwrap();
    assert_eq!((selected(&s), selected_guides(&s)), (vec![a], vec![0, 1]));

    // 1. Moving via object.transform moves both art and guides in one step.
    let n = undo_len(&s);
    s.execute("object.transform", &json!({"matrix": [1, 0, 0, 1, 25, 35]})).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert_eq!((b.x0, b.y0), (35.0, 45.0));
    assert_eq!(guides(&s), vec![(true, 225.0), (false, 335.0)]);
    assert_eq!(undo_len(&s), n + 1);

    // Undo restores both in one step.
    s.execute("edit.undo", &json!({})).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert_eq!((b.x0, b.y0), (10.0, 10.0));
    assert_eq!(guides(&s), vec![(true, 200.0), (false, 300.0)]);

    // Redo reapplies both.
    s.execute("edit.redo", &json!({})).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert_eq!((b.x0, b.y0), (35.0, 45.0));
    assert_eq!(guides(&s), vec![(true, 225.0), (false, 335.0)]);

    // Undo back to start.
    s.execute("edit.undo", &json!({})).unwrap();

    // 2. object.move also moves both.
    s.execute("object.move", &json!({"dx": 15, "dy": 20})).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert_eq!((b.x0, b.y0), (25.0, 30.0));
    assert_eq!(guides(&s), vec![(true, 215.0), (false, 320.0)]);
    s.execute("edit.undo", &json!({})).unwrap();

    // 3. Arrow-key nudge moves both.
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0})).unwrap();
    let k = s.prefs.keyboard_increment;
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert_eq!(b.x0, 10.0 + k);
    assert_eq!(guides(&s)[0], (true, 200.0 + k));
    assert_eq!(guides(&s)[1], (false, 300.0));
    s.execute("edit.undo", &json!({})).unwrap();

    // 4. Copy (e.g. Alt-drag) duplicates both art and guides, leaving originals in place.
    let res = s.execute("object.transform", &json!({"matrix": [1, 0, 0, 1, 50, 60], "copy": true})).unwrap();
    let copied_id = NodeId(res["ids"][0].as_u64().unwrap());
    assert_ne!(copied_id, a);
    let b_orig = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    let b_copy = s.doc().unwrap().doc.bounds_of(&[copied_id], false).unwrap();
    assert_eq!((b_orig.x0, b_orig.y0), (10.0, 10.0));
    assert_eq!((b_copy.x0, b_copy.y0), (60.0, 70.0));
    assert_eq!(guides(&s), vec![(true, 200.0), (false, 300.0), (true, 250.0), (false, 360.0)]);
    assert_eq!((selected(&s), selected_guides(&s)), (vec![copied_id], vec![2, 3]));

    // Undo takes back both copies in one step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s), vec![(true, 200.0), (false, 300.0)]);
    assert!(s.doc().unwrap().doc.node(copied_id).is_none());
    assert!(s.doc().unwrap().doc.node(a).is_some());

    guides_and_art_later_cases(&mut s, a);
}

/// Cases 5 to 8 of [`guides_and_art_move_together_when_selected`]: explicit ids, preview copies and moving a guide.
fn guides_and_art_later_cases(s: &mut Session, a: NodeId) {
    // 5. Explicit ids: only the specified art moves, guides stay put.
    sel(s, &[a]);
    s.execute("guide.select", &json!({"indexes": [0, 1], "toggle": true})).unwrap();
    s.execute("object.transform", &json!({"ids": [a.0], "matrix": [1, 0, 0, 1, 40, 50]})).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert_eq!((b.x0, b.y0), (50.0, 60.0));
    assert_eq!(guides(s), vec![(true, 200.0), (false, 300.0)]);
    s.execute("edit.undo", &json!({})).unwrap();

    // 6. Interactive preview copies: dragging with Alt previews the copies and commit keeps them.
    s.begin_interaction("Move").unwrap();
    s.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, 10, 20], "copy": true})).unwrap();
    s.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, 30, 40], "copy": true})).unwrap();
    s.commit_interaction().unwrap();
    assert_eq!(guides(s).len(), 4);
    assert_eq!(guides(s)[2], (true, 230.0));
    assert_eq!(guides(s)[3], (false, 340.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(s), vec![(true, 200.0), (false, 300.0)]);

    // 7. Dragging a guide (guide.move) when art is also selected moves both art and guides.
    sel(s, &[a]);
    s.execute("guide.select", &json!({"indexes": [0, 1], "toggle": true})).unwrap();
    s.execute("guide.move", &json!({"dx": 10, "dy": 20})).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert_eq!((b.x0, b.y0), (20.0, 30.0));
    assert_eq!(guides(s), vec![(true, 210.0), (false, 320.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert_eq!((b.x0, b.y0), (10.0, 10.0));
    assert_eq!(guides(s), vec![(true, 200.0), (false, 300.0)]);

    // 8. Copy-dragging a guide (guide.move { copy: true }) when art is also selected duplicates both.
    s.execute("guide.move", &json!({"dx": 10, "dy": 20, "copy": true})).unwrap();
    assert_eq!(guides(s).len(), 4);
    assert_eq!(guides(s)[2], (true, 210.0));
    assert_eq!(guides(s)[3], (false, 320.0));
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(s).len(), 2);
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 1);
}

/// #451: an artboard guide (`guide.add {artboard}`) runs across its artboard only, and is moved,
/// copied and deleted with it; canvas guides stay put.
#[test]
fn artboard_guides_go_with_their_artboard() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 900, "y": 0, "width": 400, "height": 300})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 1000, "artboard": 1})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 100})).unwrap();
    let list = |s: &mut Session| s.execute("guide.list", &json!({})).unwrap();
    let layer = s.doc().unwrap().current_layer().unwrap().0;
    assert_eq!(
        list(&mut s),
        json!([
            {"index": 0, "vertical": true, "pos": 1000.0, "selected": false, "artboard": 1, "layer": layer, "shown": true, "editable": true},
            {"index": 1, "vertical": false, "pos": 100.0, "selected": false, "layer": layer, "shown": true, "editable": true},
        ])
    );
    let spans = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        d.guides.iter().map(|g| d.guide_span(g)).collect::<Vec<_>>()
    };
    assert_eq!(spans(&s), [Some((0.0, 300.0)), None]);
    for bad in [json!(5), json!("x"), json!(-1)] {
        assert!(s.execute("guide.add", &json!({"vertical": true, "pos": 10, "artboard": bad})).is_err(), "{bad}");
    }
    // Moved, the artboard takes its guides along; in Artboard Options too, unless resized.
    s.execute("artboard.move", &json!({"index": 1, "dx": 50, "dy": 20})).unwrap();
    assert_eq!(guides(&s), [(true, 1050.0), (false, 100.0)]);
    s.execute("artboard.setProps", &json!({"index": 1, "x": 960.5})).unwrap();
    assert_eq!(guides(&s)[0], (true, 1060.5));
    s.execute("artboard.setProps", &json!({"index": 1, "x": 900, "width": 500})).unwrap();
    assert_eq!((guides(&s)[0], spans(&s)[0]), ((true, 1060.5), Some((20.0, 320.0))));
    // Renumbered, it keeps them.
    s.execute("artboard.reorder", &json!({"index": 1, "to": 0})).unwrap();
    assert_eq!(list(&mut s)[0]["artboard"], 0);
    // Duplicated (or Alt-dragged), the copy gets copies of them.
    let dup = s.execute("artboard.duplicate", &json!({"index": 0})).unwrap()["index"].as_u64().unwrap();
    let dx = s.doc().unwrap().doc.artboards[dup as usize].rect.x0 - 900.0;
    assert_eq!(guides(&s), [(true, 1060.5), (false, 100.0), (true, 1060.5 + dx)]);
    assert_eq!(list(&mut s)[2]["artboard"], dup);
    s.execute("artboard.move", &json!({"index": 0, "dx": 0, "dy": 400, "copy": true})).unwrap();
    assert_eq!((guides(&s).len(), list(&mut s)[3]["artboard"].clone()), (4, json!(3)));
    // Rearranged, they follow their artboards.
    s.execute("artboard.rearrange", &json!({"columns": 4})).unwrap();
    let d = &s.doc().unwrap().doc;
    for (g, ab) in d.guides.iter().filter_map(|g| Some((g, d.artboards.iter().find(|a| Some(a.id) == g.artboard)?))) {
        assert_eq!(g.pos - ab.rect.x0, 160.5, "{g:?} on {ab:?}");
    }
    // Deleted, the artboard takes its guides; the selected ones left stay selected.
    s.execute("guide.select", &json!({"indexes": [1, 2]})).unwrap();
    let n = undo_len(&s);
    s.execute("artboard.delete", &json!({"index": 0})).unwrap();
    assert_eq!((guides(&s).len(), selected_guides(&s)), (3, vec![0, 1]));
    assert_eq!(
        list(&mut s)[0],
        json!({"index": 0, "vertical": false, "pos": 100.0, "selected": true, "layer": layer, "shown": true, "editable": true})
    );
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((guides(&s).len(), selected_guides(&s)), (4, vec![1, 2]));
    // Cut, too.
    s.execute("artboard.cut", &json!({"index": 0})).unwrap();
    assert_eq!((guides(&s).len(), selected_guides(&s)), (3, vec![0, 1]));
}

/// #451: a guide dragged out of a ruler snaps (to the art's corners, side midpoints and anchors,
/// the artboards' edges and centres) and is made in one undo step where it is released over the
/// canvas; with the Artboard tool it is an artboard guide of the active artboard.
#[test]
fn guides_dragged_out_of_a_ruler_snap_and_take_the_artboard_tool_s_artboard() {
    use vectorcraft_tools::{Overlay, PointerEvent, PointerKind};
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let v = ViewInfo::default();
    let ev = |kind, x, y| PointerEvent::new(kind, x, y);
    let n = undo_len(&s);
    s.ruler_guide(true, &ev(PointerKind::Drag, -10.0, 30.0), false, v).unwrap();
    assert!(guides(&s).is_empty(), "still over the ruler");
    s.ruler_guide(true, &ev(PointerKind::Drag, 153.0, 30.0), true, v).unwrap();
    assert_eq!(guides(&s), [(true, 150.0)], "onto the line through the top and bottom midpoints");
    assert!(s.overlays(v).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "center")));
    s.ruler_guide(true, &ev(PointerKind::Drag, 797.0, 30.0), true, v).unwrap();
    assert_eq!(guides(&s), [(true, 800.0)], "the artboard's edge");
    s.ruler_guide(true, &ev(PointerKind::Up, 797.0, 30.0), true, v).unwrap();
    assert_eq!(guides(&s), [(true, 800.0)]);
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "New Guide");
    assert!(s.overlays(v).is_empty());
    // Released off the canvas: none.
    s.ruler_guide(false, &ev(PointerKind::Drag, 30.0, 250.0), true, v).unwrap();
    s.ruler_guide(false, &ev(PointerKind::Up, 30.0, -10.0), false, v).unwrap();
    assert_eq!((guides(&s).len(), undo_len(&s)), (1, n + 1));
    // The Artboard tool: an artboard guide of its active artboard.
    s.execute("artboard.new", &json!({"x": 900, "y": 0, "width": 400, "height": 300})).unwrap();
    s.select_tool("artboard", v).unwrap();
    s.set_tool_option("active", &json!(1));
    s.ruler_guide(false, &ev(PointerKind::Drag, 1000.0, 50.0), true, v).unwrap();
    s.ruler_guide(false, &ev(PointerKind::Up, 1000.0, 50.0), true, v).unwrap();
    let layer = s.doc().unwrap().current_layer().unwrap().0;
    assert_eq!(
        s.execute("guide.list", &json!({})).unwrap()[1],
        json!({"index": 1, "vertical": false, "pos": 50.0, "selected": false, "artboard": 1, "layer": layer, "shown": true, "editable": true})
    );
}

/// Guides saved before guides had layers are put on a layer when the document opens.
#[test]
fn guides_from_older_files_are_put_on_a_layer() {
    let mut s = Session::new();
    let mut d = vectorcraft_doc::Document::new(400.0, 300.0);
    d.guides.push(vectorcraft_doc::Guide::new(true, 50.0));
    d.guides.push(vectorcraft_doc::Guide::new(false, 80.0));
    s.add_document(d, None);
    let st = s.doc().unwrap();
    let top = st.doc.default_layer();
    assert!(top.is_some() && st.doc.guides.iter().all(|g| g.layer == top), "{:?}", st.doc.guides);
}

/// Ruler guides are layer objects: made on the current layer, hidden, locked and deleted with it,
/// and moved to another layer.
#[test]
fn ruler_guides_live_on_layers() {
    let mut s = session();
    let first = s.doc().unwrap().current_layer().unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[0].layer, Some(first), "on the current layer");
    let second = id_of(&s.execute("layer.new", &json!({"name": "Guides"})).unwrap());
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[1].layer, Some(second), "the new layer is current");
    assert!(s.execute("guide.add", &json!({"vertical": true, "pos": 1, "layer": 99999})).is_err());
    // Hidden or locked with its layer.
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": false})).unwrap();
    assert_eq!(s.execute("guide.list", &json!({})).unwrap()[1]["shown"], false);
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": true, "locked": true})).unwrap();
    let row = s.execute("guide.list", &json!({})).unwrap()[1].clone();
    assert_eq!((row["shown"].clone(), row["editable"].clone()), (json!(true), json!(false)));
    s.execute("layer.setProps", &json!({"ids": [second.0], "locked": false})).unwrap();
    // Moved to the first layer, then back.
    s.execute("guide.setLayer", &json!({"index": 1, "layer": first.0})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[1].layer, Some(first));
    s.execute("guide.setLayer", &json!({"index": 1, "layer": second.0})).unwrap();
    assert!(s.execute("guide.setLayer", &json!({"index": 1, "layer": 99999})).is_err());
    // Several at once, as dragging their rows onto a layer's row does.
    s.execute("guide.setLayer", &json!({"indexes": [0, 1], "layer": first.0})).unwrap();
    assert!(s.doc().unwrap().doc.guides.iter().all(|g| g.layer == Some(first)));
    s.execute("guide.setLayer", &json!({"indexes": [1], "layer": second.0})).unwrap();
    assert!(s.execute("guide.setLayer", &json!({"indexes": [7], "layer": first.0})).is_err());
    // Deleting a layer deletes its guides; undo brings them back.
    s.execute("layer.delete", &json!({"ids": [second.0]})).unwrap();
    assert_eq!(guides(&s), [(true, 200.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s).len(), 2);
}

/// Guides on a hidden or locked layer leave the selection and can't be moved, deleted or moved to
/// another layer; new guides skip a locked current layer as new art does; no guide goes onto a
/// locked layer.
#[test]
fn guides_on_hidden_or_locked_layers_are_left_alone() {
    let mut s = session();
    let first = s.doc().unwrap().current_layer().unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    let second = id_of(&s.execute("layer.new", &json!({})).unwrap());
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0, 1]})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [first.0], "locked": true})).unwrap();
    assert_eq!(selected_guides(&s), [1], "the locked layer's guide is deselected");
    assert_eq!(s.execute("guide.select", &json!({"indexes": [0]})).unwrap()["selected"], json!([]), "and can't be selected");
    assert!(s.execute("guide.move", &json!({"index": 0, "pos": 50})).is_err());
    assert!(s.execute("guide.remove", &json!({"index": 0})).is_err());
    assert!(s.execute("guide.setLayer", &json!({"index": 0, "layer": second.0})).is_err());
    assert!(s.execute("guide.setLayer", &json!({"index": 1, "layer": first.0})).is_err(), "not onto a locked layer");
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": false})).unwrap();
    assert!(selected_guides(&s).is_empty(), "the hidden layer's guide is deselected");
    // Both layers hidden or locked: a new guide is refused, as new art is.
    assert!(s.execute("guide.add", &json!({"vertical": true, "pos": 10})).is_err());
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": true, "locked": true})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [first.0], "locked": false})).unwrap();
    // The current layer locked: the new guide goes on the top shown, unlocked layer.
    s.execute("guide.add", &json!({"vertical": true, "pos": 20})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[2].layer, Some(first));
    assert_eq!(guides(&s).len(), 3);
}

/// Guides follow their layers through Duplicate, Merge Selected and Flatten Artwork, in one undo
/// step each.
#[test]
fn guides_go_with_their_layers_when_duplicated_merged_and_flattened() {
    let mut s = session();
    let first = s.doc().unwrap().current_layer().unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    let second = id_of(&s.execute("layer.new", &json!({})).unwrap());
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    let on = |s: &Session, l: NodeId| s.doc().unwrap().doc.guides_on(l).map(|(_, g)| (g.vertical, g.pos)).collect::<Vec<_>>();
    // Duplicate Layer copies its guides onto the copy.
    let copy = s.execute("layer.duplicate", &json!({"ids": [second.0]})).unwrap()["ids"][0].as_u64().map(NodeId).unwrap();
    assert_eq!((on(&s, second), on(&s, copy)), (vec![(false, 300.0)], vec![(false, 300.0)]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s).len(), 2);
    // Merged into the first layer, the second layer's guide goes with its art.
    s.execute("layer.merge", &json!({"ids": [second.0, first.0]})).unwrap();
    assert_eq!(on(&s, first), [(true, 200.0), (false, 300.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((on(&s, first), on(&s, second)), (vec![(true, 200.0)], vec![(false, 300.0)]));
    // Flattened into the first layer, a hidden layer is discarded with its guides.
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": false})).unwrap();
    s.execute("layer.flatten", &json!({"id": first.0})).unwrap();
    assert_eq!(guides(&s), [(true, 200.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s).len(), 2);
}

/// #451: art moved with the Selection tool lands flush on the artboard's edges and centre (Smart
/// Guides).
#[test]
fn moved_art_snaps_to_the_artboard_edges_and_centre() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let mut drag = |from: (f64, f64), to: (f64, f64)| {
        for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
        }
        let b = bounds(&s, a);
        (b.x0, b.y0)
    };
    assert_eq!(drag((150.0, 150.0), (53.0, 147.0)), (0.0, 97.0), "3 pt off the left edge: flush with it");
    assert_eq!(drag((50.0, 147.0), (402.0, 298.0)), (350.0, 250.0), "its centre on the artboard's");
}

/// #414: the Selection tool picks a ruler guide over the art and drags it in one undo step; with
/// guides hidden or locked the press goes to the art.
#[test]
fn selection_tool_drags_a_ruler_guide() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 150})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("selection", v).unwrap();
    let drag = |s: &mut Session, v: ViewInfo, from: (f64, f64), to: (f64, f64)| {
        s.pointer(&PointerEvent::new(PointerKind::Down, from.0, from.1), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, to.0, to.1), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1), v).unwrap();
    };
    let n = undo_len(&s);
    drag(&mut s, v, (151.0, 120.0), (171.0, 140.0));
    assert_eq!(guides(&s), [(true, 170.0)]);
    assert_eq!(selected_guides(&s), [0]);
    assert!(selected(&s).is_empty());
    assert_eq!(bounds(&s, a).x0, 100.0, "the art stays");
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Move Guide");
    // Hidden guides: the press takes the art.
    let hidden = ViewInfo { guides: false, ..v };
    drag(&mut s, hidden, (170.0, 120.0), (180.0, 120.0));
    assert_eq!((guides(&s), bounds(&s, a).x0), (vec![(true, 170.0)], 110.0));
    // Locked guides too.
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    drag(&mut s, v, (170.0, 120.0), (180.0, 120.0));
    assert_eq!((guides(&s), bounds(&s, a).x0), (vec![(true, 170.0)], 120.0));
}

// ---------- File ----------

#[test]
fn close_all_documents() {
    let mut s = session();
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(s.execute("file.closeAll", &json!({})).unwrap()["closed"], 2);
    assert!(s.active().is_none());
    assert!(s.execute("file.closeAll", &json!({})).is_err());
}

/// Close and Activate refuse a document index of 2^32, and both documents stay open with the second
/// one active. A 32-bit build read the index as 0.
#[test]
fn close_and_activate_refuse_a_document_index_past_u32() {
    let mut s = session();
    s.execute("file.new", &json!({})).unwrap();
    for cmd in ["document.activate", "file.close"] {
        assert!(s.execute(cmd, &json!({"index": 1u64 << 32})).is_err(), "{cmd}");
        assert_eq!((s.documents().len(), s.active_index()), (2, Some(1)), "{cmd}");
    }
}

#[test]
fn document_color_mode_converts_colours() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    fill(&mut s, a, "#ff0000");
    s.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.color_mode, ColorMode::Cmyk);
    // Converted through the colour settings: a press red, mostly magenta and yellow.
    let Some(Color::Cmyk { c, m, y, k }) = node(&s, a).appearance.fill_paint().color() else { panic!("not CMYK") };
    assert!(m > 0.8 && y > 0.8 && c < 0.05 && k < 0.05, "{c} {m} {y} {k}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.color_mode, ColorMode::Rgb);
    assert!(s.execute("file.documentColorMode", &json!({"mode": "lab"})).is_err());
}

/// Four RGB greys (#000000, #333333, #808080, #e6e6e6, as in issue #421) and a red, a 40% tint of a
/// grey global swatch, a gradient to grey and a pattern of a grey tile → (session, the five
/// rectangles, the tinted one, the gradient one).
fn greys_document() -> (Session, Vec<NodeId>, NodeId, NodeId) {
    let mut s = session();
    let ids: Vec<NodeId> = ["#000000", "#333333", "#808080", "#e6e6e6", "#ff0000"]
        .iter()
        .map(|hex| {
            let id = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
            fill(&mut s, id, hex);
            id
        })
        .collect();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#808080", "global": true})).unwrap();
    let tint = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [tint.0], "swatch": "Ink", "tint": 40})).unwrap();
    let grad = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#333333"}]);
    s.execute("paint.setFill", &json!({"ids": [grad.0], "gradient": {"kind": "linear", "stops": stops}})).unwrap();
    let tile = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    fill(&mut s, tile, "#808080");
    s.execute("object.pattern.make", &json!({"ids": [tile.0], "name": "Grey Tile", "edit": false})).unwrap();
    (s, ids, tint, grad)
}

/// The colour of pattern `name`'s first tile object.
fn pattern_fill(s: &Session, name: &str) -> Color {
    s.doc().unwrap().doc.pattern(name).unwrap().art[0].appearance.fill_paint().color().unwrap()
}

/// K of a colour that prints on the black plate only.
fn k_only(c: Color) -> f32 {
    match c {
        Color::Cmyk { c: 0.0, m: 0.0, y: 0.0, k } => k,
        other => panic!("not K only: {other:?}"),
    }
}

#[test]
fn document_color_mode_separates_rgb_greys_through_the_profile() {
    let (mut s, ids, _, _) = greys_document();
    s.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    // As Illustrator converts them: four-colour greys and a rich black.
    let Color::Cmyk { c, m, y, k } = fill_of(&s, ids[2]) else { panic!("not CMYK") };
    assert!(c > 0.05 && m > 0.05 && y > 0.05 && k > 0.0, "a four-colour mid grey: {c} {m} {y} {k}");
    let Color::Cmyk { c, k, .. } = fill_of(&s, ids[0]) else { panic!("not CMYK") };
    assert!(k > 0.9 && c > 0.3, "rich black: {c} {k}");
    // Pattern tiles are swatches too: their art converts with the rest.
    assert!(matches!(pattern_fill(&s, "Grey Tile"), Color::Cmyk { .. }), "{:?}", pattern_fill(&s, "Grey Tile"));
    // Gray colours stay Gray.
    let mut g = session();
    let a = rect(&mut g, 0.0, 0.0, 10.0, 10.0);
    g.execute("paint.setFill", &json!({"ids": [a.0], "color": {"gray": 0.3}})).unwrap();
    g.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    assert_eq!(fill_of(&g, a), Color::gray(0.3));
}

#[test]
fn document_color_mode_can_put_rgb_greys_on_the_black_plate() {
    let (mut profile, pids, ..) = greys_document();
    profile.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let (mut s, ids, tint, grad) = greys_document();
    let e = s.execute("file.documentColorMode", &json!({"mode": "cmyk", "grays": "rich"})).unwrap_err().to_string();
    assert!(e.contains("grays"), "{e}");
    s.execute("file.documentColorMode", &json!({"mode": "cmyk", "grays": "black"})).unwrap();
    let close = |a: f32, b: f32| (a - b).abs() < 0.005;
    for (id, want) in ids.iter().zip([1.0, 0.8, 0.498, 0.098]) {
        let k = k_only(fill_of(&s, *id));
        assert!(close(k, want), "K {k}, want {want}");
    }
    // Other colours convert through the profile as before.
    assert_eq!(fill_of(&s, ids[4]), fill_of(&profile, pids[4]));
    // Swatches, their tints, gradient stops and pattern tiles follow the same rule.
    let d = &s.doc().unwrap().doc;
    assert!(close(k_only(d.swatch("Ink").unwrap().paint.color().unwrap()), 0.498));
    let Paint::Solid { color, swatch, tint: t } = node(&s, tint).appearance.fill_paint() else { panic!("not solid") };
    assert!(close(k_only(color), 0.498 * 0.4) && swatch.as_deref() == Some("Ink") && t == 0.4, "{color:?} {swatch:?} {t}");
    let Paint::Gradient(g) = node(&s, grad).appearance.fill_paint() else { panic!("not a gradient") };
    let ks: Vec<f32> = g.gradient.stops.iter().map(|st| k_only(st.color)).collect();
    assert!(close(ks[0], 0.0) && close(ks[1], 0.8), "{ks:?}");
    assert!(close(k_only(pattern_fill(&s, "Grey Tile")), 0.498));
    // To RGB, `grays` changes nothing.
    s.execute("file.documentColorMode", &json!({"mode": "rgb", "grays": "black"})).unwrap();
    let rgb = s.doc().unwrap().doc.layers.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("file.documentColorMode", &json!({"mode": "rgb"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers, rgb);
}

#[test]
fn opening_in_cmyk_can_put_rgb_greys_on_the_black_plate() {
    let mut s = session();
    for hex in ["#000000", "#333333", "#808080", "#e6e6e6", "#ff0000"] {
        let id = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
        fill(&mut s, id, hex);
    }
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    let data = vectorcraft_format::base64_encode(svg.as_bytes());
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "greys.svg", "dataBase64": data, "colorMode": "cmyk", "grays": "black"})).unwrap();
    let d = &o.doc().unwrap().doc;
    assert_eq!(d.color_mode, ColorMode::Cmyk);
    let mut fills = vec![];
    for l in &d.layers {
        l.walk(&mut |n| fills.extend(n.children().is_none().then(|| n.appearance.fill_paint().color()).flatten()));
    }
    assert_eq!(fills.len(), 5, "{fills:?}");
    for (c, want) in fills.iter().zip([1.0, 0.8, 0.498, 0.098]) {
        let k = k_only(*c);
        assert!((k - want).abs() < 0.005, "K {k}, want {want}");
    }
    assert!(matches!(fills[4], Color::Cmyk { m, y, .. } if m > 0.8 && y > 0.8), "red separates as usual: {:?}", fills[4]);
    let e = Session::new().execute("document.open", &json!({"name": "greys.svg", "dataBase64": data, "grays": "k"})).unwrap_err().to_string();
    assert!(e.contains("grays"), "{e}");
}

#[test]
fn file_info_sets_title() {
    let mut s = session();
    let r = s.execute("file.info", &json!({"title": "Poster"})).unwrap();
    assert_eq!(r["title"], "Poster");
    assert_eq!(s.doc().unwrap().doc.title, "Poster");
    assert_eq!(s.execute("file.info", &json!({})).unwrap()["colorMode"], "rgb");
}

#[test]
fn every_new_command_has_params_doc_and_menu() {
    for id in [
        "object.lock.above",
        "object.transformEach",
        "object.rasterize",
        "object.blend.make",
        "edit.colors.invert",
        "edit.findReplace",
        "select.same.fontSize",
        "type.changeCase",
        "view.guides.make",
        "file.closeAll",
    ] {
        let c = find_command(id).unwrap_or_else(|| panic!("{id}"));
        assert!(!c.params.is_empty() && !c.menu.is_empty(), "{id}");
    }
}

#[test]
fn saved_selection_names_are_trimmed_and_capped() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    let long = "n".repeat(1000);
    let name = s.execute("select.save", &json!({ "name": format!(" {long} ") })).unwrap()["name"].clone();
    assert_eq!(name.as_str().map(|n| n.chars().count()), Some(vectorcraft_doc::SavedSelection::MAX_NAME));
    s.execute("select.editSaved", &json!({"name": name, "newName": format!("{long}é")})).unwrap();
    let list = s.execute("select.savedList", &json!({})).unwrap();
    assert_eq!(list[0].as_str().map(|n| n.chars().count()), Some(vectorcraft_doc::SavedSelection::MAX_NAME));
}

#[test]
fn crop_image_trim_cuts_to_the_opaque_pixels_as_they_are() {
    // 100 × 80 px, transparent but for a block at (20, 10)–(50, 40) px; placed at 2 pt a pixel.
    let mut px = image::RgbaImage::new(100, 80);
    for y in 10..40 {
        for x in 20..50 {
            px.put_pixel(x, y, image::Rgba([x as u8, y as u8, 7, if x == 20 { 1 } else { 255 }]));
        }
    }
    let mut png = Vec::new();
    px.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let img = selected(&s)[0];
    s.execute("object.scale", &json!({"sx": 200, "origin": [0, 0]})).unwrap();
    let r = s.execute("object.cropImage", &json!({"trim": true})).unwrap();
    assert_eq!((r["width"].as_u64().unwrap(), r["height"].as_u64().unwrap()), (30, 30), "{r}");
    // Placed where those pixels were: (40, 20) pt, 60 × 60 pt at 2 pt a pixel.
    let b = bounds(&s, img);
    assert!(close(b.x0, 40.0) && close(b.y0, 20.0) && close(b.width(), 60.0) && close(b.height(), 60.0), "{b:?}");
    // Pixel for pixel, a barely visible (alpha 1) edge included.
    let NodeKind::Image(im) = &node(&s, img).kind else { panic!("an image") };
    let out = image::load_from_memory(&s.doc().unwrap().doc.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(out, image::imageops::crop_imm(&px, 20, 10, 30, 30).to_image());
    assert_eq!(r["trimmed"], true);
    // Nothing transparent left to cut: left as it is, and not an error (scripts trim every image).
    let r = s.execute("object.cropImage", &json!({"trim": true})).unwrap();
    assert_eq!((r["trimmed"].as_bool(), r["width"].as_u64()), (Some(false), Some(30)), "{r}");
    assert_eq!(bounds(&s, img), b);
}

#[test]
fn crop_image_trim_checks_the_rgba_size_before_decoding() {
    // 1-bit gray PNGs, header only: their samples are within the decoder's limit, and as RGBA
    // (4 bytes a pixel) they are just over or just within the 2 GB allowance.
    let trim = |w: u32, h: u32| {
        let png = crate::tests_linkspanel::png_header(w, h);
        let mut s = Session::new();
        s.execute("document.open", &json!({"name": "wall.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        s.execute("object.cropImage", &json!({"trim": true})).unwrap_err().to_string()
    };
    let over = trim(23_171, 23_170);
    assert!(over.contains("an image of 23171 × 23170 pixels is too large to draw"), "{over}");
    // One column fewer is within the allowance: it is decoded, and its missing rows can't be read.
    let within = trim(23_170, 23_170);
    assert!(within.contains("the image's pixels can't be read"), "{within}");
}

#[test]
fn crop_image_trim_leaves_a_cmyk_image_alone() {
    // CMYK images keep their ink amounts and have no alpha: nothing to cut, and not an error.
    let inks = vectorcraft_doc::cmyk::Inks::new(4, 2, [255, 0, 0, 0].repeat(8)).unwrap();
    let tiff = vectorcraft_doc::ImageBlob::cmyk_tiff(&inks).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "inks.tif", "dataBase64": vectorcraft_format::base64_encode(&tiff.bytes)})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let r = s.execute("object.cropImage", &json!({"trim": true})).unwrap();
    assert_eq!((r["trimmed"].as_bool(), r["width"].as_u64(), r["height"].as_u64()), (Some(false), Some(4), Some(2)), "{r}");
}
