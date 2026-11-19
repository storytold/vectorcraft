//! The export progress dialog: while an export runs in the background (Background Export) a modal
//! window shows what is being exported, how far it has got (artboard by artboard for raster
//! exports of several artboards, file by file for Export for Screens, as a share for an exporter
//! plug-in that reports its progress; busy for one artboard and for formats made in one go, which
//! report no steps) and two buttons: Cancel (`file.cancelExport`, also
//! Esc) stops it and writes nothing (Export for Screens keeps the files it wrote before the
//! Cancel), Continue in Background (`file.exportInBackground`) closes the
//! dialog and leaves the export to the status bar.
//!
//! A save running in the background (Background Save) shows in the same window, with a moving bar
//! (a save counts no steps) and how many more saves wait: Cancel (`file.cancelSave`, also Esc)
//! stops it before it replaces its file, and Continue in Background (`file.saveInBackground`)
//! leaves it to the status bar.
//!
//! It opens after [`DELAY`] so a quick export doesn't flash it, and waits while another dialog is
//! open (the overwrite question, an options dialog): never two modals at once.

use serde_json::json;

use crate::background::Kind;
use crate::dialogs::modal;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// Seconds an export runs before the dialog opens.
pub const DELAY: f64 = 0.4;

/// Where the job a frame first saw is remembered (its id, the time it was first seen).
fn seen_id() -> egui::Id {
    egui::Id::unique("export-progress-seen")
}

/// What the dialog shows: (label, steps done, steps in all, share done, jobs of its kind after this
/// one, what the steps are).
fn shown(app: &VectorcraftApp, now: f64, ctx: &egui::Context) -> Option<(String, u64, u64, f32, usize, Steps)> {
    let jobs: Vec<_> = app.background.jobs.iter().filter(|j| !j.hidden && !j.cancelled()).filter(|j| j.progress.is_some()).collect();
    let first = jobs.first()?;
    let more = jobs.iter().filter(|j| j.kind == first.kind).count() - 1;
    let since = ctx.data_mut(|m| {
        let seen = m.get_temp_mut_or_insert_with(seen_id(), || (first.id(), now));
        if seen.0 != first.id() {
            *seen = (first.id(), now);
        }
        seen.1
    });
    if now - since < DELAY {
        return None;
    }
    let (done, total) = first.progress.as_ref().map(|p| p.counts()).unwrap_or_default();
    let frac = first.progress.as_ref().and_then(|p| p.fraction()).unwrap_or(0.0) as f32;
    let steps = match &first.progress {
        _ if first.kind == Kind::Save => Steps::Save,
        Some(p) if p.counts_share() => Steps::Share,
        Some(p) if p.counts_files() => Steps::Files,
        _ => Steps::Artboards,
    };
    Some((first.label.clone(), done, total, frac, more, steps))
}

/// What an export's steps are.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Steps {
    Artboards,
    /// Files (Export for Screens).
    Files,
    /// An exporter plug-in's own steps, shown as a share of the work.
    Share,
    /// A save, which counts no steps: a moving bar.
    Save,
}

/// Draws the window for the first export running in the background that wasn't hidden or
/// cancelled, once it has run [`DELAY`] seconds and no other dialog is open, and runs the command
/// of the button pressed.
pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let Some((label, done, total, frac, more, steps)) = shown(app, now, ctx) else { return };
    if app.ui.dialog.is_some() {
        return;
    }
    let t = Tokens::get(ctx);
    let heading = format!("{}…", crate::i18n::msg(&label));
    let pressed = window(ctx, "export-progress", &heading, |ui| {
        if steps == Steps::Save {
            indeterminate(ui, tl!("Encoding and writing the file…"));
            if more > 0 {
                ui.add_space(6.0);
                let queued = crate::i18n::tn(more as u64, "{n} more save waiting", "{n} more saves waiting");
                ui.label(egui::RichText::new(queued).size(11.5).color(t.text_dim));
            }
            return;
        }
        // One step says nothing a spinner doesn't: the bar would sit at 0 % until the file is written.
        if steps == Steps::Share && total > 0 {
            share(ui, tl!("Encoding and writing the file…"), frac);
        } else if total > 1 {
            let step = if steps == Steps::Files { tl!("File {n} of {total}") } else { tl!("Artboard {n} of {total}") };
            let step = crate::i18n::fmt(step, &[("n", &(done + 1).min(total).to_string()), ("total", &total.to_string())]);
            // The count on its own line: drawn over the bar it would straddle the fill's edge.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(step).color(t.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{:.0} %", frac * 100.0)).color(t.text_dim));
                });
            });
            ui.add_space(4.0);
            ui.add(egui::ProgressBar::new(frac).desired_width(360.0).desired_height(6.0).fill(t.accent));
        } else {
            busy(ui, tl!("Encoding and writing the file…"));
        }
        if more > 0 {
            ui.add_space(6.0);
            let queued = crate::i18n::tn(more as u64, "{n} more export waiting", "{n} more exports waiting");
            ui.label(egui::RichText::new(queued).size(11.5).color(t.text_dim));
        }
    });
    let id = match (pressed, steps == Steps::Save) {
        (Some(Pressed::Cancel), false) => "file.cancelExport",
        (Some(Pressed::Hide), false) => "file.exportInBackground",
        (Some(Pressed::Cancel), true) => "file.cancelSave",
        (Some(Pressed::Hide), true) => "file.saveInBackground",
        (None, _) => return,
    };
    if let Err(e) = app.run(id, json!({})) {
        app.status(e);
    }
}

/// A button of a progress [`window`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressed {
    /// Cancel, or Esc: stop the work.
    Cancel,
    /// Continue in Background: close the window; the work goes on, shown in the status bar.
    Hide,
}

/// A modal progress window with only a Cancel button (and bound to Esc).
pub fn window_cancel(ctx: &egui::Context, id: &str, heading: &str, body: impl FnOnce(&mut egui::Ui)) -> Option<Pressed> {
    window_impl(ctx, id, heading, false, body)
}

/// A modal window (`id`) headed `heading`, in the UI language: `body` draws how far the work has
/// got, then come Cancel (also Esc) and Continue in Background → the one pressed. Work that may
/// take a while in the background shows it: exports, and heavy plug-in filters
/// ([`crate::plugin_runs`]).
pub fn window(ctx: &egui::Context, id: &str, heading: &str, body: impl FnOnce(&mut egui::Ui)) -> Option<Pressed> {
    window_impl(ctx, id, heading, true, body)
}

fn window_impl(ctx: &egui::Context, id: &str, heading: &str, allow_hide: bool, body: impl FnOnce(&mut egui::Ui)) -> Option<Pressed> {
    let mut pressed = None;
    modal::show(ctx, heading, egui::Id::unique(id), -40.0, 20, |ui| {
        ui.set_width(360.0);
        modal::heading(ui, heading);
        ui.add_space(12.0);
        body(ui);
        ui.add_space(16.0);
        let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::secondary_button(ui, "Cancel").clicked() {
                pressed = Some(Pressed::Cancel);
            }
            if allow_hide {
                ui.add_space(8.0);
                if widgets::secondary_button(ui, "Continue in Background").clicked() {
                    pressed = Some(Pressed::Hide);
                }
            }
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        pressed = Some(Pressed::Cancel);
    }
    pressed
}

/// Work that reports the share it has done (0 to 1), in a [`window`]: `what` it does, in the UI
/// language, and the share as a percentage, over a bar filled that far. A plug-in's own steps are
/// shown this way.
pub fn share(ui: &mut egui::Ui, what: &str, done: f32) {
    let t = Tokens::get(ui.ctx());
    let done = if done.is_finite() { done.clamp(0.0, 1.0) } else { 0.0 };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(what).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(format!("{:.0} %", done * 100.0)).color(t.text_dim));
        });
    });
    ui.add_space(4.0);
    ui.add(egui::ProgressBar::new(done).desired_width(360.0).desired_height(6.0).fill(t.accent));
    // The work reports from another thread: draw what it reports next.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}

/// Work that counts no steps, in a [`window`]: a spinner and `what` it does, in the UI language.
pub fn busy(ui: &mut egui::Ui, what: &str) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(14.0).color(t.accent));
        ui.label(egui::RichText::new(what).color(t.text_dim));
    });
}

/// Seconds the moving segment of an [`indeterminate`] bar takes to cross the track once.
const SWEEP: f64 = 1.6;

/// The share of the track an [`indeterminate`] bar's segment covers.
const SEGMENT: f32 = 0.3;

/// Where the segment of an [`indeterminate`] bar is at `time` seconds: its left and right ends as
/// shares of the track, clipped to it (it enters at the left and leaves at the right).
pub fn sweep_at(time: f64) -> (f32, f32) {
    let phase = if time.is_finite() { (time / SWEEP).rem_euclid(1.0) as f32 } else { 0.0 };
    let left = phase * (1.0 + SEGMENT) - SEGMENT;
    (left.max(0.0), (left + SEGMENT).min(1.0))
}

/// Work whose progress isn't known, in a [`window`]: `what` it does, in the UI language, over a
/// bar whose segment keeps sweeping across it while the window shows.
pub fn indeterminate(ui: &mut egui::Ui, what: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(what).color(t.text_dim));
    ui.add_space(4.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(360.0, 6.0), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, what));
    let radius = egui::CornerRadius::same(3);
    ui.painter().rect_filled(rect, radius, ui.visuals().extreme_bg_color);
    let (from, to) = sweep_at(ui.input(|i| i.time));
    if to > from {
        let x = |share: f32| rect.left() + rect.width() * share;
        let segment = egui::Rect::from_min_max(egui::pos2(x(from), rect.top()), egui::pos2(x(to), rect.bottom()));
        ui.painter().rect_filled(segment, radius, t.accent);
    }
    ui.ctx().request_repaint();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::cmd::fileio::progress::Progress;

    use crate::{Services, VectorcraftApp, background};

    /// An app whose background writer waits until `go` is set.
    fn app(go: Arc<std::sync::atomic::AtomicBool>) -> VectorcraftApp {
        let services = Services {
            write_shared: Some(Arc::new(move |_: &str, _: &[u8]| {
                while !go.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            })),
            ..Default::default()
        };
        VectorcraftApp::new(Session::new(), services)
    }

    /// Draws one frame of the dialog at `time` seconds → the texts it drew.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> String {
        let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| super::show(app, ui.ctx()));
        out.textures_delta.clear();
        let mut texts = String::new();
        for shape in out.shapes {
            if let egui::epaint::Shape::Text(t) = shape.shape {
                texts.push_str(t.galley.text());
                texts.push('\n');
            }
        }
        texts
    }

    #[test]
    fn opens_after_the_delay_and_cancel_closes_it() {
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut app = app(go.clone());
        let p = Progress::new();
        let r = background::run_with_progress(
            &mut app,
            true,
            "Exporting a.png".into(),
            None,
            Some(p.clone()),
            |w| w("/a.png", b"x").map(|()| json!({})),
            |_, r| r,
        );
        assert_eq!(r.unwrap()["background"], true);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        assert!(!frame(&mut app, &ctx, 0.0).contains("Exporting a.png"), "not before the delay");
        frame(&mut app, &ctx, 1.0); // a new window measures itself unseen first
        let shown = frame(&mut app, &ctx, 1.1);
        assert!(shown.contains("Exporting a.png") && shown.contains("Cancel") && shown.contains("Continue in Background"), "{shown}");
        app.run("file.cancelExport", json!({})).unwrap();
        assert!(p.cancelled());
        assert_eq!(app.ui.status, crate::io::EXPORT_CANCELLED);
        assert!(!frame(&mut app, &ctx, 2.0).contains("Exporting a.png"), "a cancelled export isn't shown");
        go.store(true, std::sync::atomic::Ordering::SeqCst);
        background::wait_all(&mut app);
    }

    #[test]
    fn continue_in_background_hides_it_and_the_export_goes_on() {
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut app = app(go.clone());
        let p = Progress::new();
        background::run_with_progress(
            &mut app,
            true,
            "Exporting b.png".into(),
            None,
            Some(p.clone()),
            |w| w("/b.png", b"x").map(|()| json!({})),
            |_, r| r,
        )
        .unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0);
        frame(&mut app, &ctx, 1.0);
        assert!(frame(&mut app, &ctx, 1.1).contains("Exporting b.png"));
        assert_eq!(app.run("file.exportInBackground", json!({})).unwrap()["hidden"], 1);
        assert!(!frame(&mut app, &ctx, 2.0).contains("Exporting b.png"));
        assert!(!p.cancelled());
        go.store(true, std::sync::atomic::Ordering::SeqCst);
        background::wait_all(&mut app);
        assert!(app.background.jobs.is_empty());
    }

    /// A Background Save shows in the window after the delay, with a moving bar and how many more
    /// saves wait, never over another dialog; Continue in Background hides it and the saves go on.
    /// With Background Save off the save is written at once and no window shows.
    #[test]
    fn a_background_save_shows_in_the_window_and_continue_hides_it() {
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut app = app(go.clone());
        for name in ["a", "b"] {
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            let r = app.run("file.save", json!({"path": format!("/docs/{name}.vectorcraft")})).unwrap();
            assert_eq!(r["background"], true, "{r}");
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        assert!(!frame(&mut app, &ctx, 0.0).contains("Saving a.vectorcraft"), "not before the delay");
        // Another dialog is open: the window waits.
        app.ui.dialog = Some(crate::state::Dialog::new("about", json!({})));
        frame(&mut app, &ctx, 1.0);
        assert!(!frame(&mut app, &ctx, 1.1).contains("Saving a.vectorcraft"), "never over another dialog");
        app.ui.dialog = None;
        frame(&mut app, &ctx, 1.2);
        let shown = frame(&mut app, &ctx, 1.3);
        assert!(shown.contains("Saving a.vectorcraft…") && shown.contains("1 more save waiting"), "{shown}");
        assert!(shown.contains("Cancel") && shown.contains("Continue in Background") && !shown.contains("export"), "{shown}");
        assert_eq!(app.run("file.saveInBackground", json!({})).unwrap()["hidden"], 2);
        assert!(!frame(&mut app, &ctx, 2.0).contains("Saving"), "hidden saves aren't shown");
        assert!(crate::menus::enabled(&app, "file.cancelSave"), "they can still be canceled");
        go.store(true, std::sync::atomic::Ordering::SeqCst);
        background::wait_all(&mut app);
        assert!(app.background.jobs.is_empty());
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/docs/b.vectorcraft"));
        app.session.execute("prefs.set", &json!({"values": {"backgroundSave": false}})).unwrap();
        app.services.write = Some(Box::new(|_: &str, _: &[u8]| Ok(())));
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let r = app.run("file.save", json!({"path": "/docs/c.vectorcraft"}));
        assert!(r.as_ref().is_ok_and(|r| r.get("background").is_none()), "{r:?}");
        assert!(app.background.jobs.is_empty() && !frame(&mut app, &ctx, 3.0).contains("Saving"));
    }

    #[test]
    fn an_export_of_files_counts_files() {
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut app = app(go.clone());
        let p = Progress::of_files();
        let reporting = p.clone();
        background::run_with_progress(
            &mut app,
            true,
            "Exporting icons".into(),
            None,
            Some(p.clone()),
            move |w| {
                vectorcraft_engine::cmd::fileio::progress::scope(&reporting, || {
                    vectorcraft_doc::cancel::progress(1, 3, "file 2 of 3");
                    w("/icons/b.png", b"x").map(|()| json!({}))
                })
            },
            |_, r| r,
        )
        .unwrap();
        // The worker reports the step before it waits on the writer.
        for _ in 0..10_000 {
            if p.counts() == (1, 3) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0);
        frame(&mut app, &ctx, 1.0);
        let shown = frame(&mut app, &ctx, 1.1);
        assert!(shown.contains("File 2 of 3") && !shown.contains("Artboard"), "{shown}");
        go.store(true, std::sync::atomic::Ordering::SeqCst);
        background::wait_all(&mut app);
    }
}
