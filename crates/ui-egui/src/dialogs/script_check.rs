//! File › Scripts › Check Script…: the report `script.check` gives on a script written for the
//! reference app, read without running it: what kind of file it is and what it needs, what it does
//! outside the document, the menu commands it runs, the document-model members it uses, its
//! directives and includes, and its problems, in the framework's hosted report with its Copy and OK.
//! `ui.checkScript {path? | data? | dataBase64?, name?}` opens it; a path also checks the files
//! the script's `#include` lines name. Without a file it asks for one, and File › Open of a `.jsx`,
//! `.jsxinc` or `.jsxbin` file opens it too.

use craft::theme::variant::Tone;
use craft::ui::patterns::form_dialogs::{HostedReport, ReportGroup, ReportNote};
use serde_json::{Value, json};

// PLAN: assumed `craft::host::PickRequest` and `crate::file_asks::{ask, open_params, Owner}` (fl.A3, fl.A23)
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};
use craft::host::PickRequest;

/// `ui.checkScript`: check the script at `path` (and the files it includes) or given as `data` or
/// `dataBase64`, and show the report; without any of them, ask for a `.jsx`, `.jsxinc` or `.jsxbin`
/// file and show the report when it is picked.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if ["path", "data", "dataBase64"].iter().any(|k| p.get(k).is_some()) {
        return check(app, p);
    }
    let request = PickRequest::open("script").filter("Scripts", vectorcraft_engine::cmd::scripts::EXTS);
    Ok(file_asks::ask(app, request, Owner::Free, |app, picked| match file_asks::open_params(&picked) {
        Some(params) => check(app, &Value::Object(params)),
        // A cancelled pick runs nothing.
        None => Ok(Value::Null),
    }))
}

/// Runs `script.check` on `p` (a path also checks its includes) and opens its report.
fn check(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut params = p.clone();
    if params.get("path").is_some()
        && let Some(o) = params.as_object_mut()
    {
        o.entry("includes").or_insert(json!(true));
    }
    let r = app.run("script.check", params)?;
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| {
            host.open(build(&r));
        });
    }
    Ok(r)
}

/// Puts the text Copy copied on the clipboard.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    if let Some(text) = dialogs.with(|host| host.get_mut::<HostedReport>().and_then(HostedReport::take_copied)) {
        app.clipboard_out = Some(text);
        app.status("Copied the script report");
    }
}

/// What need `id` means for the person reading the report.
pub(crate) fn need_label(id: &str) -> Option<&'static str> {
    Some(match id {
        "language" => tl!("Needs the script language's own objects or directives"),
        "dom" => tl!("Needs the document model"),
        "files" => tl!("Needs files"),
        "menuCommands" => tl!("Needs menu commands"),
        "dialogs" => tl!("Needs dialogs"),
        "hostOnly" => tl!("Needs features only the reference app has"),
        _ => return None,
    })
}

pub(crate) fn outside_label(id: &str) -> Option<&'static str> {
    Some(match id {
        "filesWritten" => tl!("Writes, moves or deletes files"),
        "filesOpened" => tl!("Opens files in other apps"),
        "network" => tl!("Opens network connections"),
        "nativeCode" => tl!("Loads native code"),
        "otherApps" => tl!("Sends messages to other apps"),
        "environment" => tl!("Reads or sets environment variables"),
        "unseenCode" => tl!("Runs code the checker can't read"),
        "preferences" => tl!("Changes the reference app's preferences"),
        _ => return None,
    })
}

pub(crate) fn status_label(id: &str) -> Option<&'static str> {
    Some(match id {
        "checked" => tl!("Checked"),
        "missing" => tl!("Not found"),
        "notFollowed" => tl!("Not followed: outside the script's folder"),
        "cycle" => tl!("Includes itself"),
        "notRead" => tl!("Not read"),
        "limit" => tl!("Not read: a limit was reached"),
        "unreadable" => tl!("Compiled or not text"),
        _ => return None,
    })
}

pub(crate) fn problem_label(code: &str) -> Option<&'static str> {
    Some(match code {
        "unterminatedString" => tl!("String not closed"),
        "unterminatedComment" => tl!("Comment not closed"),
        "unterminatedRegExp" => tl!("Regular expression not closed"),
        "unterminatedTemplate" => tl!("Template literal not closed"),
        "unterminatedXml" => tl!("XML literal not closed"),
        "unclosedBracket" => tl!("Bracket never closed"),
        "unexpectedBracket" => tl!("Closing bracket with no opening one"),
        "newerSyntax" => tl!("Syntax newer than ES3"),
        "unknownDirective" => tl!("Unknown directive"),
        "notUtf8" => tl!("Not valid UTF-8: unreadable bytes were replaced"),
        "mixedLineEndings" => tl!("Mixed line endings"),
        "tooDeep" => tl!("Nesting too deep to follow"),
        _ => return None,
    })
}

/// Where something is: `line 12`, or `lib/a.jsxinc, line 3` in an included file.
fn place(item: &Value) -> String {
    let line = item["line"].as_u64().map(|l| l.to_string());
    match (item["file"].as_str(), line) {
        (Some(file), Some(line)) => crate::i18n::fmt(tl!("{file}, line {line}"), &[("file", file), ("line", &line)]),
        (Some(file), None) => file.to_string(),
        (None, Some(line)) => crate::i18n::fmt(tl!("line {line}"), &[("line", &line)]),
        (None, None) => String::new(),
    }
}

/// The script's lines an entry lists: `line 4`, `lines 4, 9, 12`.
fn lines(item: &Value) -> String {
    let lines: Vec<String> = item["lines"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|l| l.to_string()).collect();
    match lines.as_slice() {
        [] => String::new(),
        [line] => crate::i18n::fmt(tl!("line {line}"), &[("line", line)]),
        _ => crate::i18n::fmt(tl!("lines {lines}"), &[("lines", &lines.join(", "))]),
    }
}

/// `count` and `lines` of an entry as a note: `3 · lines 4, 9, 12`.
fn tally(item: &Value) -> String {
    let count = item["count"].as_u64().unwrap_or_default();
    match lines(item) {
        l if l.is_empty() => count.to_string(),
        l => format!("{count} · {l}"),
    }
}

fn list<'r>(r: &'r Value, key: &str) -> &'r [Value] {
    r[key].as_array().map_or(&[][..], Vec::as_slice)
}

/// A neutral note of `text`, in the mono role when it is user data, with `detail` after it when there is one.
fn note(text: &str, code: bool, detail: &str) -> ReportNote {
    let note = ReportNote::new(Tone::Neutral, text.to_string());
    let note = if code { note.code() } else { note };
    if detail.is_empty() { note } else { note.detail(detail.to_string()) }
}

/// Adds a group under `heading` when it has notes.
fn push(groups: &mut Vec<ReportGroup>, heading: &str, notes: Vec<ReportNote>) {
    if !notes.is_empty() {
        groups.push(ReportGroup::new(heading.to_string(), notes));
    }
}

/// The groups of a script's report: its name over what it needs, then each section that has entries.
fn groups(r: &Value) -> Vec<ReportGroup> {
    let name = r["name"].as_str().unwrap_or_default();
    if r["kind"].as_str() != Some("script") {
        return vec![ReportGroup::new(name.to_string(), Vec::new())];
    }
    let mut needs: Vec<ReportNote> = list(r, "needs").iter().filter_map(Value::as_str).filter_map(need_label).map(|n| note(n, false, "")).collect();
    if needs.is_empty() {
        needs.push(note(tl!("Needs nothing beyond plain JavaScript"), false, ""));
    }
    let mut groups = vec![ReportGroup::new(name.to_string(), needs)];

    let outside = list(r, "outside").iter().filter_map(|o| {
        let label = o["id"].as_str().and_then(outside_label)?;
        let modes: Vec<&str> = list(o, "modes").iter().filter_map(Value::as_str).collect();
        let label = if modes.is_empty() { label.to_string() } else { format!("{label} ({})", modes.join(", ")) };
        Some(note(&label, false, &tally(o)))
    });
    push(&mut groups, tl!("Outside the document"), outside.collect());

    let menu = &r["menuCommands"];
    let mut menu_notes: Vec<ReportNote> =
        list(menu, "strings").iter().map(|s| note(s["value"].as_str().unwrap_or_default(), true, &tally(s))).collect();
    let dynamic = menu["dynamic"].as_u64().unwrap_or_default();
    if dynamic > 0 {
        let count = dynamic.to_string();
        menu_notes.push(note(&crate::i18n::fmt(tl!("Names the script builds as it runs: {count}"), &[("count", &count)]), false, ""));
    }
    push(&mut groups, tl!("Menu commands"), menu_notes);

    let dom = &r["dom"];
    let mut dom_notes: Vec<ReportNote> = list(dom, "members")
        .iter()
        .map(|m| note(m["path"].as_str().unwrap_or_default(), true, &m["count"].as_u64().unwrap_or_default().to_string()))
        .collect();
    let types: Vec<&str> = list(dom, "types").iter().filter_map(|n| n["name"].as_str()).collect();
    if !types.is_empty() {
        dom_notes.push(note(&crate::i18n::fmt(tl!("Object types: {names}"), &[("names", &types.join(", "))]), false, ""));
    }
    push(&mut groups, tl!("Document model"), dom_notes);

    let directives = list(r, "directives").iter().map(|d| {
        let text = format!(
            "{}{} {}",
            d["form"].as_str().unwrap_or_default(),
            d["name"].as_str().unwrap_or_default(),
            d["value"].as_str().unwrap_or_default()
        );
        note(text.trim_end(), true, &place(d))
    });
    push(&mut groups, tl!("Directives"), directives.collect());

    let includes = list(r, "includes").iter().map(|i| {
        let status = i["status"].as_str().and_then(status_label).unwrap_or_default();
        let detail = [status.to_string(), place(i)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        note(i["value"].as_str().unwrap_or_default(), true, &detail)
    });
    push(&mut groups, tl!("Includes"), includes.collect());

    let problems = list(r, "problems").iter().filter_map(|p| {
        let label = p["code"].as_str().and_then(problem_label)?;
        let label = match p["detail"].as_str() {
            Some(detail) => format!("{} {detail}", crate::i18n::colon(label)),
            None => label.to_string(),
        };
        Some(note(&label, false, &place(p)))
    });
    push(&mut groups, tl!("Problems"), problems.collect());

    if r["truncated"].as_bool() == Some(true)
        && let Some(last) = groups.last_mut()
    {
        last.notes.push(note(tl!("Some lists were cut short at their limit."), false, ""));
    }
    groups
}

/// The hosted report of `r`, `script.check`'s result: the verdict sentence and the groups.
fn build(r: &Value) -> HostedReport {
    let summary = match r["kind"].as_str() {
        Some("script") => tl!("VectorCraft read this script without running it."),
        Some("compiled") => tl!("Compiled script: ask its author for the .jsx file"),
        Some("tooLarge") => tl!("Too large to check"),
        _ => tl!("Not a script for the reference app"),
    };
    HostedReport::new(tl!("Check Script"), summary, Vec::new()).groups(groups(r))
}
