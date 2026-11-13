use std::collections::HashMap;

use super::{
    BTreeMap, BTreeSet, DataSet, DataValue, Document, EngineError, MAX_NAME, NodeId, NodeKind, Result, Session, Value, Variable, VariableKind,
    Variables, absolute_path, bad, json, named, read_file_head, str_param, variables_unlocked, write_file,
};

/// The namespace of a variable library's elements, the default namespace of its data sets' values,
/// and the `category` of its text variables.
const NS_VARIABLES: &str = "http://ns.adobe.com/Variables/1.0/"; // brand-ok: the namespace variable libraries use

const NS_CUSTOM: &str = "http://ns.adobe.com/GenericCustomNamespace/1.0/"; // brand-ok: the namespace variable libraries use

const NS_FLOWS: &str = "http://ns.adobe.com/Flows/1.0/"; // brand-ok: the namespace variable libraries use

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
/// any other document type declaration, so [`nesting`](super::place::docx::nesting) counts the
/// elements the parser reads.
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

/// A variable library as Load Variable Library reads it: the names of its text variables in the
/// order it declares them, the name and `trait` of each variable of another kind, its data sets
/// with their text values, and the element id each text variable's `docRef` names.
pub(crate) struct Library {
    text: Vec<String>,
    other: Vec<(String, String)>,
    datasets: Vec<DataSet>,
    refs: Vec<(String, String)>,
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
/// A variable whose `trait` is `textcontent` is a text variable. In a data set, an element named
/// after a text variable gives its value: the text of its `p` elements, one paragraph each, and
/// none when it has no `p` element. Values for variables of other kinds are left out. A variable
/// or data set declared twice, a value for a variable the library doesn't declare, and a library
/// past the caps are refused. Entities that its document type declaration defines are read as
/// [`without_doctype`] says.
fn parse_library(xml: &str, cmd: &str) -> Result<Library> {
    let xml = without_doctype(xml, cmd)?;
    // roxmltree parses nested elements recursively.
    if crate::cmd::place::docx::nesting(&xml) > MAX_LIBRARY_NESTING {
        return Err(bad(cmd, format!("the variable library nests elements more than {MAX_LIBRARY_NESTING} deep")));
    }
    let opts = roxmltree::ParsingOptions { nodes_limit: MAX_LIBRARY_NODES, ..roxmltree::ParsingOptions::default() };
    let doc =
        roxmltree::Document::parse_with_options(&xml, opts).map_err(|e| bad(cmd, format!("the variable library is not well-formed XML: {e}")))?;
    let is = |n: &roxmltree::Node, name: &str| n.is_element() && n.tag_name().name() == name;
    let Some(root) = doc.descendants().find(|n| is(n, "variableSets")) else {
        return Err(bad(cmd, "the file holds no `variableSets` element"));
    };
    let sets: Vec<roxmltree::Node> = root.children().filter(|n| is(n, "variableSet")).collect();
    let (mut text, mut other, mut declared, mut refs) = (Vec::new(), Vec::new(), BTreeSet::new(), Vec::new());
    for var in sets.iter().flat_map(|s| s.children()).filter(|n| is(n, "variables")).flat_map(|v| v.children()).filter(|n| is(n, "variable")) {
        let name = named(var.attribute("varName"), "varName", cmd)?;
        if !declared.insert(name.clone()) {
            return Err(bad(cmd, format!("variable `{name}` is declared twice")));
        }
        if declared.len() > MAX_LIBRARY_VARIABLES {
            return Err(bad(cmd, format!("the variable library declares more than {MAX_LIBRARY_VARIABLES} variables")));
        }
        match var.attribute("trait") {
            Some("textcontent") => {
                // `docRef="id('…')"` names the element of the variable's bound object.
                let element = var.attribute("docRef").and_then(|r| r.trim().strip_prefix("id('")?.strip_suffix("')")).filter(|e| !e.is_empty());
                if let Some(element) = element {
                    refs.push((name.clone(), element.to_string()));
                }
                text.push(name);
            }
            Some(t) => other.push((name, t.chars().take(MAX_NAME).collect())),
            None => return Err(bad(cmd, format!("variable `{name}` has no `trait`"))),
        }
    }
    if declared.is_empty() {
        return Err(bad(cmd, "the variable library declares no variables"));
    }
    let text_names: BTreeSet<&str> = text.iter().map(String::as_str).collect();
    let (mut datasets, mut seen) = (Vec::new(), BTreeSet::new());
    for row in
        sets.iter().flat_map(|s| s.children()).filter(|n| is(n, "sampleDataSets")).flat_map(|d| d.children()).filter(|n| is(n, "sampleDataSet"))
    {
        let name = named(row.attribute("dataSetName"), "dataSetName", cmd)?;
        if !seen.insert(name.clone()) {
            return Err(bad(cmd, format!("data set `{name}` is declared twice")));
        }
        if seen.len() > MAX_LIBRARY_DATASETS {
            return Err(bad(cmd, format!("the variable library declares more than {MAX_LIBRARY_DATASETS} data sets")));
        }
        let (mut values, mut held) = (BTreeMap::new(), BTreeSet::new());
        for el in row.children().filter(roxmltree::Node::is_element) {
            let var = el.tag_name().name();
            if !declared.contains(var) {
                return Err(bad(cmd, format!("data set `{name}` holds a value for `{var}`, which the library doesn't declare")));
            }
            if !held.insert(var) {
                return Err(bad(cmd, format!("data set `{name}` holds two values for `{var}`")));
            }
            if !text_names.contains(var) {
                continue;
            }
            let paras: Vec<String> = el
                .children()
                .filter(|n| is(n, "p"))
                .map(|p| p.descendants().filter(roxmltree::Node::is_text).filter_map(|t| t.text()).collect())
                .collect();
            if !paras.is_empty() {
                values.insert(var.to_string(), DataValue::Text(paras.join("\n")));
            }
        }
        datasets.push(DataSet { name, values });
    }
    Ok(Library { text, other, datasets, refs })
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
        parse_library(&format!("<svg>{element}</svg>"), C)
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
/// stays as the file draws it. The warning about the variables of other kinds that were left out,
/// if any.
pub(crate) fn give_svg_library(doc: &mut Document, lib: Library, found: &HashMap<String, NodeId>) -> Option<String> {
    let mut vars =
        Variables { variables: lib.text.iter().map(|n| Variable { name: n.clone(), kind: VariableKind::Text }).collect(), ..Variables::default() };
    vars.datasets = lib.datasets;
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
    left_out_warning(lib.other.iter().map(|(_, t)| t.as_str()))
}

/// Load Variable Library: replace the document's variables and data sets with a variable library's
/// ([`parse_library`]) in one undo step, or none when the document already holds what the library
/// gives. Every data set is deleted, and every variable of the document is deleted with its
/// bindings and values, except a text variable the library also declares: it stays bound to its
/// objects, so the library's data sets change them (#4136; a template's bound variables take a
/// library of the same names:
/// <https://community.adobe.com/questions-652/illustrator-error-with-csv-files-812184>). The
/// library's other text variables are then defined, unbound, and its data sets are added. The
/// library's variables of other kinds are left out, named in `skipped` and in a warning.
pub(super) fn load_library(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.loadLibrary";
    let bytes = library_bytes(p, C)?;
    let xml = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).map_err(|_| bad(C, "the variable library is not UTF-8"))?;
    let lib = parse_library(xml, C)?;
    let vars = &s.doc()?.doc.variables;
    let kept = |v: &Variable| v.kind == VariableKind::Text && lib.text.contains(&v.name);
    let deleted: Vec<String> = vars.variables.iter().filter(|v| !kept(v)).map(|v| v.name.clone()).collect();
    let deleted_sets: Vec<String> = vars.datasets.iter().map(|d| d.name.clone()).collect();
    let added: Vec<Variable> = lib
        .text
        .iter()
        .filter(|n| !vars.variables.iter().any(|v| kept(v) && &v.name == *n))
        .map(|n| Variable { name: n.clone(), kind: VariableKind::Text })
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
    let skipped: Vec<Value> = lib.other.iter().map(|(name, t)| json!({"name": name, "trait": t})).collect();
    let mut out = json!({
        "variables": lib.text, "deleted": deleted, "deletedDataSets": deleted_sets, "names": names, "skipped": skipped,
    });
    if let Some(warning) = left_out_warning(lib.other.iter().map(|(_, t)| t.as_str())) {
        out["warning"] = json!(warning);
    }
    Ok(out)
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

/// Save Variable Library: the document's text variables and its data sets as a variable library in
/// the layout [`parse_library`] reads, so Load Variable Library reads back the same text variables,
/// data set names and text values. A text value is written as one `p` element per line. Variables
/// of other kinds are left out with their values, named in `skipped` and in a warning. A text
/// variable whose name can't name an element ([`element_name`]), a data set name or text value that
/// holds a character XML can't hold, a name that Load Variable Library would read differently or
/// refuse (white space at either end of a data set name, a name longer than `MAX_NAME` characters,
/// a name used twice), and a library past the caps Load Variable Library reads are refused. With `path` the library is written to the file; without it the reply carries it as
/// `xml`.
pub(super) fn save_library(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.saveLibrary";
    let vars = &s.doc()?.doc.variables;
    if vars.variables.is_empty() {
        return Err(bad(C, "define a variable first"));
    }
    let (text, other): (Vec<&Variable>, Vec<&Variable>) = vars.variables.iter().partition(|v| v.kind == VariableKind::Text);
    if text.is_empty() {
        return Err(bad(C, "a variable library holds text variables, and the document has none"));
    }
    if let Some(name) = text.iter().map(|v| &v.name).find(|n| !element_name(n)) {
        return Err(bad(
            C,
            format!(
                "variable `{name}` can't be saved in a variable library: rename it to start with a letter or `_`, without spaces or symbols other than `-`, `_` and `.`"
            ),
        ));
    }
    // Load Variable Library refuses a name longer than `MAX_NAME` characters and a name declared
    // twice.
    let mut declared = BTreeSet::new();
    for v in &text {
        named(Some(&v.name), "varName", C)?;
        if !declared.insert(v.name.as_str()) {
            return Err(bad(C, format!("variable `{}` is already defined", v.name)));
        }
    }
    if text.len() > MAX_LIBRARY_VARIABLES {
        return Err(bad(C, format!("the variable library declares more than {MAX_LIBRARY_VARIABLES} variables")));
    }
    if vars.datasets.len() > MAX_LIBRARY_DATASETS {
        return Err(bad(C, format!("the variable library declares more than {MAX_LIBRARY_DATASETS} data sets")));
    }
    fn text_value<'a>(ds: &'a DataSet, var: &str) -> Option<&'a str> {
        match ds.values.get(var) {
            Some(DataValue::Text(t)) => Some(t.as_str()),
            _ => None,
        }
    }
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [\n\
         \t<!ENTITY ns_vars \"{NS_VARIABLES}\">\n\
         \t<!ENTITY ns_custom \"{NS_CUSTOM}\">\n\
         \t<!ENTITY ns_flows \"{NS_FLOWS}\">\n\
         ]>\n\
         <svg>\n\
         <variableSets xmlns=\"&ns_vars;\">\n\
         \t<variableSet varSetName=\"binding1\" locked=\"none\">\n\
         \t\t<variables>\n"
    );
    for v in &text {
        xml += &format!("\t\t\t<variable varName=\"{}\" trait=\"textcontent\" category=\"&ns_flows;\"></variable>\n", v.name);
    }
    xml += "\t\t</variables>\n\t\t<v:sampleDataSets xmlns:v=\"&ns_vars;\" xmlns=\"&ns_custom;\">\n";
    // Load Variable Library caps the file and the text its entity references expand to: one
    // `&ns_flows;` per variable, two `&ns_vars;` and one `&ns_custom;`. The library stays within
    // the cap less what the references add.
    let grows = |value: &str, reference: &str| value.len().saturating_sub(reference.len());
    let growth = text.len() * grows(NS_FLOWS, "&ns_flows;") + 2 * grows(NS_VARIABLES, "&ns_vars;") + grows(NS_CUSTOM, "&ns_custom;");
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
        if !name.chars().all(xml_char) || text.iter().filter_map(|v| text_value(ds, &v.name)).any(|t| !t.chars().all(xml_char)) {
            return Err(bad(C, format!("data set `{name}` holds a control character, which a variable library can't hold")));
        }
        xml += &format!("\t\t\t<v:sampleDataSet dataSetName=\"{}\">\n", escaped(name, true));
        for v in &text {
            let Some(t) = text_value(ds, &v.name) else { continue };
            xml += &format!("\t\t\t\t<{}>\n", v.name);
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
            xml += &format!("\t\t\t\t</{}>\n", v.name);
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
    let skipped: Vec<Value> = other.iter().map(|v| json!({"name": v.name, "kind": v.kind.label()})).collect();
    let mut out = json!({
        "variables": text.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), "datasets": vars.datasets.len(), "skipped": skipped,
    });
    if let Some(warning) = left_out_warning(other.iter().map(|v| v.kind.label())) {
        out["warning"] = json!(warning);
    }
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
