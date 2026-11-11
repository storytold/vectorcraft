//! `script.check`: a script written for the reference app (`.jsx`, `.jsxinc`, `.js`, or a compiled
//! `.jsxbin`) checked without running any of it, by `vectorcraft-scriptcheck`: what kind of file it
//! is, what it needs, its directives and includes, the platform features and document-model member
//! chains it uses, the menu commands it runs, what it does outside the document, and its problems.
//! With a path and `includes`, the files its `#include` lines name are read from inside its folder,
//! each through the same checks as every other file a command reads.

use serde_json::{Value, json};
use vectorcraft_scriptcheck::{Limits, Loaded, Loader, Report};

/// The ids a reply can hold, for the windows that show it.
pub use vectorcraft_scriptcheck::{IncludeStatus, Need, OUTSIDE, PROBLEMS};

use super::{CommandSpec, EngineError, Result, Session, always, bad, bool_or, cmd, str_param};
use crate::script;

/// The extensions of the scripts File › Open shows a check for.
pub const EXTS: &[&str] = &["jsx", "jsxinc", "jsxbin"];

/// The extensions of the scripts `script.run` takes as a file, in any case.
pub const RUN_EXTS: &[&str] = &["js", "jsx"];

/// Does `path` name a file `script.run` takes?
pub fn runs(path: &str) -> bool {
    std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| RUN_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "script.run",
            "Run Script",
            [],
            None,
            "{path?: a .js or .jsx file | source?: the script's text (exactly one of the two), name?: the name errors and the report use (default: the script's #script line, else the path's file name, or \"script\"), args?: {…} (at most 1 MiB as JSON; vc.args in the script), answers?: [{kind: alert|confirm|prompt, value: \"ok\" (alert) | true or false (confirm) | text or null (prompt)}] (for a run with nobody to ask: the answers to its dialogs, in order; at most 10000 and 1 MiB as JSON; refused in the app, where the person answers; an empty object is an empty list), maxFuel?: whole number from 1 to 100000000000 (instructions in all; default 1000000000)} run a JavaScript file against the open documents, through the vc object: vc.run(id, params) runs a command and returns its result, a failed command throws an error the script can catch and puts back the documents and the session state it changed, while any file it wrote stays written and is named; vc.document is a snapshot of the active document or null; vc.commands lists the commands a script may run; vc.log(value) adds to the report and the Script Console (script.console); $.write(...values) writes the values to the Script Console as String(value) gives them, and $.writeln(...values) does the same and ends the line; $.evalFile(path, timeout?) runs a file with the same globals and returns its last value (timeout, in milliseconds from 0, is taken and not waited on), $.fileName is the file being run (empty for text) and $.includePath the #includepath list; alert(message[, title, errorIcon]), confirm(message[, noAsDflt, title]) and prompt(message, preset[, title]) open a dialog with a Stop button (default title \"Script Alert\"; message and preset as String(value) gives them; errorIcon and noAsDflt as Boolean(value) gives them; No is the default with noAsDflt): alert returns undefined, confirm true for Yes and false for No, prompt the field's text for OK and null for Cancel; in the app the run waits at the call for the person's answer and Stop ends it as script.stop does, a dialog cannot open inside a callback of a built-in, a getter, a conversion or a file $.evalFile runs and throws there (an #include file's code is the script's own and can), and a run with nobody to ask takes the next of answers, and ends when none is left or the next is for another kind of dialog, or when the script ends with answers unused; vc.args is args. Before anything runs, the script's preprocessor lines (# or //@ at the start of a line, outside strings and comments) are read: #include \"file\" and //@include put that file's text in place of the line, looked for beside the file that includes it, then in each #includepath folder in turn (separated by ;, relative to that file's folder; a later #includepath replaces the list), then in each JSINCLUDE folder; #script names the run; #target takes any value and changes nothing; #strict on makes an assignment to a read-only or getter-only property, or a new property on an object that is not extensible (preventExtensions, seal, freeze, vc, $), throw a TypeError, without the rest of ES5 strict mode, and #strict off turns that off (the last #strict line of the script and its includes sets the run; off without one; a $.evalFile file's own line holds for its code); #targetengine and #engine are refused as unsupported, any other #name as unknown, and another //@name is a comment. A script run from a file reads its includes and $.evalFile's files only in that file's folder and the folders below it, refusing a path with a .. name or a link that leads out (a JSINCLUDE folder outside is passed over); a script given as text reads none. File(path) and Folder(path) name files and folders and read them in the same folders, and inside the automation read roots when they are set: exists, length, created, modified, getFiles(mask or function; at most 2048 entries), resolve, and open('r') with read, readln, readch, seek, tell and close; a path outside, a network or device path and a link that leads out give false or null with error \"Script file access denied\", a file is read whole at most 8 MiB (checked from its size first) and a run reads at most 32 MiB, and writing, the file dialogs and execute return false or null with error set. An error in another file is at that file's line and column; vc.api.version and vc.api.has(feature) describe the API. The run is one transaction: on success each changed document gets one undo step, Run Script, and each preference it changed keeps its value, and on an uncaught error, a limit or script.stop every open document and the session go back as they were; files a command wrote before that stay written and are named. While it runs, other commands that change anything end with a busy error. Commands that change anything outside the documents (script.*, except script.check, plugin.*, prefs.*, actionSet.*, shortcutSet.*, help.*, print.*, library.*, importMatches.*, edit.undoGroup.*, file.recovery.*, stroke.widthProfile.*, saving, importing or deleting a preset, text.addFontFiles, text.findFontFiles, text.rescanFonts, links.findFiles, file.place.queue, color.loadProfile, color.removeProfile, file.print, view.proofSetup, view.proofColors, view.overprintPreview, view.separationsPreview) throw. VectorCraft's own limits: 8 MiB of source a file and 32 MiB in all, 64 included or evaluated files 16 deep, 1024 paths looked up for them, 256 MiB of heap (128 MiB on the web), 2000000 host calls, 200000000 instructions without a pause, 64 levels, 16 MiB and 1048576 entries for a value crossing to or from a command, 10000 log entries and 4 MiB of log, 16 KiB of a dialog's message, 256 bytes of its title and 16 KiB of a prompt's preset (past these a RangeError), and 100000 objects and 512 MiB of new document data, past which a person is asked to continue or stop and a run with nobody to ask ends → {ok: true, name, changed: [document names], written: [paths], log: [values], fuelUsed, hostCalls, runTimeMs (the time the script ran, its commands included, waits between frames and for a person's answer left out; 0 without a clock), timingAvailable (false on the web), hostCallCounts: {vcRun, documentReads, commandListReads, vcLog, consoleWrites, fileCalls, fileExecutes}, slowestCommands: [{id, calls, totalMs, maxMs}] (at most 5, the longest in all first, then by id), console: {lines: [text], droppedLines, truncatedLines} (the run's own output, vc.log lines and error), dialogs: [{kind, message, title, errorIcon (alert), noAsDefault (confirm), preset (prompt), answer: \"ok\" | true | false | text | null | \"stop\" | \"missing\"}] (in the order they opened, at most 10000 and 1 MiB), dialogCount, dialogsTruncated}, or, in the app, {running: true, name} and the report later in ui.inspect's scriptReport; a run that fails ends with its error, and its report there also has kind, message, line, column, file (when the error is in another file than the script), fuelLeft, fuelUsed, hostCalls and the fields above",
            always,
            run
        ),
        cmd!(
            query "script.stop",
            "Stop Script",
            [],
            None,
            "{} stop the running script, which puts every document back as it was before the run; asking again while it stops is harmless → {stopped: whether a script was running}",
            always,
            stop
        ),
        cmd!(
            query "script.console",
            "Script Console",
            [],
            None,
            "{} the Script Console's lines, oldest first: each run's start, its $.write and $.writeln output, its vc.log entries, its uncaught error as name:line:column: message (name: file:line:column: message when it is in another file), its end and a report of its time, host calls and slowest commands; a session keeps at most 2048 lines and 1 MiB of text, dropping the oldest, a line at most 4096 bytes and a script's name in its lines at most 256 bytes, each cut with … (VectorCraft's own limits); line feeds, carriage returns and the two together break lines, other control characters show as their Unicode Control Pictures → {lines: [{run: the run's number in the session, name, kind: start|output|log|error|end|report, text}], droppedLines, truncatedLines} (an end line's text is ended: finished|error|fuel|hostLimit|limit|stopped|internal)",
            always,
            console_lines
        ),
        cmd!(
            query "script.console.clear",
            "Clear Script Console",
            [],
            None,
            "{} empty the Script Console and set its dropped and cut counts to zero; a running script goes on writing, and scriptReport keeps the last run's console → {cleared: lines removed}",
            always,
            console_clear
        ),
        cmd!(
            query "script.listInstalled",
            "List Installed Scripts",
            [],
            None,
            "{} list the .js and .jsx files of the Scripts folder next to the settings, which File › Scripts lists, each subfolder as a submenu, in menu order; only names are read, never a script's text; links, hidden files, names starting with a dot and folders without a script are left out. VectorCraft's own limits: 2048 folder entries looked at, 512 scripts, 128 subfolders, 4 levels of subfolders, names of 255 bytes, labels of 128 characters, 250 ms → {folder: the Scripts folder's path | null (a session without one, and the web), scripts: [{path, label: the file name without its extension (the whole file name when another row of its folder has that label), folders: [the labels of the subfolders it is in]}], truncated: a limit was reached or a subfolder could not be read, unreadable: the folder is there and could not be read, or is a link}",
            always,
            list_installed
        ),
        cmd!(
            query "script.check",
            "Check Script",
            [],
            None,
            "{path? | data?: the script's text | dataBase64?, name?: the file's name (default: the path's; with a path, only the reply's name), includes?: false (with a path: also check the files its #include lines name, inside the script's folder; not on the web)} check a script written for the reference app (.jsx, .jsxinc, .js, or a compiled .jsxbin) without running any of it → {name, kind: script|compiled|module|notText|tooLarge, needs: [language|dom|files|menuCommands|dialogs|hostOnly], lines, targets: [{value, app, instance?, version?, locale?, line, file?}], directives: [{name, value, line, form: \"#\"|\"//@\", file?}], includes: [{value, line, status: checked|missing|notFollowed|cycle|notRead|limit|unreadable, path?, file?}], features: [{id, count, lines}], dom: {members: [{path, count}], types: [{name, count}]}, menuCommands: {strings: [{value, count, lines}], dynamic}, outside: [{id: filesWritten|filesOpened|network|nativeCode|otherApps|environment|unseenCode|preferences, count, lines, modes?}], problems: [{code, line?, file?, detail?}], truncated} (file: the included file it is in; lines: at most 20 of the script's own, while counts include the included files; at most 8 MiB a file and 32 MiB in all, 64 included files 16 deep, 1,024 paths looked up for them, 1,000 entries a list, text echoed cut to 120 characters)",
            always,
            check
        ),
    ]
}

/// A refusal of `script.run`'s parameters, which only a program's own call can give.
fn param(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "script.run".into(), msg: msg.into() }
}

/// `maxFuel`: a whole number from 1 to [`script::MAX_FUEL`], or the default when omitted.
fn max_fuel(p: &Value) -> Result<u64> {
    let Some(v) = p.get("maxFuel") else { return Ok(script::DEFAULT_FUEL) };
    v.as_u64()
        .filter(|n| (1..=script::MAX_FUEL).contains(n))
        .ok_or_else(|| param(format!("`maxFuel` must be a whole number from 1 to {}", script::MAX_FUEL)))
}

/// The text of a script read as bytes: a byte order mark is left out, and compiled scripts and
/// bytes that are not UTF-8 are refused.
fn script_text(bytes: &[u8]) -> Result<String> {
    if bytes.starts_with(b"@JSXBIN@") {
        return Err(EngineError::Other(COMPILED.to_string()));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| EngineError::Other(NOT_TEXT.to_string()))?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(text).to_string())
}

const COMPILED: &str = "this is a compiled script, which cannot be run; use its source text";
const NOT_TEXT: &str = "the script is not UTF-8 text";
/// The error for a script over [`script::MAX_SOURCE_BYTES`].
pub const TOO_LARGE: &str = "the script is too large to run";

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let optional_str = |key: &str| match p.get(key) {
        None => Ok(None),
        Some(Value::String(t)) => Ok(Some(t.as_str())),
        Some(_) => Err(param(format!("`{key}` must be a string"))),
    };
    let (path, source, given_name) = (optional_str("path")?, optional_str("source")?, optional_str("name")?);
    // Checked before anything copies or writes out `args`, which would go one call deeper per level.
    if let Some(place) = p.get("args").and_then(script::too_deep) {
        return Err(param(format!("`args{place}` is nested more than 64 levels deep")));
    }
    let args = match p.get("args") {
        None => serde_json::Map::new(),
        Some(Value::Object(o)) => o.clone(),
        Some(_) => return Err(param("`args` must be an object")),
    };
    if serde_json::to_vec(&args).map_or(true, |b| b.len() > script::MAX_ARGS_BYTES) {
        return Err(param("`args` is larger than 1 MiB"));
    }
    let answers = script::dialogs::parse_answers(p.get("answers")).map_err(param)?;
    if s.script_frames && !answers.is_empty() {
        return Err(param("`answers` is only for a run with nobody to ask: in the app, the person answers each dialog"));
    }
    let fuel = max_fuel(p)?;
    let text = match (path, source) {
        (Some(""), None) => return Err(param("`path` is empty")),
        (Some(path), None) => {
            if path.to_ascii_lowercase().ends_with(".jsxbin") {
                return Err(EngineError::Other(COMPILED.to_string()));
            }
            if !runs(path) {
                return Err(param("`path` must name a .js or .jsx file"));
            }
            // One byte more than a script may have shows that the file is too large.
            let bytes = super::fileio::read_file_head(path, script::MAX_SOURCE_BYTES + 1)?;
            if bytes.len() > script::MAX_SOURCE_BYTES {
                return Err(EngineError::Other(TOO_LARGE.to_string()));
            }
            script_text(&bytes)?
        }
        (None, Some(source)) => {
            if source.len() > script::MAX_SOURCE_BYTES {
                return Err(EngineError::Other(TOO_LARGE.to_string()));
            }
            script_text(source.as_bytes())?
        }
        _ => return Err(param("give exactly one of `path` and `source`")),
    };
    let name = given_name.map(str::to_string).or_else(|| path.map(super::fileio::file_name)).unwrap_or_else(|| "script".to_string());
    let locale = script::dollar::ui_locale(s.ui_language.as_deref().unwrap_or(&s.prefs.interface_language));
    let request =
        script::Request { name, name_given: given_name.is_some(), file: path.map(str::to_string), source: text, args, fuel, locale, answers };
    let name = s.script_start(request)?;
    if s.script_frames {
        return Ok(json!({"running": true, "name": name}));
    }
    s.script_run_to_end()
}

fn stop(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({"stopped": s.script_request_stop()}))
}

fn console_lines(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(s.script_console().to_json())
}

fn console_clear(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({"cleared": s.clear_script_console()}))
}

/// The error for a script that File › Scripts listed and that [`script::folder::read`] no longer
/// reads.
pub const NOT_LISTED: &str = "the script is no longer in the Scripts folder";

impl Session {
    /// Have File › Scripts and `script.listInstalled` list the scripts of the folder `dir`. The
    /// folder is only read, and never made.
    pub fn attach_scripts_dir(&mut self, dir: std::path::PathBuf) {
        self.scripts_dir = Some(dir);
    }

    /// The Scripts folder ([`Session::attach_scripts_dir`]); none on the web, which has no folders.
    pub fn scripts_dir(&self) -> Option<&std::path::Path> {
        self.scripts_dir.as_deref().filter(|_| !cfg!(target_arch = "wasm32"))
    }

    /// The text of the script at `path`, which a listing of the Scripts folder gave
    /// ([`script::folder::read`]), checked as `script.run` checks a file's.
    pub fn installed_script(&self, path: &str) -> Result<String> {
        let root = self.scripts_dir().ok_or_else(|| EngineError::Other(NOT_LISTED.to_string()))?;
        crate::file_access::check_read(path).map_err(EngineError::Other)?;
        // One byte more than a script may have shows that the file is too large.
        let bytes = script::folder::read(root, std::path::Path::new(path), script::MAX_SOURCE_BYTES + 1)
            .ok_or_else(|| EngineError::Other(NOT_LISTED.to_string()))?;
        if bytes.len() > script::MAX_SOURCE_BYTES {
            return Err(EngineError::Other(TOO_LARGE.to_string()));
        }
        script_text(&bytes)
    }
}

fn list_installed(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(root) = s.scripts_dir() else {
        return Ok(json!({"folder": null, "scripts": [], "truncated": false, "unreadable": false}));
    };
    let folder = root.to_string_lossy();
    crate::file_access::check_read(&folder).map_err(EngineError::Other)?;
    let listing = script::folder::list(root);
    let scripts: Vec<Value> = listing
        .scripts()
        .into_iter()
        .map(|(folders, label, path)| json!({"path": path.to_string_lossy(), "label": label, "folders": folders}))
        .collect();
    Ok(json!({"folder": folder, "scripts": scripts, "truncated": listing.truncated, "unreadable": listing.unreadable}))
}

/// The folder a script is in, which its included files are read from.
struct Folder {
    dir: std::path::PathBuf,
    /// The folder with its links followed (none where the file system can't tell).
    real: Option<std::path::PathBuf>,
}

impl Folder {
    /// The folder of the script at `path`.
    fn of(path: &str) -> Self {
        let dir = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
        Folder { real: std::fs::canonicalize(dir).ok(), dir: dir.to_path_buf() }
    }
}

impl Loader for Folder {
    fn load(&mut self, path: &str, max: usize) -> Loaded {
        let file = self.dir.join(path);
        // A link that leads out of the script's folder isn't followed.
        if let (Some(real), Ok(target)) = (&self.real, std::fs::canonicalize(&file))
            && !target.starts_with(real)
        {
            return Loaded::Refused;
        }
        let file = file.to_string_lossy();
        if crate::file_access::check_read(&file).is_err() {
            return Loaded::Refused;
        }
        super::fileio::read_file_head(&file, max).map_or(Loaded::Missing, Loaded::Bytes)
    }
}

/// The bytes the base64 text `b64` holds, up to `max` and a few more: only the text they take is
/// decoded.
pub(super) fn decode_head(b64: &str, max: usize) -> Option<Vec<u8>> {
    let chars = max.div_ceil(3).saturating_mul(4);
    let mut kept = 0usize;
    let end = b64
        .bytes()
        .position(|b| {
            kept += usize::from(!b.is_ascii_whitespace());
            kept > chars
        })
        .unwrap_or(b64.len());
    vectorcraft_format::base64_decode(b64.get(..end)?)
}

fn check(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "script.check";
    let limits = Limits::default();
    // One byte more than a file may have shows that the file is too large.
    let max = limits.file_bytes.saturating_add(1);
    let path = str_param(p, "path");
    let bytes: std::borrow::Cow<[u8]> = match (path, str_param(p, "data"), str_param(p, "dataBase64")) {
        (Some(path), ..) => super::fileio::read_file_head(path, max)?.into(),
        (None, Some(text), _) => text.as_bytes().get(..max).unwrap_or(text.as_bytes()).into(),
        (None, None, Some(b64)) => decode_head(b64, max).ok_or_else(|| bad(C, "bad dataBase64"))?.into(),
        _ => return Err(bad(C, "give `path`, `data` or `dataBase64`")),
    };
    let name = super::fileio::file_name(str_param(p, "name").or(path).unwrap_or_default());
    // With a path, the file is checked under its own name, which its includes are followed from;
    // `name` names the reply.
    let file = path.map(super::fileio::file_name).unwrap_or_else(|| name.clone());
    let mut folder = path.filter(|_| bool_or(p, "includes", false)).map(Folder::of);
    let report = vectorcraft_scriptcheck::check(&file, &bytes, folder.as_mut().map(|f| f as &mut dyn Loader), &limits);
    Ok(report_json(&name, &report))
}

/// Object `v` with `key: value` added when there is a value.
fn with(mut v: Value, key: &str, value: Option<impl serde::Serialize>) -> Value {
    if let (Some(o), Some(x)) = (v.as_object_mut(), value) {
        o.insert(key.into(), json!(x));
    }
    v
}

/// The reply for report `r` on the script `name`.
fn report_json(name: &str, r: &Report) -> Value {
    let targets: Vec<Value> = r
        .targets
        .iter()
        .map(|t| {
            let v = json!({"value": t.value, "app": t.app, "line": t.line});
            let v = with(v, "instance", t.instance.as_deref());
            let v = with(v, "version", t.version.as_deref());
            let v = with(v, "locale", t.locale.as_deref());
            with(v, "file", t.file.as_deref())
        })
        .collect();
    let directives: Vec<Value> = r
        .directives
        .iter()
        .map(|d| with(json!({"name": d.name, "value": d.value, "line": d.line, "form": d.form}), "file", d.file.as_deref()))
        .collect();
    let includes: Vec<Value> = r
        .includes
        .iter()
        .map(|i| with(with(json!({"value": i.value, "line": i.line, "status": i.status.id()}), "path", i.path.as_deref()), "file", i.file.as_deref()))
        .collect();
    let features: Vec<Value> = r.features.iter().map(|f| json!({"id": f.id, "count": f.count, "lines": f.lines})).collect();
    let members: Vec<Value> = r.members.iter().map(|m| json!({"path": m.name, "count": m.count})).collect();
    let types: Vec<Value> = r.types.iter().map(|t| json!({"name": t.name, "count": t.count})).collect();
    let strings: Vec<Value> = r.menu_strings.iter().map(|s| json!({"value": s.value, "count": s.count, "lines": s.lines})).collect();
    let outside: Vec<Value> = r
        .outside
        .iter()
        .map(|o| with(json!({"id": o.id, "count": o.count, "lines": o.lines}), "modes", Some(&o.modes).filter(|m| !m.is_empty())))
        .collect();
    let problems: Vec<Value> = r
        .problems
        .iter()
        .map(|p| with(with(with(json!({"code": p.code}), "line", p.line), "file", p.file.as_deref()), "detail", p.detail.as_deref()))
        .collect();
    json!({
        "name": name,
        "kind": r.kind.id(),
        "needs": r.needs.iter().map(|n| n.id()).collect::<Vec<_>>(),
        "lines": r.lines,
        "targets": targets,
        "directives": directives,
        "includes": includes,
        "features": features,
        "dom": {"members": members, "types": types},
        "menuCommands": {"strings": strings, "dynamic": r.menu_dynamic},
        "outside": outside,
        "problems": problems,
        "truncated": r.truncated,
    })
}
