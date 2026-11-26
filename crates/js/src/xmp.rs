//! XMP metadata for the scripts' `XMPMeta` and `XMPFile` ([`crate::xmp_classes`]): a packet read
//! into the list of its top-level properties and written back as RDF/XML, as the XMP Specification
//! Part 1 (ISO 16684-1,
//! <https://raw.githubusercontent.com/adobe/XMP-Toolkit-SDK/main/docs/XMPSpecificationPart1.pdf>)
//! describes the serialization.
//!
//! A packet is read with `craft_xml`, which refuses a document type declaration and every entity
//! reference other than the predefined and numeric ones, and bounds the input, the nesting, the
//! nodes and the attributes. The `rdf:Description` elements under `rdf:RDF` give the properties:
//! a simple value (text, or a URI in `rdf:resource`), an array of simple items (`rdf:Bag`,
//! `rdf:Seq` or `rdf:Alt`, each item with an optional `xml:lang`), or any other value (a structure,
//! a value with qualifiers, an array of structures), which is kept as the XML it was written in and
//! written back with the namespace declarations it uses; a value with qualifiers in the `rdf:value`
//! form gives the text and the options of its `rdf:value`. The namespaces of the property, element and
//! attribute names under `rdf:Description` are added to the realm's [`Registry`] with the prefix the
//! packet gives them, or a new prefix when that one is taken; a namespace the packet declares and no
//! such name uses is not added.
//!
//! VectorCraft's own rules: a packet or a written packet holds at most [`MAX_BYTES`] bytes,
//! elements nest at most 64 deep, and each holds at most 1,024 attributes and has at most 1,024
//! namespace declarations in scope, both as read and as written; a
//! child of `rdf:RDF` other than `rdf:Description` and text beside the descriptions or their
//! properties make the packet malformed, a property given twice keeps its first value, and
//! [`find_packet`] finds only a UTF-8 packet. A realm registers at most [`MAX_NAMESPACES`] namespaces holding at most
//! [`MAX_REGISTRY_BYTES`] bytes, each URI at most [`MAX_URI_BYTES`] bytes and each prefix at most
//! [`MAX_PREFIX_BYTES`] bytes.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use craft_xml::{Doctype, El, Limits, Names, Node, Options, UnknownEntity, XML_NS};

/// The RDF namespace.
pub(crate) const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// The namespace of the `x:xmpmeta` element around a packet's `rdf:RDF`.
const X: &str = "adobe:ns:meta/"; // brand-ok: the XMP namespace the files use
/// The Dublin Core namespace.
pub(crate) const DC: &str = "http://purl.org/dc/elements/1.1/";
/// The XMP basic namespace.
pub(crate) const XMP: &str = "http://ns.adobe.com/xap/1.0/"; // brand-ok: the XMP namespace the files use
/// The XMP rights namespace.
pub(crate) const XMP_RIGHTS: &str = "http://ns.adobe.com/xap/1.0/rights/"; // brand-ok: the XMP namespace the files use
/// The PDF namespace.
pub(crate) const PDF: &str = "http://ns.adobe.com/pdf/1.3/"; // brand-ok: the XMP namespace the files use
/// The namespace of the image editor's properties.
pub(crate) const PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/"; // brand-ok: the XMP namespace the files use
/// The XML namespace.
pub(crate) const XML: &str = XML_NS;
/// The namespace of the `xmlns` attributes, which no prefix may name.
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

/// The most bytes of a packet read or written.
pub const MAX_BYTES: usize = 8 << 20;
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 1 << 20;
const MAX_ATTRS: usize = 1024;
/// The most namespaces a realm registers.
const MAX_NAMESPACES: usize = 4096;
/// The most bytes of a namespace URI a realm registers.
pub(crate) const MAX_URI_BYTES: usize = 1024;
/// The most bytes of a namespace prefix a realm registers, a `_n_` suffix included.
pub(crate) const MAX_PREFIX_BYTES: usize = 128;
/// The most bytes a realm's namespaces hold, counted by [`Registry::bytes`].
pub(crate) const MAX_REGISTRY_BYTES: usize = 1 << 20;
/// The bytes counted for each namespace besides its URI and prefix.
const ENTRY_BYTES: usize = 96;
/// The `id` of a packet wrapper (XMP Specification Part 1, 7.3.2).
const WRAPPER_ID: &str = "W5M0MpCehiHzreSzNTczkc9d";
/// The most bytes [`find_packet`] reads of a packet header, from its `<?xpacket begin=` to its
/// `?>`.
const MAX_HEADER: usize = 256;

/// The property is a URI (`rdf:resource`).
pub(crate) const PROP_VALUE_IS_URI: u32 = 0x2;
/// The property has qualifiers.
const PROP_HAS_QUALIFIERS: u32 = 0x10;
/// The property has an `xml:lang` qualifier.
const PROP_HAS_LANG: u32 = 0x40;
/// The property is a structure.
pub(crate) const PROP_IS_STRUCT: u32 = 0x100;
/// The property is an array.
pub(crate) const PROP_IS_ARRAY: u32 = 0x200;
/// The array's order matters (`rdf:Seq`).
pub(crate) const ARRAY_IS_ORDERED: u32 = 0x400;
/// The array's items are alternatives (`rdf:Alt`).
pub(crate) const ARRAY_IS_ALTERNATIVE: u32 = 0x800;
/// The array's items are text in several languages.
pub(crate) const ARRAY_IS_ALTTEXT: u32 = 0x1000;

/// Why XMP could not be read, written or changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XmpError {
    /// The packet is not well-formed XML, or not XMP.
    Malformed,
    /// The packet, or the packet written, is longer than [`MAX_BYTES`], or a namespace URI is longer
    /// than [`MAX_URI_BYTES`] or its prefix longer than [`MAX_PREFIX_BYTES`].
    TooLarge,
    /// The realm already has [`MAX_NAMESPACES`] namespaces, or one more would take its namespaces
    /// over [`MAX_REGISTRY_BYTES`].
    Namespaces,
    /// A name that is not an XML name without a colon, or a prefix that names another namespace.
    BadName,
    /// Text with a character XML does not allow.
    BadText,
    /// A path form not read yet: a structure field, a qualifier or a selector.
    NotYet,
    /// The metadata a packet gives is larger than the budget [`parse_within`] was given.
    Heap,
}

/// The kind of an array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    Bag,
    Seq,
    Alt,
}

impl Form {
    fn element(self) -> &'static str {
        match self {
            Form::Bag => "rdf:Bag",
            Form::Seq => "rdf:Seq",
            Form::Alt => "rdf:Alt",
        }
    }

    /// The array's options, as `XMPProperty.options` gives them.
    fn options(self) -> u32 {
        match self {
            Form::Bag => PROP_IS_ARRAY,
            Form::Seq => PROP_IS_ARRAY | ARRAY_IS_ORDERED,
            Form::Alt => PROP_IS_ARRAY | ARRAY_IS_ORDERED | ARRAY_IS_ALTERNATIVE,
        }
    }

    /// The form the options of a new array give: `Alt` with [`ARRAY_IS_ALTERNATIVE`] or
    /// [`ARRAY_IS_ALTTEXT`], `Seq` with [`ARRAY_IS_ORDERED`], else `Bag`.
    pub(crate) fn from_options(options: u32) -> Form {
        if options & (ARRAY_IS_ALTERNATIVE | ARRAY_IS_ALTTEXT) != 0 {
            Form::Alt
        } else if options & ARRAY_IS_ORDERED != 0 {
            Form::Seq
        } else {
            Form::Bag
        }
    }
}

/// An item of an array of simple values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) text: String,
    pub(crate) lang: Option<String>,
}

impl Item {
    /// Does the item name a language? `xml:lang=""` names none.
    pub(crate) fn has_lang(&self) -> bool {
        self.lang.as_deref().is_some_and(|l| !l.is_empty())
    }

    /// The options `XMPProperty.options` gives for the item: an `xml:lang` qualifier, also an empty
    /// one, is a qualifier and a language, as on a property.
    pub(crate) fn options(&self) -> u32 {
        if self.lang.is_some() { PROP_HAS_QUALIFIERS | PROP_HAS_LANG } else { 0 }
    }
}

/// A property's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Simple {
        text: String,
        uri: bool,
    },
    Array {
        form: Form,
        items: Vec<Item>,
    },
    /// A value kept as the XML element it was read as, with the namespace declarations it uses, its
    /// options and its text when it has one.
    Other {
        xml: String,
        /// Validation counts the expanded XML without retaining it.
        expanded_bytes: usize,
        options: u32,
        text: Option<String>,
        /// The most namespace declarations `xml` puts in scope at one of its elements.
        decls: usize,
    },
}

impl Value {
    /// The options `XMPProperty.options` gives.
    pub(crate) fn options(&self) -> u32 {
        match self {
            Value::Simple { uri, .. } => {
                if *uri {
                    PROP_VALUE_IS_URI
                } else {
                    0
                }
            }
            Value::Array { form, items } => {
                let alt_text = *form == Form::Alt && !items.is_empty() && items.iter().all(Item::has_lang);
                form.options() | if alt_text { ARRAY_IS_ALTTEXT } else { 0 }
            }
            Value::Other { options, .. } => *options,
        }
    }

    /// The text `XMPProperty.value` gives; `None` for an array or a structure.
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Value::Simple { text, .. } => Some(text),
            Value::Array { .. } => None,
            Value::Other { text, .. } => text.as_deref(),
        }
    }
}

/// A top-level property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Prop {
    /// The namespace, which the properties a packet gives in the same namespace share.
    pub(crate) ns: Rc<str>,
    pub(crate) name: String,
    pub(crate) value: Value,
}

impl Prop {
    /// The estimated bytes the property holds.
    pub(crate) fn heap_bytes(&self) -> usize {
        let value = match &self.value {
            Value::Simple { text, .. } => text.len(),
            Value::Array { items, .. } => items.iter().map(|i| 48 + i.text.len() + i.lang.as_ref().map_or(0, String::len)).sum(),
            Value::Other { expanded_bytes, text, .. } => expanded_bytes + text.as_ref().map_or(0, String::len),
        };
        96 + self.ns.len() + self.name.len() + value
    }
}

/// The metadata of one `XMPMeta`: its `rdf:about` and its properties, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Meta {
    pub(crate) about: String,
    pub(crate) props: Vec<Prop>,
}

impl Meta {
    /// The estimated bytes the metadata holds.
    pub(crate) fn heap_bytes(&self) -> usize {
        32 + self.about.len() + self.props.iter().map(Prop::heap_bytes).sum::<usize>()
    }

    /// The position of the property `name` in the namespace `ns`.
    pub(crate) fn find(&self, ns: &str, name: &str) -> Option<usize> {
        self.props.iter().position(|p| &*p.ns == ns && p.name == name)
    }
}

/// The namespaces a realm knows, each with its prefix (without the colon), in the order they were
/// registered.
#[derive(Clone, Debug)]
pub(crate) struct Registry {
    /// Prefix by namespace, and namespace by prefix: each namespace has one prefix and each prefix
    /// names one namespace.
    prefixes: HashMap<String, String>,
    uris: HashMap<String, String>,
    /// The bytes the namespaces hold, as [`entry_bytes`] counts them.
    bytes: usize,
}

/// The bytes counted for the namespace `uri` with the prefix `prefix`: both strings are kept twice.
fn entry_bytes(uri: &str, prefix: &str) -> usize {
    ENTRY_BYTES + 2 * (uri.len() + prefix.len())
}

impl Default for Registry {
    fn default() -> Self {
        let standard =
            [("rdf", RDF), ("x", X), ("xml", XML), ("dc", DC), ("xmp", XMP), ("xmpRights", XMP_RIGHTS), ("pdf", PDF), ("photoshop", PHOTOSHOP)]; // brand-ok: the XMP namespace prefix the files use
        Self {
            prefixes: standard.iter().map(|(p, u)| ((*u).to_string(), (*p).to_string())).collect(),
            uris: standard.iter().map(|(p, u)| ((*p).to_string(), (*u).to_string())).collect(),
            bytes: standard.iter().map(|(p, u)| entry_bytes(u, p)).sum(),
        }
    }
}

impl Registry {
    /// The bytes the namespaces hold, at most [`MAX_REGISTRY_BYTES`].
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// The prefix of the namespace `uri`.
    pub(crate) fn prefix_of(&self, uri: &str) -> Option<&str> {
        self.prefixes.get(uri).map(String::as_str)
    }

    /// The namespace the prefix `prefix` names.
    pub(crate) fn uri_of(&self, prefix: &str) -> Option<&str> {
        self.uris.get(prefix).map(String::as_str)
    }

    /// Registers the namespace `uri` with the prefix `suggested` (a trailing colon is left out), or
    /// with `suggested_1_`, `suggested_2_` … when that one names another namespace. → the prefix
    /// the namespace has, which is its earlier one when it was registered before. A URI over
    /// [`MAX_URI_BYTES`] or a prefix over [`MAX_PREFIX_BYTES`] is [`XmpError::TooLarge`], and a
    /// namespace past [`MAX_NAMESPACES`] or [`MAX_REGISTRY_BYTES`] is [`XmpError::Namespaces`].
    pub(crate) fn register(&mut self, uri: &str, suggested: &str) -> Result<String, XmpError> {
        let (prefix, growth) = self.registration(uri, suggested)?;
        if growth != 0 {
            self.bytes += growth;
            self.prefixes.insert(uri.to_string(), prefix.clone());
            self.uris.insert(prefix.clone(), uri.to_string());
        }
        Ok(prefix)
    }

    /// The prefix and retained bytes a registration would add, without changing the registry.
    pub(crate) fn registration(&self, uri: &str, suggested: &str) -> Result<(String, usize), XmpError> {
        if let Some(p) = self.prefix_of(uri) {
            return Ok((p.to_string(), 0));
        }
        let suggested = suggested.strip_suffix(':').unwrap_or(suggested);
        if uri.len() > MAX_URI_BYTES || suggested.len() > MAX_PREFIX_BYTES {
            return Err(XmpError::TooLarge);
        }
        // XML does not allow the prefix `xmlns` to be declared or a prefix to name its namespace.
        if uri.is_empty() || !is_text(uri) || !is_ncname(suggested) || suggested == "xmlns" || uri == XMLNS {
            return Err(XmpError::BadName);
        }
        if self.prefixes.len() >= MAX_NAMESPACES {
            return Err(XmpError::Namespaces);
        }
        let mut prefix = suggested.to_string();
        let mut n = 1;
        while self.uri_of(&prefix).is_some() {
            prefix = format!("{suggested}_{n}_");
            n += 1;
        }
        if prefix.len() > MAX_PREFIX_BYTES {
            return Err(XmpError::TooLarge);
        }
        let bytes = self.bytes.saturating_add(entry_bytes(uri, &prefix));
        if bytes > MAX_REGISTRY_BYTES {
            return Err(XmpError::Namespaces);
        }
        let growth = entry_bytes(uri, &prefix);
        Ok((prefix, growth))
    }
}

/// Is `s` an XML name without a colon?
pub(crate) fn is_ncname(s: &str) -> bool {
    craft_xml::is_name(s) && !s.contains(':')
}

/// Does `s` hold only characters XML allows?
pub(crate) fn is_text(s: &str) -> bool {
    s.chars().all(craft_xml::is_xml_char)
}

/// Where a property path leads: the whole property, or one item of an array by its position from 1
/// (-1 for the last).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Whole,
    Item(i64),
}

/// The property name and step of the path `path` in the namespace `ns`: `name`, `prefix:name`,
/// either with `[n]` or `[last()]` after it. A prefix must name `ns`.
pub(crate) fn parse_path(path: &str, ns: &str, registry: &Registry) -> Result<(String, Step), XmpError> {
    if path.contains(['/', '?', '@']) {
        return Err(XmpError::NotYet);
    }
    let (name, step) = match path.split_once('[') {
        None => (path, Step::Whole),
        Some((name, rest)) => {
            let inner = rest.strip_suffix(']').ok_or(XmpError::BadName)?;
            let step = match inner {
                "last()" => Step::Item(-1),
                n if !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()) => match n.parse::<i64>() {
                    Ok(i) if i >= 1 => Step::Item(i),
                    _ => return Err(XmpError::BadName),
                },
                _ => return Err(XmpError::NotYet),
            };
            (name, step)
        }
    };
    let local = match name.split_once(':') {
        Some((prefix, local)) => {
            if registry.uri_of(prefix) != Some(ns) {
                return Err(XmpError::BadName);
            }
            local
        }
        None => name,
    };
    if !is_ncname(local) {
        return Err(XmpError::BadName);
    }
    Ok((local.to_string(), step))
}

/// The bindings of namespace prefixes in scope, innermost last; `""` is the default namespace.
type Scope = Vec<(String, String)>;

fn push_decls(el: &El, scope: &mut Scope) {
    for (prefix, uri) in el.ns_decls() {
        scope.push((prefix.to_string(), uri.to_string()));
    }
}

/// Is `name` a namespace declaration?
fn is_decl(name: &str) -> bool {
    name == "xmlns" || name.starts_with("xmlns:")
}

/// The namespace and local name of the qualified name `name`: an unprefixed element is in the
/// default namespace, an unprefixed attribute (`element` false) in none. `None` for a prefix
/// nothing declares.
fn resolve<'n>(name: &'n str, scope: &Scope, element: bool) -> Option<(String, &'n str)> {
    let (prefix, local) = match name.split_once(':') {
        Some((p, l)) => (p, l),
        None if element => ("", name),
        None => return Some((String::new(), name)),
    };
    if prefix == "xml" {
        return Some((XML.to_string(), local));
    }
    let uri = scope.iter().rev().find(|(p, _)| p == prefix).map(|(_, u)| u.clone());
    match uri {
        Some(u) => Some((u, local)),
        None if prefix.is_empty() => Some((String::new(), local)),
        None => None,
    }
}

fn is_named(name: &str, scope: &Scope, element: bool, ns: &str, local: &str) -> bool {
    resolve(name, scope, element).is_some_and(|(u, l)| u == ns && l == local)
}

/// The text of `el`'s own text and CDATA children.
fn text_of(el: &El) -> String {
    let mut s = String::new();
    for n in &el.children {
        if let Node::Text(t) | Node::CData(t) = n {
            s.push_str(t);
        }
    }
    s
}

/// The attributes of `el` that are not namespace declarations.
fn attrs(el: &El) -> Vec<(&str, &str)> {
    el.attrs.iter().filter(|(k, _)| !is_decl(k)).map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// The `rdf:RDF` element at or below `el`, with `scope` holding the bindings in scope there.
fn find_rdf<'a>(el: &'a El, scope: &mut Scope) -> Option<&'a El> {
    let mark = scope.len();
    push_decls(el, scope);
    if is_named(&el.name, scope, true, RDF, "RDF") {
        return Some(el);
    }
    for c in el.elements() {
        if let Some(found) = find_rdf(c, scope) {
            return Some(found);
        }
    }
    scope.truncate(mark);
    None
}

/// Tests: reads the packet `text` with no budget ([`parse_within`]).
#[cfg(test)]
pub(crate) fn parse(text: &str, registry: &mut Registry) -> Result<Meta, XmpError> {
    parse_within(text, registry, usize::MAX)
}

/// Reads the packet `text`, registering the namespaces it uses in `registry`, and stops with
/// [`XmpError::Heap`] as soon as the properties read hold more than `budget` bytes, as
/// [`Meta::heap_bytes`] counts them. A packet that fails to read leaves `registry` unchanged.
pub(crate) fn parse_within(text: &str, registry: &mut Registry, budget: usize) -> Result<Meta, XmpError> {
    let mut staged = registry.clone();
    let meta = parse_into(text, &mut staged, budget, true)?;
    *registry = staged;
    Ok(meta)
}

/// Checks a packet within `budget` without retaining expanded XML fragments.
pub(crate) fn validate_within(text: &str, registry: &mut Registry, budget: usize) -> Result<(), XmpError> {
    let mut staged = registry.clone();
    parse_into(text, &mut staged, budget, false)?;
    *registry = staged;
    Ok(())
}

/// The properties a packet has given so far.
struct Reading {
    meta: Meta,
    /// The names `meta` has.
    seen: HashSet<(Rc<str>, String)>,
    /// The namespaces of the properties, each held once.
    pool: HashSet<Rc<str>>,
    /// The bytes `meta` holds, and the most it may hold.
    bytes: usize,
    budget: usize,
    retain_fragments: bool,
}

impl Reading {
    /// The shared copy of the namespace `ns`.
    fn intern(&mut self, ns: String) -> Rc<str> {
        if let Some(shared) = self.pool.get(ns.as_str()) {
            return Rc::clone(shared);
        }
        let shared: Rc<str> = ns.into();
        self.pool.insert(Rc::clone(&shared));
        shared
    }

    /// Adds `prop` unless a property of that name was read before.
    fn add(&mut self, prop: Prop) -> Result<(), XmpError> {
        if self.seen.insert((Rc::clone(&prop.ns), prop.name.clone())) {
            self.bytes = self.bytes.saturating_add(prop.heap_bytes());
            if self.bytes > self.budget {
                return Err(XmpError::Heap);
            }
            self.meta.props.push(prop);
        }
        Ok(())
    }
}

/// Is `node` text that is not XML whitespace alone?
fn is_stray_text(node: &Node) -> bool {
    matches!(node, Node::Text(t) | Node::CData(t) if !t.trim_matches([' ', '\t', '\r', '\n']).is_empty())
}

fn parse_into(text: &str, registry: &mut Registry, budget: usize, retain_fragments: bool) -> Result<Meta, XmpError> {
    if text.len() > MAX_BYTES {
        return Err(XmpError::TooLarge);
    }
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let limits = Limits { max_input: MAX_BYTES, max_depth: MAX_DEPTH, max_nodes: MAX_NODES, max_attrs: MAX_ATTRS, ..Limits::DEFAULT };
    let opts = Options {
        names: Names::AsWritten,
        doctype: Doctype::Reject,
        unknown_entity: UnknownEntity::Error,
        drop_blank_text: true,
        limits,
        ..Options::default()
    }
    .strict();
    let root = opts.parse(text.as_bytes()).map_err(|e| if e.is_limit() { XmpError::TooLarge } else { XmpError::Malformed })?;
    let mut scope = Scope::new();
    let rdf = find_rdf(&root, &mut scope).ok_or(XmpError::Malformed)?;
    // Text beside the descriptions or their properties is not RDF.
    if rdf.children.iter().any(is_stray_text) {
        return Err(XmpError::Malformed);
    }
    let mut reading = Reading { meta: Meta::default(), seen: HashSet::new(), pool: HashSet::new(), bytes: 32, budget, retain_fragments };
    for d in rdf.elements() {
        let mark = scope.len();
        push_decls(d, &mut scope);
        if !is_named(&d.name, &scope, true, RDF, "Description") || d.children.iter().any(is_stray_text) {
            return Err(XmpError::Malformed);
        }
        for (k, v) in attrs(d) {
            let Some((ns, local)) = resolve(k, &scope, false) else { return Err(XmpError::Malformed) };
            if ns == RDF {
                if local == "about" && reading.meta.about.is_empty() {
                    reading.meta.about = v.to_string();
                    reading.bytes = reading.bytes.saturating_add(v.len());
                    if reading.bytes > reading.budget {
                        return Err(XmpError::Heap);
                    }
                }
                continue;
            }
            if ns.is_empty() || ns == XML {
                continue;
            }
            let prefix = k.split_once(':').map_or("ns", |(p, _)| p);
            registry.register(&ns, prefix)?;
            let ns = reading.intern(ns);
            reading.add(Prop { ns, name: local.to_string(), value: Value::Simple { text: v.to_string(), uri: false } })?;
        }
        for p in d.elements() {
            let prop = read_property(p, &mut scope, registry, &mut reading)?;
            reading.add(prop)?;
        }
        scope.truncate(mark);
    }
    Ok(reading.meta)
}

fn read_property(p: &El, scope: &mut Scope, registry: &mut Registry, reading: &mut Reading) -> Result<Prop, XmpError> {
    let mark = scope.len();
    push_decls(p, scope);
    let (ns, local) = resolve(&p.name, scope, true).ok_or(XmpError::Malformed)?;
    if ns.is_empty() || ns == RDF || ns == XML {
        return Err(XmpError::Malformed);
    }
    let prefix = p.name.split_once(':').map_or("ns", |(p, _)| p);
    registry.register(&ns, prefix)?;
    let ns = reading.intern(ns);
    let duplicate = reading.seen.contains(&(Rc::clone(&ns), local.to_string()));
    let own = attrs(p);
    let children: Vec<&El> = p.elements().collect();
    let value = if children.is_empty() && own.is_empty() {
        Value::Simple { text: text_of(p), uri: false }
    } else if let ([], [(k, v)]) = (children.as_slice(), own.as_slice())
        && is_named(k, scope, false, RDF, "resource")
        && text_of(p).trim().is_empty()
    {
        Value::Simple { text: (*v).to_string(), uri: true }
    } else if let (Some(array), true) = (array_of(&children, scope), own.is_empty() && text_of(p).trim().is_empty()) {
        array
    } else {
        // A written packet puts a property 3 elements deep (`x:xmpmeta`, `rdf:RDF`,
        // `rdf:Description`), which may be deeper than the packet read put it.
        if PROPERTY_DEPTH + height(p) > MAX_DEPTH {
            return Err(XmpError::TooLarge);
        }
        register_names(p, scope, registry)?;
        register_nested(p, scope, registry)?;
        let available = if duplicate { usize::MAX } else { reading.budget.saturating_sub(reading.bytes).saturating_sub(96 + ns.len() + local.len()) };
        other(p, &own, &children, scope, mark, (available, reading.retain_fragments && !duplicate))?
    };
    scope.truncate(mark);
    Ok(Prop { ns, name: local.to_string(), value })
}

/// Registers the namespaces of the name and the attributes of `el`, with `scope` holding the
/// bindings in scope there, other than RDF, XML and no namespace. A prefix nothing declares makes
/// the packet malformed.
fn register_names(el: &El, scope: &Scope, registry: &mut Registry) -> Result<(), XmpError> {
    let names = std::iter::once((el.name.as_str(), true)).chain(attrs(el).into_iter().map(|(k, _)| (k, false)));
    for (name, element) in names {
        let (ns, _) = resolve(name, scope, element).ok_or(XmpError::Malformed)?;
        if !ns.is_empty() && ns != RDF && ns != XML {
            registry.register(&ns, name.split_once(':').map_or("ns", |(p, _)| p))?;
        }
    }
    Ok(())
}

/// Registers the namespaces the elements inside `el` use, as [`register_names`] does.
fn register_nested(el: &El, scope: &mut Scope, registry: &mut Registry) -> Result<(), XmpError> {
    for c in el.elements() {
        let mark = scope.len();
        push_decls(c, scope);
        let r = register_names(c, scope, registry).and_then(|()| register_nested(c, scope, registry));
        scope.truncate(mark);
        r?;
    }
    Ok(())
}

/// The elements a written packet holds around a property.
const PROPERTY_DEPTH: usize = 3;

/// The elements `el` nests, itself included.
fn height(el: &El) -> usize {
    1 + el.elements().map(height).max().unwrap_or(0)
}

/// The array of simple items the only child of a property element holds, when it is one.
fn array_of(children: &[&El], scope: &mut Scope) -> Option<Value> {
    let [container] = children else { return None };
    let mark = scope.len();
    push_decls(container, scope);
    let form = [Form::Bag, Form::Seq, Form::Alt].into_iter().find(|f| is_named(&container.name, scope, true, RDF, &f.element()[4..]));
    let mut items = Vec::new();
    let ok = form.is_some() && attrs(container).is_empty() && text_of(container).trim().is_empty() && {
        container.elements().all(|li| {
            let inner = scope.len();
            push_decls(li, scope);
            let mut lang = None;
            let mut simple = is_named(&li.name, scope, true, RDF, "li") && li.elements().next().is_none();
            for (k, v) in attrs(li) {
                if is_named(k, scope, false, XML, "lang") {
                    lang = Some(v.to_string());
                } else {
                    simple = false;
                }
            }
            scope.truncate(inner);
            if simple {
                items.push(Item { text: text_of(li), lang });
            }
            simple
        })
    };
    scope.truncate(mark);
    match (ok, form) {
        (true, Some(form)) => Some(Value::Array { form, items }),
        _ => None,
    }
}

/// A value kept as the XML it was read as, refused when the element written for it would hold more
/// than [`MAX_ATTRS`] attributes or declarations in scope, which a packet could then not read back.
fn other(p: &El, own: &[(&str, &str)], children: &[&El], scope: &mut Scope, mark: usize, (budget, keep): (usize, bool)) -> Result<Value, XmpError> {
    let (options, text) = match qualified_value(own, children, scope) {
        Some(found) => found,
        None => kept_form(p, own, children, scope),
    };
    let mut xml = Fragment { text: String::new(), bytes: 0, budget: budget.saturating_sub(text.as_ref().map_or(0, String::len)), keep };
    let added = write_fragment(p, scope.get(..mark).unwrap_or(&[]), &mut xml)?;
    let decls = added + nested_decls(p);
    if p.attrs.len() + added > MAX_ATTRS || decls > MAX_ATTRS {
        return Err(XmpError::TooLarge);
    }
    Ok(Value::Other { xml: xml.text, expanded_bytes: xml.bytes, options, text, decls })
}

/// The most namespace declarations `el` and the elements inside it put in scope at one element.
fn nested_decls(el: &El) -> usize {
    el.ns_decls().count() + el.elements().map(nested_decls).max().unwrap_or(0)
}

/// The array form of the container `el`, read with the prefixes it declares itself.
fn container_form(el: &El, scope: &mut Scope) -> Option<Form> {
    let inner = scope.len();
    push_decls(el, scope);
    let form = [Form::Bag, Form::Seq, Form::Alt].into_iter().find(|f| is_named(&el.name, scope, true, RDF, &f.element()[4..]));
    scope.truncate(inner);
    form
}

/// The options and text of a kept value that is not in the `rdf:value` form.
fn kept_form(p: &El, own: &[(&str, &str)], children: &[&El], scope: &mut Scope) -> (u32, Option<String>) {
    let form = children.first().and_then(|c| container_form(c, scope));
    let resource = own.iter().find(|(k, _)| is_named(k, scope, false, RDF, "resource")).map(|(_, v)| (*v).to_string());
    let lang = own.iter().any(|(k, _)| is_named(k, scope, false, XML, "lang"));
    // An empty property element whose attributes are fields is a structure.
    let fields = own.iter().any(|(k, _)| resolve(k, scope, false).is_some_and(|(ns, _)| !ns.is_empty() && ns != RDF && ns != XML));
    match form {
        Some(f) if children.len() == 1 => (f.options() | PROP_HAS_QUALIFIERS, None),
        _ if !children.is_empty() || own.iter().any(|(k, _)| is_named(k, scope, false, RDF, "parseType")) => (PROP_IS_STRUCT, None),
        _ if fields && resource.is_none() && !p.children.iter().any(is_stray_text) => (PROP_IS_STRUCT, None),
        _ => {
            let uri = if resource.is_some() { PROP_VALUE_IS_URI } else { 0 };
            (PROP_HAS_QUALIFIERS | uri | if lang { PROP_HAS_LANG } else { 0 }, Some(resource.unwrap_or_else(|| text_of(p))))
        }
    }
}

/// The options and text of a value with qualifiers in the `rdf:value` form (XMP Specification
/// Part 1, 7.9.2.5): `rdf:value` and the qualifiers are the fields of the property, written with
/// `rdf:parseType="Resource"`, in its one `rdf:Description`, or as the attributes of an empty
/// property element. `None` for a value in another form.
fn qualified_value(own: &[(&str, &str)], children: &[&El], scope: &mut Scope) -> Option<(u32, Option<String>)> {
    let lang = if own.iter().any(|(k, _)| is_named(k, scope, false, XML, "lang")) { PROP_HAS_LANG | PROP_HAS_QUALIFIERS } else { 0 };
    let resource_type = own.iter().any(|(k, v)| is_named(k, scope, false, RDF, "parseType") && *v == "Resource");
    let found = if children.is_empty() || resource_type {
        value_field(children, own, scope)
    } else if let [d] = children {
        let mark = scope.len();
        push_decls(d, scope);
        let found = if is_named(&d.name, scope, true, RDF, "Description") {
            let fields: Vec<&El> = d.elements().collect();
            value_field(&fields, &attrs(d), scope)
        } else {
            None
        };
        scope.truncate(mark);
        found
    } else {
        None
    };
    found.map(|(options, text)| (options | lang, text))
}

/// The options and text the `rdf:value` among the field elements `fields` and the attributes
/// `held` of one structure gives; the other fields and the attributes outside the RDF namespace
/// are its qualifiers.
fn value_field(fields: &[&El], held: &[(&str, &str)], scope: &mut Scope) -> Option<(u32, Option<String>)> {
    let is_rdf = |k: &str, scope: &Scope| resolve(k, scope, false).is_some_and(|(ns, _)| ns == RDF);
    let lang_in = |list: &[(&str, &str)], scope: &Scope| list.iter().any(|(k, _)| is_named(k, scope, false, XML, "lang"));
    let mut qualifiers = held.iter().any(|(k, _)| !is_rdf(k, scope));
    let mut lang = lang_in(held, scope);
    if let Some((_, text)) = held.iter().find(|(k, _)| is_named(k, scope, false, RDF, "value")) {
        qualifiers |= !fields.is_empty();
        return Some((option_bits(0, qualifiers, lang), Some((*text).to_string())));
    }
    let at = fields.iter().position(|f| {
        let mark = scope.len();
        push_decls(f, scope);
        let found = is_named(&f.name, scope, true, RDF, "value");
        scope.truncate(mark);
        found
    })?;
    let value = fields.get(at)?;
    qualifiers |= fields.len() > 1;
    let mark = scope.len();
    push_decls(value, scope);
    let own = attrs(value);
    lang |= lang_in(&own, scope);
    let resource = own.iter().find(|(k, _)| is_named(k, scope, false, RDF, "resource")).map(|(_, v)| (*v).to_string());
    let inner: Vec<&El> = value.elements().collect();
    let found = match inner.as_slice() {
        [] => match resource {
            Some(uri) => (PROP_VALUE_IS_URI, Some(uri)),
            None => (0, Some(text_of(value))),
        },
        [c] => match container_form(c, scope) {
            Some(f) => (f.options(), None),
            None => (PROP_IS_STRUCT, None),
        },
        _ => (PROP_IS_STRUCT, None),
    };
    scope.truncate(mark);
    Some((option_bits(found.0, qualifiers, lang), found.1))
}

/// `options` with [`PROP_HAS_QUALIFIERS`] when the value has `qualifiers`, and with
/// [`PROP_HAS_LANG`] as well when one of them is `xml:lang`.
fn option_bits(options: u32, qualifiers: bool, lang: bool) -> u32 {
    let qualifiers = if qualifiers || lang { PROP_HAS_QUALIFIERS } else { 0 };
    options | qualifiers | if lang { PROP_HAS_LANG } else { 0 }
}

#[cfg(test)]
std::thread_local! {
    static FRAGMENT_BYTES_WRITTEN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Fragment {
    text: String,
    bytes: usize,
    budget: usize,
    keep: bool,
}

impl Fragment {
    fn push(&mut self, text: &str) -> Result<(), XmpError> {
        if text.len() > self.budget.saturating_sub(self.bytes) {
            return Err(XmpError::Heap);
        }
        self.bytes += text.len();
        if self.keep {
            self.text.push_str(text);
            #[cfg(test)]
            FRAGMENT_BYTES_WRITTEN.with(|bytes| bytes.set(bytes.get() + text.len()));
        }
        Ok(())
    }

    fn escaped(&mut self, text: &str, attribute: bool) -> Result<(), XmpError> {
        let mut escaped = String::new();
        for character in text.chars() {
            let mut buffer = [0; 4];
            let character = character.encode_utf8(&mut buffer);
            if attribute {
                craft_xml::push_esc_attr(&mut escaped, character);
            } else {
                craft_xml::push_esc(&mut escaped, character);
            }
            self.push(&escaped)?;
            escaped.clear();
        }
        Ok(())
    }
}

/// Writes `p` and its inherited namespace bindings within the remaining property budget.
fn write_fragment(p: &El, outer: &[(String, String)], out: &mut Fragment) -> Result<usize, XmpError> {
    let mut used: Vec<String> = Vec::new();
    collect_prefixes(p, &mut used);
    let own: Vec<&str> = p.ns_decls().map(|(prefix, _)| prefix).collect();
    out.push("<")?;
    out.push(&p.name)?;
    let mut added = 0;
    for prefix in used.iter().filter(|u| !own.contains(&u.as_str())) {
        if let Some((_, uri)) = outer.iter().rev().find(|(q, _)| q == prefix) {
            added += 1;
            if prefix.is_empty() {
                out.push(" xmlns=\"")?;
            } else {
                out.push(" xmlns:")?;
                out.push(prefix)?;
                out.push("=\"")?;
            }
            out.escaped(uri, true)?;
            out.push("\"")?;
        }
    }
    write_rest(p, out)?;
    Ok(added)
}

/// The prefixes the names of `el` and the elements inside it use, `""` for an unprefixed element.
fn collect_prefixes(el: &El, used: &mut Vec<String>) {
    let mut add = |prefix: &str| {
        if prefix != "xml" && !used.iter().any(|u| u == prefix) {
            used.push(prefix.to_string());
        }
    };
    add(el.name.split_once(':').map_or("", |(p, _)| p));
    for (k, _) in &el.attrs {
        if let Some((p, _)) = k.split_once(':')
            && p != "xmlns"
        {
            add(p);
        }
    }
    for c in el.elements() {
        collect_prefixes(c, used);
    }
}

/// Writes the attributes, the children and the end tag of `el`, whose name is already written.
fn write_rest(el: &El, out: &mut Fragment) -> Result<(), XmpError> {
    for (k, v) in &el.attrs {
        out.push(" ")?;
        out.push(k)?;
        out.push("=\"")?;
        out.escaped(v, true)?;
        out.push("\"")?;
    }
    if el.children.is_empty() {
        return out.push("/>");
    }
    out.push(">")?;
    for n in &el.children {
        match n {
            Node::El(c) => {
                out.push("<")?;
                out.push(&c.name)?;
                write_rest(c, out)?;
            }
            Node::Text(t) | Node::CData(t) => out.escaped(t, false)?,
            Node::Pi { .. } | Node::Comment(_) => {}
        }
    }
    out.push("</")?;
    out.push(&el.name)?;
    out.push(">")
}

/// Text written while keeping to [`MAX_BYTES`].
struct Out(String);

impl Out {
    fn push(&mut self, s: &str) -> Result<(), XmpError> {
        if self.0.len().saturating_add(s.len()) > MAX_BYTES {
            return Err(XmpError::TooLarge);
        }
        self.0.push_str(s);
        Ok(())
    }

    fn text(&mut self, s: &str) -> Result<(), XmpError> {
        self.push(&craft_xml::esc(s))
    }

    fn attr(&mut self, s: &str) -> Result<(), XmpError> {
        self.push(&craft_xml::esc_attr(s))
    }
}

/// `meta` as an XMP packet: the `x:xmpmeta` element inside a writable packet wrapper, or alone
/// without `wrapper`.
pub(crate) fn serialize(meta: &Meta, registry: &Registry, wrapper: bool) -> Result<String, XmpError> {
    let mut namespaces: Vec<(&str, &str)> = Vec::new();
    let mut declared = HashSet::new();
    for p in &meta.props {
        if matches!(p.value, Value::Other { .. }) || !declared.insert(&*p.ns) {
            continue;
        }
        let prefix = registry.prefix_of(&p.ns).ok_or(XmpError::BadName)?;
        namespaces.push((prefix, &*p.ns));
    }
    // A read takes at most `MAX_ATTRS` declarations in scope at one element: those of `x:xmpmeta`
    // and `rdf:RDF`, a declaration on `rdf:Description` for each namespace, and the declarations of a
    // kept value.
    let around = 2 + namespaces.len();
    let kept = meta.props.iter().map(|p| if let Value::Other { decls, .. } = p.value { decls } else { 0 }).max().unwrap_or(0);
    if around + kept > MAX_ATTRS {
        return Err(XmpError::TooLarge);
    }
    let prefix = |ns: &str| registry.prefix_of(ns).ok_or(XmpError::BadName);
    let mut out = Out(String::new());
    if wrapper {
        out.push(&format!("<?xpacket begin=\"\u{FEFF}\" id=\"{WRAPPER_ID}\"?>\n"))?;
    }
    out.push(&format!("<x:xmpmeta xmlns:x=\"{X}\">\n <rdf:RDF xmlns:rdf=\"{RDF}\">\n  <rdf:Description rdf:about=\""))?;
    out.attr(&meta.about)?;
    out.push("\"")?;
    for (p, uri) in &namespaces {
        out.push(&format!(" xmlns:{p}=\""))?;
        out.attr(uri)?;
        out.push("\"")?;
    }
    out.push(">\n")?;
    for p in &meta.props {
        match &p.value {
            Value::Simple { text, uri: false } => {
                let name = format!("{}:{}", prefix(&p.ns)?, p.name);
                out.push(&format!("   <{name}>"))?;
                out.text(text)?;
                out.push(&format!("</{name}>\n"))?;
            }
            Value::Simple { text, uri: true } => {
                out.push(&format!("   <{}:{} rdf:resource=\"", prefix(&p.ns)?, p.name))?;
                out.attr(text)?;
                out.push("\"/>\n")?;
            }
            Value::Array { form, items } => {
                let name = format!("{}:{}", prefix(&p.ns)?, p.name);
                out.push(&format!("   <{name}>\n    <{}>\n", form.element()))?;
                for item in items {
                    out.push("     <rdf:li")?;
                    if let Some(lang) = &item.lang {
                        out.push(" xml:lang=\"")?;
                        out.attr(lang)?;
                        out.push("\"")?;
                    }
                    out.push(">")?;
                    out.text(&item.text)?;
                    out.push("</rdf:li>\n")?;
                }
                out.push(&format!("    </{}>\n   </{name}>\n", form.element()))?;
            }
            Value::Other { xml, .. } => {
                out.push("   ")?;
                out.push(xml)?;
                out.push("\n")?;
            }
        }
    }
    out.push("  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>")?;
    if wrapper {
        out.push("\n<?xpacket end=\"w\"?>")?;
    }
    Ok(out.0)
}

/// Where a packet lies in a file's bytes: from the start of its `<?xpacket begin=` to the end of
/// its `<?xpacket end=…?>`, and whether its wrapper marks it writable (`end="w"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketSpan {
    pub start: usize,
    pub end: usize,
    pub writeable: bool,
}

/// The characters of whitespace padding just before the `<?xpacket end=` trailer of `packet` (XMP
/// Specification Part 1, 7.3), or 0 when it has no trailer.
pub(crate) fn pad_size(packet: &str) -> usize {
    packet.rfind("<?xpacket end=").map_or(0, |end| {
        let before = packet.get(..end).unwrap_or_default();
        before.len() - before.trim_end_matches([' ', '\t', '\r', '\n']).len()
    })
}

/// The first UTF-8 packet in `bytes` that has both ends of its wrapper (XMP Specification Part 1,
/// 7.3), or `None`. A start is a `<?xpacket begin=` whose header holds the wrapper's `id` within
/// [`MAX_HEADER`] bytes. A start with no end before the next start is skipped, and so is a trailer
/// that does not close before the next start. Inside a packet, a start or a trailer in a CDATA
/// section or a comment is text, and a packet that is not UTF-8 is passed over for the next one. A
/// section that does not close, or whose end follows a trailer and then a start, is read as text, so
/// a later packet's start is not hidden in it. The scan takes time in proportion to the length of
/// `bytes`.
pub fn find_packet(bytes: &[u8]) -> Option<PacketSpan> {
    const BEGIN: &[u8] = b"<?xpacket begin=";
    const END: &[u8] = b"<?xpacket end=";
    const SECTIONS: [(&[u8], &[u8]); 2] = [(b"<![CDATA[", b"]]>"), (b"<!--", b"-->")];
    let len = bytes.len();
    let rest = |at: usize| bytes.get(at..).unwrap_or_default();
    let ids = [format!("id=\"{WRAPPER_ID}\""), format!("id='{WRAPPER_ID}'")];
    let is_start = |at: usize| {
        rest(at).starts_with(BEGIN) && {
            let head = bytes.get(at..len.min(at.saturating_add(MAX_HEADER))).unwrap_or_default();
            let head = head.windows(2).position(|w| w == b"?>").and_then(|close| head.get(..close)).unwrap_or_default();
            ids.iter().any(|id| head.windows(id.len()).any(|w| w == id.as_bytes()))
        }
    };
    // Every start, in order.
    let starts: Vec<usize> = bytes.windows(BEGIN.len()).enumerate().filter(|(at, w)| *w == BEGIN && is_start(*at)).map(|(at, _)| at).collect();
    let start_after = |from: usize| starts.get(starts.partition_point(|&s| s < from)).copied();
    let mut ends = Later::new(END);
    let mut closes = Later::new(b"?>");
    let mut section_ends = SECTIONS.map(|(_, close)| Later::new(close));
    let mut start = None;
    let mut pos = 0;
    while let Some(at) = rest(pos).iter().position(|&b| b == b'<').map(|i| i + pos) {
        pos = at + 1;
        if starts.binary_search(&at).is_ok() {
            start = Some(at);
            pos = at + BEGIN.len();
            continue;
        }
        let Some(from) = start else {
            // Nothing before the next start matters.
            pos = start_after(pos)?;
            continue;
        };
        let here = rest(at);
        if let Some(kind) = SECTIONS.iter().position(|(open, _)| here.starts_with(open)) {
            let (open, close) = SECTIONS[kind];
            let inside = at + open.len();
            let end = section_ends[kind].find(bytes, inside);
            // A trailer and then a start inside the section belong to packets of their own.
            let hides_packet = end.is_some_and(|end| ends.find(bytes, inside).is_some_and(|e| e < end && start_after(e).is_some_and(|s| s < end)));
            if let (Some(end), false) = (end, hides_packet) {
                pos = end + close.len();
            }
            continue;
        }
        if !here.starts_with(END) {
            continue;
        }
        let after = at + END.len();
        let close = closes.find(bytes, after)?;
        if let Some(next) = start_after(after).filter(|&next| next < close) {
            start = Some(next);
            pos = next + BEGIN.len();
            continue;
        }
        let span = PacketSpan { start: from, end: close + 2, writeable: bytes.get(after + 1) == Some(&b'w') };
        if bytes.get(span.start..span.end).is_some_and(|p| std::str::from_utf8(p).is_ok()) {
            return Some(span);
        }
        start = None;
        pos = span.end;
    }
    None
}

/// The next place of a marker at or after a given point. While the points do not go back, each
/// search starts past the last one, so the searches together read the bytes about once.
struct Later {
    marker: &'static [u8],
    /// The point of the last search, and what it found.
    from: usize,
    found: Option<usize>,
}

impl Later {
    fn new(marker: &'static [u8]) -> Later {
        Later { marker, from: usize::MAX, found: None }
    }

    fn find(&mut self, bytes: &[u8], from: usize) -> Option<usize> {
        if self.from <= from && self.found.is_none_or(|f| f >= from) {
            return self.found;
        }
        self.from = from;
        self.found = bytes.get(from..).and_then(|r| r.windows(self.marker.len()).position(|w| w == self.marker)).map(|i| i + from);
        self.found
    }
}

#[cfg(test)]
mod fragment_budget_tests {
    use super::*;

    #[test]
    fn expanded_namespace_fragments_stop_before_exceeding_the_heap_budget() {
        let mut declarations = String::new();
        let mut fields = String::new();
        for index in 0..16 {
            declarations.push_str(&format!(" xmlns:n{index}=\"urn:{index}:{}\"", "x".repeat(950)));
            fields.push_str(&format!("<n{index}:field>value</n{index}:field>"));
        }
        let packet = format!(
            "<rdf:RDF xmlns:rdf=\"{RDF}\" xmlns:p=\"urn:property\"{declarations}><rdf:Description><p:value rdf:parseType=\"Resource\">{fields}</p:value></rdf:Description></rdf:RDF>"
        );
        let budget = 4096;
        FRAGMENT_BYTES_WRITTEN.with(|bytes| bytes.set(0));
        assert_eq!(parse_within(&packet, &mut Registry::default(), budget), Err(XmpError::Heap));
        let written = FRAGMENT_BYTES_WRITTEN.with(std::cell::Cell::get);
        assert!(written <= budget, "expanded fragment wrote {written} bytes with a {budget}-byte budget");
        let meta = parse_within(&packet, &mut Registry::default(), 64 * 1024).unwrap();
        assert!(meta.props[0].heap_bytes() > budget, "the valid control expands inherited namespace bindings");
        FRAGMENT_BYTES_WRITTEN.with(|bytes| bytes.set(0));
        validate_within(&packet, &mut Registry::default(), 64 * 1024).unwrap();
        assert_eq!(FRAGMENT_BYTES_WRITTEN.with(std::cell::Cell::get), 0, "validation must not retain fragment strings");
        let duplicate = packet.replacen("<p:value rdf:parseType=", "<p:value>first</p:value><p:value rdf:parseType=", 1);
        let meta = parse_within(&duplicate, &mut Registry::default(), 512).unwrap();
        assert_eq!(meta.props[0].value.text(), Some("first"), "a duplicate does not consume retained fragment space");
    }
}
