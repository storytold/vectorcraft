//! `paint-order="stroke"` on imported type: the characters' stroke becomes the object's own stroke
//! below the Characters row, so it paints under the letters as on shapes.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeKind, StrokeLayer, TextObject};
use vectorcraft_svg::import_with_report;
use vectorcraft_testkit::raster::render_artboard;

const HALO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="80">
  <path d="M10 10 h60 v30 h-60z" fill="#fc0" stroke="#000" stroke-width="10" paint-order="stroke"/>
  <text x="100" y="45" font-size="36" font-weight="bold" fill="#fc0" stroke="#000" stroke-width="8" stroke-linejoin="round"
        paint-order="stroke">Ab</text>
</svg>"##;

fn text(d: &Document) -> (Node, TextObject) {
    let mut found = None;
    d.walk(|n: &Node| {
        if let NodeKind::Text(t) = &n.kind {
            found = Some((n.clone(), (**t).clone()));
        }
    });
    found.expect("a text")
}

fn under(n: &Node) -> &StrokeLayer {
    assert_eq!((n.appearance.items.len(), n.appearance.contents_at()), (1, 1), "one stroke below the characters: {:?}", n.appearance);
    match &n.appearance.items[0] {
        AppearanceItem::Stroke(s) => s,
        i => panic!("{i:?}"),
    }
}

/// Pixels of the letters' yellow left showing (a stroke drawn over them hides them).
fn yellow_letters(d: &Document) -> usize {
    let img = render_artboard(d);
    (100..190).flat_map(|x| (0..80).map(move |y| (x, y))).filter(|&(x, y)| img.over_white(x, y) == [0xff, 0xcc, 0]).count()
}

#[test]
fn a_stroke_first_text_strokes_under_its_characters() {
    let (d, warnings) = import_with_report(HALO).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let (n, t) = text(&d);
    let s = under(&n);
    assert_eq!((s.paint.clone(), s.width, s.join), (Paint::solid(Color::BLACK), 8.0, vectorcraft_doc::LineJoin::Round));
    assert!(t.runs.iter().all(|r| !r.style.has_stroke() && r.style.fill == Paint::solid(Color::rgb8(0xff, 0xcc, 0))), "{:?}", t.runs);

    // The letters stay yellow inside their black halo.
    assert!(yellow_letters(&d) > 150, "{}", yellow_letters(&d));
}

#[test]
fn the_stroke_under_scales_with_the_text_and_keeps_its_gradient() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="200">
      <linearGradient id="g" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>
      <g transform="scale(2)"><text x="10" y="40" font-size="20" fill="#fc0" stroke="url(#g)" stroke-width="3" paint-order="stroke markers">Ab</text></g>
    </svg>"##;
    let (d, warnings) = import_with_report(svg).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let (n, t) = text(&d);
    let s = under(&n);
    assert_eq!(s.width, 6.0);
    let Paint::Gradient(g) = &s.paint else { panic!("{:?}", s.paint) };
    // The bounding-box gradient spans the text's ink in the document (x from about 20).
    let geom = g.geom.unwrap();
    assert!(geom.start.x > 15.0 && geom.end.x > geom.start.x + 40.0, "{geom:?}");
    assert!(t.runs.iter().all(|r| !r.style.has_stroke()));

    // Non-scaling: the stroke under the characters is as wide as on screen.
    let svg = svg.replace("paint-order", r#"vector-effect="non-scaling-stroke" paint-order"#);
    let (d, _) = import_with_report(&svg).unwrap();
    assert_eq!(under(&text(&d).0).width, 3.0);
}

#[test]
fn characters_with_different_strokes_keep_them_and_warn() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="80">
      <text id="mixed" x="10" y="45" font-size="36" fill="#fc0" stroke="#000" stroke-width="8" paint-order="stroke">A<tspan stroke-width="2">b</tspan></text>
    </svg>"##;
    let (d, warnings) = import_with_report(svg).unwrap();
    assert!(warnings.iter().any(|w| w.contains("paint-order on text 'mixed'")), "{warnings:?}");
    let (n, t) = text(&d);
    assert!(n.appearance.items.is_empty());
    assert!(t.runs.iter().all(|r| r.style.has_stroke()));
}

#[test]
fn normal_paint_order_leaves_the_character_strokes() {
    let (d, warnings) = import_with_report(&HALO.replace(r#"paint-order="stroke">Ab"#, ">Ab")).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let (n, t) = text(&d);
    assert!(n.appearance.items.is_empty() && t.runs.iter().all(|r| r.style.has_stroke()));
    assert!(yellow_letters(&d) < 20, "{}", yellow_letters(&d));
}
