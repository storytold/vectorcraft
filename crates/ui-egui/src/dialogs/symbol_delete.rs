//! The Symbols panel's Delete Symbol question ([`ask`]). A symbol no instance uses gets "Delete the
//! selected symbol?" with Yes and No ([`KIND`]); a symbol with placed instances gets Expand
//! Instances, Delete Instances and Cancel ([`USED`]).
//!
//! The question carries the [`vectorcraft_engine::DocState::uid`] of the document it was asked in and
//! the symbol names. Yes or either Instances button deletes the symbol only while that document is
//! the active one and still has the symbol; an unused symbol that has gained instances since asks
//! the in-use question instead.

use craft::ui::dialog::{PromptChoice, StandardPrompt};
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// The dialog kind of the question for a symbol no instance uses.
pub const KIND: &str = "deleteSymbol";

/// The dialog kind of the question for a symbol with placed instances.
pub const USED: &str = "deleteUsedSymbol";

/// The symbols a delete question is about.
enum Target {
    One(String),
    Many(Vec<String>),
}

/// What a delete question carries back in its answer: the document it was asked in, the symbols, and
/// whether it is the in-use question.
struct Ask {
    doc: u64,
    target: Target,
    used: bool,
}

/// The delete questions. One type for both, so the host's per-type result never mixes with another
/// area's prompt; the [`Ask`] tag tells them apart.
pub(crate) struct SymbolQuestion(pub(crate) StandardPrompt);

crate::dialogs::confirm::wrapped_dialog!(SymbolQuestion, StandardPrompt, KIND);

/// Delete Symbol for symbol `name` of the active document. A symbol with instances always asks
/// what becomes of them; an unused one is deleted at once with `now` (Alt-click on the panel's
/// button) and asked about otherwise.
pub(crate) fn ask(app: &mut VectorcraftApp, name: &str, now: bool) {
    let Some(uid) = app.session.active().map(|st| st.uid) else { return };
    match instances(app, uid, name) {
        None => {}
        Some(0) if now => {
            if let Err(e) = app.run("symbol.delete", json!({ "name": name })) {
                app.status(e);
            }
        }
        Some(n) => open(app, Ask { doc: uid, target: Target::One(name.to_string()), used: n > 0 }),
    }
}

/// Delete Symbol for the unused symbols `names` that Select All Unused picked (#4918): asked once
/// ("Delete the selected symbols?", or the singular for one), or at once with `now`. Only those still
/// unused then (`symbol.unused`: brush art, patterns and nested instances count) go, in one undo step.
pub(crate) fn ask_many(app: &mut VectorcraftApp, names: &[String], now: bool) {
    let Some(uid) = app.session.active().map(|st| st.uid) else { return };
    if now {
        // An error already went to the status bar.
        let _ = delete_unused(app, uid, names);
    } else {
        open(app, Ask { doc: uid, target: Target::Many(names.to_vec()), used: false });
    }
}

/// Deletes those of `names` that are still unused in document `uid` while it is the active one.
fn delete_unused(app: &mut VectorcraftApp, uid: u64, names: &[String]) -> Result<Value, String> {
    if app.session.active().map(|st| st.uid) != Some(uid) {
        return Ok(Value::Null);
    }
    let unused: Vec<String> =
        app.run("symbol.unused", json!({})).ok().and_then(|r| serde_json::from_value(r["names"].clone()).ok()).unwrap_or_default();
    let names: Vec<&String> = names.iter().filter(|n| unused.contains(n)).collect();
    if names.is_empty() {
        return Ok(Value::Null);
    }
    let r = app.run("symbol.delete", json!({ "names": names }));
    if let Err(e) = &r {
        app.status(e.clone());
    }
    r
}

/// Opens the question `ask` describes.
fn open(app: &mut VectorcraftApp, ask: Ask) {
    let prompt = if ask.used {
        StandardPrompt::custom(tl!("One or more of the symbols are in use and cannot be deleted until their instances are expanded or deleted."))
            .button(PromptChoice::Proceed, tl!("Expand Instances"))
            .button(PromptChoice::Alternate, tl!("Delete Instances"))
            .button(PromptChoice::Cancel, tl!("Cancel"))
    } else {
        let plural = matches!(&ask.target, Target::Many(names) if names.len() > 1);
        StandardPrompt::custom(if plural { tl!("Delete the selected symbols?") } else { tl!("Delete the selected symbol?") })
            .button(PromptChoice::Proceed, tl!("Yes"))
            .button(PromptChoice::Cancel, tl!("No"))
    };
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| host.open(SymbolQuestion(prompt.tag(ask))));
    }
}

/// Runs the answer of a delete question, once per frame.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    // The host lock is released before any command runs.
    let Some(mut answer) = dialogs.with(|host| host.take::<SymbolQuestion>()).and_then(|result| result.ok()) else { return };
    let Some(ask) = answer.take_tag::<Ask>() else { return };
    let r = match (ask.used, answer.choice) {
        (false, PromptChoice::Proceed) => confirm_unused(app, ask),
        (true, PromptChoice::Proceed) => confirm_used(app, ask, true),
        (true, PromptChoice::Alternate) => confirm_used(app, ask, false),
        _ => return,
    };
    if let Err(e) = r {
        app.status(e);
    }
}

/// How many instances symbol `name` has, while the document `uid` is the active one and has the
/// symbol; `None` otherwise.
fn instances(app: &mut VectorcraftApp, uid: u64, name: &str) -> Option<u64> {
    if app.session.active().map(|st| st.uid) != Some(uid) {
        return None;
    }
    let list = app.run("symbol.list", json!({})).ok()?;
    list["symbols"].as_array()?.iter().find(|s| s["name"].as_str() == Some(name)).map(|s| s["instances"].as_u64().unwrap_or(0))
}

/// Yes: deletes the symbol, or asks the in-use question when it has gained instances.
fn confirm_unused(app: &mut VectorcraftApp, ask: Ask) -> Result<Value, String> {
    let Ask { doc, target, .. } = ask;
    match target {
        Target::Many(names) => delete_unused(app, doc, &names),
        Target::One(name) => match instances(app, doc, &name) {
            None => Ok(Value::Null),
            Some(0) => app.run("symbol.delete", json!({ "name": name })),
            Some(_) => {
                open(app, Ask { doc, target: Target::One(name), used: true });
                Ok(Value::Null)
            }
        },
    }
}

/// Expand Instances turns the instances into ordinary art; Delete Instances removes them. Either one
/// deletes the symbol in one undo step.
fn confirm_used(app: &mut VectorcraftApp, ask: Ask, expand: bool) -> Result<Value, String> {
    let Target::One(name) = ask.target else { return Ok(Value::Null) };
    if instances(app, ask.doc, &name).is_none() {
        return Ok(Value::Null);
    }
    app.run("symbol.delete", json!({ "name": name, "expandInstances": expand }))
}
