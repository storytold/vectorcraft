//! SVG Options across each other: every styling, id mode and tspan mode reads back, and linked
//! images are named after their bytes (and are formats browsers show).

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, CharStyle, Document, ImageBlob, ImageObject, Node, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_svg::{ExportOptions, ImageMode, ObjectIds, Styling, export, export_full, import};

/// A red rectangle with an awkward name, point type and area type of two paragraphs each.
fn doc() -> Document {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    let red = Appearance::basic(Paint::solid(Color::rgb8(255, 0, 0)), Paint::solid(Color::BLACK), 2.0);
    let mut a = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 60.0)), red);
    a.name = Some("A \"quoted\" & <odd> 100%".into());
    d.insert(Some(l), usize::MAX, a).unwrap();
    let st = CharStyle { size: 12.0, font_family: "Source Sans 3".into(), fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
    let mut t = TextObject::point(Point::new(20.0, 120.0), "", st.clone());
    t.runs = vec![TextRun { text: "one two\nthree".into(), style: st.clone(), inline: None }];
    let i = d.alloc_id();
    d.insert(Some(l), usize::MAX, Node::new(i, NodeKind::Text(Box::new(t.clone())))).unwrap();
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 60.0)) };
    t.xf = Affine::translate((20.0, 140.0));
    t.runs = vec![TextRun { text: "para one\npara two".into(), style: st, inline: None }];
    let i = d.alloc_id();
    d.insert(Some(l), usize::MAX, Node::new(i, NodeKind::Text(Box::new(t)))).unwrap();
    d
}

/// A large palette should create one class per distinct declaration, preserving first-use
/// numbers even when earlier styles recur after hundreds of others.
#[test]
fn many_css_styles_preserve_first_use_numbers() {
    let mut d = Document::new(300.0, 10.0);
    let layer = d.layers[0].id;
    let colors = (0u16..256).chain([0, 128, 255]);
    for (x, color) in colors.enumerate() {
        let x = x as f64;
        let appearance = Appearance::basic(Paint::solid(Color::rgb8(color as u8, 31, 63)), Paint::None, 0.0);
        let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 0.0, x + 0.5, 1.0)), appearance);
        d.insert(Some(layer), usize::MAX, n).unwrap();
    }

    for (styling, definition_marker) in [(Styling::InternalCss, ".cls-"), (Styling::StyleEntities, "<!ENTITY st")] {
        let svg = export(&d, &ExportOptions { styling, ..Default::default() });
        let paths: Vec<_> = svg.lines().filter(|line| line.contains("<rect ")).collect();
        assert_eq!(paths.len(), 259, "{styling:?}");
        for (i, path) in paths.iter().enumerate() {
            let number = if i < 256 { i + 1 } else { [1, 129, 256][i - 256] };
            let expected = if styling == Styling::InternalCss { format!("class=\"cls-{number}\"") } else { format!("style=\"&st{number};\"") };
            assert!(path.contains(&expected), "path {i} should reference {expected}: {path}");
        }
        assert_eq!(svg.matches(definition_marker).count(), 256, "{styling:?}");
    }
}

#[test]
fn every_combination_reads_back() {
    let d = doc();
    for styling in [Styling::PresentationAttributes, Styling::InlineStyle, Styling::StyleEntities, Styling::InternalCss] {
        for object_ids in [ObjectIds::LayerNames, ObjectIds::Minimal, ObjectIds::Unique] {
            for (fewer_tspans, minify) in [(false, false), (true, true)] {
                let o = ExportOptions { styling, object_ids, fewer_tspans, minify, ..Default::default() };
                let s = export(&d, &o);
                let back = import(&s).unwrap_or_else(|e| panic!("{o:?}: {e}\n{s}"));
                let mut red = false;
                back.walk(|n| {
                    red |= n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.paint == Paint::solid(Color::rgb8(255, 0, 0))));
                });
                assert!(red, "{o:?}: the fill came back\n{s}");
                // Paragraph breaks are new lines, never characters in a line.
                assert!(!s.contains("\n</tspan>") && !s.contains("two\n"), "{o:?}\n{s}");
            }
        }
    }
}

/// A document with one embedded image under `key`.
fn image_doc(key: &str, bytes: Vec<u8>) -> Document {
    let mut d = Document::new(100.0, 100.0);
    d.images.insert(key.into(), ImageBlob::new("image/png", bytes));
    let l = d.layers[0].id;
    let im = ImageObject { key: key.into(), width: 2, height: 2, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    d.insert(Some(l), usize::MAX, n).unwrap();
    d
}

#[test]
fn linked_images_are_named_after_their_bytes() {
    let link = ExportOptions { images: ImageMode::Link, ..Default::default() };
    // Keys such as "raster-1" repeat across documents: two exports into one folder must not clash.
    let a = export_full(&image_doc("raster-1", vec![1, 2, 3]), &link, None);
    let b = export_full(&image_doc("raster-1", vec![4, 5, 6]), &link, None);
    assert_eq!((a.linked.len(), b.linked.len()), (1, 1));
    assert_ne!(a.linked[0].name, b.linked[0].name);
    assert!(a.linked[0].name.ends_with(".png") && a.svg.contains(&format!("href=\"{}\"", a.linked[0].name)), "{}", a.svg);
    // The same bytes under another key are the same file.
    assert_eq!(export_full(&image_doc("other", vec![1, 2, 3]), &link, None).linked[0].name, a.linked[0].name);
}

#[test]
fn cmyk_tiffs_are_written_as_png() {
    let inks = vectorcraft_doc::cmyk::Inks::new(2, 2, [0u8, 0, 0, 0].repeat(4)).unwrap();
    let mut d = image_doc("cmyk", vec![]);
    d.images.insert("cmyk".into(), ImageBlob::cmyk_tiff(&inks).unwrap());
    // Browsers don't show TIFF: embedded and linked images are PNG.
    assert!(export(&d, &ExportOptions::default()).contains("href=\"data:image/png;base64,"));
    let linked = export_full(&d, &ExportOptions { images: ImageMode::Link, ..Default::default() }, None);
    assert!(linked.linked[0].name.ends_with(".png"), "{}", linked.linked[0].name);
}
