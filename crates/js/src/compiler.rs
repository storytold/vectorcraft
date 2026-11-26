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

mod pattern;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{
    AssignOp, Ast, BinaryOp, Binding, Expr, ExprId, ForInTarget, ForInit, FunctionId, FunctionKind, LexKind, LogicalOp, Property, PropertyKey,
    PropertyValue, Stmt, StmtId, TemplatePiece, UnaryOp, UpdateOp, VarDecl, XmlNamePart, XmlPart, XmlQualifier,
};
use crate::bytecode::{BlockScope, Code, CodeKind, Handler, NamePrefix, Op, Source, Template, fallthrough_effect, verify};
use crate::error::{SyntaxError, TOO_DEEP};
use crate::heap::Key;
use crate::value::Str;
use pattern::Mode;

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
    /// The body of a `for-of` loop, whose Iterator Record is the top value at `depth`: a jump
    /// out of the loop goes to the label `exits` holds for it, after the loop's protected range,
    /// where the iterator is closed and the jump goes on.
    Iterator { depth: u32, env_depth: u32, exits: Vec<(Exit, Label)> },
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
    /// The code was read with [`crate::Options::es2015`]: blocks have scopes of their own.
    es2015: bool,
    /// The code is a module's ([`compile_module`]), which has no completion value.
    module: bool,
    scopes: Vec<BlockScope>,
    templates: Vec<Template>,
    uses_this: bool,
    /// The function declarations at the top of the body, which are var-scoped.
    top_funcs: HashSet<StmtId>,
    /// The function declarations a block scope instantiates.
    block_funcs: HashSet<StmtId>,
    /// The function declarations in blocks whose names are also var-scoped (Annex B.3.3).
    annex: HashSet<StmtId>,
    /// The optional chains being compiled, innermost last: where a step that finds `undefined`
    /// or `null` jumps, and the stack depth the chain started at.
    optional: Vec<(Label, u32)>,
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
            es2015: false,
            module: false,
            scopes: Vec::new(),
            templates: Vec::new(),
            uses_this: false,
            top_funcs: HashSet::new(),
            block_funcs: HashSet::new(),
            annex: HashSet::new(),
            optional: Vec::new(),
        }
    }

    /// Script and eval code keep the value of the last expression statement as their
    /// completion value; function and module code do not.
    fn keeps_completion(&self) -> bool {
        self.kind != CodeKind::Function && !self.module
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
    /// With [`crate::Options::es2015`], only the function declarations at the top of the body are
    /// hoisted; one in a block is instantiated when the block is entered, and outside strict mode
    /// code its name is also a variable (Annex B.3.3) unless that clashes with a parameter or with
    /// a `let` or a `const` of the body or of a block or loop around the declaration, or with a
    /// function of a block around the one that holds it.
    fn hoist(&mut self, body: &[StmtId], params: &[Key]) -> CResult<Hoisted> {
        let mut out = Hoisted::default();
        let mut seen = HashSet::new();
        // The `let` and `const` names and the block functions of each statement list and loop the
        // walk enters, with the index of the one around it; the body's are first, and its
        // functions are var-scoped. Only Annex B's block functions read them, so without
        // [`crate::Options::es2015`] none is collected.
        let top = if self.es2015 { self.lexical_names(body) } else { HashSet::new() };
        let mut scopes: Vec<(Option<usize>, HashSet<Key>, HashSet<Key>)> = vec![(None, top, HashSet::new())];
        let mut work: Vec<(StmtId, bool, usize)> = body.iter().rev().map(|&s| (s, false, 0)).collect();
        let mut add_var = |out: &mut Hoisted, k: Key| {
            if seen.insert(k.clone()) {
                out.vars.push(k);
            }
        };
        let mut annex: Vec<(Key, usize, StmtId)> = Vec::new();
        while let Some((id, nested, scope)) = work.pop() {
            let Some(stmt) = self.ast.stmt(id) else { return internal("statement out of range") };
            let inner = self.es2015;
            let mut enter = |names: HashSet<Key>, funcs: HashSet<Key>| {
                scopes.push((Some(scope), names, funcs));
                scopes.len() - 1
            };
            match stmt {
                Stmt::Var(decls) => decls.iter().flat_map(|d| self.ast.bound_names(&d.target)).for_each(|n| add_var(&mut out, key_of(&n.name))),
                Stmt::Lexical(..) => {}
                Stmt::FunctionDecl(fid) => {
                    let Some(name) = self.ast.function(*fid).and_then(|f| f.name.as_ref()) else {
                        return internal("function declaration without a name");
                    };
                    let k = key_of(&name.name);
                    if self.es2015 && nested {
                        if !self.strict {
                            annex.push((k, scope, id));
                        }
                    } else {
                        let i = self.function(*fid)?;
                        out.fun_decls.push((k, i));
                        self.top_funcs.insert(id);
                    }
                }
                Stmt::Block(list) => {
                    let s = if inner { enter(self.lexical_names(list), self.block_function_names(list)) } else { scope };
                    work.extend(list.iter().rev().map(|&st| (st, nested || inner, s)));
                }
                Stmt::If { consequent, alternate, .. } => {
                    // A function declaration standing alone as a branch is in a block of its own
                    // (Annex B.3.4).
                    for &branch in alternate.iter().chain(std::iter::once(consequent)) {
                        let alone = matches!(self.ast.stmt(branch), Some(Stmt::FunctionDecl(_)));
                        let s = if alone && inner { enter(HashSet::new(), HashSet::new()) } else { scope };
                        work.push((branch, nested || inner, s));
                    }
                }
                Stmt::Labeled { body, .. } => work.push((*body, nested, scope)),
                Stmt::DoWhile { body, .. } | Stmt::While { body, .. } | Stmt::With { body, .. } => work.push((*body, nested || inner, scope)),
                Stmt::For { init, body, .. } => {
                    let s = match init {
                        Some(ForInit::Var(decls)) => {
                            decls.iter().flat_map(|d| self.ast.bound_names(&d.target)).for_each(|n| add_var(&mut out, key_of(&n.name)));
                            scope
                        }
                        Some(ForInit::Lexical(_, decls)) => {
                            enter(decls.iter().flat_map(|d| self.ast.bound_names(&d.target)).map(|n| key_of(&n.name)).collect(), HashSet::new())
                        }
                        _ => scope,
                    };
                    work.push((*body, nested || inner, s));
                }
                Stmt::ForIn { target, body, .. } | Stmt::ForOf { target, body, .. } => {
                    let s = match target {
                        ForInTarget::Var(d) => {
                            self.ast.bound_names(&d.target).iter().for_each(|n| add_var(&mut out, key_of(&n.name)));
                            scope
                        }
                        ForInTarget::Lexical(_, binding) => {
                            enter(self.ast.bound_names(binding).iter().map(|n| key_of(&n.name)).collect(), HashSet::new())
                        }
                        ForInTarget::Expr(_) => scope,
                    };
                    work.push((*body, nested || inner, s));
                }
                Stmt::Switch { cases, .. } => {
                    let s = if inner {
                        let names = cases.iter().flat_map(|c| self.lexical_names(&c.body)).collect();
                        let funcs = cases.iter().flat_map(|c| self.block_function_names(&c.body)).collect();
                        enter(names, funcs)
                    } else {
                        scope
                    };
                    for case in cases.iter().rev() {
                        work.extend(case.body.iter().rev().map(|&st| (st, nested || inner, s)));
                    }
                }
                Stmt::Try { block, handler, finalizer } => {
                    if let Some(f) = finalizer {
                        let s = if inner { enter(self.lexical_names(f), self.block_function_names(f)) } else { scope };
                        work.extend(f.iter().rev().map(|&st| (st, nested || inner, s)));
                    }
                    if let Some(h) = handler {
                        let s = if inner {
                            // A `var` cannot redeclare a destructured catch parameter's names
                            // (ECMA-262 11th edition section B.3.5), so a block function of that
                            // name in the body is not also made a `var` (section B.3.3).
                            let mut names = self.lexical_names(&h.body);
                            if matches!(h.param, Binding::Pattern(_)) {
                                names.extend(self.bound_keys(&h.param));
                            }
                            enter(names, self.block_function_names(&h.body))
                        } else {
                            scope
                        };
                        work.extend(h.body.iter().rev().map(|&st| (st, nested || inner, s)));
                    }
                    let s = if inner { enter(self.lexical_names(block), self.block_function_names(block)) } else { scope };
                    work.extend(block.iter().rev().map(|&st| (st, nested || inner, s)));
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
        for (k, scope, id) in annex {
            let mut cur = Some(scope);
            let mut clash = params.contains(&k) || *k == *key_of("arguments");
            // The functions of the declaration's own block are judged with it (Annex B.3.3.4);
            // those of the blocks around it clash.
            let mut own = true;
            while let Some((outer, names, funcs)) = cur.and_then(|i| scopes.get(i)).filter(|_| !clash) {
                clash = names.contains(&k) || (!own && funcs.contains(&k));
                own = false;
                cur = *outer;
            }
            if !clash {
                self.annex.insert(id);
                add_var(&mut out, k);
            }
        }
        Ok(out)
    }

    /// The `let` and `const` names a statement list declares.
    fn lexical_names(&self, list: &[StmtId]) -> HashSet<Key> {
        list.iter()
            .filter_map(|&s| match self.ast.stmt(s) {
                Some(Stmt::Lexical(_, decls)) => Some(decls.iter().flat_map(|d| self.ast.bound_names(&d.target)).map(|n| key_of(&n.name))),
                _ => None,
            })
            .flatten()
            .collect()
    }

    /// The function declaration a statement is, or the one at the end of its chain of labels,
    /// with the statement that holds it.
    fn declared_function(&self, mut s: StmtId) -> Option<(StmtId, FunctionId)> {
        loop {
            match self.ast.stmt(s)? {
                Stmt::FunctionDecl(fid) => return Some((s, *fid)),
                Stmt::Labeled { body, .. } => s = *body,
                _ => return None,
            }
        }
    }

    /// The names of the function declarations a statement list holds, labelled ones included.
    fn block_function_names(&self, list: &[StmtId]) -> HashSet<Key> {
        list.iter()
            .filter_map(|&s| self.declared_function(s))
            .filter_map(|(_, fid)| self.ast.function(fid).and_then(|f| f.name.as_ref()))
            .map(|name| key_of(&name.name))
            .collect()
    }

    /// The scope a statement list declares with [`crate::Options::es2015`]: its `let` and
    /// `const` names, and (`functions`) its function declarations, labelled ones included. `None`
    /// when it declares none.
    fn list_scope(&mut self, list: &[StmtId], functions: bool) -> CResult<Option<u32>> {
        if !self.es2015 {
            return Ok(None);
        }
        let mut scope = BlockScope::default();
        for &s in list {
            if let Some(Stmt::Lexical(kind, decls)) = self.ast.stmt(s) {
                scope.lets.extend(decls.iter().flat_map(|d| self.ast.bound_names(&d.target)).map(|n| (key_of(&n.name), *kind == LexKind::Const)));
                continue;
            }
            let Some((decl, fid)) = self.declared_function(s).filter(|(decl, _)| functions && !self.top_funcs.contains(decl)) else { continue };
            let Some(name) = self.ast.function(fid).and_then(|f| f.name.as_ref()) else {
                return internal("function declaration without a name");
            };
            let k = key_of(&name.name);
            let i = self.function(fid)?;
            scope.funcs.push((k, i));
            self.block_funcs.insert(decl);
        }
        if scope.lets.is_empty() && scope.funcs.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.push_scope(scope)))
    }

    fn push_scope(&mut self, scope: BlockScope) -> u32 {
        self.scopes.push(scope);
        len_u32(self.scopes.len() - 1)
    }

    /// A block's statements, in an environment of their own when they declare `let`, `const` or
    /// (with [`crate::Options::es2015`]) functions.
    fn block(&mut self, list: &[StmtId]) -> CResult<()> {
        let Some(scope) = self.list_scope(list, true)? else { return self.stmts(list) };
        self.emit(Op::PushBlock(scope));
        self.env_depth += 1;
        self.stmts(list)?;
        self.emit(Op::PopEnv);
        self.env_depth -= 1;
        Ok(())
    }

    /// `let` or `const` declarations: each binding initialized with its value or `undefined`.
    fn lexical_decls(&mut self, decls: &[VarDecl]) -> CResult<()> {
        for d in decls {
            match (d.init, &d.target) {
                (Some(init), Binding::Name(name)) => self.named_expr(init, &name.name)?,
                (Some(init), Binding::Pattern(_)) => self.expr(init)?,
                (None, Binding::Name(_)) => self.emit(Op::Undefined),
                (None, Binding::Pattern(_)) => return internal("a destructuring declaration without an initializer"),
            }
            self.bind_to(&d.target, Mode::Lexical)?;
        }
        Ok(())
    }

    /// Compiles a nested function and returns its index in the function table.
    fn function(&mut self, fid: FunctionId) -> CResult<u32> {
        let code = compile_function(self.ast, fid, self.source, self.recursion, true, self.es2015)?;
        // An arrow function reads the `this` and `arguments` of the code around it.
        if code.arrow {
            self.uses_this |= code.uses_this;
            self.uses_arguments |= code.uses_arguments;
        }
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
            Stmt::Lexical(_, decls) => self.lexical_decls(decls),
            Stmt::FunctionDecl(fid) => self.function_decl(id, *fid),
            Stmt::Empty | Stmt::Debugger => Ok(()),
            Stmt::Expr(e) => {
                self.expr(*e)?;
                self.emit(if self.keeps_completion() { Op::SetCompletion } else { Op::Pop });
                Ok(())
            }
            Stmt::Block(list) => self.block(list),
            Stmt::If { .. } => self.if_chain(id),
            Stmt::DoWhile { .. } | Stmt::While { .. } | Stmt::For { .. } | Stmt::ForIn { .. } | Stmt::ForOf { .. } => self.loop_stmt(id, Vec::new()),
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
                self.statement_completion_start();
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
            Stmt::Try { block, handler, finalizer } => {
                self.statement_completion_start();
                self.try_stmt(block, handler.as_ref(), finalizer.as_deref())
            }
            Stmt::DefaultXmlNamespace(e) => {
                self.expr(*e)?;
                self.emit(Op::SetDefaultNamespace);
                Ok(())
            }
        }
    }

    /// A function declaration where it stands. One at the top of the body was instantiated on
    /// entry, and one in a block when the block was entered; outside strict mode code, one in a
    /// block (or standing alone as an `if` branch) is also assigned to its variable here
    /// (Annex B.3.3 and B.3.4).
    fn function_decl(&mut self, id: StmtId, fid: FunctionId) -> CResult<()> {
        if !self.es2015 || self.top_funcs.contains(&id) {
            return Ok(());
        }
        let Some(name) = self.ast.function(fid).and_then(|f| f.name.as_ref()) else { return internal("function declaration without a name") };
        if !self.annex.contains(&id) {
            return Ok(());
        }
        let n = self.name_key(key_of(&name.name));
        if self.block_funcs.contains(&id) {
            self.emit(Op::GetVar(n));
        } else {
            let i = self.function(fid)?;
            self.emit(Op::Closure(i));
        }
        self.emit(Op::AnnexB(n));
        Ok(())
    }

    fn var_decls(&mut self, decls: &[VarDecl]) -> CResult<()> {
        for d in decls {
            if let Some(init) = d.init {
                match &d.target {
                    Binding::Name(name) => self.named_expr(init, &name.name)?,
                    Binding::Pattern(_) => self.expr(init)?,
                }
                self.bind_to(&d.target, Mode::Var)?;
            }
        }
        Ok(())
    }

    fn if_chain(&mut self, id: StmtId) -> CResult<()> {
        let saved = self.depth;
        let end = self.new_label();
        let mut cur = id;
        self.statement_completion_start();
        while let Some(Stmt::If { test, consequent, alternate }) = self.ast.stmt(cur) {
            self.at(self.ast.stmt_span(cur).start);
            self.expr(*test)?;
            let otherwise = self.new_label();
            self.jump(Op::JumpIfFalse, otherwise);
            self.branch(*consequent)?;
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
                self.branch(alt)?;
                break;
            }
        }
        self.bind(end);
        Ok(())
    }

    /// An `if` branch. With [`crate::Options::es2015`], a function declaration standing alone as
    /// the branch is in a block of its own (Annex B.3.4).
    fn branch(&mut self, id: StmtId) -> CResult<()> {
        if self.es2015 && matches!(self.ast.stmt(id), Some(Stmt::FunctionDecl(_))) {
            return self.block(&[id]);
        }
        self.stmt(id)
    }

    fn labeled(&mut self, id: StmtId) -> CResult<()> {
        let mut labels = Vec::new();
        let mut cur = id;
        while let Some(Stmt::Labeled { label, body }) = self.ast.stmt(cur) {
            labels.push(label.name.clone());
            cur = *body;
        }
        match self.ast.stmt(cur) {
            Some(Stmt::DoWhile { .. } | Stmt::While { .. } | Stmt::For { .. } | Stmt::ForIn { .. } | Stmt::ForOf { .. }) => {
                self.loop_stmt(cur, labels)
            }
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

    /// With [`crate::Options::es2015`], in script and eval code, sets the completion value to
    /// `undefined` once before a loop's first test or step, so a loop whose body never completes
    /// gives `undefined` (ECMA-262 2015 sections 13.7.2.6, 13.7.3.6, 13.7.4.8 and 13.7.5.13).
    fn loop_completion_start(&mut self) {
        if self.es2015 && self.kind != CodeKind::Function {
            self.emit(Op::Undefined);
            self.emit(Op::SetCompletion);
        }
    }

    /// With [`crate::Options::es2015`], in script and eval code, sets the completion value to
    /// `undefined` before an `if`, `switch`, `with` or `try` statement, which completes with
    /// `undefined` when its body gives no value (ECMA-262 2015 sections 13.6.7, 13.11.7, 13.12.11
    /// and 13.15.8). An empty statement keeps the earlier value.
    fn statement_completion_start(&mut self) {
        self.loop_completion_start();
    }

    fn loop_stmt(&mut self, id: StmtId, labels: Vec<String>) -> CResult<()> {
        let brk = self.new_label();
        let cont = self.new_label();
        match self.ast.stmt(id) {
            Some(Stmt::While { test, body }) => {
                self.loop_completion_start();
                self.bind(cont);
                self.expr(*test)?;
                self.jump(Op::JumpIfFalse, brk);
                self.loop_body(*body, labels, brk, cont)?;
                self.jump(Op::Jump, cont);
            }
            Some(Stmt::DoWhile { body, test }) => {
                let top = self.new_label();
                self.loop_completion_start();
                self.bind(top);
                self.loop_body(*body, labels, brk, cont)?;
                self.bind(cont);
                self.expr(*test)?;
                self.jump(Op::JumpIfTrue, top);
            }
            Some(Stmt::For { init: Some(ForInit::Lexical(kind, decls)), test, update, body }) => {
                return self.for_lexical(*kind, decls, *test, *update, *body, labels, brk, cont);
            }
            Some(Stmt::For { init, test, update, body }) => {
                match init {
                    Some(ForInit::Var(decls)) => self.var_decls(decls)?,
                    Some(ForInit::Lexical(..)) => return internal("lexical for loop"),
                    Some(ForInit::Expr(e)) => {
                        self.expr(*e)?;
                        self.emit(Op::Pop);
                    }
                    None => {}
                }
                self.loop_completion_start();
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
            Some(Stmt::ForOf { target, iterable, body }) => return self.for_of(target, *iterable, *body, labels),
            _ => return internal("not a loop"),
        }
        self.bind(brk);
        Ok(())
    }

    /// `for (let …; test; update) body` or `for (const …; …)`: the declarations in a scope of the
    /// loop's own, copied for each iteration of a `let` loop before the update runs, so a closure
    /// made in one iteration keeps that iteration's bindings (ECMA-262 2015 section 13.7.4.7).
    #[allow(clippy::too_many_arguments)]
    fn for_lexical(
        &mut self,
        kind: LexKind,
        decls: &[VarDecl],
        test: Option<ExprId>,
        update: Option<ExprId>,
        body: StmtId,
        labels: Vec<String>,
        brk: Label,
        cont: Label,
    ) -> CResult<()> {
        let lets = decls.iter().flat_map(|d| self.bound_keys(&d.target)).map(|k| (k, kind == LexKind::Const)).collect();
        let scope = BlockScope { lets, funcs: Vec::new() };
        let scope = self.push_scope(scope);
        self.emit(Op::PushBlock(scope));
        self.env_depth += 1;
        self.lexical_decls(decls)?;
        let per_iteration = kind == LexKind::Let;
        if per_iteration {
            self.emit(Op::CopyEnv);
        }
        self.loop_completion_start();
        let head = self.new_label();
        self.bind(head);
        if let Some(t) = test {
            self.expr(t)?;
            self.jump(Op::JumpIfFalse, brk);
        }
        self.loop_body(body, labels, brk, cont)?;
        self.bind(cont);
        if per_iteration {
            self.emit(Op::CopyEnv);
        }
        if let Some(u) = update {
            self.expr(u)?;
            self.emit(Op::Pop);
        }
        self.jump(Op::Jump, head);
        self.bind(brk);
        self.emit(Op::PopEnv);
        self.env_depth -= 1;
        Ok(())
    }

    /// `for (let x in o)`, `for (const x in o)` and `for each` with them: the object read where
    /// `x` is not yet initialized, then a new binding of `x` for each name.
    fn for_in_lexical(&mut self, kind: LexKind, binding: &Binding, object: ExprId, body: StmtId, labels: Vec<String>, each: bool) -> CResult<()> {
        let lets = self.bound_keys(binding).into_iter().map(|k| (k, kind == LexKind::Const)).collect();
        let scope = self.push_scope(BlockScope { lets, funcs: Vec::new() });
        self.emit(Op::PushBlock(scope));
        self.env_depth += 1;
        self.expr(object)?;
        self.emit(Op::PopEnv);
        self.env_depth -= 1;
        self.loop_completion_start();
        self.emit(Op::ForInStart);
        let iter_depth = self.depth;
        let head = self.new_label();
        let brk = self.new_label();
        self.bind(head);
        self.jump(Op::ForInNext, brk);
        let outer_env = self.env_depth;
        self.emit(Op::PushBlock(scope));
        self.env_depth += 1;
        if each {
            // The value initializes the binding; the name under it goes next.
            self.emit(Op::ForEachValue(0));
            self.bind_to(binding, Mode::Lexical)?;
            self.emit(Op::Pop);
        } else {
            self.bind_to(binding, Mode::Lexical)?;
        }
        self.contexts.push(Ctx::Jump { labels, is_loop: true, plain_break: true, brk, cont: Some(head), depth: self.depth, env_depth: outer_env });
        let r = self.stmt(body);
        self.contexts.pop();
        r?;
        self.emit(Op::PopEnv);
        self.env_depth -= 1;
        self.jump(Op::Jump, head);
        self.bind(brk);
        self.depth = iter_depth;
        self.emit(Op::Pop);
        Ok(())
    }

    fn for_in(&mut self, target: &ForInTarget, object: ExprId, body: StmtId, labels: Vec<String>, each: bool) -> CResult<()> {
        if let ForInTarget::Lexical(kind, binding) = target {
            return self.for_in_lexical(*kind, binding, object, body, labels, each);
        }
        if let ForInTarget::Var(d) = target {
            self.var_decls(std::slice::from_ref(d))?;
        }
        self.expr(object)?;
        self.loop_completion_start();
        self.emit(Op::ForInStart);
        let iter_depth = self.depth;
        let head = self.new_label();
        let brk = self.new_label();
        self.bind(head);
        self.jump(Op::ForInNext, brk);
        match target {
            ForInTarget::Var(d) => {
                if each {
                    self.emit(Op::ForEachValue(0));
                }
                // The target takes a copy; the name or value stays for the pops below.
                self.emit(Op::Dup);
                self.bind_to(&d.target, Mode::Var)?;
            }
            ForInTarget::Expr(e) if self.is_pattern(*e) => {
                if each {
                    self.emit(Op::ForEachValue(0));
                }
                self.emit(Op::Dup);
                self.destructure(*e, Mode::Assign)?;
            }
            ForInTarget::Expr(e) if each => self.store_value_to(*e)?,
            ForInTarget::Expr(e) => self.store_key_to(*e)?,
            ForInTarget::Lexical(..) => return internal("lexical for-in loop"),
        }
        if each {
            // The value; the name under it goes next.
            self.emit(Op::Pop);
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
            Some(Expr::XmlScopeName(name)) => {
                self.xml_scope_ref(target, *name)?;
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

    /// Assigns the value under the name on top of the stack to a `for each` target expression,
    /// leaving the name and then the value on top. The target is evaluated before the value is
    /// read (ECMA-357 section 12.3).
    fn store_value_to(&mut self, target: ExprId) -> CResult<()> {
        let target = self.ast.unparenthesized(target);
        self.at(self.ast.expr_span(target).start);
        match self.ast.expr(target) {
            Some(Expr::Ident(name)) => {
                let n = self.var_name(name);
                self.emit(Op::ForEachValue(0));
                self.emit(Op::SetVar(n));
            }
            Some(Expr::Member { object, property }) => {
                self.expr(*object)?;
                self.emit(Op::ForEachValue(1));
                let n = self.name_key(key_of(&property.name));
                self.emit(Op::SetMember(n));
            }
            Some(Expr::Index { object, index }) => {
                self.expr(*object)?;
                self.expr(*index)?;
                self.emit(Op::ToKey);
                self.emit(Op::ForEachValue(2));
                self.emit(Op::SetIndex);
            }
            Some(Expr::XmlScopeName(name)) => {
                self.xml_scope_ref(target, *name)?;
                self.emit(Op::ForEachValue(2));
                self.emit(Op::SetIndex);
            }
            _ => {
                self.expr(target)?;
                self.emit(Op::Pop);
                self.emit(Op::ForEachValue(0));
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
        self.statement_completion_start();
        self.expr(*discriminant)?;
        let scope = if self.es2015 {
            let all: Vec<StmtId> = cases.iter().flat_map(|c| c.body.iter().copied()).collect();
            self.list_scope(&all, true)?
        } else {
            None
        };
        if let Some(scope) = scope {
            self.emit(Op::PushBlock(scope));
            self.env_depth += 1;
        }
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
        if scope.is_some() {
            self.emit(Op::PopEnv);
            self.env_depth -= 1;
        }
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
            Ctx::Finally { .. } | Ctx::Iterator { .. } => None,
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
                Some(&Ctx::Iterator { depth, env_depth, .. }) => {
                    // The loop is left: the jump goes past the loop's exception handler, where its
                    // iterator is closed, a `return` keeping its value (see `for_of`).
                    if exit == Exit::Return {
                        let Some(under) = self.depth.checked_sub(1 + depth) else { return internal("stack below a for-of loop") };
                        if under > 0 {
                            self.emit(Op::PopUnder(under));
                        }
                        self.pop_envs(env_depth)?;
                        self.emit(Op::Swap);
                    } else {
                        self.unwind_to(depth, env_depth)?;
                    }
                    let label = self.iterator_exit(i, exit)?;
                    self.jump(Op::Jump, label);
                    break;
                }
                None => return internal("context out of range"),
            }
        }
        self.depth = saved;
        self.env_depth = saved_env;
        Ok(())
    }

    /// The label where the `for-of` loop of the iterator context `ctx` closes its iterator for
    /// `exit` and goes on with it.
    fn iterator_exit(&mut self, ctx: usize, exit: Exit) -> CResult<Label> {
        let Some(Ctx::Iterator { exits, .. }) = self.contexts.get(ctx) else { return internal("not an iterator context") };
        if let Some(&(_, label)) = exits.iter().find(|(e, _)| *e == exit) {
            return Ok(label);
        }
        let label = self.new_label();
        let Some(Ctx::Iterator { exits, .. }) = self.contexts.get_mut(ctx) else { return internal("not an iterator context") };
        exits.push((exit, label));
        Ok(label)
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
        self.block(block)?;
        if let Some(h) = handler {
            let after = self.new_label();
            let catch = self.new_label();
            self.jump(Op::Jump, after);
            let end = self.pc();
            self.handlers.push((start, end, catch, try_depth, env_depth));
            self.bind(catch);
            self.depth = try_depth + 1;
            // A caught exception drops the value the try block gave (ECMA-262 2015 section 13.15.8).
            self.statement_completion_start();
            match &h.param {
                Binding::Name(name) => {
                    self.at(name.span.start);
                    let n = self.var_name(&name.name);
                    self.emit(Op::PushCatch(n));
                    self.env_depth += 1;
                    self.block(&h.body)?;
                    self.emit(Op::PopEnv);
                    self.env_depth -= 1;
                }
                Binding::Pattern(p) => {
                    // The exception is bound to a name no script can write, then destructured
                    // into the pattern's names (ECMA-262 11th edition section 13.15.7).
                    self.at(self.ast.expr_span(*p).start);
                    let n = self.name_key(key_of("\u{0}catch"));
                    self.emit(Op::PushCatch(n));
                    self.emit(Op::GetVar(n));
                    let lets = self.bound_keys(&h.param).into_iter().map(|k| (k, false)).collect();
                    let scope = self.push_scope(BlockScope { lets, funcs: Vec::new() });
                    self.emit(Op::PushBlock(scope));
                    self.env_depth += 2;
                    self.destructure(*p, Mode::Lexical)?;
                    self.block(&h.body)?;
                    self.emit(Op::PopEnv);
                    self.emit(Op::PopEnv);
                    self.env_depth -= 2;
                }
            }
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
        let keeps_completion = self.keeps_completion();
        if keeps_completion {
            self.emit(Op::GetCompletion);
        }
        self.block(fin)?;
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
            Expr::This => {
                self.uses_this = true;
                self.emit(Op::This);
            }
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
                        Some(e) => self.array_element(*e)?,
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
            Expr::New { callee, args } if self.has_spread(args) => {
                self.expr(*callee)?;
                self.spread_args(args)?;
                self.at(self.ast.expr_span(id).start);
                self.emit(Op::NewSpread);
            }
            Expr::New { callee, args } => {
                self.expr(*callee)?;
                self.args(args)?;
                self.at(self.ast.expr_span(id).start);
                self.emit(Op::New(len_u32(args.len())));
            }
            Expr::Arrow(fid) => {
                let i = self.function(*fid)?;
                self.emit(Op::Closure(i));
            }
            Expr::Template { quasis, exprs } => self.template(quasis, exprs)?,
            Expr::TaggedTemplate { .. } => self.chain(id)?,
            Expr::Spread(_) => return internal("spread outside an array literal or argument list"),
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
            Expr::OptionalChain(inner) => self.optional_chain(*inner, false)?,
            Expr::Optional(object) => {
                self.expr(*object)?;
                self.at(self.ast.expr_span(id).end);
                self.nullish_check()?;
            }
        }
        Ok(())
    }

    /// Pushes the nearest XML value in scope that holds the E4X name `name` of the expression
    /// `id`, then the name, as an assignment, update or `delete` uses them.
    fn xml_scope_ref(&mut self, id: ExprId, name: ExprId) -> CResult<()> {
        self.expr(name)?;
        self.at(self.ast.expr_span(id).start);
        self.emit(Op::XmlScopeRef);
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

    fn has_spread(&self, args: &[ExprId]) -> bool {
        args.iter().any(|&a| matches!(self.ast.expr(a), Some(Expr::Spread(_))))
    }

    /// `array` → `array` with the element appended: a value, or a spread's elements.
    fn array_element(&mut self, e: ExprId) -> CResult<()> {
        if let Some(Expr::Spread(arg)) = self.ast.expr(e) {
            self.expr(*arg)?;
            self.at(self.ast.expr_span(e).start);
            self.emit(Op::ArraySpread);
        } else {
            self.expr(e)?;
            self.emit(Op::ArrayPush);
        }
        Ok(())
    }

    /// Pushes an array of the arguments, for a call or `new` with a spread among them.
    fn spread_args(&mut self, args: &[ExprId]) -> CResult<()> {
        self.emit(Op::NewArray);
        for &a in args {
            self.array_element(a)?;
        }
        Ok(())
    }

    /// A template literal: its pieces joined with the strings of its substitutions.
    fn template(&mut self, quasis: &[TemplatePiece], exprs: &[ExprId]) -> CResult<()> {
        let Some((first, rest)) = quasis.split_first() else { return internal("template without pieces") };
        let i = self.string(&first.cooked);
        self.emit(Op::String(i));
        if rest.len() != exprs.len() {
            return internal("template pieces do not match its substitutions");
        }
        for (piece, &e) in rest.iter().zip(exprs) {
            self.expr(e)?;
            self.at(self.ast.expr_span(e).start);
            self.emit(Op::ToStr);
            self.emit(Op::Binary(BinaryOp::Add));
            if !piece.cooked.is_empty() {
                let i = self.string(&piece.cooked);
                self.emit(Op::String(i));
                self.emit(Op::Binary(BinaryOp::Add));
            }
        }
        Ok(())
    }

    /// Adds a tagged template's strings to the template table and returns their index.
    fn template_entry(&mut self, quasis: &[TemplatePiece]) -> u32 {
        let cooked = quasis.iter().map(|q| Str::from_units(q.cooked.clone())).collect();
        let raw = quasis.iter().map(|q| Str::from_units(q.raw.clone())).collect();
        self.templates.push(Template { cooked, raw });
        len_u32(self.templates.len() - 1)
    }

    fn object_literal(&mut self, props: &[Property]) -> CResult<()> {
        self.emit(Op::NewObject);
        for p in props {
            if let PropertyKey::Computed(k) = p.key {
                self.computed_property(k, &p.value)?;
                continue;
            }
            if self.es2015 && p.is_proto_setter(self.ast) {
                if let PropertyValue::Init(e) = &p.value {
                    self.expr(*e)?;
                    self.emit(Op::InitProto);
                }
                continue;
            }
            let units = p.key.to_utf16();
            let n = self.name_key(units.clone().into());
            match &p.value {
                PropertyValue::Init(e) => {
                    self.expr(*e)?;
                    if self.es2015 && self.is_anonymous_function(*e) {
                        self.emit(Op::SetFunctionName(n));
                    }
                    self.emit(Op::InitProp(n));
                }
                PropertyValue::Get(f) => {
                    let i = self.function(*f)?;
                    self.emit(Op::Closure(i));
                    if self.es2015 {
                        let named = self.name_key([NamePrefix::Get.units(), &units].concat().into());
                        self.emit(Op::SetFunctionName(named));
                    }
                    self.emit(Op::InitGetter(n));
                }
                PropertyValue::Set(f) => {
                    let i = self.function(*f)?;
                    self.emit(Op::Closure(i));
                    if self.es2015 {
                        let named = self.name_key([NamePrefix::Set.units(), &units].concat().into());
                        self.emit(Op::SetFunctionName(named));
                    }
                    self.emit(Op::InitSetter(n));
                }
                PropertyValue::Method(f) => {
                    let i = self.function(*f)?;
                    self.emit(Op::Closure(i));
                    self.emit(Op::SetFunctionName(n));
                    self.emit(Op::InitProp(n));
                }
            }
        }
        Ok(())
    }

    /// `object` → `object`, with a property whose name is the value of `key` (ECMA-262 11th
    /// edition section 12.2.6.8). A function defined there is named after the key (ECMA-262 2015
    /// sections 12.2.6.9 and 14.3.8).
    fn computed_property(&mut self, key: ExprId, value: &PropertyValue) -> CResult<()> {
        self.expr(key)?;
        self.emit(Op::ToKey);
        match value {
            PropertyValue::Init(e) => {
                self.expr(*e)?;
                if self.is_anonymous_function(*e) {
                    self.emit(Op::SetFunctionNameKey(NamePrefix::None));
                }
                self.emit(Op::InitIndex);
            }
            PropertyValue::Get(f) | PropertyValue::Set(f) => {
                let getter = matches!(value, PropertyValue::Get(_));
                let i = self.function(*f)?;
                self.emit(Op::Closure(i));
                self.emit(Op::SetFunctionNameKey(if getter { NamePrefix::Get } else { NamePrefix::Set }));
                self.emit(Op::InitAccessorIndex(getter));
            }
            PropertyValue::Method(f) => {
                let i = self.function(*f)?;
                self.emit(Op::Closure(i));
                self.emit(Op::SetFunctionNameKey(NamePrefix::None));
                self.emit(Op::InitIndex);
            }
        }
        Ok(())
    }

    /// The expression is an anonymous function definition (ECMA-262 2015 section 14.1.9): a
    /// function expression without a name or an arrow function, parenthesized or not.
    fn is_anonymous_function(&self, e: ExprId) -> bool {
        match self.ast.expr(self.ast.unparenthesized(e)) {
            Some(Expr::Arrow(_)) => true,
            Some(Expr::Function(fid)) => self.ast.function(*fid).is_some_and(|f| f.name.is_none()),
            _ => false,
        }
    }

    /// Compiles `e`; with [`crate::Options::es2015`], an anonymous function definition gets the
    /// `name` `name`, as a binding, parameter or assignment gives it (ECMA-262 2015 sections
    /// 12.14.4, 13.3.1.4, 13.3.2.4 and 14.1.18).
    fn named_expr(&mut self, e: ExprId, name: &str) -> CResult<()> {
        self.expr(e)?;
        if self.es2015 && self.is_anonymous_function(e) {
            let n = self.name_key(key_of(name));
            self.emit(Op::SetFunctionName(n));
        }
        Ok(())
    }

    /// A run of member accesses and calls (`a.b[c](d).e`), compiled from the innermost object out
    /// with a loop.
    fn chain(&mut self, id: ExprId) -> CResult<()> {
        self.chain_with(id, false)
    }

    /// [`Self::chain`]; with `keep_this` it leaves `value this`, the object of its last access as
    /// a call's `this` (or `undefined` when it ends with something else). An [`Expr::Optional`]
    /// step jumps out of the innermost optional chain being compiled.
    fn chain_with(&mut self, id: ExprId, keep_this: bool) -> CResult<()> {
        /// A suffix, or an optional step's check of the value before it.
        #[derive(Clone, Copy)]
        enum Link {
            Suffix(ExprId),
            Check(ExprId),
        }
        let mut links = Vec::new();
        let mut cur = id;
        loop {
            match self.ast.expr(cur) {
                Some(
                    Expr::Member { object, .. } | Expr::Index { object, .. } | Expr::XmlDescendants { object, .. } | Expr::XmlFilter { object, .. },
                ) => {
                    links.push(Link::Suffix(cur));
                    cur = self.ast.unparenthesized(*object);
                }
                Some(Expr::Call { callee, .. } | Expr::TaggedTemplate { tag: callee, .. }) => {
                    links.push(Link::Suffix(cur));
                    cur = self.ast.unparenthesized(*callee);
                }
                Some(Expr::Optional(object)) => {
                    links.push(Link::Check(cur));
                    cur = self.ast.unparenthesized(*object);
                }
                _ => break,
            }
        }
        let is_call = |c: &Self, link: Option<&Link>| matches!(link, Some(Link::Suffix(e)) if matches!(c.ast.expr(*e), Some(Expr::Call { .. } | Expr::TaggedTemplate { .. })));
        // The first suffix after the base, and for each link the next suffix out from it, with
        // the checks between them skipped.
        let first_is_call = is_call(self, links.iter().rev().find(|l| matches!(l, Link::Suffix(_))));
        let mut receiver = false;
        let mut eval_call = false;
        match self.ast.expr(cur) {
            Some(Expr::Ident(name)) if first_is_call => {
                self.at(self.ast.expr_span(cur).start);
                let n = self.var_name(name);
                self.emit(Op::GetVarThis(n));
                receiver = true;
                eval_call = name == "eval";
            }
            Some(Expr::OptionalChain(inner)) if first_is_call => {
                self.optional_chain(*inner, true)?;
                receiver = true;
            }
            _ => self.expr(cur)?,
        }
        let count = links.len();
        for j in (0..count).rev() {
            let s = match links.get(j) {
                Some(&Link::Suffix(s)) => s,
                Some(&Link::Check(c)) => {
                    eval_call = false;
                    self.at(self.ast.expr_span(c).end);
                    if receiver {
                        self.emit(Op::Swap);
                        self.nullish_check()?;
                        self.emit(Op::Swap);
                    } else {
                        self.nullish_check()?;
                    }
                    continue;
                }
                None => return internal("suffix out of range"),
            };
            let next = links.get(..j).and_then(|outer| outer.iter().rev().find(|l| matches!(l, Link::Suffix(_))));
            let next_is_call = if next.is_some() { is_call(self, next) } else { keep_this };
            let span = self.ast.expr_span(s);
            let eval = std::mem::take(&mut eval_call);
            let had_receiver = std::mem::take(&mut receiver);
            match self.ast.expr(s) {
                Some(Expr::Member { property, .. }) => {
                    self.at(span.start);
                    let n = self.name_key(key_of(&property.name));
                    self.emit(if next_is_call { Op::GetMemberThis(n) } else { Op::GetMember(n) });
                    receiver = next_is_call;
                }
                Some(Expr::Index { index, .. }) => {
                    self.expr(*index)?;
                    self.at(span.start);
                    self.emit(if next_is_call { Op::GetIndexThis } else { Op::GetIndex });
                    receiver = next_is_call;
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
                Some(call @ (Expr::Call { .. } | Expr::TaggedTemplate { .. })) => {
                    if !had_receiver {
                        self.emit(Op::Undefined);
                    }
                    let eval = match call {
                        Expr::TaggedTemplate { quasis, exprs, .. } => {
                            let t = self.template_entry(quasis);
                            self.at(span.start);
                            self.emit(Op::TemplateObject(t));
                            self.args(exprs)?;
                            self.at(span.start);
                            self.emit(Op::Call(len_u32(exprs.len()).saturating_add(1)));
                            continue;
                        }
                        Expr::Call { args, .. } => {
                            if eval {
                                self.uses_arguments = true;
                                self.uses_this = true;
                            }
                            if self.has_spread(args) {
                                self.spread_args(args)?;
                                self.at(span.start);
                                self.emit(Op::CallSpread(eval));
                                continue;
                            }
                            self.args(args)?;
                            self.at(span.start);
                            (eval, len_u32(args.len()))
                        }
                        _ => return internal("not a call"),
                    };
                    match eval {
                        (true, n) => self.emit(Op::CallEval(n)),
                        (false, n) => self.emit(Op::Call(n)),
                    }
                }
                _ => return internal("not a member or call"),
            }
        }
        if keep_this && !receiver {
            self.emit(Op::Undefined);
        }
        Ok(())
    }

    /// An optional chain: its links, with each `?.` step jumping to its end when it finds
    /// `undefined` or `null`, the result then `undefined` (`undefined undefined` with
    /// `keep_this`, for a call of the chain in parentheses).
    fn optional_chain(&mut self, inner: ExprId, keep_this: bool) -> CResult<()> {
        self.enter()?;
        let short = self.new_label();
        let end = self.new_label();
        let entry = self.depth;
        self.optional.push((short, entry));
        let r = self.chain_with(self.ast.unparenthesized(inner), keep_this);
        self.optional.pop();
        self.leave();
        r?;
        self.jump(Op::Jump, end);
        self.bind(short);
        self.depth = entry;
        self.emit(Op::Undefined);
        if keep_this {
            self.emit(Op::Undefined);
        }
        self.bind(end);
        Ok(())
    }

    /// An optional step: jumps to the end of the innermost optional chain, dropping what the
    /// chain has pushed, when the value on top is `undefined` or `null`.
    fn nullish_check(&mut self) -> CResult<()> {
        let Some(&(label, entry)) = self.optional.last() else { return internal("an optional step outside its chain") };
        let Some(drop) = self.depth.checked_sub(entry).filter(|&d| d > 0) else { return internal("an optional step without a value") };
        self.patches.push((self.ops.len(), label));
        self.emit(Op::JumpIfNullish { target: 0, drop });
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
            Some(Expr::XmlScopeName(name)) => {
                self.xml_scope_ref(arg, *name)?;
                self.emit(Op::DeleteIndex);
            }
            // `delete a?.b` is true without deleting anything when the chain stops at a `?.` step.
            Some(Expr::OptionalChain(inner))
                if matches!(self.ast.expr(self.ast.unparenthesized(*inner)), Some(Expr::Member { .. } | Expr::Index { .. })) =>
            {
                let short = self.new_label();
                let end = self.new_label();
                let entry = self.depth;
                self.optional.push((short, entry));
                let r = self.delete_expr(*inner);
                self.optional.pop();
                r?;
                self.jump(Op::Jump, end);
                self.bind(short);
                self.depth = entry;
                self.emit(Op::True);
                self.bind(end);
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
                    self.jump(
                        match op {
                            LogicalOp::And => Op::JumpIfFalseKeep,
                            LogicalOp::Or => Op::JumpIfTrueKeep,
                            LogicalOp::Coalesce => Op::JumpIfNotNullishKeep,
                        },
                        end,
                    );
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
        // `name = function () {}` names the function after an identifier written without
        // parentheses (ECMA-262 2015 section 12.14.4).
        let named = chain.last().and_then(|&(op, _, node)| match self.ast.expr(node) {
            Some(Expr::Assign { target, .. }) if op.is_none() => match self.ast.expr(*target) {
                Some(Expr::Ident(name)) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        });
        match named {
            Some(name) => self.named_expr(cur, &name)?,
            None => self.expr(cur)?,
        }
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
        if op.is_none() && self.es2015 && self.is_pattern(target) {
            return Ok(());
        }
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
            Some(Expr::XmlScopeName(name)) => {
                self.xml_scope_ref(target, *name)?;
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
        if self.es2015 && self.is_pattern(target) {
            // The assignment's value is the value assigned (ECMA-262 11th edition section 12.15.4).
            self.emit(Op::Dup);
            return self.destructure(target, Mode::Assign);
        }
        match self.ast.expr(target) {
            Some(Expr::Ident(name)) => {
                let n = self.var_name(name);
                self.emit(Op::PutRef(n));
            }
            Some(Expr::Member { property, .. }) => {
                let n = self.name_key(key_of(&property.name));
                self.emit(Op::SetMember(n));
            }
            Some(Expr::Index { .. } | Expr::XmlScopeName(_)) => self.emit(Op::SetIndex),
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
            Some(Expr::XmlScopeName(name)) => {
                self.xml_scope_ref(target, *name)?;
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
                    | Op::JumpIfNotNullishKeep(t)
                    | Op::JumpIfNullish { target: t, .. }
                    | Op::ForInNext(t)
                    | Op::IterStep(t)
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
            decl_name: meta.decl_name,
            params: meta.params,
            vars: hoisted.vars,
            fun_decls: hoisted.fun_decls,
            uses_arguments: self.uses_arguments,
            source: self.source.clone(),
            span: meta.span,
            scopes: std::mem::take(&mut self.scopes),
            top_scope: meta.top_scope,
            templates: std::mem::take(&mut self.templates),
            rest: meta.rest,
            length: meta.length,
            arrow: meta.arrow,
            uses_this: self.uses_this,
            param_init: meta.param_init,
            pattern_names: meta.pattern_names,
            es2015: self.es2015,
            own_name: meta.own_name,
            method: meta.method,
        };
        code.max_stack = verify(&code).map_err(CompileFail::Internal)?;
        Ok(Rc::new(code))
    }
}

/// What a code knows about the function it is, if it is one.
#[derive(Default)]
struct Meta {
    self_name: Option<Key>,
    decl_name: Option<Key>,
    params: Vec<Key>,
    span: (u32, u32),
    top_scope: Option<u32>,
    rest: Option<Key>,
    length: u32,
    arrow: bool,
    param_init: bool,
    pattern_names: Vec<Key>,
    own_name: Option<Key>,
    method: bool,
}

fn len_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Compiles function `fid` of `ast`. With `bind_name`, a function expression's name is bound
/// inside it. `es2015` is [`crate::Options::es2015`] of the script it is in.
pub(crate) fn compile_function(ast: &Ast, fid: FunctionId, source: &Rc<Source>, recursion: u32, bind_name: bool, es2015: bool) -> CResult<Rc<Code>> {
    let Some(f) = ast.function(fid) else { return internal("function out of range") };
    let mut c = FnCompiler::new(ast, source, CodeKind::Function, f.strict, recursion);
    c.es2015 = es2015;
    c.at(f.span.start);
    c.enter()?;
    // A destructured parameter has a binding of its own under a name no script can write, which
    // holds its argument until the pattern takes it.
    let param_key = |c: &FnCompiler, i: usize, p: &Binding| match p {
        Binding::Name(n) => key_of(&n.name),
        Binding::Pattern(_) => c.hidden_param(i),
    };
    let params: Vec<Key> = f.params.iter().enumerate().map(|(i, p)| param_key(&c, i, p)).collect();
    let rest = f.rest.as_ref().map(|r| param_key(&c, f.params.len(), r));
    let pattern_names: Vec<Key> = f.params.iter().chain(&f.rest).filter(|p| matches!(p, Binding::Pattern(_))).flat_map(|p| c.bound_keys(p)).collect();
    let all_params: Vec<Key> = f.params.iter().chain(&f.rest).flat_map(|p| c.bound_keys(p)).collect();
    let hoisted = c.hoist(&f.body, &all_params)?;
    let top_scope = c.list_scope(&f.body, false)?;
    let param_init = !f.simple_params();
    if param_init {
        // Each parameter is initialized in order, from its argument or, when that is
        // `undefined`, its default value; then the body's environment is entered.
        let all = f.params.iter().enumerate().map(|(i, p)| (p, f.defaults.get(i).copied().flatten())).chain(f.rest.iter().map(|r| (r, None)));
        for (i, (p, default)) in all.enumerate() {
            let k = params.get(i).or(rest.as_ref()).cloned().unwrap_or_default();
            let n = c.name_key(k);
            c.at(match p {
                Binding::Name(name) => name.span.start,
                Binding::Pattern(e) => ast.expr_span(*e).start,
            });
            c.emit(Op::LoadParam(n));
            if let Some(default) = default {
                let skip = c.new_label();
                c.emit(Op::Dup);
                c.emit(Op::Undefined);
                c.emit(Op::Binary(BinaryOp::StrictEq));
                c.jump(Op::JumpIfFalse, skip);
                c.emit(Op::Pop);
                match p {
                    Binding::Name(name) => c.named_expr(default, &name.name)?,
                    Binding::Pattern(_) => c.expr(default)?,
                }
                c.bind(skip);
            }
            match p {
                Binding::Name(name) => {
                    c.at(name.span.start);
                    let n = c.var_name(&name.name);
                    c.emit(Op::InitVar(n));
                }
                Binding::Pattern(e) => c.destructure(*e, Mode::Lexical)?,
            }
        }
        c.emit(Op::BodyEnv);
    }
    c.stmts(&f.body)?;
    c.at(f.span.end.saturating_sub(1));
    c.emit(Op::Undefined);
    c.emit(Op::Return);
    let name = f.name.as_ref().map(|n| key_of(&n.name));
    let length = f.defaults.iter().position(Option::is_some).unwrap_or(params.len());
    let meta = Meta {
        decl_name: if f.kind == FunctionKind::Declaration { name.clone() } else { None },
        self_name: if bind_name && f.kind == FunctionKind::Expression { name } else { None },
        params,
        span: (f.span.start, f.span.end),
        top_scope,
        rest,
        length: len_u32(length),
        arrow: f.kind == FunctionKind::Arrow,
        param_init,
        pattern_names,
        own_name: es2015.then(|| key_of(f.name.as_ref().map_or("", |n| n.name.as_str()))),
        method: es2015 && matches!(f.kind, FunctionKind::Method | FunctionKind::Getter | FunctionKind::Setter),
    };
    c.finish(hoisted, meta)
}

/// Compiles a script (`kind` [`CodeKind::Global`]) or eval code ([`CodeKind::Eval`]).
pub(crate) fn compile_program(program: &crate::Program, source: &Rc<Source>, kind: CodeKind) -> CResult<Rc<Code>> {
    let mut c = FnCompiler::new(&program.ast, source, kind, program.strict, 0);
    c.es2015 = program.es2015;
    let hoisted = c.hoist(&program.body, &[])?;
    let top_scope = c.list_scope(&program.body, false)?;
    c.stmts(&program.body)?;
    c.emit(Op::ReturnCompletion);
    let end = u32::try_from(source.text.len()).unwrap_or(u32::MAX);
    c.finish(hoisted, Meta { span: (0, end), top_scope, ..Meta::default() })
}

/// Compiles a module's statements ([`crate::parse_module`]): strict code that runs in an
/// environment of its own, as strict eval code does, and returns `undefined`.
pub(crate) fn compile_module(program: &crate::Program, source: &Rc<Source>) -> CResult<Rc<Code>> {
    let mut c = FnCompiler::new(&program.ast, source, CodeKind::Eval, true, 0);
    c.es2015 = true;
    c.module = true;
    let hoisted = c.hoist(&program.body, &[])?;
    let top_scope = c.list_scope(&program.body, false)?;
    c.stmts(&program.body)?;
    c.emit(Op::Undefined);
    c.emit(Op::Return);
    let end = u32::try_from(source.text.len()).unwrap_or(u32::MAX);
    c.finish(hoisted, Meta { span: (0, end), top_scope, ..Meta::default() })
}

/// The function of a `Function` constructor's source, `(function anonymous(…) {…})`: the program
/// must be that one parenthesized function expression and nothing else. The name `anonymous` is
/// not bound inside the function (section 15.3.2.1).
pub(crate) fn compile_constructed_function(program: &crate::Program, source: &Rc<Source>) -> CResult<Option<Rc<Code>>> {
    let [stmt] = program.body.as_slice() else { return Ok(None) };
    let Some(Stmt::Expr(e)) = program.ast.stmt(*stmt) else { return Ok(None) };
    let Some(Expr::Paren(inner)) = program.ast.expr(*e) else { return Ok(None) };
    let Some(Expr::Function(fid)) = program.ast.expr(*inner) else { return Ok(None) };
    compile_function(&program.ast, *fid, source, 0, false, program.es2015).map(Some)
}
