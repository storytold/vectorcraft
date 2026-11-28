//! Blending in CMYK documents: transparency groups blend in DeviceCMYK.

use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, ColorMode, Document, Node, NodeId, OpacityMask};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

fn rect(r: Rect, c: Color) -> Node {
    Node::path(NodeId(0), shapes::rectangle(r), Appearance::basic(Paint::solid(c), Paint::None, 0.0))
}

fn add(d: &mut Document, mut n: Node) {
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

/// Magenta with cyan multiplied over it: in a group when `grouped`, else at the top of the page.
fn doc(mode: ColorMode, grouped: bool) -> Document {
    let mut d = Document::new_with_mode(100.0, 100.0, mode);
    add(&mut d, rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::cmyk(0.0, 1.0, 0.0, 0.0)));
    let mut top = rect(Rect::new(20.0, 20.0, 80.0, 80.0), Color::cmyk(1.0, 0.0, 0.0, 0.0));
    top.blend = BlendMode::Multiply;
    if grouped {
        top.id = d.alloc_id();
        let mut g = Node::group(NodeId(0), vec![Arc::new(top)]);
        g.opacity = 0.5;
        add(&mut d, g);
    } else {
        add(&mut d, top);
    }
    d
}

/// The file's text with spaces removed (content streams uncompressed), after checking it reads.
fn text(d: &Document) -> String {
    let bytes = export(d, &PdfOptions::uncompressed()).unwrap();
    import(&bytes).expect("the rewritten file still reads");
    String::from_utf8_lossy(&bytes).replace(' ', "")
}

#[test]
fn groups_of_cmyk_documents_blend_in_device_cmyk() {
    for grouped in [true, false] {
        let t = text(&doc(ColorMode::Cmyk, grouped));
        // Top-level blending gets a group of its own, so it blends in CMYK too.
        assert!(t.contains("/S/Transparency") && t.contains("/CS/DeviceCMYK"), "grouped {grouped}: {t}");
        assert!(!t.contains("/CS/DeviceRGB"), "grouped {grouped}: {t}");
        assert!(t.contains("/BM/Multiply"));
    }
    // Compressed output gets the same group dictionaries.
    let bytes = export(&doc(ColorMode::Cmyk, true), &PdfOptions::default()).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("/CS/DeviceCMYK"));
}

#[test]
fn rgb_documents_and_opaque_cmyk_pages_are_unchanged() {
    let t = text(&doc(ColorMode::Rgb, true));
    assert!(t.contains("/CS/DeviceRGB") && !t.contains("/CS/DeviceCMYK"), "{t}");
    let mut d = doc(ColorMode::Cmyk, false);
    let top = d.layers[0].children().unwrap()[1].id;
    d.node_mut(top).unwrap().blend = BlendMode::Normal;
    assert!(!text(&d).contains("/Transparency"));
}

#[test]
fn soft_mask_groups_keep_screen_luminance() {
    let mut d = Document::new_with_mode(100.0, 100.0, ColorMode::Cmyk);
    let mut n = rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::cmyk(0.0, 0.0, 0.0, 1.0));
    n.extras_mut().mask = Some(Box::new(OpacityMask::new(rect(Rect::new(0.0, 0.0, 50.0, 100.0), Color::WHITE), true)));
    add(&mut d, n);
    let t = text(&d);
    // The page's group blends in CMYK; the mask's group measures luminance in RGB.
    assert!(t.contains("/CS/DeviceCMYK") && t.contains("/S/Luminosity") && t.contains("/CS/DeviceRGB"), "{t}");
}

#[test]
fn constant_alpha_masks_are_white() {
    // #1286: a black cover hides the whole page in readers that take the mask's luminosity.
    for grouped in [true, false] {
        let t = text(&doc(ColorMode::Cmyk, grouped));
        assert!(t.contains("/S/Alpha") && t.contains("111rg") && !t.contains("000rg"), "grouped {grouped}: {t}");
    }
}

#[test]
fn a_cmyk_page_without_blending_through_is_an_isolated_group() {
    // #2156: CoreGraphics drops most of a page put in a non-isolated group. Without a blend mode
    // reaching the paper, isolating the page's group changes nothing it draws.
    let mut d = Document::new_with_mode(100.0, 100.0, ColorMode::Cmyk);
    add(&mut d, rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::cmyk(0.0, 1.0, 0.0, 0.0)));
    let mut top = rect(Rect::new(20.0, 20.0, 80.0, 80.0), Color::cmyk(1.0, 0.0, 0.0, 0.0));
    top.opacity = 0.6;
    add(&mut d, top);
    let t = text(&d);
    assert!(t.contains("/Itrue") && t.contains("/CS/DeviceCMYK") && !t.contains("/S/Alpha"), "{t}");
    // Blending that reaches the paper keeps the page's group non-isolated.
    for grouped in [true, false] {
        let t = text(&doc(ColorMode::Cmyk, grouped));
        assert!(!t.contains("/Itrue") && t.contains("/S/Alpha"), "grouped {grouped}: {t}");
    }
}

fn rgb_warning(d: &Document) -> Option<String> {
    export_with_report(d, &PdfOptions::uncompressed()).unwrap().warnings.into_iter().find(|w| w.contains("RGB"))
}

#[test]
fn rgb_colours_in_a_cmyk_document_are_reported_and_separated() {
    let mut d = Document::new_with_mode(100.0, 100.0, ColorMode::Cmyk);
    add(&mut d, rect(Rect::new(0.0, 0.0, 50.0, 50.0), Color::rgb(0.11, 0.1, 0.1)));
    add(&mut d, rect(Rect::new(50.0, 0.0, 100.0, 50.0), Color::rgb(0.11, 0.1, 0.1)));
    add(&mut d, rect(Rect::new(0.0, 50.0, 50.0, 100.0), Color::rgb(0.9, 0.2, 0.2)));
    let w = rgb_warning(&d).expect("a warning");
    assert!(w.contains("2 RGB colours"), "{w}");
    // The colours are written as CMYK, not RGB.
    let t = text(&d);
    assert!(t.contains("k0") && !t.contains("rg\n"), "{t}");
}

#[test]
fn cmyk_colours_and_rgb_documents_have_no_rgb_warning() {
    let mut d = Document::new_with_mode(100.0, 100.0, ColorMode::Cmyk);
    add(&mut d, rect(Rect::new(0.0, 0.0, 50.0, 50.0), Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    assert_eq!(rgb_warning(&d), None);
    let mut d = Document::new_with_mode(100.0, 100.0, ColorMode::Rgb);
    add(&mut d, rect(Rect::new(0.0, 0.0, 50.0, 50.0), Color::rgb(0.1, 0.1, 0.1)));
    assert_eq!(rgb_warning(&d), None);
}
