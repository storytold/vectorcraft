//! The document's units and ruler guides in an SVG's `<metadata>` (`guides_and_units`, which Save
//! turns on): they reopen with the file (#4646), where the art is.

use vectorcraft_doc::{Appearance, Artboard, Document, Guide, Node, Unit};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, export, export_full, import};

/// Two artboards, the first away from the origin, in millimetres, with a canvas guide on the
/// layer, an artboard guide of each artboard and a horizontal canvas guide on no layer.
fn doc() -> Document {
    let mut d = Document::new(200.0, 100.0);
    d.units = Unit::Millimeters;
    d.artboards[0].rect = Rect::new(100.0, 50.0, 300.0, 150.0);
    d.artboards.push(Artboard { id: 2, name: "Artboard 2".into(), rect: Rect::new(400.0, 50.0, 500.0, 150.0), ..d.artboards[0].clone() });
    let l = d.layers[0].id;
    let r = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(110.0, 60.0, 150.0, 90.0)), Appearance::default());
    d.insert(Some(l), usize::MAX, r).unwrap();
    d.guides = vec![
        Guide { layer: Some(l), ..Guide::new(true, 130.0) },
        Guide { artboard: Some(1), ..Guide::new(false, 70.0) },
        Guide { artboard: Some(2), ..Guide::new(true, 450.0) },
        Guide::new(false, 120.0),
    ];
    d
}

fn on() -> ExportOptions {
    ExportOptions { guides_and_units: true, ..ExportOptions::default() }
}

#[test]
fn units_and_guides_reopen_where_the_art_is() {
    let d = doc();
    let back = import(&export(&d, &on())).unwrap();
    assert_eq!(back.units, Unit::Millimeters);
    // The file's user space starts at the artboard's corner, as the art does.
    let layer = back.layers[0].id;
    let board = back.artboards[0].id;
    let guides: Vec<_> = back.guides.iter().map(|g| (g.vertical, (g.pos * 1000.0).round() / 1000.0, g.artboard, g.layer)).collect();
    assert_eq!(
        guides,
        vec![(true, 30.0, None, Some(layer)), (false, 20.0, Some(board), None), (false, 70.0, None, None)],
        "the second artboard's guide stays with its artboard, which the file doesn't hold"
    );
}

#[test]
fn off_by_default_and_with_the_native_document() {
    let d = doc();
    assert!(!export(&d, &ExportOptions::default()).contains("vectorcraft:settings"));
    let opts = ExportOptions { preserve_editing: true, ..on() };
    let svg = export_full(&d, &opts, Some(b"native")).svg;
    assert!(!svg.contains("vectorcraft:settings") && svg.contains("vectorcraft:document"), "{svg}");
}

#[test]
fn nothing_to_keep_writes_nothing() {
    let mut d = Document::new(100.0, 100.0);
    d.units = Unit::Pixels;
    assert!(!export(&d, &on()).contains("vectorcraft:settings"));
    // Other units with no guides are kept alone.
    d.units = Unit::Inches;
    let svg = export(&d, &on());
    assert!(svg.contains("<vectorcraft:settings") && !svg.contains("vectorcraft:guide"), "{svg}");
    assert_eq!(import(&svg).unwrap().units, Unit::Inches);
}

#[test]
fn damaged_settings_are_skipped() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100">
  <metadata>
    <vc:settings xmlns:vc="urn:vectorcraft:editing" units="parsecs">
      <vc:guide orientation="vertical" position="NaN"/>
      <vc:guide orientation="diagonal" position="5"/>
      <vc:guide orientation="horizontal" position="1e400"/>
      <vc:guide orientation="horizontal"/>
      <vc:guide orientation="vertical" position="12" layer="Nowhere" artboard="true"/>
    </vc:settings>
  </metadata>
  <rect width="10" height="10"/>
</svg>"#;
    let d = import(svg).unwrap();
    // Unknown units keep the root's (pixels); only the one readable guide comes in.
    assert_eq!(d.units, Unit::Pixels);
    let guides: Vec<_> = d.guides.iter().map(|g| (g.vertical, g.pos, g.artboard, g.layer)).collect();
    assert_eq!(guides, vec![(true, 12.0, Some(d.artboards[0].id), None)]);
}
