//! `for (… of …)` and destructuring with [`crate::Options::es2015`] (ECMA-262 11th edition,
//! <https://262.ecma-international.org/11.0/>): binding patterns in declarations, parameters and
//! catch clauses (section 13.3.3), assignment patterns read from array and object literals
//! (section 12.15.5), and the shorthand, computed and `name = default` properties an object
//! literal takes while it may still turn out to be a pattern (section 12.2.6).
//!
//! A pattern is kept as the array or object expression it looks like (see [`Binding`]). A binding
//! pattern is read as one; an assignment pattern is read as a literal and checked when an `=` or a
//! loop head shows that it is a pattern.

use super::{Parser, Result, describe, property_key};
use crate::ast::{Binding, Expr, ExprId, ForInTarget, FunctionKind, Name, Property, PropertyKey, PropertyValue, Stmt, StmtId, VarDecl};
use crate::token::{Punct, Span, TokenKind};

impl Parser<'_> {
    /// The current token is `of` written without escapes, in a loop head.
    pub(super) fn at_of(&self) -> bool {
        self.es2015 && self.tok.ident() == Some("of") && !self.tok.escaped
    }

    /// A shorthand property with an initializer (`{a = 1}`) or a second `__proto__: value` read
    /// since `mark` that no pattern took is a `SyntaxError`.
    pub(super) fn check_cover(&self, mark: usize) -> Result<()> {
        match self.cover_inits.get(mark) {
            Some(&at) if self.proto_twice.contains(&at) => Err(self.error_at_offset(at, "an object literal must not set '__proto__' twice")),
            Some(&at) => Err(self.error_at_offset(at, "a shorthand property cannot have an initializer outside a destructuring pattern")),
            None => Ok(()),
        }
    }

    /// The name or pattern a `var`, `let`, `const` or catch clause declares.
    pub(super) fn parse_declaration_target(&mut self) -> Result<Binding> {
        if self.es2015 && (self.at(Punct::LBracket) || self.at(Punct::LBrace)) {
            return Ok(Binding::Pattern(self.parse_binding_pattern()?));
        }
        Ok(Binding::Name(self.binding_identifier()?))
    }

    /// A parameter's name or pattern; a name's strict-mode checks wait for the body.
    pub(super) fn parse_param_target(&mut self) -> Result<Binding> {
        if self.es2015 && (self.at(Punct::LBracket) || self.at(Punct::LBrace)) {
            return Ok(Binding::Pattern(self.parse_binding_pattern()?));
        }
        Ok(Binding::Name(self.function_name_or_param()?.0))
    }

    /// `[…]` or `{…}`: a binding pattern, each nested pattern counted as one more level of
    /// nesting.
    pub(super) fn parse_binding_pattern(&mut self) -> Result<ExprId> {
        self.enter()?;
        let result = if self.at(Punct::LBracket) { self.binding_array() } else { self.binding_object() };
        self.leave();
        result
    }

    /// `[a, , [b], c = 1, ...d]`: a comma with nothing before it skips a value, and a rest element
    /// comes last, without a comma after it.
    fn binding_array(&mut self) -> Result<ExprId> {
        let start = self.expect(Punct::LBracket)?.span;
        let mut elements = Vec::new();
        loop {
            if self.eat(Punct::RBracket)? {
                break;
            }
            if self.eat(Punct::Comma)? {
                elements.push(None);
                continue;
            }
            if self.at(Punct::Ellipsis) {
                let rest_start = self.bump()?.span;
                let target = self.binding_target_expr()?;
                elements.push(Some(self.ast.push_expr(Expr::Spread(target), self.span_from(rest_start))));
                if !self.at(Punct::RBracket) {
                    return Err(self.error_at_token(&self.tok, "a rest element must be the last element of a pattern"));
                }
                continue;
            }
            elements.push(Some(self.binding_element()?));
            if !self.at(Punct::RBracket) {
                self.expect(Punct::Comma)?;
            }
        }
        Ok(self.ast.push_expr(Expr::Array(elements), self.span_from(start)))
    }

    /// `{a, b = 1, c: d, "e": [f], [g]: h = 2}`.
    fn binding_object(&mut self) -> Result<ExprId> {
        let start = self.expect(Punct::LBrace)?.span;
        let mut properties = Vec::new();
        while !self.eat(Punct::RBrace)? {
            let property_start = self.tok.span;
            let shorthand = matches!(self.tok.kind, TokenKind::Ident(_)) && !self.peek().is_some_and(|t| t.is_punct(Punct::Colon));
            let (key, value) = if shorthand {
                let name = self.binding_identifier()?;
                let target = self.ast.push_expr(Expr::Ident(name.name.clone()), name.span);
                (PropertyKey::Ident(name.name), self.binding_default(target)?)
            } else {
                let key = if self.at(Punct::LBracket) { self.computed_key()? } else { self.parse_property_key()?.0 };
                self.expect(Punct::Colon)?;
                (key, self.binding_element()?)
            };
            properties.push(Property { key, value: PropertyValue::Init(value), span: self.span_from(property_start) });
            if !self.at(Punct::RBrace) {
                self.expect(Punct::Comma)?;
            }
        }
        Ok(self.ast.push_expr(Expr::Object(properties), self.span_from(start)))
    }

    /// A name or a nested pattern, as a pattern's target.
    pub(super) fn binding_target_expr(&mut self) -> Result<ExprId> {
        if self.es2015 && (self.at(Punct::LBracket) || self.at(Punct::LBrace)) {
            return self.parse_binding_pattern();
        }
        let name = self.binding_identifier()?;
        Ok(self.ast.push_expr(Expr::Ident(name.name), name.span))
    }

    /// A pattern's target with its optional default value.
    fn binding_element(&mut self) -> Result<ExprId> {
        let target = self.binding_target_expr()?;
        self.binding_default(target)
    }

    /// `= value` after a target, kept as an assignment whose target is the target.
    fn binding_default(&mut self, target: ExprId) -> Result<ExprId> {
        if !self.eat(Punct::Eq)? {
            return Ok(target);
        }
        let value = self.parse_assignment(false)?;
        let span = self.ast.expr_span(target).to(self.ast.expr_span(value));
        Ok(self.ast.push_expr(Expr::Assign { op: None, target, value }, span))
    }

    /// `[expression]` as a property name.
    fn computed_key(&mut self) -> Result<PropertyKey> {
        self.expect(Punct::LBracket)?;
        let key = self.parse_assignment(false)?;
        self.expect(Punct::RBracket)?;
        Ok(PropertyKey::Computed(key))
    }

    /// A property of an object literal with [`crate::Options::es2015`]: what ECMAScript 5.1 reads,
    /// a computed name (`[k]: v`, `get [k]() {}`), a shorthand name (`a`, the value of `a`), a
    /// method (`m() {}`, `[k]() {}`; ECMA-262 2015 section 14.3), and a shorthand name with an
    /// initializer (`a = 1`), which only a pattern may hold.
    pub(super) fn parse_property_es2015(&mut self) -> Result<Property> {
        let start = self.tok.span;
        // `*name(…) {…}`: a generator method (ECMA-262 2015 section 14.4).
        if self.eat(Punct::Star)? {
            let key = if self.at(Punct::LBracket) { self.computed_key()? } else { self.parse_property_key()?.0 };
            let function = self.parse_function_rest(FunctionKind::Method, None, start, true)?;
            return Ok(Property { key, value: PropertyValue::Method(function), span: self.span_from(start) });
        }
        // `get` and `set` start an accessor only when written without escapes and followed by a
        // property name.
        let accessor = match self.tok.ident().filter(|_| !self.tok.escaped) {
            Some("get") => Some(FunctionKind::Getter),
            Some("set") => Some(FunctionKind::Setter),
            _ => None,
        };
        if let Some(kind) = accessor
            && self.peek().is_some_and(|t| property_key(&t).is_some() || t.is_punct(Punct::LBracket))
        {
            self.bump()?;
            let key = if self.at(Punct::LBracket) { self.computed_key()? } else { self.parse_property_key()?.0 };
            let function = self.parse_function_rest(kind, None, start, false)?;
            let value = if kind == FunctionKind::Getter { PropertyValue::Get(function) } else { PropertyValue::Set(function) };
            return Ok(Property { key, value, span: self.span_from(start) });
        }
        if self.at(Punct::LBracket) {
            let key = self.computed_key()?;
            return self.cover_property_value(key, start);
        }
        if let TokenKind::Ident(name) = &self.tok.kind
            && self.peek().is_some_and(|t| t.is_punct(Punct::Comma) || t.is_punct(Punct::RBrace) || t.is_punct(Punct::Eq))
        {
            let name = name.clone();
            self.check_identifier(&self.tok, &name)?;
            let token = self.bump()?;
            let target = self.ast.push_expr(Expr::Ident(name.clone()), token.span);
            let value = if self.at(Punct::Eq) {
                self.cover_inits.push(self.tok.span.start);
                self.bump()?;
                let init = self.parse_assignment(false)?;
                let span = token.span.to(self.ast.expr_span(init));
                self.ast.push_expr(Expr::Assign { op: None, target, value: init }, span)
            } else {
                target
            };
            return Ok(Property { key: PropertyKey::Ident(name), value: PropertyValue::Init(value), span: self.span_from(start) });
        }
        let (key, _) = self.parse_property_key()?;
        self.cover_property_value(key, start)
    }

    /// A method's parameters and body after its name, or `: value` after a property name, where
    /// the value may still turn out to be a pattern.
    fn cover_property_value(&mut self, key: PropertyKey, start: Span) -> Result<Property> {
        if self.at(Punct::LParen) {
            let function = self.parse_function_rest(FunctionKind::Method, None, start, false)?;
            return Ok(Property { key, value: PropertyValue::Method(function), span: self.span_from(start) });
        }
        self.expect(Punct::Colon)?;
        self.cover_ok = true;
        let value = self.parse_assignment(false)?;
        Ok(Property { key, value: PropertyValue::Init(value), span: self.span_from(start) })
    }

    /// An assignment's, `++`'s or `--`'s target without the call that ECMAScript 5.1 code may
    /// assign to: an identifier (in strict code neither `eval` nor `arguments`), a property or an
    /// E4X name standing alone.
    fn check_simple_target(&self, expr: ExprId, message: &str) -> Result<()> {
        let inner = self.ast.unparenthesized(expr);
        match self.ast.expr(inner) {
            Some(Expr::Ident(name)) => {
                if self.ctx.strict && super::is_eval_or_arguments(name) {
                    return Err(self.error_at_offset(self.ast.expr_span(inner).start, format!("cannot assign to '{name}' in strict mode")));
                }
                Ok(())
            }
            Some(Expr::Member { .. } | Expr::Index { .. } | Expr::XmlScopeName(_)) => Ok(()),
            _ => Err(self.error_at_offset(self.ast.expr_span(expr).start, message)),
        }
    }

    /// A `for-in` or `for-of` loop's target expression: an assignment pattern, or a target that
    /// an assignment takes (without a call in a `for-of` loop).
    pub(super) fn loop_target(&mut self, expr: ExprId, message: &str, of: bool) -> Result<()> {
        if self.es2015 && matches!(self.ast.expr(expr), Some(Expr::Array(_) | Expr::Object(_))) {
            return self.check_assignment_pattern(expr);
        }
        if of { self.check_simple_target(expr, message) } else { self.check_target(expr, message) }
    }

    /// The targets of the elements and property values of the pattern `id`, each with its default
    /// value taken off. A rest element that is not last or has a comma after it, and a method, a
    /// getter or a setter, are errors.
    fn pattern_targets(&self, id: ExprId) -> Result<Vec<ExprId>> {
        let mut out = Vec::new();
        match self.ast.expr(id) {
            Some(Expr::Array(elements)) => {
                let count = elements.len();
                for (i, el) in elements.iter().enumerate() {
                    let Some(el) = *el else { continue };
                    if let Some(Expr::Spread(arg)) = self.ast.expr(el) {
                        if i + 1 != count || self.rest_commas.contains(&el.0) {
                            return Err(self.error_at_offset(self.ast.expr_span(el).start, "a rest element must be the last element of a pattern"));
                        }
                        out.push(*arg);
                    } else {
                        out.push(self.without_default(el));
                    }
                }
            }
            Some(Expr::Object(properties)) => {
                for p in properties {
                    match p.value {
                        PropertyValue::Init(v) => out.push(self.without_default(v)),
                        _ => return Err(self.error_at_offset(p.span.start, "a method, getter or setter cannot be a destructuring target")),
                    }
                }
            }
            _ => return Err(self.error_at_offset(self.ast.expr_span(id).start, "invalid destructuring pattern")),
        }
        Ok(out)
    }

    /// An element's target: the target of `target = default`, or the element itself.
    fn without_default(&self, el: ExprId) -> ExprId {
        match self.ast.expr(el) {
            Some(Expr::Assign { op: None, target, .. }) => *target,
            _ => el,
        }
    }

    fn is_pattern(&self, id: ExprId) -> bool {
        matches!(self.ast.expr(id), Some(Expr::Array(_) | Expr::Object(_)))
    }

    /// Checks that the array or object literal `pattern` is an assignment pattern (ECMA-262 11th
    /// edition, section 12.15.5.1): each target is a nested pattern or one that an assignment
    /// takes, a parenthesized pattern is not a target, and the shorthand properties with an
    /// initializer in it are no longer errors.
    pub(super) fn check_assignment_pattern(&mut self, pattern: ExprId) -> Result<()> {
        let mut work = vec![pattern];
        let mut gaps = Vec::new();
        while let Some(id) = work.pop() {
            for target in self.pattern_targets(id)? {
                if self.is_pattern(target) {
                    work.push(target);
                } else {
                    self.check_simple_target(target, "invalid destructuring target")?;
                }
            }
            self.cover_gaps(id, &mut gaps);
        }
        self.take_covers(pattern, gaps);
        Ok(())
    }

    /// Checks that the array or object literal `pattern`, read as an arrow function's parameter,
    /// is a binding pattern: each target is a name written bare or a nested pattern.
    fn check_binding_pattern(&mut self, pattern: ExprId) -> Result<()> {
        let mut work = vec![pattern];
        let mut gaps = Vec::new();
        while let Some(id) = work.pop() {
            for target in self.pattern_targets(id)? {
                match self.ast.expr(target) {
                    Some(Expr::Ident(_)) => {}
                    Some(Expr::Array(_) | Expr::Object(_)) => work.push(target),
                    _ => return Err(self.error_at_offset(self.ast.expr_span(target).start, "invalid arrow function parameters")),
                }
            }
            self.cover_gaps(id, &mut gaps);
        }
        self.take_covers(pattern, gaps);
        Ok(())
    }

    /// Adds to `gaps` where the `=` of each shorthand property with an initializer that the
    /// object pattern `id` holds itself lies: between the property's target and its default value.
    /// A pattern may also name `__proto__` more than once, so where each `__proto__: target`
    /// starts is added as well.
    fn cover_gaps(&self, id: ExprId, gaps: &mut Vec<(u32, u32)>) {
        let Some(Expr::Object(properties)) = self.ast.expr(id) else { return };
        for p in properties {
            if p.is_proto_setter(&self.ast) {
                gaps.push((p.span.start, p.span.start.saturating_add(1)));
            }
            if let PropertyValue::Init(v) = p.value
                && let Some(Expr::Assign { op: None, target, value }) = self.ast.expr(v)
            {
                gaps.push((self.ast.expr_span(*target).end, self.ast.expr_span(*value).start));
            }
        }
    }

    /// Forgets the shorthand properties with an initializer whose `=` lies in one of `gaps`, the
    /// gaps of the object patterns in `pattern`. One inside a default value or a computed name
    /// stays an error. The positions in `cover_inits` rise, so only those inside `pattern` are
    /// read, in one pass with the gaps sorted.
    fn take_covers(&mut self, pattern: ExprId, mut gaps: Vec<(u32, u32)>) {
        if gaps.is_empty() {
            return;
        }
        gaps.sort_unstable();
        let span = self.ast.expr_span(pattern);
        let lo = self.cover_inits.partition_point(|&at| at < span.start);
        let hi = self.cover_inits.partition_point(|&at| at < span.end).max(lo);
        let mut gaps = gaps.into_iter().peekable();
        let kept: Vec<u32> = self
            .cover_inits
            .get(lo..hi)
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|&at| {
                while gaps.next_if(|&(_, to)| to <= at).is_some() {}
                !gaps.peek().is_some_and(|&(from, to)| from <= at && at < to)
            })
            .collect();
        drop(self.cover_inits.splice(lo..hi, kept));
    }

    /// An arrow function's parameter from its parenthesized head: `None` when `item` is neither a
    /// name nor a pattern.
    pub(super) fn arrow_param(&mut self, item: ExprId) -> Option<Result<Binding>> {
        match self.ast.expr(item) {
            Some(Expr::Ident(name)) => Some(Ok(Binding::Name(Name { name: name.clone(), span: self.ast.expr_span(item) }))),
            Some(Expr::Array(_) | Expr::Object(_)) => Some(self.check_binding_pattern(item).map(|()| Binding::Pattern(item))),
            _ => None,
        }
    }

    /// `for (var … of …)`, from `of`: the variable takes no initializer, and no catch clause
    /// around the loop in the same function has a parameter of the same name (ECMA-262 2015
    /// Annex B.3.5).
    pub(super) fn parse_for_of_var(&mut self, decl: VarDecl, start: Span) -> Result<StmtId> {
        if decl.init.is_some() {
            return Err(self.error_at_token(&self.tok, "a for-of variable cannot have an initializer"));
        }
        for name in self.ast.bound_names(&decl.target) {
            for scope in self.scopes.iter().rev() {
                if !scope.function && scope.params.contains(&name.name) {
                    return Err(self.redeclared(&name));
                }
                if scope.function {
                    break;
                }
            }
        }
        self.parse_for_of(ForInTarget::Var(decl), start)
    }

    /// The rest of `for (target of iterable) body`, from `of`; the iterable is an assignment
    /// expression.
    pub(super) fn parse_for_of(&mut self, target: ForInTarget, start: Span) -> Result<StmtId> {
        if !self.at_of() {
            return Err(self.error_at_token(&self.tok, format!("expected 'of' but found {}", describe(&self.tok))));
        }
        self.bump()?;
        let iterable = self.parse_assignment(false)?;
        self.expect(Punct::RParen)?;
        let body = self.parse_loop_body()?;
        Ok(self.ast.push_stmt(Stmt::ForOf { target, iterable, body }, self.span_from(start)))
    }
}
