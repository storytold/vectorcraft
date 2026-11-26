//! Which end of the pen is down (#764).
//!
//! The hosts read a pen's pressure, tilt, rotation and end from the system and write them into the
//! framework's [`PenSource`]: from XInput2 on X11 (XWayland included), from AppKit on macOS, from
//! each pointer message of a pen on Windows, and from Pointer Events in the web build. The tool
//! host reads the pressure, tilt and rotation from the same source.
//!
//! The source also says which end of the pen moved. Turning the pen to its eraser end selects the
//! Eraser tool, and turning it back to the tip selects the tool used before
//! ([`sync_eraser_tool`]), as the reference app's help describes under "Erase parts of a path
//! using a stylus pen":
//! <https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/erase-parts-of-a-path.html>
//!
//! The web build reports the eraser end only as a press of the pen's eraser button (`button` 5), so
//! in a browser the end changes when the pen presses: a press of the eraser end selects the Eraser
//! before that press, and a press of the tip selects the tool used before.

use craft::ui::runtime::pen::PenSource;

use crate::VectorcraftApp;

/// The tool the pen's eraser end selects.
const ERASER: &str = "eraser";

/// The end of the pen the canvas saw last, and the tool the eraser end stands in for
/// ([`sync_eraser_tool`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct PenEnd {
    /// `Some(true)`: the eraser end; `Some(false)`: the tip; `None`: no pen seen yet.
    end: Option<bool>,
    /// The tool that was active when the eraser end selected the Eraser.
    before: Option<&'static str>,
}

impl PenEnd {
    /// A tool was chosen (the toolbar, a shortcut, a menu, `tool.select`): the tip no longer
    /// brings back the tool from before the eraser end.
    pub(crate) fn forget_tool(&mut self) {
        self.before = None;
    }

    /// The end the pen turned to since the last call (`Some(true)`: the eraser end), or `None`
    /// when it has not turned, or when another pointer moved last. Each turn is reported once.
    fn take_flip(&mut self, pen: Option<&PenSource>) -> Option<bool> {
        let eraser = pen?.current()?.eraser;
        let flipped = self.end.map_or(eraser, |e| e != eraser);
        self.end = Some(eraser);
        flipped.then_some(eraser)
    }
}

/// Select the Eraser tool when the pen turns to its eraser end, and select the tool active before
/// it when the pen turns back to its tip while the Eraser is still active. The Eraser chosen
/// before the eraser end came in stays when the tip comes back. Returns whether the tool changed.
///
/// The canvas calls this once a frame while no press is held and the pointer is in the window, so
/// a turn during a press takes effect after the release, and a turn over another window takes
/// effect when the pointer comes in.
pub(crate) fn sync_eraser_tool(app: &mut VectorcraftApp) -> bool {
    let Some(eraser) = app.pen_end.take_flip(app.services.pen.as_ref()) else { return false };
    let tool = app.session.tool_id();
    if eraser {
        if tool == ERASER {
            return false;
        }
        app.select_tool(ERASER);
        // After `select_tool`, which forgets the tool before.
        let selected = app.session.tool_id() == ERASER;
        app.pen_end.before = selected.then_some(tool);
        selected
    } else {
        match app.pen_end.before.take() {
            Some(before) if tool == ERASER => {
                app.return_to_tool(before);
                app.session.tool_id() == before
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::behavior::PointerSample;
    use vectorcraft_engine::Session;

    fn reading(eraser: bool) -> Option<PointerSample> {
        let mut sample = PointerSample::new(0.0);
        sample.eraser = eraser;
        Some(sample)
    }

    #[test]
    fn turning_the_pen_swaps_to_the_eraser_and_back_after_the_press() {
        let pen = PenSource::default();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.pen = Some(pen.clone());
        app.session.execute("file.new", &serde_json::json!({"width": 800, "height": 600})).unwrap();
        app.select_tool("paintbrush");

        pen.set(reading(false));
        assert!(!sync_eraser_tool(&mut app), "the tip hovers");
        pen.set(reading(true));
        assert!(sync_eraser_tool(&mut app));
        assert_eq!(app.session.tool_id(), ERASER);
        assert!(!sync_eraser_tool(&mut app), "a turn acts once");
        pen.set(reading(false));
        assert!(sync_eraser_tool(&mut app));
        assert_eq!(app.session.tool_id(), "paintbrush", "the tip brings back the tool before");

        // An Eraser chosen before the eraser end came stays when the tip comes back.
        app.select_tool(ERASER);
        pen.set(reading(true));
        assert!(!sync_eraser_tool(&mut app));
        pen.set(reading(false));
        assert!(!sync_eraser_tool(&mut app));
        assert_eq!(app.session.tool_id(), ERASER);

        // A tool chosen while the eraser end stands in cancels the return.
        app.select_tool("paintbrush");
        pen.set(reading(true));
        sync_eraser_tool(&mut app);
        app.select_tool("rectangle");
        pen.set(reading(false));
        assert!(!sync_eraser_tool(&mut app));
        assert_eq!(app.session.tool_id(), "rectangle");
    }
}
