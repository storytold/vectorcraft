//! The parser: tokens into a [`Program`] (ECMA-262 5.1 sections 11 to 14), with the early errors
//! of section 16 and of strict mode (Annex C).
//!
//! It is a recursive-descent parser whose recursion is counted: each statement, assignment
//! expression, primary expression and function body it enters adds one to a depth, and going past
//! [`crate::Options::max_depth`] gives a `SyntaxError`. Binary operators are read with an operator
//! stack, and prefix operators, assignment chains, `else if` chains and labels with loops, so long
//! runs of them do not nest at all.

use crate::LineTable;
use crate::ast::{
    AssignOp, Ast, BinaryOp, CatchClause, Expr, ExprId, ForInTarget, ForInit, Function, FunctionKind, LogicalOp, Name, Program, Property,
    PropertyKey, PropertyValue, Stmt, StmtId, SwitchCase, UnaryOp, UpdateOp, VarDecl,
};
use crate::error::{SyntaxError, TOO_DEEP};
use crate::lexer::{Goal, Lexer};
use crate::token::{Directive, Keyword, Punct, Span, Token, TokenKind, strict_reserved};

type Result<T> = std::result::Result<T, SyntaxError>;

/// A label in scope, and whether it labels an iteration statement (`continue` may name only
/// those).
#[derive(Clone, Debug)]
struct Label {
    name: String,
    is_loop: bool,
}

/// What the code being parsed allows, saved and replaced at each function body.
#[derive(Clone, Debug, Default)]
struct Context {
    strict: bool,
    in_function: bool,
    labels: Vec<Label>,
    /// Iteration statements open in the current function.
    loops: u32,
    /// Iteration and `switch` statements open in the current function.
    breakables: u32,
}

/// A binary operator, with the logical ones apart.
#[derive(Clone, Copy, Debug)]
enum BinKind {
    Binary(BinaryOp),
    Logical(LogicalOp),
}

/// A prefix operator.
#[derive(Clone, Copy, Debug)]
enum Prefix {
    Unary(UnaryOp),
    Update(UpdateOp),
}

pub(crate) struct Parser<'s> {
    src: &'s [u16],
    lex: Lexer<'s>,
    lines: LineTable,
    tok: Token,
    /// Where the last token read ends.
    prev_end: u32,
    ast: Ast,
    depth: usize,
    max_depth: usize,
    ctx: Context,
    directives: Vec<Directive>,
    /// The tokens read, kept for [`crate::tokenize`].
    tokens: Option<Vec<Token>>,
    /// The next statement may be a function declaration outside strict mode code: it is an `if`
    /// branch or the body of a label in a statement list (ECMA-262 2015 Annex B.3.2 and B.3.4).
    /// Read and cleared as each statement starts.
    function_allowed: bool,
    /// The next statement stands in a statement list, where a label's body may be a function
    /// declaration. Read and cleared as each statement starts.
    in_list: bool,
}

impl<'s> Parser<'s> {
    pub(crate) fn new(src: &'s [u16], options: &crate::Options, keep_tokens: bool) -> Result<Self> {
        let lex = Lexer::new(src, options.preprocessor_lines, options.max_depth);
        let placeholder =
            Token { kind: TokenKind::Eof, span: Span::default(), line: 1, column: 1, newline_before: false, escaped: false, legacy_octal: false };
        let mut parser = Self {
            src,
            lex,
            lines: LineTable::new(src),
            tok: placeholder,
            prev_end: 0,
            ast: Ast::default(),
            depth: 0,
            max_depth: options.max_depth,
            ctx: Context { strict: options.strict, ..Context::default() },
            directives: Vec::new(),
            tokens: keep_tokens.then(Vec::new),
            function_allowed: false,
            in_list: false,
        };
        parser.tok = parser.next_significant()?;
        Ok(parser)
    }

    pub(crate) fn parse_program(mut self) -> Result<(Program, Vec<Token>)> {
        let body = self.parse_body(false)?;
        if self.tok.kind != TokenKind::Eof {
            return Err(self.unexpected());
        }
        if let Some(tokens) = &mut self.tokens {
            tokens.push(self.tok.clone());
        }
        let program = Program { ast: self.ast, body, strict: self.ctx.strict, directives: self.directives, lines: self.lines };
        Ok((program, self.tokens.unwrap_or_default()))
    }

    // ---- tokens ----

    /// The next token that is not a preprocessor line; preprocessor lines are kept.
    fn next_significant(&mut self) -> Result<Token> {
        loop {
            let token = self.lex.next_token(Goal::Div)?;
            if let TokenKind::Directive(directive) = &token.kind {
                self.directives.push(directive.clone());
                if let Some(tokens) = &mut self.tokens {
                    tokens.push(token);
                }
                continue;
            }
            return Ok(token);
        }
    }

    /// Reads the current token and moves to the next.
    fn bump(&mut self) -> Result<Token> {
        // Kept before reading on, so a preprocessor line after the token comes after it.
        if let Some(tokens) = &mut self.tokens {
            tokens.push(self.tok.clone());
        }
        let next = self.next_significant()?;
        let token = std::mem::replace(&mut self.tok, next);
        self.prev_end = token.span.end;
        Ok(token)
    }

    /// The token after the current one, without reading it. `None` when the lexer cannot read one
    /// (reading it for real reports the error).
    fn peek(&self) -> Option<Token> {
        let mut lex = self.lex.clone();
        loop {
            match lex.next_token(Goal::Div) {
                Ok(Token { kind: TokenKind::Directive(_), .. }) => {}
                Ok(token) => return Some(token),
                Err(_) => return None,
            }
        }
    }

    fn at(&self, p: Punct) -> bool {
        self.tok.is_punct(p)
    }

    fn at_keyword(&self, k: Keyword) -> bool {
        self.tok.is_keyword(k)
    }

    fn eat(&mut self, p: Punct) -> Result<bool> {
        if self.at(p) {
            self.bump()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn expect(&mut self, p: Punct) -> Result<Token> {
        if self.at(p) {
            return self.bump();
        }
        Err(self.error_at_token(&self.tok, format!("expected '{}' but found {}", p.as_str(), describe(&self.tok))))
    }

    fn expect_keyword(&mut self, k: Keyword) -> Result<Token> {
        if self.at_keyword(k) {
            return self.bump();
        }
        Err(self.error_at_token(&self.tok, format!("expected '{}' but found {}", k.as_str(), describe(&self.tok))))
    }

    /// Ends a statement: a `;`, or one inserted before `}`, the end of the source or a token on a
    /// new line (section 7.9).
    fn semicolon(&mut self) -> Result<()> {
        if self.eat(Punct::Semicolon)? || self.at(Punct::RBrace) || self.tok.kind == TokenKind::Eof || self.tok.newline_before {
            return Ok(());
        }
        Err(self.unexpected())
    }

    // ---- errors ----

    fn error_at_offset(&self, offset: u32, message: impl Into<String>) -> SyntaxError {
        let (line, column) = self.lines.line_column(offset);
        SyntaxError { message: message.into(), line, column, offset }
    }

    fn error_at_token(&self, token: &Token, message: impl Into<String>) -> SyntaxError {
        SyntaxError { message: message.into(), line: token.line, column: token.column, offset: token.span.start }
    }

    fn unexpected(&self) -> SyntaxError {
        self.error_at_token(&self.tok, format!("unexpected {}", describe(&self.tok)))
    }

    /// Counts one more level of nesting; past the limit, a `SyntaxError` at the current token.
    fn enter(&mut self) -> Result<()> {
        if self.depth >= self.max_depth {
            return Err(self.error_at_token(&self.tok, TOO_DEEP));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn span_from(&self, start: Span) -> Span {
        Span::new(start.start, self.prev_end.max(start.start))
    }

    // ---- names ----

    /// Checks a name used as an identifier: not a reserved word written with escapes, and not a
    /// strict-mode reserved word in strict code.
    fn check_identifier(&self, token: &Token, name: &str) -> Result<()> {
        if token.escaped && Keyword::from_word(name).is_some() {
            return Err(self.error_at_token(token, format!("keyword '{name}' must not contain escaped characters")));
        }
        if self.ctx.strict && strict_reserved(name) {
            return Err(self.error_at_token(token, format!("'{name}' is a reserved word in strict mode")));
        }
        Ok(())
    }

    /// Reads an identifier that a declaration binds: checked as [`Self::check_identifier`] does,
    /// and in strict code neither `eval` nor `arguments`.
    fn binding_identifier(&mut self) -> Result<Name> {
        let TokenKind::Ident(name) = &self.tok.kind else {
            return Err(self.error_at_token(&self.tok, format!("expected an identifier but found {}", describe(&self.tok))));
        };
        let name = name.clone();
        self.check_identifier(&self.tok, &name)?;
        if self.ctx.strict && is_eval_or_arguments(&name) {
            return Err(self.error_at_token(&self.tok, format!("'{name}' cannot be declared in strict mode")));
        }
        let token = self.bump()?;
        Ok(Name { name, span: token.span })
    }

    // ---- program and function bodies ----

    /// Reads source elements up to `}` (in a function body) or the end of the source, reading the
    /// directive prologue at their start (section 14.1). A `"use strict"` directive makes the rest
    /// strict mode code.
    fn parse_body(&mut self, in_function: bool) -> Result<Vec<StmtId>> {
        let mut body = Vec::new();
        let mut prologue = true;
        // A string with an octal escape before a `"use strict"` directive in the same prologue.
        let mut octal: Option<Token> = None;
        while !(self.tok.kind == TokenKind::Eof || (in_function && self.at(Punct::RBrace))) {
            if prologue && matches!(self.tok.kind, TokenKind::String(_)) {
                let first = self.tok.clone();
                let stmt = self.parse_statement_item()?;
                body.push(stmt);
                if !self.is_directive(stmt, &first) {
                    prologue = false;
                    continue;
                }
                if self.is_use_strict(&first) {
                    self.ctx.strict = true;
                    if let Some(octal) = &octal {
                        return Err(self.error_at_token(octal, "octal escape sequences are not allowed in strict mode"));
                    }
                }
                if first.legacy_octal && octal.is_none() {
                    octal = Some(first);
                }
                continue;
            }
            prologue = false;
            body.push(self.parse_statement_item()?);
        }
        Ok(body)
    }

    /// Whether `stmt`, which started at `first`, is a directive: an expression statement made of
    /// that one string literal.
    fn is_directive(&self, stmt: StmtId, first: &Token) -> bool {
        match self.ast.stmt(stmt) {
            Some(Stmt::Expr(e)) => matches!(self.ast.expr(*e), Some(Expr::String(_))) && self.ast.expr_span(*e) == first.span,
            _ => false,
        }
    }

    /// The string token is exactly `"use strict"` or `'use strict'`, without escapes or line
    /// continuations.
    fn is_use_strict(&self, token: &Token) -> bool {
        let raw = self.src.get(token.span.start as usize..token.span.end as usize).unwrap_or_default();
        raw.len() == 12 && raw.get(1..11).is_some_and(|inner| inner.iter().copied().eq("use strict".encode_utf16()))
    }

    // ---- statements ----

    /// A source element: a function declaration or a statement.
    fn parse_statement_item(&mut self) -> Result<StmtId> {
        if self.at_keyword(Keyword::Function) {
            self.enter()?;
            let result = self.parse_function_declaration();
            self.leave();
            return result;
        }
        self.in_list = true;
        self.parse_statement()
    }

    fn parse_statement(&mut self) -> Result<StmtId> {
        self.enter()?;
        let result = self.statement();
        self.leave();
        result
    }

    /// A statement where Annex B lets sloppy code put a function declaration: an `if` branch.
    fn parse_branch(&mut self) -> Result<StmtId> {
        self.function_allowed = !self.ctx.strict;
        self.parse_statement()
    }

    fn statement(&mut self) -> Result<StmtId> {
        let start = self.tok.span;
        let function_allowed = std::mem::take(&mut self.function_allowed);
        let in_list = std::mem::take(&mut self.in_list);
        match &self.tok.kind {
            TokenKind::Keyword(Keyword::Function) => {
                if !function_allowed {
                    let message = if self.ctx.strict {
                        "in strict mode code, functions can only be declared at top level or inside a block"
                    } else {
                        "a function declaration cannot stand here; put it in a block"
                    };
                    return Err(self.error_at_token(&self.tok, message));
                }
                self.parse_function_declaration()
            }
            TokenKind::Ident(_) if self.peek().is_some_and(|t| t.is_punct(Punct::Colon)) => self.parse_labeled(in_list),
            _ => self.plain_statement(start),
        }
    }

    fn plain_statement(&mut self, start: Span) -> Result<StmtId> {
        match &self.tok.kind {
            TokenKind::Punct(Punct::LBrace) => {
                let body = self.parse_block()?;
                Ok(self.ast.push_stmt(Stmt::Block(body), self.span_from(start)))
            }
            TokenKind::Punct(Punct::Semicolon) => {
                self.bump()?;
                Ok(self.ast.push_stmt(Stmt::Empty, start))
            }
            TokenKind::Keyword(k) => self.keyword_statement(*k, start),
            _ => {
                let expr = self.parse_expression(false)?;
                self.semicolon()?;
                Ok(self.ast.push_stmt(Stmt::Expr(expr), self.span_from(start)))
            }
        }
    }

    fn keyword_statement(&mut self, k: Keyword, start: Span) -> Result<StmtId> {
        match k {
            Keyword::Var => {
                self.bump()?;
                let decls = self.parse_var_decls(false)?;
                self.semicolon()?;
                Ok(self.ast.push_stmt(Stmt::Var(decls), self.span_from(start)))
            }
            Keyword::If => self.parse_if(),
            Keyword::Do => self.parse_do_while(start),
            Keyword::While => self.parse_while(start),
            Keyword::For => self.parse_for(start),
            Keyword::Continue | Keyword::Break => self.parse_jump(k == Keyword::Break, start),
            Keyword::Return => self.parse_return(start),
            Keyword::With => self.parse_with(start),
            Keyword::Switch => self.parse_switch(start),
            Keyword::Throw => self.parse_throw(start),
            Keyword::Try => self.parse_try(start),
            Keyword::Debugger => {
                self.bump()?;
                self.semicolon()?;
                Ok(self.ast.push_stmt(Stmt::Debugger, self.span_from(start)))
            }
            _ => {
                let expr = self.parse_expression(false)?;
                self.semicolon()?;
                Ok(self.ast.push_stmt(Stmt::Expr(expr), self.span_from(start)))
            }
        }
    }

    /// `{ … }`: its statements.
    fn parse_block(&mut self) -> Result<Vec<StmtId>> {
        self.expect(Punct::LBrace)?;
        let mut body = Vec::new();
        while !self.at(Punct::RBrace) {
            if self.tok.kind == TokenKind::Eof {
                return Err(self.unexpected());
            }
            body.push(self.parse_statement_item()?);
        }
        self.bump()?;
        Ok(body)
    }

    /// `var` declarations after the `var`. `no_in` leaves `in` to a `for (… in …)` around them.
    fn parse_var_decls(&mut self, no_in: bool) -> Result<Vec<VarDecl>> {
        let mut decls = Vec::new();
        loop {
            let name = self.binding_identifier()?;
            let init = if self.eat(Punct::Eq)? { Some(self.parse_assignment(no_in)?) } else { None };
            decls.push(VarDecl { name, init });
            if !self.eat(Punct::Comma)? {
                return Ok(decls);
            }
        }
    }

    /// `if (…) … else if (…) … else …`, the `else if` chain read with a loop.
    fn parse_if(&mut self) -> Result<StmtId> {
        let mut chain = Vec::new();
        let alternate = loop {
            let start = self.expect_keyword(Keyword::If)?.span;
            self.expect(Punct::LParen)?;
            let test = self.parse_expression(false)?;
            self.expect(Punct::RParen)?;
            let consequent = self.parse_branch()?;
            chain.push((start, test, consequent));
            if !self.at_keyword(Keyword::Else) {
                break None;
            }
            self.bump()?;
            if !self.at_keyword(Keyword::If) {
                break Some(self.parse_branch()?);
            }
        };
        let mut alternate = alternate;
        let mut last = None;
        for (start, test, consequent) in chain.into_iter().rev() {
            let id = self.ast.push_stmt(Stmt::If { test, consequent, alternate }, self.span_from(start));
            alternate = Some(id);
            last = Some(id);
        }
        last.ok_or_else(|| self.unexpected())
    }

    /// The body of an iteration statement, with `break` and `continue` allowed in it.
    fn parse_loop_body(&mut self) -> Result<StmtId> {
        self.ctx.loops += 1;
        self.ctx.breakables += 1;
        let body = self.parse_statement();
        self.ctx.loops -= 1;
        self.ctx.breakables -= 1;
        body
    }

    fn parse_do_while(&mut self, start: Span) -> Result<StmtId> {
        self.bump()?;
        let body = self.parse_loop_body()?;
        self.expect_keyword(Keyword::While)?;
        self.expect(Punct::LParen)?;
        let test = self.parse_expression(false)?;
        self.expect(Punct::RParen)?;
        // A `;` after `do … while (…)` may be left out even on the same line, as ECMA-262 2015
        // section 11.9.1 allows.
        self.eat(Punct::Semicolon)?;
        Ok(self.ast.push_stmt(Stmt::DoWhile { body, test }, self.span_from(start)))
    }

    fn parse_while(&mut self, start: Span) -> Result<StmtId> {
        self.bump()?;
        self.expect(Punct::LParen)?;
        let test = self.parse_expression(false)?;
        self.expect(Punct::RParen)?;
        let body = self.parse_loop_body()?;
        Ok(self.ast.push_stmt(Stmt::While { test, body }, self.span_from(start)))
    }

    fn parse_for(&mut self, start: Span) -> Result<StmtId> {
        self.bump()?;
        self.expect(Punct::LParen)?;
        let init = if self.at(Punct::Semicolon) {
            None
        } else if self.at_keyword(Keyword::Var) {
            self.bump()?;
            let mut decls = self.parse_var_decls(true)?;
            if decls.len() == 1 && self.at_keyword(Keyword::In) {
                let decl = decls.pop().ok_or_else(|| self.unexpected())?;
                return self.parse_for_in(ForInTarget::Var(decl), start);
            }
            Some(ForInit::Var(decls))
        } else {
            let expr = self.parse_expression(true)?;
            if self.at_keyword(Keyword::In) {
                self.check_target(expr, "invalid left-hand side in for-in loop")?;
                return self.parse_for_in(ForInTarget::Expr(expr), start);
            }
            Some(ForInit::Expr(expr))
        };
        self.expect(Punct::Semicolon)?;
        let test = if self.at(Punct::Semicolon) { None } else { Some(self.parse_expression(false)?) };
        self.expect(Punct::Semicolon)?;
        let update = if self.at(Punct::RParen) { None } else { Some(self.parse_expression(false)?) };
        self.expect(Punct::RParen)?;
        let body = self.parse_loop_body()?;
        Ok(self.ast.push_stmt(Stmt::For { init, test, update, body }, self.span_from(start)))
    }

    /// The rest of `for (target in object) body`, from `in`.
    fn parse_for_in(&mut self, target: ForInTarget, start: Span) -> Result<StmtId> {
        self.expect_keyword(Keyword::In)?;
        let object = self.parse_expression(false)?;
        self.expect(Punct::RParen)?;
        let body = self.parse_loop_body()?;
        Ok(self.ast.push_stmt(Stmt::ForIn { target, object, body }, self.span_from(start)))
    }

    /// `break` or `continue`, with an optional label on the same line, which must name a label
    /// around it in the same function (an iteration statement's, for `continue`).
    fn parse_jump(&mut self, is_break: bool, start: Span) -> Result<StmtId> {
        let keyword = self.bump()?;
        let label = match &self.tok.kind {
            TokenKind::Ident(name) if !self.tok.newline_before => {
                let name = name.clone();
                self.check_identifier(&self.tok, &name)?;
                let token = self.bump()?;
                let found = self.ctx.labels.iter().rev().find(|l| l.name == name);
                match found {
                    None => return Err(self.error_at_token(&token, format!("undefined label '{name}'"))),
                    Some(l) if !is_break && !l.is_loop => {
                        return Err(self.error_at_token(&token, format!("label '{name}' does not name a loop, so continue cannot use it")));
                    }
                    Some(_) => {}
                }
                Some(Name { name, span: token.span })
            }
            _ => None,
        };
        if label.is_none() {
            if is_break && self.ctx.breakables == 0 {
                return Err(self.error_at_token(&keyword, "break outside a loop or switch"));
            }
            if !is_break && self.ctx.loops == 0 {
                return Err(self.error_at_token(&keyword, "continue outside a loop"));
            }
        }
        self.semicolon()?;
        let stmt = if is_break { Stmt::Break(label) } else { Stmt::Continue(label) };
        Ok(self.ast.push_stmt(stmt, self.span_from(start)))
    }

    fn parse_return(&mut self, start: Span) -> Result<StmtId> {
        let keyword = self.bump()?;
        if !self.ctx.in_function {
            return Err(self.error_at_token(&keyword, "return outside a function"));
        }
        let ends = self.at(Punct::Semicolon) || self.at(Punct::RBrace) || self.tok.kind == TokenKind::Eof || self.tok.newline_before;
        let arg = if ends { None } else { Some(self.parse_expression(false)?) };
        self.semicolon()?;
        Ok(self.ast.push_stmt(Stmt::Return(arg), self.span_from(start)))
    }

    fn parse_with(&mut self, start: Span) -> Result<StmtId> {
        if self.ctx.strict {
            return Err(self.error_at_token(&self.tok, "with statements are not allowed in strict mode"));
        }
        self.bump()?;
        self.expect(Punct::LParen)?;
        let object = self.parse_expression(false)?;
        self.expect(Punct::RParen)?;
        let body = self.parse_statement()?;
        Ok(self.ast.push_stmt(Stmt::With { object, body }, self.span_from(start)))
    }

    fn parse_switch(&mut self, start: Span) -> Result<StmtId> {
        self.bump()?;
        self.expect(Punct::LParen)?;
        let discriminant = self.parse_expression(false)?;
        self.expect(Punct::RParen)?;
        self.expect(Punct::LBrace)?;
        self.ctx.breakables += 1;
        let cases = self.parse_cases();
        self.ctx.breakables -= 1;
        let cases = cases?;
        Ok(self.ast.push_stmt(Stmt::Switch { discriminant, cases }, self.span_from(start)))
    }

    /// A `switch` statement's clauses, up to and with its `}`; at most one `default`.
    fn parse_cases(&mut self) -> Result<Vec<SwitchCase>> {
        let mut cases = Vec::new();
        let mut has_default = false;
        while !self.eat(Punct::RBrace)? {
            let case_start = self.tok.span;
            let test = if self.at_keyword(Keyword::Case) {
                self.bump()?;
                Some(self.parse_expression(false)?)
            } else if self.at_keyword(Keyword::Default) {
                if has_default {
                    return Err(self.error_at_token(&self.tok, "more than one default clause in switch statement"));
                }
                has_default = true;
                self.bump()?;
                None
            } else {
                return Err(self.unexpected());
            };
            self.expect(Punct::Colon)?;
            let mut body = Vec::new();
            while !(self.at_keyword(Keyword::Case) || self.at_keyword(Keyword::Default) || self.at(Punct::RBrace)) {
                if self.tok.kind == TokenKind::Eof {
                    return Err(self.unexpected());
                }
                body.push(self.parse_statement_item()?);
            }
            cases.push(SwitchCase { test, body, span: self.span_from(case_start) });
        }
        Ok(cases)
    }

    fn parse_throw(&mut self, start: Span) -> Result<StmtId> {
        self.bump()?;
        if self.tok.newline_before {
            return Err(self.error_at_token(&self.tok, "line break after throw"));
        }
        let arg = self.parse_expression(false)?;
        self.semicolon()?;
        Ok(self.ast.push_stmt(Stmt::Throw(arg), self.span_from(start)))
    }

    fn parse_try(&mut self, start: Span) -> Result<StmtId> {
        self.bump()?;
        let block = self.parse_block()?;
        let handler = if self.at_keyword(Keyword::Catch) {
            self.bump()?;
            self.expect(Punct::LParen)?;
            let param = self.binding_identifier()?;
            self.expect(Punct::RParen)?;
            Some(CatchClause { param, body: self.parse_block()? })
        } else {
            None
        };
        let finalizer = if self.at_keyword(Keyword::Finally) {
            self.bump()?;
            Some(self.parse_block()?)
        } else {
            None
        };
        if handler.is_none() && finalizer.is_none() {
            return Err(self.error_at_token(&self.tok, "missing catch or finally after try"));
        }
        Ok(self.ast.push_stmt(Stmt::Try { block, handler, finalizer }, self.span_from(start)))
    }

    /// `a: b: statement`: the labels read with a loop, each unique among the labels around it.
    /// `in_list`: the labeled statement stands in a statement list, so outside strict mode code
    /// its body may be a function declaration.
    fn parse_labeled(&mut self, in_list: bool) -> Result<StmtId> {
        let mut labels = Vec::new();
        while let TokenKind::Ident(name) = &self.tok.kind {
            if !self.peek().is_some_and(|t| t.is_punct(Punct::Colon)) {
                break;
            }
            let name = name.clone();
            self.check_identifier(&self.tok, &name)?;
            if self.ctx.labels.iter().any(|l| l.name == name) || labels.iter().any(|(l, _): &(Name, Span)| l.name == name) {
                return Err(self.error_at_token(&self.tok, format!("duplicate label '{name}'")));
            }
            let token = self.bump()?;
            self.bump()?;
            labels.push((Name { name, span: token.span }, token.span));
        }
        let is_loop = self.at_keyword(Keyword::For) || self.at_keyword(Keyword::While) || self.at_keyword(Keyword::Do);
        let outer = self.ctx.labels.len();
        self.ctx.labels.extend(labels.iter().map(|(n, _)| Label { name: n.name.clone(), is_loop }));
        self.function_allowed = in_list && !self.ctx.strict;
        let body = self.parse_statement();
        self.ctx.labels.truncate(outer);
        let mut body = body?;
        for (label, label_start) in labels.into_iter().rev() {
            let span = Span::new(label_start.start, self.ast.stmt_span(body).end);
            body = self.ast.push_stmt(Stmt::Labeled { label, body }, span);
        }
        Ok(body)
    }

    // ---- functions ----

    fn parse_function_declaration(&mut self) -> Result<StmtId> {
        let start = self.expect_keyword(Keyword::Function)?.span;
        let name = self.function_name()?;
        let id = self.parse_function_rest(FunctionKind::Declaration, Some(name), start)?;
        Ok(self.ast.push_stmt(Stmt::FunctionDecl(id), self.span_from(start)))
    }

    /// A function's name, checked as an identifier; the strict-mode checks wait for the body,
    /// whose prologue may make the function strict.
    fn function_name(&mut self) -> Result<(Name, Token)> {
        let TokenKind::Ident(name) = &self.tok.kind else {
            return Err(self.error_at_token(&self.tok, format!("expected a function name but found {}", describe(&self.tok))));
        };
        let name = name.clone();
        self.check_identifier(&self.tok, &name)?;
        let token = self.bump()?;
        Ok((Name { name, span: token.span }, token))
    }

    /// The parameters and body of a function, from `(`.
    fn parse_function_rest(&mut self, kind: FunctionKind, name: Option<(Name, Token)>, start: Span) -> Result<crate::ast::FunctionId> {
        self.enter()?;
        let result = self.function_rest(kind, name, start);
        self.leave();
        result
    }

    fn function_rest(&mut self, kind: FunctionKind, name: Option<(Name, Token)>, start: Span) -> Result<crate::ast::FunctionId> {
        self.expect(Punct::LParen)?;
        let mut params: Vec<(Name, Token)> = Vec::new();
        if !self.at(Punct::RParen) {
            loop {
                let (param, token) = self.function_name_or_param()?;
                params.push((param, token));
                if !self.eat(Punct::Comma)? {
                    break;
                }
            }
        }
        let close = self.expect(Punct::RParen)?;
        match kind {
            FunctionKind::Getter if !params.is_empty() => return Err(self.error_at_token(&close, "a getter must not have parameters")),
            FunctionKind::Setter if params.len() != 1 => return Err(self.error_at_token(&close, "a setter must have exactly one parameter")),
            _ => {}
        }
        self.expect(Punct::LBrace)?;
        let inner = Context { strict: self.ctx.strict, in_function: true, ..Context::default() };
        let outer = std::mem::replace(&mut self.ctx, inner);
        let body = self.parse_body(true);
        let strict = self.ctx.strict;
        self.ctx = outer;
        let body = body?;
        self.expect(Punct::RBrace)?;
        if strict {
            self.check_strict_function(name.as_ref(), &params)?;
        }
        Ok(self.ast.push_function(Function {
            kind,
            name: name.map(|(n, _)| n),
            params: params.into_iter().map(|(n, _)| n).collect(),
            body,
            strict,
            span: self.span_from(start),
        }))
    }

    /// A parameter's name.
    fn function_name_or_param(&mut self) -> Result<(Name, Token)> {
        let TokenKind::Ident(name) = &self.tok.kind else {
            return Err(self.error_at_token(&self.tok, format!("expected a parameter name but found {}", describe(&self.tok))));
        };
        let name = name.clone();
        self.check_identifier(&self.tok, &name)?;
        let token = self.bump()?;
        Ok((Name { name, span: token.span }, token))
    }

    /// The rules strict mode code sets for a function's name and parameters (Annex C): neither
    /// `eval`, `arguments` nor a strict-mode reserved word, and no parameter name twice.
    fn check_strict_function(&self, name: Option<&(Name, Token)>, params: &[(Name, Token)]) -> Result<()> {
        for (n, token) in name.into_iter().chain(params) {
            if is_eval_or_arguments(&n.name) {
                return Err(self.error_at_token(token, format!("'{}' cannot be declared in strict mode", n.name)));
            }
            if strict_reserved(&n.name) {
                return Err(self.error_at_token(token, format!("'{}' is a reserved word in strict mode", n.name)));
            }
        }
        for (i, (p, token)) in params.iter().enumerate() {
            if params.iter().take(i).any(|(q, _)| q.name == p.name) {
                return Err(self.error_at_token(token, format!("duplicate parameter name '{}' is not allowed in strict mode", p.name)));
            }
        }
        Ok(())
    }

    // ---- expressions ----

    /// An expression: assignment expressions separated by commas. `no_in` leaves `in` to a
    /// `for (… in …)` around it.
    fn parse_expression(&mut self, no_in: bool) -> Result<ExprId> {
        let first = self.parse_assignment(no_in)?;
        if !self.at(Punct::Comma) {
            return Ok(first);
        }
        let mut list = vec![first];
        while self.eat(Punct::Comma)? {
            list.push(self.parse_assignment(no_in)?);
        }
        let span = self.ast.expr_span(first).to(Span::new(self.prev_end, self.prev_end));
        Ok(self.ast.push_expr(Expr::Sequence(list), span))
    }

    fn parse_assignment(&mut self, no_in: bool) -> Result<ExprId> {
        self.enter()?;
        let result = self.assignment(no_in);
        self.leave();
        result
    }

    /// `a = b += c`: the targets kept on a stack, so a long chain does not nest.
    fn assignment(&mut self, no_in: bool) -> Result<ExprId> {
        let mut targets: Vec<(ExprId, AssignOp)> = Vec::new();
        loop {
            let expr = self.parse_conditional(no_in)?;
            let Some(op) = assign_op(&self.tok) else {
                let mut value = expr;
                while let Some((target, op)) = targets.pop() {
                    let span = self.ast.expr_span(target).to(self.ast.expr_span(value));
                    value = self.ast.push_expr(Expr::Assign { op, target, value }, span);
                }
                return Ok(value);
            };
            self.check_target(expr, "invalid assignment target")?;
            self.bump()?;
            targets.push((expr, op));
        }
    }

    /// An assignment's, `++`'s, `--`'s or `for-in`'s target: an identifier (in strict code
    /// neither `eval` nor `arguments`), a property, or outside strict mode code a call (assigning
    /// to its result fails at run time, as ECMA-262 2015 Annex B and web browsers have it).
    fn check_target(&self, expr: ExprId, message: &str) -> Result<()> {
        let inner = self.ast.unparenthesized(expr);
        match self.ast.expr(inner) {
            Some(Expr::Ident(name)) => {
                if self.ctx.strict && is_eval_or_arguments(name) {
                    return Err(self.error_at_offset(self.ast.expr_span(inner).start, format!("cannot assign to '{name}' in strict mode")));
                }
                Ok(())
            }
            Some(Expr::Member { .. } | Expr::Index { .. }) => Ok(()),
            Some(Expr::Call { .. }) if !self.ctx.strict => Ok(()),
            _ => Err(self.error_at_offset(self.ast.expr_span(expr).start, message)),
        }
    }

    fn parse_conditional(&mut self, no_in: bool) -> Result<ExprId> {
        let test = self.parse_binary(no_in)?;
        if !self.at(Punct::Question) {
            return Ok(test);
        }
        self.bump()?;
        let consequent = self.parse_assignment(false)?;
        self.expect(Punct::Colon)?;
        let alternate = self.parse_assignment(no_in)?;
        let span = self.ast.expr_span(test).to(self.ast.expr_span(alternate));
        Ok(self.ast.push_expr(Expr::Conditional { test, consequent, alternate }, span))
    }

    /// Binary and logical operators, read with an operator stack by precedence (all are left
    /// associative).
    fn parse_binary(&mut self, no_in: bool) -> Result<ExprId> {
        let mut operands = vec![self.parse_unary()?];
        let mut ops: Vec<(BinKind, u8)> = Vec::new();
        while let Some((op, precedence)) = binary_op(&self.tok, no_in) {
            while ops.last().is_some_and(|&(_, top)| top >= precedence) {
                self.reduce(&mut operands, &mut ops);
            }
            self.bump()?;
            ops.push((op, precedence));
            operands.push(self.parse_unary()?);
        }
        while !ops.is_empty() {
            self.reduce(&mut operands, &mut ops);
        }
        operands.pop().ok_or_else(|| self.unexpected())
    }

    /// Joins the top two operands with the top operator.
    fn reduce(&mut self, operands: &mut Vec<ExprId>, ops: &mut Vec<(BinKind, u8)>) {
        let (Some((op, _)), Some(right), Some(left)) = (ops.pop(), operands.pop(), operands.pop()) else {
            return;
        };
        let span = self.ast.expr_span(left).to(self.ast.expr_span(right));
        let expr = match op {
            BinKind::Binary(op) => Expr::Binary { op, left, right },
            BinKind::Logical(op) => Expr::Logical { op, left, right },
        };
        operands.push(self.ast.push_expr(expr, span));
    }

    /// Prefix operators, read with a loop and applied innermost first.
    fn parse_unary(&mut self) -> Result<ExprId> {
        let mut prefixes = Vec::new();
        while let Some(prefix) = prefix_op(&self.tok) {
            prefixes.push((prefix, self.tok.span));
            self.bump()?;
        }
        let mut expr = self.parse_postfix()?;
        for (prefix, start) in prefixes.into_iter().rev() {
            let span = start.to(self.ast.expr_span(expr));
            expr = match prefix {
                Prefix::Update(op) => {
                    self.check_target(expr, "invalid increment or decrement operand")?;
                    self.ast.push_expr(Expr::Update { op, prefix: true, target: expr }, span)
                }
                Prefix::Unary(op) => {
                    if op == UnaryOp::Delete && self.ctx.strict && matches!(self.ast.expr(self.ast.unparenthesized(expr)), Some(Expr::Ident(_))) {
                        return Err(self.error_at_offset(start.start, "delete of an unqualified identifier in strict mode"));
                    }
                    self.ast.push_expr(Expr::Unary { op, arg: expr }, span)
                }
            };
        }
        Ok(expr)
    }

    fn parse_postfix(&mut self) -> Result<ExprId> {
        let expr = self.parse_left_hand_side()?;
        let op = match &self.tok.kind {
            TokenKind::Punct(Punct::PlusPlus) if !self.tok.newline_before => UpdateOp::Increment,
            TokenKind::Punct(Punct::MinusMinus) if !self.tok.newline_before => UpdateOp::Decrement,
            _ => return Ok(expr),
        };
        self.check_target(expr, "invalid increment or decrement operand")?;
        let token = self.bump()?;
        let span = self.ast.expr_span(expr).to(token.span);
        Ok(self.ast.push_expr(Expr::Update { op, prefix: false, target: expr }, span))
    }

    /// Member accesses, calls and `new` (section 11.2): each `new` takes the first argument list
    /// after its callee, and a `new` left without one constructs with no arguments.
    fn parse_left_hand_side(&mut self) -> Result<ExprId> {
        let mut news = Vec::new();
        while self.at_keyword(Keyword::New) {
            news.push(self.bump()?.span);
        }
        let mut expr = self.parse_primary()?;
        loop {
            match &self.tok.kind {
                TokenKind::Punct(Punct::Dot) => {
                    self.bump()?;
                    let Some(name) = self.tok.identifier_name().map(str::to_string) else {
                        return Err(self.error_at_token(&self.tok, format!("expected a property name but found {}", describe(&self.tok))));
                    };
                    let token = self.bump()?;
                    let span = self.ast.expr_span(expr).to(token.span);
                    expr = self.ast.push_expr(Expr::Member { object: expr, property: Name { name, span: token.span } }, span);
                }
                TokenKind::Punct(Punct::LBracket) => {
                    self.bump()?;
                    let index = self.parse_expression(false)?;
                    let close = self.expect(Punct::RBracket)?;
                    let span = self.ast.expr_span(expr).to(close.span);
                    expr = self.ast.push_expr(Expr::Index { object: expr, index }, span);
                }
                TokenKind::Punct(Punct::LParen) => {
                    let args = self.parse_arguments()?;
                    let span = Span::new(self.ast.expr_span(expr).start, self.prev_end);
                    expr = match news.pop() {
                        Some(new) => self.ast.push_expr(Expr::New { callee: expr, args }, Span::new(new.start, self.prev_end)),
                        None => self.ast.push_expr(Expr::Call { callee: expr, args }, span),
                    };
                }
                _ => break,
            }
        }
        while let Some(new) = news.pop() {
            let span = Span::new(new.start, self.ast.expr_span(expr).end);
            expr = self.ast.push_expr(Expr::New { callee: expr, args: Vec::new() }, span);
        }
        Ok(expr)
    }

    fn parse_arguments(&mut self) -> Result<Vec<ExprId>> {
        self.expect(Punct::LParen)?;
        let mut args = Vec::new();
        if !self.at(Punct::RParen) {
            loop {
                args.push(self.parse_assignment(false)?);
                if !self.eat(Punct::Comma)? {
                    break;
                }
            }
        }
        self.expect(Punct::RParen)?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<ExprId> {
        self.enter()?;
        let result = self.primary();
        self.leave();
        result
    }

    fn primary(&mut self) -> Result<ExprId> {
        let start = self.tok.span;
        let expr = match &self.tok.kind {
            TokenKind::Keyword(Keyword::This) => Expr::This,
            TokenKind::Keyword(Keyword::Null) => Expr::Null,
            TokenKind::Keyword(Keyword::True) => Expr::Bool(true),
            TokenKind::Keyword(Keyword::False) => Expr::Bool(false),
            TokenKind::Keyword(Keyword::Function) => return self.parse_function_expression(),
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.check_identifier(&self.tok, &name)?;
                Expr::Ident(name)
            }
            TokenKind::Number(n) => {
                if self.ctx.strict && self.tok.legacy_octal {
                    return Err(self.error_at_token(&self.tok, "octal literals are not allowed in strict mode"));
                }
                Expr::Number(*n)
            }
            TokenKind::String(s) => {
                if self.ctx.strict && self.tok.legacy_octal {
                    return Err(self.error_at_token(&self.tok, "octal escape sequences are not allowed in strict mode"));
                }
                Expr::String(s.clone())
            }
            TokenKind::Punct(Punct::Slash | Punct::SlashEq) => {
                let regexp = self.lex.rescan_regexp(&self.tok)?;
                self.tok = regexp;
                let TokenKind::RegExp { pattern, flags } = &self.tok.kind else {
                    return Err(self.unexpected());
                };
                Expr::RegExp { pattern: pattern.clone(), flags: flags.clone() }
            }
            TokenKind::Punct(Punct::LBracket) => return self.parse_array(),
            TokenKind::Punct(Punct::LBrace) => return self.parse_object(),
            TokenKind::Punct(Punct::LParen) => {
                self.bump()?;
                let inner = self.parse_expression(false)?;
                self.expect(Punct::RParen)?;
                return Ok(self.ast.push_expr(Expr::Paren(inner), self.span_from(start)));
            }
            _ => return Err(self.unexpected()),
        };
        let token = self.bump()?;
        Ok(self.ast.push_expr(expr, token.span))
    }

    fn parse_function_expression(&mut self) -> Result<ExprId> {
        let start = self.expect_keyword(Keyword::Function)?.span;
        let name = if matches!(self.tok.kind, TokenKind::Ident(_)) { Some(self.function_name()?) } else { None };
        let id = self.parse_function_rest(FunctionKind::Expression, name, start)?;
        Ok(self.ast.push_expr(Expr::Function(id), self.span_from(start)))
    }

    /// `[a, , b]`: elements, a comma with nothing before it leaving a hole.
    fn parse_array(&mut self) -> Result<ExprId> {
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
            elements.push(Some(self.parse_assignment(false)?));
            if !self.at(Punct::RBracket) {
                self.expect(Punct::Comma)?;
            }
        }
        Ok(self.ast.push_expr(Expr::Array(elements), self.span_from(start)))
    }

    /// `{a: 1, "b": 2, 3: c, get d() {…}, set d(v) {…}}`. A name may repeat, a later definition
    /// replacing an earlier one (ECMA-262 2015 section 12.2.6.1).
    fn parse_object(&mut self) -> Result<ExprId> {
        let start = self.expect(Punct::LBrace)?.span;
        let mut properties = Vec::new();
        while !self.eat(Punct::RBrace)? {
            let property = self.parse_property()?;
            properties.push(property);
            if !self.at(Punct::RBrace) {
                self.expect(Punct::Comma)?;
            }
        }
        Ok(self.ast.push_expr(Expr::Object(properties), self.span_from(start)))
    }

    fn parse_property(&mut self) -> Result<Property> {
        let start = self.tok.span;
        // `get` and `set` start an accessor only when written without escapes.
        let accessor = match self.tok.ident().filter(|_| !self.tok.escaped) {
            Some("get") => Some(FunctionKind::Getter),
            Some("set") => Some(FunctionKind::Setter),
            _ => None,
        };
        if let Some(kind) = accessor {
            let word = self.bump()?;
            if property_key(&self.tok).is_some() {
                let (key, _) = self.parse_property_key()?;
                let function = self.parse_function_rest(kind, None, start)?;
                let value = if kind == FunctionKind::Getter { PropertyValue::Get(function) } else { PropertyValue::Set(function) };
                return Ok(Property { key, value, span: self.span_from(start) });
            }
            let key = PropertyKey::Ident(word.ident().unwrap_or_default().to_string());
            return self.property_value(key, start);
        }
        let (key, _) = self.parse_property_key()?;
        self.property_value(key, start)
    }

    /// `: value` after a data property's name.
    fn property_value(&mut self, key: PropertyKey, start: Span) -> Result<Property> {
        self.expect(Punct::Colon)?;
        let value = self.parse_assignment(false)?;
        Ok(Property { key, value: PropertyValue::Init(value), span: self.span_from(start) })
    }

    fn parse_property_key(&mut self) -> Result<(PropertyKey, Token)> {
        let Some(key) = property_key(&self.tok) else {
            return Err(self.error_at_token(&self.tok, format!("expected a property name but found {}", describe(&self.tok))));
        };
        if self.ctx.strict && self.tok.legacy_octal {
            return Err(self.error_at_token(&self.tok, "octal literals and escape sequences are not allowed in strict mode"));
        }
        let token = self.bump()?;
        Ok((key, token))
    }
}

/// The key a token gives as a property name: an identifier name, a string or a number.
fn property_key(token: &Token) -> Option<PropertyKey> {
    match &token.kind {
        TokenKind::String(s) => Some(PropertyKey::String(s.clone())),
        TokenKind::Number(n) => Some(PropertyKey::Number(*n)),
        _ => token.identifier_name().map(|n| PropertyKey::Ident(n.to_string())),
    }
}

fn is_eval_or_arguments(name: &str) -> bool {
    name == "eval" || name == "arguments"
}

fn assign_op(token: &Token) -> Option<AssignOp> {
    let TokenKind::Punct(p) = &token.kind else { return None };
    Some(match p {
        Punct::Eq => None,
        Punct::PlusEq => Some(BinaryOp::Add),
        Punct::MinusEq => Some(BinaryOp::Sub),
        Punct::StarEq => Some(BinaryOp::Mul),
        Punct::SlashEq => Some(BinaryOp::Div),
        Punct::PercentEq => Some(BinaryOp::Mod),
        Punct::ShlEq => Some(BinaryOp::Shl),
        Punct::ShrEq => Some(BinaryOp::Shr),
        Punct::UShrEq => Some(BinaryOp::UShr),
        Punct::AmpEq => Some(BinaryOp::BitAnd),
        Punct::PipeEq => Some(BinaryOp::BitOr),
        Punct::CaretEq => Some(BinaryOp::BitXor),
        _ => return None,
    })
}

/// A binary operator and its precedence (higher binds tighter); `in` is left out under `no_in`.
fn binary_op(token: &Token, no_in: bool) -> Option<(BinKind, u8)> {
    use BinKind::{Binary as B, Logical as L};
    match &token.kind {
        TokenKind::Keyword(Keyword::Instanceof) => Some((B(BinaryOp::Instanceof), 7)),
        TokenKind::Keyword(Keyword::In) if !no_in => Some((B(BinaryOp::In), 7)),
        TokenKind::Punct(p) => Some(match p {
            Punct::PipePipe => (L(LogicalOp::Or), 1),
            Punct::AmpAmp => (L(LogicalOp::And), 2),
            Punct::Pipe => (B(BinaryOp::BitOr), 3),
            Punct::Caret => (B(BinaryOp::BitXor), 4),
            Punct::Amp => (B(BinaryOp::BitAnd), 5),
            Punct::EqEq => (B(BinaryOp::Eq), 6),
            Punct::NotEq => (B(BinaryOp::NotEq), 6),
            Punct::EqEqEq => (B(BinaryOp::StrictEq), 6),
            Punct::NotEqEq => (B(BinaryOp::StrictNotEq), 6),
            Punct::Lt => (B(BinaryOp::Lt), 7),
            Punct::Gt => (B(BinaryOp::Gt), 7),
            Punct::LtEq => (B(BinaryOp::LtEq), 7),
            Punct::GtEq => (B(BinaryOp::GtEq), 7),
            Punct::Shl => (B(BinaryOp::Shl), 8),
            Punct::Shr => (B(BinaryOp::Shr), 8),
            Punct::UShr => (B(BinaryOp::UShr), 8),
            Punct::Plus => (B(BinaryOp::Add), 9),
            Punct::Minus => (B(BinaryOp::Sub), 9),
            Punct::Star => (B(BinaryOp::Mul), 10),
            Punct::Slash => (B(BinaryOp::Div), 10),
            Punct::Percent => (B(BinaryOp::Mod), 10),
            _ => return None,
        }),
        _ => None,
    }
}

fn prefix_op(token: &Token) -> Option<Prefix> {
    Some(match &token.kind {
        TokenKind::Keyword(Keyword::Delete) => Prefix::Unary(UnaryOp::Delete),
        TokenKind::Keyword(Keyword::Void) => Prefix::Unary(UnaryOp::Void),
        TokenKind::Keyword(Keyword::Typeof) => Prefix::Unary(UnaryOp::Typeof),
        TokenKind::Punct(Punct::Plus) => Prefix::Unary(UnaryOp::Plus),
        TokenKind::Punct(Punct::Minus) => Prefix::Unary(UnaryOp::Minus),
        TokenKind::Punct(Punct::Tilde) => Prefix::Unary(UnaryOp::BitNot),
        TokenKind::Punct(Punct::Bang) => Prefix::Unary(UnaryOp::Not),
        TokenKind::Punct(Punct::PlusPlus) => Prefix::Update(UpdateOp::Increment),
        TokenKind::Punct(Punct::MinusMinus) => Prefix::Update(UpdateOp::Decrement),
        _ => return None,
    })
}

/// A token as an error message names it.
fn describe(token: &Token) -> String {
    match &token.kind {
        TokenKind::Eof => "end of input".into(),
        TokenKind::Ident(name) => format!("identifier '{name}'"),
        TokenKind::Keyword(k) => format!("token '{}'", k.as_str()),
        TokenKind::Punct(p) => format!("token '{}'", p.as_str()),
        TokenKind::Number(_) => "number".into(),
        TokenKind::String(_) => "string".into(),
        TokenKind::RegExp { .. } => "regular expression".into(),
        TokenKind::Directive(_) => "preprocessor line".into(),
    }
}
