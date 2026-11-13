//! Pattern swatches, pattern editing mode and Repeat, driven through `Session::execute`.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::pattern::{PATTERN_EDIT_LAYER, RepeatKind, TileType, pattern_paint};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::tests_menucmds::place_undecodable_png;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

/// A red 10×10 square made into pattern "Dots" (editing finished).
fn make_dots(s: &mut Session) -> NodeId {
    let r = rect(s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [r.0], "none": true})).unwrap();
    sel(s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    r
}

fn close(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

// ---------- patterns ----------

#[test]
fn make_creates_definition_swatch_and_edit_mode() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.pattern.make", &json!({})).unwrap();
    assert_eq!(v["name"], "New Pattern 1");
    let d = doc(&s);
    let def = d.pattern("New Pattern 1").unwrap();
    assert_eq!(def.tile, Rect::new(0.0, 0.0, 10.0, 10.0));
    assert!(d.swatch("New Pattern 1").is_some_and(|sw| matches!(sw.paint, Paint::Pattern { .. })));
    let pe = d.pattern_edit.as_ref().unwrap();
    assert_eq!(d.node(pe.layer).unwrap().name.as_deref(), Some(PATTERN_EDIT_LAYER));
    assert_eq!(d.node(pe.layer).unwrap().children().unwrap().len(), 1);
    // The original art stays in the document; new art goes into the edit layer.
    assert!(d.node(r).is_some());
    assert_eq!(s.doc().unwrap().insertion_parent(), Some(pe.layer));
}

#[test]
fn done_saves_edited_art_and_leaves_mode() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    // Draw another shape inside the pattern.
    rect(&mut s, 2.0, 2.0, 3.0, 3.0);
    s.execute("object.pattern.done", &json!({})).unwrap();
    let d = doc(&s);
    assert!(d.pattern_edit.is_none());
    assert_eq!(d.pattern("P").unwrap().art.len(), 2);
    assert!(d.layers.iter().all(|l| l.name.as_deref() != Some(PATTERN_EDIT_LAYER)));
    assert!(s.doc().unwrap().isolation.is_none());
}

/// Exit Isolation Mode while a pattern is edited (the canvas context menu and the Layers panel
/// menu offer it) leaves pattern editing as Done does, with the drawn art saved into the pattern.
#[test]
fn exit_isolation_while_editing_a_pattern_saves_it_and_leaves_the_mode() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    rect(&mut s, 2.0, 2.0, 3.0, 3.0);
    s.execute("object.exitIsolation", &json!({})).unwrap();
    let d = doc(&s);
    assert!(d.pattern_edit.is_none());
    assert_eq!(d.pattern("P").unwrap().art.len(), 2);
    assert!(d.layers.iter().all(|l| l.name.as_deref() != Some(PATTERN_EDIT_LAYER)));
    let st = s.doc().unwrap();
    assert_eq!((st.isolation, st.active_layer), (None, st.doc.default_layer()));
    assert_eq!(st.history.undo.last().unwrap().label, "Done Editing Pattern");
}

/// Point type that a Type tool click placed in the tile and left empty is discarded when Done or
/// Exit Isolation Mode leaves pattern editing, so the pattern does not keep it.
#[test]
fn empty_point_type_is_not_saved_into_the_pattern() {
    for cmd in ["object.pattern.done", "object.exitIsolation"] {
        let mut s = session();
        s.prefs.placeholder_text = false;
        let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
        sel(&mut s, &[r]);
        s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
        let v = ViewInfo::default();
        s.select_tool("type", v).unwrap();
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, 200.0, 200.0), v).unwrap();
        }
        assert!(s.tool_wants_text(), "{cmd}: the click places point type to edit");
        s.execute(cmd, &json!({})).unwrap();
        assert!(!s.tool_wants_text(), "{cmd}");
        let art = &doc(&s).pattern("P").unwrap().art;
        assert_eq!(art.len(), 1, "{cmd}: the tile keeps only the square");
        assert!(!matches!(art[0].kind, NodeKind::Text(_)), "{cmd}");
        let labels = s.doc().unwrap().history.undo.iter().map(|e| e.label.as_str()).collect::<Vec<_>>();
        assert_eq!(labels.last(), Some(&"Done Editing Pattern"), "{cmd}");
        assert!(!labels.contains(&"Type"), "{cmd}: {labels:?}");
    }
}

/// The same while a Cmd/Ctrl press has lent the Selection tool to the Type tool: Done or Exit
/// Isolation Mode before the release gives the Type tool back first, so the empty point type is
/// still discarded, and the release that follows leaves the tile as it is.
#[test]
fn empty_point_type_is_not_saved_into_the_pattern_during_a_cmd_press() {
    for cmd in ["object.pattern.done", "object.exitIsolation"] {
        let mut s = session();
        s.prefs.placeholder_text = false;
        let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
        sel(&mut s, &[r]);
        s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
        let v = ViewInfo::default();
        s.select_tool("type", v).unwrap();
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, 200.0, 200.0), v).unwrap();
        }
        let press = Mods { cmd: true, ..Mods::default() };
        s.pointer(&PointerEvent::new(PointerKind::Down, 300.0, 300.0).with_mods(press), v).unwrap();
        assert_eq!(s.tool_id(), "selection", "{cmd}: the press lends the Selection tool");
        s.execute(cmd, &json!({})).unwrap();
        assert_eq!(s.tool_id(), "type", "{cmd}: the Type tool is back");
        assert!(!s.tool_wants_text(), "{cmd}");
        s.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 300.0).with_mods(press), v).unwrap();
        let art = &doc(&s).pattern("P").unwrap().art;
        assert_eq!(art.len(), 1, "{cmd}: the tile keeps only the square");
        assert!(!matches!(art[0].kind, NodeKind::Text(_)), "{cmd}");
    }
}

/// Done during a drag of the lent Selection tool keeps the drag as its own step before the tile is
/// saved, so no drag is left open for the release to end. With Real-time Drawing and Editing off,
/// the drag's last position is applied first, as the release would apply it.
#[test]
fn done_during_a_lent_selection_drag_keeps_the_drag_as_its_own_step() {
    for real_time in [true, false] {
        let mut s = session();
        s.prefs.placeholder_text = false;
        s.execute("prefs.set", &json!({"key": "realTimeDrawing", "value": real_time})).unwrap();
        let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
        sel(&mut s, &[r]);
        s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
        let st = s.doc().unwrap();
        let b = st.doc.bounds_of(&st.selection.objects, false).expect("the tile art is selected");
        let c = b.center();
        let v = ViewInfo::default();
        s.select_tool("type", v).unwrap();
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, 200.0, 200.0), v).unwrap();
        }
        let press = Mods { cmd: true, ..Mods::default() };
        s.pointer(&PointerEvent::new(PointerKind::Down, c.x, c.y).with_mods(press), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, c.x + 20.0, c.y).with_mods(press), v).unwrap();
        assert!(s.in_interaction(), "{real_time}: the lent Selection tool drags the square");
        s.execute("object.pattern.done", &json!({})).unwrap();
        assert!(!s.in_interaction(), "{real_time}: the drag is kept before the tile is saved");
        assert_eq!(s.tool_id(), "type", "{real_time}");
        s.pointer(&PointerEvent::new(PointerKind::Up, c.x + 20.0, c.y).with_mods(press), v).unwrap();
        let labels = s.doc().unwrap().history.undo.iter().map(|e| e.label.as_str()).collect::<Vec<_>>();
        assert_eq!(labels.last(), Some(&"Done Editing Pattern"), "{real_time}: {labels:?}");
        assert!(labels.contains(&"Move"), "{real_time}: {labels:?}");
        let art = &doc(&s).pattern("P").unwrap().art;
        assert_eq!(art.len(), 1, "{real_time}: the tile keeps only the square");
        assert!(!matches!(art[0].kind, NodeKind::Text(_)), "{real_time}");
    }
}

#[test]
fn cancel_new_pattern_removes_it() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    assert!(doc(&s).pattern("P").is_none());
    assert!(doc(&s).swatch("P").is_none());
    assert_eq!(doc(&s).layers.len(), 1);
}

fn doc_mut(s: &mut Session) -> &mut vectorcraft_doc::Document {
    Arc::make_mut(&mut s.doc_mut().unwrap().doc)
}

fn is_pattern(p: &Paint, name: &str) -> bool {
    matches!(p, Paint::Pattern { pattern, .. } if pattern == name)
}

/// How many paints of pattern `name` the document holds: in the art and the definitions
/// ([`vectorcraft_doc::Document::visit_paints`]), the swatches, the character styles and the
/// stored brush library.
fn pattern_uses(d: &vectorcraft_doc::Document, name: &str) -> usize {
    let mut n = 0;
    d.visit_paints(&mut |p| n += usize::from(is_pattern(p, name)));
    n += d.swatches_iter().filter(|w| is_pattern(&w.paint, name)).count();
    let styled = d.char_styles.iter().flat_map(|c| ["fill", "stroke"].map(|k| c.attrs.get(k).cloned())).flatten();
    n += styled.filter(|v| serde_json::from_value::<Paint>(v.clone()).is_ok_and(|p| is_pattern(&p, name))).count();
    let needle = format!("\"pattern\":\"{name}\"");
    n + d.unknown.get(vectorcraft_brush::DOC_KEY).map_or(0, |v| v.to_string().matches(&needle).count())
}

/// Canceling a new pattern removes every swatch of it, a color group's included, and the fills and
/// strokes of it on art outside the editing layer become no paint. The object's other paints, and
/// a swatch that only shares the pattern's name, stay.
#[test]
fn cancel_new_pattern_forgets_group_swatch_and_outside_fills_and_strokes() {
    let mut s = session();
    let other = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setStroke", &json!({"ids": [other.0], "color": "#00ff00"})).unwrap();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "Tiles"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [other.0], "swatch": "Tiles"})).unwrap();
    let d = doc_mut(&mut s);
    let solid = Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0));
    d.swatch_groups.push(vectorcraft_color::SwatchGroup {
        name: "Mine".into(),
        swatches: vec![
            vectorcraft_color::Swatch { name: "Grouped".into(), paint: pattern_paint("Tiles"), global: false, spot: false },
            vectorcraft_color::Swatch { name: "Tiles".into(), paint: solid.clone(), global: false, spot: false },
        ],
    });
    assert!(pattern_uses(doc(&s), "Tiles") >= 3);
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    let d = doc(&s);
    assert!(d.pattern("Tiles").is_none());
    assert_eq!(pattern_uses(d, "Tiles"), 0);
    assert!(d.swatch("Grouped").is_none(), "the group's pattern swatch goes");
    assert_eq!(d.swatch("Tiles").map(|w| &w.paint), Some(&solid), "a color swatch named like the pattern stays");
    let n = node(&s, other);
    assert!(n.appearance.fill_paint().is_none());
    assert!(matches!(n.appearance.stroke_paint(), Paint::Solid { .. }), "the stroke stays");
}

/// Save a Copy keeps the copy as its own pattern when the new pattern is canceled, and the copy's
/// tile art loses its paints of the canceled pattern.
#[test]
fn cancel_new_pattern_forgets_its_paints_in_a_saved_copy() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "Tiles"})).unwrap();
    let layer = doc(&s).pattern_edit.as_ref().unwrap().layer;
    let tile = doc(&s).node(layer).unwrap().children().unwrap()[0].id;
    s.execute("paint.setFill", &json!({"ids": [tile.0], "swatch": "Tiles"})).unwrap();
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({})).unwrap()["name"], "Tiles copy");
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    let d = doc(&s);
    let copy = d.pattern("Tiles copy").expect("the copy stays");
    assert_eq!(copy.art.len(), 1);
    assert!(copy.art[0].appearance.fill_paint().is_none());
    assert_eq!(pattern_uses(d, "Tiles"), 0);
}

/// Canceling a new pattern also clears its paints in type, opacity mask art, symbol definitions,
/// graph designs, graphic and character styles and brush art, and in the fill, stroke and
/// appearance new art takes, so new art does not take a paint of a pattern that is gone.
#[test]
fn cancel_new_pattern_forgets_hidden_paints_and_new_art_defaults() {
    let mut s = session();
    let masked = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let m = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("transparency.makeOpacityMask", &json!({"ids": [masked.0, m.0]})).unwrap();
    let t = id_of(&s.execute("text.create", &json!({"x": 20, "y": 300, "text": "Hi"})).unwrap());
    let band = rect(&mut s, 200.0, 200.0, 40.0, 10.0);
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "Tiles"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [band.0], "swatch": "Tiles"})).unwrap();
    s.execute("brush.new", &json!({"type": "art", "name": "Band", "ids": [band.0]})).unwrap();
    let p = pattern_paint("Tiles");
    let d = doc_mut(&mut s);
    let art = d.node(band).unwrap().clone();
    d.symbols.push(vectorcraft_doc::Symbol { name: "Sym".into(), art: Arc::new(art.clone()), dynamic: false });
    d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Bar".into(), art: Arc::new(art.clone()) });
    d.graphic_styles.push(vectorcraft_doc::GraphicStyle {
        name: "Style".into(),
        appearance: (*art.appearance).clone(),
        id: 0,
        opacity: 1.0,
        blend: Default::default(),
        isolate: false,
        knockout: Default::default(),
        unit_box: true,
    });
    let attrs = json!({"fill": p, "stroke": p, "size": 12});
    d.char_styles.push(vectorcraft_doc::TextStyleDef { name: "Char".into(), attrs: attrs.as_object().unwrap().clone() });
    if let Some(NodeKind::Text(text)) = d.node_mut(t).map(|n| &mut n.kind) {
        text.runs[0].style.fill = p.clone();
    }
    let mask = d.node_mut(masked).and_then(|n| n.extras_if_any()).and_then(|e| e.mask.as_deref_mut()).expect("the mask");
    Arc::make_mut(&mut mask.art).appearance = vectorcraft_doc::Appearance::basic(p.clone(), Paint::None, 0.0).into();
    s.paint.fill = p.clone();
    s.paint.appearance = Some(vectorcraft_doc::Appearance::basic(Paint::None, p.clone(), 1.0));
    assert!(pattern_uses(doc(&s), "Tiles") >= 10, "{}", pattern_uses(doc(&s), "Tiles"));
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    assert_eq!(pattern_uses(doc(&s), "Tiles"), 0);
    assert!(doc(&s).char_styles[0].attrs["size"] == json!(12), "the style's other attributes stay");
    assert!(s.paint.fill.is_none());
    assert!(s.paint.appearance.as_ref().unwrap().items.iter().all(|it| match it {
        vectorcraft_doc::AppearanceItem::Fill(f) => !is_pattern(&f.paint, "Tiles"),
        vectorcraft_doc::AppearanceItem::Stroke(k) => !is_pattern(&k.paint, "Tiles"),
    }));
    let fresh = rect(&mut s, 300.0, 300.0, 20.0, 20.0);
    let n = node(&s, fresh);
    assert!(!is_pattern(&n.appearance.fill_paint(), "Tiles") && !is_pattern(&n.appearance.stroke_paint(), "Tiles"));
}

/// Cancel is one undo step: Undo gives the pattern, its paints and the fill new art takes back, and
/// Redo clears them again.
#[test]
fn cancel_new_pattern_is_one_undo_step() {
    let mut s = session();
    let other = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "Tiles"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [other.0], "swatch": "Tiles"})).unwrap();
    s.paint.fill = pattern_paint("Tiles");
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    assert!(s.paint.fill.is_none() && doc(&s).pattern("Tiles").is_none());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(is_pattern(&s.paint.fill, "Tiles"));
    assert!(doc(&s).pattern("Tiles").is_some() && doc(&s).pattern_edit.is_some());
    assert!(is_pattern(&node(&s, other).appearance.fill_paint(), "Tiles"));
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.paint.fill.is_none() && doc(&s).pattern("Tiles").is_none());
    assert_eq!(pattern_uses(doc(&s), "Tiles"), 0);
}

/// Canceling the edit of a pattern that Pattern Options renamed while it was edited gives the
/// pattern its old name back, with its swatch and the paints of it, so nothing is left painted
/// with a name no pattern has.
#[test]
fn cancel_after_renaming_an_edited_pattern_restores_its_name_and_paints() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    s.execute("pattern.options", &json!({"newName": "Spots"})).unwrap();
    assert!(is_pattern(&node(&s, big).appearance.fill_paint(), "Spots"));
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    let d = doc(&s);
    assert!(d.pattern("Dots").is_some() && d.pattern("Spots").is_none());
    assert!(d.swatch("Dots").is_some_and(|w| is_pattern(&w.paint, "Dots")));
    assert!(is_pattern(&node(&s, big).appearance.fill_paint(), "Dots"));
    assert_eq!(pattern_uses(d, "Spots"), 0);
    // When Save a Copy has taken the old name since, the pattern keeps its new name and paints.
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    s.execute("pattern.options", &json!({"newName": "Spots"})).unwrap();
    s.execute("object.pattern.saveCopy", &json!({"name": "Dots"})).unwrap();
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    let d = doc(&s);
    assert!(d.pattern("Dots").is_some() && d.pattern("Spots").is_some());
    assert!(is_pattern(&node(&s, big).appearance.fill_paint(), "Spots"));
}

#[test]
fn edit_then_cancel_restores_definition() {
    let mut s = session();
    make_dots(&mut s);
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    s.execute("pattern.options", &json!({"tileType": "hexByRow", "width": 40})).unwrap();
    assert_eq!(doc(&s).pattern("Dots").unwrap().tile_type, TileType::HexByRow);
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    let def = doc(&s).pattern("Dots").unwrap();
    assert_eq!(def.tile_type, TileType::Grid);
    assert_eq!(def.tile.width(), 20.0);
}

/// Pattern editing mode is not entered while an opacity mask is edited: Edit Pattern and Make are
/// off, with or without a name, and Make also with `edit: false`. Mask editing goes on, its layer
/// still isolated. After Stop Editing Opacity Mask, Edit Pattern works again.
#[test]
fn pattern_editing_is_not_entered_while_an_opacity_mask_is_edited() {
    let mut s = session();
    make_dots(&mut s);
    let obj = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let m = rect(&mut s, 100.0, 100.0, 50.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [m.0], "color": "#ffffff"})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({"ids": [obj.0, m.0]})).unwrap();
    let layer = s.execute("transparency.editOpacityMask", &json!({"id": obj.0})).unwrap()["layer"].as_u64().map(NodeId);
    let why = "stop editing the opacity mask first";
    for (cmd, p) in [
        ("object.pattern.edit", json!({"name": "Dots"})),
        ("object.pattern.edit", json!({})),
        ("object.pattern.make", json!({})),
        ("object.pattern.make", json!({"name": "Mask Art", "edit": false})),
    ] {
        let err = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(&err, EngineError::Disabled(id, w) if id == cmd && w == why), "{cmd} {p}: {err}");
    }
    let st = s.doc().unwrap();
    assert!(st.doc.pattern_edit.is_none() && st.doc.pattern("Mask Art").is_none() && st.doc.pattern("New Pattern 1").is_none());
    assert_eq!(st.doc.mask_edit.map(|e| (e.object, Some(e.layer))), Some((obj, layer)));
    assert_eq!((st.isolation, st.active_layer), (layer, layer));
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.isolation, st.doc.pattern_edit.as_ref().map(|e| e.layer));
}

#[test]
fn edit_defaults_to_selected_objects_pattern() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    sel(&mut s, &[big]);
    let v = s.execute("object.pattern.edit", &json!({})).unwrap();
    assert_eq!(v["name"], "Dots");
    assert!(doc(&s).pattern_edit.is_some());
    // Edit again while editing is refused.
    assert!(s.execute("object.pattern.edit", &json!({"name": "Dots"})).is_err());
}

#[test]
fn set_fill_with_pattern_swatch() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    assert!(matches!(node(&s, big).appearance.fill_paint(), Paint::Pattern { pattern, .. } if pattern == "Dots"));
}

#[test]
fn set_fill_with_pattern_name_without_swatch() {
    let mut s = session();
    make_dots(&mut s);
    Arc::make_mut(&mut s.doc_mut().unwrap().doc).swatches.retain(|sw| sw.name != "Dots");
    let big = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    assert!(matches!(node(&s, big).appearance.fill_paint(), Paint::Pattern { .. }));
}

#[test]
fn options_set_tile_type_spacing_and_overlap() {
    let mut s = session();
    make_dots(&mut s);
    s.execute(
        "pattern.options",
        &json!({"name": "Dots", "tileType": "brickByColumn", "brickOffset": 0.25, "overlap": {"h": "right", "v": "bottom"}, "copies": 7, "dimCopies": 40}),
    )
    .unwrap();
    let def = doc(&s).pattern("Dots").unwrap();
    assert_eq!(def.tile_type, TileType::BrickByColumn { offset: 0.25 });
    assert!(def.overlap.right_in_front && def.overlap.bottom_in_front);
    assert_eq!(def.copies, 7);
    assert_eq!(def.dim_copies, 40.0);
    // Size tile to art with spacing.
    s.execute("pattern.options", &json!({"name": "Dots", "sizeTileToArt": true, "hSpacing": 4, "vSpacing": 6})).unwrap();
    let def = doc(&s).pattern("Dots").unwrap();
    assert!((def.tile.width() - 14.0).abs() < 1e-6 && (def.tile.height() - 16.0).abs() < 1e-6, "{:?}", def.tile);
    assert!(s.execute("pattern.options", &json!({"name": "Dots", "tileType": "zigzag"})).is_err());
    assert!(s.execute("pattern.options", &json!({"name": "Dots", "width": -1})).is_err());
}

#[test]
fn options_rename_updates_swatch_and_paints() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.execute("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    assert!(doc(&s).pattern("Spots").is_some() && doc(&s).pattern("Dots").is_none());
    assert!(doc(&s).swatch("Spots").is_some());
    assert!(matches!(node(&s, big).appearance.fill_paint(), Paint::Pattern { pattern, .. } if pattern == "Spots"));
}

/// Pattern Options refuses a new name that a swatch of another kind, a second swatch of the
/// pattern or a color group has, and takes a name that only the pattern's own swatch has.
#[test]
fn options_rename_refuses_a_name_another_swatch_has() {
    let mut s = session();
    make_dots(&mut s);
    s.execute("swatch.new", &json!({"name": "Brick", "color": "#aa3300"})).unwrap();
    s.execute("swatch.newGroup", &json!({"name": "Mine", "colors": ["#0000ff"]})).unwrap();
    // "Dots copy": a second swatch of the pattern.
    s.execute("swatch.duplicate", &json!({"name": "Dots"})).unwrap();
    for taken in [" Brick ", "Mine", "Dots copy", "[Registration]"] {
        let e = s.execute("pattern.options", &json!({"name": "Dots", "newName": taken})).unwrap_err().to_string();
        assert!(e.contains(&format!("a swatch or color group named `{}` exists", taken.trim())), "{e}");
    }
    assert!(doc(&s).pattern("Dots").is_some());
    let named = |s: &Session, n: &str| doc(s).swatches_iter().filter(|w| w.name == n).count();
    assert_eq!((named(&s, "Brick"), named(&s, "Dots"), named(&s, "Dots copy")), (1, 1, 1));
    // The pattern's own swatch, renamed in Swatch Options: the pattern can take its name.
    s.execute("swatch.delete", &json!({"name": "Dots copy"})).unwrap();
    s.execute("swatch.edit", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    assert_eq!(s.execute("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap()["name"], "Spots");
    assert_eq!(named(&s, "Spots"), 1);
    assert!(matches!(&doc(&s).swatch("Spots").unwrap().paint, Paint::Pattern { pattern, .. } if pattern == "Spots"));
    // With no swatch of the pattern under its name, a color group's name is still refused.
    s.execute("swatch.edit", &json!({"name": "Spots", "newName": "Spot Tiles"})).unwrap();
    assert!(s.execute("pattern.options", &json!({"name": "Spots", "newName": "Mine"})).is_err());
}

/// Renaming a pattern renames only its own swatches: a color swatch that has the pattern's old
/// name keeps it, and art linked to that swatch stays linked.
#[test]
fn options_rename_leaves_other_swatches_of_the_old_name() {
    let mut s = session();
    make_dots(&mut s);
    s.execute("swatch.edit", &json!({"name": "Dots", "newName": "Dot Tiles"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Dots", "color": "#3366cc", "global": true})).unwrap();
    let r = rect(&mut s, 100.0, 100.0, 20.0, 20.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Dots"})).unwrap();
    s.execute("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    assert!(doc(&s).swatch("Dots").is_some_and(|w| matches!(w.paint, Paint::Solid { .. })), "the color swatch keeps its name");
    assert!(matches!(&doc(&s).swatch("Dot Tiles").unwrap().paint, Paint::Pattern { pattern, .. } if pattern == "Spots"));
    assert!(matches!(node(&s, r).appearance.fill_paint(), Paint::Solid { swatch: Some(l), .. } if l == "Dots"));
}

#[test]
fn delete_pattern_unpaints_objects() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    let v = s.execute("pattern.delete", &json!({"name": "Dots"})).unwrap();
    assert_eq!(v["unpainted"], 1);
    assert!(doc(&s).pattern("Dots").is_none() && doc(&s).swatch("Dots").is_none());
    assert!(node(&s, big).appearance.fill_paint().is_none());
    assert!(s.execute("pattern.delete", &json!({"name": "Dots"})).is_err());
}

/// Puts paints of pattern `name` where the layers do not reach: a symbol definition, another
/// pattern's tile, a graphic style, a character style, a type run, a color group's swatch and the
/// fill new art takes.
fn paint_outside_the_layers(s: &mut Session, name: &str) {
    let t = id_of(&s.execute("text.create", &json!({"x": 20, "y": 300, "text": "Hi"})).unwrap());
    let p = pattern_paint(name);
    let d = doc_mut(s);
    let shape = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0));
    let art = Node::path(d.alloc_id(), shape, vectorcraft_doc::Appearance::basic(p.clone(), Paint::None, 0.0));
    d.symbols.push(vectorcraft_doc::Symbol { name: "Sym".into(), art: Arc::new(art.clone()), dynamic: false });
    d.patterns.push(vectorcraft_doc::pattern::PatternDef::new("Other", vec![Arc::new(art.clone())]));
    d.graphic_styles.push(vectorcraft_doc::GraphicStyle {
        name: "Style".into(),
        appearance: (*art.appearance).clone(),
        id: 0,
        opacity: 1.0,
        blend: Default::default(),
        isolate: false,
        knockout: Default::default(),
        unit_box: true,
    });
    let attrs = json!({"fill": p});
    d.char_styles.push(vectorcraft_doc::TextStyleDef { name: "Char".into(), attrs: attrs.as_object().unwrap().clone() });
    if let Some(NodeKind::Text(text)) = d.node_mut(t).map(|n| &mut n.kind) {
        text.runs[0].style.fill = p.clone();
    }
    d.swatch_groups.push(vectorcraft_color::SwatchGroup {
        name: "Mine".into(),
        swatches: vec![vectorcraft_color::Swatch { name: "Grouped".into(), paint: p.clone(), global: false, spot: false }],
    });
    s.paint.fill = p;
}

/// Delete Pattern clears the pattern's paints outside the layers too, and in the fill new art takes.
#[test]
fn delete_pattern_forgets_paints_outside_the_layers() {
    let mut s = session();
    make_dots(&mut s);
    paint_outside_the_layers(&mut s, "Dots");
    assert_eq!(pattern_uses(doc(&s), "Dots"), 7);
    let v = s.execute("pattern.delete", &json!({"name": "Dots"})).unwrap();
    assert_eq!(v["unpainted"], 5, "the symbol, the other tile, the graphic style, the character style and the type");
    assert_eq!(pattern_uses(doc(&s), "Dots"), 0);
    assert!(doc(&s).swatch("Grouped").is_none() && doc(&s).pattern("Other").is_some());
    assert!(s.paint.fill.is_none());
}

/// A Pattern Options rename renames the pattern's paints outside the layers too, and in the fill
/// new art takes.
#[test]
fn rename_pattern_reaches_paints_outside_the_layers() {
    let mut s = session();
    make_dots(&mut s);
    paint_outside_the_layers(&mut s, "Dots");
    s.execute("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    assert_eq!(pattern_uses(doc(&s), "Dots"), 0);
    assert_eq!(pattern_uses(doc(&s), "Spots"), 7);
    assert!(is_pattern(&s.paint.fill, "Spots"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(is_pattern(&s.paint.fill, "Dots"), "Undo gives the fill its old name back");
}

/// With New Art Has Basic Appearance off, Redo of Delete Pattern gives new art the paints the
/// command left, the selected object's appearance taken after the pattern's fill was cleared.
#[test]
fn redo_of_delete_pattern_keeps_the_inherited_new_art_paints() {
    let mut s = session();
    let r = make_dots(&mut s);
    s.prefs.new_art_basic = false;
    sel(&mut s, &[r]);
    s.paint.fill = pattern_paint("Dots");
    s.execute("pattern.delete", &json!({"name": "Dots"})).unwrap();
    let after = s.paint.clone();
    assert!(!is_pattern(&after.fill, "Dots"));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.paint, after);
}

/// A Pattern Options rename tried as a preview changes the fill new art takes only while the
/// preview stands: another preview and Cancel start from the fill the interaction began with.
/// Keeping the preview keeps the rename, and Undo takes it back.
#[test]
fn rename_previews_restore_the_fill_new_art_takes() {
    let mut s = session();
    make_dots(&mut s);
    s.paint.fill = pattern_paint("Dots");
    s.begin_interaction("Pattern Options").unwrap();
    s.preview("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    assert!(is_pattern(&s.paint.fill, "Spots"));
    s.preview("pattern.options", &json!({"name": "Dots", "newName": "Tiles"})).unwrap();
    assert!(is_pattern(&s.paint.fill, "Tiles"));
    s.cancel_interaction().unwrap();
    assert!(is_pattern(&s.paint.fill, "Dots") && doc(&s).pattern("Dots").is_some());
    s.begin_interaction("Pattern Options").unwrap();
    s.preview("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    s.commit_interaction().unwrap();
    assert!(is_pattern(&s.paint.fill, "Spots") && doc(&s).pattern("Spots").is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(is_pattern(&s.paint.fill, "Dots") && doc(&s).pattern("Dots").is_some());
}

/// With New Art Has Basic Appearance off, a canceled rename preview leaves new art with the
/// appearance of the selected object as the document has it again, so no paint of the previewed
/// name is left.
#[test]
fn canceled_rename_preview_leaves_no_inherited_paint_of_the_new_name() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.prefs.new_art_basic = false;
    sel(&mut s, &[big]);
    s.paint = vectorcraft_tools::PaintDefaults::default();
    s.begin_interaction("Pattern Options").unwrap();
    s.preview("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    s.cancel_interaction().unwrap();
    assert!(doc(&s).pattern("Dots").is_some() && doc(&s).pattern("Spots").is_none());
    let items = s.paint.appearance.iter().flat_map(|a| a.items.iter()).map(|it| match it {
        vectorcraft_doc::AppearanceItem::Fill(f) => f.paint.clone(),
        vectorcraft_doc::AppearanceItem::Stroke(k) => k.paint.clone(),
    });
    let paints: Vec<Paint> = [s.paint.fill.clone(), s.paint.stroke.clone()].into_iter().chain(items).collect();
    assert!(paints.iter().all(|p| !is_pattern(p, "Spots")), "{paints:?}");
}

#[test]
fn transform_patterns_only_moves_placement() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    let before = node(&s, big).geometric_bounds();
    s.execute("pattern.transform", &json!({"ids": [big.0], "matrix": [2, 0, 0, 2, 5, 0]})).unwrap();
    let n = node(&s, big);
    assert_eq!(n.geometric_bounds(), before);
    let Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!() };
    assert_eq!(xf.as_coeffs(), [2.0, 0.0, 0.0, 2.0, 5.0, 0.0]);
}

/// #1448, #1460: with Transform Objects off, the transform commands leave the art where it is and
/// move only its pattern tiles (Transform Patterns on), or nothing at all; Transform Again repeats
/// that.
#[test]
fn transforms_without_transform_objects_move_only_the_pattern() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.execute("select.set", &json!({"ids": [big.0]})).unwrap();
    let before = node(&s, big).geometric_bounds();
    let tiles = |s: &Session| {
        let n = node(s, big);
        let Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!() };
        xf.as_coeffs()
    };
    for (cmd, p, want) in [
        ("object.move", json!({"dx": 5, "dy": 0}), [1.0, 0.0, 0.0, 1.0, 5.0, 0.0]),
        ("object.transformAgain", json!({}), [1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
        ("object.reflect", json!({"axis": "vertical", "origin": [0, 0]}), [-1.0, 0.0, 0.0, 1.0, -10.0, 0.0]),
        ("object.transformEach", json!({"moveH": 10, "reference": 0}), [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
    ] {
        // Transform Again repeats the move as it was made, options included.
        if cmd != "object.transformAgain" {
            let mut off = p.clone();
            off["objects"] = json!(false);
            off["patterns"] = json!(false);
            s.execute(cmd, &off).unwrap();
            assert_eq!(node(&s, big).geometric_bounds(), before, "{cmd}: neither transforms");
            off["patterns"] = json!(true);
            s.execute(cmd, &off).unwrap();
        } else {
            s.execute(cmd, &p).unwrap();
        }
        assert_eq!(node(&s, big).geometric_bounds(), before, "{cmd}");
        assert_eq!(tiles(&s), want, "{cmd}");
    }
}

#[test]
fn selection_backquote_drag_changes_pattern_fill_and_stroke_without_moving_mixed_art() {
    let mut s = session();
    make_dots(&mut s);
    let patterned = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let solid = rect(&mut s, 220.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [patterned.0], "swatch": "Dots"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [patterned.0], "swatch": "Dots"})).unwrap();
    s.execute("prefs.set", &json!({"key": "transformPatternTiles", "value": false})).unwrap();
    sel(&mut s, &[patterned, solid]);
    let before_patterned = node(&s, patterned);
    let before_solid = node(&s, solid);
    let history = crate::tests::history(&s).0;
    let v = ViewInfo { show_bbox: false, ..Default::default() };
    s.select_tool("selection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 125.0, 125.0), v).unwrap();
    s.pointer(&PointerEvent { backquote: true, ..PointerEvent::new(PointerKind::Drag, 145.0, 125.0) }, v).unwrap();
    s.pointer(&PointerEvent { backquote: true, ..PointerEvent::new(PointerKind::Up, 145.0, 125.0) }, v).unwrap();
    let after = node(&s, patterned);
    assert_eq!(after.geometric_bounds(), before_patterned.geometric_bounds());
    assert_eq!(node(&s, solid), before_solid);
    for paint in [after.appearance.fill_paint(), after.appearance.stroke_paint()] {
        let Paint::Pattern { xf, .. } = paint else { panic!("pattern paint expected") };
        assert_eq!(xf.as_coeffs()[4], 20.0);
    }
    assert_eq!(crate::tests::history(&s).0, history + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, patterned), before_patterned);
    assert_eq!(node(&s, solid), before_solid);
}

#[test]
fn selection_backquote_drag_changes_stroke_only_pattern() {
    let mut s = session();
    make_dots(&mut s);
    let id = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [id.0], "none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id.0], "swatch": "Dots"})).unwrap();
    sel(&mut s, &[id]);
    let before = node(&s, id).geometric_bounds();
    let v = ViewInfo { show_bbox: false, ..Default::default() };
    s.select_tool("selection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 125.0), v).unwrap();
    s.pointer(&PointerEvent { backquote: true, ..PointerEvent::new(PointerKind::Drag, 120.0, 125.0) }, v).unwrap();
    s.pointer(&PointerEvent { backquote: true, ..PointerEvent::new(PointerKind::Up, 120.0, 125.0) }, v).unwrap();
    let after = node(&s, id);
    assert_eq!(after.geometric_bounds(), before);
    let Paint::Pattern { xf, .. } = after.appearance.stroke_paint() else { panic!("pattern stroke expected") };
    assert_eq!(xf.as_coeffs()[4], 20.0);
}

#[test]
fn list_reports_patterns_and_edit_state() {
    let mut s = session();
    make_dots(&mut s);
    let v = s.execute("pattern.list", &json!({})).unwrap();
    assert_eq!(v["patterns"][0]["name"], "Dots");
    assert_eq!(v["patterns"][0]["tileType"], "grid");
    assert_eq!(v["patterns"][0]["width"], 20.0);
    assert!(v["editing"].is_null());
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    assert_eq!(s.execute("pattern.list", &json!({})).unwrap()["editing"]["name"], "Dots");
}

#[test]
fn save_a_copy_while_editing() {
    let mut s = session();
    make_dots(&mut s);
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    let v = s.execute("object.pattern.saveCopy", &json!({})).unwrap();
    assert_eq!(v["name"], "Dots copy");
    assert!(doc(&s).pattern("Dots copy").is_some());
}

/// Make, Save a Copy and Pattern Options trim the name they are given, as the Pattern Options
/// panel does; the trimmed name is the one compared with the names there are, and a blank name
/// gives the name the command gives when there is none.
#[test]
fn made_copied_and_renamed_patterns_trim_their_names() {
    let mut s = session();
    make_dots(&mut s);
    let make = |s: &mut Session, name: &str| {
        let r = rect(s, 0.0, 0.0, 10.0, 10.0);
        s.execute("object.pattern.make", &json!({"ids": [r.0], "name": name, "edit": false})).unwrap()["name"].clone()
    };
    assert_eq!(make(&mut s, "  Grid \t"), "Grid");
    assert_eq!(make(&mut s, " Dots"), "Dots 2");
    assert_eq!(make(&mut s, ""), "New Pattern 1");
    assert_eq!(make(&mut s, " \t "), "New Pattern 2");
    s.execute("object.pattern.edit", &json!({"name": "Grid"})).unwrap();
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({"name": " Lines\t"})).unwrap()["name"], "Lines");
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({"name": "  "})).unwrap()["name"], "Grid copy");
    s.execute("object.pattern.done", &json!({})).unwrap();
    assert_eq!(s.execute("pattern.options", &json!({"name": "Dots 2", "newName": " Spots "})).unwrap()["name"], "Spots");
    assert_eq!(s.execute("pattern.options", &json!({"name": "Spots", "newName": " "})).unwrap()["name"], "Spots");
    let e = s.execute("pattern.options", &json!({"name": "Spots", "newName": " Lines"})).unwrap_err().to_string();
    assert!(e.contains("a pattern named `Lines` exists"), "{e}");
    // Save a Copy names the copy of a pattern whose saved name has white space around it, as a
    // saved file can hold, from the trimmed name (" Old " gives "Old copy").
    let d = Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    d.patterns.iter_mut().filter(|p| p.name == "Grid copy").for_each(|p| p.name = " Old ".into());
    s.execute("object.pattern.edit", &json!({"name": " Old "})).unwrap();
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({})).unwrap()["name"], "Old copy");
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    let names: Vec<&str> = doc(&s).patterns.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Dots", "Grid", "Spots", "New Pattern 1", "New Pattern 2", "Lines", " Old ", "Old copy"]);
    assert!(doc(&s).swatch("Spots").is_some() && doc(&s).swatch(" Spots ").is_none());
}

/// Make and Save a Copy number a name that a color group has, as they number a swatch's name
/// (swatches and color groups share one namespace).
#[test]
fn made_and_copied_patterns_number_a_color_groups_name() {
    let mut s = session();
    s.execute("swatch.newGroup", &json!({"name": "Mine", "colors": ["#0000ff"]})).unwrap();
    s.execute("swatch.newGroup", &json!({"name": "New Pattern 1", "colors": ["#00ff00"]})).unwrap();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    assert_eq!(s.execute("object.pattern.make", &json!({"ids": [r.0], "name": "Mine", "edit": false})).unwrap()["name"], "Mine 2");
    assert_eq!(s.execute("object.pattern.make", &json!({"ids": [r.0], "edit": false})).unwrap()["name"], "New Pattern 2");
    s.execute("object.pattern.edit", &json!({"name": "Mine 2"})).unwrap();
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({"name": "Mine"})).unwrap()["name"], "Mine 3");
}

#[test]
fn undo_make_pattern() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(doc(&s).pattern("P").is_none() && doc(&s).pattern_edit.is_none());
}

#[test]
fn hit_testing_in_edit_mode_only_reaches_tile_layer() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let other = rect(&mut s, 200.0, 200.0, 50.0, 50.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    let d = doc(&s);
    let opt = vectorcraft_doc::hit::HitOptions::default();
    assert!(vectorcraft_doc::hit::hit_test(d, Point::new(220.0, 220.0), opt).is_none());
    let h = vectorcraft_doc::hit::hit_test(d, Point::new(5.0, 5.0), opt).unwrap();
    assert_ne!(h.leaf, r);
    let _ = other;
}

#[test]
fn save_load_roundtrip_keeps_patterns_and_repeats() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    let r = rect(&mut s, 300.0, 50.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.repeat.radial", &json!({"instances": 6})).unwrap();
    let bytes = vectorcraft_format::save(doc(&s), false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.patterns, doc(&s).patterns);
    assert_eq!(back.node(big).unwrap().appearance.fill_paint(), node(&s, big).appearance.fill_paint());
    assert!(back.layers[0].children().unwrap().iter().any(|n| matches!(n.kind, NodeKind::Repeat(_))));
}

#[test]
fn svg_export_writes_pattern_def() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    let svg = vectorcraft_svg::export(doc(&s), &vectorcraft_svg::ExportOptions::default());
    assert!(svg.contains("<pattern id=\"pattern-1\""), "{svg}");
    assert!(svg.contains("patternUnits=\"userSpaceOnUse\""));
    assert!(svg.contains("width=\"20\" height=\"20\""));
    assert!(svg.contains("fill=\"url(#pattern-1)\"") || svg.contains("fill:url(#pattern-1)"), "{svg}");
    // The tile art is inside the pattern.
    let start = svg.find("<pattern").unwrap();
    let end = svg.find("</pattern>").unwrap();
    assert!(vectorcraft_testkit::svg_shapes(&svg[start..end]) > 0);
}

/// A pattern fill is written as a PDF tiling pattern, which PDF import reads back as a pattern
/// swatch: a rotated hatch over a long bar (#3849) stays one object with one pattern fill, in a few
/// kilobytes, instead of hundreds of tiles.
#[test]
fn pdf_export_writes_pattern_fills_as_tiling_patterns() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="2812" height="300" viewBox="0 0 2812 300">
  <defs>
    <pattern id="hatch" patternUnits="userSpaceOnUse" width="24" height="24" patternTransform="rotate(45)">
      <rect width="12" height="24" fill="#EB003C"/>
      <rect x="12" width="12" height="24" fill="#FFFFFF"/>
    </pattern>
  </defs>
  <rect x="80" y="80" width="2650" height="140" fill="url(#hatch)" stroke="#EB003C" stroke-width="4"/>
</svg>"##;
    let d = vectorcraft_svg::import(svg).unwrap();
    let rep = vectorcraft_pdf::export_with_report(&d, &Default::default()).unwrap();
    assert!(rep.warnings.iter().all(|w| !w.contains("pattern")), "{:?}", rep.warnings);
    assert!(rep.bytes.len() < 8_000, "{} bytes", rep.bytes.len());
    let plain = vectorcraft_pdf::export(&d, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    assert!(String::from_utf8_lossy(&plain).contains("/PatternType 1"));
    let back = vectorcraft_pdf::import(&rep.bytes).unwrap();
    assert_eq!(back.patterns.len(), 1);
    assert_eq!(back.patterns[0].art.len(), 2, "the hatch's two stripes");
    let mut leaves = vec![];
    back.walk(|n| {
        if !n.is_container() {
            leaves.push(n.appearance.fill_paint());
        }
    });
    assert!(leaves.len() <= 2, "{leaves:?}");
    assert!(leaves.iter().any(|p| matches!(p, Paint::Pattern { pattern, .. } if *pattern == back.patterns[0].name)), "{leaves:?}");
}

/// A pattern fill nested more than four patterns deep is written as its tiles, and the export says so.
#[test]
fn pdf_export_writes_patterns_nested_too_deep_as_tiles_with_a_warning() {
    use std::sync::Arc;
    use vectorcraft_doc::pattern::{PatternDef, pattern_paint};
    use vectorcraft_doc::{Appearance, Document};
    let square = |paint: Paint| {
        Node::path(NodeId(0), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(paint, Paint::None, 0.0))
    };
    let mut d = Document::new(100.0, 100.0);
    d.patterns.push(PatternDef::new("P0", vec![Arc::new(square(Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0))))]));
    for i in 1..=4 {
        d.patterns.push(PatternDef::new(&format!("P{i}"), vec![Arc::new(square(pattern_paint(&format!("P{}", i - 1))))]));
    }
    // P3 nests four tiling patterns (P3 to P0); P4 one more, which is written as its tiles.
    for (top, tiles) in [("P3", false), ("P4", true)] {
        let mut d = d.clone();
        let mut n = square(pattern_paint(top));
        n.id = d.alloc_id();
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, n).unwrap();
        let rep = vectorcraft_pdf::export_with_report(&d, &Default::default()).unwrap();
        assert_eq!(rep.warnings.iter().any(|w| w.contains("are written as their tiles")), tiles, "{top}: {:?}", rep.warnings);
    }
}

#[test]
fn every_raster_export_says_when_a_pattern_holds_an_image_that_could_not_be_decoded() {
    // A PNG whose header reads but whose pixel data is cut short, made into a pattern and then
    // cleared from the page: only the pattern's tile draws it.
    let mut s = session();
    let placed = crate::tests_menucmds::place_undecodable_png_sized(&mut s, 20, 20);
    sel(&mut s, &[placed]);
    s.execute("object.pattern.make", &json!({"name": "Cut", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    sel(&mut s, &[placed]);
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(doc(&s).node(placed).is_none());
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Cut"})).unwrap();
    // The first export draws the pattern's tile, and the later ones draw it from the tile kept.
    for n in 0..3 {
        let r = s.execute("document.export", &json!({"format": "png", "background": "transparent"})).unwrap();
        let warnings = r["warnings"].as_array().cloned().unwrap_or_default();
        assert!(warnings.iter().any(|w| w.as_str().is_some_and(|w| w.contains("couldn't be decoded"))), "export {n}: {r}");
    }
}

#[test]
fn a_raster_export_says_when_a_pattern_tile_draws_an_undecodable_image_from_its_kept_rasterize_image() {
    // A PNG whose pixel data is cut short, with Rasterize, a drop shadow and a blur: the shadow
    // draws the Rasterize image, and the blurred content draws the image kept from that draw.
    let mut s = session();
    let placed = place_undecodable_png(&mut s);
    for (effect, params) in [
        ("rasterize", json!({"background": "white"})),
        ("stylize.dropShadow", json!({"x": 2, "y": 2, "blur": 1})),
        ("blur.gaussian", json!({"radius": 4})),
    ] {
        s.execute("effect.apply", &json!({"effect": effect, "params": params, "ids": [placed.0]})).unwrap();
    }
    sel(&mut s, &[placed]);
    s.execute("object.pattern.make", &json!({"name": "Cut", "width": 80, "height": 40})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    sel(&mut s, &[placed]);
    s.execute("edit.clear", &json!({})).unwrap();
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Cut"})).unwrap();
    let warns = |s: &mut Session| {
        let r = s.execute("document.export", &json!({"format": "png", "background": "transparent"})).unwrap();
        r["warnings"].as_array().is_some_and(|w| w.iter().any(|w| w.as_str().is_some_and(|w| w.contains("couldn't be decoded"))))
    };
    assert!(warns(&mut s), "the first export");
    // A new overlap draws a new tile of the same art, and the tile renderer draws its shadow and
    // its blurred content from the rasters it kept.
    s.execute("pattern.options", &json!({"name": "Cut", "overlap": {"h": "right"}})).unwrap();
    assert!(warns(&mut s), "the export after Pattern Options");
}

// ---------- repeat ----------

#[test]
fn radial_repeat_wraps_selection() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({"instances": 8, "radius": 50})).unwrap();
    let n = node(&s, id_of(&v));
    let NodeKind::Repeat(spec) = &n.kind else { panic!("{:?}", n.kind) };
    assert_eq!(spec.source.len(), 1);
    let RepeatKind::Radial { instances, radius, center, .. } = spec.kind else { panic!() };
    assert_eq!((instances, radius), (8, 50.0));
    assert!(close(center, Point::new(200.0, 150.0)));
    assert_eq!(spec.transforms(None).len(), 8);
    assert_eq!(n.kind_label(), "Radial Repeat");
}

/// Repeat given `ids` that list an object and a group that holds it (directly, or inside another
/// group) repeats the outer group, which holds the object once, as it does with those objects
/// selected.
#[test]
fn repeat_of_an_object_and_a_group_that_holds_it_repeats_the_group() {
    let nested = || {
        let mut s = session();
        let (a, b) = (rect(&mut s, 195.0, 95.0, 10.0, 10.0), rect(&mut s, 215.0, 95.0, 10.0, 10.0));
        sel(&mut s, &[a, b]);
        let inner = id_of(&s.execute("object.group", &json!({})).unwrap());
        let c = rect(&mut s, 235.0, 95.0, 10.0, 10.0);
        sel(&mut s, &[inner, c]);
        let outer = id_of(&s.execute("object.group", &json!({})).unwrap());
        s.execute("select.none", &json!({})).unwrap();
        (s, [a, inner, outer])
    };
    for (picks, group) in [(&[0, 1][..], 1), (&[1, 0, 2], 2)] {
        let (mut given, ids) = nested();
        let list: Vec<NodeId> = picks.iter().map(|i| ids[*i]).collect();
        let v = given.execute("object.repeat.radial", &json!({ "ids": list.iter().map(|i| i.0).collect::<Vec<_>>() })).unwrap();
        let NodeKind::Repeat(spec) = node(&given, id_of(&v)).kind else { panic!("{v}") };
        assert_eq!(spec.source.iter().map(|n| n.id).collect::<Vec<_>>(), [ids[group]]);
        let (mut selected, _) = nested();
        sel(&mut selected, &list);
        selected.execute("object.repeat.radial", &json!({})).unwrap();
        assert!(doc(&given).layers == doc(&selected).layers, "ids {list:?}");
    }
}

#[test]
fn radial_instance_transforms_rotate_around_center() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({"instances": 4, "radius": 50})).unwrap();
    let NodeKind::Repeat(spec) = node(&s, id_of(&v)).kind else { panic!() };
    let t = spec.transforms(None);
    let c = Point::new(200.0, 100.0);
    assert!(close(t[0] * c, Point::new(200.0, 100.0)));
    assert!(close(t[1] * c, Point::new(250.0, 150.0)));
    assert!(close(t[2] * c, Point::new(200.0, 200.0)));
    assert!(close(t[3] * c, Point::new(150.0, 150.0)));
}

#[test]
fn expand_yields_n_copies() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({"instances": 5})).unwrap();
    let id = id_of(&v);
    s.execute("object.repeat.expand", &json!({})).unwrap();
    let g = node(&s, id);
    assert!(matches!(g.kind, NodeKind::Group { .. }));
    assert_eq!(g.children().unwrap().len(), 5);
    // Every generated node has a real, unique id.
    let mut ids = vec![];
    g.walk(&mut |n| ids.push(n.id));
    assert!(ids.iter().all(|i| i.0 != 0));
    let mut u = ids.clone();
    u.sort();
    u.dedup();
    assert_eq!(u.len(), ids.len());
}

/// #1756: Object → Expand turns radial, grid and mirror repeats into groups of their instances, in
/// one undo step, as Expand Repeat does.
#[test]
fn object_expand_expands_repeats() {
    for (cmd, opts) in [("object.repeat.radial", json!({"instances": 5})), ("object.repeat.grid", json!({})), ("object.repeat.mirror", json!({}))] {
        let mut s = session();
        let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
        sel(&mut s, &[r]);
        let id = id_of(&s.execute(cmd, &opts).unwrap());
        let NodeKind::Repeat(spec) = node(&s, id).kind else { panic!("{cmd}") };
        let n = spec.expand(None).len();
        assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(true), "{cmd}");
        let undo = s.doc().unwrap().history.undo.len();
        s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
        let g = node(&s, id);
        assert!(matches!(g.kind, NodeKind::Group { .. }), "{cmd}");
        assert_eq!(g.children().unwrap().len(), n, "{cmd}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{cmd}");
        assert_eq!(s.doc().unwrap().selection.objects, vec![id], "{cmd}");
        // Nothing left to expand: the instances are plain rectangles.
        assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(false), "{cmd}");
    }
}

/// Expand reaches repeats inside a selected group, and with every option on also outlines the
/// instances' strokes.
#[test]
fn object_expand_expands_repeats_in_groups() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let rep = id_of(&s.execute("object.repeat.radial", &json!({"instances": 4})).unwrap());
    let other = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[rep, other]);
    s.execute("object.group", &json!({})).unwrap();
    s.execute("object.expand", &json!({})).unwrap();
    let mut repeats = 0;
    for l in &doc(&s).layers {
        l.walk(&mut |c| repeats += usize::from(matches!(c.kind, NodeKind::Repeat(_))));
    }
    assert_eq!(repeats, 0);
    let g = node(&s, rep);
    assert_eq!(g.children().unwrap().len(), 4);
    assert!(g.children().unwrap().iter().all(|c| c.appearance.stroke_paint().is_none()));
}

#[test]
fn grid_repeat_and_options() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.grid", &json!({"hSpacing": 5, "vSpacing": 5})).unwrap();
    let id = id_of(&v);
    assert_eq!(node(&s, id).geometric_bounds(), Some(Rect::new(0.0, 0.0, 40.0, 40.0)));
    s.execute("object.repeat.options", &json!({"rows": 2, "cols": 4, "flipCols": true, "gridType": "brickByRow"})).unwrap();
    let NodeKind::Repeat(spec) = node(&s, id).kind else { panic!() };
    let RepeatKind::Grid { rows, cols, flip_cols, grid_type, .. } = spec.kind else { panic!() };
    assert_eq!((rows, cols, flip_cols), (2, 4, true));
    assert_eq!(grid_type, TileType::BrickByRow { offset: 0.5 });
    assert_eq!(spec.expand(None).len(), 8);
}

#[test]
fn mirror_repeat_reflects() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.mirror", &json!({"offset": 5})).unwrap();
    let n = node(&s, id_of(&v));
    let b = n.geometric_bounds().unwrap();
    assert!((b.x1 - 30.0).abs() < 1e-9 && b.x0.abs() < 1e-9, "{b:?}");
    s.execute("object.repeat.options", &json!({"angle": 0})).unwrap();
    let b = node(&s, id_of(&v)).geometric_bounds().unwrap();
    assert!((b.width() - 10.0).abs() < 1e-9, "{b:?}");
}

#[test]
fn switching_repeat_kind_keeps_source() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({})).unwrap();
    let id = id_of(&v);
    let v2 = s.execute("object.repeat.grid", &json!({})).unwrap();
    assert_eq!(id_of(&v2), id);
    let NodeKind::Repeat(spec) = node(&s, id).kind else { panic!() };
    assert!(matches!(spec.kind, RepeatKind::Grid { .. }));
    assert_eq!(spec.source[0].id, r);
}

#[test]
fn release_restores_source() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    s.execute("object.repeat.mirror", &json!({})).unwrap();
    assert!(doc(&s).layers[0].children().unwrap().len() == 1);
    let v = s.execute("object.repeat.release", &json!({})).unwrap();
    assert_eq!(v["ids"], json!([a.0, b.0]));
    assert_eq!(node(&s, a).geometric_bounds(), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
    assert!(s.execute("object.repeat.release", &json!({})).is_err());
}

#[test]
fn repeat_moves_with_transform() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({"instances": 4, "radius": 50})).unwrap();
    let id = id_of(&v);
    let before = node(&s, id).geometric_bounds().unwrap();
    s.execute("object.transform", &json!({"ids": [id.0], "matrix": [1, 0, 0, 1, 10, 20]})).unwrap();
    let after = node(&s, id).geometric_bounds().unwrap();
    assert!(after.x0 > before.x0 && after.y0 > before.y0, "{before:?} {after:?}");
}

#[test]
fn repeat_hit_test_uses_instances() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.repeat.radial", &json!({"instances": 4, "radius": 50})).unwrap();
    let d = doc(&s);
    let opt = vectorcraft_doc::hit::HitOptions::default();
    assert!(vectorcraft_doc::hit::hit_test(d, Point::new(250.0, 150.0), opt).is_some());
    // The empty centre of the ring is not the repeat.
    assert!(vectorcraft_doc::hit::hit_test(d, Point::new(200.0, 150.0), opt).is_none());
}

#[test]
fn repeat_commands_validate() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    assert!(s.execute("object.repeat.radial", &json!({"instances": 0})).is_err());
    assert!(s.execute("object.repeat.options", &json!({"instances": 4})).is_err(), "no repeat selected");
    s.execute("select.none", &json!({})).ok();
    assert!(s.execute("object.pattern.make", &json!({})).is_err());
}

/// #1970: picking another pattern swatch keeps the transform the fill already had; a fill that was
/// a colour gets the pattern untransformed.
#[test]
fn picking_another_pattern_swatch_keeps_the_pattern_transform() {
    let mut s = session();
    make_dots(&mut s);
    let bars = rect(&mut s, 0.0, 0.0, 20.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [bars.0], "color": "#0000ff"})).unwrap();
    s.execute("object.pattern.make", &json!({"ids": [bars.0], "name": "Bars", "edit": false})).unwrap();
    let big = rect(&mut s, 20.0, 20.0, 200.0, 200.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "color": "#00ff00"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Bars"})).unwrap();
    let Paint::Pattern { xf, .. } = node(&s, big).appearance.fill_paint() else { panic!() };
    assert_eq!(xf.as_coeffs(), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.execute("pattern.transform", &json!({"ids": [big.0], "matrix": [2, 0, 0, 2, 15, 10]})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Bars"})).unwrap();
    let Paint::Pattern { pattern, xf } = node(&s, big).appearance.fill_paint() else { panic!() };
    assert_eq!(pattern, "Bars");
    assert_eq!(xf.as_coeffs(), [2.0, 0.0, 0.0, 2.0, 15.0, 10.0]);
    // The Appearance panel picks the swatch for one fill through `appearance.setItem`.
    let index = node(&s, big).appearance.items.iter().position(|it| matches!(it, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [big.0], "index": index, "swatch": "Dots"})).unwrap();
    let Paint::Pattern { pattern, xf } = node(&s, big).appearance.fill_paint() else { panic!() };
    assert_eq!(pattern, "Dots");
    assert_eq!(xf.as_coeffs(), [2.0, 0.0, 0.0, 2.0, 15.0, 10.0]);
}

/// A pattern swatch made from the current fill holds the pattern untransformed: a fill picked up
/// from art whose tiles were moved does not carry that placement to the art the swatch is applied
/// to, which keeps its own (#1970).
#[test]
fn a_pattern_swatch_made_from_a_transformed_fill_holds_the_pattern_untransformed() {
    let mut s = session();
    make_dots(&mut s);
    let src = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [src.0], "swatch": "Dots"})).unwrap();
    s.execute("pattern.transform", &json!({"ids": [src.0], "matrix": [2, 0, 0, 2, 15, 10]})).unwrap();
    sel(&mut s, &[]);
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    let Paint::Pattern { xf, .. } = &s.paint.fill else { panic!("the picked-up pattern fill") };
    assert_eq!(xf.as_coeffs(), [2.0, 0.0, 0.0, 2.0, 15.0, 10.0], "the current fill has the tiles' placement");
    s.execute("swatch.new", &json!({"name": "Picked"})).unwrap();
    let Paint::Pattern { pattern, xf } = doc(&s).swatch("Picked").unwrap().paint.clone() else { panic!("a pattern swatch") };
    assert_eq!((pattern.as_str(), xf.as_coeffs()), ("Dots", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    let dst = rect(&mut s, 300.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [dst.0], "color": "#00ff00"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [dst.0], "swatch": "Dots"})).unwrap();
    s.execute("pattern.transform", &json!({"ids": [dst.0], "matrix": [3, 0, 0, 3, 0, 0]})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [dst.0], "swatch": "Picked"})).unwrap();
    let Paint::Pattern { xf, .. } = node(&s, dst).appearance.fill_paint() else { panic!("a pattern fill") };
    assert_eq!(xf.as_coeffs(), [3.0, 0.0, 0.0, 3.0, 0.0, 0.0], "the art keeps its own placement");
}

#[test]
fn inner_glow_on_a_pattern_fill_stays_inside_the_object_on_the_threaded_renderer() {
    // #2426: the glow's pixmap was drawn with the pattern's paint transform, so it landed
    // scaled at the top left instead of along the object's edges.
    let mut s = session();
    make_dots(&mut s);
    let bg = rect(&mut s, 0.0, 0.0, 400.0, 400.0);
    s.execute("paint.setFill", &json!({"ids": [bg.0], "color": "#223344"})).unwrap();
    let big = rect(&mut s, 100.0, 100.0, 150.0, 150.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    sel(&mut s, &[big]);
    s.execute("effect.apply", &json!({"effect": "stylize.innerGlow", "params": {"blur": 10}})).unwrap();
    let mut r = vectorcraft_render::Renderer::new();
    r.threads = 2;
    let view = Affine::scale(1.5) * Affine::translate((-20.0, -30.0));
    let out = r.render(&s.doc().unwrap().doc, 400, 400, view, &Default::default());
    let [red, ..] = out.pixel(110, 100);
    assert!(red < 0x30, "glow outside the object: red {red}");
    let [red, ..] = out.pixel(123, 200);
    assert!(red > 0x40, "no glow along the edge: red {red}");
}

fn mirror_axis(s: &Session) -> vectorcraft_tools::mirror::MirrorAxis {
    let st = s.active().unwrap();
    vectorcraft_tools::mirror::MirrorAxis::of(&st.doc, &st.selection, 1.0).unwrap()
}

#[test]
fn mirror_axis_drag_previews_once_and_undo_redo_preserve_source() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 60.0, 80.0);
    let id = id_of(&s.execute("object.repeat.mirror", &json!({})).unwrap());
    let before = node(&s, id);
    let axis = mirror_axis(&s);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let h = crate::tests::history(&s);
    for (kind, dx) in [(PointerKind::Down, 0.0), (PointerKind::Drag, 20.0), (PointerKind::Drag, 30.0), (PointerKind::Up, 40.0)] {
        s.pointer(&PointerEvent::new(kind, axis.center.x + dx, axis.center.y), v).unwrap();
        assert!(close(mirror_axis(&s).center, axis.center + vectorcraft_geom::Vec2::new(dx, 0.0)));
    }
    assert_eq!(crate::tests::history(&s).0, h.0 + 1);
    let after = node(&s, id);
    let (NodeKind::Repeat(a), NodeKind::Repeat(b)) = (&before.kind, &after.kind) else { panic!() };
    assert_eq!(a.source, b.source, "moving the mirror axis leaves the source artwork in place");
    assert_ne!(a.transforms(None), b.transforms(None), "the reflected copy updates live");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(node(&s, id), after);
}

#[test]
fn mirror_axis_rotation_snaps_and_escape_restores_document_and_history() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 60.0, 80.0);
    let id = id_of(&s.execute("object.repeat.mirror", &json!({})).unwrap());
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let before = node(&s, id);
    let h = crate::tests::history(&s);
    let axis = mirror_axis(&s);
    let end = axis.ends[1];
    let destination = axis.center + vectorcraft_geom::Vec2::new(70.0, 60.0);
    s.pointer(&PointerEvent::new(PointerKind::Down, end.x, end.y), v).unwrap();
    let shift = Mods { shift: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Drag, destination.x, destination.y).with_mods(shift), v).unwrap();
    assert!((mirror_axis(&s).angle - 45.0).abs() < 1e-6);
    assert_eq!(mirror_axis(&s).center, axis.center);
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert!(!s.tool_busy());
    s.pointer(&PointerEvent::new(PointerKind::Up, destination.x, destination.y), v).unwrap();
    assert_eq!(node(&s, id), before);
    assert_eq!(crate::tests::history(&s), h);
    // A click is not an edit, and a subsequent real drag still works after cancellation.
    s.pointer(&PointerEvent::new(PointerKind::Down, end.x, end.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, end.x, end.y), v).unwrap();
    assert_eq!(crate::tests::history(&s), h);
    s.pointer(&PointerEvent::new(PointerKind::Down, end.x, end.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, destination.x, destination.y).with_mods(shift), v).unwrap();
    assert!((mirror_axis(&s).angle - 45.0).abs() < 1e-6);
    assert_eq!(crate::tests::history(&s).0, h.0 + 1);
}

#[test]
fn mirror_axis_hidden_controls_and_other_repeat_kinds_do_not_capture_drags() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    for command in ["object.repeat.mirror", "object.repeat.radial", "object.repeat.grid"] {
        let mut s = session();
        rect(&mut s, 100.0, 100.0, 60.0, 80.0);
        let id = id_of(&s.execute(command, &json!({})).unwrap());
        let before = node(&s, id);
        let st = s.active().unwrap();
        if command != "object.repeat.mirror" {
            assert!(vectorcraft_tools::mirror::MirrorAxis::of(&st.doc, &st.selection, 1.0).is_none());
        }
        let v = ViewInfo { show_bbox: false, ..Default::default() };
        s.select_tool("selection", v).unwrap();
        // Away from every instance: the hidden axis's top handle cannot edit the repeat.
        s.pointer(&PointerEvent::new(PointerKind::Down, 170.0, 76.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 80.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 80.0), v).unwrap();
        assert_eq!(node(&s, id), before);
    }
}

/// The document, its revision, the undo and redo stacks and the selection: what a refused call
/// must leave as it was.
fn untouched(s: &Session) -> (std::sync::Arc<vectorcraft_doc::Document>, u64, (usize, usize, bool), Vec<NodeId>) {
    let st = s.doc().unwrap();
    (st.doc.clone(), st.revision, crate::tests::history(s), st.selection.objects.to_vec())
}

/// Each call in `calls` is refused with the parameter error `want` and changes nothing.
fn refused(s: &mut Session, calls: Vec<(&str, Value, String)>) {
    let before = untouched(s);
    for (cmd, p, want) in calls {
        let r = s.execute(cmd, &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}, want {want:?}");
        assert!(untouched(s) == before, "{cmd} {p}: nothing changes");
    }
}

fn number(k: &str, v: Value) -> String {
    format!("`{k}` must be a number, not {v}")
}

fn switch(k: &str) -> String {
    format!("`{k}` must be true or false")
}

fn string(k: &str) -> String {
    format!("`{k}` must be a string")
}

/// Radial, Grid, Mirror and Repeat Options refuse a param of the wrong type with an error naming
/// it, and change nothing; a distance given as text with a unit is refused like any other string.
/// Null counts as a param left out, which keeps its default.
#[test]
fn repeat_commands_refuse_a_param_of_the_wrong_type() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 60.0, 80.0);
    sel(&mut s, &[r]);
    refused(
        &mut s,
        vec![
            ("object.repeat.radial", json!({"instances": "8"}), number("instances", json!("8"))),
            ("object.repeat.radial", json!({"radius": true}), number("radius", json!(true))),
            ("object.repeat.radial", json!({"radius": "5 mm"}), number("radius", json!("5 mm"))),
            ("object.repeat.radial", json!({"radius": -5}), "`radius` must be a number not below 0".into()),
            ("object.repeat.grid", json!({"hSpacing": "5 mm"}), number("hSpacing", json!("5 mm"))),
            ("object.repeat.grid", json!({"vSpacing": [1]}), number("vSpacing", json!([1]))),
            ("object.repeat.grid", json!({"rows": "3"}), number("rows", json!("3"))),
            ("object.repeat.grid", json!({"cols": false}), number("cols", json!(false))),
            ("object.repeat.mirror", json!({"angle": "45"}), number("angle", json!("45"))),
            ("object.repeat.mirror", json!({"offset": {}}), number("offset", json!({}))),
        ],
    );
    // Null params keep the defaults: each repeat matches the one made with `{}`.
    for (cmd, nulls) in [
        ("object.repeat.radial", json!({"instances": null, "radius": null})),
        ("object.repeat.grid", json!({"hSpacing": null, "vSpacing": null, "rows": null, "cols": null})),
        ("object.repeat.mirror", json!({"angle": null, "offset": null})),
    ] {
        let made = |p: &Value| {
            let mut s = session();
            let r = rect(&mut s, 100.0, 100.0, 60.0, 80.0);
            sel(&mut s, &[r]);
            let id = id_of(&s.execute(cmd, p).unwrap());
            node(&s, id)
        };
        assert_eq!(made(&nulls), made(&json!({})), "{cmd}");
    }
    // A grid's rows and cols take any number, as Repeat Options does: 2.5 rows are 2.
    let id = id_of(&s.execute("object.repeat.grid", &json!({"rows": 2.5, "cols": 4})).unwrap());
    assert!(matches!(node(&s, id).kind, NodeKind::Repeat(ref r) if matches!(r.kind, RepeatKind::Grid { rows: 2, cols: 4, .. })));
    // `brickOffset` alone sets the offset of a brick grid, and leaves a plain grid as it is.
    let grid_type = |s: &Session| match node(s, id).kind {
        NodeKind::Repeat(r) => match r.kind {
            RepeatKind::Grid { grid_type, .. } => grid_type,
            other => panic!("not a grid: {other:?}"),
        },
        other => panic!("not a repeat: {other:?}"),
    };
    s.execute("object.repeat.options", &json!({"brickOffset": 0.25})).unwrap();
    assert_eq!(grid_type(&s), TileType::Grid);
    s.execute("object.repeat.options", &json!({"gridType": "brickByRow"})).unwrap();
    s.execute("object.repeat.options", &json!({"brickOffset": 0.25})).unwrap();
    assert_eq!(grid_type(&s), TileType::BrickByRow { offset: 0.25 });

    // Repeat Options checks every param, whichever kind the selected repeat is.
    s.execute("object.repeat.radial", &json!({"instances": 4, "radius": 50})).unwrap();
    let kind = |s: &Session| match node(s, id).kind {
        NodeKind::Repeat(r) => r.kind,
        other => panic!("not a repeat: {other:?}"),
    };
    let radial = kind(&s);
    refused(
        &mut s,
        vec![
            ("object.repeat.options", json!({"instances": "4"}), number("instances", json!("4"))),
            ("object.repeat.options", json!({"radius": "50 pt"}), number("radius", json!("50 pt"))),
            ("object.repeat.options", json!({"startAngle": true}), number("startAngle", json!(true))),
            ("object.repeat.options", json!({"endAngle": "x"}), number("endAngle", json!("x"))),
            ("object.repeat.options", json!({"hSpacing": "1"}), number("hSpacing", json!("1"))),
            ("object.repeat.options", json!({"vSpacing": [1]}), number("vSpacing", json!([1]))),
            ("object.repeat.options", json!({"rows": "2"}), number("rows", json!("2"))),
            ("object.repeat.options", json!({"cols": [2]}), number("cols", json!([2]))),
            ("object.repeat.options", json!({"brickOffset": "0.5"}), number("brickOffset", json!("0.5"))),
            ("object.repeat.options", json!({"angle": "10"}), number("angle", json!("10"))),
            ("object.repeat.options", json!({"reverseOverlap": "true"}), switch("reverseOverlap")),
            ("object.repeat.options", json!({"flipRows": 1}), switch("flipRows")),
            ("object.repeat.options", json!({"flipCols": "no"}), switch("flipCols")),
            ("object.repeat.options", json!({"gridType": 3}), string("gridType")),
            ("object.repeat.options", json!({"center": [1]}), "center must be [x, y] with finite coordinates".into()),
            ("object.repeat.options", json!({"center": "0, 0"}), "center must be [x, y] with finite coordinates".into()),
        ],
    );
    let all_null = json!({"instances": null, "radius": null, "startAngle": null, "endAngle": null, "reverseOverlap": null, "center": null});
    s.execute("object.repeat.options", &all_null).unwrap();
    assert_eq!(kind(&s), radial, "null params keep the options");
    s.execute("object.repeat.options", &json!({"instances": 6, "reverseOverlap": true, "center": [10, 20]})).unwrap();
    assert!(matches!(kind(&s), RepeatKind::Radial { instances: 6, reverse_overlap: true, .. }));
}

/// Make, Edit Pattern, Save a Copy, Pattern Options and Delete Pattern refuse a param of the wrong
/// type (and Pattern Options an `overlap` side it doesn't know) with an error naming it, and change
/// nothing. Null counts as a param left out, which keeps its default.
#[test]
fn pattern_commands_refuse_a_param_of_the_wrong_type() {
    let mut s = session();
    let r = make_dots(&mut s);
    sel(&mut s, &[r]);
    let options = |p: Value| {
        let mut p = p;
        p["name"] = json!("Dots");
        p
    };
    refused(
        &mut s,
        vec![
            ("object.pattern.make", json!({"name": 5}), string("name")),
            ("object.pattern.make", json!({"tileType": 2}), string("tileType")),
            ("object.pattern.make", json!({"brickOffset": "0.5"}), number("brickOffset", json!("0.5"))),
            ("object.pattern.make", json!({"width": "20"}), number("width", json!("20"))),
            ("object.pattern.make", json!({"height": true}), number("height", json!(true))),
            ("object.pattern.make", json!({"edit": "false"}), switch("edit")),
            ("object.pattern.edit", json!({"name": 5}), string("name")),
            ("pattern.options", json!({"name": ["Dots"]}), string("name")),
            ("pattern.options", options(json!({"newName": 5})), string("newName")),
            ("pattern.options", options(json!({"tileType": []})), string("tileType")),
            ("pattern.options", options(json!({"width": "20"})), number("width", json!("20"))),
            ("pattern.options", options(json!({"hSpacing": "1"})), number("hSpacing", json!("1"))),
            ("pattern.options", options(json!({"vSpacing": true})), number("vSpacing", json!(true))),
            ("pattern.options", options(json!({"copies": "5"})), number("copies", json!("5"))),
            ("pattern.options", options(json!({"dimCopies": "50"})), number("dimCopies", json!("50"))),
            ("pattern.options", options(json!({"sizeTileToArt": "true"})), switch("sizeTileToArt")),
            ("pattern.options", options(json!({"showTileEdge": 1})), switch("showTileEdge")),
            ("pattern.options", options(json!({"showSwatchBounds": "no"})), switch("showSwatchBounds")),
            ("pattern.options", options(json!({"overlap": "right"})), "`overlap` must be an object, not \"right\"".into()),
            ("pattern.options", options(json!({"overlap": {"h": "center"}})), "`overlap.h` must be one of left, right".into()),
            ("pattern.options", options(json!({"overlap": {"v": 1}})), "`overlap.v` must be one of top, bottom".into()),
            ("pattern.delete", json!({"name": 5}), string("name")),
        ],
    );
    // Null params keep the defaults: a new pattern named "New Pattern 1", a grid tile the size of
    // the art, edited.
    let made = s
        .execute("object.pattern.make", &json!({"name": null, "tileType": null, "brickOffset": null, "width": null, "height": null, "edit": null}))
        .unwrap();
    assert_eq!(made["name"], "New Pattern 1");
    let def = doc(&s).pattern("New Pattern 1").unwrap().clone();
    assert_eq!(def.tile_type, TileType::Grid);
    assert!((def.tile.width() - 10.0).abs() < 1e-6, "{:?}", def.tile);
    assert_eq!(doc(&s).pattern_edit.as_ref().map(|e| e.pattern.as_str()), Some("New Pattern 1"));
    // In editing mode: Save a Copy refuses a name that isn't a string, and Pattern Options with
    // null params keeps the edited pattern's options.
    refused(&mut s, vec![("object.pattern.saveCopy", json!({"name": 5}), string("name"))]);
    s.execute("pattern.options", &json!({"name": null, "overlap": {"h": null, "v": null}, "copies": null, "sizeTileToArt": null})).unwrap();
    assert!(doc(&s).pattern("New Pattern 1").unwrap() == &def);
    s.execute("pattern.options", &json!({"overlap": {"h": "right"}})).unwrap();
    assert!(doc(&s).pattern("New Pattern 1").unwrap().overlap.right_in_front);
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({"name": null})).unwrap()["name"], "New Pattern 1 copy");
    s.execute("object.pattern.done", &json!({})).unwrap();
    assert!(s.execute("pattern.delete", &json!({"name": null})).is_err(), "no name given");
}
