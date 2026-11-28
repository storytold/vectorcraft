//! Shared graphics adapter preference validation and saved-file compatibility.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use craft_gpu_device::{AdapterKey, Preference};
use serde_json::{Value, json};
use vectorcraft_engine::{Prefs, Session};

fn unavailable_adapter() -> Preference {
    Preference::Adapter(AdapterKey {
        name: "A disconnected graphics adapter".into(),
        backend: "vulkan".into(),
        vendor: 0x1234,
        device: 0x5678,
        pci_bus_id: "0000:04:00.0".into(),
        driver: "A retained driver".into(),
    })
}

fn malformed_preferences() -> Vec<Value> {
    vec![
        Value::Null,
        json!(true),
        json!(1),
        json!(2.5),
        json!([]),
        json!({}),
        json!("automatic"),
        json!("powerSaving"),
        json!("highPerformance"),
        json!("lowPower"),
        json!({"kind":"future"}),
        json!({"kind":false}),
        json!({"kind":"adapter","vendor":"not a number"}),
        json!({"kind":"adapter","backend":42}),
    ]
}

#[test]
fn gpu_backend_shared_preference_defaults_validates_and_round_trips_choices() {
    let mut session = Session::new();
    assert_eq!(session.prefs.gpu_preference, Preference::Auto);
    assert_eq!(session.execute("prefs.get", &json!({"key":"gpuPreference"})).unwrap(), json!({"kind":"auto"}));
    for preference in [Preference::Auto, Preference::LowPower, Preference::Software, unavailable_adapter()] {
        let value = json!(preference);
        session.execute("prefs.set", &json!({"key":"gpuPreference", "value":value})).unwrap();
        assert_eq!(session.prefs.gpu_preference, preference);
        assert_eq!(session.execute("prefs.get", &json!({"key":"gpuPreference"})).unwrap(), value);
        let restored: Prefs = serde_json::from_value(session.prefs.to_json()).unwrap();
        assert_eq!(restored.gpu_preference, preference, "no device enumeration is needed to retain the complete saved key");
    }
    let rows = session.execute("prefs.list", &json!({})).unwrap();
    let rows = rows.as_array().unwrap();
    let row = rows.iter().find(|row| row["key"] == "gpuPreference").unwrap();
    assert_eq!(row["category"], "Performance");
    assert_eq!(row["kind"], "gpuPreference");
    assert!(row.get("options").is_none(), "the app supplies available adapter choices, not a static string list");
    assert!(!rows.iter().any(|row| row["key"] == "gpuBackend"));
    assert!(session.execute("prefs.get", &json!({"key":"gpuBackend"})).is_err());
    assert!(session.execute("prefs.set", &json!({"key":"gpuBackend", "value":"vulkan"})).is_err());
    for value in malformed_preferences() {
        let before = session.prefs.to_json();
        assert!(session.execute("prefs.set", &json!({"values":{"gpuPreference":value,"keyboardIncrement":4}})).is_err());
        assert_eq!(session.prefs.to_json(), before, "strict prefs.set must not apply a valid sibling after a malformed choice");
    }
    session.execute("prefs.reset", &json!({"category":"Performance"})).unwrap();
    assert_eq!(session.prefs.gpu_preference, Preference::Auto);

    for (old, expected) in [
        ("automatic", Preference::Auto),
        ("powerSaving", Preference::Auto),
        ("highPerformance", Preference::Auto),
        ("lowPower", Preference::LowPower),
    ] {
        let restored: Prefs = serde_json::from_value(json!({"gpuPreference":old,"keyboardIncrement":4})).unwrap();
        assert_eq!(restored.gpu_preference, expected);
        assert_eq!(restored.keyboard_increment, 4.0);
    }
    for value in malformed_preferences() {
        // Saved legacy strings above have a migration; every other malformed value uses Auto.
        let expected = if value == json!("lowPower") { Preference::LowPower } else { Preference::Auto };
        let saved: Prefs = serde_json::from_value(json!({"gpuPreference":value,"keyboardIncrement":4})).unwrap();
        assert_eq!(saved.gpu_preference, expected);
        assert_eq!(saved.keyboard_increment, 4.0, "a malformed preference must not discard valid siblings");
    }
    for backend in [json!("vulkan"), json!("dx12"), json!("future"), Value::Null, json!({"bad":true})] {
        let saved: Prefs = serde_json::from_value(json!({"gpuBackend":backend,"keyboardIncrement":4})).unwrap();
        assert_eq!(saved.gpu_preference, Preference::Auto, "a backend-only choice cannot invent an AdapterKey");
        assert_eq!(saved.keyboard_increment, 4.0);
        assert!(saved.to_json().get("gpuBackend").is_none());
    }
}

fn imported_preferences(prefs: Value) -> Value {
    json!({"data": json!({"format":"vectorcraft-preferences", "version":1, "prefs":prefs}).to_string()})
}

#[test]
fn gpu_backend_import_keeps_other_values_when_the_preference_is_invalid() {
    for value in malformed_preferences() {
        let mut session = Session::new();
        session.execute("prefs.set", &json!({"key":"gpuPreference","value":unavailable_adapter()})).unwrap();
        let expected = if value == json!("lowPower") { Preference::LowPower } else { Preference::Auto };
        let params = imported_preferences(json!({"gpuPreference":value, "keyboardIncrement":4}));
        assert_eq!(session.execute("prefs.import", &params).unwrap(), json!({"imported":2}));
        assert_eq!(session.prefs.gpu_preference, expected);
        assert_eq!(session.prefs.keyboard_increment, 4.0);
    }
    for (old, expected) in [
        ("automatic", Preference::Auto),
        ("powerSaving", Preference::Auto),
        ("highPerformance", Preference::Auto),
        ("lowPower", Preference::LowPower),
    ] {
        let mut session = Session::new();
        session.execute("prefs.import", &imported_preferences(json!({"gpuPreference":old,"keyboardIncrement":6}))).unwrap();
        assert_eq!(session.prefs.gpu_preference, expected);
        assert_eq!(session.prefs.keyboard_increment, 6.0);
    }
}

#[test]
fn gpu_backend_import_preserves_shared_choices_and_other_import_rules() {
    let mut session = Session::new();
    for preference in [Preference::Auto, Preference::LowPower, Preference::Software, unavailable_adapter()] {
        let params = imported_preferences(json!({"gpuPreference":preference, "keyboardIncrement":4}));
        assert_eq!(session.execute("prefs.import", &params).unwrap(), json!({"imported":2}));
        assert_eq!(session.prefs.gpu_preference, preference);
        assert_eq!(session.prefs.keyboard_increment, 4.0);
    }
    let saved = session.prefs.gpu_preference.clone();
    session.execute("prefs.import", &imported_preferences(json!({"keyboardIncrement":5}))).unwrap();
    assert_eq!(session.prefs.gpu_preference, saved, "omitting gpuPreference retains the saved unavailable adapter");
    assert_eq!(session.prefs.keyboard_increment, 5.0);
    // The obsolete key does not name a new adapter, and a partial import must not erase one.
    assert_eq!(
        session.execute("prefs.import", &imported_preferences(json!({"gpuBackend":"vulkan","keyboardIncrement":6}))).unwrap(),
        json!({"imported":1})
    );
    assert_eq!(session.prefs.gpu_preference, saved);
    assert_eq!(session.prefs.keyboard_increment, 6.0);
    for prefs in [
        json!({"gpuPreference":{"kind":"future"}, "keyboardIncrement":true}),
        json!({"gpuPreference":false, "noSuchPreference":4}),
        json!({"gpuPreference":"lowPower", "unitsGeneral":"not a unit"}),
        json!({"gpuBackend":"future", "keyboardIncrement":true}),
    ] {
        let before = session.prefs.to_json();
        assert!(session.execute("prefs.import", &imported_preferences(prefs)).is_err());
        assert_eq!(session.prefs.to_json(), before, "migration must not bypass another invalid value");
    }
    let exported = session.execute("prefs.export", &json!({})).unwrap();
    let file: Value = serde_json::from_str(exported["data"].as_str().unwrap()).unwrap();
    assert_eq!(file["prefs"]["gpuPreference"], json!(saved));
    assert!(file["prefs"].get("gpuBackend").is_none());
}

/// Serde defaults alone do not make an AdapterKey meaningful. Hardware need not be present.
#[test]
fn gpu_backend_rejects_keys_without_a_backend_or_identity_at_every_preference_boundary() {
    let malformed = [
        json!({"kind":"adapter"}),
        json!({"kind":"adapter","name":"Disconnected"}),
        json!({"kind":"adapter","name":"Disconnected","backend":""}),
        json!({"kind":"adapter","name":"Disconnected","backend":"future"}),
        json!({"kind":"adapter","name":"Disconnected","backend":"noop"}),
        json!({"kind":"adapter","name":"Disconnected","backend":"Vulkan"}),
        json!({"kind":"adapter","backend":"vulkan"}),
        json!({"kind":"adapter","backend":"gl","name":"  "}),
    ];
    for value in malformed {
        let mut session = Session::new();
        session.execute("prefs.set", &json!({"key":"gpuPreference","value":unavailable_adapter()})).unwrap();
        let before = session.prefs.to_json();
        assert!(session.execute("prefs.set", &json!({"values":{"gpuPreference":value,"keyboardIncrement":4}})).is_err());
        assert_eq!(session.prefs.to_json(), before, "a semantically malformed key must reject the whole strict batch");
        let restored: Prefs = serde_json::from_value(json!({"gpuPreference":value,"keyboardIncrement":4})).unwrap();
        assert_eq!(restored.gpu_preference, Preference::Auto);
        assert_eq!(restored.keyboard_increment, 4.0, "loading a bad key must preserve valid siblings");
        session.execute("prefs.import", &imported_preferences(json!({"gpuPreference":value,"keyboardIncrement":5}))).unwrap();
        assert_eq!(session.prefs.gpu_preference, Preference::Auto);
        assert_eq!(session.prefs.keyboard_increment, 5.0);
        let before = session.prefs.to_json();
        assert!(session.execute("prefs.import", &imported_preferences(json!({"gpuPreference":value,"keyboardIncrement":true}))).is_err());
        assert_eq!(session.prefs.to_json(), before, "normalizing a bad key must not bypass another invalid imported value");
    }
    for value in [
        json!({"kind":"adapter","backend":"vulkan","vendor":4660,"device":22136}),
        json!({"kind":"adapter","backend":"dx12","vendor":4660}),
        json!({"kind":"adapter","backend":"metal","name":"Disconnected Metal adapter"}),
        json!({"kind":"adapter","backend":"gl","name":"Disconnected OpenGL adapter"}),
        json!({"kind":"adapter","backend":"webgpu","name":"Retained browser adapter"}),
    ] {
        // Old keys may omit the optional bus and driver. The SDK's defaulted fields remain so.
        let expected: Preference = serde_json::from_value(value.clone()).unwrap();
        let mut session = Session::new();
        session.execute("prefs.set", &json!({"key":"gpuPreference","value":value})).unwrap();
        assert_eq!(session.prefs.gpu_preference, expected);
        let saved: Prefs = serde_json::from_value(session.prefs.to_json()).unwrap();
        assert_eq!(saved.gpu_preference, expected);
        session.execute("prefs.import", &imported_preferences(json!({"gpuPreference":value,"keyboardIncrement":6}))).unwrap();
        assert_eq!(session.prefs.gpu_preference, expected);
        assert_eq!(session.prefs.keyboard_increment, 6.0);
    }
}
