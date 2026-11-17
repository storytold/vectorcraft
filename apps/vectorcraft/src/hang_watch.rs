//! The hang report. A thread reads the interface thread's marks
//! ([`vectorcraft_ui_egui::hang_beacon`]) every [`POLL`]. When the interface thread has made no
//! mark for [`STALL`] while inside a stage of its frame, the thread writes what the app knows to the
//! log (`logs/vectorcraft.log`) and to `logs/hang-report-<process id>.txt`: the stage, a clipboard
//! call in progress, the last commands, the last frames' times, the platform, the window system, the
//! graphics processor and the present mode the app asked for, and how long the session has run with
//! how many documents open. The process holds a lock on the file until the interface thread answers
//! again; then the log says after how long and the file is removed. A file nobody holds at a later
//! launch was left by a session that was closed while it did not answer, and a locked one belongs to
//! another instance that is stalled now and is left alone. That launch writes the report to the log,
//! moves the newest abandoned file to `hang-report.1.txt` and removes the others, says so in the
//! status bar, and Help › Copy Troubleshooting Info copies the reports. Where the file system has no
//! file locks, the file is written and taken without one.
//!
//! No native stack is recorded: the marks are what the report has. The report holds command ids and
//! counts, never document contents or file paths. [`STALL`] is VectorCraft's own value.
//!
//! A stall in the renderer is timed only after a frame that had input in any window: input asks for
//! the next frame at once, so a frame that does not start then is late. After a frame without
//! input, the window waiting for input looks the same as one stuck drawing, so that time is not
//! timed. Nor is the time after a frame that minimizes or hides the main window or hands it to the
//! system to move or resize. A window the system stops redrawing while it is hidden can still look
//! like a stall after a frame with input; the report's stage says so. A system file dialog shown in
//! line (on Windows and Linux) and an open menu of the macOS menu bar wait for the user and are not
//! timed.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

#[cfg(target_os = "macos")]
use vectorcraft_ui_egui::hang_beacon::Activity;
use vectorcraft_ui_egui::hang_beacon::{BEACON, Beacon, FrameTime, Sample, Stage};

use super::logging::timestamp;

/// How long the interface thread may stay in one stage before the report is written.
pub const STALL: Duration = Duration::from_secs(3);
/// How often the watching thread reads the marks.
pub const POLL: Duration = Duration::from_millis(500);
/// The report of a stall that has not ended, in the log folder: this, the process id, `.txt`.
pub const REPORT_PREFIX: &str = "hang-report-";
/// At most this many report files are taken when a launch looks for abandoned ones; older abandoned
/// ones are removed.
const MAX_REPORT_FILES: usize = 16;
/// At most this many entries of the log folder are looked at for report files.
const MAX_SCANNED_ENTRIES: usize = 1024;
/// The report the last session left, moved here at launch.
pub const PREVIOUS_FILE: &str = "hang-report.1.txt";
/// A report file larger than this is not read.
pub const MAX_REPORT_BYTES: u64 = 64 * 1024;
/// Environment values in the report are cut to this many characters.
const MAX_VALUE_CHARS: usize = 64;

/// What [`Watch::check`] found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// The interface thread has been in the sample's stage, without a mark, for this long.
    Stalled(Duration),
    /// The interface thread made a mark again after a [`Event::Stalled`], about this long after
    /// the stall began.
    Answered(Duration),
}

/// Decides from successive samples when the interface thread has stalled and when it answers
/// again. Time comes from the caller, so tests drive it without waiting.
#[derive(Debug)]
pub struct Watch {
    stall: Duration,
    /// The step seen in a busy stage and when it was first seen.
    seen: Option<(u64, Duration)>,
    reported: bool,
}

impl Watch {
    pub fn new(stall: Duration) -> Watch {
        Watch { stall, seen: None, reported: false }
    }

    /// The marks read at `now` (any monotonic clock that started before the first call).
    pub fn check(&mut self, now: Duration, sample: Sample) -> Option<Event> {
        let same = self.seen.is_some_and(|(step, _)| step == sample.step);
        if same && sample.busy() {
            let since = self.seen.map_or(now, |(_, since)| since);
            let stalled = now.saturating_sub(since);
            if !self.reported && stalled >= self.stall {
                self.reported = true;
                return Some(Event::Stalled(stalled));
            }
            return None;
        }
        let answered = match (self.reported, self.seen) {
            (true, Some((_, since))) => Some(Event::Answered(now.saturating_sub(since))),
            _ => None,
        };
        self.reported = false;
        self.seen = sample.busy().then_some((sample.step, now));
        answered
    }
}

/// What the report says about the platform.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// The window system the window opened on: Wayland, X11, AppKit, Win32, other or unknown.
    pub window_system: String,
    /// `XDG_CURRENT_DESKTOP`.
    pub desktop: Option<String>,
    /// `XDG_SESSION_TYPE`.
    pub session_type: Option<String>,
    /// Running in a Flatpak sandbox.
    pub flatpak: bool,
    /// The graphics processor the window draws with.
    pub graphics: Option<String>,
    /// The present mode and frame queue the app asked the window surface for.
    pub surface: String,
}

impl Facts {
    /// The facts from the environment (`env`, `flatpak_info`: whether `/.flatpak-info` exists) and
    /// what the app knows.
    pub fn new(env: impl Fn(&str) -> Option<String>, flatpak_info: bool, window_system: &str, graphics: Option<String>, surface: String) -> Facts {
        let value = |name: &str| env(name).map(|v| clean(&v)).filter(|v| !v.is_empty());
        Facts {
            window_system: window_system.to_string(),
            desktop: value("XDG_CURRENT_DESKTOP"),
            session_type: value("XDG_SESSION_TYPE"),
            flatpak: flatpak_info || env("FLATPAK_ID").is_some_and(|v| !v.is_empty()),
            graphics,
            surface,
        }
    }

    /// The facts as report lines.
    pub fn lines(&self) -> String {
        let or_unknown = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".to_string());
        format!(
            "VectorCraft {} on {} {}\nWindow system: {}; desktop: {}; session type: {}; Flatpak: {}\nGraphics: {}; {}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.window_system,
            or_unknown(&self.desktop),
            or_unknown(&self.session_type),
            if self.flatpak { "yes" } else { "no" },
            or_unknown(&self.graphics),
            self.surface,
        )
    }
}

/// `v` with control characters removed, cut to [`MAX_VALUE_CHARS`] characters.
fn clean(v: &str) -> String {
    v.chars().filter(|c| !c.is_control()).take(MAX_VALUE_CHARS).collect::<String>().trim().to_string()
}

/// The marks other than the stage: the last commands, the last frames and the session.
fn activity_lines(beacon: &Beacon, uptime: Duration) -> String {
    let commands = match beacon.recent_commands() {
        Some(c) if c.is_empty() => "none".to_string(),
        Some(c) => c.join(", "),
        None => "unavailable (being written)".to_string(),
    };
    let (work, total) = match beacon.recent_frames() {
        Some(frames) => {
            let list = |f: &dyn Fn(&FrameTime) -> Option<f32>| {
                let times: Vec<String> = frames.iter().filter_map(f).map(|ms| format!("{ms:.1}")).collect();
                if times.is_empty() { "none".to_string() } else { times.join(", ") }
            };
            (list(&|t| Some(t.work_ms)), list(&|t| t.previous_total_ms))
        }
        None => ("unavailable (being written)".to_string(), "unavailable (being written)".to_string()),
    };
    format!(
        "Recent commands, oldest first: {commands}\nApp work per frame (ms), oldest first: {work}\nWork and drawing per frame, without waiting for the display (ms), oldest first: {total}\nSession: running for {} s, {} documents open\n",
        uptime.as_secs(),
        beacon.documents(),
    )
}

/// The report of a stall that has lasted `stalled` in `sample`'s stage, written at `at`.
pub fn report(at: SystemTime, stalled: Duration, sample: Sample, beacon: &Beacon, uptime: Duration, facts: &Facts) -> String {
    let waiting = sample.activity.describe().map(|a| format!("Waiting on: {a}\n")).unwrap_or_default();
    format!(
        "VectorCraft hang report, {}\nThe interface thread has not moved on for {:.1} s.\nStage: {}\n{waiting}{}{}No native stack is recorded: the stage and the activity are what the app marks.\n",
        timestamp(at),
        stalled.as_secs_f64(),
        sample.stage.describe(),
        activity_lines(beacon, uptime),
        facts.lines(),
    )
}

/// What the watching thread and Help › Copy Troubleshooting Info share.
pub struct Shared {
    pub facts: Facts,
    pub started: Instant,
    /// This session's last report.
    pub last: Mutex<Option<String>>,
    /// The report the last session left ([`take_previous`]).
    pub previous: Option<String>,
}

/// The text Help › Copy Troubleshooting Info copies.
pub fn troubleshooting(shared: &Shared, beacon: &Beacon, at: SystemTime) -> String {
    let last = shared.last.lock().unwrap_or_else(PoisonError::into_inner).clone();
    format!(
        "VectorCraft troubleshooting info, {}\n{}{}\nThis session's last hang report:\n{}\nThe hang report the last session left (it was closed while it did not answer):\n{}\n",
        timestamp(at),
        shared.facts.lines(),
        activity_lines(beacon, shared.started.elapsed()),
        last.as_deref().unwrap_or("none\n"),
        shared.previous.as_deref().unwrap_or("none\n"),
    )
}

/// The report file of this process in `dir`.
fn report_path(dir: &Path) -> PathBuf {
    dir.join(format!("{REPORT_PREFIX}{}.txt", std::process::id()))
}

/// A report file this process wrote and holds locked until the interface answers again.
#[derive(Debug)]
pub struct Held {
    path: PathBuf,
    _file: File,
}

/// Write `text` to this process's report file in `dir` and lock it. Where the file system has no
/// file locks, the file is written unlocked and the log says so.
fn write_report(dir: &Path, text: &str) -> std::io::Result<Held> {
    std::fs::create_dir_all(dir)?;
    let path = report_path(dir);
    let mut file = File::create(&path)?;
    if let Err(e) = file.lock() {
        log::warn!("the hang report file is written without a lock: {e}");
    }
    file.write_all(text.as_bytes())?;
    Ok(Held { path, _file: file })
}

/// Whether a report file whose `try_lock` gave `locked` was abandoned: nobody holds it. A file
/// system without file locks answers [`std::io::ErrorKind::Unsupported`], and its files count as
/// abandoned; any other error leaves the file alone.
fn abandoned(locked: Result<(), std::fs::TryLockError>) -> bool {
    match locked {
        Ok(()) => true,
        Err(std::fs::TryLockError::Error(e)) => e.kind() == std::io::ErrorKind::Unsupported,
        Err(std::fs::TryLockError::WouldBlock) => false,
    }
}

/// The report file at `path`, opened and locked, with its length, when it was abandoned
/// ([`abandoned`]) and is not empty (an empty one may be being written).
fn open_abandoned(path: &Path) -> Option<(File, u64)> {
    let file = File::options().read(true).write(true).open(path).ok()?;
    if !abandoned(file.try_lock()) {
        return None;
    }
    let len = file.metadata().map_or(0, |m| m.len());
    (len > 0).then_some((file, len))
}

/// The reports that sessions closed while they did not answer left in `dir`, oldest first; `None`
/// when there are none. The newest file is moved to [`PREVIOUS_FILE`] and the others are removed
/// once read. Of more than [`MAX_REPORT_FILES`], the newest are taken and the older ones removed
/// unread; the first [`MAX_SCANNED_ENTRIES`] entries of `dir` are looked at. A report file another
/// process holds locked, or one still empty, is left alone. A file over [`MAX_REPORT_BYTES`] is
/// taken without being read. A file that can't be moved is removed, so no later launch offers it
/// again: the caller writes the reports to the log. Each file is opened once.
pub fn take_previous(dir: &Path) -> Option<String> {
    let mut found: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .take(MAX_SCANNED_ENTRIES)
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_prefix(REPORT_PREFIX)?.strip_suffix(".txt"))
                .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH), e.path()))
        .collect();
    found.sort();
    // Newest first: only abandoned reports count toward the limit.
    let mut texts = Vec::new();
    for (_, path) in found.into_iter().rev() {
        let Some((file, len)) = open_abandoned(&path) else { continue };
        if texts.len() == MAX_REPORT_FILES {
            drop(file);
            remove_quietly(&path);
            continue;
        }
        let mut bytes = Vec::new();
        let text = if len <= MAX_REPORT_BYTES && (&file).take(MAX_REPORT_BYTES).read_to_end(&mut bytes).is_ok() {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            format!("The report was larger than {MAX_REPORT_BYTES} bytes and was not read.\n")
        };
        // Closed (and unlocked) before it is moved: Windows moves no open file.
        drop(file);
        if !texts.is_empty() {
            remove_quietly(&path);
        } else if let Err(e) = std::fs::rename(&path, dir.join(PREVIOUS_FILE)) {
            log::warn!("couldn't move the hang report file to {PREVIOUS_FILE}: {e}");
            remove_quietly(&path);
        }
        texts.push(text);
    }
    texts.reverse();
    (!texts.is_empty()).then(|| texts.join("\n"))
}

/// Remove the report file at `path`; a failure goes to the log.
fn remove_quietly(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        log::warn!("couldn't remove the hang report file: {e}");
    }
}

/// Set when the app closes normally ([`closing`]): the watching thread writes no report file after it.
pub static CLOSING: AtomicBool = AtomicBool::new(false);

/// Remove `held`'s file, closing it first. → the error message; a file already gone is none.
fn remove(held: Held) -> Option<String> {
    let Held { path, _file: file } = held;
    drop(file);
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Some(e.to_string()),
        _ => None,
    }
}

/// Act on what [`Watch::check`] found: write (and hold) or remove this process's report file in
/// `dir`, then log it. The file comes first: logging can wait on what the interface thread waits on.
#[allow(clippy::too_many_arguments)]
pub fn record(
    event: Event,
    sample: Sample,
    beacon: &Beacon,
    shared: &Shared,
    dir: Option<&Path>,
    held: &mut Option<Held>,
    closing: &AtomicBool,
    at: SystemTime,
) {
    match event {
        Event::Stalled(stalled) => {
            let text = report(at, stalled, sample, beacon, shared.started.elapsed(), &shared.facts);
            let mut failed = None;
            if let Some(dir) = dir.filter(|_| !closing.load(Ordering::Acquire)) {
                match write_report(dir, &text) {
                    Ok(file) => *held = Some(file),
                    Err(e) => failed = Some(e.to_string()),
                }
                // The app began to close while the file was written: it is not a report to leave.
                if closing.load(Ordering::Acquire) {
                    failed = held.take().and_then(remove).or(failed);
                }
            }
            *shared.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(text.clone());
            log::warn!("the interface stopped answering:\n{}", text.trim_end());
            if let Some(e) = failed {
                log::warn!("couldn't write or remove the hang report file: {e}");
            }
        }
        Event::Answered(after) => {
            let failed = held.take().and_then(remove);
            log::warn!("the interface answered again about {:.1} s after it stopped", after.as_secs_f64());
            if let Some(e) = failed {
                log::warn!("couldn't remove the hang report file: {e}");
            }
        }
    }
}

/// The app is closing normally: mark the interface thread idle on `beacon` (saving and teardown are
/// not timed), remove this process's report file in `dir` and have the watching thread write no
/// more. It does not wait for the watching thread. The file can be removed while that thread holds
/// it open and locked: the standard library opens files with `FILE_SHARE_DELETE` on Windows, and a
/// byte-range lock does not prevent deletion. Where Windows deletes a file only once its last
/// handle closes, the name goes when the process ends.
pub fn closing(dir: Option<&Path>, closing: &AtomicBool, beacon: &Beacon) {
    beacon.enter(Stage::Idle);
    closing.store(true, Ordering::Release);
    if let Some(dir) = dir {
        match std::fs::remove_file(report_path(dir)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => log::warn!("couldn't remove the hang report file: {e}"),
            _ => {}
        }
    }
}

/// The window system `window` opened on, for [`Facts::window_system`].
pub fn window_system(window: Option<&winit::window::Window>) -> &'static str {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match window.and_then(|w| w.window_handle().ok()).map(|h| h.as_raw()) {
        Some(RawWindowHandle::Wayland(_)) => "Wayland",
        Some(RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)) => "X11",
        Some(RawWindowHandle::AppKit(_)) => "AppKit",
        Some(RawWindowHandle::Win32(_)) => "Win32",
        Some(_) => "other",
        None => "unknown",
    }
}

/// The present mode and frame queue the app asked the window surface for, for [`Facts::surface`];
/// `None` when the window has no wgpu renderer.
pub fn surface(config: Option<(String, Option<u32>)>) -> String {
    match config {
        Some((mode, Some(frames))) => format!("requested present mode {mode}, frame latency {frames}"),
        Some((mode, None)) => format!("requested present mode {mode}"),
        None => "no wgpu renderer".to_string(),
    }
}

/// Start the watching thread; the report file goes in `dir` (none: the log only).
pub fn start(dir: Option<PathBuf>, shared: Arc<Shared>) {
    let spawned = std::thread::Builder::new().name("hang-watch".into()).spawn(move || {
        let mut watch = Watch::new(STALL);
        let mut held = None;
        loop {
            std::thread::sleep(POLL);
            #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
            let mut sample = BEACON.sample();
            // An open menu of the menu bar waits for the user.
            #[cfg(target_os = "macos")]
            if super::mac_menu::is_tracking() {
                sample.activity = Activity::Menu;
            }
            if let Some(event) = watch.check(shared.started.elapsed(), sample) {
                record(event, sample, &BEACON, &shared, dir.as_deref(), &mut held, &CLOSING, SystemTime::now());
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("no hang report this session: the watching thread didn't start: {e}");
    }
}

#[cfg(test)]
mod tests;
