use super::{
    BTreeMap, BTreeSet, DataSet, DataValue, EngineError, MAX_NAME, Result, Session, Value, Variable, VariableKind, absolute_path, bad, json, named,
    read_file_head, str_param, variables_unlocked,
};

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
/// order it declares them, the name and `trait` of each variable of another kind, and its data
/// sets with their text values.
struct Library {
    text: Vec<String>,
    other: Vec<(String, String)>,
    datasets: Vec<DataSet>,
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
    let (mut text, mut other, mut declared) = (Vec::new(), Vec::new(), BTreeSet::new());
    for var in sets.iter().flat_map(|s| s.children()).filter(|n| is(n, "variables")).flat_map(|v| v.children()).filter(|n| is(n, "variable")) {
        let name = named(var.attribute("varName"), "varName", cmd)?;
        if !declared.insert(name.clone()) {
            return Err(bad(cmd, format!("variable `{name}` is declared twice")));
        }
        if declared.len() > MAX_LIBRARY_VARIABLES {
            return Err(bad(cmd, format!("the variable library declares more than {MAX_LIBRARY_VARIABLES} variables")));
        }
        match var.attribute("trait") {
            Some("textcontent") => text.push(name),
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
    Ok(Library { text, other, datasets })
}

/// Load Variable Library: replace the document's variables and data sets with a variable library's
/// ([`parse_library`]) in one undo step. Every variable of the document is deleted with its bindings
/// and values, and every data set is deleted. The library's text variables are then defined,
/// unbound, and its data sets are added. The library's variables of other kinds are left out, named
/// in `skipped` and in a warning.
pub(super) fn load_library(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "variable.loadLibrary";
    let bytes = library_bytes(p, C)?;
    let xml = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).map_err(|_| bad(C, "the variable library is not UTF-8"))?;
    let lib = parse_library(xml, C)?;
    let vars = &s.doc()?.doc.variables;
    let deleted: Vec<String> = vars.variables.iter().map(|v| v.name.clone()).collect();
    let deleted_sets: Vec<String> = vars.datasets.iter().map(|d| d.name.clone()).collect();
    // Lock Variables refuses New Variable and Delete Variable, so the load is refused when it would
    // delete or define a variable.
    if !deleted.is_empty() || !lib.text.is_empty() {
        variables_unlocked(s).map_err(|why| EngineError::Disabled(C.into(), why))?;
    }
    let added: Vec<Variable> = lib.text.iter().map(|n| Variable { name: n.clone(), kind: VariableKind::Text }).collect();
    let names: Vec<String> = lib.datasets.iter().map(|d| d.name.clone()).collect();
    s.edit("Load Variable Library", |d, _| {
        d.variables.prune(&deleted);
        d.variables.variables.extend(added);
        d.variables.datasets = lib.datasets;
        d.variables.active_dataset = None;
        Ok(())
    })?;
    // A highlight of a variable that is gone highlights nothing.
    let st = s.doc_mut()?;
    if st.variables_highlight.as_ref().is_some_and(|h| st.doc.variables.variable(h).is_none()) {
        st.variables_highlight = None;
    }
    st.revision += 1;
    let skipped: Vec<Value> = lib.other.iter().map(|(name, t)| json!({"name": name, "trait": t})).collect();
    let mut out = json!({
        "variables": lib.text, "deleted": deleted, "deletedDataSets": deleted_sets, "names": names, "skipped": skipped,
    });
    // Each `trait` once, in the order the library first uses it, with how many variables have it.
    let mut kinds: Vec<(&str, usize)> = Vec::new();
    for (_, t) in &lib.other {
        match kinds.iter_mut().find(|(k, _)| k == t) {
            Some((_, n)) => *n += 1,
            None => kinds.push((t, 1)),
        }
    }
    if !kinds.is_empty() {
        let kinds = kinds.iter().map(|(k, n)| format!("{k} ({n})")).collect::<Vec<_>>().join(", ");
        out["warning"] = json!(format!("variables that aren't text were left out: {kinds}"));
    }
    Ok(out)
}
