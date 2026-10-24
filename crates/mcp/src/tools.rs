//! MCP tool definitions (names, descriptions, JSON Schemas) and their implementations on top of a
//! [`Backend`].

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{ARTBOARD_PARAMS, FORMATS, OPEN_EXTS, SAVE_FORMATS, format, save_reads_option, writable_format};

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
/// The arguments `save_file` passes on to `document.save` beside `path`.
const SAVE_FILE_OPTIONS: [&str; 4] = ["compress", "version", "preview", "options"];
/// The arguments `doc_inspect` / `inspect_document` pass on to `document.inspect`.
const INSPECT_KEYS: [&str; 6] = ["depth", "childLimit", "ids", "kind", "limit", "offset"];

/// Their schema.
fn inspect_props() -> Value {
    json!({
        "depth": {"type": "integer", "minimum": 0, "description": "Child levels of the layer tree to include (default all; 0 = top layers with counts)"},
        "childLimit": {"type": "integer", "minimum": 0, "description": "Children shown per node (default all; a level that shows fewer reports childCount)"},
        "ids": {"type": "array", "maxItems": 10000, "items": {"type": "integer", "minimum": 0}, "description": "List just these nodes (any level) as `nodes` instead of the whole layer tree; an unknown id is an error"},
        "kind": {"type": "string", "description": "List every node of this panel label (Layer, Group, Path, Type, Image, ...; ignores case) as `nodes`, each with its ancestor `path`; searches the subtrees of `ids` when given"},
        "limit": {"type": "integer", "minimum": 0, "description": "At most this many layers or nodes (the reply adds `total`)"},
        "offset": {"type": "integer", "minimum": 0, "description": "Layers or nodes skipped first (default 0)"}
    })
}
fn paint_schema(what: &str) -> Value {
    json!({
        "description": format!("{what}: \"#rrggbb\", \"none\", [r,g,b] (0..1), {{\"c\",\"m\",\"y\",\"k\"}}, {{\"gray\"}}, or a full paint.setFill params object ({{\"gradient\":…}} / {{\"swatch\":name}})"),
        "anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "number"}}, {"type": "object"}]
    })
}
/// The options `draw_path` and `draw_shape` apply to the object they draw (#2112).
fn with_draw_options(mut props: Value) -> Value {
    let more = json!({
        "dash": {"description": "Dash and gap lengths in pt ([d, g, …]), or null for a solid stroke (stroke.set `dash`)", "anyOf": [{"type": "array", "items": {"type": "number"}}, {"type": "null"}]},
        "cap": {"type": "string", "enum": ["butt", "round", "square", "projecting"], "description": "Stroke cap"},
        "join": {"type": "string", "enum": ["miter", "round", "bevel"], "description": "Stroke corner join"},
        "opacity": {"type": "number", "minimum": 0, "maximum": 100, "description": "Opacity in % (object.setProps `opacity`)"},
        "name": string("Name in the Layers panel"),
        "effects": {
            "type": "array",
            "description": "Live effects applied in order, as apply_effect: [{effect: id (effect.list), params?: {…}}]",
            "items": obj(json!({"effect": string("Effect id, e.g. \"blur.gaussian\""), "params": {"type": "object"}}), &["effect"]),
        },
    });
    if let (Some(p), Value::Object(more)) = (props.as_object_mut(), more) {
        p.extend(more);
    }
    props
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
            "Discover command ids, labels, parameter descriptions and enabled state. Returns an array. The whole catalogue is large: narrow it with filter (or query), menu, and brief. list_commands lists the same commands; it takes enabledOnly instead of enabled_only and returns {count, commands}.",
            obj(
                json!({"filter": string("Case-insensitive id, label or menu substring"), "query": string("Same as filter"), "menu": string(MENU_FILTER), "enabled_only": {"type":"boolean"}, "brief": {"type":"boolean", "description": BRIEF}}),
                &[],
            ),
            true,
        ),
        tool(
            "command_run",
            "Run a command",
            "Run an engine command by id with its JSON params. Command parameters follow the registry's descriptions. run_command runs it the same way; it takes the id as command instead of id.",
            obj(json!({"id": string("Command id"), "params": {"type":"object"}}), &["id"]),
            false,
        ),
        tool(
            "command_batch",
            "Run several commands",
            "Run steps in order; each edit is its own undo step, or with atomic:true the whole batch is one (named `label`, else after its first edit; `atomic` says which documents). A step's id is a command id, or draw_path / draw_shape with that tool's arguments as params (its result is the tool's: the new object's `id`). A param string that is exactly \"$N.path\" (N: 0-based step index) or \"$last.path\" takes that value from an earlier step's result, e.g. {\"id\":\"$1.id\"}; \"$$\" starts a literal \"$\". Returns completed, failed, results and created (the ids draw_path / draw_shape steps made, in order). Stops on the first error by default. At most 256 steps; kept step results total at most 8 MiB, past which the batch stops with the reason.",
            obj(
                json!({"steps": {"type":"array", "maxItems": 256, "items":obj(json!({"id":string("Command id, or draw_path / draw_shape"),"params":{"type":"object"}}), &["id"])}, "stop_on_error":{"type":"boolean", "default":true}, "atomic":{"type":"boolean", "default":false, "description":"Make the batch's edits one undo step in the active document and in each document a step opens or creates; in a document open before the batch that a step switches to, each edit stays its own undo step"}, "label":string("The undo step's name with atomic")}),
                &["steps"],
            ),
            false,
        ),
        tool(
            "doc_inspect",
            "Inspect document",
            "Summary of the active document, including artboards, layer tree, selection and history. On a large document scope it: depth 0 for the layer skeleton, ids for just those nodes, kind for every node of one kind, limit/offset to page. inspect_document returns the same summary.",
            obj(inspect_props(), &[]),
            true,
        ),
        tool(
            "render_preview",
            "Render a preview",
            "Render an artboard (default the first) as inline PNG without changing the document. max_side defaults to 1024, limited to 1..4096.",
            obj(
                json!({"max_side":{"type":"integer","minimum":1,"maximum":4096},"artboard":{"type":"integer","minimum":0,"description":"0-based artboard (default 0)"}}),
                &[],
            ),
            true,
        ),
        tool(
            "ui_inspect",
            "Inspect the interface",
            "Inspect the connected desktop interface; returns a tool error in headless mode. The same call as inspect_ui.",
            empty(),
            true,
        ),
        tool(
            "ui_screenshot",
            "Capture the interface",
            "Capture the connected desktop window as inline PNG; requires a running desktop app. The same call as screenshot with window:true. max_side scales the returned image down to at most that many pixels on its longer side (1..4096; default: the full capture).",
            obj(json!({"max_side": {"type":"integer","minimum":1,"maximum":4096,"description":MAX_SIDE_WINDOW}}), &[]),
            true,
        ),
        tool(
            "list_commands",
            "List commands",
            "List VectorCraft commands (id, label, menu path, shortcut, parameter description, enabled state). Every editing action is a command; run any of them with run_command. Coordinates are points in document space, y down, origin at the first artboard's top-left. command_list lists the same commands as a bare array and takes enabled_only.",
            obj(
                json!({"filter": string("Case-insensitive substring matched against id, label and menu path (e.g. \"align\", \"Object\")"), "query": string("Same as filter"), "menu": string(MENU_FILTER), "enabledOnly": {"type": "boolean", "description": "Only commands that can run right now"}, "brief": {"type": "boolean", "description": BRIEF}}),
                &[],
            ),
            true,
        ),
        tool(
            "run_command",
            "Run command",
            "Execute any VectorCraft command by id with JSON params (see list_commands for ids and params). Examples: {\"command\":\"object.group\"}, {\"command\":\"object.align\",\"params\":{\"horizontal\":\"left\"}}, {\"command\":\"file.new\",\"params\":{\"width\":800,\"height\":600}}. Returns the command's result (e.g. {id} for creation commands). command_run runs it the same way and takes the id as id.",
            obj(
                json!({"command": string("Command id, e.g. shape.rectangle, object.group, paint.setFill"), "params": {"type": "object", "description": "Command parameters"}}),
                &["command"],
            ),
            false,
        ),
        tool(
            "inspect_document",
            "Inspect document",
            "Summary of the active document: artboards, layer tree (ids, names, kinds, bounds, fill/stroke), selection, undo history, current tool and default paint. On a large document pass depth: 0 for the layer skeleton (truncated levels report childCount), then locate objects with run_command document.find and read them with document.node. doc_inspect returns the same summary.",
            obj(inspect_props(), &[]),
            true,
        ),
        tool(
            "inspect_ui",
            "Inspect UI",
            "UI state of the running desktop app: tool and options, panels, dialogs, view (zoom/center), window, documents, perf. Desktop app only. The same call as ui_inspect.",
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
            "Create a path from points ([[x,y],…] or [{x,y,in?:[x,y],out?:[x,y]},…] with Bézier handles) or SVG path data `d`, then apply optional paint, stroke dash/cap/join, opacity, name and live effects. Omitted paint fields inherit the current new-art appearance (initially white fill and 1 pt black stroke). For a filled path without an outline, pass an explicit fill and stroke:\"none\". Returns the new object id.",
            obj(
                with_draw_options(json!({
                    "points": {"type": "array", "description": "Anchors: [x,y] pairs or {x, y, in?, out?, smooth?} objects", "items": {"anyOf": [{"type": "array", "items": {"type": "number"}, "minItems": 2}, {"type": "object"}]}},
                    "d": string("SVG path data (alternative to points), e.g. \"M0 0 L100 0 L50 80 Z\""),
                    "closed": {"type": "boolean", "description": "Close the path (points only)"},
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                })),
                &[],
            ),
            false,
        ),
        tool(
            "draw_shape",
            "Draw shape",
            "Create a live shape and apply optional paint. rectangle/ellipse: x, y, width, height (rectangle: radius = corner radius). polygon: cx, cy, radius, sides. star: cx, cy, radius1, radius2, points. line: x1, y1, x2, y2. Omitted paint fields inherit the current new-art appearance (initially white fill and 1 pt black stroke); lines start with no fill and a visible stroke. For a filled shape without an outline, pass an explicit fill and stroke:\"none\". Stroke dash/cap/join, opacity, name and live effects apply to the new shape as in draw_path. Returns the new object id.",
            obj(
                with_draw_options(json!({
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
                })),
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
            "Render the active artboard to PNG and return it as an image (optionally also saving it to `path`). With window:true (desktop app only) captures the whole app window instead, as ui_screenshot does; max_side then scales the returned image down (a file written to `path` keeps the full capture).",
            obj(
                json!({
                    "path": string("Also write the PNG here"),
                    "scale": num("Pixels per point (default 1)"),
                    "artboard": {"type": "integer", "minimum": 0, "description": "Artboard index (default 0)"},
                    "window": {"type": "boolean", "description": "Capture the app window (desktop app only)"},
                    "max_side": {"type": "integer", "minimum": 1, "maximum": 4096, "description": MAX_SIDE_WINDOW},
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
                "Save the active document with the engine's document.save: by default to its own file in its own format (native .vectorcraft unless it was opened from or saved as SVG, PDF or a restorable .ai). With `path`, the extension picks the format: .{}. Other formats are exports (see export). A native file is gzipped when `compress` is true (default: the useCompression preference, Preferences > File Handling > Use Compression); `version` writes it for an older VectorCraft and `preview` embeds a PNG of the first artboard. `options` carries the format's save options (see run_command document.formats); giving `compress`, `version` or `preview` both at the top level and in `options` with different values is an error. The reply's `format` says what was written and `warnings` what that format loses and each option given that the format doesn't read (such as `compress` for .svg). In the running app the call returns once the file is written, or with the error, even with Background Save on. When the save asks first (before replacing a file that opened with losses, or with a file dialog for an untitled document without `path`), the reply is `pending` or \"cancelled\" as before.",
                save_extensions().join(", .")
            ),
            obj(
                json!({
                    "path": string("Destination; its extension picks the format (default: the document's own file)"),
                    "compress": {"type": "boolean", "description": "Native files: gzip the file (default: the useCompression preference)"},
                    "version": {"type": "integer", "minimum": 1, "maximum": vectorcraft_format::VERSION, "description": "Native files: the format version to write (default: the current one; 1 and 2 are never compressed)"},
                    "preview": {"type": "boolean", "description": "Native files: embed a PNG of the first artboard, at most 256 px (default false)"},
                    "options": {"type": "object", "description": "The format's save options, passed through (see run_command document.formats); default: as last saved"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "export",
            "Export",
            "Export the active document with the engine's document.export (the same bytes in the app and headless), in any format of the `format` enum: vectors (svg/svgz with the artboard as viewBox, eps, dxf, emf, wmf), pdf and ai (one page per artboard: all, or `artboard` / `range`; ai is the PDF-compatible .ai file that Save writes, which carries the document when every artboard is written in order), rasters (one rendered artboard: png, jpg, webp, gif, png8 as an indexed .png, tiff, bmp, tga, psd with its layers), txt (the stories, back to front), vectorcraft or template (a native copy or template; the document keeps its path). run_command document.formats lists each format's options. For SVG/SVGZ, top-level `outlineText: true` writes text as paths; DXF takes it too (its type is outlined anyway unless `preserve: editability`). PDF instead uses `options.advanced.outlineText: true`, default false (live text). `selection: true` exports only the selected objects, cropped to their bounds. Template layers are left out; live effects are kept (geometry baked, SVG filters for shadows/glows/blur). Without `path` the bytes come back as dataBase64.",
            obj(
                json!({
                    "format": {"type": "string", "enum": FORMATS.iter().filter(|f| f.write).map(|f| f.id).collect::<Vec<_>>(), "description": "Default: from the path's extension"},
                    "path": string("Destination file (omit to get dataBase64)"),
                    "scale": num("Raster pixels per point (default 1)"),
                    "artboard": {"type": "integer", "minimum": 0, "description": "0-based artboard (default: the first; PDF and .ai default: all)"},
                    "range": string("1-based artboards such as \"1-3, 5\" (PDF and .ai: one page each; SVG and SVGZ: one file each; EPS: one file each, after a central file of the whole document when there are two or more; rasters, DXF, EMF and WMF: one, or one file each with options.useArtboards: true)"),
                    "selection": {"type": "boolean", "description": "Export only the selection, cropped to it"},
                    "outlineText": {"type": "boolean", "description": "SVG/SVGZ: true writes text as outlines (viewable without the fonts). DXF takes it too (type outlined anyway unless preserve: editability). PDF uses options.advanced.outlineText instead (default false: live text)"},
                    "options": {"type": "object", "description": "More format options, passed through (see run_command document.formats), e.g. {\"quality\": 80} for jpg or {\"advanced\": {\"outlineText\": true}} for PDF text as outlines (default false: live text)"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "add_text",
            "Add text",
            "Create point type at (x, y) (the first baseline), area type when width/height are given (`fit` autoHeight grows the frame to the text, shrinkText scales the text down to fit), or type in/on an existing path (`path` id + `mode` area|onPath; `pathEffect` rainbow|skew|3dRibbon|stairStep|gravity). Returns the text object id, with `overflow: true` when the text doesn't fit its frame or path (the lines past it aren't drawn: enlarge the frame, shrink the text or give `fit`).",
            obj(
                json!({
                    "text": {"type": "string"},
                    "x": num("Baseline origin x (area type: left; point type: where the line starts, or its centre or end with justify center/right)"),
                    "y": num("First baseline y (area type: top)"),
                    "width": num("Area type frame width"),
                    "height": num("Area type frame height"),
                    "vertical": {"type": "boolean", "description": "Vertical writing for point, area or path type (default false, horizontal), as text.create / text.createInPath"},
                    "fit": {"type": "string", "enum": ["none", "autoHeight", "shrinkText"], "description": "Area type: keep the frame (none), fit its height to the text (autoHeight) or scale overflowing text down (shrinkText)"},
                    "fitMinPercent": num("shrinkText: the smallest scale in percent, 10 to 100 (default 50)"),
                    "path": {"type": "integer", "description": "Existing path id to flow the text in (area) or along (onPath)"},
                    "mode": {"type": "string", "enum": ["area", "onPath"]},
                    "pathEffect": {"type": "string", "enum": ["rainbow", "skew", "3dRibbon", "stairStep", "gravity"]},
                    "size": num("Font size in pt"),
                    "font": {"type": "string", "description": "Font family"},
                    "color": paint_schema("Text colour (#rrggbb)"),
                    "justify": {"type": "string", "enum": ["auto", "left", "center", "right", "justifyAll"], "description": "Paragraph alignment (default auto: the start of the text's direction). Point type lines up on x: center centres each line on it, right ends it there"},
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
        tool("undo", "Undo", "Undo the last change (Edit → Undo). redo reapplies it.", empty(), false),
        tool("redo", "Redo", "Redo the last undone change (Edit → Redo): the change the last undo took back.", empty(), false),
    ]
}

// ---------- implementation ----------

type Args = Map<String, Value>;

fn req_str<'a>(a: &'a Args, k: &str) -> Result<&'a str, String> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing string argument `{k}`"))
}

/// A file tool's path made absolute against this process's working directory. An empty path is
/// returned as given.
fn absolute_path(p: &str) -> Result<String, String> {
    if p.is_empty() {
        return Ok(String::new());
    }
    let abs = std::path::absolute(p).map_err(|e| format!("{p}: {e}"))?;
    abs.into_os_string().into_string().map_err(|abs| format!("{p}: the absolute path {} is not UTF-8", abs.to_string_lossy()))
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

fn apply_paint(b: &mut dyn Backend, a: &Args, ids: Option<Value>) -> Result<Value, String> {
    let (steps, applied) = paint_steps(a)?;
    for (cmd, mut p) in steps {
        if let Some(ids) = &ids {
            p["ids"] = ids.clone();
        }
        exec(b, cmd, p)?;
    }
    Ok(Value::Object(applied))
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
    draw(b, a, "Pen", ("path.create", p), json!({}))
}

fn draw_shape(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
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
fn select_ids(b: &mut dyn Backend, a: &Args) -> Result<(), String> {
    if let Some(ids) = a.get("ids") {
        exec(b, "select.set", json!({"ids": ids}))?;
    }
    Ok(())
}

fn add_text(b: &mut dyn Backend, a: &Args) -> Result<Value, String> {
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
    let path = a.get("path").and_then(Value::as_str).map(absolute_path).transpose()?;
    let path = path.as_deref();
    let window = a.get("window").and_then(Value::as_bool) == Some(true);
    let max = max_side(a)?;
    if max.is_some() && !window {
        return Err("`max_side` applies to a window capture (`window: true`); size an artboard render with `scale`".into());
    }
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
fn max_side(a: &Args) -> Result<Option<u32>, String> {
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

fn image_result(png: &[u8], info: Value) -> ToolResult {
    ToolResult {
        content: vec![
            json!({"type": "image", "data": vectorcraft_format::base64_encode(png), "mimeType": "image/png"}),
            json!({"type": "text", "text": info.to_string()}),
        ],
        is_error: false,
    }
}

/// The `menu` argument of the command listings.
const MENU_FILTER: &str = "Case-insensitive substring of the menu path, its levels joined with \" > \" (e.g. \"Object\", \"Object > Path\")";
/// The `brief` argument of the command listings.
const BRIEF: &str = "Leave out each command's params description (the bulk of the catalogue); run command_list with the id as filter to read it";
/// The `max_side` argument of the window captures.
const MAX_SIDE_WINDOW: &str =
    "Window capture only: longest side of the returned image in pixels; a larger capture is scaled down to it (default: the full capture)";

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
        "open_file" => j(b.call("app.open", json!({"path": absolute_path(req_str(a, "path")?)?}))?),
        "save_file" => {
            let mut p = json!({"path": a.get("path").and_then(Value::as_str).map(absolute_path).transpose()?});
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
                let path = absolute_path(path)?;
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
    let r = b.call("ui.render", json!({"scale":scale, "artboard": index}))?;
    let encoded = r.get("pngBase64").and_then(Value::as_str).ok_or("renderer returned no image")?;
    let png = vectorcraft_format::base64_decode(encoded).ok_or("renderer returned bad base64")?;
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|e| format!("read preview: {e}"))?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("encode preview: {e}"))?;
    Ok(image_result(out.get_ref(), json!({"width":img.width(),"height":img.height()})))
}
