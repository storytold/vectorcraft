//! Graphics backend preference validation and saved-file compatibility.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use serde_json::{Value, json};
use vectorcraft_engine::{Prefs, Session};

#[test]
fn gpu_backend_defaults_validates_and_round_trips_platform_choices() {
    use vectorcraft_engine::cmd::prefscmds::GPU_BACKENDS;
    let mut session = Session::new();
    assert_eq!(session.prefs.gpu_backend, "automatic");
    assert_eq!(session.execute("prefs.get", &json!({"key":"gpuBackend"})).unwrap(), "automatic");
    for (value, label) in GPU_BACKENDS {
        session.execute("prefs.set", &json!({"key":"gpuBackend", "value":label})).unwrap();
        assert_eq!(session.prefs.gpu_backend, *value);
        let restored: Prefs = serde_json::from_value(session.prefs.to_json()).unwrap();
        assert_eq!(restored.gpu_backend, *value);
        assert_eq!(restored.gpu_preference, "automatic", "backend and power choice are independent");
    }
    let row = session.execute("prefs.list", &json!({})).unwrap();
    let row = row.as_array().unwrap().iter().find(|row| row["key"] == "gpuBackend").unwrap();
    assert_eq!(row["category"], "Performance");
    assert_eq!(row["options"], json!(GPU_BACKENDS.iter().map(|(value, _)| *value).collect::<Vec<_>>()));
    session.execute("prefs.reset", &json!({"category":"Performance"})).unwrap();
    for bad in [json!("unknown"), json!(""), json!(1), Value::Null, json!(true), json!(2.5), json!([]), json!({})] {
        assert!(session.execute("prefs.set", &json!({"key":"gpuBackend", "value":bad})).is_err());
        assert_eq!(session.prefs.gpu_backend, "automatic");
    }
    let old: Prefs = serde_json::from_value(json!({"keyboardIncrement":4})).unwrap();
    assert_eq!(old.gpu_backend, "automatic");
    for value in ["future", "dx12", "metal", "vulkan", "gl"] {
        let saved: Prefs = serde_json::from_value(json!({"gpuBackend":value,"keyboardIncrement":4})).unwrap();
        let expected = vectorcraft_engine::cmd::prefscmds::gpu_backend_name(Some(value));
        assert_eq!(saved.gpu_backend, expected);
        assert_eq!(saved.keyboard_increment, 4.0, "an unsupported backend does not discard other preferences");
    }
    for value in [Value::Null, json!(true), json!(1), json!(2.5), json!([]), json!({"future":"backend"})] {
        let saved: Prefs = serde_json::from_value(json!({"gpuBackend":value,"keyboardIncrement":4})).unwrap();
        assert_eq!(saved.gpu_backend, "automatic");
        assert_eq!(saved.keyboard_increment, 4.0, "a malformed backend must not discard other preferences");
    }
}

fn imported_preferences(prefs: Value) -> Value {
    json!({"data": json!({"format":"vectorcraft-preferences", "version":1, "prefs":prefs}).to_string()})
}

#[test]
fn gpu_backend_import_keeps_other_values_when_the_backend_is_invalid() {
    use vectorcraft_engine::cmd::prefscmds::GPU_BACKENDS;
    let mut invalid = vec![Value::Null, json!(true), json!(1), json!(2.5), json!([]), json!({}), json!(""), json!("future")];
    invalid.extend(
        ["dx12", "vulkan", "gl", "metal"].into_iter().filter(|value| !GPU_BACKENDS.iter().any(|(id, _)| id == value)).map(|value| json!(value)),
    );
    for value in invalid {
        let mut session = Session::new();
        let before = session.prefs.to_json();
        assert!(session.execute("prefs.set", &json!({"key":"gpuBackend", "value":&value})).is_err(), "{value}");
        assert_eq!(session.prefs.to_json(), before, "strict prefs.set remains atomic for {value}");
        let params = imported_preferences(json!({"gpuBackend":value, "keyboardIncrement":4}));
        assert_eq!(session.execute("prefs.import", &params).unwrap(), json!({"imported":2}));
        assert_eq!(session.prefs.gpu_backend, "automatic");
        assert_eq!(session.prefs.keyboard_increment, 4.0);
    }
}

#[test]
fn gpu_backend_import_preserves_supported_choices_and_other_import_rules() {
    use vectorcraft_engine::cmd::prefscmds::GPU_BACKENDS;
    let mut session = Session::new();
    for (id, label) in GPU_BACKENDS {
        for value in [id.to_string(), id.to_ascii_uppercase(), label.to_string(), label.to_ascii_lowercase()] {
            let params = imported_preferences(json!({"gpuBackend":value, "keyboardIncrement":4}));
            assert_eq!(session.execute("prefs.import", &params).unwrap(), json!({"imported":2}));
            assert_eq!(session.prefs.gpu_backend, *id, "offered IDs and labels retain the existing case-insensitive matching");
            assert_eq!(session.prefs.keyboard_increment, 4.0);
        }
    }
    let kept_backend = session.prefs.gpu_backend.clone();
    session.execute("prefs.import", &imported_preferences(json!({"keyboardIncrement":5}))).unwrap();
    assert_eq!(session.prefs.gpu_backend, kept_backend, "an omitted backend leaves the current choice alone");
    assert_eq!(session.prefs.keyboard_increment, 5.0);
    for prefs in [json!({"gpuBackend":"future", "keyboardIncrement":true}), json!({"gpuBackend":false, "noSuchPreference":4})] {
        let before = session.prefs.to_json();
        assert!(session.execute("prefs.import", &imported_preferences(prefs)).is_err());
        assert_eq!(session.prefs.to_json(), before, "normalizing the backend must not bypass another invalid value");
    }
}
