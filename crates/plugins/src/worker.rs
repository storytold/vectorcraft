//! The messages between the web build's page and its plug-in worker, and the worker's side of them
//! ([`Worker`]).
//!
//! The web build checks a module being installed, and runs filters, in a Web Worker of its own: a
//! module that loops in its start function, `vc_manifest` or `vc_run` then leaves the page
//! responsive, and the page ends it by terminating the worker. The page sends [`ToWorker`]
//! messages and reads [`FromWorker`] replies, each one byte buffer ([`ToWorker::encode`],
//! [`FromWorker::decode`]): a little-endian `u32` length, a JSON header of that length, then the
//! header's `blobs`, one after another.
//!
//! The worker posts [`FromWorker::Ready`] once it reads messages. It answers a
//! [`ToWorker::Check`] with the manifest JSON a [`Plugin::load`] of the module read
//! ([`FromWorker::Checked`]), which the page then loads the module with
//! ([`Plugin::load_checked`]), and a [`ToWorker::Run`] with the run's progress
//! ([`FromWorker::Progress`]), its command calls ([`FromWorker::Call`], each answered with
//! [`ToWorker::Answer`] before the run goes on) and its end ([`FromWorker::Done`]). One worker runs
//! one thing at a time; the page starts one per check or run.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::host::{CallFailure, LogLevel, LogLine, Report};
use crate::runtime::{RunJob, Step};
use crate::{Error, Limits, Plugin, Result};

/// The version of these messages, which [`FromWorker::Ready`] gives: a page and a worker of other
/// builds (a worker file a browser kept from before an update) don't run plug-ins together.
pub const PROTOCOL: u64 = 2;

/// What the page asks the worker to do.
#[derive(Clone, Debug, PartialEq)]
pub enum ToWorker {
    /// Load `module` as [`Plugin::load`] does, running its start function, `vc_abi_version` and
    /// `vc_manifest`.
    Check { module: Vec<u8> },
    /// Run filter `module`, which a [`ToWorker::Check`] read `manifest` from, on `input` with
    /// `params` under `limits`. With `calls`, each command call it makes comes back as a
    /// [`FromWorker::Call`].
    Run { module: Vec<u8>, manifest: Vec<u8>, input: Vec<u8>, params: Vec<u8>, limits: Limits, calls: bool },
    /// The answer to the run's last [`FromWorker::Call`]: the command's result, or why it didn't run.
    Answer(std::result::Result<Value, CallFailure>),
}

/// What the worker tells the page.
#[derive(Clone, Debug, PartialEq)]
pub enum FromWorker {
    /// It reads messages now, in [`PROTOCOL`] `protocol`.
    Ready { protocol: u64 },
    /// The module checked: its manifest JSON, or why it doesn't load.
    Checked(Result<Vec<u8>>),
    /// How far the run has got, as the plug-in reports it (`total` 0: not known).
    Progress { done: u64, total: u64 },
    /// The run called command `command` with `args`, and waits for the [`ToWorker::Answer`].
    Call { command: String, args: Value },
    /// The run ended: its output JSON, or why it has none, and the lines it logged.
    Done(Result<Vec<u8>>, Vec<LogLine>),
}

/// `header` with `blobs` after it, as one message.
fn pack(mut header: Value, blobs: &[&[u8]]) -> Vec<u8> {
    header["blobs"] = json!(blobs.iter().map(|b| b.len()).collect::<Vec<_>>());
    let text = header.to_string().into_bytes();
    let mut out = Vec::with_capacity(4 + text.len() + blobs.iter().map(|b| b.len()).sum::<usize>());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(&text);
    for b in blobs {
        out.extend_from_slice(b);
    }
    out
}

/// The header and the blobs of message `msg`; `None` when it isn't one.
fn unpack(msg: &[u8]) -> Option<(Value, Vec<Vec<u8>>)> {
    let len = u32::from_le_bytes(msg.get(..4)?.try_into().ok()?) as usize;
    let header: Value = serde_json::from_slice(msg.get(4..4usize.checked_add(len)?)?).ok()?;
    let mut at = 4 + len;
    let mut blobs = Vec::new();
    for n in header.get("blobs")?.as_array()? {
        let end = at.checked_add(usize::try_from(n.as_u64()?).ok()?)?;
        blobs.push(msg.get(at..end)?.to_vec());
        at = end;
    }
    (at == msg.len()).then_some((header, blobs))
}

/// Error `e` as a header member.
fn error_json(e: &Error) -> Value {
    let (kind, text) = match e {
        Error::Module(t) => ("module", t.as_str()),
        Error::Abi(t) => ("abi", t.as_str()),
        Error::Manifest(t) => ("manifest", t.as_str()),
        Error::Trap(t) => ("trap", t.as_str()),
        Error::Limit(t) => ("limit", t.as_str()),
        Error::Params(t) => ("params", t.as_str()),
        Error::Output(t) => ("output", t.as_str()),
        Error::Failed(t) => ("failed", t.as_str()),
        Error::NotFound(t) => ("notFound", t.as_str()),
        Error::Cancelled => ("cancelled", ""),
        Error::SafeMode => ("safeMode", ""),
        Error::NeedsApproval(t) => ("needsApproval", t.as_str()),
        Error::Io(t) => ("io", t.as_str()),
    };
    json!({ "kind": kind, "text": text })
}

/// The error header member `v` gives ([`error_json`]).
fn error_from(v: &Value) -> Option<Error> {
    let text = v.get("text")?.as_str()?.to_string();
    Some(match v.get("kind")?.as_str()? {
        "module" => Error::Module(text),
        "abi" => Error::Abi(text),
        "manifest" => Error::Manifest(text),
        "trap" => Error::Trap(text),
        "limit" => Error::Limit(text),
        "params" => Error::Params(text),
        "output" => Error::Output(text),
        "failed" => Error::Failed(text),
        "notFound" => Error::NotFound(text),
        "cancelled" => Error::Cancelled,
        "safeMode" => Error::SafeMode,
        "needsApproval" => Error::NeedsApproval(text),
        "io" => Error::Io(text),
        _ => return None,
    })
}

/// Limits `l` as a header member.
fn limits_json(l: &Limits) -> Value {
    json!({
        "maxModuleBytes": l.max_module_bytes,
        "maxMemoryBytes": l.max_memory_bytes,
        "maxOutputBytes": l.max_output_bytes,
        "fuelBase": l.fuel_base,
        "fuelPerByte": l.fuel_per_byte,
        "maxRecursionDepth": l.max_recursion_depth,
        "wallTimeMs": l.wall_time_ms,
        "heavyFuel": l.heavy_fuel,
        "heavyWallTimeMs": l.heavy_wall_time_ms,
        "exportFuel": l.export_fuel,
        "exportWallTimeMs": l.export_wall_time_ms,
    })
}

/// The limits header member `v` gives ([`limits_json`]).
fn limits_from(v: &Value) -> Option<Limits> {
    let n = |k: &str| v.get(k).and_then(Value::as_u64);
    let size = |k: &str| n(k).and_then(|x| usize::try_from(x).ok());
    Some(Limits {
        max_module_bytes: size("maxModuleBytes")?,
        max_memory_bytes: size("maxMemoryBytes")?,
        max_output_bytes: size("maxOutputBytes")?,
        fuel_base: n("fuelBase")?,
        fuel_per_byte: n("fuelPerByte")?,
        max_recursion_depth: size("maxRecursionDepth")?,
        wall_time_ms: n("wallTimeMs")?,
        heavy_fuel: n("heavyFuel")?,
        heavy_wall_time_ms: n("heavyWallTimeMs")?,
        export_fuel: n("exportFuel")?,
        export_wall_time_ms: n("exportWallTimeMs")?,
    })
}

fn level_name(l: LogLevel) -> &'static str {
    match l {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
    }
}

fn level_from(name: &str) -> Option<LogLevel> {
    Some(match name {
        "error" => LogLevel::Error,
        "warn" => LogLevel::Warn,
        "info" => LogLevel::Info,
        "debug" => LogLevel::Debug,
        _ => return None,
    })
}

impl ToWorker {
    /// The message, as the page posts it.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ToWorker::Check { module } => pack(json!({"kind": "check"}), &[module]),
            ToWorker::Run { module, manifest, input, params, limits, calls } => {
                pack(json!({"kind": "run", "limits": limits_json(limits), "calls": calls}), &[module, manifest, input, params])
            }
            // The result goes in a blob of its own, so it may nest as deep as a command's result.
            ToWorker::Answer(Ok(result)) => pack(json!({"kind": "answer"}), &[result.to_string().as_bytes()]),
            ToWorker::Answer(Err(f)) => pack(json!({"kind": "answer", "failure": {"code": f.code, "why": f.why}}), &[]),
        }
    }

    /// The message `msg` holds; `None` when it holds none.
    pub fn decode(msg: &[u8]) -> Option<ToWorker> {
        let (h, mut blobs) = unpack(msg)?;
        match (h.get("kind")?.as_str()?, blobs.len()) {
            ("check", 1) => Some(ToWorker::Check { module: blobs.pop()? }),
            ("run", 4) => {
                let (params, input, manifest, module) = (blobs.pop()?, blobs.pop()?, blobs.pop()?, blobs.pop()?);
                let limits = limits_from(h.get("limits")?)?;
                Some(ToWorker::Run { module, manifest, input, params, limits, calls: h.get("calls")?.as_bool()? })
            }
            ("answer", n) => match (blobs.pop(), h.get("failure")) {
                (Some(result), None) if n == 1 => Some(ToWorker::Answer(Ok(serde_json::from_slice(&result).ok()?))),
                (None, Some(f)) => {
                    let code = i32::try_from(f.get("code")?.as_i64()?).ok()?;
                    Some(ToWorker::Answer(Err(CallFailure { code, why: f.get("why")?.as_str()?.to_string() })))
                }
                _ => None,
            },
            _ => None,
        }
    }
}

impl FromWorker {
    /// The message, as the worker posts it.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            FromWorker::Ready { protocol } => pack(json!({"kind": "ready", "protocol": protocol}), &[]),
            FromWorker::Checked(Ok(manifest)) => pack(json!({"kind": "checked"}), &[manifest]),
            FromWorker::Checked(Err(e)) => pack(json!({"kind": "checked", "error": error_json(e)}), &[]),
            FromWorker::Progress { done, total } => pack(json!({"kind": "progress", "done": done, "total": total}), &[]),
            // The arguments go in a blob of their own, so they may nest as deep as a call's arguments.
            FromWorker::Call { command, args } => pack(json!({"kind": "call", "command": command}), &[args.to_string().as_bytes()]),
            FromWorker::Done(out, lines) => {
                let lines: Vec<Value> = lines.iter().map(|l| json!([level_name(l.level), l.text])).collect();
                match out {
                    Ok(output) => pack(json!({"kind": "done", "lines": lines}), &[output]),
                    Err(e) => pack(json!({"kind": "done", "lines": lines, "error": error_json(e)}), &[]),
                }
            }
        }
    }

    /// The message `msg` holds; `None` when it holds none.
    pub fn decode(msg: &[u8]) -> Option<FromWorker> {
        let (h, mut blobs) = unpack(msg)?;
        let error = h.get("error");
        match (h.get("kind")?.as_str()?, blobs.len(), error) {
            ("ready", 0, None) => Some(FromWorker::Ready { protocol: h.get("protocol")?.as_u64()? }),
            ("checked", 1, None) => Some(FromWorker::Checked(Ok(blobs.pop()?))),
            ("checked", 0, Some(e)) => Some(FromWorker::Checked(Err(error_from(e)?))),
            ("progress", 0, None) => Some(FromWorker::Progress { done: h.get("done")?.as_u64()?, total: h.get("total")?.as_u64()? }),
            ("call", 1, None) => {
                Some(FromWorker::Call { command: h.get("command")?.as_str()?.to_string(), args: serde_json::from_slice(&blobs.pop()?).ok()? })
            }
            ("done", n, e) => {
                let mut lines = Vec::new();
                for l in h.get("lines")?.as_array()? {
                    let (level, text) = (l.get(0)?.as_str()?, l.get(1)?.as_str()?);
                    lines.push(LogLine { level: level_from(level)?, text: text.to_string() });
                }
                let out = match (n, e) {
                    (1, None) => Ok(blobs.pop()?),
                    (0, Some(e)) => Err(error_from(e)?),
                    _ => return None,
                };
                Some(FromWorker::Done(out, lines))
            }
            _ => None,
        }
    }
}

/// Writes the `lines` a run of plug-in `id` logged in a worker to the app's log, as a run on the
/// page writes them.
pub fn log_lines(id: &str, lines: &[LogLine]) {
    for l in lines {
        log::log!(target: "vectorcraft_plugins", l.level.log_level(), "{id}: {}", l.text);
    }
}

/// The worker's side: it reads the page's messages ([`Worker::handle`]) and posts its replies with
/// the function it was made with.
pub struct Worker {
    post: Rc<dyn Fn(Vec<u8>)>,
    /// The run waiting for the answer to a command call, and its plug-in.
    run: Option<(Plugin, RunJob)>,
}

impl Worker {
    /// A worker that posts each reply with `post`.
    pub fn new(post: Rc<dyn Fn(Vec<u8>)>) -> Worker {
        Worker { post, run: None }
    }

    /// Tells the page that it reads messages now ([`FromWorker::Ready`]).
    pub fn ready(&self) {
        (self.post)(FromWorker::Ready { protocol: PROTOCOL }.encode());
    }

    /// Does what message `msg` asks. A message it doesn't read, and an answer when no run waits for
    /// one, end what it is doing with an error.
    pub fn handle(&mut self, msg: &[u8]) {
        match ToWorker::decode(msg) {
            Some(ToWorker::Check { module }) => {
                let checked = Plugin::load(&module, Limits::default()).map(|p| p.manifest_text().to_vec());
                (self.post)(FromWorker::Checked(checked).encode());
            }
            Some(ToWorker::Run { module, manifest, input, params, limits, calls }) => {
                self.run = None;
                let plugin = match Plugin::load_checked(&module, limits.clone(), &manifest) {
                    Ok(p) => p,
                    Err(e) => return self.done(Err(e), Report::default()),
                };
                let (job, step) = self.scoped(|| plugin.begin_run(&input, &params, &limits, calls));
                self.step(plugin, job, step);
            }
            Some(ToWorker::Answer(answer)) => {
                let Some((plugin, mut job)) = self.run.take() else {
                    return self.done(Err(Error::Failed(unexpected())), Report::default());
                };
                let step = self.scoped(|| plugin.resume_run(&mut job, answer));
                self.step(plugin, job, step);
            }
            None => {
                self.run = None;
                self.done(Err(Error::Failed(unexpected())), Report::default());
            }
        }
    }

    /// Runs `f` with the run's progress reports posted as [`FromWorker::Progress`].
    fn scoped<T>(&self, f: impl FnOnce() -> T) -> T {
        let post = Rc::clone(&self.post);
        let sink: vectorcraft_doc::cancel::ProgressSink = Rc::new(move |done, total, _| post(FromWorker::Progress { done, total }.encode()));
        vectorcraft_doc::cancel::scope(&vectorcraft_doc::cancel::CancelToken::new(), Some(sink), f)
    }

    /// Posts where the run of `plugin` is: a command call (keeping the run to go on with), or its end.
    fn step(&mut self, plugin: Plugin, job: RunJob, step: Step) {
        match step {
            Step::Call { command, args } => {
                (self.post)(FromWorker::Call { command, args }.encode());
                self.run = Some((plugin, job));
            }
            Step::Done(out, report) => self.done(out, report),
        }
    }

    fn done(&self, out: Result<Vec<u8>>, report: Report) {
        (self.post)(FromWorker::Done(out, report.lines).encode());
    }
}

/// Why the worker, or the page, ends what it is doing when a message makes no sense where it is: a
/// bug, as a thread that stops is.
const UNEXPECTED: &str = "internal error: a message between the page and the plug-in worker came out of turn (please report this bug)";

/// [`UNEXPECTED`], for an error.
pub fn unexpected() -> String {
    UNEXPECTED.to_string()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::wat as tpl;

    const INPUT: &str = r#"{"objects":[]}"#;

    /// A worker whose replies are kept, decoded, in the cell it gives.
    fn worker() -> (Worker, Rc<RefCell<Vec<FromWorker>>>) {
        let got = Rc::new(RefCell::new(Vec::new()));
        let kept = Rc::clone(&got);
        let w = Worker::new(Rc::new(move |m: Vec<u8>| kept.borrow_mut().push(FromWorker::decode(&m).expect("a reply the page reads"))));
        (w, got)
    }

    fn take(got: &Rc<RefCell<Vec<FromWorker>>>) -> Vec<FromWorker> {
        std::mem::take(&mut *got.borrow_mut())
    }

    /// A filter that calls `select.all` with `{"a":1}` and returns the call's result.
    fn caller() -> Vec<u8> {
        let calls = r#"(import "env" "vc_call" (func $vc_call (param i32 i32 i32 i32) (result i32)))
  (import "env" "vc_call_result" (func $vc_call_result (param i32 i32) (result i32)))
  (data (i32.const 0x2000) "select.all")
  (data (i32.const 0x2100) "{\22a\22:1}")"#;
        let manifest = r#"{"id":"test.worker.call","name":"Caller","kind":"filter","api":2,"permissions":["doc.read","doc.write","commands"],"commandAllowlist":["select.all"]}"#;
        let run = "(if (i32.ne (call $vc_call (i32.const 0x2000) (i32.const 10) (i32.const 0x2100) (i32.const 7)) (i32.const 0)) (then unreachable)) \
                   (i64.or (i64.shl (i64.extend_i32_u (call $vc_call_result (i32.const 0x3000) (i32.const 64))) (i64.const 32)) (i64.const 0x3000))";
        wat::parse_str(tpl::module(manifest, run, calls, 1)).unwrap()
    }

    fn checked(w: &mut Worker, got: &Rc<RefCell<Vec<FromWorker>>>, module: &[u8]) -> Vec<u8> {
        w.handle(&ToWorker::Check { module: module.to_vec() }.encode());
        match take(got).as_slice() {
            [FromWorker::Checked(Ok(manifest))] => manifest.clone(),
            other => panic!("{other:?}"),
        }
    }

    fn run_msg(module: &[u8], manifest: &[u8], calls: bool) -> Vec<u8> {
        let limits = Limits::default();
        ToWorker::Run { module: module.to_vec(), manifest: manifest.to_vec(), input: INPUT.into(), params: b"{}".to_vec(), limits, calls }.encode()
    }

    #[test]
    fn every_message_reads_back_as_it_was_sent() {
        let to = [
            ToWorker::Check { module: b"\0asm".to_vec() },
            ToWorker::Run {
                module: vec![1, 2],
                manifest: b"{}".to_vec(),
                input: vec![],
                params: vec![3],
                limits: Limits { wall_time_ms: 7, ..Limits::default() },
                calls: true,
            },
            ToWorker::Answer(Ok(json!({"n": [1, "two"]}))),
            ToWorker::Answer(Err(CallFailure { code: -3, why: "no".into() })),
        ];
        for m in to {
            assert_eq!(ToWorker::decode(&m.encode()), Some(m));
        }
        let line = LogLine { level: LogLevel::Warn, text: "careful".into() };
        let from = [
            FromWorker::Ready { protocol: PROTOCOL },
            FromWorker::Checked(Ok(b"{\"id\":\"x\"}".to_vec())),
            FromWorker::Checked(Err(Error::Module("bad".into()))),
            FromWorker::Progress { done: 3, total: 0 },
            FromWorker::Call { command: "select.all".into(), args: json!({}) },
            FromWorker::Done(Ok(vec![]), vec![line.clone()]),
            FromWorker::Done(Err(Error::Cancelled), vec![]),
            FromWorker::Done(Err(Error::Limit("time budget".into())), vec![line]),
        ];
        for m in from {
            assert_eq!(FromWorker::decode(&m.encode()), Some(m));
        }
    }

    /// A call's arguments and a command's result read back at the deepest nesting a JSON parse
    /// takes: the message adds no level around them.
    #[test]
    fn arguments_and_results_as_deep_as_json_takes_read_back() {
        let nested = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        let deepest = (1..512).take_while(|&n| serde_json::from_str::<Value>(&nested(n)).is_ok()).last().unwrap();
        assert!(deepest < 511, "the parse has a depth limit");
        let value: Value = serde_json::from_str(&nested(deepest)).unwrap();
        let call = FromWorker::Call { command: "select.all".into(), args: value.clone() };
        assert_eq!(FromWorker::decode(&call.encode()), Some(call));
        let answer = ToWorker::Answer(Ok(value));
        assert_eq!(ToWorker::decode(&answer.encode()), Some(answer));
    }

    #[test]
    fn a_cut_or_padded_message_is_not_read() {
        let m = ToWorker::Check { module: b"\0asm".to_vec() }.encode();
        assert_eq!(ToWorker::decode(&m[..m.len() - 1]), None);
        assert_eq!(ToWorker::decode(&[m.as_slice(), &[0]].concat()), None);
        assert_eq!(ToWorker::decode(&[]), None);
        assert_eq!(FromWorker::decode(&m), None, "a request is not a reply");
    }

    #[test]
    fn a_checked_module_loads_on_the_page_without_running_its_code() {
        let (mut w, got) = worker();
        w.ready();
        assert_eq!(take(&got), [FromWorker::Ready { protocol: PROTOCOL }]);
        let module = caller();
        let manifest = checked(&mut w, &got, &module);
        let page = Plugin::load_checked(&module, Limits::default(), &manifest).unwrap();
        assert_eq!(page.manifest(), Plugin::load(&module, Limits::default()).unwrap().manifest());
        // A module that traps before its manifest can be read is refused in the worker.
        let trapping =
            wat::parse_str(tpl::module(&tpl::manifest("test.worker.trap", "filter", "{}"), tpl::ECHO, "(start $s) (func $s unreachable)", 1))
                .unwrap();
        w.handle(&ToWorker::Check { module: trapping }.encode());
        assert!(matches!(take(&got).as_slice(), [FromWorker::Checked(Err(Error::Trap(_)))]));
    }

    /// A module that carries its manifest in its custom section, with neither `vc_abi_version` nor
    /// `vc_manifest`, is checked in a worker and loads on the page from that section.
    #[test]
    fn a_module_with_a_manifest_section_loads_on_the_page_from_its_section() {
        let (mut w, got) = worker();
        let section = r#"{"id":"test.worker.section","name":"Sectioned","version":"1","kind":"filter","abi":1}"#;
        let module = wat::parse_str(tpl::sectioned(section, tpl::ECHO, "", 1)).unwrap();
        let manifest = checked(&mut w, &got, &module);
        assert_eq!(manifest, section.as_bytes(), "the worker sends the section's manifest");
        let page = Plugin::load_checked(&module, Limits::default(), &manifest).unwrap();
        assert_eq!(page.manifest(), Plugin::load(&module, Limits::default()).unwrap().manifest());
    }

    #[test]
    fn a_page_load_checks_the_manifest_it_is_given() {
        let module = caller();
        // The manifest lists `commands`, which the module's imports need; one without it is refused,
        // as `Plugin::load` refuses it.
        let without = r#"{"id":"test.worker.call","name":"Caller","kind":"filter","api":2,"permissions":["doc.read"]}"#;
        assert!(Plugin::load_checked(&module, Limits::default(), without.as_bytes()).is_err());
        assert!(Plugin::load_checked(&module, Limits::default(), b"not json").is_err());
        let no_run = wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "vc_abi_version") (result i32) (i32.const 1)))"#).unwrap();
        assert!(matches!(Plugin::load_checked(&no_run, Limits::default(), b"{}"), Err(Error::Abi(_))));
    }

    #[test]
    fn a_run_comes_back_for_each_command_call_and_ends_with_its_output() {
        let (mut w, got) = worker();
        let module = caller();
        let manifest = checked(&mut w, &got, &module);
        w.handle(&run_msg(&module, &manifest, true));
        assert_eq!(take(&got), [FromWorker::Call { command: "select.all".into(), args: json!({"a": 1}) }]);
        w.handle(&ToWorker::Answer(Ok(json!({"n": 1}))).encode());
        assert_eq!(take(&got), [FromWorker::Done(Ok(br#"{"n":1}"#.to_vec()), vec![])]);
        // The output equals what a run on the page with the same answer returns.
        let page = Plugin::load(&module, Limits::default()).unwrap();
        let mut bridge = |_: &str, _: &Value| Ok(json!({"n": 1}));
        assert_eq!(page.run_calling(INPUT.as_bytes(), b"{}", page.limits(), &mut bridge).0.unwrap(), br#"{"n":1}"#);
    }

    /// A run in the worker keeps to the memory limit the page sends with it, as a run on the page
    /// with those limits does.
    #[test]
    fn a_run_keeps_to_the_memory_limit_it_is_sent() {
        let (mut w, got) = worker();
        let module = wat::parse_str(tpl::module(&tpl::manifest("test.worker.memory", "filter", "{}"), tpl::ECHO, "", 2)).unwrap();
        let manifest = checked(&mut w, &got, &module);
        let limits = Limits { max_memory_bytes: 1 << 16, ..Limits::default() };
        let msg = ToWorker::Run { module, manifest, input: INPUT.into(), params: b"{}".to_vec(), limits, calls: false };
        w.handle(&msg.encode());
        assert!(matches!(take(&got).as_slice(), [FromWorker::Done(Err(_), _)]), "two pages don't fit in one");
    }

    #[test]
    fn a_refused_call_fails_the_run_with_the_reason() {
        let (mut w, got) = worker();
        let module = caller();
        let manifest = checked(&mut w, &got, &module);
        w.handle(&run_msg(&module, &manifest, true));
        take(&got);
        w.handle(&ToWorker::Answer(Err(CallFailure { code: crate::host::CALL_COMMAND_FAILED, why: "nope".into() })).encode());
        match take(&got).as_slice() {
            [FromWorker::Done(Err(Error::Failed(why)), _)] => assert!(why.contains("nope"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_answer_no_run_waits_for_and_an_unread_message_end_with_an_error() {
        let (mut w, got) = worker();
        w.handle(&ToWorker::Answer(Ok(json!(1))).encode());
        assert!(matches!(take(&got).as_slice(), [FromWorker::Done(Err(Error::Failed(_)), _)]));
        let module = caller();
        let manifest = checked(&mut w, &got, &module);
        w.handle(&run_msg(&module, &manifest, true));
        take(&got);
        w.handle(b"junk");
        assert!(matches!(take(&got).as_slice(), [FromWorker::Done(Err(Error::Failed(_)), _)]));
        // The run that waited is gone: a late answer finds none.
        w.handle(&ToWorker::Answer(Ok(json!(1))).encode());
        assert!(matches!(take(&got).as_slice(), [FromWorker::Done(Err(Error::Failed(_)), _)]));
    }

    #[test]
    fn progress_is_posted_while_the_run_computes() {
        let (mut w, got) = worker();
        let manifest = r#"{"id":"test.worker.progress","name":"P","kind":"filter","api":2,"permissions":["doc.read","progress"]}"#;
        let first = "(call $vc_progress (i64.const 1) (i64.const 4)) (call $vc_progress (i64.const 3) (i64.const 4))";
        let module = wat::parse_str(tpl::with_imports(manifest, tpl::HOST_IMPORTS, first, INPUT)).unwrap();
        let text = checked(&mut w, &got, &module);
        w.handle(&run_msg(&module, &text, false));
        let replies = take(&got);
        let shares: Vec<_> =
            replies.iter().filter_map(|r| if let FromWorker::Progress { done, total } = r { Some((*done, *total)) } else { None }).collect();
        assert!(shares.len() >= 2 && shares.windows(2).all(|w| w[0].0 < w[1].0), "{replies:?}");
        assert!(matches!(replies.last(), Some(FromWorker::Done(Ok(_), _))), "{replies:?}");
    }
}
