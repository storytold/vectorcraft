//! Variables panel: the document's variables (what they drive, which objects) and its
//! datasets. Clicking a dataset applies it; clicking a variable selects what it binds; the
//! arrows step to the next or previous one. Everything the reference panel does here runs a
//! `variable.*` / `dataset.*` command, so the panel and an agent drive the document the same
//! way — the commands are also in Window › Variables, the palette and over MCP.
//!
//! A row's name goes in italics when an edit has moved the art away from what the row says:
//! the document then no longer matches the active dataset, and "Update Data Set" writes the
//! art back into it.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Weak};

use craft::behavior::{ColumnKey, ColumnWidth, IconId, Selection, SelectionChange, keyed};
use craft::host::PickRequest;
use craft::keymap::CommandRef;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, PopupKind, Props as _, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _, Trigger, Validity,
};
use craft::ui::display::{Icon, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Message;
use craft::ui::sections::FooterBar;
use craft::ui::style::TypeRole;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, VariableKind};
use vectorcraft_engine::DocState;
use vectorcraft_engine::cmd::{VariableSort, VariableSortKey, variable_order};

use crate::VectorcraftApp;
use crate::caches::CanvasCache;
use crate::file_asks::{self, Owner};

/// The panel id, as the dock, the Window menu and `window.panel` name it.
pub const ID: &str = "variables";

/// The variable list's columns, each with the order a press on its header sorts by.
const TYPE: ColumnKey = ColumnKey::new("type");
const NAME: ColumnKey = ColumnKey::new("name");
const OBJECT: ColumnKey = ColumnKey::new("object");
const SORTS: [(ColumnKey, VariableSortKey); 3] = [(TYPE, VariableSortKey::Type), (NAME, VariableSortKey::Name), (OBJECT, VariableSortKey::Object)];

/// The panel's own commands, for its menu rows and its bottom bar.
const UNBIND: &str = "ui.variables.unbind";
const DELETE_VARIABLE: &str = "ui.variables.deleteVariable";
const DELETE_DATA_SET: &str = "ui.variables.deleteDataSet";
const IMPORT: &str = "ui.variables.import";
const IMPORT_DATA_SETS: &str = "ui.variables.importDataSets";
const EXPORT_DATA_SETS: &str = "ui.variables.exportDataSets";
const LOAD_LIBRARY: &str = "ui.variables.loadLibrary";
const SAVE_LIBRARY: &str = "ui.variables.saveLibrary";
const SELECT_BOUND: &str = "ui.variables.selectBoundObject";
const SELECT_ALL_BOUND: &str = "ui.variables.selectAllBound";
const LOCK: &str = "variable.lock";

/// The panel menu's rows in the reference panel's order; an empty id is a separator.
const MENU: [&str; 25] = [
    "variable.define",
    "variable.rename",
    "variable.makeTextDynamic",
    "variable.makeVisibilityDynamic",
    "variable.makeImageDynamic",
    "variable.makeGraphDynamic",
    UNBIND,
    DELETE_VARIABLE,
    LOCK,
    "",
    "dataset.capture",
    "dataset.update",
    "dataset.new",
    "dataset.set",
    "dataset.rename",
    DELETE_DATA_SET,
    "",
    IMPORT,
    IMPORT_DATA_SETS,
    EXPORT_DATA_SETS,
    LOAD_LIBRARY,
    SAVE_LIBRARY,
    "",
    SELECT_BOUND,
    SELECT_ALL_BOUND,
];

/// The bottom bar's two rows of buttons: each one's icon and command.
const FOOTER: [&[(&str, &str)]; 2] = [
    &[
        ("type", "variable.makeTextDynamic"),
        ("eye", "variable.makeVisibilityDynamic"),
        ("image", "variable.makeImageDynamic"),
        ("chart-column", "variable.makeGraphDynamic"),
        ("plus", "variable.define"),
        ("link-2-off", UNBIND),
        // The bin deletes the highlighted variable only, as the reference panel's does.
        ("trash-2", DELETE_VARIABLE),
        // Drawn pressed while the variables are locked.
        ("lock", LOCK),
    ],
    // Capture records what the art holds now as a new row (paste: take it in); Update writes the art
    // back into the row that is active (save: write it out).
    &[("clipboard-paste", "dataset.capture"), ("save", "dataset.update")],
];

/// The file types of the Import… and Import Data Sets… dialogs.
const CSV: (&str, &[&str]) = ("CSV", &["csv", "tsv", "txt"]);
const XML: (&str, &[&str]) = ("XML", &["xml"]);

/// The icon of a variable's kind, so a text variable and a visibility one read apart.
fn kind_icon(kind: VariableKind) -> &'static str {
    match kind {
        VariableKind::Text => "type",
        VariableKind::Visibility => "eye",
        VariableKind::Image => "image",
        VariableKind::Graph => "chart-column",
        VariableKind::Unknown => "square-dashed",
    }
}

/// One variable row: its kind's icon, its name, and the object it drives.
#[derive(Debug, PartialEq)]
struct VarRow {
    name: String,
    kind: VariableKind,
    bound: Vec<u64>,
    /// The name of the object it drives, or "Multiple objects" when it drives more than one.
    object: String,
}

/// One dataset row: its name and how many values it carries.
#[derive(Debug, PartialEq)]
struct SetRow {
    name: String,
    values: usize,
    active: bool,
    /// False once an edit has moved the art away from what this row says.
    matches: bool,
}

/// The panel's two lists: the variables in the order `variable.sort` set, and the data sets.
#[derive(Debug, PartialEq)]
pub struct Rows {
    vars: Vec<VarRow>,
    sets: Vec<SetRow>,
}

/// Build the panel's rows. `xml_id` names a variable's one object by its XML id, as Identify
/// Objects By › XML ID does.
fn build_rows(st: &DocState, xml_id: bool) -> Rows {
    #[cfg(test)]
    BUILT.with(|n| n.set(n.get() + 1));
    // In the order `variable.sort` set, which `variable.list` reports too.
    let all = &st.doc.variables.variables;
    let vars: Vec<VarRow> = variable_order(st, xml_id)
        .into_iter()
        .filter_map(|i| all.get(i))
        .map(|v| {
            let bound: Vec<u64> = st.doc.variables.objects_of(&v.name).iter().map(|i| i.0).collect();
            let object = match bound.as_slice() {
                [] => String::new(),
                [one] => st.doc.node(NodeId(*one)).map(|n| n.identify_label(xml_id)).unwrap_or_default(),
                _ => tl!("Multiple objects").to_string(),
            };
            VarRow { name: v.name.clone(), kind: v.kind, bound, object }
        })
        .collect();
    // What the art holds now, read once for every row's drift check.
    let current = st.doc.captured_values();
    let sets: Vec<SetRow> = st
        .doc
        .variables
        .datasets
        .iter()
        .map(|d| SetRow {
            name: d.name.clone(),
            values: d.values.len(),
            active: st.doc.variables.active_dataset.as_deref() == Some(d.name.as_str()),
            matches: d.matches(&current),
        })
        .collect();
    Rows { vars, sets }
}

#[cfg(test)]
thread_local! {
    /// How many times [`build_rows`] has run on this thread.
    static BUILT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Everything the rows are built from: the document version (its address, which the held
/// [`Weak`] keeps from being reused, and its revision), the list's order, Identify Objects By, the
/// interface language that "Multiple objects" is written in, and the document's new art. New art
/// on a locked artboard is read by a data set's drift check, and selecting or deselecting it
/// changes the set in place without a new revision.
pub struct RowsKey {
    doc: Weak<Document>,
    revision: u64,
    sort: VariableSort,
    xml_id: bool,
    lang: crate::i18n::Lang,
    new_art: Arc<HashSet<NodeId>>,
}

impl RowsKey {
    fn of(st: &DocState, xml_id: bool) -> Self {
        RowsKey {
            doc: Arc::downgrade(&st.doc),
            revision: st.revision,
            sort: st.variables_sort,
            xml_id,
            lang: crate::i18n::current(),
            new_art: st.doc.new_art.get(),
        }
    }

    fn same(&self, other: &RowsKey) -> bool {
        self.doc.ptr_eq(&other.doc)
            && self.revision == other.revision
            && self.sort == other.sort
            && self.xml_id == other.xml_id
            && self.lang == other.lang
            && Arc::ptr_eq(&self.new_art, &other.new_art)
    }
}

/// The panel's rows for `st`, built again only when one of the inputs in [`RowsKey`] changes.
/// The selection and the highlighted row are not among them: the panel reads those every frame.
fn cached_rows(cache: &mut CanvasCache, st: &DocState, xml_id: bool) -> Rc<Rows> {
    let key = RowsKey::of(st, xml_id);
    if let Some((_, rows)) = cache.variable_rows.as_ref().filter(|(k, _)| k.same(&key)) {
        return rows.clone();
    }
    let rows = Rc::new(build_rows(st, xml_id));
    cache.variable_rows = Some((key, rows.clone()));
    rows
}

/// The row the panel acts on: Delete, Options… and Select Bound Object all mean it. A name
/// the document no longer defines highlights nothing (deleted from the palette, undone).
fn highlighted_row(st: &DocState) -> Option<String> {
    st.variables_highlight.clone().filter(|h| st.doc.variables.variable(h).is_some())
}

/// What a variable drives, as object ids: what Select Bound Object selects.
fn bound_objects(st: &DocState, name: &str) -> Vec<u64> {
    st.doc.variables.objects_of(name).iter().map(|i| i.0).collect()
}

/// A row click: highlight the row, and select what it drives (its bindings are left alone).
pub(crate) fn click_row(app: &mut VectorcraftApp, name: &str, bound: &[u64]) {
    app.run("variable.highlight", json!({"variable": name})).ok();
    if !bound.is_empty() {
        app.run("select.set", json!({"ids": bound})).ok();
    }
}

/// Select Bound Object: select what the highlighted variable drives; its bindings stay.
pub(crate) fn select_bound_object(app: &mut VectorcraftApp) {
    let ids = app.session.active().map(|st| highlighted_row(st).map(|h| bound_objects(st, &h))).unwrap_or_default().unwrap_or_default();
    if !ids.is_empty() {
        app.run("select.set", json!({"ids": ids})).ok();
    }
}

/// What the selection allows the bottom bar's and the panel menu's commands to do.
struct Picked {
    selected: Vec<u64>,
    has_selection: bool,
    /// Unbind needs a bound object under the selection, not just any selection.
    selection_bound: bool,
    /// Make Text Dynamic needs exactly one type object: a text variable only drives type.
    single_type: bool,
    /// Make Image Dynamic needs exactly one image object.
    single_image: bool,
    /// Make Graph Dynamic needs a selection of graph roots only.
    selection_graph: bool,
}

fn picked(st: &vectorcraft_engine::DocState) -> Picked {
    let selected: Vec<u64> = st.selection.objects.iter().map(|i| i.0).collect();
    let selection_bound = selected.iter().any(|id| st.doc.variables.bindings.contains_key(&NodeId(*id)));
    let single_type = matches!(selected.as_slice(), [one] if st.doc.node(NodeId(*one)).is_some_and(|n| matches!(n.kind, NodeKind::Text(_))));
    let single_image = matches!(selected.as_slice(), [one] if st.doc.node(NodeId(*one)).is_some_and(|n| matches!(n.kind, NodeKind::Image(_))));
    let selection_graph = !selected.is_empty() && selected.iter().all(|id| st.doc.node(NodeId(*id)).is_some_and(|n| n.extras().graph.is_some()));
    Picked { has_selection: !selected.is_empty(), selected, selection_bound, single_type, single_image, selection_graph }
}

/// Delete Variable: the highlighted variable only (the command's default); the rest stay.
pub(crate) fn delete_highlighted(app: &mut VectorcraftApp) {
    app.run("variable.delete", json!({})).ok();
}

/// Delete Data Set: the current one only (the command's default); the rest of the rows stay.
pub(crate) fn delete_active_dataset(app: &mut VectorcraftApp) {
    app.run("dataset.delete", json!({})).ok();
}

/// The Data Set box: a new name typed over the current data set's renames it with
/// `dataset.rename`, in one undo step. The same name runs nothing, so no undo step is added; an
/// empty or taken name is refused, and the status bar says why.
pub(crate) fn rename_active_dataset(app: &mut VectorcraftApp, from: &str, to: &str) {
    if from.is_empty() || to.trim() == from {
        return;
    }
    if let Err(e) = app.run("dataset.rename", json!({"name": from, "newName": to})) {
        app.status(e);
    }
}

/// What the bottom bar's buttons and the panel menu's rows may do: one rule for both.
struct RowStates {
    picked: Picked,
    /// Lock Variables: no variable is defined, deleted or renamed until they are unlocked.
    locked: bool,
    highlighted: Option<String>,
    /// The highlighted variable drives an object.
    highlighted_bound: bool,
    /// Some object is bound to a variable.
    any_bound: bool,
    has_variables: bool,
    has_sets: bool,
    active_set: bool,
}

fn row_states(st: &DocState) -> RowStates {
    let vars = &st.doc.variables;
    let highlighted = highlighted_row(st);
    RowStates {
        picked: picked(st),
        locked: vars.locked,
        highlighted_bound: highlighted.as_deref().is_some_and(|h| !vars.objects_of(h).is_empty()),
        highlighted,
        any_bound: !vars.bindings.is_empty(),
        has_variables: !vars.variables.is_empty(),
        has_sets: !vars.datasets.is_empty(),
        active_set: vars.active_dataset.is_some(),
    }
}

impl RowStates {
    /// Whether the row or button of command `id` can run.
    fn enabled(&self, id: &str) -> bool {
        let p = &self.picked;
        match id {
            "variable.makeTextDynamic" => p.single_type && !self.locked,
            "variable.makeVisibilityDynamic" => p.has_selection && !self.locked,
            "variable.makeImageDynamic" => p.single_image && !self.locked,
            "variable.makeGraphDynamic" => p.selection_graph && !self.locked,
            "variable.define" => !self.locked,
            "variable.rename" => self.has_variables && !self.locked,
            UNBIND => p.selection_bound,
            DELETE_VARIABLE => self.highlighted.is_some() && !self.locked,
            "dataset.update" | "dataset.set" | DELETE_DATA_SET => self.active_set,
            "dataset.rename" => self.has_sets,
            EXPORT_DATA_SETS | SAVE_LIBRARY => self.has_variables,
            SELECT_BOUND => self.highlighted_bound,
            SELECT_ALL_BOUND => self.any_bound,
            _ => true,
        }
    }
}

/// The name of command `id`'s menu row and button; the lock's follows `locked`.
fn title(id: &str, locked: bool) -> &'static str {
    match id {
        "variable.define" => tl!("New Variable…"),
        "variable.rename" => tl!("Variable Options…"),
        "variable.makeTextDynamic" => tl!("Make Text Dynamic"),
        "variable.makeVisibilityDynamic" => tl!("Make Visibility Dynamic"),
        "variable.makeImageDynamic" => tl!("Make Image Dynamic"),
        "variable.makeGraphDynamic" => tl!("Make Graph Dynamic"),
        UNBIND => tl!("Unbind Variable"),
        DELETE_VARIABLE => tl!("Delete Variable"),
        LOCK if locked => tl!("Unlock Variables"),
        LOCK => tl!("Lock Variables"),
        "dataset.capture" => tl!("Capture Data Set"),
        "dataset.update" => tl!("Update Data Set"),
        "dataset.new" => tl!("New Data Set…"),
        "dataset.set" => tl!("Edit Data Set…"),
        "dataset.rename" => tl!("Rename Data Set…"),
        DELETE_DATA_SET => tl!("Delete Data Set"),
        IMPORT => tl!("Import…"),
        IMPORT_DATA_SETS => tl!("Import Data Sets…"),
        EXPORT_DATA_SETS => tl!("Export Data Sets…"),
        LOAD_LIBRARY => tl!("Load Variable Library…"),
        SAVE_LIBRARY => tl!("Save Variable Library…"),
        SELECT_BOUND => tl!("Select Bound Object"),
        SELECT_ALL_BOUND => tl!("Select All Bound Objects"),
        _ => "",
    }
}

/// What a pass of the panel asks for, run once the panel is drawn.
enum Act {
    /// Run a command with its params.
    Run(&'static str, Value),
    /// The Data Set box: rename the current data set from the first name to the second.
    Rename(String, String),
    /// A click on a variable's row.
    Click(String),
    /// A double click on a variable's row: Variable Options for it.
    Options(String),
}

/// Draw document variables, data sets and their two-row footer of capture and binding controls.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        Text::new(tl!("No document")).shade(Shade::Muted).show(ui);
        return;
    };
    let xml_id = app.session.prefs.identify_objects_by == "xmlId";
    let rows = cached_rows(&mut app.canvas, st, xml_id);
    let states = row_states(st);
    let sort = st.variables_sort;
    let (mut acts, mut pressed) = (Vec::new(), None);
    Stack::new().fill().footer(|ui, _| pressed = footer(ui, &states)).show(ui, |ui, _| {
        data_set_bar(ui, &rows.sets, &mut acts);
        variable_table(ui, &rows.vars, sort, &states, &mut acts);
        Text::new(tl!("Data Sets")).role(TypeRole::Heading).show(ui);
        data_set_list(ui, &rows.sets, &mut acts);
    });
    if let Some(id) = pressed {
        activate(app, id);
    }
    for act in acts {
        match act {
            Act::Run(id, params) => {
                app.run(id, params).ok();
            }
            Act::Rename(from, to) => rename_active_dataset(app, &from, &to),
            Act::Click(name) => {
                let bound = rows.vars.iter().find(|v| v.name == name).map(|v| v.bound.clone()).unwrap_or_default();
                click_row(app, &name, &bound);
            }
            Act::Options(name) => {
                crate::dialogs::variables::open(app, "variable.rename", Some(&name));
            }
        }
    }
}

/// The current data set across the top: its name in a box that renames it, the list that chooses
/// another, and the arrows that step through them.
fn data_set_bar(ui: &mut Ui, sets: &[SetRow], acts: &mut Vec<Act>) {
    let current = sets.iter().find(|s| s.active).map(|s| s.name.as_str());
    Row::new().show(ui, |ui, _| {
        Field::new(tl!("Data Set")).show(ui, |ui, _| {
            let typed = TextField::new(
                tl!("Data Set"),
                Bind::given(current.unwrap_or_default().to_owned()).check(|name| {
                    // An empty or blank name is the box's to refuse; the engine refuses a taken one.
                    if name.trim().is_empty() { Validity::Refuse(Message::new(tl!("A data set needs a name"))) } else { Validity::Ok }
                }),
            )
            .enabled(current.is_some())
            .show(ui)
            .events
            .landed()
            .cloned();
            if let (Some(from), Some(to)) = (current, typed) {
                acts.push(Act::Rename(from.to_owned(), to));
            }
        });
        // The closed list shows no name, only its arrow; the open list marks the current data set.
        let list = IconButton::new(IconId::new("chevron-down")).label(tl!("Data Sets")).tooltip(tl!("Data Sets")).popup(PopupKind::Menu).show(ui);
        Menu::new(tl!("Data Sets")).opened_by(&list.response, Trigger::Press).show(ui, |ui, _| {
            for set in sets {
                if MenuItem::new(set.name.clone()).radio_mark(set.active).show(ui).ran && !set.active {
                    acts.push(Act::Run("dataset.select", json!({"name": set.name})));
                }
            }
        });
        for (icon, label, command) in
            [("chevron-left", tl!("Previous Data Set"), "dataset.prev"), ("chevron-right", tl!("Next Data Set"), "dataset.next")]
        {
            if IconButton::new(IconId::new(icon)).label(label).tooltip(label).enabled(!sets.is_empty()).show(ui).clicked() {
                acts.push(Act::Run(command, json!({})));
            }
        }
    });
}

/// The variables under their three sortable columns: a row's kind, its name and what it drives.
/// A click highlights the row and selects what it drives; a double click opens Variable Options.
fn variable_table(ui: &mut Ui, vars: &[VarRow], sort: VariableSort, states: &RowStates, acts: &mut Vec<Act>) {
    let columns = vec![
        ColumnSpec::new(TYPE, tl!("Type"), ColumnWidth::Controls(1.0)).icon(IconId::new("arrow-up-down")).tip(tl!("Type")).sortable(),
        ColumnSpec::new(NAME, tl!("Variables"), ColumnWidth::fill(1.0)).sortable(),
        ColumnSpec::new(OBJECT, tl!("Objects"), ColumnWidth::fill(1.0)).sortable(),
    ];
    let sorted = SORTS.iter().find(|(_, by)| *by == sort.by).map(|(key, _)| ColumnSort::new(key.clone(), sort.descending));
    let by_name: HashMap<&str, &VarRow> = vars.iter().map(|v| (v.name.as_str(), v)).collect();
    let out = Table::<String>::new(tl!("Variables"), columns)
        .select(SelectMode::Single)
        .sort(sorted)
        .empty(tl!("None"))
        .selection(Bind::given(states.highlighted.clone().map(Selection::single).unwrap_or_default()))
        .show_items(ui, &keyed(vars, |v| v.name.clone()), &mut |name, icx| {
            let Some(row) = by_name.get(name.as_str()) else { return ListRow::spacer() };
            ListRow::new(row.name.clone())
                .cells(icx.cells())
                .cell_field(TYPE, Icon::new(IconSpec::svg(IconId::new(kind_icon(row.kind)))), |_| {})
                .cell(NAME, row.name.clone())
                .cell(OBJECT, row.object.clone())
                .cell_muted(OBJECT)
        });
    for event in out.events {
        match event {
            CollectionEvent::Column(ColumnEvent::Sort { key, descending, .. }) => {
                if let Some((_, by)) = SORTS.iter().find(|(column, _)| *column == key) {
                    acts.push(Act::Run("variable.sort", json!({"by": by.id(), "descending": descending})));
                }
            }
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => acts.extend(keys.into_iter().next().map(Act::Click)),
            // A click on the highlighted row changes no selection, and still selects what it drives.
            CollectionEvent::Picked { key, .. } => acts.push(Act::Click(key)),
            CollectionEvent::Activate(name) if !states.locked => acts.push(Act::Options(name)),
            _ => {}
        }
    }
}

/// The data sets, the active one selected; a row the art has drifted from is in italics, so an edit
/// that left the document not matching it is visible. A click on another row applies it.
fn data_set_list(ui: &mut Ui, sets: &[SetRow], acts: &mut Vec<Act>) {
    let active = sets.iter().find(|s| s.active).map(|s| s.name.clone());
    let by_name: HashMap<&str, &SetRow> = sets.iter().map(|s| (s.name.as_str(), s)).collect();
    let out = List::<String>::new(tl!("Data Sets"))
        .select(SelectMode::Single)
        .empty(tl!("None"))
        .selection(Bind::given(active.clone().map(Selection::single).unwrap_or_default()))
        .show_items(ui, &keyed(sets, |s| s.name.clone()), &mut |name, _| {
            let Some(row) = by_name.get(name.as_str()) else { return ListRow::spacer() };
            ListRow::new(format!("{}{} · {}", if row.active { "● " } else { "" }, row.name, row.values)).italic(!row.matches)
        });
    for event in out.events {
        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
            && let Some(name) = keys.into_iter().next()
            && active.as_ref() != Some(&name)
        {
            acts.push(Act::Run("dataset.select", json!({"name": name})));
        }
    }
}

/// The bottom bar: make the selection dynamic, define and delete what the lists show, lock the
/// variables, and capture or update a data set. Returns the command of the button pressed.
fn footer(ui: &mut Ui, states: &RowStates) -> Option<&'static str> {
    let mut pressed = None;
    FooterBar::body().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            for row in FOOTER {
                Row::new().wrap(true).show(ui, |ui, _| {
                    for &(icon, id) in row {
                        let name = title(id, states.locked);
                        let button = IconButton::new(IconId::new(icon)).label(name).tooltip(name).enabled(states.enabled(id));
                        if button.selected(id == LOCK && states.locked).show(ui).clicked() {
                            pressed = Some(id);
                        }
                    }
                });
            }
        });
    });
    pressed
}

/// Runs the button or row of command `id`: the panel's own through [`run`], the rest as their menu
/// items do.
fn activate(app: &mut VectorcraftApp, id: &str) {
    match run(app, id, &json!({})) {
        Some(Err(e)) => app.status(e),
        Some(Ok(_)) => {}
        None => crate::menus::invoke(app, id, json!({})),
    }
}

/// The panel menu's rows.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    MENU.iter().map(|id| if id.is_empty() { PanelRow::Separator } else { PanelRow::Command(CommandRef::new(*id)) }).collect()
}

/// The titles and enabled states of [`menu_rows`]: the rules the bottom bar follows too.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let rows = app.session.active().map(row_states);
    let locked = rows.as_ref().is_some_and(|s| s.locked);
    for id in MENU.iter().filter(|id| !id.is_empty()) {
        let enabled = rows.as_ref().is_some_and(|s| s.enabled(id));
        states.set(CommandRef::new(*id), CommandState::new().title(title(id, locked)).enabled(enabled));
    }
}

/// Runs the panel's own command `id` (`ui.variables.<row>`); `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let done = |act: fn(&mut VectorcraftApp), app: &mut VectorcraftApp| {
        act(app);
        Ok(Value::Null)
    };
    Some(match id {
        UNBIND => {
            let ids = app.session.active().map(|st| picked(st).selected).unwrap_or_default();
            app.run("variable.unbind", json!({"ids": ids}))
        }
        DELETE_VARIABLE => done(delete_highlighted, app),
        DELETE_DATA_SET => done(delete_active_dataset, app),
        IMPORT => Ok(import_data_source(app)),
        IMPORT_DATA_SETS => Ok(import_datasets(app)),
        EXPORT_DATA_SETS => Ok(export_datasets(app)),
        LOAD_LIBRARY => Ok(load_library(app)),
        SAVE_LIBRARY => Ok(save_library(app)),
        SELECT_BOUND => done(select_bound_object, app),
        SELECT_ALL_BOUND => {
            let ids: Vec<u64> = app.session.active().map(|st| st.doc.variables.bindings.keys().map(|i| i.0).collect()).unwrap_or_default();
            app.run("select.set", json!({"ids": ids}))
        }
        _ => return None,
    })
}

/// Asks for a file to read with `filters` and runs `command` on it; a cancelled dialog does nothing
/// → what [`file_asks::ask`] returns.
fn ask_open(app: &mut VectorcraftApp, filters: &[(&str, &[&str])], command: &'static str) -> Value {
    let request = filters.iter().fold(PickRequest::open("variables"), |r, (name, exts)| r.filter(*name, exts));
    file_asks::ask(app, request, Owner::Document, move |app, picked| match file_asks::open_params(&picked) {
        Some(params) => app.run(command, Value::Object(params)),
        None => Ok(Value::Null),
    })
}

/// Asks for a file `name` to write with `filters` and runs `command` with its path; a cancelled
/// dialog does nothing → what [`file_asks::ask`] returns.
fn ask_save(app: &mut VectorcraftApp, name: &str, filters: &[(&str, &[&str])], command: &'static str) -> Value {
    let request = filters.iter().fold(PickRequest::save("variables").file_name(name), |r, (name, exts)| r.filter(*name, exts));
    file_asks::ask(app, request, Owner::Document, move |app, picked| match picked.first_path() {
        Some(path) => app.run(command, json!({"path": path.to_string_lossy()})),
        None => Ok(Value::Null),
    })
}

/// Import…: one file dialog for a CSV, TSV, TXT or XML file, and `variable.import` on the chosen
/// file, which reads it by its extension.
fn import_data_source(app: &mut VectorcraftApp) -> Value {
    ask_open(app, &[("Data sources", &["csv", "tsv", "txt", "xml"]), CSV, XML], "variable.import")
}

/// Import Data Sets…: a comma- or tab-separated file, and `dataset.import` on it.
fn import_datasets(app: &mut VectorcraftApp) -> Value {
    ask_open(app, &[CSV], "dataset.import")
}

/// Export Data Sets…: a path, and `dataset.export` to it.
fn export_datasets(app: &mut VectorcraftApp) -> Value {
    ask_save(app, "datasets.csv", &[("CSV", &["csv"])], "dataset.export")
}

/// Load Variable Library…: an XML file, and `variable.loadLibrary` on it.
fn load_library(app: &mut VectorcraftApp) -> Value {
    ask_open(app, &[XML], "variable.loadLibrary")
}

/// Save Variable Library…: a path for an XML file, and `variable.saveLibrary` to it.
fn save_library(app: &mut VectorcraftApp) -> Value {
    ask_save(app, "variables.xml", &[XML], "variable.saveLibrary")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::LangExt;

    fn app_with_data() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        let t = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Alice"})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
        app.session.execute("variable.bind", &json!({"variable": "Name", "ids": [t]})).unwrap();
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app
    }

    /// Frames of the panel with input events, for typing into its Data Set box and pressing its headers.
    struct Driver {
        ctx: egui::Context,
        time: f64,
    }

    impl Driver {
        fn new() -> Self {
            let ctx = crate::testing::ui_context();
            ctx.enable_accesskit();
            Self { ctx, time: 0.0 }
        }

        fn frame(&mut self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
            self.time += 0.1;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 600.0));
            let input = egui::RawInput { events, time: Some(self.time), screen_rect: Some(screen), ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| show(app, ui));
            out.textures_delta.clear();
            out
        }

        /// The center of the panel's one text box, the Data Set box.
        fn data_set_box(&mut self, app: &mut VectorcraftApp) -> egui::Pos2 {
            use egui::accesskit::Role;
            let nodes = self.nodes(app);
            let (_, node) = nodes.iter().find(|(_, n)| matches!(n.role(), Role::TextInput | Role::MultilineTextInput)).expect("the Data Set box");
            center(node)
        }

        /// Clicks the Data Set box, replaces its text with `text` and presses Enter.
        fn type_name(&mut self, app: &mut VectorcraftApp, text: &str) {
            use egui::{Event, Key, Modifiers, PointerButton};
            let pos = self.data_set_box(app);
            let (button, none) = (PointerButton::Primary, Modifiers::default());
            self.frame(app, vec![Event::PointerMoved(pos), Event::PointerButton { pos, button, pressed: true, modifiers: none }]);
            self.frame(app, vec![Event::PointerButton { pos, button, pressed: false, modifiers: none }]);
            let all = Modifiers { ctrl: true, command: true, ..Default::default() };
            let select_all = Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: all };
            self.frame(app, vec![select_all, Event::Text(text.into())]);
            self.frame(app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: none }]);
            self.frame(app, vec![]);
        }

        /// The panel's accessibility nodes after two frames without input.
        fn nodes(&mut self, app: &mut VectorcraftApp) -> Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> {
            self.frame(app, vec![]);
            let out = self.frame(app, vec![]);
            out.platform_output.accesskit_update.expect("an accessibility tree").nodes
        }

        /// Clicks the center of `node`.
        fn click(&mut self, app: &mut VectorcraftApp, node: &egui::accesskit::Node) {
            use egui::{Event, Modifiers, PointerButton};
            let pos = center(node);
            let (button, none) = (PointerButton::Primary, Modifiers::default());
            self.frame(app, vec![Event::PointerMoved(pos), Event::PointerButton { pos, button, pressed: true, modifiers: none }]);
            self.frame(app, vec![Event::PointerButton { pos, button, pressed: false, modifiers: none }]);
            self.frame(app, vec![]);
        }
    }

    /// The center of `node`'s bounds.
    fn center(node: &egui::accesskit::Node) -> egui::Pos2 {
        let b = node.bounds().expect("bounds");
        egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5)
    }

    fn set_names(app: &mut VectorcraftApp) -> Vec<String> {
        let sets = app.run("dataset.list", json!({})).unwrap();
        sets["datasets"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap_or_default().to_string()).collect()
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// The same name adds no undo step; an empty name is refused by the box and a taken one by the
    /// engine, whose reason the status bar shows, and the names stay as they were. A new name renames
    /// the current data set in one undo step.
    #[test]
    fn the_data_set_box_refuses_an_empty_or_taken_name() {
        let mut app = app_with_data();
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app.session.execute("dataset.select", &json!({"name": "Data Set 1"})).unwrap();
        let before = undo_steps(&app);
        let mut d = Driver::new();
        d.type_name(&mut app, "Data Set 1");
        assert_eq!(undo_steps(&app), before, "the same name runs nothing");
        for name in ["   ", "Data Set 2"] {
            app.ui.status.clear();
            d.type_name(&mut app, name);
            assert_eq!(set_names(&mut app), ["Data Set 1", "Data Set 2"], "{name:?}");
            assert_eq!(undo_steps(&app), before, "{name:?}");
        }
        assert!(!app.ui.status.is_empty(), "the status bar says why a taken name is refused");
        d.type_name(&mut app, "Spring");
        assert_eq!(set_names(&mut app), ["Spring", "Data Set 2"]);
        assert_eq!(undo_steps(&app), before + 1, "a new name renames in one undo step");
        // Through the helper too: an unchanged or padded name runs nothing.
        rename_active_dataset(&mut app, "Spring", " Spring ");
        assert_eq!(undo_steps(&app), before + 1);
    }

    /// A press on a column header sorts the list by that column, and a second press on it reverses
    /// the order, as `variable.list` reports it; sorting is not an edit.
    #[test]
    fn a_header_click_sorts_the_list_and_a_second_click_reverses_it() {
        use egui::accesskit::Role;
        let mut app = app_with_data();
        let t = app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "Bob"})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("layer.setProps", &json!({"id": t, "name": "Aaron"})).unwrap();
        let alice = app.session.doc().unwrap().doc.variables.objects_of("Name")[0];
        app.session.execute("layer.setProps", &json!({"id": alice.0, "name": "Bella"})).unwrap();
        app.session.execute("variable.define", &json!({"name": "age", "kind": "visibility"})).unwrap();
        app.session.execute("variable.bind", &json!({"variable": "age", "ids": [t]})).unwrap();
        let listed = |app: &mut VectorcraftApp| -> Vec<String> {
            let list = app.run("variable.list", json!({})).unwrap();
            list["variables"].as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap().to_string()).collect()
        };
        let sort = |app: &VectorcraftApp| {
            let s = app.session.doc().unwrap().variables_sort;
            (s.by, s.descending)
        };
        let steps = undo_steps(&app);
        let mut d = Driver::new();
        // Objects: Aaron (age's type) before Bella (Name's); Type: text before visibility.
        for (header, key, ascending) in [
            ("Variables", VariableSortKey::Name, ["age", "Name"]),
            ("Objects", VariableSortKey::Object, ["age", "Name"]),
            ("Type", VariableSortKey::Type, ["Name", "age"]),
        ] {
            for descending in [false, true] {
                let nodes = d.nodes(&mut app);
                let (_, cell) = nodes.iter().find(|(_, n)| n.role() == Role::ColumnHeader && n.label() == Some(header)).expect("the header");
                let cell = cell.clone();
                d.click(&mut app, &cell);
                assert_eq!(sort(&app), (key, descending), "{header}");
                let mut want = ascending.map(str::to_string).to_vec();
                if descending {
                    want.reverse();
                }
                assert_eq!(listed(&mut app), want, "{header}");
            }
        }
        assert_eq!(undo_steps(&app), steps, "no undo step");
    }

    /// Choosing a data set, or stepping to one with the arrows, shows a status message when no
    /// object is bound to a variable it has a value for, for example after Import Data Sets…
    /// defines new variables. With such an object bound, choosing a data set or stepping to one
    /// clears the message.
    #[test]
    fn a_data_set_with_nothing_bound_shows_a_status_message() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        let t = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "x"})).unwrap()["id"].as_u64().unwrap();
        app.run("dataset.import", json!({"csv": "Name\nAnn\nBob\n"})).unwrap();
        let (a, b) = ("Data Set 1", "Data Set 2");
        let message = |row: &str| format!("nothing in data set {row} was applied: no object is bound to a variable it has a value for");
        app.run("dataset.select", json!({"name": a})).unwrap();
        assert_eq!(app.ui.status, message(a));
        app.run("dataset.next", json!({})).unwrap();
        assert_eq!(app.ui.status, message(b));
        app.run("dataset.prev", json!({})).unwrap();
        assert_eq!(app.ui.status, message(a));

        app.run("variable.bind", json!({"variable": "Name", "ids": [t]})).unwrap();
        app.run("dataset.select", json!({"name": b})).unwrap();
        assert_eq!(app.ui.status, "");
        // Stepping to a data set that applies clears the message as well.
        for step in ["dataset.next", "dataset.prev"] {
            app.run("variable.unbind", json!({"variable": "Name", "ids": [t]})).unwrap();
            app.run("dataset.select", json!({"name": a})).unwrap();
            assert_eq!(app.ui.status, message(a));
            app.run("variable.bind", json!({"variable": "Name", "ids": [t]})).unwrap();
            app.run(step, json!({})).unwrap();
            assert_eq!(app.ui.status, "", "{step}");
        }
    }

    /// The panel menu's rows each have a state, and the rows that act on the selection, the highlight,
    /// the bindings or the variables follow them and the lock, as the bottom bar's buttons do; the
    /// Lock row reads Unlock Variables once they are locked.
    #[test]
    fn the_variables_menu_rows_follow_the_row_states() {
        let states = |app: &VectorcraftApp| {
            let mut states = CommandStates::default();
            menu_states(app, &mut states);
            states
        };
        let on = |app: &VectorcraftApp, id: &str| states(app).get(&CommandRef::new(id)).is_some_and(CommandState::is_enabled);
        let check = |app: &VectorcraftApp, rows: &[(&str, bool)], when: &str| {
            for (id, want) in rows {
                assert_eq!(on(app, id), *want, "{id} {when}");
            }
        };
        let mut app = app_with_data();
        let listed = states(&app);
        for row in menu_rows(&app) {
            if let PanelRow::Command(c) = row {
                assert!(listed.get(&c).is_some(), "{} has no state", c.id);
            }
        }
        let t = app.session.doc().unwrap().doc.variables.objects_of("Name")[0].0;
        // The text just made is selected.
        app.run("select.none", json!({})).unwrap();
        let nothing_picked =
            [("variable.makeTextDynamic", false), (UNBIND, false), (DELETE_VARIABLE, false), (SELECT_BOUND, false), (SELECT_ALL_BOUND, true)];
        check(&app, &nothing_picked, "with nothing selected or highlighted");
        app.run("select.set", json!({"ids": [t]})).unwrap();
        app.run("variable.highlight", json!({"variable": "Name"})).unwrap();
        check(&app, &[("variable.makeTextDynamic", true), (UNBIND, true), (DELETE_VARIABLE, true), (SELECT_BOUND, true)], "on the bound text");
        app.run("variable.lock", json!({})).unwrap();
        let locked =
            [("variable.makeTextDynamic", false), ("variable.define", false), ("variable.rename", false), (DELETE_VARIABLE, false), (UNBIND, true)];
        check(&app, &locked, "while locked");
        assert_eq!(states(&app).get(&CommandRef::new(LOCK)), Some(&CommandState::new().title(tl!("Unlock Variables")).enabled(true)));
        let none = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        check(&none, &[("variable.define", false), ("dataset.capture", false)], "without a document");
    }

    /// What the panel acts on is the highlight, and only while it names a variable: a stale one
    /// (deleted elsewhere, undone) highlights nothing, so Delete and Options… stay disabled.
    #[test]
    fn a_highlight_of_a_deleted_variable_highlights_nothing() {
        let mut app = app_with_data();
        app.session.execute("variable.highlight", &json!({"variable": "Name"})).unwrap();
        assert_eq!(highlighted_row(app.session.doc().unwrap()).as_deref(), Some("Name"));
        assert_eq!(bound_objects(app.session.doc().unwrap(), "Name").len(), 1);
        app.session.execute("variable.delete", &json!({"names": ["Name"]})).unwrap();
        assert!(highlighted_row(app.session.doc().unwrap()).is_none());
    }

    /// The reviewer's regression case, through the calls the panel and its menu make: highlight
    /// a variable, Select Bound Object, then delete one variable and the current dataset. The
    /// selection lands on what the row drives with the binding intact, and the others remain.
    #[test]
    fn selecting_and_deleting_act_on_the_highlighted_row_only() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        let a = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Alice"})).unwrap()["id"].as_u64().unwrap();
        let b = app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "Bob"})).unwrap()["id"].as_u64().unwrap();
        for (name, id) in [("Name", a), ("Other", b)] {
            app.session.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
            app.session.execute("variable.bind", &json!({"variable": name, "ids": [id]})).unwrap();
        }
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app.session.execute("dataset.capture", &json!({})).unwrap();
        app.session.execute("dataset.select", &json!({"name": "Data Set 1"})).unwrap();

        // A row click highlights; Select Bound Object selects what it drives, binding intact.
        click_row(&mut app, "Name", &[a]);
        assert_eq!(app.session.doc().unwrap().selection.objects, vec![NodeId(a)]);
        assert!(!app.session.doc().unwrap().doc.variables.objects_of("Name").is_empty());
        select_bound_object(&mut app);
        assert_eq!(app.session.doc().unwrap().selection.objects, vec![NodeId(a)]);
        assert!(!app.session.doc().unwrap().doc.variables.objects_of("Name").is_empty());

        // Delete Variable deletes the highlighted one; Delete Data Set the current one.
        delete_highlighted(&mut app);
        delete_active_dataset(&mut app);
        let vars = app.run("variable.list", json!({})).unwrap();
        assert_eq!(vars["variables"].as_array().unwrap().len(), 1);
        assert_eq!(vars["variables"][0]["name"], "Other");
        let sets = app.run("dataset.list", json!({})).unwrap();
        assert_eq!(sets["datasets"].as_array().unwrap().len(), 1);
        assert_eq!(sets["datasets"][0]["name"], "Data Set 2");
    }

    /// A click on the highlighted row selects what it drives again, as a click on any other row
    /// does: after the art is deselected on the canvas, the row stays highlighted, and a second
    /// click on it selects the bound object.
    #[test]
    fn a_click_on_the_highlighted_row_selects_its_bound_object_again() {
        use egui::accesskit::Role;
        let mut app = app_with_data();
        let bound = app.session.doc().unwrap().doc.variables.objects_of("Name")[0];
        let mut d = Driver::new();
        let row = |d: &mut Driver, app: &mut VectorcraftApp| {
            let nodes = d.nodes(app);
            nodes.iter().find(|(_, n)| n.role() != Role::ColumnHeader && n.label() == Some("Name")).expect("the Name row").1.clone()
        };
        let selected = |app: &VectorcraftApp| app.session.doc().unwrap().selection.objects.clone();
        app.session.execute("select.set", &json!({"ids": []})).unwrap();
        let cell = row(&mut d, &mut app);
        d.click(&mut app, &cell);
        assert_eq!(highlighted_row(app.session.doc().unwrap()).as_deref(), Some("Name"));
        assert_eq!(selected(&app), vec![bound], "the first click selects the bound object");
        app.session.execute("select.set", &json!({"ids": []})).unwrap();
        assert!(selected(&app).is_empty());
        let cell = row(&mut d, &mut app);
        d.click(&mut app, &cell);
        assert_eq!(highlighted_row(app.session.doc().unwrap()).as_deref(), Some("Name"));
        assert_eq!(selected(&app), vec![bound], "a click on the highlighted row selects the bound object again");
    }

    /// The rows as the panel built them on every frame before [`cached_rows`] kept them: the
    /// identity checks compare against it.
    fn old_rows(st: &DocState, xml_id: bool) -> Rows {
        let all = &st.doc.variables.variables;
        let vars: Vec<VarRow> = variable_order(st, xml_id)
            .into_iter()
            .filter_map(|i| all.get(i))
            .map(|v| {
                let bound: Vec<u64> = st.doc.variables.objects_of(&v.name).iter().map(|i| i.0).collect();
                let object = match bound.as_slice() {
                    [] => String::new(),
                    [one] => st.doc.node(NodeId(*one)).map(|n| n.identify_label(xml_id)).unwrap_or_default(),
                    _ => tl!("Multiple objects").to_string(),
                };
                VarRow { name: v.name.clone(), kind: v.kind, bound, object }
            })
            .collect();
        let current = st.doc.captured_values();
        let sets: Vec<SetRow> = st
            .doc
            .variables
            .datasets
            .iter()
            .map(|d| SetRow {
                name: d.name.clone(),
                values: d.values.len(),
                active: st.doc.variables.active_dataset.as_deref() == Some(d.name.as_str()),
                matches: d.matches(&current),
            })
            .collect();
        Rows { vars, sets }
    }

    fn built() -> u64 {
        BUILT.with(|n| n.get())
    }

    /// Frames drawn while nothing changes build the rows once and draw the same rows after that.
    #[test]
    fn frames_without_a_change_build_the_rows_once() {
        let mut app = app_with_data();
        let mut d = Driver::new();
        d.frame(&mut app, vec![]);
        let before = built();
        for _ in 0..5 {
            d.frame(&mut app, vec![]);
        }
        assert_eq!(built(), before, "frames of an unchanged document built the rows again");
        let st = app.session.active().unwrap();
        assert!(Rc::ptr_eq(&cached_rows(&mut app.canvas, st, false), &cached_rows(&mut app.canvas, st, false)));
        assert_eq!(*cached_rows(&mut app.canvas, st, false), old_rows(st, false));
    }

    /// Draws a frame after `what` changed one of the rows' inputs, checks that the frame built the
    /// rows again and that they equal the rows the old per-frame build gives, and returns them.
    fn rebuilt(d: &mut Driver, app: &mut VectorcraftApp, what: &str) -> Rc<Rows> {
        let before = built();
        d.frame(app, vec![]);
        assert!(built() > before, "{what}: the frame drew the rows it kept");
        let xml_id = app.session.prefs.identify_objects_by == "xmlId";
        let st = app.session.active().unwrap();
        let rows = cached_rows(&mut app.canvas, st, xml_id);
        assert_eq!(*rows, old_rows(st, xml_id), "{what}");
        rows
    }

    fn matches(rows: &Rows, set: &str) -> bool {
        rows.sets.iter().find(|s| s.name == set).unwrap_or_else(|| panic!("no data set {set}")).matches
    }

    /// Each input of the rows builds them again: an edit, the list's order, Identify Objects By,
    /// the interface language, new art on a locked artboard selected or deselected without a new
    /// revision, Undo, Redo and another document.
    #[test]
    fn each_input_of_the_rows_builds_them_again() {
        let mut app = app_with_data();
        let mut d = Driver::new();
        d.frame(&mut app, vec![]);
        let t = app.session.active().unwrap().doc.variables.objects_of("Name")[0].0;

        app.session.execute("variable.define", &json!({"name": "Shown", "kind": "visibility"})).unwrap();
        assert_eq!(rebuilt(&mut d, &mut app, "variable.define").vars.len(), 2);
        let r = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("variable.bind", &json!({"variable": "Shown", "ids": [r]})).unwrap();
        let rows = rebuilt(&mut d, &mut app, "variable.bind");
        assert_eq!(rows.vars.iter().find(|v| v.name == "Shown").map(|v| v.bound.clone()), Some(vec![r]));
        app.session.execute("dataset.capture", &json!({})).unwrap();
        assert_eq!(rebuilt(&mut d, &mut app, "dataset.capture").sets.len(), 2);
        app.session.execute("text.editRange", &json!({"id": t, "insert": "Bob"})).unwrap();
        assert!(!matches(&rebuilt(&mut d, &mut app, "an edit of bound text"), "Data Set 1"));

        // On a locked artboard the text is not read, so the data set no longer differs from it.
        app.session.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
        assert!(matches(&rebuilt(&mut d, &mut app, "artboard lock"), "Data Set 1"));
        // New art there is read again; selecting or deselecting it keeps the revision.
        let revision = app.session.active().unwrap().revision;
        app.session.active().unwrap().doc.new_art.set(Arc::new(HashSet::from([NodeId(t)])));
        assert!(!matches(&rebuilt(&mut d, &mut app, "new art selected"), "Data Set 1"));
        app.session.active().unwrap().doc.new_art.set(Arc::default());
        assert!(matches(&rebuilt(&mut d, &mut app, "new art deselected"), "Data Set 1"));
        assert_eq!(app.session.active().unwrap().revision, revision, "new art moved the revision");
        app.session.execute("artboard.setProps", &json!({"index": 0, "locked": false})).unwrap();
        assert!(!matches(&rebuilt(&mut d, &mut app, "artboard unlock"), "Data Set 1"));

        app.session.execute("variable.sort", &json!({"by": "name", "descending": true})).unwrap();
        let names: Vec<String> = rebuilt(&mut d, &mut app, "variable.sort").vars.iter().map(|v| v.name.clone()).collect();
        assert_eq!(names, ["Shown", "Name"]);

        // Identify Objects By names the one object by its XML id; the preference is not an edit.
        app.session.execute("layer.setProps", &json!({"id": r, "name": "My Rect #2"})).unwrap();
        rebuilt(&mut d, &mut app, "layer.setProps");
        app.session.prefs.identify_objects_by = "xmlId".into();
        let rows = rebuilt(&mut d, &mut app, "Identify Objects By");
        assert_eq!(rows.vars.iter().find(|v| v.name == "Shown").map(|v| v.object.as_str()), Some("My_Rect__x23_2"));
        app.session.prefs.identify_objects_by = "objectName".into();
        rebuilt(&mut d, &mut app, "Identify Objects By back");

        // A variable bound to more than one object says so in the interface language.
        app.session.execute("variable.bind", &json!({"variable": "Shown", "ids": [t]})).unwrap();
        rebuilt(&mut d, &mut app, "a second binding");
        crate::i18n::set_current(crate::i18n::Lang::from_code("de").expect("de registered"));
        let rows = rebuilt(&mut d, &mut app, "the language");
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert_eq!(rows.vars.iter().find(|v| v.name == "Shown").map(|v| v.object.as_str()), Some("Mehrere Objekte"));
        let rows = rebuilt(&mut d, &mut app, "the language back");
        assert_eq!(rows.vars.iter().find(|v| v.name == "Shown").map(|v| v.object.as_str()), Some("Multiple objects"));

        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(rebuilt(&mut d, &mut app, "edit.undo").vars.iter().find(|v| v.name == "Shown").map(|v| v.bound.len()), Some(1));
        app.session.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(rebuilt(&mut d, &mut app, "edit.redo").vars.iter().find(|v| v.name == "Shown").map(|v| v.bound.len()), Some(2));

        // Another document, and back.
        app.session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        assert!(rebuilt(&mut d, &mut app, "another document").vars.is_empty());
        assert!(app.session.set_active(0));
        assert_eq!(rebuilt(&mut d, &mut app, "the first document again").vars.len(), 2);
    }

    /// A menu item that needs a name opens the dialog rather than running the command with
    /// empty params, which would only say "missing `name`".
    #[test]
    fn the_menu_dialogs_open_instead_of_failing() {
        let mut app = app_with_data();
        let ctx = crate::testing::ui_context();
        app.ui_ctx = Some(ctx.clone());
        for (cmd, field) in [
            ("variable.define", "kind"),
            ("variable.rename", "newName"),
            ("variable.bind", "variable"),
            ("dataset.new", "name"),
            ("dataset.rename", "newName"),
            ("dataset.select", "name"),
        ] {
            crate::menus::invoke(&mut app, cmd, json!({}));
            let open = crate::tests::picks::open_dialogs(&app).pop().unwrap_or_else(|| panic!("{cmd} opened no dialog"));
            assert_eq!(open.key.as_deref(), Some(crate::dialogs::variables::KEY), "{cmd}");
            assert!(open.fields.as_ref().is_some_and(|f| f.get(field).is_some()), "{cmd}: no `{field}` field");
            app.dialogs().expect("the window's dialog host").with(|host| host.cancel_by_id(&ctx, open.id));
        }
    }
}
