//! The params descriptions name the keys their commands accept (#1442).

use serde_json::json;

use crate::cmd::params::{UNCHECKED, check_effect_params, check_keys, check_params, doc_keys};
use crate::{EngineError, Session, command_specs, find_command};

fn keys(doc: &str) -> Vec<&str> {
    doc_keys(doc).unwrap().keys
}

#[test]
fn a_description_names_its_keys() {
    assert_eq!(keys("{} → {id}"), Vec::<&str>::new());
    assert_eq!(
        keys("{x, y, width?: pt=612|\"210 mm\", sides=6, name?|title?: a name (default: Untitled-N)} → {index}"),
        ["x", "y", "width", "sides", "name", "title"]
    );
    // Brackets, quotes and backticks keep their commas and bars; a nested object isn't listed.
    assert_eq!(
        keys("{at: [x, y] (page, not view), stops: [{position: pt, leader?: \", \"}], csv?: a `|, ` in a label is a break, on?: bool}"),
        ["at", "stops", "csv", "on"]
    );
    // Alternatives after a note are keys when they have their own `?` or `:`; else values.
    assert_eq!(
        keys("{steps?: n | distance?: pt | smooth?: bool, spacing?: smooth|steps|distance, path? | data?: text | dataBase64?}"),
        ["steps", "distance", "smooth", "spacing", "path", "data", "dataBase64"]
    );
    let more = doc_keys("{name, …settings as file.new, …the format's options}").unwrap();
    assert_eq!((more.keys, more.more), (vec!["name"], vec!["settings as file.new", "the format's options"]));
    for bad in ["same as paint.setFill", "{a, b", "{opacity? 0..1}", "{path | name+dataBase64}", "{kind?: Group, Path or Type}"] {
        assert!(doc_keys(bad).is_err(), "{bad}");
    }
}

/// Every command's description reads as a params object, so none is left unchecked by accident.
#[test]
fn every_description_parses() {
    let bad: Vec<String> = command_specs().iter().filter_map(|c| doc_keys(c.params).err().map(|e| format!("{}: {e}", c.id))).collect();
    assert!(bad.is_empty(), "{} descriptions don't parse:\n{}", bad.len(), bad.join("\n"));
    let stale: Vec<_> = UNCHECKED.iter().filter(|id| find_command(id).is_none()).collect();
    assert!(stale.is_empty() && UNCHECKED.is_sorted(), "UNCHECKED: sorted ids of commands; not commands: {stale:?}");
}

fn refusal(id: &str, params: serde_json::Value) -> String {
    match check_params(id, &params) {
        Err(EngineError::BadParams { cmd, msg }) if cmd == id => msg,
        other => panic!("{id} {params}: {other:?}"),
    }
}

#[test]
fn an_unknown_key_is_named_with_the_nearest_accepted_one() {
    assert_eq!(refusal("file.new", json!({"widht": 300})), "unknown parameter `widht`; did you mean `width`?");
    assert_eq!(refusal("file.new", json!({"Width": 300})), "unknown parameter `Width`; did you mean `width`?");
    assert_eq!(refusal("select.all", json!({"zzz": 1})), "unknown parameter `zzz`; the command takes no parameters");
    assert_eq!(
        refusal("shape.line", json!({"x1": 0, "y1": 0, "x2": 5, "y2": 5, "colour": "red"})),
        "unknown parameter `colour`; the command takes: x1, y1, x2, y2, fill"
    );
    for (id, ok) in [
        ("file.new", json!({"width": 300, "title": "a", "artboardLayout": {"anything": 1}})),
        ("file.new", json!({})),
        ("file.new", json!(null)),
        // Keys listed by reference, and any key where the description says so.
        ("file.newPresets.save", json!({"name": "a", "width": 300})),
        ("document.export", json!({"format": "png", "styling": "style"})),
        // Not a command: running it says so.
        ("no.such", json!({"x": 1})),
    ] {
        assert!(check_params(id, &ok).is_ok(), "{id} {ok}");
    }
    assert!(check_keys("x", "{a, …as file.new}", &json!({"a": 1, "width": 2})).is_ok());
    assert!(check_keys("x", "{a, …as file.new}", &json!({"b": 1})).is_err());
}

/// An export's options: a key no format reads is refused, at the top level and in the PDF objects,
/// under each command that takes them.
#[test]
fn export_options_refuse_a_key_no_format_reads() {
    for id in ["document.export", "document.exportSelection", "document.serialize"] {
        assert_eq!(
            refusal(id, json!({"format": "pdf", "standard": "pdfX4", "noSuchOption": true})),
            "unknown parameter `noSuchOption`; no format takes it (see document.formats)"
        );
        assert_eq!(refusal(id, json!({"format": "pdf", "standardd": "pdfX4"})), "unknown parameter `standardd`; did you mean `standard`?");
        assert_eq!(
            refusal(id, json!({"format": "pdf", "options": {"standard": "pdfX4"}})),
            "unknown parameter `options`; no format takes it (see document.formats)"
        );
    }
    // A command that writes one format refuses another format's option, and says which format.
    for (id, why) in [
        ("document.exportPdf", "PDF doesn't take it"),
        ("document.pdfSettings", "PDF doesn't take it"),
        ("pdf.preset.save", "PDF doesn't take it"),
        ("document.exportEps", "EPS doesn't take it"),
        ("document.exportDxf", "DXF doesn't take it"),
    ] {
        for key in ["noSuchOption", "quality"] {
            assert_eq!(refusal(id, json!({key: 1})), format!("unknown parameter `{key}`; {why} (see document.formats)"));
        }
    }
    assert_eq!(
        refusal("document.exportEps", json!({"flattener": {"balanse": 50}})),
        "unknown parameter `flattener.balanse`; did you mean `flattener.balance`?"
    );
    assert!(refusal("document.exportDxf", json!({"flattener": {}})).contains("DXF doesn't take it"));
    assert!(refusal("document.exportEps", json!({"marks": {}})).contains("EPS doesn't take it"));
    for id in ["document.pdfSettings", "pdf.preset.save"] {
        assert!(refusal(id, json!({"marks": {"useDocumentBleed": true}})).starts_with("unknown parameter `marks.useDocumentBleed`;"), "{id}");
    }
    for (given, message) in [
        (json!({"marks": {"useDocumentBleed": true}}), "unknown parameter `marks.useDocumentBleed`; `marks` takes:"),
        (json!({"bleed": {"useDocumen": true}}), "unknown parameter `bleed.useDocumen`; did you mean `bleed.useDocument`?"),
        (json!({"output": {"intent": "x"}}), "unknown parameter `output.intent`;"),
        (json!({"advanced": {"outlineTxt": true}}), "unknown parameter `advanced.outlineTxt`; did you mean `advanced.outlineText`?"),
        (json!({"security": {"openPasswrd": "x"}}), "unknown parameter `security.openPasswrd`; did you mean `security.openPassword`?"),
        (json!({"compression": {"color": {"dpi": 300}}}), "unknown parameter `compression.color.dpi`;"),
        (json!({"flattener": {"balanse": 50}}), "unknown parameter `flattener.balanse`; did you mean `flattener.balance`?"),
    ] {
        for id in ["document.export", "document.exportPdf"] {
            let got = refusal(id, given.clone());
            assert!(got.starts_with(message), "{id} {given}: {got}");
        }
    }
}

/// The check passes each key `document.formats` lists, each key a command's own description names,
/// and each field of the PDF settings and the flattener options. A key an encoder reads that none
/// of these names is outside this test.
#[test]
fn export_options_pass_every_listed_key() {
    use crate::cmd::fileio::{FORMATS, dxf, eps, pdf::OPTIONS};
    for f in FORMATS.iter().filter(|f| f.write) {
        for o in f.options {
            let params = json!({"format": f.id, o.name: null});
            assert!(check_params("document.export", &params).is_ok(), "{params}");
        }
    }
    for (id, options) in [("document.exportEps", eps::OPTIONS), ("document.exportDxf", dxf::OPTIONS), ("document.exportPdf", OPTIONS)] {
        for o in options {
            assert!(check_params(id, &json!({o.name: null})).is_ok(), "{id} {}", o.name);
        }
    }
    for id in [
        "document.export",
        "document.exportSelection",
        "document.serialize",
        "document.exportPdf",
        "document.pdfSettings",
        "pdf.preset.save",
        "document.exportEps",
        "document.exportDxf",
    ] {
        // An export reads these two when it writes over the file a lossy document came from. The
        // PDF settings and preset commands take them through document.exportPdf's description.
        check_params(id, &json!({"path": "a", "acknowledgeLoss": true})).unwrap_or_else(|e| panic!("{id}: {e}"));
        let keys = doc_keys(find_command(id).unwrap().params).unwrap().keys;
        let refused: Vec<&str> = keys.into_iter().filter(|key| check_params(id, &json!({*key: null})).is_err()).collect();
        // `document.exportPdf`'s description explains the page boxes as `BleedBox = …` between its
        // keys, which reads as two more keys.
        let prose = if id == "document.exportPdf" { vec!["BleedBox", "MediaBox"] } else { vec![] };
        assert_eq!(refused, prose, "{id} names and refuses these");
    }
    let settings = serde_json::to_value(vectorcraft_pdf::PdfSettings::default()).unwrap();
    for key in settings.as_object().unwrap().keys() {
        assert!(OPTIONS.iter().any(|o| o.name == key), "document.formats doesn't list the PDF option `{key}`");
    }
    let mut every = settings;
    every["flattener"] = serde_json::to_value(crate::cmd::flatten::FlattenOptions::default()).unwrap();
    every["security"]["openPassword"] = json!("a");
    every["security"]["permissionsPassword"] = json!("b");
    every["preset"] = json!("High Quality Print");
    for id in ["document.export", "document.exportPdf", "document.pdfSettings", "pdf.preset.save"] {
        check_params(id, &every).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

const SAVES: [&str; 4] = ["document.save", "file.saveAs", "file.saveCopy", "file.saveAsTemplate"];

/// A save refuses a key no save format reads, beside the path, in `options` and in the PDF and
/// SVG objects of either, under each save command.
#[test]
fn save_options_refuse_a_key_no_save_format_reads() {
    let why = "no save format takes it (see file.formatOptions)";
    for id in SAVES {
        assert_eq!(refusal(id, json!({"path": "a.pdf", "noSuchOption": true})), format!("unknown parameter `noSuchOption`; {why}"));
        assert_eq!(
            refusal(id, json!({"path": "a.pdf", "options": {"noSuchOption": true}})),
            format!("unknown parameter `options.noSuchOption`; {why}")
        );
        assert_eq!(refusal(id, json!({"compres": true})), "unknown parameter `compres`; did you mean `compress`?");
        assert_eq!(refusal(id, json!({"options": {"compres": true}})), "unknown parameter `options.compres`; did you mean `options.compress`?");
        // An export's own key is not a save's.
        assert_eq!(refusal(id, json!({"path": "a.png", "quality": 80})), format!("unknown parameter `quality`; {why}"));
        for (given, message) in [
            (json!({"path": "a.pdf", "marks": {"useDocumentBleed": true}}), "unknown parameter `marks.useDocumentBleed`; `marks` takes:"),
            (json!({"path": "a.pdf", "options": {"marks": {"x": 1}}}), "unknown parameter `options.marks.x`; `options.marks` takes:"),
            (json!({"path": "a.ai", "bleed": {"useDocumen": true}}), "unknown parameter `bleed.useDocumen`; did you mean `bleed.useDocument`?"),
            // The `svg` object is checked when the save writes no SVG too.
            (json!({"path": "a.pdf", "svg": {"noSuchOption": true}}), "unknown parameter `svg.noSuchOption`; `svg` takes:"),
            (
                json!({"path": "a.pdf", "options": {"svg": {"minfy": true}}}),
                "unknown parameter `options.svg.minfy`; did you mean `options.svg.minify`?",
            ),
        ] {
            let got = refusal(id, given.clone());
            assert!(got.starts_with(message), "{id} {given}: {got}");
        }
        // A key that another format a save writes reads still passes: the save leaves it out.
        for given in [
            json!({"path": "a.svg", "compress": true}),
            json!({"path": "a.pdf", "options": {"preview": true, "styling": "style"}}),
            json!({"path": "a.vectorcraft", "standard": "pdfX4", "marks": {"trim": true}}),
            json!({"path": "a.ai", "acknowledgeLoss": true, "modified": null, "svg": {"minify": true}}),
            json!({"path": "a.svg", "svg": {"styling": "css", "artboard": 0, "useArtboards": true}}),
        ] {
            check_params(id, &given).unwrap_or_else(|e| panic!("{id} {given}: {e}"));
        }
    }
}

/// A save passes each option of each format it writes, beside the path and in `options`, and
/// each key its description names.
#[test]
fn save_options_pass_every_listed_key() {
    use crate::cmd::fileio::{FORMATS, SAVE_FORMATS, TEMPLATE_FORMATS};
    for f in FORMATS.iter().filter(|f| SAVE_FORMATS.contains(&f.id) || TEMPLATE_FORMATS.contains(&f.id)) {
        for o in f.options {
            for params in [json!({"format": f.id, o.name: null}), json!({"format": f.id, "options": {o.name: null}})] {
                for id in SAVES {
                    check_params(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
                }
            }
        }
    }
    for id in SAVES {
        let keys = doc_keys(find_command(id).unwrap().params).unwrap().keys;
        let refused: Vec<&str> = keys.into_iter().filter(|key| check_params(id, &json!({*key: null})).is_err()).collect();
        assert_eq!(refused, Vec::<&str>::new(), "{id} names and refuses these");
    }
}

/// Export for Screens refuses a key it doesn't take, and in its rows and settings a key no format
/// reads, by the row's path; `document.exportPresets.save` checks the rows of its settings.
#[test]
fn export_for_screens_refuses_a_key_no_format_reads() {
    let id = "document.exportForScreens";
    assert!(
        refusal(id, json!({"folder": "out", "noSuchOption": true})).starts_with("unknown parameter `noSuchOption`; the command takes: artboard, ")
    );
    assert_eq!(refusal(id, json!({"subfolder": true})), "unknown parameter `subfolder`; did you mean `subfolders`?");
    let no_format = "no format takes it (see document.formats)";
    for (given, message) in [
        (json!({"formats": [{"format": "png", "noSuchOption": 1}]}), format!("unknown parameter `formats[0].noSuchOption`; {no_format}")),
        (
            json!({"formats": [{"format": "png"}, {"format": "pdf", "marks": {"useDocumentBleed": true}}]}),
            "unknown parameter `formats[1].marks.useDocumentBleed`; `formats[1].marks` takes:".into(),
        ),
        (
            json!({"formats": [{"format": "pdf", "security": {"openPasswrd": "x"}}]}),
            "unknown parameter `formats[0].security.openPasswrd`; did you mean `formats[0].security.openPassword`?".into(),
        ),
        (
            json!({"formats": [{"format": "eps", "flattener": {"balanse": 50}}]}),
            "unknown parameter `formats[0].flattener.balanse`; did you mean `formats[0].flattener.balance`?".into(),
        ),
        (json!({"formats": [{"format": "jpg", "sufix": "@2x"}]}), "unknown parameter `formats[0].sufix`; did you mean `formats[0].suffix`?".into()),
        (json!({"settings": {"pnj": {}}}), "unknown parameter `settings.pnj`; did you mean `settings.png`?".into()),
        (json!({"settings": {"dxf": {}}}), "unknown parameter `settings.dxf`; `settings` takes: ".into()),
        (
            json!({"settings": {"png": {"bckground": "white"}}}),
            "unknown parameter `settings.png.bckground`; did you mean `settings.png.background`?".into(),
        ),
        (json!({"settings": {"pdf": {"bleed": {"top": 9, "inner": 1}}}}), "unknown parameter `settings.pdf.bleed.inner`;".into()),
    ] {
        let got = refusal(id, given.clone());
        assert!(got.starts_with(&message), "{given}: {got}");
    }
    let save = "document.exportPresets.save";
    assert_eq!(
        refusal(save, json!({"name": "a", "settings": {"formats": [{"format": "png", "noSuchOption": 1}]}})),
        format!("unknown parameter `settings.formats[0].noSuchOption`; {no_format}")
    );
    assert!(
        refusal(save, json!({"name": "a", "settings": {"settings": {"pdf": {"marks": {"x": 1}}}}}))
            .starts_with("unknown parameter `settings.settings.pdf.marks.x`;")
    );
    assert_eq!(
        refusal(save, json!({"name": "a", "settings": {"formtas": [{"format": "png"}]}})),
        "unknown parameter `settings.formtas`; did you mean `settings.formats`?"
    );
    assert!(
        refusal(save, json!({"name": "a", "settings": {"noSuchOption": true}}))
            .starts_with("unknown parameter `settings.noSuchOption`; `settings` takes: artboard, ")
    );
    // Export Assets takes the same rows and settings, and stores them.
    for asset in ["assets.export", "assets.settings.set"] {
        assert_eq!(
            refusal(asset, json!({"formats": [{"format": "png", "noSuchOption": 1}]})),
            format!("unknown parameter `formats[0].noSuchOption`; {no_format}")
        );
        assert!(refusal(asset, json!({"settings": {"pdf": {"marks": {"x": 1}}}})).starts_with("unknown parameter `settings.pdf.marks.x`;"));
        check_params(asset, &json!({"formats": [{"format": "jpg", "scale": "2x", "quality": 80}]})).unwrap();
    }
    // An exporter plug-in's `params` in a row (its format is a plug-in's: tests/plugin_export.rs).
    check_params(id, &json!({"formats": [{"format": "png", "params": {}}]})).unwrap();
    // Keys the preset leaves out are still Export for Screens params.
    check_params(save, &json!({"name": "a", "settings": {"zip": true, "formats": [{"format": "jpg", "quality": 80}]}})).unwrap();
    // The encoders read Include Artboard Fill and Appearance of Black from a row or a format's
    // settings.
    for given in [
        json!({"formats": [{"format": "png", "includeArtboardFill": false, "richBlack": true}]}),
        json!({"settings": {"png": {"includeArtboardFill": false, "richBlack": true}}}),
    ] {
        check_params(id, &given).unwrap_or_else(|e| panic!("{given}: {e}"));
    }
}

/// Export for Screens passes each key its description names, each option of a format in a row or
/// in settings, and the built-in presets; `settings` takes the formats a row can write.
#[test]
fn export_for_screens_passes_every_listed_key() {
    use crate::cmd::fileio::{FORMATS, check_screen_settings, screen_export_builtin_presets};
    let id = "document.exportForScreens";
    let keys = doc_keys(find_command(id).unwrap().params).unwrap().keys;
    assert!(keys.len() > 20, "{keys:?}");
    // The keys the check takes are the ones the description names.
    let (mut named, mut taken) = (keys.clone(), crate::cmd::fileio::SCREEN_PARAM_KEYS.to_vec());
    named.sort_unstable();
    taken.sort_unstable();
    assert_eq!(named, taken);
    let refused: Vec<&str> = keys.into_iter().filter(|key| check_params(id, &json!({*key: null})).is_err()).collect();
    assert_eq!(refused, Vec::<&str>::new(), "{id} names and refuses these");
    check_params(id, &json!({"artboard": 0, "openLocation": true})).unwrap();
    let s = Session::new();
    for f in FORMATS.iter().filter(|f| f.write) {
        for o in f.options {
            let row = json!({"formats": [{"format": "png", "scale": 2, "suffix": "", "folder": "x", o.name: null}]});
            check_params(id, &row).unwrap_or_else(|e| panic!("{row}: {e}"));
        }
        let written = check_screen_settings(&s, &json!({"formats": [{"format": f.id}]})).is_ok();
        let settings = json!({"settings": {f.id: {}}});
        assert_eq!(check_params(id, &settings).is_ok(), written, "{settings}");
    }
    for preset in screen_export_builtin_presets() {
        let settings = serde_json::Value::Object(preset.settings);
        check_params(id, &settings).unwrap_or_else(|e| panic!("{}: {e}", preset.name));
        check_params("document.exportPresets.save", &json!({"name": "a", "settings": settings})).unwrap();
    }
}

/// A saved row loses the keys Export for Screens refuses in it, inside its object options too, and
/// keeps the others.
#[test]
fn a_saved_row_drops_the_keys_no_format_reads() {
    use crate::cmd::params::drop_unread_row_options;
    let serde_json::Value::Object(mut row) = json!({
        "format": "pdf", "scale": "1x", "noSuchOption": 1, "params": {"x": 1},
        "marks": {"useDocumentBleed": true, "trim": true}, "preset": "High Quality Print",
        "security": {"passwordRequired": true, "noSuchLock": 1}
    }) else {
        panic!("an object")
    };
    drop_unread_row_options(&mut row);
    assert_eq!(
        serde_json::Value::Object(row.clone()),
        json!({
            "format": "pdf", "scale": "1x", "params": {"x": 1}, "marks": {"trim": true}, "preset": "High Quality Print",
            "security": {"passwordRequired": true}
        })
    );
    check_params("document.exportForScreens", &json!({"formats": [row]})).unwrap();
    // Remembered security whose passwords were left out, with one given again.
    let security = json!({"passwordRequired": true, "openPassword": "x"});
    for id in ["document.exportForScreens", "assets.export", "assets.settings.set"] {
        check_params(id, &json!({"formats": [{"format": "pdf", "security": security}], "settings": {"pdf": {"security": security}}})).unwrap();
    }
}

/// `stroke.set` names every key it reads (its own, `item`, and `ids`/`id`/`target`), so a
/// misnamed one is refused instead of answering `objects: 1` with nothing changed.
#[test]
fn stroke_set_refuses_an_unknown_key() {
    assert!(!UNCHECKED.contains(&"stroke.set"));
    let e = refusal("stroke.set", json!({"ids": [7], "width": 0.5}));
    assert!(e.starts_with("unknown parameter `width`") && e.contains("weight"), "{e}");
    for ok in [
        json!({"ids": [7], "weight": 0.5}),
        json!({"id": 7, "weight": 1, "cap": "round", "join": "bevel", "miterLimit": 4, "align": "inside"}),
        json!({"dash": [6, 3], "dashOffset": 1, "alignDashes": true, "startArrow": "Arrow", "endArrow": null, "arrowAlign": "tip"}),
        json!({"profile": "lens", "item": null, "target": "contents"}),
        // `item_target` reads `aligned` for an explicit item.
        json!({"item": 1, "aligned": true, "weight": 2}),
        // What document.inspect reports in strokeOptions reads back unchanged.
        json!({"widthPoints": null, "gradientMode": "within"}),
    ] {
        assert!(check_params("stroke.set", &ok).is_ok(), "{ok}");
    }
}

/// `object.setLiveShape` names every key it reads, and each of its `items` takes `id` and
/// `corners` alone: another key, at the top level, in an item or in a batch step, is refused
/// instead of being ignored.
#[test]
fn set_live_shape_refuses_an_unknown_key() {
    const ID: &str = "object.setLiveShape";
    assert!(!UNCHECKED.contains(&ID));
    let e = refusal(ID, json!({"id": 3, "path": [[0, 0], [1, 1]], "radius": 5}));
    assert!(e.starts_with("unknown parameter `path`; the command takes: ") && e.contains("relativeRadii"), "{e}");
    let e = refusal(ID, json!({"radiuss": 5}));
    assert_eq!(e, "unknown parameter `radiuss`; did you mean `radius`?");
    let e = refusal(ID, json!({"items": [{"id": 3, "corners": [0]}, {"id": 4, "corner": [0]}], "radius": 5}));
    assert_eq!(e, "unknown parameter `items[1].corner`; did you mean `items[1].corners`?");
    let e = refusal(ID, json!({"items": [{"id": 3, "radius": 5}]}));
    assert_eq!(e, "unknown parameter `items[0].radius`; the command takes: items[0].id, items[0].corners");
    let step = |params| json!({"commands": [{"command": "select.all", "params": {}}, {"command": ID, "params": params}]});
    for (bad, key) in [(json!({"path": "x"}), "path"), (json!({"items": [{"id": 3, "kind": "chamfer"}]}), "items[0].kind")] {
        match check_params("command.batch", &step(bad.clone())) {
            Err(EngineError::BadParams { cmd, msg }) => {
                assert!(cmd == ID && msg.starts_with(&format!("unknown parameter `{key}`")), "{bad}: {cmd} {msg}")
            }
            other => panic!("{bad}: {other:?}"),
        }
    }
    for ok in [
        json!({"items": [{"id": 3, "corners": [0, 2]}, {"id": 4}], "radius": 5}),
        json!({"corners": [0], "kind": "chamfer", "radius": 2, "relativeRadii": false}),
        json!({"id": 3, "radius": 5}),
        json!({"ids": [3], "sides": 6, "polygonAngle": 15, "polygonRadius": 20, "sideLength": null, "makeSidesEqual": true}),
        json!({"pieStart": 30, "pieEnd": 90, "invertPie": true}),
        json!({"items": null}),
    ] {
        assert!(check_params(ID, &ok).is_ok(), "{ok}");
        assert!(check_params("command.batch", &step(ok.clone())).is_ok(), "{ok}");
    }
}

/// An object id given as something other than a whole number (`"5"`, `-1`) is refused as such,
/// naming its key, not as a missing id; an id left out is still a missing one.
#[test]
fn a_malformed_object_id_is_not_reported_as_missing() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let undo = |s: &Session| s.doc().unwrap().history.undo.len();
    let before = undo(&s);
    let points = json!([[0, 0], [10, 10], [20, 0]]);
    for bad in [json!("5"), json!(-1)] {
        let id = || json!({ "id": bad });
        let with = |extra: serde_json::Value| {
            let mut p = id();
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            p
        };
        for (cmd, p, key) in [
            ("stroke.widthPoint.set", id(), "id"),
            ("stroke.widthPoint.copy", id(), "id"),
            ("stroke.widthPoint.remove", id(), "id"),
            ("path.curvatureEdit", id(), "id"),
            ("path.removeAnchor", id(), "id"),
            ("path.convertAnchor", id(), "id"),
            ("path.reshapeSegment", id(), "id"),
            ("path.split", id(), "id"),
            ("path.appendAnchor", id(), "id"),
            ("path.close", id(), "id"),
            ("path.setHandle", id(), "id"),
            ("path.moveAnchor", id(), "id"),
            ("path.setAnchors", id(), "id"),
            ("path.reshape", id(), "id"),
            ("path.insertAnchor", id(), "id"),
            ("path.freehand", json!({"points": points, "extend": {"id": bad}}), "id"),
            ("path.freehand", json!({"points": points, "redraw": {"id": bad}}), "id"),
            ("document.node", id(), "id"),
            ("layer.target", id(), "id"),
            ("layer.setCurrent", id(), "id"),
            ("layer.selectAll", id(), "id"),
            ("layer.move", with(json!({"target": 1})), "id"),
            ("layer.move", json!({"ids": [1], "target": bad}), "target"),
            ("node.move", id(), "id"),
            ("node.move", json!({"id": 1, "parent": bad}), "parent"),
            ("text.reshapeArea", with(json!({"anchors": []})), "id"),
            ("text.reshapeArea", json!({"items": [{"id": bad, "anchors": []}]}), "id"),
            ("text.reshapePath", id(), "id"),
            ("text.setPathHandle", id(), "id"),
            ("text.editRange", id(), "id"),
            ("text.startList", id(), "id"),
            ("text.getRange", id(), "id"),
            ("text.setGlyphAlternate", id(), "id"),
            ("text.glyphAlternates", id(), "id"),
            ("text.touchType", id(), "id"),
            ("text.discardEmpty", id(), "id"),
            ("text.createInPath", json!({"path": bad}), "path"),
            ("object.envelope.setMeshPoint", id(), "id"),
            ("object.mesh.setPointColor", with(json!({"index": 0})), "id"),
            ("object.mesh.movePoint", with(json!({"index": 0, "x": 0, "y": 0})), "id"),
            ("object.mesh.movePoints", json!({"items": [{"id": bad, "index": 0}]}), "id"),
            ("object.mesh.addLine", with(json!({"x": 0, "y": 0})), "id"),
            ("object.mesh.deletePoint", with(json!({"index": 0})), "id"),
            ("object.slice.setRect", id(), "id"),
            ("appearance.transfer", json!({"source": bad, "target": 1}), "source"),
            ("appearance.transfer", json!({"source": 1, "target": bad}), "target"),
            ("appearance.copyFrom", json!({"source": bad}), "source"),
            ("livePaint.strokeEdge", json!({"point": [0, 0], "group": bad}), "group"),
            ("livePaint.fill", json!({"point": [0, 0], "group": bad}), "group"),
            ("livePaint.info", json!({"group": bad}), "group"),
            ("livePaint.info", id(), "id"),
            ("select.magicWand", id(), "id"),
        ] {
            let e = s.execute(cmd, &p).unwrap_err().to_string();
            // `layer.move` checks an `id` that stands in for `ids` before it runs.
            let named = e.contains(&format!("`{key}` must be a whole number")) || (cmd == "layer.move" && e.contains("invalid object id"));
            assert!(named, "{cmd} {p}: {e}");
        }
    }
    for cmd in ["path.close", "layer.target", "text.reshapePath", "object.mesh.movePoint", "select.magicWand"] {
        let e = s.execute(cmd, &json!({})).unwrap_err().to_string();
        assert!(e.contains("missing"), "{cmd}: {e}");
    }
    assert_eq!(undo(&s), before);
}

/// A required number given as something other than a number (`"40"`, `true`, `[]`, `{}`) is refused
/// as the wrong type, naming its key and showing the value as given, not as a missing number; one left
/// out or given as null is still a missing number. Neither changes the document or records an undo step.
#[test]
fn a_malformed_required_number_is_not_reported_as_missing() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let with = |base: &serde_json::Value, key: &str, v: Option<serde_json::Value>| {
        let mut p = base.clone();
        let o = p.as_object_mut().unwrap();
        match v {
            Some(v) => o.insert(key.into(), v),
            None => o.remove(key),
        };
        p
    };
    let cases = [
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}), "x"),
        ("edit.colors.saturate", json!({"intensity": 40}), "intensity"),
        ("object.envelope.setMeshPoint", json!({"id": 1, "index": 0, "x": 0, "y": 0}), "index"),
        ("object.envelope.setMeshPoint", json!({"id": 1, "index": 0, "x": 0, "y": 0}), "x"),
        ("object.mesh.movePoint", json!({"id": 1, "index": 0, "x": 0, "y": 0}), "y"),
        ("guide.add", json!({"pos": 40}), "pos"),
    ];
    for (cmd, base, key) in &cases {
        for (v, shown) in [(json!("40"), "\"40\""), (json!(true), "true"), (json!([]), "[]"), (json!({}), "{}")] {
            let p = with(base, key, Some(v));
            match s.execute(cmd, &p) {
                Err(EngineError::BadParams { cmd: c, msg }) => {
                    assert_eq!((c.as_str(), msg), (*cmd, format!("`{key}` must be a number, not {shown}")), "{p}");
                }
                other => panic!("{cmd} {p}: {other:?}"),
            }
        }
        for p in [with(base, key, None), with(base, key, Some(json!(null)))] {
            match s.execute(cmd, &p) {
                Err(EngineError::BadParams { cmd: c, msg }) => assert_eq!((c.as_str(), msg), (*cmd, format!("missing number `{key}`")), "{p}"),
                other => panic!("{cmd} {p}: {other:?}"),
            }
        }
    }
    assert_eq!(s.doc().unwrap().doc, doc);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // A whole JSON number is a number for a fractional parameter.
    s.execute("guide.add", &json!({"pos": 40})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
}

/// A required whole number given as something other than a whole number of 0 or more (`"3"`, `2.5`,
/// `-1`, `true`, `[]`, `{}`) is refused as the wrong type, naming its key, not as a missing integer; one
/// left out or given as null is still missing. An optional one left out or null keeps its default.
/// Neither changes the document or records an undo step.
#[test]
fn a_malformed_required_integer_is_not_reported_as_missing() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let with = |base: &serde_json::Value, key: &str, v: Option<serde_json::Value>| {
        let mut p = base.clone();
        let o = p.as_object_mut().unwrap();
        match v {
            Some(v) => o.insert(key.into(), v),
            None => o.remove(key),
        };
        p
    };
    let refused = |s: &mut Session, cmd: &str, p: &serde_json::Value, want: &str| match s.execute(cmd, p) {
        Err(EngineError::BadParams { cmd: c, msg }) => assert_eq!((c.as_str(), msg.as_str()), (cmd, want), "{p}"),
        other => panic!("{cmd} {p}: {other:?}"),
    };
    let ends = json!({"sourceDocument": 1, "grab": [0, 0], "at": [0, 0], "ids": [1]});
    let cases = [
        ("appearance.setItem", json!({"index": 0}), "index", "missing integer `index`"),
        ("path.removeAnchor", json!({"id": 1, "anchor": 0}), "anchor", "missing integer `anchor`"),
        ("path.split", json!({"id": 1, "anchor": 0}), "anchor", "need `anchor` or `segment` + `t`"),
        ("addon.pen.removePoint", json!({"anchor": 0}), "anchor", "missing integer `anchor`"),
        ("edit.dropSelection", ends, "sourceDocument", "missing integer `sourceDocument`"),
        ("document.activate", json!({"index": 0}), "index", "missing index"),
        ("document.move", json!({"to": 0}), "to", "missing `to`: a non-negative integer"),
    ];
    let malformed = [json!("3"), json!(2.5), json!(-1), json!(true), json!([]), json!({})];
    for (cmd, base, key, missing) in &cases {
        for v in &malformed {
            refused(&mut s, cmd, &with(base, key, Some(v.clone())), &format!("`{key}` must be a whole number"));
        }
        for p in [with(base, key, None), with(base, key, Some(json!(null)))] {
            refused(&mut s, cmd, &p, missing);
        }
    }
    // An optional `subpath` given as anything else is refused the same way; left out or null, it is
    // subpath 0 and the command goes on to read `anchor`.
    let base = json!({"id": 1, "anchor": "3"});
    for v in &malformed {
        refused(&mut s, "path.removeAnchor", &with(&base, "subpath", Some(v.clone())), "`subpath` must be a whole number");
    }
    for p in [with(&base, "subpath", None), with(&base, "subpath", Some(json!(null)))] {
        refused(&mut s, "path.removeAnchor", &p, "`anchor` must be a whole number");
    }
    assert_eq!(s.doc().unwrap().doc, doc);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // A whole JSON number is a whole number.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
}

/// A command that takes the keys of one that isn't checked yet isn't checked either.
#[test]
fn a_reference_to_an_unchecked_command_is_unchecked() {
    assert!(UNCHECKED.contains(&"paint.setFill") && !UNCHECKED.contains(&"paint.setStroke"), "pick another pair");
    assert!(check_params("paint.setStroke", &json!({"zzz": 1})).is_ok());
    assert!(check_keys("x", "{a, …as paint.setFill}", &json!({"zzz": 1})).is_ok());
}

#[test]
fn a_batch_checks_its_steps() {
    let step = |params| json!({"commands": [{"command": "select.all"}, {"command": "file.new", "params": params}]});
    assert!(check_params("command.batch", &step(json!({"width": 300}))).is_ok());
    match check_params("command.batch", &step(json!({"widht": 300}))) {
        Err(EngineError::BadParams { cmd, msg }) => {
            assert_eq!((cmd.as_str(), msg.as_str()), ("file.new", "unknown parameter `widht`; did you mean `width`?"))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(refusal("command.batch", json!({"steps": []})), "unknown parameter `steps`; the command takes: label, commands, artboard");
}

/// #1592: the `params` of an effect are held to the effect's own keys and types, where the effect
/// is named (`effect.apply`) and where the document says which it is (`effect.setParams`).
#[test]
fn effect_params_are_the_effects_own() {
    use vectorcraft_render::effects::effect_catalog;
    for e in effect_catalog() {
        doc_keys(e.params).unwrap_or_else(|why| panic!("{}: {why}", e.id));
        check_effect_params("effect.apply", e.id, &e.defaults).unwrap_or_else(|why| panic!("{}'s defaults: {why}", e.id));
    }
    let apply = |effect: &str, params| json!({"effect": effect, "params": params});
    assert_eq!(refusal("effect.apply", apply("blur.gaussian", json!({"blurr": 3}))), "unknown parameter `blurr`; the command takes: radius, linear");
    assert_eq!(
        refusal("appearance.addEffect", apply("stylize.dropShadow", json!({"opacty": 50}))),
        "unknown parameter `opacty`; did you mean `opacity`?"
    );
    assert_eq!(refusal("effect.apply", apply("blur.gaussian", json!({"radius": "big"}))), "`radius` must be a number");
    assert_eq!(refusal("effect.apply", apply("threeD.revolve", json!({"shade": 1}))), "`shade` must be true or false");
    assert_eq!(refusal("effect.apply", apply("threeD.revolve", json!({"edge": 1}))), "`edge` must be a string");
    // Out of range is the effect's to clamp; a key only its description names is one it takes;
    // a plug-in's are its manifest's; an unknown effect is the command's to report.
    for (effect, params) in [
        ("stylize.dropShadow", json!({"opacity": 500, "x": -3, "color": "#ff0000"})),
        ("stylize.dropShadow", json!({"color": [1, 0, 0]})),
        ("threeD.revolve", json!({"segments": 100000})),
        ("path.outlineStroke", json!({"width": 2})),
        ("plugin.anything", json!({"whatever": 1})),
        ("no.such", json!({"whatever": 1})),
    ] {
        assert!(check_params("effect.apply", &apply(effect, params.clone())).is_ok(), "{effect} {params}");
    }
    // `effect.setParams` names an index: the session knows the effect there.
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("effect.apply", &apply("blur.gaussian", json!({"radius": 2}))).unwrap();
    for (params, why) in [(json!({"blurr": 3}), "unknown parameter `blurr`"), (json!({"radius": true}), "`radius` must be a number")] {
        let e = s.execute("effect.setParams", &json!({"index": 0, "params": params})).unwrap_err().to_string();
        assert!(e.contains(why), "{e}");
    }
    s.execute("effect.setParams", &json!({"index": 0, "params": {"radius": 1e9}, "visible": false})).unwrap();
    // No effect at the index: the command says so, not the check.
    assert!(s.check_params("effect.setParams", &json!({"index": 7, "params": {"blurr": 3}})).is_ok());
}

/// An effect parameter that only the effect's params description names, with no dialog default,
/// is held to the type the effect reads: the earlier 3D Perspective and light, Scribble's
/// `variation` and an Outline Stroke's `width` are numbers, and a Gaussian Blur's `linear` is true
/// or false. The `style` of Crop Marks is `roman` or `japanese`. Every such key of every effect is
/// listed, or has a check of its own. A key left out keeps its default.
#[test]
fn an_effect_parameter_without_a_default_is_held_to_its_type() {
    use crate::cmd::params::undefaulted_type;
    use vectorcraft_render::effects::effect_catalog;
    // Read by checks of their own: a stored texture or a texture to load, an SVG filter, a Warp's
    // map and the style of Crop Marks.
    let own_check = |effect: &str, key: &str| match key {
        "textureImage" | "name" | "xml" => true,
        "version" => effect.starts_with("warp."),
        "style" => effect == "cropMarks",
        _ => false,
    };
    let mut unlisted = vec![];
    for e in effect_catalog() {
        for key in doc_keys(e.params).unwrap().keys {
            if e.defaults.get(key).is_none() && undefaulted_type(e.id, key).is_none() && !own_check(e.id, key) {
                unlisted.push(format!("{}.{key}", e.id));
            }
        }
    }
    assert!(unlisted.is_empty(), "parameters without a default or a listed type: {unlisted:?}");
    let apply = |effect: &str, params| json!({"effect": effect, "params": params});
    for (effect, params, why) in [
        ("threeD.revolve", json!({"perspective": true}), "`perspective` must be a number"),
        ("threeD.extrude", json!({"lightAzimuth": "30"}), "`lightAzimuth` must be a number"),
        ("threeD.rotate", json!({"lightElevation": null}), "`lightElevation` must be a number"),
        ("threeD.inflate", json!({"perspective": "50%"}), "`perspective` must be a number"),
        ("path.outlineStroke", json!({"width": "10 pt"}), "`width` must be a number"),
        ("stylize.scribble", json!({"variation": false}), "`variation` must be a number"),
        ("blur.gaussian", json!({"radius": 2, "linear": 1}), "`linear` must be true or false"),
        ("cropMarks", json!({"style": "bogus"}), "`style` must be roman or japanese"),
        ("cropMarks", json!({"style": 1}), "`style` must be roman or japanese"),
        ("cropMarks", json!({"style": null}), "`style` must be roman or japanese"),
    ] {
        assert_eq!(refusal("effect.apply", apply(effect, params.clone())), why, "{effect} {params}");
        assert_eq!(refusal("appearance.addEffect", apply(effect, params.clone())), why, "{effect} {params}");
    }
    for (effect, params) in [
        ("threeD.revolve", json!({"perspective": 40, "lightAzimuth": 30.5, "lightElevation": 0})),
        ("path.outlineStroke", json!({"width": 10})),
        ("stylize.scribble", json!({"variation": 2})),
        ("blur.gaussian", json!({"linear": true})),
        ("blur.gaussian", json!({})),
        ("cropMarks", json!({"style": "japanese"})),
        ("cropMarks", json!({})),
    ] {
        assert!(check_params("effect.apply", &apply(effect, params.clone())).is_ok(), "{effect} {params}");
    }
}

/// `effect.apply`, `appearance.addEffect` and `effect.setParams` refuse such a parameter of the
/// wrong type when they run, and so does a step of a `command.batch` whose effect a step before it
/// added, with the document and its undo history as they were. A saved step still plays with it,
/// as with a key the effect no longer takes.
#[test]
fn effect_commands_refuse_an_undefaulted_parameter_of_the_wrong_type() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let state = |s: &Session| {
        let st = s.doc().unwrap();
        (st.history.undo.len(), st.doc.clone())
    };
    let before = state(&s);
    let apply = |effect: &str, params| json!({"effect": effect, "params": params});
    for (id, params, why) in [
        ("effect.apply", apply("path.outlineStroke", json!({"width": "10 pt"})), "`width` must be a number"),
        ("appearance.addEffect", apply("blur.gaussian", json!({"linear": 1})), "`linear` must be true or false"),
        ("effect.apply", apply("threeD.revolve", json!({"perspective": "50"})), "`perspective` must be a number"),
    ] {
        let e = s.execute(id, &params).unwrap_err().to_string();
        assert!(e.contains(why), "{id} {params}: {e}");
        assert_eq!(state(&s), before, "{id} {params}");
    }
    let batch = json!({"commands": [
        {"command": "effect.apply", "params": apply("path.outlineStroke", json!({"width": 3}))},
        {"command": "effect.setParams", "params": {"index": 0, "params": {"width": "10 pt"}}},
    ]});
    let e = s.execute("command.batch", &batch).unwrap_err().to_string();
    assert!(e.contains("`width` must be a number"), "{e}");
    assert_eq!(state(&s), before);
    s.execute("effect.apply", &apply("path.outlineStroke", json!({"width": 3}))).unwrap();
    let applied = state(&s);
    for params in [json!({"width": "10 pt"}), json!({"width": null}), json!({"width": true})] {
        let e = s.execute("effect.setParams", &json!({"index": 0, "params": params.clone()})).unwrap_err().to_string();
        assert!(e.contains("`width` must be a number"), "{params}: {e}");
        assert_eq!(state(&s), applied, "{params}");
    }
    s.execute("effect.setParams", &json!({"index": 0, "params": {"width": 4}})).unwrap();
    s.replay("effect.setParams", &json!({"index": 0, "params": {"retired": "x", "width": "10 pt"}})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.doc.layers[0].children().unwrap()[0].appearance.effects[0].params["width"], json!("10 pt"));
}

/// The Transform effect (`distort.transform`) is held to its params description: `effect.apply`
/// and `effect.setParams` refuse each of its on/off options given as text, such as `reflectX:
/// "false"`, and each of its numbers given as text, such as `copies: "0"`, and leave the effect
/// as it was; true, false and numbers pass. The loop takes the keys from the effect's defaults, so
/// an option the effect gains is checked too.
#[test]
fn transform_effect_refuses_a_boolean_or_number_given_as_text() {
    let info = vectorcraft_render::effects::effect_info("distort.transform").unwrap();
    let one = |key: &str, v: serde_json::Value| serde_json::Value::Object([(key.to_string(), v)].into_iter().collect());
    let apply = |params| json!({"effect": "distort.transform", "params": params});
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("effect.apply", &apply(json!({"moveH": 20}))).unwrap();
    let state = |s: &Session| {
        let st = s.doc().unwrap();
        (st.history.undo.len(), st.doc.layers[0].children().unwrap()[0].appearance.effects[0].params.clone())
    };
    let before = state(&s);
    let mut switches = vec![];
    for (key, default) in info.defaults.as_object().unwrap() {
        let why = match default {
            serde_json::Value::Bool(_) => format!("`{key}` must be true or false"),
            serde_json::Value::Number(_) => format!("`{key}` must be a number"),
            _ => continue,
        };
        if default.is_boolean() {
            switches.push(key.as_str());
        }
        let text = json!(default.to_string());
        assert_eq!(refusal("effect.apply", apply(one(key, text.clone()))), why);
        let e = s.execute("effect.setParams", &json!({"index": 0, "params": one(key, text)})).unwrap_err().to_string();
        assert!(e.contains(&why), "{e}");
        assert!(check_params("effect.apply", &apply(one(key, default.clone()))).is_ok(), "{key}");
    }
    assert!(["reflectX", "reflectY", "random"].iter().all(|k| switches.contains(k)), "{switches:?}");
    assert_eq!(state(&s), before);
    s.execute("effect.setParams", &json!({"index": 0, "params": {"reflectX": true, "random": false, "copies": 2}})).unwrap();
}

/// What a command notes for its journal entry is a key its description names, so the entry
/// passes the check a caller's params get.
#[test]
fn journal_entries_pass_the_check() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.scale", &json!({"sx": 150})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 100, "text": "a"})).unwrap();
    assert!(s.journal.iter().count() >= 5);
    for (id, params) in s.journal.iter() {
        assert!(check_params(id, params).is_ok(), "{id} {params}");
    }
}

/// An index of a path, type, panel, library, style or ruler command given as something other than a
/// whole number of 0 or more (`"3"`, `2.5`, `-1`, `true`, `[]`, `{}`) is refused as the wrong type,
/// naming its key. A required one is not reported as missing, and an optional one (`subpath` of
/// `path.appendAnchor`, `path.close` and `path.reverse`, `artboard` of the ruler origin commands) is
/// not read as its default. A required one left out or null is still missing, and an optional one left
/// out or null keeps its default. Neither changes the document or records an undo step.
#[test]
fn a_malformed_index_is_not_missing_or_its_default() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = s.execute("path.create", &json!({"d": "M0 0 L10 0 M20 0 L30 0"})).unwrap()["id"].clone();
    s.execute("library.create", &json!({"name": "Brand"})).unwrap();
    s.execute("library.createGroup", &json!({"name": "Warm"})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let with = |base: &serde_json::Value, key: &str, v: Option<serde_json::Value>| {
        let mut p = base.clone();
        let o = p.as_object_mut().unwrap();
        match v {
            Some(v) => o.insert(key.into(), v),
            None => o.remove(key),
        };
        p
    };
    let refused = |s: &mut Session, cmd: &str, p: &serde_json::Value, want: &str| match s.execute(cmd, p) {
        Err(EngineError::BadParams { cmd: c, msg }) => assert_eq!((c.as_str(), msg.as_str()), (cmd, want), "{p}"),
        other => panic!("{cmd} {p}: {other:?}"),
    };
    let malformed = [json!("3"), json!(2.5), json!(-1), json!(true), json!([]), json!({})];
    let handle = json!({"id": id, "x": 5, "y": 5});
    let required = [
        ("path.setHandle", handle.clone(), "anchor", "missing anchor"),
        ("path.moveAnchor", handle.clone(), "anchor", "missing anchor"),
        ("path.insertAnchor", json!({"id": id}), "segment", "missing segment"),
        ("text.setPathHandle", handle.clone(), "anchor", "missing anchor"),
        ("artboard.reorder", json!({"index": 0}), "to", "missing to"),
        ("library.moveGroup", json!({"group": "Warm"}), "to", "missing `to` (an index)"),
        ("graphicStyle.move", json!({"name": "Default"}), "to", "missing `to` (an index)"),
    ];
    for (cmd, base, key, missing) in &required {
        for v in &malformed {
            refused(&mut s, cmd, &with(base, key, Some(v.clone())), &format!("`{key}` must be a whole number"));
        }
        for p in [with(base, key, None), with(base, key, Some(json!(null)))] {
            refused(&mut s, cmd, &p, missing);
        }
    }
    let optional = [
        ("path.appendAnchor", handle.clone(), "subpath"),
        ("path.close", json!({"id": id}), "subpath"),
        ("path.reverse", json!({"ids": [id]}), "subpath"),
        ("document.rulerOrigin", json!({}), "artboard"),
        ("document.setRulerOrigin", json!({"x": 5, "y": 5}), "artboard"),
        ("document.resetRulerOrigin", json!({}), "artboard"),
    ];
    for (cmd, base, key) in &optional {
        for v in &malformed {
            refused(&mut s, cmd, &with(base, key, Some(v.clone())), &format!("`{key}` must be a whole number"));
        }
    }
    // A whole number that names no artboard is still out of range.
    refused(&mut s, "document.resetRulerOrigin", &json!({"artboard": 1}), "index out of range");
    assert_eq!(s.doc().unwrap().doc, doc);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // Left out or null, an optional one keeps its default.
    for (cmd, base, key) in &optional {
        for p in [with(base, key, None), with(base, key, Some(json!(null)))] {
            s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"));
        }
    }
}
