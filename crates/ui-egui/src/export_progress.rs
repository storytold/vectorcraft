//! The export progress dialog: while an export runs in the background (Background Export) a modal
//! window shows what is being exported, how far it has got (artboard by artboard for raster
//! exports of several artboards; busy for one artboard and for formats made in one go, which
//! report no steps) and two buttons: Cancel (`file.cancelExport`, also
//! Esc) stops it and writes nothing, Continue in Background (`file.exportInBackground`) closes the
//! dialog and leaves the export to the status bar.
//!
//! It opens after [`DELAY`] so a quick export doesn't flash it, and waits while another dialog is
//! open (the overwrite question, an options dialog): never two modals at once.

use serde_json::json;

use crate::dialogs::modal;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// Seconds an export runs before the dialog opens.
pub const DELAY: f64 = 0.4;

/// Where the job a frame first saw is remembered (its id, the time it was first seen).
fn seen_id() -> egui::Id {
    egui::Id::new("export-progress-seen")
}

/// What the dialog shows: (label, steps done, steps in all, share done, exports after this one).
fn shown(app: &VectorcraftApp, now: f64, ctx: &egui::Context) -> Option<(String, u64, u64, f32, usize)> {
    let jobs: Vec<_> = app.background.jobs.iter().filter(|j| !j.hidden && !j.cancelled()).filter(|j| j.progress.is_some()).collect();
    let first = jobs.first()?;
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
    Some((first.label.clone(), done, total, frac, jobs.len() - 1))
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let Some((label, done, total, frac, more)) = shown(app, now, ctx) else { return };
    if app.ui.dialog.is_some() {
        return;
    }
    let t = Tokens::get(ctx);
    let heading = format!("{}…", crate::i18n::msg(&label));
    let mut action = None;
    modal::show(ctx, &heading, egui::Id::new("export-progress"), -40.0, 20, |ui| {
        ui.set_width(360.0);
        modal::heading(ui, &heading);
        ui.add_space(12.0);
        // One step says nothing a spinner doesn't: the bar would sit at 0 % until the file is written.
        if total > 1 {
            let step = crate::i18n::fmt(tl!("Artboard {n} of {total}"), &[("n", &(done + 1).min(total).to_string()), ("total", &total.to_string())]);
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
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                ui.label(egui::RichText::new(tl!("Encoding and writing the file…")).color(t.text_dim));
            });
        }
        if more > 0 {
            ui.add_space(6.0);
            let queued = crate::i18n::tn(more as u64, "{n} more export waiting", "{n} more exports waiting");
            ui.label(egui::RichText::new(queued).size(11.5).color(t.text_dim));
        }
        ui.add_space(16.0);
        let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::secondary_button(ui, "Cancel").clicked() {
                action = Some("file.cancelExport");
            }
            ui.add_space(8.0);
            if widgets::secondary_button(ui, "Continue in Background").clicked() {
                action = Some("file.exportInBackground");
            }
        });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some("file.cancelExport");
    }
    if let Some(id) = action
        && let Err(e) = app.run(id, json!({}))
    {
        app.status(e);
    }
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
}
