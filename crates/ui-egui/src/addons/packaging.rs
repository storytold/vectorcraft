//! Packaging Tools' submenu (Object › Plug-ins › Packaging Tools) and its Fold Lines panel. The
//! panel lists the fold line roles, each with its swatch's chip and how many of the selected objects
//! have it (of the whole document while nothing is selected); clicking a role gives the selected
//! paths that role (`addon.packaging.setRole`). While the selection holds lines that fold, its Fold
//! Angle field shows the angle they fold at and sets it (`addon.packaging.setFoldAngle`, as Set Fold
//! Angle… in the submenu does), and Reset takes their own angles away. Its menu's Role Swatches…
//! picks the swatch that carries a role (`addon.packaging.setRoleSwatch`). Its Check button, like the
//! Check Fold Lines menu item, checks the fold lines (`addon.packaging.check`) and lists the issues
//! found under it (clicking one selects its objects); each document's list is kept until that
//! document changes. Its Fold Preview button, like the Fold Preview menu item, shows the blank folded
//! in 3D ([`preview`], `ui.foldPreview`).

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use craft::keymap::CommandRef;
use craft::menu::Node;
use craft::ui::buttons::Button;
use craft::ui::contract::{
    ActivateOn, Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, WordSetCx as _, clicked,
};
use craft::ui::display::{Divider, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row as Line;
use craft::ui::list::{Finding, Findings, FindingsList, FindingsWords, List, ListRow, Severity, ThumbSize};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::doc::Document;

use super::cleanup::Positional;
use crate::VectorcraftApp;
use crate::edits::{EditPath, apply_edits};
use crate::menus::{Sep, c, cp};
use crate::panels::empty_state;

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

/// Object › Plug-ins › Packaging Tools.
pub fn menu() -> Vec<Node> {
    let make = |label, role: &str| cp(label, SET_ROLE, json!({ "role": role }));
    vec![
        cp("Fold Lines", "window.panel", json!({ "panel": ID })),
        Sep,
        make("Make Cut Line", "cut"),
        make("Make Crease Line", "crease"),
        make("Make Partial Cut Line", "partialCut"),
        make("Make Perforation Line", "perforation"),
        Sep,
        c("Check Fold Lines", CHECK),
        c("Set Fold Angle…", SET_FOLD_ANGLE),
        Sep,
        cp("Fold Preview", PREVIEW, json!({})),
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

/// The Fold Lines panel's run-only state: the last check of each document and the role rows.
#[derive(Default)]
pub(crate) struct FoldLines {
    /// The last check of each document.
    checks: Checks,
    /// The role rows for the document state they were read for.
    rows: Rows,
}

/// Runs `addon.packaging.check` with `p` and shows its summary in the status bar → the check, kept
/// for the panel.
fn run_check(app: &mut VectorcraftApp, p: Value) -> Option<Arc<Checked>> {
    let r = app.run(CHECK, p).map_err(|e| app.status(e)).ok()?;
    let key = doc_key(app)?;
    let uid = key.0;
    let checked = Arc::new(Checked::of(key, &r));
    app.status(checked.summary());
    let checks = &mut app.addon_panels.fold.checks;
    // A check of a document that has since changed or closed can never be shown again.
    checks.retain(|_, c| c.key.as_ref().is_some_and(|(_, doc)| doc.strong_count() > 0));
    checks.insert(uid, Arc::clone(&checked));
    Some(checked)
}

/// Check Fold Lines from its menu item: checks with `p`, keeps the issues for the Fold Lines panel
/// and shows the panel.
pub(crate) fn check(app: &mut VectorcraftApp, p: Value) {
    if run_check(app, p).is_some()
        && let Err(e) = crate::panel_docking::reveal_panel(app, ID)
    {
        app.status(e);
    }
}

/// The last check of the active document, while the document has not changed since.
fn last_check(app: &VectorcraftApp) -> Option<Arc<Checked>> {
    let (now, current) = doc_key(app)?;
    let checked = app.addon_panels.fold.checks.get(&now)?;
    let same = match &checked.key {
        Some((uid, doc)) => *uid == now && doc.ptr_eq(&current),
        None => false,
    };
    same.then(|| Arc::clone(checked))
}

/// Selects the objects issue `n` of `checked` names; an issue that names none selects nothing.
fn select_issue(app: &mut VectorcraftApp, checked: &Checked, n: usize) {
    if let Some(issue) = checked.issues.get(n).filter(|issue| !issue.ids.is_empty())
        && let Err(e) = app.run("select.set", json!({ "ids": issue.ids }))
    {
        app.status(e);
    }
}

/// The Check button, then the last check's summary and issues (errors and warnings by their icons);
/// clicking an issue, or Enter on it, selects its objects.
fn show_check(app: &mut VectorcraftApp, ui: &mut Ui) {
    Divider::horizontal().show(ui);
    if Button::new(tl!("Check").to_string()).show(ui).clicked() {
        run_check(app, json!({}));
    }
    if Button::new(tl!("Fold Preview").to_string()).show(ui).clicked()
        && let Err(e) = app.run(PREVIEW, json!({}))
    {
        app.status(e);
    }
    let Some(checked) = last_check(app) else { return };
    Text::new(crate::i18n::msg(&checked.summary())).show(ui);
    let findings: Vec<Finding> = (0u64..)
        .zip(&checked.issues)
        .map(|(id, issue)| Finding::new(id, if issue.error { Severity::Error } else { Severity::Warning }, crate::i18n::msg(&issue.message)))
        .collect();
    let out =
        FindingsList::new(tl!("Issues").to_string()).show_items(ui, &Findings::new(&findings), &mut |key, icx| match findings.get(*key as usize) {
            Some(finding) => finding.row(&icx.words::<FindingsWords>()),
            None => ListRow::spacer(),
        });
    for event in &out.events {
        if let CollectionEvent::Activate(n) = event {
            select_issue(app, &checked, *n as usize);
        }
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
fn read_rows(app: &mut VectorcraftApp) -> Rows {
    let Some(st) = app.session.active() else { return Rows::default() };
    let key = (st.uid, st.revision, st.selection.objects.iter().map(|id| id.0).collect::<Vec<_>>());
    if app.addon_panels.fold.rows.key.as_ref() == Some(&key) {
        return app.addon_panels.fold.rows.clone();
    }
    let answer = app.session.execute("addon.packaging.roles", &json!({})).unwrap_or(Value::Null);
    let d = app.session.active().map(|st| st.doc.clone());
    let row = |r: &Value| {
        let swatch = r["swatch"].as_str()?.to_string();
        let paint = d.as_ref().and_then(|d| d.swatch(&swatch)).map(|s| s.paint.clone());
        Some((r["role"].as_str()?.to_string(), swatch, paint, r["count"].as_u64()?))
    };
    let kept = Rows {
        key: Some(key),
        rows: answer["roles"].as_array().into_iter().flatten().filter_map(row).collect(),
        selection: answer["scope"] == "selection",
        fold_lines: answer["foldLines"].as_u64().unwrap_or(0),
        angle: answer["foldAngle"].as_f64(),
        stored_angles: answer["storedAngles"].as_u64().unwrap_or(0),
    };
    app.addon_panels.fold.rows = kept.clone();
    kept
}

/// The role rows for the active document as it is now.
#[cfg(test)]
fn rows(app: &mut VectorcraftApp) -> Vec<Row> {
    read_rows(app).rows
}

/// The fields Set Fold Angle… opens with: `addon.packaging.setFoldAngle`'s options on the angle
/// the selected lines that it can change all fold at, else 90°, and those lines as the `ids` OK
/// sends, so OK changes the lines the dialog opened on even when the selection changes while it is
/// open, with the uid of their document as `__doc`, so OK changes nothing once another document is
/// the active one. The command's reason while it can't change any.
pub(crate) fn dialog_fields(app: &VectorcraftApp) -> Result<serde_json::Map<String, Value>, String> {
    let command = vectorcraft_engine::find_command(SET_FOLD_ANGLE).ok_or_else(|| format!("unknown command {SET_FOLD_ANGLE}"))?;
    (command.enabled)(&app.session)?;
    let angle = vectorcraft_engine::cmd::selection_fold_angles(&app.session).angle.unwrap_or(90.0);
    let ids: Vec<u64> = vectorcraft_engine::cmd::selection_fold_lines(&app.session).iter().map(|id| id.0).collect();
    let doc = app.session.active().map(|st| st.uid);
    match json!({ "angle": angle, "__params": { "ids": ids }, "__doc": doc }) {
        Value::Object(fields) => Ok(fields),
        _ => Err("no fields".into()),
    }
}

/// Set Fold Angle…: opens the parameter dialog on [`dialog_fields`].
pub(crate) fn open_dialog(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = dialog_fields(app)?;
    crate::dialogs::command::open(app, SET_FOLD_ANGLE, "Set Fold Angle", fields)
}

/// The Fold Angle row, while the selection holds lines that fold and can be changed: the angle
/// they all fold at (empty when they differ), which sets theirs as it is typed, as one undo step
/// that a cancel takes back, and Reset, which takes away the angles of their own.
fn show_angle(app: &mut VectorcraftApp, ui: &mut Ui, kept: &Rows) {
    if !kept.selection || kept.fold_lines == 0 {
        return;
    }
    let mut edit = None;
    let mut reset = false;
    let _ = Line::new().fill_width().show(ui, |ui, _| {
        Field::new(tl!("Fold Angle")).show(ui, |ui, _| {
            let field = NumberField::new(tl!("Fold Angle"), Bind::given(kept.angle))
                .unit_kind(UnitKind::Degrees)
                .range(FOLD_ANGLES)
                .max_decimals(angle_places(kept.angle));
            edit = Some(field.id_salt("fold-angle").show(ui).edit);
        });
        reset = clicked(&Button::new(tl!("Reset")).enabled(kept.stored_angles > 0).show(ui));
    });
    let run = |app: &mut VectorcraftApp, p: Value| {
        if let Err(e) = app.run(SET_FOLD_ANGLE, p) {
            app.status(e);
        }
    };
    // One undo step per edit of the field; a cancel reverts it.
    if let Some(edit) = edit {
        apply_edits(app, &edit, EditPath::Scope, |app, a: &Option<f64>, _| {
            if let Some(a) = a {
                run(app, json!({ "angle": a }));
            }
        });
    }
    if reset {
        run(app, json!({ "reset": true }));
    }
}

/// The label of role `role` (its id when it is none of [`ROLES`]).
fn role_label(role: &str) -> &str {
    ROLES.iter().find(|(_, id)| *id == role).map_or(role, |(label, _)| *label)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(selected) = app.session.active().map(|st| !st.selection.is_empty()) else {
        empty_state(ui, "scissors", tl!("No document"), tl!("Open a document to mark its fold lines."));
        return;
    };
    if !selected {
        Text::new(tl!("No objects are selected.").to_string()).show(ui);
    }
    let kept = read_rows(app);
    let rows = &kept.rows;
    let source = Positional { rows, label: |row| role_label(&row.0) };
    let out = List::<usize>::new(tl!("Fold line roles").to_string())
        .select(SelectMode::None)
        .activate(ActivateOn::Click)
        .enabled(selected)
        .show_items(ui, &source, &mut |key, icx| match rows.get(*key) {
            Some((role, swatch, paint, count)) => {
                let paint = paint.clone().unwrap_or(vectorcraft_color::Paint::None);
                let name = swatch.clone();
                icx.tip(move |ui| {
                    Text::new(name).show(ui);
                });
                ListRow::new(tl!(role_label(role)).to_string())
                    .thumb(ThumbSize::Small, move |cx, rect| crate::paint_preview::paint_rect(cx, rect, &paint))
                    .trailing(count.to_string())
            }
            None => ListRow::spacer(),
        });
    let picked = out.events.iter().find_map(|event| match event {
        CollectionEvent::Activate(key) => rows.get(*key).map(|row| row.0.clone()),
        _ => None,
    });
    if let Some(role) = picked
        && let Err(e) = app.run(SET_ROLE, json!({ "role": role }))
    {
        app.status(e);
    }
    show_angle(app, ui, &kept);
    show_check(app, ui);
}

/// Role Swatches… (the dialog of `addon.packaging.setRoleSwatch`): when another role than the one
/// the Swatch field was filled for (`filled_for`) is picked → that role's current swatch, which
/// the caller puts in the field. The same role again → `None`, so a swatch typed for it stays.
// PLAN: the contract (pl.CA5) gives `app: &VectorcraftApp`, but the roles answer is `Session::execute(&mut self)` and the role table is
// engine-private; this takes `&mut` until the requests file's open question 1 decides (option B reads the engine's `role_swatch`).
pub(crate) fn follow_role(app: &mut VectorcraftApp, role: &str, filled_for: &mut Option<String>) -> Option<String> {
    if filled_for.as_deref() == Some(role) {
        return None;
    }
    *filled_for = Some(role.to_string());
    let answer = app.session.execute("addon.packaging.roles", &json!({})).unwrap_or(Value::Null);
    answer["roles"].as_array().into_iter().flatten().find(|r| r["role"] == role).and_then(|r| r["swatch"].as_str()).map(str::to_string)
}

/// The panel's ≡ menu rows: Role Swatches… opens the options of `addon.packaging.setRoleSwatch` on
/// the Cut role and its current swatch (from the rows the panel last read).
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    vec![PanelRow::Command(role_swatches(app))]
}

/// Role Swatches… needs a document.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    states.set(role_swatches(app), CommandState::new().enabled(app.session.active().is_some()));
}

/// The command of the menu's Role Swatches… row.
fn role_swatches(app: &VectorcraftApp) -> CommandRef {
    let swatch = app.addon_panels.fold.rows.rows.iter().find(|r| r.0 == "cut").map_or_else(|| "Cut".to_string(), |r| r.1.clone());
    CommandRef::with_params(SET_ROLE_SWATCH, json!({ "role": "cut", "swatch": swatch }))
}

pub mod preview;

#[cfg(test)]
mod tests;
