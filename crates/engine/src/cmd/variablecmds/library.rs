use std::collections::HashMap;

use std::fmt::Write as _;

use vectorcraft_doc::{GraphSpec, MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES};

use super::{
    BTreeMap, BTreeSet, DataSet, DataValue, Document, EngineError, MAX_GRAPH_CSV, MAX_NAME, NodeId, NodeKind, Result, Session, Value, Variable,
    VariableKind, Variables, absolute_path, bad, json, named, read_file_head, str_param, variables_unlocked, write_file,
};

/// The namespace of a variable library's elements, the default namespace of its data sets' values,
/// and the `category` of its visibility and image variables.
const NS_VARIABLES: &str = "http://ns.adobe.com/Variables/1.0/"; // brand-ok: the namespace variable libraries use

const NS_CUSTOM: &str = "http://ns.adobe.com/GenericCustomNamespace/1.0/"; // brand-ok: the namespace variable libraries use

/// The `category` of a library's text variables.
const NS_FLOWS: &str = "http://ns.adobe.com/Flows/1.0/"; // brand-ok: the namespace variable libraries use

/// The `category` of a library's graph variables.
const NS_GRAPHS: &str = "http://ns.adobe.com/Graphs/1.0/"; // brand-ok: the namespace variable libraries use

/// The white space XML puts round a value: a value's text is read without it at either end.
const XML_SPACE: [char; 4] = [' ', '\t', '\n', '\r'];

/// Caps on a variable library (paths and MCP payloads are untrusted): its size, its XML nodes, how
/// deep its elements nest, and the entities, variables and data sets it declares.
const MAX_LIBRARY_BYTES: usize = 8 << 20;

const MAX_LIBRARY_ENTITIES: usize = 64;

const MAX_LIBRARY_NODES: u32 = 2_000_000;

const MAX_LIBRARY_NESTING: usize = 256;

const MAX_LIBRARY_VARIABLES: usize = 256;

const MAX_LIBRARY_DATASETS: usize = 1_000;

/// Bytes of a variable library from `path`, `xml` or `dataBase64`.
fn library_bytes(p: &Value, cmd: &str) -> Result<Vec<u8>> {
    let bytes = match (str_param(p, "path"), str_param(p, "xml"), str_param(p, "dataBase64")) {
        // One byte past the cap is enough to refuse the file below.
        (Some(path), ..) => read_file_head(&absolute_path(path), MAX_LIBRARY_BYTES + 1)?,
        (None, Some(xml), _) => xml.as_bytes().to_vec(),
        (None, None, Some(b64)) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "bad dataBase64"))?,
        _ => return Err(bad(cmd, "give `path`, `xml` or `dataBase64`")),
    };
    if bytes.len() > MAX_LIBRARY_BYTES {
        return Err(bad(cmd, format!("the variable library is larger than {MAX_LIBRARY_BYTES} bytes")));
    }
    Ok(bytes)
}

/// `xml` without the document type declaration of its prolog, with each reference to a general
/// entity that the declaration defines replaced by the entity's value, its quotes and `>` escaped,
/// outside CDATA sections, comments and processing instructions. The declaration may hold an
/// external identifier and an internal subset of white space, comments, processing instructions and
/// up to [`MAX_LIBRARY_ENTITIES`] entity declarations, each with a name of at most [`MAX_NAME`]
/// bytes and a quoted value that holds no `<`, `&` or `%`. Anything else in it is refused, and so is
/// a text that the replacements make longer than [`MAX_LIBRARY_BYTES`]. The XML parser then refuses
/// any other document type declaration, so its depth and node limits count the elements it
/// reads.
fn without_doctype(xml: &str, cmd: &str) -> Result<String> {
    let b = xml.as_bytes();
    let past = |from: usize, end: &str| xml.get(from..).and_then(|rest| rest.find(end)).map(|at| from + at + end.len());
    let spaces = |from: usize| from + b.get(from..).map_or(0, |rest| rest.iter().take_while(|c| c.is_ascii_whitespace()).count());
    let at = |i: usize, s: &str| xml.get(i..).is_some_and(|rest| rest.starts_with(s));
    let refused = || bad(cmd, "the variable library's document type declaration holds something other than entities of plain text");
    // The prolog: an XML declaration, white space, comments and processing instructions.
    let mut i = 0;
    loop {
        i = spaces(i);
        if at(i, "<?") {
            i = past(i + 2, "?>").ok_or_else(refused)?;
        } else if at(i, "<!--") {
            i = past(i + 4, "-->").ok_or_else(refused)?;
        } else {
            break;
        }
    }
    if !at(i, "<!DOCTYPE") {
        return Ok(xml.to_string());
    }
    let start = i;
    // A quoted literal starting at `i` → the index past it and its text.
    let literal = |i: usize| -> Option<(usize, &str)> {
        let quote = *b.get(i).filter(|c| matches!(c, b'"' | b'\''))?;
        let len = b.get(i + 1..)?.iter().position(|c| *c == quote)?;
        Some((i + len + 2, xml.get(i + 1..i + 1 + len)?))
    };
    // `<!DOCTYPE` S Name (S ExternalID)? S? ('[' intSubset ']' S?)? '>'
    i = spaces(i + "<!DOCTYPE".len());
    i += b.get(i..).map_or(0, |rest| rest.iter().take_while(|c| !c.is_ascii_whitespace() && !matches!(c, b'[' | b'>')).count());
    i = spaces(i);
    for (id, literals) in [("SYSTEM", 1), ("PUBLIC", 2)] {
        if at(i, id) {
            i += id.len();
            for _ in 0..literals {
                i = literal(spaces(i)).ok_or_else(refused)?.0;
            }
            i = spaces(i);
        }
    }
    let mut entities: Vec<(&str, String)> = Vec::new();
    if at(i, "[") {
        i += 1;
        loop {
            i = spaces(i);
            if at(i, "]") {
                i = spaces(i + 1);
                break;
            } else if at(i, "<!--") {
                i = past(i + 4, "-->").ok_or_else(refused)?;
            } else if at(i, "<?") {
                i = past(i + 2, "?>").ok_or_else(refused)?;
            } else if at(i, "<!ENTITY") {
                i = spaces(i + "<!ENTITY".len());
                let len =
                    b.get(i..).map_or(0, |rest| rest.iter().take_while(|c| !c.is_ascii_whitespace() && !matches!(c, b'"' | b'\'' | b'>')).count());
                let name = xml.get(i..i + len).filter(|n| !n.is_empty() && n.len() <= MAX_NAME && !n.starts_with('%')).ok_or_else(refused)?;
                let (end, value) = literal(spaces(i + len)).ok_or_else(refused)?;
                if value.contains(['<', '&', '%']) {
                    return Err(refused());
                }
                i = spaces(end);
                if !at(i, ">") {
                    return Err(refused());
                }
                i += 1;
                if entities.len() == MAX_LIBRARY_ENTITIES {
                    return Err(bad(cmd, format!("the variable library declares more than {MAX_LIBRARY_ENTITIES} entities")));
                }
                entities.push((name, value.replace('"', "&quot;").replace('\'', "&apos;").replace('>', "&gt;")));
            } else {
                return Err(refused());
            }
        }
    }
    if !at(i, ">") {
        return Err(refused());
    }
    let mut out = String::with_capacity(xml.len());
    out.push_str(xml.get(..start).unwrap_or_default());
    // The rest, copied with the references replaced outside CDATA sections, comments and
    // processing instructions, where the parser reads none.
    let mut i = i + 1;
    while let Some(c) = xml.get(i..).and_then(|rest| rest.chars().next()) {
        let verbatim = [("<![CDATA[", "]]>"), ("<!--", "-->"), ("<?", "?>")].into_iter().find(|(open, _)| at(i, open));
        if let Some((open, close)) = verbatim {
            let end = past(i + open.len(), close).unwrap_or(xml.len());
            out.push_str(xml.get(i..end).unwrap_or_default());
            i = end;
            continue;
        }
        // Entity names are at most `MAX_NAME` bytes long, so the `;` is looked for that far.
        let entity = if c == '&' {
            let name = xml.get(i + 1..).and_then(|rest| rest.get(..rest.bytes().take(MAX_NAME + 1).position(|b| b == b';')?));
            name.and_then(|name| entities.iter().find(|(n, _)| *n == name))
        } else {
            None
        };
        match entity {
            Some((name, value)) => {
                out.push_str(value);
                i += name.len() + 2;
                if out.len() > MAX_LIBRARY_BYTES {
                    return Err(bad(cmd, format!("the variable library's entities expand to more than {MAX_LIBRARY_BYTES} bytes")));
                }
            }
            None => {
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    if out.len() > MAX_LIBRARY_BYTES {
        return Err(bad(cmd, format!("the variable library's entities expand to more than {MAX_LIBRARY_BYTES} bytes")));
    }
    Ok(out)
}

/// One variable a library declares: its name, its kind, and its `trait` as the library spells it
/// (`None` when it has none).
struct LibraryVariable {
    name: String,
    kind: VariableKind,
    trait_name: Option<String>,
}

/// A variable library as Load Variable Library reads it: its variables in the order it declares
/// them, its data sets with their values, and the element id each text variable's `docRef` names.
pub(crate) struct Library {
    variables: Vec<LibraryVariable>,
    datasets: Vec<DataSet>,
    refs: Vec<(String, String)>,
}

/// The kind of a library variable whose `trait` is `t`: `textcontent` is text, `visibility` is
/// visibility, `fileref` (a linked file) is image and `graphdata` is graph. A variable without a
/// `trait`, or with one VectorCraft doesn't know, is of unknown kind.
fn trait_kind(t: Option<&str>) -> VariableKind {
    match t {
        Some("textcontent") => VariableKind::Text,
        Some("visibility") => VariableKind::Visibility,
        Some("fileref") => VariableKind::Image,
        Some("graphdata") => VariableKind::Graph,
        _ => VariableKind::Unknown,
    }
}

/// The `trait` and the `category` reference a library declares a variable of `kind` with, or `None`
/// for a variable of unknown kind, which is declared with neither.
fn kind_trait(kind: VariableKind) -> Option<(&'static str, &'static str)> {
    match kind {
        VariableKind::Text => Some(("textcontent", "&ns_flows;")),
        VariableKind::Visibility => Some(("visibility", "&ns_vars;")),
        VariableKind::Image => Some(("fileref", "&ns_vars;")),
        VariableKind::Graph => Some(("graphdata", "&ns_graphs;")),
        VariableKind::Unknown => None,
    }
}

impl Library {
    /// The element ids the text variables' `docRef`s name.
    pub(crate) fn elements(&self) -> std::collections::HashSet<String> {
        self.refs.iter().map(|(_, e)| e.clone()).collect()
    }
}

/// Read a variable library: XML whose `variableSets` element holds `variableSet` elements, each
/// with a `variables` element of `variable` elements (`varName`, `trait`) and a `sampleDataSets`
/// element of `sampleDataSet` elements (`dataSetName`). Elements are matched by their local names.
/// A variable's kind comes from its `trait` ([`trait_kind`]). In a data set, an element named after
/// a variable gives its value, as [`library_value`] reads it. A variable or data set declared
/// twice, a value for a variable the library doesn't declare, a value that doesn't fit its
/// variable's kind, and a library past the caps are refused. With `text_only`, only text values
/// are read, and the values of the other kinds are passed over unread. Entities that its document
/// type declaration defines are read as [`without_doctype`] says.
fn parse_library(xml: &str, cmd: &str, text_only: bool) -> Result<Library> {
    let xml = without_doctype(xml, cmd)?;
    let opts = craft_xml::Options {
        names: craft_xml::Names::AsWritten,
        limits: craft_xml::Limits { max_depth: MAX_LIBRARY_NESTING, max_nodes: MAX_LIBRARY_NODES as usize, ..craft_xml::Limits::DEFAULT },
        ..craft_xml::Options::default()
    }
    .strict();
    let doc = opts.parse(xml.as_bytes()).map_err(|e| match e {
        craft_xml::Error::TooDeep(_) => bad(cmd, format!("the variable library nests elements more than {MAX_LIBRARY_NESTING} deep")),
        _ => bad(cmd, format!("the variable library is not well-formed XML: {e}")),
    })?;
    crate::cmd::xmlattrs::validate(xml.as_bytes(), opts.limits).map_err(|e| bad(cmd, format!("the variable library is not well-formed XML: {e}")))?;
    let Some(root) = std::iter::once(&doc).chain(doc.descendants()).find(|n| n.is(craft_xml::Local("variableSets"))) else {
        return Err(bad(cmd, "the file holds no `variableSets` element"));
    };
    let sets: Vec<&craft_xml::El> = root.children(craft_xml::Local("variableSet")).collect();
    let (mut variables, mut kinds, mut refs) = (Vec::new(), BTreeMap::new(), Vec::new());
    for var in sets.iter().flat_map(|s| s.children(craft_xml::Local("variables"))).flat_map(|v| v.children(craft_xml::Local("variable"))) {
        let name = named(var.attr("varName"), "varName", cmd)?;
        let trait_name = var.attr("trait").map(|t| t.chars().take(MAX_NAME).collect::<String>());
        let kind = trait_kind(trait_name.as_deref());
        if kinds.insert(name.clone(), kind).is_some() {
            return Err(bad(cmd, format!("variable `{name}` is declared twice")));
        }
        if kinds.len() > MAX_LIBRARY_VARIABLES {
            return Err(bad(cmd, format!("the variable library declares more than {MAX_LIBRARY_VARIABLES} variables")));
        }
        if kind == VariableKind::Text {
            // `docRef="id('…')"` names the element of the variable's bound object.
            let element = var.attr("docRef").and_then(|r| r.trim().strip_prefix("id('")?.strip_suffix("')")).filter(|e| !e.is_empty());
            if let Some(element) = element {
                refs.push((name.clone(), element.to_string()));
            }
        }
        variables.push(LibraryVariable { name, kind, trait_name });
    }
    if variables.is_empty() {
        return Err(bad(cmd, "the variable library declares no variables"));
    }
    let (mut datasets, mut seen) = (Vec::new(), BTreeSet::new());
    for row in sets.iter().flat_map(|s| s.children(craft_xml::Local("sampleDataSets"))).flat_map(|d| d.children(craft_xml::Local("sampleDataSet"))) {
        let name = named(row.attr("dataSetName"), "dataSetName", cmd)?;
        if !seen.insert(name.clone()) {
            return Err(bad(cmd, format!("data set `{name}` is declared twice")));
        }
        if seen.len() > MAX_LIBRARY_DATASETS {
            return Err(bad(cmd, format!("the variable library declares more than {MAX_LIBRARY_DATASETS} data sets")));
        }
        let (mut values, mut held) = (BTreeMap::new(), BTreeSet::new());
        for el in row.elements() {
            let var = el.local();
            let Some(kind) = kinds.get(var) else {
                return Err(bad(cmd, format!("data set `{name}` holds a value for `{var}`, which the library doesn't declare")));
            };
            if !held.insert(var) {
                return Err(bad(cmd, format!("data set `{name}` holds two values for `{var}`")));
            }
            if text_only && *kind != VariableKind::Text {
                continue;
            }
            if let Some(value) = library_value(el, *kind, &name, cmd)? {
                values.insert(var.to_string(), value);
            }
        }
        datasets.push(DataSet { name, values });
    }
    Ok(Library { variables, datasets, refs })
}

/// The value a data set's element `el` gives a variable of `kind`, or `None` when it gives none.
/// A text value is the text of its `p` elements, one paragraph each, and none without a `p`
/// element. A visibility value is `true` or `false`, in any case. An image value is the absolute
/// path of a file, or a `file:` URL of one ([`library_path`]); the file isn't read. A graph value is
/// a `data` element ([`graph_value`]). Visibility, image and graph values are read without the
/// white space round them, and an element holding only white space gives none. A variable of
/// unknown kind takes no value, so what its element holds is passed over. `set` names the data set
/// in the errors.
fn library_value(el: &craft_xml::El, kind: VariableKind, set: &str, cmd: &str) -> Result<Option<DataValue>> {
    let refused = |what: &str| bad(cmd, format!("data set `{set}` holds a value for `{}` that isn't {what}", el.local()));
    let text = || if el.elements().next().is_some() { None } else { Some(el.deep_text()) };
    match kind {
        VariableKind::Text => {
            let paras: Vec<String> = el.children(craft_xml::Local("p")).map(craft_xml::El::deep_text).collect();
            Ok((!paras.is_empty()).then(|| DataValue::Text(paras.join("\n"))))
        }
        VariableKind::Visibility => {
            let what = "`true` or `false`";
            let text = text().ok_or_else(|| refused(what))?;
            match text.trim_matches(XML_SPACE) {
                "" => Ok(None),
                t if t.eq_ignore_ascii_case("true") => Ok(Some(DataValue::Visible(true))),
                t if t.eq_ignore_ascii_case("false") => Ok(Some(DataValue::Visible(false))),
                _ => Err(refused(what)),
            }
        }
        VariableKind::Image => {
            let what = "a file's absolute path or `file:` URL";
            let text = text().ok_or_else(|| refused(what))?;
            match text.trim_matches(XML_SPACE) {
                "" => Ok(None),
                t => library_path(t).map(|p| Some(DataValue::Image(p))).ok_or_else(|| refused(what)),
            }
        }
        VariableKind::Graph => graph_value(el, set, cmd),
        VariableKind::Unknown => Ok(None),
    }
}

/// The absolute path a library's image value `text` names: a `file:` URL's path (`file:///…`,
/// `file://localhost/…` or `file:/…`), its `%XX` escapes decoded (a URL with a host names a share,
/// `//host/…`), or `text` itself. `None` unless that is an absolute path: one starting with `/` or
/// `\\`, or a drive's path from its root (`C:\…`, `C:/…`).
fn library_path(text: &str) -> Option<String> {
    let drive = |p: &str| matches!(p.as_bytes(), [l, b':', b'\\' | b'/', ..] if l.is_ascii_alphabetic());
    let url = match (text.strip_prefix("file://"), text.strip_prefix("file:")) {
        (Some(rest), _) => Some(rest.strip_prefix("localhost").filter(|r| r.starts_with('/')).unwrap_or(rest)),
        // `file:/…` names a path without a host.
        (None, Some(rest)) if rest.starts_with('/') => Some(rest),
        _ => None,
    };
    let path = match url {
        Some(rest) => {
            if rest.starts_with('/') {
                let p = percent_decoded(rest.trim_start_matches('/'));
                if drive(&p) { p } else { format!("/{p}") }
            } else {
                format!("//{}", percent_decoded(rest))
            }
        }
        None => text.to_string(),
    };
    (path.len() > 1 && (path.starts_with('/') || path.starts_with("\\\\") || drive(&path))).then_some(path)
}

/// `s` with its `%XX` escapes decoded, or as it is when the decoded bytes aren't UTF-8.
fn percent_decoded(s: &str) -> String {
    let hex = |b: u8| char::from(b).to_digit(16);
    let (mut out, mut rest) = (Vec::with_capacity(s.len()), s.as_bytes());
    while let Some((&b, tail)) = rest.split_first() {
        match tail {
            [h, l, more @ ..] if b == b'%' && hex(*h).is_some() && hex(*l).is_some() => {
                out.push(((hex(*h).unwrap_or(0) << 4) | hex(*l).unwrap_or(0)) as u8);
                rest = more;
            }
            _ => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// How a library writes the image value `path`: an absolute path from the root of a Unix file
/// system as a `file:` URL (`file:///` and the path, with `%`, `#`, `?`, spaces, controls and
/// bytes past ASCII escaped), any other path as it is. [`library_path`] reads it back.
fn library_file(path: &str) -> String {
    if !path.starts_with('/') || path.starts_with("//") {
        return path.to_string();
    }
    let mut url = String::from("file:///");
    for &b in path.as_bytes() {
        if matches!(b, b'%' | b'#' | b'?') || !(0x21..=0x7e).contains(&b) {
            let _ = write!(url, "%{b:02X}");
        } else {
            url.push(char::from(b));
        }
    }
    url
}

/// A library's graph value: the `data` element that `el` holds, read as CSV of the kind Graph
/// Data takes. Its `propertyRow` holds a corner cell and then the names of the series, and each
/// `row` of its `values` holds the row's category in a first `value` whose `key` is `name`, then
/// the row's values. The graph has categories when a row's name isn't blank or is marked
/// `type="string"`. Labels are read as they are; a value is read without the white space round it,
/// and one that is empty, marked `type="string"` or not a finite number is a blank. Without rows or
/// series, or when `el` holds only white space, there is no value. Anything else in `el`, more than
/// [`MAX_GRAPH_CATEGORIES`] rows or [`MAX_GRAPH_SERIES`] series (what a graph holds), and CSV longer
/// than graph values hold, are refused.
fn graph_value(el: &craft_xml::El, set: &str, cmd: &str) -> Result<Option<DataValue>> {
    let var = el.local();
    let refused = || bad(cmd, format!("data set `{set}` holds a value for `{var}` that isn't a graph's `data` element"));
    let named = |name: &'static str| craft_xml::Local(name);
    let mut children = el.elements();
    let data = match (children.next(), children.next()) {
        (None, _) if el.deep_text().trim_matches(XML_SPACE).is_empty() => return Ok(None),
        (Some(data), None) if data.local() == "data" => data,
        _ => return Err(refused()),
    };
    // Labels keep their white space; numbers are read without it.
    let series: Vec<String> =
        data.child(named("propertyRow")).map(|r| r.children(named("value")).skip(1).map(craft_xml::El::deep_text).collect()).unwrap_or_default();
    let rows: Vec<&craft_xml::El> = data.children(named("values")).flat_map(|v| v.children(named("row"))).take(MAX_GRAPH_CATEGORIES + 1).collect();
    if rows.is_empty() && series.is_empty() {
        return Ok(None);
    }
    let too_many = || bad(cmd, format!("data set `{set}` holds a graph value for `{var}` with {}", graph_limits()));
    // A row's values after its name.
    let wide = |row: &&craft_xml::El| {
        let mut values = row.children(named("value")).peekable();
        let _name = values.next_if(|v| v.attr("key") == Some("name"));
        values.count() > MAX_GRAPH_SERIES
    };
    if rows.len() > MAX_GRAPH_CATEGORIES || series.len() > MAX_GRAPH_SERIES || rows.iter().any(wide) {
        return Err(too_many());
    }
    // Each row's name and the values after it. A blank name marked `type="string"` is a blank
    // category, so a graph whose categories are all blank keeps them.
    let mut labelled = false;
    let read: Vec<(Option<String>, Vec<Option<f64>>)> = rows
        .iter()
        .map(|row| {
            let mut values = row.children(named("value")).peekable();
            let name = values.next_if(|v| v.attr("key") == Some("name"));
            labelled |= name.is_some_and(|v| v.attr("type") == Some("string") || !v.deep_text().is_empty());
            let name = name.map(craft_xml::El::deep_text);
            let number = |v: &craft_xml::El| v.deep_text().trim_matches(XML_SPACE).parse::<f64>().ok().filter(|n| n.is_finite());
            let numbers = values.map(|v| if v.attr("type") == Some("string") { None } else { number(v) });
            (name, numbers.collect())
        })
        .collect();
    let categories = if labelled { read.iter().map(|(name, _)| name.clone().unwrap_or_default()).collect() } else { vec![] };
    let mut spec = GraphSpec { series, categories, ..GraphSpec::default() };
    spec.set_cells(read.into_iter().map(|(_, numbers)| numbers).collect());
    // Data set values hold graph data without the line break at its end.
    let csv = spec.to_csv().trim_end().to_string();
    if csv.len() > MAX_GRAPH_CSV {
        return Err(bad(cmd, format!("data set `{set}` holds a graph value for `{var}` longer than {MAX_GRAPH_CSV} bytes")));
    }
    Ok(Some(DataValue::Graph(csv)))
}

/// The variable library an SVG carries (SVG Options › Include Variable Data), read as
/// [`parse_library`] reads one, or `None` when the SVG holds no `variableSets` element. The
/// element is read on its own, from its start tag to the end tag that closes it
/// ([`variable_sets_range`]), with each reference to an
/// entity whose name starts with `ns_` replaced by the value the text before the element declares
/// for it (up to [`MAX_LIBRARY_ENTITIES`] of them, plain text of at most [`MAX_NAME`] bytes), as
/// files name the library's namespaces with such entities. The `ns_vars`, `ns_custom` and
/// `ns_flows` entities take the library's namespaces when the file doesn't declare them. An
/// element larger than [`MAX_LIBRARY_BYTES`], before or after the replacements, is refused.
pub(crate) fn svg_library(svg: &str) -> Option<Result<Library>> {
    const C: &str = "document.open";
    let (start, end) = variable_sets_range(svg)?;
    let read = || {
        let end = end.ok_or_else(|| bad(C, "the variable library declares no variables"))?;
        let element = svg.get(start..end).unwrap_or_default();
        if element.len() > MAX_LIBRARY_BYTES {
            return Err(bad(C, format!("the variable library is larger than {MAX_LIBRARY_BYTES} bytes")));
        }
        let mut entities: Vec<(String, String)> = Vec::new();
        for decl in svg.get(..start).unwrap_or_default().split("<!ENTITY").skip(1).take(MAX_LIBRARY_ENTITIES) {
            let decl = decl.trim_start();
            let Some((name, rest)) = decl.split_once(|c: char| c.is_ascii_whitespace()) else { continue };
            let rest = rest.trim_start();
            let Some(quote) = rest.chars().next().filter(|q| matches!(q, '"' | '\'')) else { continue };
            let Some(value) = rest.get(1..).and_then(|v| v.split(quote).next()) else { continue };
            if name.starts_with("ns_") && name.len() <= MAX_NAME && value.len() <= MAX_NAME && !value.contains(['<', '&', '%', '"']) {
                entities.push((format!("&{name};"), value.to_string()));
            }
        }
        for (name, value) in [("&ns_vars;", NS_VARIABLES), ("&ns_custom;", NS_CUSTOM), ("&ns_flows;", NS_FLOWS)] {
            if !entities.iter().any(|(n, _)| n == name) {
                entities.push((name.to_string(), value.to_string()));
            }
        }
        // Counted before anything is replaced, so a hostile file can't make a large text first.
        let grown =
            entities.iter().map(|(name, value)| element.matches(name.as_str()).count() * value.len().saturating_sub(name.len())).sum::<usize>();
        if element.len() + grown > MAX_LIBRARY_BYTES {
            return Err(bad(C, format!("the variable library's entities expand to more than {MAX_LIBRARY_BYTES} bytes")));
        }
        let mut element = element.to_string();
        for (name, value) in &entities {
            element = element.replace(name.as_str(), value);
        }
        // The SVG keeps only the text variables, so the values of the other kinds aren't read.
        parse_library(&format!("<svg>{element}</svg>"), C, true)
    };
    Some(read())
}

/// Where the first `variableSets` element of `svg` that is markup starts, and where the end tag
/// that closes it ends (`None` when no end tag does), or `None` when there is no such element.
/// Comments, CDATA sections, processing instructions and the document type declaration are
/// skipped, so text inside them that looks like the element is not taken for it.
fn variable_sets_range(svg: &str) -> Option<(usize, Option<usize>)> {
    const NAME: &str = "variableSets";
    // The element's name ends at white space, `>` or `/`.
    let named = |rest: &str| rest.strip_prefix(NAME).is_some_and(|r| r.starts_with(|c: char| c.is_ascii_whitespace() || matches!(c, '>' | '/')));
    if !svg.contains("<variableSets") {
        return None;
    }
    let (mut at, mut start, mut depth) = (0, None, 0usize);
    while let Some(off) = svg.get(at..).and_then(|s| s.find('<')) {
        at += off;
        let rest = svg.get(at..).unwrap_or_default();
        // The end of a construct that holds text, or of the tag at `at`.
        let past = |close: &str| rest.find(close).map(|e| at + e + close.len());
        let next = if rest.starts_with("<!--") {
            past("-->")
        } else if rest.starts_with("<![CDATA[") {
            past("]]>")
        } else if rest.starts_with("<?") {
            past("?>")
        } else if rest.starts_with("<!DOCTYPE") {
            // An internal subset ends at `]`, then the declaration at `>`.
            match (rest.find('['), rest.find('>')) {
                (Some(open), Some(gt)) if open < gt => {
                    rest.get(open..).and_then(|s| s.find(']')).and_then(|c| rest.get(open + c..)?.find('>').map(|g| at + open + c + g + 1))
                }
                (_, gt) => gt.map(|g| at + g + 1),
            }
        } else if let Some(close) = rest.strip_prefix("</") {
            let tag_end = past(">");
            if let Some(s) = start
                && named(close)
            {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((s, tag_end));
                }
            }
            tag_end
        } else {
            let tag_end = past(">");
            if named(rest.get(1..).unwrap_or_default()) {
                let empty = tag_end.and_then(|e| svg.get(..e)).is_some_and(|t| t.ends_with("/>"));
                if start.is_none() {
                    start = Some(at);
                }
                if empty && depth == 0 {
                    return Some((at, tag_end));
                }
                depth += usize::from(!empty);
            }
            tag_end
        };
        let Some(next) = next else { break };
        at = next;
    }
    start.map(|s| (s, None))
}

/// Give `doc`, opened from an SVG, the variable data `lib` it carries ([`svg_library`]): the
/// library's text variables, its data sets, no active data set, and each text variable bound to
/// the type object made of the element its `docRef` names, or the one type object among the
/// objects a group made of it holds (`found`: the object made of each element, by id). A `docRef`
/// that names no type object, or one another variable is bound to already, binds nothing. The art
/// stays as the file draws it. Variables of other kinds are left out with their values. The
/// warning about those variables, if any.
pub(crate) fn give_svg_library(doc: &mut Document, lib: Library, found: &HashMap<String, NodeId>) -> Option<String> {
    let (text, other): (Vec<LibraryVariable>, Vec<LibraryVariable>) = lib.variables.into_iter().partition(|v| v.kind == VariableKind::Text);
    let mut vars =
        Variables { variables: text.iter().map(|v| Variable { name: v.name.clone(), kind: VariableKind::Text }).collect(), ..Variables::default() };
    vars.datasets = lib.datasets;
    for ds in &mut vars.datasets {
        ds.values.retain(|_, v| matches!(v, DataValue::Text(_)));
    }
    // A type object written with a paint of its own, or with fills and strokes of its appearance,
    // is a group around its text (and the appearance's paths).
    let text_of = |id: NodeId| match doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(_)) => Some(id),
        Some(NodeKind::Group { children, .. }) => {
            let mut texts = children.iter().filter(|c| matches!(c.kind, NodeKind::Text(_)));
            match (texts.next(), texts.next()) {
                (Some(one), None) => Some(one.id),
                _ => None,
            }
        }
        _ => None,
    };
    for (name, element) in &lib.refs {
        let Some(id) = found.get(element).and_then(|id| text_of(*id)) else { continue };
        if !vars.bindings.contains_key(&id) {
            vars.bind(id, name);
        }
    }
    doc.variables = vars.into();
    left_out_warning(other.iter().map(|v| v.trait_name.as_deref().unwrap_or_else(|| v.kind.label())))
}

/// Load Variable Library: replace the document's variables and data sets with a variable library's
/// ([`parse_library`]) in one undo step, or none when the document already holds what the library
/// gives. Every data set is deleted, and every variable of the document is deleted with its
/// bindings and values, except a variable the library also declares with the same kind: it stays
/// bound to its objects, so the library's data sets change them (#4136; a template's bound
/// variables take a library of the same names:
/// <https://community.adobe.com/questions-652/illustrator-error-with-csv-files-812184>). The
/// library's other variables are then defined with their kinds, unbound, and its data sets are
/// added.
pub(super) fn load_library(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.loadLibrary";
    let bytes = library_bytes(p, C)?;
    let xml = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).map_err(|_| bad(C, "the variable library is not UTF-8"))?;
    let lib = parse_library(xml, C, false)?;
    let vars = &s.doc()?.doc.variables;
    let kept = |v: &Variable| lib.variables.iter().any(|l| l.name == v.name && l.kind == v.kind);
    let deleted: Vec<String> = vars.variables.iter().filter(|v| !kept(v)).map(|v| v.name.clone()).collect();
    let deleted_sets: Vec<String> = vars.datasets.iter().map(|d| d.name.clone()).collect();
    let added: Vec<Variable> = lib
        .variables
        .iter()
        .filter(|l| !vars.variables.iter().any(|v| kept(v) && v.name == l.name))
        .map(|l| Variable { name: l.name.clone(), kind: l.kind })
        .collect();
    // Lock Variables refuses New Variable and Delete Variable, so the load is refused when it would
    // delete or define a variable.
    if !deleted.is_empty() || !added.is_empty() {
        variables_unlocked(s).map_err(|why| EngineError::Disabled(C.into(), why))?;
    }
    let names: Vec<String> = lib.datasets.iter().map(|d| d.name.clone()).collect();
    // A load that leaves the variables and data sets as they were adds no undo step.
    super::edit_variables(s, "Load Variable Library", |vars| {
        vars.prune(&deleted);
        vars.variables.extend(added);
        vars.datasets = lib.datasets;
        vars.active_dataset = None;
        Ok(())
    })?;
    // A highlight of a variable that is gone highlights nothing.
    let st = s.doc_mut()?;
    st.drop_stale_variables_highlight();
    st.revision += 1;
    let declared: Vec<&str> = lib.variables.iter().map(|l| l.name.as_str()).collect();
    Ok(json!({"variables": declared, "deleted": deleted, "deletedDataSets": deleted_sets, "names": names}))
}

/// The warning about variables that aren't text and were left out, given the kind of each: every
/// kind once, in the order it first comes, with how many variables have it. `None` without any.
fn left_out_warning<'a>(kinds: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut counted: Vec<(&str, usize)> = Vec::new();
    for t in kinds {
        match counted.iter_mut().find(|(k, _)| *k == t) {
            Some((_, n)) => *n += 1,
            None => counted.push((t, 1)),
        }
    }
    if counted.is_empty() {
        return None;
    }
    let kinds = counted.iter().map(|(k, n)| format!("{k} ({n})")).collect::<Vec<_>>().join(", ");
    Some(format!("variables that aren't text were left out: {kinds}"))
}

/// Whether `name` can name an element: an XML name without a colon (XML 1.0, productions 4, 4a and
/// 5). A data set holds a text variable's value in an element named after the variable.
fn element_name(name: &str) -> bool {
    let start = |c: char| {
        matches!(c, 'A'..='Z' | '_' | 'a'..='z' | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}'
            | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
    };
    let more = |c: char| start(c) || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}');
    let mut chars = name.chars();
    chars.next().is_some_and(start) && chars.all(more)
}

/// Whether XML can hold `c` (XML 1.0, production 2): tab, line feed, carriage return, and the
/// characters from U+0020 on except U+FFFE and U+FFFF.
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

/// `s` escaped for XML text, or with `attribute` for a quoted attribute value. `&`, `<` and `>`
/// become entity references, and so do quotes in an attribute value. A carriage return, and a tab
/// or line feed in an attribute value, become character references, which the parser reads back
/// as they are.
fn escaped(s: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_escaped(&mut out, c, attribute);
    }
    out
}

/// `c` added to `out`, escaped as [`escaped`] says.
fn push_escaped(out: &mut String, c: char, attribute: bool) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '\r' => out.push_str("&#13;"),
        '"' if attribute => out.push_str("&quot;"),
        '\'' if attribute => out.push_str("&apos;"),
        '\t' if attribute => out.push_str("&#9;"),
        '\n' if attribute => out.push_str("&#10;"),
        c => out.push(c),
    }
}

/// Add the graph value `csv` of the variable `var` to `xml` as the `data` element [`graph_value`]
/// reads: the CSV read as Graph Data reads it, a `propertyRow` with a blank corner cell and the
/// series names when it names series, and a `row` per category, its name in a first `value` with
/// `key="name"` (blank without categories) and then a `value` per series, blank where the row has
/// none. A label that isn't a number is marked `type="string"`, and so is a blank category. CSV
/// that names series and has no rows is written as the `propertyRow` alone. Refused for CSV with
/// neither rows nor series, which reads back as no value, for more than [`MAX_GRAPH_CATEGORIES`]
/// rows or [`MAX_GRAPH_SERIES`] series (what a graph holds), when a label holds a character XML
/// can't hold, and when `xml` grows past `room` bytes.
fn push_graph(xml: &mut String, var: &str, csv: &str, room: usize) -> std::result::Result<(), GraphRefusal> {
    let (series, categories, rows) = super::super::graph::parse_csv(csv);
    if rows.is_empty() && series.is_empty() {
        return Err(GraphRefusal::Empty);
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0).max(series.len());
    if rows.len() > MAX_GRAPH_CATEGORIES || width > MAX_GRAPH_SERIES {
        return Err(GraphRefusal::TooMany);
    }
    if !series.iter().chain(&categories).all(|l| l.chars().all(xml_char)) {
        return Err(GraphRefusal::Control);
    }
    let labelled = !categories.is_empty();
    let label = |l: &str, category: bool| {
        let text = !l.is_empty() && l.trim().parse::<f64>().is_err();
        let blank_category = category && labelled && l.is_empty();
        let key = if category { " key=\"name\"" } else { "" };
        let string = if text || blank_category { " type=\"string\"" } else { "" };
        format!("<value{key}{string}>{}</value>", escaped(l, false))
    };
    let _ = writeln!(xml, "\t\t\t\t<{var}>\n\t\t\t\t\t<data numDataColumns=\"{}\">", width + usize::from(labelled));
    if !series.is_empty() {
        let names: String = (0..width).map(|s| label(series.get(s).map(String::as_str).unwrap_or_default(), false)).collect();
        let _ = writeln!(xml, "\t\t\t\t\t\t<propertyRow key=\"name\"><value></value>{names}</propertyRow>");
    }
    xml.push_str("\t\t\t\t\t\t<values>\n");
    for (i, row) in rows.iter().enumerate() {
        let name = label(categories.get(i).map(String::as_str).unwrap_or_default(), true);
        let values: String = (0..width)
            .map(|s| match row.get(s).copied().flatten() {
                Some(v) => format!("<value>{v}</value>"),
                None => "<value></value>".to_string(),
            })
            .collect();
        let _ = writeln!(xml, "\t\t\t\t\t\t\t<row>{name}{values}</row>");
        if xml.len() > room {
            return Err(GraphRefusal::TooLarge);
        }
    }
    let _ = writeln!(xml, "\t\t\t\t\t\t</values>\n\t\t\t\t\t</data>\n\t\t\t\t</{var}>");
    Ok(())
}

/// What a graph holds at most, for the errors about graph data past it.
fn graph_limits() -> String {
    format!("more than {MAX_GRAPH_CATEGORIES} rows or {MAX_GRAPH_SERIES} series, which a graph doesn't hold")
}

/// Why [`push_graph`] refused a graph value.
enum GraphRefusal {
    /// A label holds a character XML can't hold.
    Control,
    /// The data has more rows or series than a graph holds.
    TooMany,
    /// The data has neither rows nor series.
    Empty,
    /// The library grew past the room it has.
    TooLarge,
}

/// Save Variable Library: the document's variables and its data sets as a variable library in the
/// layout [`parse_library`] reads, so Load Variable Library reads back the same variables with their
/// kinds, data set names and values. Each variable is declared with the `trait` and `category` of
/// its kind ([`kind_trait`]), and a variable of unknown kind with neither. A text value is written
/// as one `p` element per line, a visibility value as `true` or `false`, an image value as
/// [`library_file`] writes its path, and a graph value as a `data` element ([`push_graph`]). A
/// variable whose name can't name an element ([`element_name`]; a variable of unknown kind has no
/// values, so no element is named after it), a name or value that holds a character XML can't
/// hold, a name that Load Variable Library would read differently or refuse (white space at either
/// end, a name longer than `MAX_NAME` characters, a name used twice), an image value that isn't a
/// file's absolute path, and a library past the caps Load Variable Library reads are refused. With
/// `path` the library is written to the file; without it the reply carries it as `xml`.
pub(super) fn save_library(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.saveLibrary";
    let vars = &s.doc()?.doc.variables;
    if vars.variables.is_empty() {
        return Err(bad(C, "define a variable first"));
    }
    let all: Vec<&Variable> = vars.variables.iter().collect();
    if let Some(name) = all.iter().filter(|v| v.kind != VariableKind::Unknown).map(|v| &v.name).find(|n| !element_name(n)) {
        return Err(bad(
            C,
            format!(
                "variable `{name}` can't be saved in a variable library: rename it to start with a letter or `_`, without spaces or symbols other than `-`, `_` and `.`"
            ),
        ));
    }
    // Load Variable Library reads a name trimmed and refuses one longer than `MAX_NAME` characters
    // or declared twice.
    let mut declared = BTreeSet::new();
    for v in &all {
        if named(Some(&v.name), "varName", C)? != v.name || !v.name.chars().all(xml_char) {
            return Err(bad(
                C,
                format!("variable `{}` starts or ends with white space or holds a control character, which a variable library can't hold", v.name),
            ));
        }
        if !declared.insert(v.name.as_str()) {
            return Err(bad(C, format!("variable `{}` is already defined", v.name)));
        }
    }
    if all.len() > MAX_LIBRARY_VARIABLES {
        return Err(bad(C, format!("the variable library declares more than {MAX_LIBRARY_VARIABLES} variables")));
    }
    if vars.datasets.len() > MAX_LIBRARY_DATASETS {
        return Err(bad(C, format!("the variable library declares more than {MAX_LIBRARY_DATASETS} data sets")));
    }
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [\n\
         \t<!ENTITY ns_vars \"{NS_VARIABLES}\">\n\
         \t<!ENTITY ns_custom \"{NS_CUSTOM}\">\n\
         \t<!ENTITY ns_flows \"{NS_FLOWS}\">\n\
         \t<!ENTITY ns_graphs \"{NS_GRAPHS}\">\n\
         ]>\n\
         <svg>\n\
         <variableSets xmlns=\"&ns_vars;\">\n\
         \t<variableSet varSetName=\"binding1\" locked=\"none\">\n\
         \t\t<variables>\n"
    );
    for v in &all {
        let name = escaped(&v.name, true);
        match kind_trait(v.kind) {
            Some((t, category)) => xml += &format!("\t\t\t<variable varName=\"{name}\" trait=\"{t}\" category=\"{category}\"></variable>\n"),
            None => xml += &format!("\t\t\t<variable varName=\"{name}\"></variable>\n"),
        }
    }
    xml += "\t\t</variables>\n\t\t<v:sampleDataSets xmlns:v=\"&ns_vars;\" xmlns=\"&ns_custom;\">\n";
    // Load Variable Library caps the file and the text its entity references expand to: each
    // variable's `category`, two `&ns_vars;` and one `&ns_custom;`. The library stays within the
    // cap less what the references add.
    let grows = |value: &str, reference: &str| value.len().saturating_sub(reference.len());
    let growth = all
        .iter()
        .filter_map(|v| kind_trait(v.kind))
        .map(|(_, category)| match category {
            "&ns_flows;" => grows(NS_FLOWS, category),
            "&ns_graphs;" => grows(NS_GRAPHS, category),
            _ => grows(NS_VARIABLES, category),
        })
        .sum::<usize>()
        + 2 * grows(NS_VARIABLES, "&ns_vars;")
        + grows(NS_CUSTOM, "&ns_custom;");
    let room = MAX_LIBRARY_BYTES.saturating_sub(growth);
    let too_large = || bad(C, format!("the variable library is larger than {MAX_LIBRARY_BYTES} bytes"));
    let mut seen = BTreeSet::new();
    for ds in &vars.datasets {
        let name = &ds.name;
        // Load Variable Library reads a data set's name trimmed and at most `MAX_NAME` characters
        // long ([`named`]), and refuses a name declared twice.
        if named(Some(name), "dataSetName", C)? != *name {
            return Err(bad(C, format!("data set `{name}` starts or ends with white space, which a variable library can't hold")));
        }
        if !seen.insert(name.as_str()) {
            return Err(bad(C, format!("dataset `{name}` already exists")));
        }
        let control = || bad(C, format!("data set `{name}` holds a control character, which a variable library can't hold"));
        let mut text_values = all.iter().filter_map(|v| match ds.values.get(&v.name) {
            Some(DataValue::Text(t)) if v.kind == VariableKind::Text => Some(t),
            _ => None,
        });
        if !name.chars().all(xml_char) || text_values.any(|t| !t.chars().all(xml_char)) {
            return Err(control());
        }
        xml += &format!("\t\t\t<v:sampleDataSet dataSetName=\"{}\">\n", escaped(name, true));
        for v in &all {
            let Some(value) = ds.values.get(&v.name).filter(|value| value.fits(v.kind)) else { continue };
            let var = &v.name;
            match value {
                DataValue::Text(t) => {
                    xml += &format!("\t\t\t\t<{var}>\n");
                    for line in t.split('\n') {
                        xml += "\t\t\t\t\t<p>";
                        // A value can be larger than the cap, so the text stops growing once it passes it.
                        for c in line.chars() {
                            push_escaped(&mut xml, c, false);
                            if xml.len() > room {
                                return Err(too_large());
                            }
                        }
                        xml += "</p>\n";
                    }
                    xml += &format!("\t\t\t\t</{var}>\n");
                }
                DataValue::Visible(shown) => xml += &format!("\t\t\t\t<{var}>{shown}</{var}>\n"),
                DataValue::Image(path) => {
                    // Load Variable Library reads back only a file's absolute path.
                    let file = library_file(path);
                    if library_path(file.trim_matches(XML_SPACE)).as_deref() != Some(path.as_str()) {
                        return Err(bad(C, format!("data set `{name}` holds an image value for `{var}` that isn't a file's absolute path")));
                    }
                    if !file.chars().all(xml_char) {
                        return Err(control());
                    }
                    xml += &format!("\t\t\t\t<{var}>{}</{var}>\n", escaped(&file, false));
                }
                DataValue::Graph(csv) => match push_graph(&mut xml, var, csv, room) {
                    Ok(()) => {}
                    Err(GraphRefusal::Control) => return Err(control()),
                    Err(GraphRefusal::TooMany) => {
                        return Err(bad(C, format!("data set `{name}` holds a graph value for `{var}` with {}", graph_limits())));
                    }
                    Err(GraphRefusal::Empty) => {
                        return Err(bad(C, format!("data set `{name}` holds a graph value for `{var}` without rows or series")));
                    }
                    Err(GraphRefusal::TooLarge) => return Err(too_large()),
                },
            }
            if xml.len() > room {
                return Err(too_large());
            }
        }
        xml += "\t\t\t</v:sampleDataSet>\n";
        if xml.len() > room {
            return Err(too_large());
        }
    }
    xml += "\t\t</v:sampleDataSets>\n\t</variableSet>\n</variableSets>\n</svg>\n";
    if xml.len() > room {
        return Err(too_large());
    }
    let mut out = json!({"variables": all.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), "datasets": vars.datasets.len()});
    match str_param(p, "path") {
        Some(path) => {
            let path = absolute_path(path);
            write_file(&path, xml.as_bytes())?;
            out["path"] = json!(path);
        }
        None => out["xml"] = json!(xml),
    }
    Ok(out)
}
