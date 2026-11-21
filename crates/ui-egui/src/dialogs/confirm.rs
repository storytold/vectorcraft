//! Questions and messages on the dialog host: a question answered with OK or Cancel, where OK runs
//! a command ([`ask`]); a message with a Close button ([`tell`]); the question whether a save or
//! export may replace files ([`ask_replace_files`]); the Type tool's missing-fonts question
//! ([`ask_missing_font_edit`]). Both texts are shown as given: callers translate their template
//! before filling in a name, so the name is never translated.

use craft::ui::dialog::{Alert, DialogId, DialogResult, DialogStack, HeldOk, HostedAlert, PromptChoice, StackDialog, StandardPrompt, StartupStage, dialog_id};
use egui::{Context, Ui};
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// The key of a confirmation, and what the reply of a command that opened one says is pending.
pub const KIND: &str = "confirm";

/// The key of a message with a Close button only ([`tell`]).
pub const MESSAGE: &str = "message";

/// What the reply of a save or export says is pending while the question about replacing files asks
/// ([`ask_replace_files`]).
pub const REPLACE_FILES: &str = "replaceFiles";

/// The key of the question a click or double-click asks before it edits type set in fonts that
/// aren't available ([`ask_missing_font_edit`]).
pub const MISSING_FONT_EDIT: &str = vectorcraft_tools::text::MISSING_FONT_EDIT;

/// Writes the `StackDialog` impl of a wrapper around `$inner` that forwards everything to it and
/// answers `$key` as its id and key. `StandardPrompt` derives its id from the kind of question and
/// the host keeps results per type, so each family of questions is a type of its own: taking one
/// family's answer never takes another's.
macro_rules! wrapped_dialog {
    ($wrapper:ident, $inner:ty, $key:expr) => {
        impl StackDialog for $wrapper {
            type Result = <$inner as StackDialog>::Result;

            fn id(&self) -> DialogId {
                dialog_id($key)
            }

            fn instance(&self) -> Option<DialogId> {
                self.0.instance()
            }

            fn key(&self) -> Option<&str> {
                Some($key)
            }

            fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<Self::Result>> {
                self.0.show(ui, stack)
            }

            fn confirm(&mut self, ctx: &Context) -> Option<Self::Result> {
                self.0.confirm(ctx)
            }

            fn held_state(&self) -> Option<HeldOk> {
                self.0.held_state()
            }

            fn json_fields(&self) -> Option<Value> {
                self.0.json_fields()
            }

            fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
                self.0.set_json_field(ctx, field, value)
            }

            fn cancelled(&mut self, ctx: &Context) {
                self.0.cancelled(ctx)
            }

            fn preview_color(&mut self, key: &str, color: Option<craft::theme::Srgba>) {
                self.0.preview_color(key, color)
            }
        }
    };
}

/// A confirmation: its tag is the command OK runs and that command's params.
pub(crate) struct Question(StandardPrompt);
wrapped_dialog!(Question, StandardPrompt, KIND);

/// A message with a Close button.
pub(crate) struct Message(HostedAlert);
wrapped_dialog!(Message, HostedAlert, MESSAGE);

/// The Type tool's question whether to edit type set in fonts that aren't available; its tag is the
/// tool's edit token.
pub(crate) struct MissingFontQuestion(StandardPrompt);
wrapped_dialog!(MissingFontQuestion, StandardPrompt, MISSING_FONT_EDIT);

/// The tag of the question whether a save or export may replace files ([`ask_replace_files`]).
pub(crate) struct ReplaceFiles {
    /// The uid of the document that asked.
    pub(crate) doc: u64,
    /// The files that are there and which the question names.
    pub(crate) files: Vec<String>,
    /// The files an earlier OK allowed.
    pub(crate) allowed: Vec<String>,
    /// The command OK runs again on that document.
    pub(crate) command: String,
    /// Its params.
    pub(crate) params: Value,
}

/// Ask `message` (with an optional `detail` line), both in the UI language; OK runs `command`
/// with `params`.
pub fn ask(app: &mut VectorcraftApp, message: &str, detail: &str, command: &str, params: Value) {
    let Some(dialogs) = app.dialogs() else {
        app.status(format!("{message} (no window to ask in)"));
        return;
    };
    let mut prompt = StandardPrompt::custom(message.to_string());
    if !detail.is_empty() {
        prompt = prompt.paragraph(detail.to_string());
    }
    let prompt = prompt.button(PromptChoice::Proceed, tl!("OK")).button(PromptChoice::Cancel, tl!("Cancel")).tag((command.to_string(), params));
    dialogs.with(|host| host.open(Question(prompt)));
}

/// Tell the user `message` (with an optional `detail` line), both in the UI language, in a dialog
/// they close. While another dialog is open (the message comes from work in the background) it
/// waits its turn instead of replacing it: neither what the user was doing there nor the message
/// is lost.
pub fn tell(app: &mut VectorcraftApp, message: &str, detail: &str) {
    let Some(dialogs) = app.dialogs() else {
        return;
    };
    if !app.told.admit(message, detail) {
        return;
    }
    let alert = HostedAlert::new(Alert::new(message.to_string(), detail.to_string()).ok_label(tl!("Close")));
    dialogs.with(|host| host.enqueue(StartupStage::Warning, Message(alert)));
}

/// Ask whether to replace `files`, files that are there beside the one a save or export of the
/// document `doc` (its uid) writes, which the save panel did not ask about (a file per artboard, a
/// linked image). OK runs `command` with `params` again on that document, now replacing those
/// files and `allowed` (those an earlier OK allowed); Cancel writes nothing.
pub(crate) fn ask_replace_files(app: &mut VectorcraftApp, doc: u64, files: &[String], allowed: &[String], command: &str, params: Value) {
    let Some(dialogs) = app.dialogs() else {
        return;
    };
    let name = |f: &String| vectorcraft_engine::cmd::fileio::file_name(f);
    let first = files.first().map(name).unwrap_or_default();
    let prompt = StandardPrompt::replace_file(first, None).lines(files.iter().skip(1).map(|f| name(f).into())).tag(ReplaceFiles {
        doc,
        files: files.to_vec(),
        allowed: allowed.to_vec(),
        command: command.to_string(),
        params,
    });
    // Asked when a Background Save or Export ends: a dialog open then stays, and this waits its turn
    // (however many messages wait: the save or export needs the answer).
    dialogs.with(|host| host.enqueue(StartupStage::Warning, prompt));
}

/// Ask whether to edit type whose story is drawn in fonts that aren't available, which the Type
/// tool names in `p` (`token`, `missing` and `style`: "family style" names, `more`: how many more
/// there are). The detail names each font (family and style, the family's own names, which are
/// not translated). OK starts the edit where the type was clicked; Cancel and Escape start nothing.
/// While another dialog is open (a press sent with `ui.pointer` reaches the tool then), that dialog
/// stays and the Type tool forgets the edit, as Cancel would.
pub(crate) fn ask_missing_font_edit(app: &mut VectorcraftApp, p: &Value) {
    let token = p.get("token").cloned().unwrap_or(Value::Null);
    let Some(dialogs) = app.dialogs() else {
        answer_missing_font_edit(app, &token, false);
        return;
    };
    if app.any_dialog_open() && !dialogs.with(|host| host.is_open::<MissingFontQuestion>()) {
        answer_missing_font_edit(app, &token, false);
        return;
    }
    let names = |k: &str| p.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>();
    let mut fonts = names("missing");
    fonts.extend(names("style"));
    if p.get("more").and_then(Value::as_u64).is_some_and(|n| n > 0) {
        fonts.push("…".to_string());
    }
    let detail = format!("{}\n{}", fonts.join(", "), tl!("These fonts aren't available. Their text shows in another font."));
    let prompt = StandardPrompt::custom(tl!("Edit type set in fonts that aren't available?"))
        .paragraph(detail)
        .button(PromptChoice::Proceed, tl!("OK"))
        .button(PromptChoice::Cancel, tl!("Cancel"))
        .tag(token);
    dialogs.with(|host| host.open(MissingFontQuestion(prompt)));
}

/// Give the Type tool the answer to [`ask_missing_font_edit`]'s question about the edit `token`
/// names: with `ok` the edit starts at once; without, the tool forgets it.
pub(crate) fn answer_missing_font_edit(app: &mut VectorcraftApp, token: &Value, ok: bool) {
    app.session.set_tool_option("missingFontEdit", &json!({"token": token, "ok": ok}));
    if ok {
        let r = app.session.tool_poll(app.view_info(), false);
        crate::canvas::apply_requests(app, r);
    }
}

/// Each frame: runs the command of a confirmation answered OK, and forgets the messages the user
/// closed. The command runs after the host's lock is released and the question is closed, so a
/// dialog the command opens stays open.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &Context) {
    let Some(dialogs) = app.dialogs() else {
        return;
    };
    if let Some(DialogResult::Ok(mut answer)) = dialogs.with(|host| host.take::<Question>())
        && answer.choice == PromptChoice::Proceed
        && let Some((command, params)) = answer.take_tag::<(String, Value)>()
        && let Err(e) = app.run(&command, params)
    {
        app.status(e);
    }
    // The host opens queued messages in `enqueue` order, so the one closed is the oldest.
    while dialogs.with(|host| host.take::<Message>()).is_some() {
        app.told.pop_front();
    }
}

/// Each frame: the Type tool's answer to the missing-fonts question. OK starts the edit; Cancel
/// and every other way it closed make the tool forget it.
pub(crate) fn pump_missing_font(app: &mut VectorcraftApp, _ctx: &Context) {
    let Some(dialogs) = app.dialogs() else {
        return;
    };
    let (ok, token) = match dialogs.with(|host| host.take::<MissingFontQuestion>()) {
        None => return,
        Some(DialogResult::Ok(mut answer)) => (answer.choice == PromptChoice::Proceed, answer.take_tag::<Value>()),
        // PLAN: a question the host cancelled carries no tag, so the tool's token is unknown here.
        Some(_) => (false, None),
    };
    answer_missing_font_edit(app, &token.unwrap_or(Value::Null), ok);
}

/// The command and params of the confirmation open now, if one is.
#[cfg(test)]
// PLAN: the prompt exposes no message text, so the question's words cannot be read back.
pub(crate) fn open_question(app: &VectorcraftApp) -> Option<(String, Value)> {
    app.dialogs()?.with(|host| host.get::<Question>().and_then(|q| q.0.tag_as::<(String, Value)>().cloned()))
}
