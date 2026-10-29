use super::{Key, VectorcraftApp, is_modifier, json};

/// The second press of Escape pressed twice comes less than this many seconds after the first.
const ESCAPE_PAIR_SECONDS: f64 = 1.0;

/// The key of the last counted Escape's time in egui's temporary data.
pub(super) fn escape_pair_id() -> egui::Id {
    egui::Id::unique("escape-pair")
}

/// Reads this frame's presses for Escape pressed twice. `None` when the frame has no Escape press, or
/// has another key or pointer button press as well (a key repeat counts as another press, and the
/// modifier keys do not count). Otherwise, whether the last counted Escape came less than
/// [`ESCAPE_PAIR_SECONDS`] before. A press of Escape, another key or a pointer button removes the
/// last counted Escape's time.
pub(super) fn escape_press(ctx: &egui::Context) -> Option<bool> {
    let (escape, other, now) = ctx.input(|i| {
        let (mut escape, mut other) = (false, false);
        for e in &i.events {
            match e {
                egui::Event::Key { key: Key::Escape, pressed: true, repeat: false, .. } => escape = true,
                egui::Event::Key { key, pressed: true, .. } if !is_modifier(*key) => other = true,
                egui::Event::PointerButton { pressed: true, .. } => other = true,
                _ => {}
            }
        }
        (escape, other, i.time)
    });
    if !escape && !other {
        return None;
    }
    let counted = ctx.data_mut(|d| d.remove_temp::<f64>(escape_pair_id()));
    (escape && !other).then(|| counted.is_some_and(|t| now - t < ESCAPE_PAIR_SECONDS))
}

/// Does this Escape reset the view's rotation? It does when the view is rotated, no popup is open,
/// the ruler origin is not being dragged, and the press is the second of a pair (`second`) or the
/// Rotate View tool is chosen.
pub(super) fn escape_resets_rotation(app: &VectorcraftApp, ctx: &egui::Context, second: bool) -> bool {
    app.view().is_some_and(|v| v.rotation != 0.0)
        && (second || app.session.tool_id() == "rotateView")
        && !egui::Popup::is_any_open(ctx)
        && !crate::canvas::ruler_origin_dragging(ctx)
}

/// The keys in Presentation Mode, where nothing is edited: Escape leaves the mode, and ← and → go to
/// the previous and the next artboard ([`presentation_step`]). Every other key press, the text it
/// types and the clipboard keys are removed from the input, so no shortcut, tool or field acts on
/// them. The macOS menu bar's key equivalents arrive as key presses too ([`crate::native_menu`]).
pub(super) fn presentation_keys(app: &mut VectorcraftApp, ctx: &egui::Context, escape_pair: Option<bool>) {
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
        // The Escape that left the mode can start a pair, as in `handle`.
        if escape_pair.is_some() && !egui::Popup::is_any_open(ctx) {
            let now = ctx.input(|i| i.time);
            ctx.data_mut(|d| d.insert_temp(escape_pair_id(), now));
        }
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
