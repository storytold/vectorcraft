//! Font weights in SVG and CSS: every style is written with its numeric weight (Semibold 600,
//! Light 300, Black 900…; Bold as `bold`), and SVG import reads the numbers back to the same face.

use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, TextObject};
use vectorcraft_geom::Point;
use vectorcraft_svg::{CssOptions, ExportOptions, css_rules, export, import};
use vectorcraft_text::FontDb;

/// A document with one text in `family` `style`.
fn doc(family: &str, style: &str) -> (Document, vectorcraft_doc::NodeId) {
    let mut d = Document::new(300.0, 100.0);
    let st = CharStyle { font_family: family.into(), font_style: style.into(), size: 24.0, ..CharStyle::default() };
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 50.0), "Weight", st))));
    let (l, id) = (d.layers[0].id, n.id);
    d.insert(Some(l), 0, n).unwrap();
    (d, id)
}

/// The `font-weight` the SVG export and the CSS of the text write.
fn weights(style: &str) -> (Option<String>, Option<String>) {
    let (d, id) = doc("Source Sans 3", style);
    let svg = export(&d, &ExportOptions::default());
    let attr = svg.split("font-weight=\"").nth(1).map(|s| s.split('"').next().unwrap().to_string());
    let sheet = css_rules(&d, &[id], &CssOptions::default());
    let css = sheet.rules[0].props.iter().find(|(k, _)| *k == "font-weight").map(|(_, v)| v.clone());
    (attr, css)
}

fn style_back(family: &str, style: &str) -> String {
    let svg = export(&doc(family, style).0, &ExportOptions::default());
    let mut found = None;
    import(&svg).unwrap().walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            found = Some(t.runs[0].style.font_style.clone());
        }
    });
    found.expect("the text comes back")
}

#[test]
fn every_style_is_written_with_its_numeric_weight() {
    for (style, want) in [
        ("Thin", Some("100")),
        ("ExtraLight", Some("200")),
        ("Light", Some("300")),
        ("Regular", None),
        ("Italic", None),
        ("Medium", Some("500")),
        ("Semibold", Some("600")),
        ("Demibold", Some("600")),
        ("Bold", Some("bold")),
        ("Bold Italic", Some("bold")),
        ("Extrabold", Some("800")),
        ("Black", Some("900")),
        ("Heavy", Some("900")),
        // Japanese faces' numbered weights (#2873); W4 is the regular weight.
        ("W0", Some("100")),
        ("W3", Some("300")),
        ("W4", None),
        ("W6", Some("600")),
        ("W9", Some("900")),
        ("Wide", None),
    ] {
        let want = want.map(str::to_string);
        assert_eq!(weights(style), (want.clone(), want), "{style}");
    }
}

#[test]
fn numeric_weights_read_back_to_the_style() {
    let db = FontDb::global();
    for (family, style) in
        [("Source Sans 3", "Semibold"), ("Source Sans 3", "Bold"), ("Source Sans 3", "Italic"), ("Inter", "Medium"), ("Inter", "SemiBold")]
    {
        let back = style_back(family, style);
        let face = |s: &str| db.face(family, s).map(|f| f.style.clone());
        assert_eq!(face(&back), face(style), "{family} {style} came back as {back}");
    }
    // Styles the family doesn't have keep their weight.
    assert_eq!(style_back("Source Sans 3", "Light"), "Light");
    assert_eq!(style_back("Source Sans 3", "Black Italic"), "Black Italic");
    assert_eq!(style_back("Source Sans 3", "Heavy"), "Black");
}
