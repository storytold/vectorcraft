//! File › Scripts › Check Script…: the report `script.check` gives on a script written for the
//! reference app, read without running it: what kind of file it is and what it needs, what it does
//! outside the document, the menu commands it runs, the document-model members it uses, its
//! directives and includes, and its problems, with Copy Report (the report as JSON) and Close.
//! `ui.checkScript {path? | data? | dataBase64?, name?}` opens it; a path also checks the files
//! the script's `#include` lines name. Without a file it asks for one (the web opens the browser's file
//! picker), and File › Open of a `.jsx`, `.jsxinc` or `.jsxbin` file opens it too.
//!
//! Fields: `report` (`script.check`'s result) and `discard` (set it and confirm for Copy Report;
//! confirming copies the report either way).

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{FilePick, VectorcraftApp, io, widgets};

/// The dialog kind of a script's report.
pub const KIND: &str = "scriptCheck";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Check Script").into(),
    body,
    confirm,
    // Close, with Copy Report beside it.
    ok: None,
    discard: Some("Copy Report"),
    min_width: 460.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

/// `ui.checkScript`: check the script at `path` (and the files it includes) or given as `data` or
/// `dataBase64`, and show the report; without any of them, pick a `.jsx`, `.jsxinc` or `.jsxbin`
/// file (the web opens File › Open's picker, and [`io::open_bytes`] shows the report).
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut params = p.clone();
    if ["path", "data", "dataBase64"].iter().all(|k| p.get(k).is_none()) {
        if app.services.open_async.is_some() {
            return io::open_dialog(app).map(|_| Value::Null);
        }
        let pick = FilePick { filters: vec![FilePick::filter("Scripts", vectorcraft_engine::cmd::scripts::EXTS)], ..Default::default() };
        params = json!({ "path": crate::picks::open(app, &pick).ok_or("cancelled")? });
    }
    if params.get("path").is_some()
        && let Some(o) = params.as_object_mut()
    {
        o.entry("includes").or_insert(json!(true));
    }
    let r = app.run("script.check", params)?;
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "report": r })));
    Ok(r)
}

/// Copy Report: the report as JSON on the clipboard. The dialog stays open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let report = d.fields.get("report").cloned().unwrap_or(Value::Null);
    app.clipboard_out = Some(serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    if let Some(open) = app.ui.dialog.as_mut().filter(|open| open.kind == KIND) {
        open.fields.remove("discard");
    }
    app.status("Copied the script report");
    Ok(report)
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

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let Some(r) = d.fields.get("report") else { return true };
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(r["name"].as_str().unwrap_or_default()).font(theme::semibold(12.5)).color(t.text_strong));
    ui.add_space(4.0);
    match r["kind"].as_str() {
        Some("script") => report(ui, r, &t),
        Some("compiled") => {
            widgets::dim_label(ui, tl!("Compiled script: ask its author for the .jsx file"));
        }
        Some("tooLarge") => {
            widgets::dim_label(ui, tl!("Too large to check"));
        }
        _ => {
            widgets::dim_label(ui, tl!("Not a script for the reference app"));
        }
    }
    false
}

/// A row of a section: `text` (user data in `mono`), then a count and lines, dimmed.
fn row(ui: &mut egui::Ui, t: &Tokens, text: &str, mono: bool, note: &str) {
    ui.horizontal_wrapped(|ui| {
        let text = egui::RichText::new(text).color(t.text_strong);
        ui.label(if mono { text.monospace() } else { text.size(12.5) });
        if !note.is_empty() {
            ui.label(egui::RichText::new(note).size(11.5).color(t.text_dim));
        }
    });
}

/// `count` and `lines` of an entry as a note: `3 · lines 4, 9, 12`.
fn tally(item: &Value) -> String {
    let count = item["count"].as_u64().unwrap_or_default();
    match lines(item) {
        l if l.is_empty() => count.to_string(),
        l => format!("{count} · {l}"),
    }
}

fn section(ui: &mut egui::Ui, heading: &str, rows: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.0);
    widgets::section_header(ui, heading);
    rows(ui);
}

fn list<'r>(r: &'r Value, key: &str) -> &'r [Value] {
    r[key].as_array().map_or(&[][..], Vec::as_slice)
}

fn report(ui: &mut egui::Ui, r: &Value, t: &Tokens) {
    widgets::dim_label(ui, tl!("VectorCraft read this script without running it."));
    let needs: Vec<&str> = list(r, "needs").iter().filter_map(Value::as_str).filter_map(need_label).collect();
    if needs.is_empty() {
        row(ui, t, tl!("Needs nothing beyond plain JavaScript"), false, "");
    }
    for need in needs {
        row(ui, t, need, false, "");
    }
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let outside = list(r, "outside");
            if !outside.is_empty() {
                section(ui, tl!("Outside the document"), |ui| {
                    for o in outside {
                        let Some(label) = o["id"].as_str().and_then(outside_label) else { continue };
                        let modes: Vec<&str> = list(o, "modes").iter().filter_map(Value::as_str).collect();
                        let label = if modes.is_empty() { label.to_string() } else { format!("{label} ({})", modes.join(", ")) };
                        row(ui, t, &label, false, &tally(o));
                    }
                });
            }
            let menu = &r["menuCommands"];
            let dynamic = menu["dynamic"].as_u64().unwrap_or_default();
            if !list(menu, "strings").is_empty() || dynamic > 0 {
                section(ui, tl!("Menu commands"), |ui| {
                    for s in list(menu, "strings") {
                        row(ui, t, s["value"].as_str().unwrap_or_default(), true, &tally(s));
                    }
                    if dynamic > 0 {
                        let count = dynamic.to_string();
                        row(ui, t, &crate::i18n::fmt(tl!("Names the script builds as it runs: {count}"), &[("count", &count)]), false, "");
                    }
                });
            }
            let dom = &r["dom"];
            let types: Vec<&str> = list(dom, "types").iter().filter_map(|n| n["name"].as_str()).collect();
            if !list(dom, "members").is_empty() || !types.is_empty() {
                section(ui, tl!("Document model"), |ui| {
                    for m in list(dom, "members") {
                        row(ui, t, m["path"].as_str().unwrap_or_default(), true, &m["count"].as_u64().unwrap_or_default().to_string());
                    }
                    if !types.is_empty() {
                        row(ui, t, &crate::i18n::fmt(tl!("Object types: {names}"), &[("names", &types.join(", "))]), false, "");
                    }
                });
            }
            let directives = list(r, "directives");
            if !directives.is_empty() {
                section(ui, tl!("Directives"), |ui| {
                    for d in directives {
                        let text = format!(
                            "{}{} {}",
                            d["form"].as_str().unwrap_or_default(),
                            d["name"].as_str().unwrap_or_default(),
                            d["value"].as_str().unwrap_or_default()
                        );
                        row(ui, t, text.trim_end(), true, &place(d));
                    }
                });
            }
            let includes = list(r, "includes");
            if !includes.is_empty() {
                section(ui, tl!("Includes"), |ui| {
                    for i in includes {
                        let status = i["status"].as_str().and_then(status_label).unwrap_or_default();
                        let note = [status.to_string(), place(i)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                        row(ui, t, i["value"].as_str().unwrap_or_default(), true, &note);
                    }
                });
            }
            let problems = list(r, "problems");
            if !problems.is_empty() {
                section(ui, tl!("Problems"), |ui| {
                    for p in problems {
                        let Some(label) = p["code"].as_str().and_then(problem_label) else { continue };
                        let label = match p["detail"].as_str() {
                            Some(detail) => format!("{label}: {detail}"),
                            None => label.to_string(),
                        };
                        row(ui, t, &label, false, &place(p));
                    }
                });
            }
            ui.add_space(6.0);
        });
    });
    if r["truncated"].as_bool() == Some(true) {
        ui.add_space(6.0);
        widgets::dim_label(ui, tl!("Some lists were cut short at their limit."));
    }
}
