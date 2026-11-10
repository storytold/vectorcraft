use super::{
    BTreeMap, BTreeSet, DataSet, DataValue, EngineError, MAX_NAME, Result, Session, Value, Variable, VariableKind, Variables, absolute_path, bad,
    json, read_file_head, str_param, variables_unlocked, write_file,
};
use crate::cmd::place::text::windows_1252;

/// Cap on a graph variable's CSV (MCP and files are untrusted).
pub(super) const MAX_GRAPH_CSV: usize = 256 * 1024;

/// Caps on a datasets CSV (paths and MCP payloads are untrusted).
const MAX_CSV_BYTES: usize = 1_048_576;

const MAX_CSV_ROWS: usize = 1_000;

const MAX_CSV_COLS: usize = 256;

/// Bytes of a datasets CSV from `path`, `data`, `dataBase64` or `csv`, and whether they came
/// as text (`data` or `csv`).
fn datasets_csv_bytes(p: &Value, cmd: &str) -> Result<(Vec<u8>, bool)> {
    let (bytes, text) = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64"), str_param(p, "csv")) {
        // One byte past the cap is enough to refuse the file below.
        (Some(path), ..) => (read_file_head(&absolute_path(path), MAX_CSV_BYTES + 1)?, false),
        (None, Some(text), ..) => (text.as_bytes().to_vec(), true),
        (None, None, Some(b64), _) => (vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "bad dataBase64"))?, false),
        (None, None, None, Some(csv)) => (csv.as_bytes().to_vec(), true),
        _ => return Err(bad(cmd, "give `path`, `data`, `dataBase64` or `csv`")),
    };
    if bytes.len() > MAX_CSV_BYTES {
        return Err(bad(cmd, format!("CSV is larger than {MAX_CSV_BYTES} bytes")));
    }
    Ok((bytes, text))
}

/// The text of a datasets CSV given as a file or as `dataBase64`: UTF-16 after a UTF-16
/// byte-order mark, else UTF-8, with or without a byte-order mark, else Windows-1252. A file
/// that starts with a byte-order mark must be valid in its encoding, and a file without one must
/// not hold NUL bytes (UTF-16 without a byte-order mark).
fn datasets_csv_text(bytes: &[u8], cmd: &str) -> Result<String> {
    let utf16 = |rest: &[u8], le: bool| {
        let (units, odd) = rest.as_chunks::<2>();
        let units = units.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) });
        let text = char::decode_utf16(units).collect::<std::result::Result<String, _>>();
        text.ok().filter(|_| odd.is_empty()).ok_or_else(|| bad(cmd, "CSV is not UTF-16"))
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => return utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => return utf16(rest, false),
        _ if bytes.contains(&0) => return Err(bad(cmd, "CSV holds NUL bytes: save it as UTF-8, or as UTF-16 with a byte-order mark")),
        _ => {}
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(_) if bytes.starts_with(b"\xEF\xBB\xBF") => Err(bad(cmd, "CSV is not UTF-8")),
        Err(_) => Ok(bytes.iter().map(|&b| windows_1252(b)).collect()),
    }
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
                if row.len() > MAX_CSV_COLS {
                    return Err(bad(cmd, format!("line {line}: more than {MAX_CSV_COLS} columns")));
                }
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

/// A visibility cell: true/false, 1/0, yes/no (ASCII, case-insensitive). An error names the
/// file line `line`.
fn parse_visible_cell(s: &str, var: &str, line: usize, cmd: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(bad(cmd, format!("line {line}: value for `{var}` must be true or false"))),
    }
}

/// The value of a cell on file line `line`. A quoted empty text cell is an empty text value; any
/// other empty cell is no value. An error names the line and the reason.
fn cell_to_value(kind: VariableKind, cell: &str, quoted: bool, var: &str, line: usize, cmd: &str) -> Result<Option<DataValue>> {
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
        VariableKind::Graph => return Err(bad(cmd, format!("line {line}: value for `{var}` is longer than {MAX_GRAPH_CSV} bytes"))),
        VariableKind::Visibility => DataValue::Visible(parse_visible_cell(t, var, line, cmd)?),
        VariableKind::Unknown => return Err(bad(cmd, format!("line {line}: value for `{var}` must match its variable kind"))),
    }))
}

fn value_to_cell(v: &DataValue) -> String {
    match v {
        DataValue::Text(t) | DataValue::Image(t) | DataValue::Graph(t) => t.clone(),
        DataValue::Visible(b) => if *b { "true" } else { "false" }.to_string(),
    }
}

/// The header cell that [`datasets_to_csv`] writes for `v`, as [`header_variable`] reads it: `#`
/// before a visibility variable's name, `@` before an image variable's name, and the name alone
/// for a text or graph variable.
fn export_header(v: &Variable) -> String {
    match v.kind {
        VariableKind::Visibility => format!("#{}", v.name),
        VariableKind::Image => format!("@{}", v.name),
        VariableKind::Text | VariableKind::Graph | VariableKind::Unknown => v.name.clone(),
    }
}

/// What [`dataset_import`] does with each variable's column of the CSV that [`datasets_to_csv`]
/// writes, in a document without the variables: the first list names the variables whose header
/// or values it refuses, so it doesn't import the file (a `%` header, a `#` header whose values
/// aren't true or false, a header that names a variable an earlier column named), and the second
/// the variables it reads as another variable (a text variable whose name starts with `#` or `@`,
/// and a graph variable, which no header cell defines).
fn kinds_a_new_document_loses(vars: &Variables) -> (Vec<&str>, Vec<&str>) {
    const C: &str = "dataset.import";
    let none = BTreeMap::new();
    let mut defined = Vec::new();
    let (mut refused, mut other) = (Vec::new(), Vec::new());
    let mut seen = BTreeSet::new();
    for (i, v) in vars.variables.iter().enumerate() {
        let header = export_header(v);
        let cell = header.trim();
        let read = if cell.is_empty() { None } else { header_variable(cell, i + 1, &none, &mut defined, C).ok() };
        // The import refuses a header that names a variable an earlier column named.
        let Some((name, kind)) = read.filter(|(name, _)| seen.insert(name.clone())) else {
            refused.push(v.name.as_str());
            continue;
        };
        let cells_read = vars.datasets.iter().filter_map(|d| d.values.get(&v.name)).all(|val| {
            let cell = value_to_cell(val);
            cell_to_value(kind, &cell, cell.is_empty(), &name, 2, C).is_ok()
        });
        if !cells_read {
            refused.push(v.name.as_str());
        } else if name != v.name || kind != v.kind {
            other.push(v.name.as_str());
        }
    }
    (refused, other)
}

/// Build the CSV of every dataset: a header of each variable's [`export_header`] in definition
/// order, then one row per dataset. The dataset names are not written. Refused when a variable's
/// header is another variable's name, which the import reads as that variable (a visibility
/// variable `Show` next to a text variable `#Show`). A dataset with no value for any variable would
/// be a blank line, which the import skips, so its row gets a quoted empty cell: in the first column
/// whose variable isn't a text variable, where it reads back as no value, or else in the first
/// column, where it reads back as an empty text.
fn datasets_to_csv(vars: &Variables, cmd: &str) -> Result<String> {
    let names: BTreeSet<&str> = vars.variables.iter().map(|v| v.name.as_str()).collect();
    let mut header = Vec::with_capacity(vars.variables.len());
    for v in &vars.variables {
        let cell = export_header(v);
        if cell != v.name && names.contains(cell.as_str()) {
            let (name, other) = (&v.name, &cell);
            return Err(bad(cmd, format!("the header `{cell}` for `{name}` would read back as the variable `{other}`: rename one of them")));
        }
        header.push(csv_escape(&cell));
    }
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
    Ok(out)
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
/// and the variables the document doesn't define yet are defined, unbound. Two headers that name
/// one variable are refused. The rows replace every dataset of the document and are named "Data
/// Set 1", "Data Set 2" and so on, in file order. The document's variables and their bindings
/// stay. One undo step, and none when the variables and datasets stay as they were.
pub(super) fn dataset_import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.import";
    let (bytes, given_as_text) = datasets_csv_bytes(p, C)?;
    // `data` and `csv` are already text, NUL characters included (`dataset.export` writes them).
    let text = if given_as_text { String::from_utf8(bytes).map_err(|_| bad(C, "CSV is not UTF-8"))? } else { datasets_csv_text(&bytes, C)? };
    let rows = parse_datasets_csv(&text, C)?;
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
        if let Some(&(first, ..)) = columns.iter().find(|(_, n, _)| *n == name) {
            let (first, column) = (first + 1, i + 1);
            return Err(bad(C, format!("columns {first} and {column} both name the variable `{name}`")));
        }
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
            if let Some(v) = cell_to_value(kind, cell, was_quoted, var, line, C)? {
                values.insert(var.clone(), v);
            }
        }
        parsed.push(DataSet { name, values });
    }
    let names: Vec<String> = parsed.iter().map(|d| d.name.clone()).collect();
    let new_variables: Vec<String> = defined.iter().map(|v| v.name.clone()).collect();
    let deleted: Vec<String> = s.doc()?.doc.variables.datasets.iter().map(|d| d.name.clone()).collect();
    // The import is one undo step, and an import that leaves the variables and data sets as they
    // were adds none, so the active data set stays when nothing changes.
    super::edit_variables(s, "Import Data Sets", |vars| {
        if defined.is_empty() && vars.datasets == parsed {
            return Ok(());
        }
        vars.variables.extend(defined);
        vars.datasets = parsed;
        vars.active_dataset = None;
        Ok(())
    })?;
    Ok(json!({"imported": names.len(), "names": names, "defined": new_variables, "deletedDataSets": deleted}))
}

/// Export every dataset as CSV ([`datasets_to_csv`]: one column per variable, no dataset names).
/// With `path`, write the file; else return
/// `{csv}`. The reply's `warning` names the variables whose column makes a document without them
/// refuse the file, or else the variables it doesn't read back with their kind
/// ([`kinds_a_new_document_loses`]); the app shows it in its status bar.
/// A document with more data sets or variables than [`dataset_import`] reads is refused.
pub(super) fn dataset_export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.export";
    let st = s.doc()?;
    if st.doc.variables.variables.is_empty() {
        return Err(bad(C, "define a variable first"));
    }
    if st.doc.variables.datasets.len() > MAX_CSV_ROWS {
        return Err(bad(C, format!("more than {MAX_CSV_ROWS} data sets: Import Data Sets reads at most {MAX_CSV_ROWS}")));
    }
    if st.doc.variables.variables.len() > MAX_CSV_COLS {
        return Err(bad(C, format!("more than {MAX_CSV_COLS} variables: Import Data Sets reads at most {MAX_CSV_COLS}")));
    }
    let csv = datasets_to_csv(&st.doc.variables, C)?;
    if csv.len() > MAX_CSV_BYTES {
        return Err(bad(C, format!("CSV would be larger than {MAX_CSV_BYTES} bytes")));
    }
    let mut out = match str_param(p, "path") {
        Some(path) => {
            let path = absolute_path(path);
            write_file(&path, csv.as_bytes())?;
            json!({"path": path, "datasets": st.doc.variables.datasets.len()})
        }
        None => json!({"csv": csv, "datasets": st.doc.variables.datasets.len()}),
    };
    let (refused, lost) = kinds_a_new_document_loses(&st.doc.variables);
    let list = |vars: &[&str]| vars.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
    if !refused.is_empty() {
        let names = list(&refused);
        out["warning"] = json!(format!("a new document refuses this file because of these variables: {names}"));
    } else if !lost.is_empty() {
        let names = list(&lost);
        out["warning"] = json!(format!("a new document won't read these variables back with their kind: {names}"));
    }
    Ok(out)
}
