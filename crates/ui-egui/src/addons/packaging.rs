//! Packaging Tools' submenu (Object › Plug-ins › Packaging Tools) and its Fold Lines panel. The
//! panel lists the fold line roles, each with its swatch's chip and how many of the selected objects
//! have it (of the whole document while nothing is selected); clicking a role gives the selected
//! paths that role (`addon.packaging.setRole`). Its menu's Role Swatches… picks the swatch that
//! carries a role (`addon.packaging.setRoleSwatch`). Its Check button, like the Check Fold Lines menu
//! item, checks the fold lines (`addon.packaging.check`) and lists the issues found under it, each
//! with a Select button; the list is kept until the document changes.

use std::sync::{Arc, Weak};

use egui::{Align, Layout, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::doc::Document;

use crate::VectorcraftApp;
use crate::menus::Item;
use crate::panels::{empty_state, pstate, set_pstate};
use crate::widgets::{self, dim_label};

/// The panel's id (`window.panel {panel: "foldLines"}`).
pub const ID: &str = "foldLines";
/// The command that gives objects a role.
pub const SET_ROLE: &str = "addon.packaging.setRole";
/// The command that picks a role's swatch.
pub const SET_ROLE_SWATCH: &str = "addon.packaging.setRoleSwatch";
/// The command that checks the fold lines.
pub const CHECK: &str = "addon.packaging.check";
/// The roles by name, as `addon.packaging.setRole` and Role Swatches… take them.
pub const ROLES: &[(&str, &str)] =
    &[("Cut Line", "cut"), ("Crease Line", "crease"), ("Partial Cut Line", "partialCut"), ("Perforation Line", "perforation")];

/// Where the panel keeps the last `addon.packaging.roles` answer (panel state).
const STATE: &str = "fold-lines";
/// Where the panel keeps the last check (panel state).
const CHECKED: &str = "fold-lines-check";

/// Object › Plug-ins › Packaging Tools.
pub fn menu() -> Vec<Item> {
    let make = |label, role: &str| Item::Cmd(label, SET_ROLE, json!({ "role": role }));
    vec![
        Item::Cmd("Fold Lines", "window.panel", json!({ "panel": ID })),
        Item::Sep,
        make("Make Cut Line", "cut"),
        make("Make Crease Line", "crease"),
        make("Make Partial Cut Line", "partialCut"),
        make("Make Perforation Line", "perforation"),
        Item::Sep,
        Item::Cmd("Check Fold Lines", CHECK, Value::Null),
    ]
}

/// One issue of a check: whether it is an error, its message (English) and the objects it names.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Issue {
    pub error: bool,
    pub message: String,
    pub ids: Vec<u64>,
}

/// The document a check was made on: its id, and its content as it was then. Selecting changes
/// neither, so the issues stay listed while their Select buttons are used. The content is held
/// weakly: it is compared, never kept alive.
type DocKey = (u64, Weak<Document>);

/// The last check of a document.
#[derive(Clone, Debug, Default)]
pub(crate) struct Checked {
    pub key: Option<DocKey>,
    pub panels: u64,
    pub hinges: u64,
    pub issues: Vec<Issue>,
}

impl Checked {
    /// The check `r` (an `addon.packaging.check` answer) of the document `key`.
    fn of(key: DocKey, r: &Value) -> Checked {
        let count = |k: &str| r[k].as_u64().unwrap_or(0);
        let issues: Vec<Issue> = r["issues"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| Issue {
                error: i["severity"] == "error",
                message: i["message"].as_str().unwrap_or_default().to_string(),
                ids: i["ids"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect(),
            })
            .collect();
        Checked { key: Some(key), panels: count("panels"), hinges: count("hinges"), issues }
    }

    /// The summary line, in English (the status bar and the panel show it translated).
    fn summary(&self) -> String {
        let errors = self.issues.iter().filter(|i| i.error).count();
        crate::i18n::fmt(
            "Panels: {panels}, hinges: {hinges}, errors: {errors}, warnings: {warnings}",
            &[
                ("panels", &self.panels.to_string()),
                ("hinges", &self.hinges.to_string()),
                ("errors", &errors.to_string()),
                ("warnings", &(self.issues.len() - errors).to_string()),
            ],
        )
    }
}

/// The active document's [`DocKey`].
fn doc_key(app: &VectorcraftApp) -> Option<DocKey> {
    app.session.active().map(|st| (st.uid, Arc::downgrade(&st.doc)))
}

/// Runs `addon.packaging.check` with `p` and shows its summary in the status bar → the check, kept
/// in `ctx` for the panel.
fn run_check(app: &mut VectorcraftApp, ctx: &egui::Context, p: Value) -> Option<Checked> {
    let r = app.run(CHECK, p).map_err(|e| app.status(e)).ok()?;
    let checked = Checked::of(doc_key(app)?, &r);
    app.status(checked.summary());
    set_pstate(ctx, CHECKED, checked.clone());
    Some(checked)
}

/// Check Fold Lines from its menu item: checks with `p`, keeps the issues for the Fold Lines panel
/// and shows the panel.
pub(crate) fn check(app: &mut VectorcraftApp, p: Value) {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    if run_check(app, &ctx, p).is_some()
        && let Err(e) = crate::panel_docking::reveal_panel(app, ID)
    {
        app.status(e);
    }
}

/// The last check of the active document, while the document has not changed since.
fn last_check(app: &VectorcraftApp, ctx: &egui::Context) -> Option<Checked> {
    let checked: Checked = pstate(ctx, CHECKED);
    let same = match (&checked.key, doc_key(app)) {
        (Some((uid, doc)), Some((now, current))) => *uid == now && doc.ptr_eq(&current),
        _ => false,
    };
    same.then_some(checked)
}

/// The Check button, then the last check's summary and issues (errors in the error color, warnings
/// in the warning color), each with a Select button; clicking an issue selects its objects too.
fn show_check(app: &mut VectorcraftApp, ui: &mut Ui) {
    widgets::divider(ui);
    let width = ui.available_width();
    if widgets::flat_button(ui, tl!("Check"), width).clicked() {
        run_check(app, ui.ctx(), json!({}));
    }
    let Some(checked) = last_check(app, ui.ctx()) else { return };
    ui.add_space(4.0);
    dim_label(ui, &crate::i18n::msg(&checked.summary()));
    if checked.issues.is_empty() {
        dim_label(ui, tl!("No issues found."));
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut select = None;
    for (n, issue) in checked.issues.iter().enumerate() {
        ui.horizontal(|ui| {
            let can = !issue.ids.is_empty();
            let picked = ui
                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let button = ui.add_enabled_ui(can, |ui| widgets::flat_button(ui, tl!("Select"), 56.0)).inner;
                    let text = egui::RichText::new(crate::i18n::msg(&issue.message)).color(if issue.error { t.error } else { t.warning });
                    let row =
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| ui.add(egui::Label::new(text).sense(Sense::click()).wrap())).inner;
                    button.clicked() || (row.clicked() && can)
                })
                .inner;
            if picked {
                select = Some(n);
            }
        });
    }
    if let Some(issue) = select.and_then(|n| checked.issues.get(n))
        && let Err(e) = app.run("select.set", json!({ "ids": issue.ids }))
    {
        app.status(e);
    }
}

/// One role row: (role id, swatch name, the swatch's paint if the document has it, count).
type Row = (String, String, Option<vectorcraft_color::Paint>, u64);

/// The panel's rows for one document state.
#[derive(Clone, Default)]
struct Rows {
    /// What they were read for: the document, its revision and its selection.
    key: Option<(u64, u64, Vec<u64>)>,
    rows: Vec<Row>,
}

/// The rows for the active document as it is now, read again only when it changed.
fn rows(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Row> {
    let Some(st) = app.session.active() else { return vec![] };
    let key = (st.uid, st.revision, st.selection.objects.iter().map(|id| id.0).collect::<Vec<_>>());
    let mut kept: Rows = pstate(ctx, STATE);
    if kept.key.as_ref() == Some(&key) {
        return kept.rows;
    }
    let answer = app.session.execute("addon.packaging.roles", &json!({})).unwrap_or(Value::Null);
    let d = app.session.active().map(|st| st.doc.clone());
    let row = |r: &Value| {
        let swatch = r["swatch"].as_str()?.to_string();
        let paint = d.as_ref().and_then(|d| d.swatch(&swatch)).map(|s| s.paint.clone());
        Some((r["role"].as_str()?.to_string(), swatch, paint, r["count"].as_u64()?))
    };
    kept = Rows { key: Some(key), rows: answer["roles"].as_array().into_iter().flatten().filter_map(row).collect() };
    set_pstate(ctx, STATE, kept.clone());
    kept.rows
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        empty_state(ui, "scissors", tl!("No document"), tl!("Open a document to mark its fold lines."));
        return;
    };
    let selected = !st.selection.is_empty();
    if !selected {
        dim_label(ui, tl!("No objects are selected."));
        ui.add_space(4.0);
    }
    let mut picked = None;
    for (role, swatch, paint, count) in rows(app, ui.ctx()) {
        let label = ROLES.iter().find(|(_, id)| *id == role).map_or(role.as_str(), |(l, _)| *l);
        let clicked = ui
            .add_enabled_ui(selected, |ui| {
                ui.horizontal(|ui| {
                    let (chip, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    widgets::paint_chip(ui, chip, paint.as_ref().unwrap_or(&vectorcraft_color::Paint::None));
                    let resp = ui.selectable_label(false, tl!(label)).on_hover_text(swatch.as_str());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| dim_label(ui, &count.to_string()));
                    resp.clicked()
                })
                .inner
            })
            .inner;
        if clicked {
            picked = Some(role);
        }
    }
    if let Some(role) = picked
        && let Err(e) = app.run(SET_ROLE, json!({ "role": role }))
    {
        app.status(e);
    }
    show_check(app, ui);
}

/// Role Swatches… (the dialog of `addon.packaging.setRoleSwatch`): when another role is picked, its
/// Swatch field shows that role's current swatch. `__role` keeps the role the field was filled for.
pub(crate) fn follow_role(app: &mut VectorcraftApp, d: &mut crate::state::Dialog) {
    let role = d.str("role");
    if d.str("__role") == role {
        return;
    }
    let answer = app.session.execute("addon.packaging.roles", &json!({})).unwrap_or(Value::Null);
    let current = answer["roles"].as_array().into_iter().flatten().find(|r| r["role"] == role.as_str()).and_then(|r| r["swatch"].as_str());
    if let Some(swatch) = current {
        d.fields.insert("swatch".into(), json!(swatch));
    }
    d.fields.insert("__role".into(), json!(role));
}

/// The panel's ≡ menu: Role Swatches… opens the options of `addon.packaging.setRoleSwatch` on the
/// Cut role and its current swatch.
pub fn panel_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    if widgets::menu_item(ui, tl!("Role Swatches…"), app.session.active().is_some(), false) {
        let swatch = rows(app, ui.ctx()).into_iter().find(|r| r.0 == "cut").map_or_else(|| "Cut".to_string(), |r| r.1);
        let (id, p) = crate::menus::click_target("Role Swatches…", SET_ROLE_SWATCH, &json!({ "role": "cut", "swatch": swatch }));
        crate::menus::invoke(app, &id, p);
    }
}

#[cfg(test)]
mod tests;
