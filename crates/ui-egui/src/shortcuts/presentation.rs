use super::{VectorcraftApp, json};

/// Does this Escape reset the view's rotation? The caller has checked that nothing earlier took
/// the press. It does when the view is rotated and no pointer button is held or let go in this
/// frame (a tab or panel drag takes the Escape, in its release frame too).
pub(super) fn escape_resets_rotation(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    app.view().is_some_and(|v| v.rotation != 0.0) && !ctx.input(|i| i.pointer.any_down() || i.pointer.any_released())
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
