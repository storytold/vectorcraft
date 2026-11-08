//! The interpreter: runs bytecode on an explicit stack of frames.
//!
//! A call to a function written in JavaScript pushes a frame and goes on in the same loop, so
//! JavaScript recursion uses the heap, not the Rust stack. The loop runs again inside a Rust call
//! only where the language needs a function's result in the middle of an operation (a getter or
//! setter, `valueOf` and `toString` during a conversion, indirect `eval`); those nested runs are
//! counted against [`crate::Limits::reentry`].
//!
//! Before each instruction the interpreter takes one unit of fuel, checks the interrupt flag and,
//! when the heap has grown past its threshold, collects garbage. Running out of fuel, the heap
//! limit, the frame limit, the re-entry limit and the interrupt flag each end the run with an
//! [`Abrupt::Limit`] that unwinds every frame without running catch or finally blocks.
//!
//! Collection runs only between instructions. An operation that may run JavaScript in the middle
//! keeps its operands on the operand stack until it is done, so everything it holds is reachable
//! from the roots when a nested run collects.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ast::BinaryOp;
use crate::bytecode::{Code, CodeKind, Op, Source};
use crate::compiler::{CompileFail, compile_program};
use crate::heap::{
    ArgMap, Binding, CONFIGURABLE, Class, ENUMERABLE, ENV_BYTES, Env, EnvId, ForIn, Func, HIDDEN, Heap, Key, NativeKind, OBJECT_BYTES, OPEN, ObjId,
    Object, PROP_BYTES, Prop, PropMap, ROPE_BYTES, Roots, WRITABLE, code_bytes, key, string_bytes,
};
use crate::value::{Str, Value, array_index, index_key, number_to_str, string_to_number, to_int32, to_uint32};
use crate::{LimitKind, Limits, Options};

/// How an operation ended other than normally.
#[derive(Debug)]
pub(crate) enum Abrupt {
    /// A JavaScript exception: `try` and `catch` can handle it.
    Throw(Value),
    /// A limit was reached; nothing handles it.
    Limit(LimitKind),
    /// The interpreter found itself in a state it never makes (a bug); nothing handles it.
    Internal(String),
}

pub(crate) type JsResult<T> = Result<T, Abrupt>;

fn internal<T>(message: &str) -> JsResult<T> {
    Err(Abrupt::Internal(message.into()))
}

/// The kinds of error object the engine throws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorType {
    Error,
    Eval,
    Range,
    Reference,
    Syntax,
    Type,
    Uri,
}

/// The objects the interpreter itself refers to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Intrinsics {
    pub(crate) object_proto: ObjId,
    pub(crate) function_proto: ObjId,
    pub(crate) array_proto: ObjId,
    pub(crate) string_proto: ObjId,
    pub(crate) number_proto: ObjId,
    pub(crate) boolean_proto: ObjId,
    pub(crate) regexp_proto: ObjId,
    pub(crate) date_proto: ObjId,
    pub(crate) error_protos: [ObjId; 7],
    pub(crate) eval: ObjId,
    pub(crate) throw_type_error: ObjId,
}

impl Intrinsics {
    fn placeholder() -> Self {
        let o = ObjId(0);
        Self {
            object_proto: o,
            function_proto: o,
            array_proto: o,
            string_proto: o,
            number_proto: o,
            boolean_proto: o,
            regexp_proto: o,
            date_proto: o,
            error_protos: [o; 7],
            eval: o,
            throw_type_error: o,
        }
    }

    pub(crate) fn error_proto(&self, t: ErrorType) -> ObjId {
        let i = match t {
            ErrorType::Error => 0,
            ErrorType::Eval => 1,
            ErrorType::Range => 2,
            ErrorType::Reference => 3,
            ErrorType::Syntax => 4,
            ErrorType::Type => 5,
            ErrorType::Uri => 6,
        };
        self.error_protos.get(i).copied().unwrap_or(self.object_proto)
    }

    fn all(&self) -> Vec<ObjId> {
        let mut v = vec![
            self.object_proto,
            self.function_proto,
            self.array_proto,
            self.string_proto,
            self.number_proto,
            self.boolean_proto,
            self.regexp_proto,
            self.date_proto,
            self.eval,
            self.throw_type_error,
        ];
        v.extend(self.error_protos);
        v
    }
}

pub(crate) struct Frame {
    pub(crate) code: Rc<Code>,
    pub(crate) pc: usize,
    /// Where this frame's operands start on the operand stack.
    base: usize,
    env: EnvId,
    var_env: EnvId,
    /// `with` and catch environments entered and not yet left.
    env_depth: u32,
    this: Value,
    callee: Option<ObjId>,
    /// The object `new` made, returned unless the function returns an object.
    construct: Option<ObjId>,
    completion: Value,
    /// Returning from this frame returns from the Rust call that pushed it.
    boundary: bool,
}

/// Where the exception being thrown was raised: the code and the instruction.
type ThrowSite = (Rc<Code>, usize);

pub(crate) struct Interp {
    pub(crate) heap: Heap,
    pub(crate) stack: Vec<Value>,
    pub(crate) frames: Vec<Frame>,
    /// Values native code holds while it may run JavaScript.
    pub(crate) temp_roots: Vec<Value>,
    pub(crate) global: ObjId,
    pub(crate) global_env: EnvId,
    pub(crate) intrinsics: Intrinsics,
    pub(crate) limits: Limits,
    pub(crate) options: Options,
    pub(crate) fuel: u64,
    pub(crate) interrupt: Arc<AtomicBool>,
    pub(crate) reentry: usize,
    gc_threshold: usize,
    /// UTF-16 code units of source parsed in this run, for [`Options::max_source_units`].
    pub(crate) source_units: usize,
    pub(crate) throw_site: Option<ThrowSite>,
    /// The last exception a run did not catch.
    pub(crate) last_thrown: Value,
    /// The clock and time zone `Date` reads.
    pub(crate) date_host: crate::date_builtins::DateHost,
    /// The bytes native code holds outside the heap's cells through [`HeldBytes`].
    pub(crate) held: Rc<HeldCount>,
}

/// The bytes all live [`HeldBytes`] hold, and the bytes dropped holders released that the heap's
/// total still counts.
#[derive(Default)]
pub(crate) struct HeldCount {
    now: std::cell::Cell<usize>,
    released: std::cell::Cell<usize>,
}

/// Bytes that native code holds in Rust memory while it works (a matcher's state and stack, a
/// replacement being built, a pattern being compiled), counted against the heap limit. They are
/// in the heap's total, and in the live total of each collection, until the holder is dropped;
/// the heap's total gives them back before the next limit check.
pub(crate) struct HeldBytes {
    count: Rc<HeldCount>,
    bytes: usize,
}

impl HeldBytes {
    pub(crate) fn new(interp: &Interp) -> Self {
        Self { count: interp.held.clone(), bytes: 0 }
    }

    /// The bytes this holder holds.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Holds `n` more bytes, ending the run with the heap limit when they would not fit after a
    /// collection. Everything the caller still needs must be reachable from the roots.
    pub(crate) fn add(&mut self, interp: &mut Interp, n: usize) -> JsResult<()> {
        interp.reserve_heap(n)?;
        interp.heap.bytes = interp.heap.bytes.saturating_add(n);
        self.bytes = self.bytes.saturating_add(n);
        self.count.now.set(self.count.now.get().saturating_add(n));
        Ok(())
    }
}

impl Drop for HeldBytes {
    fn drop(&mut self) {
        self.count.now.set(self.count.now.get().saturating_sub(self.bytes));
        self.count.released.set(self.count.released.get().saturating_add(self.bytes));
    }
}

/// The least heap size a collection waits for.
const MIN_GC_BYTES: usize = 1 << 20;

/// The most arguments `Function.prototype.apply` passes.
pub(crate) const MAX_APPLY_ARGS: usize = 1 << 16;

/// The estimated bytes of a name's entries in the list and the set `for-in` builds.
const KEY_ENTRY_BYTES: usize = 32;

/// The estimated bytes of the index names `0` to `len - 1` while `for-in` lists them.
fn index_keys_bytes(len: usize) -> usize {
    let mut total = 0usize;
    let (mut lo, mut hi, mut digits) = (0usize, 10usize, 1usize);
    while lo < len {
        let count = hi.min(len) - lo;
        total = total.saturating_add(count.saturating_mul(string_bytes(digits) + KEY_ENTRY_BYTES));
        lo = hi;
        hi = hi.saturating_mul(10);
        digits += 1;
    }
    total
}

/// The estimated bytes of a binding or property named by `len` code units.
fn binding_bytes(len: usize) -> usize {
    PROP_BYTES + string_bytes(len)
}

/// At least the bytes that a function object made from `code` adds to the heap: the function,
/// its prototype object, their properties and the environment holding a function expression's
/// own name.
fn closure_bytes(code: &Code) -> usize {
    let props = if code.strict { 5 } else { 3 };
    let named = code.self_name.as_ref().map_or(0, |n| ENV_BYTES + binding_bytes(n.len()));
    2 * OBJECT_BYTES + props * binding_bytes("constructor".len()) + named
}

/// At least the bytes that declaration binding instantiation (section 10.5) adds to the heap for
/// the function declarations and variables of `code`.
fn declaration_bytes(code: &Code) -> usize {
    let funs = code.fun_decls.iter().fold(0usize, |total, (name, fi)| {
        let closure = code.functions.get(*fi as usize).map_or(0, |f| closure_bytes(f));
        total.saturating_add(binding_bytes(name.len()) + closure)
    });
    code.vars.iter().fold(funs, |total, v| total.saturating_add(binding_bytes(v.len())))
}

/// A call to a function written in JavaScript, ready to enter.
struct Entry {
    callee: ObjId,
    code: Rc<Code>,
    env: EnvId,
    this: Value,
    args: Vec<Value>,
}

/// What a call resolved to.
enum Prepared {
    Closure(Entry),
    Done(Value),
}

/// A variable found in the environment chain.
enum Resolved {
    Decl(EnvId),
    Obj { object: ObjId, with_this: bool },
    Unresolved,
}

/// A reference base as an operand: a declarative environment's index as a number, a binding
/// object as itself, and an unresolvable reference as `undefined`. The environment stays
/// reachable through the frame's environment chain while the operand is on the stack.
fn resolved_to_value(r: &Resolved) -> Value {
    match r {
        Resolved::Decl(e) => Value::Number(f64::from(e.0)),
        Resolved::Obj { object, .. } => Value::Object(*object),
        Resolved::Unresolved => Value::Undefined,
    }
}

fn value_to_resolved(v: &Value) -> JsResult<Resolved> {
    match v {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Value::Number(n) if *n >= 0.0 && *n <= f64::from(u32::MAX) && n.fract() == 0.0 => Ok(Resolved::Decl(EnvId(*n as u32))),
        Value::Object(object) => Ok(Resolved::Obj { object: *object, with_this: false }),
        Value::Undefined => Ok(Resolved::Unresolved),
        _ => internal("not a reference base"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hint {
    Default,
    Number,
    String,
}

fn is(k: &[u16], s: &str) -> bool {
    k.len() == s.len() && k.iter().zip(s.bytes()).all(|(&a, b)| a == u16::from(b))
}

impl Interp {
    pub(crate) fn new(options: Options, limits: Limits) -> Self {
        let mut heap = Heap::default();
        let global = heap.alloc_object(Object::new(Class::Ordinary, None));
        let global_env = heap.alloc_env(Env::Obj { object: global, with_this: false, outer: None });
        let mut interp = Self {
            heap,
            stack: Vec::new(),
            frames: Vec::new(),
            temp_roots: Vec::new(),
            global,
            global_env,
            intrinsics: Intrinsics::placeholder(),
            fuel: u64::MAX,
            // The built-ins are installed without limits; the realm's limits apply from the
            // first run.
            limits: Limits { fuel: u64::MAX, heap_bytes: usize::MAX, frames: usize::MAX, reentry: usize::MAX },
            options,
            interrupt: Arc::new(AtomicBool::new(false)),
            reentry: 0,
            gc_threshold: MIN_GC_BYTES,
            source_units: 0,
            throw_site: None,
            last_thrown: Value::Undefined,
            date_host: crate::date_builtins::DateHost::default(),
            held: Rc::default(),
        };
        crate::builtins::install(&mut interp);
        interp.fuel = limits.fuel;
        interp.limits = limits;
        interp.gc_threshold = interp.next_threshold();
        interp
    }

    fn next_threshold(&self) -> usize {
        (self.heap.bytes.saturating_mul(2)).max(MIN_GC_BYTES).min(self.limits.heap_bytes)
    }

    /// Recomputes when the next collection runs, after the limits change.
    pub(crate) fn reset_gc_threshold(&mut self) {
        self.gc_threshold = self.next_threshold();
    }

    // ---- errors ----

    pub(crate) fn error_object(&mut self, t: ErrorType, message: &str) -> ObjId {
        let proto = self.intrinsics.error_proto(t);
        let mut o = Object::new(Class::Error, Some(proto));
        if !message.is_empty() {
            o.props.insert(key("message"), Prop::Data { value: Value::str(message), flags: HIDDEN });
        }
        self.heap.alloc_object(o)
    }

    pub(crate) fn throw(&mut self, t: ErrorType, message: &str) -> Abrupt {
        Abrupt::Throw(Value::Object(self.error_object(t, message)))
    }

    pub(crate) fn type_error(&mut self, message: &str) -> Abrupt {
        self.throw(ErrorType::Type, message)
    }

    // ---- the operand stack ----

    fn push(&mut self, v: Value) -> JsResult<()> {
        self.stack.push(v);
        Ok(())
    }

    fn frame(&self) -> JsResult<&Frame> {
        self.frames.last().ok_or_else(|| Abrupt::Internal("no frame".into()))
    }

    fn frame_mut(&mut self) -> JsResult<&mut Frame> {
        self.frames.last_mut().ok_or_else(|| Abrupt::Internal("no frame".into()))
    }

    fn base(&self) -> usize {
        self.frames.last().map_or(0, |f| f.base)
    }

    fn pop(&mut self) -> JsResult<Value> {
        if self.stack.len() <= self.base() {
            return internal("operand stack underflow");
        }
        self.stack.pop().ok_or_else(|| Abrupt::Internal("operand stack underflow".into()))
    }

    /// The value `n` places below the top (0 is the top).
    fn peek(&self, n: usize) -> JsResult<Value> {
        let len = self.stack.len();
        match len.checked_sub(n + 1).filter(|&i| i >= self.base()) {
            Some(i) => self.stack.get(i).cloned().ok_or_else(|| Abrupt::Internal("operand stack underflow".into())),
            None => internal("operand stack underflow"),
        }
    }

    fn drop_n(&mut self, n: usize) -> JsResult<()> {
        let len = self.stack.len();
        match len.checked_sub(n).filter(|&i| i >= self.base()) {
            Some(i) => {
                self.stack.truncate(i);
                Ok(())
            }
            None => internal("operand stack underflow"),
        }
    }

    /// Replaces the top `n` values with `v`.
    fn replace(&mut self, n: usize, v: Value) -> JsResult<()> {
        self.drop_n(n)?;
        self.push(v)
    }

    // ---- running ----

    /// Takes a unit of fuel and checks the interrupt flag.
    pub(crate) fn use_fuel(&mut self) -> JsResult<()> {
        if self.fuel == 0 {
            return Err(Abrupt::Limit(LimitKind::Fuel));
        }
        self.fuel -= 1;
        if self.interrupt.load(Ordering::Relaxed) {
            return Err(Abrupt::Limit(LimitKind::Interrupted));
        }
        Ok(())
    }

    fn tick(&mut self) -> JsResult<()> {
        self.use_fuel()?;
        self.settle_held();
        if self.heap.bytes > self.gc_threshold {
            self.collect_garbage();
            if self.heap.bytes > self.limits.heap_bytes {
                return Err(Abrupt::Limit(LimitKind::Heap));
            }
            self.gc_threshold = self.next_threshold();
        }
        Ok(())
    }

    /// Ends the run with the heap limit when `bytes` more would not fit in it after a
    /// collection. Everything the caller still needs must be reachable from the roots.
    pub(crate) fn reserve_heap(&mut self, bytes: usize) -> JsResult<()> {
        self.settle_held();
        if self.heap.bytes.saturating_add(bytes) <= self.limits.heap_bytes {
            return Ok(());
        }
        self.collect_garbage();
        self.gc_threshold = self.next_threshold();
        if self.heap.bytes.saturating_add(bytes) > self.limits.heap_bytes {
            return Err(Abrupt::Limit(LimitKind::Heap));
        }
        Ok(())
    }

    /// Takes the bytes that dropped [`HeldBytes`] released off the heap's total.
    fn settle_held(&mut self) {
        let released = self.held.released.replace(0);
        self.heap.bytes = self.heap.bytes.saturating_sub(released);
    }

    /// Collects garbage now and sets when the next collection runs.
    #[cfg(test)]
    pub(crate) fn collect_for_room(&mut self) {
        self.collect_garbage();
        self.gc_threshold = self.next_threshold();
    }

    /// Charges code compiled at run time to the heap, reserving its estimated size and `extra`
    /// more first. The code is not reachable from the roots yet, so a collection here leaves it
    /// out of the live total and the charge adds it.
    pub(crate) fn charge_code(&mut self, code: &Code, extra: usize) -> JsResult<()> {
        let bytes = code_bytes(code);
        self.reserve_heap(bytes.saturating_add(extra))?;
        self.heap.bytes = self.heap.bytes.saturating_add(bytes);
        Ok(())
    }

    pub(crate) fn collect_garbage(&mut self) {
        let mut roots = Roots::default();
        roots.values.extend(self.stack.iter());
        roots.values.extend(self.temp_roots.iter());
        roots.values.push(&self.last_thrown);
        for f in &self.frames {
            roots.values.push(&f.this);
            roots.values.push(&f.completion);
            roots.objects.extend(f.callee.iter().chain(f.construct.iter()));
            roots.envs.push(f.env);
            roots.envs.push(f.var_env);
            roots.codes.push(&f.code);
        }
        roots.objects.push(self.global);
        roots.objects.extend(self.intrinsics.all());
        roots.envs.push(self.global_env);
        self.heap.collect(&roots);
        // The collection counts the live total again, so bytes already released are not in it.
        self.held.released.set(0);
        self.heap.bytes = self.heap.bytes.saturating_add(self.held.now.get());
    }

    /// Runs frames until the boundary frame on top returns, and returns its value.
    pub(crate) fn execute(&mut self) -> JsResult<Value> {
        loop {
            match self.step() {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => {}
                Err(Abrupt::Throw(v)) => self.unwind(v)?,
                Err(other) => {
                    self.note_site();
                    self.abort_to_boundary();
                    return Err(other);
                }
            }
        }
    }

    fn step(&mut self) -> JsResult<Option<Value>> {
        self.tick()?;
        let frame = self.frame_mut()?;
        let Some(&op) = frame.code.ops.get(frame.pc) else { return internal("ran past the end of the code") };
        frame.pc += 1;
        match op {
            Op::Return => {
                let v = self.pop()?;
                self.return_value(v)
            }
            Op::ReturnCompletion => {
                let v = self.frame()?.completion.clone();
                self.return_value(v)
            }
            _ => self.exec(op).map(|()| None),
        }
    }

    fn note_site(&mut self) {
        if self.throw_site.is_none()
            && let Some(f) = self.frames.last()
        {
            self.throw_site = Some((f.code.clone(), f.pc.saturating_sub(1)));
        }
    }

    /// Finds the handler for an exception: in the top frame, or in the frames below it up to the
    /// boundary frame. Returns the exception when no handler takes it before the boundary frame.
    fn unwind(&mut self, v: Value) -> JsResult<()> {
        self.note_site();
        loop {
            let Some(frame) = self.frames.last_mut() else { return Err(Abrupt::Throw(v)) };
            if let Some(h) = frame.code.handler(frame.pc.saturating_sub(1)) {
                let target_len = frame.base + h.depth as usize;
                while frame.env_depth > h.env_depth {
                    let outer = self.heap.env(frame.env).and_then(Env::outer);
                    let Some(outer) = outer else { return internal("environment chain ended early") };
                    frame.env = outer;
                    frame.env_depth -= 1;
                }
                frame.pc = h.target as usize;
                if self.stack.len() < target_len {
                    return internal("handler stack deeper than the stack");
                }
                self.stack.truncate(target_len);
                self.stack.push(v);
                self.throw_site = None;
                return Ok(());
            }
            let Some(frame) = self.frames.pop() else { return Err(Abrupt::Throw(v)) };
            self.stack.truncate(frame.base);
            if frame.boundary {
                return Err(Abrupt::Throw(v));
            }
        }
    }

    fn abort_to_boundary(&mut self) {
        while let Some(frame) = self.frames.pop() {
            self.stack.truncate(frame.base);
            if frame.boundary {
                break;
            }
        }
    }

    fn return_value(&mut self, v: Value) -> JsResult<Option<Value>> {
        let Some(frame) = self.frames.pop() else { return internal("return without a frame") };
        self.stack.truncate(frame.base);
        let v = match frame.construct {
            Some(o) if !matches!(v, Value::Object(_)) => Value::Object(o),
            _ => v,
        };
        if frame.boundary {
            return Ok(Some(v));
        }
        self.push(v)?;
        Ok(None)
    }

    fn jump_to(&mut self, target: u32) -> JsResult<()> {
        self.frame_mut()?.pc = target as usize;
        Ok(())
    }

    fn name(&self, i: u32) -> JsResult<Key> {
        self.frame()?.code.names.get(i as usize).cloned().ok_or_else(|| Abrupt::Internal("name out of range".into()))
    }

    fn strict(&self) -> bool {
        self.frames.last().is_some_and(|f| f.code.strict)
    }

    fn exec(&mut self, op: Op) -> JsResult<()> {
        match op {
            Op::Undefined => self.push(Value::Undefined),
            Op::Null => self.push(Value::Null),
            Op::True => self.push(Value::Bool(true)),
            Op::False => self.push(Value::Bool(false)),
            Op::Number(i) => {
                let n = self.frame()?.code.numbers.get(i as usize).copied();
                self.push(Value::Number(n.unwrap_or(f64::NAN)))
            }
            Op::String(i) => {
                let s = self.frame()?.code.strings.get(i as usize).cloned();
                self.push(s.map_or(Value::Undefined, Value::String))
            }
            Op::This => {
                let this = self.frame()?.this.clone();
                self.push(this)
            }
            Op::Pop => self.pop().map(drop),
            Op::PopUnder(n) => self.op_pop_under(n),
            Op::Dup => self.op_dup(1),
            Op::Dup2 => self.op_dup(2),
            Op::Swap => self.op_swap(),
            Op::Rot3 => self.op_rot3(),
            Op::GetVar(n) => self.op_get_var(n, false),
            Op::GetVarThis(n) => self.op_get_var(n, true),
            Op::SetVar(n) => self.op_set_var(n),
            Op::RefVar(n) => self.op_ref_var(n),
            Op::GetRef(n) => self.op_get_ref(n),
            Op::PutRef(n) => self.op_put_ref(n),
            Op::TypeofVar(n) => self.op_typeof_var(n),
            Op::DeleteVar(n) => self.op_delete_var(n),
            Op::IncVar { name, prefix, dec } => self.op_inc_var(name, prefix, dec),
            Op::GetMember(n) => self.op_get_member(n, false),
            Op::GetMemberThis(n) => self.op_get_member(n, true),
            Op::SetMember(n) => self.op_set_member(n),
            Op::DeleteMember(n) => self.op_delete_member(n),
            Op::IncMember { name, prefix, dec } => self.op_inc_member(name, prefix, dec),
            Op::ToKey => self.op_to_key(),
            Op::CheckObject => self.op_check_object(),
            Op::GetIndex => self.op_get_index(false),
            Op::GetIndexThis => self.op_get_index(true),
            Op::SetIndex => self.op_set_index(),
            Op::DeleteIndex => self.op_delete_index(),
            Op::IncIndex { prefix, dec } => self.op_inc_index(prefix, dec),
            Op::Call(n) => self.op_call(n, false),
            Op::CallEval(n) => self.op_call(n, true),
            Op::New(n) => self.op_new(n),
            Op::Binary(b) => self.op_binary(b),
            Op::Neg | Op::Plus | Op::BitNot | Op::Not | Op::Typeof | Op::ToNumber => self.op_unary(op),
            Op::Jump(t) => self.jump_to(t),
            Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => self.op_branch(t, matches!(op, Op::JumpIfTrue(_)), false),
            Op::JumpIfFalseKeep(t) | Op::JumpIfTrueKeep(t) => self.op_branch(t, matches!(op, Op::JumpIfTrueKeep(_)), true),
            Op::Throw => Err(Abrupt::Throw(self.pop()?)),
            Op::ThrowReferenceError(i) => self.op_throw_reference_error(i),
            Op::EndFinally(t) => self.op_end_finally(t),
            Op::NewObject => self.op_new_object(),
            Op::NewArray => self.op_new_array(),
            Op::ArrayPush => self.op_array_push(true),
            Op::ArrayHole => self.op_array_push(false),
            Op::InitProp(n) => self.op_init_prop(n),
            Op::InitGetter(n) => self.op_init_accessor(n, true),
            Op::InitSetter(n) => self.op_init_accessor(n, false),
            Op::Closure(i) => self.op_closure(i),
            Op::RegExp(i) => self.op_regexp(i),
            Op::PushWith => self.op_push_with(),
            Op::PushCatch(n) => self.op_push_catch(n),
            Op::PopEnv => self.op_pop_env(),
            Op::ForInStart => self.op_for_in_start(),
            Op::ForInNext(t) => self.op_for_in_next(t),
            Op::SetCompletion => {
                let v = self.pop()?;
                self.frame_mut()?.completion = v;
                Ok(())
            }
            Op::GetCompletion => {
                let v = self.frame()?.completion.clone();
                self.push(v)
            }
            Op::Return | Op::ReturnCompletion => internal("return reached the dispatcher"),
        }
    }

    // ---- stack operations ----

    fn op_pop_under(&mut self, n: u32) -> JsResult<()> {
        let top = self.pop()?;
        self.drop_n(n as usize)?;
        self.push(top)
    }

    fn op_dup(&mut self, n: usize) -> JsResult<()> {
        for _ in 0..n {
            let v = self.peek(n - 1)?;
            self.push(v)?;
        }
        Ok(())
    }

    fn op_swap(&mut self) -> JsResult<()> {
        let b = self.pop()?;
        let a = self.pop()?;
        self.push(b)?;
        self.push(a)
    }

    fn op_rot3(&mut self) -> JsResult<()> {
        let c = self.pop()?;
        let b = self.pop()?;
        let a = self.pop()?;
        self.push(b)?;
        self.push(c)?;
        self.push(a)
    }

    fn op_branch(&mut self, target: u32, when: bool, keep: bool) -> JsResult<()> {
        let v = self.peek(0)?;
        let jump = v.truthy() == when;
        if !(keep && jump) {
            self.pop()?;
        }
        if jump { self.jump_to(target) } else { Ok(()) }
    }

    fn op_throw_reference_error(&mut self, i: u32) -> JsResult<()> {
        let message = self.frame()?.code.strings.get(i as usize).map(Str::to_rust_string).unwrap_or_default();
        Err(self.throw(ErrorType::Reference, &message))
    }

    fn op_end_finally(&mut self, table: u32) -> JsResult<()> {
        let Value::Number(kind) = self.pop()? else { return internal("finally kind is not a number") };
        if kind == 0.0 {
            return self.pop().map(drop);
        }
        if kind == 1.0 {
            return Err(Abrupt::Throw(self.pop()?));
        }
        let k = (kind - 2.0) as usize;
        let target = self.frame()?.code.finally_tables.get(table as usize).and_then(|t| t.get(k)).copied();
        match target {
            Some(t) => self.jump_to(t),
            None => internal("finally exit out of range"),
        }
    }

    // ---- variables ----

    fn resolve(&self, name: &[u16]) -> JsResult<Resolved> {
        let mut cur = Some(self.frame()?.env);
        while let Some(e) = cur {
            match self.heap.env(e) {
                Some(Env::Decl { bindings, outer }) => {
                    if bindings.contains(name) {
                        return Ok(Resolved::Decl(e));
                    }
                    cur = *outer;
                }
                Some(Env::Obj { object, with_this, outer }) => {
                    if self.has_property(*object, name)? {
                        return Ok(Resolved::Obj { object: *object, with_this: *with_this });
                    }
                    cur = *outer;
                }
                None => return internal("environment out of range"),
            }
        }
        Ok(Resolved::Unresolved)
    }

    fn binding_value(&self, env: EnvId, name: &[u16]) -> JsResult<Value> {
        match self.heap.env(env) {
            Some(Env::Decl { bindings, .. }) => Ok(bindings.get(name).map_or(Value::Undefined, |b| b.value.clone())),
            _ => internal("not a declarative environment"),
        }
    }

    fn not_defined(&mut self, name: &[u16]) -> Abrupt {
        let message = format!("{} is not defined", String::from_utf16_lossy(name));
        self.throw(ErrorType::Reference, &message)
    }

    fn get_var(&mut self, name: &Key) -> JsResult<(Value, Value)> {
        let r = self.resolve(name)?;
        self.get_resolved(r, name)
    }

    /// GetValue of a reference whose base was resolved earlier (section 8.7.1).
    fn get_resolved(&mut self, r: Resolved, name: &Key) -> JsResult<(Value, Value)> {
        match r {
            Resolved::Decl(e) => Ok((self.binding_value(e, name)?, Value::Undefined)),
            Resolved::Obj { object, with_this } => {
                let v = self.get_from(object, name, &Value::Object(object))?;
                Ok((v, if with_this { Value::Object(object) } else { Value::Undefined }))
            }
            Resolved::Unresolved => Err(self.not_defined(name)),
        }
    }

    fn set_var(&mut self, name: &Key, v: Value) -> JsResult<()> {
        let r = self.resolve(name)?;
        self.set_resolved(r, name, v)
    }

    /// PutValue of a reference whose base was resolved earlier (section 8.7.2).
    fn set_resolved(&mut self, r: Resolved, name: &Key, v: Value) -> JsResult<()> {
        let strict = self.strict();
        match r {
            Resolved::Decl(e) => {
                let Some(Env::Decl { bindings, .. }) = self.heap.env_mut(e) else { return internal("not a declarative environment") };
                match bindings.get_mut(name) {
                    Some(b) if b.mutable => {
                        b.value = v;
                        Ok(())
                    }
                    _ if strict => Err(self.type_error("assignment to a constant binding")),
                    _ => Ok(()),
                }
            }
            Resolved::Obj { object, .. } => self.put(object, name.clone(), v, strict),
            Resolved::Unresolved if strict => Err(self.not_defined(name)),
            Resolved::Unresolved => {
                let global = self.global;
                self.put(global, name.clone(), v, false)
            }
        }
    }

    fn op_get_var(&mut self, n: u32, with_this: bool) -> JsResult<()> {
        let name = self.name(n)?;
        let (v, this) = self.get_var(&name)?;
        self.push(v)?;
        if with_this { self.push(this) } else { Ok(()) }
    }

    fn op_set_var(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let v = self.peek(0)?;
        self.set_var(&name, v)
    }

    fn op_typeof_var(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let v = match self.resolve(&name)? {
            Resolved::Unresolved => Value::Undefined,
            _ => self.get_var(&name)?.0,
        };
        let t = self.type_of(&v);
        self.push(Value::str(t))
    }

    fn op_delete_var(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let result = match self.resolve(&name)? {
            Resolved::Unresolved => true,
            Resolved::Decl(e) => match self.heap.env_mut(e) {
                Some(Env::Decl { bindings, .. }) => {
                    if bindings.get(&name).is_some_and(|b| b.deletable) {
                        bindings.remove(&name);
                        true
                    } else {
                        false
                    }
                }
                _ => return internal("not a declarative environment"),
            },
            Resolved::Obj { object, .. } => self.delete(object, &name, false)?,
        };
        self.push(Value::Bool(result))
    }

    fn op_inc_var(&mut self, n: u32, prefix: bool, dec: bool) -> JsResult<()> {
        let name = self.name(n)?;
        let r = self.resolve(&name)?;
        let base = resolved_to_value(&r);
        let (v, _) = self.get_resolved(r, &name)?;
        // The base stays on the stack, where the collector sees it, while ToNumber runs script.
        self.push(base)?;
        let old = self.to_number(&v)?;
        let base = self.pop()?;
        let new = if dec { old - 1.0 } else { old + 1.0 };
        self.set_resolved(value_to_resolved(&base)?, &name, Value::Number(new))?;
        self.push(Value::Number(if prefix { new } else { old }))
    }

    /// Resolves a variable before the right-hand side of an assignment runs, and pushes the
    /// reference's base.
    fn op_ref_var(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let r = self.resolve(&name)?;
        self.push(resolved_to_value(&r))
    }

    /// Replaces the reference base on top of the stack with the variable's value.
    fn op_get_ref(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let base = self.peek(0)?;
        let (v, _) = self.get_resolved(value_to_resolved(&base)?, &name)?;
        self.pop()?;
        self.push(v)
    }

    /// Stores the value on top of the stack through the reference base under it, and leaves the
    /// value.
    fn op_put_ref(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let v = self.peek(0)?;
        let base = self.peek(1)?;
        self.set_resolved(value_to_resolved(&base)?, &name, v)?;
        let v = self.pop()?;
        self.pop()?;
        self.push(v)
    }

    // ---- properties ----

    fn object(&self, id: ObjId) -> JsResult<&Object> {
        self.heap.object(id).ok_or_else(|| Abrupt::Internal("object out of range".into()))
    }

    fn object_mut(&mut self, id: ObjId) -> JsResult<&mut Object> {
        self.heap.object_mut(id).ok_or_else(|| Abrupt::Internal("object out of range".into()))
    }

    /// [[GetOwnProperty]] (section 8.12.1, with the special cases of arrays, strings and
    /// arguments objects).
    pub(crate) fn get_own_property(&self, id: ObjId, k: &[u16]) -> JsResult<Option<Prop>> {
        let o = self.object(id)?;
        match &o.class {
            Class::Array { length, length_writable } if is(k, "length") => {
                return Ok(Some(Prop::Data { value: Value::Number(f64::from(*length)), flags: if *length_writable { WRITABLE } else { 0 } }));
            }
            Class::String(s) => {
                if is(k, "length") {
                    return Ok(Some(Prop::Data { value: Value::Number(s.len() as f64), flags: 0 }));
                }
                if let Some(i) = array_index(k)
                    && (i as usize) < s.len()
                {
                    let unit = s.flat().get(i as usize).copied().unwrap_or(0);
                    return Ok(Some(Prop::Data { value: Value::String(Str::from_units(vec![unit])), flags: ENUMERABLE }));
                }
            }
            Class::Arguments(map) => {
                if let Some(name) = array_index(k).and_then(|i| map.names.get(i as usize)).and_then(Option::as_ref)
                    && let Some(Prop::Data { flags, .. }) = o.props.get(k)
                {
                    let flags = *flags;
                    return Ok(Some(Prop::Data { value: self.binding_value(map.env, name)?, flags }));
                }
            }
            _ => {}
        }
        Ok(o.props.get(k).cloned())
    }

    /// [[GetProperty]]: the property found on the object or its prototype chain.
    pub(crate) fn find_property(&self, id: ObjId, k: &[u16]) -> JsResult<Option<Prop>> {
        let mut cur = Some(id);
        let mut steps = 0usize;
        while let Some(o) = cur {
            if let Some(p) = self.get_own_property(o, k)? {
                return Ok(Some(p));
            }
            cur = self.object(o)?.proto;
            steps += 1;
            if steps > 1_000_000 {
                return internal("prototype chain too long");
            }
        }
        Ok(None)
    }

    pub(crate) fn has_property(&self, id: ObjId, k: &[u16]) -> JsResult<bool> {
        Ok(self.find_property(id, k)?.is_some())
    }

    /// [[Get]] starting at `start`, with `receiver` as `this` for a getter.
    pub(crate) fn get_from(&mut self, start: ObjId, k: &[u16], receiver: &Value) -> JsResult<Value> {
        match self.find_property(start, k)? {
            None | Some(Prop::Accessor { get: None, .. }) => Ok(Value::Undefined),
            Some(Prop::Data { value, .. }) => Ok(value),
            Some(Prop::Accessor { get: Some(g), .. }) => self.call(&Value::Object(g), receiver, &[]),
        }
    }

    /// The prototype a primitive's properties are looked up on.
    fn primitive_proto(&mut self, v: &Value, k: &[u16]) -> JsResult<ObjId> {
        match v {
            Value::Object(o) => Ok(*o),
            Value::String(_) => Ok(self.intrinsics.string_proto),
            Value::Number(_) => Ok(self.intrinsics.number_proto),
            Value::Bool(_) => Ok(self.intrinsics.boolean_proto),
            Value::Undefined | Value::Null => {
                let message = format!(
                    "cannot read property '{}' of {}",
                    String::from_utf16_lossy(k),
                    if matches!(v, Value::Null) { "null" } else { "undefined" }
                );
                Err(self.type_error(&message))
            }
        }
    }

    /// `base[k]` for any value (GetValue, section 8.7.1).
    pub(crate) fn get_value(&mut self, base: &Value, k: &[u16]) -> JsResult<Value> {
        if let Value::String(s) = base {
            if is(k, "length") {
                return Ok(Value::Number(s.len() as f64));
            }
            if let Some(i) = array_index(k)
                && (i as usize) < s.len()
            {
                let unit = s.flat().get(i as usize).copied().unwrap_or(0);
                return Ok(Value::String(Str::from_units(vec![unit])));
            }
        }
        let start = self.primitive_proto(base, k)?;
        self.get_from(start, k, base)
    }

    /// Adds or replaces an own property directly, keeping the heap estimate.
    pub(crate) fn define(&mut self, id: ObjId, k: Key, prop: Prop) -> JsResult<()> {
        let o = self.object_mut(id)?;
        let new = !o.props.contains(&k);
        if let Class::Array { length, .. } = &mut o.class
            && let Some(i) = array_index(&k)
            && i >= *length
        {
            *length = i + 1;
        }
        let name_len = k.len();
        o.props.insert(k, prop);
        if new {
            self.heap.bytes += PROP_BYTES + string_bytes(name_len);
        }
        Ok(())
    }

    fn reject(&mut self, throw: bool, k: &[u16], why: &str) -> JsResult<()> {
        if throw {
            let message = format!("cannot assign to property '{}': {why}", String::from_utf16_lossy(k));
            return Err(self.type_error(&message));
        }
        Ok(())
    }

    /// [[Put]] (section 8.12.5).
    pub(crate) fn put(&mut self, id: ObjId, k: Key, v: Value, throw: bool) -> JsResult<()> {
        match self.get_own_property(id, &k)? {
            Some(Prop::Data { flags, .. }) => {
                if flags & WRITABLE == 0 {
                    return self.reject(throw, &k, "it is read-only");
                }
                return self.write_own(id, k, v, throw);
            }
            Some(Prop::Accessor { set: Some(s), .. }) => return self.call(&Value::Object(s), &Value::Object(id), &[v]).map(drop),
            Some(Prop::Accessor { set: None, .. }) => return self.reject(throw, &k, "it has only a getter"),
            None => {}
        }
        let proto = self.object(id)?.proto;
        match proto.map(|p| self.find_property(p, &k)).transpose()?.flatten() {
            Some(Prop::Accessor { set: Some(s), .. }) => return self.call(&Value::Object(s), &Value::Object(id), &[v]).map(drop),
            Some(Prop::Accessor { set: None, .. }) => return self.reject(throw, &k, "it has only a getter"),
            Some(Prop::Data { flags, .. }) if flags & WRITABLE == 0 => return self.reject(throw, &k, "it is read-only"),
            _ => {}
        }
        let o = self.object(id)?;
        if !o.extensible {
            return self.reject(throw, &k, "the object is not extensible");
        }
        if let Class::Array { length, length_writable: false } = o.class
            && array_index(&k).is_some_and(|i| i >= length)
        {
            return self.reject(throw, &k, "the array's length is read-only");
        }
        self.define(id, k, Prop::Data { value: v, flags: OPEN })
    }

    /// Sets the value of an existing writable own data property.
    fn write_own(&mut self, id: ObjId, k: Key, v: Value, throw: bool) -> JsResult<()> {
        let o = self.object(id)?;
        match &o.class {
            Class::Array { .. } if is(&k, "length") => return self.set_array_length(id, &v, throw),
            Class::Arguments(map) => {
                if let Some(name) = array_index(&k).and_then(|i| map.names.get(i as usize)).and_then(Option::as_ref) {
                    let (env, name) = (map.env, name.clone());
                    if let Some(Env::Decl { bindings, .. }) = self.heap.env_mut(env)
                        && let Some(b) = bindings.get_mut(&name)
                    {
                        b.value = v.clone();
                    }
                }
            }
            _ => {}
        }
        match self.object_mut(id)?.props.get_mut(&k) {
            Some(Prop::Data { value, .. }) => {
                *value = v;
                Ok(())
            }
            _ => internal("own data property vanished"),
        }
    }

    /// Assigning to an array's `length` (section 15.4.5.1).
    fn set_array_length(&mut self, id: ObjId, v: &Value, throw: bool) -> JsResult<()> {
        let n = self.to_number(v)?;
        let new = to_uint32(n);
        if f64::from(new) != n {
            return Err(self.throw(ErrorType::Range, "invalid array length"));
        }
        let o = self.object(id)?;
        let Class::Array { length: old, length_writable } = o.class else { return internal("not an array") };
        if !length_writable {
            return self.reject(throw, &key("length"), "it is read-only");
        }
        if new < old {
            // Shrinking visits every property: each takes fuel before anything is removed.
            for _ in 0..o.props.len() {
                self.use_fuel()?;
            }
        }
        let o = self.object_mut(id)?;
        let Class::Array { length, .. } = &mut o.class else { return internal("not an array") };
        *length = new;
        if new < old {
            let doomed: Vec<Key> = o.props.keys().into_iter().filter(|k| array_index(k).is_some_and(|i| i >= new)).collect();
            for k in doomed {
                o.props.remove(&k);
            }
        }
        Ok(())
    }

    /// [[Delete]] (section 8.12.7).
    pub(crate) fn delete(&mut self, id: ObjId, k: &[u16], throw: bool) -> JsResult<bool> {
        match self.get_own_property(id, k)? {
            None => Ok(true),
            Some(p) if p.configurable() => {
                let o = self.object_mut(id)?;
                o.props.remove(k);
                if let Class::Arguments(map) = &mut o.class
                    && let Some(slot) = array_index(k).and_then(|i| map.names.get_mut(i as usize))
                {
                    *slot = None;
                }
                Ok(true)
            }
            Some(_) if throw => {
                let message = format!("cannot delete property '{}'", String::from_utf16_lossy(k));
                Err(self.type_error(&message))
            }
            Some(_) => Ok(false),
        }
    }

    /// PutValue on a property reference (section 8.7.2), whose base may be a primitive.
    fn put_value(&mut self, base: &Value, k: Key, v: Value) -> JsResult<()> {
        let strict = self.strict();
        match base {
            Value::Object(o) => self.put(*o, k, v, strict),
            Value::Undefined | Value::Null => {
                let message = format!(
                    "cannot set property '{}' of {}",
                    String::from_utf16_lossy(&k),
                    if matches!(base, Value::Null) { "null" } else { "undefined" }
                );
                Err(self.type_error(&message))
            }
            _ => {
                let proto = self.primitive_proto(base, &k)?;
                let own = matches!(base, Value::String(s) if is(&k, "length") || array_index(&k).is_some_and(|i| (i as usize) < s.len()));
                if !own && let Some(Prop::Accessor { set: Some(s), .. }) = self.find_property(proto, &k)? {
                    return self.call(&Value::Object(s), base, &[v]).map(drop);
                }
                self.reject(strict, &k, "the value is a primitive")
            }
        }
    }

    fn op_get_member(&mut self, n: u32, with_this: bool) -> JsResult<()> {
        let name = self.name(n)?;
        let base = self.peek(0)?;
        let v = self.get_value(&base, &name)?;
        self.replace(1, v)?;
        if with_this { self.push(base) } else { Ok(()) }
    }

    fn op_set_member(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let v = self.peek(0)?;
        let base = self.peek(1)?;
        self.put_value(&base, name, v.clone())?;
        self.replace(2, v)
    }

    fn check_coercible(&mut self, base: &Value, k: &[u16]) -> JsResult<()> {
        if base.is_nullish() {
            self.primitive_proto(base, k)?;
        }
        Ok(())
    }

    fn op_check_object(&mut self) -> JsResult<()> {
        let base = self.peek(0)?;
        self.check_coercible(&base, &[])
    }

    fn op_to_key(&mut self) -> JsResult<()> {
        let k = self.peek(0)?;
        let base = self.peek(1)?;
        self.check_coercible(&base, &[])?;
        let name = self.to_key(&k)?;
        self.replace(1, Value::String(Str::from_rc(name)))
    }

    fn op_get_index(&mut self, with_this: bool) -> JsResult<()> {
        let k = self.peek(0)?;
        let base = self.peek(1)?;
        self.check_coercible(&base, &[])?;
        let name = self.to_key(&k)?;
        let v = self.get_value(&base, &name)?;
        self.replace(2, v)?;
        if with_this { self.push(base) } else { Ok(()) }
    }

    fn op_set_index(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let k = self.peek(1)?;
        let base = self.peek(2)?;
        let name = self.to_key(&k)?;
        self.put_value(&base, name, v.clone())?;
        self.replace(3, v)
    }

    fn op_delete_member(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let base = self.peek(0)?;
        let o = self.to_object(&base)?;
        let strict = self.strict();
        let r = self.delete(o, &name, strict)?;
        self.replace(1, Value::Bool(r))
    }

    fn op_delete_index(&mut self) -> JsResult<()> {
        let k = self.peek(0)?;
        let base = self.peek(1)?;
        self.check_coercible(&base, &[])?;
        let name = self.to_key(&k)?;
        let o = self.to_object(&base)?;
        let strict = self.strict();
        let r = self.delete(o, &name, strict)?;
        self.replace(2, Value::Bool(r))
    }

    fn inc_property(&mut self, base: &Value, name: Key, prefix: bool, dec: bool) -> JsResult<Value> {
        let v = self.get_value(base, &name)?;
        let old = self.to_number(&v)?;
        let new = if dec { old - 1.0 } else { old + 1.0 };
        self.put_value(base, name, Value::Number(new))?;
        Ok(Value::Number(if prefix { new } else { old }))
    }

    fn op_inc_member(&mut self, n: u32, prefix: bool, dec: bool) -> JsResult<()> {
        let name = self.name(n)?;
        let base = self.peek(0)?;
        let r = self.inc_property(&base, name, prefix, dec)?;
        self.replace(1, r)
    }

    fn op_inc_index(&mut self, prefix: bool, dec: bool) -> JsResult<()> {
        let k = self.peek(0)?;
        let base = self.peek(1)?;
        self.check_coercible(&base, &[])?;
        let name = self.to_key(&k)?;
        let r = self.inc_property(&base, name, prefix, dec)?;
        self.replace(2, r)
    }

    // ---- object literals and functions ----

    fn op_new_object(&mut self) -> JsResult<()> {
        let o = self.new_object();
        self.push(Value::Object(o))
    }

    pub(crate) fn new_object(&mut self) -> ObjId {
        let proto = self.intrinsics.object_proto;
        self.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)))
    }

    fn op_new_array(&mut self) -> JsResult<()> {
        let proto = self.intrinsics.array_proto;
        let a = self.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: true }, Some(proto)));
        self.push(Value::Object(a))
    }

    fn op_array_push(&mut self, with_value: bool) -> JsResult<()> {
        let v = if with_value { Some(self.pop()?) } else { None };
        let Value::Object(a) = self.peek(0)? else { return internal("array literal is not an object") };
        let o = self.object_mut(a)?;
        let Class::Array { length, .. } = &mut o.class else { return internal("array literal is not an array") };
        let i = *length;
        if i == u32::MAX - 1 {
            return Err(self.throw(ErrorType::Range, "array literal too long"));
        }
        *length = i + 1;
        match v {
            Some(v) => self.define(a, index_key(i), Prop::Data { value: v, flags: OPEN }),
            None => Ok(()),
        }
    }

    fn op_init_prop(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let v = self.pop()?;
        let Value::Object(o) = self.peek(0)? else { return internal("object literal is not an object") };
        self.define(o, name, Prop::Data { value: v, flags: OPEN })
    }

    fn op_init_accessor(&mut self, n: u32, getter: bool) -> JsResult<()> {
        let name = self.name(n)?;
        let Value::Object(f) = self.pop()? else { return internal("accessor is not a function") };
        let Value::Object(o) = self.peek(0)? else { return internal("object literal is not an object") };
        let (mut get, mut set) = match self.object(o)?.props.get(&name) {
            Some(Prop::Accessor { get, set, .. }) => (*get, *set),
            _ => (None, None),
        };
        if getter {
            get = Some(f);
        } else {
            set = Some(f);
        }
        self.define(o, name, Prop::Accessor { get, set, flags: ENUMERABLE | CONFIGURABLE })
    }

    fn op_closure(&mut self, i: u32) -> JsResult<()> {
        let frame = self.frame()?;
        let env = frame.env;
        let Some(code) = frame.code.functions.get(i as usize).cloned() else { return internal("function out of range") };
        let f = self.make_closure(code, env)?;
        self.push(Value::Object(f))
    }

    /// A function object for `code` closing over `env` (section 13.2).
    pub(crate) fn make_closure(&mut self, code: Rc<Code>, env: EnvId) -> JsResult<ObjId> {
        let env = match &code.self_name {
            Some(_) => self.heap.alloc_env(Env::Decl { bindings: PropMap::default(), outer: Some(env) }),
            None => env,
        };
        let proto = self.intrinsics.function_proto;
        let params = code.params.len();
        let strict = code.strict;
        let self_name = code.self_name.clone();
        let f = self.heap.alloc_object(Object::new(Class::Function(Func::Closure { code, env }), Some(proto)));
        self.define(f, key("length"), Prop::Data { value: Value::Number(params as f64), flags: 0 })?;
        let p = self.new_object();
        self.define(p, key("constructor"), Prop::Data { value: Value::Object(f), flags: HIDDEN })?;
        self.define(f, key("prototype"), Prop::Data { value: Value::Object(p), flags: WRITABLE })?;
        if strict {
            let thrower = Some(self.intrinsics.throw_type_error);
            for name in ["caller", "arguments"] {
                self.define(f, key(name), Prop::Accessor { get: thrower, set: thrower, flags: 0 })?;
            }
        }
        if let Some(name) = self_name {
            self.env_insert(env, name, Binding { value: Value::Object(f), mutable: false, deletable: false })?;
        }
        Ok(f)
    }

    fn env_insert(&mut self, env: EnvId, name: Key, b: Binding) -> JsResult<()> {
        let Some(Env::Decl { bindings, .. }) = self.heap.env_mut(env) else { return internal("not a declarative environment") };
        let new = !bindings.contains(&name);
        let name_len = name.len();
        bindings.insert(name, b);
        if new {
            self.heap.bytes += PROP_BYTES + string_bytes(name_len);
        }
        Ok(())
    }

    fn op_regexp(&mut self, i: u32) -> JsResult<()> {
        let Some((source, flags)) = self.frame()?.code.regexps.get(i as usize).cloned() else { return internal("regular expression out of range") };
        let r = crate::regexp_builtins::new_regexp(self, &source, &flags)?;
        self.push(Value::Object(r))
    }

    // ---- environments ----

    fn op_push_with(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let object = self.to_object(&v)?;
        self.pop()?;
        let outer = self.frame()?.env;
        let env = self.heap.alloc_env(Env::Obj { object, with_this: true, outer: Some(outer) });
        let frame = self.frame_mut()?;
        frame.env = env;
        frame.env_depth += 1;
        Ok(())
    }

    fn op_push_catch(&mut self, n: u32) -> JsResult<()> {
        let name = self.name(n)?;
        let v = self.pop()?;
        let outer = self.frame()?.env;
        let mut bindings = PropMap::default();
        let name_bytes = string_bytes(name.len());
        bindings.insert(name, Binding { value: v, mutable: true, deletable: false });
        let env = self.heap.alloc_env(Env::Decl { bindings, outer: Some(outer) });
        self.heap.bytes += name_bytes;
        let frame = self.frame_mut()?;
        frame.env = env;
        frame.env_depth += 1;
        Ok(())
    }

    fn op_pop_env(&mut self) -> JsResult<()> {
        let env = self.frame()?.env;
        let Some(outer) = self.heap.env(env).and_then(Env::outer) else { return internal("left the outermost environment") };
        let frame = self.frame_mut()?;
        if frame.env_depth == 0 {
            return internal("left more environments than were entered");
        }
        frame.env = outer;
        frame.env_depth -= 1;
        Ok(())
    }

    // ---- for-in ----

    /// The names `for-in` visits (section 12.6.4): the enumerable properties of the object and
    /// its prototypes, own properties in the order they were added, each name once, and none
    /// that an object nearer the start of the chain has (enumerable or not).
    ///
    /// For a String object, the heap budget for all its index names is checked before any is
    /// built. Each index name and each property visited takes a unit of fuel.
    fn enumerable_keys(&mut self, id: ObjId) -> JsResult<Vec<Key>> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(o) = cur {
            let obj = self.object(o)?;
            let string_len = if let Class::String(s) = &obj.class { Some(s.len()) } else { None };
            let is_array = matches!(obj.class, Class::Array { .. });
            if let Some(len) = string_len {
                self.reserve_heap(index_keys_bytes(len))?;
                for i in 0..len {
                    self.use_fuel()?;
                    let k = index_key(u32::try_from(i).unwrap_or(u32::MAX));
                    if seen.insert(k.clone()) {
                        out.push(k);
                    }
                }
                seen.insert(key("length"));
            }
            if is_array {
                seen.insert(key("length"));
            }
            for _ in 0..self.object(o)?.props.len() {
                self.use_fuel()?;
            }
            let obj = self.object(o)?;
            for (k, p) in obj.props.iter() {
                if seen.insert(k.clone()) && p.enumerable() {
                    out.push(k.clone());
                }
            }
            cur = obj.proto;
        }
        Ok(out)
    }

    fn op_for_in_start(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let (keys, object) = if v.is_nullish() {
            (Vec::new(), None)
        } else {
            let o = self.to_object(&v)?;
            // A wrapper made by ToObject is not on the stack; a collection while the names are
            // listed or their bytes reserved must not free it.
            let saved = self.temp_roots.len();
            self.temp_roots.push(Value::Object(o));
            let keys = self.enumerable_keys(o).and_then(|keys| {
                let bytes = keys.iter().map(|k| string_bytes(k.len())).sum::<usize>();
                self.reserve_heap(bytes)?;
                self.heap.bytes = self.heap.bytes.saturating_add(bytes);
                Ok(keys)
            });
            self.temp_roots.truncate(saved);
            (keys?, Some(o))
        };
        let iter = self.heap.alloc_object(Object::new(Class::ForIn(Box::new(ForIn { keys, pos: 0, object })), None));
        self.replace(1, Value::Object(iter))
    }

    fn op_for_in_next(&mut self, target: u32) -> JsResult<()> {
        let Value::Object(iter) = self.peek(0)? else { return internal("for-in iterator is not an object") };
        loop {
            let Class::ForIn(state) = &mut self.object_mut(iter)?.class else { return internal("not a for-in iterator") };
            let Some(k) = state.keys.get(state.pos).cloned() else { return self.jump_to(target) };
            state.pos += 1;
            let object = state.object;
            if let Some(o) = object
                && self.has_property(o, &k)?
            {
                return self.push(Value::String(Str::from_rc(k)));
            }
        }
    }

    // ---- calls ----

    pub(crate) fn is_callable(&self, v: &Value) -> bool {
        v.as_object().and_then(|o| self.heap.object(o)).is_some_and(|o| matches!(o.class, Class::Function(_)))
    }

    fn func_of(&self, v: &Value) -> Option<Func> {
        match &self.heap.object(v.as_object()?)?.class {
            Class::Function(f) => Some(f.clone()),
            _ => None,
        }
    }

    /// Resolves `call` and `apply` and runs native and host functions; a function written in
    /// JavaScript comes back to be entered by the caller.
    fn prepare_call(&mut self, mut f: Value, mut this: Value, mut args: Vec<Value>) -> JsResult<Prepared> {
        loop {
            let Some(func) = self.func_of(&f) else { return Err(self.type_error("the value is not a function")) };
            match func {
                Func::Closure { code, env } => {
                    let callee = f.as_object().ok_or_else(|| Abrupt::Internal("callee is not an object".into()))?;
                    return Ok(Prepared::Closure(Entry { callee, code, env, this, args }));
                }
                Func::Native { kind: NativeKind::Call, .. } => {
                    self.use_fuel()?;
                    f = this;
                    this = if args.is_empty() { Value::Undefined } else { args.remove(0) };
                }
                Func::Native { kind: NativeKind::Apply, .. } => {
                    self.use_fuel()?;
                    if !self.is_callable(&this) {
                        return Err(self.type_error("apply was called on a value that is not a function"));
                    }
                    let list = args.get(1).cloned().unwrap_or(Value::Undefined);
                    f = this;
                    this = args.first().cloned().unwrap_or(Value::Undefined);
                    args = self.list_from_array_like(&list)?;
                }
                Func::Native { f: native, .. } => {
                    // A list from `apply` is not on the operand stack, and a native function can run
                    // JavaScript that collects, so the arguments stay rooted for the call.
                    let mark = self.temp_roots.len();
                    self.temp_roots.push(this.clone());
                    self.temp_roots.extend(args.iter().cloned());
                    let r = native(self, this, &args, false);
                    self.temp_roots.truncate(mark);
                    return Ok(Prepared::Done(r?));
                }
                Func::Host { f: host, .. } => {
                    let js_args: Vec<crate::JsValue> = args.iter().map(|v| self.to_js_value(v)).collect();
                    return match host(&js_args) {
                        Ok(v) => Ok(Prepared::Done(self.from_js_value(v)?)),
                        Err(message) => {
                            let bytes = string_bytes(message.encode_utf16().count());
                            self.reserve_heap(bytes)?;
                            self.heap.bytes += bytes;
                            Err(self.throw(ErrorType::Error, &message))
                        }
                    };
                }
            }
        }
    }

    /// The elements of an array-like `apply` argument (section 15.3.4.3).
    fn list_from_array_like(&mut self, list: &Value) -> JsResult<Vec<Value>> {
        let Value::Object(o) = list else {
            if list.is_nullish() {
                return Ok(Vec::new());
            }
            return Err(self.type_error("the argument list is not an object"));
        };
        let len = self.get_from(*o, &key("length"), list)?;
        let n = to_uint32(self.to_number(&len)?) as usize;
        if n > MAX_APPLY_ARGS {
            return Err(self.throw(ErrorType::Range, "too many arguments"));
        }
        let saved = self.temp_roots.len();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            // Each element read takes fuel, so a long array-like list stays within the run's limits.
            match self.use_fuel().and_then(|()| self.get_from(*o, &index_key(u32::try_from(i).unwrap_or(u32::MAX)), list)) {
                Ok(v) => {
                    self.temp_roots.push(v.clone());
                    out.push(v);
                }
                Err(e) => {
                    self.temp_roots.truncate(saved);
                    return Err(e);
                }
            }
        }
        self.temp_roots.truncate(saved);
        Ok(out)
    }

    fn op_call(&mut self, argc: u32, eval: bool) -> JsResult<()> {
        let n = argc as usize;
        let start =
            self.stack.len().checked_sub(n + 2).filter(|&s| s >= self.base()).ok_or_else(|| Abrupt::Internal("call below the stack".into()))?;
        let f = self.stack.get(start).cloned().unwrap_or(Value::Undefined);
        let this = self.stack.get(start + 1).cloned().unwrap_or(Value::Undefined);
        let args: Vec<Value> = self.stack.get(start + 2..).map(<[Value]>::to_vec).unwrap_or_default();
        if eval && f.as_object() == Some(self.intrinsics.eval) {
            return self.direct_eval(start, args);
        }
        match self.prepare_call(f, this, args)? {
            Prepared::Done(v) => {
                self.stack.truncate(start);
                self.push(v)
            }
            Prepared::Closure(entry) => {
                self.stack.truncate(start);
                self.enter_function(entry, None, false)
            }
        }
    }

    fn op_new(&mut self, argc: u32) -> JsResult<()> {
        let n = argc as usize;
        let start =
            self.stack.len().checked_sub(n + 1).filter(|&s| s >= self.base()).ok_or_else(|| Abrupt::Internal("new below the stack".into()))?;
        let f = self.stack.get(start).cloned().unwrap_or(Value::Undefined);
        let args: Vec<Value> = self.stack.get(start + 1..).map(<[Value]>::to_vec).unwrap_or_default();
        match self.func_of(&f) {
            Some(Func::Closure { code, env }) => {
                let Some(callee) = f.as_object() else { return internal("callee is not an object") };
                let proto = match self.get_from(callee, &key("prototype"), &f)? {
                    Value::Object(p) => p,
                    _ => self.intrinsics.object_proto,
                };
                let obj = self.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)));
                self.stack.truncate(start);
                self.enter_function(Entry { callee, code, env, this: Value::Object(obj), args }, Some(obj), false)
            }
            Some(Func::Native { f: native, constructor: true, .. }) => {
                let v = native(self, Value::Undefined, &args, true)?;
                self.stack.truncate(start);
                self.push(v)
            }
            _ => Err(self.type_error("the value is not a constructor")),
        }
    }

    /// Pushes a frame for a call to a function written in JavaScript, its operands already off
    /// the stack.
    fn enter_function(&mut self, entry: Entry, construct: Option<ObjId>, boundary: bool) -> JsResult<()> {
        let Entry { callee, code, env, this, args } = entry;
        if self.frames.len() >= self.limits.frames {
            return Err(Abrupt::Limit(LimitKind::Frames));
        }
        let params = code.params.iter().fold(0usize, |total, p| total.saturating_add(binding_bytes(p.len())));
        let arguments =
            if code.uses_arguments { OBJECT_BYTES.saturating_add(args.len().saturating_add(4).saturating_mul(binding_bytes(10))) } else { 0 };
        let need = declaration_bytes(&code).saturating_add(ENV_BYTES).saturating_add(params).saturating_add(arguments);
        if self.heap.bytes.saturating_add(need) > self.limits.heap_bytes {
            let mark = self.temp_roots.len();
            self.temp_roots.push(Value::Object(callee));
            self.temp_roots.push(this.clone());
            self.temp_roots.extend(construct.map(Value::Object));
            self.temp_roots.extend(args.iter().cloned());
            let reserved = self.reserve_heap(need);
            self.temp_roots.truncate(mark);
            reserved?;
        }
        let this = if code.strict {
            this
        } else {
            match this {
                Value::Undefined | Value::Null => Value::Object(self.global),
                Value::Object(_) => this,
                prim => Value::Object(self.to_object(&prim)?),
            }
        };
        let fenv = self.instantiate_function(&code, env, &args, callee)?;
        let base = self.stack.len();
        self.frames.push(Frame {
            code,
            pc: 0,
            base,
            env: fenv,
            var_env: fenv,
            env_depth: 0,
            this,
            callee: Some(callee),
            construct,
            completion: Value::Undefined,
            boundary,
        });
        Ok(())
    }

    /// Declaration binding instantiation for function code (section 10.5). Each parameter and
    /// declaration takes a unit of fuel.
    fn instantiate_function(&mut self, code: &Rc<Code>, outer: EnvId, args: &[Value], callee: ObjId) -> JsResult<EnvId> {
        let mut bindings = PropMap::default();
        for (i, p) in code.params.iter().enumerate() {
            self.use_fuel()?;
            bindings.insert(p.clone(), Binding { value: args.get(i).cloned().unwrap_or(Value::Undefined), mutable: true, deletable: false });
        }
        let name_bytes = bindings.iter().map(|(k, _)| string_bytes(k.len())).sum::<usize>();
        let env = self.heap.alloc_env(Env::Decl { bindings, outer: Some(outer) });
        self.heap.bytes += name_bytes;
        for (name, fi) in &code.fun_decls {
            self.use_fuel()?;
            let Some(fcode) = code.functions.get(*fi as usize).cloned() else { return internal("function out of range") };
            let f = self.make_closure(fcode, env)?;
            self.env_insert(env, name.clone(), Binding { value: Value::Object(f), mutable: true, deletable: false })?;
        }
        let has = |interp: &Self, name: &[u16]| matches!(interp.heap.env(env), Some(Env::Decl { bindings, .. }) if bindings.contains(name));
        if code.uses_arguments && !has(self, &key("arguments")) {
            let a = self.create_arguments(code, env, args, callee)?;
            self.env_insert(env, key("arguments"), Binding { value: Value::Object(a), mutable: !code.strict, deletable: false })?;
        }
        for v in &code.vars {
            self.use_fuel()?;
            if !has(self, v) {
                self.env_insert(env, v.clone(), Binding { value: Value::Undefined, mutable: true, deletable: false })?;
            }
        }
        Ok(env)
    }

    /// The `arguments` object (section 10.6).
    fn create_arguments(&mut self, code: &Code, env: EnvId, args: &[Value], callee: ObjId) -> JsResult<ObjId> {
        let mut names = vec![None; args.len()];
        if !code.strict {
            let mut mapped = std::collections::HashSet::new();
            for (i, p) in code.params.iter().enumerate().take(args.len()).rev() {
                if mapped.insert(p.clone())
                    && let Some(slot) = names.get_mut(i)
                {
                    *slot = Some(p.clone());
                }
            }
        }
        let proto = self.intrinsics.object_proto;
        let a = self.heap.alloc_object(Object::new(Class::Arguments(Box::new(ArgMap { env, names })), Some(proto)));
        for (i, v) in args.iter().enumerate() {
            self.define(a, index_key(u32::try_from(i).unwrap_or(u32::MAX)), Prop::Data { value: v.clone(), flags: OPEN })?;
        }
        self.define(a, key("length"), Prop::Data { value: Value::Number(args.len() as f64), flags: HIDDEN })?;
        if code.strict {
            let thrower = Some(self.intrinsics.throw_type_error);
            for name in ["callee", "caller"] {
                self.define(a, key(name), Prop::Accessor { get: thrower, set: thrower, flags: 0 })?;
            }
        } else {
            self.define(a, key("callee"), Prop::Data { value: Value::Object(callee), flags: HIDDEN })?;
        }
        Ok(a)
    }

    /// Calls `f` from Rust and returns its result, running the loop again for a function written
    /// in JavaScript.
    pub(crate) fn call(&mut self, f: &Value, this: &Value, args: &[Value]) -> JsResult<Value> {
        if self.reentry >= self.limits.reentry {
            return Err(Abrupt::Limit(LimitKind::Reentry));
        }
        let saved = self.stack.len();
        self.stack.push(f.clone());
        self.stack.push(this.clone());
        self.stack.extend(args.iter().cloned());
        self.reentry += 1;
        let r = self.call_rooted(f.clone(), this.clone(), args.to_vec(), saved);
        self.reentry -= 1;
        self.stack.truncate(saved);
        r
    }

    fn call_rooted(&mut self, f: Value, this: Value, args: Vec<Value>, saved: usize) -> JsResult<Value> {
        match self.prepare_call(f, this, args)? {
            Prepared::Done(v) => Ok(v),
            Prepared::Closure(entry) => {
                self.stack.truncate(saved);
                self.enter_function(entry, None, true)?;
                self.execute()
            }
        }
    }

    // ---- eval and scripts ----

    /// The options for source parsed next in this run: [`Options::max_source_units`] is what
    /// the run may still parse.
    fn remaining_source_options(&self) -> Options {
        Options { max_source_units: self.options.max_source_units.saturating_sub(self.source_units), ..self.options.clone() }
    }

    /// Checks that `units` more code units of source fit in what this run may still parse,
    /// before the source is flattened or encoded.
    pub(crate) fn check_source_units(&self, units: usize) -> Result<(), CompileError> {
        crate::check_length(units, &self.remaining_source_options()).map_err(CompileError::Syntax)
    }

    /// Encodes `text` as UTF-16 for parsing, after checking that it fits in what this run may
    /// still parse.
    pub(crate) fn encode_source(&self, text: &str) -> Result<Vec<u16>, CompileError> {
        crate::to_utf16(text, &self.remaining_source_options()).map_err(CompileError::Syntax)
    }

    /// Parses and compiles source for eval code or a script, counting it against the source
    /// limit.
    pub(crate) fn compile_source(&mut self, units: &[u16], strict: bool, kind: CodeKind) -> Result<Rc<Code>, CompileError> {
        self.source_units = self.source_units.saturating_add(units.len());
        let remaining = self.options.max_source_units.saturating_sub(self.source_units.saturating_sub(units.len()));
        let options = Options { strict, max_source_units: remaining, ..self.options.clone() };
        let program = crate::parse_utf16(units, &options).map_err(CompileError::Syntax)?;
        let source = Rc::new(Source { lines: program.lines.clone(), text: units.into() });
        compile_program(&program, &source, kind).map_err(|e| match e {
            CompileFail::Syntax(s) => CompileError::Syntax(s),
            CompileFail::Internal(m) => CompileError::Internal(m),
        })
    }

    /// Turns a compile failure inside a running script into what it throws.
    pub(crate) fn compile_error_to_abrupt(&mut self, e: CompileError) -> Abrupt {
        match e {
            CompileError::Syntax(s) => self.throw(ErrorType::Syntax, &s.message),
            CompileError::Internal(m) => Abrupt::Internal(m),
        }
    }

    fn direct_eval(&mut self, start: usize, args: Vec<Value>) -> JsResult<()> {
        let x = args.into_iter().next().unwrap_or(Value::Undefined);
        let Value::String(s) = &x else {
            self.stack.truncate(start);
            return self.push(x);
        };
        let strict = self.strict();
        self.check_source_units(s.len()).map_err(|e| self.compile_error_to_abrupt(e))?;
        let units = s.flat();
        let code = self.compile_source(&units, strict, CodeKind::Eval).map_err(|e| self.compile_error_to_abrupt(e))?;
        self.charge_code(&code, declaration_bytes(&code).saturating_add(ENV_BYTES))?;
        let frame = self.frame()?;
        let (caller_env, caller_var_env, this) = (frame.env, frame.var_env, frame.this.clone());
        let (env, var_env) = if code.strict {
            let e = self.heap.alloc_env(Env::Decl { bindings: PropMap::default(), outer: Some(caller_env) });
            (e, e)
        } else {
            (caller_env, caller_var_env)
        };
        self.stack.truncate(start);
        self.instantiate_global(&code, var_env, true)?;
        self.push_code_frame(code, env, var_env, this, false)
    }

    /// Indirect eval and scripts: runs `code` in the global environment and returns its
    /// completion value.
    pub(crate) fn run_global(&mut self, code: Rc<Code>, is_eval: bool) -> JsResult<Value> {
        // Indirect eval runs inside an operation and counts against the re-entry limit; the
        // script a realm runs does not.
        let nested = usize::from(is_eval);
        if is_eval && self.reentry >= self.limits.reentry {
            return Err(Abrupt::Limit(LimitKind::Reentry));
        }
        self.charge_code(&code, declaration_bytes(&code).saturating_add(ENV_BYTES))?;
        let (env, global) = (self.global_env, self.global);
        let (env, var_env) = if code.strict && is_eval {
            let e = self.heap.alloc_env(Env::Decl { bindings: PropMap::default(), outer: Some(env) });
            (e, e)
        } else {
            (env, env)
        };
        self.instantiate_global(&code, var_env, is_eval)?;
        self.push_code_frame(code, env, var_env, Value::Object(global), true)?;
        self.reentry += nested;
        let r = self.execute();
        self.reentry -= nested;
        r
    }

    fn push_code_frame(&mut self, code: Rc<Code>, env: EnvId, var_env: EnvId, this: Value, boundary: bool) -> JsResult<()> {
        if self.frames.len() >= self.limits.frames {
            return Err(Abrupt::Limit(LimitKind::Frames));
        }
        let base = self.stack.len();
        self.frames.push(Frame {
            code,
            pc: 0,
            base,
            env,
            var_env,
            env_depth: 0,
            this,
            callee: None,
            construct: None,
            completion: Value::Undefined,
            boundary,
        });
        Ok(())
    }

    /// Declaration binding instantiation for global and eval code (section 10.5). The caller has
    /// reserved [`declaration_bytes`] of `code`; each declaration takes a unit of fuel.
    fn instantiate_global(&mut self, code: &Rc<Code>, var_env: EnvId, is_eval: bool) -> JsResult<()> {
        let configurable = if is_eval { CONFIGURABLE } else { 0 };
        for (name, fi) in &code.fun_decls {
            self.use_fuel()?;
            let Some(fcode) = code.functions.get(*fi as usize).cloned() else { return internal("function out of range") };
            let f = self.make_closure(fcode, var_env)?;
            self.declare(var_env, name, Some(Value::Object(f)), configurable)?;
        }
        for v in &code.vars {
            self.use_fuel()?;
            self.declare(var_env, v, None, configurable)?;
        }
        Ok(())
    }

    /// Declares `name` in a variable environment, with `value` for a function declaration.
    fn declare(&mut self, env: EnvId, name: &Key, value: Option<Value>, configurable: u8) -> JsResult<()> {
        match self.heap.env(env) {
            Some(Env::Decl { bindings, .. }) => {
                let exists = bindings.contains(name);
                match value {
                    Some(v) => self.env_insert(env, name.clone(), Binding { value: v, mutable: true, deletable: configurable != 0 }),
                    None if !exists => {
                        self.env_insert(env, name.clone(), Binding { value: Value::Undefined, mutable: true, deletable: configurable != 0 })
                    }
                    None => Ok(()),
                }
            }
            Some(Env::Obj { object, .. }) => {
                let object = *object;
                let declared = self.has_property(object, name)?;
                let flags = WRITABLE | ENUMERABLE | configurable;
                match value {
                    None if declared => Ok(()),
                    None => self.define(object, name.clone(), Prop::Data { value: Value::Undefined, flags }),
                    Some(v) => {
                        let existing = self.find_property(object, name)?;
                        match existing {
                            None => self.define(object, name.clone(), Prop::Data { value: v, flags }),
                            Some(p) if p.configurable() => self.define(object, name.clone(), Prop::Data { value: v, flags }),
                            Some(Prop::Data { flags: f, .. }) if f & (WRITABLE | ENUMERABLE) == WRITABLE | ENUMERABLE => {
                                self.put(object, name.clone(), v, true)
                            }
                            Some(_) => {
                                let message = format!("cannot declare function {}", String::from_utf16_lossy(name));
                                Err(self.type_error(&message))
                            }
                        }
                    }
                }
            }
            None => internal("environment out of range"),
        }
    }

    // ---- conversions ----

    pub(crate) fn type_of(&self, v: &Value) -> &'static str {
        match v {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Object(_) if self.is_callable(v) => "function",
            Value::Object(_) => "object",
        }
    }

    /// ToPrimitive (section 9.1) through [[DefaultValue]] (section 8.12.8).
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_primitive(&mut self, v: &Value, hint: Hint) -> JsResult<Value> {
        let Value::Object(o) = v else { return Ok(v.clone()) };
        // Without a hint, a Date object converts as with the String hint.
        let date = hint == Hint::Default && matches!(self.heap.object(*o).map(|ob| &ob.class), Some(Class::Date(_)));
        let order = if hint == Hint::String || date { ["toString", "valueOf"] } else { ["valueOf", "toString"] };
        for name in order {
            let f = self.get_from(*o, &key(name), v)?;
            if self.is_callable(&f) {
                let r = self.call(&f, v, &[])?;
                if !matches!(r, Value::Object(_)) {
                    return Ok(r);
                }
            }
        }
        Err(self.type_error("cannot convert the object to a primitive value"))
    }

    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_number(&mut self, v: &Value) -> JsResult<f64> {
        Ok(match v {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Number(n) => *n,
            Value::String(s) => string_to_number(&s.flat()),
            Value::Object(_) => {
                let p = self.to_primitive(v, Hint::Number)?;
                return self.to_number(&p);
            }
        })
    }

    /// ToString applied to a primitive.
    pub(crate) fn primitive_to_str(&mut self, v: &Value) -> Str {
        match v {
            Value::Undefined => Str::from("undefined"),
            Value::Null => Str::from("null"),
            Value::Bool(b) => Str::from(if *b { "true" } else { "false" }),
            Value::Number(n) => {
                let s = number_to_str(*n);
                self.heap.bytes += string_bytes(s.len());
                s
            }
            Value::String(s) => s.clone(),
            Value::Object(_) => Str::from("[object]"),
        }
    }

    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_str(&mut self, v: &Value) -> JsResult<Str> {
        let p = self.to_primitive(v, Hint::String)?;
        Ok(self.primitive_to_str(&p))
    }

    /// A property name from a value (ToString, with a short way for array indices).
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_key(&mut self, v: &Value) -> JsResult<Key> {
        match v {
            Value::String(s) => {
                let rope = s.is_rope();
                let flat = s.flat();
                if rope {
                    self.heap.bytes += string_bytes(flat.len());
                }
                Ok(flat)
            }
            Value::Number(n) if *n >= 0.0 && *n < 4_294_967_295.0 && n.fract() == 0.0 => Ok(index_key(*n as u32)),
            _ => Ok(self.to_str(v)?.flat()),
        }
    }

    /// ToObject (section 9.9).
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_object(&mut self, v: &Value) -> JsResult<ObjId> {
        let (class, proto) = match v {
            Value::Object(o) => return Ok(*o),
            Value::Undefined | Value::Null => return Err(self.type_error("cannot convert undefined or null to an object")),
            Value::Bool(_) => (Class::Boolean, self.intrinsics.boolean_proto),
            Value::Number(n) => (Class::Number(*n), self.intrinsics.number_proto),
            Value::String(s) => (Class::String(s.clone()), self.intrinsics.string_proto),
        };
        Ok(self.heap.alloc_object(Object::new(class, Some(proto))))
    }

    /// Concatenates two strings, ending the run with the heap limit when the result could not
    /// be flattened within it.
    pub(crate) fn concat(&mut self, a: &Str, b: &Str) -> JsResult<Str> {
        let len = a.len() + b.len();
        if len > self.limits.heap_bytes / 4 {
            return Err(Abrupt::Limit(LimitKind::Heap));
        }
        self.heap.bytes += ROPE_BYTES;
        Ok(Str::concat(a, b))
    }

    // ---- operators ----

    fn op_unary(&mut self, op: Op) -> JsResult<()> {
        let v = self.peek(0)?;
        let r = match op {
            Op::Neg => Value::Number(-self.to_number(&v)?),
            Op::Plus | Op::ToNumber => Value::Number(self.to_number(&v)?),
            Op::BitNot => Value::Number(f64::from(!to_int32(self.to_number(&v)?))),
            Op::Not => Value::Bool(!v.truthy()),
            Op::Typeof => Value::str(self.type_of(&v)),
            _ => return internal("not a unary operator"),
        };
        self.replace(1, r)
    }

    fn op_binary(&mut self, op: BinaryOp) -> JsResult<()> {
        let b = self.peek(0)?;
        let a = self.peek(1)?;
        let r = self.binary(op, &a, &b)?;
        self.replace(2, r)
    }

    pub(crate) fn binary(&mut self, op: BinaryOp, a: &Value, b: &Value) -> JsResult<Value> {
        let num = |interp: &mut Self, f: fn(f64, f64) -> f64| -> JsResult<Value> {
            let x = interp.to_number(a)?;
            let y = interp.to_number(b)?;
            Ok(Value::Number(f(x, y)))
        };
        let int = |interp: &mut Self, f: fn(i32, i32) -> i32| -> JsResult<Value> {
            let x = to_int32(interp.to_number(a)?);
            let y = to_int32(interp.to_number(b)?);
            Ok(Value::Number(f64::from(f(x, y))))
        };
        match op {
            BinaryOp::Add => self.add(a, b),
            BinaryOp::Sub => num(self, |x, y| x - y),
            BinaryOp::Mul => num(self, |x, y| x * y),
            BinaryOp::Div => num(self, |x, y| x / y),
            BinaryOp::Mod => num(self, |x, y| x % y),
            BinaryOp::Shl => int(self, |x, y| x.wrapping_shl((y & 31) as u32)),
            BinaryOp::Shr => int(self, |x, y| x >> (y & 31)),
            BinaryOp::UShr => {
                let x = to_uint32(self.to_number(a)?);
                let y = to_uint32(self.to_number(b)?);
                Ok(Value::Number(f64::from(x >> (y & 31))))
            }
            BinaryOp::BitAnd => int(self, |x, y| x & y),
            BinaryOp::BitOr => int(self, |x, y| x | y),
            BinaryOp::BitXor => int(self, |x, y| x ^ y),
            BinaryOp::Lt => Ok(Value::Bool(self.less_than(a, b, true)? == Some(true))),
            BinaryOp::Gt => Ok(Value::Bool(self.less_than(b, a, false)? == Some(true))),
            BinaryOp::LtEq => Ok(Value::Bool(self.less_than(b, a, false)? == Some(false))),
            BinaryOp::GtEq => Ok(Value::Bool(self.less_than(a, b, true)? == Some(false))),
            BinaryOp::Eq => Ok(Value::Bool(self.loose_eq(a, b)?)),
            BinaryOp::NotEq => Ok(Value::Bool(!self.loose_eq(a, b)?)),
            BinaryOp::StrictEq => Ok(Value::Bool(strict_eq(a, b))),
            BinaryOp::StrictNotEq => Ok(Value::Bool(!strict_eq(a, b))),
            BinaryOp::Instanceof => self.instance_of(a, b).map(Value::Bool),
            BinaryOp::In => {
                let Value::Object(o) = b else { return Err(self.type_error("the right side of 'in' is not an object")) };
                let k = self.to_key(a)?;
                self.has_property(*o, &k).map(Value::Bool)
            }
        }
    }

    /// The addition operator (section 11.6.1).
    fn add(&mut self, a: &Value, b: &Value) -> JsResult<Value> {
        if let (Value::Number(x), Value::Number(y)) = (a, b) {
            return Ok(Value::Number(x + y));
        }
        let pa = self.to_primitive(a, Hint::Default)?;
        let pb = self.to_primitive(b, Hint::Default)?;
        if matches!(pa, Value::String(_)) || matches!(pb, Value::String(_)) {
            let sa = self.primitive_to_str(&pa);
            let sb = self.primitive_to_str(&pb);
            return self.concat(&sa, &sb).map(Value::String);
        }
        Ok(Value::Number(self.to_number(&pa)? + self.to_number(&pb)?))
    }

    /// The abstract relational comparison `x < y` (section 11.8.5): `None` when either side is
    /// NaN.
    fn less_than(&mut self, x: &Value, y: &Value, left_first: bool) -> JsResult<Option<bool>> {
        let (px, py) = if left_first {
            let px = self.to_primitive(x, Hint::Number)?;
            (px, self.to_primitive(y, Hint::Number)?)
        } else {
            let py = self.to_primitive(y, Hint::Number)?;
            (self.to_primitive(x, Hint::Number)?, py)
        };
        if let (Value::String(a), Value::String(b)) = (&px, &py) {
            return Ok(Some(*a.flat() < *b.flat()));
        }
        let nx = self.to_number(&px)?;
        let ny = self.to_number(&py)?;
        if nx.is_nan() || ny.is_nan() {
            return Ok(None);
        }
        Ok(Some(nx < ny))
    }

    /// The abstract equality comparison (section 11.9.3).
    fn loose_eq(&mut self, a: &Value, b: &Value) -> JsResult<bool> {
        let (mut x, mut y) = (a.clone(), b.clone());
        loop {
            match (&x, &y) {
                (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => return Ok(true),
                (Value::Number(_), Value::Number(_))
                | (Value::String(_), Value::String(_))
                | (Value::Bool(_), Value::Bool(_))
                | (Value::Object(_), Value::Object(_)) => return Ok(strict_eq(&x, &y)),
                (Value::Number(n), Value::String(s)) | (Value::String(s), Value::Number(n)) => return Ok(*n == string_to_number(&s.flat())),
                (Value::Bool(v), _) => x = Value::Number(f64::from(u8::from(*v))),
                (_, Value::Bool(v)) => y = Value::Number(f64::from(u8::from(*v))),
                (Value::Number(_) | Value::String(_), Value::Object(_)) => y = self.to_primitive(&y, Hint::Default)?,
                (Value::Object(_), Value::Number(_) | Value::String(_)) => x = self.to_primitive(&x, Hint::Default)?,
                _ => return Ok(false),
            }
        }
    }

    /// `instanceof` (sections 11.8.6 and 15.3.5.3).
    fn instance_of(&mut self, v: &Value, f: &Value) -> JsResult<bool> {
        if !self.is_callable(f) {
            return Err(self.type_error("the right side of 'instanceof' is not a function"));
        }
        let Some(fid) = f.as_object() else { return Ok(false) };
        let Value::Object(mut o) = v.clone() else { return Ok(false) };
        let Value::Object(proto) = self.get_from(fid, &key("prototype"), f)? else {
            return Err(self.type_error("the function's prototype is not an object"));
        };
        let mut steps = 0usize;
        while let Some(p) = self.object(o)?.proto {
            if p == proto {
                return Ok(true);
            }
            o = p;
            steps += 1;
            if steps > 1_000_000 {
                return internal("prototype chain too long");
            }
        }
        Ok(false)
    }

    // ---- host values ----

    pub(crate) fn to_js_value(&self, v: &Value) -> crate::JsValue {
        match v {
            Value::Undefined => crate::JsValue::Undefined,
            Value::Null => crate::JsValue::Null,
            Value::Bool(b) => crate::JsValue::Bool(*b),
            Value::Number(n) => crate::JsValue::Number(*n),
            Value::String(s) => crate::JsValue::String(s.to_rust_string()),
            Value::Object(o) => crate::JsValue::Object(self.heap.object(*o).map_or("Object", |o| o.class.name()).to_string()),
        }
    }

    /// A host value as a JavaScript value. A string is charged to the heap, and one that does not
    /// fit in it ends the run with the heap limit. Everything the caller still needs must be
    /// reachable from the roots.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn from_js_value(&mut self, v: crate::JsValue) -> JsResult<Value> {
        Ok(match v {
            crate::JsValue::Undefined => Value::Undefined,
            crate::JsValue::Null => Value::Null,
            crate::JsValue::Bool(b) => Value::Bool(b),
            crate::JsValue::Number(n) => Value::Number(n),
            crate::JsValue::String(s) => {
                let bytes = string_bytes(s.encode_utf16().count());
                self.reserve_heap(bytes)?;
                self.heap.bytes += bytes;
                Value::str(&s)
            }
            crate::JsValue::Object(_) => Value::Object(self.new_object()),
        })
    }

    /// A description of a thrown value that runs no JavaScript: `Name: message` for an error,
    /// the text of a primitive.
    pub(crate) fn describe(&mut self, v: &Value) -> String {
        let Value::Object(o) = v else { return self.primitive_to_str(v).to_rust_string() };
        let data = |interp: &Self, k: &str| match interp.find_property(*o, &key(k)) {
            Ok(Some(Prop::Data { value: Value::String(s), .. })) => Some(s.to_rust_string()),
            _ => None,
        };
        match (data(self, "name"), data(self, "message")) {
            (Some(n), Some(m)) if !m.is_empty() => format!("{n}: {m}"),
            (Some(n), _) => n,
            (None, Some(m)) => m,
            (None, None) => format!("[object {}]", self.heap.object(*o).map_or("Object", |o| o.class.name())),
        }
    }
}

/// A compile failure, before it becomes an exception or a run error.
pub(crate) enum CompileError {
    Syntax(crate::SyntaxError),
    Internal(String),
}

/// The strict equality comparison (section 11.9.6).
pub(crate) fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Object(x), Value::Object(y)) => x == y,
        _ => false,
    }
}
