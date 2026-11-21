//! The ruler guides on the canvas: the document's guides shown through the `Guides` control, which
//! only draws them. Moving and deleting them stays with the tools
//! ([`vectorcraft_tools::rulerguide`]), so the pointer reaches the tool.

use super::grid::pref_color;
use super::{Color32, Ui, VectorcraftApp};
use crate::doc_windows::Win;
use crate::theme::Tokens;
// PLAN: assumed crate paths `craft_ui_canvas::{canvas, canvas_controls::Guides}`, `contract::Bind`,
// `Show`; `super::{BodyKind, canvas_id}` (canvas-w10).
use super::BodyKind;
use craft::behavior::Axis;
use craft::theme::Rgba;
use craft_ui_canvas::canvas::{CanvasSpace, Guide, GuideDrag, GuideKey, GuideLine};
use craft_ui_canvas::canvas_controls::Guides;
use craft_ui_canvas::contract::{Bind, Show as _};

/// The guides of one document as shown: the guides, the selected keys and the key guide (Align to
/// Key Object). `None` while guides are hidden (Hide Guides, Trim View) or presenting.
pub(super) type Shown = (Vec<Guide>, Vec<GuideKey>, Option<GuideKey>);

/// The ruler guides of document `st` as the control takes them. A guide on a hidden layer hides
/// with it. An artboard guide of an artboard with a guide label
/// ([`vectorcraft_doc::Artboard::guide_label`]) takes the label's colour; the others the guide
/// preference's colour, all in the guide preference's style.
pub(super) fn shown(app: &VectorcraftApp, st: &vectorcraft_engine::DocState, t: &Tokens) -> Option<Shown> {
    if !app.ui.view.guides_shown() || app.ui.screen_mode >= 3 {
        return None;
    }
    let doc = &st.doc;
    let prefs = &app.session.prefs;
    let rgb = |c: Color32| Rgba::rgb(c.r(), c.g(), c.b());
    let color = rgb(pref_color(&prefs.guide_color, t.guide));
    let line = if prefs.guide_style == "dots" { GuideLine::Dots } else { GuideLine::Solid };
    let labels = guide_labels(doc);
    let guides = doc
        .ruler_guides()
        .iter()
        .filter(|g| doc.guide_shown(g))
        .filter_map(|g| {
            let id = g.id?;
            let mut guide = Guide::new(GuideKey::new(id.0), if g.vertical { Axis::Y } else { Axis::X }, g.pos);
            guide.span = doc.guide_span(g);
            guide.color = Some(g.artboard.and_then(|a| labels.get(&a)).map_or(color, |&c| rgb(c)));
            guide.line = line;
            guide.locked = !doc.guide_editable(g);
            Some(guide)
        })
        .collect();
    let selected = st.selection.guides.iter().map(|id| GuideKey::new(id.0)).collect();
    Some((guides, selected, st.selection.key_guide.map(|id| GuideKey::new(id.0))))
}

/// Shows the guides of window `win`'s document over the canvas. `pending` is the guide a ruler
/// drag places in this pass (`rulers::show` returns it, so it is shown before this). The control
/// is display-only in every body: guide editing is the tools' one mechanism, so it reports no edit.
pub(super) fn show(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace, win: Win, _kind: BodyKind, pending: Option<GuideDrag>) {
    let Some(st) = app.session.documents().get(win.0) else { return };
    let t = Tokens::get(ui.ctx());
    let Some((mut guides, selected, key)) = shown(app, st, &t) else { return };
    let mut control = Guides::new("Ruler guides", super::canvas_id(win), Bind::bound(&mut guides))
        .selected_guides(&selected)
        .key_guide(key)
        .interactive(false);
    if let Some(drag) = pending {
        control = control.pending(drag);
    }
    // The edits of a display-only control are none: the local copy is dropped.
    control.show_at(ui, space.view.screen_rect());
}

/// The guide label colors of `doc`'s artboards that have one, by artboard id.
pub(super) fn guide_labels(doc: &vectorcraft_doc::Document) -> std::collections::HashMap<u32, Color32> {
    doc.artboards
        .iter()
        .filter_map(|a| {
            let [r, g, b] = a.guide_label?.rgb();
            Some((a.id, Color32::from_rgb(r, g, b)))
        })
        .collect()
}

#[cfg(test)]
mod tests;
