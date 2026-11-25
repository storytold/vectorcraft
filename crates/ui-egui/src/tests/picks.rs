//! File picks through the framework's file service: what asked runs again with the answer, on the
//! document it asked for; while a pick's sheet is open nothing closes or quits.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

// PLAN: assumed ui-egui depends on craft-host, craft-ui-panels and craft-ui (hub-w1/hub-w2 add them).
use craft::host::{Host, PickKind, PickRequest, Picked, ScriptedFiles};
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, HeldOk, OpenDialog, PromptKind, StandardPrompt};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::requests::{PathKind, PathRequest};
use egui::{Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::file_access;

use crate::dialogs::confirm::REPLACE_FILES;
use crate::{Services, VectorcraftApp, file_asks};

const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect width="10" height="10"/></svg>"#;

/// The scripted file service of one test: picks stay open until the test answers them.
pub(crate) struct Picker {
    files: Arc<ScriptedFiles>,
    written: Rc<RefCell<Vec<String>>>,
}

impl Picker {
    /// The requests of every pick asked so far.
    pub(crate) fn requests(&self) -> Vec<PickRequest> {
        self.files.requests()
    }
    /// The newest open pick answers `paths`.
    pub(crate) fn answer(&self, paths: &[&str]) {
        self.files.answer(Picked::Paths(paths.iter().map(PathBuf::from).collect()));
    }
    /// The newest open Save pick answers `path`, with the index of the file type picked in the panel.
    pub(crate) fn answer_saved(&self, path: &str, file_type: Option<usize>) {
        self.files.answer(Picked::Saved { path: PathBuf::from(path), file_type });
    }
    /// The newest open pick is cancelled.
    pub(crate) fn cancel(&self) {
        self.files.answer(Picked::Cancelled);
    }
    /// The bytes a save through the service (`Files::save`) wrote to `path`.
    pub(crate) fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.files.file(std::path::Path::new(path))
    }
    /// The paths files were written to.
    pub(crate) fn written(&self) -> Vec<String> {
        self.written.borrow().clone()
    }
}

/// Give `app` a scripted file service, a reader of [`SVG`] at any path (unless it has a reader) and
/// a writer that lists the paths written.
pub(crate) fn scripted(app: &mut VectorcraftApp) -> Picker {
    scripted_files(app, ScriptedFiles::held())
}

/// [`scripted`] with picks that report whether they are sheets on the window (macOS).
pub(crate) fn scripted_sheets(app: &mut VectorcraftApp, on: bool) -> Picker {
    scripted_files(app, ScriptedFiles::held().sheets(on))
}

/// [`scripted`] with `files` as the app's file service: the app's host carries it, and so does the
/// window's context, as a frame installs the app's host there.
fn scripted_files(app: &mut VectorcraftApp, files: ScriptedFiles) -> Picker {
    let files = Arc::new(files);
    install_files(app, files.clone());
    let written = Rc::new(RefCell::new(Vec::new()));
    let w = written.clone();
    app.services.read.get_or_insert_with(|| Box::new(|_: &str| Ok(SVG.to_vec())));
    // A writer the test gave goes on getting the bytes.
    let mut given = app.services.write.take();
    app.services.write = Some(Box::new(move |path: &str, bytes: &[u8]| {
        w.borrow_mut().push(path.to_string());
        given.as_mut().map_or(Ok(()), |write| write(path, bytes))
    }));
    Picker { files, written }
}

/// Makes `files` the file service of `app`'s host and of its window's context, as a frame installs the app's host
/// there; an app without a window gets one.
pub(crate) fn install_files(app: &mut VectorcraftApp, files: Arc<ScriptedFiles>) {
    let ctx = app.ui_ctx.get_or_insert_with(crate::testing::ui_context).clone();
    app.host = Arc::new(app.host.with_files(files));
    Host::clone(&app.host).install(&ctx);
}

/// Runs `command` with no path: it asks for a file and replies that it waits for the pick (fl.A3).
pub(crate) fn asks(app: &mut VectorcraftApp, command: &str) {
    assert_eq!(app.run(command, json!({})), Ok(json!({"pending": "fileDialog"})), "{command} waits for its pick");
}

/// Run what the frame runs for the picks: their continuations, then the saves.
pub(crate) fn poll(app: &mut VectorcraftApp, ctx: &egui::Context) {
    file_asks::poll(app);
    crate::io::save::pump(app, ctx);
}

/// The dialogs the window has open, in opening order (the top one last).
pub(crate) fn open_dialogs(app: &VectorcraftApp) -> Vec<OpenDialog> {
    app.dialogs().map(|d| d.with(|h| h.open_dialogs())).unwrap_or_default()
}

/// Sets `fields` of dialog `id` as a person editing them would, presses OK when `ok`, and runs the
/// app's pump over the answer. A disabled OK does nothing, so the caller checks what the press did.
/// A dialog that holds its OK files the answer for the app; once the pump accepted it the dialog
/// closes, as its next pass would close it.
pub(crate) fn answer_dialog(app: &mut VectorcraftApp, id: DialogId, fields: &[(&str, Value)], ok: bool) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    let host = app.dialogs().expect("the window's dialog host");
    host.with(|h| {
        for (field, value) in fields {
            h.set_json_field(&ctx, id, field, value.clone()).unwrap_or_else(|e| panic!("{field}: {e}"));
        }
        if ok {
            h.confirm_by_id(&ctx, id);
        }
    });
    crate::dialogs::host::pump(app, &ctx);
    if host.with(|h| h.held_state(id) == Some(HeldOk::Accepted) && h.confirm_by_id(&ctx, id)) {
        crate::dialogs::host::pump(app, &ctx);
    }
}

/// The open question whether to replace a file: [`file_asks::ask_replace`]'s, with the file name it
/// names, or the replace-files question of a save or export, which names a list.
fn replace_question(app: &VectorcraftApp) -> Option<(DialogId, Option<String>)> {
    app.dialogs()?.with(|h| {
        h.open_dialogs().into_iter().rev().find_map(|d| {
            if d.key.as_deref() == Some(REPLACE_FILES) {
                return Some((d.id, None));
            }
            match h.get_by_id::<StandardPrompt>(d.id)?.kind() {
                PromptKind::ReplaceFile { name, .. } => Some((d.id, Some(name.clone()))),
                _ => None,
            }
        })
    })
}

/// The file name the open replace question ([`file_asks::ask_replace`]) names.
pub(crate) fn replace_asked(app: &VectorcraftApp) -> Option<String> {
    replace_question(app).and_then(|(_, name)| name)
}

/// Proceeds with (`proceed`) or cancels the open replace question, then runs the app's pump over
/// the answer, where what it allows is written.
pub(crate) fn answer_replace(app: &mut VectorcraftApp, proceed: bool) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    let (id, _) = replace_question(app).expect("a replace question is open");
    let host = app.dialogs().expect("the window's dialog host");
    let answered = host.with(|h| if proceed { h.confirm_by_id(&ctx, id) } else { h.cancel_by_id(&ctx, id) });
    assert!(answered, "the replace question took the answer");
    crate::dialogs::host::pump(app, &ctx);
}

/// Opens a question on the window and draws one frame, so the app counts a dialog open → its id.
pub(crate) fn show_question(app: &mut VectorcraftApp) -> DialogId {
    let ctx = app.ui_ctx.get_or_insert_with(crate::testing::ui_context).clone();
    crate::dialogs::confirm::ask(app, "Go on?", "", "edit.undo", json!({}));
    draw_dialogs(app, &ctx);
    open_dialogs(app).last().map(|d| d.id).expect("the question is open")
}

/// Closes dialog `id` and draws one frame, so the app no longer counts it open.
pub(crate) fn close_shown(app: &mut VectorcraftApp, id: DialogId) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    app.dialogs().expect("the window's dialog host").with(|h| h.cancel_by_id(&ctx, id));
    draw_dialogs(app, &ctx);
}

/// One frame that draws the open dialogs and runs the app's pump over their answers.
pub(crate) fn draw_dialogs(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs(app, ui));
    out.textures_delta.clear();
}

/// Presses Cancel on dialog `id` and runs the app's pump over the answer.
pub(crate) fn cancel_dialog(app: &mut VectorcraftApp, id: DialogId) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    let host = app.dialogs().expect("the window's dialog host");
    assert!(host.with(|h| h.cancel_by_id(&ctx, id)), "Cancel was refused");
    crate::dialogs::host::pump(app, &ctx);
}

fn app() -> VectorcraftApp {
    VectorcraftApp::new(Session::new(), Services::default())
}

/// The picks' answers, then the rest of a frame's pumps (Save Changes goes on once its save is written).
fn settle(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    poll(app, &ctx);
    crate::dialogs::host::pump(app, &ctx);
}

/// The id of the open dialog with `key`.
// PLAN: assumed the hosted dialogs' keys equal the old dialog kinds (`svgOptions`, `missingFonts`).
fn find(app: &VectorcraftApp, key: &str) -> Option<DialogId> {
    open_dialogs(app).iter().find(|d| d.key.as_deref() == Some(key)).map(|d| d.id)
}

fn top(app: &VectorcraftApp) -> DialogId {
    open_dialogs(app).last().map(|d| d.id).expect("a dialog is open")
}

fn fields_of(app: &VectorcraftApp, id: DialogId) -> Option<Value> {
    open_dialogs(app).into_iter().find(|d| d.id == id).and_then(|d| d.fields)
}

/// One whole frame of the app (its `tick` polls the picks and opens the URL a save asked for) → the URLs the frame
/// asked egui to open.
fn opened_urls(app: &mut VectorcraftApp) -> Vec<String> {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))), ..Default::default() };
    crate::testing::app_frame(app, &ctx, raw)
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect()
}

/// A hosted dialog with one folder row, open on the window's host.
fn browse_dialog(app: &mut VectorcraftApp) -> DialogId {
    let ctx = app.ui_ctx.clone().expect("a window context");
    let params = vec![Param::new("folder", "Folder", ParamKind::text("")).browse(PathKind::Folder)];
    let dialog = HostedParamDialog::new(&ctx, "browseTest", DialogFields::from_pairs([("folder", "")]), move |fields| {
        ParamDialog::new("browseTest", "Browse test", params.clone()).values(Bind::bound(fields))
    });
    app.dialogs().expect("the window's dialog host").with(|h| h.open(dialog))
}

/// Asks for a folder on behalf of dialog `id`'s `folder` row.
fn browse_folder(app: &mut VectorcraftApp, id: DialogId) {
    file_asks::browse(app, id, PathRequest::new("folder", PathKind::Folder), PickRequest::folder("export"), file_asks::answer_path);
}

// ------------------------------------------------------------------------- driven like a person

/// Room for a dialog, a panel or a menu.
const SCREEN: (f32, f32) = (1600.0, 6000.0);

/// One frame of `draw` with `events` → its texts and where they were drawn.
fn frame(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    events: Vec<egui::Event>,
    draw: &impl Fn(&mut VectorcraftApp, &mut egui::Ui),
) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(SCREEN.0, SCREEN.1))), events, ..Default::default() };
    // With the app's frame input, as the window's frames have it: its host carries the file service a dialog picks with.
    let out = crate::testing::part_frame(app, ctx, raw, |app, ui| draw(app, ui));
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

/// Where text `label` was drawn (menu items pad theirs).
fn at(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts
        .iter()
        .find(|(t, _)| t.trim() == label)
        .unwrap_or_else(|| panic!("no `{label}` in {:?}", texts.iter().map(|t| &t.0).collect::<Vec<_>>()))
        .1
        .center()
}

/// Draw `draw` until it settles, then click its text `label`.
fn click(app: &mut VectorcraftApp, label: &str, draw: impl Fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    crate::testing::install(&ctx);
    frame(&ctx, app, vec![], &draw);
    frame(&ctx, app, vec![], &draw);
    let pos = at(&frame(&ctx, app, vec![], &draw), label);
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // Hover, press and release in frames of their own, as a person's click comes.
    for event in [egui::Event::PointerMoved(pos), button(true), button(false)] {
        frame(&ctx, app, vec![event], &draw);
    }
}

/// The open dialogs, drawn as the shell draws them, then the app's pump over their answers.
pub(crate) fn dialogs(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    if let Some(host) = app.dialogs() {
        host.with(|h| h.run(ui));
    }
    crate::dialogs::host::pump(app, ui.ctx());
}

// ------------------------------------------------------------------------------------- the tests

/// A command asks for a file and goes on with the answer; cancelled, nothing happens.
#[test]
fn a_command_asks_for_a_file_and_goes_on_with_the_answer() {
    type Answer = fn(&Picker);
    type Then = fn(&mut VectorcraftApp, &Picker);
    // PLAN: `ui.findLinksInFolder` needs a linked image and a folder search; the find-links tests cover it.
    let rows: [(&str, PickKind, Answer, Then); 3] = [
        (
            "file.open",
            PickKind::Open,
            |p| p.answer(&["/art/a.svg"]),
            |app, _| {
                assert_eq!(app.session.documents().len(), 2);
                assert_eq!(app.session.active().and_then(|d| d.path.clone()).as_deref(), Some("/art/a.svg"));
            },
        ),
        (
            "file.saveAs",
            PickKind::Save,
            |p| p.answer_saved("/art/poster.svg", None),
            |app, p| {
                // SVG Options asks next, and its OK writes the file there.
                let options = find(app, "svgOptions").expect("SVG Options");
                answer_dialog(app, options, &[], true);
                assert_eq!(p.written(), ["/art/poster.svg"]);
            },
        ),
        (
            "file.place",
            PickKind::OpenMany,
            |p| p.answer(&["/art/p.svg"]),
            |app, _| {
                assert!(!open_dialogs(app).is_empty(), "the place dialog opens: {}", app.ui.status);
            },
        ),
    ];
    for (command, kind, answer, then) in rows {
        let mut a = app();
        let picker = scripted(&mut a);
        a.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        a.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        asks(&mut a, command);
        assert_eq!(picker.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [kind], "{command}");
        settle(&mut a);
        assert!(a.file_dialog_open(), "{command}");
        answer(&picker);
        settle(&mut a);
        assert!(!a.file_dialog_open(), "{command}");
        then(&mut a, &picker);
    }
    for (command, ..) in rows {
        let mut a = app();
        let picker = scripted(&mut a);
        a.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        asks(&mut a, command);
        picker.cancel();
        settle(&mut a);
        assert!(!a.file_dialog_open(), "{command}");
        assert!(a.session.documents().len() == 1 && open_dialogs(&a).is_empty() && picker.written().is_empty(), "{command}: {}", a.ui.status);
    }
}

/// Save as PDF with View PDF after Saving, asked for by automation without a path: the save panel
/// it opened answers as automation's, so the PDF is written but not opened in the viewer, even
/// when a person picked the file. The person's own Save as PDF opens it.
#[test]
fn a_pdf_automation_asked_for_is_not_opened_when_its_save_panel_answers() {
    let mut app = app();
    let picker = scripted_sheets(&mut app, true);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    let view = json!({"viewAfterSaving": true});

    app.ui.answering_control = true;
    let _ = app.run("file.export.pdf", view.clone());
    app.ui.answering_control = false;
    assert_eq!(picker.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::Save]);
    picker.answer_saved("/art/automated.pdf", None);
    let urls = opened_urls(&mut app);
    assert_eq!(picker.written(), ["/art/automated.pdf"]);
    assert!(urls.is_empty(), "{urls:?}");

    let _ = app.run("file.export.pdf", view);
    picker.answer_saved("/art/mine.pdf", None);
    let urls = opened_urls(&mut app);
    assert_eq!(picker.written(), ["/art/automated.pdf", "/art/mine.pdf"]);
    assert!(matches!(urls.as_slice(), [url] if url.ends_with("/art/mine.pdf")), "{urls:?}");
}

/// A replace question automation asked while confined to a folder: what Replace allows runs as
/// automation's, with that folder still the only place it may write, as the answer of a file dialog does.
#[test]
fn a_replace_question_goes_on_under_the_access_it_was_asked_with() {
    let mut app = app();
    app.ui_ctx = Some(crate::testing::ui_context());
    let (inside, outside) = (vectorcraft_testkit::TempDir::new("replace-scope-in"), vectorcraft_testkit::TempDir::new("replace-scope-out"));
    let roots = file_access::AutomationRoots::new(&[inside.as_path()], &[inside.as_path()]).unwrap();
    let path = inside.join("Art.svg").to_string_lossy().into_owned();
    let elsewhere = outside.join("Art.svg").to_string_lossy().into_owned();
    let seen = Rc::new(RefCell::new(None));
    let s = seen.clone();
    file_access::confine(roots.as_ref(), || {
        file_asks::as_automated(&mut app, true, |app| {
            file_asks::ask_replace(app, &path, "Art", move |app| {
                *s.borrow_mut() = Some((file_access::check_write(&elsewhere).is_ok(), app.automation_asked()));
                Ok(Value::Null)
            })
        })
    });
    assert_eq!(replace_asked(&app).as_deref(), Some("Art.svg"));
    answer_replace(&mut app, true);
    assert_eq!(*seen.borrow(), Some((false, true)), "(may write outside the folder, runs as automation's)");
}

/// The question whether a save or export may replace files, asked by automation confined to a
/// folder: OK runs the command again with that folder still the only place it may read.
#[test]
fn a_replace_files_question_runs_its_command_under_the_access_it_was_asked_with() {
    let mut app = app();
    app.ui_ctx = Some(crate::testing::ui_context());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    let (inside, outside) =
        (vectorcraft_testkit::TempDir::new("replace-files-scope-in"), vectorcraft_testkit::TempDir::new("replace-files-scope-out"));
    let roots = file_access::AutomationRoots::new(&[inside.as_path()], &[inside.as_path()]).unwrap();
    let elsewhere = outside.join("Other.svg");
    std::fs::write(&elsewhere, SVG).unwrap();
    let elsewhere = elsewhere.to_string_lossy().into_owned();
    let doc = app.session.active().unwrap().uid;
    let there = [inside.join("Art.svg").to_string_lossy().into_owned()];
    file_access::confine(roots.as_ref(), || {
        file_asks::as_automated(&mut app, true, |app| {
            crate::dialogs::confirm::ask_replace_files(app, doc, &there, &[], "file.open", json!({ "path": elsewhere }));
        })
    });
    assert_eq!(crate::dialogs::confirm::open_replace_files(&app), Some(there.to_vec()));
    answer_replace(&mut app, true);
    assert!(app.ui.status.contains("outside the read root"), "the file outside the folder is refused: {}", app.ui.status);
    assert_eq!(app.session.documents().len(), 1, "nothing else opened");
}

/// A confirmation automation asked while confined to a folder: OK runs its command as automation's,
/// with that folder still the only place it may read.
#[test]
fn a_confirmation_runs_its_command_under_the_access_it_was_asked_with() {
    let mut app = app();
    app.ui_ctx = Some(crate::testing::ui_context());
    let (inside, outside) = (vectorcraft_testkit::TempDir::new("confirm-scope-in"), vectorcraft_testkit::TempDir::new("confirm-scope-out"));
    let roots = file_access::AutomationRoots::new(&[inside.as_path()], &[inside.as_path()]).unwrap();
    let elsewhere = outside.join("Other.svg");
    std::fs::write(&elsewhere, SVG).unwrap();
    let elsewhere = elsewhere.to_string_lossy().into_owned();
    file_access::confine(roots.as_ref(), || {
        file_asks::as_automated(&mut app, true, |app| {
            crate::dialogs::confirm::ask(app, "Open it?", "", "file.open", json!({ "path": elsewhere }));
        })
    });
    let id = open_dialogs(&app).last().map(|d| d.id).expect("the question is open");
    answer_dialog(&mut app, id, &[], true);
    assert!(app.ui.status.contains("outside the read root"), "the file outside the folder is refused: {}", app.ui.status);
    assert!(app.session.documents().is_empty(), "nothing opened");
}

/// A Browse row's answer fills the open dialog's field; an answer after the dialog closed does nothing.
#[test]
fn a_dialogs_folder_button_fills_its_field_when_the_picker_answers() {
    let mut app = app();
    let picker = scripted(&mut app);
    let id = browse_dialog(&mut app);
    browse_folder(&mut app, id);
    assert_eq!(picker.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::Folder]);
    picker.answer(&["/out"]);
    settle(&mut app);
    // The dialog writes the answer into its field in its next pass.
    let ctx = app.ui_ctx.clone().unwrap();
    draw_dialogs(&mut app, &ctx);
    assert_eq!(fields_of(&app, id).map(|f| f["folder"].clone()), Some(json!("/out")));

    browse_folder(&mut app, id);
    app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, id));
    picker.answer(&["/late"]);
    settle(&mut app);
    assert!(open_dialogs(&app).is_empty(), "the answer reopened nothing");
}

/// Export Text writes the working set as text where its save panel answers.
#[test]
fn keyboard_shortcuts_export_text_writes_the_working_set_as_text() {
    let mut app = app();
    let picker = scripted(&mut app);
    let texts: Rc<RefCell<Vec<(String, String)>>> = Default::default();
    let t = texts.clone();
    app.services.write = Some(Box::new(move |path: &str, b: &[u8]| {
        t.borrow_mut().push((path.to_string(), String::from_utf8_lossy(b).into_owned()));
        Ok(())
    }));
    // PLAN: the working set is the active set's defaults here; shortcuts-w6's `Overrides` setter is not named.
    app_frame(&app.ui_ctx.clone().unwrap(), &mut app, false);
    let _ = app.run("shortcuts.exportText", json!({}));
    let requests = picker.requests();
    let [request] = requests.as_slice() else { panic!("{requests:?}") };
    assert_eq!(request.kind, PickKind::Save);
    assert_eq!(request.filters.iter().map(|f| f.extensions.join(",")).collect::<Vec<_>>(), ["txt", "pdf", "csv"]);
    picker.answer_saved("/keys/list.txt", Some(0));
    settle(&mut app);
    let texts = texts.borrow();
    let [(path, text)] = texts.as_slice() else { panic!("{texts:?}") };
    assert_eq!(path, "/keys/list.txt");
    assert!(text.starts_with("Keyboard Shortcuts · VectorCraft Defaults\n"), "{text}");
    assert!(text.contains(&format!("\t{}\n", crate::keymap::chord_text("Ctrl+S"))), "{text}");
}

/// The file type picked in Export Text's save panel picks the file: a name typed with `.pdf` or
/// `.csv` is that file, and a name typed without an extension gets the picked type's extension.
#[test]
fn keyboard_shortcuts_export_text_writes_the_format_picked_in_its_save_dialog() {
    let csv = &b"\xef\xbb\xbfGroup,Command,Shortcut\r\n"[..];
    for (answer, file_type, path, start) in [
        ("/keys/list.pdf", None, "/keys/list.pdf", &b"%PDF-"[..]),
        ("/keys/list.csv", None, "/keys/list.csv", csv),
        ("/keys/table", Some(2), "/keys/table.csv", csv),
        ("/keys/sheet", Some(1), "/keys/sheet.pdf", &b"%PDF-"[..]),
        ("/keys/plain", Some(0), "/keys/plain.txt", "Keyboard Shortcuts · ".as_bytes()),
    ] {
        let mut app = app();
        let picker = scripted(&mut app);
        // Each file written: its path and bytes.
        let files: Rc<RefCell<Vec<_>>> = Default::default();
        let f = files.clone();
        app.services.write = Some(Box::new(move |path: &str, b: &[u8]| {
            f.borrow_mut().push((path.to_string(), b.to_vec()));
            Ok(())
        }));
        app_frame(&app.ui_ctx.clone().unwrap(), &mut app, false);
        let _ = app.run("shortcuts.exportText", json!({}));
        picker.answer_saved(answer, file_type);
        settle(&mut app);
        let files = files.borrow();
        let [(written, bytes)] = files.as_slice() else { panic!("{answer}: {files:?}") };
        assert_eq!(written, path);
        assert!(bytes.starts_with(start), "{path}: {:?}", String::from_utf8_lossy(&bytes[..bytes.len().min(40)]));
    }
}

/// Keyboard Shortcuts' Import opens a shortcut set another app saved as text (`.kys`) in a review
/// over the dialog when its file dialog answers; Cancel shows Keyboard Shortcuts again.
#[test]
fn keyboard_shortcuts_import_opens_a_text_set_over_the_dialog() {
    let mut app = app();
    let picker = scripted(&mut app);
    let dir = std::env::temp_dir().join(format!("vc-picks-kys-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let set = dir.join("Mine.kys");
    std::fs::write(&set, "/Menus {\n /Acme\\ Undo { /Modifiers 11 /Key 902 }\n}\n").unwrap();
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let ctx = app.ui_ctx.clone().unwrap();
    app_frame(&ctx, &mut app, false);
    app.run("edit.keyboardShortcuts", json!({})).unwrap();
    app_frame(&ctx, &mut app, false);
    // Keyboard Shortcuts is the framework's dialog, not one of the dialog host's: the review is its own page over it.
    let review = |app: &VectorcraftApp| app.shortcuts_dialog.as_ref().and_then(|d| d.state.snapshot(d.keymap.as_ref()?)?.review.map(|r| r.name));
    assert!(app.shortcuts_dialog.as_ref().is_some_and(|d| d.state.is_open()), "Keyboard Shortcuts opens");
    click(&mut app, "Import…", |app, ui| app.ui(ui));
    assert_eq!(picker.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::Open], "Import… asks for the file");
    picker.answer(&[&set.to_string_lossy()]);
    settle(&mut app);
    app_frame(&ctx, &mut app, false);
    assert_eq!(review(&app).as_deref(), Some("Mine"), "{}", app.ui.status);
    // Escape, the review's Cancel Import.
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))), events: vec![escape], ..Default::default() };
    crate::testing::part_frame(&mut app, &ctx, raw, |app, ui| app.ui(ui));
    app_frame(&ctx, &mut app, false);
    assert_eq!(review(&app), None, "the review closes");
    assert!(app.shortcuts_dialog.as_ref().is_some_and(|d| d.state.is_open()), "Keyboard Shortcuts shows again");
    let _ = std::fs::remove_dir_all(dir);
}

/// `shortcuts.export` and `shortcuts.import` given a `path` that is not a string open no file
/// dialog, read and write nothing, and give an error naming `path`; a null `path` still asks.
#[test]
fn keyboard_shortcuts_commands_reject_a_path_that_is_not_a_string() {
    let mut app = app();
    let picker = scripted(&mut app);
    let read: Rc<RefCell<Vec<String>>> = Default::default();
    let r = read.clone();
    app.services.read = Some(Box::new(move |path: &str| {
        r.borrow_mut().push(path.to_string());
        Ok(SVG.to_vec())
    }));
    for path in [json!(5), json!(true), json!({"file": "/keys/a.json"}), json!(["/keys/a.json"])] {
        for id in ["shortcuts.export", "shortcuts.import"] {
            assert_eq!(app.run(id, json!({ "path": path })), Err("`path` must be a string".to_string()), "{id} {path}");
            assert!(picker.requests().is_empty(), "{id} {path}: {:?}", picker.requests());
        }
    }
    assert!(picker.written().is_empty() && read.borrow().is_empty());
    assert_eq!(app.run("shortcuts.export", json!({"path": null})), Ok(json!({"pending": "fileDialog"})));
    assert_eq!(picker.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::Save]);
}

/// Save As answers on the document that asked for it: the one made active meanwhile isn't saved,
/// and the one that asked is active again.
#[test]
fn save_as_saves_the_document_that_asked() {
    let mut app = app();
    let picker = scripted(&mut app);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.run("document.activate", json!({"index": 0})).unwrap();
    asks(&mut app, "file.saveAs");
    app.run("document.activate", json!({"index": 1})).unwrap();
    picker.answer_saved("/art/a.svg", None);
    settle(&mut app);
    assert_eq!(app.session.active().map(|d| d.uid), Some(uids[0]));
    let options = find(&app, "svgOptions").expect("SVG Options");
    answer_dialog(&mut app, options, &[], true);
    assert_eq!(picker.written(), ["/art/a.svg"]);
    let paths: Vec<Option<String>> = app.session.documents().iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths, [Some("/art/a.svg".to_string()), None]);
    assert_eq!(app.session.active().map(|d| d.uid), Some(uids[0]));
}

/// An answer for a document closed meanwhile (here through the engine, as an agent can) is dropped:
/// nothing is written, no other document is saved, and the status says why.
#[test]
fn an_answer_for_a_closed_document_is_dropped() {
    let mut app = app();
    let picker = scripted(&mut app);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    asks(&mut app, "file.saveAs");
    app.session.execute("file.close", &json!({"index": 0})).unwrap();
    app.sync_views();
    picker.answer_saved("/art/a.svg", None);
    settle(&mut app);
    assert_eq!(app.ui.status, file_asks::closed());
    assert!(picker.written().is_empty());
    assert!(find(&app, "svgOptions").is_none(), "no SVG Options");
    assert_eq!(app.session.documents().iter().map(|d| d.path.clone()).collect::<Vec<_>>(), [None]);
    assert!(!app.file_dialog_open());
}

/// While a pick's sheet is on the window (macOS) nothing quits or closes: Quit, Close and Close
/// All say a file dialog is open and close nothing; once it answers or is cancelled they work
/// again.
#[test]
fn nothing_closes_or_quits_while_a_sheet_is_open() {
    let mut app = app();
    let picker = scripted_sheets(&mut app, true);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    assert!(!app.file_dialog_open() && !app.file_sheet_open());
    asks(&mut app, "file.open");
    assert!(app.file_dialog_open() && app.file_sheet_open());
    let refused = |app: &mut VectorcraftApp| {
        for (id, p) in [("app.quit", json!({})), ("file.close", json!({})), ("file.close", json!({"index": 0})), ("file.closeAll", json!({}))] {
            assert_eq!(app.run(id, p.clone()), Err(file_asks::busy().to_string()), "{id} {p}");
        }
        assert_eq!(app.session.documents().len(), 2);
        assert_ne!(app.ui.status, "quit");
        assert!(open_dialogs(app).is_empty(), "no Save Changes either");
    };
    refused(&mut app);
    // Answered: the file opens, and documents close again.
    picker.answer(&["/art/c.svg"]);
    settle(&mut app);
    assert!(!app.file_dialog_open());
    assert_eq!(app.session.documents().len(), 3);
    app.run("file.close", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    // Cancelled: the same.
    asks(&mut app, "file.saveAs");
    refused(&mut app);
    picker.cancel();
    settle(&mut app);
    assert!(!app.file_dialog_open());
    app.run("file.closeAll", json!({})).unwrap();
    assert!(app.session.documents().is_empty());
    asks(&mut app, "file.open");
    assert!(app.run("app.quit", json!({})).is_err());
    picker.cancel();
    settle(&mut app);
    app.run("app.quit", json!({})).unwrap();
    assert_eq!(app.ui.status, "quit");
}

/// A view Window › New Window opened does not close while a pick's sheet is on the window either.
#[test]
fn no_view_closes_while_a_sheet_is_open() {
    let mut app = app();
    let picker = scripted_sheets(&mut app, true);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("window.newWindow", json!({})).unwrap();
    asks(&mut app, "file.open");
    assert_eq!(app.run("file.close", json!({})), Err(file_asks::busy().to_string()));
    assert_eq!(crate::doc_windows::windows(&app), [(0, 1), (0, 2)]);
    picker.cancel();
    settle(&mut app);
    app.run("file.close", json!({})).unwrap();
    assert_eq!(crate::doc_windows::windows(&app), [(0, 1)]);
}

/// On Linux the picker is a window of its own, not a sheet: Close and Quit go on while it is open,
/// and its answer for the document closed meanwhile is dropped, nothing written.
#[test]
fn on_linux_documents_close_while_a_dialog_is_open() {
    let mut app = app();
    let picker = scripted_sheets(&mut app, false);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    asks(&mut app, "file.saveAs");
    assert!(app.file_dialog_open() && !app.file_sheet_open());
    app.run("file.close", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    picker.answer_saved("/art/a.svg", None);
    settle(&mut app);
    assert_eq!(app.ui.status, file_asks::closed());
    assert!(picker.written().is_empty());
    assert!(find(&app, "svgOptions").is_none());
    // Quit goes on too.
    asks(&mut app, "file.open");
    assert!(app.file_dialog_open());
    app.run("app.quit", json!({})).unwrap();
    assert_eq!(app.ui.status, "quit");
}

/// File › Open acts on no document: its answer opens the file even when the document active at
/// the time closed meanwhile.
#[test]
fn an_open_answer_goes_on_after_the_active_document_closes() {
    let mut app = app();
    let picker = scripted_sheets(&mut app, false);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    asks(&mut app, "file.open");
    app.run("file.close", json!({})).unwrap();
    assert!(app.session.documents().is_empty());
    picker.answer(&["/art/b.svg"]);
    settle(&mut app);
    assert_eq!(app.session.documents().iter().map(|d| d.path.clone()).collect::<Vec<_>>(), [Some("/art/b.svg".to_string())]);
    assert_ne!(app.ui.status, file_asks::closed());
}

/// Save Changes' Save, answered by its Save panel, saves and closes the document it asked about,
/// even when an earlier document closed meanwhile, then asks about the next one.
#[test]
fn save_changes_answers_for_its_own_document() {
    let mut app = app();
    let picker = scripted_sheets(&mut app, false);
    let modified = |app: &mut VectorcraftApp| app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    // X clean, A modified and untitled, C modified with a file.
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    modified(&mut app);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    modified(&mut app);
    app.session.active_mut().unwrap().path = Some("/art/c.vectorcraft".into());
    let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.run("file.closeAll", json!({})).unwrap();
    // Save: A's Save panel shows, and X closes meanwhile.
    let id = top(&app);
    answer_dialog(&mut app, id, &[], true);
    assert_eq!(picker.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::Save]);
    app.run("file.close", json!({"index": 0})).unwrap();
    picker.answer_saved("/art/a.vectorcraft", None);
    settle(&mut app);
    assert_eq!(picker.written(), ["/art/a.vectorcraft"], "{}", app.ui.status);
    assert_eq!(app.session.documents().iter().map(|d| d.uid).collect::<Vec<_>>(), [uids[2]], "A closed, C untouched");
    assert!(!open_dialogs(&app).is_empty(), "Save Changes asks about C next");
}

/// Closing a modified background tab whose Save opens a Save panel: once the panel answers, the
/// document closes and the document that was in front before the question is in front again.
#[test]
fn save_changes_answered_by_a_save_dialog_keeps_the_front_document() {
    let mut app = app();
    let picker = scripted_sheets(&mut app, false);
    // A clean and in front, B modified and untitled, C clean.
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.session.set_active(0);
    let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.run("file.close", json!({"index": 1})).unwrap();
    let id = top(&app);
    answer_dialog(&mut app, id, &[], true);
    assert_eq!(picker.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::Save]);
    picker.answer_saved("/art/b.vectorcraft", None);
    settle(&mut app);
    assert_eq!(picker.written(), ["/art/b.vectorcraft"], "{}", app.ui.status);
    assert_eq!(app.session.documents().iter().map(|d| d.uid).collect::<Vec<_>>(), [uids[0], uids[2]]);
    assert_eq!(app.session.active().map(|d| d.uid), Some(uids[0]), "A is in front again");
}

/// One whole frame of the app (its logic and its window, then the close path the desktop host runs); `close` presses
/// the window's close button.
fn app_frame(ctx: &egui::Context, app: &mut VectorcraftApp, close: bool) -> Vec<egui::ViewportCommand> {
    let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))), ..Default::default() };
    if close {
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    }
    let out = crate::testing::part_frame(app, ctx, raw, |app, ui| {
        app.tick(ui.ctx());
        app.ui(ui);
        craft::ui::shell::embed::logic(ui.ctx(), || app.close_decision());
    });
    out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
}

/// The window's close button doesn't close the window while a pick's sheet is on it: the status
/// says a file dialog is open. On Linux, and once the sheet answers, it closes.
#[test]
fn the_window_stays_open_while_a_sheet_is_on_it() {
    for sheets in [true, false] {
        let mut app = app();
        let picker = scripted_sheets(&mut app, sheets);
        let ctx = app.ui_ctx.clone().unwrap();
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app_frame(&ctx, &mut app, false);
        asks(&mut app, "file.open");
        let commands = app_frame(&ctx, &mut app, true);
        assert_eq!(commands.contains(&egui::ViewportCommand::CancelClose), sheets, "sheets: {sheets}");
        assert_eq!(app.ui.status == file_asks::busy(), sheets, "sheets: {sheets}: {}", app.ui.status);
        assert_eq!(app.session.documents().len(), 1);
        picker.cancel();
        app_frame(&ctx, &mut app, false);
        assert!(!app_frame(&ctx, &mut app, true).contains(&egui::ViewportCommand::CancelClose), "sheets: {sheets}");
    }
}

/// `ui.inspect`'s `fileDialog` is the kind of pick that is up (open, save, place, folder), null once
/// it answers.
#[test]
fn inspect_tells_the_file_dialog_open() {
    let mut app = app();
    let picker = scripted(&mut app);
    let ctx = app.ui_ctx.clone().unwrap();
    let shows = |app: &VectorcraftApp| crate::control::inspect(app, &ctx)["fileDialog"].clone();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    assert_eq!(shows(&app), Value::Null);
    /// What asks for a kind of pick.
    type Ask = fn(&mut VectorcraftApp);
    let asks: [(&str, Ask); 4] = [
        ("open", |app| asks(app, "file.open")),
        ("save", |app| asks(app, "file.saveAs")),
        ("place", |app| asks(app, "file.place")),
        ("folder", |app| {
            let id = browse_dialog(app);
            browse_folder(app, id);
        }),
    ];
    for (kind, ask) in asks {
        ask(&mut app);
        assert!(app.file_dialog_open(), "{kind}");
        assert_eq!(shows(&app), json!(kind));
        picker.cancel();
        settle(&mut app);
        assert!(!app.file_dialog_open(), "{kind}");
        assert_eq!(shows(&app), Value::Null, "{kind}");
    }
}

/// Find Fonts' folder button: when the folder picked is one the search refuses (other apps' or the
/// system's folders, such as the home folder's Library where font tools leave fonts to test), the
/// dialog says why. Without it the dialog reads as before the folder was picked, only with "Not
/// found" under each font, and the reason is in the status bar alone.
#[test]
fn missing_fonts_says_why_the_folder_picked_wasnt_searched() {
    let mut app = app();
    let picker = scripted(&mut app);
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    app.services.write = None;
    let root = std::fs::canonicalize(vectorcraft_testkit::temp_dir("missing-fonts-refused")).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Type", "font": "Findme Sans 3"})).unwrap();
    let doc = root.join("doc.vectorcraft");
    app.run("document.save", json!({"path": doc.to_string_lossy()})).unwrap();
    app.session.execute("file.close", &json!({})).unwrap();
    crate::io::open_path(&mut app, &doc.to_string_lossy()).unwrap();
    // Missing Fonts opens from the frame's pump after the open.
    settle(&mut app);
    let missing = find(&app, "missingFonts").expect("Missing Fonts");
    // The test's folder is the home folder: its Library holds other apps' folders.
    app.session.search_rules = Some(vectorcraft_engine::cmd::findfiles::Rules { home: Some(root.clone()), ..Default::default() });
    let fonts = root.join("Library").join("Application Support").join("Font Tool").join("Export");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(fonts.join("Findme.otf"), vectorcraft_testkit::fonts::renamed("Findme Sans 3")).unwrap();
    click(&mut app, "Find in Folder…", dialogs);
    assert_eq!(picker.requests().iter().map(|r| r.kind).collect::<Vec<_>>(), [PickKind::Folder]);
    picker.answer(&[&fonts.to_string_lossy()]);
    let ctx = app.ui_ctx.clone().unwrap();
    let state = |app: &VectorcraftApp| fields_of(app, missing).map(|f| f["state"].clone());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app_frame(&ctx, &mut app, false);
        if state(&app) != Some(json!("searching")) || std::time::Instant::now() > until {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // PLAN: assumed Missing Fonts' `state` field and the line's text carry over (files-w11).
    assert_eq!(state(&app), Some(json!("failed")));
    let text = frame(&ctx, &mut app, vec![], &dialogs).into_iter().map(|(t, _)| t).collect::<Vec<_>>().join("\n");
    assert!(text.contains("VectorCraft doesn't search other apps' or the system's folders"), "{text}");
    assert!(!text.contains("Not found"), "the folder wasn't searched: {text}");
}
