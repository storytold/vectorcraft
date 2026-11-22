//! An async function's frame while it waits at `await` (ECMA-262 2017 sections 25.5 and 6.2.3.1,
//! <https://262.ecma-international.org/8.0/#await>): [`Interp::suspend_frame`] moves the running
//! frame, its operands and its pending `finally` state into an [`Activation`], which lives on the
//! heap inside the functions that resume it, and [`Interp::restore_frame`] puts it back on top of
//! the frames to go on. An activation that nothing can resume any more is collected like any
//! other object.

use std::rc::Rc;

use super::{Frame, Interp, JsResult, ThrowSite, internal};
use crate::bytecode::Code;
use crate::heap::{EnvId, ObjId};
use crate::value::Value;

/// The estimated bytes of an activation, beyond the object and its operands.
pub(crate) const ACTIVATION_BYTES: usize = 128;
/// The estimated bytes of each operand, `finally` mark or `finally` site an activation keeps.
pub(crate) const SLOT_BYTES: usize = 16;

/// A suspended frame. Stack slots in `marks` and `sites` count from the frame's first operand.
#[derive(Clone)]
pub(crate) struct Activation {
    code: Rc<Code>,
    pc: usize,
    env: EnvId,
    var_env: EnvId,
    env_depth: u32,
    this: Value,
    callee: Option<ObjId>,
    completion: Value,
    /// The promise the async function's call returned.
    pub(crate) promise: ObjId,
    stack: Vec<Value>,
    marks: Vec<(ObjId, u32, usize)>,
    sites: Vec<(ThrowSite, usize)>,
}

impl Activation {
    /// The values the activation holds, for the collector.
    pub(crate) fn values(&self) -> impl Iterator<Item = &Value> {
        self.stack.iter().chain([&self.this, &self.completion])
    }

    /// The objects and environments the activation holds, for the collector.
    pub(crate) fn ids(&self) -> impl Iterator<Item = u32> {
        self.callee.iter().chain([&self.promise]).chain(self.marks.iter().map(|(o, _, _)| o)).map(|o| o.0).chain([self.env.0, self.var_env.0])
    }

    /// The code the activation runs and the code of its `finally` sites, for the collector.
    pub(crate) fn codes(&self) -> impl Iterator<Item = &Code> {
        std::iter::once(&*self.code).chain(self.sites.iter().map(|((code, _), _)| &**code))
    }

    /// The estimated bytes of the activation, beyond the object.
    pub(crate) fn bytes(&self) -> usize {
        ACTIVATION_BYTES.saturating_add(SLOT_BYTES.saturating_mul(self.stack.len() + self.marks.len() + self.sites.len()))
    }
}

impl Interp {
    /// The promise of the async function the top frame runs, when it runs one.
    pub(crate) fn async_promise(&self) -> Option<ObjId> {
        self.frames.last().and_then(|f| f.async_promise)
    }

    /// Makes the promise of the async function call whose frame was just pushed.
    pub(crate) fn start_async_call(&mut self) -> JsResult<()> {
        let p = self.new_async_promise()?;
        match self.frames.last_mut() {
            Some(f) => {
                f.async_promise = Some(p);
                Ok(())
            }
            None => internal("an async call without a frame"),
        }
    }

    /// The bytes [`Self::suspend_frame`] will move off the operand stack, for the heap check made
    /// before it.
    pub(crate) fn suspend_bytes(&self) -> usize {
        let Some(f) = self.frames.last() else { return ACTIVATION_BYTES };
        let n = self.stack.len().saturating_sub(f.base)
            + self.finally_marks.iter().filter(|&&(_, _, s)| s >= f.base).count()
            + self.finally_sites.iter().filter(|&&(_, s)| s >= f.base).count();
        ACTIVATION_BYTES.saturating_add(SLOT_BYTES.saturating_mul(n))
    }

    /// Takes the top frame, which runs an async function, off the frames with its operands and
    /// its `finally` state. Gives the activation and whether the frame was a boundary frame.
    pub(crate) fn suspend_frame(&mut self) -> JsResult<(Activation, bool)> {
        let Some(frame) = self.frames.pop() else { return internal("await without a frame") };
        let Some(promise) = frame.async_promise else { return internal("await outside an async function") };
        let base = frame.base;
        if self.stack.len() < base {
            return internal("frame operands below the stack");
        }
        let stack = self.stack.split_off(base);
        let mut marks = Vec::new();
        self.finally_marks.retain(|&(o, mark, slot)| {
            if slot < base {
                return true;
            }
            marks.push((o, mark, slot - base));
            false
        });
        let at = self.finally_sites.iter().position(|&(_, slot)| slot >= base).unwrap_or(self.finally_sites.len());
        let sites = self.finally_sites.split_off(at).into_iter().map(|(site, slot)| (site, slot - base)).collect();
        let activation = Activation {
            code: frame.code,
            pc: frame.pc,
            env: frame.env,
            var_env: frame.var_env,
            env_depth: frame.env_depth,
            this: frame.this,
            callee: frame.callee,
            completion: frame.completion,
            promise,
            stack,
            marks,
            sites,
        };
        Ok((activation, frame.boundary))
    }

    /// Puts a suspended frame back on top of the frames as a boundary frame, with its operands at
    /// the top of the operand stack.
    pub(crate) fn restore_frame(&mut self, a: Activation) -> JsResult<()> {
        if self.frames.len() >= self.limits.frames {
            return Err(crate::interp::Abrupt::Limit(crate::LimitKind::Frames));
        }
        let base = self.stack.len();
        for v in a.stack {
            self.push(v)?;
        }
        self.finally_marks.extend(a.marks.into_iter().map(|(o, mark, slot)| (o, mark, slot + base)));
        self.finally_sites.extend(a.sites.into_iter().map(|(site, slot)| (site, slot + base)));
        self.frames.push(Frame {
            code: a.code,
            pc: a.pc,
            base,
            env: a.env,
            var_env: a.var_env,
            env_depth: a.env_depth,
            this: a.this,
            callee: a.callee,
            construct: None,
            completion: a.completion,
            boundary: true,
            async_promise: Some(a.promise),
            annex_off: Vec::new(),
        });
        Ok(())
    }
}
