//! VectorCraft's own Fonts folder: a missing font's file, found in a folder, is copied into it
//! without replacing a file there, and type set in the font redraws in it. One test in its own
//! process, so the process-wide font database and Fonts folder start fresh, as when the app starts.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::findfiles::{Rules, plain};

/// `text.findFontFiles {}` once the search ended (10 seconds at most).
fn finished(s: &mut Session) -> Value {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let r = s.execute("text.findFontFiles", &json!({})).unwrap();
        if r["state"] != "searching" || Instant::now() > until {
            return r;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_found_font_is_copied_without_replacing_a_file_and_its_type_redraws() {
    let root = vectorcraft_testkit::temp_dir("font-folder");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Downloads/Deep")).unwrap();
    let root = plain(std::fs::canonicalize(&root).unwrap());
    let fonts = root.join("App Fonts");
    vectorcraft_text::set_app_font_dir(fonts.clone());
    assert!(vectorcraft_text::system_font_dirs().contains(&fonts));
    let file = root.join("Downloads").join("Deep").join("Findme.otf");
    std::fs::write(&file, vectorcraft_testkit::fonts::renamed("Findme Sans 3")).unwrap();
    let serif = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/SourceSerif4-Regular.ttf");
    std::fs::copy(serif, root.join("other.ttf")).unwrap();

    let mut s = Session::new();
    s.search_rules = Some(Rules { user_content: vec![root.clone()], ..Rules::default() });
    s.search_threads = Some(2);
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Findme", "font": "Findme Sans 3"})).unwrap();
    let r = s.execute("text.missingFonts", &json!({})).unwrap();
    assert_eq!((r["fonts"][0]["family"].as_str(), r["fonts"][0]["status"].as_str()), (Some("Findme Sans 3"), Some("missing")), "{r}");
    s.doc_mut().unwrap().mark_saved();
    let revision = s.doc().unwrap().revision;

    s.execute("text.findFontFiles", &json!({"folder": root})).unwrap();
    let r = finished(&mut s);
    assert_eq!((&r["state"], &r["fonts"][0]["files"]), (&json!("done"), &json!([file])), "{r}");

    // A file of that name is in the Fonts folder already: it stays, and the copy takes a number.
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("Findme.otf"), b"keep").unwrap();
    let r = s.execute("text.addFontFiles", &json!({"files": [file]})).unwrap();
    assert_eq!(r["copied"], json!([{"from": file, "to": fonts.join("Findme 2.otf")}]), "{r}");
    assert!(r["faces"].as_u64().is_some_and(|n| n > 0) && r["families"].as_u64().is_some(), "{r}");
    assert_eq!(std::fs::read(fonts.join("Findme.otf")).unwrap(), b"keep");

    // The type is set in the font now, without editing the document.
    let listed = s.execute("text.fonts", &json!({})).unwrap();
    let findme = listed.as_array().unwrap().iter().find(|f| f["family"] == "Findme Sans 3").cloned().unwrap();
    assert_eq!((findme["status"].as_str(), findme["resolved"]["family"].as_str()), (Some("exact"), Some("Findme Sans 3")), "{findme}");
    let st = s.doc().unwrap();
    assert!(st.revision > revision && !st.is_dirty());
    assert_eq!(s.execute("text.missingFonts", &json!({})).unwrap()["count"], 0);

    // The same file again is kept, not copied a third time.
    let r = s.execute("text.addFontFiles", &json!({"files": [file]})).unwrap();
    assert_eq!((r["copied"].clone(), r["kept"].clone()), (json!([]), json!([{"from": file, "to": fonts.join("Findme 2.otf")}])), "{r}");
    assert!(!fonts.join("Findme 3.otf").exists());
    // Only files the last search found are copied.
    let r = s.execute("text.addFontFiles", &json!({"files": [root.join("other.ttf")]}));
    assert!(r.as_ref().is_err_and(|e| e.to_string().contains("the last text.findFontFiles search")), "{r:?}");
    let _ = std::fs::remove_dir_all(&root);
}
