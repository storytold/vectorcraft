//! The Fold Preview window: the document's blank folded into its 3D shape
//! ([`vectorcraft_three_d::fold::fold`]), with the printed art on the outside of each panel and the
//! board's inside color on the back, drawn by [`vectorcraft_three_d::fold::raster::render`].
//!
//! The window folds the blank that the document's shown fold lines make, about its largest panel
//! or the panel the user double-clicked. Its print is a raster of the document with the fold lines
//! and the layers that do not print left out ([`vectorcraft_engine::cmd::print_hidden`]), rendered
//! once per document revision on the host's job queue, and sampled through each panel's projective
//! map when the fold, the view or the print changes. Dragging turns the view, a Shift-drag
//! keeps to one axis, a middle-drag or a Space-drag moves it, the wheel zooms, a double-click on a
//! panel makes it the base, and the arrow keys turn the view by 5° when it has the keyboard focus.
//! The settings live in `UiState::fold_preview` (`ui.inspect`); `ui.foldPreview` sets them.

use std::ops::RangeInclusive;
use std::sync::Arc;

use craft::behavior::{Metrics, NavMove, PointerButton, PressPhase, Sense};
use craft::egui::{ColorImage, Pos2, Rect, Response, Ui, Vec2, pos2};
use craft::host::queue::{JobQueue, Priority, QueueOptions};
use craft::host::{Wake, default_jobs};
use craft::keymap::Key;
use craft::platform::clock::monotonic_ms;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, Common, Component, Cx, Entry, Focus as _, Input, MeasureCx, NoValue, Outcome, PaintUi, Pointer as _, Props as _, Role, Show as _,
    ShowContainer as _, Sizing, WordSetCx as _,
};
use craft::ui::display::{Shade, Text};
use craft::ui::form::{SliderField, SliderFieldLayout};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::Select;
use craft::ui::primitives::{Revision, TextureStore, TextureStoreOptions};
use craft::ui::runtime::env::{Env, Label, UnitKind};
use craft::ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft::ui::runtime::keys;
use craft::ui::style::{ComponentStyle, FocusLook, StyleCx, SurfaceKind};
use craft::ui::value_drag::SliderScale;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect as DRect};
use vectorcraft_three_d::fold::raster::{self, Camera, Image, MAX_PIXELS, MAX_SIDE, Print};
use vectorcraft_three_d::fold::{Blank, Folded, fold, pick};

use crate::VectorcraftApp;
use crate::thumb_worker::Settle;

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
/// The texture store the drawn image lives in, and its one key.
const STORE: &str = "fold-preview";

craft::ui::words! {
    /// The words of the Fold Preview window.
    pub struct FoldPreviewWords {
        image: "foldPreview.image" = "Fold Preview";
        view: "foldPreview.view" = "View";
        inside: "foldPreview.inside" = "Inside";
        white: "foldPreview.white" = "White";
        show_fold_lines: "foldPreview.showFoldLines" = "Show Fold Lines";
        reset: "foldPreview.reset" = "Reset";
        perspective: "foldPreview.perspective" = "Perspective";
        fold: "foldPreview.fold" = "Fold";
        zoom: "foldPreview.zoom" = "Zoom";
        front: "foldPreview.front" = "Front";
        back: "foldPreview.back" = "Back";
        left: "foldPreview.left" = "Left";
        right: "foldPreview.right" = "Right";
        top: "foldPreview.top" = "Top";
        bottom: "foldPreview.bottom" = "Bottom";
        three_quarter: "foldPreview.threeQuarter" = "Three-Quarter";
    }
}

impl FoldPreviewWords {
    /// The name of the view preset `id` (see [`VIEWS`]).
    fn view_label(&self, id: &str) -> Label {
        let word = match id {
            "front" => &self.front,
            "back" => &self.back,
            "left" => &self.left,
            "right" => &self.right,
            "top" => &self.top,
            "bottom" => &self.bottom,
            _ => &self.three_quarter,
        };
        word.clone()
    }
}

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

impl DrawKey {
    /// The revision of the image this key draws: the store renders again only when it changes.
    fn revision(&self) -> Revision {
        let c = &self.camera;
        let bits = [c.rotate_x, c.rotate_y, c.scale, c.pan[0], c.pan[1], c.perspective, c.center[0], c.center[1], c.center[2]].map(f64::to_bits);
        Revision::of(&(self.fold, c.width, c.height, bits, self.inside, self.print))
    }
}

/// The last image drawn, where it sat on screen and what it shows, for picking a panel with a
/// double-click.
#[derive(Clone)]
struct Shown {
    image: Arc<Image>,
    rect: Rect,
    fold: FoldKey,
}

/// What the window draws this pass: the folded blank and the print to put on it.
#[derive(Clone)]
struct Folding {
    doc: Arc<vectorcraft_engine::doc::Document>,
    blanks: Arc<Vec<Blank>>,
    blank: usize,
    folded: Arc<Folded>,
    key: FoldKey,
    fold_ms: f64,
    print: Option<(PrintKey, Arc<Print>)>,
}

/// The pass's folding, and what the status line says about it.
struct Scene {
    blanks: usize,
    errors: bool,
    drawn: Option<Folding>,
}

/// The Fold Preview window's state, kept with the add-on panels.
#[derive(Default)]
pub(crate) struct View {
    blanks: Option<Blanks>,
    /// The last fold, what it folds and how long it took (ms).
    folded: Option<(FoldKey, Arc<Folded>, f64)>,
    /// The print shown and what it shows.
    print: Option<(PrintKey, Arc<Print>)>,
    /// The print being rendered.
    pending: Option<PrintKey>,
    /// Renders the print off the UI thread; made on the first pass, when the host is known.
    prints: Option<JobQueue<PrintKey, Option<Print>>>,
    settle: Settle,
    /// The last image drawn.
    shown: Option<Shown>,
    frame: Option<Frame>,
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

/// The texture store the drawn image lives in.
fn store(ctx: &craft::egui::Context) -> TextureStore<u8> {
    TextureStore::in_context(ctx, STORE, TextureStoreOptions::default())
}

/// The queue the print renders on: the host's jobs, else the default ones, with one lane (none on
/// wasm32, where the pass runs the job).
fn print_queue(ui: &Ui) -> JobQueue<PrintKey, Option<Print>> {
    let ctx = ui.ctx().clone();
    let jobs = Env::of(ui).view().host().map_or_else(default_jobs, |host| Arc::clone(host.jobs_shared()));
    let options = QueueOptions { lanes: QueueOptions::default_lanes().min(1), name: "fold-print" };
    JobQueue::new(jobs, Wake::new(move || ctx.request_repaint()), options)
}

impl View {
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
        let t0 = monotonic_ms();
        let f = Arc::new(fold(blank, Some(panel), amount, TOLERANCE)?);
        let ms = monotonic_ms() - t0;
        self.folded = Some((key, f.clone(), ms));
        Some((f, ms))
    }

    /// What the window draws for `doc` at `revision` with `set`: its blanks, the fold and the print.
    fn scene(&mut self, ui: &Ui, doc: &Arc<vectorcraft_engine::doc::Document>, uid: u64, revision: u64, set: &Settings) -> Scene {
        let (blanks, errors) = self.blanks(uid, revision, doc);
        let base = set.base.map(|[x, y]| Point::new(x, y));
        let picked = pick(&blanks, base).or_else(|| pick(&blanks, None));
        let drawn = picked.and_then(|(b, panel)| {
            let blank = blanks.get(b)?;
            let key = (uid, revision, b, panel, set.amount.to_bits());
            let (folded, fold_ms) = self.fold(key, blank, panel, set.amount)?;
            let print = self.print_of(ui, doc, key, blank, set.show_fold_lines);
            Some(Folding { doc: doc.clone(), blanks: blanks.clone(), blank: b, folded, key, fold_ms, print })
        });
        Scene { blanks: blanks.len(), errors, drawn }
    }

    /// The print to draw for the fold `key` of `blank`, brought up to date first.
    fn print_of(
        &mut self,
        ui: &Ui,
        doc: &Arc<vectorcraft_engine::doc::Document>,
        key: FoldKey,
        blank: &Blank,
        fold_lines: bool,
    ) -> Option<(PrintKey, Arc<Print>)> {
        let region = print_region(blank)?;
        let (width, height, scale) = print_size(region);
        let pkey = PrintKey {
            uid: key.0,
            revision: key.1,
            fold_lines,
            region: [region.x0.to_bits(), region.y0.to_bits(), region.x1.to_bits(), region.y1.to_bits()],
            width,
            height,
        };
        self.update_print(ui, doc, pkey, region, scale);
        self.print_for(&pkey)
    }

    /// Take in the prints that finished, then bring the print up to `key`: queued on the host's
    /// jobs, and not before the document has settled unless there is no print of it yet.
    fn update_print(&mut self, ui: &Ui, doc: &Arc<vectorcraft_engine::doc::Document>, key: PrintKey, region: DRect, scale: f64) {
        let queue = self.prints.get_or_insert_with(|| print_queue(ui));
        let inline = QueueOptions::default_lanes() == 0;
        if inline {
            queue.run_inline(1);
        }
        let mut done = Vec::new();
        while let Some(finished) = queue.try_recv() {
            done.push(finished);
        }
        for finished in done {
            if self.pending == Some(finished.key) {
                self.pending = None;
            }
            if let Some(print) = finished.result {
                self.keep_print(finished.key, print);
            }
        }
        let shown = self.print.as_ref().map(|(k, _)| *k);
        if shown == Some(key) || self.pending == Some(key) {
            return;
        }
        let have = shown.is_some_and(|k| k.uid == key.uid);
        if have && !self.settle.settled(ui.ctx(), key.uid, key.revision, monotonic_ms() / 1000.0) {
            return;
        }
        let opts = vectorcraft_render::RenderOptions {
            skip_templates: true,
            image_mips: true,
            hidden: vectorcraft_engine::cmd::print_hidden(doc, key.fold_lines).into_iter().collect(),
            ..Default::default()
        };
        let doc = doc.clone();
        let Some(queue) = &self.prints else { return };
        // A render of an older revision is of no use once this one is wanted.
        queue.reprioritize(|_, _| None);
        queue.submit(key, Priority::VISIBLE, move |_| {
            let img = vectorcraft_render::Renderer::new().render_region_with(&doc, region, scale, &opts);
            Print::new(region, img.width, img.height, img.pixels)
        });
        self.pending = Some(key);
        if inline {
            ui.ctx().request_repaint();
        }
    }

    /// The print to draw for `want`: the one shown when it is of the same document, blank region and
    /// Show Fold Lines setting (an older revision while the new one renders), else none.
    fn print_for(&self, want: &PrintKey) -> Option<(PrintKey, Arc<Print>)> {
        let same = |k: &PrintKey| (k.uid, k.fold_lines, k.region) == (want.uid, want.fold_lines, want.region);
        self.print.as_ref().filter(|(k, _)| same(k)).map(|(k, p)| (*k, p.clone()))
    }

    fn keep_print(&mut self, key: PrintKey, print: Print) {
        self.print = Some((key, Arc::new(print)));
    }

    /// Keep what the view drew this pass: a new image and its times, or only where the last one sits.
    fn keep(&mut self, drawn: Option<Drawn>) {
        let Some(drawn) = drawn else { return };
        match drawn.fresh {
            Some((image, frame)) => {
                self.shown = Some(Shown { image, rect: drawn.rect, fold: drawn.fold });
                self.frame = Some(frame);
            }
            None => {
                if let Some(shown) = &mut self.shown {
                    shown.rect = drawn.rect;
                }
            }
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
    let frame = app.addon_panels.fold_view.frame;
    let mut out = serde_json::to_value(&app.ui.fold_preview).unwrap_or_default();
    if let Some(o) = out.as_object_mut() {
        o.insert(
            "frame".into(),
            frame.map_or(Value::Null, |f| json!({"width": f.width, "height": f.height, "foldMs": f.fold_ms, "drawMs": f.draw_ms})),
        );
    }
    out
}

/// The toolbar: the view presets, Inside, Show Fold Lines and Reset.
fn toolbar(ui: &mut Ui, words: &FoldPreviewWords, doc: &vectorcraft_engine::doc::Document, set: &mut Settings) {
    let views: Vec<Entry<&'static str>> = VIEWS.iter().map(|v| Entry::new(v.1, words.view_label(v.1))).collect();
    let solid = |s: &&vectorcraft_color::Swatch| matches!(s.paint, vectorcraft_color::Paint::Solid { .. });
    let insides: Vec<Entry<String>> = std::iter::once(Entry::new(String::new(), words.white.clone()))
        .chain(doc.swatches.iter().filter(solid).map(|s| Entry::new(s.name.clone(), s.name.clone())))
        .collect();
    Row::new().wrap(true).show(ui, |ui, _| {
        let current = set.view().map_or("", |v| v.1);
        if let Some(&id) = Select::new(words.view.clone(), Bind::given(current), &views).show(ui).value()
            && let Some(&(_, _, x, y)) = VIEWS.iter().find(|v| v.1 == id)
        {
            (set.rotate_x, set.rotate_y, set.pan) = (x, y, [0.0; 2]);
        }
        let inside = Select::new(words.inside.clone(), Bind::given(set.inside.clone()), &insides).placeholder(set.inside.clone()).show(ui);
        if let Some(name) = inside.value() {
            set.inside.clone_from(name);
        }
        Checkbox::new(Bind::bound(&mut set.show_fold_lines)).label(words.show_fold_lines.clone()).show(ui);
        if Button::new(words.reset.clone()).show(ui).clicks > 0 {
            let fresh = Settings::default();
            (set.rotate_x, set.rotate_y, set.zoom, set.pan, set.perspective) = (fresh.rotate_x, fresh.rotate_y, 1.0, [0.0; 2], 0.0);
        }
    });
}

/// A slider over `value` as a percentage, which keeps `value` within `range` (as fractions).
// PLAN: the sheet says `Field` + `Slider`; `SliderField` is the one component with the caption, number field and slider, and the percent unit keeps the old window's look.
fn percent_slider(ui: &mut Ui, label: Label, value: &mut f64, range: RangeInclusive<f64>, scale: SliderScale) {
    let before = *value * 100.0;
    let mut shown = before;
    let bounds = (range.start() * 100.0)..=(range.end() * 100.0);
    SliderField::new(label, Bind::bound(&mut shown), bounds)
        .layout(SliderFieldLayout::Inline)
        .scale(scale)
        .step(1.0)
        .unit_kind(UnitKind::Percent)
        .show(ui);
    if shown != before {
        *value = (shown / 100.0).clamp(*range.start(), *range.end());
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

/// Shows the Fold Preview window: the toolbar, the folded blank, the status lines and the sliders.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        crate::panels::empty_state(ui, "dc-cube", tl!("No document"), tl!("Open a document to preview its folded shape."));
        return;
    };
    let (uid, revision, doc) = (st.uid, st.revision, st.doc.clone());
    let mut set = app.ui.fold_preview.clone().clamped();
    let words = Env::of(ui).words::<FoldPreviewWords>();
    let view = &mut app.addon_panels.fold_view;
    let mut drawn = None;
    Stack::new().fill().show(ui, |ui, _| {
        toolbar(ui, &words, &doc, &mut set);
        SliderField::new(words.perspective.clone(), Bind::bound(&mut set.perspective), 0.0..=160.0)
            .layout(SliderFieldLayout::Inline)
            .step(1.0)
            .unit_kind(UnitKind::Degrees)
            .show(ui);
        let scene = view.scene(ui, &doc, uid, revision, &set);
        let out = FoldView::new(words.image.clone(), set.clone(), scene.drawn.clone(), view.shown.clone()).sizing(Sizing::Fill).show(ui);
        set = out.set;
        drawn = out.drawn;
        let folded = scene.drawn.as_ref().and_then(|d| Some((d.blanks.get(d.blank)?, &*d.folded)));
        for line in status(scene.blanks, folded, scene.errors) {
            Text::new(crate::i18n::msg(&line)).shade(Shade::Muted).show(ui);
        }
        percent_slider(ui, words.fold.clone(), &mut set.amount, 0.0..=1.0, SliderScale::Linear);
        percent_slider(ui, words.zoom.clone(), &mut set.zoom, 0.1..=8.0, SliderScale::Log);
    });
    view.keep(drawn);
    if set != app.ui.fold_preview {
        app.ui.fold_preview = set;
    }
}

/// What the view drew in one pass: where, which fold, and the image when it was rendered anew.
struct Drawn {
    rect: Rect,
    fold: FoldKey,
    fresh: Option<(Arc<Image>, Frame)>,
}

/// What the folded view reports in one pass.
#[derive(Debug)]
struct FoldViewOutput {
    response: Response,
    /// The settings after this pass's gestures.
    set: Settings,
    drawn: Option<Drawn>,
}

impl std::fmt::Debug for Drawn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Drawn").field("rect", &self.rect).field("fold", &self.fold).finish_non_exhaustive()
    }
}

impl Outcome for FoldViewOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of the folded view: the room it asks for and its focus ring.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct FoldViewStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
}

/// The folded blank, drawn into the room it is given. A drag turns the view (Shift keeps to one
/// axis), a middle drag or a drag with Space held moves it, the wheel zooms, the arrow keys turn it by
/// [`KEY_TURN`], and a double click on a panel picks it as the base. The settings it reports are
/// the ones it was given with the gestures applied.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(content_painter, sample = FoldView::sample)]
struct FoldView {
    #[common]
    common: Common,
    set: Settings,
    folding: Option<Folding>,
    /// The last image drawn, which a double click picks a panel from.
    last: Option<Shown>,
    drawn: Option<Drawn>,
}

impl FoldView {
    /// A view of `folding` under `set`, named `label`.
    fn new(label: impl Into<Label>, set: Settings, folding: Option<Folding>, last: Option<Shown>) -> Self {
        FoldView { common: Common::new(), set, folding, last, drawn: None }.label(label)
    }

    /// An empty view, for the suite.
    fn sample() -> Self {
        FoldView::new("Fold Preview", Settings::default(), None, None)
    }

    /// The base point of the panel at `at` in the last image drawn, while that image shows the
    /// blank now folded of the same document revision.
    fn pick(&self, at: Pos2) -> Option<[f64; 2]> {
        let (folding, shown) = (self.folding.as_ref()?, self.last.as_ref()?);
        if (shown.fold.0, shown.fold.1, shown.fold.2) != (folding.key.0, folding.key.1, folding.key.2) {
            return None;
        }
        let (img, rect) = (&shown.image, shown.rect);
        let x = (at.x - rect.left()) / rect.width() * img.width as f32;
        let y = (at.y - rect.top()) / rect.height() * img.height as f32;
        let panel = img.panel_at(x.max(0.0) as u32, y.max(0.0) as u32)?;
        let p = folding.blanks.get(folding.blank)?.panels.get(panel)?.point_inside()?;
        Some([p.x, p.y])
    }

    /// The camera for `blank` folded as `f` in an image of `width` × `height` pixels at `ppp` pixels
    /// a point.
    fn camera(&self, blank: &Blank, f: &Folded, width: u32, height: u32, ppp: f64) -> Option<Camera> {
        let set = &self.set;
        let (lo, hi) = raster::bounds(blank, f)?;
        let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
        let radius = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() / 2.0;
        let mut fit = FIT * f64::from(width.min(height)) / radius.max(1e-6);
        if set.perspective > 0.0 {
            // The eye at two radii from the middle or farther, so at a zoom of 1 the whole blank is
            // in front of it.
            let focal = f64::from(width.min(height)) / 2.0 / (set.perspective.to_radians() / 2.0).tan();
            fit = fit.min(focal / (2.0 * radius.max(1e-6)));
        }
        Some(Camera {
            width,
            height,
            rotate_x: set.rotate_x,
            rotate_y: set.rotate_y,
            scale: fit * set.zoom,
            pan: [set.pan[0] * ppp, set.pan[1] * ppp],
            perspective: set.perspective,
            center,
        })
    }
}

impl Component for FoldView {
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    /// The view keeps the arrow keys, so they turn it rather than move the focus on.
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = FoldViewStyle;
    type Output = FoldViewOutput;

    fn style(cx: &StyleCx<'_>, _: FoldViewStyleKey) -> FoldViewStyle {
        let plot = &cx.tokens.size.plot;
        FoldViewStyle { metrics: Metrics::new().min_size(Vec2::splat(plot.width.min(plot.height))), focus: cx.focus_look() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG | Sense::FOCUS | Sense::TAKE_FOCUS | Sense::WHEEL
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let (phase, mods) = (cx.press().phase, cx.press().mods);
        match phase {
            PressPhase::Drag { start, delta, .. } => {
                let (mut dx, mut dy) = (f64::from(delta.x), f64::from(delta.y));
                let space = keys::held(&cx.response.ctx).contains(&Key::Space);
                if start.button == PointerButton::Middle || (start.button == PointerButton::Primary && space) {
                    self.set.pan[0] += dx;
                    self.set.pan[1] += dy;
                } else if start.button == PointerButton::Primary {
                    if mods.shift {
                        if dx.abs() >= dy.abs() {
                            dy = 0.0;
                        } else {
                            dx = 0.0;
                        }
                    }
                    self.set.rotate_y = wrapped(self.set.rotate_y + dx * DRAG_TURN);
                    self.set.rotate_x = wrapped(self.set.rotate_x + dy * DRAG_TURN);
                }
            }
            PressPhase::Release { start, pos, click: true, .. } if start.button == PointerButton::Primary && start.clicks == 2 => {
                if let Some(base) = self.pick(pos) {
                    self.set.base = Some(base);
                }
            }
            _ => {}
        }
        if let Some(wheel) = cx.take_wheel() {
            let scroll = f64::from(wheel.scroll.y + (wheel.zoom - 1.0) * 200.0);
            if scroll != 0.0 {
                self.set.zoom = (self.set.zoom * (scroll * 0.002).exp()).clamp(0.1, 8.0);
            }
        }
        let rtl = cx.flow.is_rtl();
        let (mut turn_y, mut turn_x) = (0.0, 0.0);
        while let Some((step, _)) = cx.intents().take_move() {
            match (step, rtl) {
                (NavMove::Back, false) | (NavMove::Forward, true) => turn_y -= KEY_TURN,
                (NavMove::Forward, false) | (NavMove::Back, true) => turn_y += KEY_TURN,
                (NavMove::Up, _) => turn_x -= KEY_TURN,
                (NavMove::Down, _) => turn_x += KEY_TURN,
                _ => {}
            }
        }
        if turn_x != 0.0 || turn_y != 0.0 {
            self.set.rotate_y = wrapped(self.set.rotate_y + turn_y);
            self.set.rotate_x = wrapped(self.set.rotate_x + turn_x);
        }
    }

    fn value_text(&self, cx: &Cx<'_, Self>) -> Option<String> {
        let words = cx.words::<FoldPreviewWords>();
        let view = self.set.view()?;
        Some(cx.env.resolve(&words.view_label(view.1)).as_str().to_owned())
    }

    /// The image is rendered at the view's size, lowered when the window has more pixels than the
    /// view may have in all or along either side, so it keeps the window's proportions; it is
    /// rendered again only when the fold, the view, the inside color or the print changed.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.draw().surface(cx.rect, SurfaceKind::Canvas, Default::default());
        let Some(folding) = &self.folding else { return };
        let Some(blank) = folding.blanks.get(folding.blank) else { return };
        let ctx = &cx.response.ctx;
        let (w, h) = (f64::from(cx.rect.width()), f64::from(cx.rect.height()));
        let mut ppp = f64::from(ctx.pixels_per_point());
        let area = w * h * ppp * ppp;
        if area > f64::from(MAX_PIXELS) {
            ppp *= (f64::from(MAX_PIXELS) / area).sqrt();
        }
        ppp = ppp.min(f64::from(MAX_SIDE) / w.max(h).max(1.0));
        let width = ((w * ppp).floor() as u32).clamp(1, MAX_SIDE);
        let height = ((h * ppp).floor() as u32).clamp(1, MAX_SIDE);
        let Some(camera) = self.camera(blank, &folding.folded, width, height, ppp) else { return };
        let inside = inside_color(&folding.doc, &self.set.inside).unwrap_or([255; 4]);
        let print = folding.print.as_ref();
        let key = DrawKey { fold: folding.key, camera, inside, print: print.map(|(k, _)| *k) };
        let mut fresh = None;
        let texture = store(ctx).get_or_put(ctx, 0, key.revision(), || {
            let t0 = monotonic_ms();
            let scene = raster::Scene { blank, folded: &folding.folded, print: print.map(|(_, p)| &**p), inside, outside: [255; 4], camera };
            let Some(img) = raster::render(&scene) else {
                return ColorImage::from_rgba_premultiplied([1, 1], &[0; 4]);
            };
            let color = ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
            let frame = Frame { width: img.width, height: img.height, fold_ms: folding.fold_ms, draw_ms: monotonic_ms() - t0 };
            fresh = Some((Arc::new(img), frame));
            color
        });
        ui.draw().texture(cx.rect, texture.id, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)));
        self.drawn = Some(Drawn { rect: cx.rect, fold: folding.key, fresh });
    }

    fn finish(self, cx: Cx<'_, Self>) -> FoldViewOutput {
        FoldViewOutput { response: cx.response.clone(), set: self.set, drawn: self.drawn }
    }
}

#[cfg(test)]
mod tests;
