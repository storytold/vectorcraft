use super::{
    BTreeMap, DataSet, DataValue, EngineError, MAX_NAME, Result, Session, Value, Variable, VariableKind, Variables, absolute_path, bad, json,
    read_file_head, str_param, variables_unlocked, write_file,
};

/// Cap on a graph variable's CSV (MCP and files are untrusted).
pub(super) const MAX_GRAPH_CSV: usize = 256 * 1024;

/// Caps on a datasets CSV (paths and MCP payloads are untrusted).
const MAX_CSV_BYTES: usize = 1_048_576;

const MAX_CSV_ROWS: usize = 1_000;

const MAX_CSV_COLS: usize = 256;

/// Bytes of a datasets CSV from `path`, `data`, `dataBase64` or `csv`.
fn datasets_csv_bytes(p: &Value, cmd: &str) -> Result<Vec<u8>> {
    let bytes = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64"), str_param(p, "csv")) {
        // One byte past the cap is enough to refuse the file below.
        (Some(path), ..) => read_file_head(&absolute_path(path), MAX_CSV_BYTES + 1)?,
        (None, Some(text), ..) => text.as_bytes().to_vec(),
        (None, None, Some(b64), _) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "bad dataBase64"))?,
        (None, None, None, Some(csv)) => csv.as_bytes().to_vec(),
        _ => return Err(bad(cmd, "give `path`, `data`, `dataBase64` or `csv`")),
    };
    if bytes.len() > MAX_CSV_BYTES {
        return Err(bad(cmd, format!("CSV is larger than {MAX_CSV_BYTES} bytes")));
    }
    Ok(bytes)
}

/// A row of a datasets CSV: the file line it starts on, its cells, and for each cell whether it was quoted.
type CsvRow = (usize, Vec<String>, Vec<bool>);

/// Whether a datasets CSV separates its cells with tabs: its header row (the first line that
/// isn't blank), read with tabs between cells, has a tab outside quotes and no comma outside
/// quotes.
fn tab_separated(text: &str) -> bool {
    // `start`: nothing but `\r` read yet in this cell. `blank`: no cell on this line holds
    // anything but white space yet.
    let (mut in_quotes, mut start, mut blank, mut tab) = (false, true, true, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    blank = false;
                } else {
                    in_quotes = false;
                }
            }
            '"' if start => in_quotes = true,
            c if in_quotes => blank &= c.is_whitespace(),
            ',' => return false,
            '\t' => {
                tab = true;
                start = true;
                continue;
            }
            '\r' => continue,
            '\n' if blank => {
                tab = false;
                start = true;
                continue;
            }
            '\n' => return tab,
            c => blank &= c.is_whitespace(),
        }
        start = false;
    }
    tab
}

/// Parse a datasets CSV into rows of cells (no header interpretation yet). Cells are separated
/// by commas, or by tabs when [`tab_separated`] says so. `"` quotes a cell (doubled inside),
/// which may then hold separators and line breaks, as [`csv_escape`] writes them. An unquoted
/// cell is trimmed. A quoted cell keeps what its quotes hold as written, spaces included, and
/// drops white space after the closing quote at the end of the cell. A line whose cells are all
/// blank and unquoted is skipped; a quoted empty cell keeps its line as a row.
fn parse_datasets_csv(text: &str, cmd: &str) -> Result<Vec<CsvRow>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let sep = if tab_separated(text) { '\t' } else { ',' };
    let (mut rows, mut row, mut quoted, mut cur) = (Vec::new(), Vec::<String>::new(), Vec::<bool>::new(), String::new());
    // `closed`: where the closing quote of a quoted cell falls in `cur`. `start`: the line the
    // current row starts on.
    let (mut in_quotes, mut closed, mut line, mut start) = (false, None, 1usize, 1usize);
    let mut chars = text.chars().peekable();
    loop {
        let c = chars.next();
        match c {
            Some('"') if in_quotes => {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                    closed = Some(cur.len());
                }
            }
            Some('"') if cur.is_empty() => in_quotes = true,
            Some(ch) if ch == sep && !in_quotes => {
                quoted.push(closed.is_some());
                row.push(end_csv_cell(&mut cur, &mut closed));
            }
            Some('\r') if !in_quotes => {}
            Some('\n') | None if !in_quotes => {
                quoted.push(closed.is_some());
                row.push(end_csv_cell(&mut cur, &mut closed));
                if row.iter().any(|c| !c.trim().is_empty()) || quoted.contains(&true) {
                    rows.push((start, std::mem::take(&mut row), std::mem::take(&mut quoted)));
                    // +1 for the header.
                    if rows.len() > MAX_CSV_ROWS + 1 {
                        return Err(bad(cmd, format!("more than {MAX_CSV_ROWS} data rows")));
                    }
                }
                row.clear();
                quoted.clear();
                if c.is_none() {
                    return Ok(rows);
                }
                line += 1;
                start = line;
            }
            None => return Err(bad(cmd, format!("line {line}: unclosed quote"))),
            Some(c) => {
                line += usize::from(c == '\n');
                cur.push(c);
            }
        }
        if row.len() > MAX_CSV_COLS {
            return Err(bad(cmd, format!("line {line}: more than {MAX_CSV_COLS} columns")));
        }
    }
}

/// Take the cell read so far out of `cur`. Without a closing quote (`closed` is `None`) the cell
/// is trimmed; with one, the text before it is kept as written and the text after it is
/// trimmed at the end.
fn end_csv_cell(cur: &mut String, closed: &mut Option<usize>) -> String {
    let cell = std::mem::take(cur);
    match closed.take() {
        Some(at) => match cell.split_at_checked(at) {
            Some((quoted, after)) => format!("{quoted}{}", after.trim_end()),
            None => cell,
        },
        None => cell.trim().to_string(),
    }
}

fn csv_escape(s: &str) -> String {
    // An empty string is quoted so that it reads back as an empty value, not as a missing one. A
    // tab is quoted so that a header of one variable whose name holds a tab isn't read as
    // tab-separated.
    if s.is_empty() || s.contains([',', '"', '\n', '\r', '\t']) || s != s.trim() { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// A visibility cell: true/false, 1/0, yes/no (ASCII, case-insensitive).
fn parse_visible_cell(s: &str, var: &str, cmd: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(bad(cmd, format!("value for `{var}` must be true or false"))),
    }
}

/// The value of a cell. A quoted empty text cell is an empty text value; any other empty cell is no value.
fn cell_to_value(kind: VariableKind, cell: &str, quoted: bool, var: &str, cmd: &str) -> Result<Option<DataValue>> {
    // A text cell is not trimmed here (the parser trims unquoted cells); image, graph and
    // visibility cells are trimmed.
    let t = if kind == VariableKind::Text { cell } else { cell.trim() };
    if t.is_empty() {
        return Ok((quoted && kind == VariableKind::Text).then(|| DataValue::Text(String::new())));
    }
    Ok(Some(match kind {
        VariableKind::Text => DataValue::Text(t.to_string()),
        VariableKind::Image => DataValue::Image(t.to_string()),
        VariableKind::Graph if t.len() <= MAX_GRAPH_CSV => DataValue::Graph(t.to_string()),
        VariableKind::Graph => return Err(bad(cmd, format!("value for `{var}` is longer than {MAX_GRAPH_CSV} bytes"))),
        VariableKind::Visibility => DataValue::Visible(parse_visible_cell(t, var, cmd)?),
    }))
}

fn value_to_cell(v: &DataValue) -> String {
    match v {
        DataValue::Text(t) | DataValue::Image(t) | DataValue::Graph(t) => t.clone(),
        DataValue::Visible(b) => if *b { "true" } else { "false" }.to_string(),
    }
}

/// Build the CSV of every dataset: a header of the variables in definition order, then one row per
/// dataset. The dataset names are not written. A dataset with no value for any variable would be a
/// blank line, which the import skips, so its row gets a quoted empty cell: in the first column whose
/// variable isn't a text variable, where it reads back as no value, or else in the first column,
/// where it reads back as an empty text.
fn datasets_to_csv(vars: &Variables) -> String {
    let header: Vec<String> = vars.variables.iter().map(|v| csv_escape(&v.name)).collect();
    let mut out = header.join(",");
    out.push('\n');
    let keep = vars.variables.iter().position(|v| v.kind != VariableKind::Text).unwrap_or(0);
    for ds in &vars.datasets {
        let mut row: Vec<String> =
            vars.variables.iter().map(|v| ds.values.get(&v.name).map(|val| csv_escape(&value_to_cell(val))).unwrap_or_default()).collect();
        if row.iter().all(String::is_empty)
            && let Some(cell) = row.get_mut(keep)
        {
            *cell = csv_escape("");
        }
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// The variable that a datasets CSV header cell (trimmed, not empty) in 1-based `column` names,
/// and its kind. A cell that is the name of a variable in `kinds` or in `defined` (the variables
/// that earlier columns define) names that variable. Otherwise `#` before the name makes it a
/// visibility variable, `@` makes it an image variable, and a cell without a prefix names a text
/// variable. A prefixed name that is already a variable must be one of the prefix's kind; a new
/// name is added to `defined`. A `%` cell that names no variable (graph data files) is refused.
fn header_variable(
    cell: &str,
    column: usize,
    kinds: &BTreeMap<String, VariableKind>,
    defined: &mut Vec<Variable>,
    cmd: &str,
) -> Result<(String, VariableKind)> {
    let known = |name: &str, defined: &[Variable]| kinds.get(name).copied().or_else(|| defined.iter().find(|v| v.name == name).map(|v| v.kind));
    if let Some(kind) = known(cell, defined) {
        return Ok((cell.to_string(), kind));
    }
    let (name, kind) = if let Some(rest) = cell.strip_prefix('#') {
        (rest.trim(), VariableKind::Visibility)
    } else if let Some(rest) = cell.strip_prefix('@') {
        (rest.trim(), VariableKind::Image)
    } else if cell.starts_with('%') {
        return Err(bad(cmd, format!("column {column}: `{cell}` is a `%` column of graph data files, which are not read")));
    } else {
        (cell, VariableKind::Text)
    };
    if name.is_empty() {
        return Err(bad(cmd, format!("column {column}: `{cell}` names no variable")));
    }
    match known(name, defined) {
        Some(k) if k == kind => Ok((name.to_string(), kind)),
        Some(k) => Err(bad(cmd, format!("column {column}: `{cell}` names `{name}`, which is a {} variable", k.label()))),
        None if name.chars().count() > MAX_NAME => Err(bad(cmd, format!("column {column}: the variable name is longer than {MAX_NAME} characters"))),
        None => {
            defined.push(Variable { name: name.to_string(), kind });
            Ok((name.to_string(), kind))
        }
    }
}

/// Import datasets from CSV. Every header cell names a variable, as [`header_variable`] reads it,
/// and the variables the document doesn't define yet are defined, unbound. The rows replace every
/// dataset of the document and are named "Data Set 1", "Data Set 2" and so on, in file order. The
/// document's variables and their bindings stay. One undo step.
pub(super) fn dataset_import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.import";
    let bytes = datasets_csv_bytes(p, C)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| bad(C, "CSV is not UTF-8"))?;
    let rows = parse_datasets_csv(text, C)?;
    let Some((_, header, _)) = rows.first() else {
        return Err(bad(C, "CSV is empty"));
    };
    let kinds: BTreeMap<String, VariableKind> = s.doc()?.doc.variables.variables.iter().map(|v| (v.name.clone(), v.kind)).collect();
    let mut columns: Vec<(usize, String, VariableKind)> = Vec::new();
    let mut defined: Vec<Variable> = Vec::new();
    for (i, h) in header.iter().enumerate() {
        let cell = h.trim();
        if cell.is_empty() {
            return Err(bad(C, format!("empty header in column {}", i + 1)));
        }
        let (name, kind) = header_variable(cell, i + 1, &kinds, &mut defined, C)?;
        columns.push((i, name, kind));
    }
    // Lock Variables refuses New Variable, so the import is refused when a header would define one.
    if !defined.is_empty() {
        variables_unlocked(s).map_err(|why| EngineError::Disabled(C.into(), why))?;
    }
    if rows.len() == 1 {
        return Err(bad(C, "CSV has a header but no data rows"));
    }
    let mut parsed: Vec<DataSet> = Vec::new();
    for (n, &(line, ref row, ref quoted)) in rows.iter().skip(1).enumerate() {
        let name = format!("Data Set {}", n + 1);
        let mut values = BTreeMap::new();
        for &(col, ref var, kind) in &columns {
            let cell = row.get(col).map(String::as_str).unwrap_or("");
            let was_quoted = quoted.get(col).copied().unwrap_or(false);
            match cell_to_value(kind, cell, was_quoted, var, C) {
                Ok(Some(v)) => {
                    values.insert(var.clone(), v);
                }
                Ok(None) => {}
                Err(_) => return Err(bad(C, format!("line {line}: value for `{var}` must match its variable kind"))),
            }
        }
        parsed.push(DataSet { name, values });
    }
    let names: Vec<String> = parsed.iter().map(|d| d.name.clone()).collect();
    let new_variables: Vec<String> = defined.iter().map(|v| v.name.clone()).collect();
    let deleted: Vec<String> = s.doc()?.doc.variables.datasets.iter().map(|d| d.name.clone()).collect();
    s.edit("Import Data Sets", |d, _| {
        d.variables.variables.extend(defined);
        d.variables.datasets = parsed;
        d.variables.active_dataset = None;
        Ok(())
    })?;
    Ok(json!({"imported": names.len(), "names": names, "defined": new_variables, "deletedDataSets": deleted}))
}

/// Export every dataset as CSV (one column per variable, no dataset names). With `path`, write the
/// file; else return `{csv}`.
pub(super) fn dataset_export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.export";
    let st = s.doc()?;
    if st.doc.variables.variables.is_empty() {
        return Err(bad(C, "define a variable first"));
    }
    let csv = datasets_to_csv(&st.doc.variables);
    if csv.len() > MAX_CSV_BYTES {
        return Err(bad(C, format!("CSV would be larger than {MAX_CSV_BYTES} bytes")));
    }
    match str_param(p, "path") {
        Some(path) => {
            let path = absolute_path(path);
            write_file(&path, csv.as_bytes())?;
            Ok(json!({"path": path, "datasets": st.doc.variables.datasets.len()}))
        }
        None => Ok(json!({"csv": csv, "datasets": st.doc.variables.datasets.len()})),
    }
}
