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
mod clipboard_probe;
pub mod community;
pub mod control;
pub mod credits;
pub mod cursors;
pub mod dialogs;
pub mod dock;
pub mod find_font;
pub mod floating;
pub mod font_menu;
mod free_transform;
pub mod graphics;
pub mod i18n;
pub mod icon_data;
pub mod icons;
pub mod io;
pub mod menus;
pub mod native_menu;
pub mod palette;
mod panel_docking;
pub mod panels;
pub mod picks;
pub mod place;
pub mod prefs_dialog;
pub mod print;
pub mod recovery;
pub mod render_worker;
mod scrub;
pub mod shortcut_editor;
pub mod shortcuts;
pub mod state;
pub mod sysclip;
pub mod theme;
pub mod titlebar;
pub mod toolbar;
mod touch;
mod ui_fonts;
pub mod unsaved;
pub mod widgets;
pub mod workspaces;

#[cfg(test)]
mod tests_adjust;
#[cfg(test)]
mod tests_aisave;
#[cfg(test)]
mod tests_automation;
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
mod tests_font_menu;
#[cfg(test)]
mod tests_fonts;
#[cfg(test)]
mod tests_home;
#[cfg(test)]
mod tests_labels;
#[cfg(test)]
mod tests_nativemenu;
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
mod tests_pathtype;
#[cfg(test)]
mod tests_pdfoutput;
#[cfg(test)]
mod tests_picks;
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
mod tests_removeanchors;
#[cfg(test)]
mod tests_save;
#[cfg(test)]
mod tests_saveext;
#[cfg(test)]
mod tests_screenmode;
#[cfg(test)]
mod tests_scrub;
#[cfg(test)]
mod tests_selectall;
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
mod tests_sysclip_probe;
#[cfg(test)]
mod tests_transparencygrid;
#[cfg(test)]
mod tests_widthtool;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_engine::{Session, ViewInfo};

pub use control::{ControlRequest, ControlResponse};
pub use state::{UiState, View};
pub use sysclip::{ClipboardProbeFactory, SystemClipboard};

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
    /// A second system-clipboard handle, for checking whether Paste has something to take on a
    /// background thread (Linux: an X11 clipboard owner that never answers would freeze the UI).
    /// Taken on the first frame. Without it the check runs on the UI thread.
    pub clipboard_probe: Option<ClipboardProbeFactory>,
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
    /// The macOS menu bar, when the desktop app installed one: the in-window menus are hidden then.
    pub native_menu: Option<native_menu::NativeMenu>,
    /// Show file dialogs off the UI thread (desktop Linux, where a dialog in line holds the window
    /// and the compositor finds it not answering): what asked runs again with the answer
    /// ([`picks`]). Without it they are shown in line.
    pub start_pick: Option<picks::StartPick>,
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
    /// [`VectorcraftApp::selection_bounds`] for (document uid, revision).
    pub selection_bounds: Option<((u64, u64), Option<vectorcraft_geom::Rect>)>,
    /// The tools' cursors as OS cursor bitmaps.
    pub cursors: cursors::Images,
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
    /// General › Anti-aliased Artwork.
    pub anti_alias: bool,
    /// [`vectorcraft_render::placed_document::generation`]: placed documents' bitmaps made since.
    pub placed: u64,
    /// View › Pixel Preview: the document pixels rendered (x0, y0, x1, y1), one per point, shown
    /// with hard edges. None: the art is rendered for the screen.
    pub pixel: Option<[i64; 4]>,
    /// Images sampled smoothly ([`vectorcraft_render::RenderOptions::smooth_images`]).
    pub smooth_images: bool,
    /// Isolation mode's group or layer ([`vectorcraft_render::RenderOptions::isolated`]).
    pub isolated: Option<u64>,
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
    /// macOS: the room the app bar keeps at its left for the window buttons over it, in points
    /// (0 where the OS draws no buttons over the app bar).
    pub titlebar_inset: f32,
    /// Last applied effect (Effect → Apply Last Effect).
    pub last_effect: Option<(String, serde_json::Value)>,
    /// Commands run through [`Self::run`] so far: the native menu bar reads its rows again when it
    /// moves ([`native_menu::sync`]).
    run_count: u64,
    control_rx: Option<Receiver<ControlRequest>>,
    /// Window captures asked for and not answered yet.
    pending_screenshots: Vec<PendingScreenshot>,
    queued_screenshots: Vec<(u64, f64, u32)>,
    screenshot_token: u64,
    /// Synthetic input events (from the control channel) injected one step per frame.
    pub synthetic: Vec<egui::Event>,
    styled: bool,
    fonts_ready: bool,
    /// The egui context the UI's textures were uploaded to (0: none yet; see
    /// [`Self::adopt_context`]).
    context: u64,
    /// Installed fonts added to the UI's for characters its own fonts lack (CJK names…).
    ui_fonts: ui_fonts::UiFonts,
    frame: u64,
    last_time: f64,
    /// Canvas rect of the last frame (screen points), for control-channel coordinate mapping.
    pub canvas_rect: Option<egui::Rect>,
    /// Hover position in document coordinates.
    pub hover_doc: Option<vectorcraft_geom::Point>,
    /// System clipboard: SVG to publish next frame, the last SVG we published (so pasting it back
    /// uses the lossless internal clipboard) and what came with a paste (a Paste event's text, a
    /// picture or file the host read: [`Self::paste_from_host`]).
    clipboard_out: Option<String>,
    clipboard_published: Option<String>,
    pub(crate) clipboard_in: Option<vectorcraft_engine::cmd::clipboard::Flavour>,
    /// A URL to open through egui next frame (when the host has no `open_url` service).
    pending_url: Option<String>,
    /// Windows and Linux: the window has no OS decorations, so the app bar is the title bar (drag,
    /// double-click to maximize, caption buttons) and invisible edge zones resize the window.
    pub custom_titlebar: bool,
    /// Last window title sent to the OS (`ViewportCommand::Title`, see `chrome::sync_window_title`):
    /// sent again only when it changes, so idle frames don't spam the backend.
    pub(crate) last_window_title: String,
    /// The graphics adapter the window renders with ("name (backend)"), as the host reports it:
    /// shown in Help › About and `ui.inspect` for GPU bug reports. `None` when unknown.
    pub graphics_adapter: Option<String>,
    /// File → Place: picked files, the place cursor's thumbnails, the Control bar's image details.
    pub place: place::PlaceState,
    /// The Paste commands can paste from the system clipboard alone: it holds something to paste
    /// (SVG without a `system_clipboard`) while the internal clipboard is empty. It enables the
    /// Paste menu items.
    pub(crate) system_paste: bool,
    /// When `system_paste` was last checked in line (app time, s; at most once per frame).
    system_paste_at: f64,
    /// The thread that checks the system clipboard for `system_paste` instead
    /// ([`Services::clipboard_probe`]).
    clipboard_probe: Option<clipboard_probe::Probe>,
    /// File dialogs shown off the UI thread and what runs again with their answers.
    pub(crate) picks: picks::Picks,
    /// The look for fonts installed or removed while the app was in the background, running
    /// ([`Self::refresh_installed_fonts`]): whether they were.
    font_check: Option<std::sync::mpsc::Receiver<bool>>,
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
    /// A numeric field is being scrubbed: the document's edits meanwhile are one undo step
    /// ([`scrub::begin_frame`]).
    scrub_group: bool,
    /// The folders control requests, and the input they inject, may read and write
    /// (`--automation-read-root`, `--automation-write-root`); `None`: anywhere. See
    /// [`Self::with_automation_roots`].
    automation_roots: Option<Arc<AutomationRoots>>,
    /// This frame carries input the control channel injected ([`Self::raw_input_hook`]): it runs
    /// confined to [`Self::automation_roots`].
    synthetic_frame: bool,
}

/// A window capture asked for through the control channel (`ui.screenshot`), answered once a
/// frame is presented; a window that isn't presented never delivers one, hence the deadline.
struct PendingScreenshot {
    token: u64,
    path: Option<String>,
    /// Send the PNG back as `pngBase64`.
    data: bool,
    reply: Sender<ControlResponse>,
    /// When to give up (ms, as [`now_ms`]).
    deadline: f64,
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
                selection_bounds: None,
                cursors: Default::default(),
            },
            perf: Perf::default(),
            titlebar_inset: 0.0,
            last_effect: None,
            run_count: 0,
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
            context: 0,
            ui_fonts: Default::default(),
            frame: 0,
            last_time: 0.0,
            canvas_rect: None,
            hover_doc: None,
            custom_titlebar: false,
            last_window_title: String::new(),
            graphics_adapter: None,
            place: Default::default(),
            system_paste: false,
            system_paste_at: f64::NEG_INFINITY,
            clipboard_probe: None,
            picks: picks::Picks::default(),
            font_check: None,
            paste_chord: Default::default(),
            background: Default::default(),
            recovery: Default::default(),
            host_modifiers: Default::default(),
            synthetic_modifiers: false,
            ime_marked: None,
            ime_discard: false,
            scrub_group: false,
            automation_roots: None,
            synthetic_frame: false,
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Confine what the control channel does to `roots` (#832): its requests, the input it
    /// injects (in the frames that carry it) and the saves and exports they start read and write
    /// only inside them ([`file_access`]). The host's file services check every path they're given
    /// against the roots in force, and so does the engine; outside those scopes the person at the
    /// keyboard works as usual.
    pub fn with_automation_roots(mut self, roots: Option<Arc<AutomationRoots>>) -> Self {
        let Some(roots) = roots else { return self };
        self.automation_roots = Some(roots);
        let s = &mut self.services;
        if let Some(read) = s.read.take() {
            s.read = Some(Box::new(move |p: &str| file_access::check_read(p).and_then(|()| read(p))));
        }
        if let Some(mut write) = s.write.take() {
            s.write = Some(Box::new(move |p: &str, b: &[u8]| file_access::check_write(p).and_then(|()| write(p, b))));
        }
        if let Some(write) = s.write_shared.take() {
            s.write_shared = Some(Arc::new(move |p: &str, b: &[u8]| file_access::check_write(p).and_then(|()| write(p, b))));
        }
        // Edit Original and Show in Folder hand a file to another app: only one automation may read.
        for service in [&mut s.open_file, &mut s.reveal] {
            if let Some(mut open) = service.take() {
                *service = Some(Box::new(move |p: &str| file_access::check_read(p).and_then(|()| open(p))));
            }
        }
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
            guides: self.ui.view.guides,
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
        // A file dialog it shows off the UI thread runs it again with the path picked.
        if self.picks.is_entry_free() {
            let entry = picks::Entry::Command(id.to_string(), params.clone());
            return picks::as_entry(self, move || entry, |app| app.run_now(id, params));
        }
        self.run_now(id, params)
    }

    /// [`Self::run`] it, inside what asks for file dialogs.
    fn run_now(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.run_count = self.run_count.wrapping_add(1);
        if let Some(r) = menus::run_ui_command(self, id, &params) {
            return r;
        }
        let mut params = params;
        // Paste in place, in front, in back (#693) and Select › All on Active Artboard (#1006): onto
        // or on the active artboard, the view's.
        if matches!(id, "edit.pasteInPlace" | "edit.pasteInFront" | "edit.pasteInBack" | "select.allOnArtboard")
            && params.get("artboard").is_none()
            && self.view().is_some()
            && let Some(n) = self.session.active().map(|d| d.doc.artboards.len())
            && let Some(p) = params.as_object_mut()
        {
            p.insert("artboard".into(), serde_json::json!(panels::artboards::selected(self, n)));
        }
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
                    r.truncate(MAX_RECENT_FONTS);
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
            let roots = self.automation_roots.clone();
            let outcome = file_access::confine(roots.as_ref(), || vectorcraft_engine::guard::catch_panic(|| control::handle(self, ctx, &req)))
                .unwrap_or_else(|msg| control::err(format!("internal error: {msg} (please report this bug)")));
            match outcome {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path, data } => {
                    self.screenshot_token += 1;
                    let token = self.screenshot_token;
                    let settle = ctx.global_style().animation_time as f64 * 2000.0 + 80.0;
                    self.queued_screenshots.push((token, now_ms() + settle, 0));
                    self.pending_screenshots.push(PendingScreenshot { token, path, data, reply, deadline: now_ms() + settle + 8000.0 });
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
            if let Some(i) = self.pending_screenshots.iter().position(|p| p.token == token) {
                let p = self.pending_screenshots.remove(i);
                let roots = self.automation_roots.clone();
                let _ = p.reply.send(file_access::confine(roots.as_ref(), || control::save_screenshot(self, &image, p.path.as_deref(), p.data)));
            }
        }
        let now = now_ms();
        self.pending_screenshots.retain(|p| {
            if now < p.deadline {
                return true;
            }
            let _ = p.reply.send(serde_json::json!({
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

    /// Linked files another app changed while their document is open: every two seconds, and as
    /// soon as the window comes back to the front, stamp the active document's linked files on a
    /// worker thread and act on the changed ones as Preferences › File Handling › Update Links
    /// says ([`vectorcraft_engine::link_watch`]). No look starts while a dialog is open, so an
    /// Ask When Modified question never goes unseen.
    #[cfg(not(target_arch = "wasm32"))]
    fn tick_link_updates(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(r) = self.session.poll_link_scan() {
            match r {
                Ok(r) => {
                    let n = |k: &str| r[k].as_array().map_or(0, Vec::len);
                    if n("updated") > 0 {
                        self.status(format!("Updated {} linked file(s) changed on disk", n("updated")));
                    }
                    if n("ask") > 0 {
                        dialogs::missing_links::ask_update_changed(self, r["ask"].as_array().cloned().unwrap_or_default());
                    }
                    if n("modified") > 0 {
                        self.status(format!("{} linked image(s) changed on disk: Update Links shows the new versions", n("modified")));
                    }
                }
                Err(e) => self.status(e.to_string()),
            }
        }
        let (last_key, focus_key) = (egui::Id::new("linkWatch.lastScan"), egui::Id::new("linkWatch.focused"));
        let focused = ctx.input(|i| i.focused);
        let was_focused: bool = ctx.data(|d| d.get_temp(focus_key)).unwrap_or(focused);
        ctx.data_mut(|d| d.insert_temp(focus_key, focused));
        if self.ui.dialog.is_some() || self.session.active().is_none() {
            return;
        }
        let last: f64 = ctx.data(|d| d.get_temp(last_key)).unwrap_or(f64::NEG_INFINITY);
        // The active document as the last look saw it: its uid and revision.
        let (seen_key, doc) = (egui::Id::new("linkWatch.seen"), self.session.active().map(|d| (d.uid, d.revision)));
        if now - last >= 2.0 || (focused && !was_focused) {
            // Walking the document's links costs a pass over it: only when a look is due.
            // `start_link_scan` starts none for a document without links, which then doesn't
            // wake the app for looks of its own (any frame still looks once one is due).
            ctx.data_mut(|d| d.insert_temp(last_key, now));
            self.session.start_link_scan();
            let links = self.session.link_scan.is_some();
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("linkWatch.links"), links);
                d.insert_temp(seen_key, doc);
            });
        }
        if self.session.link_scan.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if ctx.data(|d| d.get_temp::<bool>(egui::Id::new("linkWatch.links"))).unwrap_or(false) {
            ctx.request_repaint_after(std::time::Duration::from_millis(2000));
        } else if ctx.data(|d| d.get_temp::<Option<(u64, u64)>>(seen_key)).is_some_and(|seen| seen != doc) {
            // The document changed (a file placed into one without links) or another came to the
            // front since that look: wake for the next one even if the app sits idle until then,
            // or a file placed and changed meanwhile is first seen after the change, which is then
            // never acted on.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((last + 2.0 - now).clamp(0.0, 2.0)));
        }
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
                io::report_open_error(self, &name, &e);
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

/// The most fonts Type › Recent Fonts lists (Preferences › Type › Number of Recent Fonts).
pub const MAX_RECENT_FONTS: usize = 15;

impl VectorcraftApp {
    /// Type › Recent Fonts: the fonts used last, newest first, as many as Preferences › Type ›
    /// Number of Recent Fonts says.
    pub fn recent_fonts(&self) -> &[String] {
        let n = usize::try_from(self.session.prefs.recent_fonts_count).unwrap_or(MAX_RECENT_FONTS).clamp(1, MAX_RECENT_FONTS);
        self.ui.recent_fonts.get(..n).unwrap_or(&self.ui.recent_fonts)
    }

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

    /// The selection's visual bounds (stroke and effects included), square to the page: measured
    /// once per revision, as the canvas reads it every frame (a traced photo selects a group of
    /// hundreds of thousands of paths).
    pub fn selection_bounds(&mut self) -> Option<vectorcraft_geom::Rect> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision);
        if let Some((k, b)) = self.canvas.selection_bounds
            && k == key
        {
            return b;
        }
        let b = st.doc.bounds_of(&st.selection.objects, true);
        self.canvas.selection_bounds = Some((key, b));
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
        let confined = self.synthetic_frame.then(|| self.automation_roots.clone()).flatten();
        if let Err(msg) = file_access::confine(confined.as_ref(), || vectorcraft_engine::guard::catch_panic(|| self.logic_frame(ctx))) {
            self.status(format!("Internal error: {msg} (please report this bug)"));
        }
    }

    fn logic_frame(&mut self, ctx: &egui::Context) {
        let lang = self.ui_language();
        i18n::set_current(lang);
        // The engine gives new type the Japanese defaults while the UI is in Japanese.
        if self.session.ui_language.as_deref() != Some(lang.code()) {
            self.session.ui_language = Some(lang.code().to_string());
        }
        self.adopt_context(ctx);
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
        // Read the system clipboard only when that alone decides whether Paste is enabled. A
        // background thread reads it where the host installs one (an unresponsive owner must never
        // stall the frame loop); otherwise, or once that thread is gone, it is read here, at most a
        // few times a second (opening it locks it against other apps on some systems).
        let wanted = self.session.clipboard.is_empty() && self.session.active().is_some();
        if let Some(make) = self.services.clipboard_probe.take() {
            self.clipboard_probe = clipboard_probe::Probe::start(make, ctx.clone());
        }
        if let Some(pasteable) = self.clipboard_probe.as_mut().and_then(|p| p.pasteable(wanted)) {
            self.system_paste = pasteable;
        } else if !(0.0..SYSTEM_CLIPBOARD_POLL).contains(&(now - self.system_paste_at)) {
            self.clipboard_probe = None;
            self.system_paste_at = now;
            self.system_paste = wanted && self.system_clipboard_pasteable();
        }
        self.poll_font_check(ctx);
        picks::poll(self, ctx);
        background::poll(self);
        #[cfg(not(target_arch = "wasm32"))]
        self.tick_link_updates(ctx, now);
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
        // The OS title bar (and the taskbar / Alt-Tab entry) follows the active file; with the
        // system title bar this is where the document name lives, as the in-app mark is hidden.
        crate::chrome::sync_window_title(self, ctx);
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
        native_menu::run(self, ctx);
        if self.fonts_ready {
            shortcuts::handle(self, ctx);
        }
        // Native only: the web host reads dropped files asynchronously and feeds the inboxes.
        #[cfg(not(target_arch = "wasm32"))]
        self.take_dropped_files(ctx);
    }

    /// Files dropped on the window: documents, libraries, presets and plug-ins opened, pictures
    /// and text placed on the canvas ([`Self::drop_target`]).
    #[cfg(not(target_arch = "wasm32"))]
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let (dropped, shift) = ctx.input(|i| (i.raw.dropped_files.clone(), i.modifiers.shift));
        if dropped.is_empty() {
            return;
        }
        let pos = place::drag_pos(ctx);
        let mut files = vec![];
        for f in dropped {
            let path = Some(f.path().to_string_lossy().to_string()).filter(|s| !s.is_empty());
            let name = path.as_deref().map_or_else(|| "dropped".into(), vectorcraft_engine::cmd::fileio::file_name);
            // Native file drops usually carry a path. Use the same file reader as File → Open
            // (and File → Place), even if the drop handle can't supply the bytes itself.
            let bytes = if path.is_some() && self.services.read.is_some() { Ok(vec![]) } else { f.bytes() };
            match bytes {
                Ok(bytes) => {
                    // A backend may give bytes without a path or extension. Sniff their format
                    // before deciding: an SVG opens as a document, a raster image is placed.
                    let target = if path.is_none() && fileio::detect(&name, &bytes).is_none_or(|f| !f.raster) {
                        place::DropTarget::Open
                    } else {
                        self.drop_target(&name, pos, shift)
                    };
                    files.push((target, (name, path, bytes)));
                }
                Err(e) => self.status(format!("Couldn't read {name}: {e}")),
            }
        }
        place::drop_files(self, files);
    }

    /// Fonts installed or removed while the app was in the background are listed when it comes
    /// back (Refresh Font List by itself): a look at the font folders, a scan only when they changed.
    /// The look runs on another thread (asking DirectWrite for the fonts font services loaded
    /// meanwhile takes tens of milliseconds on Windows, #579), [`Self::poll_font_check`] scans.
    fn refresh_installed_fonts(&mut self) {
        if self.font_check.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let look = move || {
            // The UI gone meanwhile has nothing to refresh.
            let _ = tx.send(vectorcraft_text::FontDb::global().installed_fonts_changed());
        };
        let spawned =
            if cfg!(target_arch = "wasm32") { None } else { std::thread::Builder::new().name("font-check".into()).spawn(look.clone()).ok() };
        if spawned.is_none() {
            look();
        }
        self.font_check = Some(rx);
    }

    /// Rescan the fonts once [`Self::refresh_installed_fonts`]'s look says they changed.
    fn poll_font_check(&mut self, ctx: &egui::Context) {
        let Some(check) = &self.font_check else { return };
        match check.try_recv() {
            Ok(changed) => {
                self.font_check = None;
                if changed {
                    // A failure shows in the status bar, as the menu item's does.
                    let _ = self.run("text.rescanFonts", json!({}));
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.font_check = None,
        }
    }

    /// Inject synthetic events (one press/release step or wheel turn per frame). Handlers read the
    /// modifiers egui holds (`i.modifiers`), so a synthetic key, button or wheel turn holds its own
    /// for the frames it spans (a drag's moves included); the keyboard's come back after.
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        // The native menu's key equivalents become the input their keys make, ahead of what came
        // after them (see `native_menu`).
        let keys = self.services.native_menu.as_mut().map(native_menu::NativeMenu::take_keys).unwrap_or_default();
        if !keys.is_empty() {
            let events: Vec<egui::Event> = keys.into_iter().flat_map(|k| native_menu::key_events(k, || self.system_clipboard_text())).collect();
            raw.events.splice(0..0, events);
        }
        for e in &raw.events {
            match e {
                egui::Event::ModifiersChanged(m) => self.host_modifiers = *m,
                egui::Event::WindowFocused(false) => self.host_modifiers = egui::Modifiers::NONE,
                egui::Event::WindowFocused(true) => self.refresh_installed_fonts(),
                _ => {}
            }
        }
        self.synthetic_frame = !self.synthetic.is_empty();
        let Some(first) = self.synthetic.first() else {
            if std::mem::take(&mut self.synthetic_modifiers) {
                raw.events.push(egui::Event::ModifiersChanged(self.host_modifiers));
            }
            return;
        };
        // Pointer events go one per frame so egui sees presses, drags and releases as real input;
        // keyboard sequences go up to the key release.
        let n = match first {
            egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::MouseWheel { .. } => 1,
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
                egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } | egui::Event::MouseWheel { modifiers, .. } => {
                    Some(*modifiers)
                }
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
        let confined = self.synthetic_frame.then(|| self.automation_roots.clone()).flatten();
        if let Err(msg) = file_access::confine(confined.as_ref(), || vectorcraft_engine::guard::catch_panic(|| self.ui_frame(ui))) {
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
        scrub::begin_frame(self, &ctx);
        font_menu::end_stale_preview(self, &ctx);
        floating::track(self, &ctx);
        let t = theme::Tokens::get(&ctx);
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
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| {
            if self.ui.screen_mode < 3 {
                chrome::doc_tabs(self, ui);
            }
            canvas::show(self, ui);
        });
        dock::floating_panel(self, &ctx);
        floating::show(self, &ctx);
        panels::library_panel::show_window(self, &ctx);
        dialogs::show(self, &ctx);
        palette::show(self, &ctx);
        if self.custom_titlebar {
            titlebar::resize_zones(ui);
        }
        self.ui_fonts.frame(&ctx);
        scrub::end_frame(self, &ctx);
        native_menu::sync(self, &ctx);
        self.perf.frame_ms = now_ms() - t0;
        let _ = json!(null);
    }
}
