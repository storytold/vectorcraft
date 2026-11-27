//! `vc_call` and `vc_call_result`: a filter of API level 2 that lists `commands` calls the commands
//! its manifest's `commandAllowlist` names, through what the host gives the run to make them with.
//! Every misuse is a code the plug-in sees, and fails the run.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_plugins::host::{
    CALL_BAD_JSON, CALL_BAD_UTF8, CALL_COMMAND_FAILED, CALL_LIMIT, CALL_NO_CONTEXT, CALL_NO_RESULT, CALL_NOT_ALLOWED, CALL_OUTSIDE_MEMORY,
    CallFailure, MAX_CALL_DATA, MAX_CALLS,
};
use vectorcraft_plugins::{Error, Limits, Manifest, Plugin, wat as tpl};

const INPUT: &str = r#"{"objects":[]}"#;

/// The command imports, as `$vc_call` and `$vc_call_result`, and the strings the tests pass: the id
/// "select.all" at 0x2000, "object.delete" at 0x2010, the arguments `{"a":1}` at 0x2100, the byte
/// 0xff at 0x2200 and `[1]` at 0x2300. 0x3000 is free for a result.
const CALLS: &str = r#"(import "env" "vc_call" (func $vc_call (param i32 i32 i32 i32) (result i32)))
  (import "env" "vc_call_result" (func $vc_call_result (param i32 i32) (result i32)))
  (data (i32.const 0x2000) "select.all")
  (data (i32.const 0x2010) "object.delete")
  (data (i32.const 0x2100) "{\22a\22:1}")
  (data (i32.const 0x2200) "\ff")
  (data (i32.const 0x2300) "[1]")"#;

/// A filter `id` that may call `select.all`.
fn manifest(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"Caller","kind":"filter","api":2,"permissions":["doc.read","doc.write","commands"],"commandAllowlist":["select.all"]}}"#
    )
}

/// `vc_call` of the id at `id` (`len` bytes) with the arguments at `args` (`args_len` bytes).
fn call(id: &str, len: &str, args: &str, args_len: &str) -> String {
    format!("(call $vc_call (i32.const {id}) (i32.const {len}) (i32.const {args}) (i32.const {args_len}))")
}

/// `select.all` with `{"a":1}`.
fn select_all() -> String {
    call("0x2000", "10", "0x2100", "7")
}

/// Instructions that trap unless `expr` is `want`.
fn expect(expr: &str, want: i32) -> String {
    format!("(if (i32.ne {expr} (i32.const {want})) (then unreachable))")
}

/// A filter that runs `first`, then returns the input.
fn calling(id: &str, first: &str) -> Plugin {
    let src = tpl::with_imports(&manifest(id), CALLS, first, INPUT);
    Plugin::load(&wat::parse_str(&src).unwrap(), Limits::default()).unwrap()
}

/// Runs `p` with a bridge that answers every call with `answer` → the run's result and the calls it
/// got.
fn run_with(p: &Plugin, answer: impl Fn(&str) -> Result<Value, CallFailure>) -> (vectorcraft_plugins::Result<Vec<u8>>, Vec<(String, Value)>) {
    let mut seen = Vec::new();
    let mut bridge = |id: &str, args: &Value| {
        seen.push((id.to_string(), args.clone()));
        answer(id)
    };
    let (out, _) = p.run_calling(INPUT.as_bytes(), b"{}", p.limits(), &mut bridge);
    (out, seen)
}

#[test]
fn a_call_runs_its_command_and_the_result_is_read_back() {
    // The result `{"n":1}` is 7 bytes: a 2-byte buffer gets its length and nothing copied, then a
    // larger one gets it, which the run returns as its output.
    let run = format!(
        "{} {} (if (i32.ne (i32.load8_u (i32.const 0x3000)) (i32.const 0)) (then unreachable)) {} (i64.or (i64.shl (i64.const 7) (i64.const 32)) (i64.const 0x3000))",
        expect(&select_all(), 0),
        expect("(call $vc_call_result (i32.const 0x3000) (i32.const 2))", 7),
        expect("(call $vc_call_result (i32.const 0x3000) (i32.const 64))", 7),
    );
    let src = tpl::module(&manifest("test.call.result"), &run, CALLS, 1);
    let p = Plugin::load(&wat::parse_str(&src).unwrap(), Limits::default()).unwrap();
    let (out, seen) = run_with(&p, |_| Ok(json!({"n": 1})));
    assert_eq!(out.unwrap(), br#"{"n":1}"#);
    assert_eq!(seen, [("select.all".to_string(), json!({"a": 1}))]);
    // No bytes of arguments are `{}`; a result buffer outside memory is a code, not a failure.
    let p = calling(
        "test.call.empty",
        &[
            expect("(call $vc_call_result (i32.const 0x3000) (i32.const 64))", CALL_NO_RESULT),
            expect(&call("0x2000", "10", "0", "0"), 0),
            expect("(call $vc_call_result (i32.const -16) (i32.const 64))", CALL_OUTSIDE_MEMORY),
        ]
        .concat(),
    );
    let (out, seen) = run_with(&p, |_| Ok(Value::Null));
    assert_eq!(out.unwrap(), INPUT.as_bytes());
    assert_eq!(seen, [("select.all".to_string(), json!({}))]);
}

/// Each misuse returns its code, and the run fails with an error that names the call, even when the
/// plug-in goes on and returns an output. The host makes no call for any of them.
#[test]
fn a_call_the_host_refuses_fails_the_run_even_when_the_plugin_goes_on() {
    let cases = [
        (call("0x2010", "13", "0", "0"), CALL_NOT_ALLOWED, "vc_call `object.delete`: the manifest's `commandAllowlist` doesn't list it"),
        (call("0x2000", "10", "0x2200", "1"), CALL_BAD_UTF8, "vc_call `select.all`: the id or the arguments are not UTF-8"),
        (call("0x2000", "10", "0x2300", "3"), CALL_BAD_JSON, "vc_call `select.all`: the arguments are not a JSON object"),
        (call("0x2000", "10", "0x2100", "3"), CALL_BAD_JSON, "the arguments are not a JSON object"),
        (call("0x2000", "10", "-16", "7"), CALL_OUTSIDE_MEMORY, "vc_call `select.all`: the id or the arguments lie outside the module's memory"),
        (call("-4", "10", "0", "0"), CALL_OUTSIDE_MEMORY, "vc_call `?`: the id or the arguments lie outside the module's memory"),
        (call("0x2000", "200", "0", "0"), CALL_NOT_ALLOWED, "doesn't list it"),
    ];
    for (i, (first, code, says)) in cases.into_iter().enumerate() {
        let p = calling(&format!("test.call.refused{i}"), &expect(&first, code));
        let (out, seen) = run_with(&p, |_| Ok(Value::Null));
        let e = out.expect_err(says).to_string();
        assert!(e.contains(says), "{first}: {e}");
        assert!(seen.is_empty(), "{first}");
    }
}

/// Arguments longer than [`MAX_CALL_DATA`] are refused from their length alone: these 100 MiB start
/// outside memory, and the code is still the size limit.
#[test]
fn oversized_arguments_are_refused_before_they_are_read() {
    let p = calling("test.call.huge", &expect(&call("0x2000", "10", "-16", &(100 << 20).to_string()), CALL_LIMIT));
    let (out, seen) = run_with(&p, |_| Ok(Value::Null));
    let e = out.unwrap_err();
    assert!(matches!(&e, Error::Limit(why) if why.contains(&format!("104857600 bytes, limit {MAX_CALL_DATA}"))), "{e}");
    assert!(seen.is_empty());
}

#[test]
fn a_failed_command_fails_the_run_with_its_reason() {
    let p = calling("test.call.failed", &expect(&select_all(), CALL_COMMAND_FAILED));
    let (out, seen) = run_with(&p, |_| Err(CallFailure { code: CALL_COMMAND_FAILED, why: "nothing is selected".into() }));
    assert_eq!(out, Err(Error::Failed("vc_call `select.all`: nothing is selected".into())));
    assert_eq!(seen.len(), 1);
    // Also when the plug-in traps after it: the call's failure is the one reported.
    let p = calling("test.call.failed.trap", &format!("(drop {}) unreachable", select_all()));
    let (out, _) = run_with(&p, |_| Err(CallFailure { code: CALL_COMMAND_FAILED, why: "nothing is selected".into() }));
    assert_eq!(out, Err(Error::Failed("vc_call `select.all`: nothing is selected".into())));
}

/// After the run's first failed call, a plug-in that goes on calling gets [`CALL_NO_CONTEXT`] for
/// each call and the host makes none of them; the run fails with the first call's reason.
#[test]
fn no_call_is_made_after_a_failed_one() {
    let rest = [select_all(), select_all(), select_all()].map(|c| expect(&c, CALL_NO_CONTEXT)).concat();
    let p = calling("test.call.after.failed", &format!("{}{rest}", expect(&select_all(), CALL_COMMAND_FAILED)));
    let (out, seen) = run_with(&p, |_| Err(CallFailure { code: CALL_COMMAND_FAILED, why: "nothing is selected".into() }));
    assert_eq!(out, Err(Error::Failed("vc_call `select.all`: nothing is selected".into())));
    assert_eq!(seen.len(), 1);
    // Also after a call the host refused without making it.
    let p = calling(
        "test.call.after.refused",
        &format!("{}{}", expect(&call("0x2010", "13", "0", "0"), CALL_NOT_ALLOWED), expect(&select_all(), CALL_NO_CONTEXT)),
    );
    let (out, seen) = run_with(&p, |_| Ok(Value::Null));
    assert!(out.unwrap_err().to_string().contains("vc_call `object.delete`: the manifest's `commandAllowlist` doesn't list it"));
    assert!(seen.is_empty());
}

/// A result larger than [`MAX_CALL_DATA`] fails the run, and so do more than [`MAX_CALLS`] calls.
#[test]
fn results_and_calls_are_capped() {
    let p = calling("test.call.big", &expect(&select_all(), CALL_LIMIT));
    let (out, _) = run_with(&p, |_| Ok(Value::String("x".repeat(MAX_CALL_DATA))));
    assert!(matches!(out, Err(Error::Limit(ref why)) if why.starts_with("command call size (vc_call `select.all`")), "{out:?}");
    // A result of exactly MAX_CALL_DATA bytes (the string and its two quotes) is kept.
    let p = calling("test.call.edge", &expect(&select_all(), 0));
    let (out, _) = run_with(&p, |_| Ok(Value::String("x".repeat(MAX_CALL_DATA - 2))));
    assert_eq!(out.unwrap(), INPUT.as_bytes());
    let many = format!(
        "(local $i i32) (loop $l {} (local.set $i (i32.add (local.get $i) (i32.const 1))) (br_if $l (i32.lt_u (local.get $i) (i32.const {MAX_CALLS})))) {}",
        expect(&select_all(), 0),
        expect(&select_all(), CALL_LIMIT)
    );
    let p = calling("test.call.many", &many);
    let (out, seen) = run_with(&p, |_| Ok(Value::Null));
    assert!(matches!(out, Err(Error::Limit(ref why)) if why.starts_with("command calls (")), "{out:?}");
    assert_eq!(seen.len(), MAX_CALLS);
}

/// Each call costs instructions, so a budget too small for the calls ends the run: 256,000 (the
/// base and the input's share) pays for 25 calls of 10,000 and the loop around them.
#[test]
fn calls_cost_instructions() {
    let many = format!("(loop $l {} (br $l))", expect(&select_all(), 0));
    let src = tpl::with_imports(&manifest("test.call.fuel"), CALLS, &many, INPUT);
    let p = Plugin::load(&wat::parse_str(&src).unwrap(), Limits { fuel_base: 200_000, ..Limits::default() }).unwrap();
    let (out, seen) = run_with(&p, |_| Ok(Value::Null));
    assert_eq!(out, Err(Error::Limit("instruction budget".into())));
    assert_eq!(seen.len(), 25);
}

/// Cancel stops the run at the next call, and what it did is the caller's to undo.
#[test]
fn cancel_stops_a_run_at_its_next_call() {
    let p = calling("test.call.cancel", &[expect(&select_all(), 0), expect(&select_all(), 0)].concat());
    let token = vectorcraft_doc::cancel::CancelToken::new();
    let mut calls = 0;
    let mut bridge = |_: &str, _: &Value| {
        calls += 1;
        token.cancel();
        Ok(Value::Null)
    };
    let (out, _) = vectorcraft_doc::cancel::scope(&token, None, || p.run_calling(INPUT.as_bytes(), b"{}", p.limits(), &mut bridge));
    assert_eq!(out, Err(Error::Cancelled));
    assert_eq!(calls, 1);
}

/// Without a bridge (a run that isn't a filter's run on a document), and while the module is being
/// loaded, `vc_call` returns [`CALL_NO_CONTEXT`]; in a run, that fails the run as any failed call
/// does, also when the plug-in goes on.
#[test]
fn without_a_bridge_a_call_returns_no_context() {
    let no_context = |out: vectorcraft_plugins::Result<Vec<u8>>| {
        let e = out.unwrap_err();
        assert!(matches!(e, Error::Abi(_)) && e.to_string().contains("no command can be called here"), "{e}");
    };
    // The start function's call gets the code, and the module loads; a run, with a bridge or
    // without one, then fails on it, and the bridge is never asked.
    let start = format!("(func $start {}) (start $start)", expect(&select_all(), CALL_NO_CONTEXT));
    let src = tpl::with_imports(&manifest("test.call.none"), &format!("{CALLS} {start}"), "", INPUT);
    let p = Plugin::load(&wat::parse_str(&src).unwrap(), Limits::default()).unwrap();
    no_context(p.run(INPUT.as_bytes(), b"{}"));
    let (out, seen) = run_with(&p, |_| Ok(json!({})));
    no_context(out);
    assert!(seen.is_empty());
    // A call from `vc_run` without a bridge.
    no_context(calling("test.call.none", &expect(&select_all(), CALL_NO_CONTEXT)).run(INPUT.as_bytes(), b"{}"));
}

/// A module that imports `vc_call` or `vc_call_result` needs the permission `commands`, which needs
/// `api` 2 and a `commandAllowlist`; only a filter with the usual budget lists one.
#[test]
fn the_call_imports_need_the_commands_permission() {
    let import = r#"(import "env" "vc_call_result" (func (param i32 i32) (result i32)))"#;
    let without = r#"{"id":"test.call.perm","name":"C","kind":"filter","api":2,"permissions":["doc.read"]}"#;
    let e = Plugin::load(&wat::parse_str(tpl::with_imports(without, import, "", INPUT)).unwrap(), Limits::default()).unwrap_err().to_string();
    assert!(
        e.contains("imports `env.vc_call_result`, which a plug-in may use only at plug-in API level 2 (`api` 2) and with the permission `commands`"),
        "{e}"
    );
    for (manifest, says) in [
        (r#"{"id":"a","name":"x","kind":"filter","permissions":["commands"],"commandAllowlist":["select.all"]}"#, "needs plug-in API level 2"),
        (r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"]}"#, "`commands` needs a `commandAllowlist`"),
        (r#"{"id":"a","name":"x","kind":"filter","api":2,"commandAllowlist":["select.all"]}"#, "`commandAllowlist` needs the permission `commands`"),
        (
            r#"{"id":"a","name":"x","kind":"effect","api":2,"permissions":["doc.read","effect.render","commands"],"commandAllowlist":["select.all"]}"#,
            "only a filter with the usual budget",
        ),
        (
            r#"{"id":"a","name":"x","kind":"filter","api":2,"budget":"heavy","permissions":["commands"],"commandAllowlist":["select.all"]}"#,
            "only a filter with the usual budget",
        ),
        (
            r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":["select.all","select.all"]}"#,
            "names `select.all`",
        ),
        (r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":["select..all"]}"#, "not a command id"),
        (r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":["1select"]}"#, "not a command id"),
        (r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":[""]}"#, "not a command id"),
        (r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":["select/all"]}"#, "not a command id"),
        (r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":["select\\all"]}"#, "not a command id"),
        (r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":["../select.all"]}"#, "not a command id"),
        (
            r#"{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":[]}"#,
            "`commands` needs a `commandAllowlist`",
        ),
    ] {
        let e = Manifest::parse(manifest.as_bytes()).unwrap_err().to_string();
        assert!(e.contains(says), "{manifest}: {e}");
    }
    let long = format!(r#"{{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":["{}"]}}"#, "a".repeat(129));
    assert!(Manifest::parse(long.as_bytes()).unwrap_err().to_string().contains("not a command id"));
    let ids: Vec<String> = (0..257).map(|i| format!("\"c{i}\"")).collect();
    let many = format!(r#"{{"id":"a","name":"x","kind":"filter","api":2,"permissions":["commands"],"commandAllowlist":[{}]}}"#, ids.join(","));
    assert!(Manifest::parse(many.as_bytes()).unwrap_err().to_string().contains("at most 256 commands"));
    let m = Manifest::parse(manifest("test.call.ok").as_bytes()).unwrap();
    assert!(m.allows_call("select.all") && !m.allows_call("select.none"));
    assert_eq!(serde_json::to_value(&m).unwrap()["commandAllowlist"], json!(["select.all"]));
}

/// A small xorshift generator, so the cases are the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Up to `max` random bytes.
    fn bytes(&mut self, max: u64) -> Vec<u8> {
        (0..self.below(max + 1)).map(|_| self.next() as u8).collect()
    }

    /// A random JSON value nested at most `depth` deep.
    fn json(&mut self, depth: u32) -> Value {
        match self.below(if depth == 0 { 4 } else { 6 }) {
            0 => Value::Null,
            1 => Value::Bool(self.below(2) == 0),
            2 => json!((self.next() as i64) >> self.below(64)),
            3 => Value::String(String::from_utf8_lossy(&self.bytes(12)).into_owned()),
            4 => Value::Array((0..self.below(4)).map(|_| self.json(depth - 1)).collect()),
            _ => Value::Object((0..self.below(4)).map(|i| (format!("k{i}"), self.json(depth - 1))).collect()),
        }
    }
}

/// Any bytes as a call's id and arguments: random bytes, JSON values of every kind, cut-off and
/// corrupted JSON, ids with path characters or a NUL, and arguments of exactly [`MAX_CALL_DATA`]
/// bytes and one byte more. The host makes the call only for a listed id with a JSON object of at
/// most [`MAX_CALL_DATA`] bytes; every other call gets a code and fails the run with an error, and
/// none panics. The plug-in calls with its parameters as the id and its input as the arguments.
#[test]
fn any_bytes_as_a_call_get_a_code() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let ids: [&[u8]; 7] = [b"select.all", b"select.al", b"select.all\0", b"../select.all", b"select/all", b"\xff\xfe", b""];
    let mut cases: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for i in 0..600 {
        let args = match i % 4 {
            0 => rng.bytes(48),
            1 => serde_json::to_vec(&rng.json(3)).unwrap(),
            2 => {
                let mut v = serde_json::to_vec(&rng.json(3)).unwrap();
                v.truncate(rng.below(v.len() as u64 + 1) as usize);
                v
            }
            _ => {
                let mut v = serde_json::to_vec(&json!({"a": rng.json(3)})).unwrap();
                let at = rng.below(v.len() as u64) as usize;
                v[at] = rng.next() as u8;
                v
            }
        };
        let id = if rng.below(3) == 0 { rng.bytes(16) } else { ids[rng.below(ids.len() as u64) as usize].to_vec() };
        cases.push((args, id));
    }
    for len in [MAX_CALL_DATA, MAX_CALL_DATA + 1] {
        let mut v = br#"{"a":""#.to_vec();
        v.resize(len - 2, b'x');
        v.extend_from_slice(br#""}"#);
        cases.push((v, b"select.all".to_vec()));
    }
    let p = calling("test.call.fuzz", "(drop (call $vc_call (local.get $pp) (local.get $pl) (local.get $in) (local.get $il)))");
    let mut made = 0;
    for (args, id) in &cases {
        let mut seen = Vec::new();
        let mut bridge = |_: &str, a: &Value| {
            seen.push(a.clone());
            Ok(Value::Null)
        };
        let (out, _) = p.run_calling(args, id, p.limits(), &mut bridge);
        let object = if args.len() > MAX_CALL_DATA {
            None
        } else if args.is_empty() {
            Some(json!({}))
        } else {
            serde_json::from_slice::<Value>(args).ok().filter(Value::is_object)
        };
        let shown = String::from_utf8_lossy(&args[..args.len().min(64)]).into_owned();
        match object.filter(|_| id.as_slice() == b"select.all") {
            Some(v) => {
                assert_eq!(out.as_deref().ok(), Some(INPUT.as_bytes()), "{shown}");
                assert_eq!(seen, [v], "{shown}");
                made += 1;
            }
            None => {
                assert!(seen.is_empty(), "{shown}");
                let e = out.expect_err(&shown);
                assert!(matches!(e, Error::Abi(_) | Error::Limit(_)), "{shown}: {e}");
            }
        }
    }
    // Both kinds of case came up: calls made and calls refused.
    assert!(made >= 5 && made < cases.len() - 5, "{made} of {}", cases.len());
}
