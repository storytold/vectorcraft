//! Touch gestures on the canvas (Preferences › Devices › Enable Touch Gestures, #585): a tap with
//! two fingers undoes, one with three redoes. In the Touch workspace a tap with two fingers
//! switches between selection and drawing modes instead ([`TOGGLE_MODE`]).
//!
//! The canvas recognizes the taps (`CanvasOutput::multi_tap`); [`tap_command`] names the command.
//!
//! Where the platform tells a pen from a finger (the web shell reads Pointer Events: an iPad with
//! an Apple Pencil), it says which drives the pointer ([`set_source`]). Once a pen has been seen in this session,
//! fingers only navigate: pinch, two-finger pan and the taps; a finger or a resting palm never
//! presses a tool on the canvas.

use std::cell::Cell;

/// What drives the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// A mouse or trackpad, or a platform that can't tell (the desktop apps).
    #[default]
    Mouse,
    /// A pen (Apple Pencil, a stylus).
    Pen,
    /// A finger on a touch screen.
    Finger,
}

thread_local! {
    static SOURCE: Cell<Source> = const { Cell::new(Source::Mouse) };
    static PEN_SEEN: Cell<bool> = const { Cell::new(false) };
}

/// The pointer's presses now come from `source`; a pen makes fingers navigate only from then on.
pub fn set_source(source: Source) {
    SOURCE.with(|s| s.set(source));
    if source == Source::Pen {
        PEN_SEEN.with(|p| p.set(true));
    }
}

/// Set the session-only pen policy (reload starts with finger drawing enabled).
pub fn set_pen_seen(seen: bool) {
    PEN_SEEN.with(|p| p.set(seen));
}

/// A pen has been seen.
pub fn pen_seen() -> bool {
    PEN_SEEN.with(Cell::get)
}

/// The press in progress is a finger's while a pen has been seen: it navigates, it doesn't draw.
pub(crate) fn finger_navigates_only() -> bool {
    SOURCE.with(Cell::get) == Source::Finger && pen_seen()
}

/// The command a tap of `fingers` runs: two undo, three redo. In the Touch workspace (`touch_workspace`)
/// two switch between the Selection tool and the last drawing tool ([`TOGGLE_MODE`]).
pub(crate) fn tap_command(fingers: usize, touch_workspace: bool) -> Option<&'static str> {
    match fingers {
        2 if touch_workspace => Some(TOGGLE_MODE),
        2 => Some("edit.undo"),
        3 => Some("edit.redo"),
        _ => None,
    }
}

/// Switch between selection and drawing modes: the Selection tool, and back to the drawing tool
/// chosen last.
pub const TOGGLE_MODE: &str = "tool.toggleTouchMode";

/// The drawing tool a switch out of selection mode chooses when none has been chosen in this window.
pub const FIRST_DRAWING_TOOL: &str = "pencil";

/// A tool of selection mode: one that selects objects or parts of them.
fn selects(id: &str) -> bool {
    vectorcraft_tools::catalog::is_selection_tool(id)
        || matches!(id, "magicWand" | "lasso" | "enclosingLasso" | "polygonalLasso" | "perspectiveSelection")
}

/// A tool of drawing mode: any tool but those of selection mode and those that only move, turn or
/// zoom the view.
pub(crate) fn is_drawing_tool(id: &str) -> bool {
    !selects(id) && !matches!(id, "hand" | "rotateView" | "zoom" | "printTiling")
}

/// [`TOGGLE_MODE`]: from a tool of selection mode, choose the drawing tool chosen last (none yet:
/// [`FIRST_DRAWING_TOOL`]); from any other tool, keep it as the drawing tool and choose the
/// Selection tool. → `{tool}`, the tool now active. A switch that fails returns its error and
/// leaves the drawing tool to return to as it was.
pub(crate) fn toggle_mode(app: &mut crate::VectorcraftApp) -> Result<serde_json::Value, String> {
    let now = app.session.tool_id().to_string();
    let (next, outgoing) = if selects(&now) {
        (app.ui.touch_drawing_tool.clone().unwrap_or_else(|| FIRST_DRAWING_TOOL.to_string()), None)
    } else {
        ("selection".to_string(), is_drawing_tool(&now).then_some(now))
    };
    app.try_select_tool(&next)?;
    if let Some(tool) = outgoing {
        app.ui.touch_drawing_tool = Some(tool);
    }
    Ok(serde_json::json!({"tool": app.session.tool_id()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingers_navigate_only_once_a_pen_was_seen() {
        set_pen_seen(false);
        set_source(Source::Finger);
        assert!(!finger_navigates_only(), "no pen yet: a finger draws");
        set_source(Source::Pen);
        assert!(pen_seen() && !finger_navigates_only());
        set_source(Source::Finger);
        assert!(finger_navigates_only());
        set_source(Source::Mouse);
        assert!(!finger_navigates_only(), "a mouse still draws");
        set_pen_seen(false);
    }
}
