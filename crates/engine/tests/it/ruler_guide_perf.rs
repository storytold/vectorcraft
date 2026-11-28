//! Timings of a document with 10,000 ruler guides and 100,000 objects: save, load, open, the guide
//! list, hovering and dragging with the Selection tool, an edit that does not touch guides and one
//! that does. Ignored by default; it prints its timings and sets no bound on them:
//! `cargo test -p vectorcraft-engine --test it ruler_guide_perf -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::Document;
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::Rect;
use vectorcraft_testkit::fixtures::DocBuilder;
use vectorcraft_tools::{PointerEvent, PointerKind};

const GUIDES: usize = 10_000;
const LAYERS: usize = 4;
const GROUPS_PER_LAYER: usize = 250;
const RECTS_PER_GROUP: usize = 100;

/// The `.vectorcraft` file of the document: 100,000 rectangles in groups of 100 on four layers,
/// and the ruler guides in its saved list, spread over the layers.
fn file() -> Vec<u8> {
    let mut b = DocBuilder::new(2000.0, 2000.0);
    let mut layers = vec![b.layer];
    for l in 0..LAYERS {
        if l > 0 {
            layers.push(b.layer(&format!("Layer {}", l + 1)));
        }
        for g in 0..GROUPS_PER_LAYER {
            let kids = (0..RECTS_PER_GROUP)
                .map(|r| {
                    let (x, y) = (((g * RECTS_PER_GROUP + r) % 400) as f64 * 5.0, (l * GROUPS_PER_LAYER + g) as f64 * 2.0);
                    b.rect_node(Rect::new(x, y, x + 3.0, y + 1.5), Color::rgb(0.2, 0.4, 0.6))
                })
                .collect();
            b.group(kids, false);
        }
    }
    let d: Document = b.build();
    let mut v: Value = serde_json::from_slice(&vectorcraft_format::save(&d, false)).unwrap();
    v["document"]["guides"] =
        (0..GUIDES).map(|i| json!({"vertical": i % 2 == 0, "pos": (i / 2) as f64 * 0.37 + 0.1, "layer": layers[i % LAYERS]})).collect();
    serde_json::to_vec(&v).unwrap()
}

/// The median of `n` runs of `f`.
fn median(n: usize, mut f: impl FnMut()) -> Duration {
    let mut t: Vec<Duration> = (0..n)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    t.sort();
    t[n / 2]
}

fn report(what: &str, d: Duration) {
    eprintln!("ruler_guide_perf: {what:<44} {:>10.3} ms", d.as_secs_f64() * 1000.0);
}

fn pointer(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::new(kind, x, y), ViewInfo::default()).unwrap();
}

#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn ruler_guide_perf() {
    let bytes = file();
    eprintln!("ruler_guide_perf: file of {} bytes", bytes.len());
    report("load (format::load), median of 5", median(5, || drop(vectorcraft_format::load(&bytes).unwrap())));
    let doc = vectorcraft_format::load(&bytes).unwrap();
    report("save (format::save), median of 5", median(5, || drop(vectorcraft_format::save(&doc, false))));

    let mut s = Session::new();
    let data = vectorcraft_format::base64_encode(&bytes);
    let start = Instant::now();
    s.execute("document.open", &json!({"name": "perf.vectorcraft", "dataBase64": data})).unwrap();
    report("document.open", start.elapsed());
    let listed = s.execute("guide.list", &json!({})).unwrap().as_array().map_or(0, Vec::len);
    assert_eq!(listed, GUIDES);
    report("guide.list, median of 11", median(11, || drop(s.execute("guide.list", &json!({})).unwrap())));

    s.select_tool("selection", ViewInfo::default()).unwrap();
    let mut i = 0.0;
    report(
        "hover (Selection tool move), median of 201",
        median(201, || {
            i += 1.0;
            pointer(&mut s, PointerKind::Move, 600.0 + i * 0.13, 300.0 + i * 0.07);
        }),
    );
    // A drag of the first rectangle's group (the rectangle is at 0,0 to 3,1.5), with its
    // snapping: one undo step.
    let steps = s.doc().unwrap().history.undo.len();
    pointer(&mut s, PointerKind::Down, 1.5, 0.75);
    let mut j = 0.0;
    report(
        "drag (Selection tool drag step), median of 51",
        median(51, || {
            j += 1.0;
            pointer(&mut s, PointerKind::Drag, 1.5 + j * 0.9, 0.75 + j * 0.4);
        }),
    );
    let start = Instant::now();
    pointer(&mut s, PointerKind::Up, 1.5 + j * 0.9, 0.75 + j * 0.4);
    report("drag release (commit)", start.elapsed());
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "the drag moved the group");

    let mut k = 0.0;
    report(
        "edit without guides (shape.rectangle), median of 21",
        median(21, || {
            k += 1.0;
            s.execute("shape.rectangle", &json!({"x": k, "y": 1900, "width": 5, "height": 5})).unwrap();
        }),
    );
    report("undo, median of 11", median(11, || drop(s.execute("edit.undo", &json!({})).unwrap())));
    let mut m = 0.0;
    report(
        "guide edit (guide.move), median of 21",
        median(21, || {
            m += 1.0;
            s.execute("guide.move", &json!({"index": GUIDES / 2, "pos": 100.0 + m})).unwrap();
        }),
    );
    report("file.saveCopy, median of 3", median(3, || drop(s.execute("file.saveCopy", &json!({})).unwrap())));
}
