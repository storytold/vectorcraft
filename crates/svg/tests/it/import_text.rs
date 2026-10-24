//! SVG `<text>` import fidelity: placement in the tree, type on a path, per-character positions,
//! paints and the style cascade.

use vectorcraft_color::{GradientKind, Paint};
use vectorcraft_doc::{CharStyle, Document, Justify, Node, NodeKind, TextKind, TextObject};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_svg::{import, import_with_report};
use vectorcraft_text::{FontDb, layout};

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="400" height="300">{body}</svg>"#)
}

fn open(body: &str) -> Document {
    import(&svg(body)).unwrap_or_else(|e| panic!("{e}"))
}

/// All non-layer nodes in paint order.
fn art(d: &Document) -> Vec<&Node> {
    let mut v = Vec::new();
    d.walk(|n| {
        if !n.is_layer() {
            v.push(n)
        }
    });
    v
}

fn texts(d: &Document) -> Vec<&TextObject> {
    art(d)
        .into_iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some(&**t),
            _ => None,
        })
        .collect()
}

fn only_text(d: &Document) -> &TextObject {
    let t = texts(d);
    assert_eq!(t.len(), 1, "{:?}", art(d).iter().map(|n| &n.kind).collect::<Vec<_>>());
    t[0]
}

/// Document-space pen position of each glyph.
fn glyph_origins(t: &TextObject) -> Vec<Point> {
    layout(FontDb::global(), t).glyphs.iter().map(|g| t.xf * g.origin).collect()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() < tol
}

#[test]
fn text_keeps_its_z_order_between_shapes() {
    let d = open(r#"<rect width="10" height="10"/><text x="5" y="20">Hi</text><rect y="30" width="10" height="10"/>"#);
    let kinds: Vec<&str> = art(&d)
        .iter()
        .map(|n| match n.kind {
            NodeKind::Text(_) => "text",
            NodeKind::Path { .. } => "path",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["path", "text", "path"]);
    let t = only_text(&d);
    assert_eq!(t.plain_text(), "Hi");
    assert!(t.xf.translation().to_point().distance(Point::new(5.0, 20.0)) < 1e-9);
}

#[test]
fn text_keeps_its_parent_group_clip_and_mask() {
    let d = open(
        r##"<defs><clipPath id="c"><rect width="50" height="50"/></clipPath>
        <mask id="m"><rect width="100" height="100" fill="white"/></mask></defs>
        <g id="outer" transform="translate(10 20)"><rect width="5" height="5"/>
          <g clip-path="url(#c)"><text id="clipped" y="30">In clip</text></g>
          <text id="own" y="60" clip-path="url(#c)" opacity="0.5">Own clip</text>
          <text id="masked" y="90" mask="url(#m)">Masked</text>
        </g><rect width="5" height="5"/>"##,
    );
    // Two top-level children (a group and a rect): no layers, everything stays on Layer 1.
    assert_eq!(d.layers.len(), 1);
    let top = d.layers[0].children().unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].name.as_deref(), Some("outer"));
    let inner = top[0].children().unwrap();
    assert_eq!(inner.len(), 4, "rect, clip group, clip group, masked text");
    // `<g clip-path>` around the text → a clip group holding the clip path and the text.
    let NodeKind::Group { children, clip: true } = &inner[1].kind else { panic!("{:?}", inner[1].kind) };
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    let NodeKind::Text(t) = &children[1].kind else { panic!() };
    assert_eq!(children[1].name.as_deref(), Some("clipped"));
    assert!(t.xf.translation().to_point().distance(Point::new(10.0, 50.0)) < 1e-9, "{:?}", t.xf);
    // The text's own clip-path and opacity.
    let NodeKind::Group { children, clip: true } = &inner[2].kind else { panic!("{:?}", inner[2].kind) };
    assert!((inner[2].opacity - 0.5).abs() < 1e-6);
    assert!(matches!(children[1].kind, NodeKind::Text(_)) && children[1].name.as_deref() == Some("own"));
    // The text's own mask.
    assert!(matches!(inner[3].kind, NodeKind::Text(_)));
    assert!(inner[3].extras().mask.is_some());
    assert_eq!(inner[3].name.as_deref(), Some("masked"));
}

#[test]
fn top_level_text_joins_the_layer_below_it() {
    let d = open(r#"<g id="Back"><rect width="5" height="5"/></g><text y="20">Note</text><g id="Front"><rect width="5" height="5"/></g>"#);
    assert_eq!(d.layers.iter().map(|l| l.name.as_deref().unwrap()).collect::<Vec<_>>(), ["Back", "Front"]);
    let back = d.layers[0].children().unwrap();
    assert_eq!(back.len(), 2);
    assert!(matches!(back[1].kind, NodeKind::Text(_)));
    // Text before the first layer goes to its bottom.
    let d = open(r#"<text y="20">First</text><g id="Only"><rect width="5" height="5"/></g>"#);
    let only = d.layers[0].children().unwrap();
    assert!(matches!(only[0].kind, NodeKind::Text(_)) && matches!(only[1].kind, NodeKind::Path { .. }));
}

#[test]
fn hidden_text_is_skipped_and_use_instances_are_kept() {
    let d = open(
        r##"<text y="10" display="none">gone</text><g visibility="hidden"><text y="20">hidden</text></g>
        <defs><text id="t" y="30">Used</text></defs><use href="#t" x="10"/><use xlink:href="#t" x="50"/>"##,
    );
    // An undisplayed text comes back as a hidden object (since SVG import keeps hidden objects).
    let hidden: Vec<&Node> = art(&d).into_iter().filter(|n| !n.visible).collect();
    assert!(matches!(hidden.as_slice(), [n] if matches!(&n.kind, NodeKind::Text(t) if t.plain_text() == "gone")), "{hidden:?}");
    let t: Vec<&TextObject> = art(&d)
        .into_iter()
        .filter(|n| n.visible)
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some(&**t),
            _ => None,
        })
        .collect();
    assert_eq!(t.len(), 2);
    assert!(t.iter().all(|t| t.plain_text() == "Used"));
    assert!(t[0].xf.translation().to_point().distance(Point::new(10.0, 30.0)) < 1e-9);
    assert!(t[1].xf.translation().to_point().distance(Point::new(50.0, 30.0)) < 1e-9);
}

#[test]
fn text_path_becomes_type_on_a_path() {
    let d = open(
        r##"<path id="p" d="M10 100 L210 100" fill="none" stroke="black"/>
        <text font-size="10"><textPath href="#p" startOffset="25%">Curve</textPath></text>
        <text font-size="10"><textPath xlink:href="#p" startOffset="20">Twenty</textPath></text>
        <text font-size="10" text-anchor="middle"><textPath href="#p" startOffset="50%">Mid</textPath></text>"##,
    );
    let t = texts(&d);
    assert_eq!(t.len(), 3);
    let TextKind::OnPath { path, start, .. } = &t[0].kind else { panic!("{:?}", t[0].kind) };
    assert!(close(*start, 0.25, 1e-9));
    let b = t[0].xf.transform_rect_bbox(path.bounds().unwrap());
    assert!(close(b.x0, 10.0, 1e-6) && close(b.x1, 210.0, 1e-6) && close(b.y0, 100.0, 1e-6), "{b:?}");
    assert_eq!(t[0].plain_text(), "Curve");
    let TextKind::OnPath { start, .. } = &t[1].kind else { panic!() };
    assert!(close(*start, 0.1, 1e-6), "{start}");
    // Anchored in the middle: the text starts half its width before the offset.
    let TextKind::OnPath { start, .. } = &t[2].kind else { panic!() };
    let first = glyph_origins(t[2])[0];
    let width: f64 = layout(FontDb::global(), t[2]).glyphs.iter().map(|g| g.advance).sum();
    assert!(*start < 0.5 && close(first.x + width / 2.0, 110.0, 0.5), "{start} {first:?} {width}");
    assert_eq!(t[2].para.justify, Justify::Left);
}

#[test]
fn per_character_positions_rotation_and_shifts() {
    let d = open(r#"<text x="10 30 60" y="50" rotate="15 -30" dy="0 -4" font-size="12">abcd</text>"#);
    let t = only_text(&d);
    assert_eq!(t.plain_text(), "abcd");
    let o = glyph_origins(t);
    assert!(close(o[0].x, 10.0, 0.02) && close(o[1].x, 30.0, 0.02) && close(o[2].x, 60.0, 0.02), "{o:?}");
    // `d` follows `c` at its natural advance.
    assert!(o[3].x > 60.0 && o[3].x < 70.0, "{o:?}");
    let chars: Vec<CharStyle> = t.runs.iter().flat_map(|r| r.text.chars().map(|_| r.style.clone())).collect();
    let rot: Vec<f64> = chars.iter().map(|s| s.rotation).collect();
    assert_eq!(rot, [-15.0, 30.0, 30.0, 30.0], "the last rotate value repeats");
    let shift: Vec<f64> = chars.iter().map(|s| s.baseline_shift).collect();
    assert_eq!(shift, [0.0, 4.0, 4.0, 4.0], "dy accumulates as baseline shift");
}

#[test]
fn per_character_positions_with_middle_anchor() {
    // SVG anchors each absolutely placed chunk on its own: `a` is centred on 50, `b` on 100.
    let d = open(r#"<text x="50 100" y="40" text-anchor="middle" font-size="20">ab</text>"#);
    let t = only_text(&d);
    let lay = layout(FontDb::global(), t);
    // Natural advance: the kerning that moves `b` onto its x is added after `a`.
    let natural = |i: usize| lay.glyphs[i].advance - t.runs[i].style.kerning.unwrap_or(0.0) / 1000.0 * t.runs[i].style.size;
    let mid = |i: usize| (t.xf * lay.glyphs[i].origin).x + natural(i) / 2.0;
    assert_eq!(t.runs.len(), 2);
    assert!(close(mid(0), 50.0, 0.05) && close(mid(1), 100.0, 0.05), "{} {}", mid(0), mid(1));
}

#[test]
fn tspan_lines_dx_and_baseline_shift() {
    let d = open(
        r#"<text x="10" y="20" font-size="10">One<tspan x="10" dy="20">Two</tspan><tspan x="10" dy="12">Three</tspan>
        <tspan dx="5">four</tspan><tspan baseline-shift="super">2</tspan><tspan baseline-shift="-3">x</tspan></text>"#,
    );
    let t = only_text(&d);
    assert_eq!(t.plain_text(), "One\nTwo\nThree four2x");
    let lay = layout(FontDb::global(), t);
    let base: Vec<f64> = lay.lines.iter().map(|l| (t.xf * Point::new(0.0, l.baseline)).y).collect();
    assert!(close(base[0], 20.0, 1e-6) && close(base[1], 40.0, 1e-6) && close(base[2], 52.0, 1e-6), "{base:?}");
    let run = |s: &str| t.runs.iter().find(|r| r.text.contains(s)).unwrap().style.clone();
    assert_eq!(run("Two").leading, Some(20.0));
    assert_eq!(run("Three").leading, None, "12 pt is the auto leading of 10 pt type");
    assert!(close(run("2").baseline_shift, 4.0, 1e-9) && close(run("x").baseline_shift, -3.0, 1e-9));
    // dx on "four": the space before it gets 5 pt of kerning.
    let sp = t.runs.iter().find(|r| r.text.ends_with(' ')).unwrap();
    assert!(close(sp.style.kerning.unwrap(), 500.0, 1e-6), "{:?}", t.runs);
}

#[test]
fn weights_spacing_and_decoration() {
    let d = open(
        r#"<text y="20" font-size="10" font-weight="300" letter-spacing="1" word-spacing="2">a b<tspan font-weight="600">c</tspan><tspan font-weight="bolder">d</tspan><tspan font-weight="900" font-style="italic">e</tspan><tspan font-style="italic">f</tspan><tspan text-decoration="underline" font-weight="normal">g</tspan></text>"#,
    );
    let t = only_text(&d);
    let style = |c: char| t.runs.iter().find(|r| r.text.contains(c)).unwrap().style.clone();
    assert_eq!(style('a').font_style, "Light");
    assert_eq!(style('c').font_style, "SemiBold");
    assert_eq!(style('d').font_style, "Regular", "bolder than 300 is 400");
    assert_eq!(style('e').font_style, "Black Italic");
    assert_eq!(style('f').font_style, "Light Italic");
    assert!(style('g').underline && style('g').font_style == "Regular");
    assert!(close(style('a').tracking, 100.0, 1e-9));
    assert!(close(style(' ').kerning.unwrap(), 200.0, 1e-9), "word spacing kerns spaces");
    assert_eq!(style('a').kerning, None);
}

/// `font-variation-settings` becomes the characters' Variable Font settings, inherited as CSS
/// inherits it; `normal` and `initial` set none, and `unset` and a value CSS ignores leave the
/// inherited settings.
#[test]
fn font_variation_settings_become_variable_font_settings() {
    let d = open(
        r#"<text y="20" style="font-variation-settings: 'wght' 650, &quot;wdth&quot; 87.5">a<tspan>b</tspan><tspan style="font-variation-settings: normal">c</tspan><tspan style="font-variation-settings: 'weight' 650">d</tspan><tspan style="font-variation-settings: 'wght' heavy">e</tspan><tspan style="font-variation-settings: initial">f</tspan><tspan style="font-variation-settings: unset">g</tspan></text>"#,
    );
    let t = only_text(&d);
    let axes = |c: char| t.runs.iter().find(|r| r.text.contains(c)).unwrap().style.font_axes.clone().into_iter().collect::<Vec<_>>();
    let set = vec![("wdth".to_string(), 87.5), ("wght".to_string(), 650.0)];
    for c in ['a', 'b', 'd', 'e', 'g'] {
        assert_eq!(axes(c), set, "{c}");
    }
    for c in ['c', 'f'] {
        assert!(axes(c).is_empty(), "{c}: {:?}", axes(c));
    }
}

#[test]
fn css_selectors_reach_text() {
    let d = open(
        r#"<style>
        text { font-family: Inter }
        g text { fill: #ff0000 }
        #big { font-size: 30px }
        .a.b { fill: #00ff00 }
        g > text.only-child { fill: #0000ff }
        </style>
        <text id="plain" y="10">p</text>
        <g><text id="big" y="40">g</text><text class="a b" y="80">ab</text><text class="a" y="90">a</text><text class="only-child" y="100">c</text></g>"#,
    );
    let t = texts(&d);
    let fill = |i: usize| t[i].first_style().fill.color().map(|c| c.to_hex());
    assert_eq!(t[0].first_style().font_family, "Inter");
    assert_eq!(fill(0).as_deref(), Some("#000000"));
    assert_eq!(fill(1).as_deref(), Some("#ff0000"), "descendant selector");
    assert!(close(t[1].first_style().size, 30.0, 1e-9), "#id selector");
    assert_eq!(fill(2).as_deref(), Some("#00ff00"), "multiple classes");
    assert_eq!(fill(3).as_deref(), Some("#ff0000"), "one of two classes doesn't match .a.b");
    assert_eq!(fill(4).as_deref(), Some("#0000ff"), "child combinator");
}

#[test]
fn gradient_fill_resolves_in_text_space() {
    let d = open(
        r##"<defs>
          <linearGradient id="user" gradientUnits="userSpaceOnUse" x1="20" y1="0" x2="120" y2="0"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>
          <linearGradient id="box"><stop offset="0" stop-color="#0f0"/><stop offset="1" stop-color="#000"/></linearGradient>
          <radialGradient id="r"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></radialGradient>
        </defs>
        <text x="20" y="50" font-size="20" fill="url(#user)" stroke="url(#r)">Gradient<tspan fill="url(#box)">Box</tspan></text>"##,
    );
    let t = only_text(&d);
    let Paint::Gradient(g) = &t.runs[0].style.fill else { panic!("{:?}", t.runs[0].style.fill) };
    assert_eq!(g.gradient.kind, GradientKind::Linear);
    assert_eq!(g.gradient.stops[0].color.to_hex(), "#ff0000");
    // Text space has its origin at the text's x/y.
    let geom = g.geom.unwrap();
    assert!(geom.start.distance(Point::new(0.0, -50.0)) < 1e-6 && geom.end.distance(Point::new(100.0, -50.0)) < 1e-6, "{geom:?}");
    let Paint::Gradient(s) = &t.runs[0].style.stroke else { panic!("{:?}", t.runs[0].style.stroke) };
    assert_eq!(s.gradient.kind, GradientKind::Radial);
    // objectBoundingBox: the gradient spans the laid-out text.
    let Paint::Gradient(b) = &t.runs[1].style.fill else { panic!("{:?}", t.runs[1].style.fill) };
    let bounds = layout(FontDb::global(), t).bounds;
    let geom = b.geom.unwrap();
    assert!(close(geom.start.x, bounds.x0, 1e-3) && close(geom.end.x, bounds.x1, 1e-3), "{geom:?} {bounds:?}");
    assert_eq!(t.runs[1].text, "Box");
}

#[test]
fn pattern_fill_on_text_and_shapes() {
    let d = open(
        r##"<defs><pattern id="dots" width="10" height="10" patternUnits="userSpaceOnUse" x="2"><circle cx="5" cy="5" r="3" fill="red"/></pattern></defs>
        <rect width="100" height="100" fill="url(#dots)"/><text x="10" y="50" fill="url(#dots)">P</text>"##,
    );
    assert_eq!(d.patterns.len(), 1);
    let p = &d.patterns[0];
    assert_eq!(p.name, "dots");
    assert_eq!(p.tile, Rect::new(0.0, 0.0, 10.0, 10.0));
    let NodeKind::Group { clip: true, children } = &p.art[0].kind else { panic!("content is clipped to the tile") };
    assert_eq!(children.len(), 2);
    let a = art(&d);
    let Paint::Pattern { pattern, xf } = a[0].appearance.fill_paint() else { panic!() };
    assert_eq!(pattern, "dots");
    assert!(xf.translation().to_point().distance(Point::new(2.0, 0.0)) < 1e-9);
    let t = only_text(&d);
    let Paint::Pattern { pattern, xf } = &t.runs[0].style.fill else { panic!("{:?}", t.runs[0].style.fill) };
    assert_eq!(pattern, "dots");
    // In text space (origin at 10, 50) the tiles stay where they are in the document.
    assert!(xf.translation().to_point().distance(Point::new(-8.0, -50.0)) < 1e-9, "{xf:?}");
}

#[test]
fn vertical_writing_mode_runs_down_a_path() {
    let d = open(r#"<text x="100" y="20" writing-mode="tb" font-size="10">Down</text>"#);
    let t = only_text(&d);
    let TextKind::OnPath { path, start, .. } = &t.kind else { panic!("{:?}", t.kind) };
    assert_eq!(*start, 0.0);
    let b = path.bounds().unwrap();
    assert!(close(b.x0, b.x1, 1e-9) && close(b.y0, 20.0, 1e-9) && b.height() > 20.0, "{b:?}");
    let o = glyph_origins(t);
    assert!(o.windows(2).all(|w| w[1].y > w[0].y), "glyphs step down: {o:?}");
    // The em box is centred on the column.
    let lay = layout(FontDb::global(), t);
    assert!(lay.bounds.x0 < 100.0 && lay.bounds.x1 > 100.0, "{:?}", lay.bounds);
}

#[test]
fn unusable_text_path_falls_back_to_point_type_with_a_warning() {
    let (d, w) = import_with_report(&svg(r##"<text x="5" y="10"><textPath href="#missing">Lost</textPath></text>"##)).unwrap();
    let t = only_text(&d);
    assert!(matches!(t.kind, TextKind::Point));
    assert_eq!(t.plain_text(), "Lost");
    assert!(w.iter().any(|w| w.contains("textPath")), "{w:?}");
}

#[test]
fn white_space_handling() {
    let d = open("<text y=\"10\">  Hello\n   big\tworld  </text><text y=\"30\" xml:space=\"preserve\"> a  b </text>");
    let t = texts(&d);
    assert_eq!(t[0].plain_text(), "Hello big world");
    assert_eq!(t[1].plain_text(), " a  b ");
}

#[test]
fn placeholder_ids_never_clash_with_the_files() {
    let d = open(r#"<rect id="vectorcraft-text-0" width="5" height="5"/><text y="20">Safe</text>"#);
    assert_eq!(only_text(&d).plain_text(), "Safe");
    assert_eq!(art(&d)[0].name.as_deref(), Some("vectorcraft-text-0"));
}

#[test]
fn linked_text_keeps_its_url_and_stroke_style() {
    let d = open(
        r#"<rect width="5" height="5"/><a href="https://example.com/t"><text y="20" stroke="red" stroke-width="0.1em" stroke-linejoin="round" stroke-dasharray="2 1">Go</text></a>"#,
    );
    let n = art(&d).into_iter().find(|n| matches!(n.kind, NodeKind::Text(_))).unwrap();
    assert_eq!(n.url(), Some("https://example.com/t"));
    let st = only_text(&d).first_style();
    assert!(close(st.stroke_width, 1.2, 1e-9), "{}", st.stroke_width);
    assert_eq!(st.stroke_join, vectorcraft_doc::LineJoin::Round);
    assert_eq!(st.stroke_dash.as_ref().map(|d| d.pattern.clone()), Some(vec![2.0, 1.0]));
}

#[test]
fn plain_texts_among_siblings_stay_texts() {
    let d = open(
        r#"<g><rect width="5" height="5"/><text y="20">A</text><g><text y="30">B</text></g><rect y="40" width="5" height="5"/></g><g><text y="50">C</text></g>"#,
    );
    let kinds: Vec<&str> = art(&d)
        .iter()
        .map(|n| match n.kind {
            NodeKind::Text(_) => "text",
            NodeKind::Path { .. } => "path",
            NodeKind::Group { .. } => "group",
            _ => "other",
        })
        .collect();
    assert_eq!(texts(&d).iter().map(|t| t.plain_text()).collect::<Vec<_>>(), ["A", "B", "C"], "{kinds:?}");
    assert_eq!(kinds.iter().filter(|k| **k == "path").count(), 2, "{kinds:?}");
}

#[test]
fn far_off_positions_stay_finite() {
    let d = open(r#"<text x="1e308 -1e308 1e308" y="1e308" dx="1e308 1e308" dy="1e308 1e308" rotate="inf 5" text-anchor="middle">abc</text>"#);
    let t = only_text(&d);
    assert!(t.xf.as_coeffs().iter().all(|v| v.is_finite()), "{:?}", t.xf);
    for r in &t.runs {
        assert!(r.style.baseline_shift.is_finite() && r.style.kerning.is_none_or(f64::is_finite) && r.style.rotation.is_finite(), "{r:?}");
    }
}

/// Drawn bounds of the laid-out text in the document.
fn drawn(t: &TextObject) -> Rect {
    t.xf.transform_rect_bbox(layout(FontDb::global(), t).bounds)
}

/// The linear part of a transform.
fn linear(t: &TextObject) -> [f64; 4] {
    let [a, b, c, d, ..] = t.xf.as_coeffs();
    [a, b, c, d]
}

fn close4(a: [f64; 4], b: [f64; 4]) -> bool {
    // usvg's transforms are single precision.
    a.iter().zip(b).all(|(a, b)| close(*a, b, 1e-6))
}

#[test]
fn a_scale_above_text_becomes_its_size() {
    // #396: a group's or the text's own scale is the size the type shows and draws at.
    let d = open(
        r#"<text x="50" y="100" font-size="7">HHHH Hamburg</text>
        <g transform="scale(3)"><text x="16" y="60" font-size="7">HHHH Hamburg</text></g>
        <text x="50" y="400" font-size="7" transform="scale(2)">HHHH Hamburg</text>"#,
    );
    let t = texts(&d);
    let sizes: Vec<f64> = t.iter().map(|t| t.first_style().size).collect();
    assert!(close(sizes[0], 7.0, 1e-9) && close(sizes[1], 21.0, 1e-9) && close(sizes[2], 14.0, 1e-9), "{sizes:?}");
    for t in &t {
        assert!(close4(linear(t), [1.0, 0.0, 0.0, 1.0]), "{:?}", t.xf);
    }
    assert!(t[1].xf.translation().to_point().distance(Point::new(48.0, 180.0)) < 1e-9, "{:?}", t[1].xf);
    assert!(t[2].xf.translation().to_point().distance(Point::new(100.0, 800.0)) < 1e-9, "{:?}", t[2].xf);
    // Drawn as large as their sizes say.
    let h: Vec<f64> = t.iter().map(|t| drawn(t).height() / t.first_style().size).collect();
    assert!(close(h[1], h[0], 1e-6) && close(h[2], h[0], 1e-6), "{h:?}");
}

#[test]
fn a_viewbox_scale_becomes_the_size() {
    let d = import(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="306" height="396" viewBox="0 0 612 792"><text x="50" y="100" font-size="7">HH</text></svg>"#,
    )
    .unwrap();
    let t = only_text(&d);
    assert!(close(t.first_style().size, 3.5, 1e-9), "{}", t.first_style().size);
    assert!(close4(linear(t), [1.0, 0.0, 0.0, 1.0]), "{:?}", t.xf);
    assert!(t.xf.translation().to_point().distance(Point::new(25.0, 50.0)) < 1e-9, "{:?}", t.xf);
}

#[test]
fn a_rotated_scale_scales_every_length_and_keeps_the_rotation() {
    // The same text written at twice the size: the scale moves into size, leading, baseline
    // shift and the character strokes; spacing in ems stays.
    let scaled = open(
        r#"<text transform="translate(100 50) rotate(30) scale(2)" x="5" y="10" font-size="7" letter-spacing="1" word-spacing="2" stroke="red" stroke-width="0.5" stroke-dasharray="1 2">Ab <tspan baseline-shift="3">c</tspan><tspan x="5" dy="12">Two</tspan></text>"#,
    );
    let written = open(
        r#"<text transform="translate(100 50) rotate(30)" x="10" y="20" font-size="14" letter-spacing="2" word-spacing="4" stroke="red" stroke-width="1" stroke-dasharray="2 4">Ab <tspan baseline-shift="6">c</tspan><tspan x="10" dy="24">Two</tspan></text>"#,
    );
    let (s, w) = (only_text(&scaled), only_text(&written));
    assert_eq!(s.runs, w.runs);
    assert!(s.runs.iter().any(|r| r.style.leading == Some(24.0) && r.text.contains("Two")), "{:?}", s.runs);
    assert!(close(s.xf.determinant(), 1.0, 1e-6), "{:?}", s.xf);
    let r = 30f64.to_radians();
    assert!(close4(linear(s), [r.cos(), r.sin(), -r.sin(), r.cos()]), "{:?}", s.xf);
    for (a, b) in glyph_origins(s).iter().zip(glyph_origins(w)) {
        assert!(a.distance(b) < 1e-6, "{a:?} {b:?}");
    }
}

#[test]
fn a_stretch_or_skew_stays_the_transform() {
    // The scale across the baseline is the size; a stretch along it or a skew stays.
    let d = open(
        r#"<text x="10" y="20" font-size="7" transform="scale(3 1)">HH</text>
        <text x="10" y="20" font-size="7" transform="scale(1 3)">HH</text>
        <text x="10" y="20" font-size="7" transform="skewX(20)">HH</text>"#,
    );
    let t = texts(&d);
    let sizes: Vec<f64> = t.iter().map(|t| t.first_style().size).collect();
    assert!(close(sizes[0], 7.0, 1e-9) && close(sizes[1], 21.0, 1e-9) && close(sizes[2], 7.0, 1e-9), "{sizes:?}");
    assert!(close4(linear(t[0]), [3.0, 0.0, 0.0, 1.0]), "{:?}", t[0].xf);
    assert!(close4(linear(t[1]), [1.0 / 3.0, 0.0, 0.0, 1.0]), "{:?}", t[1].xf);
    assert!(close4(linear(t[2]), [1.0, 0.0, 20f64.to_radians().tan(), 1.0]), "{:?}", t[2].xf);
    // `scale(1 3)` draws three times as tall as `scale(3 1)`, a third as wide.
    let (wide, tall) = (drawn(t[0]), drawn(t[1]));
    assert!(close(tall.height(), 3.0 * wide.height(), 1e-6) && close(3.0 * tall.width(), wide.width(), 1e-6), "{wide:?} {tall:?}");
    assert!(t[1].xf.translation().to_point().distance(Point::new(10.0, 60.0)) < 1e-9, "{:?}", t[1].xf);
}

#[test]
fn type_on_a_scaled_path_takes_the_scale() {
    let d = open(
        r##"<g transform="scale(2)"><path id="p" d="M10 100 L110 100" fill="none"/>
        <text font-size="10"><textPath href="#p">Curve</textPath></text></g>"##,
    );
    let t = only_text(&d);
    assert!(close(t.first_style().size, 20.0, 1e-9), "{}", t.first_style().size);
    assert!(close(t.xf.determinant(), 1.0, 1e-6), "{:?}", t.xf);
    let TextKind::OnPath { path, .. } = &t.kind else { panic!("{:?}", t.kind) };
    let b = t.xf.transform_rect_bbox(path.bounds().unwrap());
    assert!(close(b.x0, 20.0, 1e-9) && close(b.x1, 220.0, 1e-9) && close(b.y0, 200.0, 1e-9), "{b:?}");
}

#[test]
fn folded_sizes_stay_in_the_character_panels_range() {
    // Past 1296 pt (or under 0.1 pt) the rest of the scale stays the transform.
    let d = open(
        r#"<g transform="scale(20)"><text y="10" font-size="100">Big</text></g>
        <g transform="scale(0.001)"><text y="10" font-size="7">Small</text></g>
        <text y="10" font-size="2000">Huge</text>"#,
    );
    let t = texts(&d);
    let size = |i: usize| t[i].first_style().size;
    assert!(close(size(0), 1296.0, 1e-9) && close(size(0) * linear(t[0])[0], 2000.0, 1e-9), "{:?}", t[0].xf);
    assert!(close(size(1), 0.1, 1e-9) && close(size(1) * linear(t[1])[0], 0.007, 1e-9), "{:?}", t[1].xf);
    assert!(close(size(2), 2000.0, 1e-9) && close4(linear(t[2]), [1.0, 0.0, 0.0, 1.0]), "unscaled type keeps its size");
}

#[test]
fn gradients_on_scaled_text_stay_in_place() {
    let d = open(
        r##"<defs><linearGradient id="user" gradientUnits="userSpaceOnUse" x1="20" y1="0" x2="120" y2="0"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
        <g transform="scale(2)"><text x="20" y="50" font-size="20" fill="url(#user)">Gradient</text></g>"##,
    );
    let t = only_text(&d);
    assert!(close(t.first_style().size, 40.0, 1e-9));
    let Paint::Gradient(g) = &t.runs[0].style.fill else { panic!("{:?}", t.runs[0].style.fill) };
    let geom = g.geom.unwrap();
    let (start, end) = (t.xf * geom.start, t.xf * geom.end);
    assert!(start.distance(Point::new(40.0, 0.0)) < 1e-6 && end.distance(Point::new(240.0, 0.0)) < 1e-6, "{start:?} {end:?}");
}

/// The start of a type-on-a-path object's path and a point a little way along it, in document
/// space.
fn path_start(t: &TextObject) -> (Point, Point) {
    use vectorcraft_geom::kurbo::{ParamCurve, PathEl};
    let TextKind::OnPath { path, .. } = &t.kind else { panic!("{:?}", t.kind) };
    let mut bp = path.to_bezpath();
    bp.apply_affine(t.xf);
    let PathEl::MoveTo(p0) = bp.elements()[0] else { panic!("{bp:?}") };
    let next = bp.segments().next().unwrap().eval(0.05);
    (p0, next)
}

#[test]
fn text_path_follows_basic_shapes_as_their_equivalent_paths() {
    let (d, w) = import_with_report(&svg(r##"<circle id="c" cx="150" cy="150" r="100"/>
        <ellipse id="e" cx="150" cy="150" rx="80" ry="40" transform="translate(10 0)"/>
        <rect id="r" x="20" y="30" width="100" height="60" rx="10"/>
        <rect id="s" x="20" y="30" width="100" height="60"/>
        <line id="l" x1="10" y1="200" x2="210" y2="200"/>
        <polyline id="pl" points="10,10 110,10 110,60"/>
        <polygon id="pg" points="10 10, 60 10, 35 50"/>
        <text font-size="10"><textPath xlink:href="#c">TEXT ON A CIRCLE</textPath></text>
        <text font-size="10"><textPath href="#e">Ellipse</textPath></text>
        <text font-size="10"><textPath href="#r">Rounded</textPath></text>
        <text font-size="10"><textPath href="#s">Square</textPath></text>
        <text font-size="10"><textPath href="#l">Line</textPath></text>
        <text font-size="10"><textPath href="#pl">Polyline</textPath></text>
        <text font-size="10"><textPath href="#pg">Polygon</textPath></text>"##))
    .unwrap();
    assert!(!w.iter().any(|w| w.contains("textPath")), "{w:?}");
    let t = texts(&d);
    assert_eq!(t.len(), 7);
    assert!(t.iter().all(|t| matches!(t.kind, TextKind::OnPath { .. })), "{:?}", t.iter().map(|t| &t.kind).collect::<Vec<_>>());
    let near = |a: Point, b: Point| (a - b).hypot() < 1e-6;
    // Circle: starts at 3 o'clock and runs clockwise (down first, y pointing down), around the circle.
    let (p0, next) = path_start(t[0]);
    assert!(near(p0, Point::new(250.0, 150.0)) && next.y > p0.y && next.x < p0.x, "{p0:?} {next:?}");
    let TextKind::OnPath { path, .. } = &t[0].kind else { panic!("not on a path") };
    let b = t[0].xf.transform_rect_bbox(path.bounds().unwrap());
    assert!(close(b.x0, 50.0, 1e-6) && close(b.y0, 50.0, 1e-6) && close(b.x1, 250.0, 1e-6) && close(b.y1, 250.0, 1e-6), "{b:?}");
    assert!(path.is_closed());
    // Ellipse, with the shape's own transform.
    let (p0, next) = path_start(t[1]);
    assert!(near(p0, Point::new(240.0, 150.0)) && next.y > p0.y, "{p0:?} {next:?}");
    // Rounded rect: after the top-left corner, along the top edge to the right.
    let (p0, next) = path_start(t[2]);
    assert!(near(p0, Point::new(30.0, 30.0)) && next.x > p0.x && close(next.y, 30.0, 1e-9), "{p0:?} {next:?}");
    let (p0, next) = path_start(t[3]);
    assert!(near(p0, Point::new(20.0, 30.0)) && next.x > p0.x, "{p0:?} {next:?}");
    // Line, polyline and polygon follow their points.
    assert!(near(path_start(t[4]).0, Point::new(10.0, 200.0)));
    assert!(near(path_start(t[5]).0, Point::new(10.0, 10.0)));
    let TextKind::OnPath { path, .. } = &t[6].kind else { panic!("not on a path") };
    assert!(path.is_closed() && near(path_start(t[6]).0, Point::new(10.0, 10.0)));
    assert_eq!(t[0].plain_text(), "TEXT ON A CIRCLE");
}

#[test]
fn text_path_to_an_empty_shape_still_falls_back_to_point_type() {
    let (d, w) =
        import_with_report(&svg(r##"<circle id="c" cx="5" cy="5" r="0"/><text x="5" y="10"><textPath href="#c">Zero</textPath></text>"##)).unwrap();
    assert!(matches!(only_text(&d).kind, TextKind::Point));
    assert!(w.iter().any(|w| w.contains("textPath")), "{w:?}");
}

/// #1557: the `font` shorthand sets the type's size, weight and style.
#[test]
fn the_font_shorthand_styles_type() {
    let d = open(r#"<text x="10" y="40" style="font: italic bold 30px sans-serif">Deco</text>"#);
    let t = only_text(&d);
    let st = &t.runs[0].style;
    assert!(close(st.size * t.xf.determinant().abs().sqrt(), 30.0, 1e-6), "{}", st.size);
    let style = st.font_style.to_ascii_lowercase();
    assert!(style.contains("bold") && style.contains("italic"), "{style}");
}

/// Inkscape's flowed text (`<flowRoot>`) becomes area type in its region's rectangle, a paragraph
/// per `<flowPara>`; another kind of region is left out with a warning, not silently.
#[test]
fn flowed_text_becomes_area_type() {
    let (d, w) = import_with_report(&svg(
        r#"<flowRoot style="font-size:16px;font-family:sans-serif"><flowRegion><rect x="20" y="30" width="100" height="160"/></flowRegion><flowPara>This is   flowed text</flowPara><flowPara>Second <flowSpan>one</flowSpan></flowPara></flowRoot>
        <flowRoot><flowRegion><path d="M0 0 L10 0 L5 10 Z"/></flowRegion><flowPara>Lost</flowPara></flowRoot>"#,
    ))
    .unwrap();
    let t = only_text(&d);
    assert_eq!(t.plain_text(), "This is flowed text\nSecond one");
    let TextKind::Area { frame } = &t.kind else { panic!("{:?}", t.kind) };
    let b = t.xf.transform_rect_bbox(frame.bounds().unwrap());
    assert!(close(b.x0, 20.0, 1e-6) && close(b.y0, 30.0, 1e-6) && close(b.x1, 120.0, 1e-6) && close(b.y1, 190.0, 1e-6), "{b:?}");
    assert!(close(t.runs[0].style.size * t.xf.determinant().abs().sqrt(), 16.0, 1e-6));
    assert!(w.iter().any(|w| w.contains("flowRoot")), "{w:?}");
}

/// Flowed text that can't be read is left out with a warning each, and never stops the import: no
/// region, a region too large for numbers, a flowRoot inside a flowRoot (read once, as the outer one).
#[test]
fn flowed_text_that_cannot_be_read_is_reported() {
    let (d, w) = import_with_report(&svg(
        r#"<flowRoot id="none"><flowPara>No region</flowPara></flowRoot>
        <flowRoot id="huge"><flowRegion><rect width="1e999" height="10"/></flowRegion><flowPara>Huge</flowPara></flowRoot>
        <flowRoot id="far"><flowRegion><rect x="1e308" width="1e308" height="10" transform="scale(1e308)"/></flowRegion><flowPara>Far</flowPara></flowRoot>
        <flowRoot id="outer"><flowRegion><rect width="50" height="50"/></flowRegion><flowPara>Outer</flowPara><flowRoot><flowRegion><rect width="5" height="5"/></flowRegion><flowPara>inner</flowPara></flowRoot></flowRoot>"#,
    ))
    .unwrap();
    assert_eq!(only_text(&d).plain_text(), "Outer\ninner");
    for id in ["none", "huge", "far"] {
        assert!(w.iter().any(|w| w.contains(id) && w.contains("flowRoot")), "{id}: {w:?}");
    }
}
