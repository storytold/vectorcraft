//! Destructuring and `for (… of …)` (ECMA-262 11th edition,
//! <https://262.ecma-international.org/11.0/>, sections 12.15.5, 13.3.3 and 13.7.5).
//!
//! A pattern takes the value on top of the stack. An array pattern steps the value's iterator
//! once for each element and closes it when the pattern ends with the iterator not done, and when
//! anything in the pattern throws (an exception handler around the elements). An object pattern
//! reads each property by name. A target that is not a pattern is written as an assignment
//! (`[a.b] = …`), a `var` or a `let`, `const`, parameter or catch binding, as [`Mode`] says.

use super::{CResult, Ctx, Exit, FnCompiler, internal, key_of, len_u32};
use crate::ast::{Binding, Expr, ExprId, ForInTarget, LexKind, Property, PropertyKey, PropertyValue, StmtId};
use crate::bytecode::{BlockScope, CodeKind, Op};
use crate::heap::Key;

/// How a pattern's targets are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// An assignment: each target is evaluated as an assignment's target is.
    Assign,
    /// `var`: the variable is assigned.
    Var,
    /// `let`, `const`, a parameter or a catch clause: the binding is initialized.
    Lexical,
}

impl FnCompiler<'_> {
    /// Whether `id`, without parentheses, is an array or object pattern.
    pub(super) fn is_pattern(&self, id: ExprId) -> bool {
        matches!(self.ast.expr(self.ast.unparenthesized(id)), Some(Expr::Array(_) | Expr::Object(_)))
    }

    /// The names a binding binds, as keys.
    pub(super) fn bound_keys(&self, binding: &Binding) -> Vec<Key> {
        self.ast.bound_names(binding).iter().map(|n| key_of(&n.name)).collect()
    }

    /// `value` → nothing: writes the value to the targets of the pattern `pattern`.
    pub(super) fn destructure(&mut self, pattern: ExprId, mode: Mode) -> CResult<()> {
        self.enter()?;
        let pattern = self.ast.unparenthesized(pattern);
        self.at(self.ast.expr_span(pattern).start);
        let r = match self.ast.expr(pattern) {
            Some(Expr::Array(elements)) => self.array_pattern(elements, mode),
            Some(Expr::Object(properties)) => self.object_pattern(properties, mode),
            _ => internal("not a destructuring pattern"),
        };
        self.leave();
        r
    }

    /// `value` → nothing: writes the value to a name binding or a pattern.
    pub(super) fn bind_to(&mut self, binding: &Binding, mode: Mode) -> CResult<()> {
        match binding {
            Binding::Pattern(p) => self.destructure(*p, mode),
            Binding::Name(name) => {
                self.at(name.span.start);
                let n = self.var_name(&name.name);
                if mode == Mode::Lexical {
                    self.emit(Op::InitVar(n));
                } else {
                    self.emit(Op::SetVar(n));
                    self.emit(Op::Pop);
                }
                Ok(())
            }
        }
    }

    /// An element's target and its default value.
    fn split_default(&self, element: ExprId) -> (ExprId, Option<ExprId>) {
        match self.ast.expr(element) {
            Some(Expr::Assign { op: None, target, value }) => (*target, Some(*value)),
            _ => (element, None),
        }
    }

    /// Pushes what writing to `target` needs before its value, and gives how many values that is:
    /// an assignment's object and key; a `var` name's reference, resolved before the value is
    /// read (sections 13.3.3.6 and 13.3.3.7); nothing for another binding or a pattern.
    fn target_refs(&mut self, target: ExprId, mode: Mode) -> CResult<u32> {
        if mode == Mode::Var
            && let Some(Expr::Ident(name)) = self.ast.expr(target)
        {
            self.at(self.ast.expr_span(target).start);
            let n = self.var_name(name);
            self.emit(Op::RefVar(n));
            return Ok(1);
        }
        if mode != Mode::Assign || self.is_pattern(target) {
            return Ok(0);
        }
        let before = self.depth;
        self.assign_target(None, self.ast.unparenthesized(target))?;
        Ok(self.depth.saturating_sub(before))
    }

    /// `refs… value` → nothing: writes the value to `target`.
    fn write_target(&mut self, target: ExprId, mode: Mode) -> CResult<()> {
        let target = self.ast.unparenthesized(target);
        if self.is_pattern(target) {
            return self.destructure(target, mode);
        }
        self.at(self.ast.expr_span(target).start);
        match (mode, self.ast.expr(target)) {
            (Mode::Assign, _) => {
                self.store(target)?;
                self.emit(Op::Pop);
            }
            (_, Some(Expr::Ident(name))) => {
                let n = self.var_name(name);
                if mode == Mode::Lexical {
                    self.emit(Op::InitVar(n));
                } else {
                    self.emit(Op::PutRef(n));
                    self.emit(Op::Pop);
                }
            }
            _ => return internal("a binding pattern's target is not a name"),
        }
        Ok(())
    }

    /// `value` → `value`, or the default value when the value is `undefined`. An anonymous
    /// function as the default value of a plain name gets that name (ECMA-262 2015 sections
    /// 12.14.5.3, 12.14.5.4, 13.3.3.6 and 13.3.3.7).
    fn default_value(&mut self, target: ExprId, default: Option<ExprId>) -> CResult<()> {
        let Some(default) = default else { return Ok(()) };
        let skip = self.new_label();
        self.emit(Op::Dup);
        self.emit(Op::Undefined);
        self.emit(Op::Binary(crate::ast::BinaryOp::StrictEq));
        self.jump(Op::JumpIfFalse, skip);
        self.emit(Op::Pop);
        match self.ast.expr(target) {
            Some(Expr::Ident(name)) => self.named_expr(default, name)?,
            _ => self.expr(default)?,
        }
        self.bind(skip);
        Ok(())
    }

    /// `value` → nothing, for `[a, , b = 1, ...c]` (sections 12.15.5.5 and 13.3.3.6).
    fn array_pattern(&mut self, elements: &[Option<ExprId>], mode: Mode) -> CResult<()> {
        self.emit(Op::GetIterator);
        let record = self.depth;
        let start = self.pc();
        for element in elements {
            let Some(element) = *element else {
                self.emit(Op::IterSkip);
                continue;
            };
            if let Some(Expr::Spread(target)) = self.ast.expr(element) {
                let under = self.target_refs(*target, mode)?;
                self.emit(Op::IterRest(under));
                self.write_target(*target, mode)?;
                continue;
            }
            let (target, default) = self.split_default(element);
            let under = self.target_refs(target, mode)?;
            self.emit(Op::IterValue(under));
            self.default_value(target, default)?;
            self.write_target(target, mode)?;
        }
        let end = self.pc();
        let close_throw = self.new_label();
        let after = self.new_label();
        self.emit(Op::IterClose);
        self.jump(Op::Jump, after);
        self.handlers.push((start, end, close_throw, record, self.env_depth));
        self.bind(close_throw);
        self.depth = record + 1;
        self.emit(Op::IterCloseThrow);
        self.bind(after);
        self.depth = record.saturating_sub(1);
        Ok(())
    }

    /// `value` → nothing, for `{a, b: c, [d]: e = 1}` (sections 12.15.5.4 and 13.3.3.5).
    fn object_pattern(&mut self, properties: &[Property], mode: Mode) -> CResult<()> {
        self.emit(Op::RequireObjectCoercible);
        for p in properties {
            let PropertyValue::Init(value) = p.value else { return internal("a method, getter or setter in a pattern") };
            self.at(p.span.start);
            match &p.key {
                PropertyKey::Computed(e) => {
                    self.expr(*e)?;
                    self.emit(Op::ToKey);
                }
                key => {
                    let i = self.string(&key.to_utf16());
                    self.emit(Op::String(i));
                }
            }
            let (target, default) = self.split_default(value);
            let under = self.target_refs(target, mode)?;
            self.emit(Op::PatternGet(under));
            self.default_value(target, default)?;
            self.write_target(target, mode)?;
            self.emit(Op::Pop);
        }
        self.emit(Op::Pop);
        Ok(())
    }

    /// `for (target of iterable) body` (sections 13.7.5.12 and 13.7.5.13): the iterable is read
    /// where a `let` or `const` head's names are not yet initialized; each step's value goes to
    /// the target, in a new environment for a `let` or `const` head; the iterator is closed when
    /// the loop is left early, by `break`, `return`, a jump to an outer loop or an exception.
    pub(super) fn for_of(&mut self, target: &ForInTarget, iterable: ExprId, body: StmtId, labels: Vec<String>) -> CResult<()> {
        let scope = match target {
            ForInTarget::Lexical(kind, binding) => {
                let constant = *kind == LexKind::Const;
                let lets = self.bound_keys(binding).into_iter().map(|k| (k, constant)).collect();
                Some(self.push_scope(BlockScope { lets, funcs: Vec::new() }))
            }
            _ => None,
        };
        match scope {
            Some(s) => {
                self.emit(Op::PushBlock(s));
                self.env_depth += 1;
                self.expr(iterable)?;
                self.emit(Op::PopEnv);
                self.env_depth -= 1;
            }
            None => self.expr(iterable)?,
        }
        // The loop's completion value starts as `undefined` (ForIn/OfBodyEvaluation, step 2).
        if self.kind != CodeKind::Function {
            self.emit(Op::Undefined);
            self.emit(Op::SetCompletion);
        }
        self.at(self.ast.expr_span(iterable).start);
        self.emit(Op::GetIterator);
        let record = self.depth;
        let outer_env = self.env_depth;
        let head = self.new_label();
        let brk = self.new_label();
        let done = self.new_label();
        let close_throw = self.new_label();
        let after = self.new_label();
        self.contexts.push(Ctx::Iterator { depth: record, env_depth: outer_env, exits: Vec::new() });
        self.bind(head);
        self.jump(Op::IterStep, done);
        let start = self.pc();
        if let Some(s) = scope {
            self.emit(Op::PushBlock(s));
            self.env_depth += 1;
        }
        let r = self.for_of_target(target);
        let r = r.and_then(|()| {
            self.push_jump_ctx(labels, true, true, brk, Some(head));
            if let Some(Ctx::Jump { env_depth, .. }) = self.contexts.last_mut() {
                *env_depth = outer_env;
            }
            let r = self.stmt(body);
            self.contexts.pop();
            r
        });
        let exits = match self.contexts.pop() {
            Some(Ctx::Iterator { exits, .. }) => exits,
            _ => return internal("for-of context missing"),
        };
        r?;
        if scope.is_some() {
            self.emit(Op::PopEnv);
            self.env_depth -= 1;
        }
        let end = self.pc();
        self.jump(Op::Jump, head);
        self.handlers.push((start, end, close_throw, record, outer_env));
        self.bind(close_throw);
        self.depth = record + 1;
        self.emit(Op::IterCloseThrow);
        self.bind(brk);
        self.depth = record;
        self.emit(Op::IterClose);
        self.jump(Op::Jump, after);
        // `return` and jumps to outer statements close the iterator here, outside the range the
        // loop's handler covers, so an exception from `return` goes to the handlers around the
        // loop; the jump then goes on through them.
        for (exit, label) in exits {
            self.bind(label);
            self.depth = if exit == Exit::Return { record + 1 } else { record };
            self.env_depth = outer_env;
            self.emit(Op::IterClose);
            self.emit_exit(exit)?;
        }
        self.depth = record;
        self.env_depth = outer_env;
        self.bind(done);
        self.depth = record;
        self.emit(Op::Pop);
        self.bind(after);
        Ok(())
    }

    /// `value` → nothing: a `for-of` loop's step value written to its target.
    fn for_of_target(&mut self, target: &ForInTarget) -> CResult<()> {
        match target {
            ForInTarget::Var(d) => self.bind_to(&d.target, Mode::Var),
            ForInTarget::Lexical(_, binding) => self.bind_to(binding, Mode::Lexical),
            ForInTarget::Expr(e) if self.is_pattern(*e) => self.destructure(*e, Mode::Assign),
            ForInTarget::Expr(e) => {
                self.store_key_to(*e)?;
                self.emit(Op::Pop);
                Ok(())
            }
        }
    }

    /// The key of a destructured parameter's own binding, which no script can name.
    pub(super) fn hidden_param(&self, i: usize) -> Key {
        key_of(&format!("\u{0}p{}", len_u32(i)))
    }
}
