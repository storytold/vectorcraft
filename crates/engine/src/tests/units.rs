//! Preferences ▸ Units: the General unit is the open document's units (Preferences and Document
//! Setup set it, new documents start in the preference), Stroke and Type are preferences, and the
//! canvas measurement labels follow them.

use serde_json::json;
use vectorcraft_doc::Unit;
use vectorcraft_tools::{Mods, Overlay, PointerEvent, PointerKind};

use crate::cmd::prefscmds::{EAST_ASIAN_UNITS, PREF_SPECS, PrefKind, UNITS};
use crate::units::Measure;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn units(s: &Session) -> Unit {
    s.doc().unwrap().doc.units
}

#[test]
fn preferences_general_sets_the_open_documents_units_as_one_undo_step() {
    let mut s = session();
    assert_eq!(s.general_unit(), Unit::Points);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "Millimeters"})).unwrap();
    assert_eq!((units(&s), s.general_unit(), s.default_units()), (Unit::Millimeters, Unit::Millimeters, Unit::Millimeters));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Setting it again (the Preferences dialog's OK sends every value) adds no step.
    s.execute("prefs.set", &json!({"values": {"unitsGeneral": "millimeters", "keyboardIncrement": 2}})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(units(&s), Unit::Points);
    assert_eq!(s.prefs.units_general, "millimeters", "undo is the document's; the preference stays");
}

#[test]
fn preferences_general_with_document_units_false_sets_only_the_preference() {
    let mut s = session();
    s.execute("document.setUnits", &json!({"units": "Millimeters"})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("prefs.set", &json!({"values": {"unitsGeneral": "inches"}, "documentUnits": false})).unwrap();
    assert_eq!((units(&s), s.prefs.units_general.as_str()), (Unit::Millimeters, "inches"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    let e = s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "points", "documentUnits": "no"})).unwrap_err();
    assert!(e.to_string().contains("`documentUnits` must be true or false"), "{e}");
    assert_eq!((units(&s), s.prefs.units_general.as_str()), (Unit::Millimeters, "inches"), "nothing changes on error");
}

#[test]
fn new_documents_start_in_the_preference_and_document_setup_only_changes_the_document() {
    let mut s = Session::new();
    assert_eq!(s.general_unit(), Unit::Points, "no document: the preference");
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "inches"})).unwrap();
    assert_eq!(s.general_unit(), Unit::Inches);
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    assert_eq!(units(&s), Unit::Inches);
    s.execute("file.new", &json!({"width": 100, "height": 100, "units": "Pixels"})).unwrap();
    assert_eq!((units(&s), s.general_unit()), (Unit::Pixels, Unit::Pixels), "given units win");
    s.execute("document.setUnits", &json!({"units": "cm"})).unwrap();
    assert_eq!((units(&s), s.default_units()), (Unit::Centimeters, Unit::Inches));
    // Each document keeps its own units.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(s.general_unit(), Unit::Inches);
    assert!(s.execute("file.new", &json!({"units": "furlongs"})).is_err());
}

#[test]
fn document_setup_without_a_document_fails() {
    let mut s = Session::new();
    assert!(s.execute("document.setUnits", &json!({"units": "mm"})).is_err());
    // The preference alone still changes.
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "Millimeters"})).unwrap();
    assert_eq!(s.prefs.units_general, "millimeters");
}

#[test]
fn every_unit_is_a_units_choice() {
    let want: Vec<(&str, &str)> = Unit::ALL.iter().map(|u| (u.key(), u.label())).collect();
    assert_eq!(UNITS, want.as_slice());
    let mut s = session();
    for u in Unit::ALL {
        s.execute("prefs.set", &json!({"values": {"unitsGeneral": u.label(), "unitsStroke": u.key(), "unitsType": u.key()}})).unwrap();
        assert_eq!((s.general_unit(), s.type_unit()), (u, u));
        assert_eq!(s.stroke_unit(), u);
    }
}

#[test]
fn stroke_and_type_units_are_preferences() {
    let mut s = session();
    s.execute("prefs.set", &json!({"values": {"unitsStroke": "millimeters", "unitsType": "pixels"}})).unwrap();
    assert_eq!((s.unit(Measure::General), s.unit(Measure::Stroke), s.unit(Measure::Type)), (Unit::Points, Unit::Millimeters, Unit::Pixels));
    assert_eq!(units(&s), Unit::Points, "they leave the document alone");
}

#[test]
fn length_preferences_take_typed_units_and_say_what_they_follow() {
    let mut s = session();
    s.execute("prefs.set", &json!({"values": {"keyboardIncrement": "1 mm", "gridlineEvery": "1 in", "typeSizeIncrement": 3}})).unwrap();
    assert!((s.prefs.keyboard_increment - 72.0 / 25.4).abs() < 1e-9);
    assert_eq!((s.prefs.gridline_every, s.prefs.type_size_increment), (72.0, 3.0));
    assert!(s.execute("prefs.set", &json!({"key": "pasteOffset", "value": "1 yd"})).is_err(), "out of range");
    let list = s.execute("prefs.list", &json!({})).unwrap();
    let row = |k: &str| list.as_array().unwrap().iter().find(|r| r["key"] == k).unwrap().clone();
    assert_eq!((row("keyboardIncrement")["measure"].clone(), row("keyboardIncrement")["unit"].clone()), (json!("general"), json!("pt")));
    assert_eq!(row("baselineShiftIncrement")["measure"], json!("type"));
    assert!(row("snappingTolerance")["measure"].is_null(), "a screen distance");
    let lengths = PREF_SPECS.iter().filter(|p| matches!(p.kind, PrefKind::Length { .. })).count();
    assert_eq!(lengths, 6);
}

#[test]
fn canvas_measurement_labels_follow_the_units() {
    let mut s = session();
    s.execute("prefs.set", &json!({"values": {"unitsGeneral": "millimeters", "unitsStroke": "inches"}})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("rectangle", v).unwrap();
    for (kind, x, y) in [(PointerKind::Down, 0.0, 0.0), (PointerKind::Drag, 72.0, 36.0)] {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(Mods::default()), v).unwrap();
    }
    let labels: Vec<String> =
        s.overlays(v).into_iter().filter_map(|o| if let Overlay::Measure { text, .. } = o { Some(text) } else { None }).collect();
    assert_eq!(labels, ["W: 25.40 mm\nH: 12.70 mm"]);
}

#[test]
fn units_survive_a_native_round_trip() {
    let mut s = session();
    s.execute("document.setUnits", &json!({"units": "Feet & Inches"})).unwrap();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(back.units, Unit::FeetInches);
}

#[test]
fn distance_params_take_a_unit() {
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Hi"})).unwrap()["id"].clone();
    s.execute("select.set", &json!({ "ids": [id] })).unwrap();
    s.execute("text.setStyle", &json!({"size": "0.5 in", "leading": "1in", "tracking": "20"})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not text") };
    let st = t.first_style();
    assert_eq!((st.size, st.leading, st.tracking), (36.0, Some(72.0), 20.0));
}

#[test]
fn east_asian_type_unit_is_a_preference_that_offers_q_and_ha() {
    let want: Vec<(&str, &str)> = Unit::EAST_ASIAN.iter().map(|u| (u.key(), u.label())).collect();
    assert_eq!(EAST_ASIAN_UNITS, want.as_slice());
    let mut s = session();
    assert_eq!(s.east_asian_type_unit(), Unit::Points, "points until chosen");
    for u in Unit::EAST_ASIAN {
        s.execute("prefs.set", &json!({"key": "unitsAsianType", "value": u.key()})).unwrap();
        assert_eq!(s.east_asian_type_unit(), u);
        assert_eq!(s.unit(Measure::AsianType), u);
        assert_eq!(s.type_unit(), Unit::Points, "Type is its own preference");
    }
    // By label, in any case.
    s.execute("prefs.set", &json!({"key": "unitsAsianType", "value": "ha"})).unwrap();
    assert_eq!(s.prefs.units_asian_type, "ha");
    s.execute("prefs.set", &json!({"key": "unitsAsianType", "value": "Q"})).unwrap();
    assert_eq!(s.prefs.units_asian_type, "q");
    // No other Units preference offers them.
    for key in ["unitsGeneral", "unitsStroke", "unitsType"] {
        for v in ["q", "Q", "ha", "Ha"] {
            assert!(s.execute("prefs.set", &json!({"key": key, "value": v})).is_err(), "{key} = {v}");
        }
    }
    assert_eq!((Measure::AsianType.name(), Measure::AsianType.pref_key()), ("asianType", "unitsAsianType"));
    let list = s.execute("prefs.list", &json!({})).unwrap();
    let row = list.as_array().unwrap().iter().find(|r| r["key"] == "unitsAsianType").unwrap().clone();
    assert_eq!(row["options"].as_array().unwrap().iter().rev().take(2).collect::<Vec<_>>(), [&json!("ha"), &json!("q")]);
}

#[test]
fn east_asian_type_unit_leaves_the_document_in_points() {
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "日本語"})).unwrap()["id"].clone();
    s.execute("select.set", &json!({ "ids": [id] })).unwrap();
    s.execute("text.setStyle", &json!({"size": 12, "leading": 15})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    s.execute("prefs.set", &json!({"key": "unitsAsianType", "value": "q"})).unwrap();
    let after = &s.doc().unwrap().doc;
    assert_eq!(units(&s), Unit::Points);
    assert_eq!(after.units, before.units);
    let size = |d: &vectorcraft_doc::Document| match &d.node(NodeId(id.as_u64().unwrap())).unwrap().kind {
        NodeKind::Text(t) => (t.first_style().size, t.first_style().leading),
        _ => panic!("not text"),
    };
    assert_eq!(size(after), size(&before));
    assert_eq!(size(after), (12.0, Some(15.0)));
    // Typed in Q or Ha, the value is stored in points.
    s.execute("text.setStyle", &json!({"size": "17 Q", "leading": "20 H"})).unwrap();
    let (sz, lead) = size(&s.doc().unwrap().doc);
    assert!((sz - Unit::Q.to_pt(17.0)).abs() < 1e-9, "{sz}");
    assert!((lead.unwrap() - Unit::Ha.to_pt(20.0)).abs() < 1e-9, "{lead:?}");
    assert!((sz - 17.0 * 72.0 / 101.6).abs() < 1e-9);
}

/// A session whose Preferences › Units › General is `unit`, with no document open.
fn general_in(unit: &str) -> Session {
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": unit})).unwrap();
    s
}

/// `bytes` opened as `name` in `s` → the opened document's units.
fn opened_units(s: &mut Session, name: &str, bytes: &[u8]) -> Unit {
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
    units(s)
}

/// The active document of a session in `unit`, exported as `format`.
fn exported(unit: &str, format: &str) -> Vec<u8> {
    let mut s = session();
    s.execute("document.setUnits", &json!({"units": unit})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    let data = s.execute("document.export", &json!({"format": format})).unwrap()["dataBase64"].clone();
    vectorcraft_format::base64_decode(data.as_str().unwrap()).unwrap()
}

/// A PostScript `.ai` with one empty layer and `header` among its comments.
fn postscript_ai(header: &str) -> Vec<u8> {
    format!(
        "%!PS-Adobe-3.0\n%%Creator: test\n%AI5_FileFormat 14.0\n{header}%AI3_Cropmarks: 0 0 200 100\n%%EndComments\n%%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\n0 A\n0 Xw\n\nLB\n%AI5_EndLayer--\n%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n"
    )
    .into_bytes()
}

#[test]
fn files_that_record_no_units_open_in_the_general_unit() {
    let pdf = vectorcraft_testkit::pdf::pdf(&[vectorcraft_testkit::pdf::PdfPage::new(120.0, 60.0, "0 0 0 rg 10 10 50 30 re f")], None);
    let png = exported("Inches", "png");
    let ai = postscript_ai("");
    let mut s = general_in("Millimeters");
    assert_eq!(opened_units(&mut s, "plain.pdf", &pdf), Unit::Millimeters, "a PDF records no units (#5820)");
    assert_eq!(opened_units(&mut s, "image.png", &png), Unit::Millimeters, "nor does an image");
    assert_eq!(opened_units(&mut s, "old.ai", &ai), Unit::Millimeters, "nor a .ai without ruler units");
    // Only the units change: the page keeps its size in points.
    s.execute("document.open", &json!({"name": "plain.pdf", "dataBase64": vectorcraft_format::base64_encode(&pdf)})).unwrap();
    let board = s.doc().unwrap().doc.artboards[0].rect;
    assert!((board.width() - 120.0).abs() < 1e-6 && (board.height() - 60.0).abs() < 1e-6, "{board:?}");
    // With the preference left at points, they open in points as before.
    let mut p = general_in("Points");
    assert_eq!(opened_units(&mut p, "plain.pdf", &pdf), Unit::Points);
}

#[test]
fn files_that_record_units_keep_them_whatever_the_general_unit() {
    let mut s = general_in("Millimeters");
    assert_eq!(opened_units(&mut s, "inches.ai", &postscript_ai("%AI5_RulerUnits: 0\n")), Unit::Inches, "a .ai's ruler units");
    assert_eq!(opened_units(&mut s, "points.ai", &postscript_ai("%AI5_RulerUnits: 2\n")), Unit::Points, "ruler units in points are kept too");
    assert_eq!(opened_units(&mut s, "saved.vectorcraft", &exported("Inches", "vectorcraft")), Unit::Inches, "a native file");
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2in" height="1in" viewBox="0 0 144 72"><rect width="10" height="10"/></svg>"#;
    assert_eq!(opened_units(&mut s, "inches.svg", svg.as_bytes()), Unit::Inches, "an SVG's root size");
}

#[test]
fn revert_reopens_a_file_without_units_in_the_general_unit() {
    let dir = std::env::temp_dir().join(format!("vc-5820-revert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plain.pdf");
    std::fs::write(&path, vectorcraft_testkit::pdf::pdf(&[vectorcraft_testkit::pdf::PdfPage::new(120.0, 60.0, "0 0 0 rg 10 10 50 30 re f")], None))
        .unwrap();
    let mut s = general_in("Millimeters");
    s.execute("document.open", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(units(&s), Unit::Millimeters);
    s.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
    s.execute("document.setUnits", &json!({"units": "Inches"})).unwrap();
    s.execute("file.revert", &json!({})).unwrap();
    assert_eq!(units(&s), Unit::Millimeters, "as the file opens");
    let _ = std::fs::remove_dir_all(&dir);
}
