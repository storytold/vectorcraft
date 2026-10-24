//! `vectorcraft-cli`: VectorCraft from the command line.
//!
//! ```text
//! vectorcraft-cli mcp [--connect 127.0.0.1:7979 | --headless] [--automation-read-root DIR]... [--automation-write-root DIR]...
//! vectorcraft-cli run [--in FILE] [--cmd id [--params '{json}'] | --commands FILE|-]... [--export out.svg]... [--scale 2]
//! vectorcraft-cli commands
//! vectorcraft-cli convert IN OUT [--scale 2] [--artboard 0 | --range 1-3,5] [--outline-text] [--decimals N] [--replace-lossy] [--editing-data] [--ai-version 7|pdf] [--hidden-objects layer|omit|visible] [--transparent-images flatten|omit]
//! vectorcraft-cli info FILE
//! vectorcraft-cli bench FILE [--size 2880x1800] [--iters 5]
//! vectorcraft-cli perf [--paths 50000] [--deep]
//! ```
// Denied, not forbidden: the font listers shared with the desktop app allow it for their
// DirectWrite (Windows) and CoreText (macOS) calls, and nothing else may.
#![deny(unsafe_code)]

use std::io::Write;
use std::path::Path;
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

/// The desktop app's CoreText font lister, shared: exports and MCP find the same fonts.
#[cfg(target_os = "macos")]
#[path = "../../vectorcraft/src/mac_fonts.rs"]
mod mac_fonts;
mod perf;
/// The desktop app's settings folder, shared: exports and MCP read the same Fonts folder.
#[path = "../../vectorcraft/src/prefs_dir.rs"]
mod prefs_dir;
/// The desktop app's DirectWrite font lister, shared: exports and MCP find the same fonts.
#[cfg(all(windows, not(target_vendor = "win7")))]
#[path = "../../vectorcraft/src/system_fonts.rs"]
mod system_fonts;

use serde_json::{Value, json};
use vectorcraft_engine::file_access::{self, AutomationRoots};
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
vectorcraft-cli — VectorCraft automation

USAGE:
  vectorcraft-cli mcp [--connect ADDR | --headless] [--automation-read-root DIR]... [--automation-write-root DIR]...
      Run the MCP server on stdio. Default: connect to a running app at 127.0.0.1:7979
      (vectorcraft --control 7979), falling back to a headless in-process session.
      --automation-read-root / --automation-write-root confine the files the agent's commands read
      (open, place, relink, library loads…) and write (save, export, package, library saves…) to
      DIR, links followed; each may be given more than once, a path inside any of its folders being
      allowed, and a kind left out grants none of its access. They imply --headless: a running app
      enforces its own (vectorcraft --control PORT --automation-read-root DIR …).

  vectorcraft-cli run [--in FILE] [--cmd ID [--params JSON] | --commands FILE|-]... [--export FILE]... [--scale N]
      Headless batch: open FILE (any readable format) or start a new document, run commands in
      order, export each FILE in the format its extension picks (see Writable formats; a .ait FILE
      is a template, written as file.saveAsTemplate writes it). --scale sizes raster exports
      (pixels per point, default 1); other formats leave it out. Prints one JSON result per step.
      --commands reads steps from FILE, or from standard input for -, where it stands among the
      --cmd and --export steps: a JSON array of {\"command\": ID, \"params\": {...}} (or \"id\" for
      \"command\"), or one such object per line (JSON Lines), at most 64 MiB and 100000 steps.
      Errors name the step, counting from 0 as \"$N\" references do.

  vectorcraft-cli commands
      Print the command catalogue as JSON.

  vectorcraft-cli convert IN OUT [--scale N] [--artboard I | --range R] [--outline-text] [--decimals N] [--replace-lossy] [--editing-data] [--ai-version 7|pdf] [--hidden-objects layer|omit|visible] [--transparent-images flatten|omit]
      Open IN (any readable format) and export OUT in the format its extension picks (see Writable
      formats). --scale sizes a raster OUT (pixels per point, default 1); other formats leave it
      out. --artboard is 0-based, --range 1-based (\"1-3,5\"); a PDF gets every artboard
      unless one of them is given, EPS the bounds of the art, the other formats the first artboard.
      A .ait OUT is a template of every artboard, written as file.saveAsTemplate writes it (not with
      --artboard or --range).
      Live effects are kept, and hidden layers and objects (written hidden in SVG and PSD), with
      SVG's data-* attributes; --outline-text writes SVG text as paths, --decimals sets SVG's coordinate
      precision (1–7, default 3). OUT may not be IN when reading
      IN left things out (hidden text, art or layers it could not read; the notes say which) unless
      --replace-lossy is given. --editing-data opens a .ai or an EPS with editing data from its editing copy
      even where that draws differently from its PDF part or page (which can be older), as long as
      it can be read. The
      result has the export's warnings, and importWarnings for what reading IN left out.
      --ai-version 7 writes a .ai as the legacy version 7 (layers, groups and type kept; one file
      per artboard, OUT-<artboard>.ai when there are several; default pdf, PDF-compatible), with
      --hidden-objects (default layer: a hidden layer above each layer's hidden objects; omit;
      visible), --transparent-images (default flatten, with the art under them; omit) and
      --outline-text (type as outlines instead of text).

  vectorcraft-cli info FILE
      Print a JSON summary: the import warnings (what didn't come in as it was, such as an EPS
      read from its preview and why), title, colour mode, units, artboards, object counts by
      kind, fonts.

  vectorcraft-cli bench FILE [--size WxH] [--iters N]
      Render FILE (any readable format) fitted to WxH (default 2880x1800) and print ms per frame
      (warm), multithreaded and single-threaded.

  vectorcraft-cli perf [--paths N] [--deep]
      Check the performance budgets (render, pan, hit test, save/load, SVG, Pathfinder, handle
      and segment drags) on a synthetic N-path document (default 50000). Exits non-zero if a
      budget is exceeded, unless the report says the machine is busy. --deep also times work with
      no budget yet and prints memory use (the working set on Windows), with the peak so far on
      Windows and Linux.
";

/// The usage text plus the formats `document.open` reads and `document.export` writes.
fn usage() -> String {
    use vectorcraft_engine::cmd::fileio::{OPEN_EXTS, export_extensions};
    format!("{USAGE}\nReadable formats: .{}\nWritable formats: .{}\n", OPEN_EXTS.join(", ."), export_extensions().join(", ."))
}

fn main() -> ExitCode {
    // Before any font lookup: the fonts font services load (#579).
    #[cfg(all(windows, not(target_vendor = "win7")))]
    system_fonts::install();
    #[cfg(target_os = "macos")]
    mac_fonts::install();
    prefs_dir::install_fonts();
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

/// A headless session that remembers what the user keeps between runs: the PDF presets saved, the
/// ICC profiles loaded and the plug-ins installed (#2168, #2174, #2541), in the folders next to the preferences that the desktop
/// app uses too. `VECTORCRAFT_NO_PREFS` and `VECTORCRAFT_CONFIG_DIR` apply as they do there.
fn with_stores(mut h: Headless) -> Headless {
    prefs_dir::attach_profile_store(&mut h.session);
    prefs_dir::attach_plugin_store(&mut h.session);
    prefs_dir::attach_pdf_preset_store(&mut h.session);
    h
}

fn mcp(args: &[String]) -> Result<(), String> {
    let mut connect: Option<String> = None;
    let mut headless = false;
    let (mut read_roots, mut write_roots): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        // The roots also come as `--flag=DIR`, as the other Craft apps take them.
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--automation-") => (f, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        match flag {
            "--connect" => connect = Some(it.next().cloned().ok_or("--connect needs an address")?),
            "--headless" => headless = true,
            "--automation-read-root" | "--automation-write-root" => {
                let dir = inline.or_else(|| it.next().cloned()).ok_or_else(|| format!("{flag} needs a folder"))?;
                if flag == "--automation-read-root" { read_roots.push(dir) } else { write_roots.push(dir) }
            }
            other => return Err(format!("unknown mcp option `{other}`")),
        }
    }
    if headless && connect.is_some() {
        return Err("use either --connect or --headless".into());
    }
    let read: Vec<&Path> = read_roots.iter().map(Path::new).collect();
    let write: Vec<&Path> = write_roots.iter().map(Path::new).collect();
    let roots = AutomationRoots::new(&read, &write)?;
    if roots.is_some() && connect.is_some() {
        return Err("--automation-read-root and --automation-write-root confine the headless server; a running app enforces its own roots:                     start it with `vectorcraft --control <port> --automation-read-root <dir> --automation-write-root <dir>`"
            .into());
    }
    let backend: Box<dyn Backend> = if let Some(roots) = roots {
        // Every thread of this process, not only the server's: nothing reaches past the roots.
        file_access::confine_process(roots.clone());
        eprintln!("vectorcraft-cli: files confined: {}", roots.describe());
        Box::new(Headless::with_document().with_automation_roots(Some(roots)))
    } else if headless {
        Box::new(with_stores(Headless::with_document()))
    } else if let Some(addr) = connect {
        // Explicit address: fail loudly if the app isn't there.
        Box::new(Remote::connect(&addr).map_err(|e| format!("cannot connect to {addr}: {e}"))?)
    } else {
        match Remote::connect(DEFAULT_ADDR) {
            Ok(r) => Box::new(r),
            Err(_) => Box::new(with_stores(Headless::with_document())),
        }
    };
    eprintln!("vectorcraft-cli: MCP server on stdio ({})", backend.describe());
    // The binary owns the logger, not the library: installing one here keeps an embedder that
    // uses `vectorcraft_mcp` free to bring its own. Silent until a client sends logging/setLevel.
    vectorcraft_mcp::logging::install();
    let stdin = std::io::stdin();
    // The server reads and writes from threads of its own, so it takes the handles, not locks held here.
    Server::new(backend).serve(std::io::BufReader::new(stdin), std::io::stdout()).map_err(|e| e.to_string())
}

fn commands() -> Result<(), String> {
    let mut h = Headless::new();
    let v = h.call("engine.commands", json!({}))?;
    outln!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

fn convert(args: &[String]) -> Result<(), String> {
    let mut files = vec![];
    let (mut scale, mut artboard, mut range, mut outline_text, mut replace_lossy, mut editing_data) =
        (1.0f64, None::<u64>, None::<String>, false, false, false);
    let mut decimals = None::<u64>;
    // Legacy `.ai` options, passed on as the export's own (`aiVersion`, `hiddenObjects`,
    // `transparentImages`); the engine checks their values.
    let mut legacy: Vec<(&str, String)> = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--scale" | "-s" => scale = it.next().and_then(|v| v.parse().ok()).ok_or("--scale needs a number")?,
            "--artboard" | "-a" => artboard = Some(it.next().and_then(|v| v.parse().ok()).ok_or("--artboard needs an index")?),
            "--range" | "-r" => range = Some(it.next().cloned().ok_or("--range needs artboards such as 1-3,5")?),
            "--outline-text" => outline_text = true,
            "--decimals" => decimals = Some(it.next().and_then(|v| v.parse::<u64>().ok()).ok_or("--decimals needs a number")?),
            "--replace-lossy" => replace_lossy = true,
            "--editing-data" => editing_data = true,
            "--ai-version" => legacy.push(("aiVersion", it.next().cloned().ok_or("--ai-version needs 7 or pdf")?)),
            "--hidden-objects" => legacy.push(("hiddenObjects", it.next().cloned().ok_or("--hidden-objects needs layer, omit or visible")?)),
            "--transparent-images" => legacy.push(("transparentImages", it.next().cloned().ok_or("--transparent-images needs flatten or omit")?)),
            f => files.push(f.to_string()),
        }
    }
    let [input, output] = <[String; 2]>::try_from(files).map_err(|_| "convert needs IN and OUT")?;
    let template = is_template(&output);
    if template && (artboard.is_some() || range.is_some()) {
        return Err("a .ait template holds every artboard: --artboard and --range don't apply".into());
    }
    let mut h = Headless::new();
    let opened = if editing_data {
        let params = json!({"path": input, "editingData": "always"});
        h.call("engine.execute", json!({"command": "document.open", "params": params})).map_err(|e| format!("open {input}: {e}"))?
    } else {
        h.call("app.open", json!({"path": input})).map_err(|e| format!("open {input}: {e}"))?
    };
    let (command, mut params) = if template {
        // Save as Template writes every artboard, with the .ai options.
        ("file.saveAsTemplate", json!({"path": output, "format": "ait", "outlineText": outline_text, "acknowledgeLoss": replace_lossy}))
    } else {
        // A conversion keeps hidden layers and objects, written hidden, where the format can (SVG,
        // PSD).
        let params = json!({"path": output, "artboard": artboard, "range": range, "outlineText": outline_text, "hiddenLayers": true, "acknowledgeLoss": replace_lossy});
        let mut params = with_pixel_scale(params, &output, scale);
        if let Some(decimals) = decimals {
            params["decimals"] = json!(decimals);
        }
        ("document.export", params)
    };
    for (key, value) in legacy {
        params[key] = json!(value);
    }
    let mut r = h.call("engine.execute", json!({"command": command, "params": params})).map_err(|e| format!("export {output}: {e}"))?;
    // What reading the input left out (a missing linked file, an unsupported filter…) matters as
    // much as what writing the output did.
    if let (Some(w), Some(out)) = (opened.get("warnings").filter(|w| w.as_array().is_some_and(|a| !a.is_empty())), r.as_object_mut()) {
        out.insert("importWarnings".into(), w.clone());
    }
    outln!("{r}");
    Ok(())
}

/// Is `path` a `.ait` template? `file.saveAsTemplate` writes it; exports don't.
fn is_template(path: &str) -> bool {
    vectorcraft_engine::cmd::fileio::format_for_name(path).is_some_and(|f| f.id == "ait")
}

/// The export `params` for `path`, with `--scale` (pixels per point) when its format is raster.
/// DXF has a `scale` of its own (drawing units per unit), which would resize the drawing and leave
/// its units unset.
fn with_pixel_scale(mut params: Value, path: &str, scale: f64) -> Value {
    if vectorcraft_engine::cmd::fileio::writable_format(None, Some(path)).is_ok_and(|f| f.raster) {
        params["scale"] = json!(scale);
    }
    params
}

fn info(args: &[String]) -> Result<(), String> {
    let file = args.first().ok_or("info needs a FILE")?;
    let mut h = Headless::new();
    let opened = h.call("app.open", json!({"path": file})).map_err(|e| format!("open {file}: {e}"))?;
    let base = h.call("engine.execute", json!({"command": "file.info", "params": {}}))?;
    let doc = h.session.doc().map_err(|e| e.to_string())?.doc.clone();
    let mut kinds: std::collections::BTreeMap<&'static str, usize> = Default::default();
    doc.walk(|n| *kinds.entry(n.kind_label()).or_default() += 1);
    let fonts = h.call("engine.execute", json!({"command": "text.fonts", "params": {}})).unwrap_or(Value::Null);
    let artboards: Vec<Value> =
        doc.artboards.iter().map(|a| json!({"name": a.name, "rect": [a.rect.x0, a.rect.y0, a.rect.width(), a.rect.height()]})).collect();
    // What didn't come in as it was (an EPS read from its preview says why).
    let v = json!({"file": file, "warnings": opened["warnings"], "info": base, "artboards": artboards, "kinds": kinds, "fonts": fonts});
    outln!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

enum Step {
    Cmd(String, Value),
    Export(String),
}

/// Most bytes `run --commands` reads from a file or standard input: the steps are untrusted input.
const MAX_COMMANDS_BYTES: u64 = 64 * 1024 * 1024;
/// Most steps one `run --commands` takes.
const MAX_COMMANDS_STEPS: usize = 100_000;

/// The text of `--commands source`: the file `source`, or standard input for `-`, refused past
/// [`MAX_COMMANDS_BYTES`].
fn read_commands(source: &str) -> Result<String, String> {
    use std::io::Read;
    let mut bytes = vec![];
    let read = if source == "-" {
        std::io::stdin().lock().take(MAX_COMMANDS_BYTES + 1).read_to_end(&mut bytes)
    } else {
        std::fs::File::open(source).and_then(|f| f.take(MAX_COMMANDS_BYTES + 1).read_to_end(&mut bytes))
    };
    read.map_err(|e| format!("--commands {source}: {e}"))?;
    if bytes.len() as u64 > MAX_COMMANDS_BYTES {
        return Err(format!("--commands {source}: more than {} MiB of steps", MAX_COMMANDS_BYTES / (1024 * 1024)));
    }
    String::from_utf8(bytes).map_err(|_| format!("--commands {source}: not UTF-8 text"))
}

/// The steps of `--commands source` from its `text`: a JSON array of `{"command": ID, "params":
/// {…}}` objects (`"id"` may stand for `"command"`, as in MCP's `command_batch`), or one such object
/// per line (JSON Lines, blank lines skipped). `first` is the run's index of the first of them
/// (the `N` of `"$N"`), which an error names with the entry's place in the text.
fn parse_commands(source: &str, text: &str, first: usize) -> Result<Vec<Step>, String> {
    // Notepad on Windows starts UTF-8 files with a byte order mark.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let too_many = || format!("--commands {source}: more than {MAX_COMMANDS_STEPS} steps");
    // Each entry with where it is in the text: its line for JSON Lines.
    let mut entries: Vec<(Option<usize>, Value)> = vec![];
    if text.trim_start().starts_with('[') {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("--commands {source}: {e}"))?;
        let Value::Array(items) = v else { return Err(format!("--commands {source}: not a JSON array")) };
        if items.len() > MAX_COMMANDS_STEPS {
            return Err(too_many());
        }
        entries.extend(items.into_iter().map(|v| (None, v)));
    } else {
        for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            if entries.len() == MAX_COMMANDS_STEPS {
                return Err(too_many());
            }
            let v = serde_json::from_str(line).map_err(|e| format!("--commands {source}: step {} (line {}): {e}", first + entries.len(), n + 1))?;
            entries.push((Some(n + 1), v));
        }
    }
    entries
        .into_iter()
        .enumerate()
        .map(|(k, (line, v))| {
            command_step(v).map_err(|e| {
                let place = line.map_or_else(|| format!("entry {}", k + 1), |l| format!("line {l}"));
                format!("--commands {source}: step {} ({place}): {e}", first + k)
            })
        })
        .collect()
}

/// The step of one `--commands` entry, `{"command": ID, "params": {…}}`.
fn command_step(v: Value) -> Result<Step, String> {
    let Value::Object(mut o) = v else { return Err("a step must be an object such as {\"command\": \"select.all\", \"params\": {}}".into()) };
    let id = match (o.remove("command"), o.remove("id")) {
        (Some(Value::String(id)), None) | (None, Some(Value::String(id))) => id,
        (Some(_), Some(_)) => return Err("give `command` or `id`, not both".into()),
        (None, None) => return Err("no `command`".into()),
        _ => return Err("`command` must be a string, a command id".into()),
    };
    let params = match o.remove("params") {
        None | Some(Value::Null) => json!({}),
        Some(p @ Value::Object(_)) => p,
        Some(_) => return Err(format!("`params` of {id} must be a JSON object")),
    };
    if let Some(key) = o.keys().next() {
        return Err(format!("unknown key `{key}`: a step has `command` (or `id`) and `params`"));
    }
    Ok(Step::Cmd(id, params))
}

fn run(args: &[String]) -> Result<(), String> {
    let mut input: Option<String> = None;
    let mut steps: Vec<Step> = vec![];
    // The step of the `--cmd` just given, which a `--params` sets.
    let mut params_for: Option<usize> = None;
    let mut stdin_read = false;
    let mut scale = 1.0;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |what: &str| it.next().cloned().ok_or_else(|| format!("{a} needs {what}"));
        match a.as_str() {
            "--in" | "-i" => input = Some(val("a file")?),
            "--cmd" | "-c" => {
                steps.push(Step::Cmd(val("a command id")?, json!({})));
                params_for = Some(steps.len() - 1);
            }
            "--params" | "-p" => {
                let raw = val("JSON")?;
                let p: Value = serde_json::from_str(&raw).map_err(|e| format!("--params {raw}: {e}"))?;
                if !p.is_object() {
                    return Err(format!("--params must be a JSON object, got {raw}"));
                }
                match params_for.and_then(|i| steps.get_mut(i)) {
                    Some(Step::Cmd(_, params)) => *params = p,
                    _ => return Err("--params must follow a --cmd".into()),
                }
            }
            "--commands" => {
                let source = val("a file, or - for standard input")?;
                if source == "-" && std::mem::replace(&mut stdin_read, true) {
                    return Err("--commands - is given twice: standard input is read once".into());
                }
                let text = read_commands(&source)?;
                let read = parse_commands(&source, &text, steps.len())?;
                steps.extend(read);
                params_for = None;
            }
            "--export" | "-o" => {
                steps.push(Step::Export(val("a file")?));
                params_for = None;
            }
            "--scale" | "-s" => {
                let raw = val("a number")?;
                scale = raw.parse::<f64>().map_err(|_| format!("--scale {raw}: not a number"))?;
            }
            other => return Err(format!("unknown run option `{other}`")),
        }
    }

    let mut h = with_stores(Headless::new());
    let mut out = std::io::stdout().lock();
    // A reader that stopped early (a closed pipe) isn't an error, and the batch goes on: its later
    // steps (an export…) still run, unprinted.
    let mut closed = false;
    let mut emit = |v: Value| {
        if closed {
            return Ok(());
        }
        match writeln!(out, "{v}") {
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                closed = true;
                Ok(())
            }
            r => r.map_err(|e| e.to_string()),
        }
    };
    if let Some(path) = &input {
        let r = h.call("app.open", json!({"path": path})).map_err(|e| format!("open {path}: {e}"))?;
        emit(json!({"step": "open", "path": path, "result": r}))?;
    } else if !matches!(steps.first(), Some(Step::Cmd(id, _)) if id == "file.new") {
        h.ensure_document();
    }
    // Each step's result, for later steps to refer to (`"$1.id"`, see `vectorcraft_engine::steps`).
    let mut results = vec![];
    for step in steps {
        // The step's index, as `"$N"` names its result.
        let n = results.len();
        let r = match step {
            Step::Cmd(id, params) => {
                let params = vectorcraft_engine::steps::resolve(&params, &results).map_err(|e| format!("step {n} ({id}): {e}"))?;
                let r = h.call("engine.execute", json!({"command": id, "params": params})).map_err(|e| format!("step {n} ({id}): {e}"))?;
                emit(json!({"step": "cmd", "command": id, "result": r}))?;
                r
            }
            Step::Export(path) => {
                let r = if is_template(&path) {
                    h.call("engine.execute", json!({"command": "file.saveAsTemplate", "params": {"path": path, "format": "ait"}}))
                } else {
                    h.call("app.export", with_pixel_scale(json!({"path": path}), &path, scale))
                };
                let r = r.map_err(|e| format!("step {n}: export {path}: {e}"))?;
                emit(json!({"step": "export", "result": r}))?;
                r
            }
        };
        results.push(r);
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
