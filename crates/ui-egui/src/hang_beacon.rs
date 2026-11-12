//! Where the interface thread is in its frame, for the desktop app's hang report (the app's
//! `hang_watch` module).
//!
//! The interface thread marks each stage of a frame (the host's input hook, the app's logic, the
//! interface, and the hand-off to the renderer), a system clipboard call in progress, whether any
//! window's pass had input ([`watch_input`]), and each command it runs. Another thread reads the
//! marks without waiting for the interface thread: the stage and the activity are atomics, and the
//! recent commands and frame times are read with `try_lock`. Marking costs a few atomic stores a
//! frame and one short lock per command and per frame. The web build marks the same way, and
//! nothing reads the marks there.
//!
//! Commands are kept by id only: no parameters, file names or document contents. A command run
//! again moves to the end of the list instead of taking another place, so a panel that runs a
//! query every frame does not push out the commands before it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use vectorcraft_engine::cmd::clipboard::Flavour;

use crate::sysclip::{ClipboardRead, SystemClipboard};

/// How many different command ids [`Beacon::recent_commands`] keeps.
pub const RECENT_COMMANDS: usize = 8;
/// How many frames [`Beacon::recent_frames`] keeps.
pub const RECENT_FRAMES: usize = 16;
/// The longest command id [`Beacon::command`] keeps as it is.
pub const MAX_COMMAND_ID: usize = 64;

/// The marks the desktop app's interface thread writes.
pub static BEACON: Beacon = Beacon::new();

/// The part of a frame the interface thread is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Stage {
    /// Outside the app's own frame code: waiting for input or a repaint, or a frame without input
    /// handed to the renderer.
    Idle = 0,
    /// The host's input hook.
    Input = 1,
    /// The app's logic: background results, timers, the control channel.
    Logic = 2,
    /// The interface: input handling, commands, panels and dialogs.
    Interface = 3,
    /// A frame that had input was handed to the renderer. Drawing and presenting it follow, and
    /// then the next frame, which input asks for at once.
    Drawing = 4,
}

impl Stage {
    fn from_u8(n: u8) -> Stage {
        match n {
            1 => Stage::Input,
            2 => Stage::Logic,
            3 => Stage::Interface,
            4 => Stage::Drawing,
            _ => Stage::Idle,
        }
    }

    /// Whether the interface thread is expected to finish this stage soon.
    pub fn busy(self) -> bool {
        self != Stage::Idle
    }

    /// The stage for a report, in English.
    pub fn describe(self) -> &'static str {
        match self {
            Stage::Idle => "between frames",
            Stage::Input => "reading the window's input",
            Stage::Logic => "running the app's logic (background results, timers, the control channel)",
            Stage::Interface => "running the interface (input handling, commands, panels and dialogs)",
            Stage::Drawing => {
                "after a frame with input went to the renderer: drawing or presenting it, or waiting for the next frame, which the system may hold back while the window is hidden"
            }
        }
    }
}

/// What the interface thread is waiting on inside a stage, where the app knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Activity {
    None = 0,
    /// A read of the system clipboard, or a check of what it holds.
    ClipboardRead = 1,
    /// A write to the system clipboard.
    ClipboardWrite = 2,
    /// A system file dialog shown in line, which waits for the user and keeps the window
    /// answering the system ([`Sample::busy`]).
    Dialog = 3,
    /// A menu of the system menu bar is open, which waits for the user ([`Sample::busy`]). The
    /// host sets it on the sample it reads; the interface thread does not mark it.
    Menu = 4,
}

impl Activity {
    fn from_u8(n: u8) -> Activity {
        match n {
            1 => Activity::ClipboardRead,
            2 => Activity::ClipboardWrite,
            3 => Activity::Dialog,
            4 => Activity::Menu,
            _ => Activity::None,
        }
    }

    /// Whether the activity waits for the user, so a long one is not a stall.
    pub fn waits_for_user(self) -> bool {
        matches!(self, Activity::Dialog | Activity::Menu)
    }

    /// The activity for a report, in English; `None` when there is none.
    pub fn describe(self) -> Option<&'static str> {
        match self {
            Activity::None => None,
            Activity::ClipboardRead => Some("reading the system clipboard"),
            Activity::ClipboardWrite => Some("writing to the system clipboard"),
            Activity::Dialog => Some("a system file dialog, waiting for the user"),
            Activity::Menu => Some("a menu of the system menu bar, waiting for the user"),
        }
    }
}

/// What the interface thread was doing when it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Counts every mark: the same value in two samples means the thread made no mark between them.
    pub step: u64,
    pub stage: Stage,
    pub activity: Activity,
}

impl Sample {
    /// Whether the interface thread is expected to move on soon: inside a stage, and not showing a
    /// system dialog or menu that waits for the user.
    pub fn busy(&self) -> bool {
        self.stage.busy() && !self.activity.waits_for_user()
    }
}

/// One frame's times, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTime {
    /// The app's own work: from the input hook to the end of the interface.
    pub work_ms: f32,
    /// The renderer's measure of the frame before: the app's work and drawing, without waiting for
    /// the display. `None` when the host has none.
    pub previous_total_ms: Option<f32>,
}

/// The marks: see the module documentation.
pub struct Beacon {
    step: AtomicU64,
    stage: AtomicU8,
    activity: AtomicU8,
    documents: AtomicUsize,
    /// A pass of some window had input since [`Beacon::take_input`] last ran.
    input: AtomicBool,
    /// The windows a pass handed to the system since [`Beacon::take_handed_off`] last ran. Only the
    /// interface thread uses it.
    handed_off: Mutex<Vec<egui::ViewportId>>,
    commands: Mutex<VecDeque<String>>,
    frames: Mutex<VecDeque<FrameTime>>,
}

impl Default for Beacon {
    fn default() -> Self {
        Self::new()
    }
}

impl Beacon {
    pub const fn new() -> Beacon {
        Beacon {
            step: AtomicU64::new(0),
            stage: AtomicU8::new(Stage::Idle as u8),
            activity: AtomicU8::new(Activity::None as u8),
            documents: AtomicUsize::new(0),
            input: AtomicBool::new(false),
            handed_off: Mutex::new(Vec::new()),
            commands: Mutex::new(VecDeque::new()),
            frames: Mutex::new(VecDeque::new()),
        }
    }

    fn bump(&self) {
        self.step.fetch_add(1, Ordering::Release);
    }

    /// The interface thread enters `stage`.
    pub fn enter(&self, stage: Stage) {
        self.stage.store(stage as u8, Ordering::Relaxed);
        self.bump();
    }

    /// The interface thread starts `activity`; it ends when the guard is dropped, and the activity
    /// before it applies again.
    pub fn activity(&self, activity: Activity) -> ActivityGuard<'_> {
        let before = Activity::from_u8(self.activity.swap(activity as u8, Ordering::Relaxed));
        self.bump();
        ActivityGuard { beacon: self, before }
    }

    /// The interface thread runs the command `id`. An id that is not a command id's shape (longer
    /// than [`MAX_COMMAND_ID`] characters, or with characters other than ASCII letters, digits, `.`,
    /// `_` and `-`) is kept as `(other)`: ids can come from saved files and the control channel. An
    /// id already in the list moves to its end.
    pub fn command(&self, id: &str) {
        self.bump();
        let shaped = id.len() <= MAX_COMMAND_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        let kept = if shaped { id } else { "(other)" };
        let mut commands = self.commands.lock().unwrap_or_else(PoisonError::into_inner);
        if commands.back().is_some_and(|last| last == kept) {
            return;
        }
        let entry = match commands.iter().position(|c| c == kept) {
            Some(i) => commands.remove(i).unwrap_or_else(|| kept.to_string()),
            None => {
                if commands.len() == RECENT_COMMANDS {
                    commands.pop_front();
                }
                kept.to_string()
            }
        };
        commands.push_back(entry);
    }

    /// A pass of some window had input ([`watch_input`]).
    pub fn input(&self) {
        self.input.store(true, Ordering::Relaxed);
    }

    /// Whether a pass of some window had input since the last call.
    pub fn take_input(&self) -> bool {
        self.input.swap(false, Ordering::Relaxed)
    }

    /// A pass of window `id` sent it commands that hand it to the system ([`hands_off`],
    /// [`watch_input`]).
    pub fn hand_off(&self, id: egui::ViewportId) {
        let mut windows = self.handed_off.lock().unwrap_or_else(PoisonError::into_inner);
        if !windows.contains(&id) {
            windows.push(id);
        }
    }

    /// The windows a pass handed to the system since the last call.
    pub fn take_handed_off(&self) -> Vec<egui::ViewportId> {
        std::mem::take(&mut *self.handed_off.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// A frame ended with `time`, with `documents` documents open.
    pub fn frame(&self, time: FrameTime, documents: usize) {
        self.documents.store(documents, Ordering::Relaxed);
        let mut frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        if frames.len() == RECENT_FRAMES {
            frames.pop_front();
        }
        frames.push_back(time);
    }

    /// The current marks.
    pub fn sample(&self) -> Sample {
        let step = self.step.load(Ordering::Acquire);
        Sample { step, stage: Stage::from_u8(self.stage.load(Ordering::Relaxed)), activity: Activity::from_u8(self.activity.load(Ordering::Relaxed)) }
    }

    /// The ids of the last [`RECENT_COMMANDS`] commands, oldest first; `None` while the interface
    /// thread is writing them.
    pub fn recent_commands(&self) -> Option<Vec<String>> {
        let commands = match self.commands.try_lock() {
            Ok(c) => c,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        Some(commands.iter().cloned().collect())
    }

    /// The last [`RECENT_FRAMES`] frames' times, oldest first; `None` while the interface thread is
    /// writing them.
    pub fn recent_frames(&self) -> Option<Vec<FrameTime>> {
        let frames = match self.frames.try_lock() {
            Ok(f) => f,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        Some(frames.iter().copied().collect())
    }

    /// The documents open at the end of the last frame.
    pub fn documents(&self) -> usize {
        self.documents.load(Ordering::Relaxed)
    }
}

/// Whether a pass's input asks for the next frame at once: events, or files dropped or hovered.
pub fn had_input(raw: &egui::RawInput) -> bool {
    !raw.events.is_empty() || !raw.dropped_files.is_empty() || !raw.hovered_files.is_empty()
}

/// Mark on `beacon` every pass of every window of `ctx` that had input ([`had_input`]): the main
/// window's, after the host's input hook added its own, and those of the windows shown in line
/// (floating panels and documents), whose input does not pass through that hook. Mark as well every
/// pass of a window shown in line that sends its window commands that hand it to the system
/// ([`hands_off`]), before egui passes them on; the main window's are read at the frame's end.
pub fn watch_input(beacon: &'static Beacon, ctx: &egui::Context) {
    ctx.on_begin_pass(
        "VectorCraft hang report input",
        std::sync::Arc::new(move |ui| {
            if ui.ctx().input(|i| had_input(&i.raw)) {
                beacon.input();
            }
        }),
    );
    ctx.on_end_pass(
        "VectorCraft hang report hand-off",
        std::sync::Arc::new(move |ui| {
            if ui.ctx().viewport_id() != egui::ViewportId::ROOT && ui.ctx().viewport(|vp| hands_off(&vp.commands)) {
                beacon.hand_off(ui.ctx().viewport_id());
            }
        }),
    );
}

/// Whether `commands` minimize or hide their window, or hand it to the system to move or resize:
/// the system may hold back the next frame until the user is done.
pub fn hands_off(commands: &[egui::ViewportCommand]) -> bool {
    use egui::ViewportCommand as C;
    commands.iter().any(|c| matches!(c, C::Minimized(true) | C::Visible(false) | C::StartDrag | C::BeginResize(_)))
}

/// The window whose end of frame sets the stage ([`after_frame`]): the main window when it is
/// `main_shown`; otherwise a window shown in line (a floating panel or document) that `viewports`
/// (the main window's input) gives as focused and not minimized; otherwise none.
pub fn shown_window(main_shown: bool, viewports: &egui::ViewportIdMap<egui::ViewportInfo>) -> Option<egui::ViewportId> {
    if main_shown {
        return Some(egui::ViewportId::ROOT);
    }
    viewports.iter().find(|(id, v)| **id != egui::ViewportId::ROOT && v.focused == Some(true) && v.minimized != Some(true)).map(|(id, _)| *id)
}

/// The stage at the end of a frame: [`Stage::Drawing`] when a pass had `input` and a window is
/// `shown` (focused, not minimized or hidden: [`shown_window`]), since input asks for the next
/// frame at once; otherwise [`Stage::Idle`]. A frame that sends that window `commands` that
/// minimize or hide it, or hand it to the system to move or resize ([`hands_off`]), also ends idle.
pub fn after_frame(input: bool, shown: bool, commands: &[egui::ViewportCommand]) -> Stage {
    if input && shown && !hands_off(commands) { Stage::Drawing } else { Stage::Idle }
}

/// Ends an [`Activity`] when dropped ([`Beacon::activity`]).
pub struct ActivityGuard<'a> {
    beacon: &'a Beacon,
    before: Activity,
}

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.beacon.activity.store(self.before as u8, Ordering::Relaxed);
        self.beacon.bump();
    }
}

/// A [`SystemClipboard`] whose calls are marked on [`BEACON`] as clipboard activity. The desktop
/// app wraps the clipboard the interface thread uses; the background probe's own clipboard stays
/// unmarked.
pub struct Marked(pub Box<dyn SystemClipboard>);

impl SystemClipboard for Marked {
    fn write(&mut self, flavours: &[Flavour]) -> Result<(), String> {
        let _mark = BEACON.activity(Activity::ClipboardWrite);
        self.0.write(flavours)
    }

    fn holds_ours(&mut self) -> bool {
        let _mark = BEACON.activity(Activity::ClipboardRead);
        self.0.holds_ours()
    }

    fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour> {
        let _mark = BEACON.activity(Activity::ClipboardRead);
        self.0.read(mimes)
    }

    fn has(&mut self, mimes: &[&'static str]) -> bool {
        let _mark = BEACON.activity(Activity::ClipboardRead);
        self.0.has(mimes)
    }

    fn read_deferred(&mut self, mimes: &[&'static str]) -> Option<ClipboardRead> {
        let _mark = BEACON.activity(Activity::ClipboardRead);
        self.0.read_deferred(mimes)
    }

    fn change_count(&mut self) -> Option<u64> {
        let _mark = BEACON.activity(Activity::ClipboardRead);
        self.0.change_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mark_moves_the_step_and_the_stage_reads_back() {
        let b = Beacon::new();
        let first = b.sample();
        assert_eq!((first.stage, first.activity), (Stage::Idle, Activity::None));
        b.enter(Stage::Interface);
        let s = b.sample();
        assert!(s.step > first.step);
        assert_eq!(s.stage, Stage::Interface);
        b.command("edit.copy");
        assert!(b.sample().step > s.step);
        b.enter(Stage::Idle);
        assert!(!b.sample().stage.busy());
    }

    #[test]
    fn an_activity_ends_with_its_guard_and_restores_the_one_before() {
        let b = Beacon::new();
        {
            let _write = b.activity(Activity::ClipboardWrite);
            {
                let _read = b.activity(Activity::ClipboardRead);
                assert_eq!(b.sample().activity, Activity::ClipboardRead);
            }
            assert_eq!(b.sample().activity, Activity::ClipboardWrite);
        }
        assert_eq!(b.sample().activity, Activity::None);
    }

    #[test]
    fn only_the_last_commands_and_frames_are_kept_oldest_first() {
        let b = Beacon::new();
        for n in 0..RECENT_COMMANDS + 3 {
            b.command(&format!("cmd.{n}"));
        }
        let commands = b.recent_commands().unwrap_or_default();
        assert_eq!(commands.len(), RECENT_COMMANDS);
        assert_eq!(commands.first().map(String::as_str), Some("cmd.3"));
        assert_eq!(commands.last().map(String::as_str), Some("cmd.10"));
        for n in 0..RECENT_FRAMES + 2 {
            b.frame(FrameTime { work_ms: n as f32, previous_total_ms: None }, 2);
        }
        let frames = b.recent_frames().unwrap_or_default();
        assert_eq!(frames.len(), RECENT_FRAMES);
        assert_eq!(frames.first().map(|f| f.work_ms), Some(2.0));
        assert_eq!(b.documents(), 2);
    }

    #[test]
    fn a_command_run_every_frame_does_not_push_out_the_ones_before() {
        let b = Beacon::new();
        for n in 0..RECENT_COMMANDS - 2 {
            b.command(&format!("cmd.{n}"));
        }
        // Two panels that each run a query every frame.
        for _ in 0..100 {
            b.command("symbol.list");
            b.command("swatch.list");
        }
        b.command("cmd.0");
        let commands = b.recent_commands().unwrap_or_default();
        let want: Vec<String> = ["cmd.1", "cmd.2", "cmd.3", "cmd.4", "cmd.5", "symbol.list", "swatch.list", "cmd.0"].map(String::from).to_vec();
        assert_eq!(commands, want);
    }

    #[test]
    fn a_frame_with_input_ends_drawing_unless_the_window_is_hidden_or_handed_to_the_system() {
        use egui::ViewportCommand as C;
        assert_eq!(after_frame(true, true, &[]), Stage::Drawing);
        assert_eq!(after_frame(true, true, &[C::Focus]), Stage::Drawing);
        assert_eq!(after_frame(false, true, &[]), Stage::Idle);
        assert_eq!(after_frame(true, false, &[]), Stage::Idle);
        for c in [C::Minimized(true), C::Visible(false), C::StartDrag, C::BeginResize(egui::ResizeDirection::East)] {
            assert_eq!(after_frame(true, true, std::slice::from_ref(&c)), Stage::Idle, "{c:?}");
        }
    }

    #[test]
    fn input_is_events_or_files() {
        let mut raw = egui::RawInput::default();
        assert!(!had_input(&raw));
        raw.hovered_files.push(egui::HoveredFile::default());
        assert!(had_input(&raw));
        #[derive(Debug)]
        struct Dropped;
        impl egui::DroppedFile for Dropped {
            fn path(&self) -> &std::path::Path {
                std::path::Path::new("a.svg")
            }
            fn bytes(&self) -> Result<Vec<u8>, String> {
                Ok(Vec::new())
            }
        }
        let mut raw = egui::RawInput::default();
        raw.dropped_files.push(std::sync::Arc::new(Dropped));
        assert!(had_input(&raw));
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerGone);
        assert!(had_input(&raw));
    }

    #[test]
    fn input_in_a_window_shown_in_line_is_marked() {
        static B: Beacon = Beacon::new();
        // The host runs a window shown in line as its own pass, with that window's input.
        let child_events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events = child_events.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            let id = viewport.ids.this;
            let viewports = std::iter::once((id, egui::ViewportInfo::default())).collect();
            let input = egui::RawInput { viewport_id: id, viewports, events: events.take(), ..Default::default() };
            ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui)).textures_delta.clear();
        });
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        watch_input(&B, &ctx);
        let child = egui::ViewportId::from_hash_of("floating panel");
        let frame = |main: Vec<egui::Event>| {
            ctx.run_ui(egui::RawInput { events: main, ..Default::default() }, |ui| {
                ui.ctx().show_viewport_immediate(child, egui::ViewportBuilder::default(), |_, _| {});
            })
            .textures_delta
            .clear();
        };
        frame(Vec::new());
        assert!(!B.take_input(), "no input anywhere");
        child_events.replace(vec![egui::Event::PointerGone]);
        frame(Vec::new());
        assert!(B.take_input(), "the floating window's input");
        assert!(!B.take_input(), "taken");
        frame(vec![egui::Event::PointerGone]);
        assert!(B.take_input(), "the main window's input");
    }

    #[test]
    fn a_window_shown_in_line_handed_to_the_system_is_marked() {
        static B: Beacon = Beacon::new();
        egui::Context::set_immediate_viewport_renderer(|ctx, mut viewport| {
            let id = viewport.ids.this;
            let viewports = std::iter::once((id, egui::ViewportInfo::default())).collect();
            let input = egui::RawInput { viewport_id: id, viewports, ..Default::default() };
            ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui)).textures_delta.clear();
        });
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        watch_input(&B, &ctx);
        let child = egui::ViewportId::from_hash_of("floating panel");
        let frame = |command: Option<egui::ViewportCommand>, main: Option<egui::ViewportCommand>| {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.ctx().show_viewport_immediate(child, egui::ViewportBuilder::default(), |ui, _| {
                    if let Some(c) = command.clone() {
                        ui.ctx().send_viewport_cmd(c);
                    }
                });
                if let Some(c) = main.clone() {
                    ui.ctx().send_viewport_cmd(c);
                }
            })
            .textures_delta
            .clear();
        };
        frame(None, None);
        assert!(B.take_handed_off().is_empty(), "nothing handed off");
        frame(Some(egui::ViewportCommand::StartDrag), None);
        assert_eq!(B.take_handed_off(), [child], "the floating window starts a move");
        assert!(B.take_handed_off().is_empty(), "taken");
        frame(Some(egui::ViewportCommand::Focus), None);
        assert!(B.take_handed_off().is_empty(), "another command");
        frame(None, Some(egui::ViewportCommand::StartDrag));
        assert!(B.take_handed_off().is_empty(), "the main window's commands are read at the frame's end");
    }

    #[test]
    fn a_focused_window_shown_in_line_is_the_shown_window() {
        use egui::{ViewportId, ViewportIdMap, ViewportInfo};
        let child = ViewportId::from_hash_of("document window");
        let info = |focused: bool, minimized: bool| ViewportInfo { focused: Some(focused), minimized: Some(minimized), ..Default::default() };
        let map = |root: ViewportInfo, window: ViewportInfo| -> ViewportIdMap<ViewportInfo> {
            [(ViewportId::ROOT, root), (child, window)].into_iter().collect()
        };
        assert_eq!(shown_window(true, &map(info(true, false), info(false, false))), Some(ViewportId::ROOT));
        assert_eq!(shown_window(false, &map(info(false, false), info(true, false))), Some(child), "the document window has the focus");
        assert_eq!(shown_window(false, &map(info(false, false), info(true, true))), None, "minimized");
        assert_eq!(shown_window(false, &map(info(false, false), info(false, false))), None, "no window has the focus");
        assert_eq!(shown_window(false, &map(info(true, false), info(false, false))), None, "the main window is not shown");
    }

    #[test]
    fn the_marked_clipboard_marks_its_calls() {
        type Seen = std::sync::Arc<Mutex<Vec<Activity>>>;
        struct Probe(Seen);
        impl Probe {
            fn see(&self) {
                self.0.lock().unwrap().push(BEACON.sample().activity);
            }
        }
        impl SystemClipboard for Probe {
            fn write(&mut self, _: &[Flavour]) -> Result<(), String> {
                self.see();
                Ok(())
            }
            fn holds_ours(&mut self) -> bool {
                self.see();
                false
            }
            fn read(&mut self, _: &[&'static str]) -> Option<Flavour> {
                self.see();
                None
            }
            fn has(&mut self, _: &[&'static str]) -> bool {
                self.see();
                false
            }
        }
        let seen = Seen::default();
        let mut m = Marked(Box::new(Probe(seen.clone())));
        let _ = m.write(&[]);
        let _ = m.read(&[]);
        let _ = m.has(&[]);
        let _ = m.holds_ours();
        assert_eq!(BEACON.sample().activity, Activity::None);
        assert_eq!(*seen.lock().unwrap(), vec![Activity::ClipboardWrite, Activity::ClipboardRead, Activity::ClipboardRead, Activity::ClipboardRead]);
    }

    #[test]
    fn ids_that_are_not_command_ids_are_kept_as_other() {
        let b = Beacon::new();
        b.command("file.open");
        b.command("/home/someone/private plan.vectorcraft");
        b.command(&"y".repeat(MAX_COMMAND_ID));
        b.command(&"x".repeat(MAX_COMMAND_ID + 1));
        let commands = b.recent_commands().unwrap_or_default();
        // Both are `(other)`, which moves to the end the second time.
        assert_eq!(commands, vec!["file.open".to_string(), "y".repeat(MAX_COMMAND_ID), "(other)".into()]);
    }

    #[test]
    fn a_reader_does_not_wait_while_the_commands_are_being_written() {
        let b = Beacon::new();
        b.command("edit.paste");
        let held = b.commands.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(b.recent_commands(), None);
        drop(held);
        assert_eq!(b.recent_commands(), Some(vec!["edit.paste".to_string()]));
    }
}
