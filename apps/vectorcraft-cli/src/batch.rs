//! `vectorcraft-cli run`: open a file or start a document, run commands in order, export.

use std::io::{Read, Write};

use serde_json::{Value, json};
use vectorcraft_mcp::{Backend, Headless};

use crate::cli::{RunArgs, RunItem};
use crate::convert::{is_template, with_pixel_scale};
use crate::error::{CliError, CommandsError, StepError};
use crate::mcp::with_stores;

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
fn read_commands(source: &str) -> Result<String, CommandsError> {
    let mut bytes = vec![];
    let read = if source == "-" {
        std::io::stdin().lock().take(MAX_COMMANDS_BYTES + 1).read_to_end(&mut bytes)
    } else {
        std::fs::File::open(source).and_then(|f| f.take(MAX_COMMANDS_BYTES + 1).read_to_end(&mut bytes))
    };
    read.map_err(|error| CommandsError::Read { file: source.to_string(), error })?;
    if bytes.len() as u64 > MAX_COMMANDS_BYTES {
        return Err(CommandsError::TooLarge { file: source.to_string(), mib: MAX_COMMANDS_BYTES / (1024 * 1024) });
    }
    String::from_utf8(bytes).map_err(|_| CommandsError::NotUtf8 { file: source.to_string() })
}

/// The steps of `--commands source` from its `text`: a JSON array of `{"command": ID, "params":
/// {…}}` objects (`"id"` may stand for `"command"`, as in MCP's `command_batch`), or one such object
/// per line (JSON Lines, blank lines skipped). `first` is the run's index of the first of them
/// (the `N` of `"$N"`), which an error names with the entry's place in the text.
fn parse_commands(source: &str, text: &str, first: usize) -> Result<Vec<Step>, CommandsError> {
    // Notepad on Windows starts UTF-8 files with a byte order mark.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let too_many = || CommandsError::TooManySteps { file: source.to_string(), max: MAX_COMMANDS_STEPS };
    // Each entry with where it is in the text: its line for JSON Lines.
    let mut entries: Vec<(Option<usize>, Value)> = vec![];
    if text.trim_start().starts_with('[') {
        let v: Value = serde_json::from_str(text).map_err(|error| CommandsError::Json { file: source.to_string(), error })?;
        let Value::Array(items) = v else { return Err(CommandsError::NotArray { file: source.to_string() }) };
        if items.len() > MAX_COMMANDS_STEPS {
            return Err(too_many());
        }
        entries.extend(items.into_iter().map(|v| (None, v)));
    } else {
        for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
            if entries.len() == MAX_COMMANDS_STEPS {
                return Err(too_many());
            }
            let v = serde_json::from_str(line).map_err(|error| CommandsError::Line {
                file: source.to_string(),
                step: first + entries.len(),
                line: n + 1,
                error,
            })?;
            entries.push((Some(n + 1), v));
        }
    }
    entries
        .into_iter()
        .enumerate()
        .map(|(k, (line, v))| {
            command_step(v).map_err(|error| {
                let place = line.map_or_else(|| format!("entry {}", k + 1), |l| format!("line {l}"));
                CommandsError::Step { file: source.to_string(), step: first + k, place, error }
            })
        })
        .collect()
}

/// The step of one `--commands` entry, `{"command": ID, "params": {…}}`.
fn command_step(v: Value) -> Result<Step, StepError> {
    let Value::Object(mut o) = v else { return Err(StepError::NotObject) };
    let id = match (o.remove("command"), o.remove("id")) {
        (Some(Value::String(id)), None) | (None, Some(Value::String(id))) => id,
        (Some(_), Some(_)) => return Err(StepError::CommandAndId),
        (None, None) => return Err(StepError::NoCommand),
        _ => return Err(StepError::CommandNotString),
    };
    let params = match o.remove("params") {
        None | Some(Value::Null) => json!({}),
        Some(p @ Value::Object(_)) => p,
        Some(_) => return Err(StepError::ParamsNotObject(id)),
    };
    if let Some(key) = o.keys().next() {
        return Err(StepError::UnknownKey(key.clone()));
    }
    Ok(Step::Cmd(id, params))
}

/// The steps of the command line, with the files of `--commands` read: all of it before anything
/// runs, so a malformed file stops the batch with nothing done.
fn steps(items: &[RunItem]) -> Result<Vec<Step>, CommandsError> {
    let mut steps: Vec<Step> = vec![];
    for item in items {
        match item {
            RunItem::Cmd(id) => steps.push(Step::Cmd(id.clone(), json!({}))),
            // `cli::parse` has checked that a `--params` follows a `--cmd` and sets it once.
            RunItem::Params(p) => {
                if let Some(Step::Cmd(_, params)) = steps.last_mut() {
                    *params = p.clone();
                }
            }
            RunItem::Commands(source) => {
                let text = read_commands(source)?;
                let read = parse_commands(source, &text, steps.len())?;
                steps.extend(read);
            }
            RunItem::Export(path) => steps.push(Step::Export(path.clone())),
        }
    }
    Ok(steps)
}

/// Prints one JSON result per line. A reader that stopped early (a closed pipe) isn't an error,
/// and the batch goes on: its later steps (an export…) still run, unprinted.
struct Emitter<'a> {
    out: &'a mut dyn Write,
    closed: bool,
}

impl Emitter<'_> {
    fn emit(&mut self, v: &Value) -> Result<(), CliError> {
        if self.closed {
            return Ok(());
        }
        match writeln!(self.out, "{v}") {
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                self.closed = true;
                Ok(())
            }
            r => r.map_err(CliError::Stdout),
        }
    }
}

pub(crate) fn run(args: &RunArgs, out: &mut dyn Write) -> Result<(), CliError> {
    let steps = steps(&args.items)?;
    let mut h = with_stores(Headless::new());
    let mut emitter = Emitter { out, closed: false };
    if let Some(path) = &args.input {
        let r = h.call("app.open", json!({"path": path})).map_err(|message| CliError::Open { path: path.clone(), message })?;
        emitter.emit(&json!({"step": "open", "path": path, "result": r}))?;
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
                let params =
                    vectorcraft_engine::steps::resolve(&params, &results).map_err(|message| CliError::Step { step: n, id: id.clone(), message })?;
                let r = h.call("engine.execute", json!({"command": id, "params": params})).map_err(|message| CliError::Step {
                    step: n,
                    id: id.clone(),
                    message,
                })?;
                emitter.emit(&json!({"step": "cmd", "command": id, "result": r}))?;
                r
            }
            Step::Export(path) => {
                let r = if is_template(&path) {
                    h.call("engine.execute", json!({"command": "file.saveAsTemplate", "params": {"path": path, "format": "ait"}}))
                } else {
                    h.call("app.export", with_pixel_scale(json!({"path": path}), &path, args.scale))
                };
                let r = r.map_err(|message| CliError::StepExport { step: n, path: path.clone(), message })?;
                emitter.emit(&json!({"step": "export", "result": r}))?;
                r
            }
        };
        results.push(r);
    }
    Ok(())
}
