//! The Brushes panel's question when art Alt-dragged (Option-dragged on macOS) onto a brush that
//! strokes use is to replace its art ([`ask`]): Apply to Strokes, Leave Strokes or Cancel. Apply to
//! Strokes runs `brush.replaceArt` and the strokes redraw with the new art; Leave Strokes runs it
//! with `applyToStrokes: false`, which keeps the brush and its strokes as they were and adds a copy
//! of the brush with the new art.
//!
//! The question is a prompt on the window's dialog host; [`pump`] reads its answer each frame.

use craft::ui::dialog::{DialogResult, PromptChoice, StandardPrompt};
use serde_json::json;

use crate::VectorcraftApp;

/// The key of the Replace Brush Art question.
pub const KIND: &str = "brushReplaceArt";

/// The Replace Brush Art prompt; `StandardPrompt` has one id and one result type for every custom
/// prompt, so each question is a type of its own.
pub struct ReplaceArtQuestion(StandardPrompt);

super::confirm::wrapped_dialog!(ReplaceArtQuestion, StandardPrompt, KIND);

/// What the question is about: the document it was asked in, the brush and the art.
struct ReplaceArtAsk {
    doc: Option<u64>,
    name: String,
    ids: Vec<u64>,
}

/// Replace the art of brush `name` with the objects `ids`: at once when no stroke uses the brush,
/// else after asking whether the strokes take the new art. Art the brush cannot take (art with no
/// width, say) is refused before anything asks, with the reason in the status bar.
pub fn replace(app: &mut VectorcraftApp, name: &str, ids: &[u64]) {
    let params = json!({"name": name, "ids": ids});
    let Some(used) = super::brush_delete::used(app, &[name.to_string()]) else { return };
    if !used {
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
    let Some(dialogs) = app.dialogs() else {
        app.status("no window");
        return;
    };
    let doc = app.session.active().map(|d| d.uid);
    let prompt = StandardPrompt::custom(tl!(
        "Strokes use this brush. Apply the new art to those strokes, or leave them as they are and add a copy of the brush with the new art?"
    ))
    .button(PromptChoice::Proceed, tl!("Apply to Strokes"))
    .button(PromptChoice::Alternate, tl!("Leave Strokes"))
    .button(PromptChoice::Cancel, tl!("Cancel"))
    .tag(ReplaceArtAsk { doc, name: name.to_string(), ids: ids.to_vec() });
    dialogs.with(|host| host.open(ReplaceArtQuestion(prompt)));
}

/// Runs the command of the answer the question got this frame; Cancel and any other close do nothing.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(DialogResult::Ok(mut answer)) = dialogs.with(|host| host.take::<ReplaceArtQuestion>()) else { return };
    let apply = match answer.choice {
        PromptChoice::Proceed => true,
        PromptChoice::Alternate => false,
        _ => return,
    };
    if let Some(ask) = answer.take_tag::<ReplaceArtAsk>() {
        confirm(app, ask, apply);
    }
}

/// Runs `brush.replaceArt`. When another document has become the active one, no brush changes.
fn confirm(app: &mut VectorcraftApp, question: ReplaceArtAsk, apply: bool) {
    if question.doc != app.session.active().map(|doc| doc.uid) {
        return;
    }
    let params = json!({"name": question.name, "ids": question.ids, "applyToStrokes": apply});
    if let Err(e) = app.run("brush.replaceArt", params) {
        app.status(e);
    }
}

#[cfg(test)]
mod tests;
