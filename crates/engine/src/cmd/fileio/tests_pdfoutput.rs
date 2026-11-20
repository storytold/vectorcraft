//! `document.exportPdf` output and advanced options reach the writer: colours converted and tagged
//! with profiles, the output intent written, and type as real text.

use serde_json::{Value, json};

use super::*;

fn pdf(s: &mut Session, p: Value) -> (String, Value) {
    let v = s.execute("document.exportPdf", &p).unwrap();
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    (String::from_utf8_lossy(&bytes).into_owned(), v["warnings"].clone())
}

#[test]
fn export_pdf_converts_colours_writes_the_output_intent_and_real_text() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"model": "rgb", "r": 1.0, "g": 0.0, "b": 0.0}})).unwrap();
    s.execute("text.create", &json!({"x": 80, "y": 60, "text": "Press ready"})).unwrap();
    let generic = vectorcraft_color::cms::GENERIC_CMYK;
    let p = json!({
        "preset": "VectorCraft Default",
        "compression": {"compressText": false},
        "output": {"conversion": "destination", "destination": generic, "profiles": "destination", "outputIntent": generic, "outputConditionId": "Generic press", "trapped": true},
        "advanced": {"outlineText": false},
    });
    let (text, warnings) = pdf(&mut s, p.clone());
    assert_eq!(warnings, json!([]), "everything asked for is applied");
    assert!(text.contains("/ICCBased") && text.contains("/N 4"), "CMYK colours tagged with the destination");
    assert!(text.contains("/S/GTS_PDFX") && text.contains("/Trapped/True"));
    assert!(text.contains("/ToUnicode") && (text.contains("/FontFile2") || text.contains("/FontFile3")), "real text");
    // The Summary has nothing to report either.
    let mut q = p;
    q["includeDocument"] = json!(true);
    assert_eq!(s.execute("document.pdfSettings", &q).unwrap()["warnings"], json!([]));
    // Converting to a profile that isn't there is a bad parameter.
    let bad = s.execute("document.exportPdf", &json!({"output": {"conversion": "destination", "destination": "Missing"}}));
    assert!(matches!(bad, Err(crate::EngineError::BadParams { .. })), "{bad:?}");
}

/// Over a saved preset whose Profiles setting is `none`, `"default"` leaves the setting unset,
/// while `null` keeps the preset's value; an unknown value is a bad parameter.
#[test]
fn profiles_default_unsets_a_presets_profiles_setting() {
    use vectorcraft_pdf::{PdfPreset, PdfSettings, ProfileInclusion};
    let mut settings = PdfSettings::default();
    settings.output.profiles = Some(ProfileInclusion::None);
    let saved = [PdfPreset { name: "No profiles".into(), description: String::new(), settings }];
    let resolve = |output: Value| pdf::resolve("document.exportPdf", &json!({"preset": "No profiles", "output": output}), &saved);
    assert_eq!(resolve(json!({"profiles": "default"})).unwrap().output.profiles, None);
    assert_eq!(resolve(json!({"profiles": null})).unwrap().output.profiles, Some(ProfileInclusion::None));
    assert_eq!(resolve(json!({"profiles": "all"})).unwrap().output.profiles, Some(ProfileInclusion::All));
    let bad = resolve(json!({"profiles": "sometimes"}));
    assert!(matches!(bad, Err(crate::EngineError::BadParams { .. })), "{bad:?}");
}
