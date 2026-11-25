//! The Brushes panel's Delete Brush question ([`ask`]): Yes or No when none of the brushes is in
//! use, else Remove Strokes or Cancel. Yes and Remove Strokes run `brush.delete`, which leaves the
//! strokes that used the brushes as plain strokes.
//!
//! The question is a prompt on the window's dialog host; [`pump`] reads its answer each frame.

use craft::ui::dialog::{DialogResult, PromptChoice, StandardPrompt};
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// The key of the Delete Brush question.
pub const KIND: &str = "brushDelete";

/// The Delete Brush prompt; `StandardPrompt` has one id and one result type for every custom prompt,
/// so each question is a type of its own.
pub struct BrushDeleteQuestion(StandardPrompt);

super::confirm::wrapped_dialog!(BrushDeleteQuestion, StandardPrompt, KIND);

/// What the question is about: the document it was asked in, the `brush.delete` params and whether a
/// stroke uses one of the brushes.
struct BrushDeleteAsk {
    doc: Option<u64>,
    params: Value,
    used: bool,
}

/// Ask whether to delete the brushes `params` names (`used`: one of them is in use).
pub fn ask(app: &mut VectorcraftApp, params: Value, used: bool) {
    let Some(dialogs) = app.dialogs() else {
        app.status("no window");
        return;
    };
    let (message, yes, no) = words(used);
    let doc = app.session.active().map(|d| d.uid);
    let prompt =
        StandardPrompt::custom(message).button(PromptChoice::Proceed, yes).button(PromptChoice::Cancel, no).tag(BrushDeleteAsk { doc, params, used });
    dialogs.with(|host| host.open(BrushDeleteQuestion(prompt)));
}

/// The question's message and its Proceed and Cancel labels (`used`: one of the brushes is in use).
fn words(used: bool) -> (&'static str, &'static str, &'static str) {
    if used {
        (
            tl!("Strokes use one or more of these brushes. Take the brushes off those strokes and delete the brushes?"),
            tl!("Remove Strokes"),
            tl!("Cancel"),
        )
    } else {
        (tl!("Delete the selected brushes?"), tl!("Yes"), tl!("No"))
    }
}

/// Whether a stroke uses one of `names` (`brush.unused`). `None` when the query fails: the reason is
/// in the status bar and the caller changes nothing.
pub fn used(app: &mut VectorcraftApp, names: &[String]) -> Option<bool> {
    let unused = match app.run("brush.unused", json!({})) {
        Ok(unused) => unused,
        Err(e) => {
            app.status(e);
            return None;
        }
    };
    let unused = unused.get("names").and_then(Value::as_array).cloned().unwrap_or_default();
    Some(names.iter().any(|name| !unused.iter().any(|n| n.as_str() == Some(name))))
}

/// The brush names in `brush.delete` params (`name` or `names`).
fn names(params: &Value) -> Vec<String> {
    match params.get("name").and_then(Value::as_str) {
        Some(name) => vec![name.to_string()],
        None => params.get("names").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
    }
}

/// Runs the command of the answer the question got this frame. Only Yes and Remove Strokes act.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(DialogResult::Ok(mut answer)) = dialogs.with(|host| host.take::<BrushDeleteQuestion>()) else { return };
    if answer.choice != PromptChoice::Proceed {
        return;
    }
    if let Some(ask) = answer.take_tag::<BrushDeleteAsk>() {
        confirm(app, ask);
    }
}

/// Yes on brushes that a stroke has started to use since the question opened asks again with Remove
/// Strokes and Cancel. When another document has become the active one, nothing is deleted.
fn confirm(app: &mut VectorcraftApp, question: BrushDeleteAsk) {
    if question.doc != app.session.active().map(|doc| doc.uid) {
        return;
    }
    if !question.used {
        match used(app, &names(&question.params)) {
            None => return,
            Some(true) => return ask(app, question.params, true),
            Some(false) => {}
        }
    }
    if let Err(e) = app.run("brush.delete", question.params) {
        app.status(e);
    }
}

/// The message of the Delete Brush question open now and whether it is the in-use one.
#[cfg(test)]
pub(crate) fn open_question(app: &VectorcraftApp) -> Option<(String, bool)> {
    let used = app.dialogs()?.with(|host| host.get::<BrushDeleteQuestion>().and_then(|q| q.0.tag_as::<BrushDeleteAsk>().map(|a| a.used)))?;
    Some((words(used).0.to_string(), used))
}

#[cfg(test)]
pub(super) mod tests;
