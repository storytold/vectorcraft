use super::edit::{regenerate, spec_of, target};
use super::series::{switch_xy, transpose};
use super::*;

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

pub(super) fn parse_csv(csv: &str) -> (Vec<String>, Vec<String>, Vec<Vec<Option<f64>>>) {
    // Records end at line breaks outside quotes; a record is tab-separated if it has a tab outside quotes.
    let rows: Vec<Vec<Cell>> = split_unquoted(csv, '\n')
        .into_iter()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
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

/// Apply CSV graph data to the graph root `id` and rebuild its art (Variables data merge).
pub(crate) fn apply_csv(d: &mut Document, id: NodeId, csv: &str) -> Result<()> {
    let Some(node) = d.node(id) else {
        return Err(EngineError::NoNode(id));
    };
    let Some(mut spec) = node.extras().graph.as_deref().cloned() else {
        return Err(bad("graph.applyCsv", format!("node {} is not a graph", id.0)));
    };
    let (series, categories, rows) = parse_csv(csv);
    spec.series = series;
    spec.categories = categories;
    spec.set_cells(rows);
    regenerate(d, id, spec)
}

pub(super) fn apply_data(g: &mut GraphSpec, p: &Value) -> bool {
    let mut changed = false;
    if let Some(csv) = str_param(p, "csv") {
        let (s, c, r) = parse_csv(csv);
        g.series = s;
        g.categories = c;
        g.set_cells(r);
        changed = true;
    }
    let strings =
        |v: &Value| v.as_array().map(|a| a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>());
    if let Some(s) = p.get("series").and_then(strings) {
        g.series = s;
        changed = true;
    }
    if let Some(c) = p.get("categories").and_then(strings) {
        g.categories = c;
        changed = true;
    }
    if let Some(rows) = p.get("rows").and_then(Value::as_array) {
        // `null` (or anything that isn't a finite number) is a blank cell.
        g.set_cells(
            rows.iter().map(|r| r.as_array().map(|a| a.iter().map(|v| v.as_f64().filter(|v| v.is_finite())).collect()).unwrap_or_default()).collect(),
        );
        changed = true;
    }
    drop_empty_rows(g);
    changed
}

/// Drop rows with neither a value nor a label; a label-only row stays, its values blank.
fn drop_empty_rows(g: &mut GraphSpec) {
    let mut cells = g.cells();
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
        format!("{:.2}", v).trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

pub(super) fn set_data(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.setData";
    let id = target(s, p, C)?;
    let mut spec = spec_of(s, id)?;
    let flip = bool_or(p, "transpose", false);
    let swap = bool_or(p, "switchXY", false);
    if swap && spec.kind != GraphKind::Scatter {
        return Err(bad(C, "Switch x/y is for scatter graphs"));
    }
    // Transpose and Switch x/y act after any new data in the same call.
    let changed = apply_data(&mut spec, p);
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
    // Series assigned to the right axis that the new data no longer has are dropped, so a series added later starts
    // on the left.
    let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0);
    spec.right_series.retain(|i| *i < count);
    s.edit("Graph Data", |d, _| regenerate(d, id, spec))?;
    Ok(json!({ "id": id.0 }))
}
