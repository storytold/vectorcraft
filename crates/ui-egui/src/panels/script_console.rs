//! Script Console panel (Window › Script Console): the lines of this session's script runs
//! (`script.console`), each marked with its run's number and its script's name: a start, the
//! script's `$.write` and `$.writeln` output and `vc.log` entries, its uncaught error, an end and a
//! report of its time and host calls. The panel lays out only the rows in view. A click selects a
//! line and Shift-click a range. Clear runs `script.console.clear`; Copy Selected and Copy All run
//! `ui.copyScriptConsole`. Closing the panel only hides it: runs go on writing to the console.

use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::script::console::{Console, ConsoleLine, Ending, Entry};

use crate::VectorcraftApp;
use crate::theme::Tokens;

pub const ID: &str = "scriptConsole";
/// Panel state: the selected lines, by their numbers ([`Console::first_number`]), first and last.
const SELECTION: &str = "script-console-selection";
/// Panel state: how many rows the last frame laid out.
pub(crate) const DRAWN: &str = "script-console-drawn";
const ROW_HEIGHT: f32 = 16.0;

/// How a run ended, in the UI language.
fn ending_label(e: Ending) -> &'static str {
    match e {
        Ending::Finished => tl!("Finished"),
        Ending::Error => tl!("Ended with an error"),
        Ending::Fuel => tl!("Out of fuel"),
        Ending::HostLimit => tl!("Ended at a host limit"),
        Ending::Limit => tl!("Ended at a limit"),
        Ending::Stopped => tl!("Stopped"),
        Ending::Internal => tl!("Ended by an internal fault"),
    }
}

/// A line as the panel shows it, in the UI language: its run's number, its script and its text.
pub(crate) fn shown(line: &ConsoleLine) -> String {
    let text = match &line.entry {
        Entry::Start => "▶".to_string(),
        Entry::Output(t) | Entry::Log(t) | Entry::Error(t) => t.clone(),
        Entry::End(e) => format!("■ {}", ending_label(*e)),
        Entry::Report { ms, host_calls, slowest } => {
            let time = ms.map_or_else(|| tl!("time unavailable").to_string(), millis);
            let calls = crate::i18n::tn(*host_calls, "{n} host call", "{n} host calls");
            let mut s = crate::i18n::fmt(tl!("{time}, {calls}"), &[("time", &time), ("calls", &calls)]);
            if !slowest.is_empty() {
                let list: Vec<String> = slowest.iter().map(|c| format!("{} ×{} {}", c.id, c.calls, millis(c.total_ms))).collect();
                s.push_str("; ");
                s.push_str(&crate::i18n::fmt(tl!("slowest: {commands}"), &[("commands", &list.join(", "))]));
            }
            s
        }
    };
    format!("#{} {}: {}", line.run, line.name, text)
}

/// A time in milliseconds, in the UI language.
fn millis(ms: u64) -> String {
    crate::i18n::fmt(tl!("{ms} ms"), &[("ms", &ms.to_string())])
}

/// The notice of the lines the console dropped and cut, or `None` when it has done neither.
fn notice(c: &Console) -> Option<String> {
    (c.dropped_lines() > 0 || c.truncated_lines() > 0).then(|| {
        let dropped = crate::i18n::tn(c.dropped_lines(), "{n} earlier line dropped", "{n} earlier lines dropped");
        let cut = crate::i18n::tn(c.truncated_lines(), "{n} line cut", "{n} lines cut");
        crate::i18n::fmt(tl!("{dropped}, {cut}"), &[("dropped", &dropped), ("cut", &cut)])
    })
}

/// `ui.copyScriptConsole {first?, last?}`: copy the console's lines numbered `first` to `last`
/// (both kept lines; default every line, with the notice of dropped and cut lines first when the
/// console dropped or cut any) to the clipboard, as the panel shows them → `{lines: n}`, where `n`
/// does not count the notice.
pub(crate) fn copy(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let c = app.session.script_console();
    let start = c.first_number();
    let end = start + c.lines().len() as u64;
    let (first, last) = (p.get("first"), p.get("last"));
    let mut text: Vec<String> = Vec::new();
    let (from, to) = if first.is_none() && last.is_none() {
        text.extend(notice(c));
        (start, end)
    } else {
        let held = |v: Option<&Value>| v.and_then(Value::as_u64).filter(|n| (start..end).contains(n));
        match (held(first), held(last)) {
            (Some(a), Some(b)) if a <= b => (a, b + 1),
            _ => return Err("`first` and `last` must be numbers of lines the console holds, first before last".to_string()),
        }
    };
    let picked = c.lines().iter().skip((from - start) as usize).take((to - from) as usize);
    let lines = picked.map(shown).collect::<Vec<_>>();
    let n = lines.len();
    text.extend(lines);
    app.clipboard_out = Some(text.join("\n"));
    Ok(json!({"lines": n}))
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let c = app.session.script_console();
    let start = c.first_number();
    let count = c.lines().len();
    let mut selection: Option<(u64, u64)> = super::pstate(ui.ctx(), SELECTION);
    // A selection whose lines were dropped or cleared is gone.
    if selection.is_some_and(|(a, b)| a < start || b >= start + count as u64) {
        selection = None;
    }
    if let Some(n) = notice(c) {
        crate::widgets::dim_label(ui, &n);
    }
    let mut clicked: Option<(u64, bool)> = None;
    let mut drawn = 0usize;
    let height = crate::widgets::panel_list_height(ui, 260.0, 0.0);
    crate::widgets::list_box(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt("script-console").max_height(height).stick_to_bottom(true).auto_shrink([false, true]).show_rows(
            ui,
            ROW_HEIGHT,
            count,
            |ui, range| {
                drawn = range.len();
                for (i, line) in c.lines().range(range.clone()).enumerate() {
                    let number = start + (range.start + i) as u64;
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
                    if selection.is_some_and(|(a, b)| (a..=b).contains(&number)) {
                        ui.painter().rect_filled(r, 0.0, t.row_selected);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 0.0, t.hover);
                    }
                    let color = match line.entry {
                        Entry::Error(_) => t.error,
                        Entry::Start | Entry::End(_) | Entry::Report { .. } => t.text_dim,
                        Entry::Output(_) | Entry::Log(_) => t.text,
                    };
                    let text = shown(line);
                    ui.painter().with_clip_rect(r).text(
                        r.left_center() + vec2(4.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &text,
                        egui::FontId::monospace(11.5),
                        color,
                    );
                    let resp = resp.on_hover_text(&text);
                    if resp.clicked() {
                        clicked = Some((number, ui.input(|i| i.modifiers.shift)));
                    }
                }
            },
        );
    });
    super::set_pstate(ui.ctx(), DRAWN, drawn);
    if let Some((number, shift)) = clicked {
        selection = Some(match (selection, shift) {
            (Some((a, b)), true) => (a.min(number), b.max(number)),
            _ => (number, number),
        });
    }
    super::set_pstate(ui.ctx(), SELECTION, selection);
    crate::widgets::bottom_bar(ui, |ui| {
        if ui.add_enabled(count > 0, egui::Button::new(tl!("Clear"))).clicked() {
            run(app, "script.console.clear", json!({}));
        }
        if ui.add_enabled(selection.is_some(), egui::Button::new(tl!("Copy Selected"))).clicked()
            && let Some((first, last)) = selection
        {
            run(app, "ui.copyScriptConsole", json!({"first": first, "last": last}));
        }
        if ui.add_enabled(count > 0, egui::Button::new(tl!("Copy All"))).clicked() {
            run(app, "ui.copyScriptConsole", json!({}));
        }
    });
}

/// The panel menu: Clear, Copy Selected and Copy All.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let count = app.session.script_console().lines().len();
    let selection: Option<(u64, u64)> = super::pstate(ui.ctx(), SELECTION);
    if crate::widgets::menu_item(ui, tl!("Clear"), count > 0, false) {
        run(app, "script.console.clear", json!({}));
    }
    if crate::widgets::menu_item(ui, tl!("Copy Selected"), selection.is_some(), false)
        && let Some((first, last)) = selection
    {
        run(app, "ui.copyScriptConsole", json!({"first": first, "last": last}));
    }
    if crate::widgets::menu_item(ui, tl!("Copy All"), count > 0, false) {
        run(app, "ui.copyScriptConsole", json!({}));
    }
}

fn run(app: &mut VectorcraftApp, id: &str, params: Value) {
    if let Err(e) = app.run(id, params) {
        app.ui.status = e;
    }
}
