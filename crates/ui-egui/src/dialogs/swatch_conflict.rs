//! Swatch Conflict: pasting objects whose global or spot swatch has a name the document gives
//! another colour asks, swatch by swatch, whether to merge (the objects take the document's
//! swatch) or add (the pasted swatch comes in under a new name); Apply to All answers the rest the
//! same way. OK on the last one answers the paste with `swatchConflict`; Cancel pastes nothing.
//!
//! The dialog is not held: the paste that asked takes the answer ([`take`]) and runs the paste itself. Fields:
//! `choice` (`merge` or `add`) and `applyToAll` (with more than one conflict).

use craft::theme::Srgba;
use craft::ui::choice::{Checkbox, Radio, RadioGroup, Swatch};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult as Answer, DialogStack, FieldRefusal, Prompt, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::env::Env;
use craft::ui::runtime::words::CommonWords;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::{Context, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_color::Color;

use super::swatch_options::NO_WINDOW;
use crate::VectorcraftApp;
use crate::placing::Taken;

/// The dialog kind of Swatch Conflict.
pub const KIND: &str = "swatchConflict";

/// How a conflict is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    /// The document's swatch is used.
    Merge,
    /// The pasted swatch comes in under a new name.
    Add,
}

impl Choice {
    /// The `choice` field's id, and the `swatchConflict` answer of the paste command.
    const fn id(self) -> &'static str {
        match self {
            Choice::Merge => "merge",
            Choice::Add => "add",
        }
    }

    fn named(id: &str) -> Option<Choice> {
        [Choice::Merge, Choice::Add].into_iter().find(|c| c.id() == id)
    }
}

/// The choices: (choice, label, what it does).
const CHOICES: [(Choice, &str, &str); 2] = [
    (Choice::Merge, "Merge Swatches", "Use the document's swatch: pasted colors take its color."),
    (Choice::Add, "Add Swatches", "Keep the pasted color as a new swatch with a numbered name."),
];

/// The question about the swatches a paste brings that the document names differently. Its answer is the value
/// [`take`] hands the paste: `command`, `params` (with `swatchConflict`), `deferred`, `asked` and `drawing`.
struct SwatchConflictDialog {
    id: DialogId,
    /// The conflicts as `clipboard.conflicts` lists them: `[{name, document, clipboard, spot}]`.
    conflicts: Vec<Value>,
    /// The conflict asked about.
    index: usize,
    choice: Choice,
    apply_to_all: bool,
    /// The answers so far, by swatch name.
    choices: Map<String, Value>,
    /// The paste command and its parameters.
    command: String,
    params: Value,
    /// The drawing and content generation the question is about.
    drawing: Option<(u64, u64)>,
    /// The deferred paste waiting on the answer ([`defer`]).
    deferred: Option<u64>,
    open: bool,
}

impl SwatchConflictDialog {
    fn new(conflicts: Vec<Value>, command: &str, params: &Value, drawing: Option<(u64, u64)>) -> Self {
        SwatchConflictDialog {
            id: dialog_id(KIND),
            conflicts,
            index: 0,
            choice: Choice::Merge,
            apply_to_all: false,
            choices: Map::new(),
            command: command.to_owned(),
            params: Some(params).filter(|p| p.is_object()).cloned().unwrap_or_else(|| json!({})),
            drawing,
            deferred: None,
            open: true,
        }
    }

    /// Whether the Apply to All box takes input: more than one conflict is asked.
    fn offers_apply_to_all(&self) -> bool {
        self.conflicts.len() > 1
    }

    /// Records the answer. With conflicts left (and Apply to All off) it asks about the next one and answers
    /// `None`; else it answers the paste's value.
    fn answer(&mut self) -> Option<Value> {
        let i = self.index;
        let answered = if self.apply_to_all { self.conflicts.get(i..) } else { self.conflicts.get(i..=i) }.unwrap_or_default();
        for name in answered.iter().filter_map(|c| c["name"].as_str()) {
            self.choices.insert(name.to_owned(), json!(self.choice.id()));
        }
        if !self.apply_to_all && i + 1 < self.conflicts.len() {
            self.index = i + 1;
            // The press that answered this one closed the prompt's own open state.
            self.open = true;
            return None;
        }
        let mut params = self.params.clone();
        params["swatchConflict"] = Value::Object(self.choices.clone());
        Some(json!({"command": self.command, "params": params, "deferred": self.deferred, "asked": self.conflicts, "drawing": self.drawing}))
    }
}

/// A chip colour from a `#rrggbb` string; transparent for none.
fn chip(hex: Option<&str>) -> Srgba {
    hex.and_then(Color::from_hex).map_or(Srgba::rgb(0.0, 0.0, 0.0).with_alpha(0.0), |c| {
        let [r, g, b] = c.to_rgb();
        Srgba::rgb(r, g, b)
    })
}

impl StackDialog for SwatchConflictDialog {
    type Result = Value;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<Answer<Value>> {
        let env = Env::of(ui);
        let common = env.words::<CommonWords>();
        let Some(current) = self.conflicts.get(self.index).cloned() else { return Some(Answer::Cancel) };
        let (position, count) = (self.index + 1, self.conflicts.len());
        let name = current["name"].as_str().unwrap_or_default();
        let message = crate::i18n::fmt(tl!("This document already has a swatch named “{name}” with another color."), &[("name", name)]);
        let counter =
            (count > 1).then(|| crate::i18n::fmt(tl!("Conflict {n} of {count}"), &[("n", &position.to_string()), ("count", &count.to_string())]));
        let chips = [(tl!("Document"), chip(current["document"].as_str())), (tl!("Pasted"), chip(current["clipboard"].as_str()))];
        let answers = [FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let Self { choice, apply_to_all, open, .. } = self;
        let shown = Prompt::new(tl!("Swatch Conflict").to_owned(), message)
            .answers(answers)
            .extras(|ui| {
                if let Some(counter) = &counter {
                    Text::new(counter.clone()).show(ui);
                }
                Row::new().show(ui, |ui, _| {
                    for (caption, color) in &chips {
                        Stack::new().show(ui, |ui, _| {
                            Swatch::new((*caption).to_owned(), *color).show(ui);
                            Text::new((*caption).to_owned()).show(ui);
                        });
                    }
                });
                RadioGroup::new(tl!("Options").to_owned(), Bind::bound(&mut *choice)).show(ui, |ui, _| {
                    for (value, text, help) in CHOICES {
                        Radio::new(value).label(tl!(text).to_owned()).detail(tl!(help).to_owned()).show(ui);
                    }
                });
                Checkbox::new(Bind::bound(&mut *apply_to_all)).label(tl!("Apply to All").to_owned()).enabled(count > 1).show(ui);
            })
            .opened(Bind::bound(open))
            .show(ui);
        match shown.picked {
            Some(0) => self.answer().map(Answer::Ok),
            Some(_) => Some(Answer::Cancel),
            None => None,
        }
    }

    /// The answer an OK gives: the paste's value on the last conflict, else the next conflict is asked and there is
    /// no answer yet.
    fn confirm(&mut self, _: &Context) -> Option<Value> {
        self.answer()
    }

    /// The choice, Apply to All when it takes input, and the conflicts asked about, which automation reads only.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = json!({ "choice": self.choice.id(), "conflicts": self.conflicts });
        if self.offers_apply_to_all() {
            fields["applyToAll"] = Value::Bool(self.apply_to_all);
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "choice" => match value.as_str().and_then(Choice::named) {
                Some(choice) => self.choice = choice,
                None => return Err(FieldRefusal::Rejects(&value).message(field)),
            },
            "applyToAll" if self.offers_apply_to_all() => match value.as_bool() {
                Some(on) => self.apply_to_all = on,
                None => return Err(FieldRefusal::Rejects(&value).message(field)),
            },
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// Before running paste `command` with `params`: when the pasted objects' swatches conflict with
/// the document's and `params` don't say how to resolve them (`swatchConflict`), open the dialog
/// instead and answer `{dialog}`. A question still open from an earlier paste is replaced by this one: the host
/// answers it Cancel.
// PLAN: files-w8 owns `placing::cancel_swatch_paste`; the replaced question's Cancel reaches the paste through `take`.
pub fn ask(app: &mut VectorcraftApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    if params.get("swatchConflict").is_some() {
        return None;
    }
    let conflicts = found(app, command, params).filter(|a| !a.is_empty())?;
    let drawing = app.session.active().map(|doc| (doc.uid, doc.reverts));
    let dialog = SwatchConflictDialog::new(conflicts, command, params, drawing);
    let Some(host) = app.dialogs() else { return Some(Err(NO_WINDOW.into())) };
    host.with(|h| h.open(dialog));
    Some(Ok(json!({ "dialog": KIND })))
}

/// The id of the open question, for the paste to take its answer by; `None` when none is open.
pub(crate) fn id(app: &VectorcraftApp) -> Option<DialogId> {
    let id = dialog_id(KIND);
    app.dialogs()?.with(|h| h.get_by_id::<SwatchConflictDialog>(id).map(|_| id))
}

/// What the question `id` answered this pass. An immediate question whose drawing is inactive or
/// reverted answers Cancel. Placing checks deferred answers against their original drawing.
pub(crate) fn take(app: &VectorcraftApp, id: DialogId) -> Taken {
    let Some(host) = app.dialogs() else { return Taken::Open };
    match host.with(|h| h.take_by_id::<SwatchConflictDialog>(id)) {
        Some(Answer::Ok(value)) => {
            let drawing = app.session.active().map(|doc| (doc.uid, doc.reverts));
            let retired = value["deferred"].is_null() && value["drawing"] != json!(drawing);
            if retired { Taken::Cancelled } else { Taken::Answered(value) }
        }
        Some(_) => Taken::Cancelled,
        None => Taken::Open,
    }
}

/// Stores the deferred paste `chain` on the open question, which hands it back with its answer.
pub(crate) fn defer(app: &VectorcraftApp, chain: u64) {
    if let Some(host) = app.dialogs() {
        host.with(|h| {
            if let Some(dialog) = h.get_mut_by_id::<SwatchConflictDialog>(dialog_id(KIND)) {
                dialog.deferred = Some(chain);
            }
        });
    }
}

/// Whether the conflicts paste `command` with `params` raises are no longer the `asked` ones (another profile
/// converts the pasted swatches meanwhile): it asks about them again.
pub(crate) fn changed(app: &mut VectorcraftApp, command: &str, params: &Value, asked: &Value) -> bool {
    found(app, command, params).is_some_and(|now| Value::Array(now) != *asked)
}

/// The swatch conflicts paste `command` with `params` raises (`clipboard.conflicts`), with the
/// swatches as the paste brings them: converted from another profile when it converts, and for
/// Paste on All Artboards without the backgrounds of copied artboards, which it leaves behind.
fn found(app: &mut VectorcraftApp, command: &str, params: &Value) -> Option<Vec<Value>> {
    let mut q = json!({});
    if let Some(m) = params.get("profileMismatch") {
        q["profileMismatch"] = m.clone();
    }
    if command == "edit.pasteOnAllArtboards" {
        q["allArtboards"] = json!(true);
    }
    let found = app.session.execute("clipboard.conflicts", &q).ok()?;
    found["swatches"].as_array().cloned()
}
