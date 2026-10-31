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

/// Export for Screens writes the characters Windows does not allow in a file name (`<`, `>`, `"`,
/// `|`, `?`, `*`, control characters) as `-` in a pattern's text and in the names its tokens stand
/// for, on every platform.
#[test]
fn a_pattern_writes_the_characters_windows_does_not_allow_as_dashes() {
    let mut s = session(1);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "a|b?\"c\u{7}"})).unwrap();
    let r = export(&mut s, json!({"name": "<name>*<x>", "formats": [{"format": "svg"}]}));
    assert_eq!(names(&r), ["a-b--c---x-.svg"]);
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

/// Document formats write each artboard as a document of its own: one artboard, its art and
/// not the other artboards' (#3868).
#[test]
fn document_formats_write_one_artboard_each() {
    let mut s = session(2);
    let second = s.doc().unwrap().doc.artboards[1].rect;
    s.execute("shape.ellipse", &json!({"x": second.x0 + 5.0, "y": 5, "width": 10, "height": 10})).unwrap();
    let r = export(&mut s, json!({"formats": [{"format": "vectorcraft"}, {"format": "ai"}, {"format": "eps"}]}));
    let n = names(&r);
    assert_eq!(n, ["Artboard-1.vectorcraft", "Artboard-1.ai", "Artboard-1.eps", "Artboard-2.vectorcraft", "Artboard-2.ai", "Artboard-2.eps"]);
    let all = files(&r);
    for (name, bytes) in &all {
        let mut t = Session::new();
        t.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
        let d = &t.doc().unwrap().doc;
        assert_eq!(d.artboards.len(), 1, "{name}: that artboard alone");
        let objects: usize = d.layers.iter().filter_map(|l| l.children()).map(|c| c.len()).sum();
        assert_eq!(objects, 1, "{name}: only its own artboard's art");
    }
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

/// Export presets go to a file and come back from one: a name in use gets a number unless
/// `replace`, a built-in name is never replaced, and a file in another format or with a setting an
/// export refuses adds nothing.
#[test]
fn export_presets_export_and_import() {
    let mut s = session(2);
    assert!(s.execute("document.exportPresets.export", &json!({})).is_err(), "nothing saved to export");
    let web = json!({"name": "ACME-<name>", "folders": "item", "formats": [{"format": "jpg", "quality": 80}]});
    s.execute("document.exportPresets.save", &json!({"name": "Web JPG", "settings": web})).unwrap();
    let out = s.execute("document.exportPresets.export", &json!({})).unwrap();
    assert_eq!(out["count"], 1);
    let data = out["data"].as_str().unwrap().to_string();
    assert_eq!(serde_json::from_str::<Value>(&data).unwrap()["format"], "vcexportpresets");
    // Into a session with no saved presets: the preset as it was saved, and it exports.
    let mut other = session(2);
    let r = other.execute("document.exportPresets.import", &json!({ "data": data })).unwrap();
    assert_eq!(r["imported"], json!(["Web JPG"]));
    assert_eq!(other.prefs.screen_export_presets, s.prefs.screen_export_presets);
    assert_eq!(names(&export(&mut other, json!({"exportPreset": "Web JPG", "range": "2"}))), ["Artboard-2/ACME-Artboard 2.jpg"]);
    // Again: a second copy with a number; with replace, the saved one changes.
    let r = other.execute("document.exportPresets.import", &json!({ "data": data })).unwrap();
    assert_eq!(r["imported"], json!(["Web JPG 2"]));
    s.execute("document.exportPresets.save", &json!({"name": "Web JPG", "settings": {"formats": [{"format": "png"}]}})).unwrap();
    let changed = s.execute("document.exportPresets.export", &json!({"names": ["Web JPG"]})).unwrap()["data"].clone();
    let r = other.execute("document.exportPresets.import", &json!({"data": changed, "replace": true})).unwrap();
    assert_eq!(r["imported"], json!(["Web JPG"]));
    let saved: Vec<(&str, &Value)> =
        other.prefs.screen_export_presets.iter().map(|p| (p.name.as_str(), &p.settings["formats"][0]["format"])).collect();
    assert_eq!(saved, [("Web JPG", &json!("png")), ("Web JPG 2", &json!("jpg"))]);
    // A built-in preset exports by name and comes back under a new name; the built-in one stays.
    let path = std::env::temp_dir().join(format!("vc-export-presets-{}.vcexportpresets", std::process::id()));
    let path = path.to_string_lossy().to_string();
    let out = s.execute("document.exportPresets.export", &json!({"names": ["Logo Package"], "path": path})).unwrap();
    assert_eq!((out["path"].as_str(), out["count"].as_u64()), (Some(path.as_str()), Some(1)));
    let r = other.execute("document.exportPresets.import", &json!({"path": path, "replace": true})).unwrap();
    assert_eq!(r["imported"], json!(["Logo Package 2"]));
    let builtin = screens::builtin_presets();
    assert_eq!(other.prefs.screen_export_presets.last().map(|p| &p.settings), builtin.first().map(|p| &p.settings));
    let _ = std::fs::remove_file(&path);
    // Refused files add nothing.
    let before = other.prefs.screen_export_presets.clone();
    let preset = |settings: Value| {
        json!({"format": "vcexportpresets", "presets": [{"name": "Fine", "settings": {"formats": [{"format": "png"}]}}, {"name": "Bad", "settings": settings}]}).to_string()
    };
    for bad in [
        json!({"data": "{\"format\": \"vcflattener\", \"presets\": []}"}),
        json!({"data": "not json"}),
        json!({"data": preset(json!({"formats": [{"format": "txt"}]}))}),
        json!({"data": preset(json!({"folders": "sideways"}))}),
        json!({"dataBase64": "%%"}),
        json!({}),
    ] {
        assert!(other.execute("document.exportPresets.import", &bad).is_err(), "{bad}");
    }
    assert_eq!(other.prefs.screen_export_presets, before);
    assert!(s.execute("document.exportPresets.export", &json!({"names": ["Never saved"]})).is_err());
    // `names` that is not a list of names is an error, not every saved preset.
    for bad in [json!({"names": "Web JPG"}), json!({"names": [3]}), json!({"names": {"Web JPG": true}})] {
        assert!(s.execute("document.exportPresets.export", &bad).is_err(), "{bad}");
    }
    // One file imports at most 1,000 presets; names repeated in a file are numbered in turn.
    let many = |n: usize| {
        let one = json!({"name": "Same", "settings": {"formats": [{"format": "png"}]}});
        json!({"format": "vcexportpresets", "presets": vec![one; n]}).to_string()
    };
    assert!(other.execute("document.exportPresets.import", &json!({"data": many(1001)})).is_err(), "more than 1,000");
    assert_eq!(other.prefs.screen_export_presets, before);
    let r = other.execute("document.exportPresets.import", &json!({"data": many(1000)})).unwrap();
    let imported = r["imported"].as_array().unwrap();
    assert_eq!((imported.len(), imported.first(), imported.last()), (1000, Some(&json!("Same")), Some(&json!("Same 1000"))));
    // An export writes no file that an import refuses: at most 1,000 presets.
    assert!(other.prefs.screen_export_presets.len() > 1000);
    assert!(other.execute("document.exportPresets.export", &json!({})).is_err(), "every saved preset");
    let names: Vec<String> = other.prefs.screen_export_presets.iter().take(1001).map(|p| p.name.clone()).collect();
    assert!(other.execute("document.exportPresets.export", &json!({ "names": names })).is_err(), "1,001 names");
    let r = other.execute("document.exportPresets.export", &json!({ "names": names[..1000] })).unwrap();
    assert_eq!(r["count"], 1000);
}

#[test]
fn export_presets_import_replace_numbers_duplicates_from_one_file() {
    let mut source = session(1);
    source.execute("document.exportPresets.save", &json!({"name": "A", "settings": {"formats": [{"format": "png"}]}})).unwrap();
    let data = source.execute("document.exportPresets.export", &json!({"names": ["A", "A"]})).unwrap()["data"].clone();

    let mut empty = session(1);
    let reply = empty.execute("document.exportPresets.import", &json!({"data": data, "replace": true})).unwrap();
    assert_eq!(reply["imported"], json!(["A", "A 2"]));
    assert_eq!(empty.prefs.screen_export_presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["A", "A 2"]);

    let mut existing = session(1);
    existing.execute("document.exportPresets.save", &json!({"name": "A", "settings": {"formats": [{"format": "jpg"}]}})).unwrap();
    let reply = existing.execute("document.exportPresets.import", &json!({"data": data, "replace": true})).unwrap();
    assert_eq!(reply["imported"], json!(["A", "A"]));
    assert_eq!(existing.prefs.screen_export_presets.len(), 1);
    assert_eq!(existing.prefs.screen_export_presets[0].settings["formats"][0]["format"], "png");
}

#[test]
fn export_presets_import_refuses_oversize_input_from_every_source() {
    let mut s = session(1);
    let good = s.execute("document.exportPresets.export", &json!({"names": ["Logo Package"]})).unwrap()["data"].as_str().unwrap().to_string();
    let oversized = format!("{good}{}", " ".repeat((4 << 20) + 1));
    let path = std::env::temp_dir().join(format!("vc-oversize-export-presets-{}.vcexportpresets", std::process::id()));
    std::fs::write(&path, &oversized).unwrap();
    for params in [
        json!({"path": path.to_string_lossy()}),
        json!({"data": oversized}),
        json!({"dataBase64": vectorcraft_format::base64_encode(oversized.as_bytes())}),
    ] {
        let error = s.execute("document.exportPresets.import", &params).unwrap_err().to_string();
        assert!(error.contains("larger than 4194304 bytes"), "{error}");
        assert!(s.prefs.screen_export_presets.is_empty());
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn export_presets_import_accepts_wrapped_base64_near_size_limit() {
    const LIMIT: usize = 4 << 20;
    let empty = json!({
        "format": "vcexportpresets",
        "presets": [{"name": "Wrapped", "settings": {"formats": [{"format": "png"}], "prefix": ""}}],
    });
    let prefix = "x".repeat(LIMIT - 1 - empty.to_string().len());
    let file = json!({
        "format": "vcexportpresets",
        "presets": [{"name": "Wrapped", "settings": {"formats": [{"format": "png"}], "prefix": prefix}}],
    })
    .to_string();
    assert_eq!(file.len(), LIMIT - 1);
    let encoded = vectorcraft_format::base64_encode(file.as_bytes());
    let wrapped = encoded.as_bytes().chunks(76).map(|line| std::str::from_utf8(line).unwrap()).collect::<Vec<_>>().join("\n");
    assert!(wrapped.len() > LIMIT.div_ceil(3) * 4);
    assert_eq!(vectorcraft_format::base64_decode(&wrapped).as_deref(), Some(file.as_bytes()));

    let mut s = session(1);
    let reply = s.execute("document.exportPresets.import", &json!({"dataBase64": wrapped})).unwrap();
    assert_eq!(reply["imported"], json!(["Wrapped"]));
    assert_eq!(s.prefs.screen_export_presets[0].settings["prefix"], prefix);
}

#[test]
fn export_presets_export_refuses_oversize_output() {
    let mut s = session(1);
    let prefix = "x".repeat((4 << 20) + 1);
    s.execute("document.exportPresets.save", &json!({"name": "Large", "settings": {"prefix": prefix}})).unwrap();

    let error = s.execute("document.exportPresets.export", &json!({})).unwrap_err().to_string();
    assert!(error.contains("larger than 4194304 bytes"), "{error}");

    let path = std::env::temp_dir().join(format!("vc-oversize-preset-output-{}.vcexportpresets", std::process::id()));
    assert!(!path.exists());
    let error = s.execute("document.exportPresets.export", &json!({"path": path.to_string_lossy()})).unwrap_err().to_string();
    assert!(error.contains("larger than 4194304 bytes"), "{error}");
    assert!(!path.exists());
}

/// An imported preset whose name is taken and ends in a number gets the next free number, not a
/// second one; any other taken name gets the first free number.
#[test]
fn an_imported_numbered_name_counts_on() {
    let taken = ["Logo Package", "Logo Package 2", "Logo Package 4", "Web", "Web 2", "2", "Max 18446744073709551615"];
    let name = |base: &str| import_name(base, |n| taken.contains(&n));
    assert_eq!(name("Print"), "Print");
    assert_eq!(name("Logo Package 3"), "Logo Package 3");
    assert_eq!(name("Logo Package"), "Logo Package 3");
    assert_eq!(name("Logo Package 2"), "Logo Package 3");
    assert_eq!(name("Logo Package 4"), "Logo Package 5");
    assert_eq!(name("Web"), "Web 3");
    // A number alone, or one past the largest, is not counted on.
    assert_eq!(name("2"), "2 2");
    assert_eq!(name("Max 18446744073709551615"), "Max 18446744073709551615 2");
    // Through the command: a built-in preset imported twice is Logo Package 2, then Logo Package 3.
    let mut s = session(1);
    let data = s.execute("document.exportPresets.export", &json!({"names": ["Logo Package"]})).unwrap()["data"].clone();
    assert_eq!(s.execute("document.exportPresets.import", &json!({ "data": data })).unwrap()["imported"], json!(["Logo Package 2"]));
    let data = s.execute("document.exportPresets.export", &json!({"names": ["Logo Package 2"]})).unwrap()["data"].clone();
    assert_eq!(s.execute("document.exportPresets.import", &json!({ "data": data })).unwrap()["imported"], json!(["Logo Package 3"]));
}

/// A preset name, imported or saved, keeps at most 255 characters, as an asset name does.
#[test]
fn export_preset_names_are_capped() {
    let mut s = session(1);
    let long = "é".repeat(300);
    let capped = "é".repeat(255);
    let file = json!({"format": "vcexportpresets", "presets": [{"name": long, "settings": {"formats": [{"format": "png"}]}}]}).to_string();
    let r = s.execute("document.exportPresets.import", &json!({ "data": file })).unwrap();
    assert_eq!(r["imported"], json!([capped]));
    let r = s.execute("document.exportPresets.save", &json!({"name": format!("  {}", "w".repeat(400)), "settings": {}})).unwrap();
    assert_eq!(r["name"], "w".repeat(255));
    let names: Vec<usize> = s.prefs.screen_export_presets.iter().map(|p| p.name.chars().count()).collect();
    assert_eq!(names, [255, 255]);
    // The same long name imported again gets a number within the limit, and saving under the name
    // it got changes that preset alone.
    let r = s.execute("document.exportPresets.import", &json!({ "data": file })).unwrap();
    let again = r["imported"][0].as_str().unwrap().to_string();
    assert!(again.chars().count() <= 255 && again != capped && again.ends_with(" 2"), "{again}");
    s.execute("document.exportPresets.save", &json!({"name": again, "settings": {"formats": [{"format": "svg"}]}})).unwrap();
    let formats: Vec<Option<&str>> =
        s.prefs.screen_export_presets.iter().map(|p| p.settings.get("formats").and_then(|f| f[0]["format"].as_str())).collect();
    assert_eq!(formats, [Some("png"), None, Some("svg")]);
}
