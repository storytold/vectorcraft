//! A script's `alert`, `confirm` and `prompt` in the app: the dialog the run waits at, its buttons,
//! Enter and Escape, Stop, another dialog in the way, quitting while it waits, and the same answers
//! given by a run with nobody to ask; and the Color Picker of `app.showColorPicker`. The helpers
//! over the dialog host are shared with the other script tests.

use craft::color_ui::ui::{ColorPickerDialog, PickedColor};
use craft::ui::dialog::{DialogId, StackDialog, StandardPrompt};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::dialogs::color_picker;
use crate::dialogs::script_run::{self, CLOSE_KIND, CloseQuestion, NOTICE_KIND, ScriptNotice};
use crate::{VectorcraftApp, menus, script_runs};

pub(super) fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), crate::Services::default()));
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    app
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

const DRAW: &str = "vc.run('shape.rectangle', {x:0,y:0,width:10,height:10});";

fn start(app: &mut VectorcraftApp, source: &str) {
    menus::run_ui_command(app, "ui.runScript", &json!({"source": source, "name": "d.js"})).unwrap().unwrap();
}

/// Takes the answers the dialogs gave, as the frame's pump does.
pub(super) fn pump(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().unwrap();
    script_run::pump(app, &ctx);
}

pub(super) fn is_open<D: StackDialog>(app: &VectorcraftApp) -> bool {
    script_run::with_host(app, |host, _| host.is_open::<D>()).unwrap()
}

/// The id of the open dialog with `key`.
pub(super) fn open_id(app: &VectorcraftApp, key: &str) -> DialogId {
    let open = script_run::with_host(app, |host, _| host.open_dialogs()).unwrap();
    open.into_iter().find(|d| d.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("no open dialog {key}")).id
}

/// The fields of the open dialog with `key`.
pub(super) fn fields(app: &VectorcraftApp, key: &str) -> Value {
    let open = script_run::with_host(app, |host, _| host.open_dialogs()).unwrap();
    open.into_iter().find(|d| d.key.as_deref() == Some(key)).and_then(|d| d.fields).unwrap()
}

/// `ui.dialog.set` of one field.
pub(super) fn set(app: &VectorcraftApp, key: &str, field: &str, value: Value) -> Result<(), String> {
    let id = open_id(app, key);
    script_run::with_host(app, |host, ctx| host.set_json_field(ctx, id, field, value)).unwrap()
}

/// `ui.dialog.confirm`, then the frame's pump.
pub(super) fn confirm(app: &mut VectorcraftApp, key: &str) -> bool {
    let id = open_id(app, key);
    let done = script_run::with_host(app, |host, ctx| host.confirm_by_id(ctx, id)).unwrap();
    pump(app);
    done
}

/// `ui.dialog.cancel`, then the frame's pump.
pub(super) fn cancel(app: &mut VectorcraftApp, key: &str) {
    let id = open_id(app, key);
    script_run::with_host(app, |host, ctx| host.cancel_by_id(ctx, id)).unwrap();
    pump(app);
}

/// Opens a question the run knows nothing about, over whatever is open.
pub(super) fn open_blocker(app: &VectorcraftApp) {
    let blocker = StandardPrompt::custom("Blocked").button(craft::ui::dialog::PromptChoice::Proceed, "OK");
    script_run::with_host(app, |host, _| host.open(CloseQuestion(blocker))).unwrap();
}

/// One pass of the dialog host with `events`; a key pressed in it is let go in a second pass, as a
/// person lets go of it. Returns the last pass's output.
pub(super) fn frame(app: &VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
    let ctx = app.ui_ctx.clone().unwrap();
    let dialogs = app.dialogs().unwrap();
    let released: Vec<egui::Event> = events
        .iter()
        .filter_map(|e| match e {
            egui::Event::Key { key, physical_key, modifiers, pressed: true, .. } => {
                Some(egui::Event::Key { key: *key, physical_key: *physical_key, pressed: false, repeat: false, modifiers: *modifiers })
            }
            _ => None,
        })
        .collect();
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)));
    let pressed = !released.is_empty();
    let mut last = egui::FullOutput::default();
    for events in std::iter::once(events).chain(pressed.then_some(released)) {
        last = ctx.run_ui(egui::RawInput { events, screen_rect, ..Default::default() }, |ui| dialogs.with(|host| host.run(ui)));
        last.textures_delta.clear();
    }
    last
}

pub(super) fn key(k: egui::Key) -> egui::Event {
    egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
}

/// Polls the run until it ends, or until its dialog is open (true).
fn until_dialog(app: &mut VectorcraftApp) -> bool {
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return false;
        }
        script_runs::poll(app);
        if is_open::<ScriptNotice>(app) {
            return true;
        }
    }
    panic!("the script neither ended nor opened a dialog");
}

fn log(app: &VectorcraftApp) -> Value {
    app.session.script_report().unwrap()["log"].clone()
}

#[test]
fn alert_waits_and_stop_rolls_back_first_dialog() {
    for stop_with_cancel in [false, true] {
        let mut app = app();
        start(&mut app, &format!("{DRAW} while (1) {{ alert('x'); }}"));
        assert!(until_dialog(&mut app));
        let f = fields(&app, NOTICE_KIND);
        assert_eq!((f["kind"].as_str(), f["message"].as_str(), f["title"].clone()), (Some("alert"), Some("x"), Value::Null));
        for _ in 0..5 {
            script_runs::poll(&mut app);
        }
        assert!(app.session.script_running(), "the run waits at the dialog");
        assert_eq!(objects(&app), 0);
        if stop_with_cancel {
            cancel(&mut app, NOTICE_KIND);
        } else {
            set(&app, NOTICE_KIND, "answer", json!("stop")).unwrap();
            assert!(confirm(&mut app, NOTICE_KIND));
        }
        assert!(!is_open::<ScriptNotice>(&app));
        assert!(!until_dialog(&mut app));
        assert_eq!(objects(&app), 0);
        assert_eq!(app.ui.status, "The script was stopped and its changes were undone");
        assert_eq!(app.session.script_report().unwrap()["dialogs"][0]["answer"], "stop");
    }
}

#[test]
fn confirm_enter_uses_no_as_default() {
    for (no_as_default, want) in [(true, false), (false, true)] {
        let mut app = app();
        start(&mut app, &format!("vc.log(confirm('Go?', {no_as_default}));"));
        assert!(until_dialog(&mut app));
        frame(&app, vec![]);
        frame(&app, vec![]);
        frame(&app, vec![key(egui::Key::Enter)]);
        pump(&mut app);
        assert!(!is_open::<ScriptNotice>(&app), "Enter answers the dialog: noAsDefault {no_as_default}");
        assert!(!until_dialog(&mut app));
        assert_eq!(log(&app), json!([want]), "noAsDefault {no_as_default}");
    }
    for no_as_default in [false, true] {
        let mut app = app();
        start(&mut app, &format!("vc.log(confirm('Go?', {no_as_default}));"));
        assert!(until_dialog(&mut app));
        frame(&app, vec![]);
        frame(&app, vec![]);
        frame(&app, vec![key(egui::Key::Escape)]);
        pump(&mut app);
        assert!(!until_dialog(&mut app));
        assert_eq!(log(&app), json!([false]), "Escape is No: noAsDefault {no_as_default}");
    }
}

#[test]
fn prompt_ok_and_cancel_return_values() {
    let mut app = app();
    start(&mut app, "vc.log(prompt('Name', 'pre', 'Ask')); vc.log(prompt('Again', 'x'));");
    assert!(until_dialog(&mut app));
    let heading = script_run::with_host(&app, |host, _| host.get::<ScriptNotice>().map(ScriptNotice::heading)).unwrap();
    assert_eq!(heading.as_deref(), Some("Ask"));
    assert_eq!(fields(&app, NOTICE_KIND)["value"], "pre");
    set(&app, NOTICE_KIND, "value", json!("typed")).unwrap();
    assert!(confirm(&mut app, NOTICE_KIND));
    assert!(until_dialog(&mut app));
    cancel(&mut app, NOTICE_KIND);
    assert!(!until_dialog(&mut app));
    assert_eq!(log(&app), json!(["typed", null]));
    let mut app = self::app();
    start(&mut app, "vc.log(prompt('Name', ''));");
    assert!(until_dialog(&mut app));
    let e = set(&app, NOTICE_KIND, "answer", json!("yes")).unwrap_err();
    assert!(e.contains("ok, cancel, stop"), "{e}");
    assert!(is_open::<ScriptNotice>(&app), "a wrong answer leaves the dialog open");
    assert!(confirm(&mut app, NOTICE_KIND));
    assert!(!until_dialog(&mut app));
    assert_eq!(log(&app), json!([""]), "OK with an empty field gives an empty string");
}

#[test]
fn the_dialog_opens_in_the_frame_the_run_waits() {
    let mut app = app();
    start(&mut app, "alert('x');");
    script_runs::poll(&mut app);
    assert!(is_open::<ScriptNotice>(&app), "no later frame is drawn while the run waits");
}

#[test]
fn a_prompt_text_past_the_limit_keeps_the_dialog_open() {
    let mut app = app();
    start(&mut app, "vc.log(prompt('Name', ''));");
    assert!(until_dialog(&mut app));
    let long = "x".repeat(vectorcraft_engine::script::dialogs::MAX_PROMPT_BYTES + 1);
    let e = set(&app, NOTICE_KIND, "value", json!(long)).unwrap_err();
    assert!(e.contains("bytes"), "{e}");
    assert!(is_open::<ScriptNotice>(&app), "the refused value leaves the dialog open");
    assert!(app.session.script_dialog().is_some(), "the run still waits");
    set(&app, NOTICE_KIND, "value", json!("ok")).unwrap();
    assert!(confirm(&mut app, NOTICE_KIND));
    assert!(!until_dialog(&mut app));
    assert_eq!(log(&app), json!(["ok"]));
}

#[test]
fn dialog_wait_survives_another_modal() {
    let mut app = app();
    start(&mut app, "alert('x');");
    open_blocker(&app);
    for _ in 0..3 {
        script_runs::poll(&mut app);
    }
    assert!(is_open::<CloseQuestion>(&app), "the open dialog stays");
    assert!(!is_open::<ScriptNotice>(&app), "nothing opens over it");
    assert!(app.session.script_running());
    cancel(&mut app, CLOSE_KIND);
    assert!(until_dialog(&mut app), "the script's dialog opens once the other one closes");
}

#[test]
fn close_while_notification_waits_abandons_or_resumes() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} vc.log(prompt('Name', ''));"));
    assert!(until_dialog(&mut app));
    set(&app, NOTICE_KIND, "value", json!("half")).unwrap();
    script_run::ask_to_close(&mut app);
    assert!(is_open::<CloseQuestion>(&app));
    script_runs::poll(&mut app);
    assert!(is_open::<CloseQuestion>(&app), "the run goes on waiting");
    cancel(&mut app, CLOSE_KIND);
    assert_eq!(fields(&app, NOTICE_KIND)["value"], "half", "Cancel shows the dialog again as it was");
    script_run::ask_to_close(&mut app);
    assert!(confirm(&mut app, CLOSE_KIND));
    assert!(!app.session.script_running());
    assert!(!is_open::<ScriptNotice>(&app), "no dialog is left for the abandoned run");
    assert_eq!(objects(&app), 0);
    assert_eq!(app.ui.status, "The script was stopped and its changes were undone");
}

#[test]
fn close_question_ends_with_a_run_stopped_another_way() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} vc.log(prompt('Name', ''));"));
    assert!(until_dialog(&mut app));
    script_run::ask_to_close(&mut app);
    assert!(is_open::<CloseQuestion>(&app));
    assert_eq!(app.run("script.stop", json!({})).unwrap()["stopped"], true);
    script_runs::poll(&mut app);
    assert!(!app.session.script_running());
    assert!(!is_open::<CloseQuestion>(&app), "the question about a run that has ended closes");
    assert!(!is_open::<ScriptNotice>(&app), "the dialog the run no longer waits at is not shown again");
    assert_eq!(objects(&app), 0);
    // A run asked to stop no longer waits at its dialog: the question stays, over nothing.
    let mut app = self::app();
    start(&mut app, &format!("{DRAW} vc.log(prompt('Name', ''));"));
    assert!(until_dialog(&mut app));
    script_run::ask_to_close(&mut app);
    app.run("script.stop", json!({})).unwrap();
    script_run::forget::<ScriptNotice>(&app);
    assert!(is_open::<CloseQuestion>(&app), "the run has not ended yet");
    cancel(&mut app, CLOSE_KIND);
    assert!(!is_open::<ScriptNotice>(&app), "Cancel does not show a dialog the run no longer waits at");
}

#[test]
fn notification_message_with_control_characters_and_a_multiline_title() {
    let mut app = app();
    let long = "word ".repeat(3_000);
    start(&mut app, &format!("alert({long:?} + '\\u0007\\r\\n', 'T\\nitle', true);"));
    assert!(until_dialog(&mut app));
    let heading = script_run::with_host(&app, |host, _| host.get::<ScriptNotice>().map(ScriptNotice::heading)).unwrap();
    assert_eq!(heading.as_deref(), Some("T\u{240a}itle"));
    assert!(confirm(&mut app, NOTICE_KIND));
    assert!(!until_dialog(&mut app));
    assert_eq!(app.session.script_report().unwrap()["dialogs"][0]["errorIcon"], true);
}

#[test]
fn the_app_and_a_run_with_nobody_to_ask_give_the_same_results() {
    let source = "vc.log(confirm('a', true, 'T')); vc.log(prompt('b', 'p')); alert('c', undefined, false); vc.log('done');";
    let mut app = app();
    start(&mut app, source);
    for a in ["yes", "ok", "ok"] {
        assert!(until_dialog(&mut app));
        set(&app, NOTICE_KIND, "answer", json!(a)).unwrap();
        assert!(confirm(&mut app, NOTICE_KIND));
    }
    assert!(!until_dialog(&mut app));
    let framed = app.session.script_report().unwrap().clone();
    let mut s = Session::new();
    let given = json!([{"kind": "confirm", "value": true}, {"kind": "prompt", "value": "p"}, {"kind": "alert", "value": "ok"}]);
    let headless = s.execute("script.run", &json!({"source": source, "name": "d.js", "answers": given})).unwrap();
    assert_eq!(framed["log"], headless["log"]);
    assert_eq!(framed["dialogs"], headless["dialogs"]);
    assert_eq!(framed["log"], json!([true, "p", "done"]));
}

/// Is the Color Picker of the script's `app.showColorPicker` open?
fn picker_open(app: &VectorcraftApp) -> bool {
    let id = color_picker::script_picker(app);
    id.is_some_and(|id| script_run::with_host(app, |host, _| host.get_by_id::<ColorPickerDialog>(id).is_some()).unwrap())
}

/// Polls the run until it ends, or until its Color Picker is open (true).
fn until_picker(app: &mut VectorcraftApp) -> bool {
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return false;
        }
        script_runs::poll(app);
        if picker_open(app) {
            return true;
        }
    }
    panic!("the script neither ended nor opened the Color Picker");
}

/// Answers the script's Color Picker with OK (or Cancel), then runs the frame's picker pump.
fn answer_picker(app: &mut VectorcraftApp, ok: bool) {
    let id = color_picker::script_picker(app).unwrap();
    let ctx = app.ui_ctx.clone().unwrap();
    let done = script_run::with_host(app, |host, ctx| if ok { host.confirm_by_id(ctx, id) } else { host.cancel_by_id(ctx, id) }).unwrap();
    assert!(done);
    color_picker::pump(app, &ctx);
}

#[test]
fn the_color_picker_waits_and_answers_the_run() {
    let mut app = app();
    let gray = "var g = new GrayColor(); g.gray = 40;";
    start(
        &mut app,
        &format!(
            "{DRAW} var c = new RGBColor(); c.red = 255; var p = app.showColorPicker(c); vc.log([p.typename, p.red, p.green, p.blue].join()); \
             {gray} var q = app.showColorPicker(g); vc.log([q.typename, q.gray].join()); var r = app.showColorPicker(g); vc.log([r.typename, r.gray].join());"
        ),
    );
    assert!(until_picker(&mut app));
    assert!(!is_open::<ScriptNotice>(&app), "the picker opens in place of a notice");
    set(&app, color_picker::SCRIPT_KEY, "color", json!("#00ff00")).unwrap();
    let ctx = app.ui_ctx.clone().unwrap();
    color_picker::pump(&mut app, &ctx);
    assert_ne!(crate::panels::current_paints(&app).0.color().map(|c| c.to_hex()).as_deref(), Some("#00ff00"), "the selection shows no preview");
    answer_picker(&mut app, true);
    assert!(until_picker(&mut app), "the next call opens the picker again");
    answer_picker(&mut app, true);
    assert!(until_picker(&mut app));
    answer_picker(&mut app, false);
    assert!(!until_picker(&mut app));
    assert_eq!(
        log(&app),
        json!(["RGBColor,0,255,0", "GrayColor,40", "GrayColor,40"]),
        "OK gives the color picked, untouched its own color, Cancel the color given"
    );
    assert_ne!(crate::panels::current_paints(&app).0.color().map(|c| c.to_hex()).as_deref(), Some("#00ff00"), "OK paints nothing");
    let dialogs = app.session.script_report().unwrap()["dialogs"].clone();
    assert_eq!(dialogs[2]["answer"], Value::Null, "Cancel is listed as null");
}

#[test]
fn the_same_color_set_in_another_model_is_a_new_pick() {
    let mut app = app();
    start(&mut app, "var g = new GrayColor(); g.gray = 40; var p = app.showColorPicker(g); vc.log(p.typename);");
    assert!(until_picker(&mut app));
    // The gray's own sRGB, set as a plain color, as a screen sample sets one.
    let open = app.color_dialogs.pickers.iter().find(|o| Some(o.id) == color_picker::script_picker(&app)).map(|o| (o.id, o.start)).unwrap();
    script_run::with_host(&app, |host, _| host.get_mut_by_id::<ColorPickerDialog>(open.0).map(|d| d.set_color(PickedColor::rgb(open.1)))).unwrap();
    answer_picker(&mut app, true);
    assert!(!until_picker(&mut app));
    assert_eq!(log(&app), json!(["RGBColor"]), "the same sRGB as a plain color is an RGB pick, not the gray the script gave");
}

#[test]
fn stopping_or_quitting_the_run_closes_its_color_picker() {
    let source = format!("{DRAW} var c = new RGBColor(); app.showColorPicker(c); vc.log('after');");
    let mut app = app();
    start(&mut app, &source);
    assert!(until_picker(&mut app));
    assert_eq!(app.run("script.stop", json!({})).unwrap()["stopped"], true);
    assert!(!until_picker(&mut app));
    assert!(!picker_open(&app), "no picker is left for the stopped run");
    assert!(color_picker::script_picker(&app).is_none());
    assert_eq!(objects(&app), 0, "the run's edit goes back");
    assert_eq!(log(&app), json!([]));
    let mut app = self::app();
    start(&mut app, &source);
    assert!(until_picker(&mut app));
    script_run::ask_to_close(&mut app);
    assert!(is_open::<CloseQuestion>(&app), "quitting asks over the picker");
    cancel(&mut app, CLOSE_KIND);
    assert!(picker_open(&app), "Cancel shows the picker again");
    script_run::ask_to_close(&mut app);
    assert!(confirm(&mut app, CLOSE_KIND));
    assert!(!app.session.script_running());
    assert!(!picker_open(&app), "no picker is left for the abandoned run");
    assert_eq!(objects(&app), 0);
}

#[test]
fn the_color_picker_opens_on_the_scripts_color_with_only_web_colors_left_on() {
    let mut app = app();
    let ctx = app.ui_ctx.clone().unwrap();
    // A frame of the window's dialogs starts the open pickers with the app's host, whose settings
    // keep what a picker remembers.
    let frame = |app: &mut VectorcraftApp| {
        let _ = crate::testing::part_frame(app, &ctx, egui::RawInput::default(), |app, ui| {
            if let Some(dialogs) = app.dialogs() {
                dialogs.with(|host| host.run(ui));
            }
        });
    };
    // A fill picker closed with Only Web Colors on, which later fill pickers recall.
    color_picker::open(&mut app, &json!({})).unwrap();
    frame(&mut app);
    set(&app, "fill", "webColors", json!(true)).unwrap();
    let id = open_id(&app, "fill");
    assert!(script_run::with_host(&app, |host, ctx| host.confirm_by_id(ctx, id)).unwrap());
    color_picker::pump(&mut app, &ctx);
    color_picker::open(&mut app, &json!({})).unwrap();
    frame(&mut app);
    assert_eq!(fields(&app, "fill")["webColors"], json!(true), "the next fill picker recalls Only Web Colors");
    let id = open_id(&app, "fill");
    assert!(script_run::with_host(&app, |host, ctx| host.confirm_by_id(ctx, id)).unwrap());
    color_picker::pump(&mut app, &ctx);
    // A frame without the picker drops its overlay.
    frame(&mut app);
    assert!(!script_run::busy(&app), "no picker is left open");
    start(
        &mut app,
        "var c = new RGBColor(); c.red = 18; c.green = 52; c.blue = 86; var p = app.showColorPicker(c); vc.log([p.typename, p.red, p.green, p.blue].join());",
    );
    assert!(until_picker(&mut app));
    frame(&mut app);
    answer_picker(&mut app, true);
    assert!(!until_picker(&mut app));
    assert_eq!(log(&app), json!(["RGBColor,18,52,86"]), "untouched OK gives the script's color, not its nearest web color");
}

#[test]
fn a_pick_takes_the_model_of_the_document_the_script_made() {
    for (doc, made, want) in [("{}", "CMYK", "CMYKColor"), ("{\"colorMode\": \"cmyk\"}", "RGB", "RGBColor")] {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), crate::Services::default()));
        app.run("file.new", serde_json::from_str(doc).unwrap()).unwrap();
        app.session.prefs.show_external_script_warning = false;
        start(
            &mut app,
            &format!(
                "app.documents.add(DocumentColorSpace.{made}, 50, 60, 1); var c = new RGBColor(); c.red = 255; \
                 var p = app.showColorPicker(c); vc.log(p.typename);"
            ),
        );
        assert!(until_picker(&mut app));
        set(&app, color_picker::SCRIPT_KEY, "color", json!("#00ff00")).unwrap();
        answer_picker(&mut app, true);
        assert!(!until_picker(&mut app));
        assert_eq!(log(&app), json!([want]), "an RGB pick in the {made} document the script made");
    }
}
