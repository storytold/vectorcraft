//! MCP prompts (`prompts/list`, `prompts/get`) and `completion/complete`.
//!
//! A prompt is a reusable VectorCraft workflow: `prompts/get` hands the model one templated user
//! message, so a client can offer "make a poster" the way it offers a slash command. Every prompt's
//! arguments double as completion sources, and the resource templates in [`crate::resources`] share
//! the same [`Live`] catalogues, so `completion/complete` can suggest real command ids, effect ids,
//! swatch names and export formats instead of a fixed list that goes stale.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::FORMATS;

use crate::backend::Backend;

/// How many values one `completion/complete` reply may carry (the spec's maximum).
const MAX_VALUES: usize = 100;

/// A catalogue the completion values are read from, so they follow the app instead of a copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Live {
    /// Export formats the engine writes (`document.formats`).
    Formats,
    /// Image Trace preset names (`imageTrace.presets`).
    TracePresets,
    /// Live effect ids (`effect.list`).
    Effects,
    /// Every command id (`engine.commands`).
    Commands,
    /// Swatch and colour-group names in the document (`swatch.list`).
    Swatches,
    /// Layer and object ids in the active document (`document.inspect`).
    Objects,
}

/// Where one prompt argument's suggestions come from.
#[derive(Clone, Copy)]
pub enum Values {
    /// A fixed list, written out here.
    Fixed(&'static [&'static str]),
    /// The live catalogue.
    Live(Live),
}

impl Values {
    fn resolve(&self, b: &mut dyn Backend) -> Vec<String> {
        match self {
            Values::Fixed(v) => v.iter().map(|s| (*s).to_string()).collect(),
            Values::Live(live) => live_values(b, *live),
        }
    }
}

/// The values of one live catalogue, read fresh so a suggestion is never stale.
pub(crate) fn live_values(b: &mut dyn Backend, live: Live) -> Vec<String> {
    match live {
        Live::Formats => FORMATS.iter().filter(|f| f.write).map(|f| f.id.to_string()).collect(),
        Live::TracePresets => names(b, "imageTrace.presets", Some("presets")),
        Live::Effects => names(b, "effect.list", Some("catalog")),
        // The command catalogue is a control method, not an engine command.
        Live::Commands => b.call("engine.commands", json!({})).map(|v| ids_in(&v, None)).unwrap_or_default(),
        Live::Swatches => names(b, "swatch.list", Some("swatches")),
        Live::Objects => object_ids(b),
    }
}

/// The `id` or `name` of every entry a query command returns, optionally under one key.
///
/// These are engine commands, so they go through `engine.execute`; that reaches the same commands
/// in both backends, where a control method the headless session doesn't implement would not.
fn names(b: &mut dyn Backend, command: &str, key: Option<&str>) -> Vec<String> {
    b.call("engine.execute", json!({"command": command, "params": {}})).map(|v| ids_in(&v, key)).unwrap_or_default()
}

fn ids_in(reply: &Value, key: Option<&str>) -> Vec<String> {
    let items = match key {
        Some(k) => reply.get(k).and_then(Value::as_array).map_or(&[][..], Vec::as_slice),
        None => reply.as_array().map_or(&[][..], Vec::as_slice),
    };
    // Catalogs are small, but never trust a document that grew one.
    items.iter().take(MAX_VALUES * 8).filter_map(|i| i.get("id").or_else(|| i.get("name"))).filter_map(Value::as_str).map(str::to_string).collect()
}

/// Ids of every layer and object in the active document, for `vectorcraft://object/{id}`.
fn object_ids(b: &mut dyn Backend) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(v) = b.call("document.inspect", json!({})) else { return out };
    let Some(layers) = v.get("layers").and_then(Value::as_array) else { return out };
    let mut stack: Vec<&Value> = layers.iter().rev().collect();
    while let Some(node) = stack.pop() {
        if let Some(id) = node.get("id").and_then(Value::as_i64) {
            out.push(id.to_string());
        }
        if let Some(kids) = node.get("children").and_then(Value::as_array) {
            stack.extend(kids.iter().rev());
        }
    }
    out
}

/// One prompt argument.
pub struct PromptArg {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
    /// Substituted for `{name}` when the caller leaves the argument out.
    pub default: &'static str,
    pub values: Values,
}

/// One prompt: a workflow with templated arguments.
pub struct PromptDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub arguments: &'static [PromptArg],
    /// The message body; every `{argument}` is replaced with its value.
    pub text: &'static str,
}

/// Recolour methods (`recolor.apply`'s `method`), written here because they are a closed set.
const RECOLOR_METHODS: &[&str] = &["exact", "preserveTints", "scaleTints", "tintsShades", "hueShift"];

static NONE: &[&str] = &[];

/// Every prompt, in `prompts/list` order.
pub static PROMPTS: &[PromptDef] = &[
    PromptDef {
        name: "poster",
        title: "Design a poster",
        description: "Lay out a poster from a brief: live shapes, gradients, type and live effects, checked with screenshots before export.",
        arguments: &[
            PromptArg {
                name: "brief",
                description: "What the poster is for, and the words it must carry",
                required: true,
                default: "",
                values: Values::Fixed(NONE),
            },
            PromptArg {
                name: "palette",
                description: "The colours to use, or how to choose them",
                required: false,
                default: "a limited scheme of three to five colours taken from the brief",
                values: Values::Fixed(&[
                    "a limited scheme of three to five colours taken from the brief",
                    "a monochrome scheme in one hue, varying value and opacity",
                    "a two-colour complementary scheme",
                    "an analogous scheme across neighbouring hues",
                ]),
            },
            PromptArg {
                name: "text",
                description: "The exact headline and supporting lines to set",
                required: false,
                default: "invent short, concrete wording that fits the brief",
                values: Values::Fixed(NONE),
            },
        ],
        text: "\
Design a poster in Vector W3K2.

Brief: {brief}
Palette: {palette}
Wording: {text}

Coordinates are points in document space: y points down and the origin is the first
artboard's top-left. Work in the document that is already open (inspect_document); only
run file.new when the brief asks for a different artboard size.

Method:
1. Plan the layout before drawing — a hierarchy of shapes, then the type on top.
2. draw_shape for geometry (rectangle with `radius` for corners, ellipse, polygon, star,
   line); draw_path for anything with curves, from `points` or from SVG `d`.
3. set_paint for colour. A fill may be \"#rrggbb\", [r,g,b], {c,m,y,k}, {gray} or a whole
   paint.setFill params object — pass {\"gradient\": {…}} for a gradient.
4. add_text for type: point type at (x, y), area type with width and height, or mode
   \"area\"/\"onPath\" to flow it in or along a path.
5. apply_effect for live effects; call it with no `effect` first to get the catalogue with
   each effect's parameters and defaults.
6. screenshot to look at what you made, fix what is wrong, and only then export.

Keep it editable: live shapes, live effects and live text. Do not flatten and do not
rasterise unless the brief asks for it.",
    },
    PromptDef {
        name: "icon-set",
        title: "Draw an icon set",
        description: "Build a row of consistent icons as live vector shapes on a grid, then check them as a set.",
        arguments: &[
            PromptArg { name: "subject", description: "What the icons depict", required: true, default: "", values: Values::Fixed(NONE) },
            PromptArg {
                name: "count",
                description: "How many icons in the set",
                required: false,
                default: "6",
                values: Values::Fixed(&["4", "6", "8", "12", "16"]),
            },
            PromptArg {
                name: "detail",
                description: "How detailed each icon should be",
                required: false,
                default: "a single flat silhouette, no strokes",
                values: Values::Fixed(&[
                    "a single flat silhouette, no strokes",
                    "a filled silhouette with a stroke around it",
                    "an outlined shape with its strokes only",
                    "two-tone: a filled shape and a smaller accent shape",
                ]),
            },
        ],
        text: "\
Draw a set of {count} icons of {subject} in Vector W3K2, {detail}.

Coordinates are points in document space (y down, origin at the first artboard's
top-left).

Method:
1. Decide the cell size and the gap first, then lay every icon into its own cell so the set
   reads as one family: the same stroke weight, the same optical size, the same corner radius.
2. Build each icon from draw_shape and draw_path — live shapes, not strokes traced from
   somewhere else. Use `points` with `in`/`out` handles for the curves a silhouette needs.
3. set_paint the whole set the same way, then break one colour out on the object that needs
   the accent.
4. screenshot the set and judge it as a set: equal weight, aligned baselines, consistent
   padding. Fix what looks off rather than adding more detail.

Return the ids of the icons in a grid, and leave them as separate objects on one layer.",
    },
    PromptDef {
        name: "recolor",
        title: "Recolour artwork",
        description: "Recolour the selection with explicit rows, or cluster its colours first with recolor.colors.",
        arguments: &[
            PromptArg {
                name: "palette",
                description: "The colours to map onto, or 'cluster' to group similar colours first",
                required: true,
                default: "",
                values: Values::Fixed(&[
                    "cluster",
                    "a monochrome ramp from light to dark",
                    "a complementary two-colour scheme",
                    "one hue per layer, keeping the original values",
                ]),
            },
            PromptArg {
                name: "method",
                description: "How a row's colours take their new colour (recolor.apply's own default is exact; scaleTints suits a recolour driven from recolor.reduce)",
                required: false,
                default: "exact",
                values: Values::Fixed(RECOLOR_METHODS),
            },
        ],
        text: "\
Recolour the current selection in Vector W3K2 with {palette}, using the `{method}` recolour method.

Method:
1. inspect_document to see what is selected and what colours it actually uses.
2. If you do not know the colours, run recolor.colors first: it groups similar colours into
   rows (k-means in Lab, weighted by use) and hands back the map. Then edit the `to` colour of
   each row and apply it. That is the reliable route for artwork you did not draw.
3. If you know the colours, run recolor.apply directly with `map`: one row per source colour,
   `from` as keys or \"#rrggbb\", `to` as a colour. Each new colour keeps the model of the one it
   replaces, and gradients, meshes, text and pattern tiles follow.
4. Group the colours with `group` so the result stays editable, and use `limitTo` to snap new
   colours to a swatch library.
5. screenshot to check the result against the original intent.

One undo step covers the whole change; gradients and linked swatches stay linked.",
    },
    PromptDef {
        name: "trace-and-style",
        title: "Trace an image and style it",
        description: "Trace a placed raster with Image Trace, then apply a live effect and paint over the result.",
        arguments: &[
            PromptArg { name: "path", description: "Path to the image file", required: true, default: "", values: Values::Fixed(NONE) },
            PromptArg {
                name: "preset",
                description: "Image Trace preset to start from",
                required: false,
                default: "Default",
                values: Values::Live(Live::TracePresets),
            },
            PromptArg {
                name: "effect",
                description: "A live effect to apply to the traced shapes",
                required: false,
                default: "stylize.scribble",
                values: Values::Live(Live::Effects),
            },
        ],
        text: "\
Trace the image at {path} in Vector W3K2 with the \"{preset}\" preset, then style the result
with the {effect} effect.

Method:
1. open_file the image: it becomes a document of its own pixel size.
2. imageTrace.make with the preset name, adjusting `params` — mode blackAndWhite, grayscale or
   color, then threshold, colors, paths, corners, noise, method abutting or overlapping,
   ignoreWhite, snapCurvesToLines. Run imageTrace.presets to see every preset with its numbers.
   Lower paths and higher corners give the clean, logo-like result; high fidelity needs paths
   near 90.
3. apply_effect {effect} on the traced shapes. Call apply_effect with no `effect` for the
   catalogue and each effect's parameters.
4. set_paint over the traced shapes to give the artwork its own colours.
5. screenshot, then remove the placed image if it is still underneath.

The trace is real geometry: every traced shape stays selectable and editable.",
    },
    PromptDef {
        name: "export-set",
        title: "Export an artboard set",
        description: "Export every artboard in several formats at once, with per-format options, and report what each one lost.",
        arguments: &[
            PromptArg {
                name: "formats",
                description: "Formats to write, in order",
                required: false,
                default: "svg, pdf, png",
                values: Values::Live(Live::Formats),
            },
            PromptArg {
                name: "directory",
                description: "Where the files go",
                required: false,
                default: "the current directory",
                values: Values::Fixed(NONE),
            },
            PromptArg {
                name: "scale",
                description: "Raster scale in pixels per point (raster formats only)",
                required: false,
                default: "2",
                values: Values::Fixed(&["1", "2", "3", "4"]),
            },
        ],
        text: "\
Export the document in Vector W3K2 to {formats}, into {directory}, at {scale} pixels per
point for the raster formats.

Method:
1. document.formats lists every format the engine reads and writes, with each one's options and
   their defaults. Read it before exporting so the options you pass are real ones.
2. export once per format. A path's extension picks the format; `options` carries the rest, for
   example {\"quality\": 80} for jpg. pdf writes one page per artboard — all of them, or
   `artboard` (0-based) / `range` (\"1-3, 5\", 1-based); the other formats write one artboard.
3. export without a path to get the bytes back as dataBase64 instead of a file.
4. Report the `warnings` each export returns: they name what that format cannot carry.

Live effects are kept on export (geometry baked; shadows, glows and blur become SVG filters),
and text stays text unless you pass outlineText.",
    },
];

/// The message body as a client should receive it.
///
/// Our source wraps these texts at the column limit, which would leave line breaks in the middle
/// of a sentence — and a substituted argument landing on one of them. So each source line is
/// folded into the paragraph it belongs to, while list items keep a line of their own.
fn flow(text: &str) -> String {
    text.split("\n\n")
        .map(|paragraph| {
            let mut out = String::new();
            for line in paragraph.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if !out.is_empty() {
                    out.push(if is_item(line) { '\n' } else { ' ' });
                }
                out.push_str(line);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// A numbered or dashed list item, which starts a line instead of continuing one.
fn is_item(line: &str) -> bool {
    if let Some(body) = line.strip_prefix("- ") {
        return !body.is_empty();
    }
    line.split_once(". ").is_some_and(|(number, _)| !number.is_empty() && number.len() <= 3 && number.chars().all(|c| c.is_ascii_digit()))
}

/// `prompts/list`.
pub fn list() -> Value {
    let prompts: Vec<Value> = PROMPTS
        .iter()
        .map(|p| {
            let arguments: Vec<Value> =
                p.arguments.iter().map(|a| json!({"name": a.name, "description": a.description, "required": a.required})).collect();
            json!({"name": p.name, "title": p.title, "description": p.description, "arguments": arguments})
        })
        .collect();
    json!({ "prompts": prompts })
}

/// `prompts/get`: the prompt's messages with `{argument}` filled in.
pub fn get(name: &str, args: &Value) -> Result<Value, String> {
    let Some(p) = PROMPTS.iter().find(|p| p.name == name) else {
        let ids: Vec<&str> = PROMPTS.iter().map(|p| p.name).collect();
        return Err(format!("unknown prompt `{name}` ({})", ids.join("|")));
    };
    let mut text = flow(p.text);
    for a in p.arguments {
        let given = args.get(a.name).filter(|v| !v.is_null()).map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
        let value = match given {
            Some(v) if !v.is_empty() => v,
            _ if a.required => return Err(format!("prompt `{name}` needs the `{a_name}` argument", a_name = a.name)),
            _ => a.default.to_string(),
        };
        text = text.replace(&format!("{{{}}}", a.name), &value);
    }
    Ok(json!({
        "description": p.description,
        "messages": [{"role": "user", "content": {"type": "text", "text": text}}],
    }))
}

/// `completion/complete`: values for a prompt argument or a resource-template variable.
///
/// Unknown references and unknown argument names come back empty rather than as an error: a
/// completion is a convenience, and the caller is usually mid-typing.
pub fn complete(b: &mut dyn Backend, params: &Value) -> Value {
    let empty = json!({"completion": {"values": [], "total": 0, "hasMore": false}});
    let Some(reference) = params.get("ref") else { return empty };
    let typed = params.get("argument").and_then(|a| a.get("value")).and_then(Value::as_str).unwrap_or("");
    let name = params.get("argument").and_then(|a| a.get("name")).and_then(Value::as_str).unwrap_or("");

    let values = match reference.get("type").and_then(Value::as_str) {
        Some("ref/prompt") => match reference.get("name").and_then(Value::as_str).and_then(|n| PROMPTS.iter().find(|p| p.name == n)) {
            Some(p) => match p.arguments.iter().find(|a| a.name == name) {
                Some(a) => a.values.resolve(b),
                None => Vec::new(),
            },
            None => Vec::new(),
        },
        Some("ref/resource") => match reference.get("uri").and_then(Value::as_str).map(crate::resources::template_source) {
            Some((_, Some(live))) => live_values(b, live),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };

    // Rank by prefix first (what the user is typing), then by substring.
    let needle = typed.to_lowercase();
    let mut matches: Vec<String> = values.iter().filter(|v| v.to_lowercase().starts_with(&needle)).cloned().collect();
    if matches.is_empty() && !needle.is_empty() {
        matches = values.iter().filter(|v| v.to_lowercase().contains(&needle)).cloned().collect();
    }
    let total = matches.len();
    let has_more = total > MAX_VALUES;
    matches.truncate(MAX_VALUES);
    json!({"completion": {"values": matches, "total": total, "hasMore": has_more}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b() -> Box<dyn Backend> {
        Box::new(crate::Headless::with_document())
    }

    #[test]
    fn every_prompt_lists_and_renders() {
        let listed = list();
        let entries = listed["prompts"].as_array().expect("prompts");
        assert_eq!(entries.len(), PROMPTS.len());
        for (p, entry) in PROMPTS.iter().zip(entries) {
            assert!(!p.name.is_empty() && !p.text.is_empty(), "{}", p.name);
            assert_eq!(entry["name"], p.name);
            assert_eq!(entry["description"], p.description);
            // Arguments are advertised with the names prompts/get will accept.
            let names: Vec<&str> = p.arguments.iter().map(|a| a.name).collect();
            let listed: Vec<String> =
                entry["arguments"].as_array().map_or(Vec::new(), |a| a.iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect());
            let names: Vec<String> = names.into_iter().map(str::to_string).collect();
            assert_eq!(names, listed, "{}: listed arguments", p.name);

            // Every required argument supplied: the prompt renders.
            let args: Value = json!(p.arguments.iter().filter(|a| a.required).map(|a| (a.name, "x")).collect::<std::collections::BTreeMap<_, _>>());
            let v = get(p.name, &args).unwrap_or_else(|e| panic!("{} renders: {e}", p.name));
            assert_eq!(v["messages"].as_array().map(Vec::len), Some(1));
            assert_eq!(v["messages"][0]["role"], "user");
            let text = v["messages"][0]["content"]["text"].as_str().unwrap_or_default();
            assert!(text.len() > 200, "{}: too short", p.name);
            // Templating left nothing behind, and the source's line wrapping is gone.
            // Only a declared argument's braces are placeholders; JSON examples carry braces too.
            for a in p.arguments {
                assert!(!text.contains(&format!("{{{}}}", a.name)), "{}: `{}` left in place", p.name, a.name);
            }
            assert!(!text.contains('\n') || text.contains("\n\n") || p.arguments.is_empty(), "{}: stray line break", p.name);
            assert!(!text.contains("  "), "{}: double space", p.name);

            // Only a missing required argument may stop it.
            let missing: Value = json!({});
            assert_eq!(get(p.name, &missing).is_err(), p.arguments.iter().any(|a| a.required), "{}", p.name);
        }
    }

    #[test]
    fn flow_keeps_paragraphs_and_list_items() {
        let flowed = flow("first line\nsecond line\n\nnew paragraph\n1. one\n2. two\n- bullet");
        assert_eq!(flowed, "first line second line\n\nnew paragraph\n1. one\n2. two\n- bullet");
        assert_eq!(flow(""), "");
        assert_eq!(flow("no breaks"), "no breaks");
    }

    #[test]
    fn prompt_arguments_are_filled_in() {
        let v = get("export-set", &json!({"formats": "pdf, tiff", "scale": "4"})).unwrap();
        let text = v["messages"][0]["content"]["text"].as_str().unwrap_or_default();
        assert!(text.contains("pdf, tiff"), "{text}");
        assert!(text.contains("4 pixels per point"), "{text}");
    }

    #[test]
    fn unknown_prompt_and_missing_argument_are_errors() {
        assert!(get("nope", &json!({})).is_err());
        let e = get("poster", &json!({"brief": ""})).unwrap_err();
        assert!(e.contains("brief"), "{e}");
    }

    #[test]
    fn completions_rank_prefix_first_and_cap_at_100() {
        let mut b = b();
        let v = complete(
            b.as_mut(),
            &json!({"ref": {"type": "ref/prompt", "name": "trace-and-style"}, "argument": {"name": "preset", "value": "photo"}}),
        );
        let values = v["completion"]["values"].as_array().unwrap();
        assert!(!values.is_empty(), "imageTrace.presets should answer: {v}");
        assert!(values.iter().all(|x| x.as_str().unwrap_or_default().to_lowercase().contains("photo")), "{v}");

        // A nonsense prefix falls back to substring matches, and never exceeds 100.
        let v = complete(b.as_mut(), &json!({"ref": {"type": "ref/prompt", "name": "export-set"}, "argument": {"name": "formats", "value": ""}}));
        let all = v["completion"]["values"].as_array().unwrap().len();
        assert!(all > 3, "formats should answer: {v}");
        assert!(v["completion"]["total"].as_u64().unwrap() >= all as u64);
    }

    #[test]
    fn unknown_completion_targets_are_empty_not_errors() {
        let mut b = b();
        for params in [
            json!({"ref": {"type": "ref/prompt", "name": "nope"}, "argument": {"name": "x", "value": ""}}),
            json!({"ref": {"type": "ref/prompt", "name": "poster"}, "argument": {"name": "nope", "value": ""}}),
            json!({"ref": {"type": "ref/resource", "uri": "vectorcraft://nope/{id}"}, "argument": {"name": "id", "value": ""}}),
            json!({"ref": {"type": "ref/other"}, "argument": {"name": "x", "value": ""}}),
            json!({}),
        ] {
            let v = complete(b.as_mut(), &params);
            assert_eq!(v["completion"]["values"], json!([]), "{params}");
        }
    }
}
