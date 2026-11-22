//! The params description of each built-in command is parsed once per process; the answers read
//! from it are those of parsing it where they are used.

use std::hint::black_box;

use serde_json::{Value, json};

use crate::cmd::params::{PARSES, accepted_keys, check_keys, doc_keys, id_stands_for_ids, read_once, takes_key, uncached};
use crate::{Session, command_specs};

/// Every answer the key functions give about `doc` for command `id`, as text.
fn answers(id: &str, doc: &'static str) -> Vec<String> {
    let own = doc_keys(doc).map(|p| p.keys).unwrap_or_default();
    let accepted = accepted_keys(doc);
    let mut out = vec![format!("{accepted:?}"), format!("{}", id_stands_for_ids(doc))];
    let near = own.first().map(|k| format!("{k}q"));
    for key in own.iter().copied().chain(["ids", "id", "zzz"]).chain(near.as_deref()) {
        out.push(format!("{key}: {}", takes_key(doc, key)));
    }
    let all: serde_json::Map<String, Value> = accepted.iter().flatten().map(|k| (k.to_string(), Value::Null)).collect();
    let mut cases = vec![json!({}), json!({"zzz": 1}), Value::Object(all)];
    cases.extend(near.map(|k| json!({ k: 1 })));
    for params in cases {
        out.push(format!("{params}: {:?}", check_keys(id, doc, &params)));
    }
    out
}

#[test]
fn every_command_gets_the_answers_of_its_parsed_description() {
    for c in command_specs() {
        assert_eq!(answers(c.id, c.params), uncached(|| answers(c.id, c.params)), "{}", c.id);
    }
}

#[test]
fn a_built_in_description_is_not_parsed_again() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200, "created": null})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let run = |s: &mut Session| {
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        s.execute("object.move", &json!({"ids": [id], "dx": 1})).unwrap();
        s.execute("object.move", &json!({"ids": [id], "dx": -1})).unwrap();
        for c in command_specs() {
            black_box((accepted_keys(c.params), takes_key(c.params, "ids"), takes_key(c.params, "id"), id_stands_for_ids(c.params)));
            black_box(check_keys(c.id, c.params, &json!({"zzz": 1})).is_err());
        }
    };
    run(&mut s);
    PARSES.set(0);
    run(&mut s);
    assert_eq!(PARSES.get(), 0, "descriptions parsed again");
    // Without the table each command parses its description where it is used.
    uncached(|| run(&mut s));
    assert!(PARSES.get() > command_specs().len());
}

/// A description that isn't a built-in command's, even with the same words (a plug-in's), is parsed
/// where it is used, and so is one that names a plug-in command, whose keys change as plug-ins are
/// installed and removed.
#[test]
fn other_descriptions_are_parsed_where_they_are_used() {
    let spec = command_specs().iter().find(|c| id_stands_for_ids(c.params)).unwrap();
    let copy: &'static str = String::from(spec.params).leak();
    assert_eq!(answers(spec.id, copy), answers(spec.id, spec.params));
    PARSES.set(0);
    assert!(takes_key(copy, "ids") && id_stands_for_ids(copy));
    assert!(PARSES.get() > 0);
    assert!(read_once("{a, …settings as file.new}"));
    assert!(!read_once("{a, …settings as plugin.some.command}"));
    assert!(!read_once("{a, …the options of `plugin.some.command`}"));
}

/// This thread's CPU time in ns where Linux reports it, else the time since `start`.
fn now(start: std::time::Instant) -> (u128, bool) {
    let cpu = std::fs::read_to_string("/proc/thread-self/schedstat").ok().and_then(|s| s.split_whitespace().next()?.parse().ok());
    match cpu {
        Some(ns) => (ns, true),
        None => (start.elapsed().as_nanos(), false),
    }
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// `cargo test --release -p vectorcraft-engine --lib params_cache -- --ignored --nocapture` prints
/// the time of the key questions over every built-in description and of `Session::execute` on a
/// one-object document, with descriptions parsed where they are used (`parse`) and read once
/// (`once`), alternating in one process: the median of 7 runs, in this thread's CPU time on Linux.
#[test]
#[ignore = "benchmark"]
fn bench_params_keys_once() {
    let start = std::time::Instant::now();
    let specs = command_specs();
    let questions = || {
        for c in specs {
            black_box((accepted_keys(c.params), takes_key(c.params, "ids"), takes_key(c.params, "id")));
            black_box((takes_key(c.params, "ids"), takes_key(c.params, "id"), id_stands_for_ids(c.params)));
        }
    };
    let time = |f: &dyn Fn()| {
        let t = now(start);
        f();
        (now(start).0 - t.0, t.1)
    };
    questions();
    uncached(questions);
    let (mut parse, mut once, mut cpu) = (Vec::new(), Vec::new(), true);
    for _ in 0..7 {
        let (p, c1) = time(&|| uncached(|| (0..200).for_each(|_| questions())));
        let (o, c2) = time(&|| (0..200).for_each(|_| questions()));
        parse.push(p);
        once.push(o);
        cpu &= c1 && c2;
    }
    let per = |ns: u128| ns / (200 * specs.len() as u128);
    println!("clock: {}", if cpu { "thread CPU time" } else { "wall time" });
    println!("key questions, ns per description: parse {} once {}", per(median(parse)), per(median(once)));
    for (cmd, params) in [("select.set", "ids"), ("object.move", "move")] {
        let doc = || {
            let mut s = Session::new();
            s.execute("file.new", &json!({"width": 200, "height": 200, "created": null})).unwrap();
            let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
            let p = if params == "ids" { json!({"ids": [id]}) } else { json!({"ids": [id], "dx": 1}) };
            (s, p)
        };
        let run = |s: &mut Session, p: &Value| {
            for _ in 0..20_000 {
                black_box(s.execute(cmd, p).unwrap());
            }
        };
        let ((mut a, pa), (mut b, pb)) = (doc(), doc());
        assert_eq!(uncached(|| a.execute(cmd, &pa).unwrap()), b.execute(cmd, &pb).unwrap(), "{cmd}: both ways give the same result");
        let timed = |s: &mut Session, p: &Value, cached: bool| {
            let t = now(start).0;
            if cached {
                run(s, p)
            } else {
                uncached(|| run(s, p))
            }
            now(start).0 - t
        };
        let (mut parse, mut once) = (Vec::new(), Vec::new());
        for i in 0..7 {
            let (mut s1, p1) = doc();
            let (mut s2, p2) = doc();
            if i % 2 == 0 {
                parse.push(timed(&mut s1, &p1, false));
                once.push(timed(&mut s2, &p2, true));
            } else {
                once.push(timed(&mut s2, &p2, true));
                parse.push(timed(&mut s1, &p1, false));
            }
            // Each document has a `uid` of its own.
            let state = |s: &Session| {
                let mut d = crate::inspect::document(s);
                d.as_object_mut().unwrap().remove("uid");
                d
            };
            assert_eq!(state(&s1), state(&s2), "{cmd}: same document");
        }
        let ms = |ns: u128| ns as f64 / 1e6;
        println!("{cmd} x 20,000: parse {:.1} ms once {:.1} ms", ms(median(parse)), ms(median(once)));
    }
}
