//! File → New (`file.new`, `file.newPresets*`): artboard layouts, orientation, bleed, colour mode,
//! raster effects resolution, background, presets, Recent and Saved.

use serde_json::{Value, json};
use vectorcraft_doc::{Background, ColorMode, Unit};
use vectorcraft_geom::Rect;

use crate::*;

fn new_doc(p: Value) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &p).unwrap();
    s
}

fn boards(s: &Session) -> Vec<Rect> {
    s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect()
}

fn presets(s: &mut Session, category: &str) -> Vec<Value> {
    let r = s.execute("file.newPresets", &json!({ "category": category })).unwrap();
    r["categories"][0]["presets"].as_array().unwrap().clone()
}

#[test]
fn four_artboards_in_two_columns_make_a_two_by_two_grid() {
    let s = new_doc(json!({"width": 100, "height": 50, "artboards": 4, "artboardLayout": {"columns": 2, "spacing": 10}}));
    let origins: Vec<(f64, f64)> = boards(&s).iter().map(|r| (r.x0, r.y0)).collect();
    assert_eq!(origins, [(0.0, 0.0), (110.0, 0.0), (0.0, 60.0), (110.0, 60.0)]);
    assert!(boards(&s).iter().all(|r| r.width() == 100.0 && r.height() == 50.0));
    let names: Vec<String> = s.doc().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
    assert_eq!(names, ["Artboard 1", "Artboard 2", "Artboard 3", "Artboard 4"]);
    // Grid by Column fills down first; Right-to-Left starts at the right.
    let s = new_doc(json!({"width": 100, "height": 50, "artboards": 4, "artboardLayout": {"layout": "gridByColumn", "columns": 2, "spacing": 10}}));
    assert_eq!(boards(&s).iter().map(|r| (r.x0, r.y0)).collect::<Vec<_>>(), [(0.0, 0.0), (0.0, 60.0), (110.0, 0.0), (110.0, 60.0)]);
    let s = new_doc(json!({"width": 100, "height": 50, "artboards": 3, "artboardLayout": {"layout": "row", "rightToLeft": true}}));
    assert_eq!(boards(&s).iter().map(|r| r.x0).collect::<Vec<_>>(), [240.0, 120.0, 0.0]);
    let s = new_doc(json!({"width": 100, "height": 50, "artboards": 3, "artboardLayout": "column"}));
    assert_eq!(boards(&s).iter().map(|r| r.y0).collect::<Vec<_>>(), [0.0, 70.0, 140.0]);
    // Without a layout: one row, 20 pt apart (as before).
    let s = new_doc(json!({"width": 100, "height": 50, "artboards": 2}));
    assert_eq!(boards(&s)[1].x0, 120.0);
}

#[test]
fn orientation_swaps_width_and_height() {
    let s = new_doc(json!({"width": 612, "height": 792, "orientation": "landscape"}));
    assert_eq!((boards(&s)[0].width(), boards(&s)[0].height()), (792.0, 612.0));
    let s = new_doc(json!({"width": 792, "height": 612, "orientation": "Portrait"}));
    assert_eq!((boards(&s)[0].width(), boards(&s)[0].height()), (612.0, 792.0));
    let s = new_doc(json!({"preset": "HDTV 1080p", "orientation": "portrait"}));
    assert_eq!((boards(&s)[0].width(), boards(&s)[0].height()), (1080.0, 1920.0));
}

#[test]
fn every_setting_reaches_the_document() {
    let mut s = Session::new();
    let r = s
        .execute(
            "file.new",
            &json!({"name": "Poster", "width": "210 mm", "height": "297mm", "units": "Millimeters", "bleed": [9, 9, 4.5, 4.5],
                "backgroundContents": "white", "colorMode": "cmyk", "rasterEffectsPpi": 150}),
        )
        .unwrap();
    assert_eq!(r["previewMode"], "default");
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.title, "Poster");
    assert_eq!(d.units, Unit::Millimeters);
    assert!((d.artboards[0].rect.width() - 595.2756).abs() < 1e-3);
    assert_eq!(d.setup.bleed, [9.0, 9.0, 4.5, 4.5]);
    assert_eq!(d.setup.background, Background::White);
    assert_eq!(d.color_mode, ColorMode::Cmyk);
    assert_eq!(d.raster_effects_ppi, 150.0);
    // Saved and reopened, the settings stay.
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(d)).unwrap();
    assert_eq!((back.raster_effects_ppi, back.setup.bleed, back.setup.background), (150.0, d.setup.bleed, Background::White));
}

#[test]
fn a_length_with_a_percentage_is_a_share_of_the_value_before_it() {
    let s = new_doc(json!({"width": "200mm*50%", "height": "100mm+25%", "units": "Millimeters"}));
    let r = boards(&s)[0];
    assert!((r.width() - Unit::Millimeters.to_pt(100.0)).abs() < 1e-9, "{}", r.width());
    assert!((r.height() - Unit::Millimeters.to_pt(125.0)).abs() < 1e-9, "{}", r.height());
}

#[test]
fn a_white_background_exports_opaque() {
    let alpha = |bg: &str| {
        let mut s = new_doc(json!({"width": 20, "height": 20, "backgroundContents": bg}));
        let r = s.execute("document.export", &json!({"format": "png"})).unwrap();
        let png = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
        image::load_from_memory(&png).unwrap().to_rgba8().get_pixel(10, 10).0
    };
    assert_eq!(alpha("white"), [255, 255, 255, 255]);
    assert_eq!(alpha("transparent")[3], 0);
}

#[test]
fn a_preset_starts_the_document_and_params_change_it() {
    let s = new_doc(json!({"preset": "a4"}));
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.units, d.color_mode, d.raster_effects_ppi), (Unit::Points, ColorMode::Cmyk, 300.0), "print sizes in prefs unitsGeneral");
    assert!((d.artboards[0].rect.height() - 841.89).abs() < 0.01);
    let s = new_doc(json!({"preset": "Web 1920×1080", "colorMode": "cmyk", "artboards": 2}));
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.units, d.color_mode, d.artboards.len()), (Unit::Pixels, ColorMode::Cmyk, 2));
    assert!(Session::new().execute("file.new", &json!({"preset": "Napkin"})).is_err());
    // Print presets and plain sizes start in Preferences ▸ Units ▸ General; screen presets in pixels.
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "millimeters"})).unwrap();
    s.execute("file.new", &json!({"preset": "A4"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.units, Unit::Millimeters);
    assert_eq!(presets(&mut s, "Print")[4]["size"], "210 × 297 mm");
    s.execute("file.new", &json!({"width": 100})).unwrap();
    assert_eq!(s.doc().unwrap().doc.units, Unit::Millimeters);
    s.execute("file.new", &json!({"preset": "HDTV 720p"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.units, Unit::Pixels);
}

#[test]
fn ppi_overprint_preview_and_bad_params() {
    // Overprint Preview is process-wide.
    let _view = crate::tests_colormgmt::GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = new_doc(json!({"rasterEffectsPpi": 300, "previewMode": "overprint"}));
    assert_eq!(s.doc().unwrap().doc.raster_effects_ppi, 300.0);
    let overprint = |s: &mut Session| s.execute("view.proofSetup", &json!({})).unwrap()["overprintPreview"].clone();
    assert_eq!(overprint(&mut s), true);
    s.execute("view.overprintPreview", &json!({"on": false})).unwrap();
    let r = s.execute("file.new", &json!({"previewMode": "pixel"})).unwrap();
    assert_eq!(r["previewMode"], "pixel", "for the app's Pixel Preview");
    assert_eq!(overprint(&mut s), false);
    for p in [
        json!({"orientation": "sideways"}),
        json!({"width": -5}),
        json!({"width": "wide"}),
        json!({"bleed": 100}),
        json!({"backgroundContents": "plaid"}),
        json!({"colorMode": "lab"}),
        json!({"rasterEffectsPpi": 0}),
        json!({"previewMode": "xray"}),
        json!({"artboardLayout": {"layout": "spiral"}}),
        json!({"artboardLayout": {"columns": 0}}),
        json!({"artboardLayout": {"rows": 2}}),
        json!({"units": "furlongs"}),
        json!({"width": 160000, "artboards": 1000}),
    ] {
        let mut s = Session::new();
        assert!(s.execute("file.new", &p).is_err(), "{p}");
        assert!(s.active().is_none(), "{p}: no document on an error");
    }
}

#[test]
fn categories_list_generated_presets_with_neutral_names() {
    let mut s = Session::new();
    let r = s.execute("file.newPresets", &json!({})).unwrap();
    let names: Vec<&str> = r["categories"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Recent", "Saved", "Mobile", "Web", "Print", "Film & Video", "Art & Illustration", "Branding", "Social"]);
    let print = presets(&mut s, "print");
    let letter = print.iter().find(|p| p["name"] == "Letter").unwrap();
    assert_eq!(
        (letter["size"].as_str(), letter["colorMode"].as_str(), letter["orientation"].as_str()),
        (Some("612 × 792 pt"), Some("cmyk"), Some("portrait"))
    );
    assert_eq!(presets(&mut s, "Film & Video").iter().find(|p| p["name"] == "4K UHD").unwrap()["size"], "3840 × 2160 px");
    for c in ["Mobile", "Web", "Print", "Film & Video", "Art & Illustration", "Branding", "Social"] {
        assert!(!presets(&mut s, c).is_empty(), "{c}");
    }
    // A listed preset can be passed straight back to file.new.
    let story = presets(&mut s, "Social").into_iter().find(|p| p["name"] == "Social Story 1080×1920").unwrap();
    s.execute("file.new", &story).unwrap();
    assert_eq!(boards(&s)[0], Rect::new(0.0, 0.0, 1080.0, 1920.0));
    assert!(s.execute("file.newPresets", &json!({"category": "Furniture"})).is_err());
}

/// Branding has the Japanese business card, 91 × 55 mm, after the 85 × 55 mm one (#3828).
#[test]
fn branding_has_the_japanese_business_card() {
    let mut s = Session::new();
    let branding = presets(&mut s, "Branding");
    let names: Vec<&str> = branding.iter().map(|p| p["name"].as_str().unwrap()).collect();
    let at = names.iter().position(|n| *n == "Business Card 91 × 55 mm").unwrap_or_else(|| panic!("{names:?}"));
    assert_eq!(names[at - 1], "Business Card 85 × 55 mm");
    s.execute("file.new", &json!({"preset": "Business Card 91 × 55 mm"})).unwrap();
    let b = boards(&s)[0];
    let mm = 72.0 / 25.4;
    assert!((b.width() - 91.0 * mm).abs() < 1e-6 && (b.height() - 55.0 * mm).abs() < 1e-6, "{b:?}");
}

#[test]
fn recent_remembers_the_last_sizes() {
    let mut s = Session::new();
    assert!(presets(&mut s, "Recent").is_empty());
    s.execute("file.new", &json!({"preset": "A4"})).unwrap();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("file.new", &json!({"preset": "A4", "orientation": "landscape"})).unwrap();
    let recent = presets(&mut s, "Recent");
    let names: Vec<&str> = recent.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["A4", "Custom", "A4"], "newest first; a turned preset keeps its name");
    // The same size again moves to the front instead of repeating.
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    assert_eq!(presets(&mut s, "Recent").len(), 3);
    assert_eq!(presets(&mut s, "Recent")[0]["size"], "300 × 200 pt");
    for i in 0..20 {
        s.execute("file.new", &json!({"width": 100 + i, "height": 100})).unwrap();
    }
    assert_eq!(presets(&mut s, "Recent").len(), 12);
}

/// A saved preset or a Recent entry whose size, artboards, spacing or bleed the preferences hold
/// outside the range `file.new` takes reads as the nearer end of that range (a size of 0 or less
/// reads as 1 pt), so `file.new` makes a document from it.
#[test]
fn a_saved_preset_with_values_out_of_range_reads_in_range() {
    let mut s = Session::new();
    let prefs = json!({
        "newDocPresets": [
            {"name": "Low", "width": -5, "height": 0, "artboards": 0, "spacing": -50, "bleed": [-5, -1, 0, 9]},
            {"name": "High", "width": f64::MAX, "height": 200000, "spacing": f64::MAX, "bleed": [100, f64::MAX, 72, 0]},
            {"name": "Many", "width": 10, "height": 10, "artboards": 5000, "spacing": 0},
        ],
        "recentNewDocs": [{"name": "Custom", "width": 100, "height": 50, "artboards": 3, "spacing": -50}],
    });
    s.apply_prefs(serde_json::from_value(prefs).unwrap());
    let read = |p: &Value| {
        let n = |v: &Value| v.as_f64().unwrap();
        let bleed: Vec<f64> = p["bleed"].as_array().unwrap().iter().map(n).collect();
        (n(&p["width"]), n(&p["height"]), n(&p["artboards"]), n(&p["artboardLayout"]["spacing"]), bleed)
    };
    let saved: Vec<_> = presets(&mut s, "Saved").iter().map(read).collect();
    assert_eq!(
        saved,
        [
            (1.0, 1.0, 1.0, 0.0, vec![0.0, 0.0, 0.0, 9.0]),
            (3_999_856.0, 200000.0, 1.0, 1.0e5, vec![72.0, 72.0, 72.0, 0.0]),
            (10.0, 10.0, 1000.0, 0.0, vec![0.0; 4]),
        ]
    );
    assert_eq!(read(&presets(&mut s, "Recent")[0]), (100.0, 50.0, 3.0, 0.0, vec![0.0; 4]));
    // file.new makes each one with those values.
    s.execute("file.new", &json!({"preset": "Low"})).unwrap();
    assert_eq!((boards(&s), s.doc().unwrap().doc.setup.bleed), (vec![Rect::new(0.0, 0.0, 1.0, 1.0)], [0.0, 0.0, 0.0, 9.0]));
    s.execute("file.new", &json!({"preset": "High"})).unwrap();
    assert_eq!((boards(&s), s.doc().unwrap().doc.setup.bleed), (vec![Rect::new(0.0, 0.0, 3_999_856.0, 200000.0)], [72.0, 72.0, 72.0, 0.0]));
    s.execute("file.new", &json!({"preset": "Many"})).unwrap();
    assert_eq!(boards(&s).len(), 1000);
    let custom = presets(&mut s, "Recent").into_iter().find(|p| p["name"] == "Custom").unwrap();
    s.execute("file.new", &custom).unwrap();
    assert_eq!(boards(&s).iter().map(|r| r.x0).collect::<Vec<_>>(), [0.0, 100.0, 200.0], "side by side, not overlapping");
}

/// `file.new` and `artboard.new` take the same largest artboard side (the canvas less the largest
/// bleed on both sides) and refuse a side just past it, or of 0 or less, with an error that names
/// the limit in points and, when the document's units aren't points, in those units too.
#[test]
fn file_new_and_artboard_new_take_the_same_largest_side() {
    let max = 3_999_856.0;
    let in_meters = "at most 3999856 pt (1411.0603 m)";
    let s = new_doc(json!({"width": max, "height": 170078}));
    assert_eq!(boards(&s), [Rect::new(0.0, 0.0, max, 170078.0)]);
    let mut s = Session::new();
    let err = s.execute("file.new", &json!({"width": max + 1.0, "height": 100})).unwrap_err().to_string();
    assert!(err.contains("width and height must be more than 0 and at most 3999856 pt") && !err.contains('('), "{err}");
    let err = s.execute("file.new", &json!({"width": 100, "height": "1412 m", "units": "Meters"})).unwrap_err().to_string();
    assert!(err.contains(in_meters), "{err}");
    // The value in other units is rounded down, so it is one file.new takes.
    let err = s.execute("file.new", &json!({"width": 100, "height": "56000 in", "units": "Inches"})).unwrap_err().to_string();
    assert!(err.contains("at most 3999856 pt (55553.5555 in)"), "{err}");
    assert!(s.execute("file.new", &json!({"width": 0, "height": 100})).is_err());
    assert!(s.doc().is_err(), "no document was made");
    s.execute("file.new", &json!({"width": "55553.5555 in", "height": 100, "units": "Inches"})).unwrap();

    let mut s = new_doc(json!({"units": "Meters"}));
    s.execute("artboard.new", &json!({"x": -2_000_000, "y": 0, "width": max, "height": 100})).unwrap();
    assert_eq!(boards(&s)[1], Rect::new(-2_000_000.0, 0.0, max - 2_000_000.0, 100.0));
    for p in [
        json!({"x": -2_000_000, "y": 0, "width": max + 1.0, "height": 100}),
        json!({"x": 0, "y": 2000, "width": 100, "height": max + 1.0}),
        json!({"width": 0}),
        json!({"height": -1}),
    ] {
        let err = s.execute("artboard.new", &p).unwrap_err().to_string();
        assert!(err.contains(in_meters), "{p}: {err}");
    }
    assert_eq!(boards(&s).len(), 2, "the refused artboards were not added");
    // A size taken from a wider artboard is refused too.
    s.execute("artboard.setProps", &json!({"index": 1, "x": -2_000_000, "y": 0, "width": max + 1.0, "height": 100})).unwrap();
    for p in [json!({}), json!({"x": 0, "y": 5000, "height": 100}), json!({"side": "bottom", "beside": 1})] {
        let err = s.execute("artboard.new", &p).unwrap_err().to_string();
        assert!(err.contains(in_meters), "{p}: {err}");
    }
    assert_eq!(boards(&s).len(), 2);

    // A side below 1 pt is kept as given, as file.new keeps it, and a size taken from such an artboard keeps it too.
    let mut s = new_doc(json!({"width": 0.5, "height": 0.25}));
    assert_eq!(boards(&s), [Rect::new(0.0, 0.0, 0.5, 0.25)]);
    s.execute("artboard.new", &json!({"x": 0, "y": 10, "width": 0.5, "height": 0.75})).unwrap();
    s.execute("artboard.new", &json!({"x": 0, "y": 20})).unwrap();
    assert_eq!(boards(&s)[1..], [Rect::new(0.0, 10.0, 0.5, 10.75), Rect::new(0.0, 20.0, 0.5, 20.75)]);
    // artboard.setProps keeps such sides when it changes something else, on a locked artboard too,
    // and still raises a supplied side to 1 pt.
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Small"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 2, "locked": true})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 2, "name": "Small locked"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "height": 0.5})).unwrap();
    assert_eq!(boards(&s), [Rect::new(0.0, 0.0, 0.5, 1.0), Rect::new(0.0, 10.0, 0.5, 10.75), Rect::new(0.0, 20.0, 0.5, 20.75)]);
}

#[test]
fn saved_presets_live_in_the_preferences() {
    let mut s = Session::new();
    assert!(s.execute("file.newPresets.delete", &json!({"name": "Mine"})).is_err());
    let r = s
        .execute(
            "file.newPresets.save",
            &json!({"name": "Mine", "width": 400, "height": 300, "units": "Pixels", "artboards": 4, "artboardLayout": {"columns": 2}, "bleed": 9}),
        )
        .unwrap();
    assert_eq!(r, json!({"name": "Mine", "count": 1}));
    let saved = presets(&mut s, "Saved");
    assert_eq!(
        (saved[0]["name"].as_str(), saved[0]["size"].as_str(), saved[0]["artboardLayout"]["columns"].as_u64()),
        (Some("Mine"), Some("400 × 300 px"), Some(2))
    );
    // The same name replaces it.
    s.execute("file.newPresets.save", &json!({"name": "mine", "width": 500, "height": 300})).unwrap();
    assert_eq!(presets(&mut s, "Saved").len(), 1);
    s.execute("file.new", &json!({"preset": "Mine"})).unwrap();
    assert_eq!(boards(&s)[0].width(), 500.0);
    // Kept with the preferences, and by a reset.
    let prefs: crate::Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(prefs.new_doc_presets, s.prefs.new_doc_presets);
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.new_doc_presets.len(), 1);
    assert_eq!(s.execute("file.newPresets.delete", &json!({"name": "MINE"})).unwrap(), json!({"deleted": "mine"}));
    assert!(presets(&mut s, "Saved").is_empty());
    assert!(s.execute("file.newPresets.save", &json!({"name": " "})).is_err());
}

#[test]
fn a_saved_preset_with_the_resolution_out_of_range_reads_in_range() {
    let mut s = Session::new();
    let saved = json!({"newDocPresets": [{"name": "Zero", "rasterEffectsPpi": 0}, {"name": "Huge", "rasterEffectsPpi": 5000}]});
    s.apply_prefs(serde_json::from_value(saved).unwrap());
    let ppi: Vec<Option<f64>> = presets(&mut s, "Saved").iter().map(|p| p["rasterEffectsPpi"].as_f64()).collect();
    assert_eq!(ppi, [Some(1.0), Some(2400.0)]);
    s.execute("file.new", &json!({"preset": "Zero"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.raster_effects_ppi, 1.0);
}

/// Advanced Options › Align New Objects to Pixel Grid starts on in the Mobile and Web presets and
/// off in the others and without a preset; a `file.new` param overrides the preset; the document
/// takes the value; saved presets and Recent keep it, and a saved preset from before it reads off.
#[test]
fn align_new_objects_to_pixel_grid_follows_the_preset() {
    let align = |s: &Session| s.doc().unwrap().doc.setup.align_new_objects_to_pixel_grid;
    let mut s = Session::new();
    for category in ["Mobile", "Web"] {
        assert!(presets(&mut s, category).iter().all(|p| p["alignNewObjectsToPixelGrid"] == true), "{category}");
    }
    for category in ["Print", "Film & Video", "Art & Illustration", "Branding", "Social"] {
        assert!(presets(&mut s, category).iter().all(|p| p["alignNewObjectsToPixelGrid"] == false), "{category}");
    }
    s.execute("file.new", &json!({"preset": "Web 1920×1080"})).unwrap();
    assert!(align(&s));
    assert_eq!(presets(&mut s, "Recent")[0]["alignNewObjectsToPixelGrid"], true);
    s.execute("file.new", &json!({"preset": "Phone 390×844"})).unwrap();
    assert!(align(&s));
    s.execute("file.new", &json!({"preset": "Letter"})).unwrap();
    assert!(!align(&s));
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    assert!(!align(&s));
    s.execute("file.new", &json!({"preset": "Web 1920×1080", "alignNewObjectsToPixelGrid": false})).unwrap();
    assert!(!align(&s));
    s.execute("file.new", &json!({"preset": "A4", "alignNewObjectsToPixelGrid": true})).unwrap();
    assert!(align(&s));
    assert!(Session::new().execute("file.new", &json!({"alignNewObjectsToPixelGrid": "on"})).is_err());
    // A Recent entry passed back makes the document it was made as.
    let recent = presets(&mut s, "Recent")[0].clone();
    assert_eq!((recent["name"].as_str(), recent["alignNewObjectsToPixelGrid"].as_bool()), (Some("A4"), Some(true)));
    s.execute("file.new", &recent).unwrap();
    assert!(align(&s));
    // Saved presets keep it.
    s.execute("file.newPresets.save", &json!({"name": "Icons", "width": 64, "height": 64, "alignNewObjectsToPixelGrid": true})).unwrap();
    s.execute("file.newPresets.save", &json!({"name": "Banner", "preset": "Web 1920×1080", "alignNewObjectsToPixelGrid": false})).unwrap();
    let saved: Vec<Value> = presets(&mut s, "Saved").iter().map(|p| p["alignNewObjectsToPixelGrid"].clone()).collect();
    assert_eq!(saved, [json!(true), json!(false)]);
    s.execute("file.new", &json!({"preset": "Icons"})).unwrap();
    assert!(align(&s));
    s.execute("file.new", &json!({"preset": "Banner"})).unwrap();
    assert!(!align(&s));
    let prefs: crate::Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(prefs.new_doc_presets, s.prefs.new_doc_presets);
    // A saved preset from before the setting reads off, whatever its name.
    let mut old = Session::new();
    old.apply_prefs(serde_json::from_value(json!({"newDocPresets": [{"name": "My Web", "width": 100, "height": 100}]})).unwrap());
    assert_eq!(presets(&mut old, "Saved")[0]["alignNewObjectsToPixelGrid"], false);
    old.execute("file.new", &json!({"preset": "My Web"})).unwrap();
    assert!(!align(&old));
}

#[test]
fn rearrange_keeps_its_grid() {
    let mut s = new_doc(json!({"width": 100, "height": 50, "artboards": 3}));
    s.execute("artboard.rearrange", &json!({"columns": 2, "spacing": 5})).unwrap();
    assert_eq!(boards(&s).iter().map(|r| (r.x0, r.y0)).collect::<Vec<_>>(), [(0.0, 0.0), (105.0, 0.0), (0.0, 55.0)]);
}

/// #681: every layout and order of Rearrange All Artboards, five 100 × 50 artboards 10 pt apart
/// with Columns (the rows of a grid by column) 2: each artboard lands in its (row, column) cell, the
/// art on it rides along, and it all undoes in one step.
#[test]
fn rearrange_lays_out_by_layout_and_order() {
    type Cells = [(u32, u32); 5];
    let cells: [(&str, &str, Cells); 8] = [
        ("gridByRow", "leftToRight", [(0, 0), (0, 1), (1, 0), (1, 1), (2, 0)]),
        ("gridByRow", "rightToLeft", [(0, 1), (0, 0), (1, 1), (1, 0), (2, 1)]),
        ("gridByColumn", "leftToRight", [(0, 0), (1, 0), (0, 1), (1, 1), (0, 2)]),
        ("gridByColumn", "rightToLeft", [(0, 2), (1, 2), (0, 1), (1, 1), (0, 0)]),
        ("row", "leftToRight", [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]),
        ("row", "rightToLeft", [(0, 4), (0, 3), (0, 2), (0, 1), (0, 0)]),
        ("column", "leftToRight", [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]),
        ("column", "rightToLeft", [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]),
    ];
    for (layout, order, want) in cells {
        // One row, 20 pt apart; a 10 pt square 10 pt into each artboard.
        let mut s = new_doc(json!({"width": 100, "height": 50, "artboards": 5}));
        let art: Vec<u64> = boards(&s)
            .iter()
            .map(|r| {
                s.execute("shape.rectangle", &json!({"x": r.x0 + 10.0, "y": r.y0 + 10.0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap()
            })
            .collect();
        let before = boards(&s);
        let undo = s.doc().unwrap().history.undo.len();
        let p = json!({"layout": layout, "order": order, "columns": 2, "spacing": 10});
        let r = s.execute("artboard.rearrange", &p).unwrap();
        let (rows, cols) = want.iter().fold((0, 0), |(r, c), (wr, wc)| (r.max(wr + 1), c.max(wc + 1)));
        assert_eq!(r, json!({"artboards": 5, "rows": rows, "columns": cols}), "{layout} {order}");
        let got: Vec<(f64, f64)> = boards(&s).iter().map(|r| (r.x0, r.y0)).collect();
        let cells: Vec<(f64, f64)> = want.iter().map(|&(r, c)| (f64::from(c) * 110.0, f64::from(r) * 60.0)).collect();
        assert_eq!(got, cells, "{layout} {order}");
        assert!(boards(&s).iter().all(|r| r.width() == 100.0 && r.height() == 50.0));
        for (id, (x, y)) in art.iter().zip(&got) {
            let b = s.doc().unwrap().doc.node(NodeId(*id)).unwrap().geometric_bounds().unwrap();
            assert_eq!((b.x0, b.y0), (x + 10.0, y + 10.0), "{layout} {order}: the art moves with its artboard");
        }
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(boards(&s), before);
    }
}

/// Art centered on an artboard's right or bottom edge moves with that artboard, and art centered on
/// the edge two artboards share, away from its ends, moves with the artboard to the right of it.
#[test]
fn rearrange_moves_art_centered_on_an_artboard_edge() {
    let mut s = new_doc(json!({"width": 100, "height": 50, "artboards": 2, "artboardLayout": {"layout": "row", "spacing": 0}}));
    assert_eq!(boards(&s), [Rect::new(0.0, 0.0, 100.0, 50.0), Rect::new(100.0, 0.0, 200.0, 50.0)]);
    let mut line = |x1: f64, y1: f64, x2: f64, y2: f64| {
        NodeId(s.execute("shape.line", &json!({"x1": x1, "y1": y1, "x2": x2, "y2": y2})).unwrap()["id"].as_u64().unwrap())
    };
    let (bottom, right, shared) = (line(10.0, 50.0, 40.0, 50.0), line(200.0, 10.0, 200.0, 40.0), line(100.0, 10.0, 100.0, 40.0));
    // Right to left, artboard 0 moves 100 pt right and artboard 1 moves 100 pt left.
    s.execute("artboard.rearrange", &json!({"layout": "row", "order": "rightToLeft", "spacing": 0})).unwrap();
    assert_eq!(boards(&s), [Rect::new(100.0, 0.0, 200.0, 50.0), Rect::new(0.0, 0.0, 100.0, 50.0)]);
    let x0 = |id: NodeId| s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
    assert_eq!((x0(bottom), x0(right), x0(shared)), (110.0, 100.0, 0.0));
}

/// `byColumn: true` is Grid by Column; without `moveArtwork` the art stays; bad values are errors.
#[test]
fn rearrange_aliases_and_rejects_bad_values() {
    let mut s = new_doc(json!({"width": 100, "height": 50, "artboards": 3}));
    let id = s.execute("shape.rectangle", &json!({"x": 130, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("artboard.rearrange", &json!({"byColumn": true, "columns": 2, "spacing": 0, "moveArtwork": false})).unwrap();
    assert_eq!(boards(&s).iter().map(|r| (r.x0, r.y0)).collect::<Vec<_>>(), [(0.0, 0.0), (0.0, 50.0), (100.0, 0.0)]);
    assert_eq!(s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap().x0, 130.0, "the art stays");
    let before = boards(&s);
    for p in [json!({"layout": "diagonal"}), json!({"order": "upward"}), json!({"layout": 3}), json!({"order": true})] {
        assert!(s.execute("artboard.rearrange", &p).is_err(), "{p}");
    }
    assert_eq!(boards(&s), before, "nothing moved");
}

/// Rearrange All Artboards with the artboards already where it puts them, or with no artboard,
/// records no undo step: the step to redo stays, the document stays unmodified, and the reply is the
/// same.
#[test]
fn rearrange_that_moves_no_artboard_records_no_undo_step() {
    let mut s = new_doc(json!({"width": 100, "height": 50, "artboards": 3}));
    let p = json!({"columns": 2, "spacing": 5});
    let reply = s.execute("artboard.rearrange", &p).unwrap();
    let placed = boards(&s);
    // A rectangle drawn and undone leaves a step to redo, and the document is marked saved.
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let history = |s: &Session| {
        let st = s.doc().unwrap();
        (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
    };
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false));
    assert_eq!(s.execute("artboard.rearrange", &p).unwrap(), reply);
    assert_eq!((history(&s), boards(&s)), (before, placed));
    // A file can hold no artboard.
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).artboards.clear();
    s.doc_mut().unwrap().mark_saved();
    let before = history(&s);
    assert_eq!(s.execute("artboard.rearrange", &p).unwrap(), json!({"artboards": 0, "rows": 1, "columns": 1}));
    assert_eq!(history(&s), before);
}

/// A session whose New Document profiles are in a fresh folder for test `name` (not made yet).
fn with_profiles(name: &str) -> (Session, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("vc-newdoc-profiles-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    s.attach_new_doc_profiles(dir.clone());
    (s, dir)
}

fn swatch_names(s: &Session) -> Vec<String> {
    s.doc().unwrap().doc.swatches.iter().map(|w| w.name.clone()).collect()
}

/// Edit the Print profile: Brand Red, the Brand Box style, the Badge and Logo symbols (Logo's art an
/// embedded image) and an image left on its layer, saved.
fn edit_print_profile(s: &mut Session, dir: &std::path::Path) {
    // Edit writes the shipped profile and opens it.
    s.execute("file.newProfiles.edit", &json!({"name": "print"})).unwrap();
    assert!(dir.join("Print.vectorcraft").is_file());
    assert_eq!(s.doc().unwrap().doc.color_mode, ColorMode::Cmyk);
    s.execute("swatch.new", &json!({"name": "Brand Red", "color": "#cc2200"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    s.execute("graphicStyle.new", &json!({"name": "Brand Box"})).unwrap();
    s.execute("symbol.new", &json!({"name": "Badge"})).unwrap();
    // One embedded image becomes a symbol's art, another stays on the profile's layer.
    let png = |rgb: [u8; 3]| {
        let mut out = vec![];
        image::RgbaImage::from_pixel(4, 4, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        vectorcraft_format::base64_encode(&out)
    };
    s.execute("file.place", &json!({"name": "red.png", "dataBase64": png([255, 0, 0]), "link": false})).unwrap();
    s.execute("symbol.new", &json!({"name": "Logo"})).unwrap();
    s.execute("file.place", &json!({"name": "blue.png", "dataBase64": png([0, 0, 255]), "link": false})).unwrap();
    assert_eq!(s.doc().unwrap().doc.images.len(), 2);
    s.execute("document.save", &json!({})).unwrap();
    assert_eq!(s.execute("file.newProfiles", &json!({})).unwrap()["profiles"][2]["customized"], true);
}

#[test]
fn an_edited_profile_starts_the_documents_of_its_category() {
    let (mut s, dir) = with_profiles("edited");
    let listed = s.execute("file.newProfiles", &json!({})).unwrap();
    let names: Vec<&str> = listed["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Mobile", "Web", "Print", "Film & Video", "Art & Illustration", "Branding", "Social"]);
    assert!(listed["profiles"].as_array().unwrap().iter().all(|p| p["customized"] == false), "{listed}");
    edit_print_profile(&mut s, &dir);

    let r = s.execute("file.new", &json!({"preset": "A4"})).unwrap();
    assert_eq!(r["profile"], "Print", "{r}");
    assert!(swatch_names(&s).contains(&"Brand Red".to_string()));
    let d = &s.doc().unwrap().doc;
    assert!(d.graphic_styles.iter().any(|g| g.name == "Brand Box"));
    let badge = d.symbols.iter().find(|y| y.name == "Badge").expect("the profile's symbol");
    assert!(d.symbols.iter().any(|y| y.name == "Logo"));
    assert_eq!(d.images.len(), 1, "only the symbol's image comes along");
    // The profile's art stays behind: one empty layer, with an id the symbol's art doesn't use.
    assert_eq!(d.layers.len(), 1);
    assert!(d.layers[0].children().is_none_or(Vec::is_empty));
    let mut symbol_ids = vec![];
    badge.art.walk(&mut |n| symbol_ids.push(n.id));
    assert!(!symbol_ids.contains(&d.layers[0].id), "{symbol_ids:?}");
    assert_eq!(d.artboards[0].rect.width().round(), 595.0);
    let red = d.swatches.iter().find(|w| w.name == "Brand Red").cloned().unwrap();
    assert!(matches!(red.paint, vectorcraft_color::Paint::Solid { color: vectorcraft_color::Color::Cmyk { .. }, .. }), "{:?}", red.paint);
    // In RGB the profile's colors are converted, as Document Color Mode converts them.
    s.execute("file.new", &json!({"preset": "A4", "colorMode": "rgb"})).unwrap();
    let red = s.doc().unwrap().doc.swatches.iter().find(|w| w.name == "Brand Red").cloned().expect("converted, not dropped");
    assert!(matches!(red.paint, vectorcraft_color::Paint::Solid { color: vectorcraft_color::Color::Rgb { .. }, .. }), "{:?}", red.paint);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_profile_follows_the_preset_access_and_reset() {
    let (mut s, dir) = with_profiles("choice");
    edit_print_profile(&mut s, &dir);
    // A request that may not read the folder gets the defaults, and the reason; edit is refused
    // before anything is written.
    let elsewhere = dir.with_extension("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let roots = crate::file_access::AutomationRoots::new(&[elsewhere.as_path()], &[dir.as_path()]).unwrap().unwrap();
    let r = crate::file_access::confine(Some(&roots), || s.execute("file.new", &json!({"preset": "A4"}))).unwrap();
    assert!(r.get("profile").is_none() && r["profileError"]["profile"] == "Print", "{r}");
    // The reason does not say where the folder is.
    let reason = r["profileError"]["reason"].as_str().unwrap_or_default();
    let folder = dir.file_name().unwrap().to_string_lossy().to_string();
    assert!(!reason.is_empty() && !reason.contains(&folder), "{reason}");
    assert!(!swatch_names(&s).contains(&"Brand Red".to_string()));
    // Nothing tells such a request which profiles have files.
    let r = crate::file_access::confine(Some(&roots), || s.execute("file.new", &json!({"preset": "Web 1920×1080"}))).unwrap();
    assert_eq!(r["profileError"]["profile"], "Web", "{r}");
    let listed = crate::file_access::confine(Some(&roots), || s.execute("file.newProfiles", &json!({}))).unwrap();
    assert!(listed["profiles"].as_array().unwrap().iter().all(|p| p.get("customized").is_none() && p.get("path").is_none()), "{listed}");
    assert!(crate::file_access::confine(Some(&roots), || s.execute("file.newProfiles.edit", &json!({"name": "Web"}))).is_err());
    assert!(!dir.join("Web.vectorcraft").exists());
    let _ = std::fs::remove_dir_all(&elsewhere);
    // A saved preset made from a Print preset keeps the profile.
    s.execute("file.newPresets.save", &json!({"name": "Zine", "preset": "A5"})).unwrap();
    assert_eq!(presets(&mut s, "Saved")[0]["profile"], "Print");
    assert_eq!(s.execute("file.new", &json!({"preset": "Zine"})).unwrap()["profile"], "Print");

    // Other categories, no preset and `profile: null` start with the defaults.
    for p in [json!({"preset": "Web 1920×1080"}), json!({}), json!({"preset": "A4", "profile": null})] {
        let r = s.execute("file.new", &p).unwrap();
        assert!(r.get("profile").is_none() && !swatch_names(&s).contains(&"Brand Red".to_string()), "{p}: {r}");
    }
    // `profile` picks one for any settings.
    assert_eq!(s.execute("file.new", &json!({"width": 300, "height": 300, "profile": "PRINT"})).unwrap()["profile"], "Print");

    // Reset deletes the file, once; the shipped profile applies again.
    assert_eq!(s.execute("file.newProfiles.reset", &json!({"name": "Print"})).unwrap(), json!({"name": "Print", "reset": true}));
    assert_eq!(s.execute("file.newProfiles.reset", &json!({"name": "Print"})).unwrap(), json!({"name": "Print", "reset": false}));
    let r = s.execute("file.new", &json!({"preset": "A4"})).unwrap();
    assert!(r.get("profile").is_none() && !swatch_names(&s).contains(&"Brand Red".to_string()), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The new document's links find their files from the profile's file, as a document opened from
/// that file does; a document without a profile has no such file.
#[test]
fn the_new_document_links_from_the_profile_file() {
    let (mut s, dir) = with_profiles("links");
    edit_print_profile(&mut s, &dir);
    assert_eq!(s.execute("file.new", &json!({"preset": "A4"})).unwrap()["profile"], "Print");
    let file = dir.join("Print.vectorcraft").to_string_lossy().to_string();
    assert_eq!(s.doc().unwrap().links_from.as_deref(), Some(file.as_str()));
    s.execute("file.new", &json!({"preset": "Web 1920×1080"})).unwrap();
    assert_eq!(s.doc().unwrap().links_from, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A color profile the profile's file carries is installed when this machine lacks it, as opening
/// the file installs it, so the new document keeps it and a save carries it; one that can't be used
/// is a warning.
#[test]
fn a_profile_brings_the_color_profile_its_file_carries() {
    use vectorcraft_color::cms;
    let (mut s, dir) = with_profiles("icc");
    let builtin = cms::profiles().into_iter().find(|p| p.kind == cms::ProfileKind::Rgb && p.builtin && p.name != cms::SRGB).unwrap();
    let icc = cms::icc_bytes(&builtin.name).unwrap().to_vec();
    let name = format!("Profile RGB {}", std::process::id());
    cms::register_icc(&icc, Some(name.clone())).unwrap();
    let bytes_of = |v: &Value| vectorcraft_format::base64_decode(v.as_str().expect("base64")).unwrap();
    s.execute("file.new", &json!({"preset": "Web 1920×1080", "profile": null})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).color_profiles.rgb = Some(name.clone());
    let file = String::from_utf8(bytes_of(&s.execute("file.saveCopy", &json!({"compress": false})).unwrap()["dataBase64"])).unwrap();
    cms::unregister_icc(&name);
    // A profile made on another machine names a color profile this one lacks.
    let elsewhere = format!("Profile RGB elsewhere {}", std::process::id());
    assert!(cms::profile(&elsewhere).is_none());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Web.vectorcraft"), file.replace(&name, &elsewhere)).unwrap();
    let r = s.execute("file.new", &json!({"preset": "Web 1920×1080"})).unwrap();
    assert!(r["profile"] == "Web" && r.get("warnings").is_none(), "{r}");
    assert_eq!(cms::profile(&elsewhere).map(|p| p.kind), Some(cms::ProfileKind::Rgb));
    assert_eq!(s.doc().unwrap().doc.color_profiles.rgb.as_deref(), Some(elsewhere.as_str()));
    let saved: Value = serde_json::from_slice(&bytes_of(&s.execute("file.saveCopy", &json!({"compress": false})).unwrap()["dataBase64"])).unwrap();
    assert_eq!(vectorcraft_format::base64_decode(saved["profiles"][&elsewhere]["data"].as_str().unwrap()).unwrap(), icc);
    cms::unregister_icc(&elsewhere);
    // Damaged profile data starts the document with a warning.
    let broken = format!("Profile RGB broken {}", std::process::id());
    let mut v: Value = serde_json::from_str(&file.replace(&name, &broken)).unwrap();
    v["profiles"][&broken]["data"] = json!("AAAA");
    std::fs::write(dir.join("Web.vectorcraft"), serde_json::to_vec(&v).unwrap()).unwrap();
    let r = s.execute("file.new", &json!({"preset": "Web 1920×1080"})).unwrap();
    assert!(r["profile"] == "Web" && r["warnings"][0].as_str().unwrap_or_default().contains(&broken), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_damaged_or_oversized_profile_leaves_the_defaults_and_is_reported() {
    let (mut s, dir) = with_profiles("damaged");
    s.execute("file.new", &json!({"preset": "Letter", "profile": null})).unwrap();
    let defaults = swatch_names(&s);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Print.vectorcraft"), b"not a document").unwrap();
    let r = s.execute("file.new", &json!({"preset": "Letter"})).unwrap();
    assert_eq!(r["profileError"]["profile"], "Print", "{r}");
    assert!(!r["profileError"]["reason"].as_str().unwrap_or("").is_empty(), "{r}");
    assert!(r.get("profile").is_none());
    assert_eq!(swatch_names(&s), defaults);
    // Past the limit the file is not read at all (a sparse file of that length).
    let big = std::fs::File::create(dir.join("Web.vectorcraft")).unwrap();
    big.set_len(crate::cmd::newdoc::MAX_PROFILE_BYTES + 1).unwrap();
    drop(big);
    let r = s.execute("file.new", &json!({"preset": "Web 1920×1080"})).unwrap();
    assert!(r["profileError"]["reason"].as_str().unwrap_or("").contains("larger than"), "{r}");
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect.width(), 1920.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn profile_names_are_checked_and_editing_needs_the_folder() {
    let mut s = Session::new();
    for p in [json!({"profile": "Posters"}), json!({"profile": 5}), json!({"profile": ""})] {
        let e = s.execute("file.new", &p).unwrap_err().to_string();
        assert!(e.contains("no New Document profile called") && e.contains("Film & Video"), "{p}: {e}");
    }
    assert!(s.execute("file.newProfiles.edit", &json!({"name": "Posters"})).is_err());
    for cmd in ["file.newProfiles.edit", "file.newProfiles.reset"] {
        let e = s.execute(cmd, &json!({"name": "Print"})).unwrap_err().to_string();
        assert!(e.contains("preferences folder"), "{cmd}: {e}");
    }
    // Without a folder every profile is the shipped one and documents start with the defaults.
    let r = s.execute("file.newProfiles", &json!({})).unwrap();
    assert!(r["folder"].is_null() && r["profiles"][0].get("path").is_none(), "{r}");
    let r = s.execute("file.new", &json!({"preset": "A4"})).unwrap();
    assert!(r.get("profile").is_none() && r.get("profileError").is_none(), "{r}");
}

#[test]
fn the_new_layer_takes_an_id_no_profile_art_has() {
    // A damaged profile whose symbol art has the id the new layer would otherwise take.
    let settings = crate::cmd::newdoc::DocSettings::default();
    let mut profile = settings.document();
    let next = settings.document().peek_next_id().max(profile.peek_next_id());
    let art = std::sync::Arc::new(vectorcraft_doc::Node::group(vectorcraft_doc::NodeId(next), vec![]));
    profile.symbols.push(vectorcraft_doc::Symbol { name: "Badge".into(), art, dynamic: false });
    let d = settings.document_from_profile(profile);
    assert_eq!(d.layers.len(), 1);
    assert_ne!(d.layers[0].id, vectorcraft_doc::NodeId(next));
    // Ids given out later skip the symbol's art too.
    assert!(d.peek_next_id() > d.layers[0].id.0 && d.peek_next_id() > next);
}
