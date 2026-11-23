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

/// OK on the open question, run by `confirm`, opens one document, installed with `roots`.
fn confirm_opens_one_under(
    app: &mut VectorcraftApp,
    roots: Option<&std::sync::Arc<vectorcraft_engine::file_access::AutomationRoots>>,
    confirm: impl FnOnce(&mut VectorcraftApp) -> Value,
) {
    let docs = app.session.documents().len();
    let r = confirm(app);
    assert_eq!(app.session.documents().len(), docs + 1, "{r}");
    let opened = &app.session.documents()[r["index"].as_u64().unwrap() as usize];
    assert_eq!(opened.opened_by.as_ref().map(|o| o.describe()), roots.map(|o| o.describe()));
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
    confirm_opens_one_under(app, roots.as_ref(), |app| super::super::confirm(app).unwrap());
}

/// An open asked outside automation roots and confirmed by a caller under them finishes under that
/// caller's roots: the document is installed with them. `tagged` asks with the settings in force.
fn open_confirmed_under_roots(app: &mut VectorcraftApp, tagged: &[u8]) {
    use vectorcraft_engine::file_access;
    let dir = crate::tests_folder::Folder::new("profile-mismatch-confirm-scope");
    let roots = file_access::AutomationRoots::new(&[dir.as_path()], &[]).unwrap();
    let r = crate::io::open_bytes(app, "tagged.vectorcraft", tagged, None).unwrap();
    assert_eq!(r, json!({ "dialog": OPEN_KIND }));
    confirm_opens_one_under(app, roots.as_ref(), |app| file_access::confine(roots.as_ref(), || super::super::confirm(app)).unwrap());
}

/// A PNG of one color, 8 × 8.
fn png(rgb: [u8; 3]) -> Vec<u8> {
    let [r, g, b] = rgb;
    let layout = (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8);
    let img = craft_image_codecs::Image::from_raw(8, 8, layout.0, layout.1, [r, g, b, 255].repeat(64)).unwrap();
    craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap()
}

/// The linked files of a document asked about under automation roots, and confirmed outside them,
/// are read under those roots, apart from where the document is installed: of two linked files
/// changed since the save, the one inside the roots updates, and the one outside keeps the picture
/// the document holds. The Ask When Opening box for a mismatch is on.
fn links_read_under_the_roots_of_the_open(app: &mut VectorcraftApp) {
    use vectorcraft_engine::file_access;
    let dir = crate::tests_folder::Folder::new("profile-mismatch-links");
    let (inside, outside) = (dir.join("inside"), dir.join("outside"));
    let (a, b) = (inside.join("in.png"), outside.join("out.png"));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    for p in [&a, &b] {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, png([200, 40, 40])).unwrap();
        s.execute("file.place", &json!({"path": p.to_str().unwrap(), "at": [50, 50]})).unwrap();
    }
    s.execute("edit.assignProfile", &json!({ "rgb": cms::WIDE_GAMUT_RGB })).unwrap();
    let r = s.execute("document.serialize", &json!({})).unwrap();
    let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    // The profiles in use are the process's: back to the working spaces.
    s.execute("edit.assignProfile", &json!({ "rgb": null })).unwrap();
    for p in [&a, &b] {
        let stamp = std::fs::metadata(p).unwrap().modified().unwrap();
        std::fs::write(p, png([40, 40, 200])).unwrap();
        // Link stamps use milliseconds: the new file is dated 2 s later.
        let later = std::fs::FileTimes::new().set_modified(stamp + std::time::Duration::from_secs(2));
        std::fs::OpenOptions::new().write(true).open(p).unwrap().set_times(later).unwrap();
    }
    let update = std::mem::replace(&mut app.session.prefs.update_links, "automatically".into());
    let roots = file_access::AutomationRoots::new(&[inside.as_path()], &[]).unwrap();
    let r = file_access::confine(roots.as_ref(), || crate::io::open_bytes(app, "linked.vectorcraft", &bytes, None)).unwrap();
    assert_eq!(r, json!({ "dialog": OPEN_KIND }));
    let r = super::super::confirm(app).unwrap();
    let updated: Vec<&str> = r["updatedLinks"].as_array().map(|a| a.iter().filter_map(|l| l["name"].as_str()).collect()).unwrap_or_default();
    assert_eq!(updated, ["in.png"], "{r}");
    app.session.prefs.update_links = update;
    // A question about the link outside the roots is not part of this check.
    if app.ui.dialog.is_some() {
        super::super::cancel(app);
    }
}

/// A file read while another one's question is open waits and asks its own question in turn, and
/// each opens only after its own answer: Cancel on the first leaves it closed and asks about the
/// second. `tagged` asks with the settings in force.
fn files_opened_together_ask_in_turn(app: &mut VectorcraftApp, tagged: &[u8]) {
    let docs = app.session.documents().len();
    assert_eq!(crate::io::open_bytes(app, "first.vectorcraft", tagged, None).unwrap(), json!({ "dialog": OPEN_KIND }));
    let r = crate::io::open_bytes(app, "second.vectorcraft", tagged, None).unwrap();
    assert_eq!(r, json!({ "dialog": OPEN_KIND, "queued": true }));
    assert_eq!(app.queued_profile_opens.len(), 1);
    super::super::cancel(app);
    assert!(app.ui.dialog.as_ref().is_some_and(|d| d.kind == OPEN_KIND) && app.pending_profile_open.is_some(), "the second asks");
    assert!(app.queued_profile_opens.is_empty());
    assert_eq!(app.session.documents().len(), docs, "neither is open");
    super::super::confirm(app).unwrap();
    assert_eq!(app.session.documents().len(), docs + 1);
    assert!(app.ui.dialog.is_none() && app.pending_profile_open.is_none());
}

/// Color Settings change the open question while it is open: OK asks the question as it is now
/// (another working space) instead of opening, and opens by the policy once there is no question
/// (the working space is the file's profile). `tagged` (Wide Gamut RGB) asks with the settings in
/// force, the working space sRGB.
fn an_answer_waits_for_the_question_it_answers(app: &mut VectorcraftApp, tagged: &[u8]) {
    let docs = app.session.documents().len();
    crate::io::open_bytes(app, "tagged.vectorcraft", tagged, None).unwrap();
    app.run("edit.colorSettings", json!({"rgb": cms::DISPLAY_P3})).unwrap();
    assert_eq!(super::super::confirm(app).unwrap(), json!({ "dialog": OPEN_KIND }));
    assert_eq!(app.session.documents().len(), docs, "not opened under the old question");
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("working"), cms::canonical_name(cms::DISPLAY_P3), "asked again");
    app.run("edit.colorSettings", json!({"rgb": cms::WIDE_GAMUT_RGB})).unwrap();
    let r = super::super::confirm(app).unwrap();
    assert!(r.get("index").is_some() && app.ui.dialog.is_none(), "{r}");
    assert_eq!(app.session.documents().len(), docs + 1);
}

/// Swatch Conflict asks again when the conflicts a converting paste raises change while it is open:
/// the destination is assigned another profile, so the pasted swatch converts to another color. OK
/// then pastes nothing and asks about the new conflict; OK on that one pastes as chosen.
fn swatch_conflicts_are_asked_again_when_they_change(app: &mut VectorcraftApp) {
    use super::super::swatch_conflict;
    let swatch = json!({"name": "Brand", "color": "#3366cc", "global": true});
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("swatch.new", swatch.clone()).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Brand"})).unwrap();
    app.run("edit.assignProfile", json!({ "rgb": cms::WIDE_GAMUT_RGB })).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("swatch.new", swatch).unwrap();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    let before = count(app);
    let r = app.run("edit.paste", json!({"profileMismatch": "convert"})).unwrap();
    assert_eq!(r["dialog"], swatch_conflict::KIND, "{r}");
    let asked = app.ui.dialog.as_ref().unwrap().fields["conflicts"].clone();
    set(app, "choice", json!("add"));
    app.run("edit.assignProfile", json!({ "rgb": cms::DISPLAY_P3 })).unwrap();
    let r = super::super::confirm(app).unwrap();
    assert_eq!(r["dialog"], swatch_conflict::KIND, "asked again: {r}");
    assert_eq!(count(app), before, "nothing pasted under the old list");
    assert_ne!(app.ui.dialog.as_ref().unwrap().fields["conflicts"], asked);
    set(app, "choice", json!("add"));
    let r = super::super::confirm(app).unwrap();
    assert_eq!((count(app), r["renamed"].as_array().map(Vec::len)), (before + 1, Some(1)), "{r}");
}

/// Cancel on the paste question pastes nothing, and a null `profileMismatch` asks as leaving it out
/// does. `count` counts the active document's objects.
fn paste_canceled(app: &mut VectorcraftApp, count: impl Fn(&VectorcraftApp) -> usize) {
    let before = count(app);
    for params in [json!({}), json!({ "profileMismatch": null })] {
        assert_eq!(app.run("edit.paste", params.clone()).unwrap(), json!({ "dialog": PASTE_KIND }), "{params} asks");
        super::super::cancel(app);
        assert_eq!(count(app), before);
    }
}

/// Another document becomes the active one while the paste question is open: OK pastes nothing, into
/// neither document, whichever is active.
fn paste_asked_before_another_document_is_active(app: &mut VectorcraftApp) {
    let counts = |app: &VectorcraftApp| -> Vec<(u64, usize)> {
        app.session.documents().iter().map(|st| (st.uid, st.doc.layers[0].children().map_or(0, |c| c.len()))).collect()
    };
    let (asked_in, before) = (app.session.active_index().unwrap(), counts(app));
    assert_eq!(app.run("edit.paste", json!({})).unwrap(), json!({ "dialog": PASTE_KIND }));
    app.run("document.activate", json!({"index": 0})).unwrap();
    super::super::confirm(app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(counts(app), before, "nothing pasted into any document");
    app.run("document.activate", json!({ "index": asked_in })).unwrap();
}

/// A drag of the art of the document assigned Display P3 into the active one asks as Paste does:
/// Cancel drops nothing, and OK drops it as chosen. When the source document's profile changes while
/// the question is open, OK drops nothing and asks again about the new profile. `count` counts the
/// active document's objects.
fn drop_asks_as_paste_does(app: &mut VectorcraftApp, count: impl Fn(&VectorcraftApp) -> usize) {
    let p3 = Some(cms::DISPLAY_P3);
    let docs = app.session.documents();
    let at = docs.iter().position(|st| vectorcraft_engine::cmd::colormgmt::doc_profiles(&st.doc).0.as_deref() == p3).unwrap();
    let source = &docs[at];
    let ids: Vec<u64> = source.doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect();
    let n = ids.len();
    let drop = json!({"sourceDocument": source.uid, "ids": ids, "grab": [0, 0], "at": [50, 50]});
    let before = count(app);
    assert_eq!(app.run(DROP, drop.clone()).unwrap(), json!({ "dialog": PASTE_KIND }));
    assert_eq!(count(app), before, "nothing dropped yet");
    super::super::cancel(app);
    assert_eq!(count(app), before, "Cancel drops nothing");
    assert_eq!(app.run(DROP, drop).unwrap(), json!({ "dialog": PASTE_KIND }));
    // The source document is assigned another profile before OK.
    let asked_in = app.session.active_index().unwrap();
    app.run("document.activate", json!({ "index": at })).unwrap();
    app.session.execute("edit.assignProfile", &json!({ "rgb": cms::WIDE_GAMUT_RGB })).unwrap();
    app.run("document.activate", json!({ "index": asked_in })).unwrap();
    assert_eq!(super::super::confirm(app).unwrap(), json!({ "dialog": PASTE_KIND }));
    assert_eq!(count(app), before, "nothing dropped under the old question");
    let asked = &app.ui.dialog.as_ref().unwrap().fields["mismatches"][0]["clipboard"];
    assert_eq!(asked, &json!(cms::WIDE_GAMUT_RGB), "asked again about the new profile");
    set(app, "choice", json!("convert"));
    let r = super::super::confirm(app).unwrap();
    assert_eq!(r["profileMismatch"][0]["choice"], "convert");
    assert!(app.ui.dialog.is_none());
    assert_eq!(count(app), before + n);
}

/// A paste question about both models has a choice for each, starting on its model's policy, and
/// picking one model's choice in the dialog leaves the other's as it was; OK checks the choice.
#[test]
fn a_paste_question_about_both_models_has_a_choice_for_each() {
    let both = [
        json!({"model": "rgb", "clipboard": "Wide RGB", "document": "sRGB", "default": "preserveNumbers", "convertible": true}),
        json!({"model": "cmyk", "clipboard": "Ink A", "document": "Ink B", "default": "convert", "convertible": true}),
    ];
    assert_eq!(first_choice(&both), json!({"rgb": "preserveNumbers", "cmyk": "convert"}));
    assert_eq!(first_choice(&both[1..]), json!("convert"), "one model starts on its policy");
    for (choice, ok) in [
        (json!({"rgb": "convert", "cmyk": "preserveNumbers"}), true),
        (json!("convert"), true),
        (json!({"rgb": "convert"}), false),
        (json!({"rgb": "convert", "cmyk": "keep"}), false),
        (json!({"rgb": "convert", "cmyk": "convert", "lab": "convert"}), false),
        (json!(""), false),
    ] {
        assert_eq!(valid_choice(&choice, &both), ok, "{choice}");
    }
    // Picking CMYK's choice keeps RGB's, also when one choice was set for both.
    let start = first_choice(&both);
    assert_eq!(pick(&start, &both, "cmyk", "preserveNumbers"), json!({"rgb": "preserveNumbers", "cmyk": "preserveNumbers"}));
    assert_eq!(pick(&json!("convert"), &both, "rgb", "preserveNumbers"), json!({"rgb": "preserveNumbers", "cmyk": "convert"}));
    assert_eq!(pick(&json!("convert"), &both[..1], "rgb", "preserveNumbers"), json!("preserveNumbers"));
    // The dialog draws a pair of choices under each model's line.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let fields = json!({ "mismatches": both, "choice": start, "__command": "edit.paste", "__params": {} });
    app.ui.dialog = Some(Dialog::new(PASTE_KIND, fields));
    let text = frame(&mut app);
    assert_eq!(text.matches("Preserve the color numbers").count(), 2, "{text}");
}

/// With Color Settings' Ask boxes on, File › Open asks about a document whose profile is not the
/// working space, or that has none, and opens it as chosen; Cancel doesn't open it, and a refused
/// choice keeps the dialog. Paste, and a drag from another document, ask about colors from another
/// profile and paste or drop as chosen. With the boxes off, nothing asks. An open asked or confirmed
/// under automation roots finishes under them, and a paste asked in one document pastes nothing once
/// another is active.
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
    open_confirmed_under_roots(&mut app, &tagged);
    links_read_under_the_roots_of_the_open(&mut app);
    files_opened_together_ask_in_turn(&mut app, &tagged);

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
    paste_canceled(&mut app, count);
    paste_asked_before_another_document_is_active(&mut app);
    drop_asks_as_paste_does(&mut app, count);
    let before = count(&app);
    app.run("edit.colorSettings", json!({"askMismatchPaste": false})).unwrap();
    let r = app.run("edit.paste", json!({})).unwrap();
    assert_eq!(r["profileMismatch"][0]["choice"], "preserveNumbers");
    assert_eq!(count(&app), before + 1);
    swatch_conflicts_are_asked_again_when_they_change(&mut app);
    all_artboards_leaves_backgrounds_out(&mut app);
    // It changes the working space: last.
    app.run("edit.colorSettings", json!({"askMismatchOpen": true})).unwrap();
    an_answer_waits_for_the_question_it_answers(&mut app, &tagged);
}

/// Convert is offered only for a model whose colors can convert (`convertible`).
#[test]
fn convert_needs_both_profiles() {
    let stuck = json!({"model": "rgb", "clipboard": "Gone", "document": "sRGB", "default": "preserveNumbers", "convertible": false});
    let free = json!({"model": "rgb", "clipboard": "Wide RGB", "document": "sRGB", "default": "convert", "convertible": true});
    assert!(allowed(&stuck, "preserveNumbers") && !allowed(&stuck, "convert"));
    assert!(allowed(&free, "convert") && allowed(&json!({"model": "rgb"}), "convert"));
}

/// Paste on All Artboards asks about what it brings, without the backgrounds of copied artboards.
fn all_artboards_leaves_backgrounds_out(app: &mut VectorcraftApp) {
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("artboard.setProps", json!({"index": 0, "color": "#3366cc"})).unwrap();
    app.run("edit.assignProfile", json!({ "rgb": cms::WIDE_GAMUT_RGB })).unwrap();
    app.run("artboard.copy", json!({"index": 0})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let listed = |app: &mut VectorcraftApp, command: &str| paste_mismatches(app, command, &json!({})).map(|m| m.len());
    assert_eq!(listed(app, "edit.paste"), Some(1), "the background is RGB");
    assert_eq!(listed(app, ALL_ARTBOARDS), Some(0));
}
