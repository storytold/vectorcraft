//! VectorCraft's egui frontend: an Illustrator-style UI over `vectorcraft-engine`.
//!
//! The UI is thin: every action goes through [`VectorcraftApp::run`], which dispatches UI commands
//! (view/window) here and everything else to the engine. The same entry point serves menus,
//! shortcuts, the ⌘K palette and the control channel ([`control`]).
#![forbid(unsafe_code)]

/// Translate a UI string literal into the language the UI is drawn in (see [`i18n`]).
#[macro_export]
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

pub mod background;
mod brand;
pub mod canvas;
pub mod chrome;
pub mod community;
pub mod control;
pub mod cursors;
pub mod dialogs;
pub mod dock;
pub mod find_font;
pub mod floating;
pub mod i18n;
pub mod icon_data;
pub mod icons;
pub mod io;
pub mod menus;
pub mod palette;
pub mod panels;
pub mod place;
pub mod prefs_dialog;
pub mod print;
pub mod recovery;
pub mod render_worker;
pub mod shortcut_editor;
pub mod shortcuts;
pub mod state;
pub mod sysclip;
pub mod theme;
pub mod titlebar;
pub mod toolbar;
mod ui_fonts;
pub mod unsaved;
pub mod widgets;
pub mod workspaces;

#[cfg(test)]
mod tests_adjust;
#[cfg(test)]
mod tests_aireopen;
#[cfg(test)]
mod tests_aisave;
#[cfg(test)]
mod tests_background;
#[cfg(test)]
mod tests_clipboard;
#[cfg(test)]
mod tests_contextmenu;
#[cfg(test)]
mod tests_cut;
#[cfg(test)]
mod tests_distortkeys;
#[cfg(test)]
mod tests_docsetup;
#[cfg(test)]
mod tests_floating;
#[cfg(test)]
mod tests_fonts;
#[cfg(test)]
mod tests_home;
#[cfg(test)]
mod tests_labels;
#[cfg(test)]
mod tests_nativeoptions;
#[cfg(test)]
mod tests_numfields;
#[cfg(test)]
mod tests_overprint;
#[cfg(test)]
mod tests_paintchips;
#[cfg(test)]
mod tests_pastechords;
#[cfg(test)]
mod tests_pdfoutput;
#[cfg(test)]
mod tests_place;
#[cfg(test)]
mod tests_plugins;
#[cfg(test)]
mod tests_printps;
#[cfg(test)]
mod tests_printtiling;
#[cfg(test)]
mod tests_puppetwarp;
#[cfg(test)]
mod tests_recolor;
#[cfg(test)]
mod tests_recovery;
#[cfg(test)]
mod tests_save;
#[cfg(test)]
mod tests_saveext;
#[cfg(test)]
mod tests_slices;
#[cfg(test)]
mod tests_svg;
#[cfg(test)]
mod tests_svgsave;
#[cfg(test)]
mod tests_synthetic;
#[cfg(test)]
mod tests_sysclip;
#[cfg(test)]
mod tests_sysclip_emf;
#[cfg(test)]
mod tests_transparencygrid;
#[cfg(test)]
mod tests_widthtool;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::{Session, ViewInfo};

pub use control::{ControlRequest, ControlResponse};
pub use state::{UiState, View};
pub use sysclip::SystemClipboard;

/// What a file dialog shows: a suggested file name (save dialogs), the folder to start in and the
/// file-type filters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilePick {
    pub name: String,
    pub folder: Option<String>,
    /// `(label, extensions without the dot)`, the default type first; empty = every file.
    pub filters: Vec<(&'static str, &'static [&'static str])>,
}

impl FilePick {
    /// A save dialog suggesting `name`.
    pub fn named(name: &str) -> Self {
        Self { name: name.to_string(), ..Default::default() }
    }
}

pub type PickOpen = Box<dyn FnMut(&FilePick) -> Option<String>>;
pub type PickSave = Box<dyn FnMut(&FilePick) -> Option<String>>;
pub type ReadFn = Box<dyn Fn(&str) -> Result<Vec<u8>, String>>;
pub type WriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;
pub type Inbox = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
pub type DownloadFn = Box<dyn FnMut(&str, &[u8])>;

/// Opens a URL in the system browser.
pub type OpenUrlFn = Box<dyn FnMut(&str)>;

/// Shows a file in the system file manager, or opens it in its app.
pub type RevealFn = Box<dyn FnMut(&str) -> Result<(), String>>;

/// Platform services injected by the host app (desktop or web).
#[derive(Default)]
pub struct Services {
    /// Show an open dialog; returns a path.
    pub pick_open: Option<PickOpen>,
    /// Show a save dialog (suggested name, folder, file types); returns a path.
    pub pick_save: Option<PickSave>,
    pub read: Option<ReadFn>,
    pub write: Option<WriteFn>,
    /// Files that arrived asynchronously (web open / drops): (name, bytes).
    pub inbox: Option<Inbox>,
    /// Web: trigger a browser download instead of writing a path.
    pub download: Option<DownloadFn>,
    /// Web: start an async open (bytes arrive via `inbox`).
    pub open_async: Option<Box<dyn FnMut()>>,
    /// Read the system clipboard's text (desktop). Without it, pasted text only arrives with
    /// egui's Paste event (web, and keyboard paste everywhere).
    pub clipboard_read: Option<Box<dyn FnMut() -> Option<String>>>,
    /// Open a URL in the system browser (desktop). Without it, egui opens it (a new tab on the web).
    pub open_url: Option<OpenUrlFn>,
    /// Show an open dialog for several files (File → Place…); returns their paths (none: cancelled).
    pub pick_open_multi: Option<Box<dyn FnMut() -> Vec<String>>>,
    /// Web: start an async pick of files to place (they arrive via `place_inbox`).
    pub place_async: Option<Box<dyn FnMut()>>,
    /// Files that arrived asynchronously to be placed (web: picked for Place, or dropped on the
    /// canvas).
    pub place_inbox: Option<place::PlaceInbox>,
    /// The system clipboard with all its formats (desktop): Copy offers text, SVG, PDF and PNG,
    /// Paste takes SVG, PDF, text and bitmaps from other apps. Without it, SVG text only (through
    /// egui and `clipboard_read`).
    pub system_clipboard: Option<Box<dyn SystemClipboard>>,
    /// File → Show in Folder: select a file in the system file manager (desktop).
    pub reveal: Option<RevealFn>,
    /// Write a file from any thread (desktop): lets Background Save and Export write off the UI
    /// thread ([`background`]). Without it they run at once.
    pub write_shared: Option<background::SharedWriteFn>,
    /// Open a file (or a folder) in the system's default app for it (desktop: Edit Original, Show
    /// Package). Without it (the web) those answer with an error.
    pub open_file: Option<RevealFn>,
    /// Show a folder picker; returns its path (desktop: Relink to Folder, Package).
    pub pick_folder: Option<Box<dyn FnMut() -> Option<String>>>,
    /// Where Data Recovery keeps its copies when not in a folder (the web's browser storage;
    /// tests): installed into the session ([`vectorcraft_engine::cmd::recovery`]).
    pub recovery_store: Option<std::sync::Arc<dyn vectorcraft_engine::cmd::recovery::RecoveryStore>>,
    /// File → Print: the system's printers and print queue (desktop), the browser's print dialog
    /// (web). Without it Print saves the job as a PDF.
    pub print: Option<Box<dyn print::PrintService>>,
}

/// Cached canvas raster.
pub struct CanvasCache {
    pub renderer: vectorcraft_render::Renderer,
    pub texture: Option<egui::TextureHandle>,
    pub key: Option<CacheKey>,
    pub last_ms: f64,
    pub worker: Option<render_worker::Worker>,
    pub worker_started: bool,
    /// The slices as laid out for (document uid, revision): the canvas draws them every frame.
    pub slices: Option<SliceCache>,
    /// The print tiling's pages for (document uid, revision) (View → Show Print Tiling).
    pub print_tiling: Option<PrintTilingCache>,
    /// [`VectorcraftApp::selection_box`] for (document uid, revision, Use Preview Bounds).
    pub selection_box: Option<((u64, u64, bool), Option<vectorcraft_doc::OrientedBox>)>,
    /// The artboard under the last right-click on empty canvas: the context menu lists its
    /// commands (Duplicate, Rename, Lock, Export, Delete).
    pub context_artboard: Option<usize>,
    /// The artboard whose name is being edited in place on the canvas, and the text so far.
    pub renaming_artboard: Option<(usize, String)>,
}

/// [`CanvasCache::slices`]: the layout of the slices of (document uid, revision).
pub type SliceCache = ((u64, u64), std::sync::Arc<Vec<vectorcraft_doc::SliceArea>>);

/// [`CanvasCache::print_tiling`]: the print tiling of (document uid, revision).
pub type PrintTilingCache = ((u64, u64), std::sync::Arc<Vec<vectorcraft_pdf::TilingPage>>);

#[derive(Clone, Debug, PartialEq)]
pub struct CacheKey {
    pub doc: usize,
    pub revision: u64,
    pub zoom: f64,
    pub cx: f64,
    pub cy: f64,
    pub w: u32,
    pub h: u32,
    pub outline: bool,
    pub trim: bool,
    pub ppp: f32,
    pub hidden: Vec<u64>,
    pub rot: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Perf {
    pub frame_ms: f64,
    pub render_ms: f64,
    pub fps: f64,
}

pub struct VectorcraftApp {
    pub session: Session,
    pub ui: UiState,
    pub views: Vec<View>,
    pub services: Services,
    pub canvas: CanvasCache,
    pub perf: Perf,
    /// macOS: draw our own title strip under the traffic lights.
    pub integrated_titlebar: bool,
    /// The host installed a native menu bar (macOS): don't draw in-window menus.
    pub native_menu: bool,
    /// Last applied effect (Effect → Apply Last Effect).
    pub last_effect: Option<(String, serde_json::Value)>,
    /// Commands whose shortcuts the native menu handles (skip them in egui to avoid double firing).
    pub native_shortcuts: std::collections::HashSet<String>,
    control_rx: Option<Receiver<ControlRequest>>,
    /// (token, path, reply, deadline ms): a window that isn't presented never delivers its frame.
    pending_screenshots: Vec<(u64, Option<String>, Sender<ControlResponse>, f64)>,
    queued_screenshots: Vec<(u64, f64, u32)>,
    screenshot_token: u64,
    /// Synthetic input events (from the control channel) injected one step per frame.
    pub synthetic: Vec<egui::Event>,
    styled: bool,
    fonts_ready: bool,
    /// Installed fonts added to the UI's for characters its own fonts lack (CJK names…).
    ui_fonts: ui_fonts::UiFonts,
    frame: u64,
    last_time: f64,
    /// Canvas rect of the last frame (screen points), for control-channel coordinate mapping.
    pub canvas_rect: Option<egui::Rect>,
    /// Hover position in document coordinates.
    pub hover_doc: Option<vectorcraft_geom::Point>,
    /// System clipboard: SVG to publish next frame, the last SVG we published (so pasting it back
    /// uses the lossless internal clipboard) and text that arrived with a Paste event.
    clipboard_out: Option<String>,
    clipboard_published: Option<String>,
    pub(crate) clipboard_in: Option<String>,
    /// A URL to open through egui next frame (when the host has no `open_url` service).
    pending_url: Option<String>,
    /// Windows and Linux: the window has no OS decorations, so the app bar is the title bar (drag,
    /// double-click to maximize, caption buttons) and invisible edge zones resize the window.
    pub custom_titlebar: bool,
    /// File → Place: picked files, the place cursor's thumbnails, the Control bar's image details.
    pub place: place::PlaceState,
    /// The Paste commands can paste from the system clipboard alone: it holds something to paste
    /// (SVG without a `system_clipboard`) while the internal clipboard is empty. It enables the
    /// Paste menu items.
    pub(crate) system_paste: bool,
    /// When `system_paste` was last checked (app time, s; at most once per frame).
    system_paste_at: f64,
    /// Keyboard pastes of something other than text (see [`shortcuts::PasteChord`]).
    pub(crate) paste_chord: shortcuts::PasteChord,
    /// Saves and exports running in the background (Preferences → File Handling).
    pub background: background::Background,
    /// Data Recovery's timer and startup question ([`recovery`]).
    pub recovery: recovery::Timer,
    /// Modifiers the keyboard holds (from the host's input), given back after synthetic input.
    host_modifiers: egui::Modifiers,
    /// Synthetic input set the modifiers egui holds (see [`Self::raw_input_hook`]).
    synthetic_modifiers: bool,
    /// The marked text the system IME last sent (`None` once it commits or clears). When the Type
    /// tool stops composing on its own (a click, a tool switch), the IME is told to drop it.
    pub(crate) ime_marked: Option<String>,
    /// The IME must drop its marked text (see [`Self::take_ime_discard`]).
    pub(crate) ime_discard: bool,
}

/// Seconds between two looks at the system clipboard for [`VectorcraftApp::system_paste`].
const SYSTEM_CLIPBOARD_POLL: f64 = 0.25;

impl VectorcraftApp {
    pub fn new(mut session: Session, services: Services) -> Self {
        // The font menus and the first file opened need the installed fonts: catalog them now.
        vectorcraft_text::FontDb::global().scan_in_background();
        if let Some(store) = &services.recovery_store {
            session.recovery.set_store(store.clone());
        }
        let views = session.documents().iter().map(View::of).collect();
        Self {
            session,
            ui: UiState::default(),
            views,
            services,
            canvas: CanvasCache {
                renderer: vectorcraft_render::Renderer::new(),
                texture: None,
                key: None,
                last_ms: 0.0,
                worker: None,
                worker_started: false,
                slices: None,
                print_tiling: None,
                selection_box: None,
                context_artboard: None,
                renaming_artboard: None,
            },
            perf: Perf::default(),
            integrated_titlebar: false,
            native_menu: false,
            last_effect: None,
            native_shortcuts: Default::default(),
            clipboard_out: None,
            clipboard_published: None,
            clipboard_in: None,
            pending_url: None,
            control_rx: None,
            pending_screenshots: vec![],
            queued_screenshots: vec![],
            screenshot_token: 0,
            synthetic: vec![],
            styled: false,
            fonts_ready: false,
            ui_fonts: Default::default(),
            frame: 0,
            last_time: 0.0,
            canvas_rect: None,
            hover_doc: None,
            custom_titlebar: false,
            place: Default::default(),
            system_paste: false,
            system_paste_at: f64::NEG_INFINITY,
            paste_chord: Default::default(),
            background: Default::default(),
            recovery: Default::default(),
            host_modifiers: Default::default(),
            synthetic_modifiers: false,
            ime_marked: None,
            ime_discard: false,
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Keep `views` aligned with the session's documents (a new one starts at its saved view).
    pub fn sync_views(&mut self) {
        let docs = self.session.documents();
        self.views.truncate(docs.len());
        self.views.extend(docs.iter().skip(self.views.len()).map(View::of));
    }

    pub fn view(&self) -> Option<&View> {
        self.session.active_index().and_then(|i| self.views.get(i))
    }
    pub fn view_mut(&mut self) -> Option<&mut View> {
        self.sync_views();
        let i = self.session.active_index()?;
        self.views.get_mut(i)
    }

    pub fn view_info(&self) -> ViewInfo {
        ViewInfo {
            zoom: self.view().map(|v| v.zoom).unwrap_or(1.0),
            outline: self.ui.view.outline,
            smart_guides: self.ui.view.smart_guides,
            snap_to_grid: self.ui.view.snap_to_grid,
            snap_to_pixel: self.ui.view.snap_to_pixel,
            show_bbox: self.ui.view.bounding_box,
            snap_to_point: self.ui.view.snap_to_point,
            corner_widgets: self.ui.view.corner_widgets,
            screen: self.screen_frame(),
        }
    }

    /// The canvas on screen in document coordinates (none before it is laid out).
    pub fn screen_frame(&self) -> Option<vectorcraft_tools::ScreenFrame> {
        let (rect, view) = (self.canvas_rect?, self.view()?);
        let xf = canvas::Xf::new(rect, view);
        let px = |x: f32, y: f32| xf.delta_to_doc(egui::vec2(x, y));
        Some(vectorcraft_tools::ScreenFrame {
            origin: xf.to_doc(rect.left_top()),
            right: px(1.0, 0.0),
            down: px(0.0, 1.0),
            size: (f64::from(rect.width()), f64::from(rect.height())),
        })
    }

    /// Run a UI or engine command by id. The single entry point for every frontend path.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        if let Some(r) = menus::run_ui_command(self, id, &params) {
            return r;
        }
        let mut params = params;
        if id.starts_with("edit.paste") {
            if let Err(e) = self.adopt_system_clipboard() {
                self.ui.status = e.clone();
                return Err(e);
            }
            // Paste (also without formatting) goes to the centre of the view.
            if matches!(id, "edit.paste" | "edit.pasteWithoutFormatting")
                && ["center", "dx", "dy"].iter().all(|k| params.get(k).is_none())
                && let Some(c) = self.view().filter(|v| v.fitted).map(|v| v.center)
                && let Some(p) = params.as_object_mut()
            {
                p.insert("center".into(), serde_json::json!([c.x, c.y]));
            }
            if let Some(r) = dialogs::swatch_conflict::ask(self, id, &params) {
                return r;
            }
        }
        // Native files carry the view they reopen at.
        if fileio::SaveMode::of(id).is_some() {
            io::remember_view(self);
        }
        let r = self.session.execute(id, &params).map_err(|e| e.to_string());
        if r.is_ok() && matches!(id, "edit.copy" | "edit.cut") {
            self.publish_clipboard();
        }
        self.sync_views();
        match &r {
            Err(e) => self.ui.status = e.clone(),
            Ok(v) => {
                if id == "file.new" {
                    self.ui.status.clear();
                    // New Document's Pixel preview mode (Overprint Preview is the engine's).
                    if v["previewMode"] == "pixel" {
                        self.ui.view.pixel_preview = true;
                    }
                }
                if id == "text.setStyle"
                    && let Some(font) = params.get("font").and_then(Value::as_str)
                {
                    let r = &mut self.ui.recent_fonts;
                    r.retain(|f| f != font);
                    r.insert(0, font.to_string());
                    r.truncate(10);
                }
            }
        }
        r
    }

    /// Open a link in the browser (Help → Discord, website, GitHub…).
    pub fn open_url(&mut self, url: &str) {
        match self.services.open_url.as_mut() {
            Some(open) => open(url),
            None => self.pending_url = Some(url.to_string()),
        }
        self.ui.status = format!("Opened {url}");
    }

    /// Run a Help link command and open the URL it returns.
    pub fn open_link(&mut self, id: &str) {
        if let Ok(v) = self.run(id, serde_json::json!({}))
            && let Some(u) = v["url"].as_str()
        {
            let u = u.to_string();
            self.open_url(&u);
        }
    }

    /// Select a tool (also used by the toolbar and shortcuts).
    pub fn select_tool(&mut self, id: &str) {
        let v = self.view_info();
        if let Err(e) = self.session.select_tool(id, v) {
            self.ui.status = e.to_string();
        }
        if let Some(g) = vectorcraft_tools::catalog::group_of(id)
            && let Some(slot) = self.ui.group_tool.get_mut(g)
        {
            *slot = id.to_string();
        }
        toolbar::remember(self, id);
        self.ui.flyout = None;
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            // Guarded per request: a panic must not drop the channel (taken out of `self` above).
            let outcome = vectorcraft_engine::guard::catch_panic(|| control::handle(self, ctx, &req))
                .unwrap_or_else(|msg| control::err(format!("internal error: {msg} (please report this bug)")));
            match outcome {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path } => {
                    self.screenshot_token += 1;
                    let token = self.screenshot_token;
                    let settle = ctx.global_style().animation_time as f64 * 2000.0 + 80.0;
                    self.queued_screenshots.push((token, now_ms() + settle, 0));
                    self.pending_screenshots.push((token, path, reply, now_ms() + settle + 8000.0));
                }
            }
        }
        self.control_rx = Some(rx);
    }

    fn issue_screenshots(&mut self, ctx: &egui::Context) {
        let now = now_ms();
        self.queued_screenshots.retain_mut(|(token, at, frames)| {
            *frames += 1;
            if now >= *at && *frames >= 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(*token)));
                false
            } else {
                true
            }
        });
        if !self.queued_screenshots.is_empty() || !self.pending_screenshots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        let events: Vec<_> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied()?;
                        Some((token, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (token, image) in events {
            if let Some(i) = self.pending_screenshots.iter().position(|(t, ..)| *t == token) {
                let (_, path, reply, _) = self.pending_screenshots.remove(i);
                let _ = reply.send(control::save_screenshot(self, &image, path.as_deref()));
            }
        }
        let now = now_ms();
        self.pending_screenshots.retain(|(_, _, reply, deadline)| {
            if now < *deadline {
                return true;
            }
            let _ = reply.send(serde_json::json!({
                "ok": false,
                "error": "no frame was presented (screen locked, window minimized or fully covered); ui.render still renders the artboard"
            }));
            false
        });
    }

    /// Did the Type tool end an IME composition on its own this frame? Interrupting the IME
    /// through egui (`should_interrupt_composition`) doesn't reach the macOS input context, which
    /// keeps the marked text and types it again into the next composition: the host discards it.
    pub fn take_ime_discard(&mut self) -> bool {
        std::mem::take(&mut self.ime_discard)
    }

    /// Show a transient status message.
    pub fn status(&mut self, s: impl Into<String>) {
        self.ui.status = s.into();
    }

    fn drain_inbox(&mut self) {
        let arrived: Vec<(String, Vec<u8>)> =
            self.services.inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (name, bytes) in arrived {
            if let Err(e) = io::open_bytes(self, &name, &bytes, None) {
                self.status(format!("Couldn't open {name}: {e}"));
            }
        }
        place::drain(self);
    }
}

pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

impl VectorcraftApp {
    /// The selection's bounding box, rotated with rotated objects ([`Session::transform_box`]). The
    /// canvas and the transform fields read it every frame: it is measured once per revision.
    pub fn selection_box(&mut self) -> Option<vectorcraft_doc::OrientedBox> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision, self.session.prefs.use_preview_bounds);
        if let Some((k, b)) = self.canvas.selection_box
            && k == key
        {
            return b;
        }
        let b = self.session.transform_box(&st.selection.objects);
        self.canvas.selection_box = Some((key, b));
        b
    }
}

/// eframe isn't a dependency of this crate (the host owns the event loop); these entry points are
/// called from the host's `eframe::App` impl.
impl VectorcraftApp {
    /// The language the UI is drawn in: the Preferences dialog's choice while it is open (so a
    /// change shows before OK), else the `interfaceLanguage` preference (`auto` = the system's).
    pub fn ui_language(&self) -> i18n::Lang {
        let editing =
            self.ui.dialog.as_ref().filter(|d| d.kind == "preferences").and_then(|d| d.fields.get("interfaceLanguage")).and_then(Value::as_str);
        i18n::Lang::from_pref(editing.unwrap_or(&self.session.prefs.interface_language))
    }

    /// Per-frame logic before layout (control channel, shortcuts, inbox). A bug that panics costs
    /// one frame and shows an error, instead of closing the app with unsaved work.
    pub fn logic(&mut self, ctx: &egui::Context) {
        if let Err(msg) = vectorcraft_engine::guard::catch_panic(|| self.logic_frame(ctx)) {
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
    }

    fn logic_frame(&mut self, ctx: &egui::Context) {
        i18n::set_current(self.ui_language());
        if !self.styled {
            theme::install_fonts(ctx);
            theme::apply(ctx, self.ui.brightness);
            egui_extras::install_image_loaders(ctx);
            self.styled = true;
        } else {
            self.fonts_ready = true;
        }
        self.frame += 1;
        let now = ctx.input(|i| i.time);
        let dt = now - self.last_time;
        if dt > 0.0 {
            self.perf.fps = self.perf.fps * 0.9 + (1.0 / dt).min(240.0) * 0.1;
        }
        self.last_time = now;
        self.sync_views();
        // Read the system clipboard only when that alone decides whether Paste is enabled, and at
        // most a few times a second (opening it locks it against other apps on some systems).
        if !(0.0..SYSTEM_CLIPBOARD_POLL).contains(&(now - self.system_paste_at)) {
            self.system_paste_at = now;
            self.system_paste = self.session.clipboard.is_empty() && self.session.active().is_some() && self.system_clipboard_pasteable();
        }
        background::poll(self);
        if !self.background.jobs.is_empty() {
            // Keep the status bar's progress moving and pick the result up when it arrives.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // The window's close button (or the system quitting the app) asks about unsaved documents,
        // once the saves running in the background are done.
        if ctx.input(|i| i.viewport().close_requested()) {
            background::wait_all(self);
            if unsaved::any_dirty(self) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if let Err(e) = unsaved::close_all(self, "quit") {
                    self.status(e);
                }
            } else {
                // Quitting with nothing unsaved: no copies to leave behind.
                vectorcraft_engine::cmd::recovery::forget_all(&mut self.session);
            }
        }
        if let Some(wait) = recovery::frame(self, now) {
            // The timer and heartbeat run in an idle window too.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        shortcut_editor::sync(&self.ui);
        prefs_dialog::apply_runtime(self, ctx);
        self.drain_control(ctx);
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        }
        self.collect_screenshots(ctx);
        self.issue_screenshots(ctx);
        if let Some(u) = self.pending_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(u));
        }
        if let Some(t) = self.clipboard_out.take() {
            ctx.copy_text(t);
        }
        self.drain_inbox();
        if self.fonts_ready {
            shortcuts::handle(self, ctx);
        }
        // Native only: the web host reads dropped files asynchronously and feeds the inboxes.
        #[cfg(not(target_arch = "wasm32"))]
        self.take_dropped_files(ctx);
    }

    /// Files dropped on the window: placed where they were dropped on the canvas, else opened.
    #[cfg(not(target_arch = "wasm32"))]
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let (dropped, pos, shift) = ctx.input(|i| (i.raw.dropped_files.clone(), i.pointer.latest_pos(), i.modifiers.shift));
        if dropped.is_empty() {
            return;
        }
        let target = self.drop_target(pos, shift);
        let mut files = vec![];
        for f in dropped {
            let path = Some(f.path().to_string_lossy().to_string()).filter(|s| !s.is_empty());
            let name = path.as_deref().map_or_else(|| "dropped".into(), vectorcraft_engine::cmd::fileio::file_name);
            // A file placed by its path is read by the engine.
            let bytes = if path.is_some() && target != place::DropTarget::Open { Ok(vec![]) } else { f.bytes() };
            match bytes {
                Ok(b) => files.push((name, path, b)),
                Err(e) => self.status(format!("Couldn't read {name}: {e}")),
            }
        }
        place::drop_files(self, files, target);
    }

    /// Inject synthetic events (one press/release step per frame). Handlers read the modifiers
    /// egui holds (`i.modifiers`), so a synthetic key or button holds its own for the frames it
    /// spans (a drag's moves included); the keyboard's come back after.
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        for e in &raw.events {
            match e {
                egui::Event::ModifiersChanged(m) => self.host_modifiers = *m,
                egui::Event::WindowFocused(false) => self.host_modifiers = egui::Modifiers::NONE,
                _ => {}
            }
        }
        let Some(first) = self.synthetic.first() else {
            if std::mem::take(&mut self.synthetic_modifiers) {
                raw.events.push(egui::Event::ModifiersChanged(self.host_modifiers));
            }
            return;
        };
        // Pointer events go one per frame so egui sees presses, drags and releases as real input;
        // keyboard sequences go up to the key release.
        let n = match first {
            egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        if let egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. } = first {
            raw.events.push(egui::Event::PointerMoved(*p));
        }
        let (now, later) = self.synthetic.split_at(n.min(self.synthetic.len()));
        // This frame's key or button, else the button a drag holds down (released later).
        let held = now
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .or_else(|| match later.iter().find(|e| matches!(e, egui::Event::PointerButton { .. })) {
                Some(egui::Event::PointerButton { pressed: false, modifiers, .. }) => Some(*modifiers),
                _ => None,
            });
        match held {
            Some(m) => {
                raw.events.push(egui::Event::ModifiersChanged(m));
                self.synthetic_modifiers = true;
            }
            None if std::mem::take(&mut self.synthetic_modifiers) => raw.events.push(egui::Event::ModifiersChanged(self.host_modifiers)),
            None => {}
        }
        raw.events.extend(self.synthetic.drain(..n));
    }

    /// Lay out the whole window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Err(msg) = vectorcraft_engine::guard::catch_panic(|| self.ui_frame(ui)) {
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
    }

    fn ui_frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.fonts_ready {
            ctx.request_repaint();
            return;
        }
        let t0 = now_ms();
        let t = theme::Tokens::get(&ctx);
        floating::begin_frame(&ctx);
        if self.ui.screen_mode < 2 {
            chrome::app_bar(self, ui);
            if self.ui.control_bar {
                chrome::control_bar(self, ui);
            }
        }
        if self.ui.status_bar && self.ui.screen_mode < 3 {
            chrome::status_bar(self, ui);
            chrome::hint_bar(self, ui);
        }
        if self.ui.toolbar && self.ui.screen_mode < 3 {
            toolbar::show(self, ui);
        }
        if self.ui.dock && self.ui.screen_mode < 3 {
            dock::show(self, ui);
        }
        floating::toolbar_column(self, ui);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| {
            if self.ui.screen_mode < 3 {
                chrome::doc_tabs(self, ui);
            }
            canvas::show(self, ui);
        });
        floating::show(self, &ctx);
        dock::floating_panel(self, &ctx);
        panels::library_panel::show_window(self, &ctx);
        dialogs::show(self, &ctx);
        palette::show(self, &ctx);
        if self.custom_titlebar {
            titlebar::resize_zones(ui);
        }
        self.ui_fonts.frame(&ctx);
        self.perf.frame_ms = now_ms() - t0;
        let _ = json!(null);
    }
}
