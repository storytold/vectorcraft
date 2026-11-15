use vectorcraft_engine::Session;

use super::*;

fn frame(app: &mut VectorcraftApp) -> String {
    crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
}

fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
    app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
}

/// The native file of a document with a blue rectangle, assigned `rgb` (none: untagged).
fn file(rgb: Option<&str>) -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#3366cc"})).unwrap();
    if let Some(p) = rgb {
        s.execute("edit.assignProfile", &json!({ "rgb": p })).unwrap();
    }
    let r = s.execute("document.serialize", &json!({})).unwrap();
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
}

fn rgb_profile(app: &VectorcraftApp) -> Option<String> {
    vectorcraft_engine::cmd::colormgmt::doc_profiles(&app.session.doc().unwrap().doc).0
}

/// An open asked under automation roots finishes under them, though OK is pressed outside them: its
/// linked files are read, and the document is installed, with the roots of the open. `tagged` asks
/// with the settings in force.
fn open_asked_under_roots(app: &mut VectorcraftApp, tagged: &[u8]) {
    use vectorcraft_engine::file_access;
    let dir = crate::tests_folder::Folder::new("profile-mismatch-scope");
    let roots = file_access::AutomationRoots::new(&[dir.as_path()], &[]).unwrap();
    let r = file_access::confine(roots.as_ref(), || crate::io::open_bytes(app, "tagged.vectorcraft", tagged, None)).unwrap();
    assert_eq!(r, json!({ "dialog": OPEN_KIND }));
    assert!(file_access::current().is_none(), "OK is pressed outside the roots");
    super::super::confirm(app).unwrap();
    assert!(app.session.doc().unwrap().opened_by.is_some(), "installed under the roots of the open");
}

/// Another document becomes the active one while the paste question is open: OK pastes nothing, into
/// neither document, and the document the paste was asked in is active again afterward. `count`
/// counts the active document's objects.
fn paste_asked_before_another_document_is_active(app: &mut VectorcraftApp, count: impl Fn(&VectorcraftApp) -> usize) {
    let (asked_in, before) = (app.session.active_index().unwrap(), count(app));
    assert_eq!(app.run("edit.paste", json!({})).unwrap(), json!({ "dialog": PASTE_KIND }));
    app.run("document.activate", json!({"index": 0})).unwrap();
    let other = count(app);
    super::super::confirm(app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(count(app), other, "nothing pasted into the document now active");
    app.run("document.activate", json!({ "index": asked_in })).unwrap();
    assert_eq!(count(app), before, "nor into the one it was asked in");
}

/// With Color Settings' Ask boxes on, File › Open asks about a document whose profile is not the
/// working space, or that has none, and opens it as chosen; Cancel doesn't open it, and a refused
/// choice keeps the dialog. Paste asks about colors from another profile and pastes as chosen. With
/// the boxes off, nothing asks. An open asked under automation roots finishes under them, and a
/// paste asked in one document pastes nothing once another is active.
#[test]
fn opening_and_pasting_ask_about_profiles() {
    const CHILD: &str = "VECTORCRAFT_TEST_PROFILE_PROMPTS";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: change them in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "dialogs::profile_mismatch::tests::opening_and_pasting_ask_about_profiles", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated profile prompt test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let (tagged, plain) = (file(Some(cms::WIDE_GAMUT_RGB)), file(None));
    // The boxes are off: the document opens as the policy says.
    let r = crate::io::open_bytes(&mut app, "tagged.vectorcraft", &tagged, None).unwrap();
    assert!(r.get("index").is_some() && app.ui.dialog.is_none());
    assert_eq!(rgb_profile(&app).as_deref(), Some(cms::WIDE_GAMUT_RGB));

    // Profile Mismatches: Ask When Opening.
    app.run("edit.colorSettings", json!({"askMismatchOpen": true, "rgbPolicy": "convert"})).unwrap();
    let docs = app.session.documents().len();
    let r = crate::io::open_bytes(&mut app, "tagged.vectorcraft", &tagged, None).unwrap();
    assert_eq!(r, json!({ "dialog": OPEN_KIND }));
    assert_eq!(app.session.documents().len(), docs, "not open yet");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(
        (d.str("issue"), d.str("model"), d.str("profile"), d.str("choice")),
        ("mismatch".into(), "RGB".into(), cms::WIDE_GAMUT_RGB.into(), "convert".into())
    );
    let text = frame(&mut app);
    for label in [
        "Embedded Profile Mismatch",
        "Use the embedded profile",
        "Convert the colors to the working space",
        "Discard the embedded profile",
        cms::WIDE_GAMUT_RGB,
    ] {
        assert!(text.contains(label), "{label} in {text}");
    }
    // Cancel: nothing opens.
    super::super::cancel(&mut app);
    assert!(app.pending_profile_open.is_none());
    assert_eq!(app.session.documents().len(), docs);
    crate::io::open_bytes(&mut app, "tagged.vectorcraft", &tagged, None).unwrap();
    set(&mut app, "choice", json!("useEmbedded"));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none() && app.pending_profile_open.is_none());
    assert_eq!(app.session.documents().len(), docs + 1);
    assert_eq!(rgb_profile(&app).as_deref(), Some(cms::WIDE_GAMUT_RGB), "kept, though the policy converts");

    open_asked_under_roots(&mut app, &tagged);

    // Missing Profiles: Ask When Opening, and a choice the file refuses.
    app.run("edit.colorSettings", json!({"askMismatchOpen": false, "askMissingOpen": true, "rgbPolicy": "preserveEmbedded"})).unwrap();
    crate::io::open_bytes(&mut app, "plain.vectorcraft", &plain, None).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("issue"), d.str("choice")), (OPEN_KIND, "missing".into(), "leave".into()));
    let text = frame(&mut app);
    for label in ["Missing Profile", "Leave as is", "Assign the working space", "Assign profile:"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    set(&mut app, "choice", json!("assign"));
    set(&mut app, "assignProfile", json!(cms::DEVICE_CMYK));
    assert!(super::super::confirm(&mut app).is_err());
    assert!(app.ui.dialog.as_ref().is_some_and(|d| d.kind == OPEN_KIND) && app.pending_profile_open.is_some(), "still asking");
    set(&mut app, "assignProfile", json!(cms::DISPLAY_P3));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(rgb_profile(&app).as_deref(), Some(cms::DISPLAY_P3));
    assert!(app.session.doc().unwrap().is_dirty(), "assigning changed the document");

    // Profile Mismatches: Ask When Pasting.
    app.run("edit.colorSettings", json!({"askMissingOpen": false, "askMismatchPaste": true})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let r = app.run("edit.paste", json!({})).unwrap();
    assert_eq!(r, json!({ "dialog": PASTE_KIND }));
    let text = frame(&mut app);
    for label in ["Paste Profile Mismatch", "Preserve the color numbers", "Convert to keep the colors' appearance", cms::DISPLAY_P3] {
        assert!(text.contains(label), "{label} in {text}");
    }
    set(&mut app, "choice", json!("convert"));
    let r = super::super::confirm(&mut app).unwrap();
    assert_eq!(r["profileMismatch"][0]["choice"], "convert");
    assert!(app.ui.dialog.is_none());
    // Cancel pastes nothing; with the box off nothing asks.
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    let before = count(&app);
    app.run("edit.paste", json!({})).unwrap();
    super::super::cancel(&mut app);
    assert_eq!(count(&app), before);
    paste_asked_before_another_document_is_active(&mut app, count);
    app.run("edit.colorSettings", json!({"askMismatchPaste": false})).unwrap();
    let r = app.run("edit.paste", json!({})).unwrap();
    assert_eq!(r["profileMismatch"][0]["choice"], "preserveNumbers");
    assert_eq!(count(&app), before + 1);
}
