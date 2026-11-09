//! A single-key tool shortcut held down (General › Hold a Tool Shortcut to Use the Tool
//! Temporarily, `holdToolKeys`): the tool it chose stays until the key is let go, and then the tool
//! active before comes back. A quick press chooses the tool for good.

use super::{Key, VectorcraftApp};

/// A key let go this many seconds or more after its press returns to the tool active before it.
/// VectorCraft's own value.
pub(crate) const HOLD_SECONDS: f64 = 0.25;

/// A tool shortcut's key held down since it chose its tool.
pub(crate) struct HeldTool {
    /// The key that chose the tool.
    key: Key,
    /// The viewport that saw the press: other viewports do not see the key.
    viewport: egui::ViewportId,
    /// The tool the key chose.
    tool: &'static str,
    /// The tool to go back to.
    previous: &'static str,
    /// When the key went down (`egui::InputState::time`).
    since: f64,
    /// The tool took a drag, a click or a dialog while the key was down.
    used: bool,
    /// When a frame first saw the key let go. The decision can come in a later frame (after a
    /// drag, a dialog or a press of the same frame), and it uses this time.
    released: Option<f64>,
}

/// Chooses `tool` for a press of its single-key shortcut `key` (`None`: no key press typed it). A
/// press that changes the tool, with the preference on, starts a hold. A hold started while another
/// key is held goes on top of it: letting go of the later key returns to the earlier key's tool,
/// and letting go of the earlier key then returns to the tool before both. With the preference on,
/// a key's repeats choose nothing: a key held down chooses its tool once. With it off, each repeat
/// chooses the tool again.
pub(super) fn choose(app: &mut VectorcraftApp, ctx: &egui::Context, tool: &str, key: Option<Key>, repeat: bool) {
    if repeat && app.session.prefs.hold_tool_keys {
        return;
    }
    let previous = app.session.tool_id();
    // `select_tool` ends the holds of a tool chosen any other way; the keys held stay held here.
    let holds = std::mem::take(&mut app.held_tool);
    app.select_tool(tool);
    app.held_tool = holds;
    let chosen = app.session.tool_id();
    if chosen == previous {
        return;
    }
    let viewport = ctx.viewport_id();
    let (down, now) = key.map_or((false, 0.0), |k| ctx.input(|i| (i.key_down(k), i.time)));
    match key {
        // A key let go in the frame that pressed it was a quick press.
        Some(key) if down && app.session.prefs.hold_tool_keys && chosen == tool => {
            app.held_tool.retain(|h| h.viewport == viewport && h.key != key);
            app.held_tool.push(HeldTool { key, viewport, tool: chosen, previous, since: now, used: false, released: None });
        }
        _ => app.held_tool.clear(),
    }
}

/// A press that reaches the active tool, a drag of the Zoom, Hand or Rotate View tool, or the
/// tool's dialog opened with Enter, uses the tool of the key held last when that tool is active,
/// also when that and the key's release come between two frames of [`follow`].
pub(crate) fn note_tool_use(app: &mut VectorcraftApp) {
    let active = app.session.tool_id();
    if let Some(held) = app.held_tool.last_mut().filter(|h| h.tool == active) {
        held.used = true;
    }
}

/// After the tool takes `key`: a dialog that Enter left open is the tool's, so it uses the tool.
pub(super) fn note_enter_dialog(app: &mut VectorcraftApp, key: Key) {
    if key == Key::Enter && app.ui.dialog.is_some() {
        note_tool_use(app);
    }
}

/// Once per frame of the viewport that saw the press: when the key held last is let go, goes back
/// to the tool active before it, unless the press was quick and the tool was not used. A key let
/// go during a drag, with the button held or with a dialog open, goes back once that ends. Choosing
/// another tool, a quick press, turning the preference off or the window losing the keyboard ends
/// every hold without going back. A frame of another viewport that has the keyboard also ends them.
pub(super) fn follow(app: &mut VectorcraftApp, ctx: &egui::Context, focused: bool) {
    // Every key let go is timed in the frame that first sees it up, under the key held last too.
    let (viewport, time) = (ctx.viewport_id(), ctx.input(|i| i.time));
    for held in app.held_tool.iter_mut().filter(|h| h.viewport == viewport && h.released.is_none()) {
        if !ctx.input(|i| i.key_down(held.key)) {
            held.released = Some(time);
        }
    }
    // Each pass looks at the key held last; one that goes back hands over to the key under it.
    loop {
        let gesture = app.session.tool_busy() || app.session.pointer_held();
        // A dialog, or a canvas drag that does not reach the tool (a pan, zoom or turn of the view),
        // defers the decision.
        let dialog = app.ui.dialog.is_some() || crate::canvas::dragging(ctx);
        let active = app.session.tool_id();
        let on = app.session.prefs.hold_tool_keys;
        let Some(held) = app.held_tool.last_mut() else { return };
        if !on {
            app.held_tool.clear();
            return;
        }
        if held.viewport != ctx.viewport_id() {
            // This viewport has the keyboard, so the viewport that saw the press lost it.
            if focused {
                app.held_tool.clear();
            }
            return;
        }
        if !focused {
            app.held_tool.clear();
            return;
        }
        let (down, now) = ctx.input(|i| (i.key_down(held.key), i.time));
        if gesture || dialog {
            // A gesture of the held tool with the key down uses the tool; a gesture lent to a
            // selection tool does not. A dialog uses it only when Enter opened it, and a view drag
            // when it is the held tool's ([`note_tool_use`]).
            held.used |= gesture && down && active == held.tool;
            return;
        }
        if active != held.tool {
            app.held_tool.clear();
            return;
        }
        if down {
            return;
        }
        // The canvas takes a pointer press of the same frame, and the tool an Enter press, after
        // this: the decision waits for the next frame, measured from this one.
        let pressed = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. } | egui::Event::Key { key: Key::Enter, pressed: true, .. }))
        });
        let released = held.released.unwrap_or(now);
        if pressed && released == now {
            ctx.request_repaint();
            return;
        }
        let back = held.used || released - held.since >= HOLD_SECONDS;
        let previous = held.previous;
        if !back {
            app.held_tool.clear();
            return;
        }
        app.held_tool.pop();
        app.return_to_tool(previous);
    }
}

#[cfg(test)]
mod tests;
