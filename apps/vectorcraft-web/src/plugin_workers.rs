//! The plug-in workers ([`vectorcraft_ui_egui::plugin_runs::Workers`]): each one a Web Worker that
//! runs `vectorcraft-plugin-worker`, which Trunk builds beside the app (`index.html`), started from
//! the loader file Trunk writes for it next to `index.html`.

use std::rc::Rc;

use vectorcraft_ui_egui::plugin_runs::{SharedInbox, WorkerLink, Workers};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::Closure;

/// The loader Trunk writes for the worker (`data-loader-shim`), beside `index.html`.
const LOADER: &str = "vectorcraft-plugin-worker_loader.js";

/// Starts the workers; each message or failure asks for a frame, which reads it.
pub(crate) struct PageWorkers {
    pub(crate) ctx: egui::Context,
}

/// One worker and its message and error handlers.
struct PageWorker {
    worker: web_sys::Worker,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
}

/// The text of a browser error value.
fn js_err(e: wasm_bindgen::JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

impl Workers for PageWorkers {
    fn start(&self, inbox: SharedInbox) -> Result<Box<dyn WorkerLink>, String> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or_else(|| "no document".to_string())?;
        let base = document.base_uri().map_err(js_err)?.ok_or_else(|| "no document".to_string())?;
        let url = web_sys::Url::new_with_base(LOADER, &base).map_err(js_err)?.href();
        let worker = web_sys::Worker::new(&url).map_err(js_err)?;
        let (to, ctx) = (Rc::clone(&inbox), self.ctx.clone());
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            to.borrow_mut().push(&js_sys::Uint8Array::new(&e.data()).to_vec());
            ctx.request_repaint();
        });
        let ctx = self.ctx.clone();
        // A worker that fails to load, panics or posts a message that can't be read.
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let why = e.dyn_ref::<web_sys::ErrorEvent>().map_or_else(|| e.type_(), web_sys::ErrorEvent::message);
            inbox.borrow_mut().fail(why);
            ctx.request_repaint();
        });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        worker.set_onmessageerror(Some(on_error.as_ref().unchecked_ref()));
        Ok(Box::new(PageWorker { worker, _on_message: on_message, _on_error: on_error }))
    }

    fn now_ms(&self) -> f64 {
        web_sys::window().and_then(|w| w.performance()).map_or(0.0, |p| p.now())
    }
}

impl WorkerLink for PageWorker {
    fn post(&mut self, msg: Vec<u8>) -> Result<(), String> {
        let bytes = js_sys::Uint8Array::from(msg.as_slice());
        self.worker.post_message_with_transfer(&bytes, &js_sys::Array::of1(&bytes.buffer())).map_err(js_err)
    }

    fn terminate(&mut self) {
        self.worker.set_onmessage(None);
        self.worker.set_onerror(None);
        self.worker.set_onmessageerror(None);
        self.worker.terminate();
    }
}
