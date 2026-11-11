//! Injected writers and picker answers: no native panel or filesystem permissions are needed.

use std::io::ErrorKind;
use std::sync::{Arc, Mutex, mpsc};

use serde_json::{Value, json};
use vectorcraft_engine::{EngineError, READ_ONLY_LOCATION, Session};

use super::{fileio, save};
use crate::control::ControlRequest;
use crate::tests::picks::off_the_ui_thread;
use crate::{Services, VectorcraftApp, background, dialogs, picks};

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
    let mut app = VectorcraftApp::new(Session::new(), services);
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
    picks::poll(app, &egui::Context::default());
}

fn normal_error(app: &VectorcraftApp) {
    let d = app.ui.dialog.as_ref().expect("a persistent error dialog");
    assert_eq!(d.kind, dialogs::confirm::MESSAGE);
    assert!(d.str("detail").contains("injected refusal"));
    assert!(!d.str("detail").contains(READ_ONLY_LOCATION));
    assert!(app.ui.told.is_empty(), "no duplicate error dialog");
}

#[test]
fn readonly_fallback_prefills_save_as_and_cancel_keeps_the_failed_save_state() {
    for kind in [ErrorKind::PermissionDenied, ErrorKind::ReadOnlyFilesystem] {
        let (mut app, files) = app("svg", kind);
        let before = state(&app);
        let (shown, _) = off_the_ui_thread(&mut app.services);
        let r = save::save_current_in(&mut app, &json!({"modified": null}), Some(DOCUMENTS.into())).unwrap();
        assert_eq!(r["pending"], "fileDialog");
        let requests = shown.requests();
        let [picks::PickRequest::Save(pick)] = requests.as_slice() else { panic!("expected one save panel: {requests:?}") };
        assert_eq!(pick.name, "Poster.svg");
        assert_eq!(pick.folder.as_deref(), Some(DOCUMENTS));
        assert_eq!(pick.filters.first(), crate::FilePick::filters(fileio::save_filters("svg")).first());
        assert!(app.ui.status.contains("choose where to save it"));
        assert_eq!(state(&app), before);
        assert!(app.ui.dialog.is_none());
        shown.answer(&[]);
        poll(&mut app);
        assert_eq!(state(&app), before);
        assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.svg"]);
        assert!(!app.picks.is_waiting());
    }
}

#[test]
fn readonly_save_as_retargets_only_after_write_and_keeps_options() {
    let (mut app, files) = app("svg", ErrorKind::PermissionDenied);
    let before = state(&app);
    let uid = app.session.doc().unwrap().uid;
    let (shown, _) = off_the_ui_thread(&mut app.services);
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
    shown.answer(&["/home/artist/Documents/Poster.svg"]);
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
    assert!(shown.requests().is_empty() && app.ui.dialog.is_none());
}

#[test]
fn readonly_second_failure_shows_one_dialog_without_reopening_the_picker() {
    let (mut app, files) = app("svg", ErrorKind::PermissionDenied);
    files.lock().unwrap().fail_second = true;
    let before = state(&app);
    let (shown, _) = off_the_ui_thread(&mut app.services);
    app.run("file.save", json!({})).unwrap();
    shown.answer(&["/elsewhere/Poster.svg"]);
    poll(&mut app);
    normal_error(&app);
    assert_eq!(state(&app), before);
    assert!(shown.requests().is_empty() && !app.picks.is_waiting());
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
        // A fallback would return pending, so no inline/native dialog is necessary here.
        app.services.pick_save = Some(Box::new(move |_| panic!("{command}: no fallback")));
        assert!(app.run(command, params).is_err(), "{command}");
        normal_error(&app);
        assert_eq!(state(&app), before);
    }
}

#[test]
fn readonly_control_returns_the_prefix_and_never_opens_a_panel_even_after_completion() {
    for wait in [true, false] {
        let (mut app, _) = app("svg", ErrorKind::PermissionDenied);
        app.services.pick_save = Some(Box::new(|_| panic!("automation never picks")));
        app.services.write_shared = Some(Arc::new(|_, _| panic!("typed writer required")));
        app.session.prefs.background_save = true;
        let before = state(&app);
        let (tx, rx) = mpsc::channel();
        app = app.with_control(rx);
        let (req, reply) = ControlRequest::new("app.save", json!({"wait": wait}));
        tx.send(req).unwrap();
        app.drain_control(&egui::Context::default());
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
        assert!(app.ui.dialog.is_none() && app.ui.told.is_empty() && !app.picks.is_waiting());
    }
}

#[test]
fn readonly_background_completion_resumes_the_original_uid_without_repeating_save() {
    for streamed in [false, true] {
        let format = if streamed { "vectorcraft" } else { "svg" };
        let (mut app, files) = app(format, ErrorKind::ReadOnlyFilesystem);
        let original = files.lock().unwrap().original.clone();
        let uid = app.session.doc().unwrap().uid;
        let (shown, _) = off_the_ui_thread(&mut app.services);
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
        assert_eq!(shown.requests().len(), 1);
        // Switch again while the native panel is open: its answer belongs to the original uid.
        app.run("document.activate", json!({"index": 1})).unwrap();
        let chosen = format!("/elsewhere/Poster.{format}");
        shown.answer(&[&chosen]);
        poll(&mut app);
        background::wait_all(&mut app);
        assert_eq!(app.session.doc().unwrap().uid, uid);
        assert_eq!(app.session.doc().unwrap().path.as_deref(), Some(chosen.as_str()));
        assert!(!app.session.doc().unwrap().is_dirty());
        assert!(app.session.document_mut(other).unwrap().path.is_none());
        assert_eq!(files.lock().unwrap().attempts, [original, chosen]);
        assert!(shown.requests().is_empty());
    }
}

#[test]
fn readonly_busy_picker_and_save_changes_keep_the_existing_error_flow() {
    let (mut app, _) = app("svg", ErrorKind::PermissionDenied);
    let (shown, _) = off_the_ui_thread(&mut app.services);
    app.run("file.saveAs", json!({})).unwrap_err();
    assert_eq!(shown.requests().len(), 1);
    assert!(app.run("file.save", json!({})).is_err());
    normal_error(&app);
    assert_eq!(shown.requests().len(), 1);
    shown.answer(&[]);
    poll(&mut app);
    app.ui.dialog = None;
    let st = app.session.doc().unwrap();
    app.ui.dialog = Some(crate::state::Dialog::new(crate::unsaved::KIND, json!({"index": 0, "uid": st.uid, "then": "close"})));
    assert!(crate::unsaved::confirm(&mut app).is_err());
    normal_error(&app);
    assert!(shown.requests().is_empty());
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.session.doc().unwrap().is_dirty());
}

#[test]
fn readonly_changed_format_asks_for_its_options_before_writing() {
    let (mut app, files) = app("svg", ErrorKind::PermissionDenied);
    let before = state(&app);
    let (shown, _) = off_the_ui_thread(&mut app.services);
    assert_eq!(app.run("file.save", json!({})).unwrap()["pending"], "fileDialog");
    shown.answer(&["/elsewhere/Poster.pdf"]);
    poll(&mut app);
    let dialog = app.ui.dialog.as_ref().expect("the picked format's Save dialog");
    assert_eq!(dialog.kind, "savePdf");
    assert_eq!(dialog.str("path"), "/elsewhere/Poster.pdf");
    assert_eq!(state(&app), before);
    assert_eq!(files.lock().unwrap().attempts, ["/locked/Poster.svg"]);
}
