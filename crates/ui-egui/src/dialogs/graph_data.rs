//! Object › Graph › Data…: the Graph Data window, a worksheet of the graph's data.
//!
//! The first row holds the series labels (the legend), the first column the category labels, the
//! top-left cell stays blank; the other cells are values (a blank cell has none). A cell is picked
//! with a click and edited in the entry box above the sheet: Tab enters it and moves right, Enter
//! (Return) down, Shift with either the other way, and the arrow keys move between cells (so they
//! don't move the caret in the entry box). Pasting tab- or line-separated text fills the cells from
//! the picked one. Import Data reads a tab-separated text file, Transpose row/column swaps rows and
//! columns, Switch x/y swaps a scatter graph's y and x columns, Cell Style sets the number of
//! decimals the sheet shows and the width of every column, dragging a column's right edge sets that
//! column's width (in whole digits, 0 to 20, as Cell Style), and Revert reads the graph's data
//! again. Apply (`graph.setData` with its series, categories and rows) redraws the graph and leaves
//! the window open.
//!
//! Fields: `grid` (rows of cell texts), `sel` ([row, column]), `entry` (the picked cell's text
//! being edited), `decimals` (null: as typed), `width` (digits), `widths` (a column's own width in
//! digits by its index, null or missing: `width`; setting `width` empties it), `__graph` (the graph's
//! id) and `__scatter`.

use serde_json::{Value, json};
use vectorcraft_doc::{MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES};

use super::DialogSpec;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Graph Data.
pub const KIND: &str = "graphData";

/// The sheet's size: a row of labels over a graph's categories, a column of labels before its
/// series; the least it shows.
const MAX_ROWS: usize = MAX_GRAPH_CATEGORIES + 1;
const MAX_COLS: usize = MAX_GRAPH_SERIES + 1;
const MIN_ROWS: usize = 8;
const MIN_COLS: usize = 6;
/// The largest file Import Data reads, 1 MB (1,048,576 bytes), the same as Import Data Sets'.
const MAX_IMPORT_BYTES: usize = vectorcraft_engine::cmd::MAX_GRAPH_DATA_FILE;
/// The Cell Style limits (Adjust column width and decimal digits).
const MAX_DECIMALS: u64 = 10;
const MAX_WIDTH: u64 = 20;
/// A cell's height on the sheet.
const ROW_H: f32 = 19.0;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Graph Data").into(),
    body,
    confirm: |app, d| apply(app, d).map(|_| Value::Null),
    ok: Some("Apply"),
    min_width: 620.0,
    ..DialogSpec::FORM
};

/// Open Graph Data for the selected graph.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    // The query fails without a graph selected, with the engine's own message.
    let v = app.session.execute("graph.setData", &json!({})).map_err(|e| e.to_string())?;
    let graph = app.session.active().and_then(|st| st.selection.objects.iter().find_map(|id| graph_of(&st.doc, *id)).map(|id| (st, id)));
    let scatter = graph.is_some_and(|(st, id)| {
        st.doc.node(id).and_then(|n| n.extras().graph.as_deref()).is_some_and(|g| g.kind == vectorcraft_doc::GraphKind::Scatter)
    });
    let id = graph.map(|(_, id)| id.0);
    let grid = grid_from_data(&v);
    let entry = cell(&grid, 0, 1);
    app.ui.dialog = Some(Dialog::new(
        KIND,
        json!({ "grid": grid, "sel": [0, 1], "entry": entry, "decimals": null, "width": 7, "widths": [], "__graph": id, "__scatter": scatter }),
    ));
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

/// A button's work, done once the sheet is drawn.
type Action = fn(&mut VectorcraftApp, &mut Dialog) -> Result<(), String>;

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

fn cell(grid: &[Vec<String>], r: usize, c: usize) -> String {
    grid.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default()
}

/// Cell `(r, c)` of the dialog's sheet, read in place.
fn cell_at(d: &Dialog, r: usize, c: usize) -> &str {
    d.fields.get("grid").and_then(|g| g.get(r)).and_then(|row| row.get(c)).and_then(Value::as_str).unwrap_or_default()
}

/// Set cell `(r, c)` of the dialog's sheet in place, growing it as needed (within the sheet's size).
fn set_cell_at(d: &mut Dialog, r: usize, c: usize, text: &str) {
    if r >= MAX_ROWS || c >= MAX_COLS {
        return;
    }
    let grid = d.fields.entry("grid").or_insert_with(|| json!([]));
    let Some(rows) = grid.as_array_mut() else { return };
    if rows.len() <= r {
        rows.resize(r + 1, json!([]));
    }
    let Some(row) = rows.get_mut(r).and_then(Value::as_array_mut) else { return };
    if row.len() <= c {
        row.resize(c + 1, json!(""));
    }
    if let Some(cell) = row.get_mut(c) {
        *cell = json!(clean(text));
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

/// How many rows of the dialog's sheet hold data (below the labels' row), read in place.
fn data_rows(d: &Dialog) -> usize {
    let filled = |r: &Value| r.as_array().is_some_and(|r| r.iter().any(|c| c.as_str().is_some_and(|c| !c.trim().is_empty())));
    d.fields.get("grid").and_then(Value::as_array).and_then(|g| g.iter().rposition(filled)).unwrap_or(0)
}

/// How big the sheet is drawn: the data, a spare row and column, and at least MIN_ROWS × MIN_COLS.
fn extent(d: &Dialog, sel: (usize, usize)) -> (usize, usize) {
    let rows = d.fields.get("grid").and_then(Value::as_array);
    let n = rows.map_or(0, Vec::len);
    let w = rows.into_iter().flatten().filter_map(Value::as_array).map(Vec::len).max().unwrap_or(0);
    ((n + 1).max(MIN_ROWS).max(sel.0 + 2).min(MAX_ROWS), (w + 1).max(MIN_COLS).max(sel.1 + 2).min(MAX_COLS))
}

/// A key that moves the picked cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Move {
    Right,
    Down,
    Up,
    Left,
}

/// The cell `m` moves to from `sel`, within the sheet (the top-left blank cell is skipped).
pub(crate) fn step(sel: (usize, usize), m: Move) -> (usize, usize) {
    let to = match m {
        Move::Right => (sel.0, (sel.1 + 1).min(MAX_COLS - 1)),
        Move::Down => ((sel.0 + 1).min(MAX_ROWS - 1), sel.1),
        Move::Up => (sel.0.saturating_sub(1), sel.1),
        Move::Left => (sel.0, sel.1.saturating_sub(1)),
    };
    if to == (0, 0) { sel } else { to }
}

/// A cell as the sheet shows it: a number with `decimals` places when Cell Style sets them, else as
/// typed (the number Apply sends is as typed either way).
pub(crate) fn shown(text: &str, decimals: Option<u64>) -> String {
    match (decimals, text.trim().parse::<f64>()) {
        (Some(d), Ok(v)) if v.is_finite() && !text.trim().starts_with('"') => format!("{v:.*}", d.min(MAX_DECIMALS) as usize),
        _ => text.to_string(),
    }
}

fn grid(d: &Dialog) -> Grid {
    d.fields.get("grid").and_then(|g| serde_json::from_value(g.clone()).ok()).unwrap_or_default()
}

fn sel(d: &Dialog) -> (usize, usize) {
    let get = |i: usize| d.fields.get("sel").and_then(|s| s.get(i)).and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
    (get(0).min(MAX_ROWS - 1), get(1).min(MAX_COLS - 1))
}

/// The entry box's text into the picked cell.
fn commit(d: &mut Dialog) {
    let (r, c) = sel(d);
    let text = d.str("entry");
    if cell_at(d, r, c) != text {
        set_cell_at(d, r, c, &text);
    }
}

/// Pick cell `to`, its text in the entry box.
fn pick(d: &mut Dialog, to: (usize, usize)) {
    commit(d);
    d.fields.insert("sel".into(), json!([to.0, to.1]));
    let text = cell_at(d, to.0, to.1).to_string();
    d.fields.insert("entry".into(), json!(text));
}

fn replace_grid(d: &mut Dialog, g: Grid) {
    let (r, c) = sel(d);
    d.fields.insert("entry".into(), json!(cell(&g, r, c)));
    d.fields.insert("grid".into(), json!(g));
}

/// Paste: text with tabs or line breaks fills the cells from the picked one.
fn paste_block(d: &mut Dialog, text: &str) {
    let (r0, c0) = sel(d);
    for (r, row) in grid_from_text(text).into_iter().enumerate() {
        for (c, t) in row.into_iter().enumerate() {
            set_cell_at(d, r0 + r, c0 + c, &t);
        }
    }
    let text = cell_at(d, r0, c0).to_string();
    d.fields.insert("entry".into(), json!(text));
}

/// Apply: the sheet's data to the graph.
fn apply(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    commit(&mut d);
    // The graph the sheet opened on (else the selected one).
    let mut p = data_from_grid(&grid(&d));
    p["id"] = d.fields.get("__graph").cloned().unwrap_or(Value::Null);
    let r = app.run("graph.setData", p);
    if let Some(open) = app.ui.dialog.as_mut().filter(|o| o.kind == KIND) {
        open.fields = d.fields;
    }
    r
}

/// Revert: the graph's data again.
fn revert(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let id = d.fields.get("__graph").cloned().unwrap_or(Value::Null);
    let v = app.session.execute("graph.setData", &json!({ "id": id })).map_err(|e| e.to_string())?;
    replace_grid(d, grid_from_data(&v));
    Ok(())
}

/// Import Data: a tab-separated text file's rows into the sheet.
fn import(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let pick = crate::FilePick { filters: vec![crate::FilePick::filter("Text", &["txt", "tsv", "csv"])], ..Default::default() };
    let path = crate::picks::open(app, &pick).ok_or("cancelled")?;
    // One byte past the limit tells a larger file apart without reading all of it.
    let bytes = match (&app.services.read_head, &app.services.read) {
        (Some(read_head), _) => read_head(&path, MAX_IMPORT_BYTES + 1)?,
        (None, Some(read)) => read(&path)?,
        // The button is off where files can't be read.
        (None, None) => return Ok(()),
    };
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err("the file is larger than 1 MB".into());
    }
    replace_grid(d, grid_from_text(&decode(&bytes)));
    Ok(())
}

/// Draw the graph-data sheet: keyboard cell movement, block paste, import, transpose, switch-x/y
/// and revert actions, decimals and column widths, and only the cells in view.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let entry_id = egui::Id::unique("graph-data-entry");
    if ui.memory(|m| m.has_focus(entry_id)) {
        // Tab, Enter (Shift: the other way) and the arrow keys move the picked cell while the entry box
        // has the keys; the entry keeps them (egui would move the focus on with Tab).
        // Shift first: a key asked for without modifiers also matches it with Shift held.
        let keys = [
            (egui::Modifiers::SHIFT, egui::Key::Tab, Move::Left),
            (egui::Modifiers::SHIFT, egui::Key::Enter, Move::Up),
            (egui::Modifiers::NONE, egui::Key::Tab, Move::Right),
            (egui::Modifiers::NONE, egui::Key::Enter, Move::Down),
            (egui::Modifiers::NONE, egui::Key::ArrowDown, Move::Down),
            (egui::Modifiers::NONE, egui::Key::ArrowUp, Move::Up),
            (egui::Modifiers::NONE, egui::Key::ArrowRight, Move::Right),
            (egui::Modifiers::NONE, egui::Key::ArrowLeft, Move::Left),
        ];
        let moved = keys.iter().find(|(m, k, _)| ui.input_mut(|i| i.consume_key(*m, *k))).map(|(_, _, mv)| *mv);
        if let Some(m) = moved {
            pick(d, step(sel(d), m));
            ui.memory_mut(|mem| {
                mem.request_focus(entry_id);
                mem.move_focus(egui::FocusDirection::None);
            });
        }
        // A pasted block (a spreadsheet's cells) fills the sheet instead of the entry box.
        let block = ui.input_mut(|i| {
            let k = i.events.iter().position(|e| matches!(e, egui::Event::Paste(s) if s.contains('\t') || s.contains('\n')))?;
            match i.events.remove(k) {
                egui::Event::Paste(s) => Some(s),
                _ => None,
            }
        });
        if let Some(text) = block {
            paste_block(d, &text);
        }
    }
    let mut action: Option<Action> = None;
    ui.horizontal(|ui| {
        let mut text = d.str("entry");
        // The entry box has the keys when the window opens, so Enter moves down instead of applying.
        widgets::take_dialog_focus(ui, entry_id, &text);
        let r = ui.add(egui::TextEdit::singleline(&mut text).id(entry_id).align(egui::Align2::LEFT_CENTER).desired_width(260.0));
        if r.changed() {
            d.fields.insert("entry".into(), json!(clean(&text)));
        }
        ui.add_space(8.0);
        let can_read = app.services.read.is_some();
        if ui.add_enabled_ui(can_read, |ui| widgets::flat_button(ui, tl!("Import Data…"), 96.0)).inner.clicked() {
            action = Some(|app, d| crate::picks::in_dialog(app, d, import));
        }
        // Transposed, the categories become series: at most as many as a graph has.
        if ui.add_enabled_ui(data_rows(d) <= MAX_GRAPH_SERIES, |ui| widgets::flat_button(ui, tl!("Transpose"), 80.0)).inner.clicked() {
            commit(d);
            replace_grid(d, transpose(&grid(d)));
        }
        if ui.add_enabled_ui(d.bool("__scatter"), |ui| widgets::flat_button(ui, tl!("Switch x/y"), 80.0)).inner.clicked() {
            commit(d);
            replace_grid(d, switch_xy(&grid(d)));
        }
        if widgets::flat_button(ui, tl!("Revert"), 64.0).clicked() {
            action = Some(revert);
        }
    });
    // Cell Style: decimals shown and the columns' width in digits.
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Number of Decimals:"));
        let mut dec = d.fields.get("decimals").and_then(Value::as_u64);
        let mut on = dec.is_some();
        if ui.checkbox(&mut on, "").changed() {
            dec = on.then_some(2);
        }
        let mut n = dec.unwrap_or(2);
        if ui.add_enabled(on, egui::DragValue::new(&mut n).range(0..=MAX_DECIMALS)).changed() {
            dec = Some(n);
        }
        d.fields.insert("decimals".into(), json!(dec));
        ui.add_space(12.0);
        widgets::dim_label(ui, tl!("Column Width:"));
        let mut w = d.fields.get("width").and_then(Value::as_u64).unwrap_or(7);
        let r = ui.add(egui::DragValue::new(&mut w).range(0..=MAX_WIDTH));
        // A value entered with Enter sets every column, also when it is the width shown. Leaving the
        // field without a new value keeps the columns' own widths.
        if r.changed() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            set_width(d, w);
        }
    });
    ui.add_space(6.0);
    let (clicked, resized) = sheet(ui, d);
    if let Some((c, n)) = resized
        && n != col_digits(d, c)
    {
        set_col_digits(d, c, n);
    }
    if let Some(to) = clicked {
        pick(d, to);
        ui.memory_mut(|mem| mem.request_focus(entry_id));
    }
    if let Some(f) = action
        && let Err(e) = f(app, d)
        && e != "cancelled"
    {
        app.status(e);
    }
    false
}

/// Cell Style's Column Width in digits.
fn width(d: &Dialog) -> u64 {
    d.fields.get("width").and_then(Value::as_u64).unwrap_or(7).min(MAX_WIDTH)
}

/// A column's width in digits: its own when its edge was dragged, else Cell Style's.
fn col_digits(d: &Dialog, c: usize) -> u64 {
    d.fields.get("widths").and_then(|w| w.get(c)).and_then(Value::as_u64).unwrap_or_else(|| width(d)).min(MAX_WIDTH)
}

/// Cell Style's Column Width: every column `n` digits wide, the ones given their own width too.
fn set_width(d: &mut Dialog, n: u64) {
    d.fields.insert("width".into(), json!(n.min(MAX_WIDTH)));
    note_set(d, "width");
}

/// `ui.dialog.set` set `field`: a Column Width sets every column, as Cell Style's does.
pub(super) fn note_set(d: &mut Dialog, field: &str) {
    if field == "width" {
        d.fields.insert("widths".into(), json!([]));
    }
}

/// Set column `c`'s own width in digits (at most MAX_WIDTH).
fn set_col_digits(d: &mut Dialog, c: usize, n: u64) {
    if c >= MAX_COLS {
        return;
    }
    let widths = d.fields.entry("widths").or_insert_with(|| json!([]));
    if !widths.is_array() {
        *widths = json!([]);
    }
    let Some(widths) = widths.as_array_mut() else { return };
    if widths.len() <= c {
        widths.resize(c + 1, Value::Null);
    }
    if let Some(w) = widths.get_mut(c) {
        *w = json!(n.min(MAX_WIDTH));
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

/// The id of column `c`'s right edge on the sheet.
fn edge_id(c: usize) -> egui::Id {
    egui::Id::unique(("graph-data-column-edge", c))
}

/// The cell clicked on the sheet ([row, column]), and the column whose edge is dragged with its width
/// in digits.
type SheetInput = (Option<(usize, usize)>, Option<(usize, u64)>);

/// Draw the sheet's cells in view, each column at its own width. Returns the cell clicked, and the
/// column whose right edge is being dragged with the width in digits the pointer gives it.
fn sheet(ui: &mut egui::Ui, d: &Dialog) -> SheetInput {
    let at = sel(d);
    let (rows, cols) = extent(d, at);
    let decimals = d.fields.get("decimals").and_then(Value::as_u64);
    // Each column's left edge, and the sheet's width last: a column is its width and a 1-point gap.
    let mut xs = Vec::with_capacity(cols + 1);
    let mut x = 0.0;
    xs.push(x);
    for c in 0..cols {
        x += digits_px(col_digits(d, c)) + 1.0;
        xs.push(x);
    }
    let left = |c: usize| xs.get(c).copied().unwrap_or(x);
    let (mut clicked, mut resized) = (None, None);
    widgets::list_box(ui, |ui| {
        // Only the cells in view are drawn: a graph's sheet can be 10,001 × 257.
        egui::ScrollArea::both().id_salt("graph-data-sheet").max_height(280.0).auto_shrink([false, false]).show_viewport(ui, |ui, view| {
            let ch = ROW_H;
            ui.set_min_size(egui::vec2(x, rows as f32 * ch));
            let origin = ui.max_rect().min;
            let (r0, r1) = ((view.min.y / ch).floor().max(0.0) as usize, ((view.max.y / ch).ceil().max(0.0) as usize).min(rows));
            // The columns that reach into the view.
            let c0 = (0..cols).find(|&c| left(c + 1) > view.min.x).unwrap_or(cols);
            let c1 = (c0..cols).find(|&c| left(c) >= view.max.x).unwrap_or(cols);
            for r in r0..r1 {
                for c in c0..c1 {
                    let size = egui::vec2(left(c + 1) - left(c) - 1.0, ch - 1.0);
                    let rect = egui::Rect::from_min_size(origin + egui::vec2(left(c), r as f32 * ch), size);
                    let text = shown(cell_at(d, r, c), decimals);
                    let b = egui::Button::selectable((r, c) == at, egui::RichText::new(text).monospace()).truncate();
                    if ui.put(rect, b).clicked() && (r, c) != (0, 0) {
                        clicked = Some((r, c));
                    }
                }
            }
            // A column's right edge, over the rows in view: dragging it sets that column's width. The
            // edge being dragged stays in the loop when it leaves the view, so the drag goes on, also in
            // the frame it is released.
            let (top, bottom) = (origin.y + r0 as f32 * ch, origin.y + r1 as f32 * ch);
            let dragged = ui
                .ctx()
                .dragged_id()
                .or_else(|| ui.ctx().drag_stopped_id())
                .and_then(|id| (0..cols).find(|&c| edge_id(c) == id))
                .filter(|c| !(c0..c1).contains(c));
            for c in (c0..c1).chain(dragged) {
                let gap = origin.x + left(c + 1) - 0.5;
                let edge = egui::Rect::from_x_y_ranges(gap - 3.0..=gap + 3.0, top..=bottom);
                let h = ui.interact(edge, edge_id(c), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeColumn);
                // The width is the column's width when the drag started plus how far the pointer has
                // moved since, so a scroll offset that changes with the sheet's width doesn't change it.
                if h.drag_started()
                    && let Some(p) = ui.input(|i| i.pointer.press_origin()).or_else(|| h.interact_pointer_pos())
                {
                    ui.data_mut(|m| m.insert_temp(edge_id(c), (p.x, left(c + 1) - left(c) - 1.0)));
                }
                if h.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
                }
                // The release can come in the same frame as the last move: it sets the width too.
                if (h.dragged() || h.drag_stopped())
                    && let Some(p) = h.interact_pointer_pos()
                    && let Some((x0, w0)) = ui.data(|m| m.get_temp::<(f32, f32)>(edge_id(c)))
                {
                    resized = Some((c, px_digits(w0 + p.x - x0)));
                }
                if h.drag_stopped() {
                    ui.data_mut(|m| m.remove::<(f32, f32)>(edge_id(c)));
                }
            }
        });
    });
    (clicked, resized)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn g(rows: &[&[&str]]) -> Grid {
        rows.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect()
    }

    fn app_with(csv: &str, ty: &str) -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let id =
            app.run("graph.create", json!({"type": ty, "x": 0, "y": 0, "width": 200, "height": 150, "csv": csv})).unwrap()["id"].as_u64().unwrap();
        (app, id)
    }

    fn data(app: &mut VectorcraftApp, id: u64) -> Value {
        app.session.execute("graph.setData", &json!({"id": id})).unwrap()
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
            let d = app.ui.dialog.clone().unwrap();
            apply(&mut app, &d).unwrap();
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
    /// read. Where the host only reads whole files, a larger file is refused too.
    #[test]
    fn import_data_refuses_a_file_larger_than_its_limit() {
        use std::{cell::RefCell, collections::HashMap, rc::Rc};
        let (mut app, _) = app_with(",a\nQ1,1", "column");
        open(&mut app).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        let at_limit = [b"\tb\nQ2\t2\n".as_slice(), &vec![b' '; MAX_IMPORT_BYTES - 8]].concat();
        let files: HashMap<String, Vec<u8>> =
            [("/big.txt".to_string(), vec![b'1'; MAX_IMPORT_BYTES + 1]), ("/limit.txt".to_string(), at_limit)].into();
        let files = Rc::new(files);
        let picked = Rc::new(RefCell::new(String::new()));
        let p = picked.clone();
        app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(p.borrow().clone())));
        app.services.read = Some(Box::new(|_: &str| Err("the whole file was read".to_string())));
        let asked = Rc::new(RefCell::new(vec![]));
        let (f, a) = (files.clone(), asked.clone());
        app.services.read_head = Some(Box::new(move |path: &str, max: usize| {
            a.borrow_mut().push(max);
            Ok(f.get(path).ok_or("no such file")?.iter().copied().take(max).collect())
        }));
        let before = grid(&d);
        *picked.borrow_mut() = "/big.txt".into();
        assert_eq!(import(&mut app, &mut d), Err("the file is larger than 1 MB".to_string()));
        assert_eq!(grid(&d), before, "the sheet is left as it was");
        *picked.borrow_mut() = "/limit.txt".into();
        import(&mut app, &mut d).unwrap();
        assert_eq!((cell_at(&d, 0, 1), cell_at(&d, 1, 0)), ("b", "Q2"));
        assert_eq!(*asked.borrow(), [MAX_IMPORT_BYTES + 1; 2], "each read stops one byte past the limit");
        // A host that reads only whole files.
        app.services.read_head = None;
        let f = files.clone();
        app.services.read = Some(Box::new(move |path: &str| Ok(f.get(path).ok_or("no such file")?.clone())));
        *picked.borrow_mut() = "/big.txt".into();
        assert_eq!(import(&mut app, &mut d), Err("the file is larger than 1 MB".to_string()));
        assert_eq!(cell_at(&d, 0, 1), "b");
    }

    #[test]
    fn transpose_switch_and_moves() {
        assert_eq!(transpose(&g(&[&["", "a", "b"], &["Q1", "1", "2"]])), g(&[&["", "Q1"], &["a", "1"], &["b", "2"]]));
        assert_eq!(step((0, 1), Move::Right), (0, 2));
        assert_eq!(step((0, 1), Move::Down), (1, 1));
        assert_eq!(step((1, 0), Move::Up), (1, 0), "the top-left cell stays blank");
        assert_eq!(step((MAX_ROWS - 1, 1), Move::Down), (MAX_ROWS - 1, 1));
        assert_eq!(shown("3.14159", Some(2)), "3.14");
        assert_eq!(shown("3.14159", None), "3.14159");
        // Switch x/y in the sheet does what the engine's does.
        let (mut app, id) = app_with("y1,x1,y2,x2\n1,10,2,20\n3,30,4,40", "scatter");
        open(&mut app).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        let switched = switch_xy(&grid(&d));
        replace_grid(&mut d, switched);
        apply(&mut app, &d).unwrap();
        let sheet = data(&mut app, id);
        let (mut other, oid) = app_with("y1,x1,y2,x2\n1,10,2,20\n3,30,4,40", "scatter");
        other.run("graph.setData", json!({"id": oid, "switchXY": true})).unwrap();
        assert_eq!(sheet, data(&mut other, oid));
    }

    #[test]
    fn data_opens_the_sheet_and_apply_redraws_the_graph() {
        let (mut app, id) = app_with(",a,b\nQ1,1,2", "column");
        crate::menus::invoke(&mut app, "graph.setData", json!({}));
        let mut d = app.ui.dialog.take().unwrap();
        assert_eq!(d.kind, KIND);
        pick(&mut d, (1, 2));
        d.fields.insert("entry".into(), json!("5"));
        pick(&mut d, (2, 0));
        d.fields.insert("entry".into(), json!("Q2"));
        pick(&mut d, (2, 1));
        d.fields.insert("entry".into(), json!("7"));
        app.ui.dialog = Some(d.clone());
        apply(&mut app, &d).unwrap();
        assert_eq!(data(&mut app, id)["rows"], json!([[1.0, 5.0], [7.0, null]]));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "Apply leaves the window open");
        // A pasted block fills the cells from the picked one.
        let mut d = app.ui.dialog.take().unwrap();
        pick(&mut d, (1, 1));
        paste_block(&mut d, "8\t9\n10\t11\n");
        assert_eq!((cell_at(&d, 1, 1), cell_at(&d, 2, 2), d.str("entry")), ("8", "11", "8".to_string()));
        // Revert reads the graph again; the tool's Graph Data after drawing opens the same sheet.
        replace_grid(&mut d, vec![]);
        revert(&mut app, &mut d).unwrap();
        assert_eq!(cell_at(&d, 0, 1), "a");
        super::super::tools::open_tool_dialog(&mut app, "graphData", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
    }

    #[test]
    fn the_keys_move_the_picked_cell_and_keep_the_entry_box() {
        let (mut app, id) = app_with(",a,b\nQ1,1,2", "column");
        open(&mut app).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
            out.textures_delta.clear();
        };
        let key = |k: egui::Key, m: egui::Modifiers| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m };
        let none = egui::Modifiers::NONE;
        // The window opening gives the entry box the keys.
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        frame(&mut app, vec![egui::Event::Text("9".into())]);
        frame(&mut app, vec![key(egui::Key::Tab, none)]);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(sel(d), (0, 2), "Tab: right");
        assert!(cell_at(d, 0, 1).ends_with('9'), "the typed text went into the cell it left: {}", cell_at(d, 0, 1));
        // Still the entry's keys after a Tab: Enter moves down rather than applying.
        frame(&mut app, vec![key(egui::Key::Enter, none)]);
        assert_eq!(sel(app.ui.dialog.as_ref().unwrap()), (1, 2), "Enter: down");
        assert_eq!(data(&mut app, id)["series"], json!(["a", "b"]), "not applied");
        frame(&mut app, vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)]);
        assert_eq!(sel(app.ui.dialog.as_ref().unwrap()), (1, 1), "Shift+Tab: left");
        frame(&mut app, vec![key(egui::Key::ArrowUp, none)]);
        assert_eq!(sel(app.ui.dialog.as_ref().unwrap()), (0, 1), "Up");
    }

    /// Dragging a column's right edge shows the column-resize pointer and sets that column's width in
    /// whole digits, the columns after it moving along; the other columns keep Cell Style's width.
    /// A drag past 20 digits stops at 20, and Cell Style's Column Width then sets every column.
    #[test]
    fn dragging_a_column_edge_sets_that_columns_width() {
        let (mut app, _) = app_with(",a,b\nQ1,1,2", "column");
        open(&mut app).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
            out.textures_delta.clear();
            out.platform_output.cursor_icon
        };
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let edge = |c: usize| ctx.read_response(edge_id(c)).map(|r| r.rect).unwrap_or_else(|| panic!("column {c}'s edge is drawn"));
        let drag = |app: &mut VectorcraftApp, c: usize, dx: f32| {
            let at = edge(c).center();
            frame(app, vec![egui::Event::PointerMoved(at), button(at, true)]);
            for step in 1..=4 {
                frame(app, vec![egui::Event::PointerMoved(at + egui::vec2(dx * step as f32 / 4.0, 0.0))]);
            }
            frame(app, vec![button(at + egui::vec2(dx, 0.0), false)]);
            frame(app, vec![egui::Event::PointerMoved(egui::pos2(0.0, 0.0))]);
        };
        let digits = |app: &VectorcraftApp| {
            let d = app.ui.dialog.as_ref().unwrap();
            (0..4).map(|c| col_digits(d, c)).collect::<Vec<_>>()
        };
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        // The window takes its place over the first frames: hover where the edge is now.
        let hovered = (0..10).any(|_| frame(&mut app, vec![egui::Event::PointerMoved(edge(1).center())]) == egui::CursorIcon::ResizeColumn);
        assert!(hovered, "hovering an edge shows the column-resize pointer");
        let (one, two) = (edge(1).center().x, edge(2).center().x);
        // 70 points is 10 digits.
        drag(&mut app, 1, 70.0);
        assert_eq!(digits(&app), [7, 17, 7, 7]);
        let moved = [edge(1).center().x - one, edge(2).center().x - two];
        assert!(moved.iter().all(|m| (m - 70.0).abs() < 0.01), "the columns after it move along: {moved:?}");
        drag(&mut app, 2, -500.0);
        assert_eq!(digits(&app), [7, 17, 0, 7], "no narrower than 0 digits");
        drag(&mut app, 0, 500.0);
        assert_eq!(digits(&app), [20, 17, 0, 7], "no wider than 20 digits");
        let d = app.ui.dialog.as_mut().unwrap();
        set_width(d, 9);
        assert_eq!(digits(&app), [9; 4], "Cell Style sets every column");
    }

    /// A drag whose last move comes in the frame it is released sets the width to where it was
    /// released. A held drag on a sheet scrolled to its right end keeps its width while the scroll
    /// offset follows the narrower sheet. Entering the width that Column Width shows sets every
    /// column, the dragged ones too.
    #[test]
    fn a_column_drag_ends_where_released_and_the_shown_width_entered_sets_every_column() {
        let rows: Vec<Vec<f64>> = (0..3).map(|i| vec![i as f64; 30]).collect();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150, "rows": rows})).unwrap();
        open(&mut app).unwrap();
        let cols = extent(app.ui.dialog.as_ref().unwrap(), (0, 1)).1;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out =
                ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
            out.textures_delta.clear();
            out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
        };
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let edge = |c: usize| ctx.read_response(edge_id(c)).map(|r| r.rect).unwrap_or_else(|| panic!("column {c}'s edge is drawn"));
        let digits = |app: &VectorcraftApp, c: usize| col_digits(app.ui.dialog.as_ref().unwrap(), c);
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        // The last move and the release in one frame: 70 points is 10 digits.
        let at = edge(1).center();
        frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
        let to = at + egui::vec2(70.0, 0.0);
        frame(&mut app, vec![egui::Event::PointerMoved(to), button(to, false)]);
        frame(&mut app, vec![egui::Event::PointerMoved(egui::pos2(0.0, 0.0))]);
        assert_eq!(digits(&app, 1), 17, "the width where the drag was released");
        // Scroll the sheet to its right end, then narrow a column by 5 digits and hold the pointer still.
        let over = edge(1).center() - egui::vec2(20.0, 0.0);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(-100_000.0, 0.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(over), wheel]);
        for _ in 0..30 {
            frame(&mut app, vec![]);
        }
        let c = cols - 2;
        let at = edge(c).center();
        frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
        for step in 1..=5 {
            frame(&mut app, vec![egui::Event::PointerMoved(at - egui::vec2(7.0 * step as f32, 0.0))]);
        }
        for _ in 0..10 {
            frame(&mut app, vec![]);
        }
        assert_eq!(digits(&app, c), 2, "a held drag keeps its width");
        let to = at - egui::vec2(35.0, 0.0);
        frame(&mut app, vec![button(to, false)]);
        frame(&mut app, vec![egui::Event::PointerMoved(egui::pos2(0.0, 0.0))]);
        assert_eq!(digits(&app, c), 2, "the width where the drag was released");
        // Column Width still shows 7: entering 7 there sets every column to 7.
        let nodes = frame(&mut app, vec![]);
        let label = nodes
            .iter()
            .find(|(_, n)| n.role() == egui::accesskit::Role::Label && n.value() == Some("Column Width:"))
            .and_then(|(_, n)| n.bounds())
            .expect("the Column Width label is drawn");
        let field = nodes
            .iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::SpinButton)
            .filter_map(|(_, n)| n.bounds())
            .filter(|b| b.x0 >= label.x1 && b.y0 < label.y1 && b.y1 > label.y0)
            .min_by(|a, b| a.x0.total_cmp(&b.x0))
            .expect("the Column Width field is drawn");
        let at = egui::pos2(((field.x0 + field.x1) / 2.0) as f32, ((field.y0 + field.y1) / 2.0) as f32);
        let click = |app: &mut VectorcraftApp, at| {
            frame(app, vec![egui::Event::PointerMoved(at), button(at, true)]);
            frame(app, vec![button(at, false)]);
            frame(app, vec![]);
        };
        // Clicking into the field and away from it, without a new value, keeps the columns' widths.
        click(&mut app, at);
        click(&mut app, egui::pos2(((label.x0 + label.x1) / 2.0) as f32, ((label.y0 + label.y1) / 2.0) as f32));
        assert_eq!((digits(&app, 1), digits(&app, c), digits(&app, 0)), (17, 2, 7), "leaving the field keeps the widths");
        click(&mut app, at);
        frame(&mut app, vec![egui::Event::Text("7".into())]);
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&mut app, vec![enter]);
        frame(&mut app, vec![]);
        assert_eq!((digits(&app, 1), digits(&app, c), digits(&app, 0)), (7, 7, 7), "Column Width sets every column");
    }

    #[test]
    fn a_column_width_in_points_maps_to_whole_digits() {
        for n in 0..=MAX_WIDTH {
            assert_eq!(px_digits(digits_px(n)), n);
        }
        assert_eq!((px_digits(-40.0), px_digits(1e9), px_digits(f32::NAN)), (0, MAX_WIDTH, 0));
        let mut d = Dialog::new(KIND, json!({ "width": 7 }));
        set_col_digits(&mut d, 3, 99);
        set_col_digits(&mut d, MAX_COLS, 5);
        assert_eq!((col_digits(&d, 2), col_digits(&d, 3)), (7, MAX_WIDTH));
        assert_eq!(d.fields["widths"].as_array().map(Vec::len), Some(4), "a column past the sheet is left alone");
        // `width` set by `ui.dialog.set` sets every column too, also when set back to the width before.
        for w in [12, 7] {
            crate::dialogs::set_field(&mut d, "width".into(), json!(w));
            assert_eq!((col_digits(&d, 2), col_digits(&d, 3)), (w, w));
        }
        set_col_digits(&mut d, 1, 4);
        assert_eq!((col_digits(&d, 1), col_digits(&d, 3)), (4, 7), "a drag after it gives that column its own width");
    }

    /// How many buttons (the sheet's cells and the window's own) Graph Data draws in the second of two
    /// frames in a 1280 × 900 window, for a graph of `categories` rows of `series` values.
    fn buttons_drawn(categories: usize, series: usize) -> usize {
        let rows: Vec<Vec<f64>> = (0..categories).map(|i| vec![i as f64; series]).collect();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150, "rows": rows})).unwrap();
        open(&mut app).unwrap();
        let tree = crate::widgets::accessibility_tests::tree(|ui| super::super::show(&mut app, ui.ctx()));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        tree.iter().filter(|n| n.role() == egui::accesskit::Role::Button).count()
    }

    #[test]
    fn a_full_size_sheet_draws_only_what_is_in_view() {
        // 40 categories of 40 values fill 41 × 41 cells of a 42 × 42 sheet, larger than the view both ways: it
        // draws some cells, but fewer than the data fill.
        let some = buttons_drawn(40, 40);
        assert!(some > 20 && some < 41 * 41, "{some} buttons");
        // A full-size sheet draws the same cells in view, and none more.
        assert_eq!(buttons_drawn(MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES), some);
    }
}
