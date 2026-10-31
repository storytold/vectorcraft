//! The `postMessage` bridge to a same-origin parent page (`?host=parent`), for pages that embed
//! VectorCraft in an `<iframe>`. Messages are plain objects with a `type`:
//!
//! - `vectorcraft:ready` (to the parent): `{ version }`, the app is listening; send a document now.
//! - `vectorcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It opens like a file picked with File › Open or dropped on the window.
//! - `vectorcraft:save` (to the parent): `{ id, name, bytes }` (a transferred `ArrayBuffer`) for
//!   every file VectorCraft would otherwise download (Save, Save As, Export…).
//! - `vectorcraft:saved` (from the parent): `{ id }`, that save is stored. Until it arrives a
//!   document save keeps the app reported dirty; after 30 s without it the app posts
//!   `vectorcraft:failed { id, error }` and stays dirty.
//! - `vectorcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `vectorcraft:failed` (to the parent): `{ error }` when the app couldn't start, or when a
//!   `vectorcraft:open` document is over [`crate::host_open::MAX_OPEN_BYTES`] (it isn't opened), when the page
//!   has no canvas, or when a save went unconfirmed.
//!
//! The bridge is installed only for a same-origin parent, or for a parent on the origin named by
//! `?host_origin=` (a bare `scheme://host[:port]` origin); messages go to that exact origin and only
//! the parent window on it is heard. A cross-origin parent without `host_origin` gets no bridge, so
//! saves download as usual.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use vectorcraft_ui_egui::{SaveKind, VectorcraftApp};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use crate::bridge::SaveAcks;

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
    reported: Rc<Cell<Option<bool>>>,
    /// Saves waiting for `vectorcraft:saved`.
    acks: Rc<RefCell<SaveAcks>>,
}

impl Bridge {
    /// `None` when the page isn't framed, or the parent is cross-origin and not the origin
    /// `host_origin` allows (see [`crate::bridge::target_origin`]).
    pub fn new(host_origin: Option<&str>) -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let own = window.location().origin().ok()?;
        // Reading a cross-origin parent's location throws: treat that as cross-origin.
        let parent_origin = parent.location().origin().ok();
        let origin = crate::bridge::target_origin(&own, parent_origin.as_deref(), host_origin)?;
        Some(Self { window, parent, origin, reported: Rc::default(), acks: Rc::default() })
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
            match string(&data, "type").as_deref() {
                Some("vectorcraft:open") => {}
                Some("vectorcraft:saved") => {
                    let id = js_sys::Reflect::get(&data, &JsValue::from_str("id")).ok().and_then(|v| v.as_f64());
                    if let Some(id) = id.filter(|id| id.is_finite() && *id >= 0.0 && id.fract() == 0.0) {
                        bridge.acks.borrow_mut().saved(id as u64);
                        ctx.request_repaint();
                    }
                    return;
                }
                _ => return,
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

    fn post_save(&self, name: &str, bytes: &[u8], kind: SaveKind) -> Result<(), String> {
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let id = self.acks.borrow_mut().posted(name, kind == SaveKind::Document, js_sys::Date::now());
        let sent = message(&[
            ("type", "vectorcraft:save".into()),
            ("id", JsValue::from_f64(id as f64)),
            ("name", name.into()),
            ("bytes", buffer.clone().into()),
        ])
        .ok_or_else(|| "couldn't build the message".to_string())
        .and_then(|m| {
            self.parent
                .post_message_with_transfer(&m, &self.origin, &js_sys::Array::of1(&buffer))
                .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
        });
        if sent.is_err() {
            self.acks.borrow_mut().failed(id);
        }
        sent
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
pub fn deliver(name: &str, bytes: &[u8], kind: SaveKind) -> Option<Result<(), String>> {
    let bridge = BRIDGE.with(|b| b.borrow().clone())?;
    Some(bridge.post_save(name, bytes, kind))
}

/// After each frame: tell the parent when unsaved changes appear or go away (held dirty while a
/// document save is unconfirmed), and fail saves the parent didn't confirm within 30 s.
pub fn report_dirty(app: &VectorcraftApp, ctx: &egui::Context) {
    let Some(bridge) = BRIDGE.with(|b| b.borrow().clone()) else { return };
    let expired = bridge.acks.borrow_mut().expire(js_sys::Date::now());
    for (id, name) in expired {
        bridge.send(&[
            ("type", "vectorcraft:failed".into()),
            ("id", JsValue::from_f64(id as f64)),
            ("error", format!("{name}: the host page didn't confirm the save").into()),
        ]);
    }
    let engine_dirty = app.session.documents().iter().any(|d| d.is_dirty());
    let (dirty, waiting) = {
        let acks = bridge.acks.borrow();
        (acks.dirty(engine_dirty), acks.waiting())
    };
    if waiting {
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
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
