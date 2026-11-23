//! Object › Graph › Data…: the Graph Data window, a worksheet of the graph's data.
//!
//! The first row holds the series labels (the legend), the first column the category labels, the
//! top-left cell stays blank; the other cells are values (a blank cell has none). A cell is picked
//! with a click and edited in the entry box above the sheet: Enter writes the text into the cell and
//! moves down, Tab off the box moves right, Shift with either the other way, and the arrow keys move
//! between the sheet's cells. Pasting tab- or line-separated text into the sheet fills the cells from
//! the picked one. Import Data reads a tab-separated text file, Transpose row/column swaps rows and
//! columns, Switch x/y swaps a scatter graph's y and x columns, Cell Style sets the number of
//! decimals the sheet shows and the width of every column, dragging a column's right edge sets that
//! column's width (in whole digits, 0 to 20, as Cell Style), and Revert reads the graph's data
//! again. Apply (`graph.setData` with its series, categories and rows) redraws the graph and leaves
//! the window open.
//!
//! Fields (`ui.inspect`, `ui.dialog.set`): `grid` (rows of cell texts), `sel` ([row, column]),
//! `entry` (the picked cell's text being edited), `decimals` (null: as typed), `width` (digits) and
//! `widths` (a column's own width in digits by its index, null or missing: `width`; setting `width`
//! empties it).

use std::collections::BTreeMap;

use craft::behavior::{EditEvent, NumberCodec};
use craft::host::{PickRequest, Picked};
use craft::table::ui::{
    CellAt, CellDirection, CellEdit, CellGrid, CellSelection, CellSpace, ColumnEdges, GridEdit, GridModel, GridState, LineIndex, Positions,
};
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, PaintUi, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{
    DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id, serde_fields, set_serde_field,
};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::primitives::TextRun;
use craft::ui::runtime::env::{Env, Label, Units};
use craft::ui::runtime::focus::Initial;
use craft::ui::style::TypeRole;
use craft::ui::text::{NumberField, TextField};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES};

use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};

/// The dialog kind of Graph Data, the key of its dialog on the host.
pub const KIND: &str = "graphData";

/// The sheet's size: a row of labels over a graph's categories, a column of labels before its
/// series; the least rows it shows.
const MAX_ROWS: usize = MAX_GRAPH_CATEGORIES + 1;
const MAX_COLS: usize = MAX_GRAPH_SERIES + 1;
const MIN_ROWS: usize = 8;
/// The largest file Import Data reads, 1 MB (1,048,576 bytes), the same as Import Data Sets'.
const MAX_IMPORT_BYTES: usize = vectorcraft_engine::cmd::MAX_GRAPH_DATA_FILE;
/// The Cell Style limits (Adjust column width and decimal digits), and what they start at.
const MAX_DECIMALS: u64 = 10;
const MAX_WIDTH: u64 = 20;
const DEFAULT_DECIMALS: u64 = 2;
const DEFAULT_WIDTH: u64 = 7;
/// A cell's height on the sheet.
const ROW_H: f32 = 19.0;

/// Open Graph Data for the selected graph.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    // The query fails without a graph selected, with the engine's own message.
    let v = app.session.execute("graph.setData", &json!({})).map_err(|e| e.to_string())?;
    let graph = app.session.active().and_then(|st| st.selection.objects.iter().find_map(|id| graph_of(&st.doc, *id)).map(|id| (st, id)));
    let scatter = graph.is_some_and(|(st, id)| {
        st.doc.node(id).and_then(|n| n.extras().graph.as_deref()).is_some_and(|g| g.kind == vectorcraft_doc::GraphKind::Scatter)
    });
    let id = graph.map(|(_, id)| id.0);
    let dialog = GraphDataDialog::new(grid_from_data(&v), id, scatter, can_import(app));
    let dialogs = app.dialogs().ok_or("no window to show the dialog in")?;
    dialogs.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// The graph `id` belongs to (itself, or the graph its parts are in).
fn graph_of(doc: &vectorcraft_doc::Document, id: vectorcraft_doc::NodeId) -> Option<vectorcraft_doc::NodeId> {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if doc.node(c).is_some_and(|n| n.extras().graph.is_some()) {
            return Some(c);
        }
        cur = doc.parent_of(c);
    }
    None
}

// ---------- the worksheet ----------

type Grid = Vec<Vec<String>>;

/// The values the window edits: the cells and Cell Style. What `FormDialog` binds and what Apply sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Sheet {
    /// The cells by row; a row or cell past them is blank.
    grid: Grid,
    /// The decimals a number cell shows; `None` shows the text as typed.
    decimals: Option<u64>,
    /// Cell Style's Column Width in digits.
    width: u64,
    /// A column's own width in digits by its index; `None` takes `width`.
    widths: Vec<Option<u64>>,
}

impl Default for Sheet {
    fn default() -> Self {
        Sheet { grid: Grid::new(), decimals: None, width: DEFAULT_WIDTH, widths: Vec::new() }
    }
}

/// A button's work that needs the app, done by [`pump`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// Import Data…: ask for a file.
    Import,
    /// Revert: read the graph's data again.
    Revert,
}

/// The entry box: the text of the cell `at`, which it writes into the sheet when the cell is left.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    text: String,
    at: (usize, usize),
}

/// The row and column lines the grid lays out, rebuilt only when the rows shown or the column widths change.
struct Lines {
    key: Option<(usize, u64, Vec<Option<u64>>)>,
    rows: LineIndex,
    columns: LineIndex,
    /// The own width of the column whose edge a drag holds, as it was when the drag began.
    resize_from: Option<Option<u64>>,
}

impl Lines {
    fn new() -> Self {
        Lines {
            key: None,
            rows: LineIndex::uniform(ROW_H, 0),
            columns: LineIndex::uniform(digits_px(DEFAULT_WIDTH), MAX_COLS as u32),
            resize_from: None,
        }
    }

    /// Lays out `rows` rows and every column of `sheet` at its width.
    fn sync(&mut self, rows: usize, sheet: &Sheet) {
        if self.key.as_ref().is_some_and(|(r, w, ws)| *r == rows && *w == sheet.width && *ws == sheet.widths) {
            return;
        }
        let own: BTreeMap<u32, f32> = sheet.widths.iter().enumerate().filter_map(|(c, w)| Some((u32::try_from(c).ok()?, digits_px((*w)?)))).collect();
        self.rows = LineIndex::uniform(ROW_H, u32::try_from(rows).unwrap_or(u32::MAX));
        self.columns = LineIndex::new(&own, digits_px(width(sheet)), MAX_COLS as u32);
        self.key = Some((rows, sheet.width, sheet.widths.clone()));
    }
}

/// The sheet as the grid reads it.
struct SheetModel<'a> {
    sheet: &'a Sheet,
    rows: &'a LineIndex,
    columns: &'a LineIndex,
}

impl GridModel for SheetModel<'_> {
    fn rows(&self) -> &LineIndex {
        self.rows
    }

    fn columns(&self) -> &LineIndex {
        self.columns
    }

    fn selectable(&self, cell: CellAt<u32>) -> bool {
        selectable(cell)
    }

    fn last_used(&self) -> CellAt<u32> {
        let column = self.sheet.grid.iter().filter_map(|r| r.iter().rposition(|c| !c.trim().is_empty())).max().unwrap_or(0);
        CellAt::new(data_rows(self.sheet) as u32, column as u32)
    }

    fn paint_cell(&self, ui: &mut PaintUi<'_>, rect: egui::Rect, cell: CellAt<u32>, _zoom: f32) {
        let text = shown(&self.cell_text(cell), self.sheet.decimals);
        ui.draw().text(rect, &TextRun::new(text).role(TypeRole::Mono).elide());
    }

    fn cell_text(&self, cell: CellAt<u32>) -> String {
        cell_of(&self.sheet.grid, cell.row as usize, cell.column as usize)
    }
}

/// The top-left cell stays blank: no pick lands on it.
fn selectable(cell: CellAt<u32>) -> bool {
    (cell.row, cell.column) != (0, 0)
}

/// Runs `f` with the rules the picked cell moves by: the sheet's full size, whatever it shows.
fn with_space<R>(f: impl FnOnce(&CellSpace<'_, u32>) -> R) -> R {
    let rows = Positions::new(MAX_ROWS as u32);
    f(&CellSpace::new(&rows, MAX_COLS as u32).selectable(&selectable))
}

/// The cells' text of a grid, or an empty text past it.
fn cell_of(grid: &[Vec<String>], r: usize, c: usize) -> String {
    grid.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default()
}

/// The picked cell of the sheet view ([row, column]).
fn active(state: &GridState) -> (usize, usize) {
    state.selection.active().map_or((0, 1), |c| (c.row as usize, c.column as usize))
}

/// Picks cell `to`; `false` when the sheet has no such cell or it is the blank one.
fn select(state: &mut GridState, to: (usize, usize)) -> bool {
    let (Ok(row), Ok(column)) = (u32::try_from(to.0), u32::try_from(to.1)) else { return false };
    with_space(|space| state.selection.select(CellAt::new(row, column), space))
}

/// The sheet's cells from the `graph.setData` query: the series in the first row, each category's
/// label then values below (the label column is there, blank, when the graph has no labels).
pub(crate) fn grid_from_data(v: &Value) -> Grid {
    let strings = |k: &str| -> Vec<String> {
        v[k].as_array().map(|a| a.iter().map(|s| s.as_str().unwrap_or_default().to_string()).collect()).unwrap_or_default()
    };
    let (series, categories) = (strings("series"), strings("categories"));
    let rows: Vec<Vec<Value>> =
        v["rows"].as_array().map(|a| a.iter().map(|r| r.as_array().cloned().unwrap_or_default()).collect()).unwrap_or_default();
    let mut grid: Grid = vec![std::iter::once(String::new()).chain(series).collect()];
    for (c, row) in rows.iter().enumerate() {
        let label = categories.get(c).cloned().unwrap_or_default();
        grid.push(std::iter::once(label).chain(row.iter().map(|x| x.as_f64().map(|f| f.to_string()).unwrap_or_default())).collect());
    }
    grid
}

/// The sheet as `graph.setData` takes it ([`vectorcraft_engine::cmd::graph_data_from_grid`]),
/// which Import Data Sets' `%` columns also use.
pub(crate) fn data_from_grid(grid: &Grid) -> Value {
    vectorcraft_engine::cmd::graph_data_from_grid(grid)
}

/// Text from a file or the clipboard as cells ([`vectorcraft_engine::cmd::graph_grid_from_text`]).
pub(crate) fn grid_from_text(text: &str) -> Grid {
    vectorcraft_engine::cmd::graph_grid_from_text(text)
}

/// A text file's bytes as text ([`vectorcraft_engine::cmd::graph_data_text`]).
pub(crate) fn decode(bytes: &[u8]) -> String {
    vectorcraft_engine::cmd::graph_data_text(bytes)
}

/// A cell's text ([`vectorcraft_engine::cmd::graph_cell_text`]).
fn clean(s: &str) -> String {
    vectorcraft_engine::cmd::graph_cell_text(s)
}

/// Set cell `(r, c)` of the sheet, growing it as needed (within the sheet's size).
fn set_cell_at(sheet: &mut Sheet, r: usize, c: usize, text: &str) {
    if r >= MAX_ROWS || c >= MAX_COLS {
        return;
    }
    if sheet.grid.len() <= r {
        sheet.grid.resize(r + 1, Vec::new());
    }
    let Some(row) = sheet.grid.get_mut(r) else { return };
    if row.len() <= c {
        row.resize(c + 1, String::new());
    }
    if let Some(cell) = row.get_mut(c) {
        *cell = clean(text);
    }
}

/// Transpose row/column: the sheet's rows become its columns.
pub(crate) fn transpose(grid: &Grid) -> Grid {
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    (0..cols).map(|c| grid.iter().map(|r| r.get(c).cloned().unwrap_or_default()).collect()).collect()
}

/// Switch x/y: each scatter series' y and x columns (1 and 2, 3 and 4…) trade places, labels too; a
/// last y column without its x stays where it is.
pub(crate) fn switch_xy(grid: &Grid) -> Grid {
    // Every row as wide as the widest, so a label row shorter than the data swaps with it.
    let width = grid.iter().map(Vec::len).max().unwrap_or(0);
    grid.iter()
        .map(|r| {
            let mut r = r.clone();
            r.resize(width, String::new());
            let mut c = 1;
            while c + 1 < r.len() {
                r.swap(c, c + 1);
                c += 2;
            }
            r
        })
        .collect()
}

/// How many rows of the sheet hold data (below the labels' row).
fn data_rows(sheet: &Sheet) -> usize {
    sheet.grid.iter().rposition(|r| r.iter().any(|c| !c.trim().is_empty())).unwrap_or(0)
}

/// How many rows the sheet draws: the data, a spare row, the picked cell's, and at least MIN_ROWS.
fn extent(sheet: &Sheet, sel: (usize, usize)) -> usize {
    (sheet.grid.len() + 1).max(MIN_ROWS).max(sel.0 + 2).min(MAX_ROWS)
}

/// A cell as the sheet shows it: a number with `decimals` places when Cell Style sets them, else as
/// typed (the number Apply sends is as typed either way).
pub(crate) fn shown(text: &str, decimals: Option<u64>) -> String {
    match (decimals, text.trim().parse::<f64>()) {
        (Some(d), Ok(v)) if v.is_finite() && !text.trim().starts_with('"') => format!("{v:.*}", d.min(MAX_DECIMALS) as usize),
        _ => text.to_string(),
    }
}

/// The sheet's cells replaced by `g`.
fn replace_grid(sheet: &mut Sheet, g: Grid) {
    sheet.grid = g;
}

/// Paste: text with tabs or line breaks fills the cells from the cell `at`.
fn paste_block(sheet: &mut Sheet, at: (usize, usize), text: &str) {
    for (r, row) in grid_from_text(text).into_iter().enumerate() {
        for (c, t) in row.into_iter().enumerate() {
            set_cell_at(sheet, at.0 + r, at.1 + c, &t);
        }
    }
}

/// Keeps what a field set on the sheet may hold within the sheet's size.
fn limit(sheet: &mut Sheet) {
    sheet.grid.truncate(MAX_ROWS);
    for row in &mut sheet.grid {
        row.truncate(MAX_COLS);
    }
    sheet.widths.truncate(MAX_COLS);
    sheet.width = sheet.width.min(MAX_WIDTH);
    sheet.decimals = sheet.decimals.map(|d| d.min(MAX_DECIMALS));
}

/// The entry box's text into its cell.
fn commit_text(sheet: &mut Sheet, entry: &Entry) {
    if cell_of(&sheet.grid, entry.at.0, entry.at.1) != entry.text {
        set_cell_at(sheet, entry.at.0, entry.at.1, &entry.text);
    }
}

/// The entry box shows the picked cell's text.
fn load(sheet: &Sheet, state: &GridState, entry: &mut Entry) {
    entry.at = active(state);
    entry.text = cell_of(&sheet.grid, entry.at.0, entry.at.1);
}

/// The picked cell moved since the entry box loaded: the box's text goes into the cell it left and
/// the box loads the new one.
fn follow(sheet: &mut Sheet, state: &GridState, entry: &mut Entry) {
    if active(state) != entry.at {
        commit_text(sheet, entry);
        load(sheet, state, entry);
    }
}

/// Enter, Tab or Shift with either in the entry box: its text into the picked cell, then the pick moves
/// one cell toward `step` (it stays at the sheet's edge and off the blank cell) and the box loads that cell.
fn commit_entry(sheet: &mut Sheet, state: &mut GridState, entry: &mut Entry, step: CellDirection) {
    commit_text(sheet, entry);
    with_space(|space| state.selection.step(step, space));
    load(sheet, state, entry);
}

/// Apply: the sheet's data to the graph.
fn apply(app: &mut VectorcraftApp, sheet: &Sheet, graph: Option<u64>) -> Result<Value, String> {
    // The graph the sheet opened on (else the selected one).
    let mut p = data_from_grid(&sheet.grid);
    p["id"] = json!(graph);
    app.run("graph.setData", p)
}

/// The graph's data as sheet cells.
fn graph_grid(app: &mut VectorcraftApp, graph: Option<u64>) -> Result<Grid, String> {
    let v = app.session.execute("graph.setData", &json!({ "id": graph })).map_err(|e| e.to_string())?;
    Ok(grid_from_data(&v))
}

/// Whether Import Data can ask for a file here: the host has a picker and the app can read files.
fn can_import(app: &VectorcraftApp) -> bool {
    file_asks::can(app, craft::host::PickKind::Open) && app.services.read.is_some()
}

/// Import Data's answer: a tab-separated text file's rows into the sheet.
fn import(app: &mut VectorcraftApp, picked: Picked) -> Result<Value, String> {
    let bytes = match picked {
        Picked::Paths(paths) => {
            let Some(path) = paths.first().map(|p| p.to_string_lossy().into_owned()) else { return Ok(Value::Null) };
            // One byte past the limit tells a larger file apart without reading all of it.
            match (&app.services.read_head, &app.services.read) {
                (Some(read_head), _) => read_head(&path, MAX_IMPORT_BYTES + 1)?,
                (None, Some(read)) => read(&path)?,
                // The button is off where files can't be read.
                (None, None) => return Ok(Value::Null),
            }
        }
        // The web build answers with the file's bytes.
        Picked::Contents(files) => match files.into_iter().next() {
            Some((_, bytes)) => bytes,
            None => return Ok(Value::Null),
        },
        // Cancelled, or no picker could be shown: nothing to say.
        _ => return Ok(Value::Null),
    };
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err("the file is larger than 1 MB".into());
    }
    let grid = grid_from_text(&decode(&bytes));
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| host.get_mut::<GraphDataDialog>().map(|d| d.replace(grid)));
    }
    Ok(Value::Null)
}

/// What the window asked of the app since the last [`pump`].
struct Asks {
    graph: Option<u64>,
    apply: Option<Sheet>,
    actions: Vec<Action>,
}

/// Runs once per frame after the shell: answers what the open Graph Data window asked (Apply, Import,
/// Revert) and tells it whether Import can ask for a file.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let can = can_import(app);
    let asked = dialogs.with(|host| {
        host.get_mut::<GraphDataDialog>().map(|d| {
            d.can_import = can;
            d.take_asks()
        })
    });
    let Some(Asks { graph, apply: filed, actions }) = asked else { return };
    if let Some(sheet) = filed {
        if let Err(e) = apply(app, &sheet, graph) {
            app.status(e);
        }
        dialogs.with(|host| host.get_mut::<GraphDataDialog>().map(GraphDataDialog::applied));
    }
    for action in actions {
        match action {
            Action::Import => {
                let request = PickRequest::open("document").filter("Text", &["txt", "tsv", "csv"]);
                file_asks::ask(app, request, Owner::Document, import);
            }
            Action::Revert => match graph_grid(app, graph) {
                Ok(grid) => {
                    dialogs.with(|host| host.get_mut::<GraphDataDialog>().map(|d| d.replace(grid)));
                }
                Err(e) => app.status(e),
            },
        }
    }
}

// ---------- Cell Style ----------

/// Cell Style's Column Width in digits.
fn width(sheet: &Sheet) -> u64 {
    sheet.width.min(MAX_WIDTH)
}

/// A column's width in digits: its own when its edge was dragged, else Cell Style's.
fn col_digits(sheet: &Sheet, c: usize) -> u64 {
    sheet.widths.get(c).copied().flatten().unwrap_or_else(|| width(sheet)).min(MAX_WIDTH)
}

/// Cell Style's Column Width: every column `n` digits wide, the ones given their own width too.
fn set_width(sheet: &mut Sheet, n: u64) {
    sheet.width = n.min(MAX_WIDTH);
    sheet.widths.clear();
}

/// Set column `c`'s own width in digits (at most MAX_WIDTH).
fn set_col_digits(sheet: &mut Sheet, c: usize, n: u64) {
    if c >= MAX_COLS {
        return;
    }
    if sheet.widths.len() <= c {
        sheet.widths.resize(c + 1, None);
    }
    if let Some(w) = sheet.widths.get_mut(c) {
        *w = Some(n.min(MAX_WIDTH));
    }
}

/// A column `n` digits wide, in points on the sheet.
fn digits_px(n: u64) -> f32 {
    (n.min(MAX_WIDTH) as f32 * 7.0 + 10.0).max(12.0)
}

/// The width in digits nearest to `px` points, within Cell Style's 0 to MAX_WIDTH.
fn px_digits(px: f32) -> u64 {
    if px.is_finite() { ((px - 10.0) / 7.0).round().clamp(0.0, MAX_WIDTH as f32) as u64 } else { 0 }
}

/// A column's edge dragged: the width the drag gives it in whole digits, on every update and at the commit; a
/// cancel gives back the width it began with (`began`, kept at the begin). A drag that stays where it began changes
/// nothing.
fn handle_edit(sheet: &mut Sheet, began: &mut Option<Option<u64>>, column: u32, width: &EditEvent<f32>) {
    let c = column as usize;
    match width {
        EditEvent::Begin { .. } => *began = Some(sheet.widths.get(c).copied().flatten()),
        // The grid's cancel carries no starting width: the one kept at the begin goes back.
        EditEvent::Cancel { .. } => {
            if let (Some(own), Some(w)) = (began.take(), sheet.widths.get_mut(c)) {
                *w = own;
            }
            return;
        }
        _ => {}
    }
    let Some(px) = width.landed() else { return };
    let n = px_digits(*px);
    if n != col_digits(sheet, c) {
        set_col_digits(sheet, c, n);
    }
}

// ---------- the window ----------

/// What the body edits besides the sheet.
struct Pane<'a> {
    state: &'a mut GridState,
    entry: &'a mut Entry,
    lines: &'a mut Lines,
    actions: &'a mut Vec<Action>,
    scatter: bool,
    can_import: bool,
}

/// A whole number field of `units` named `label`, from 0 to `max`.
fn whole(units: &Units, label: impl Into<Label>, value: u64, max: u64) -> NumberField<'static> {
    NumberField::new(label, Bind::given(Some(value as f64))).codec(NumberCodec::new(units.reader(None), 0)).step(1.0).range(0.0..=max as f64)
}

/// The whole number a field reported, within 0 to `max`.
fn digits(typed: Option<f64>, max: u64) -> Option<u64> {
    typed.filter(|n| n.is_finite()).map(|n| n.round().clamp(0.0, max as f64) as u64)
}

/// Draws the entry box with the sheet's buttons, Cell Style and the sheet itself.
fn body(ui: &mut egui::Ui, values: &mut Sheet, pane: &mut Pane<'_>) -> bool {
    let Pane { state, entry, lines, actions, scatter, can_import } = pane;
    let units = Env::of(ui).units();
    Row::new().show(ui, |ui, _| {
        let edit = TextField::new(tl!("Cell Value"), Bind::bound(&mut entry.text)).submits().show(ui);
        if edit.submitted {
            let step = if edit.submit_mods.shift { CellDirection::Up } else { CellDirection::Down };
            commit_entry(values, state, entry, step);
        } else if let Some(forward) = edit.left_by_tab {
            commit_entry(values, state, entry, if forward { CellDirection::Right } else { CellDirection::Left });
        }
        if Button::new(tl!("Import Data…")).enabled(*can_import).show(ui).clicked() {
            actions.push(Action::Import);
        }
        // Transposed, the categories become series: at most as many as a graph has.
        if Button::new(tl!("Transpose")).enabled(data_rows(values) <= MAX_GRAPH_SERIES).show(ui).clicked() {
            commit_text(values, entry);
            replace_grid(values, transpose(&values.grid));
            load(values, state, entry);
        }
        if Button::new(tl!("Switch x/y")).enabled(*scatter).show(ui).clicked() {
            commit_text(values, entry);
            replace_grid(values, switch_xy(&values.grid));
            load(values, state, entry);
        }
        if Button::new(tl!("Revert")).show(ui).clicked() {
            actions.push(Action::Revert);
        }
    });
    // Cell Style: decimals shown and the columns' width in digits.
    Row::new().show(ui, |ui, _| {
        let on = Checkbox::new(Bind::given(values.decimals.is_some())).label(tl!("Number of Decimals:")).show(ui).value().copied();
        if let Some(on) = on {
            values.decimals = on.then_some(DEFAULT_DECIMALS);
        }
        let field = whole(&units, tl!("Number of Decimals"), values.decimals.unwrap_or(DEFAULT_DECIMALS), MAX_DECIMALS)
            .enabled(values.decimals.is_some())
            .show(ui);
        if let Some(n) = digits(field.edit.value().copied().flatten(), MAX_DECIMALS) {
            values.decimals = Some(n);
        }
        Text::new(tl!("Column Width:")).show(ui);
        let field = whole(&units, tl!("Column Width"), width(values), MAX_WIDTH).show(ui);
        // A value entered with Enter sets every column, also when it is the width shown. Leaving the
        // field without a new value keeps the columns' own widths.
        let typed = digits(field.edit.value().copied().flatten(), MAX_WIDTH);
        if typed.is_some() || field.edit.submitted {
            set_width(values, typed.unwrap_or_else(|| width(values)));
        }
    });
    lines.sync(extent(values, active(state)), values);
    let model = SheetModel { sheet: values, rows: &lines.rows, columns: &lines.columns };
    let out = CellGrid::new(tl!("Graph Data"), &model, state).headings(false).column_edges(ColumnEdges::Cells).default_stop().show_sheet(ui);
    if let Some(CellEdit { cell, text, .. }) = out.committed {
        let (r, c) = (cell.row as usize, cell.column as usize);
        set_cell_at(values, r, c, &text);
        if entry.at == (r, c) {
            entry.text = cell_of(&values.grid, r, c);
        }
    }
    for edit in out.edits {
        match edit {
            GridEdit::Paste(text) => {
                paste_block(values, active(state), &text);
                load(values, state, entry);
            }
            GridEdit::ResizeColumn { column, width, .. } => handle_edit(values, &mut lines.resize_from, column, &width),
            _ => {}
        }
    }
    // The box mirrors the cell being edited in place, else follows the picked cell.
    match out.draft {
        Some(draft) => entry.text = draft,
        None => follow(values, state, entry),
    }
    true
}

/// The Graph Data window: a [`StackDialog`] on the window's host, over the sheet it edits. Apply files the
/// sheet for [`pump`], which sends it to the graph and keeps the window open.
pub(crate) struct GraphDataDialog {
    id: DialogId,
    sheet: Sheet,
    state: GridState,
    entry: Entry,
    /// The graph the sheet opened on.
    graph: Option<u64>,
    scatter: bool,
    can_import: bool,
    lines: Lines,
    actions: Vec<Action>,
    /// Apply's sheet, until the pump has sent it.
    held: HeldAnswer<Sheet>,
}

impl GraphDataDialog {
    /// The window over `grid`, the data of `graph`; Switch x/y is on for a `scatter` graph.
    fn new(grid: Grid, graph: Option<u64>, scatter: bool, can_import: bool) -> Self {
        let sheet = Sheet { grid, ..Sheet::default() };
        let state = GridState::new(CellSelection::cell(CellAt::new(0, 1)), 1.0);
        let mut entry = Entry { text: String::new(), at: (0, 1) };
        load(&sheet, &state, &mut entry);
        GraphDataDialog {
            id: dialog_id(KIND),
            sheet,
            state,
            entry,
            graph,
            scatter,
            can_import,
            lines: Lines::new(),
            actions: Vec::new(),
            held: HeldAnswer::new(),
        }
    }

    /// Everything the window asked since the last call, once.
    fn take_asks(&mut self) -> Asks {
        Asks { graph: self.graph, apply: self.held.take(), actions: std::mem::take(&mut self.actions) }
    }

    /// The pump sent Apply's sheet to the graph; the window stays open and Apply is ready again.
    fn applied(&mut self) {
        self.held.answer(Ok(()));
        self.held.finish();
    }

    /// The cells replaced by `grid` (Import, Revert); the box loads the picked cell.
    fn replace(&mut self, grid: Grid) {
        replace_grid(&mut self.sheet, grid);
        load(&self.sheet, &self.state, &mut self.entry);
    }

    /// Files Apply: the box's text into its cell, then the sheet for the pump.
    fn file_apply(&mut self) {
        commit_text(&mut self.sheet, &self.entry);
        self.held.file(self.sheet.clone());
    }
}

impl StackDialog for GraphDataDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let GraphDataDialog { sheet, state, entry, scatter, can_import, lines, actions, held, .. } = self;
        let mut pane = Pane { state, entry, lines, actions, scatter: *scatter, can_import: *can_import };
        let shown = FormDialog::new(tl!("Graph Data"), Bind::bound(&mut *sheet))
            .ok_label(tl!("Apply"))
            .ok_enabled(!held.is_waiting())
            .accept_stays_open(true)
            .enter_confirms(false)
            // The sheet owns the scroll; the controls above it stay in view.
            .collection_body(true)
            .shaped(|dialog| dialog.initial_focus(Initial::Default))
            .show(ui, |ui, _, values| body(ui, values, &mut pane));
        // The pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
        if shown.picked == Some(0) && !held.is_waiting() {
            commit_text(sheet, pane.entry);
            held.file(sheet.clone());
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }

    /// OK as the button: Apply files the sheet for the pump and answers none, so the window stays open.
    fn confirm(&mut self, _: &egui::Context) -> Option<()> {
        if !self.held.is_waiting() {
            self.file_apply();
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = serde_fields(&self.sheet)?;
        if let Value::Object(map) = &mut fields {
            let (r, c) = active(&self.state);
            map.insert("sel".to_owned(), json!([r, c]));
            map.insert("entry".to_owned(), json!(self.entry.text));
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejected = |value: &Value| FieldRefusal::Rejects(value).message(field);
        match field {
            "sel" => {
                let at = value.as_array().and_then(|a| match a.as_slice() {
                    [r, c] => Some((usize::try_from(r.as_u64()?).ok()?, usize::try_from(c.as_u64()?).ok()?)),
                    _ => None,
                });
                let at = at.ok_or_else(|| rejected(&value))?;
                commit_text(&mut self.sheet, &self.entry);
                if !select(&mut self.state, at) {
                    return Err(rejected(&value));
                }
                load(&self.sheet, &self.state, &mut self.entry);
            }
            "entry" => self.entry.text = clean(value.as_str().ok_or_else(|| rejected(&value))?),
            "width" => set_width(&mut self.sheet, value.as_u64().ok_or_else(|| rejected(&value))?),
            _ => {
                set_serde_field(&mut self.sheet, field, value)?;
                limit(&mut self.sheet);
                load(&self.sheet, &self.state, &mut self.entry);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::host::ScriptedFiles;
    use egui_kittest::kittest::{NodeT, Queryable};
    use serde_json::json;

    use super::*;

    type Harness = egui_kittest::Harness<'static, VectorcraftApp>;

    fn g(rows: &[&[&str]]) -> Grid {
        rows.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect()
    }

    fn app_with(csv: &str, ty: &str) -> (VectorcraftApp, u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let id =
            app.run("graph.create", json!({"type": ty, "x": 0, "y": 0, "width": 200, "height": 150, "csv": csv})).unwrap()["id"].as_u64().unwrap();
        (app, id)
    }

    fn data(app: &mut VectorcraftApp, id: u64) -> Value {
        app.session.execute("graph.setData", &json!({"id": id})).unwrap()
    }

    /// Reads the open window.
    fn read<R>(app: &VectorcraftApp, f: impl FnOnce(&GraphDataDialog) -> R) -> R {
        app.dialogs().expect("a window").with(|host| f(host.get::<GraphDataDialog>().expect("Graph Data is open")))
    }

    /// Edits the open window.
    fn edit<R>(app: &VectorcraftApp, f: impl FnOnce(&mut GraphDataDialog) -> R) -> R {
        app.dialogs().expect("a window").with(|host| f(host.get_mut::<GraphDataDialog>().expect("Graph Data is open")))
    }

    fn set_field(app: &VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let ctx = crate::testing::ui_context();
        app.dialogs().expect("a window").with(|host| host.set_json_field(&ctx, dialog_id(KIND), field, value))
    }

    /// OK through the host, then the pump: what Apply does.
    fn apply_through_host(app: &mut VectorcraftApp) {
        let ctx = crate::testing::ui_context();
        let closed = app.dialogs().expect("a window").with(|host| host.confirm::<GraphDataDialog>(&ctx));
        assert!(!closed, "Apply leaves the window open");
        pump(app, &ctx);
    }

    /// The app with the window drawn each frame as the shell draws it.
    // PLAN: the shell's own frame (hub-w2) replaces `host.run` here, and runs `file_asks::poll` (files-w7) itself.
    fn harness(app: VectorcraftApp) -> Harness {
        let mut fonts = false;
        egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::testing::install(ui.ctx());
                    return;
                }
                if let Some(dialogs) = app.dialogs() {
                    dialogs.with(|host| host.run(ui));
                }
                pump(app, ui.ctx());
                file_asks::poll(app);
            },
            app,
        )
    }

    #[test]
    fn opening_and_applying_unchanged_keeps_the_data() {
        for (csv, ty) in [
            ("\tSales, EU\tb|c\nQ1, early\t1\t\n\"2013\"\t3\t4", "column"),
            ("1,2\n3,4", "column"),
            ("y,x\n1,10\n2,20", "scatter"),
            (",,\nQ1,1,2\n,3,4", "column"),
        ] {
            let (mut app, id) = app_with(csv, ty);
            let before = data(&mut app, id);
            open(&mut app).unwrap();
            apply_through_host(&mut app);
            assert_eq!(data(&mut app, id), before, "{csv}");
        }
    }

    #[test]
    fn the_sheet_holds_labels_in_the_first_row_and_column() {
        let (mut app, id) = app_with("1,2\n3,4", "column");
        assert_eq!(grid_from_data(&data(&mut app, id)), g(&[&[""], &["", "1", "2"], &["", "3", "4"]]));
        let p = data_from_grid(&g(&[&["", "a", "", ""], &["Q1", "1", "", ""], &["\"2013\"", "x", "2"], &[], &[""]]));
        assert_eq!(p, json!({"series": ["a"], "categories": ["Q1", "2013"], "rows": [[1.0, null], [null, 2.0]]}));
        assert_eq!(grid_from_text("\tA, B\tC\r\nx\t1\t2\n"), g(&[&["", "A, B", "C"], &["x", "1", "2"]]));
        assert_eq!(decode(b"\xEF\xBB\xBFa\tb"), "a\tb");
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("a\tb".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode(&utf16), "a\tb");
        let wide = vec!["x"; 400].join("\t");
        assert_eq!(grid_from_text(&wide)[0].len(), MAX_COLS);
    }

    #[test]
    fn utf16_without_a_byte_order_mark_reads_by_its_line_breaks() {
        let text = "\tSales é\r\nQ1\t1\n";
        let le: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode(&le), text, "little-endian");
        assert_eq!(decode(&be), text, "big-endian");
        assert_eq!(grid_from_text(&decode(&le)), g(&[&["", "Sales é"], &["Q1", "1"]]));
        // One row has no line break: the side of each unit that holds the NUL byte gives the order.
        for (bytes, order) in [(b"\0Q\x001\0\t\x002".to_vec(), "big-endian"), (b"Q\x001\0\t\x002\0".to_vec(), "little-endian")] {
            assert_eq!(decode(&bytes), "Q1\t2", "one line, {order}");
        }
        // A file cut short by one byte, or with an unpaired surrogate, reads as it would after a
        // byte-order mark.
        assert_eq!(decode(&[le.as_slice(), &[0x0A]].concat()), text, "odd length");
        assert_eq!(decode(&[le.as_slice(), &[0x00, 0xD8]].concat()), format!("{text}\u{FFFD}"), "unpaired surrogate");
        // A line break in both byte orders leaves the order unknown, and UTF-8 with a stray NUL
        // byte isn't UTF-16: both read as UTF-8, as before.
        assert_eq!(decode(&[0x0A, 0, 0, 0x0A]), "\n\0\0\n");
        assert_eq!(decode(b"a,b\n1,2\0"), "a,b\n1,2\0");
    }

    #[test]
    fn a_sheet_sent_wider_and_taller_than_a_graph_is_cut_to_one() {
        // One long row and one long column: their product would be ten billion cells.
        let mut grid: Grid = vec![vec!["1".to_string(); 100_000]];
        grid.extend((0..100_000).map(|_| vec!["2".to_string()]));
        let p = data_from_grid(&grid);
        assert_eq!(p["rows"].as_array().unwrap().len(), MAX_ROWS - 1);
        assert_eq!(p["series"].as_array().unwrap().len(), MAX_COLS - 1);
    }

    /// Import Data reads a file no further than one byte past 1 MB where the host can read part of
    /// a file: a larger file is refused with the sheet left as it was, and a file at the limit is
    /// read. Where the host only reads whole files, a larger file is refused too, and so is a file
    /// the web build answers with that is over the limit.
    #[test]
    fn import_data_refuses_a_file_larger_than_its_limit() {
        let (mut app, _) = app_with(",a\nQ1,1", "column");
        open(&mut app).unwrap();
        let at_limit = [b"\tb\nQ2\t2\n".as_slice(), &vec![b' '; MAX_IMPORT_BYTES - 8]].concat();
        let files: HashMap<String, Vec<u8>> =
            [("/big.txt".to_string(), vec![b'1'; MAX_IMPORT_BYTES + 1]), ("/limit.txt".to_string(), at_limit.clone())].into();
        let files = Rc::new(files);
        app.services.read = Some(Box::new(|_: &str| Err("the whole file was read".to_string())));
        let asked = Rc::new(RefCell::new(vec![]));
        let (f, a) = (files.clone(), asked.clone());
        app.services.read_head = Some(Box::new(move |path: &str, max: usize| {
            a.borrow_mut().push(max);
            Ok(f.get(path).ok_or("no such file")?.iter().copied().take(max).collect())
        }));
        let scripted = Arc::new(ScriptedFiles::new());
        // The window's host asks for the file, as the hosts install the app's on its context.
        app.host.with_files(scripted.clone()).install(app.ui_ctx.as_ref().expect("a window"));
        app.host = Arc::new(app.host.with_files(scripted.clone()));
        let mut h = harness(app);
        h.run_steps(4);
        let import = |h: &mut Harness, picked: Picked| {
            scripted.queue(picked);
            h.get_by_label("Import Data…").click();
            h.run_steps(5);
        };
        let at = |h: &Harness, r: usize, c: usize| read(h.state(), |d| cell_of(&d.sheet.grid, r, c));
        import(&mut h, Picked::Paths(vec!["/big.txt".into()]));
        assert_eq!(at(&h, 0, 1), "a", "the sheet is left as it was");
        import(&mut h, Picked::Paths(vec!["/limit.txt".into()]));
        assert_eq!((at(&h, 0, 1), at(&h, 1, 0)), ("b".to_string(), "Q2".to_string()));
        assert_eq!(*asked.borrow(), [MAX_IMPORT_BYTES + 1; 2], "each read stops one byte past the limit");
        // A host that reads only whole files.
        let app = h.state_mut();
        app.services.read_head = None;
        let f = files.clone();
        app.services.read = Some(Box::new(move |path: &str| Ok(f.get(path).ok_or("no such file")?.clone())));
        import(&mut h, Picked::Paths(vec!["/big.txt".into()]));
        assert_eq!(at(&h, 0, 1), "b");
        // The web build answers with the bytes.
        import(&mut h, Picked::Contents(vec![("big.txt".into(), vec![b'1'; MAX_IMPORT_BYTES + 1])]));
        assert_eq!(at(&h, 0, 1), "b");
        import(&mut h, Picked::Contents(vec![("small.txt".into(), b"\tc\n".to_vec())]));
        assert_eq!(at(&h, 0, 1), "c");
    }

    #[test]
    fn transpose_and_switch_xy() {
        assert_eq!(transpose(&g(&[&["", "a", "b"], &["Q1", "1", "2"]])), g(&[&["", "Q1"], &["a", "1"], &["b", "2"]]));
        assert_eq!(shown("3.14159", Some(2)), "3.14");
        assert_eq!(shown("3.14159", None), "3.14159");
        // Switch x/y in the sheet does what the engine's does.
        let (mut app, id) = app_with("y1,x1,y2,x2\n1,10,2,20\n3,30,4,40", "scatter");
        open(&mut app).unwrap();
        let mut h = harness(app);
        h.run_steps(4);
        h.get_by_label("Switch x/y").click();
        h.run_steps(3);
        apply_through_host(h.state_mut());
        let sheet = data(h.state_mut(), id);
        let (mut other, oid) = app_with("y1,x1,y2,x2\n1,10,2,20\n3,30,4,40", "scatter");
        other.run("graph.setData", json!({"id": oid, "switchXY": true})).unwrap();
        assert_eq!(sheet, data(&mut other, oid));
    }

    #[test]
    fn data_opens_the_sheet_and_apply_redraws_the_graph() {
        let (mut app, id) = app_with(",a,b\nQ1,1,2", "column");
        crate::menus::invoke(&mut app, "graph.setData", json!({}));
        assert!(app.dialogs().is_some_and(|d| d.with(|host| host.is_open::<GraphDataDialog>())));
        // Cells picked and typed into through the dialog's fields: the entry box's text goes into the cell it leaves.
        for (at, text) in [([1, 2], "5"), ([2, 0], "Q2"), ([2, 1], "7")] {
            set_field(&app, "sel", json!(at)).unwrap();
            set_field(&app, "entry", json!(text)).unwrap();
        }
        apply_through_host(&mut app);
        assert_eq!(data(&mut app, id)["rows"], json!([[1.0, 5.0], [7.0, null]]));
        assert!(app.dialogs().is_some_and(|d| d.with(|host| host.is_open::<GraphDataDialog>())), "Apply leaves the window open");
        assert_eq!(read(&app, |d| d.held.state()), None, "Apply is ready again");
        // A pasted block fills the cells from the picked one.
        set_field(&app, "sel", json!([1, 1])).unwrap();
        edit(&app, |d| {
            paste_block(&mut d.sheet, active(&d.state), "8\t9\n10\t11\n");
            load(&d.sheet, &d.state, &mut d.entry);
            assert_eq!((cell_of(&d.sheet.grid, 1, 1), cell_of(&d.sheet.grid, 2, 2), d.entry.text.clone()), ("8".into(), "11".into(), "8".into()));
            replace_grid(&mut d.sheet, vec![]);
        });
        // Revert reads the graph again.
        let mut h = harness(app);
        h.run_steps(4);
        h.get_by_label("Revert").click();
        h.run_steps(3);
        assert_eq!(read(h.state(), |d| cell_of(&d.sheet.grid, 0, 1)), "a");
        // The tool's Graph Data after drawing opens the same sheet.
        let app = h.state_mut();
        let ctx = crate::testing::ui_context();
        app.dialogs().expect("a window").with(|host| host.cancel::<GraphDataDialog>(&ctx));
        super::super::tools::open_tool_dialog(app, "graphData", json!({}));
        assert!(app.dialogs().is_some_and(|d| d.with(|host| host.is_open::<GraphDataDialog>())));
    }

    #[test]
    fn the_entry_box_writes_its_text_and_moves_the_picked_cell() {
        let (mut app, id) = app_with(",a,b\nQ1,1,2", "column");
        open(&mut app).unwrap();
        let moves = [(CellDirection::Down, (1, 1)), (CellDirection::Right, (1, 2)), (CellDirection::Left, (1, 1)), (CellDirection::Up, (0, 1))];
        for (step, to) in moves {
            edit(&app, |d| {
                let from = active(&d.state);
                let typed = format!("t{}{}", from.0, from.1);
                d.entry.text = typed.clone();
                commit_entry(&mut d.sheet, &mut d.state, &mut d.entry, step);
                assert_eq!(cell_of(&d.sheet.grid, from.0, from.1), typed, "the typed text went into the cell it left");
                assert_eq!(active(&d.state), to, "{step:?}");
                assert_eq!(d.entry.text, cell_of(&d.sheet.grid, to.0, to.1), "the box loads the cell it moved to");
            });
        }
        // The top-left cell stays blank: a move onto it stays where it was.
        edit(&app, |d| {
            commit_entry(&mut d.sheet, &mut d.state, &mut d.entry, CellDirection::Left);
            assert_eq!(active(&d.state), (0, 1));
        });
        // Entering text does not apply.
        assert_eq!(data(&mut app, id)["series"], json!(["a", "b"]), "not applied");
        // A pick set from outside loads the cell's text into the box.
        set_field(&app, "sel", json!([1, 0])).unwrap();
        assert_eq!(read(&app, |d| d.entry.text.clone()), "Q1");
    }

    /// A column's end edge in the cell rows is dragged with the mouse: the width is the one at the release, in whole
    /// digits; Esc mid-drag gives the width at the start back; entering the width Column Width shows sets every column.
    #[test]
    fn a_column_drag_ends_where_released_and_the_shown_width_entered_sets_every_column() {
        let (mut app, _) = app_with(",a,b\nQ1,1,2", "column");
        open(&mut app).unwrap();
        let mut h = harness(app);
        h.run_steps(5);
        let digits = |h: &Harness, c: usize| read(h.state(), |d| col_digits(&d.sheet, c));
        let grid = h.get_by_role_and_label(egui::accesskit::Role::Grid, "Graph Data").accesskit_node().locate().0;
        // Column `c`'s end edge over the cells, on the second data row: a pointer-only part, found by the rect the last
        // pass sensed. It runs down every row shown, past the part of the sheet in view.
        let edge = |h: &Harness, c: usize| {
            let part = craft::behavior::PartKey::new("column_edge").at(c as u64);
            let rect = craft::ui::testing::pointer::part_rect_of_node(&h.ctx, grid, part).unwrap_or_else(|| panic!("column {c}'s edge is sensed"));
            egui::pos2(rect.center().x, rect.top() + ROW_H * 1.5)
        };
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let drag = |h: &mut Harness, c: usize, dx: f32, release: bool| {
            // The pointer comes over the edge before it presses, as a mouse's does.
            let at = edge(h, c);
            h.event(egui::Event::PointerMoved(at));
            h.step();
            h.event(button(at, true));
            h.step();
            for step in 1..=4 {
                h.event(egui::Event::PointerMoved(at + egui::vec2(dx * step as f32 / 4.0, 0.0)));
                h.step();
            }
            if release {
                h.event(button(at + egui::vec2(dx, 0.0), false));
                h.step();
            }
        };
        // 70 points is 10 digits, on the dragged column only.
        drag(&mut h, 1, 70.0, true);
        assert_eq!((digits(&h, 0), digits(&h, 1), digits(&h, 2)), (7, 17, 7));
        // Esc in the middle of a drag puts the width back.
        drag(&mut h, 2, 70.0, false);
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.step();
        h.event(button(edge(&h, 2), false));
        h.step();
        assert_eq!(digits(&h, 2), 7, "Esc restores the width at the start");
        // The last move and the release in one frame.
        let at = edge(&h, 0);
        h.event(egui::Event::PointerMoved(at));
        h.step();
        h.event(button(at, true));
        h.step();
        h.event(egui::Event::PointerMoved(at + egui::vec2(-35.0, 0.0)));
        h.event(button(at + egui::vec2(-35.0, 0.0), false));
        // That frame reports the drag's start, the next one the release.
        h.step();
        h.step();
        assert_eq!(digits(&h, 0), 2, "the width where the drag was released");
        // Column Width still shows 7: entering 7 there sets every column to 7.
        set_field(h.state(), "width", json!(7)).unwrap();
        assert_eq!((digits(&h, 0), digits(&h, 1), digits(&h, 2)), (7, 7, 7), "Column Width sets every column");
    }

    #[test]
    fn a_column_width_in_points_maps_to_whole_digits() {
        assert!((0..=MAX_WIDTH).all(|n| px_digits(digits_px(n)) == n));
        assert_eq!((px_digits(-40.0), px_digits(1e9), px_digits(f32::NAN)), (0, MAX_WIDTH, 0));
        let mut sheet = Sheet::default();
        set_col_digits(&mut sheet, 3, 99);
        set_col_digits(&mut sheet, MAX_COLS, 5);
        assert_eq!((col_digits(&sheet, 2), col_digits(&sheet, 3)), (DEFAULT_WIDTH, MAX_WIDTH));
        assert_eq!(sheet.widths.len(), 4, "a column past the sheet is left alone");
        // `width` set through the dialog's fields sets every column too, also when set back to the width before.
        let (mut app, _) = app_with(",a\nQ1,1", "column");
        open(&mut app).unwrap();
        for w in [12, 7] {
            set_field(&app, "width", json!(w)).unwrap();
            assert_eq!(read(&app, |d| (col_digits(&d.sheet, 2), col_digits(&d.sheet, 3))), (w, w));
            edit(&app, |d| set_col_digits(&mut d.sheet, 1, 4));
            set_field(&app, "width", json!(w)).unwrap();
            assert_eq!(read(&app, |d| col_digits(&d.sheet, 1)), w, "setting width back to the one shown clears a column's own");
        }
        edit(&app, |d| set_col_digits(&mut d.sheet, 1, 4));
        assert_eq!(read(&app, |d| (col_digits(&d.sheet, 1), col_digits(&d.sheet, 3))), (4, 7), "a drag after it gives that column its own width");
    }
}
