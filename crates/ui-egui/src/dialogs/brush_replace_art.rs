//! The Brushes panel's question when art Alt-dragged (Option-dragged on macOS) onto a brush that
//! strokes use is to replace its art ([`ask`]): Apply to Strokes, Leave Strokes or Cancel. Apply to
//! Strokes runs `brush.replaceArt` and the strokes redraw with the new art; Leave Strokes runs it
//! with `applyToStrokes: false`, which keeps the brush and its strokes as they were and adds a copy
//! of the brush with the new art.
//!
//! Fields: `name` (the brush), `ids` (the art), `apply` (set by the button pressed) and `__doc` (the
//! uid of the document it asks about).

use serde_json::{Value, json};

use super::{DialogAction, DialogSpec};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::widgets;

/// The dialog kind of the question.
pub const KIND: &str = "brushReplaceArt";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| {
        tl!("Strokes use this brush. Apply the new art to those strokes, or leave them as they are and add a copy of the brush with the new art?")
            .to_string()
    },
    body_with_actions: Some(body),
    confirm,
    ok: None,
    max_width: Some(540.0),
    ..DialogSpec::FORM
};

/// The field that holds the uid of the document the question is about.
const DOC: &str = "__doc";

/// Replace the art of brush `name` with the objects `ids`: at once when no stroke uses the brush,
/// else after asking whether the strokes take the new art. Art the brush cannot take (art with no
/// width, say) is refused before anything asks, with the reason in the status bar.
pub fn replace(app: &mut VectorcraftApp, name: &str, ids: &[u64]) {
    let params = json!({"name": name, "ids": ids});
    if super::brush_delete::used(app, &[name.to_string()]) != Some(true) {
        if let Err(e) = app.run("brush.replaceArt", params) {
            app.status(e);
        }
        return;
    }
    // A preview on a copy of the document checks the art without changing anything.
    if !app.session.in_interaction() && app.begin_interaction("Replace Brush Art").is_ok() {
        let checked = app.session.preview("brush.replaceArt", &params);
        // The interaction began above, on the active document: ending it puts that document back
        // as it was, and can fail only when no document is open, when there is nothing to put back.
        let _ = app.session.cancel_interaction();
        if let Err(e) = checked {
            app.status(e.to_string());
            return;
        }
    }
    ask(app, name, ids);
}

/// Ask whether the strokes that use brush `name` take the art of the objects `ids`.
pub fn ask(app: &mut VectorcraftApp, name: &str, ids: &[u64]) {
    let doc = app.session.active().map(|d| d.uid);
    app.ui.dialog = Some(Dialog::new(KIND, json!({"name": name, "ids": ids, "apply": true, DOC: doc})));
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let mut action = DialogAction::None;
    ui.horizontal(|ui| {
        if widgets::primary_button_enabled(ui, tl!("Apply to Strokes"), true).clicked() {
            d.fields.insert("apply".into(), json!(true));
            action = DialogAction::Confirm;
        }
        if widgets::secondary_button(ui, tl!("Leave Strokes")).clicked() {
            d.fields.insert("apply".into(), json!(false));
            action = DialogAction::Confirm;
        }
        if widgets::secondary_button(ui, tl!("Cancel")).clicked() {
            action = DialogAction::Cancel;
        }
    });
    action
}

/// Closes before running. When another document has become the active one, it closes as Cancel does
/// and changes no brush.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    if d.fields.get(DOC).and_then(Value::as_u64) != app.session.active().map(|doc| doc.uid) {
        return Ok(json!({}));
    }
    let params = json!({
        "name": d.fields.get("name").cloned().unwrap_or(Value::Null),
        "ids": d.fields.get("ids").cloned().unwrap_or_else(|| json!([])),
        "applyToStrokes": d.bool("apply"),
    });
    app.run("brush.replaceArt", params)
}

#[cfg(test)]
mod tests;
