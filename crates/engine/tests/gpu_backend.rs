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
