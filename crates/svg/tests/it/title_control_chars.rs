use vectorcraft_doc::Document;
use vectorcraft_svg::{ExportOptions, export, import};

#[test]
fn title_and_description_control_chars_are_filtered() {
    let mut d = Document::new(200.0, 200.0);
    d.title = "Poster\u{b}v2".into();
    d.metadata.description = "a\u{1}b".into();
    let svg = export(&d, &ExportOptions::default());
    assert!(import(&svg).is_ok(), "export wrote invalid XML: {svg}");
    assert!(svg.contains("<title>Poster v2</title>"));
    assert!(svg.contains("<desc>ab</desc>"));
}

#[test]
fn title_and_description_markup_is_escaped() {
    let mut d = Document::new(200.0, 200.0);
    d.title = "A & <B>".into();
    d.metadata.description = "x & y".into();
    let svg = export(&d, &ExportOptions::default());
    assert!(import(&svg).is_ok(), "export wrote invalid XML: {svg}");
    assert!(svg.contains("<title>A &amp; &lt;B&gt;</title>"));
    assert!(svg.contains("<desc>x &amp; y</desc>"));
}

/// #3544: an artboard exported on its own has its own name as `<title>` (escaped); the document
/// (Save, or no artboard of its own) keeps the document's title, and so does an unnamed artboard.
#[test]
fn an_artboard_exported_alone_is_titled_after_it() {
    let mut d = Document::new(24.0, 24.0);
    d.title = "Icons".into();
    let mut second = d.artboards[0].clone();
    (second.id, second.name) = (2, "arrow <right> & co".into());
    second.rect = second.rect.with_origin((100.0, 0.0));
    d.artboards[0].name = "arrow-left".into();
    d.artboards.push(second);
    let title = |d: &Document, opts: ExportOptions| {
        let svg = export(d, &opts);
        assert!(import(&svg).is_ok(), "export wrote invalid XML: {svg}");
        svg.lines().find(|l| l.contains("<title>")).map(|l| l.trim().to_string())
    };
    let alone = |artboard| ExportOptions { artboard, artboard_title: true, ..ExportOptions::default() };
    assert_eq!(title(&d, alone(Some(1))).as_deref(), Some("<title>arrow &lt;right&gt; &amp; co</title>"));
    assert_eq!(title(&d, alone(Some(0))).as_deref(), Some("<title>arrow-left</title>"));
    // The document: Save's first artboard, or the bounds of all art.
    assert_eq!(title(&d, ExportOptions::default()).as_deref(), Some("<title>Icons</title>"));
    assert_eq!(title(&d, ExportOptions { artboard: Some(1), ..ExportOptions::default() }).as_deref(), Some("<title>Icons</title>"));
    assert_eq!(title(&d, alone(None)).as_deref(), Some("<title>Icons</title>"));
    // An unnamed artboard falls back to the document's title.
    d.artboards[1].name = " ".into();
    assert_eq!(title(&d, alone(Some(1))).as_deref(), Some("<title>Icons</title>"));
    // So does one whose only characters can't appear in XML.
    d.artboards[1].name = "\u{1}\u{2}".into();
    assert_eq!(title(&d, alone(Some(1))).as_deref(), Some("<title>Icons</title>"));
}
