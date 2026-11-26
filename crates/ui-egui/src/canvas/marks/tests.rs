//! The marks on the art: the memos, the gates and the placed marks, read from [`marks_of`].

use super::*;
use serde_json::json;
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app
}

/// An upright view of 800 × 600 points on the document's middle at zoom 1.
fn view() -> CanvasView {
    CanvasView::new(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)), Point::new(200.0, 150.0), 1.0)
}

fn text(app: &mut VectorcraftApp, x: i64, y: i64, text: &str) -> NodeId {
    NodeId(app.session.execute("text.create", &json!({"x": x, "y": y, "text": text})).unwrap()["id"].as_u64().unwrap())
}

/// The memo's marks of `ids`, in order.
fn kept_hidden(app: &VectorcraftApp, ids: &[NodeId]) -> Vec<Arc<Vec<PlacedMark>>> {
    ids.iter().map(|id| app.canvas.hidden_chars.marks[id].1.clone()).collect()
}

/// Passes until the fonts and the hyphenation exceptions (which other tests change) stay the same
/// between two of them: the memo then holds what one pass laid out.
fn settled(app: &mut VectorcraftApp, view: &CanvasView, ids: &[NodeId]) -> Vec<Arc<Vec<PlacedMark>>> {
    (0..100)
        .find_map(|_| {
            marks_of(app, view);
            let (inputs, before) = (app.canvas.hidden_chars.inputs.clone(), kept_hidden(app, ids));
            marks_of(app, view);
            let after = kept_hidden(app, ids);
            (app.canvas.hidden_chars.inputs == inputs && before.iter().zip(&after).all(|(a, b)| Arc::ptr_eq(a, b))).then_some(after)
        })
        .expect("the fonts settle")
}

/// An unchanged type object keeps its memo entry; an edit lays out only the object edited; a zoom
/// reuses the entries (the marks are in document points); a new font generation lays everything out.
/// Other tests change the fonts while this one runs, so the edit and the zoom are tried again on a
/// new document until the fonts stay the same from the settled pass through the edited one to the
/// zoomed one.
#[test]
fn hidden_characters_lay_out_only_type_that_changed() {
    let zoomed = CanvasView::new(view().rect, Point::new(100.0, 100.0), 2.0);
    let (mut app, ids, before, after, zoom) = (0..100)
        .find_map(|_| {
            let mut app = app();
            let ids = [text(&mut app, 20, 40, "a b\nc d"), text(&mut app, 20, 160, "e f")];
            app.ui.view.hidden_chars = true;
            let before = settled(&mut app, &view(), &ids);
            let inputs = app.canvas.hidden_chars.inputs.clone();
            app.session.execute("text.setText", &json!({"id": ids[1].0, "text": "e f g"})).unwrap();
            marks_of(&mut app, &view());
            let edited = app.canvas.hidden_chars.inputs == inputs;
            let after = kept_hidden(&app, &ids);
            marks_of(&mut app, &zoomed);
            let zoom = kept_hidden(&app, &ids);
            (edited && app.canvas.hidden_chars.inputs == inputs).then_some((app, ids, before, after, zoom))
        })
        .expect("the fonts settle");
    assert!(Arc::ptr_eq(&before[0], &after[0]), "the object not edited is not laid out again");
    assert!(!Arc::ptr_eq(&before[1], &after[1]), "the edited object is");
    assert!(zoom.iter().zip(&after).all(|(a, b)| Arc::ptr_eq(a, b)), "a zoom lays out no type");
    app.canvas.hidden_chars.inputs = (u64::MAX, String::new());
    marks_of(&mut app, &view());
    assert!(kept_hidden(&app, &ids).iter().zip(&after).all(|(a, b)| !Arc::ptr_eq(a, b)), "other fonts lay all type out again");
}

/// Type partly or wholly in a turned view is placed; type wholly outside it is neither placed nor kept.
#[test]
fn hidden_characters_skip_type_outside_a_rotated_view() {
    let mut app = app();
    let ids = [text(&mut app, 20, 60, "a b"), text(&mut app, 3000, 3000, "c d")];
    app.ui.view.hidden_chars = true;
    let turned = view().rotation(45.0);
    let m = marks_of(&mut app, &turned);
    assert!(!m.hidden.is_empty());
    assert!(m.hidden.iter().all(|p| p.at.x < 1000.0), "{:?}", m.hidden);
    assert!(app.canvas.hidden_chars.marks.contains_key(&ids[0]) && !app.canvas.hidden_chars.marks.contains_key(&ids[1]));
}

/// A text holding each hidden character gets each kind of mark once.
#[test]
fn hidden_characters_get_every_kind_of_mark() {
    let mut app = app();
    text(&mut app, 20, 60, "a b\tc\u{00A0}d\u{202F}e\u{2003}f\u{00AD}g\u{2028}h\ni");
    app.ui.view.hidden_chars = true;
    let m = marks_of(&mut app, &view());
    use HiddenMark::*;
    for kind in [Space, Tab, NoBreakSpace, NarrowNoBreakSpace, FixedSpace, DiscretionaryHyphen, LineBreak, ParagraphEnd, StoryEnd] {
        assert_eq!(m.hidden.iter().filter(|p| p.mark == kind).count(), 1, "{kind:?} in {:?}", m.hidden);
    }
}

/// Turning Show Hidden Characters off drops the memo's text nodes, and so does closing the document.
#[test]
fn hidden_characters_off_and_a_closed_document_release_the_text_nodes() {
    let mut app = app();
    let id = text(&mut app, 20, 60, "one\ttwo\u{00A0}x\nthree\u{2028}four");
    app.ui.view.hidden_chars = true;
    let count = |app: &VectorcraftApp| {
        let children = app.session.active().unwrap().doc.layers[0].children().unwrap();
        Arc::strong_count(children.iter().find(|n| n.id == id).unwrap())
    };
    marks_of(&mut app, &view());
    let held = count(&app);
    assert!(app.canvas.text_nodes.list.is_some(), "the list of the text objects is kept");
    app.ui.view.hidden_chars = false;
    marks_of(&mut app, &view());
    assert!(app.canvas.hidden_chars.marks.is_empty() && count(&app) < held, "off: no node is kept");
    assert!(app.canvas.text_nodes.list.is_none(), "off: no list of the text objects is kept");
    app.ui.view.hidden_chars = true;
    marks_of(&mut app, &view());
    assert_eq!(app.canvas.hidden_chars.marks.len(), 1);
    app.session.execute("file.close", &json!({})).unwrap();
    marks_of(&mut app, &view());
    assert!(app.canvas.hidden_chars.marks.is_empty(), "no node of the closed document is kept");
    assert!(app.canvas.text_nodes.list.is_none(), "no list of the closed document's text objects is kept");
}

/// Closing the last document drops the memos' text nodes on the canvas's next frame, whether it
/// shows the Home screen or, with Show The Home Screen When No Documents Are Open off, an empty window.
#[test]
fn a_canvas_frame_without_a_document_releases_the_text_nodes() {
    let ctx = crate::testing::ui_context();
    let frame = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))), ..Default::default() };
        crate::testing::part_frame(app, &ctx, raw, crate::canvas::show);
    };
    for home in [true, false] {
        let mut app = app();
        app.session.execute("prefs.set", &json!({"key": "showHomeScreen", "value": home})).unwrap();
        text(&mut app, 20, 60, "one\ttwo qzxv\nthree");
        app.ui.view.hidden_chars = true;
        app.ui.view.auto_spell_check = true;
        frame(&mut app);
        assert!(app.canvas.text_nodes.list.is_some() && !app.canvas.hidden_chars.marks.is_empty(), "home {home}: the canvas keeps the memos");
        app.session.execute("file.close", &json!({})).unwrap();
        assert_eq!(crate::menus::home_showing(&app), home);
        frame(&mut app);
        assert!(app.canvas.text_nodes.list.is_none(), "home {home}: no list of the closed document's text objects is kept");
        assert!(app.canvas.hidden_chars.marks.is_empty(), "home {home}: no hidden-character marks of the closed document are kept");
        assert!(app.canvas.spelling.lines.is_empty(), "home {home}: no spelling marks of the closed document are kept");
    }
}

/// `paths` small rectangles and three point type objects with hidden characters and words no
/// dictionary knows: the first on the first layer, the second in a group on that layer and the
/// third on a second layer. Show Hidden Characters and Auto Spell Check are on. Returns the type
/// objects, the group and the second layer.
fn type_among_paths(paths: usize) -> (VectorcraftApp, [NodeId; 3], NodeId, NodeId) {
    let mut app = app();
    let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
    let path = id(app.session.execute("shape.rectangle", &json!({"x": 300, "y": 250, "width": 5, "height": 5})).unwrap());
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let template = doc.node(path).unwrap().clone();
        let more: Vec<_> = (1..paths)
            .map(|_| {
                let mut p = template.clone();
                p.id = doc.alloc_id();
                Arc::new(p)
            })
            .collect();
        Arc::make_mut(&mut doc.layers[0]).children_mut().unwrap().extend(more);
        doc.drop_lookup_cache();
    }
    let a = text(&mut app, 20, 40, "one\ttwo\u{00A0}the\nthree\u{2028}four");
    let b = text(&mut app, 20, 110, "Qzvbtrw the\u{2003}cat");
    app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let group = id(app.session.execute("object.group", &json!({})).unwrap());
    let layer = id(app.session.execute("layer.new", &json!({})).unwrap());
    let c = text(&mut app, 20, 180, "Zvqtrbx the\u{202F}cat\n");
    assert_eq!(app.session.active().unwrap().doc.layers.iter().find(|l| l.id == layer).and_then(|l| l.children()).map(|c| c.len()), Some(1));
    app.ui.view.hidden_chars = true;
    app.ui.view.auto_spell_check = true;
    (app, [a, b, c], group, layer)
}

/// The list of text objects Show Hidden Characters and Auto Spell Check last drew over.
fn listed_type(app: &VectorcraftApp) -> TextList {
    app.canvas.text_nodes.list.as_ref().map(|(_, l)| l.clone()).unwrap()
}

/// Show Hidden Characters and Auto Spell Check find the type among the document's other objects
/// once for each revision, in one list both draw from: passes that change nothing draw from the
/// same list, and draw the marks and lines a pass drawing from a new list draws. The list holds the
/// visible type in `Document::walk` order.
#[test]
fn hidden_characters_and_auto_spell_check_share_one_list_of_the_type() {
    let (mut app, ids, _, _) = type_among_paths(2_000);
    let mut walked = vec![];
    app.session.active().unwrap().doc.walk(|n| {
        if matches!(n.kind, NodeKind::Text(_)) && n.visible {
            walked.push(n.id);
        }
    });
    assert_eq!(walked.len(), 3);
    let first = marks_of(&mut app, &view());
    let list = listed_type(&app);
    assert_eq!(list.iter().map(|n| n.id).collect::<Vec<_>>(), walked);
    assert!(list.iter().all(|n| ids.contains(&n.id)), "only type");
    assert_eq!(first.spelling.len(), 2, "{:?}", first.spelling);
    for _ in 0..3 {
        let m = marks_of(&mut app, &view());
        assert_eq!((m.hidden, m.spelling), (first.hidden.clone(), first.spelling.clone()));
        assert!(Arc::ptr_eq(&list, &listed_type(&app)), "a pass that changes nothing lists no type");
    }
    app.canvas.text_nodes = TextNodes::default();
    app.canvas.hidden_chars = HiddenChars::default();
    app.canvas.spelling = SpellingMarks::default();
    let again = marks_of(&mut app, &view());
    assert_eq!((again.hidden, again.spelling), (first.hidden, first.spelling), "the marks and lines drawn from a new list");
    assert!(!Arc::ptr_eq(&list, &listed_type(&app)));
}

/// Auto Spell Check alone keeps the list too, and turning it off drops it.
#[test]
fn auto_spell_check_alone_keeps_the_list_of_the_type() {
    let (mut app, _, _, _) = type_among_paths(10);
    app.ui.view.hidden_chars = false;
    assert_eq!(marks_of(&mut app, &view()).spelling.len(), 2);
    let list = listed_type(&app);
    marks_of(&mut app, &view());
    assert!(Arc::ptr_eq(&list, &listed_type(&app)), "kept while Auto Spell Check draws");
    app.ui.view.auto_spell_check = false;
    marks_of(&mut app, &view());
    assert!(app.canvas.text_nodes.list.is_none(), "dropped with both off");
}

/// The list follows the document: a new text object, an edit, Undo, Redo and a switch to another
/// document each list the type again, and the marks are those of the type the document holds.
#[test]
fn the_list_of_type_follows_edits_undo_redo_and_the_active_document() {
    let (mut app, ids, _, _) = type_among_paths(50);
    // Each listed object's id and node.
    let nodes = |app: &VectorcraftApp| listed_type(app).iter().map(|n| (n.id, Arc::as_ptr(n))).collect::<Vec<_>>();
    // The nonbreaking spaces marked in `id`.
    let no_breaks =
        |app: &VectorcraftApp, id: NodeId| app.canvas.hidden_chars.marks[&id].1.iter().filter(|p| p.mark == HiddenMark::NoBreakSpace).count();
    marks_of(&mut app, &view());
    let first = nodes(&app);
    assert_eq!(first.len(), 3);
    let added = text(&mut app, 200, 40, "g h");
    marks_of(&mut app, &view());
    let with_new = nodes(&app);
    assert_eq!(with_new.len(), 4);
    assert!(with_new.iter().any(|(id, _)| *id == added), "the new object");
    assert!(app.canvas.hidden_chars.marks.contains_key(&added), "has marks");
    app.session.execute("text.setText", &json!({"id": ids[0].0, "text": "x\ty"})).unwrap();
    marks_of(&mut app, &view());
    let edited = nodes(&app);
    let node_of = |list: &[(NodeId, *const Node)], id| list.iter().find(|(i, _)| *i == id).map(|(_, n)| *n);
    assert_ne!(node_of(&edited, ids[0]), node_of(&with_new, ids[0]), "the edited object's new node");
    assert_eq!(node_of(&edited, ids[1]), node_of(&with_new, ids[1]), "the others' nodes");
    assert_eq!(no_breaks(&app, ids[0]), 0, "the marks of the edited text");
    app.session.execute("edit.undo", &json!({})).unwrap();
    marks_of(&mut app, &view());
    assert_eq!(nodes(&app), with_new, "Undo");
    assert_eq!(no_breaks(&app, ids[0]), 1, "the marks of the text before");
    app.session.execute("edit.redo", &json!({})).unwrap();
    marks_of(&mut app, &view());
    assert_eq!(nodes(&app), edited, "Redo");
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    assert!(marks_of(&mut app, &view()).hidden.is_empty(), "another document: none of the first one's marks");
    assert!(nodes(&app).is_empty() && app.canvas.hidden_chars.marks.is_empty());
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    marks_of(&mut app, &view());
    assert_eq!(nodes(&app), edited, "back to the first document");
}

/// Neither overlay draws over type that the canvas does not draw: a hidden text object, or one in
/// a hidden group or layer. Shown again, its marks and lines come back.
#[test]
fn hidden_characters_and_auto_spell_check_leave_out_type_in_hidden_groups_and_layers() {
    let (mut app, [a, b, c], group, layer) = type_among_paths(10);
    let drawn = |app: &mut VectorcraftApp| {
        let m = marks_of(app, &view());
        let mut ids: Vec<_> = app.canvas.hidden_chars.marks.keys().copied().collect();
        ids.sort();
        let mut spelled: Vec<_> = app.canvas.spelling.lines.keys().copied().collect();
        spelled.sort();
        (ids, spelled, m.hidden.len(), m.spelling.len())
    };
    let all = drawn(&mut app);
    assert_eq!((all.0.clone(), all.1.clone(), all.3), (vec![a, b, c], vec![a, b, c], 2));
    app.session.execute("object.setProps", &json!({"ids": [group.0], "visible": false})).unwrap();
    let (ids, spelled, marks, lines) = drawn(&mut app);
    assert_eq!((ids, spelled, lines), (vec![a, c], vec![a, c], 1), "a hidden group");
    assert!(marks < all.2, "fewer marks: {marks} of {}", all.2);
    assert!(listed_type(&app).iter().all(|n| n.id != b), "not listed");
    app.session.execute("object.setProps", &json!({"ids": [group.0], "visible": true})).unwrap();
    assert_eq!(drawn(&mut app), all, "the group shown again");
    app.session.execute("layer.setProps", &json!({"id": layer.0, "visible": false})).unwrap();
    let (ids, spelled, _, lines) = drawn(&mut app);
    assert_eq!((ids, spelled, lines), (vec![a, b], vec![a, b], 1), "a hidden layer");
    app.session.execute("layer.setProps", &json!({"id": layer.0, "visible": true})).unwrap();
    assert_eq!(drawn(&mut app), all, "the layer shown again");
    app.session.execute("object.setProps", &json!({"ids": [a.0], "visible": false})).unwrap();
    let (ids, spelled, _, _) = drawn(&mut app);
    assert_eq!((ids, spelled), (vec![b, c], vec![b, c]), "a hidden text object");
    app.session.execute("object.setProps", &json!({"ids": [a.0], "visible": true})).unwrap();
    assert_eq!(drawn(&mut app), all, "the text object shown again");
}

/// An ignored diagnostic for the marks' cost on a document of many paths and a little type: passes
/// drawing from the kept list, and passes that list the type again each time. There is no timing
/// threshold; run on an idle machine and compare the printed cases.
#[test]
#[ignore = "manual performance diagnostic; no stable timing threshold"]
fn type_marks_on_many_paths_diagnostic() {
    use std::time::Instant;
    const PASSES: usize = 50;
    for paths in [10_000, 100_000] {
        let (mut app, _, _, _) = type_among_paths(paths);
        marks_of(&mut app, &view());
        let t = Instant::now();
        for _ in 0..PASSES {
            marks_of(&mut app, &view());
        }
        let kept = t.elapsed();
        let t = Instant::now();
        for _ in 0..PASSES {
            app.canvas.text_nodes = TextNodes::default();
            marks_of(&mut app, &view());
        }
        let listed = t.elapsed();
        eprintln!("{paths} paths: {:?} a pass from the kept list, {:?} a pass listing the type again", kept / PASSES as u32, listed / PASSES as u32);
    }
}

/// The selected groups' gaps are in their Gap Preview Color, and the list is kept per revision.
#[test]
fn live_paint_gaps_follow_the_selected_groups() {
    let mut app = app();
    let mut groups = vec![];
    for (x, color) in [(0, "#123456"), (150, "#654321")] {
        let anchors = [(0, 0), (100, 0), (100, 100), (0, 100), (0, 2)].map(|(dx, y)| json!({"x": x + dx, "y": y}));
        let p = app.session.execute("path.create", &json!({ "anchors": anchors })).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.set", &json!({"ids": [p]})).unwrap();
        let g = app.session.execute("livePaint.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("livePaint.gapOptions", &json!({"id": g, "color": color})).unwrap();
        groups.push(g);
    }
    app.session.execute("select.set", &json!({"ids": [groups[0]]})).unwrap();
    let first = live_paint_gap_overlays(&mut app);
    assert!(!first.is_empty());
    let kept = app.canvas.live_paint_gaps.as_ref().unwrap().1.clone();
    live_paint_gap_overlays(&mut app);
    assert!(Arc::ptr_eq(&kept, &app.canvas.live_paint_gaps.as_ref().unwrap().1), "one revision, one list");
    app.session.execute("select.set", &json!({"ids": [groups[1]]})).unwrap();
    assert_ne!(format!("{first:?}"), format!("{:?}", live_paint_gap_overlays(&mut app)), "the other group's gaps, in its color");
    app.session.execute("select.set", &json!({"ids": []})).unwrap();
    assert!(live_paint_gap_overlays(&mut app).is_empty());
}

/// Draw Inside's arms: an open rectangle round the object, each arm a third of its side or 16
/// points at most, kept with the object deselected and gone with the mode.
#[test]
fn draw_inside_marks_the_corners_of_its_object() {
    let mut app = app();
    let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 120, "height": 90})).unwrap()["id"].as_u64().unwrap();
    let id = NodeId(id);
    assert!(marks_of(&mut app, &view()).arms.is_empty());
    app.run("view.drawMode", json!({"mode": "inside"})).unwrap();
    let arms = marks_of(&mut app, &view()).arms;
    assert_eq!(arms.len(), 8);
    assert!(arms.iter().all(|[a, b]| (a.distance(*b) - 16.0).abs() < 1e-3), "{arms:?}");
    // Outside the object with its stroke.
    let outer = app.session.active().unwrap().doc.node(id).unwrap().visual_bounds().unwrap();
    let corner = view().to_screen(Point::new(outer.x0 - 4.0, outer.y0 - 4.0));
    assert!(arms.iter().any(|[a, _]| a.distance(corner) < 1e-3), "padded by 4 points");
    app.run("select.set", json!({"ids": []})).unwrap();
    assert_eq!(marks_of(&mut app, &view()).arms.len(), 8, "deselected, the object is still marked");
    let small = inside_arms(&view(), Rect::new(0.0, 0.0, 30.0, 30.0));
    assert!(small.iter().all(|[a, b]| a.distance(*b) < 16.0 && (a.distance(*b) - 38.0 / 3.0).abs() < 1e-3), "a third of a 38-point side");
    app.run("view.drawMode", json!({"mode": "normal"})).unwrap();
    assert!(marks_of(&mut app, &view()).arms.is_empty());
}

/// A document with one point type object of `text` at the top left.
fn spelling_doc(text_: &str) -> (VectorcraftApp, NodeId) {
    let mut app = app();
    let id = text(&mut app, 20, 60, text_);
    app.ui.view.auto_spell_check = true;
    (app, id)
}

fn segments(app: &mut VectorcraftApp) -> usize {
    marks_of(app, &view()).spelling.len()
}

/// One segment under each unknown word, within the type's box, and none for known words.
#[test]
fn auto_spell_check_underlines_the_unknown_words() {
    let (mut app, _) = spelling_doc("the cat sat");
    assert_eq!(segments(&mut app), 0, "every word is known");
    let (mut app, id) = spelling_doc("the Qzvbtrw sat\nZvqtrbx");
    let m = marks_of(&mut app, &view());
    assert_eq!(m.spelling.len(), 2, "{:?}", m.spelling);
    let b = app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
    let (a, z) = m.spelling[0];
    assert!(a.x > b.x0 + 5.0 && z.x < b.x1 - 5.0 && a.y > b.y0 && a.y <= b.y1, "under the middle word: {a:?}..{z:?} in {b:?}");
    let w = wave(view().to_screen(a), view().to_screen(z));
    assert!(w.windows(2).any(|p| p[0].y != p[1].y), "wavy");
}

/// An edit, Ignore All and Add to Dictionary each change the segments of the next pass.
#[test]
fn auto_spell_check_follows_edits_ignore_all_and_the_dictionary() {
    let _dictionary = crate::testing::custom_dictionary();
    let (mut app, id) = spelling_doc("Wqzxvtr and Pzxqvrt");
    assert_eq!(segments(&mut app), 2);
    app.session.execute("text.setText", &json!({"id": id.0, "text": "Wqzxvtr and Pzxqvrt or Qxzvwtr"})).unwrap();
    assert_eq!(segments(&mut app), 3, "the edit is checked");
    app.session.execute("text.spelling.ignoreAll", &json!({"word": "wqzxvtr"})).unwrap();
    assert_eq!(segments(&mut app), 2, "Ignore All");
    app.session.execute("text.spelling.addWord", &json!({"word": "Pzxqvrt"})).unwrap();
    let after_add = segments(&mut app);
    app.session.execute("text.spelling.removeWord", &json!({"word": "pzxqvrt"})).unwrap();
    assert_eq!(after_add, 1, "Add to Dictionary");
    assert_eq!(segments(&mut app), 2, "deleted from the dictionary again");
}

/// Type with an effect shown, and type inside a hidden object, has no segments.
#[test]
fn auto_spell_check_leaves_out_type_with_effects_and_hidden_type() {
    let (mut app, id) = spelling_doc("Xqzvtrw");
    assert_eq!(segments(&mut app), 1);
    app.session.execute("effect.apply", &json!({"ids": [id.0], "effect": "distort.roughen", "params": {"size": 1}})).unwrap();
    assert_eq!(segments(&mut app), 0, "an effect");
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(segments(&mut app), 1, "undone");
    app.session.execute("object.hide", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(segments(&mut app), 0, "hidden");
}

/// Auto Spell Check is a toggle, off at first: off, there are no segments and no node is kept.
#[test]
fn auto_spell_check_is_a_toggle_of_the_canvas_lines() {
    let (mut app, _) = spelling_doc("Yqzvtrx");
    app.ui.view.auto_spell_check = false;
    assert_eq!(segments(&mut app), 0);
    assert_eq!(app.run("edit.autoSpellCheck", json!({})).unwrap(), json!(true));
    assert_eq!(segments(&mut app), 1, "on");
    assert_eq!(app.run("edit.autoSpellCheck", json!({})).unwrap(), json!(false));
    assert_eq!(segments(&mut app), 0, "off");
    assert!(app.canvas.spelling.lines.is_empty(), "no node is kept");
}
