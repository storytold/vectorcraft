//! The ruler guides on the canvas: the document's guides shown through the `Guides` control, which
//! only draws them. Moving and deleting them stays with the tools
//! ([`vectorcraft_tools::rulerguide`]), so the pointer reaches the tool.

use craft::behavior::Axis;
use craft::theme::{Rgba, Tokens};
use craft::ui::canvas::{CanvasSpace, Guide, GuideKey, GuideLine};
use craft::ui::canvas_controls::Guides;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::runtime::env::Env;
use egui::{Color32, Ui};

use super::BodyKind;
use crate::VectorcraftApp;
use crate::doc_windows::Win;

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
    let color = guide_color(prefs, t);
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

/// The ruler guides' colour (Preferences › Guides & Grid), the theme's guide colour when the
/// preference does not parse.
pub(super) fn guide_color(prefs: &vectorcraft_engine::Prefs, t: &Tokens) -> Rgba {
    vectorcraft_color::Color::from_hex(&prefs.guide_color).map_or(t.color.overlay.guide, |c| {
        let c = crate::panels::c32(&c);
        Rgba::rgb(c.r(), c.g(), c.b())
    })
}

/// Shows the guides of window `win`'s document over the canvas, with the guide a ruler drag places
/// in this pass, which the control reads from the canvas. The control is display-only in every
/// body: guide editing is the tools' one mechanism, so it reports no edit.
pub(super) fn show(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace, win: Win, _kind: BodyKind) {
    let Some(st) = app.session.documents().get(win.0) else { return };
    let env = Env::of(ui);
    let Some((mut guides, selected, key)) = shown(app, st, env.tokens.tokens()) else { return };
    // The edits of a display-only control are none: the local copy is dropped.
    let control = Guides::new("Ruler guides", super::canvas_id(win), Bind::bound(&mut guides)).selected_guides(&selected).interactive(false);
    match key {
        Some(key) => control.key_guide(key),
        None => control,
    }
    .show_at(ui, space.view.rect);
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
