//! Transparency Flattener Presets (M3.93): built-in presets stay as they are, saved presets are
//! made, changed, renamed and deleted, travel through export/import and persist with the
//! preferences.

use serde_json::{Value, json};

use crate::cmd::{FlattenOptions, FlattenerPreset};
use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn names(s: &mut Session) -> Vec<(String, bool)> {
    let list = run(s, "flattener.presets.list", json!({}));
    list["presets"].as_array().unwrap().iter().map(|p| (p["name"].as_str().unwrap().to_string(), p["builtIn"].as_bool().unwrap())).collect()
}

#[test]
fn built_in_presets_are_listed_and_protected() {
    let mut s = Session::new();
    let builtin = [("High Resolution", true), ("Medium Resolution", true), ("Low Resolution", true)].map(|(n, b)| (n.to_string(), b));
    assert_eq!(names(&mut s), builtin);
    for name in ["High Resolution", "medium", "LOW"] {
        assert!(s.execute("flattener.presets.save", &json!({"name": name, "balance": 10})).is_err(), "{name} changed");
        assert!(s.execute("flattener.presets.delete", &json!({ "name": name })).is_err(), "{name} deleted");
    }
    assert!(s.execute("flattener.presets.delete", &json!({"name": "Nope"})).is_err());
    assert_eq!(FlattenOptions::preset("high"), FlattenOptions::find_preset("High Resolution", &[]));
    assert_eq!(names(&mut s), builtin, "nothing changed");
    // A saved preset can't take a built-in name either.
    run(&mut s, "flattener.presets.save", json!({"name": "Mine"}));
    assert!(s.execute("flattener.presets.save", &json!({"name": "Mine", "newName": "low resolution"})).is_err());
}

#[test]
fn saved_presets_are_made_changed_renamed_and_used() {
    let mut s = Session::new();
    let r = run(&mut s, "flattener.presets.save", json!({"name": "Press", "preset": "high", "balance": 40}));
    assert_eq!((r["name"].as_str(), r["created"].as_bool()), (Some("Press"), Some(true)));
    assert_eq!((r["options"]["balance"].as_f64(), r["options"]["lineArtPpi"].as_f64()), (Some(40.0), Some(1200.0)));
    // A change starts from the preset's own options.
    let r = run(&mut s, "flattener.presets.save", json!({"name": "press", "options": {"antiAlias": false}}));
    assert_eq!(
        (r["created"].as_bool(), r["options"]["balance"].as_f64(), r["options"]["antiAlias"].as_bool()),
        (Some(false), Some(40.0), Some(false))
    );
    assert_eq!(r["name"], "Press", "the saved name keeps its case");
    // Renaming, and names that are taken.
    run(&mut s, "flattener.presets.save", json!({"name": "Press", "newName": "Print"}));
    run(&mut s, "flattener.presets.save", json!({"name": "Other"}));
    assert!(s.execute("flattener.presets.save", &json!({"name": "Print", "newName": "other"})).is_err());
    assert!(s.execute("flattener.presets.save", &json!({"name": "  "})).is_err());
    assert!(s.execute("flattener.presets.save", &json!({"name": "Print", "balance": 101})).is_err(), "options are validated");
    // Unnamed presets are numbered.
    assert_eq!(run(&mut s, "flattener.presets.save", json!({}))["name"], "Flattener Preset 1");
    assert_eq!(run(&mut s, "flattener.presets.save", json!({}))["name"], "Flattener Preset 2");
    let saved: Vec<String> = names(&mut s).into_iter().filter(|n| !n.1).map(|n| n.0).collect();
    assert_eq!(saved, ["Print", "Other", "Flattener Preset 1", "Flattener Preset 2"]);
    // Anywhere options are taken, a saved preset works by name.
    let o = s.flatten_options(&json!({"preset": "PRINT"})).unwrap();
    assert_eq!((o.balance, o.line_art_ppi, o.anti_alias), (40.0, 1200.0, false));
    assert!(s.flatten_options(&json!({"preset": "Gone"})).is_err());
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let id = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}))["id"].clone();
    run(&mut s, "transparency.set", json!({"ids": [id], "opacity": 50}));
    let r = run(&mut s, "object.flattenTransparency", json!({"ids": [id], "preset": "Print"}));
    assert_eq!(r["options"]["balance"], 40.0);
    assert_eq!(run(&mut s, "flattener.presets.delete", json!({"name": "print"}))["deleted"], "Print");
    assert!(s.execute("object.flattenTransparency", &json!({"preset": "Print"})).is_err());
}

#[test]
fn presets_round_trip_through_export_and_import() {
    let mut s = Session::new();
    assert!(s.execute("flattener.presets.export", &json!({})).is_err(), "nothing saved to export");
    run(&mut s, "flattener.presets.save", json!({"name": "A", "preset": "low", "clipComplexRegions": true}));
    run(&mut s, "flattener.presets.save", json!({"name": "B", "balance": 0}));
    let saved = s.prefs.flattener_presets.clone();
    let out = run(&mut s, "flattener.presets.export", json!({}));
    assert_eq!(out["count"], 2);
    let data = out["data"].as_str().unwrap().to_string();
    for n in ["A", "B"] {
        run(&mut s, "flattener.presets.delete", json!({ "name": n }));
    }
    assert_eq!(run(&mut s, "flattener.presets.import", json!({ "data": data }))["imported"], json!(["A", "B"]));
    assert_eq!(s.prefs.flattener_presets, saved, "the same presets come back");
    // Names in use get a number, unless the import replaces them.
    assert_eq!(run(&mut s, "flattener.presets.import", json!({ "data": data }))["imported"], json!(["A 2", "B 2"]));
    run(&mut s, "flattener.presets.save", json!({"name": "A", "balance": 55}));
    run(&mut s, "flattener.presets.import", json!({"data": data, "replace": true}));
    assert_eq!(s.prefs.flattener_presets[0], saved[0]);
    assert_eq!(s.prefs.flattener_presets.len(), 4);
    // Built-in presets export by id or name, and import as saved copies.
    let b64 = {
        let out = run(&mut s, "flattener.presets.export", json!({"names": ["high", "Medium Resolution"]}));
        assert_eq!(out["count"], 2);
        vectorcraft_format::base64_encode(out["data"].as_str().unwrap().as_bytes())
    };
    let r = run(&mut s, "flattener.presets.import", json!({ "dataBase64": b64 }));
    assert_eq!(r["imported"], json!(["High Resolution 2", "Medium Resolution 2"]));
    assert_eq!(s.flatten_options(&json!({"preset": "High Resolution 2"})).unwrap(), FlattenOptions::preset("high").unwrap());
    // Files on disk, and what isn't a presets file.
    let path = std::env::temp_dir().join(format!("vc-flattener-{}.{}", std::process::id(), crate::cmd::flatten::PRESET_FORMAT));
    let path = path.to_string_lossy().to_string();
    assert_eq!(run(&mut s, "flattener.presets.export", json!({"names": ["A"], "path": path}))["count"], 1);
    assert_eq!(run(&mut s, "flattener.presets.import", json!({ "path": path }))["imported"], json!(["A 3"]));
    let _ = std::fs::remove_file(&path);
    assert!(s.execute("flattener.presets.export", &json!({"names": ["Nope"]})).is_err());
    for bad in [json!({"data": "{}"}), json!({"data": "not json"}), json!({"data": "{\"format\": \"other\", \"presets\": []}"}), json!({})] {
        assert!(s.execute("flattener.presets.import", &bad).is_err(), "{bad}");
    }
    let wild = json!({"format": "vcflattener", "presets": [{"name": "Wild", "options": {"lineArtPpi": 99999}}]}).to_string();
    assert!(s.execute("flattener.presets.import", &json!({ "data": wild })).is_err(), "out-of-range values are rejected");
}

#[test]
fn saved_presets_persist_with_the_preferences() {
    let mut s = Session::new();
    // Preferences saved before presets existed load without any.
    let old = json!({"keyboardIncrement": 2.0});
    assert!(serde_json::from_value::<Prefs>(old).unwrap().flattener_presets.is_empty());
    assert!(Prefs::default().to_json().get("flattenerPresets").is_none(), "nothing saved while there are none");
    run(&mut s, "flattener.presets.save", json!({"name": "Keep", "gradientPpi": 72}));
    let saved = serde_json::to_string(&s.prefs).unwrap();
    let back: Prefs = serde_json::from_str(&saved).unwrap();
    assert_eq!(back.flattener_presets, [FlattenerPreset { name: "Keep".into(), options: s.flatten_options(&json!({"preset": "Keep"})).unwrap() }]);
    let mut t = Session::new();
    t.apply_prefs(back);
    assert_eq!(t.flatten_options(&json!({"preset": "keep"})).unwrap().gradient_ppi, 72.0);
    // They aren't a preference key, and resetting the preferences keeps them.
    assert!(t.execute("prefs.set", &json!({"key": "flattenerPresets", "value": []})).is_err());
    run(&mut t, "prefs.reset", json!({}));
    assert_eq!(t.prefs.flattener_presets.len(), 1);
    assert_eq!(run(&mut t, "prefs.get", json!({}))["prefs"]["flattenerPresets"][0]["name"], "Keep");
}
