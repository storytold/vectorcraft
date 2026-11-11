//! Dynamic Text's Block for area type: `type.dynamicText`, Area Type Options › Block and the refit
//! after edits.

use serde_json::{Value, json};
use vectorcraft_doc::{AreaFit, NodeId, NodeKind, TextKind, TextObject};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn area(s: &mut Session, w: f64, h: f64, text: &str) -> u64 {
    let id = s.execute("text.create", &json!({"x": 40, "y": 40, "size": 12, "area": {"width": w, "height": h}, "text": text})).unwrap()["id"]
        .as_u64()
        .unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    id
}

fn text(s: &Session, id: u64) -> TextObject {
    match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    }
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The size of the character at `byte`.
fn size_at(t: &TextObject, byte: usize) -> f64 {
    super::block::size_at(&t.runs, byte)
}

/// Each laid-out line's (start byte, content width, room): the room is the frame's span less the
/// indents.
fn lines(t: &TextObject) -> Vec<(usize, f64, f64)> {
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    lay.lines.iter().map(|l| (l.start, l.x1 - l.x0, l.avail.1 - l.avail.0)).collect()
}

/// Every typed line is one laid-out line whose width is within one 0.1 pt size step of its room.
fn assert_block(t: &TextObject) {
    let typed: Vec<usize> = std::iter::once(0).chain(t.plain_text().match_indices(['\n', '\u{2028}']).map(|(i, c)| i + c.len())).collect();
    let laid = lines(t);
    assert_eq!(laid.iter().map(|l| l.0).collect::<Vec<_>>(), typed, "one laid-out line per typed line: {laid:?}");
    for &(start, width, room) in &laid {
        let size = size_at(t, start);
        if width <= 1e-9 || size >= 1296.0 - 1e-9 || size <= 0.1 + 1e-9 {
            continue;
        }
        assert!(width <= room + 0.01, "line at {start} is wider than its room: {width} > {room}");
        assert!(width >= room * (size - 0.1) / size - 0.01, "line at {start} at {size} pt is short of its room: {width} < {room}");
    }
}

#[test]
fn block_sets_each_typed_line_to_the_frame_width_by_its_size() {
    let mut s = session();
    let id = area(&mut s, 300.0, 400.0, "BIG\na much longer second line\nmid one");
    let n = undo_len(&s);
    let r = s.execute("type.dynamicText", &json!({})).unwrap();
    assert_eq!(r, json!({"on": true, "ids": [id], "fitScale": 1.0, "wordGap": 100.0, "lineGap": 0.0}));
    assert_eq!(undo_len(&s), n + 1, "one undo step");
    let t = text(&s, id);
    assert!(t.dynamic_text);
    assert_eq!(t.area.fit, AreaFit::None);
    assert_block(&t);
    let (big, long, mid) = (size_at(&t, 0), size_at(&t, 4), size_at(&t, 31));
    assert!(big > mid && mid > long, "a shorter line is set larger: {big} {mid} {long}");
    for size in [big, long, mid] {
        assert!(((size * 10.0).round() - size * 10.0).abs() < 1e-6, "{size} is a whole number of tenths");
    }
    // Each character's leading is its size (line gap 0).
    assert!(t.runs.iter().all(|r| r.style.leading == Some(r.style.size)), "{:?}", t.runs);
    // Inspect reports it.
    let doc = s.execute("document.inspect", &json!({})).unwrap();
    let node = doc["layers"][0]["children"].as_array().unwrap().iter().find(|k| k["id"] == id).unwrap().clone();
    assert_eq!((node["dynamicText"].as_bool(), node["fit"].as_str()), (Some(true), Some("block")));
}

#[test]
fn block_keeps_the_proportions_inside_a_line_and_fits_again_after_edits() {
    let mut s = session();
    let id = area(&mut s, 300.0, 400.0, "one two three\nfour");
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 3, "size": 24})).unwrap();
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let t = text(&s, id);
    assert_block(&t);
    let ratio = size_at(&t, 0) / size_at(&t, 4);
    assert!((ratio - 2.0).abs() < 0.02, "the first word stays twice the size of the rest: {ratio}");
    let four = size_at(&t, 14);
    // Typing into the first line makes it smaller, in the edit's own undo step; the second line stays.
    let first = size_at(&t, 4);
    let n = undo_len(&s);
    s.execute("text.editRange", &json!({"id": id, "start": 13, "end": 13, "insert": " and more words"})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let t = text(&s, id);
    assert_block(&t);
    assert!(size_at(&t, 4) < first, "{} < {first}", size_at(&t, 4));
    assert_eq!(size_at(&t, t.plain_text().find("four").unwrap()), four);
    // A narrower frame sets every line smaller.
    s.execute("text.areaOptions", &json!({"width": 150})).unwrap();
    let narrow = text(&s, id);
    assert_block(&narrow);
    let f = size_at(&narrow, narrow.plain_text().find("four").unwrap()) / four;
    assert!((f - 0.5).abs() < 0.02, "half the width, half the size: {f}");
    // Undo brings the wider frame's sizes back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s, id).runs, t.runs);
}

#[test]
fn block_turns_off_keeping_its_sizes_and_records_nothing_when_nothing_changes() {
    let mut s = session();
    // The tab goes to a stop that stays where it is as the size changes.
    let id = area(&mut s, 300.0, 400.0, "Fit me\nand\tme too");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let fitted = text(&s, id).runs;
    let laid = lines(&text(&s, id));
    assert_eq!(laid.iter().map(|l| l.0).collect::<Vec<_>>(), vec![0, 7], "{laid:?}");
    assert!(laid.iter().all(|l| l.1 <= l.2 + 0.01 && l.1 > l.2 * 0.95), "each line fills its room: {laid:?}");
    let n = undo_len(&s);
    assert_eq!(s.execute("type.dynamicText", &json!({"on": true})).unwrap()["on"], true);
    assert_eq!(undo_len(&s), n, "already on: no undo step");
    let r = s.execute("type.dynamicText", &json!({})).unwrap();
    assert_eq!(r["on"], false);
    assert_eq!(undo_len(&s), n + 1);
    let t = text(&s, id);
    assert!(!t.dynamic_text);
    assert_eq!(t.runs, fitted, "off keeps the sizes it was drawn at");
    // Edits no longer fit the lines.
    s.execute("text.areaOptions", &json!({"width": 100})).unwrap();
    assert_eq!(text(&s, id).runs, fitted);
    assert_eq!(s.execute("type.dynamicText", &json!({"on": false})).unwrap()["on"], false);
    assert_eq!(undo_len(&s), n + 2, "already off: no undo step");
}

#[test]
fn bad_params_and_other_type_are_refused() {
    let mut s = session();
    let id = area(&mut s, 300.0, 400.0, "Fit me");
    for bad in [
        json!({"on": "yes"}),
        json!({"on": 1}),
        json!({"wordGap": "wide"}),
        json!({"wordGap": -1}),
        json!({"wordGap": 1001}),
        json!({"lineGap": 1297}),
        json!({"lineGap": -1297}),
        json!({"lineGap": true}),
    ] {
        assert!(s.execute("type.dynamicText", &bad).is_err(), "{bad}");
    }
    assert!(!text(&s, id).dynamic_text);
    let point = s.execute("text.create", &json!({"x": 10, "y": 500, "text": "point"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [point]})).unwrap();
    let e = s.execute("type.dynamicText", &json!({})).unwrap_err().to_string();
    assert!(e.contains("select type on a path or area type"), "{e}");
    // With point type selected too, only the area type changes.
    s.execute("select.set", &json!({"ids": [id, point]})).unwrap();
    let r = s.execute("type.dynamicText", &json!({})).unwrap();
    assert_eq!(r["ids"], json!([id]));
    assert!(text(&s, id).dynamic_text && !text(&s, point).dynamic_text);
    // A gap with only type on a path selected is an error, and nothing changes.
    let path = s.execute("path.create", &json!({"d": "M60 550 L340 550"})).unwrap()["id"].as_u64().unwrap();
    let along = s.execute("text.createInPath", &json!({"path": path, "mode": "onPath", "text": "Along"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [along]})).unwrap();
    let n = undo_len(&s);
    for gap in [json!({"wordGap": 300}), json!({"on": false, "lineGap": 2})] {
        let e = s.execute("type.dynamicText", &gap).unwrap_err().to_string();
        assert!(e.contains("need area type"), "{e}");
    }
    assert_eq!(undo_len(&s), n);
    assert!(!text(&s, along).dynamic_text);
}

#[test]
fn gaps_set_word_spacing_and_leading() {
    let mut s = session();
    let id = area(&mut s, 300.0, 400.0, "two words\nthree more words");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let plain = text(&s, id);
    let r = s.execute("type.dynamicText", &json!({"wordGap": 300, "lineGap": 6})).unwrap();
    assert_eq!((r["on"].as_bool(), r["wordGap"].as_f64(), r["lineGap"].as_f64()), (Some(true), Some(300.0), Some(6.0)));
    let t = text(&s, id);
    assert!(t.dynamic_text);
    assert_eq!((t.area.word_gap, t.area.line_gap), (300.0, 6.0));
    // The layout uses the gap as the Desired word spacing; the paragraph keeps its own.
    assert_eq!(t.para_at(0).justification.word_spacing, [80.0, 100.0, 133.0]);
    assert_eq!(t.layout_justification(t.para_at(0)).word_spacing, [80.0, 300.0, 300.0]);
    assert!(t.runs.iter().all(|r| (r.style.leading.unwrap() - r.style.size - 6.0).abs() < 1e-9), "{:?}", t.runs);
    assert_block(&t);
    assert!(size_at(&t, 0) < size_at(&plain, 0), "wider word spaces leave the letters less room");
    // (own, used by the layout) word spacing of the first paragraph.
    let spacing = |s: &Session| {
        let t = text(s, id);
        (t.para_at(0).justification.word_spacing, t.layout_justification(t.para_at(0)).word_spacing)
    };
    // Minimum and Maximum move out only as far as the current gap needs, and the paragraph's own
    // values apply again when Block is off.
    s.execute("type.dynamicText", &json!({"wordGap": 1000})).unwrap();
    assert_eq!(spacing(&s), ([80.0, 100.0, 133.0], [80.0, 1000.0, 1000.0]));
    s.execute("type.dynamicText", &json!({"wordGap": 100})).unwrap();
    assert_eq!(spacing(&s), ([80.0, 100.0, 133.0], [80.0, 100.0, 133.0]));
    s.execute("type.dynamicText", &json!({"wordGap": 0})).unwrap();
    assert_eq!(spacing(&s), ([80.0, 100.0, 133.0], [0.0, 0.0, 133.0]));
    s.execute("type.dynamicText", &json!({"on": false})).unwrap();
    assert_eq!(spacing(&s), ([80.0, 100.0, 133.0], [80.0, 100.0, 133.0]));
    // A paragraph's own values, outside the default range and equal to Desired too, stay through
    // gap changes and refits after edits.
    s.execute("text.setFormat", &json!({"wordSpacing": [20, 20, 150]})).unwrap();
    s.execute("type.dynamicText", &json!({"on": true, "wordGap": 20})).unwrap();
    assert_eq!(spacing(&s), ([20.0, 20.0, 150.0], [20.0, 20.0, 150.0]));
    s.execute("type.dynamicText", &json!({"wordGap": 1000})).unwrap();
    assert_eq!(spacing(&s), ([20.0, 20.0, 150.0], [20.0, 1000.0, 1000.0]));
    s.execute("type.dynamicText", &json!({"wordGap": 100})).unwrap();
    s.execute("text.editRange", &json!({"id": id, "start": 0, "end": 0, "insert": "a "})).unwrap();
    assert_eq!(spacing(&s), ([20.0, 20.0, 150.0], [20.0, 100.0, 150.0]));
    s.execute("type.dynamicText", &json!({"on": false})).unwrap();
    assert_eq!(spacing(&s), ([20.0, 20.0, 150.0], [20.0, 20.0, 150.0]));
    // A gap alone on a frame without Block turns it on.
    let other = area(&mut s, 200.0, 200.0, "x y");
    let r = s.execute("type.dynamicText", &json!({"lineGap": -2})).unwrap();
    assert_eq!(r["on"], true);
    let t = text(&s, other);
    assert!(t.dynamic_text);
    assert_eq!((t.area.word_gap, t.area.line_gap), (100.0, -2.0));
}

#[test]
fn area_type_options_offer_block_with_its_gaps() {
    let mut s = session();
    let id = area(&mut s, 300.0, 400.0, "Fit me\nand me too");
    let q = s.execute("text.areaOptions", &json!({})).unwrap();
    assert_eq!((q["wordGap"].as_f64(), q["lineGap"].as_f64()), (Some(100.0), Some(0.0)));
    // Gaps alone are kept for Block, without turning it on or changing the text.
    let (n, runs) = (undo_len(&s), text(&s, id).runs);
    s.execute("text.areaOptions", &json!({"wordGap": 200})).unwrap();
    let t = text(&s, id);
    assert_eq!((undo_len(&s), t.dynamic_text, t.area.word_gap), (n + 1, false, 200.0));
    assert_eq!(t.runs, runs);
    let r = s.execute("text.areaOptions", &json!({"fit": "block", "lineGap": 4})).unwrap();
    assert_eq!((r["fit"].as_str(), r["wordGap"].as_f64(), r["lineGap"].as_f64()), (Some("block"), Some(200.0), Some(4.0)));
    assert!(text(&s, id).dynamic_text);
    assert_block(&text(&s, id));
    // The reply passed back (as the dialog does) changes nothing; naming Block again keeps the gaps.
    let n = undo_len(&s);
    assert_eq!(s.execute("text.areaOptions", &r).unwrap(), r);
    s.execute("text.areaOptions", &json!({"fit": "block"})).unwrap();
    assert_eq!(undo_len(&s), n);
    s.execute("text.areaOptions", &json!({"lineGap": 2})).unwrap();
    let t = text(&s, id);
    assert_eq!((t.dynamic_text, t.area.word_gap, t.area.line_gap), (true, 200.0, 2.0));
    assert!(s.execute("text.areaOptions", &json!({"lineGap": 5000})).is_err());
    assert!(s.execute("text.areaOptions", &json!({"wordGap": 2000})).is_err());
    assert!(s.execute("text.areaOptions", &json!({"wordGap": "wide"})).is_err());
    assert!(s.execute("text.areaOptions", &json!({"fit": "squash"})).unwrap_err().to_string().contains("shrinkText|block"));
    // Shrink Text to Fit replaces Block, keeping the sizes.
    let runs = text(&s, id).runs;
    s.execute("text.areaOptions", &json!({"fit": "shrinkText"})).unwrap();
    let t = text(&s, id);
    assert!(!t.dynamic_text);
    assert_eq!(t.runs, runs);
    // Block replaces Shrink Text to Fit and Auto Size.
    s.execute("text.areaOptions", &json!({"fit": "block"})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::None);
    // New area type can start with it.
    let made = s
        .execute(
            "text.create",
            &json!({"x": 10, "y": 300, "size": 12, "area": {"width": 200, "height": 200, "fit": "block", "wordGap": 150}, "text": "made"}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let t = text(&s, made);
    assert_eq!((t.dynamic_text, t.area.word_gap, t.area.line_gap), (true, 150.0, 0.0));
    assert_block(&t);
}

#[test]
fn block_round_trips_through_the_native_format() {
    let mut s = session();
    let id = area(&mut s, 300.0, 400.0, "Fit me\nand me too");
    s.execute("type.dynamicText", &json!({"wordGap": 120, "lineGap": 3})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let bytes = vectorcraft_format::save(&doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("text") };
    assert_eq!((t.dynamic_text, t.area.word_gap, t.area.line_gap), (true, 120.0, 3.0));
    assert_eq!(t.runs, text(&s, id).runs, "the fitted sizes are saved in the characters");
    // Saved as `dynamicText` with the gaps beside the area options; missing gaps read as their
    // defaults, which are left out when saved.
    let saved = serde_json::to_value(&**t).unwrap();
    assert_eq!((&saved["dynamicText"], &saved["area"]["wordGap"], &saved["area"]["lineGap"]), (&json!(true), &json!(120.0), &json!(3.0)));
    let plain = serde_json::from_value::<vectorcraft_doc::AreaOptions>(json!({})).unwrap();
    assert_eq!((plain.word_gap, plain.line_gap), (100.0, 0.0));
    let plain: Value = serde_json::to_value(&plain).unwrap();
    assert!(plain.get("wordGap").is_none() && plain.get("lineGap").is_none(), "{plain}");
    // Turned off, it opens off with the sizes it had.
    s.execute("type.dynamicText", &json!({"on": false})).unwrap();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("text") };
    assert!(!t.dynamic_text);
    assert_eq!(t.runs, text(&s, id).runs);
}

#[test]
fn edge_cases_fit_without_failing() {
    let mut s = session();
    // Empty text, an empty line and a line of spaces keep their sizes.
    let empty = area(&mut s, 300.0, 400.0, "");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    assert_eq!(size_at(&text(&s, empty), 0), 12.0);
    let gaps = area(&mut s, 300.0, 400.0, "top\n\n   \nbottom line");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let t = text(&s, gaps);
    assert_eq!((size_at(&t, 4), size_at(&t, 5)), (12.0, 12.0));
    assert_block(&t);
    // One word.
    let word = area(&mut s, 300.0, 400.0, "Word");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    assert_block(&text(&s, word));
    // A wide frame stops at 1,296 pt; a very long line in a narrow frame stops at 0.1 pt and wraps.
    let wide = area(&mut s, 100_000.0, 400.0, "i");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    assert_eq!(size_at(&text(&s, wide), 0), 1296.0);
    let narrow = area(&mut s, 1.0, 400.0, &"long words ".repeat(40));
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let t = text(&s, narrow);
    assert_eq!(size_at(&t, 0), 0.1);
    assert!(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow, "at 0.1 pt the line still does not fit");
    // A frame inset so far that it has no width keeps the sizes.
    let inset = area(&mut s, 50.0, 400.0, "Inset");
    s.execute("text.areaOptions", &json!({"inset": 30})).unwrap();
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    assert_eq!(size_at(&text(&s, inset), 0), 12.0);
}

#[test]
fn block_fits_columns_lists_vertical_and_right_to_left_type() {
    let mut s = session();
    // Two columns: each line is set to a column's width.
    let cols = area(&mut s, 400.0, 600.0, "left column\nsecond");
    s.execute("text.areaOptions", &json!({"columns": 2, "gutter": 20})).unwrap();
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let t = text(&s, cols);
    assert_block(&t);
    assert!(lines(&t).iter().all(|l| (l.2 - 190.0).abs() < 1e-6), "{:?}", lines(&t));
    // A bulleted list: each item stays one line, the marker and its tab taking their room before
    // the text (the room the layout reports takes in the marker's too).
    let list = area(&mut s, 300.0, 400.0, "first item\nsecond item here");
    s.execute("text.setFormat", &json!({"list": "bullet"})).unwrap();
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let t = text(&s, list);
    let laid = lines(&t);
    assert_eq!(laid.iter().map(|l| l.0).collect::<Vec<_>>(), vec![0, 11], "{laid:?}");
    assert!(laid.iter().all(|l| l.1 <= l.2), "{laid:?}");
    assert!(size_at(&t, 0) > size_at(&t, 11), "the shorter item is larger");
    // Right-to-left text.
    let rtl = area(&mut s, 300.0, 400.0, "שלום עולם\nטקסט ארוך יותר בשורה");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    assert_block(&text(&s, rtl));
    // Vertical type: each column of text is set to the frame's height.
    let v = s
        .execute(
            "text.create",
            &json!({"x": 40, "y": 40, "size": 12, "vertical": true, "area": {"width": 300, "height": 200}, "text": "縦書き\nもっと長い縦書きの行"}),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    s.execute("select.set", &json!({"ids": [v]})).unwrap();
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let t = text(&s, v);
    assert!(t.vertical);
    assert_block(&t);
    assert!(lines(&t).iter().all(|l| (l.2 - 200.0).abs() < 1e-6), "{:?}", lines(&t));
}

#[test]
fn threaded_frames_are_refused_and_threading_turns_block_off() {
    let mut s = session();
    let a = area(&mut s, 200.0, 300.0, "Fit me\nand me too");
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let fitted = text(&s, a).runs;
    let b = s.execute("shape.rectangle", &json!({"x": 400, "y": 40, "width": 200, "height": 300})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let n = undo_len(&s);
    s.execute("text.thread.create", &json!({})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let t = text(&s, a);
    assert!(!t.dynamic_text, "threading turns Block off");
    assert_eq!(t.runs, fitted, "the text keeps its sizes");
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let e = s.execute("type.dynamicText", &json!({"on": true})).unwrap_err().to_string();
    assert!(e.contains("area type in a thread") && e.contains(&a.to_string()), "{e}");
    let runs = text(&s, a).runs;
    let e = s.execute("text.areaOptions", &json!({"fit": "block"})).unwrap_err().to_string();
    assert!(e.contains("area type in a thread") && e.contains(&a.to_string()), "{e}");
    assert_eq!(text(&s, a).runs, runs, "Area Type Options refuses it too, changing nothing");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(text(&s, a).dynamic_text, "undo brings it back with the thread gone");
    assert!(matches!(text(&s, a).kind, TextKind::Area { .. }));
}

#[test]
fn long_text_fits_in_a_bounded_number_of_layouts() {
    let mut s = session();
    let story: String = (0..300).map(|i| format!("line number {i} {}\n", "x".repeat(i % 17))).collect();
    let id = area(&mut s, 300.0, 100_000.0, &story);
    let before = vectorcraft_text::test_fonts::layouts_on_this_thread();
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    let used = vectorcraft_text::test_fonts::layouts_on_this_thread() - before;
    assert_block(&text(&s, id));
    // The command fits the object to compare and to apply (each a measure, at most two layouts a
    // pass and the caches), plus the reply's own.
    let bound = 2 * (2 + 2 * super::block::BLOCK_PASSES as u64) + 4;
    assert!(used <= bound, "{used} layouts");
}
