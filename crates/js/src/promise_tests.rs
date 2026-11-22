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
fn calling_promise_without_new_or_without_an_executor_is_a_type_error() {
    let mut r = realm();
    for src in [
        "Promise(function () {})",
        "new Promise(1)",
        "Promise.prototype.then.call({}, null)",
        "Promise.resolve.call(1, 2)",
        "Promise.all.call(Math.max, [])",
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
fn an_ended_sliced_run_drops_its_jobs() {
    let mut r = realm();
    r.start("var ran = 0; Promise.resolve().then(function () { for (;;) ran++; });").unwrap();
    let mut slices = 0;
    while r.advance(500).unwrap() == Progress::Paused && !r.interp().promises.draining {
        slices += 1;
        assert!(slices < 1000);
    }
    assert!(r.is_running());
    r.end();
    assert!(!r.is_running());
    assert_eq!(r.interp().promises.queue_bytes(), 0);
    assert_eq!(r.run("typeof ran").unwrap(), JsValue::String("number".into()));
}
