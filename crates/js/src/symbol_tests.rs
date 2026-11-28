//! `Symbol`, Symbol-keyed properties and the iterators of arrays, strings and `arguments`
//! objects, in a realm with [`Options::es2015`].

use crate::{JsValue, LimitKind, Limits, Options, Realm, RunError, RunErrorKind};

fn es2015(limits: Limits) -> Realm {
    Realm::with_options(Options { es2015: true, ..Options::default() }, limits)
}

fn run(src: &str) -> JsValue {
    match es2015(Limits::default()).run(src) {
        Ok(v) => v,
        Err(e) => panic!("{src}\n{e:?}"),
    }
}

fn truth(src: &str) {
    assert_eq!(run(src), JsValue::Bool(true), "{src}");
}

fn string(src: &str) -> String {
    match run(src) {
        JsValue::String(s) => s,
        other => panic!("{src}: {other:?}"),
    }
}

/// The run throws an error whose message starts with `name`.
fn throws(src: &str, name: &str) {
    match es2015(Limits::default()).run(src) {
        Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with(name), "{src}: {message}"),
        other => panic!("{src}: {other:?}"),
    }
}

#[test]
fn symbols_are_unique_values_with_a_description() {
    truth("typeof Symbol() === 'symbol' && typeof Symbol('a') === 'symbol'");
    truth("Symbol('a') !== Symbol('a') && Symbol('a') != Symbol('a')");
    truth("var s = Symbol('a'); s === s && s == Object(s) && Object(s) == s");
    truth("Symbol('a').description === 'a' && Symbol().description === undefined && Symbol('').description === ''");
    assert_eq!(string("String(Symbol('a b'))"), "Symbol(a b)");
    assert_eq!(string("Symbol().toString()"), "Symbol()");
    truth("var s = Symbol(); Object(s).valueOf() === s && typeof Object(s) === 'object'");
    truth("Object.prototype.toString.call(Symbol()) === '[object Symbol]'");
    truth("!!Symbol() && Symbol.length === 0 && Symbol.prototype.constructor === Symbol");
    throws("new Symbol()", "TypeError");
    throws("Symbol.prototype.toString.call({})", "TypeError");
}

#[test]
fn symbol_for_and_key_for_use_the_registry() {
    truth("Symbol.for('k') === Symbol.for('k') && Symbol.for('k') !== Symbol('k')");
    truth("Symbol.keyFor(Symbol.for('k')) === 'k' && Symbol.keyFor(Symbol('k')) === undefined");
    truth("Symbol.keyFor(Symbol.iterator) === undefined && Symbol.for('x').description === 'x'");
    throws("Symbol.keyFor('k')", "TypeError");
    throws("Symbol.for(Symbol())", "TypeError");
}

#[test]
fn symbol_registry_hashing_charges_flat_keys_on_hits_and_misses() {
    for hit in [false, true] {
        let mut realm = es2015(Limits::default());
        let key = "x".repeat(32_768);
        assert_eq!(realm.run(&format!("var registryKey = '{key}';")), Ok(JsValue::Undefined));
        if hit {
            assert_eq!(realm.run("typeof Symbol.for(registryKey)"), Ok(JsValue::String("symbol".into())));
        }
        realm.set_limits(Limits { fuel: 500, ..Limits::default() });
        assert!(matches!(realm.run("Symbol.for(registryKey)"), Err(RunError { kind: RunErrorKind::Limit(LimitKind::Fuel), .. })), "hit={hit}");
        realm.set_limits(Limits { fuel: 10_000, ..Limits::default() });
        assert_eq!(realm.run("Symbol.for(registryKey) === Symbol.for(registryKey)"), Ok(JsValue::Bool(true)));
    }
}

#[test]
fn symbol_registry_keeps_a_flat_key_buffer_only_once() {
    let mut realm = es2015(Limits::default());
    let key = "x".repeat(32_768);
    assert_eq!(realm.run(&format!("var sharedKey = '{key}';")), Ok(JsValue::Undefined));
    let used = realm.collect_garbage();
    let room = crate::heap::SYMBOL_BYTES + crate::symbol::REGISTRY_ENTRY_BYTES + 4_096;
    realm.set_limits(Limits { heap_bytes: used + room, ..Limits::default() });
    assert_eq!(realm.run("var registered = Symbol.for(sharedKey); Symbol.keyFor(registered) === sharedKey"), Ok(JsValue::Bool(true)));
    assert!(realm.collect_garbage() <= used + room);
}

#[test]
fn implicit_conversion_of_a_symbol_throws_a_type_error() {
    throws("Symbol() + ''", "TypeError");
    throws("'' + Symbol('a')", "TypeError");
    throws("+Symbol()", "TypeError");
    throws("Symbol() * 2", "TypeError");
    throws("Symbol() < 1", "TypeError");
    throws("[Symbol()].join()", "TypeError");
    throws("'abc'.indexOf(Symbol())", "TypeError");
    throws("Symbol(Symbol())", "TypeError");
    throws("new String(Symbol())", "TypeError");
    truth("String(Symbol('x')) === 'Symbol(x)' && Symbol('x').toString() === 'Symbol(x)'");
}

#[test]
fn symbol_keys_are_hidden_from_for_in_keys_and_json() {
    let setup = "var s = Symbol('s'); var o = {a: 1}; o[s] = 2; ";
    truth(&format!("{setup} var n = []; for (var k in o) n.push(k); n.join() === 'a'"));
    truth(&format!("{setup} Object.keys(o).join() === 'a' && Object.getOwnPropertyNames(o).join() === 'a'"));
    truth(&format!("{setup} JSON.stringify(o) === '{{\"a\":1}}'"));
    truth("JSON.stringify({a: Symbol()}) === '{}' && JSON.stringify([Symbol()]) === '[null]' && JSON.stringify(Symbol()) === undefined");
    truth(&format!("{setup} var l = Object.getOwnPropertySymbols(o); l.length === 1 && l[0] === s && o[s] === 2"));
    truth(&format!("{setup} s in o && o.hasOwnProperty(s) && o.propertyIsEnumerable(s) && delete o[s] && !(s in o)"));
    truth(&format!("{setup} o[Object(s)] === 2 && o[s.toString()] === undefined"));
    throws("var w = new String('x'); w.toString = function () { return Symbol(); }; JSON.stringify(w)", "TypeError");
    truth("var o = Object(Symbol()); delete Symbol.prototype[Symbol.toStringTag]; Object.prototype.toString.call(o) === '[object Object]'");
}

#[test]
fn define_property_freeze_and_descriptors_take_symbol_keys() {
    truth(
        "var s = Symbol(); var o = {}; Object.defineProperty(o, s, {value: 1}); \
         var d = Object.getOwnPropertyDescriptor(o, s); d.value === 1 && !d.writable && !d.enumerable && !d.configurable",
    );
    truth("var s = Symbol(); var o = {}; o[s] = 1; Object.freeze(o); o[s] = 2; o[s] === 1 && Object.isFrozen(o)");
    truth("var s = Symbol(); var o = {}; o[s] = 1; Object.preventExtensions(o); Object.isSealed(o) === false");
    truth("var s = Symbol(); var p = {}; p[s] = {value: 3, enumerable: true}; var o = Object.create(null, p); o[s] === 3");
    throws("'use strict'; var s = Symbol(); var o = Object.freeze((function () { var x = {}; x[s] = 1; return x; })()); o[s] = 2;", "TypeError");
    truth("var s = Symbol(); var o = {}; o[s] = 1; o[s]++; o[s] === 2");
}

#[test]
fn code_run_without_the_option_reads_no_to_string_tag() {
    let mut realm = es2015(Limits::default());
    let setup = "var read = 0, o = {}; Object.defineProperty(o, Symbol.toStringTag, { get: function () { read++; return 'Thing'; } }); \
                 Object.prototype.toString.call(o) === '[object Thing]' && read === 1";
    assert_eq!(realm.run(setup), Ok(JsValue::Bool(true)));
    realm.set_es2015(false);
    assert_eq!(realm.run("Object.prototype.toString.call(o) === '[object Object]' && read === 1"), Ok(JsValue::Bool(true)));
}

#[test]
fn code_run_without_the_option_calls_no_to_primitive_or_has_instance_method() {
    let mut realm = es2015(Limits::default());
    let setup = "var o = { valueOf: function () { return 1; } }; o[Symbol.toPrimitive] = function () { return 2; }; \
                 var c = {}; c[Symbol.hasInstance] = function () { return true; }; \
                 function F() {} var B = F.bind(); \
                 Object.defineProperty(F, Symbol.hasInstance, { value: function () { return true; } }); \
                 o + 1 === 3 && ({} instanceof c) && ({} instanceof B)";
    assert_eq!(realm.run(setup), Ok(JsValue::Bool(true)));
    realm.set_es2015(false);
    assert_eq!(realm.run("o + 1 === 2 && !({} instanceof B)"), Ok(JsValue::Bool(true)));
    match realm.run("({} instanceof c)") {
        Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with("TypeError"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn well_known_symbols_hook_the_operations_that_read_them() {
    truth("var o = {}; o[Symbol.toStringTag] = 'Thing'; Object.prototype.toString.call(o) === '[object Thing]'");
    truth("var o = {}; o[Symbol.toPrimitive] = function (h) { return h === 'number' ? 7 : 'str'; }; +o === 7 && String(o) === 'str'");
    throws("var o = {}; o[Symbol.toPrimitive] = function () { return {}; }; +o", "TypeError");
    throws("var o = {}; o[Symbol.toPrimitive] = 1; +o", "TypeError");
    truth("var C = {}; C[Symbol.hasInstance] = function (v) { return v === 1; }; (1 instanceof C) && !(2 instanceof C)");
    truth("function F() {} var B = F.bind(); new F() instanceof B && typeof Function.prototype[Symbol.hasInstance] === 'function'");
    truth("Function.prototype[Symbol.hasInstance].call(Array, []) === true && Function.prototype[Symbol.hasInstance].call({}, []) === false");
    truth(
        "Symbol.iterator.toString() === 'Symbol(Symbol.iterator)' && typeof Symbol.asyncIterator === 'symbol' \
         && typeof Symbol.hasInstance === 'symbol' && typeof Symbol.toPrimitive === 'symbol' && typeof Symbol.toStringTag === 'symbol'",
    );
}

#[test]
fn an_array_iterator_driven_by_hand_steps_through_values_keys_and_entries() {
    truth(
        "var it = [10, 20][Symbol.iterator](); var a = it.next(), b = it.next(), c = it.next(), d = it.next(); \
         a.value === 10 && !a.done && b.value === 20 && !b.done && c.value === undefined && c.done && d.done",
    );
    truth("var it = ['a', 'b'].keys(); it.next().value === 0 && it.next().value === 1 && it.next().done");
    truth("var e = ['x'].entries().next().value; e.length === 2 && e[0] === 0 && e[1] === 'x'");
    truth("Array.prototype[Symbol.iterator] === Array.prototype.values");
    truth("var a = [1]; var it = a.values(); it.next(); a.push(2); it.next().value === 2");
    truth("var it = [].values(); it[Symbol.iterator]() === it && Object.prototype.toString.call(it) === '[object Array Iterator]'");
    truth("var it = Array.prototype.values.call({length: 1, 0: 'q'}); it.next().value === 'q'");
    throws("[].values().next.call({})", "TypeError");
    throws("[].values().next.call(''[Symbol.iterator]())", "TypeError");
}

#[test]
fn a_string_iterator_steps_by_code_point() {
    truth(
        "var it = 'a\\uD83D\\uDE00b'[Symbol.iterator](); var x = it.next().value, y = it.next().value, z = it.next().value; \
         x === 'a' && y === '\\uD83D\\uDE00' && y.length === 2 && z === 'b' && it.next().done",
    );
    truth("var it = '\\uDE00\\uD83D'[Symbol.iterator](); it.next().value === '\\uDE00' && it.next().value === '\\uD83D' && it.next().done");
    truth("Object.prototype.toString.call(''[Symbol.iterator]()) === '[object String Iterator]'");
    throws("String.prototype[Symbol.iterator].call(null)", "TypeError");
}

#[test]
fn arguments_objects_have_the_array_values_iterator() {
    truth("(function () { return arguments[Symbol.iterator] === Array.prototype.values; })()");
    truth("(function () { 'use strict'; var it = arguments[Symbol.iterator](); return it.next().value === 5 && it.next().done; })(5)");
    truth("(function () { return !arguments.propertyIsEnumerable(Symbol.iterator); })()");
}

#[test]
fn realms_without_es2015_have_no_symbol() {
    for options in [Options::default(), Options { e4x: true, preprocessor_lines: true, ..Options::default() }] {
        let mut realm = Realm::with_options(options, Limits::default());
        let src = "typeof Symbol === 'undefined' && Array.prototype.values === undefined && Object.getOwnPropertySymbols === undefined";
        assert_eq!(realm.run(src), Ok(JsValue::Bool(true)));
        let src = "Object.getOwnPropertyNames((function () { return arguments; })()).join()";
        assert_eq!(realm.run(src), Ok(JsValue::String("length,callee".into())));
    }
}

#[test]
fn symbols_and_the_registry_count_against_the_heap_limit() {
    let limits = || Limits { heap_bytes: 1 << 20, fuel: 50_000_000, ..Limits::default() };
    let mut realm = es2015(limits());
    match realm.run("for (var i = 0; ; i++) Symbol.for('key' + i);") {
        Err(RunError { kind: RunErrorKind::Limit(LimitKind::Heap), .. }) => {}
        other => panic!("{other:?}"),
    }
    let mut realm = es2015(limits());
    match realm.run("var keep = []; for (var i = 0; ; i++) keep.push(Symbol('d' + i));") {
        Err(RunError { kind: RunErrorKind::Limit(LimitKind::Heap), .. }) => {}
        other => panic!("{other:?}"),
    }
    // Symbols nothing holds are collected, so making them one after another does not fill the
    // heap, and keys survive a collection.
    let mut realm = es2015(limits());
    let src = "var s = Symbol('k'); var o = {}; o[s] = 'v'; for (var i = 0; i < 40000; i++) Symbol('x' + i); o[s] === 'v'";
    assert_eq!(realm.run(src), Ok(JsValue::Bool(true)));
}

#[test]
fn an_error_names_a_long_symbol_by_the_start_of_its_description() {
    // `d` is a rope of 4,194,304 units whose halves share one node, so it takes little heap until
    // something copies it whole. Each error names the Symbol by the first 256 units only.
    let setup = "var d = 'x'; for (var i = 0; i < 22; i++) d = d + d; var s = Symbol(d); \
                 var shut = Object.preventExtensions({}); var fixed = Object.defineProperty({}, s, { value: 1 }); ";
    let named = format!("Symbol({}...)", "x".repeat(256));
    for (attempt, message) in [
        ("shut[s] = 1", format!("cannot assign to property '{named}': the object is not extensible")),
        ("Object.defineProperty(shut, s, { value: 1 })", format!("cannot define property '{named}': the object is not extensible")),
        ("Object.defineProperty(fixed, s, { value: 2 })", format!("cannot define property '{named}': the property cannot be redefined")),
        ("fixed[s] = 2", format!("cannot assign to property '{named}': it is read-only")),
        ("delete fixed[s]", format!("cannot delete property '{named}'")),
        ("null[s]", "cannot read a property of null".to_string()),
    ] {
        let src = format!(
            "{setup} (function () {{ 'use strict'; try {{ {attempt}; return 'none'; }} catch (e) {{ return (e instanceof TypeError) + ' ' + e.message; }} }})()"
        );
        let mut realm = es2015(Limits { heap_bytes: 64 << 20, fuel: 1_000_000, ..Limits::default() });
        assert_eq!(realm.run(&src), Ok(JsValue::String(format!("true {message}"))), "{attempt}");
    }
    // A write that does not throw leaves the Symbol's description as it is.
    let src = format!("{setup} shut[s] = 1; fixed[s] = 2; shut[s] === undefined && fixed[s] === 1");
    assert_eq!(es2015(Limits { heap_bytes: 64 << 20, fuel: 1_000_000, ..Limits::default() }).run(&src), Ok(JsValue::Bool(true)));
}

#[test]
fn an_array_iterator_keeps_its_object_while_reading_its_length_runs_script() {
    // The `length` getter finishes the iterator by stepping it again, which lets go of `obj`, and
    // the length's `valueOf` drops the last reference to `obj`, fills the heap until it collects
    // and then reuses every free slot. The step still reads the element of `obj`.
    let src = "var obj = { 0: 'x', 1: 'y' }, it = Array.prototype.values.call(obj), calls = 0, junk, fresh = []; \
               Object.defineProperty(obj, 'length', { get: function () { \
                 calls++; if (calls > 1) return 0; it.next(); \
                 return { valueOf: function () { obj = null; \
                   for (var i = 0; i < 60000; i++) junk = { a: i }; \
                   for (var j = 0; j < 70000; j++) fresh.push({ 0: 'wrong' }); return 2; } }; } }); 0; \
               var r = it.next(); r.value + ' ' + r.done + ' ' + it.next().done";
    assert_eq!(string(src), "x false true");
}

#[test]
fn binary_and_octal_strings_are_numbers_in_an_es2015_realm_only() {
    for length in ["0b10", "0o2", "2"] {
        let src = format!(
            "var it = Array.prototype.values.call({{0: 'first', 1: 'second', length: '{length}'}}); \
             var a = it.next(), b = it.next(); a.value + ' ' + b.value + ' ' + it.next().done"
        );
        assert_eq!(string(&src), "first second true", "{length}");
    }
    truth("Number('0b101') === 5 && +'0O17' === 15 && '0b11' == 3 && isNaN('0b2') && isNaN('-0o1')");
    let src = "isNaN(Number('0b10')) && isNaN(+'0o2') && '0b1' != 1";
    assert_eq!(Realm::with_options(Options::default(), Limits::default()).run(src), Ok(JsValue::Bool(true)));
}

#[test]
fn json_calls_the_to_primitive_method_of_a_wrapper() {
    let hook = "w[Symbol.toPrimitive] = function (hint) { return hint === 'number' ? 2 : 'b'; };";
    truth(&format!("var w = new String('x'); {hook} JSON.stringify(w) === '\"b\"'"));
    truth(&format!("var w = new String('x'); delete String.prototype.toString; {hook} JSON.stringify(w) === '\"b\"'"));
    truth(&format!("var w = new String('x'); {hook} JSON.stringify({{a: 1, b: 2}}, [w]) === '{{\"b\":2}}'"));
    truth(&format!("var w = new Number(5); {hook} JSON.stringify([w], null, w) === '[\\n  2\\n]'"));
    truth("JSON.stringify([new String('x'), new Number(1)], null, new Number(1)) === '[\\n \"x\",\\n 1\\n]'");
    // A `@@toPrimitive` getter that deletes the prototype's method and gives `undefined` sends the
    // lookup on to `Object.prototype`.
    let getter = |method: &str| {
        format!(
            "Object.defineProperty(w, Symbol.toPrimitive, {{ get: function () {{ delete Object.getPrototypeOf(w).{method}; return undefined; }} }});"
        )
    };
    let over = "Object.prototype.toString = function () { return '7'; };";
    truth(&format!("var w = new String('x'); {over} {} JSON.stringify(w) === '\"7\"'", getter("toString")));
    truth(&format!("var w = new Number(3); {over} delete Number.prototype.toString; {} JSON.stringify(w) === '7'", getter("valueOf")));
}

#[test]
fn a_symbol_key_from_to_primitive_counts_against_the_heap_while_the_property_is_used() {
    // `mk(n)` makes a string of `n` units. The key's accessor deletes itself from
    // `Object.prototype`, so when the key is a Symbol only the read or write holds it and its
    // description of 2,000,000 units, and then makes a string of 1,500,000 units. With `keep`
    // (1,000,000 units) and a description still counted, that string does not fit; without one it
    // does.
    let src = |key: &str, access: &str| {
        format!(
            "function mk(n) {{ return new Array(n + 1).join('x'); }} \
             var keep = mk(1000000), made = null; \
             function use() {{ var p = Object.prototype; delete p[Object.getOwnPropertySymbols(p)[0] || 'k']; \
               made = mk(1500000); return 0; }} \
             var w = {{}}; w[Symbol.toPrimitive] = function () {{ var k = {key}; \
               Object.defineProperty(Object.prototype, k, {{ configurable: true, get: use, set: use }}); return k; }}; \
             {access}; made.length"
        )
    };
    let limits = || Limits { heap_bytes: 8 << 20, fuel: 50_000_000, ..Limits::default() };
    for access in ["({})[w]", "({})[w] = 1", "({})[w]++"] {
        // A name as the key leaves the Symbol and its description to the collector, and the string
        // fits.
        assert_eq!(es2015(limits()).run(&src("(Symbol(mk(2000000)), 'k')", access)), Ok(JsValue::Number(1_500_000.0)), "{access}");
        match es2015(limits()).run(&src("Symbol(mk(2000000))", access)) {
            Err(RunError { kind: RunErrorKind::Limit(LimitKind::Heap), .. }) => {}
            other => panic!("{access}: {other:?}"),
        }
    }
}

#[test]
fn define_properties_keeps_a_symbol_key_counted_after_a_getter_deletes_it() {
    // The second Symbol-keyed descriptor's getter deletes the first property, drops `s` and makes
    // a string of 1,500,000 units. With `keep` (1,000,000 units) and the first Symbol's
    // description of 2,000,000 units still counted, that string does not fit; with a short
    // description it does.
    let src = |description: &str, define: &str| {
        format!(
            "function mk(n) {{ return new Array(n + 1).join('x'); }} \
             var keep = mk(1000000), made = null, s = Symbol({description}), props = {{}}; \
             props[s] = {{ value: 1, enumerable: true }}; \
             Object.defineProperty(props, Symbol('t'), {{ enumerable: true, get: function () {{ \
               delete props[s]; s = null; made = mk(1500000); return {{ value: 2 }}; }} }}); \
             var o = {define}; o[Object.getOwnPropertySymbols(o)[0]] + ' ' + made.length"
        )
    };
    let limits = || Limits { heap_bytes: 8 << 20, fuel: 50_000_000, ..Limits::default() };
    for define in ["Object.defineProperties({}, props)", "Object.create(null, props)"] {
        assert_eq!(es2015(limits()).run(&src("'d'", define)), Ok(JsValue::String("1 1500000".into())), "{define}");
        match es2015(limits()).run(&src("mk(2000000)", define)) {
            Err(RunError { kind: RunErrorKind::Limit(LimitKind::Heap), .. }) => {}
            other => panic!("{define}: {other:?}"),
        }
    }
}

#[test]
fn an_operator_keeps_the_first_operand_converted_while_it_converts_the_second() {
    // `a` converts to a Symbol with a description of 2,000,000 units that nothing else holds, and
    // `b`'s conversion makes a string of 1,500,000 units. With `keep` (1,000,000 units) and the
    // description still counted, that string does not fit; when `a` gives a number it does.
    let src = |first: &str, op: &str| {
        format!(
            "function mk(n) {{ return new Array(n + 1).join('x'); }} \
             var keep = mk(1000000), made = null; \
             var a = {{}}; a[Symbol.toPrimitive] = function () {{ return {first}; }}; \
             var b = {{}}; b[Symbol.toPrimitive] = function () {{ made = mk(1500000); return 1; }}; \
             try {{ a {op} b; }} catch (e) {{ if (!(e instanceof TypeError)) throw e; }} \
             made.length === 1500000"
        )
    };
    let limits = || Limits { heap_bytes: 8 << 20, fuel: 50_000_000, ..Limits::default() };
    for op in ["+", "<", ">", "<=", ">="] {
        assert_eq!(es2015(limits()).run(&src("(Symbol(mk(2000000)), 0)", op)), Ok(JsValue::Bool(true)), "{op}");
        match es2015(limits()).run(&src("Symbol(mk(2000000))", op)) {
            Err(RunError { kind: RunErrorKind::Limit(LimitKind::Heap), .. }) => {}
            other => panic!("{op}: {other:?}"),
        }
    }
}

#[test]
fn date_objects_convert_through_their_to_primitive_method() {
    let date = "var d = new Date(0); d.valueOf = function () { return 7; }; d.toString = function () { return 'date'; };";
    truth(&format!("{date} d + 1 === 'date1' && d * 1 === 7 && String(d) === 'date' && (d == 'date')"));
    // Without a method, a Date converts with `valueOf` first when no hint is given.
    // The inherited method is read-only, so an assignment does not shadow it.
    truth(&format!("{date} d[Symbol.toPrimitive] = null; d + 1 === 'date1'"));
    truth(&format!("{date} Object.defineProperty(d, Symbol.toPrimitive, {{ value: null }}); d + 1 === 8"));
    truth(&format!("{date} Object.defineProperty(d, Symbol.toPrimitive, {{ value: undefined }}); d + 1 === 8 && String(d) === 'date'"));
    truth(&format!("{date} delete Date.prototype[Symbol.toPrimitive]; d + 1 === 8 && d == 7"));
    // The method itself, on any object.
    truth(
        "var m = Date.prototype[Symbol.toPrimitive], p = Object.getOwnPropertyDescriptor(Date.prototype, Symbol.toPrimitive); \
         typeof m === 'function' && m.length === 1 && !p.writable && !p.enumerable && p.configurable",
    );
    let o = "var o = { valueOf: function () { return 7; }, toString: function () { return 's'; } }, m = Date.prototype[Symbol.toPrimitive];";
    truth(&format!("{o} m.call(o, 'number') === 7 && m.call(o, 'string') === 's' && m.call(o, 'default') === 's'"));
    for call in ["m.call(o)", "m.call(o, 'other')", "m.call(o, new String('number'))", "m.call(1, 'number')", "m.call(o, 'defaultx')"] {
        throws(&format!("{o} {call}"), "TypeError");
    }
    // A realm without the option keeps the String-hint order for a Date.
    let src = format!("{date} d + 1 === 'date1' && d * 1 === 7");
    assert_eq!(Realm::with_options(Options::default(), Limits::default()).run(&src), Ok(JsValue::Bool(true)));
}

#[test]
fn a_host_function_returns_a_symbol_only_in_an_es2015_realm() {
    let symbol = |_: &[JsValue]| -> Result<JsValue, String> { Ok(JsValue::Symbol(Some("from host".into()))) };
    let mut realm = es2015(Limits::default());
    realm.define_function("make", symbol);
    assert_eq!(realm.run("var s = make(); typeof s === 'symbol' && String(s) === 'Symbol(from host)' && s !== make()"), Ok(JsValue::Bool(true)));
    let mut realm = Realm::with_options(Options::default(), Limits::default());
    realm.define_function("make", symbol);
    let src = "try { make(); 'none'; } catch (e) { (e instanceof TypeError) + ' ' + typeof Symbol; }";
    assert_eq!(realm.run(src), Ok(JsValue::String("true undefined".into())));
}

#[test]
fn a_host_symbol_description_counts_against_the_heap_before_it_is_made() {
    let mut realm = es2015(Limits { heap_bytes: 1 << 20, ..Limits::default() });
    realm.define_function("make", |_| Ok(JsValue::Symbol(Some("d".repeat(1 << 20)))));
    match realm.run("make()") {
        Err(RunError { kind: RunErrorKind::Limit(LimitKind::Heap), .. }) => {}
        other => panic!("{other:?}"),
    }
    let mut realm = es2015(Limits { heap_bytes: 1 << 20, ..Limits::default() });
    realm.define_function("make", |_| Ok(JsValue::Symbol(Some("d".repeat(1_000)))));
    assert_eq!(realm.run("make().description.length"), Ok(JsValue::Number(1_000.0)));
}

#[test]
fn a_symbol_lookup_takes_fuel_for_each_prototype() {
    // Building the chain takes about 20,000 units; each lookup of an absent Symbol walks all 2,000
    // prototypes, so the 100 lookups take 200,000 more.
    let src = "var s = Symbol(), o = {}; for (var i = 0; i < 2000; i++) o = Object.create(o); \
               for (var j = 0; j < 100; j++) s in o;";
    match es2015(Limits { fuel: 150_000, ..Limits::default() }).run(src) {
        Err(RunError { kind: RunErrorKind::Limit(LimitKind::Fuel), .. }) => {}
        other => panic!("{other:?}"),
    }
    let src = "var s = Symbol(), root = {}, o = root; root[s] = 'found'; for (var i = 0; i < 50; i++) o = Object.create(o); \
               o[s] === 'found' && s in o && !o.hasOwnProperty(s)";
    truth(src);
}

#[test]
fn iterator_steps_take_fuel() {
    let mut realm = es2015(Limits { fuel: 5_000, ..Limits::default() });
    let src = "var a = []; a.length = 1e9; var it = a.values(); while (!it.next().done) {}";
    match realm.run(src) {
        Err(RunError { kind: RunErrorKind::Limit(LimitKind::Fuel), .. }) => {}
        other => panic!("{other:?}"),
    }
}
