//! MCP tool definitions (names, descriptions, JSON Schemas) and their implementations on top of a
//! [`Backend`].

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{ARTBOARD_PARAMS, FORMATS, OPEN_EXTS, SAVE_FORMATS, format};

use crate::backend::Backend;

/// The result of `tools/call`: MCP content blocks plus the `isError` flag.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: false }
    }
    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(v).unwrap_or_default())
    }
    pub fn error(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: true }
    }
    pub fn to_value(&self) -> Value {
        json!({"content": self.content, "isError": self.is_error})
    }
}

// ---------- schemas ----------

fn num(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}
fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}
fn paint_schema(what: &str) -> Value {
    json!({
        "description": format!("{what}: \"#rrggbb\", \"none\", [r,g,b] (0..1), {{\"c\",\"m\",\"y\",\"k\"}}, {{\"gray\"}}, or a full paint.setFill params object ({{\"gradient\":…}} / {{\"swatch\":name}})"),
        "anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "number"}}, {"type": "object"}]
    })
}
fn mods_schema() -> Value {
    json!({
        "type": "object",
        "description": "Modifier keys held (cmd = Command on macOS / Ctrl elsewhere; alt = Option)",
        "properties": {"shift": {"type": "boolean"}, "alt": {"type": "boolean"}, "cmd": {"type": "boolean"}, "ctrl": {"type": "boolean"}},
        "additionalProperties": false
    })
}
fn obj(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

/// The extension of each format Save writes, in [`SAVE_FORMATS`] order.
fn save_extensions() -> Vec<&'static str> {
    SAVE_FORMATS.iter().filter_map(|id| format(id)?.extensions.first().copied()).collect()
}

fn tool(name: &str, title: &str, desc: &str, schema: Value, read_only: bool) -> Value {
    let writes_file = matches!(name, "screenshot" | "save_file" | "export");
    let read_only = read_only && !writes_file;
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": !read_only, "idempotentHint": read_only, "openWorldHint": false},
    })
}

/// All tools, in `tools/list` order.
pub fn tool_definitions() -> Vec<Value> {
    let empty = || obj(json!({}), &[]);
    vec![
        tool(
            "command_list",
            "List commands",
            "Discover command ids, labels, parameter descriptions and enabled state. Returns an array.",
            obj(json!({"filter": string("Case-insensitive id, label or menu substring"), "enabled_only": {"type":"boolean"}}), &[]),
            true,
        ),
        tool(
            "command_run",
            "Run a command",
            "Run an engine command by id with its JSON params. Command parameters follow the registry's descriptions.",
            obj(json!({"id": string("Command id"), "params": {"type":"object"}}), &["id"]),
            false,
        ),
        tool(
            "command_batch",
            "Run several commands",
            "Run steps in order; each edit is its own undo step. A param string that is exactly \"$N.path\" (N: 0-based step index) or \"$last.path\" takes that value from an earlier step's result, e.g. {\"id\":\"$1.id\"}; \"$$\" starts a literal \"$\". Returns completed, failed and results. Stops on the first error by default.",
            obj(
                json!({"steps": {"type":"array", "items":obj(json!({"id":string("Command id"),"params":{"type":"object"}}), &["id"])}, "stop_on_error":{"type":"boolean", "default":true}}),
                &["steps"],
            ),
            false,
        ),
        tool(
            "doc_inspect",
            "Inspect document",
            "Summary of the active document, including artboards, layer tree, selection and history.",
            obj(json!({"depth":{"type":"integer", "minimum":0},"childLimit":{"type":"integer", "minimum":0}}), &[]),
            true,
        ),
        tool(
            "render_preview",
            "Render a preview",
            "Render the first artboard as inline PNG without changing the document. max_side defaults to 1024, limited to 1..4096.",
            obj(json!({"max_side":{"type":"integer","minimum":1,"maximum":4096}}), &[]),
            true,
        ),
        tool("ui_inspect", "Inspect the interface", "Inspect the connected desktop interface; returns a tool error in headless mode.", empty(), true),
        tool(
            "ui_screenshot",
            "Capture the interface",
            "Capture the connected desktop window as inline PNG; requires a running desktop app.",
            empty(),
            true,
        ),
        tool(
            "list_commands",
            "List commands",
            "List VectorCraft commands (id, label, menu path, shortcut, parameter description, enabled state). Every editing action is a command; run any of them with run_command. Coordinates are points in document space, y down, origin at the first artboard's top-left.",
            obj(
                json!({"filter": string("Case-insensitive substring matched against id, label and menu path (e.g. \"align\", \"Object\")"), "enabledOnly": {"type": "boolean", "description": "Only commands that can run right now"}}),
                &[],
            ),
            true,
        ),
        tool(
            "run_command",
            "Run command",
            "Execute any VectorCraft command by id with JSON params (see list_commands for ids and params). Examples: {\"command\":\"object.group\"}, {\"command\":\"object.align\",\"params\":{\"align\":\"left\"}}, {\"command\":\"file.new\",\"params\":{\"width\":800,\"height\":600}}. Returns the command's result (e.g. {id} for creation commands).",
            obj(
                json!({"command": string("Command id, e.g. shape.rectangle, object.group, paint.setFill"), "params": {"type": "object", "description": "Command parameters"}}),
                &["command"],
            ),
            false,
        ),
        tool(
            "inspect_document",
            "Inspect document",
            "Summary of the active document: artboards, layer tree (ids, names, kinds, bounds, fill/stroke), selection, undo history, current tool and default paint. On a large document pass depth: 0 for the layer skeleton (truncated levels report childCount), then locate objects with run_command document.find and read them with document.node.",
            obj(
                json!({
                    "depth": {"type": "integer", "minimum": 0, "description": "Child levels of the layer tree to include (default all; 0 = top layers with counts)"},
                    "childLimit": {"type": "integer", "minimum": 0, "description": "Children shown per node (default all; a level that shows fewer reports childCount)"}
                }),
                &[],
            ),
            true,
        ),
        tool(
            "inspect_ui",
            "Inspect UI",
            "UI state of the running desktop app: tool and options, panels, dialogs, view (zoom/center), window, documents, perf. Desktop app only.",
            empty(),
            true,
        ),
        tool(
            "select_tool",
            "Select tool",
            "Switch the active tool (as clicking it in the Tools panel). Ids: selection, directSelection, groupSelection, pen, rectangle, roundedRectangle, ellipse, polygon, star, lineSegment, …",
            obj(json!({"tool": string("Tool id")}), &["tool"]),
            false,
        ),
        tool(
            "pointer_gesture",
            "Pointer gesture",
            "Drive the active tool with mouse events in document coordinates, exactly like the mouse would (e.g. a rectangle drag: down at (100,100), drag to (200,150), up at (200,150)). Optionally selects `tool` first.",
            obj(
                json!({
                    "tool": string("Optional tool to select before the gesture"),
                    "events": {
                        "type": "array",
                        "minItems": 1,
                        "items": obj(json!({
                            "kind": {"type": "string", "enum": ["down", "drag", "up", "move", "doubleclick"]},
                            "x": num("Document x (pt)"),
                            "y": num("Document y (pt)"),
                            "mods": mods_schema(),
                            "pressure": num("Pen pressure 0..1 (default 1): the Liquify tools' intensity with Use Pressure Pen on"),
                            "holdMs": num("Milliseconds the pointer then holds still, button down (0..60000): Twirl, Pucker and Bloat keep applying"),
                        }), &["kind", "x", "y"]),
                    },
                    "mods": mods_schema(),
                }),
                &["events"],
            ),
            false,
        ),
        tool(
            "draw_path",
            "Draw path",
            "Create a path from points ([[x,y],…] or [{x,y,in?:[x,y],out?:[x,y]},…] with Bézier handles) or SVG path data `d`, then apply optional paint. Returns the new object id.",
            obj(
                json!({
                    "points": {"type": "array", "description": "Anchors: [x,y] pairs or {x, y, in?, out?, smooth?} objects", "items": {"anyOf": [{"type": "array", "items": {"type": "number"}, "minItems": 2}, {"type": "object"}]}},
                    "d": string("SVG path data (alternative to points), e.g. \"M0 0 L100 0 L50 80 Z\""),
                    "closed": {"type": "boolean", "description": "Close the path (points only)"},
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                }),
                &[],
            ),
            false,
        ),
        tool(
            "draw_shape",
            "Draw shape",
            "Create a live shape and apply optional paint. rectangle/ellipse: x, y, width, height (rectangle: radius = corner radius). polygon: cx, cy, radius, sides. star: cx, cy, radius1, radius2, points. line: x1, y1, x2, y2. Returns the new object id.",
            obj(
                json!({
                    "shape": {"type": "string", "enum": ["rectangle", "ellipse", "polygon", "star", "line"]},
                    "x": num("Left (rectangle/ellipse)"),
                    "y": num("Top (rectangle/ellipse)"),
                    "width": num("Width (rectangle/ellipse)"),
                    "height": num("Height (rectangle/ellipse)"),
                    "radius": num("Corner radius (rectangle) or radius (polygon)"),
                    "cx": num("Centre x (polygon/star)"),
                    "cy": num("Centre y (polygon/star)"),
                    "sides": {"type": "integer", "minimum": 3, "description": "Polygon sides (default 6)"},
                    "radius1": num("Star outer radius"),
                    "radius2": num("Star inner radius"),
                    "points": {"type": "integer", "minimum": 3, "description": "Star points (default 5)"},
                    "rotation": num("Rotation in degrees (polygon/star)"),
                    "x1": num("Line start x"),
                    "y1": num("Line start y"),
                    "x2": num("Line end x"),
                    "y2": num("Line end y"),
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                }),
                &["shape"],
            ),
            false,
        ),
        tool(
            "set_paint",
            "Set paint",
            "Set fill, stroke and/or stroke weight on the selection (or `ids`) and as the default for new art.",
            obj(
                json!({
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                    "ids": {"type": "array", "items": {"type": "integer"}, "description": "Object ids (default: the selection)"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "press_key",
            "Press key",
            "Send a key press with modifiers, e.g. {key:\"Z\", mods:{cmd:true}} (undo), {key:\"Escape\"}, {key:\"Enter\"}. In the desktop app it goes through the real keyboard path; headless, it runs the command or tool bound to that shortcut.",
            obj(
                json!({"key": string("Key name: A–Z, 0–9, Enter, Escape, Delete, Backspace, Tab, Space, Left/Right/Up/Down, [ ]…"), "mods": mods_schema()}),
                &["key"],
            ),
            false,
        ),
        tool(
            "type_text",
            "Type text",
            "Type text into the focused widget (text tool, dialog field). Desktop app only.",
            obj(json!({"text": string("Text to type")}), &["text"]),
            false,
        ),
        tool(
            "invoke_menu",
            "Invoke menu",
            "Invoke a menu item by its command id (same ids as list_commands, including UI-only commands like view.zoomIn or window.* in the desktop app).",
            obj(json!({"command": string("Menu command id"), "params": {"type": "object"}}), &["command"]),
            false,
        ),
        tool(
            "open_panel",
            "Open panel",
            "Open a panel in the desktop app's dock by id (layers, swatches, stroke, align, pathfinder, transform, …; case-insensitive, display labels like \"Layers\" work too). Desktop app only.",
            obj(json!({"panel": string("Panel id or display label")}), &["panel"]),
            false,
        ),
        tool(
            "screenshot",
            "Screenshot",
            "Render the active artboard to PNG and return it as an image (optionally also saving it to `path`). With window:true (desktop app only) captures the whole app window instead.",
            obj(
                json!({
                    "path": string("Also write the PNG here"),
                    "scale": num("Pixels per point (default 1)"),
                    "artboard": {"type": "integer", "minimum": 0, "description": "Artboard index (default 0)"},
                    "window": {"type": "boolean", "description": "Capture the app window (desktop app only)"},
                }),
                &[],
            ),
            true,
        ),
        tool(
            "open_file",
            "Open file",
            &format!(
                "Open a file as a new, active document: .{} (see run_command document.formats). Templates (.ait, native templates) open as a new untitled document. The reply's `warnings` say what didn't come in as it was (an EPS shown as its preview image says why its PostScript couldn't be read).",
                OPEN_EXTS.join(", .")
            ),
            obj(json!({"path": string("File path")}), &["path"]),
            false,
        ),
        tool(
            "save_file",
            "Save file",
            &format!(
                "Save the active document with the engine's document.save: by default to its own file in its own format (native .vectorcraft unless it was opened from or saved as SVG, PDF or a restorable .ai). With `path`, the extension picks the format: .{}. Other formats are exports (see export). The reply's `format` says what was written and `warnings` what that format loses.",
                save_extensions().join(", .")
            ),
            obj(json!({"path": string("Destination; its extension picks the format (default: the document's own file)")}), &[]),
            false,
        ),
        tool(
            "export",
            "Export",
            "Export the active document with the engine's document.export (the same bytes in the app and headless), in any format of the `format` enum: vectors (svg/svgz with the artboard as viewBox, eps, dxf, emf, wmf), pdf (one page per artboard: all, or `artboard` / `range`), rasters (one rendered artboard: png, jpg, webp, gif, png8 as an indexed .png, tiff, bmp, tga, psd with its layers), txt (the stories, back to front), vectorcraft or template (a native copy or template; the document keeps its path). run_command document.formats lists each format's options. `selection: true` exports only the selected objects, cropped to their bounds. Template layers are left out; live effects are kept (geometry baked, SVG filters for shadows/glows/blur). Without `path` the bytes come back as dataBase64.",
            obj(
                json!({
                    "format": {"type": "string", "enum": FORMATS.iter().filter(|f| f.write).map(|f| f.id).collect::<Vec<_>>(), "description": "Default: from the path's extension"},
                    "path": string("Destination file (omit to get dataBase64)"),
                    "scale": num("Raster pixels per point (default 1)"),
                    "artboard": {"type": "integer", "minimum": 0, "description": "0-based artboard (default: the first; PDF default: all)"},
                    "range": string("1-based artboards such as \"1-3, 5\" (PDF: one page each; other formats take one)"),
                    "selection": {"type": "boolean", "description": "Export only the selection, cropped to it"},
                    "outlineText": {"type": "boolean", "description": "SVG: text as outlines (viewable without the fonts)"},
                    "options": {"type": "object", "description": "More format options, passed through (see run_command document.formats), e.g. {\"quality\": 80} for jpg"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "add_text",
            "Add text",
            "Create point type at (x, y) (the first baseline), area type when width/height are given, or type in/on an existing path (`path` id + `mode` area|onPath; `pathEffect` rainbow|skew|3dRibbon|stairStep|gravity). Returns the text object id.",
            obj(
                json!({
                    "text": {"type": "string"},
                    "x": num("Baseline origin x (point/area type: left)"),
                    "y": num("First baseline y (area type: top)"),
                    "width": num("Area type frame width"),
                    "height": num("Area type frame height"),
                    "path": {"type": "integer", "description": "Existing path id to flow the text in (area) or along (onPath)"},
                    "mode": {"type": "string", "enum": ["area", "onPath"]},
                    "pathEffect": {"type": "string", "enum": ["rainbow", "skew", "3dRibbon", "stairStep", "gravity"]},
                    "size": num("Font size in pt"),
                    "font": {"type": "string", "description": "Font family"},
                    "color": paint_schema("Text colour (#rrggbb)"),
                }),
                &["text"],
            ),
            false,
        ),
        tool(
            "apply_effect",
            "Apply live effect",
            "Append a live effect (Effect menu) to the selection or `ids`. Omit `effect` to get the catalogue (ids, parameters, defaults). Examples: stylize.dropShadow {x, y, blur, opacity}, distort.roughen, warp.arc {bend}, path.offsetPath {offset}, pathfinder.add (on groups), blur.gaussian {radius}.",
            obj(
                json!({
                    "effect": {"type": "string", "description": "Effect id (see the catalogue)"},
                    "params": {"type": "object", "description": "Effect parameters; missing keys take the dialog defaults"},
                    "ids": {"type": "array", "items": {"type": "integer"}},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "pathfinder",
            "Pathfinder",
            "Combine the selected (or `ids`) objects destructively, back to front: unite, minusFront, intersect, exclude, divide, trim, merge, crop, outline, minusBack. For a non-destructive version apply the pathfinder.* effect to a group.",
            obj(
                json!({
                    "operation": {"type": "string", "enum": ["unite", "minusFront", "intersect", "exclude", "divide", "trim", "merge", "crop", "outline", "minusBack"]},
                    "ids": {"type": "array", "items": {"type": "integer"}},
                }),
                &["operation"],
            ),
            false,
        ),
        tool(
            "transform",
            "Transform",
            "Move, rotate, scale, reflect or shear the selection (or `ids`); several may be combined and run in that order. Angles in degrees (counter-clockwise), scale in %. `origin` [x,y] defaults to the selection centre; `copy` transforms a copy.",
            obj(
                json!({
                    "ids": {"type": "array", "items": {"type": "integer"}},
                    "dx": num("Move right"),
                    "dy": num("Move down"),
                    "rotate": num("Rotation in degrees"),
                    "scale": num("Uniform scale in %"),
                    "scaleX": num("Horizontal scale in %"),
                    "scaleY": num("Vertical scale in %"),
                    "reflect": {"type": "string", "enum": ["vertical", "horizontal"]},
                    "shear": num("Shear angle in degrees (horizontal axis)"),
                    "origin": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                    "copy": {"type": "boolean"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "create_graph",
            "Create graph",
            "Create a graph (graph tools) in the plot rectangle. Data as `csv` (first row: empty cell then series names; then one row per category: label, values…) or `series`/`categories`/`rows`. An empty CSV cell or a null in `rows` is a blank value (no column; lines break around it); a number in straight quotes is a label. Edit later with run_command graph.setData / graph.setType.",
            obj(
                json!({
                    "type": {"type": "string", "enum": ["column", "stackedColumn", "bar", "stackedBar", "line", "area", "scatter", "pie", "radar"]},
                    "x": num("Plot left"),
                    "y": num("Plot top"),
                    "width": num("Plot width"),
                    "height": num("Plot height"),
                    "csv": {"type": "string"},
                    "series": {"type": "array", "items": {"type": "string"}},
                    "categories": {"type": "array", "items": {"type": "string"}},
                    "rows": {"type": "array", "items": {"type": "array", "items": {"type": ["number", "null"]}}},
                }),
                &["x", "y", "width", "height"],
            ),
            false,
        ),
        tool(
            "text_wrap",
            "Text wrap",
            "Make the selected (or `ids`) objects wrap objects: area type below them in the same layer flows around them (offset in pt, invert = flow inside). `release: true` removes the wrap.",
            obj(
                json!({
                    "ids": {"type": "array", "items": {"type": "integer"}},
                    "offset": num("Gap between text and object (default 6 pt)"),
                    "invert": {"type": "boolean"},
                    "release": {"type": "boolean"},
                }),
                &[],
            ),
            false,
        ),
        tool("undo", "Undo", "Undo the last change (Edit → Undo).", empty(), false),
        tool("redo", "Redo", "Redo the last undone change (Edit → Redo).", empty(), false),
    ]
}

// ---------- implementation ----------

type Args = Map<String, Value>;

fn req_str<'a>(a: &'a Args, k: &str) -> Result<&'a str, String> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing string argument `{k}`"))
}

fn exec(b: &mut dyn Backend, cmd: &str, params: Value) -> Result<Value, String> {
    b.call("engine.execute", json!({"command": cmd, "params": params}))
}

fn need_ui(b: &dyn Backend, tool: &str) -> Result<(), String> {
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

fn apply_paint(b: &mut dyn Backend, a: &Args, ids: Option<Value>) -> Result<Value, String> {
    let with_ids = |mut p: Value| {
        if let Some(ids) = &ids {
            p["ids"] = ids.clone();
        }
        p
    };
    let mut applied = Map::new();
    if let Some(f) = a.get("fill") {
        exec(b, "paint.setFill", with_ids(paint_params(f)?))?;
        applied.insert("fill".into(), f.clone());
    }
    if let Some(s) = a.get("stroke") {
        exec(b, "paint.setStroke", with_ids(paint_params(s)?))?;
        applied.insert("stroke".into(), s.clone());
    }
    if let Some(w) = a.get("strokeWidth") {
        let w = w.as_f64().ok_or("`strokeWidth` must be a number")?;
        exec(b, "stroke.set", with_ids(json!({"weight": w})))?;
        applied.insert("strokeWidth".into(), json!(w));
    }
    Ok(Value::Object(applied))
}

fn created_id(r: &Value) -> Option<Value> {
    r.get("id").filter(|v| v.is_u64()).map(|id| json!([id]))
}

fn draw_path(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
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
    let r = exec(b, "path.create", p)?;
    let paint = apply_paint(b, a, created_id(&r))?;
    Ok(json!({"id": r.get("id"), "paint": paint}))
}

fn draw_shape(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let shape = req_str(a, "shape")?;
    let cmd = match shape {
        "rectangle" | "rect" => "shape.rectangle",
        "ellipse" | "circle" => "shape.ellipse",
        "polygon" => "shape.polygon",
        "star" => "shape.star",
        "line" => "shape.line",
        other => return Err(format!("unknown shape `{other}` (rectangle, ellipse, polygon, star, line)")),
    };
    let mut p = a.clone();
    for k in ["shape", "fill", "stroke", "strokeWidth"] {
        p.remove(k);
    }
    let r = exec(b, cmd, Value::Object(p))?;
    let paint = apply_paint(b, a, created_id(&r))?;
    Ok(json!({"id": r.get("id"), "shape": shape, "paint": paint}))
}

/// Select `ids` when given (tools that act on the selection).
fn select_ids(b: &mut dyn Backend, a: &Args) -> Result<(), String> {
    if let Some(ids) = a.get("ids") {
        exec(b, "select.set", json!({"ids": ids}))?;
    }
    Ok(())
}

fn add_text(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
    let text = a.get("text").and_then(Value::as_str).ok_or("missing string argument `text`")?;
    let mut style = json!({});
    for k in ["size", "font"] {
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
        if let Some(c) = a.get("color") {
            p["color"] = c.clone();
        }
        if let (Some(w), Some(h)) = (a.get("width"), a.get("height")) {
            p["area"] = json!({"width": w, "height": h});
        }
        exec(b, "text.create", p)?
    };
    if let Some(e) = a.get("pathEffect").and_then(Value::as_str) {
        exec(b, "type.pathOptions", json!({"effect": e}))?;
    }
    Ok(json!({"id": r.get("id")}))
}

fn transform(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
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

fn screenshot(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let path = a.get("path").and_then(Value::as_str);
    let window = a.get("window").and_then(Value::as_bool) == Some(true);
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
        b.call("ui.render", p)?
    };
    let png = match r.get("pngBase64").and_then(Value::as_str) {
        Some(b64) => vectorcraft_format::base64_decode(b64).ok_or("the backend returned bad base64")?,
        // An app from before `data` wrote the file without sending the image: read it back
        // (loopback: the same machine).
        None => match path {
            Some(path) => std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?,
            None if window => return old_window_capture(b),
            None => return Err("the renderer returned no image".into()),
        },
    };
    let info = if window {
        json!({"window": true, "width": r.get("width"), "height": r.get("height"), "path": path})
    } else {
        json!({"width": r.get("width"), "height": r.get("height"), "path": path})
    };
    Ok(image_result(&png, info))
}

/// The window of an app from before `ui.screenshot {data}`, captured through a temporary file.
fn old_window_capture(b: &mut dyn Backend) -> Result<ToolResult, String> {
    let tmp = std::env::temp_dir().join(format!("vectorcraft-window-{}.png", std::process::id())).to_string_lossy().to_string();
    let r = b.call("ui.screenshot", json!({"path": tmp}))?;
    let png = std::fs::read(&tmp).map_err(|e| format!("read {tmp}: {e}"))?;
    // Best effort: a temporary file left behind is harmless.
    std::fs::remove_file(&tmp).ok();
    Ok(image_result(&png, json!({"window": true, "width": r.get("width"), "height": r.get("height"), "path": Value::Null})))
}

fn image_result(png: &[u8], info: Value) -> ToolResult {
    ToolResult {
        content: vec![
            json!({"type": "image", "data": vectorcraft_format::base64_encode(png), "mimeType": "image/png"}),
            json!({"type": "text", "text": info.to_string()}),
        ],
        is_error: false,
    }
}

fn filter_commands(all: Value, a: &Args) -> Value {
    let needle = a.get("filter").and_then(Value::as_str).map(str::to_lowercase).filter(|s| !s.is_empty());
    let enabled_only = a.get("enabledOnly").and_then(Value::as_bool).unwrap_or(false);
    let Value::Array(list) = all else { return all };
    let hit = |c: &Value| {
        if enabled_only && c.get("enabled").and_then(Value::as_bool) == Some(false) {
            return false;
        }
        let Some(n) = &needle else { return true };
        let field = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_lowercase();
        let menu = c
            .get("menu")
            .and_then(Value::as_array)
            .map(|m| m.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
            .to_lowercase();
        field("id").contains(n) || field("label").contains(n) || menu.contains(n)
    };
    Value::Array(list.into_iter().filter(hit).collect())
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
        "ui_screenshot" => screenshot(b, &Map::from_iter([("window".into(), json!(true))])),
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
            // Only the slicing options reach the command, which validates them.
            let slice: Map<String, Value> =
                a.iter().filter(|(k, _)| matches!(k.as_str(), "depth" | "childLimit")).map(|(k, v)| (k.clone(), v.clone())).collect();
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
            let ids = a.get("ids").cloned();
            j(json!({"applied": apply_paint(b, a, ids)?}))
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
        "open_panel" => {
            need_ui(b, name)?;
            j(b.call("ui.set", json!({"panel": req_str(a, "panel")?}))?)
        }
        "screenshot" => screenshot(b, a),
        "open_file" => j(b.call("app.open", json!({"path": req_str(a, "path")?}))?),
        "save_file" => j(b.call("app.save", json!({"path": a.get("path").and_then(Value::as_str)}))?),
        "export" => {
            // One params object for every backend; the named arguments win over `options` (a named
            // artboard or range replaces every artboard choice in `options`).
            let mut params = a.get("options").and_then(Value::as_object).cloned().unwrap_or_default();
            if ["artboard", "range"].iter().any(|k| a.get(*k).is_some_and(|v| !v.is_null())) {
                for k in ARTBOARD_PARAMS {
                    params.remove(k);
                }
            }
            for k in ["path", "format", "scale", "artboard", "range", "outlineText"] {
                if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
                    params.insert(k.into(), v.clone());
                }
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

fn command_batch(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let steps = a.get("steps").and_then(Value::as_array).ok_or("`steps` must be an array")?;
    let stop = match a.get("stop_on_error") {
        None => true,
        Some(v) => v.as_bool().ok_or("`stop_on_error` must be a boolean")?,
    };
    let (mut completed, mut failed, mut results) = (0, 0, Vec::new());
    // Each step's result (null for a failed one), for later steps' `"$N.path"` references.
    let mut values: Vec<Value> = vec![];
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
            b.call("engine.execute", json!({"command":id,"params":params}))
        })
        .unwrap_or_else(|e| Err(format!("internal error: {e}")));
        match result {
            Ok(v) => {
                completed += 1;
                values.push(v.clone());
                results.push(json!({"ok":true,"result":v}));
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
    let mut r = ToolResult::json(&json!({"completed":completed,"failed":failed,"results":results}));
    r.is_error = failed > 0;
    Ok(r)
}

fn render_preview(b: &mut dyn Backend, a: &Args) -> Result<ToolResult, String> {
    let max = match a.get("max_side") {
        None => 1024,
        Some(v) => v.as_u64().filter(|v| (1..=4096).contains(v)).ok_or("`max_side` must be an integer in 1..4096")?,
    };
    let doc = b.call("document.inspect", json!({"depth":0}))?;
    let board = doc.get("artboards").and_then(Value::as_array).and_then(|v| v.first()).ok_or("no such artboard")?;
    let width = board.get("width").and_then(Value::as_f64).ok_or("artboard has no width")?;
    let height = board.get("height").and_then(Value::as_f64).ok_or("artboard has no height")?;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("invalid artboard dimensions".into());
    }
    // The headless renderer clamps scale at 0.01. Bound its allocation even for
    // a huge artboard, then downsize to the requested dimensions.
    let scale = (max as f64 / width.max(height)).clamp(0.01, 1.0);
    if width.max(height) * scale > 4096.0 {
        return Err("artboard is too large for a bounded preview".into());
    }
    let r = b.call("ui.render", json!({"scale":scale}))?;
    let encoded = r.get("pngBase64").and_then(Value::as_str).ok_or("renderer returned no image")?;
    let png = vectorcraft_format::base64_decode(encoded).ok_or("renderer returned bad base64")?;
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|e| format!("read preview: {e}"))?;
    let img = img.thumbnail(max as u32, max as u32);
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("encode preview: {e}"))?;
    Ok(image_result(out.get_ref(), json!({"width":img.width(),"height":img.height()})))
}
