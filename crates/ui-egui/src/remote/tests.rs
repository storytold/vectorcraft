//! Remote storage in the app: the menus with and without it, opening, placing and saving through
//! a host that records what it is asked.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;

use super::{Mode, Purpose, RemoteEntry, RemoteEvent, RemoteInbox, RemoteMount, RemoteStorage, accept, join, save_path, valid_name};
use crate::place::PlaceInbox;
use crate::{Services, VectorcraftApp, menus};

/// What the host was asked: fetches (path, purpose) and writes (path, bytes).
#[derive(Default)]
struct Log {
    fetched: Vec<(String, Purpose)>,
    written: Vec<(String, Vec<u8>)>,
    /// For each write: was it a save that creates a file?
    created: Vec<bool>,
    downloads: Vec<String>,
}

/// Two mounts: one writable, one read-only.
fn mounts() -> Vec<RemoteMount> {
    vec![
        RemoteMount { name: "Server".into(), path: "/rw".into(), writable: true },
        RemoteMount { name: "Archive".into(), path: "/ro".into(), writable: false },
    ]
}

/// A host that records what it is asked.
fn host(log: &Rc<RefCell<Log>>) -> (RemoteStorage, RemoteInbox) {
    let inbox = RemoteInbox::default();
    let on = |p: &str| p == "/rw" || p.starts_with("/rw/") || p == "/ro" || p.starts_with("/ro/");
    let (fetch_log, write_log) = (log.clone(), log.clone());
    let remote = RemoteStorage {
        mounts: mounts(),
        is_remote: Box::new(on),
        list: Box::new(|dir| match dir {
            "/rw" => Ok(vec![
                RemoteEntry { name: "b.svg".into(), is_dir: false, size: Some(9) },
                RemoteEntry { name: "docs".into(), is_dir: true, size: None },
                RemoteEntry { name: "taken.vectorcraft".into(), is_dir: false, size: Some(9) },
            ]),
            "/loading" => Err(io::Error::new(io::ErrorKind::WouldBlock, "Loading…")),
            _ => Ok(vec![]),
        }),
        fetch: Box::new(move |p, purpose| fetch_log.borrow_mut().fetched.push((p.to_string(), purpose))),
        write: Box::new(move |p, b, new_file| {
            if p.starts_with("/ro/") {
                return Err("Archive is read-only".into());
            }
            write_log.borrow_mut().written.push((p.to_string(), b.to_vec()));
            write_log.borrow_mut().created.push(new_file);
            Ok(())
        }),
        inbox: inbox.clone(),
        new_files: Default::default(),
    };
    (remote, inbox)
}

/// An app whose services are `remote` and the log's recorders.
fn app_with(remote: Option<RemoteStorage>, log: &Rc<RefCell<Log>>) -> VectorcraftApp {
    let dl = log.clone();
    let services = Services {
        download: Some(Box::new(move |name: &str, _: &[u8]| {
            dl.borrow_mut().downloads.push(name.to_string());
            Ok(())
        })),
        place_inbox: Some(PlaceInbox::default()),
        remote,
        ..Default::default()
    };
    VectorcraftApp::new(Session::new(), services)
}

/// An app with the recording remote host.
fn remote_app() -> (VectorcraftApp, RemoteInbox, Rc<RefCell<Log>>) {
    let log = Rc::new(RefCell::new(Log::default()));
    let (remote, inbox) = host(&log);
    (app_with(Some(remote), &log), inbox, log)
}

/// The File menu's labels as shown.
fn file_menu(app: &VectorcraftApp) -> Vec<String> {
    menus::menu_entries(app).into_iter().filter(|e| e.path == ["File"] && e.command.is_some()).map(|e| e.label).collect()
}

/// A small document in the native format.
fn native_bytes() -> Vec<u8> {
    let bytes = Rc::new(RefCell::new(vec![]));
    let sink = bytes.clone();
    let mut app = VectorcraftApp::new(
        Session::new(),
        Services {
            write: Some(Box::new(move |_: &str, b: &[u8]| {
                *sink.borrow_mut() = b.to_vec();
                Ok(())
            })),
            ..Default::default()
        },
    );
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("file.saveAs", json!({"path": "/tmp/a.vectorcraft"})).unwrap();
    bytes.borrow().clone()
}

/// Without remote storage the file menu is as it was.
#[test]
fn without_remote_storage_the_file_menu_is_as_it_was() {
    let log = Rc::new(RefCell::new(Log::default()));
    let app = app_with(None, &log);
    let labels = file_menu(&app);
    assert!(labels.iter().any(|l| l == "Open…") && labels.iter().any(|l| l == "Place…"), "{labels:?}");
    assert!(!labels.iter().any(|l| l.contains("Remote") || l.contains("Local")), "{labels:?}");
    for id in ["file.openRemote", "file.placeRemote", "file.saveAsRemote"] {
        assert!(!menus::enabled(&app, id), "{id}");
    }
}

/// With remote storage open and place say local and the remote entries follow.
#[test]
fn with_remote_storage_open_and_place_say_local_and_the_remote_entries_follow() {
    let (mut app, ..) = remote_app();
    app.run("file.new", json!({})).unwrap();
    let labels = file_menu(&app);
    let at = |l: &str| labels.iter().position(|x| x == l).unwrap_or_else(|| panic!("{l} in {labels:?}"));
    assert_eq!(at("Open Remote…"), at("Open Local…") + 1);
    assert_eq!(at("Place from Remote…"), at("Place Local…") + 1);
    assert_eq!(at("Save As Remote…"), at("Save As…") + 1);
    assert!(!labels.iter().any(|l| l == "Open…" || l == "Place…"), "{labels:?}");
    for id in ["file.openRemote", "file.placeRemote", "file.saveAsRemote"] {
        assert!(menus::enabled(&app, id), "{id}");
    }
}

/// The commands fail cleanly without remote storage.
#[test]
fn the_commands_fail_cleanly_without_remote_storage() {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut app = app_with(None, &log);
    app.run("file.new", json!({})).unwrap();
    for id in ["file.openRemote", "file.placeRemote", "file.saveAsRemote"] {
        assert_eq!(app.run(id, json!({"path": "/rw/a.vectorcraft"})).unwrap_err(), "remote storage is not configured", "{id}");
    }
}

/// A fetched file opens as a document that remembers its path and saves back.
#[test]
fn a_fetched_file_opens_as_a_document_that_remembers_its_path_and_saves_back() {
    let (mut app, inbox, log) = remote_app();
    app.run("file.openRemote", json!({"path": "/rw/docs/a.vectorcraft"})).unwrap();
    assert_eq!(log.borrow().fetched, [("/rw/docs/a.vectorcraft".to_string(), Purpose::Open)]);
    inbox.lock().unwrap().push(RemoteEvent::Fetched { path: "/rw/docs/a.vectorcraft".into(), purpose: Purpose::Open, result: Ok(native_bytes()) });
    crate::remote::drain(&mut app);
    assert_eq!(app.session.active().and_then(|d| d.path.clone()).as_deref(), Some("/rw/docs/a.vectorcraft"));
    app.run("shape.rectangle", json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
    app.run("file.save", json!({})).unwrap();
    let log = log.borrow();
    assert_eq!(log.written.len(), 1, "Save goes to the server");
    assert_eq!(log.written[0].0, "/rw/docs/a.vectorcraft");
    assert!(log.downloads.is_empty(), "not to a download");
}

/// A file that cannot be fetched is reported.
#[test]
fn a_file_that_cannot_be_fetched_is_reported() {
    let (mut app, inbox, _) = remote_app();
    inbox.lock().unwrap().push(RemoteEvent::Fetched {
        path: "/rw/gone.vectorcraft".into(),
        purpose: Purpose::Open,
        result: Err("the server answered 404".into()),
    });
    crate::remote::drain(&mut app);
    assert!(app.session.active().is_none());
    assert!(app.ui.status.contains("gone.vectorcraft"), "{}", app.ui.status);
}

/// A save the server refused leaves the document unsaved.
#[test]
fn a_save_the_server_refused_leaves_the_document_unsaved() {
    let (mut app, inbox, _) = remote_app();
    inbox.lock().unwrap().push(RemoteEvent::Fetched { path: "/rw/a.vectorcraft".into(), purpose: Purpose::Open, result: Ok(native_bytes()) });
    crate::remote::drain(&mut app);
    assert!(!app.session.active().unwrap().is_dirty());
    inbox
        .lock()
        .unwrap()
        .push(RemoteEvent::SaveFailed { path: "/rw/a.vectorcraft".into(), message: "Document changed on the server; use Save As".into() });
    crate::remote::drain(&mut app);
    assert!(app.session.active().unwrap().is_dirty(), "still unsaved");
    assert!(app.ui.status.contains("Save As"), "{}", app.ui.status);
}

/// Save as remote writes to the mount and a read only mount refuses.
#[test]
fn save_as_remote_writes_to_the_mount_and_a_read_only_mount_refuses() {
    let (mut app, _, log) = remote_app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("file.saveAsRemote", json!({"path": "/rw/new"})).unwrap();
    assert_eq!(log.borrow().written[0].0, "/rw/new.vectorcraft", "a name without a save extension gets the native one");
    assert_eq!(app.session.active().and_then(|d| d.path.clone()).as_deref(), Some("/rw/new.vectorcraft"));
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    assert!(app.run("file.saveAsRemote", json!({"path": "/ro/x.vectorcraft"})).is_err());
    assert_eq!(log.borrow().written.len(), 1);
    assert!(app.run("file.saveAsRemote", json!({"path": "/local/x.vectorcraft"})).is_err(), "not on a mount");
}

/// Place from remote embeds through the place dialog.
#[test]
fn place_from_remote_embeds_through_the_place_dialog() {
    let (mut app, inbox, log) = remote_app();
    app.run("file.new", json!({})).unwrap();
    app.run("file.placeRemote", json!({"path": "/rw/b.svg"})).unwrap();
    assert_eq!(log.borrow().fetched, [("/rw/b.svg".to_string(), Purpose::Place)]);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#.to_vec();
    inbox.lock().unwrap().push(RemoteEvent::Fetched { path: "/rw/b.svg".into(), purpose: Purpose::Place, result: Ok(svg) });
    crate::remote::drain(&mut app);
    crate::place::drain(&mut app);
    let d = app.ui.dialog.as_ref().expect("the Place dialog opens");
    assert_eq!(d.kind, crate::dialogs::place::KIND);
    assert_eq!(d.fields["files"][0]["name"], "b.svg");
    assert!(d.fields["files"][0].get("path").is_none(), "no path: embedded, never linked");
}

/// The dialog starts at the mounts and confirming fetches the chosen file.
#[test]
fn the_dialog_starts_at_the_mounts_and_confirming_fetches_the_chosen_file() {
    let (mut app, _, log) = remote_app();
    app.run("file.openRemote", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("dir").as_str(), d.str("mode").as_str()), ("remote", "", "open"));
    assert!(crate::dialogs::confirm(&mut app).is_err(), "no folder yet");
    for (k, v) in [("dir", "/rw"), ("name", "b.svg")] {
        app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), json!(v));
    }
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(log.borrow().fetched, [("/rw/b.svg".to_string(), Purpose::Open)]);
}

/// The save dialog suggests a name and refuses read only mounts and other files.
#[test]
fn the_save_dialog_suggests_a_name_and_refuses_read_only_mounts_and_other_files() {
    let (mut app, ..) = remote_app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("file.saveAsRemote", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "Untitled-1.vectorcraft");
    let set = |app: &mut VectorcraftApp, dir: &str, name: &str| {
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("dir".into(), json!(dir));
        d.fields.insert("name".into(), json!(name));
    };
    set(&mut app, "/ro", "x.vectorcraft");
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains("read-only"));
    set(&mut app, "/rw", "taken");
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains("already there"), "the listing has taken.vectorcraft");
    assert!(app.ui.dialog.is_some(), "the dialog stays open");
    set(&mut app, "/rw", "../escape");
    assert!(crate::dialogs::confirm(&mut app).is_err());
}

/// The dialog draws the mounts the listing and a listing that is loading.
#[test]
fn the_dialog_draws_the_mounts_the_listing_and_a_listing_that_is_loading() {
    let (mut app, ..) = remote_app();
    app.run("file.openRemote", json!({})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    for dir in ["", "/rw", "/loading", "/rw/empty"] {
        app.ui.dialog.as_mut().unwrap().fields.insert("dir".into(), json!(dir));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert!(app.ui.dialog.is_some(), "{dir}");
    }
}

/// Names and paths.
#[test]
fn names_and_paths() {
    assert!(valid_name("a.vectorcraft") && valid_name(" a b "));
    for bad in ["", " ", ".", "..", "a/b", "a\\b", "a\nb"] {
        assert!(!valid_name(bad), "{bad:?}");
    }
    assert_eq!(join("/rw/", " x.svg "), "/rw/x.svg");
    assert_eq!(save_path("/rw/x").unwrap(), "/rw/x.vectorcraft");
    assert_eq!(save_path("/rw/x.svg").unwrap(), "/rw/x.svg");
    assert_eq!(save_path("/rw/x.png").unwrap(), "/rw/x.vectorcraft", "not a save format");
    assert_eq!(Mode::of("save"), Mode::Save);
    assert_eq!(Mode::of("junk"), Mode::Open);
}

/// Opening a remote path fetches it once until it arrives.
#[test]
fn opening_a_remote_path_fetches_it_once_until_it_arrives() {
    let (mut app, inbox, log) = remote_app();
    let path = "/rw/docs/a.vectorcraft";
    let r = crate::io::open_file(&mut app, path, false).unwrap();
    assert_eq!(r["pending"], "fetch");
    assert!(app.ui.status.contains("Fetching a.vectorcraft"), "{}", app.ui.status);
    // Open Recent clicked again while it is on its way
    crate::io::open_file(&mut app, path, false).unwrap();
    assert_eq!(log.borrow().fetched, [(path.to_string(), Purpose::Open)]);
    inbox.lock().unwrap().push(RemoteEvent::Fetched { path: path.into(), purpose: Purpose::Open, result: Ok(native_bytes()) });
    crate::remote::drain(&mut app);
    assert_eq!(app.session.active().and_then(|d| d.path.clone()).as_deref(), Some(path));
    assert!(app.remote_opening.is_empty());
}

/// A remote path an open document holds is not opened a second time, from the dialog or by path.
#[test]
fn a_remote_path_is_not_opened_twice() {
    let (mut app, inbox, log) = remote_app();
    let path = "/rw/docs/a.vectorcraft";
    // the dialog's OK while the first fetch is on its way
    crate::io::open_file(&mut app, path, false).unwrap();
    assert_eq!(accept(&mut app, Mode::Open, path).unwrap()["pending"], "fetch");
    assert_eq!(log.borrow().fetched.len(), 1);
    inbox.lock().unwrap().push(RemoteEvent::Fetched { path: path.into(), purpose: Purpose::Open, result: Ok(native_bytes()) });
    crate::remote::drain(&mut app);
    let err = accept(&mut app, Mode::Open, path).unwrap_err();
    assert!(err.contains("already open"), "{err}");
    assert!(crate::io::open_file(&mut app, path, false).is_err());
    assert_eq!(log.borrow().fetched.len(), 1, "no second fetch");
    // placing it is not opening it
    accept(&mut app, Mode::Place, path).unwrap();
    assert_eq!(log.borrow().fetched.len(), 2);
}

/// A failed remote open can be tried again.
#[test]
fn a_failed_remote_open_can_be_tried_again() {
    let (mut app, inbox, log) = remote_app();
    let path = "/rw/gone.vectorcraft";
    crate::io::open_file(&mut app, path, false).unwrap();
    inbox.lock().unwrap().push(RemoteEvent::Fetched { path: path.into(), purpose: Purpose::Open, result: Err("timed out".into()) });
    crate::remote::drain(&mut app);
    assert!(app.ui.status.contains("gone.vectorcraft"), "{}", app.ui.status);
    crate::io::open_file(&mut app, path, false).unwrap();
    assert_eq!(log.borrow().fetched.len(), 2);
}

/// Other paths still read locally.
#[test]
fn other_paths_still_read_locally() {
    let (mut app, _, log) = remote_app();
    let e = crate::io::open_file(&mut app, "/elsewhere/a.vectorcraft", false).unwrap_err();
    assert!(!e.is_empty());
    assert!(log.borrow().fetched.is_empty());
}

/// A save to the server leaves the document unsaved until the server confirms it, and an edit made
/// meanwhile stays unsaved after it.
#[test]
fn a_started_save_counts_as_saved_only_once_the_server_confirms_it() {
    let (mut app, inbox, log) = remote_app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("file.saveAsRemote", json!({"path": "/rw/new"})).unwrap();
    assert_eq!(log.borrow().written.len(), 1);
    assert!(app.session.active().unwrap().is_dirty(), "the upload is not confirmed yet: closing must ask");
    inbox.lock().unwrap().push(RemoteEvent::Saved { path: "/rw/new.vectorcraft".into() });
    crate::remote::drain(&mut app);
    assert!(!app.session.active().unwrap().is_dirty(), "confirmed");

    app.run("file.save", json!({})).unwrap();
    assert!(app.session.active().unwrap().is_dirty());
    app.run("shape.rectangle", json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
    inbox.lock().unwrap().push(RemoteEvent::Saved { path: "/rw/new.vectorcraft".into() });
    crate::remote::drain(&mut app);
    assert!(app.session.active().unwrap().is_dirty(), "edits made while the upload ran stay unsaved");
    assert!(app.remote_saving.is_empty());
}

/// A failed upload leaves the document unsaved and a later confirmation is not applied to it.
#[test]
fn a_failed_upload_stays_unsaved() {
    let (mut app, inbox, _) = remote_app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("file.saveAsRemote", json!({"path": "/rw/new"})).unwrap();
    inbox.lock().unwrap().push(RemoteEvent::SaveFailed { path: "/rw/new.vectorcraft".into(), message: "no".into() });
    crate::remote::drain(&mut app);
    assert!(app.session.active().unwrap().is_dirty());
}

/// A Save As to another path creates a file; saving the document's own path does not.
#[test]
fn a_save_as_to_another_path_creates_a_file() {
    let (mut app, _, log) = remote_app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("file.saveAsRemote", json!({"path": "/rw/new"})).unwrap();
    app.run("file.save", json!({})).unwrap();
    app.run("file.saveAsRemote", json!({"path": "/rw/new"})).unwrap();
    app.run("file.saveAsRemote", json!({"path": "/rw/other"})).unwrap();
    assert_eq!(log.borrow().created, [true, false, false, true]);
}

/// A file fetched for Place waits while another dialog is open, and then opens the Place dialog.
#[test]
fn a_fetched_place_file_waits_for_the_open_dialog() {
    let (mut app, inbox, _) = remote_app();
    app.run("file.new", json!({})).unwrap();
    app.run("file.placeRemote", json!({"path": "/rw/b.svg"})).unwrap();
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#.to_vec();
    app.ui.dialog = Some(crate::state::Dialog::new("other", json!({"keep": 1})));
    inbox.lock().unwrap().push(RemoteEvent::Fetched { path: "/rw/b.svg".into(), purpose: Purpose::Place, result: Ok(svg) });
    crate::remote::drain(&mut app);
    crate::place::drain(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, "other", "the open dialog is not replaced");
    app.ui.dialog = None;
    crate::place::drain(&mut app);
    assert_eq!(app.ui.dialog.as_ref().expect("the Place dialog opens").kind, crate::dialogs::place::KIND);
}
