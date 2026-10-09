//! `vectorcraft-cli`: VectorCraft from the command line.
//!
//! ```text
//! vectorcraft-cli mcp [--connect 127.0.0.1:7979 | --headless]
//! vectorcraft-cli run [--in FILE] [--cmd id [--params '{json}']]... [--export out.svg|.png|.pdf|.jpg|.webp|.vectorcraft]... [--scale 2]
//! vectorcraft-cli commands
//! vectorcraft-cli convert IN OUT [--scale 2] [--artboard 0 | --range 1-3,5] [--outline-text]
//! vectorcraft-cli info FILE
//! vectorcraft-cli bench FILE [--size 2880x1800] [--iters 5]
//! vectorcraft-cli perf [--paths 50000]
//! ```
#![forbid(unsafe_code)]

use std::io::Write;
use std::process::ExitCode;

/// `println!` / `print!` that end the program quietly when stdout is closed
/// (`vectorcraft-cli commands | head`) instead of panicking with "failed printing to stdout:
/// Broken pipe (os error 32)".
macro_rules! outln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = writeln!(std::io::stdout(), $($arg)*) {
            $crate::stdout_failed(e);
        }
    }};
}
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(e) = write!(std::io::stdout(), $($arg)*) {
            $crate::stdout_failed(e);
        }
    }};
}

mod perf;

use serde_json::{Value, json};
use vectorcraft_mcp::{Backend, DEFAULT_ADDR, Headless, Remote, Server};

/// stdout went away. A reader that stopped early (a closed pipe) ends the program quietly, as
/// ripgrep does; any other write error is reported.
fn stdout_failed(e: std::io::Error) -> ! {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    eprintln!("vectorcraft-cli: can't write to stdout: {e}");
    std::process::exit(1);
}

const USAGE: &str = "\
vectorcraft-cli — Vector W3K2 automation

USAGE:
  vectorcraft-cli mcp [--connect ADDR | --headless]
      Run the MCP server on stdio. Default: connect to a running app at 127.0.0.1:7979
      (vectorcraft --control 7979), falling back to a headless in-process session.

  vectorcraft-cli run [--in FILE] [--cmd ID [--params JSON]]... [--export FILE]... [--scale N]
      Headless batch: open FILE (any readable format) or start a new document, run commands in
      order, export (.svg, .png, .pdf, .jpg, .webp, .vectorcraft by extension). Prints one JSON result per step.

  vectorcraft-cli commands
      Print the command catalogue as JSON.

  vectorcraft-cli convert IN OUT [--scale N] [--artboard I | --range R] [--outline-text]
      Open IN (any readable format) and export OUT by extension (.svg, .pdf, .png, .jpg, .webp,
      .vectorcraft). --artboard is 0-based, --range 1-based (\"1-3,5\"); a PDF gets every artboard
      unless one of them is given, the other formats the first. Live effects are kept;
      --outline-text writes SVG text as paths.

  vectorcraft-cli info FILE
      Print a JSON summary: title, colour mode, units, artboards, object counts by kind, fonts.

  vectorcraft-cli bench FILE [--size WxH] [--iters N]
      Render FILE (any readable format) fitted to WxH (default 2880x1800) and print ms per frame
      (warm), multithreaded and single-threaded.

  vectorcraft-cli perf [--paths N]
      Check the performance budgets (render, pan, hit test, save/load, SVG, Pathfinder) on a
      synthetic N-path document (default 50000). Exits non-zero if a budget is exceeded.
";

/// The usage text plus the formats `document.open` reads.
fn usage() -> String {
    format!("{USAGE}\nReadable formats: .{}\n", vectorcraft_engine::cmd::fileio::OPEN_EXTS.join(", ."))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("mcp") => mcp(&args[1..]),
        Some("run") => run(&args[1..]),
        Some("commands") => commands(),
        Some("convert") => convert(&args[1..]),
        Some("info") => info(&args[1..]),
        Some("bench") => bench(&args[1..]),
        Some("perf") => perf::run(&args[1..]),
        Some("-h" | "--help" | "help") | None => {
            out!("{}", usage());
            Ok(())
        }
        Some("-V" | "--version") => {
            outln!("vectorcraft-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => Err(format!("unknown subcommand `{other}`\n\n{}", usage())),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vectorcraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

fn mcp(args: &[String]) -> Result<(), String> {
    let mut connect: Option<String> = None;
    let mut headless = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--connect" => connect = Some(it.next().cloned().ok_or("--connect needs an address")?),
            "--headless" => headless = true,
            other => return Err(format!("unknown mcp option `{other}`")),
        }
    }
    if headless && connect.is_some() {
        return Err("use either --connect or --headless".into());
    }
    let backend: Box<dyn Backend> = if headless {
        Box::new(Headless::with_document())
    } else if let Some(addr) = connect {
        // Explicit address: fail loudly if the app isn't there.
        Box::new(Remote::connect(&addr).map_err(|e| format!("cannot connect to {addr}: {e}"))?)
    } else {
        match Remote::connect(DEFAULT_ADDR) {
            Ok(r) => Box::new(r),
            Err(_) => Box::new(Headless::with_document()),
        }
    };
    eprintln!("vectorcraft-cli: MCP server on stdio ({})", backend.describe());
    // The binary owns the logger, not the library: installing one here keeps an embedder that
    // uses `vectorcraft_mcp` free to bring its own. Silent until a client sends logging/setLevel.
    vectorcraft_mcp::logging::install();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    Server::new(backend).serve(stdin.lock(), stdout.lock()).map_err(|e| e.to_string())
}

fn commands() -> Result<(), String> {
    let mut h = Headless::new();
    let v = h.call("engine.commands", json!({}))?;
    outln!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

fn convert(args: &[String]) -> Result<(), String> {
    let mut files = vec![];
    let (mut scale, mut artboard, mut range, mut outline_text) = (1.0f64, None::<u64>, None::<String>, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--scale" | "-s" => scale = it.next().and_then(|v| v.parse().ok()).ok_or("--scale needs a number")?,
            "--artboard" | "-a" => artboard = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--artboard needs an index")?),
            "--range" | "-r" => range = Some(it.next().cloned().ok_or("--range needs artboards such as 1-3,5")?),
            "--outline-text" => outline_text = true,
            f => files.push(f.to_string()),
        }
    }
    let [input, output] = <[String; 2]>::try_from(files).map_err(|_| "convert needs IN and OUT")?;
    let mut h = Headless::new();
    h.call("app.open", json!({"path": input})).map_err(|e| format!("open {input}: {e}"))?;
    let r = h
        .call(
            "engine.execute",
            json!({"command": "document.export", "params": {"path": output, "scale": scale, "artboard": artboard, "range": range, "outlineText": outline_text}}),
        )
        .map_err(|e| format!("export {output}: {e}"))?;
    outln!("{r}");
    Ok(())
}

fn info(args: &[String]) -> Result<(), String> {
    let file = args.first().ok_or("info needs a FILE")?;
    let mut h = Headless::new();
    h.call("app.open", json!({"path": file})).map_err(|e| format!("open {file}: {e}"))?;
    let base = h.call("engine.execute", json!({"command": "file.info", "params": {}}))?;
    let doc = h.session.doc().map_err(|e| e.to_string())?.doc.clone();
    let mut kinds: std::collections::BTreeMap<&'static str, usize> = Default::default();
    doc.walk(|n| *kinds.entry(n.kind_label()).or_default() += 1);
    let fonts = h.call("engine.execute", json!({"command": "text.fonts", "params": {}})).unwrap_or(Value::Null);
    let artboards: Vec<Value> =
        doc.artboards.iter().map(|a| json!({"name": a.name, "rect": [a.rect.x0, a.rect.y0, a.rect.width(), a.rect.height()]})).collect();
    let v = json!({"file": file, "info": base, "artboards": artboards, "kinds": kinds, "fonts": fonts});
    outln!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

enum Step {
    Cmd(String, Value),
    Export(String),
}

fn run(args: &[String]) -> Result<(), String> {
    let mut input: Option<String> = None;
    let mut steps: Vec<Step> = vec![];
    let mut scale = 1.0;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |what: &str| it.next().cloned().ok_or_else(|| format!("{a} needs {what}"));
        match a.as_str() {
            "--in" | "-i" => input = Some(val("a file")?),
            "--cmd" | "-c" => steps.push(Step::Cmd(val("a command id")?, json!({}))),
            "--params" | "-p" => {
                let raw = val("JSON")?;
                let p: Value = serde_json::from_str(&raw).map_err(|e| format!("--params {raw}: {e}"))?;
                if !p.is_object() {
                    return Err(format!("--params must be a JSON object, got {raw}"));
                }
                match steps.last_mut() {
                    Some(Step::Cmd(_, params)) => *params = p,
                    _ => return Err("--params must follow a --cmd".into()),
                }
            }
            "--export" | "-o" => steps.push(Step::Export(val("a file")?)),
            "--scale" | "-s" => {
                let raw = val("a number")?;
                scale = raw.parse::<f64>().map_err(|_| format!("--scale {raw}: not a number"))?;
            }
            other => return Err(format!("unknown run option `{other}`")),
        }
    }

    let mut h = Headless::new();
    let mut out = std::io::stdout().lock();
    let mut emit = |v: Value| writeln!(out, "{v}").map_err(|e| e.to_string());
    if let Some(path) = &input {
        let r = h.call("app.open", json!({"path": path})).map_err(|e| format!("open {path}: {e}"))?;
        emit(json!({"step": "open", "path": path, "result": r}))?;
    } else if !matches!(steps.first(), Some(Step::Cmd(id, _)) if id == "file.new") {
        h.ensure_document();
    }
    for step in steps {
        match step {
            Step::Cmd(id, params) => {
                let r = h.call("engine.execute", json!({"command": id, "params": params})).map_err(|e| format!("{id}: {e}"))?;
                emit(json!({"step": "cmd", "command": id, "result": r}))?;
            }
            Step::Export(path) => {
                let r = h.call("app.export", json!({"path": path, "scale": scale})).map_err(|e| format!("export {path}: {e}"))?;
                emit(json!({"step": "export", "result": r}))?;
            }
        }
    }
    Ok(())
}

fn bench(args: &[String]) -> Result<(), String> {
    let mut file = None;
    let (mut w, mut h, mut iters) = (2880u32, 1800u32, 5u32);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (a, b) = v.split_once('x').ok_or("--size needs WxH")?;
                w = a.parse().map_err(|_| "bad width")?;
                h = b.parse().map_err(|_| "bad height")?;
            }
            "--iters" => iters = it.next().and_then(|v| v.parse().ok()).ok_or("--iters needs a number")?,
            f if file.is_none() => file = Some(f.to_string()),
            other => return Err(format!("unknown bench option `{other}`")),
        }
    }
    let file = file.ok_or("bench needs a FILE")?;
    let mut hl = Headless::new();
    hl.call("app.open", json!({ "path": file })).map_err(|e| format!("open {file}: {e}"))?;
    let doc = hl.session.doc().map_err(|e| e.to_string())?.doc.clone();
    let b = doc.artboards.first().map(|a| a.rect).ok_or("document has no artboard")?;
    let z = (w as f64 / b.width()).min(h as f64 / b.height()) * 0.95;
    let view = vectorcraft_geom::Affine::translate((w as f64 / 2.0, h as f64 / 2.0))
        * vectorcraft_geom::Affine::scale(z)
        * vectorcraft_geom::Affine::translate(-b.center().to_vec2());
    let opts = vectorcraft_render::RenderOptions::default();
    outln!("{file}: {} nodes, {w}x{h}", doc.layers.iter().map(|l| l.count()).sum::<usize>());
    for threads in [vectorcraft_render::default_threads(), 0] {
        let mut r = vectorcraft_render::Renderer::new();
        r.threads = threads;
        r.render(&doc, w, h, view, &opts);
        let t = std::time::Instant::now();
        for _ in 0..iters {
            r.render(&doc, w, h, view, &opts);
        }
        outln!(
            "  threads {threads}: {:.1} ms/frame (drawn {}, culled {})",
            t.elapsed().as_secs_f64() * 1000.0 / iters as f64,
            r.stats.drawn,
            r.stats.culled
        );
    }
    Ok(())
}
