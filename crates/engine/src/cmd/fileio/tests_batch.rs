//! Batch exports through Export for Screens (#1935): a file name pattern (the document's and the
//! item's names, a counter, the date), folders by format, by item or both, layers and the selection
//! as what is exported, and document formats (VectorCraft, Illustrator, EPS) as rows.

use serde_json::{Value, json};

use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// A document of `artboards` 40×30 pt artboards side by side with a 20 pt square on the first.
fn session(artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 30, "artboards": artboards})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 20})).unwrap();
    s
}

fn export(s: &mut Session, p: Value) -> Value {
    s.execute("document.exportForScreens", &p).unwrap()
}

fn files(r: &Value) -> Vec<(String, Vec<u8>)> {
    r["files"].as_array().unwrap().iter().map(|f| (f["name"].as_str().unwrap().to_string(), b64(f))).collect()
}

fn names(r: &Value) -> Vec<String> {
    files(r).into_iter().map(|(n, _)| n).collect()
}

/// The request in #1935: SVG, PNG, PDF and PDF with printer's marks for artboards 1–3, named
/// `COMPANY-Logo-<filename>-####-YYYYMMDD`, sorted into a folder per extension.
#[test]
fn the_logo_package_of_the_request() {
    let mut s = session(4);
    let r = export(
        &mut s,
        json!({
            "range": "1-3",
            "name": "COMPANY-Logo-<filename>-####-YYYYMMDD.ext",
            "date": "2026-10-16",
            "folders": "format",
            "formats": [
                {"format": "svg"},
                {"format": "png", "scale": "2x", "suffix": ""},
                {"format": "pdf"},
                {"format": "pdf", "suffix": "-print", "marks": {"trim": true, "registration": true, "colorBars": true, "pageInfo": true}}
            ]
        }),
    );
    let n = names(&r);
    assert_eq!(n.len(), 12, "{n:?}");
    for (i, number) in ["0001", "0002", "0003"].iter().enumerate() {
        let stem = format!("COMPANY-Logo-Untitled-1-{number}-20261016");
        for want in [format!("svg/{stem}.svg"), format!("png/{stem}.png"), format!("pdf/{stem}.pdf"), format!("pdf/{stem}-print.pdf")] {
            assert!(n.contains(&want), "artboard {}: {want} in {n:?}", i + 1);
        }
    }
    // The printed PDF has room for its marks: a bigger page than the plain one.
    let f = files(&r);
    let page = |name: &str| {
        let bytes = &f.iter().find(|(n, _)| n == name).unwrap().1;
        let d = vectorcraft_pdf::import(bytes).unwrap();
        let a = d.artboards.first().unwrap().rect;
        (a.width(), a.height())
    };
    let plain = page("pdf/COMPANY-Logo-Untitled-1-0001-20261016.pdf");
    let print = page("pdf/COMPANY-Logo-Untitled-1-0001-20261016-print.pdf");
    assert!(print.0 > plain.0 && print.1 > plain.1, "{plain:?} vs {print:?}");
}

#[test]
fn folders_by_item_and_by_item_and_format() {
    let mut s = session(2);
    let rows = json!([{"format": "png"}, {"format": "svg"}]);
    let by_item = names(&export(&mut s, json!({"folders": "item", "formats": rows})));
    assert_eq!(by_item, ["Artboard-1/Artboard-1.png", "Artboard-1/Artboard-1.svg", "Artboard-2/Artboard-2.png", "Artboard-2/Artboard-2.svg"]);
    let both = names(&export(&mut s, json!({"folders": "itemFormat", "formats": rows})));
    assert_eq!(both[0], "Artboard-1/png/Artboard-1.png");
    let flat = names(&export(&mut s, json!({"folders": "none", "subfolders": true, "formats": rows})));
    assert_eq!(flat[0], "Artboard-1.png", "folders wins over subfolders");
    // Without folders, subfolders keeps its meaning (a folder per row).
    let rows_mode = names(&export(&mut s, json!({"subfolders": true, "formats": rows})));
    assert_eq!(rows_mode[0], "1x/Artboard-1.png");
}

#[test]
fn the_counter_counts_items_or_files_from_any_start() {
    let mut s = session(2);
    let rows = json!([{"format": "png"}, {"format": "svg"}]);
    let items = names(&export(&mut s, json!({"name": "logo-###", "counterStart": 9, "formats": rows})));
    assert_eq!(items, ["logo-009.png", "logo-009.svg", "logo-010.png", "logo-010.svg"]);
    let per_file = names(&export(&mut s, json!({"name": "logo-#", "counterPer": "file", "formats": rows})));
    assert_eq!(per_file, ["logo-1.png", "logo-2.svg", "logo-3.png", "logo-4.svg"]);
    // A pattern that names different files alike numbers them instead of dropping one.
    let same = names(&export(&mut s, json!({"name": "logo", "formats": [{"format": "png"}]})));
    assert_eq!(same, ["logo.png", "logo-2.png"]);
    // The item's name, its place and the row's size and suffix.
    let parts = names(&export(
        &mut s,
        json!({"name": "<index>_<name>_<scale><suffix>_<format>", "range": "2", "formats": [{"format": "png", "scale": "2x"}]}),
    ));
    assert_eq!(parts, ["1_Artboard 2_2x@2x_png.png"]);
}

#[test]
fn layers_and_the_selection_are_exported_by_name() {
    let mut s = session(1);
    let first_layer = s.doc().unwrap().doc.layers[0].id.0;
    let top = s.execute("layer.new", &json!({"name": "Dark Mode"})).unwrap()["id"].as_u64().unwrap();
    let circle = s.execute("shape.ellipse", &json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let r = export(&mut s, json!({"layers": [first_layer, top], "formats": [{"format": "svg"}]}));
    assert_eq!(names(&r), ["Layer-1.svg", "Dark-Mode.svg"]);
    // The selection: one file of it all, or a file per object.
    let square = s.doc().unwrap().doc.layers[0].children().unwrap()[0].id.0;
    s.execute("select.set", &json!({"ids": [square, circle]})).unwrap();
    assert_eq!(names(&export(&mut s, json!({"selection": "one", "formats": [{"format": "png"}]}))), ["Selection.png"]);
    let each = names(&export(&mut s, json!({"selection": "each", "formats": [{"format": "png"}]})));
    assert_eq!(each.len(), 2, "{each:?}");
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("document.exportForScreens", &json!({"selection": "one"})).is_err(), "nothing selected");
}

/// Document formats write each artboard as a document of its own: one artboard, its art.
#[test]
fn document_formats_write_one_artboard_each() {
    let mut s = session(2);
    let r = export(&mut s, json!({"formats": [{"format": "vectorcraft"}, {"format": "ai"}, {"format": "eps"}]}));
    let n = names(&r);
    assert_eq!(n, ["Artboard-1.vectorcraft", "Artboard-1.ai", "Artboard-1.eps", "Artboard-2.vectorcraft", "Artboard-2.ai", "Artboard-2.eps"]);
    let native = &files(&r)[0].1;
    let mut t = Session::new();
    t.execute("document.open", &json!({"name": "a.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(native)})).unwrap();
    assert_eq!(t.doc().unwrap().doc.artboards.len(), 1, "that artboard alone");
}

#[test]
fn bad_batch_params_are_refused() {
    let mut s = session(1);
    for bad in [
        json!({"folders": "byColour"}),
        json!({"counterPer": "page"}),
        json!({"date": "16/10/2026"}),
        json!({"selection": "some"}),
        json!({"selection": "one", "fullDocument": true}),
        json!({"layers": [99999]}),
        json!({"layers": [1], "assets": [1]}),
    ] {
        assert!(s.execute("document.exportForScreens", &bad).is_err(), "{bad}");
    }
}

/// Export presets: the built-in ones, saving (only what a preset keeps), using one under other
/// params, replacing, deleting, and keeping them with the preferences.
#[test]
fn export_presets_save_use_and_delete() {
    let mut s = session(2);
    let list = s.execute("document.exportPresets.list", &json!({})).unwrap();
    let built_in: Vec<&str> =
        list["presets"].as_array().unwrap().iter().filter(|p| p["builtIn"] == true).map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(built_in, ["Logo Package", "Print-Ready PDF"]);
    // The Logo Package: SVG, PNG 1x and 2x, PDF and PDF with marks, a folder per format.
    let r = export(&mut s, json!({"exportPreset": "Logo Package", "range": "1"}));
    assert_eq!(
        names(&r),
        [
            "svg/Untitled-1-Artboard 1.svg",
            "png/Untitled-1-Artboard 1.png",
            "png/Untitled-1-Artboard 1@2x.png",
            "pdf/Untitled-1-Artboard 1.pdf",
            "pdf/Untitled-1-Artboard 1-print.pdf"
        ]
    );

    let settings = json!({"name": "ACME-<name>-##", "folders": "item", "formats": [{"format": "jpg", "quality": 80}], "range": "2", "assets": [1]});
    let saved = s.execute("document.exportPresets.save", &json!({"name": "Web JPG", "settings": settings})).unwrap();
    assert_eq!(saved["created"], true);
    assert!(saved["settings"].get("range").is_none() && saved["settings"].get("assets").is_none(), "which art is not kept: {saved}");
    let r = export(&mut s, json!({"exportPreset": "Web JPG", "range": "2"}));
    assert_eq!(names(&r), ["Artboard-2/ACME-Artboard 2-01.jpg"]);
    // The call's own params win over the preset's.
    let r = export(&mut s, json!({"exportPreset": "Web JPG", "range": "2", "folders": "none"}));
    assert_eq!(names(&r), ["ACME-Artboard 2-01.jpg"]);
    // Saving again replaces it; the preferences keep it.
    let again = s.execute("document.exportPresets.save", &json!({"name": "Web JPG", "settings": {"formats": [{"format": "png"}]}})).unwrap();
    assert_eq!(again["created"], false);
    let prefs = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(prefs["screenExportPresets"][0]["name"], "Web JPG");
    s.execute("document.exportPresets.delete", &json!({"name": "Web JPG"})).unwrap();
    assert!(s.execute("document.exportForScreens", &json!({"exportPreset": "Web JPG"})).is_err());
    // Built-in names stay; bad settings are refused when saved.
    for bad in [
        ("document.exportPresets.save", json!({"name": "Logo Package", "settings": {}})),
        ("document.exportPresets.save", json!({"name": "", "settings": {}})),
        ("document.exportPresets.save", json!({"name": "X", "settings": {"formats": [{"format": "txt"}]}})),
        ("document.exportPresets.save", json!({"name": "X", "settings": {"folders": "sideways"}})),
        ("document.exportPresets.save", json!({"name": "X"})),
        ("document.exportPresets.delete", json!({"name": "Logo Package"})),
        ("document.exportPresets.delete", json!({"name": "Never saved"})),
    ] {
        assert!(s.execute(bad.0, &bad.1).is_err(), "{bad:?}");
    }
}
