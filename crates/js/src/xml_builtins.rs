//! The E4X built-ins (ECMA-357 sections 13.1 to 13.5): `XML`, `XMLList`, `Namespace`, `QName`,
//! `isXMLName` and `setDefaultXMLNamespace`. A realm has them only with [`crate::Options::e4x`].

use std::rc::Rc;

use crate::builtins::{arg, constructor, data, method, native};
use crate::heap::{
    Class, ENUMERABLE, Key, NativeFn, NativeKind, OBJECT_BYTES, ObjId, Object, PROP_BYTES, Prop, WRITABLE, XmlKind, XmlName, XmlNs, key, string_bytes,
};
use crate::interp::{Abrupt, Interp, JsResult};
use crate::string_builtins::{new_array, set_element};
use crate::value::{Str, Value};
use crate::xml::{
    Access, Content, NameRef, RESERVED_NS_BINDING, STAR, Xv, canonical_index, is_ncname, is_script_xml_name, ns_binding_allowed, xml_name_writable,
};

const SETTINGS: [(&str, Value); 5] = [
    ("ignoreComments", Value::Bool(false)),
    ("ignoreProcessingInstructions", Value::Bool(false)),
    ("ignoreWhitespace", Value::Bool(true)),
    ("prettyPrinting", Value::Bool(true)),
    ("prettyIndent", Value::Number(2.0)),
];

pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let object_proto = interp.intrinsics.object_proto;
    let alloc = |i: &mut Interp| i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));

    let xml_proto = alloc(interp);
    let list_proto = alloc(interp);
    let namespace_proto = alloc(interp);
    let qname_proto = alloc(interp);
    interp.intrinsics.xml_proto = xml_proto;
    interp.intrinsics.xml_list_proto = list_proto;
    interp.intrinsics.namespace_proto = namespace_proto;
    interp.intrinsics.qname_proto = qname_proto;

    let xml = native(interp, "XML", xml_ctor, 1, NativeKind::Plain, true)?;
    interp.intrinsics.xml_ctor = xml;
    constructor(interp, "XML", xml, xml_proto)?;
    for (name, value) in SETTINGS {
        data(interp, xml, name, value, WRITABLE)?;
    }
    method(interp, xml, "settings", settings, 0)?;
    method(interp, xml, "setSettings", set_settings, 1)?;
    method(interp, xml, "defaultSettings", default_settings, 0)?;
    let xml_methods: [(&'static str, NativeFn, u32); 39] = [
        ("addNamespace", add_namespace, 1),
        ("appendChild", append_child, 1),
        ("attribute", attribute, 1),
        ("attributes", attributes, 0),
        ("child", child, 1),
        ("childIndex", child_index, 0),
        ("children", children, 0),
        ("comments", comments, 0),
        ("contains", contains, 1),
        ("copy", copy, 0),
        ("descendants", descendants, 1),
        ("elements", elements, 1),
        ("hasOwnProperty", has_own_property, 1),
        ("hasComplexContent", has_complex_content, 0),
        ("hasSimpleContent", has_simple_content, 0),
        ("inScopeNamespaces", in_scope_namespaces, 0),
        ("insertChildAfter", insert_child_after, 2),
        ("insertChildBefore", insert_child_before, 2),
        ("length", length, 0),
        ("localName", local_name, 0),
        ("name", name, 0),
        ("namespace", namespace, 1),
        ("namespaceDeclarations", namespace_declarations, 0),
        ("nodeKind", node_kind, 0),
        ("normalize", normalize, 0),
        ("parent", parent, 0),
        ("processingInstructions", processing_instructions, 1),
        ("prependChild", prepend_child, 1),
        ("propertyIsEnumerable", property_is_enumerable, 1),
        ("removeNamespace", remove_namespace, 1),
        ("replace", replace, 2),
        ("setChildren", set_children, 1),
        ("setLocalName", set_local_name, 1),
        ("setName", set_name, 1),
        ("setNamespace", set_namespace, 1),
        ("text", text, 0),
        ("toString", to_string, 0),
        ("toXMLString", to_xml_string, 0),
        ("valueOf", value_of, 0),
    ];
    for (name, f, n) in xml_methods {
        method(interp, xml_proto, name, f, n)?;
    }

    let list = native(interp, "XMLList", xml_list_ctor, 1, NativeKind::Plain, true)?;
    constructor(interp, "XMLList", list, list_proto)?;
    let list_methods: [(&'static str, NativeFn, u32); 21] = [
        ("attribute", attribute, 1),
        ("attributes", attributes, 0),
        ("child", child, 1),
        ("children", children, 0),
        ("comments", comments, 0),
        ("contains", contains, 1),
        ("copy", copy, 0),
        ("descendants", descendants, 1),
        ("elements", elements, 1),
        ("hasOwnProperty", has_own_property, 1),
        ("hasComplexContent", has_complex_content, 0),
        ("hasSimpleContent", has_simple_content, 0),
        ("length", length, 0),
        ("normalize", normalize, 0),
        ("parent", parent, 0),
        ("processingInstructions", processing_instructions, 1),
        ("propertyIsEnumerable", property_is_enumerable, 1),
        ("text", text, 0),
        ("toString", to_string, 0),
        ("toXMLString", to_xml_string, 0),
        ("valueOf", value_of, 0),
    ];
    for (name, f, n) in list_methods {
        method(interp, list_proto, name, f, n)?;
    }

    let namespace_function = native(interp, "Namespace", namespace_ctor, 2, NativeKind::Plain, true)?;
    constructor(interp, "Namespace", namespace_function, namespace_proto)?;
    method(interp, namespace_proto, "toString", namespace_to_string, 0)?;
    let qname_function = native(interp, "QName", qname_ctor, 2, NativeKind::Plain, true)?;
    constructor(interp, "QName", qname_function, qname_proto)?;
    method(interp, qname_proto, "toString", qname_to_string, 0)?;

    let global = interp.global;
    method(interp, global, "isXMLName", is_xml_name, 1)?;
    method(interp, global, "setDefaultXMLNamespace", set_default_xml_namespace, 1)
}

// ---- values of the classes ----

fn empty() -> Key {
    Rc::from(Vec::new())
}

fn key_value(k: &Key) -> Value {
    Value::String(Str::from_rc(k.clone()))
}

fn new_namespace(interp: &mut Interp, ns: XmlNs) -> JsResult<Value> {
    let bytes = OBJECT_BYTES + 2 * PROP_BYTES + string_bytes(ns.uri.len()) + ns.prefix.as_ref().map_or(0, |p| string_bytes(p.len()));
    interp.reserve_heap(bytes)?;
    let proto = interp.intrinsics.namespace_proto;
    let prefix = ns.prefix.as_ref().map_or(Value::Undefined, key_value);
    let uri = key_value(&ns.uri);
    let o = interp.heap.alloc_object(Object::new(Class::Namespace(Box::new(ns)), Some(proto)));
    interp.define(o, key("prefix"), Prop::Data { value: prefix, flags: ENUMERABLE })?;
    interp.define(o, key("uri"), Prop::Data { value: uri, flags: ENUMERABLE })?;
    Ok(Value::Object(o))
}

fn new_qname(interp: &mut Interp, name: XmlName, attribute: bool) -> JsResult<Value> {
    let bytes = OBJECT_BYTES + 2 * PROP_BYTES + string_bytes(name.local.len()) + name.uri.as_ref().map_or(0, |u| string_bytes(u.len()));
    interp.reserve_heap(bytes)?;
    let proto = interp.intrinsics.qname_proto;
    let local = key_value(&name.local);
    let uri = name.uri.as_ref().map_or(Value::Null, key_value);
    let o = interp.heap.alloc_object(Object::new(Class::QName { name: Box::new(name), attribute, reference: false }, Some(proto)));
    interp.define(o, key("localName"), Prop::Data { value: local, flags: ENUMERABLE })?;
    interp.define(o, key("uri"), Prop::Data { value: uri, flags: ENUMERABLE })?;
    Ok(Value::Object(o))
}

fn as_namespace(interp: &Interp, v: &Value) -> Option<XmlNs> {
    let Value::Object(o) = v else { return None };
    match interp.heap.object(*o).map(|ob| &ob.class) {
        Some(Class::Namespace(ns)) => Some((**ns).clone()),
        _ => None,
    }
}

fn as_qname(interp: &Interp, v: &Value) -> Option<(XmlName, bool)> {
    let Value::Object(o) = v else { return None };
    match interp.heap.object(*o).map(|ob| &ob.class) {
        Some(Class::QName { name, attribute, .. }) => Some(((**name).clone(), *attribute)),
        _ => None,
    }
}

/// The namespace `new Namespace(...)` makes from its arguments (ECMA-357 section 13.2.2).
fn build_namespace(interp: &mut Interp, args: &[Value]) -> JsResult<XmlNs> {
    match args {
        [] => Ok(XmlNs { prefix: Some(empty()), uri: empty() }),
        [uri] => {
            if let Some(ns) = as_namespace(interp, uri) {
                return Ok(ns);
            }
            if let Some((name, _)) = as_qname(interp, uri)
                && let Some(u) = name.uri
            {
                return Ok(XmlNs { prefix: name.prefix, uri: u });
            }
            let u = interp.to_str(uri)?.flat();
            let prefix = if u.is_empty() { Some(empty()) } else { None };
            Ok(XmlNs { prefix, uri: u })
        }
        [prefix_value, uri, ..] => {
            let qname_uri = as_qname(interp, uri).and_then(|(n, _)| n.uri);
            let u = match qname_uri {
                Some(u) => u,
                None => interp.to_str(uri)?.flat(),
            };
            if u.is_empty() {
                if matches!(prefix_value, Value::Undefined) || interp.to_str(prefix_value)?.len() == 0 {
                    return Ok(XmlNs { prefix: Some(empty()), uri: u });
                }
                return Err(interp.type_error("a namespace with an empty URI cannot have a prefix"));
            }
            if matches!(prefix_value, Value::Undefined) {
                return Ok(XmlNs { prefix: None, uri: u });
            }
            let p = interp.to_str(prefix_value)?.flat();
            Ok(XmlNs { prefix: is_ncname(&p).then_some(p), uri: u })
        }
    }
}

fn namespace_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    if !construct
        && let [only] = args
        && as_namespace(interp, only).is_some()
    {
        return Ok(only.clone());
    }
    let ns = build_namespace(interp, args)?;
    new_namespace(interp, ns)
}

fn namespace_to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    match as_namespace(interp, &this) {
        Some(ns) => Ok(key_value(&ns.uri)),
        None => Err(interp.type_error("Namespace.prototype.toString needs a Namespace")),
    }
}

/// The name `new QName(...)` makes from its arguments (ECMA-357 section 13.3.2).
fn build_qname(interp: &mut Interp, args: &[Value]) -> JsResult<(XmlName, bool)> {
    let (namespace, name_value) = match args {
        [] => (None, Value::Undefined),
        [name] => (None, name.clone()),
        [ns, name, ..] => (Some(ns.clone()), name.clone()),
    };
    let copied = as_qname(interp, &name_value);
    if let (Some(c), None) = (&copied, &namespace) {
        return Ok(c.clone());
    }
    let local = if let Some((n, _)) = copied {
        n.local
    } else if matches!(name_value, Value::Undefined) {
        empty()
    } else {
        interp.to_str(&name_value)?.flat()
    };
    let (uri, prefix) = match namespace {
        Some(Value::Undefined) | None => {
            if *local == *STAR {
                (None, None)
            } else {
                (Some(interp.xml_default_uri()?), None)
            }
        }
        Some(Value::Null) => (None, None),
        Some(ns) => {
            let n = build_namespace(interp, std::slice::from_ref(&ns))?;
            (Some(n.uri), n.prefix)
        }
    };
    Ok((XmlName { uri, local, prefix }, false))
}

fn qname_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    if !construct
        && let [only] = args
        && as_qname(interp, only).is_some()
    {
        return Ok(only.clone());
    }
    let (name, attribute) = build_qname(interp, args)?;
    new_qname(interp, name, attribute)
}

fn qname_to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let Some((name, attribute)) = as_qname(interp, &this) else {
        return Err(interp.type_error("QName.prototype.toString needs a QName"));
    };
    let mut units: Vec<u16> = Vec::new();
    if attribute {
        units.push(0x40);
    }
    match &name.uri {
        None => units.extend("*::".encode_utf16()),
        Some(u) if !u.is_empty() => {
            units.extend_from_slice(u);
            units.extend("::".encode_utf16());
        }
        _ => {}
    }
    units.extend_from_slice(&name.local);
    interp.reserve_heap(string_bytes(units.len()))?;
    Ok(Value::String(Str::from_units(units)))
}

// ---- constructors ----

/// ToXML (ECMA-357 section 10.3), with null and undefined read as the empty string.
fn to_xml(interp: &mut Interp, v: &Value) -> JsResult<ObjId> {
    match interp.classify(v) {
        Xv::Node(n) => Ok(n),
        Xv::List(l) => {
            if interp.list_len(l)? == 1
                && let Some(item) = interp.list_at(l, 0)?
            {
                return Ok(item);
            }
            Err(interp.type_error("an XMLList converts to XML only when it has one item"))
        }
        Xv::Other => {
            let text = if v.is_nullish() { empty() } else { interp.to_str(v)?.flat() };
            interp.xml_from_text(&text)
        }
    }
}

fn xml_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    let v = arg(args, 0);
    interp.rooted(std::slice::from_ref(&v), |i| {
        let x = to_xml(i, &v)?;
        if construct && matches!(i.classify(&v), Xv::Node(_)) {
            return Ok(Value::Object(i.node_copy(x)?));
        }
        Ok(Value::Object(x))
    })
}

fn xml_list_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    let v = arg(args, 0);
    match interp.classify(&v) {
        Xv::List(_) if !construct => return Ok(v),
        Xv::List(l) => return Ok(Value::Object(interp.list_shallow_copy(l)?)),
        _ => {}
    }
    interp.rooted(std::slice::from_ref(&v), |i| {
        let out = i.new_xml_list(None, None)?;
        i.xml_keep(out);
        match i.classify(&v) {
            Xv::Node(n) => i.list_push(out, n)?,
            _ if v.is_nullish() => {}
            _ => {
                let text = i.to_str(&v)?.flat();
                for node in i.xml_parse(&text)? {
                    i.list_push(out, node)?;
                }
            }
        }
        Ok(Value::Object(out))
    })
}

fn is_xml_name(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = interp.to_str(&arg(args, 0))?.flat();
    Ok(Value::Bool(is_script_xml_name(&s)))
}

fn set_default_xml_namespace(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let uri = interp.namespace_uri_of(&arg(args, 0))?;
    interp.xml_set_default_uri(uri)?;
    Ok(Value::Undefined)
}

// ---- settings ----

fn settings(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.new_object();
    let this = Value::Object(interp.intrinsics.xml_ctor);
    interp.rooted(&[Value::Object(o)], |i| {
        for (name, _) in SETTINGS {
            let Value::Object(xml) = this else { break };
            let v = i.get_from(xml, &key(name), &this)?;
            i.put(o, key(name), v, false)?;
        }
        Ok(Value::Object(o))
    })
}

fn default_settings(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.new_object();
    interp.rooted(&[Value::Object(o)], |i| {
        for (name, value) in SETTINGS {
            i.put(o, key(name), value, false)?;
        }
        Ok(Value::Object(o))
    })
}

fn set_settings(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let xml = interp.intrinsics.xml_ctor;
    let v = arg(args, 0);
    if v.is_nullish() {
        for (name, value) in SETTINGS {
            interp.put(xml, key(name), value, false)?;
        }
    } else if let Value::Object(o) = v {
        for (name, default) in SETTINGS {
            let s = interp.get_from(o, &key(name), &v)?;
            let same_type = matches!((&s, &default), (Value::Bool(_), Value::Bool(_)) | (Value::Number(_), Value::Number(_)));
            if same_type {
                interp.put(xml, key(name), s, false)?;
            }
        }
    }
    Ok(Value::Undefined)
}

// ---- methods ----

fn not_xml_method(interp: &mut Interp) -> Abrupt {
    interp.type_error("the method needs an XML value")
}

fn this_node(interp: &mut Interp, this: &Value) -> JsResult<ObjId> {
    match interp.classify(this) {
        Xv::Node(n) => Ok(n),
        _ => Err(not_xml_method(interp)),
    }
}

fn this_id(interp: &mut Interp, this: &Value) -> JsResult<ObjId> {
    match (interp.classify(this), this) {
        (Xv::Node(_) | Xv::List(_), Value::Object(o)) => Ok(*o),
        _ => Err(not_xml_method(interp)),
    }
}

fn nodes_of(interp: &mut Interp, this: &Value) -> JsResult<Vec<ObjId>> {
    match interp.classify(this) {
        Xv::Node(n) => Ok(vec![n]),
        Xv::List(l) => Ok(interp.xml_list(l)?.items.clone()),
        Xv::Other => Err(not_xml_method(interp)),
    }
}

fn content_of(interp: &mut Interp, v: &Value) -> JsResult<Content> {
    Ok(match interp.classify(v) {
        Xv::Node(n) => Content::Node(n),
        Xv::List(l) => Content::List(l),
        Xv::Other => Content::Text(interp.to_str(v)?),
    })
}

/// ToAttributeName (ECMA-357 section 10.5).
fn attribute_name(interp: &mut Interp, v: &Value) -> JsResult<NameRef> {
    if let Some((name, _)) = as_qname(interp, v) {
        return Ok(NameRef { uri: name.uri, local: name.local, attribute: true });
    }
    if matches!(v, Value::Undefined | Value::Null | Value::Bool(_) | Value::Number(_)) {
        return Err(interp.type_error("the value is not an XML attribute name"));
    }
    let local = interp.to_str(v)?.flat();
    let local = local.strip_prefix(&[0x40u16]).map_or_else(|| local.clone(), Rc::from);
    let uri = if *local == *STAR { None } else { Some(empty()) };
    Ok(NameRef { uri, local, attribute: true })
}

fn star() -> NameRef {
    NameRef { uri: None, local: Rc::from(STAR), attribute: false }
}

fn name_arg(interp: &mut Interp, args: &[Value]) -> JsResult<NameRef> {
    match args.first() {
        None | Some(Value::Undefined) => Ok(star()),
        Some(v) => match interp.xml_access(v)? {
            Access::Name(n) => Ok(n),
            Access::Index(i) => {
                let s = interp.to_str(&Value::Number(f64::from(i)))?.flat();
                Ok(NameRef { uri: Some(interp.xml_default_uri()?), local: s, attribute: false })
            }
        },
    }
}

fn attribute(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let id = this_id(interp, &this)?;
    let n = attribute_name(interp, &arg(args, 0))?;
    Ok(Value::Object(interp.xml_select(id, &n)?))
}

fn attributes(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let id = this_id(interp, &this)?;
    let n = NameRef { attribute: true, ..star() };
    Ok(Value::Object(interp.xml_select(id, &n)?))
}

fn children(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let id = this_id(interp, &this)?;
    Ok(Value::Object(interp.xml_select(id, &star())?))
}

fn child(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let id = this_id(interp, &this)?;
    let p = arg(args, 0);
    match interp.xml_access(&p)? {
        Access::Name(n) => Ok(Value::Object(interp.xml_select(id, &n)?)),
        Access::Index(i) => {
            let nodes = nodes_of(interp, &this)?;
            interp.rooted(std::slice::from_ref(&this), |it| {
                let out = it.new_xml_list(Some(id), None)?;
                it.xml_keep(out);
                for node in nodes {
                    it.use_fuel()?;
                    let found = it.xml_node(node)?.children.get(i as usize).copied();
                    if let Some(found) = found {
                        it.list_push(out, found)?;
                    }
                }
                Ok(Value::Object(out))
            })
        }
    }
}

/// The children of each node of `this` that `keep` accepts, in order.
fn select_children(
    interp: &mut Interp,
    this: &Value,
    target_name: Option<(XmlName, bool)>,
    keep: impl Fn(&Interp, ObjId) -> JsResult<bool>,
) -> JsResult<Value> {
    let id = this_id(interp, this)?;
    let nodes = nodes_of(interp, this)?;
    interp.rooted(std::slice::from_ref(this), |it| {
        let out = it.new_xml_list(Some(id), target_name)?;
        it.xml_keep(out);
        for node in nodes {
            for at in 0..it.xml_node(node)?.children.len() {
                it.use_fuel()?;
                let Some(c) = it.xml_node(node)?.children.get(at).copied() else { break };
                if keep(it, c)? {
                    it.list_push(out, c)?;
                }
            }
        }
        Ok(Value::Object(out))
    })
}

fn comments(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    select_children(interp, &this, None, |i, c| Ok(i.xml_kind(c)? == XmlKind::Comment))
}

fn text(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    select_children(interp, &this, None, |i, c| Ok(i.xml_kind(c)? == XmlKind::Text))
}

fn elements(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let n = name_arg(interp, args)?;
    let target = Some((XmlName { uri: n.uri.clone(), local: n.local.clone(), prefix: None }, false));
    select_children(interp, &this, target, |i, c| {
        let node = i.xml_node(c)?;
        Ok(node.kind == XmlKind::Element
            && node.name.as_ref().is_some_and(|q| (n.is_star() || q.local == n.local) && n.uri.as_ref().is_none_or(|u| q.uri.as_ref() == Some(u))))
    })
}

fn processing_instructions(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let n = name_arg(interp, args)?;
    select_children(interp, &this, None, |i, c| {
        let node = i.xml_node(c)?;
        Ok(node.kind == XmlKind::Pi && (n.is_star() || node.name.as_ref().is_some_and(|q| q.local == n.local)))
    })
}

fn descendants(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let n = name_arg(interp, args)?;
    interp.xml_descendants_of(&this, &n)
}

fn child_index(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let node = interp.xml_node(x)?;
    let (kind, parent) = (node.kind, node.parent);
    let Some(parent) = parent.filter(|_| kind != XmlKind::Attribute) else { return Ok(Value::Number(-1.0)) };
    let count = interp.xml_node(parent)?.children.len();
    for i in 0..count {
        interp.use_fuel()?;
        if interp.xml_node(parent)?.children.get(i) == Some(&x) {
            return Ok(Value::Number(i as f64));
        }
    }
    Ok(Value::Number(-1.0))
}

fn contains(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let v = arg(args, 0);
    match interp.classify(&this) {
        Xv::Node(_) => Ok(Value::Bool(interp.loose_eq(&this, &v)?)),
        Xv::List(l) => {
            for i in 0..interp.list_len(l)? {
                interp.use_fuel()?;
                let Some(item) = interp.list_at(l, i)? else { break };
                if interp.loose_eq(&Value::Object(item), &v)? {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        Xv::Other => Err(not_xml_method(interp)),
    }
}

fn copy(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    match interp.classify(&this) {
        Xv::Node(n) => Ok(Value::Object(interp.node_copy(n)?)),
        Xv::List(l) => Ok(Value::Object(interp.list_copy(l)?)),
        Xv::Other => Err(not_xml_method(interp)),
    }
}

fn has_own_property(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let p = arg(args, 0);
    if interp.is_xml(&this) && interp.xml_has(&this, &p)? {
        return Ok(Value::Bool(true));
    }
    let Value::Object(o) = this else { return Err(not_xml_method(interp)) };
    let k = interp.to_key(&p)?;
    Ok(Value::Bool(interp.get_own_property(o, &k)?.is_some()))
}

fn has_complex_content(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    match interp.classify(&this) {
        Xv::Node(n) => Ok(Value::Bool(interp.node_complex(n)?)),
        Xv::List(l) => {
            let len = interp.list_len(l)?;
            if len == 1
                && let Some(item) = interp.list_at(l, 0)?
            {
                return Ok(Value::Bool(interp.node_complex(item)?));
            }
            for i in 0..len {
                interp.use_fuel()?;
                if let Some(item) = interp.list_at(l, i)?
                    && interp.xml_kind(item)? == XmlKind::Element
                {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        Xv::Other => Err(not_xml_method(interp)),
    }
}

fn has_simple_content(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    match interp.classify(&this) {
        Xv::Node(n) => Ok(Value::Bool(interp.node_simple(n)?)),
        Xv::List(l) => Ok(Value::Bool(interp.list_simple(l)?)),
        Xv::Other => Err(not_xml_method(interp)),
    }
}

/// The namespaces in scope at `x`, nearest declarations first, one for each prefix.
fn scope_of(interp: &mut Interp, x: Option<ObjId>) -> JsResult<Vec<XmlNs>> {
    let mut out: Vec<XmlNs> = Vec::new();
    let mut cur = x;
    while let Some(y) = cur {
        let node = interp.xml_node(y)?.clone();
        for ns in &node.ns {
            interp.use_fuel()?;
            if !out.iter().any(|n| n.prefix == ns.prefix) {
                out.push(ns.clone());
            }
        }
        cur = node.parent;
    }
    Ok(out)
}

fn namespaces_array(interp: &mut Interp, list: Vec<XmlNs>) -> JsResult<Value> {
    interp.rooted(&[], |i| {
        let a = new_array(i);
        for (at, ns) in list.into_iter().enumerate() {
            let v = new_namespace(i, ns)?;
            set_element(i, a, at, v)?;
        }
        Ok(Value::Object(a))
    })
}

fn in_scope_namespaces(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let list = scope_of(interp, Some(x))?;
    namespaces_array(interp, list)
}

fn namespace_declarations(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    if interp.xml_kind(x)? != XmlKind::Element {
        return namespaces_array(interp, Vec::new());
    }
    let parent = interp.xml_node(x)?.parent;
    let ancestors = scope_of(interp, parent)?;
    let declared: Vec<XmlNs> =
        interp.xml_node(x)?.ns.iter().filter(|ns| !ancestors.iter().any(|a| a.prefix == ns.prefix && a.uri == ns.uri)).cloned().collect();
    namespaces_array(interp, declared)
}

fn namespace(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let scope = scope_of(interp, Some(x))?;
    if let Some(p) = args.first() {
        let p = interp.to_str(p)?.flat();
        return match scope.into_iter().find(|ns| ns.prefix.as_deref() == Some(&*p)) {
            Some(ns) => new_namespace(interp, ns),
            None => Ok(Value::Undefined),
        };
    }
    let node = interp.xml_node(x)?;
    if matches!(node.kind, XmlKind::Text | XmlKind::Comment | XmlKind::Pi) {
        return Ok(Value::Null);
    }
    let Some(name) = node.name.clone() else { return Ok(Value::Null) };
    let uri = name.uri.clone().unwrap_or_else(empty);
    let found = scope
        .iter()
        .find(|ns| ns.uri == uri && ns.prefix == name.prefix)
        .or_else(|| scope.iter().find(|ns| ns.uri == uri))
        .cloned()
        .unwrap_or(XmlNs { prefix: name.prefix, uri });
    new_namespace(interp, found)
}

fn length(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    match interp.classify(&this) {
        Xv::Node(_) => Ok(Value::Number(1.0)),
        Xv::List(l) => Ok(Value::Number(interp.list_len(l)? as f64)),
        Xv::Other => Err(not_xml_method(interp)),
    }
}

fn local_name(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    Ok(interp.xml_node(x)?.name.as_ref().map_or(Value::Null, |n| key_value(&n.local)))
}

fn name(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let node = interp.xml_node(x)?;
    let attribute = node.kind == XmlKind::Attribute;
    match node.name.clone() {
        Some(n) => new_qname(interp, n, attribute),
        None => Ok(Value::Null),
    }
}

fn node_kind(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    Ok(Value::str(match interp.xml_kind(x)? {
        XmlKind::Element => "element",
        XmlKind::Attribute => "attribute",
        XmlKind::Text => "text",
        XmlKind::Comment => "comment",
        XmlKind::Pi => "processing-instruction",
    }))
}

fn normalize(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    for node in nodes_of(interp, &this)? {
        if interp.xml_kind(node)? == XmlKind::Element {
            interp.node_normalize(node)?;
        }
    }
    Ok(this)
}

fn parent(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    match interp.classify(&this) {
        Xv::Node(x) => Ok(interp.xml_node(x)?.parent.map_or(Value::Null, Value::Object)),
        Xv::List(l) => {
            let Some(first) = interp.list_at(l, 0)? else { return Ok(Value::Undefined) };
            let p = interp.xml_node(first)?.parent;
            for i in 1..interp.list_len(l)? {
                interp.use_fuel()?;
                let Some(item) = interp.list_at(l, i)? else { break };
                if interp.xml_node(item)?.parent != p {
                    return Ok(Value::Undefined);
                }
            }
            Ok(p.map_or(Value::Undefined, Value::Object))
        }
        Xv::Other => Err(not_xml_method(interp)),
    }
}

fn property_is_enumerable(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let p = arg(args, 0);
    let s = interp.to_str(&p)?.flat();
    match interp.classify(&this) {
        Xv::Node(_) => Ok(Value::Bool(canonical_index(&s) == Some(0))),
        Xv::List(l) => Ok(Value::Bool(canonical_index(&s).is_some_and(|i| (i as usize) < interp.list_len(l).unwrap_or(0)))),
        Xv::Other => Err(not_xml_method(interp)),
    }
}

/// [[AddInScopeNamespace]] (ECMA-357 section 9.1.1.13).
fn add_in_scope(interp: &mut Interp, x: ObjId, ns: XmlNs) -> JsResult<()> {
    if interp.xml_kind(x)? != XmlKind::Element {
        return Ok(());
    }
    let Some(prefix) = ns.prefix.clone() else { return Ok(()) };
    if !ns_binding_allowed(&prefix, &ns.uri) {
        return Err(interp.type_error(RESERVED_NS_BINDING));
    }
    let node = interp.xml_node(x)?;
    if prefix.is_empty() && node.name.as_ref().and_then(|n| n.uri.as_ref()).is_some_and(|u| u.is_empty()) {
        return Ok(());
    }
    interp.reserve_heap(crate::heap::XML_NS_BYTES)?;
    let attrs = interp.xml_node(x)?.attrs.clone();
    let node = interp.xml_node_mut(x)?;
    node.ns.retain(|m| m.prefix.as_ref() != Some(&prefix));
    node.ns.push(ns);
    if let Some(name) = &mut node.name
        && name.prefix.as_ref() == Some(&prefix)
    {
        name.prefix = None;
    }
    for a in attrs {
        interp.use_fuel()?;
        if let Some(name) = &mut interp.xml_node_mut(a)?.name
            && name.prefix.as_ref() == Some(&prefix)
        {
            name.prefix = None;
        }
    }
    Ok(())
}

fn add_namespace(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let ns = build_namespace(interp, std::slice::from_ref(&arg(args, 0)))?;
    add_in_scope(interp, x, ns)?;
    Ok(this)
}

fn append_child(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let v = arg(args, 0);
    interp.rooted(&[this.clone(), v.clone()], |i| {
        let list = i.xml_select(x, &star())?;
        i.xml_keep(list);
        let at = i.list_len(list)?;
        i.xml_put(&Value::Object(list), &Value::Number(at as f64), &v)
    })?;
    Ok(this)
}

fn prepend_child(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let v = arg(args, 0);
    let c = content_of(interp, &v)?;
    interp.rooted(std::slice::from_ref(&v), |i| i.xml_insert(x, 0, &c))?;
    Ok(this)
}

fn insert_relative(interp: &mut Interp, this: Value, args: &[Value], after: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    if interp.xml_kind(x)? != XmlKind::Element {
        return Ok(Value::Undefined);
    }
    let reference = arg(args, 0);
    let v = arg(args, 1);
    let c = content_of(interp, &v)?;
    let len = interp.xml_node(x)?.children.len();
    let at = if reference.is_nullish() {
        if after { 0 } else { len }
    } else {
        let Xv::Node(r) = interp.classify(&reference) else { return Ok(Value::Undefined) };
        interp.use_fuel()?;
        let Some(i) = interp.xml_node(x)?.children.iter().position(|&c| c == r) else { return Ok(Value::Undefined) };
        if after { i + 1 } else { i }
    };
    interp.rooted(std::slice::from_ref(&v), |i| i.xml_insert(x, at, &c))?;
    Ok(this)
}

fn insert_child_after(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    insert_relative(interp, this, args, true)
}

fn insert_child_before(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    insert_relative(interp, this, args, false)
}

/// One element of a `removeNamespace` walk; true when the walk goes on to its children.
fn remove_ns_from(interp: &mut Interp, x: ObjId, ns: &XmlNs) -> JsResult<bool> {
    let node = interp.xml_node(x)?;
    let attrs = node.attrs.clone();
    if node.name.as_ref().and_then(|n| n.uri.as_ref()) == Some(&ns.uri) {
        return Ok(false);
    }
    for a in attrs {
        interp.use_fuel()?;
        if interp.xml_node(a)?.name.as_ref().and_then(|n| n.uri.as_ref()) == Some(&ns.uri) {
            return Ok(false);
        }
    }
    let list = &mut interp.xml_node_mut(x)?.ns;
    match &ns.prefix {
        None => list.retain(|m| m.uri != ns.uri),
        Some(p) => list.retain(|m| !(m.uri == ns.uri && m.prefix.as_ref() == Some(p))),
    }
    Ok(true)
}

fn remove_namespace(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    if interp.xml_kind(x)? != XmlKind::Element {
        return Ok(this);
    }
    let ns = build_namespace(interp, std::slice::from_ref(&arg(args, 0)))?;
    if !remove_ns_from(interp, x, &ns)? {
        return Ok(this);
    }
    let mut stack = vec![(x, 0usize)];
    while let Some(&(node, next)) = stack.last() {
        interp.use_fuel()?;
        let Some(c) = interp.xml_node(node)?.children.get(next).copied() else {
            stack.pop();
            continue;
        };
        if let Some(frame) = stack.last_mut() {
            frame.1 += 1;
        }
        if interp.xml_kind(c)? == XmlKind::Element && remove_ns_from(interp, c, &ns)? {
            stack.push((c, 0));
        }
    }
    Ok(this)
}

fn replace(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    if interp.xml_kind(x)? != XmlKind::Element {
        return Ok(this);
    }
    let p = arg(args, 0);
    let v = arg(args, 1);
    interp.rooted(&[this.clone(), v.clone()], |i| {
        let c = match i.classify(&v) {
            Xv::Node(n) => {
                let copy = i.node_copy(n)?;
                i.xml_keep(copy);
                Content::Node(copy)
            }
            Xv::List(l) => {
                let copy = i.list_copy(l)?;
                i.xml_keep(copy);
                Content::List(copy)
            }
            Xv::Other => Content::Text(i.to_str(&v)?),
        };
        let n = match i.xml_access(&p)? {
            Access::Index(at) => return i.xml_replace(x, at as usize, &c),
            Access::Name(n) => n,
        };
        let matched = i.xml_select(x, &NameRef { attribute: false, ..n })?;
        i.xml_keep(matched);
        let count = i.list_len(matched)?;
        if count == 0 {
            return Ok(());
        }
        let Some(first) = i.list_at(matched, 0)? else { return Ok(()) };
        for at in (1..count).rev() {
            let Some(item) = i.list_at(matched, at)? else { continue };
            i.use_fuel()?;
            let position = i.xml_node(x)?.children.iter().position(|&c| c == item);
            if let Some(q) = position {
                let removed = i.xml_node_mut(x)?.children.remove(q);
                i.xml_node_mut(removed)?.parent = None;
            }
        }
        i.use_fuel()?;
        let at = i.xml_node(x)?.children.iter().position(|&c| c == first).unwrap_or(0);
        i.xml_replace(x, at, &c)
    })?;
    Ok(this)
}

fn set_children(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_node(interp, &this)?;
    interp.xml_put(&this, &Value::str("*"), &arg(args, 0))?;
    Ok(this)
}

fn checked_local(interp: &mut Interp, local: &Key) -> JsResult<()> {
    if is_ncname(local) {
        return Ok(());
    }
    Err(interp.type_error("the value is not a valid XML name"))
}

fn set_local_name(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let kind = interp.xml_kind(x)?;
    if matches!(kind, XmlKind::Text | XmlKind::Comment) {
        return Ok(Value::Undefined);
    }
    let v = arg(args, 0);
    let local = match as_qname(interp, &v) {
        Some((n, _)) => n.local,
        None => interp.to_str(&v)?.flat(),
    };
    checked_local(interp, &local)?;
    if kind == XmlKind::Attribute
        && let Some(old) = interp.xml_node(x)?.name.clone()
    {
        check_name_binding(interp, &XmlName { local: local.clone(), ..old }, true)?;
    }
    if let Some(n) = &mut interp.xml_node_mut(x)?.name {
        n.local = local;
    }
    Ok(Value::Undefined)
}

/// Throws when an element or attribute named `name` could not be written as XML.
fn check_name_binding(interp: &mut Interp, name: &XmlName, attribute: bool) -> JsResult<()> {
    let uri = name.uri.as_deref().unwrap_or_default();
    if !name.prefix.as_ref().is_none_or(|prefix| ns_binding_allowed(prefix, uri)) || !xml_name_writable(name, attribute) {
        return Err(interp.type_error(RESERVED_NS_BINDING));
    }
    Ok(())
}

fn name_set_in_scope(interp: &mut Interp, x: ObjId, name: &XmlName) -> JsResult<()> {
    let ns = XmlNs { prefix: name.prefix.clone(), uri: name.uri.clone().unwrap_or_else(empty) };
    match interp.xml_kind(x)? {
        XmlKind::Attribute => match interp.xml_node(x)?.parent {
            Some(p) => add_in_scope(interp, p, ns),
            None => Ok(()),
        },
        XmlKind::Element => add_in_scope(interp, x, ns),
        _ => Ok(()),
    }
}

fn set_name(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let kind = interp.xml_kind(x)?;
    if matches!(kind, XmlKind::Text | XmlKind::Comment) {
        return Ok(Value::Undefined);
    }
    let v = arg(args, 0);
    let mut name = match as_qname(interp, &v) {
        Some((n, _)) if n.uri.is_some() => n,
        Some((n, _)) => build_qname(interp, &[key_value(&n.local)])?.0,
        None => build_qname(interp, std::slice::from_ref(&v))?.0,
    };
    checked_local(interp, &name.local)?;
    if kind == XmlKind::Pi {
        name.uri = Some(empty());
    } else {
        check_name_binding(interp, &name, kind == XmlKind::Attribute)?;
    }
    interp.xml_node_mut(x)?.name = Some(name.clone());
    name_set_in_scope(interp, x, &name)?;
    Ok(Value::Undefined)
}

fn set_namespace(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_node(interp, &this)?;
    let kind = interp.xml_kind(x)?;
    if matches!(kind, XmlKind::Text | XmlKind::Comment | XmlKind::Pi) {
        return Ok(Value::Undefined);
    }
    let ns = build_namespace(interp, std::slice::from_ref(&arg(args, 0)))?;
    let Some(old) = interp.xml_node(x)?.name.clone() else { return Ok(Value::Undefined) };
    let name = XmlName { uri: Some(ns.uri), local: old.local, prefix: ns.prefix };
    check_name_binding(interp, &name, kind == XmlKind::Attribute)?;
    interp.xml_node_mut(x)?.name = Some(name.clone());
    name_set_in_scope(interp, x, &name)?;
    Ok(Value::Undefined)
}

fn to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::String(interp.xml_to_string_value(&this)?))
}

fn to_xml_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::String(interp.xml_string_of(&this, None)?))
}

fn value_of(_: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(this)
}
