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

#[test]
fn dublin_core_metadata_control_chars_are_filtered() {
    let mut d = Document::new(200.0, 200.0);
    d.title = "Poster\u{b}v2".into();
    d.metadata.author = "Ann\u{1}e".into();
    d.metadata.description = "a\u{1}b".into();
    d.metadata.keywords = vec!["k\u{1}w".into()];
    d.metadata.copyright_notice = "(c)\u{b}me\u{2}".into();
    let svg = export(&d, &ExportOptions { metadata: true, ..ExportOptions::default() });
    assert!(import(&svg).is_ok(), "export wrote invalid XML: {svg}");
    assert!(svg.contains("<dc:creator>Anne</dc:creator>"));
    assert!(svg.contains("<dc:title>Poster v2</dc:title>"));
    assert!(svg.contains("<dc:subject>kw</dc:subject>"));
    assert!(svg.contains("<dc:rights>(c) me</dc:rights>"));
}

/// The root's `<title>` is the File Info title of the imported document ([`Document::info_title`]);
/// its name stays what `Document::new` gave it, and a shape's `<title>` is that shape's tooltip.
#[test]
fn the_roots_title_is_the_file_info_title_not_the_documents_name() {
    let svg = |inner: &str| {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20">{inner}<rect width="5" height="5"><title>Tip</title></rect></svg>"#)
    };
    let d = import(&svg("<title>\n Access\n paths </title>")).unwrap();
    assert_eq!((d.title.as_str(), d.metadata.title.as_deref(), d.info_title()), ("Untitled-1", Some("Access paths"), "Access paths"));
    for none in [svg(""), svg("<title>  </title>")] {
        let d = import(&none).unwrap();
        assert_eq!((d.title.as_str(), d.metadata.title.as_deref(), d.info_title()), ("Untitled-1", None, "Untitled-1"));
    }
}

/// Exports write the File Info title, where the document has one, in place of its name.
#[test]
fn exports_write_the_file_info_title() {
    let mut d = Document::new(200.0, 200.0);
    d.title = "box.svg".into();
    d.metadata.title = Some("Access paths".into());
    let svg = export(&d, &ExportOptions { metadata: true, ..ExportOptions::default() });
    assert!(svg.contains("<title>Access paths</title>") && svg.contains("<dc:title>Access paths</dc:title>"), "{svg}");
    assert!(!svg.contains("box.svg"), "{svg}");
}
