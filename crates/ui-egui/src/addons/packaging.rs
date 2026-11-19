//! Packaging Tools' submenu (Object › Plug-ins › Packaging Tools) and its Fold Lines panel. The
//! panel lists the fold line roles, each with its swatch's chip and how many of the selected objects
//! have it (of the whole document while nothing is selected); clicking a role gives the selected
//! paths that role (`addon.packaging.setRole`). While the selection holds lines that fold, its Fold
//! Angle field shows the angle they fold at and sets it (`addon.packaging.setFoldAngle`, as Set Fold
//! Angle… in the submenu does), and Reset takes their own angles away. Its menu's Role Swatches…
//! picks the swatch that carries a role (`addon.packaging.setRoleSwatch`). Its Check button, like the Check Fold Lines menu
//! item, checks the fold lines (`addon.packaging.check`) and lists the issues found under it, each
//! with a Select button; each document's list is kept until that document changes. Its Fold Preview
//! button, like the Fold Preview menu item, shows the blank folded in 3D ([`preview`], `ui.foldPreview`).

use std::collections::HashMap;
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
/// The command that sets or resets the fold angle of fold lines.
pub const SET_FOLD_ANGLE: &str = "addon.packaging.setFoldAngle";

/// The fold angles `addon.packaging.setFoldAngle` takes, in degrees.
pub const FOLD_ANGLES: std::ops::RangeInclusive<f64> = -180.0..=180.0;
/// The places a fold angle field shows `angle` to: all it has, so an angle such as 179.9999999° is
/// not shown as 180°, which typing 180 into the field would then leave unchanged; at least one, the
/// place the field's fine steps (a tenth of a degree) round to.
pub fn angle_places(angle: Option<f64>) -> usize {
    // f64's Display is the shortest text that reads back as the same number, without an exponent.
    angle.map_or(0, |a| format!("{a}").split_once('.').map_or(0, |(_, places)| places.len())).max(1)
}
/// The command that shows the Fold Preview window.
pub const PREVIEW: &str = "ui.foldPreview";
/// The roles by name, as `addon.packaging.setRole` and Role Swatches… take them.
pub const ROLES: &[(&str, &str)] =
    &[("Cut Line", "cut"), ("Crease Line", "crease"), ("Partial Cut Line", "partialCut"), ("Perforation Line", "perforation")];

/// Where the panel keeps the last `addon.packaging.roles` answer (panel state).
const STATE: &str = "fold-lines";
/// Where the panel keeps the last check of each document (panel state, a [`Checks`]).
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
        Item::Cmd("Set Fold Angle…", SET_FOLD_ANGLE, Value::Null),
        Item::Sep,
        Item::Cmd("Fold Preview", PREVIEW, json!({})),
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

/// The last check of each document, by the document's id. The checks are shared, so reading the
/// map for a frame copies no issue lists.
type Checks = HashMap<u64, Arc<Checked>>;

/// The active document's [`DocKey`].
fn doc_key(app: &VectorcraftApp) -> Option<DocKey> {
    app.session.active().map(|st| (st.uid, Arc::downgrade(&st.doc)))
}

/// Runs `addon.packaging.check` with `p` and shows its summary in the status bar → the check, kept
/// in `ctx` for the panel.
fn run_check(app: &mut VectorcraftApp, ctx: &egui::Context, p: Value) -> Option<Arc<Checked>> {
    let r = app.run(CHECK, p).map_err(|e| app.status(e)).ok()?;
    let key = doc_key(app)?;
    let uid = key.0;
    let checked = Arc::new(Checked::of(key, &r));
    app.status(checked.summary());
    let mut checks: Checks = pstate(ctx, CHECKED);
    // A check of a document that has since changed or closed can never be shown again.
    checks.retain(|_, c| c.key.as_ref().is_some_and(|(_, doc)| doc.strong_count() > 0));
    checks.insert(uid, Arc::clone(&checked));
    set_pstate(ctx, CHECKED, checks);
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
fn last_check(app: &VectorcraftApp, ctx: &egui::Context) -> Option<Arc<Checked>> {
    let (now, current) = doc_key(app)?;
    let mut checks: Checks = pstate(ctx, CHECKED);
    let checked = checks.remove(&now)?;
    let same = match &checked.key {
        Some((uid, doc)) => *uid == now && doc.ptr_eq(&current),
        None => false,
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
    if widgets::flat_button(ui, tl!("Fold Preview"), width).clicked()
        && let Err(e) = app.run(PREVIEW, json!({}))
    {
        app.status(e);
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
    /// Whether the rows count the selection (not the whole document).
    selection: bool,
    /// How many lines that fold (Crease, Partial Cut, Perforation) Set Fold Angle can change.
    fold_lines: u64,
    /// The angle they all fold at (`None`: they differ, or there are none).
    angle: Option<f64>,
    /// How many of them hold a stored angle, readable or not: what Reset takes away.
    stored_angles: u64,
}

/// The rows for the active document as it is now, read again only when it changed.
fn read_rows(app: &mut VectorcraftApp, ctx: &egui::Context) -> Rows {
    let Some(st) = app.session.active() else { return Rows::default() };
    let key = (st.uid, st.revision, st.selection.objects.iter().map(|id| id.0).collect::<Vec<_>>());
    let mut kept: Rows = pstate(ctx, STATE);
    if kept.key.as_ref() == Some(&key) {
        return kept;
    }
    let answer = app.session.execute("addon.packaging.roles", &json!({})).unwrap_or(Value::Null);
    let d = app.session.active().map(|st| st.doc.clone());
    let row = |r: &Value| {
        let swatch = r["swatch"].as_str()?.to_string();
        let paint = d.as_ref().and_then(|d| d.swatch(&swatch)).map(|s| s.paint.clone());
        Some((r["role"].as_str()?.to_string(), swatch, paint, r["count"].as_u64()?))
    };
    kept = Rows {
        key: Some(key),
        rows: answer["roles"].as_array().into_iter().flatten().filter_map(row).collect(),
        selection: answer["scope"] == "selection",
        fold_lines: answer["foldLines"].as_u64().unwrap_or(0),
        angle: answer["foldAngle"].as_f64(),
        stored_angles: answer["storedAngles"].as_u64().unwrap_or(0),
    };
    set_pstate(ctx, STATE, kept.clone());
    kept
}

/// The role rows for the active document as it is now.
fn rows(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Row> {
    read_rows(app, ctx).rows
}

/// The fields Set Fold Angle… opens with: `addon.packaging.setFoldAngle`'s options on the angle
/// the selected lines that it can change all fold at, else 90°. The command's reason while it
/// can't change any.
pub(crate) fn dialog_fields(app: &VectorcraftApp) -> Result<Value, String> {
    let command = vectorcraft_engine::find_command(SET_FOLD_ANGLE).ok_or_else(|| format!("unknown command {SET_FOLD_ANGLE}"))?;
    (command.enabled)(&app.session)?;
    let angle = vectorcraft_engine::cmd::selection_fold_angles(&app.session).angle.unwrap_or(90.0);
    Ok(json!({ "__command": SET_FOLD_ANGLE, "__label": "Set Fold Angle", "angle": angle }))
}

/// The Fold Angle row, while the selection holds lines that fold and can be changed: the angle
/// they all fold at (empty when they differ), which sets theirs when changed, and Reset, which
/// takes away the angles of their own.
fn show_angle(app: &mut VectorcraftApp, ui: &mut Ui, kept: &Rows) {
    if !kept.selection || kept.fold_lines == 0 {
        return;
    }
    let mut call = None;
    ui.horizontal(|ui| {
        widgets::field_label(ui, tl!("Fold Angle"));
        if let Some(a) = widgets::mixed_field_within(ui, "fold-angle", kept.angle, "°", angle_places(kept.angle), 64.0, FOLD_ANGLES) {
            call = Some(json!({ "angle": a }));
        }
        if ui.add_enabled_ui(kept.stored_angles > 0, |ui| widgets::flat_button_fit(ui, tl!("Reset"), 56.0)).inner.clicked() {
            call = Some(json!({ "reset": true }));
        }
    });
    if let Some(p) = call
        && let Err(e) = app.run(SET_FOLD_ANGLE, p)
    {
        app.status(e);
    }
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
    let kept = read_rows(app, ui.ctx());
    for (role, swatch, paint, count) in kept.rows.iter().cloned() {
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
    show_angle(app, ui, &kept);
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

pub mod preview;

#[cfg(test)]
mod tests;
