//! A single-key tool shortcut held down (General › Hold a Tool Shortcut to Use the Tool
//! Temporarily, `holdToolKeys`): the tool it chose stays until the key is let go, and then the tool
//! active before comes back. A quick press chooses the tool for good.

use craft::ui::runtime::commands::{FirePhase, FireSource, Fired};

use super::VectorcraftApp;

/// A key let go this many seconds or more after its press returns to the tool active before it.
/// VectorCraft's own value.
pub(crate) const HOLD_SECONDS: f64 = 0.25;

/// A tool shortcut's key held down since it chose its tool.
pub(crate) struct HeldTool {
    /// The tool command the key fires; its release names the hold.
    command: String,
    /// The viewport that saw the press: other viewports do not see the key.
    viewport: egui::ViewportId,
    /// The tool the key chose.
    tool: &'static str,
    /// The tool to go back to.
    previous: &'static str,
    /// When the key went down (the fired press's input time).
    since: f64,
    /// The tool took a drag, a click or a dialog while the key was down.
    used: bool,
    /// When the key was let go. The decision can come in a later pass (after a drag, a dialog or a
    /// press of the same pass), and it uses this time.
    released: Option<f64>,
}

/// Chooses the tool of a fired tool key. A press that changes the tool, with the preference on, starts
/// a hold. A hold started while another key is held goes on top of it: letting go of the later key
/// returns to the earlier key's tool, and letting go of the earlier key then returns to the tool before
/// both. With the preference on, a key's repeats choose nothing: a key held down chooses its tool once.
/// With it off, each repeat chooses the tool again.
pub(super) fn choose(app: &mut VectorcraftApp, ctx: &egui::Context, fired: &Fired) {
    let Some(tool) = fired.command.id.strip_prefix("tool:") else { return };
    // Only a key is held: a click on a Tools button, a menu row or the palette chooses the tool and no more.
    let hold = app.session.prefs.hold_tool_keys && fired.source == FireSource::Key;
    if fired.phase == FirePhase::Repeat && hold {
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
    if hold && chosen == tool {
        let viewport = ctx.viewport_id();
        app.held_tool.retain(|h| h.viewport == viewport && h.command != fired.command.id);
        app.held_tool.push(HeldTool {
            command: fired.command.id.clone(),
            viewport,
            tool: chosen,
            previous,
            since: fired.time,
            used: false,
            released: None,
        });
    } else {
        app.held_tool.clear();
    }
}

/// A press that reaches the active tool, a drag of the Zoom, Hand or Rotate View tool, or the
/// tool's dialog opened with Enter, uses the tool of the key held last when that tool is active,
/// also when that and the key's release come between two passes of [`follow`].
pub(crate) fn note_tool_use(app: &mut VectorcraftApp) {
    let active = app.session.tool_id();
    if let Some(held) = app.held_tool.last_mut().filter(|h| h.tool == active) {
        held.used = true;
    }
}

/// The release of a tool key: times the hold it ended, then decides. A release that comes with the window
/// losing the keyboard goes to another window, so it ends every hold without going back.
pub(super) fn release(app: &mut VectorcraftApp, ctx: &egui::Context, fired: &Fired) {
    if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))) {
        app.held_tool.clear();
        return;
    }
    let viewport = ctx.viewport_id();
    for held in app.held_tool.iter_mut().filter(|h| h.viewport == viewport && h.released.is_none() && h.command == fired.command.id) {
        held.released = Some(fired.time);
    }
    follow(app, ctx);
}

/// When the key held last is let go, goes back to the tool active before it, unless the press was
/// quick and the tool was not used. A key let go during a drag, with the button held or with a dialog
/// open, goes back once that ends. Choosing another tool or turning the preference off ends every
/// hold without going back.
pub(super) fn follow(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // The keys go to another window: whatever release comes later, or none, ends this window's holds without going
    // back.
    if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))) {
        app.held_tool.retain(|h| h.viewport != ctx.viewport_id());
        return;
    }
    // Each step looks at the key held last; one that goes back hands over to the key under it.
    loop {
        let gesture = app.session.tool_busy() || app.session.pointer_held();
        // A dialog, or a canvas drag that does not reach the tool (a pan, zoom or turn of the view),
        // defers the decision.
        let dialog = app.any_dialog_open() || crate::canvas::in_gesture(app, ctx);
        let active = app.session.tool_id();
        let on = app.session.prefs.hold_tool_keys;
        let Some(held) = app.held_tool.last_mut() else { return };
        if !on {
            app.held_tool.clear();
            return;
        }
        if held.viewport != ctx.viewport_id() {
            return;
        }
        if gesture || dialog {
            // A gesture of the held tool with the key down uses the tool; a gesture lent to a
            // selection tool does not. A dialog uses it only when Enter opened it, and a view drag
            // when it is the held tool's ([`note_tool_use`]).
            held.used |= gesture && held.released.is_none() && active == held.tool;
            return;
        }
        if active != held.tool {
            app.held_tool.clear();
            return;
        }
        let Some(released) = held.released else { return };
        // The canvas takes a pointer press of the same pass, and the tool an Enter press, after
        // this: the decision waits for the next pass, measured from this one.
        let (pressed, now) = ctx.input(|i| {
            let pressed = i.events.iter().any(|e| {
                matches!(e, egui::Event::PointerButton { pressed: true, .. } | egui::Event::Key { key: egui::Key::Enter, pressed: true, .. })
            });
            (pressed, i.time)
        });
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
