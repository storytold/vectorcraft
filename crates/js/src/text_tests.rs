//! Tests of `String`, `Number`, `RegExp` and `Date` run through a realm.

use proptest::prelude::*;

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
    ];
    for (src, want) in cases {
        shows(src, want);
    }
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
    let build = "var s = ' '; for (var i = 0; i < 20; i++) s = s + s; s.charAt(0)";
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
fn compiling_a_pattern_counts_against_the_heap() {
    let src = "var s = '[\\\\s\\\\S]a*'; for (var i = 0; i < 14; i++) s = s + s; new RegExp(s).source.length";
    let e = fail_with(Realm::with_options(Options::default(), Limits { heap_bytes: 2 << 20, ..Limits::default() }), src);
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    assert_eq!(run(src), JsValue::Number(131_072.0));
    // The compiler stops while it reads the pattern, before the code it builds passes the room.
    let pattern: Vec<u16> = "[\\s\\S]a*".repeat(1000).encode_utf16().collect();
    let flags = crate::regexp_vm::parse_flags(&[]).unwrap();
    let room = crate::regexp_vm::compile_bytes(400, 0, 0);
    let result = crate::regexp_vm::compile(&pattern, flags, 200, room, &mut |_| Ok(()));
    assert!(matches!(result, Err(crate::regexp_vm::CompileFail::Abrupt(crate::interp::Abrupt::Limit(LimitKind::Heap)))));
    assert!(crate::regexp_vm::compile(&pattern, flags, 200, usize::MAX, &mut |_| Ok(())).is_ok());
    // A literal pattern takes what its instructions hold, not the most a unit can take.
    let literal: Vec<u16> = "a".repeat(10_000).encode_utf16().collect();
    let room = crate::regexp_vm::compile_bytes(20_000, 0, 0);
    assert!(crate::regexp_vm::compile(&literal, flags, 200, room, &mut |_| Ok(())).is_ok());
    // A class's ranges count while they are read, even when they collapse into one range.
    let class: Vec<u16> = format!("[{}]", "a".repeat(10_000)).encode_utf16().collect();
    let result = crate::regexp_vm::compile(&class, flags, 200, 1000, &mut |_| Ok(()));
    assert!(matches!(result, Err(crate::regexp_vm::CompileFail::Abrupt(crate::interp::Abrupt::Limit(LimitKind::Heap)))));
    // The joining and flattening after the last unit count too.
    let ab: Vec<u16> = "ab".encode_utf16().collect();
    let room = crate::regexp_vm::compile_bytes(1, 0, 0);
    let result = crate::regexp_vm::compile(&ab, flags, 200, room, &mut |_| Ok(()));
    assert!(matches!(result, Err(crate::regexp_vm::CompileFail::Abrupt(crate::interp::Abrupt::Limit(LimitKind::Heap)))));
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
}

#[test]
fn the_matcher_state_counts_against_the_heap() {
    let mut realm = Realm::with_options(Options::default(), Limits { heap_bytes: 1 << 20, ..Limits::default() });
    let pattern: Vec<u16> = "(a)".repeat(1000).encode_utf16().collect();
    let flags = crate::regexp_vm::parse_flags(&[]).unwrap();
    let Ok(re) = crate::regexp_vm::compile(&pattern, flags, 200, usize::MAX, &mut |_| Ok(())) else { panic!("the pattern compiles") };
    let interp = realm.interp();
    interp.collect_for_room();
    let live = interp.heap.bytes;
    interp.limits.heap_bytes = live + 1000;
    assert!(matches!(crate::regexp_vm::Matcher::new(&re, interp), Err(crate::interp::Abrupt::Limit(LimitKind::Heap))));
    interp.limits.heap_bytes = 1 << 20;
    assert!(crate::regexp_vm::Matcher::new(&re, interp).is_ok());
    // The stack takes only the room left: with room for the state and no more, backtracking ends
    // with the heap limit.
    let pattern: Vec<u16> = "a*b".encode_utf16().collect();
    let Ok(re) = crate::regexp_vm::compile(&pattern, flags, 200, usize::MAX, &mut |_| Ok(())) else { panic!("the pattern compiles") };
    let input: Vec<u16> = "aaaaaaaaaa".encode_utf16().collect();
    interp.limits.heap_bytes = live + 64;
    let Ok(mut m) = crate::regexp_vm::Matcher::new(&re, interp) else { panic!("the state fits") };
    assert!(matches!(m.search(interp, &input, 0), Err(crate::interp::Abrupt::Limit(LimitKind::Heap))));
    interp.limits.heap_bytes = 1 << 20;
    let Ok(mut m) = crate::regexp_vm::Matcher::new(&re, interp) else { panic!("the state fits") };
    assert!(matches!(m.search(interp, &input, 0), Ok(false)));
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
    ] {
        let full = format!("function Math_floor(t) {{ return t - t % 1000; }} function isNaN_(n) {{ return n !== n; }} {src}");
        assert_eq!(run_in(&mut realm, &full), JsValue::Bool(true), "{src}");
    }
    let e = fail_with(date_realm(0.0, 0.0), "new Date(NaN).toISOString()");
    assert!(e.message.starts_with("RangeError"), "{e:?}");
    let e = fail_with(date_realm(0.0, 0.0), "Date.prototype.getTime.call({})");
    assert!(e.message.starts_with("TypeError"), "{e:?}");
}
