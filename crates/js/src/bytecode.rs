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
    /// Pushes the completion value, which a `finally` block in script and eval code keeps and
    /// sets back with `SetCompletion` when it ends normally.
    GetCompletion,
    /// `[qualifier] name` → an E4X name object.
    XmlMakeName {
        attribute: bool,
        qualified: bool,
    },
    /// `object name` → the descendants that match.
    XmlDescendants,
    /// `value` → the string an embedded expression puts in a literal (0 content, 1 attribute
    /// value, 2 name, 3 run of attributes).
    XmlEscape(u8),
    /// The `parts` strings → the XML or XMLList they spell.
    XmlLiteral {
        parts: u32,
        list: bool,
    },
    /// `name` → the value of the name on the nearest XML value in scope that holds it.
    XmlScopeGet,
    /// `name` → `object name`: the nearest XML value in scope that holds the name, for an
    /// assignment, update or `delete`.
    XmlScopeRef,
    /// `object` → the filter's state.
    XmlFilterStart,
    /// `state` → `state item`, or jumps with `state` when the items are used up.
    XmlFilterNext(u32),
    /// `state value` → `state`, the current item kept when the value is truthy.
    XmlFilterTest,
    /// `state` → the XMLList of the items kept.
    XmlFilterEnd,
    /// `iterator name x₁…xₙ` → `iterator name x₁…xₙ value`: the value under the name a `for each`
    /// got, read after the `n` values its target pushed.
    ForEachValue(u32),
    /// `value` → nothing: sets the default XML namespace of the running code.
    SetDefaultNamespace,
    /// Enters a block's environment: the `let` and `const` names of `scopes[i]`, not yet
    /// initialized, and its function declarations.
    PushBlock(u32),
    /// Replaces the current environment with a copy of it, for a `for` loop's next iteration.
    CopyEnv,
    /// `value` → nothing: initializes the nearest binding of `names[i]`, a `let`, a `const` or
    /// a parameter of a function with default or rest parameters.
    InitVar(u32),
    /// Pushes the value of the parameter `names[i]` of the running function, initialized or not.
    LoadParam(u32),
    /// Ends the parameters of a function with default or rest parameters: enters the environment
    /// of its body's variables and function declarations, and of its `let` and `const` names.
    BodyEnv,
    /// `value` → nothing: assigns the function declared in a block outside strict mode code to
    /// the variable `names[i]` of the running code (ECMA-262 2015 Annex B.3.3).
    AnnexB(u32),
    /// `array value` → `array`, with the elements of the value appended (spread).
    ArraySpread,
    /// `function this array` → result: a call with the array's elements as its arguments, a
    /// direct eval when `eval` is true and the function is the global `eval`.
    CallSpread(bool),
    /// `function array` → the new object, constructed with the array's elements as arguments.
    NewSpread,
    /// `value` → its string (a template literal's substitution).
    ToStr,
    /// Pushes the template object of `templates[i]`, the same object for each evaluation.
    TemplateObject(u32),
}

/// The names a block declares, entered with [`Op::PushBlock`].
#[derive(Debug, Default)]
pub(crate) struct BlockScope {
    /// The `let` and `const` names, each with whether it is a `const`.
    pub(crate) lets: Vec<(Key, bool)>,
    /// The function declarations: their names and indices in `functions`.
    pub(crate) funcs: Vec<(Key, u32)>,
}

/// A tagged template's strings: as read and as written.
#[derive(Debug)]
pub(crate) struct Template {
    pub(crate) cooked: Vec<Str>,
    pub(crate) raw: Vec<Str>,
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
    /// Which source this is, as [`crate::Realm::error_source`] gives it.
    pub(crate) tag: u32,
    /// Writes to read-only and getter-only properties, and new properties on objects that are
    /// not extensible, throw in this source's code even where it is not strict mode code.
    pub(crate) strict_writes: bool,
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
    /// A function declaration's name, which `$.stack` shows.
    pub(crate) decl_name: Option<Key>,
    pub(crate) params: Vec<Key>,
    pub(crate) vars: Vec<Key>,
    /// Function declarations to instantiate on entry: their names and indices in `functions`.
    pub(crate) fun_decls: Vec<(Key, u32)>,
    /// The body refers to `arguments` or calls `eval`.
    pub(crate) uses_arguments: bool,
    pub(crate) source: Rc<Source>,
    /// Where the function's text starts and ends in the source.
    pub(crate) span: (u32, u32),
    /// The block scopes [`Op::PushBlock`] enters.
    pub(crate) scopes: Vec<BlockScope>,
    /// The scope of the `let` and `const` names at the top of the body, script or eval code.
    pub(crate) top_scope: Option<u32>,
    /// The tagged templates [`Op::TemplateObject`] makes objects of.
    pub(crate) templates: Vec<Template>,
    /// The rest parameter's name.
    pub(crate) rest: Option<Key>,
    /// The function's `length`: its parameters before the first with a default value.
    pub(crate) length: u32,
    /// The code is an arrow function's.
    pub(crate) arrow: bool,
    /// The code reads `this` or calls `eval`, itself or in an arrow function inside it.
    pub(crate) uses_this: bool,
    /// The function has default or rest parameters: its parameters start uninitialized, its
    /// code initializes them and then enters its body with [`Op::BodyEnv`].
    pub(crate) param_init: bool,
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
        Op::NewObject | Op::NewArray | Op::Closure(_) | Op::RegExp(_) | Op::GetCompletion => e(0, 1),
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
        Op::ForInNext(t) | Op::XmlFilterNext(t) => Effect { pops: 1, next: Some(2), jumps: vec![(t, 1)] },
        Op::XmlMakeName { qualified, .. } => e(1 + u32::from(qualified), 1),
        Op::XmlDescendants | Op::XmlFilterTest => e(2, 1),
        Op::XmlEscape(_) | Op::XmlScopeGet | Op::XmlFilterStart | Op::XmlFilterEnd => e(1, 1),
        Op::XmlLiteral { parts, .. } => e(parts, 1),
        Op::XmlScopeRef => e(1, 2),
        Op::ForEachValue(n) => e(n + 2, n + 3),
        Op::SetDefaultNamespace => e(1, 0),
        Op::PushBlock(_) | Op::CopyEnv | Op::BodyEnv => e(0, 0),
        Op::InitVar(_) | Op::AnnexB(_) => e(1, 0),
        Op::LoadParam(_) | Op::TemplateObject(_) => e(0, 1),
        Op::ArraySpread | Op::NewSpread => e(2, 1),
        Op::CallSpread(_) => e(3, 1),
        Op::ToStr => e(1, 1),
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
        | Op::PushCatch(i)
        | Op::InitVar(i)
        | Op::LoadParam(i)
        | Op::AnnexB(i) => check(i, code.names.len(), "name"),
        Op::Closure(i) => check(i, code.functions.len(), "function"),
        Op::PushBlock(i) => check(i, code.scopes.len(), "scope"),
        Op::TemplateObject(i) => check(i, code.templates.len(), "template"),
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
    if code.fun_decls.iter().chain(code.scopes.iter().flat_map(|s| &s.funcs)).any(|(_, i)| *i as usize >= code.functions.len()) {
        return Err("function declaration index out of range".into());
    }
    if code.top_scope.is_some_and(|i| i as usize >= code.scopes.len()) {
        return Err("top scope out of range".into());
    }
    if code.templates.iter().any(|t| t.cooked.len() != t.raw.len()) {
        return Err("a template whose strings do not match".into());
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
