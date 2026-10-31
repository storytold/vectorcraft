//! `commands`, `info` and `bench`: what the CLI reports about the app and about a file.

use std::io::Write;

use serde_json::{Value, json};
use vectorcraft_mcp::{Backend, Headless};

use crate::cli::{BenchArgs, InfoArgs};
use crate::error::CliError;

/// Prints the command catalogue as JSON.
pub(crate) fn commands(out: &mut dyn Write) -> Result<(), CliError> {
    let mut h = Headless::new();
    let v = h.call("engine.commands", json!({})).map_err(CliError::Engine)?;
    writeln!(out, "{}", serde_json::to_string_pretty(&v)?).map_err(CliError::Stdout)
}

/// Prints a JSON summary of `args.file`.
pub(crate) fn info(args: &InfoArgs, out: &mut dyn Write) -> Result<(), CliError> {
    let file = &args.file;
    let mut h = Headless::new();
    let opened = h.call("app.open", json!({"path": file})).map_err(|message| CliError::Open { path: file.clone(), message })?;
    let base = h.call("engine.execute", json!({"command": "file.info", "params": {}})).map_err(CliError::Engine)?;
    let doc = h.session.doc().map_err(|e| CliError::Engine(e.to_string()))?.doc.clone();
    let mut kinds: std::collections::BTreeMap<&'static str, usize> = Default::default();
    doc.walk(|n| *kinds.entry(n.kind_label()).or_default() += 1);
    let fonts = h.call("engine.execute", json!({"command": "text.fonts", "params": {}})).unwrap_or(Value::Null);
    let artboards: Vec<Value> =
        doc.artboards.iter().map(|a| json!({"name": a.name, "rect": [a.rect.x0, a.rect.y0, a.rect.width(), a.rect.height()]})).collect();
    // What didn't come in as it was (an EPS read from its preview says why).
    let v = json!({"file": file, "warnings": opened["warnings"], "info": base, "artboards": artboards, "kinds": kinds, "fonts": fonts});
    writeln!(out, "{}", serde_json::to_string_pretty(&v)?).map_err(CliError::Stdout)
}

/// Times rendering `args.file`, multithreaded and single-threaded.
pub(crate) fn bench(args: &BenchArgs, out: &mut dyn Write) -> Result<(), CliError> {
    let (file, iters) = (&args.file, args.iters);
    let (w, h) = (args.size.width, args.size.height);
    let mut hl = Headless::new();
    hl.call("app.open", json!({ "path": file })).map_err(|message| CliError::Open { path: file.clone(), message })?;
    let doc = hl.session.doc().map_err(|e| CliError::Engine(e.to_string()))?.doc.clone();
    let b = doc.artboards.first().map(|a| a.rect).ok_or(CliError::NoArtboard)?;
    let z = (w as f64 / b.width()).min(h as f64 / b.height()) * 0.95;
    let view = vectorcraft_geom::Affine::translate((w as f64 / 2.0, h as f64 / 2.0))
        * vectorcraft_geom::Affine::scale(z)
        * vectorcraft_geom::Affine::translate(-b.center().to_vec2());
    let opts = vectorcraft_render::RenderOptions::default();
    writeln!(out, "{file}: {} nodes, {w}x{h}", doc.layers.iter().map(|l| l.count()).sum::<usize>()).map_err(CliError::Stdout)?;
    for threads in [vectorcraft_render::default_threads(), 0] {
        let mut r = vectorcraft_render::Renderer::new();
        r.threads = threads;
        r.render(&doc, w, h, view, &opts);
        let t = std::time::Instant::now();
        for _ in 0..iters {
            r.render(&doc, w, h, view, &opts);
        }
        writeln!(
            out,
            "  threads {threads}: {:.1} ms/frame (drawn {}, culled {})",
            t.elapsed().as_secs_f64() * 1000.0 / f64::from(iters),
            r.stats.drawn,
            r.stats.culled
        )
        .map_err(CliError::Stdout)?;
    }
    Ok(())
}
