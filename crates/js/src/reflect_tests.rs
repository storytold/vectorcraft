//! `reflect`: the description of an object's properties and methods, from a host class's members
//! or from an object's own properties.

use std::rc::Rc;

use super::*;

fn realm() -> Realm {
    let mut r = Realm::new();
    r.add_script_extensions().unwrap();
    r
}

fn text(r: &mut Realm, source: &str) -> String {
    match r.run(source) {
        Ok(JsValue::String(s)) => s,
        other => panic!("{source}: {other:?}"),
    }
}

fn thrown(r: &mut Realm, source: &str) -> String {
    match r.run(source) {
        Err(e) if e.kind == RunErrorKind::Thrown => e.message,
        other => panic!("{source}: {other:?}"),
    }
}

/// `name:type:dataType` of each entry of a list, joined with spaces.
const LIST: &str = "function list(a) { var s = []; for (var i = 0; i < a.length; i++) s.push(a[i].name + ':' + a[i].type + ':' + a[i].dataType); \
                    return s.join(' '); }";

#[test]
fn a_script_object_lists_its_own_properties_and_methods_without_running_getters() {
    let mut r = realm();
    let setup = "var o = {a: 1, b: 'x', c: null, d: [1], f: function (p, q) {}, get g() { throw new Error('ran'); }, set h(v) {}};\
                 Object.defineProperty(o, 'k', {value: true, writable: false});";
    r.run(setup).unwrap();
    r.run(LIST).unwrap();
    assert_eq!(text(&mut r, "o.reflect.name"), "Object");
    assert_eq!(
        text(&mut r, "list(o.reflect.properties)"),
        "a:readwrite:number b:readwrite:string c:readwrite:null d:readwrite:Array g:readonly:unknown h:readwrite:unknown k:readonly:boolean"
    );
    assert_eq!(text(&mut r, "list(o.reflect.methods)"), "f:method:unknown");
    assert_eq!(text(&mut r, "list(o.reflect.methods[0].arguments)"), "p:parameter:* q:parameter:*");
    assert_eq!(text(&mut r, "list(o.reflect.properties[0].arguments) + '|' + o.reflect.description + '|' + o.reflect.help"), "|undefined|undefined");
    // A value replaced by a function becomes a method on the next read; each read is a new snapshot.
    assert_eq!(
        text(&mut r, "var before = o.reflect; o.a = function () {}; delete o.b; list(before.methods) + '|' + list(o.reflect.methods)"),
        "f:method:unknown|a:method:unknown f:method:unknown"
    );
    assert_eq!(text(&mut r, "String(o.reflect.find('b'))"), "null");
}

#[test]
fn find_and_names_look_up_properties_then_methods() {
    let mut r = realm();
    r.run("var o = {width: 2, draw: function () {}};").unwrap();
    assert_eq!(text(&mut r, "o.reflect.find('width').dataType + ' ' + o.reflect.find('draw').type"), "number method");
    assert_eq!(text(&mut r, "String(o.reflect.find('nothing'))"), "null");
    assert_eq!(text(&mut r, "o.reflect.properties.width.name + ' ' + o.reflect.methods.draw.name"), "width draw");
    // Array indexes are numbers, found by number or by their text.
    assert_eq!(
        text(
            &mut r,
            "var a = [5, 'x']; a[3] = true; var f = a.reflect.find(1); typeof f.name + ' ' + f.dataType + ' ' + a.reflect.find('3').dataType"
        ),
        "number string boolean"
    );
    assert_eq!(text(&mut r, "[5].reflect.properties.length + ' ' + [5].reflect.find('length').type"), "2 readwrite");
    // A name that is missing or of another type is refused.
    assert!(thrown(&mut r, "o.reflect.find();").starts_with("TypeError"));
    assert!(thrown(&mut r, "o.reflect.find({});").starts_with("TypeError"));
    assert!(thrown(&mut r, "o.reflect.find.call({}, 'width');").starts_with("TypeError"));
}

#[test]
fn find_charges_the_flattening_of_a_rope_name() {
    // Three ropes of about 524,288 units, each 1 MiB once flat, with a 3 MiB heap.
    let build = "var o = {a: 1}; var s = 'x'; for (var i = 0; i < 19; i++) s = s + s; var t = s + 'y', u = s + 'z';";
    let limits = Limits { heap_bytes: 3 << 20, ..Limits::default() };
    let mut r = Realm::with_options(Options::default(), limits);
    r.add_script_extensions().unwrap();
    match r.run(&format!("{build} o.reflect.find(s); o.reflect.find(t); o.reflect.find(u); 0")) {
        Err(e) => assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_rest_parameter_is_listed_with_the_arguments() {
    let mut r = Realm::with_options(Options { es2015: true, ..Options::default() }, Limits::default());
    r.add_script_extensions().unwrap();
    r.run(LIST).unwrap();
    assert_eq!(text(&mut r, "var o = {f: function (first, ...rest) {}}; list(o.reflect.methods[0].arguments)"), "first:parameter:* rest:parameter:*");
    assert_eq!(text(&mut r, "var g = {f: function (...all) {}}; list(g.reflect.find('f').arguments)"), "all:parameter:*");
    // A destructured parameter has no name of its own.
    assert_eq!(
        text(
            &mut r,
            "var d = {f: function (a, [b, c], ...[e]) {}}; list(d.reflect.methods[0].arguments) + '|' + d.reflect.methods[0].arguments.length"
        ),
        "a:parameter:* undefined:parameter:* undefined:parameter:*|3"
    );
}

#[test]
fn a_symbol_is_described_by_its_prototype() {
    let mut r = Realm::with_options(Options { es2015: true, ..Options::default() }, Limits::default());
    r.add_script_extensions().unwrap();
    r.run(LIST).unwrap();
    assert_eq!(text(&mut r, "Symbol('s').reflect.name"), "Symbol");
    assert_eq!(text(&mut r, "Symbol('s').reflect.find('toString').type"), "method");
    assert_eq!(text(&mut r, "var o = {s: Symbol('s')}; o.reflect.find('s').dataType"), "symbol");
}

#[test]
fn a_reflection_info_has_every_field() {
    let mut r = realm();
    let fields = "var i = ({a: 1}).reflect.properties[0]; var k = []; for (var n in i) k.push(n); k.join(' ')";
    assert_eq!(text(&mut r, fields), "name description help type dataType defaultValue min max isCollection arguments");
    let values =
        "var i = ({a: 1}).reflect.properties[0]; [i.description, i.help, i.defaultValue, i.min, i.max, i.isCollection, i.arguments.length].join()";
    assert_eq!(text(&mut r, values), ",,,,,false,0");
    assert_eq!(text(&mut r, "var k = []; for (var n in ({}).reflect) k.push(n); k.join(' ')"), "name description help properties methods");
}

#[test]
fn functions_primitives_and_built_ins_are_described() {
    let mut r = realm();
    r.run(LIST).unwrap();
    assert_eq!(
        text(&mut r, "function f(a, b) {} f.reflect.name + ' ' + list(f.reflect.properties)"),
        "Function length:readonly:number prototype:readwrite:Object"
    );
    assert_eq!(text(&mut r, "Math.reflect.name + ' ' + Math.reflect.find('PI').type + ' ' + Math.reflect.find('max').type"), "Math readonly method");
    assert_eq!(text(&mut r, "'abc'.reflect.name + ' ' + 'abc'.reflect.find('charAt').type"), "String method");
    assert_eq!(text(&mut r, "(5).reflect.name + ' ' + true.reflect.name"), "Number Boolean");
    assert!(thrown(&mut r, "var n = null; n.reflect;").starts_with("TypeError"));
    let getter = "Object.getOwnPropertyDescriptor(Object.prototype, 'reflect').get";
    assert!(thrown(&mut r, &format!("{getter}.call(undefined);")).starts_with("TypeError"));
    // Neither a getter nor `toString` runs, so a cyclic object is described as it is.
    assert_eq!(text(&mut r, "var c = {}; c.self = c; c.toString = function () { throw 1; }; c.reflect.find('self').dataType"), "Object");
}

#[test]
fn a_reflection_cannot_be_changed() {
    let mut r = realm();
    r.run("var o = {a: 1}; var x = o.reflect;").unwrap();
    // Outside strict mode the writes are ignored.
    let tries = "o.reflect = 5; x.name = 'y'; x.properties[0].dataType = 'y'; x.properties[1] = 2; x.extra = 1; delete x.methods;\
                 [typeof o.reflect, x.name, x.properties[0].dataType, x.properties.length, x.extra, typeof x.methods].join()";
    assert_eq!(text(&mut r, tries), "object,Object,number,1,,object");
    // In strict mode each one throws a TypeError.
    for write in
        ["o.reflect = 5", "x.name = 'y'", "x.properties[0].type = 'y'", "x.properties.push(1)", "x.extra = 1", "delete x.find", "x.find.extra = 1"]
    {
        assert!(thrown(&mut r, &format!("(function () {{ 'use strict'; {write}; }})();")).starts_with("TypeError"), "{write}");
    }
    assert_eq!(
        text(
            &mut r,
            "Object.isFrozen(x) + ' ' + Object.isFrozen(x.properties) + ' ' + Object.isFrozen(x.properties[0].arguments) + ' ' + Object.isFrozen(x.find)"
        ),
        "true true true true"
    );
    // The frozen `find` still finds.
    assert_eq!(text(&mut r, "x.find('a').name"), "a");
}

#[test]
fn a_very_wide_object_is_refused() {
    let mut r = realm();
    let wide = |n: usize| format!("var w = {{}}; for (var i = 0; i < {n}; i++) w['p' + i] = i;");
    r.run(&wide(MAX_REFLECTED_MEMBERS)).unwrap();
    assert_eq!(text(&mut r, "String(w.reflect.properties.length)"), MAX_REFLECTED_MEMBERS.to_string());
    r.run(&wide(MAX_REFLECTED_MEMBERS + 1)).unwrap();
    assert!(thrown(&mut r, "w.reflect;").starts_with("RangeError"));
    // A function with too many parameters is counted with the members.
    let params = (0..MAX_REFLECTED_MEMBERS).map(|i| format!("a{i}")).collect::<Vec<_>>().join(",");
    assert!(thrown(&mut r, &format!("({{f: function ({params}) {{}}}}).reflect;")).starts_with("RangeError"));
}

fn constant(value: f64) -> HostFunction {
    Rc::new(move |_| Ok(Data::Number(value)))
}

/// `Item` with `size` (read-write, 1 to 10, default 5), `kind` and the method `fit(target,
/// margin?)`; and `Box`, a child that declares `kind` again as a method and adds `cut`, which is
/// not available.
fn classes() -> Vec<HostClass> {
    let item = HostClass {
        name: "Item".into(),
        parent: None,
        members: vec![
            ("size".into(), HostMember::Property { get: constant(5.0), set: constant(0.0) }),
            ("kind".into(), HostMember::Get(constant(1.0))),
            ("fit".into(), HostMember::Method(constant(2.0))),
        ],
        info: vec![
            (
                "size".into(),
                MemberInfo {
                    data_type: "number".into(),
                    description: Some("The size.".into()),
                    help: Some("How large the item is.".into()),
                    default: Some(Data::Number(5.0)),
                    min: Some(1.0),
                    max: Some(10.0),
                    ..MemberInfo::default()
                },
            ),
            (
                "fit".into(),
                MemberInfo {
                    data_type: "boolean".into(),
                    collection: true,
                    arguments: vec![
                        ArgInfo { name: "target".into(), data_type: "Item".into(), ..ArgInfo::default() },
                        ArgInfo {
                            name: "margin".into(),
                            data_type: "number".into(),
                            default: Some(Data::Number(0.0)),
                            min: Some(0.0),
                            ..ArgInfo::default()
                        },
                    ],
                    ..MemberInfo::default()
                },
            ),
        ],
        index: None,
        construct: None,
    };
    let child = HostClass {
        name: "Box".into(),
        parent: Some("Item".into()),
        members: vec![("kind".into(), HostMember::Method(constant(3.0))), ("cut".into(), HostMember::Method(constant(4.0)))],
        info: vec![("cut".into(), MemberInfo { unavailable: true, ..MemberInfo::default() })],
        index: None,
        construct: None,
    };
    vec![item, child]
}

#[test]
fn a_host_class_is_described_by_its_members_and_their_info() {
    let mut r = realm();
    r.define_host_classes(classes(), vec![("item".into(), host_ref("Item", &[1.0])), ("box".into(), host_ref("Box", &[2.0]))]).unwrap();
    r.run(LIST).unwrap();
    for target in ["item", "Item"] {
        assert_eq!(text(&mut r, &format!("{target}.reflect.name")), "Item");
        assert_eq!(text(&mut r, &format!("list({target}.reflect.properties)")), "size:readwrite:number kind:readonly:unknown");
        assert_eq!(text(&mut r, &format!("list({target}.reflect.methods)")), "fit:method:boolean");
    }
    let size = "var s = item.reflect.find('size'); [s.description, s.help, s.defaultValue, s.min, s.max, s.isCollection].join('|')";
    assert_eq!(text(&mut r, size), "The size.|How large the item is.|5|1|10|false");
    let fit = "var f = item.reflect.find('fit'); f.isCollection + ' ' + list(f.arguments) + ' ' + f.arguments.margin.defaultValue + ' ' + f.arguments[1].min";
    assert_eq!(text(&mut r, fit), "true target:parameter:Item margin:parameter:number 0 0");
    // The child lists its parent's members first, its own declaration of one in its place, and
    // leaves out the member that is not available.
    for target in ["box", "Box"] {
        assert_eq!(text(&mut r, &format!("{target}.reflect.name")), "Box");
        assert_eq!(text(&mut r, &format!("list({target}.reflect.properties)")), "size:readwrite:number");
        assert_eq!(text(&mut r, &format!("list({target}.reflect.methods)")), "kind:method:unknown fit:method:boolean");
        assert_eq!(text(&mut r, &format!("String({target}.reflect.find('cut'))")), "null");
    }
    // No member's function ran: a data property of the class's type names it.
    assert_eq!(text(&mut r, "({b: box}).reflect.find('b').dataType"), "Box");
}

#[test]
fn info_for_a_member_a_class_does_not_have_is_refused() {
    for (info, message) in [
        (vec![("nothing".to_string(), MemberInfo::default())], "not one of its members"),
        (vec![("size".to_string(), MemberInfo::default()), ("size".to_string(), MemberInfo::default())], "twice"),
    ] {
        let mut r = realm();
        let mut class = classes().remove(0);
        class.info = info;
        let e = r.define_host_classes(vec![class], Vec::new()).unwrap_err();
        assert!(e.message.contains(message), "{}", e.message);
    }
    let mut r = realm();
    let mut class = classes().remove(0);
    class.members.push(("size".into(), HostMember::Get(constant(1.0))));
    let e = r.define_host_classes(vec![class], Vec::new()).unwrap_err();
    assert!(e.message.contains("names the member size twice"), "{}", e.message);
    // `constructor` is the class's own constructor, so no member may take its name.
    let mut r = realm();
    let mut class = classes().remove(0);
    class.members.push(("constructor".into(), HostMember::Get(constant(1.0))));
    let e = r.define_host_classes(vec![class], Vec::new()).unwrap_err();
    assert!(e.message.contains("names a member constructor"), "{}", e.message);
    assert_eq!(text(&mut r, "typeof Item"), "undefined");
}
