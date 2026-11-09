//! The compiler: a syntax tree to bytecode, one [`Code`] per function, script or eval code.
//!
//! Variables are looked up by name at run time through the environment chain, as ECMA-262 5.1
//! section 10 describes, so `with` and direct `eval` need no special analysis. Declarations are
//! hoisted: each code lists its `var` names and function declarations, and the interpreter binds
//! them when the code is entered (section 10.5).
//!
//! The compiler counts the operand stack depth as it emits, so `break`, `continue` and `return`
//! know how many values to drop. A jump out of a `try` with a `finally` block goes through the
//! block: it pushes a completion (a value and a kind number) and jumps to the block, whose
//! `EndFinally` instruction goes on with the jump afterwards. Long left-nested runs of binary
//! operators and member accesses, right-nested assignment chains, prefix operators, `else if`
//! chains and labels are compiled with loops; every other kind of nesting is counted, and code
//! nested deeper than [`MAX_DEPTH`] gives a `SyntaxError` instead of exhausting the stack.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{
    AssignOp, Ast, BinaryOp, Expr, ExprId, ForInTarget, ForInit, FunctionId, FunctionKind, LogicalOp, PropertyValue, Stmt, StmtId, UnaryOp, UpdateOp,
    VarDecl, XmlNamePart, XmlPart, XmlQualifier,
};
use crate::bytecode::{Code, CodeKind, Handler, Op, Source, fallthrough_effect, verify};
use crate::error::{SyntaxError, TOO_DEEP};
use crate::heap::Key;
use crate::value::Str;

/// How deeply the compiler may recurse: past the parser's default nesting limit, and shallow
/// enough for a 1 MiB stack.
pub(crate) const MAX_DEPTH: u32 = 600;

/// Why code could not be compiled.
#[derive(Debug)]
pub(crate) enum CompileFail {
    Syntax(SyntaxError),
    /// The compiler made code the verifier rejects.
    Internal(String),
}

type CResult<T> = Result<T, CompileFail>;

fn internal<T>(message: impl Into<String>) -> CResult<T> {
    Err(CompileFail::Internal(message.into()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Label(usize);

/// Where `break`, `continue` and `return` go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exit {
    /// To the end of the jump context at this index.
    Break(usize),
    /// To the continue point of the loop at this index.
    Continue(usize),
    Return,
}

enum Ctx {
    /// A loop, a `switch` or a labeled statement.
    Jump { labels: Vec<String>, is_loop: bool, plain_break: bool, brk: Label, cont: Option<Label>, depth: u32, env_depth: u32 },
    /// The protected part (and catch clause) of a `try` with a `finally` block.
    Finally { try_depth: u32, env_depth: u32, entry: Label, exits: Vec<Exit> },
}

struct FnCompiler<'a> {
    ast: &'a Ast,
    source: &'a Rc<Source>,
    kind: CodeKind,
    strict: bool,
    ops: Vec<Op>,
    positions: Vec<u32>,
    pos: u32,
    numbers: Vec<f64>,
    number_index: HashMap<u64, u32>,
    strings: Vec<Str>,
    string_index: HashMap<Vec<u16>, u32>,
    names: Vec<Key>,
    name_index: HashMap<Key, u32>,
    functions: Vec<Rc<Code>>,
    regexps: Vec<(Str, Str)>,
    handlers: Vec<(u32, u32, Label, u32, u32)>,
    finally_tables: Vec<Vec<Label>>,
    labels: Vec<Option<u32>>,
    patches: Vec<(usize, Label)>,
    depth: u32,
    env_depth: u32,
    contexts: Vec<Ctx>,
    uses_arguments: bool,
    recursion: u32,
}

/// The `var` names and function declarations a body hoists.
#[derive(Default)]
struct Hoisted {
    vars: Vec<Key>,
    fun_decls: Vec<(Key, u32)>,
}

fn key_of(name: &str) -> Key {
    crate::heap::key(name)
}

impl<'a> FnCompiler<'a> {
    fn new(ast: &'a Ast, source: &'a Rc<Source>, kind: CodeKind, strict: bool, recursion: u32) -> Self {
        Self {
            ast,
            source,
            kind,
            strict,
            ops: Vec::new(),
            positions: Vec::new(),
            pos: 0,
            numbers: Vec::new(),
            number_index: HashMap::new(),
            strings: Vec::new(),
            string_index: HashMap::new(),
            names: Vec::new(),
            name_index: HashMap::new(),
            functions: Vec::new(),
            regexps: Vec::new(),
            handlers: Vec::new(),
            finally_tables: Vec::new(),
            labels: Vec::new(),
            patches: Vec::new(),
            depth: 0,
            env_depth: 0,
            contexts: Vec::new(),
            uses_arguments: false,
            recursion,
        }
    }

    // ---- emitting ----

    fn emit(&mut self, op: Op) {
        let (pops, pushes) = fallthrough_effect(op);
        self.depth = self.depth.saturating_sub(pops) + pushes;
        self.ops.push(op);
        self.positions.push(self.pos);
    }

    fn new_label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() - 1)
    }

    fn bind(&mut self, label: Label) {
        if let Some(slot) = self.labels.get_mut(label.0) {
            *slot = Some(u32::try_from(self.ops.len()).unwrap_or(u32::MAX));
        }
    }

    /// Emits a jump instruction (`make` builds it from its target) to `label`, patched when the
    /// code is finished.
    fn jump(&mut self, make: fn(u32) -> Op, label: Label) {
        self.patches.push((self.ops.len(), label));
        self.emit(make(0));
    }

    fn at(&mut self, offset: u32) {
        self.pos = offset;
    }

    fn number(&mut self, n: f64) -> u32 {
        if let Some(&i) = self.number_index.get(&n.to_bits()) {
            return i;
        }
        let i = u32::try_from(self.numbers.len()).unwrap_or(u32::MAX);
        self.numbers.push(n);
        self.number_index.insert(n.to_bits(), i);
        i
    }

    fn string(&mut self, units: &[u16]) -> u32 {
        if let Some(&i) = self.string_index.get(units) {
            return i;
        }
        let i = u32::try_from(self.strings.len()).unwrap_or(u32::MAX);
        self.strings.push(Str::from_units(units.to_vec()));
        self.string_index.insert(units.to_vec(), i);
        i
    }

    fn name_key(&mut self, key: Key) -> u32 {
        if let Some(&i) = self.name_index.get(&key) {
            return i;
        }
        let i = u32::try_from(self.names.len()).unwrap_or(u32::MAX);
        self.names.push(key.clone());
        self.name_index.insert(key, i);
        i
    }

    /// The index of a variable name, noting a use of `arguments`.
    fn var_name(&mut self, name: &str) -> u32 {
        if name == "arguments" {
            self.uses_arguments = true;
        }
        self.name_key(key_of(name))
    }

    fn push_number(&mut self, n: f64) {
        let i = self.number(n);
        self.emit(Op::Number(i));
    }

    fn enter(&mut self) -> CResult<()> {
        self.recursion += 1;
        if self.recursion > MAX_DEPTH {
            let (line, column) = self.source.lines.line_column(self.pos);
            return Err(CompileFail::Syntax(SyntaxError { message: TOO_DEEP.into(), line, column, offset: self.pos }));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.recursion -= 1;
    }

    // ---- declarations ----

    /// Collects the `var` names and function declarations of a body, outside nested functions,
    /// in source order, and compiles the functions.
    fn hoist(&mut self, body: &[StmtId]) -> CResult<Hoisted> {
        let mut out = Hoisted::default();
        let mut seen = HashSet::new();
        let mut work: Vec<StmtId> = body.iter().rev().copied().collect();
        let mut add_var = |out: &mut Hoisted, d: &VarDecl| {
            let k = key_of(&d.name.name);
            if seen.insert(k.clone()) {
                out.vars.push(k);
            }
        };
        while let Some(id) = work.pop() {
            let Some(stmt) = self.ast.stmt(id) else { return internal("statement out of range") };
            match stmt {
                Stmt::Var(decls) => decls.iter().for_each(|d| add_var(&mut out, d)),
                Stmt::FunctionDecl(fid) => {
                    let Some(name) = self.ast.function(*fid).and_then(|f| f.name.as_ref()) else {
                        return internal("function declaration without a name");
                    };
                    let k = key_of(&name.name);
                    let i = self.function(*fid)?;
                    out.fun_decls.push((k, i));
                }
                Stmt::Block(list) => work.extend(list.iter().rev()),
                Stmt::If { consequent, alternate, .. } => {
                    work.extend(alternate.iter());
                    work.push(*consequent);
                }
                Stmt::DoWhile { body, .. } | Stmt::While { body, .. } | Stmt::With { body, .. } | Stmt::Labeled { body, .. } => work.push(*body),
                Stmt::For { init, body, .. } => {
                    work.push(*body);
                    if let Some(ForInit::Var(decls)) = init {
                        decls.iter().for_each(|d| add_var(&mut out, d));
                    }
                }
                Stmt::ForIn { target, body, .. } => {
                    work.push(*body);
                    if let ForInTarget::Var(d) = target {
                        add_var(&mut out, d);
                    }
                }
                Stmt::Switch { cases, .. } => {
                    for case in cases.iter().rev() {
                        work.extend(case.body.iter().rev());
                    }
                }
                Stmt::Try { block, handler, finalizer } => {
                    if let Some(f) = finalizer {
                        work.extend(f.iter().rev());
                    }
                    if let Some(h) = handler {
                        work.extend(h.body.iter().rev());
                    }
                    work.extend(block.iter().rev());
                }
                Stmt::Expr(_)
                | Stmt::Empty
                | Stmt::Continue(_)
                | Stmt::Break(_)
                | Stmt::Return(_)
                | Stmt::Throw(_)
                | Stmt::Debugger
                | Stmt::DefaultXmlNamespace(_) => {}
            }
        }
        Ok(out)
    }

    /// Compiles a nested function and returns its index in the function table.
    fn function(&mut self, fid: FunctionId) -> CResult<u32> {
        let code = compile_function(self.ast, fid, self.source, self.recursion, true)?;
        self.functions.push(code);
        Ok(u32::try_from(self.functions.len() - 1).unwrap_or(u32::MAX))
    }

    // ---- statements ----

    fn stmts(&mut self, list: &[StmtId]) -> CResult<()> {
        for &s in list {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, id: StmtId) -> CResult<()> {
        self.enter()?;
        let saved = self.depth;
        let r = self.stmt_inner(id);
        self.depth = saved;
        self.leave();
        r
    }

    fn stmt_inner(&mut self, id: StmtId) -> CResult<()> {
        let Some(stmt) = self.ast.stmt(id) else { return internal("statement out of range") };
        self.at(self.ast.stmt_span(id).start);
        match stmt {
            Stmt::Var(decls) => self.var_decls(decls),
            Stmt::FunctionDecl(_) | Stmt::Empty | Stmt::Debugger => Ok(()),
            Stmt::Expr(e) => {
                self.expr(*e)?;
                self.emit(if self.kind == CodeKind::Function { Op::Pop } else { Op::SetCompletion });
                Ok(())
            }
            Stmt::Block(list) => self.stmts(list),
            Stmt::If { .. } => self.if_chain(id),
            Stmt::DoWhile { .. } | Stmt::While { .. } | Stmt::For { .. } | Stmt::ForIn { .. } => self.loop_stmt(id, Vec::new()),
            Stmt::Continue(label) => self.jump_stmt(label.as_ref().map(|l| l.name.as_str()), true),
            Stmt::Break(label) => self.jump_stmt(label.as_ref().map(|l| l.name.as_str()), false),
            Stmt::Return(value) => {
                match value {
                    Some(e) => self.expr(*e)?,
                    None => self.emit(Op::Undefined),
                }
                self.emit_exit(Exit::Return)
            }
            Stmt::With { object, body } => {
                self.expr(*object)?;
                self.emit(Op::PushWith);
                self.env_depth += 1;
                self.stmt(*body)?;
                self.emit(Op::PopEnv);
                self.env_depth -= 1;
                Ok(())
            }
            Stmt::Switch { .. } => self.switch_stmt(id, Vec::new()),
            Stmt::Labeled { .. } => self.labeled(id),
            Stmt::Throw(e) => {
                self.expr(*e)?;
                self.at(self.ast.stmt_span(id).start);
                self.emit(Op::Throw);
                Ok(())
            }
            Stmt::Try { block, handler, finalizer } => self.try_stmt(block, handler.as_ref(), finalizer.as_deref()),
            Stmt::DefaultXmlNamespace(e) => {
                self.expr(*e)?;
                self.emit(Op::SetDefaultNamespace);
                Ok(())
            }
        }
    }

    fn var_decls(&mut self, decls: &[VarDecl]) -> CResult<()> {
        for d in decls {
            if let Some(init) = d.init {
                self.expr(init)?;
                self.at(d.name.span.start);
                let n = self.var_name(&d.name.name);
                self.emit(Op::SetVar(n));
                self.emit(Op::Pop);
            }
        }
        Ok(())
    }

    fn if_chain(&mut self, id: StmtId) -> CResult<()> {
        let saved = self.depth;
        let end = self.new_label();
        let mut cur = id;
        while let Some(Stmt::If { test, consequent, alternate }) = self.ast.stmt(cur) {
            self.at(self.ast.stmt_span(cur).start);
            self.expr(*test)?;
            let otherwise = self.new_label();
            self.jump(Op::JumpIfFalse, otherwise);
            self.stmt(*consequent)?;
            let Some(alt) = *alternate else {
                self.bind(otherwise);
                break;
            };
            self.jump(Op::Jump, end);
            self.bind(otherwise);
            self.depth = saved;
            if matches!(self.ast.stmt(alt), Some(Stmt::If { .. })) {
                cur = alt;
            } else {
                self.stmt(alt)?;
                break;
            }
        }
        self.bind(end);
        Ok(())
    }

    fn labeled(&mut self, id: StmtId) -> CResult<()> {
        let mut labels = Vec::new();
        let mut cur = id;
        while let Some(Stmt::Labeled { label, body }) = self.ast.stmt(cur) {
            labels.push(label.name.clone());
            cur = *body;
        }
        match self.ast.stmt(cur) {
            Some(Stmt::DoWhile { .. } | Stmt::While { .. } | Stmt::For { .. } | Stmt::ForIn { .. }) => self.loop_stmt(cur, labels),
            Some(Stmt::Switch { .. }) => self.switch_stmt(cur, labels),
            _ => {
                let brk = self.new_label();
                self.push_jump_ctx(labels, false, false, brk, None);
                self.stmt(cur)?;
                self.contexts.pop();
                self.bind(brk);
                Ok(())
            }
        }
    }

    fn push_jump_ctx(&mut self, labels: Vec<String>, is_loop: bool, plain_break: bool, brk: Label, cont: Option<Label>) {
        self.contexts.push(Ctx::Jump { labels, is_loop, plain_break, brk, cont, depth: self.depth, env_depth: self.env_depth });
    }

    /// Compiles a loop body inside its jump context.
    fn loop_body(&mut self, body: StmtId, labels: Vec<String>, brk: Label, cont: Label) -> CResult<()> {
        self.push_jump_ctx(labels, true, true, brk, Some(cont));
        let r = self.stmt(body);
        self.contexts.pop();
        r
    }

    fn loop_stmt(&mut self, id: StmtId, labels: Vec<String>) -> CResult<()> {
        let brk = self.new_label();
        let cont = self.new_label();
        match self.ast.stmt(id) {
            Some(Stmt::While { test, body }) => {
                self.bind(cont);
                self.expr(*test)?;
                self.jump(Op::JumpIfFalse, brk);
                self.loop_body(*body, labels, brk, cont)?;
                self.jump(Op::Jump, cont);
            }
            Some(Stmt::DoWhile { body, test }) => {
                let top = self.new_label();
                self.bind(top);
                self.loop_body(*body, labels, brk, cont)?;
                self.bind(cont);
                self.expr(*test)?;
                self.jump(Op::JumpIfTrue, top);
            }
            Some(Stmt::For { init, test, update, body }) => {
                match init {
                    Some(ForInit::Var(decls)) => self.var_decls(decls)?,
                    Some(ForInit::Expr(e)) => {
                        self.expr(*e)?;
                        self.emit(Op::Pop);
                    }
                    None => {}
                }
                let head = self.new_label();
                self.bind(head);
                if let Some(t) = test {
                    self.expr(*t)?;
                    self.jump(Op::JumpIfFalse, brk);
                }
                self.loop_body(*body, labels, brk, cont)?;
                self.bind(cont);
                if let Some(u) = update {
                    self.expr(*u)?;
                    self.emit(Op::Pop);
                }
                self.jump(Op::Jump, head);
            }
            Some(Stmt::ForIn { target, object, body, each }) => return self.for_in(target, *object, *body, labels, *each),
            _ => return internal("not a loop"),
        }
        self.bind(brk);
        Ok(())
    }

    fn for_in(&mut self, target: &ForInTarget, object: ExprId, body: StmtId, labels: Vec<String>, each: bool) -> CResult<()> {
        if let ForInTarget::Var(d) = target {
            self.var_decls(std::slice::from_ref(d))?;
        }
        self.expr(object)?;
        self.emit(Op::ForInStart);
        let iter_depth = self.depth;
        let head = self.new_label();
        let brk = self.new_label();
        self.bind(head);
        self.jump(Op::ForInNext, brk);
        if each {
            self.emit(Op::ForEachValue);
        }
        match target {
            ForInTarget::Var(d) => {
                let n = self.var_name(&d.name.name);
                self.emit(Op::SetVar(n));
            }
            ForInTarget::Expr(e) => self.store_key_to(*e)?,
        }
        self.emit(Op::Pop);
        self.loop_body(body, labels, brk, head)?;
        self.jump(Op::Jump, head);
        self.bind(brk);
        self.depth = iter_depth;
        self.emit(Op::Pop);
        Ok(())
    }

    /// Assigns the name on top of the stack to a `for-in` target expression, leaving it on top.
    fn store_key_to(&mut self, target: ExprId) -> CResult<()> {
        let target = self.ast.unparenthesized(target);
        self.at(self.ast.expr_span(target).start);
        match self.ast.expr(target) {
            Some(Expr::Ident(name)) => {
                let n = self.var_name(name);
                self.emit(Op::SetVar(n));
            }
            Some(Expr::Member { object, property }) => {
                self.expr(*object)?;
                self.emit(Op::Swap);
                let n = self.name_key(key_of(&property.name));
                self.emit(Op::SetMember(n));
            }
            Some(Expr::Index { object, index }) => {
                self.expr(*object)?;
                self.expr(*index)?;
                self.emit(Op::ToKey);
                self.emit(Op::Rot3);
                self.emit(Op::SetIndex);
            }
            _ => {
                self.expr(target)?;
                self.emit(Op::Pop);
                self.throw_invalid_target();
            }
        }
        Ok(())
    }

    fn throw_invalid_target(&mut self) {
        let s = self.string(&"invalid assignment target".encode_utf16().collect::<Vec<u16>>());
        let depth = self.depth;
        self.emit(Op::ThrowReferenceError(s));
        self.depth = depth;
    }

    fn switch_stmt(&mut self, id: StmtId, labels: Vec<String>) -> CResult<()> {
        let Some(Stmt::Switch { discriminant, cases }) = self.ast.stmt(id) else { return internal("not a switch") };
        self.expr(*discriminant)?;
        let d = self.depth;
        let brk = self.new_label();
        let case_labels: Vec<Label> = cases.iter().map(|_| self.new_label()).collect();
        for (case, &label) in cases.iter().zip(&case_labels) {
            if let Some(test) = case.test {
                self.emit(Op::Dup);
                self.expr(test)?;
                self.at(case.span.start);
                self.emit(Op::Binary(BinaryOp::StrictEq));
                self.jump(Op::JumpIfTrue, label);
            }
        }
        match cases.iter().position(|c| c.test.is_none()).and_then(|i| case_labels.get(i)) {
            Some(&default) => self.jump(Op::Jump, default),
            None => self.jump(Op::Jump, brk),
        }
        self.push_jump_ctx(labels, false, true, brk, None);
        for (case, &label) in cases.iter().zip(&case_labels) {
            self.bind(label);
            self.depth = d;
            self.stmts(&case.body)?;
        }
        self.contexts.pop();
        self.bind(brk);
        self.depth = d;
        self.emit(Op::Pop);
        Ok(())
    }

    fn jump_stmt(&mut self, label: Option<&str>, is_continue: bool) -> CResult<()> {
        let target = self.contexts.iter().enumerate().rev().find_map(|(i, c)| match c {
            Ctx::Jump { labels, is_loop, plain_break, .. } => {
                let hit = match label {
                    Some(l) => labels.iter().any(|x| x == l) && (!is_continue || *is_loop),
                    None if is_continue => *is_loop,
                    None => *plain_break,
                };
                hit.then_some(i)
            }
            Ctx::Finally { .. } => None,
        });
        match target {
            Some(i) => self.emit_exit(if is_continue { Exit::Continue(i) } else { Exit::Break(i) }),
            None => internal("break or continue without a target"),
        }
    }

    /// Emits a jump out to `exit`, dropping the values and environments in between and going
    /// through each `finally` block on the way.
    fn emit_exit(&mut self, exit: Exit) -> CResult<()> {
        let (saved, saved_env) = (self.depth, self.env_depth);
        let mut i = self.contexts.len();
        loop {
            let Some(next) = i.checked_sub(1) else {
                if exit != Exit::Return {
                    return internal("jump target not found");
                }
                self.emit(Op::Return);
                break;
            };
            i = next;
            match self.contexts.get(i) {
                Some(&Ctx::Jump { brk, cont, depth, env_depth, .. }) => {
                    let label = match exit {
                        Exit::Break(t) if t == i => Some(brk),
                        Exit::Continue(t) if t == i => cont,
                        _ => None,
                    };
                    if let Some(label) = label {
                        self.unwind_to(depth, env_depth)?;
                        self.jump(Op::Jump, label);
                        break;
                    }
                }
                Some(&Ctx::Finally { try_depth, env_depth, entry, .. }) => {
                    self.route_through_finally(i, exit, try_depth, env_depth, entry)?;
                    break;
                }
                None => return internal("context out of range"),
            }
        }
        self.depth = saved;
        self.env_depth = saved_env;
        Ok(())
    }

    fn route_through_finally(&mut self, ctx: usize, exit: Exit, try_depth: u32, env_depth: u32, entry: Label) -> CResult<()> {
        let Some(Ctx::Finally { exits, .. }) = self.contexts.get_mut(ctx) else { return internal("not a finally context") };
        let k = match exits.iter().position(|e| *e == exit) {
            Some(k) => k,
            None => {
                exits.push(exit);
                exits.len() - 1
            }
        };
        if exit == Exit::Return {
            let Some(under) = self.depth.checked_sub(1 + try_depth) else { return internal("stack below a try block") };
            if under > 0 {
                self.emit(Op::PopUnder(under));
            }
            self.pop_envs(env_depth)?;
        } else {
            self.unwind_to(try_depth, env_depth)?;
            self.emit(Op::Undefined);
        }
        self.push_number((k + 2) as f64);
        self.jump(Op::Jump, entry);
        Ok(())
    }

    fn unwind_to(&mut self, depth: u32, env_depth: u32) -> CResult<()> {
        if self.depth < depth {
            return internal("jump to a deeper stack");
        }
        while self.depth > depth {
            self.emit(Op::Pop);
        }
        self.pop_envs(env_depth)
    }

    fn pop_envs(&mut self, env_depth: u32) -> CResult<()> {
        if self.env_depth < env_depth {
            return internal("jump into an environment");
        }
        for _ in env_depth..self.env_depth {
            self.ops.push(Op::PopEnv);
            self.positions.push(self.pos);
        }
        Ok(())
    }

    fn try_stmt(&mut self, block: &[StmtId], handler: Option<&crate::ast::CatchClause>, finalizer: Option<&[StmtId]>) -> CResult<()> {
        let try_depth = self.depth;
        let env_depth = self.env_depth;
        let fin_entry = self.new_label();
        if finalizer.is_some() {
            self.contexts.push(Ctx::Finally { try_depth, env_depth, entry: fin_entry, exits: Vec::new() });
        }
        let start = self.pc();
        self.stmts(block)?;
        if let Some(h) = handler {
            let after = self.new_label();
            let catch = self.new_label();
            self.jump(Op::Jump, after);
            let end = self.pc();
            self.handlers.push((start, end, catch, try_depth, env_depth));
            self.bind(catch);
            self.depth = try_depth + 1;
            self.at(h.param.span.start);
            let n = self.var_name(&h.param.name);
            self.emit(Op::PushCatch(n));
            self.env_depth += 1;
            self.stmts(&h.body)?;
            self.emit(Op::PopEnv);
            self.env_depth -= 1;
            self.bind(after);
        }
        let Some(fin) = finalizer else { return Ok(()) };
        let protected_end = self.pc();
        let Some(Ctx::Finally { exits, .. }) = self.contexts.pop() else { return internal("finally context missing") };
        let after = self.new_label();
        let stub = self.new_label();
        self.handlers.push((start, protected_end, stub, try_depth, env_depth));
        self.emit(Op::Undefined);
        self.push_number(0.0);
        self.bind(fin_entry);
        // In script and eval code the finally block keeps the completion value of the try or
        // catch block when it ends normally (ECMA-262 5.1 section 12.14).
        let keeps_completion = self.kind != CodeKind::Function;
        if keeps_completion {
            self.emit(Op::GetCompletion);
        }
        self.stmts(fin)?;
        if keeps_completion {
            self.emit(Op::SetCompletion);
        }
        let dispatch: Vec<Label> = exits.iter().map(|_| self.new_label()).collect();
        let table = u32::try_from(self.finally_tables.len()).unwrap_or(u32::MAX);
        self.finally_tables.push(dispatch.clone());
        self.emit(Op::EndFinally(table));
        self.jump(Op::Jump, after);
        self.bind(stub);
        self.depth = try_depth + 1;
        self.push_number(1.0);
        self.jump(Op::Jump, fin_entry);
        for (exit, label) in exits.into_iter().zip(dispatch) {
            self.bind(label);
            self.depth = try_depth + 1;
            if exit != Exit::Return {
                self.emit(Op::Pop);
            }
            self.emit_exit(exit)?;
        }
        self.bind(after);
        self.depth = try_depth;
        Ok(())
    }

    fn pc(&self) -> u32 {
        u32::try_from(self.ops.len()).unwrap_or(u32::MAX)
    }

    // ---- expressions ----

    fn expr(&mut self, id: ExprId) -> CResult<()> {
        self.enter()?;
        let r = self.expr_inner(self.ast.unparenthesized(id));
        self.leave();
        r
    }

    fn expr_inner(&mut self, id: ExprId) -> CResult<()> {
        let Some(e) = self.ast.expr(id) else { return internal("expression out of range") };
        self.at(self.ast.expr_span(id).start);
        match e {
            Expr::This => self.emit(Op::This),
            Expr::Ident(name) => {
                let n = self.var_name(name);
                self.emit(Op::GetVar(n));
            }
            Expr::Null => self.emit(Op::Null),
            Expr::Bool(b) => self.emit(if *b { Op::True } else { Op::False }),
            Expr::Number(n) => self.push_number(*n),
            Expr::String(s) => {
                let i = self.string(s);
                self.emit(Op::String(i));
            }
            Expr::RegExp { pattern, flags } => {
                self.regexps.push((Str::from_units(pattern.clone()), Str::from(flags.as_str())));
                let i = u32::try_from(self.regexps.len() - 1).unwrap_or(u32::MAX);
                self.emit(Op::RegExp(i));
            }
            Expr::Array(elements) => {
                self.emit(Op::NewArray);
                for el in elements {
                    match el {
                        Some(e) => {
                            self.expr(*e)?;
                            self.emit(Op::ArrayPush);
                        }
                        None => self.emit(Op::ArrayHole),
                    }
                }
            }
            Expr::Object(props) => self.object_literal(props)?,
            Expr::Function(fid) => {
                let i = self.function(*fid)?;
                self.emit(Op::Closure(i));
            }
            Expr::Member { .. } | Expr::Index { .. } | Expr::Call { .. } => self.chain(id)?,
            Expr::New { callee, args } => {
                self.expr(*callee)?;
                self.args(args)?;
                self.at(self.ast.expr_span(id).start);
                self.emit(Op::New(len_u32(args.len())));
            }
            Expr::Update { op, prefix, target } => self.update(*op, *prefix, *target)?,
            Expr::Unary { .. } => self.unary(id)?,
            Expr::Binary { .. } | Expr::Logical { .. } => self.binary_chain(id)?,
            Expr::Conditional { test, consequent, alternate } => {
                self.expr(*test)?;
                let d = self.depth;
                let otherwise = self.new_label();
                let end = self.new_label();
                self.jump(Op::JumpIfFalse, otherwise);
                self.expr(*consequent)?;
                self.jump(Op::Jump, end);
                self.bind(otherwise);
                self.depth = d.saturating_sub(1);
                self.expr(*alternate)?;
                self.bind(end);
            }
            Expr::Assign { .. } => self.assign_chain(id)?,
            Expr::Sequence(list) => {
                for (i, e) in list.iter().enumerate() {
                    if i > 0 {
                        self.emit(Op::Pop);
                    }
                    self.expr(*e)?;
                }
                if list.is_empty() {
                    self.emit(Op::Undefined);
                }
            }
            Expr::Paren(inner) => self.expr(*inner)?,
            Expr::XmlLiteral { parts, list } => self.xml_literal(parts, *list)?,
            Expr::XmlName { attribute, qualifier, name } => self.xml_name(*attribute, *qualifier, name)?,
            Expr::XmlScopeName(name) => {
                self.expr(*name)?;
                self.at(self.ast.expr_span(id).start);
                self.emit(Op::XmlScopeGet);
            }
            Expr::XmlDescendants { .. } | Expr::XmlFilter { .. } => self.chain(id)?,
        }
        Ok(())
    }

    fn xml_literal(&mut self, parts: &[XmlPart], list: bool) -> CResult<()> {
        for part in parts {
            match part {
                XmlPart::Text(units) => {
                    let i = self.string(units);
                    self.emit(Op::String(i));
                }
                XmlPart::Content(e) | XmlPart::Attribute(e) | XmlPart::Name(e) | XmlPart::Attributes(e) => {
                    self.expr(*e)?;
                    self.emit(Op::XmlEscape(match part {
                        XmlPart::Content(_) => 0,
                        XmlPart::Attribute(_) => 1,
                        XmlPart::Name(_) => 2,
                        _ => 3,
                    }));
                }
            }
        }
        self.emit(Op::XmlLiteral { parts: len_u32(parts.len()), list });
        Ok(())
    }

    fn xml_name(&mut self, attribute: bool, qualifier: Option<XmlQualifier>, name: &XmlNamePart) -> CResult<()> {
        match qualifier {
            Some(XmlQualifier::Any) => self.emit(Op::Null),
            Some(XmlQualifier::Expr(e)) => self.expr(e)?,
            None => {}
        }
        match name {
            XmlNamePart::Ident(n) => {
                let i = self.string(&n.encode_utf16().collect::<Vec<u16>>());
                self.emit(Op::String(i));
            }
            XmlNamePart::Star => {
                let i = self.string(&[u16::from(b'*')]);
                self.emit(Op::String(i));
            }
            XmlNamePart::Computed(e) => self.expr(*e)?,
        }
        self.emit(Op::XmlMakeName { attribute, qualified: qualifier.is_some() });
        Ok(())
    }

    /// `object.(predicate)` with the object already on the stack: each item is tested with itself
    /// as the innermost scope, in list order.
    fn xml_filter(&mut self, predicate: ExprId) -> CResult<()> {
        self.emit(Op::XmlFilterStart);
        let head = self.new_label();
        let done = self.new_label();
        self.bind(head);
        let depth = self.depth;
        self.jump(Op::XmlFilterNext, done);
        self.emit(Op::PushWith);
        self.env_depth += 1;
        self.expr(predicate)?;
        self.emit(Op::PopEnv);
        self.env_depth -= 1;
        self.emit(Op::XmlFilterTest);
        self.jump(Op::Jump, head);
        self.bind(done);
        self.depth = depth;
        self.emit(Op::XmlFilterEnd);
        Ok(())
    }

    fn args(&mut self, args: &[ExprId]) -> CResult<()> {
        for a in args {
            self.expr(*a)?;
        }
        Ok(())
    }

    fn object_literal(&mut self, props: &[crate::ast::Property]) -> CResult<()> {
        self.emit(Op::NewObject);
        for p in props {
            let n = self.name_key(p.key.to_utf16().into());
            match &p.value {
                PropertyValue::Init(e) => {
                    self.expr(*e)?;
                    self.emit(Op::InitProp(n));
                }
                PropertyValue::Get(f) => {
                    let i = self.function(*f)?;
                    self.emit(Op::Closure(i));
                    self.emit(Op::InitGetter(n));
                }
                PropertyValue::Set(f) => {
                    let i = self.function(*f)?;
                    self.emit(Op::Closure(i));
                    self.emit(Op::InitSetter(n));
                }
            }
        }
        Ok(())
    }

    /// A run of member accesses and calls (`a.b[c](d).e`), compiled from the innermost object out
    /// with a loop.
    fn chain(&mut self, id: ExprId) -> CResult<()> {
        let mut suffixes = Vec::new();
        let mut cur = id;
        loop {
            match self.ast.expr(cur) {
                Some(
                    Expr::Member { object, .. } | Expr::Index { object, .. } | Expr::XmlDescendants { object, .. } | Expr::XmlFilter { object, .. },
                ) => {
                    suffixes.push(cur);
                    cur = self.ast.unparenthesized(*object);
                }
                Some(Expr::Call { callee, .. }) => {
                    suffixes.push(cur);
                    cur = self.ast.unparenthesized(*callee);
                }
                _ => break,
            }
        }
        let is_call = |c: &Self, e: Option<&ExprId>| e.is_some_and(|&e| matches!(c.ast.expr(e), Some(Expr::Call { .. })));
        let first_is_call = is_call(self, suffixes.last());
        let mut base_has_this = false;
        let mut eval_call = false;
        match self.ast.expr(cur) {
            Some(Expr::Ident(name)) if first_is_call => {
                self.at(self.ast.expr_span(cur).start);
                let n = self.var_name(name);
                self.emit(Op::GetVarThis(n));
                base_has_this = true;
                eval_call = name == "eval";
            }
            _ => self.expr(cur)?,
        }
        let count = suffixes.len();
        for j in (0..count).rev() {
            let Some(&s) = suffixes.get(j) else { return internal("suffix out of range") };
            let next_is_call = j > 0 && is_call(self, suffixes.get(j - 1));
            let span = self.ast.expr_span(s);
            match self.ast.expr(s) {
                Some(Expr::Member { property, .. }) => {
                    self.at(span.start);
                    let n = self.name_key(key_of(&property.name));
                    self.emit(if next_is_call { Op::GetMemberThis(n) } else { Op::GetMember(n) });
                }
                Some(Expr::Index { index, .. }) => {
                    self.expr(*index)?;
                    self.at(span.start);
                    self.emit(if next_is_call { Op::GetIndexThis } else { Op::GetIndex });
                }
                Some(Expr::XmlDescendants { name, .. }) => {
                    self.expr(*name)?;
                    self.at(span.start);
                    self.emit(Op::XmlDescendants);
                }
                Some(Expr::XmlFilter { predicate, .. }) => {
                    self.at(span.start);
                    self.xml_filter(*predicate)?;
                }
                Some(Expr::Call { args, .. }) => {
                    let innermost = j + 1 == count;
                    let has_this = if innermost {
                        base_has_this
                    } else {
                        matches!(suffixes.get(j + 1).and_then(|&e| self.ast.expr(e)), Some(Expr::Member { .. } | Expr::Index { .. }))
                    };
                    if !has_this {
                        self.emit(Op::Undefined);
                    }
                    self.args(args)?;
                    self.at(span.start);
                    let n = len_u32(args.len());
                    if innermost && eval_call {
                        self.uses_arguments = true;
                        self.emit(Op::CallEval(n));
                    } else {
                        self.emit(Op::Call(n));
                    }
                }
                _ => return internal("not a member or call"),
            }
        }
        Ok(())
    }

    fn unary(&mut self, id: ExprId) -> CResult<()> {
        let mut ops = Vec::new();
        let mut cur = id;
        while let Some(Expr::Unary { op, arg }) = self.ast.expr(cur) {
            if matches!(op, UnaryOp::Delete | UnaryOp::Typeof) {
                break;
            }
            ops.push(*op);
            cur = self.ast.unparenthesized(*arg);
        }
        match self.ast.expr(cur) {
            Some(Expr::Unary { op: UnaryOp::Typeof, arg }) => self.typeof_expr(*arg)?,
            Some(Expr::Unary { op: UnaryOp::Delete, arg }) => self.delete_expr(*arg)?,
            _ => self.expr(cur)?,
        }
        self.at(self.ast.expr_span(id).start);
        for op in ops.into_iter().rev() {
            match op {
                UnaryOp::Plus => self.emit(Op::Plus),
                UnaryOp::Minus => self.emit(Op::Neg),
                UnaryOp::BitNot => self.emit(Op::BitNot),
                UnaryOp::Not => self.emit(Op::Not),
                UnaryOp::Void => {
                    self.emit(Op::Pop);
                    self.emit(Op::Undefined);
                }
                UnaryOp::Delete | UnaryOp::Typeof => return internal("unexpected operator"),
            }
        }
        Ok(())
    }

    fn typeof_expr(&mut self, arg: ExprId) -> CResult<()> {
        let arg = self.ast.unparenthesized(arg);
        if let Some(Expr::Ident(name)) = self.ast.expr(arg) {
            let n = self.var_name(name);
            self.emit(Op::TypeofVar(n));
        } else {
            self.expr(arg)?;
            self.emit(Op::Typeof);
        }
        Ok(())
    }

    fn delete_expr(&mut self, arg: ExprId) -> CResult<()> {
        let arg = self.ast.unparenthesized(arg);
        match self.ast.expr(arg) {
            Some(Expr::Ident(name)) => {
                let n = self.var_name(name);
                self.emit(Op::DeleteVar(n));
            }
            Some(Expr::Member { object, property }) => {
                self.expr(*object)?;
                let n = self.name_key(key_of(&property.name));
                self.emit(Op::DeleteMember(n));
            }
            Some(Expr::Index { object, index }) => {
                self.expr(*object)?;
                self.expr(*index)?;
                self.emit(Op::DeleteIndex);
            }
            _ => {
                self.expr(arg)?;
                self.emit(Op::Pop);
                self.emit(Op::True);
            }
        }
        Ok(())
    }

    /// A run of binary and logical operators nested on the left (`a + b + c`), compiled with a
    /// loop.
    fn binary_chain(&mut self, id: ExprId) -> CResult<()> {
        enum Step {
            Binary(BinaryOp),
            Logical(LogicalOp),
        }
        let mut spine = Vec::new();
        let mut cur = id;
        loop {
            match self.ast.expr(cur) {
                Some(Expr::Binary { op, left, right }) => {
                    spine.push((Step::Binary(*op), *right, cur));
                    cur = self.ast.unparenthesized(*left);
                }
                Some(Expr::Logical { op, left, right }) => {
                    spine.push((Step::Logical(*op), *right, cur));
                    cur = self.ast.unparenthesized(*left);
                }
                _ => break,
            }
        }
        self.expr(cur)?;
        for (step, right, node) in spine.into_iter().rev() {
            match step {
                Step::Binary(op) => {
                    self.expr(right)?;
                    self.at(self.ast.expr_span(node).start);
                    self.emit(Op::Binary(op));
                }
                Step::Logical(op) => {
                    let end = self.new_label();
                    self.jump(if op == LogicalOp::And { Op::JumpIfFalseKeep } else { Op::JumpIfTrueKeep }, end);
                    self.expr(right)?;
                    self.bind(end);
                }
            }
        }
        Ok(())
    }

    /// A chain of assignments nested on the right (`a = b.c = d[e] = 1`), compiled with a loop:
    /// each target's object and key first, from the left, then the value, then the stores from
    /// the right.
    fn assign_chain(&mut self, id: ExprId) -> CResult<()> {
        let mut chain: Vec<(AssignOp, ExprId, ExprId)> = Vec::new();
        let mut cur = id;
        while let Some(Expr::Assign { op, target, value }) = self.ast.expr(cur) {
            chain.push((*op, self.ast.unparenthesized(*target), cur));
            cur = self.ast.unparenthesized(*value);
        }
        for &(op, target, node) in &chain {
            self.at(self.ast.expr_span(node).start);
            self.assign_target(op, target)?;
        }
        self.expr(cur)?;
        for &(op, target, node) in chain.iter().rev() {
            self.at(self.ast.expr_span(node).start);
            if let Some(b) = op {
                self.emit(Op::Binary(b));
            }
            self.store(target)?;
        }
        Ok(())
    }

    /// Pushes what an assignment to `target` needs before its value: the object and key, and the
    /// current value for a compound assignment.
    fn assign_target(&mut self, op: AssignOp, target: ExprId) -> CResult<()> {
        match self.ast.expr(target) {
            Some(Expr::Ident(name)) => {
                let n = self.var_name(name);
                self.emit(Op::RefVar(n));
                if op.is_some() {
                    self.emit(Op::Dup);
                    self.emit(Op::GetRef(n));
                }
            }
            Some(Expr::Member { object, property }) => {
                self.expr(*object)?;
                if op.is_some() {
                    self.emit(Op::Dup);
                    let n = self.name_key(key_of(&property.name));
                    self.emit(Op::GetMember(n));
                } else {
                    self.emit(Op::CheckObject);
                }
            }
            Some(Expr::Index { object, index }) => {
                self.expr(*object)?;
                self.expr(*index)?;
                self.emit(Op::ToKey);
                if op.is_some() {
                    self.emit(Op::Dup2);
                    self.emit(Op::GetIndex);
                }
            }
            _ => {
                self.expr(target)?;
                if op.is_none() {
                    self.emit(Op::Pop);
                }
            }
        }
        Ok(())
    }

    fn store(&mut self, target: ExprId) -> CResult<()> {
        match self.ast.expr(target) {
            Some(Expr::Ident(name)) => {
                let n = self.var_name(name);
                self.emit(Op::PutRef(n));
            }
            Some(Expr::Member { property, .. }) => {
                let n = self.name_key(key_of(&property.name));
                self.emit(Op::SetMember(n));
            }
            Some(Expr::Index { .. }) => self.emit(Op::SetIndex),
            _ => self.throw_invalid_target(),
        }
        Ok(())
    }

    fn update(&mut self, op: UpdateOp, prefix: bool, target: ExprId) -> CResult<()> {
        let target = self.ast.unparenthesized(target);
        let dec = op == UpdateOp::Decrement;
        match self.ast.expr(target) {
            Some(Expr::Ident(name)) => {
                let n = self.var_name(name);
                self.emit(Op::IncVar { name: n, prefix, dec });
            }
            Some(Expr::Member { object, property }) => {
                self.expr(*object)?;
                let n = self.name_key(key_of(&property.name));
                self.emit(Op::IncMember { name: n, prefix, dec });
            }
            Some(Expr::Index { object, index }) => {
                self.expr(*object)?;
                self.expr(*index)?;
                self.emit(Op::IncIndex { prefix, dec });
            }
            _ => {
                self.expr(target)?;
                self.emit(Op::ToNumber);
                self.throw_invalid_target();
            }
        }
        Ok(())
    }

    // ---- finishing ----

    fn finish(mut self, hoisted: Hoisted, meta: Meta) -> CResult<Rc<Code>> {
        let resolve = |labels: &[Option<u32>], l: Label| labels.get(l.0).copied().flatten();
        for &(i, label) in &self.patches {
            let Some(target) = resolve(&self.labels, label) else { return internal("jump to an unplaced label") };
            match self.ops.get_mut(i) {
                Some(
                    Op::Jump(t)
                    | Op::JumpIfFalse(t)
                    | Op::JumpIfTrue(t)
                    | Op::JumpIfFalseKeep(t)
                    | Op::JumpIfTrueKeep(t)
                    | Op::ForInNext(t)
                    | Op::XmlFilterNext(t),
                ) => *t = target,
                _ => return internal("patching an instruction that is not a jump"),
            }
        }
        let mut handlers = Vec::new();
        for &(start, end, label, depth, env_depth) in &self.handlers {
            let Some(target) = resolve(&self.labels, label) else { return internal("handler at an unplaced label") };
            handlers.push(Handler { start, end, target, depth, env_depth });
        }
        let mut finally_tables = Vec::new();
        for table in &self.finally_tables {
            let resolved: Option<Vec<u32>> = table.iter().map(|&l| resolve(&self.labels, l)).collect();
            let Some(resolved) = resolved else { return internal("finally exit at an unplaced label") };
            finally_tables.push(resolved);
        }
        let mut code = Code {
            ops: std::mem::take(&mut self.ops),
            positions: std::mem::take(&mut self.positions),
            numbers: std::mem::take(&mut self.numbers),
            strings: std::mem::take(&mut self.strings),
            names: std::mem::take(&mut self.names),
            functions: std::mem::take(&mut self.functions),
            regexps: std::mem::take(&mut self.regexps),
            handlers,
            finally_tables,
            max_stack: 0,
            strict: self.strict,
            self_name: meta.self_name,
            params: meta.params,
            vars: hoisted.vars,
            fun_decls: hoisted.fun_decls,
            uses_arguments: self.uses_arguments,
            source: self.source.clone(),
            span: meta.span,
        };
        code.max_stack = verify(&code).map_err(CompileFail::Internal)?;
        Ok(Rc::new(code))
    }
}

/// What a code knows about the function it is, if it is one.
#[derive(Default)]
struct Meta {
    self_name: Option<Key>,
    params: Vec<Key>,
    span: (u32, u32),
}

fn len_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Compiles function `fid` of `ast`. With `bind_name`, a function expression's name is bound
/// inside it.
pub(crate) fn compile_function(ast: &Ast, fid: FunctionId, source: &Rc<Source>, recursion: u32, bind_name: bool) -> CResult<Rc<Code>> {
    let Some(f) = ast.function(fid) else { return internal("function out of range") };
    let mut c = FnCompiler::new(ast, source, CodeKind::Function, f.strict, recursion);
    c.at(f.span.start);
    c.enter()?;
    let hoisted = c.hoist(&f.body)?;
    c.stmts(&f.body)?;
    c.at(f.span.end.saturating_sub(1));
    c.emit(Op::Undefined);
    c.emit(Op::Return);
    let name = f.name.as_ref().map(|n| key_of(&n.name));
    let meta = Meta {
        self_name: if bind_name && f.kind == FunctionKind::Expression { name } else { None },
        params: f.params.iter().map(|p| key_of(&p.name)).collect(),
        span: (f.span.start, f.span.end),
    };
    c.finish(hoisted, meta)
}

/// Compiles a script (`kind` [`CodeKind::Global`]) or eval code ([`CodeKind::Eval`]).
pub(crate) fn compile_program(program: &crate::Program, source: &Rc<Source>, kind: CodeKind) -> CResult<Rc<Code>> {
    let mut c = FnCompiler::new(&program.ast, source, kind, program.strict, 0);
    let hoisted = c.hoist(&program.body)?;
    c.stmts(&program.body)?;
    c.emit(Op::ReturnCompletion);
    let end = u32::try_from(source.text.len()).unwrap_or(u32::MAX);
    c.finish(hoisted, Meta { span: (0, end), ..Meta::default() })
}

/// The function of a `Function` constructor's source, `(function anonymous(…) {…})`: the program
/// must be that one parenthesized function expression and nothing else. The name `anonymous` is
/// not bound inside the function (section 15.3.2.1).
pub(crate) fn compile_constructed_function(program: &crate::Program, source: &Rc<Source>) -> CResult<Option<Rc<Code>>> {
    let [stmt] = program.body.as_slice() else { return Ok(None) };
    let Some(Stmt::Expr(e)) = program.ast.stmt(*stmt) else { return Ok(None) };
    let Some(Expr::Paren(inner)) = program.ast.expr(*e) else { return Ok(None) };
    let Some(Expr::Function(fid)) = program.ast.expr(*inner) else { return Ok(None) };
    compile_function(&program.ast, *fid, source, 0, false).map(Some)
}
