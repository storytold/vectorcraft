//! Swatches: colour groups as first-class swatch homes (colour mode, spot plates, PDF, Document
//! Info), naming and kinds, Swatch Options edits reaching linked art, deleting, new swatches in
//! groups and colour groups made from artwork.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn rect(s: &mut Session) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap())
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

/// Add a global solid swatch to colour group `group`.
fn grouped_swatch(s: &mut Session, group: &str, name: &str, color: Color) {
    assert_eq!(run(s, "swatch.new", json!({"name": name, "color": color, "global": true, "group": group}))["name"], name);
}

#[test]
fn document_color_mode_converts_grouped_swatches() {
    let mut s = session();
    grouped_swatch(&mut s, "Grays", "Ink", Color::rgb(0.2, 0.4, 0.6));
    run(&mut s, "object.convertDocumentColorMode", json!({"mode": "cmyk"}));
    for g in &doc(&s).swatch_groups {
        for sw in &g.swatches {
            assert!(!matches!(sw.paint.color(), Some(Color::Rgb { .. })), "{} in {} is still RGB", sw.name, g.name);
        }
    }
    assert!(matches!(doc(&s).swatch("Ink").unwrap().paint.color(), Some(Color::Cmyk { .. })));
    // The File menu's mode switch converts them as well.
    let mut s = session();
    run(&mut s, "file.documentColorMode", json!({"mode": "cmyk"}));
    assert!(matches!(doc(&s).swatch("Bright Red").unwrap().paint.color(), Some(Color::Cmyk { .. })));
}

#[test]
fn grouped_spot_swatches_print_on_their_own_plate() {
    let mut s = session();
    grouped_swatch(&mut s, "Brights", "Signal Orange", Color::cmyk(0.0, 0.6, 1.0, 0.0));
    assert_eq!(run(&mut s, "swatch.setSpot", json!({"name": "Signal Orange"})), json!({"name": "Signal Orange", "spot": true}));
    assert!(doc(&s).swatch("Signal Orange").is_some_and(|w| w.spot && w.global));
    let plates = run(&mut s, "color.plates", json!({}));
    assert!(plates["plates"].as_array().unwrap().iter().any(|p| p["name"] == "Signal Orange" && p["spot"] == true), "{plates}");
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "swatch": "Signal Orange"}));
    let bytes = vectorcraft_pdf::export(doc(&s), &vectorcraft_pdf::PdfOptions { compress: false, created: Some(0), ..Default::default() }).unwrap();
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(pdf.contains("/Separation") && pdf.contains("Signal#20Orange"), "a grouped spot swatch is a Separation");
    let info = run(&mut s, "document.info", json!({}));
    assert_eq!(info["spotColors"], json!(["Signal Orange"]));
}

#[test]
fn document_info_counts_grouped_swatches() {
    let mut s = session();
    let d = doc(&s);
    let all = d.swatches.len() + d.swatch_groups.iter().map(|g| g.swatches.len()).sum::<usize>();
    assert!(all > d.swatches.len());
    assert_eq!(run(&mut s, "document.info", json!({}))["swatches"], json!(all));
}

#[test]
fn new_swatches_are_named_by_their_colour_model() {
    let mut s = session();
    let name = |s: &mut Session, p: Value| run(s, "swatch.new", p)["name"].as_str().unwrap().to_string();
    assert_eq!(name(&mut s, json!({"color": {"c": 10, "m": 20, "y": 30, "k": 0}})), "C=10 M=20 Y=30 K=0");
    assert_eq!(name(&mut s, json!({"color": "#ff8000"})), "R=255 G=128 B=0");
    assert_eq!(name(&mut s, json!({"color": {"gray": 40}})), "Gray K=40");
    // The same colour again (or an explicit name in use) gets the next free number, document-wide.
    assert_eq!(name(&mut s, json!({"color": {"c": 10, "m": 20, "y": 30, "k": 0}})), "C=10 M=20 Y=30 K=0 2");
    assert_eq!(name(&mut s, json!({"name": "Grays", "color": "#000000"})), "Grays 2");
    assert_eq!(name(&mut s, json!({"name": "Bright Red", "color": "#000000"})), "Bright Red 2");
    // A spot swatch is always global; saving a linked colour stores the colour, not the link.
    let spot = name(&mut s, json!({"name": "Ink", "color": "#336699", "spot": true}));
    assert!(doc(&s).swatch(&spot).is_some_and(|w| w.spot && w.global));
    let copy = name(&mut s, json!({"swatch": "Ink"}));
    assert_eq!(doc(&s).swatch(&copy).unwrap().paint, Paint::solid(Color::from_hex("#336699").unwrap()));
    assert!(s.execute("swatch.new", &json!({"none": true})).is_err(), "None is not a swatch");
}

#[test]
fn new_swatch_saves_the_active_pattern_or_gradient() {
    let mut s = session();
    let a = rect(&mut s);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "object.pattern.make", json!({}));
    run(&mut s, "object.pattern.done", json!({}));
    let pattern = doc(&s).patterns[0].name.clone();
    let name = run(&mut s, "swatch.new", json!({"pattern": pattern}))["name"].as_str().unwrap().to_string();
    assert_eq!(name, "New Pattern Swatch 1");
    assert!(matches!(&doc(&s).swatch(&name).unwrap().paint, Paint::Pattern { pattern: p, .. } if *p == pattern));
    assert!(s.execute("swatch.new", &json!({"pattern": "Nope"})).is_err());
    let g = json!({"gradient": {"kind": "radial", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]}});
    let name = run(&mut s, "swatch.new", g)["name"].as_str().unwrap().to_string();
    assert_eq!(name, "New Gradient Swatch 1");
    assert!(matches!(doc(&s).swatch(&name).unwrap().paint, Paint::Gradient(_)));
    assert!(s.execute("swatch.new", &json!({"pattern": pattern, "spot": true})).is_err(), "only colours are spot");
}

#[test]
fn colour_groups_hold_solid_colours_only() {
    let mut s = session();
    let g = run(&mut s, "swatch.newGroup", json!({"name": "Mix", "swatches": ["Sunset", "Red", "[None]", "Bright Blue"], "colors": ["#00ff00"]}));
    let d = doc(&s);
    let group = d.swatch_groups.iter().find(|x| x.name == g["name"]).unwrap();
    let names: Vec<&str> = group.swatches.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Red", "Bright Blue", "R=0 G=255 B=0"]);
    assert!(d.swatches.iter().any(|w| w.name == "Sunset") && d.swatches.iter().any(|w| w.name == "[None]"), "gradients and None stay put");
    assert!(d.swatch_groups.iter().find(|x| x.name == "Brights").unwrap().swatches.iter().all(|w| w.name != "Bright Blue"), "moved, not copied");
}

fn fill_of(s: &Session, id: NodeId) -> Paint {
    doc(s).node(id).unwrap().appearance.fill_paint()
}

fn linked(hex: &str, name: &str) -> Paint {
    Paint::Solid { color: Color::from_hex(hex).unwrap(), swatch: Some(name.into()) }
}

#[test]
fn editing_a_global_swatch_recolours_linked_art_in_one_step() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    let (a, b) = (rect(&mut s), rect(&mut s));
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "swatch": "Brand"}));
    run(&mut s, "paint.setFill", json!({"ids": [b.0], "color": "#2a6fb0"}));
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Hi"}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"ids": [t.0], "swatch": "Brand"}));
    run(&mut s, "select.set", json!({"ids": []}));
    run(&mut s, "paint.setStroke", json!({"swatch": "Brand"}));
    let undo_depth = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "swatch.edit", json!({"name": "Brand", "color": "#ff0000"}));
    assert_eq!(r, json!({"name": "Brand", "relinked": 2}));
    assert_eq!(fill_of(&s, a), linked("#ff0000", "Brand"));
    assert_eq!(fill_of(&s, b), Paint::solid(Color::from_hex("#2a6fb0").unwrap()), "unlinked art keeps its colour");
    let text_fill = |s: &Session| match &doc(s).node(t).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(tx) => tx.runs[0].style.fill.clone(),
        _ => panic!("not a text object"),
    };
    assert_eq!(text_fill(&s), linked("#ff0000", "Brand"), "text runs follow the swatch");
    assert_eq!(s.paint.stroke, linked("#ff0000", "Brand"), "so does the default stroke");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_depth + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fill_of(&s, a), linked("#2a6fb0", "Brand"));
    assert_eq!(doc(&s).swatch("Brand").unwrap().paint.color(), Color::from_hex("#2a6fb0"));
}

#[test]
fn renaming_relinks_and_spot_forces_global() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "swatch": "Brand"}));
    // A name in use gets the next free number.
    let r = run(&mut s, "swatch.edit", json!({"name": "Brand", "newName": "Red"}));
    assert_eq!(r, json!({"name": "Red 2", "relinked": 1}));
    assert_eq!(fill_of(&s, a), linked("#2a6fb0", "Red 2"));
    assert!(doc(&s).swatch("Brand").is_none());
    run(&mut s, "swatch.edit", json!({"name": "Red 2", "spot": true, "global": false}));
    assert!(doc(&s).swatch("Red 2").is_some_and(|w| w.spot && w.global), "spot colours stay global");
    // Turning Global off (no longer spot) unlinks the art, which keeps its colour.
    let r = run(&mut s, "swatch.edit", json!({"name": "Red 2", "spot": false, "global": false, "color": "#000000"}));
    assert_eq!(r["relinked"], 1);
    assert_eq!(fill_of(&s, a), Paint::solid(Color::from_hex("#2a6fb0").unwrap()));
    assert!(doc(&s).swatch("Red 2").is_some_and(|w| !w.global && w.paint.color() == Some(Color::BLACK)));
}

#[test]
fn grouped_swatches_edit_and_convert_modes() {
    let mut s = session();
    let r = run(&mut s, "swatch.edit", json!({"name": "Bright Red", "mode": "cmyk", "global": true}));
    assert_eq!(r["name"], "Bright Red");
    let w = doc(&s).swatch("Bright Red").unwrap();
    assert!(w.global && matches!(w.paint.color(), Some(Color::Cmyk { .. })));
    assert_eq!(doc(&s).swatch_group_of("Bright Red").map(|g| doc(&s).swatch_groups[g].name.clone()), Some("Brights".into()));
    run(&mut s, "swatch.edit", json!({"name": "Bright Red", "color": "#123456", "mode": "web"}));
    assert_eq!(doc(&s).swatch("Bright Red").unwrap().paint.color().unwrap().to_hex(), "#003366");
    run(&mut s, "swatch.edit", json!({"name": "Bright Red", "mode": "gray"}));
    assert!(matches!(doc(&s).swatch("Bright Red").unwrap().paint.color(), Some(Color::Gray { .. })));
    // Gradients can be renamed but have no colour, mode or spot; None can't be edited.
    assert!(s.execute("swatch.edit", &json!({"name": "Sunset", "color": "#000000"})).is_err());
    assert!(s.execute("swatch.edit", &json!({"name": "Sunset", "spot": true})).is_err());
    assert!(s.execute("swatch.edit", &json!({"name": "[None]", "newName": "x"})).is_err());
    assert!(s.execute("swatch.edit", &json!({"name": "Bright Red", "mode": "lab"})).is_err());
    assert_eq!(run(&mut s, "swatch.edit", json!({"name": "Sunset", "newName": "Dusk"}))["name"], "Dusk");
}

#[test]
fn swatch_list_reports_kinds_and_groups() {
    let mut s = session();
    let all = run(&mut s, "swatch.list", json!({}));
    let list = all["swatches"].as_array().unwrap();
    assert_eq!(list.len(), doc(&s).swatches_iter().count());
    let find = |n: &str| list.iter().find(|w| w["name"] == n).unwrap().clone();
    assert_eq!(find("[None]")["kind"], "none");
    assert_eq!(find("Red")["kind"], "color");
    assert_eq!(find("Red")["hex"], "#ed1c24");
    assert_eq!(find("Sunset")["kind"], "gradient");
    assert_eq!(find("K=50")["group"], "Grays");
    let grays = run(&mut s, "swatch.list", json!({"group": "Grays"}));
    assert_eq!(grays["swatches"].as_array().unwrap().len(), 9);
    assert_eq!(grays["groups"][0]["name"], "Grays");
    assert!(s.execute("swatch.list", &json!({"group": "Nope"})).is_err());
}

#[test]
fn deleting_a_global_swatch_unlinks_art_that_keeps_its_colour() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "swatch": "Brand"}));
    run(&mut s, "select.set", json!({"ids": []}));
    run(&mut s, "paint.setFill", json!({"swatch": "Brand"}));
    let before = doc(&s).clone();
    assert_eq!(run(&mut s, "swatch.delete", json!({"name": "Brand"})), json!({"deleted": ["Brand"], "unlinked": 1}));
    assert!(doc(&s).swatch("Brand").is_none());
    assert_eq!(fill_of(&s, a), Paint::solid(Color::from_hex("#2a6fb0").unwrap()));
    assert_eq!(s.paint.fill, Paint::solid(Color::from_hex("#2a6fb0").unwrap()), "the default fill is unlinked too");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*doc(&s), before, "one undo step restores swatch and link");
    // unlink: false keeps the stale link.
    assert_eq!(run(&mut s, "swatch.delete", json!({"name": "Brand", "unlink": false}))["unlinked"], 0);
    assert_eq!(fill_of(&s, a), linked("#2a6fb0", "Brand"));
}

#[test]
fn deleting_several_swatches_and_groups() {
    let mut s = session();
    let r = run(&mut s, "swatch.delete", json!({"names": ["Red", "Brights", "Sunset"]}));
    assert_eq!(r["deleted"], json!(["Red", "Brights", "Sunset"]));
    let d = doc(&s);
    assert!(d.swatch("Red").is_none() && d.swatch("Sunset").is_none() && d.swatch("Bright Red").is_none());
    assert!(d.swatch_groups.iter().all(|g| g.name != "Brights") && d.swatch_groups.iter().any(|g| g.name == "Grays"));
    assert!(s.execute("swatch.delete", &json!({"name": "[None]"})).is_err(), "None stays");
    assert!(s.execute("swatch.delete", &json!({"names": ["Orange", "Nope"]})).is_err(), "unknown names fail the whole call");
    assert!(doc(&s).swatch("Orange").is_some());
    assert!(s.execute("swatch.delete", &json!({})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert!(doc(&s).swatch("Bright Red").is_some() && doc(&s).swatch("Red").is_some());
}

#[test]
fn new_swatches_go_into_groups_convert_modes_and_can_be_spot() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Ink", "color": "#ff0000", "spot": true, "mode": "cmyk", "group": "Brights"}));
    let d = doc(&s);
    let ink = d.swatch("Ink").unwrap();
    assert!(ink.spot && ink.global, "spot colours are global");
    assert!(matches!(ink.paint.color(), Some(Color::Cmyk { .. })), "mode converts the colour");
    assert_eq!(d.swatch_group_of("Ink").map(|g| d.swatch_groups[g].name.as_str()), Some("Brights"));
    assert!(s.execute("swatch.new", &json!({"color": "#00ff00", "group": "Nope"})).is_err());
    assert!(s.execute("swatch.new", &json!({"swatch": "Sunset", "group": "Brights"})).is_err(), "groups hold solid colours only");
    assert!(s.execute("swatch.new", &json!({"color": "#00ff00", "mode": "lab"})).is_err());
}

/// Two rectangles: `a` filled red and stroked green, `b` filled red and stroked with global swatch
/// "Brand"; both selected.
fn art(s: &mut Session) -> (NodeId, NodeId) {
    run(s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    let (a, b) = (rect(s), rect(s));
    run(s, "paint.setFill", json!({"ids": [a.0, b.0], "color": "#ff0000"}));
    run(s, "paint.setStroke", json!({"ids": [a.0], "color": "#00ff00"}));
    run(s, "paint.setStroke", json!({"ids": [b.0], "swatch": "Brand"}));
    run(s, "select.set", json!({"ids": [a.0, b.0]}));
    (a, b)
}

#[test]
fn a_group_from_artwork_holds_its_unique_colours_and_links_the_art() {
    let mut s = session();
    let (a, b) = art(&mut s);
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "swatch.newGroup", json!({"name": "Art", "fromArtwork": true}));
    assert_eq!(r["swatches"], json!(["Brand", "R=255 G=0 B=0", "R=0 G=255 B=0"]), "the linked swatch moves in, the rest are new");
    assert_eq!(r["linked"], 3, "both red fills and the green stroke");
    let d = doc(&s);
    assert!(d.swatch("R=255 G=0 B=0").unwrap().global);
    assert_eq!(d.swatch_group_of("Brand").map(|g| d.swatch_groups[g].name.as_str()), Some("Art"));
    assert_eq!(fill_of(&s, a), linked("#ff0000", "R=255 G=0 B=0"));
    assert_eq!(fill_of(&s, b), linked("#ff0000", "R=255 G=0 B=0"));
    assert_eq!(doc(&s).node(b).unwrap().appearance.stroke_paint(), linked("#2a6fb0", "Brand"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // Without Convert to Global the swatches are process colours and the art stays unlinked.
    run(&mut s, "edit.undo", json!({}));
    let r = run(&mut s, "swatch.newGroup", json!({"fromArtwork": true, "toGlobal": false}));
    assert_eq!((r["name"].as_str(), r["linked"].as_u64()), (Some("Color Group"), Some(0)));
    assert!(!doc(&s).swatch("R=0 G=255 B=0").unwrap().global);
    assert_eq!(fill_of(&s, a), Paint::solid(Color::from_hex("#ff0000").unwrap()));
    run(&mut s, "select.set", json!({"ids": []}));
    assert!(s.execute("swatch.newGroup", &json!({"fromArtwork": true})).is_err(), "needs selected art");
}

#[test]
fn tints_of_global_swatches_get_swatches_only_when_asked() {
    let mut s = session();
    let (_, b) = art(&mut s);
    // A 50% tint of Brand on b's stroke.
    let tint = Color::from_hex("#95b7d8").unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).map_solid_paints_in(&[b], &mut |c, l| {
        let hit = l.as_deref() == Some("Brand");
        if hit {
            *c = tint;
        }
        hit
    });
    let without = run(&mut s, "swatch.newGroup", json!({"fromArtwork": true}));
    assert_eq!(without["swatches"], json!(["Brand", "R=255 G=0 B=0", "R=0 G=255 B=0"]), "a tint brings its swatch");
    run(&mut s, "edit.undo", json!({}));
    let with = run(&mut s, "swatch.newGroup", json!({"fromArtwork": true, "includeTints": true}));
    assert_eq!(with["swatches"], json!(["Brand", "R=255 G=0 B=0", "R=0 G=255 B=0", "R=149 G=183 B=216"]));
    assert_eq!(doc(&s).node(b).unwrap().appearance.stroke_paint(), Paint::Solid { color: tint, swatch: Some("Brand".into()) }, "tints stay linked");
}

#[test]
fn group_names_are_never_blank_and_never_shared_with_their_swatches() {
    let mut s = session();
    art(&mut s);
    // A blank name falls back to the default; a swatch never takes its own group's name.
    let r = run(&mut s, "swatch.newGroup", json!({"name": "  ", "colors": ["#0000ff"]}));
    assert_eq!(r["name"], "Color Group");
    let r = run(&mut s, "swatch.newGroup", json!({"name": "R=255 G=0 B=0", "fromArtwork": true}));
    assert_eq!(r["name"], "R=255 G=0 B=0");
    assert_eq!(r["swatches"], json!(["Brand", "R=255 G=0 B=0 2", "R=0 G=255 B=0"]));
}

#[test]
fn a_group_from_a_mesh_holds_its_point_colours() {
    let mut s = session();
    let a = rect(&mut s);
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "color": "#ff0000"}));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "object.mesh.create", json!({"rows": 2, "cols": 2, "appearance": "center", "highlight": 100}));
    let r = run(&mut s, "swatch.newGroup", json!({"fromArtwork": true}));
    let names = r["swatches"].as_array().unwrap();
    assert!(names.contains(&json!("R=255 G=0 B=0")) && names.contains(&json!("R=255 G=255 B=255")), "{r}");
}
