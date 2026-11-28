//! Generator objects (ECMA-262 11th edition, <https://262.ecma-international.org/11.0/>,
//! sections 14.4 and 25.4), in a realm with [`crate::Options::es2015`].
//!
//! A call to a generator function runs its parameters' initialization and stops at
//! [`Op::GeneratorStart`]: its frame, with the operands above the frame's base and the `finally`
//! blocks it is inside, moves into a new generator object, which the call returns. `next`,
//! `return` and `throw` push the frame back on the interpreter's frame stack and go on in the same
//! loop, so a generator driven from JavaScript uses no Rust stack, and each resumption takes a unit
//! of fuel and counts against the frame limit. `yield` moves the frame back into the object. A
//! suspended generator's frame is reachable only through its object, so the collector keeps it
//! while the object is reachable and frees it with the object.
//!
//! [`Op::GeneratorStart`]: crate::bytecode::Op::GeneratorStart

use std::rc::Rc;

use crate::LimitKind;
use crate::builtins::iterator::iter_result;
use crate::bytecode::Code;
use crate::heap::{Class, EnvId, OBJECT_BYTES, ObjId, Object, key};
use crate::interp::{Abrupt, Frame, Interp, JsResult, Prepared, ThrowSite, internal};
use crate::value::Value;

/// How a generator is resumed: by `next`, `throw` or `return` (sections 25.4.1.3 and 25.4.1.4).
/// The code after a `yield` reads it as the number [`ResumeMode::number`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResumeMode {
    Next,
    Throw,
    Return,
}

impl ResumeMode {
    fn number(self) -> f64 {
        match self {
            ResumeMode::Next => 0.0,
            ResumeMode::Throw => 1.0,
            ResumeMode::Return => 2.0,
        }
    }

    fn of(v: &Value) -> JsResult<Self> {
        match v {
            Value::Number(n) if *n == 0.0 => Ok(ResumeMode::Next),
            Value::Number(n) if *n == 1.0 => Ok(ResumeMode::Throw),
            Value::Number(n) if *n == 2.0 => Ok(ResumeMode::Return),
            _ => internal("a resumption mode that is not one"),
        }
    }
}

/// The estimated bytes of a suspended generator's frame besides its operands.
const GENERATOR_BYTES: usize = 128;

/// The estimated bytes of each operand and `finally` entry a suspended frame holds.
const SLOT_BYTES: usize = 16;

/// A generator's frame while it is suspended: what [`Frame`] holds, with the operands above its
/// base and its `finally` entries, their stack slots counted from its base.
#[derive(Clone)]
pub(crate) struct SavedFrame {
    pub(crate) code: Rc<Code>,
    pc: usize,
    pub(crate) env: EnvId,
    pub(crate) var_env: EnvId,
    env_depth: u32,
    pub(crate) this: Value,
    /// The frame was running a class's heritage or computed keys when it yielded.
    strict_class: bool,
    pub(crate) callee: Option<ObjId>,
    pub(crate) completion: Value,
    pub(crate) stack: Vec<Value>,
    /// [`Interp::finally_sites`] entries.
    pub(crate) sites: Vec<(ThrowSite, usize)>,
    /// [`Interp::finally_marks`] entries.
    pub(crate) marks: Vec<(ObjId, u32, usize)>,
}

/// A generator's [[GeneratorState]] (section 25.4.3), with its frame while it is suspended.
#[derive(Clone)]
pub(crate) enum GenState {
    SuspendedStart(SavedFrame),
    SuspendedYield(SavedFrame),
    Executing,
    Completed,
}

impl GenState {
    /// The suspended frame, if the generator is suspended.
    pub(crate) fn saved(&self) -> Option<&SavedFrame> {
        match self {
            GenState::SuspendedStart(s) | GenState::SuspendedYield(s) => Some(s),
            GenState::Executing | GenState::Completed => None,
        }
    }
}

/// The bytes a suspended frame adds to its generator object, which a collection counts.
pub(crate) fn saved_bytes(s: &SavedFrame) -> usize {
    GENERATOR_BYTES.saturating_add(SLOT_BYTES.saturating_mul(s.stack.len().saturating_add(s.sites.len()).saturating_add(s.marks.len())))
}

/// A generator resumed by `next`, `return` or `throw`, ready to be pushed: the value it was
/// resumed with and how, unless it had not started.
pub(crate) struct Resumed {
    generator: ObjId,
    saved: SavedFrame,
    received: Option<(Value, ResumeMode)>,
}

impl Interp {
    fn generator_state_mut(&mut self, g: ObjId) -> Option<&mut GenState> {
        match self.heap.object_mut(g).map(|o| &mut o.class) {
            Some(Class::Generator(s)) => Some(s),
            _ => None,
        }
    }

    /// Marks the generator `g`, whose frame was popped without being saved, completed.
    pub(crate) fn end_generator(&mut self, g: Option<ObjId>) {
        if let Some(state) = g.and_then(|g| self.generator_state_mut(g))
            && let GenState::SuspendedStart(saved) | GenState::SuspendedYield(saved) = std::mem::replace(state, GenState::Completed)
        {
            self.release_saved(&saved);
        }
    }

    /// Takes a saved frame that left its generator object off the heap's total.
    fn release_saved(&mut self, saved: &SavedFrame) {
        self.heap.bytes = self.heap.bytes.saturating_sub(saved_bytes(saved));
    }

    /// Gives a frame's caller `v`: the Rust call that pushed a boundary frame, or the frame below.
    fn deliver(&mut self, boundary: bool, v: Value) -> JsResult<Option<Value>> {
        if boundary {
            return Ok(Some(v));
        }
        self.push(v)?;
        Ok(None)
    }

    /// Moves `frame`, just popped, with its operands and `finally` entries, into a [`SavedFrame`].
    fn save_frame(&mut self, frame: Frame) -> SavedFrame {
        let base = frame.base;
        let stack = self.stack.split_off(base.min(self.stack.len()));
        let i = self.finally_sites.partition_point(|&(_, slot)| slot < base);
        let sites = self.finally_sites.split_off(i).into_iter().map(|(site, slot)| (site, slot - base)).collect();
        let mut marks = Vec::new();
        self.finally_marks.retain(|&(id, mark, slot)| {
            let below = slot < base;
            if !below {
                marks.push((id, mark, slot - base));
            }
            below
        });
        SavedFrame {
            code: frame.code,
            pc: frame.pc,
            env: frame.env,
            var_env: frame.var_env,
            env_depth: frame.env_depth,
            this: frame.this,
            strict_class: frame.strict_class,
            callee: frame.callee,
            completion: frame.completion,
            stack,
            sites,
            marks,
        }
    }

    /// [`Op::GeneratorStart`](crate::bytecode::Op::GeneratorStart): OrdinaryCreateFromConstructor
    /// of the function's `prototype` (section 14.4.11), or %GeneratorPrototype% when that is not an
    /// object, holding the frame, which the call returns.
    pub(crate) fn op_generator_start(&mut self) -> JsResult<Option<Value>> {
        let Some(callee) = self.frame()?.callee else { return internal("a generator's body without its function") };
        // A realm made without `Options::es2015` that reads the syntax later has no generator
        // built-ins.
        let Some(fallback) = self.symbols.objects.map(|o| o.generator_proto) else {
            return Err(self.type_error("generators need the ECMAScript 2015 built-ins, and this realm was made without them"));
        };
        let proto = match self.get_from(callee, &key("prototype"), &Value::Object(callee))? {
            Value::Object(p) => p,
            _ => fallback,
        };
        self.rooted(&[Value::Object(proto)], |i| i.reserve_heap(OBJECT_BYTES + GENERATOR_BYTES))?;
        let Some(frame) = self.frames.pop() else { return internal("no frame") };
        let boundary = frame.boundary;
        let saved = self.save_frame(frame);
        let bytes = saved_bytes(&saved);
        let g = self.heap.alloc_object(Object::new(Class::Generator(Box::new(GenState::SuspendedStart(saved))), Some(proto)));
        self.heap.bytes = self.heap.bytes.saturating_add(bytes);
        self.deliver(boundary, Value::Object(g))
    }

    /// [`Op::Yield`](crate::bytecode::Op::Yield): GeneratorYield (section 25.4.3.7). The frame,
    /// already past the instruction, goes into the generator object, and its caller gets the
    /// iterator result.
    pub(crate) fn op_yield(&mut self, raw: bool) -> JsResult<Option<Value>> {
        let Some(g) = self.frame()?.generator else { return internal("yield outside a generator's body") };
        if !raw {
            let v = self.peek(0)?;
            let result = iter_result(self, v, false)?;
            self.replace(1, result)?;
        }
        // What `save_frame` keeps: the frame's operands without the result, popped below, and its
        // own `finally` entries, not those of the frames under it.
        let base = self.frame()?.base;
        let sites = self.finally_sites.len().saturating_sub(self.finally_sites.partition_point(|&(_, slot)| slot < base));
        let marks = self.finally_marks.iter().filter(|&&(_, _, slot)| slot >= base).count();
        let held = self.stack.len().saturating_sub(base.saturating_add(1)).saturating_add(sites).saturating_add(marks);
        let bytes = GENERATOR_BYTES.saturating_add(SLOT_BYTES.saturating_mul(held));
        self.reserve_heap(bytes)?;
        let result = self.pop()?;
        let Some(frame) = self.frames.pop() else { return internal("no frame") };
        let boundary = frame.boundary;
        let saved = self.save_frame(frame);
        self.heap.bytes = self.heap.bytes.saturating_add(saved_bytes(&saved));
        let Some(state) = self.generator_state_mut(g) else { return internal("a generator's frame without its object") };
        *state = GenState::SuspendedYield(saved);
        self.deliver(boundary, result)
    }

    /// GeneratorResume and GeneratorResumeAbrupt (sections 25.4.3.3 and 25.4.3.4) for `next`,
    /// `return` and `throw` called on `this` with `sent`: the result when the generator does not
    /// run, or its frame to push.
    pub(crate) fn prepare_resume(&mut self, this: Value, mode: ResumeMode, sent: Value) -> JsResult<Prepared> {
        // Whether it has started, or `None` when it is running or completed.
        let (g, started) = match this {
            Value::Object(g) => match self.heap.object(g).map(|o| &o.class) {
                Some(Class::Generator(s)) => match **s {
                    GenState::SuspendedStart(_) => (g, Some(false)),
                    GenState::SuspendedYield(_) => (g, Some(true)),
                    GenState::Executing => return Err(self.type_error("the generator is already running")),
                    GenState::Completed => (g, None),
                },
                _ => return Err(self.type_error("the value is not a generator object")),
            },
            _ => return Err(self.type_error("the value is not a generator object")),
        };
        let Some(started) = started else {
            return match mode {
                ResumeMode::Next => Ok(Prepared::Done(iter_result(self, Value::Undefined, true)?)),
                ResumeMode::Return => Ok(Prepared::Done(iter_result(self, sent, true)?)),
                ResumeMode::Throw => Err(Abrupt::Throw(sent)),
            };
        };
        if !started && mode != ResumeMode::Next {
            self.end_generator(Some(g));
            return match mode {
                ResumeMode::Throw => Err(Abrupt::Throw(sent)),
                _ => Ok(Prepared::Done(iter_result(self, sent, true)?)),
            };
        }
        if self.frames.len() >= self.limits.frames {
            return Err(Abrupt::Limit(LimitKind::Frames));
        }
        let Some(state) = self.generator_state_mut(g) else { return internal("a generator object without its state") };
        let (GenState::SuspendedStart(saved) | GenState::SuspendedYield(saved)) = std::mem::replace(state, GenState::Executing) else {
            return internal("a suspended generator without its frame");
        };
        self.release_saved(&saved);
        let received = started.then_some((sent, mode));
        Ok(Prepared::Resume(Box::new(Resumed { generator: g, saved, received })))
    }

    /// Pushes a resumed generator's frame, with the value it was resumed with and how on top of
    /// its operands when it had yielded.
    pub(crate) fn enter_resumed(&mut self, r: Resumed, boundary: bool) -> JsResult<()> {
        let Resumed { generator, saved, received } = r;
        if self.frames.len() >= self.limits.frames {
            let bytes = saved_bytes(&saved);
            if let Some(state) = self.generator_state_mut(generator) {
                *state = if received.is_some() { GenState::SuspendedYield(saved) } else { GenState::SuspendedStart(saved) };
                self.heap.bytes = self.heap.bytes.saturating_add(bytes);
            }
            return Err(Abrupt::Limit(LimitKind::Frames));
        }
        let base = self.stack.len();
        self.drop_left_finally_sites(base);
        let SavedFrame { code, pc, env, var_env, env_depth, this, strict_class, callee, completion, stack, sites, marks } = saved;
        self.stack.extend(stack);
        if let Some((value, mode)) = received {
            self.stack.push(value);
            self.stack.push(Value::Number(mode.number()));
        }
        self.finally_sites.extend(sites.into_iter().map(|(site, slot)| (site, slot + base)));
        self.finally_marks.extend(marks.into_iter().map(|(id, mark, slot)| (id, mark, slot + base)));
        self.frames.push(Frame {
            code,
            pc,
            base,
            env,
            var_env,
            env_depth,
            this,
            // A generator is neither an arrow function nor a constructor, so it reads its own
            // `this` and has no `new.target`.
            this_in_env: false,
            strict_class,
            callee,
            construct: None,
            new_target: None,
            this_env: None,
            completion,
            boundary,
            generator: Some(generator),
        });
        Ok(())
    }

    /// [`Op::Resume`](crate::bytecode::Op::Resume): `received mode` → `received`.
    pub(crate) fn op_resume(&mut self, target: u32) -> JsResult<()> {
        let mode = ResumeMode::of(&self.peek(0)?)?;
        let received = self.peek(1)?;
        self.pop()?;
        self.pop()?;
        match mode {
            ResumeMode::Next => self.push(received),
            ResumeMode::Throw => {
                self.thrown_mark = None;
                Err(Abrupt::Throw(received))
            }
            ResumeMode::Return => {
                self.push(received)?;
                self.jump_to(target)
            }
        }
    }

    /// GetMethod (section 7.3.10): the property `name` of `v`, `None` when it is `undefined` or
    /// `null`.
    fn get_method(&mut self, v: &Value, name: &str) -> JsResult<Option<Value>> {
        let f = self.get_value(v, &key(name))?;
        if f.is_nullish() {
            return Ok(None);
        }
        if !self.is_callable(&f) {
            return Err(self.type_error(&format!("the iterator's {name} is not a function")));
        }
        Ok(Some(f))
    }

    /// [`Op::DelegateIterator`](crate::bytecode::Op::DelegateIterator): `value` → `iterator next`.
    pub(crate) fn op_delegate_iterator(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let Some(sym) = self.well_known(|w| &w.iterator) else { return internal("no Symbol.iterator in this realm") };
        let method = self.get_value_sym(&v, &sym)?;
        if !self.is_callable(&method) {
            return Err(self.type_error("the value is not iterable"));
        }
        let iterator = self.call(&method, &v, &[])?;
        if !matches!(iterator, Value::Object(_)) {
            return Err(self.type_error("the iterator is not an object"));
        }
        self.replace(1, iterator.clone())?;
        let next = self.get_value(&iterator, &key("next"))?;
        self.push(next)
    }

    /// [`Op::DelegateCall`](crate::bytecode::Op::DelegateCall): the method of `yield*`'s iterator
    /// that the generator's resumption calls (section 14.4.14, step 7).
    pub(crate) fn op_delegate_call(&mut self, target: u32) -> JsResult<()> {
        let mode = ResumeMode::of(&self.peek(0)?)?;
        let iterator = self.peek(3)?;
        let method = match mode {
            ResumeMode::Next => Some(self.peek(2)?),
            ResumeMode::Throw => self.get_method(&iterator, "throw")?,
            ResumeMode::Return => self.get_method(&iterator, "return")?,
        };
        let received = self.peek(1)?;
        match (method, mode) {
            (Some(m), _) => {
                self.replace(2, Value::Number(mode.number()))?;
                self.push(m)?;
                self.push(iterator)?;
                self.push(received)
            }
            (None, ResumeMode::Return) => {
                self.replace(2, received)?;
                self.jump_to(target)
            }
            (None, _) => {
                // The iterator has no `throw`: it is closed, and the protocol violation is a
                // `TypeError` (step 7.b.iii).
                if let Some(close) = self.get_method(&iterator, "return")? {
                    let result = self.call(&close, &iterator, &[])?;
                    if !matches!(result, Value::Object(_)) {
                        return Err(self.type_error("the iterator result is not an object"));
                    }
                }
                Err(self.type_error("the iterator has no throw method"))
            }
        }
    }

    /// [`Op::DelegateResult`](crate::bytecode::Op::DelegateResult): `mode result` → `result`, or
    /// when it is done, a jump with `mode value`.
    pub(crate) fn op_delegate_result(&mut self, target: u32) -> JsResult<()> {
        let result = self.peek(0)?;
        if !matches!(result, Value::Object(_)) {
            return Err(self.type_error("the iterator result is not an object"));
        }
        if self.get_value(&result, &key("done"))?.truthy() {
            let value = self.get_value(&result, &key("value"))?;
            self.replace(1, value)?;
            return self.jump_to(target);
        }
        self.replace(2, result)
    }

    /// [`Op::DelegateDone`](crate::bytecode::Op::DelegateDone): `mode value` → `value`, a jump for
    /// `return`.
    pub(crate) fn op_delegate_done(&mut self, target: u32) -> JsResult<()> {
        let mode = ResumeMode::of(&self.peek(1)?)?;
        let value = self.peek(0)?;
        self.replace(2, value)?;
        if mode == ResumeMode::Return {
            return self.jump_to(target);
        }
        Ok(())
    }
}
