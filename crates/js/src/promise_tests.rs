//! `Promise` and the promise job queue in realms with [`Options::es2015`].

use super::*;

fn realm() -> Realm {
    Realm::with_options(Options { es2015: true, ..Options::default() }, Limits::default())
}

fn run(src: &str) -> JsValue {
    match realm().run(src) {
        Ok(v) => v,
        Err(e) => panic!("{src}\n{e:?}"),
    }
}

fn string(src: &str) -> String {
    match run(src) {
        JsValue::String(s) => s,
        other => panic!("{src}: {other:?}"),
    }
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

#[test]
fn an_extendscript_realm_has_no_promise() {
    assert_eq!(Realm::new().run("typeof Promise").unwrap(), JsValue::String("undefined".into()));
    assert_eq!(string("typeof Promise"), "function");
}

#[test]
fn adding_promise_leaves_the_language_and_the_other_built_ins_as_they_were() {
    let mut r = Realm::new();
    r.add_es2015();
    assert_eq!(r.run("typeof Promise").unwrap(), JsValue::String("function".into()));
    assert_eq!(r.run("typeof Symbol").unwrap(), JsValue::String("undefined".into()));
    assert!(matches!(r.run("let a = 1;"), Err(RunError { kind: RunErrorKind::Syntax, .. })));
    r.set_es2015(true);
    assert_eq!(r.run("let b = 2; b").unwrap(), JsValue::Number(2.0));
}

#[test]
fn promise_prototype_cannot_be_replaced() {
    let src = "var before = Promise.prototype; Promise.prototype = {}; \
               var p = new Promise(function () {}); \
               [Promise.prototype === before, Object.getPrototypeOf(p) === before, p instanceof Promise].join()";
    assert_eq!(string(src), "true,true,true");
}

#[test]
fn then_callbacks_run_after_the_scripts_code_in_order() {
    let src = "Promise.resolve(1).then(function (v) { log.push('a' + v); });\n\
               new Promise(function (res) { log.push('executor'); res(2); }).then(function (v) { log.push('b' + v); });\n\
               log.push('sync');";
    assert_eq!(log(src), "executor sync a1 b2");
}

#[test]
fn jobs_run_first_in_first_out_and_a_jobs_jobs_join_the_end() {
    let src = "var p = Promise.resolve();\n\
               p.then(function () { log.push(1); Promise.resolve().then(function () { log.push(4); }); });\n\
               p.then(function () { log.push(2); });\n\
               p.then(function () { log.push(3); });";
    assert_eq!(log(src), "1 2 3 4");
}

#[test]
fn a_thenable_is_adopted_in_a_later_job() {
    let src = "var thenable = { then: function (res) { log.push('then'); res(7); } };\n\
               Promise.resolve(thenable).then(function (v) { log.push('got' + v); });\n\
               log.push('sync');";
    assert_eq!(log(src), "sync then got7");
    let rejecting = "var t = { then: function (res, rej) { rej('no'); res(1); } };\n\
                     new Promise(function (res) { res(t); }).then(null, function (r) { log.push('rejected ' + r); });";
    assert_eq!(log(rejecting), "rejected no");
    let throwing = "var t = { then: function (res) { res(1); throw new Error('late'); } };\n\
                    Promise.resolve(t).then(function (v) { log.push('kept ' + v); });";
    assert_eq!(log(throwing), "kept 1");
}

#[test]
fn a_promise_resolved_with_itself_rejects_with_a_type_error() {
    let src = "var res; var p = new Promise(function (r) { res = r; }); res(p);\n\
               p.then(null, function (e) { log.push(e instanceof TypeError); });";
    assert_eq!(log(src), "true");
}

#[test]
fn collections_while_a_promise_is_resolved_with_itself_keep_its_type_error() {
    let src = "var bad = 0; function check(e) { if (!(e instanceof TypeError)) bad++; }\n\
               for (var i = 0; i < 200; i++) { var res; var p = new Promise(function (r) { res = r; }); p.then(null, check); res(p); }";
    let mut finished = 0;
    for spare in (40_000..240_000).step_by(641) {
        let mut r = realm();
        let live = r.collect_garbage();
        r.set_limits(Limits { heap_bytes: live + spare, ..Limits::default() });
        match r.run(src) {
            Ok(_) => finished += 1,
            Err(e) if e.kind == RunErrorKind::Limit(LimitKind::Heap) => continue,
            Err(e) => panic!("{spare}: {e:?}"),
        }
        r.set_limits(Limits::default());
        assert_eq!(r.run("bad"), Ok(JsValue::Number(0.0)), "{spare}");
    }
    assert!(finished > 0);
}

#[test]
fn catch_finally_and_the_combinators_settle_as_the_specification_says() {
    let src = "Promise.reject(new Error('x')).catch(function (e) { log.push('caught ' + e.message); });\n\
               Promise.resolve(5).finally(function () { log.push('finally'); return 9; }).then(function (v) { log.push('kept ' + v); });\n\
               Promise.all([1, Promise.resolve(2), { then: function (r) { r(3); } }]).then(function (v) { log.push('all ' + v.join(',')); });\n\
               Promise.allSettled([Promise.reject('r'), 4]).then(function (v) { log.push(v[0].status + ':' + v[0].reason + ' ' + v[1].status + ':' + v[1].value); });\n\
               Promise.race([new Promise(function () {}), Promise.resolve('fast')]).then(function (v) { log.push('race ' + v); });\n\
               Promise.all('ab').then(function (v) { log.push('chars ' + v.length); });\n\
               Promise.all(5).then(null, function (e) { log.push('not a list ' + (e instanceof TypeError)); });\n\
               Promise.race(new Error('e')).then(null, function (e) { log.push('no length ' + (e instanceof TypeError)); });\n\
               Promise.all({ length: 1.5 }).then(null, function (e) { log.push('bad length ' + (e instanceof TypeError)); });";
    let out = log(src);
    for part in [
        "caught x",
        "finally",
        "kept 5",
        "all 1,2,3",
        "rejected:r fulfilled:4",
        "race fast",
        "chars 2",
        "not a list true",
        "no length true",
        "bad length true",
    ] {
        assert!(out.contains(part), "{part} in {out}");
    }
}

#[test]
fn promise_combinator_iterators_use_custom_values_and_boxed_string_code_points() {
    let src = "var input = {}; input[Symbol.iterator] = function () {\n\
                 var n = 0; return { next: function () { return n < 2 ? { value: ++n, done: false } : { done: true }; } };\n\
               };\n\
               Promise.all(input).then(function (v) { log.push('all ' + v.join(',')); });\n\
               Promise.allSettled(input).then(function (v) { log.push('settled ' + v[0].value + ',' + v[1].value); });\n\
               Promise.race(input).then(function (v) { log.push('race ' + v); });\n\
               var a = [99]; a[Symbol.iterator] = input[Symbol.iterator];\n\
               Promise.all(a).then(function (v) { log.push('array ' + v.join(',')); });\n\
               Promise.all(new String('\u{1d306}x')).then(function (v) { log.push('string ' + v.length + ',' + v[0].length); });";
    let out = log(src);
    for expected in ["all 1,2", "settled 1,2", "race 1", "array 1,2", "string 2,2"] {
        assert!(out.contains(expected), "{expected} in {out}");
    }
}

#[test]
fn promise_combinator_iterators_close_after_resolve_or_then_throws() {
    let src = "function check(name, failure) {\n\
                 var closed = 0, input = {};\n\
                 input[Symbol.iterator] = function () { return {\n\
                   next: function () { return { value: 1, done: false }; },\n\
                   return: function () { closed++; throw 'close'; }\n\
                 }; };\n\
                 Promise.resolve = failure === 'resolve' ? function () { throw 'resolve'; } :\n\
                   function () { return { get then() { throw 'then'; } }; };\n\
                 Promise[name](input).then(null, function (e) { log.push(name + ':' + failure + ':' + e + ':' + closed); });\n\
               }\n\
               var original = Promise.resolve;\n\
               for (var i = 0; i < 3; i++) { var name = ['all', 'allSettled', 'race'][i]; check(name, 'resolve'); check(name, 'then'); }\n\
               Promise.resolve = original;";
    let out = log(src);
    for name in ["all", "allSettled", "race"] {
        for failure in ["resolve", "then"] {
            let expected = format!("{name}:{failure}:{failure}:1");
            assert!(out.contains(&expected), "{expected} in {out}");
        }
    }
}

#[test]
fn promise_combinator_iterators_do_not_close_when_the_step_throws_or_finishes() {
    let src = "var closed = 0;\n\
               function input(failure) { var o = {}; o[Symbol.iterator] = function () { return {\n\
                 next: function () { if (failure === 'next') throw failure;\n\
                   return { get done() { if (failure === 'done') throw failure; return false; },\n\
                     get value() { throw failure; } }; },\n\
                 return: function () { closed++; return {}; }\n\
               }; }; return o; }\n\
               for (var i = 0; i < 3; i++) {\n\
                 Promise.all(input(['next', 'done', 'value'][i])).then(null, function (e) { log.push(e); });\n\
               }\n\
               var empty = {}; empty[Symbol.iterator] = function () { return {\n\
                 next: function () { return { done: true }; }, return: function () { closed++; return {}; }\n\
               }; };\n\
               Promise.all(empty).then(function (v) { log.push('empty ' + v.length); });\n\
               Promise.race(empty).then(function () { log.push('race settled'); });\n\
               log.push('closed ' + closed);";
    let out = log(src);
    for expected in ["next", "done", "value", "empty 0", "closed 0"] {
        assert!(out.contains(expected), "{expected} in {out}");
    }
    assert!(!out.contains("race settled"), "{out}");
}

#[test]
fn the_combinators_read_an_array_to_its_changing_length_and_reject_noniterable_array_likes() {
    // `Promise.resolve` runs once for each input and appends to the input while it is read.
    let src = "var resolve = Promise.resolve; var input;\n\
               Promise.resolve = function (x) {\n\
                 if (input.length < 3) { input[input.length] = x + 1; if (!(input instanceof Array)) input.length++; }\n\
                 return resolve.call(this, x);\n\
               };\n\
               input = [1]; Promise.all(input).then(function (v) { log.push('array ' + v.join(',')); });\n\
               input = { length: 1, 0: 1 }; Promise.all(input).then(null, function (e) { log.push('object ' + (e instanceof TypeError)); });";
    let out = log(src);
    assert!(out.contains("array 1,2,3"), "{out}");
    assert!(out.contains("object true"), "{out}");
}

#[test]
fn a_string_a_combinator_reads_is_charged_to_the_heap_before_it_is_flattened() {
    // A rope of 4 Mi code units is a few nodes until it is flattened to 8 MiB, more than the heap
    // has room for. The combinator would stop at its first input.
    let mut r = realm();
    r.run("var s = 'ab'; for (var i = 0; i < 21; i++) s = s + s; var calls = 0; Promise.resolve = function () { calls++; throw 'stop'; };").unwrap();
    let live = r.collect_garbage();
    r.set_limits(Limits { heap_bytes: live + (1 << 20), ..Limits::default() });
    let e = r.run("Promise.race(s)").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    r.set_limits(Limits::default());
    assert_eq!(r.run("calls").unwrap(), JsValue::Number(0.0));
}

#[test]
fn calling_promise_without_new_or_without_an_executor_is_a_type_error() {
    let mut r = realm();
    for src in [
        "Promise(function () {})",
        "new Promise(1)",
        "Promise.prototype.then.call({}, null)",
        "Promise.resolve.call(1, 2)",
        "Promise.all.call(Math.max, [])",
        "Promise.resolve.call(() => {}, 1)",
        "Promise.reject.call({ m() {} }.m, 1)",
        "Promise.race.call((() => {}).bind(null), [])",
    ] {
        match r.run(src) {
            Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with("TypeError"), "{src}: {message}"),
            other => panic!("{src}: {other:?}"),
        }
    }
}

#[test]
fn an_unhandled_rejection_is_reported_with_its_line() {
    let mut r = realm();
    r.run("var a = 1;\nPromise.reject(new Error('lost'));\nnew Promise(function () {\n  throw new TypeError('also');\n});\nPromise.reject(2).catch(function () {});")
        .unwrap();
    let got: Vec<(String, u32)> = r.unhandled_rejections().iter().map(|j| (j.message.clone(), j.line)).collect();
    assert_eq!(got, vec![("Error: lost".to_string(), 2), ("TypeError: also".to_string(), 4)]);
    assert_eq!(r.unhandled_rejection_count(), 2);
    assert!(r.unhandled_rejections().iter().all(|j| j.authored));
}

#[test]
fn a_long_rejection_reason_is_reported_cut_to_its_first_500_characters() {
    let mut r = realm();
    r.run(
        "function rep(t, n) { var s = ''; for (var i = 0; i < n; i++) s = s + t; return s; }\n\
         var ab = 'ab'; for (var i = 0; i < 16; i++) ab = ab + ab;\n\
         Promise.reject(ab);\n\
         Promise.reject(new RangeError(ab));\n\
         Promise.reject(rep('\\ud83d\\ude00', 600));\n\
         Promise.reject(rep('x', 500));",
    )
    .unwrap();
    let got: Vec<String> = r.unhandled_rejections().iter().map(|j| j.message.clone()).collect();
    let want = vec![
        format!("{}…", "ab".repeat(250)),
        format!("RangeError: {}…", "ab".repeat(244)),
        format!("{}…", "\u{1F600}".repeat(500)),
        "x".repeat(500),
    ];
    assert_eq!(got, want);
}

#[test]
fn combinator_rejection_relays_keep_the_input_location() {
    for method in ["all", "race"] {
        let mut r = realm();
        r.run(&format!("var input =\n  Promise.reject('lost');\nPromise.{method}([input]);")).unwrap();
        let reported = r.unhandled_rejections();
        assert_eq!(reported.len(), 1, "{method}: {reported:?}");
        assert_eq!(reported[0].line, 2, "{method}: {:?}", reported[0]);
        assert!(reported[0].column > 0);
        assert_eq!(reported[0].tag, r.script_tag());
    }
    let mut r = realm();
    r.run("var reject; new Promise(function (_, no) { reject = no; });\nPromise.reject('handled').catch(function () {\n  reject('lost');\n});")
        .unwrap();
    assert_eq!(r.unhandled_rejections().len(), 1);
    assert_eq!(r.unhandled_rejections()[0].line, 3);
    assert!(r.unhandled_rejections()[0].column > 0);
    let mut r = realm();
    r.run("eval(\"Promise.reject('eval')\"); Function(\"Promise.reject('function')\")();").unwrap();
    let reported = r.unhandled_rejections();
    assert_eq!(reported.len(), 2);
    assert!(reported.iter().all(|r| r.tag == UNKNOWN_SOURCE), "{reported:?}");
}

#[test]
fn an_interpreter_error_given_a_long_message_is_reported_as_the_scripts_text_without_copying_it() {
    let mut r = realm();
    r.run(
        "var ab = 'ab'; for (var i = 0; i < 16; i++) ab = ab + ab;\n\
         var e; try { null.x; } catch (x) { e = x; }\n\
         e.message = ab;\n\
         Promise.reject(e);\n\
         try { null.x; } catch (x) { Promise.reject(x); }",
    )
    .unwrap();
    let got: Vec<(String, bool)> = r.unhandled_rejections().iter().map(|j| (j.message.clone(), j.authored)).collect();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], (format!("TypeError: {}a…", "ab".repeat(244)), true));
    assert!(!got[1].1, "an error the interpreter threw, unchanged: {:?}", got[1]);
    let interp = r.interp();
    match interp.find_property(interp.global, &crate::heap::key("ab")) {
        Ok(Some(crate::heap::Prop::Data { value: crate::value::Value::String(s), .. })) => assert!(s.is_rope(), "the message was flattened"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_rejection_handled_in_a_later_job_is_not_reported() {
    let mut r = realm();
    r.run("var p = Promise.reject(1); Promise.resolve().then(function () { p.catch(function () {}); });").unwrap();
    assert!(r.unhandled_rejections().is_empty());
    assert_eq!(r.unhandled_rejection_count(), 0);
}

#[test]
fn a_run_that_fails_reports_no_rejections_and_drops_its_jobs() {
    let mut r = realm();
    let e = r.run("var ran = false; Promise.reject(1); Promise.resolve().then(function () { ran = true; }); throw 'stop';").unwrap_err();
    assert_eq!((e.kind, e.message.as_str()), (RunErrorKind::Thrown, "stop"));
    assert!(r.unhandled_rejections().is_empty());
    assert_eq!(r.run("ran").unwrap(), JsValue::Bool(false));
}

#[test]
fn an_endless_then_chain_is_stopped_by_fuel() {
    let mut r = Realm::with_options(Options { es2015: true, ..Options::default() }, Limits { fuel: 200_000, ..Limits::default() });
    let e = r.run("function again() { return Promise.resolve().then(again); } again();").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel));
    let e = r.run("var res; var p = new Promise(function (r) { res = r; }); function loop() { p.then(loop); } p.then(loop); res(1);").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel));
    assert!(!r.is_running());
}

#[test]
fn each_job_takes_fuel() {
    // `then` on a settled promise queues a job, and on a pending one it does not; the difference in
    // fuel between 50 calls and none is the jobs' fuel.
    let used = |start: &str, calls: usize| {
        let mut r = realm();
        r.set_limits(Limits { fuel: 1_000_000, ..Limits::default() });
        r.run(&format!("var p = {start};{}", "p.then();".repeat(calls))).unwrap();
        1_000_000 - r.fuel_left()
    };
    let settled = used("Promise.resolve()", 50) - used("Promise.resolve()", 0);
    let pending = used("new Promise(function () {})", 50) - used("new Promise(function () {})", 0);
    assert_eq!(settled - pending, 50);
}

#[test]
fn more_jobs_than_the_queue_holds_end_the_run_with_a_clear_error() {
    let mut r = realm();
    r.interp().promises.max_jobs = 100;
    let e = r.run("var p = Promise.resolve(); for (var i = 0; i < 200; i++) p.then(function () {});").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Jobs));
    assert!(e.message.contains("more than 100 promise jobs"), "{}", e.message);
    assert!(r.run("var q = Promise.resolve(); for (var i = 0; i < 50; i++) q.then(function () {}); 1").is_ok());
}

#[test]
fn a_sliced_run_pauses_inside_the_jobs_and_ends_once_the_queue_is_empty() {
    let mut r = realm();
    r.set_limits(Limits { fuel: 10_000_000, ..Limits::default() });
    r.start("var t = 0; Promise.resolve().then(function () { for (var i = 0; i < 5000; i++) t += i; }); 'done'").unwrap();
    let mut paused_in_jobs = 0;
    let v = loop {
        match r.advance(500).unwrap() {
            Progress::Paused => {
                if r.interp().promises.draining {
                    paused_in_jobs += 1;
                }
            }
            Progress::Done(v) => break v,
        }
    };
    assert_eq!(v, JsValue::String("done".into()));
    assert!(paused_in_jobs > 1, "{paused_in_jobs}");
    assert!(!r.is_running());
    assert_eq!(r.run("t").unwrap(), JsValue::Number(12_497_500.0));
}

#[test]
fn a_sliced_run_whose_last_job_ends_a_slice_is_done_in_that_slice() {
    let mut r = realm();
    r.start("Promise.resolve().then(); Promise.reject(1).then(null, function () {}); 'done'").unwrap();
    let v = loop {
        match r.advance(1).unwrap() {
            Progress::Paused => {
                let interp = r.interp();
                assert!(!interp.frames.is_empty() || interp.promises.queue_size().0 > 0, "paused with nothing left to run");
            }
            Progress::Done(v) => break v,
        }
    };
    assert_eq!(v, JsValue::String("done".into()));
    assert!(!r.is_running());
}

#[test]
fn a_pause_the_last_job_asks_for_does_not_pause_the_next_run() {
    let mut r = realm();
    let flag = r.pause_flag();
    r.define_function("pauseNow", move |_| {
        flag.set(true);
        Ok(JsValue::Undefined)
    });
    r.start("Promise.resolve().then(pauseNow); 'first'").unwrap();
    assert_eq!(r.advance(1_000_000).unwrap(), Progress::Done(JsValue::String("first".into())));
    r.start("var a = 1; var b = 2; a + b").unwrap();
    assert_eq!(r.advance(1_000_000).unwrap(), Progress::Done(JsValue::Number(3.0)));
}

#[test]
fn a_sliced_run_pauses_inside_a_finally_callback() {
    let mut r = realm();
    r.set_limits(Limits { fuel: 10_000_000, ..Limits::default() });
    r.set_unbroken_fuel(5_000);
    r.start(
        "var t = 0, kept, caught; function work() { for (var i = 0; i < 5000; i++) t += i; return Promise.resolve('ignored'); }\n\
         Promise.resolve(1).finally(work).then(function (v) { kept = v; });\n\
         Promise.reject(2).finally(work).then(null, function (e) { caught = e; });\n\
         'done'",
    )
    .unwrap();
    let mut paused_in_jobs = 0;
    let v = loop {
        match r.advance(500).unwrap() {
            Progress::Paused => {
                if r.interp().promises.draining {
                    paused_in_jobs += 1;
                }
            }
            Progress::Done(v) => break v,
        }
    };
    assert_eq!(v, JsValue::String("done".into()));
    assert!(paused_in_jobs > 10, "{paused_in_jobs}");
    assert_eq!(r.run("t + ' ' + kept + ' ' + caught").unwrap(), JsValue::String("24995000 1 2".into()));
}

#[test]
fn a_finally_callback_that_returns_a_pending_promise_holds_the_result_until_a_later_run_settles_it() {
    let mut r = realm();
    r.run(
        "var log = [], settle = {};\n\
         function wait(name) { return function () { return new Promise(function (res, rej) { settle[name] = { res: res, rej: rej }; }); }; }\n\
         function report(name) { return [function (v) { log.push(name + ' kept ' + v); }, function (e) { log.push(name + ' rejected ' + e); }]; }\n\
         var a = report('a'), b = report('b'), c = report('c'), d = report('d');\n\
         Promise.resolve(1).finally(wait('a')).then(a[0], a[1]);\n\
         Promise.reject(2).finally(wait('b')).then(b[0], b[1]);\n\
         Promise.resolve(3).finally(wait('c')).then(c[0], c[1]);\n\
         Promise.reject(4).finally(wait('d')).then(d[0], d[1]);",
    )
    .unwrap();
    assert_eq!(r.run("Object.keys(settle).sort().join(' ') + '|' + log.join(' ')").unwrap(), JsValue::String("a b c d|".into()));
    r.collect_garbage();
    r.run("settle.a.res('ignored'); settle.b.res('ignored'); settle.c.rej('new c'); settle.d.rej('new d');").unwrap();
    assert_eq!(r.run("log.join(', ')").unwrap(), JsValue::String("a kept 1, b rejected 2, c rejected new c, d rejected new d".into()));
}

#[test]
fn an_ended_sliced_run_drops_its_jobs() {
    let mut r = realm();
    r.start(
        "var ran = 0, after = false; Promise.resolve().then(function () { for (;;) ran++; }); Promise.resolve().then(function () { after = true; });",
    )
    .unwrap();
    let mut slices = 0;
    while r.advance(500).unwrap() == Progress::Paused && !r.interp().promises.draining {
        slices += 1;
        assert!(slices < 1000);
    }
    assert!(r.is_running());
    assert_eq!(r.interp().promises.queue_size().0, 1);
    r.end();
    assert!(!r.is_running());
    assert_eq!(r.interp().promises.queue_bytes(), 0);
    assert_eq!(r.interp().promises.queue_size(), (0, 0));
    assert_eq!(r.run("typeof ran").unwrap(), JsValue::String("number".into()));
    assert_eq!(r.run("after").unwrap(), JsValue::Bool(false));
}
