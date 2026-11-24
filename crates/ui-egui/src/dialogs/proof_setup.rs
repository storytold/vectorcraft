//! View → Proof Setup → Customize…: the device to simulate (the working CMYK space, or any CMYK or
//! RGB profile), Preserve CMYK Numbers or Preserve RGB Numbers, the rendering intent (used while
//! the numbers are not preserved), Simulate Paper Color and Simulate Black Ink (CMYK devices only).
//! Preview shows the proof on the canvas while the dialog is open; OK sets the proof setup and
//! turns Proof Colors on (`view.proofSetup`), and Cancel puts the earlier proof view back. Save…
//! writes the dialog's settings to a proof setup file (`view.proofSetup.save`) and Load… reads one
//! into the dialog (`view.proofSetup.read`). Both are desktop only. Their pickers start in the
//! Proof Setups folder next to the preferences, whose setups View → Proof Setup lists below
//! Customize… ([`saved_items`]): Save… makes the folder when it may be written, and Load…'s picker
//! starts in its own default folder until the folder exists.
//!
//! The dialog is a held [`HostedParamDialog`]: [`pump`] runs once per frame after the shell pass,
//! previews the dialog's values, puts the view back when the dialog stops previewing or is covered,
//! answers the Save…/Load… buttons and keeps the saved-setups listing.
//!
//! Fields: `device` (`workingCmyk`, `cmyk:<profile>` or `rgb:<profile>`), `preserveNumbers`,
//! `intent`, `simulatePaper`, `simulateBlack` and `preview`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use craft::host::queue::{JobQueue, Priority, QueueOptions};
use craft::host::{PickRequest, Picked, Wake};
use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::sections::{DialogRole, FooterButton};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Intent, ProfileKind, ProofTarget};
use vectorcraft_engine::cmd::colormgmt::proof_file::{self, EXT};
use vectorcraft_render::proof;

use super::host::{HeldOptions, open_held_with};
use crate::VectorcraftApp;
use crate::file_asks::{Owner, ask};
use crate::menus::Node;

/// The dialog kind of Proof Setup › Customize.
pub const KIND: &str = "proofSetup";

const CMD: &str = "view.proofSetup";

/// The device a proof target that is not a device (an RGB display or a color blindness type)
/// starts the dialog on.
const WORKING: &str = "workingCmyk";

/// The footer button ids.
const SAVE: &str = "save";
const LOAD_BUTTON: &str = "load";

/// The open Proof Setup dialog and what its preview did to the proof view (`UiState::proof_setup`).
#[derive(Clone, Debug)]
pub(crate) struct Open {
    /// The dialog on the window's stack.
    id: DialogId,
    /// The proof view the dialog found, which Cancel puts back after a preview.
    before: proof::ProofView,
    /// The proof view the preview last put on the canvas; `None` while no preview of this dialog is shown.
    shown: Option<proof::ProofView>,
    /// The `view.proofSetup` parameters the preview last sent, so an unchanged dialog does not send them
    /// again (the engine may normalize the view, so `shown` cannot say it).
    sent: Option<Value>,
}

/// Open the dialog on the current proof setup. While it is open, it stays as it is, so Cancel
/// still puts back the view it found first.
pub fn open(app: &mut VectorcraftApp) {
    let held = app.ui.proof_setup.as_ref().is_some_and(|o| app.dialogs().is_some_and(|d| d.with(|host| host.stack().contains(o.id))));
    if held {
        return;
    }
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status(tl!("There is no window to show Proof Setup in."));
        return;
    };
    let before = proof::view();
    let s = &before.setup;
    let device = match &s.target {
        ProofTarget::WorkingCmyk | ProofTarget::Cmyk(_) | ProofTarget::Rgb(_) => s.target.id(),
        _ => WORKING.to_string(),
    };
    let fields = DialogFields::from_pairs([
        ("device", FieldValue::Text(device)),
        ("preserveNumbers", FieldValue::Bool(s.preserve_numbers)),
        ("intent", FieldValue::Text(s.intent.id().to_string())),
        ("simulatePaper", FieldValue::Bool(s.simulate_paper)),
        ("simulateBlack", FieldValue::Bool(s.simulate_black)),
    ]);
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, build);
    let options = HeldOptions { cancel: Some(Box::new(cancel)), ..Default::default() };
    match open_held_with(app, dialog, |app, result: &ParamResult| confirm(app, &result.fields), options) {
        Some(id) => app.ui.proof_setup = Some(Open { id, before, shown: None, sent: None }),
        None => app.status(tl!("There is no window to show Proof Setup in.")),
    }
}

/// The devices to simulate, as (`device` value, name shown): the working CMYK space, then every
/// CMYK profile and every RGB profile.
fn devices() -> Vec<(String, String)> {
    let profiles = cms::profiles();
    let of = |kind: ProfileKind, prefix: &str| {
        profiles.iter().filter(move |p| p.kind == kind).map(move |p| (format!("{prefix}{}", p.name), p.name.clone())).collect::<Vec<_>>()
    };
    std::iter::once((WORKING.to_string(), tl!("Working CMYK").to_string()))
        .chain(of(ProfileKind::Cmyk, "cmyk:"))
        .chain(of(ProfileKind::Rgb, "rgb:"))
        .collect()
}

/// Whether `device` is an RGB profile.
fn rgb_device(device: &str) -> bool {
    device.starts_with("rgb:")
}

/// The dialog over the host's values, made again every pass.
fn build(fields: &mut DialogFields) -> ParamDialog<'_> {
    // The web has no folder to keep setups in.
    let footer = if cfg!(target_arch = "wasm32") {
        Vec::new()
    } else {
        vec![
            (SAVE.to_string(), FooterButton::new(tl!("Save…").to_string(), DialogRole::Other)),
            (LOAD_BUTTON.to_string(), FooterButton::new(tl!("Load…").to_string(), DialogRole::Other)),
        ]
    };
    ParamDialog::new(KIND, tl!("Proof Setup").to_string(), rows(fields)).values(Bind::bound(fields)).previews(true).extra_buttons(footer)
}

/// The rows for the values `fields` hold now.
fn rows(fields: &DialogFields) -> Vec<Param> {
    let mut devices = devices();
    let device = fields.text("device").unwrap_or(WORKING);
    if !devices.iter().any(|(id, _)| id == device) {
        // A profile that is no longer loaded shows its name as it is.
        devices.push((device.to_string(), device.split_once(':').map_or(device, |(_, name)| name).to_string()));
    }
    let rgb = rgb_device(device);
    let paper = fields.bool("simulatePaper").unwrap_or(false);
    // Paper and black ink are a printing press's: an RGB device has neither.
    let cmyk: Vec<String> = devices.iter().filter(|(id, _)| !rgb_device(id)).map(|(id, _)| id.clone()).collect();
    let preserve = if rgb { tl!("Preserve RGB Numbers") } else { tl!("Preserve CMYK Numbers") };
    let intents = Intent::ALL.iter().map(|i| (i.id().to_string(), i.label().to_string().into())).collect();
    // Simulate Paper Color shows the press black too, so its box shows checked while paper is on.
    let black = Param::new("simulateBlack", tl!("Simulate Black Ink").to_string(), ParamKind::bool(false));
    let black =
        if paper && !rgb { black.enabled_when("simulatePaper", false).disabled_value(true) } else { black.enabled_when_in("device", cmyk.clone()) };
    vec![
        Param::new(
            "device",
            tl!("Device to Simulate:").to_string(),
            ParamKind::choice(devices.into_iter().map(|(id, name)| (id, name.into())).collect(), Some(WORKING.to_string())),
        ),
        Param::new("preserveNumbers", preserve.to_string(), ParamKind::bool(false)),
        Param::new("intent", tl!("Rendering intent:").to_string(), ParamKind::choice(intents, None)).enabled_when("preserveNumbers", false),
        Param::new("simulatePaper", tl!("Simulate Paper Color").to_string(), ParamKind::bool(false)).enabled_when_in("device", cmyk),
        black,
    ]
}

/// What the dialog's values send to `view.proofSetup`.
fn params(fields: &DialogFields) -> Value {
    json!({
        "target": fields.text("device").unwrap_or(WORKING),
        "preserveNumbers": fields.bool("preserveNumbers").unwrap_or(false),
        "intent": fields.text("intent").unwrap_or_default(),
        "simulatePaper": fields.bool("simulatePaper").unwrap_or(false),
        "simulateBlack": fields.bool("simulateBlack").unwrap_or(false),
        "proof": true,
    })
}

/// What [`pump`] reads from the open dialog in one visit to the host.
struct Seen {
    /// The dialog is the top one: nothing covers it.
    top: bool,
    /// Its Preview box is on.
    previewing: bool,
    fields: DialogFields,
    /// The footer button pressed since the last frame.
    action: Option<String>,
}

/// The dialog `id` as the window's host holds it; `None` once it has closed.
fn seen(app: &VectorcraftApp, id: DialogId) -> Option<Seen> {
    app.dialogs()?.with(|host| {
        let top = host.stack().is_top(id);
        let dialog = host.get_mut_by_id::<HostedParamDialog>(id)?;
        Some(Seen { top, previewing: dialog.is_previewing(), fields: dialog.fields().clone(), action: dialog.take_footer_action() })
    })
}

/// Once per frame after the shell pass ([`super::host::pump`]): the saved setups' listing, then the
/// open dialog. A dialog on top that previews shows its values on the canvas, sent again whenever
/// they change; covered by another dialog, or with Preview off, the earlier proof view comes back.
/// Cancel is not read here: [`cancel`] runs when the host reports the dialog closed.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    list_saved(app, ctx);
    let Some(id) = app.ui.proof_setup.as_ref().map(|o| o.id) else { return };
    let Some(seen) = seen(app, id) else { return };
    if seen.top && seen.previewing {
        preview(app, &seen.fields);
    } else {
        restore(app);
    }
    if let Some(action) = seen.action {
        act(app, id, &seen.fields, &action);
    }
}

/// Show the dialog's values on the canvas unless they are the ones already shown.
fn preview(app: &mut VectorcraftApp, fields: &DialogFields) {
    let p = params(fields);
    let Some(open) = app.ui.proof_setup.as_ref() else { return };
    if open.sent.as_ref() == Some(&p) {
        return;
    }
    // A preview that starts while the canvas shows no preview of this dialog (none yet, or a proof
    // setup a command set since) takes the view the canvas shows now as the one to put back.
    let now = proof::view();
    let before = if open.shown.as_ref().is_some_and(|shown| dialog_part(&now) == dialog_part(shown)) { open.before.clone() } else { now };
    match app.run(CMD, p.clone()) {
        Ok(_) => {
            if let Some(open) = app.ui.proof_setup.as_mut() {
                (open.before, open.sent, open.shown) = (before, Some(p), Some(proof::view()));
            }
        }
        Err(e) => app.status(e),
    }
}

/// Put back the proof view the dialog found before its preview, and forget the preview. A proof
/// setup set since the preview (by `view.proofSetup`, for example) stays.
fn restore(app: &mut VectorcraftApp) {
    let Some(open) = app.ui.proof_setup.as_mut() else { return };
    open.sent = None;
    let Some(shown) = open.shown.take() else { return };
    let before = open.before.clone();
    if dialog_part(&proof::view()) == dialog_part(&shown) {
        set_view(app, &before);
    }
}

/// The parts of a proof view the dialog sets: the device, the intent, the three options and Proof
/// Colors.
fn dialog_part(v: &proof::ProofView) -> (&ProofTarget, Intent, bool, bool, bool, bool) {
    let s = &v.setup;
    (&s.target, s.intent, s.simulate_paper, s.preserve_numbers, s.simulate_black, v.proof_colors)
}

/// Set the dialog's parts of the proof view to those of `before` and redraw the documents. The view
/// is set directly, not through `view.proofSetup`, which refuses a device profile removed since it
/// was set; Overprint Preview stays as the user set it.
fn set_view(app: &mut VectorcraftApp, before: &proof::ProofView) {
    let mut v = proof::view();
    let was = v.clone();
    v.setup.target = before.setup.target.clone();
    v.setup.intent = before.setup.intent;
    v.setup.simulate_paper = before.setup.simulate_paper;
    v.setup.preserve_numbers = before.setup.preserve_numbers;
    v.setup.simulate_black = before.setup.simulate_black;
    v.proof_colors = before.proof_colors;
    if v == was {
        return;
    }
    proof::set_view(v);
    let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    for uid in uids {
        if let Some(d) = app.session.document_mut(uid) {
            d.revision += 1;
        }
    }
}

/// OK: set the proof setup and turn Proof Colors on. An `Err` keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, fields: &DialogFields) -> Result<(), String> {
    app.run(CMD, params(fields))?;
    app.ui.proof_setup = None;
    Ok(())
}

/// Cancel: after a preview, the earlier proof view comes back.
fn cancel(app: &mut VectorcraftApp) {
    restore(app);
    app.ui.proof_setup = None;
}

/// A footer button was pressed in the dialog `id` holding `fields`.
fn act(app: &mut VectorcraftApp, id: DialogId, fields: &DialogFields, action: &str) {
    match action {
        SAVE => save(app, fields),
        LOAD_BUTTON => load(app, id),
        _ => {}
    }
}

/// Save…: the dialog's settings go to the picked file, and the menu lists the saved setups again.
fn save(app: &mut VectorcraftApp, fields: &DialogFields) {
    // The picker opens in the folder, made for it when it may be written. [`list_saved`] removes
    // it again when the picker closes without a file in it.
    let dir = app.session.proof_setup_dir().map(Path::to_path_buf);
    let made = dir
        .clone()
        .filter(|dir| !dir.is_dir() && vectorcraft_engine::file_access::check_write(&dir.to_string_lossy()).is_ok())
        .filter(|dir| std::fs::create_dir_all(dir).is_ok());
    if made.is_some() {
        saved().made = made;
    }
    let mut request =
        PickRequest::save("proof-setups").file_name(format!("{}.{EXT}", tl!("Proof Setup"))).filter("Proof setups", &[EXT]).suggested_extension(EXT);
    if let Some(dir) = dir {
        request = request.directory(dir);
    }
    let mut p = params(fields);
    if let Some(o) = p.as_object_mut() {
        o.remove("proof");
    }
    ask(app, request, Owner::Free, move |app, picked| {
        let Picked::Saved { path, .. } = picked else { return Ok(Value::Null) };
        let path = path.to_string_lossy().into_owned();
        let mut p = p;
        if let Some(o) = p.as_object_mut() {
            o.insert("path".into(), json!(path));
        }
        app.run("view.proofSetup.save", p)?;
        refresh_saved(app);
        app.status(format!("Saved {path}"));
        Ok(Value::Null)
    });
}

/// Load…: the picked file's settings fill the dialog `id` (Preview then shows them).
fn load(app: &mut VectorcraftApp, id: DialogId) {
    let mut request = PickRequest::open("proof-setups").filter("Proof setups", &[EXT]);
    // Before a setup is saved there is no folder, and the picker starts in its own default folder.
    if let Some(dir) = app.session.proof_setup_dir().filter(|dir| dir.is_dir()) {
        request = request.directory(dir);
    }
    ask(app, request, Owner::Free, move |app, picked| {
        let Some(path) = picked.first_path() else { return Ok(Value::Null) };
        let r = app.run("view.proofSetup.read", json!({ "path": path.to_string_lossy() }))?;
        let fields = [
            ("device", "target"),
            ("intent", "intent"),
            ("preserveNumbers", "preserveNumbers"),
            ("simulatePaper", "simulatePaper"),
            ("simulateBlack", "simulateBlack"),
        ];
        // The dialog may have closed while the picker was open: then there is nothing to fill.
        // PLAN: sh.K23's `set_json_fields` refuses a row that is disabled by the file's own device (an RGB
        // device disables paper and black ink), so the whole load would fail; the values are set unchecked.
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| {
                if let Some(dialog) = host.get_mut_by_id::<HostedParamDialog>(id) {
                    for (field, key) in fields {
                        match &r[key] {
                            Value::Bool(b) => dialog.set_field(field, FieldValue::Bool(*b)),
                            Value::String(s) => dialog.set_field(field, FieldValue::Text(s.clone())),
                            _ => {}
                        }
                    }
                }
            });
        }
        Ok(Value::Null)
    });
}

// ---------- the saved setups in View › Proof Setup ----------

/// The command a saved setup's menu item runs.
pub const LOAD: &str = "view.proofSetup.load";

#[derive(Default)]
struct Saved {
    /// (name, path) of each setup, as `view.proofSetup.list` last gave them.
    setups: Vec<(&'static str, String)>,
    /// How many listings differed from the one before (the macOS menu bar follows it).
    revision: u64,
    /// A first listing was asked for.
    started: bool,
    /// The session's count of saved setups at the last [`pump`] (`Session::proof_setup_saves`).
    saves: u64,
    /// The Proof Setups folder that Save… made for its picker, removed by [`pump`] once no picker
    /// is open if it is still empty.
    made: Option<PathBuf>,
    /// The listings on the host's jobs, as (name, path) of each setup; made by the first listing.
    queue: Option<JobQueue<(), Vec<(String, String)>>>,
}

/// The listing the menus show. One per process in the app; one per thread in unit tests, which run
/// in parallel threads, each with its own folder.
fn saved() -> MutexGuard<'static, Saved> {
    #[cfg(not(test))]
    let saved: &'static Mutex<Saved> = {
        static S: std::sync::OnceLock<Mutex<Saved>> = std::sync::OnceLock::new();
        S.get_or_init(Default::default)
    };
    #[cfg(test)]
    let saved: &'static Mutex<Saved> = {
        thread_local!(static S: &'static Mutex<Saved> = Box::leak(Box::default()));
        S.with(|s| *s)
    };
    saved.lock().unwrap_or_else(|e| e.into_inner())
}

/// The setups in `dir` as `view.proofSetup.list` lists them: file names only, at most 100 (the
/// engine's cap).
fn listing(dir: &Path) -> Vec<(String, String)> {
    // The app's own folder, read for the person at the keyboard.
    let listed = vectorcraft_engine::file_access::unconfined(|| proof_file::listed(dir));
    listed
        .ok()
        .flatten()
        .and_then(|r| r["setups"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| Some((e["name"].as_str()?.to_string(), e["path"].as_str()?.to_string())))
        .collect()
}

/// Read the Proof Setups folder's listing again on the host's jobs, unless the session has no
/// folder. A listing asked for while one runs waits for it; one still waiting is replaced. The
/// window is asked for a frame when a listing ends.
pub fn refresh_saved(app: &VectorcraftApp) {
    let Some(dir) = app.session.proof_setup_dir().map(Path::to_path_buf) else { return };
    let mut saved = saved();
    let queue = saved.queue.get_or_insert_with(|| {
        let wake = app.ui_ctx.clone().map_or_else(Wake::none, |ctx| Wake::new(move || ctx.request_repaint()));
        JobQueue::new(app.host.jobs_shared().clone(), wake, QueueOptions { lanes: 1, name: "vectorcraft-proof-setups" })
    });
    queue.submit((), Priority::BACKGROUND, move |_| listing(&dir));
}

/// Take in the listings that ended, keeping the last. A name past the menu's name limit is left out.
fn take() {
    let mut saved = saved();
    let Some(listed) = saved.queue.as_ref().and_then(|queue| std::iter::from_fn(|| queue.try_recv()).last()) else { return };
    let setups: Vec<(&'static str, String)> =
        listed.result.into_iter().filter_map(|(name, path)| Some((crate::scripts_menu::intern(&name)?, path))).collect();
    if saved.setups != setups {
        saved.setups = setups;
        saved.revision += 1;
    }
}

/// Wait for the listing under way and take it in.
#[cfg(test)]
pub(crate) fn wait() {
    if let Some(queue) = &saved().queue {
        queue.finish(std::time::Duration::from_secs(10));
    }
    take();
}

/// List the folder on the first frame, after `view.proofSetup.save` wrote a setup (from the dialog,
/// a control client or the command palette), and when the window becomes the active one, so a setup
/// copied into the folder is listed without a restart; take in the listings that ended. Once no
/// picker is open and its answer was carried out, the folder Save… made is removed if it is still empty.
fn list_saved(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // A pass applies only its own input: the focus event is in the frame's first pass, so a second
    // pass of the frame does not list again.
    let focused = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true))));
    let saves = app.session.proof_setup_saves();
    let start = {
        let mut saved = saved();
        // An answered pick waits for `file_asks::poll` to write into the folder.
        if !app.file_dialog_open()
            && !app.file_asks.asking()
            && let Some(dir) = saved.made.take()
        {
            // A folder that a setup was saved in is not empty, and stays.
            let _ = std::fs::remove_dir(dir);
        }
        let start = !saved.started || focused || saves != saved.saves;
        (saved.started, saved.saves) = (true, saves);
        start
    };
    if start {
        refresh_saved(app);
    }
    take();
}

/// Changes whenever the menus' listing changes.
pub fn saved_revision() -> u64 {
    saved().revision
}

/// View › Proof Setup's saved setups, each by its file's name, loading it with Proof Colors on.
pub fn saved_items() -> Vec<Node> {
    saved().setups.iter().map(|&(name, ref path)| crate::menus::named(name, LOAD, json!({ "path": path }))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::DefaultProofView;
    use craft::host::ScriptedFiles;
    use craft::ui::dialog::DialogHost;
    use std::sync::Arc;
    use vectorcraft_engine::Session;

    fn app() -> (VectorcraftApp, egui::Context) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({})).unwrap();
        let ctx = crate::testing::ui_context();
        app.ui_ctx = Some(ctx.clone());
        (app, ctx)
    }

    fn rgb() -> String {
        format!("rgb:{}", cms::DISPLAY_P3)
    }

    fn id(app: &VectorcraftApp) -> DialogId {
        app.ui.proof_setup.as_ref().map(|o| o.id).unwrap()
    }

    fn on_host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
        app.dialogs().unwrap().with(f)
    }

    fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
        super::super::host::pump(app, ctx);
    }

    /// Sets a field of the open dialog as automation does, then runs the frame.
    fn set(app: &mut VectorcraftApp, ctx: &egui::Context, field: &str, value: Value) {
        let id = id(app);
        on_host(app, |host| host.set_json_field(ctx, id, field, value)).unwrap();
        pump(app, ctx);
    }

    /// The open dialog's values.
    fn values(app: &VectorcraftApp) -> Value {
        let id = id(app);
        on_host(app, |host| host.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields)).unwrap()
    }

    /// OK: the held dialog files its answer, and the frame's pump runs `confirm` on it.
    fn confirm_dialog(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let id = id(app);
        on_host(app, |host| host.confirm_by_id(ctx, id));
        pump(app, ctx);
    }

    fn cancel_dialog(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let id = id(app);
        assert!(on_host(app, |host| host.cancel_by_id(ctx, id)));
        pump(app, ctx);
    }

    /// Presses a footer button of the open dialog and runs the frame.
    // PLAN: the press goes straight to `act`; the button-to-id step is the framework's `ParamDialog::extra_buttons` contract.
    fn press(app: &mut VectorcraftApp, ctx: &egui::Context, action: &str) {
        let id = id(app);
        let fields = on_host(app, |host| host.get_mut_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone())).unwrap();
        act(app, id, &fields, action);
        pump(app, ctx);
    }

    /// `files` as the app's file service, on its host and on the window's context, where the picks find it.
    fn scripted(app: &mut VectorcraftApp, files: ScriptedFiles) -> Arc<ScriptedFiles> {
        let files = Arc::new(files);
        crate::tests_picks::install_files(app, files.clone());
        files
    }

    fn rectangle(app: &mut VectorcraftApp) {
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    }

    /// View › Proof Setup › Customize… opens the dialog on the current proof setup; OK sets the
    /// device, Preserve Numbers, the intent and Simulate Black Ink, and turns Proof Colors on.
    #[test]
    fn customize_sets_the_proof_setup_and_turns_proof_colors_on() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let item = crate::menus::menu_entries(&app).into_iter().find(|e| e.path == ["View", "Proof Setup"] && e.label == "Customize…").unwrap();
        app.run(item.command.as_deref().unwrap(), item.params.clone()).unwrap();
        let v = values(&app);
        assert_eq!((&v["device"], &v["intent"], &v["preserveNumbers"]), (&json!(WORKING), &json!("relative"), &json!(false)));
        let device = format!("cmyk:{}", cms::DEVICE_CMYK);
        for (k, val) in [("device", json!(device)), ("intent", json!("perceptual")), ("simulateBlack", json!(true)), ("preview", json!(false))] {
            set(&mut app, &ctx, k, val);
        }
        confirm_dialog(&mut app, &ctx);
        assert!(app.ui.proof_setup.is_none());
        let v = proof::view();
        assert!(v.proof_colors);
        assert_eq!(v.setup.target, ProofTarget::Cmyk(cms::DEVICE_CMYK.into()));
        assert_eq!(v.setup.intent, Intent::Perceptual);
        assert!(v.setup.simulate_black && !v.setup.preserve_numbers && !v.setup.simulate_paper);
        // Opened again, it shows those settings, and the menu checks Customize….
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        let v = values(&app);
        assert_eq!((&v["device"], &v["intent"], &v["simulateBlack"]), (&json!(device), &json!("perceptual"), &json!(true)));
        assert_eq!(crate::menus::checked(&app, "ui.proofSetupDialog", &json!({})), Some(true));
        cancel_dialog(&mut app, &ctx);
    }

    /// Preview shows the dialog's settings on the canvas at once, follows their changes, and
    /// Cancel (or turning Preview off) puts the earlier proof view back.
    #[test]
    fn preview_shows_the_settings_and_cancel_puts_the_view_back() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        app.run("view.proofSetup", json!({"target": "protanopia"})).unwrap();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        assert_eq!(values(&app)["device"], json!(WORKING), "a color blindness proof starts on the working CMYK");
        set(&mut app, &ctx, "device", json!(rgb()));
        set(&mut app, &ctx, "preserveNumbers", json!(true));
        let v = proof::view();
        assert!(v.proof_colors, "the preview turns Proof Colors on");
        assert_eq!(v.setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()));
        assert!(v.setup.preserve_numbers);
        set(&mut app, &ctx, "preview", json!(false));
        assert_eq!(proof::view(), before, "Preview off puts the view back");
        set(&mut app, &ctx, "preview", json!(true));
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()));
        cancel_dialog(&mut app, &ctx);
        assert!(app.ui.proof_setup.is_none());
        assert_eq!(proof::view(), before, "Cancel puts the view back");
    }

    /// Cancel and Preview off put back a proof view whose device profile was removed after it was
    /// set, and redraw the documents.
    #[test]
    fn cancel_puts_back_a_view_whose_profile_was_removed() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let name = "Proof Setup Removed RGB";
        cms::register_icc(&cms::icc_bytes(cms::DISPLAY_P3).unwrap(), Some(name.into())).unwrap();
        app.run("view.proofSetup", json!({"target": format!("rgb:{name}"), "proof": true})).unwrap();
        app.run("color.removeProfile", json!({"name": name})).unwrap();
        let before = proof::view();
        assert_eq!(before.setup.target, ProofTarget::Rgb(name.into()), "removing the profile leaves it the device");
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        set(&mut app, &ctx, "device", json!(WORKING));
        assert_eq!(proof::view().setup.target, ProofTarget::WorkingCmyk, "the preview shows the working CMYK");
        set(&mut app, &ctx, "preview", json!(false));
        assert_eq!(proof::view(), before, "Preview off puts the view back");
        set(&mut app, &ctx, "preview", json!(true));
        let revision = app.session.doc().unwrap().revision;
        cancel_dialog(&mut app, &ctx);
        assert_eq!(proof::view(), before, "Cancel puts the view back");
        assert!(app.session.doc().unwrap().revision > revision, "the document redraws");
    }

    /// Run again while the dialog is open, Proof Setup keeps the dialog as it is, so Cancel still
    /// puts back the view it found the first time.
    #[test]
    fn running_proof_setup_again_keeps_the_open_dialog() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        set(&mut app, &ctx, "device", json!(rgb()));
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()));
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        assert_eq!(values(&app)["device"], json!(rgb()), "the open dialog stays as it is");
        cancel_dialog(&mut app, &ctx);
        assert_eq!(proof::view(), before, "Cancel puts back the view the first run found");
    }

    /// When another dialog (Saturate) opens over a previewing Proof Setup dialog, the proof setup
    /// the dialog found comes back, and Overprint Preview turned on during the preview stays on.
    #[test]
    fn another_dialog_in_its_place_puts_the_view_back() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        set(&mut app, &ctx, "device", json!(rgb()));
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()), "the preview shows");
        app.run("view.overprintPreview", json!({"on": true})).unwrap();
        rectangle(&mut app);
        app.run("ui.saturateDialog", json!({})).unwrap();
        pump(&mut app, &ctx);
        let v = proof::view();
        assert!(v.overprint, "Overprint Preview stays on");
        assert_eq!((v.setup, v.proof_colors), (before.setup.clone(), before.proof_colors), "the proof setup comes back");
        let proof_id = id(&app);
        assert!(!on_host(&app, |host| host.stack().is_top(proof_id)));
        // Closing Saturate brings the dialog back; the final comparison is on the whole view.
        app.run("view.overprintPreview", json!({"on": false})).unwrap();
        on_host(&app, |host| host.cancel_by_id(&ctx, host.stack().top().unwrap()));
        pump(&mut app, &ctx);
        cancel_dialog(&mut app, &ctx);
        assert_eq!(proof::view(), before);
    }

    /// Back from under a dialog opened over it (here Saturate, as a file-replacement question is),
    /// which undid the preview, the dialog previews its settings again, and Cancel puts back the
    /// view it found.
    #[test]
    fn back_from_under_another_dialog_it_previews_again() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        set(&mut app, &ctx, "device", json!(rgb()));
        rectangle(&mut app);
        app.run("ui.saturateDialog", json!({})).unwrap();
        pump(&mut app, &ctx);
        assert_eq!(proof::view(), before, "the preview is undone while the other dialog shows");
        on_host(&app, |host| host.cancel_by_id(&ctx, host.stack().top().unwrap()));
        pump(&mut app, &ctx);
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()), "the preview shows again");
        cancel_dialog(&mut app, &ctx);
        assert_eq!(proof::view(), before, "Cancel puts back the view it found");
    }

    /// A proof setup a command set while the dialog previews stays when Preview is turned off;
    /// Preview on shows the dialog's values again, and Cancel puts back the command's view.
    #[test]
    fn a_view_set_while_the_dialog_previews_stays() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        set(&mut app, &ctx, "device", json!(rgb()));
        app.run("view.proofSetup", json!({"target": "protanopia", "proof": true})).unwrap();
        let set_by_command = proof::view();
        set(&mut app, &ctx, "preview", json!(false));
        assert_eq!(proof::view(), set_by_command, "Preview off keeps the view the command set");
        set(&mut app, &ctx, "preview", json!(true));
        assert_eq!(proof::view().setup.target, ProofTarget::Rgb(cms::DISPLAY_P3.into()), "the preview shows again");
        cancel_dialog(&mut app, &ctx);
        assert_eq!(proof::view(), set_by_command, "Cancel puts back the view the command set");
    }

    /// A device the dialog can't use (a profile that is not loaded) is refused, and the dialog
    /// stays open with the proof view as it was.
    #[test]
    fn ok_with_an_unknown_device_keeps_the_dialog_open() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let name = "Proof Setup Gone RGB";
        cms::register_icc(&cms::icc_bytes(cms::DISPLAY_P3).unwrap(), Some(name.into())).unwrap();
        app.run("view.proofSetup", json!({"target": format!("rgb:{name}")})).unwrap();
        app.run("color.removeProfile", json!({"name": name})).unwrap();
        let before = proof::view();
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        set(&mut app, &ctx, "preview", json!(false));
        let dialog = id(&app);
        confirm_dialog(&mut app, &ctx);
        assert!(on_host(&app, |host| host.stack().contains(dialog)), "the dialog stays open");
        assert!(app.ui.proof_setup.is_some(), "the preview record stays");
        assert_eq!(proof::view(), before);
        cancel_dialog(&mut app, &ctx);
    }

    /// Save… writes the dialog's settings to the picked file in the Proof Setups folder, which View ›
    /// Proof Setup then lists; Load… puts a file's settings back in the dialog, and the menu item
    /// applies the setup with Proof Colors on.
    #[test]
    fn save_and_load_keep_a_custom_proof_setup() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let dir = vectorcraft_testkit::TempDir::new("proof-setup-dialog");
        let folder = dir.join("Proof Setups");
        let file = folder.join(format!("Screen.{EXT}"));
        let files = scripted(&mut app, ScriptedFiles::answering([Picked::Saved { path: file.clone(), file_type: None }]));
        app.session.attach_proof_setups(folder.clone());
        refresh_saved(&app);
        wait();
        assert!(saved_items().is_empty());
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        for (k, v) in [("device", json!(rgb())), ("intent", json!("saturation")), ("preserveNumbers", json!(true)), ("preview", json!(false))] {
            set(&mut app, &ctx, k, v);
        }
        press(&mut app, &ctx, SAVE);
        crate::file_asks::poll(&mut app);
        assert_eq!(files.requests()[0].directory.as_deref(), Some(folder.as_path()), "the picker starts in the Proof Setups folder");
        assert!(file.is_file(), "the picked file is saved");
        assert_eq!(proof::view(), Default::default(), "saving leaves the proof view alone");
        wait();
        let items = crate::menus::menu_entries(&app).into_iter().filter(|e| e.path == ["View", "Proof Setup"] && e.command.as_deref() == Some(LOAD));
        let items: Vec<_> = items.map(|e| (e.label.to_string(), e.params.clone())).collect();
        assert_eq!(items, [("Screen".to_string(), json!({ "path": file.to_string_lossy() }))]);
        // A control client that may not read the folder is not shown the saved setup.
        std::fs::create_dir_all(dir.join("out")).unwrap();
        let write_only = vectorcraft_engine::file_access::AutomationRoots::new(&[], &[&dir.join("out")]).unwrap();
        let listed = vectorcraft_engine::file_access::confine(write_only.as_ref(), || crate::menus::menu_entries(&app));
        assert!(!listed.iter().any(|e| e.command.as_deref() == Some(LOAD)), "a setup outside the read roots is left out");
        // A setup the command saves, outside the dialog, is listed at the next frame.
        app.run("view.proofSetup.save", json!({ "name": "Direct" })).unwrap();
        pump(&mut app, &ctx);
        wait();
        assert_eq!(saved_items().len(), 2, "the menu lists the new setup");
        std::fs::remove_file(folder.join(format!("Direct.{EXT}"))).unwrap();
        refresh_saved(&app);
        wait();
        // Load… puts the file's settings back in the dialog.
        // Preserve Numbers off first: the intent takes no input while it is on.
        for (k, v) in [("device", json!(WORKING)), ("preserveNumbers", json!(false)), ("intent", json!("perceptual"))] {
            set(&mut app, &ctx, k, v);
        }
        files.queue(Picked::Paths(vec![file.clone()]));
        press(&mut app, &ctx, LOAD_BUTTON);
        crate::file_asks::poll(&mut app);
        let v = values(&app);
        assert_eq!((&v["device"], &v["intent"], &v["preserveNumbers"]), (&json!(rgb()), &json!("saturation"), &json!(true)));
        assert_eq!(proof::view(), Default::default(), "loading into the dialog leaves the proof view alone");
        cancel_dialog(&mut app, &ctx);
        // The menu item applies the saved setup and turns Proof Colors on.
        app.run(LOAD, json!({ "path": file.to_string_lossy() })).unwrap();
        let v = proof::view();
        assert!(v.proof_colors && v.setup.preserve_numbers);
        assert_eq!((v.setup.target, v.setup.intent), (ProofTarget::Rgb(cms::DISPLAY_P3.into()), Intent::Saturation));
    }

    /// Save… whose picker is canceled saves nothing and leaves no Proof Setups folder behind; the
    /// folder stays while the picker is open and when a file is saved in it.
    #[test]
    fn a_canceled_save_leaves_no_folder() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let dir = vectorcraft_testkit::TempDir::new("proof-setup-dialog-cancel");
        let folder = dir.join("Proof Setups");
        let files = scripted(&mut app, ScriptedFiles::held());
        app.session.attach_proof_setups(folder.clone());
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        press(&mut app, &ctx, SAVE);
        assert!(app.file_dialog_open(), "the picker is open");
        assert!(folder.is_dir(), "the open picker's folder stays");
        files.answer(Picked::Cancelled);
        crate::file_asks::poll(&mut app);
        pump(&mut app, &ctx);
        assert!(!folder.exists(), "the canceled picker's folder is removed");
        // A picker that answers a file there saves it, and the folder stays.
        press(&mut app, &ctx, SAVE);
        let file = folder.join(format!("Kept.{EXT}"));
        files.answer(Picked::Saved { path: file.clone(), file_type: None });
        crate::file_asks::poll(&mut app);
        pump(&mut app, &ctx);
        assert!(file.is_file(), "the picked file is saved");
        assert!(folder.is_dir());
        cancel_dialog(&mut app, &ctx);
    }

    /// A saved setup whose name ends in an ellipsis still loads when its menu item is chosen.
    #[test]
    fn a_saved_setup_named_with_an_ellipsis_loads() {
        let p = json!({ "path": "Press….vcproof" });
        assert_eq!(crate::menus::click_target("Press…", LOAD, &p), (LOAD.to_string(), p.clone()));
    }

    /// Load…'s picker starts in the Proof Setups folder once it exists, and in its own default
    /// folder before; Load… does not make the folder.
    #[test]
    fn load_starts_in_the_folder_once_it_exists() {
        let _v = DefaultProofView::new();
        let (mut app, ctx) = app();
        let dir = vectorcraft_testkit::TempDir::new("proof-setup-dialog-load-folder");
        let folder = dir.join("Proof Setups");
        let files = scripted(&mut app, ScriptedFiles::new());
        app.session.attach_proof_setups(folder.clone());
        app.run("ui.proofSetupDialog", json!({})).unwrap();
        press(&mut app, &ctx, LOAD_BUTTON);
        assert!(!folder.exists(), "Load… does not make the folder");
        std::fs::create_dir_all(&folder).unwrap();
        press(&mut app, &ctx, LOAD_BUTTON);
        let started: Vec<_> = files.requests().into_iter().map(|r| r.directory).collect();
        assert_eq!(started, [None, Some(folder)]);
        cancel_dialog(&mut app, &ctx);
    }

    /// The device list holds the working CMYK space and every CMYK and RGB profile, each a target
    /// `view.proofSetup` accepts.
    #[test]
    fn devices_list_every_cmyk_and_rgb_profile() {
        let list = devices();
        assert_eq!(list[0].0, WORKING);
        for p in cms::profiles().iter().filter(|p| p.kind != ProfileKind::Gray) {
            assert!(list.iter().any(|(_, n)| *n == p.name), "{} is listed", p.name);
        }
        for (id, _) in &list {
            let t = ProofTarget::parse(id).unwrap();
            assert_eq!(&t.id(), id);
            assert!(rgb_device(id) == matches!(t, ProofTarget::Rgb(_)));
        }
    }
}
