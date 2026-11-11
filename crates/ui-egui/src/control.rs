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
        leave_out_unreadable_paths(fields);
    }
    let mut inspected = json!({
        "scriptRunning": app.session.script_running(),
        "scriptReport": app.session.script_report(),
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
    });
    let ids = unreadable_library_ids(app);
    for field in PANEL_FIELDS.iter().chain(&LIBRARY_ID_FIELDS) {
        if let Some(v) = inspected["ui"].get_mut(*field) {
            hide_library_ids(v, &ids);
        }
    }
    let pairs: Vec<_> = ids.iter().map(|(id, shown)| (id.as_str(), shown.as_str())).collect();
    if let Some(sets) = inspected["ui"].get_mut("action_sets") {
        swap_library_params(sets, &pairs);
    }
    inspected
}

/// The fields of the UI state, by their keys in `ui.inspect`'s `ui`, that hold library ids or panel
/// ids besides [`PANEL_FIELDS`]: the library panels, the Color Guide's limit and the workspaces.
const LIBRARY_ID_FIELDS: [&str; 4] = ["library_panels", "color_guide_limit", "custom_workspaces", "workspace_changes"];

/// The fields of the UI state, by their keys in `ui.inspect`'s `ui`, that hold panel ids: the panel
/// popped out of the collapsed column, the docking layout, closed panels' places and groups, the panels
/// shown as icons or collapsed, and the older floating panels' record.
const PANEL_FIELDS: [&str; 7] =
    ["open_panel", "docking", "docking_hidden", "docking_hidden_groups", "docking_icons", "docking_collapsed", "floating_panels"];

/// The ids of the loaded libraries (`loaded/<path>`) that the UI state refers to and whose file the
/// request may not read, each with the id a reply shows instead: `loaded/<file name>`, as a library
/// loaded without a path is named. The UI state refers to a library by its bare id in a library panel
/// and the Color Guide's limit, by a panel id (`library:<kind>:loaded/<path>`) in the fields of
/// [`PANEL_FIELDS`] and the workspaces' layouts, and by either in the `library` and `panel` params of
/// recorded actions' steps. Text the person types (a command palette query, a workspace's or an
/// action's name) is not read. When another of those ids, or a library loaded in the session, already
/// has that id, the next free one
/// of `loaded/<file name> (2)`, `(3)` and so on is shown, in the sorted order of the full ids, so that
/// two libraries stay two and `ui.inspect` and `ui.docking` show a library under the same id.
fn unreadable_library_ids(app: &VectorcraftApp) -> Vec<(String, String)> {
    let mut ids = std::collections::BTreeSet::new();
    let ui = serde_json::to_value(&app.ui).unwrap_or_default();
    for field in PANEL_FIELDS {
        collect_library_panel_ids(&ui[field], &mut ids);
    }
    collect_library_params(&ui["action_sets"], &mut ids);
    for workspace in ["custom_workspaces", "workspace_changes"].iter().filter_map(|k| ui[k].as_array()).flatten() {
        for (_, layout) in workspace.as_object().into_iter().flatten().filter(|(k, _)| k.as_str() != "name") {
            collect_library_panel_ids(layout, &mut ids);
        }
    }
    collect_library_panel_ids(&json!(app.ui.transient_panel), &mut ids);
    for id in app.ui.library_panels.iter().map(|p| p.id.as_str()).chain([app.ui.color_guide_limit.as_str()]) {
        add_library_id(id, &mut ids);
    }
    let mut hidden = Vec::new();
    // The ids of the libraries loaded in the session, so that a hidden one does not take the id of
    // one loaded without a path.
    let s = &app.session;
    let loaded = s.swatch_libraries.infos().chain(s.style_libraries.infos()).chain(s.symbol_libraries.infos()).chain(s.brush_libraries.infos());
    let mut taken: std::collections::BTreeSet<String> = loaded.map(|info| info.id.clone()).collect();
    for id in ids {
        let path = &id["loaded/".len()..];
        // Either separator: a path saved on another platform counts as a path here too.
        let name = path.rsplit(['/', '\\']).find(|part| !part.is_empty()).unwrap_or(path).to_string();
        if name != path && !readable(path) {
            hidden.push((id, name));
        } else {
            taken.insert(id);
        }
    }
    let mut hidden = hidden
        .into_iter()
        .map(|(id, name)| {
            let mut shown = format!("loaded/{name}");
            let mut n = 2;
            while taken.contains(&shown) {
                shown = format!("loaded/{name} ({n})");
                n += 1;
            }
            taken.insert(shown.clone());
            (id, shown)
        })
        .collect::<Vec<_>>();
    // The longest first, so that an id that begins another one does not replace part of it.
    hidden.sort_by_key(|(id, _)| std::cmp::Reverse(id.len()));
    hidden
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

/// Writes each id of `ids` as the id the reply shows instead, wherever `v` holds it: in a library's
/// id, in its panel's id (`library:swatches:loaded/<path>`) and in keys.
fn hide_library_ids(v: &mut Value, ids: &[(String, String)]) {
    let pairs: Vec<_> = ids.iter().map(|(id, shown)| (id.as_str(), shown.as_str())).collect();
    swap_library_ids(v, &pairs);
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
/// file or a listed script outside it.
fn readable(path: &str) -> bool {
    vectorcraft_engine::file_access::check_read(path).is_ok()
}

/// Does a recorded step's param `key` hold a file or folder path: `path`, `folder` or a name ending in
/// `Folder` (`destinationFolder`, `searchFolder`)?
fn is_path_param(key: &str) -> bool {
    matches!(key, "path" | "folder") || key.ends_with("Folder")
}

/// The params of each step of each action of action set `set` (`ui.action_sets`).
fn steps_mut(set: &mut Value) -> impl Iterator<Item = &mut serde_json::Map<String, Value>> {
    let actions = set.get_mut("actions").and_then(Value::as_array_mut).into_iter().flatten();
    let steps = actions.filter_map(|action| action.get_mut("steps").and_then(Value::as_array_mut)).flatten();
    steps.filter_map(|step| step.get_mut(1).and_then(Value::as_object_mut))
}

/// Takes out of a recorded step's params, at any depth (the commands of a `command.batch` step), the
/// paths the request may not read: a path param ([`is_path_param`]) and each path of a `paths` list.
fn leave_out_unreadable_params(params: &mut serde_json::Map<String, Value>) {
    params.retain(|key, v| !(is_path_param(key) && v.as_str().is_some_and(|p| !readable(p))));
    if let Some(Value::Array(paths)) = params.get_mut("paths") {
        paths.retain(|p| p.as_str().is_none_or(readable));
    }
    for v in params.values_mut() {
        leave_out_unreadable_values(v);
    }
}

/// [`leave_out_unreadable_params`] on each object `v` holds.
fn leave_out_unreadable_values(v: &mut Value) {
    match v {
        Value::Object(fields) => leave_out_unreadable_params(fields),
        Value::Array(items) => items.iter_mut().for_each(leave_out_unreadable_values),
        _ => {}
    }
}

/// Takes out of `ui.inspect`'s copy of the UI state the paths the request may not read: recent files,
/// a library panel's file, folder preferences (saved, and in an open Preferences dialog) and the paths
/// in recorded actions' steps outside the read root. The UI state itself is not changed.
fn leave_out_unreadable_paths(ui: &mut serde_json::Map<String, Value>) {
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
}

/// Takes out of `prefs` the folder preferences ([`PREF_FOLDERS`]) the request may not read. An
/// empty value (no folder) and a scratch disk choice ([`SCRATCH_CHOICES`]) stay.
fn leave_out_unreadable_folders(prefs: &mut serde_json::Map<String, Value>) {
    for key in PREF_FOLDERS {
        let Some(written) = prefs.get(key).and_then(Value::as_str) else { continue };
        let path = written.trim();
        if path.is_empty() || (key.starts_with("scratch") && SCRATCH_CHOICES.contains(&path)) || readable(written) {
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
fn modifiers(p: &Value) -> egui::Modifiers {
    let b = |n: &str| p.get(n).and_then(Value::as_bool).unwrap_or(false);
    egui::Modifiers { alt: b("alt"), ctrl: b("ctrl"), shift: b("shift"), mac_cmd: b("cmd") && cfg!(target_os = "macos"), command: b("cmd") }
}

/// The params that name a library or a library panel, at any depth of a request (`ui.set {panel}`,
/// `engine.execute {params: {panel}}`, `ui.libraryPanel {panel, library}`), with the docking
/// commands' destinations (`window.panel.move {anchor, before}`, `window.panel.float {onto, above,
/// below}`) and a list of panels.
const LIBRARY_PARAMS: [&str; 8] = ["panel", "library", "anchor", "before", "onto", "above", "below", "panels"];

/// The param of `window.panel.layout` that holds a serialized docking action: each panel id in it is
/// translated.
const DOCKING_ACTION: &str = "action";

/// Runs a control request. A library loaded from outside the read root keeps the id `ui.inspect`
/// shows for it ([`unreadable_library_ids`]) both ways: a request may name it or its panel by that id
/// in a [`LIBRARY_PARAMS`] param, and the reply to such a request shows that id.
pub fn handle(app: &mut VectorcraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let ids = if names_a_loaded_library(&req.params) { unreadable_library_ids(app) } else { Vec::new() };
    let shown: Vec<_> = ids.iter().map(|(id, shown)| (shown.as_str(), id.as_str())).collect();
    let mut params = req.params.clone();
    if shown.is_empty() || !swap_library_params(&mut params, &shown) {
        return dispatch(app, ctx, req);
    }
    let named = ControlRequest { method: req.method.clone(), params, reply: req.reply.clone(), state: req.state.clone(), seq: req.seq };
    match dispatch(app, ctx, &named) {
        Outcome::Done(mut reply) => {
            hide_library_ids(&mut reply, &ids);
            Outcome::Done(reply)
        }
        outcome => outcome,
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
        "ui.docking" => {
            let mut docking = crate::panel_docking::inspect(app, ctx);
            let ids = unreadable_library_ids(app);
            hide_library_ids(&mut docking, &ids);
            ok(docking)
        }
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
            if let Err(e) = reserve_synthetic(app, 2 + usize::from(s("text").is_some())) {
                return err(e);
            }
            let m = modifiers(p);
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
            if let Err(e) = reserve_synthetic(app, 1) {
                return err(e);
            }
            let pos = egui::pos2(p.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32, p.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32);
            app.synthetic.push(egui::Event::PointerMoved(pos));
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.click" | "ui.drag" => synthetic_click(app, ctx, req),
        "ui.wheel" => {
            if let Err(e) = reserve_synthetic(app, 2) {
                return err(e);
            }
            // Mouse wheel notches over screen point (x, y): `dy` up (+) or down, `dx` sideways
            // (`unit: "point"` for trackpad-like points), with modifiers as `ui.click` takes them.
            let f = |k: &str| p.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let unit = if s("unit") == Some("point") { egui::MouseWheelUnit::Point } else { egui::MouseWheelUnit::Line };
            // Notches, not a flood: a turn stays within what a real wheel sends.
            let delta = egui::vec2(f("dx"), f("dy")).clamp(egui::Vec2::splat(-100.0), egui::Vec2::splat(100.0));
            app.synthetic.push(egui::Event::PointerMoved(egui::pos2(f("x"), f("y"))));
            app.synthetic.push(egui::Event::MouseWheel {
                unit,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers: modifiers(p),
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
            let w = p.get("width").and_then(Value::as_f64).unwrap_or(1440.0) as f32;
            let h = p.get("height").and_then(Value::as_f64).unwrap_or(900.0) as f32;
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
            if p.get("focus").and_then(Value::as_bool).unwrap_or(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            ctx.request_repaint();
            Outcome::Screenshot { path: s("path").map(str::to_string), data: p.get("data").and_then(Value::as_bool) == Some(true) }
        }
        "ui.render" => {
            let Some(st) = app.session.active() else { return err("no document") };
            let doc = st.doc.clone();
            // The artboard asked for (0-based, as headless), else the first.
            let index = p.get("artboard").and_then(Value::as_u64).map_or(Some(0), |i| usize::try_from(i).ok());
            let Some(r) = index.and_then(|i| doc.artboards.get(i)).map(|a| a.rect) else { return err("no such artboard") };
            let scale = p.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
            if let Err(e) = vectorcraft_render::raster_size(r, scale) {
                return err(e);
            }
            // Rendered with the options of a PNG export on white, so its pixels are the exported file's.
            // `traceViews: true` draws Image Trace objects as their View shows them, as the canvas does.
            let mut opts = RasterExportOptions { background: Some([255; 3]), ..Default::default() }.render_options(RasterFormat::Png);
            opts.trace_views = p.get("traceViews").and_then(Value::as_bool) == Some(true);
            let img = app.canvas.renderer.render_region_with(&doc, r, scale, &opts);
            let png = match img.to_png() {
                Ok(png) => png,
                Err(e) => return err(e),
            };
            let data = p.get("data").and_then(Value::as_bool) == Some(true);
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
            RotateViewPress { window, document_resets, xf: Xf::new(rect, view), start_angle: 0.0, start_rotation: view.rotation }
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
    let Some(value) = value else { return Ok(()) };
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
    let base_mods: Mods = p.get("mods").and_then(|m| serde_json::from_value(m.clone()).ok()).unwrap_or_default();
    let view = app.view_info();
    let xf = app.canvas_rect.zip(app.view().copied()).map(|(rect, v)| Xf::new(rect, &v));
    let from = app.session.input_count();
    let held =
        PointerEvent::json_backquote(p, false).and_then(|d| events.iter().map(|e| PointerEvent::json_backquote(e, d)).collect::<Result<Vec<_>, _>>());
    let held = match held {
        Ok(held) => held,
        Err(e) => return err(e),
    };
    for (e, backquote) in events.iter().zip(held) {
        let kind = match e.get("kind").and_then(Value::as_str).unwrap_or("") {
            "down" => PointerKind::Down,
            "drag" => PointerKind::Drag,
            "up" => PointerKind::Up,
            "move" => PointerKind::Move,
            "doubleclick" | "dblclick" => PointerKind::DoubleClick,
            other => return err(format!("unknown pointer kind `{other}`")),
        };
        let x = e.get("x").and_then(Value::as_f64).unwrap_or(0.0);
        let y = e.get("y").and_then(Value::as_f64).unwrap_or(0.0);
        let screen = e.get("space").and_then(Value::as_str) == Some("screen");
        let mods = e.get("mods").and_then(|m| serde_json::from_value(m.clone()).ok()).unwrap_or(base_mods);
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
    if let Err(e) = reserve_synthetic(app, events) {
        return err(e);
    }
    // Screen-space (egui points) pointer input through egui itself: reaches every widget.
    let f = |k: &str| p.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let button = match s("button") {
        Some("right") | Some("secondary") => egui::PointerButton::Secondary,
        _ => egui::PointerButton::Primary,
    };
    let modifiers = modifiers(p);
    let a = egui::pos2(f("x"), f("y"));
    let end = if req.method == "ui.drag" { egui::pos2(f("toX"), f("toY")) } else { a };
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
