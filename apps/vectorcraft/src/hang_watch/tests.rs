use super::*;
use std::time::UNIX_EPOCH;
use vectorcraft_ui_egui::hang_beacon::{Activity, Stage};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn sample(step: u64, stage: Stage) -> Sample {
    Sample { step, stage, activity: Activity::None }
}

fn facts() -> Facts {
    let env = |name: &str| match name {
        "XDG_CURRENT_DESKTOP" => Some("KDE".to_string()),
        "XDG_SESSION_TYPE" => Some("wayland\u{7}".to_string()),
        _ => None,
    };
    Facts::new(env, true, "Wayland", Some("Test GPU (Vulkan, DiscreteGpu)".to_string()), surface(Some(("AutoVsync".to_string(), Some(1)))))
}

fn shared(previous: Option<String>) -> Shared {
    Shared { facts: facts(), started: Instant::now(), last: Mutex::new(None), previous }
}

#[test]
fn a_stage_held_for_the_stall_time_is_reported_once_and_its_end_after_it() {
    let mut w = Watch::new(STALL);
    assert_eq!(w.check(ms(0), sample(5, Stage::Interface)), None);
    assert_eq!(w.check(ms(2_999), sample(5, Stage::Interface)), None);
    assert_eq!(w.check(ms(3_000), sample(5, Stage::Interface)), Some(Event::Stalled(ms(3_000))));
    assert_eq!(w.check(ms(9_000), sample(5, Stage::Interface)), None);
    assert_eq!(w.check(ms(9_500), sample(6, Stage::Idle)), Some(Event::Answered(ms(9_500))));
    assert_eq!(w.check(ms(20_000), sample(6, Stage::Idle)), None);
}

#[test]
fn frames_that_keep_moving_and_an_idle_window_are_never_reported() {
    let mut w = Watch::new(STALL);
    // A long animation: the step changes at every read.
    for n in 0..100 {
        assert_eq!(w.check(ms(n * 500), sample(n, Stage::Drawing)), None);
    }
    // A window waiting for input for a minute.
    for n in 0..120 {
        assert_eq!(w.check(ms(50_000 + n * 500), sample(1_000, Stage::Idle)), None);
    }
}

#[test]
fn a_file_dialog_shown_in_line_is_waiting_for_the_user_not_stalled() {
    let mut w = Watch::new(STALL);
    let dialog = Sample { step: 7, stage: Stage::Interface, activity: Activity::Dialog };
    for n in 0..60 {
        assert_eq!(w.check(ms(n * 500), dialog), None);
    }
    // Back from the dialog, the command goes on and is timed again.
    assert_eq!(w.check(ms(30_000), sample(8, Stage::Interface)), None);
    assert_eq!(w.check(ms(33_000), sample(8, Stage::Interface)), Some(Event::Stalled(ms(3_000))));
}

#[test]
fn an_open_menu_bar_menu_is_waiting_for_the_user_not_stalled() {
    let mut w = Watch::new(STALL);
    let menu = Sample { step: 3, stage: Stage::Drawing, activity: Activity::Menu };
    for n in 0..60 {
        assert_eq!(w.check(ms(n * 500), menu), None);
    }
}

#[test]
fn a_new_busy_step_starts_the_time_again() {
    let mut w = Watch::new(STALL);
    assert_eq!(w.check(ms(0), sample(1, Stage::Logic)), None);
    assert_eq!(w.check(ms(2_500), sample(2, Stage::Interface)), None);
    assert_eq!(w.check(ms(5_000), sample(2, Stage::Interface)), None);
    assert_eq!(w.check(ms(5_500), sample(2, Stage::Interface)), Some(Event::Stalled(ms(3_000))));
    // A mark inside the stall (a clipboard read ending) is an answer.
    assert_eq!(w.check(ms(6_000), sample(3, Stage::Interface)), Some(Event::Answered(ms(3_500))));
}

#[test]
fn the_report_names_the_stage_the_activity_the_commands_and_the_platform() {
    let beacon = Beacon::new();
    beacon.command("edit.copy");
    beacon.command("edit.paste");
    beacon.frame(FrameTime { work_ms: 1.24, previous_total_ms: Some(3.5) }, 2);
    let s = Sample { step: 9, stage: Stage::Interface, activity: Activity::ClipboardRead };
    let text = report(UNIX_EPOCH + Duration::from_secs(1_794_385_145), ms(3_240), s, &beacon, Duration::from_secs(754), &facts());
    for want in [
        "VectorCraft hang report, 2026-11-11T08:19:05.000Z",
        "has not moved on for 3.2 s",
        "Stage: running the interface",
        "Waiting on: reading the system clipboard",
        "Recent commands, oldest first: edit.copy, edit.paste",
        "App work per frame (ms), oldest first: 1.2",
        "without waiting for the display (ms), oldest first: 3.5",
        "Session: running for 754 s, 2 documents open",
        "Window system: Wayland; desktop: KDE; session type: wayland; Flatpak: yes",
        "Graphics: Test GPU (Vulkan, DiscreteGpu); requested present mode AutoVsync, frame latency 1",
    ] {
        assert!(text.contains(want), "{want:?} missing from:\n{text}");
    }
}

#[test]
fn environment_values_lose_control_characters_and_are_cut_short() {
    let long = "x".repeat(500);
    let env = |name: &str| (name == "XDG_CURRENT_DESKTOP").then(|| format!("A\nB{long}"));
    let f = Facts::new(env, false, "X11", None, String::new());
    assert_eq!(f.desktop.as_deref().map(str::len), Some(MAX_VALUE_CHARS));
    assert!(f.desktop.as_deref().is_some_and(|d| d.starts_with("ABx")));
    assert!(!f.flatpak);
    assert!(f.lines().contains("Graphics: unknown"));
}

#[test]
fn a_stall_writes_the_report_file_and_its_end_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let shared = shared(None);
    let beacon = Beacon::new();
    let s = sample(4, Stage::Drawing);
    let (mut held, open) = (None, AtomicBool::new(false));
    record(Event::Stalled(ms(3_000)), s, &beacon, &shared, Some(dir.path()), &mut held, &open, UNIX_EPOCH);
    let path = report_path(dir.path());
    // Windows refuses reads through another handle while the file is locked: the length is read.
    let last = shared.last.lock().unwrap().clone().unwrap_or_default();
    assert!(last.contains("Stage: after a frame with input went to the renderer"));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), last.len() as u64);
    // While the stall lasts, a launch of another instance leaves the file alone.
    assert_eq!(take_previous(dir.path()), None);
    assert!(path.exists());
    record(Event::Answered(ms(4_000)), s, &beacon, &shared, Some(dir.path()), &mut held, &open, UNIX_EPOCH);
    assert!(!path.exists());
    // This session's report stays for Copy Troubleshooting Info.
    assert!(shared.last.lock().unwrap().is_some());
}

#[test]
fn closing_normally_removes_the_report_and_writes_no_more() {
    let dir = tempfile::tempdir().unwrap();
    let shared = shared(None);
    let beacon = Beacon::new();
    let (mut held, flag) = (None, AtomicBool::new(false));
    record(Event::Stalled(ms(3_000)), sample(4, Stage::Interface), &beacon, &shared, Some(dir.path()), &mut held, &flag, UNIX_EPOCH);
    let path = report_path(dir.path());
    assert!(path.exists());
    // The interface answered after a frame with input, and the app quit before the watching thread
    // looked again: saving and teardown are not timed.
    beacon.enter(Stage::Drawing);
    closing(Some(dir.path()), &flag, &beacon);
    assert_eq!(beacon.sample().stage, Stage::Idle);
    assert!(!path.exists());
    assert_eq!(take_previous(dir.path()), None);
    // The watching thread then sees the answer.
    record(Event::Answered(ms(3_500)), sample(5, Stage::Idle), &beacon, &shared, Some(dir.path()), &mut held, &flag, UNIX_EPOCH);
    assert!(held.is_none());
    // A stall the watching thread finds while the app closes goes to the log only.
    record(Event::Stalled(ms(3_000)), sample(9, Stage::Interface), &beacon, &shared, Some(dir.path()), &mut held, &flag, UNIX_EPOCH);
    assert!(!path.exists());
    assert!(held.is_none());
}

#[test]
fn the_next_launch_takes_a_left_report_once_and_keeps_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(take_previous(dir.path()), None);
    let left = dir.path().join(format!("{REPORT_PREFIX}4242.txt"));
    std::fs::write(&left, "VectorCraft hang report, then\n").unwrap();
    // Other files in the log folder are not reports.
    std::fs::write(dir.path().join(format!("{REPORT_PREFIX}notes.txt")), "x").unwrap();
    std::fs::write(dir.path().join(format!("{REPORT_PREFIX}17.txt")), "").unwrap();
    assert_eq!(take_previous(dir.path()).as_deref(), Some("VectorCraft hang report, then\n"));
    assert!(!left.exists());
    assert_eq!(std::fs::read_to_string(dir.path().join(PREVIOUS_FILE)).unwrap(), "VectorCraft hang report, then\n");
    assert_eq!(take_previous(dir.path()), None);
    // An empty file may be one being written: it stays.
    assert!(dir.path().join(format!("{REPORT_PREFIX}17.txt")).exists());
}

#[test]
fn of_several_left_reports_the_newest_is_kept_and_all_are_offered_oldest_first() {
    let dir = tempfile::tempdir().unwrap();
    for (pid, text, secs) in [(9, "newer\n", 2_000), (10, "older\n", 1_000)] {
        let path = dir.path().join(format!("{REPORT_PREFIX}{pid}.txt"));
        std::fs::write(&path, text).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
    }
    assert_eq!(take_previous(dir.path()).as_deref(), Some("older\n\nnewer\n"));
    assert_eq!(std::fs::read_to_string(dir.path().join(PREVIOUS_FILE)).unwrap(), "newer\n");
}

#[test]
fn a_report_that_cannot_be_moved_is_offered_once() {
    let dir = tempfile::tempdir().unwrap();
    // Something that is not a file is in the way.
    std::fs::create_dir_all(dir.path().join(PREVIOUS_FILE).join("x")).unwrap();
    let left = dir.path().join(format!("{REPORT_PREFIX}5.txt"));
    std::fs::write(&left, "kept\n").unwrap();
    assert_eq!(take_previous(dir.path()).as_deref(), Some("kept\n"));
    assert!(!left.exists());
    assert_eq!(take_previous(dir.path()), None);
}

#[test]
fn reports_beyond_the_newest_are_removed_not_left_for_a_later_launch() {
    let dir = tempfile::tempdir().unwrap();
    let count = MAX_REPORT_FILES + 3;
    for n in 0..count {
        let path = dir.path().join(format!("{REPORT_PREFIX}{}.txt", 100 + n));
        std::fs::write(&path, format!("report {n}\n")).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(1_000 + n as u64)).unwrap();
    }
    let text = take_previous(dir.path()).unwrap();
    assert!(text.starts_with("report 3\n"), "{text}");
    assert!(text.ends_with(&format!("report {}\n", count - 1)), "{text}");
    assert!(!text.contains("report 2\n"));
    assert_eq!(take_previous(dir.path()), None);
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok).map(|e| e.file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from(PREVIOUS_FILE)]);
}

#[test]
fn newer_empty_files_do_not_push_out_an_abandoned_report() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join(format!("{REPORT_PREFIX}1.txt"));
    std::fs::write(&old, "old\n").unwrap();
    std::fs::File::options().write(true).open(&old).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(1_000)).unwrap();
    for n in 0..MAX_REPORT_FILES + 1 {
        let path = dir.path().join(format!("{REPORT_PREFIX}{}.txt", 100 + n));
        std::fs::write(&path, "").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(2_000)).unwrap();
    }
    assert_eq!(take_previous(dir.path()).as_deref(), Some("old\n"));
}

#[test]
fn a_file_system_without_locks_does_not_hide_a_report() {
    use std::io::{Error, ErrorKind};
    assert!(abandoned(Ok(())));
    assert!(!abandoned(Err(std::fs::TryLockError::WouldBlock)));
    assert!(abandoned(Err(std::fs::TryLockError::Error(Error::from(ErrorKind::Unsupported)))));
    // Any other error may hide a live session's lock: the file is left alone.
    assert!(!abandoned(Err(std::fs::TryLockError::Error(Error::from(ErrorKind::PermissionDenied)))));
}

#[test]
fn an_oversized_report_file_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let big = vec![b'x'; usize::try_from(MAX_REPORT_BYTES).unwrap() + 1];
    let left = dir.path().join(format!("{REPORT_PREFIX}7.txt"));
    std::fs::write(&left, big).unwrap();
    let text = take_previous(dir.path()).unwrap();
    assert!(text.starts_with("The report was larger than"));
    assert!(!left.exists());
}

#[test]
fn troubleshooting_info_holds_both_sessions_reports() {
    let shared = shared(Some("VectorCraft hang report, last session\n".to_string()));
    *shared.last.lock().unwrap() = Some("VectorCraft hang report, this session\n".to_string());
    let beacon = Beacon::new();
    let text = troubleshooting(&shared, &beacon, UNIX_EPOCH);
    assert!(text.starts_with("VectorCraft troubleshooting info, 1970-01-01T00:00:00.000Z\n"));
    assert!(text.contains("This session's last hang report:\nVectorCraft hang report, this session\n"));
    assert!(text.contains("(it was closed while it did not answer):\nVectorCraft hang report, last session\n"));
    assert!(text.contains("Recent commands, oldest first: none"));
}
