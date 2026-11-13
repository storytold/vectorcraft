//! Programmatic control of the running app (agents, tests, MCP).
//!
//! Methods (JSON-lines over the host's transport):
//! - `engine.execute {command, params}` / `ui.menu.invoke {command|id, params}`: run any command
//! - `engine.commands`: engine + UI commands with enablement
//! - `document.inspect`: document summary; `ui.inspect`: UI state
//! - `ui.docking`: the panel layout in use, the open flyout, the focused panel and the docking
//!   controls' rects
//! - `ui.menu.list`: flattened menu tree
//! - `ui.contextMenu.list`: the canvas context menu for the current selection, flattened
//! - `ui.tool.select {tool}`, `ui.tool.list`
//! - `ui.pointer {events:[{kind: down|drag|up|move|doubleclick, x, y, backquote?, space?: "doc"|"screen"}], mods?, backquote?}`:
//!   drive the active tool through the same path as the mouse, after the synthetic input sent
//!   before it was applied
//! - `ui.key {key, shift?, alt?, cmd?}` / `ui.text {text}`: synthetic keyboard input
//! - `ui.move {x, y}` / `ui.click {x, y, button?, count?, shift?…}` / `ui.drag {x, y, toX, toY, steps?}`:
//!   real egui pointer input in screen points (reaches every widget: panels, flyouts, dialogs)
//! - `ui.set {brightness?, panel?, dockTab?, rulers?, outline?, …}`
//! - `ui.dialog.set {field, value}` or `{fields: [{field, value}…]}` (all or none; setting a field
//!   the dialog does not read is an error that lists its fields: [`crate::dialogs::accepts_field`];
//!   a `__` field other than the controls of the dialog's kind is an error too, so a request cannot
//!   change the command, targets or file the dialog's OK acts on: [`crate::dialogs::protected_field`];
//!   `action: confirm | cancel` then confirms or cancels that dialog in the same request and replies
//!   `{dialog, result}`) / `ui.dialog.confirm` / `ui.dialog.cancel`
//! - `ui.resize {width, height}`, `ui.focus`, `ui.screenshot {path?, data?}` (`data: true`: the PNG
//!   comes back as `pngBase64` as well)
//! - `ui.render {path?, scale?, artboard?, data?, traceViews?}`: render an artboard headlessly
//!   (`artboard`: its 0-based index, default the first; PNG with the pixels of a PNG export on
//!   white; `traceViews: true` draws Image Trace objects in their View, as the canvas does; without
//!   a path, or with `data: true`, as `pngBase64`)
//! - `app.open {path}` (any readable format) / `app.save {path?, svg?: {…SVG options}}` / `app.quit`
//!   (`file.close`, `file.closeAll` and `app.quit` first open a `saveChanges` dialog for each
//!   modified document: `ui.dialog.confirm` saves, set `discard: true` then confirm to discard)
//! - `app.export {path?, format?, …document.export options}`: encoded by the engine, written through
//!   the host; the document keeps its path. No path → `{dataBase64, format, bytes}` (as headless)
//! - `wait: true` on `app.save` and `app.export`: the file is written before the reply, even with
//!   Background Save or Background Export on. A `wait` other than true, false or null is refused

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use serde_json::{Value, json};
use vectorcraft_render::encode::{RasterExportOptions, RasterFormat};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::VectorcraftApp;
use crate::canvas::Xf;

/// Synthetic pointer input advances one event per frame: keep gestures and backlog modest.
pub(crate) const MAX_DRAG_STEPS: u64 = 256;
pub(crate) const MAX_CLICK_COUNT: u64 = 16;
pub(crate) const MAX_SYNTHETIC_EVENTS: usize = 1024;
pub(crate) const MAX_CONTROL_REQUESTS_PER_FRAME: usize = 64;
/// Most items one `ui.dialog.set {fields}` may store, as MCP `answer_dialog` allows.
pub(crate) const MAX_DIALOG_FIELDS: usize = 256;

fn input_count(p: &Value, key: &str, default: u64, max: u64) -> Result<usize, String> {
    let n = match p.get(key) {
        None => default,
        Some(v) => v.as_u64().ok_or_else(|| format!("`{key}` must be a non-negative integer"))?,
    };
    if n > max {
        return Err(format!("`{key}` must be at most {max}"));
    }
    // Preserve legacy zero behaviour: both a zero count and zero steps still perform once.
    usize::try_from(n.max(1)).map_err(|_| format!("`{key}` is out of range"))
}

/// The fields one `ui.dialog.set` stores, in order: `{field, value?}`, or `{fields: [{field, value?}…]}`.
/// Every item is checked before any is stored, so a refused item leaves the dialog as it was.
fn dialog_fields(p: &Value) -> Result<Vec<(String, Value)>, String> {
    let Some(list) = p.get("fields") else {
        let f = p.get("field").and_then(Value::as_str).ok_or("missing `field`")?;
        return Ok(vec![(f.to_string(), p.get("value").cloned().unwrap_or(Value::Null))]);
    };
    if p.get("field").is_some() {
        return Err("give `field` or `fields`, not both".into());
    }
    let list = list.as_array().ok_or("`fields` must be a list of {field, value?}")?;
    if list.len() > MAX_DIALOG_FIELDS {
        return Err(format!("`fields` holds {} items; at most {MAX_DIALOG_FIELDS}", list.len()));
    }
    list.iter()
        .enumerate()
        .map(|(i, item)| {
            let f = item.get("field").and_then(Value::as_str).ok_or_else(|| format!("fields {i}: missing string `field`"))?;
            Ok((f.to_string(), item.get("value").cloned().unwrap_or(Value::Null)))
        })
        .collect()
}

/// What `ui.dialog.set {action}` does with the dialog once its fields are set.
enum DialogAnswer {
    Confirm,
    Cancel,
}

/// The `action` of a `ui.dialog.set`: none, `confirm` or `cancel`. It is checked before any field is set.
fn dialog_action(p: &Value) -> Result<Option<DialogAnswer>, String> {
    match p.get("action") {
        None => Ok(None),
        Some(v) if v == "confirm" => Ok(Some(DialogAnswer::Confirm)),
        Some(v) if v == "cancel" => Ok(Some(DialogAnswer::Cancel)),
        Some(_) => Err("`action` must be confirm or cancel".into()),
    }
}

fn reserve_synthetic(app: &mut VectorcraftApp, additional: usize) -> Result<(), String> {
    if additional > MAX_SYNTHETIC_EVENTS.saturating_sub(app.synthetic.len()) {
        return Err(format!("synthetic input queue limit ({MAX_SYNTHETIC_EVENTS} events); retry after pending input drains"));
    }
    app.synthetic.try_reserve(additional).map_err(|e| format!("cannot reserve synthetic input: {e}"))
}

pub type ControlResponse = Value;

pub(crate) struct PendingPointerReply {
    pub from: u64,
    pub to: u64,
    pub document: Option<u64>,
    pub reply: Sender<ControlResponse>,
}

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<ControlResponse>,
    state: Arc<AtomicU8>,
    /// Its place in the order requests were made ([`ControlRequest::next_seq`]).
    seq: u64,
}

/// The sequence number of the next request made.
static NEXT_SEQ: AtomicU64 = AtomicU64::new(0);

/// A request's state: waiting to run, taken by the app ([`ControlRequest::claim`]), or given up
/// by its caller before that ([`Ticket::give_up`]). It leaves `QUEUED` once, one way or the other.
const QUEUED: u8 = 0;
const STARTED: u8 = 1;
const GIVEN_UP: u8 = 2;

/// The caller's side of a queued [`ControlRequest`].
pub struct Ticket(Arc<AtomicU8>);

impl Ticket {
    /// Stop waiting for the request → `true` when it had not started, and so never runs: a caller
    /// told "timeout" can then retry without the edit being applied twice, or applied later when
    /// nobody expects it (#1725). `false`: the app is running it (or has); its outcome is unknown.
    pub fn give_up(&self) -> bool {
        self.0.compare_exchange(QUEUED, GIVEN_UP, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<ControlResponse>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let seq = NEXT_SEQ.fetch_add(1, Ordering::SeqCst);
        (Self { method: method.into(), params, reply: tx, state: Arc::new(AtomicU8::new(QUEUED)), seq }, rx)
    }

    /// The sequence number the next request made gets: every request made so far is before it.
    pub fn next_seq() -> u64 {
        NEXT_SEQ.load(Ordering::SeqCst)
    }

    /// Whether this request was made before the one numbered `seq`.
    pub fn made_before(&self, seq: u64) -> bool {
        self.seq < seq
    }

    /// The caller's handle on this request, to give it up with.
    pub fn ticket(&self) -> Ticket {
        Ticket(self.state.clone())
    }

    /// Whether its caller gave it up (it never runs), without taking it.
    pub fn given_up(&self) -> bool {
        self.state.load(Ordering::SeqCst) == GIVEN_UP
    }

    /// Take the request to run it → `false` when its caller gave it up meanwhile (don't run it).
    pub fn claim(&self) -> bool {
        self.state.compare_exchange(QUEUED, STARTED, Ordering::SeqCst, Ordering::SeqCst).is_ok()
    }
}

pub enum Outcome {
    Done(Value),
    /// A window capture, answered once a frame was presented: written to `path`, the PNG sent
    /// back too with `data`.
    Screenshot {
        path: Option<String>,
        data: bool,
    },
    /// A `ui.pointer` reply ([`pointer_reply`]), sent once the events numbered `from..to`
    /// ([`vectorcraft_engine::Session::input_applied`]) have been made and the edit they started
    /// on another thread is applied, so it shows the selection after the edit.
    AfterToolEdit {
        from: u64,
        to: u64,
        document: Option<u64>,
    },
}

/// The reply to `ui.pointer`: the selection, the key object and the active tool.
pub(crate) fn pointer_reply(app: &mut VectorcraftApp) -> Value {
    pointer_reply_for(app, app.session.active().map(|d| d.uid))
}

pub(crate) fn pointer_reply_for(app: &VectorcraftApp, document: Option<u64>) -> Value {
    let state = document.and_then(|uid| app.session.documents().iter().find(|d| d.uid == uid));
    match state {
        Some(state) => json!({
            "ok": true,
            "result": {
                "selection": state.selection.objects.iter().map(|id| id.0).collect::<Vec<_>>(),
                "key": state.selection.key.map(|id| id.0),
                "tool": app.session.tool_id(),
            }
        }),
        None => json!({"ok": false, "error": "the gesture's document is no longer open"}),
    }
}

fn ok(v: Value) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}
pub(crate) fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}
fn wrap(r: Result<Value, String>) -> Outcome {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

pub fn all_commands(app: &VectorcraftApp) -> Value {
    let mut v: Vec<Value> = app.session.commands().into_iter().map(|c| serde_json::to_value(c).unwrap_or_default()).collect();
    for (id, label, sc, params) in crate::menus::UI_COMMANDS {
        v.push(json!({"id": id, "label": label, "shortcut": sc, "params": params, "enabled": crate::menus::enabled(app, id), "ui": true}));
    }
    Value::Array(v)
}

pub fn inspect(app: &VectorcraftApp, ctx: &egui::Context) -> Value {
    let r = ctx.content_rect();
    let mut ui = serde_json::to_value(&app.ui).unwrap_or_default();
    // The old floating panels' record, kept only to read older preferences: `ui.docking` has
    // the layout.
    if let Some(fields) = ui.as_object_mut() {
        fields.remove("floating_panels");
        leave_out_unreadable_paths(app, fields);
    }
    json!({
        "scriptRunning": app.session.script_running(),
        "scriptReport": app.session.shown_script_report(),
        "tool": app.session.tool_id(),
        "toolOptions": app.session.tool_options(),
        "ui": ui,
        // Not saved with the UI state, so not in `ui`: 0 normal … 3 Presentation Mode.
        "screenMode": app.ui.screen_mode,
        // The Contextual Task Bar: pinned (not saved either) and where it shows.
        "taskBar": {
            "pinned": app.ui.task_bar_place.pinned,
            "rect": crate::canvas::task_bar_rect(ctx).map(|r| json!([r.left(), r.top(), r.width(), r.height()])),
        },
        // The Free Transform tool's widget, where it shows.
        "freeTransformWidget": crate::free_transform::rect(ctx).map(|r| json!([r.left(), r.top(), r.width(), r.height()])),
        "view": app.view().map(|v| serde_json::to_value(v).unwrap_or_default()),
        "canvasRect": app.canvas_rect.map(|c| json!([c.left(), c.top(), c.width(), c.height()])),
        // Window › Arrange › Tile (not saved, so not in `ui`) and the panes the last frame drew:
        // each one's document index, view number (Window › New Window) and rect. Canvas
        // coordinates map to the current window's pane, inside its rulers and scroll bars
        // (`canvasRect`).
        "tiled": app.ui.tiled,
        "panes": app.canvas.panes.last.iter().map(|(w, c)| json!({
            "document": w.0,
            "view": w.1,
            "rect": [c.left(), c.top(), c.width(), c.height()],
            "active": crate::doc_windows::current(app) == Some(*w),
        })).collect::<Vec<_>>(),
        // Window › Arrange › Float in Window: the floating windows' documents, by index. While the
        // current one floats, `canvasRect` is in its window's coordinates.
        "floating": crate::doc_windows::floating_windows(app).iter().map(|w| w.0).collect::<Vec<_>>(),
        // Window › New Window: every window, as [document index, view number], in tab order, and
        // the current one.
        "windows": crate::doc_windows::windows(app),
        "currentWindow": crate::doc_windows::current(app),
        "window": [r.width(), r.height()],
        "documents": app.session.documents().iter().map(|d| json!({"title": d.title(), "dirty": d.is_dirty()})).collect::<Vec<_>>(),
        "activeDocument": app.session.active_index(),
        "perf": {"frameMs": app.perf.frame_ms, "renderMs": app.perf.render_ms, "fps": app.perf.fps},
        "graphicsAdapter": app.graphics_adapter,
        // The menus are in the macOS menu bar rather than the window.
        "nativeMenuBar": app.services.native_menu.is_some(),
        // Saves and exports still being written in the background (Background Save / Export).
        "background": app.background.jobs.iter().filter(|j| !j.label.is_empty()).map(|j| j.label.as_str()).collect::<Vec<_>>(),
        // Documents still being read off the UI thread (File › Open, Open Recent): their paths, without
        // the ones outside the read root.
        "opening": app.opening.pending.iter().map(|p| p.path.as_str()).filter(|p| readable(p)).collect::<Vec<_>>(),
        // Files still being read off the UI thread for Place (the Place dialog's OK, drops): their
        // names, in the order they are placed.
        "placing": app.placing.pending.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        // The screen color sampler is shown, waiting for the person at the computer
        // (`ui.sampleScreenColor`).
        "screenSampler": app.screen_sampler.is_some(),
        // A native file dialog open off the UI thread (`open`, `place`, `save`, `folder`), waiting
        // for the person at the computer: what asked for it runs again when it answers.
        "fileDialog": app.picks.waiting_kind(),
    })
}

/// The fields of the UI state, by their keys in `ui.inspect`'s `ui`, that hold panel ids: the panel
/// popped out of the collapsed column, the docking layout, closed panels' places and groups, the panels
/// shown as icons or collapsed, and the older floating panels' record.
const PANEL_FIELDS: [&str; 7] =
    ["open_panel", "docking", "docking_hidden", "docking_hidden_groups", "docking_icons", "docking_collapsed", "floating_panels"];

/// The ids of the loaded libraries (`loaded/<path>`) that the UI state refers to and whose file the
/// request may not read, each with the id a reply shows instead: `loaded/<file name>`, as a library
/// loaded without a path is named. The UI state refers to a library by its bare id in a library panel
/// and the Color Guide's limit, by a panel id (`library:<kind>:loaded/<path>`) in the fields of
/// [`PANEL_FIELDS`], the workspaces' layouts and the keys of shortcut overrides (`panel:<panel id>`,
/// saved and in an open Keyboard Shortcuts dialog), and by either in the `library` and `panel` params of
/// recorded actions' steps. Text the person types (a command palette query, a workspace's or an
/// action's name) is not read. The session's loaded libraries whose file the request may not read are
/// hidden too. The ids are numbered as the library commands number them
/// ([`vectorcraft_engine::cmd::swatchlib::concealed_library_ids`]), so that two libraries stay two and
/// `ui.inspect`, `ui.docking` and the library commands show a library under the same id.
fn unreadable_library_ids(app: &VectorcraftApp) -> Hiding {
    let mut rest = std::collections::BTreeSet::new();
    let ui = serde_json::to_value(&app.ui).unwrap_or_default();
    for field in PANEL_FIELDS {
        collect_library_panel_ids(&ui[field], &mut rest);
    }
    // A shortcut for showing a panel is keyed `panel:<id>`, saved and in an open Keyboard Shortcuts dialog.
    for overrides in [&ui["shortcut_overrides"], &ui["dialog"]["fields"]["overrides"]] {
        for panel in overrides.as_object().into_iter().flat_map(|o| o.keys()).filter_map(|key| key.strip_prefix("panel:")) {
            collect_library_panel_ids(&json!(panel), &mut rest);
        }
    }
    collect_library_params(&ui["action_sets"], &mut rest);
    for workspace in ["custom_workspaces", "workspace_changes"].iter().filter_map(|k| ui[k].as_array()).flatten() {
        for (_, layout) in workspace.as_object().into_iter().flatten().filter(|(k, _)| k.as_str() != "name") {
            collect_library_panel_ids(layout, &mut rest);
        }
    }
    collect_library_panel_ids(&json!(app.ui.transient_panel), &mut rest);
    let mut ids = rest;
    for id in app.ui.library_panels.iter().map(|p| p.id.as_str()).chain([app.ui.color_guide_limit.as_str()]) {
        add_library_id(id, &mut ids);
    }
    let mut hidden = vectorcraft_engine::cmd::swatchlib::concealed_library_ids(&app.session, ids.iter().cloned());
    // The other loaded library ids the session and the UI state hold.
    let s = &app.session;
    let loaded = s.swatch_libraries.infos().chain(s.style_libraries.infos()).chain(s.symbol_libraries.infos()).chain(s.brush_libraries.infos());
    let readable_ids = loaded
        .map(|info| info.id.clone())
        .chain(ids)
        .filter(|id| id.len() > "loaded/".len() && id.starts_with("loaded/") && !hidden.iter().any(|(h, _)| h == id))
        .collect();
    // The longest first, so that an id that begins another one does not replace part of it.
    hidden.sort_by_key(|(id, _)| std::cmp::Reverse(id.len()));
    Hiding { hidden, readable: readable_ids, sent: Vec::new() }
}

/// What a reply hides ([`hide_library_ids`]).
#[derive(Debug, Default)]
struct Hiding {
    /// The ids of the loaded libraries the request may not read, longest first, each with the id
    /// shown instead ([`unreadable_library_ids`]).
    hidden: Vec<(String, String)>,
    /// The other loaded library ids the UI state and the session hold: an occurrence of a hidden id or
    /// path that begins one of them stays ([`begins_known_id`]).
    readable: std::collections::BTreeSet<String>,
    /// The paths outside the read roots that the request sent ([`sent_paths`]), each as a hidden
    /// library id with the id shown. They are matched after the hidden libraries.
    sent: Vec<(String, String)>,
}

impl Hiding {
    /// Adds the hidden and readable ids of `other` that this one does not hold.
    fn merge(&mut self, other: Hiding) {
        self.add_hidden(other.hidden);
        self.readable.extend(other.readable);
    }

    /// Adds the hidden ids of `pairs` that this one does not hold, each with the id shown.
    fn add_hidden(&mut self, pairs: Vec<(String, String)>) {
        for pair in pairs {
            if !self.hidden.iter().any(|(id, _)| *id == pair.0) {
                self.hidden.push(pair);
            }
        }
        self.hidden.sort_by_key(|(id, _)| std::cmp::Reverse(id.len()));
    }

    /// Adds the sent paths of `pairs`, each as a hidden id with the id shown. A hidden id this one holds
    /// for the same path (written with either separator when it is a Windows path
    /// ([`splits_at_backslash`]), relative or as the absolute path it reads as ([`absolute_form`]), and
    /// in any case of ASCII letters with `fold`) is shown as the sent path is.
    fn set_hidden(&mut self, pairs: Vec<(String, String)>, fold: bool) {
        let key = |id: &str| {
            let path = id.strip_prefix("loaded/").unwrap_or(id);
            let id = absolute_form(path).unwrap_or_else(|| path.to_string());
            let windows = splits_at_backslash(&id);
            let id = if windows { id.replace('\\', "/") } else { id };
            (windows, if fold { id.to_ascii_lowercase() } else { id })
        };
        let sent: Vec<(bool, String)> = pairs.iter().map(|(id, _)| key(id)).collect();
        for (id, shown) in &mut self.hidden {
            if let Some(at) = sent.iter().position(|k| *k == key(id)) {
                shown.clone_from(&pairs[at].1);
            }
        }
        for pair in pairs {
            if !self.sent.iter().any(|(id, _)| *id == pair.0) {
                self.sent.push(pair);
            }
        }
    }
}

/// The paths outside the read roots that `v`'s strings and keys name at any depth, written as paths
/// ([`unreadable_path_like`]) or held by a path param
/// ([`vectorcraft_engine::file_access::is_path_key`]), a relative one too, each as a hidden library id
/// (`loaded/<path>`) with `loaded/<file name>` shown. A reply hides them as it hides a hidden library's
/// path, with that file name also for a hidden library shown as `loaded/<file name> (2)`, so that it
/// reads the same whether or not a library was loaded from one of them.
fn sent_paths(v: &Value, out: &mut Vec<(String, String)>) {
    collect_sent_paths(v, false, out);
}

/// [`sent_paths`] on `v`, which a path param holds when `path_param` is set.
fn collect_sent_paths(v: &Value, path_param: bool, out: &mut Vec<(String, String)>) {
    fn add(s: &str, path_param: bool, out: &mut Vec<(String, String)>) {
        if !unreadable_path_like(s) && !(path_param && !readable_value(s)) {
            return;
        }
        // A URL that names another host as the path after its scheme (`//host/share/…`).
        let path = match file_url_path(s) {
            Some(Some(path)) => path,
            Some(None) => percent_decoded(&s[5..]),
            None => s.to_string(),
        };
        let name = vectorcraft_engine::cmd::swatchlib::concealed_file_name(&path);
        if name == path {
            return;
        }
        let pair = (format!("loaded/{path}"), format!("loaded/{name}"));
        if !out.contains(&pair) {
            out.push(pair);
        }
    }
    match v {
        Value::String(s) => add(s, path_param, out),
        Value::Array(items) => items.iter().for_each(|item| collect_sent_paths(item, path_param, out)),
        Value::Object(fields) => {
            for (key, item) in fields {
                add(key, false, out);
                collect_sent_paths(item, vectorcraft_engine::file_access::is_path_key(key), out);
            }
        }
        _ => {}
    }
}

/// Adds `id` to `ids` when it is a loaded library's id (`loaded/<path>`).
fn add_library_id(id: &str, ids: &mut std::collections::BTreeSet<String>) {
    if id.len() > "loaded/".len() && id.starts_with("loaded/") {
        ids.insert(id.to_string());
    }
}

/// Adds to `ids` the loaded library id of every library panel id (`library:<kind>:loaded/<path>`)
/// that `v` holds as a string or a key.
fn collect_library_panel_ids(v: &Value, ids: &mut std::collections::BTreeSet<String>) {
    let add = |s: &str, ids: &mut std::collections::BTreeSet<String>| {
        if let Some((_, id)) = s.strip_prefix(crate::panels::library_panel::PREFIX).and_then(|rest| rest.split_once(':')) {
            add_library_id(id, ids);
        }
    };
    match v {
        Value::String(s) => add(s, ids),
        Value::Array(items) => items.iter().for_each(|item| collect_library_panel_ids(item, ids)),
        Value::Object(fields) => {
            for (key, item) in fields {
                add(key, ids);
                collect_library_panel_ids(item, ids);
            }
        }
        _ => {}
    }
}

/// Writes each id of `ids` as the id the reply shows instead, and the path in it as the file name
/// shown, wherever `v` holds them, in every string and key: a library's id, its panel's id
/// (`library:swatches:loaded/<path>`), status text, a dialog, the command palette query, a script's
/// report and a recorded step. An occurrence that begins another loaded library's id stays
/// ([`begins_known_id`]): a readable library's id that begins with a hidden one keeps its path. On
/// Windows the ids and paths are matched in any case of ASCII letters.
fn hide_library_ids(v: &mut Value, hiding: &Hiding) {
    let pairs = hidden_text(hiding);
    if pairs.iter().all(Vec::is_empty) {
        return;
    }
    replace_in_value(v, &Rewrite { pairs, hiding, fold: cfg!(windows) });
}

/// The text [`hide_library_ids`] replaces, in two groups: the hidden libraries' text, then the text of
/// the paths the request sent ([`Hiding::sent`]), each longest first. For each id, that is the id with
/// the id shown, and the path in it with the file name shown, a Windows path ([`splits_at_backslash`])
/// written with either separator too, and as `{:?}` quotes it (each `\`, quote and control character
/// escaped). A relative path is also replaced as the absolute path the file access check reads it as,
/// against the working folder.
fn hidden_text(hiding: &Hiding) -> Vec<Vec<(String, String)>> {
    vec![text_pairs(&hiding.hidden), text_pairs(&hiding.sent)]
}

/// Is `path` a Windows path, whose parts a `\` separates as well as a `/`: any path on Windows, and
/// elsewhere one that starts with a drive (`C:`) or a `\`? Outside Windows, a `\` in any other path is
/// part of a name.
fn splits_at_backslash(path: &str) -> bool {
    cfg!(windows) || path.starts_with('\\') || matches!(path.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic())
}

/// The absolute path that `path`, a relative path (one that does not start with a separator, a drive
/// or `~`), reads as from the working folder, as the file access check reads it; `None` for any other
/// path.
fn absolute_form(path: &str) -> Option<String> {
    let relative = !path.starts_with(['/', '\\', '~']) && !matches!(path.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic());
    relative.then(|| std::path::absolute(path).ok()).flatten().map(|p| p.to_string_lossy().into_owned())
}

/// The text [`hidden_text`] replaces for the ids of `ids`, longest first.
fn text_pairs(ids: &[(String, String)]) -> Vec<(String, String)> {
    let quoted = |s: &str| {
        let q = format!("{s:?}");
        q[1..q.len() - 1].to_string()
    };
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (id, shown) in ids {
        let path = &id["loaded/".len()..];
        let name = shown.strip_prefix("loaded/").unwrap_or(shown);
        for path in std::iter::once(path.to_string()).chain(absolute_form(path)) {
            // A Windows path is also written with the other separator; outside Windows, a native
            // path's `\` is part of a name, so that path is written only as it is.
            let mut spellings = vec![path.clone()];
            if splits_at_backslash(&path) {
                spellings.extend([path.replace('\\', "/"), path.replace('/', "\\")].into_iter().filter(|p| splits_at_backslash(p)));
            }
            let debug: Vec<_> = spellings.iter().map(|p| (quoted(p), quoted(name))).collect();
            let plain = spellings.into_iter().map(|p| (p, name.to_string()));
            for (path, name) in plain.chain(debug) {
                for (from, to) in [(format!("loaded/{path}"), format!("loaded/{name}")), (path, name)] {
                    if !pairs.iter().any(|(f, _)| *f == from) {
                        pairs.push((from, to));
                    }
                }
            }
        }
    }
    pairs.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    pairs
}

/// What [`replace_in_value`] writes: the groups of text to replace ([`hidden_text`]), what the reply
/// hides, and whether to match in any case of ASCII letters.
struct Rewrite<'a> {
    pairs: Vec<Vec<(String, String)>>,
    hiding: &'a Hiding,
    fold: bool,
}

/// The keys of encoded file data in a reply. `thumbnail` holds the PNG of a library item in
/// `library.get` and of a page in `document.pdfInfo`.
const BASE64_FIELDS: [&str; 4] = ["dataBase64", "pngBase64", "thumbnailBase64", "thumbnail"];

/// Is `s` written in the base64 alphabet, padded to a multiple of 4?
fn is_base64(s: &str) -> bool {
    let body = s.trim_end_matches('=');
    s.len().is_multiple_of(4) && s.len() - body.len() <= 2 && body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

/// Writes each `from` text of the rewrite's pairs as its `to` text in every string and key of `v`, as
/// [`replace_text`] does. Encoded file data, a base64 string under a key of [`BASE64_FIELDS`], stays
/// as it is. A rewritten key that reads the same as another key of its object takes ` (2)`, ` (3)`
/// and so on; a key that is not rewritten keeps its name.
fn replace_in_value(v: &mut Value, r: &Rewrite) {
    match v {
        Value::String(s) => {
            if let Some(replaced) = replace_text(s, r) {
                *s = replaced;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| replace_in_value(item, r)),
        Value::Object(fields) => {
            // The keys that stay are written first, so a rewritten key that reads as one of them, or
            // as another rewritten key, is the one numbered and no field is lost.
            let mut rewritten = Vec::new();
            for (key, mut item) in std::mem::take(fields) {
                if BASE64_FIELDS.contains(&key.as_str()) && item.as_str().is_some_and(is_base64) {
                    fields.insert(key, item);
                    continue;
                }
                replace_in_value(&mut item, r);
                match replace_text(&key, r) {
                    Some(written) => rewritten.push((written, item)),
                    None => {
                        fields.insert(key, item);
                    }
                }
            }
            for (written, item) in rewritten {
                let mut key = written.clone();
                let mut n = 2;
                while fields.contains_key(&key) {
                    key = format!("{written} ({n})");
                    n += 1;
                }
                fields.insert(key, item);
            }
        }
        _ => {}
    }
}

/// The byte offsets in `text` at which `needle` starts, not overlapping, in any case of ASCII letters
/// with `fold`.
fn occurrences(text: &str, needle: &str, fold: bool) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    if fold {
        let (text, needle) = (text.to_ascii_lowercase(), needle.to_ascii_lowercase());
        text.match_indices(needle.as_str()).map(|(at, _)| at).collect()
    } else {
        text.match_indices(needle).map(|(at, _)| at).collect()
    }
}

/// `s` with the `from` texts of the rewrite's pairs written as their `to` texts, at each occurrence
/// that does not begin another loaded library's id ([`begins_known_id`]). A hidden id or path written
/// with `%XX` escapes (in a `file:` URL) is replaced too; the escapes around it stay. Every occurrence
/// is found in `s` as it was given, the hidden libraries' text before the sent paths' text, each group
/// longest `from` first, and one that overlaps an occurrence already found, or a loaded library's id
/// that one began, stays: a replacement's text is not searched again. With the rewrite's `fold`, the
/// ids and paths of a group as written are found first, so that two libraries whose paths differ only
/// in letter case keep their own ids, and then the ones written in another case. `None` when nothing
/// changed.
fn replace_text(s: &str, r: &Rewrite) -> Option<String> {
    let decoded = if has_percent_escape(s) { decoded_with_spans(s).filter(|(decoded, _)| decoded != s) } else { None };
    let mut found = Found { kept: readable_ids_in(s, None, r), ..Found::default() };
    if let Some((text, spans)) = &decoded {
        found.kept.extend(readable_ids_in(text, Some(spans.as_slice()), r));
    }
    for group in &r.pairs {
        for fold in [false, true].into_iter().take(if r.fold { 2 } else { 1 }) {
            for (from, to) in group {
                if let Some((text, spans)) = &decoded {
                    found.claim(text, Some(spans.as_slice()), from, to, fold, r);
                }
                found.claim(s, None, from, to, fold, r);
            }
        }
    }
    if found.cuts.is_empty() {
        return None;
    }
    let mut cuts = found.cuts;
    cuts.sort_unstable_by_key(|&(start, _, _)| start);
    let mut out = String::with_capacity(s.len());
    let mut copied = 0;
    for (start, stop, to) in cuts {
        out.push_str(&s[copied..start]);
        out.push_str(to);
        copied = stop;
    }
    out.push_str(&s[copied..]);
    Some(out)
}

/// The spans of a text that [`replace_text`] replaces, each with its replacement, every span it has
/// taken (the ones it replaces and the loaded library ids it keeps), and the whole readable library ids
/// the text holds ([`readable_ids_in`]).
#[derive(Default)]
struct Found<'a> {
    cuts: Vec<(usize, usize, &'a str)>,
    taken: Vec<(usize, usize)>,
    kept: Vec<(usize, usize)>,
}

impl<'a> Found<'a> {
    /// Takes each occurrence of `from` in `text` that overlaps no span taken and lies inside no whole
    /// readable library id: as a span to write as `to`, or, where it begins a loaded library's id
    /// ([`begins_known_id`]), that id's span, kept. `spans` gives the range of the original text each
    /// byte of a decoded `text` was read from.
    fn claim(&mut self, text: &str, spans: Option<&[(usize, usize)]>, from: &str, to: &'a str, fold: bool, r: &Rewrite) {
        let span = |at: usize, end: usize| spans.map_or((at, end), |s| (s[at].0, s[end - 1].1));
        for at in occurrences(text, from, fold) {
            let end = at + from.len();
            let (start, stop) = span(at, end);
            let inside_readable = self.kept.iter().any(|&(a, b)| a <= start && stop <= b);
            if inside_readable || self.taken.iter().any(|&(a, b)| start < b && a < stop) || ends_a_longer_path(text, at, from) {
                continue;
            }
            match begins_known_id(text, at, end, r) {
                Some(len) => self.taken.push(span(at, at + len)),
                None => {
                    // A `file:` URL of the path is replaced whole, so no `file://<name>` is left that
                    // would name a host, unless part of its scheme was taken already.
                    let url = span(url_start(text, at, from), end).0;
                    let start = if self.taken.iter().any(|&(a, b)| url < b && a < stop) { start } else { url };
                    self.taken.push((start, stop));
                    self.cuts.push((start, stop, to));
                }
            }
        }
    }
}

/// Where a `file:` URL that holds `from`, an absolute path found at `at` in `text`, begins: `at` less
/// the `file:` before it, with `//` and a host (`localhost`, another host or none) between them, in any
/// case of its letters; before a drive, the host is followed by a `/` too. `at` when no such URL holds it.
fn url_start(text: &str, at: usize, from: &str) -> usize {
    let drive = matches!(from.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic());
    if !from.starts_with('/') && !drive {
        return at;
    }
    let Some(before) = text.get(..at) else { return at };
    let Some(scheme) = before.to_ascii_lowercase().rfind("file:") else { return at };
    let rest = &before[scheme + "file:".len()..];
    let host = |h: &str| h.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']' | '%'));
    let holds = if drive {
        rest.strip_prefix("//").and_then(|r| r.strip_suffix('/')).is_some_and(host)
    } else {
        rest.is_empty() || rest.strip_prefix("//").is_some_and(host)
    };
    if holds { scheme } else { at }
}

/// `text` with its `%XX` escapes decoded, and for each byte of the decoded text the range of `text`
/// it was read from. A decoded byte that is not part of UTF-8 text is read as U+FFFD.
fn decoded_with_spans(text: &str) -> Option<(String, Vec<(usize, usize)>)> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut spans = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|h| bytes[i] == b'%' && h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(v) => {
                out.push(v);
                spans.push((i, i + 3));
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                spans.push((i, i + 1));
                i += 1;
            }
        }
    }
    let mut decoded = String::with_capacity(out.len());
    let mut decoded_spans = Vec::with_capacity(spans.len());
    let mut at = 0;
    for chunk in out.utf8_chunks() {
        decoded.push_str(chunk.valid());
        decoded_spans.extend_from_slice(spans.get(at..at + chunk.valid().len())?);
        at += chunk.valid().len();
        for _ in chunk.invalid() {
            decoded.push(char::REPLACEMENT_CHARACTER);
            let span = *spans.get(at)?;
            decoded_spans.extend([span; 3]);
            at += 1;
        }
    }
    Some((decoded, decoded_spans))
}

/// Does `text` hold a `%XX` escape, one that [`decoded_with_spans`] decodes? Only such an escape
/// changes the decoded text.
fn has_percent_escape(text: &str) -> bool {
    text.as_bytes().windows(3).any(|w| w[0] == b'%' && w[1].is_ascii_hexdigit() && w[2].is_ascii_hexdigit())
}

/// Is `from`, a relative path (one that does not start with a separator, a drive or `loaded/`), at
/// `text[at..]` the end of a longer path: after any `.` and `..` parts just before it, is it preceded by
/// a letter, a digit, `_`, `-`, `.`, `~` or a separator? `/work/data/icon.svg` and
/// `/work/./data/icon.svg` do not hold the path `data/icon.svg`; `./data/icon.svg` and
/// `../data/icon.svg` do.
fn ends_a_longer_path(text: &str, at: usize, from: &str) -> bool {
    let absolute =
        from.starts_with(['/', '\\']) || from.starts_with("loaded/") || matches!(from.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic());
    let mut before = &text[..at];
    while let Some(head) = before.strip_suffix(['/', '\\']) {
        let Some(rest) = head.strip_suffix("..").or_else(|| head.strip_suffix('.')) else { break };
        before = rest;
        if !before.ends_with(['/', '\\']) {
            break;
        }
    }
    !absolute && before.ends_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '/' | '\\'))
}

/// Does text that follows a path continue it: a letter, a digit, `_` or a separator, after any `.`,
/// `-` or `~`? A space, punctuation, a quote, a sentence's final period and the end of the text end it.
fn continues_path(rest: &str) -> bool {
    rest.trim_start_matches(['.', '-', '~']).starts_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '/' | '\\'))
}

/// Does the occurrence `text[at..end]` begin a longer loaded library id ([`Hiding::readable`]), written
/// with or without its `loaded/` and with no `.` or `..` part, that ends where the text stops
/// continuing a path ([`continues_path`])? The length of that id as written, when it does.
/// A readable library's id that begins with a hidden one stays, also with a space after the hidden
/// part; a hidden path followed by other path text (`<path>.bak`, `<path>/../x`) is still hidden.
fn begins_known_id(text: &str, at: usize, end: usize, r: &Rewrite) -> Option<usize> {
    let rest = &text[at..];
    let same = |a: &str, b: &str| if r.fold { a.eq_ignore_ascii_case(b) } else { a == b };
    let dotted = |id: &str| id.split(['/', '\\']).any(|part| part == "." || part == "..");
    let prefixed = rest.get(.."loaded/".len()).is_some_and(|head| same(head, "loaded/"));
    r.hiding.readable.iter().filter(|known| !dotted(known)).find_map(|known| {
        let id = if prefixed { known.as_str() } else { known.strip_prefix("loaded/").unwrap_or(known) };
        let begins = id.len() > end - at && rest.get(..id.len()).is_some_and(|head| same(head, id)) && !continues_path(&rest[id.len()..]);
        begins.then_some(id.len())
    })
}

/// The spans of `text` that hold a whole loaded library id ([`Hiding::readable`]), written with or
/// without its `loaded/` and with no `.` or `..` part: text that does not continue a path follows it
/// ([`continues_path`]), and it begins the text or follows a space, a quote, `,`, `;`, an opening
/// bracket or, written with its `loaded/`, `=`, unless a `file:` URL holds it ([`url_start`]). Any
/// other character before it, such as `+` or `@`, can be part of a longer path's name, and so can `=`
/// before a path. The URL must name no host or `localhost`
/// ([`in_local_file_url`]).
/// A hidden id or path found inside such a span stays, wherever it begins. `spans` gives the range of
/// the original text each byte of a decoded `text` was read from.
fn readable_ids_in(text: &str, spans: Option<&[(usize, usize)]>, r: &Rewrite) -> Vec<(usize, usize)> {
    let span = |at: usize, end: usize| spans.map_or((at, end), |s| (s[at].0, s[end - 1].1));
    let dotted = |id: &str| id.split(['/', '\\']).any(|part| part == "." || part == "..");
    let mut kept = Vec::new();
    for known in r.hiding.readable.iter().filter(|known| !dotted(known)) {
        let bare = known.strip_prefix("loaded/").filter(|bare| !bare.is_empty());
        for id in std::iter::once(known.as_str()).chain(bare) {
            let prefixed = id.starts_with("loaded/");
            for at in occurrences(text, id, r.fold) {
                let end = at + id.len();
                let after_delimiter = text[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ',' | ';' | '(' | '[' | '{' | '<') || (prefixed && c == '='));
                if !continues_path(&text[end..]) && (after_delimiter || in_local_file_url(text, at, id)) {
                    kept.push(span(at, end));
                }
            }
        }
    }
    kept
}

/// Is `path`, an absolute path at `text[at..]`, held by a `file:` URL ([`url_start`]) that names no host
/// or `localhost`, in any case of its letters?
fn in_local_file_url(text: &str, at: usize, path: &str) -> bool {
    let url = url_start(text, at, path);
    let Some(between) = text.get(url + "file:".len()..at).filter(|_| url < at) else { return false };
    let host = between.strip_prefix("//").unwrap_or(between).trim_end_matches('/');
    host.is_empty() || host.eq_ignore_ascii_case("localhost")
}

/// Text that a person or a script wrote, by its key: a command palette query, a script's report and a
/// workspace's name. [`swap_library_ids`] leaves it as written.
const TEXT_FIELDS: [&str; 3] = ["palette_query", "scriptReport", "name"];

/// Writes each `from` id of `pairs` as its `to` id wherever `v` holds it as a whole id ([`swap_whole_ids`]),
/// in strings and keys, except in the [`TEXT_FIELDS`].
fn swap_library_ids(v: &mut Value, pairs: &[(&str, &str)]) {
    if pairs.is_empty() {
        return;
    }
    match v {
        Value::String(s) => {
            if let Some(swapped) = swap_whole_ids(s, pairs) {
                *s = swapped;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| swap_library_ids(item, pairs)),
        Value::Object(fields) => {
            for (key, mut item) in std::mem::take(fields) {
                if TEXT_FIELDS.contains(&key.as_str()) {
                    fields.insert(key, item);
                    continue;
                }
                swap_library_ids(&mut item, pairs);
                fields.insert(swap_whole_ids(&key, pairs).unwrap_or(key), item);
            }
        }
        _ => {}
    }
}

/// `s` with each `from` id of `pairs` written as its `to` id where it is a whole id: the whole string,
/// or a part that starts the string or follows a `:`, `"` or `` ` `` (a panel id, a quoted id) and ends
/// the string or a quote. An id that only begins or ends a longer one stays. `None` when nothing changed.
fn swap_whole_ids(s: &str, pairs: &[(&str, &str)]) -> Option<String> {
    let mut out: Option<String> = None;
    for (from, to) in pairs {
        let text = out.as_deref().unwrap_or(s);
        if !text.contains(from) {
            continue;
        }
        let mut next = String::with_capacity(text.len());
        let mut copied = 0;
        for (at, _) in text.match_indices(from) {
            let end = at + from.len();
            let starts = at == 0 || text[..at].ends_with([':', '"', '`']);
            let ends = end == text.len() || text[end..].starts_with(['"', '`']);
            if at >= copied && starts && ends {
                next.push_str(&text[copied..at]);
                next.push_str(to);
                copied = end;
            }
        }
        if copied > 0 {
            next.push_str(&text[copied..]);
            out = Some(next);
        }
    }
    out
}

/// Does `v` hold a string or a key that names a loaded library (`loaded/…`)?
fn names_a_loaded_library(v: &Value) -> bool {
    match v {
        Value::String(s) => s.contains("loaded/"),
        Value::Array(items) => items.iter().any(names_a_loaded_library),
        Value::Object(fields) => fields.iter().any(|(key, item)| key.contains("loaded/") || names_a_loaded_library(item)),
        _ => false,
    }
}

/// The preferences that hold a folder, by their keys in `ui.engine_prefs` and the Preferences dialog.
const PREF_FOLDERS: [&str; 6] = ["fontsFolder", "pluginsFolder", "scratchPrimary", "scratchSecondary", "recoveryFolder", "templatesFolder"];

/// The scratch disk choices that are not paths.
const SCRATCH_CHOICES: [&str; 2] = ["Startup", "None"];

/// May the request being served read `path`? Under an automation read root (a control client's
/// request), `ui.inspect` leaves out a path outside the root, as `ui.menu.list` leaves out a recent
/// file or a listed script outside it. Another platform's absolute path
/// ([`vectorcraft_engine::file_access::written_elsewhere`]) is never readable under a read root.
fn readable(path: &str) -> bool {
    !vectorcraft_engine::file_access::written_elsewhere(path) && vectorcraft_engine::file_access::check_read(path).is_ok()
}

/// Is `s` a path the request may not read, written as an absolute path (either separator, with or
/// without a drive) or a `file:` URL ([`file_url_path`])? Under a read root, a `file:` URL that names
/// another host is never readable.
fn unreadable_absolute(s: &str) -> bool {
    match file_url_path(s) {
        Some(path) => !readable_url_path(path),
        None => {
            let absolute = s.starts_with(['/', '\\']) || matches!(s.as_bytes(), [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic());
            absolute && !readable(s)
        }
    }
}

/// Is `s` written as a path that the request may not read: an absolute path or a `file:` URL
/// ([`unreadable_absolute`]), `~` or a path under it, or a path that starts with `.` or `..` and a
/// separator? It is checked as the file access check reads it, relative to the working folder. Other
/// text with a separator (`a/b`) stays.
fn unreadable_path_like(s: &str) -> bool {
    if unreadable_absolute(s) {
        return true;
    }
    let t = s.trim_start();
    let relative = t.trim_end() == "~" || ["~/", "~\\", "./", ".\\", "../", "..\\"].iter().any(|p| t.starts_with(p));
    relative && !readable_value(s)
}

/// May the request read the path `s` names, a path or a `file:` URL ([`file_url_path`])?
fn readable_value(s: &str) -> bool {
    match file_url_path(s) {
        Some(path) => readable_url_path(path),
        None => readable(s),
    }
}

/// May the request read the local path a `file:` URL names ([`file_url_path`])? A URL that names
/// another host may be read only without a read root in force.
fn readable_url_path(path: Option<String>) -> bool {
    match path {
        Some(path) => readable(&path),
        None => vectorcraft_engine::file_access::current().is_none(),
    }
}

/// The local path that `s` names when it is a `file:` URL, with the scheme in any case and its `%XX`
/// escapes decoded: `Some(None)` for a URL that names another host (`file://host/share`).
/// `file:///C:/x` names `C:/x`.
fn file_url_path(s: &str) -> Option<Option<String>> {
    let rest = s.get(..5).filter(|scheme| scheme.eq_ignore_ascii_case("file:")).map(|_| &s[5..])?;
    let Some(rest) = rest.strip_prefix("//") else { return Some(Some(percent_decoded(rest))) };
    let rest = if rest.get(..9).is_some_and(|host| host.eq_ignore_ascii_case("localhost")) { &rest[9..] } else { rest };
    if !rest.starts_with('/') {
        return Some(None);
    }
    let path = match rest.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &rest[1..],
        _ => rest,
    };
    Some(Some(percent_decoded(path)))
}

/// `s` with `%XX` escapes decoded, as the SVG importer decodes an href.
fn percent_decoded(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = Vec::with_capacity(s.len());
    let mut rest = s.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let hex = tail
            .get(..2)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(v) if b == b'%' => {
                out.push(v);
                rest = tail.get(2..).unwrap_or_default();
            }
            _ => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The params of each step of each action of action set `set` (`ui.action_sets`).
fn steps_mut(set: &mut Value) -> impl Iterator<Item = &mut serde_json::Map<String, Value>> {
    let actions = set.get_mut("actions").and_then(Value::as_array_mut).into_iter().flatten();
    let steps = actions.filter_map(|action| action.get_mut("steps").and_then(Value::as_array_mut)).flatten();
    steps.filter_map(|step| step.get_mut(1).and_then(Value::as_object_mut))
}

/// Takes out of a recorded step's params, at any depth (the commands of a `command.batch` step), the
/// paths the request may not read: the value of a path param
/// ([`vectorcraft_engine::file_access::is_path_key`]: `path`, `file`, `diffPath`, `destinationFolder`
/// and the like), each path of a list under such a param, and under any other param, as a key too, a
/// string written as a path ([`unreadable_path_like`]).
fn leave_out_unreadable_params(params: &mut serde_json::Map<String, Value>) {
    use vectorcraft_engine::file_access::is_path_key;
    params.retain(|key, v| {
        !unreadable_path_like(key)
            && match v.as_str() {
                Some(p) => !unreadable_path_like(p) && !(is_path_key(key) && !readable_value(p)),
                None => true,
            }
    });
    for (key, v) in params.iter_mut() {
        if let Value::Array(paths) = v
            && is_path_key(key)
        {
            paths.retain(|p| p.as_str().is_none_or(readable_value));
        }
        leave_out_unreadable_values(v);
    }
}

/// [`leave_out_unreadable_params`] on each object `v` holds, and a string written as a path that the
/// request may not read ([`unreadable_path_like`]) taken out of each list.
fn leave_out_unreadable_values(v: &mut Value) {
    match v {
        Value::Object(fields) => leave_out_unreadable_params(fields),
        Value::Array(items) => {
            items.retain(|item| !item.as_str().is_some_and(unreadable_path_like));
            items.iter_mut().for_each(leave_out_unreadable_values);
        }
        _ => {}
    }
}

/// Takes out of `ui.inspect`'s copy of the UI state the paths the request may not read: recent files,
/// a library panel's file, folder preferences (saved, and in an open Preferences dialog) and the paths
/// in recorded actions' steps outside the read root. The file selected in an open JavaScript Files
/// dialog is named as `svgInteractivity.info` names the active document's linked files. The UI state
/// itself is not changed.
fn leave_out_unreadable_paths(app: &VectorcraftApp, ui: &mut serde_json::Map<String, Value>) {
    if let Some(Value::Array(files)) = ui.get_mut("recent_files") {
        files.retain(|f| f.as_str().is_some_and(readable));
    }
    if let Some(Value::Array(panels)) = ui.get_mut("library_panels") {
        for panel in panels.iter_mut().filter_map(Value::as_object_mut) {
            if panel.get("path").and_then(Value::as_str).is_some_and(|p| !readable(p)) {
                panel.remove("path");
            }
        }
    }
    for step in ui.get_mut("action_sets").and_then(Value::as_array_mut).into_iter().flatten().flat_map(steps_mut) {
        leave_out_unreadable_params(step);
    }
    if let Some(Value::Object(prefs)) = ui.get_mut("engine_prefs") {
        leave_out_unreadable_folders(prefs);
    }
    if let Some(Value::Object(dialog)) = ui.get_mut("dialog")
        && dialog.get("kind").and_then(Value::as_str) == Some("preferences")
        && let Some(Value::Object(fields)) = dialog.get_mut("fields")
    {
        leave_out_unreadable_folders(fields);
    }
    if let Some(Value::Object(dialog)) = ui.get_mut("dialog")
        && dialog.get("kind").and_then(Value::as_str) == Some(crate::dialogs::javascript_files::KIND)
        && let Some(Value::Object(fields)) = dialog.get_mut("fields")
        && let Some(Value::String(selected)) = fields.get_mut("selected")
        && let Some(st) = app.session.active()
    {
        *selected = vectorcraft_engine::cmd::links::shown_script(st, selected).into_owned();
    }
}

/// Takes out of `prefs` the folder preferences ([`PREF_FOLDERS`]) the request may not read, as written
/// or trimmed as the app uses it. An empty value (no folder) and a scratch disk choice
/// ([`SCRATCH_CHOICES`]) stay.
fn leave_out_unreadable_folders(prefs: &mut serde_json::Map<String, Value>) {
    for key in PREF_FOLDERS {
        let Some(written) = prefs.get(key).and_then(Value::as_str) else { continue };
        let path = written.trim();
        if path.is_empty() || (key.starts_with("scratch") && SCRATCH_CHOICES.contains(&path)) || (readable(written) && readable(path)) {
            continue;
        }
        prefs.remove(key);
    }
}

fn key_from(name: &str) -> Option<egui::Key> {
    egui::Key::from_name(name).or(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(egui::Key::Enter),
        "esc" | "escape" => Some(egui::Key::Escape),
        "delete" => Some(egui::Key::Delete),
        "backspace" => Some(egui::Key::Backspace),
        "left" => Some(egui::Key::ArrowLeft),
        "right" => Some(egui::Key::ArrowRight),
        "up" => Some(egui::Key::ArrowUp),
        "down" => Some(egui::Key::ArrowDown),
        "space" => Some(egui::Key::Space),
        "tab" => Some(egui::Key::Tab),
        _ => None,
    })
}

/// The modifier keys a request holds down (`cmd`, `ctrl`, `alt`, `shift`; `cmd` is Command on
/// macOS, Ctrl elsewhere).
fn modifiers(p: &Value) -> Result<egui::Modifiers, String> {
    let b = |n: &str| crate::menus::opt_bool(p, n).map(|on| on.unwrap_or(false));
    let cmd = b("cmd")?;
    Ok(egui::Modifiers { alt: b("alt")?, ctrl: b("ctrl")?, shift: b("shift")?, mac_cmd: cmd && cfg!(target_os = "macos"), command: cmd })
}

/// An optional number param of a control request as `f32`, `default` when it is omitted or null.
fn coord(p: &Value, key: &str, default: f32) -> Result<f32, String> {
    crate::menus::opt_number(p, key).map(|n| n.map_or(default, |n| n as f32))
}

/// A pointer packet's `mods`: none when omitted or null, else an object of booleans.
fn packet_mods(value: Option<&Value>) -> Result<Option<Mods>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(m) if !m.is_object() => Err("`mods` must be an object of true or false values".to_string()),
        Some(m) => serde_json::from_value(m.clone()).map(Some).map_err(|e| format!("`mods` must be an object of true or false values: {e}")),
    }
}

/// The params that name a library or a library panel, at any depth of a request (`ui.set {panel}`,
/// `engine.execute {params: {panel}}`, `ui.libraryPanel {panel, library}`), with the docking
/// commands' destinations (`window.panel.move {anchor, before}`, `ui.dock {target}`,
/// `window.panel.float {onto, above, below}`) and a list of panels. Only a whole id is translated
/// ([`swap_whole_ids`]), so other text in these params stays.
const LIBRARY_PARAMS: [&str; 9] = ["panel", "library", "anchor", "target", "before", "onto", "above", "below", "panels"];

/// The param of `window.panel.layout` that holds a serialized docking action: each panel id in it is
/// translated.
const DOCKING_ACTION: &str = "action";

/// Runs a control request. A library loaded from outside the read root keeps the id `ui.inspect`
/// shows for it ([`unreadable_library_ids`]) both ways: a request may name it or its panel by that id
/// in a [`LIBRARY_PARAMS`] param, and the reply shows that id. Under a read root, the reply shows each
/// such library's id and path as `ui.inspect` does ([`hide_library_ids`]) in every string and key,
/// errors included, an error from a panic too: the libraries hidden before it ran and after it. The
/// reply also shows each path outside the read root that the request sent by its file name, whether it
/// succeeded or failed ([`sent_paths`]), except where a `path` or `files` entry of its result names the
/// file the request wrote inside the write roots, as the request gave it. A reply with no `/`, `\` or `%` in it is not searched, since every hidden path
/// holds a separator, written or escaped. A screenshot's reply is hidden when it is sent
/// ([`hide_in_deferred_reply`]); a `ui.pointer` reply sent after the tool's edit holds no library id
/// or path.
pub fn handle(app: &mut VectorcraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    if vectorcraft_engine::file_access::current().is_none() {
        return dispatch(app, ctx, req);
    }
    // Pointer, key and text input does not load, close or rename a library: the libraries hidden after
    // it ran are the ones hidden before, so frequent input does not read the UI state twice.
    let before = if INPUT_METHODS.contains(&req.method.as_str()) { Hiding::default() } else { unreadable_library_ids(app) };
    let shown: Vec<_> = before.hidden.iter().map(|(id, shown)| (shown.as_str(), id.as_str())).collect();
    let mut params = req.params.clone();
    let swapped = !shown.is_empty() && names_a_loaded_library(&req.params) && swap_library_params(&mut params, &shown);
    let ran = vectorcraft_engine::guard::catch_panic(|| {
        if swapped {
            let request = ControlRequest { method: req.method.clone(), params, reply: req.reply.clone(), state: req.state.clone(), seq: req.seq };
            dispatch(app, ctx, &request)
        } else {
            dispatch(app, ctx, req)
        }
    });
    let outcome = match ran {
        Ok(outcome) => outcome,
        Err(msg) => {
            app.picks.recover();
            app.session.set_replaying(false);
            err(format!("internal error: {msg} (please report this bug)"))
        }
    };
    let Outcome::Done(mut reply) = outcome else { return outcome };
    hide_in_reply(app, &mut reply, &req.params, before);
    Outcome::Done(reply)
}

/// Hides in `reply` the libraries hidden now and in `before`, and the paths outside the read root that
/// `sent`, the request's params, holds ([`handle`]). The `path` and `files` entries of the reply's
/// `result`, at any depth, that name a file the request gave inside the write roots, the file it
/// wrote, stay as given ([`written_paths`]).
fn hide_in_reply(app: &VectorcraftApp, reply: &mut Value, sent: &Value, before: Hiding) {
    if !holds_a_separator(reply) {
        return;
    }
    let mut given = Vec::new();
    written_paths(sent, false, &mut given);
    let kept = kept_written_paths(reply, &given);
    let mut hiding = unreadable_library_ids(app);
    hiding.merge(before);
    let mut paths = Vec::new();
    sent_paths(sent, &mut paths);
    hiding.set_hidden(paths, cfg!(windows));
    hide_library_ids(reply, &hiding);
    restore_written_paths(reply, kept);
}

/// The strings that `v`'s path params hold at any depth ([`vectorcraft_engine::file_access::is_path_key`])
/// that name a file inside the write roots; `v` is a path param's value when `path_param` is set.
fn written_paths(v: &Value, path_param: bool, out: &mut Vec<String>) {
    use vectorcraft_engine::file_access::{Access, current, is_path_key};
    match v {
        Value::String(s) if path_param && !s.is_empty() && current().is_some_and(|r| r.check(s, Access::Write).is_ok()) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|item| written_paths(item, path_param, out)),
        Value::Object(fields) => {
            for (key, item) in fields {
                written_paths(item, is_path_key(key), out);
            }
        }
        _ => {}
    }
}

/// The entries under `reply`'s `result`, at any depth (a `command.batch` step's result too), that a
/// `path` key or a `files` list holds and that are one of `given`: each entry's JSON pointer and text.
fn kept_written_paths(reply: &Value, given: &[String]) -> Vec<(String, String)> {
    let mut kept = Vec::new();
    if !given.is_empty() {
        kept_under(&reply["result"], "/result", given, &mut kept);
    }
    kept
}

/// [`kept_written_paths`] for `v`, found at the JSON pointer `at`.
fn kept_under(v: &Value, at: &str, given: &[String], out: &mut Vec<(String, String)>) {
    let is_given = |v: &Value| v.as_str().filter(|s| given.iter().any(|g| g == s)).map(str::to_string);
    match v {
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                kept_under(item, &format!("{at}/{i}"), given, out);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                let here = format!("{at}/{}", key.replace('~', "~0").replace('/', "~1"));
                match (key.as_str(), is_given(item), item.as_array()) {
                    ("path", Some(path), _) => out.push((here, path)),
                    ("files", _, Some(rows)) => {
                        for (i, row) in rows.iter().enumerate() {
                            match is_given(row) {
                                Some(path) => out.push((format!("{here}/{i}"), path)),
                                None => kept_under(row, &format!("{here}/{i}"), given, out),
                            }
                        }
                    }
                    _ => kept_under(item, &here, given, out),
                }
            }
        }
        _ => {}
    }
}

/// Writes back in `reply` the entries [`kept_written_paths`] took from it.
fn restore_written_paths(reply: &mut Value, kept: Vec<(String, String)>) {
    for (at, path) in kept {
        if let Some(slot) = reply.pointer_mut(&at) {
            *slot = json!(path);
        }
    }
}

/// Hides in a reply sent after its request was handled (a screenshot's) what [`handle`] hides in a
/// reply, with `sent` as the request's params. Without a read root in force it leaves the reply as
/// it is.
pub(crate) fn hide_in_deferred_reply(app: &VectorcraftApp, reply: &mut Value, sent: &Value) {
    if vectorcraft_engine::file_access::current().is_some() {
        hide_in_reply(app, reply, sent, Hiding::default());
    }
}

/// The control methods that send synthetic input.
const INPUT_METHODS: [&str; 7] = ["ui.pointer", "ui.click", "ui.drag", "ui.move", "ui.wheel", "ui.key", "ui.text"];

/// Does a string or a key of `v`, at any depth, hold a `/`, a `\` or a `%`?
fn holds_a_separator(v: &Value) -> bool {
    let has = |s: &str| s.contains(['/', '\\', '%']);
    match v {
        Value::String(s) => has(s),
        Value::Array(items) => items.iter().any(holds_a_separator),
        Value::Object(fields) => fields.iter().any(|(key, item)| has(key) || holds_a_separator(item)),
        _ => false,
    }
}

/// Writes each `from` id of `pairs` in the [`LIBRARY_PARAMS`] params of `params`, at any depth, as its
/// `to` id → whether one was.
fn swap_library_params(params: &mut Value, pairs: &[(&str, &str)]) -> bool {
    match params {
        Value::Array(items) => items.iter_mut().fold(false, |swapped, item| swap_library_params(item, pairs) | swapped),
        Value::Object(fields) => fields.iter_mut().fold(false, |swapped, (key, item)| {
            let this = match item {
                Value::String(s) if LIBRARY_PARAMS.contains(&key.as_str()) => swap_whole_ids(s, pairs).map(|to| *s = to).is_some(),
                Value::Array(items) if LIBRARY_PARAMS.contains(&key.as_str()) => items.iter_mut().fold(false, |swapped, item| {
                    let this = item.as_str().and_then(|s| swap_whole_ids(s, pairs)).map(|to| *item = Value::String(to)).is_some();
                    this | swapped
                }),
                Value::Object(_) if key == DOCKING_ACTION => {
                    let before = item.clone();
                    swap_library_ids(item, pairs);
                    *item != before
                }
                _ => swap_library_params(item, pairs),
            };
            this | swapped
        }),
        _ => false,
    }
}

/// Adds to `ids` the loaded library ids that the [`LIBRARY_PARAMS`] params of `v` name, at any depth:
/// a library's id or its panel's id.
fn collect_library_params(v: &Value, ids: &mut std::collections::BTreeSet<String>) {
    match v {
        Value::Array(items) => items.iter().for_each(|item| collect_library_params(item, ids)),
        Value::Object(fields) => {
            for (key, item) in fields {
                if !LIBRARY_PARAMS.contains(&key.as_str()) {
                    collect_library_params(item, ids);
                    continue;
                }
                for s in item.as_str().into_iter().chain(item.as_array().into_iter().flatten().filter_map(Value::as_str)) {
                    add_library_id(s, ids);
                    collect_library_panel_ids(&json!(s), ids);
                }
            }
        }
        _ => {}
    }
}

fn dispatch(app: &mut VectorcraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    match req.method.as_str() {
        "engine.execute" | "ui.menu.invoke" | "command" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            let params = p.get("params").cloned().unwrap_or(json!({}));
            let params = if params.is_null() { json!({}) } else { params };
            // A key the command's description doesn't name is an error, not a default (#1442).
            if let Err(e) = crate::menus::check_params(&app.session, id, &params) {
                return err(e);
            }
            wrap(app.run(id, params))
        }
        "engine.commands" => ok(all_commands(app)),
        "document.inspect" => wrap(app.run("document.inspect", p.clone())),
        "ui.inspect" => ok(inspect(app, ctx)),
        "ui.docking" => ok(crate::panel_docking::inspect(app, ctx)),
        // A request that stops with a panic quoting the status text, after clearing the Color Guide's
        // limit with `forget`.
        #[cfg(test)]
        "test.panic" => {
            if p.get("forget").and_then(Value::as_bool) == Some(true) {
                app.ui.color_guide_limit.clear();
            }
            panic!("failed on {}", app.ui.status)
        }
        // A request that succeeds with the status text.
        #[cfg(test)]
        "test.status" => ok(json!(app.ui.status)),
        "ui.menu.list" => ok(serde_json::to_value(crate::menus::menu_entries(app)).unwrap_or_default()),
        "ui.contextMenu.list" => ok(serde_json::to_value(crate::menus::context_entries(app)).unwrap_or_default()),
        "ui.tool.select" => wrap(app.run("tool.select", json!({"tool": s("tool").unwrap_or("")}))),
        "ui.tool.list" => {
            let groups: Vec<_> = vectorcraft_tools::TOOL_GROUPS.iter().chain(vectorcraft_tools::addons::GROUPS).copied().collect();
            ok(serde_json::to_value(groups).unwrap_or_default())
        }
        "ui.pointer" => pointer_events(app, ctx, req),
        "ui.key" => {
            let Some(k) = s("key").and_then(key_from) else { return err("unknown or missing `key`") };
            let m = match modifiers(p) {
                Ok(m) => m,
                Err(e) => return err(e),
            };
            if let Err(e) = reserve_synthetic(app, 2 + usize::from(s("text").is_some())) {
                return err(e);
            }
            app.synthetic.push(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m });
            // Typed text comes with the press, so it shares the key's frame and modifiers.
            if let Some(t) = s("text") {
                app.synthetic.push(egui::Event::Text(t.to_string()));
            }
            app.synthetic.push(egui::Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m });
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.move" => {
            let pos = match coord(p, "x", 0.0).and_then(|x| Ok(egui::pos2(x, coord(p, "y", 0.0)?))) {
                Ok(pos) => pos,
                Err(e) => return err(e),
            };
            if let Err(e) = reserve_synthetic(app, 1) {
                return err(e);
            }
            app.synthetic.push(egui::Event::PointerMoved(pos));
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.click" | "ui.drag" => synthetic_click(app, ctx, req),
        "ui.wheel" => {
            // Mouse wheel notches over screen point (x, y): `dy` up (+) or down, `dx` sideways
            // (`unit: "point"` for trackpad-like points), with modifiers as `ui.click` takes them.
            let given =
                (|| Ok::<_, String>(([coord(p, "dx", 0.0)?, coord(p, "dy", 0.0)?, coord(p, "x", 0.0)?, coord(p, "y", 0.0)?], modifiers(p)?)))();
            let ([dx, dy, x, y], modifiers) = match given {
                Ok(given) => given,
                Err(e) => return err(e),
            };
            if let Err(e) = reserve_synthetic(app, 2) {
                return err(e);
            }
            let unit = if s("unit") == Some("point") { egui::MouseWheelUnit::Point } else { egui::MouseWheelUnit::Line };
            // Notches, not a flood: a turn stays within what a real wheel sends.
            let delta = egui::vec2(dx, dy).clamp(egui::Vec2::splat(-100.0), egui::Vec2::splat(100.0));
            app.synthetic.push(egui::Event::PointerMoved(egui::pos2(x, y)));
            app.synthetic.push(egui::Event::MouseWheel {
                unit,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers,
                source: egui::MouseWheelSource::Unknown,
            });
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.text" => {
            if let Err(e) = reserve_synthetic(app, 1) {
                return err(e);
            }
            app.synthetic.push(egui::Event::Text(s("text").unwrap_or("").to_string()));
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.set" => set_ui(app, req),
        "ui.dialog.set" => match app.ui.dialog.as_mut() {
            Some(d) => {
                let fields = match dialog_fields(p) {
                    Ok(fields) => fields,
                    Err(e) => return err(e),
                };
                let action = match dialog_action(p) {
                    Ok(action) => action,
                    Err(e) => return err(e),
                };
                // Every field is checked before any is set, so a request sets all of them or none.
                if let Some((f, _)) = fields.iter().find(|(f, _)| !crate::dialogs::accepts_field(d, f)) {
                    if crate::dialogs::protected_field(d, f) {
                        return err(format!("the `{}` dialog cannot set protected field `{f}`", d.kind));
                    }
                    let names = crate::dialogs::field_names(d).join(", ");
                    return err(format!("the `{}` dialog has no field `{f}`; its fields: {names}", d.kind));
                }
                // In order: a field that sets others (an export preset) goes before the fields after it.
                // A field that fails leaves the dialog as it was before the request.
                let before = app.ui.dialog.clone();
                for (f, value) in fields {
                    let old = app.ui.dialog.as_ref().and_then(|d| d.fields.get(&f).cloned());
                    if let Some(d) = app.ui.dialog.as_mut() {
                        crate::dialogs::set_field(d, f.clone(), value);
                    }
                    if let Err(e) = crate::dialogs::field_set(app, &f, old.as_ref()) {
                        app.ui.dialog = before;
                        return err(e);
                    }
                }
                let dialog = app.ui.dialog.as_ref().and_then(|d| serde_json::to_value(d).ok()).unwrap_or_default();
                // `action` answers the dialog the fields were just set in, within this request.
                match action {
                    None => ok(dialog),
                    Some(DialogAnswer::Confirm) => match crate::dialogs::confirm(app) {
                        Ok(result) => ok(json!({"dialog": dialog, "result": result})),
                        Err(e) => err(e),
                    },
                    Some(DialogAnswer::Cancel) => {
                        crate::dialogs::cancel(app);
                        ok(json!({"dialog": dialog, "result": null}))
                    }
                }
            }
            None => err("no dialog open"),
        },
        "ui.dialog.confirm" => wrap(crate::dialogs::confirm(app)),
        "ui.dialog.cancel" => {
            crate::dialogs::cancel(app);
            ok(Value::Null)
        }
        "ui.resize" => {
            let (w, h) = match coord(p, "width", 1440.0).and_then(|w| Ok((w, coord(p, "height", 900.0)?))) {
                Ok(size) => size,
                Err(e) => return err(e),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
            ok(Value::Null)
        }
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ok(Value::Null)
        }
        "ui.screenshot" => {
            // Refused now rather than once the frame comes (it is checked again then).
            if let Some(Err(e)) = s("path").map(vectorcraft_engine::file_access::check_write) {
                return err(e);
            }
            let flags = crate::menus::opt_bool(p, "focus").and_then(|focus| Ok((focus, crate::menus::opt_bool(p, "data")?)));
            let (focus, data) = match flags {
                Ok(flags) => flags,
                Err(e) => return err(e),
            };
            if focus.unwrap_or(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            ctx.request_repaint();
            Outcome::Screenshot { path: s("path").map(str::to_string), data: data == Some(true) }
        }
        "ui.render" => {
            let given = (|| {
                Ok::<_, String>((
                    crate::menus::opt_index(p, "artboard")?,
                    crate::menus::opt_number(p, "scale")?,
                    crate::menus::opt_bool(p, "traceViews")?,
                    crate::menus::opt_bool(p, "data")?,
                ))
            })();
            let (index, scale, trace_views, data) = match given {
                Ok(given) => given,
                Err(e) => return err(e),
            };
            let Some(st) = app.session.active() else { return err("no document") };
            let doc = st.doc.clone();
            // The artboard asked for (0-based, as headless), else the first.
            let Some(r) = doc.artboards.get(index.unwrap_or(0)).map(|a| a.rect) else { return err("no such artboard") };
            let scale = scale.unwrap_or(1.0);
            if let Err(e) = vectorcraft_render::raster_size(r, scale) {
                return err(e);
            }
            // Rendered with the options of a PNG export on white, so its pixels are the exported file's.
            // `traceViews: true` draws Image Trace objects as their View shows them, as the canvas does.
            let mut opts = RasterExportOptions { background: Some([255; 3]), ..Default::default() }.render_options(RasterFormat::Png);
            opts.trace_views = trace_views == Some(true);
            let img = app.canvas.renderer.render_region_with(&doc, r, scale, &opts);
            let png = match img.to_png() {
                Ok(png) => png,
                Err(e) => return err(e),
            };
            let data = data == Some(true);
            match s("path") {
                Some(path) => match app.services.write.as_mut() {
                    Some(w) => wrap(
                        w(path, &png)
                            .map(|_| with_data(json!({"path": path, "width": img.width, "height": img.height}), data.then_some(png.as_slice()))),
                    ),
                    None => err("no writer"),
                },
                None => ok(with_data(json!({"width": img.width, "height": img.height}), Some(&png))),
            }
        }
        "app.open" => match p.get("importer").filter(|v| !v.is_null()) {
            // The importer's parameters, else their defaults: automation gets no dialog asking for them.
            Some(importer) => {
                let params = p.get("params").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!({}));
                wrap(app.run("file.openWith", json!({"path": s("path"), "importer": importer, "params": params})))
            }
            // `params` go to the importer File › Open picks for the file, as in `document.open`.
            None => {
                let mut open = json!({"path": s("path")});
                if let Some(params) = p.get("params").filter(|v| !v.is_null()) {
                    open["params"] = params.clone();
                }
                wrap(app.run("file.open", open))
            }
        },
        "app.save" | "app.export" => {
            let mut p = if p.is_object() { p.clone() } else { json!({}) };
            // `wait: true`: the file is written before the reply, whatever Background Save and
            // Background Export are set to, so the reply is the written file or the error.
            let wait = match crate::menus::opt_bool(&p, "wait") {
                Ok(w) => w == Some(true),
                Err(e) => return err(e),
            };
            if let Some(o) = p.as_object_mut() {
                o.remove("wait");
            }
            let method = req.method.as_str();
            // The key check of `document.export` or `document.save` runs before the file is written.
            let checked = if method == "app.export" { "document.export" } else { "document.save" };
            if let Err(e) = crate::menus::check_params(&app.session, checked, &p) {
                return err(e);
            }
            wrap(if wait { crate::background::in_foreground(app, |app| save_or_export(app, method, &p)) } else { save_or_export(app, method, &p) })
        }
        // As the menu's Quit: `{"pending": "saveChanges"}` while a modified document is asked
        // about; otherwise null, and the host closes the window (#830).
        "app.quit" => wrap(app.run("app.quit", json!({}))),
        other => err(format!("unknown method `{other}`")),
    }
}

/// `app.save` (`method`) or `app.export` with params `p`.
fn save_or_export(app: &mut VectorcraftApp, method: &str, p: &Value) -> Result<Value, String> {
    if method == "app.save" {
        return app.run("file.save", p.clone());
    }
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    match s("path") {
        Some(path) => crate::io::export(app, s("format"), Some(path.to_string()), p),
        // No save dialog for an agent: the bytes come back, as in headless mode.
        None => app.run("document.export", p.clone()),
    }
}

/// `result` with the PNG as `pngBase64` when there is one.
fn with_data(mut result: Value, png: Option<&[u8]>) -> Value {
    if let Some(png) = png {
        result["pngBase64"] = json!(vectorcraft_format::base64_encode(png));
    }
    result
}

/// The largest image whose PNG goes back in a `ui.screenshot` reply (`data: true`): the reply
/// makes a straight-alpha copy, the encoded PNG and, for `data`, a base64 string (~1.33× the
/// PNG), so this bounds that working set before any of it is allocated. A path-only capture keeps
/// the shared 128 MiB image budget.
pub(crate) const MAX_SCREENSHOT_PNG_REPLY_BYTES: usize = 64 * 1024 * 1024;

/// The checked RGBA size of `w × h`, when it fits.
fn screenshot_rgba_bytes(w: usize, h: usize) -> Option<usize> {
    w.checked_mul(h).and_then(|n| n.checked_mul(4))
}

/// The reply to `ui.screenshot`: the window capture `image` written to `path`, and sent back as
/// `pngBase64` with `data`.
pub fn save_screenshot(app: &mut VectorcraftApp, image: &egui::ColorImage, path: Option<&str>, data: bool) -> Value {
    save_screenshot_with_limit(app, image, path, data, MAX_SCREENSHOT_PNG_REPLY_BYTES)
}

/// [`save_screenshot`] with the inline-PNG encoding limit supplied, so a small image can exercise
/// the over-limit path without allocating a multi-megapixel frame. The public function always uses
/// [`MAX_SCREENSHOT_PNG_REPLY_BYTES`].
pub(crate) fn save_screenshot_with_limit(
    app: &mut VectorcraftApp,
    image: &egui::ColorImage,
    path: Option<&str>,
    data: bool,
    max_reply_bytes: usize,
) -> Value {
    let [w, h] = image.size;
    if path.is_none() && !data {
        return json!({"ok": true, "result": {"width": w, "height": h}});
    }
    // A data-only reply has no second way to deliver the image, so refuse an oversized one before
    // allocating its working set. A request that named a path still writes that file: only the
    // inline copy is downgraded.
    if data && path.is_none() && screenshot_rgba_bytes(w, h).is_none_or(|bytes| bytes > max_reply_bytes) {
        return json!({"ok": false, "error": format!(
            "a PNG reply of this {w}×{h} screenshot would exceed the {max_reply_bytes}-byte reply-encoding limit; write it to a path instead"
        )});
    }
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let img = vectorcraft_render::Rendered { width: w as u32, height: h as u32, pixels: rgba };
    let png = match img.to_png() {
        Ok(png) => png,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let reply_png = data && png.len() <= max_reply_bytes;
    if data && !reply_png && path.is_none() {
        return json!({"ok": false, "error": format!(
            "this screenshot's PNG is {} bytes, over the {max_reply_bytes}-byte reply-encoding limit; write it to a path instead",
            png.len()
        )});
    }
    let data = reply_png.then_some(png.as_slice());
    let Some(path) = path else {
        return json!({"ok": true, "result": with_data(json!({"width": w, "height": h}), data)});
    };
    match app.services.write.as_mut() {
        Some(wr) => match wr(path, &png) {
            Ok(()) => json!({"ok": true, "result": with_data(json!({"path": path, "width": w, "height": h}), data)}),
            Err(e) => json!({"ok": false, "error": e}),
        },
        None => json!({"ok": false, "error": "no writer configured"}),
    }
}

#[cfg(test)]
mod tests;

/// The window and coordinate mapping where a control-channel Rotate View drag began.
#[derive(Clone, Copy)]
pub(crate) struct RotateViewPress {
    window: (u64, u32),
    document_resets: u64,
    xf: Xf,
    start_angle: f64,
    start_rotation: f64,
}

/// Rotate View belongs to the canvas, not the engine's placeholder tool. Keep its press mapping
/// across requests so document-coordinate drags do not move again as the view turns underneath.
fn rotate_view_pointer(
    app: &mut VectorcraftApp,
    kind: PointerKind,
    position: vectorcraft_geom::Point,
    screen: bool,
    shift: bool,
) -> Result<(), String> {
    let window = app.session.active().zip(crate::doc_windows::current(app)).map(|(d, w)| (d.uid, w.1)).ok_or("no document")?;
    let document_resets = app.session.tool_resets();
    let stored = if kind == PointerKind::Down { None } else { app.control_rotate_press };
    if kind == PointerKind::Drag && stored.is_none() {
        return Err("Rotate View needs a down event before a drag".into());
    }
    let mut press = match stored {
        Some(press) => press,
        None => {
            let rect = app.canvas_rect.ok_or("no canvas yet")?;
            let view = app.view().ok_or("no document")?;
            RotateViewPress { window, document_resets, xf: Xf::at(rect, view, app.canvas_ui_zoom), start_angle: 0.0, start_rotation: view.rotation }
        }
    };
    // A gesture cannot turn a different document/view or resume after its document was reset.
    if (press.window, press.document_resets) != (window, document_resets) {
        return Err("the Rotate View gesture's window or document changed".into());
    }
    let point = if screen { egui::pos2(position.x as f32, position.y as f32) } else { press.xf.to_screen(position) };
    let center = press.xf.rect.center();
    if !point.is_finite() || !center.is_finite() {
        return Err("Rotate View needs finite canvas coordinates".into());
    }
    match kind {
        PointerKind::Down => {
            press.start_angle = (point.y - center.y).atan2(point.x - center.x) as f64;
            app.control_rotate_press = Some(press);
        }
        PointerKind::Drag => {
            app.view_mut().ok_or("no document")?.rotation =
                crate::canvas::drag_rotation(center, point, press.start_angle, press.start_rotation, shift);
        }
        PointerKind::Up => {
            app.control_rotate_press = None;
        }
        PointerKind::Move | PointerKind::DoubleClick => {}
    }
    Ok(())
}

/// A Rotate View packet accepts only the documented boolean modifier fields.
fn validate_rotate_view_mods(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value.filter(|value| !value.is_null()) else { return Ok(()) };
    let Some(fields) = value.as_object() else { return Err("Rotate View `mods` must be an object of booleans".into()) };
    if fields.iter().any(|(key, value)| !matches!(key.as_str(), "shift" | "alt" | "cmd" | "ctrl" | "space") || !value.is_boolean()) {
        return Err("Rotate View `mods` accepts only boolean shift, alt, cmd, ctrl and space".into());
    }
    Ok(())
}

/// Reject malformed coordinates and modifiers before any event can change the angle or press.
fn validate_rotate_view_events(params: &Value, events: &[Value]) -> Result<(), String> {
    validate_rotate_view_mods(params.get("mods"))?;
    for event in events {
        for axis in ["x", "y"] {
            if event.get(axis).and_then(Value::as_f64).filter(|value| value.is_finite()).is_none() {
                return Err(format!("Rotate View `{axis}` must be a finite number"));
            }
        }
        validate_rotate_view_mods(event.get("mods"))?;
        if event.get("space").is_some_and(|space| !matches!(space.as_str(), Some("doc" | "screen"))) {
            return Err("Rotate View `space` must be doc or screen".into());
        }
    }
    Ok(())
}

/// `ui.pointer`: pointer events through the tools.
fn pointer_events(app: &mut VectorcraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let rotating = app.session.tool_id() == "rotateView";
    let before = if rotating { app.view().map(|view| view.rotation) } else { None };
    let outcome = pointer_events_inner(app, ctx, req);
    if rotating && matches!(&outcome, Outcome::Done(reply) if reply.get("ok").and_then(Value::as_bool) == Some(false)) {
        // A failed packet must neither leave a partial turn nor allow a later drag to resume it.
        app.control_rotate_press = None;
        if let (Some(before), Some(view)) = (before, app.view_mut()) {
            view.rotation = before;
        }
        ctx.request_repaint();
    }
    outcome
}

/// Apply a pointer packet after Rotate View validation; other tools retain their existing routing.
fn pointer_events_inner(app: &mut VectorcraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let Some(events) = p.get("events").and_then(Value::as_array) else { return err("missing `events`") };
    let rotating = app.session.tool_id() == "rotateView";
    if rotating && let Err(error) = validate_rotate_view_events(p, events) {
        return err(error);
    }
    // Every event is read before any is sent, so a malformed packet sends none of them.
    let read = packet_mods(p.get("mods")).and_then(|base| {
        let base = base.unwrap_or_default();
        events
            .iter()
            .map(|e| {
                let kind = match e.get("kind").and_then(Value::as_str).unwrap_or("") {
                    "down" => PointerKind::Down,
                    "drag" => PointerKind::Drag,
                    "up" => PointerKind::Up,
                    "move" => PointerKind::Move,
                    "doubleclick" | "dblclick" => PointerKind::DoubleClick,
                    other => return Err(format!("unknown pointer kind `{other}`")),
                };
                PointerEvent::json_check(e)?;
                let at = |k: &str| crate::menus::opt_number(e, k).map(|n| n.unwrap_or(0.0));
                Ok((kind, at("x")?, at("y")?, packet_mods(e.get("mods"))?.unwrap_or(base)))
            })
            .collect::<Result<Vec<_>, String>>()
    });
    let read = match read {
        Ok(read) => read,
        Err(e) => return err(e),
    };
    let view = app.view_info();
    let xf = app.canvas_xf();
    let screen = |e: &Value| e.get("space").and_then(Value::as_str) == Some("screen");
    if !rotating && xf.is_none() && events.iter().any(screen) {
        return err("no canvas yet");
    }
    let from = app.session.input_count();
    let held =
        PointerEvent::json_backquote(p, false).and_then(|d| events.iter().map(|e| PointerEvent::json_backquote(e, d)).collect::<Result<Vec<_>, _>>());
    let held = match held {
        Ok(held) => held,
        Err(e) => return err(e),
    };
    for ((e, backquote), (kind, x, y, mods)) in events.iter().zip(held).zip(read) {
        let screen = screen(e);
        if rotating {
            if let Err(e) = rotate_view_pointer(app, kind, vectorcraft_geom::Point::new(x, y), screen, mods.shift) {
                return err(e);
            }
            continue;
        }
        let pos = if screen {
            match xf {
                Some(xf) => xf.to_doc(egui::pos2(x as f32, y as f32)),
                None => return err("no canvas yet"),
            }
        } else {
            vectorcraft_geom::Point::new(x, y)
        };
        // A tilt given in screen axes leans the same way on the page as on the screen.
        let pen = PointerEvent::json_pen(e).map(|q| match xf {
            Some(xf) if screen => crate::canvas::pose_on_page(q, &xf),
            _ => q,
        });
        crate::canvas::dispatch(app, &PointerEvent { kind, pos, mods, backquote, pressure: PointerEvent::json_pressure(e), pen }, view);
        let hold = PointerEvent::json_hold(e);
        if hold > 0.0 {
            let r = app.session.tool_tick(hold, view);
            crate::canvas::apply_requests(app, r);
        }
    }
    ctx.request_repaint();
    // A Shape Builder merge of a large selection is computed on another thread, and the
    // reply to the events that started it waits for it without holding the frames up. A
    // gesture sent while it is computed is made after it, and its reply waits until then;
    // a hover is answered at once.
    let to = app.session.input_count();
    if !app.session.input_applied(from, to) {
        return Outcome::AfterToolEdit { from, to, document: app.session.active().map(|d| d.uid) };
    }
    Outcome::Done(pointer_reply(app))
}

/// `ui.click` and `ui.drag`: pointer input through egui itself.
fn synthetic_click(app: &mut VectorcraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    // Validate the entire expansion before appending even the initial move/press.
    let steps = if req.method == "ui.drag" {
        match input_count(p, "steps", 8, MAX_DRAG_STEPS) {
            Ok(n) => n,
            Err(e) => return err(e),
        }
    } else {
        0
    };
    let count = match input_count(p, "count", 1, MAX_CLICK_COUNT) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let Some(events) = count.checked_mul(2).and_then(|n| n.checked_add(steps)).and_then(|n| n.checked_add(1)) else {
        return err("synthetic input event count is out of range");
    };
    // Screen-space (egui points) pointer input through egui itself: reaches every widget.
    let drag = req.method == "ui.drag";
    let given = (|| {
        let a = egui::pos2(coord(p, "x", 0.0)?, coord(p, "y", 0.0)?);
        let end = if drag { egui::pos2(coord(p, "toX", 0.0)?, coord(p, "toY", 0.0)?) } else { a };
        Ok::<_, String>((a, end, modifiers(p)?))
    })();
    let (a, end, modifiers) = match given {
        Ok(given) => given,
        Err(e) => return err(e),
    };
    if let Err(e) = reserve_synthetic(app, events) {
        return err(e);
    }
    let button = match s("button") {
        Some("right") | Some("secondary") => egui::PointerButton::Secondary,
        _ => egui::PointerButton::Primary,
    };
    app.synthetic.push(egui::Event::PointerMoved(a));
    app.synthetic.push(egui::Event::PointerButton { pos: a, button, pressed: true, modifiers });
    if req.method == "ui.drag" {
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            app.synthetic.push(egui::Event::PointerMoved(a + (end - a) * t));
        }
    }
    app.synthetic.push(egui::Event::PointerButton { pos: end, button, pressed: false, modifiers });
    for _ in 1..count {
        app.synthetic.push(egui::Event::PointerButton { pos: end, button, pressed: true, modifiers });
        app.synthetic.push(egui::Event::PointerButton { pos: end, button, pressed: false, modifiers });
    }
    ctx.request_repaint();
    ok(Value::Null)
}

/// `ui.set`: view flags, panels and status.
fn set_ui(app: &mut VectorcraftApp, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    // A flag of the wrong type is refused before anything is set.
    let flags = ["rulers", "outline", "grid", "smartGuides", "boundingBox", "controlBar", "imageTracePreview"];
    if let Some(e) = flags.iter().find_map(|k| crate::menus::opt_bool(p, k).err()) {
        return err(e);
    }
    // Outline is a setting of the active document's view: without one, nothing is set.
    if p.get("outline").is_some_and(Value::is_boolean) && app.session.active().is_none() {
        return err("no document");
    }
    let mut r = Ok(Value::Null);
    if let Some(b) = s("brightness") {
        r = app.run("window.brightness", json!({"brightness": b}));
    }
    if let Some(pn) = s("panel") {
        r = app.run("window.panel", json!({"panel": pn}));
    }
    if let Some(st) = s("status") {
        app.ui.status = st.to_string();
    }
    for (k, flag) in [
        ("rulers", "view.rulers"),
        ("outline", "view.outline"),
        ("grid", "view.grid"),
        ("smartGuides", "view.smartGuides"),
        ("boundingBox", "view.boundingBox"),
    ] {
        if let Some(want) = p.get(k).and_then(Value::as_bool) {
            let cur = match k {
                "rulers" => app.ui.view.rulers,
                "outline" => app.outline_view(),
                "grid" => app.ui.view.grid,
                "smartGuides" => app.ui.view.smart_guides,
                _ => app.ui.view.bounding_box,
            };
            if cur != want {
                r = app.run(flag, json!({}));
            }
        }
    }
    if let Some(v) = p.get("controlBar").and_then(Value::as_bool) {
        app.ui.control_bar = v;
    }
    if let Some(v) = p.get("imageTracePreview").and_then(Value::as_bool) {
        r = app.run("imageTrace.preview", json!({"on": v}));
    }
    if p.get("closePanel").is_some() {
        app.ui.open_panel = None;
    }
    wrap(r)
}
