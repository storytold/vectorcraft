//! `window.vectorcraft`: the control channel for the page, as promises.
//!
//! `request`, `execute`, `commands`, `docInspect`, `uiInspect` and `screenshot` come from
//! `craft-web-api` (storytold/craft-libs#413), the same in every Craft app; VectorCraft adds its own
//! functions on the same object.
//!
//! The same JSON methods as the desktop control server (`docs/control-protocol.md`), so an agent
//! driving VectorCraft in a browser tab (or an `<iframe>` in another app) uses the same commands as
//! the CLI and the MCP server:
//!
//! ```js
//! await vectorcraft.execute("shape.rectangle", {x: 0, y: 0, width: 100, height: 50})  // engine command → its result
//! await vectorcraft.request("ui.inspect")                      // control method → its result
//! await vectorcraft.commands()                                 // command list
//! const {data} = await vectorcraft.screenshot()                // the canvas as PNG (base64)
//! await vectorcraft.openFile("logo.svg", bytes)               // opened as a dropped file is
//! vectorcraft.captureDownloads(true)                           // saves stay in the page…
//! vectorcraft.files(); await vectorcraft.readFile("logo.png")  // …to be read back
//! ```
//!
//! Results resolve with the method's `result`; errors reject with an `Error`.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use craft_web_api::methods::Methods;
use craft_web_api::page::{PageApi, Request};
use serde_json::{Value, json};
use vectorcraft_ui_egui::{ControlRequest, ControlTicket as Ticket};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::saved_files::SavedFiles;

use crate::web::Inbox;

thread_local! {
    static TX: RefCell<Option<Sender<ControlRequest>>> = const { RefCell::new(None) };
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static INBOX: RefCell<Option<Inbox>> = const { RefCell::new(None) };
    static CAPTURE: Cell<bool> = const { Cell::new(false) };
    static SAVED: RefCell<SavedFiles> = RefCell::new(SavedFiles::default());
}

/// Wire the page's requests to the app: control requests go to `tx`, opened files to `inbox`.
/// After a graphics restart the app keeps its channel: `tx` is `None` and only the context changes.
pub fn connect(tx: Option<Sender<ControlRequest>>, inbox: Inbox, ctx: egui::Context) {
    if let Some(tx) = tx {
        TX.with(|t| *t.borrow_mut() = Some(tx));
    }
    INBOX.with(|i| *i.borrow_mut() = Some(inbox));
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

fn repaint() {
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}

/// Keep a save instead of downloading it, when the page asked for that. True when kept.
pub fn capture(path: &str, bytes: &[u8]) -> bool {
    if !CAPTURE.with(Cell::get) {
        return false;
    }
    SAVED.with(|s| s.borrow_mut().keep(path, bytes));
    true
}

fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

/// A reply as a promise outcome: the `result`, or an `Error` carrying the `error`.
fn outcome(reply: &Value) -> Result<JsValue, JsValue> {
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(to_js(reply.get("result").unwrap_or(&Value::Null)))
    } else {
        Err(js_sys::Error::new(reply.get("error").and_then(Value::as_str).unwrap_or("error")).into())
    }
}

/// How often a waiting call looks for the app's reply, in milliseconds (about a frame).
const POLL_MS: i32 = 16;

/// A queued control request whose reply has not arrived. Dropping it before the reply (the page's
/// timeout fired) gives the request up if the app has not started it, so the app never runs it late.
struct Waiting {
    reply: Receiver<Value>,
    ticket: Ticket,
    answered: bool,
}

impl Waiting {
    /// The app's reply once it has answered or gone away; `None` while the request is waiting.
    fn poll(&mut self) -> Option<Value> {
        match self.reply.try_recv() {
            Ok(reply) => {
                self.answered = true;
                Some(reply)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.answered = true;
                Some(json!({"ok": false, "error": "request dropped"}))
            }
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if !self.answered {
            self.ticket.give_up();
        }
    }
}

/// Queue a control request for the app and wait for its reply, as the protocol's `{ok, result}` /
/// `{ok: false, error}` object.
fn send(method: &str, params: Value) -> Pin<Box<dyn Future<Output = Value>>> {
    let (req, rx) = ControlRequest::new(method, params);
    let ticket = req.ticket();
    let sent = TX.with(|t| t.borrow().as_ref().is_some_and(|tx| tx.send(req).is_ok()));
    if !sent {
        return Box::pin(async { json!({"ok": false, "error": "VectorCraft is not running yet"}) });
    }
    repaint();
    let mut waiting = Waiting { reply: rx, ticket, answered: false };
    Box::pin(async move {
        loop {
            if let Some(reply) = waiting.poll() {
                return reply;
            }
            sleep(POLL_MS).await;
        }
    })
}

pub(crate) async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

fn set_fn(obj: &js_sys::Object, name: &str, f: JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(obj, &name.into(), &f).map(|_| ())
}

/// Install `window.vectorcraft`: the crate's functions, then VectorCraft's own (`inspect` is its
/// `ui.inspect`).
pub fn install() -> Result<(), JsValue> {
    let request: Request = Rc::new(|method, params| send(&method, params));
    Rc::new(PageApi::new("vectorcraft", Methods::default(), request)).install()?;
    let window = web_sys::window().ok_or("no window")?;
    let obj: js_sys::Object = js_sys::Reflect::get(&window, &"vectorcraft".into())?.unchecked_into();
    set_fn(
        &obj,
        "inspect",
        Closure::<dyn Fn() -> js_sys::Promise>::new(|| future_to_promise(async { outcome(&send("ui.inspect", json!({})).await) })).into_js_value(),
    )?;
    set_fn(
        &obj,
        "openFile",
        Closure::<dyn Fn(String, js_sys::Uint8Array) -> JsValue>::new(|name: String, bytes: js_sys::Uint8Array| {
            let queued = INBOX.with(|i| {
                i.borrow().as_ref().is_some_and(|inbox| {
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes.to_vec()));
                    true
                })
            });
            repaint();
            JsValue::from_bool(queued)
        })
        .into_js_value(),
    )?;
    set_fn(&obj, "captureDownloads", Closure::<dyn Fn(bool)>::new(|on: bool| CAPTURE.with(|c| c.set(on))).into_js_value())?;
    set_fn(
        &obj,
        "files",
        Closure::<dyn Fn() -> JsValue>::new(|| {
            SAVED.with(|s| to_js(&Value::Array(s.borrow().list().map(|(n, size)| json!({"path": n, "size": size})).collect())))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "readFile",
        Closure::<dyn Fn(String) -> Result<JsValue, JsValue>>::new(|name: String| {
            SAVED.with(|s| {
                s.borrow()
                    .get(&name)
                    .map(|b| js_sys::Uint8Array::from(b).into())
                    .ok_or_else(|| js_sys::Error::new(&format!("{name}: not found")).into())
            })
        })
        .into_js_value(),
    )?;
    Ok(())
}
