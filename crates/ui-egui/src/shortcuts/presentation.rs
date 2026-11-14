use super::{Key, VectorcraftApp, json};

/// Does this Escape reset the view's rotation? The caller has checked that nothing earlier took
/// the press. It does when the view is rotated; no popup, modal window (a progress window, About)
/// or Appearance row popup is open; no text field had the keyboard in the last frame (egui takes
/// the focus away on Escape before this runs); no pointer button is held or let go in this frame
/// (a tab or panel drag takes the Escape, in its release frame too); and neither a ruler-origin
/// drag nor a floating Tools move is ending.
pub(super) fn escape_resets_rotation(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    app.view().is_some_and(|v| v.rotation != 0.0)
        && !egui::Popup::is_any_open(ctx)
        && !crate::panel_docking::modal_open(app, ctx)
        && app.ui.transient_panel.is_none()
        && !super::text_focused_in_previous_frame(ctx)
        && !ctx.input(|i| i.pointer.any_down() || i.pointer.any_released())
        && !crate::canvas::ruler_origin_dragging(ctx)
        && !crate::tools_float::escape_cancelled_move(ctx)
}

/// The keys in Presentation Mode, where nothing is edited: Escape leaves the mode, and ← and → go to
/// the previous and the next artboard ([`presentation_step`]). Every other key press, the text it
/// types and the clipboard keys are removed from the input, so no shortcut, tool or field acts on
/// them. The macOS menu bar's key equivalents arrive as key presses too ([`crate::native_menu`]).
pub(super) fn presentation_keys(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let (mut leave, mut steps) = (false, vec![]);
    ctx.input_mut(|i| {
        i.events.retain(|e| match e {
            egui::Event::Key { key: Key::Escape, pressed: true, .. } => {
                leave = true;
                false
            }
            egui::Event::Key { key: Key::ArrowLeft, pressed: true, .. } => {
                steps.push(false);
                false
            }
            egui::Event::Key { key: Key::ArrowRight, pressed: true, .. } => {
                steps.push(true);
                false
            }
            egui::Event::Key { pressed: true, .. }
            | egui::Event::Text(_)
            | egui::Event::Ime(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Paste(_)
            | egui::Event::PasteImage(_)
            | egui::Event::MiddleClickPaste { .. } => false,
            _ => true,
        })
    });
    for next in steps {
        presentation_step(app, next);
    }
    if leave {
        let _ = app.run("view.presentation", json!({}));
    }
}

/// Go to the next artboard (`next`) or the previous one with `view.goToArtboard`, as the status bar
/// navigator's Next and Previous buttons do, for the arrow keys and a click in Presentation Mode.
/// On the last artboard a step forward leaves the view as it is, and so does a step back on the
/// first.
pub(crate) fn presentation_step(app: &mut VectorcraftApp, next: bool) {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let at = crate::panels::artboards::selected(app, n);
    let to = if next { (at + 1 < n).then_some("next") } else { (at > 0).then_some("previous") };
    if let Some(to) = to
        && let Err(e) = app.run("view.goToArtboard", json!({ "index": to }))
    {
        app.status(e);
    }
}
