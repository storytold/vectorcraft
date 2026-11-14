//! Objects with a URL (Window › Attributes) export as link annotations over their bounds that
//! open it; hidden ones and PDF/X files have none.

use std::sync::Arc;

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{Array, Dict, Name};
use vectorcraft_color::Color;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use super::{add, doc, rect_node};
use crate::*;

/// A rectangle over `r` linking to `url`.
fn linked(r: Rect, url: &str) -> Node {
    let mut n = rect_node(r, Color::rgb(1.0, 0.0, 0.0));
    n.edit_attrs(|a| a.url = url.into());
    n
}

/// Each link annotation of each page: its URI and its rectangle (PDF space, y up).
fn links(pdf: &[u8]) -> Vec<Vec<(String, [f32; 4])>> {
    let file = Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    file.pages()
        .iter()
        .map(|page| {
            let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") else { return vec![] };
            annots
                .iter::<Dict<'_>>()
                .filter(|a| a.get::<Name<'_>>(b"Subtype").is_some_and(|s| s.as_ref() == b"Link"))
                .map(|a| {
                    let action = a.get::<Dict<'_>>(b"A").unwrap();
                    assert_eq!(action.get::<Name<'_>>(b"S").unwrap().as_ref(), b"URI");
                    let uri = action.get::<craft_pdf::syntax::object::String<'_>>(b"URI").unwrap().as_bytes().to_vec();
                    let r: Vec<f32> = a.get::<Array<'_>>(b"Rect").unwrap().iter::<f32>().collect();
                    (String::from_utf8(uri).unwrap(), [r[0], r[1], r[2], r[3]])
                })
                .collect()
        })
        .collect()
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

#[test]
fn a_linked_object_exports_a_link_over_its_bounds() {
    let mut d = doc(200.0, 100.0);
    add(&mut d, rect_node(Rect::new(0.0, 0.0, 5.0, 5.0), Color::rgb(0.0, 0.0, 1.0)));
    add(&mut d, linked(Rect::new(10.0, 10.0, 50.0, 30.0), "https://example.com/a b"));
    for bytes in [export(&d, &PdfOptions::default()).unwrap(), export(&d, &PdfOptions::uncompressed()).unwrap()] {
        let pages = links(&bytes);
        assert_eq!(pages.len(), 1);
        let [(uri, rect)] = pages[0].as_slice() else { panic!("one link: {pages:?}") };
        // In ASCII, as a PDF URI is (the space percent-encoded).
        assert_eq!(uri, "https://example.com/a%20b");
        // y up from the page's bottom edge: 100 − 30 to 100 − 10.
        assert!(near(*rect, [10.0, 70.0, 50.0, 90.0]), "{rect:?}");
    }
}

#[test]
fn hidden_and_unprinted_objects_have_no_links_and_clipping_trims_them() {
    let mut d = doc(200.0, 100.0);
    let mut hidden = linked(Rect::new(10.0, 10.0, 50.0, 30.0), "https://example.com/hidden");
    hidden.visible = false;
    add(&mut d, hidden);
    // Clipped to x 20…40: the link covers only what shows.
    let clip = Node::path(NodeId(0), shapes::rectangle(Rect::new(20.0, 0.0, 40.0, 100.0)), Default::default());
    let mut group = Node::group(NodeId(0), vec![Arc::new(clip), Arc::new(linked(Rect::new(0.0, 40.0, 200.0, 60.0), "https://example.com/clipped"))]);
    group.set_clips(true);
    let group = d.reid(&group);
    add(&mut d, group);
    // Off the artboard: nothing to click.
    add(&mut d, linked(Rect::new(300.0, 10.0, 350.0, 30.0), "https://example.com/off"));
    let pages = links(&export(&d, &PdfOptions::default()).unwrap());
    let [(uri, rect)] = pages[0].as_slice() else { panic!("one link: {pages:?}") };
    assert_eq!(uri, "https://example.com/clipped");
    assert!(near(*rect, [20.0, 40.0, 40.0, 60.0]), "{rect:?}");

    // A layer that doesn't print leaves its links out with its art.
    let mut d = doc(200.0, 100.0);
    add(&mut d, linked(Rect::new(10.0, 10.0, 50.0, 30.0), "https://example.com/"));
    if let vectorcraft_doc::NodeKind::Layer { printable, .. } = &mut Arc::make_mut(&mut d.layers[0]).kind {
        *printable = false;
    }
    assert_eq!(links(&export(&d, &PdfOptions::default()).unwrap()), vec![Vec::<(String, [f32; 4])>::new()]);
}

/// A linked symbol instance whose art has a live effect links over the art where the effect draws it.
#[test]
fn a_linked_instance_keeps_its_link_when_its_art_has_effects() {
    let mut art = rect_node(Rect::new(10.0, 40.0, 20.0, 50.0), Color::rgb(0.0, 0.0, 1.0));
    art.appearance.effects.push(vectorcraft_doc::Effect {
        id: "distort.transform".into(),
        params: serde_json::json!({"moveH": 100.0}),
        visible: true,
    });
    let mut d = doc(200.0, 100.0);
    d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Moved".into(), art: Arc::new(art) });
    let mut instance = Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Moved".into(), xf: Affine::IDENTITY });
    instance.edit_attrs(|a| a.url = "https://example.com/symbol".into());
    add(&mut d, instance);
    let pages = links(&export(&d, &PdfOptions::default()).unwrap());
    let [(uri, rect)] = pages[0].as_slice() else { panic!("one link: {pages:?}") };
    assert_eq!(uri, "https://example.com/symbol");
    // x 110…120 after the 100 pt move; y up: 100 − 50 to 100 − 40.
    assert!(near(*rect, [110.0, 50.0, 120.0, 60.0]), "{rect:?}");
}

#[test]
fn each_page_links_its_own_artboard_objects() {
    let mut d = doc(200.0, 100.0);
    let mut second = d.artboards[0].clone();
    second.id = 2;
    second.rect = Rect::new(300.0, 0.0, 400.0, 100.0);
    d.artboards.push(second);
    add(&mut d, linked(Rect::new(310.0, 20.0, 330.0, 40.0), "mailto:a@example.com"));
    let pages = links(&export(&d, &PdfOptions::default()).unwrap());
    assert_eq!(pages[0], vec![]);
    let [(uri, rect)] = pages[1].as_slice() else { panic!("one link: {pages:?}") };
    assert_eq!(uri, "mailto:a@example.com");
    assert!(near(*rect, [10.0, 60.0, 30.0, 80.0]), "{rect:?}");
}

#[test]
fn pdfx_files_have_no_links() {
    let mut d: Document = doc(200.0, 100.0);
    add(&mut d, linked(Rect::new(10.0, 10.0, 50.0, 30.0), "https://example.com/"));
    let mut o = PdfOptions::default();
    (o.settings.standard, o.settings.compatibility) = (Standard::PdfX4, Standard::PdfX4.version());
    o.created = Some(1_700_000_000);
    let report = export_with_report(&d, &o).unwrap();
    assert_eq!(links(&report.bytes), vec![Vec::<(String, [f32; 4])>::new()]);
    assert!(report.warnings.iter().any(|w| w.contains("URLs aren't clickable")), "{:?}", report.warnings);
    // Without links, no such warning.
    assert!(!export_with_report(&doc(200.0, 100.0), &o).unwrap().warnings.iter().any(|w| w.contains("URLs")));
}
