//! Preferences files: `prefs.export` writes the preferences as JSON and `prefs.import` checks and
//! applies such a file.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;

use crate::cmd::prefscmds::{PREF_GROUPS, PREF_SPECS, PREFS_FORMAT};
use crate::*;

fn is_bad_params(r: Result<Value>) -> bool {
    matches!(r, Err(EngineError::BadParams { .. }))
}

/// Folder preferences, which a preferences file leaves out.
const FOLDERS: [&str; 4] = ["fontsFolder", "pluginsFolder", "recoveryFolder", "templatesFolder"];

/// A preferences file holding `prefs`, as `prefs.export` writes one.
fn file(prefs: Value) -> String {
    json!({"format": PREFS_FORMAT, "version": 1, "prefs": prefs}).to_string()
}

/// A session whose preferences differ from the defaults in every kind of value.
fn changed() -> Session {
    let mut s = Session::new();
    let values = json!({
        "showToolTips": false,
        "keyboardIncrement": 2.5,
        "historyStates": 42,
        "uiAccent": "green",
        "guideColor": "#12ab34",
        "unitsGeneral": "inches",
        "eyedropper": {"sampleSize": 5},
    });
    s.execute("prefs.set", &json!({"values": values})).unwrap();
    s
}

/// The preferences a preferences file holds, read from a session.
fn in_file(s: &Session) -> Value {
    let all = s.prefs.to_json();
    let keys = PREF_SPECS.iter().map(|sp| sp.key).filter(|k| !FOLDERS.contains(k)).chain(PREF_GROUPS.iter().copied());
    Value::Object(keys.map(|k| (k.to_string(), all[k].clone())).collect())
}

#[test]
fn exported_preferences_import_unchanged() {
    let mut s = changed();
    s.prefs.fonts_folder = "/fonts/here".into();
    let r = s.execute("prefs.export", &json!({})).unwrap();
    let text = r["data"].as_str().unwrap().to_string();
    let written: Value = serde_json::from_str(&text).unwrap();
    assert_eq!((written["format"].as_str(), written["version"].as_u64()), (Some(PREFS_FORMAT), Some(1)));
    let prefs = written["prefs"].as_object().unwrap();
    assert_eq!(r["count"], json!(prefs.len()));
    assert_eq!(prefs.len(), PREF_SPECS.len() - FOLDERS.len() + PREF_GROUPS.len());
    for k in FOLDERS {
        assert!(!prefs.contains_key(k), "{k} is left out");
    }
    assert!(!prefs.contains_key("printPresets"), "saved presets are left out");
    assert_eq!(prefs["uiAccent"], "green");

    // Into a session with defaults and a document in millimeters.
    let mut t = Session::new();
    t.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    t.execute("document.setUnits", &json!({"units": "Millimeters"})).unwrap();
    let undo = t.doc().unwrap().history.undo.len();
    let r = t.execute("prefs.import", &json!({"data": text})).unwrap();
    assert_eq!(r["imported"], json!(prefs.len()));
    assert_eq!(in_file(&t), in_file(&s), "export then import gives the same preferences");
    assert_eq!(t.prefs.units_general, "inches");
    assert_eq!(t.doc().unwrap().doc.units, Unit::Millimeters, "the document's units stay as they are");
    assert_eq!(t.doc().unwrap().history.undo.len(), undo, "no undo step");
    assert_eq!(t.prefs.fonts_folder, "", "folders are not carried over");

    // Through a file, too.
    let path = std::env::temp_dir().join(format!("vc-prefs-{}.json", std::process::id())).to_string_lossy().to_string();
    assert_eq!(s.execute("prefs.export", &json!({"path": path})).unwrap()["path"], json!(path));
    let mut u = Session::new();
    u.execute("prefs.import", &json!({"path": path})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(in_file(&u), in_file(&s));
}

/// A file written by hand, not by `prefs.export`: so the two sides cannot agree on a wrong layout.
#[test]
fn a_hand_written_file_sets_what_it_names_and_leaves_the_rest() {
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"key": "historyStates", "value": 77})).unwrap();
    let text = r##"{
        "format": "vectorcraft-preferences",
        "version": 1,
        "prefs": {"uiAccent": "Violet", "keyboardIncrement": "3 pt", "guideColor": "#ABCDEF", "pathfinder": {}}
    }"##;
    assert_eq!(s.execute("prefs.import", &json!({"data": text})).unwrap(), json!({"imported": 4}));
    assert_eq!(s.prefs.ui_accent, "violet");
    assert_eq!(s.prefs.keyboard_increment, 3.0);
    assert_eq!(s.prefs.guide_color, "#abcdef");
    assert_eq!(s.prefs.history_states, 77, "a preference the file leaves out keeps its value");
}

#[test]
fn a_bad_preferences_file_is_refused_and_nothing_changes() {
    let mut s = changed();
    let before = s.prefs.to_json().to_string();
    let too_large = " ".repeat((1 << 20) + 1);
    let cases: Vec<(Value, &str)> = vec![
        (json!({"data": ""}), "not a preferences file"),
        (json!({"data": "{\"format\": "}), "not a preferences file"),
        (json!({"data": "[]"}), "not a preferences file: no JSON object"),
        (json!({"data": "{\"version\": 1, \"prefs\": {}}"}), "no `format`"),
        (json!({"data": "{\"format\": \"vcprintpresets\", \"version\": 1, \"prefs\": {}}"}), "(format `vcprintpresets`)"),
        (json!({"data": "{\"format\": \"vectorcraft-preferences\", \"prefs\": {\"uiAccent\": \"blue\"}}"}), "no `version`"),
        (json!({"data": "{\"format\": \"vectorcraft-preferences\", \"version\": 0, \"prefs\": {\"uiAccent\": \"blue\"}}"}), "no `version`"),
        (json!({"data": file(json!({"uiAccent": "blue"})).replace("\"version\":1", "\"version\":2")}), "newer version of VectorCraft (version 2)"),
        (json!({"data": "{\"format\": \"vectorcraft-preferences\", \"version\": 1}"}), "no `prefs`"),
        (json!({"data": file(json!({}))}), "holds no preferences"),
        (json!({"data": file(json!({"uiAccent": "blue", "noSuchPreference": 1}))}), "unknown preference `noSuchPreference`"),
        (json!({"data": file(json!({"uiAccent": "blue", "showToolTips": 3}))}), "`showToolTips` must be true or false"),
        (json!({"data": file(json!({"uiAccent": "blue", "historyStates": 5000}))}), "`historyStates` must be a whole number between 5 and 1000"),
        (json!({"data": file(json!({"uiAccent": "blue", "eyedropper": 5}))}), "`eyedropper` must be an object"),
        (json!({"data": file(json!({"uiAccent": "blue", "fontsFolder": "/fonts"}))}), "`fontsFolder` names a folder"),
        (json!({"data": too_large}), "larger than 1 MB"),
        (json!({"data": 5}), "`data` must be a string"),
        (json!({"path": "/a.json", "data": file(json!({"uiAccent": "blue"}))}), "give `path` or `data`"),
        (json!({}), "give `path` or `data`"),
    ];
    for (params, says) in cases {
        let r = s.execute("prefs.import", &params);
        let e = r.as_ref().map_err(ToString::to_string).unwrap_err();
        assert!(e.contains(says), "{params}: {e}");
        assert!(is_bad_params(r), "{params}");
        assert_eq!(s.prefs.to_json().to_string(), before, "{params}: nothing changes");
    }
}

/// `prefs.import` as a step of a batch is an error, and the preferences stay as they are.
#[test]
fn prefs_import_is_refused_as_a_batch_step() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let before = s.prefs.to_json();
    let steps = json!({"commands": [{"command": "prefs.import", "params": {"data": file(json!({"uiAccent": "green"}))}}]});
    let e = s.execute("command.batch", &steps).unwrap_err().to_string();
    assert!(e.contains("`prefs.import` runs on its own"), "{e}");
    assert_eq!(s.prefs.to_json(), before);
}

#[test]
fn a_file_over_the_limit_is_refused_before_it_is_parsed() {
    let mut s = Session::new();
    let before = s.prefs.to_json().to_string();
    let path = std::env::temp_dir().join(format!("vc-prefs-large-{}.json", std::process::id()));
    // A valid file padded past 1 MB: refused for its size, not read for its preferences.
    let mut text = file(json!({"uiAccent": "pink"}));
    text.push_str(&" ".repeat(1 << 20));
    std::fs::write(&path, text).unwrap();
    let e = s.execute("prefs.import", &json!({"path": path.to_string_lossy()})).unwrap_err().to_string();
    let _ = std::fs::remove_file(&path);
    assert!(e.contains("larger than 1 MB"), "{e}");
    assert_eq!(s.prefs.to_json().to_string(), before);
    // Nor is such a file written: a preferences file over 1 MB could not be imported again.
    s.prefs.hyphenation_exceptions = "x".repeat(1 << 20);
    let e = s.execute("prefs.export", &json!({})).unwrap_err().to_string();
    assert!(e.contains("larger than 1 MB"), "{e}");
    s.prefs.hyphenation_exceptions.clear();
    let missing = std::env::temp_dir().join("vc-prefs-no-such-file.json");
    assert!(s.execute("prefs.import", &json!({"path": missing.to_string_lossy()})).is_err());
    assert_eq!(s.prefs.to_json().to_string(), before);
}
