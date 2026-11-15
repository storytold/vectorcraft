use super::*;

/// The document and the number of undo steps, to compare before and after a command that sets
/// what the type already has.
fn state(s: &Session) -> (std::sync::Arc<Document>, usize) {
    let st = s.doc().unwrap();
    (st.doc.clone(), st.history.undo.len())
}

/// Assert that `cmd` with `p` leaves the document and its undo history as they were.
fn records_nothing(s: &mut Session, cmd: &str, p: Value) {
    let before = state(s);
    s.execute(cmd, &p).unwrap();
    let after = state(s);
    assert!(*after.0 == *before.0, "{cmd} {p} changed the document");
    assert_eq!(after.1, before.1, "{cmd} {p} recorded an undo step");
}

/// Assert that `cmd` with `p` changes the document in exactly one undo step.
fn records_one_step(s: &mut Session, cmd: &str, p: Value) {
    let before = state(s);
    s.execute(cmd, &p).unwrap();
    let after = state(s);
    assert!(*after.0 != *before.0, "{cmd} {p} left the document as it was");
    assert_eq!(after.1, before.1 + 1, "{cmd} {p} did not record one undo step");
}

fn session_with(text: &str, area: Option<Value>) -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let mut p = json!({"x": 10, "y": 20, "text": text, "size": 24});
    if let Some(a) = area {
        p["area"] = a;
    }
    let id = s.execute("text.create", &p).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    (s, id)
}

/// Ruby set again with the same reading and options, or removed where there is none, records no
/// undo step; a step that does change the ruby still undoes in one.
#[test]
fn ruby_that_changes_nothing_records_no_undo_step() {
    let (mut s, id) = session_with("漢字です", None);
    records_nothing(&mut s, "type.ruby", json!({"remove": true}));
    let p = json!({"text": "かんじ", "start": 0, "end": 6, "position": "below", "size": 60});
    records_one_step(&mut s, "type.ruby", p.clone());
    records_nothing(&mut s, "type.ruby", p);
    s.execute("type.ruby", &json!({"remove": true})).unwrap();
    records_nothing(&mut s, "type.ruby", json!({"remove": true, "start": 0, "end": 6}));
    s.execute("edit.undo", &json!({})).unwrap();
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!("text") };
    assert_eq!(t.ruby.len(), 1, "undo brings the ruby back");
}

/// `text.setStyle` (the Character panel) given the attributes the characters already have
/// records no undo step, with a range too.
#[test]
fn character_attributes_already_set_record_no_undo_step() {
    let (mut s, _) = session_with("Headline", None);
    records_nothing(&mut s, "text.setStyle", json!({"size": 24}));
    s.execute("text.setStyle", &json!({"tracking": 50, "fill": "#ff0000"})).unwrap();
    records_nothing(&mut s, "text.setStyle", json!({"tracking": 50, "fill": "#ff0000"}));
    records_nothing(&mut s, "text.setStyle", json!({"tracking": 50, "start": 0, "end": 4}));
    s.execute("text.setStyle", &json!({"justify": "center"})).unwrap();
    records_nothing(&mut s, "text.setStyle", json!({"justify": "center"}));
    records_one_step(&mut s, "text.setStyle", json!({"tracking": 80, "start": 0, "end": 4}));
}

/// `text.setStyle` on a graph label that leaves the label's own text as it is but changes the
/// style its kind of label keeps still records one undo step; given again, it records none.
#[test]
fn a_kept_graph_label_style_change_alone_records_one_undo_step() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4";
    let graph = NodeId(
        s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let doc = s.doc().unwrap().doc.clone();
    let legend = doc.node(graph).unwrap().children().unwrap().iter().find(|c| c.name.as_deref() == Some("Legend")).unwrap();
    let (label, size) = legend
        .children()
        .unwrap()
        .iter()
        .find_map(|c| match &c.kind {
            NodeKind::Text(t) if t.plain_text() == "2025" => Some((c.id, t.runs[0].style.size)),
            _ => None,
        })
        .unwrap();
    let kept = |s: &Session| s.doc().unwrap().doc.node(graph).unwrap().extras().graph.as_deref().unwrap().label_styles.legend.clone();
    assert!(kept(&s).is_none());
    let p = json!({"ids": [label.0], "size": size});
    let before = state(&s);
    s.execute("text.setStyle", &p).unwrap();
    let after = state(&s);
    assert!(after.0.node(label) == before.0.node(label), "the label's own text changed");
    assert!(kept(&s).is_some(), "the legend keeps the style");
    assert_eq!(after.1, before.1 + 1, "the kept style change recorded no undo step");
    records_nothing(&mut s, "text.setStyle", p);
}

/// `text.setText` with the text the type already has records no undo step; new text records one.
#[test]
fn the_same_text_records_no_undo_step() {
    let (mut s, _) = session_with("Same words", None);
    records_nothing(&mut s, "text.setText", json!({"text": "Same words"}));
    records_one_step(&mut s, "text.setText", json!({"text": "Other words"}));
}

/// Fit Headline run again on a headline it already fitted records no undo step; the first run
/// records one.
#[test]
fn fit_headline_again_records_no_undo_step() {
    let (mut s, _) = session_with("HEADLINE\nbody text", Some(json!({"width": 300, "height": 120})));
    let start = state(&s);
    let tracking = s.execute("text.fitHeadline", &json!({})).unwrap()["tracking"].clone();
    let fitted = state(&s);
    assert!(*fitted.0 != *start.0 && fitted.1 == start.1 + 1, "the first Fit Headline did not record one undo step");
    let before = state(&s);
    assert_eq!(s.execute("text.fitHeadline", &json!({})).unwrap()["tracking"], tracking);
    let after = state(&s);
    assert!(*after.0 == *before.0 && after.1 == before.1, "a second Fit Headline recorded an undo step");
}

/// Reshaping area type or type on a path by no distance, or moving a path handle to where it is,
/// records no undo step; a reshape that moves something records one.
#[test]
fn reshaping_type_by_nothing_records_no_undo_step() {
    let (mut s, id) = session_with("Boxed words", Some(json!({"width": 200, "height": 100})));
    records_nothing(&mut s, "text.reshapeArea", json!({"id": id, "anchors": [[0, 2]], "dx": 0, "dy": 0}));
    records_one_step(&mut s, "text.reshapeArea", json!({"id": id, "anchors": [[0, 2]], "dx": 20, "dy": 10}));
    let path =
        s.execute("path.create", &json!({"anchors": [{"x": 100, "y": 300, "in": [50, 300], "out": [150, 300]}, {"x": 400, "y": 300}]})).unwrap();
    let on_path = s.execute("text.createInPath", &json!({"path": path["id"], "mode": "onPath", "text": "Along the curve"})).unwrap()["id"].clone();
    records_nothing(&mut s, "text.reshapePath", json!({"id": on_path, "anchors": [[0, 1]], "dx": 0, "dy": 0}));
    records_one_step(&mut s, "text.reshapePath", json!({"id": on_path, "anchors": [[0, 1]], "dx": 0, "dy": 20}));
    let handle = json!({"id": on_path, "anchor": 0, "which": "out", "x": 100, "y": 250});
    records_one_step(&mut s, "text.setPathHandle", handle.clone());
    records_nothing(&mut s, "text.setPathHandle", handle);
}
