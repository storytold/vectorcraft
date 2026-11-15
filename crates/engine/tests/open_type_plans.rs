//! A document of type opened with the shaping plans font faces keep, compared with the same file
//! opened while every shaped segment makes a plan of its own: the nodes and the first frame's
//! pixels are the same. Its own test binary: the switch for kept plans applies to the whole process.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard, Once};

use serde_json::Value;
use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, TextObject, TextRun};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_geom::{Affine, Point};
use vectorcraft_render::{RenderOptions, Renderer};
use vectorcraft_text::FontDb;
use vectorcraft_text::test_fonts::{VARIABLE_FAMILY, keep_shape_plans, variable_font};

const WIDTH: u32 = 900;
const HEIGHT: u32 = 1300;

/// Taken by each test: the switch for kept plans applies to the whole process.
static SWITCH: Mutex<()> = Mutex::new(());

fn switch() -> MutexGuard<'static, ()> {
    SWITCH.lock().unwrap_or_else(|e| e.into_inner())
}

fn style() -> CharStyle {
    CharStyle { size: 60.0, ..CharStyle::default() }
}

/// A saved document of type objects whose shaping differs in script, direction, language, features
/// and variable font instance, the object in the plain style first.
fn file() -> Vec<u8> {
    static ADD_FONT: Once = Once::new();
    ADD_FONT.call_once(|| {
        FontDb::global().add_font(variable_font().unwrap());
    });
    let manual = CharStyle { kerning: Some(0.0), ..style() };
    let variable = |name: &str| CharStyle { font_family: VARIABLE_FAMILY.into(), font_style: name.into(), ..style() };
    let mut mixed = TextObject::point(Point::ZERO, "AVAV", style());
    mixed.runs = vec![TextRun::new("AV", style()), TextRun::new("AV", manual.clone())];
    let mut vertical = TextObject::point(Point::ZERO, "AB\u{65e5}\u{672c}", style());
    vertical.vertical = true;
    let objects = [
        TextObject::point(Point::ZERO, "AVATAR office", style()),
        TextObject::point(Point::ZERO, "To Wa fi", style()),
        TextObject::point(Point::ZERO, "123 ... 456", style()),
        TextObject::point(Point::ZERO, "AVAV", manual),
        mixed,
        TextObject::point(Point::ZERO, "fi office", CharStyle { language: Some("tr".into()), ..style() }),
        TextObject::point(Point::ZERO, "AVAV", CharStyle { optical_kerning: true, ..style() }),
        TextObject::point(Point::ZERO, "Small Caps", CharStyle { small_caps: Some(70.0), ..style() }),
        TextObject::point(Point::ZERO, "\u{645}\u{631}\u{62d}\u{628}\u{627}", style()),
        TextObject::point(Point::ZERO, "\u{5e9}\u{5dc}\u{5d5}\u{5dd}", style()),
        TextObject::point(Point::ZERO, "lIn lIn", variable("Regular")),
        TextObject::point(Point::ZERO, "lIn lIn", variable("Bold")),
        vertical,
    ];
    let mut doc = Document::new(WIDTH as f64, HEIGHT as f64);
    let layer = doc.layers[0].id;
    let count = objects.len();
    for (i, mut t) in objects.into_iter().enumerate() {
        // Rows down the page; the vertical object, last, stands in the right margin.
        t.xf = Affine::translate(if i + 1 == count { (800.0, 60.0) } else { (20.0, 90.0 * (i + 1) as f64) });
        let id = doc.alloc_id();
        doc.insert(Some(layer), usize::MAX, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
    }
    vectorcraft_format::save_file(&doc)
}

/// The session `bytes` opened in, as `document.open` reads a file and then opens it.
fn open(bytes: &[u8]) -> Session {
    let name = "plans.vectorcraft";
    let mut s = Session::new();
    let read = fileio::read_for_open(name, bytes, Some(name.into()), &Value::Null, false, false).unwrap();
    fileio::open_read(&mut s, read).unwrap();
    s
}

fn first_frame(doc: &Document) -> Vec<u8> {
    Renderer::new().render(doc, WIDTH, HEIGHT, Affine::IDENTITY, &RenderOptions::default()).pixels
}

#[test]
fn an_opened_document_has_the_nodes_of_one_laid_out_with_plans_of_their_own() {
    let _switch = switch();
    let bytes = file();
    keep_shape_plans(false);
    let own = open(&bytes);
    keep_shape_plans(true);
    // The second open shapes every object with plans a face kept.
    open(&bytes);
    let kept = open(&bytes);
    let (own, kept) = (own.doc().unwrap(), kept.doc().unwrap());
    let mut laid_out = 0;
    kept.doc.walk(|node| {
        if let NodeKind::Text(t) = &node.kind {
            assert!(t.cached_bounds.is_some(), "{:?}", t.plain_text());
            laid_out += 1;
        }
    });
    assert_eq!(laid_out, 13);
    assert_eq!(kept.doc.layers, own.doc.layers);
    assert!(*kept.doc == *own.doc, "the whole document is the same");
}

#[test]
fn an_opened_document_draws_the_first_frame_of_one_shaped_with_plans_of_its_own() {
    let _switch = switch();
    let bytes = file();
    let opened = open(&bytes);
    let doc = &opened.doc().unwrap().doc;
    keep_shape_plans(false);
    let own = first_frame(doc);
    keep_shape_plans(true);
    // Lays out each object with plans a face kept: the first frame lays type out again.
    first_frame(doc);
    let kept = first_frame(doc);
    assert_ne!(kept, first_frame(&Document::new(WIDTH as f64, HEIGHT as f64)), "the frame shows the type");
    let differing = kept.iter().zip(&own).filter(|(a, b)| a != b).count();
    assert_eq!((kept.len(), differing), (own.len(), 0), "bytes of the frame that differ");
}
