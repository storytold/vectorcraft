//! Artwork Cleanup's panel and submenu (Object › Plug-ins › Artwork Cleanup). The panel runs the
//! Document Health Check on the selection, or the document while nothing is selected, lists what it
//! found, and shows or fixes the issues picked in the list (Fix All: every issue listed), each fix
//! one undo step. Below, the add-on's one-click tools open their options as their menu items do.

use egui::{Align, Layout, Ui};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::menus::Item;
use crate::panels::{empty_state, pstate, set_pstate};
use crate::widgets::{self, dim_label};

/// The panel's id (`window.panel {panel: "artworkCleanup"}`).
pub const ID: &str = "artworkCleanup";

/// Where the panel keeps its last check (panel state).
const STATE: &str = "artwork-cleanup";

/// The one-click tools: (menu label, command, the options their dialog opens with).
fn tools() -> [(&'static str, &'static str, Value); 3] {
    [
        ("Simplify Anchors (Keep Shape)…", "addon.cleanup.simplify", json!({"tolerance": 20, "protectCorners": false})),
        ("Rejoin Path Pieces…", "addon.cleanup.rejoin", json!({"gap": 0.1})),
        ("Deep Ungroup…", "addon.cleanup.deepUngroup", json!({"includeClipGroups": false})),
    ]
}

/// Object › Plug-ins › Artwork Cleanup.
pub fn menu() -> Vec<Item> {
    let mut items = vec![Item::Cmd("Document Health Check", "window.panel", json!({ "panel": ID })), Item::Sep];
    items.extend(tools().into_iter().map(|(label, id, p)| Item::Cmd(label, id, p)));
    items.extend([Item::Sep, Item::Cmd("Fix All Issues", "addon.cleanup.fix", Value::Null)]);
    items
}

/// The panel's last check.
#[derive(Clone, Default)]
struct Results {
    /// Whether a check has run (for this panel).
    checked: bool,
    /// The scope it looked at (`selection` or `document`).
    scope: String,
    /// The document and its revision it looked at.
    doc: usize,
    revision: u64,
    /// What it found: (kind, label, count).
    rows: Vec<(String, String, u64)>,
    /// The kinds picked in the list.
    picked: Vec<String>,
    /// What the last fix did.
    status: String,
}

/// Run the check (in `scope`, or the default one) → its results.
fn check(app: &mut VectorcraftApp, scope: Option<&str>) -> Results {
    let p = scope.map_or_else(|| json!({}), |s| json!({ "scope": s }));
    let mut r = Results { checked: true, doc: app.session.active_index().unwrap_or(0), ..Results::default() };
    r.revision = app.session.active().map_or(0, |d| d.revision);
    match app.session.execute("addon.cleanup.check", &p) {
        Ok(v) => {
            r.scope = v["scope"].as_str().unwrap_or("document").to_string();
            let row = |f: &Value| Some((f["kind"].as_str()?.to_string(), f["label"].as_str()?.to_string(), f["count"].as_u64()?));
            r.rows = v["findings"].as_array().into_iter().flatten().filter_map(row).collect();
        }
        Err(e) => app.status(e.to_string()),
    }
    r
}

/// Fix the issues of `kinds` in the last check's scope, then check that scope again.
fn fix(app: &mut VectorcraftApp, r: &Results, kinds: Vec<String>) -> Results {
    match app.run("addon.cleanup.fix", json!({ "scope": r.scope, "kinds": kinds })) {
        Ok(v) => {
            let fixed = v["total"].as_u64().unwrap_or(0).to_string();
            // A selection fixed away leaves nothing to check again: nothing is left.
            let mut again = if app.session.active().is_some_and(|d| d.selection.is_empty()) && r.scope == "selection" {
                Results { checked: true, scope: r.scope.clone(), ..Results::default() }
            } else {
                check(app, Some(&r.scope))
            };
            again.status = crate::i18n::fmt(tl!("Issues fixed: {count}"), &[("count", &fixed)]);
            again
        }
        Err(e) => {
            app.status(e);
            r.clone()
        }
    }
}

/// What the panel's buttons asked for this frame.
enum Action {
    Check,
    Show(String),
    Fix(Vec<String>),
    Tool(usize),
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        empty_state(ui, "sparkles", tl!("No document"), tl!("Open a document to check it for issues."));
        return;
    };
    let (selected, revision, doc) = (!st.selection.is_empty(), st.revision, app.session.active_index().unwrap_or(0));
    let mut r: Results = pstate(ui.ctx(), STATE);
    if r.doc != doc {
        r = Results::default();
    }
    let mut action = None;
    let width = ui.available_width();
    if widgets::flat_button(ui, if selected { tl!("Check Selection") } else { tl!("Check Document") }, width).clicked() {
        action = Some(Action::Check);
    }
    if r.checked {
        ui.add_space(4.0);
        results(ui, &mut r, &mut action);
        if r.revision != revision {
            dim_label(ui, tl!("Changed since the last check."));
        }
    }
    widgets::divider(ui);
    ui.add_enabled_ui(selected, |ui| {
        for (i, (label, ..)) in tools().iter().enumerate() {
            if widgets::flat_button(ui, tl!(label), width).clicked() {
                action = Some(Action::Tool(i));
            }
        }
    });
    match action {
        Some(Action::Check) => r = check(app, None),
        Some(Action::Show(kind)) => {
            if let Err(e) = app.run("addon.cleanup.show", json!({ "kind": kind, "scope": r.scope })) {
                app.status(e);
            }
        }
        Some(Action::Fix(kinds)) => r = fix(app, &r, kinds),
        Some(Action::Tool(i)) => {
            if let Some((label, id, p)) = tools().into_iter().nth(i) {
                let (id, p) = crate::menus::click_target(label, id, &p);
                crate::menus::invoke(app, &id, p);
            }
        }
        None => {}
    }
    set_pstate(ui.ctx(), STATE, r);
}

/// The list of what the check found (click picks a row, Ctrl/Cmd- or Shift-click adds one), and
/// Show, Fix Selected and Fix All.
fn results(ui: &mut Ui, r: &mut Results, action: &mut Option<Action>) {
    if r.rows.is_empty() {
        dim_label(ui, tl!("No issues found."));
    }
    for (kind, label, count) in &r.rows {
        let on = r.picked.contains(kind);
        let clicked = ui
            .horizontal(|ui| {
                let resp = ui.selectable_label(on, tl!(label));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| dim_label(ui, &count.to_string()));
                resp.clicked()
            })
            .inner;
        if clicked {
            match ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                true if on => r.picked.retain(|k| k != kind),
                true => r.picked.push(kind.clone()),
                false => r.picked = vec![kind.clone()],
            }
        }
    }
    if !r.status.is_empty() {
        dim_label(ui, &r.status);
    }
    ui.horizontal(|ui| {
        if ui.add_enabled_ui(r.picked.len() == 1, |ui| widgets::flat_button(ui, tl!("Show"), 56.0)).inner.clicked()
            && let Some(kind) = r.picked.first()
        {
            *action = Some(Action::Show(kind.clone()));
        }
        if ui.add_enabled_ui(!r.picked.is_empty(), |ui| widgets::flat_button(ui, tl!("Fix Selected"), 88.0)).inner.clicked() {
            *action = Some(Action::Fix(r.picked.clone()));
        }
        if ui.add_enabled_ui(!r.rows.is_empty(), |ui| widgets::flat_button(ui, tl!("Fix All"), 64.0)).inner.clicked() {
            *action = Some(Action::Fix(r.rows.iter().map(|row| row.0.clone()).collect()));
        }
    });
}

#[cfg(test)]
mod tests;
