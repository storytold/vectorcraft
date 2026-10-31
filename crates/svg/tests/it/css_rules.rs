//! CSS for objects (the CSS Properties panel): shapes give backgrounds, borders and corner radii,
//! type its font, gradients `linear-gradient()`/`radial-gradient()`, and every property both write
//! reads as SVG export writes it.

use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, Effect, Justify, LiveShape, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::shapes::{self, CornerKind};
use vectorcraft_geom::{Affine, Point, Rect};
use vectorcraft_svg::{CssOptions, CssRule, CssUnits, ExportOptions, Styling, css_rules, export};

fn doc() -> Document {
    Document::new(400.0, 300.0)
}

fn add(d: &mut Document, n: Node) -> NodeId {
    let (l, id) = (d.layers[0].id, n.id);
    d.insert(Some(l), usize::MAX, n).unwrap();
    id
}

/// A live rectangle at (x, y), `w` × `h`, with corner `radius`, filled red and stroked 2 pt blue.
fn rect(d: &mut Document, (x, y, w, h): (f64, f64, f64, f64), radius: f64) -> Node {
    let fill = Paint::solid(Color::rgb8(255, 0, 0));
    let r = Rect::new(0.0, 0.0, w, h);
    let mut n = Node::path(
        d.alloc_id(),
        shapes::rounded_rectangle(r, radius).transformed(Affine::translate((x, y))),
        Appearance::basic(fill, Paint::solid(Color::rgb8(0, 0, 255)), 2.0),
    );
    if let NodeKind::Path { live, .. } = &mut n.kind {
        *live = Some(Box::new(LiveShape::Rectangle {
            w,
            h,
            radii: [radius; 4],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::translate((x, y)),
        }));
    }
    n
}

fn text(d: &mut Document, s: &str, st: CharStyle) -> Node {
    Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 40.0), s, st))))
}

fn prop<'a>(r: &'a CssRule, k: &str) -> Option<&'a str> {
    r.props.iter().find(|(p, _)| *p == k).map(|(_, v)| v.as_str())
}

fn one(d: &Document, id: NodeId, opts: &CssOptions) -> CssRule {
    let sheet = css_rules(d, &[id], opts);
    assert_eq!(sheet.rules.len(), 1, "{sheet:?}");
    sheet.rules.into_iter().next().unwrap()
}

#[test]
fn rectangle_gives_background_border_and_radius() {
    let mut d = doc();
    let mut n = rect(&mut d, (10.0, 20.0, 100.0, 50.0), 8.0);
    n.name = Some("Hero Card".into());
    let id = add(&mut d, n);
    let opts = CssOptions { position: true, ..CssOptions::default() };
    let r = one(&d, id, &opts);
    assert_eq!(r.selector, ".Hero_Card");
    assert_eq!(prop(&r, "background-color"), Some("#ff0000"));
    assert_eq!(prop(&r, "border"), Some("2px solid #0000ff"));
    assert_eq!(prop(&r, "border-radius"), Some("8px"));
    assert_eq!((prop(&r, "position"), prop(&r, "left"), prop(&r, "top")), (Some("absolute"), Some("10px"), Some("20px")));
    assert_eq!((prop(&r, "width"), prop(&r, "height")), (Some("100px"), Some("50px")));
    assert!(r.unsupported.is_none() && r.image.is_none());
    assert_eq!(
        r.text(),
        ".Hero_Card {\n  position: absolute;\n  left: 10px;\n  top: 20px;\n  width: 100px;\n  height: 50px;\n  background-color: #ff0000;\n  border: 2px solid #0000ff;\n  border-radius: 8px;\n}"
    );
}

/// An object centered on an artboard's right or bottom edge is positioned from that artboard's
/// top-left corner; on the edge two artboards share, away from its ends, from the one to the right
/// of it.
#[test]
fn position_is_from_the_artboard_whose_edge_holds_the_center() {
    let mut d = doc();
    // Artboard 2 is (500, 0) to (900, 300); artboard 3 starts at its right edge.
    for rect in [Rect::new(500.0, 0.0, 900.0, 300.0), Rect::new(900.0, 0.0, 1000.0, 300.0)] {
        let mut a = d.artboards[0].clone();
        a.rect = rect;
        d.artboards.push(a);
    }
    let opts = CssOptions { position: true, ..CssOptions::default() };
    for ((x, y), want) in [((980.0, 100.0), ("80px", "100px")), ((600.0, 280.0), ("100px", "280px")), ((880.0, 100.0), ("-20px", "100px"))] {
        let n = rect(&mut d, (x, y, 40.0, 40.0), 0.0);
        let id = add(&mut d, n);
        let r = one(&d, id, &opts);
        assert_eq!((prop(&r, "left"), prop(&r, "top")), (Some(want.0), Some(want.1)), "at ({x}, {y})");
    }
}

#[test]
fn live_corners_give_per_corner_radii_and_only_round_ones() {
    let mut d = doc();
    let mut n = rect(&mut d, (0.0, 0.0, 100.0, 50.0), 0.0);
    if let Some(LiveShape::Rectangle { radii, kinds, .. }) = n.live_shape_mut() {
        *radii = [0.0, 8.0, 0.0, 4.0];
        kinds[0] = CornerKind::Chamfer;
    }
    let id = add(&mut d, n);
    assert_eq!(prop(&one(&d, id, &CssOptions::default()), "border-radius"), Some("0 8px 0 4px"), "a square corner's kind doesn't matter");
    if let Some(LiveShape::Rectangle { kinds, .. }) = d.node_mut(id).and_then(|n| n.live_shape_mut()) {
        kinds[1] = CornerKind::InvertedRound;
    }
    assert!(prop(&one(&d, id, &CssOptions::default()), "border-radius").is_none(), "CSS can't cut a corner in");
    // A radius is drawn no larger than its neighbours leave room for, past half the shorter
    // side when they are small (#2332), and exported as drawn: 50 less the 4 below it.
    if let Some(LiveShape::Rectangle { radii, kinds, .. }) = d.node_mut(id).and_then(|n| n.live_shape_mut()) {
        *radii = [60.0, 8.0, 0.0, 4.0];
        *kinds = Default::default();
    }
    assert_eq!(prop(&one(&d, id, &CssOptions::default()), "border-radius"), Some("46px 8px 0 4px"));
}

#[test]
fn ellipses_round_fully_and_options_change_units_and_names() {
    let mut d = doc();
    let mut e =
        Node::path(d.alloc_id(), shapes::ellipse(Rect::new(0.0, 0.0, 72.0, 36.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    if let NodeKind::Path { live, .. } = &mut e.kind {
        *live = Some(Box::new(LiveShape::Ellipse { w: 72.0, h: 36.0, pie: (0.0, 360.0), xf: Affine::IDENTITY }));
    }
    let e = add(&mut d, e);
    let a = rect(&mut d, (0.0, 0.0, 10.0, 10.0), 0.0);
    let a = add(&mut d, a);
    let b = rect(&mut d, (20.0, 0.0, 10.0, 10.0), 0.0);
    let b = add(&mut d, b);
    let opts = CssOptions { units: CssUnits::In, ..CssOptions::default() };
    let sheet = css_rules(&d, &[e, a, b], &opts);
    let sel: Vec<&str> = sheet.rules.iter().map(|r| r.selector.as_str()).collect();
    assert_eq!(sel, [".ellipse", ".rectangle", ".rectangle-2"], "unnamed objects are named after their kind, uniquely");
    assert_eq!(prop(&sheet.rules[0], "border-radius"), Some("50%"));
    assert_eq!((prop(&sheet.rules[0], "width"), prop(&sheet.rules[0], "height")), (Some("1in"), Some("0.5in")));
    assert!(prop(&sheet.rules[1], "border-radius").is_none(), "square corners");
    // Named objects only: the unnamed ones are counted, not written.
    let named = css_rules(&d, &[e, a, b], &CssOptions { unnamed: false, ..CssOptions::default() });
    assert!(named.rules.is_empty());
    assert_eq!(named.skipped, 3);
    // No dimensions: no width or height.
    let r = one(&d, a, &CssOptions { dimensions: false, ..CssOptions::default() });
    assert!(prop(&r, "width").is_none() && prop(&r, "height").is_none());
}

#[test]
fn type_gives_font_properties() {
    let mut d = doc();
    let st = CharStyle {
        font_family: "Source Sans 3".into(),
        font_style: "Bold Italic".into(),
        size: 18.0,
        tracking: 50.0,
        leading: Some(24.0),
        underline: true,
        fill: Paint::solid(Color::rgb8(0x33, 0x66, 0x99)),
        ..CharStyle::default()
    };
    let mut n = text(&mut d, "Hello", st);
    if let NodeKind::Text(t) = &mut n.kind {
        t.para.justify = Justify::Center;
    }
    let id = add(&mut d, n);
    let r = one(&d, id, &CssOptions::default());
    assert_eq!(r.selector, ".type");
    assert_eq!(prop(&r, "font-family"), Some("'Source Sans 3', sans-serif"));
    assert_eq!(prop(&r, "font-size"), Some("18px"));
    assert_eq!(prop(&r, "font-weight"), Some("bold"));
    assert_eq!(prop(&r, "font-style"), Some("italic"));
    assert_eq!(prop(&r, "color"), Some("#336699"));
    assert_eq!(prop(&r, "letter-spacing"), Some("0.9px"));
    assert_eq!(prop(&r, "line-height"), Some("24px"));
    assert_eq!(prop(&r, "text-decoration"), Some("underline"));
    assert_eq!(prop(&r, "text-align"), Some("center"));
    assert!(prop(&r, "background-color").is_none() && r.unsupported.is_none());
}

/// Area type that Shrink Text to Fit scales down gives the size, spacing and leading it draws at.
#[test]
fn shrunk_area_type_gives_the_size_it_draws_at() {
    let mut d = doc();
    let st = CharStyle { size: 18.0, tracking: 50.0, leading: Some(24.0), fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
    let mut n = text(&mut d, "Area type that is too long for its frame", st);
    let mut fit = 1.0;
    if let NodeKind::Text(t) = &mut n.kind {
        t.kind = vectorcraft_doc::TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 40.0)) };
        t.area.fit = vectorcraft_doc::AreaFit::ShrinkText { min_percent: 10.0 };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        assert!(!lay.overflow && lay.fit_scale < 0.9, "the text is shrunk to fit: {}", lay.fit_scale);
        fit = lay.fit_scale;
    }
    let id = add(&mut d, n);
    let r = one(&d, id, &CssOptions::default());
    let px = |v: f64| format!("{}px", vectorcraft_svg::fmt_num(v, 3));
    assert_eq!(prop(&r, "font-size"), Some(px(18.0 * fit).as_str()), "{r:?}");
    assert_eq!(prop(&r, "letter-spacing"), Some(px(0.9 * fit).as_str()), "{r:?}");
    assert_eq!(prop(&r, "line-height"), Some(px(24.0 * fit).as_str()), "{r:?}");
}

#[test]
fn shared_properties_read_as_svg_export_writes_them() {
    let mut d = doc();
    let st = CharStyle {
        font_family: "Source Sans 3".into(),
        font_style: "Bold".into(),
        size: 18.0,
        kerning: Some(20.0),
        features: vec!["dlig".into(), "-liga".into()],
        strikethrough: true,
        fill: Paint::solid(Color::BLACK),
        ..CharStyle::default()
    };
    let mut n = text(&mut d, "Hello", st);
    n.opacity = 0.5;
    n.blend = BlendMode::Multiply;
    let id = add(&mut d, n);
    let r = one(&d, id, &CssOptions::default());
    let svg = export(&d, &ExportOptions { styling: Styling::InternalCss, ..ExportOptions::default() });
    let style = &svg[svg.find("<style>").unwrap()..svg.find("</style>").unwrap()];
    for k in ["font-family", "font-weight", "font-kerning", "font-feature-settings", "text-decoration", "opacity", "mix-blend-mode"] {
        let v = prop(&r, k).unwrap_or_else(|| panic!("{k} is written: {r:?}"));
        // The SVG sheet escapes quotes as XML.
        let decl = format!("{k}:{v}").replace('"', "&quot;").replace('\'', "&apos;");
        assert!(style.contains(&decl), "{decl} as in SVG export: {style}");
    }
    // Lengths are the same, with units: CSS takes a bare number only for 0, and a browser drops
    // `font-size:18`, leaving the type at its default size (#3209). Presentation attributes take
    // bare numbers (`text_layout`).
    let spacing = prop(&r, "letter-spacing").unwrap_or_default();
    assert!(prop(&r, "font-size") == Some("18px") && spacing.ends_with("px"), "{r:?}");
    for decl in ["font-size:18px".to_string(), format!("letter-spacing:{spacing}")] {
        assert!(style.contains(&decl), "{decl} as in SVG export: {style}");
    }
    let inline = export(&d, &ExportOptions { styling: Styling::InlineStyle, ..ExportOptions::default() });
    assert!(inline.contains("font-size:18px") && inline.contains(&format!("letter-spacing:{spacing}")), "{inline}");
}

#[test]
fn gradients_give_css_gradients() {
    let mut d = doc();
    let mut n = rect(&mut d, (0.0, 0.0, 200.0, 100.0), 0.0);
    n.appearance = Appearance::basic(Paint::Gradient(Box::new(GradientPaint::new(Gradient::default()))), Paint::None, 0.0).into();
    let id = add(&mut d, n);
    let r = one(&d, id, &CssOptions::default());
    assert_eq!(prop(&r, "background-image"), Some("linear-gradient(90deg, #ffffff 0%, #000000 100%)"));
    // Placed top to bottom over the upper half: the stops sit on the vertical gradient line.
    let mut g = GradientPaint::new(Gradient::default());
    g.geom = Some(GradientGeom { start: Point::new(100.0, 0.0), end: Point::new(100.0, 50.0), aspect: 1.0, focal: None });
    let mut n = rect(&mut d, (0.0, 0.0, 200.0, 100.0), 0.0);
    n.appearance = Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 0.0).into();
    let id = add(&mut d, n);
    assert_eq!(prop(&one(&d, id, &CssOptions::default()), "background-image"), Some("linear-gradient(180deg, #ffffff 0%, #000000 50%)"));
    // Radial: a circle at the box's centre.
    let radial = GradientPaint::new(Gradient { kind: GradientKind::Radial, ..Gradient::default() });
    let mut n = rect(&mut d, (0.0, 0.0, 100.0, 100.0), 0.0);
    n.appearance = Appearance::basic(Paint::Gradient(Box::new(radial)), Paint::None, 0.0).into();
    let id = add(&mut d, n);
    let bg = one(&d, id, &CssOptions::default()).props.into_iter().find(|(k, _)| *k == "background-image").unwrap().1;
    assert!(bg.starts_with("radial-gradient(circle ") && bg.contains(" at 50px 50px, #ffffff 0%, #000000 100%)"), "{bg}");
}

#[test]
fn shadows_and_unsupported_art() {
    let mut d = doc();
    let mut n = rect(&mut d, (0.0, 0.0, 50.0, 50.0), 0.0);
    n.appearance.effects.push(Effect {
        id: "stylize.dropShadow".into(),
        params: serde_json::json!({"x": 2, "y": 3, "blur": 4, "opacity": 50}),
        visible: true,
    });
    let id = add(&mut d, n);
    let r = one(&d, id, &CssOptions::default());
    assert_eq!(prop(&r, "box-shadow"), Some("2px 3px 4px rgba(0, 0, 0, 0.5)"));
    // A shadow with Darkness takes the object's own colors, which a CSS shadow can't.
    let mut n = rect(&mut d, (0.0, 0.0, 50.0, 50.0), 0.0);
    n.appearance.effects.push(Effect {
        id: "stylize.dropShadow".into(),
        params: serde_json::json!({"x": 2, "y": 3, "blur": 4, "opacity": 50, "shade": "darkness"}),
        visible: true,
    });
    let shaded = add(&mut d, n);
    let r = one(&d, shaded, &CssOptions::default());
    assert!(prop(&r, "box-shadow").is_none() && r.unsupported.is_some(), "{r:?}");
    // A star is no box: described as its box, or with rasterize as its picture.
    let star = Node::path(
        d.alloc_id(),
        shapes::polygon(Point::new(50.0, 50.0), 30.0, 5, 0.0),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    let star = add(&mut d, star);
    let boxed = css_rules(&d, &[star], &CssOptions::default());
    let r = &boxed.rules[0];
    assert!(r.unsupported.is_some() && r.image.is_none() && prop(r, "background-color") == Some("#000000"));
    assert!(!boxed.warnings.is_empty());
    let r = one(&d, star, &CssOptions { rasterize: true, ..CssOptions::default() });
    assert_eq!(r.image.as_deref(), Some("path.png"));
    assert_eq!(prop(&r, "background-image"), Some("url(path.png)"));
    assert!(prop(&r, "background-color").is_none());
}

#[test]
fn groups_stand_for_their_objects_and_hidden_ones_are_left_out() {
    let mut d = doc();
    let a = rect(&mut d, (0.0, 0.0, 10.0, 10.0), 0.0);
    let mut b = rect(&mut d, (20.0, 0.0, 10.0, 10.0), 0.0);
    b.visible = false;
    let g = Node::group(d.alloc_id(), vec![std::sync::Arc::new(a), std::sync::Arc::new(b)]);
    let g = add(&mut d, g);
    let sheet = css_rules(&d, &[g, d.layers[0].id], &CssOptions::default());
    assert_eq!(sheet.rules.len(), 1, "one visible rectangle, once: {sheet:?}");
    assert_eq!(sheet.text(), sheet.rules[0].text());
}

#[test]
fn font_family_ends_with_a_generic_family_for_viewers_without_the_font() {
    let family = |name: &str| {
        let mut d = doc();
        let st = CharStyle { font_family: name.into(), size: 18.0, ..CharStyle::default() };
        let n = text(&mut d, "Hello", st);
        let id = add(&mut d, n);
        prop(&one(&d, id, &CssOptions::default()), "font-family").map(str::to_string)
    };
    // Source Sans 3 is a sans face; an unknown family gets the same default; a generic family stands alone.
    assert_eq!(family("Source Sans 3").as_deref(), Some("'Source Sans 3', sans-serif"));
    assert_eq!(family("No Such Font").as_deref(), Some("'No Such Font', sans-serif"));
    assert_eq!(family("serif").as_deref(), Some("serif"));
}

#[test]
fn font_family_with_quote_or_backslash_is_escaped() {
    for (family, want) in
        [("Bob's Font", r"'Bob\'s Font', sans-serif"), (r"Back\slash Font", r"'Back\\slash Font', sans-serif"), ("Bob's", r"'Bob\'s', sans-serif")]
    {
        let mut d = doc();
        let st = CharStyle { font_family: family.into(), size: 18.0, ..CharStyle::default() };
        let n = text(&mut d, "Hello", st);
        let id = add(&mut d, n);
        let r = one(&d, id, &CssOptions::default());
        assert_eq!(prop(&r, "font-family"), Some(want));
    }
}
