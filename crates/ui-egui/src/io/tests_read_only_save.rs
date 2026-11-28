//! Injected writers and picker answers: no native panel or filesystem permissions are needed.

use std::io::ErrorKind;
use std::sync::{Arc, Mutex, mpsc};

use serde_json::{Value, json};
use vectorcraft_engine::{EngineError, READ_ONLY_LOCATION, Session};

use super::{fileio, save};
use crate::dialogs::confirm;
use crate::tests::picks::{self, answer_dialog, answer_replace, open_dialogs, scripted};
use crate::{Services, VectorcraftApp, background};

const DOCUMENTS: &str = "/home/artist/Documents";

#[derive(Default)]
struct Files {
    original: String,
    attempts: Vec<String>,
    kind: Option<ErrorKind>,
    fail_second: bool,
}

fn write(files: &Mutex<Files>, path: &str) -> vectorcraft_engine::Result<()> {
    let mut files = files.lock().unwrap();
    files.attempts.push(path.into());
    if path == files.original || files.fail_second {
        let reason = format!("{path}: injected refusal");
        return Err(match files.kind {
            Some(ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem) => EngineError::ReadOnlyLocation(reason),
            _ => EngineError::Other(reason),
        });
    }
    Ok(())
}

fn app(format: &str, kind: ErrorKind) -> (VectorcraftApp, Arc<Mutex<Files>>) {
    let files = Arc::new(Mutex::new(Files { original: format!("/locked/Poster.{format}"), kind: Some(kind), ..Default::default() }));
    let typed = files.clone();
    let ordinary = files.clone();
    let services = Services {
        save_write: Some(Arc::new(move |path, _| write(&typed, path))),
        write: Some(Box::new(move |path, _| {
            write(&ordinary, path).map_err(|e| match e {
                EngineError::ReadOnlyLocation(reason) => reason,
                e => e.to_string(),
            })
        })),
        ..Default::default()
    };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.run("shape.rectangle", json!({"x": 1, "y": 1, "width": 10, "height": 10})).unwrap();
    let st = app.session.doc_mut().unwrap();
    st.path = Some(files.lock().unwrap().original.clone());
    st.format = fileio::format(format).unwrap().id;
    st.save_options = if format == "svg" {
        json!({"decimals": 2, "preserveEditing": false})
    } else {
        json!({"pdfCompatible": false, "compress": true, "preview": false})
    }
    .as_object()
    .unwrap()
    .clone();
    (app, files)
}

fn state(app: &VectorcraftApp) -> (Option<String>, bool, &'static str, Value) {
    let st = app.session.doc().unwrap();
    (st.path.clone(), st.is_dirty(), st.format, json!(st.save_options))
}

fn poll(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    picks::poll(app, &ctx);
}

fn normal_error(app: &VectorcraftApp) {
    let (_, detail) = confirm::open_message(app).expect("a persistent error dialog");
    assert!(detail.contains("injected refusal"));
    assert!(!detail.contains(READ_ONLY_LOCATION));
    assert_eq!(app.told.0.len(), 1, "no duplicate error dialog");
}

/// The fields of the open dialog with `key`, as automation lists them.
fn fields_of(app: &VectorcraftApp, key: &str) -> Option<Value> {
    open_dialogs(app).into_iter().rfind(|d| d.key.as_deref() == Some(key)).and_then(|d| d.fields)
}

#[test]
fn readonly_fallback_prefills_save_as_and_cancel_keeps_the_failed_save_state() {
    for kind in [ErrorKind::PermissionDenied, ErrorKind::ReadOnlyFilesystem] {
        let (mut app, files) = app("svg", kind);
        let before = state(&app);
        let picker = scripted(&mut app);
        let r = save::save_current_in(&mut app, &json!({"modified": null}), Some(DOCUMENTS.into())).unwrap();
        assert_eq!(r["pending"], "fileDialog");
        let requests = picker.requests();
        assert_eq!(requests.len(), 1, "expected one save panel");
        let request = &requests[0];
        assert_eq!(request.file_name.as_deref(), Some("Poster.svg"));
        assert_eq!(request.directory.as_deref(), Some(std::path::Path::new(DOCUMENTS)));
        assert!(request.filters.first().is_some_and(|f| f.extensions.iter().any(|e| e == "svg")));
        assert!(app.ui.status.contains("choose where to save it"));
        assert_eq!(state(&app), before);
        assert!(!app.any_dialog_open());
        picker.cancel();
        poll(&mut app);
        assert_eq!(state(&app), before);
        assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.svg"]);
        assert!(!app.file_dialog_open());
    }
}

#[test]
fn readonly_save_as_retargets_only_after_write_and_keeps_options() {
    let (mut app, files) = app("svg", ErrorKind::PermissionDenied);
    let before = state(&app);
    let uid = app.session.doc().unwrap().uid;
    let picker = scripted(&mut app);
    assert_eq!(app.run("file.save", json!({})).unwrap()["pending"], "fileDialog");
    // Hold the second write after the picker answers: the chosen path is not yet the document's.
    let (entered, writing) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let f = files.clone();
    app.services.save_write = Some(Arc::new(move |path, _| {
        entered.send(()).unwrap();
        released.lock().unwrap().recv().unwrap();
        write(&f, path)
    }));
    app.services.write_shared = Some(Arc::new(|_, _| panic!("typed saves must use their own writer")));
    app.session.prefs.background_save = true;
    picker.answer_saved("/home/artist/Documents/Poster.svg", None);
    poll(&mut app);
    writing.recv().unwrap();
    assert_eq!(state(&app), before);
    release.send(()).unwrap();
    background::wait_all(&mut app);
    let st = app.session.document_mut(uid).unwrap();
    assert_eq!(st.path.as_deref(), Some("/home/artist/Documents/Poster.svg"));
    assert!(!st.is_dirty());
    assert_eq!(st.format, before.2);
    assert_eq!(json!(st.save_options), before.3);
    assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.svg", "/home/artist/Documents/Poster.svg"]);
    assert!(picker.requests().len() == 1 && !app.any_dialog_open());
}

#[test]
fn readonly_second_failure_shows_one_dialog_without_reopening_the_picker() {
    let (mut app, files) = app("svg", ErrorKind::PermissionDenied);
    files.lock().unwrap().fail_second = true;
    let before = state(&app);
    let picker = scripted(&mut app);
    let _ = app.run("file.save", json!({}));
    picker.answer_saved("/elsewhere/Poster.svg", None);
    poll(&mut app);
    normal_error(&app);
    assert_eq!(state(&app), before);
    assert!(picker.requests().len() == 1 && !app.file_dialog_open());
    assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.svg", "/elsewhere/Poster.svg"]);
}

#[test]
fn readonly_other_errors_and_explicit_destinations_keep_the_dialog() {
    for (command, params, kind) in [
        ("file.save", json!({}), ErrorKind::StorageFull),
        ("file.save", json!({}), ErrorKind::NotFound),
        ("file.save", json!({}), ErrorKind::Other),
        ("file.save", json!({"path": "/locked/Poster.svg"}), ErrorKind::PermissionDenied),
        ("file.saveAs", json!({"path": "/locked/Poster.svg", "options": {"decimals": 2}}), ErrorKind::PermissionDenied),
        ("file.saveCopy", json!({"path": "/locked/Poster.svg", "options": {"decimals": 2}}), ErrorKind::PermissionDenied),
        ("file.exportAs", json!({"path": "/locked/Poster.svg", "format": "svg"}), ErrorKind::PermissionDenied),
    ] {
        let (mut app, _) = app("svg", kind);
        let before = state(&app);
        let picker = scripted(&mut app);
        assert!(app.run(command, params).is_err(), "{command}");
        normal_error(&app);
        assert_eq!(state(&app), before);
        assert!(picker.requests().is_empty(), "{command}: no fallback");
    }
}

#[test]
fn readonly_control_returns_the_prefix_and_never_opens_a_panel_even_after_completion() {
    for wait in [true, false] {
        let (mut app, _) = app("svg", ErrorKind::PermissionDenied);
        let picker = scripted(&mut app);
        app.services.write_shared = Some(Arc::new(|_, _| panic!("typed writer required")));
        app.session.prefs.background_save = true;
        let before = state(&app);
        let (tx, rx) = mpsc::channel();
        app = app.with_control(rx);
        let (req, reply) = craft::automation::ControlRequest::new("app.save", json!({"wait": wait}));
        tx.send(req).unwrap();
        app.run_control(&crate::testing::ui_context());
        let reply = reply.try_recv().unwrap();
        if wait {
            assert_eq!(reply["ok"], false);
            assert!(reply["error"].as_str().unwrap().starts_with("READ_ONLY_LOCATION: "), "{reply}");
        } else {
            assert_eq!(reply["result"]["background"], true);
            assert!(!app.ui.answering_control);
            background::wait_all(&mut app);
            assert!(app.ui.status.contains("READ_ONLY_LOCATION: "));
        }
        assert_eq!(state(&app), before);
        assert!(!app.any_dialog_open() && app.told.0.is_empty() && !app.file_dialog_open());
        assert!(picker.requests().is_empty(), "automation never picks");
    }
}

#[test]
fn readonly_automation_continuation_keeps_the_prefix_and_never_opens_a_panel() {
    let (mut app, files) = app("svg", ErrorKind::PermissionDenied);
    let before = state(&app);
    let picker = scripted(&mut app);
    assert!(!app.automation_is_acting());
    // PLAN: assumed `answering_control` is what `automation_is_acting` reads while a command runs for automation
    app.ui.answering_control = true;
    let r = app.run("file.save", json!({}));
    app.ui.answering_control = false;
    assert!(r.unwrap_err().starts_with("READ_ONLY_LOCATION: "));
    assert_eq!(state(&app), before);
    assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.svg"]);
    assert!(!app.any_dialog_open() && app.told.0.is_empty() && !app.file_dialog_open());
    assert!(picker.requests().is_empty(), "automation never picks");
}

#[test]
fn readonly_background_completion_resumes_the_original_uid_without_repeating_save() {
    for streamed in [false, true] {
        let format = if streamed { "vectorcraft" } else { "svg" };
        let (mut app, files) = app(format, ErrorKind::ReadOnlyFilesystem);
        let original = files.lock().unwrap().original.clone();
        let uid = app.session.doc().unwrap().uid;
        let picker = scripted(&mut app);
        if streamed {
            let f = files.clone();
            app.services.save_stream = Some(Arc::new(move |path, fill| {
                let mut bytes = Vec::new();
                fill(&mut bytes).unwrap();
                write(&f, path)
            }));
            app.services.save_write = Some(Arc::new(|_, _| panic!("native save must stream")));
        }
        app.services.write_shared = Some(Arc::new(|_, _| panic!("typed writer required")));
        app.session.prefs.background_save = true;
        assert_eq!(app.run("file.save", json!({})).unwrap()["background"], true);
        app.run("file.new", json!({"width": 20, "height": 20})).unwrap();
        let other = app.session.doc().unwrap().uid;
        background::wait_all(&mut app);
        assert_eq!(app.session.doc().unwrap().uid, uid);
        assert_eq!(picker.requests().len(), 1);
        // Switch again while the native panel is open: its answer belongs to the original uid.
        app.run("document.activate", json!({"index": 1})).unwrap();
        let chosen = format!("/elsewhere/Poster.{format}");
        picker.answer_saved(&chosen, None);
        poll(&mut app);
        background::wait_all(&mut app);
        assert_eq!(app.session.doc().unwrap().uid, uid);
        assert_eq!(app.session.doc().unwrap().path.as_deref(), Some(chosen.as_str()));
        assert!(!app.session.doc().unwrap().is_dirty());
        assert!(app.session.document_mut(other).unwrap().path.is_none());
        assert_eq!(files.lock().unwrap().attempts, [original, chosen]);
        assert_eq!(picker.requests().len(), 1);
    }
}

#[test]
fn readonly_busy_picker_and_save_changes_keep_the_existing_error_flow() {
    let (mut app, files) = app("svg", ErrorKind::PermissionDenied);
    let picker = scripted(&mut app);
    let _ = app.run("file.saveAs", json!({}));
    assert_eq!(picker.requests().len(), 1);
    // The document's save is waiting on its picker (one save flow per document): Save is refused
    // as in progress, writes nothing and opens no second panel or dialog.
    let busy = app.run("file.save", json!({})).unwrap_err();
    assert_eq!(busy, craft::dialogs::save::SAVE_IN_PROGRESS.english);
    assert!(files.lock().unwrap().attempts.is_empty());
    assert!(!app.any_dialog_open() && app.told.0.is_empty());
    assert_eq!(picker.requests().len(), 1);
    picker.cancel();
    poll(&mut app);
    assert!(!app.any_dialog_open());
    let ctx = crate::testing::ui_context();
    let _ = crate::unsaved::close(&mut app, 0);
    let save = open_dialogs(&app).pop().expect("the unsaved-changes question").id;
    answer_dialog(&mut app, save, &[], true);
    crate::unsaved::pump(&mut app, &ctx);
    assert!(app.ui.status.contains("injected refusal"), "{}", app.ui.status);
    assert_eq!(picker.requests().len(), 1);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.session.doc().unwrap().is_dirty());
}

#[test]
fn readonly_changed_format_asks_for_its_options_before_writing() {
    let (mut app, files) = app("svg", ErrorKind::PermissionDenied);
    let before = state(&app);
    let picker = scripted(&mut app);
    assert_eq!(app.run("file.save", json!({})).unwrap()["pending"], "fileDialog");
    picker.answer_saved("/elsewhere/Poster.pdf", None);
    poll(&mut app);
    assert!(fields_of(&app, "pdf-export").is_some(), "the picked format's Save dialog");
    let job = app.save_pdf.as_ref().and_then(|open| open.job.as_ref());
    assert_eq!(job.and_then(|job| job.path.as_deref()), Some("/elsewhere/Poster.pdf"));
    assert_eq!(state(&app), before);
    assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.svg"]);
}

/// Typed saves keep the sidecar preflight both before the refused write and at the retry's
/// destination. Confirming the ordinary Save still permits its one read-only fallback.
#[test]
fn readonly_typed_save_checks_sidecars_before_writing_and_after_retry() {
    for in_background in [false, true] {
        let (mut app, files) = app("vectorcraft", ErrorKind::PermissionDenied);
        app.run("artboard.new", json!({"x": 100, "y": 0, "width": 40, "height": 40, "name": "B"})).unwrap();
        app.session.doc_mut().unwrap().save_options.insert("separateArtboards".into(), json!(true));
        let before = state(&app);
        let there = ["/locked/Poster-B.vectorcraft", "/elsewhere/Poster-B.vectorcraft"];
        app.services.exists = Some(Arc::new(move |path| there.contains(&path)));
        app.services.write_shared = Some(Arc::new(|_, _| panic!("typed writer required")));
        app.session.prefs.background_save = in_background;
        let picker = scripted(&mut app);

        app.run("file.save", json!({})).unwrap();
        background::wait_all(&mut app);
        let asked = confirm::open_replace_files(&app).expect("ask before writing the existing artboard file");
        assert_eq!(asked, ["/locked/Poster-B.vectorcraft"]);
        assert!(files.lock().unwrap().attempts.is_empty());
        assert!(picker.requests().is_empty());
        assert_eq!(state(&app), before);
        answer_replace(&mut app, false);
        assert_eq!(state(&app), before);
        assert!(files.lock().unwrap().attempts.is_empty());

        app.run("file.save", json!({})).unwrap();
        background::wait_all(&mut app);
        answer_replace(&mut app, true);
        background::wait_all(&mut app);
        assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.vectorcraft"]);
        assert_eq!(state(&app), before);
        let requests = picker.requests();
        assert_eq!(requests.len(), 1, "expected one fallback panel");
        assert_eq!(requests[0].file_name.as_deref(), Some("Poster.vectorcraft"));

        picker.answer_saved("/elsewhere/Poster.vectorcraft", None);
        poll(&mut app);
        background::wait_all(&mut app);
        let asked = confirm::open_replace_files(&app).expect("the retry also checks its destination's artboard file");
        assert_eq!(asked, ["/elsewhere/Poster-B.vectorcraft"]);
        assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.vectorcraft"]);
        assert_eq!(state(&app), before);
        answer_replace(&mut app, true);
        background::wait_all(&mut app);
        assert_eq!(
            files.lock().unwrap().attempts,
            [
                "/locked/Poster.vectorcraft",
                "/elsewhere/Poster.vectorcraft",
                "/elsewhere/Poster-Artboard-1.vectorcraft",
                "/elsewhere/Poster-B.vectorcraft"
            ]
        );
        assert_eq!(app.session.doc().unwrap().path.as_deref(), Some("/elsewhere/Poster.vectorcraft"));
        assert!(!app.session.doc().unwrap().is_dirty());
        assert!(picker.requests().len() == 1 && !app.any_dialog_open());
    }
}
