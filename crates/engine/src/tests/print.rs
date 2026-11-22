//! File → Print: `print.setup` saves the settings with the document (one undo step, kept in the
//! native file), `print.preview` lists what would print, `file.print` writes the PDF.

use serde_json::{Value, json};

use crate::*;

/// A 300 × 200 RGB document with a rectangle; and the rectangle's id.
fn session() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
    (s, id)
}

/// `file.print {p}` → (PDF bytes, its answer).
fn print(s: &mut Session, p: Value) -> (Vec<u8>, Value) {
    let v = s.execute("file.print", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
    (vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap(), v)
}

#[test]
fn setup_is_saved_with_the_document_as_one_undo_step() {
    let (mut s, _) = session();
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["copies"], 1, "defaults before any setup");
    assert!(s.doc().unwrap().doc.print_setup.is_none(), "reading the settings changes nothing");
    let v = s.execute("print.setup", &json!({"settings": {"copies": 3, "media": "a4", "marks": {"trim": true}}})).unwrap();
    assert_eq!((v["settings"]["copies"].clone(), v["settings"]["media"].clone()), (json!(3), json!("a4")));
    let undo = s.doc().unwrap().history.undo.len();
    // Further options go over the saved ones; the same settings again are no new step.
    let v = s.execute("print.setup", &json!({"settings": {"collate": false}})).unwrap();
    let got = (v["settings"]["copies"].clone(), v["settings"]["collate"].clone(), v["settings"]["marks"]["trim"].clone());
    assert_eq!(got, (json!(3), json!(false), json!(true)));
    s.execute("print.setup", &json!({"settings": {"collate": false}})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Saved in the native file, and gone again after undoing both steps.
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert_eq!(back.print_setup, doc.print_setup);
    assert!(back.print_setup.as_ref().is_some_and(|p| p["copies"] == 3));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.print_setup.is_none());
    // Files from before print settings load without them.
    let mut v: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    v.as_object_mut().unwrap().remove("print_setup");
    assert!(vectorcraft_format::load(v.to_string().as_bytes()).unwrap().print_setup.is_none());
}

#[test]
fn overprint_black_is_saved_with_the_setup_and_in_presets() {
    let (mut s, _) = session();
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["output"]["overprintBlack"], false, "off by default");
    let v = s.execute("print.setup", &json!({"settings": {"output": {"mode": "separations", "overprintBlack": true}}})).unwrap();
    assert_eq!(v["settings"]["output"]["overprintBlack"], true);
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert!(back.print_setup.as_ref().is_some_and(|p| p["output"]["overprintBlack"] == true), "kept in the native file");
    // Settings saved before the option load with it off; a value that is not true or false is refused.
    let old: vectorcraft_pdf::PrintSettings = serde_json::from_value(json!({"output": {"mode": "separations"}})).unwrap();
    assert!(!old.output.overprint_black);
    assert!(matches!(s.execute("print.setup", &json!({"settings": {"output": {"overprintBlack": "yes"}}})), Err(EngineError::BadParams { .. })));
    // Print presets keep it and list it as a change from [Default].
    let mut s = Session::new();
    s.execute("print.presets.save", &json!({"name": "Seps", "settings": {"output": {"overprintBlack": true}}})).unwrap();
    assert!(s.prefs.print_presets[0].settings.output.overprint_black);
    let v = s.execute("print.presets.list", &json!({})).unwrap();
    let changed: Vec<&str> = v["presets"][1]["changed"].as_array().unwrap().iter().map(|c| c["option"].as_str().unwrap()).collect();
    assert_eq!(changed, ["output.overprintBlack"]);
}

#[test]
fn the_scale_link_keeps_width_and_height_equal() {
    let (mut s, _) = session();
    let scale = |s: &mut Session, p: Value| s.execute("print.setup", &json!({ "settings": p })).unwrap()["settings"]["scale"].clone();
    assert_eq!(scale(&mut s, json!({})), json!({"width": 100.0, "height": 100.0, "linked": true}), "on for a new job");
    // Linked, one percentage sets the other, whichever it is.
    assert_eq!(scale(&mut s, json!({"scaling": "custom", "scale": {"width": 40}})), json!({"width": 40.0, "height": 40.0, "linked": true}));
    assert_eq!(scale(&mut s, json!({"scale": {"height": 250}})), json!({"width": 250.0, "height": 250.0, "linked": true}));
    assert_eq!(scale(&mut s, json!({"scale": {"width": 60, "height": 60.0}})), json!({"width": 60.0, "height": 60.0, "linked": true}));
    // Two different percentages with linked: true are refused and keep what was saved.
    let apart = s.execute("print.setup", &json!({"settings": {"scale": {"width": 50, "height": 200, "linked": true}}}));
    assert!(matches!(apart, Err(EngineError::BadParams { .. })), "{apart:?}");
    assert!(s.execute("print.preview", &json!({"settings": {"scale": {"width": 50, "height": 200, "linked": true}}})).is_err());
    // `linked: null` keeps the saved link, as null keeps any saved setting.
    let kept = s.execute("print.setup", &json!({"settings": {"scale": {"width": 50, "height": 200, "linked": null}}}));
    assert!(matches!(kept, Err(EngineError::BadParams { .. })), "{kept:?}");
    assert_eq!(scale(&mut s, json!({}))["width"], 60.0);
    // Without linked, two different percentages turn the link off and print as given.
    assert_eq!(scale(&mut s, json!({"scale": {"width": 50, "height": 200}})), json!({"width": 50.0, "height": 200.0, "linked": false}));
    assert_eq!(s.execute("print.preview", &json!({})).unwrap()["sheets"][0]["scale"], json!([50.0, 200.0]));
    // Unlinked, each changes alone; turning the link on sets the height to the width.
    assert_eq!(scale(&mut s, json!({"scale": {"height": 120}})), json!({"width": 50.0, "height": 120.0, "linked": false}));
    assert_eq!(scale(&mut s, json!({"scale": {"linked": true}})), json!({"width": 50.0, "height": 50.0, "linked": true}));
    assert_eq!(s.execute("print.preview", &json!({})).unwrap()["sheets"][0]["scale"], json!([50.0, 50.0]));
    // Kept in the native file and in print presets.
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert_eq!(back.print_setup.as_ref().map(|p| p["scale"].clone()), Some(json!({"width": 50.0, "height": 50.0, "linked": true})));
    s.execute("print.presets.save", &json!({"name": "Apart", "settings": {"scaling": "custom", "scale": {"width": 30, "height": 90}}})).unwrap();
    let preset = s.prefs.print_presets.iter().find(|p| p.name == "Apart").map(|p| p.settings.scale.clone());
    assert_eq!(preset.map(|p| (p.width, p.height, p.linked)), Some((30.0, 90.0, false)));
    assert_eq!(scale(&mut s, json!({}))["linked"], true, "saving a preset leaves the document's settings");
    let v = s.execute("print.setup", &json!({"preset": "Apart"})).unwrap();
    assert_eq!(v["settings"]["scale"], json!({"width": 30.0, "height": 90.0, "linked": false}));
}

#[test]
fn a_scale_saved_without_the_link_keeps_its_width_and_height() {
    let (mut s, _) = session();
    // Settings saved before the link had no `linked`: they print as they did, unlinked.
    let mut old = serde_json::to_value(vectorcraft_pdf::PrintSettings::default()).unwrap();
    old["scaling"] = json!("custom");
    old["scale"] = json!({"width": 50, "height": 200});
    let mut doc = (*s.doc().unwrap().doc).clone();
    doc.print_setup = Some(old);
    s.doc_mut().unwrap().doc = std::sync::Arc::new(doc);
    let pv = s.execute("print.preview", &json!({})).unwrap();
    assert_eq!(pv["sheets"][0]["scale"], json!([50.0, 200.0]));
    assert_eq!(pv["settings"]["scale"]["linked"], false);
    // One percentage changes alone until the link is turned on.
    assert_eq!(s.execute("print.preview", &json!({"settings": {"scale": {"width": 80}}})).unwrap()["sheets"][0]["scale"], json!([80.0, 200.0]));
}

#[test]
fn print_writes_the_saved_setup_with_overrides() {
    let (mut s, _) = session();
    s.execute("print.setup", &json!({"settings": {"media": "a4", "scaling": "fit"}})).unwrap();
    let (bytes, v) = print(&mut s, json!({"settings": {"copies": 2}}));
    assert_eq!(v["pages"], 2);
    let info = vectorcraft_pdf::info(&bytes, None).unwrap();
    assert_eq!(info.pages.len(), 2);
    // A4 turned landscape for the wide artboard.
    assert_eq!((info.pages[0].width.round(), info.pages[0].height.round()), (842.0, 595.0));
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["copies"], 1, "printing doesn't change the saved setup");
    // To a file.
    let path = std::env::temp_dir().join(format!("vc-print-{}.pdf", std::process::id()));
    let v = s.execute("file.print", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(v["pages"], 1);
    assert_eq!(std::fs::read(&path).unwrap().len() as u64, v["bytes"].as_u64().unwrap());
    let _ = std::fs::remove_file(&path);
}

/// A document with four 300 × 200 artboards, set up to print two copies, and a saved print preset
/// "Sheet" that prints artboard 4 only.
fn four_artboards_and_a_preset() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 4})).unwrap();
    s.execute("print.setup", &json!({"settings": {"copies": 2}})).unwrap();
    s.execute("print.presets.save", &json!({"name": "Sheet", "settings": {"artboards": "range", "range": "4"}})).unwrap();
    s
}

#[test]
fn print_takes_a_print_preset_by_name() {
    let mut s = four_artboards_and_a_preset();
    // The preset's settings stand in for the document's: one copy of artboard 4.
    let (bytes, v) = print(&mut s, json!({"preset": "sheet"}));
    assert_eq!((v["pages"].clone(), vectorcraft_pdf::info(&bytes, None).unwrap().pages.len()), (json!(1), 1));
    let (_, v) = print(&mut s, json!({"preset": "Sheet", "settings": {"copies": 3}}));
    assert_eq!(v["pages"], 3, "settings apply over the preset's");
    let (_, v) = print(&mut s, json!({"preset": "[Default]"}));
    assert_eq!(v["pages"], 4, "[Default]: every artboard, one copy");
    assert_eq!(s.execute("print.setup", &json!({})).unwrap()["settings"]["copies"], 2, "printing doesn't change the saved setup");
    // A PostScript file too.
    let v = s.execute("file.print", &json!({"preset": "Sheet", "format": "postscript"})).unwrap();
    assert_eq!(v["pages"], 1);
}

#[test]
fn an_unknown_print_preset_is_refused_with_the_presets() {
    let mut s = four_artboards_and_a_preset();
    for c in ["file.print", "print.preview", "print.setup"] {
        // Agents (MCP, the control channel) may pass `preset`.
        s.check_params(c, &json!({"preset": "Sheet", "settings": {}})).unwrap();
        let e = s.execute(c, &json!({"preset": "Nope"})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{c}: {e}");
        assert!(e.to_string().contains("no print preset named `Nope` (presets: [Default], Sheet)"), "{c}: {e}");
        assert!(matches!(s.execute(c, &json!({"preset": 4})), Err(EngineError::BadParams { .. })), "{c}");
    }
    assert_eq!(s.doc().unwrap().doc.print_setup.as_ref().unwrap()["copies"], 2);
}

#[test]
fn setup_with_a_preset_stores_its_settings() {
    let mut s = four_artboards_and_a_preset();
    let undo = s.doc().unwrap().history.undo.len();
    let v = s.execute("print.setup", &json!({"preset": "Sheet"})).unwrap();
    let got = (v["settings"]["copies"].clone(), v["settings"]["artboards"].clone(), v["settings"]["range"].clone());
    assert_eq!(got, (json!(1), json!("range"), json!("4")));
    let stored = s.doc().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!(stored, v["settings"]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // The same preset again is no new step; settings apply over it.
    s.execute("print.setup", &json!({"preset": "Sheet"})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let v = s.execute("print.setup", &json!({"preset": "Sheet", "settings": {"collate": false}})).unwrap();
    assert_eq!((v["settings"]["range"].clone(), v["settings"]["collate"].clone()), (json!("4"), json!(false)));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.print_setup.as_ref().unwrap()["copies"], 2);
}

#[test]
fn preview_lists_pages_tiles_inks_and_warnings() {
    let (mut s, _) = session();
    let v = s.execute("print.preview", &json!({"settings": {"output": {"mode": "separations"}, "copies": 2}})).unwrap();
    assert_eq!(v["pages"], 8);
    assert_eq!(
        v["sheets"].as_array().unwrap().iter().map(|p| p["ink"].as_str().unwrap()).collect::<Vec<_>>(),
        ["Cyan", "Magenta", "Yellow", "Black"]
    );
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("RGB")), "{}", v["warnings"]);
    assert_eq!(v["settings"]["copies"], 2);
    let v = s.execute("print.preview", &json!({"settings": {"scaling": "tileFull", "scale": {"width": 400, "height": 400}}})).unwrap();
    let tiles = &v["tiles"][0];
    assert_eq!((tiles["columns"].clone(), tiles["rows"].clone()), (json!(2), json!(2)), "{v}");
    assert_eq!(v["pages"], 4);
    assert_eq!(v["sheets"][3]["tile"], 4);
}

#[test]
fn copies_per_sheet_print_a_grid_of_the_artboard() {
    // A 2 × 3.5 in card filled by a rectangle.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 144, "height": 252})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 144, "height": 252})).unwrap();
    let v = s.execute("print.setup", &json!({})).unwrap();
    assert_eq!((v["settings"]["copiesPerSheet"].clone(), v["settings"]["gutter"].clone()), (json!(false), json!(0.0)));
    // With an 18 pt gutter, Letter holds 3 × 3 copies; the first, at the top left, spans x 72 to 216.
    let set = json!({"settings": {"copiesPerSheet": true, "gutter": 18}});
    let v = s.execute("print.preview", &set).unwrap();
    let copies = v["sheets"][0]["copies"].as_array().unwrap();
    assert_eq!((v["pages"].clone(), copies.len(), copies[0].clone()), (json!(1), 9, json!([72.0, 0.0, 216.0, 252.0])));
    // The PDF draws the card's paths nine times.
    let cards = |bytes: &[u8]| {
        let mut n = 0;
        vectorcraft_pdf::import(bytes).unwrap().walk(|node| {
            n += usize::from(node.path_data().and_then(|p| p.bounds()).is_some_and(|b| (b.width().round(), b.height().round()) == (144.0, 252.0)))
        });
        n
    };
    let (bytes, v) = print(&mut s, set);
    assert_eq!(v["pages"], 1);
    let one = cards(&print(&mut s, json!({})).0);
    assert!(one > 0 && cards(&bytes) == 9 * one, "{one}");
    // PostScript places the art once per copy.
    let mut ps = |p: Value| {
        let v = s.execute("file.print", &p).unwrap();
        String::from_utf8(vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap()
    };
    let one = ps(json!({"format": "postscript"})).matches(" cm\n").count();
    let grid = ps(json!({"format": "postscript", "settings": {"copiesPerSheet": true, "gutter": 18}})).matches(" cm\n").count();
    assert_eq!(grid, one + 8);
}

#[test]
fn print_presets_keep_copies_per_sheet_and_older_presets_leave_it_off() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 144, "height": 252})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 144, "height": 252})).unwrap();
    // [Default] prints one copy per sheet.
    let v = s.execute("print.presets.list", &json!({})).unwrap();
    assert_eq!((v["presets"][0]["settings"]["copiesPerSheet"].clone(), v["presets"][0]["settings"]["gutter"].clone()), (json!(false), json!(0.0)));
    // A saved preset keeps the option and the gutter, lists them as changed, and prints with them.
    s.execute("print.presets.save", &json!({"name": "Cards", "settings": {"copiesPerSheet": true, "gutter": 18}})).unwrap();
    let v = s.execute("print.presets.list", &json!({})).unwrap();
    let changed: Vec<&str> = v["presets"][1]["changed"].as_array().unwrap().iter().filter_map(|c| c["option"].as_str()).collect();
    assert_eq!(changed, ["copiesPerSheet", "gutter"]);
    let v = s.execute("print.preview", &json!({"preset": "Cards"})).unwrap();
    assert_eq!(v["sheets"][0]["copies"].as_array().map(Vec::len), Some(9));
    // The option survives export and import.
    let text = s.execute("print.presets.export", &json!({"names": ["Cards"]})).unwrap()["data"].as_str().unwrap().to_string();
    let mut t = Session::new();
    t.execute("print.presets.import", &json!({"data": text})).unwrap();
    assert_eq!((t.prefs.print_presets[0].settings.copies_per_sheet, t.prefs.print_presets[0].settings.gutter), (true, 18.0));
    // A presets file written without the option imports with it off and no gutter.
    let old = r#"{"format": "vcprintpresets", "presets": [{"name": "Old", "settings": {"copies": 2}}]}"#;
    t.execute("print.presets.import", &json!({"data": old})).unwrap();
    let q = &t.prefs.print_presets[1].settings;
    assert_eq!((q.copies, q.copies_per_sheet, q.gutter), (2, false, 0.0));
}

#[test]
fn bad_settings_are_bad_params() {
    let (mut s, _) = session();
    for p in [
        json!({"settings": "letter"}),
        json!({"settings": {"copies": 0}}),
        json!({"settings": {"copies": -1}}),
        json!({"settings": {"media": "quarto"}}),
        json!({"settings": {"artboards": "range", "range": "2"}}),
        json!({"settings": {"scaling": "tileFull", "overlap": 500}}),
        json!({"settings": {"gutter": 800}}),
    ] {
        for c in ["file.print", "print.preview"] {
            assert!(matches!(s.execute(c, &p), Err(EngineError::BadParams { .. })), "{c} {p}");
        }
    }
    // The setup refuses what it can check without laying the document out.
    assert!(matches!(s.execute("print.setup", &json!({"settings": {"copies": 1000}})), Err(EngineError::BadParams { .. })));
    assert!(s.doc().unwrap().doc.print_setup.is_none());
}

#[test]
fn raster_effects_print_as_images() {
    let (mut s, id) = session();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    let (bytes, _) = print(&mut s, json!({}));
    assert!(String::from_utf8_lossy(&bytes).contains("/Image"));
}

#[test]
fn type_in_a_missing_family_warns_as_pdf_export_does() {
    let (mut s, _) = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap()["id"].clone();
    let fallback = |v: &Value| -> Vec<String> {
        v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("fallback font")).map(str::to_string).collect()
    };
    let export = fallback(&s.execute("document.exportPdf", &json!({})).unwrap());
    assert!(export.first().is_some_and(|w| w.contains("Nonexistent Sans Probe")), "{export:?}");
    let (_, v) = print(&mut s, json!({}));
    assert_eq!(fallback(&v), export);
    // Type printed as an image (a raster effect, Print as Bitmap) is drawn in the fallback font too.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    for settings in [json!({}), json!({"advanced": {"printAsBitmap": true}})] {
        let (_, v) = print(&mut s, json!({ "settings": settings }));
        assert_eq!(fallback(&v), export, "{settings}");
    }
}

/// The warnings of `file.print` (PDF and PostScript) and `print.preview` list what the job
/// leaves out of the document or prints differently first, then the notes on the options.
#[test]
fn what_a_print_loses_comes_before_the_notes_on_the_options() {
    let (mut s, _) = session();
    s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let notes = |v: &Value| -> Vec<String> { v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect() };
    // A fixed flatness isn't written to a PDF: a note on the options, made before the type's.
    let fixed = json!({"settings": {"graphics": {"autoFlatness": false}}});
    for v in [print(&mut s, fixed.clone()).1, s.execute("print.preview", &fixed).unwrap()] {
        let n = notes(&v);
        assert!(n.iter().any(|w| w.contains("Nonexistent Sans Probe")), "{n:?}");
        assert!(n.last().is_some_and(|w| w.starts_with("a fixed flatness")), "{n:?}");
    }
    // PostScript doesn't apply the printer profile.
    let profile = json!({"format": "postscript", "settings": {"color": {"profile": vectorcraft_color::cms::SRGB}}});
    let n = notes(&s.execute("file.print", &profile).unwrap());
    assert!(n.iter().any(|w| w.contains("Nonexistent Sans Probe")), "{n:?}");
    assert!(n.last().is_some_and(|w| w.starts_with("the printer profile")), "{n:?}");
    // An artboard larger than Letter: copies per sheet that don't fit come after the art that
    // doesn't print.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 700, "height": 900})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 700, "height": 900})).unwrap();
    let n = notes(&print(&mut s, json!({"settings": {"copiesPerSheet": true}})).1);
    assert!(n.iter().any(|w| w.contains("part of it doesn't print")), "{n:?}");
    assert!(n.last().is_some_and(|w| w.ends_with("it prints one copy per sheet")), "{n:?}");
}

#[test]
fn type_that_does_not_fit_its_frame_is_named() {
    let (mut s, _) = session();
    let p = json!({"x": 10, "y": 100, "text": "Ab\nCd\nEf\nGh", "size": 12, "area": {"width": 100, "height": 20}});
    let id = s.execute("text.create", &p).unwrap()["id"].clone();
    let over = |v: &Value| -> Vec<String> {
        v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("fit its frame")).map(str::to_string).collect()
    };
    let (_, v) = print(&mut s, json!({}));
    assert_eq!(over(&v), [format!("1 text object doesn't fit its frame: the text past the end of the frame doesn't print (id {id})")]);
    assert_eq!(over(&v), over(&s.execute("print.preview", &json!({})).unwrap()));
    // Type printed as an image is named too.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id]})).unwrap();
    for settings in [json!({}), json!({"advanced": {"printAsBitmap": true}})] {
        let (_, v) = print(&mut s, json!({ "settings": settings }));
        assert_eq!(over(&v).len(), 1, "{settings}");
    }
    s.execute("text.areaOptions", &json!({"ids": [id], "height": 150})).unwrap();
    let (_, v) = print(&mut s, json!({}));
    assert_eq!(over(&v), Vec::<String>::new());
}

#[test]
fn for_a_printer_a_landscape_page_is_written_upright() {
    let (mut s, _) = session();
    let size = |pdf: &[u8]| vectorcraft_pdf::info(pdf, None).unwrap().pages.iter().map(|p| (p.width, p.height)).collect::<Vec<_>>();
    let settings = json!({"autoRotate": false, "orientation": "landscape"});
    let (saved, _) = print(&mut s, json!({"settings": settings}));
    assert_eq!(size(&saved), [(792.0, 612.0)]);
    let (spooled, v) = print(&mut s, json!({"settings": settings, "forPrinter": true}));
    assert_eq!(size(&spooled), [(612.0, 792.0)]);
    assert_eq!(v["pages"], 1);
}

#[test]
fn a_spot_ink_converted_to_process_is_saved_and_listed_without_a_plate() {
    let (mut s, _) = session();
    s.execute("swatch.new", &json!({"name": "Gold", "color": {"model": "cmyk", "c": 0, "m": 0.2, "y": 0.8, "k": 0.1}, "spot": true})).unwrap();
    let ink = json!({"name": "Gold", "convertToProcess": true});
    let v = s.execute("print.setup", &json!({"settings": {"output": {"mode": "separations", "inks": [ink]}}})).unwrap();
    assert_eq!(v["settings"]["output"]["inks"], json!([{"name": "Gold", "print": true, "convertToProcess": true}]));
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert_eq!(back.print_setup.as_ref().map(|p| p["output"]["inks"][0]["convertToProcess"].clone()), Some(json!(true)), "kept in the file");
    let pv = s.execute("print.preview", &json!({})).unwrap();
    let names = |list: &Value| list.as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(&pv["inks"]), ["Cyan", "Magenta", "Yellow", "Black"], "no Gold plate");
    assert_eq!(names(&pv["documentInks"]), ["Cyan", "Magenta", "Yellow", "Black", "Gold"]);
    assert_eq!(pv["documentInks"][4]["convertToProcess"], json!(true));
    assert_eq!(pv["documentInks"][0].get("convertToProcess"), None, "written only when on");
}
