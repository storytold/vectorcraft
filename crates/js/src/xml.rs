//! E4X values (ECMA-357): the internal methods of XML and XMLList, the XML parser and serializer,
//! and the bytecode operations on them.
//!
//! Tree nodes are heap objects linked by object index. No walk uses native recursion: parsing,
//! copying, comparing, normalizing and serializing keep explicit stacks whose depth is at most
//! [`crate::Options::max_depth`], which every insertion enforces. Each visited node takes fuel, and
//! each new node, name, reference or output unit is charged to the heap before it is made.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::LimitKind;
use crate::builtins::Builder;
use crate::chars::{is_id_part, is_id_start};
use crate::error::TOO_DEEP;
use crate::heap::{
    Binding, Class, Env, Key, OBJECT_BYTES, ObjId, Object, XML_LIST_BYTES, XML_NODE_BYTES, XML_NS_BYTES, XML_REF_BYTES, XmlFilterState, XmlKind,
    XmlListData, XmlName, XmlNode, XmlNs, key, string_bytes,
};
use crate::interp::{Abrupt, ErrorType, HeldBytes, Interp, JsResult};
use crate::string_builtins::{UNITS_PER_FUEL, flat_charged};
use crate::value::{Str, Value, index_key};

fn parser_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub(crate) const STAR: &[u16] = &[0x2A];
/// The estimated bytes, for one attribute, of the copies of two attribute lists and the count of
/// names and values that comparing two XML values builds.
const ATTR_COUNT_BYTES: usize = 96;
/// The estimated bytes of one frame of the work stack that comparing two XML values uses.
const EQUALS_FRAME_BYTES: usize = 24;
const XML_URI: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_URI: &str = "http://www.w3.org/2000/xmlns/";
pub(crate) const RESERVED_NS_BINDING: &str = "a namespace declaration uses a reserved prefix or URI";
/// The hidden binding that holds a function's default XML namespace URI.
const DEFAULT_NS_BINDING: &[u16] = &[0, 0x64, 0x78, 0x6E, 0x73];

const LT: u16 = b'<' as u16;
const GT: u16 = b'>' as u16;
const SLASH: u16 = b'/' as u16;
const AMP: u16 = b'&' as u16;
const EQ: u16 = b'=' as u16;
const DQUOTE: u16 = b'"' as u16;
const SQUOTE: u16 = b'\'' as u16;
const COLON: u16 = b':' as u16;
const QUESTION: u16 = b'?' as u16;
const CR: u16 = 0x0D;
const LF: u16 = 0x0A;
const TAB: u16 = 0x09;
const SPACE: u16 = 0x20;

/// The values of the XML constructor's settings properties.
#[derive(Clone, Copy)]
pub(crate) struct Settings {
    pub(crate) ignore_comments: bool,
    pub(crate) ignore_pis: bool,
    pub(crate) ignore_whitespace: bool,
    pub(crate) pretty: bool,
    pub(crate) indent: u32,
}

/// A name an XML value is accessed with. `uri` is `None` for a name that matches any namespace.
#[derive(Clone)]
pub(crate) struct NameRef {
    pub(crate) uri: Option<Key>,
    pub(crate) local: Key,
    pub(crate) attribute: bool,
}

impl NameRef {
    pub(crate) fn is_star(&self) -> bool {
        *self.local == *STAR
    }

    fn matches(&self, name: &XmlName) -> bool {
        (self.is_star() || self.local == name.local) && self.uri.as_ref().is_none_or(|u| name.uri.as_ref() == Some(u))
    }

    fn matches_child(&self, c: &XmlNode) -> bool {
        let element = c.name.as_ref().filter(|_| c.kind == XmlKind::Element);
        (self.is_star() || element.is_some_and(|n| n.local == self.local))
            && self.uri.as_ref().is_none_or(|u| element.is_some_and(|n| n.uri.as_ref() == Some(u)))
    }

    fn to_name(&self) -> XmlName {
        XmlName { uri: self.uri.clone(), local: self.local.clone(), prefix: None }
    }
}

/// How an XML value is accessed: by position or by name.
pub(crate) enum Access {
    Index(u32),
    Name(NameRef),
}

/// What a value is, for the operations that treat XML and XMLList differently.
#[derive(Clone, Copy)]
pub(crate) enum Xv {
    Node(ObjId),
    List(ObjId),
    Other,
}

/// A value to store in a tree.
pub(crate) enum Content {
    Text(Str),
    Node(ObjId),
    List(ObjId),
}

pub(crate) fn is_xml_space(c: u16) -> bool {
    matches!(c, SPACE | TAB | CR | LF)
}

fn is_surrogate(c: u16) -> bool {
    (0xD800..=0xDFFF).contains(&c)
}

/// A unit a name may start with (the XMLName grammar with `:` for a prefix).
pub(crate) fn is_name_start(c: u16) -> bool {
    c == COLON || is_surrogate(c) || (c != 0x24 && is_id_start(u32::from(c)))
}

/// A unit a name may hold after its first.
pub(crate) fn is_name_part(c: u16) -> bool {
    is_name_start(c) || matches!(c, 0x2D | 0x2E | 0xB7) || (c != 0x24 && is_id_part(u32::from(c)))
}

/// The offset after the XML name that starts at `src[pos]`, when one does.
pub(crate) fn scan_name(src: &[u16], pos: usize) -> Option<usize> {
    if !src.get(pos).copied().is_some_and(is_name_start) {
        return None;
    }
    let mut end = pos + 1;
    while src.get(end).copied().is_some_and(is_name_part) {
        end += 1;
    }
    valid_xml_units(src.get(pos..end).unwrap_or_default()).then_some(end)
}

fn skip_xml_space(src: &[u16], mut pos: usize) -> usize {
    while src.get(pos).copied().is_some_and(is_xml_space) {
        pos += 1;
    }
    pos
}

/// Whether `units` holds only `name="value"` attributes separated by white space.
fn is_attribute_run(units: &[u16]) -> bool {
    let mut at = skip_xml_space(units, 0);
    while at < units.len() {
        let Some(name_end) = scan_name(units, at) else { return false };
        let eq = skip_xml_space(units, name_end);
        if units.get(eq) != Some(&EQ) {
            return false;
        }
        let open = skip_xml_space(units, eq + 1);
        let quote = match units.get(open).copied() {
            Some(q @ (DQUOTE | SQUOTE)) => q,
            _ => return false,
        };
        let value = units.get(open + 1..).unwrap_or_default();
        let Some(len) = value.iter().position(|&c| c == quote || c == LT) else { return false };
        if value.get(len) != Some(&quote) {
            return false;
        }
        let end = open + len + 2;
        at = skip_xml_space(units, end);
        if at == end && at < units.len() {
            return false;
        }
    }
    true
}

/// An XML name without a prefix (ECMA-357 section 13.1.2.1).
pub(crate) fn is_ncname(units: &[u16]) -> bool {
    let Some((&first, rest)) = units.split_first() else { return false };
    first != COLON && is_name_start(first) && rest.iter().all(|&c| c != COLON && is_name_part(c)) && valid_xml_units(units)
}

/// Whether every unit is an XML character, with each surrogate part of a pair.
pub(crate) fn valid_xml_units(units: &[u16]) -> bool {
    let mut at = 0;
    while let Some(&c) = units.get(at) {
        if starts_pair(c, units.get(at + 1).copied()) {
            at += 2;
        } else if is_xml_char(u32::from(c)) {
            at += 1;
        } else {
            return false;
        }
    }
    true
}

fn starts_pair(c: u16, next: Option<u16>) -> bool {
    (0xD800..=0xDBFF).contains(&c) && next.is_some_and(|n| (0xDC00..=0xDFFF).contains(&n))
}

/// The names the global `isXMLName` accepts: JavaScript identifiers without `$`, with `-` allowed
/// after the first unit.
pub(crate) fn is_script_xml_name(units: &[u16]) -> bool {
    let Some((&first, rest)) = units.split_first() else { return false };
    first != 0x24 && is_id_start(u32::from(first)) && rest.iter().all(|&c| c == 0x2D || (c != 0x24 && is_id_part(u32::from(c))))
}

fn is_xml_char(c: u32) -> bool {
    matches!(c, 0x09 | 0x0A | 0x0D | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// Reads the reference starting at `src[pos]` (`&`): the offset after it and the character.
pub(crate) fn scan_reference(src: &[u16], pos: usize) -> Option<(usize, u32)> {
    let rest = src.get(pos + 1..)?;
    let semi = rest.iter().take(64).position(|&c| c == 0x3B)?;
    let body = String::from_utf16(rest.get(..semi)?).ok()?;
    let c = match body.as_str() {
        "amp" => 0x26,
        "lt" => 0x3C,
        "gt" => 0x3E,
        "apos" => 0x27,
        "quot" => 0x22,
        _ => {
            let digits = body.strip_prefix('#')?;
            let (radix, digits) = digits.strip_prefix('x').map_or((10, digits), |d| (16, d));
            if digits.is_empty() || digits.starts_with(['+', '-']) {
                return None;
            }
            let c = u32::from_str_radix(digits, radix).ok()?;
            if !is_xml_char(c) {
                return None;
            }
            c
        }
    };
    Some((pos + 1 + semi + 1, c))
}

fn push_char(out: &mut Vec<u16>, c: u32) {
    if let Some(ch) = char::from_u32(c) {
        let mut buf = [0u16; 2];
        out.extend_from_slice(ch.encode_utf16(&mut buf));
    }
}

fn name_bytes(n: &XmlName) -> usize {
    string_bytes(n.local.len()) + n.uri.as_ref().map_or(0, |u| string_bytes(u.len())) + n.prefix.as_ref().map_or(0, |p| string_bytes(p.len()))
}

/// The namespace `stack` declares for `uri`, nearest first, skipping prefixes redeclared nearer.
/// `prefer` picks that prefix when several match; `need_prefix` rejects the default namespace.
fn ns_lookup(stack: &[XmlNs], uri: &Key, prefer: Option<&Key>, need_prefix: bool) -> Option<XmlNs> {
    let mut seen: HashSet<&[u16]> = HashSet::new();
    let mut best = None;
    for ns in stack.iter().rev() {
        let Some(p) = &ns.prefix else { continue };
        if !seen.insert(p) {
            continue;
        }
        if ns.uri == *uri && !(need_prefix && p.is_empty()) {
            if prefer == Some(p) {
                return Some(ns.clone());
            }
            if best.is_none() {
                best = Some(ns.clone());
            }
        }
    }
    best
}

fn prefix_in_scope<'a>(stack: &'a [XmlNs], prefix: &[u16]) -> Option<&'a XmlNs> {
    stack.iter().rev().find(|ns| ns.prefix.as_deref() == Some(prefix))
}

/// Whether Namespaces in XML allows declaring `prefix` for `uri`: `xml` only with its fixed URI,
/// `xmlns` never, and neither fixed URI with another prefix or as the default namespace.
pub(crate) fn ns_binding_allowed(prefix: &[u16], uri: &[u16]) -> bool {
    if units_are(prefix, "xmlns") {
        return false;
    }
    let is_xml_uri = units_are(uri, XML_URI);
    if units_are(prefix, "xml") {
        return is_xml_uri;
    }
    !is_xml_uri && !units_are(uri, XMLNS_URI)
}

/// Whether a node named `name` can be written as XML: no name is in the `xmlns` namespace, and
/// no attribute in no namespace is named `xmlns`.
pub(crate) fn xml_name_writable(name: &XmlName, attribute: bool) -> bool {
    let uri = name.uri.as_deref().unwrap_or_default();
    !units_are(uri, XMLNS_URI) && !(attribute && uri.is_empty() && units_are(&name.local, "xmlns"))
}

fn units_are(units: &[u16], text: &str) -> bool {
    units.iter().copied().eq(text.encode_utf16())
}

/// A prefix no entry of `stack` uses.
fn fresh_prefix(stack: &[XmlNs]) -> Key {
    let mut n = 0u32;
    loop {
        let candidate = key(&format!("ns{n}"));
        if prefix_in_scope(stack, &candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}

/// One open element while serializing.
struct OutFrame {
    node: ObjId,
    next: usize,
    indent: u64,
    child_indent: u64,
    pretty_children: bool,
    ns_mark: usize,
    prefix: Key,
    local: Key,
}

impl Interp {
    // ---- kinds and accessors ----

    pub(crate) fn classify(&self, v: &Value) -> Xv {
        if let Value::Object(o) = v {
            match self.heap.object(*o).map(|ob| &ob.class) {
                Some(Class::Xml(_)) => return Xv::Node(*o),
                Some(Class::XmlList(_)) => return Xv::List(*o),
                _ => {}
            }
        }
        Xv::Other
    }

    pub(crate) fn is_xml(&self, v: &Value) -> bool {
        !matches!(self.classify(v), Xv::Other)
    }

    /// A name made by E4X syntax, as the key of an access.
    fn is_reference_name(&self, v: &Value) -> bool {
        matches!(v, Value::Object(o) if matches!(self.heap.object(*o).map(|ob| &ob.class), Some(Class::QName { reference: true, .. })))
    }

    /// The access goes through the E4X operations.
    pub(crate) fn is_xml_access(&self, base: &Value, k: &Value) -> bool {
        self.options.e4x && (self.is_xml(base) || self.is_reference_name(k))
    }

    pub(crate) fn xml_node(&self, id: ObjId) -> JsResult<&XmlNode> {
        match self.heap.object(id).map(|o| &o.class) {
            Some(Class::Xml(n)) => Ok(n),
            _ => Err(Abrupt::Internal("not an XML value".into())),
        }
    }

    pub(crate) fn xml_node_mut(&mut self, id: ObjId) -> JsResult<&mut XmlNode> {
        match self.heap.object_mut(id).map(|o| &mut o.class) {
            Some(Class::Xml(n)) => Ok(n),
            _ => Err(Abrupt::Internal("not an XML value".into())),
        }
    }

    pub(crate) fn xml_list(&self, id: ObjId) -> JsResult<&XmlListData> {
        match self.heap.object(id).map(|o| &o.class) {
            Some(Class::XmlList(l)) => Ok(l),
            _ => Err(Abrupt::Internal("not an XMLList value".into())),
        }
    }

    pub(crate) fn xml_list_mut(&mut self, id: ObjId) -> JsResult<&mut XmlListData> {
        match self.heap.object_mut(id).map(|o| &mut o.class) {
            Some(Class::XmlList(l)) => Ok(l),
            _ => Err(Abrupt::Internal("not an XMLList value".into())),
        }
    }

    pub(crate) fn xml_kind(&self, id: ObjId) -> JsResult<XmlKind> {
        Ok(self.xml_node(id)?.kind)
    }

    fn child_at(&self, x: ObjId, i: usize) -> JsResult<Option<ObjId>> {
        Ok(self.xml_node(x)?.children.get(i).copied())
    }

    fn children_len(&self, x: ObjId) -> JsResult<usize> {
        Ok(self.xml_node(x)?.children.len())
    }

    pub(crate) fn list_at(&self, l: ObjId, i: usize) -> JsResult<Option<ObjId>> {
        Ok(self.xml_list(l)?.items.get(i).copied())
    }

    pub(crate) fn list_len(&self, l: ObjId) -> JsResult<usize> {
        Ok(self.xml_list(l)?.items.len())
    }

    // ---- charging and allocation ----

    pub(crate) fn xml_keep(&mut self, id: ObjId) {
        self.temp_roots.push(Value::Object(id));
    }

    fn xml_take_fuel(&mut self, n: usize) -> JsResult<()> {
        let n = n as u64;
        if n == 0 {
            return Ok(());
        }
        if self.fuel < n {
            self.fuel = 0;
            return Err(Abrupt::Limit(LimitKind::Fuel));
        }
        self.fuel -= n - 1;
        self.use_fuel()
    }

    fn xml_text_fuel(&mut self, units: usize) -> JsResult<()> {
        self.xml_take_fuel(units / UNITS_PER_FUEL + 1)
    }

    fn xml_charge(&mut self, bytes: usize) -> JsResult<()> {
        self.reserve_heap(bytes)?;
        self.heap.bytes = self.heap.bytes.saturating_add(bytes);
        Ok(())
    }

    /// Charges for the link that will attach the new node `node`, which is held as a root across
    /// any collection the charge triggers.
    fn xml_charge_link(&mut self, node: ObjId) -> JsResult<()> {
        self.temp_roots.push(Value::Object(node));
        let charged = self.xml_charge(XML_REF_BYTES);
        self.temp_roots.pop();
        charged
    }

    /// A new node. What `node` refers to must be reachable from the roots.
    pub(crate) fn new_xml_node(&mut self, node: XmlNode) -> JsResult<ObjId> {
        if let Some(name) = &node.name
            && matches!(node.kind, XmlKind::Element | XmlKind::Attribute)
            && !xml_name_writable(name, node.kind == XmlKind::Attribute)
        {
            return Err(self.type_error(RESERVED_NS_BINDING));
        }
        let extra = XML_NODE_BYTES + node.name.as_ref().map_or(0, name_bytes) + string_bytes(node.value.len());
        self.reserve_heap(OBJECT_BYTES + extra)?;
        self.heap.bytes = self.heap.bytes.saturating_add(extra);
        let proto = self.intrinsics.xml_proto;
        Ok(self.heap.alloc_object(Object::new(Class::Xml(Box::new(node)), Some(proto))))
    }

    pub(crate) fn new_text_node(&mut self, value: Str, parent: Option<ObjId>) -> JsResult<ObjId> {
        self.new_xml_node(XmlNode { kind: XmlKind::Text, name: None, parent, value, attrs: Vec::new(), children: Vec::new(), ns: Vec::new() })
    }

    /// A new list. Its target must be reachable from the roots.
    pub(crate) fn new_xml_list(&mut self, target: Option<ObjId>, target_name: Option<(XmlName, bool)>) -> JsResult<ObjId> {
        self.reserve_heap(OBJECT_BYTES + XML_LIST_BYTES)?;
        self.heap.bytes = self.heap.bytes.saturating_add(XML_LIST_BYTES);
        let proto = self.intrinsics.xml_list_proto;
        let data = XmlListData { items: Vec::new(), target, target_name };
        Ok(self.heap.alloc_object(Object::new(Class::XmlList(Box::new(data)), Some(proto))))
    }

    /// Adds `item` to the end of `list`. Both are held as roots across any collection the charge
    /// starts.
    pub(crate) fn list_push(&mut self, list: ObjId, item: ObjId) -> JsResult<()> {
        self.rooted(&[Value::Object(list), Value::Object(item)], |i| i.xml_charge(XML_REF_BYTES))?;
        self.xml_list_mut(list)?.items.push(item);
        Ok(())
    }

    /// A new name object made by E4X syntax.
    fn new_reference_name(&mut self, name: XmlName, attribute: bool) -> JsResult<ObjId> {
        let bytes = name_bytes(&name);
        self.reserve_heap(OBJECT_BYTES + bytes)?;
        self.heap.bytes = self.heap.bytes.saturating_add(bytes);
        let proto = self.intrinsics.qname_proto;
        Ok(self.heap.alloc_object(Object::new(Class::QName { name: Box::new(name), attribute, reference: true }, Some(proto))))
    }

    // ---- settings and the default namespace ----

    pub(crate) fn xml_settings(&mut self) -> JsResult<Settings> {
        let c = self.intrinsics.xml_ctor;
        let this = Value::Object(c);
        let read = |i: &mut Self, name: &str| i.get_from(c, &key(name), &this);
        let ignore_comments = read(self, "ignoreComments")?.truthy();
        let ignore_pis = read(self, "ignoreProcessingInstructions")?.truthy();
        let ignore_whitespace = read(self, "ignoreWhitespace")?.truthy();
        let pretty = read(self, "prettyPrinting")?.truthy();
        let v = read(self, "prettyIndent")?;
        let n = self.to_number(&v)?;
        let indent = if n.is_finite() && n > 0.0 { n.min(f64::from(u32::MAX)) as u32 } else { 0 };
        Ok(Settings { ignore_comments, ignore_pis, ignore_whitespace, pretty, indent })
    }

    /// The URI of the running code's default XML namespace (ECMA-357 section 12.1.1).
    pub(crate) fn xml_default_uri(&self) -> JsResult<Key> {
        let mut cur = Some(self.frame()?.env);
        while let Some(e) = cur {
            match self.heap.env(e) {
                Some(Env::Decl { bindings, outer }) => {
                    if let Some(Binding { value: Value::String(s), .. }) = bindings.get(DEFAULT_NS_BINDING) {
                        return Ok(s.flat());
                    }
                    cur = *outer;
                }
                Some(Env::Obj { outer, .. }) => cur = *outer,
                None => return Err(Abrupt::Internal("environment out of range".into())),
            }
        }
        Ok(self.xml_default_uri.clone())
    }

    /// Sets the default XML namespace of the running function, or of the global code.
    pub(crate) fn xml_set_default_uri(&mut self, uri: Key) -> JsResult<()> {
        if units_are(&uri, XMLNS_URI) {
            return Err(self.type_error(RESERVED_NS_BINDING));
        }
        let env = self.frame()?.var_env;
        if let Some(Env::Decl { bindings, .. }) = self.heap.env_mut(env) {
            let value = Value::String(Str::from_rc(uri));
            bindings.insert(Rc::from(DEFAULT_NS_BINDING), Binding { value, mutable: true, deletable: false });
        } else {
            self.xml_default_uri = uri;
        }
        Ok(())
    }

    // ---- names ----

    fn name_from_string(&mut self, s: &[u16]) -> JsResult<NameRef> {
        if let Some((&0x40, rest)) = s.split_first() {
            let uri = if rest == STAR { None } else { Some(Rc::from(Vec::new())) };
            return Ok(NameRef { uri, local: Rc::from(rest), attribute: true });
        }
        let uri = if s == STAR { None } else { Some(self.xml_default_uri()?) };
        Ok(NameRef { uri, local: Rc::from(s), attribute: false })
    }

    /// ToXMLName, with a canonical array index read as a position (ECMA-357 section 10.6).
    pub(crate) fn xml_access(&mut self, k: &Value) -> JsResult<Access> {
        match k {
            Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 && *n < 4_294_967_295.0 => Ok(Access::Index(*n as u32)),
            Value::String(s) => {
                let flat = s.flat();
                if let Some(i) = canonical_index(&flat) {
                    return Ok(Access::Index(i));
                }
                Ok(Access::Name(self.name_from_string(&flat)?))
            }
            Value::Object(o) => {
                if let Some(Class::QName { name, attribute, .. }) = self.heap.object(*o).map(|ob| &ob.class) {
                    return Ok(Access::Name(NameRef { uri: name.uri.clone(), local: name.local.clone(), attribute: *attribute }));
                }
                let s = self.to_str(k)?.flat();
                if let Some(i) = canonical_index(&s) {
                    return Ok(Access::Index(i));
                }
                Ok(Access::Name(self.name_from_string(&s)?))
            }
            _ => Err(self.type_error("the value is not an XML name")),
        }
    }

    /// The access a bracket, `in` or `delete` key selects. A key that is not a string or object
    /// converts with ToString first, as a property accessor does (ECMA-357 section 11.2.1).
    fn key_access(&mut self, k: &Value) -> JsResult<Access> {
        match k {
            Value::Undefined | Value::Null | Value::Bool(_) | Value::Number(_) => {
                let s = self.to_str(k)?;
                self.xml_access(&Value::String(s))
            }
            _ => self.xml_access(k),
        }
    }

    fn xml_name_only(&mut self, k: &Value) -> JsResult<NameRef> {
        match self.xml_access(k)? {
            Access::Name(n) => Ok(n),
            Access::Index(i) => self.name_from_string(&index_key(i)),
        }
    }

    pub(crate) fn xml_method_key(&mut self, k: &Value) -> JsResult<Key> {
        if let Value::Object(o) = k
            && let Some(Class::QName { name, .. }) = self.heap.object(*o).map(|ob| &ob.class)
        {
            return Ok(name.local.clone());
        }
        self.to_key(k)
    }

    // ---- [[Get]], [[HasProperty]], [[Delete]] ----

    fn not_xml(&mut self) -> Abrupt {
        self.type_error("the XML operation needs an XML or XMLList value")
    }

    fn collect_matching(&mut self, x: ObjId, n: &NameRef, out: ObjId) -> JsResult<()> {
        let count = if n.attribute { self.xml_node(x)?.attrs.len() } else { self.children_len(x)? };
        for i in 0..count {
            self.use_fuel()?;
            let node = self.xml_node(x)?;
            let found = if n.attribute {
                node.attrs.get(i).copied().filter(|&a| self.xml_node(a).is_ok_and(|an| an.name.as_ref().is_some_and(|q| n.matches(q))))
            } else {
                node.children.get(i).copied().filter(|&c| self.xml_node(c).is_ok_and(|cn| n.matches_child(cn)))
            };
            if let Some(found) = found {
                self.list_push(out, found)?;
            }
        }
        Ok(())
    }

    /// The items of `base` matching `n`, with `base` and `n` as the list's write target.
    pub(crate) fn xml_select(&mut self, base: ObjId, n: &NameRef) -> JsResult<ObjId> {
        self.rooted(&[Value::Object(base)], |i| {
            let out = i.new_xml_list(Some(base), Some((n.to_name(), n.attribute)))?;
            i.xml_keep(out);
            match i.classify(&Value::Object(base)) {
                Xv::Node(x) => i.collect_matching(x, n, out)?,
                Xv::List(l) => {
                    for at in 0..i.list_len(l)? {
                        i.use_fuel()?;
                        let Some(item) = i.list_at(l, at)? else { break };
                        if i.xml_kind(item)? == XmlKind::Element {
                            i.collect_matching(item, n, out)?;
                        }
                    }
                }
                Xv::Other => return Err(i.not_xml()),
            }
            Ok(out)
        })
    }

    /// [[Get]] (ECMA-357 sections 9.1.1.1 and 9.2.1.1).
    pub(crate) fn xml_get(&mut self, base: &Value, k: &Value) -> JsResult<Value> {
        let xv = self.classify(base);
        if matches!(xv, Xv::Other) {
            return Err(self.not_xml());
        }
        match self.key_access(k)? {
            Access::Index(i) => Ok(match xv {
                Xv::Node(_) if i == 0 => base.clone(),
                Xv::List(l) => self.list_at(l, i as usize)?.map_or(Value::Undefined, Value::Object),
                _ => Value::Undefined,
            }),
            Access::Name(n) => {
                let Value::Object(o) = base else { return Err(self.not_xml()) };
                Ok(Value::Object(self.xml_select(*o, &n)?))
            }
        }
    }

    fn node_has_name(&mut self, x: ObjId, n: &NameRef) -> JsResult<bool> {
        let (attrs, children) = {
            let node = self.xml_node(x)?;
            (node.attrs.len(), node.children.len())
        };
        self.xml_take_fuel((if n.attribute { attrs } else { children }) / UNITS_PER_FUEL + 1)?;
        let node = self.xml_node(x)?;
        if n.attribute {
            return Ok(node.attrs.iter().any(|&a| self.xml_node(a).is_ok_and(|an| an.name.as_ref().is_some_and(|q| n.matches(q)))));
        }
        Ok(node.children.iter().any(|&c| self.xml_node(c).is_ok_and(|cn| n.matches_child(cn))))
    }

    /// [[HasProperty]] (ECMA-357 sections 9.1.1.6 and 9.2.1.5).
    pub(crate) fn xml_has(&mut self, base: &Value, k: &Value) -> JsResult<bool> {
        let xv = self.classify(base);
        match (xv, self.key_access(k)?) {
            (Xv::Node(_), Access::Index(i)) => Ok(i == 0),
            (Xv::Node(x), Access::Name(n)) => self.node_has_name(x, &n),
            (Xv::List(l), Access::Index(i)) => Ok((i as usize) < self.list_len(l)?),
            (Xv::List(l), Access::Name(n)) => self.list_has_name(l, &n),
            (Xv::Other, _) => Err(self.not_xml()),
        }
    }

    fn list_has_name(&mut self, l: ObjId, n: &NameRef) -> JsResult<bool> {
        for at in 0..self.list_len(l)? {
            self.use_fuel()?;
            let Some(item) = self.list_at(l, at)? else { break };
            if self.xml_kind(item)? == XmlKind::Element && self.node_has_name(item, n)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether a scope object has the variable `name`.
    pub(crate) fn scope_has(&mut self, object: ObjId, name: &[u16]) -> JsResult<bool> {
        if self.options.e4x && self.is_xml(&Value::Object(object)) {
            let k = Value::String(Str::from_units(name.to_vec()));
            return self.xml_has(&Value::Object(object), &k);
        }
        self.has_property(object, name)
    }

    fn node_delete_at(&mut self, x: ObjId, i: usize) -> JsResult<()> {
        let len = self.children_len(x)?;
        if i < len {
            self.xml_text_fuel(len)?;
            self.remove_child(x, i)?;
        }
        Ok(())
    }

    /// Takes the child at `i` out of `x`. The caller has checked `i` and taken the fuel.
    fn remove_child(&mut self, x: ObjId, i: usize) -> JsResult<()> {
        let c = self.xml_node_mut(x)?.children.remove(i);
        self.xml_node_mut(c)?.parent = None;
        Ok(())
    }

    /// Takes `gone`, children of `x` in document order, out of `x` in one pass. The caller has
    /// taken the fuel.
    fn remove_children(&mut self, x: ObjId, gone: &[ObjId]) -> JsResult<()> {
        let mut next = gone.iter().peekable();
        self.xml_node_mut(x)?.children.retain(|c| {
            let hit = next.peek() == Some(&c);
            if hit {
                next.next();
            }
            !hit
        });
        for &c in gone {
            self.xml_node_mut(c)?.parent = None;
        }
        Ok(())
    }

    fn node_delete_name(&mut self, x: ObjId, n: &NameRef) -> JsResult<()> {
        if n.attribute {
            let count = self.xml_node(x)?.attrs.len();
            self.xml_take_fuel(count)?;
            let attrs = std::mem::take(&mut self.xml_node_mut(x)?.attrs);
            let mut kept = Vec::with_capacity(attrs.len());
            for a in attrs {
                let hit = self.xml_node(a)?.name.as_ref().is_some_and(|q| n.matches(q));
                if hit {
                    self.xml_node_mut(a)?.parent = None;
                } else {
                    kept.push(a);
                }
            }
            self.xml_node_mut(x)?.attrs = kept;
            return Ok(());
        }
        let count = self.children_len(x)?;
        self.xml_take_fuel(count)?;
        let children = std::mem::take(&mut self.xml_node_mut(x)?.children);
        let mut kept = Vec::with_capacity(children.len());
        for c in children {
            if n.matches_child(self.xml_node(c)?) {
                self.xml_node_mut(c)?.parent = None;
            } else {
                kept.push(c);
            }
        }
        self.xml_node_mut(x)?.children = kept;
        Ok(())
    }

    fn index_in_parent(&mut self, parent: ObjId, child: ObjId) -> JsResult<Option<usize>> {
        let len = self.children_len(parent)?;
        self.xml_text_fuel(len)?;
        Ok(self.xml_node(parent)?.children.iter().position(|&c| c == child))
    }

    /// [[Delete]] (ECMA-357 sections 9.1.1.3 and 9.2.1.3).
    pub(crate) fn xml_delete(&mut self, base: &Value, k: &Value) -> JsResult<bool> {
        let xv = self.classify(base);
        match (xv, self.key_access(k)?) {
            (Xv::Node(_), Access::Index(_)) => Err(self.type_error("an XML value's children cannot be deleted by position")),
            (Xv::Node(x), Access::Name(n)) => {
                self.node_delete_name(x, &n)?;
                Ok(true)
            }
            (Xv::List(l), Access::Index(i)) => {
                let Some(item) = self.list_at(l, i as usize)? else { return Ok(true) };
                let node = self.xml_node(item)?;
                let (parent, kind) = (node.parent, node.kind);
                if let Some(parent) = parent {
                    if kind == XmlKind::Attribute {
                        let attrs = &mut self.xml_node_mut(parent)?.attrs;
                        attrs.retain(|&a| a != item);
                        self.xml_node_mut(item)?.parent = None;
                    } else if let Some(q) = self.index_in_parent(parent, item)? {
                        self.node_delete_at(parent, q)?;
                    }
                }
                self.xml_list_mut(l)?.items.remove(i as usize);
                Ok(true)
            }
            (Xv::List(l), Access::Name(n)) => {
                for at in 0..self.list_len(l)? {
                    self.use_fuel()?;
                    let Some(item) = self.list_at(l, at)? else { break };
                    if self.xml_kind(item)? == XmlKind::Element {
                        self.node_delete_name(item, &n)?;
                    }
                }
                Ok(true)
            }
            (Xv::Other, _) => Err(self.not_xml()),
        }
    }

    /// The function a call `base.name(...)` runs and its `this` (ECMA-357 section 11.2.2.1). A
    /// call looks the name up among the methods, never among the children.
    pub(crate) fn xml_method(&mut self, base: &Value, name: &[u16]) -> JsResult<(Value, Value)> {
        let mut base = base.clone();
        loop {
            let Value::Object(o) = base else { return Err(self.not_xml()) };
            let f = self.get_from(o, name, &base)?;
            if !matches!(f, Value::Undefined) {
                return Ok((f, base));
            }
            match self.classify(&base) {
                Xv::List(l) if self.list_len(l)? == 1 => {
                    let Some(item) = self.list_at(l, 0)? else { return Ok((f, base)) };
                    base = Value::Object(item);
                }
                Xv::Node(x) if self.node_simple(x)? => {
                    let s = self.node_to_str(x)?;
                    let this = Value::String(s);
                    let f = self.get_value(&this, name)?;
                    return Ok((f, this));
                }
                _ => return Ok((f, base)),
            }
        }
    }

    /// A method found through an XML value in scope, for a call of a bare name in a filter.
    pub(crate) fn xml_scope_method(&mut self, name: &[u16]) -> JsResult<Option<(Value, Value)>> {
        let mut cur = Some(self.frame()?.env);
        while let Some(e) = cur {
            match self.heap.env(e) {
                Some(Env::Decl { bindings, outer }) => {
                    if bindings.contains(name) {
                        return Ok(None);
                    }
                    cur = *outer;
                }
                Some(&Env::Obj { object, outer, .. }) => {
                    if self.scope_has(object, name)? {
                        return Ok(None);
                    }
                    let this = Value::Object(object);
                    if self.is_xml(&this) {
                        let f = self.get_from(object, name, &this)?;
                        if self.is_callable(&f) {
                            return Ok(Some((f, this)));
                        }
                    }
                    cur = outer;
                }
                None => return Err(Abrupt::Internal("environment out of range".into())),
            }
        }
        Ok(None)
    }

    pub(crate) fn xml_enumerate(&mut self, id: ObjId) -> JsResult<Vec<Key>> {
        let len = match self.classify(&Value::Object(id)) {
            Xv::List(l) => self.list_len(l)?,
            _ => 1,
        };
        self.xml_take_fuel(len)?;
        self.xml_charge(len.saturating_mul(string_bytes(10)))?;
        Ok((0..len).map(|i| index_key(i as u32)).collect())
    }

    // ---- insertion ----

    fn check_attach_node(&mut self, x: ObjId, v: ObjId) -> JsResult<()> {
        let mut depth = 0usize;
        let mut cur = Some(x);
        while let Some(a) = cur {
            if a == v {
                return Err(self.type_error("an XML value cannot be inserted into itself or its descendants"));
            }
            depth += 1;
            self.use_fuel()?;
            cur = self.xml_node(a)?.parent;
        }
        let height = self.node_height(v)?;
        if depth.saturating_add(height) > self.options.max_depth {
            return Err(Abrupt::Limit(LimitKind::Heap));
        }
        Ok(())
    }

    /// Refuses, before anything changes, an insertion of `c` under `x` that would make a cycle or
    /// a tree past the depth limit.
    fn check_attach(&mut self, x: ObjId, c: &Content) -> JsResult<()> {
        match c {
            Content::Text(_) => Ok(()),
            Content::Node(v) => {
                if self.xml_kind(*v)? == XmlKind::Attribute {
                    return Ok(());
                }
                self.check_attach_node(x, *v)
            }
            Content::List(l) => {
                for at in 0..self.list_len(*l)? {
                    let Some(item) = self.list_at(*l, at)? else { break };
                    if self.xml_kind(item)? != XmlKind::Attribute {
                        self.check_attach_node(x, item)?;
                    }
                }
                Ok(())
            }
        }
    }

    /// The levels of the tree at `root`, counting it.
    fn node_height(&mut self, root: ObjId) -> JsResult<usize> {
        let mut stack = vec![(root, 0usize)];
        let mut height = 1;
        while let Some(&(node, next)) = stack.last() {
            self.use_fuel()?;
            match self.child_at(node, next)? {
                Some(c) => {
                    if let Some(top) = stack.last_mut() {
                        top.1 += 1;
                    }
                    stack.push((c, 0));
                    height = height.max(stack.len());
                }
                None => {
                    stack.pop();
                }
            }
        }
        Ok(height)
    }

    /// Removes `v` from its parent, so a node is never in two lists of children. Returns the
    /// position in `x` to use after the removal. The caller has taken [`Self::place_fuel`].
    fn detach(&mut self, x: ObjId, v: ObjId, at: usize) -> JsResult<usize> {
        let Some(parent) = self.xml_node(v)?.parent else { return Ok(at) };
        let Some(q) = self.xml_node(parent)?.children.iter().position(|&c| c == v) else { return Ok(at) };
        self.remove_child(parent, q)?;
        Ok(if parent == x && q < at { at - 1 } else { at })
    }

    /// Takes the fuel to put `v` among `len` children: the insertion and, when `v` has a parent,
    /// the scan and removal that take it out.
    fn place_fuel(&mut self, len: usize, v: ObjId) -> JsResult<()> {
        self.xml_text_fuel(len + 1)?;
        if let Some(parent) = self.xml_node(v)?.parent {
            let old = self.children_len(parent)?;
            self.xml_text_fuel(old)?;
            self.xml_text_fuel(old)?;
        }
        Ok(())
    }

    /// Moves `v` to position `at` of `x` and returns where it went. The caller has taken the fuel
    /// and heap.
    fn place_child(&mut self, x: ObjId, at: usize, v: ObjId) -> JsResult<usize> {
        let at = self.detach(x, v, at)?;
        let node = self.xml_node_mut(x)?;
        let at = at.min(node.children.len());
        node.children.insert(at, v);
        self.xml_node_mut(v)?.parent = Some(x);
        Ok(at)
    }

    /// Charges for one more child of `x`. `x` and `v` are held as roots across any collection.
    fn xml_charge_child(&mut self, x: ObjId, v: ObjId) -> JsResult<()> {
        self.rooted(&[Value::Object(x), Value::Object(v)], |i| i.xml_charge(XML_REF_BYTES))
    }

    /// Takes the fuel and heap for the insertion first, so a limit leaves `v` where it was.
    fn insert_node(&mut self, x: ObjId, at: usize, v: ObjId) -> JsResult<usize> {
        self.xml_charge_child(x, v)?;
        let len = self.children_len(x)?;
        self.place_fuel(len, v)?;
        self.place_child(x, at, v)
    }

    /// The node to place as a child for `item`: `item` itself, or a new text node holding the
    /// value when it is an attribute. `roots` are held across any collection.
    fn child_for_item(&mut self, item: ObjId, roots: &[Value]) -> JsResult<ObjId> {
        if self.xml_kind(item)? != XmlKind::Attribute {
            return Ok(item);
        }
        let value = self.xml_node(item)?.value.clone();
        let t = self.rooted(roots, |i| i.new_text_node(value, None))?;
        self.xml_keep(t);
        Ok(t)
    }

    /// Takes the fuel and heap to put every item of `l` under `x` and returns the nodes to place,
    /// from [`Self::child_for_item`]. The tree does not change here, so a limit leaves it as it
    /// was.
    fn reserve_list_insert(&mut self, x: ObjId, l: ObjId) -> JsResult<Vec<ObjId>> {
        let mut items = self.xml_list(l)?.items.clone();
        let bytes = XML_REF_BYTES.saturating_mul(items.len());
        let roots = [Value::Object(x), Value::Object(l)];
        self.rooted(&roots, |i| i.xml_charge(bytes))?;
        let len = self.children_len(x)?;
        for (j, item) in items.iter_mut().enumerate() {
            *item = self.child_for_item(*item, &roots)?;
            self.place_fuel(len + j, *item)?;
        }
        Ok(items)
    }

    /// Puts `items` at `at` in order, after [`Self::reserve_list_insert`].
    fn place_children(&mut self, x: ObjId, at: usize, items: &[ObjId]) -> JsResult<()> {
        let mut at = at;
        for &item in items {
            at = self.place_child(x, at, item)? + 1;
        }
        Ok(())
    }

    /// Puts a new text node holding `s` at position `at` of `x`.
    fn insert_text(&mut self, x: ObjId, at: usize, s: &Str) -> JsResult<()> {
        let t = self.new_text_node(s.clone(), Some(x))?;
        self.xml_keep(t);
        self.xml_charge(XML_REF_BYTES)?;
        let node = self.xml_node_mut(x)?;
        let at = at.min(node.children.len());
        node.children.insert(at, t);
        Ok(())
    }

    /// [[Insert]] (ECMA-357 section 9.1.1.11).
    pub(crate) fn xml_insert(&mut self, x: ObjId, at: usize, c: &Content) -> JsResult<()> {
        if self.xml_kind(x)? != XmlKind::Element {
            return Ok(());
        }
        self.check_attach(x, c)?;
        match c {
            Content::Text(s) => self.insert_text(x, at, s)?,
            Content::Node(v) if self.xml_kind(*v)? == XmlKind::Attribute => {
                let value = self.xml_node(*v)?.value.clone();
                self.insert_text(x, at, &value)?;
            }
            Content::Node(v) => {
                self.insert_node(x, at, *v)?;
            }
            Content::List(l) => {
                let items = self.reserve_list_insert(x, *l)?;
                self.place_children(x, at, &items)?;
            }
        }
        Ok(())
    }

    /// Charges for a [`Self::set_child`] of `v` at `at` that adds a child.
    fn charge_set_child(&mut self, x: ObjId, at: usize, v: ObjId) -> JsResult<()> {
        if at < self.children_len(x)? {
            return Ok(());
        }
        self.xml_charge_child(x, v)
    }

    /// Puts `v` at `at`, or last when `at` is past the end, after [`Self::charge_set_child`].
    fn set_child(&mut self, x: ObjId, at: usize, v: ObjId) -> JsResult<()> {
        let node = self.xml_node_mut(x)?;
        let old = match node.children.get_mut(at) {
            Some(slot) => Some(std::mem::replace(slot, v)),
            None => {
                node.children.push(v);
                None
            }
        };
        if let Some(old) = old {
            self.xml_node_mut(old)?.parent = None;
        }
        self.xml_node_mut(v)?.parent = Some(x);
        Ok(())
    }

    /// [[Replace]] (ECMA-357 section 9.1.1.12).
    pub(crate) fn xml_replace(&mut self, x: ObjId, at: usize, c: &Content) -> JsResult<()> {
        if self.xml_kind(x)? != XmlKind::Element {
            return Ok(());
        }
        let at = at.min(self.children_len(x)?);
        match c {
            Content::Node(v) if self.xml_kind(*v)? != XmlKind::Attribute => {
                self.check_attach(x, c)?;
                if self.child_at(x, at)? == Some(*v) {
                    return Ok(());
                }
                self.charge_set_child(x, at, *v)?;
                let len = self.children_len(x)?;
                self.place_fuel(len, *v)?;
                let at = self.detach(x, *v, at)?;
                self.set_child(x, at, *v)
            }
            Content::List(l) => {
                self.check_attach(x, c)?;
                let items = self.reserve_list_insert(x, *l)?;
                let len = self.children_len(x)?;
                self.xml_text_fuel(len)?;
                if at < len {
                    self.remove_child(x, at)?;
                }
                self.place_children(x, at, &items)
            }
            Content::Node(v) => {
                let s = self.xml_node(*v)?.value.clone();
                let t = self.new_text_node(s, Some(x))?;
                self.xml_keep(t);
                self.charge_set_child(x, at, t)?;
                self.set_child(x, at, t)
            }
            Content::Text(s) => {
                let t = self.new_text_node(s.clone(), Some(x))?;
                self.xml_keep(t);
                self.charge_set_child(x, at, t)?;
                self.set_child(x, at, t)
            }
        }
    }

    /// Replaces the first of `matches`, children of `x` in document order, with `c` and removes
    /// the rest; appends `c` when there are none. A limit leaves `x` as it was. The value in `c`
    /// is never one of `matches`.
    pub(crate) fn xml_replace_matches(&mut self, x: ObjId, matches: &[ObjId], c: &Content) -> JsResult<()> {
        let len = self.children_len(x)?;
        self.xml_text_fuel(len)?;
        self.xml_text_fuel(len)?;
        let at = match matches.first() {
            Some(first) => self.xml_node(x)?.children.iter().position(|c| c == first).unwrap_or(len),
            None => len,
        };
        self.xml_replace(x, at, c)?;
        self.remove_children(x, matches.get(1..).unwrap_or_default())
    }

    // ---- [[Put]] ----

    /// A value to assign into a tree, copied when it is XML (steps 3 and 4 of section 9.1.1.2).
    fn content_for_put(&mut self, v: &Value) -> JsResult<Content> {
        match self.classify(v) {
            Xv::Node(n) if !matches!(self.xml_kind(n)?, XmlKind::Text | XmlKind::Attribute) => {
                let c = self.node_copy(n)?;
                self.xml_keep(c);
                Ok(Content::Node(c))
            }
            Xv::List(l) => {
                let c = self.list_copy(l)?;
                self.xml_keep(c);
                Ok(Content::List(c))
            }
            _ => Ok(Content::Text(self.to_str(v)?)),
        }
    }

    fn content_to_str(&mut self, c: &Content) -> JsResult<Str> {
        match c {
            Content::Text(s) => Ok(s.clone()),
            Content::Node(n) => self.node_to_str(*n),
            Content::List(l) => self.list_to_str(*l),
        }
    }

    fn put_attribute(&mut self, x: ObjId, n: &NameRef, c: &Content) -> JsResult<()> {
        if !is_ncname(&n.local) {
            return Ok(());
        }
        let value = if let Content::List(l) = c {
            let mut s = Builder::new();
            for i in 0..self.list_len(*l)? {
                if i > 0 {
                    s.push_units(self, &[SPACE])?;
                }
                let Some(item) = self.list_at(*l, i)? else { break };
                let text = self.node_to_str(item)?.flat();
                s.push_units(self, &text)?;
            }
            let Value::String(s) = s.finish(self) else { return Err(Abrupt::Internal("not a string".into())) };
            s
        } else {
            self.content_to_str(c)?
        };
        let mut found = None;
        let count = self.xml_node(x)?.attrs.len();
        self.xml_take_fuel(count)?;
        let attrs = std::mem::take(&mut self.xml_node_mut(x)?.attrs);
        let mut kept = Vec::with_capacity(attrs.len());
        for a in attrs {
            let hit = self.xml_node(a)?.name.as_ref().is_some_and(|q| n.matches(q));
            if hit && found.is_some() {
                self.xml_node_mut(a)?.parent = None;
            } else {
                if hit {
                    found = Some(a);
                }
                kept.push(a);
            }
        }
        self.xml_node_mut(x)?.attrs = kept;
        if let Some(a) = found {
            self.xml_node_mut(a)?.value = value;
            return Ok(());
        }
        let name = XmlName { uri: Some(n.uri.clone().unwrap_or_else(|| Rc::from(Vec::new()))), local: n.local.clone(), prefix: None };
        let a = self.new_xml_node(XmlNode {
            kind: XmlKind::Attribute,
            name: Some(name),
            parent: Some(x),
            value,
            attrs: Vec::new(),
            children: Vec::new(),
            ns: Vec::new(),
        })?;
        self.xml_keep(a);
        self.xml_charge(XML_REF_BYTES)?;
        self.xml_node_mut(x)?.attrs.push(a);
        Ok(())
    }

    fn put_element(&mut self, x: ObjId, n: &NameRef, c: &Content) -> JsResult<()> {
        if !is_ncname(&n.local) && !n.is_star() {
            return Ok(());
        }
        let len = self.children_len(x)?;
        self.xml_take_fuel(len)?;
        let mut matches = Vec::new();
        for k in 0..len {
            let Some(child) = self.child_at(x, k)? else { break };
            if n.matches_child(self.xml_node(child)?) {
                matches.push(child);
            }
        }
        match c {
            Content::Text(s) if !n.is_star() => self.put_element_text(x, n, &matches, s),
            _ => self.xml_replace_matches(x, &matches, c),
        }
    }

    /// Makes `s` the only content of the first of `matches`, or of a new element named `n` when
    /// there are none, and removes the other matches. A limit leaves `x` as it was.
    fn put_element_text(&mut self, x: ObjId, n: &NameRef, matches: &[ObjId], s: &Str) -> JsResult<()> {
        let at = self.children_len(x)?;
        self.xml_text_fuel(at)?;
        let target = match matches.first() {
            Some(&first) => first,
            None => {
                let uri = match &n.uri {
                    Some(u) => u.clone(),
                    None => self.xml_default_uri()?,
                };
                let y = self.new_xml_node(XmlNode {
                    kind: XmlKind::Element,
                    name: Some(XmlName { uri: Some(uri), local: n.local.clone(), prefix: None }),
                    parent: Some(x),
                    value: Str::from(""),
                    attrs: Vec::new(),
                    children: Vec::new(),
                    ns: Vec::new(),
                })?;
                self.xml_keep(y);
                self.check_attach_node(x, y)?;
                self.charge_set_child(x, at, y)?;
                y
            }
        };
        let count = self.children_len(target)?;
        self.xml_take_fuel(count)?;
        let text = if s.len() > 0 {
            let t = self.new_text_node(s.clone(), Some(target))?;
            self.xml_keep(t);
            self.xml_charge_child(target, t)?;
            Some(t)
        } else {
            None
        };
        if matches.is_empty() {
            self.set_child(x, at, target)?;
        }
        self.remove_children(x, matches.get(1..).unwrap_or_default())?;
        let children = std::mem::take(&mut self.xml_node_mut(target)?.children);
        for child in children {
            self.xml_node_mut(child)?.parent = None;
        }
        self.xml_node_mut(target)?.children.extend(text);
        Ok(())
    }

    fn node_put(&mut self, x: ObjId, n: &NameRef, v: &Value) -> JsResult<()> {
        if self.xml_kind(x)? != XmlKind::Element {
            return Ok(());
        }
        let c = self.content_for_put(v)?;
        if n.attribute { self.put_attribute(x, n, &c) } else { self.put_element(x, n, &c) }
    }

    /// [[ResolveValue]] (ECMA-357 section 9.2.1.10), walking the targets with a loop. Also returns
    /// the outermost element it added, if any. An error takes the added elements back out.
    fn resolve_value(&mut self, x: ObjId) -> JsResult<Option<(ObjId, Option<ObjId>)>> {
        let mut created = None;
        let r = self.resolve_value_adding(x, &mut created);
        match (r, created) {
            (Ok(Some(base)), created) => Ok(Some((base, created))),
            (Ok(None), _) => Ok(None),
            (Err(e), Some(c)) => {
                self.undo_created(c)?;
                Err(e)
            }
            (Err(e), None) => Err(e),
        }
    }

    /// Takes `created`, an element a path assignment added, back out of its parent. It takes no
    /// fuel or heap.
    fn undo_created(&mut self, created: ObjId) -> JsResult<()> {
        if let Some(parent) = self.xml_node(created)?.parent {
            self.xml_node_mut(parent)?.children.retain(|&c| c != created);
            self.xml_node_mut(created)?.parent = None;
        }
        Ok(())
    }

    /// The steps of [`Self::resolve_value`], setting `created` to the first element they add.
    fn resolve_value_adding(&mut self, x: ObjId, created: &mut Option<ObjId>) -> JsResult<Option<ObjId>> {
        let mut chain = Vec::new();
        let mut cur = x;
        let base = loop {
            self.use_fuel()?;
            let Xv::List(l) = self.classify(&Value::Object(cur)) else { break cur };
            if self.list_len(l)? > 0 {
                break cur;
            }
            let data = self.xml_list(l)?;
            let (Some(target), Some((name, false))) = (data.target, data.target_name.as_ref()) else { return Ok(None) };
            if *name.local == *STAR {
                return Ok(None);
            }
            chain.push(l);
            cur = target;
        };
        let mut base = base;
        self.xml_keep(base);
        while let Some(l) = chain.pop() {
            let Some((name, attribute)) = self.xml_list(l)?.target_name.clone() else { return Ok(None) };
            let n = NameRef { uri: name.uri, local: name.local, attribute };
            let mut target = self.xml_select(base, &n)?;
            self.xml_keep(target);
            if self.list_len(target)? == 0 {
                if let Xv::List(b) = self.classify(&Value::Object(base))
                    && self.list_len(b)? > 1
                {
                    return Ok(None);
                }
                let owner = match self.classify(&Value::Object(base)) {
                    Xv::Node(b) => Some(b),
                    Xv::List(b) => self.list_at(b, 0)?,
                    Xv::Other => None,
                };
                let before = match owner {
                    Some(o) => self.children_len(o)?,
                    None => 0,
                };
                self.put_name(base, &n, &Value::str(""))?;
                if created.is_none()
                    && let Some(o) = owner
                    && self.children_len(o)? > before
                {
                    *created = self.xml_node(o)?.children.last().copied();
                }
                target = self.xml_select(base, &n)?;
                self.xml_keep(target);
            }
            base = target;
        }
        Ok(Some(base))
    }

    fn put_name(&mut self, base: ObjId, n: &NameRef, v: &Value) -> JsResult<()> {
        match self.classify(&Value::Object(base)) {
            Xv::Node(x) => self.node_put(x, n, v),
            Xv::List(l) => {
                if self.list_len(l)? == 0 {
                    let Some((r, created)) = self.resolve_value(l)? else { return Ok(()) };
                    let done = self.put_resolved(l, r, n, v);
                    if done.is_err()
                        && let Some(c) = created
                    {
                        self.undo_created(c)?;
                        self.xml_list_mut(l)?.items.clear();
                    }
                    return done;
                }
                if self.list_len(l)? == 1
                    && let Some(item) = self.list_at(l, 0)?
                {
                    self.node_put(item, n, v)?;
                }
                Ok(())
            }
            Xv::Other => Err(self.not_xml()),
        }
    }

    /// Assigns `v` to the name `n` of `r`, the value [`Self::resolve_value`] gave for the empty
    /// list `l`, when `r` is one node, and makes that node the item of `l`.
    fn put_resolved(&mut self, l: ObjId, r: ObjId, n: &NameRef, v: &Value) -> JsResult<()> {
        let single = match self.classify(&Value::Object(r)) {
            Xv::List(rl) if self.list_len(rl)? == 1 => self.list_at(rl, 0)?,
            Xv::Node(rn) => Some(rn),
            _ => None,
        };
        let Some(single) = single else { return Ok(()) };
        self.list_push(l, single)?;
        self.node_put(single, n, v)
    }

    fn list_put_index(&mut self, x: ObjId, index: usize, v: &Value) -> JsResult<()> {
        let data = self.xml_list(x)?;
        let target = data.target;
        let target_name = data.target_name.clone();
        let (r, created) = match target {
            Some(t) => match self.resolve_value(t)? {
                Some((r, created)) => (Some(r), created),
                None => return Ok(()),
            },
            None => (None, None),
        };
        let put = self.list_put_resolved(x, index, v, r, target_name.as_ref());
        if put.is_err()
            && let Some(c) = created
        {
            self.undo_created(c)?;
        }
        put
    }

    /// The steps of [`Self::list_put_index`] after the target `r` is resolved.
    fn list_put_resolved(&mut self, x: ObjId, index: usize, v: &Value, r: Option<ObjId>, target_name: Option<&(XmlName, bool)>) -> JsResult<()> {
        if index >= self.list_len(x)? {
            let r = match r {
                Some(r) => match self.classify(&Value::Object(r)) {
                    Xv::List(rl) => {
                        if self.list_len(rl)? != 1 {
                            return Ok(());
                        }
                        self.list_at(rl, 0)?
                    }
                    _ => Some(r),
                },
                None => None,
            };
            if let Some(r) = r
                && self.xml_kind(r)? != XmlKind::Element
            {
                return Ok(());
            }
            let (kind, name) = match target_name {
                Some((name, true)) => {
                    if let Some(r) = r {
                        let n = NameRef { uri: name.uri.clone(), local: name.local.clone(), attribute: true };
                        if self.node_has_name(r, &n)? {
                            return Ok(());
                        }
                    }
                    (XmlKind::Attribute, Some(name.clone()))
                }
                Some((name, false)) if *name.local != *STAR => (XmlKind::Element, Some(name.clone())),
                _ => (XmlKind::Text, None),
            };
            if let Some(r) = r {
                match self.classify(v) {
                    Xv::Node(n) if kind != XmlKind::Attribute => self.check_attach(r, &Content::Node(n))?,
                    Xv::List(l) if kind != XmlKind::Attribute => self.check_attach(r, &Content::List(l))?,
                    _ => {}
                }
            }
            let y =
                self.new_xml_node(XmlNode { kind, name, parent: r, value: Str::from(""), attrs: Vec::new(), children: Vec::new(), ns: Vec::new() })?;
            self.xml_keep(y);
            let i = self.list_len(x)?;
            let put = self.list_append_new(x, i, r, kind, y).and_then(|()| self.list_put_at(x, i, v));
            if put.is_err() {
                self.undo_list_append(x, y)?;
            }
            return put;
        }
        self.list_put_at(x, index, v)
    }

    /// Adds the new node `y` as item `i` of `x` and, when there is a parent `r`, to the tree.
    fn list_append_new(&mut self, x: ObjId, i: usize, r: Option<ObjId>, kind: XmlKind, y: ObjId) -> JsResult<()> {
        if kind == XmlKind::Attribute {
            if let Some(r) = r {
                self.xml_charge(XML_REF_BYTES)?;
                self.xml_node_mut(r)?.attrs.push(y);
            }
        } else if let Some(r) = r {
            let len = self.children_len(r)?;
            let at = if i > 0 {
                let prev = self.list_at(x, i - 1)?;
                let position = match prev {
                    Some(prev) => self.index_in_parent(r, prev)?,
                    None => None,
                };
                position.map_or(len, |j| j + 1)
            } else {
                len
            };
            self.check_attach_node(r, y)?;
            self.insert_node(r, at, y)?;
        }
        self.list_push(x, y)
    }

    /// Takes `y` out of its parent and of `x` after an assignment that added it stopped. It takes
    /// no fuel or heap, so it runs after a limit.
    fn undo_list_append(&mut self, x: ObjId, y: ObjId) -> JsResult<()> {
        if let Some(parent) = self.xml_node(y)?.parent {
            let node = self.xml_node_mut(parent)?;
            node.children.retain(|&c| c != y);
            node.attrs.retain(|&a| a != y);
            self.xml_node_mut(y)?.parent = None;
        }
        self.xml_list_mut(x)?.items.retain(|&item| item != y);
        Ok(())
    }

    /// Assigns `v` to item `i` of `x`.
    fn list_put_at(&mut self, x: ObjId, i: usize, v: &Value) -> JsResult<()> {
        let Some(item) = self.list_at(x, i)? else { return Ok(()) };
        let xv = self.classify(v);
        let value = match xv {
            Xv::Node(n) if !matches!(self.xml_kind(n)?, XmlKind::Text | XmlKind::Attribute) => Content::Node(n),
            Xv::List(l) => Content::List(l),
            _ => Content::Text(self.to_str(v)?),
        };
        let item_node = self.xml_node(item)?;
        let (item_kind, parent) = (item_node.kind, item_node.parent);
        if item_kind == XmlKind::Attribute {
            let Some(parent) = parent else { return Ok(()) };
            let name = self.xml_node(item)?.name.clone();
            let Some(name) = name else { return Ok(()) };
            let n = NameRef { uri: name.uri, local: name.local, attribute: true };
            let s = self.content_to_str(&value)?;
            self.put_attribute(parent, &n, &Content::Text(s))?;
            let attrs = self.xml_select(parent, &n)?;
            if let Some(a) = self.list_at(attrs, 0)?
                && let Some(slot) = self.xml_list_mut(x)?.items.get_mut(i)
            {
                *slot = a;
            }
            return Ok(());
        }
        match value {
            Content::List(l) => {
                let c = self.list_shallow_copy(l)?;
                self.xml_keep(c);
                let place = match parent {
                    Some(parent) => self.index_in_parent(parent, item)?.map(|q| (parent, q)),
                    None => None,
                };
                let mut items = self.xml_list(c)?.items.clone();
                if place.is_some() {
                    let roots = [Value::Object(x), Value::Object(c)];
                    for item in &mut items {
                        self.use_fuel()?;
                        *item = self.child_for_item(*item, &roots)?;
                    }
                    self.xml_list_mut(c)?.items.clone_from(&items);
                }
                self.xml_charge(XML_REF_BYTES.saturating_mul(items.len()))?;
                if let Some((parent, q)) = place {
                    self.xml_replace(parent, q, &Content::List(c))?;
                }
                self.xml_list_mut(x)?.items.splice(i..=i, items);
                Ok(())
            }
            Content::Node(_) => self.list_replace_item(x, i, item, parent, &value),
            Content::Text(_) if matches!(item_kind, XmlKind::Text | XmlKind::Comment | XmlKind::Pi) => {
                self.list_replace_item(x, i, item, parent, &value)
            }
            Content::Text(_) => self.node_put(item, &NameRef { uri: None, local: Rc::from(STAR), attribute: false }, v),
        }
    }

    fn list_replace_item(&mut self, x: ObjId, i: usize, item: ObjId, parent: Option<ObjId>, value: &Content) -> JsResult<()> {
        let mut replaced = None;
        if let Some(parent) = parent
            && let Some(q) = self.index_in_parent(parent, item)?
        {
            self.xml_replace(parent, q, value)?;
            replaced = self.child_at(parent, q)?;
        }
        let new_item = match (replaced, value) {
            (_, Content::Node(n)) => *n,
            (Some(r), Content::Text(_)) => r,
            (None, Content::Text(s)) => {
                let t = self.new_text_node(s.clone(), None)?;
                self.xml_keep(t);
                t
            }
            (_, Content::List(_)) => return Ok(()),
        };
        if let Some(slot) = self.xml_list_mut(x)?.items.get_mut(i) {
            *slot = new_item;
        }
        Ok(())
    }

    /// [[Put]] (ECMA-357 sections 9.1.1.2 and 9.2.1.2).
    pub(crate) fn xml_put(&mut self, base: &Value, k: &Value, v: &Value) -> JsResult<()> {
        self.rooted(&[base.clone(), v.clone()], |i| {
            let access = i.key_access(k)?;
            match (i.classify(base), access) {
                (Xv::Node(_), Access::Index(_)) => Err(i.type_error("an XML value's children cannot be assigned by position")),
                (Xv::List(l), Access::Index(at)) => i.list_put_index(l, at as usize, v),
                (_, Access::Name(n)) => {
                    let Value::Object(o) = base else { return Err(i.not_xml()) };
                    i.put_name(*o, &n, v)
                }
                (Xv::Other, _) => Err(i.not_xml()),
            }
        })
    }

    // ---- copies ----

    fn copy_shallow(&mut self, src: ObjId, parent: Option<ObjId>) -> JsResult<ObjId> {
        let node = self.xml_node(src)?;
        let copy = XmlNode {
            kind: node.kind,
            name: node.name.clone(),
            parent,
            value: node.value.clone(),
            attrs: Vec::new(),
            children: Vec::new(),
            ns: node.ns.clone(),
        };
        let attrs = node.attrs.clone();
        self.xml_charge(XML_NS_BYTES.saturating_mul(copy.ns.len()))?;
        let c = self.new_xml_node(copy)?;
        self.xml_keep(c);
        for a in attrs {
            self.use_fuel()?;
            let n = self.xml_node(a)?;
            let attr = XmlNode {
                kind: XmlKind::Attribute,
                name: n.name.clone(),
                parent: Some(c),
                value: n.value.clone(),
                attrs: Vec::new(),
                children: Vec::new(),
                ns: Vec::new(),
            };
            let ac = self.new_xml_node(attr)?;
            self.xml_charge_link(ac)?;
            self.xml_node_mut(c)?.attrs.push(ac);
        }
        Ok(c)
    }

    /// [[DeepCopy]] of an XML value (ECMA-357 section 9.1.1.7), with no parent.
    pub(crate) fn node_copy(&mut self, root: ObjId) -> JsResult<ObjId> {
        self.rooted(&[Value::Object(root)], |i| {
            let top = i.copy_shallow(root, None)?;
            i.xml_keep(top);
            let mark = i.temp_roots.len();
            let mut stack = vec![(root, top, 0usize)];
            while let Some(&(src, dst, next)) = stack.last() {
                i.use_fuel()?;
                i.temp_roots.truncate(mark);
                match i.child_at(src, next)? {
                    Some(child) => {
                        if let Some(frame) = stack.last_mut() {
                            frame.2 += 1;
                        }
                        let c = i.copy_shallow(child, Some(dst))?;
                        i.xml_charge_link(c)?;
                        i.xml_node_mut(dst)?.children.push(c);
                        stack.push((child, c, 0));
                    }
                    None => {
                        stack.pop();
                    }
                }
            }
            Ok(top)
        })
    }

    /// A list of copies of the items of `l`, with its target (ECMA-357 section 9.2.1.7).
    pub(crate) fn list_copy(&mut self, l: ObjId) -> JsResult<ObjId> {
        self.rooted(&[Value::Object(l)], |i| {
            let data = i.xml_list(l)?;
            let (target, name) = (data.target, data.target_name.clone());
            let out = i.new_xml_list(target, name)?;
            i.xml_keep(out);
            for at in 0..i.list_len(l)? {
                let Some(item) = i.list_at(l, at)? else { break };
                let c = i.node_copy(item)?;
                i.list_push(out, c)?;
            }
            Ok(out)
        })
    }

    /// A new list holding the same nodes as `l`.
    pub(crate) fn list_shallow_copy(&mut self, l: ObjId) -> JsResult<ObjId> {
        self.rooted(&[Value::Object(l)], |i| {
            let data = i.xml_list(l)?;
            let (target, name) = (data.target, data.target_name.clone());
            let out = i.new_xml_list(target, name)?;
            i.xml_keep(out);
            for at in 0..i.list_len(l)? {
                i.use_fuel()?;
                let Some(item) = i.list_at(l, at)? else { break };
                i.list_push(out, item)?;
            }
            Ok(out)
        })
    }

    // ---- comparison ----

    fn same_shallow(&mut self, a: ObjId, b: ObjId) -> JsResult<bool> {
        let (x, y) = (self.xml_node(a)?, self.xml_node(b)?);
        if x.kind != y.kind || x.attrs.len() != y.attrs.len() || x.children.len() != y.children.len() || x.value.flat() != y.value.flat() {
            return Ok(false);
        }
        match (&x.name, &y.name) {
            (None, None) => {}
            (Some(p), Some(q)) if p.local == q.local && p.uri == q.uri => {}
            _ => return Ok(false),
        }
        if x.attrs.is_empty() {
            return Ok(true);
        }
        let count = x.attrs.len();
        self.xml_take_fuel(count)?;
        let mut held = HeldBytes::new(self);
        held.add(self, ATTR_COUNT_BYTES.saturating_mul(count))?;
        let (x, y) = (self.xml_node(a)?, self.xml_node(b)?);
        let (xa, ya) = (x.attrs.clone(), y.attrs.clone());
        let mut counts: HashMap<(Option<Key>, Key, Key), usize> = HashMap::new();
        let attr_key = |i: &Self, id: ObjId| -> JsResult<(Option<Key>, Key, Key)> {
            let n = i.xml_node(id)?;
            let name = n.name.as_ref().ok_or_else(|| Abrupt::Internal("attribute without a name".into()))?;
            Ok((name.uri.clone(), name.local.clone(), n.value.flat()))
        };
        for a in xa {
            *counts.entry(attr_key(self, a)?).or_default() += 1;
        }
        for b in ya {
            match counts.get_mut(&attr_key(self, b)?) {
                Some(n) if *n > 0 => *n -= 1,
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    /// [[Equals]] of two XML values (ECMA-357 section 9.1.1.9). The bytes it holds count against
    /// the heap limit, and `a` and `b` are held as roots.
    pub(crate) fn node_equals(&mut self, a: ObjId, b: ObjId) -> JsResult<bool> {
        self.rooted(&[Value::Object(a), Value::Object(b)], |i| {
            if !i.same_shallow(a, b)? {
                return Ok(false);
            }
            let mut held = HeldBytes::new(i);
            held.add(i, EQUALS_FRAME_BYTES)?;
            let mut stack = vec![(a, b, 0usize)];
            while let Some(&(x, y, next)) = stack.last() {
                i.use_fuel()?;
                let (Some(cx), Some(cy)) = (i.child_at(x, next)?, i.child_at(y, next)?) else {
                    stack.pop();
                    continue;
                };
                if let Some(frame) = stack.last_mut() {
                    frame.2 += 1;
                }
                if !i.same_shallow(cx, cy)? {
                    return Ok(false);
                }
                if stack.len() == stack.capacity() {
                    held.add(i, EQUALS_FRAME_BYTES.saturating_mul(stack.len()))?;
                }
                stack.push((cx, cy, 0));
            }
            Ok(true)
        })
    }

    fn list_equals(&mut self, l: ObjId, v: &Value) -> JsResult<bool> {
        let len = self.list_len(l)?;
        if matches!(v, Value::Undefined) && len == 0 {
            return Ok(true);
        }
        if let Xv::List(other) = self.classify(v) {
            if len != self.list_len(other)? {
                return Ok(false);
            }
            for i in 0..len {
                let (Some(a), Some(b)) = (self.list_at(l, i)?, self.list_at(other, i)?) else { return Ok(false) };
                if !self.loose_eq(&Value::Object(a), &Value::Object(b))? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if len == 1
            && let Some(item) = self.list_at(l, 0)?
        {
            return self.loose_eq(&Value::Object(item), v);
        }
        Ok(false)
    }

    /// The E4X steps of the `==` operator (ECMA-357 section 11.5.1), or `None` when the ordinary
    /// steps apply.
    pub(crate) fn xml_loose_eq(&mut self, a: &Value, b: &Value) -> JsResult<Option<bool>> {
        if let Xv::List(l) = self.classify(a) {
            return self.list_equals(l, b).map(Some);
        }
        if let Xv::List(l) = self.classify(b) {
            return self.list_equals(l, a).map(Some);
        }
        if let (Xv::Node(x), Xv::Node(y)) = (self.classify(a), self.classify(b)) {
            let (xk, yk) = (self.xml_kind(x)?, self.xml_kind(y)?);
            let textual = |k| matches!(k, XmlKind::Text | XmlKind::Attribute);
            if (textual(xk) && self.node_simple(y)?) || (textual(yk) && self.node_simple(x)?) {
                return Ok(Some(self.node_to_str(x)?.flat() == self.node_to_str(y)?.flat()));
            }
            return self.node_equals(x, y).map(Some);
        }
        if let (Value::Object(x), Value::Object(y)) = (a, b) {
            let classes = (self.heap.object(*x).map(|o| &o.class), self.heap.object(*y).map(|o| &o.class));
            match classes {
                (Some(Class::QName { name: p, .. }), Some(Class::QName { name: q, .. })) => return Ok(Some(p.uri == q.uri && p.local == q.local)),
                (Some(Class::Namespace(p)), Some(Class::Namespace(q))) => return Ok(Some(p.uri == q.uri)),
                _ => {}
            }
        }
        for (x, y) in [(a, b), (b, a)] {
            if let Xv::Node(n) = self.classify(x)
                && self.node_simple(n)?
            {
                let s = self.node_to_str(n)?.flat();
                let other = self.to_str(y)?.flat();
                return Ok(Some(s == other));
            }
        }
        Ok(None)
    }

    pub(crate) fn xml_concat(&mut self, a: &Value, b: &Value) -> JsResult<Value> {
        self.rooted(&[a.clone(), b.clone()], |i| {
            let out = i.new_xml_list(None, None)?;
            i.xml_keep(out);
            for v in [a, b] {
                match i.classify(v) {
                    Xv::Node(n) => i.list_push(out, n)?,
                    Xv::List(l) => {
                        for at in 0..i.list_len(l)? {
                            i.use_fuel()?;
                            let Some(item) = i.list_at(l, at)? else { break };
                            i.list_push(out, item)?;
                        }
                    }
                    Xv::Other => {}
                }
            }
            Ok(Value::Object(out))
        })
    }

    // ---- content tests and walks ----

    pub(crate) fn node_simple(&mut self, x: ObjId) -> JsResult<bool> {
        let node = self.xml_node(x)?;
        if matches!(node.kind, XmlKind::Comment | XmlKind::Pi) {
            return Ok(false);
        }
        let len = node.children.len();
        self.xml_text_fuel(len)?;
        Ok(!self.xml_node(x)?.children.iter().any(|&c| self.xml_node(c).is_ok_and(|n| n.kind == XmlKind::Element)))
    }

    pub(crate) fn node_complex(&mut self, x: ObjId) -> JsResult<bool> {
        let node = self.xml_node(x)?;
        if node.kind != XmlKind::Element {
            return Ok(false);
        }
        Ok(!self.node_simple(x)?)
    }

    /// The nodes under `x` matching `n`, in document order ([[Descendants]], section 9.1.1.8).
    pub(crate) fn node_descendants(&mut self, x: ObjId, n: &NameRef, out: ObjId) -> JsResult<()> {
        let mut stack = vec![(x, 0usize)];
        if n.attribute {
            self.collect_matching(x, n, out)?;
        }
        while let Some(&(node, next)) = stack.last() {
            self.use_fuel()?;
            let Some(child) = self.child_at(node, next)? else {
                stack.pop();
                continue;
            };
            if let Some(frame) = stack.last_mut() {
                frame.1 += 1;
            }
            if n.attribute {
                self.collect_matching(child, n, out)?;
            } else if n.matches_child(self.xml_node(child)?) {
                self.list_push(out, child)?;
            }
            stack.push((child, 0));
        }
        Ok(())
    }

    /// Merges adjacent text and removes empty text under `root` (ECMA-357 section 13.4.4.26).
    pub(crate) fn node_normalize(&mut self, root: ObjId) -> JsResult<()> {
        let mut stack = vec![(root, 0usize)];
        while let Some(&(x, mut i)) = stack.last() {
            self.use_fuel()?;
            let Some(child) = self.child_at(x, i)? else {
                stack.pop();
                continue;
            };
            let kind = self.xml_kind(child)?;
            if kind == XmlKind::Text {
                while let Some(next) = self.child_at(x, i + 1)? {
                    if self.xml_kind(next)? != XmlKind::Text {
                        break;
                    }
                    let joined = {
                        let (p, q) = (self.xml_node(child)?.value.clone(), self.xml_node(next)?.value.clone());
                        self.concat(&p, &q)?
                    };
                    self.node_delete_at(x, i + 1)?;
                    self.xml_node_mut(child)?.value = joined;
                }
                if self.xml_node(child)?.value.len() == 0 {
                    self.node_delete_at(x, i)?;
                } else {
                    i += 1;
                }
                if let Some(frame) = stack.last_mut() {
                    frame.1 = i;
                }
                continue;
            }
            if let Some(frame) = stack.last_mut() {
                frame.1 = i + 1;
            }
            if kind == XmlKind::Element {
                stack.push((child, 0));
            }
        }
        Ok(())
    }

    // ---- strings ----

    /// ToString of an XML value (ECMA-357 section 10.1.1).
    pub(crate) fn node_to_str(&mut self, x: ObjId) -> JsResult<Str> {
        if matches!(self.xml_kind(x)?, XmlKind::Text | XmlKind::Attribute) {
            return Ok(self.xml_node(x)?.value.clone());
        }
        if !self.node_simple(x)? {
            return self.xml_string_of(&Value::Object(x), None);
        }
        let mut out = Builder::new();
        for i in 0..self.children_len(x)? {
            self.use_fuel()?;
            let Some(c) = self.child_at(x, i)? else { break };
            let node = self.xml_node(c)?;
            if !matches!(node.kind, XmlKind::Comment | XmlKind::Pi) {
                let text = node.value.flat();
                out.push_units(self, &text)?;
            }
        }
        let Value::String(s) = out.finish(self) else { return Err(Abrupt::Internal("not a string".into())) };
        Ok(s)
    }

    pub(crate) fn list_simple(&mut self, l: ObjId) -> JsResult<bool> {
        let len = self.list_len(l)?;
        if len == 1 {
            return match self.list_at(l, 0)? {
                Some(item) => self.node_simple(item),
                None => Ok(true),
            };
        }
        for i in 0..len {
            self.use_fuel()?;
            if let Some(item) = self.list_at(l, i)?
                && self.xml_kind(item)? == XmlKind::Element
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// ToString of an XMLList value (ECMA-357 section 10.1.2).
    pub(crate) fn list_to_str(&mut self, l: ObjId) -> JsResult<Str> {
        if !self.list_simple(l)? {
            return self.xml_string_of(&Value::Object(l), None);
        }
        let mut out = Builder::new();
        for i in 0..self.list_len(l)? {
            self.use_fuel()?;
            let Some(item) = self.list_at(l, i)? else { break };
            if !matches!(self.xml_kind(item)?, XmlKind::Comment | XmlKind::Pi) {
                let text = self.node_to_str(item)?.flat();
                out.push_units(self, &text)?;
            }
        }
        let Value::String(s) = out.finish(self) else { return Err(Abrupt::Internal("not a string".into())) };
        Ok(s)
    }

    pub(crate) fn xml_to_string_value(&mut self, v: &Value) -> JsResult<Str> {
        match self.classify(v) {
            Xv::Node(n) => self.node_to_str(n),
            Xv::List(l) => self.list_to_str(l),
            Xv::Other => Err(self.not_xml()),
        }
    }

    fn write_escaped(&mut self, out: &mut Builder, text: &[u16], attribute: bool) -> JsResult<()> {
        let mut from = 0;
        for (i, &c) in text.iter().enumerate() {
            let rep: Option<&str> = match c {
                LT => Some("&lt;"),
                AMP => Some("&amp;"),
                GT if !attribute => Some("&gt;"),
                DQUOTE if attribute => Some("&quot;"),
                LF if attribute => Some("&#xA;"),
                CR if attribute => Some("&#xD;"),
                TAB if attribute => Some("&#x9;"),
                _ => None,
            };
            if let Some(rep) = rep {
                out.push_units(self, text.get(from..i).unwrap_or_default())?;
                out.push_str(self, rep)?;
                from = i + 1;
            }
        }
        out.push_units(self, text.get(from..).unwrap_or_default())
    }

    /// Picks the prefix of a name in `scope`, adding a declaration to `scope` when none is there.
    fn out_prefix(&mut self, name: &XmlName, scope: &mut Vec<XmlNs>, attribute: bool) -> JsResult<Key> {
        self.xml_take_fuel(scope.len() + 1)?;
        let empty: Key = Rc::from(Vec::new());
        let uri = name.uri.clone().unwrap_or_else(|| empty.clone());
        if uri.is_empty() {
            if !attribute && prefix_in_scope(scope, &empty).is_some_and(|ns| !ns.uri.is_empty()) {
                self.xml_charge(XML_NS_BYTES)?;
                scope.push(XmlNs { prefix: Some(empty.clone()), uri });
            }
            return Ok(empty);
        }
        if *uri == *key(XML_URI) {
            return Ok(key("xml"));
        }
        if let Some(ns) = ns_lookup(scope, &uri, name.prefix.as_ref(), attribute) {
            return Ok(ns.prefix.unwrap_or(empty));
        }
        let prefix = match &name.prefix {
            Some(p) if !(attribute && p.is_empty()) && prefix_in_scope(scope, p).is_none() => p.clone(),
            _ if !attribute && prefix_in_scope(scope, &empty).is_none() => empty,
            _ => fresh_prefix(scope),
        };
        self.xml_charge(XML_NS_BYTES)?;
        scope.push(XmlNs { prefix: Some(prefix.clone()), uri });
        Ok(prefix)
    }

    fn write_name(&mut self, out: &mut Builder, prefix: &[u16], local: &[u16]) -> JsResult<()> {
        if !prefix.is_empty() {
            out.push_units(self, prefix)?;
            out.push_units(self, &[COLON])?;
        }
        out.push_units(self, local)
    }

    /// Writes `x`; an element with children is left open on `stack`.
    #[allow(clippy::too_many_arguments)]
    fn write_enter(
        &mut self,
        out: &mut Builder,
        x: ObjId,
        indent: u64,
        settings: &Settings,
        pretty: bool,
        scope: &mut Vec<XmlNs>,
        stack: &mut Vec<OutFrame>,
    ) -> JsResult<()> {
        self.use_fuel()?;
        let node = self.xml_node(x)?;
        let (kind, value) = (node.kind, node.value.flat());
        self.xml_text_fuel(value.len())?;
        if pretty {
            out.push_repeated(self, &[SPACE], indent)?;
        }
        match kind {
            XmlKind::Text => {
                let trimmed: &[u16] = if pretty { trim_xml_space(&value) } else { &value };
                return self.write_escaped(out, trimmed, false);
            }
            XmlKind::Attribute => return self.write_escaped(out, &value, true),
            XmlKind::Comment => {
                out.push_str(self, "<!--")?;
                out.push_units(self, &value)?;
                return out.push_str(self, "-->");
            }
            XmlKind::Pi => {
                let target = self.xml_node(x)?.name.as_ref().map(|n| n.local.clone()).unwrap_or_else(|| Rc::from(Vec::new()));
                out.push_str(self, "<?")?;
                out.push_units(self, &target)?;
                out.push_units(self, &[SPACE])?;
                out.push_units(self, &value)?;
                return out.push_str(self, "?>");
            }
            XmlKind::Element => {}
        }
        let ns_mark = scope.len();
        let node = self.xml_node(x)?;
        let declared = node.ns.clone();
        let attrs = node.attrs.clone();
        let name = node.name.clone().ok_or_else(|| Abrupt::Internal("element without a name".into()))?;
        self.xml_take_fuel(declared.len().saturating_mul(scope.len() + 1))?;
        let mut unnamed = Vec::new();
        for ns in declared {
            if ns.prefix.is_none() {
                unnamed.push(ns);
            } else if !scope.iter().any(|a| a.uri == ns.uri && a.prefix == ns.prefix) {
                self.xml_charge(XML_NS_BYTES)?;
                scope.push(ns);
            }
        }
        for ns in unnamed {
            if !scope.iter().any(|a| a.uri == ns.uri) {
                let prefix = fresh_prefix(scope);
                self.xml_charge(XML_NS_BYTES)?;
                scope.push(XmlNs { prefix: Some(prefix), uri: ns.uri });
            }
        }
        let prefix = self.out_prefix(&name, scope, false)?;
        let mut attr_prefixes = Vec::new();
        for &a in &attrs {
            self.use_fuel()?;
            let an = self.xml_node(a)?.name.clone().ok_or_else(|| Abrupt::Internal("attribute without a name".into()))?;
            attr_prefixes.push(self.out_prefix(&an, scope, true)?);
        }
        out.push_units(self, &[LT])?;
        self.write_name(out, &prefix, &name.local)?;
        for (&a, attr_prefix) in attrs.iter().zip(&attr_prefixes) {
            let an = self.xml_node(a)?;
            let local = an.name.as_ref().map(|q| q.local.clone()).unwrap_or_else(|| Rc::from(Vec::new()));
            let value = an.value.flat();
            out.push_units(self, &[SPACE])?;
            self.write_name(out, attr_prefix, &local)?;
            out.push_units(self, &[EQ, DQUOTE])?;
            self.write_escaped(out, &value, true)?;
            out.push_units(self, &[DQUOTE])?;
        }
        for at in ns_mark..scope.len() {
            let Some(ns) = scope.get(at).cloned() else { break };
            let ns_prefix = ns.prefix.unwrap_or_else(|| Rc::from(Vec::new()));
            out.push_str(self, " xmlns")?;
            if !ns_prefix.is_empty() {
                out.push_units(self, &[COLON])?;
                out.push_units(self, &ns_prefix)?;
            }
            out.push_units(self, &[EQ, DQUOTE])?;
            self.write_escaped(out, &ns.uri, true)?;
            out.push_units(self, &[DQUOTE])?;
        }
        let count = self.children_len(x)?;
        if count == 0 {
            scope.truncate(ns_mark);
            return out.push_str(self, "/>");
        }
        out.push_units(self, &[GT])?;
        let first_is_text = match self.child_at(x, 0)? {
            Some(c) => self.xml_kind(c)? == XmlKind::Text,
            None => false,
        };
        let pretty_children = pretty && (count > 1 || !first_is_text);
        let child_indent = if pretty_children { indent.saturating_add(u64::from(settings.indent)) } else { 0 };
        stack.push(OutFrame { node: x, next: 0, indent, child_indent, pretty_children, ns_mark, prefix, local: name.local });
        Ok(())
    }

    fn write_node(&mut self, out: &mut Builder, root: ObjId, indent: u64, settings: &Settings, pretty: bool) -> JsResult<()> {
        let mut scope: Vec<XmlNs> = Vec::new();
        let mut stack: Vec<OutFrame> = Vec::new();
        self.write_enter(out, root, indent, settings, pretty, &mut scope, &mut stack)?;
        while let Some(frame) = stack.last_mut() {
            let (node, next, child_indent, pretty_children) = (frame.node, frame.next, frame.child_indent, frame.pretty_children);
            frame.next += 1;
            match self.child_at(node, next)? {
                Some(child) => {
                    if pretty_children {
                        out.push_units(self, &[LF])?;
                    }
                    self.write_enter(out, child, child_indent, settings, pretty && pretty_children, &mut scope, &mut stack)?;
                }
                None => {
                    let Some(frame) = stack.pop() else { break };
                    if frame.pretty_children {
                        out.push_units(self, &[LF])?;
                        out.push_repeated(self, &[SPACE], frame.indent)?;
                    }
                    out.push_units(self, &[LT, SLASH])?;
                    self.write_name(out, &frame.prefix, &frame.local)?;
                    out.push_units(self, &[GT])?;
                    scope.truncate(frame.ns_mark);
                }
            }
        }
        Ok(())
    }

    /// ToXMLString (ECMA-357 section 10.2). `pretty` overrides the setting when it is given.
    pub(crate) fn xml_string_of(&mut self, v: &Value, pretty: Option<bool>) -> JsResult<Str> {
        let settings = self.xml_settings()?;
        let pretty = pretty.unwrap_or(settings.pretty);
        let mut out = Builder::new();
        match self.classify(v) {
            Xv::Node(n) => self.write_node(&mut out, n, 0, &settings, pretty)?,
            Xv::List(l) => {
                for i in 0..self.list_len(l)? {
                    self.use_fuel()?;
                    let Some(item) = self.list_at(l, i)? else { break };
                    if i > 0 && pretty {
                        out.push_units(self, &[LF])?;
                    }
                    self.write_node(&mut out, item, 0, &settings, pretty)?;
                }
            }
            Xv::Other => return Err(self.not_xml()),
        }
        let Value::String(s) = out.finish(self) else { return Err(Abrupt::Internal("not a string".into())) };
        Ok(s)
    }

    // ---- the parser ----

    fn syntax(&mut self, message: &str) -> Abrupt {
        self.throw(ErrorType::Syntax, message)
    }

    fn decode_into(&mut self, src: &[u16], from: usize, to: usize, out: &mut Vec<u16>, attribute: bool) -> JsResult<()> {
        self.xml_text_fuel(to.saturating_sub(from))?;
        let mut pos = from;
        while pos < to {
            let Some(&c) = src.get(pos) else { break };
            if c == AMP {
                let Some((after, ch)) = scan_reference(src, pos).filter(|(after, _)| *after <= to) else {
                    return Err(self.syntax("invalid XML character or entity reference"));
                };
                push_char(out, ch);
                pos = after;
                continue;
            }
            if pos + 1 < to && starts_pair(c, src.get(pos + 1).copied()) {
                out.extend_from_slice(src.get(pos..pos + 2).unwrap_or_default());
                pos += 2;
                continue;
            }
            if !is_xml_char(u32::from(c)) {
                return Err(self.syntax("invalid character in XML text"));
            }
            pos += 1;
            match (c, attribute) {
                (CR, _) => {
                    if src.get(pos) == Some(&LF) {
                        pos += 1;
                    }
                    out.push(if attribute { SPACE } else { LF });
                }
                (TAB | LF, true) => out.push(SPACE),
                _ => out.push(c),
            }
        }
        Ok(())
    }

    fn find_from(src: &[u16], from: usize, needle: &[u16]) -> Option<usize> {
        src.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| from + i)
    }

    fn read_name(&mut self, src: &[u16], pos: usize) -> JsResult<usize> {
        scan_name(src, pos).ok_or_else(|| self.syntax("invalid XML name"))
    }

    fn check_units(&mut self, src: &[u16], from: usize, to: usize) -> JsResult<()> {
        if valid_xml_units(src.get(from..to).unwrap_or_default()) {
            return Ok(());
        }
        Err(self.syntax("invalid character in XML text"))
    }

    /// Splits `prefix:local`.
    fn split_name<'a>(&mut self, raw: &'a [u16]) -> JsResult<(Option<&'a [u16]>, &'a [u16])> {
        let Some(colon) = raw.iter().position(|&c| c == COLON) else { return Ok((None, raw)) };
        let (prefix, rest) = raw.split_at(colon);
        let local = rest.get(1..).unwrap_or_default();
        if !is_ncname(prefix) || !is_ncname(local) {
            return Err(self.syntax("invalid XML qualified name"));
        }
        Ok((Some(prefix), local))
    }

    fn resolve_prefix(&mut self, prefix: &[u16], own: &[XmlNs], open: &[ObjId]) -> JsResult<Option<Key>> {
        if prefix == key("xml").as_ref() {
            return Ok(Some(key(XML_URI)));
        }
        self.xml_take_fuel(own.len() + open.len() + 1)?;
        if let Some(ns) = prefix_in_scope(own, prefix) {
            return Ok(Some(ns.uri.clone()));
        }
        for &e in open.iter().rev() {
            let node = self.xml_node(e)?;
            let len = node.ns.len();
            if let Some(ns) = prefix_in_scope(&node.ns, prefix) {
                return Ok(Some(ns.uri.clone()));
            }
            self.xml_take_fuel(len / UNITS_PER_FUEL)?;
        }
        Ok(None)
    }

    fn place_parsed(&mut self, node: ObjId, open: &[ObjId], roots: &mut Vec<ObjId>) -> JsResult<()> {
        if let Some(&top) = open.last() {
            self.xml_charge_link(node)?;
            self.xml_node_mut(top)?.children.push(node);
            self.xml_node_mut(node)?.parent = Some(top);
        } else {
            self.xml_keep(node);
            self.xml_charge(XML_REF_BYTES)?;
            roots.push(node);
        }
        Ok(())
    }

    fn flush_text(&mut self, text: &mut Vec<u16>, ignore_whitespace: bool, open: &[ObjId], roots: &mut Vec<ObjId>) -> JsResult<()> {
        if text.is_empty() {
            return Ok(());
        }
        if ignore_whitespace && text.iter().all(|&c| is_xml_space(c)) {
            text.clear();
            return Ok(());
        }
        let t = self.new_text_node(Str::from_units(std::mem::take(text)), None)?;
        self.place_parsed(t, open, roots)
    }

    fn parse_open_tag(
        &mut self,
        src: &[u16],
        pos: usize,
        open: &mut Vec<ObjId>,
        names: &mut Vec<(usize, usize)>,
        roots: &mut Vec<ObjId>,
    ) -> JsResult<usize> {
        if open.len() >= self.options.max_depth {
            return Err(self.syntax(TOO_DEEP));
        }
        let name_start = pos + 1;
        let name_end = self.read_name(src, name_start)?;
        let raw_name = src.get(name_start..name_end).unwrap_or_default();
        let (elem_prefix, elem_local) = self.split_name(raw_name)?;
        let mut at = name_end;
        let mut raw_attrs: Vec<(usize, usize, Vec<u16>)> = Vec::new();
        let closes = loop {
            let before = at;
            while src.get(at).copied().is_some_and(is_xml_space) {
                at += 1;
            }
            match src.get(at).copied() {
                None => return Err(self.syntax("unterminated XML tag")),
                Some(SLASH) if src.get(at + 1) == Some(&GT) => {
                    at += 2;
                    break true;
                }
                Some(GT) => {
                    at += 1;
                    break false;
                }
                Some(_) if at == before => return Err(self.syntax("expected white space before an attribute")),
                Some(_) => {
                    self.use_fuel()?;
                    let end = self.read_name(src, at)?;
                    let mut p = end;
                    while src.get(p).copied().is_some_and(is_xml_space) {
                        p += 1;
                    }
                    if src.get(p) != Some(&EQ) {
                        return Err(self.syntax("expected '=' after an attribute name"));
                    }
                    p += 1;
                    while src.get(p).copied().is_some_and(is_xml_space) {
                        p += 1;
                    }
                    let quote = match src.get(p).copied() {
                        Some(q @ (DQUOTE | SQUOTE)) => q,
                        _ => return Err(self.syntax("expected a quoted attribute value")),
                    };
                    let value_start = p + 1;
                    let Some(value_end) = Self::find_from(src, value_start, &[quote]) else {
                        return Err(self.syntax("unterminated attribute value"));
                    };
                    if src.get(value_start..value_end).is_some_and(|v| v.contains(&LT)) {
                        return Err(self.syntax("'<' cannot appear in an attribute value"));
                    }
                    let mut value = Vec::new();
                    self.decode_into(src, value_start, value_end, &mut value, true)?;
                    raw_attrs.push((at, end, value));
                    at = value_end + 1;
                }
            }
        };
        self.xml_charge(XML_NS_BYTES.saturating_mul(raw_attrs.len()))?;
        let mut ns: Vec<XmlNs> = Vec::new();
        let mut seen_decl: HashSet<Vec<u16>> = HashSet::new();
        for (start, end, value) in &raw_attrs {
            let raw = src.get(*start..*end).unwrap_or_default();
            if raw == key("xmlns").as_ref() {
                if !ns_binding_allowed(&[], value) {
                    return Err(self.syntax(RESERVED_NS_BINDING));
                }
                if !seen_decl.insert(Vec::new()) {
                    return Err(self.syntax("a namespace is declared twice"));
                }
                ns.push(XmlNs { prefix: Some(Rc::from(Vec::new())), uri: Rc::from(value.as_slice()) });
            } else if let (Some(p), local) = self.split_name(raw)?
                && p == key("xmlns").as_ref()
            {
                if value.is_empty() {
                    return Err(self.syntax("a namespace prefix cannot be declared with an empty URI"));
                }
                if !ns_binding_allowed(local, value) {
                    return Err(self.syntax(RESERVED_NS_BINDING));
                }
                if !seen_decl.insert(local.to_vec()) {
                    return Err(self.syntax("a namespace is declared twice"));
                }
                ns.push(XmlNs { prefix: Some(Rc::from(local)), uri: Rc::from(value.as_slice()) });
            }
        }
        let uri = match elem_prefix {
            Some(p) => match self.resolve_prefix(p, &ns, open)? {
                Some(u) => u,
                None => return Err(self.syntax("an XML name uses an undeclared namespace prefix")),
            },
            None => match self.resolve_prefix(&[], &ns, open)? {
                Some(u) => u,
                None => self.xml_default_uri()?,
            },
        };
        let name = XmlName { uri: Some(uri), local: Rc::from(elem_local), prefix: Some(Rc::from(elem_prefix.unwrap_or_default())) };
        let element = self.new_xml_node(XmlNode {
            kind: XmlKind::Element,
            name: Some(name),
            parent: None,
            value: Str::from(""),
            attrs: Vec::new(),
            children: Vec::new(),
            ns,
        })?;
        self.place_parsed(element, open, roots)?;
        let mut unique: HashSet<(Key, Vec<u16>)> = HashSet::new();
        for (start, end, value) in raw_attrs {
            self.use_fuel()?;
            let raw = src.get(start..end).unwrap_or_default();
            let (prefix, local) = self.split_name(raw)?;
            if raw == key("xmlns").as_ref() || prefix.is_some_and(|p| p == key("xmlns").as_ref()) {
                continue;
            }
            let (auri, stored_prefix): (Key, Key) = match prefix {
                Some(p) => {
                    let own = self.xml_node(element)?.ns.clone();
                    match self.resolve_prefix(p, &own, open)? {
                        Some(u) => (u, Rc::from(p)),
                        None => return Err(self.syntax("an XML name uses an undeclared namespace prefix")),
                    }
                }
                None => (Rc::from(Vec::new()), Rc::from(Vec::new())),
            };
            if !unique.insert((auri.clone(), local.to_vec())) {
                return Err(self.type_error("an XML element has two attributes with the same name"));
            }
            let a = self.new_xml_node(XmlNode {
                kind: XmlKind::Attribute,
                name: Some(XmlName { uri: Some(auri), local: Rc::from(local), prefix: Some(stored_prefix) }),
                parent: Some(element),
                value: Str::from_units(value),
                attrs: Vec::new(),
                children: Vec::new(),
                ns: Vec::new(),
            })?;
            self.xml_charge_link(a)?;
            self.xml_node_mut(element)?.attrs.push(a);
        }
        if !closes {
            open.push(element);
            names.push((name_start, name_end));
        }
        Ok(at)
    }

    fn parse_close_tag(&mut self, src: &[u16], pos: usize, open: &mut Vec<ObjId>, names: &mut Vec<(usize, usize)>) -> JsResult<usize> {
        let name_start = pos + 2;
        let name_end = self.read_name(src, name_start)?;
        let mut at = name_end;
        while src.get(at).copied().is_some_and(is_xml_space) {
            at += 1;
        }
        if src.get(at) != Some(&GT) {
            return Err(self.syntax("expected '>' at the end of a closing tag"));
        }
        let Some((open_start, open_end)) = names.pop() else {
            return Err(self.syntax("a closing tag has no matching open tag"));
        };
        if src.get(open_start..open_end) != src.get(name_start..name_end) {
            return Err(self.syntax("the closing tag does not match the open tag"));
        }
        open.pop();
        Ok(at + 1)
    }

    /// Parses XML text into nodes, kept reachable through the temporary roots until the caller's
    /// scope ends.
    pub(crate) fn xml_parse(&mut self, source: &[u16]) -> JsResult<Vec<ObjId>> {
        let mut held = HeldBytes::new(self);
        held.add(self, string_bytes(source.len()).saturating_mul(2))?;
        let settings = self.xml_settings()?;
        let mut src = source;
        let mut offset_start = 0;
        while src.get(offset_start).copied().is_some_and(is_xml_space) {
            offset_start += 1;
        }
        if src.get(offset_start..).is_some_and(|r| r.starts_with(&parser_units("<?xml")) && r.get(5).copied().is_some_and(is_xml_space)) {
            let Some(end) = Self::find_from(src, offset_start, &parser_units("?>")) else {
                return Err(self.syntax("unterminated XML declaration"));
            };
            src = src.get(end + 2..).unwrap_or_default();
        }
        let mut pos = 0;
        let mut roots = Vec::new();
        let mut open: Vec<ObjId> = Vec::new();
        let mut names: Vec<(usize, usize)> = Vec::new();
        let mut text: Vec<u16> = Vec::new();
        while let Some(&c) = src.get(pos) {
            self.use_fuel()?;
            if c != LT {
                let end = Self::find_from(src, pos, &[LT]).unwrap_or(src.len());
                let section_end = parser_units("]]>");
                if src.get(pos..end).is_some_and(|run| run.windows(3).any(|w| w == section_end)) {
                    return Err(self.syntax("']]>' cannot appear in XML text"));
                }
                self.decode_into(src, pos, end, &mut text, false)?;
                pos = end;
                continue;
            }
            let rest = src.get(pos..).unwrap_or_default();
            if rest.starts_with(&parser_units("<![CDATA[")) {
                let Some(end) = Self::find_from(src, pos + 9, &parser_units("]]>")) else {
                    return Err(self.syntax("unterminated CDATA section"));
                };
                self.xml_text_fuel(end - pos)?;
                self.check_units(src, pos + 9, end)?;
                text.extend_from_slice(src.get(pos + 9..end).unwrap_or_default());
                pos = end + 3;
                continue;
            }
            self.flush_text(&mut text, settings.ignore_whitespace, &open, &mut roots)?;
            if rest.starts_with(&parser_units("<!--")) {
                let Some(end) = Self::find_from(src, pos + 4, &parser_units("--")) else {
                    return Err(self.syntax("unterminated XML comment"));
                };
                if src.get(end + 2) != Some(&GT) {
                    return Err(self.syntax("'--' cannot appear in an XML comment"));
                }
                self.xml_text_fuel(end - pos)?;
                self.check_units(src, pos + 4, end)?;
                if !settings.ignore_comments {
                    let value = Str::from_units(src.get(pos + 4..end).unwrap_or_default().to_vec());
                    let n = self.new_xml_node(XmlNode {
                        kind: XmlKind::Comment,
                        name: None,
                        parent: None,
                        value,
                        attrs: Vec::new(),
                        children: Vec::new(),
                        ns: Vec::new(),
                    })?;
                    self.place_parsed(n, &open, &mut roots)?;
                }
                pos = end + 3;
            } else if rest.starts_with(&parser_units("<!")) {
                return Err(self.syntax("document type declarations are not supported in XML values"));
            } else if rest.get(1) == Some(&QUESTION) {
                let name_end = self.read_name(src, pos + 2)?;
                let target = src.get(pos + 2..name_end).unwrap_or_default();
                if target.len() == 3 && String::from_utf16_lossy(target).eq_ignore_ascii_case("xml") {
                    return Err(self.syntax("'xml' cannot name a processing instruction"));
                }
                let Some(end) = Self::find_from(src, name_end, &parser_units("?>")) else {
                    return Err(self.syntax("unterminated processing instruction"));
                };
                self.xml_text_fuel(end - pos)?;
                self.check_units(src, name_end, end)?;
                if !settings.ignore_pis {
                    let mut from = name_end;
                    while from < end && src.get(from).copied().is_some_and(is_xml_space) {
                        from += 1;
                    }
                    let value = Str::from_units(src.get(from..end).unwrap_or_default().to_vec());
                    let name = XmlName { uri: Some(Rc::from(Vec::new())), local: Rc::from(target), prefix: None };
                    let n = self.new_xml_node(XmlNode {
                        kind: XmlKind::Pi,
                        name: Some(name),
                        parent: None,
                        value,
                        attrs: Vec::new(),
                        children: Vec::new(),
                        ns: Vec::new(),
                    })?;
                    self.place_parsed(n, &open, &mut roots)?;
                }
                pos = end + 2;
            } else if rest.get(1) == Some(&SLASH) {
                pos = self.parse_close_tag(src, pos, &mut open, &mut names)?;
            } else {
                pos = self.parse_open_tag(src, pos, &mut open, &mut names, &mut roots)?;
            }
        }
        if !open.is_empty() {
            return Err(self.syntax("an XML element is not closed"));
        }
        self.flush_text(&mut text, settings.ignore_whitespace, &open, &mut roots)?;
        Ok(roots)
    }

    /// One XML value from text (ECMA-357 section 10.3.1).
    pub(crate) fn xml_from_text(&mut self, text: &[u16]) -> JsResult<ObjId> {
        let nodes = self.xml_parse(text)?;
        match nodes.as_slice() {
            [] => {
                let t = self.new_text_node(Str::from(""), None)?;
                self.xml_keep(t);
                Ok(t)
            }
            [one] => Ok(*one),
            _ => Err(self.syntax("an XML value has more than one root")),
        }
    }

    // ---- bytecode operations ----

    pub(crate) fn op_xml_make_name(&mut self, attribute: bool, qualified: bool) -> JsResult<()> {
        let name_value = self.peek(0)?;
        let qualifier = if qualified { Some(self.peek(1)?) } else { None };
        let (mut uri, local): (Option<Key>, Key) = match &name_value {
            Value::Object(o) if matches!(self.heap.object(*o).map(|ob| &ob.class), Some(Class::QName { .. })) => {
                let Some(Class::QName { name, .. }) = self.heap.object(*o).map(|ob| &ob.class) else {
                    return Err(Abrupt::Internal("not a QName".into()));
                };
                (name.uri.clone(), name.local.clone())
            }
            v => {
                let local = self.to_str(v)?.flat();
                let uri = if *local == *STAR {
                    None
                } else if attribute {
                    Some(Rc::from(Vec::new()))
                } else {
                    Some(self.xml_default_uri()?)
                };
                (uri, local)
            }
        };
        if let Some(q) = qualifier {
            uri = match &q {
                Value::Null => None,
                Value::Undefined => return Err(self.type_error("a namespace qualifier is undefined")),
                Value::Object(o) => match self.heap.object(*o).map(|ob| &ob.class) {
                    Some(Class::Namespace(ns)) => Some(ns.uri.clone()),
                    Some(Class::QName { name, .. }) => name.uri.clone(),
                    _ => Some(self.to_str(&q)?.flat()),
                },
                _ => Some(self.to_str(&q)?.flat()),
            };
        }
        let id = self.new_reference_name(XmlName { uri, local, prefix: None }, attribute)?;
        self.replace(1 + usize::from(qualified), Value::Object(id))
    }

    pub(crate) fn op_xml_descendants(&mut self) -> JsResult<()> {
        let name = self.peek(0)?;
        let base = self.peek(1)?;
        let n = self.xml_name_only(&name)?;
        let v = self.xml_descendants_of(&base, &n)?;
        self.replace(2, v)
    }

    /// The descendants of a value that match `n`.
    pub(crate) fn xml_descendants_of(&mut self, base: &Value, n: &NameRef) -> JsResult<Value> {
        let xv = self.classify(base);
        if matches!(xv, Xv::Other) {
            return Err(self.not_xml());
        }
        self.rooted(std::slice::from_ref(base), |i| {
            let out = i.new_xml_list(None, None)?;
            i.xml_keep(out);
            match xv {
                Xv::Node(x) => i.node_descendants(x, n, out)?,
                Xv::List(l) => {
                    for at in 0..i.list_len(l)? {
                        i.use_fuel()?;
                        let Some(item) = i.list_at(l, at)? else { break };
                        i.node_descendants(item, n, out)?;
                    }
                }
                Xv::Other => {}
            }
            Ok(Value::Object(out))
        })
    }

    pub(crate) fn op_xml_escape(&mut self, kind: u8) -> JsResult<()> {
        let v = self.peek(0)?;
        let result = match kind {
            0 if self.is_xml(&v) => self.xml_string_of(&v, Some(false))?,
            0 | 1 => {
                let text = self.to_str(&v)?.flat();
                let mut out = Builder::new();
                self.write_escaped(&mut out, &text, kind == 1)?;
                let Value::String(s) = out.finish(self) else { return Err(Abrupt::Internal("not a string".into())) };
                s
            }
            _ => {
                let text = self.to_str(&v)?;
                let units = flat_charged(self, &text)?;
                self.xml_take_fuel(units.len() / UNITS_PER_FUEL + 1)?;
                if kind == 2 && scan_name(&units, 0) != Some(units.len()) {
                    return Err(self.syntax("an embedded expression used as an XML name does not give a name"));
                }
                if kind != 2 && !is_attribute_run(&units) {
                    return Err(self.syntax("an embedded expression used as XML attributes does not give only attributes"));
                }
                text
            }
        };
        self.replace(1, Value::String(result))
    }

    pub(crate) fn op_xml_literal(&mut self, parts: u32, list: bool) -> JsResult<()> {
        let n = parts as usize;
        let mut out = Builder::new();
        for at in (0..n).rev() {
            let Value::String(s) = self.peek(at)? else { return Err(Abrupt::Internal("an XML literal part is not a string".into())) };
            out.push_units(self, &s.flat())?;
        }
        let Value::String(text) = out.finish(self) else { return Err(Abrupt::Internal("not a string".into())) };
        let text = text.flat();
        let value = self.rooted(&[], |i| {
            if list {
                let nodes = i.xml_parse(&text)?;
                let out = i.new_xml_list(None, None)?;
                i.xml_keep(out);
                for node in nodes {
                    i.list_push(out, node)?;
                }
                Ok(Value::Object(out))
            } else {
                Ok(Value::Object(i.xml_from_text(&text)?))
            }
        })?;
        self.replace(n, value)
    }

    /// The nearest XML or XMLList value in scope whose [[HasProperty]] accepts `n`, or a
    /// ReferenceError when none does (ECMA-357 section 11.1).
    fn xml_scope_holder(&mut self, n: &NameRef) -> JsResult<ObjId> {
        let mut cur = Some(self.frame()?.env);
        let mut any_xml = false;
        while let Some(e) = cur {
            match self.heap.env(e) {
                Some(Env::Decl { outer, .. }) => cur = *outer,
                Some(&Env::Obj { object, outer, .. }) => {
                    let v = Value::Object(object);
                    if self.is_xml(&v) {
                        any_xml = true;
                        let found = match self.classify(&v) {
                            Xv::Node(x) => self.node_has_name(x, n)?,
                            _ => self.list_has_name(object, n)?,
                        };
                        if found {
                            return Ok(object);
                        }
                    }
                    cur = outer;
                }
                None => return Err(Abrupt::Internal("environment out of range".into())),
            }
        }
        let message = if any_xml { "no XML value in scope has the XML name" } else { "an XML name is used outside any XML value" };
        Err(self.throw(ErrorType::Reference, message))
    }

    pub(crate) fn op_xml_scope_get(&mut self) -> JsResult<()> {
        let name = self.peek(0)?;
        let n = self.xml_name_only(&name)?;
        let object = self.xml_scope_holder(&n)?;
        let v = self.xml_select(object, &n)?;
        self.replace(1, Value::Object(v))
    }

    pub(crate) fn op_xml_scope_ref(&mut self) -> JsResult<()> {
        let name = self.peek(0)?;
        let n = self.xml_name_only(&name)?;
        let object = self.xml_scope_holder(&n)?;
        self.replace(1, Value::Object(object))?;
        self.push(name)
    }

    pub(crate) fn op_xml_filter_start(&mut self) -> JsResult<()> {
        let base = self.peek(0)?;
        let input = match self.classify(&base) {
            Xv::Node(x) => vec![x],
            Xv::List(l) => self.xml_list(l)?.items.clone(),
            Xv::Other => return Err(self.not_xml()),
        };
        let bytes = XML_LIST_BYTES.saturating_add(XML_REF_BYTES.saturating_mul(input.len()));
        self.reserve_heap(OBJECT_BYTES + bytes)?;
        self.heap.bytes = self.heap.bytes.saturating_add(bytes);
        let state = self.heap.alloc_object(Object::new(Class::XmlFilter(Box::new(XmlFilterState { input, pos: 0, kept: Vec::new() })), None));
        self.replace(1, Value::Object(state))
    }

    fn filter_state(&mut self, id: ObjId) -> JsResult<&mut XmlFilterState> {
        match self.heap.object_mut(id).map(|o| &mut o.class) {
            Some(Class::XmlFilter(s)) => Ok(s),
            _ => Err(Abrupt::Internal("not an XML filter".into())),
        }
    }

    pub(crate) fn op_xml_filter_next(&mut self, target: u32) -> JsResult<()> {
        let Value::Object(id) = self.peek(0)? else { return Err(Abrupt::Internal("not an XML filter".into())) };
        let state = self.filter_state(id)?;
        let Some(&item) = state.input.get(state.pos) else { return self.jump_to(target) };
        state.pos += 1;
        self.push(Value::Object(item))
    }

    pub(crate) fn op_xml_filter_test(&mut self) -> JsResult<()> {
        let value = self.peek(0)?;
        let Value::Object(id) = self.peek(1)? else { return Err(Abrupt::Internal("not an XML filter".into())) };
        if value.truthy() {
            self.xml_charge(XML_REF_BYTES)?;
            let state = self.filter_state(id)?;
            if let Some(&item) = state.pos.checked_sub(1).and_then(|p| state.input.get(p)) {
                state.kept.push(item);
            }
        }
        self.pop()?;
        Ok(())
    }

    pub(crate) fn op_xml_filter_end(&mut self) -> JsResult<()> {
        let Value::Object(id) = self.peek(0)? else { return Err(Abrupt::Internal("not an XML filter".into())) };
        let out = self.new_xml_list(None, None)?;
        let kept = std::mem::take(&mut self.filter_state(id)?.kept);
        let bytes = XML_REF_BYTES.saturating_mul(kept.len());
        self.xml_list_mut(out)?.items = kept;
        self.heap.bytes = self.heap.bytes.saturating_add(bytes);
        self.replace(1, Value::Object(out))
    }

    pub(crate) fn op_for_each_value(&mut self) -> JsResult<()> {
        let name = self.peek(0)?;
        let Value::Object(iter) = self.peek(1)? else { return Err(Abrupt::Internal("for-in iterator is not an object".into())) };
        let object = match self.heap.object(iter).map(|o| &o.class) {
            Some(Class::ForIn(state)) => state.object,
            _ => return Err(Abrupt::Internal("not a for-in iterator".into())),
        };
        let v = match object {
            Some(o) => {
                let k = self.to_key(&name)?;
                self.get_value(&Value::Object(o), &k)?
            }
            None => Value::Undefined,
        };
        self.replace(1, v)
    }

    /// The URI of a Namespace value or of its string form.
    pub(crate) fn namespace_uri_of(&mut self, v: &Value) -> JsResult<Key> {
        if let Value::Object(o) = v {
            match self.heap.object(*o).map(|ob| &ob.class) {
                Some(Class::Namespace(ns)) => return Ok(ns.uri.clone()),
                Some(Class::QName { name, .. }) => {
                    if let Some(u) = &name.uri {
                        return Ok(u.clone());
                    }
                }
                _ => {}
            }
        }
        Ok(self.to_str(v)?.flat())
    }

    pub(crate) fn op_set_default_namespace(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let uri = self.namespace_uri_of(&v)?;
        self.xml_set_default_uri(uri)?;
        self.pop()?;
        Ok(())
    }
}

/// `s` without leading and trailing XML white space.
fn trim_xml_space(s: &[u16]) -> &[u16] {
    let start = s.iter().position(|&c| !is_xml_space(c)).unwrap_or(s.len());
    let end = s.iter().rposition(|&c| !is_xml_space(c)).map_or(start, |i| i + 1);
    s.get(start..end).unwrap_or_default()
}

/// The array index a string spells canonically.
pub(crate) fn canonical_index(s: &[u16]) -> Option<u32> {
    if s.is_empty() || s.len() > 10 || (s.len() > 1 && s.first() == Some(&0x30)) {
        return None;
    }
    let mut n: u64 = 0;
    for &c in s {
        if !(0x30..=0x39).contains(&c) {
            return None;
        }
        n = n * 10 + u64::from(c - 0x30);
    }
    u32::try_from(n).ok().filter(|&v| v != u32::MAX)
}
