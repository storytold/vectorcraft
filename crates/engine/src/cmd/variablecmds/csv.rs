use super::{
    BTreeMap, BTreeSet, DataSet, DataValue, EngineError, MAX_NAME, Result, Session, Value, Variable, VariableKind, Variables, absolute_path, bad,
    json, read_file_head, str_param, variables_unlocked, write_file,
};
use crate::cmd::place::text::windows_1252;
use crate::cmd::{MAX_GRAPH_DATA_FILE, text_param};

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
/// byte-order mark, else UTF-16 without one when the file holds NUL bytes (see
/// [`utf16_without_mark`]), else UTF-8, with or without a byte-order mark, else Windows-1252. A
/// file that starts with a byte-order mark must be valid in its encoding, and a file with NUL
/// bytes and no byte-order mark must read as UTF-16 by [`utf16_without_mark`]'s rule.
fn datasets_csv_text(bytes: &[u8], cmd: &str) -> Result<String> {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => return utf16(rest, true).ok_or_else(|| bad(cmd, "CSV is not UTF-16")),
        [0xFE, 0xFF, rest @ ..] => return utf16(rest, false).ok_or_else(|| bad(cmd, "CSV is not UTF-16")),
        _ if bytes.contains(&0) => {
            return utf16_without_mark(bytes).ok_or_else(|| bad(cmd, "CSV holds NUL bytes: save it as UTF-8, or as UTF-16 with a byte-order mark"));
        }
        _ => {}
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(_) if bytes.starts_with(b"\xEF\xBB\xBF") => Err(bad(cmd, "CSV is not UTF-8")),
        Err(_) => Ok(bytes.iter().map(|&b| windows_1252(b)).collect()),
    }
}

/// `bytes` as UTF-16, little-endian when `le` is set, or `None` when they aren't valid UTF-16.
fn utf16(bytes: &[u8], le: bool) -> Option<String> {
    let (units, odd) = bytes.as_chunks::<2>();
    let units = units.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) });
    char::decode_utf16(units).collect::<std::result::Result<String, _>>().ok().filter(|_| odd.is_empty())
}

/// The byte order of `bytes`, which hold NUL bytes and no byte-order mark, read as UTF-16:
/// `Some(true)` for little-endian, or `None` when they don't read as UTF-16. At least half of the
/// two-byte units must hold a NUL byte, as characters from U+0001 to U+00FF do; a file with fewer,
/// such as UTF-8 or Windows-1252 text with stray NUL bytes, is refused. The byte order is the one
/// that reads a line break (`\n` or `\r`). The other order reads one only from U+0A00 or U+0D00,
/// so a file that reads one in both orders is refused. A file that reads none in either order (one
/// line) is big-endian when more of its units start with a NUL byte than end with one, as
/// characters from U+0001 to U+00FF do in that order, and little-endian otherwise. An odd last
/// byte is left out of the count.
pub fn utf16_order_without_mark(bytes: &[u8]) -> Option<bool> {
    let units = bytes.as_chunks::<2>().0;
    if units.iter().filter(|c| c.contains(&0)).count() * 2 < units.len() {
        return None;
    }
    let is_line_break = |u: u16| matches!(u, 0x0A | 0x0D);
    let lines_le = units.iter().any(|&c| is_line_break(u16::from_le_bytes(c)));
    let lines_be = units.iter().any(|&c| is_line_break(u16::from_be_bytes(c)));
    match (lines_le, lines_be) {
        (true, true) => None,
        (false, false) => Some(units.iter().filter(|c| c[0] == 0).count() <= units.iter().filter(|c| c[1] == 0).count()),
        (_, lines_be) => Some(!lines_be),
    }
}

/// `bytes`, which hold NUL bytes and no byte-order mark, as UTF-16 in the byte order
/// [`utf16_order_without_mark`] gives, or `None`. The file must have an even length, and the text
/// must be valid UTF-16 in that order and hold no NUL character.
fn utf16_without_mark(bytes: &[u8]) -> Option<String> {
    utf16(bytes, utf16_order_without_mark(bytes)?).filter(|text| !text.contains('\0'))
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
/// or values it refuses, so it doesn't import the file (a `%` header over values, which it reads as
/// paths of graph data files, a `#` header whose values aren't true or false, a header that names
/// a variable an earlier column named), and the second the variables it reads as another variable
/// (a variable whose name starts with `#`, `@` or `%` and isn't of that prefix's kind, and a graph
/// variable, which the export writes without `%`).
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
        // A `%` header's cells are paths of graph data files, which the values the export writes aren't.
        let files = kind == VariableKind::Graph && name != cell;
        let cells_read = vars.datasets.iter().filter_map(|d| d.values.get(&v.name)).all(|val| {
            let cell = value_to_cell(val);
            if files { cell.trim().is_empty() } else { cell_to_value(kind, &cell, cell.is_empty(), &name, 2, C).is_ok() }
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
/// name is added to `defined`. `%` before a name makes it a graph variable whose column holds the
/// paths of graph data files ([`graph_file_value`]); [`dataset_import`] tells such a column apart
/// by a returned name that is not the cell.
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
    } else if let Some(rest) = cell.strip_prefix('%') {
        (rest.trim(), VariableKind::Graph)
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

/// The most bytes the `%` columns of one import read from graph data files and keep as graph
/// values, counted together: each file's bytes once (it is read once, however many cells name it),
/// and the graph value of every cell, which each data set holds a copy of.
const MAX_GRAPH_FILES_TOTAL: usize = 256 * 1_048_576;

/// The graph data files the `%` columns of one import have read, by path, and the bytes counted
/// against [`MAX_GRAPH_FILES_TOTAL`] so far.
#[derive(Default)]
struct GraphFiles {
    read: BTreeMap<String, String>,
    total: usize,
}

/// The graph value of a `%` column's cell `path` on file line `line`, in 1-based `column`, for the
/// graph variable `var`: the graph data file at `path` (taken from the folder `base` when
/// relative), read as Graph Data's Import Data reads one ([`crate::cmd::graph::graph_csv_from_file`]),
/// or no value for an empty cell. The path is followed as an image variable's path is
/// ([`crate::cmd::fileio::document_path`]): only files on this computer, or on the network share
/// `base` is on, are read. A relative path without `base`, a path on another computer or share, a
/// device or a URL, a file that can't be read or is larger than 1 MB, a file that gives a graph
/// value longer than [`MAX_GRAPH_CSV`] bytes and more than [`MAX_GRAPH_FILES_TOTAL`] bytes in all
/// are refused, with the line and the column.
fn graph_file_value(files: &mut GraphFiles, path: &str, base: Option<&str>, var: &str, (line, column): (usize, usize)) -> Result<Option<DataValue>> {
    const C: &str = "dataset.import";
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }
    // Without `base`, a path that stays relative once a `file:` URL is read is refused here, so the
    // error says what to give.
    if base.is_none() && craft_docpath::resolve(path, None).is_ok_and(|p| !p.is_absolute()) {
        return Err(bad(
            C,
            format!("line {line}, column {column}: the graph data file `{path}` for `{var}` is a relative path: give `base`, or an absolute path"),
        ));
    }
    // `document_path` takes a relative path from the folder of a document; any file name in `base`
    // stands for the CSV there.
    let document = base.map(|base| std::path::Path::new(base).join("data sets.csv").to_string_lossy().into_owned());
    let full = crate::cmd::fileio::document_path(path, document.as_deref())
        .map_err(|r| bad(C, format!("line {line}, column {column}: the graph data file `{path}` for `{var}` isn't followed: {r}")))?;
    let csv = match files.read.get(&full) {
        Some(csv) => csv.clone(),
        None => {
            // One byte past the limit is enough to refuse the file.
            let bytes = read_file_head(&full, MAX_GRAPH_DATA_FILE + 1)
                .map_err(|e| bad(C, format!("line {line}, column {column}: the graph data file for `{var}` can't be read: {e}")))?;
            if bytes.len() > MAX_GRAPH_DATA_FILE {
                return Err(bad(
                    C,
                    format!("line {line}, column {column}: the graph data file `{path}` for `{var}` is larger than {MAX_GRAPH_DATA_FILE} bytes"),
                ));
            }
            let csv = crate::cmd::graph::graph_csv_from_file(&bytes)?;
            if csv.len() > MAX_GRAPH_CSV {
                return Err(bad(
                    C,
                    format!(
                        "line {line}, column {column}: the graph data file `{path}` for `{var}` gives a graph value longer than {MAX_GRAPH_CSV} bytes"
                    ),
                ));
            }
            files.total += bytes.len();
            files.read.insert(full, csv.clone());
            csv
        }
    };
    files.total += csv.len();
    if files.total > MAX_GRAPH_FILES_TOTAL {
        return Err(bad(C, format!("line {line}, column {column}: the graph data files hold more than {MAX_GRAPH_FILES_TOTAL} bytes in all")));
    }
    Ok(Some(DataValue::Graph(csv)))
}

/// Import datasets from CSV. Every header cell names a variable, as [`header_variable`] reads it,
/// and the variables the document doesn't define yet are defined, unbound. Two headers that name
/// one variable are refused. The cells of a `%` column are paths of graph data files
/// ([`graph_file_value`]); a relative one is taken from the folder `base` gives, else from the
/// folder of the file `path` names. The rows replace every dataset of the document and are named
/// "Data Set 1", "Data Set 2" and so on, in file order. The document's variables and their bindings
/// stay. One undo step, and none when the variables and datasets stay as they were.
pub(super) fn dataset_import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "dataset.import";
    let base = match text_param(p, "base", C)? {
        Some("") => return Err(bad(C, "`base` is empty")),
        Some(base) => Some(absolute_path(base)),
        None => str_param(p, "path").and_then(|path| std::path::Path::new(&absolute_path(path)).parent().map(|d| d.to_string_lossy().into_owned())),
    };
    let (bytes, given_as_text) = datasets_csv_bytes(p, C)?;
    // `data` and `csv` are already text, NUL characters included (`dataset.export` writes them).
    let text = if given_as_text { String::from_utf8(bytes).map_err(|_| bad(C, "CSV is not UTF-8"))? } else { datasets_csv_text(&bytes, C)? };
    let rows = parse_datasets_csv(&text, C)?;
    let Some((_, header, _)) = rows.first() else {
        return Err(bad(C, "CSV is empty"));
    };
    let kinds: BTreeMap<String, VariableKind> = s.doc()?.doc.variables.variables.iter().map(|v| (v.name.clone(), v.kind)).collect();
    // Each column: its index, its variable and kind, and whether its cells are graph data files.
    let mut columns: Vec<(usize, String, VariableKind, bool)> = Vec::new();
    let mut defined: Vec<Variable> = Vec::new();
    for (i, h) in header.iter().enumerate() {
        let cell = h.trim();
        if cell.is_empty() {
            return Err(bad(C, format!("empty header in column {}", i + 1)));
        }
        let (name, kind) = header_variable(cell, i + 1, &kinds, &mut defined, C)?;
        if let Some(&(first, ..)) = columns.iter().find(|(_, n, ..)| *n == name) {
            let (first, column) = (first + 1, i + 1);
            return Err(bad(C, format!("columns {first} and {column} both name the variable `{name}`")));
        }
        let files = kind == VariableKind::Graph && name != cell;
        columns.push((i, name, kind, files));
    }
    // Lock Variables refuses New Variable, so the import is refused when a header would define one.
    if !defined.is_empty() {
        variables_unlocked(s).map_err(|why| EngineError::Disabled(C.into(), why))?;
    }
    if rows.len() == 1 {
        return Err(bad(C, "CSV has a header but no data rows"));
    }
    let mut parsed: Vec<DataSet> = Vec::new();
    let mut files = GraphFiles::default();
    for (n, &(line, ref row, ref quoted)) in rows.iter().skip(1).enumerate() {
        let name = format!("Data Set {}", n + 1);
        let mut values = BTreeMap::new();
        for &(col, ref var, kind, from_files) in &columns {
            let cell = row.get(col).map(String::as_str).unwrap_or("");
            let was_quoted = quoted.get(col).copied().unwrap_or(false);
            let value = if from_files {
                graph_file_value(&mut files, cell, base.as_deref(), var, (line, col + 1))?
            } else {
                cell_to_value(kind, cell, was_quoted, var, line, C)?
            };
            if let Some(v) = value {
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
