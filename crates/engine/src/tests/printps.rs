//! File → Print to a PostScript file: `file.print {format: "postscript"}` writes the job the PDF
//! print lays out as DSC-conforming PostScript.

use serde_json::{Value, json};

use crate::*;

/// A document of `artboards` 300 × 200 artboards with a rectangle on the first.
fn session(artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": artboards})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    s
}

/// `file.print {format: "postscript", …p}` → (the PostScript, the answer).
fn print_ps(s: &mut Session, mut p: Value) -> (String, Value) {
    p["format"] = json!("postscript");
    let v = s.execute("file.print", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    (String::from_utf8(bytes).unwrap(), v)
}

/// The values of DSC comment `key` (`%%Key:`), in order.
fn dsc(ps: &str, key: &str) -> Vec<String> {
    let prefix = format!("%%{key}:");
    ps.lines().filter_map(|l| l.strip_prefix(&prefix)).map(|v| v.trim().to_string()).collect()
}

#[test]
fn pages_equals_the_page_count() {
    let mut s = session(3);
    let (ps, v) = print_ps(&mut s, json!({}));
    assert_eq!((v["pages"].clone(), v["format"].clone()), (json!(3), json!("postscript")));
    assert!(ps.starts_with("%!PS-Adobe-3.0\n") && ps.ends_with("%%EOF\n"));
    assert_eq!(dsc(&ps, "Pages"), ["3"]);
    assert_eq!(dsc(&ps, "Page").len(), 3);
    assert_eq!(dsc(&ps, "LanguageLevel"), ["3"]);
    // Copies, a range of artboards, and the level.
    let (ps, v) = print_ps(&mut s, json!({"level": 2, "settings": {"copies": 2, "artboards": "range", "range": "1-2"}}));
    assert_eq!((v["pages"].clone(), dsc(&ps, "Pages")), (json!(4), vec!["4".to_string()]));
    assert_eq!(dsc(&ps, "Page").len(), 4);
    assert_eq!(dsc(&ps, "LanguageLevel"), ["2"]);
    // Letter paper, turned to the wide artboards.
    assert_eq!(dsc(&ps, "PageBoundingBox")[0], "0 0 792 612");
}

#[test]
fn marks_and_separations_are_written() {
    let mut s = session(1);
    let (ps, _) = print_ps(&mut s, json!({"settings": {"marks": {"trim": true, "registration": true}}}));
    // Marks print in Registration: on every ink.
    assert!(ps.contains("/All /DeviceCMYK"), "{ps}");
    let (none, _) = print_ps(&mut s, json!({}));
    assert!(!none.contains("/All /DeviceCMYK"));
    // Separations: a page per ink with its screen.
    let (ps, v) =
        print_ps(&mut s, json!({"settings": {"output": {"mode": "separations", "inks": [{"name": "Cyan", "frequency": 120, "angle": 30}]}}}));
    assert_eq!(v["pages"], 4);
    assert_eq!(dsc(&ps, "PlateColor"), ["Cyan", "Magenta", "Yellow", "Black"]);
    assert!(ps.contains("120 30 {dup mul exch dup mul add 1 exch sub} setscreen"));
    // PostScript writes the screens, so no warning says they are left to the device.
    assert!(!v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("screens")), "{v}");
    // Negatives and a fixed flatness.
    let (ps, _) = print_ps(&mut s, json!({"settings": {"output": {"image": "negative"}, "graphics": {"autoFlatness": false, "flatness": 4}}}));
    assert!(ps.contains("{1 exch sub} settransfer") && ps.contains("4 setflat"));
}

#[test]
fn a_ps_path_prints_postscript_and_pdf_stays_the_default() {
    let mut s = session(1);
    let v = s.execute("file.print", &json!({})).unwrap();
    assert_eq!(v["format"], "pdf");
    let path = std::env::temp_dir().join(format!("vc-printps-{}.ps", std::process::id()));
    let v = s.execute("file.print", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(v["format"], "postscript");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%!PS-Adobe-3.0"));
    let _ = std::fs::remove_file(&path);
    // The file reads back: the rectangle on the first page.
    let back = vectorcraft_eps::import(&bytes).unwrap();
    assert!(!back.document.layers[0].children().unwrap().is_empty());
    for bad in [json!({"format": "ps2"}), json!({"format": "postscript", "level": 1}), json!({"format": "postscript", "flattenerPreset": "nope"})] {
        assert!(matches!(s.execute("file.print", &bad), Err(EngineError::BadParams { .. })), "{bad}");
    }
}

#[test]
fn transparency_is_flattened_for_postscript() {
    let mut s = session(1);
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let (ps, v) = print_ps(&mut s, json!({}));
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("flattened")), "{v}");
    assert!(!v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("opaque: EPS")), "{v}");
    assert_eq!(dsc(&ps, "Pages"), ["1"]);
}

#[test]
fn type_in_a_missing_family_is_named_when_its_artboard_prints() {
    let mut s = session(2);
    let r = s.doc().unwrap().doc.artboards[1].rect;
    s.execute("text.create", &json!({"x": r.x0 + 10.0, "y": r.y0 + 50.0, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let fallback = |v: &Value| v["warnings"].as_array().unwrap().iter().filter(|w| w.as_str().unwrap().contains("fallback font")).count();
    let (_, v) = print_ps(&mut s, json!({}));
    assert_eq!(fallback(&v), 1, "{v}");
    assert!(v["warnings"].to_string().contains("Nonexistent Sans Probe"), "{v}");
    let (_, v) = print_ps(&mut s, json!({"settings": {"artboards": "range", "range": "1"}}));
    assert_eq!(fallback(&v), 0, "{v}");
    // Off the paper: 100 pt wide paper at 100% shows the middle of the 300 pt wide artboard.
    let paper = json!({"artboards": "range", "range": "2", "media": "custom", "width": 100, "height": 300, "autoRotate": false});
    let (_, v) = print_ps(&mut s, json!({ "settings": paper }));
    assert_eq!(fallback(&v), 0, "{v}");
}

#[test]
fn type_that_does_not_fit_its_frame_is_named_when_its_artboard_prints() {
    let mut s = session(2);
    let r = s.doc().unwrap().doc.artboards[1].rect;
    let p = json!({"x": r.x0 + 10.0, "y": r.y0 + 10.0, "text": "Ab\nCd\nEf\nGh", "size": 12, "area": {"width": 100, "height": 20}});
    let id = s.execute("text.create", &p).unwrap()["id"].clone();
    let over = |v: &Value| v["warnings"].as_array().unwrap().iter().filter(|w| w.as_str().unwrap().contains("fit its frame")).count();
    let (_, v) = print_ps(&mut s, json!({}));
    assert_eq!(over(&v), 1, "{v}");
    assert!(v["warnings"].to_string().contains(&format!("(id {id})")), "{v}");
    let (_, v) = print_ps(&mut s, json!({"settings": {"artboards": "range", "range": "1"}}));
    assert_eq!(over(&v), 0, "{v}");
}
