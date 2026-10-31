//! The command line, as clap types (RFC-0012). Values are parsed into their types here, so a bad
//! one is a usage error naming its flag (exit 2) and the handlers never see it. A flag a
//! subcommand doesn't take is a usage error too, so a typo such as `--artbord` can't be silently
//! ignored, and so is a repeated one (`--scale 1 --scale 2`) where repeating means nothing. A
//! value starting with `-` is written `--cmd=-x` (or `./-x`), so a value flag never takes the
//! next flag as its value; negative numbers are kept where they mean something.

use std::ffi::OsString;

use clap::error::ErrorKind;
use clap::{ArgAction, ArgMatches, Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use serde_json::Value;

/// vectorcraft-cli: VectorCraft automation
#[derive(Parser)]
#[command(name = "vectorcraft-cli", version, arg_required_else_help = true)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    Mcp(McpArgs),
    Run(RunArgs),
    /// Print the command catalogue as JSON
    Commands,
    Convert(ConvertArgs),
    Info(InfoArgs),
    Bench(BenchArgs),
    Perf(PerfArgs),
}

/// Run the MCP server on stdio
///
/// By default it connects to a running app at 127.0.0.1:7979 (`vectorcraft --control 7979`), falling
/// back to a headless in-process session.
///
/// --automation-read-root and --automation-write-root confine the files the agent's commands read
/// (open, place, relink, library loads…) and write (save, export, package, library saves…) to DIR,
/// links followed; each may be given more than once, a path inside any of its folders being
/// allowed, and a kind left out grants none of its access. They imply --headless: a running app
/// enforces its own (`vectorcraft --control PORT --automation-read-root DIR …`).
#[derive(Args)]
pub(crate) struct McpArgs {
    /// Connect to the app listening at ADDR, failing if it isn't there
    #[arg(long, value_name = "ADDR", conflicts_with_all = ["headless", "automation_read_root", "automation_write_root"])]
    pub(crate) connect: Option<String>,
    /// Run an in-process session instead of connecting to the app
    #[arg(long)]
    pub(crate) headless: bool,
    /// A folder the session may read from; repeat for several
    #[arg(long, value_name = "DIR", action = ArgAction::Append)]
    pub(crate) automation_read_root: Vec<String>,
    /// A folder the session may write to; repeat for several
    #[arg(long, value_name = "DIR", action = ArgAction::Append)]
    pub(crate) automation_write_root: Vec<String>,
}

/// Headless batch: open a file or start a new document, run commands in order, export
///
/// Opens --in FILE (any readable format) or starts a new document, runs the steps in the order
/// given, and exports each --export FILE in the format its extension picks (see Writable formats;
/// a .ait FILE is a template, written as file.saveAsTemplate writes it). Prints one JSON result
/// per step.
///
/// --commands reads steps from FILE, or from standard input for -, where it stands among the --cmd
/// and --export steps: a JSON array of {"command": ID, "params": {...}} (or "id" for "command"),
/// or one such object per line (JSON Lines), at most 64 MiB and 100000 steps. Errors name the step,
/// counting from 0 as "$N" references do.
#[derive(Args)]
pub(crate) struct RunArgs {
    /// The document to open first (any readable format)
    #[arg(short = 'i', long = "in", value_name = "FILE")]
    pub(crate) input: Option<String>,
    /// A command to run, by id; repeat for several
    #[arg(short = 'c', long, value_name = "ID", action = ArgAction::Append)]
    pub(crate) cmd: Vec<String>,
    /// JSON parameters (an object) for the --cmd before it
    #[arg(short = 'p', long, value_name = "JSON", action = ArgAction::Append, value_parser = json_object)]
    pub(crate) params: Vec<Value>,
    /// Steps from a file, or from standard input for -; repeat for several files
    #[arg(long, value_name = "FILE|-", action = ArgAction::Append)]
    pub(crate) commands: Vec<String>,
    /// Export the document to FILE, in the format its extension picks; repeat for several
    #[arg(short = 'o', long, value_name = "FILE", action = ArgAction::Append)]
    pub(crate) export: Vec<String>,
    /// Size raster exports, in pixels per point (default 1); other formats leave it out
    #[arg(short = 's', long, value_name = "N", default_value_t = 1.0, value_parser = positive, allow_negative_numbers = true)]
    pub(crate) scale: f64,
    /// `--cmd`, `--params`, `--commands` and `--export` in command-line order, set by [`parse`].
    #[arg(skip)]
    pub(crate) items: Vec<RunItem>,
}

/// One `--cmd`, `--params`, `--commands` or `--export` of `run`, in command-line order.
pub(crate) enum RunItem {
    Cmd(String),
    Params(Value),
    Commands(String),
    Export(String),
}

/// Open IN and export OUT in the format its extension picks
///
/// IN is any readable format (see Readable formats) and OUT any writable one. --scale sizes a
/// raster OUT (pixels per point, default 1); other formats leave it out. --artboard is 0-based,
/// --range 1-based ("1-3,5"); a PDF gets every artboard unless one of them is given, EPS the
/// bounds of the art, the other formats the first artboard. A .ait OUT is a template of every
/// artboard, written as file.saveAsTemplate writes it (not with --artboard or --range).
///
/// Live effects are kept, and hidden layers and objects (written hidden in SVG and PSD), with SVG's
/// data-* attributes; --outline-text writes SVG text as paths, --decimals sets SVG's coordinate
/// precision (1–7, default 3). OUT may not be IN when reading IN left things out (hidden text, art
/// or layers it could not read; the notes say which) unless --replace-lossy is given.
/// --editing-data opens a .ai or an EPS with editing data from its editing copy even where that
/// draws differently from its PDF part or page (which can be older), as long as it can be read. A
/// PDF OUT omits the embedded native document unless --preserve-editing is given (the app's Save As
/// PDF still defaults to keeping it). With --preserve-editing, when the attachment is written the
/// result's warnings say so (it is left out, with its own warning, for --artboard or a --range
/// without every artboard).
///
/// The result has the export's warnings, and importWarnings for what reading IN left out.
///
/// --ai-version 7 writes a .ai as the legacy version 7 (layers, groups and type kept; one file
/// per artboard, OUT-<artboard>.ai when there are several; default pdf, PDF-compatible), with
/// --hidden-objects (default layer: a hidden layer above each layer's hidden objects; omit;
/// visible), --transparent-images (default flatten, with the art under them; omit) and
/// --outline-text (type as outlines instead of text).
#[derive(Args)]
pub(crate) struct ConvertArgs {
    /// The file to read
    #[arg(value_name = "IN")]
    pub(crate) input: String,
    /// The file to write; its extension picks the format
    #[arg(value_name = "OUT")]
    pub(crate) output: String,
    /// Size a raster OUT, in pixels per point (default 1)
    #[arg(short = 's', long, value_name = "N", default_value_t = 1.0, value_parser = positive, allow_negative_numbers = true)]
    pub(crate) scale: f64,
    /// Export only this artboard (0-based)
    #[arg(short = 'a', long, value_name = "I", conflicts_with = "range")]
    pub(crate) artboard: Option<u64>,
    /// Export only these artboards (1-based), such as 1-3,5
    #[arg(short = 'r', long, value_name = "R")]
    pub(crate) range: Option<String>,
    /// Write SVG text as paths, and .ai type as outlines
    #[arg(long)]
    pub(crate) outline_text: bool,
    /// SVG coordinate precision, 1-7 (default 3)
    #[arg(long, value_name = "N")]
    pub(crate) decimals: Option<u64>,
    /// Allow an OUT that is IN when reading IN left things out
    #[arg(long)]
    pub(crate) replace_lossy: bool,
    /// Open a .ai or EPS from its editing copy
    #[arg(long)]
    pub(crate) editing_data: bool,
    /// Embed the native document in a PDF OUT
    #[arg(long)]
    pub(crate) preserve_editing: bool,
    /// Write a .ai as the legacy version 7, or as a PDF-compatible file (the default)
    #[arg(long, value_name = "7|pdf", value_enum)]
    pub(crate) ai_version: Option<AiVersion>,
    /// What a legacy .ai does with hidden objects (default: layer)
    #[arg(long, value_name = "layer|omit|visible", value_enum)]
    pub(crate) hidden_objects: Option<HiddenObjects>,
    /// What a legacy .ai does with transparent images (default: flatten)
    #[arg(long, value_name = "flatten|omit", value_enum)]
    pub(crate) transparent_images: Option<TransparentImages>,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum AiVersion {
    #[value(name = "7")]
    V7,
    Pdf,
}

impl AiVersion {
    /// The value of the export's `aiVersion` parameter.
    pub(crate) fn param(self) -> &'static str {
        match self {
            Self::V7 => "7",
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum HiddenObjects {
    Layer,
    Omit,
    Visible,
}

impl HiddenObjects {
    /// The value of the export's `hiddenObjects` parameter.
    pub(crate) fn param(self) -> &'static str {
        match self {
            Self::Layer => "layer",
            Self::Omit => "omit",
            Self::Visible => "visible",
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum TransparentImages {
    Flatten,
    Omit,
}

impl TransparentImages {
    /// The value of the export's `transparentImages` parameter.
    pub(crate) fn param(self) -> &'static str {
        match self {
            Self::Flatten => "flatten",
            Self::Omit => "omit",
        }
    }
}

/// Print a JSON summary of a file
///
/// The import warnings (what didn't come in as it was, such as an EPS read from its preview and
/// why), title, colour mode, units, artboards, object counts by kind, fonts.
#[derive(Args)]
pub(crate) struct InfoArgs {
    /// The file to read (any readable format)
    #[arg(value_name = "FILE")]
    pub(crate) file: String,
}

/// Time rendering a file
///
/// Renders FILE (any readable format) fitted to WxH and prints ms per frame (warm), multithreaded
/// and single-threaded.
#[derive(Args)]
pub(crate) struct BenchArgs {
    /// The file to render
    #[arg(value_name = "FILE")]
    pub(crate) file: String,
    /// Frame size (default 2880x1800)
    #[arg(long, value_name = "WxH", default_value = "2880x1800", value_parser = size)]
    pub(crate) size: Size,
    /// Frames to time (default 5)
    #[arg(long, value_name = "N", default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..))]
    pub(crate) iters: u32,
}

/// Check the performance budgets
///
/// Checks the budgets (render, pan, hit test, save/load, SVG, Pathfinder, handle and segment drags)
/// on a synthetic N-path document. Exits non-zero if a budget is exceeded, unless the report says
/// the machine is busy.
#[derive(Args)]
pub(crate) struct PerfArgs {
    /// Paths in the synthetic document (default 50000)
    #[arg(long, value_name = "N", default_value_t = 50_000, value_parser = at_least_one)]
    pub(crate) paths: usize,
    /// Also time work with no budget yet and print memory use (the working set on Windows), with
    /// the peak so far on Windows and Linux
    #[arg(long)]
    pub(crate) deep: bool,
}

/// A frame size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Size {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Why a value isn't accepted: clap prints it after the flag's name.
#[derive(Debug, thiserror::Error)]
enum ValueError {
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("expected a JSON object")]
    NotObject,
    #[error("expected a number above 0")]
    NotPositive,
    #[error("expected a whole number above 0")]
    NotCount,
    #[error("expected WIDTHxHEIGHT, such as 2880x1800, both above 0")]
    Size,
}

fn json_object(s: &str) -> Result<Value, ValueError> {
    let v: Value = serde_json::from_str(s)?;
    if v.is_object() { Ok(v) } else { Err(ValueError::NotObject) }
}

fn positive(s: &str) -> Result<f64, ValueError> {
    s.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).ok_or(ValueError::NotPositive)
}

fn at_least_one(s: &str) -> Result<usize, ValueError> {
    s.parse::<usize>().ok().filter(|n| *n > 0).ok_or(ValueError::NotCount)
}

fn size(s: &str) -> Result<Size, ValueError> {
    let (w, h) = s.split_once('x').ok_or(ValueError::Size)?;
    match (w.parse::<u32>(), h.parse::<u32>()) {
        (Ok(width), Ok(height)) if width > 0 && height > 0 => Ok(Size { width, height }),
        _ => Err(ValueError::Size),
    }
}

/// Usage errors that clap's own checks can't express, raised once the line is parsed and before any
/// work starts.
#[derive(Debug, thiserror::Error)]
enum UsageError {
    #[error("--params must follow a --cmd")]
    ParamsWithoutCmd,
    #[error("--params is given twice for one --cmd")]
    ParamsTwice,
    #[error("--commands - is given twice: standard input is read once")]
    StdinTwice,
    #[error("a .ait template holds every artboard: --artboard and --range don't apply")]
    TemplateArtboards,
}

impl UsageError {
    fn into_clap(self, cmd: &mut clap::Command) -> clap::Error {
        let kind = match self {
            Self::ParamsWithoutCmd => ErrorKind::MissingRequiredArgument,
            Self::ParamsTwice => ErrorKind::ArgumentConflict,
            Self::StdinTwice | Self::TemplateArtboards => ErrorKind::ArgumentConflict,
        };
        cmd.error(kind, self)
    }
}

/// The command line's clap definition, also for generating completions and man pages
/// (`examples/man-and-completions.rs`). The help ends with the formats the commands read and write.
pub fn command() -> clap::Command {
    use vectorcraft_engine::cmd::fileio::{OPEN_EXTS, export_extensions};
    let readable = format!("Readable formats: .{}", OPEN_EXTS.join(", ."));
    let writable = format!("Writable formats: .{}", export_extensions().join(", ."));
    let both = format!("{readable}\n{writable}");
    Cli::command()
        .after_help(both.clone())
        .mut_subcommand("run", |c| c.after_help(both.clone()))
        .mut_subcommand("convert", |c| c.after_help(both))
        .mut_subcommand("info", |c| c.after_help(readable.clone()))
        .mut_subcommand("bench", |c| c.after_help(readable))
}

/// `--cmd`, `--params`, `--commands` and `--export` values in command-line order.
fn run_items(m: &ArgMatches) -> Vec<RunItem> {
    fn strings(m: &ArgMatches, id: &str, make: fn(String) -> RunItem, into: &mut Vec<(usize, RunItem)>) {
        if let (Some(at), Ok(Some(values))) = (m.indices_of(id), m.try_get_many::<String>(id)) {
            into.extend(at.zip(values).map(|(i, v)| (i, make(v.clone()))));
        }
    }
    let mut items: Vec<(usize, RunItem)> = Vec::new();
    strings(m, "cmd", RunItem::Cmd, &mut items);
    strings(m, "commands", RunItem::Commands, &mut items);
    strings(m, "export", RunItem::Export, &mut items);
    if let (Some(at), Ok(Some(params))) = (m.indices_of("params"), m.try_get_many::<Value>("params")) {
        items.extend(at.zip(params).map(|(i, p)| (i, RunItem::Params(p.clone()))));
    }
    items.sort_by_key(|(i, _)| *i);
    items.into_iter().map(|(_, item)| item).collect()
}

/// Parses the arguments (without the program name). Help and `--version` come back as errors too
/// ([`clap::Error::use_stderr`] is false for them).
pub(crate) fn parse(args: &[OsString]) -> Result<Cli, clap::Error> {
    let mut cmd = command();
    let argv = std::iter::once(OsString::from("vectorcraft-cli")).chain(args.iter().cloned());
    let matches = match cmd.try_get_matches_from_mut(argv) {
        // `--help` or `-h` anywhere asks for the help, even after a mistake that stopped clap
        // before it got there (`run --cmd file.new --params not-json -h`).
        Err(e) if args.iter().take_while(|a| *a != "--").any(|a| *a == "--help" || *a == "-h") => {
            let help = [OsString::from("vectorcraft-cli"), args.first().cloned().unwrap_or_default(), OsString::from("--help")];
            return Err(cmd.try_get_matches_from_mut(help).err().unwrap_or(e));
        }
        matches => matches?,
    };
    let mut cli = Cli::from_arg_matches(&matches)?;
    match (&mut cli.command, matches.subcommand()) {
        (Command::Run(r), Some((name, m))) => {
            r.items = run_items(m);
            let sub = cmd.find_subcommand_mut(name);
            if let Some(sub) = sub
                && let Err(e) = check_run(&r.items)
            {
                return Err(e.into_clap(sub));
            }
        }
        (Command::Convert(c), Some((name, _))) => {
            if (c.artboard.is_some() || c.range.is_some())
                && crate::convert::is_template(&c.output)
                && let Some(sub) = cmd.find_subcommand_mut(name)
            {
                return Err(UsageError::TemplateArtboards.into_clap(sub));
            }
        }
        _ => {}
    }
    Ok(cli)
}

/// `--params` sets the `--cmd` just before it, and standard input is read once.
fn check_run(items: &[RunItem]) -> Result<(), UsageError> {
    // The --cmd that a --params would set: none after a --commands or an --export, and one --params each.
    let (mut after_cmd, mut has_params) = (false, false);
    let mut stdin = false;
    for item in items {
        match item {
            RunItem::Cmd(_) => (after_cmd, has_params) = (true, false),
            RunItem::Params(_) if !after_cmd => return Err(UsageError::ParamsWithoutCmd),
            RunItem::Params(_) if std::mem::replace(&mut has_params, true) => return Err(UsageError::ParamsTwice),
            RunItem::Params(_) => {}
            RunItem::Commands(source) => {
                after_cmd = false;
                if source == "-" && std::mem::replace(&mut stdin, true) {
                    return Err(UsageError::StdinTwice);
                }
            }
            RunItem::Export(_) => after_cmd = false,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn try_parse(args: &[&str]) -> Result<Cli, clap::Error> {
        parse(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    fn usage_error(args: &[&str]) -> ErrorKind {
        match try_parse(args) {
            Err(e) => e.kind(),
            Ok(_) => panic!("{args:?} parsed"),
        }
    }

    #[test]
    fn the_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn unknown_and_repeated_flags_are_usage_errors() {
        assert_eq!(usage_error(&["convert", "a.svg", "b.png", "--artbord", "1"]), ErrorKind::UnknownArgument);
        assert_eq!(usage_error(&["convert", "a.svg", "b.png", "--scale", "1", "--scale", "2"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["perf", "--deep", "--deep"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["run", "--in", "a", "--in", "b"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["run", "--scale", "1", "-s", "2"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["commands", "extra"]), ErrorKind::UnknownArgument);
        assert_eq!(usage_error(&["frobnicate"]), ErrorKind::InvalidSubcommand);
        assert_eq!(usage_error(&[]), ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
    }

    #[test]
    fn values_have_types() {
        for bad in ["0", "-1", "nan", "inf", "two", ""] {
            assert_eq!(usage_error(&["convert", "a.svg", "b.png", "--scale", bad]), ErrorKind::ValueValidation, "--scale {bad}");
        }
        assert_eq!(usage_error(&["convert", "a.svg", "b.png", "--artboard", "x"]), ErrorKind::ValueValidation);
        assert_eq!(usage_error(&["convert", "a.svg", "b.png", "--ai-version", "8"]), ErrorKind::InvalidValue);
        assert_eq!(usage_error(&["convert", "a.svg", "b.png", "--hidden-objects", "x"]), ErrorKind::InvalidValue);
        assert_eq!(usage_error(&["convert", "a.svg", "b.png", "--transparent-images", "x"]), ErrorKind::InvalidValue);
        assert_eq!(usage_error(&["convert", "a.svg"]), ErrorKind::MissingRequiredArgument);
        assert_eq!(usage_error(&["convert", "a.svg", "b.png", "c.png"]), ErrorKind::UnknownArgument);
        assert_eq!(usage_error(&["convert", "a.svg", "b.png", "--artboard", "1", "--range", "1"]), ErrorKind::ArgumentConflict);
        for bad in ["[1]", "3", "{oops", "\"x\""] {
            assert_eq!(usage_error(&["run", "--cmd", "x", "--params", bad]), ErrorKind::ValueValidation, "--params {bad}");
        }
        for bad in ["bad", "10", "10x", "0x5", "5x0", "1x2x3"] {
            assert_eq!(usage_error(&["bench", "f.svg", "--size", bad]), ErrorKind::ValueValidation, "--size {bad}");
        }
        assert_eq!(usage_error(&["bench", "f.svg", "--iters", "0"]), ErrorKind::ValueValidation);
        assert_eq!(usage_error(&["perf", "--paths", "0"]), ErrorKind::ValueValidation);
        assert_eq!(usage_error(&["perf", "--paths", "many"]), ErrorKind::ValueValidation);
    }

    #[test]
    fn a_value_flag_never_takes_the_next_flag() {
        assert_eq!(usage_error(&["run", "--cmd", "--export", "a.svg"]), ErrorKind::InvalidValue);
        assert_eq!(usage_error(&["mcp", "--connect", "--headless"]), ErrorKind::InvalidValue);
        // `=` carries a value that starts with `-`.
        let Ok(Cli { command: Command::Run(r) }) = try_parse(&["run", "--cmd=-x"]) else { panic!("did not parse") };
        assert!(matches!(&r.items[..], [RunItem::Cmd(id)] if id == "-x"));
    }

    #[test]
    fn values_may_go_with_equals_or_apart_and_a_double_dash_ends_the_flags() {
        let Ok(Cli { command: Command::Convert(c) }) = try_parse(&["convert", "--scale=2", "in.svg", "-a", "3", "--", "-out.png"]) else {
            panic!("did not parse")
        };
        assert_eq!((c.scale, c.artboard, c.input.as_str(), c.output.as_str()), (2.0, Some(3), "in.svg", "-out.png"));
        let Ok(Cli { command: Command::Bench(b) }) = try_parse(&["bench", "--size=640x480", "f.svg"]) else { panic!("did not parse") };
        assert_eq!(b.size, Size { width: 640, height: 480 });
        assert_eq!(b.iters, 5);
    }

    #[test]
    fn mcp_options_conflict() {
        assert_eq!(usage_error(&["mcp", "--connect", "a:1", "--headless"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["mcp", "--connect", "a:1", "--automation-read-root", "d"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["mcp", "--connect=a:1", "--automation-write-root=d"]), ErrorKind::ArgumentConflict);
        let Ok(Cli { command: Command::Mcp(m) }) =
            try_parse(&["mcp", "--headless", "--automation-read-root=a", "--automation-read-root", "b", "--automation-write-root", "c"])
        else {
            panic!("did not parse")
        };
        assert_eq!((m.automation_read_root, m.automation_write_root), (vec!["a".to_string(), "b".into()], vec!["c".to_string()]));
    }

    #[test]
    fn run_steps_keep_the_order_of_the_command_line() {
        let args = ["run", "--cmd", "a", "--params", "{}", "--export", "x.svg", "--commands", "f.json", "-c", "b", "-o", "y.png"];
        let Ok(Cli { command: Command::Run(r) }) = try_parse(&args) else { panic!("did not parse") };
        let kinds: Vec<&str> = r
            .items
            .iter()
            .map(|i| match i {
                RunItem::Cmd(_) => "cmd",
                RunItem::Params(_) => "params",
                RunItem::Commands(_) => "commands",
                RunItem::Export(_) => "export",
            })
            .collect();
        assert_eq!(kinds, ["cmd", "params", "export", "commands", "cmd", "export"]);
    }

    #[test]
    fn run_checks_that_cannot_be_expressed_in_clap() {
        assert_eq!(usage_error(&["run", "--params", "{}"]), ErrorKind::MissingRequiredArgument);
        assert_eq!(usage_error(&["run", "--cmd", "a", "--commands", "f.json", "--params", "{}"]), ErrorKind::MissingRequiredArgument);
        assert_eq!(usage_error(&["run", "--cmd", "a", "--export", "x.svg", "--params", "{}"]), ErrorKind::MissingRequiredArgument);
        assert_eq!(usage_error(&["run", "--commands", "-", "--commands", "-"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["run", "--cmd", "a", "--params", "{}", "--params", "{}"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["convert", "a.svg", "b.ait", "--artboard", "0"]), ErrorKind::ArgumentConflict);
        assert_eq!(usage_error(&["convert", "a.svg", "b.ait", "--range", "1"]), ErrorKind::ArgumentConflict);
        assert!(try_parse(&["run"]).is_ok());
    }

    #[test]
    fn help_and_version_are_not_failures() {
        for args in [&["--help"][..], &["-h"], &["help"], &["help", "run"], &["run", "--help"], &["-V"], &["--version"]] {
            let e = try_parse(args).err().map(|e| (e.use_stderr(), e.exit_code()));
            assert_eq!(e, Some((false, 0)), "{args:?}");
        }
        // Help wins over a mistake that clap would stop at first.
        for args in [
            &["run", "--cmd", "file.new", "--params", "not json", "-h"][..],
            &["perf", "--paths", "many", "-h"],
            &["convert", "--scale", "x", "--help"],
        ] {
            let e = try_parse(args).err().map(|e| (e.use_stderr(), e.exit_code()));
            assert_eq!(e, Some((false, 0)), "{args:?}");
        }
    }
}
