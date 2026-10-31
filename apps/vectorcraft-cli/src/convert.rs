//! `vectorcraft-cli convert`: open a file and export it in another format.

use std::io::Write;

use serde_json::{Value, json};
use vectorcraft_mcp::{Backend, Headless};

use crate::cli::ConvertArgs;
use crate::error::CliError;

/// Is `path` a `.ait` template? `file.saveAsTemplate` writes it; exports don't.
pub(crate) fn is_template(path: &str) -> bool {
    vectorcraft_engine::cmd::fileio::format_for_name(path).is_some_and(|f| f.id == "ait")
}

/// The export `params` for `path`, with `--scale` (pixels per point) when its format is raster.
/// DXF has a `scale` of its own (drawing units per unit), which would resize the drawing and leave
/// its units unset.
pub(crate) fn with_pixel_scale(mut params: Value, path: &str, scale: f64) -> Value {
    if vectorcraft_engine::cmd::fileio::writable_format(None, Some(path)).is_ok_and(|f| f.raster) {
        params["scale"] = json!(scale);
    }
    params
}

pub(crate) fn convert(args: &ConvertArgs, out: &mut dyn Write) -> Result<(), CliError> {
    let (input, output) = (&args.input, &args.output);
    let template = is_template(output);
    let mut h = Headless::new();
    let opened = if args.editing_data {
        let params = json!({"path": input, "editingData": "always"});
        h.call("engine.execute", json!({"command": "document.open", "params": params}))
    } else {
        h.call("app.open", json!({"path": input}))
    }
    .map_err(|message| CliError::Open { path: input.clone(), message })?;
    let (command, mut params) = if template {
        // Save as Template writes every artboard, with the .ai options.
        ("file.saveAsTemplate", json!({"path": output, "format": "ait", "outlineText": args.outline_text, "acknowledgeLoss": args.replace_lossy}))
    } else {
        // A conversion keeps hidden layers and objects, written hidden, where the format can (SVG,
        // PSD).
        let params = json!({
            "path": output, "artboard": args.artboard, "range": args.range, "outlineText": args.outline_text,
            "hiddenLayers": true, "acknowledgeLoss": args.replace_lossy,
        });
        let mut params = with_pixel_scale(params, output, args.scale);
        if let Some(decimals) = args.decimals {
            params["decimals"] = json!(decimals);
        }
        if vectorcraft_engine::cmd::fileio::format_for_name(output).is_some_and(|f| f.id == "pdf") {
            params["preserveEditing"] = json!(args.preserve_editing);
        }
        ("document.export", params)
    };
    // Legacy `.ai` options, passed on as the export's own (`aiVersion`, `hiddenObjects`,
    // `transparentImages`).
    if let Some(v) = args.ai_version {
        params["aiVersion"] = json!(v.param());
    }
    if let Some(v) = args.hidden_objects {
        params["hiddenObjects"] = json!(v.param());
    }
    if let Some(v) = args.transparent_images {
        params["transparentImages"] = json!(v.param());
    }
    let mut r = h
        .call("engine.execute", json!({"command": command, "params": params}))
        .map_err(|message| CliError::Export { path: output.clone(), message })?;
    // What reading the input left out (a missing linked file, an unsupported filter…) matters as
    // much as what writing the output did.
    if let (Some(w), Some(obj)) = (opened.get("warnings").filter(|w| w.as_array().is_some_and(|a| !a.is_empty())), r.as_object_mut()) {
        obj.insert("importWarnings".into(), w.clone());
    }
    if args.preserve_editing
        && vectorcraft_engine::cmd::fileio::format_for_name(output).is_some_and(|f| f.id == "pdf")
        && let Some(obj) = r.as_object_mut()
    {
        // The export leaves the attachment out (with this warning) for --artboard or a --range
        // without every artboard; otherwise it writes it. Read that from the result, not the file.
        let skipped = vectorcraft_engine::cmd::fileio::pdf::EDITING_NEEDS_EVERY_ARTBOARD;
        let embedded = !obj.get("warnings").and_then(Value::as_array).is_some_and(|w| w.iter().any(|w| w.as_str() == Some(skipped)));
        if embedded {
            let note = "The PDF embeds the native VectorCraft document as an attachment (preserveEditing).";
            if let Some(Value::Array(w)) = obj.get_mut("warnings") {
                if !w.iter().any(|w| w.as_str() == Some(note)) {
                    w.push(json!(note));
                }
            } else {
                obj.insert("warnings".into(), json!([note]));
            }
        }
    }
    writeln!(out, "{r}").map_err(CliError::Stdout)
}
