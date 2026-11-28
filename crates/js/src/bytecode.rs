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
    /// Pushes a copy of the value `n` places under the top one (`Pick(0)` is `Dup`).
    Pick(u32),
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
    /// Jumps, keeping the top value, when it is neither `undefined` nor `null`; otherwise pops
    /// it (`??`).
    JumpIfNotNullishKeep(u32),
    /// When the top value is `undefined` or `null`, pops `drop` values (the top one among them)
    /// and jumps; otherwise goes on with the stack as it was (a step of an optional chain).
    JumpIfNullish {
        target: u32,
        drop: u32,
    },
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
    /// `object value` → `object`: `__proto__: value` sets the object's prototype to the value
    /// when it is an object or `null` (ECMA-262 2015 section B.3.1).
    InitProto,
    /// `function` → `function`: gives the new function the `name` `names[i]` (ECMA-262 2015
    /// section 9.2.11).
    SetFunctionName(u32),
    /// `name function` → `name function`: gives the new function the computed `name` on the
    /// stack, after the prefix.
    SetFunctionNameKey(NamePrefix),
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
    /// Starts (`true`) or ends (`false`) a class's heritage and computed keys in code that is not
    /// strict mode code: the code between runs as strict mode code (ECMA-262 2015 section 10.2.1).
    StrictClass(bool),
    /// `[superclass]` → `constructor prototype`: makes a class's prototype object and its
    /// constructor from `functions[ctor]`, the superclass on the stack when `heritage` is true
    /// (ECMA-262 2015 section 14.5.14).
    ClassStart {
        ctor: u32,
        heritage: bool,
    },
    /// `constructor prototype key` → `constructor prototype`: defines the method, getter (`kind` 1)
    /// or setter (`kind` 2) made from `functions[func]` under the key, on the constructor when
    /// `is_static` is true and otherwise on the prototype.
    ClassMethod {
        func: u32,
        kind: u8,
        is_static: bool,
    },
    /// `object` → `object`: defines the enumerable method, getter (`kind` 1) or setter (`kind`
    /// 2) made from `functions[func]` under `names[name]` on an object literal's object, its home
    /// object (section 12.2.6.8).
    InitMethod {
        func: u32,
        kind: u8,
        name: u32,
    },
    /// `this key` → `this home key`: the key as a property name, with the prototype of the
    /// running method's home object between, for a `super` property (section 12.3.5.3).
    SuperRef,
    /// `this home key` → the property's value, read from the home object with `this` as the
    /// receiver.
    SuperGet,
    /// `this home key` → `value this`, for a call.
    SuperGetThis,
    /// `this home key value` → `value`, written through the home object with `this` as the
    /// receiver.
    SuperSet,
    /// `this home key` → `this home key this home key`.
    SuperDup,
    /// `this home key` → the old or new value: `++` or `--` on a `super` property.
    SuperInc {
        prefix: bool,
        dec: bool,
    },
    /// Pushes the constructor `super(…)` calls: the prototype of the running class constructor.
    SuperCtor,
    /// `constructor arg…` → the object it constructs, with the running constructor's
    /// `new.target`.
    SuperCall(u32),
    /// `constructor array` → the object it constructs, with the array's elements as arguments.
    SuperCallSpread,
    /// `value` → `value`: binds the running derived constructor's `this` to the value; a
    /// `ReferenceError` when it is bound already.
    BindThis,
    /// Pushes `new.target`.
    NewTarget,
    /// `value` → its Iterator Record (GetIterator, ECMA-262 11th edition section 7.4.1).
    GetIterator,
    /// `record` → `record value`, or jumps with `record` when the iterator is done.
    IterStep(u32),
    /// `record x₁…xₙ` → `record x₁…xₙ value`: the next value, `undefined` once it is done.
    IterValue(u32),
    /// `record` → `record`: steps the iterator without reading the result's `value`, for a hole
    /// in an array pattern.
    IterSkip,
    /// `record x₁…xₙ` → `record x₁…xₙ array`: the values left, in an array.
    IterRest(u32),
    /// `record` → nothing: closes the iterator unless it is done (IteratorClose).
    IterClose,
    /// `record exception` → throws the exception after closing the iterator unless it is done.
    IterCloseThrow,
    /// `value` → `value`: a `TypeError` when it is `undefined` or `null`.
    RequireObjectCoercible,
    /// `object key x₁…xₙ` → `object key x₁…xₙ object[key]`: an object pattern's property.
    PatternGet(u32),
    /// `object key value` → `object`: a property with a computed name.
    InitIndex,
    /// `object key function` → `object`: a getter (`true`) or a setter with a computed name.
    InitAccessorIndex(bool),
    /// Ends a generator function's call once its parameters are initialized: the frame becomes a
    /// new generator object, which the call returns. The first `next` goes on after it.
    GeneratorStart,
    /// `value` → `received mode`: suspends the generator, giving its caller `{ value, done: false }`
    /// (or, when true, the value itself, an iterator result from `yield*`'s delegate). It goes on
    /// when the generator is resumed, with the value it was resumed with and how (0 `next`, 1
    /// `throw`, 2 `return`).
    Yield(bool),
    /// `received mode` → `received`: goes on for `next`, throws the value for `throw`, and jumps
    /// with it for `return`.
    Resume(u32),
    /// `value` → `iterator next`: GetIterator (ECMA-262 11th edition, section 7.4.1) for `yield*`,
    /// which calls the iterator's methods itself.
    DelegateIterator,
    /// `iterator next received mode` → `iterator next mode method iterator received`, ready for
    /// `Call(1)`: the delegate's `next`, `throw` or `return`. With `return` and no `return` method
    /// it jumps with `iterator next received`; with `throw` and no `throw` method it closes the
    /// iterator and throws a `TypeError` (section 14.4.14).
    DelegateCall(u32),
    /// `mode result` → `result` when the iterator result is not done; when it is, jumps with
    /// `mode value`. A result that is not an object is a `TypeError`.
    DelegateResult(u32),
    /// `mode value` → `value`; for `return`, jumps with it.
    DelegateDone(u32),
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
    /// The function has parameters with default values or destructured parameters: its
    /// parameters start uninitialized, its code initializes them and then enters its body with
    /// [`Op::BodyEnv`].
    pub(crate) param_init: bool,
    /// The code is a class constructor's: calling it without `new` is a `TypeError`.
    pub(crate) class_ctor: bool,
    /// The code is a derived class constructor's: its `this` is bound by `super(…)`.
    pub(crate) derived: bool,
    /// The code reads `new.target`, itself or in an arrow function inside it.
    pub(crate) uses_new_target: bool,
    /// The names the destructured parameters bind, which start uninitialized beside the
    /// parameters. A destructured parameter's own entry in `params` is a name no script can
    /// write.
    pub(crate) pattern_names: Vec<Key>,
    /// The code was read with [`crate::Options::es2015`]: its function's `length` can be
    /// redefined, as ECMA-262 2015 section 19.2.4.1 has it.
    pub(crate) es2015: bool,
    /// The `name` its function objects get when they are made (ECMA-262 2015 sections 14.1.19
    /// and 14.1.20): a declaration's or a named function expression's name, with
    /// [`crate::Options::es2015`].
    pub(crate) own_name: Option<Key>,
    /// The code is a method's, a getter's or a setter's, read with [`crate::Options::es2015`], or
    /// a class method's or accessor's: its functions have no `prototype` and cannot be
    /// constructed (section 14.3.8).
    pub(crate) method: bool,
    /// The code is a generator function's: a call makes a generator object ([`Op::GeneratorStart`]),
    /// and it is not a constructor.
    pub(crate) generator: bool,
}

/// What [`Op::SetFunctionNameKey`] puts before a computed name: `get ` and `set ` for accessors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NamePrefix {
    None,
    Get,
    Set,
}

impl NamePrefix {
    /// The prefix's code units, with the space that follows it.
    pub(crate) fn units(self) -> &'static [u16] {
        const GET: &[u16] = &[0x67, 0x65, 0x74, 0x20];
        const SET: &[u16] = &[0x73, 0x65, 0x74, 0x20];
        match self {
            NamePrefix::None => &[],
            NamePrefix::Get => GET,
            NamePrefix::Set => SET,
        }
    }

    /// The prefix with the space that follows it, as text.
    pub(crate) fn text(self) -> &'static str {
        match self {
            NamePrefix::None => "",
            NamePrefix::Get => "get ",
            NamePrefix::Set => "set ",
        }
    }
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

    /// The parameters are plain names, without default values or a rest parameter, so outside
    /// strict mode code the `arguments` object follows them (ECMA-262 2015 section 9.2.12).
    pub(crate) fn simple_params(&self) -> bool {
        !self.param_init && self.rest.is_none()
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
        Op::Pick(n) => {
            let below = n.checked_add(1).ok_or("operand too large")?;
            e(below, below.checked_add(1).ok_or("operand too large")?)
        }
        Op::GetVarThis(_) => e(0, 2),
        Op::SetVar(_) | Op::GetMember(_) | Op::DeleteMember(_) | Op::IncMember { .. } => e(1, 1),
        Op::Neg | Op::Plus | Op::BitNot | Op::Not | Op::Typeof | Op::ToNumber | Op::ArrayHole | Op::ForInStart | Op::CheckObject => e(1, 1),
        Op::GetMemberThis(_) => e(1, 2),
        Op::SetMember(_) | Op::GetIndex | Op::DeleteIndex | Op::IncIndex { .. } | Op::Binary(_) => e(2, 1),
        Op::ArrayPush | Op::InitProp(_) | Op::InitGetter(_) | Op::InitSetter(_) | Op::InitProto => e(2, 1),
        Op::SetFunctionName(_) => e(1, 1),
        Op::SetFunctionNameKey(_) => e(2, 2),
        Op::GetIndexThis => e(2, 2),
        Op::SetIndex => e(3, 1),
        Op::Call(n) | Op::CallEval(n) => e(n.checked_add(2).ok_or("operand too large")?, 1),
        Op::New(n) => e(n.checked_add(1).ok_or("operand too large")?, 1),
        Op::PopEnv => e(0, 0),
        Op::Jump(t) => Effect { pops: 0, next: None, jumps: vec![(t, 0)] },
        Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => Effect { pops: 1, next: Some(0), jumps: vec![(t, 0)] },
        Op::JumpIfFalseKeep(t) | Op::JumpIfTrueKeep(t) | Op::JumpIfNotNullishKeep(t) => Effect { pops: 1, next: Some(0), jumps: vec![(t, 1)] },
        Op::JumpIfNullish { drop: 0, .. } => return Err("a nullish jump that drops no values".into()),
        Op::JumpIfNullish { target, drop } => Effect { pops: drop, next: Some(drop), jumps: vec![(target, 0)] },
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
        Op::PushBlock(_) | Op::CopyEnv | Op::BodyEnv | Op::StrictClass(_) => e(0, 0),
        Op::InitVar(_) | Op::AnnexB(_) => e(1, 0),
        Op::LoadParam(_) | Op::TemplateObject(_) => e(0, 1),
        Op::ArraySpread | Op::NewSpread => e(2, 1),
        Op::CallSpread(_) => e(3, 1),
        Op::ClassStart { heritage, .. } => e(u32::from(heritage), 2),
        Op::ClassMethod { .. } => e(3, 2),
        Op::InitMethod { .. } => e(1, 1),
        Op::SuperRef => e(2, 3),
        Op::SuperGet => e(3, 1),
        Op::SuperGetThis => e(3, 2),
        Op::SuperSet => e(4, 1),
        Op::SuperDup => e(3, 6),
        Op::SuperInc { .. } => e(3, 1),
        Op::SuperCtor | Op::NewTarget => e(0, 1),
        Op::SuperCall(n) => e(n.checked_add(1).ok_or("operand too large")?, 1),
        Op::SuperCallSpread => e(2, 1),
        Op::BindThis => e(1, 1),
        Op::ToStr | Op::GetIterator | Op::IterSkip | Op::RequireObjectCoercible => e(1, 1),
        Op::IterStep(t) => Effect { pops: 1, next: Some(2), jumps: vec![(t, 1)] },
        Op::IterValue(n) | Op::IterRest(n) => e(n.checked_add(1).ok_or("operand too large")?, n.checked_add(2).ok_or("operand too large")?),
        Op::PatternGet(n) => e(n.checked_add(2).ok_or("operand too large")?, n.checked_add(3).ok_or("operand too large")?),
        Op::IterClose => e(1, 0),
        Op::IterCloseThrow => end(2),
        Op::InitIndex | Op::InitAccessorIndex(_) => e(3, 1),
        Op::GeneratorStart => e(0, 0),
        Op::Yield(_) => e(1, 2),
        Op::Resume(t) | Op::DelegateDone(t) => Effect { pops: 2, next: Some(1), jumps: vec![(t, 1)] },
        Op::DelegateIterator => e(1, 2),
        Op::DelegateCall(t) => Effect { pops: 2, next: Some(4), jumps: vec![(t, 1)] },
        Op::DelegateResult(t) => Effect { pops: 2, next: Some(1), jumps: vec![(t, 2)] },
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
        | Op::SetFunctionName(i)
        | Op::PushCatch(i)
        | Op::InitVar(i)
        | Op::LoadParam(i)
        | Op::AnnexB(i) => check(i, code.names.len(), "name"),
        Op::Closure(i) | Op::ClassStart { ctor: i, .. } | Op::ClassMethod { func: i, .. } => check(i, code.functions.len(), "function"),
        Op::InitMethod { func, name, .. } => {
            check(func, code.functions.len(), "function")?;
            check(name, code.names.len(), "name")
        }
        Op::PushBlock(i) => check(i, code.scopes.len(), "scope"),
        Op::TemplateObject(i) => check(i, code.templates.len(), "template"),
        Op::RegExp(i) => check(i, code.regexps.len(), "regular expression"),
        Op::GeneratorStart | Op::Yield(_) if !code.generator => Err("a generator instruction outside a generator function".into()),
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
