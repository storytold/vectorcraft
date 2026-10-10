//! Import fidelity: soft masks → opacity masks, isolated and knockout groups, tiling patterns →
//! pattern swatches, text → point type, mesh shadings → gradient meshes, gradient stop opacity
//! and shading Extend flags; strokes stay live strokes.

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
    let n = all(d).into_iter().find(|n| n.mask.is_some()).expect("a masked object");
    let m = n.mask.as_deref().unwrap().clone();
    (n, m)
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
        n.mask = Some(Box::new(OpacityMask { invert, ..OpacityMask::new(art, clip) }));
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
    assert!(g.isolate && g.mask.is_none());
    assert!((g.opacity - 0.5).abs() < 0.01, "{}", g.opacity);
    assert_eq!(g.children().unwrap()[0].blend, BlendMode::Multiply);
    // Not isolated: its 50% (a constant alpha mask in the file) is opacity again, here on the
    // only object inside, which blends with the backdrop as before.
    let back = roundtrip(&half_group(false));
    let nodes = all(&back);
    assert!(nodes.iter().all(|n| !n.isolate && n.mask.is_none()));
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

// ---------- meshes, stops and extend ----------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join("") + ">"
}

#[test]
fn a_coons_patch_mesh_becomes_a_gradient_mesh() {
    // One patch over (10, 10)–(90, 90): corner colours red (0, 0), green (0, 1), blue (1, 1),
    // white (1, 0) in u/v.
    let pts: [(u8, u8); 12] =
        [(10, 10), (10, 37), (10, 63), (10, 90), (37, 90), (63, 90), (90, 90), (90, 63), (90, 37), (90, 10), (63, 10), (37, 10)];
    let mut data = vec![0u8];
    for (x, y) in pts {
        data.extend([x, y]);
    }
    data.extend([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
    let s = first_extra(1);
    let sh = stream(
        "/ShadingType 6 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 255 0 255 0 1 0 1 0 1] /Filter /ASCIIHexDecode",
        &hex(&data),
    );
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

#[test]
fn gradient_stop_opacity_round_trips() {
    let mut d = Document::new(100.0, 100.0);
    let mut stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    stops[1].opacity = 0.2;
    let g = GradientPaint::new(Gradient { kind: GradientKind::Linear, stops });
    let n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)),
        Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 0.0),
    );
    add(&mut d, n);
    let back = roundtrip(&d);
    let nodes = all(&back);
    assert!(nodes.iter().all(|n| n.mask.is_none()), "the opacity is in the stops, not a mask");
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

#[test]
fn appearance_items_are_untouched_for_plain_art() {
    // Plain art keeps importing as before: no masks, groups or clips appear.
    let d = open(&one_page("q 1 0 0 rg 10 10 30 30 re f Q 0 0 1 RG 2 w 50 50 30 30 re S", "", &[]));
    let nodes = all(&d);
    assert_eq!(nodes.iter().filter(|n| n.path_data().is_some()).count(), 2);
    assert!(nodes.iter().all(|n| n.mask.is_none() && !matches!(n.kind, NodeKind::Group { .. })));
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
    // As Illustrator writes an object: the fill of the outline left open, then the stroke of it
    // closed, each in its own transform. Two subpaths: one ending on its start, one not.
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
