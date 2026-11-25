//! Async functions and `await` in realms with [`Options::es2017`].

use super::*;

fn realm() -> Realm {
    Realm::with_options(Options { es2017: true, ..Options::default() }, Limits::default())
}

/// Runs `src`, which writes to the global `log`, and gives `log` after the jobs ran.
fn log(src: &str) -> String {
    let mut r = realm();
    if let Err(e) = r.run(&format!("var log = [];\n{src}")) {
        panic!("{src}\n{e:?}");
    }
    match r.run("log.join(' ')") {
        Ok(JsValue::String(s)) => s,
        other => panic!("{src}: {other:?}"),
    }
}

fn syntax_error(r: &mut Realm, src: &str) {
    match r.run(src) {
        Err(e) if e.kind == RunErrorKind::Syntax => {}
        other => panic!("{src}: {other:?}"),
    }
}

#[test]
fn awaits_resume_in_the_job_order_promise_reactions_have() {
    let src = "async function f() { log.push('f1'); await null; log.push('f2'); await null; log.push('f3'); }\n\
               f();\n\
               Promise.resolve().then(function () { log.push('t1'); }).then(function () { log.push('t2'); });\n\
               log.push('sync');";
    assert_eq!(log(src), "f1 sync f2 t1 f3 t2");
}

#[test]
fn the_function_returns_a_promise_its_return_resolves_and_its_throw_rejects() {
    let src = "async function ok(x) { return x + 1; }\n\
               async function bad() { throw new TypeError('no'); }\n\
               var p = ok(1); log.push(p instanceof Promise);\n\
               p.then(function (v) { log.push('ok ' + v); });\n\
               bad().then(null, function (e) { log.push(e instanceof TypeError); });";
    assert_eq!(log(src), "true ok 2 true");
}

#[test]
fn a_rejected_await_throws_where_it_stands_and_finally_runs_after_an_await() {
    let src = "async function f() {\n\
                 try { await Promise.reject('r'); log.push('not here'); }\n\
                 catch (e) { log.push('caught ' + e); }\n\
                 finally { await 1; log.push('finally'); }\n\
                 return 'end';\n\
               }\n\
               f().then(function (v) { log.push(v); });";
    assert_eq!(log(src), "caught r finally end");
    let rethrow = "async function g() { try { await Promise.reject(new Error('x')); } finally { log.push('cleanup'); } }\n\
                   g().catch(function (e) { log.push('rejected ' + e.message); });";
    assert_eq!(log(rethrow), "cleanup rejected x");
}

#[test]
fn arrows_expressions_and_methods_can_be_async() {
    let src = "var a = async x => (await x) * 2;\n\
               var b = async (x, y) => { return await x + y; };\n\
               var c = async function named() { return typeof named; };\n\
               var o = { async m(v) { return this.k + await v; }, k: 'k' };\n\
               Promise.all([a(2), b(Promise.resolve(1), 2), c(), o.m('v')]).then(function (r) { log.push(r.join(',')); });";
    assert_eq!(log(src), "4,3,function,kv");
}

#[test]
fn async_functions_are_not_constructors_and_have_no_prototype() {
    let src = "async function f() {}\n\
               log.push(f.hasOwnProperty('prototype'));\n\
               try { new f(); } catch (e) { log.push(e instanceof TypeError); }\n\
               var AF = Object.getPrototypeOf(f).constructor;\n\
               log.push(AF !== Function, typeof AF, Object.getPrototypeOf(AF) === Function);\n\
               log.push(typeof this.AsyncFunction);\n\
               var g = AF('a', 'return await a;');\n\
               log.push(Object.getPrototypeOf(g) === Object.getPrototypeOf(f));\n\
               g(5).then(function (v) { log.push(v); });\n\
               log.push(String(async function h() { await 0; }));";
    assert_eq!(log(src), "false true true function true undefined true async function h() { await 0; } 5");
}

#[test]
fn async_function_inherits_from_the_built_in_function_after_a_script_replaced_its_constructor() {
    let mut r = Realm::new();
    assert_eq!(r.run("Function.prototype.constructor = {}; 0"), Ok(JsValue::Number(0.0)));
    r.add_es2015();
    r.set_es2017(true);
    let src = "var AF = Object.getPrototypeOf(async function () {}).constructor; Object.getPrototypeOf(AF) === Function && typeof AF.call";
    assert_eq!(r.run(src), Ok(JsValue::String("function".into())));
}

#[test]
fn with_e4x_async_before_a_double_colon_is_a_qualified_name() {
    let options = Options { es2017: true, e4x: true, ..Options::default() };
    assert!(parse("async::b", &options).is_ok());
    assert!(parse("var x = <a/>; x.async::b", &options).is_ok());
}

#[test]
fn await_is_an_identifier_outside_async_functions() {
    let src = "var await = 3; function f(await) { return await; } log.push(f(await));\n\
               var async = function (x) { return x; }; log.push(async(1));\n\
               async\nfunction g() {}\n\
               log.push(typeof g);";
    assert_eq!(log(src), "3 1 function");
    // A function in an async arrow function's parameters reads `await` as its own body does,
    // and an async function declared in a script may be named `await`.
    let nested = "var g = async (x = function () { return await; }) => x;\n\
                  var h = async (x = async function () { await 0; }) => x;\n\
                  async function await() { return 1; }\n\
                  log.push(typeof g, typeof h, typeof await);";
    assert_eq!(log(nested), "function function function");
    // An arrow function in an async arrow function's parameters reads `await` as its own body does.
    let arrow = "var await = 7; var f = async (cb = () => await) => cb(); f().then(function (v) { log.push(v); });";
    assert_eq!(log(arrow), "7");
    let mut r = realm();
    syntax_error(&mut r, "async function f() { var await; }");
    syntax_error(&mut r, "async function f(await) {}");
    syntax_error(&mut r, "async function f(x = await 1) {}");
    syntax_error(&mut r, "function f() { await 1; }");
    syntax_error(&mut r, "if (1) async function f() {}");
    syntax_error(&mut r, "async (x = await 1) => x");
    syntax_error(&mut r, "async\n(x) => x");
    syntax_error(&mut r, "async (a = (...await) => {}) => {}");
}

#[test]
fn a_suspended_function_survives_a_collection_and_an_unreachable_one_is_freed() {
    let mut r = realm();
    r.run(
        "var resume; var gate = new Promise(function (res) { resume = res; });\n\
           var out = [];\n\
           async function f() { var local = { n: 41 }; var v = await gate; out.push(local.n + v); }\n\
           f();",
    )
    .unwrap();
    r.interp().collect_garbage();
    r.run("resume(1);").unwrap();
    assert_eq!(r.run("out.join()").unwrap(), JsValue::String("42".into()));
    // A function waiting on a promise nothing can settle is collected with it.
    r.interp().collect_garbage();
    let before = r.interp().heap.live_cells();
    r.run("(function () { async function w() { var big = []; for (var i = 0; i < 200; i++) big.push({ i: i }); await new Promise(function () {}); } w(); })();")
        .unwrap();
    r.interp().collect_garbage();
    let after = r.interp().heap.live_cells();
    assert!(after < before + 50, "{before} {after}");
}

#[test]
fn a_run_that_ends_with_awaits_pending_ends_cleanly() {
    let mut r = realm();
    let v = r.run("async function f() { await new Promise(function () {}); return 1; } f(); 'done'").unwrap();
    assert_eq!(v, JsValue::String("done".into()));
    assert!(!r.is_running());
    assert_eq!(r.run("1 + 1").unwrap(), JsValue::Number(2.0));
}

#[test]
fn each_resume_takes_fuel() {
    let mut r = realm();
    r.set_limits(Limits { fuel: 20_000, ..Limits::default() });
    let e = r.run("async function f() { for (;;) await 0; } f();").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel));
}

#[test]
fn es5_and_extendscript_realms_do_not_read_async_functions() {
    let mut es2015 = Realm::with_options(Options { es2015: true, ..Options::default() }, Limits::default());
    syntax_error(&mut es2015, "async function f() {}");
    assert_eq!(es2015.run("var async = 1, await = 2; async + await").unwrap(), JsValue::Number(3.0));
    syntax_error(&mut Realm::with_options(Options::default(), Limits::default()), "async function f() {}");
    syntax_error(&mut Realm::new(), "var f = async () => 1;");
}

/// Starts `src` in a realm with `start`, calls `change` once the run has paused, runs it to its
/// end, and gives the run's value and the realm's options after it.
fn run_changing_language(src: &str, start: Options, change: impl FnOnce(&mut Realm)) -> (Result<JsValue, RunError>, Options) {
    let mut realm = Realm::with_options(start, Limits::default());
    if let Err(e) = realm.start(src) {
        return (Err(e), realm.interp().options.clone());
    }
    match realm.advance(20) {
        Ok(crate::Progress::Paused) => {}
        other => panic!("{other:?}"),
    }
    change(&mut realm);
    loop {
        match realm.advance(1000) {
            Ok(crate::Progress::Done(v)) => return (Ok(v), realm.interp().options.clone()),
            Ok(_) => {}
            Err(e) => return (Err(e), realm.interp().options.clone()),
        }
    }
}

#[test]
fn language_changes_during_a_run_apply_after_it_in_the_order_they_were_made() {
    let src = "for (var i = 0; i < 200; i++) {} eval('(async function () {})'); 1";
    let es2017 = Options { es2017: true, ..Options::default() };
    let es2015 = Options { es2015: true, ..Options::default() };
    let (v, o) = run_changing_language(src, es2017.clone(), |r| r.set_es2017(false));
    assert_eq!(v, Ok(JsValue::Number(1.0)), "the run keeps async functions for the code it evaluates");
    assert!(o.es2015 && !o.es2017, "{o:?}");
    let (v, o) = run_changing_language(src, es2017.clone(), |r| {
        r.set_es2015(false);
        r.set_es2017(true);
    });
    assert_eq!(v, Ok(JsValue::Number(1.0)));
    assert!(o.es2015 && o.es2017, "the later call wins: {o:?}");
    let (v, o) = run_changing_language(src, es2017, |r| {
        r.set_es2017(true);
        r.set_es2015(false);
    });
    assert_eq!(v, Ok(JsValue::Number(1.0)));
    assert!(!o.es2015 && !o.es2017, "turning es2015 off turns es2017 off: {o:?}");
    let (v, o) = run_changing_language("for (var i = 0; i < 200; i++) {} 1", es2015, |r| r.set_es2017(true));
    assert_eq!(v, Ok(JsValue::Number(1.0)));
    assert!(o.es2015 && o.es2017, "{o:?}");
}
