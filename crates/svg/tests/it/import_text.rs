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
fn metadata_ids_preserve_the_original_text_cascade() {
    for (rules, text, color) in [
        ("text:not(.x){fill:red!important} text{fill:blue!important}", r#"<text y="20">Hi</text>"#, (255, 0, 0)),
        ("#t{fill:blue} [id]{fill:red}", r#"<text id="t" y="20">Hi</text>"#, (0, 0, 255)),
        ("#holder{fill:blue} g[id]{fill:red}", r#"<g id="holder"><text y="20"><tspan>Hi</tspan></text></g>"#, (0, 0, 255)),
    ] {
        for metadata in [false, true] {
            let attribute = if metadata { r#" data-note="kept""# } else { "" };
            let doc = open(&format!(r#"<style>{rules} rect[id]{{fill:black}}</style>{text}<rect width="5" height="5"{attribute}/>"#));
            let expected = Paint::solid(vectorcraft_color::Color::rgb8(color.0, color.1, color.2));
            assert_eq!(only_text(&doc).runs[0].style.fill, expected, "{rules}, metadata {metadata}");
        }
    }
}

#[test]
fn metadata_ids_preserve_text_preprocessing_and_relative_styles() {
    for body in [
        r#"<style>g{font-size:10px} text{font-size:2em} tspan{font-size:1.5em}</style><g transform="translate(4 5)"><text y="20">A<tspan>B</tspan></text></g>"#,
        r#"<style>text{display:none;fill:red}</style><text y="20">Hidden</text>"#,
        r#"<style>text{transform:translate(10px,20px)}</style><text y="20">Moved</text>"#,
        r##"<defs><path id="p" d="M0 20L150 20"/></defs><text font-size="12"><textPath href="#p">Path</textPath></text>"##,
    ] {
        let load = |attribute: &str| {
            let doc = open(&format!(r#"<style>rect[id]{{fill:black}}</style>{body}<rect width="5" height="5"{attribute}/>"#));
            art(&doc)
                .into_iter()
                .filter_map(|n| match &n.kind {
                    NodeKind::Text(t) => Some((n.visible, serde_json::to_value(t).unwrap())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let expected = load("");
        assert!(!expected.is_empty(), "{body}");
        assert_eq!(load(r#" data-note="kept""#), expected, "{body}");
    }
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
fn a_css_transform_places_text() {
    // A `transform` declaration in a rule or `style` beats the attribute, as on shapes.
    for body in [
        r#"<style>text { transform: translate(10px, 20px) }</style><text transform="translate(300 0)" y="20">Moved</text>"#,
        r#"<text style="transform: translate(10px, 20px)" transform="translate(300 0)" y="20">Moved</text>"#,
    ] {
        let d = open(body);
        let t = only_text(&d);
        assert!(t.xf.translation().to_point().distance(Point::new(10.0, 40.0)) < 1e-9, "{body}: {:?}", t.xf);
    }
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

/// SVG 2 §5.6 (https://www.w3.org/TR/SVG2/struct.html#UseElement): the element a `<use>` shows
/// inherits from the `<use>`, not from where it is defined.
#[test]
fn a_use_of_a_text_passes_its_properties_on_to_the_text() {
    let d = open(
        r##"<defs font-size="30"><text id="t" y="50">HI</text></defs>
        <use href="#t" font-size="100" fill="red"/><g font-weight="bold"><use href="#t" x="200"/></g>
        <use href="#t" y="200" opacity="0.5" style="font-size:20px"/>"##,
    );
    let t = texts(&d);
    assert_eq!(t.len(), 3, "{t:?}");
    let s: Vec<CharStyle> = t.iter().map(|t| t.first_style()).collect();
    assert!(close(s[0].size, 100.0, 1e-9), "{}", s[0].size);
    assert_eq!(s[0].fill, Paint::solid(vectorcraft_color::Color::rgb8(255, 0, 0)));
    assert!(close(s[1].size, 12.0, 1e-9), "the defaults, not the <defs>'s 30: {}", s[1].size);
    assert_eq!(s[1].font_style, "Bold");
    assert_eq!(s[1].fill, Paint::solid(vectorcraft_color::Color::BLACK));
    assert!(close(s[2].size, 20.0, 1e-9), "{}", s[2].size);
    assert!(t[0].xf.translation().to_point().distance(Point::new(0.0, 50.0)) < 1e-9, "{:?}", t[0].xf);
    assert!(t[1].xf.translation().to_point().distance(Point::new(200.0, 50.0)) < 1e-9, "{:?}", t[1].xf);
    assert!(t[2].xf.translation().to_point().distance(Point::new(0.0, 250.0)) < 1e-9, "{:?}", t[2].xf);
    // The `<use>`'s opacity stays on the object it becomes.
    let faded: Vec<&Node> = art(&d).into_iter().filter(|n| (n.opacity - 0.5).abs() < 1e-6).collect();
    assert_eq!(faded.len(), 1, "{faded:?}");
    // The text's own properties win over the `<use>`'s.
    let d = open(r##"<defs><text id="t" y="50" font-size="20" fill="blue">HI</text></defs><use href="#t" font-size="100" fill="red"/>"##);
    let s = only_text(&d).first_style();
    assert!(close(s.size, 20.0, 1e-9), "{}", s.size);
    assert_eq!(s.fill, Paint::solid(vectorcraft_color::Color::rgb8(0, 0, 255)));
}

/// A `<use>` inside the text it shows does not make the text inherit from itself.
#[test]
fn a_use_inside_the_text_it_shows_opens() {
    let d = open(r##"<text id="t" y="50" font-size="20">HI<use href="#t" font-size="100"/><tspan><use href="#t"/></tspan></text>"##);
    assert!(texts(&d).iter().all(|t| close(t.first_style().size, 20.0, 1e-9)), "{:?}", texts(&d));
}

/// Past 1 MiB of text built for `<use>` elements, the rest show their text with its own properties.
#[test]
fn uses_of_a_long_text_stop_taking_their_properties_past_the_limit() {
    let long = "a".repeat(350_000);
    let uses: String = (1..=4).map(|i| format!(r##"<use href="#t" font-size="{}"/>"##, i * 10)).collect();
    let (d, w) = import_with_report(&svg(&format!(r##"<defs><text id="t" y="50">{long}</text></defs>{uses}"##))).unwrap();
    let sizes: Vec<f64> = texts(&d).iter().map(|t| t.first_style().size).collect();
    assert_eq!(sizes.len(), 4, "{sizes:?}");
    assert!(close(sizes[0], 10.0, 1e-9) && close(sizes[1], 20.0, 1e-9), "{sizes:?}");
    assert!(close(sizes[2], 12.0, 1e-9) && close(sizes[3], 12.0, 1e-9), "{sizes:?}");
    assert_eq!(w.iter().filter(|w| w.contains("KiB of text and text paths shown through <use> elements")).count(), 1, "{w:?}");
}

/// The path a text follows counts toward that limit: each `<use>` of the text lays it out along
/// the whole path again.
#[test]
fn uses_of_a_text_on_a_long_path_stop_taking_their_properties_past_the_limit() {
    let d = format!("M0 0{}", "h1".repeat(350_000));
    let uses: String = (1..=4).map(|i| format!(r##"<use href="#t" font-size="{}"/>"##, i * 10)).collect();
    let body = format!(r##"<defs><path id="p" d="{d}"/><text id="t"><textPath href="#p">HI</textPath></text></defs>{uses}"##);
    let (d, w) = import_with_report(&svg(&body)).unwrap();
    let sizes: Vec<f64> = texts(&d).iter().map(|t| t.first_style().size).collect();
    assert_eq!(sizes.len(), 4, "{sizes:?}");
    assert!(close(sizes[0], 10.0, 1e-9) && close(sizes[1], 20.0, 1e-9), "{sizes:?}");
    assert!(close(sizes[2], 12.0, 1e-9) && close(sizes[3], 12.0, 1e-9), "{sizes:?}");
    assert_eq!(w.iter().filter(|w| w.contains("KiB of text and text paths shown through <use> elements")).count(), 1, "{w:?}");
    // The same text on a short path is within the limit however many `<use>` elements show it.
    let uses: String = (1..=300).map(|i| format!(r##"<use href="#t" font-size="{i}"/>"##)).collect();
    let body = format!(r##"<defs><path id="p" d="M0 20L150 20"/><text id="t"><textPath href="#p">HI</textPath></text></defs>{uses}"##);
    let (d, w) = import_with_report(&svg(&body)).unwrap();
    assert!(texts(&d).iter().enumerate().all(|(i, t)| close(t.first_style().size, (i + 1) as f64, 1e-9)), "{w:?}");
    assert!(!w.iter().any(|w| w.contains("shown through <use> elements")), "{w:?}");
}

/// Many `<use>` elements of one text that inherit the same properties each get it.
#[test]
fn many_uses_of_a_text_in_one_group_all_take_its_properties() {
    let uses: String = (0..200).map(|i| format!(r##"<use href="#t" x="{i}"/>"##)).collect();
    let d = open(&format!(r##"<defs><text id="t" y="50">HI</text></defs><g font-size="40">{uses}</g>"##));
    let t = texts(&d);
    assert_eq!(t.len(), 200);
    assert!(t.iter().all(|t| close(t.first_style().size, 40.0, 1e-9)));
    assert!(t[199].xf.translation().to_point().distance(Point::new(199.0, 50.0)) < 1e-9, "{:?}", t[199].xf);
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
fn text_on_a_closed_path_wraps_past_the_path_start() {
    // A closed path 400 long: text anchored before its start, or offset past its end, wraps
    // around the path instead of stopping at its start or end.
    let d = open(
        r##"<path id="p" d="M50 50 H150 V150 H50 Z" fill="none" stroke="black"/>
        <text font-size="20" text-anchor="end"><textPath href="#p">Wave</textPath></text>
        <text font-size="20"><textPath href="#p" startOffset="440">Wave</textPath></text>
        <text font-size="20" direction="rtl"><textPath href="#p">Wave</textPath></text>
        <text font-size="20" text-anchor="middle"><textPath href="#p">Wave</textPath></text>
        <text font-size="20"><textPath href="#p" startOffset="-40">Wave</textPath></text>
        <text font-size="20"><textPath href="#p" startOffset="-10%">Wave</textPath></text>
        <path id="q" d="M50 50 H150 V150 H50 Z M200 50 H300 V150 H200 Z" fill="none" stroke="black"/>
        <text font-size="20" text-anchor="end"><textPath href="#q">Wave</textPath></text>
        <path id="o" d="M50 50 H150 V150 H50" fill="none" stroke="black"/>
        <text font-size="20" text-anchor="end"><textPath href="#o">Wave</textPath></text>
        <text font-size="20"><textPath href="#o" startOffset="440">Wave</textPath></text>"##,
    );
    let t = texts(&d);
    assert_eq!(t.len(), 9);
    let start_of = |t: &TextObject| match t.kind {
        TextKind::OnPath { start, .. } => start,
        _ => panic!("{:?}", t.kind),
    };
    let width: f64 = layout(FontDb::global(), t[0]).glyphs.iter().map(|g| g.advance).sum();
    assert!(width > 0.0);
    // Ending at the path's start: the text starts its width before the end of the loop.
    assert!(close(start_of(t[0]), 1.0 - width / 400.0, 1e-6), "{} {width}", start_of(t[0]));
    assert!(close(start_of(t[1]), 0.1, 1e-9), "{}", start_of(t[1]));
    // Right-to-left text starts at its edge toward the path's end, here the path's start.
    assert!(close(start_of(t[2]), 1.0 - width / 400.0, 1e-6), "{} {width}", start_of(t[2]));
    // Centered on the path's start: half the text before it, half after.
    assert!(close(start_of(t[3]), 1.0 - width / 800.0, 1e-6), "{} {width}", start_of(t[3]));
    // A negative startOffset, in user units or as a percentage, counts back from the path's end.
    assert!(close(start_of(t[4]), 0.9, 1e-9), "{}", start_of(t[4]));
    assert!(close(start_of(t[5]), 0.9, 1e-9), "{}", start_of(t[5]));
    // A path of two closed subpaths, and an open path, do not wrap: the text starts at the path's
    // start or end.
    assert_eq!(start_of(t[6]), 0.0);
    assert_eq!(start_of(t[7]), 0.0);
    assert_eq!(start_of(t[8]), 1.0);

    // Laid out, the end-anchored text draws every glyph up the left side and ends at the start
    // point; the centered text crosses the start point from the left side onto the top.
    let lay = layout(FontDb::global(), t[0]);
    assert!(!lay.overflow);
    assert_eq!(lay.glyphs.len(), 4);
    for g in &lay.glyphs {
        let o = t[0].xf * g.origin;
        assert!(close(o.x, 50.0, 0.05) && o.y > 50.0 && o.y < 150.0, "{o:?}");
    }
    let last = lay.glyphs.last().map(|g| t[0].xf * (g.origin + baseline_dir(g.angle) * g.advance));
    assert!(last.is_some_and(|p| close(p.x, 50.0, 0.05) && close(p.y, 50.0, 0.05)), "{last:?}");
    let lay = layout(FontDb::global(), t[3]);
    assert!(!lay.overflow);
    assert_eq!(lay.glyphs.len(), 4);
    let first = t[3].xf * lay.glyphs[0].origin;
    assert!(close(first.x, 50.0, 0.05) && first.y > 50.0, "{first:?}");
    let last = t[3].xf * lay.glyphs[3].origin;
    assert!(close(last.y, 50.0, 0.05) && last.x > 50.0, "{last:?}");

    // Text longer than the loop draws its first characters, whatever its anchor.
    let d = open(
        r##"<path id="s" d="M0 0 H20 V20 H0 Z" fill="none" stroke="black"/>
        <text font-size="20" text-anchor="end"><textPath href="#s">Wave Wave Wave</textPath></text>"##,
    );
    let t = only_text(&d);
    let lay = layout(FontDb::global(), t);
    assert!(lay.overflow);
    assert!(!lay.glyphs.is_empty() && lay.glyphs.len() < 14, "{}", lay.glyphs.len());
    assert!(lay.glyphs.iter().enumerate().all(|(i, g)| g.byte == i), "{:?}", lay.glyphs.iter().map(|g| g.byte).collect::<Vec<_>>());
}

/// The unit vector along a glyph's baseline.
fn baseline_dir(angle: f64) -> vectorcraft_geom::Vec2 {
    vectorcraft_geom::Vec2::new(angle.cos(), angle.sin())
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

/// `text-anchor` aligns a line's advance, which ends after its last character's letter spacing,
/// and VectorCraft aligns a line without that tracking: the imported lines move back by their
/// share of it and stay where a reader draws them, and a centred or right-aligned tracked line
/// exported and read back stays where it was (export anchors lines the same way).
#[test]
fn anchored_letter_spaced_lines_stay_where_a_reader_draws_them() {
    let starts = |t: &TextObject| {
        let lay = layout(FontDb::global(), t);
        lay.lines.iter().map(|l| (t.xf * lay.glyphs[l.glyph_start].origin).x).collect::<Vec<_>>()
    };
    let advances = |t: &TextObject| {
        let lay = layout(FontDb::global(), t);
        lay.lines.iter().map(|l| lay.glyphs[l.glyph_start..l.glyph_end].iter().map(|g| g.advance).sum::<f64>()).collect::<Vec<_>>()
    };
    for (anchor, share) in [("middle", 0.5), ("end", 1.0)] {
        let d = open(&format!(r#"<text x="200" y="100" text-anchor="{anchor}" letter-spacing="8" font-size="40">WAVE</text>"#));
        let t = only_text(&d);
        let (start, advance) = (starts(t)[0], advances(t)[0]);
        assert!(close(start, 200.0 - share * advance, 1e-6), "{anchor}: the line starts at {start}, not {}", 200.0 - share * advance);
        let back = import(&vectorcraft_svg::export(&d, &vectorcraft_svg::ExportOptions::default())).unwrap();
        let again = starts(only_text(&back))[0];
        assert!(close(again, start, 1e-6), "{anchor}: read back, the line starts at {again}, not {start}");
    }
    // Lines anchored differently: each moves by its own share.
    let d = open(
        r#"<text y="100" font-size="20" letter-spacing="4"><tspan x="200" text-anchor="middle">Wave</tspan><tspan x="200" dy="30" text-anchor="end">Line</tspan></text>"#,
    );
    let t = only_text(&d);
    let (s, a) = (starts(t), advances(t));
    assert!(close(s[0], 200.0 - a[0] / 2.0, 1e-6) && close(s[1], 200.0 - a[1], 1e-6), "{s:?} {a:?}");
}

/// Document-space left and right edges of the advance of each laid-out line.
fn line_edges(t: &TextObject) -> Vec<(f64, f64)> {
    let lay = layout(FontDb::global(), t);
    lay.lines
        .iter()
        .map(|l| {
            let g = &lay.glyphs[l.glyph_start..l.glyph_end];
            let left = g.iter().map(|g| (t.xf * g.origin).x).fold(f64::INFINITY, f64::min);
            let right = g.iter().map(|g| (t.xf * g.origin).x + g.advance).fold(f64::NEG_INFINITY, f64::max);
            (left, right)
        })
        .collect()
}

/// SVG 2 §11.10.1.1 (https://www.w3.org/TR/SVG2/text.html#TextAnchoringProperties): with
/// `direction: rtl`, `start` puts the right side of the text at its position and `end` puts the
/// left side there; `middle` centers it either way. The direction is the effective one of the element that
/// holds the chunk's first character.
#[test]
fn rtl_point_text_start_and_end_use_directional_edges() {
    // (markup, share of the advance left of x = 200)
    for (body, share) in [
        (r#"<text x="200" y="60" direction="rtl" font-size="20">Wave</text>"#, 1.0),
        (r#"<text x="200" y="60" direction="rtl" text-anchor="start" font-size="20">Wave</text>"#, 1.0),
        (r#"<text x="200" y="60" direction="rtl" text-anchor="middle" font-size="20">Wave</text>"#, 0.5),
        (r#"<text x="200" y="60" direction="rtl" text-anchor="end" font-size="20">Wave</text>"#, 0.0),
        (r#"<g direction="rtl"><text x="200" y="60" font-size="20">Wave</text></g>"#, 1.0),
        (r#"<g direction="rtl" text-anchor="end"><text x="200" y="60" font-size="20">Wave</text></g>"#, 0.0),
        (r#"<text x="200" y="60" direction="rtl" font-size="20"><tspan direction="ltr">Wave</tspan></text>"#, 0.0),
        (r#"<text x="200" y="60" text-anchor="start" font-size="20">Wave</text>"#, 0.0),
        (r#"<text x="200" y="60" direction="ltr" text-anchor="end" font-size="20">Wave</text>"#, 1.0),
        // With `unicode-bidi: plaintext`, the direction of the chunk's first strong character.
        (r#"<text x="200" y="60" unicode-bidi="plaintext" font-size="20">&#x5D0;&#x5D1;&#x5D2;</text>"#, 1.0),
        (r#"<text x="200" y="60" unicode-bidi="plaintext" text-anchor="end" font-size="20">12 &#x5D0;&#x5D1;</text>"#, 0.0),
        (r#"<text x="200" y="60" direction="rtl" unicode-bidi="plaintext" font-size="20">Wave</text>"#, 0.0),
        (r#"<text x="200" y="60" direction="rtl" unicode-bidi="plaintext" text-anchor="end" font-size="20">12</text>"#, 1.0),
    ] {
        let d = open(body);
        let (left, right) = line_edges(only_text(&d))[0];
        let want = 200.0 - share * (right - left);
        assert!(close(left, want, 1e-6), "{body}: the advance spans {left}..{right}, so it does not start at {want}");
    }
}

/// On a path, right-to-left text anchored by `start` ends at `startOffset` and one anchored by
/// `end` begins there (its glyphs run from the path's start in visual order); `middle` centers it.
#[test]
fn rtl_type_on_a_path_starts_at_its_directional_edge() {
    // (anchor, direction, share of the advance before the offset)
    for (anchor, direction, share) in [("start", "rtl", 1.0), ("middle", "rtl", 0.5), ("end", "rtl", 0.0), ("start", "ltr", 0.0), ("end", "ltr", 1.0)]
    {
        let d = open(&format!(
            r##"<path id="p" d="M0 100 L400 100" fill="none"/>
            <text font-size="20" text-anchor="{anchor}" direction="{direction}"><textPath href="#p" startOffset="200">Wave</textPath></text>"##
        ));
        let t = only_text(&d);
        assert!(matches!(t.kind, TextKind::OnPath { .. }), "{:?}", t.kind);
        let lay = layout(FontDb::global(), t);
        let left = lay.glyphs.iter().map(|g| (t.xf * g.origin).x).fold(f64::INFINITY, f64::min);
        let width: f64 = lay.glyphs.iter().map(|g| g.advance).sum();
        let want = 200.0 - share * width;
        assert!(close(left, want, 0.05), "{anchor} {direction}: the text starts at {left}, not {want}");
    }
}

/// Letter-spaced right-to-left lines: a line keeps the tracking after each of its characters, and
/// its whole advance sits on its anchor as `start`, `middle` and `end` place it.
#[test]
fn rtl_letter_spaced_lines_stay_on_their_anchor() {
    for (anchor, share) in [("start", 1.0), ("middle", 0.5), ("end", 0.0)] {
        let d = open(&format!(r#"<text x="200" y="100" direction="rtl" text-anchor="{anchor}" letter-spacing="8" font-size="40">WAVE</text>"#));
        let t = only_text(&d);
        let (left, right) = line_edges(t)[0];
        let want = 200.0 - share * (right - left);
        assert!(close(left, want, 1e-6), "{anchor}: the advance spans {left}..{right}, so it does not start at {want}");
        let natural = {
            let d = open(&format!(r#"<text x="200" y="100" direction="rtl" text-anchor="{anchor}" font-size="40">WAVE</text>"#));
            let (l, r) = line_edges(only_text(&d))[0];
            r - l
        };
        assert!(close(right - left, natural + 4.0 * 8.0, 1e-6), "{anchor}: the advance {} holds the tracking of all 4 letters", right - left);
    }
    // A right-to-left line anchored by `start` beside a left-to-right `middle` line: each keeps its own anchor.
    let d = open(
        r#"<text y="100" font-size="20" letter-spacing="4"><tspan x="200" direction="rtl">Wave</tspan><tspan x="200" dy="30" text-anchor="middle">Line</tspan></text>"#,
    );
    let t = only_text(&d);
    let e = line_edges(t);
    assert_eq!(e.len(), 2, "{e:?}");
    assert!(close(e[0].1, 200.0, 1e-6), "the right-to-left line ends at 200: {e:?}");
    let lay = layout(FontDb::global(), t);
    let a: f64 = lay.glyphs[lay.lines[1].glyph_start..lay.lines[1].glyph_end].iter().map(|g| g.advance).sum();
    assert!(close(e[1].0, 200.0 - a / 2.0, 1e-6), "the middle line is centered on 200 with its tracking: {e:?}");
}

/// Each line is a chunk anchored by the `text-anchor` and `direction` of its first character.
#[test]
fn rtl_lines_use_each_chunk_first_character_direction_and_anchor() {
    let d = open(
        r#"<text y="60" font-size="20"><tspan x="200" direction="rtl">Wave</tspan><tspan x="200" dy="30">Line</tspan><tspan x="200" dy="30" direction="rtl" text-anchor="end">Last</tspan><tspan x="200" dy="30" direction="rtl" text-anchor="middle">Mid</tspan></text>"#,
    );
    let e = line_edges(only_text(&d));
    assert_eq!(e.len(), 4, "{e:?}");
    assert!(close(e[0].1, 200.0, 1e-6), "a right-to-left start line ends at 200: {e:?}");
    assert!(close(e[1].0, 200.0, 1e-6), "a left-to-right start line starts at 200: {e:?}");
    assert!(close(e[2].0, 200.0, 1e-6), "a right-to-left end line starts at 200: {e:?}");
    assert!(close((e[3].0 + e[3].1) / 2.0, 200.0, 1e-6), "a middle line is centered on 200: {e:?}");
}

/// The Character Direction of the run holding `part`.
fn char_direction_of(t: &TextObject, part: &str) -> Option<vectorcraft_doc::ParaDirection> {
    t.runs.iter().find(|r| r.text.contains(part)).unwrap_or_else(|| panic!("no run holds {part:?}: {:?}", t.runs)).style.char_direction
}

/// SVG 1.1 §10.7.4: a `<tspan>` whose `unicode-bidi` is `embed` or `bidi-override` opens a bidi
/// embedding in its `direction`; with `normal`, its `direction` leaves the ordering alone. The
/// embedding becomes the Character Direction of its characters, and the `<text>` keeps its own
/// paragraph direction. https://www.w3.org/TR/SVG11/text.html#RelationshipWithBi-directionality
#[test]
fn a_tspan_bidi_embedding_becomes_its_character_direction() {
    use vectorcraft_doc::ParaDirection::{LeftToRight as Ltr, RightToLeft as Rtl};
    let read = |body: &str| {
        let (d, w) = import_with_report(&svg(body)).unwrap_or_else(|e| panic!("{e}"));
        (only_text(&d).clone(), w)
    };
    // (outer direction, inner direction, unicode-bidi, the inner run's Character Direction, a warning)
    for (outer, inner, bidi, want, warns) in [
        ("ltr", "rtl", "embed", Some(Rtl), false),
        ("rtl", "ltr", "embed", Some(Ltr), false),
        ("ltr", "rtl", "bidi-override", Some(Rtl), true),
        ("rtl", "ltr", "bidi-override", Some(Ltr), true),
        ("ltr", "rtl", "normal", None, false),
        ("rtl", "ltr", "normal", None, false),
        ("ltr", "rtl", "EMBED", Some(Rtl), false),
        ("ltr", "rtl", "sideways", None, false),
    ] {
        let (t, w) = read(&format!(
            r#"<text x="20" y="40" font-size="20" direction="{outer}">ab <tspan direction="{inner}" unicode-bidi="{bidi}">1 2</tspan> cd</text>"#
        ));
        let case = format!("{outer} text, {inner} {bidi} tspan");
        assert_eq!(t.plain_text(), "ab 1 2 cd", "{case}: the characters stay in logical order");
        assert_eq!(char_direction_of(&t, "1 2"), want, "{case}: {:?}", t.runs);
        assert_eq!(char_direction_of(&t, "ab"), None, "{case}");
        assert_eq!(t.para_at(0).direction == Some(Rtl), outer == "rtl", "{case}: the paragraph keeps the text's direction");
        assert_eq!(w.iter().any(|w| w.contains("bidi-override") && w.contains("not supported")), warns, "{case}: {w:?}");
        if !warns {
            assert!(w.is_empty(), "{case}: {w:?}");
        }
    }
    // In left-to-right text, digits in a right-to-left embedding are drawn right to left.
    let order = |bidi: &str| {
        let (t, _) = read(&format!(r#"<text x="20" y="40" font-size="20">ab <tspan direction="rtl" unicode-bidi="{bidi}">1 2</tspan></text>"#));
        let lay = layout(FontDb::global(), &t);
        let x = |byte: usize| lay.glyphs.iter().find(|g| g.byte == byte).map(|g| g.origin.x).unwrap_or_else(|| panic!("no glyph at {byte}"));
        x(3) < x(5)
    };
    assert!(!order("embed"), "an embedding draws 2 before 1");
    assert!(order("normal"), "without one, 1 comes first");
}

/// `unicode-bidi` is not inherited: a `<tspan>` without one stays in its parent's embedding, and
/// `inherit` takes the parent's value. CSS rules and `style` set it as they set any property.
#[test]
fn unicode_bidi_is_not_inherited_and_reads_from_the_cascade() {
    use vectorcraft_doc::ParaDirection::{LeftToRight as Ltr, RightToLeft as Rtl};
    let read = |body: &str| {
        let (d, w) = import_with_report(&svg(body)).unwrap_or_else(|e| panic!("{e}"));
        (only_text(&d).clone(), w)
    };
    // An inner `<tspan>` with only a `direction` stays in the outer embedding.
    let (t, w) = read(r#"<text y="40">ab <tspan direction="rtl" unicode-bidi="embed">12 <tspan direction="ltr">34</tspan></tspan> cd</text>"#);
    assert_eq!((char_direction_of(&t, "12"), char_direction_of(&t, "34"), char_direction_of(&t, "cd")), (Some(Rtl), Some(Rtl), None), "{:?}", t.runs);
    assert!(w.is_empty(), "{w:?}");
    // `inherit` opens an embedding of its own, inside the outer one: kept one level deep, with a warning.
    let (t, w) = read(
        r#"<text y="40">ab <tspan direction="rtl" unicode-bidi="embed">12 <tspan direction="ltr" unicode-bidi="inherit">34</tspan></tspan></text>"#,
    );
    assert_eq!((char_direction_of(&t, "12"), char_direction_of(&t, "34")), (Some(Rtl), Some(Ltr)), "{:?}", t.runs);
    assert!(w.len() == 1 && w[0].contains("inside another"), "{w:?}");
    // A `unicode-bidi` on the `<text>` or a group is not passed on to the `<tspan>`.
    let (t, _) = read(r#"<g unicode-bidi="embed"><text y="40" unicode-bidi="embed">ab <tspan direction="rtl">12</tspan></text></g>"#);
    assert!(t.runs.iter().all(|r| r.style.char_direction.is_none()), "{:?}", t.runs);
    // From a CSS rule and from `style`; `direction` inherits from the `<text>`.
    for body in [
        r#"<style>.e{unicode-bidi:embed}</style><text y="40" direction="rtl">ab <tspan class="e">12</tspan></text>"#,
        r#"<text y="40" direction="rtl">ab <tspan style="unicode-bidi: embed">12</tspan></text>"#,
    ] {
        let (t, w) = read(body);
        assert_eq!(char_direction_of(&t, "12"), Some(Rtl), "{body}: {:?}", t.runs);
        assert_eq!(char_direction_of(&t, "ab"), None, "{body}: {:?}", t.runs);
        assert!(w.is_empty(), "{body}: {w:?}");
    }
    // On a path and in a link too.
    let (t, _) = read(
        r##"<defs><path id="p" d="M20 100 H380"/></defs><text><textPath href="#p">ab <tspan direction="ltr" unicode-bidi="embed">12</tspan> <a href="https://example.com/"><tspan direction="rtl" unicode-bidi="embed">34</tspan></a></textPath></text>"##,
    );
    assert!(matches!(t.kind, TextKind::OnPath { .. }));
    assert_eq!((char_direction_of(&t, "12"), char_direction_of(&t, "34")), (Some(Ltr), Some(Rtl)), "{:?}", t.runs);
}

/// The text model holds embeddings only: isolates and overrides become embeddings in their
/// `direction`, and `plaintext` one in the direction of its first strong character, each with a
/// warning once a text (CSS Writing Modes 3 §2.2,
/// https://www.w3.org/TR/css-writing-modes-3/#unicode-bidi).
#[test]
fn unicode_bidi_values_the_text_model_cannot_hold_are_reported() {
    use vectorcraft_doc::ParaDirection::{LeftToRight as Ltr, RightToLeft as Rtl};
    for (bidi, dir, text, want, says) in [
        ("isolate", "rtl", "12", Some(Rtl), "isolation is not supported"),
        ("isolate-override", "rtl", "12", Some(Rtl), "isolation and override are not supported"),
        ("plaintext", "ltr", "\u{5D0}\u{5D1}", Some(Rtl), "first strong character"),
        ("plaintext", "rtl", "12 ab", Some(Ltr), "first strong character"),
    ] {
        let body = format!(
            r#"<text id="t" y="40">ab <tspan direction="{dir}" unicode-bidi="{bidi}">{text}</tspan> <tspan direction="{dir}" unicode-bidi="{bidi}">{text}</tspan></text>"#
        );
        let (d, w) = import_with_report(&svg(&body)).unwrap_or_else(|e| panic!("{e}"));
        let t = only_text(&d);
        assert_eq!(char_direction_of(t, text), want, "{bidi}: {:?}", t.runs);
        assert!(w.len() == 1 && w[0].starts_with("text 't': ") && w[0].contains(&format!("\"{bidi}\"")) && w[0].contains(says), "{bidi}: {w:?}");
    }
    // `plaintext` looks for the first strong character among the characters imported, however far in.
    let far = format!("{}\u{5D0}\u{5D1}", "1 ".repeat(1500));
    for (body, part, want) in [
        (r#"<tspan display="none">&#x5D0;&#x5D1;</tspan>ab"#.to_string(), "ab", Ltr),
        (format!("<tspan>{far}</tspan>"), "\u{5D0}", Rtl),
        (format!("<tspan>\n</tspan>12 <tspan>{far}</tspan>"), "\u{5D0}", Rtl),
    ] {
        let (d, _) =
            import_with_report(&svg(&format!(r#"<text y="40">x <tspan direction="ltr" unicode-bidi="plaintext">{body}</tspan></text>"#))).unwrap();
        let t = only_text(&d);
        assert_eq!(char_direction_of(t, part), Some(want), "{body}: {:?}", t.runs);
    }
    // `plaintext` on the `<text>`: each paragraph runs in the direction of its first strong character.
    for (body, want) in [
        ("<text y=\"40\" unicode-bidi=\"plaintext\">12 \u{5D0}\u{5D1} cd</text>", vec![Some(Rtl)]),
        (r#"<text y="40" direction="rtl" unicode-bidi="plaintext">12 ab</text>"#, vec![Some(Ltr)]),
        (
            "<text unicode-bidi=\"plaintext\"><tspan x=\"10\" y=\"20\">ab</tspan><tspan x=\"10\" dy=\"40\">\u{5D0}\u{5D1} cd</tspan></text>",
            vec![Some(Ltr), Some(Rtl)],
        ),
        // A paragraph without a strong character runs left to right.
        (
            "<text unicode-bidi=\"plaintext\"><tspan x=\"10\" y=\"20\">\u{5D0}\u{5D1}</tspan><tspan x=\"10\" dy=\"40\">1-2</tspan></text>",
            vec![Some(Rtl), Some(Ltr)],
        ),
    ] {
        let (d, w) = import_with_report(&svg(body)).unwrap();
        let t = only_text(&d);
        let got: Vec<_> = (0..t.paragraph_styles().len()).map(|i| t.para_at(i).direction).collect();
        assert_eq!(got, want, "{body}");
        assert!(w.is_empty(), "{body}: {w:?}");
    }
    // Absolutely placed characters sit on their x in the direction of their own paragraph.
    let (d, _) = import_with_report(&svg(
        r#"<text unicode-bidi="plaintext" font-size="20"><tspan x="200" y="40">&#x5D0;&#x5D1;</tspan><tspan x="100" y="80">ab &#x5D0;&#x5D1; cd</tspan><tspan x="300">ef</tspan></text>"#,
    ))
    .unwrap();
    let t = only_text(&d);
    assert_eq!((t.para_at(0).direction, t.para_at(1).direction), (Some(Rtl), Some(Ltr)));
    let lay = layout(FontDb::global(), t);
    for (byte, x) in [(5, 100.0), (15, 300.0)] {
        let g = lay.glyphs.iter().find(|g| g.byte == byte).unwrap_or_else(|| panic!("no glyph at {byte}"));
        let left = (t.xf * g.origin).x;
        assert!(close(left, x, 0.05), "the character at {byte} starts at {left}, not {x}");
    }
    // An override on the `<text>` keeps its paragraph direction.
    let (d, w) = import_with_report(&svg(r#"<text y="40" direction="rtl" unicode-bidi="bidi-override">ab</text>"#)).unwrap();
    assert_eq!(only_text(&d).para_at(0).direction, Some(Rtl));
    assert!(w.len() == 1 && w[0].contains("override on the text"), "{w:?}");
}

/// A line starts at its own x, also when it is anchored as the line before it is and when another
/// line holds absolutely placed characters.
#[test]
fn each_line_starts_at_its_own_x() {
    let d = open(r#"<text y="40" font-size="20"><tspan x="50">Wave</tspan><tspan x="120" dy="30">Line</tspan></text>"#);
    let e = line_edges(only_text(&d));
    assert!(close(e[0].0, 50.0, 1e-6) && close(e[1].0, 120.0, 1e-6), "{e:?}");
    let d = open(r#"<text y="40" font-size="20" text-anchor="end"><tspan x="150">Wave</tspan><tspan x="100" dy="30">Line</tspan></text>"#);
    let e = line_edges(only_text(&d));
    assert!(close(e[0].1, 150.0, 1e-6) && close(e[1].1, 100.0, 1e-6), "{e:?}");
    let d = open(r#"<text y="40" font-size="20"><tspan x="50 100">ab</tspan><tspan x="200" dy="30" direction="rtl">Line</tspan></text>"#);
    let t = only_text(&d);
    let e = line_edges(t);
    assert_eq!(e.len(), 2, "{e:?}");
    assert!(close(e[0].0, 50.0, 0.05) && close(e[1].1, 200.0, 0.05), "{e:?}");
    let b = (t.xf * layout(FontDb::global(), t).glyphs[1].origin).x;
    assert!(close(b, 100.0, 0.05), "b starts at {b}");
}

/// Absolutely placed chunks are anchored on their own, each by the `text-anchor` and `direction`
/// of its first character, whatever the first chunk's anchor.
#[test]
fn absolutely_placed_chunks_use_their_own_rtl_anchor() {
    // (markup, share of each chunk's advance left of its x)
    for (body, shares) in [
        (r#"<text x="80 200" y="40" font-size="20">a<tspan direction="rtl">b</tspan></text>"#, [0.0, 1.0]),
        (r#"<text x="80 200" y="40" font-size="20">a<tspan direction="rtl" text-anchor="end">b</tspan></text>"#, [0.0, 0.0]),
        (r#"<text y="40" font-size="20"><tspan x="80">a</tspan><tspan x="200" direction="rtl">b</tspan></text>"#, [0.0, 1.0]),
        (r#"<text x="80 200" y="40" text-anchor="middle" font-size="20">a<tspan direction="rtl" text-anchor="end">b</tspan></text>"#, [0.5, 0.0]),
        (r#"<text x="80 200" y="40" text-anchor="middle" font-size="20">a<tspan direction="rtl" text-anchor="start">b</tspan></text>"#, [0.5, 1.0]),
        (r#"<text x="80 200" y="40" text-anchor="middle" font-size="20">a<tspan direction="rtl">b</tspan></text>"#, [0.5, 0.5]),
        (r#"<text x="80 200" y="40" direction="rtl" font-size="20">ab</text>"#, [1.0, 1.0]),
    ] {
        let d = open(body);
        let t = only_text(&d);
        let lay = layout(FontDb::global(), t);
        assert_eq!(lay.glyphs.len(), 2, "{body}");
        // Natural advance: the kerning that moves `b` onto its x is added after `a`.
        let style = |i: usize| t.runs.iter().flat_map(|r| r.text.chars().map(move |_| &r.style)).nth(i).unwrap();
        let natural = |i: usize| lay.glyphs[i].advance - style(i).kerning.unwrap_or(0.0) / 1000.0 * style(i).size;
        for (i, (x, share)) in [80.0, 200.0].into_iter().zip(shares).enumerate() {
            let left = (t.xf * lay.glyphs[i].origin).x;
            let want = x - share * natural(i);
            assert!(close(left, want, 0.05), "{body}: chunk {i} starts at {left}, not {want}");
        }
    }
}

/// A right-to-left chunk of several characters is anchored by its visual edges: its first
/// character is drawn at its right, so the chunk runs left from that character's right edge.
#[test]
fn absolutely_placed_rtl_chunks_of_several_characters_anchor_by_their_visual_edges() {
    // (markup, share of the right-to-left chunk's advance left of x = 200)
    for (body, share) in [
        (r#"<text x="80 200" y="40" font-size="20">a<tspan direction="rtl">אב</tspan></text>"#, 1.0),
        (r#"<text x="80 200" y="40" font-size="20">a<tspan direction="rtl" text-anchor="middle">אב</tspan></text>"#, 0.5),
        (r#"<text x="80 200" y="40" font-size="20">a<tspan direction="rtl" text-anchor="end">אב</tspan></text>"#, 0.0),
        (r#"<text x="80 200" y="40" font-size="20">a<tspan text-anchor="end">אב</tspan></text>"#, 1.0),
        // A digit is not a strong character: the text still runs left to right.
        (r#"<text x="80 200" y="40" font-size="20">1<tspan direction="rtl" text-anchor="end">אב</tspan></text>"#, 0.0),
        // The chunk starts a line, and another line holds absolutely placed characters.
        (r#"<text y="40" font-size="20"><tspan x="200" direction="rtl">אב</tspan><tspan x="50 100" dy="30">ab</tspan></text>"#, 1.0),
        (
            r#"<text y="40" font-size="20"><tspan x="200" direction="rtl" text-anchor="middle">אב</tspan><tspan x="50 100" dy="30">ab</tspan></text>"#,
            0.5,
        ),
        (
            r#"<text y="40" font-size="20"><tspan x="200" direction="rtl" text-anchor="end">אב</tspan><tspan x="50 100" dy="30">ab</tspan></text>"#,
            0.0,
        ),
    ] {
        let d = open(body);
        let t = only_text(&d);
        let lay = layout(FontDb::global(), t);
        let chunk: Vec<_> = lay.glyphs.iter().filter(|g| g.rtl).collect();
        assert_eq!(chunk.len(), 2, "{body}");
        assert!(chunk.iter().all(|g| g.advance > 1.0), "{body}: both letters have an advance");
        let left = chunk.iter().map(|g| (t.xf * g.origin).x).fold(f64::INFINITY, f64::min);
        let right = chunk.iter().map(|g| (t.xf * g.origin).x + g.advance).fold(f64::NEG_INFINITY, f64::max);
        let want = 200.0 - share * (right - left);
        assert!(close(left, want, 0.05), "{body}: the chunk spans {left}..{right}, so it does not start at {want}");
        // The first character is the right one of the two.
        let first = lay.glyphs.iter().filter(|g| g.rtl).min_by_key(|g| g.byte).map(|g| (t.xf * g.origin).x);
        assert!(first.is_some_and(|x| x > left + 1.0), "{body}: the first character starts at {first:?}, the chunk at {left}");
    }
}

#[test]
fn a_small_line_gap_in_tiny_type_keeps_its_leading() {
    // 0.0085 against an auto leading of 0.006 differs by less than 0.01 but is not a whole multiple of it.
    let d = open(r#"<text x="1" y="1" font-size="0.005">One<tspan x="1" dy="0.0085">Two</tspan></text>"#);
    let t = only_text(&d);
    let two = t.runs.iter().find(|r| r.text.contains("Two")).unwrap().style.leading;
    assert!(two.is_some_and(|l| close(l, 0.0085, 1e-9)), "{two:?}");
}

#[test]
fn blank_lines_in_tiny_type_keep_their_own_spacing() {
    // B sits 0.0085 below A (kept as its leading); C is three 0.006 lines below B, so the two blank
    // lines between them must be 0.006 apart, not carry B's 0.0085.
    let d = open(r#"<text x="1" y="1" font-size="0.005">A<tspan x="1" dy="0.0085">B</tspan><tspan x="1" dy="0.018">C</tspan></text>"#);
    let t = only_text(&d);
    let blank = t.runs.iter().find(|r| r.text == "\n\n").expect("a run of the two blank lines");
    assert!(blank.style.leading.is_some_and(|l| close(l, 0.006, 1e-9)), "{:?}", blank.style.leading);
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
fn comment_markers_inside_css_strings_are_not_comments() {
    let d = open(r#"<style>text { font-family: "A/*B"; fill: red }</style><text y="10">x</text>"#);
    let t = only_text(&d);
    assert_eq!(t.first_style().fill.color().map(|c| c.to_hex()).as_deref(), Some("#ff0000"));
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

/// Total advance of a single-line text.
fn advance(t: &TextObject) -> f64 {
    layout(FontDb::global(), t).glyphs.iter().map(|g| g.advance).sum()
}

/// SVG 1.1 §10.9.1: `textLength` fits the characters to that advance, by spacing them out
/// (`lengthAdjust="spacing"`, the default) or by stretching them (`spacingAndGlyphs`).
/// https://www.w3.org/TR/SVG11/text.html#TextElementTextLengthAttribute
#[test]
fn text_length_fits_the_advance() {
    let natural = advance(only_text(&open(r#"<text x="20" y="100" font-size="20">Hello World</text>"#)));
    for adjust in ["spacingAndGlyphs", "spacing"] {
        for want in [50.0, 150.0, 300.0] {
            let (d, w) = import_with_report(&svg(&format!(
                r#"<text x="20" y="100" font-size="20" textLength="{want}" lengthAdjust="{adjust}">Hello World</text>"#
            )))
            .unwrap();
            assert!(w.is_empty(), "{w:?}");
            let t = only_text(&d);
            assert_eq!(t.plain_text(), "Hello World");
            let got = advance(t);
            assert!(close(got, want, 0.05), "{adjust} {want}: {got} (natural {natural})");
            let scaled = t.runs.iter().all(|r| close(r.style.h_scale, 100.0 * want / natural, 0.5));
            assert_eq!(scaled, adjust == "spacingAndGlyphs", "{adjust} {want}: {:?}", t.runs);
        }
    }
    // On a tspan, only its characters are fitted; units and percentages of the viewport resolve.
    let t = open(r#"<text x="20" y="100" font-size="20">Hi <tspan textLength="25%">there</tspan>!</text>"#);
    let t = only_text(&t);
    let lay = layout(FontDb::global(), t);
    let span: f64 = lay.glyphs.iter().filter(|g| (3..8).contains(&g.byte)).map(|g| g.advance).sum();
    assert!(close(span, 100.0, 0.05), "{span}");
    // A negative length is ignored.
    let t = open(r#"<text x="20" y="100" font-size="20" textLength="-5">Hello World</text>"#);
    assert!(close(advance(only_text(&t)), natural, 1e-6));
}
