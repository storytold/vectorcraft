use super::*;
use vectorcraft_engine::Session;

/// Applies `f` to the ruler's values for the current paragraph and sends the result through the
/// same path a ruler edit takes.
fn edit_ruler(app: &mut VectorcraftApp, f: impl FnOnce(&mut RulerValues)) {
    let (node, para, span) = current_para(app).unwrap();
    let before = ruler_values(&para, &para.tabs);
    let mut after = before.clone();
    f(&mut after);
    apply_ruler_edit(app, &node, &para, span, &before, &after);
}

/// Moves both indent markers by `by` points, as a drag of the lower marker with the box does: the
/// first line's indent is relative to the left one, so it goes along.
fn move_box(v: &mut RulerValues, by: f32) {
    v.left_indent += by;
}

/// Moves the upper (first-line) marker to `at` points, as its drag does: the first line's indent is
/// relative to the left one.
fn upper_to(v: &mut RulerValues, at: f32) {
    v.first_line_indent = at - v.left_indent;
}

/// Moves the lower (left) marker alone to `at` points, as its drag does: the first line stays where it is.
fn lower_to(v: &mut RulerValues, at: f32) {
    v.first_line_indent -= at - v.left_indent;
    v.left_indent = at;
}

/// A left-aligned stop with no leader at `position`, as a click on the ruler adds one.
fn stop_at(position: f32) -> RulerStop {
    RulerStop::new(position, RulerAlign::Left, TabLeader::None)
}

/// Types `value` in the X field, as the field does for the stop or marker picked on the ruler.
fn type_position(app: &mut VectorcraftApp, value: f64) {
    let (node, para, span) = current_para(app).unwrap();
    let picked = (app.ui.tabs_panel.stop, app.ui.tabs_panel.marker);
    set_position(app, &node, &para, span, picked, value);
}

/// Runs the panel menu command `id` (`ui.tabs.*`) on the app.
fn tab_command(app: &mut VectorcraftApp, id: &str) {
    app.run(id, json!({})).unwrap();
}

/// The stops' positions of the current text.
fn stop_positions(app: &VectorcraftApp) -> Vec<f64> {
    current(app).unwrap().1.iter().map(|s| s.position).collect()
}

/// A rectangle and two area texts, the first with a stop at 40 pt and the second with one at
/// 90 pt, nothing selected. → the app and the ids of the rectangle and the two texts.
fn mixed_selection_app() -> (VectorcraftApp, [u64; 3]) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 10, "y": 200, "width": 40, "height": 40})).unwrap());
    let a = id(app.run("text.create", json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 60}})).unwrap());
    let b = id(app.run("text.create", json!({"x": 10, "y": 100, "text": "c\td", "area": {"width": 250, "height": 60}})).unwrap());
    for (text, position) in [(a, 40), (b, 90)] {
        app.run("select.set", json!({"ids": [text]})).unwrap();
        app.run("text.tabs.set", json!({"stops": [{"position": position}]})).unwrap();
    }
    app.run("select.set", json!({"ids": []})).unwrap();
    (app, [rect, a, b])
}

/// With a rectangle selected first and a text after it, the panel shows the text's stops on its
/// ruler, not the "No text selected" state.
#[test]
fn the_ruler_shows_text_selected_after_an_object_that_is_not_text() {
    let (mut app, [rect, a, _]) = mixed_selection_app();
    app.run("select.set", json!({"ids": [rect, a]})).unwrap();
    let (node, stops, span) = current(&app).unwrap();
    assert_eq!(node.id.0, a);
    assert_eq!(stops.iter().map(|s| s.position).collect::<Vec<_>>(), [40.0]);
    assert_eq!(span, 300.0);
}

/// With a group selected first whose text follows a rectangle in it, the panel shows that text's
/// stops, ahead of a text selected after the group.
#[test]
fn the_ruler_shows_text_inside_a_selected_group() {
    let (mut app, [rect, a, b]) = mixed_selection_app();
    app.run("select.set", json!({"ids": [rect, b]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [group, a]})).unwrap();
    let (node, stops, span) = current(&app).unwrap();
    assert_eq!(node.id.0, b);
    assert_eq!(stops.iter().map(|s| s.position).collect::<Vec<_>>(), [90.0]);
    assert_eq!(span, 250.0);
}

/// A stop added on the ruler of a mixed selection sets the first text's stops plus the new one on
/// every selected text, and leaves the rectangle as it was.
#[test]
fn a_stop_added_on_a_mixed_selection_sets_every_selected_text() {
    let (mut app, [rect, a, b]) = mixed_selection_app();
    let shape = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(rect)).unwrap().clone();
    let before = shape(&app);
    app.run("select.set", json!({"ids": [rect, a, b]})).unwrap();
    edit_ruler(&mut app, |v| v.stops.push(stop_at(60.0)));
    for text in [a, b] {
        assert_eq!(text_of(&app, text).para_at(0).tabs.iter().map(|s| s.position).collect::<Vec<_>>(), [40.0, 60.0], "text {text}");
    }
    assert_eq!(shape(&app), before);
}

fn indent_app(left: f64, first: f64) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 20, "text": "one\ntwo", "area": {"width": 300, "height": 100}})).unwrap();
    app.run("text.setFormat", json!({"leftIndent": left, "firstLineIndent": first})).unwrap();
    app
}

fn indent_values(app: &VectorcraftApp, paragraph: usize) -> (f64, f64) {
    let (node, _, _) = current(app).unwrap();
    let NodeKind::Text(t) = node.kind else { panic!("selected text") };
    let p = t.para_at(paragraph);
    (p.left_indent, p.first_line_indent)
}

#[test]
fn upper_and_lower_marker_drags_map_indents_and_undo_once() {
    let mut app = indent_app(24.0, 12.0);
    let before = app.session.active().unwrap().history.undo.len();
    edit_ruler(&mut app, |v| upper_to(v, 60.0));
    assert_eq!(indent_values(&app, 0), (24.0, 36.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    let before = app.session.active().unwrap().history.undo.len();
    edit_ruler(&mut app, |v| lower_to(v, 40.0));
    assert_eq!(indent_values(&app, 0), (40.0, -4.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    let before = app.session.active().unwrap().history.undo.len();
    edit_ruler(&mut app, |v| move_box(v, 10.0));
    assert_eq!(indent_values(&app, 0), (50.0, -4.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
}

#[test]
fn mixed_indent_upper_marker_keeps_each_left_indent_in_one_undo_step() {
    let mut app = indent_app(24.0, 12.0);
    let id = current(&app).unwrap().0.id.0;
    app.run("text.setFormat", json!({"id": id, "start": 4, "end": 4, "leftIndent": 60, "firstLineIndent": -10})).unwrap();
    let before = app.session.active().unwrap().history.undo.len();
    edit_ruler(&mut app, |v| upper_to(v, 70.0));
    assert_eq!(indent_values(&app, 0), (24.0, 46.0));
    assert_eq!(indent_values(&app, 1), (60.0, 10.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    assert_eq!(indent_values(&app, 1), (60.0, -10.0));

    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let a = app.run("text.create", json!({"x": 10, "y": 20, "text": "one", "area": {"width": 300, "height": 100}})).unwrap()["id"].clone();
    let b = app.run("text.create", json!({"x": 10, "y": 140, "text": "two", "area": {"width": 300, "height": 100}})).unwrap()["id"].clone();
    app.run("text.setFormat", json!({"id": a, "leftIndent": 24, "firstLineIndent": 12})).unwrap();
    app.run("text.setFormat", json!({"id": b, "leftIndent": 60, "firstLineIndent": -10})).unwrap();
    app.run("select.set", json!({"ids": [a, b.clone()]})).unwrap();
    assert_eq!(current_para(&app).unwrap().1.left_indent, 24.0);
    edit_ruler(&mut app, |v| upper_to(v, 70.0));
    assert_eq!(indent_values(&app, 0), (24.0, 46.0));
    let second = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(b.as_u64().unwrap())).unwrap();
    let NodeKind::Text(second) = &second.kind else { panic!("second text") };
    assert_eq!((second.para_at(0).left_indent, second.para_at(0).first_line_indent), (60.0, 10.0));
}

#[test]
fn lower_marker_keeps_each_first_line_position_in_one_undo_step() {
    let mut app = indent_app(24.0, 12.0);
    let id = current(&app).unwrap().0.id.0;
    app.run("text.setFormat", json!({"id": id, "start": 4, "end": 7, "firstLineIndent": 28})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    assert_eq!(indent_values(&app, 1), (24.0, 28.0));
    let before = app.session.active().unwrap().history.undo.len();
    edit_ruler(&mut app, |v| lower_to(v, 40.0));
    assert_eq!(indent_values(&app, 0), (40.0, -4.0));
    assert_eq!(indent_values(&app, 1), (40.0, 12.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(indent_values(&app, 0), (24.0, 12.0));
    assert_eq!(indent_values(&app, 1), (24.0, 28.0));
    let before = app.session.active().unwrap().history.undo.len();
    edit_ruler(&mut app, |v| move_box(v, 16.0));
    assert_eq!(indent_values(&app, 0), (40.0, 12.0));
    assert_eq!(indent_values(&app, 1), (40.0, 28.0));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
}

/// After Select All in threaded text, a marker change keeps the left indent and first-line position
/// of each paragraph in every frame of the story, when the edited frame's paragraphs agree and
/// another frame's differ.
#[test]
fn indent_markers_after_select_all_keep_each_story_paragraphs_indents() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
    let story = (1..=12).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let a = app.run("text.create", json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 200, "height": 60}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    app.run("text.setFormat", json!({"ids": [a], "leftIndent": 24, "firstLineIndent": 12})).unwrap();
    let b = app.run("text.thread.add", json!({"from": a, "rect": [300, 10, 200, 250]})).unwrap()["id"].as_u64().unwrap();
    app.run("text.setFormat", json!({"ids": [b], "leftIndent": 60, "firstLineIndent": -10})).unwrap();
    let paragraphs = |app: &VectorcraftApp| -> Vec<(u64, f64, f64)> {
        let st = app.session.active().unwrap();
        [a, b]
            .into_iter()
            .flat_map(|id| match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                NodeKind::Text(t) => {
                    (0..t.paragraph_count()).map(|i| (id, t.para_at(i).left_indent, t.para_at(i).first_line_indent)).collect::<Vec<_>>()
                }
                _ => panic!("not text"),
            })
            .collect()
    };
    assert!(paragraphs(&app).iter().any(|p| p.1 == 24.0) && paragraphs(&app).iter().any(|p| p.1 == 60.0), "{:?}", paragraphs(&app));
    let select_all = |app: &mut VectorcraftApp| {
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
        app.run("select.all", json!({})).unwrap();
        assert_eq!(super::super::character::text_story(app), Some(vec![vectorcraft_doc::NodeId(a), vectorcraft_doc::NodeId(b)]));
    };
    select_all(&mut app);

    let before = paragraphs(&app);
    let (node, para, span) = current_para(&app).unwrap();
    assert_eq!(node.id.0, a);
    let targets = indent_targets(&app, &node);
    set_marker(&mut app, IndentMarker::Upper, 70.0, &para, span, &targets, false);
    let after = paragraphs(&app);
    assert_eq!(after.len(), before.len());
    for (was, now) in before.iter().zip(&after) {
        assert_eq!((now.1, now.1 + now.2), (was.1, 70.0), "a paragraph of frame {} keeps its left indent: {after:?}", now.0);
    }
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(paragraphs(&app), before);

    select_all(&mut app);
    let (node, para, span) = current_para(&app).unwrap();
    let targets = indent_targets(&app, &node);
    set_marker(&mut app, IndentMarker::Lower, 40.0, &para, span, &targets, false);
    let after = paragraphs(&app);
    assert_eq!(after.len(), before.len());
    for (was, now) in before.iter().zip(&after) {
        assert_eq!((now.1, now.1 + now.2), (40.0, was.1 + was.2), "a paragraph of frame {} keeps its first-line position: {after:?}", now.0);
    }
}

/// After Select All in threaded text with mixed indents, a marker change that makes the first
/// frame's paragraphs wrap onto more lines pushes text into the next frame while the change is
/// applied. Every paragraph of the story still gets the new position: the upper marker keeps each
/// left indent, and the lower marker keeps each first-line position.
#[test]
fn indent_markers_after_select_all_follow_text_that_reflows_into_the_next_frame() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 1000})).unwrap();
    let story = (1..=12).map(|i| format!("p{i} aaa bbb ccc")).collect::<Vec<_>>().join("\n");
    let a = app.run("text.create", json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 200, "height": 60}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let b = app.run("text.thread.add", json!({"from": a, "rect": [300, 10, 200, 950]})).unwrap()["id"].as_u64().unwrap();
    app.run("text.setFormat", json!({"ids": [b], "leftIndent": 10})).unwrap();
    // Each paragraph of the story by its first word, with its left indent and first-line position;
    // a paragraph continued from the frame before is counted once. No paragraph of the story is
    // empty, so an empty piece (after a frame's last line break) is not one.
    let paragraphs = |app: &VectorcraftApp| -> Vec<(String, f64, f64)> {
        let st = app.session.active().unwrap();
        [a, b]
            .into_iter()
            .flat_map(|id| match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                NodeKind::Text(t) => t
                    .plain_text()
                    .split('\n')
                    .enumerate()
                    .filter(|(i, text)| !text.is_empty() && (*i > 0 || !t.first_para_continues))
                    .map(|(i, text)| {
                        let p = t.para_at(i);
                        (text.split(' ').next().unwrap_or_default().to_string(), p.left_indent, p.left_indent + p.first_line_indent)
                    })
                    .collect::<Vec<_>>(),
                _ => panic!("not text"),
            })
            .collect()
    };
    let in_a = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(a)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text().len(),
        _ => panic!("not text"),
    };
    // Select All in the first frame and move `marker` to `to` → the story's paragraphs before and
    // after.
    let apply = |app: &mut VectorcraftApp, marker, to| {
        let before = paragraphs(app);
        assert_eq!(before.len(), 12, "{before:?}");
        let shown = in_a(app);
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
        app.run("select.all", json!({})).unwrap();
        let (node, para, span) = current_para(app).unwrap();
        let targets = indent_targets(app, &node);
        set_marker(app, marker, to, &para, span, &targets, false);
        let after = paragraphs(app);
        assert!(in_a(app) < shown, "the first frame holds less text once it wraps: {} of {shown} bytes, {after:?}", in_a(app));
        assert_eq!(after.iter().map(|p| &p.0).collect::<Vec<_>>(), before.iter().map(|p| &p.0).collect::<Vec<_>>());
        (before, after)
    };
    let (before, after) = apply(&mut app, IndentMarker::Upper, 150.0);
    assert!(before.iter().any(|p| p.1 == 0.0) && before.iter().any(|p| p.1 == 10.0), "{before:?}");
    for (was, now) in before.iter().zip(&after) {
        assert_eq!((now.1, now.2), (was.1, 150.0), "{} keeps its left indent and starts at 150 pt: {after:?}", now.0);
    }
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(paragraphs(&app), before);
    // First lines at 150 and 160 pt; a left indent of 180 pt leaves room for one word a line.
    app.run("text.setFormat", json!({"ids": [a, b], "firstLineIndent": 150})).unwrap();
    let (before, after) = apply(&mut app, IndentMarker::Lower, 180.0);
    assert!(before.iter().any(|p| p.2 == 150.0) && before.iter().any(|p| p.2 == 160.0), "{before:?}");
    for (was, now) in before.iter().zip(&after) {
        assert_eq!((now.1, now.2), (180.0, was.2), "{} is indented 180 pt and keeps its first-line position: {after:?}", now.0);
    }
}

/// A story of twelve paragraphs that wrap, threaded through three frames: the first two frames each
/// end inside a paragraph, which runs on into the next frame. → the app and the frames in thread
/// order.
fn split_story_app() -> (VectorcraftApp, [u64; 3]) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 900, "height": 1000})).unwrap();
    // Each paragraph takes more lines than a short frame holds.
    let words = "aaa bbb ccc ddd eee fff ggg hhh iii jjj ".repeat(3);
    let story = (1..=12).map(|i| format!("p{i} {}", words.trim_end())).collect::<Vec<_>>().join("\n");
    let a = app.run("text.create", json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 50}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let b = app.run("text.thread.add", json!({"from": a, "rect": [300, 10, 150, 50]})).unwrap()["id"].as_u64().unwrap();
    let c = app.run("text.thread.add", json!({"from": b, "rect": [600, 10, 250, 950]})).unwrap()["id"].as_u64().unwrap();
    for id in [b, c] {
        assert!(text_of(&app, id).first_para_continues, "frame {id} starts inside a paragraph: {:?}", text_of(&app, id).plain_text());
    }
    (app, [a, b, c])
}

fn text_of(app: &VectorcraftApp, id: u64) -> vectorcraft_doc::TextObject {
    match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

/// With the Selection tool, tab stops set in the panel go to every paragraph of every frame of a
/// selected threaded story, its locked and hidden frames included.
#[test]
fn selection_tool_tab_stops_reach_every_frame_of_the_story() {
    let (mut app, frames) = split_story_app();
    app.run("select.set", json!({"ids": [frames[1]]})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    app.run("select.set", json!({"ids": [frames[2]]})).unwrap();
    app.run("object.hide", json!({})).unwrap();
    app.run("select.set", json!({"ids": [frames[0]]})).unwrap();
    let stop = TabStop { position: 36.0, align: Default::default(), leader: String::new(), align_on: '.' };
    assert!(apply(&mut app, std::slice::from_ref(&stop)));
    for id in frames {
        let t = text_of(&app, id);
        assert!((0..t.paragraph_count()).all(|i| t.para_at(i).tabs == [stop.clone()]), "frame {id} takes the stop");
    }
}

/// With the Selection tool, Clear All Tabs is on when only another frame of the selected story has
/// tab stops, and clears them there.
#[test]
fn clear_all_tabs_is_on_when_another_frame_of_the_story_has_stops() {
    let (mut app, [a, b, c]) = split_story_app();
    app.run("text.tabs.set", json!({"ids": [c], "stops": [{"position": 36}]})).unwrap();
    app.run("select.set", json!({"ids": [c]})).unwrap();
    app.run("object.hide", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert!(current(&app).unwrap().1.is_empty(), "the selected frame has no stops");
    assert!(story_has_tabs(&app));
    super::super::paragraph::run_on_paragraphs(&mut app, "text.tabs.clear", json!({})).unwrap();
    for id in [a, b, c] {
        let t = text_of(&app, id);
        assert!((0..t.paragraph_count()).all(|i| t.para_at(i).tabs.is_empty()), "frame {id} has no stops");
    }
    assert!(!story_has_tabs(&app));
}

/// Each paragraph of the story of `frames` by its first word, with its left indent and first-line
/// position, counted once; each piece of a paragraph split between frames has the same values.
fn story_paragraphs(app: &VectorcraftApp, frames: &[u64]) -> Vec<(String, f64, f64)> {
    let mut out: Vec<(String, f64, f64)> = vec![];
    for (k, id) in frames.iter().enumerate() {
        let t = text_of(app, *id);
        for (i, text) in t.plain_text().split('\n').enumerate() {
            let p = t.para_at(i);
            let values = (p.left_indent, p.left_indent + p.first_line_indent);
            if i == 0 && k > 0 && t.first_para_continues {
                let last = out.last().unwrap();
                assert_eq!((last.1, last.2), values, "both pieces of {} agree", last.0);
            } else if !text.is_empty() {
                out.push((text.split(' ').next().unwrap_or_default().to_string(), values.0, values.1));
            }
        }
    }
    out
}

/// Where each target is in the story: its frame's place in `frames` and its start there.
fn story_positions(targets: &[IndentTarget], frames: &[u64]) -> Vec<(usize, usize)> {
    targets.iter().map(|p| (frames.iter().position(|f| *f == p.id).unwrap(), p.start)).collect()
}

/// With the Selection tool on every frame of a threaded story, selected last frame first, the
/// indent markers list each paragraph of the story once, in the story's order, and a marker drag
/// over mixed indents sets every paragraph in one undo step, the two split paragraphs included.
#[test]
fn indent_markers_on_selected_threaded_frames_follow_the_story_and_set_a_split_paragraph_once() {
    let (mut app, [a, b, c]) = split_story_app();
    app.run("text.setFormat", json!({"ids": [c], "leftIndent": 10})).unwrap();
    app.run("select.set", json!({"ids": [c, b, a]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect::<Vec<_>>(), vec![c, b, a]);
    let before = story_paragraphs(&app, &[a, b, c]);
    assert_eq!(before.len(), 12, "{before:?}");
    assert!(before.iter().any(|p| p.1 == 0.0) && before.iter().any(|p| p.1 == 10.0), "{before:?}");
    let (node, para, span) = current_para(&app).unwrap();
    let targets = indent_targets(&app, &node);
    let positions = story_positions(&targets, &[a, b, c]);
    assert_eq!(positions.len(), 12, "one target for each paragraph of the story: {positions:?}");
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "the targets follow the story: {positions:?}");
    let undo = app.session.active().unwrap().history.undo.len();
    set_marker(&mut app, IndentMarker::Upper, 60.0, &para, span, &targets, false);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    let after = story_paragraphs(&app, &[a, b, c]);
    assert_eq!(after.iter().map(|p| &p.0).collect::<Vec<_>>(), before.iter().map(|p| &p.0).collect::<Vec<_>>());
    for (was, now) in before.iter().zip(&after) {
        assert_eq!((now.1, now.2), (was.1, 60.0), "{} keeps its left indent and starts at 60 pt: {after:?}", now.0);
    }
}

/// A marker drag over many paragraphs of a threaded story with mixed indents sends one
/// `text.setFormat` for all of them, in one undo step, and a drag that changes no paragraph runs
/// nothing.
#[test]
fn indent_markers_over_many_threaded_paragraphs_send_one_format_command() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 900, "height": 3000})).unwrap();
    let story = (1..=128).map(|i| format!("p{i}")).collect::<Vec<_>>().join("\n");
    let a = app.run("text.create", json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 100}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let b = app.run("text.thread.add", json!({"from": a, "rect": [300, 10, 200, 2900]})).unwrap()["id"].as_u64().unwrap();
    app.run("text.setFormat", json!({"ids": [b], "leftIndent": 10})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let before = story_paragraphs(&app, &[a, b]);
    assert_eq!(before.len(), 128, "{before:?}");
    assert!(before.iter().any(|p| p.1 == 0.0) && before.iter().any(|p| p.1 == 10.0), "{before:?}");
    let steps = |app: &VectorcraftApp| (app.session.active().unwrap().history.undo.len(), app.session.journal.len());
    let drag = |app: &mut VectorcraftApp, marker, to| {
        let (node, para, span) = current_para(app).unwrap();
        let targets = indent_targets(app, &node);
        assert_eq!(targets.len(), 128);
        set_marker(app, marker, to, &para, span, &targets, false);
    };
    let (undo, journal) = steps(&app);
    drag(&mut app, IndentMarker::Upper, 60.0);
    assert_eq!(steps(&app), (undo + 1, journal + 1));
    let (name, params) = app.session.journal.last().cloned().unwrap();
    assert_eq!(name, "command.batch");
    let commands = params["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 1, "one command for every paragraph");
    assert_eq!(commands[0]["command"], "text.setFormat");
    assert_eq!(commands[0]["params"]["paragraphIndents"].as_array().unwrap().len(), 128);
    let after = story_paragraphs(&app, &[a, b]);
    for (was, now) in before.iter().zip(&after) {
        assert_eq!((&now.0, now.1, now.2), (&was.0, was.1, 60.0), "{after:?}");
    }
    // The same position again changes no paragraph.
    drag(&mut app, IndentMarker::Upper, 60.0);
    assert_eq!(steps(&app), (undo + 1, journal + 1), "nothing ran");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(story_paragraphs(&app, &[a, b]), before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(story_paragraphs(&app, &[a, b]), after);
}

/// After Select All in threaded text, a paragraph split between two frames is one target.
#[test]
fn indent_markers_after_select_all_list_a_split_paragraph_once() {
    let (mut app, [a, b, c]) = split_story_app();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    app.session.set_tool_option("textSelection", &json!({"editing": a, "anchor": 0, "caret": 0}));
    app.run("select.all", json!({})).unwrap();
    let ids = [a, b, c].map(vectorcraft_doc::NodeId);
    assert_eq!(super::super::character::text_story(&app), Some(ids.to_vec()));
    let (node, _, _) = current_para(&app).unwrap();
    let positions = story_positions(&indent_targets(&app, &node), &[a, b, c]);
    assert_eq!(positions.len(), 12, "{positions:?}");
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
}

/// With the Selection tool on any frame of a threaded story, the indent markers list every
/// paragraph of the story, each once in story order: a selected story is one text. Text in a
/// selected group is listed with the story's frames, and an empty text object keeps its one
/// paragraph.
#[test]
fn indent_markers_on_some_threaded_frames_list_the_whole_story_once() {
    let (mut app, [a, b, c]) = split_story_app();
    let count = |app: &VectorcraftApp, id: u64| text_of(app, id).paragraph_count();
    let targets_of = |app: &mut VectorcraftApp, ids: Vec<u64>| {
        app.run("select.set", json!({"ids": ids})).unwrap();
        let (node, _, _) = current_para(app).unwrap();
        story_positions(&indent_targets(app, &node), &[a, b, c])
    };
    let story = count(&app, a) + count(&app, b) + count(&app, c) - 2;
    let all = targets_of(&mut app, vec![c, a, b]);
    assert_eq!(all.len(), story, "two frames that share a paragraph list it once: {all:?}");
    assert!(all.windows(2).all(|w| w[0] < w[1]), "{all:?}");
    for frame in [a, b, c] {
        assert_eq!(targets_of(&mut app, vec![frame]), all, "frame {frame} alone lists the whole story");
    }
    // A grouped empty text object besides the last frame of the story.
    let empty = app.run("text.create", json!({"x": 10, "y": 900, "text": "", "area": {"width": 100, "height": 40}})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [empty, c]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    let node = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(c)).unwrap().clone();
    let targets = indent_targets(&app, &node);
    let ids: Vec<u64> = targets.iter().map(|p| p.id).collect();
    assert_eq!(ids.iter().filter(|id| **id == empty).count(), 1, "{ids:?}");
    let in_story: Vec<IndentTarget> = targets.into_iter().filter(|p| p.id != empty).collect();
    assert_eq!(story_positions(&in_story, &[a, b, c]), all);
}

/// A marker drag on one selected frame of a threaded story sets every paragraph of the story, in
/// its locked and hidden frames too, in one undo step, whether the paragraphs share their indents
/// or have their own.
#[test]
fn indent_markers_on_one_threaded_frame_set_its_locked_and_hidden_frames() {
    let (mut app, [a, b, c]) = split_story_app();
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    app.run("select.set", json!({"ids": [c]})).unwrap();
    app.run("object.hide", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    // Every paragraph has the same indents.
    let (node, para, span) = current_para(&app).unwrap();
    let targets = indent_targets(&app, &node);
    assert_eq!(targets.len(), 12);
    let undo = app.session.active().unwrap().history.undo.len();
    set_marker(&mut app, IndentMarker::Upper, 30.0, &para, span, &targets, false);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    let shared = story_paragraphs(&app, &[a, b, c]);
    assert_eq!(shared.len(), 12, "{shared:?}");
    assert!(shared.iter().all(|p| (p.1, p.2) == (0.0, 30.0)), "{shared:?}");
    // The paragraphs of the hidden frame have a left indent of their own.
    app.run("text.setFormat", json!({"ids": [c], "leftIndent": 10})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let before = story_paragraphs(&app, &[a, b, c]);
    assert_eq!(before.len(), 12, "{before:?}");
    assert!(before.iter().any(|p| p.1 == 0.0) && before.iter().any(|p| p.1 == 10.0), "{before:?}");
    let (node, para, span) = current_para(&app).unwrap();
    let targets = indent_targets(&app, &node);
    assert_eq!(targets.len(), 12);
    let undo = app.session.active().unwrap().history.undo.len();
    set_marker(&mut app, IndentMarker::Upper, 60.0, &para, span, &targets, false);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    let after = story_paragraphs(&app, &[a, b, c]);
    assert_eq!(after.iter().map(|p| &p.0).collect::<Vec<_>>(), before.iter().map(|p| &p.0).collect::<Vec<_>>());
    for (was, now) in before.iter().zip(&after) {
        assert_eq!((now.1, now.2), (was.1, 60.0), "{} keeps its left indent and starts at 60 pt: {after:?}", now.0);
    }
}

#[test]
fn lower_marker_bounds_cover_each_touched_first_line() {
    let targets = [
        IndentTarget { id: 1, start: 0, left: 24.0, first_line: -20.0, level: 0.0 },
        IndentTarget { id: 1, start: 4, left: 24.0, first_line: 280.0, level: 0.0 },
    ];
    assert_eq!(bounded_marker_position(IndentMarker::Lower, 40.0, 300.0, &targets, true), Some(20.0));
    assert_eq!(bounded_marker_position(IndentMarker::Lower, 1200.0, 300.0, &targets, false), Some(300.0));
    // A paragraph at list level 2 is laid out 18 pt further in: its own left indent stays at 0 or more.
    let listed = [IndentTarget { id: 1, start: 0, left: 18.0, first_line: 0.0, level: 18.0 }];
    assert_eq!(bounded_marker_position(IndentMarker::Lower, 5.0, 300.0, &listed, true), Some(18.0));
}

/// A list paragraph at level 2 is laid out 18 pt further in than its left indent: the ruler shows
/// its markers there, and a marker moved on the ruler sets the paragraph's own left indent.
#[test]
fn indent_markers_show_and_set_a_list_paragraphs_laid_out_indent() {
    let mut app = indent_app(10.0, 6.0);
    app.run("text.setFormat", json!({"list": "bullet", "listLevel": 2})).unwrap();
    let para = current_para(&app).unwrap().1;
    assert_eq!((marker_position(IndentMarker::Lower, &para), marker_position(IndentMarker::Upper, &para)), (28.0, 34.0));
    edit_ruler(&mut app, |v| move_box(v, 12.0));
    assert_eq!(indent_values(&app, 0), (22.0, 6.0), "the ruler's 40 pt is the paragraph's 22 pt and the level's 18");
    edit_ruler(&mut app, |v| upper_to(v, 70.0));
    assert_eq!(indent_values(&app, 0), (22.0, 30.0));
    edit_ruler(&mut app, |v| lower_to(v, 20.0));
    assert_eq!(indent_values(&app, 0), (2.0, 50.0), "the first line stays at 70 pt");
}

/// The X field of a selected indent marker sets the marker's absolute position: the first-line
/// marker moves the first line alone, the left marker keeps the first line where it is.
#[test]
fn selected_marker_position_field_sets_its_absolute_position() {
    let mut app = indent_app(24.0, 12.0);
    app.ui.tabs_panel.marker = Some(RulerItem::FirstLine);
    type_position(&mut app, 80.0);
    assert_eq!(indent_values(&app, 0), (24.0, 56.0));
    app.ui.tabs_panel.marker = Some(RulerItem::Hanging);
    type_position(&mut app, 40.0);
    assert_eq!(indent_values(&app, 0), (40.0, 40.0));
}

/// A ruler edit that changes no value sends nothing: no undo step, and the selection stays.
#[test]
fn a_drag_that_moves_no_stop_records_no_undo_step() {
    let mut app = guide_app(&[50.0]);
    let id = current(&app).unwrap().0.id;
    let undo = app.session.active().unwrap().history.undo.len();
    edit_ruler(&mut app, |_| {});
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.len(), undo);
    assert_eq!(st.selection.objects, vec![id]);
}

/// Area type at (10, 20), 300 by 100 pt, with stops at `stops` points, and the app.
fn guide_app(stops: &[f64]) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 20, "text": "a\tb\tc", "area": {"width": 300, "height": 100}})).unwrap();
    let stops: Vec<_> = stops.iter().map(|p| json!({"position": p})).collect();
    app.run("text.tabs.set", json!({"stops": stops})).unwrap();
    app
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 0.01
}

/// The guide for a stop at `position` on the ruler of the text `node`'s first paragraph, in
/// document space.
fn guide_of(node: &Node, position: f64) -> Option<(Point, Point)> {
    let NodeKind::Text(t) = &node.kind else { panic!("text") };
    // PLAN: assumed guide_axis(&TextObject, usize) -> Option<GuideAxis> and guide_at(GuideAxis, f64) -> (Point, Point) (type-w17)
    Some(guide_at(guide_axis(t, 0)?, position))
}

/// The guide for a stop crosses the selected area type at the place the stop would land, measured
/// from the frame's left edge, from the frame's top to its bottom.
#[test]
fn dragging_a_stop_draws_a_guide_through_the_text() {
    let app = guide_app(&[50.0]);
    let node = current(&app).unwrap().0;
    let frame = node.geometric_bounds().unwrap();
    let (a, b) = guide_of(&node, 70.0).expect("a guide for area type");
    assert!(near(a, Point::new(frame.x0 + 70.0, frame.y0)) && near(b, Point::new(frame.x0 + 70.0, frame.y1)), "{a:?} {b:?} in {frame:?}");
}

/// When a text wrap object pushes the paragraph's lines right, the guide is measured from where the
/// first line starts, as the layout measures the stops, and a first-line indent does not move it.
#[test]
fn the_stop_guide_starts_where_a_wrap_object_lets_the_line_start() {
    let mut app = guide_app(&[70.0]);
    let text = current(&app).unwrap().0.id.0;
    app.run("text.setFormat", json!({"firstLineIndent": 12})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 60, "height": 200})).unwrap();
    app.run("object.textWrap.make", json!({"offset": 0})).unwrap();
    app.run("select.set", json!({"ids": [text]})).unwrap();
    let node = current(&app).unwrap().0;
    let (a, b) = guide_of(&node, 70.0).unwrap();
    assert!((a.x - 130.0).abs() < 0.01 && (b.x - 130.0).abs() < 0.01, "70 pt right of the wrap object's edge at 60: {a:?} {b:?}");
    // The tab after "a" ends at the stop: "b" starts 70 pt right of the wrap object's edge.
    let NodeKind::Text(t) = &node.kind else { panic!("text") };
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let b_at = lay.glyphs.iter().find(|g| t.plain_text().get(g.byte..).is_some_and(|s| s.starts_with('b'))).unwrap().origin.x;
    assert!(((t.xf * Point::new(b_at, 0.0)).x - 130.0).abs() < 0.01, "b at {b_at}");
}

/// The guide for each stop of a ruler drag is at that stop's position from the frame's left edge.
#[test]
fn a_command_drag_guide_follows_the_grabbed_stop() {
    let app = guide_app(&[30.0, 50.0, 80.0]);
    let node = current(&app).unwrap().0;
    let x0 = node.geometric_bounds().unwrap().x0;
    for position in [30.0, 50.0, 80.0] {
        let (a, _) = guide_of(&node, position).unwrap();
        assert!((a.x - (x0 + position)).abs() < 0.01, "{a:?} for the stop at {position}");
    }
}

/// The guide turns with rotated area type, and runs through point type from the type's origin
/// across its lines; in vertical type it runs across the columns.
#[test]
fn the_stop_guide_follows_rotated_point_and_vertical_type() {
    // Rotated area type: the guide is the frame's edge moved 70 pt along the frame's top.
    let mut app = guide_app(&[50.0]);
    app.run("object.rotate", json!({"angle": 30})).unwrap();
    let node = current(&app).unwrap().0;
    let NodeKind::Text(t) = &node.kind else { panic!("text") };
    let TextKind::Area { frame } = &t.kind else { panic!("area type") };
    let local = frame.bounds().unwrap();
    let (a, b) = guide_of(&node, 70.0).unwrap();
    let (ea, eb) = (t.xf * Point::new(local.x0 + 70.0, local.y0), t.xf * Point::new(local.x0 + 70.0, local.y1));
    assert!(near(a, ea) && near(b, eb), "{a:?} {b:?}, expected {ea:?} {eb:?}");
    assert!((a.x - b.x).abs() > 10.0, "not upright: {a:?} {b:?}");
    // Point type with its origin at (10, 20): the guide is upright at 10 + 70 and crosses the baseline.
    for vertical in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({})).unwrap();
        app.run("text.create", json!({"x": 10, "y": 20, "text": "a\tb", "vertical": vertical})).unwrap();
        app.run("text.tabs.set", json!({"stops": [{"position": 50}]})).unwrap();
        let node = current(&app).unwrap().0;
        let (a, b) = guide_of(&node, 70.0).unwrap_or_else(|| panic!("a guide, vertical {vertical}"));
        let NodeKind::Text(t) = &node.kind else { panic!("text") };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let at = t.xf * lay.physical_point(Point::new(70.0, 0.0));
        if vertical {
            assert!((a.y - at.y).abs() < 0.01 && (b.y - at.y).abs() < 0.01, "across the column: {a:?} {b:?}, {at:?}");
            assert!((a.x - b.x).abs() > 1.0, "{a:?} {b:?}");
        } else {
            assert!((a.x - 80.0).abs() < 0.01 && (b.x - 80.0).abs() < 0.01, "{a:?} {b:?}");
            assert!(a.y < 19.0 && b.y > 20.0, "across the baseline: {a:?} {b:?}");
        }
    }
}

/// An X typed past the next stop keeps the stop selected once the stops are sorted. An X the
/// stops cannot take (past 100,000 pt) leaves the stops and the selection as they were.
#[test]
fn an_x_typed_past_the_next_stop_keeps_the_stop_selected() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50}]})).unwrap();
    app.ui.tabs_panel.stop = 0;
    type_position(&mut app, 70.0);
    assert_eq!(stop_positions(&app), [50.0, 70.0]);
    assert_eq!(app.ui.tabs_panel.stop, 1);
    app.ui.tabs_panel.stop = 0;
    type_position(&mut app, 200000.0);
    assert_eq!(stop_positions(&app), [50.0, 70.0]);
    assert_eq!(app.ui.tabs_panel.stop, 0);
    // An X less than 1e-6 pt above another stop merges the two into the stop already there,
    // which is selected.
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50}, {"position": 70}]})).unwrap();
    app.ui.tabs_panel.stop = 0;
    type_position(&mut app, 50.0000001);
    assert_eq!(stop_positions(&app), [50.0, 70.0]);
    assert_eq!(app.ui.tabs_panel.stop, 0);
}

/// The undo steps of the active document.
fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// A position typed in the X field of a paragraph with no stops adds a stop there with the chosen
/// alignment, keeps its hundredths of a point, picks it, and is one undo step.
#[test]
fn typing_x_into_a_paragraph_without_stops_adds_and_selects_a_stop() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
    assert!(current(&app).unwrap().1.is_empty());
    app.ui.tabs_panel.align = TabAlign::Right;
    app.ui.tabs_panel.stop = 3;
    let before = undo_steps(&app);
    type_position(&mut app, 36.47);
    let stops = current(&app).unwrap().1;
    assert_eq!(stops.iter().map(|s| (s.position, s.align)).collect::<Vec<_>>(), [(36.47, TabAlign::Right)]);
    assert_eq!(app.ui.tabs_panel.stop, 0);
    assert_eq!(undo_steps(&app), before + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert!(current(&app).unwrap().1.is_empty());
}

/// In vertical type the position field is Y, and a position typed there for a paragraph with no
/// stops adds a stop.
#[test]
fn typing_y_into_vertical_type_without_stops_adds_a_stop() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 20, "text": "a\tb", "vertical": true})).unwrap();
    type_position(&mut app, 48.0);
    let stops = current(&app).unwrap().1;
    assert_eq!(stops.iter().map(|s| (s.position, s.align)).collect::<Vec<_>>(), [(48.0, TabAlign::Left)]);
}

/// A first stop past 100,000 pt adds no stop and records no undo step. A negative position adds
/// the stop at 0 pt.
#[test]
fn a_refused_first_stop_adds_nothing_and_records_no_undo() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
    let before = undo_steps(&app);
    type_position(&mut app, 200000.0);
    assert!(current(&app).unwrap().1.is_empty());
    assert_eq!(undo_steps(&app), before);
    type_position(&mut app, -5.0);
    assert_eq!(stop_positions(&app), [0.0]);
    assert_eq!(undo_steps(&app), before + 1);
}

/// With the Selection tool on two texts, a first stop typed for the shown text, which has no
/// stops, becomes every selected text's stops in one undo step, replacing the other text's stop.
#[test]
fn typing_the_first_stop_over_mixed_paragraphs_uses_the_displayed_list() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let a = id(app.run("text.create", json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 60}})).unwrap());
    let b = id(app.run("text.create", json!({"x": 10, "y": 100, "text": "c\td", "area": {"width": 250, "height": 60}})).unwrap());
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("text.tabs.set", json!({"stops": [{"position": 90}]})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert_eq!(current(&app).unwrap().0.id.0, a);
    let before = undo_steps(&app);
    type_position(&mut app, 25.0);
    for text in [a, b] {
        assert_eq!(text_of(&app, text).para_at(0).tabs.iter().map(|s| s.position).collect::<Vec<_>>(), [25.0], "text {text}");
    }
    assert_eq!(undo_steps(&app), before + 1);
}

/// The panel menu's Repeat Tab, Delete Tab and Clear All Tabs can be chosen while the text has
/// tab stops and show no check mark. Repeat Tab and Delete Tab act on the stop selected on the
/// ruler, and Clear All Tabs removes every stop.
#[test]
fn panel_menu_repeats_deletes_and_clears_tab_stops() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50, "leader": "."}]})).unwrap();
    // Repeat Tab with the second stop selected: 20 copies of it, 20 pt apart (its distance from
    // the first stop), leader and all.
    app.ui.tabs_panel.stop = 1;
    tab_command(&mut app, "ui.tabs.repeatTab");
    let repeated = stop_positions(&app);
    assert_eq!((repeated.len(), &repeated[..4]), (22, &[30.0, 50.0, 70.0, 90.0][..]));
    assert!(current(&app).unwrap().1[2..].iter().all(|s| s.leader == "."));
    // Delete Tab removes the selected stop.
    tab_command(&mut app, "ui.tabs.deleteTab");
    assert_eq!(stop_positions(&app)[..3], [30.0, 70.0, 90.0]);
    // Clear All Tabs removes every stop, in one undo step of its own name.
    tab_command(&mut app, "ui.tabs.clearAll");
    assert!(stop_positions(&app).is_empty());
    assert_eq!(app.session.active().unwrap().history.undo.last().unwrap().label, "Clear All Tabs");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(stop_positions(&app).len(), 21);
}

#[test]
fn panel_menu_does_not_change_stops_while_an_indent_marker_is_selected() {
    let mut app = indent_app(24.0, 12.0);
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}, {"position": 70}]})).unwrap();
    app.ui.tabs_panel.stop = 1;
    app.ui.tabs_panel.marker = Some(RulerItem::FirstLine);
    let before = current(&app).unwrap().1;
    assert!(!can(&app, REPEAT_TAB) && !can(&app, DELETE_TAB), "Repeat Tab and Delete Tab are off");
    // A disabled command refuses; the stops stay as they were either way.
    let _ = app.run("ui.tabs.repeatTab", json!({}));
    assert_eq!(current(&app).unwrap().1, before);
    let _ = app.run("ui.tabs.deleteTab", json!({}));
    assert_eq!(current(&app).unwrap().1, before);
}

/// With 256 stops, a click on an empty part of the ruler adds none and shows the limit in the
/// status bar. Repeat Tab near the end of the list adds copies only up to 256 stops and shows the
/// limit too.
#[test]
fn the_ruler_and_repeat_tab_stop_at_256_stops() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
    // 250 stops 1 pt apart from 0 pt, then six 2 pt apart from 260 pt.
    let start: Vec<f64> = (0..250).map(f64::from).chain((0..6).map(|i| 260.0 + 2.0 * f64::from(i))).collect();
    let stops: Vec<Value> = start.iter().map(|p| json!({"position": p})).collect();
    app.session.execute("text.tabs.set", &json!({ "stops": stops })).unwrap();
    app.ui.tabs_panel.stop = 3;
    edit_ruler(&mut app, |v| v.stops.push(stop_at(290.0)));
    assert_eq!(stop_positions(&app), start);
    assert_eq!(app.ui.tabs_panel.stop, 3);
    assert_eq!(app.ui.status, "at most 256 tab stops");
    // Repeat Tab on the stop at 260 pt (11 pt past the one before it): five copies, not 20.
    app.status("");
    app.ui.tabs_panel.stop = 250;
    tab_command(&mut app, "ui.tabs.repeatTab");
    let repeated = stop_positions(&app);
    assert_eq!(repeated.len(), 256);
    assert_eq!(repeated[249..], [249.0, 260.0, 271.0, 282.0, 293.0, 304.0, 315.0]);
    assert_eq!(app.ui.status, "at most 256 tab stops");
}

/// Repeat Tab on the first stop spaces the copies by the stop's distance from the paragraph's
/// left indent (18 pt further in for each list level past the first), from the paragraph the
/// caret is in while the Type tool edits text.
#[test]
fn repeat_tab_measures_the_first_stop_from_the_left_indent() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a\tb\nc\td", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.setFormat", &json!({"leftIndent": 20})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    // 30 pt from the indent at 20: copies at 80, 110, ...
    app.ui.tabs_panel.stop = 0;
    tab_command(&mut app, "ui.tabs.repeatTab");
    let repeated = stop_positions(&app);
    assert_eq!((repeated.len(), &repeated[..4]), (21, &[50.0, 80.0, 110.0, 140.0][..]));
    // A stop left of the indent: 15 pt from the indent at 20.
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 5}]})).unwrap();
    tab_command(&mut app, "ui.tabs.repeatTab");
    assert_eq!(stop_positions(&app)[..4], [5.0, 20.0, 35.0, 50.0]);
    // A list paragraph at level 2 is set 18 pt further in, at 38: copies every 12 pt.
    app.session.execute("text.setFormat", &json!({"list": "bullet", "listLevel": 2})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}]})).unwrap();
    tab_command(&mut app, "ui.tabs.repeatTab");
    assert_eq!(stop_positions(&app)[..3], [50.0, 62.0, 74.0]);
    // The Type tool's caret in the second paragraph, indented 40 pt: its stop at 50 repeats
    // every 10 pt, and the first paragraph keeps its stop.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 20, "text": "a\tb\nc\td"})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.run("text.setFormat", json!({"id": id, "leftIndent": 20})).unwrap();
    app.run("text.setFormat", json!({"id": id, "start": 4, "end": 7, "leftIndent": 40})).unwrap();
    app.run("text.tabs.set", json!({"id": id, "stops": [{"position": 50}]})).unwrap();
    app.session.set_tool_option("select", &json!({"start": 5, "end": 5}));
    tab_command(&mut app, "ui.tabs.repeatTab");
    assert_eq!(stop_positions(&app)[..3], [50.0, 60.0, 70.0]);
    app.session.set_tool_option("select", &json!({"start": 0, "end": 0}));
    assert_eq!(stop_positions(&app), [50.0]);
}

/// After Delete Tab removes the last stop, the ruler shows the stop before it selected, and
/// Delete Tab removes that one next.
#[test]
fn delete_tab_after_the_last_stop_deletes_the_stop_the_ruler_shows_selected() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
    app.session.execute("text.tabs.set", &json!({"stops": [{"position": 30}, {"position": 50}]})).unwrap();
    app.ui.tabs_panel.stop = 1;
    tab_command(&mut app, "ui.tabs.deleteTab");
    assert_eq!(current(&app).unwrap().1.len(), 1);
    tab_command(&mut app, "ui.tabs.deleteTab");
    assert!(current(&app).unwrap().1.is_empty());
}

/// The panel's rect while it floats.
fn tabs_rect(app: &VectorcraftApp) -> egui::Rect {
    app.ui.dock_workspace.layout().floating.iter().find(|w| w.contains(&"tabs".to_string())).unwrap().rect
}

/// Sets the canvas to a 1000 by 700 pt area starting `top` points down the window, at `zoom`.
fn show_canvas(app: &mut VectorcraftApp, top: f32, zoom: f64) {
    if let Some(v) = app.view_mut() {
        v.rect = egui::Rect::from_min_size(egui::pos2(50.0, top), egui::vec2(1000.0, 700.0));
        v.zoom = zoom;
    }
}

/// Position Panel Above Text on text narrower than a floating panel can be at the zoom: the panel
/// comes out wider than asked, and its ruler draws the text at the zoom, running on past the text,
/// so its marks stay over the text. A ruler wider than the narrowest panel, for other text or in
/// another document fits the span to its width as before.
#[test]
fn above_text_keeps_the_zoom_on_a_ruler_wider_than_asked() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    app.session.execute("text.create", &json!({"x": 40, "y": 200, "text": "a\tb", "area": {"width": 100, "height": 100}})).unwrap();
    show_canvas(&mut app, 40.0, 0.5);
    let (node, _, span) = current(&app).unwrap();
    place_above_text(&mut app, &node, span);
    // The panel asked for: a 50 pt ruler, less than the narrowest panel's.
    let asked = tabs_rect(&app).width();
    assert!((asked - RULER_LEFT * 2.0 - 50.0).abs() < 0.01, "{asked}");
    // Screen points per point of text on a ruler `width` wide, for the selected text.
    let scale = |app: &VectorcraftApp, width: f32| {
        let (node, _, span) = current(app).unwrap();
        // PLAN: assumed ruler_scale(&VectorcraftApp, &Node, f64, f32) -> f64 (type-w17 note 8)
        ruler_scale(app, &node, span, width)
    };
    let narrowest = crate::panel_docking::PANEL_MIN_WIDTH;
    let fitted = (f64::from(narrowest) - 8.0) / span;
    let at_zoom = scale(&app, narrowest);
    assert!((at_zoom - 0.5).abs() < 1e-3, "{at_zoom}");
    let other =
        app.session.execute("text.create", &json!({"x": 300, "y": 200, "text": "c", "area": {"width": 100, "height": 100}})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [other]})).unwrap();
    assert!((scale(&app, narrowest) - fitted).abs() < 1e-3, "other text fits its span to the ruler");
    app.session.execute("select.set", &json!({"ids": [node.id.0]})).unwrap();
    assert!((scale(&app, narrowest) - 0.5).abs() < 1e-3);
    // At another zoom the ruler follows it.
    if let Some(v) = app.view_mut() {
        v.zoom = 2.0;
    }
    assert!((scale(&app, narrowest) - 2.0).abs() < 1e-3, "another zoom");
    if let Some(v) = app.view_mut() {
        v.zoom = 0.5;
    }
    app.session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    app.session.execute("text.create", &json!({"x": 40, "y": 200, "text": "d", "area": {"width": 100, "height": 100}})).unwrap();
    if let Some(v) = app.view_mut() {
        v.zoom = 0.5;
    }
    assert!((scale(&app, narrowest) - fitted).abs() < 1e-3, "another document");
    assert!(app.session.set_active(0));
    assert!((scale(&app, narrowest) - 0.5).abs() < 1e-3);
    // The panel made wider than the narrowest floating window fits the span to its ruler again.
    let at = tabs_rect(&app).min;
    crate::panel_docking::place_floating(&mut app, "tabs", at, 400.0).unwrap();
    let wide = scale(&app, tabs_rect(&app).width());
    assert!((wide - (400.0 - 8.0) / span).abs() < 1e-3, "{wide}");
}

/// Position Panel Above Text floats the panel over the text, its ruler's 0 on the frame's left
/// edge and a stop's mark over the column it sets, at any zoom (#729).
#[test]
fn above_text_puts_the_ruler_over_the_text_at_the_zoom() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    app.session.execute("text.create", &json!({"x": 40, "y": 200, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
    for zoom in [1.0, 2.0] {
        // The canvas starts low enough in the window for the panel to fit above the text at 200%.
        show_canvas(&mut app, 440.0, zoom);
        let (node, _, span) = current(&app).unwrap();
        above_text(&mut app, &node, span).unwrap();
        let g = tabs_rect(&app);
        let frame = egui::Rect::from_points(&app.canvas_view().unwrap().quad(node.geometric_bounds().unwrap()));
        assert!((g.min.x + RULER_LEFT - frame.left()).abs() < 0.01, "0 on the frame's left edge");
        assert!(g.min.y < frame.top(), "above the text");
        // The ruler (the panel less its margins) spans the frame at this zoom.
        let ruler = g.width() - RULER_LEFT * 2.0;
        assert!((ruler - frame.width()).abs() < 0.01, "{ruler} {}", frame.width());
        assert!((ruler - 300.0 * zoom as f32).abs() < 0.01);
    }
    let windows = app.ui.dock_workspace.layout().floating.iter().filter(|w| w.contains(&"tabs".to_string())).count();
    assert_eq!(windows, 1, "placed again, not floated twice");
}

/// Text near the top of the window: Position Panel Above Text puts the panel just below the text,
/// so the panel does not go above the screen and the text stays in view. Text with no room below
/// it either gets the panel at the window's top.
#[test]
fn above_text_keeps_the_panel_bar_in_the_window() {
    for (text_height, below) in [(100.0, true), (760.0, false)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        app.session.execute("text.create", &json!({"x": 40, "y": 10, "text": "a\tb", "area": {"width": 300, "height": text_height}})).unwrap();
        show_canvas(&mut app, 40.0, 1.0);
        let (node, _, span) = current(&app).unwrap();
        let frame = egui::Rect::from_points(&app.canvas_view().unwrap().quad(node.geometric_bounds().unwrap()));
        assert!(frame.top() < 240.0, "no room above the text");
        above_text(&mut app, &node, span).unwrap();
        let top = if below { frame.bottom() + 6.0 } else { 0.0 };
        assert_eq!(tabs_rect(&app).min.y, top, "text {text_height} high");
    }
}

/// Position Panel Above Text puts the panel at the window's top when the place below the text is
/// above the window (text panned above the canvas), or when the panel at the height it floats at
/// does not fit below the text in the canvas.
#[test]
fn above_text_puts_the_panel_at_the_top_when_below_the_text_does_not_fit() {
    // The view's centre and the text's height: panned so the text is above the canvas, and a text
    // whose bottom leaves the canvas 14 points less room than the height the panel floats at (the
    // text's top is at 60 on screen, the canvas's bottom at 740 and the panel 6 below the text).
    let floats = crate::panel_docking::floating_height(&VectorcraftApp::new(Session::new(), Default::default()), "tabs");
    for (center_y, text_height) in [(600.0, 100.0), (350.0, f64::from(694.0 - floats))] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 600, "height": 800})).unwrap();
        app.session.execute("text.create", &json!({"x": 40, "y": 20, "text": "a\tb", "area": {"width": 300, "height": text_height}})).unwrap();
        show_canvas(&mut app, 40.0, 1.0);
        if let Some(v) = app.view_mut() {
            v.center = vectorcraft_geom::Point::new(300.0, center_y);
        }
        let (node, _, span) = current(&app).unwrap();
        above_text(&mut app, &node, span).unwrap();
        assert_eq!(tabs_rect(&app).min.y, 0.0, "view centered at y {center_y}");
    }
}
