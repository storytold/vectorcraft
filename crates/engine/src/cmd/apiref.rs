//! `engine.apiReference`: the command API reference in Markdown, written when it is asked for from
//! the parameter schemas of the built-in commands ([`crate::schemas`]). Each command that has a
//! schema gets a section, in registry order, with its parameters in the schema's order; a command
//! without a schema, a plug-in's included, is left out. The text depends only on the registry, so
//! every call in one build returns the same bytes.
//!
//! Text from a schema is escaped before it goes into a heading, a list item or a table cell: the
//! Markdown punctuation characters get a backslash (CommonMark 0.31.2, backslash escapes,
//! <https://spec.commonmark.org/0.31.2/#backslash-escapes>), so a description shows as plain text and
//! cannot end a table cell or make a link, an image or HTML; line breaks, tabs and other control
//! characters become spaces.

use serde_json::{Value, json};
use vectorcraft_apischema::{Bound, MAX_DEPTH, MAX_ID, Omitted, Param, ParamSchema, Type, Writes};

use super::{CommandSpec, Result, Session, always, cmd, command_specs};
use crate::EngineError;

const ID: &str = "engine.apiReference";

/// The largest reference the command returns, in bytes of UTF-8.
pub const MAX_BYTES: usize = 1 << 20;

static REFERENCE: ParamSchema = ParamSchema::new(
    &[],
    "the command API reference in Markdown, written from the built-in commands' parameter schemas: a section for each command that has one, in registry order",
)
.returns("{markdown: string}")
.writes(Writes::Nothing)
.since(2);

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "engine.apiReference",
        "Command API Reference",
        [],
        None,
        "{} the command API reference in Markdown, written from the built-in commands' parameter schemas: a section for each command that has one, in registry order → {markdown: string}",
        always,
        api_reference,
        schema: &REFERENCE
    )]
}

fn api_reference(_: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "markdown": reference(command_specs())? }))
}

const HEADER: &str = "# VectorCraft command API reference\n\nThis reference lists only the commands that have a parameter schema, in registry order. `engine.commands` lists every command, with its params text and whether it can run now.\n";

/// The reference for the commands of `specs` that have a schema, or an error when it would be
/// larger than [`MAX_BYTES`]; a reference that is too large is never cut short.
pub(crate) fn reference(specs: &[CommandSpec]) -> Result<String> {
    let mut out = String::from(HEADER);
    let mut sections = 0;
    for spec in specs {
        let Some(schema) = spec.schema else { continue };
        let text = section(spec, schema);
        if out.len() + text.len() > MAX_BYTES {
            return Err(EngineError::Other(format!("`{ID}`: the reference would be larger than {MAX_BYTES} bytes")));
        }
        out.push_str(&text);
        sections += 1;
    }
    if sections == 0 {
        out.push_str("\nNo commands have parameter schemas.\n");
    }
    Ok(out)
}

/// One command's section: its heading, a list of what it is, and its parameters.
fn section(spec: &CommandSpec, schema: &ParamSchema) -> String {
    let menu = if spec.menu.is_empty() { "None".to_string() } else { spec.menu.iter().map(|m| escape(m)).collect::<Vec<_>>().join(" › ") };
    let mut s = format!("\n## {}\n\n", escape(spec.id));
    s.push_str(&format!("- Label: {}\n", escape(spec.label)));
    s.push_str(&format!("- Menu: {menu}\n"));
    s.push_str(&format!("- API level: {}\n", schema.since));
    s.push_str(&format!("- Deprecated: {}\n", level_text(schema.deprecated)));
    s.push_str(&format!("- Writes: {}\n", writes_text(schema.writes)));
    s.push_str(&format!("- Cost: {}\n", schema.cost));
    s.push_str(&format!("- Summary: {}\n", text_or_none(schema.summary)));
    s.push_str(&format!("- Returns: {}\n\n", text_or_none(schema.returns)));
    if schema.params.is_empty() {
        s.push_str("No parameters.\n");
        return s;
    }
    s.push_str("| Parameter | Type | Range | Unit | Default | Required | Since | Deprecated | Description |\n");
    s.push_str("|---|---|---|---|---|---|---|---|---|\n");
    rows("", schema.params, 0, &mut s);
    s
}

/// A table row for each of `params`, and after each one the rows of the members of an object it
/// holds: `edits[].name` for an object in a list, `options.size` for an object.
fn rows(prefix: &str, params: &[Param], depth: usize, out: &mut String) {
    for p in params {
        let name = if prefix.is_empty() { p.name.to_string() } else { format!("{prefix}.{}", p.name) };
        let (range, unit) = range_and_unit(&p.ty);
        out.push_str(&format!(
            "| {} | {} | {range} | {unit} | {} | {} | {} | {} | {} |\n",
            escape(&name),
            type_text(&p.ty, depth),
            default_text(p.omitted),
            if p.required { "Yes" } else { "No" },
            p.since,
            level_text(p.deprecated),
            description(p),
        ));
        if depth < MAX_DEPTH
            && let Some((members, path)) = members(&p.ty, name, depth)
        {
            rows(&path, members, depth + 1, out);
        }
    }
}

/// The parameters of the object `ty` holds, directly or as the items of lists, with the path of
/// its members' prefix (`name`, `name[]`, `name[][]`).
fn members(ty: &Type, mut path: String, depth: usize) -> Option<(&'static [Param], String)> {
    let mut ty = ty;
    for _ in depth..MAX_DEPTH {
        match ty {
            Type::Object(params) => return Some((params, path)),
            Type::Array(item) => {
                path.push_str("[]");
                ty = item;
            }
            _ => return None,
        }
    }
    None
}

fn type_text(ty: &Type, depth: usize) -> String {
    match ty {
        Type::Number { .. } => "number".into(),
        Type::Integer { .. } => "integer".into(),
        Type::Boolean => "boolean".into(),
        Type::String => "string".into(),
        Type::Enum(names) => format!("one of {}", names.iter().map(|n| format!("\"{}\"", escape(n))).collect::<Vec<_>>().join(", ")),
        Type::Id => "object id (integer)".into(),
        Type::Ids => "list of object ids".into(),
        Type::Color => escape("color (a string, [r, g, b] or an object)"),
        Type::Point => escape("point ([x, y], two numbers)"),
        Type::Object(_) => "object".into(),
        Type::Array(_) if depth >= MAX_DEPTH => "list".into(),
        Type::Array(item) => format!("list of {}", type_text(item, depth + 1)),
    }
}

/// The range and unit cells of a value of type `ty`.
fn range_and_unit(ty: &Type) -> (String, String) {
    let ends = |low: Option<String>, high: Option<String>| {
        let both: Vec<String> = low.into_iter().chain(high).collect();
        if both.is_empty() { "None".to_string() } else { both.join(", ") }
    };
    match ty {
        Type::Number { min, max, unit } => {
            let low = min.map(|b| match b {
                Bound::Inclusive(a) => format!("at least {a}"),
                Bound::Exclusive(a) => format!("more than {a}"),
            });
            let high = max.map(|b| match b {
                Bound::Inclusive(a) => format!("at most {a}"),
                Bound::Exclusive(a) => format!("less than {a}"),
            });
            (ends(low, high), unit.map_or("None".to_string(), escape))
        }
        Type::Integer { min, max } => (ends(min.map(|a| format!("at least {a}")), max.map(|a| format!("at most {a}"))), "None".into()),
        Type::Id => (format!("at least 0, at most {MAX_ID}"), "None".into()),
        _ => ("None".into(), "None".into()),
    }
}

fn default_text(omitted: Omitted) -> String {
    match omitted {
        Omitted::Nothing => "None".into(),
        Omitted::Value(json) => format!("{} (JSON)", escape(json)),
        Omitted::Note(note) => format!("{} (worked out by the command)", escape(note)),
    }
}

/// The parameter's doc text, and what the schema says about how its value is checked.
fn description(p: &Param) -> String {
    let mut parts = Vec::new();
    if !p.doc.is_empty() {
        parts.push(escape(p.doc));
    }
    if p.null_omitted {
        parts.push("null reads as omitted".to_string());
    }
    if p.lenient {
        parts.push("lenient (a value of another type or out of range is passed to the command, not refused)".to_string());
    }
    if parts.is_empty() { "None".to_string() } else { parts.join("; ") }
}

fn level_text(level: Option<u32>) -> String {
    level.map_or("None".to_string(), |l| format!("API level {l}"))
}

fn writes_text(writes: Writes) -> &'static str {
    match writes {
        Writes::Nothing => "nothing",
        Writes::Session => "session",
        Writes::Preferences => "preferences",
        Writes::Document => "document",
    }
}

fn text_or_none(text: &str) -> String {
    if text.is_empty() { "None".to_string() } else { escape(text) }
}

/// The characters [`escape`] puts a backslash before.
const ESCAPED: &[char] = &['\\', '|', '[', ']', '<', '>', '`', '*', '_', '!', '&', '#', '~', ':', '@'];

/// `text` as plain Markdown text on one line: each of [`ESCAPED`] with a backslash before it, the
/// `.` after `www` too (so no autolink starts), and each control character as a space.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.push(' ');
            continue;
        }
        let after_www = out.len().checked_sub(3).and_then(|at| out.as_bytes().get(at..)).is_some_and(|b| b.eq_ignore_ascii_case(b"www"));
        if ESCAPED.contains(&c) || (c == '.' && after_www) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(_: &mut Session, _: &Value) -> Result<Value> {
        Ok(Value::Null)
    }

    fn spec(id: &'static str, menu: &'static [&'static str], schema: Option<&'static ParamSchema>) -> CommandSpec {
        CommandSpec { id, label: "Label", menu, shortcut: None, params: "", enabled: always, run, journal: false, schema }
    }

    /// The cells of a table row as a GFM table reads them: split at each `|` that has no backslash
    /// before it, with the outer pipes and the spaces around each cell taken off.
    fn cells(row: &str) -> Vec<String> {
        let inner = row.strip_prefix('|').and_then(|r| r.strip_suffix('|')).unwrap_or_else(|| panic!("not a table row: {row}"));
        let (mut out, mut cell, mut chars) = (Vec::new(), String::new(), inner.chars());
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    cell.push(c);
                    cell.extend(chars.next());
                }
                '|' => out.push(std::mem::take(&mut cell).trim().to_string()),
                _ => cell.push(c),
            }
        }
        out.push(cell.trim().to_string());
        out
    }

    /// What `text` shows as: each escaped character as itself. Panics on a Markdown punctuation
    /// character without its backslash, or a backslash before a character that needs none.
    fn literal(text: &str) -> String {
        let (mut out, mut chars) = (String::new(), text.chars());
        while let Some(c) = chars.next() {
            assert!(!ESCAPED.contains(&c) || c == '\\', "`{c}` is not escaped in {text}");
            if c == '\\' {
                let next = chars.next().unwrap_or_else(|| panic!("a backslash ends {text}"));
                assert!(ESCAPED.contains(&next) || next == '.', "`\\{next}` in {text}");
                out.push(next);
            } else {
                out.push(c);
            }
        }
        out
    }

    /// The shown cells of the row of parameter `name`.
    fn row(markdown: &str, name: &str) -> Vec<String> {
        let found = markdown.lines().filter(|l| l.starts_with('|')).find(|l| cells(l).first().map(|c| literal(c)).as_deref() == Some(name));
        let row = found.unwrap_or_else(|| panic!("no row for {name} in\n{markdown}"));
        let cells = cells(row);
        assert_eq!(cells.len(), 9, "{row}");
        cells.iter().map(|c| literal(c)).collect()
    }

    /// The text after `- {label}: ` in the section's list.
    fn item(markdown: &str, label: &str) -> String {
        let prefix = format!("- {label}: ");
        let line = markdown.lines().find_map(|l| l.strip_prefix(prefix.as_str()));
        literal(line.unwrap_or_else(|| panic!("no {label} in\n{markdown}")))
    }

    /// Two calls give the same bytes, with no document open, and the reference has one section
    /// for each built-in command that has a schema, in registry order, ending with this command's.
    #[test]
    fn the_reference_lists_each_schema_once_in_registry_order_and_is_the_same_each_time() {
        let mut s = Session::new();
        let first = s.execute(ID, &json!({})).unwrap();
        let second = s.execute(ID, &Value::Null).unwrap();
        assert_eq!(first, second);
        let markdown = first["markdown"].as_str().unwrap();
        assert_eq!(markdown, reference(command_specs()).unwrap());
        assert!(markdown.len() < MAX_BYTES / 10, "{} bytes", markdown.len());
        let headings: Vec<String> = markdown.lines().filter_map(|l| l.strip_prefix("## ")).map(literal).collect();
        let expected: Vec<&str> = command_specs().iter().filter(|c| c.schema.is_some()).map(|c| c.id).collect();
        assert_eq!(headings, expected);
        assert_eq!(headings.last().map(String::as_str), Some(ID));
        assert!(expected.contains(&"view.saved.new") && !headings.iter().any(|h| h == "file.new"));
        let new_view = &markdown[markdown.find("## view.saved.new").unwrap()..];
        let names: Vec<String> = new_view
            .lines()
            .skip_while(|l| !l.starts_with("|---"))
            .skip(1)
            .take_while(|l| l.starts_with('|'))
            .map(|l| literal(&cells(l)[0]))
            .collect();
        assert_eq!(names, ["name", "center", "zoom", "rotation"]);
        let edit = &markdown[markdown.find("## view.saved.edit").unwrap()..];
        assert_eq!(row(edit, "edits[].newName")[8], "null reads as omitted");
        assert_eq!(row(edit, "newName")[8], "rename; null reads as omitted");
    }

    const INNER: &[Param] =
        &[Param::new("size", Type::Integer { min: Some(1), max: Some(10) }), Param::new("mode", Type::Enum(&["a", "b"])).defaults_to("\"a\"")];

    static FULL: ParamSchema = ParamSchema::new(
        &[
            Param::new("zoom", Type::Number { min: Some(Bound::Exclusive(0.0)), max: Some(Bound::Inclusive(64.0)), unit: Some("pt") })
                .doc("1 = 100%"),
            Param::new("name", Type::String).default_note("the first free name").null_omitted(),
            Param::new("old", Type::Boolean).defaults_to("true").since(2).deprecated(3).lenient(),
            Param::new("edits", Type::Array(&Type::Object(INNER))).optional(),
            Param::new("ids", Type::Ids).optional(),
            Param::new("at", Type::Point).optional(),
            Param::new("color", Type::Color).optional(),
            Param::new("id", Type::Id).optional(),
            Param::new("ratio", Type::Number { min: Some(Bound::Inclusive(-1.5)), max: Some(Bound::Exclusive(2.0)), unit: None }).optional(),
        ],
        "do the thing",
    )
    .returns("{done}")
    .writes(Writes::Preferences)
    .since(2)
    .deprecated(4)
    .cost(5);

    /// Each field of a schema shows, nested members in their order under dotted names.
    #[test]
    fn every_field_of_a_schema_is_shown() {
        let markdown = reference(&[spec("test.full", &["View", "Saved"], Some(&FULL))]).unwrap();
        for (label, shown) in [
            ("Label", "Label"),
            ("Menu", "View › Saved"),
            ("API level", "2"),
            ("Deprecated", "API level 4"),
            ("Writes", "preferences"),
            ("Cost", "5"),
            ("Summary", "do the thing"),
            ("Returns", "{done}"),
        ] {
            assert_eq!(item(&markdown, label), shown, "{label}");
        }
        let lenient = "lenient (a value of another type or out of range is passed to the command, not refused)";
        let expected: [[&str; 9]; 11] = [
            ["zoom", "number", "more than 0, at most 64", "pt", "None", "Yes", "1", "None", "1 = 100%"],
            ["name", "string", "None", "None", "the first free name (worked out by the command)", "No", "1", "None", "null reads as omitted"],
            ["old", "boolean", "None", "None", "true (JSON)", "No", "2", "API level 3", lenient],
            ["edits", "list of object", "None", "None", "None", "No", "1", "None", "None"],
            ["edits[].size", "integer", "at least 1, at most 10", "None", "None", "Yes", "1", "None", "None"],
            ["edits[].mode", "one of \"a\", \"b\"", "None", "None", "\"a\" (JSON)", "No", "1", "None", "None"],
            ["ids", "list of object ids", "None", "None", "None", "No", "1", "None", "None"],
            ["at", "point ([x, y], two numbers)", "None", "None", "None", "No", "1", "None", "None"],
            ["color", "color (a string, [r, g, b] or an object)", "None", "None", "None", "No", "1", "None", "None"],
            ["id", "object id (integer)", "at least 0, at most 9007199254740991", "None", "None", "No", "1", "None", "None"],
            ["ratio", "number", "at least -1.5, less than 2", "None", "None", "No", "1", "None", "None"],
        ];
        let names: Vec<String> =
            markdown.lines().filter(|l| l.starts_with("| ") && !l.starts_with("| Parameter")).map(|l| literal(&cells(l)[0])).collect();
        assert_eq!(names, expected.map(|r| r[0]));
        for want in expected {
            assert_eq!(row(&markdown, want[0]), want, "{}", want[0]);
        }
    }

    const HOSTILE: &str = "a | b [link](https://evil.example) <script>x</script> `code` \\ *em* _u_ ![img](x) &amp; ~~s~~ # h www.example.com me@example.com\r\nnext\tline";

    static ESCAPES: ParamSchema =
        ParamSchema::new(&[Param::new("a|b", Type::String).doc(HOSTILE).defaults_to("\"<b>|</b>\"")], HOSTILE).returns(HOSTILE);

    /// Markdown in a schema's text shows as plain text: a description stays in its own cell of a
    /// nine-cell row, every punctuation character is escaped, and line breaks and tabs are spaces.
    #[test]
    fn markdown_in_schema_text_stays_literal_in_its_cell() {
        let markdown = reference(&[spec("test.escapes", &["A|B", "[x](y)"], Some(&ESCAPES))]).unwrap();
        let shown = HOSTILE.replace(['\r', '\n', '\t'], " ");
        let cells = row(&markdown, "a|b");
        assert_eq!(cells[4], "\"<b>|</b>\" (JSON)");
        assert_eq!(cells[8], shown);
        assert_eq!(item(&markdown, "Summary"), shown);
        assert_eq!(item(&markdown, "Returns"), shown);
        assert_eq!(item(&markdown, "Menu"), "A|B › [x](y)");
        assert!(markdown.contains("www\\.example"), "{markdown}");
        let table: Vec<&str> = markdown.lines().filter(|l| l.starts_with('|')).collect();
        assert_eq!(table.len(), 3, "the header, the delimiter row and one row: {table:?}");
        assert!(!markdown.contains('\r') && !markdown.contains('\t'));
    }

    /// A registry with no schema gives the header and a line that says so; a command without a
    /// schema adds nothing.
    #[test]
    fn a_registry_without_schemas_gives_a_document_that_says_so() {
        let empty = format!("{HEADER}\nNo commands have parameter schemas.\n");
        assert_eq!(reference(&[]).unwrap(), empty);
        assert_eq!(reference(&[spec("test.none", &["View"], None)]).unwrap(), empty);
        let one = reference(&[spec("test.none", &[], None), spec("test.full", &[], Some(&FULL)), spec("test.other", &[], None)]).unwrap();
        assert_eq!(one, reference(&[spec("test.full", &[], Some(&FULL))]).unwrap());
    }

    /// A reference of exactly [`MAX_BYTES`] is returned; one byte more is an error naming the
    /// command, with no part of the reference.
    #[test]
    fn a_reference_past_the_limit_is_an_error() {
        let with = |summary: String| -> Result<String> {
            let schema: &'static ParamSchema = Box::leak(Box::new(ParamSchema::new(&[], Box::leak(summary.into_boxed_str()))));
            reference(&[spec("test.big", &[], Some(schema))])
        };
        let base = with("x".into()).unwrap().len();
        let fits = with("x".repeat(MAX_BYTES - base + 1)).unwrap();
        assert_eq!(fits.len(), MAX_BYTES);
        let e = with("x".repeat(MAX_BYTES - base + 2)).unwrap_err().to_string();
        assert!(e.contains(ID) && e.contains("larger than 1048576 bytes"), "{e}");
        assert!(e.len() < 200, "{e}");
    }

    /// An unknown key is refused with a parameter error naming it, and the command needs no
    /// document.
    #[test]
    fn an_unknown_parameter_is_refused_naming_it() {
        let mut s = Session::new();
        let e = s.execute(ID, &json!({"prefix": "view."})).unwrap_err().to_string();
        assert!(e.contains(ID) && e.contains("`prefix`"), "{e}");
        assert!(s.execute(ID, &json!([])).is_err());
        assert!(s.active().is_none());
        assert!(s.execute(ID, &json!({})).is_ok());
    }

    #[test]
    fn escape_marks_each_punctuation_character_and_flattens_control_characters() {
        assert_eq!(escape("a|b"), "a\\|b");
        assert_eq!(escape("WWW.x and w.x"), "WWW\\.x and w.x");
        assert_eq!(escape("é\u{7}\u{85}z"), "é  z");
        assert_eq!(escape("1.5 > 1"), "1.5 \\> 1");
    }
}
