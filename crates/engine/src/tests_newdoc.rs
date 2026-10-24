//! File → New (`file.new`, `file.newPresets*`): artboard layouts, orientation, bleed, colour mode,
//! raster effects resolution, background, presets, Recent and Saved.

use serde_json::{Value, json};
use vectorcraft_doc::{Background, ColorMode, Unit};
use vectorcraft_geom::Rect;

use super::*;

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
            (163830.0, 163830.0, 1.0, 1.0e5, vec![72.0, 72.0, 72.0, 0.0]),
            (10.0, 10.0, 1000.0, 0.0, vec![0.0; 4]),
        ]
    );
    assert_eq!(read(&presets(&mut s, "Recent")[0]), (100.0, 50.0, 3.0, 0.0, vec![0.0; 4]));
    // file.new makes each one with those values.
    s.execute("file.new", &json!({"preset": "Low"})).unwrap();
    assert_eq!((boards(&s), s.doc().unwrap().doc.setup.bleed), (vec![Rect::new(0.0, 0.0, 1.0, 1.0)], [0.0, 0.0, 0.0, 9.0]));
    s.execute("file.new", &json!({"preset": "High"})).unwrap();
    assert_eq!((boards(&s), s.doc().unwrap().doc.setup.bleed), (vec![Rect::new(0.0, 0.0, 163830.0, 163830.0)], [72.0, 72.0, 72.0, 0.0]));
    s.execute("file.new", &json!({"preset": "Many"})).unwrap();
    assert_eq!(boards(&s).len(), 1000);
    let custom = presets(&mut s, "Recent").into_iter().find(|p| p["name"] == "Custom").unwrap();
    s.execute("file.new", &custom).unwrap();
    assert_eq!(boards(&s).iter().map(|r| r.x0).collect::<Vec<_>>(), [0.0, 100.0, 200.0], "side by side, not overlapping");
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
