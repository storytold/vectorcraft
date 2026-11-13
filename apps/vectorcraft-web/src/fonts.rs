//! Craft-fonts faces served beside the wasm (`fonts/<sha16>/…`, copied by
//! `cargo xtask web-fonts`) instead of embedded: [`load_startup_fonts`] fetches the `startup`
//! ones before the app starts, [`spawn_background_fonts`] the `background` ones once it runs, and
//! [`want_chars`] a `demand` face (Korean, Devanagari) when the interface language or the
//! interface's own text needs its script. The browser checks each file against its SHA-256
//! (Subresource Integrity). A face that fails is logged and skipped: the app runs without it. With
//! `WEB_FONTS` empty (built without craft-fonts) these do nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use vectorcraft_text::craft_fonts::demand_fonts;
use vectorcraft_text::{FontDb, WEB_FONTS, WebFont};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

use crate::demand::Demand;

/// Per startup face: the page shows its loading text meanwhile.
const STARTUP_TIMEOUT_MS: i32 = 20_000;
/// Per background face.
const BACKGROUND_TIMEOUT_MS: i32 = 120_000;
/// Background fetches at once, leaving the connection to the app's own requests.
const BACKGROUND_PARALLEL: usize = 4;

thread_local! {
    /// The background fetch runs once per page, though a graphics restart creates the app again.
    static BACKGROUND_STARTED: Cell<bool> = const { Cell::new(false) };
    /// The `demand` faces' fetches.
    static DEMAND: RefCell<Demand> = RefCell::new(Demand::default());
    /// The running app's context, asked for a frame when a `demand` face arrives (none before the
    /// app starts: its first frame looks through the faces loaded by then).
    static REPAINT: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// Fetch, in the background, the `demand` faces for the scripts of `chars` that are not loaded or
/// being fetched. Each one that arrives asks the app for a frame, and the interface draws those
/// characters with it from then on. Set as the interface's missing-glyph handler
/// (`vectorcraft_ui_egui::on_missing_glyphs`), and called with the interface language's name
/// before the app starts.
pub fn want_chars(chars: &[char]) {
    let wanted = demand_fonts(WEB_FONTS, chars.iter().copied());
    if !wanted.is_empty() {
        fetch_demand(&wanted);
    }
}

/// Fetch the `demand` faces `wanted` (indexes into `WEB_FONTS`) that [`Demand::start`] lets start.
/// A face whose fetch fails is fetched again after the wait [`Demand::finished`] gives.
fn fetch_demand(wanted: &[usize]) {
    for i in DEMAND.with(|d| d.borrow_mut().start(wanted, js_sys::Date::now())) {
        let Some(f) = WEB_FONTS.get(i) else { continue };
        wasm_bindgen_futures::spawn_local(async move {
            let ok = load(f, BACKGROUND_TIMEOUT_MS).await;
            let retry = DEMAND.with(|d| d.borrow_mut().finished(i, ok, js_sys::Date::now()));
            if ok && let Some(ctx) = REPAINT.with(|r| r.borrow().clone()) {
                ctx.request_repaint();
            }
            if let Some(wait) = retry {
                // A second longer, so a clock the browser coarsens still reads the wait as over.
                crate::api::sleep(wait.saturating_add(1_000)).await;
                fetch_demand(&[i]);
            }
        });
    }
}

/// Fetch and register the `startup` faces (one after the other). Returns how many loaded.
pub async fn load_startup_fonts() -> usize {
    let mut loaded = 0;
    for f in WEB_FONTS.iter().filter(|f| f.startup && !f.demand) {
        if load(f, STARTUP_TIMEOUT_MS).await {
            loaded += 1;
        }
    }
    loaded
}

/// Fetch and register the `background` faces, [`BACKGROUND_PARALLEL`] at a time, asking `ctx` for
/// a frame after each so the font menus and the canvas pick it up. Only the first call fetches
/// anything; each call makes `ctx` the context a `demand` face asks for a frame.
pub fn spawn_background_fonts(ctx: egui::Context) {
    REPAINT.with(|r| *r.borrow_mut() = Some(ctx.clone()));
    if BACKGROUND_STARTED.with(|s| s.replace(true)) {
        return;
    }
    let queue: Rc<Vec<&'static WebFont>> = Rc::new(WEB_FONTS.iter().filter(|f| !f.startup && !f.demand).collect());
    let next = Rc::new(Cell::new(0usize));
    for _ in 0..BACKGROUND_PARALLEL.min(queue.len()) {
        let (queue, next, ctx) = (queue.clone(), next.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let i = next.get();
                next.set(i + 1);
                let Some(f) = queue.get(i) else { break };
                if load(f, BACKGROUND_TIMEOUT_MS).await {
                    ctx.request_repaint();
                }
            }
        });
    }
}

/// Fetch one face and add it to the font database. False (and a console warning) on failure.
async fn load(f: &WebFont, timeout_ms: i32) -> bool {
    let bytes = match fetch(f.url, f.integrity, timeout_ms).await {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!("web font {} {}: {e}", f.family, f.style);
            return false;
        }
    };
    if FontDb::global().add_font(bytes) == 0 {
        log::warn!("web font {} {}: not a font, or already loaded", f.family, f.style);
        return false;
    }
    true
}

/// GET `url` (relative to the page) with Subresource Integrity `integrity`, aborted after
/// `timeout_ms`.
async fn fetch(url: &str, integrity: &str, timeout_ms: i32) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("no window")?;
    let abort = web_sys::AbortController::new().map_err(js)?;
    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    init.set_integrity(integrity);
    init.set_signal(Some(&abort.signal()));
    let on_timeout = {
        let abort = abort.clone();
        Closure::<dyn FnMut()>::new(move || abort.abort())
    };
    let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(on_timeout.as_ref().unchecked_ref(), timeout_ms).map_err(js)?;
    let result = async {
        let response: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(url, &init)).await.map_err(js)?.dyn_into().map_err(js)?;
        if !response.ok() {
            return Err(format!("HTTP {} for {url}", response.status()));
        }
        let buffer = JsFuture::from(response.array_buffer().map_err(js)?).await.map_err(js)?;
        Ok(js_sys::Uint8Array::new(&buffer).to_vec())
    }
    .await;
    window.clear_timeout_with_handle(timer);
    // Kept alive until the timer can no longer fire.
    drop(on_timeout);
    result
}

fn js(e: wasm_bindgen::JsValue) -> String {
    format!("{e:?}")
}
