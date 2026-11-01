//! OpenType SVG glyphs: `glyph_art` makes a glyph's art of its SVG document, as the importer opens
//! SVG files, in font units.

use proptest::prelude::*;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_svg::glyph_art;

const BLUE: Color = Color::Rgb { r: 0.0, g: 0.0, b: 1.0 };

/// [`glyph_art`] in a font of 1000 units per em, in black text.
fn art(doc: &str, gid: u32) -> Option<Node> {
    glyph_art(doc.as_bytes(), gid, 1000.0, Color::BLACK, None).map(|(n, _)| n)
}

fn fills(n: &Node) -> Vec<Paint> {
    let mut out = vec![];
    n.walk(&mut |c| out.push(c.appearance.fill_paint()));
    out
}

#[test]
fn the_glyphs_element_is_its_art_in_font_units() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg">
      <defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
      <g id="glyph3"><path fill="#0f0" d="M0 0h10v-10z"/></g>
      <g id="glyph7"><path fill="url(#g)" d="M100 0h500v-700h-500z"/></g>
    </svg>"##;
    let art7 = art(doc, 7).unwrap();
    let b = art7.geometric_bounds().unwrap();
    assert!((b.x0 - 100.0).abs() < 1e-6 && (b.x1 - 600.0).abs() < 1e-6 && (b.y0 + 700.0).abs() < 1e-6 && b.y1.abs() < 1e-6, "{b:?}");
    let fills = fills(&art7);
    assert!(fills.iter().any(|p| matches!(p, Paint::Gradient(_))), "its gradient: {fills:?}");
    assert!(!fills.contains(&Paint::solid(Color::rgb8(0, 255, 0))), "not the other glyph's art");
    assert!(art(doc, 5).is_none(), "no element, no art");
}

#[test]
fn a_glyph_that_is_its_whole_document_is_its_art() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" id="glyph9"><path fill="#f00" d="M0 0h500v-500z"/></svg>"##;
    assert_eq!(fills(&art(doc, 9).unwrap()).iter().filter(|p| **p == Paint::solid(Color::rgb8(255, 0, 0))).count(), 1);
}

#[test]
fn current_color_and_context_colours_are_the_texts() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph1">
      <path fill="currentColor" d="M0 0h500v-500z"/>
      <path fill="context-fill" stroke="context-stroke" stroke-width="10" d="M0 0h500v-500z"/>
    </g></svg>"##;
    let (art, coloured) = glyph_art(doc.as_bytes(), 1, 1000.0, BLUE, Some(Color::rgb8(255, 0, 0))).unwrap();
    assert!(coloured);
    let mut seen = vec![];
    art.walk(&mut |c| seen.push((c.appearance.fill_paint(), c.appearance.stroke_paint())));
    assert!(seen.contains(&(Paint::solid(BLUE), Paint::None)), "currentColor: {seen:?}");
    assert!(seen.contains(&(Paint::solid(BLUE), Paint::solid(Color::rgb8(255, 0, 0)))), "context-fill and -stroke: {seen:?}");
    let (art, _) = glyph_art(doc.as_bytes(), 1, 1000.0, BLUE, None).unwrap();
    let mut strokes = vec![];
    art.walk(&mut |c| strokes.push(c.appearance.stroke_paint()));
    assert!(strokes.iter().all(|p| *p == Paint::None), "no stroke without the text's: {strokes:?}");
    let plain = r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph1"><path fill="#f00" d="M0 0h5v-5z"/></g></svg>"##;
    assert!(!glyph_art(plain.as_bytes(), 1, 1000.0, BLUE, None).unwrap().1, "a document in its own colours");
}

#[test]
fn a_view_box_is_the_em_square() {
    // The view box 2000 units wide is the em: in a font of 1000 units per em, art at 500 is at 250,
    // in one of 2048 at 512; a size on the root counts for nothing.
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10pt" height="10pt" viewBox="0 0 2000 2000">
      <g id="glyph2"><path fill="#000" d="M500 500h500v500h-500z"/></g></svg>"##;
    for (upem, x) in [(1000.0, 250.0), (2048.0, 512.0)] {
        let b = glyph_art(doc.as_bytes(), 2, upem, Color::BLACK, None).unwrap().0.geometric_bounds().unwrap();
        assert!((b.x0 - x).abs() < 1e-6 && (b.width() - x).abs() < 1e-6, "{upem}: {b:?}");
    }
}

/// Text (which would lay type out inside type), embedded content and scripts are left out of a
/// glyph, as the OpenType `SVG ` table asks.
#[test]
fn text_and_scripts_are_left_out() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g id="glyph1">
      <path fill="#f00" d="M0 0h500v-500z"/>
      <text font-family="Svgtest Glyphs" font-size="500">E<tspan>F</tspan></text>
      <script>alert(1)</script>
    </g></svg>"##;
    let art = art(doc, 1).unwrap();
    let mut kinds = vec![];
    art.walk(&mut |n| kinds.push(n.kind_label()));
    assert!(art.children().is_some_and(|c| !c.is_empty()), "the path stays");
    let mut other = false;
    art.walk(&mut |n| other |= matches!(n.kind, NodeKind::Text(_)));
    assert!(!other, "{kinds:?}");
}

#[test]
fn gzipped_documents_are_read_and_malformed_ones_are_no_art() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph4"><path fill="#000" d="M0 0h10v-10z"/></g></svg>"##;
    assert!(glyph_art(&vectorcraft_svg::compress(doc), 4, 1000.0, Color::BLACK, None).is_some(), "gzipped");
    assert!(art("not xml <", 4).is_none());
    // A document that unpacks past the glyph cap (a bomb).
    let huge = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><g id=\"glyph4\"><!--{}--></g></svg>", "x".repeat(4 << 20));
    assert!(glyph_art(&vectorcraft_svg::compress(&huge), 4, 1000.0, Color::BLACK, None).is_none());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A font's glyph documents are untrusted: whatever they hold, reading one doesn't panic.
    #[test]
    fn any_document_reads_without_panicking(body in "[<>/=\"a-z0-9 #:.()-]{0,200}", bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
        let doc = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"><g id=\"glyph1\">{body}</g></svg>");
        let _ = glyph_art(doc.as_bytes(), 1, 1000.0, Color::BLACK, Some(BLUE));
        let _ = glyph_art(&bytes, 1, 0.0, Color::BLACK, None);
    }
}

/// A left-out element inside another glyph's element goes with it, and the glyph asked for, nested
/// in a group that holds other glyphs' art, is still found.
#[test]
fn cuts_inside_cuts_and_nested_glyphs_read() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg">
      <g id="glyph1"><path fill="#0f0" d="M0 0h5v-5z"/><text>A</text></g>
      <g><g id="glyph2"><path fill="#f00" d="M0 0h500v-500z"/><script>x</script></g></g>
    </svg>"##;
    assert!(fills(&art(doc, 2).unwrap()).contains(&Paint::solid(Color::rgb8(255, 0, 0))));
    assert!(art(doc, 1).is_some_and(|a| fills(&a).contains(&Paint::solid(Color::rgb8(0, 255, 0)))));
}

/// A font's glyph is art, never a script: event attributes on its shapes are left out, so no
/// export carries them on.
#[test]
fn event_attributes_are_left_out() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" onload="a()"><g id="glyph1" onclick="b()">
      <path id="p" fill="#f00" onmouseover="c()" d="M0 0h500v-500z"/></g></svg>"##;
    let art = art(doc, 1).unwrap();
    let mut events = vec![];
    art.walk(&mut |n| events.extend(n.extras().attrs.as_ref().map(|a| a.events.clone()).unwrap_or_default()));
    assert!(events.is_empty(), "{events:?}");
}

/// A size on the root without a `viewBox` sets up the em square as a `viewBox` would.
#[test]
fn a_root_size_is_the_em_square_without_a_view_box() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" width="2000" height="2000">
      <g id="glyph2"><path fill="#000" d="M500 500h500v500h-500z"/></g></svg>"##;
    let b = art(doc, 2).unwrap().geometric_bounds().unwrap();
    assert!((b.x0 - 250.0).abs() < 1e-6 && (b.width() - 250.0).abs() < 1e-6, "{b:?}");
}

/// A `viewBox` that isn't square fits the em square by its larger side, centred along the other.
#[test]
fn a_view_box_that_isnt_square_is_centred_in_the_em_square() {
    // 2000 wide, 1000 high: half a unit per unit, the box 500 high centred in the em's 1000.
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2000 1000">
      <g id="glyph2"><path fill="#000" d="M0 0h2000v1000h-2000z"/></g></svg>"##;
    let b = art(doc, 2).unwrap().geometric_bounds().unwrap();
    assert!(b.x0.abs() < 1e-6 && (b.y0 - 250.0).abs() < 1e-6 && (b.height() - 500.0).abs() < 1e-6, "{b:?}");
}

/// A glyph element inside others is drawn on its own, as if used: what its parents apply doesn't
/// reach it, and what it refers to still does.
#[test]
fn a_nested_glyph_is_drawn_on_its_own() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg">
      <defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
      <g transform="translate(1000 0)" opacity="0.5"><g id="glyph3"><path fill="url(#g)" d="M0 0h500v-500z"/></g></g>
    </svg>"##;
    let art = art(doc, 3).unwrap();
    let b = art.geometric_bounds().unwrap();
    assert!(b.x0.abs() < 1e-6 && (b.x1 - 500.0).abs() < 1e-6, "not moved by its parent: {b:?}");
    let mut opacities = vec![];
    art.walk(&mut |n| opacities.push(n.opacity));
    assert!(opacities.iter().all(|o| *o == 1.0), "not faded by its parent: {opacities:?}");
    assert!(fills(&art).iter().any(|p| matches!(p, Paint::Gradient(_))), "its gradient");
}

/// A namespace the glyph's parents declare still binds it when it is drawn on its own.
#[test]
fn a_nested_glyph_keeps_the_namespaces_its_parents_declare() {
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg">
      <g xmlns:l="http://www.w3.org/1999/xlink"><defs><path id="a" d="M0 0h500v-500z"/></defs>
        <g id="glyph3"><use fill="#f00" l:href="#a"/></g></g>
    </svg>"##;
    assert!(art(doc, 3).is_some_and(|a| fills(&a).contains(&Paint::solid(Color::rgb8(255, 0, 0)))));
}

/// An image in a glyph's document is in its art, holding its pixels (the art has no document to
/// keep them in), named by their content; one whose pixels don't read is left out.
#[test]
fn images_are_in_the_art_with_their_pixels() {
    let png = vectorcraft_text::test_fonts::solid_png(4, [0, 0, 255, 255]);
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g id="glyph1">
      <image x="0" y="-500" width="500" height="500" xlink:href="data:image/png;base64,{}"/>
      <image width="10" height="10" xlink:href="missing.png"/>
    </g></svg>"##,
        vectorcraft_svg::base64_encode(&png)
    );
    let art = art(&doc, 1).unwrap();
    let mut images = vec![];
    art.walk(&mut |n| {
        if let NodeKind::Image(im) = &n.kind {
            images.push((**im).clone());
        }
    });
    assert_eq!(images.len(), 1, "the one that reads");
    let pixels = images[0].pixels.as_ref().expect("its own pixels");
    assert_eq!(pixels.bytes.as_slice(), png.as_slice());
    assert_eq!(images[0].key, pixels.content_key());
    let b = art.geometric_bounds().unwrap();
    assert!(b.x0.abs() < 1e-6 && (b.width() - 500.0).abs() < 1e-6 && (b.y0 + 500.0).abs() < 1e-6, "in font units: {b:?}");
}

/// An image whose data is an SVG document is left out, as the `SVG ` table asks: a glyph draws no
/// documents inside it.
#[test]
fn svg_images_are_left_out() {
    let inner = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#f00"/></svg>"##;
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g id="glyph1">
      <path fill="#00f" d="M0 0h500v-500z"/>
      <image width="500" height="500" xlink:href="data:image/svg+xml;base64,{}"/>
      <image width="500" height="500" href=" DATA:image/SVG+xml;utf8,{}"/>
    </g></svg>"##,
        vectorcraft_svg::base64_encode(inner.as_bytes()),
        vectorcraft_svg::xml_escape(&inner.replace('#', "%23"))
    );
    let fills = fills(&art(&doc, 1).unwrap());
    assert!(fills.contains(&Paint::solid(Color::rgb8(0, 0, 255))), "the glyph's own art");
    assert!(!fills.contains(&Paint::solid(Color::rgb8(255, 0, 0))), "not the images' documents: {fills:?}");
}

/// Of `href` and `xlink:href`, the plain one counts, as the importer reads them (SVG 2): an image
/// whose `href` is a PNG stays, whatever its `xlink:href`; one whose `href` is a document goes.
#[test]
fn an_images_plain_href_decides_whether_it_is_an_svg_image() {
    let png = vectorcraft_svg::base64_encode(&vectorcraft_text::test_fonts::solid_png(4, [0, 0, 255, 255]));
    let svg = vectorcraft_svg::base64_encode(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="#f00"/></svg>"##,
    );
    let image = |href: &str, xlink: &str| {
        format!(
            r##"<image width="500" height="500" href="data:image/{href};base64,{}" xlink:href="data:image/{xlink};base64,{}"/>"##,
            if href == "png" { &png } else { &svg },
            if xlink == "png" { &png } else { &svg }
        )
    };
    let images = |inner: String| {
        let doc = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><g id="glyph1"><path d="M0 0h1v-1z"/>{inner}</g></svg>"##
        );
        let mut n = 0;
        art(&doc, 1).unwrap().walk(&mut |c| n += usize::from(matches!(c.kind, NodeKind::Image(_))));
        n
    };
    assert_eq!(images(image("png", "svg+xml")), 1, "the PNG stays");
    assert_eq!(images(image("svg+xml", "png")), 0, "the document goes");
}
