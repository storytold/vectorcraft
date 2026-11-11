//! The embedding API: a [`Realm`] holds a global object and its built-ins, runs scripts in them,
//! and enforces [`Limits`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::bytecode::CodeKind;
use crate::heap::{Func, HIDDEN, Object, Prop, key};
use crate::interp::{Abrupt, CompileError, Interp};
use crate::value::Value;
use crate::{Options, SyntaxError};

/// The limits a run is held to. Reaching one ends the run with [`RunErrorKind::Limit`]; scripts
/// cannot catch it, and their `finally` blocks do not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Instructions a run may execute.
    pub fuel: u64,
    /// Estimated bytes the heap (objects, environments and the strings they hold) may reach
    /// after a collection. A string longer than a quarter of this many code units cannot be
    /// made.
    pub heap_bytes: usize,
    /// Call frames that may be active at once.
    pub frames: usize,
    /// How deeply the interpreter may run again inside an operation (getters and setters,
    /// conversions that call `valueOf` or `toString`, indirect `eval`).
    pub reentry: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for Limits {
    fn default() -> Self {
        Self { fuel: 1_000_000_000, heap_bytes: 512 << 20, frames: 10_000, reentry: 32 }
    }
}

/// A wasm build has less memory and a smaller stack.
#[cfg(target_arch = "wasm32")]
impl Default for Limits {
    fn default() -> Self {
        Self { fuel: 250_000_000, heap_bytes: 128 << 20, frames: 2_000, reentry: 16 }
    }
}

/// Which limit a run reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    Fuel,
    Heap,
    Frames,
    Reentry,
    /// The host set the interrupt flag.
    Interrupted,
    /// A getter, a callback of a built-in or another nested run spent [`MAX_UNBROKEN_FUEL`]
    /// without returning to the point where a resumable run can pause.
    Unbroken,
    /// A host function ended the run ([`crate::HostError::Abort`]).
    Host,
}

/// The fuel [`Realm::advance`] lets a slice spend, past the slice it was asked for, in a nested run
/// it cannot pause.
pub const MAX_UNBROKEN_FUEL: u64 = 200_000_000;

/// How far a resumable run got.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// The slice ended; call [`Realm::advance`] again.
    Paused,
    /// The script ended, with its completion value.
    Done(JsValue),
}

/// A flag the host sets, from any thread, to stop the running script at its next instruction.
/// It stays set until [`Interrupt::clear`].
#[derive(Clone, Debug, Default)]
pub struct Interrupt(Arc<AtomicBool>);

impl Interrupt {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// A value passed between a script and the host. An object is given by its class name.
#[derive(Clone, Debug, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunErrorKind {
    /// The source does not parse.
    Syntax,
    /// The script threw an exception it did not catch.
    Thrown,
    Limit(LimitKind),
    /// The interpreter reached a state it never makes.
    Internal,
}

/// Why a run did not complete. `line` and `column` count from 1 (the column in UTF-16 code units)
/// and give where the error was raised; they are 0 when that is not known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunError {
    pub kind: RunErrorKind,
    pub message: String,
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (line {}, column {})", self.message, self.line, self.column)
    }
}

impl std::error::Error for RunError {}

impl From<SyntaxError> for RunError {
    fn from(e: SyntaxError) -> Self {
        RunError { kind: RunErrorKind::Syntax, message: e.to_string(), line: e.line, column: e.column }
    }
}

/// A global environment with the built-ins, in which scripts run one after another.
pub struct Realm {
    interp: Interp,
    unbroken_fuel: u64,
}

impl Default for Realm {
    fn default() -> Self {
        Self::new()
    }
}

impl Realm {
    pub fn new() -> Self {
        Self::with_options(Options::default(), Limits::default())
    }

    /// A realm whose scripts (and the source given to `eval` and `Function`) are read with
    /// `options`; [`Options::max_source_units`] caps the source parsed in one run, all of it
    /// counted together.
    pub fn with_options(options: Options, limits: Limits) -> Self {
        Self { interp: Interp::new(options, limits), unbroken_fuel: MAX_UNBROKEN_FUEL }
    }

    /// Sets the fuel [`Realm::advance`] lets a nested run spend past its slice (by default
    /// [`MAX_UNBROKEN_FUEL`]).
    pub fn set_unbroken_fuel(&mut self, fuel: u64) {
        self.unbroken_fuel = fuel;
    }

    pub fn limits(&self) -> &Limits {
        &self.interp.limits
    }

    /// Sets the limits for the next run. A heap that is already larger than the new
    /// [`Limits::heap_bytes`] ends the next run with the heap limit unless a collection frees
    /// enough of it.
    pub fn set_limits(&mut self, limits: Limits) {
        self.interp.limits = limits;
        self.interp.reset_gc_threshold();
    }

    /// The realm's interrupt flag. Setting it stops a running script, or the next one to start.
    pub fn interrupt(&self) -> Interrupt {
        Interrupt(self.interp.interrupt.clone())
    }

    /// Defines a global function that calls `f` with its arguments. An `Err` from `f` is thrown
    /// to the script as an `Error` with that message.
    pub fn define_function(&mut self, name: &str, f: impl Fn(&[JsValue]) -> Result<JsValue, String> + 'static) {
        let interp = &mut self.interp;
        let proto = interp.intrinsics.function_proto;
        let func = Func::Host { f: std::rc::Rc::new(f), name: name.into() };
        let o = interp.heap.alloc_object(Object::new(crate::heap::Class::Function(func), Some(proto)));
        let global = interp.global;
        let _ = interp.define(global, key(name), Prop::Data { value: Value::Object(o), flags: HIDDEN });
    }

    /// Runs `source` as a script in the global environment and returns its completion value (the
    /// value of the last expression statement that ran). A realm whose built-ins could not be
    /// installed runs nothing and gives an internal error.
    pub fn run(&mut self, source: &str) -> Result<JsValue, RunError> {
        let code = self.compile(source)?;
        let result = self.interp.run_global(code, false);
        self.outcome(result)
    }

    /// Compiles `source` and readies the realm for a run.
    fn compile(&mut self, source: &str) -> Result<std::rc::Rc<crate::bytecode::Code>, RunError> {
        let interp = &mut self.interp;
        if let Some(message) = &interp.install_error {
            return Err(RunError { kind: RunErrorKind::Internal, message: message.clone(), line: 0, column: 0 });
        }
        interp.fuel = interp.limits.fuel;
        interp.source_units = 0;
        interp.throw_site = None;
        interp.last_thrown = Value::Undefined;
        interp.host_abort = None;
        interp.waiting = false;
        let strict = interp.options.strict;
        match interp.encode_source(source).and_then(|units| interp.compile_source(&units, strict, CodeKind::Global)) {
            Ok(code) => Ok(code),
            Err(CompileError::Syntax(e)) => Err(e.into()),
            Err(CompileError::Internal(m)) => Err(RunError { kind: RunErrorKind::Internal, message: m, line: 0, column: 0 }),
        }
    }

    /// The end of a run: its completion value or error, with the interpreter's stacks emptied.
    fn outcome(&mut self, result: Result<Value, Abrupt>) -> Result<JsValue, RunError> {
        let interp = &mut self.interp;
        let (line, column) = interp.throw_site.take().map_or((0, 0), |(code, pc)| code.line_column(pc));
        interp.stack.clear();
        interp.frames.clear();
        interp.temp_roots.clear();
        interp.join_stack.clear();
        interp.waiting = false;
        match result {
            Ok(v) => Ok(interp.to_js_value(&v)),
            Err(Abrupt::Throw(v)) => {
                let message = interp.describe(&v);
                interp.last_thrown = v;
                Err(RunError { kind: RunErrorKind::Thrown, message, line, column })
            }
            Err(Abrupt::Limit(LimitKind::Host)) => {
                let message = interp.host_abort.take().unwrap_or_else(|| format!("limit reached: {:?}", LimitKind::Host));
                Err(RunError { kind: RunErrorKind::Limit(LimitKind::Host), message, line, column })
            }
            Err(Abrupt::Limit(kind)) => Err(RunError { kind: RunErrorKind::Limit(kind), message: format!("limit reached: {kind:?}"), line, column }),
            Err(Abrupt::Internal(message)) => Err(RunError { kind: RunErrorKind::Internal, message, line, column }),
        }
    }

    /// Begins a run that [`Realm::advance`] carries on: compiles `source` and enters it.
    pub fn start(&mut self, source: &str) -> Result<(), RunError> {
        let code = self.compile(source)?;
        match self.interp.enter_global(code, false) {
            Ok(()) => Ok(()),
            Err(e) => self.outcome(Err(e)).map(drop),
        }
    }

    /// Runs the script begun with [`Realm::start`] for about `slice` units of fuel (at least one
    /// instruction), and pauses where it can resume: between two instructions of the script's own
    /// code. A getter, a built-in's callback or another nested run has native frames under it, so
    /// it runs on to its end, for at most [`Realm::set_unbroken_fuel`] more. Once this has returned
    /// [`Progress::Done`] or an error, the run is over. While a host function's call waits for its
    /// result ([`Realm::waits`]), the run does not advance: this gives an internal error.
    pub fn advance(&mut self, slice: u64) -> Result<Progress, RunError> {
        let interp = &mut self.interp;
        if interp.frames.is_empty() {
            return Err(RunError { kind: RunErrorKind::Internal, message: "no run to advance".into(), line: 0, column: 0 });
        }
        if interp.waiting {
            return Err(RunError { kind: RunErrorKind::Internal, message: "the run waits for a host function's result".into(), line: 0, column: 0 });
        }
        let total = interp.fuel;
        let cap = slice.saturating_add(self.unbroken_fuel).min(total);
        interp.fuel = cap;
        interp.in_slice = true;
        let r = interp.execute_slice(cap - slice.min(cap));
        interp.in_slice = false;
        let used = cap - interp.fuel;
        interp.fuel = total - used;
        match r {
            Ok(None) => Ok(Progress::Paused),
            Ok(Some(v)) => self.outcome(Ok(v)).map(Progress::Done),
            // The run's own fuel is not used up, so it was the allowance of this slice.
            Err(Abrupt::Limit(LimitKind::Fuel)) if total > cap => self.outcome(Err(Abrupt::Limit(LimitKind::Unbroken))).map(Progress::Done),
            Err(e) => self.outcome(Err(e)).map(Progress::Done),
        }
    }

    /// Is a run begun with [`Realm::start`] still going?
    pub fn is_running(&self) -> bool {
        !self.interp.frames.is_empty()
    }

    /// The flag a host function sets to end the current slice at the next point it can pause.
    pub fn pause_flag(&self) -> std::rc::Rc<std::cell::Cell<bool>> {
        self.interp.pause.clone()
    }

    /// The flag that is set while a host function runs where it may return
    /// [`crate::HostError::Wait`]: called by the script's own code in a run [`Realm::advance`]
    /// carries on, and not from a getter, a conversion or a built-in's callback.
    pub fn wait_flag(&self) -> std::rc::Rc<std::cell::Cell<bool>> {
        self.interp.can_wait.clone()
    }

    /// Does a host function's call wait for the result [`Realm::answer`] gives it?
    pub fn waits(&self) -> bool {
        self.interp.waiting
    }

    /// Gives the call that waits ([`crate::HostError::Wait`]) its result `d`; the next
    /// [`Realm::advance`] goes on from there. A result past the heap limit ends the run with that
    /// limit, and with no call waiting this gives an internal error.
    pub fn answer(&mut self, d: crate::Data) -> Result<(), RunError> {
        let interp = &mut self.interp;
        if !interp.waiting {
            return Err(RunError { kind: RunErrorKind::Internal, message: "no host function waits for a result".into(), line: 0, column: 0 });
        }
        interp.waiting = false;
        // The value on top of the stack stood in for the result.
        let r = interp.write_data(d, 0, false).and_then(|v| {
            interp.stack.pop();
            interp.push(v)
        });
        match r {
            Ok(()) => Ok(()),
            Err(e) => self.outcome(Err(e)).map(drop),
        }
    }

    /// Has this realm stop when `interrupt` is set, in place of its own flag.
    pub fn share_interrupt(&mut self, interrupt: &Interrupt) {
        self.interp.interrupt = interrupt.0.clone();
    }

    /// Fuel the run has left.
    pub fn fuel_left(&self) -> u64 {
        self.interp.fuel
    }

    /// Sets how much a value passed to or from a host function may carry.
    pub fn set_data_limits(&mut self, limits: crate::DataLimits) {
        self.interp.data_limits = limits;
    }

    /// Adds the frozen object `name`, with `items`, to the global environment. Scripts cannot add
    /// to it, change it or replace it. A heap too small for it ends with the error the realm's
    /// next run would give.
    pub fn define_host_object(&mut self, name: &str, items: Vec<(String, crate::HostItem)>) -> Result<(), RunError> {
        match self.interp.install_host_object(name, items) {
            Ok(()) => Ok(()),
            Err(e) => self.outcome(Err(e)).map(drop),
        }
    }

    /// Defines the global function `name`, whose arguments arrive converted as `args` says
    /// ([`crate::HostItem::Converted`]). Unlike a host object, a script may replace or delete it, as
    /// it may a built-in function.
    pub fn define_host_function(&mut self, name: &str, args: Vec<crate::Arg>, f: crate::HostFunction) -> Result<(), RunError> {
        match self.interp.install_host_function(name, args, f) {
            Ok(()) => Ok(()),
            Err(e) => self.outcome(Err(e)).map(drop),
        }
    }

    /// Defines the global variable `name` holding the exception the last run did not catch
    /// (`undefined` if there was none).
    pub fn bind_last_thrown(&mut self, name: &str) {
        let interp = &mut self.interp;
        let v = interp.last_thrown.clone();
        let global = interp.global;
        let _ = interp.define(global, key(name), Prop::Data { value: v, flags: HIDDEN });
    }

    /// Sets the clock `Date.now()` and `new Date()` read: the current time in milliseconds since
    /// 1970-01-01 UTC. By default it is the system clock; a wasm build has none, and its default
    /// gives NaN.
    pub fn set_clock(&mut self, clock: impl Fn() -> f64 + 'static) {
        self.interp.date_host.clock = std::rc::Rc::new(clock);
    }

    /// Sets the time zone `Date` uses for local time: `offset` gives local time's offset from UTC
    /// in milliseconds at a time value (milliseconds since 1970-01-01 UTC), so it can follow
    /// daylight saving time. An offset that is not finite or is a day or more counts as 0. By
    /// default local time is UTC.
    pub fn set_time_zone(&mut self, offset: impl Fn(f64) -> f64 + 'static) {
        self.interp.date_host.offset = std::rc::Rc::new(offset);
    }

    /// Collects garbage now and returns the heap's estimated size in bytes afterwards.
    pub fn collect_garbage(&mut self) -> usize {
        self.interp.collect_garbage();
        self.interp.heap.bytes
    }

    #[cfg(test)]
    pub(crate) fn interp(&mut self) -> &mut Interp {
        &mut self.interp
    }
}
