//! Concept to Vector panel (Window › Concept to Vector, Object › Concept to Vector…): vector
//! variations of the selected image or unexpanded Image Trace object, one for each ticked style,
//! compared side by side and inserted to the right of the image.
//!
//! The panel calls the built-in provider ([`offline::ID`]), whose styles are made with Image Trace
//! on this computer: its prompt field is disabled and it makes vector art only. Generate prepares
//! the request at once (`conceptVector.generate`'s checks) and runs the provider on a thread of its
//! own ([`ConceptRuns`]); a new Generate cancels the one running. Results from another provider
//! are named as that provider names its styles. The results stay in this session
//! until Insert Selected puts the ticked ones in the document as one undo step
//! (`conceptVector.insert`). The web build has no thread for it, so it makes one part a frame
//! (the built-in provider: the image decoded, a style's colour layers found, or one colour layer
//! traced), and Cancel takes effect before the next part. Unit tests that set neither
//! `off_the_ui_thread` nor `by_frames` run the command at once.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::cancel::CancelToken;
use vectorcraft_engine::cmd::conceptvector::{ConceptRun, PartRun, provider_error};
use vectorcraft_engine::concept_vector::{MAX_STYLES, ProviderError, Variation, offline};

use crate::VectorcraftApp;
use crate::widgets::{self, dim_label};

pub const ID: &str = "conceptVector";

/// The UI command that opens the panel for the selected image.
pub const OPEN: &str = "ui.conceptVector";

/// What the status bar says while styles are made.
const LABEL: &str = "Making the Concept to Vector styles";

/// The most points a preview draws; the points past them are left out of it.
const PREVIEW_POINTS: usize = 40_000;

/// A preview's height in points.
const PREVIEW_H: f32 = 110.0;

/// Pixels per point of a preview's painted image.
const PREVIEW_SCALE: f32 = 2.0;

/// The thinnest a preview draws a stroke, in points, so that thin lines still show.
const MIN_STROKE: f32 = 0.5;

/// What a run's thread hands back.
type Done = (ConceptRun, Result<Vec<Variation>, ProviderError>);

/// How a Generate runs.
enum Work {
    /// On a thread of its own, which hands back its results.
    Thread(Receiver<Done>),
    /// On the UI thread, one part a frame ([`ConceptRun::run_part`]), where no thread can run it.
    Frames(Box<ConceptRun>, PartRun),
}

/// A Generate running.
struct Running {
    token: CancelToken,
    /// Styles made and in all.
    steps: Arc<[AtomicU64; 2]>,
    work: Work,
}

/// One variation as the panel draws it: its fills and strokes painted into `image` in the order of
/// its paths.
struct Preview {
    style: String,
    label: String,
    aspect: f32,
    image: egui::ColorImage,
    /// `image`, once drawn.
    texture: Option<egui::TextureHandle>,
    warnings: Vec<String>,
}

/// The job the panel shows.
struct Shown {
    job: u64,
    previews: Vec<Preview>,
    /// The styles ticked for Insert Selected.
    picked: Vec<String>,
}

/// The panel's settings and its run.
pub struct ConceptRuns {
    styles: Vec<String>,
    detail: u8,
    running: Option<Running>,
    shown: Option<Shown>,
    /// Set by unit tests whose app makes styles off the UI thread.
    pub(crate) off_the_ui_thread: bool,
    /// Set by unit tests whose app makes styles one a frame, as the web build does.
    pub(crate) by_frames: bool,
}

impl Default for ConceptRuns {
    fn default() -> Self {
        Self {
            styles: offline::STYLES.iter().map(|s| s.id.to_string()).collect(),
            detail: 50,
            running: None,
            shown: None,
            off_the_ui_thread: false,
            by_frames: false,
        }
    }
}

impl ConceptRuns {
    /// Is a Generate running?
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// The job the panel shows.
    pub fn job(&self) -> Option<u64> {
        self.shown.as_ref().map(|s| s.job)
    }
}

/// The params of `conceptVector.generate` for the panel's settings.
fn params(app: &VectorcraftApp) -> Value {
    json!({ "provider": offline::ID, "styles": app.concept_runs.styles, "detail": app.concept_runs.detail })
}

/// Whether the panel can make styles of the selection; else why not.
pub fn eligible(app: &VectorcraftApp) -> Result<(), String> {
    let p = json!({ "provider": offline::ID, "styles": [offline::STYLES[0].id] });
    ConceptRun::prepare(&app.session, &p).map(|_| ()).map_err(|e| match e {
        vectorcraft_engine::EngineError::BadParams { msg, .. } => msg,
        e => e.to_string(),
    })
}

/// `ui.conceptVector`: opens the panel for the selection; an error when it has nothing to use.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    eligible(app)?;
    app.run("window.panel.activate", json!({ "panel": ID }))
}

/// Makes the ticked styles of the selection: off the UI thread where it can; on the web, one part
/// a frame; else (unit tests, `VECTORCRAFT_SYNC_OPEN`) at once.
pub fn generate(app: &mut VectorcraftApp) -> Result<(), String> {
    let p = params(app);
    let frames = cfg!(target_arch = "wasm32") || app.concept_runs.by_frames;
    if !(frames || app.concept_runs.off_the_ui_thread || crate::opening::available()) {
        let v = app.run("conceptVector.generate", p)?;
        show_job(app, &v);
        return Ok(());
    }
    let run = ConceptRun::prepare(&app.session, &p).map_err(|e| e.to_string())?;
    cancel(app);
    let token = CancelToken::new();
    let steps = Arc::new([AtomicU64::new(0), AtomicU64::new(run.style_count() as u64)]);
    let work = if frames {
        Work::Frames(Box::new(run), PartRun::default())
    } else {
        let (tx, done) = channel();
        let stop = token.clone();
        let counts = Arc::clone(&steps);
        std::thread::Builder::new()
            .name("vectorcraft-concept".into())
            .spawn(move || {
                let [done_now, total_now] = &*counts;
                let report = |done: u64, total: u64| {
                    done_now.store(done, Ordering::Relaxed);
                    total_now.store(total, Ordering::Relaxed);
                    !stop.is_cancelled()
                };
                let made = run.run(&report);
                // The app may have quit meanwhile: nothing to hand it to.
                let _ = tx.send((run, made));
            })
            .map_err(|e| e.to_string())?;
        Work::Thread(done)
    };
    app.concept_runs.running = Some(Running { token, steps, work });
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
    app.status(format!("{}…", crate::i18n::msg(LABEL)));
    Ok(())
}

/// Cancel: the run stops at its next check (on the web, before its next part), and its results
/// are dropped.
pub fn cancel(app: &mut VectorcraftApp) {
    if let Some(r) = app.concept_runs.running.take() {
        r.token.cancel();
        if app.ui.status == format!("{}…", crate::i18n::msg(LABEL)) {
            app.ui.status.clear();
        }
    }
}

/// Each frame: makes the next part of a run made one part a frame, and keeps a finished run's
/// results as a job and shows it. Not while a dialog or an interaction is open, as
/// [`crate::trace_runs::poll`].
pub fn poll(app: &mut VectorcraftApp) {
    if app.ui.dialog.is_some() || app.session.in_interaction() {
        return;
    }
    let Some(r) = app.concept_runs.running.as_mut() else { return };
    let ended = match &mut r.work {
        Work::Thread(_) => None,
        Work::Frames(run, made) => {
            let stop = r.token.clone();
            match run.run_part(made, &|_, _| !stop.is_cancelled()) {
                Ok(false) => {
                    r.steps[0].store(made.styles_done() as u64, Ordering::Relaxed);
                    if let Some(ctx) = &app.ui_ctx {
                        ctx.request_repaint();
                    }
                    return;
                }
                Ok(true) => Some(Ok(())),
                Err(e) => Some(Err(e)),
            }
        }
    };
    let received = match (&ended, &r.work) {
        (None, Work::Thread(rx)) => match rx.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        },
        _ => None,
    };
    let Some(r) = app.concept_runs.running.take() else { return };
    let done = match (ended, r.work) {
        (Some(end), Work::Frames(run, made)) => Some((*run, end.map(|()| made.made()))),
        _ => received,
    };
    finish(app, &r.token, done);
}

/// Keeps a run's results `done` as a job and shows it; else why there are none, in the status bar.
/// Nothing when `token` was canceled.
fn finish(app: &mut VectorcraftApp, token: &CancelToken, done: Option<Done>) {
    if app.ui.status == format!("{}…", crate::i18n::msg(LABEL)) {
        app.ui.status.clear();
    }
    if token.is_cancelled() {
        return;
    }
    match done {
        // Nothing is kept or shown when the run's document has closed.
        Some((run, Ok(made))) => {
            if let Some(v) = run.finish(&mut app.session, made) {
                show_job(app, &v);
            }
        }
        Some((_, Err(e))) => app.status(provider_error(e).to_string()),
        None => app.status("Concept to Vector stopped: its thread ended without results (please report this bug)"),
    }
}

/// Shows the variations of `job` (a job as `conceptVector.generate` returns it), every one ticked,
/// each named as the job names its style: by the name its provider gives the style.
fn show_job(app: &mut VectorcraftApp, job: &Value) {
    let Some(id) = job["job"].as_u64() else { return };
    let Some((variations, (w, h))) = app.session.concept_variations(id) else { return };
    let listed = job["variations"].as_array().map(Vec::as_slice).unwrap_or_default();
    let label_of = |style: &str| listed.iter().find(|v| v["style"] == style).and_then(|v| v["label"].as_str());
    let previews: Vec<Preview> = variations
        .iter()
        .map(|v| {
            let label = label_of(&v.style).map_or_else(|| v.style.clone(), str::to_string);
            preview(v, label, (w, h))
        })
        .collect();
    let picked = previews.iter().map(|p| p.style.clone()).collect();
    app.concept_runs.shown = Some(Shown { job: id, previews, picked });
}

/// A preview's width in points, `height` high, for an image `aspect` wide per unit high.
fn preview_width(aspect: f32, height: f32) -> f32 {
    (height * aspect).clamp(height * 0.4, height * 2.5)
}

/// The box an image `aspect` wide per unit high takes inside `inner`: as large as fits, centered,
/// so a preview whose width [`preview_width`] clamps keeps the image's proportions.
fn art_rect(inner: Rect, aspect: f32) -> Rect {
    let size = vec2(inner.width().min(inner.height() * aspect), inner.height().min(inner.width() / aspect));
    Rect::from_center_size(inner.center(), size)
}

/// The size in pixels of the fills of a preview of an image `aspect` wide per unit high: its
/// [`art_rect`] inside the 4 pt margin, at [`PREVIEW_SCALE`].
fn image_size(aspect: f32) -> [usize; 2] {
    let inner = Rect::from_min_size(Pos2::ZERO, vec2(preview_width(aspect, PREVIEW_H), PREVIEW_H)).shrink(4.0);
    let art = art_rect(inner, aspect);
    let px = |pt: f32| (pt * PREVIEW_SCALE).round().max(1.0) as usize;
    [px(art.width()), px(art.height())]
}

/// Paints `rings` (polylines in 0..1 of the image's box, each closed, nonzero winding) in `color`
/// over `img`, antialiased with four samples a pixel high.
fn fill_rings(img: &mut egui::ColorImage, rings: &[Vec<Pos2>], color: Color32) {
    const SUB: usize = 4;
    let [w, h] = img.size;
    let (sx, sy) = (w as f32, h as f32);
    let mut edges: Vec<(f32, f32, f32, f32)> = vec![];
    for ring in rings {
        for (a, b) in ring.iter().zip(ring.iter().skip(1).chain(ring.first())) {
            edges.push((a.x * sx, a.y * sy, b.x * sx, b.y * sy));
        }
    }
    let top = edges.iter().map(|e| e.1.min(e.3)).fold(f32::INFINITY, f32::min);
    let bottom = edges.iter().map(|e| e.1.max(e.3)).fold(f32::NEG_INFINITY, f32::max);
    let left = edges.iter().map(|e| e.0.min(e.2)).fold(f32::INFINITY, f32::min);
    let right = edges.iter().map(|e| e.0.max(e.2)).fold(f32::NEG_INFINITY, f32::max);
    if !(top.is_finite() && bottom.is_finite() && left.is_finite() && right.is_finite()) {
        return;
    }
    let (y0, y1) = (top.floor().clamp(0.0, sy) as usize, bottom.ceil().clamp(0.0, sy) as usize);
    // Only the columns the rings reach: `cover[i]` is column `c0 + i`.
    let (c0, c1) = (left.floor().clamp(0.0, sx) as usize, right.ceil().clamp(0.0, sx) as usize);
    let mut cover = vec![0f32; c1.saturating_sub(c0)];
    let mut xs: Vec<(f32, i32)> = vec![];
    for row in y0..y1 {
        cover.fill(0.0);
        for k in 0..SUB {
            let y = row as f32 + (k as f32 + 0.5) / SUB as f32;
            xs.clear();
            for &(x0, ya, x1, yb) in &edges {
                if (ya <= y) != (yb <= y) {
                    xs.push((x0 + (y - ya) / (yb - ya) * (x1 - x0), if yb > ya { 1 } else { -1 }));
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            for pair in xs.windows(2) {
                wind += pair[0].1;
                let (a, b) = (pair[0].0.max(0.0), pair[1].0.min(sx));
                if wind == 0 || b <= a {
                    continue;
                }
                let (start, end) = ((a.floor() as usize).saturating_sub(c0), (b.ceil() as usize).min(c1).saturating_sub(c0));
                for (i, c) in cover.iter_mut().enumerate().take(end).skip(start) {
                    let px = (c0 + i) as f32;
                    *c += (b.min(px + 1.0) - a.max(px)).max(0.0) / SUB as f32;
                }
            }
        }
        for (i, c) in cover.iter().enumerate().filter(|(_, c)| **c > 0.0) {
            let px = c0 + i;
            let a = c.min(1.0);
            let mix = |s: u8, t: u8| (f32::from(t) * a + f32::from(s) * (1.0 - a)).round() as u8;
            if let Some(d) = img.pixels.get_mut(row * w + px) {
                *d = Color32::from_rgb(mix(d.r(), color.r()), mix(d.g(), color.g()), mix(d.b(), color.b()));
            }
        }
    }
}

/// Paints `rings` (polylines in 0..1 of the image's box) stroked `width` pixels wide in `color`
/// over `img`: each segment as a rectangle, with a disc at each point for round joins and caps
/// once the stroke is wide enough for them to show.
fn stroke_rings(img: &mut egui::ColorImage, rings: &[Vec<Pos2>], width: f32, color: Color32) {
    let (sx, sy) = (img.size[0] as f32, img.size[1] as f32);
    let r = width / 2.0;
    let unit = |p: Pos2| Pos2::new(p.x / sx, p.y / sy);
    for ring in rings {
        let pts: Vec<Pos2> = ring.iter().map(|q| Pos2::new(q.x * sx, q.y * sy)).collect();
        for (k, a) in pts.iter().enumerate() {
            if r > 0.75 {
                let disc: Vec<Pos2> = (0..8)
                    .map(|i| {
                        let t = i as f32 * std::f32::consts::TAU / 8.0;
                        unit(*a + vec2(t.cos(), t.sin()) * r)
                    })
                    .collect();
                fill_rings(img, &[disc], color);
            }
            let Some(b) = pts.get(k + 1) else { continue };
            let d = *b - *a;
            let len = d.length();
            if len > 0.0 {
                let n = vec2(-d.y, d.x) * (r / len);
                fill_rings(img, &[vec![unit(*a + n), unit(*b + n), unit(*b - n), unit(*a - n)]], color);
            }
        }
    }
}

/// `path`'s elements, made as they are taken, so that taking the first few of a long path does
/// not convert the rest.
fn elements(path: &vectorcraft_geom::PathData) -> impl Iterator<Item = vectorcraft_geom::PathEl> + '_ {
    use vectorcraft_geom::PathEl;
    path.subpaths.iter().filter_map(|sp| sp.anchors.first().map(|first| (sp, first.p))).flat_map(|(sp, start)| {
        let segments = (0..sp.segment_count()).map(move |i| {
            let c = sp.segment(i);
            if sp.segment_is_line(i) { PathEl::LineTo(c.p3) } else { PathEl::CurveTo(c.p1, c.p2, c.p3) }
        });
        std::iter::once(PathEl::MoveTo(start)).chain(segments).chain(sp.closed.then_some(PathEl::ClosePath))
    })
}

/// Variation `v` of an image `size` pixels big: its paths painted in order, each path's fill and
/// then its stroke, stopping after [`PREVIEW_POINTS`] points in all.
fn preview(v: &Variation, label: String, size: (u32, u32)) -> Preview {
    let (w, h) = (f64::from(size.0.max(1)), f64::from(size.1.max(1)));
    let aspect = (w / h) as f32;
    let mut image = egui::ColorImage::filled(image_size(aspect), Color32::WHITE);
    let mut left = PREVIEW_POINTS;
    let at = |q: vectorcraft_geom::Point| Pos2::new((q.x / w) as f32, (q.y / h) as f32);
    let rgb = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
    for p in &v.paths {
        if left == 0 {
            break;
        }
        let mut rings: Vec<Vec<Pos2>> = vec![];
        let mut ring: Vec<Pos2> = vec![];
        // Each element adds at least one point, and `flatten` makes a bounded number of each curve,
        // so the elements past the points left are neither made nor flattened.
        vectorcraft_geom::flatten(elements(&p.path).take(left.saturating_add(1)), (w.max(h) / 400.0).max(0.25), |el| {
            let q = match el {
                vectorcraft_geom::PathEl::MoveTo(q) => {
                    if ring.len() > 1 {
                        rings.push(std::mem::take(&mut ring));
                    }
                    ring.clear();
                    q
                }
                vectorcraft_geom::PathEl::LineTo(q) => q,
                vectorcraft_geom::PathEl::ClosePath => {
                    if let Some(first) = ring.first().copied() {
                        ring.push(first);
                    }
                    if ring.len() > 1 {
                        rings.push(std::mem::take(&mut ring));
                    }
                    ring.clear();
                    return;
                }
                _ => return,
            };
            if left > 0 {
                left -= 1;
                ring.push(at(q));
            }
        });
        if ring.len() > 1 {
            rings.push(ring);
        }
        if let Some(c) = p.fill {
            fill_rings(&mut image, &rings, rgb(c));
        }
        if let Some((c, width)) = p.stroke {
            // To scale with the image, and at least MIN_STROKE points.
            let px = ((width / w) as f32 * image.size[0] as f32).max(MIN_STROKE * PREVIEW_SCALE);
            stroke_rings(&mut image, &rings, px, rgb(c));
        }
    }
    Preview { style: v.style.clone(), label, aspect, image, texture: None, warnings: v.warnings.clone() }
}

/// Insert Selected: the ticked variations go to the right of the image, as one undo step.
pub fn insert(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(shown) = &app.concept_runs.shown else { return Err("`job` must be a job number from conceptVector.generate".into()) };
    let p = json!({ "job": shown.job, "variations": shown.picked });
    app.run("conceptVector.insert", p)
}

/// A style's name as shown: translated as Image Trace's preset of that name where the catalog has
/// one (the Outline style is not the Outline view), as any other string else.
fn style_name(label: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "imageTrace", label)
}

/// Draws preview `p`, `height` points high.
fn draw_preview(ui: &mut Ui, p: &mut Preview, height: f32) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let width = preview_width(p.aspect, height);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, Color32::WHITE);
    let inner = art_rect(rect.shrink(4.0), p.aspect);
    let texture =
        p.texture.get_or_insert_with(|| ctx.load_texture(format!("concept-vector-{}", p.style), p.image.clone(), egui::TextureOptions::LINEAR));
    ui.painter().image(texture.id(), inner, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    dim_label(ui, tl!("Offline Image Trace styles, made on this computer"));
    ui.add_space(4.0);
    let ready = eligible(app);
    // The styles, at most eight of them.
    let count = app.concept_runs.styles.len();
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Styles"));
        ui.label(format!("{count} / {MAX_STYLES}"));
    });
    for s in &offline::STYLES {
        let mut on = app.concept_runs.styles.iter().any(|id| id == s.id);
        let allowed = on || count < MAX_STYLES;
        if ui.add_enabled(allowed, egui::Checkbox::new(&mut on, style_name(s.label))).changed() {
            let styles = &mut app.concept_runs.styles;
            styles.retain(|id| id != s.id);
            if on {
                styles.push(s.id.to_string());
                // In the provider's order.
                styles.sort_by_key(|id| offline::STYLES.iter().position(|st| st.id == id));
            }
        }
    }
    ui.add_space(4.0);
    let mut detail = app.concept_runs.detail;
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Detail"));
        if widgets::slider(ui, egui::Slider::new(&mut detail, 0..=100)).changed() {
            app.concept_runs.detail = detail;
        }
    });
    let mut no_prompt = String::new();
    ui.add_enabled(false, egui::TextEdit::singleline(&mut no_prompt).hint_text(tl!("Offline styles do not use prompts")));
    ui.add_space(6.0);
    if let Some(r) = &app.concept_runs.running {
        let (done, total) = (r.steps[0].load(Ordering::Relaxed), r.steps[1].load(Ordering::Relaxed));
        let frac = if total > 0 { done.min(total) as f32 / total as f32 } else { 0.0 };
        ui.add(egui::ProgressBar::new(frac).desired_height(6.0));
        if widgets::flat_button(ui, "Cancel", 80.0).clicked() {
            cancel(app);
            app.status(vectorcraft_engine::EngineError::Cancelled.to_string());
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    } else {
        let can = ready.is_ok() && count > 0;
        let resp = ui.add_enabled_ui(can, |ui| widgets::flat_button_fit(ui, "Generate", 90.0)).inner;
        if resp.clicked()
            && let Err(e) = generate(app)
        {
            app.status(e);
        }
    }
    if let Err(why) = &ready {
        ui.label(egui::RichText::new(crate::i18n::msg(why)).small());
    }
    show_results(app, ui);
}

/// The comparison strip of the job shown, with a tick on each and Insert Selected.
fn show_results(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(Shown { previews, picked, .. }) = &mut app.concept_runs.shown else { return };
    ui.separator();
    egui::ScrollArea::horizontal().id_salt("concept-vector-strip").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for p in previews.iter_mut() {
                ui.vertical(|ui| {
                    draw_preview(ui, p, PREVIEW_H);
                    let mut on = picked.contains(&p.style);
                    if ui.checkbox(&mut on, style_name(&p.label)).changed() {
                        picked.retain(|s| *s != p.style);
                        if on {
                            picked.push(p.style.clone());
                        }
                    }
                    for w in &p.warnings {
                        ui.label(egui::RichText::new(w).small());
                    }
                });
            }
        });
    });
    let any = !picked.is_empty();
    let resp = ui.add_enabled_ui(any, |ui| widgets::flat_button_fit(ui, "Insert Selected", 110.0)).inner;
    if resp.clicked() {
        match insert(app) {
            Ok(_) => {}
            Err(e) => app.status(e),
        }
    }
}

#[cfg(test)]
mod tests;
