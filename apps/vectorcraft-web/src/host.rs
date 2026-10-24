//! The `postMessage` bridge to a same-origin parent page (`?host=parent`), for pages that embed
//! VectorCraft in an `<iframe>`. Messages are plain objects with a `type`:
//!
//! - `vectorcraft:ready` (to the parent): `{ version }`, the app is listening; send a document now.
//! - `vectorcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It opens like a file picked with File › Open or dropped on the window.
//! - `vectorcraft:save` (to the parent): `{ name, bytes }` (a transferred `ArrayBuffer`) for every
//!   file VectorCraft would otherwise download (Save, Save As, Export…). Once it is posted the app
//!   counts a native save as saved.
//! - `vectorcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `vectorcraft:failed` (to the parent): `{ error }` when the app couldn't start, or when a
//!   `vectorcraft:open` document is over [`crate::host_open::MAX_OPEN_BYTES`] (it isn't opened).
//!
//! Only the parent window on the page's own origin is heard or answered.

use std::cell::{Cell, RefCell};

use vectorcraft_ui_egui::VectorcraftApp;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

/// Longest file name taken from the parent page.
const MAX_NAME_CHARS: usize = 255;

thread_local! {
    /// The bridge [`deliver`] posts through, once installed.
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct Bridge {
    window: web_sys::Window,
    parent: web_sys::Window,
    origin: String,
    /// The unsaved-changes state last reported to the parent.
    reported: std::rc::Rc<Cell<Option<bool>>>,
}

impl Bridge {
    /// `None` when the page isn't framed (there is no parent to talk to).
    pub fn new() -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let origin = window.location().origin().ok()?;
        Some(Self { window, parent, origin, reported: Default::default() })
    }

    /// Make this the bridge [`deliver`] and [`report_dirty`] use.
    pub fn install(&self) {
        BRIDGE.with(|b| *b.borrow_mut() = Some(self.clone()));
    }

    pub fn post_ready(&self) {
        self.send(&[("type", "vectorcraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into())]);
    }

    pub fn post_failed(&self, error: &str) {
        self.send(&[("type", "vectorcraft:failed".into()), ("error", error.into())]);
    }

    /// Hear documents from the parent; they arrive through the open inbox like a picked file.
    pub fn listen(&self, inbox: crate::web::Inbox, ctx: egui::Context) {
        let bridge = self.clone();
        let parent = self.parent.clone();
        let origin = self.origin.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            let data = event.data();
            if string(&data, "type").as_deref() != Some("vectorcraft:open") {
                return;
            }
            let name = string(&data, "name")
                .filter(|n| !n.trim().is_empty())
                .map(|n| n.chars().take(MAX_NAME_CHARS).collect())
                .unwrap_or_else(|| "Untitled.vectorcraft".to_string());
            let Some(array) = byte_array(&data) else { return };
            // Checked before the bytes are copied out of the parent's buffer.
            if let Err(e) = crate::host_open::check_open_size(&name, u64::from(array.byte_length())) {
                log::warn!("vectorcraft-web: {e}");
                bridge.post_failed(&e);
                return;
            }
            inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, array.to_vec()));
            ctx.request_repaint();
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    fn post_save(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let message = message(&[("type", "vectorcraft:save".into()), ("name", name.into()), ("bytes", buffer.clone().into())])
            .ok_or_else(|| "couldn't build the message".to_string())?;
        self.parent
            .post_message_with_transfer(&message, &self.origin, &js_sys::Array::of1(&buffer))
            .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
    }

    /// Post a message to the parent; false if it couldn't be sent.
    fn send(&self, fields: &[(&str, JsValue)]) -> bool {
        message(fields).is_some_and(|m| self.parent.post_message(&m, &self.origin).is_ok())
    }
}

/// The installed bridge, in host mode.
pub fn installed() -> Option<Bridge> {
    BRIDGE.with(|b| b.borrow().clone())
}

/// In host mode, hand a file that would be downloaded to the parent page: `None` outside host
/// mode, else whether it was posted.
pub fn deliver(name: &str, bytes: &[u8]) -> Option<Result<(), String>> {
    let bridge = BRIDGE.with(|b| b.borrow().clone())?;
    Some(bridge.post_save(name, bytes))
}

/// After each frame: tell the parent when unsaved changes appear or go away.
pub fn report_dirty(app: &VectorcraftApp) {
    let Some(bridge) = BRIDGE.with(|b| b.borrow().clone()) else { return };
    let dirty = app.session.documents().iter().any(|d| d.is_dirty());
    if bridge.reported.get() != Some(dirty) && bridge.send(&[("type", "vectorcraft:dirty".into()), ("dirty", dirty.into())]) {
        bridge.reported.set(Some(dirty));
    }
}

fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
    let m = js_sys::Object::new();
    for (key, value) in fields {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
    }
    Some(m)
}

fn string(data: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
}

/// `bytes` (an `ArrayBuffer` or `Uint8Array`) as a view, not yet copied.
fn byte_array(data: &JsValue) -> Option<js_sys::Uint8Array> {
    let bytes = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).ok()?;
    if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        Some(array.clone())
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        Some(js_sys::Uint8Array::new(&bytes))
    } else {
        None
    }
}
