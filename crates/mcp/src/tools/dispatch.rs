use super::definitions::{INSPECT_KEYS, SAVE_FILE_OPTIONS, tool_definitions};
use super::handlers::{
    Args, absolute_path, add_text, answer_dialog, apply_paint, draw_path, draw_shape, exec, image_result, max_side, need_ui, req_str, screenshot,
    select_ids, transform,
};
use super::{ARTBOARD_PARAMS, Backend, Map, ToolResult, Value, format, json, save_reads_option, writable_format};

/// The `menu` argument of the command listings.
pub(super) const MENU_FILTER: &str =
    "Case-insensitive substring of the menu path, its levels joined with \" > \" (e.g. \"Object\", \"Object > Path\")";

/// The `brief` argument of the command listings.
pub(super) const BRIEF: &str =
    "Leave out each command's params description (the bulk of the catalogue); run command_list with the id as filter to read it";

/// The `max_side` argument of the window captures.
pub(super) const MAX_SIDE_WINDOW: &str =
    "Window capture only: longest side of the returned image in pixels; a larger capture is scaled down to it (default: the full capture)";

/// The `traceViews` argument of the artboard renders.
pub(super) const TRACE_VIEWS: &str = "Draw Image Trace objects in the view set with imageTrace.setView (outlines, the source image…), as the app's window does; by default they draw their tracing result, as exports do";

/// `traceViews`: false when absent.
pub(super) fn trace_views(a: &Args) -> Result<bool, String> {
    match a.get("traceViews") {
        None => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| "`traceViews` must be a boolean".into()),
    }
}

/// The command catalogue narrowed by the listing tools' arguments: `filter` (or `query`) on the
/// id, label and menu path, `menu` on the menu path, `enabledOnly`, and `brief` (no `params`
/// text: the full catalogue is too large for many clients, #2173).
fn filter_commands(all: Value, a: &Args) -> Value {
    let text = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_lowercase).filter(|s| !s.is_empty());
    let needle = text("filter").or_else(|| text("query"));
    let in_menu = text("menu");
    let enabled_only = a.get("enabledOnly").and_then(Value::as_bool).unwrap_or(false);
    let brief = a.get("brief").and_then(Value::as_bool).unwrap_or(false);
    let Value::Array(list) = all else { return all };
    let menu_path = |c: &Value, sep: &str| {
        c.get("menu")
            .and_then(Value::as_array)
            .map(|m| m.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(sep))
            .unwrap_or_default()
            .to_lowercase()
    };
    let hit = |c: &Value| {
        if enabled_only && c.get("enabled").and_then(Value::as_bool) == Some(false) {
            return false;
        }
        if let Some(m) = &in_menu
            && !menu_path(c, " > ").contains(m.as_str())
        {
            return false;
        }
        let Some(n) = &needle else { return true };
        let field = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_lowercase();
        field("id").contains(n.as_str()) || field("label").contains(n.as_str()) || menu_path(c, " ").contains(n.as_str())
    };
    Value::Array(
        list.into_iter()
            .filter(hit)
            .map(|mut c| {
                if brief && let Some(o) = c.as_object_mut() {
                    o.remove("params");
                }
                c
            })
            .collect(),
    )
}

fn dispatch(b: &mut dyn Backend, name: &str, a: &Args) -> Result<ToolResult, String> {
    let j = |v: Value| Ok(ToolResult::json(&v));
    match name {
        "command_list" => {
            let all = b.call("engine.commands", json!({}))?;
            let mut filters = a.clone();
            if let Some(v) = filters.remove("enabled_only") {
                filters.insert("enabledOnly".into(), v);
            }
            j(filter_commands(all, &filters))
        }
        "command_batch" => command_batch(b, a),
        "render_preview" => render_preview(b, a),
        "ui_screenshot" => {
            let mut args = a.clone();
            args.insert("window".into(), json!(true));
            screenshot(b, &args)
        }
        "list_commands" => {
            let all = b.call("engine.commands", json!({}))?;
            let list = filter_commands(all, a);
            j(json!({"count": list.as_array().map_or(0, Vec::len), "commands": list}))
        }
        "command_run" | "run_command" | "invoke_menu" => {
            let cmd = req_str(a, if name == "command_run" { "id" } else { "command" })?;
            let params = match a.get("params") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(_) => return Err("`params` must be an object".into()),
            };
            let method = if name == "invoke_menu" && b.has_ui() { "ui.menu.invoke" } else { "engine.execute" };
            j(b.call(method, json!({"command": cmd, "params": params}))?)
        }
        "doc_inspect" | "inspect_document" => {
            // Only the slicing and scope options reach the command, which validates them.
            let slice: Map<String, Value> =
                a.iter().filter(|(k, _)| INSPECT_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
            j(b.call("document.inspect", Value::Object(slice))?)
        }
        "ui_inspect" | "inspect_ui" => {
            need_ui(b, name)?;
            j(b.call("ui.inspect", json!({}))?)
        }
        "select_tool" => j(b.call("ui.tool.select", json!({"tool": req_str(a, "tool")?}))?),
        "pointer_gesture" => {
            let events = a.get("events").and_then(Value::as_array).filter(|e| !e.is_empty()).ok_or("`events` must be a non-empty array")?;
            for (i, e) in events.iter().enumerate() {
                let kind = e.get("kind").and_then(Value::as_str).unwrap_or("");
                if crate::headless::pointer_kind(kind).is_none() {
                    return Err(format!("event {i}: unknown kind `{kind}` (down, drag, up, move, doubleclick)"));
                }
                if !e.get("x").is_some_and(Value::is_number) || !e.get("y").is_some_and(Value::is_number) {
                    return Err(format!("event {i}: numeric `x` and `y` required"));
                }
            }
            if let Some(t) = a.get("tool").and_then(Value::as_str) {
                b.call("ui.tool.select", json!({"tool": t}))?;
            }
            let events: Vec<Value> = events
                .iter()
                .map(|e| {
                    let mut e = e.clone();
                    e["space"] = json!("doc");
                    e
                })
                .collect();
            let mut p = json!({"events": events});
            if let Some(m) = a.get("mods") {
                p["mods"] = m.clone();
            }
            j(b.call("ui.pointer", p)?)
        }
        "draw_path" => j(draw_path(b, a)?),
        "draw_shape" => j(draw_shape(b, a)?),
        "set_paint" => {
            if !["fill", "stroke", "strokeWidth"].iter().any(|k| a.contains_key(*k)) {
                return Err("give at least one of `fill`, `stroke`, `strokeWidth`".into());
            }
            let (applied, objects) = apply_paint(b, a, a.get("ids").cloned())?;
            let mut reply = json!({"applied": applied, "objects": objects});
            if objects == 0 {
                reply["note"] = json!(
                    "no object was painted (nothing selected, or `ids` is empty or holds no object this paint applies to): only the paint for new art changed"
                );
            }
            j(reply)
        }
        "press_key" => {
            let mut p = json!({"key": req_str(a, "key")?});
            if let Some(Value::Object(m)) = a.get("mods") {
                for (k, v) in m {
                    p[k.as_str()] = v.clone();
                }
            }
            j(b.call("ui.key", p)?)
        }
        "type_text" => {
            need_ui(b, name)?;
            j(b.call("ui.text", json!({"text": req_str(a, "text")?}))?)
        }
        "answer_dialog" => {
            need_ui(b, name)?;
            j(answer_dialog(b, a)?)
        }
        "open_panel" => {
            need_ui(b, name)?;
            j(b.call("ui.set", json!({"panel": req_str(a, "panel")?}))?)
        }
        "screenshot" => screenshot(b, a),
        "open_file" => {
            let path = absolute_path(b, req_str(a, "path")?)?;
            j(b.call("app.open", json!({"path": path}))?)
        }
        "save_file" => {
            let mut p = json!({"path": a.get("path").and_then(Value::as_str).map(|p| absolute_path(b, p)).transpose()?});
            for k in SAVE_FILE_OPTIONS {
                if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
                    p[k] = v.clone();
                }
            }
            // The app writes the file before it replies, even with Background Save on, so a failed
            // save is this call's error.
            if b.has_ui() {
                p["wait"] = json!(true);
            }
            let mut r = b.call("app.save", p)?;
            let notes = save_file_warnings(a, r.get("format").and_then(Value::as_str));
            if let Some(o) = r.as_object_mut().filter(|_| !notes.is_empty()) {
                let mut all = o.get("warnings").and_then(Value::as_array).cloned().unwrap_or_default();
                all.extend(notes.into_iter().map(Value::from));
                o.insert("warnings".into(), Value::Array(all));
            }
            j(r)
        }
        "export" => {
            // One params object for every backend; the named arguments win over `options` (a named
            // artboard or range replaces every artboard choice in `options`).
            let mut params = a.get("options").and_then(Value::as_object).cloned().unwrap_or_default();
            if ["artboard", "range"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_null())) {
                for k in ARTBOARD_PARAMS {
                    params.remove(k);
                }
            }
            for k in named_export_params(a) {
                if let Some(v) = a.get(*k).filter(|v| !v.is_null()) {
                    params.insert((*k).into(), v.clone());
                }
            }
            if let Some(path) = params.get("path").and_then(Value::as_str) {
                let path = absolute_path(b, path)?;
                params.insert("path".into(), json!(path));
            }
            let cmd = if a.get("selection").and_then(Value::as_bool) == Some(true) { "document.exportSelection" } else { "document.export" };
            j(exec(b, cmd, Value::Object(params))?)
        }
        "add_text" => j(add_text(b, a)?),
        "apply_effect" => {
            let Some(effect) = a.get("effect").and_then(Value::as_str) else {
                let all = exec(b, "effect.list", json!({}))?;
                return j(json!({"catalog": all.get("catalog")}));
            };
            let mut p = json!({"effect": effect, "params": a.get("params").cloned().unwrap_or(json!({}))});
            if let Some(ids) = a.get("ids") {
                p["ids"] = ids.clone();
            }
            j(exec(b, "effect.apply", p)?)
        }
        "pathfinder" => {
            select_ids(b, a)?;
            j(exec(b, &format!("object.pathfinder.{}", req_str(a, "operation")?), json!({}))?)
        }
        "transform" => j(transform(b, a)?),
        "create_graph" => {
            let mut p = Value::Object(a.clone());
            if p.get("type").is_none() {
                p["type"] = json!("column");
            }
            j(exec(b, "graph.create", p)?)
        }
        "text_wrap" => {
            select_ids(b, a)?;
            if a.get("release").and_then(Value::as_bool) == Some(true) {
                return j(exec(b, "object.textWrap.release", json!({}))?);
            }
            let mut p = json!({});
            for k in ["offset", "invert"] {
                if let Some(v) = a.get(k) {
                    p[k] = v.clone();
                }
            }
            j(exec(b, "object.textWrap.make", p)?)
        }
        "undo" => j(exec(b, "edit.undo", json!({}))?),
        "redo" => j(exec(b, "edit.redo", json!({}))?),
        other => Err(format!("unknown tool `{other}` (see tools/list)")),
    }
}

/// The named `export` arguments passed on to `document.export`. `scale` is raster pixels per
/// point, so only a raster format gets it: DXF's own (drawing units per unit, given in `options`)
/// would resize the drawing and leave its units unset.
fn named_export_params(a: &Args) -> &'static [&'static str] {
    let format = writable_format(a.get("format").and_then(Value::as_str), a.get("path").and_then(Value::as_str));
    if format.is_ok_and(|f| f.raster) {
        &["path", "format", "scale", "artboard", "range", "outlineText"]
    } else {
        &["path", "format", "artboard", "range", "outlineText"]
    }
}

/// What `save_file` with arguments `a` adds to the save's `warnings`, for a file written in format
/// `written` (none when nothing was written yet): each key in `options` that the format doesn't
/// read. The save itself warns about `compress`, `version` and `preview` given at the top level.
fn save_file_warnings(a: &Args, written: Option<&str>) -> Vec<String> {
    let Some(f) = written.and_then(format) else { return vec![] };
    let nested = a.get("options").and_then(Value::as_object);
    nested
        .into_iter()
        .flatten()
        .filter(|(key, v)| !v.is_null() && !save_reads_option(f, key))
        .map(|(key, _)| format!("`options.{key}` was not used: {} files don't read it", f.label))
        .collect()
}

/// Run one tool. Failures (unknown tool, bad arguments, command errors) come back as an
/// `isError` result so the model can read and correct them.
pub fn call_tool(b: &mut dyn Backend, name: &str, args: &Value) -> ToolResult {
    let empty = Map::new();
    let a = match args {
        Value::Object(o) => o,
        Value::Null => &empty,
        _ => return ToolResult::error("tool arguments must be a JSON object"),
    };
    match vectorcraft_engine::guard::catch_panic(|| dispatch(b, name, a)) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => ToolResult::error(format!("{name}: {e}")),
        Err(e) => ToolResult::error(format!("internal error in `{name}`: {e} (please report this bug)")),
    }
}

/// Most steps one `command_batch` may hold, and most bytes its kept step results may total.
/// Batches run sequentially and every result is kept for later steps' `"$N.path"` references, so
/// without caps one batch could do unbounded work and pile up an unbounded reply (#1727).
pub(crate) const MAX_COMMAND_BATCH_STEPS: usize = 256;

pub(crate) const MAX_COMMAND_BATCH_BYTES: usize = 8 * 1024 * 1024;

/// Refuses an argument the tool's schema doesn't list, as `tools/call` does (#713).
pub(crate) fn check_tool_arguments(name: &str, args: &Args) -> Result<(), String> {
    let defs = tool_definitions();
    let props =
        defs.iter().find(|t| t.get("name").and_then(Value::as_str) == Some(name)).and_then(|t| t.get("inputSchema")?.get("properties")?.as_object());
    match props.and_then(|p| args.keys().find(|k| !p.contains_key(*k))) {
        Some(key) => Err(format!("unknown argument `{key}` for `{name}`")),
        None => Ok(()),
    }
}

fn command_batch(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let steps = a.get("steps").and_then(Value::as_array).ok_or("`steps` must be an array")?;
    if steps.len() > MAX_COMMAND_BATCH_STEPS {
        return Err(format!("`steps` holds {} steps; at most {MAX_COMMAND_BATCH_STEPS}", steps.len()));
    }
    let stop = match a.get("stop_on_error") {
        None => true,
        Some(v) => v.as_bool().ok_or("`stop_on_error` must be a boolean")?,
    };
    let atomic = match a.get("atomic") {
        None => false,
        Some(v) => v.as_bool().ok_or("`atomic` must be a boolean")?,
    };
    if atomic {
        let label = a.get("label").cloned().unwrap_or(Value::Null);
        b.call("engine.execute", json!({"command":"edit.undoGroup.begin","params":{"label":label}}))?;
    } else if a.get("label").is_some() {
        return Err("`label` names the undo step of an atomic batch: give atomic:true too".into());
    }
    let (mut completed, mut failed, mut results) = (0, 0, Vec::new());
    // Each step's result (null for a failed one), for later steps' `"$N.path"` references.
    let mut values: Vec<Value> = vec![];
    // The kept results all go back in the reply, so their total size is capped: one batch can't
    // pile up an unbounded answer (#1727).
    let mut kept_bytes = 0usize;
    // The objects the drawing steps made (#2113).
    let mut created: Vec<Value> = vec![];
    for step in steps {
        let result = vectorcraft_engine::guard::catch_panic(|| {
            let args = step.as_object().ok_or("each step must be an object")?;
            if args.keys().any(|k| k != "id" && k != "params") {
                return Err("unknown step argument (expected id, params)".into());
            }
            let id = req_str(args, "id")?;
            let params = match args.get("params") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => vectorcraft_engine::steps::resolve(v, &values)?,
                Some(_) => return Err("`params` must be an object".into()),
            };
            match id {
                // The drawing tools, with the arguments `tools/call` takes (#2113).
                "draw_path" | "draw_shape" => {
                    let args = params.as_object().cloned().unwrap_or_default();
                    check_tool_arguments(id, &args)?;
                    let made = if id == "draw_path" { draw_path(b, &args)? } else { draw_shape(b, &args)? };
                    if let Some(new) = made.get("id").filter(|v| v.is_u64()) {
                        created.push(new.clone());
                    }
                    Ok(made)
                }
                _ => b.call("engine.execute", json!({"command":id,"params":params})),
            }
        })
        .unwrap_or_else(|e| Err(format!("internal error: {e}")));
        match result {
            Ok(v) => {
                let entry = json!({"ok": true, "result": v});
                let size = entry.to_string().len();
                if kept_bytes.saturating_add(size) > MAX_COMMAND_BATCH_BYTES {
                    // The results already kept stay; this step ran, and its result is replaced by
                    // the reason the batch stops (saying it ran, so nobody runs it again). The loop breaks either way — carrying on would defeat the cap —
                    // and the atomic group below still closes, as for any stopped batch.
                    failed += 1;
                    values.push(Value::Null);
                    results.push(json!({"ok": false, "error": format!("this step ran, but the batch's results exceed {MAX_COMMAND_BATCH_BYTES} bytes: its result is dropped and the batch stops here")}));
                    break;
                }
                kept_bytes += size;
                completed += 1;
                values.push(entry["result"].clone());
                results.push(entry);
            }
            Err(e) => {
                failed += 1;
                values.push(Value::Null);
                results.push(json!({"ok":false,"error":e}));
                if stop {
                    break;
                }
            }
        }
    }
    // The steps that ran stay, as one undo step.
    if atomic {
        b.call("engine.execute", json!({"command":"edit.undoGroup.end","params":{}}))?;
    }
    let mut r = ToolResult::json(&json!({"completed":completed,"failed":failed,"results":results,"created":created}));
    r.is_error = failed > 0;
    Ok(r)
}

fn render_preview(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let max = max_side(a)?.unwrap_or(1024);
    let index = match a.get("artboard") {
        None => 0,
        Some(v) => v.as_u64().ok_or("`artboard` must be a 0-based artboard index")?,
    };
    let trace_views = trace_views(a)?;
    let doc = b.call("document.inspect", json!({"depth":0}))?;
    let board =
        doc.get("artboards").and_then(Value::as_array).and_then(|v| v.get(usize::try_from(index).ok()?)).ok_or(format!("no artboard {index}"))?;
    let width = board.get("width").and_then(Value::as_f64).ok_or("artboard has no width")?;
    let height = board.get("height").and_then(Value::as_f64).ok_or("artboard has no height")?;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("invalid artboard dimensions".into());
    }
    // The headless renderer clamps scale at 0.01. Bound its allocation even for
    // a huge artboard, then downsize to the requested dimensions.
    let scale = (f64::from(max) / width.max(height)).clamp(0.01, 1.0);
    if width.max(height) * scale > 4096.0 {
        return Err("artboard is too large for a bounded preview".into());
    }
    let mut p = json!({"scale":scale, "artboard": index});
    if trace_views {
        p["traceViews"] = json!(true);
    }
    let r = b.call("ui.render", p)?;
    let encoded = r.get("pngBase64").and_then(Value::as_str).ok_or("renderer returned no image")?;
    let png = vectorcraft_format::base64_decode(encoded).ok_or("renderer returned bad base64")?;
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|e| format!("read preview: {e}"))?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("encode preview: {e}"))?;
    Ok(image_result(out.get_ref(), json!({"width":img.width(),"height":img.height()})))
}
