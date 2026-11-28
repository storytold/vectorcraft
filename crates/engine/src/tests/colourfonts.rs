//! Colour fonts through the session: the SVG importer it registers reads OpenType SVG glyphs,
//! which then draw, export and outline as art.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_text::test_fonts::{SVG_CHAR, source_sans_gid, svg_font};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|x| NodeId(x.as_u64().unwrap())).collect()
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

/// `font` with licence flags (OS/2 `fsType`) that forbid embedding it.
fn not_embeddable(mut font: Vec<u8>) -> Vec<u8> {
    let tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
    let os2 = (0..tables).map(|i| 12 + 16 * i).find(|r| &font[*r..*r + 4] == b"OS/2").unwrap();
    let at = u32::from_be_bytes(font[os2 + 8..os2 + 12].try_into().unwrap()) as usize;
    font[at + 8..at + 10].copy_from_slice(&0x0002u16.to_be_bytes());
    font
}

/// The fills of `pdf`'s art read back, and where each is.
fn fills_of(pdf: &[u8]) -> Vec<(vectorcraft_color::Paint, vectorcraft_geom::Rect)> {
    let back = vectorcraft_pdf::import_with_report(pdf, &vectorcraft_pdf::ImportOptions::default()).unwrap().document;
    let mut fills = vec![];
    for l in &back.layers {
        l.walk(&mut |n| {
            if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
                && let Some(b) = n.geometric_bounds()
            {
                fills.push((n.appearance.fill_paint(), b));
            }
        });
    }
    fills
}

/// A session with `family`, the SVG test font of `document` for [`SVG_CHAR`]'s glyph, and 32 pt
/// type of `SVG_CHAR` and `F` in it at (10, 50).
fn with_svg_type(family: &str, document: &str) -> Session {
    let g = source_sans_gid(SVG_CHAR).unwrap();
    let mut s = session();
    vectorcraft_text::FontDb::global().add_font(svg_font(family, &[(g, g, document)]).unwrap());
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": format!("{SVG_CHAR}F"), "size": 32, "font": family})).unwrap();
    s
}

/// OpenType SVG glyphs become groups of the art their documents draw, through the SVG importer the
/// session registers.
#[test]
fn create_outlines_turns_svg_glyphs_into_groups() {
    let _fonts = crate::adding_fonts();
    let g = source_sans_gid(SVG_CHAR).unwrap();
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="a"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs><g id="glyph{g}"><path fill="url(#a)" d="M0 0h500v-700h-500z"/></g></svg>"##
    );
    let mut s = with_svg_type("Svgtest Outlines", &doc);
    let r = ids(&s.execute("type.createOutlines", &json!({})).unwrap());
    let ch = node(&s, r[0]).children().unwrap().clone();
    assert!(matches!(ch[0].kind, NodeKind::Group { .. }), "the glyph's art: {:?}", ch[0].kind_label());
    let mut gradient = false;
    ch[0].walk(&mut |n| gradient |= matches!(n.appearance.fill_paint(), vectorcraft_color::Paint::Gradient(_)));
    assert!(gradient, "with its gradient");
    // Half an em wide at 32 pt, standing 0.7 em on the baseline at y 50.
    let b = ch[0].geometric_bounds().unwrap();
    assert!((b.width() - 16.0).abs() < 1e-6 && (b.y1 - 50.0).abs() < 1e-6 && (b.height() - 22.4).abs() < 1e-6, "{b:?}");
    assert!(matches!(ch[1].kind, NodeKind::Path { .. } | NodeKind::Compound { .. }), "the `F` an outline");
}

/// In a PDF with real text, an SVG glyph is text like any other (selectable, searchable), which
/// draws nothing itself, and its art is drawn where it sits: no outline of the font's covers it.
#[test]
fn pdf_real_text_draws_svg_glyphs_as_art_and_keeps_their_characters() {
    let _fonts = crate::adding_fonts();
    let g = source_sans_gid(SVG_CHAR).unwrap();
    let doc = format!(r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph{g}"><path fill="#f00" d="M0 0h500v-700h-500z"/></g></svg>"##);
    let s = with_svg_type("Svgtest Pdf", &doc);
    let d = s.doc().unwrap().doc.clone();
    let pdf = vectorcraft_pdf::export(&d, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    // The glyphs each text-showing line writes (the font's codes are octal escapes), and whether
    // it is invisible. Only text objects count: the file ID in the trailer and the XMP is random
    // base64, which now and then spells `Tj` or `TJ`.
    let raw = String::from_utf8_lossy(&pdf);
    let shown: Vec<(bool, usize)> = raw
        .lines()
        .filter(|l| l.starts_with("BT ") && (l.contains("Tj") || l.contains("TJ")))
        .map(|l| (l.contains("3 Tr"), l.matches("\\0").count()))
        .collect();
    assert_eq!(shown, [(false, 2)], "both characters once, as text");
    // The glyphs as the font draws them (its Type 3 glyph procedures, `<width> 0 d0` and what
    // they paint): the SVG glyph's draws nothing, rather than the font's plain outline over the
    // art; the `F` its outline.
    let procs: Vec<&str> = raw.split("stream\n").filter(|c| c.contains(" d0")).collect();
    assert_eq!(procs.len(), 2, "{procs:?}");
    assert_eq!(procs.iter().filter(|c| !c.contains(" Do")).count(), 1, "one glyph draws nothing: {procs:?}");
    let back = vectorcraft_pdf::import_with_report(&pdf, &vectorcraft_pdf::ImportOptions::default()).unwrap().document;
    let mut red = false;
    for l in &back.layers {
        l.walk(&mut |n| red |= n.appearance.fill_paint() == vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(255, 0, 0)));
    }
    assert!(red, "the glyph's art");
}

/// An SVG glyph whose document holds no art for it keeps its outline in PDF real text too (its
/// text draws nothing, as SVG glyphs' do).
#[test]
fn pdf_real_text_draws_the_outline_of_an_svg_glyph_without_art() {
    let _fonts = crate::adding_fonts();
    let doc = r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph1"/></svg>"##;
    let s = with_svg_type("Svgtest Bare Pdf", doc);
    let d = s.doc().unwrap().doc.clone();
    let pdf = vectorcraft_pdf::export(&d, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    // The `E`'s outline: from x 10 (its side bearing past it), 0.66 em tall, on the baseline at 50.
    let e = vectorcraft_geom::Rect::new(12.0, 32.0, 20.0, 48.0);
    assert!(fills_of(&pdf).iter().any(|(_, b)| b.intersect(e).area() > 10.0), "the glyph's outline");
}

/// In a font that may not be embedded, an SVG glyph's art is drawn once.
#[test]
fn pdf_draws_an_svg_glyphs_art_once_when_its_font_cant_be_embedded() {
    let _fonts = crate::adding_fonts();
    let g = source_sans_gid(SVG_CHAR).unwrap();
    let doc = format!(r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph{g}"><path fill="#f00" d="M0 0h500v-700h-500z"/></g></svg>"##);
    let mut s = session();
    vectorcraft_text::FontDb::global().add_font(not_embeddable(svg_font("Svgtest Locked", &[(g, g, &doc)]).unwrap()));
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": format!("{SVG_CHAR}F"), "size": 32, "font": "Svgtest Locked"})).unwrap();
    let d = s.doc().unwrap().doc.clone();
    let pdf = vectorcraft_pdf::export(&d, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    let red = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(255, 0, 0));
    assert_eq!(fills_of(&pdf).iter().filter(|(p, _)| *p == red).count(), 1);
}

/// Expand Appearance of type with an effect of its own (drawn reshaped as its outlines) keeps its
/// colour glyphs, as their art.
#[test]
fn expand_appearance_keeps_the_colour_glyphs_of_type_with_effects() {
    let _fonts = crate::adding_fonts();
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, colr_family, colr_font};
    vectorcraft_text::FontDb::global().add_font(colr_font(true).unwrap());
    let mut s = session();
    let r = s.execute("text.create", &json!({"x": 10, "y": 50, "text": format!("{COLOUR_CHAR}F"), "size": 32, "font": colr_family(true)})).unwrap();
    let t = NodeId(r["id"].as_u64().unwrap());
    s.execute("effect.apply", &json!({"ids": [t.0], "effect": "distort.transform", "params": {"moveH": 5}})).unwrap();
    let r = ids(&s.execute("effect.expandAppearance", &json!({})).unwrap());
    let mut gradients = 0;
    for id in r {
        node(&s, id).walk(&mut |n| gradients += usize::from(matches!(n.appearance.fill_paint(), vectorcraft_color::Paint::Gradient(_))));
    }
    assert!(gradients > 0, "the colour glyph's gradient layers");
    vectorcraft_testkit::invariants::check_document(&s.doc().unwrap().doc).unwrap();
}

/// The canvas draws an SVG glyph as its art: its own colours, and the type's fill where it uses
/// `currentColor`.
#[test]
fn the_canvas_draws_svg_glyphs_as_their_art() {
    let _fonts = crate::adding_fonts();
    let g = source_sans_gid(SVG_CHAR).unwrap();
    // The left half of a 500 by 700 box red, the right half in the text's colour.
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph{g}"><path fill="#f00" d="M0 0h250v-700h-250z"/><path fill="currentColor" d="M250 0h250v-700h-250z"/></g></svg>"##
    );
    let s = with_svg_type("Svgtest Canvas", &doc);
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let img = vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 100, 100, vectorcraft_geom::Affine::IDENTITY, &opts);
    // At 32 pt from x 10 on the baseline at y 50: 16 pt wide, 22.4 pt tall.
    assert_eq!(img.pixel(14, 40), [255, 0, 0, 255], "its own red");
    assert_eq!(img.pixel(22, 40), [0, 0, 0, 255], "the type's black fill");
}

/// Expand Appearance of type with an effect keeps its emoji as images, their pixels taken into
/// the document; the PDF of the type before expanding draws them too.
#[test]
fn expand_appearance_keeps_the_emoji_of_type_with_effects() {
    let _fonts = crate::adding_fonts();
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, COLOUR_LARGE, COLOUR_PPEM, colour_bitmap_family, colour_bitmap_font, solid_png};
    vectorcraft_text::FontDb::global().add_font(colour_bitmap_font(false).unwrap());
    let mut s = session();
    let r = s
        .execute("text.create", &json!({"x": 10, "y": 50, "text": format!("{COLOUR_CHAR}F"), "size": 32, "font": colour_bitmap_family(false)}))
        .unwrap();
    let t = NodeId(r["id"].as_u64().unwrap());
    // Turned, not only moved: type that a Transform only moves stays type in exports.
    s.execute("effect.apply", &json!({"ids": [t.0], "effect": "distort.transform", "params": {"rotate": 10}})).unwrap();
    let pdf = vectorcraft_pdf::export_with_report(&s.doc().unwrap().doc, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    assert!(pdf.warnings.is_empty(), "{:?}", pdf.warnings);
    assert!(pdf.bytes.windows(14).any(|w| w == b"/Subtype/Image"), "the emoji, in the PDF");
    let r = ids(&s.execute("effect.expandAppearance", &json!({})).unwrap());
    let mut images = vec![];
    for id in r {
        node(&s, id).walk(&mut |n| {
            if let NodeKind::Image(im) = &n.kind {
                images.push((**im).clone());
            }
        });
    }
    let d = &s.doc().unwrap().doc;
    let glyph = vectorcraft_doc::ImageBlob::png(solid_png(COLOUR_PPEM.into(), COLOUR_LARGE));
    assert_eq!(images.len(), 1, "the emoji");
    assert!(images[0].pixels.is_none(), "the document holds them");
    assert_eq!(d.images[&images[0].key].bytes, glyph.bytes);
    vectorcraft_testkit::invariants::check_document(d).unwrap();
}

/// Emoji type in a warp envelope: SVG export writes each emoji's pixels once for all the warp's
/// pieces, and Object › Expand takes them into the document.
#[test]
fn an_envelope_shares_and_expands_the_emoji_of_its_type() {
    let _fonts = crate::adding_fonts();
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, colour_bitmap_family, colour_bitmap_font};
    vectorcraft_text::FontDb::global().add_font(colour_bitmap_font(false).unwrap());
    let mut s = session();
    let r = s
        .execute("text.create", &json!({"x": 10, "y": 50, "text": COLOUR_CHAR.to_string(), "size": 32, "font": colour_bitmap_family(false)}))
        .unwrap();
    s.execute("select.set", &json!({"ids": [r["id"]]})).unwrap();
    let e = NodeId(s.execute("object.envelope.makeWithWarp", &json!({"style": "arch", "bend": 50})).unwrap()["id"].as_u64().unwrap());
    let svg = vectorcraft_svg::export_full(&s.doc().unwrap().doc, &Default::default(), None).svg;
    assert_eq!(svg.matches("data:image/png").count(), 1, "one copy of the pixels");
    s.execute("object.envelope.expand", &json!({})).unwrap();
    let mut images = vec![];
    node(&s, e).walk(&mut |n| {
        if let NodeKind::Image(im) = &n.kind {
            images.push((**im).clone());
        }
    });
    let d = &s.doc().unwrap().doc;
    assert!(images.len() > 1, "the warp's pieces");
    assert!(images.iter().all(|im| im.pixels.is_none() && d.images.contains_key(&im.key)), "the document holds them");
    vectorcraft_testkit::invariants::check_document(d).unwrap();
}

/// An image in an SVG glyph's document draws on the canvas, and Create Outlines takes its pixels
/// into the document.
#[test]
fn svg_glyph_images_draw_and_outline() {
    let g = source_sans_gid(SVG_CHAR).unwrap();
    let png = vectorcraft_text::test_fonts::solid_png(4, [0, 0, 255, 255]);
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image id="glyph{g}" y="-700" width="500" height="700" xlink:href="data:image/png;base64,{}"/></svg>"##,
        vectorcraft_svg::base64_encode(&png)
    );
    let mut s = with_svg_type("Svgtest Images", &doc);
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let img = vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 100, 100, vectorcraft_geom::Affine::IDENTITY, &opts);
    // At 32 pt from x 10 on the baseline at y 50: 16 pt wide, 22.4 pt tall.
    assert_eq!(img.pixel(18, 40), [0, 0, 255, 255], "the glyph's image");
    let r = ids(&s.execute("type.createOutlines", &json!({})).unwrap());
    let mut images = vec![];
    node(&s, r[0]).walk(&mut |n| {
        if let NodeKind::Image(im) = &n.kind {
            images.push((**im).clone());
        }
    });
    let d = &s.doc().unwrap().doc;
    assert_eq!(images.len(), 1);
    assert!(images[0].pixels.is_none() && d.images[&images[0].key].bytes.as_slice() == png.as_slice(), "the document holds its pixels");
    vectorcraft_testkit::invariants::check_document(d).unwrap();
}
