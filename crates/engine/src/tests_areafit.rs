//! Area type fitting: Auto Size (the frame's height follows the text), Shrink Text to Fit, and
//! overflow reporting.

use serde_json::{Value, json};
use vectorcraft_doc::{AreaFit, NodeKind, TextKind, TextObject};
use vectorcraft_geom::Rect;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;

const STORY: &str = "The quick brown fox jumps over the lazy dog. A second sentence follows the first one here. Then a third one ends it.";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn area(s: &mut Session, w: f64, h: f64, text: &str, extra: Value) -> NodeId {
    let mut a = json!({"width": w, "height": h});
    if let (Some(a), Some(e)) = (a.as_object_mut(), extra.as_object()) {
        a.extend(e.clone());
    }
    let id = s.execute("text.create", &json!({"x": 40, "y": 40, "size": 12, "area": a, "text": text})).unwrap()["id"].as_u64().unwrap();
    NodeId(id)
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    }
}

fn frame(s: &Session, id: NodeId) -> Rect {
    match text(s, id).kind {
        TextKind::Area { frame } => frame.bounds().unwrap(),
        k => panic!("not area type: {k:?}"),
    }
}

fn overflows(s: &Session, id: NodeId) -> bool {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(s, id)).overflow
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn area_options_set_and_report_the_fit() {
    let mut s = session();
    let id = area(&mut s, 120.0, 30.0, STORY, json!({}));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let q = s.execute("text.areaOptions", &json!({})).unwrap();
    assert_eq!((q["fit"].as_str(), q["fitMinPercent"].as_f64()), (Some("none"), Some(50.0)));
    assert_eq!((q["overflow"].as_bool(), q["fitScale"].as_f64()), (Some(true), Some(1.0)));
    let n = undo_len(&s);
    let r = s.execute("text.areaOptions", &json!({"fit": "shrinkText", "fitMinPercent": 20})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!((r["fit"].as_str(), r["fitMinPercent"].as_f64()), (Some("shrinkText"), Some(20.0)));
    assert_eq!(r["overflow"], false, "{r}");
    let f = r["fitScale"].as_f64().unwrap();
    assert!((0.2..1.0).contains(&f), "{f}");
    assert_eq!(text(&s, id).area.fit, AreaFit::ShrinkText { min_percent: 20.0 });
    assert_eq!(text(&s, id).runs[0].style.size, 12.0, "the stored size stays");
    // The minimum alone, clamped; the object form; and bad values.
    s.execute("text.areaOptions", &json!({"fitMinPercent": 3})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::ShrinkText { min_percent: 10.0 });
    s.execute("text.areaOptions", &json!({"fit": {"shrinkText": {"minPercent": 95}}})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::ShrinkText { min_percent: 95.0 });
    assert_eq!(s.execute("text.areaOptions", &json!({})).unwrap()["overflow"], true, "95 % is not enough");
    assert!(s.execute("text.areaOptions", &json!({"fit": "squash"})).is_err());
    assert!(s.execute("text.areaOptions", &json!({"fit": 7})).is_err());
    // The reply's own keys go back in unchanged (the dialog sends everything).
    let q = s.execute("text.areaOptions", &json!({})).unwrap();
    let n = undo_len(&s);
    let again = s.execute("text.areaOptions", &q).unwrap();
    assert_eq!(again, q);
    assert_eq!(undo_len(&s), n + 1);
    // Undo back to no fit.
    while text(&s, id).area.fit != AreaFit::None {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert!(overflows(&s, id));
}

#[test]
fn inspect_reports_overflow_and_fit_scale() {
    let mut s = session();
    let id = area(&mut s, 120.0, 30.0, STORY, json!({"fit": "shrinkText", "fitMinPercent": 10}));
    s.execute("text.create", &json!({"x": 10, "y": 500, "text": "point"})).unwrap();
    let doc = s.execute("document.inspect", &json!({})).unwrap();
    let kids = doc["layers"][0]["children"].as_array().unwrap();
    let node = kids.iter().find(|k| k["id"] == id.0).unwrap();
    assert_eq!(node["overflow"], false);
    assert_eq!(node["fit"], "shrinkText");
    assert!(node["fitScale"].as_f64().unwrap() < 1.0);
    let point = kids.iter().find(|k| k["id"] != id.0).unwrap();
    assert!(point.get("overflow").is_none() && point.get("fitScale").is_none(), "point type never overflows");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.areaOptions", &json!({"fit": "none"})).unwrap();
    let doc = s.execute("document.inspect", &json!({})).unwrap();
    let node = doc["layers"][0]["children"].as_array().unwrap().iter().find(|k| k["id"] == id.0).unwrap().clone();
    assert_eq!((node["overflow"].as_bool(), node["fitScale"].as_f64(), node["fit"].as_str()), (Some(true), Some(1.0), Some("none")));
}

#[test]
fn auto_height_follows_edits_in_the_same_undo_step() {
    let mut s = session();
    let id = area(&mut s, 200.0, 300.0, "One line", json!({"fit": "autoHeight"}));
    let one = frame(&s, id);
    assert!(one.height() < 30.0, "the frame fits one line: {one:?}");
    assert_eq!((one.x0, one.y0, one.width()), (0.0, 0.0, 200.0), "top and width stay");
    assert!(!overflows(&s, id));
    let n = undo_len(&s);
    s.execute("text.editRange", &json!({"id": id.0, "start": 8, "insert": format!(" {STORY}")})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let grown = frame(&s, id);
    assert!(grown.height() > one.height() * 2.0, "{grown:?}");
    assert!(!overflows(&s, id), "the grown frame holds the text");
    // Tight: a point less overflows.
    let mut t = text(&s, id);
    t.area.fit = AreaFit::None;
    t.kind = TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Shape::to_path(&Rect::new(0.0, 0.0, 200.0, grown.height() - 1.0), 0.1)),
    };
    assert!(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow);
    // Character changes resize it too.
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 3, "size": 40})).unwrap();
    assert!(frame(&s, id).height() > grown.height());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), grown);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), one, "one undo restores the text and the frame");
    assert_eq!(text(&s, id).plain_text(), "One line");
}

#[test]
fn auto_height_follows_typing_with_the_type_tool() {
    let mut s = session();
    s.prefs.auto_size_area_type = true;
    // An empty frame (no placeholder text), so its first height is one line.
    s.prefs.placeholder_text = false;
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for (kind, (x, y)) in [(PointerKind::Down, (100.0, 100.0)), (PointerKind::Drag, (300.0, 400.0)), (PointerKind::Up, (300.0, 400.0))] {
        s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0];
    assert_eq!(text(&s, id).area.fit, AreaFit::AutoHeight, "the preference gives new area type Auto Size");
    let empty = frame(&s, id);
    assert!(empty.height() < 30.0, "{empty:?}");
    let n = undo_len(&s);
    for word in STORY.split_inclusive(' ') {
        s.tool_text(word, v).unwrap();
    }
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!(text(&s, id).plain_text(), STORY);
    assert_eq!(undo_len(&s), n + 1, "one typing step");
    let typed = frame(&s, id);
    assert!(typed.height() > empty.height() * 2.0, "{typed:?}");
    assert!(!overflows(&s, id));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), empty);
    assert_eq!(text(&s, id).plain_text(), "");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), typed);
}

#[test]
fn auto_height_in_columns_and_by_hand() {
    let mut s = session();
    let id = area(&mut s, 300.0, 400.0, &STORY.repeat(3), json!({"fit": "autoHeight"}));
    let one = frame(&s, id).height();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    // Height is ignored while Auto Size is on.
    let r = s.execute("text.areaOptions", &json!({"columns": 2, "gutter": 12, "height": 500})).unwrap();
    let two = r["height"].as_f64().unwrap();
    assert!(two < 500.0 && two > one * 0.5, "{one} → {two}");
    assert_eq!(r["overflow"], false);
    assert_eq!(r["fit"], "autoHeight");
    // The least height the two columns fit: a point less overflows, and both columns are used.
    let mut t = text(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(lay.glyphs.iter().any(|g| g.origin.x > 160.0), "text flows into the second column");
    t.area.fit = AreaFit::None;
    t.kind = TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Shape::to_path(&Rect::new(0.0, 0.0, 300.0, two - 1.0), 0.1)),
    };
    assert!(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow);
    // Dragging the bottom edge sets the height by hand: Auto Size goes off.
    let b = frame(&s, id);
    s.execute("text.reshapeArea", &json!({"id": id.0, "anchors": [[0, 2], [0, 3]], "dx": 0, "dy": 50})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::None);
    assert!((frame(&s, id).height() - (b.height() + 50.0)).abs() < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    // A new width keeps it: the height follows the reflow.
    s.execute("text.reshapeArea", &json!({"id": id.0, "anchors": [[0, 1], [0, 2]], "dx": 100, "dy": 0})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::AutoHeight);
    assert!(frame(&s, id).height() < b.height());
    // Point type has no frame to size.
    let p = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "x"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [p]})).unwrap();
    assert!(s.execute("text.areaOptions", &json!({"fit": "autoHeight"})).is_err(), "point type has no area options");
}

/// Typing at the start of a story in Auto Size area type with 2 columns lays the columns out at
/// most twice a keystroke: the bisection for their height runs on the one column's lines.
#[test]
fn auto_height_in_columns_lays_out_the_columns_twice_a_keystroke() {
    use crate::cmd::typecmd::COLUMN_LAYOUTS;
    let mut s = session();
    let story = format!("{STORY}\n").repeat(4);
    let id = area(&mut s, 300.0, 100.0, &story, json!({"fit": "autoHeight"}));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.areaOptions", &json!({"columns": 2, "gutter": 12})).unwrap();
    let before = frame(&s, id);
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    // The caret goes before the first character.
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 41.0, 45.0), v).unwrap();
    }
    let typed = "More words at the start, enough for another line or two in the first column. ";
    for ch in typed.chars() {
        let start = COLUMN_LAYOUTS.with(std::cell::Cell::get);
        s.tool_text(&ch.to_string(), v).unwrap();
        let laid = COLUMN_LAYOUTS.with(std::cell::Cell::get) - start;
        assert!(laid <= 2, "{ch:?} laid out the columns {laid} times");
    }
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!(text(&s, id).plain_text(), format!("{typed}{story}"));
    assert!(frame(&s, id).height() > before.height(), "the frame follows the text: {before:?} → {:?}", frame(&s, id));
    assert!(!overflows(&s, id));
}

/// Two sessions run the same keystrokes, edits, restyles, Area Type Options, undos and redos in
/// Auto Size area type with 2 and then 3 columns, the second with `FULL_SEARCH` on (the columns
/// laid out at every step of the bisection), and the type is the same in both after each step.
#[test]
fn auto_height_in_columns_matches_the_full_search() {
    use crate::cmd::typecmd::FULL_SEARCH;
    enum Step {
        Click(f64, f64),
        Type(&'static str),
        Key(ToolKey),
        Exec(&'static str, Value),
    }
    use Step::*;
    let v = ViewInfo::default();
    let story = format!("{STORY}\n{STORY} {STORY}\nShort one.\n{STORY}");
    let make = || {
        let mut s = session();
        let id = area(&mut s, 300.0, 100.0, &story, json!({"fit": "autoHeight"}));
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("text.areaOptions", &json!({"columns": 2, "gutter": 12})).unwrap();
        (s, id)
    };
    let ((mut quick, id), (mut full, full_id)) = (make(), make());
    assert_eq!(id, full_id);
    let run = |s: &mut Session, step: &Step| match step {
        Click(x, y) => {
            s.select_tool("type", v).unwrap();
            for kind in [PointerKind::Down, PointerKind::Up] {
                s.pointer(&PointerEvent::new(kind, *x, *y), v).unwrap();
            }
        }
        Type(typed) => {
            for ch in typed.chars() {
                s.tool_text(&ch.to_string(), v).unwrap();
            }
        }
        Key(k) => {
            s.tool_key(*k, Mods::default(), v).unwrap();
        }
        Exec(cmd, p) => {
            s.execute(cmd, p).unwrap();
        }
    };
    let mut steps = vec![
        // Typing at the start, a new paragraph and Backspace, then typing in the second column.
        Click(41.0, 45.0),
        Type("Words typed first "),
        Key(ToolKey::Enter),
        Type("A new paragraph of a few words"),
        Key(ToolKey::Backspace),
        Key(ToolKey::Backspace),
        Key(ToolKey::Escape),
        Click(200.0, 50.0),
        Type(" and more in the second column"),
        Key(ToolKey::Escape),
        // A long insertion and a deletion, larger type, paragraph spacing.
        Exec("text.editRange", json!({"id": id.0, "start": 20, "end": 20, "insert": format!("{STORY} {STORY}")})),
        Exec("text.editRange", json!({"id": id.0, "start": 0, "end": 60})),
        Exec("text.setRangeStyle", json!({"id": id.0, "start": 0, "end": 12, "size": 30})),
        Exec("text.setRangeStyle", json!({"id": id.0, "start": 150, "end": 170, "size": 20})),
        Exec("select.set", json!({"ids": [id.0]})),
        Exec("text.setFormat", json!({"spaceBefore": 6, "spaceAfter": 4})),
        // Three columns, typing in them, Area Type Options and the leading model.
        Exec("text.areaOptions", json!({"columns": 3})),
        Click(41.0, 60.0),
        Type("More"),
        Key(ToolKey::Escape),
        Exec("select.set", json!({"ids": [id.0]})),
        Exec("text.areaOptions", json!({"gutter": 30, "inset": 6})),
        Exec("text.areaOptions", json!({"firstBaseline": "capHeight"})),
        Exec("text.areaOptions", json!({"firstBaseline": "fixed", "firstBaselineMin": 20})),
        Exec("text.areaOptions", json!({"firstBaseline": "leading", "firstBaselineMin": 0})),
        Exec("text.setFormat", json!({"leadingModel": "emBoxTop"})),
    ];
    let start = undo_len(&quick);
    for (i, step) in steps.iter().enumerate() {
        run(&mut quick, step);
        FULL_SEARCH.with(|c| c.set(true));
        run(&mut full, step);
        FULL_SEARCH.with(|c| c.set(false));
        assert_eq!(text(&quick, id), text(&full, id), "after step {i}");
    }
    let edits = undo_len(&quick) - start;
    assert_eq!(edits, 14, "three typing sessions and eleven edits, one undo step each");
    steps.clear();
    steps.extend((0..edits).map(|_| Exec("edit.undo", json!({}))));
    steps.extend((0..edits).map(|_| Exec("edit.redo", json!({}))));
    for (i, step) in steps.iter().enumerate() {
        run(&mut quick, step);
        FULL_SEARCH.with(|c| c.set(true));
        run(&mut full, step);
        FULL_SEARCH.with(|c| c.set(false));
        assert_eq!(text(&quick, id), text(&full, id), "after undo or redo {i}");
    }
    assert_eq!((undo_len(&quick), undo_len(&full)), (start + edits, start + edits));
}

/// A line that starts with a letter smaller than the rest of it can sit higher or lower at the top
/// of a column than `columns_hold` places it. When the columns do not hold the text at the upper
/// height the search ends on, or do hold it at the lower one, Auto Size lays the columns out at
/// every step, and the frame gets the height the full search gives it. In 2 columns, lines in 6 pt
/// alternate with lines that start with a 6 pt letter and go on in 30 pt, with auto leading and
/// with 36 pt leading, and lines in 6 pt are added at the start one at a time.
#[test]
fn auto_height_in_columns_matches_the_full_search_when_lines_start_small() {
    use crate::cmd::typecmd::{COLUMN_LAYOUTS, FULL_SEARCH};
    let (small, big) = ("small words in six point type\n", "b BIG\n");
    let story = format!("{small}{big}").repeat(13);
    for leading in [json!("auto"), json!(36)] {
        let make = || {
            let mut s = session();
            let area = json!({"width": 300, "height": 100, "fit": "autoHeight"});
            let id = s.execute("text.create", &json!({"x": 40, "y": 40, "size": 6, "area": area, "text": story})).unwrap()["id"].as_u64().unwrap();
            for k in 0..13 {
                let start = k * (small.len() + big.len()) + small.len() + 1;
                s.execute("text.setRangeStyle", &json!({"id": id, "start": start, "end": start + 4, "size": 30})).unwrap();
            }
            s.execute("text.setRangeStyle", &json!({"id": id, "leading": leading})).unwrap();
            s.execute("select.set", &json!({"ids": [id]})).unwrap();
            s.execute("text.areaOptions", &json!({"columns": 2, "gutter": 12})).unwrap();
            (s, NodeId(id))
        };
        let ((mut quick, id), (mut full, _)) = (make(), make());
        let mut every_step = 0;
        for i in 0..12 {
            let add = json!({"id": id.0, "start": 0, "end": 0, "insert": "more small words\n"});
            let start = COLUMN_LAYOUTS.with(std::cell::Cell::get);
            quick.execute("text.editRange", &add).unwrap();
            let laid = COLUMN_LAYOUTS.with(std::cell::Cell::get) - start;
            every_step += usize::from(laid > 2);
            FULL_SEARCH.with(|c| c.set(true));
            full.execute("text.editRange", &add).unwrap();
            FULL_SEARCH.with(|c| c.set(false));
            assert_eq!(text(&quick, id), text(&full, id), "leading {leading}, after line {i}");
        }
        assert!(every_step > 0, "leading {leading}: the columns were never laid out at every step");
    }
}

/// Time typing at the start of a story of 5,000, 20,000 and 50,000 characters in Auto Size area
/// type in 2 and 3 columns: 20 letters typed one at a time with the Type tool, with Auto Size as it
/// runs and with `FULL_SEARCH` on (the columns laid out at every step of the bisection).
/// Run with `cargo test --release -p vectorcraft-engine --lib auto_height_in_columns_typing_timing -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn auto_height_in_columns_typing_timing() {
    use crate::cmd::typecmd::{COLUMN_LAYOUTS, FULL_SEARCH};
    const WORDS: &str = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame of this story. ";
    let v = ViewInfo::default();
    for n in [5_000, 20_000, 50_000] {
        let story = format!("{}\n", WORDS.repeat(6).trim_end()).repeat(n / 600 + 1)[..n].to_string();
        for cols in [2, 3] {
            let mut row = format!("{n} characters in {cols} columns:");
            for full in [false, true] {
                FULL_SEARCH.with(|c| c.set(full));
                let mut s = Session::new();
                s.execute("file.new", &json!({"width": 4400, "height": 1200})).unwrap();
                let area = json!({"width": 500, "height": 100, "fit": "autoHeight"});
                let id = s.execute("text.create", &json!({"x": 20, "y": 20, "size": 9, "text": story, "area": area})).unwrap()["id"].clone();
                s.execute("select.set", &json!({"ids": [id]})).unwrap();
                s.execute("text.areaOptions", &json!({"columns": cols, "gutter": 12})).unwrap();
                s.select_tool("type", v).unwrap();
                for kind in [PointerKind::Down, PointerKind::Up] {
                    s.pointer(&PointerEvent::new(kind, 21.0, 24.0), v).unwrap();
                }
                let layouts = COLUMN_LAYOUTS.with(std::cell::Cell::get);
                let mut ms: Vec<f64> = "abcdefghijklmnopqrst"
                    .chars()
                    .map(|ch| {
                        let start = std::time::Instant::now();
                        s.tool_text(&ch.to_string(), v).unwrap();
                        start.elapsed().as_secs_f64() * 1000.0
                    })
                    .collect();
                let per_key = (COLUMN_LAYOUTS.with(std::cell::Cell::get) - layouts) as f64 / ms.len() as f64;
                ms.sort_by(f64::total_cmp);
                let how = if full { "every step laid out" } else { "Auto Size" };
                row += &format!(" {how} {:.1} ms a keystroke ({per_key:.1} column layouts);", ms[ms.len() / 2]);
            }
            FULL_SEARCH.with(|c| c.set(false));
            println!("{row}");
        }
    }
}

#[test]
fn shrink_text_round_trips_through_the_native_format() {
    let mut s = session();
    let id = area(&mut s, 120.0, 30.0, STORY, json!({"fit": "shrinkText", "fitMinPercent": 35}));
    let doc = s.doc().unwrap().doc.clone();
    let bytes = vectorcraft_format::save(&doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    let NodeKind::Text(t) = &back.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.area.fit, AreaFit::ShrinkText { min_percent: 35.0 });
}

#[test]
fn auto_height_follows_the_leading_model_and_character_alignment() {
    let mut s = session();
    let id = area(&mut s, 200.0, 300.0, STORY, json!({"fit": "autoHeight"}));
    let roman = frame(&s, id);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    // Top-to-Top leading: the first line moves up to touch the frame, the frame follows.
    s.execute("text.setFormat", &json!({"leadingModel": "emBoxTop"})).unwrap();
    let em = frame(&s, id);
    assert_ne!(em.height(), roman.height());
    assert!(!overflows(&s, id), "the frame holds the text under emBoxTop");
    // Mixed sizes aligned on the em box centre still fit exactly.
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 3, "size": 30})).unwrap();
    s.execute("text.setFormat", &json!({"charAlign": "emBoxCenter"})).unwrap();
    let mixed = frame(&s, id);
    assert!(mixed.height() > em.height());
    assert!(!overflows(&s, id));
    let mut t = text(&s, id);
    t.area.fit = AreaFit::None;
    t.kind = TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Shape::to_path(&Rect::new(0.0, 0.0, 200.0, mixed.height() - 1.0), 0.1)),
    };
    assert!(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow, "tight");
}

/// #1471: the commands that make type or change what it holds say which text no longer fits, so
/// the call that pushed it out of its frame doesn't read as plain success.
#[test]
fn making_and_restyling_type_reports_its_overflow() {
    let mut s = session();
    // A headline in a frame lower than its line draws nothing.
    let low = s.execute("text.create", &json!({"x": 0, "y": 20, "text": "NEON DRIVE", "size": 60, "area": {"width": 400, "height": 60}})).unwrap();
    assert_eq!(low["overflow"], json!([low["id"]]), "{low}");
    // One that fits says nothing, nor does point type.
    let fits = s.execute("text.create", &json!({"x": 0, "y": 200, "text": "NEON DRIVE", "size": 60, "area": {"width": 400, "height": 90}})).unwrap();
    assert!(fits.get("overflow").is_none(), "{fits}");
    let point = s.execute("text.create", &json!({"x": 0, "y": 400, "text": "NEON DRIVE", "size": 60})).unwrap();
    assert!(point.get("overflow").is_none(), "{point}");
    // Growing the type pushes it out; of the two restyled, only the area type is named.
    let ids = json!([fits["id"], point["id"]]);
    let grown = s.execute("text.setStyle", &json!({"ids": ids, "size": 96})).unwrap();
    assert_eq!((&grown["ids"], &grown["overflow"]), (&ids, &json!([fits["id"]])), "{grown}");
    let back = s.execute("text.setStyle", &json!({"ids": ids, "size": 60})).unwrap();
    assert!(back.get("overflow").is_none(), "{back}");
    // More words than the frame holds.
    let long = s.execute("text.setText", &json!({"id": fits["id"], "text": STORY})).unwrap();
    assert_eq!(long["overflow"], json!([fits["id"]]), "{long}");
    // Type flowed into a path too small for it.
    let frame = s.execute("shape.rectangle", &json!({"x": 500, "y": 300, "width": 60, "height": 20})).unwrap();
    let inside = s.execute("text.createInPath", &json!({"path": frame["id"], "mode": "area", "text": STORY, "size": 12, "fit": "none"})).unwrap();
    assert_eq!(inside["overflow"], json!([inside["id"]]), "{inside}");
}
