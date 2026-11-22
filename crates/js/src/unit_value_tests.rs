//! `UnitValue` and the operators in a realm that has it.

use super::*;

fn realm() -> Realm {
    let mut r = Realm::new();
    r.add_unit_values().unwrap();
    r
}

fn number(r: &mut Realm, source: &str) -> f64 {
    match r.run(source) {
        Ok(JsValue::Number(n)) => n,
        other => panic!("{source}: {other:?}"),
    }
}

fn text(r: &mut Realm, source: &str) -> String {
    match r.run(source) {
        Ok(JsValue::String(s)) => s,
        other => panic!("{source}: {other:?}"),
    }
}

fn boolean(r: &mut Realm, source: &str) -> bool {
    match r.run(source) {
        Ok(JsValue::Bool(b)) => b,
        other => panic!("{source}: {other:?}"),
    }
}

/// The name of the error `statement` throws, or `none`.
fn error_name(r: &mut Realm, statement: &str) -> String {
    text(r, &format!("(function () {{ try {{ {statement}; return 'none'; }} catch (e) {{ return e.name; }} }})()"))
}

fn close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() <= 1e-12 * want.abs().max(1.0), "{what}: {got} is not {want}");
}

/// `got` is within a millionth of `want`, which is not 0, however small `want` is.
fn relative(got: f64, want: f64, what: &str) {
    assert!((got / want - 1.0).abs() <= 1e-6, "{what}: {got} is not {want}");
}

/// Each length unit with its length in inches.
const LENGTHS: [(&str, f64); 13] = [
    ("in", 1.0),
    ("ft", 12.0),
    ("yd", 36.0),
    ("mi", 63_360.0),
    ("mm", 1.0 / 25.4),
    ("cm", 1.0 / 2.54),
    ("m", 1.0 / 0.0254),
    ("km", 1000.0 / 0.0254),
    ("pt", 1.0 / 72.0),
    ("pc", 1.0 / 6.0),
    ("tpt", 1.0 / 72.27),
    ("tpc", 12.0 / 72.27),
    ("ci", 12.7872 / 72.0),
];

#[test]
fn every_length_unit_converts_to_every_other_and_back() {
    let mut r = realm();
    for (from, a) in LENGTHS {
        for (to, b) in LENGTHS {
            let there = number(&mut r, &format!("UnitValue(3, '{from}').as('{to}')"));
            close(there, 3.0 * a / b, &format!("3 {from} in {to}"));
            let back = number(&mut r, &format!("UnitValue({there}, '{to}').as('{from}')"));
            close(back, 3.0, &format!("3 {from} through {to} and back"));
        }
    }
}

#[test]
fn the_table_relations_are_exact() {
    let mut r = realm();
    for (source, want) in [
        ("UnitValue(1, 'in').as('pt')", 72.0),
        ("UnitValue(1, 'ft').as('in')", 12.0),
        ("UnitValue(1, 'yd').as('ft')", 3.0),
        ("UnitValue(1, 'mi').as('m')", 1609.344),
        ("UnitValue(1, 'm').as('cm')", 100.0),
        ("UnitValue(1, 'km').as('m')", 1000.0),
        ("UnitValue(10, 'mm').as('cm')", 1.0),
        ("UnitValue(2.54, 'cm').as('in')", 1.0),
        ("UnitValue(1, 'pc').as('pt')", 12.0),
        ("UnitValue(1, 'tpc').as('tpt')", 12.0),
        ("UnitValue(72.27, 'tpt').as('in')", 1.0),
        ("UnitValue(1, 'ci').as('pt')", 12.7872),
        ("UnitValue(1, 'in').as('px')", 72.0),
    ] {
        assert_eq!(number(&mut r, source), want, "{source}");
    }
}

#[test]
fn units_are_named_by_abbreviation_singular_or_plural() {
    let mut r = realm();
    for (name, abbreviation) in [
        ("inch", "in"),
        ("inches", "in"),
        ("feet", "ft"),
        ("millimeter", "mm"),
        ("centimeters", "cm"),
        ("traditional point", "tpt"),
        ("traditional picas", "tpc"),
        ("ciceros", "ci"),
        ("pixel", "px"),
        ("percent", "%"),
    ] {
        assert_eq!(text(&mut r, &format!("UnitValue(1, '{name}').type")), abbreviation, "{name}");
        assert_eq!(text(&mut r, &format!("UnitValue('2 {name}').type")), abbreviation, "2 {name}");
    }
}

#[test]
fn the_constructor_takes_a_number_and_unit_text_or_another_unit_value() {
    let mut r = realm();
    assert_eq!(text(&mut r, "String(new UnitValue(12, 'pt'))"), "12 pt");
    assert_eq!(text(&mut r, "String(UnitValue('12 cm'))"), "12 cm");
    assert_eq!(text(&mut r, "String(UnitValue('  -1.5e1  centimeters '))"), "-15 cm");
    assert_eq!(text(&mut r, "String(UnitValue('10%'))"), "10 %");
    assert_eq!(text(&mut r, "String(UnitValue('12', 'in'))"), "12 in");
    assert!(boolean(&mut r, "UnitValue(1, 'in') instanceof UnitValue"));
    assert_eq!(text(&mut r, "Object.prototype.toString.call(UnitValue(1, 'in'))"), "[object UnitValue]");
    let copy = "var a = UnitValue(5, 'px'); a.baseUnit = UnitValue(1, 'mm'); var b = UnitValue(a); a.value = 1; \
                [a === b, b.value, b.type, b.as('mm')].join()";
    assert_eq!(text(&mut r, copy), "false,5,px,5", "a copy is a new object with the same value, unit and base");
}

#[test]
fn an_unknown_unit_or_text_makes_an_invalid_unit_value() {
    let mut r = realm();
    for source in [
        "UnitValue(12, 'furlong')",
        "UnitValue('12')",
        "UnitValue('pt')",
        "UnitValue('12 pt extra')",
        "UnitValue(12)",
        "UnitValue()",
        "UnitValue(NaN, 'pt')",
        "UnitValue(Infinity, 'pt')",
        "UnitValue(12, 'PT')",
    ] {
        assert_eq!(text(&mut r, &format!("var u = {source}; u.type + ' ' + u.value")), "? 0", "{source}");
        assert_eq!(text(&mut r, &format!("String({source})")), "UnitValue 0.00000", "{source}");
    }
    for statement in [
        "UnitValue(1, 'zz').as('pt')",
        "UnitValue(1, 'zz').convert('pt')",
        "UnitValue(1, 'zz') + 1",
        "UnitValue(1, 'in') + UnitValue(1, 'zz')",
        "UnitValue('bad') == 0",
        "1 > UnitValue('bad')",
        "+UnitValue('bad')",
        "-UnitValue('bad')",
        "~UnitValue('bad')",
        "!UnitValue('bad')",
        "UnitValue('bad') + 'x'",
        "'x' < UnitValue('bad')",
        "UnitValue('bad') == null",
    ] {
        assert_eq!(error_name(&mut r, statement), "RangeError", "{statement}");
    }
}

#[test]
fn as_leaves_the_object_and_convert_changes_it() {
    let mut r = realm();
    assert_eq!(text(&mut r, "var u = UnitValue(1, 'in'); [u.as('pt'), String(u)].join()"), "72,1 in");
    assert_eq!(text(&mut r, "var u = UnitValue(1, 'in'); [u.convert('pt'), String(u)].join()"), "true,72 pt");
    assert_eq!(error_name(&mut r, "UnitValue(1, 'in').as('furlong')"), "Error");
    assert_eq!(text(&mut r, "var u = UnitValue(1, 'in'); try { u.convert('furlong'); } catch (e) {} String(u)"), "1 in");
    assert_eq!(text(&mut r, "var u = UnitValue(1, 'in'); try { u.type = 'furlong'; } catch (e) {} String(u)"), "1 in");
    assert_eq!(text(&mut r, "var u = UnitValue(1, 'in'); u.type = 'points'; u.value = 3; String(u)"), "3 pt");
    assert_eq!(error_name(&mut r, "UnitValue(1, 'in').value = NaN"), "RangeError");
    assert_eq!(error_name(&mut r, "UnitValue.prototype.as.call({}, 'pt')"), "TypeError");
}

#[test]
fn pixels_and_percentages_convert_through_the_base() {
    let mut r = realm();
    assert_eq!(number(&mut r, "UnitValue(72, 'px').as('in')"), 1.0);
    assert_eq!(number(&mut r, "var p = UnitValue(10, 'px'); p.baseUnit = UnitValue(0.75, 'pt'); p.as('pt')"), 7.5);
    close(number(&mut r, "var p = UnitValue(10, 'px'); p.baseUnit = UnitValue(1/96, 'in'); p.as('in')"), 10.0 / 96.0, "10 px at 96 per inch");
    assert_eq!(text(&mut r, "String(UnitValue.baseUnit)"), "1 pt");
    assert_eq!(number(&mut r, "var p = UnitValue(40, '%'); p.baseUnit = UnitValue(10, 'ft'); p.as('ft')"), 4.0);
    assert_eq!(number(&mut r, "var m = UnitValue(10, 'm'); m.baseUnit = UnitValue(1, 'km'); m.as('%')"), 1.0);
    assert_eq!(number(&mut r, "var p = UnitValue(3, 'px'); p.baseUnit = UnitValue(2, 'pt'); p.as('%')"), 300.0);
    assert_eq!(number(&mut r, "var p = UnitValue(50, '%'); p.as('px')"), 0.5);
    let class = "var before = UnitValue(1, 'in'); UnitValue.baseUnit = UnitValue(0.5, 'pt'); \
                 var after = UnitValue(1, 'in'); var r = [before.as('px'), after.as('px'), String(UnitValue.baseUnit.type)]; \
                 UnitValue.baseUnit = null; r.push(UnitValue(1, 'in').as('px')); r.join()";
    assert_eq!(text(&mut r, class), "72,144,pt,72", "the class base applies to objects made after it is set, and null restores it");
    assert_eq!(number(&mut r, "var p = UnitValue(1, 'in'); p.baseUnit = UnitValue(1, 'cm'); p.baseUnit = null; p.as('px')"), 72.0);
    let large = "var p = UnitValue(100, '%'); p.baseUnit = UnitValue(1e308, 'pt'); var q = UnitValue(2, 'px'); \
                 q.baseUnit = UnitValue(1e308, 'pt'); var l = UnitValue(1e308, 'km'); l.baseUnit = UnitValue(1e300, 'pt'); \
                 [p.as('pt'), q.as('in'), l.as('px')].join()";
    let got: Vec<f64> = text(&mut r, large).split(',').map(|s| s.parse().unwrap()).collect();
    close(got[0], 1e308, "100% of a large base");
    close(got[1], 1e308 / 72.0 * 2.0, "2 px of a large base in inches");
    close(got[2], 1e308 / 1e300 * 1000.0 * 5000.0 / 127.0 * 72.0, "a large length in pixels of a large base");
}

#[test]
fn a_base_must_be_a_length_greater_than_zero() {
    let mut r = realm();
    for base in ["UnitValue(0, 'pt')", "UnitValue(-1, 'pt')", "UnitValue(1, 'px')", "UnitValue(1, '%')", "UnitValue(1, 'zz')"] {
        assert_eq!(error_name(&mut r, &format!("UnitValue.baseUnit = {base}")), "RangeError", "{base}");
        assert_eq!(error_name(&mut r, &format!("UnitValue(1, 'px').baseUnit = {base}")), "RangeError", "{base}");
    }
    assert_eq!(error_name(&mut r, "UnitValue.baseUnit = 5"), "RangeError");
    assert_eq!(error_name(&mut r, "UnitValue(1, 'px').baseUnit = '1 pt'"), "RangeError");
    assert_eq!(number(&mut r, "UnitValue(1, 'in').as('px')"), 72.0, "a refused base leaves the class base as it was");
}

#[test]
fn arithmetic_gives_a_new_unit_value_in_the_left_unit() {
    let mut r = realm();
    for (source, want) in [
        ("UnitValue(1, 'm') + UnitValue(10, 'cm')", "1.1 m"),
        ("UnitValue(10, 'cm') + UnitValue(1, 'm')", "110 cm"),
        ("UnitValue(10, 'cm') * 20", "200 cm"),
        ("20 * UnitValue(10, 'cm')", "200 cm"),
        ("10 - UnitValue(3, 'pt')", "7 pt"),
        ("UnitValue(10, 'pt') - 3", "7 pt"),
        ("20 / UnitValue(4, 'pt')", "5 pt"),
        ("UnitValue(20, 'pt') / 4", "5 pt"),
        ("UnitValue(7, 'pt') % 4", "3 pt"),
        ("UnitValue(1, 'in') - UnitValue(36, 'pt')", "0.5 in"),
    ] {
        assert_eq!(text(&mut r, &format!("String({source})")), want, "{source}");
    }
    assert_eq!(text(&mut r, "var u = UnitValue(1, 'pt'); u += 2; String(u)"), "3 pt", "compound assignment uses the operator");
    assert_eq!(text(&mut r, "var u = UnitValue(1, 'in'); var v = u + 1; String(u)"), "1 in", "the operands do not change");
    assert_eq!(text(&mut r, "var u = UnitValue(2, 'px'); u.baseUnit = UnitValue(1, 'mm'); (u * 3).as('mm') + ''"), "6", "the result keeps the base");
    assert_eq!(text(&mut r, "UnitValue(1, 'pt') + 'x'"), "1 ptx", "a string concatenates as in standard JavaScript");
}

#[test]
fn the_left_objects_base_measures_its_pixels_and_percentages() {
    let mut r = realm();
    let pair = "var l = UnitValue(1, 'px'); l.baseUnit = UnitValue(2, 'pt'); var m = UnitValue(1, 'px'); \
                m.baseUnit = UnitValue(4, 'pt'); var p = UnitValue(50, '%'); p.baseUnit = UnitValue(1, 'in');";
    assert_eq!(text(&mut r, &format!("{pair} String(l + UnitValue(1, 'in'))")), "37 px");
    assert_eq!(text(&mut r, &format!("{pair} String(l + m)")), "3 px");
    close(number(&mut r, &format!("{pair} (p + l).value")), 50.0 + 200.0 / 72.0, "2 pt as a percentage of 1 in");
    assert!(boolean(&mut r, &format!("{pair} l < m && m > l && l == UnitValue(2, 'pt')")));
    let large = "var b = UnitValue(0, 'px'); b.baseUnit = UnitValue(1e308, 'pt'); var c = UnitValue(1e300, 'px'); \
                 c.baseUnit = UnitValue(1e10, 'pt'); [(b + UnitValue(1e304, 'km')).value, (b + c).value].join()";
    let got: Vec<f64> = text(&mut r, large).split(',').map(|s| s.parse().unwrap()).collect();
    close(got[0], 1e304 / 1e308 * 1000.0 * 5000.0 / 127.0 * 72.0, "a large length in pixels of a large base");
    close(got[1], 1e300 / 1e308 * 1e10, "large pixels of one large base in pixels of another");
    let small = "var s = UnitValue(0, 'px'); s.baseUnit = UnitValue(1e-300, 'pt'); var t = UnitValue(1e-200, 'px'); \
                 t.baseUnit = UnitValue(1e-200, 'pt'); (s + t).value";
    let units = "var u = UnitValue(0, 'px'); u.baseUnit = UnitValue(1e20, 'pt'); var v = UnitValue(1e308, 'px'); \
                 v.baseUnit = UnitValue(1e10, 'km'); (u + v).value";
    close(number(&mut r, units), 1e298 * 1000.0 * 5000.0 / 127.0 * 72.0, "large pixels of a base in a larger unit");
    let apart = "var w = UnitValue(0, 'px'); w.baseUnit = UnitValue(7e-16, 'km'); var x = UnitValue(1e-20, 'px'); \
                 x.baseUnit = UnitValue(1e308, 'pt'); (w + x).value";
    close(number(&mut r, apart), 1e288 / (1000.0 * 5000.0 / 127.0 * 72.0) / 7e-16, "bases too far apart for their ratio");
    relative(number(&mut r, small), 1e-100, "small pixels of one small base in pixels of another");
    let tiny = "var y = UnitValue(0, 'px'); y.baseUnit = UnitValue(1e-200, 'km'); var z = UnitValue(1e-320, 'px'); (y + z).value";
    relative(number(&mut r, tiny), 1e-320 * 1e200 / (1000.0 * 5000.0 / 127.0 * 72.0), "a tiny value in pixels of a tiny base");
    let tinier = "var y = UnitValue(0, 'px'); y.baseUnit = UnitValue(1e-320, 'pt'); var z = UnitValue(1e-320, 'px'); \
                  z.baseUnit = UnitValue(1e-10, 'pt'); (y + z).value";
    relative(number(&mut r, tinier), 1e-10, "a tiny value of a small base in pixels of a tiny base");
}

#[test]
fn comparisons_use_the_numbers_and_greater_is_the_negation() {
    let mut r = realm();
    for (source, want) in [
        ("UnitValue(98, 'cm') < UnitValue(1, 'm')", true),
        ("UnitValue(98, 'cm') < 1", false),
        ("UnitValue(98, 'cm') == 98", true),
        ("98 == UnitValue(98, 'cm')", true),
        ("UnitValue(1, 'in') == UnitValue(72, 'pt')", true),
        ("UnitValue(1, 'in') != UnitValue(72, 'pt')", false),
        ("UnitValue(1, 'in') === UnitValue(72, 'pt')", false),
        ("var u = UnitValue(1, 'in'); u === u", true),
        ("UnitValue(1, 'in') <= UnitValue(72, 'pt')", true),
        ("UnitValue(1, 'in') >= UnitValue(72, 'pt')", true),
        ("UnitValue(1, 'in') > UnitValue(72, 'pt')", false),
        ("UnitValue(1, 'in') > UnitValue(71, 'pt')", true),
        ("UnitValue(1, 'in') >= UnitValue(73, 'pt')", false),
        ("2 > UnitValue(1, 'pt')", true),
        ("2 >= UnitValue(3, 'pt')", false),
    ] {
        assert_eq!(boolean(&mut r, source), want, "{source}");
    }
    for statement in [
        "UnitValue(1, 'pt') < NaN",
        "UnitValue(1, 'pt') > NaN",
        "NaN == UnitValue(1, 'pt')",
        "UnitValue(1, 'pt') < Infinity",
        "UnitValue(1, 'in') < UnitValue(1, 'zz')",
    ] {
        assert_eq!(error_name(&mut r, statement), "RangeError", "{statement}");
    }
}

#[test]
fn unary_operators_use_the_number() {
    let mut r = realm();
    assert_eq!(text(&mut r, "var u = UnitValue(2.5, 'pt'); [+u, -u, ~u, !u, !UnitValue(0, 'pt'), typeof u].join()"), "2.5,-2.5,-3,true,false,object");
}

#[test]
fn results_that_are_not_finite_and_division_by_zero_throw() {
    let mut r = realm();
    for statement in [
        "UnitValue(1e308, 'pt') * 10",
        "UnitValue(1, 'pt') / 0",
        "1 / UnitValue(0, 'pt')",
        "UnitValue(1, 'pt') % 0",
        "UnitValue(1, 'pt') + NaN",
        "UnitValue(1e308, 'mi').as('pt')",
        "UnitValue(1, 'pt') + UnitValue(1e308, 'mi')",
    ] {
        assert_eq!(error_name(&mut r, statement), "RangeError", "{statement}");
    }
    assert_eq!(number(&mut r, "UnitValue(1e300, 'pt').as('mi') * 0"), 0.0);
    close(number(&mut r, "UnitValue(1e307, 'km').as('mi')"), 1e307 * (1000.0 / 1609.344), "a large value that fits after the division");
}

#[test]
fn a_realm_without_unit_values_keeps_the_standard_operators() {
    let mut r = Realm::new();
    assert_eq!(text(&mut r, "typeof UnitValue"), "undefined");
    let mut r = realm();
    let plain = "var o = {valueOf: function () { return 4; }}; [o + 1, o * 2, o < 5, o == 4].join()";
    assert_eq!(text(&mut r, plain), "5,8,true,true", "other objects use their valueOf");
    let named = "function P() {} P.prototype['+'] = function () { return 'called'; }; String(new P() + 1)";
    assert_eq!(text(&mut r, named), "[object Object]1", "a method named for an operator is not called");
}

#[test]
fn hostile_input_ends_in_an_error_or_a_limit() {
    let mut r = realm();
    let long = "var s = 'pt'; for (var i = 0; i < 17; i++) s += s; ";
    assert_eq!(text(&mut r, &format!("{long} UnitValue(1, s).type")), "?");
    assert_eq!(text(&mut r, &format!("{long} UnitValue('1 ' + s).type")), "?");
    assert_eq!(error_name(&mut r, &format!("{long} UnitValue(1, 'in').as(s)")), "Error");
    let loop_text = "var o = {toString: function () { return String(UnitValue(o)); }}; UnitValue(o)";
    match r.run(loop_text) {
        Err(e) => assert!(matches!(e.kind, RunErrorKind::Limit(_) | RunErrorKind::Thrown), "{e:?}"),
        other => panic!("{other:?}"),
    }
    let value_of = "var o = {valueOf: function () { return UnitValue(1, 'pt') * o; }}; UnitValue(1, 'pt') * o";
    match r.run(value_of) {
        Err(e) => assert!(matches!(e.kind, RunErrorKind::Limit(_) | RunErrorKind::Thrown), "{e:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(text(&mut r, "String(UnitValue(3, 'pt'))"), "3 pt", "the realm runs on");
}

#[test]
fn unit_values_survive_a_collection() {
    let mut r = realm();
    r.run("var keep = UnitValue(2, 'in'); for (var i = 0; i < 1000; i++) UnitValue(i, 'pt');").unwrap();
    r.collect_garbage();
    assert_eq!(text(&mut r, "String(keep + UnitValue(72, 'pt')) + ' ' + String(UnitValue(1, 'pc'))"), "3 in 1 pc");
}

#[test]
fn each_measurement_counts_toward_the_heap() {
    let mut r = realm();
    // The growth from 100 objects to 200, so the script's own code and strings cancel out.
    let mut per_hundred = |make: &str| {
        let mut grown = |count: usize| {
            r.run("var kept = [];").unwrap();
            let before = r.collect_garbage();
            r.run(&format!("for (var i = 0; i < {count}; i++) kept.push({make});")).unwrap();
            r.collect_garbage() - before
        };
        grown(200) - grown(100)
    };
    // The first runs also leave bindings and names the later ones reuse.
    per_hundred("{}");
    let plain = per_hundred("{}");
    let measures = per_hundred("UnitValue(i, 'pt')");
    assert_eq!(measures - plain, 100 * crate::heap::UNIT_VALUE_BYTES);
}

#[test]
fn an_install_that_hits_the_heap_limit_installs_everything_on_the_next_call() {
    let cost = {
        let mut r = Realm::new();
        let before = r.collect_garbage();
        r.add_unit_values().unwrap();
        r.collect_garbage() - before
    };
    let mut failed = 0;
    for step in 0..=16 {
        let mut r = Realm::new();
        let live = r.collect_garbage();
        r.set_limits(Limits { heap_bytes: live + cost * step / 16, ..Limits::default() });
        let first = r.add_unit_values();
        r.set_limits(Limits::default());
        if first.is_err() {
            failed += 1;
            r.add_unit_values().unwrap();
        }
        assert_eq!(number(&mut r, "UnitValue(1, 'in').as('pt')"), 72.0, "limit at {step}/16 of the install");
    }
    assert!(failed >= 1, "no limit stopped the install");
}

/// Installs with members that stop after the global `UnitValue` is defined.
fn stopped_install(r: &mut Realm) {
    let stopped = crate::unit_value::install_with(r.interp(), |i, proto| {
        crate::builtins::method(i, proto, "toString", |_, _, _, _| Ok(crate::value::Value::Undefined), 0)?;
        let global = i.global;
        crate::builtins::data(i, global, "UnitValue", crate::value::Value::Number(1.0), 0)?;
        Err(crate::interp::Abrupt::Internal("stopped".into()))
    });
    assert!(stopped.is_err());
}

#[test]
fn an_install_that_fails_part_way_installs_everything_on_the_next_call() {
    let mut r = Realm::new();
    stopped_install(&mut r);
    assert_eq!(text(&mut r, "typeof UnitValue"), "undefined");
    assert!(boolean(&mut r, "!('UnitValue' in this)"));
    let mut kept = Realm::new();
    kept.run("var UnitValue = 'mine';").unwrap();
    stopped_install(&mut kept);
    assert_eq!(text(&mut kept, "UnitValue"), "mine");
    r.add_unit_values().unwrap();
    assert_eq!(number(&mut r, "UnitValue(1, 'in').as('pt')"), 72.0);
    assert_eq!(text(&mut r, "String(UnitValue(1, 'in') + UnitValue(72, 'pt'))"), "2 in");
}

#[test]
fn a_second_install_changes_nothing() {
    let mut r = realm();
    r.run("UnitValue.baseUnit = UnitValue(1, 'mm'); var Kept = UnitValue;").unwrap();
    r.add_unit_values().unwrap();
    assert!(boolean(&mut r, "Kept === UnitValue && UnitValue(1, 'mm').as('px') === 1"));
}
