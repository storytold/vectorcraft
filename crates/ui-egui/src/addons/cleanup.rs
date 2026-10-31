//! Artwork Cleanup's panel and submenu (Object › Plug-ins › Artwork Cleanup). The panel runs the
//! Document Health Check on the selection, or the document while nothing is selected, lists what it
//! found, and shows or fixes the issues picked in the list (Fix All: every issue listed), each fix
//! one undo step. Below, the add-on's one-click tools open their options as their menu items do,
//! and its text tools, which the panel can hide, act on the selected type.

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
/// Where the panel keeps whether its text tools are hidden (panel state).
const TEXT_HIDDEN: &str = "artwork-cleanup-text-hidden";

/// The one-click tools: (menu label, command, the options their dialog opens with).
fn tools() -> [(&'static str, &'static str, Value); 3] {
    [
        ("Simplify Anchors (Keep Shape)…", "addon.cleanup.simplify", json!({"tolerance": 20, "protectCorners": false})),
        ("Rejoin Path Pieces…", "addon.cleanup.rejoin", json!({"gap": 0.1})),
        ("Deep Ungroup…", "addon.cleanup.deepUngroup", json!({"includeClipGroups": false})),
    ]
}

/// Replace Missing Fonts' options: the default type font.
fn replace_fonts() -> Value {
    json!({ "font": vectorcraft_doc::CharStyle::default().font_family })
}

/// A row of the panel's text tools: a label above (if any), then its buttons (label, what a
/// modifier click does, command, params).
type TextRow = (Option<&'static str>, Vec<(&'static str, Option<&'static str>, &'static str, Value)>);

fn text_rows() -> [TextRow; 4] {
    let align = |a: &str| json!({ "align": a });
    let by = |b: &str| json!({ "by": b });
    [
        (
            Some("Set Alignment, Keep Position:"),
            vec![
                ("Left", None, "addon.cleanup.alignInPlace", align("left")),
                ("Center", None, "addon.cleanup.alignInPlace", align("center")),
                ("Right", None, "addon.cleanup.alignInPlace", align("right")),
            ],
        ),
        (
            Some("Break Text into:"),
            vec![
                ("Lines", Some("Shift-click: break into paragraphs"), "addon.cleanup.breakText", by("lines")),
                ("Words", None, "addon.cleanup.breakText", by("words")),
                ("Glyphs", None, "addon.cleanup.breakText", by("glyphs")),
            ],
        ),
        (
            None,
            vec![
                ("Split at Cursor", None, "addon.cleanup.splitText", json!({})),
                ("Reset Transform", Some("Alt-click: reset the horizontal scale too"), "addon.cleanup.resetTextTransform", json!({})),
            ],
        ),
        (None, vec![("Combine Point Text", None, "addon.cleanup.combineText", json!({}))]),
    ]
}

/// The text tools one to a row below the others: (label, command, params).
fn text_singles() -> [(&'static str, &'static str, Value); 3] {
    [
        ("Remove Line Breaks", "addon.cleanup.removeBreaks", json!({ "kind": "line" })),
        ("Remove Paragraph Breaks", "addon.cleanup.removeBreaks", json!({ "kind": "paragraph" })),
        ("Replace Missing Fonts…", "addon.cleanup.replaceFonts", replace_fonts()),
    ]
}

/// `p` for a click with `held`: Shift-click on Lines breaks into paragraphs, Alt-click on Reset
/// Transform resets the horizontal scale too.
fn modified(id: &str, mut p: Value, held: egui::Modifiers) -> Value {
    match id {
        "addon.cleanup.breakText" if held.shift && p["by"] == "lines" => p["by"] = json!("paragraphs"),
        "addon.cleanup.resetTextTransform" if held.alt => p["scale"] = json!(true),
        _ => {}
    }
    p
}

/// Object › Plug-ins › Artwork Cleanup.
pub fn menu() -> Vec<Item> {
    let mut items = vec![Item::Cmd("Document Health Check", "window.panel", json!({ "panel": ID })), Item::Sep];
    items.extend(tools().into_iter().map(|(label, id, p)| Item::Cmd(label, id, p)));
    items.extend([Item::Sep, Item::Cmd("Fix All Issues", "addon.cleanup.fix", Value::Null), Item::Sep]);
    let by = |b: &str| json!({ "by": b });
    let align = |a: &str| json!({ "align": a });
    items.extend([
        Item::Cmd("Combine Point Text", "addon.cleanup.combineText", json!({})),
        Item::Cmd("Break Text into Lines", "addon.cleanup.breakText", by("lines")),
        Item::Cmd("Break Text into Paragraphs", "addon.cleanup.breakText", by("paragraphs")),
        Item::Cmd("Break Text into Words", "addon.cleanup.breakText", by("words")),
        Item::Cmd("Break Text into Glyphs", "addon.cleanup.breakText", by("glyphs")),
        Item::Cmd("Split Text at Cursor", "addon.cleanup.splitText", json!({})),
        Item::Sub(
            "Set Alignment, Keep Position",
            vec![
                Item::Cmd("Left", "addon.cleanup.alignInPlace", align("left")),
                Item::Cmd("Center", "addon.cleanup.alignInPlace", align("center")),
                Item::Cmd("Right", "addon.cleanup.alignInPlace", align("right")),
            ],
        ),
        Item::Cmd("Reset Text Transform", "addon.cleanup.resetTextTransform", json!({})),
        Item::Cmd("Reset Text Transform and Scale", "addon.cleanup.resetTextTransform", json!({ "scale": true })),
    ]);
    items.extend(text_singles().into_iter().map(|(label, id, p)| Item::Cmd(label, id, p)));
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
    /// A tool's button: (its label, command, params), run as its menu item is.
    Run(&'static str, &'static str, Value),
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
        for (label, id, p) in tools() {
            if widgets::flat_button(ui, tl!(label), width).clicked() {
                action = Some(Action::Run(label, id, p));
            }
        }
    });
    let hidden: bool = pstate(ui.ctx(), TEXT_HIDDEN);
    widgets::divider(ui);
    if widgets::section_toggle(ui, "Text", !hidden) {
        set_pstate(ui.ctx(), TEXT_HIDDEN, !hidden);
    }
    if !hidden {
        let editing = app.session.tool_options().get("editing").is_some_and(|v| !v.is_null());
        text_tools(ui, width, selected, editing, &mut action);
    }
    match action {
        Some(Action::Check) => r = check(app, None),
        Some(Action::Show(kind)) => {
            if let Err(e) = app.run("addon.cleanup.show", json!({ "kind": kind, "scope": r.scope })) {
                app.status(e);
            }
        }
        Some(Action::Fix(kinds)) => r = fix(app, &r, kinds),
        Some(Action::Run(label, id, p)) => {
            let (id, p) = crate::menus::click_target(label, id, &p);
            crate::menus::invoke(app, &id, p);
        }
        None => {}
    }
    set_pstate(ui.ctx(), STATE, r);
}

/// The text tools: rows of buttons, then one to a row. Each is enabled when its command can act:
/// Split at Cursor while a Type tool edits text, Replace Missing Fonts always, the others with a
/// selection.
fn text_tools(ui: &mut Ui, width: f32, selected: bool, editing: bool, action: &mut Option<Action>) {
    let enabled = |id: &str| match id {
        "addon.cleanup.splitText" => editing,
        "addon.cleanup.replaceFonts" => true,
        _ => selected,
    };
    let mut button = |ui: &mut Ui, label: &'static str, tip: Option<&str>, id: &'static str, p: Value, w: f32| {
        // Lines and words of text, not lines drawn: their own catalog rows (the button translates
        // the others).
        let shown = if id == "addon.cleanup.breakText" { crate::i18n::tr_ctx(crate::i18n::current(), "breakText", label) } else { label };
        let mut resp = ui.add_enabled_ui(enabled(id), |ui| widgets::flat_button(ui, shown, w)).inner;
        if let Some(tip) = tip {
            resp = resp.on_hover_text(tl!(tip));
        }
        if resp.clicked() {
            *action = Some(Action::Run(label, id, modified(id, p, ui.input(|i| i.modifiers))));
        }
    };
    for (heading, buttons) in text_rows() {
        if let Some(h) = heading {
            widgets::subheader(ui, h);
        }
        ui.horizontal(|ui| {
            let n = buttons.len() as f32;
            let each = ((width - ui.spacing().item_spacing.x * (n - 1.0)) / n).max(40.0);
            for (label, tip, id, p) in buttons {
                button(ui, label, tip, id, p, each);
            }
        });
    }
    for (label, id, p) in text_singles() {
        button(ui, label, None, id, p, width);
    }
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
