//! Script Console panel (Window › Script Console): the lines of this session's script runs
//! (`script.console`), each marked with its run's number and its script's name: a start, the
//! script's `$.write` and `$.writeln` output and `vc.log` entries, its uncaught error, an end and a
//! report of its time and host calls. The panel lays out only the rows in view and follows the newest
//! line. A click selects a line, Shift-click a range and Ctrl-click adds one. Clear runs
//! `script.console.clear`; Copy Selected runs `ui.scriptConsole.copySelected` (the first to the last
//! selected line) and Copy All `ui.copyScriptConsole`. Closing the panel only hides it: runs go on
//! writing to the console.

use std::hash::{DefaultHasher, Hash, Hasher};

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::script::console::{Console, ConsoleLine, Ending, Entry};

// PLAN: assumed paths of the framework items below, re-exported by craft-ui (behavior, theme and keymap types)
use craft::behavior::{DisplayOrder, IconId, ItemSource};
use craft::keymap::CommandRef;
use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, ScrollAnchor, Show, ShowCollection};
use craft::ui::list::{List, ListRow, RowInk};
use craft::ui::menu::PanelRow;
use craft::ui::notices::Notice;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::sections::FooterBar;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;

use crate::VectorcraftApp;

pub const ID: &str = "scriptConsole";

/// The commands of the panel's footer and menu, in their order: Clear, Copy Selected, Copy All.
const COMMANDS: [&str; 3] = ["script.console.clear", "ui.scriptConsole.copySelected", "ui.copyScriptConsole"];

/// The console's lines as the list's rows: the key of a line is its number ([`Console::first_number`]).
struct Lines {
    start: u64,
    count: usize,
}

impl DisplayOrder<u64> for Lines {
    fn len(&self) -> usize {
        self.count
    }

    fn key(&self, at: usize) -> u64 {
        self.start + at as u64
    }

    fn index_of(&self, key: &u64) -> Option<usize> {
        key.checked_sub(self.start).map(|at| at as usize).filter(|&at| at < self.count)
    }
}

impl ItemSource<u64> for Lines {}

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

/// The first and last selected line that the console still holds, or `None` when none is selected.
fn selected_range(app: &VectorcraftApp) -> Option<(u64, u64)> {
    let c = app.session.script_console();
    let start = c.first_number();
    let end = start + c.lines().len() as u64;
    let kept = app.ui.script_console_selection.items.iter().copied().filter(|k| (start..end).contains(k));
    kept.fold(None, |range, k| Some(range.map_or((k, k), |(a, b)| (a.min(k), b.max(k)))))
}

/// `ui.scriptConsole.copySelected`: copy the lines from the first to the last selected one.
fn copy_selected(app: &mut VectorcraftApp) -> Result<Value, String> {
    match selected_range(app) {
        Some((first, last)) => copy(app, &json!({"first": first, "last": last})),
        // Null ends are refused by `copy`, which words the error.
        None => copy(app, &json!({"first": Value::Null, "last": Value::Null})),
    }
}

/// Runs the console's own UI commands: `Some` for `ui.scriptConsole.copySelected`, `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    (id == COMMANDS[1]).then(|| copy_selected(app))
}

/// The panel menu's rows: Clear, Copy Selected and Copy All.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    COMMANDS.iter().map(|id| PanelRow::Command(CommandRef::new(*id))).collect()
}

/// The states of the panel's commands: Clear and Copy All while there are lines, Copy Selected while a
/// selected line is still kept. The titles are the short ones of the footer.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let any = !app.session.script_console().lines().is_empty();
    let selected = selected_range(app).is_some();
    for (id, title, on) in [(COMMANDS[0], tl!("Clear"), any), (COMMANDS[1], tl!("Copy Selected"), selected), (COMMANDS[2], tl!("Copy All"), any)] {
        states.set(CommandRef::new(id), CommandState::new().enabled(on).title(title));
    }
}

/// What the panel's menu model depends on, for the menu cache's key.
pub(crate) fn key(app: &VectorcraftApp) -> u64 {
    let c = app.session.script_console();
    let mut hasher = DefaultHasher::new();
    (c.lines().len(), c.dropped_lines(), c.truncated_lines(), &app.ui.script_console_selection.items).hash(&mut hasher);
    hasher.finish()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let c = app.session.script_console();
    let start = c.first_number();
    let count = c.lines().len();
    let end = start + count as u64;
    // A selection whose lines were dropped or cleared is gone.
    let selection = &mut app.ui.script_console_selection;
    let held = |k: &u64| (start..end).contains(k);
    selection.items.retain(held);
    selection.lead = selection.lead.take().filter(held);
    selection.anchor = selection.anchor.take().filter(held);
    if let Some(n) = notice(c) {
        Notice::new(Tone::Info, n).show(ui);
    }
    List::<u64>::new(tl!("Script Console")).scroll_anchor(ScrollAnchor::End).selection(Bind::bound(&mut app.ui.script_console_selection)).show_items(
        ui,
        &Lines { start, count },
        &mut |key, icx| {
            let line = key.checked_sub(start).and_then(|at| c.lines().get(at as usize));
            let Some(line) = line else { return ListRow::new(String::new()) };
            let row = ListRow::new(shown(line)).role(TypeRole::Mono).state(icx.row.clone());
            match line.entry {
                Entry::Error(_) => row.icon(IconSpec::svg(IconId::new("error"))).tone(Tone::Bad),
                Entry::Start | Entry::End(_) | Entry::Report { .. } => row.ink(RowInk::Muted),
                Entry::Output(_) | Entry::Log(_) => row,
            }
        },
    );
    let picked = FooterBar::buttons([
        FooterButton::new(tl!("Clear"), DialogRole::Other).enabled(count > 0),
        FooterButton::new(tl!("Copy Selected"), DialogRole::Other).enabled(selected_range(app).is_some()),
        FooterButton::new(tl!("Copy All"), DialogRole::Other).enabled(count > 0),
    ])
    .show_footer(ui)
    .picked;
    if let Some(id) = picked.and_then(|at| COMMANDS.get(at)) {
        run_ui(app, id, json!({}));
    }
}

fn run_ui(app: &mut VectorcraftApp, id: &str, params: Value) {
    if let Err(e) = app.run(id, params) {
        app.ui.status = e;
    }
}
