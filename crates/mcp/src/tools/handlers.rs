use super::dispatch::trace_views;
use super::*;

// ---------- implementation ----------

pub(crate) type Args = Map<String, Value>;

pub(super) fn req_str<'a>(a: &'a Args, k: &str) -> Result<&'a str, String> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing string argument `{k}`"))
}

/// A file tool's path made absolute against this process's working directory when it goes to the app
/// (`--connect`). Headless, and for an empty path, it is returned as given.
pub(super) fn absolute_path(b: &dyn Backend, p: &str) -> Result<String, String> {
    if p.is_empty() || !b.has_ui() {
        return Ok(p.to_owned());
    }
    let abs = std::path::absolute(p).map_err(|e| format!("{p}: {e}"))?;
    abs.into_os_string().into_string().map_err(|abs| format!("{p}: the absolute path {} is not UTF-8", abs.to_string_lossy()))
}

pub(super) fn exec(b: &mut dyn Backend, cmd: &str, params: Value) -> Result<Value, String> {
    b.call("engine.execute", json!({"command": cmd, "params": params}))
}

pub(super) fn need_ui(b: &dyn Backend, tool: &str) -> Result<(), String> {
    if b.has_ui() {
        Ok(())
    } else {
        Err(format!(
            "`{tool}` needs the desktop app: run `vectorcraft --control 7979` and start the server with `vectorcraft-cli mcp --connect 127.0.0.1:7979`"
        ))
    }
}

/// Paint argument → `paint.setFill` / `paint.setStroke` params.
fn paint_params(v: &Value) -> Result<Value, String> {
    Ok(match v {
        Value::Null => json!({"none": true}),
        Value::String(s) if s.eq_ignore_ascii_case("none") => json!({"none": true}),
        Value::String(_) | Value::Array(_) => json!({"color": v}),
        Value::Object(o) if ["color", "none", "swatch", "gradient"].iter().any(|k| o.contains_key(*k)) => v.clone(),
        Value::Object(_) => json!({"color": v}),
        other => return Err(format!("unsupported paint value {other}")),
    })
}

/// A command a drawing tool runs and its params.
type Step<'a> = (&'a str, Value);

/// The commands that apply `a`'s `fill`, `stroke`, `strokeWidth`, `dash`, `cap` and `join`, without
/// targets, and what each applies.
fn paint_steps(a: &Args) -> Result<(Vec<Step<'static>>, Map<String, Value>), String> {
    let (mut steps, mut applied) = (vec![], Map::new());
    if let Some(f) = a.get("fill") {
        steps.push(("paint.setFill", paint_params(f)?));
        applied.insert("fill".into(), f.clone());
    }
    if let Some(s) = a.get("stroke") {
        steps.push(("paint.setStroke", paint_params(s)?));
        applied.insert("stroke".into(), s.clone());
    }
    // Weight, dash, cap and join go to one stroke.set.
    let mut stroke = Map::new();
    if let Some(w) = a.get("strokeWidth") {
        let w = w.as_f64().ok_or("`strokeWidth` must be a number")?;
        stroke.insert("weight".into(), json!(w));
        applied.insert("strokeWidth".into(), json!(w));
    }
    for k in ["dash", "cap", "join"] {
        if let Some(v) = a.get(k) {
            stroke.insert(k.into(), v.clone());
            applied.insert(k.into(), v.clone());
        }
    }
    if !stroke.is_empty() {
        steps.push(("stroke.set", Value::Object(stroke)));
    }
    Ok((steps, applied))
}

/// Sets the paint `a` gives on `ids` (default: the selection) and as the default for new art.
/// Returns the paint applied and the most objects one of its edits painted (the `objects` of the
/// `paint.setFill`, `paint.setStroke` and `stroke.set` replies).
pub(super) fn apply_paint(b: &mut dyn Backend, a: &Args, ids: Option<Value>) -> Result<(Value, u64), String> {
    let (steps, applied) = paint_steps(a)?;
    let mut objects = 0;
    for (cmd, mut p) in steps {
        if let Some(ids) = &ids {
            p["ids"] = ids.clone();
        }
        let r = exec(b, cmd, p)?;
        objects = objects.max(r.get("objects").and_then(Value::as_u64).unwrap_or(0));
    }
    Ok((Value::Object(applied), objects))
}

/// Checks `draw_path` / `draw_shape`'s `dash`, `cap`, `join`, `opacity`, `name` and `effects`
/// before anything is drawn, so a bad one leaves no half-styled object behind (#2112).
fn check_draw_options(b: &mut dyn Backend, a: &Args) -> Result<(), String> {
    match a.get("dash") {
        None | Some(Value::Null) => {}
        Some(Value::Array(d)) if d.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)) => {}
        Some(_) => return Err("`dash` must be a list of lengths (numbers) or null".into()),
    }
    let one_of = |k: &str, names: &[&str]| match a.get(k) {
        None => Ok(()),
        Some(v) if v.as_str().is_some_and(|s| names.contains(&s)) => Ok(()),
        Some(_) => Err(format!("`{k}` must be one of {}", names.join(", "))),
    };
    one_of("cap", &["butt", "round", "square", "projecting"])?;
    one_of("join", &["miter", "round", "bevel"])?;
    if let Some(o) = a.get("opacity")
        && !o.as_f64().is_some_and(|o| (0.0..=100.0).contains(&o))
    {
        return Err("`opacity` must be a number from 0 to 100 (%)".into());
    }
    if a.get("name").is_some_and(|n| !n.is_string()) {
        return Err("`name` must be a string".into());
    }
    let Some(effects) = a.get("effects") else { return Ok(()) };
    let effects = effects.as_array().ok_or("`effects` must be a list of {effect, params?}")?;
    if effects.is_empty() {
        return Ok(());
    }
    let catalog = exec(b, "effect.list", json!({}))?;
    let known: Vec<&str> = catalog.get("catalog").and_then(Value::as_array).into_iter().flatten().filter_map(|e| e.get("id")?.as_str()).collect();
    for (i, e) in effects.iter().enumerate() {
        let e = e.as_object().ok_or_else(|| format!("effect {i} must be {{effect, params?}}"))?;
        if let Some(k) = e.keys().find(|k| *k != "effect" && *k != "params") {
            return Err(format!("effect {i}: unknown key `{k}` (expected effect, params)"));
        }
        let id = e.get("effect").and_then(Value::as_str).ok_or_else(|| format!("effect {i}: missing string `effect`"))?;
        if !known.contains(&id) {
            return Err(format!("effect {i}: unknown effect `{id}` (see apply_effect without arguments)"));
        }
        if e.get("params").is_some_and(|p| !p.is_object()) {
            return Err(format!("effect {i}: `params` must be an object"));
        }
    }
    Ok(())
}

/// Draws an object with `create` (a creating command and its params) and styles it with `a`'s
/// paint and draw options ([`check_draw_options`], checked first) as ONE `command.batch`, named
/// `label` (the undo step the creating command makes on its own): any step the engine refuses — a
/// colour it can't read, a swatch the document lacks — rolls the whole call back, so it leaves no
/// object and no undo step behind (#3545). The styling steps act on the new object, which the
/// creating command selects, and on its whole appearance (`item: null`) → `made` with the new
/// object's `id`, the `paint` applied and the other options applied.
fn draw(b: &mut dyn Backend, a: &Args, label: &str, create: Step<'_>, mut made: Value) -> Result<Value, String> {
    check_draw_options(b, a)?;
    let (paint, applied) = paint_steps(a)?;
    let mut steps = vec![create];
    steps.extend(paint);
    let mut props = json!({});
    for k in ["name", "opacity"] {
        if let Some(v) = a.get(k) {
            props[k] = v.clone();
            made[k] = v.clone();
        }
    }
    if a.contains_key("name") || a.contains_key("opacity") {
        steps.push(("object.setProps", props));
    }
    // The fill, stroke and effects go to the new object's whole appearance (item: null), as the
    // Fill and Stroke proxies and the Effect menu apply them to a selected object.
    let effects = a.get("effects").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    for e in effects {
        steps.push(("effect.apply", json!({"effect": e.get("effect"), "params": e.get("params").cloned().unwrap_or(json!({}))})));
    }
    let commands: Vec<Value> = steps
        .into_iter()
        .map(|(cmd, mut p)| {
            if matches!(cmd, "paint.setFill" | "paint.setStroke" | "stroke.set" | "effect.apply") {
                p["item"] = Value::Null;
            }
            json!({"command": cmd, "params": p})
        })
        .collect();
    // The engine names the failed step of its batch; the caller sent no batch, so the reply gives
    // the step's own error.
    let r = exec(b, "command.batch", json!({"label": label, "commands": commands})).map_err(|e| match e.split_once(" failed: ") {
        Some((step, error)) if step.starts_with("batch step ") => error.to_string(),
        _ => e,
    })?;
    let results = r.get("results").and_then(Value::as_array).ok_or("command.batch returned no results")?;
    made["id"] = results.first().and_then(|r| r.get("id")).cloned().unwrap_or(Value::Null);
    made["paint"] = Value::Object(applied);
    if !effects.is_empty() {
        made["effects"] = Value::Array(results.iter().skip(results.len().saturating_sub(effects.len())).cloned().collect());
    }
    Ok(made)
}

pub(super) fn draw_path(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let mut p = json!({});
    if let Some(d) = a.get("d").and_then(Value::as_str) {
        p["d"] = json!(d);
    } else {
        let pts = a.get("points").and_then(Value::as_array).ok_or("give `points` or `d`")?;
        let mut anchors = vec![];
        for (i, pt) in pts.iter().enumerate() {
            let anchor = match pt {
                Value::Array(xy) if xy.len() >= 2 && xy[0].is_number() && xy[1].is_number() => json!({"x": xy[0], "y": xy[1]}),
                Value::Object(o) if o.get("x").is_some_and(Value::is_number) && o.get("y").is_some_and(Value::is_number) => pt.clone(),
                _ => return Err(format!("point {i} must be [x,y] or {{x,y,…}}")),
            };
            anchors.push(anchor);
        }
        if anchors.is_empty() {
            return Err("`points` is empty".into());
        }
        p["anchors"] = Value::Array(anchors);
        p["closed"] = json!(a.get("closed").and_then(Value::as_bool).unwrap_or(false));
    }
    draw(b, a, "Pen", ("path.create", p), json!({}))
}

pub(super) fn draw_shape(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let shape = req_str(a, "shape")?;
    // Each command and the name of the undo step it makes.
    let (cmd, label) = match shape {
        "rectangle" | "rect" if a.get("radius").and_then(Value::as_f64).is_some_and(|r| r > 0.0) => ("shape.rectangle", "Rounded Rectangle"),
        "rectangle" | "rect" => ("shape.rectangle", "Rectangle"),
        "ellipse" | "circle" => ("shape.ellipse", "Ellipse"),
        "polygon" => ("shape.polygon", "Polygon"),
        "star" => ("shape.star", "Star"),
        "line" => ("shape.line", "Line"),
        other => return Err(format!("unknown shape `{other}` (rectangle, ellipse, polygon, star, line)")),
    };
    let mut p = a.clone();
    for k in ["shape", "fill", "stroke", "strokeWidth", "dash", "cap", "join", "opacity", "name", "effects"] {
        p.remove(k);
    }
    draw(b, a, label, (cmd, Value::Object(p)), json!({"shape": shape}))
}

/// Select `ids` when given (tools that act on the selection).
pub(super) fn select_ids(b: &mut dyn Backend, a: &Args) -> Result<(), String> {
    if let Some(ids) = a.get("ids") {
        exec(b, "select.set", json!({"ids": ids}))?;
    }
    Ok(())
}

pub(super) fn add_text(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let text = a.get("text").and_then(Value::as_str).ok_or("missing string argument `text`")?;
    let mut style = json!({});
    for k in ["size", "font", "color", "justify", "vertical"] {
        if let Some(v) = a.get(k) {
            style[k] = v.clone();
        }
    }
    let r = if let Some(path) = a.get("path") {
        let mut p = json!({"path": path, "mode": a.get("mode").cloned().unwrap_or(json!("onPath")), "text": text});
        for (k, v) in style.as_object().into_iter().flatten() {
            p[k.as_str()] = v.clone();
        }
        exec(b, "text.createInPath", p)?
    } else {
        let x = a.get("x").and_then(Value::as_f64).ok_or("give `x` and `y` (or a `path`)")?;
        let y = a.get("y").and_then(Value::as_f64).ok_or("give `x` and `y` (or a `path`)")?;
        let mut p = json!({"x": x, "y": y, "text": text});
        for (k, v) in style.as_object().into_iter().flatten() {
            p[k.as_str()] = v.clone();
        }
        if let (Some(w), Some(h)) = (a.get("width"), a.get("height")) {
            p["area"] = json!({"width": w, "height": h});
            for k in ["fit", "fitMinPercent"] {
                if let Some(v) = a.get(k) {
                    p["area"][k] = v.clone();
                }
            }
        }
        exec(b, "text.create", p)?
    };
    if let Some(e) = a.get("pathEffect").and_then(Value::as_str) {
        exec(b, "type.pathOptions", json!({"effect": e}))?;
    }
    let mut made = json!({"id": r.get("id")});
    // A path effect is applied after the text is made: ask again how it fits then.
    let overflow = match r.get("id").filter(|_| a.get("pathEffect").is_some()) {
        Some(id) => exec(b, "document.node", json!({"id": id, "summary": true}))?.get("overflow").and_then(Value::as_bool) == Some(true),
        None => r.get("overflow").is_some(),
    };
    if overflow {
        made["overflow"] = json!(true);
    }
    Ok(made)
}

pub(super) fn transform(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    select_ids(b, a)?;
    // With `copy`, the first operation duplicates and the rest transform the copy (the new selection).
    let copy = std::cell::Cell::new(a.get("copy").and_then(Value::as_bool).unwrap_or(false));
    let with = |mut p: Value| {
        if let Some(o) = a.get("origin") {
            p["origin"] = o.clone();
        }
        p["copy"] = json!(copy.replace(false));
        p
    };
    let mut done = vec![];
    if a.contains_key("dx") || a.contains_key("dy") {
        let num = |k: &str| a.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        exec(b, "object.move", json!({"dx": num("dx"), "dy": num("dy"), "copy": copy.replace(false)}))?;
        done.push("move");
    }
    if let Some(r) = a.get("rotate").and_then(Value::as_f64) {
        exec(b, "object.rotate", with(json!({"angle": r})))?;
        done.push("rotate");
    }
    let s = a.get("scale").and_then(Value::as_f64);
    let (sx, sy) = (a.get("scaleX").and_then(Value::as_f64).or(s), a.get("scaleY").and_then(Value::as_f64).or(s));
    if sx.is_some() || sy.is_some() {
        exec(b, "object.scale", with(json!({"sx": sx.unwrap_or(100.0), "sy": sy.unwrap_or(100.0)})))?;
        done.push("scale");
    }
    if let Some(axis) = a.get("reflect").and_then(Value::as_str) {
        exec(b, "object.reflect", with(json!({"axis": axis})))?;
        done.push("reflect");
    }
    if let Some(sh) = a.get("shear").and_then(Value::as_f64) {
        exec(b, "object.shear", with(json!({"angle": sh})))?;
        done.push("shear");
    }
    if done.is_empty() {
        return Err("give at least one of dx/dy, rotate, scale/scaleX/scaleY, reflect, shear".into());
    }
    Ok(json!({"applied": done}))
}

/// Most fields one `answer_dialog` call may set.
pub(super) const MAX_DIALOG_FIELDS: usize = 256;

/// Sets the open dialog's fields in order with one `ui.dialog.set {fields}`, which stores all of
/// them or none, then confirms or cancels it. Every argument is checked before anything is sent to
/// the app.
pub(super) fn answer_dialog(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let set = match a.get("set") {
        None => &[][..],
        Some(Value::Array(s)) => s.as_slice(),
        Some(_) => return Err("`set` must be a list of {field, value?}".into()),
    };
    if set.len() > MAX_DIALOG_FIELDS {
        return Err(format!("`set` holds {} fields; at most {MAX_DIALOG_FIELDS}", set.len()));
    }
    let mut fields = vec![];
    for (i, item) in set.iter().enumerate() {
        let item = item.as_object().ok_or_else(|| format!("set {i} must be {{field, value?}}"))?;
        if let Some(k) = item.keys().find(|k| *k != "field" && *k != "value") {
            return Err(format!("set {i}: unknown key `{k}` (expected field, value)"));
        }
        let field = item.get("field").and_then(Value::as_str).ok_or_else(|| format!("set {i}: missing string `field`"))?;
        fields.push(json!({"field": field, "value": item.get("value").cloned().unwrap_or(Value::Null)}));
    }
    let then = match a.get("action") {
        None => None,
        Some(v) if v == "confirm" => Some("ui.dialog.confirm"),
        Some(v) if v == "cancel" => Some("ui.dialog.cancel"),
        Some(_) => return Err("`action` must be confirm or cancel".into()),
    };
    if fields.is_empty() && then.is_none() {
        return Err("give `set` and/or `action`".into());
    }
    let dialog = if fields.is_empty() { Value::Null } else { b.call("ui.dialog.set", json!({"fields": fields}))? };
    let result = match then {
        Some(method) => b.call(method, json!({}))?,
        None => Value::Null,
    };
    Ok(json!({"dialog": dialog, "result": result}))
}

pub(super) fn screenshot(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let path = a.get("path").and_then(Value::as_str).map(|p| absolute_path(b, p)).transpose()?;
    let path = path.as_deref();
    let window = a.get("window").and_then(Value::as_bool) == Some(true);
    let max = max_side(a)?;
    if max.is_some() && !window {
        return Err("`max_side` applies to a window capture (`window: true`); size an artboard render with `scale`".into());
    }
    let trace_views = trace_views(a)?;
    // The backend writes the file, where its automation roots allow, and sends the image back too
    // (`data`): this process writes nothing.
    let mut p = json!({"data": true});
    if let Some(path) = path {
        p["path"] = json!(path);
    }
    let r = if window {
        need_ui(b, "screenshot {window:true}")?;
        b.call("ui.screenshot", p)?
    } else {
        for k in ["scale", "artboard"] {
            if let Some(v) = a.get(k) {
                p[k] = v.clone();
            }
        }
        if trace_views {
            p["traceViews"] = json!(true);
        }
        b.call("ui.render", p)?
    };
    let png = match r.get("pngBase64").and_then(Value::as_str) {
        Some(b64) => vectorcraft_format::base64_decode(b64).ok_or("the backend returned bad base64")?,
        // An app from before `data` wrote the file without sending the image: read it back
        // (loopback: the same machine).
        None => match path {
            Some(path) => std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?,
            None if window => return old_window_capture(b, max),
            None => return Err("the renderer returned no image".into()),
        },
    };
    if window {
        let mut info = json!({"window": true, "width": r.get("width"), "height": r.get("height"), "path": path});
        let png = fit_capture(png, max, &mut info)?;
        Ok(image_result(&png, info))
    } else {
        Ok(image_result(&png, json!({"width": r.get("width"), "height": r.get("height"), "path": path})))
    }
}

/// The window of an app from before `ui.screenshot {data}`, captured through a temporary file.
fn old_window_capture(b: &mut dyn Backend, max: Option<u32>) -> Result<ToolResult, String> {
    let tmp = std::env::temp_dir().join(format!("vectorcraft-window-{}.png", std::process::id())).to_string_lossy().to_string();
    let r = b.call("ui.screenshot", json!({"path": tmp}))?;
    let png = std::fs::read(&tmp).map_err(|e| format!("read {tmp}: {e}"))?;
    // Best effort: a temporary file left behind is harmless.
    std::fs::remove_file(&tmp).ok();
    let mut info = json!({"window": true, "width": r.get("width"), "height": r.get("height"), "path": Value::Null});
    let png = fit_capture(png, max, &mut info)?;
    Ok(image_result(&png, info))
}

/// `max_side`: absent, or an integer in 1..4096.
pub(super) fn max_side(a: &Args) -> Result<Option<u32>, String> {
    match a.get("max_side") {
        None => Ok(None),
        Some(v) => Ok(Some(
            v.as_u64().filter(|v| (1..=4096).contains(v)).and_then(|v| u32::try_from(v).ok()).ok_or("`max_side` must be an integer in 1..4096")?,
        )),
    }
}

/// A window capture with `max_side`: scaled down when its longer side is larger, with its own size
/// in `capturedWidth` / `capturedHeight` and the returned image's in `width` / `height`. Without
/// `max_side` the capture and `info` stay as they are.
fn fit_capture(png: Vec<u8>, max: Option<u32>, info: &mut Value) -> Result<Vec<u8>, String> {
    let Some(max) = max else { return Ok(png) };
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|e| format!("read capture: {e}"))?;
    info["capturedWidth"] = json!(img.width());
    info["capturedHeight"] = json!(img.height());
    if img.width().max(img.height()) <= max {
        info["width"] = json!(img.width());
        info["height"] = json!(img.height());
        return Ok(png);
    }
    let img = img.thumbnail(max, max);
    info["width"] = json!(img.width());
    info["height"] = json!(img.height());
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("encode capture: {e}"))?;
    Ok(out.into_inner())
}

pub(super) fn image_result(png: &[u8], info: Value) -> ToolResult {
    ToolResult {
        content: vec![
            json!({"type": "image", "data": vectorcraft_format::base64_encode(png), "mimeType": "image/png"}),
            json!({"type": "text", "text": info.to_string()}),
        ],
        is_error: false,
    }
}
