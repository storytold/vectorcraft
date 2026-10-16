//! FreeHand documents opened through `document.open`: pages, layers, paths and paints.

use super::*;
use serde_json::json;

#[test]
fn a_synthetic_freehand_file_opens_with_its_page_layer_and_paints() {
    let bytes = vectorcraft_freehand::synth::demo();
    let mut s = Session::new();
    let result = open_bytes(&mut s, "mark.fh9", &bytes, None).unwrap();
    assert_eq!(result["format"], "freehand");
    assert!(result["warnings"].as_array().unwrap().is_empty(), "{result}");
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.artboards.len(), 1);
    let page = doc.artboards[0].rect;
    assert!((page.width() - 200.0).abs() < 1e-3 && (page.height() - 100.0).abs() < 1e-3);
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layers[0].name.as_deref(), Some("Ink"));
    let flat = format!("{:?}", doc.layers);
    assert!(flat.contains("Path"), "{flat}");
    let mut saw_red = false;
    let mut saw_dash = false;
    let mut saw_gradient = false;
    walk(&doc.layers[0], &mut |n| {
        for item in &n.appearance.items {
            match item {
                vectorcraft_doc::AppearanceItem::Fill(fill) => match &fill.paint {
                    vectorcraft_color::Paint::Solid { color: vectorcraft_color::Color::Rgb { r, g, b }, .. } if *r > 0.9 && *g < 0.1 && *b < 0.1 => {
                        saw_red = true;
                    }
                    vectorcraft_color::Paint::Gradient(g) if g.gradient.stops.len() >= 2 => saw_gradient = true,
                    _ => {}
                },
                vectorcraft_doc::AppearanceItem::Stroke(stroke) => {
                    if (stroke.width - 2.0).abs() < 1e-3
                        && let Some(dash) = &stroke.dash
                        && dash.pattern.len() == 2
                        && dash.pattern.first().is_some_and(|v| (v - 8.0).abs() < 1e-3)
                    {
                        saw_dash = true;
                    }
                }
            }
        }
    });
    assert!(saw_red && saw_dash && saw_gradient, "red {saw_red} dash {saw_dash} gradient {saw_gradient}");
}

fn walk(n: &vectorcraft_doc::Node, f: &mut impl FnMut(&vectorcraft_doc::Node)) {
    f(n);
    if let Some(kids) = n.children() {
        for kid in kids {
            walk(kid, f);
        }
    }
}

#[test]
fn freehand_open_errors_name_the_problem() {
    let mut s = Session::new();
    let empty = open_bytes(&mut s, "empty.fh9", b"", None).unwrap_err();
    assert_eq!(empty.to_string(), "the FreeHand file is empty");
    let short = open_bytes(&mut s, "short.fh9", b"AG", None).unwrap_err();
    assert_eq!(short.to_string(), "the FreeHand file is cut off in the header");
    let wrong = open_bytes(&mut s, "logo.fh9", b"not a freehand file!!", None).unwrap_err();
    assert_eq!(wrong.to_string(), "not a FreeHand file");
    let old = open_bytes(&mut s, "old.fh3", b"FH31", None).unwrap_err();
    assert!(old.to_string().starts_with("unsupported FreeHand version 3:"), "{old}");
}

#[test]
fn freehand_is_readable_and_not_writable() {
    let f = format("freehand").unwrap();
    assert_eq!(f.label, "FreeHand");
    assert!(f.read && !f.write);
    assert_eq!(f.mime, "application/x-freehand");
    for ext in ["fh", "fh3", "fh4", "fh5", "fh7", "fh8", "fh9", "fh10", "fh11", "ft8", "ft9", "ft10", "ft11"] {
        assert!(OPEN_EXTS.contains(&ext) && PLACE_EXTS.contains(&ext), "{ext}");
        assert_eq!(format(ext).map(|f| f.id), Some("freehand"));
    }
    assert!(!SAVE_FORMATS.contains(&"freehand"));
    let mut s = Session::new();
    assert!(s.execute("document.export", &json!({"format":"freehand"})).is_err());
    let sniffed = open_bytes(&mut s, "renamed.bin", &vectorcraft_freehand::synth::demo(), None).unwrap();
    assert_eq!(sniffed["format"], "freehand");
}
