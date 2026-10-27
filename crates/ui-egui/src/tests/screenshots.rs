//! `ui.screenshot` after egui 0.37 replaced `Event::Screenshot`/`UserData` with a one-shot
//! `ScreenshotCallback`: exactly one file and reply per request, a late completion writes
//! nothing, admission is bounded before issue, a changed document or window geometry never saves
//! stale pixels, and a hidden window asks for the paint that reaches issuance.

use std::sync::Arc;

use egui::{Event, Pos2, Rect, ViewportCommand, ViewportId, ViewportInfo, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::control::ControlRequest;
use crate::{Services, VectorcraftApp};

const SCREEN: egui::Vec2 = vec2(320.0, 200.0);

/// Build a screenshot test app with a write service, control channel and temporary directory on a
/// 100 by 100 document.
fn app(tag: &str) -> (VectorcraftApp, std::sync::mpsc::Sender<ControlRequest>, std::path::PathBuf) {
    let dir = vectorcraft_testkit::temp_dir(tag);
    let services =
        Services { write: Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| e.to_string()))), ..Default::default() };
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), services).with_control(rx);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    (app, tx, dir)
}

/// Submit a `ui.screenshot` control request and return its reply receiver.
fn request(tx: &std::sync::mpsc::Sender<ControlRequest>, params: serde_json::Value) -> std::sync::mpsc::Receiver<serde_json::Value> {
    let (req, reply) = ControlRequest::new("ui.screenshot", params);
    tx.send(req).unwrap();
    reply
}

/// Run one tick-then-UI host frame at the requested screen size and discard texture deltas.
fn frame_at(app: &mut VectorcraftApp, ctx: &egui::Context, size: egui::Vec2, events: Vec<Event>) -> egui::FullOutput {
    let raw = egui::RawInput { events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    out
}

/// Run one host frame at the standard 320 by 200 screenshot size.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::FullOutput {
    frame_at(app, ctx, SCREEN, vec![])
}

/// The callback of the screenshot command this frame issued, if any.
fn issued(out: &egui::FullOutput) -> Option<egui::ScreenshotCallback> {
    let viewport = out.viewport_output.get(&ViewportId::ROOT)?;
    viewport.commands.iter().rev().find_map(|command| match command {
        ViewportCommand::Screenshot(callback) => Some(callback.clone()),
        _ => None,
    })
}

/// Make an admitted capture due now: the settle policy measures wall-clock animation time, which
/// a test must not race. The frame count in `issue_screenshots` still proves the settle passes.
fn allow_due(app: &mut VectorcraftApp) {
    for queued in &mut app.queued_screenshots {
        queued.due_ms = f64::MIN;
    }
}

/// Run full frames until the capture is issued (bounded: a failure is an error, not a hang).
fn drive_until_issued(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::ScreenshotCallback {
    for _ in 0..64 {
        let out = frame(app, ctx);
        if let Some(callback) = issued(&out) {
            return callback;
        }
    }
    panic!("the settled frame issues the capture");
}

/// Two full frames install and settle the interface fonts before the first request.
fn ready(app: &mut VectorcraftApp, ctx: &egui::Context) {
    frame(app, ctx);
    frame(app, ctx);
}

/// Drain an admitted request and make it due: a frame takes it from the control channel.
fn admit(app: &mut VectorcraftApp, ctx: &egui::Context) {
    frame(app, ctx);
    allow_due(app);
}

/// One admitted capture completes once, writes the named file, replies with its size and releases
/// the reservation.
#[test]
fn a_request_saves_one_file_and_replies_once() {
    let (mut app, tx, dir) = app("screenshot-one");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let path = dir.join("shot.png");
    let reply = request(&tx, json!({"path": path.to_string_lossy(), "data": true}));
    assert!(reply.try_recv().is_err(), "no reply before a frame is presented");
    admit(&mut app, &ctx);
    let callback = drive_until_issued(&mut app, &ctx);
    assert!(!path.exists(), "the callback writes nothing itself");
    callback.complete(Arc::new(egui::ColorImage::filled([320, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    let answer = reply.try_recv().expect("one reply");
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!((answer["result"]["width"].as_u64(), answer["result"]["height"].as_u64()), (Some(320), Some(200)));
    assert!(path.is_file());
    assert!(reply.try_recv().is_err(), "exactly one reply");
    assert_eq!(app.screenshot_reserved, 0, "the reservation is released");
    let _ = std::fs::remove_dir_all(dir);
}

/// A timed-out request replies once and writes nothing; a later completion is inert.
#[test]
fn a_timed_out_request_writes_nothing_and_a_late_completion_is_inert() {
    let (mut app, tx, dir) = app("screenshot-timeout");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let path = dir.join("late.png");
    let reply = request(&tx, json!({"path": path.to_string_lossy()}));
    admit(&mut app, &ctx);
    let callback = drive_until_issued(&mut app, &ctx);
    // The frame never came: the deadline passes and the request answers once.
    for pending in &mut app.pending_screenshots {
        pending.deadline_ms = -1.0;
    }
    app.begin_host_iteration();
    app.tick(&ctx);
    let answer = reply.try_recv().expect("the timeout reply");
    assert_eq!(answer["ok"], false, "{answer}");
    callback.complete(Arc::new(egui::ColorImage::filled([320, 200], egui::Color32::WHITE)));
    app.begin_host_iteration();
    app.tick(&ctx);
    assert!(!path.exists(), "a late completion writes nothing");
    assert!(reply.try_recv().is_err(), "no second reply");
    let _ = std::fs::remove_dir_all(dir);
}

/// A completion collected after the deadline is neither encoded nor written and replies once with
/// the no-frame error.
#[test]
fn a_completion_that_arrives_after_the_deadline_is_not_saved() {
    let (mut app, tx, dir) = app("screenshot-deadline");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let path = dir.join("too-late.png");
    let reply = request(&tx, json!({"path": path.to_string_lossy()}));
    admit(&mut app, &ctx);
    let callback = drive_until_issued(&mut app, &ctx);
    // The callback stored the image, but the UI thread collects it after the deadline.
    for pending in &mut app.pending_screenshots {
        pending.deadline_ms = -1.0;
    }
    callback.complete(Arc::new(egui::ColorImage::filled([320, 200], egui::Color32::WHITE)));
    app.begin_host_iteration();
    app.tick(&ctx);
    let answer = reply.try_recv().expect("the timeout reply");
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["error"].as_str().unwrap_or_default().contains("no frame was presented"), "{answer}");
    assert!(!path.exists(), "an image collected after the deadline is not encoded or written");
    assert!(reply.try_recv().is_err(), "no second reply");
    let _ = std::fs::remove_dir_all(dir);
}

/// Admission refuses a fifth request without evicting the four pending ones and releases capacity
/// when one completes.
#[test]
fn admission_rejects_more_than_four_requests_without_evicting_one_and_releases_a_finished_one() {
    let (mut app, tx, dir) = app("screenshot-admission");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let mut replies = vec![];
    for i in 0..5 {
        replies.push(request(&tx, json!({"path": dir.join(format!("{i}.png")).to_string_lossy()})));
    }
    frame(&mut app, &ctx);
    let last = replies.pop().unwrap().try_recv().expect("the fifth is refused at once");
    assert_eq!(last["ok"], false, "{last}");
    assert!(last["error"].as_str().unwrap_or_default().contains("already 4"), "{last}");
    for reply in &replies {
        assert!(reply.try_recv().is_err(), "the four admitted requests stay pending");
    }
    assert_eq!(app.pending_screenshots.len(), 4);
    // Complete the first: its reservation must come back, so a new request is admitted.
    allow_due(&mut app);
    let callback = drive_until_issued(&mut app, &ctx);
    callback.complete(Arc::new(egui::ColorImage::filled([320, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    assert_eq!(replies[0].try_recv().expect("the completed reply")["ok"], true);
    let more = request(&tx, json!({"path": dir.join("more.png").to_string_lossy()}));
    frame(&mut app, &ctx);
    assert!(more.try_recv().is_err(), "a released reservation admits the next request");
    assert_eq!(app.pending_screenshots.len(), 4);
    let _ = std::fs::remove_dir_all(dir);
}

/// Pixels with a different size are stale, produce no reply or file, and a retry with the matching
/// image completes the request.
#[test]
fn an_image_of_another_size_is_stale_and_retried() {
    let (mut app, tx, dir) = app("screenshot-size");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let path = dir.join("retry.png");
    let reply = request(&tx, json!({"path": path.to_string_lossy()}));
    admit(&mut app, &ctx);
    // An image of another size means the window changed under the capture: it is not saved.
    drive_until_issued(&mut app, &ctx).complete(Arc::new(egui::ColorImage::filled([300, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    assert!(reply.try_recv().is_err(), "no reply for the stale image");
    assert!(!path.exists());
    // The retry issues a fresh slot after the settle frames, and the matching image completes it.
    let second = drive_until_issued(&mut app, &ctx);
    second.complete(Arc::new(egui::ColorImage::filled([320, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    let answer = reply.try_recv().expect("the retry's reply");
    assert_eq!(answer["ok"], true, "{answer}");
    assert!(path.is_file());
    let _ = std::fs::remove_dir_all(dir);
}

/// Changing and restoring the DPI still invalidates in-flight pixels by epoch; the retry for the
/// current geometry succeeds.
#[test]
fn a_geometry_change_and_change_back_still_rejects_the_old_pixels() {
    let (mut app, tx, dir) = app("screenshot-epoch");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let path = dir.join("epoch.png");
    let reply = request(&tx, json!({"path": path.to_string_lossy()}));
    admit(&mut app, &ctx);
    let callback = drive_until_issued(&mut app, &ctx);
    // DPI changes and changes back before the callback arrives: the epoch, not value equality,
    // decides that the pixels are stale.
    ctx.set_pixels_per_point(2.0);
    frame(&mut app, &ctx);
    ctx.set_pixels_per_point(1.0);
    frame(&mut app, &ctx);
    callback.complete(Arc::new(egui::ColorImage::filled([320, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    assert!(reply.try_recv().is_err(), "no reply for pixels taken before the change back");
    assert!(!path.exists());
    // The retry issues for the current geometry and completes once.
    drive_until_issued(&mut app, &ctx).complete(Arc::new(egui::ColorImage::filled([320, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    let answer = reply.try_recv().expect("the retry's reply");
    assert_eq!(answer["ok"], true, "{answer}");
    assert!(path.is_file());
    let _ = std::fs::remove_dir_all(dir);
}

/// Switching documents while a capture is in flight fails the request instead of saving new-
/// document pixels.
#[test]
fn a_document_change_while_a_capture_is_in_flight_fails_instead_of_saving_it() {
    let (mut app, tx, dir) = app("screenshot-document");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let path = dir.join("other-document.png");
    let reply = request(&tx, json!({"path": path.to_string_lossy()}));
    admit(&mut app, &ctx);
    let callback = drive_until_issued(&mut app, &ctx);
    // The user switches documents during the 240 ms settle/in-flight window: pixels of the new
    // document must not be answered for the old request.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    callback.complete(Arc::new(egui::ColorImage::filled([320, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    let answer = reply.try_recv().expect("the identity reply");
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["error"].as_str().unwrap_or_default().contains("document changed"), "{answer}");
    assert!(!path.exists());
    assert!(reply.try_recv().is_err(), "no second reply");
    let _ = std::fs::remove_dir_all(dir);
}

/// Switching documents during settle fails at issuance without ever issuing a capture or keeping a
/// reservation.
#[test]
fn a_document_change_during_the_settle_fails_at_issuance_without_issuing_a_capture() {
    let (mut app, tx, dir) = app("screenshot-document-settle");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let path = dir.join("settled-away.png");
    let reply = request(&tx, json!({"path": path.to_string_lossy()}));
    admit(&mut app, &ctx);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let mut answer = None;
    for _ in 0..8 {
        let out = frame(&mut app, &ctx);
        assert!(issued(&out).is_none(), "a changed document is never captured");
        if let Ok(value) = reply.try_recv() {
            answer = Some(value);
            break;
        }
    }
    let answer = answer.expect("the identity failure comes at issuance");
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["error"].as_str().unwrap_or_default().contains("document changed"), "{answer}");
    assert!(!path.exists());
    assert!(app.pending_screenshots.is_empty(), "the admitted reservation is released");
    let _ = std::fs::remove_dir_all(dir);
}

/// A second stale-sized completion answers the request once with an error and writes nothing.
#[test]
fn a_second_stale_completion_replies_once_with_an_error_and_writes_nothing() {
    let (mut app, tx, dir) = app("screenshot-stale-twice");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let path = dir.join("stale.png");
    let reply = request(&tx, json!({"path": path.to_string_lossy()}));
    admit(&mut app, &ctx);
    drive_until_issued(&mut app, &ctx).complete(Arc::new(egui::ColorImage::filled([300, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    assert!(reply.try_recv().is_err(), "the first stale image is retried, not answered");
    drive_until_issued(&mut app, &ctx).complete(Arc::new(egui::ColorImage::filled([300, 200], egui::Color32::BLACK)));
    app.begin_host_iteration();
    app.tick(&ctx);
    let answer = reply.try_recv().expect("the second stale image fails the request");
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["error"].as_str().unwrap_or_default().contains("changed while the screenshot was in flight"), "{answer}");
    assert!(!path.exists());
    assert!(reply.try_recv().is_err(), "exactly one error reply");
    let _ = std::fs::remove_dir_all(dir);
}

/// A hidden window's logic pass requests a hidden paint instead of issuing the queued capture; the
/// capture remains pending.
#[test]
fn a_hidden_window_is_asked_to_paint_the_queued_capture() {
    let (mut app, tx, dir) = app("screenshot-hidden");
    let ctx = egui::Context::default();
    ready(&mut app, &ctx);
    let reply = request(&tx, json!({}));
    let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), ..Default::default() };
    raw.viewports.insert(ViewportId::ROOT, ViewportInfo { minimized: Some(true), inner_rect: None, focused: Some(false), ..Default::default() });
    app.begin_host_iteration();
    let out = ctx.run_logic(&raw, |ctx| app.tick(ctx));
    let commands = out.viewport_commands.get(&ViewportId::ROOT).cloned().unwrap_or_default();
    assert!(
        commands.iter().any(|c| matches!(c, ViewportCommand::RequestPaintWhileHidden)),
        "a queued capture reaches issuance only through RequestPaintWhileHidden: {commands:?}"
    );
    assert_eq!(app.pending_screenshots.len(), 1);
    assert!(app.pending_screenshots.iter().all(|p| p.slot.is_none()), "logic alone never issues a capture");
    assert!(reply.try_recv().is_err(), "no reply before the paint pass");
    let _ = std::fs::remove_dir_all(dir);
}

/// An oversized claimed image is refused from the reply-encoding budget before any copy or encode.
#[test]
fn a_data_reply_over_the_encoding_budget_is_refused_before_encoding() {
    let (mut app, _tx, _dir) = app("screenshot-encoding");
    // A huge claimed image with no pixels: the guard must look at the checked size before copying
    // or encoding anything. (A real 100 000 × 100 000 ColorImage would already be impossible.)
    let image = egui::ColorImage { size: [100_000, 100_000], source_size: egui::Vec2::ZERO, pixels: vec![] };
    let answer = crate::control::save_screenshot(&mut app, &image, None, true);
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["error"].as_str().unwrap_or_default().contains("reply-encoding limit"), "{answer}");
}

/// A path plus data capture writes the file and omits an oversized inline PNG; without a path the
/// same request fails.
#[test]
fn a_path_plus_data_capture_writes_the_file_and_omits_an_oversized_inline_png() {
    let (mut app, _tx, dir) = app("screenshot-path-data");
    let image = egui::ColorImage::filled([2, 2], egui::Color32::RED);
    let path = dir.join("shot.png");
    let path_text = path.to_string_lossy().into_owned();
    let answer = crate::control::save_screenshot_with_limit(&mut app, &image, Some(&path_text), true, 1);
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["path"], json!(path_text), "{answer}");
    assert_eq!(answer["result"]["width"], json!(2), "{answer}");
    assert_eq!(answer["result"]["height"], json!(2), "{answer}");
    assert!(answer["result"].get("pngBase64").is_none(), "the oversized inline copy is downgraded, not the file: {answer}");
    assert!(path.is_file(), "the named file is still written");

    // Without a path there is no delivery channel left, so that request still fails before it
    // copies or encodes the image.
    let answer = crate::control::save_screenshot_with_limit(&mut app, &image, None, true, 1);
    assert_eq!(answer["ok"], false, "{answer}");
    assert!(answer["error"].as_str().unwrap_or_default().contains("reply-encoding limit"), "{answer}");
    let _ = std::fs::remove_dir_all(dir);
}
