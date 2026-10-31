//! Import fidelity: soft masks → opacity masks, isolated and knockout groups, tiling patterns →
//! pattern swatches, text → point type, mesh shadings → gradient meshes, gradient stop opacity,
//! shading Extend flags and shapes filled with a shading → gradient-filled paths; strokes stay
//! live strokes.

use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Knockout, LineCap, LineJoin, Node, NodeId, NodeKind, OpacityMask};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};

use crate::*;

/// A stream object with dictionary entries `dict`.
pub(crate) fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// A 100 × 100 pt page drawing `content` with `resources`, plus `extra` objects.
fn one_page(content: &str, resources: &str, extra: &[&str]) -> Vec<u8> {
    let page = PdfPage { resources: resources.into(), ..PdfPage::new(100.0, 100.0, content) };
    pdf_with(&[page], extra, None)
}

fn open(bytes: &[u8]) -> Document {
    import(bytes).unwrap()
}

fn open_with(bytes: &[u8], f: impl FnOnce(&mut ImportOptions)) -> ImportReport {
    let mut o = ImportOptions::default();
    f(&mut o);
    import_with_report(bytes, &o).unwrap()
}

/// Every node of the document, depth first.
fn all(d: &Document) -> Vec<Node> {
    let mut out = vec![];
    d.walk(|n| out.push(n.clone()));
    out
}

fn rect(r: Rect, c: Color) -> Node {
    Node::path(NodeId(0), shapes::rectangle(r), Appearance::basic(Paint::solid(c), Paint::None, 0.0))
}

fn add(d: &mut Document, mut n: Node) {
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

fn roundtrip(d: &Document) -> Document {
    open(&export(d, &PdfOptions::default()).unwrap())
}

/// The fill colour of a node's first fill.
fn fill_color(n: &Node) -> Option<Color> {
    n.appearance.fill().and_then(|f| f.paint.color())
}

fn near(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 0.6)
}

// ---------- soft masks ----------

/// A page filling a red square through soft mask `smask` (its group, object 5, draws `mask`).
fn masked(smask: &str, mask: &str) -> Vec<u8> {
    let g = first_extra(1);
    let group = stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceRGB >>", mask);
    one_page("/GS0 gs 1 0 0 rg 0 0 100 100 re f", &format!("/ExtGState << /GS0 << /SMask << /Type /Mask {smask} /G {g} 0 R >> >> >>"), &[&group])
}

fn the_mask(d: &Document) -> (Node, OpacityMask) {
    let n = all(d).into_iter().find(|n| n.extras().mask.is_some()).expect("a masked object");
    let m = n.extras().mask.as_deref().unwrap().clone();
    (n, m)
}

/// The two pixels of the image a soft mask of kind `s` draws: black, opaque on the left and clear
/// on the right (its own `/SMask`).
fn mask_image_pixels(s: &str) -> [[u8; 4]; 2] {
    let g = first_extra(1);
    let group = stream(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceRGB >> /Resources << /XObject << /Im0 {} 0 R >> >>",
            g + 1
        ),
        "q 100 0 0 100 0 0 cm /Im0 Do Q",
    );
    let image = stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode /SMask {} 0 R",
            g + 2
        ),
        "000000000000>",
    );
    let smask =
        stream("/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /ASCIIHexDecode", "FF00>");
    let bytes = one_page(
        "/GS0 gs 1 0 0 rg 0 0 100 100 re f",
        &format!("/ExtGState << /GS0 << /SMask << /Type /Mask /S /{s} /G {g} 0 R >> >> >>"),
        &[&group, &image, &smask],
    );
    let d = open(&bytes);
    let (_, m) = the_mask(&d);
    let mut keys = vec![];
    m.art.walk(&mut |n| {
        if let NodeKind::Image(im) = &n.kind {
            keys.push(im.key.clone());
        }
    });
    let [key] = keys.as_slice() else { panic!("one image in the mask art: {keys:?}") };
    let px = image::load_from_memory(&d.images[key].bytes).unwrap().to_rgba8();
    [px.get_pixel(0, 0).0, px.get_pixel(1, 0).0]
}

/// An alpha soft mask whose art is an image with its own alpha (some PDF writers rasterize masks
/// so): the image is read white with that alpha, so the mask's luminance is the alpha, not the
/// image's colour (ISO 32000-1 11.5.2: an alpha mask takes the group's alpha, not its colour).
#[test]
fn an_alpha_soft_mask_of_an_image_uses_the_images_alpha() {
    assert_eq!(mask_image_pixels("Alpha"), [[255, 255, 255, 255], [255, 255, 255, 0]]);
}

/// A luminosity soft mask takes the group's colour (ISO 32000-1 11.5.3): its image stays black.
#[test]
fn a_luminosity_soft_mask_of_an_image_keeps_the_images_colour() {
    assert_eq!(mask_image_pixels("Luminosity"), [[0, 0, 0, 255], [0, 0, 0, 0]]);
}

#[test]
fn a_luminosity_soft_mask_becomes_an_opacity_mask() {
    let d = open(&masked("/S /Luminosity", "1 1 1 rg 0 0 50 100 re f"));
    let (n, m) = the_mask(&d);
    assert!(n.path_data().is_some(), "the mask is on the square itself");
    assert!(m.clip && !m.invert);
    assert_eq!(fill_color(&m.art).map(|c| c.to_rgb()), Some([1.0, 1.0, 1.0]));
    assert!(near(m.art.geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 50.0, 100.0)), "{:?}", m.art.geometric_bounds());
    // A white backdrop: outside the art stays visible (no clip); an inverting transfer function inverts.
    let (_, m) = the_mask(&open(&masked("/S /Luminosity /BC [1 1 1]", "0 0 0 rg 0 0 50 100 re f")));
    assert!(!m.clip);
    let tr = "/TR << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>";
    let (_, m) = the_mask(&open(&masked(&format!("/S /Luminosity {tr}"), "1 1 1 rg 0 0 50 100 re f")));
    assert!(m.invert && m.clip);
}

#[test]
fn an_alpha_soft_mask_becomes_an_opacity_mask_of_its_shape() {
    // The art is painted white with its opacity kept: its luminance is its alpha.
    let bytes = {
        let g = first_extra(1);
        let group = stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceRGB >> /Resources << /ExtGState << /GA << /ca 0.5 >> >> >>",
            "/GA gs 0 0 1 rg 0 0 50 100 re f",
        );
        one_page(
            "/GS0 gs 1 0 0 rg 0 0 100 100 re f",
            &format!("/ExtGState << /GS0 << /SMask << /Type /Mask /S /Alpha /G {g} 0 R >> >> >>"),
            &[&group],
        )
    };
    let (_, m) = the_mask(&open(&bytes));
    assert!(m.clip && !m.invert);
    let f = m.art.appearance.fill().unwrap();
    assert_eq!(f.paint.color().map(|c| c.to_rgb()), Some([1.0, 1.0, 1.0]));
    assert!((f.opacity * m.art.opacity - 0.5).abs() < 0.01);
}

#[test]
fn opacity_masks_round_trip_through_pdf() {
    for (clip, invert) in [(true, false), (false, false), (true, true), (false, true)] {
        let mut d = Document::new(100.0, 100.0);
        let mut n = rect(Rect::new(10.0, 10.0, 90.0, 90.0), Color::rgb(1.0, 0.0, 0.0));
        let art = rect(Rect::new(10.0, 10.0, 50.0, 90.0), Color::gray(0.25));
        n.extras_mut().mask = Some(Box::new(OpacityMask { invert, ..OpacityMask::new(art, clip) }));
        add(&mut d, n);
        let back = roundtrip(&d);
        let (n, m) = the_mask(&back);
        assert!(near(n.geometric_bounds().unwrap(), Rect::new(10.0, 10.0, 90.0, 90.0)));
        assert_eq!((m.clip, m.invert), (clip, invert), "clip {clip}, invert {invert}");
        assert!(near(m.art.geometric_bounds().unwrap(), Rect::new(10.0, 10.0, 50.0, 90.0)), "{:?}", m.art.geometric_bounds());
        let lum = crate::import_mask::luminance(&fill_color(&m.art).unwrap());
        assert!((lum - 0.75).abs() < 0.02, "{lum}");
    }
}

// ---------- isolation and knockout ----------

/// A 50% group (isolated or not) holding a Multiply square over a backdrop.
fn half_group(isolate: bool) -> Document {
    let mut d = Document::new(100.0, 100.0);
    add(&mut d, rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(0.8, 0.6, 0.2)));
    let mut m = rect(Rect::new(20.0, 20.0, 80.0, 80.0), Color::rgb(0.5, 0.5, 1.0));
    (m.id, m.blend) = (d.alloc_id(), BlendMode::Multiply);
    let mut g = Node::group(NodeId(0), vec![Arc::new(m)]);
    (g.opacity, g.isolate) = (0.5, isolate);
    add(&mut d, g);
    d
}

#[test]
fn isolated_and_non_isolated_groups_round_trip() {
    let back = roundtrip(&half_group(true));
    let groups: Vec<Node> = all(&back).into_iter().filter(|n| matches!(n.kind, NodeKind::Group { clip: false, .. })).collect();
    let [g] = groups.as_slice() else { panic!("one group: {groups:?}") };
    assert!(g.isolate && g.extras().mask.is_none());
    assert!((g.opacity - 0.5).abs() < 0.01, "{}", g.opacity);
    assert_eq!(g.children().unwrap()[0].blend, BlendMode::Multiply);
    // Not isolated: its 50% (a constant alpha mask in the file) is opacity again, here on the
    // only object inside, which blends with the backdrop as before.
    let back = roundtrip(&half_group(false));
    let nodes = all(&back);
    assert!(nodes.iter().all(|n| !n.isolate && n.extras().mask.is_none()));
    let m = nodes.iter().find(|n| n.blend == BlendMode::Multiply).unwrap();
    let opacity: f32 =
        nodes.iter().filter(|n| n.children().is_some_and(|c| c.iter().any(|c| c.id == m.id))).map(|g| g.opacity).product::<f32>() * m.opacity;
    assert!((opacity - 0.5).abs() < 0.01, "{opacity}");
}

#[test]
fn isolate_and_knockout_flags_of_a_group_are_kept() {
    let x = first_extra(1);
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /I true /K true >> /Resources << /ExtGState << /M << /BM /Multiply >> >> >>",
        "/M gs 1 0 0 rg 10 10 50 50 re f 0 0 1 rg 30 30 50 50 re f",
    );
    let d = open(&one_page("0 1 0 rg 0 0 100 100 re f /X0 Do", &format!("/XObject << /X0 {x} 0 R >>"), &[&form]));
    let groups: Vec<Node> = all(&d).into_iter().filter(|n| matches!(n.kind, NodeKind::Group { .. })).collect();
    let [g] = groups.as_slice() else { panic!("one group: {groups:?}") };
    assert!(g.isolate);
    assert_eq!(g.knockout, Knockout::On);
    assert_eq!(g.children().unwrap().len(), 2);
    // Without the flags, the form's group is just its art.
    let plain = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >>",
        "1 0 0 rg 10 10 50 50 re f 0 0 1 rg 30 30 50 50 re f",
    );
    let d = open(&one_page("/X0 Do", &format!("/XObject << /X0 {x} 0 R >>"), &[&plain]));
    assert!(all(&d).iter().all(|n| !matches!(n.kind, NodeKind::Group { .. })));
}

#[test]
fn knockout_groups_after_a_group_drawn_straight_keep_their_flag() {
    // Two knockout groups (ISO 32000-1, 11.4.6.3), each of a full-opacity element and a 50% one,
    // both elements groups of their own. A full-opacity element adds nothing as a group and is
    // drawn straight onto its parent; the knockout group after it still knocks out.
    let x = first_extra(1);
    let ring = |at: usize| {
        let res = format!("/ExtGState << /A << /ca 1 >> /B << /ca 0.5 >> >> /XObject << /E {} 0 R /F {} 0 R >>", at, at + 1);
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /K true >> /Resources << {res} >>"),
            "q /A gs /E Do Q q /B gs /F Do Q",
        )
    };
    let element = |art: &str| stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >>", art);
    let d = open(&one_page(
        "/X0 Do /X1 Do",
        &format!("/XObject << /X0 {x} 0 R /X1 {} 0 R >>", x + 1),
        &[
            &ring(x + 2),
            &ring(x + 4),
            &element("0 0 1 rg 10 10 40 40 re f"),
            &element("0 0 1 rg 20 20 20 20 re f"),
            &element("0 0 1 rg 50 50 40 40 re f"),
            &element("0 0 1 rg 60 60 20 20 re f"),
        ],
    ));
    let groups: Vec<Node> = all(&d).into_iter().filter(|n| matches!(n.kind, NodeKind::Group { .. })).collect();
    assert_eq!(groups.len(), 2, "{groups:?}");
    for g in &groups {
        assert_eq!(g.knockout, Knockout::On, "{g:?}");
        let opacities: Vec<f32> = g.children().unwrap().iter().map(|c| c.opacity).collect();
        assert_eq!(opacities, [1.0, 0.5]);
    }
}

// ---------- tiling patterns ----------

#[test]
fn tiling_patterns_become_pattern_swatches() {
    let p = first_extra(1);
    let colored = stream("/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>", "1 0 0 rg 0 0 5 5 re f");
    let d = open(&one_page("/Pattern cs /P0 scn 0 0 100 50 re f 0 50 100 50 re f", &format!("/Pattern << /P0 {p} 0 R >>"), &[&colored]));
    assert_eq!(d.patterns.len(), 1, "one swatch for both fills");
    let def = &d.patterns[0];
    assert!(near(def.tile, Rect::new(0.0, 0.0, 10.0, 10.0)), "{:?}", def.tile);
    assert_eq!(def.art.len(), 1);
    assert_eq!(fill_color(&def.art[0]), Some(Color::rgb(1.0, 0.0, 0.0)));
    assert!(d.swatches.iter().any(|s| matches!(&s.paint, Paint::Pattern { pattern, .. } if *pattern == def.name)), "a pattern swatch");
    let fills: Vec<Paint> = all(&d).iter().filter_map(|n| n.appearance.fill().map(|f| f.paint.clone())).collect();
    assert_eq!(fills.len(), 2);
    assert!(fills.iter().all(|f| matches!(f, Paint::Pattern { pattern, .. } if *pattern == def.name)), "{fills:?}");
    // An uncoloured pattern draws in the current colour: one swatch per colour.
    let stencil = stream("/PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>", "0 0 5 5 re f");
    let d = open(&one_page(
        "/CS0 cs 1 0 0 /P0 scn 0 0 100 50 re f 0 0 1 /P0 scn 0 50 100 50 re f",
        &format!("/Pattern << /P0 {p} 0 R >> /ColorSpace << /CS0 [/Pattern /DeviceRGB] >>"),
        &[&stencil],
    ));
    let colors: Vec<Color> = d.patterns.iter().filter_map(|p| fill_color(&p.art[0])).collect();
    assert_eq!(colors, vec![Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0)]);
}

// ---------- text ----------

const HELVETICA: &str =
    "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> /F2 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >> >>";

fn texts(d: &Document) -> Vec<vectorcraft_doc::TextObject> {
    all(d).into_iter().filter_map(|n| if let NodeKind::Text(t) = n.kind { Some(*t) } else { None }).collect()
}

#[test]
fn text_becomes_point_type() {
    let bytes = one_page(
        "BT /F1 24 Tf 1 0 0 rg 10 30 Td (Hello) Tj [(Wor) -20 (ld)] TJ ET BT /F1 12 Tf 10 70 Td [(Next) -1000 (line)] TJ /F2 12 Tf 0 0 1 rg ( in bold) Tj ET",
        HELVETICA,
        &[],
    );
    let r = open_with(&bytes, |_| {});
    let t = texts(&r.document);
    assert_eq!(t.len(), 2, "{t:?}");
    let text = |o: &vectorcraft_doc::TextObject| o.runs.iter().map(|r| r.text.as_str()).collect::<String>();
    assert_eq!(text(&t[0]), "HelloWorld");
    let st = &t[0].runs[0].style;
    assert_eq!((st.font_family.as_str(), st.font_style.as_str(), st.size), ("Helvetica", "Regular", 24.0));
    assert_eq!(st.fill.color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    // The baseline origin, in y-down page space.
    let o = t[0].xf * kurbo::Point::ORIGIN;
    assert!((o.x - 10.0).abs() < 0.01 && (o.y - 70.0).abs() < 0.01, "{o:?}");
    // A gap of an em between words reads as a space, tracked on its own to keep the gap's width
    // (#508); another font on the line is another run.
    assert_eq!(text(&t[1]), "Next line in bold");
    let runs: Vec<(&str, &str)> = t[1].runs.iter().map(|r| (r.text.as_str(), r.style.font_style.as_str())).collect();
    assert_eq!(runs, [("Next", "Regular"), (" ", "Regular"), ("line", "Regular"), (" in bold", "Bold")]);
    assert!(t[1].runs[1].style.tracking > t[1].runs[0].style.tracking + 100.0, "{:?}", t[1].runs);
    assert_eq!(t[1].runs[3].style.fill.color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert!(!r.warnings.iter().any(|w| w.contains("outlines")), "{:?}", r.warnings);
    // As outlines: paths, no type.
    let r = open_with(&bytes, |o| o.text_as = TextAs::Outlines);
    assert!(texts(&r.document).is_empty());
    assert!(all(&r.document).iter().any(|n| n.name.as_deref() == Some("<Text Outlines>")));
}

#[test]
fn character_spacing_does_not_become_spaces() {
    for size in [12.0, 13.0, 14.0, 15.0] {
        for tracking in [150.0, 180.0, 200.0, 220.0, 250.0] {
            let spacing = size * tracking / 1000.0;
            let content = format!("BT /F1 {size} Tf {spacing} Tc 10 40 Td (OPEN DAILY) Tj ET");
            let t = texts(&open(&one_page(&content, HELVETICA, &[])));
            assert_eq!(t.len(), 1, "size {size}, tracking {tracking}: {t:?}");
            assert_eq!(t[0].plain_text(), "OPEN DAILY", "size {size}, tracking {tracking}");
        }
    }
}

/// Invisible text (rendering mode 3: a scanned page's recognized text, laid over the scan) is
/// type with neither fill nor stroke, and the visible text keeps its paint. On a line with
/// stroked text it is a run of its own, not stroked. As outlines it would draw nothing: it is left
/// out, and the open says so (#2675).
#[test]
fn invisible_text_becomes_type_without_paint() {
    let bytes = one_page(
        "0.5 g 10 40 80 40 re f \
         BT /F1 10 Tf 3 Tr 12 70 Td (Invisible one) Tj 0 -12 Td (Invisible two) Tj ET \
         BT /F1 10 Tf 0 g 0 Tr 12 20 Td (Visible) Tj ET \
         BT /F1 10 Tf 1 0 0 RG 1 Tr 12 90 Td (Out) Tj 3 Tr (line) Tj ET",
        HELVETICA,
        &[],
    );
    let r = open_with(&bytes, |_| {});
    let t = texts(&r.document);
    let text = |o: &vectorcraft_doc::TextObject| o.runs.iter().map(|r| r.text.as_str()).collect::<String>();
    assert_eq!(t.iter().map(text).collect::<Vec<_>>(), ["Invisible one", "Invisible two", "Visible", "Outline"], "{t:?}");
    let unpainted = |st: &vectorcraft_doc::CharStyle| st.fill.is_none() && !st.has_stroke();
    for o in &t[..2] {
        assert!(o.runs.iter().all(|r| unpainted(&r.style)), "{:?}", o.runs);
    }
    assert!(t[2].runs.iter().all(|r| !r.style.fill.is_none()), "{:?}", t[2].runs);
    let runs: Vec<(&str, bool)> = t[3].runs.iter().map(|r| (r.text.as_str(), unpainted(&r.style))).collect();
    assert_eq!(runs, [("Out", false), ("line", true)]);
    assert!(t[3].runs[0].style.has_stroke());
    assert!(!r.warnings.iter().any(|w| w == INVISIBLE_TEXT_NOTE), "{:?}", r.warnings);
    // As outlines: only the visible text's.
    let r = open_with(&bytes, |o| o.text_as = TextAs::Outlines);
    assert!(texts(&r.document).is_empty());
    assert!(r.warnings.iter().any(|w| w == INVISIBLE_TEXT_NOTE), "{:?}", r.warnings);
    let outlines: Vec<Rect> =
        all(&r.document).iter().filter(|n| n.name.as_deref() == Some("<Text Outlines>")).filter_map(|n| n.geometric_bounds()).collect();
    // y down: "Visible" sits on 80, "Out" on 10; the invisible lines on 30 and 42 left nothing.
    assert!(!outlines.is_empty() && outlines.iter().all(|b| b.y1 < 20.0 || b.y0 > 70.0), "{outlines:?}");
}

#[test]
fn font_names_resolve_to_available_families() {
    let f = crate::import_text::Families::available();
    let (family, style, found) = f.resolve("ABCDEF+SourceSans3-Bold", None, false);
    assert_eq!((family.as_str(), style.as_str(), found), ("Source Sans 3", "Bold", true));
    let (family, style, found) = f.resolve("TimesNewRomanPS-BoldItalicMT", None, false);
    assert_eq!((family.as_str(), style.as_str()), ("Times New Roman", "Bold Italic"));
    let _ = found;
    let (family, style, found) = f.resolve("MissingSansPro", Some(700), true);
    assert_eq!((family.as_str(), style.as_str(), found), ("Missing Sans Pro", "Bold Italic", false));
}

/// A value of the page's font dictionary that the reader can't parse (a name with a broken `#`
/// escape, a number with a stray `-`) is left out when the page's fonts are noted, and the fonts
/// after it are noted.
#[test]
fn fonts_are_noted_past_font_resources_the_reader_cannot_parse() {
    for junk in ["/#zz", "12-.5"] {
        let resources = format!("/Font << /A {junk} /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>");
        let bytes = one_page("BT /F1 24 Tf 10 30 Td (Hello) Tj ET", &resources, &[]);
        // Opened by the reader alone, without the checks of `pages::open`.
        let pdf = craft_pdf::syntax::Pdf::new(bytes).unwrap();
        let mut fonts = crate::import_scan::FontNames::new();
        let mut ocgs = crate::import_scan::Ocgs::read(&pdf);
        crate::import_scan::scan_page(&pdf.pages()[0], &mut ocgs, false, &mut fonts, pdf.decode_limits().max_decoded_stream()).unwrap();
        let noted: Vec<_> = fonts.into_values().map(|f| (f.name, f.embedded)).collect();
        assert_eq!(noted, [("Helvetica".to_string(), false)], "{junk}");
    }
}

// ---------- meshes, stops and extend ----------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join("") + ">"
}

/// A Coons patch mesh shading's dictionary for [`coons_patch`] data.
const COONS_MESH: &str = "/ShadingType 6 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 255 0 255 0 1 0 1 0 1] /Filter /ASCIIHexDecode";

/// One patch over (10, 10)–(90, 90): corner colours red (0, 0), green (0, 1), blue (1, 1), white
/// (1, 0) in u/v.
fn coons_patch() -> Vec<u8> {
    let pts: [(u8, u8); 12] =
        [(10, 10), (10, 37), (10, 63), (10, 90), (37, 90), (63, 90), (90, 90), (90, 63), (90, 37), (90, 10), (63, 10), (37, 10)];
    let mut data = vec![0u8];
    for (x, y) in pts {
        data.extend([x, y]);
    }
    data.extend([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
    data
}

#[test]
fn a_coons_patch_mesh_becomes_a_gradient_mesh() {
    let s = first_extra(1);
    let sh = stream(COONS_MESH, &hex(&coons_patch()));
    let d = open(&one_page("/Sh0 sh", &format!("/Shading << /Sh0 {s} 0 R >>"), &[&sh]));
    let meshes: Vec<vectorcraft_doc::live::GradientMesh> =
        all(&d).into_iter().filter_map(|n| if let NodeKind::Mesh(m) = n.kind { Some(m) } else { None }).collect();
    let [m] = meshes.as_slice() else { panic!("one mesh: {meshes:?}") };
    assert_eq!((m.rows, m.cols), (1, 1));
    let at = |r, c| &m.points[m.idx(r, c)];
    // Row 0 is v = 0 (PDF y 10 → page y 90), column 1 is u = 1.
    assert!((at(0, 0).p - kurbo::Point::new(10.0, 90.0)).hypot() < 0.01, "{:?}", at(0, 0).p);
    assert!((at(1, 1).p - kurbo::Point::new(90.0, 10.0)).hypot() < 0.01, "{:?}", at(1, 1).p);
    assert_eq!(at(0, 0).color.to_rgb(), [1.0, 0.0, 0.0]);
    assert_eq!(at(1, 0).color.to_rgb(), [0.0, 1.0, 0.0]);
    assert_eq!(at(1, 1).color.to_rgb(), [0.0, 0.0, 1.0]);
    assert_eq!(at(0, 1).color.to_rgb(), [1.0, 1.0, 1.0]);
    assert!(!all(&d).iter().any(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })), "the mesh lies inside the page");
}

/// Illustrator writes meshes of many thousands of patches (14,355 in one of a user's files): one
/// of 5,000 comes in as gradient meshes, not a flat grey.
#[test]
fn a_mesh_shading_of_thousands_of_patches_becomes_gradient_meshes() {
    let data: Vec<u8> = std::iter::repeat_n(coons_patch(), 5000).flatten().collect();
    let s = first_extra(1);
    let sh = stream(COONS_MESH, &hex(&data));
    let r = open_with(&one_page("/Sh0 sh", &format!("/Shading << /Sh0 {s} 0 R >>"), &[&sh]), |_| {});
    assert!(!r.warnings.iter().any(|w| w.contains("flat grey")), "{:?}", r.warnings);
    assert_eq!(all(&r.document).iter().filter(|n| matches!(n.kind, NodeKind::Mesh(_))).count(), 5000);
}

/// A stroke painted with a mesh shading (how Illustrator writes a freeform gradient on a stroke)
/// is the mesh clipped to the area the stroke paints, dashes included, not a flat grey.
#[test]
fn a_stroke_painted_with_a_mesh_shading_is_the_mesh_clipped_to_the_stroke() {
    let s = first_extra(1);
    let sh = stream(COONS_MESH, &hex(&coons_patch()));
    let resources = format!("/Pattern << /P0 << /PatternType 2 /Shading {s} 0 R >> >>");
    // A 10 pt line from (20, 50) to (80, 50) with butt caps: page y 45 to 55. Dashed 10 on, 10
    // off: pieces at 20–30, 40–50 and 60–70.
    for (dash, pieces, end) in [("[] 0 d", 1, 80.0), ("[10 10] 0 d", 3, 70.0)] {
        let content = format!("/Pattern CS /P0 SCN 10 w {dash} 20 50 m 80 50 l S");
        let r = open_with(&one_page(&content, &resources, &[&sh]), |_| {});
        assert!(!r.warnings.iter().any(|w| w.contains("flat grey")), "{:?}", r.warnings);
        let nodes = all(&r.document);
        assert_eq!(nodes.iter().filter(|n| matches!(n.kind, NodeKind::Mesh(_))).count(), 1, "{dash}");
        let clip = nodes
            .iter()
            .find_map(|n| match &n.kind {
                NodeKind::Group { children, clip: true } => children.first().cloned(),
                _ => None,
            })
            .expect("a clip group");
        let NodeKind::Path { path, .. } = &clip.kind else { panic!("{:?}", clip.kind) };
        let b = path.bounds().unwrap();
        assert!((b.x0 - 20.0).abs() < 0.01 && (b.x1 - end).abs() < 0.01 && (b.y0 - 45.0).abs() < 0.01 && (b.y1 - 55.0).abs() < 0.01, "{dash}: {b:?}");
        let subpaths = path.to_bezpath().elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
        assert_eq!(subpaths, pieces, "{dash}");
    }
}

#[test]
fn gradient_stop_opacity_round_trips() {
    let mut d = Document::new(100.0, 100.0);
    let mut stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    stops[1].opacity = 0.2;
    let g = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)),
        Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 0.0),
    );
    add(&mut d, n);
    let back = roundtrip(&d);
    let nodes = all(&back);
    assert!(nodes.iter().all(|n| n.extras().mask.is_none()), "the opacity is in the stops, not a mask");
    let g = nodes.iter().find_map(|n| match n.appearance.fill().map(|f| &f.paint) {
        Some(Paint::Gradient(g)) => Some(g.gradient.clone()),
        _ => None,
    });
    let g = g.expect("a gradient fill");
    let first = g.stops.first().unwrap();
    let last = g.stops.last().unwrap();
    assert!((first.opacity - 1.0).abs() < 0.02 && (last.opacity - 0.2).abs() < 0.02, "{:?}", g.stops);
    assert_eq!(last.color.to_rgb(), [0.0, 0.0, 1.0]);
}

#[test]
fn a_shading_that_does_not_extend_paints_only_between_its_ends() {
    let shading = |extend: &str| {
        format!(
            "/Shading << /Sh0 << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [20 0 80 0] /Extend [{extend}] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>"
        )
    };
    let d = open(&one_page("/Sh0 sh", &shading("false false"), &[]));
    let clips: Vec<Node> = all(&d).into_iter().filter(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })).collect();
    let [c] = clips.as_slice() else { panic!("one clip group: {clips:?}") };
    let b = c.geometric_bounds().unwrap();
    assert!((b.x0 - 20.0).abs() < 0.01 && (b.x1 - 80.0).abs() < 0.01 && b.y0 <= 0.0 && b.y1 >= 100.0, "{b:?}");
    let d = open(&one_page("/Sh0 sh", &shading("true true"), &[]));
    assert!(!all(&d).iter().any(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })));
    // Extended at the start only: from the left edge to the end.
    let d = open(&one_page("/Sh0 sh", &shading("true false"), &[]));
    let b = all(&d).into_iter().find(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })).unwrap().geometric_bounds().unwrap();
    assert!(b.x0 <= 0.0 && (b.x1 - 80.0).abs() < 0.01, "{b:?}");
}

/// GUARDS: an exponential function with N off 1 (how a gradient midpoint is written) keeps its
/// curve: sampled stops must follow it, not collapse to the two ends because neighbouring samples
/// look straight.
#[test]
fn a_shading_with_an_exponent_keeps_its_curve() {
    let shading = "/Shading << /Sh0 << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [1 1 1] /N 1.4234 >> >> >>";
    let d = open(&one_page("/Sh0 sh", shading, &[]));
    let g = all(&d)
        .into_iter()
        .find_map(|n| n.appearance.items.iter().find_map(|i| if let Paint::Gradient(g) = i.paint() { Some(g.clone()) } else { None }))
        .expect("a gradient");
    assert!(g.gradient.stops.len() > 2, "{:?}", g.gradient.stops);
    // Halfway along, the curve is at 0.5^1.4234 ≈ 0.373, well off the straight 0.5.
    let (c, _) = g.gradient.sample(0.5);
    let v = c.to_rgb()[0];
    assert!((v - 0.5f32.powf(1.4234)).abs() < 0.01, "{v}");
}

/// A 100 × 100 square filled with a function-based shading (type 1) over `domain`, placed by
/// `matrix`, whose PostScript calculator function is `program` (x y → r g b).
fn function_shading(domain: &str, matrix: &str, program: &str) -> ImportReport {
    let f = first_extra(1);
    let resources = format!(
        "/Pattern << /P0 << /PatternType 2 /Shading << /ShadingType 1 /ColorSpace /DeviceRGB /Domain [{domain}] /Matrix [{matrix}] /Function {f} 0 R >> >> >>"
    );
    let function = stream(&format!("/FunctionType 4 /Domain [{domain}] /Range [0 1 0 1 0 1]"), program);
    open_with(&one_page("/Pattern cs /P0 scn 0 0 100 100 re f", &resources, &[&function]), |_| {})
}

/// A browser writes a repeating linear gradient as a function-based shading whose colour changes
/// along one axis only: it comes in as a linear gradient with hard stops, red and white as in the
/// file, not as a flat grey (#3621).
#[test]
fn a_function_based_shading_along_one_axis_becomes_a_linear_gradient() {
    let _held = shadings();
    // Four stripes of red, then white, across the square (x 0..4 over 100 pt).
    let r = function_shading("0 4 0 1", "25 0 0 100 0 0", "{ pop dup truncate sub 0.5 le { 1 0 0 } { 1 1 1 } ifelse }");
    assert!(!r.warnings.iter().any(|w| w.contains("flat")), "{:?}", r.warnings);
    let g = all(&r.document)
        .into_iter()
        .find_map(|n| match n.appearance.fill().map(|f| &f.paint) {
            Some(Paint::Gradient(g)) => Some(g.clone()),
            _ => None,
        })
        .expect("a gradient fill");
    assert_eq!(g.gradient.kind, GradientKind::Linear);
    let (red, white) = ([1.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    assert!(g.gradient.stops.iter().all(|s| s.color.to_rgb() == red || s.color.to_rgb() == white), "{:?}", g.gradient.stops);
    let geom = g.geom.as_ref().unwrap();
    assert!((geom.start.x - 0.0).abs() < 0.01 && (geom.end.x - 100.0).abs() < 0.01, "across the square: {geom:?}");
    for (t, want) in [(0.06, red), (0.19, white), (0.31, red), (0.44, white), (0.56, red), (0.94, white)] {
        assert_eq!(g.gradient.sample(t).0.to_rgb(), want, "at {t}");
    }
    // The colour changes where the file's does (x = 0.5, 1 … 3.5, and at x = 4 itself, red
    // again): hard stops, not ramps.
    let edges: Vec<f32> =
        g.gradient.stops.windows(2).filter(|w| (w[0].offset - w[1].offset).abs() < 1e-4 && w[0].color != w[1].color).map(|w| w[0].offset).collect();
    assert_eq!(edges.len(), 8, "{:?}", g.gradient.stops);
    for (k, e) in edges.iter().take(7).enumerate() {
        assert!((e - (k + 1) as f32 / 8.0).abs() < 1e-4, "change {k} at {e}");
    }
}

/// A function-based shading paints only over its domain: a square filled with one whose domain
/// covers its left half is clipped to that half; one whose domain covers it all isn't clipped.
#[test]
fn a_function_based_shading_paints_only_over_its_domain() {
    let _held = shadings();
    let clips = |matrix: &str| -> Vec<Node> {
        let r = function_shading("0 1 0 1", matrix, "{ pop 0 0 }");
        all(&r.document).into_iter().filter(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })).collect()
    };
    let half = clips("50 0 0 100 0 0");
    let [c] = half.as_slice() else { panic!("one clip group: {half:?}") };
    let b = c.geometric_bounds().unwrap();
    assert!(b.x0.abs() < 0.01 && (b.x1 - 50.0).abs() < 0.01 && b.y0.abs() < 0.01 && (b.y1 - 100.0).abs() < 0.01, "{b:?}");
    assert!(clips("100 0 0 100 0 0").is_empty());
}

/// Held by tests that import function-based shadings, so the evaluations one of them counts are
/// its own.
pub(crate) static SHADINGS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn shadings() -> std::sync::MutexGuard<'static, ()> {
    SHADINGS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The function evaluations function-based shadings take while `f` imports (hold [`shadings`]).
fn evaluations(f: impl FnOnce() -> ImportReport) -> (ImportReport, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    crate::import_shading::EVALUATIONS.store(0, Relaxed);
    let r = f();
    (r, crate::import_shading::EVALUATIONS.load(Relaxed))
}

/// Stripes across the square: red, then white, `n` times (x 0..n over 100 pt).
const STRIPES: &str = "{ pop dup truncate sub 0.5 le { 1 0 0 } { 1 1 1 } ifelse }";

/// A shading used by many paths is read once (#3621 review): a file can't make the import
/// evaluate its function once per path.
#[test]
fn a_function_based_shading_filling_many_paths_is_read_once() {
    let _held = shadings();
    let f = first_extra(1);
    let resources = format!(
        "/Pattern << /P0 << /PatternType 2 /Shading << /ShadingType 1 /ColorSpace /DeviceRGB /Domain [0 4 0 1] /Matrix [25 0 0 100 0 0] /Function {f} 0 R >> >> >>"
    );
    let function = stream("/FunctionType 4 /Domain [0 4 0 1] /Range [0 1 0 1 0 1]", STRIPES);
    let content: String = (0..2000).map(|i| format!("{} {} 2 2 re f ", i % 50 * 2, i / 50 * 2)).collect();
    let once = evaluations(|| function_shading("0 4 0 1", "25 0 0 100 0 0", STRIPES)).1;
    let (r, many) = evaluations(|| open_with(&one_page(&format!("/Pattern cs /P0 scn {content}"), &resources, &[&function]), |_| {}));
    assert!(once > 0 && many <= once, "one path {once}, 2000 paths {many}");
    assert!(all(&r.document).iter().any(|n| matches!(n.appearance.fill().map(|f| &f.paint), Some(Paint::Gradient(_)))));
}

/// Shadings that each take many evaluations share one budget per import: past it they come in as
/// their average colour, then grey, with a warning, and the import stays bounded.
#[test]
fn function_based_shadings_share_one_budget_per_import() {
    let _held = shadings();
    // 40 shadings, each other stripes (fine ones: each takes thousands of evaluations).
    let n = 40;
    let patterns: String = (0..n)
        .map(|i| {
            format!(
                "/P{i} << /PatternType 2 /Shading << /ShadingType 1 /ColorSpace /DeviceRGB /Domain [0 {w} 0 1] /Matrix [{s} 0 0 100 0 0] /Function {} 0 R >> >> ",
                first_extra(1) + i,
                w = 100 + i,
                s = 100.0 / (100 + i) as f64
            )
        })
        .collect();
    let functions: Vec<String> =
        (0..n).map(|i| stream(&format!("/FunctionType 4 /Domain [0 {} 0 1] /Range [0 1 0 1 0 1]", 100 + i), STRIPES)).collect();
    let content: String = (0..n).map(|i| format!("/Pattern cs /P{i} scn 0 {i} 100 1 re f ")).collect();
    let extra: Vec<&str> = functions.iter().map(String::as_str).collect();
    let (r, used) = evaluations(|| open_with(&one_page(&content, &format!("/Pattern << {patterns}>>"), &extra), |_| {}));
    assert!(used <= crate::import_shading::MAX_FUNCTION_EVALUATIONS, "{used}");
    let gradients = all(&r.document).iter().filter(|n| matches!(n.appearance.fill().map(|f| &f.paint), Some(Paint::Gradient(_)))).count();
    assert!(gradients > 0 && gradients < n, "{gradients} of {n} read as gradients");
    assert!(r.warnings.iter().any(|w| w.contains("too many to read")), "{:?}", r.warnings);
}

/// A page drawing one small form over and over makes objects only up to the import's limit, then
/// leaves the rest out with a warning, instead of taking memory for every drawing (#3569).
#[test]
fn a_form_drawn_over_and_over_stops_at_the_import_s_objects() {
    let squares: String = (0..25).map(|i| format!("{} {} 2 2 re f ", i % 5 * 4, i / 5 * 4)).collect();
    let form = stream("/Type /XObject /Subtype /Form /BBox [0 0 20 20]", &squares);
    let bytes = one_page(&"/F Do ".repeat(200), &format!("/XObject << /F {} 0 R >>", first_extra(1)), &[&form]);
    // With the import's objects lowered to `limit` (on this thread: the limit is this thread's).
    let open_within = |limit: usize| {
        let old = crate::import::TEST_MAX_OBJECTS.replace(limit);
        let r = crate::import::read(&bytes, &ImportOptions::default());
        crate::import::TEST_MAX_OBJECTS.set(old);
        r.unwrap()
    };
    let paths = |r: &ImportReport| all(&r.document).iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).count();
    let left_out = |r: &ImportReport| r.warnings.iter().any(|w| w.contains("the rest were left out"));
    let whole = open_within(10_000);
    assert_eq!(paths(&whole), 5000);
    assert!(!left_out(&whole), "{:?}", whole.warnings);
    let cut = open_within(1000);
    assert_eq!(paths(&cut), 1000);
    assert!(left_out(&cut), "{:?}", cut.warnings);
}

/// A colour that changes in step with the sampling grid along y only (one colour at every grid
/// point) is still read along y, not as a flat gradient along x.
#[test]
fn a_function_based_shading_in_step_with_the_grid_is_read_along_its_axis() {
    let _held = shadings();
    // Red in bands a quarter of a period wide around every grid row (17·y − 0.37 a whole number).
    let r = function_shading("0 1 0 1", "100 0 0 100 0 0", "{ exch pop 17 mul 0.37 sub 0.125 add dup floor sub 0.25 lt { 1 0 0 } { 1 1 1 } ifelse }");
    let g = all(&r.document)
        .into_iter()
        .find_map(|n| match n.appearance.fill().map(|f| &f.paint) {
            Some(Paint::Gradient(g)) => Some(g.clone()),
            _ => None,
        })
        .expect("a gradient fill");
    let geom = g.geom.as_ref().unwrap();
    assert!((geom.start.x - geom.end.x).abs() < 0.01 && (geom.start.y - geom.end.y).abs() > 99.0, "along y: {geom:?}");
    assert!(g.gradient.stops.iter().any(|s| s.color.to_rgb() == [1.0, 1.0, 1.0]), "{:?}", g.gradient.stops);
}

/// A function-based shading whose colour changes along both axes has no gradient: it comes in as
/// the colour it averages to, never as a grey that isn't in it.
#[test]
fn a_function_based_shading_along_both_axes_becomes_its_average_colour() {
    let _held = shadings();
    // r = g = x·y, b = 0 over the unit square: averages to (0.25, 0.25, 0).
    let r = function_shading("0 1 0 1", "100 0 0 100 0 0", "{ mul dup 0 }");
    let c = all(&r.document).iter().find_map(fill_color).expect("a flat fill").to_rgb();
    assert!((c[0] - 0.25).abs() < 0.03 && (c[1] - 0.25).abs() < 0.03 && c[2].abs() < 1e-6, "{c:?}");
    assert!(r.warnings.iter().any(|w| w.contains("average")), "{:?}", r.warnings);
}

/// A diamond (10–90 × 10–90) clipping `paint`, which fills its bounds: how a shape filled with a
/// gradient is written (`W n`, then `sh`).
fn clipped_diamond(paint: &str) -> Document {
    let shading = "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [10 0 90 0] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>";
    let resources = format!("/Shading << /Sh0 {shading} >> /Pattern << /P0 << /PatternType 2 /Shading {shading} >> >>");
    open(&one_page(&format!("q 10 50 m 50 90 l 90 50 l 50 10 l h W n {paint} Q"), &resources, &[]))
}

#[test]
fn a_shape_filled_with_a_shading_becomes_one_path_with_the_gradient() {
    let d = clipped_diamond("/Sh0 sh");
    assert!(!all(&d).iter().any(|n| matches!(n.kind, NodeKind::Group { .. })), "no group");
    let objs = objects(&d);
    let [n] = objs.as_slice() else { panic!("one object: {objs:#?}") };
    let p = n.path_data().unwrap();
    assert!(p.is_closed() && p.anchor_count() == 4, "the diamond: {p:?}");
    assert!(near(n.geometric_bounds().unwrap(), Rect::new(10.0, 10.0, 90.0, 90.0)), "{:?}", n.geometric_bounds());
    let Some(Paint::Gradient(g)) = n.appearance.fill().map(|f| &f.paint) else { panic!("a gradient fill: {:?}", n.appearance.items) };
    assert_eq!(g.gradient.kind, GradientKind::Linear);
    assert_eq!(g.gradient.stops.first().map(|s| s.color.to_rgb()), Some([1.0, 0.0, 0.0]));
    assert_eq!(g.gradient.stops.last().map(|s| s.color.to_rgb()), Some([0.0, 0.0, 1.0]));
    let geom = g.geom.as_ref().unwrap();
    assert!((geom.start.x - 10.0).abs() < 0.01 && (geom.end.x - 90.0).abs() < 0.01, "placed as before: {geom:?}");
    assert!(n.appearance.stroke().is_none());
    // A gradient that doesn't cover the clip, and a flat colour, stay clipped.
    for paint in ["/Pattern cs /P0 scn 40 40 20 20 re f", "1 0 0 rg 0 0 100 100 re f"] {
        let d = clipped_diamond(paint);
        assert!(all(&d).iter().any(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })), "{paint}");
    }
}

#[test]
fn appearance_items_are_untouched_for_plain_art() {
    // Plain art keeps importing as before: no masks, groups or clips appear.
    let d = open(&one_page("q 1 0 0 rg 10 10 30 30 re f Q 0 0 1 RG 2 w 50 50 30 30 re S", "", &[]));
    let nodes = all(&d);
    assert_eq!(nodes.iter().filter(|n| n.path_data().is_some()).count(), 2);
    assert!(nodes.iter().all(|n| n.extras().mask.is_none() && !matches!(n.kind, NodeKind::Group { .. })));
    assert!(matches!(nodes.iter().find(|n| n.appearance.stroke().is_some()).unwrap().appearance.items[0], AppearanceItem::Stroke(_)));
}

/// Type set on a path (each glyph placed and turned along a curve, as apps write it) comes back
/// as type on a path through the glyphs, not as a straight line; straight type stays point type.
#[test]
fn glyphs_turned_along_a_curve_become_type_on_a_path() {
    // "ARCHING" along an arc of radius 100 around (150, 0), each glyph where the one before
    // it ends (Helvetica's advances), clockwise from 135° (PDF y up: the arc bulges upwards).
    let mut content = String::from("BT /F1 18 Tf ");
    let mut a = 135f64.to_radians();
    for (c, adv) in "ARCHING".chars().zip([667.0, 722.0, 722.0, 722.0, 278.0, 722.0, 778.0]) {
        let (x, y) = (150.0 + 100.0 * a.cos(), 100.0 * a.sin());
        // The baseline's direction is the tangent, clockwise along the arc.
        let (dx, dy) = (a.sin(), -a.cos());
        content.push_str(&format!("{dx:.5} {dy:.5} {:.5} {dx:.5} {x:.3} {y:.3} Tm ({c}) Tj ", -dy));
        a -= adv / 1000.0 * 18.0 / 100.0;
    }
    content.push_str("ET");
    let t = texts(&open(&one_page(&content, HELVETICA, &[])));
    assert_eq!(t.len(), 1, "one text object: {t:?}");
    let text: String = t[0].runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, "ARCHING");
    let vectorcraft_doc::TextKind::OnPath { path, .. } = &t[0].kind else { panic!("type on a path: {:?}", t[0].kind) };
    let b = path.bounds().unwrap();
    assert!(b.height() > 15.0, "the path curves: {b:?}");
    // Every glyph fits on the path, the last one too.
    assert_eq!(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t[0]).glyphs.len(), 7);
    // Straight type with a slight rotation stays one point type object.
    let straight = texts(&open(&one_page("BT /F1 18 Tf 0.98481 0.17365 -0.17365 0.98481 20 40 Tm (Straight) Tj ET", HELVETICA, &[])));
    assert_eq!(straight.len(), 1);
    assert!(matches!(straight[0].kind, vectorcraft_doc::TextKind::Point), "{:?}", straight[0].kind);
}

/// The leaves (objects that aren't groups or clipping paths) of a document.
fn objects(d: &Document) -> Vec<Node> {
    all(d).into_iter().filter(|n| !n.is_container() && !matches!(n.kind, NodeKind::Path { clipping: true, .. })).collect()
}

#[test]
fn an_object_written_as_a_fill_and_a_closed_stroke_stays_one_path_with_a_live_stroke() {
    // The fill of the outline left open, then the stroke of it closed, each in its own
    // transform. Two subpaths: one ending on its start, one not.
    let outline = "0 0 m -20 1 l -30 -12 l -30 30 l 0 0 l 5 5 m 10 5 l 10 10 l";
    let content = format!(
        "0.2 0.2 0.2 rg q 1 0 0 1 50 50 cm {outline} f Q 1 1 1 RG 1.5 w 1 J 1 j 4 M [3 2] 1 d q 1 0 0 1 50 50 cm {} S Q",
        outline.replace(" 5 5 m", " h 5 5 m") + " h"
    );
    let d = open(&one_page(&content, "", &[]));
    let objs = objects(&d);
    assert_eq!(objs.len(), 1, "one object, not a fill and a separate stroke: {objs:#?}");
    let n = &objs[0];
    assert_eq!(fill_color(n), Some(Color::rgb(0.2, 0.2, 0.2)));
    let st = n.appearance.stroke().expect("a live stroke");
    assert_eq!(st.paint.color(), Some(Color::WHITE));
    assert!((st.width - 1.5).abs() < 1e-6);
    assert_eq!((st.cap, st.join), (LineCap::Round, LineJoin::Round));
    assert!((st.miter_limit - 4.0).abs() < 1e-6);
    let dash = st.dash.as_ref().expect("dashed");
    assert_eq!((dash.pattern.as_slice(), dash.offset), (&[3.0, 2.0][..], 1.0));
    // The stroke's outline: both subpaths closed, as they were stroked.
    let p = n.path_data().unwrap();
    assert_eq!(p.subpaths.len(), 2);
    assert!(p.is_closed());
}

#[test]
fn a_fill_in_a_translucent_group_doesnt_take_the_stroke_drawn_after_it() {
    // A fill at 75 % under an opaque stroke of the same outline: the fill in a transparency group
    // drawn at 75 %, the stroke after the group.
    let x = first_extra(1);
    let outline = "10 10 m 60 10 l 60 60 l 10 60 l h";
    let form = stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >>", &format!("1 0 0 rg {outline} f"));
    let content = format!("q /G gs /X0 Do Q 0 0 1 RG 2 w {outline} S");
    let d = open(&one_page(&content, &format!("/ExtGState << /G << /ca 0.75 /CA 0.75 >> >> /XObject << /X0 {x} 0 R >>"), &[&form]));
    let objs = objects(&d);
    let stroke = objs.iter().find(|n| n.appearance.stroke().is_some()).expect("a stroke");
    let alpha = all(&d)
        .iter()
        .filter(|n| n.appearance.stroke().is_some() || n.children().is_some_and(|c| c.iter().any(|k| k.id == stroke.id)))
        .map(|n| n.opacity)
        .product::<f32>();
    assert_eq!(alpha, 1.0, "the stroke stays opaque: {objs:#?}");
    let fill = objs.iter().find(|n| n.appearance.fill().is_some()).expect("a fill");
    assert!((fill.opacity * fill.appearance.fill().unwrap().opacity - 0.75).abs() < 1e-3, "{fill:#?}");
}

#[test]
fn a_fill_in_a_soft_masked_group_doesnt_take_the_stroke_drawn_after_it() {
    // ISO 32000-1 §11.6.5.2: the soft mask in the graphics state applies to the group drawn under
    // it, not to the stroke after `Q`.
    let (m, x) = (first_extra(1), first_extra(1) + 1);
    let outline = "10 10 m 60 10 l 60 60 l 10 60 l h";
    let mask = stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceRGB >>", "1 1 1 rg 0 0 35 100 re f");
    let form = stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency >>", &format!("1 0 0 rg {outline} f"));
    let content = format!("q /GS gs /X0 Do Q 0 0 1 RG 2 w {outline} S");
    let resources = format!("/ExtGState << /GS << /SMask << /Type /Mask /S /Luminosity /G {m} 0 R >> >> >> /XObject << /X0 {x} 0 R >>");
    let d = open(&one_page(&content, &resources, &[&mask, &form]));
    let nodes = all(&d);
    let stroke = nodes.iter().find(|n| n.appearance.stroke().is_some()).expect("a stroke");
    assert!(stroke.appearance.fill().is_none() && stroke.extras().mask.is_none(), "the stroke is its own unmasked object: {stroke:#?}");
    let masked = nodes.iter().filter(|n| n.extras().mask.is_some()).collect::<Vec<_>>();
    assert!(!masked.is_empty() && masked.iter().all(|n| n.appearance.stroke().is_none()), "{masked:#?}");
}

#[test]
fn different_outlines_filled_and_stroked_stay_apart() {
    let d = open(&one_page("1 0 0 rg 10 10 m 40 10 l 40 40 l f 0 0 1 RG 10 10 m 40 10 l 40 40 l 10 40 l h S 0 g 50 50 m 60 60 l S", "", &[]));
    let objs = objects(&d);
    assert_eq!(objs.len(), 3);
    assert!(objs[0].appearance.stroke().is_none() && objs[1].appearance.fill().is_none());
}

#[test]
fn a_stroke_under_a_stretched_transform_stays_a_stroke() {
    // Stretched twice as wide: the stroke keeps its average width (√2 × 2 pt) rather than
    // becoming a filled outline.
    let d = open(&one_page("0 0 1 RG 2 w q 2 0 0 1 0 0 cm 10 10 m 40 10 l 40 40 l S Q", "", &[]));
    let objs = objects(&d);
    assert_eq!(objs.len(), 1);
    assert!(objs[0].appearance.fill().is_none());
    let st = objs[0].appearance.stroke().expect("a stroke");
    assert!((st.width - 2.0 * 2f64.sqrt()).abs() < 1e-6, "{}", st.width);
    let b = objs[0].geometric_bounds().unwrap();
    assert!((b.x0 - 20.0).abs() < 1e-6 && (b.x1 - 80.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn art_past_the_page_clip_keeps_the_clip() {
    // A page-sized clip, then a backdrop far larger than the page and a square on it: the square
    // needs no clip, the backdrop keeps it, so the art's bounds stay on the page.
    let r = open_with(&one_page("0 0 100 100 re W n 0 g -500 -500 1100 1100 re f 1 0 0 rg 10 10 20 20 re f", "", &[]), |_| {});
    let d = &r.document;
    let art = d.art_bounds().expect("art");
    assert!(art.x0 >= -0.01 && art.y0 >= -0.01 && art.x1 <= 100.01 && art.y1 <= 100.01, "the art stays on the page: {art:?}");
    let clips: Vec<Node> = all(d).into_iter().filter(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })).collect();
    let [c] = clips.as_slice() else { panic!("one clip group, the backdrop's: {clips:?}") };
    assert_eq!(c.children().map(Vec::len), Some(2), "the clip path and the backdrop");
    assert!(r.warnings.iter().any(|w| w == crate::import::PAST_PAGE_NOTE), "{:?}", r.warnings);
    // Art inside the page gets no clip and no warning.
    let r = open_with(&one_page("0 0 100 100 re W n 1 0 0 rg 10 10 20 20 re f", "", &[]), |_| {});
    assert!(!all(&r.document).iter().any(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })));
    assert!(!r.warnings.iter().any(|w| w == crate::import::PAST_PAGE_NOTE), "{:?}", r.warnings);
}

/// Glyphs closed up by up to half an em (the proportional widths of CJK punctuation) still
/// continue their line.
#[test]
fn glyphs_closed_up_by_half_an_em_stay_one_line() {
    let t = texts(&open(&one_page("BT /F1 20 Tf 10 40 Td [(T) 400 (i) 450 (g) 0 (h) 0 (t)] TJ ET", HELVETICA, &[])));
    assert_eq!(t.len(), 1, "one line: {:?}", t.iter().map(|t| t.plain_text()).collect::<Vec<_>>());
    assert_eq!(t[0].plain_text(), "Tight");
}

/// #1542: a clip only counts as covering the page when it does; a diamond whose bounding box is
/// the page still clips, a page rectangle or a turned square around it doesn't make a group.
#[test]
fn clips_with_the_page_as_bounding_box_still_clip() {
    let clipped = |clip: &str| {
        let d = open(&one_page(&format!("q {clip} W n 1 0.5 0 rg 0 0 100 100 re f Q"), "", &[]));
        all(&d).iter().any(|n| matches!(n.kind, NodeKind::Group { clip: true, .. }))
    };
    assert!(clipped("50 0 m 100 50 l 50 100 l 0 50 l h"), "diamond");
    assert!(!clipped("0 0 100 100 re"), "the page");
    assert!(!clipped("50 -60 m 160 50 l 50 160 l -60 50 l h"), "a turned square around the page");
}

/// Issue #2674: `0 w` is the thinnest line the device can draw (ISO 32000-1, 8.4.3.2), not no
/// line: it imports as a visible hairline and survives a PDF round trip.
#[test]
fn zero_width_lines_import_as_hairlines() {
    let d = open(&one_page("0 0 0 RG 0 w 10 50 m 90 50 l S 0.9 g 0 w 20 60 30 20 re b 1 w 10 20 m 90 20 l S", "", &[]));
    let widths = |d: &Document| -> Vec<f64> { all(d).iter().filter_map(|n| n.appearance.stroke().map(|s| s.width)).collect() };
    let w = widths(&d);
    assert_eq!(w.len(), 3, "{w:?}");
    assert!(w.iter().all(|w| *w > 0.0 && *w <= 1.0), "zero width strokes become hairlines: {w:?}");
    let back = widths(&roundtrip(&d));
    assert_eq!(back.len(), 3, "hairlines survive the PDF round trip: {back:?}");
}
