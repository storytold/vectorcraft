use super::*;

const TEXT: &str = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame of this story. ";

fn setup() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let a = s.execute("text.create", &json!({"x": 10, "y": 10, "text": TEXT.repeat(4), "area": {"width": 150, "height": 60}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    (s, a, b)
}

fn text(s: &Session, id: u64) -> String {
    match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

#[test]
fn thread_flows_reflows_on_edit_and_survives_delete() {
    let (mut s, a, b) = setup();
    s.execute("text.thread.create", &json!({})).unwrap();
    let (ta, tb) = (text(&s, a), text(&s, b));
    assert!(!ta.is_empty() && !tb.is_empty());
    assert_eq!(format!("{ta}{tb}"), TEXT.repeat(4));
    // Editing the first frame re-flows into the second.
    s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "NEW "})).unwrap();
    assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), format!("NEW {}", TEXT.repeat(4)));
    assert_ne!(text(&s, b), tb, "the overflow moved on");
    // Deleting the first frame moves all its text into the second.
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(text(&s, b), format!("NEW {}", TEXT.repeat(4)));
    assert!(s.doc().unwrap().doc.text_threads.is_empty(), "a one-frame thread is no thread");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.text_threads.len(), 1);
}

#[test]
fn edits_copy_only_the_stories_of_threads_whose_frames_changed() {
    let (mut s, a, b) = setup();
    s.execute("text.thread.create", &json!({})).unwrap();
    let c = s.execute("text.create", &json!({"x": 10, "y": 200, "text": TEXT.repeat(2), "area": {"width": 150, "height": 40}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let d = s.execute("shape.rectangle", &json!({"x": 200, "y": 220, "width": 150, "height": 60})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [c, d]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    let texts = |s: &Session| [a, b, c, d].map(|id| text(s, id));
    let copied = || COPIED.with(std::cell::Cell::get);
    let (shown, start) = (texts(&s), copied());
    // A rectangle drawn, moved and dragged beside the threads changes none of their frames.
    let r = s.execute("shape.rectangle", &json!({"x": 400, "y": 250, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.begin_interaction("Move").unwrap();
    for dx in 1..=3 {
        s.preview("object.move", &json!({"dx": dx, "dy": 0})).unwrap();
    }
    s.commit_interaction().unwrap();
    assert_eq!(copied(), start, "edits beside the threads copied their stories");
    assert_eq!(texts(&s), shown);
    // Typing in the first thread copies and re-flows that thread's story only.
    s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "NEW "})).unwrap();
    assert_eq!(copied(), start + 1);
    assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), format!("NEW {}", TEXT.repeat(4)));
    assert_eq!(texts(&s)[2..], shown[2..]);
    let ids = |t: [u64; 2]| t.map(NodeId).to_vec();
    assert_eq!(s.doc().unwrap().doc.text_threads, [ids([a, b]), ids([c, d])]);
}

/// Time edits of a story of 50,000 characters threaded through 8 frames, each frame wrapping
/// around an ellipse, with one style, a style change every 10 words and a style change at every
/// word: 21 characters typed into the first frame, then 200 moves of a rectangle below the
/// frames and 200 frames of a drag of it.
/// Run with `cargo test --release -p vectorcraft-engine --lib threaded_story_edit_timing -- --ignored --nocapture`.
#[test]
#[ignore = "manual timing probe; wall-clock performance depends on machine load"]
fn threaded_story_edit_timing() {
    fn median(s: &mut Session, n: u32, f: &dyn Fn(&mut Session, f64)) -> f64 {
        let mut ms: Vec<f64> = (1..=n)
            .map(|i| {
                let start = std::time::Instant::now();
                f(s, f64::from(i));
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        ms[ms.len() / 2]
    }
    let story = format!("{}\n", TEXT.repeat(6).trim_end()).repeat(81)[..50_000].to_string();
    for words in [0, 10, 1] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 4400, "height": 1200})).unwrap();
        let area = json!({"width": 500, "height": 900});
        let first = s.execute("text.create", &json!({"x": 20, "y": 20, "text": story, "size": 9, "area": area})).unwrap()["id"].as_u64().unwrap();
        if words > 0 {
            s.edit("Restyle", |d, _| {
                let Some(NodeKind::Text(t)) = d.node_mut(NodeId(first)).map(|n| &mut n.kind) else { panic!("not text") };
                let style = t.runs[0].style.clone();
                let pieces: Vec<&str> = story.split_inclusive([' ', '\n']).collect();
                t.runs = pieces
                    .chunks(words)
                    .enumerate()
                    .map(|(i, c)| {
                        let mut run = TextRun::new(c.concat(), style.clone());
                        run.style.underline = i % 2 == 1;
                        run
                    })
                    .collect();
                refresh_bounds(t);
                Ok(())
            })
            .unwrap();
        }
        let mut ids = vec![first];
        for k in 1..8 {
            let frame = json!({"x": 20 + k * 540, "y": 20, "width": 500, "height": 900});
            ids.push(s.execute("shape.rectangle", &frame).unwrap()["id"].as_u64().unwrap());
        }
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        for k in 0..8 {
            let e = s.execute("shape.ellipse", &json!({"x": 195 + k * 540, "y": 395, "width": 150, "height": 150})).unwrap()["id"].clone();
            s.execute("object.textWrap.make", &json!({"ids": [e]})).unwrap();
        }
        let r = s.execute("shape.rectangle", &json!({"x": 20, "y": 1100, "width": 30, "height": 30})).unwrap()["id"].clone();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        let typing = median(&mut s, 21, &|s, _| {
            s.execute("text.editRange", &json!({"id": first, "start": 0, "end": 0, "insert": "x"})).unwrap();
        });
        let edit = median(&mut s, 200, &|s, _| {
            s.execute("object.move", &json!({"dx": 0.5, "dy": 0})).unwrap();
        });
        s.begin_interaction("Move").unwrap();
        let frame = median(&mut s, 200, &|s, i| {
            s.preview("object.move", &json!({"dx": i * 0.25, "dy": 0})).unwrap();
        });
        s.commit_interaction().unwrap();
        let d = &s.doc().unwrap().doc;
        let runs: usize = ids.iter().filter_map(|id| text_of(d, NodeId(*id))).map(|t| t.runs.len()).sum();
        println!("{runs} runs in 8 frames: typing {typing:.1} ms, a move below them {edit:.3} ms, a drag frame {frame:.3} ms (medians)");
    }
}

fn new_id(v: Value) -> u64 {
    v["id"].as_u64().unwrap()
}

fn threads(s: &Session) -> Vec<Vec<u64>> {
    s.doc().unwrap().doc.text_threads.iter().map(|t| t.iter().map(|i| i.0).collect()).collect()
}

/// Bounds of area type `id`'s frame on the page.
fn frame_box(s: &Session, id: u64) -> vectorcraft_geom::Rect {
    let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(id)) else { panic!("not area type") };
    let TextKind::Area { frame } = &t.kind else { panic!("not area type") };
    t.xf.transform_rect_bbox(frame.bounds().unwrap())
}

/// #2572: the text loaded from an out port threads into a closed path, into a new frame of the
/// first one's shape and size at a point, and into a new rectangle, each in one undo step.
#[test]
fn add_threads_into_a_path_a_frame_at_a_point_and_a_rectangle() {
    let (mut s, a, b) = setup();
    assert_eq!(new_id(s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap()), b);
    assert_eq!(threads(&s), [[a, b]]);
    assert!(!text(&s, b).is_empty(), "the overflow flowed into the path");
    assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), TEXT.repeat(4));
    assert_eq!(s.doc().unwrap().selection.objects, [NodeId(a), NodeId(b)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(threads(&s).is_empty());
    assert!(matches!(s.doc().unwrap().doc.node(NodeId(b)).unwrap().kind, NodeKind::Path { .. }), "undo gives the path back");

    let c = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [400, 200]})).unwrap());
    assert_eq!(frame_box(&s, c), vectorcraft_geom::Rect::new(400.0, 200.0, 550.0, 260.0), "the first frame's size, its top-left at the click");
    let d = new_id(s.execute("text.thread.add", &json!({"from": c, "rect": [10, 100, 300, 150]})).unwrap());
    assert_eq!(frame_box(&s, d), vectorcraft_geom::Rect::new(10.0, 100.0, 310.0, 250.0));
    assert_eq!(threads(&s), [[a, c, d]], "each new frame comes after the one it was threaded from");
    assert_eq!(format!("{}{}{}", text(&s, a), text(&s, c), text(&s, d)), TEXT.repeat(4));
    assert!(!text(&s, d).is_empty());
    // A frame threaded from the middle of a thread goes in after it.
    let e = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [0, 0]})).unwrap());
    assert_eq!(threads(&s), [[a, e, c, d]]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(NodeId(e)).is_none());
    assert_eq!(threads(&s), [[a, c, d]]);
}

/// #2572: the text loaded from an in port threads back: the new frame or the path clicked comes
/// before the frame it was loaded from, so the story starts there and flows on into it; in the
/// middle of a thread the new frame goes in between, each in one undo step.
#[test]
fn add_before_threads_the_story_from_the_new_frame_into_the_clicked_one() {
    let (mut s, a, b) = setup();
    let c = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [400, 200], "before": true})).unwrap());
    assert_eq!(threads(&s), [[c, a]]);
    assert_eq!(frame_box(&s, c), vectorcraft_geom::Rect::new(400.0, 200.0, 550.0, 260.0), "the clicked frame's size at the click");
    assert!(TEXT.repeat(4).starts_with(&text(&s, c)) && !text(&s, c).is_empty(), "the story starts in the new frame: {:?}", text(&s, c));
    assert_eq!(format!("{}{}", text(&s, c), text(&s, a)), TEXT.repeat(4));
    assert_eq!(s.doc().unwrap().selection.objects, [NodeId(a), NodeId(c)]);
    // In between the frame and the one before it.
    let d = new_id(s.execute("text.thread.add", &json!({"from": a, "rect": [10, 100, 300, 150], "before": true})).unwrap());
    assert_eq!(threads(&s), [[c, d, a]]);
    assert_eq!(format!("{}{}{}", text(&s, c), text(&s, d), text(&s, a)), TEXT.repeat(4));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(threads(&s), [[c, a]]);
    // A closed path clicked with the in port's text comes first, with the start of the story.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(new_id(s.execute("text.thread.add", &json!({"from": a, "to": b, "before": true})).unwrap()), b);
    assert_eq!(threads(&s), [[b, a]]);
    assert!(TEXT.repeat(4).starts_with(&text(&s, b)) && !text(&s, b).is_empty());
    assert_eq!(format!("{}{}", text(&s, b), text(&s, a)), TEXT.repeat(4));
    // `before: false` is the out port's.
    let e = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [0, 0], "before": false})).unwrap());
    assert_eq!(threads(&s), [[b, a, e]]);
}

/// Type on a path along a line beside the area type of [`setup`].
fn type_on_a_line(s: &mut Session) -> u64 {
    let line = new_id(s.execute("shape.line", &json!({"x1": 10, "y1": 250, "x2": 110, "y2": 250})).unwrap());
    new_id(s.execute("text.createInPath", &json!({"path": line, "mode": "onPath", "text": ""})).unwrap())
}

/// Type on a path threads as area type does, in any mix: a story flows from area type along a
/// path and on into area type, and a new frame threaded from type on a path at a point is type
/// on a path of the same shape.
#[test]
fn type_on_a_path_threads_with_area_type() {
    let (mut s, a, b) = setup();
    let on_path = type_on_a_line(&mut s);
    s.execute("text.thread.add", &json!({"from": a, "to": on_path})).unwrap();
    s.execute("text.thread.add", &json!({"from": on_path, "to": b})).unwrap();
    assert_eq!(threads(&s), [[a, on_path, b]]);
    let shares = [a, on_path, b].map(|id| text(&s, id));
    assert!(shares.iter().all(|t| !t.is_empty()), "each frame takes a share: {shares:?}");
    assert_eq!(shares.concat(), TEXT.repeat(4));

    let c = new_id(s.execute("text.thread.add", &json!({"from": on_path, "at": [300, 250]})).unwrap());
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(c)).unwrap().kind else { panic!("not text") };
    assert!(matches!(t.kind, TextKind::OnPath { .. }), "type on a path, as the frame it was threaded from");
    assert_eq!(threads(&s), [[a, on_path, c, b]]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(threads(&s), [[a, on_path, b]]);
}

/// Type on a path released from its thread gives its text back to the frame before it.
#[test]
fn type_on_a_path_is_released_from_its_thread() {
    let (mut s, a, _) = setup();
    let on_path = type_on_a_line(&mut s);
    s.execute("text.thread.add", &json!({"from": a, "to": on_path})).unwrap();
    assert!(!text(&s, on_path).is_empty());
    s.execute("select.set", &json!({"ids": [on_path]})).unwrap();
    s.execute("text.thread.releaseSelection", &json!({})).unwrap();
    assert!(threads(&s).is_empty());
    assert!(text(&s, on_path).is_empty());
    assert_eq!(text(&s, a), TEXT.repeat(4));
}

/// Area type threaded into from an out port takes the overflow at once, before its own text.
#[test]
fn add_into_area_type_flows_the_story_into_it() {
    let (mut s, a, _) = setup();
    let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Own text.", "area": {"width": 150, "height": 190}})).unwrap());
    s.execute("text.thread.add", &json!({"from": a, "to": c})).unwrap();
    assert_eq!(threads(&s), [[a, c]]);
    assert_eq!(format!("{}{}", text(&s, a), text(&s, c)), format!("{}Own text.", TEXT.repeat(4)));
    assert!(text(&s, c).len() > "Own text.".len(), "the overflow flowed into the frame: {:?}", text(&s, c));
}

#[test]
fn add_refuses_bad_params_and_leaves_the_document_alone() {
    let (mut s, a, b) = setup();
    let line = new_id(s.execute("shape.line", &json!({"x1": 0, "y1": 280, "x2": 50, "y2": 290})).unwrap());
    let before = s.doc().unwrap().doc.clone();
    for p in [
        json!({"to": b}),
        json!({"from": b, "at": [0, 0]}),
        json!({"from": 9999, "at": [0, 0]}),
        json!({"from": -1, "at": [0, 0]}),
        json!({"from": a}),
        json!({"from": a, "to": b, "at": [0, 0]}),
        json!({"from": a, "to": a}),
        json!({"from": a, "to": line}),
        json!({"from": a, "to": 9999}),
        json!({"from": a, "at": [0]}),
        json!({"from": a, "at": ["1", 2]}),
        json!({"from": a, "rect": [0, 0, -10, 10]}),
        json!({"from": a, "rect": [0, 0, 10, 0]}),
        json!({"from": a, "rect": [0, 0, 10]}),
        json!({"from": a, "rect": "big"}),
        json!({"from": a, "at": [0, 0], "before": "yes"}),
        json!({"from": a, "at": [0, 0], "before": 1}),
    ] {
        assert!(s.execute("text.thread.add", &p).is_err(), "{p} was accepted");
        assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before), "{p} changed the document");
    }
    s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap();
    let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 200, "text": "x", "area": {"width": 50, "height": 20}})).unwrap());
    assert!(s.execute("text.thread.add", &json!({"from": c, "to": b})).is_err(), "a frame already threaded");
}

/// A frame released from a thread that keeps two or more frames gives its text back to them:
/// the story flows through them once.
#[test]
fn release_from_a_longer_thread_keeps_the_story_once() {
    let (mut s, a, b) = setup();
    let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 100, "text": "", "area": {"width": 150, "height": 40}})).unwrap());
    s.execute("text.thread.add", &json!({"from": a, "to": c})).unwrap();
    s.execute("text.thread.add", &json!({"from": c, "to": b})).unwrap();
    assert_eq!(threads(&s), [[a, c, b]]);
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("text.thread.releaseSelection", &json!({})).unwrap();
    assert_eq!(threads(&s), [[a, b]]);
    assert!(text(&s, c).is_empty());
    assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), TEXT.repeat(4));
}

/// A thread of `n` frames (2 or 3), each showing part of the story: area type `a` of [`setup`],
/// for 3 an area type frame that is empty until it is threaded, then the rectangle `b`.
fn thread_of(n: usize) -> (Session, Vec<u64>) {
    let (mut s, a, b) = setup();
    let frames = if n == 3 {
        let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 100, "text": "", "area": {"width": 150, "height": 40}})).unwrap());
        s.execute("text.thread.add", &json!({"from": a, "to": c})).unwrap();
        s.execute("text.thread.add", &json!({"from": c, "to": b})).unwrap();
        vec![a, c, b]
    } else {
        s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap();
        vec![a, b]
    };
    assert_eq!(threads(&s), std::slice::from_ref(&frames));
    assert!(frames.iter().all(|id| !text(&s, *id).is_empty()), "each frame shows part of the story");
    (s, frames)
}

/// Convert To Point Type on a frame of threaded text takes it out of its thread with the text
/// it holds, and the other frames keep the text they hold: the story holds each character once.
/// A thread left with one frame is no thread. Undo threads the frame again.
#[test]
fn a_frame_converted_to_point_type_leaves_its_thread_with_its_text() {
    // Soft wraps of the point type become line breaks, so its characters are compared without
    // white space.
    let squeeze = |t: &str| t.split_whitespace().collect::<String>();
    for n in [2, 3] {
        for k in 0..n {
            let (mut s, frames) = thread_of(n);
            let shown: Vec<String> = frames.iter().map(|id| text(&s, *id)).collect();
            let id = frames[k];
            s.execute("type.convertToPointType", &json!({"ids": [id]})).unwrap();
            let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!("not text") };
            assert!(matches!(t.kind, TextKind::Point), "{n} frames, frame {k}: point type");
            assert_eq!(squeeze(&text(&s, id)), squeeze(&shown[k]), "{n} frames, frame {k}: the point type keeps its text");
            for (f, before) in frames.iter().zip(&shown).filter(|(f, _)| **f != id) {
                assert_eq!(text(&s, *f), *before, "{n} frames, frame {k}: frame {f} keeps its text");
            }
            let rest: Vec<u64> = frames.iter().copied().filter(|f| *f != id).collect();
            assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{n} frames, frame {k}");
            let all: String = frames.iter().map(|f| text(&s, *f)).collect();
            assert_eq!(squeeze(&all), squeeze(&TEXT.repeat(4)), "{n} frames, frame {k}: each character once");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(threads(&s), std::slice::from_ref(&frames));
            assert_eq!(frames.iter().map(|f| text(&s, *f)).collect::<Vec<_>>(), shown, "{n} frames, frame {k}: undo");
        }
    }
}

#[test]
fn release_and_remove() {
    let (mut s, a, b) = setup();
    s.execute("text.thread.create", &json!({})).unwrap();
    let tb = text(&s, b);
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("text.thread.remove", &json!({})).unwrap();
    assert_eq!(text(&s, b), tb, "remove keeps what each frame shows");
    assert!(s.doc().unwrap().doc.text_threads.is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("text.thread.releaseSelection", &json!({})).unwrap();
    assert_eq!(text(&s, b), "");
    assert_eq!(text(&s, a), TEXT.repeat(4), "the story returns to the remaining frame");
}

/// `story` threaded through three area type frames, of 150 by 60, 40 and `last` points, each
/// showing part of it.
fn three_frames(story: &str, last: f64) -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let frame = |s: &mut Session, x: f64, y: f64, h: f64, text: &str| {
        new_id(s.execute("text.create", &json!({"x": x, "y": y, "text": text, "area": {"width": 150, "height": h}})).unwrap())
    };
    let a = frame(&mut s, 10.0, 10.0, 60.0, story);
    let b = frame(&mut s, 10.0, 100.0, 40.0, "");
    let c = frame(&mut s, 200.0, 10.0, last, "");
    s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap();
    s.execute("text.thread.add", &json!({"from": b, "to": c})).unwrap();
    assert_eq!(threads(&s), [[a, b, c]]);
    assert!([a, b, c].iter().all(|id| !text(&s, *id).is_empty()), "each frame shows part of the story");
    (s, [a, b, c])
}

fn continues(s: &Session, id: u64) -> bool {
    text_of(&s.doc().unwrap().doc, NodeId(id)).is_some_and(|t| t.first_para_continues)
}

/// A double-click on a threaded port breaks the thread there: the story flows into the frames
/// before the break and the frames after it are left empty, still threaded with each other
/// when two or more. One undo step restores the thread and its text.
#[test]
fn break_splits_the_thread_and_the_story_flows_into_the_first_part() {
    let story = TEXT.repeat(12);
    let (mut s, [a, b, c]) = three_frames(&story, 40.0);
    let d = new_id(s.execute("text.create", &json!({"x": 200, "y": 100, "text": "", "area": {"width": 150, "height": 100}})).unwrap());
    s.execute("text.thread.add", &json!({"from": c, "to": d})).unwrap();
    let all = [a, b, c, d];
    let shown: Vec<String> = all.iter().map(|id| text(&s, *id)).collect();
    assert!(shown.iter().all(|t| !t.is_empty()), "each frame shows part of the story");
    let joined = |s: &Session, ids: &[u64]| ids.iter().map(|id| text(s, *id)).collect::<String>();
    // (frame, port, threads after, the frames that hold the story, the frames left empty)
    let cases = [
        (a, "out", vec![vec![b, c, d]], vec![a], vec![b, c, d]),
        (b, "in", vec![vec![b, c, d]], vec![a], vec![b, c, d]),
        (b, "out", vec![vec![a, b], vec![c, d]], vec![a, b], vec![c, d]),
        (d, "in", vec![vec![a, b, c]], vec![a, b, c], vec![d]),
        (c, "out", vec![vec![a, b, c]], vec![a, b, c], vec![d]),
    ];
    for (id, port, after, full, empty) in cases {
        let case = format!("{port} port of {id}");
        s.execute("text.thread.break", &json!({"id": id, "port": port})).unwrap();
        assert_eq!(threads(&s), after, "{case}");
        assert_eq!(joined(&s, &full), story, "{case}: the story flows into the first part");
        assert!(empty.iter().all(|id| text(&s, *id).is_empty()), "{case}: the frames after the break are empty");
        assert!(!continues(&s, empty[0]), "{case}: the first frame after the break starts a paragraph");
        assert!(!continues(&s, a), "{case}: the first frame starts the story");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(threads(&s), [all.to_vec()], "{case}: undo");
        assert_eq!(all.iter().map(|id| text(&s, *id)).collect::<Vec<_>>(), shown, "{case}: undo");
    }
}

#[test]
fn break_refuses_a_port_with_no_thread_and_bad_params() {
    let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(4), 200.0);
    let lone = new_id(s.execute("text.create", &json!({"x": 400, "y": 10, "text": "Alone.", "area": {"width": 100, "height": 100}})).unwrap());
    let ids = [a, b, c, lone];
    let shown: Vec<String> = ids.iter().map(|id| text(&s, *id)).collect();
    let before = threads(&s);
    for params in [
        json!({"id": a, "port": "in"}),
        json!({"id": c, "port": "out"}),
        json!({"id": lone, "port": "out"}),
        json!({"id": lone, "port": "in"}),
        json!({"id": a, "port": "up"}),
        json!({"id": a}),
        json!({"port": "out"}),
        json!({"id": 9999, "port": "out"}),
    ] {
        assert!(s.execute("text.thread.break", &params).is_err(), "{params}");
    }
    assert_eq!(threads(&s), before);
    assert_eq!(ids.iter().map(|id| text(&s, *id)).collect::<Vec<_>>(), shown);
}

/// Whether the document holds area type or type on a path `id`.
fn is_frame(s: &Session, id: u64) -> bool {
    text_of(&s.doc().unwrap().doc, NodeId(id)).is_some()
}

/// Create Outlines and Object › Expand on frames of threaded text take them out of their
/// thread with the text they show, which their outlines draw, and the other frames keep the
/// text they hold: each character is in the document once. A thread left with one frame is no
/// thread. Undo threads the frames again.
#[test]
fn outlined_frames_leave_their_thread_with_their_text() {
    for command in ["type.createOutlines", "object.expand"] {
        for out in [&[0][..], &[1], &[2], &[0, 2]] {
            let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
            let shown = frames.map(|id| text(&s, id));
            assert_eq!(shown.concat(), TEXT.repeat(4));
            let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
            s.execute("select.set", &json!({"ids": ids})).unwrap();
            s.execute(command, &json!({})).unwrap();
            let rest: Vec<u64> = frames.iter().copied().filter(|id| !ids.contains(id)).collect();
            for id in &ids {
                assert!(!is_frame(&s, *id), "{command} {out:?}: frame {id} is outlined");
            }
            let st = s.doc().unwrap();
            let groups = st.selection.objects.iter().filter(|g| matches!(st.doc.node(**g).map(|n| &n.kind), Some(NodeKind::Group { .. })));
            assert_eq!(groups.count(), ids.len(), "{command} {out:?}: the outlines are selected");
            for (id, before) in frames.iter().zip(&shown).filter(|(id, _)| rest.contains(id)) {
                assert_eq!(text(&s, *id), *before, "{command} {out:?}: frame {id} keeps its text");
            }
            assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{command} {out:?}");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(threads(&s), [frames], "{command} {out:?}: undo");
            assert_eq!(frames.map(|id| text(&s, id)), shown, "{command} {out:?}: undo");
        }
    }
}

/// Expand Appearance outlines a frame with an effect, and the frame leaves its thread with the
/// text it shows; Object › Expand of such a frame expands its appearance first.
#[test]
fn a_frame_whose_appearance_expands_leaves_its_thread_with_its_text() {
    for command in ["effect.expandAppearance", "object.expand"] {
        let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(4), 200.0);
        s.execute("effect.apply", &json!({"effect": "distort.roughen", "ids": [b]})).unwrap();
        let shown = [a, b, c].map(|id| text(&s, id));
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute(command, &json!({})).unwrap();
        assert!(!is_frame(&s, b), "{command}: the frame is outlined");
        assert_eq!((text(&s, a), text(&s, c)), (shown[0].clone(), shown[2].clone()), "{command}: the other frames keep their text");
        assert_eq!(threads(&s), [[a, c]], "{command}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(threads(&s), [[a, b, c]], "{command}: undo");
        assert_eq!([a, b, c].map(|id| text(&s, id)), shown, "{command}: undo");
    }
}

/// Object › Flatten Transparency outlines frames of threaded text with Convert All Text to
/// Outlines, and turns transparent frames into flat regions or an image: each such frame leaves
/// its thread with the text it shows, and the other frames keep the text they hold. Undo threads
/// the frames again. When the last frame is flattened, the frame before it takes the text it has
/// no room for.
#[test]
fn flattened_frames_leave_their_thread_with_their_text() {
    let cases = [(json!({"textToOutlines": true}), false), (json!({}), true), (json!({"balance": 0}), true)];
    for (options, transparent) in cases {
        for out in [&[0][..], &[1], &[2], &[0, 2]] {
            let case = format!("{options} {out:?}");
            let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
            let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
            if transparent {
                s.execute("transparency.set", &json!({"ids": ids, "opacity": 50})).unwrap();
            }
            let shown = frames.map(|id| text(&s, id));
            assert_eq!(shown.concat(), TEXT.repeat(4));
            let mut p = options.clone();
            p["ids"] = json!(ids);
            s.execute("object.flattenTransparency", &p).unwrap();
            let rest: Vec<u64> = frames.iter().copied().filter(|id| !ids.contains(id)).collect();
            for id in &ids {
                assert!(!is_frame(&s, *id), "{case}: frame {id} is flattened");
            }
            for (id, before) in frames.iter().zip(&shown).filter(|(id, _)| rest.contains(id)) {
                assert_eq!(text(&s, *id), *before, "{case}: frame {id} keeps its text");
            }
            assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{case}");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(threads(&s), [frames], "{case}: undo");
            assert_eq!(frames.map(|id| text(&s, id)), shown, "{case}: undo");
        }
    }
    let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
    let shown = [a, b, c].map(|id| text(&s, id));
    let laid = {
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
    };
    assert!(laid.overflow, "the last frame has more text than it shows");
    let end = laid.lines.last().map_or(0, |l| l.end);
    let hidden = shown[2][end..].trim_start_matches(' ');
    s.execute("object.flattenTransparency", &json!({"ids": [c], "textToOutlines": true})).unwrap();
    assert_eq!(text(&s, a), shown[0]);
    assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]), "the frame before it keeps its text and takes the rest");
    assert_eq!(threads(&s), [[a, b]]);
}

/// Expand Appearance keeps type whose only effect is a shadow or Crop Marks as type under a new
/// id, in a group under the frame's id: the type takes the frame's place in its thread, and
/// each frame keeps its text, the last frame the text it has no room for too. Type in a group
/// with Crop Marks keeps its id and stays in its thread.
#[test]
fn type_that_expand_appearance_keeps_takes_its_frames_place() {
    for (effect, grouped) in [("stylize.dropShadow", false), ("cropMarks", false), ("cropMarks", true)] {
        for k in [1, 2] {
            let case = format!("{effect} grouped {grouped} frame {k}");
            let (mut s, frames) = three_frames(&TEXT.repeat(12), 100.0);
            let id = frames[k];
            s.execute("select.set", &json!({"ids": [id]})).unwrap();
            let target = if grouped { new_id(s.execute("object.group", &json!({})).unwrap()) } else { id };
            s.execute("effect.apply", &json!({"effect": effect, "item": null, "ids": [target]})).unwrap();
            let shown = frames.map(|id| text(&s, id));
            s.execute("select.set", &json!({"ids": [target]})).unwrap();
            s.execute("effect.expandAppearance", &json!({})).unwrap();
            let mut types = vec![];
            if let Some(n) = s.doc().unwrap().doc.node(NodeId(target)) {
                n.walk(&mut |m| {
                    if matches!(m.kind, NodeKind::Text(_)) {
                        types.push(m.id.0);
                    }
                });
            }
            let [kept] = types[..] else { panic!("{case}: the type is kept: {types:?}") };
            assert_eq!(kept == id, grouped, "{case}: the type has a new id unless it was in a group");
            let mut now = frames;
            now[k] = kept;
            assert_eq!(threads(&s), [now], "{case}: the type takes the frame's place");
            assert_eq!(now.map(|id| text(&s, id)), shown, "{case}: each frame keeps its text");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(threads(&s), [frames], "{case}: undo");
            assert_eq!(frames.map(|id| text(&s, id)), shown, "{case}: undo");
        }
    }
}

/// The text that does not fit a thread's last frame, which its outlines do not draw, stays in
/// the story: when the last frame is outlined, the frame before it takes that text after its
/// own.
#[test]
fn outlining_the_last_frame_keeps_the_text_it_has_no_room_for() {
    let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
    let shown = [a, b, c].map(|id| text(&s, id));
    let laid = {
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
    };
    assert!(laid.overflow, "the last frame has more text than it shows");
    let end = laid.lines.last().map_or(0, |l| l.end);
    let hidden = shown[2][end..].trim_start_matches(' ');
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("type.createOutlines", &json!({})).unwrap();
    assert_eq!(text(&s, a), shown[0]);
    assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]), "the frame before it keeps its text and takes the rest");
    assert_eq!(threads(&s), [[a, b]]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!([a, b, c].map(|id| text(&s, id)), shown);
}

/// Object › Rasterize on frames of threaded text takes them out of their thread with the text
/// they show, which the image draws, and the other frames keep the text they hold. Undo threads
/// the frames again. When the last frame is rasterized, the frame before it takes the text it
/// has no room for.
#[test]
fn rasterized_frames_leave_their_thread_with_their_text() {
    for clip in [false, true] {
        for out in [&[0][..], &[1], &[2], &[0, 2]] {
            let case = format!("clippingMask {clip} {out:?}");
            let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
            let shown = frames.map(|id| text(&s, id));
            assert_eq!(shown.concat(), TEXT.repeat(4));
            let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
            s.execute("select.set", &json!({"ids": ids})).unwrap();
            s.execute("object.rasterize", &json!({"ppi": 36, "clippingMask": clip})).unwrap();
            let rest: Vec<u64> = frames.iter().copied().filter(|id| !ids.contains(id)).collect();
            for id in &ids {
                assert!(!is_frame(&s, *id), "{case}: frame {id} is rasterized");
            }
            for (id, before) in frames.iter().zip(&shown).filter(|(id, _)| rest.contains(id)) {
                assert_eq!(text(&s, *id), *before, "{case}: frame {id} keeps its text");
            }
            assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{case}");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(threads(&s), [frames], "{case}: undo");
            assert_eq!(frames.map(|id| text(&s, id)), shown, "{case}: undo");
        }
    }
    let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
    let shown = [a, b, c].map(|id| text(&s, id));
    let laid = {
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
    };
    assert!(laid.overflow, "the last frame has more text than it shows");
    let end = laid.lines.last().map_or(0, |l| l.end);
    let hidden = shown[2][end..].trim_start_matches(' ');
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("object.rasterize", &json!({"ppi": 36})).unwrap();
    assert_eq!(text(&s, a), shown[0]);
    assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]), "the frame before it keeps its text and takes the rest");
    assert_eq!(threads(&s), [[a, b]]);
}

/// Frame `id`'s first line: whether its first paragraph continues from the frame before, where
/// its text starts from the frame's left edge, and how many list markers it starts with.
fn first_line(s: &Session, id: u64) -> (bool, f64, usize) {
    let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(id)) else { panic!("not area type") };
    let TextKind::Area { frame } = &t.kind else { panic!("not area type") };
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let l = &lay.lines[0];
    let x = lay.glyphs[l.glyph_start..l.glyph_end].iter().find(|g| g.len > 0).map_or(f64::NAN, |g| g.origin.x);
    let markers = lay.markers.iter().filter(|m| lay.glyphs[m.glyphs.start].line == 0).count();
    (t.first_para_continues, (x - frame.bounds().unwrap().x0 - 1e-9).round(), markers)
}

/// A paragraph's first-line indent and bullet belong to its first line, in the frame it starts
/// in: the frame it continues into starts at its left indent, without the bullet. Re-flowing,
/// saving and opening keep it. In a frame that is no longer threaded and in a copy of a frame,
/// the first paragraph has its first-line indent and bullet.
#[test]
fn a_paragraph_continued_into_the_next_frame_starts_it_without_its_first_line_indent_or_bullet() {
    let (mut s, a, b) = setup();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b], "list": "bullet"})).unwrap();
    assert_eq!((first_line(&s, a), first_line(&s, b)), ((false, 18.0, 1), (true, 18.0, 0)));
    s.execute("text.setFormat", &json!({"ids": [a, b], "list": "none", "leftIndent": 0, "firstLineIndent": 24})).unwrap();
    assert_eq!((first_line(&s, a), first_line(&s, b)), ((false, 24.0, 0), (true, 0.0, 0)));
    assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), TEXT.repeat(4));
    // Typing re-flows the story and keeps it.
    s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "NEW "})).unwrap();
    assert_eq!(first_line(&s, b), (true, 0.0, 0));
    // Saved and opened: a native file re-flows its threads, a restored document keeps its text.
    for name in ["thread.vectorcraft", "thread.eps"] {
        let bytes = if name.ends_with(".eps") {
            let r = s.execute("document.exportEps", &json!({})).unwrap();
            vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
        } else {
            vectorcraft_format::save(&s.doc().unwrap().doc, false)
        };
        let r = s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
        assert_eq!(r["restored"], name.ends_with(".eps"), "{r}");
        assert_eq!(threads(&s), [[a, b]], "{name}");
        assert_eq!((first_line(&s, a), first_line(&s, b)), ((false, 24.0, 0), (true, 0.0, 0)), "{name}");
    }
    // A copy of the second frame is in no thread, and starts with the indent.
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let copy = s.execute("edit.duplicate", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    assert_eq!((first_line(&s, b), first_line(&s, copy)), ((true, 0.0, 0), (false, 24.0, 0)));
    // …and so is a pasted one, and each copy's cached bounds and baselines are its own layout's.
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let pasted = s.execute("edit.paste", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    let dragged = s.execute("layer.move", &json!({"ids": [b], "target": a, "place": "above", "copy": true})).unwrap()["ids"][0].as_u64().unwrap();
    assert_eq!((first_line(&s, pasted), first_line(&s, dragged)), ((false, 24.0, 0), (false, 24.0, 0)));
    // Every text object in the document has the cached bounds and baselines of its own layout.
    let own_caches = |s: &Session, what: &str| {
        let mut texts = vec![];
        s.doc().unwrap().doc.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                texts.push((n.id, t));
            }
        });
        for (id, t) in texts {
            let mut fresh = (**t).clone();
            refresh_bounds(&mut fresh);
            assert!(t.cached_bounds.is_some() && t.cached_bounds == fresh.cached_bounds, "{what}: {id:?}");
            assert_eq!(t.cached_baselines, fresh.cached_baselines, "{what}: {id:?}");
        }
    };
    own_caches(&s, "copies");
    // Release to Layers (Build), Release Opacity Mask, Edit Opacity Mask and Draw Inside copy the
    // frame too.
    let layer = s.doc().unwrap().doc.layer_containing(NodeId(b)).unwrap().0;
    s.execute("layer.releaseToLayers", &json!({"id": layer, "build": true})).unwrap();
    own_caches(&s, "Release to Layers (Build)");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({"ids": [a, b]})).unwrap();
    s.execute("transparency.releaseOpacityMask", &json!({})).unwrap();
    own_caches(&s, "Release Opacity Mask");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("transparency.editOpacityMask", &json!({"id": a})).unwrap();
    own_caches(&s, "Edit Opacity Mask");
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    for _ in 0..3 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("view.drawMode", &json!({"mode": "inside"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 210, "y": 20, "width": 20, "height": 20})).unwrap();
    own_caches(&s, "Draw Inside");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("view.drawMode", &json!({"mode": "normal"})).unwrap();
    assert_eq!((threads(&s), first_line(&s, b)), (vec![vec![a, b]], (true, 0.0, 0)));
    // Remove Threading: each frame keeps its text, and its first paragraph starts in it.
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("text.thread.remove", &json!({})).unwrap();
    assert_eq!(first_line(&s, b), (false, 24.0, 0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(first_line(&s, b), (true, 0.0, 0));
    // Released, the frame is empty and starts the next text typed into it as a paragraph.
    let continues = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.first_para_continues,
        _ => panic!("not text"),
    };
    s.execute("text.thread.releaseSelection", &json!({})).unwrap();
    assert!(text(&s, b).is_empty() && !continues(&s, b));
    s.execute("edit.undo", &json!({})).unwrap();
    // With the first frame released, the second one holds the whole story and starts it.
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("text.thread.releaseSelection", &json!({})).unwrap();
    assert!(threads(&s).is_empty() && text(&s, a).is_empty());
    assert_eq!(first_line(&s, b), (false, 24.0, 0));
    s.execute("edit.undo", &json!({})).unwrap();
    // Converted to point type, it is in no thread, and its first line starts at the indent where
    // the frame's left edge was.
    let left = frame_box(&s, b).x0;
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("type.convertToPointType", &json!({})).unwrap();
    assert!(!continues(&s, b));
    {
        let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(b)).unwrap().kind else { panic!("not text") };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let g = lay.glyphs.iter().find(|g| g.len > 0).unwrap();
        let x = (t.xf * g.origin).x - left;
        assert!((x - 24.0).abs() < 1e-6, "{x}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(first_line(&s, b), (true, 0.0, 0));
    // Text that ends the first frame with a paragraph break: the second frame starts a
    // paragraph, with its indent.
    let all = text(&s, a).len();
    s.execute("text.editRange", &json!({"id": a, "start": 0, "end": all, "insert": "Short line.\n".repeat(12)})).unwrap();
    assert!(text(&s, a).ends_with('\n'), "{:?}", text(&s, a));
    assert_eq!(first_line(&s, b), (false, 24.0, 0));
}

/// When Create Outlines takes frames out of a thread, a frame that now starts its thread or is
/// in no thread starts its paragraph with the first-line indent; a frame after one that ends
/// inside the paragraph still continues it. The frames that stay hold each character they held
/// once, and keep their text when no mark changes. Undo marks the frames again.
#[test]
fn a_frame_that_starts_its_thread_after_outlining_has_its_first_line_indent() {
    for (out, want) in [
        (&[0][..], [None, Some(false), Some(true)]),
        (&[0, 2], [None, Some(false), None]),
        (&[1], [Some(false), None, Some(true)]),
        (&[2], [Some(false), Some(true), None]),
    ] {
        let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
        s.execute("text.setFormat", &json!({"ids": frames, "leftIndent": 0, "firstLineIndent": 24})).unwrap();
        let before = frames.map(|id| first_line(&s, id));
        assert_eq!(before, [(false, 24.0, 0), (true, 0.0, 0), (true, 0.0, 0)], "{out:?}");
        let shown = frames.map(|id| text(&s, id));
        assert_eq!(shown.concat(), TEXT.repeat(4), "{out:?}: the last frame has room for the rest");
        let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        s.execute("type.createOutlines", &json!({})).unwrap();
        let stay: Vec<usize> = (0..3).filter(|k| want[*k].is_some()).collect();
        for k in &stay {
            let on = want[*k] == Some(true);
            let indent = if on { 0.0 } else { 24.0 };
            assert_eq!(first_line(&s, frames[*k]), (on, indent, 0), "{out:?}: frame {k}");
        }
        let held: String = stay.iter().map(|k| shown[*k].as_str()).collect();
        assert_eq!(stay.iter().map(|k| text(&s, frames[*k])).collect::<String>(), held, "{out:?}: each character once");
        if stay.iter().all(|k| want[*k] == Some(before[*k].0)) {
            for k in &stay {
                assert_eq!(text(&s, frames[*k]), shown[*k], "{out:?}: frame {k} keeps its text");
            }
        }
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(frames.map(|id| first_line(&s, id)), before, "{out:?}: undo");
    }
}

/// The text that a thread's last frame has no room for is measured as the frame lays it out,
/// without the first-line indent of the paragraph it continues: when the frame is outlined, the
/// frame before it takes only that text.
#[test]
fn outlining_a_continued_last_frame_keeps_only_the_text_it_has_no_room_for() {
    let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
    s.execute("text.setFormat", &json!({"ids": [a, b, c], "leftIndent": 0, "firstLineIndent": 72})).unwrap();
    assert_eq!(first_line(&s, c), (true, 0.0, 0));
    let shown = [a, b, c].map(|id| text(&s, id));
    let laid = {
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
    };
    assert!(laid.overflow, "the last frame has more text than it shows");
    let end = laid.lines.last().map_or(0, |l| l.end);
    let hidden = shown[2][end..].trim_start_matches(' ');
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("type.createOutlines", &json!({})).unwrap();
    assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]));
}

/// The text that a thread's last frame has no room for is measured with the frame's numbered
/// paragraphs counted on from the frames before it: when the frame is outlined, the frame before
/// it takes only that text.
#[test]
fn outlining_a_numbered_last_frame_keeps_only_the_text_it_has_no_room_for() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let items: String = (1..=7).map(|i| format!("Item {i}\n")).collect();
    let story = format!("{items}{}", TEXT.repeat(4));
    let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 106}})).unwrap());
    let c = new_id(s.execute("text.create", &json!({"x": 200, "y": 10, "text": "", "area": {"width": 150, "height": 20}})).unwrap());
    s.execute("text.thread.add", &json!({"from": a, "to": c})).unwrap();
    // A hanging indent that "I." fits and "VIII." does not: the text after "VIII." starts at
    // the next tab stop.
    s.execute("text.setFormat", &json!({"ids": [a, c], "number": "upperRoman", "leftIndent": 14, "firstLineIndent": -14})).unwrap();
    let shown = [a, c].map(|id| text(&s, id));
    assert!(shown[0].ends_with("Item 7\n") && shown[1].starts_with("The quick"), "{shown:?}");
    let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)).cloned() else { panic!("not area type") };
    assert_eq!(t.list_count, 7);
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert_eq!(laid.markers.first().map(|m| m.text.as_str()), Some("VIII."));
    assert!(laid.overflow, "the last frame has more text than it shows");
    let end = laid.lines.last().map_or(0, |l| l.end);
    let from_one = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &TextObject { list_count: 0, ..t });
    assert_ne!(from_one.lines.last().map_or(0, |l| l.end), end, "counted from 1, the frame would show other text");
    let hidden = shown[1][end..].trim_start_matches(' ');
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("type.createOutlines", &json!({})).unwrap();
    assert_eq!(text(&s, a), format!("{}{hidden}", shown[0]));
}

/// A story of one-line paragraphs that each start with a quotation mark, threaded from area
/// type `a` into the rectangle `b`, which shows the paragraphs that `a` has no room for.
fn quoted_thread() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let story = "“Quoted.”\n".repeat(20);
    let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 60}})).unwrap());
    let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap());
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    assert!(text(&s, b).starts_with('“'));
    (s, a, b)
}

/// Frame `id`'s Optical Margin Alignment setting, and whether its glyphs start left of its frame
/// (its lines' opening quotation marks hang).
fn optical(s: &Session, id: u64) -> (bool, bool) {
    let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(id)) else { panic!("not area type") };
    let TextKind::Area { frame } = &t.kind else { panic!("not area type") };
    (t.optical_margin, t.cached_bounds.unwrap().x0 < frame.bounds().unwrap().x0 - 1.0)
}

/// Optical Margin Alignment is a setting of the whole thread: Type › Optical Margin Alignment
/// in any frame sets it in every frame of the thread, in one undo step, and the lines in every
/// frame hang. Typing keeps it, and frames taken out of the thread keep it.
#[test]
fn optical_margin_alignment_is_set_for_every_frame_of_a_thread() {
    let (mut s, a, b) = quoted_thread();
    assert_eq!((optical(&s, a), optical(&s, b)), ((false, false), (false, false)));
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let r = s.execute("type.opticalMarginAlignment", &json!({})).unwrap();
    assert_eq!(r, json!({"on": true, "ids": [a, b]}));
    assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)), "the second frame's lines hang too");
    // Turned off from the second frame, it is off in both, and one undo turns it back on in both.
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    assert_eq!(s.execute("type.opticalMarginAlignment", &json!({})).unwrap()["on"], false);
    assert_eq!((optical(&s, a), optical(&s, b)), ((false, false), (false, false)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)));
    // Typing in the first frame re-flows the story and keeps the setting.
    s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "“New.”\n"})).unwrap();
    assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)));
    // Any edit that sets one frame sets the thread.
    s.edit("Set", |d, _| {
        let Some(NodeKind::Text(t)) = d.node_mut(NodeId(b)).map(|n| &mut n.kind) else { panic!("not text") };
        t.optical_margin = false;
        refresh_bounds(t);
        Ok(())
    })
    .unwrap();
    assert_eq!((optical(&s, a), optical(&s, b)), ((false, false), (false, false)));
    s.execute("edit.undo", &json!({})).unwrap();
    // Released or unthreaded, each frame keeps it.
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("text.thread.releaseSelection", &json!({})).unwrap();
    assert_eq!((optical(&s, a).0, optical(&s, b).0), (true, true));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("text.thread.remove", &json!({})).unwrap();
    assert!(threads(&s).is_empty());
    assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)));
}

/// Threading type into a thread gives the joined thread the setting of its first frame.
#[test]
fn a_thread_takes_its_first_frames_optical_margin_alignment() {
    let (mut s, a, b) = quoted_thread();
    s.execute("text.thread.remove", &json!({})).unwrap();
    s.execute("type.opticalMarginAlignment", &json!({"ids": [a], "on": true})).unwrap();
    assert_eq!((optical(&s, a).0, optical(&s, b).0), (true, false));
    // Threaded again in stacking order: the second frame takes the first one's setting.
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((optical(&s, a).0, optical(&s, b).0), (true, false));
    // From `b`'s out port into `a`: `a` joins `b`'s thread after it and takes its setting.
    s.execute("text.thread.add", &json!({"from": b, "to": a})).unwrap();
    assert_eq!(threads(&s), [[b, a]]);
    assert_eq!((optical(&s, a).0, optical(&s, b).0), (false, false));
    // A new frame threaded from a frame takes the thread's setting.
    s.execute("type.opticalMarginAlignment", &json!({"ids": [a], "on": true})).unwrap();
    let c = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [400, 10]})).unwrap());
    assert_eq!([a, b, c].map(|id| optical(&s, id).0), [true; 3]);
}

/// A file saved with the frames of one thread set differently opens with the first frame's
/// setting in every frame: a native file, and an EPS file restored from the document it carries.
#[test]
fn a_thread_saved_with_differing_frames_opens_with_the_first_frames_setting() {
    let (mut s, a, b) = quoted_thread();
    for (first, name) in [(true, "old.vectorcraft"), (false, "old.vectorcraft"), (true, "old.eps"), (false, "old.eps")] {
        for (id, on) in [(a, first), (b, !first)] {
            let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
            let Some(NodeKind::Text(t)) = doc.node_mut(NodeId(id)).map(|n| &mut n.kind) else { panic!("not text") };
            t.optical_margin = on;
        }
        let bytes = if name.ends_with(".eps") {
            let r = s.execute("document.exportEps", &json!({})).unwrap();
            vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
        } else {
            vectorcraft_format::save(&s.doc().unwrap().doc, false)
        };
        let r = s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
        assert_eq!(r["restored"], name.ends_with(".eps"), "{r}");
        assert_eq!(threads(&s), [[a, b]], "{name}");
        assert_eq!((optical(&s, a), optical(&s, b)), ((first, first), (first, first)), "{name}, first frame {first}");
    }
}

/// One count runs through a thread: the second frame's numbered paragraphs go on from the
/// first frame's, also after an edit, a save and a reopen. A pasted or duplicated copy of a
/// frame, a frame converted to point type, the frames of a thread that Remove Threading breaks
/// and the frame a Release Selection leaves alone count from 1.
#[test]
fn numbered_list_paragraphs_count_on_through_a_thread() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let items = (1..=10).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
    let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": items, "area": {"width": 150, "height": 60}})).unwrap());
    let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap());
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
    let numbers = |s: &Session, id: u64| {
        let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).markers.iter().map(|m| m.text.clone()).collect::<Vec<_>>()
    };
    let all = |from: usize| (from..=10).map(|n| format!("{n}.")).collect::<Vec<_>>();
    let first = numbers(&s, a);
    assert!((2..10).contains(&first.len()), "both frames hold items: {first:?}");
    assert_eq!([first.clone(), numbers(&s, b)].concat(), all(1));
    // Typing a paragraph into the first frame numbers it and moves the rest on.
    s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "Item 0\n"})).unwrap();
    assert_eq!([numbers(&s, a), numbers(&s, b)].concat(), [vec!["1.".to_string()], all(2), vec!["11.".to_string()]].concat());
    s.execute("edit.undo", &json!({})).unwrap();
    let reopened = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Some(NodeKind::Text(back)) = reopened.node(NodeId(b)).map(|n| &n.kind) else { panic!("text") };
    let after: Vec<String> = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), back).markers.iter().map(|m| m.text.clone()).collect();
    assert_eq!(after, all(first.len() + 1), "reopened");
    // A pasted copy of the second frame is in no thread.
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let copy = s.doc().unwrap().selection.objects.first().copied().unwrap();
    assert_ne!(copy, NodeId(b));
    assert_eq!(numbers(&s, copy.0).first().map(String::as_str), Some("1."));
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let duplicate = s.execute("edit.duplicate", &json!({"dx": 10})).unwrap()["ids"][0].as_u64().unwrap();
    assert_eq!(numbers(&s, duplicate).first().map(String::as_str), Some("1."), "a duplicate");
    let layer = s.doc().unwrap().doc.layers[0].id.0;
    let moved = s.execute("layer.move", &json!({"ids": [b], "target": layer, "copy": true})).unwrap()["ids"][0].as_u64().unwrap();
    assert_eq!(numbers(&s, moved).first().map(String::as_str), Some("1."), "a copy dragged in the Layers panel");
    assert_eq!(numbers(&s, b), all(first.len() + 1), "the frame itself");
    // Convert To Point Type takes the second frame out of the thread with its text.
    s.execute("type.convertToPointType", &json!({"ids": [b]})).unwrap();
    assert!(threads(&s).is_empty());
    assert_eq!(numbers(&s, b).first().map(String::as_str), Some("1."), "the frame converted to point type");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(numbers(&s, b), all(first.len() + 1), "undo");
    // Remove Threading: each frame is a story of its own.
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("text.thread.remove", &json!({})).unwrap();
    assert!(threads(&s).is_empty());
    assert_eq!(numbers(&s, b).first().map(String::as_str), Some("1."));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(numbers(&s, b), all(first.len() + 1), "undo");
    // Release Selection of the first frame: the second one holds the whole story, from 1.
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("text.thread.releaseSelection", &json!({})).unwrap();
    assert!(threads(&s).is_empty());
    assert_eq!(numbers(&s, b), all(1), "the frame left holds the story");
}

/// New Symbol from a numbered thread keeps each frame's count in the symbol's art, so the art
/// looks as it did; Break Link puts the frames in no thread, and each counts from 1.
#[test]
fn a_symbol_made_of_a_numbered_thread_keeps_its_numbers() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let items = (1..=10).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
    let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": items, "area": {"width": 150, "height": 60}})).unwrap());
    let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap());
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
    let count = |t: &TextObject| t.list_count;
    let carried = text_of(&s.doc().unwrap().doc, NodeId(b)).map(count).unwrap();
    assert!(carried > 0);
    let instance = new_id(s.execute("symbol.new", &json!({"name": "Numbered"})).unwrap());
    let counts = |n: &Node| {
        let mut counts = vec![];
        n.walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind {
                counts.push(t.list_count);
            }
        });
        counts.sort_unstable();
        counts
    };
    let doc = &s.doc().unwrap().doc;
    assert_eq!(counts(&doc.symbols.iter().find(|x| x.name == "Numbered").unwrap().art), [0, carried]);
    // Break Link puts the frames on the artboard in no thread: each counts from 1.
    s.execute("select.set", &json!({"ids": [instance]})).unwrap();
    s.execute("symbol.breakLink", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.layers.iter().flat_map(|l| counts(l)).collect::<Vec<_>>(), [0, 0]);
}

/// The text of each frame in symbol `name`'s art, sorted.
fn symbol_texts(s: &Session, name: &str) -> Vec<String> {
    let doc = &s.doc().unwrap().doc;
    let Some(sym) = doc.symbols.iter().find(|x| x.name == name) else { panic!("no symbol {name}") };
    let mut texts = vec![];
    sym.art.walk(&mut |n| {
        if let NodeKind::Text(t) = &n.kind {
            texts.push(t.plain_text());
        }
    });
    texts.sort();
    texts
}

/// New Symbol and Redefine Symbol on frames of threaded text replace them with an instance
/// whose art holds the text they show, and the frames leave their thread: the other frames keep
/// the text they hold, so each character is drawn once. Undo threads the frames again. When the
/// last frame goes into the symbol, the frame before it takes the text it has no room for, and
/// the symbol's art holds only the text the frame shows.
#[test]
fn frames_made_into_a_symbol_leave_their_thread_with_their_text() {
    let symbol = |s: &mut Session, command: &str, ids: &[u64]| {
        let name = if command == "symbol.update" {
            let mark = new_id(s.execute("shape.rectangle", &json!({"x": 400, "y": 200, "width": 20, "height": 20})).unwrap());
            s.execute("select.set", &json!({"ids": [mark]})).unwrap();
            s.execute("symbol.new", &json!({"name": "Mark"})).unwrap();
            "Mark"
        } else {
            "Story"
        };
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        s.execute(command, &json!({"name": name})).unwrap();
        name
    };
    for command in ["symbol.new", "symbol.update"] {
        for out in [&[0][..], &[1], &[2], &[0, 2]] {
            let case = format!("{command} {out:?}");
            let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
            let shown = frames.map(|id| text(&s, id));
            assert_eq!(shown.concat(), TEXT.repeat(4));
            let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
            let name = symbol(&mut s, command, &ids);
            let rest: Vec<u64> = frames.iter().copied().filter(|id| !ids.contains(id)).collect();
            for id in &ids {
                assert!(!is_frame(&s, *id), "{case}: frame {id} is in the symbol");
            }
            let mut held: Vec<String> = out.iter().map(|k| shown[*k].clone()).collect();
            held.sort();
            assert_eq!(symbol_texts(&s, name), held, "{case}: the symbol's art holds the text the frames show");
            for (id, before) in frames.iter().zip(&shown).filter(|(id, _)| rest.contains(id)) {
                assert_eq!(text(&s, *id), *before, "{case}: frame {id} keeps its text");
            }
            assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{case}");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(threads(&s), [frames], "{case}: undo");
            assert_eq!(frames.map(|id| text(&s, id)), shown, "{case}: undo");
        }
        let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
        let shown = [a, b, c].map(|id| text(&s, id));
        let laid = {
            let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
            vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
        };
        assert!(laid.overflow, "the last frame has more text than it shows");
        let end = laid.lines.last().map_or(0, |l| l.end);
        let hidden = shown[2][end..].trim_start_matches(' ');
        let name = symbol(&mut s, command, &[c]);
        assert_eq!(text(&s, a), shown[0], "{command}");
        assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]), "{command}: the frame before it keeps its text and takes the rest");
        assert_eq!(threads(&s), [[a, b]], "{command}");
        let held = symbol_texts(&s, name);
        let held: Vec<&str> = held.iter().map(|t| t.trim_end()).collect();
        assert_eq!(held, [shown[2][..end].trim_end()], "{command}: the symbol's art holds what the frame shows");
        // With every frame of the thread in the symbol, the last frame keeps all its text.
        let (mut s, frames) = three_frames(&TEXT.repeat(12), 100.0);
        let mut shown = frames.map(|id| text(&s, id)).to_vec();
        shown.sort();
        let name = symbol(&mut s, command, &frames);
        assert_eq!(symbol_texts(&s, name), shown, "{command}: the whole thread");
        assert!(threads(&s).is_empty(), "{command}: the whole thread");
    }
}

/// A frame of a numbered thread pasted while an opacity mask is edited numbers from 1 in the
/// mask art too.
#[test]
fn a_frame_pasted_into_an_edited_opacity_mask_numbers_from_1() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let items = (1..=10).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
    let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": items, "area": {"width": 150, "height": 60}})).unwrap());
    let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap());
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
    assert!(text_of(&s.doc().unwrap().doc, NodeId(b)).is_some_and(|t| t.list_count > 0));
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let obj = new_id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 280})).unwrap());
    let m = new_id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap());
    s.execute("select.set", &json!({"ids": [obj, m]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    s.execute("transparency.editOpacityMask", &json!({})).unwrap();
    s.execute("edit.paste", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let mut counts = vec![];
    doc.node(NodeId(obj)).unwrap().extras().mask.as_ref().unwrap().art.walk(&mut |n| {
        if let NodeKind::Text(t) = &n.kind {
            counts.push(t.list_count);
        }
    });
    assert_eq!(counts, [0], "the pasted frame in the mask art");
}

/// A duplicate of a frame that carried a count of 99 or more numbers from 1 and has the layout
/// bounds of what it draws: "01." leaves the text at the left indent, where a three-digit
/// number pushed it on to the next tab stop and the paragraph took more lines.
#[test]
fn a_duplicated_frame_numbers_from_1_with_its_own_bounds() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let story = format!("{}Itemized lists paragraph text", "x\n".repeat(110));
    let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 1500}})).unwrap());
    let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 100, "height": 300})).unwrap());
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b], "number": "twoDigits"})).unwrap();
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let copy = s.execute("edit.duplicate", &json!({"dx": 200})).unwrap()["ids"][0].as_u64().unwrap();
    let laid = |s: &Session, id: u64| {
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(id)) else { panic!("area type") };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        (lay.markers[0].text.clone(), lay.lines.len(), lay.bounds, t.cached_bounds.unwrap())
    };
    let (number, lines, _, _) = laid(&s, b);
    let (copy_number, copy_lines, bounds, cached) = laid(&s, copy);
    assert!(number.len() == 4 && number.starts_with('1'), "a three-digit number: {number}");
    assert_eq!(copy_number, "01.");
    assert!(copy_lines < lines, "{copy_lines} {lines}");
    assert!((cached.y1 - bounds.y1).abs() < 1e-6, "the copy's bounds: {cached:?} {bounds:?}");
}

/// Release Selection that leaves one frame of a thread gives that frame the whole story and
/// the layout bounds of its new text.
#[test]
fn the_frame_a_release_leaves_alone_has_the_bounds_of_the_whole_story() {
    let (mut s, a, b) = setup();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("text.thread.releaseSelection", &json!({})).unwrap();
    assert!(threads(&s).is_empty());
    assert_eq!(text(&s, b), TEXT.repeat(4));
    let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(b)) else { panic!("area type") };
    let bounds = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).bounds;
    let cached = t.cached_bounds.unwrap();
    assert!((cached.y0 - bounds.y0).abs() < 1e-6 && (cached.y1 - bounds.y1).abs() < 1e-6, "{cached:?} {bounds:?}");
}
