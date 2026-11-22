//! The web build's plug-in worker ([`vectorcraft_plugins::worker`]). Trunk builds it beside the
//! app (`data-type="worker"` in `apps/vectorcraft-web/index.html`), and the page starts one for each
//! module it checks and each filter it runs, through the loader Trunk writes for it
//! (`vectorcraft-plugin-worker_loader.js`). Each message, both ways, is one `Uint8Array`.

#[cfg(target_arch = "wasm32")]
fn main() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::prelude::Closure;
    use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let poster = scope.clone();
    let post = Rc::new(move |msg: Vec<u8>| {
        let bytes = js_sys::Uint8Array::from(msg.as_slice());
        // A message that can't be posted leaves the page waiting, and its time budget ends the run.
        let _ = poster.post_message_with_transfer(&bytes, &js_sys::Array::of1(&bytes.buffer()));
    });
    let worker = Rc::new(RefCell::new(vectorcraft_plugins::worker::Worker::new(post)));
    let handler = Rc::clone(&worker);
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        let msg = js_sys::Uint8Array::new(&e.data()).to_vec();
        handler.borrow_mut().handle(&msg);
    });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The handler lives as long as the worker.
    on_message.forget();
    worker.borrow().ready();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("vectorcraft-plugin-worker only runs in the browser, as the web build's plug-in worker");
}
