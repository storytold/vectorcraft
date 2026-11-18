use super::edit::{regenerate, spec_of, target};
use super::series::{switch_xy, transpose};
use super::{Document, EngineError, GraphKind, GraphSpec, NodeId, Result, Session, Value, bad, json, switch_or, text_param};

// ---------- data ----------

/// One CSV cell: its text, and whether it was in straight quotes (a quoted number is a label, not a value).
struct Cell {
    text: String,
    quoted: bool,
}

impl Cell {
    fn number(&self) -> Option<f64> {
        if self.quoted { None } else { self.text.parse::<f64>().ok().filter(|v| v.is_finite()) }
    }
}

/// Split `s` at each `sep` outside straight quotes (a `""` inside quotes toggles twice, so stays inside).
fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut parts = vec![];
    let mut quoted = false;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch == sep && !quoted {
            parts.push(s.get(start..i).unwrap_or_default());
            start = i + ch.len_utf8();
        }
    }
    parts.push(s.get(start..).unwrap_or_default());
    parts
}

/// One CSV cell: a cell wholly in quotes keeps its inner text (`""` = `"`), others are trimmed.
fn cell(c: &str) -> Cell {
    let c = c.trim();
    match c.strip_prefix('"').and_then(|c| c.strip_suffix('"')) {
        Some(inner) => Cell { text: inner.replace("\"\"", "\""), quoted: true },
        None => Cell { text: c.trim_matches('"').to_string(), quoted: false },
    }
}

pub(crate) fn parse_csv(csv: &str) -> (Vec<String>, Vec<String>, Vec<Vec<Option<f64>>>) {
    let csv = csv.strip_prefix('\u{feff}').unwrap_or(csv);
    // Records end at line breaks outside quotes; a record is tab-separated if it has a tab outside quotes.
    let rows: Vec<Vec<Cell>> = split_unquoted(csv, '\n')
        .into_iter()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            // A lone `""` line is a single blank value (`to_csv` writes it so the row isn't dropped as empty).
            if l.trim() == "\"\"" {
                return vec![Cell { text: String::new(), quoted: false }];
            }
            let tabs = split_unquoted(l, '\t');
            let parts = if tabs.len() > 1 { tabs } else { split_unquoted(l, ',') };
            parts.into_iter().map(cell).collect()
        })
        .collect();
    let Some(first) = rows.first() else { return (vec![], vec![], vec![]) };
    // A label is quoted or doesn't read as a number; an empty unquoted cell is a blank value.
    let label = |c: &Cell| c.quoted || (c.number().is_none() && !c.text.is_empty());
    // A header row has an empty first cell or a label after it, or is a label followed only by empty cells (`Year,,`).
    // All-empty series names mean no legend.
    let empty = |c: &Cell| c.text.is_empty() && !c.quoted;
    let header = first.first().is_some_and(empty)
        || first.iter().skip(1).any(label)
        || (first.len() > 1 && first.first().is_some_and(label) && first.iter().skip(1).all(empty));
    let mut series: Vec<String> = if header { first.iter().skip(1).map(|c| c.text.clone()).collect() } else { vec![] };
    if series.iter().all(String::is_empty) {
        series.clear();
    }
    let data = rows.get(usize::from(header)..).unwrap_or_default();
    // The first column holds the category labels if any row starts with one; then it does for every row, so a
    // row with an empty label doesn't shift its values one place left.
    let labelled = data.iter().any(|r| r.first().is_some_and(label));
    let mut categories = vec![];
    let mut values = vec![];
    for r in data {
        if labelled {
            categories.push(r.first().map(|c| c.text.clone()).unwrap_or_default());
        }
        values.push(r.iter().skip(labelled as usize).map(Cell::number).collect());
    }
    (series, categories, values)
}

pub(super) fn to_csv(g: &GraphSpec) -> String {
    g.to_csv()
}

// ---------- graph data files ----------

/// The largest graph data file that Graph Data's Import Data and Import Data Sets' `%` columns
/// read, 1 MB (1,048,576 bytes).
pub const MAX_GRAPH_DATA_FILE: usize = 1 << 20;

/// A graph data sheet's size: a row of labels over a graph's categories, a column of labels
/// before its series; a cell's longest text.
const SHEET_ROWS: usize = vectorcraft_doc::MAX_GRAPH_CATEGORIES + 1;
const SHEET_COLS: usize = vectorcraft_doc::MAX_GRAPH_SERIES + 1;
const SHEET_CELL: usize = 256;

/// A graph data file's bytes as text: UTF-16 with its byte-order mark (spreadsheets' "Unicode
/// Text"), else UTF-16 without one when the bytes hold NUL bytes and Import Data Sets' rule gives
/// them a byte order ([`crate::cmd::utf16_order_without_mark`]), else UTF-8 with or without one.
/// UTF-16 is read as it is after a byte-order mark: an odd last byte is left out, and an unpaired
/// surrogate reads as U+FFFD.
pub fn graph_data_text(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], le: bool| {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|p| if le { u16::from_le_bytes(*p) } else { u16::from_be_bytes(*p) }).collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => match bytes.contains(&0).then(|| crate::cmd::utf16_order_without_mark(bytes)).flatten() {
            Some(le) => utf16(bytes, le),
            None => String::from_utf8_lossy(bytes).into_owned(),
        },
    }
}

/// A graph data sheet cell's text: one line (tabs and line breaks become spaces), at most 256
/// characters.
pub fn graph_cell_text(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '\t' | '\r' | '\n') { ' ' } else { c }).take(SHEET_CELL).collect()
}

/// Text from a file or the clipboard as graph data sheet cells: tab-separated when any line holds
/// a tab, else comma-separated (decided once, so a label with a comma in a tab-separated file
/// stays whole), at most a graph's rows and columns.
pub fn graph_grid_from_text(text: &str) -> Vec<Vec<String>> {
    let sep = if text.contains('\t') { '\t' } else { ',' };
    text.lines().take(SHEET_ROWS).map(|l| l.trim_end_matches('\r').split(sep).take(SHEET_COLS).map(|c| graph_cell_text(c.trim())).collect()).collect()
}

/// A graph data sheet as `graph.setData` takes it: the first row holds the series labels, the
/// first column the category labels (none when all are blank), and the other cells the values (a
/// blank cell, or text that isn't a number, has none), without the empty rows and columns past
/// the data.
pub fn graph_data_from_grid(grid: &[Vec<String>]) -> Value {
    let cell = |r: usize, c: usize| grid.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default();
    let filled = |s: &String| !s.trim().is_empty();
    // At most a graph's rows and columns, as a file gives (`graph_grid_from_text`): a grid sent by
    // automation is any size, and the values are their product.
    let grid = &grid[..grid.len().min(SHEET_ROWS)];
    let rows = grid.iter().rposition(|r| r.iter().take(SHEET_COLS).any(filled)).map_or(1, |i| i + 1).max(1);
    let cols = grid.iter().take(rows).filter_map(|r| r.iter().take(SHEET_COLS).rposition(filled)).max().map_or(1, |i| i + 1).max(1);
    let label = |s: &str| {
        let s = s.trim();
        // A label typed in straight quotes (as the CSV form takes a number as a label) is the text inside.
        s.strip_prefix('"').and_then(|t| t.strip_suffix('"')).unwrap_or(s).to_string()
    };
    // Series labels up to the last one given (none when all are blank).
    let mut series: Vec<String> = (1..cols).map(|c| label(&cell(0, c))).collect();
    while series.last().is_some_and(String::is_empty) {
        series.pop();
    }
    let mut categories: Vec<String> = (1..rows).map(|r| label(&cell(r, 0))).collect();
    if categories.iter().all(String::is_empty) {
        categories.clear();
    }
    let values: Vec<Vec<Value>> = (1..rows)
        .map(|r| {
            (1..cols)
                .map(|c| {
                    let t = cell(r, c);
                    t.trim().parse::<f64>().ok().filter(|v| v.is_finite() && !t.trim().starts_with('"')).map_or(Value::Null, |v| json!(v))
                })
                .collect()
        })
        .collect();
    json!({ "series": series, "categories": categories, "rows": values })
}

/// The graph value (the CSV form [`apply_csv`] reads) of a graph data file's bytes, read as Graph
/// Data's Import Data reads the file into its sheet and Apply sends the sheet: so applying the
/// value gives a graph the data that Import Data and Apply give it.
pub(crate) fn graph_csv_from_file(bytes: &[u8]) -> Result<String> {
    let data = graph_data_from_grid(&graph_grid_from_text(&graph_data_text(bytes)));
    let mut spec = GraphSpec::default();
    apply_data(&mut spec, &data, "dataset.import")?;
    Ok(spec.to_csv())
}

/// Apply CSV graph data to the graph root `id` and rebuild its art (Variables data merge), as `graph.setData` with
/// `csv` does. Data the graph already holds leaves the graph as it is.
pub(crate) fn apply_csv(d: &mut Document, id: NodeId, csv: &str) -> Result<()> {
    let Some(node) = d.node(id) else {
        return Err(EngineError::NoNode(id));
    };
    let Some(current) = node.extras().graph.as_deref() else {
        return Err(bad("graph.applyCsv", format!("node {} is not a graph", id.0)));
    };
    let mut spec = current.clone();
    apply_data(&mut spec, &json!({ "csv": csv }), "graph.applyCsv")?;
    keep_right_series(&mut spec);
    if spec == *current {
        return Ok(());
    }
    regenerate(d, id, spec)
}

/// Drop right-axis series the data no longer has, so a series added later starts on the left.
fn keep_right_series(g: &mut GraphSpec) {
    let count = g.rows.iter().map(Vec::len).max().unwrap_or(0);
    g.right_series.retain(|i| *i < count);
}

/// Apply the data `p` gives (`csv`, `series`, `categories`, `rows`) to `g`; true when it gave any. A null is read as a
/// key left out. A `csv` that is not a string, a `series` or `categories` that is not a list, and `rows` that are not
/// lists of numbers and nulls are errors, and `g` is left as it was.
pub(super) fn apply_data(g: &mut GraphSpec, p: &Value, cmd: &str) -> Result<bool> {
    let list = |key: &str| -> Result<Option<&Vec<Value>>> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(a)) => Ok(Some(a)),
            Some(v) => Err(bad(cmd, format!("`{key}` must be a list, not {v}"))),
        }
    };
    let csv = text_param(p, "csv", cmd)?;
    let strings = |a: &Vec<Value>| a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>();
    let series = list("series")?.map(strings);
    let categories = list("categories")?.map(strings);
    // A null is a blank cell.
    let rows = list("rows")?
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    let bad_row = || bad(cmd, format!("`rows` must be lists of numbers or nulls, not {r}"));
                    r.as_array()
                        .ok_or_else(bad_row)?
                        .iter()
                        .map(|v| if v.is_null() { Ok(None) } else { v.as_f64().map(|v| Some(v).filter(|v| v.is_finite())).ok_or_else(bad_row) })
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?;
    let mut changed = false;
    if let Some(csv) = csv {
        let (s, c, r) = parse_csv(csv);
        g.series = s;
        // Series labels up to the last one given, as Graph Data's sheet keeps them
        // ([`graph_data_from_grid`]): `to_csv` writes a blank label for each series without one.
        while g.series.last().is_some_and(String::is_empty) {
            g.series.pop();
        }
        g.categories = c;
        g.set_cells(r);
        changed = true;
    }
    if let Some(s) = series {
        g.series = s;
        changed = true;
    }
    if let Some(c) = categories {
        g.categories = c;
        changed = true;
    }
    if let Some(rows) = rows {
        g.set_cells(rows);
        changed = true;
    }
    drop_empty_rows(g);
    Ok(changed)
}

/// Drop rows with neither a value nor a label; a label-only row stays, its values blank. Each kept row keeps its own
/// width, so a dropped row wider than the rest adds no blank series.
fn drop_empty_rows(g: &mut GraphSpec) {
    let blank: std::collections::HashSet<[usize; 2]> = g.blanks.iter().copied().collect();
    let mut cells: Vec<Vec<Option<f64>>> = g
        .rows
        .iter()
        .take(vectorcraft_doc::MAX_GRAPH_CATEGORIES)
        .enumerate()
        .map(|(c, r)| r.iter().take(vectorcraft_doc::MAX_GRAPH_SERIES).enumerate().map(|(s, v)| (!blank.contains(&[c, s])).then_some(*v)).collect())
        .collect();
    let keep: Vec<bool> = (0..cells.len())
        .map(|c| cells.get(c).is_some_and(|r| r.iter().any(Option::is_some)) || g.categories.get(c).is_some_and(|l| !l.is_empty()))
        .collect();
    if keep.contains(&false) || cells.len() < g.rows.len() {
        let mut k = keep.iter();
        cells.retain(|_| k.next().copied().unwrap_or(false));
        let mut k = keep.iter();
        g.categories.retain(|_| k.next().copied().unwrap_or(true));
        g.set_cells(cells);
    }
}

pub(super) fn fmt_value(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        // Beyond the i64 range the cast would saturate; every f64 that large is a whole number, so
        // `{:.0}` prints its exact digits.
        if v.abs() < 9e18 { format!("{}", v.round() as i64) } else { format!("{v:.0}") }
    } else {
        let s = format!("{:.2}", v).trim_end_matches('0').trim_end_matches('.').to_string();
        // A tiny negative rounds to "-0.00"; show it as plain zero like -0.0.
        if s == "-0" { "0".to_string() } else { s }
    }
}

pub(super) fn set_data(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.setData";
    let id = target(s, p, C)?;
    let mut spec = spec_of(s, id)?;
    let flip = switch_or(p, "transpose", false, C)?;
    let swap = switch_or(p, "switchXY", false, C)?;
    if swap && spec.kind != GraphKind::Scatter {
        return Err(bad(C, "Switch x/y is for scatter graphs"));
    }
    // Transpose and Switch x/y act after any new data in the same call.
    let changed = apply_data(&mut spec, p, C)?;
    if flip {
        // The categories become series: at most as many as a graph has series.
        if spec.cells().len() > vectorcraft_doc::MAX_GRAPH_SERIES {
            return Err(bad(
                C,
                format!("a graph has at most {} series, so it transposes with at most that many categories", vectorcraft_doc::MAX_GRAPH_SERIES),
            ));
        }
        transpose(&mut spec);
        drop_empty_rows(&mut spec);
    }
    if swap {
        switch_xy(&mut spec);
    }
    if !(changed || flip || swap) {
        // Blank cells come back as null, so the rows can be edited and sent back as they are.
        return Ok(json!({ "csv": to_csv(&spec), "series": spec.series, "categories": spec.categories, "rows": spec.cells() }));
    }
    keep_right_series(&mut spec);
    s.edit("Graph Data", |d, _| regenerate(d, id, spec))?;
    Ok(json!({ "id": id.0 }))
}
