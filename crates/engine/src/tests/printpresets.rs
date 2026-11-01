//! Edit → Print Presets: [Default] is protected, saved presets round-trip through files and
//! persist with the preferences.

use serde_json::json;

use crate::cmd::printpresets::DEFAULT_PRESET;
use crate::*;

fn is_bad_params(r: Result<serde_json::Value>) -> bool {
    matches!(r, Err(EngineError::BadParams { .. }))
}

#[test]
fn default_is_listed_first_and_protected() {
    let mut s = Session::new();
    let v = s.execute("print.presets.list", &json!({})).unwrap();
    assert_eq!((v["presets"][0]["name"].clone(), v["presets"][0]["builtIn"].clone()), (json!(DEFAULT_PRESET), json!(true)));
    assert_eq!(v["presets"][0]["settings"]["copies"], 1);
    assert_eq!(v["presets"][0]["changed"], json!([]));
    for name in [DEFAULT_PRESET, "default", " [default] "] {
        assert!(is_bad_params(s.execute("print.presets.save", &json!({"name": name, "settings": {"copies": 2}}))), "{name}");
        assert!(is_bad_params(s.execute("print.presets.delete", &json!({"name": name}))), "{name}");
    }
    // Nor can a saved preset take its name.
    s.execute("print.presets.save", &json!({"name": "Proofs"})).unwrap();
    assert!(is_bad_params(s.execute("print.presets.save", &json!({"name": "Proofs", "newName": "Default"}))));
    assert_eq!(s.prefs.print_presets.len(), 1);
    assert!(is_bad_params(s.execute("print.presets.delete", &json!({"name": "Nope"}))));
}

#[test]
fn presets_save_change_rename_and_delete() {
    let mut s = Session::new();
    let v = s.execute("print.presets.save", &json!({"settings": {"media": "a4", "marks": {"trim": true}}})).unwrap();
    assert_eq!((v["name"].clone(), v["created"].clone()), (json!("Print Preset 1"), json!(true)));
    // A saved preset changes from its own settings; `preset` starts from another one.
    let v = s.execute("print.presets.save", &json!({"name": "print preset 1", "settings": {"copies": 3}})).unwrap();
    assert_eq!((v["created"].clone(), v["settings"]["media"].clone(), v["settings"]["copies"].clone()), (json!(false), json!("a4"), json!(3)));
    let v = s.execute("print.presets.save", &json!({"name": "Plain", "preset": "Print Preset 1", "settings": {"marks": {"trim": false}}})).unwrap();
    assert_eq!((v["settings"]["copies"].clone(), v["settings"]["marks"]["trim"].clone()), (json!(3), json!(false)));
    let v = s.execute("print.presets.list", &json!({})).unwrap();
    let changed: Vec<&str> = v["presets"][1]["changed"].as_array().unwrap().iter().map(|c| c["option"].as_str().unwrap()).collect();
    assert_eq!(changed, ["copies", "marks.trim", "media"]);
    // Renames keep the settings; bad settings and unknown bases are refused.
    s.execute("print.presets.save", &json!({"name": "Plain", "newName": "Draft"})).unwrap();
    assert_eq!(s.prefs.print_presets[1].name, "Draft");
    assert!(is_bad_params(s.execute("print.presets.save", &json!({"name": "Draft", "newName": "print preset 1"}))));
    assert!(is_bad_params(s.execute("print.presets.save", &json!({"name": "X", "settings": {"copies": 0}}))));
    assert!(is_bad_params(s.execute("print.presets.save", &json!({"name": "X", "preset": "Nope"}))));
    assert!(is_bad_params(s.execute("print.presets.save", &json!({"name": " "}))));
    assert_eq!(s.execute("print.presets.delete", &json!({"name": "draft"})).unwrap()["deleted"], "Draft");
    assert_eq!(s.prefs.print_presets.len(), 1);
}

#[test]
fn presets_export_and_import_round_trip() {
    let mut s = Session::new();
    s.execute("print.presets.save", &json!({"name": "Tiles", "settings": {"scaling": "tileFull", "overlap": 18}})).unwrap();
    let saved = s.prefs.print_presets.clone();
    let data = s.execute("print.presets.export", &json!({"names": ["Tiles", DEFAULT_PRESET]})).unwrap();
    assert_eq!(data["count"], 2);
    let text = data["data"].as_str().unwrap().to_string();
    s.execute("print.presets.delete", &json!({"name": "Tiles"})).unwrap();
    let r = s.execute("print.presets.import", &json!({"data": text})).unwrap();
    assert_eq!(r["imported"], json!(["Tiles", "Default 2"]), "[Default] comes in as a saved copy");
    assert_eq!(s.prefs.print_presets[0], saved[0], "the same preset comes back");
    assert_eq!(s.prefs.print_presets[1].settings, vectorcraft_pdf::PrintSettings::default());
    // replace: saved presets of the same names are overwritten.
    let r = s.execute("print.presets.import", &json!({"data": text, "replace": true})).unwrap();
    assert_eq!(r["imported"], json!(["Tiles", "Default 3"]));
    assert_eq!(s.prefs.print_presets.len(), 3);
    // Through a file, and every saved preset by default.
    let path = std::env::temp_dir().join(format!("vc-printpresets-{}.vcprintpresets", std::process::id())).to_string_lossy().to_string();
    assert_eq!(s.execute("print.presets.export", &json!({"path": path})).unwrap()["count"], 3);
    let mut t = Session::new();
    assert_eq!(t.execute("print.presets.import", &json!({"path": path})).unwrap()["imported"].as_array().unwrap().len(), 3);
    assert_eq!(t.prefs.print_presets, s.prefs.print_presets);
    let _ = std::fs::remove_file(&path);
    // Not a presets file, or values out of range: refused, nothing added.
    for bad in [
        json!({"data": "{}"}),
        json!({"data": "{\"format\": \"vcpdfpresets\", \"presets\": []}"}),
        json!({"data": "{\"format\": \"vcprintpresets\", \"presets\": [{\"name\": \"X\", \"settings\": {\"copies\": 5000}}]}"}),
        json!({"data": "{\"format\": \"vcprintpresets\", \"presets\": [{\"name\": \"X\", \"settings\": {\"media\": \"quarto\"}}]}"}),
        json!({"dataBase64": "!!"}),
        json!({}),
    ] {
        assert!(is_bad_params(t.execute("print.presets.import", &bad)), "{bad}");
    }
    assert_eq!(t.prefs.print_presets.len(), 3);
    assert!(is_bad_params(Session::new().execute("print.presets.export", &json!({}))), "nothing saved to export");
    assert!(is_bad_params(Session::new().execute("print.presets.export", &json!({"names": ["Nope"]}))));
}

#[test]
fn saved_presets_persist_with_the_preferences() {
    let mut s = Session::new();
    s.execute("print.presets.save", &json!({"name": "Kept", "settings": {"copies": 2}})).unwrap();
    let back: crate::Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(back.print_presets, s.prefs.print_presets);
    // Resetting the preferences keeps them (a library, not a preference).
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.print_presets.len(), 1);
    // Preferences saved before print presets existed still load, and none are written while empty.
    let old: crate::Prefs = serde_json::from_value(json!({"keyboardIncrement": 2.0})).unwrap();
    assert!(old.print_presets.is_empty());
    assert!(crate::Prefs::default().to_json().get("printPresets").is_none());
}
