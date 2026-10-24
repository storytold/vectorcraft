//! Pattern swatches, pattern editing mode and Repeat, driven through `Session::execute`.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::pattern::{PATTERN_EDIT_LAYER, RepeatKind, TileType};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

use super::tests_menucmds::place_undecodable_png;
use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

/// A red 10×10 square made into pattern "Dots" (editing finished).
fn make_dots(s: &mut Session) -> NodeId {
    let r = rect(s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [r.0], "none": true})).unwrap();
    sel(s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    r
}

fn close(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

// ---------- patterns ----------

#[test]
fn make_creates_definition_swatch_and_edit_mode() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.pattern.make", &json!({})).unwrap();
    assert_eq!(v["name"], "New Pattern 1");
    let d = doc(&s);
    let def = d.pattern("New Pattern 1").unwrap();
    assert_eq!(def.tile, Rect::new(0.0, 0.0, 10.0, 10.0));
    assert!(d.swatch("New Pattern 1").is_some_and(|sw| matches!(sw.paint, Paint::Pattern { .. })));
    let pe = d.pattern_edit.as_ref().unwrap();
    assert_eq!(d.node(pe.layer).unwrap().name.as_deref(), Some(PATTERN_EDIT_LAYER));
    assert_eq!(d.node(pe.layer).unwrap().children().unwrap().len(), 1);
    // The original art stays in the document; new art goes into the edit layer.
    assert!(d.node(r).is_some());
    assert_eq!(s.doc().unwrap().insertion_parent(), Some(pe.layer));
}

#[test]
fn done_saves_edited_art_and_leaves_mode() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    // Draw another shape inside the pattern.
    rect(&mut s, 2.0, 2.0, 3.0, 3.0);
    s.execute("object.pattern.done", &json!({})).unwrap();
    let d = doc(&s);
    assert!(d.pattern_edit.is_none());
    assert_eq!(d.pattern("P").unwrap().art.len(), 2);
    assert!(d.layers.iter().all(|l| l.name.as_deref() != Some(PATTERN_EDIT_LAYER)));
    assert!(s.doc().unwrap().isolation.is_none());
}

#[test]
fn cancel_new_pattern_removes_it() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    assert!(doc(&s).pattern("P").is_none());
    assert!(doc(&s).swatch("P").is_none());
    assert_eq!(doc(&s).layers.len(), 1);
}

#[test]
fn edit_then_cancel_restores_definition() {
    let mut s = session();
    make_dots(&mut s);
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    s.execute("pattern.options", &json!({"tileType": "hexByRow", "width": 40})).unwrap();
    assert_eq!(doc(&s).pattern("Dots").unwrap().tile_type, TileType::HexByRow);
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    let def = doc(&s).pattern("Dots").unwrap();
    assert_eq!(def.tile_type, TileType::Grid);
    assert_eq!(def.tile.width(), 20.0);
}

#[test]
fn edit_defaults_to_selected_objects_pattern() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    sel(&mut s, &[big]);
    let v = s.execute("object.pattern.edit", &json!({})).unwrap();
    assert_eq!(v["name"], "Dots");
    assert!(doc(&s).pattern_edit.is_some());
    // Edit again while editing is refused.
    assert!(s.execute("object.pattern.edit", &json!({"name": "Dots"})).is_err());
}

#[test]
fn set_fill_with_pattern_swatch() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    assert!(matches!(node(&s, big).appearance.fill_paint(), Paint::Pattern { pattern, .. } if pattern == "Dots"));
}

#[test]
fn set_fill_with_pattern_name_without_swatch() {
    let mut s = session();
    make_dots(&mut s);
    Arc::make_mut(&mut s.doc_mut().unwrap().doc).swatches.retain(|sw| sw.name != "Dots");
    let big = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    assert!(matches!(node(&s, big).appearance.fill_paint(), Paint::Pattern { .. }));
}

#[test]
fn options_set_tile_type_spacing_and_overlap() {
    let mut s = session();
    make_dots(&mut s);
    s.execute(
        "pattern.options",
        &json!({"name": "Dots", "tileType": "brickByColumn", "brickOffset": 0.25, "overlap": {"h": "right", "v": "bottom"}, "copies": 7, "dimCopies": 40}),
    )
    .unwrap();
    let def = doc(&s).pattern("Dots").unwrap();
    assert_eq!(def.tile_type, TileType::BrickByColumn { offset: 0.25 });
    assert!(def.overlap.right_in_front && def.overlap.bottom_in_front);
    assert_eq!(def.copies, 7);
    assert_eq!(def.dim_copies, 40.0);
    // Size tile to art with spacing.
    s.execute("pattern.options", &json!({"name": "Dots", "sizeTileToArt": true, "hSpacing": 4, "vSpacing": 6})).unwrap();
    let def = doc(&s).pattern("Dots").unwrap();
    assert!((def.tile.width() - 14.0).abs() < 1e-6 && (def.tile.height() - 16.0).abs() < 1e-6, "{:?}", def.tile);
    assert!(s.execute("pattern.options", &json!({"name": "Dots", "tileType": "zigzag"})).is_err());
    assert!(s.execute("pattern.options", &json!({"name": "Dots", "width": -1})).is_err());
}

#[test]
fn options_rename_updates_swatch_and_paints() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.execute("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    assert!(doc(&s).pattern("Spots").is_some() && doc(&s).pattern("Dots").is_none());
    assert!(doc(&s).swatch("Spots").is_some());
    assert!(matches!(node(&s, big).appearance.fill_paint(), Paint::Pattern { pattern, .. } if pattern == "Spots"));
}

/// Pattern Options refuses a new name that a swatch of another kind, a second swatch of the
/// pattern or a color group has, and takes a name that only the pattern's own swatch has.
#[test]
fn options_rename_refuses_a_name_another_swatch_has() {
    let mut s = session();
    make_dots(&mut s);
    s.execute("swatch.new", &json!({"name": "Brick", "color": "#aa3300"})).unwrap();
    s.execute("swatch.newGroup", &json!({"name": "Mine", "colors": ["#0000ff"]})).unwrap();
    // "Dots copy": a second swatch of the pattern.
    s.execute("swatch.duplicate", &json!({"name": "Dots"})).unwrap();
    for taken in [" Brick ", "Mine", "Dots copy", "[Registration]"] {
        let e = s.execute("pattern.options", &json!({"name": "Dots", "newName": taken})).unwrap_err().to_string();
        assert!(e.contains(&format!("a swatch or color group named `{}` exists", taken.trim())), "{e}");
    }
    assert!(doc(&s).pattern("Dots").is_some());
    let named = |s: &Session, n: &str| doc(s).swatches_iter().filter(|w| w.name == n).count();
    assert_eq!((named(&s, "Brick"), named(&s, "Dots"), named(&s, "Dots copy")), (1, 1, 1));
    // The pattern's own swatch, renamed in Swatch Options: the pattern can take its name.
    s.execute("swatch.delete", &json!({"name": "Dots copy"})).unwrap();
    s.execute("swatch.edit", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    assert_eq!(s.execute("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap()["name"], "Spots");
    assert_eq!(named(&s, "Spots"), 1);
    assert!(matches!(&doc(&s).swatch("Spots").unwrap().paint, Paint::Pattern { pattern, .. } if pattern == "Spots"));
    // With no swatch of the pattern under its name, a color group's name is still refused.
    s.execute("swatch.edit", &json!({"name": "Spots", "newName": "Spot Tiles"})).unwrap();
    assert!(s.execute("pattern.options", &json!({"name": "Spots", "newName": "Mine"})).is_err());
}

/// Renaming a pattern renames only its own swatches: a color swatch that has the pattern's old
/// name keeps it, and art linked to that swatch stays linked.
#[test]
fn options_rename_leaves_other_swatches_of_the_old_name() {
    let mut s = session();
    make_dots(&mut s);
    s.execute("swatch.edit", &json!({"name": "Dots", "newName": "Dot Tiles"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Dots", "color": "#3366cc", "global": true})).unwrap();
    let r = rect(&mut s, 100.0, 100.0, 20.0, 20.0);
    s.execute("paint.setFill", &json!({"ids": [r.0], "swatch": "Dots"})).unwrap();
    s.execute("pattern.options", &json!({"name": "Dots", "newName": "Spots"})).unwrap();
    assert!(doc(&s).swatch("Dots").is_some_and(|w| matches!(w.paint, Paint::Solid { .. })), "the color swatch keeps its name");
    assert!(matches!(&doc(&s).swatch("Dot Tiles").unwrap().paint, Paint::Pattern { pattern, .. } if pattern == "Spots"));
    assert!(matches!(node(&s, r).appearance.fill_paint(), Paint::Solid { swatch: Some(l), .. } if l == "Dots"));
}

#[test]
fn delete_pattern_unpaints_objects() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    let v = s.execute("pattern.delete", &json!({"name": "Dots"})).unwrap();
    assert_eq!(v["unpainted"], 1);
    assert!(doc(&s).pattern("Dots").is_none() && doc(&s).swatch("Dots").is_none());
    assert!(node(&s, big).appearance.fill_paint().is_none());
    assert!(s.execute("pattern.delete", &json!({"name": "Dots"})).is_err());
}

#[test]
fn transform_patterns_only_moves_placement() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    let before = node(&s, big).geometric_bounds();
    s.execute("pattern.transform", &json!({"ids": [big.0], "matrix": [2, 0, 0, 2, 5, 0]})).unwrap();
    let n = node(&s, big);
    assert_eq!(n.geometric_bounds(), before);
    let Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!() };
    assert_eq!(xf.as_coeffs(), [2.0, 0.0, 0.0, 2.0, 5.0, 0.0]);
}

/// #1448, #1460: with Transform Objects off, the transform commands leave the art where it is and
/// move only its pattern tiles (Transform Patterns on), or nothing at all; Transform Again repeats
/// that.
#[test]
fn transforms_without_transform_objects_move_only_the_pattern() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.execute("select.set", &json!({"ids": [big.0]})).unwrap();
    let before = node(&s, big).geometric_bounds();
    let tiles = |s: &Session| {
        let n = node(s, big);
        let Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!() };
        xf.as_coeffs()
    };
    for (cmd, p, want) in [
        ("object.move", json!({"dx": 5, "dy": 0}), [1.0, 0.0, 0.0, 1.0, 5.0, 0.0]),
        ("object.transformAgain", json!({}), [1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
        ("object.reflect", json!({"axis": "vertical", "origin": [0, 0]}), [-1.0, 0.0, 0.0, 1.0, -10.0, 0.0]),
        ("object.transformEach", json!({"moveH": 10, "reference": 0}), [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
    ] {
        // Transform Again repeats the move as it was made, options included.
        if cmd != "object.transformAgain" {
            let mut off = p.clone();
            off["objects"] = json!(false);
            off["patterns"] = json!(false);
            s.execute(cmd, &off).unwrap();
            assert_eq!(node(&s, big).geometric_bounds(), before, "{cmd}: neither transforms");
            off["patterns"] = json!(true);
            s.execute(cmd, &off).unwrap();
        } else {
            s.execute(cmd, &p).unwrap();
        }
        assert_eq!(node(&s, big).geometric_bounds(), before, "{cmd}");
        assert_eq!(tiles(&s), want, "{cmd}");
    }
}

#[test]
fn list_reports_patterns_and_edit_state() {
    let mut s = session();
    make_dots(&mut s);
    let v = s.execute("pattern.list", &json!({})).unwrap();
    assert_eq!(v["patterns"][0]["name"], "Dots");
    assert_eq!(v["patterns"][0]["tileType"], "grid");
    assert_eq!(v["patterns"][0]["width"], 20.0);
    assert!(v["editing"].is_null());
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    assert_eq!(s.execute("pattern.list", &json!({})).unwrap()["editing"]["name"], "Dots");
}

#[test]
fn save_a_copy_while_editing() {
    let mut s = session();
    make_dots(&mut s);
    s.execute("object.pattern.edit", &json!({"name": "Dots"})).unwrap();
    let v = s.execute("object.pattern.saveCopy", &json!({})).unwrap();
    assert_eq!(v["name"], "Dots copy");
    assert!(doc(&s).pattern("Dots copy").is_some());
}

/// Make, Save a Copy and Pattern Options trim the name they are given, as the Pattern Options
/// panel does; the trimmed name is the one compared with the names there are, and a blank name
/// gives the name the command gives when there is none.
#[test]
fn made_copied_and_renamed_patterns_trim_their_names() {
    let mut s = session();
    make_dots(&mut s);
    let make = |s: &mut Session, name: &str| {
        let r = rect(s, 0.0, 0.0, 10.0, 10.0);
        s.execute("object.pattern.make", &json!({"ids": [r.0], "name": name, "edit": false})).unwrap()["name"].clone()
    };
    assert_eq!(make(&mut s, "  Grid \t"), "Grid");
    assert_eq!(make(&mut s, " Dots"), "Dots 2");
    assert_eq!(make(&mut s, ""), "New Pattern 1");
    assert_eq!(make(&mut s, " \t "), "New Pattern 2");
    s.execute("object.pattern.edit", &json!({"name": "Grid"})).unwrap();
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({"name": " Lines\t"})).unwrap()["name"], "Lines");
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({"name": "  "})).unwrap()["name"], "Grid copy");
    s.execute("object.pattern.done", &json!({})).unwrap();
    assert_eq!(s.execute("pattern.options", &json!({"name": "Dots 2", "newName": " Spots "})).unwrap()["name"], "Spots");
    assert_eq!(s.execute("pattern.options", &json!({"name": "Spots", "newName": " "})).unwrap()["name"], "Spots");
    let e = s.execute("pattern.options", &json!({"name": "Spots", "newName": " Lines"})).unwrap_err().to_string();
    assert!(e.contains("a pattern named `Lines` exists"), "{e}");
    // Save a Copy names the copy of a pattern whose saved name has white space around it, as a
    // saved file can hold, from the trimmed name (" Old " gives "Old copy").
    let d = Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    d.patterns.iter_mut().filter(|p| p.name == "Grid copy").for_each(|p| p.name = " Old ".into());
    s.execute("object.pattern.edit", &json!({"name": " Old "})).unwrap();
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({})).unwrap()["name"], "Old copy");
    s.execute("object.pattern.cancel", &json!({})).unwrap();
    let names: Vec<&str> = doc(&s).patterns.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Dots", "Grid", "Spots", "New Pattern 1", "New Pattern 2", "Lines", " Old ", "Old copy"]);
    assert!(doc(&s).swatch("Spots").is_some() && doc(&s).swatch(" Spots ").is_none());
}

/// Make and Save a Copy number a name that a color group has, as they number a swatch's name
/// (swatches and color groups share one namespace).
#[test]
fn made_and_copied_patterns_number_a_color_groups_name() {
    let mut s = session();
    s.execute("swatch.newGroup", &json!({"name": "Mine", "colors": ["#0000ff"]})).unwrap();
    s.execute("swatch.newGroup", &json!({"name": "New Pattern 1", "colors": ["#00ff00"]})).unwrap();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    assert_eq!(s.execute("object.pattern.make", &json!({"ids": [r.0], "name": "Mine", "edit": false})).unwrap()["name"], "Mine 2");
    assert_eq!(s.execute("object.pattern.make", &json!({"ids": [r.0], "edit": false})).unwrap()["name"], "New Pattern 2");
    s.execute("object.pattern.edit", &json!({"name": "Mine 2"})).unwrap();
    assert_eq!(s.execute("object.pattern.saveCopy", &json!({"name": "Mine"})).unwrap()["name"], "Mine 3");
}

#[test]
fn undo_make_pattern() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(doc(&s).pattern("P").is_none() && doc(&s).pattern_edit.is_none());
}

#[test]
fn hit_testing_in_edit_mode_only_reaches_tile_layer() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let other = rect(&mut s, 200.0, 200.0, 50.0, 50.0);
    sel(&mut s, &[r]);
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    let d = doc(&s);
    let opt = vectorcraft_doc::hit::HitOptions::default();
    assert!(vectorcraft_doc::hit::hit_test(d, Point::new(220.0, 220.0), opt).is_none());
    let h = vectorcraft_doc::hit::hit_test(d, Point::new(5.0, 5.0), opt).unwrap();
    assert_ne!(h.leaf, r);
    let _ = other;
}

#[test]
fn save_load_roundtrip_keeps_patterns_and_repeats() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    let r = rect(&mut s, 300.0, 50.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.repeat.radial", &json!({"instances": 6})).unwrap();
    let bytes = vectorcraft_format::save(doc(&s), false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.patterns, doc(&s).patterns);
    assert_eq!(back.node(big).unwrap().appearance.fill_paint(), node(&s, big).appearance.fill_paint());
    assert!(back.layers[0].children().unwrap().iter().any(|n| matches!(n.kind, NodeKind::Repeat(_))));
}

#[test]
fn svg_export_writes_pattern_def() {
    let mut s = session();
    make_dots(&mut s);
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    let svg = vectorcraft_svg::export(doc(&s), &vectorcraft_svg::ExportOptions::default());
    assert!(svg.contains("<pattern id=\"pattern-1\""), "{svg}");
    assert!(svg.contains("patternUnits=\"userSpaceOnUse\""));
    assert!(svg.contains("width=\"20\" height=\"20\""));
    assert!(svg.contains("fill=\"url(#pattern-1)\"") || svg.contains("fill:url(#pattern-1)"), "{svg}");
    // The tile art is inside the pattern.
    let start = svg.find("<pattern").unwrap();
    let end = svg.find("</pattern>").unwrap();
    assert!(svg[start..end].contains("<path"));
}

/// A pattern fill is written as a PDF tiling pattern, which PDF import reads back as a pattern
/// swatch: a rotated hatch over a long bar (#3849) stays one object with one pattern fill, in a few
/// kilobytes, instead of hundreds of tiles.
#[test]
fn pdf_export_writes_pattern_fills_as_tiling_patterns() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="2812" height="300" viewBox="0 0 2812 300">
  <defs>
    <pattern id="hatch" patternUnits="userSpaceOnUse" width="24" height="24" patternTransform="rotate(45)">
      <rect width="12" height="24" fill="#EB003C"/>
      <rect x="12" width="12" height="24" fill="#FFFFFF"/>
    </pattern>
  </defs>
  <rect x="80" y="80" width="2650" height="140" fill="url(#hatch)" stroke="#EB003C" stroke-width="4"/>
</svg>"##;
    let d = vectorcraft_svg::import(svg).unwrap();
    let rep = vectorcraft_pdf::export_with_report(&d, &Default::default()).unwrap();
    assert!(rep.warnings.iter().all(|w| !w.contains("pattern")), "{:?}", rep.warnings);
    assert!(rep.bytes.len() < 8_000, "{} bytes", rep.bytes.len());
    let plain = vectorcraft_pdf::export(&d, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    assert!(String::from_utf8_lossy(&plain).contains("/PatternType 1"));
    let back = vectorcraft_pdf::import(&rep.bytes).unwrap();
    assert_eq!(back.patterns.len(), 1);
    assert_eq!(back.patterns[0].art.len(), 2, "the hatch's two stripes");
    let mut leaves = vec![];
    back.walk(|n| {
        if !n.is_container() {
            leaves.push(n.appearance.fill_paint());
        }
    });
    assert!(leaves.len() <= 2, "{leaves:?}");
    assert!(leaves.iter().any(|p| matches!(p, Paint::Pattern { pattern, .. } if *pattern == back.patterns[0].name)), "{leaves:?}");
}

/// A pattern fill nested more than four patterns deep is written as its tiles, and the export says so.
#[test]
fn pdf_export_writes_patterns_nested_too_deep_as_tiles_with_a_warning() {
    use std::sync::Arc;
    use vectorcraft_doc::pattern::{PatternDef, pattern_paint};
    use vectorcraft_doc::{Appearance, Document};
    let square = |paint: Paint| {
        Node::path(NodeId(0), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(paint, Paint::None, 0.0))
    };
    let mut d = Document::new(100.0, 100.0);
    d.patterns.push(PatternDef::new("P0", vec![Arc::new(square(Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0))))]));
    for i in 1..=4 {
        d.patterns.push(PatternDef::new(&format!("P{i}"), vec![Arc::new(square(pattern_paint(&format!("P{}", i - 1))))]));
    }
    // P3 nests four tiling patterns (P3 to P0); P4 one more, which is written as its tiles.
    for (top, tiles) in [("P3", false), ("P4", true)] {
        let mut d = d.clone();
        let mut n = square(pattern_paint(top));
        n.id = d.alloc_id();
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, n).unwrap();
        let rep = vectorcraft_pdf::export_with_report(&d, &Default::default()).unwrap();
        assert_eq!(rep.warnings.iter().any(|w| w.contains("are written as their tiles")), tiles, "{top}: {:?}", rep.warnings);
    }
}

#[test]
fn every_raster_export_says_when_a_pattern_holds_an_image_that_could_not_be_decoded() {
    // A PNG whose header reads (so it places at its size) but whose pixel data is cut short, made
    // into a pattern and then cleared from the page: only the pattern's tile draws it.
    let mut s = session();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(20, 20, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
    png.truncate(idat + 8);
    let placed = s.execute("file.place", &json!({"name": "cut.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    let placed = NodeId(placed["ids"][0].as_u64().unwrap());
    sel(&mut s, &[placed]);
    s.execute("object.pattern.make", &json!({"name": "Cut", "width": 20, "height": 20})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    sel(&mut s, &[placed]);
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(doc(&s).node(placed).is_none());
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Cut"})).unwrap();
    // The first export draws the pattern's tile, and the later ones draw it from the tile kept.
    for n in 0..3 {
        let r = s.execute("document.export", &json!({"format": "png", "background": "transparent"})).unwrap();
        let warnings = r["warnings"].as_array().cloned().unwrap_or_default();
        assert!(warnings.iter().any(|w| w.as_str().is_some_and(|w| w.contains("couldn't be decoded"))), "export {n}: {r}");
    }
}

#[test]
fn a_raster_export_says_when_a_pattern_tile_draws_an_undecodable_image_from_its_kept_rasterize_image() {
    // A PNG whose pixel data is cut short, with Rasterize, a drop shadow and a blur: the shadow
    // draws the Rasterize image, and the blurred content draws the image kept from that draw.
    let mut s = session();
    let placed = place_undecodable_png(&mut s);
    for (effect, params) in [
        ("rasterize", json!({"background": "white"})),
        ("stylize.dropShadow", json!({"x": 2, "y": 2, "blur": 1})),
        ("blur.gaussian", json!({"radius": 4})),
    ] {
        s.execute("effect.apply", &json!({"effect": effect, "params": params, "ids": [placed.0]})).unwrap();
    }
    sel(&mut s, &[placed]);
    s.execute("object.pattern.make", &json!({"name": "Cut", "width": 80, "height": 40})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    sel(&mut s, &[placed]);
    s.execute("edit.clear", &json!({})).unwrap();
    let big = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Cut"})).unwrap();
    let warns = |s: &mut Session| {
        let r = s.execute("document.export", &json!({"format": "png", "background": "transparent"})).unwrap();
        r["warnings"].as_array().is_some_and(|w| w.iter().any(|w| w.as_str().is_some_and(|w| w.contains("couldn't be decoded"))))
    };
    assert!(warns(&mut s), "the first export");
    // A new overlap draws a new tile of the same art, and the tile renderer draws its shadow and
    // its blurred content from the rasters it kept.
    s.execute("pattern.options", &json!({"name": "Cut", "overlap": {"h": "right"}})).unwrap();
    assert!(warns(&mut s), "the export after Pattern Options");
}

// ---------- repeat ----------

#[test]
fn radial_repeat_wraps_selection() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({"instances": 8, "radius": 50})).unwrap();
    let n = node(&s, id_of(&v));
    let NodeKind::Repeat(spec) = &n.kind else { panic!("{:?}", n.kind) };
    assert_eq!(spec.source.len(), 1);
    let RepeatKind::Radial { instances, radius, center, .. } = spec.kind else { panic!() };
    assert_eq!((instances, radius), (8, 50.0));
    assert!(close(center, Point::new(200.0, 150.0)));
    assert_eq!(spec.transforms(None).len(), 8);
    assert_eq!(n.kind_label(), "Radial Repeat");
}

/// Repeat given `ids` that list an object and a group that holds it (directly, or inside another
/// group) repeats the outer group, which holds the object once, as it does with those objects
/// selected.
#[test]
fn repeat_of_an_object_and_a_group_that_holds_it_repeats_the_group() {
    let nested = || {
        let mut s = session();
        let (a, b) = (rect(&mut s, 195.0, 95.0, 10.0, 10.0), rect(&mut s, 215.0, 95.0, 10.0, 10.0));
        sel(&mut s, &[a, b]);
        let inner = id_of(&s.execute("object.group", &json!({})).unwrap());
        let c = rect(&mut s, 235.0, 95.0, 10.0, 10.0);
        sel(&mut s, &[inner, c]);
        let outer = id_of(&s.execute("object.group", &json!({})).unwrap());
        s.execute("select.none", &json!({})).unwrap();
        (s, [a, inner, outer])
    };
    for (picks, group) in [(&[0, 1][..], 1), (&[1, 0, 2], 2)] {
        let (mut given, ids) = nested();
        let list: Vec<NodeId> = picks.iter().map(|i| ids[*i]).collect();
        let v = given.execute("object.repeat.radial", &json!({ "ids": list.iter().map(|i| i.0).collect::<Vec<_>>() })).unwrap();
        let NodeKind::Repeat(spec) = node(&given, id_of(&v)).kind else { panic!("{v}") };
        assert_eq!(spec.source.iter().map(|n| n.id).collect::<Vec<_>>(), [ids[group]]);
        let (mut selected, _) = nested();
        sel(&mut selected, &list);
        selected.execute("object.repeat.radial", &json!({})).unwrap();
        assert!(doc(&given).layers == doc(&selected).layers, "ids {list:?}");
    }
}

#[test]
fn radial_instance_transforms_rotate_around_center() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({"instances": 4, "radius": 50})).unwrap();
    let NodeKind::Repeat(spec) = node(&s, id_of(&v)).kind else { panic!() };
    let t = spec.transforms(None);
    let c = Point::new(200.0, 100.0);
    assert!(close(t[0] * c, Point::new(200.0, 100.0)));
    assert!(close(t[1] * c, Point::new(250.0, 150.0)));
    assert!(close(t[2] * c, Point::new(200.0, 200.0)));
    assert!(close(t[3] * c, Point::new(150.0, 150.0)));
}

#[test]
fn expand_yields_n_copies() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({"instances": 5})).unwrap();
    let id = id_of(&v);
    s.execute("object.repeat.expand", &json!({})).unwrap();
    let g = node(&s, id);
    assert!(matches!(g.kind, NodeKind::Group { .. }));
    assert_eq!(g.children().unwrap().len(), 5);
    // Every generated node has a real, unique id.
    let mut ids = vec![];
    g.walk(&mut |n| ids.push(n.id));
    assert!(ids.iter().all(|i| i.0 != 0));
    let mut u = ids.clone();
    u.sort();
    u.dedup();
    assert_eq!(u.len(), ids.len());
}

/// #1756: Object → Expand turns radial, grid and mirror repeats into groups of their instances, in
/// one undo step, as Expand Repeat does.
#[test]
fn object_expand_expands_repeats() {
    for (cmd, opts) in [("object.repeat.radial", json!({"instances": 5})), ("object.repeat.grid", json!({})), ("object.repeat.mirror", json!({}))] {
        let mut s = session();
        let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
        sel(&mut s, &[r]);
        let id = id_of(&s.execute(cmd, &opts).unwrap());
        let NodeKind::Repeat(spec) = node(&s, id).kind else { panic!("{cmd}") };
        let n = spec.expand(None).len();
        assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(true), "{cmd}");
        let undo = s.doc().unwrap().history.undo.len();
        s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
        let g = node(&s, id);
        assert!(matches!(g.kind, NodeKind::Group { .. }), "{cmd}");
        assert_eq!(g.children().unwrap().len(), n, "{cmd}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{cmd}");
        assert_eq!(s.doc().unwrap().selection.objects, vec![id], "{cmd}");
        // Nothing left to expand: the instances are plain rectangles.
        assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(false), "{cmd}");
    }
}

/// Expand reaches repeats inside a selected group, and with every option on also outlines the
/// instances' strokes.
#[test]
fn object_expand_expands_repeats_in_groups() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let rep = id_of(&s.execute("object.repeat.radial", &json!({"instances": 4})).unwrap());
    let other = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[rep, other]);
    s.execute("object.group", &json!({})).unwrap();
    s.execute("object.expand", &json!({})).unwrap();
    let mut repeats = 0;
    for l in &doc(&s).layers {
        l.walk(&mut |c| repeats += usize::from(matches!(c.kind, NodeKind::Repeat(_))));
    }
    assert_eq!(repeats, 0);
    let g = node(&s, rep);
    assert_eq!(g.children().unwrap().len(), 4);
    assert!(g.children().unwrap().iter().all(|c| c.appearance.stroke_paint().is_none()));
}

#[test]
fn grid_repeat_and_options() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.grid", &json!({"hSpacing": 5, "vSpacing": 5})).unwrap();
    let id = id_of(&v);
    assert_eq!(node(&s, id).geometric_bounds(), Some(Rect::new(0.0, 0.0, 40.0, 40.0)));
    s.execute("object.repeat.options", &json!({"rows": 2, "cols": 4, "flipCols": true, "gridType": "brickByRow"})).unwrap();
    let NodeKind::Repeat(spec) = node(&s, id).kind else { panic!() };
    let RepeatKind::Grid { rows, cols, flip_cols, grid_type, .. } = spec.kind else { panic!() };
    assert_eq!((rows, cols, flip_cols), (2, 4, true));
    assert_eq!(grid_type, TileType::BrickByRow { offset: 0.5 });
    assert_eq!(spec.expand(None).len(), 8);
}

#[test]
fn mirror_repeat_reflects() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.mirror", &json!({"offset": 5})).unwrap();
    let n = node(&s, id_of(&v));
    let b = n.geometric_bounds().unwrap();
    assert!((b.x1 - 30.0).abs() < 1e-9 && b.x0.abs() < 1e-9, "{b:?}");
    s.execute("object.repeat.options", &json!({"angle": 0})).unwrap();
    let b = node(&s, id_of(&v)).geometric_bounds().unwrap();
    assert!((b.width() - 10.0).abs() < 1e-9, "{b:?}");
}

#[test]
fn switching_repeat_kind_keeps_source() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({})).unwrap();
    let id = id_of(&v);
    let v2 = s.execute("object.repeat.grid", &json!({})).unwrap();
    assert_eq!(id_of(&v2), id);
    let NodeKind::Repeat(spec) = node(&s, id).kind else { panic!() };
    assert!(matches!(spec.kind, RepeatKind::Grid { .. }));
    assert_eq!(spec.source[0].id, r);
}

#[test]
fn release_restores_source() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    s.execute("object.repeat.mirror", &json!({})).unwrap();
    assert!(doc(&s).layers[0].children().unwrap().len() == 1);
    let v = s.execute("object.repeat.release", &json!({})).unwrap();
    assert_eq!(v["ids"], json!([a.0, b.0]));
    assert_eq!(node(&s, a).geometric_bounds(), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
    assert!(s.execute("object.repeat.release", &json!({})).is_err());
}

#[test]
fn repeat_moves_with_transform() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    let v = s.execute("object.repeat.radial", &json!({"instances": 4, "radius": 50})).unwrap();
    let id = id_of(&v);
    let before = node(&s, id).geometric_bounds().unwrap();
    s.execute("object.transform", &json!({"ids": [id.0], "matrix": [1, 0, 0, 1, 10, 20]})).unwrap();
    let after = node(&s, id).geometric_bounds().unwrap();
    assert!(after.x0 > before.x0 && after.y0 > before.y0, "{before:?} {after:?}");
}

#[test]
fn repeat_hit_test_uses_instances() {
    let mut s = session();
    let r = rect(&mut s, 195.0, 95.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    s.execute("object.repeat.radial", &json!({"instances": 4, "radius": 50})).unwrap();
    let d = doc(&s);
    let opt = vectorcraft_doc::hit::HitOptions::default();
    assert!(vectorcraft_doc::hit::hit_test(d, Point::new(250.0, 150.0), opt).is_some());
    // The empty centre of the ring is not the repeat.
    assert!(vectorcraft_doc::hit::hit_test(d, Point::new(200.0, 150.0), opt).is_none());
}

#[test]
fn repeat_commands_validate() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[r]);
    assert!(s.execute("object.repeat.radial", &json!({"instances": 0})).is_err());
    assert!(s.execute("object.repeat.options", &json!({"instances": 4})).is_err(), "no repeat selected");
    s.execute("select.none", &json!({})).ok();
    assert!(s.execute("object.pattern.make", &json!({})).is_err());
}

/// #1970: picking another pattern swatch keeps the transform the fill already had; a fill that was
/// a colour gets the pattern untransformed.
#[test]
fn picking_another_pattern_swatch_keeps_the_pattern_transform() {
    let mut s = session();
    make_dots(&mut s);
    let bars = rect(&mut s, 0.0, 0.0, 20.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [bars.0], "color": "#0000ff"})).unwrap();
    s.execute("object.pattern.make", &json!({"ids": [bars.0], "name": "Bars", "edit": false})).unwrap();
    let big = rect(&mut s, 20.0, 20.0, 200.0, 200.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "color": "#00ff00"})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Bars"})).unwrap();
    let Paint::Pattern { xf, .. } = node(&s, big).appearance.fill_paint() else { panic!() };
    assert_eq!(xf.as_coeffs(), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    s.execute("pattern.transform", &json!({"ids": [big.0], "matrix": [2, 0, 0, 2, 15, 10]})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Bars"})).unwrap();
    let Paint::Pattern { pattern, xf } = node(&s, big).appearance.fill_paint() else { panic!() };
    assert_eq!(pattern, "Bars");
    assert_eq!(xf.as_coeffs(), [2.0, 0.0, 0.0, 2.0, 15.0, 10.0]);
    // The Appearance panel picks the swatch for one fill through `appearance.setItem`.
    let index = node(&s, big).appearance.items.iter().position(|it| matches!(it, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [big.0], "index": index, "swatch": "Dots"})).unwrap();
    let Paint::Pattern { pattern, xf } = node(&s, big).appearance.fill_paint() else { panic!() };
    assert_eq!(pattern, "Dots");
    assert_eq!(xf.as_coeffs(), [2.0, 0.0, 0.0, 2.0, 15.0, 10.0]);
}

#[test]
fn inner_glow_on_a_pattern_fill_stays_inside_the_object_on_the_threaded_renderer() {
    // #2426: the glow's pixmap was drawn with the pattern's paint transform, so it landed
    // scaled at the top left instead of along the object's edges.
    let mut s = session();
    make_dots(&mut s);
    let bg = rect(&mut s, 0.0, 0.0, 400.0, 400.0);
    s.execute("paint.setFill", &json!({"ids": [bg.0], "color": "#223344"})).unwrap();
    let big = rect(&mut s, 100.0, 100.0, 150.0, 150.0);
    s.execute("paint.setFill", &json!({"ids": [big.0], "swatch": "Dots"})).unwrap();
    sel(&mut s, &[big]);
    s.execute("effect.apply", &json!({"effect": "stylize.innerGlow", "params": {"blur": 10}})).unwrap();
    let mut r = vectorcraft_render::Renderer::new();
    r.threads = 2;
    let view = Affine::scale(1.5) * Affine::translate((-20.0, -30.0));
    let out = r.render(&s.doc().unwrap().doc, 400, 400, view, &Default::default());
    let [red, ..] = out.pixel(110, 100);
    assert!(red < 0x30, "glow outside the object: red {red}");
    let [red, ..] = out.pixel(123, 200);
    assert!(red > 0x40, "no glow along the edge: red {red}");
}

fn mirror_axis(s: &Session) -> vectorcraft_tools::mirror::MirrorAxis {
    let st = s.active().unwrap();
    vectorcraft_tools::mirror::MirrorAxis::of(&st.doc, &st.selection, 1.0).unwrap()
}

#[test]
fn mirror_axis_drag_previews_once_and_undo_redo_preserve_source() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 60.0, 80.0);
    let id = id_of(&s.execute("object.repeat.mirror", &json!({})).unwrap());
    let before = node(&s, id);
    let axis = mirror_axis(&s);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let h = super::tests::history(&s);
    for (kind, dx) in [(PointerKind::Down, 0.0), (PointerKind::Drag, 20.0), (PointerKind::Drag, 30.0), (PointerKind::Up, 40.0)] {
        s.pointer(&PointerEvent::new(kind, axis.center.x + dx, axis.center.y), v).unwrap();
        assert!(close(mirror_axis(&s).center, axis.center + vectorcraft_geom::Vec2::new(dx, 0.0)));
    }
    assert_eq!(super::tests::history(&s).0, h.0 + 1);
    let after = node(&s, id);
    let (NodeKind::Repeat(a), NodeKind::Repeat(b)) = (&before.kind, &after.kind) else { panic!() };
    assert_eq!(a.source, b.source, "moving the mirror axis leaves the source artwork in place");
    assert_ne!(a.transforms(None), b.transforms(None), "the reflected copy updates live");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id), before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(node(&s, id), after);
}

#[test]
fn mirror_axis_rotation_snaps_and_escape_restores_document_and_history() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 60.0, 80.0);
    let id = id_of(&s.execute("object.repeat.mirror", &json!({})).unwrap());
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let before = node(&s, id);
    let h = super::tests::history(&s);
    let axis = mirror_axis(&s);
    let end = axis.ends[1];
    let destination = axis.center + vectorcraft_geom::Vec2::new(70.0, 60.0);
    s.pointer(&PointerEvent::new(PointerKind::Down, end.x, end.y), v).unwrap();
    let shift = Mods { shift: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Drag, destination.x, destination.y).with_mods(shift), v).unwrap();
    assert!((mirror_axis(&s).angle - 45.0).abs() < 1e-6);
    assert_eq!(mirror_axis(&s).center, axis.center);
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert!(!s.tool_busy());
    s.pointer(&PointerEvent::new(PointerKind::Up, destination.x, destination.y), v).unwrap();
    assert_eq!(node(&s, id), before);
    assert_eq!(super::tests::history(&s), h);
    // A click is not an edit, and a subsequent real drag still works after cancellation.
    s.pointer(&PointerEvent::new(PointerKind::Down, end.x, end.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, end.x, end.y), v).unwrap();
    assert_eq!(super::tests::history(&s), h);
    s.pointer(&PointerEvent::new(PointerKind::Down, end.x, end.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, destination.x, destination.y).with_mods(shift), v).unwrap();
    assert!((mirror_axis(&s).angle - 45.0).abs() < 1e-6);
    assert_eq!(super::tests::history(&s).0, h.0 + 1);
}

#[test]
fn mirror_axis_hidden_controls_and_other_repeat_kinds_do_not_capture_drags() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    for command in ["object.repeat.mirror", "object.repeat.radial", "object.repeat.grid"] {
        let mut s = session();
        rect(&mut s, 100.0, 100.0, 60.0, 80.0);
        let id = id_of(&s.execute(command, &json!({})).unwrap());
        let before = node(&s, id);
        let st = s.active().unwrap();
        if command != "object.repeat.mirror" {
            assert!(vectorcraft_tools::mirror::MirrorAxis::of(&st.doc, &st.selection, 1.0).is_none());
        }
        let v = ViewInfo { show_bbox: false, ..Default::default() };
        s.select_tool("selection", v).unwrap();
        // Away from every instance: the hidden axis's top handle cannot edit the repeat.
        s.pointer(&PointerEvent::new(PointerKind::Down, 170.0, 76.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, 200.0, 80.0), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 80.0), v).unwrap();
        assert_eq!(node(&s, id), before);
    }
}
