//! The browser shell: web `Services`, drag-and-drop, and the eframe web runner.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::cmd::recovery::RecoveryStore;
use vectorcraft_ui_egui::place::{DropTarget, PlaceArrival, PlaceInbox};
use vectorcraft_ui_egui::print::{PrintJob, PrintService, Printer};
use vectorcraft_ui_egui::{Services, VectorcraftApp};
use wasm_bindgen::JsCast as _;

type Inbox = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Where the pointer was over the canvas during the last file drag (CSS px from its top-left) and
/// whether Shift was held: where dropped files land.
type DragPos = Rc<Cell<Option<(f32, f32, bool)>>>;

const CANVAS_ID: &str = "vectorcraft_canvas";
const LOADING_ID: &str = "vectorcraft_loading";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let drag = track_drag(&canvas);
        let mut options = eframe::WebOptions::default();
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("vectorcraft-web: wgpu backend {:?}", rs.adapter.get_info().backend);
                    }
                    let inbox: Inbox = Arc::default();
                    let place_inbox: PlaceInbox = Arc::default();
                    let app = VectorcraftApp::new(Session::new(), services(inbox.clone(), place_inbox.clone(), cc.egui_ctx.clone()));
                    Ok(Box::new(WebShell { app, inbox, place_inbox, drag }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>Vector W3K2 failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

/// Follow file drags over the canvas: drag events carry the pointer position, which egui doesn't
/// get during a drag.
fn track_drag(canvas: &web_sys::HtmlCanvasElement) -> DragPos {
    let pos = DragPos::default();
    let p = pos.clone();
    let on_drag = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::DragEvent)>::new(move |e: web_sys::DragEvent| {
        p.set(Some((e.offset_x() as f32, e.offset_y() as f32, e.shift_key())));
    });
    for kind in ["dragover", "drop"] {
        if let Err(e) = canvas.add_event_listener_with_callback(kind, on_drag.as_ref().unchecked_ref()) {
            log::error!("couldn't follow {kind} events: {e:?}");
        }
    }
    // The listener lives as long as the page.
    on_drag.forget();
    pos
}

/// Wraps the app to read dropped files asynchronously (browsers can't read them synchronously)
/// and feed them through the inboxes: placed where they were dropped on the canvas, else opened.
struct WebShell {
    app: VectorcraftApp,
    inbox: Inbox,
    place_inbox: PlaceInbox,
    drag: DragPos,
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        if !dropped.is_empty() {
            let at = self.drag.take();
            let z = ctx.zoom_factor();
            let target = self.app.drop_target(at.map(|(x, y, _)| egui::pos2(x / z, y / z)), at.is_some_and(|a| a.2));
            for f in dropped {
                let (inbox, place_inbox, ctx) = (self.inbox.clone(), self.place_inbox.clone(), ctx.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into());
                    match f.bytes_async().await {
                        Ok(bytes) => {
                            match target {
                                DropTarget::Place { at, embed } => {
                                    place_inbox.lock().unwrap_or_else(|e| e.into_inner()).push(PlaceArrival { name, bytes, drop: Some((at, embed)) })
                                }
                                DropTarget::Open => inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes)),
                            }
                            ctx.request_repaint();
                        }
                        Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                    }
                });
            }
        }
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

fn services(inbox: Inbox, place_inbox: PlaceInbox, ctx: egui::Context) -> Services {
    let open_inbox = inbox.clone();
    let picked = place_inbox.clone();
    let place_ctx = ctx.clone();
    Services {
        // File → Place…: the picked files go to the Place dialog.
        place_async: Some(Box::new(move || {
            let inbox = picked.clone();
            let ctx = place_ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let dialog = fileio::place_filters().fold(rfd::AsyncFileDialog::new().set_title("Place"), |d, (name, exts)| d.add_filter(name, exts));
                let Some(files) = dialog.pick_files().await else {
                    return;
                };
                let mut arrived = vec![];
                for f in files {
                    arrived.push(PlaceArrival { name: f.file_name(), bytes: f.read().await, drop: None });
                }
                inbox.lock().unwrap_or_else(|e| e.into_inner()).extend(arrived);
                ctx.request_repaint();
            });
        })),
        place_inbox: Some(place_inbox),
        open_async: Some(Box::new(move || {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let dialog = fileio::open_filters().fold(rfd::AsyncFileDialog::new(), |d, (name, exts)| d.add_filter(name, exts));
                let Some(file) = dialog.pick_file().await else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        download: Some(Box::new(|name: &str, bytes: &[u8]| {
            if let Err(e) = download(name, bytes) {
                log::error!("download of {name} failed: {e}");
            }
        })),
        inbox: Some(inbox),
        recovery_store: Some(Arc::new(BrowserStore)),
        // File → Print: the browser's print dialog.
        print: Some(Box::new(BrowserPrint)),
        ..Default::default()
    }
}

/// Data Recovery's store on the web: the browser's local storage (kept across visits and shared by
/// the site's tabs), each entry as base64 under [`RECOVERY_PREFIX`]`<area>/<name>`. It has no
/// locks: each tab holds its area with a heartbeat, judged by the browser's clock.
struct BrowserStore;

const RECOVERY_PREFIX: &str = "vectorcraft-recovery/";

fn js_err(e: wasm_bindgen::JsValue) -> String {
    format!("{e:?}")
}

fn local_storage() -> Result<web_sys::Storage, String> {
    web_sys::window().ok_or("no window")?.local_storage().map_err(js_err)?.ok_or_else(|| "browser storage is turned off".into())
}

impl RecoveryStore for BrowserStore {
    fn list(&self) -> Result<Vec<String>, String> {
        let s = local_storage()?;
        let n = s.length().map_err(js_err)?;
        Ok((0..n).filter_map(|i| s.key(i).ok().flatten()).filter_map(|k| k.strip_prefix(RECOVERY_PREFIX).map(str::to_string)).collect())
    }
    fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let text = local_storage()?.get_item(&format!("{RECOVERY_PREFIX}{name}")).map_err(js_err)?;
        text.and_then(|t| vectorcraft_format::base64_decode(&t)).ok_or_else(|| format!("{name}: no such recovery entry"))
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        local_storage()?
            .set_item(&format!("{RECOVERY_PREFIX}{name}"), &vectorcraft_format::base64_encode(bytes))
            .map_err(|e| format!("browser storage is full or turned off: {}", js_err(e)))
    }
    fn remove(&self, name: &str) -> Result<(), String> {
        local_storage()?.remove_item(&format!("{RECOVERY_PREFIX}{name}")).map_err(js_err)
    }
    fn location(&self) -> String {
        "browser storage".into()
    }
    fn now(&self) -> Option<i64> {
        Some((js_sys::Date::now() / 1000.0) as i64)
    }
}

/// Trigger a browser download of `bytes` named after the last component of `path`.
fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "vectorcraft".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let url = blob_url(bytes, fileio::format_for_name(&name).map_or("application/octet-stream", |f| f.mime))?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

/// An object URL of a blob of `bytes` with media type `mime` (revoke it when done).
fn blob_url(bytes: &[u8], mime: &str) -> Result<String, String> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js_err)?;
    web_sys::Url::create_object_url_with_blob(&blob).map_err(js_err)
}

/// File → Print in the browser: the job's PDF in a hidden frame, printed through the browser's
/// print dialog (which picks the printer).
struct BrowserPrint;

/// The frame holding the last print job (the next job replaces it).
const PRINT_FRAME_ID: &str = "vectorcraft-print-frame";

impl PrintService for BrowserPrint {
    fn printers(&mut self) -> Vec<Printer> {
        // Browsers don't tell pages about printers: their print dialog lists them.
        vec![]
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        print_pdf(job.pdf)?;
        Ok(format!("Printing “{}”: pick the printer in the browser's print dialog", job.title))
    }
}

fn print_pdf(pdf: &[u8]) -> Result<(), String> {
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    // The previous job's frame and its blob go first.
    if let Some(old) = document.get_element_by_id(PRINT_FRAME_ID) {
        if let Some(url) = old.get_attribute("src") {
            web_sys::Url::revoke_object_url(&url).ok();
        }
        old.remove();
    }
    let url = blob_url(pdf, "application/pdf")?;
    let frame: web_sys::HtmlIFrameElement = document.create_element("iframe").map_err(js_err)?.dyn_into().map_err(|_| "not a frame")?;
    frame.set_id(PRINT_FRAME_ID);
    // Out of sight but laid out: browsers don't print a frame that isn't.
    let style = frame.style();
    for (k, v) in [("position", "fixed"), ("right", "0"), ("bottom", "0"), ("width", "0"), ("height", "0"), ("border", "0")] {
        style.set_property(k, v).map_err(js_err)?;
    }
    let loaded = frame.clone();
    let onload = wasm_bindgen::closure::Closure::once_into_js(move || {
        if let Some(w) = loaded.content_window() {
            w.focus().ok();
            if let Err(e) = w.print() {
                log::error!("printing failed: {e:?}");
            }
        }
    });
    frame.set_onload(Some(onload.unchecked_ref()));
    frame.set_src(&url);
    document.body().ok_or("no body")?.append_child(&frame).map_err(js_err)?;
    Ok(())
}
