//! SVG import of clip paths made of text (CSS Masking Module Level 1 §6.1,
//! https://www.w3.org/TR/css-masking-1/#ClipPathElement: `<text>` and `<use>` are clip path
//! content).

use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, Node, NodeKind, TextKind};
use vectorcraft_geom::kurbo::Shape;
use vectorcraft_geom::{Point, Rect};
use vectorcraft_svg::{ExportOptions, export, import, import_with_report};

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="400" height="300">{body}</svg>"#)
}

fn open(body: &str) -> Document {
    import(&svg(body)).unwrap_or_else(|e| panic!("{e}"))
}

/// The top-level objects of the first layer.
fn top(d: &Document) -> &[std::sync::Arc<Node>] {
    d.layers[0].children().unwrap()
}

/// The clip group `n` is, with its clipping path and the art it clips.
fn clip_group(n: &Node) -> (&Node, &[std::sync::Arc<Node>]) {
    let NodeKind::Group { children, clip: true } = &n.kind else { panic!("not a clip group: {:?}", n.kind) };
    let (first, rest) = children.split_first().unwrap();
    (first, rest)
}

/// The bounds of the region a clipping path clips to.
fn clip_bounds(clip: &Node) -> Rect {
    let (bp, _) = vectorcraft_effects::clip_outline(clip).expect("the clipping path clips to something");
    bp.bounding_box()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn a_text_clip_path_clips_by_the_text_as_live_type() {
    let d = open(
        r##"<defs><clipPath id="word"><text x="20" y="150" font-size="100" fill="green" stroke="blue">HI</text></clipPath></defs>
        <rect id="art" width="400" height="300" fill="red" clip-path="url(#word)"/>"##,
    );
    let objects = top(&d);
    assert_eq!(objects.len(), 1, "{objects:?}");
    let (clip, art) = clip_group(&objects[0]);
    assert_eq!(art.len(), 1);
    assert_eq!(art[0].name.as_deref(), Some("art"));
    // The clipping path is the text object with no fill or stroke, as after Make Clipping Mask.
    let NodeKind::Text(t) = &clip.kind else { panic!("the clipping path is {:?}", clip.kind) };
    assert_eq!(t.plain_text(), "HI");
    assert!(t.runs.iter().all(|r| r.style.fill == Paint::None && r.style.stroke == Paint::None));
    assert!(near(t.runs[0].style.size, 100.0, 1e-9), "{}", t.runs[0].style.size);
    assert!(t.xf.translation().to_point().distance(Point::new(20.0, 150.0)) < 1e-9, "{:?}", t.xf);
    let p = clip.clip_paint();
    assert!(p.fill.is_none() && p.stroke.is_none(), "a clip path paints nothing");
    // It clips to the glyphs: inside the line, not the whole rectangle.
    let b = clip_bounds(clip);
    assert!(b.x0 >= 20.0 - 1e-6 && b.x1 < 220.0 && b.y1 <= 150.0 + 1e-6 && b.y0 > 50.0 && b.width() > 40.0, "{b:?}");
}

#[test]
fn text_and_shapes_in_one_clip_path_clip_to_their_union() {
    let d = open(
        r##"<clipPath id="c"><rect x="300" y="10" width="50" height="40"/><text x="20" y="150" font-size="100">O</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let shapes = clip.clip_shapes(None);
    assert_eq!(shapes.len(), 2, "the rectangle and the glyph outlines: {shapes:?}");
    let rect = shapes[0].0.bounding_box();
    assert!(near(rect.x0, 300.0, 1e-6) && near(rect.y1, 50.0, 1e-6), "{rect:?}");
    let glyphs = shapes[1].0.bounding_box();
    assert!(glyphs.x0 >= 20.0 - 1e-6 && glyphs.x1 < 120.0 && glyphs.y1 < 160.0 && glyphs.y0 > 50.0, "{glyphs:?}");
    let b = clip_bounds(clip);
    assert!(near(b.x1, 350.0, 1e-6) && near(b.y0, 10.0, 1e-6), "{b:?}");
}

#[test]
fn a_text_clip_path_keeps_its_units_and_transforms() {
    // The clip path's box units and the text's own transform place the type.
    let d = open(
        r##"<clipPath id="c" clipPathUnits="objectBoundingBox">
          <text transform="translate(0.5 0) scale(0.5 1)" x="0" y="0.8" font-size="0.8">I</text></clipPath>
        <rect x="100" y="100" width="200" height="100" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    assert!(matches!(clip.kind, NodeKind::Text(_)), "{:?}", clip.kind);
    let b = clip_bounds(clip);
    // The box maps x 0.5…1 to 200…300 and y 0…1 to 100…200.
    assert!(b.x0 >= 200.0 - 1e-6 && b.x1 <= 300.0 && b.y0 >= 100.0 && b.y1 <= 180.0 + 1e-6 && b.height() > 30.0, "{b:?}");
    // The clip path's transform moves it, as it moves a shape in it.
    let d = open(
        r##"<clipPath id="c" transform="translate(100 20)"><text x="20" y="150" font-size="100">HI</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("{:?}", clip.kind) };
    assert!(t.xf.translation().to_point().distance(Point::new(120.0, 170.0)) < 1e-9, "{:?}", t.xf);
}

#[test]
fn a_text_clip_path_with_a_clip_rule_or_a_clip_of_its_own_clips_by_its_outlines() {
    // The text's own clip path cuts its glyphs: the clip keeps their part inside the rectangle.
    let d = open(
        r##"<clipPath id="half"><rect width="400" height="120"/></clipPath>
        <clipPath id="c"><text x="20" y="150" font-size="100" clip-path="url(#half)">HI</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    assert!(!matches!(clip.kind, NodeKind::Text(_)), "outlines, as the clip of the text cuts them");
    let b = clip_bounds(clip);
    assert!(b.y1 <= 120.0 + 1e-6 && b.y0 > 50.0 && b.x0 >= 20.0 - 1e-6, "{b:?}");
    // An even-odd text clip keeps its rule on the outlines.
    let d = open(
        r##"<clipPath id="c"><text x="20" y="150" font-size="100" clip-rule="evenodd">O</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let shapes = clip.clip_shapes(None);
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0].1, vectorcraft_geom::FillRule::EvenOdd);
}

#[test]
fn tspans_and_type_on_a_path_clip_too() {
    let d = open(
        r##"<clipPath id="c"><text font-size="60"><tspan x="10" y="80">A</tspan><tspan x="10" y="200">B</tspan></text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("{:?}", clip.kind) };
    assert!(t.plain_text().contains('A') && t.plain_text().contains('B'), "{:?}", t.plain_text());
    let b = clip_bounds(clip);
    assert!(b.y0 < 80.0 && b.y1 > 150.0, "both lines clip: {b:?}");

    let d = open(
        r##"<defs><path id="arc" d="M20 200 Q200 20 380 200"/></defs>
        <clipPath id="c"><text font-size="40"><textPath href="#arc">Around</textPath></text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("{:?}", clip.kind) };
    assert!(matches!(t.kind, TextKind::OnPath { .. }), "{:?}", t.kind);
    assert!(clip_bounds(clip).width() > 50.0);
}

#[test]
fn a_use_of_a_text_in_a_clip_path_clips_where_the_use_places_it() {
    let d = open(
        r##"<defs><text id="t" x="0" y="0" font-size="100">HI</text></defs>
        <clipPath id="c"><use href="#t" x="20" y="150"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("{:?}", clip.kind) };
    assert!(t.xf.translation().to_point().distance(Point::new(20.0, 150.0)) < 1e-9, "{:?}", t.xf);
    // Font-relative positions count in the `<use>`'s font size; an undisplayed `<use>` adds nothing.
    let d = open(
        r##"<defs><text id="t" x="0" y="0" font-size="100">HI</text></defs>
        <clipPath id="c" font-size="40"><use href="#t" x="2em" y="3em"/><use href="#t" x="300" y="50" display="none"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("only the displayed use: {:?}", clip.kind) };
    assert!(t.xf.translation().to_point().distance(Point::new(80.0, 120.0)) < 1e-9, "{:?}", t.xf);
    // The same text drawn as well as used in a clip path.
    let d = open(
        r##"<text id="t" x="0" y="100" font-size="50">HI</text>
        <clipPath id="c"><use xlink:href="#t" transform="translate(200 0)"/><rect width="10" height="10"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let objects = top(&d);
    assert_eq!(objects.len(), 2, "{objects:?}");
    assert!(matches!(objects[0].kind, NodeKind::Text(_)));
    let (clip, _) = clip_group(&objects[1]);
    let shapes = clip.clip_shapes(None);
    assert_eq!(shapes.len(), 2, "{shapes:?}");
    let glyphs = shapes[0].0.bounding_box();
    assert!(glyphs.x0 >= 200.0 - 1e-6 && glyphs.y1 <= 100.0 + 1e-6 && glyphs.y0 > 40.0, "{glyphs:?}");
}

#[test]
fn a_use_of_a_text_in_a_clip_path_passes_its_properties_on_to_the_text() {
    let d = open(
        r##"<defs><text id="t" x="0" y="0">HI</text></defs>
        <clipPath id="c" font-weight="bold"><use href="#t" x="20" y="150" font-size="100"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("{:?}", clip.kind) };
    assert!(near(t.runs[0].style.size, 100.0, 1e-9), "{}", t.runs[0].style.size);
    assert_eq!(t.runs[0].style.font_style, "Bold");
    assert!(clip_bounds(clip).height() > 50.0, "{:?}", clip_bounds(clip));
}

#[test]
fn a_clip_path_on_a_use_of_a_text_is_in_the_use_s_user_space() {
    // The text's transform moves its glyphs to x 200…; the `<use>`'s clip keeps x < 250 of its own
    // user space, which the text's transform doesn't move.
    let d = open(
        r##"<defs><clipPath id="k"><rect width="250" height="300"/></clipPath>
        <text id="t" transform="translate(200 0)" x="0" y="150" font-size="100">HI</text></defs>
        <clipPath id="c"><use href="#t" clip-path="url(#k)"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let b = clip_bounds(clip);
    assert!(b.x0 >= 200.0 - 1e-6 && near(b.x1, 250.0, 1e-6), "{b:?}");
    // Without a clip on the `<use>`, the text's transform still places the type.
    let d = open(
        r##"<defs><text id="t" transform="translate(200 0)" x="0" y="150" font-size="100">HI</text></defs>
        <clipPath id="c"><use href="#t" transform="translate(0 10)"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("{:?}", clip.kind) };
    assert!(t.xf.translation().to_point().distance(Point::new(200.0, 160.0)) < 1e-9, "{:?}", t.xf);
}

#[test]
fn a_css_transform_places_a_text_in_a_clip_path() {
    let d = open(
        r##"<style>.moved { transform: translate(100px, 20px) }</style>
        <clipPath id="c"><text class="moved" x="20" y="150" font-size="100">HI</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("{:?}", clip.kind) };
    assert!(t.xf.translation().to_point().distance(Point::new(120.0, 170.0)) < 1e-9, "{:?}", t.xf);
}

#[test]
fn css_transforms_place_a_use_of_a_text_in_a_clip_path() {
    let d = open(
        r##"<defs><text id="t" style="transform: translate(5px, 0)" x="0" y="0" font-size="100">HI</text></defs>
        <clipPath id="c"><use href="#t" style="transform: translate(20px, 150px)"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let NodeKind::Text(t) = &clip.kind else { panic!("{:?}", clip.kind) };
    assert!(t.xf.translation().to_point().distance(Point::new(25.0, 150.0)) < 1e-9, "{:?}", t.xf);
}

#[test]
fn a_use_refers_to_the_first_element_with_its_id() {
    let d = open(
        r##"<defs><rect id="t" x="300" width="50" height="50"/><text id="t" y="150" font-size="100">HI</text></defs>
        <clipPath id="c"><use href="#t"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    assert!(!matches!(clip.kind, NodeKind::Text(_)), "the rectangle clips: {:?}", clip.kind);
    let b = clip_bounds(clip);
    assert!(near(b.x0, 300.0, 1e-6) && near(b.x1, 350.0, 1e-6) && near(b.y1, 50.0, 1e-6), "{b:?}");
}

#[test]
fn a_text_in_a_clip_path_can_be_shown_by_a_use_elsewhere() {
    let d = open(
        r##"<clipPath id="c"><text id="w" x="20" y="150" font-size="100">HI</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>
        <use href="#w" y="100"/>"##,
    );
    let objects = top(&d);
    assert_eq!(objects.len(), 2, "{objects:?}");
    let (clip, _) = clip_group(&objects[0]);
    assert!(matches!(clip.kind, NodeKind::Text(_)), "{:?}", clip.kind);
    let mut shown = vec![];
    objects[1].walk(&mut |n: &Node| {
        if let NodeKind::Text(t) = &n.kind {
            shown.push(t.plain_text());
        }
    });
    assert_eq!(shown, ["HI"], "the use shows the text: {:?}", objects[1]);
}

#[test]
fn a_use_in_a_clip_path_can_be_shown_by_another_use() {
    let d = open(
        r##"<defs><text id="t" x="0" y="100" font-size="50">HI</text></defs>
        <clipPath id="c"><use id="u" href="#t" x="20"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>
        <use href="#u" y="100"/>"##,
    );
    let objects = top(&d);
    assert_eq!(objects.len(), 2, "{objects:?}");
    let (clip, _) = clip_group(&objects[0]);
    assert!(matches!(clip.kind, NodeKind::Text(_)), "{:?}", clip.kind);
    let mut shown = vec![];
    objects[1].walk(&mut |n: &Node| match &n.kind {
        NodeKind::Text(t) => shown.push(t.plain_text()),
        NodeKind::Path { .. } => shown.push("a path".into()),
        _ => {}
    });
    assert_eq!(shown, ["HI"], "the other use shows the text: {:?}", objects[1]);
    // A rule that styles the `<use>` where it stands still applies to what the other use shows.
    let d = open(
        r##"<style>clipPath > use { transform: translate(40px, 0) }</style>
        <defs><text id="t" x="0" y="100" font-size="50">HI</text></defs>
        <clipPath id="c"><use id="u" xlink:href="#t" x="20"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>
        <use href="#u" y="100"/>"##,
    );
    let objects = top(&d);
    let mut at = vec![];
    objects[1].walk(&mut |n: &Node| {
        if let NodeKind::Text(t) = &n.kind {
            at.push(t.xf.translation().to_point());
        }
    });
    assert!(at.len() == 1 && at[0].distance(Point::new(60.0, 200.0)) < 1e-9, "{at:?}");
}

#[test]
fn style_rules_for_paths_do_not_reach_a_text_in_a_clip_path() {
    let d = open(
        r##"<style>path { visibility: hidden } clipPath path { clip-rule: evenodd }</style>
        <clipPath id="c"><text x="20" y="150" font-size="100">HI</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    assert!(matches!(clip.kind, NodeKind::Text(_)), "the visible, non-zero text: {:?}", clip.kind);
    // The text's own values, and those it inherits, still apply.
    let d = open(
        r##"<style>path { clip-rule: nonzero }</style>
        <clipPath id="c" clip-rule="evenodd"><text x="20" y="150" font-size="100">O</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let shapes = clip.clip_shapes(None);
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0].1, vectorcraft_geom::FillRule::EvenOdd);
}

#[test]
fn a_hidden_text_alone_in_a_clip_path_is_not_live_type() {
    let d = open(
        r##"<clipPath id="c"><text x="20" y="150" font-size="100" visibility="hidden">HI</text></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let objects = top(&d);
    let clips = objects.iter().filter_map(|n| match &n.kind {
        NodeKind::Group { children, clip: true } => children.first(),
        _ => None,
    });
    for clip in clips {
        assert!(!matches!(clip.kind, NodeKind::Text(_)), "a hidden text clips nothing: {:?}", clip.kind);
    }
}

#[test]
fn a_text_left_out_by_its_conditions_clips_nothing() {
    let d = open(
        r##"<clipPath id="c"><text x="20" y="150" font-size="100" systemLanguage="zz">HI</text><rect width="10" height="10"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let b = clip_bounds(clip);
    assert!(near(b.x1, 10.0, 1e-6) && near(b.y1, 10.0, 1e-6), "only the rectangle: {b:?}");
}

#[test]
fn a_text_clip_on_a_symbol_instance_cuts_art_between_the_letters() {
    // The art sits in the space between the two letters, inside the text's box: the glyphs cut it,
    // so it stays clipped art instead of becoming an instance without the clip.
    let body = |clip: &str| {
        format!(
            r##"<defs><symbol id="s" overflow="visible"><rect x="140" y="200" width="4" height="4"/></symbol>
            <clipPath id="c">{clip}</clipPath></defs>
            <use href="#s" clip-path="url(#c)"/>"##
        )
    };
    let instances = |d: &Document| {
        let mut n = 0;
        d.walk(|node| n += usize::from(matches!(node.kind, NodeKind::SymbolInstance { .. })));
        n
    };
    let d = open(&body(r#"<text x="0" y="300" font-size="400">I I</text>"#));
    assert_eq!(instances(&d), 0, "{:?}", top(&d)[0].kind);
    assert!(matches!(top(&d)[0].kind, NodeKind::Group { clip: true, .. }));
    // A clip around the art leaves it whole: an instance.
    let d = open(&body(r#"<rect width="400" height="300"/>"#));
    assert_eq!(instances(&d), 1);
}

#[test]
fn a_clip_path_clipped_by_a_text_clip_path_nests() {
    let d = open(
        r##"<clipPath id="inner"><text x="20" y="150" font-size="100">HI</text></clipPath>
        <clipPath id="outer" clip-path="url(#inner)"><rect width="400" height="120"/></clipPath>
        <rect width="400" height="300" clip-path="url(#outer)"/>"##,
    );
    let (clip, art) = clip_group(&top(&d)[0]);
    assert!(matches!(clip.kind, NodeKind::Text(_)), "the outermost clip is the text: {:?}", clip.kind);
    let (inner, _) = clip_group(&art[0]);
    assert!(matches!(inner.kind, NodeKind::Path { clipping: true, .. }), "{:?}", inner.kind);
}

#[test]
fn hidden_text_in_a_clip_path_clips_nothing() {
    let d = open(
        r##"<clipPath id="c"><text x="20" y="150" font-size="100" visibility="hidden">HI</text><rect width="10" height="10"/></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##,
    );
    let (clip, _) = clip_group(&top(&d)[0]);
    let b = clip_bounds(clip);
    assert!(near(b.x1, 10.0, 1e-6) && near(b.y1, 10.0, 1e-6), "only the rectangle: {b:?}");
}

#[test]
fn text_further_down_in_a_clip_path_is_still_not_clip_path_content() {
    let body = r##"<clipPath id="c"><rect width="10" height="10"/><g><text y="50">No</text></g></clipPath>
        <rect width="400" height="300" clip-path="url(#c)"/>"##;
    let (_, warnings) = import_with_report(&svg(body)).unwrap();
    assert!(warnings.iter().any(|w| w.contains("in a clip path ignored")), "{warnings:?}");
}

#[test]
fn a_text_clip_survives_a_round_trip_through_svg_export() {
    let d = open(
        r##"<clipPath id="c"><text x="20" y="150" font-size="100">HI</text></clipPath>
        <rect width="400" height="300" fill="red" clip-path="url(#c)"/>"##,
    );
    let before = clip_bounds(clip_group(&top(&d)[0]).0);
    let again = import(&export(&d, &ExportOptions::default())).unwrap();
    let mut clips = vec![];
    again.walk(|n| {
        if let NodeKind::Group { children, clip: true } = &n.kind {
            clips.push(children[0].clone());
        }
    });
    assert_eq!(clips.len(), 1);
    let after = clip_bounds(&clips[0]);
    let tol = 0.01 * before.width().max(before.height());
    assert!(
        near(after.x0, before.x0, tol) && near(after.x1, before.x1, tol) && near(after.y0, before.y0, tol) && near(after.y1, before.y1, tol),
        "{before:?} → {after:?}"
    );
}
