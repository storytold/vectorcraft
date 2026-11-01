//! The Symbols panel's Delete Symbol question ([`ask`]). A symbol no instance uses gets "Delete the
//! selected symbol?" with Yes and No ([`KIND`]); a symbol with placed instances gets Expand
//! Instances, Delete Instances and Cancel ([`USED`]).
//!
//! Fields: `name` (the symbol) and `doc` (the [`vectorcraft_engine::DocState::uid`] of the document
//! it was asked in). Yes or either Instances button deletes the symbol only while that document is
//! the active one and still has the symbol; an unused symbol that has gained instances since asks
//! the in-use question instead.

use serde_json::{Value, json};

use super::{DialogResult, DialogSpec};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of the question for a symbol no instance uses.
pub const KIND: &str = "deleteSymbol";

/// The dialog kind of the question for a symbol with placed instances.
pub const USED: &str = "deleteUsedSymbol";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Delete the selected symbol?").into(),
    body: |_, _, _| false,
    confirm: confirm_unused,
    ok: Some("Yes"),
    cancel: Some("No"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

pub(super) const USED_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("One or more of the symbols are in use and cannot be deleted until their instances are expanded or deleted.").into(),
    body: |_, _, _| false,
    confirm: confirm_used,
    ok: Some("Expand Instances"),
    discard: Some("Delete Instances"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

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
        Some(n) => open(app, if n == 0 { KIND } else { USED }, name, uid),
    }
}

fn open(app: &mut VectorcraftApp, kind: &str, name: &str, uid: u64) {
    app.ui.dialog = Some(Dialog::new(kind, json!({ "name": name, "doc": uid })));
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

/// The document and symbol `d` asks about.
fn target(d: &Dialog) -> (u64, String) {
    (d.fields.get("doc").and_then(Value::as_u64).unwrap_or(0), d.str("name"))
}

/// Yes: deletes the symbol, or asks the in-use question when it has gained instances.
fn confirm_unused(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    app.ui.dialog = None;
    let (uid, name) = target(d);
    match instances(app, uid, &name) {
        None => Ok(Value::Null),
        Some(0) => app.run("symbol.delete", json!({ "name": name })),
        Some(_) => {
            open(app, USED, &name, uid);
            Ok(Value::Null)
        }
    }
}

/// Expand Instances (OK) turns the instances into ordinary art; Delete Instances (`discard`)
/// removes them. Either one deletes the symbol in one undo step.
fn confirm_used(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    app.ui.dialog = None;
    let (uid, name) = target(d);
    if instances(app, uid, &name).is_none() {
        return Ok(Value::Null);
    }
    app.run("symbol.delete", json!({ "name": name, "expandInstances": !d.bool("discard") }))
}
