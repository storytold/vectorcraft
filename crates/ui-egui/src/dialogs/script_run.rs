//! File › Scripts › Other Script…: choosing a JavaScript file and the warning before it runs
//! (`script.run` runs it), the question a run that keeps adding art asks, the question quitting
//! asks while a script runs, and the dialogs a script opens with `alert`, `confirm` and `prompt`.
//!
//! `ui.runScript {path? | source?, name?}` shows the warning for that script; without either it
//! asks for a `.js` or `.jsx` file through the app's file asks. With Preferences › General › Show
//! External Script Warning off, the chosen script runs at once. With General › Run Startup Scripts
//! on, the startup scripts run first (`script.startup.run`). The warning (kind `scriptRun`) is a
//! prompt: its `choice` field (`proceed` runs, `cancel` does not), `skip` (true: turn the warning
//! off) and `reaches`, which `ui.dialog.set` cannot set: the `kind`, `needs`, `outside`, `includes`
//! and `truncated` of `script.check`'s report on the script, read without running it (a path also
//! checks the files its `#include` lines name), or null when the check fails. The warning lists
//! what the report finds the script reaches outside the document (each of `script.check`'s
//! `outside` categories, such as files it writes, moves or deletes, files it opens in other apps,
//! the network, native code, environment variables and code the checker can't read), the files
//! and dialogs it needs, features only the reference app has, and its includes (at most
//! [`SHOWN_INCLUDES`], then the count of the rest). It says when the script is too large to check,
//! compiled or not a script, when the check failed, and when a list was cut short at its limit.
//! The listing is information: Run runs the script as before, after checking it again. When the
//! report has changed since the warning opened, nothing runs and the warning opens again with the
//! new report. The growth question
//! (`scriptGrowth`) and the question quitting asks (`scriptClose`) have the `choice` field alone.
//!
//! A script's dialog (kind `scriptDialog`) has the fields `kind` (alert, confirm or prompt),
//! `message`, `title` (null: Script Alert), `errorIcon`, `noAsDefault`, `value` (the prompt's
//! text) and `answer`, which `ui.dialog.set` takes: `ok` (alert, prompt), `yes` or `no` (confirm),
//! `cancel` (prompt) or `stop`. OK, Enter or `ui.dialog.confirm` without an `answer` gives the
//! default button's answer. Escape or `ui.dialog.cancel` gives Cancel for a prompt, No for a
//! confirm and Stop for an alert. Stop ends the run and puts back everything it did.
//!
//! A script's file dialog (kind `scriptFiles`: `File.openDialog`, `File.saveDialog`,
//! `Folder.selectDialog` and the objects' `openDlg`, `saveDlg` and `selectDlg`) has the fields
//! `kind` (open, save or folder), `prompt`, `canAlways`, `paths` (what Choose… picked), `leftOut` (Choose… picked
//! more files than a script is given) and `answer`. Only the person at the
//! keyboard answers Allow Once or Always Allow: `ui.dialog.set` refuses `once` and `always`, and a
//! click made by automation, or on paths that a pick automation opened, is refused and the dialog
//! opens again with the same paths; `deny` and `stop` are taken from automation too. The paths it
//! allows are the ones Choose… picked, at most 2,048; no field sets them. Escape, `ui.dialog.cancel`
//! and `ui.dialog.confirm` without an `answer` give Deny. Enter allows nothing: both Allow buttons
//! are not the dialog's default. `selectDlg` opens the host's folder picker in its own folder.

use std::path::Path;
use std::sync::Arc;

use craft::host::{PickRequest, Picked};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Outcome as _, Props as _, Scroll, Show as _, ShowContainer as _};
use craft::ui::dialog::{
    DialogHost, DialogId, DialogResult, DialogStack, FieldRefusal, HeldOk, Prompt, PromptAnswer, PromptChoice, StackDialog, StandardPrompt, dialog_id,
};
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::scripts::{RUN_EXTS, TOO_LARGE, runs};
use vectorcraft_engine::file_access::ScopeHandle;
use vectorcraft_engine::script::dialogs::{MAX_PROMPT_BYTES, cut_to, shown_text};
use vectorcraft_engine::script::files::{MAX_ENTRIES, PickAnswer, PickerKind, ScriptPicker};
use vectorcraft_engine::script::{DialogAnswer, MAX_SOURCE_BYTES, ScriptDialog};

use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};

/// The dialog kind of the warning before a script runs.
pub const KIND: &str = "scriptRun";
/// The dialog kind of the question a run asks when it has added a great deal.
pub const GROWTH_KIND: &str = "scriptGrowth";
/// The dialog kind of the question quitting asks while a script runs.
pub const CLOSE_KIND: &str = "scriptClose";
/// The dialog kind of a script's `alert`, `confirm` and `prompt`.
pub const NOTICE_KIND: &str = "scriptDialog";
/// The dialog kind of a script's file dialogs.
pub const PICKER_KIND: &str = "scriptFiles";

/// Why Allow Once and Always Allow are refused to automation.
pub(crate) const BY_PERSON: &str = "Only the person at the keyboard can allow a script's file access";

/// The note the file dialog shows when the person picked more files than a script is given.
const PICKED_LEFT_OUT: &str = "A script is given at most 2,048 files: the files picked after them were left out";
/// The height of the Script File Access prompt's list of picked files before it scrolls, in points.
const PICKED_LIST_HEIGHT: f32 = 120.0;

/// The warning before a script runs. One type per question, so the host's per-type results never
/// mix the three questions of this area.
pub(crate) struct RunQuestion(pub(crate) StandardPrompt);

/// The prompt's own dialog, with `reaches` added to its fields: what the warning lists.
impl StackDialog for RunQuestion {
    type Result = PromptAnswer;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn instance(&self) -> Option<DialogId> {
        StackDialog::instance(&self.0)
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<PromptAnswer>> {
        StackDialog::show(&mut self.0, ui, stack)
    }

    fn confirm(&mut self, ctx: &egui::Context) -> Option<PromptAnswer> {
        self.0.confirm(ctx)
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.0.held_state()
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = self.0.json_fields()?;
        fields["reaches"] = self.0.tag_as::<RunWarning>().map_or(Value::Null, |w| w.reaches.clone());
        Some(fields)
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        self.0.set_json_field(ctx, field, value)
    }

    fn cancelled(&mut self, ctx: &egui::Context) {
        self.0.cancelled(ctx);
    }

    fn preview_color(&mut self, key: &str, color: Option<craft::theme::Srgba>) {
        self.0.preview_color(key, color);
    }
}

/// The question a run asks when it has added a great deal.
pub(crate) struct GrowthQuestion(pub(crate) StandardPrompt);
crate::dialogs::confirm::wrapped_dialog!(GrowthQuestion, StandardPrompt, GROWTH_KIND);

/// The question quitting asks while a script runs.
pub(crate) struct CloseQuestion(pub(crate) StandardPrompt);
crate::dialogs::confirm::wrapped_dialog!(CloseQuestion, StandardPrompt, CLOSE_KIND);

/// The tag of the warning: the params `script.run` takes once the warning is answered, and the
/// parts of `script.check`'s report the warning lists ([`reaches`]).
struct RunWarning {
    params: Value,
    reaches: Value,
    /// The automation roots in force when asked: the script runs under them, and under any in force
    /// when the warning is answered.
    scope: ScopeHandle,
    /// Automation asked ([`VectorcraftApp::automation_asked`]): the script runs as automation.
    automated: bool,
}

/// What a script's `alert`, `confirm` or `prompt` dialog answers with.
pub(crate) struct ScriptAnswer {
    /// The dialog's kind: alert, confirm or prompt.
    pub(crate) kind: String,
    /// The button's answer: `ok`, `yes`, `no`, `cancel` or `stop`.
    pub(crate) answer: &'static str,
    /// The text of a prompt.
    pub(crate) value: String,
}

/// What a script's file dialog answers with.
pub(crate) struct FilesAnswer {
    /// `once`, `always`, `deny` or `stop`.
    pub(crate) answer: &'static str,
    /// The paths Choose… picked.
    pub(crate) paths: Vec<String>,
    /// Choose… left out files past the most a script is given.
    pub(crate) left_out: bool,
    /// The paths come from a pick that automation asked for.
    pub(crate) automated: bool,
}

/// The answers a script's dialog of `kind` takes, as the refusal of a wrong one lists them.
fn answers_for(kind: &str) -> &'static str {
    match kind {
        "confirm" => "yes, no, stop",
        "prompt" => "ok, cancel, stop",
        _ => "ok, stop",
    }
}

fn answer_label(answer: &str) -> &'static str {
    match answer {
        "yes" => tl!("Yes"),
        "no" => tl!("No"),
        "cancel" => tl!("Cancel"),
        "stop" => tl!("Stop"),
        _ => tl!("OK"),
    }
}

/// The dialog of a script's `alert`, `confirm` or `prompt` call.
pub(crate) struct ScriptNotice {
    kind: String,
    message: String,
    title: Option<String>,
    error_icon: bool,
    no_as_default: bool,
    value: String,
    answer: Option<&'static str>,
}

impl ScriptNotice {
    fn new(d: &ScriptDialog) -> Self {
        Self {
            kind: d.kind.id().to_string(),
            message: d.message.clone(),
            title: d.title.clone(),
            error_icon: d.error_icon,
            no_as_default: d.no_as_default,
            // `preset` serialises as the dialog's text, or null for none.
            value: serde_json::from_value(json!(d.preset)).unwrap_or_default(),
            answer: None,
        }
    }

    /// The dialog's heading: the script's title on one line, or Script Alert.
    pub(crate) fn heading(&self) -> String {
        match &self.title {
            Some(title) => shown_text(title, false),
            None => tl!("Script Alert").into(),
        }
    }

    /// The answer of the default button: OK, or for a confirm Yes, or No with `noAsDefault`.
    fn default_answer(&self) -> &'static str {
        match (self.kind.as_str(), self.no_as_default) {
            ("confirm", true) => "no",
            ("confirm", false) => "yes",
            _ => "ok",
        }
    }

    /// The buttons in the order the dialog answers them, with what each answers.
    fn choices(&self) -> Vec<(&'static str, FooterButton)> {
        let default = self.default_answer();
        let own: &[&'static str] = match self.kind.as_str() {
            "confirm" => &["yes", "no"],
            "prompt" => &["ok", "cancel"],
            _ => &["ok"],
        };
        let role = |answer: &str| match answer {
            _ if answer == default => DialogRole::Accept,
            "no" | "cancel" => DialogRole::Cancel,
            _ => DialogRole::Other,
        };
        own.iter().chain(&["stop"]).map(|answer| (*answer, FooterButton::new(answer_label(answer), role(answer)))).collect()
    }
}

impl StackDialog for ScriptNotice {
    type Result = ScriptAnswer;

    fn id(&self) -> DialogId {
        dialog_id(NOTICE_KIND)
    }

    fn key(&self) -> Option<&str> {
        Some(NOTICE_KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ScriptAnswer>> {
        let (answers, buttons): (Vec<&'static str>, Vec<FooterButton>) = self.choices().into_iter().unzip();
        let message = shown_text(&self.message, true);
        let mut prompt = Prompt::new(self.heading(), message.clone()).answers(buttons);
        if self.error_icon {
            prompt = prompt.tone(Tone::Bad, tl!("The script reports an error."));
        }
        // Escape stays No when No is the default button.
        if self.no_as_default && self.kind == "confirm" {
            prompt = prompt.escape(DialogRole::Accept);
        }
        if self.kind == "prompt" {
            let name = cut_to(&shown_text(&message, false), 200).to_string();
            let value = &mut self.value;
            prompt = prompt.extras(move |ui| {
                TextField::new(name.clone(), Bind::bound(&mut *value)).show(ui);
            });
        }
        let picked = prompt.show(ui).picked;
        if self.value.len() > MAX_PROMPT_BYTES {
            self.value = cut_to(&self.value, MAX_PROMPT_BYTES).to_string();
        }
        let answer = *answers.get(picked?)?;
        Some(DialogResult::Ok(ScriptAnswer { kind: self.kind.clone(), answer, value: self.value.clone() }))
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<ScriptAnswer> {
        let answer = self.answer.unwrap_or_else(|| self.default_answer());
        Some(ScriptAnswer { kind: self.kind.clone(), answer, value: self.value.clone() })
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = json!({
            "kind": self.kind,
            "message": self.message,
            "title": self.title,
            "errorIcon": self.error_icon,
            "noAsDefault": self.no_as_default,
            "answer": self.answer,
        });
        if self.kind == "prompt" {
            fields["value"] = json!(self.value);
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "value" if self.kind == "prompt" => {
                let text = value.as_str().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
                if text.len() > MAX_PROMPT_BYTES {
                    return Err(format!("the prompt's text is longer than {MAX_PROMPT_BYTES} bytes"));
                }
                self.value = text.to_string();
            }
            "answer" => {
                let wanted = value.as_str().unwrap_or_default();
                let Some(answer) = answers_for(&self.kind).split(", ").find(|answer| *answer == wanted) else {
                    return Err(format!("`answer` for this {} dialog must be one of: {}", self.kind, answers_for(&self.kind)));
                };
                self.answer = Some(answer);
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// The dialog of a script's file dialog call: what the run asked for, read from the run's own
/// request when the dialog opens, so no field of the dialog changes what the person reads.
pub(crate) struct ScriptFiles {
    opening: Arc<()>,
    kind: PickerKind,
    prompt: String,
    can_always: bool,
    paths: Vec<String>,
    left_out: bool,
    automated: bool,
    /// Choose… was pressed; [`pump`] opens the host's file dialog.
    pub(crate) choose: bool,
    answer: Option<&'static str>,
}

impl ScriptFiles {
    fn new(p: ScriptPicker) -> Self {
        Self {
            opening: Arc::new(()),
            kind: p.kind,
            prompt: p.prompt,
            can_always: p.can_always,
            paths: Vec::new(),
            left_out: false,
            automated: false,
            choose: false,
            answer: None,
        }
    }

    /// Keeps what a pick chose: in place of the paths before, or after them when the script asked
    /// for several files. At most [`MAX_ENTRIES`] are kept, the number of files a script is given.
    /// A pick that chose nothing changes nothing.
    pub(crate) fn add(&mut self, chosen: Vec<String>, several: bool, automated: bool) {
        if chosen.is_empty() {
            return;
        }
        self.automated = automated;
        let mut paths = if several { std::mem::take(&mut self.paths) } else { Vec::new() };
        let mut seen: std::collections::HashSet<String> = paths.iter().cloned().collect();
        for path in chosen {
            if seen.contains(&path) {
                continue;
            }
            if paths.len() >= MAX_ENTRIES {
                self.left_out = true;
                break;
            }
            seen.insert(path.clone());
            paths.push(path);
        }
        self.paths = paths;
    }

    fn answered(&self, answer: &'static str) -> FilesAnswer {
        FilesAnswer { answer, paths: self.paths.clone(), left_out: self.left_out, automated: self.automated }
    }
}

impl StackDialog for ScriptFiles {
    type Result = FilesAnswer;

    fn id(&self) -> DialogId {
        dialog_id(PICKER_KIND)
    }

    fn key(&self) -> Option<&str> {
        Some(PICKER_KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<FilesAnswer>> {
        // Neither Allow button is the default: Enter never allows file access.
        let allowed = !self.paths.is_empty();
        let mut answers = vec![("once", FooterButton::new(tl!("Allow Once"), DialogRole::Other).enabled(allowed))];
        if self.can_always {
            answers.push(("always", FooterButton::new(tl!("Always Allow This Script"), DialogRole::Other).enabled(allowed)));
        }
        answers.push(("deny", FooterButton::new(tl!("Deny"), DialogRole::Cancel)));
        answers.push(("stop", FooterButton::new(tl!("Stop"), DialogRole::Other)));
        let (names, buttons): (Vec<&'static str>, Vec<FooterButton>) = answers.into_iter().unzip();
        let mut prompt =
            Prompt::new(tl!("Script File Access"), shown_text(&self.prompt, true)).paragraph(picker_request(&self.kind)).answers(buttons);
        if self.left_out {
            prompt = prompt.tone(Tone::Bad, crate::i18n::msg(PICKED_LEFT_OUT).to_string());
        }
        let (choose, paths) = (&mut self.choose, &self.paths);
        prompt = prompt.extras(move |ui| {
            if Button::new(tl!("Choose…")).show(ui).response().clicked() {
                *choose = true;
            }
            let rows = |ui: &mut Ui| paths.iter().for_each(|path| drop(Text::new(shown_text(path, false)).show(ui)));
            if paths.len() > 1 {
                Stack::new()
                    .label(tl!("Picked files"))
                    .scroll(Scroll::vertical())
                    .room_height(PICKED_LIST_HEIGHT)
                    .id_salt("script-picked")
                    .show(ui, |ui, _| rows(ui));
            } else {
                rows(ui);
            }
        });
        let picked = prompt.show(ui).picked;
        let answer = *names.get(picked?)?;
        Some(DialogResult::Ok(self.answered(answer)))
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<FilesAnswer> {
        Some(self.answered(self.answer.unwrap_or("deny")))
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({
            "kind": self.kind.id(),
            "prompt": self.prompt,
            "canAlways": self.can_always,
            "paths": self.paths,
            "leftOut": self.left_out,
            "answer": self.answer,
        }))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field != "answer" {
            return Err(FieldRefusal::Unknown.message(field));
        }
        self.answer = Some(match value.as_str() {
            Some("deny") => "deny",
            Some("stop") => "stop",
            Some("once" | "always") => return Err(BY_PERSON.into()),
            _ => return Err("`answer` for a script's file dialog must be one of: once, always, deny, stop".into()),
        });
        Ok(())
    }
}

/// What the file dialog of `kind` asks the person to allow.
fn picker_request(kind: &PickerKind) -> &'static str {
    match kind {
        PickerKind::Open => tl!("The script asks to read a file you choose."),
        PickerKind::Save => tl!("The script asks to read and write a file you choose."),
        PickerKind::Folder => tl!("The script asks to read and write a folder you choose, with everything in it."),
    }
}

/// Runs `f` on the window's dialog host; `None` without a window, where nothing can be asked.
pub(crate) fn with_host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost, &egui::Context) -> R) -> Option<R> {
    let ctx = app.ui_ctx.clone()?;
    Some(app.dialogs()?.with(|host| f(host, &ctx)))
}

/// Whether the question asked on quitting with a script running is open.
pub(crate) fn asking_to_close(app: &VectorcraftApp) -> bool {
    with_host(app, |host, _| host.is_open::<CloseQuestion>()).unwrap_or(false)
}

/// Whether a dialog is open or waits to open: a script's dialog opens only over nothing.
pub(crate) fn busy(app: &VectorcraftApp) -> bool {
    app.any_dialog_open() || with_host(app, |host, _| !host.is_empty()).unwrap_or(false)
}

/// Closes the open `D` the run no longer waits at, and drops its result: only answers the person
/// or automation gave reach [`pump`]. A quit question asked over it stays, now over nothing, so
/// its Cancel does not show the dialog again.
pub(crate) fn forget<D: StackDialog>(app: &VectorcraftApp) {
    with_host(app, |host, ctx| {
        if !host.is_open::<D>() {
            return;
        }
        let asking = host.is_open::<CloseQuestion>();
        host.cancel::<D>(ctx);
        let _ = host.take::<D>();
        if asking {
            let _ = host.take::<CloseQuestion>();
            host.open(close_question());
        }
    });
}

/// `ui.runScript`: the script at `path` or given as `source`; without either, pick a file.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if p.get("path").is_some() || p.get("source").is_some() {
        return chosen(app, p.clone());
    }
    let request = PickRequest::open("script").filter("Scripts", RUN_EXTS);
    // PLAN: Owner::Free is not in files-w5's DOCUMENT_FREE list yet; a script's pick has no document of its own.
    Ok(file_asks::ask(app, request, Owner::Free, picked_script))
}

/// The script a pick delivered: a path on the desktop, or the file's bytes in the browser.
pub(crate) fn picked_script(app: &mut VectorcraftApp, picked: Picked) -> Result<Value, String> {
    if let Picked::Contents(files) = picked {
        let Some((name, bytes)) = files.into_iter().next() else { return Ok(Value::Null) };
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(TOO_LARGE.to_string());
        }
        let source = String::from_utf8(bytes).map_err(|_| "the script is not UTF-8 text".to_string())?;
        return chosen(app, json!({"source": source, "name": name}));
    }
    match picked.paths().into_iter().next() {
        Some(path) => chosen(app, json!({ "path": path.to_string_lossy() })),
        None => Ok(Value::Null),
    }
}

/// A script was chosen: `params` are `script.run`'s. It runs after the warning, when that is on.
pub(crate) fn chosen(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let path = params.get("path").and_then(Value::as_str);
    if path.is_some_and(|path| !runs(path)) {
        return Err("Choose a .js or .jsx script".into());
    }
    if !app.session.prefs.show_external_script_warning {
        return crate::script_runs::run_chosen(app, params);
    }
    let reaches = reaches(app, &params);
    let automated = app.automation_asked();
    warn(app, params, reaches, ScopeHandle::capture(), automated)
}

/// Opens the warning for the script `params` name, listing `reaches`; the script runs under
/// `scope`, and as automation when `automated` ([`RunWarning`]).
fn warn(app: &mut VectorcraftApp, params: Value, reaches: Value, scope: ScopeHandle, automated: bool) -> Result<Value, String> {
    let path = params.get("path").and_then(Value::as_str);
    let name = params.get("name").and_then(Value::as_str).or(path).map_or_else(|| "script".to_string(), vectorcraft_engine::cmd::fileio::file_name);
    // The script's name is the message: it is what the person reads first.
    let prompt = StandardPrompt::custom(name)
        .title(tl!("Run Script"))
        .paragraph(tl!("A script can change the open documents and can write files through the commands it runs. Run only scripts you trust."));
    let prompt = listed(prompt, &reaches)
        .dont_show_again()
        .button(PromptChoice::Proceed, tl!("Run"))
        .button(PromptChoice::Cancel, tl!("Cancel"))
        .tag(RunWarning { params, reaches, scope, automated });
    with_host(app, |host, _| host.open(RunQuestion(prompt))).ok_or_else(|| "there is no window to ask in".to_string())?;
    Ok(Value::Null)
}

/// The status when Run finds that the script's report changed since the warning opened: the
/// warning opens again with the new report, and nothing runs.
pub(crate) const CHANGED_SINCE_WARNING: &str = "The script changed after its warning opened: check the warning again";

/// The most includes the warning lists; a line after them gives the count of the rest.
pub(crate) const SHOWN_INCLUDES: usize = 20;

/// The parts of `script.check`'s report on the script `script.run`'s `params` name that the
/// warning lists, read without running the script; a path also checks the files its `#include`
/// lines name. Null when the check fails: the run reports why the script can't be read.
fn reaches(app: &mut VectorcraftApp, params: &Value) -> Value {
    checked(app, params).unwrap_or(Value::Null)
}

/// [`reaches`], with the check's error when it fails, for example when the request asking can't
/// read the script.
fn checked(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let check = match (params.get("path"), params.get("source")) {
        (Some(path), _) => json!({"path": path, "includes": true}),
        (None, Some(source)) => json!({"data": source}),
        (None, None) => return Ok(Value::Null),
    };
    let r = app.session.execute("script.check", &check).map_err(|e| e.to_string())?;
    Ok(json!({"kind": r["kind"], "needs": r["needs"], "outside": r["outside"], "includes": r["includes"], "truncated": r["truncated"]}))
}

/// `prompt` with what the report `r` finds the script reaches outside the document, one line each;
/// nothing more for a script that was checked and reaches nothing. A failed check (null) says the
/// script was not read.
fn listed(prompt: StandardPrompt, r: &Value) -> StandardPrompt {
    use super::script_check::{list, need_label, outside_label, place, status_label, tally};
    let unchecked = match r["kind"].as_str() {
        Some("script") => None,
        Some("compiled") => Some(tl!("Compiled script: ask its author for the .jsx file")),
        Some("tooLarge") => Some(tl!("Too large to check")),
        _ if r.is_null() => Some(tl!("Not read")),
        _ => Some(tl!("Not a script for the reference app")),
    };
    if let Some(text) = unchecked {
        return prompt.note(text);
    }
    // A row: its text, then its count, lines or status when it has them.
    let row = |text: &str, note: &str| if note.is_empty() { text.to_string() } else { format!("{text} · {note}") };
    let outside: Vec<String> = list(r, "outside")
        .iter()
        .filter_map(|o| {
            let label = o["id"].as_str().and_then(outside_label)?;
            let modes: Vec<&str> = list(o, "modes").iter().filter_map(Value::as_str).collect();
            let label = if modes.is_empty() { label.to_string() } else { format!("{label} ({})", modes.join(", ")) };
            Some(row(&label, &tally(o)))
        })
        .collect();
    let needs = list(r, "needs").iter().filter_map(Value::as_str).filter(|n| matches!(*n, "files" | "dialogs" | "hostOnly")).filter_map(need_label);
    // The needs come first, each a sentence of its own; the heading names only what the script does
    // outside the document.
    let mut lines: Vec<String> = needs.map(str::to_string).collect();
    if !outside.is_empty() {
        lines.push(crate::i18n::colon(tl!("Outside the document")));
        lines.extend(outside);
    }
    let includes = list(r, "includes");
    if !includes.is_empty() {
        lines.push(crate::i18n::colon(tl!("Includes")));
        for i in includes.iter().take(SHOWN_INCLUDES) {
            let status = i["status"].as_str().and_then(status_label).unwrap_or_default();
            let note = [status.to_string(), place(i)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            lines.push(row(i["value"].as_str().unwrap_or_default(), &note));
        }
        if includes.len() > SHOWN_INCLUDES {
            let count = (includes.len() - SHOWN_INCLUDES).to_string();
            lines.push(crate::i18n::fmt(tl!("Included files not listed: {count}"), &[("count", &count)]));
        }
    }
    if r["truncated"].as_bool() == Some(true) {
        lines.push(tl!("Some lists were cut short at their limit.").to_string());
    }
    if lines.is_empty() {
        return prompt;
    }
    prompt.paragraph(tl!("VectorCraft read this script without running it.")).lines(lines.into_iter().map(Into::into))
}

fn growth_question() -> GrowthQuestion {
    GrowthQuestion(
        StandardPrompt::custom(tl!("This script has added a great many objects. Continue, or stop it and undo what it did?"))
            .title(tl!("A Script Is Running"))
            .button(PromptChoice::Proceed, tl!("Continue"))
            .button(PromptChoice::Cancel, tl!("Stop")),
    )
}

fn close_question() -> CloseQuestion {
    CloseQuestion(
        StandardPrompt::custom(tl!("Stop the script and undo what it did? Then quit again."))
            .title(tl!("A Script Is Running"))
            .button(PromptChoice::Proceed, tl!("Stop"))
            .button(PromptChoice::Cancel, tl!("Cancel")),
    )
}

/// Asks whether to continue a run that has added a great deal; Stop is its Cancel button.
pub(crate) fn open_growth(app: &VectorcraftApp) {
    with_host(app, |host, _| host.open(growth_question()));
}

/// Shows the dialog the running script waits at.
pub(crate) fn open_notice(app: &VectorcraftApp, d: &ScriptDialog) {
    with_host(app, |host, _| host.open(ScriptNotice::new(d)));
}

/// Shows the file dialog the running script waits at.
pub(crate) fn open_picker(app: &VectorcraftApp, p: ScriptPicker) {
    with_host(app, |host, _| host.open(ScriptFiles::new(p)));
}

/// Quitting while a script runs asks whether to stop it, also over the script's own dialog, which
/// Cancel shows again as it was. Any other open dialog stays, and quitting asks nothing yet.
pub(crate) fn ask_to_close(app: &mut VectorcraftApp) {
    with_host(app, |host, _| {
        let own = host.get::<ScriptNotice>().map(StackDialog::id).or_else(|| host.get::<ScriptFiles>().map(StackDialog::id));
        match own {
            Some(parent) if host.stack().is_top(parent) => {
                host.open_child(close_question(), parent);
            }
            None if host.is_empty() => {
                host.open(close_question());
            }
            _ => {}
        }
    });
}

/// Closes the question quitting asks once the run it asks about has ended.
pub(crate) fn settle_close(app: &mut VectorcraftApp) {
    if !app.session.script_running() {
        with_host(app, |host, ctx| {
            if host.cancel::<CloseQuestion>(ctx) {
                let _ = host.take::<CloseQuestion>();
            }
        });
    }
}

/// Takes the answers of the script's dialogs, once per frame, and gives them to the run.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some((notice, files, run, growth, close)) = with_host(app, |host, _| {
        (
            host.take::<ScriptNotice>(),
            host.take::<ScriptFiles>(),
            host.take::<RunQuestion>(),
            host.take::<GrowthQuestion>(),
            host.take::<CloseQuestion>(),
        )
    }) else {
        return;
    };
    if let Some(result) = notice {
        settle_notice(app, result);
    }
    if let Some(result) = files {
        settle_files(app, result);
    }
    if let Some(DialogResult::Ok(mut answer)) = run
        && answer.choice == PromptChoice::Proceed
        && let Some(RunWarning { params, reaches: listed, scope, automated }) = answer.take_tag::<RunWarning>()
    {
        // A script whose report changed while the warning was open is listed again before it runs.
        // A script read when the warning opened that the answering request can't read is
        // refused with the check's error, not listed again as changed.
        // The check reads the script under the access the run would have.
        let ran = match scope.clone().within_current().run(|| checked(app, &params)) {
            Err(e) if !listed.is_null() => Err(e),
            now => Ok(now.unwrap_or(Value::Null)),
        }
        .and_then(|now| if now == listed {
            let stored =
                if answer.skip { app.run("prefs.set", json!({"key": "showExternalScriptWarning", "value": false})).map(drop) } else { Ok(()) };
            let run = |app: &mut VectorcraftApp| crate::script_runs::run_chosen(app, params).map(drop);
            let automated = automated || app.automation_asked();
            stored.and_then(|()| scope.within_current().run(|| file_asks::as_automated(app, automated, run)))
        } else {
            app.status(CHANGED_SINCE_WARNING);
            warn(app, params, now, scope, automated).map(drop)
        });
        if let Err(e) = ran {
            app.status(e);
        }
    }
    match growth {
        Some(DialogResult::Ok(answer)) if answer.choice == PromptChoice::Proceed => {
            app.session.script_continue();
        }
        // Stop is the Cancel button, and Escape.
        Some(_) => {
            app.session.script_request_stop();
        }
        None => {}
    }
    answer_close(app, close);
    if with_host(app, |host, _| host.get_mut::<ScriptFiles>().is_some_and(|d| std::mem::take(&mut d.choose))).unwrap_or(false) {
        choose(app);
    }
}

/// Takes the answer to the quit question given between two queued runs, before the next run
/// starts. True when it stopped the runs.
pub(crate) fn answer_close_between_runs(app: &mut VectorcraftApp) -> bool {
    let Some(close) = with_host(app, |host, _| host.take::<CloseQuestion>()).flatten() else {
        return false;
    };
    answer_close(app, Some(close));
    !app.session.script_running()
}

/// With Stop as the answer to the quit question, stops the run, or between two queued runs, the
/// runs not yet started.
fn answer_close(app: &mut VectorcraftApp, close: Option<DialogResult<<CloseQuestion as StackDialog>::Result>>) {
    let startup = app.session.startup_script_running();
    // Between two queued runs: none runs, and the report is the last run's.
    let between = app.session.script_running() && app.session.script_name().is_none();
    if let Some(DialogResult::Ok(answer)) = close
        && answer.choice == PromptChoice::Proceed
        && app.session.script_abandon()
    {
        if between {
            crate::script_runs::stopped_between_runs(app);
        } else {
            let error = app.session.script_failure_text().unwrap_or_default();
            crate::script_runs::ended_with_error(app, startup, error);
        }
        forget::<ScriptNotice>(app);
        forget::<ScriptFiles>(app);
        app.sync_views();
    }
}

/// Gives the running script the answer of its dialog. Cancel is Cancel for a prompt, No for a
/// confirm and Stop for an alert.
pub(crate) fn settle_notice(app: &mut VectorcraftApp, result: DialogResult<ScriptAnswer>) {
    let answer = match result.ok() {
        Some(answer) => answer,
        None => {
            let Some(d) = app.session.script_dialog() else { return };
            let kind = d.kind.id().to_string();
            let answer = match kind.as_str() {
                "prompt" => "cancel",
                "confirm" => "no",
                _ => "stop",
            };
            ScriptAnswer { kind, answer, value: String::new() }
        }
    };
    if let Err(e) = give(app, &answer) {
        app.status(e);
        if let Some(d) = app.session.script_dialog() {
            open_notice(app, &d);
        }
    }
}

fn give(app: &mut VectorcraftApp, a: &ScriptAnswer) -> Result<(), String> {
    let given = match (a.kind.as_str(), a.answer) {
        (_, "stop") => None,
        ("alert", "ok") => Some(DialogAnswer::Ok),
        ("confirm", "yes") => Some(DialogAnswer::Confirm(true)),
        ("confirm", "no") => Some(DialogAnswer::Confirm(false)),
        ("prompt", "ok") => Some(DialogAnswer::Prompt(Some(a.value.clone()))),
        ("prompt", "cancel") => Some(DialogAnswer::Prompt(None)),
        (kind, _) => return Err(format!("`answer` for this {kind} dialog must be one of: {}", answers_for(kind))),
    };
    match given {
        None => {
            app.session.script_request_stop();
        }
        Some(given) => {
            // Refused while the dialog still waits: the text is too long.
            if !app.session.script_answer(given) && app.session.script_dialog().is_some() {
                return Err(format!("the prompt's text is longer than {MAX_PROMPT_BYTES} bytes"));
            }
        }
    }
    Ok(())
}

/// Gives the running script the answer of its file dialog. Only the person at the keyboard allows
/// access: Allow Once and Always Allow from a pass automation drove, or on paths a pick that
/// automation asked for chose, are refused and the dialog opens again with the same paths; Deny
/// and Stop are taken from anyone.
pub(crate) fn settle_files(app: &mut VectorcraftApp, result: DialogResult<FilesAnswer>) {
    let answer = result.ok().unwrap_or_else(|| FilesAnswer { answer: "deny", paths: Vec::new(), left_out: false, automated: false });
    if matches!(answer.answer, "once" | "always") && (answer.automated || app.automation_is_acting()) {
        app.status(BY_PERSON);
        if let Some(picker) = app.session.script_picker() {
            let mut dialog = ScriptFiles::new(picker);
            (dialog.paths, dialog.left_out, dialog.automated) = (answer.paths, answer.left_out, answer.automated);
            with_host(app, |host, _| host.open(dialog));
        }
        return;
    }
    let can_always = app.session.script_picker().is_some_and(|p| p.can_always);
    let given = match answer.answer {
        "stop" => None,
        "once" if !answer.paths.is_empty() => Some(PickAnswer::Allow { paths: answer.paths, always: false }),
        "always" if can_always && !answer.paths.is_empty() => Some(PickAnswer::Allow { paths: answer.paths, always: true }),
        // Deny, and any allowance the dialog's buttons cannot give (no paths, always for a run that cannot keep it).
        _ => Some(PickAnswer::Deny),
    };
    match given {
        None => {
            app.session.script_request_stop();
        }
        Some(given) => {
            app.session.script_pick(given);
        }
    }
}

/// Asks for the files of the kind the running script asked for, starting at the script's file or
/// folder, with the script's file types; the answer goes to the dialog that opened the picker.
fn choose(app: &mut VectorcraftApp) {
    let Some(opening) = with_host(app, |host, _| host.get::<ScriptFiles>().map(|dialog| Arc::clone(&dialog.opening))).flatten() else {
        return;
    };
    let Some(p) = app.session.script_picker() else { return };
    let several = p.multiple && p.kind == PickerKind::Open;
    let mut request = match p.kind {
        PickerKind::Open if several => PickRequest::open_many("script"),
        PickerKind::Open => PickRequest::open("script"),
        PickerKind::Save => PickRequest::save("script"),
        PickerKind::Folder => PickRequest::folder("script"),
    };
    for (name, extensions) in &p.filters {
        let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
        request = request.filter(name.clone(), &extensions);
    }
    let start = p.start.as_deref().map(Path::new);
    match (&p.kind, start) {
        // `selectDlg` starts in its folder; `Folder.selectDialog` gives none.
        (PickerKind::Folder, Some(folder)) => request = request.directory(folder),
        (PickerKind::Folder, None) => {}
        (_, start) => {
            if let Some(name) = start.and_then(Path::file_name) {
                request = request.file_name(name.to_string_lossy().into_owned());
            }
            if let Some(folder) = start.and_then(Path::parent).filter(|folder| !folder.as_os_str().is_empty()) {
                request = request.directory(folder);
            }
        }
    }
    file_asks::ask(app, request, Owner::Free, move |app, picked| {
        let chosen = picked.paths().iter().map(|path| path.to_string_lossy().into_owned()).collect();
        let automated = app.file_asks.automated();
        with_host(app, |host, _| {
            if let Some(dialog) = host.get_mut::<ScriptFiles>().filter(|dialog| Arc::ptr_eq(&dialog.opening, &opening)) {
                dialog.add(chosen, several, automated);
            }
        });
        Ok(Value::Null)
    });
}
