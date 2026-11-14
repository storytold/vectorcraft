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
            "{path?: a .js or .jsx file | source?: the script's text (exactly one of the two), name?: the name errors and the report use (default: the script's #script line, else the path's file name, or \"script\"), args?: {…} (at most 1 MiB as JSON; vc.args in the script), answers?: [{kind: alert|confirm|prompt, value: \"ok\" (alert) | true or false (confirm) | text or null (prompt)}] (for a run with nobody to ask: the answers to its dialogs, in order; at most 10000 and 1 MiB as JSON; refused in the app, where the person answers; an empty object is an empty list), maxFuel?: whole number from 1 to 100000000000 (instructions in all; default 1000000000)} run a JavaScript file against the open documents, through the vc object: vc.run(id, params) runs a command and returns its result, a failed command throws an error the script can catch and puts back the documents and the session state it changed, while any file it wrote stays written and is named; vc.document is a snapshot of the active document or null; vc.commands lists the commands a script may run; vc.log(value) adds to the report and the Script Console (script.console); $.write(...values) writes the values to the Script Console as String(value) gives them, and $.writeln(...values) does the same and ends the line; $.evalFile(path, timeout?) runs a file with the same globals and returns its last value (timeout, in milliseconds from 0, is taken and not waited on), $.fileName is the file of the code that reads it (the file being run for code that eval or Function compiled; empty for code compiled from a script given as text), $.includePath the #includepath list and $.engineName the engine's name (VectorCraft without #targetengine); alert(message[, title, errorIcon]), confirm(message[, noAsDflt, title]) and prompt(message, preset[, title]) open a dialog with a Stop button (default title \"Script Alert\"; message and preset as String(value) gives them; errorIcon and noAsDflt as Boolean(value) gives them; No is the default with noAsDflt): alert returns undefined, confirm true for Yes and false for No, prompt the field's text for OK and null for Cancel; in the app the run waits at the call for the person's answer and Stop ends it as script.stop does, a dialog cannot open inside a callback of a built-in, a getter, a conversion or a file $.evalFile runs and throws there (an #include file's code is the script's own and can), and a run with nobody to ask takes the next of answers, and ends when none is left or the next is for another kind of dialog, or when the script ends with answers unused; vc.args is args. Before anything runs, the script's preprocessor lines (# or //@ at the start of a line, outside strings and comments) are read: #include \"file\" and //@include put that file's text in place of the line, looked for beside the file that includes it, then in each #includepath folder in turn (separated by ;, relative to that file's folder; a later #includepath replaces the list), then in each JSINCLUDE folder; #script names the run; #target takes any value and changes nothing; #strict on makes an assignment to a read-only or getter-only property, or a new property on an object that is not extensible (preventExtensions, seal, freeze, vc, $), throw a TypeError, without the rest of ES5 strict mode, and #strict off turns that off (the last #strict line of the script and its includes sets the run; off without one; a $.evalFile file's own line holds for its code); #targetengine name runs the script in the named engine, whose globals and functions stay for the next run that names it until script.engine.drop drops it or the session ends (script.engines lists them; a name of letters, digits and _ not starting with a digit, or a quoted one of 1 to 64 bytes without control characters, matched as written; only in the script itself, once, before its code and #include lines; main, and a script without the line, get a new engine each run; a session keeps 8 named engines, a ninth name is refused, each has 64 MiB of heap, and an uncaught error, a limit or script.stop keeps the globals the run set while the documents go back; host functions kept in a global act for the run that calls them; an error in code an earlier run compiled is at that run's file, line and column, within 8 MiB of source maps kept for each engine, the oldest going first; not on the web); #engine is refused as unsupported, any other #name as unknown, and another //@name is a comment. A script run from a file reads its includes and $.evalFile's files only in that file's folder and the folders below it, refusing a path with a .. name or a link that leads out (a JSINCLUDE folder outside is passed over); a script given as text reads none. File(path) and Folder(path) name files and folders and read them in the same folders, and inside the automation read roots when they are set: exists, length, created, modified, getFiles(mask or function; at most 2048 entries), resolve, and open('r') with read, readln, readch, seek, tell and close; a path outside, a network or device path and a link that leads out give false or null with error \"Script file access denied\", a file is read whole at most 8 MiB (checked from its size first) and a run reads at most 32 MiB. A script writes only in Folder.temp (a folder for the run, deleted when it ends), Folder.userData (for a script run from a file, a folder of its own in the Script Data folder next to the settings, kept between runs; for text, one in Folder.temp) and what a person allowed it in a file dialog, inside the automation write roots when they are set, and never in the folder that holds the settings outside its own Folder.userData: open('w') empties or makes the file, open('e') opens a file that is there and open('a') opens it at its end; write(...values) and writeln(...values) (with lineFeed's line end: Windows CRLF, Macintosh CR, Unix LF) encode in the file's encoding (ASCII and ISO-8859-1 refuse a character they cannot hold, BINARY writes each character's low byte), and the file is written at close or when the run ends (a file that close could not write is written again then, and a run that would succeed fails when a file it left open cannot be written), at most 8 MiB a file and 32 MiB a run, which counts what setting length adds and what copy writes once for the file and once more for each object that has it open (write turns its values into text only for an open file and gives false with \"Script file exceeds 8 MiB\" past 16777216 UTF-16 code units); copy(target) replaces a file that is there, remove() deletes a file or an empty folder at once, rename(name) takes a name without a folder that is not in use, Folder.create() makes the folder and the folders above it, and none of them when one is outside the places the script writes, and setting length or readonly changes the file; setting hidden, createAlias and execute return false with error set. In the app, File.openDialog(prompt, filter, multiSelect), File.saveDialog(prompt, filter) and Folder.selectDialog(prompt), and the objects' openDlg, saveDlg and selectDlg, wait for the person, who picks and then allows the script that file or folder once, always (kept in Script Grants.json next to the settings for that script file with that text and the files it includes, by their SHA-256, and for the sources the run evaluated, files $.evalFile ran and strings given to eval and Function, so a later run loses the grant before it evaluates another source, or when its named engine holds code the grant was not kept for, and only while its path is still the real path that was allowed, not one a link now leads elsewhere; script.grants lists them and script.grants.revoke removes them), or denies it, which gives null (a string filter such as \"Text:*.txt;All files:*.*\" gives the file types the dialog shows, *.ext patterns and * or *.* for every file; a filter that is a function is called with a File for each file picked, and the files it gives a false value for are left out, null when none is left; multiSelect gives an array of the files picked, at most 2048); only the person at the keyboard allows a file or folder, and a control request or synthetic input that allows one is refused; an open grant reads the file, a save grant reads and writes it, and a folder grant reads and writes everything inside it; a run with nobody to ask gets null with error \"Script file dialogs need a person to answer them\", and a dialog inside a callback throws. Each file a script writes is in written, also after a run that failed, at most 4096 files, with writtenMore: true when it wrote more. A command vc.run runs reads and writes files only where the script's File and Folder objects may and in the files the open documents were opened from or saved to, and fails with \"Script file access denied\" elsewhere, saving into a user library of swatches, brushes, symbols or graphic styles included (reading the app's own libraries is allowed). An error in another file is at that file's line and column; vc.api.version and vc.api.has(feature) describe the API. The run is one transaction: on success each changed document gets one undo step, Run Script, and each preference it changed keeps its value, and on an uncaught error, a limit or script.stop every open document and the session go back as they were; files a command wrote before that stay written and are named. While it runs, other commands that change anything end with a busy error. Commands that change anything outside the documents (script.*, except script.check, plugin.*, prefs.*, actionSet.*, shortcutSet.*, help.*, print.*, library.*, importMatches.*, edit.undoGroup.*, file.recovery.*, stroke.widthProfile.*, saving, importing or deleting a preset, text.addFontFiles, text.findFontFiles, text.rescanFonts, links.findFiles, file.place.queue, color.loadProfile, color.removeProfile, file.print, view.proofSetup, view.proofColors, view.overprintPreview, view.separationsPreview) throw. VectorCraft's own limits: 8 MiB of source a file and 32 MiB in all, 64 included or evaluated files 16 deep, 1024 paths looked up for them, 256 MiB of heap (128 MiB on the web), 2000000 host calls, 200000000 instructions without a pause, 64 levels, 16 MiB and 1048576 entries for a value crossing to or from a command, 10000 log entries and 4 MiB of log, 16 KiB of a dialog's message, 256 bytes of its title and 16 KiB of a prompt's preset (past these a RangeError), and 100000 objects and 512 MiB of new document data, past which a person is asked to continue or stop and a run with nobody to ask ends → {ok: true, name, changed: [document names], written: [paths], writtenMore?: true, log: [values], fuelUsed, hostCalls, runTimeMs (the time the script ran, its commands included, waits between frames and for a person's answer left out; 0 without a clock), timingAvailable (false on the web), hostCallCounts: {vcRun, documentReads, commandListReads, vcLog, consoleWrites, fileCalls, fileExecutes}, slowestCommands: [{id, calls, totalMs, maxMs}] (at most 5, the longest in all first, then by id), console: {lines: [text], droppedLines, truncatedLines} (the run's own output, vc.log lines and error), dialogs: [{kind, message, title, errorIcon (alert), noAsDefault (confirm), preset (prompt), answer: \"ok\" | true | false | text | null | \"stop\" | \"missing\"}] (in the order they opened, at most 10000 and 1 MiB), dialogCount, dialogsTruncated}, or, in the app, {running: true, name} and the report later in ui.inspect's scriptReport; a run that fails ends with its error, and its report there also has kind, message, line, column, file (when the error is in another file than the script, or in code an earlier run of its engine compiled), fuelLeft, fuelUsed, hostCalls and the fields above",
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
            query "script.engines",
            "List Script Engines",
            [],
            None,
            "{} the named engines (#targetengine) this session keeps, in the order they were made: a named engine keeps its globals between the runs that name it until it is dropped or the session ends; main, the engine of a script without #targetengine, is new for each run and is not listed; a session keeps at most 8, each with at most 64 MiB of heap (VectorCraft's own limits; none on the web) → {engines: [{name, heapBytes: its heap after its last run, running: a run has it now}], limit: 8, heapLimit: 67108864}",
            always,
            engines
        ),
        cmd!(
            query "script.engine.drop",
            "Drop Script Engine",
            [],
            None,
            "{name: a named engine script.engines lists} drop the engine and the globals its runs left; the next run that names it gets a new one; main, an engine no run made and the engine of the running script are refused → {dropped: true, name}",
            always,
            engine_drop
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
            query "script.grants",
            "List Script File Permissions",
            [],
            None,
            "{} the file access people allowed scripts with Always in a file dialog, kept in Script Grants.json next to the settings: each for one script file with one text and the files it includes (their SHA-256), so a changed or moved script, or a changed included file, has none, and with the SHA-256 of each source the run that kept it evaluated (files $.evalFile ran, strings given to eval and Function; at most 256), so a run loses it before it evaluates other code, or when its named engine holds code the grant was not kept for; at most 4096 → {grants: [{script: the script file's path, sha256, code: [SHA-256 of each evaluated source, of its UTF-16 code units in little-endian order], path: the file or folder allowed, write: it may be written, folder: the grant is a folder's, with everything in it (a file's grant is its own path only)}], file: the grants file's path | null (a session without a settings folder, and the web)}",
            always,
            grants
        ),
        cmd!(
            query "script.grants.revoke",
            "Revoke Script File Permissions",
            [],
            None,
            "{script?: a script file's path, path?: an allowed file or folder's path, all?: true (exactly one of all and the other two)} remove the kept file access of that script, of that path, of both together, or every one; a path is matched as script.grants lists it, after its links are followed (a path outside the automation read roots is matched as given); refused with the busy error while a script runs, as the run keeps the grants it started with → {removed: how many}",
            always,
            grants_revoke
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

fn engines(s: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> =
        s.script_engines().into_iter().map(|(name, heap, running)| json!({"name": name, "heapBytes": heap, "running": running})).collect();
    Ok(json!({"engines": list, "limit": script::MAX_ENGINES, "heapLimit": script::ENGINE_HEAP_BYTES}))
}

fn engine_drop(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "script.engine.drop";
    let name = match p.get("name") {
        Some(Value::String(name)) if !name.is_empty() => name.clone(),
        _ => return Err(bad(C, "`name` must be the name of a script engine")),
    };
    s.drop_script_engine(&name)?;
    Ok(json!({"dropped": true, "name": name}))
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

    /// Keep each script's data folder (`Folder.userData`) in the folder `dir`, and the file access
    /// people allow scripts with Always in [`script::files::GRANTS_FILE`] beside it. The folders are
    /// made when a script first needs them.
    pub fn attach_script_data(&mut self, dir: std::path::PathBuf) {
        self.script_data_dir = Some(dir);
    }

    /// The folder of the scripts' data folders ([`Session::attach_script_data`]); none on the web.
    pub fn script_data_dir(&self) -> Option<&std::path::Path> {
        self.script_data_dir.as_deref().filter(|_| !cfg!(target_arch = "wasm32"))
    }

    /// The Scripts folder ([`Session::attach_scripts_dir`]); none on the web, which has no folders.
    pub fn scripts_dir(&self) -> Option<&std::path::Path> {
        self.scripts_dir.as_deref().filter(|_| !cfg!(target_arch = "wasm32"))
    }

    /// The last run's report ([`Session::script_report`]) as a reply shows it: under automation read
    /// roots, a file the run wrote outside them (`written`) and the file a failed run stopped in
    /// (`file`, also in the run's error line in `console`) are named by their file name. What the
    /// script wrote itself (`log`, its output lines) stays as written.
    pub fn shown_script_report(&self) -> Option<Value> {
        let mut report = self.script_report()?.clone();
        if let Some(written) = report.get_mut("written").and_then(Value::as_array_mut) {
            for path in written.iter_mut() {
                if let Some(name) = path.as_str().and_then(hidden_name) {
                    *path = json!(name);
                }
            }
        }
        if let Some(file) = report.get("file").and_then(Value::as_str).map(str::to_string)
            && let Some(name) = hidden_name(&file)
        {
            // The run's error lines come after every line the script wrote. They are written again
            // from the report with the file's name, which also covers a line the console cut inside
            // the path. A console that dropped its oldest lines may hold only the last of them.
            let before = script::error_lines(&report);
            report["file"] = json!(name);
            if let Some(message) = report.get("message").and_then(Value::as_str) {
                report["message"] = json!(message.replace(&file, &name));
            }
            let after = script::error_lines(&report);
            if let Some(lines) = report.pointer_mut("/console/lines").and_then(Value::as_array_mut) {
                let kept = (1..=before.len().min(lines.len()))
                    .rev()
                    .find(|&n| lines.iter().rev().take(n).map(Value::as_str).eq(before.iter().rev().take(n).map(|l| Some(l.as_str()))))
                    .unwrap_or(0);
                lines.truncate(lines.len() - kept);
                lines.extend(after.iter().skip(after.len().saturating_sub(kept)).map(|l| json!(l)));
            }
        }
        Some(report)
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

/// The file name a reply names `path` by when the file is outside the automation read roots; `None`
/// when a reply may name it whole.
fn hidden_name(path: &str) -> Option<String> {
    let shown = super::fileio::shown_path(path);
    (shown != path).then(|| shown.into_owned())
}

fn list_installed(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(root) = s.scripts_dir() else {
        return Ok(json!({"folder": null, "scripts": [], "truncated": false, "unreadable": false}));
    };
    let folder = root.to_string_lossy();
    // The Scripts folder is the app's: a refusal names it as the request may see it.
    crate::file_access::check_read(&folder).map_err(|e| super::fileio::naming_in_error(EngineError::Other(e), &folder))?;
    let listing = script::folder::list(root);
    let scripts: Vec<Value> = listing
        .scripts()
        .into_iter()
        .map(|(folders, label, path)| json!({"path": path.to_string_lossy(), "label": label, "folders": folders}))
        .collect();
    Ok(json!({"folder": folder, "scripts": scripts, "truncated": listing.truncated, "unreadable": listing.unreadable}))
}

fn grants(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(dir) = s.script_data_dir() else { return Ok(json!({"grants": [], "file": null})) };
    let file = dir.with_file_name(script::files::GRANTS_FILE);
    // A script or a granted path outside the automation read roots is named by its file name alone.
    let mut grants = script::files::stored_grants(dir);
    for g in &mut grants {
        for key in ["script", "path"] {
            if let Some(name) = g[key].as_str().and_then(hidden_name) {
                g[key] = json!(name);
            }
        }
    }
    let file = file.to_string_lossy();
    Ok(json!({"grants": grants, "file": super::fileio::shown_path(&file)}))
}

fn grants_revoke(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "script.grants.revoke";
    let path_of = |key: &str| match p.get(key) {
        None => Ok(None),
        // A path the request may not read (outside the automation read roots, a network path among
        // them) is compared as written, without looking it up.
        Some(Value::String(t)) if !t.is_empty() => Ok(Some(
            crate::file_access::check_read(t)
                .ok()
                .and_then(|()| std::fs::canonicalize(t).ok())
                .map_or_else(|| t.clone(), |real| crate::cmd::findfiles::plain(real).to_string_lossy().into_owned()),
        )),
        Some(_) => Err(bad(C, format!("`{key}` must be a path"))),
    };
    let (script_path, path) = (path_of("script")?, path_of("path")?);
    let all = match p.get("all") {
        None => false,
        Some(Value::Bool(true)) => true,
        Some(_) => return Err(bad(C, "`all` must be true")),
    };
    if all == (script_path.is_some() || path.is_some()) {
        return Err(bad(C, "give `all`, or `script` and `path` or one of them"));
    }
    // A running script holds the grants it started with until it ends.
    if s.script_running() {
        return Err(EngineError::Busy);
    }
    let Some(dir) = s.script_data_dir() else { return Ok(json!({"removed": 0})) };
    let removed = script::files::revoke_grants(dir, script_path.as_deref(), path.as_deref())
        .map_err(|e| EngineError::Other(format!("the script file permissions could not be saved: {e}")))?;
    Ok(json!({"removed": removed}))
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
