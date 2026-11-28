//! `Promise` (ECMA-262 11th edition, section 25.6, <https://262.ecma-international.org/11.0/#sec-promise-objects>)
//! and the run's queue of promise jobs (section 8.4), in realms with [`crate::Options::es2015`]:
//! the constructor, `then`, `catch` and `finally`, and `Promise.resolve`, `reject`, `all`,
//! `allSettled` and `race`. `Promise.any` (ES2021) is not provided.
//!
//! A job is queued when a promise settles with reactions waiting, when `then` is called on a
//! settled promise, and when a promise is resolved with a thenable. Jobs run one at a time, in the
//! order they were queued, once the script's own code has ended and no other code runs: at the end
//! of [`crate::Realm::run`], and in the slices [`crate::Realm::advance`] runs after the script's
//! code, before it reports the run done. A job that a job queues joins the end of the queue. A job
//! takes a unit of fuel, its callback runs on the run's fuel, heap and frame limits, and a callback
//! written in JavaScript runs as the script's own code, so a slice can pause inside it. Each queued
//! job counts [`JOB_BYTES`] toward the heap limit, and more than [`MAX_JOBS`] queued jobs end the
//! run with [`LimitKind::Jobs`]. A run that ends with an error or a limit, or is ended, drops the
//! jobs still queued. A promise that is still pending when the queue is empty stays pending.
//!
//! A promise rejected while it has no handler is noted (HostPromiseRejectionTracker, section
//! 25.6.1.9); one that still has none when the run ends successfully is reported by
//! [`crate::Realm::unhandled_rejections`], with the place it was rejected at when known.
//!
//! `all`, `allSettled` and `race` read their input through `Symbol.iterator` and close the iterator
//! when processing a yielded value throws. Noniterable values reject the promise they return
//! with a `TypeError`. The statics need a constructor as `this`, and the
//! promises they and `then` make are always built-in promises: subclasses and `Symbol.species`
//! are not read.

use std::collections::VecDeque;
use std::rc::Rc;

use super::{arg, constructor, create_at, method, native, new_array};
use crate::LimitKind;
use crate::bytecode::Code;
use crate::heap::{
    BOUND_ARG_BYTES, CONFIGURABLE, Class, Func, NativeFn, NativeKind, OBJECT_BYTES, OPEN, ObjId, Object, PROP_BYTES, Prop, Roots, key,
};
use crate::host::ArgRead;
use crate::interp::{Abrupt, ErrorType, Interp, JsResult};
use crate::value::Value;

/// Where a promise was rejected: the code and the instruction.
pub(crate) type Site = (Rc<Code>, usize);

/// The estimated bytes of a promise's state, without its reactions.
pub(crate) const PROMISE_BYTES: usize = 64;
/// The estimated bytes of a reaction a pending promise holds.
pub(crate) const REACTION_BYTES: usize = 48;
/// The estimated bytes of the state a promise operation's functions share.
pub(crate) const RECORD_BYTES: usize = 48;
/// The estimated bytes of a queued job.
pub(crate) const JOB_BYTES: usize = 64;
/// The most jobs the queue may hold at once.
pub(crate) const MAX_JOBS: usize = 1 << 20;
/// The most unhandled rejections a run reports one by one.
pub(crate) const MAX_REPORTED: usize = 20;
/// The most characters of a rejection's reason a report gives.
const MAX_REASON_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Pending,
    Fulfilled,
    Rejected,
}

/// A `then` waiting for a pending promise: the handlers (`undefined` when not callable) and the
/// promise `then` returned.
#[derive(Clone)]
pub(crate) struct Reaction {
    on_fulfilled: Value,
    on_rejected: Value,
    derived: ObjId,
}

/// A promise's internal slots (section 25.6.6), with where it was rejected.
#[derive(Clone)]
pub(crate) struct PromiseData {
    pub(crate) state: State,
    pub(crate) result: Value,
    reactions: Vec<Reaction>,
    pub(crate) handled: bool,
    pub(crate) site: Option<Site>,
}

impl PromiseData {
    fn new() -> Self {
        Self { state: State::Pending, result: Value::Undefined, reactions: Vec::new(), handled: false, site: None }
    }

    /// The values the promise holds, for the collector.
    pub(crate) fn values(&self) -> Vec<Value> {
        let mut out = vec![self.result.clone()];
        for r in &self.reactions {
            out.extend([r.on_fulfilled.clone(), r.on_rejected.clone(), Value::Object(r.derived)]);
        }
        out
    }

    /// The estimated bytes of the state, beyond the object.
    pub(crate) fn bytes(&self) -> usize {
        PROMISE_BYTES + REACTION_BYTES * self.reactions.len()
    }
}

/// The state the functions a promise operation makes share.
#[derive(Clone)]
pub(crate) enum Record {
    /// A promise's resolve and reject functions (section 25.6.1.3): the first call wins.
    Resolving { promise: ObjId, done: bool },
    /// `Promise.all` and `allSettled`: the result's resolving record, the values so far and the
    /// inputs not yet settled.
    Combinator { resolving: ObjId, values: ObjId, remaining: u64 },
    /// The functions for one input of `Promise.all` or `allSettled`.
    Element { combinator: ObjId, index: u32, called: bool },
    /// `finally`'s callback.
    Finally { on_finally: Value },
    /// A value `finally` passes on or throws again.
    Thunk { value: Value },
}

impl Record {
    /// The values the record holds, for the collector.
    pub(crate) fn values(&self) -> Vec<Value> {
        match self {
            Record::Resolving { promise, .. } => vec![Value::Object(*promise)],
            Record::Combinator { resolving, values, .. } => vec![Value::Object(*resolving), Value::Object(*values)],
            Record::Element { combinator, .. } => vec![Value::Object(*combinator)],
            Record::Finally { on_finally: v } | Record::Thunk { value: v } => vec![v.clone()],
        }
    }
}

/// A queued job.
enum Job {
    /// PromiseReactionJob (section 25.6.2.1): calls `handler` with `argument` and settles
    /// `derived` with the result, or passes `argument` on when `handler` is not callable.
    Reaction { handler: Value, argument: Value, reject: bool, derived: ObjId, site: Option<Site> },
    /// PromiseResolveThenableJob (section 25.6.2.2): calls `then` on `thenable` with new
    /// resolving functions for `promise`.
    Thenable { promise: ObjId, thenable: Value, then: Value },
}

/// What happens when the running job's call ends.
enum Then {
    /// The result resolves this promise, and an exception rejects it.
    Settle(ObjId, Option<Site>),
    /// An exception rejects through this resolving record.
    Reject(ObjId),
    /// The call was a `finally` callback that a reaction job runs in place of the Then Finally or
    /// Catch Finally function: `derived` is settled as that function's result settles it.
    Finally { derived: ObjId, value: Value, throws: bool },
}

/// The built-in functions a promise operation's functions call, with their record.
#[derive(Clone, Copy)]
enum Kind {
    Resolve,
    Reject,
    AllElement,
    SettledFulfilled,
    SettledRejected,
    ThenFinally,
    CatchFinally,
    ValueThunk,
    Thrower,
}

const KINDS: [(Kind, NativeFn); 9] = [
    (Kind::Resolve, resolve_function),
    (Kind::Reject, reject_function),
    (Kind::AllElement, all_element),
    (Kind::SettledFulfilled, settled_fulfilled),
    (Kind::SettledRejected, settled_rejected),
    (Kind::ThenFinally, then_finally),
    (Kind::CatchFinally, catch_finally),
    (Kind::ValueThunk, value_thunk),
    (Kind::Thrower, thrower),
];

/// `Promise`'s objects and the run's job queue.
pub(crate) struct Promises {
    proto: Option<ObjId>,
    ctor: Option<ObjId>,
    /// The built-ins for each [`Kind`], in [`KINDS`] order.
    natives: Vec<ObjId>,
    queue: VecDeque<Job>,
    running: Option<Then>,
    /// The script's completion value while the jobs after it run.
    completion: Option<Value>,
    /// The script's own code has ended and the queue's jobs are running.
    pub(crate) draining: bool,
    /// Promises rejected in this run while they had no handler.
    rejected: Vec<ObjId>,
    /// The most jobs the queue may hold ([`MAX_JOBS`]; tests set it lower).
    pub(crate) max_jobs: usize,
}

impl Default for Promises {
    fn default() -> Self {
        Self {
            proto: None,
            ctor: None,
            natives: Vec::new(),
            queue: VecDeque::new(),
            running: None,
            completion: None,
            draining: false,
            rejected: Vec::new(),
            max_jobs: MAX_JOBS,
        }
    }
}

impl Promises {
    /// Adds what the queue and the promise objects hold to the collector's roots.
    pub(crate) fn add_roots<'a>(&'a self, roots: &mut Roots<'a>) {
        roots.objects.extend(self.proto.iter().chain(&self.ctor).chain(&self.natives).chain(&self.rejected));
        roots.values.extend(self.completion.iter());
        match &self.running {
            Some(Then::Settle(o, site)) => {
                roots.objects.push(*o);
                roots.codes.extend(site.iter().map(|(code, _)| &**code));
            }
            Some(Then::Reject(o)) => roots.objects.push(*o),
            Some(Then::Finally { derived, value, .. }) => {
                roots.objects.push(*derived);
                roots.values.push(value);
            }
            None => {}
        }
        for job in &self.queue {
            match job {
                Job::Reaction { handler, argument, derived, site, .. } => {
                    roots.values.extend([handler, argument]);
                    roots.objects.push(*derived);
                    roots.codes.extend(site.iter().map(|(code, _)| &**code));
                }
                Job::Thenable { promise, thenable, then } => {
                    roots.values.extend([thenable, then]);
                    roots.objects.push(*promise);
                }
            }
        }
    }

    /// The estimated bytes the queued jobs hold.
    pub(crate) fn queue_bytes(&self) -> usize {
        JOB_BYTES.saturating_mul(self.queue.len())
    }

    /// How many jobs are queued, and how many the queue's buffer has room for.
    #[cfg(test)]
    pub(crate) fn queue_size(&self) -> (usize, usize) {
        (self.queue.len(), self.queue.capacity())
    }

    /// Drops the noted rejections whose promise has a handler now.
    pub(crate) fn forget_handled(&mut self, heap: &crate::heap::Heap) {
        self.rejected.retain(|p| matches!(heap.object(*p).map(|o| &o.class), Some(Class::Promise(d)) if !d.handled));
    }
}

/// Creates `Promise`, its prototype methods and its statics.
pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let object_proto = interp.intrinsics.object_proto;
    let proto = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
    interp.promises.proto = Some(proto);
    if let Some(tag) = interp.well_known(|w| &w.to_string_tag) {
        interp.define_sym(proto, tag, Prop::Data { value: Value::str("Promise"), flags: CONFIGURABLE })?;
    }
    let ctor = native(interp, "Promise", promise_ctor, 1, NativeKind::Plain, true)?;
    interp.promises.ctor = Some(ctor);
    constructor(interp, "Promise", ctor, proto)?;
    method(interp, proto, "then", promise_then, 2)?;
    method(interp, proto, "catch", promise_catch, 1)?;
    method(interp, proto, "finally", promise_finally, 1)?;
    method(interp, ctor, "resolve", promise_resolve_static, 1)?;
    method(interp, ctor, "reject", promise_reject_static, 1)?;
    method(interp, ctor, "all", promise_all, 1)?;
    method(interp, ctor, "allSettled", promise_all_settled, 1)?;
    method(interp, ctor, "race", promise_race, 1)?;
    for (_, f) in KINDS {
        let n = native(interp, "", f, 1, NativeKind::Plain, false)?;
        interp.promises.natives.push(n);
    }
    Ok(())
}

/// Creates `Promise` in a realm made without [`crate::Options::es2015`], replacing a global
/// `Promise` an earlier script defined, and leaves the language the realm's runs read as it is. A
/// realm that has the built-in `Promise` already keeps it. When `Promise` cannot be created, every
/// later run of the realm ends with an internal error, as for a realm whose built-ins could not be
/// installed.
pub(crate) fn add_to(interp: &mut Interp) {
    if interp.promises.ctor.is_some() || interp.install_error.is_some() {
        return;
    }
    if let Err(e) = install(interp) {
        let reason = match e {
            Abrupt::Throw(v) => interp.describe(&v),
            Abrupt::Limit(kind) => format!("limit reached: {kind:?}"),
            Abrupt::Internal(message) => message,
        };
        interp.install_error = Some(format!("the built-ins could not be installed: {reason}"));
    }
}

// ---- helpers ----

/// Runs `f`, then drops the temporary roots it added.
fn scoped<T>(interp: &mut Interp, f: impl FnOnce(&mut Interp) -> JsResult<T>) -> JsResult<T> {
    let mark = interp.temp_roots.len();
    let r = f(interp);
    interp.temp_roots.truncate(mark);
    r
}

fn root(interp: &mut Interp, v: &Value) {
    interp.temp_roots.push(v.clone());
}

/// An exception that a promise operation turns into a rejection: it is no longer propagating, and
/// the place it was thrown at, when known, is the rejection's.
fn caught(interp: &mut Interp) -> Option<Site> {
    interp.passing = Value::Undefined;
    interp.thrown_mark = None;
    interp.throw_site.take()
}

/// The instruction the innermost script frame is running.
fn here(interp: &Interp) -> Option<Site> {
    interp.frames.last().map(|f| (f.code.clone(), f.pc.saturating_sub(1)))
}

/// The value an error of type `t` with `message` is.
fn error_value(interp: &mut Interp, t: ErrorType, message: &str) -> Value {
    match interp.throw(t, message) {
        Abrupt::Throw(v) => v,
        _ => Value::Undefined,
    }
}

fn promise_data(interp: &mut Interp, p: ObjId) -> JsResult<&mut PromiseData> {
    match interp.heap.object_mut(p).map(|o| &mut o.class) {
        Some(Class::Promise(d)) => Ok(d),
        _ => Err(Abrupt::Internal("a promise operation lost its promise".into())),
    }
}

fn record<'a>(interp: &'a mut Interp, r: &Value) -> JsResult<&'a mut Record> {
    match r.as_object().and_then(|o| interp.heap.object_mut(o)).map(|o| &mut o.class) {
        Some(Class::PromiseRecord(rec)) => Ok(rec),
        _ => Err(Abrupt::Internal("a promise function lost its record".into())),
    }
}

fn is_promise(interp: &Interp, v: &Value) -> bool {
    v.as_object().and_then(|o| interp.heap.object(o)).is_some_and(|o| matches!(o.class, Class::Promise(_)))
}

/// `this` for a `Promise.prototype` method that needs a promise.
fn this_promise(interp: &mut Interp, this: &Value, name: &str) -> JsResult<ObjId> {
    match this.as_object() {
        Some(o) if is_promise(interp, this) => Ok(o),
        _ => Err(interp.type_error(&format!("Promise.prototype.{name} was called on a value that is not a promise"))),
    }
}

/// Invoke (section 7.3.18): calls `v[name]` with `v` as `this`.
fn invoke(interp: &mut Interp, v: &Value, name: &str, args: &[Value]) -> JsResult<Value> {
    let f = interp.get_value(v, &key(name))?;
    if !interp.is_callable(&f) {
        return Err(interp.type_error(&format!("{name} is not a function")));
    }
    interp.call(&f, v, args)
}

/// A new pending promise.
fn new_promise(interp: &mut Interp) -> JsResult<ObjId> {
    interp.reserve_heap(OBJECT_BYTES + PROMISE_BYTES)?;
    let proto = interp.promises.proto;
    let o = interp.heap.alloc_object(Object::new(Class::Promise(Box::new(PromiseData::new())), proto));
    interp.heap.bytes = interp.heap.bytes.saturating_add(PROMISE_BYTES);
    Ok(o)
}

/// A new record. The values it holds must be rooted.
fn new_record(interp: &mut Interp, rec: Record) -> JsResult<ObjId> {
    interp.reserve_heap(OBJECT_BYTES + RECORD_BYTES)?;
    let o = interp.heap.alloc_object(Object::new(Class::PromiseRecord(Box::new(rec)), None));
    interp.heap.bytes = interp.heap.bytes.saturating_add(RECORD_BYTES);
    Ok(o)
}

/// A function that calls the built-in for `kind` with `rec` before its own arguments. `rec` must
/// be rooted.
fn record_function(interp: &mut Interp, kind: Kind, rec: ObjId, length: u32) -> JsResult<Value> {
    let Some(&target) = interp.promises.natives.get(kind as usize) else { return Err(Abrupt::Internal("Promise is not installed".into())) };
    interp.reserve_heap(OBJECT_BYTES + PROP_BYTES + BOUND_ARG_BYTES + 16)?;
    let proto = interp.intrinsics.function_proto;
    let args: Rc<[Value]> = Rc::from([Value::Object(rec)]);
    let f = interp.heap.alloc_object(Object::new(Class::Function(Func::Bound { target, this: Value::Undefined, args }), Some(proto)));
    interp.heap.bytes = interp.heap.bytes.saturating_add(BOUND_ARG_BYTES);
    interp.define(f, key("length"), Prop::Data { value: Value::Number(f64::from(length)), flags: 0 })?;
    Ok(Value::Object(f))
}

/// CreateResolvingFunctions (section 25.6.1.3): the shared record, the resolve function and the
/// reject function. `promise` must be rooted.
fn resolving_functions(interp: &mut Interp, promise: ObjId) -> JsResult<(ObjId, Value, Value)> {
    scoped(interp, |interp| {
        let rec = new_record(interp, Record::Resolving { promise, done: false })?;
        root(interp, &Value::Object(rec));
        let resolve = record_function(interp, Kind::Resolve, rec, 1)?;
        root(interp, &resolve);
        let reject = record_function(interp, Kind::Reject, rec, 1)?;
        Ok((rec, resolve, reject))
    })
}

/// Checks that `n` more jobs fit in the queue and in the heap. Everything the caller still needs
/// must be reachable from the roots.
fn reserve_jobs(interp: &mut Interp, n: usize) -> JsResult<()> {
    if interp.promises.queue.len().saturating_add(n) > interp.promises.max_jobs {
        return Err(Abrupt::Limit(LimitKind::Jobs));
    }
    interp.reserve_heap(JOB_BYTES.saturating_mul(n))
}

/// Queues a job [`reserve_jobs`] made room for.
fn push_job(interp: &mut Interp, job: Job) {
    interp.heap.bytes = interp.heap.bytes.saturating_add(JOB_BYTES);
    interp.promises.queue.push_back(job);
}

/// Settles pending promise `p` (FulfillPromise and RejectPromise, sections 25.6.1.4 and 25.6.1.7)
/// and queues a job for each reaction. `p` and `value` must be rooted.
fn settle(interp: &mut Interp, p: ObjId, fulfilled: bool, value: Value, site: Option<Site>) -> JsResult<()> {
    let waiting = promise_data(interp, p)?.reactions.len();
    reserve_jobs(interp, waiting)?;
    let data = promise_data(interp, p)?;
    if data.state != State::Pending {
        return Ok(());
    }
    data.state = if fulfilled { State::Fulfilled } else { State::Rejected };
    data.result = value.clone();
    if !fulfilled {
        data.site.clone_from(&site);
    }
    let reactions = std::mem::take(&mut data.reactions);
    let handled = data.handled;
    interp.heap.bytes = interp.heap.bytes.saturating_sub(REACTION_BYTES.saturating_mul(reactions.len()));
    for r in reactions {
        let handler = if fulfilled { r.on_fulfilled } else { r.on_rejected };
        push_job(interp, Job::Reaction { handler, argument: value.clone(), reject: !fulfilled, derived: r.derived, site: site.clone() });
    }
    if !fulfilled && !handled {
        interp.promises.rejected.push(p);
    }
    Ok(())
}

fn reject_promise(interp: &mut Interp, p: ObjId, reason: Value, site: Option<Site>) -> JsResult<()> {
    scoped(interp, |interp| {
        root(interp, &Value::Object(p));
        root(interp, &reason);
        settle(interp, p, false, reason, site)
    })
}

/// A resolve function's steps after its first-call check (section 25.6.1.3.2): `resolution`
/// fulfills `p`, unless it is `p` itself (a `TypeError` rejects it) or an object whose `then` is
/// callable (a job adopts its state), and an exception reading `then` rejects it.
fn resolve_promise(interp: &mut Interp, p: ObjId, resolution: Value) -> JsResult<()> {
    scoped(interp, |interp| {
        root(interp, &Value::Object(p));
        root(interp, &resolution);
        let Value::Object(o) = resolution else { return settle(interp, p, true, resolution, None) };
        if o == p {
            let e = error_value(interp, ErrorType::Type, "a promise cannot be resolved with itself");
            let site = here(interp);
            return reject_promise(interp, p, e, site);
        }
        let then = match interp.get_from(o, &key("then"), &resolution) {
            Ok(t) => t,
            Err(Abrupt::Throw(e)) => {
                let site = caught(interp);
                root(interp, &e);
                return settle(interp, p, false, e, site);
            }
            Err(e) => return Err(e),
        };
        if !interp.is_callable(&then) {
            return settle(interp, p, true, resolution, None);
        }
        root(interp, &then);
        reserve_jobs(interp, 1)?;
        push_job(interp, Job::Thenable { promise: p, thenable: resolution, then });
        Ok(())
    })
}

/// The resolve function of the resolving record `rec`.
fn resolve_through(interp: &mut Interp, rec: ObjId, value: Value) -> JsResult<()> {
    let Record::Resolving { promise, done } = record(interp, &Value::Object(rec))? else {
        return Err(Abrupt::Internal("not a resolving record".into()));
    };
    if *done {
        return Ok(());
    }
    *done = true;
    let p = *promise;
    resolve_promise(interp, p, value)
}

/// The reject function of the resolving record `rec`.
fn reject_through(interp: &mut Interp, rec: ObjId, reason: Value, site: Option<Site>) -> JsResult<()> {
    let Record::Resolving { promise, done } = record(interp, &Value::Object(rec))? else {
        return Err(Abrupt::Internal("not a resolving record".into()));
    };
    if *done {
        return Ok(());
    }
    *done = true;
    let p = *promise;
    reject_promise(interp, p, reason, site)
}

/// PerformPromiseThen (section 25.6.5.4.1) with `derived` as the result capability's promise.
/// Every value must be rooted.
fn perform_then(interp: &mut Interp, p: ObjId, on_fulfilled: Value, on_rejected: Value, derived: ObjId) -> JsResult<()> {
    let on_fulfilled = if interp.is_callable(&on_fulfilled) { on_fulfilled } else { Value::Undefined };
    let on_rejected = if interp.is_callable(&on_rejected) { on_rejected } else { Value::Undefined };
    let state = promise_data(interp, p)?.state;
    if state == State::Pending {
        interp.reserve_heap(REACTION_BYTES)?;
        interp.heap.bytes = interp.heap.bytes.saturating_add(REACTION_BYTES);
    } else {
        reserve_jobs(interp, 1)?;
    }
    let data = promise_data(interp, p)?;
    data.handled = true;
    match data.state {
        State::Pending => {
            data.reactions.push(Reaction { on_fulfilled, on_rejected, derived });
        }
        State::Fulfilled => {
            let argument = data.result.clone();
            push_job(interp, Job::Reaction { handler: on_fulfilled, argument, reject: false, derived, site: None });
        }
        State::Rejected => {
            let (argument, site) = (data.result.clone(), data.site.clone());
            push_job(interp, Job::Reaction { handler: on_rejected, argument, reject: true, derived, site });
        }
    }
    Ok(())
}

/// PromiseResolve (section 25.6.4.5.1) with `c` as the constructor: `x` when it is a promise whose
/// `constructor` is `c`, otherwise a new promise resolved with `x`.
fn promise_resolve(interp: &mut Interp, c: &Value, x: Value) -> JsResult<Value> {
    scoped(interp, |interp| {
        root(interp, &x);
        if let Value::Object(o) = x
            && is_promise(interp, &x)
        {
            let ctor = interp.get_from(o, &key("constructor"), &x)?;
            if crate::interp::same_value(&ctor, c) {
                return Ok(x);
            }
        }
        let p = new_promise(interp)?;
        root(interp, &Value::Object(p));
        resolve_promise(interp, p, x)?;
        Ok(Value::Object(p))
    })
}

// ---- the job queue ----

impl Interp {
    /// A run begun with [`crate::Realm::start`] is going: its script's code or the jobs after it.
    pub(crate) fn running(&self) -> bool {
        !self.frames.is_empty() || self.promises.draining
    }

    /// Runs the queued jobs, and the jobs they queue, until the queue is empty, after the script's
    /// code ended with `completion`, which this gives back.
    pub(crate) fn drain_jobs(&mut self, completion: Value) -> JsResult<Value> {
        self.promises.completion = Some(completion);
        loop {
            if !self.frames.is_empty() {
                let r = self.execute();
                self.job_returned(r)?;
                continue;
            }
            let Some(job) = self.promises.queue.pop_front() else { break };
            self.start_job(job)?;
        }
        Ok(self.promises.completion.take().unwrap_or(Value::Undefined))
    }

    /// [`Interp::execute_slice`] for a run begun with [`crate::Realm::start`]: runs the script's
    /// code and then the queued jobs, and gives `Some` with the script's completion value once the
    /// code has ended and the queue is empty.
    pub(crate) fn execute_run_slice(&mut self, pause_at: u64) -> JsResult<Option<Value>> {
        loop {
            if self.frames.is_empty() {
                let Some(job) = self.promises.queue.pop_front() else {
                    self.promises.draining = false;
                    // A pause asked for in the run's last step ends with the run.
                    self.pause.set(false);
                    return Ok(Some(self.promises.completion.take().unwrap_or(Value::Undefined)));
                };
                self.start_job(job)?;
                if self.can_pause_between_jobs(pause_at) {
                    return Ok(None);
                }
                continue;
            }
            match self.execute_slice(pause_at) {
                Ok(None) => return Ok(None),
                Ok(Some(v)) if self.promises.running.is_none() => {
                    // The script's own code ended.
                    if self.promises.queue.is_empty() {
                        self.pause.set(false);
                        return Ok(Some(v));
                    }
                    self.promises.completion = Some(v);
                    self.promises.draining = true;
                }
                Ok(Some(v)) => self.finish_job(Ok(v))?,
                Err(Abrupt::Throw(v)) if self.promises.running.is_some() => self.finish_job(Err(v))?,
                Err(e) => return Err(e),
            }
            if self.can_pause_between_jobs(pause_at) {
                return Ok(None);
            }
        }
    }

    /// No code is running and jobs are still queued, and the slice has used its fuel or the host
    /// asked for a pause. With the queue empty the run is done instead, and the next turn of the
    /// loop ends it.
    fn can_pause_between_jobs(&self, pause_at: u64) -> bool {
        self.frames.is_empty() && !self.promises.queue.is_empty() && (self.fuel <= pause_at || self.pause.replace(false))
    }

    fn job_returned(&mut self, r: JsResult<Value>) -> JsResult<()> {
        match r {
            Ok(v) => self.finish_job(Ok(v)),
            Err(Abrupt::Throw(v)) => self.finish_job(Err(v)),
            Err(e) => Err(e),
        }
    }

    /// Starts a job: a native callback runs to its end here, and a callback written in JavaScript
    /// gets a frame the caller runs.
    fn start_job(&mut self, job: Job) -> JsResult<()> {
        self.heap.bytes = self.heap.bytes.saturating_sub(JOB_BYTES);
        self.use_fuel()?;
        scoped(self, |interp| {
            let (f, this, args, then) = match job {
                Job::Reaction { handler, argument, reject, derived, site } => {
                    if !interp.is_callable(&handler) {
                        return if reject { reject_promise(interp, derived, argument, site) } else { resolve_promise(interp, derived, argument) };
                    }
                    root(interp, &Value::Object(derived));
                    match finally_callback(interp, &handler) {
                        // The callback runs as the job's own call, so a slice can pause inside it.
                        Some((on_finally, throws)) => {
                            interp.temp_roots.extend([handler, argument.clone()]);
                            (on_finally, Value::Undefined, Vec::new(), Then::Finally { derived, value: argument, throws })
                        }
                        None => (handler, Value::Undefined, vec![argument], Then::Settle(derived, site)),
                    }
                }
                Job::Thenable { promise, thenable, then } => {
                    interp.temp_roots.extend([Value::Object(promise), thenable.clone(), then.clone()]);
                    let (rec, resolve, reject) = resolving_functions(interp, promise)?;
                    (then, thenable, vec![resolve, reject], Then::Reject(rec))
                }
            };
            interp.temp_roots.push(f.clone());
            interp.temp_roots.push(this.clone());
            interp.temp_roots.extend(args.iter().cloned());
            interp.promises.running = Some(then);
            match interp.start_call(f, this, args) {
                Ok(Some(v)) => interp.finish_job(Ok(v)),
                Ok(None) => Ok(()),
                Err(Abrupt::Throw(e)) => interp.finish_job(Err(e)),
                Err(e) => Err(e),
            }
        })
    }

    /// Ends the running job with its call's result or exception.
    fn finish_job(&mut self, r: Result<Value, Value>) -> JsResult<()> {
        let site = if r.is_err() { caught(self) } else { None };
        let Some(then) = self.promises.running.take() else { return Err(Abrupt::Internal("no promise job is running".into())) };
        match (then, r) {
            (Then::Settle(d, _), Ok(v)) => resolve_promise(self, d, v),
            (Then::Settle(d, _), Err(e)) => reject_promise(self, d, e, site),
            (Then::Reject(_), Ok(_)) => Ok(()),
            (Then::Reject(rec), Err(e)) => scoped(self, |interp| {
                root(interp, &Value::Object(rec));
                reject_through(interp, rec, e, site)
            }),
            (Then::Finally { derived, value, throws }, Ok(result)) => scoped(self, |interp| {
                interp.temp_roots.extend([Value::Object(derived), value.clone(), result.clone()]);
                match after_finally(interp, result, value, throws) {
                    Ok(v) => resolve_promise(interp, derived, v),
                    Err(Abrupt::Throw(e)) => {
                        let site = caught(interp);
                        reject_promise(interp, derived, e, site)
                    }
                    Err(e) => Err(e),
                }
            }),
            (Then::Finally { derived, .. }, Err(e)) => reject_promise(self, derived, e, site),
        }
    }

    /// Drops the run's queued jobs and its state; with `keep_rejections` false, the rejections it
    /// noted too.
    pub(crate) fn end_jobs(&mut self, keep_rejections: bool) {
        let queued = self.promises.queue_bytes();
        self.heap.bytes = self.heap.bytes.saturating_sub(queued);
        // A new queue frees the old one's buffer, which the heap estimate does not count.
        self.promises.queue = VecDeque::new();
        self.promises.running = None;
        self.promises.completion = None;
        self.promises.draining = false;
        if !keep_rejections {
            self.promises.rejected.clear();
        }
    }

    /// The promises the run rejected that still have no handler, the first [`MAX_REPORTED`] of
    /// them described, and how many there are; the run's notes are cleared.
    pub(crate) fn take_unhandled(&mut self) -> (Vec<crate::Rejection>, usize) {
        let rejected = std::mem::take(&mut self.promises.rejected);
        let mut out = Vec::new();
        let mut attribution = Vec::new();
        let mut count = 0;
        for p in rejected {
            let (reason, site) = match self.heap.object(p).map(|o| &o.class) {
                Some(Class::Promise(d)) if !d.handled && d.state == State::Rejected => (d.result.clone(), d.site.clone()),
                _ => continue,
            };
            count += 1;
            if out.len() >= MAX_REPORTED {
                continue;
            }
            // Two code units per character keeps MAX_REASON_CHARS characters and one more, so the
            // cut below still marks a longer reason.
            let mut message = self.describe_prefix(&reason, 2 * MAX_REASON_CHARS + 2);
            if let Some((cut, _)) = message.char_indices().nth(MAX_REASON_CHARS) {
                message.truncate(cut);
                message.push('…');
            }
            let authored = if let Some(object) = reason.as_object() {
                if let Some((_, authored)) = attribution.iter().find(|(seen, _)| *seen == object) {
                    *authored
                } else {
                    let authored = self.thrown_by_script(&reason);
                    attribution.push((object, authored));
                    authored
                }
            } else {
                self.thrown_by_script(&reason)
            };
            let (tag, (line, column)) = site.map_or((crate::UNKNOWN_SOURCE, (0, 0)), |(code, pc)| (code.source.tag, code.line_column(pc)));
            out.push(crate::Rejection { message, line, column, tag, authored });
        }
        (out, count)
    }
}

// ---- Promise ----

/// `new Promise(executor)` (section 25.6.3.1). Calling `Promise` without `new` is a `TypeError`.
fn promise_ctor(interp: &mut Interp, _: Value, args: &[Value], constructed: bool) -> JsResult<Value> {
    if !constructed {
        return Err(interp.type_error("Promise must be called with new"));
    }
    let executor = arg(args, 0);
    if !interp.is_callable(&executor) {
        return Err(interp.type_error("the Promise executor is not a function"));
    }
    scoped(interp, |interp| {
        root(interp, &executor);
        let p = new_promise(interp)?;
        root(interp, &Value::Object(p));
        let (rec, resolve, reject) = resolving_functions(interp, p)?;
        interp.temp_roots.extend([Value::Object(rec), resolve.clone(), reject.clone()]);
        match interp.call(&executor, &Value::Undefined, &[resolve, reject]) {
            Ok(_) => {}
            Err(Abrupt::Throw(e)) => {
                let site = caught(interp);
                root(interp, &e);
                reject_through(interp, rec, e, site)?;
            }
            Err(e) => return Err(e),
        }
        Ok(Value::Object(p))
    })
}

/// A promise's resolve function (section 25.6.1.3.2).
fn resolve_function(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let rec = arg(args, 0).as_object().ok_or_else(|| Abrupt::Internal("a resolve function lost its record".into()))?;
    resolve_through(interp, rec, arg(args, 1))?;
    Ok(Value::Undefined)
}

/// A promise's reject function (section 25.6.1.3.1).
fn reject_function(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let rec = arg(args, 0).as_object().ok_or_else(|| Abrupt::Internal("a reject function lost its record".into()))?;
    let site = here(interp).or_else(|| match &interp.promises.running {
        Some(Then::Settle(_, site)) => site.clone(),
        _ => None,
    });
    reject_through(interp, rec, arg(args, 1), site)?;
    Ok(Value::Undefined)
}

/// `Promise.prototype.then` (section 25.6.5.4).
fn promise_then(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let p = this_promise(interp, &this, "then")?;
    scoped(interp, |interp| {
        let derived = new_promise(interp)?;
        root(interp, &Value::Object(derived));
        perform_then(interp, p, arg(args, 0), arg(args, 1), derived)?;
        Ok(Value::Object(derived))
    })
}

/// `Promise.prototype.catch` (section 25.6.5.1).
fn promise_catch(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    invoke(interp, &this, "then", &[Value::Undefined, arg(args, 0)])
}

/// `Promise.prototype.finally` (section 25.6.5.3).
fn promise_finally(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    if !matches!(this, Value::Object(_)) {
        return Err(interp.type_error("Promise.prototype.finally was called on a value that is not an object"));
    }
    let on_finally = arg(args, 0);
    if !interp.is_callable(&on_finally) {
        return invoke(interp, &this, "then", &[on_finally.clone(), on_finally]);
    }
    scoped(interp, |interp| {
        let rec = new_record(interp, Record::Finally { on_finally })?;
        root(interp, &Value::Object(rec));
        let then_finally = record_function(interp, Kind::ThenFinally, rec, 1)?;
        root(interp, &then_finally);
        let catch_finally = record_function(interp, Kind::CatchFinally, rec, 1)?;
        root(interp, &catch_finally);
        invoke(interp, &this, "then", &[then_finally, catch_finally])
    })
}

/// Then Finally and Catch Finally (sections 25.6.5.3.1 and 25.6.5.3.2): calls the callback, then
/// passes `value` on, or throws it again when `throws`, once the callback's result settles.
fn finally_step(interp: &mut Interp, args: &[Value], throws: bool) -> JsResult<Value> {
    let rec_value = arg(args, 0);
    let value = arg(args, 1);
    let Record::Finally { on_finally } = record(interp, &rec_value)? else { return Err(Abrupt::Internal("not a finally record".into())) };
    let on_finally = on_finally.clone();
    let result = interp.call(&on_finally, &Value::Undefined, &[])?;
    after_finally(interp, result, value, throws)
}

/// The steps of Then Finally and Catch Finally after the callback returned `result`: a promise
/// that passes `value` on, or throws it again when `throws`, once `result` settles.
fn after_finally(interp: &mut Interp, result: Value, value: Value, throws: bool) -> JsResult<Value> {
    scoped(interp, |interp| {
        root(interp, &value);
        root(interp, &result);
        let ctor = interp.promises.ctor.map_or(Value::Undefined, Value::Object);
        let promise = promise_resolve(interp, &ctor, result)?;
        root(interp, &promise);
        let rec = new_record(interp, Record::Thunk { value })?;
        root(interp, &Value::Object(rec));
        let thunk = record_function(interp, if throws { Kind::Thrower } else { Kind::ValueThunk }, rec, 0)?;
        root(interp, &thunk);
        invoke(interp, &promise, "then", &[thunk])
    })
}

/// When `handler` is the Then Finally or Catch Finally function of a `finally` call, its callback,
/// and whether the function throws the value it gets again.
fn finally_callback(interp: &Interp, handler: &Value) -> Option<(Value, bool)> {
    let Some(Class::Function(Func::Bound { target, args, .. })) = handler.as_object().and_then(|o| interp.heap.object(o)).map(|o| &o.class) else {
        return None;
    };
    let natives = &interp.promises.natives;
    let throws = if natives.get(Kind::ThenFinally as usize) == Some(target) {
        false
    } else if natives.get(Kind::CatchFinally as usize) == Some(target) {
        true
    } else {
        return None;
    };
    match args.first().and_then(Value::as_object).and_then(|o| interp.heap.object(o)).map(|o| &o.class) {
        Some(Class::PromiseRecord(rec)) => match &**rec {
            Record::Finally { on_finally } => Some((on_finally.clone(), throws)),
            _ => None,
        },
        _ => None,
    }
}

fn then_finally(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    finally_step(interp, args, false)
}

fn catch_finally(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    finally_step(interp, args, true)
}

fn thunk_value(interp: &mut Interp, args: &[Value]) -> JsResult<Value> {
    match record(interp, &arg(args, 0))? {
        Record::Thunk { value } => Ok(value.clone()),
        _ => Err(Abrupt::Internal("not a thunk record".into())),
    }
}

fn value_thunk(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    thunk_value(interp, args)
}

fn thrower(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    Err(Abrupt::Throw(thunk_value(interp, args)?))
}

/// `v` is a constructor: a function written in JavaScript other than an arrow function, generator
/// function or method, a built-in or registered host constructor, or a bound function whose target
/// is one.
fn is_constructor(interp: &mut Interp, v: &Value) -> JsResult<bool> {
    let mut v = v.clone();
    loop {
        let func = match v.as_object().and_then(|o| interp.heap.object(o)).map(|o| &o.class) {
            Some(Class::Function(f)) => f.clone(),
            _ => return Ok(false),
        };
        match func {
            Func::Closure { code, .. } => return Ok(!code.arrow && !code.method && !code.generator),
            Func::Native { constructor, .. } => return Ok(constructor),
            Func::Data { read: ArgRead::Hosted, .. } => return Ok(v.as_object().is_some_and(|o| interp.is_constructible_host_class(o))),
            Func::Bound { target, .. } => {
                interp.use_fuel()?;
                v = Value::Object(target);
            }
            _ => return Ok(false),
        }
    }
}

/// The `this` value of a static method, which must be a constructor.
fn this_constructor(interp: &mut Interp, this: &Value, name: &str) -> JsResult<()> {
    if is_constructor(interp, this)? {
        return Ok(());
    }
    Err(interp.type_error(&format!("Promise.{name} was called on a value that is not a constructor")))
}

/// `Promise.resolve` (section 25.6.4.6).
fn promise_resolve_static(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_constructor(interp, &this, "resolve")?;
    promise_resolve(interp, &this, arg(args, 0))
}

/// `Promise.reject` (section 25.6.4.5).
fn promise_reject_static(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_constructor(interp, &this, "reject")?;
    scoped(interp, |interp| {
        let p = new_promise(interp)?;
        root(interp, &Value::Object(p));
        let site = here(interp);
        reject_promise(interp, p, arg(args, 0), site)?;
        Ok(Value::Object(p))
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Combine {
    All,
    AllSettled,
    Race,
}

impl Combine {
    fn name(self) -> &'static str {
        match self {
            Combine::All => "all",
            Combine::AllSettled => "allSettled",
            Combine::Race => "race",
        }
    }
}

fn promise_all(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    combine(interp, this, arg(args, 0), Combine::All)
}

fn promise_all_settled(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    combine(interp, this, arg(args, 0), Combine::AllSettled)
}

fn promise_race(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    combine(interp, this, arg(args, 0), Combine::Race)
}

/// `Promise.all`, `allSettled` and `race` (sections 25.6.4.1, 25.6.4.2 and 25.6.4.4): an exception
/// on the way rejects the promise they return (IfAbruptRejectPromise).
fn combine(interp: &mut Interp, this: Value, input: Value, how: Combine) -> JsResult<Value> {
    this_constructor(interp, &this, how.name())?;
    scoped(interp, |interp| {
        root(interp, &input);
        let p = new_promise(interp)?;
        root(interp, &Value::Object(p));
        let (rec, resolve, reject) = resolving_functions(interp, p)?;
        interp.temp_roots.extend([Value::Object(rec), resolve.clone(), reject.clone()]);
        match combine_inputs(interp, &this, &input, how, rec, &resolve, &reject) {
            Ok(()) => {}
            Err(Abrupt::Throw(e)) => {
                let site = caught(interp);
                root(interp, &e);
                reject_through(interp, rec, e, site)?;
            }
            Err(e) => return Err(e),
        }
        Ok(Value::Object(p))
    })
}

fn combine_inputs(interp: &mut Interp, this: &Value, input: &Value, how: Combine, rec: ObjId, resolve: &Value, reject: &Value) -> JsResult<()> {
    let resolve_fn = interp.get_value(this, &key("resolve"))?;
    if !interp.is_callable(&resolve_fn) {
        return Err(interp.type_error(&format!("Promise.{}: the constructor's resolve is not a function", how.name())));
    }
    root(interp, &resolve_fn);
    let iterator = interp.get_iterator(input)?;
    root(interp, &Value::Object(iterator));
    let result = combine_iterator(interp, this, iterator, how, rec, &resolve_fn, (resolve, reject));
    if let Err(Abrupt::Throw(reason)) = &result {
        root(interp, reason);
        let site = interp.throw_site.clone();
        let mark = interp.thrown_mark;
        match interp.iter_close(iterator) {
            Ok(()) | Err(Abrupt::Throw(_)) => {
                interp.throw_site = site;
                interp.thrown_mark = mark;
                interp.passing = reason.clone();
            }
            Err(other) => return Err(other),
        }
    }
    result
}

fn combine_iterator(
    interp: &mut Interp,
    this: &Value,
    iterator: ObjId,
    how: Combine,
    rec: ObjId,
    resolve_fn: &Value,
    handlers: (&Value, &Value),
) -> JsResult<()> {
    let (resolve, reject) = handlers;
    let combinator = if how == Combine::Race {
        None
    } else {
        let values = new_array(interp, 0)?;
        root(interp, &Value::Object(values));
        let c = new_record(interp, Record::Combinator { resolving: rec, values, remaining: 1 })?;
        root(interp, &Value::Object(c));
        Some((c, values))
    };
    let mut index: u32 = 0;
    loop {
        let mark = interp.temp_roots.len();
        let Some(next) = interp.iter_next(iterator)? else { break };
        root(interp, &next);
        let next_promise = interp.call(resolve_fn, this, &[next])?;
        root(interp, &next_promise);
        match combinator {
            None => {
                invoke(interp, &next_promise, "then", &[resolve.clone(), reject.clone()])?;
            }
            Some((c, values)) => {
                create_at(interp, values, f64::from(index), Value::Undefined)?;
                let el = new_record(interp, Record::Element { combinator: c, index, called: false })?;
                root(interp, &Value::Object(el));
                let (on_fulfilled, on_rejected) = if how == Combine::All {
                    (record_function(interp, Kind::AllElement, el, 1)?, reject.clone())
                } else {
                    let f = record_function(interp, Kind::SettledFulfilled, el, 1)?;
                    root(interp, &f);
                    (f, record_function(interp, Kind::SettledRejected, el, 1)?)
                };
                interp.temp_roots.extend([on_fulfilled.clone(), on_rejected.clone()]);
                if let Record::Combinator { remaining, .. } = record(interp, &Value::Object(c))? {
                    *remaining += 1;
                }
                invoke(interp, &next_promise, "then", &[on_fulfilled, on_rejected])?;
            }
        }
        interp.temp_roots.truncate(mark);
        index = index.checked_add(1).filter(|&i| i < u32::MAX).ok_or_else(|| interp.throw(ErrorType::Range, "too many promises to combine"))?;
    }
    if let Some((c, _)) = combinator {
        element_done(interp, c)?;
    }
    Ok(())
}

/// Counts one more settled input of a `Promise.all` or `allSettled`; after the last one, resolves
/// the result with the values.
fn element_done(interp: &mut Interp, c: ObjId) -> JsResult<()> {
    let Record::Combinator { resolving, values, remaining } = record(interp, &Value::Object(c))? else {
        return Err(Abrupt::Internal("not a combinator record".into()));
    };
    *remaining = remaining.saturating_sub(1);
    if *remaining > 0 {
        return Ok(());
    }
    let (resolving, values) = (*resolving, *values);
    scoped(interp, |interp| {
        root(interp, &Value::Object(c));
        resolve_through(interp, resolving, Value::Object(values))
    })
}

/// Stores `value` as one input's result (the first call only) and counts the input settled.
fn element_settled(interp: &mut Interp, args: &[Value], make: impl FnOnce(&mut Interp, Value) -> JsResult<Value>) -> JsResult<Value> {
    let rec_value = arg(args, 0);
    let Record::Element { combinator, index, called } = record(interp, &rec_value)? else {
        return Err(Abrupt::Internal("not an element record".into()));
    };
    if *called {
        return Ok(Value::Undefined);
    }
    *called = true;
    let (c, index) = (*combinator, *index);
    let Record::Combinator { values, .. } = record(interp, &Value::Object(c))? else {
        return Err(Abrupt::Internal("not a combinator record".into()));
    };
    let values = *values;
    let v = make(interp, arg(args, 1))?;
    scoped(interp, |interp| {
        root(interp, &v);
        super::put_at(interp, values, f64::from(index), v.clone())?;
        element_done(interp, c)
    })?;
    Ok(Value::Undefined)
}

/// `Promise.all` Resolve Element Functions (section 25.6.4.1.2).
fn all_element(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    element_settled(interp, args, |_, v| Ok(v))
}

/// An `allSettled` result object: `{ status, value }` or `{ status, reason }`.
fn settled_object(interp: &mut Interp, fulfilled: bool, v: Value) -> JsResult<Value> {
    scoped(interp, |interp| {
        root(interp, &v);
        interp.reserve_heap(OBJECT_BYTES + 2 * PROP_BYTES + 64)?;
        let o = interp.new_object();
        root(interp, &Value::Object(o));
        let (status, name) = if fulfilled { ("fulfilled", "value") } else { ("rejected", "reason") };
        interp.define(o, key("status"), Prop::Data { value: Value::str(status), flags: OPEN })?;
        interp.define(o, key(name), Prop::Data { value: v, flags: OPEN })?;
        Ok(Value::Object(o))
    })
}

/// `Promise.allSettled` Resolve Element Functions (section 25.6.4.2.2).
fn settled_fulfilled(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    element_settled(interp, args, |interp, v| settled_object(interp, true, v))
}

/// `Promise.allSettled` Reject Element Functions (section 25.6.4.2.3).
fn settled_rejected(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    element_settled(interp, args, |interp, v| settled_object(interp, false, v))
}
