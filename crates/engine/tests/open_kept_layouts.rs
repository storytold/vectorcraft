//! A document of type opened and drawn with the layouts the open keeps, compared with the same
//! document drawn after laying every object out again: the nodes, bounds, baselines and pixels are
//! the same, and the first frame lays out only what it could not take. Its own test binary: kept
//! layouts and the font database's generation belong to the whole process.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, TextObject, TextRun};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_geom::{Affine, Point};
use vectorcraft_render::{RenderOptions, Renderer};
use vectorcraft_text::test_fonts::{drop_kept_layouts, layouts_on_this_thread};
use vectorcraft_text::{FontDb, KeptLayoutsFrame};

const WIDTH: u32 = 900;
const HEIGHT: u32 = 1300;

/// Taken by each test: kept layouts belong to the whole process.
static PROCESS: Mutex<()> = Mutex::new(());

fn process() -> MutexGuard<'static, ()> {
    PROCESS.lock().unwrap_or_else(|e| e.into_inner())
}

fn style() -> CharStyle {
    CharStyle { size: 40.0, ..CharStyle::default() }
}

fn file_of(objects: Vec<TextObject>) -> Vec<u8> {
    let mut doc = Document::new(f64::from(WIDTH), f64::from(HEIGHT));
    let layer = doc.layers[0].id;
    for (i, mut t) in objects.into_iter().enumerate() {
        t.xf = Affine::translate((20.0, 70.0 * (i + 1) as f64));
        let id = doc.alloc_id();
        doc.insert(Some(layer), usize::MAX, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
    }
    vectorcraft_format::save_file(&doc)
}

fn latin() -> Vec<u8> {
    file_of(["Plain Latin type", "AVATAR office", "123 ... 456"].map(|s| TextObject::point(Point::ZERO, s, style())).into())
}

fn mixed_fonts() -> Vec<u8> {
    let font = |family: &str, text: &str| TextRun::new(text, CharStyle { font_family: family.into(), ..style() });
    let mut mixed = TextObject::point(Point::ZERO, "", style());
    mixed.runs = vec![font("Inter", "Inter "), font("JetBrains Mono", "Mono "), font("Source Serif 4", "Serif "), font("No Such Family", "Missing")];
    file_of(vec![mixed, TextObject::point(Point::ZERO, "Source Sans", style())])
}

fn complex_scripts() -> Vec<u8> {
    let scripts = [
        "\u{645}\u{631}\u{62d}\u{628}\u{627}",
        "\u{5e9}\u{5dc}\u{5d5}\u{5dd}",
        "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947}",
        "\u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}\u{e35}",
        "\u{65e5}\u{672c}\u{8a9e} abc",
    ];
    file_of(scripts.map(|s| TextObject::point(Point::ZERO, s, style())).into())
}

/// Area type, and a story through two frames, as a session saves them.
fn threaded_story() -> Vec<u8> {
    let mut s = Session::new();
    s.add_document(Document::new(f64::from(WIDTH), f64::from(HEIGHT)), None);
    let story = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame. ".repeat(4);
    let a = s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute(
        "text.create",
        &json!({"x": 10, "y": 300, "text": "Area type on its own. ".repeat(20), "size": 12, "area": {"width": 300, "height": 200}}),
    )
    .unwrap();
    vectorcraft_format::save_file(&s.doc().unwrap().doc)
}

fn documents() -> Vec<(&'static str, Vec<u8>)> {
    vec![("Latin", latin()), ("mixed fonts", mixed_fonts()), ("complex scripts", complex_scripts()), ("threaded story", threaded_story())]
}

/// The session `bytes` opened in, as `document.open` reads a file and then opens it.
fn open(bytes: &[u8]) -> Session {
    let name = "layouts.vectorcraft";
    let mut s = Session::new();
    let read = fileio::read_for_open(name, bytes, Some(name.into()), &Value::Null, false, false).unwrap();
    fileio::open_read(&mut s, read).unwrap();
    s
}

fn frame(doc: &Document) -> Vec<u8> {
    Renderer::new().render(doc, WIDTH, HEIGHT, Affine::IDENTITY, &RenderOptions::default()).pixels
}

fn type_objects(doc: &Document) -> Vec<TextObject> {
    let mut all = vec![];
    doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            all.push((**t).clone());
        }
    });
    all
}

/// Open once to load every font the document uses: a load changes the font database's generation.
fn fonts_loaded(bytes: &[u8]) {
    let s = open(bytes);
    frame(&s.doc().unwrap().doc);
    drop_kept_layouts();
}

#[test]
fn the_first_frame_draws_the_pixels_of_laying_every_object_out_again() {
    let _process = process();
    for (name, bytes) in documents() {
        fonts_loaded(&bytes);
        let s = open(&bytes);
        let doc = &s.doc().unwrap().doc;
        let taken = frame(doc);
        drop_kept_layouts();
        let again = frame(doc);
        assert!(again != frame(&Document::new(f64::from(WIDTH), f64::from(HEIGHT))), "{name}: the frame shows the type");
        let differing = taken.iter().zip(&again).filter(|(a, b)| a != b).count();
        assert_eq!((taken.len(), differing), (again.len(), 0), "{name}: bytes of the frame that differ");
    }
}

#[test]
fn the_kept_layouts_are_the_layouts_of_the_opened_objects() {
    let _process = process();
    for (name, bytes) in documents() {
        fonts_loaded(&bytes);
        let s = open(&bytes);
        let doc = &s.doc().unwrap().doc;
        let mut taken = 0;
        let frame_drawn = KeptLayoutsFrame::begin();
        for t in type_objects(doc) {
            let fresh = vectorcraft_text::layout(FontDb::global(), &t);
            assert_eq!(t.cached_bounds, Some(fresh.bounds), "{name}: {:?}", t.plain_text());
            assert_eq!(t.cached_baselines, fresh.baselines(), "{name}: {:?}", t.plain_text());
            if let Some(kept) = vectorcraft_text::take_kept_layout(&t) {
                let outlines =
                    |l: &vectorcraft_text::TextLayout| l.glyphs.iter().map(|g| (g.font_id, g.gid, g.origin, g.outline.clone())).collect::<Vec<_>>();
                assert!(outlines(&kept) == outlines(&fresh), "{name}: {:?}", t.plain_text());
                assert_eq!(kept.bounds, fresh.bounds, "{name}: {:?}", t.plain_text());
                taken += 1;
            }
        }
        assert!(taken > 0, "{name}: the open kept layouts");
        drop(frame_drawn);
        drop_kept_layouts();
        let again = open(&bytes);
        assert!(*again.doc().unwrap().doc == **doc, "{name}: another open reads the same document");
        drop_kept_layouts();
    }
}

#[test]
fn the_first_frame_lays_out_no_object_the_open_laid_out() {
    let _process = process();
    let bytes = latin();
    fonts_loaded(&bytes);
    let s = open(&bytes);
    let doc = &s.doc().unwrap().doc;
    let before = layouts_on_this_thread();
    frame(doc);
    assert_eq!(layouts_on_this_thread() - before, 0, "the frame takes every layout");
    // A second renderer has nothing left to take.
    let before = layouts_on_this_thread();
    frame(doc);
    assert_eq!(layouts_on_this_thread() - before, 3);
}

#[test]
fn an_edited_object_is_laid_out_again() {
    let _process = process();
    let bytes = latin();
    fonts_loaded(&bytes);
    let mut s = open(&bytes);
    let id = {
        let mut ids = vec![];
        s.doc().unwrap().doc.walk(|n| {
            if matches!(n.kind, NodeKind::Text(_)) {
                ids.push(n.id.0);
            }
        });
        ids[0]
    };
    s.execute("text.setText", &json!({"id": id, "text": "Edited"})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let before = layouts_on_this_thread();
    let edited = frame(&doc);
    assert_eq!(layouts_on_this_thread() - before, 1, "only the edited object is laid out");
    drop_kept_layouts();
    assert!(edited == frame(&doc), "the frame is what laying every object out again draws");
}

#[test]
fn changing_the_hyphenation_exceptions_or_the_fonts_lays_every_object_out_again() {
    let _process = process();
    let bytes = latin();
    fonts_loaded(&bytes);
    let s = open(&bytes);
    let doc = &s.doc().unwrap().doc;
    vectorcraft_text::set_hyphenation_exceptions("");
    let before = layouts_on_this_thread();
    frame(doc);
    assert_eq!(layouts_on_this_thread() - before, 3, "hyphenation exceptions were set");
    let s = open(&bytes);
    let doc = &s.doc().unwrap().doc;
    FontDb::global().add_font(vectorcraft_text::test_fonts::variable_font().unwrap());
    let generation = FontDb::global().generation();
    let before = layouts_on_this_thread();
    frame(doc);
    assert_eq!(layouts_on_this_thread() - before, 3, "fonts were added (generation {generation})");
}

/// The layout of an object the first frame culls is dropped when that frame ends.
#[test]
fn a_frame_lets_go_of_what_it_did_not_draw() {
    let _process = process();
    let mut objects = vec![TextObject::point(Point::ZERO, "In view", style())];
    objects.extend((0..3).map(|i| TextObject::point(Point::ZERO, &format!("Out of view {i}"), style())));
    let bytes = file_of(objects);
    fonts_loaded(&bytes);
    let s = open(&bytes);
    let doc = &s.doc().unwrap().doc;
    let small = Renderer::new().render(doc, 300, 100, Affine::IDENTITY, &RenderOptions::default());
    assert_eq!(small.width, 300);
    let out = type_objects(doc).into_iter().find(|t| t.plain_text() == "Out of view 2").unwrap();
    let _frame = KeptLayoutsFrame::begin();
    assert!(vectorcraft_text::take_kept_layout(&out).is_none());
}

/// A thumbnail lays its object out, and the first frame still takes that object's layout.
#[test]
fn a_thumbnail_leaves_the_layouts_for_the_first_frame() {
    let _process = process();
    let bytes = latin();
    fonts_loaded(&bytes);
    let s = open(&bytes);
    let doc = &s.doc().unwrap().doc;
    let mut ids = vec![];
    doc.walk(|n| {
        if matches!(n.kind, NodeKind::Text(_)) {
            ids.push(n.id);
        }
    });
    let before = layouts_on_this_thread();
    assert!(Renderer::new().render_thumbnail(doc, ids[0], 64).is_some());
    assert_eq!(layouts_on_this_thread() - before, 1, "the thumbnail lays its object out");
    let before = layouts_on_this_thread();
    frame(doc);
    assert_eq!(layouts_on_this_thread() - before, 0, "the frame takes every layout");
}

/// A frame of ink amounts ends like any other.
#[test]
fn an_ink_frame_lets_go_of_what_it_did_not_draw() {
    let _process = process();
    let mut objects = vec![TextObject::point(Point::ZERO, "In view", style())];
    objects.extend((0..3).map(|i| TextObject::point(Point::ZERO, &format!("Out of view {i}"), style())));
    let bytes = file_of(objects);
    fonts_loaded(&bytes);
    let s = open(&bytes);
    let doc = &s.doc().unwrap().doc;
    let region = vectorcraft_geom::Rect::new(0.0, 0.0, 300.0, 100.0);
    assert!(!Renderer::new().render_region_inks(doc, region, 1.0, &RenderOptions::default()).is_empty());
    let out = type_objects(doc).into_iter().find(|t| t.plain_text() == "Out of view 2").unwrap();
    let _frame = KeptLayoutsFrame::begin();
    assert!(vectorcraft_text::take_kept_layout(&out).is_none());
}
