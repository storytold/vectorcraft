//! Resumable runs and the structured values host functions take and give.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;

fn realm(fuel: u64) -> Realm {
    let mut r = Realm::new();
    r.set_limits(Limits { fuel, ..Limits::default() });
    r
}

/// A realm with `host.take(value)`, which records what it was given and returns it.
fn with_take(r: &mut Realm) -> Rc<RefCell<Vec<Data>>> {
    let seen: Rc<RefCell<Vec<Data>>> = Rc::default();
    let log = seen.clone();
    let f: HostFunction = Rc::new(move |mut args| {
        let v = if args.is_empty() { Data::Undefined } else { args.remove(0) };
        log.borrow_mut().push(v.clone());
        Ok(v)
    });
    r.define_host_object("host", vec![("take".into(), HostItem::Function(f))]).unwrap();
    seen
}

fn finish(r: &mut Realm, slice: u64) -> (Result<JsValue, RunError>, u32) {
    let mut slices = 0;
    loop {
        slices += 1;
        match r.advance(slice) {
            Ok(Progress::Paused) => {}
            Ok(Progress::Done(v)) => return (Ok(v), slices),
            Err(e) => return (Err(e), slices),
        }
    }
}

#[test]
fn a_resumed_run_gives_the_result_of_one_run() {
    let source = "var t = 0; for (var i = 0; i < 2000; i++) { t += i; } t";
    let whole = realm(10_000_000).run(source).unwrap();
    let mut r = realm(10_000_000);
    r.start(source).unwrap();
    assert!(r.is_running());
    let (v, slices) = finish(&mut r, 100);
    assert_eq!(v.unwrap(), whole);
    assert!(slices > 10, "the run paused: {slices} slices");
    assert!(!r.is_running());
}

#[test]
fn state_survives_a_pause_and_a_collection_between_slices() {
    let mut r = realm(10_000_000);
    r.start("var o = {a: [1, 2, 3]}; var n = 0; for (var i = 0; i < 500; i++) { n += o.a.length; } n").unwrap();
    assert_eq!(r.advance(50).unwrap(), Progress::Paused);
    r.collect_garbage();
    assert_eq!(finish(&mut r, 50).0.unwrap(), JsValue::Number(1500.0));
}

#[test]
fn the_fuel_limit_covers_every_slice_together() {
    let mut r = realm(5_000);
    r.start("while (true) {}").unwrap();
    let (v, slices) = finish(&mut r, 100);
    assert!(matches!(v, Err(RunError { kind: RunErrorKind::Limit(LimitKind::Fuel), .. })));
    assert!((40..=60).contains(&slices), "{slices}");
}

#[test]
fn a_set_flag_stops_a_loop_in_a_getter() {
    let mut r = realm(u64::MAX);
    r.start("var o = {}; Object.defineProperty(o, 'x', {get: function () { while (true) {} }}); o.x").unwrap();
    r.interrupt().set();
    let e = r.advance(10).unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Interrupted));
}

#[test]
fn a_nested_run_past_the_slice_allowance_is_the_unbroken_limit_and_a_run_with_little_fuel_is_the_fuel_limit() {
    for (total, kind) in [(u64::MAX, LimitKind::Unbroken), (1_000, LimitKind::Fuel)] {
        for source in [
            "var o = {}; Object.defineProperty(o, 'x', {get: function () { while (true) {} }}); o.x",
            "[1].forEach(function () { while (true) {} });",
        ] {
            let mut r = realm(total);
            r.set_unbroken_fuel(5_000);
            r.start(source).unwrap();
            let e = finish(&mut r, 1).0.unwrap_err();
            assert_eq!(e.kind, RunErrorKind::Limit(kind), "{source}");
        }
    }
}

#[test]
fn neither_a_limit_nor_a_stop_can_be_caught() {
    let mut r = realm(1_000);
    r.start("try { while (true) {} } catch (e) { 'caught' } finally { 'ran' }").unwrap();
    assert_eq!(finish(&mut r, 50).0.unwrap_err().kind, RunErrorKind::Limit(LimitKind::Fuel));

    let shared = Interrupt::new();
    let mut r = realm(1_000_000);
    r.share_interrupt(&shared);
    r.start("try { while (true) {} } catch (e) { 'caught' }").unwrap();
    assert_eq!(r.advance(10).unwrap(), Progress::Paused);
    shared.set();
    assert_eq!(r.advance(10).unwrap_err().kind, RunErrorKind::Limit(LimitKind::Interrupted));
}

#[test]
fn a_host_function_can_end_the_slice() {
    let mut r = realm(1_000_000);
    let pause = r.pause_flag();
    let f: HostFunction = Rc::new(move |_| {
        pause.set(true);
        Ok(Data::Null)
    });
    r.define_host_object("host", vec![("yield".into(), HostItem::Function(f))]).unwrap();
    r.start("host.yield(); host.yield(); 7").unwrap();
    assert_eq!(r.advance(1_000).unwrap(), Progress::Paused);
    assert_eq!(r.advance(1_000).unwrap(), Progress::Paused);
    assert_eq!(r.advance(1_000).unwrap(), Progress::Done(JsValue::Number(7.0)));
}

#[test]
fn plain_values_cross_in_both_directions() {
    let mut r = realm(1_000_000);
    let seen = with_take(&mut r);
    let out = r.run("var v = host.take({a: 1, b: 'x', c: [true, null], d: undefined}); v.c[0] === true && v.b === 'x' && v.d === undefined");
    assert_eq!(out.unwrap(), JsValue::Bool(true));
    let want = Data::Object(vec![
        ("a".into(), Data::Number(1.0)),
        ("b".into(), Data::String("x".into())),
        ("c".into(), Data::Array(vec![Data::Bool(true), Data::Null])),
    ]);
    assert_eq!(seen.borrow().as_slice(), [want]);
}

#[test]
fn a_callback_of_a_built_in_can_call_the_host() {
    let mut r = realm(1_000_000);
    let seen = with_take(&mut r);
    r.start("[1, 2, 3].forEach(function (n) { host.take(n); });").unwrap();
    finish(&mut r, 2).0.unwrap();
    assert_eq!(seen.borrow().len(), 3);
}

/// `value` passed to the host throws an error of type `name` whose message holds `part`.
fn refused(setup: &str, name: &str, part: &str) {
    let mut r = realm(10_000_000);
    r.set_data_limits(DataLimits { depth: 8, bytes: 1_000, entries: 100 });
    let seen = with_take(&mut r);
    let probe = format!("var e; {setup} try {{ host.take(value); }} catch (x) {{ e = x; }} e ? e.name + '|' + e.message : 'none'");
    let JsValue::String(out) = r.run(&probe).unwrap() else { panic!("{setup}") };
    assert!(out.starts_with(name) && out.contains(part), "{setup}: {out}");
    assert!(seen.borrow().is_empty(), "{setup}: the function ran");
}

#[test]
fn values_that_are_not_plain_data_are_refused_with_their_place() {
    refused("var value = {}; value.self = value;", "TypeError", ".self refers to itself");
    refused("var value = [[1]]; value[0][0] = value;", "TypeError", "[0][0][0] refers to itself");
    refused("var value = {f: function () {}};", "TypeError", ".f is a Function");
    refused("var value = {n: 1 / 0};", "TypeError", ".n is not a finite number");
    refused("var value = {n: 0 / 0};", "TypeError", ".n is not a finite number");
    refused("var value = {d: new Date(0)};", "TypeError", ".d is a Date");
    refused("var value = {}; Object.defineProperty(value, 'g', {enumerable: true, get: function () { return 1; }});", "TypeError", ".g is a getter");
    refused("var value = [1, , 3];", "TypeError", "[0][1] is undefined");
    refused("var value = 1; for (var i = 0; i < 20; i++) value = [value];", "RangeError", "nested too deep");
    refused("var value = new Array(5000);", "RangeError", "too many entries");
    refused("var value = 'x'; for (var i = 0; i < 12; i++) value += value;", "RangeError", "too large");
}

#[test]
fn a_value_nested_ten_thousand_deep_is_refused_with_its_place() {
    let mut r = realm(100_000_000);
    let seen = with_take(&mut r);
    let probe = "var v = 1; for (var i = 0; i < 10000; i++) v = {a: v}; var m = 'none'; \
                 try { host.take({args: v}); } catch (e) { m = e.name + '|' + e.message; } m";
    let JsValue::String(out) = r.run(probe).unwrap() else { panic!("not a string") };
    assert!(out.starts_with("RangeError|") && out.contains(".args.a.a.a") && out.contains("nested too deep"), "{out}");
    assert!(seen.borrow().is_empty(), "the function ran");
}

#[test]
fn a_shared_object_is_allowed_and_sharing_cannot_multiply_the_work() {
    let mut r = realm(10_000_000);
    r.set_data_limits(DataLimits { depth: 64, bytes: 16 << 20, entries: 1_000 });
    let seen = with_take(&mut r);
    r.run("var s = {a: 1}; host.take({x: s, y: s, z: [s, s]});").unwrap();
    assert_eq!(seen.borrow().len(), 1);
    // Each level doubles the paths to the bottom: the entry limit ends the walk.
    let JsValue::String(out) =
        r.run("var v = [1]; for (var i = 0; i < 40; i++) v = [v, v]; var m = 'none'; try { host.take(v); } catch (e) { m = e.name; } m").unwrap()
    else {
        panic!()
    };
    assert_eq!(out, "RangeError");
}

#[test]
fn a_host_object_cannot_be_changed_by_a_script() {
    let mut r = realm(1_000_000);
    let args = Data::Object(vec![("list".into(), Data::Array(vec![Data::Number(1.0)]))]);
    r.define_host_object("vc", vec![("args".into(), HostItem::Value(args))]).unwrap();
    let out = r.run(
        "vc.args.list[0] = 2; vc.args.list.push; vc.args.more = 1; vc.extra = 1; var was = vc; vc = 3; \
         [was.args.list[0], was.args.more === undefined, was.extra === undefined, Object.isFrozen(was.args.list)].join()",
    );
    assert_eq!(out.unwrap(), JsValue::String("1,true,true,true".into()));
}

#[test]
fn a_getter_is_read_each_time() {
    let mut r = realm(1_000_000);
    let count = Rc::new(Cell::new(0.0));
    let c = count.clone();
    let f: HostFunction = Rc::new(move |_| {
        c.set(c.get() + 1.0);
        Ok(Data::Number(c.get()))
    });
    r.define_host_object("host", vec![("n".into(), HostItem::Getter(f))]).unwrap();
    assert_eq!(r.run("host.n + host.n").unwrap(), JsValue::Number(3.0));
}

#[test]
fn a_text_function_takes_each_argument_as_string_gives_it() {
    let mut r = realm(1_000_000);
    let seen: Rc<RefCell<Vec<Data>>> = Rc::default();
    let log = seen.clone();
    let f: HostFunction = Rc::new(move |args| {
        log.borrow_mut().extend(args);
        Ok(Data::Undefined)
    });
    r.define_host_object("host", vec![("text".into(), HostItem::TextFunction(f))]).unwrap();
    r.run("host.text(1 / 0, NaN, undefined, null, 0.1, [1, [2]], {toString: function () { return 'own'; }}, function f() {}, '\\ud800x')").unwrap();
    let texts: Vec<String> = seen.borrow().iter().map(|d| if let Data::String(s) = d { s.clone() } else { format!("{d:?}") }).collect();
    assert_eq!(texts[..7], ["Infinity", "NaN", "undefined", "null", "0.1", "1,2", "own"]);
    assert!(texts[7].starts_with("function f()"), "{}", texts[7]);
    assert_eq!(texts[8], "\u{fffd}x", "a lone surrogate becomes U+FFFD");
    let e = r.run("host.text({toString: function () { throw new Error('no text'); }})").unwrap_err();
    assert!(e.message.contains("no text"), "{}", e.message);
}

#[test]
fn errors_from_the_host_can_be_caught_and_an_abort_cannot() {
    let mut r = realm(1_000_000);
    let f: HostFunction = Rc::new(|a| match a.first() {
        Some(Data::String(k)) if k == "type" => Err(HostError::Type("bad type".into())),
        Some(Data::String(k)) if k == "range" => Err(HostError::Range("bad range".into())),
        Some(Data::String(k)) if k == "abort" => Err(HostError::Abort("stopped by the host".into())),
        _ => Err(HostError::Error("bad".into())),
    });
    r.define_host_object("host", vec![("f".into(), HostItem::Function(f))]).unwrap();
    let out = r.run(
        "var a = []; ['type', 'range', 'x'].forEach(function (k) { try { host.f(k); } catch (e) { a.push(e.name + ':' + e.message); } }); a.join()",
    );
    assert_eq!(out.unwrap(), JsValue::String("TypeError:bad type,RangeError:bad range,Error:bad".into()));
    let e = r.run("try { host.f('abort'); } catch (e) { 'caught' }").unwrap_err();
    assert_eq!((e.kind, e.message.as_str()), (RunErrorKind::Limit(LimitKind::Host), "stopped by the host"));
}

#[test]
fn an_error_says_where_in_the_source_it_was_raised() {
    let mut r = realm(1_000_000);
    r.start("var a = 1;\n  throw new TypeError('no');").unwrap();
    let e = finish(&mut r, 1).0.unwrap_err();
    assert_eq!((e.kind, e.line), (RunErrorKind::Thrown, 2));
    assert!(e.column >= 3, "{}", e.column);
    assert_eq!(e.message, "TypeError: no");
}

#[test]
fn advancing_without_a_run_is_an_error() {
    let mut r = realm(1_000);
    let e = r.advance(10).unwrap_err();
    assert_eq!((e.kind, e.message.as_str()), (RunErrorKind::Internal, "no run to advance"));
}

proptest::proptest! {
    /// A slice size does not change what a script computes.
    #[test]
    fn any_slice_size_gives_the_same_result(slice in 1u64..500, n in 0u32..300) {
        let source = format!("var a = []; for (var i = 0; i < {n}; i++) a.push(i * 2); a.join().length");
        let whole = realm(50_000_000).run(&source).unwrap();
        let mut r = realm(50_000_000);
        r.start(&source).unwrap();
        proptest::prop_assert_eq!(finish(&mut r, slice).0.unwrap(), whole);
    }

    /// Hostile text for a value never makes the bridge panic.
    #[test]
    fn hostile_values_end_with_a_value_or_a_script_error(depth in 0usize..120, width in 0usize..6, cycle in proptest::bool::ANY) {
        let mut r = realm(5_000_000);
        with_take(&mut r);
        let mut s = String::from("var v = [];");
        for _ in 0..depth {
            s.push_str(&format!("v = [v{}];", ",v".repeat(width.min(2))));
        }
        if cycle {
            s.push_str("var c = {}; c.c = c; v = [v, c];");
        }
        s.push_str("var ok = 'ok'; try { host.take(v); } catch (e) { ok = e.name; } ok");
        let JsValue::String(out) = r.run(&s).unwrap() else { panic!() };
        proptest::prop_assert!(matches!(out.as_str(), "ok" | "RangeError" | "TypeError"), "{}", out);
    }
}
