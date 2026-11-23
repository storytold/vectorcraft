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
#[test]
fn hidden_characters_lay_out_only_type_that_changed() {
    let mut app = app();
    let ids = [text(&mut app, 20, 40, "a b\nc d"), text(&mut app, 20, 160, "e f")];
    app.ui.view.hidden_chars = true;
    let before = settled(&mut app, &view(), &ids);
    app.session.execute("text.setText", &json!({"id": ids[1].0, "text": "e f g"})).unwrap();
    marks_of(&mut app, &view());
    let after = kept_hidden(&app, &ids);
    assert!(Arc::ptr_eq(&before[0], &after[0]), "the object not edited is not laid out again");
    assert!(!Arc::ptr_eq(&before[1], &after[1]), "the edited object is");
    let zoomed = CanvasView::new(view().rect, Point::new(100.0, 100.0), 2.0);
    marks_of(&mut app, &zoomed);
    assert!(kept_hidden(&app, &ids).iter().zip(&after).all(|(a, b)| Arc::ptr_eq(a, b)), "a zoom lays out no type");
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
    app.ui.view.hidden_chars = false;
    marks_of(&mut app, &view());
    assert!(app.canvas.hidden_chars.marks.is_empty() && count(&app) < held, "off: no node is kept");
    app.ui.view.hidden_chars = true;
    marks_of(&mut app, &view());
    assert_eq!(app.canvas.hidden_chars.marks.len(), 1);
    app.session.execute("file.close", &json!({})).unwrap();
    marks_of(&mut app, &view());
    assert!(app.canvas.hidden_chars.marks.is_empty(), "no node of the closed document is kept");
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
