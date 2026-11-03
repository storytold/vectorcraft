//! Object › Graph › Data…: the Graph Data window, a worksheet of the graph's data.
//!
//! The first row holds the series labels (the legend), the first column the category labels, the
//! top-left cell stays blank; the other cells are values (a blank cell has none). A cell is picked
//! with a click and edited in the entry box above the sheet: Tab enters it and moves right, Enter
//! (Return) down, Shift with either the other way, and the arrow keys move between cells (so they
//! don't move the caret in the entry box). Pasting tab- or line-separated text fills the cells from
//! the picked one. Import Data reads a tab-separated text file, Transpose row/column swaps rows and
//! columns, Switch x/y swaps a scatter graph's y and x columns, Cell Style sets the number of
//! decimals the sheet shows and its columns' width, and Revert reads the graph's data again. Apply
//! (`graph.setData` with its series, categories and rows) redraws the graph and leaves the window
//! open.
//!
//! Fields: `grid` (rows of cell texts), `sel` ([row, column]), `entry` (the picked cell's text
//! being edited), `decimals` (null: as typed), `width` (digits), `__graph` (the graph's id) and
//! `__scatter`.

use serde_json::{Value, json};
use vectorcraft_doc::{MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES};

use super::DialogSpec;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Graph Data.
pub const KIND: &str = "graphData";

/// The sheet's size: a row of labels over a graph's categories, a column of labels before its
/// series; the least it shows; a cell's longest text.
const MAX_ROWS: usize = MAX_GRAPH_CATEGORIES + 1;
const MAX_COLS: usize = MAX_GRAPH_SERIES + 1;
const MIN_ROWS: usize = 8;
const MIN_COLS: usize = 6;
const MAX_CELL: usize = 256;
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
        json!({ "grid": grid, "sel": [0, 1], "entry": entry, "decimals": null, "width": 7, "__graph": id, "__scatter": scatter }),
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

/// The sheet as `graph.setData` takes it: series labels, category labels (none when all are blank)
/// and the values (a blank cell, or text that isn't a number, has none), without the empty rows and
/// columns past the data.
pub(crate) fn data_from_grid(grid: &Grid) -> Value {
    let filled = |s: &String| !s.trim().is_empty();
    let rows = grid.iter().rposition(|r| r.iter().any(filled)).map_or(1, |i| i + 1).max(1);
    let cols = grid.iter().take(rows).filter_map(|r| r.iter().rposition(filled)).max().map_or(1, |i| i + 1).max(1);
    let label = |s: &str| {
        let s = s.trim();
        // A label typed in straight quotes (as the CSV form takes a number as a label) is the text inside.
        s.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(s).to_string()
    };
    // Series labels up to the last one given (none when all are blank).
    let mut series: Vec<String> = (1..cols).map(|c| label(&cell(grid, 0, c))).collect();
    while series.last().is_some_and(String::is_empty) {
        series.pop();
    }
    let mut categories: Vec<String> = (1..rows).map(|r| label(&cell(grid, r, 0))).collect();
    if categories.iter().all(String::is_empty) {
        categories.clear();
    }
    let values: Vec<Vec<Value>> = (1..rows)
        .map(|r| {
            (1..cols)
                .map(|c| {
                    let t = cell(grid, r, c);
                    t.trim().parse::<f64>().ok().filter(|v| v.is_finite() && !t.trim().starts_with('"')).map_or(Value::Null, |v| json!(v))
                })
                .collect()
        })
        .collect();
    json!({ "series": series, "categories": categories, "rows": values })
}

/// Text from a file or the clipboard as cells: tab-separated when any line holds a tab, else
/// comma-separated (decided once, so a label with a comma in a tab-separated file stays whole), at
/// most a graph's rows and columns.
pub(crate) fn grid_from_text(text: &str) -> Grid {
    let sep = if text.contains('\t') { '\t' } else { ',' };
    text.lines().take(MAX_ROWS).map(|l| l.trim_end_matches('\r').split(sep).take(MAX_COLS).map(|c| clean(c.trim())).collect()).collect()
}

/// A text file's bytes as text: UTF-16 with its byte-order mark (spreadsheets' "Unicode Text"),
/// else UTF-8 without one.
pub(crate) fn decode(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], le: bool| {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|p| if le { u16::from_le_bytes(*p) } else { u16::from_be_bytes(*p) }).collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// A cell's text: one line (tabs and line breaks become spaces), at most MAX_CELL characters.
fn clean(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '\t' | '\r' | '\n') { ' ' } else { c }).take(MAX_CELL).collect()
}

fn cell(grid: &Grid, r: usize, c: usize) -> String {
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
    // The button is off where files can't be read.
    let Some(read) = app.services.read.as_ref() else { return Ok(()) };
    let bytes = read(&path)?;
    replace_grid(d, grid_from_text(&decode(&bytes)));
    Ok(())
}

/// Draw the graph-data sheet: keyboard cell movement, block paste, import, transpose, switch-x/y
/// and revert actions, decimals and column width, and only the cells in view.
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
        if ui.add(egui::DragValue::new(&mut w).range(0..=MAX_WIDTH)).changed() {
            d.fields.insert("width".into(), json!(w));
        }
    });
    ui.add_space(6.0);
    let at = sel(d);
    let (rows, cols) = extent(d, at);
    let decimals = d.fields.get("decimals").and_then(Value::as_u64);
    let w = (d.fields.get("width").and_then(Value::as_u64).unwrap_or(7).min(MAX_WIDTH) as f32 * 7.0 + 10.0).max(12.0);
    let mut clicked = None;
    widgets::list_box(ui, |ui| {
        // Only the cells in view are drawn: a graph's sheet can be 10,001 × 257.
        egui::ScrollArea::both().id_salt("graph-data-sheet").max_height(280.0).auto_shrink([false, false]).show_viewport(ui, |ui, view| {
            let (cw, ch) = (w + 1.0, ROW_H);
            ui.set_min_size(egui::vec2(cols as f32 * cw, rows as f32 * ch));
            let origin = ui.max_rect().min;
            let span = |lo: f32, hi: f32, size: f32, n: usize| ((lo / size).floor().max(0.0) as usize, ((hi / size).ceil().max(0.0) as usize).min(n));
            let (r0, r1) = span(view.min.y, view.max.y, ch, rows);
            let (c0, c1) = span(view.min.x, view.max.x, cw, cols);
            for r in r0..r1 {
                for c in c0..c1 {
                    let rect = egui::Rect::from_min_size(origin + egui::vec2(c as f32 * cw, r as f32 * ch), egui::vec2(w, ch - 1.0));
                    let text = shown(cell_at(d, r, c), decimals);
                    let b = egui::Button::selectable((r, c) == at, egui::RichText::new(text).monospace()).truncate();
                    if ui.put(rect, b).clicked() && (r, c) != (0, 0) {
                        clicked = Some((r, c));
                    }
                }
            }
        });
    });
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
