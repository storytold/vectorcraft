//! Tests of `String`, `Number`, `RegExp` and `Date` run through a realm.

use proptest::prelude::*;

use crate::regexp_vm::{INST_BYTES, ITEM_BYTES};
use crate::{JsValue, LimitKind, Limits, Options, Realm, RunError, RunErrorKind};

fn run_in(realm: &mut Realm, src: &str) -> JsValue {
    match realm.run(src) {
        Ok(v) => v,
        Err(e) => panic!("{src}\n{e:?}"),
    }
}

fn run(src: &str) -> JsValue {
    run_in(&mut Realm::new(), src)
}

fn string(src: &str) -> String {
    match run(src) {
        JsValue::String(s) => s,
        other => panic!("{src}: {other:?}"),
    }
}

/// Script helpers that need no `Array.prototype` method: `show(a, sep)` joins an array (`null`
/// shows as `null`, `undefined` elements as nothing), `nan(n)` tells NaN apart.
const HELPERS: &str = "function show(a, sep) { if (a === null) return 'null'; var s = ''; \
    for (var i = 0; i < a.length; i++) s += (i ? (sep === undefined ? ',' : sep) : '') + (a[i] === undefined ? '' : a[i]); return s; } \
    function nan(n) { return n !== n ? 'NaN' : 'num'; }\n";

/// Runs `src` after [`HELPERS`] and checks it gives the string `want`.
fn shows(src: &str, want: &str) {
    match run(&format!("{HELPERS}{src}")) {
        JsValue::String(s) => assert_eq!(s, want, "{src}"),
        other => panic!("{src}: {other:?}"),
    }
}

fn truth(src: &str) {
    assert_eq!(run(src), JsValue::Bool(true), "{src}");
}

fn fail_with(mut realm: Realm, src: &str) -> RunError {
    match realm.run(src) {
        Ok(v) => panic!("{src} completed with {v:?}"),
        Err(e) => e,
    }
}

fn thrown(src: &str) -> String {
    let e = fail_with(Realm::new(), src);
    assert_eq!(e.kind, RunErrorKind::Thrown, "{src}: {e:?}");
    e.message
}

// ---- String ----

#[test]
fn string_converts_and_wraps() {
    assert_eq!(string("String(12.5) + String() + String(null)"), "12.5null");
    truth("var s = new String('ab'); typeof s === 'object' && s.length === 2 && s[1] === 'b' && s.valueOf() === 'ab'");
    assert_eq!(string("String.fromCharCode(104, 105, 65536 + 33)"), "hi!");
    assert!(thrown("String.prototype.toString.call({})").starts_with("TypeError"));
    assert!(thrown("String.prototype.trim.call(null)").starts_with("TypeError"));
}

#[test]
fn string_methods_follow_section_15_5_4() {
    let cases: &[(&str, &str)] = &[
        ("'abc'.charAt(1) + 'abc'.charAt(5) + 'abc'.charAt(-1)", "b"),
        ("'' + 'abc'.charCodeAt(0) + nan('abc'.charCodeAt(3))", "97NaN"),
        ("'a'.concat('b', 1, null)", "ab1null"),
        ("'' + 'abcabc'.indexOf('c') + 'abcabc'.indexOf('c', 3) + 'abc'.indexOf('') + 'abc'.indexOf('', 9)", "2503"),
        ("'' + 'abcabc'.lastIndexOf('c') + 'abcabc'.lastIndexOf('c', 4) + 'abc'.lastIndexOf('x')", "52-1"),
        ("'' + 'a'.localeCompare('b') + 'b'.localeCompare('a') + 'a'.localeCompare('a')", "-110"),
        ("'abcdef'.slice(1, -1) + '|' + 'abcdef'.slice(-2) + '|' + 'abc'.slice(2, 1)", "bcde|ef|"),
        ("'abcdef'.substring(4, 1) + '|' + 'abcdef'.substring(-3, 2) + '|' + 'abc'.substring(1)", "bcd|ab|bc"),
        ("'abcdef'.substr(-3, 2) + '|' + 'abcdef'.substr(1) + '|' + 'abc'.substr(1, -1)", "de|bcdef|"),
        ("'\\u00dfa\\u03a3'.toUpperCase() + '|' + 'A\\u03a3'.toLowerCase() + '|' + 'A\\u03a3b'.toLowerCase()", "SSA\u{3a3}|a\u{3c2}|a\u{3c3}b"),
        ("'[' + ' \\t\\n\\u00a0\\ufeffx y\\u2028 '.trim() + ']'", "[x y]"),
        ("show('a,b,,c'.split(','), '|')", "a|b||c"),
        ("show('abc'.split(''), '|') + ';' + 'abc'.split().length + ';' + show('abc'.split('', 2), '|')", "a|b|c;1;a|b"),
        ("''.split('').length + ';' + ''.split('x').length", "0;1"),
        ("show('a1b22c'.split(/(\\d)+/), '|')", "a|1|b|2|c"),
        ("show('ab'.split(/(?:)/), '|') + ';' + show('ab'.split(/a*?/), '|')", "a|b;a|b"),
        ("'aaa'.replace('a', '$&$&') + ';' + 'abc'.replace('b', '[$`|$\\'|$$]')", "aaaa;a[a|c|$]c"),
        ("'John Smith'.replace(/(\\w+)\\s(\\w+)/, '$2, $1') + ';' + 'x'.replace(/x/, '$1$01$10')", "Smith, John;$1$01$10"),
        ("'a-b-c'.replace(/-/g, function (m, i, s) { return '[' + i + s.length + ']'; })", "a[15]b[35]c"),
        ("'aaa'.replace(/a*?/g, '-')", "-a-a-a-"),
        ("'abc'.match(/b/).index + ';' + show('abab'.match(/a(b)/g), '|') + ';' + ('x'.match(/y/g) === null)", "1;ab|ab;true"),
        ("'' + 'abc'.search(/c/) + 'abc'.search('b') + 'abc'.search(/x/)", "21-1"),
    ];
    for (src, want) in cases {
        shows(src, want);
    }
}

#[test]
fn string_methods_count_against_the_heap_before_allocating() {
    // Doubling with `concat` and `replace` stops at the heap limit, not at an allocation failure.
    let limits = Limits { heap_bytes: 8 << 20, ..Limits::default() };
    for src in [
        "var s = 'x'; for (;;) s = s.concat(s);",
        "var s = 'xx'; for (;;) s = s.replace(/x/g, 'xx');",
        "var s = 'x'; for (;;) s = (s + s).toUpperCase();",
        "var s = 'x'; for (;;) s = (s + s + s).slice(0) + s.substr(0);",
    ] {
        let e = fail_with(Realm::with_options(Options::default(), limits.clone()), src);
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{src}: {e:?}");
    }
    // A replacement that would build a string past the heap limit is refused before it is built.
    let limits = Limits { heap_bytes: 4 << 20, ..Limits::default() };
    let e = fail_with(
        Realm::with_options(Options::default(), limits),
        "var big = 'yyyyyyyyyyyyyyyy'; for (var i = 0; i < 15; i++) big = big + big; 'xxxxxxxxxx'.replace(/x/g, big)",
    );
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
}

// ---- Number ----

#[test]
fn numbers_format_as_section_15_7_4_says() {
    let cases: &[(&str, &str)] = &[
        ("(255).toString(16) + ' ' + (-255).toString(2) + ' ' + (0.5).toString(2) + ' ' + (1e21).toString(36)", "ff -11111111 0.1 5v1j4f4ds7c000"),
        ("(123.456).toString() + ' ' + (1e21).toString(10) + ' ' + (0.000001).toString()", "123.456 1e+21 0.000001"),
        ("(0.5).toFixed(0) + ' ' + (1.255).toFixed(2) + ' ' + (0.1).toFixed(20) + ' ' + (1e21).toFixed(2)", "1 1.25 0.10000000000000000555 1e+21"),
        ("(123456).toExponential(2) + ' ' + (123456).toExponential() + ' ' + (0).toExponential(2)", "1.23e+5 1.23456e+5 0.00e+0"),
        (
            "(123.456).toPrecision(4) + ' ' + (0.0000001234).toPrecision(2) + ' ' + (123456).toPrecision(2) + ' ' + (5).toPrecision()",
            "123.5 1.2e-7 1.2e+5 5",
        ),
        (
            "(-1.5).toFixed(0) + ' ' + (NaN).toFixed(2) + ' ' + (Infinity).toExponential(50) + ' ' + (-Infinity).toPrecision(50)",
            "-2 NaN Infinity -Infinity",
        ),
        ("new Number(5).toString(2) + ' ' + Number('0x10') + ' ' + Number() + ' ' + Number.MIN_VALUE", "101 16 0 5e-324"),
    ];
    for (src, want) in cases {
        assert_eq!(string(src), *want, "{src}");
    }
    truth("Number.MAX_VALUE === 1.7976931348623157e308 && Number.POSITIVE_INFINITY === 1/0 && Number.NaN !== Number.NaN");
    truth(
        "typeof new Number(1) === 'object' && new Number(7).valueOf() === 7 && Object.prototype.toString.call(Number.prototype) === '[object Number]'",
    );
    for src in [
        "(1).toFixed(21)",
        "(1).toFixed(-1)",
        "(1).toExponential(21)",
        "(1).toPrecision(0)",
        "(1).toPrecision(22)",
        "(1).toString(1)",
        "(1).toString(37)",
    ] {
        assert!(thrown(src).starts_with("RangeError"), "{src}");
    }
    assert!(thrown("Number.prototype.toFixed.call('1', 2)").starts_with("TypeError"));
}

// ---- RegExp ----

#[test]
fn regular_expressions_match_as_section_15_10_2_says() {
    let cases: &[(&str, &str)] = &[
        ("show(/a(b)?c/.exec('ac'))", "ac,"),
        ("show(/(z)((a+)?(b+)?(c))*/.exec('zaacbbbcac'))", "zaacbbbcac,z,ac,a,,c"),
        ("show(/(a*)*/.exec('b'))", ","),
        ("show(/(a*)b\\1+/.exec('baaaac'))", "b,"),
        ("show(/(?=(a+))/.exec('baaabac'))", ",aaa"),
        ("show(/(?=(a+))a*b\\1/.exec('baaabac'))", "aba,a"),
        ("show(/(.*?)a(?!(a+)b\\2c)\\2(.*)/.exec('baaabaac'))", "baaabaac,ba,,abaac"),
        ("show(/((a)|b)+/.exec('ab'))", "ab,b,"),
        ("show(/a[a-z]{2,4}/.exec('abcdefghi'))", "abcde"),
        ("show(/a[a-z]{2,4}?/.exec('abcdefghi'))", "abc"),
        ("show(/(aa|aabaac|ba|b|c)*/.exec('aabaac'))", "aaba,ba"),
        ("show(/[^\\d\\s]+/.exec('12 ab3'))", "ab"),
        ("show(/\\bfoo\\B/.exec('a foox'))", "foo"),
        ("show(/^b/m.exec('a\\nb')) + /^b/.test('a\\nb')", "bfalse"),
        ("/ß/i.test('SS') + ' ' + show(/[a-z]+/i.exec('HeLLo')) + ' ' + /\\u017f/i.test('s')", "false HeLLo false"),
        ("/[\\u017f]/i.test('S') + ' ' + /k/i.test('\\u212a')", "false false"),
        ("show(/\\1(a)/.exec('aa')) + ';' + show(/(a\\1)/.exec('aa'))", "a,a;a,a"),
        ("/\\0\\x41\\u0042\\cJ\\101/.test('\\0AB\\nA') + ' ' + /[\\b]/.test('\\b') + ' ' + /a{,2}/.test('a{,2}')", "true true true"),
        ("/]/.test(']') + ' ' + show(/[\\d-z]+/.exec('5-z')) + ' ' + /\\8/.test('8')", "true 5-z true"),
        ("/(?:a|b)*c/.exec('ababc').index + ' ' + /x*/.exec('yyy').index", "0 0"),
        ("/[]/.test('a') + ' ' + /[^]/.test('\\n')", "false true"),
        // An octal escape starting with 4 to 7 takes two digits, so `\477-z` is `'` and the range `7-z`.
        ("show(/[\\477-z]+/.exec(\"'8y\")) + ' ' + new RegExp('[\\\\477-z]').test('8') + ' ' + /[\\377]/.test('\\xff')", "'8y true true"),
    ];
    for (src, want) in cases {
        shows(src, want);
    }
}

#[test]
fn a_realm_made_under_low_limits_has_every_built_in() {
    let low = Limits { fuel: 0, heap_bytes: 1, ..Limits::default() };
    let mut realm = Realm::with_options(Options::default(), low.clone());
    assert_eq!(realm.limits(), &low);
    realm.set_limits(Limits::default());
    assert_eq!(
        run_in(&mut realm, "typeof String + typeof Number + typeof RegExp + typeof Date + typeof /a/.exec + typeof new Date(0).getTime"),
        JsValue::String("function".repeat(6))
    );
}

#[test]
fn regexp_objects_follow_sections_15_10_4_to_15_10_7() {
    truth("var r = /a/g; r.lastIndex === 0 && r.global && !r.ignoreCase && !r.multiline && r.source === 'a'");
    truth(
        "var r = /a/g; r.exec('aa'); var i = r.lastIndex; r.exec('aa'); var j = r.lastIndex; r.exec('aa'); i === 1 && j === 2 && r.lastIndex === 0",
    );
    truth("var r = /a/; r.lastIndex = 5; r.test('a') && r.lastIndex === 5");
    truth("var r = new RegExp('a/b', 'im'); r.source === 'a\\\\/b' && r.ignoreCase && r.multiline && String(r) === '/a\\\\/b/im'");
    truth("String(new RegExp()) === '/(?:)/' && new RegExp('').test('x')");
    truth("var r = new RegExp('[/]'); String(r) === '/[/]/' && eval(String(r)).test('/') && eval(String(new RegExp('a/'))).source === 'a\\\\/'");
    truth("var r = /x/g; RegExp(r) === r && new RegExp(r) !== r && new RegExp(r).global");
    truth("var m = /(a)|(b)/.exec('b'); m.length === 3 && m[1] === undefined && m[2] === 'b' && m.index === 0 && m.input === 'b'");
    truth("Object.prototype.toString.call(RegExp.prototype) === '[object RegExp]' && Object.prototype.toString.call(/x/) === '[object RegExp]'");
    truth("var p = RegExp.prototype; p.hasOwnProperty('global') && p.hasOwnProperty('lastIndex') && p.source === '(?:)' && p.global === false");
    for src in ["new RegExp('(')", "new RegExp('a', 'gg')", "new RegExp('a', 'x')", "new RegExp('a**')", "new RegExp('[b-a]')"] {
        assert!(thrown(src).starts_with("SyntaxError"), "{src}");
    }
    assert!(thrown("new RegExp(/a/, 'g')").starts_with("TypeError"));
    assert!(thrown("RegExp.prototype.exec.call({}, 'a')").starts_with("TypeError"));
}

#[test]
fn a_catastrophic_regular_expression_ends_by_fuel() {
    let limits = Limits { fuel: 2_000_000, ..Limits::default() };
    let src = "var s = ''; for (var i = 0; i < 40; i++) s += 'a'; /(a+)+$/.test(s + 'b')";
    let e = fail_with(Realm::with_options(Options::default(), limits.clone()), src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{e:?}");
    // The interrupt flag stops it too.
    let realm = Realm::with_options(Options::default(), Limits { fuel: u64::MAX, ..Limits::default() });
    realm.interrupt().set();
    let e = fail_with(realm, src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Interrupted), "{e:?}");
}

#[test]
fn trim_and_date_parsing_take_fuel_for_the_units_they_scan() {
    let limits = Limits { fuel: 40_000, ..Limits::default() };
    let build = "var s = ' '; for (var i = 0; i < 20; i++) s = s + s; s.length";
    assert!(Realm::with_options(Options::default(), limits.clone()).run(build).is_ok());
    for scan in ["s.trim()", "Date.parse(s)", "new Date(s)"] {
        let e = fail_with(Realm::with_options(Options::default(), limits.clone()), &format!("{build}; {scan}"));
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{scan}: {e:?}");
    }
}

#[test]
fn global_replace_counts_its_matches_against_the_heap() {
    let src = "var s = 'ab'; for (var i = 0; i < 17; i++) s = s + s; s.replace(/./g, '').length";
    let e = fail_with(Realm::with_options(Options::default(), Limits { heap_bytes: 4 << 20, ..Limits::default() }), src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    assert_eq!(run(src), JsValue::Number(0.0));
}

#[test]
fn replace_holds_its_pieces_and_checks_the_result_as_it_grows() {
    // Each `$&` gives a piece for every match: the pieces count against the heap, so the run ends
    // with the heap limit long before the 4,194,304 pieces would take the fuel.
    let limits = Limits { heap_bytes: 4 << 20, fuel: 500_000, ..Limits::default() };
    let src = "var s = 'a'; for (var i = 0; i < 12; i++) s += s; var t = '$&'; for (var i = 0; i < 10; i++) t += t; s.replace(/a/g, t).length";
    let e = fail_with(Realm::with_options(Options::default(), limits), src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    // A long literal pattern is not copied for each match, and the result's length is checked as
    // the pieces are added, so the run ends with the heap limit before the fuel for 16,384
    // copies of the pattern is spent.
    let limits = Limits { heap_bytes: 4 << 20, fuel: 1_000_000, ..Limits::default() };
    let src = "var s = 'a'; for (var i = 0; i < 14; i++) s += s; var t = 'x'; for (var i = 0; i < 11; i++) t += t; s.replace(/a/g, t).length";
    let e = fail_with(Realm::with_options(Options::default(), limits), src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    // The parts of the pattern give the same result as before.
    shows("'abcabc'.replace(/(b)(c)/g, '[$$|$&|$`|$\\'|$2$1|$3|$0|$10]')", "a[$|bc|a|abc|cb|$3|$0|b0]a[$|bc|abca||cb|$3|$0|b0]");
}

#[test]
fn flattening_a_rope_counts_against_the_heap() {
    // Three ropes of about 524,288 units, each 1 MiB once flat, with a 3 MiB heap.
    let build = "var s = 'x'; for (var i = 0; i < 19; i++) s = s + s; var t = s + 'y', u = s + 'z';";
    let limits = Limits { heap_bytes: 3 << 20, ..Limits::default() };
    let built = Realm::with_options(Options::default(), limits.clone()).run(&format!("{build} s.length + t.length + u.length"));
    assert_eq!(built, Ok(JsValue::Number(1_572_866.0)));
    for read in [
        "charAt(0)",
        "charCodeAt(0)",
        "indexOf('q')",
        "lastIndexOf('q')",
        "localeCompare('a')",
        "slice(0, 1)",
        "substring(0, 1)",
        "substr(0, 1)",
        "trim()",
        "match(/q/)",
        "search(/q/)",
        "replace(/q/, '')",
        "replace('q', '')",
    ] {
        let src = format!("{build} s.{read}; t.{read}; u.{read}; 0");
        let e = fail_with(Realm::with_options(Options::default(), limits.clone()), &src);
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{read}: {e:?}");
    }
    for call in ["/q/.exec(X)", "/q/.test(X)", "Date.parse(X)", "new RegExp(X)"] {
        let src = format!("{build} {}; {}; {}; 0", call.replace('X', "s"), call.replace('X', "t"), call.replace('X', "u"));
        let e = fail_with(Realm::with_options(Options::default(), limits.clone()), &src);
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{call}: {e:?}");
    }
}

#[test]
fn operators_and_builtins_count_a_flattened_rope_against_the_heap() {
    // Three ropes of about 524,288 units, each 1 MiB once flat, with a 3 MiB heap.
    let build = "var s = 'x'; for (var i = 0; i < 19; i++) s = s + s; var t = s + 'y', u = s + 'z';";
    let limits = Limits { heap_bytes: 3 << 20, ..Limits::default() };
    let not_stopped: Vec<_> =
        ["X < 'a'", "X == 1", "+X", "X[0]", "({})[{ toString: function () { return X; } }]", "JSON.stringify(0, null, X)", "[1].join(X)"]
            .into_iter()
            .filter(|read| {
                let src = format!("{build} {}; {}; {}; 0", read.replace('X', "s"), read.replace('X', "t"), read.replace('X', "u"));
                let result = Realm::with_options(Options::default(), limits.clone()).run(&src);
                !matches!(result, Err(ref e) if e.kind == RunErrorKind::Limit(LimitKind::Heap))
            })
            .collect();
    assert!(not_stopped.is_empty(), "not stopped by the heap limit: {not_stopped:?}");
}

#[test]
fn a_flattened_temporary_stays_counted_while_the_next_copy_is_made() {
    // Ropes of 262,145 units, each about half the heap once flat: one fits, two don't. Each is
    // made by a function, so nothing but the native code holds it while it is flattened.
    let build = "var s = 'x'; for (var i = 0; i < 18; i++) s = s + s; function f(c) { return s + c; }";
    let limits = Limits { heap_bytes: 4 * 262_146, ..Limits::default() };
    let run = |read: &str| Realm::with_options(Options::default(), limits.clone()).run(&format!("{build} {read}; 0"));
    assert!(run("f('a')[0]").is_ok(), "one flattened copy fits");
    let not_stopped: Vec<_> = [
        "({ valueOf: function () { return f('a'); } }) < ({ valueOf: function () { return f('b'); } })",
        "JSON.stringify({ toJSON: function () { return f('a'); } })",
        "[1, 2].join({ toString: function () { return f('a'); } })",
        "[{ toString: function () { return f('a'); } }].join('')",
    ]
    .into_iter()
    .filter(|read| !matches!(run(read), Err(ref e) if e.kind == RunErrorKind::Limit(LimitKind::Heap)))
    .collect();
    assert!(not_stopped.is_empty(), "not stopped by the heap limit: {not_stopped:?}");
}

#[test]
fn a_converted_rope_counts_what_it_holds_while_its_copy_is_made() {
    // `mk` returns a rope whose left half is a flat string of about half the heap, held by
    // nothing else. Its flattened copy is the other half, so the two don't fit together.
    let build = "var s = 'x'; for (var i = 0; i < 18; i++) s = s + s; function f(c) { return s + c; } \
                 function mk() { var a = f('a'); a[0]; return a + 'b'; }";
    let limits = Limits { heap_bytes: 4 * 262_146, ..Limits::default() };
    let not_stopped: Vec<_> = [
        "+({ valueOf: mk })",
        "({})[{ toString: mk }]",
        "({ valueOf: mk }) == 1",
        "++({ get x() { return mk(); } }).x",
        "JSON.stringify(0, null, (function () { var o = new String(''); o.toString = mk; return o; })())",
    ]
    .into_iter()
    .filter(|read| {
        let result = Realm::with_options(Options::default(), limits.clone()).run(&format!("{build} {read}; 0"));
        !matches!(result, Err(ref e) if e.kind == RunErrorKind::Limit(LimitKind::Heap))
    })
    .collect();
    assert!(not_stopped.is_empty(), "not stopped by the heap limit: {not_stopped:?}");
}

#[test]
fn a_string_object_counts_its_flattened_rope_against_the_heap() {
    // The same three ropes, each wrapped in a String object and read by index.
    let build = "var s = 'x'; for (var i = 0; i < 19; i++) s = s + s; var t = s + 'y', u = s + 'z'; \
                 String.prototype.first = function () { return this[0]; };";
    let limits = Limits { heap_bytes: 3 << 20, ..Limits::default() };
    // Wrapping alone reads no units, so it makes no copy.
    let lengths =
        Realm::with_options(Options::default(), limits.clone()).run(&format!("{build} Object(s).length + new String(t).length + Object(u).length"));
    assert_eq!(lengths, Ok(JsValue::Number(1_572_866.0)));
    // Two copies fit. The third read finds no room for its copy and reads the unit along the
    // rope, so the live heap stays within the limit.
    for (read, expected) in [
        ("Object(X)[0]", "xxx"),
        ("new String(X)[0]", "xxx"),
        ("0 in Object(X)", "truetruetrue"),
        ("Object(X).hasOwnProperty(0)", "truetruetrue"),
        ("X.first()", "xxx"),
        ("Object(X)[524288]", "undefinedyz"),
    ] {
        let src = format!("{build} '' + ({}) + ({}) + ({})", read.replace('X', "s"), read.replace('X', "t"), read.replace('X', "u"));
        let mut realm = Realm::with_options(Options::default(), limits.clone());
        assert_eq!(realm.run(&src), Ok(JsValue::String(expected.into())), "{read}");
        let interp = realm.interp();
        interp.collect_garbage();
        assert!(interp.heap.bytes <= limits.heap_bytes, "{read}: {} bytes live", interp.heap.bytes);
    }
    let wrapped =
        Realm::with_options(Options::default(), Limits::default()).run(&format!("{build} Object(t)[524288] + new String(u)[524288] + t.first()"));
    assert_eq!(wrapped, Ok(JsValue::String("yzx".into())));
}

#[test]
fn a_string_object_takes_fuel_for_the_rope_it_flattens() {
    let limits = Limits { fuel: 40_000, ..Limits::default() };
    let build = "var s = ' '; for (var i = 0; i < 20; i++) s = s + s; Object(s).length";
    assert!(Realm::with_options(Options::default(), limits.clone()).run(build).is_ok());
    for read in ["Object(s)[0]", "0 in new String(s)"] {
        let e = fail_with(Realm::with_options(Options::default(), limits.clone()), &format!("{build}; {read}"));
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{read}: {e:?}");
    }
    // A run that ends with an error right after the read leaves the next run its whole fuel.
    let mut realm = Realm::with_options(Options::default(), Limits { fuel: 100_000, ..Limits::default() });
    let defined = realm.run("var s = ' '; for (var i = 0; i < 20; i++) s = s + s; Object.defineProperty(new String(s), 0, { value: 'a' })");
    assert!(matches!(defined, Err(ref e) if e.kind == RunErrorKind::Thrown), "{defined:?}");
    assert!(realm.run("for (var j = 0; j < 5000; j++) {} j").is_ok());
}

#[test]
fn a_call_keeps_its_operands_while_a_string_receiver_is_boxed() {
    // `g` leaves a garbage string, so the heap is close to its limit when the rope receiver of a
    // non-strict function is boxed. The callee and the argument object are off the stack by
    // then, and must survive anything boxing the receiver does.
    let build = "var s = 'x'; for (var i = 0; i < 17; i++) s = s + s; function f(c) { return s + c; } \
                 function mk() { var a = f('a'); a[0]; return a + 'b'; } 0";
    let mut realm = Realm::with_options(Options::default(), Limits::default());
    assert_eq!(realm.run(build), Ok(JsValue::Number(0.0)));
    let interp = realm.interp();
    interp.collect_garbage();
    interp.limits.heap_bytes = interp.heap.bytes + 2 * crate::heap::string_bytes(131_073) + (64 << 10);
    interp.reset_gc_threshold();
    let src = "(function () { var g = f('g'); g[0]; })(); (function (o) { return o.x; }).call(mk(), { x: 1 })";
    assert_eq!(realm.run(src), Ok(JsValue::Number(1.0)));
}

#[test]
fn compiling_a_pattern_counts_against_the_heap() {
    let src = "var s = '[\\\\s\\\\S]a*'; for (var i = 0; i < 14; i++) s = s + s; new RegExp(s).source.length";
    let e = fail_with(Realm::with_options(Options::default(), Limits { heap_bytes: 2 << 20, ..Limits::default() }), src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    assert_eq!(run(src), JsValue::Number(131_072.0));
    // The compiler stops while it reads the pattern, before the code it builds passes the room.
    let pattern = "[\\s\\S]a*".repeat(1000);
    assert!(stopped_by(&compile_within(&pattern, 400 * (ITEM_BYTES + INST_BYTES), u64::MAX).0, LimitKind::Heap));
    assert!(compile_within(&pattern, usize::MAX, u64::MAX).0.is_ok());
    // A literal pattern takes what its instructions hold, not the most a unit can take.
    assert!(compile_within(&"a".repeat(10_000), 20_000 * (ITEM_BYTES + INST_BYTES), u64::MAX).0.is_ok());
    // A class's ranges count while they are read, even when they collapse into one range.
    assert!(stopped_by(&compile_within(&format!("[{}]", "a".repeat(10_000)), 1000, u64::MAX).0, LimitKind::Heap));
    // So do the ranges of class escapes, and each atom and range read takes fuel.
    let escapes = format!("[{}]", "\\w".repeat(10_000));
    assert!(stopped_by(&compile_within(&escapes, 64 << 10, u64::MAX).0, LimitKind::Heap));
    assert!(stopped_by(&compile_within(&escapes, usize::MAX, 20_000).0, LimitKind::Fuel));
    // The lists count with the room they reserve as they double: 4097 literal units hold room for
    // 8192 items, and the flattened code after the last unit adds 4098 instructions.
    let literal = "a".repeat(4097);
    let (compiled, bytes, _) = compile_within(&literal, usize::MAX, u64::MAX);
    assert!(compiled.is_ok());
    assert!(bytes >= 8192 * ITEM_BYTES + 4098 * INST_BYTES, "{bytes}");
    assert!(stopped_by(&compile_within(&literal, 8192 * ITEM_BYTES + 4000 * INST_BYTES, u64::MAX).0, LimitKind::Heap));
}

#[test]
fn compiling_a_pattern_takes_fuel_for_each_scan_of_it() {
    // The scan for capturing groups takes a unit of fuel for each unit of the pattern, and so does
    // the compile itself.
    let literal = "a".repeat(100_000);
    let (compiled, _, fuel) = compile_within(&literal, usize::MAX, u64::MAX);
    assert!(compiled.is_ok());
    assert!(fuel >= 200_000, "{fuel}");
    assert!(stopped_by(&compile_within(&literal, usize::MAX, 150_000).0, LimitKind::Fuel));
    // The syntax check before compiling takes a unit for each unit too.
    let mut realm = Realm::new();
    let interp = realm.interp();
    let (source, flags) = (crate::value::Str::from(literal.as_str()), crate::value::Str::from(""));
    interp.fuel = 1 << 40;
    assert!(crate::regexp_builtins::new_regexp(interp, &source, &flags).is_ok());
    let used = (1u64 << 40) - interp.fuel;
    assert!(used >= 300_000, "{used}");
    interp.fuel = 250_000;
    assert!(matches!(crate::regexp_builtins::new_regexp(interp, &source, &flags), Err(crate::interp::Abrupt::Limit(LimitKind::Fuel))));
}

#[test]
fn held_bytes_leave_the_heap_total_when_their_holder_is_dropped() {
    let mut realm = Realm::new();
    let interp = realm.interp();
    interp.collect_for_room();
    let live = interp.heap.bytes;
    interp.limits.heap_bytes = live + 10_000;
    let mut held = crate::interp::HeldBytes::new(interp);
    assert!(held.add(interp, 6000).is_ok());
    assert_eq!(interp.heap.bytes, live + 6000);
    assert!(matches!(crate::interp::HeldBytes::new(interp).add(interp, 6000), Err(crate::interp::Abrupt::Limit(LimitKind::Heap))));
    drop(held);
    // The next check gives the bytes back without a collection.
    assert!(interp.reserve_heap(0).is_ok());
    assert_eq!(interp.heap.bytes, live);
    let mut again = crate::interp::HeldBytes::new(interp);
    assert!(again.add(interp, 6000).is_ok());
    assert_eq!(interp.heap.bytes, live + 6000);
}

/// Compiles `pattern` with no flags through a charge that allows `heap` bytes and `fuel` units,
/// and returns the result with the bytes and fuel the compiler asked for.
fn compile_within(pattern: &str, heap: usize, fuel: u64) -> (Result<crate::regexp_vm::Regex, crate::regexp_vm::CompileFail>, usize, u64) {
    use crate::regexp_vm::Cost;
    let units: Vec<u16> = pattern.encode_utf16().collect();
    let flags = crate::regexp_vm::parse_flags(&[]).unwrap();
    let (mut bytes, mut used) = (0usize, 0u64);
    let result = crate::regexp_vm::compile(&units, flags, 200, &mut |cost| {
        match cost {
            Cost::Heap(n) => bytes = bytes.saturating_add(n),
            Cost::Fuel(n) => used = used.saturating_add(n),
        }
        if bytes > heap {
            return Err(crate::interp::Abrupt::Limit(LimitKind::Heap));
        }
        if used > fuel {
            return Err(crate::interp::Abrupt::Limit(LimitKind::Fuel));
        }
        Ok(())
    });
    (result, bytes, used)
}

/// Whether compiling stopped with the limit `kind`.
fn stopped_by(result: &Result<crate::regexp_vm::Regex, crate::regexp_vm::CompileFail>, kind: LimitKind) -> bool {
    matches!(result, Err(crate::regexp_vm::CompileFail::Abrupt(crate::interp::Abrupt::Limit(k))) if *k == kind)
}

#[test]
fn compiling_a_pattern_collects_garbage_when_the_heap_is_full() {
    let mut realm = Realm::with_options(Options::default(), Limits { heap_bytes: 256 << 10, ..Limits::default() });
    let src = "var n = 0; for (var i = 0; i < 5000; i++) { var junk = 'x' + i + 'y' + i; new RegExp('(a|b)c[d-f]'); n++; } n";
    assert_eq!(run_in(&mut realm, src), JsValue::Number(5000.0));
}

#[test]
fn a_constructed_source_counts_against_the_heap_before_it_is_built() {
    let mut realm = Realm::new();
    let interp = realm.interp();
    interp.collect_for_room();
    let live = interp.heap.bytes;
    let pattern = vec![0x2028u16; 10_000];
    interp.limits.heap_bytes = live + 50_000;
    assert!(matches!(crate::regexp_builtins::source_text(interp, &pattern), Err(crate::interp::Abrupt::Limit(LimitKind::Heap))));
    interp.limits.heap_bytes = live + (1 << 20);
    let Ok(source) = crate::regexp_builtins::source_text(interp, &pattern) else { panic!("the source fits") };
    assert_eq!(source.flat().len(), 60_000);
    assert_eq!(string("String(new RegExp('a/\\\\/[/]\\n\\r\\u2028\\u2029'))"), "/a\\/\\/[/]\\n\\r\\u2028\\u2029/");
    // A backslash before a line terminator reads back as the terminator's escape.
    let escaped = "var r = new RegExp(String.fromCharCode(92, 10, 124, 92, 0x2028)); var back = eval(String(r));";
    assert_eq!(
        string(&format!("{escaped} String(r) + ' ' + r.test('\\n') + back.test('\\n') + back.test('\\u2028') + back.test('x')")),
        "/\\n|\\u2028/ truetruetruefalse"
    );
}

#[test]
fn the_matcher_state_counts_against_the_heap() {
    let mut realm = Realm::with_options(Options::default(), Limits { heap_bytes: 1 << 20, ..Limits::default() });
    let Ok(re) = compile_within(&"(a)".repeat(1000), usize::MAX, u64::MAX).0 else { panic!("the pattern compiles") };
    let interp = realm.interp();
    interp.collect_for_room();
    let live = interp.heap.bytes;
    interp.limits.heap_bytes = live + 1000;
    assert!(matches!(crate::regexp_vm::Matcher::new(&re, interp), Err(crate::interp::Abrupt::Limit(LimitKind::Heap))));
    interp.limits.heap_bytes = 1 << 20;
    assert!(crate::regexp_vm::Matcher::new(&re, interp).is_ok());
    // The stack takes only the room left: with room for the state and no more, backtracking ends
    // with the heap limit.
    let Ok(re) = compile_within("a*b", usize::MAX, u64::MAX).0 else { panic!("the pattern compiles") };
    let input: Vec<u16> = "aaaaaaaaaa".encode_utf16().collect();
    interp.limits.heap_bytes = live + 64;
    let Ok(mut m) = crate::regexp_vm::Matcher::new(&re, interp) else { panic!("the state fits") };
    assert!(matches!(m.search(interp, &input, 0), Err(crate::interp::Abrupt::Limit(LimitKind::Heap))));
    drop(m);
    interp.limits.heap_bytes = 1 << 20;
    let Ok(mut m) = crate::regexp_vm::Matcher::new(&re, interp) else { panic!("the state fits") };
    assert!(matches!(m.search(interp, &input, 0), Ok(false)));
    // What the matcher holds is on the heap while it lives, so other allocations cannot use the
    // same room, and a collection after it is dropped takes it off again.
    let held = m.held_bytes();
    assert!(held > 0);
    assert!(interp.heap.bytes >= live + held, "{} < {live} + {held}", interp.heap.bytes);
    interp.collect_for_room();
    assert!(interp.heap.bytes >= live + held, "{} < {live} + {held}", interp.heap.bytes);
    drop(m);
    interp.collect_for_room();
    assert_eq!(interp.heap.bytes, live);
}

#[test]
fn a_backreference_takes_fuel_for_the_units_it_compares() {
    let mut realm = Realm::with_options(Options::default(), Limits::default());
    let interp = realm.interp();
    let mut fuel_for = |pattern: &str, input: &str, want: bool| {
        let input: Vec<u16> = input.encode_utf16().collect();
        let Ok(re) = compile_within(pattern, usize::MAX, u64::MAX).0 else { panic!("the pattern compiles") };
        let Ok(mut m) = crate::regexp_vm::Matcher::new(&re, interp) else { panic!("the state fits") };
        interp.fuel = 1 << 40;
        assert!(matches!(m.match_at(interp, &input, 0), Ok(hit) if hit == want), "{pattern}");
        (1u64 << 40) - interp.fuel
    };
    // The same 20,000 iterations, each running a one-unit backreference instead of a character:
    // each comparison takes a step besides the instruction's own.
    let a = "a".repeat(20_001);
    let (by_char, by_backref) = (fuel_for("(a)(?:a)*$", &a, true), fuel_for("(a)(?:\\1)*$", &a, true));
    assert!(by_backref >= by_char + 20_000, "{by_backref} < {by_char} + 20000");
    // A long comparison takes a step for each 16 units it compares.
    let long = fuel_for("^(a{10000})\\1", &a, true);
    let plain = fuel_for("^(a{10000})", &a, true);
    assert!(long >= plain + 625, "{long} < {plain} + 625");
    // One that fails on its first unit takes a step for those 16 units only.
    let ab = format!("{}{}", "a".repeat(10_000), "b".repeat(10_000));
    let (by_backref, by_char) = (fuel_for("^(a{10000})\\1", &ab, false), fuel_for("^(a{10000})c", &ab, false));
    assert!(by_backref <= by_char + 2, "{by_backref} > {by_char} + 2");
}

#[test]
fn clearing_matcher_state_takes_fuel() {
    let limits = Limits { fuel: 500_000, ..Limits::default() };
    let groups = "var p = ''; for (var i = 0; i < 1000; i++) p += '(x)'; var s = ''; for (var i = 0; i < 2000; i++) s += 'a';";
    // Each iteration of the `*` clears the 1000 groups inside it, though only `a` runs.
    let src = format!("{groups} new RegExp('(?:a|' + p + ')*').test(s)");
    let e = fail_with(Realm::with_options(Options::default(), limits.clone()), &src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{e:?}");
    // Each attempt at a new position clears the state of all 1001 groups.
    let src = format!("{groups} new RegExp(p + 'z').test(s.replace(/a/g, 'b'))");
    let e = fail_with(Realm::with_options(Options::default(), limits), &src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{e:?}");
}

#[test]
fn deeply_nested_groups_give_a_syntax_error_and_never_overflow_the_stack() {
    let deep = |n: usize| format!("{}a{}", "(".repeat(n), ")".repeat(n));
    let src = format!("try {{ new RegExp('{}'); 'compiled' }} catch (e) {{ e.name }}", deep(50_000));
    assert_eq!(run(&src), JsValue::String("SyntaxError".into()));
    // With the depth limit raised, the same nesting compiles and matches on a small stack: the
    // compiler and the matcher keep their state on the heap.
    let handle = std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(move || {
            let options = Options { max_depth: 60_000, ..Options::default() };
            let mut realm = Realm::with_options(options, Limits::default());
            let src = format!("var m = new RegExp('{}').exec('xa'); m.length + ':' + m[50000]", deep(50_000));
            realm.run(&src)
        })
        .unwrap();
    assert_eq!(handle.join().unwrap(), Ok(JsValue::String("50001:a".into())));
}

#[test]
fn the_matcher_stack_counts_against_the_heap() {
    let limits = Limits { heap_bytes: 4 << 20, ..Limits::default() };
    let src = "var s = 'ab'; for (var i = 0; i < 18; i++) s = s + s; /(?:a|b)*c/.test(s)";
    let e = fail_with(Realm::with_options(Options::default(), limits), src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
}

proptest! {
    #[test]
    fn arbitrary_patterns_and_inputs_never_panic(pattern in "[ab()|*+?{}\\[\\]^$.\\\\dws0-9,:=!-]{0,24}", input in "[ab\\n0-9 ]{0,24}", flags in "[gim]{0,2}") {
        let mut realm = Realm::with_options(Options::default(), Limits { fuel: 200_000, ..Limits::default() });
        let quote = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
        let src = format!(
            "try {{ var r = new RegExp('{}', '{}'); var s = '{}'; [r.exec(s), s.replace(r, '$1'), s.split(r), s.match(r), s.search(r)].length }} catch (e) {{ e.name }}",
            quote(&pattern), flags, quote(&input)
        );
        match realm.run(&src) {
            Ok(_) => {}
            Err(e) => prop_assert!(matches!(e.kind, RunErrorKind::Limit(_)), "{src}: {e:?}"),
        }
    }
}

// ---- Date ----

proptest! {
    #[test]
    fn arbitrary_date_strings_never_panic(text in "[0-9TZ:.+ ,()a-zA-Z-]{0,32}") {
        let mut realm = Realm::new();
        let src = format!("var t = Date.parse('{text}'); new Date('{text}').toString(); t");
        prop_assert!(realm.run(&src).is_ok(), "{src}");
    }
}

fn date_realm(now: f64, offset_minutes: f64) -> Realm {
    let mut realm = Realm::new();
    realm.set_clock(move || now);
    realm.set_time_zone(move |_| offset_minutes * 60_000.0);
    realm
}

#[test]
fn dates_read_the_host_clock_and_time_zone() {
    // 2026-11-07T12:34:56.789Z.
    let now = 1_794_054_896_789.0;
    let mut utc = date_realm(now, 0.0);
    assert_eq!(run_in(&mut utc, "Date.now()"), JsValue::Number(now));
    assert_eq!(run_in(&mut utc, "new Date().getTime()"), JsValue::Number(now));
    assert_eq!(run_in(&mut utc, "new Date().toISOString()"), JsValue::String("2026-11-07T12:34:56.789Z".into()));
    assert_eq!(run_in(&mut utc, "String(new Date())"), JsValue::String("Sat Nov 07 2026 12:34:56 GMT+0000".into()));
    assert_eq!(run_in(&mut utc, "Date()"), JsValue::String("Sat Nov 07 2026 12:34:56 GMT+0000".into()));
    assert_eq!(run_in(&mut utc, "new Date().toUTCString()"), JsValue::String("Sat, 07 Nov 2026 12:34:56 GMT".into()));
    let mut east = date_realm(now, 90.0);
    assert_eq!(run_in(&mut east, "String(new Date())"), JsValue::String("Sat Nov 07 2026 14:04:56 GMT+0130".into()));
    assert_eq!(
        run_in(&mut east, "var d = new Date(); d.getHours() + ',' + d.getUTCHours() + ',' + d.getMinutes() + ',' + d.getTimezoneOffset()"),
        JsValue::String("14,12,4,-90".into())
    );
    assert_eq!(run_in(&mut east, "new Date(2026, 10, 7, 14, 4, 56, 789).getTime()"), JsValue::Number(now));
    // An offset of a day or more, or one that is not finite, counts as 0.
    for offset in [f64::MAX, 86_400_000.0, f64::NAN] {
        let mut odd = Realm::new();
        odd.set_clock(move || now);
        odd.set_time_zone(move |_| offset);
        assert_eq!(run_in(&mut odd, "var d = new Date(); d.getHours() + ',' + d.getTimezoneOffset()"), JsValue::String("12,0".into()), "{offset}");
    }
    let mut west = date_realm(now, -300.0);
    assert_eq!(run_in(&mut west, "new Date(2026, 0, 31).toDateString()"), JsValue::String("Sat Jan 31 2026".into()));
}

#[test]
fn dates_round_trip_through_their_strings() {
    let mut realm = date_realm(1_794_054_896_789.0, -420.0);
    for src in [
        "var d = new Date(); Date.parse(d.toString()) === Math_floor(d.getTime())",
        "var d = new Date(); Date.parse(d.toUTCString()) === Math_floor(d.getTime())",
        "var d = new Date(); Date.parse(d.toISOString()) === d.getTime()",
        "var d = new Date(-1e14); Date.parse(d.toISOString()) === d.getTime() && Date.parse(d.toString()) === d.getTime()",
        "new Date(new Date(5).toISOString()).getTime() === 5",
        "Date.UTC(2000, 1, 29) === Date.parse('2000-02-29') && Date.UTC(99, 0) === Date.UTC(1999, 0)",
        "Date.UTC(2000) === Date.UTC(2000, 0) && Date.UTC(1970) === 0 && isNaN_(Date.UTC())",
        "isNaN_(Date.parse('nonsense')) && isNaN_(new Date(8.64e15 + 1).getTime()) && new Date(8.64e15).getTime() === 8.64e15",
        "var d = new Date(2000, 0, 31); d.setMonth(1); d.getMonth() === 2 && d.getDate() === 2",
        "var d = new Date(NaN); d.setFullYear(2001); d.getFullYear() === 2001 && d.getMonth() === 0",
        "var d = new Date(0); d.setUTCHours(25, 61, 61, 1001); d.toISOString() === '1970-01-02T02:02:02.001Z'",
        "var d = new Date(0); d.setMinutes(); isNaN_(d.getTime())",
        "var d = new Date(2000, 0, 1); d.setYear(99); d.getFullYear() === 1999 && d.getYear() === 99",
        "Date.prototype.toGMTString === Date.prototype.toUTCString && String(new Date(NaN)) === 'Invalid Date'",
        "new Date(0).toJSON() === '1970-01-01T00:00:00.000Z' && new Date(NaN).toJSON() === null",
        "Object.prototype.toString.call(Date.prototype) === '[object Date]' && isNaN_(Date.prototype.getTime())",
        "var d = new Date(0); d + d === d.toString() + d.toString() && d - d === 0 && d == d.toString()",
        "isNaN_(Date.parse('1970-00-01')) && isNaN_(Date.parse('1970-13-01')) && isNaN_(Date.parse('1970-02-30'))",
        "Date.parse('thu, 01 JAN 1970 00:00:00 gmt') === 0 && Date.parse('jan 2 1970 utc+01:00') === 82800000",
        "isNaN_(Date.parse('Jan 1 1970 1:2:3:4 GMT')) && isNaN_(Date.parse('Jan 1 1970 GMT+01000')) && isNaN_(Date.parse('Jan 1 1970 x'))",
    ] {
        let full = format!("function Math_floor(t) {{ return t - t % 1000; }} function isNaN_(n) {{ return n !== n; }} {src}");
        assert_eq!(run_in(&mut realm, &full), JsValue::Bool(true), "{src}");
    }
    let e = fail_with(date_realm(0.0, 0.0), "new Date(NaN).toISOString()");
    assert!(e.message.starts_with("RangeError"), "{e:?}");
    let e = fail_with(date_realm(0.0, 0.0), "Date.prototype.getTime.call({})");
    assert!(e.message.starts_with("TypeError"), "{e:?}");
}
