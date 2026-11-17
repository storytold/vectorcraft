//! The Fold Preview window: the document's blank folded into its 3D shape
//! ([`vectorcraft_three_d::fold::fold`]), with the printed art on the outside of each panel and the
//! board's inside color on the back, drawn by [`vectorcraft_three_d::fold::raster::render`].
//!
//! The window folds the blank that the document's shown fold lines make, about its largest panel
//! or the panel the user double-clicked. Its print is a raster of the document with the fold lines
//! and the layers that do not print left out ([`vectorcraft_engine::cmd::print_hidden`]), rendered
//! once per document revision, off the UI thread on native builds ([`crate::thumb_worker`]), and
//! sampled through each panel's projective map when the fold, the view or the print changes. Dragging turns the view, a Shift-drag
//! keeps to one axis, a middle-drag or a Space-drag moves it, the wheel zooms, a double-click on a
//! panel makes it the base, and the arrow keys turn the view by 5° when it has the keyboard focus.
//! The settings live in `UiState::fold_preview` (`ui.inspect`); `ui.foldPreview` sets them.

use std::sync::Arc;

use egui::{Color32, Rect, Sense, Ui, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect as DRect};
use vectorcraft_three_d::fold::raster::{self, Camera, Image, MAX_PIXELS, MAX_SIDE, Print};
use vectorcraft_three_d::fold::{Blank, Folded, fold, pick};

use crate::VectorcraftApp;
use crate::thumb_worker::{Settle, ThumbWorker};
use crate::widgets::{self, dim_label};

/// The window's id (`window.panel {panel: "foldPreview"}`).
pub const ID: &str = "foldPreview";

/// The view presets: label, id, the turn about x (the top toward the viewer) and about y (the
/// right side toward the viewer), in degrees.
pub const VIEWS: [(&str, &str, f64, f64); 7] = [
    ("Front", "front", 0.0, 0.0),
    ("Back", "back", 0.0, 180.0),
    ("Left", "left", 0.0, -90.0),
    ("Right", "right", 0.0, 90.0),
    ("Top", "top", 90.0, 0.0),
    ("Bottom", "bottom", -90.0, 0.0),
    ("Three-Quarter", "threeQuarter", 25.0, 35.0),
];

/// The tolerance the window folds with (points), as Check Fold Lines' default.
const TOLERANCE: f64 = 0.1;
/// The share of the view's shorter side the folded blank fills at a zoom of 1.
const FIT: f64 = 0.45;
/// The print raster's longest side, in pixels, and the fewest pixels it gives a point.
const PRINT_SIDE: f64 = 2048.0;
/// Degrees the view turns for each point dragged, and for each arrow key press.
const DRAG_TURN: f64 = 0.5;
const KEY_TURN: f64 = 5.0;

/// The window's settings (kept with the UI state).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// How far the blank is folded: 0 flat, 1 every hinge at its angle.
    pub amount: f64,
    /// The view's turn, in degrees (see [`VIEWS`]).
    pub rotate_x: f64,
    pub rotate_y: f64,
    /// The zoom: 1 fits the folded blank in the view.
    pub zoom: f64,
    /// How far the view is moved, in points.
    pub pan: [f64; 2],
    /// The field of view in degrees; 0 for none.
    pub perspective: f64,
    /// The board's inside color: `#rrggbb`, the name of a swatch, or empty for white.
    pub inside: String,
    /// Draw the fold lines on the print.
    pub show_fold_lines: bool,
    /// The base panel: the one under this point (document coordinates); the largest when `None`.
    pub base: Option<[f64; 2]>,
}

impl Default for Settings {
    fn default() -> Self {
        let (_, _, rotate_x, rotate_y) = VIEWS[6];
        Self {
            amount: 1.0,
            rotate_x,
            rotate_y,
            zoom: 1.0,
            pan: [0.0; 2],
            perspective: 0.0,
            inside: String::new(),
            show_fold_lines: false,
            base: None,
        }
    }
}

impl Settings {
    /// The view preset these settings show, if any.
    fn view(&self) -> Option<&'static (&'static str, &'static str, f64, f64)> {
        VIEWS.iter().find(|v| (v.2 - self.rotate_x).abs() < 1e-9 && (v.3 - self.rotate_y).abs() < 1e-9)
    }

    /// These settings with every value in the range `ui.foldPreview` allows: saved UI state is read
    /// as it was stored. A value that is not a number takes its default.
    fn clamped(mut self) -> Self {
        let within = |v: f64, lo: f64, hi: f64, or: f64| if v.is_finite() { v.clamp(lo, hi) } else { or };
        let turn = |v: f64| if (-180.0..=180.0).contains(&v) { v } else { wrapped(v) };
        let fresh = Settings::default();
        self.amount = within(self.amount, 0.0, 1.0, fresh.amount);
        self.zoom = within(self.zoom, 0.1, 8.0, fresh.zoom);
        self.perspective = within(self.perspective, 0.0, 160.0, fresh.perspective);
        (self.rotate_x, self.rotate_y) = (turn(self.rotate_x), turn(self.rotate_y));
        self.pan = self.pan.map(|v| if v.is_finite() { v } else { 0.0 });
        self.base = self.base.filter(|b| b.iter().all(|v| v.is_finite()));
        self
    }
}

/// `angle` (degrees) brought into -180 to 180.
fn wrapped(angle: f64) -> f64 {
    let a = (angle + 180.0).rem_euclid(360.0) - 180.0;
    if a.is_finite() { a } else { 0.0 }
}

/// What the print raster shows: the document, its revision, whether the fold lines are drawn, and
/// the region and size.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PrintKey {
    uid: u64,
    revision: u64,
    fold_lines: bool,
    region: [u64; 4],
    width: u32,
    height: u32,
}

/// The last frame drawn: its size in pixels and how long the fold and the drawing took (ms).
#[derive(Clone, Copy, Debug, Default)]
struct Frame {
    width: u32,
    height: u32,
    fold_ms: f64,
    draw_ms: f64,
}

/// A document's blanks and whether its fold lines have an error, by (uid, revision).
type Blanks = ((u64, u64), Arc<Vec<Blank>>, bool);

/// What a fold folds: the document (uid, revision), the blank, the base panel and the amount's bits.
type FoldKey = (u64, u64, usize, usize, u64);

/// What the last image shows: the fold, the camera, the inside color and the print.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DrawKey {
    fold: FoldKey,
    camera: Camera,
    inside: [u8; 4],
    print: Option<PrintKey>,
}

#[derive(Default)]
struct Cache {
    blanks: Option<Blanks>,
    /// The last fold, what it folds and how long it took (ms).
    folded: Option<(FoldKey, Arc<Folded>, f64)>,
    /// What the texture shows.
    drawn: Option<DrawKey>,
    /// Folds worked out and images rendered (tests count them).
    folds: u64,
    renders: u64,
    /// The print shown and what it shows.
    print: Option<(PrintKey, Arc<Print>)>,
    /// Renders the print off the UI thread (None: on this thread).
    worker: Option<ThumbWorker<(), PrintKey>>,
    worker_started: bool,
    pending: Option<PrintKey>,
    settle: Settle,
    tex: Option<egui::TextureHandle>,
    /// The last frame's image, for double-click picking, and the screen rect it filled.
    image: Option<(Image, Rect)>,
    frame: Option<Frame>,
    /// Prints rendered (tests count them).
    prints: u64,
}

thread_local! {
    static CACHE: crate::graphics::TexCache<Cache> = crate::graphics::TexCache::default();
    static RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
}

/// The board's inside color (straight RGBA) for `inside`: white when it is empty, a swatch of `d`
/// with a plain color, or else `#rrggbb`; `None` for anything else.
fn inside_color(d: &vectorcraft_engine::doc::Document, inside: &str) -> Option<[u8; 4]> {
    if inside.is_empty() {
        return Some([255; 4]);
    }
    match d.swatch(inside).map(|s| &s.paint) {
        Some(vectorcraft_color::Paint::Solid { color, .. }) => Some(color.to_rgba8(1.0)),
        _ if inside.starts_with('#') => vectorcraft_color::Color::from_hex(inside).map(|c| c.to_rgba8(1.0)),
        _ => None,
    }
}

/// The document rect a print of `blank` covers: its panels' bounds, with a point to spare.
fn print_region(blank: &Blank) -> Option<DRect> {
    let r = blank.panels.iter().filter_map(|p| p.outline.bounds()).reduce(|a, b| a.union(b))?;
    let r = r.inflate(1.0, 1.0);
    (r.width() > 0.0 && r.height() > 0.0 && r.is_finite()).then_some(r)
}

/// The pixel size of the print of `region`: its longest side [`PRINT_SIDE`].
fn print_size(region: DRect) -> (u32, u32, f64) {
    let scale = PRINT_SIDE / region.width().max(region.height());
    let side = |len: f64| ((len * scale).ceil() as u32).clamp(1, MAX_SIDE);
    (side(region.width()), side(region.height()), scale)
}

impl Cache {
    /// The blanks of document `uid` at `revision`, worked out again only when it changed.
    fn blanks(&mut self, uid: u64, revision: u64, d: &vectorcraft_engine::doc::Document) -> (Arc<Vec<Blank>>, bool) {
        if let Some((key, blanks, errors)) = &self.blanks
            && *key == (uid, revision)
        {
            return (blanks.clone(), *errors);
        }
        let (blanks, issues) = vectorcraft_engine::cmd::fold_source(d);
        let errors = issues.iter().any(|i| i.code.is_error());
        let blanks = Arc::new(blanks);
        self.blanks = Some(((uid, revision), blanks.clone(), errors));
        (blanks, errors)
    }

    /// `blank` folded about `panel` by `amount` (what `key` names), worked out again only when the
    /// key changed, and how long that took (ms).
    fn fold(&mut self, key: FoldKey, blank: &Blank, panel: usize, amount: f64) -> Option<(Arc<Folded>, f64)> {
        if let Some((k, f, ms)) = &self.folded
            && *k == key
        {
            return Some((f.clone(), *ms));
        }
        let t0 = crate::now_ms();
        let f = Arc::new(fold(blank, Some(panel), amount, TOLERANCE)?);
        let ms = crate::now_ms() - t0;
        self.folded = Some((key, f.clone(), ms));
        self.folds += 1;
        Some((f, ms))
    }

    /// Bring the print up to `key`: on the worker when there is one, else here, and not before the
    /// document has settled unless there is no print of it yet.
    fn update_print(&mut self, ctx: &egui::Context, doc: &Arc<vectorcraft_engine::doc::Document>, key: PrintKey, region: DRect, scale: f64) {
        if !self.worker_started {
            self.worker_started = true;
            self.worker = ThumbWorker::spawn(ctx, "fold-preview");
        }
        let done = match self.worker.as_ref().map(ThumbWorker::poll) {
            Some(Some(done)) => done,
            Some(None) => {
                (self.worker, self.pending) = (None, None);
                vec![]
            }
            None => vec![],
        };
        for d in done {
            if self.pending == Some(d.key) {
                self.pending = None;
            }
            if let Some(img) = d.img {
                self.keep_print(d.key, img);
            }
        }
        let shown = self.print.as_ref().map(|(k, _)| *k);
        if shown == Some(key) || self.pending == Some(key) {
            return;
        }
        let have = shown.is_some_and(|k| k.uid == key.uid);
        if have && !self.settle.settled(ctx, key.uid, key.revision, ctx.input(|i| i.time)) {
            return;
        }
        let opts = vectorcraft_render::RenderOptions {
            skip_templates: true,
            image_mips: true,
            hidden: vectorcraft_engine::cmd::print_hidden(doc, key.fold_lines),
            ..Default::default()
        };
        if let Some(w) = &self.worker {
            let (doc, opts) = (doc.clone(), opts.clone());
            if w.submit((), key, Box::new(move |r| Some(r.render_region_with(&doc, region, scale, &opts)))) {
                self.pending = Some(key);
                return;
            }
            self.worker = None;
        }
        let img = RENDERER.with(|r| r.borrow_mut().render_region_with(doc, region, scale, &opts));
        self.keep_print(key, img);
    }

    /// The print to draw for `want`: the one shown when it is of the same document, blank region and
    /// Show Fold Lines setting (an older revision while the new one renders), else none.
    fn print_for(&self, want: &PrintKey) -> Option<(PrintKey, Arc<Print>)> {
        let same = |k: &PrintKey| (k.uid, k.fold_lines, k.region) == (want.uid, want.fold_lines, want.region);
        self.print.as_ref().filter(|(k, _)| same(k)).map(|(k, p)| (*k, p.clone()))
    }

    fn keep_print(&mut self, key: PrintKey, img: vectorcraft_render::Rendered) {
        let region =
            DRect::new(f64::from_bits(key.region[0]), f64::from_bits(key.region[1]), f64::from_bits(key.region[2]), f64::from_bits(key.region[3]));
        if let Some(print) = Print::new(region, img.width, img.height, img.pixels) {
            self.print = Some((key, Arc::new(print)));
            self.prints += 1;
        }
    }
}

/// `ui.foldPreview {amount?, view?, rotateX?, rotateY?, zoom?, perspective?, inside?,
/// showFoldLines?, base?}`: set the window's settings, show it, and answer with them and the last
/// frame's size and times. A value of the wrong kind or out of range is an error, and nothing is
/// set.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut set = app.ui.fold_preview.clone();
    let number = |key: &str, lo: f64, hi: f64| -> Result<Option<f64>, String> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_f64() {
                Some(n) if (lo..=hi).contains(&n) => Ok(Some(n)),
                _ => Err(format!("`{key}` must be a number from {lo} to {hi}")),
            },
        }
    };
    if let Some(a) = number("amount", 0.0, 1.0)? {
        set.amount = a;
    }
    match p.get("view") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let (_, _, x, y) = v
                .as_str()
                .and_then(|v| VIEWS.iter().find(|view| view.1 == v))
                .ok_or_else(|| format!("`view` must be one of {}", VIEWS.iter().map(|v| v.1).collect::<Vec<_>>().join(", ")))?;
            (set.rotate_x, set.rotate_y) = (*x, *y);
        }
    }
    if let Some(x) = number("rotateX", -180.0, 180.0)? {
        set.rotate_x = x;
    }
    if let Some(y) = number("rotateY", -180.0, 180.0)? {
        set.rotate_y = y;
    }
    if let Some(z) = number("zoom", 0.1, 8.0)? {
        set.zoom = z;
    }
    if let Some(v) = number("perspective", 0.0, 160.0)? {
        set.perspective = v;
    }
    match p.get("inside") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let inside = v.as_str().filter(|inside| {
                inside.is_empty()
                    || app.session.active().and_then(|st| inside_color(&st.doc, inside)).is_some()
                    || (inside.starts_with('#') && vectorcraft_color::Color::from_hex(inside).is_some())
            });
            let Some(inside) = inside else {
                return Err(format!("`inside` must be #rrggbb, the name of a swatch with a plain color, or empty for white, not {v}"));
            };
            set.inside = inside.to_string();
        }
    }
    match p.get("showFoldLines") {
        None | Some(Value::Null) => {}
        Some(v) => set.show_fold_lines = v.as_bool().ok_or("`showFoldLines` must be true or false")?,
    }
    match p.get("base") {
        None => {}
        Some(Value::Null) => set.base = None,
        Some(v) => {
            let a = v.as_array().filter(|a| a.len() == 2);
            let xy: Option<Vec<f64>> = a.map(|a| a.iter().filter_map(Value::as_f64).filter(|n| n.is_finite()).collect());
            let Some(&[x, y]) = xy.as_deref() else { return Err("`base` must be a point: [x, y], or null".into()) };
            let blanks = app.session.active().map(|st| vectorcraft_engine::cmd::fold_source(&st.doc).0).unwrap_or_default();
            if pick(&blanks, Some(Point::new(x, y))).is_none() {
                return Err("`base` is in no panel".into());
            }
            set.base = Some([x, y]);
        }
    }
    if p.get("view").is_some_and(|v| !v.is_null()) || p.get("zoom").is_some_and(|v| !v.is_null()) {
        set.pan = [0.0; 2];
    }
    app.ui.fold_preview = set;
    crate::panel_docking::reveal_panel(app, ID)?;
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
    Ok(state(app))
}

/// The window's settings and the last frame's size and times.
fn state(app: &VectorcraftApp) -> Value {
    let frame = CACHE.with(|c| c.borrow().frame);
    let mut out = serde_json::to_value(&app.ui.fold_preview).unwrap_or_default();
    if let Some(o) = out.as_object_mut() {
        o.insert(
            "frame".into(),
            frame.map_or(Value::Null, |f| json!({"width": f.width, "height": f.height, "foldMs": f.fold_ms, "drawMs": f.draw_ms})),
        );
    }
    out
}

/// The toolbar: the view presets, Perspective, Inside, Show Fold Lines and Reset.
fn toolbar(app: &VectorcraftApp, ui: &mut Ui, set: &mut Settings) {
    ui.horizontal_wrapped(|ui| {
        let labels: Vec<&str> = VIEWS.iter().map(|v| v.0).collect();
        let current = set.view().map_or("", |v| v.0);
        if let Some(i) = widgets::dropdown_named(ui, "fold-preview-view", Some(tl!("View")), current, &labels, 120.0)
            && let Some(&(_, _, x, y)) = VIEWS.get(i)
        {
            (set.rotate_x, set.rotate_y, set.pan) = (x, y, [0.0; 2]);
        }
        let swatches: Vec<String> = app.session.active().map_or_else(Vec::new, |st| {
            st.doc.swatches.iter().filter(|s| matches!(s.paint, vectorcraft_color::Paint::Solid { .. })).map(|s| s.name.clone()).collect()
        });
        let mut names: Vec<&str> = vec![tl!("White")];
        names.extend(swatches.iter().map(String::as_str));
        let shown = if set.inside.is_empty() { tl!("White") } else { set.inside.as_str() };
        ui.label(tl!("Inside"));
        if let Some(i) = widgets::dropdown_names_named(ui, "fold-preview-inside", Some(tl!("Inside")), shown, &names, 110.0) {
            set.inside = swatches.get(i.wrapping_sub(1)).cloned().unwrap_or_default();
        }
        if widgets::check(ui, tl!("Show Fold Lines"), set.show_fold_lines, true) {
            set.show_fold_lines = !set.show_fold_lines;
        }
        if widgets::flat_button_fit(ui, tl!("Reset"), 48.0).clicked() {
            let fresh = Settings::default();
            (set.rotate_x, set.rotate_y, set.zoom, set.pan, set.perspective) = (fresh.rotate_x, fresh.rotate_y, 1.0, [0.0; 2], 0.0);
        }
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Perspective"));
        widgets::slider(ui, egui::Slider::new(&mut set.perspective, 0.0..=160.0).suffix("°").fixed_decimals(0));
    });
}

/// The Fold and Zoom sliders.
fn sliders(ui: &mut Ui, set: &mut Settings) {
    ui.horizontal(|ui| {
        ui.label(tl!("Fold"));
        let mut percent = set.amount * 100.0;
        if widgets::slider(ui, egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%").fixed_decimals(0)).changed() {
            set.amount = (percent / 100.0).clamp(0.0, 1.0);
        }
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Zoom"));
        let mut percent = set.zoom * 100.0;
        if widgets::slider(ui, egui::Slider::new(&mut percent, 10.0..=800.0).suffix("%").fixed_decimals(0).logarithmic(true)).changed() {
            set.zoom = (percent / 100.0).clamp(0.1, 8.0);
        }
    });
}

/// Turns, moves and zooms the view with the pointer, the wheel and the arrow keys, and picks the
/// base panel of `blank` (the blank `key` folds) on a double-click.
fn interact(ui: &Ui, response: &egui::Response, set: &mut Settings, blank: Option<(&Blank, FoldKey)>) {
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let (shift, space) = ui.input(|i| (i.modifiers.shift, i.key_down(egui::Key::Space)));
    let delta = response.drag_delta();
    if delta != egui::Vec2::ZERO {
        if response.dragged_by(egui::PointerButton::Middle) || (space && response.dragged_by(egui::PointerButton::Primary)) {
            set.pan[0] += f64::from(delta.x);
            set.pan[1] += f64::from(delta.y);
        } else if response.dragged_by(egui::PointerButton::Primary) {
            let (mut dx, mut dy) = (f64::from(delta.x), f64::from(delta.y));
            if shift {
                if dx.abs() >= dy.abs() {
                    dy = 0.0;
                } else {
                    dx = 0.0;
                }
            }
            set.rotate_y = wrapped(set.rotate_y + dx * DRAG_TURN);
            set.rotate_x = wrapped(set.rotate_x + dy * DRAG_TURN);
        }
    }
    if response.hovered() {
        let scroll = f64::from(ui.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 200.0));
        if scroll != 0.0 {
            set.zoom = (set.zoom * (scroll * 0.002).exp()).clamp(0.1, 8.0);
        }
    }
    if response.has_focus() {
        // The view keeps the arrow keys, so they turn it rather than move the focus on.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(response.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
        });
        let presses = |key| ui.input_mut(|i| i.count_and_consume_key(egui::Modifiers::NONE, key)) as f64 * KEY_TURN;
        let (left, right, up, down) =
            (presses(egui::Key::ArrowLeft), presses(egui::Key::ArrowRight), presses(egui::Key::ArrowUp), presses(egui::Key::ArrowDown));
        if left + right + up + down > 0.0 {
            set.rotate_y = wrapped(set.rotate_y + right - left);
            set.rotate_x = wrapped(set.rotate_x + down - up);
        }
    }
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some((blank, key)) = blank
    {
        // The image on screen is the last one drawn; its panel numbers are used only while it shows
        // the same blank of the same document revision.
        let picked = CACHE.with(|c| {
            let c = c.borrow();
            let shown = c.drawn.map(|d| (d.fold.0, d.fold.1, d.fold.2));
            if shown != Some((key.0, key.1, key.2)) {
                return None;
            }
            let (img, rect) = c.image.as_ref()?;
            let x = (pos.x - rect.left()) / rect.width() * img.width as f32;
            let y = (pos.y - rect.top()) / rect.height() * img.height as f32;
            img.panel_at(x.max(0.0) as u32, y.max(0.0) as u32)
        });
        if let Some(p) = picked.and_then(|i| blank.panels.get(i)).and_then(|p| p.point_inside()) {
            set.base = Some([p.x, p.y]);
        }
    }
}

/// The status line under the view, in English (shown translated).
fn status(blanks: usize, folded: Option<(&Blank, &Folded)>, errors: bool) -> Vec<String> {
    let Some((blank, f)) = folded else {
        return vec!["Check the fold lines first.".into()];
    };
    let mut out = vec![crate::i18n::fmt(
        "Panels: {panels}, hinges: {hinges}",
        &[("panels", &blank.panels.len().to_string()), ("hinges", &blank.hinges.len().to_string())],
    )];
    if errors {
        out.push("Check the fold lines first.".into());
    }
    if blanks > 1 {
        out.push(crate::i18n::fmt("Other blanks not shown: {count}", &[("count", &(blanks - 1).to_string())]));
    }
    if let Some(gap) = f.hinges.iter().filter(|h| !h.closes).map(|h| h.gap).reduce(f64::max) {
        out.push(crate::i18n::fmt("A joint does not close by {gap} pt.", &[("gap", &format!("{gap:.1}"))]));
    }
    if !f.collisions.is_empty() {
        out.push("Two panels pass through each other.".into());
    }
    out
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        crate::panels::empty_state(ui, "box", tl!("No document"), tl!("Open a document to preview its folded shape."));
        return;
    };
    let (uid, revision, doc) = (st.uid, st.revision, st.doc.clone());
    let mut set = app.ui.fold_preview.clone().clamped();
    toolbar(app, ui, &mut set);
    let (blanks, errors) = CACHE.with(|c| c.borrow_mut().blanks(uid, revision, &doc));
    let base_point = set.base.map(|[x, y]| Point::new(x, y));
    let picked = pick(&blanks, base_point).or_else(|| pick(&blanks, None));
    let folded = picked.and_then(|(b, panel)| {
        let blank = blanks.get(b)?;
        let key = (uid, revision, b, panel, set.amount.to_bits());
        let (f, ms) = CACHE.with(|c| c.borrow_mut().fold(key, blank, panel, set.amount))?;
        Some((blank, f, key, ms))
    });

    let bottom = ui.spacing().interact_size.y * 3.0 + 12.0;
    let size = vec2(ui.available_width().max(64.0), (ui.available_height() - bottom).max(64.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let view_name = set.view().map_or("", |v| v.0);
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::Image, true, tl!("Fold Preview"));
        info.current_text_value = Some(crate::i18n::t(view_name).to_string());
        info
    });
    interact(ui, &response, &mut set, folded.as_ref().map(|(b, _, key, _)| (*b, *key)));
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, 2.0, t.pasteboard);
    if let Some((blank, f, key, fold_ms)) = &folded {
        draw(ui, rect, &doc, *key, blank, f, &set, *fold_ms);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    for line in status(blanks.len(), folded.as_ref().map(|(b, f, ..)| (*b, &**f)), errors) {
        dim_label(ui, &crate::i18n::msg(&line));
    }
    sliders(ui, &mut set);
    if set != app.ui.fold_preview {
        app.ui.fold_preview = set;
    }
}

/// Draws `blank` folded as `f` (the fold `key` names) into `rect`, with the print of `doc`. The image
/// is rendered again only when the fold, the view, the inside color or the print changed.
#[allow(clippy::too_many_arguments)]
fn draw(ui: &Ui, rect: Rect, doc: &Arc<vectorcraft_engine::doc::Document>, key: FoldKey, blank: &Blank, f: &Folded, set: &Settings, fold_ms: f64) {
    // Pixels per point of the view, lowered when the window has more pixels than the view may have
    // in all or along either side, so the image keeps the window's proportions.
    let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
    let mut ppp = f64::from(ui.ctx().pixels_per_point());
    let area = w * h * ppp * ppp;
    if area > f64::from(MAX_PIXELS) {
        ppp *= (f64::from(MAX_PIXELS) / area).sqrt();
    }
    ppp = ppp.min(f64::from(MAX_SIDE) / w.max(h).max(1.0));
    let width = ((w * ppp).floor() as u32).clamp(1, MAX_SIDE);
    let height = ((h * ppp).floor() as u32).clamp(1, MAX_SIDE);
    let Some((lo, hi)) = raster::bounds(blank, f) else { return };
    let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let radius = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() / 2.0;
    let mut fit = FIT * f64::from(width.min(height)) / radius.max(1e-6);
    if set.perspective > 0.0 {
        // The eye at two radii from the middle or farther, so at a zoom of 1 the whole blank is in
        // front of it.
        let focal = f64::from(width.min(height)) / 2.0 / (set.perspective.to_radians() / 2.0).tan();
        fit = fit.min(focal / (2.0 * radius.max(1e-6)));
    }
    let camera = Camera {
        width,
        height,
        rotate_x: set.rotate_x,
        rotate_y: set.rotate_y,
        scale: fit * set.zoom,
        pan: [set.pan[0] * ppp, set.pan[1] * ppp],
        perspective: set.perspective,
        center,
    };
    let inside = inside_color(doc, &set.inside).unwrap_or([255; 4]);
    // While the print of a new revision renders, the last one of the same blank and setting is
    // drawn; a print of another document, blank or Show Fold Lines setting is not.
    let print = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let region = print_region(blank)?;
        let (pw, ph, scale) = print_size(region);
        let pkey = PrintKey {
            uid: key.0,
            revision: key.1,
            fold_lines: set.show_fold_lines,
            region: [region.x0.to_bits(), region.y0.to_bits(), region.x1.to_bits(), region.y1.to_bits()],
            width: pw,
            height: ph,
        };
        c.update_print(ui.ctx(), doc, pkey, region, scale);
        c.print_for(&pkey)
    });
    let draw_key = DrawKey { fold: key, camera, inside, print: print.as_ref().map(|(k, _)| *k) };
    let kept = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let Some(tex) = c.tex.as_ref().filter(|_| c.drawn == Some(draw_key)).map(|t| t.id()) else { return false };
        ui.painter().image(tex, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        if let Some((_, at)) = &mut c.image {
            *at = rect;
        }
        true
    });
    if kept {
        return;
    }
    let t0 = crate::now_ms();
    let scene = raster::Scene { blank, folded: f, print: print.as_ref().map(|(_, p)| &**p), inside, outside: [255; 4], camera };
    let Some(img) = raster::render(&scene) else { return };
    let draw_ms = crate::now_ms() - t0;
    let color = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.drawn = Some(draw_key);
        c.renders += 1;
        match &mut c.tex {
            Some(tex) => tex.set(color, egui::TextureOptions::LINEAR),
            None => c.tex = Some(ui.ctx().load_texture("fold-preview", color, egui::TextureOptions::LINEAR)),
        }
        if let Some(tex) = &c.tex {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(tex.id(), rect, uv, Color32::WHITE);
        }
        c.frame = Some(Frame { width: img.width, height: img.height, fold_ms, draw_ms });
        c.image = Some((img, rect));
    });
}

#[cfg(test)]
pub(crate) fn prints_rendered() -> u64 {
    CACHE.with(|c| c.borrow().prints)
}

#[cfg(test)]
pub(crate) fn last_image() -> Option<Image> {
    CACHE.with(|c| c.borrow().image.as_ref().map(|(i, _)| i.clone()))
}

/// The folds worked out and the images rendered so far.
#[cfg(test)]
pub(crate) fn work_done() -> (u64, u64) {
    CACHE.with(|c| (c.borrow().folds, c.borrow().renders))
}

/// The screen rect the last image filled.
#[cfg(test)]
pub(crate) fn last_rect() -> Option<Rect> {
    CACHE.with(|c| c.borrow().image.as_ref().map(|(_, r)| *r))
}

#[cfg(test)]
mod tests;
