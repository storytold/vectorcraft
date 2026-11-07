//! Bytecode: the instructions a function, a script or eval code compiles to, and the verifier
//! that checks them before they run.
//!
//! The interpreter is a stack machine. Each instruction takes its operands from the top of the
//! frame's operand stack and pushes its result there; jumps name an instruction index. The
//! verifier walks every path through the code and checks that each jump target, exception handler
//! and `finally` dispatch target is an instruction, that each constant, name and function index is
//! in its table, that the stack never goes below the frame's base, that every path reaching an
//! instruction arrives with the same stack depth, and that no path runs past the last
//! instruction. Code that fails is reported as an internal error and never runs.

use std::rc::Rc;

use crate::LineTable;
use crate::ast::BinaryOp;
use crate::heap::Key;
use crate::value::Str;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Op {
    Undefined,
    Null,
    True,
    False,
    /// Pushes `numbers[i]`.
    Number(u32),
    /// Pushes `strings[i]`.
    String(u32),
    This,
    Pop,
    /// Removes the `n` values under the top one.
    PopUnder(u32),
    Dup,
    /// `a b` → `a b a b`.
    Dup2,
    Swap,
    /// `a b c` → `b c a`.
    Rot3,
    /// Pushes the value of the variable `names[i]`.
    GetVar(u32),
    /// Pushes the value of the variable and the `this` a call to it gets.
    GetVarThis(u32),
    /// `value` → `value`, assigned to the variable.
    SetVar(u32),
    /// Pushes the base of the reference to the variable `names[i]`, resolved before the
    /// right-hand side of an assignment runs.
    RefVar(u32),
    /// `base` → `value`: the variable's value read through the resolved base.
    GetRef(u32),
    /// `base value` → `value`, assigned to the variable through the resolved base.
    PutRef(u32),
    TypeofVar(u32),
    DeleteVar(u32),
    IncVar {
        name: u32,
        prefix: bool,
        dec: bool,
    },
    /// `object` → `object.name`.
    GetMember(u32),
    /// `object` → `object.name object`.
    GetMemberThis(u32),
    /// `object value` → `value`.
    SetMember(u32),
    DeleteMember(u32),
    IncMember {
        name: u32,
        prefix: bool,
        dec: bool,
    },
    /// `object key` → `object name`: checks that the object can have properties and turns the key
    /// into a property name, before the right-hand side of an assignment runs.
    ToKey,
    /// `object` → `object`: throws a `TypeError` when it is `undefined` or `null`, before the
    /// right-hand side of an assignment to one of its properties runs.
    CheckObject,
    /// `object key` → `object[key]`.
    GetIndex,
    /// `object key` → `object[key] object`.
    GetIndexThis,
    /// `object key value` → `value`.
    SetIndex,
    DeleteIndex,
    IncIndex {
        prefix: bool,
        dec: bool,
    },
    /// `function this arg…` → result.
    Call(u32),
    /// Like `Call`, but a direct eval when the function is the global `eval`.
    CallEval(u32),
    /// `function arg…` → the new object.
    New(u32),
    Binary(BinaryOp),
    Neg,
    Plus,
    BitNot,
    Not,
    Typeof,
    ToNumber,
    Jump(u32),
    /// Pops the top value and jumps when it is falsy.
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    /// Jumps, keeping the top value, when it is falsy; otherwise pops it.
    JumpIfFalseKeep(u32),
    JumpIfTrueKeep(u32),
    Return,
    /// Returns the value of the last expression statement (script and eval code).
    ReturnCompletion,
    Throw,
    /// Throws a `ReferenceError` with the message `strings[i]` (an assignment to something that
    /// cannot be assigned to).
    ThrowReferenceError(u32),
    /// Ends a `finally` block: `value kind` on the stack say how it was entered. Kind 0 (normal)
    /// pops both and goes on, kind 1 throws the value, and kind `2 + k` jumps to entry `k` of
    /// `finally_tables[i]` with the value left on the stack.
    EndFinally(u32),
    NewObject,
    NewArray,
    /// `array value` → `array`, the value appended.
    ArrayPush,
    /// `array` → `array`, one longer.
    ArrayHole,
    /// `object value` → `object`.
    InitProp(u32),
    /// `object function` → `object`.
    InitGetter(u32),
    InitSetter(u32),
    /// Pushes a new function made from `functions[i]` and the current environment.
    Closure(u32),
    /// Pushes a new regular expression object for `regexps[i]`.
    RegExp(u32),
    /// `object` → nothing: enters a `with` statement's environment.
    PushWith,
    /// `exception` → nothing: enters a catch clause's environment, binding `names[i]`.
    PushCatch(u32),
    PopEnv,
    /// `object` → `iterator`.
    ForInStart,
    /// `iterator` → `iterator name`, or jumps with `iterator` when the names are used up.
    ForInNext(u32),
    /// Pops the value of an expression statement into the completion value.
    SetCompletion,
}

/// What code is: a function body, a script, or eval code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodeKind {
    Function,
    Global,
    Eval,
}

/// A range of instructions an exception thrown in jumps out of: to `target`, with the operand
/// stack cut back to `depth` values and the environments to `env_depth`, and the exception pushed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Handler {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) target: u32,
    pub(crate) depth: u32,
    pub(crate) env_depth: u32,
}

/// The source a piece of code came from, for error positions and `Function.prototype.toString`.
#[derive(Debug)]
pub(crate) struct Source {
    pub(crate) lines: LineTable,
    pub(crate) text: Rc<[u16]>,
}

#[derive(Debug)]
pub(crate) struct Code {
    pub(crate) ops: Vec<Op>,
    /// The source offset each instruction comes from.
    pub(crate) positions: Vec<u32>,
    pub(crate) numbers: Vec<f64>,
    pub(crate) strings: Vec<Str>,
    pub(crate) names: Vec<Key>,
    pub(crate) functions: Vec<Rc<Code>>,
    pub(crate) regexps: Vec<(Str, Str)>,
    pub(crate) handlers: Vec<Handler>,
    pub(crate) finally_tables: Vec<Vec<u32>>,
    /// The most values the code keeps on the operand stack, set by the verifier.
    pub(crate) max_stack: u32,
    pub(crate) strict: bool,
    /// A function expression's own name, bound inside it.
    pub(crate) self_name: Option<Key>,
    pub(crate) params: Vec<Key>,
    pub(crate) vars: Vec<Key>,
    /// Function declarations to instantiate on entry: their names and indices in `functions`.
    pub(crate) fun_decls: Vec<(Key, u32)>,
    /// The body refers to `arguments` or calls `eval`.
    pub(crate) uses_arguments: bool,
    pub(crate) source: Rc<Source>,
    /// Where the function's text starts and ends in the source.
    pub(crate) span: (u32, u32),
}

impl Code {
    /// The innermost handler around instruction `pc`.
    pub(crate) fn handler(&self, pc: usize) -> Option<Handler> {
        self.handlers.iter().copied().find(|h| (h.start as usize) <= pc && pc < h.end as usize)
    }

    /// The line and column of instruction `pc`.
    pub(crate) fn line_column(&self, pc: usize) -> (u32, u32) {
        let offset = self.positions.get(pc).copied().unwrap_or(self.span.0);
        self.source.lines.line_column(offset)
    }
}

/// What an instruction does to the stack on each way out of it.
struct Effect {
    pops: u32,
    /// Values pushed when it goes on to the next instruction; `None` when it never does.
    next: Option<u32>,
    /// Instruction indices it may jump to, each with the values it pushes on that path.
    jumps: Vec<(u32, u32)>,
}

/// The values an instruction pops, and pushes when it goes on to the next instruction (0 for one
/// that never does), for the compiler's count of the stack depth.
pub(crate) fn fallthrough_effect(op: Op) -> (u32, u32) {
    match op {
        Op::EndFinally(_) => (2, 0),
        _ => effect(op, &[]).map_or((0, 0), |fx| (fx.pops, fx.next.unwrap_or(0))),
    }
}

fn effect(op: Op, finally_tables: &[Vec<u32>]) -> Result<Effect, String> {
    let e = |pops, pushes| Effect { pops, next: Some(pushes), jumps: Vec::new() };
    let end = |pops| Effect { pops, next: None, jumps: Vec::new() };
    Ok(match op {
        Op::Undefined | Op::Null | Op::True | Op::False | Op::Number(_) | Op::String(_) | Op::This => e(0, 1),
        Op::GetVar(_) | Op::TypeofVar(_) | Op::DeleteVar(_) | Op::IncVar { .. } | Op::RefVar(_) => e(0, 1),
        Op::GetRef(_) => e(1, 1),
        Op::PutRef(_) => e(2, 1),
        Op::NewObject | Op::NewArray | Op::Closure(_) | Op::RegExp(_) => e(0, 1),
        Op::Pop | Op::PushWith | Op::PushCatch(_) | Op::SetCompletion => e(1, 0),
        Op::PopUnder(n) => e(n.checked_add(1).ok_or("operand too large")?, 1),
        Op::Dup => e(1, 2),
        Op::Dup2 => e(2, 4),
        Op::Swap | Op::ToKey => e(2, 2),
        Op::Rot3 => e(3, 3),
        Op::GetVarThis(_) => e(0, 2),
        Op::SetVar(_) | Op::GetMember(_) | Op::DeleteMember(_) | Op::IncMember { .. } => e(1, 1),
        Op::Neg | Op::Plus | Op::BitNot | Op::Not | Op::Typeof | Op::ToNumber | Op::ArrayHole | Op::ForInStart | Op::CheckObject => e(1, 1),
        Op::GetMemberThis(_) => e(1, 2),
        Op::SetMember(_) | Op::GetIndex | Op::DeleteIndex | Op::IncIndex { .. } | Op::Binary(_) => e(2, 1),
        Op::ArrayPush | Op::InitProp(_) | Op::InitGetter(_) | Op::InitSetter(_) => e(2, 1),
        Op::GetIndexThis => e(2, 2),
        Op::SetIndex => e(3, 1),
        Op::Call(n) | Op::CallEval(n) => e(n.checked_add(2).ok_or("operand too large")?, 1),
        Op::New(n) => e(n.checked_add(1).ok_or("operand too large")?, 1),
        Op::PopEnv => e(0, 0),
        Op::Jump(t) => Effect { pops: 0, next: None, jumps: vec![(t, 0)] },
        Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => Effect { pops: 1, next: Some(0), jumps: vec![(t, 0)] },
        Op::JumpIfFalseKeep(t) | Op::JumpIfTrueKeep(t) => Effect { pops: 1, next: Some(0), jumps: vec![(t, 1)] },
        Op::Return | Op::Throw => end(1),
        Op::ReturnCompletion | Op::ThrowReferenceError(_) => end(0),
        Op::EndFinally(t) => {
            let table = finally_tables.get(t as usize).ok_or_else(|| format!("finally table {t} out of range"))?;
            Effect { pops: 2, next: Some(0), jumps: table.iter().map(|&target| (target, 1)).collect() }
        }
        Op::ForInNext(t) => Effect { pops: 1, next: Some(2), jumps: vec![(t, 1)] },
    })
}

/// Checks the constant, name, function and regular expression indices an instruction uses.
fn check_indices(op: Op, code: &Code) -> Result<(), String> {
    let check = |i: u32, len: usize, what: &str| if (i as usize) < len { Ok(()) } else { Err(format!("{what} {i} out of range")) };
    match op {
        Op::Number(i) => check(i, code.numbers.len(), "number"),
        Op::String(i) | Op::ThrowReferenceError(i) => check(i, code.strings.len(), "string"),
        Op::GetVar(i)
        | Op::GetVarThis(i)
        | Op::SetVar(i)
        | Op::RefVar(i)
        | Op::GetRef(i)
        | Op::PutRef(i)
        | Op::TypeofVar(i)
        | Op::DeleteVar(i)
        | Op::IncVar { name: i, .. }
        | Op::GetMember(i)
        | Op::GetMemberThis(i)
        | Op::SetMember(i)
        | Op::DeleteMember(i)
        | Op::IncMember { name: i, .. }
        | Op::InitProp(i)
        | Op::InitGetter(i)
        | Op::InitSetter(i)
        | Op::PushCatch(i) => check(i, code.names.len(), "name"),
        Op::Closure(i) => check(i, code.functions.len(), "function"),
        Op::RegExp(i) => check(i, code.regexps.len(), "regular expression"),
        _ => Ok(()),
    }
}

/// Verifies `code` (not the functions inside it, which are verified when they are compiled) and
/// returns the most values it keeps on the stack.
pub(crate) fn verify(code: &Code) -> Result<u32, String> {
    let len = code.ops.len();
    if len == 0 {
        return Err("no instructions".into());
    }
    if code.positions.len() != len {
        return Err("the position table does not match the instructions".into());
    }
    for (name, _) in &code.fun_decls {
        if name.is_empty() {
            return Err("a function declaration without a name".into());
        }
    }
    if code.fun_decls.iter().any(|(_, i)| *i as usize >= code.functions.len()) {
        return Err("function declaration index out of range".into());
    }
    let mut depth: Vec<Option<u32>> = vec![None; len];
    let mut work: Vec<(u32, u32)> = vec![(0, 0)];
    for h in &code.handlers {
        if h.start > h.end || h.end as usize > len {
            return Err(format!("handler range {}..{} out of range", h.start, h.end));
        }
        work.push((h.target, h.depth.checked_add(1).ok_or("handler depth too large")?));
    }
    let mut max = 0;
    while let Some((pc, d)) = work.pop() {
        let slot = depth.get_mut(pc as usize).ok_or_else(|| format!("jump to {pc}, past the end"))?;
        match *slot {
            Some(seen) if seen == d => continue,
            Some(seen) => return Err(format!("instruction {pc} reached with stack depths {seen} and {d}")),
            None => *slot = Some(d),
        }
        max = max.max(d);
        let op = code.ops.get(pc as usize).copied().ok_or("instruction out of range")?;
        check_indices(op, code)?;
        let fx = effect(op, &code.finally_tables)?;
        let base = d.checked_sub(fx.pops).ok_or_else(|| format!("instruction {pc} ({op:?}) pops below the stack"))?;
        if let Some(pushes) = fx.next {
            if pc as usize + 1 >= len {
                return Err("the code runs past its last instruction".into());
            }
            work.push((pc + 1, base + pushes));
        }
        for (target, pushes) in fx.jumps {
            work.push((target, base + pushes));
        }
    }
    for h in &code.handlers {
        if let Some(Some(d)) = depth.get(h.start as usize)
            && *d < h.depth
            && h.start < h.end
        {
            return Err(format!("handler at {} expects {} values but its range starts with {d}", h.target, h.depth));
        }
    }
    Ok(max)
}
