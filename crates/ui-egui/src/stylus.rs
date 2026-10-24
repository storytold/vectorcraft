//! Pen pressure that the host reads from the system itself (#764).
//!
//! winit 0.30 hands a pen's pressure to egui only on Windows, as the force of a touch event; the
//! web build makes the same touch events from the browser's Pointer Events. On Linux under X11
//! (XWayland included), winit reports a pen as a mouse, so the desktop app reads the pressure
//! from XInput2 on a connection of its own and writes it into a [`StylusFeed`]
//! ([`crate::Services::stylus`]). On macOS, where winit reports none either, the app reads each
//! event's pressure from AppKit before the window handles it, and also keeps it for that event's
//! pointer move ([`StylusFeed::set_move`]), so the moves of a fast stroke each get their own. The
//! canvas uses the feed when a frame's events carry no touch force.
//!
//! The feed also says which end of the pen moved. Turning the pen to its eraser end selects the
//! Eraser tool, and turning it back to the tip selects the tool used before
//! ([`sync_eraser_tool`]), as the reference app's help describes under "Erase parts of a path
//! using a stylus pen":
//! <https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/erase-parts-of-a-path.html>

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use crate::VectorcraftApp;

/// The tool the pen's eraser end selects.
const ERASER: &str = "eraser";

/// How long a press waits for the touch of a pen that hovers ([`StylusFeed::press`]).
const PRESS_WAIT: Duration = Duration::from_millis(5);

/// The pressures of pointer moves kept for the UI ([`StylusFeed::set_move`]); a frame replays 64
/// moves at most.
const MAX_MOVES: usize = 256;

/// What a pen reports as it moves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PenSample {
    /// 0..1; 0 while the pen hovers.
    pub pressure: f32,
    /// The pen is turned to its eraser end.
    pub eraser: bool,
}

/// What the feed holds.
#[derive(Clone, Debug, Default)]
struct Pen {
    /// The pressure of the pen when a pen was the last pointer to move.
    latest: Option<f32>,
    /// The pen that moved last is turned to its eraser end.
    eraser: bool,
    /// The highest pressure set since the UI last read the feed, counted from the latest sample
    /// whose pressure rose above 0.
    peak: Option<f32>,
    /// The last pressure above 0 set since a pen was the last pointer to move, with the same end
    /// (0 when that end has not touched).
    touched: f32,
    /// The pressure of each pointer move set since the UI last took them, oldest first.
    moves: VecDeque<f32>,
}

impl Pen {
    /// A pen hovers, and no touch has been set since the UI last read the feed.
    fn hovering(&self) -> bool {
        self.latest == Some(0.0) && self.peak.is_none()
    }
}

#[derive(Default)]
struct Shared {
    pen: Mutex<Pen>,
    /// Signaled on every [`StylusFeed::set_sample`] ([`StylusFeed::set`] included).
    set: Condvar,
    /// Written on the UI thread itself ([`StylusFeed::on_ui_thread`]): a press never waits.
    ui_thread: bool,
}

/// The pen pressure and pen end the host last read, written from any thread and read on the UI
/// thread. Cheap to clone; the clones share the value.
#[derive(Clone, Default)]
pub struct StylusFeed(Arc<Shared>);

impl StylusFeed {
    /// A feed the host writes on the UI thread, before the UI handles the event it read (macOS):
    /// a press has its touch already, and waiting for one would only stall the thread.
    pub fn on_ui_thread() -> Self {
        Self(Arc::new(Shared { ui_thread: true, ..Shared::default() }))
    }

    fn lock(&self) -> MutexGuard<'_, Pen> {
        self.0.pen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A pen turned to its tip moved with `pressure`, or `None`: another pointer moved
    /// ([`Self::set_sample`]).
    pub fn set(&self, pressure: Option<f32>) {
        self.set_sample(pressure.map(|pressure| PenSample { pressure, eraser: false }));
    }

    /// A pen moved with `sample`'s pressure (0..1; 0 while it hovers) and end, or `None`: another
    /// pointer, such as a mouse, moved. A pressure outside 0..1 is clamped, and one that is not a
    /// number is a mouse's, 1. A sample of the other end starts afresh: the touches of the end
    /// before it don't count for its presses.
    pub fn set_sample(&self, sample: Option<PenSample>) {
        let pressure = sample.map(|s| if s.pressure.is_finite() { s.pressure.clamp(0.0, 1.0) } else { 1.0 });
        let eraser = sample.is_some_and(|s| s.eraser);
        let mut pen = self.lock();
        if eraser != pen.eraser {
            *pen = Pen::default();
        }
        pen.eraser = eraser;
        match pressure {
            Some(p) if p > 0.0 => {
                let touching = pen.latest.is_some_and(|l| l > 0.0);
                pen.peak = Some(if touching { pen.peak.map_or(p, |q| q.max(p)) } else { p });
                pen.touched = p;
            }
            Some(_) => {}
            None => {
                pen.peak = None;
                pen.touched = 0.0;
                pen.moves.clear();
            }
        }
        pen.latest = pressure;
        drop(pen);
        self.0.set.notify_all();
    }

    /// Like [`Self::set_sample`], for a pen sample that comes with one pointer move of the window,
    /// read before the window handles the move (macOS): its pressure is also kept for that move
    /// ([`Self::take_moves`]).
    pub fn set_move(&self, sample: PenSample) {
        self.set_sample(Some(sample));
        let mut pen = self.lock();
        if pen.moves.len() >= MAX_MOVES {
            pen.moves.pop_front();
        }
        let pressure = pen.latest.unwrap_or(1.0);
        pen.moves.push_back(pressure);
    }

    /// The pressures of the pointer moves set with [`Self::set_move`] since the last call, oldest
    /// first (256 at most): the last one is the latest move's.
    pub fn take_moves(&self) -> Vec<f32> {
        self.lock().moves.drain(..).collect()
    }

    /// The pressure of a press the UI holds (a drag and its release) when a pen was the last
    /// pointer to move, otherwise `None`: the latest pressure, or, when the pen has lifted (0), the
    /// last pressure it touched with. The lift can reach the feed before the UI handles the last
    /// moves and the release of the press, which then keep the pen's pressure, not the lift's 0.
    /// The pressures set so far count as read for [`Self::press`].
    pub fn held(&self) -> Option<f32> {
        let mut pen = self.lock();
        pen.peak = None;
        let touched = pen.touched;
        pen.latest.map(|l| if l > 0.0 { l } else { touched })
    }

    /// The pressure of a press when a pen was the last pointer to move, otherwise `None`: the
    /// highest pressure set since the last read, counted from the latest touch, or else the latest
    /// pressure. A quick tap can touch and lift before the frame that handles its press, which
    /// then gets the tap's pressure, not the lift's 0. The host's reader can also set the touch
    /// just after the UI handles the press: when the pen still hovers, the press waits up to 5 ms
    /// for it, and gets 0 if none comes.
    pub fn press(&self) -> Option<f32> {
        self.press_within(PRESS_WAIT)
    }

    fn press_within(&self, wait: Duration) -> Option<f32> {
        let mut pen = self.lock();
        if pen.hovering() && !self.0.ui_thread {
            pen = self.wait_for_touch(pen, wait);
        }
        let peak = pen.peak.take();
        pen.latest.map(|l| peak.unwrap_or(l))
    }

    /// Whether the pen is turned to its eraser end when a pen was the last pointer to move,
    /// otherwise `None`.
    pub fn eraser(&self) -> Option<bool> {
        let pen = self.lock();
        pen.latest.map(|_| pen.eraser)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn wait_for_touch<'a>(&'a self, pen: MutexGuard<'a, Pen>, wait: Duration) -> MutexGuard<'a, Pen> {
        match self.0.set.wait_timeout_while(pen, wait, |p| p.hovering()) {
            Ok((pen, _)) => pen,
            Err(e) => e.into_inner().0,
        }
    }

    /// The browser's main thread can't wait, and the web build sets no feed.
    #[cfg(target_arch = "wasm32")]
    fn wait_for_touch<'a>(&'a self, pen: MutexGuard<'a, Pen>, _wait: Duration) -> MutexGuard<'a, Pen> {
        pen
    }
}

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
    fn take_flip(&mut self, feed: Option<&StylusFeed>) -> Option<bool> {
        let eraser = feed?.eraser()?;
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
    let Some(eraser) = app.pen_end.take_flip(app.services.stylus.as_ref()) else { return false };
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

    #[test]
    fn the_feed_keeps_the_last_pressure_within_0_to_1() {
        let feed = StylusFeed::default();
        assert_eq!(feed.held(), None, "no pen until one moves");
        let shared = feed.clone();
        shared.set(Some(0.25));
        assert_eq!(feed.held(), Some(0.25), "the clones share the value");
        feed.set(Some(1.5));
        assert_eq!(feed.held(), Some(1.0));
        feed.set(Some(f32::NAN));
        assert_eq!(feed.held(), Some(1.0), "a pressure that is not a number is a mouse's");
        feed.set(None);
        feed.set(Some(-0.5));
        assert_eq!(feed.press_within(Duration::ZERO), Some(0.0));
        feed.set(None);
        assert_eq!((feed.held(), feed.press()), (None, None), "a mouse moved");
    }

    #[test]
    fn the_feed_keeps_the_eraser_end() {
        let feed = StylusFeed::default();
        assert_eq!(feed.eraser(), None, "no pen until one moves");
        feed.set_sample(Some(PenSample { pressure: 1.5, eraser: true }));
        assert_eq!((feed.eraser(), feed.held()), (Some(true), Some(1.0)), "clamped, and the eraser end kept");
        feed.set_sample(Some(PenSample { pressure: f32::NAN, eraser: true }));
        assert_eq!((feed.eraser(), feed.held()), (Some(true), Some(1.0)));
        feed.set(Some(0.25));
        assert_eq!((feed.eraser(), feed.held()), (Some(false), Some(0.25)), "the tip");
        feed.set_sample(Some(PenSample { pressure: 0.0, eraser: true }));
        assert_eq!(feed.eraser(), Some(true), "the eraser end hovering");
        feed.set_sample(None);
        assert_eq!(feed.eraser(), None, "a mouse moved");
    }

    #[test]
    fn the_feed_keeps_the_pressure_of_each_move() {
        let feed = StylusFeed::default();
        for p in [0.2, 1.5, 0.4] {
            feed.set_move(PenSample { pressure: p, eraser: false });
        }
        feed.set(Some(0.9));
        assert_eq!(feed.take_moves(), [0.2, 1.0, 0.4], "clamped; a sample without a move keeps none");
        assert_eq!((feed.take_moves(), feed.held()), (vec![], Some(0.9)), "taken once");
        for _ in 0..MAX_MOVES + 10 {
            feed.set_move(PenSample { pressure: 0.5, eraser: false });
        }
        assert_eq!(feed.take_moves().len(), MAX_MOVES, "bounded");
        feed.set_move(PenSample { pressure: 0.5, eraser: false });
        feed.set(None);
        assert!(feed.take_moves().is_empty(), "a mouse moved");
    }

    #[test]
    fn a_held_press_keeps_the_last_touch_when_the_lift_comes_first() {
        let feed = StylusFeed::default();
        feed.set(Some(0.0));
        assert_eq!(feed.held(), Some(0.0), "a pen that has not touched");
        // The pen moves and lifts before the UI handles the last move and the release.
        for p in [0.5, 0.6, 0.0] {
            feed.set(Some(p));
        }
        assert_eq!(feed.held(), Some(0.6));
        feed.set(Some(0.3));
        assert_eq!(feed.held(), Some(0.3), "a touch again: its own pressure");
        feed.set(None);
        assert_eq!(feed.held(), None, "a mouse moved");
        feed.set(Some(0.0));
        assert_eq!(feed.held(), Some(0.0), "the pen's touch before the mouse doesn't count");
    }

    #[test]
    fn a_turned_pen_presses_without_the_touches_of_the_other_end() {
        let feed = StylusFeed::default();
        // The tip touches and lifts where the UI holds no press, so the UI does not read the feed.
        for p in [0.0, 0.8, 0.0] {
            feed.set(Some(p));
        }
        feed.set_sample(Some(PenSample { pressure: 0.0, eraser: true }));
        assert_eq!(feed.held(), Some(0.0), "the eraser end has not touched");
        assert_eq!(feed.press_within(Duration::ZERO), Some(0.0), "nor pressed");
        feed.set_sample(Some(PenSample { pressure: 0.4, eraser: true }));
        feed.set_sample(Some(PenSample { pressure: 0.0, eraser: true }));
        feed.set(Some(0.0));
        assert_eq!((feed.held(), feed.press_within(Duration::ZERO)), (Some(0.0), Some(0.0)), "back to the tip");
    }

    #[test]
    fn a_press_takes_the_highest_pressure_since_the_last_read() {
        let feed = StylusFeed::default();
        feed.set(Some(0.0));
        assert_eq!(feed.press(), Some(0.0), "a hovering pen");
        // A tap: it touches, presses harder, and lifts before the frame that handles the press.
        for p in [0.4, 0.7, 0.3, 0.0] {
            feed.set(Some(p));
        }
        assert_eq!(feed.press(), Some(0.7));
        assert_eq!(feed.press(), Some(0.0), "the press read the tap: the latest pressure");
        // A new touch counts from its own pressure, not the touch before it.
        for p in [0.9, 0.0, 0.2, 0.5, 0.3] {
            feed.set(Some(p));
        }
        assert_eq!(feed.press(), Some(0.5));
        // Pressures the UI has read do not count for the next press.
        feed.set(Some(0.8));
        assert_eq!(feed.held(), Some(0.8));
        feed.set(Some(0.4));
        assert_eq!(feed.press(), Some(0.4));
        feed.set(None);
        assert_eq!(feed.press(), None, "a mouse presses with no pen pressure");
    }

    #[test]
    fn a_press_waits_for_the_touch_of_a_hovering_pen() {
        let feed = StylusFeed::default();
        feed.set(Some(0.0));
        let reader = feed.clone();
        let late = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            reader.set(Some(0.6));
        });
        assert_eq!(feed.press_within(Duration::from_secs(10)), Some(0.6), "the touch set while the press waited");
        late.join().unwrap();
        // No touch comes: the hover's 0 once the wait ends.
        feed.set(Some(0.0));
        let start = std::time::Instant::now();
        assert_eq!(feed.press_within(Duration::from_millis(30)), Some(0.0));
        assert!(start.elapsed() >= Duration::from_millis(30));
        // A pen that touched, or a mouse, does not wait.
        feed.set(Some(0.3));
        let start = std::time::Instant::now();
        assert_eq!(feed.press_within(Duration::from_secs(10)), Some(0.3));
        feed.set(None);
        assert_eq!(feed.press_within(Duration::from_secs(10)), None);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    /// A feed written on the UI thread (macOS) has nothing to wait for: a hovering pen's press
    /// gets its 0 at once rather than stalling the thread.
    #[test]
    fn a_feed_written_on_the_ui_thread_never_waits() {
        let feed = StylusFeed::on_ui_thread();
        feed.set(Some(0.0));
        let start = std::time::Instant::now();
        assert_eq!(feed.press_within(Duration::from_secs(10)), Some(0.0));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
