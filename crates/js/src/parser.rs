//! The parser: tokens into a [`Program`] (ECMA-262 5.1 sections 11 to 14), with the early errors
//! of section 16 and of strict mode (Annex C).
//!
//! It is a recursive-descent parser whose recursion is counted: each statement, assignment
//! expression, primary expression and function body it enters adds one to a depth, and going past
//! [`crate::Options::max_depth`] gives a `SyntaxError`. Binary operators are read with an operator
//! stack, and prefix operators, assignment chains, `else if` chains and labels with loops, so long
//! runs of them do not nest at all.

mod xml;

use std::collections::{HashMap, HashSet};

use crate::LineTable;
use crate::ast::{
    AssignOp, Ast, BinaryOp, CatchClause, Expr, ExprId, ForInTarget, ForInit, Function, FunctionKind, LexKind, LogicalOp, Name, Program, Property,
    PropertyKey, PropertyValue, Stmt, StmtId, SwitchCase, TemplatePiece, UnaryOp, UpdateOp, VarDecl,
};
use crate::error::{SyntaxError, TOO_DEEP};
use crate::lexer::{Goal, Lexer};
use crate::token::{Directive, Keyword, Punct, Span, Token, TokenKind, strict_reserved};

type Result<T> = std::result::Result<T, SyntaxError>;

/// What the code being parsed allows, saved and replaced at each function body.
#[derive(Clone, Debug, Default)]
struct Context {
    strict: bool,
    in_function: bool,
    /// The labels in scope, each with whether it labels an iteration statement (`continue` may
    /// name only those). Labels in scope have distinct names.
    labels: HashMap<String, bool>,
    /// Iteration statements open in the current function.
    loops: u32,
    /// Iteration and `switch` statements open in the current function.
    breakables: u32,
    /// The current function body's directive prologue has a `"use strict"` directive.
    use_strict: bool,
    /// The code is an async function's body: `await` is an operator, not an identifier.
    in_async: bool,
    /// The code is an async function's parameters: `await` is neither an operator nor an
    /// identifier there (ECMA-262 2017 section 14.6.1).
    async_params: bool,
}

/// What the parser noted about a parenthesized expression or a call to `async` that may turn out
/// to be an arrow function's head.
#[derive(Clone, Copy, Debug)]
struct Head {
    /// A line break comes between `async` and `(`.
    line_break: bool,
    /// The parentheses hold an `await` expression or the identifier `await`.
    awaits: bool,
    /// A comma follows the last item in the parentheses.
    trailing_comma: bool,
}

/// The names declared in a scope, for the early errors of ECMA-262 2015 about redeclarations
/// (sections 13.2.1, 13.12.1, 13.15.1, 14.1.2 and 15.1.1). Kept only with
/// [`crate::Options::es2015`].
#[derive(Default)]
struct Scope {
    /// The `let`, `const` and (in a block) function declarations, each with whether it is a
    /// function declaration.
    lexical: HashMap<String, bool>,
    /// The `var` names declared in this scope or in a block inside it.
    vars: HashSet<String>,
    /// Names a lexical declaration here must not take: a function's parameters, or a catch
    /// clause's parameter.
    params: HashSet<String>,
    /// A function body or the script: `var` names go no further out, and function declarations
    /// here are var-scoped.
    function: bool,
}

/// The parameters of a function as they are read: plain names, each name's default value, and a
/// rest parameter.
type Params = (Vec<Name>, Vec<Option<ExprId>>, Option<Name>);

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
    /// `await`, in an async function's body.
    Await,
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
    /// Read E4X syntax.
    e4x: bool,
    /// Read the ECMAScript 2015 forms of [`crate::Options::es2015`].
    es2015: bool,
    /// The scopes open around the current token, outermost first (only with `es2015`).
    scopes: Vec<Scope>,
    /// Read async functions and `await` ([`crate::Options::es2017`]).
    es2017: bool,
    /// How many `await` expressions and `await` identifiers have been read.
    awaits: u32,
    /// The parenthesized expressions and calls to `async` read so far that may be arrow heads.
    heads: HashMap<ExprId, Head>,
    /// The last identifier expression read from an `async` written without escapes.
    async_ident: Option<ExprId>,
}

impl<'s> Parser<'s> {
    pub(crate) fn new(src: &'s [u16], options: &crate::Options, keep_tokens: bool) -> Result<Self> {
        let es2015 = options.modern();
        let lex = Lexer::new(src, options.preprocessor_lines, options.max_depth).with_e4x(options.e4x).with_es2015(es2015);
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
            e4x: options.e4x,
            es2015,
            scopes: if es2015 { vec![Scope { function: true, ..Scope::default() }] } else { Vec::new() },
            es2017: options.es2017,
            awaits: 0,
            heads: HashMap::new(),
            async_ident: None,
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
        let program = Program { ast: self.ast, body, strict: self.ctx.strict, directives: self.directives, lines: self.lines, es2015: self.es2015 };
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
        if name == "await" && (self.ctx.in_async || self.ctx.async_params) {
            return Err(self.error_at_token(token, "'await' cannot be an identifier in an async function"));
        }
        Ok(())
    }

    /// The current token is `async` written without escapes, read as the start of an async
    /// function or arrow function only with [`crate::Options::es2017`].
    fn at_async(&self) -> bool {
        self.es2017 && self.tok.ident() == Some("async") && !self.tok.escaped
    }

    /// The current token is `async` and `function` follows it on the same line: an async function.
    fn at_async_function(&self) -> bool {
        self.at_async() && self.peek().is_some_and(|t| t.is_keyword(Keyword::Function) && !t.newline_before)
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
        if name == "await" {
            self.awaits = self.awaits.saturating_add(1);
        }
        let token = self.bump()?;
        Ok(Name { name, span: token.span })
    }

    // ---- scopes (ECMAScript 2015) ----

    /// Opens a scope; `params` are the names a lexical declaration in it must not take.
    fn push_scope(&mut self, function: bool, params: impl IntoIterator<Item = String>) {
        if self.es2015 {
            self.scopes.push(Scope { function, params: params.into_iter().collect(), ..Scope::default() });
        }
    }

    fn pop_scope(&mut self) {
        if self.es2015 {
            self.scopes.pop();
        }
    }

    fn redeclared(&self, name: &Name) -> SyntaxError {
        self.error_at_offset(name.span.start, format!("redeclaration of '{}'", name.name))
    }

    /// Declares a `let`, a `const` or (`function`) a function declaration in a block, in the
    /// innermost scope. A name may be declared there once, and not as a `var` or a parameter too;
    /// outside strict mode code a function declaration may repeat another (Annex B.3.3.4).
    fn declare_lexical(&mut self, name: &Name, function: bool) -> Result<()> {
        let strict = self.ctx.strict;
        let Some(scope) = self.scopes.last_mut() else { return Ok(()) };
        let clash = match scope.lexical.get(&name.name) {
            Some(&was_function) => !(function && was_function && !strict),
            None => false,
        } || scope.vars.contains(&name.name)
            || scope.params.contains(&name.name);
        if clash {
            return Err(self.redeclared(name));
        }
        scope.lexical.insert(name.name.clone(), function);
        Ok(())
    }

    /// Declares a `var` name: in each scope out to the function's, where no lexical declaration
    /// may have it.
    fn declare_var(&mut self, name: &Name) -> Result<()> {
        let mut clash = false;
        for scope in self.scopes.iter_mut().rev() {
            if scope.lexical.contains_key(&name.name) {
                clash = true;
                break;
            }
            scope.vars.insert(name.name.clone());
            if scope.function {
                break;
            }
        }
        if clash { Err(self.redeclared(name)) } else { Ok(()) }
    }

    /// Declares a function declaration that stands in a statement list: var-scoped in a function
    /// body or the script, lexical in a block.
    fn declare_function(&mut self, id: crate::ast::FunctionId) -> Result<()> {
        let Some(f) = self.ast.function(id) else { return Ok(()) };
        let Some(name) = f.name.clone() else { return Ok(()) };
        // An async function in a block never repeats another name (Annex B.3.3.4 covers plain
        // function declarations only).
        let plain = !f.is_async;
        match self.scopes.last_mut() {
            Some(scope) if scope.function => {
                if scope.lexical.contains_key(&name.name) {
                    return Err(self.redeclared(&name));
                }
                scope.vars.insert(name.name);
                Ok(())
            }
            Some(_) => self.declare_lexical(&name, plain),
            None => Ok(()),
        }
    }

    /// The current token starts a `let` or `const` declaration in a statement list: `const`, or a
    /// `let` written without escapes and followed by a name, `[` or `{` (ECMA-262 2015 section
    /// 13.3.1; a `let` followed by anything else is an identifier outside strict mode code).
    fn at_lexical_declaration(&self) -> Option<LexKind> {
        if !self.es2015 {
            return None;
        }
        if self.at_keyword(Keyword::Const) {
            return Some(LexKind::Const);
        }
        if self.tok.ident() == Some("let") && !self.tok.escaped {
            let next = self.peek()?;
            if matches!(next.kind, TokenKind::Ident(_)) || next.is_punct(Punct::LBracket) || next.is_punct(Punct::LBrace) {
                return Some(LexKind::Let);
            }
        }
        None
    }

    /// `let` or `const` declarations after the keyword: each name declared in the innermost scope.
    /// `no_in` leaves `in` to a `for (… in …)` around them, where a `const` may go without an
    /// initializer.
    fn parse_lexical_decls(&mut self, kind: LexKind, no_in: bool) -> Result<Vec<VarDecl>> {
        let mut decls = Vec::new();
        loop {
            if self.at(Punct::LBracket) || self.at(Punct::LBrace) {
                return Err(self.error_at_token(&self.tok, "destructuring declarations are not supported"));
            }
            let name = self.binding_identifier()?;
            if name.name == "let" {
                return Err(self.error_at_offset(name.span.start, "'let' cannot be the name of a let or const declaration"));
            }
            self.declare_lexical(&name, false)?;
            let init = if self.eat(Punct::Eq)? { Some(self.parse_assignment(no_in)?) } else { None };
            let in_head = no_in && self.at_keyword(Keyword::In) && decls.is_empty();
            if kind == LexKind::Const && init.is_none() && !in_head {
                return Err(self.error_at_offset(name.span.start, format!("const '{}' needs an initializer", name.name)));
            }
            decls.push(VarDecl { name, init });
            if !self.eat(Punct::Comma)? {
                return Ok(decls);
            }
        }
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
                    self.ctx.use_strict = true;
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
        if self.at_keyword(Keyword::Function) || self.at_async_function() {
            self.enter()?;
            let result = self.parse_function_declaration();
            self.leave();
            let id = result?;
            if let Some(Stmt::FunctionDecl(fid)) = self.ast.stmt(id) {
                self.declare_function(*fid)?;
            }
            return Ok(id);
        }
        if let Some(kind) = self.at_lexical_declaration() {
            let start = self.bump()?.span;
            self.enter()?;
            let decls = self.parse_lexical_decls(kind, false);
            self.leave();
            let decls = decls?;
            self.semicolon()?;
            return Ok(self.ast.push_stmt(Stmt::Lexical(kind, decls), self.span_from(start)));
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
            TokenKind::Ident(_) if self.at_async_function() => {
                Err(self.error_at_token(&self.tok, "an async function declaration cannot stand here; put it in a block"))
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
            Keyword::Default if self.at_default_xml_namespace() => self.parse_default_xml_namespace(start),
            _ => {
                let expr = self.parse_expression(false)?;
                self.semicolon()?;
                Ok(self.ast.push_stmt(Stmt::Expr(expr), self.span_from(start)))
            }
        }
    }

    /// `{ … }`: its statements.
    fn parse_block(&mut self) -> Result<Vec<StmtId>> {
        self.parse_block_with(None)
    }

    /// `{ … }`, a scope of its own; `param` is a catch clause's parameter, which a lexical
    /// declaration in the block must not take.
    fn parse_block_with(&mut self, param: Option<&str>) -> Result<Vec<StmtId>> {
        self.expect(Punct::LBrace)?;
        self.push_scope(false, param.map(str::to_string));
        let body = self.block_items();
        self.pop_scope();
        let body = body?;
        self.bump()?;
        Ok(body)
    }

    fn block_items(&mut self) -> Result<Vec<StmtId>> {
        let mut body = Vec::new();
        while !self.at(Punct::RBrace) {
            if self.tok.kind == TokenKind::Eof {
                return Err(self.unexpected());
            }
            body.push(self.parse_statement_item()?);
        }
        Ok(body)
    }

    /// `var` declarations after the `var`. `no_in` leaves `in` to a `for (… in …)` around them.
    fn parse_var_decls(&mut self, no_in: bool) -> Result<Vec<VarDecl>> {
        let mut decls = Vec::new();
        loop {
            let name = self.binding_identifier()?;
            self.declare_var(&name)?;
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
        let each = self.e4x && self.tok.ident() == Some("each");
        if each {
            self.bump()?;
        }
        self.expect(Punct::LParen)?;
        if each {
            return self.parse_for_each(start);
        }
        if let Some(kind) = self.at_lexical_declaration() {
            self.bump()?;
            self.push_scope(false, std::iter::empty());
            let result = self.parse_for_lexical(kind, start);
            self.pop_scope();
            return result;
        }
        let init = if self.at(Punct::Semicolon) {
            None
        } else if self.at_keyword(Keyword::Var) {
            self.bump()?;
            let mut decls = self.parse_var_decls(true)?;
            if decls.len() == 1 && self.at_keyword(Keyword::In) {
                let decl = decls.pop().ok_or_else(|| self.unexpected())?;
                // The variable of a for-in loop takes an initializer only outside strict mode
                // code (ECMA-262 2017 Annex B.3.6).
                if self.ctx.strict && decl.init.is_some() {
                    return Err(self.error_at_token(&self.tok, "a for-in variable cannot have an initializer in strict mode code"));
                }
                return self.parse_for_in(ForInTarget::Var(decl), start, false);
            }
            Some(ForInit::Var(decls))
        } else {
            let expr = self.parse_expression(true)?;
            if self.at_keyword(Keyword::In) {
                self.check_target(expr, "invalid left-hand side in for-in loop")?;
                return self.parse_for_in(ForInTarget::Expr(expr), start, false);
            }
            Some(ForInit::Expr(expr))
        };
        self.parse_for_rest(init, start)
    }

    /// The rest of `for (init; test; update) body`, from the first `;`.
    fn parse_for_rest(&mut self, init: Option<ForInit>, start: Span) -> Result<StmtId> {
        self.expect(Punct::Semicolon)?;
        let test = if self.at(Punct::Semicolon) { None } else { Some(self.parse_expression(false)?) };
        self.expect(Punct::Semicolon)?;
        let update = if self.at(Punct::RParen) { None } else { Some(self.parse_expression(false)?) };
        self.expect(Punct::RParen)?;
        let body = self.parse_loop_body()?;
        Ok(self.ast.push_stmt(Stmt::For { init, test, update, body }, self.span_from(start)))
    }

    /// `for (let …` or `for (const …`, after the keyword, in the loop's own scope.
    fn parse_for_lexical(&mut self, kind: LexKind, start: Span) -> Result<StmtId> {
        let mut decls = self.parse_lexical_decls(kind, true)?;
        if decls.len() == 1 && self.at_keyword(Keyword::In) {
            let Some(decl) = decls.pop() else { return Err(self.unexpected()) };
            if decl.init.is_some() {
                return Err(self.error_at_token(&self.tok, "a for-in variable declared with let or const cannot have an initializer"));
            }
            return self.parse_for_in(ForInTarget::Lexical(kind, decl.name), start, false);
        }
        self.parse_for_rest(Some(ForInit::Lexical(kind, decls)), start)
    }

    /// The rest of `for (target in object) body`, from `in`.
    fn parse_for_in(&mut self, target: ForInTarget, start: Span, each: bool) -> Result<StmtId> {
        self.expect_keyword(Keyword::In)?;
        let object = self.parse_expression(false)?;
        self.expect(Punct::RParen)?;
        let body = self.parse_loop_body()?;
        Ok(self.ast.push_stmt(Stmt::ForIn { target, object, body, each }, self.span_from(start)))
    }

    /// `for each (target in object) body`, from the target (ECMA-357 section 12.3).
    fn parse_for_each(&mut self, start: Span) -> Result<StmtId> {
        if self.at_keyword(Keyword::Var) {
            self.bump()?;
            let mut decls = self.parse_var_decls(true)?;
            let (Some(decl), true) = (decls.pop(), decls.is_empty()) else {
                return Err(self.error_at_token(&self.tok, "for each takes one variable"));
            };
            return self.parse_for_in(ForInTarget::Var(decl), start, true);
        }
        if let Some(kind) = self.at_lexical_declaration() {
            self.bump()?;
            self.push_scope(false, std::iter::empty());
            let result = self.parse_for_each_lexical(kind, start);
            self.pop_scope();
            return result;
        }
        let expr = self.parse_expression(true)?;
        self.check_target(expr, "invalid left-hand side in for each loop")?;
        self.parse_for_in(ForInTarget::Expr(expr), start, true)
    }

    /// `for each (let x in object) body` or `for each (const x in object) body`, after `let` or
    /// `const`.
    fn parse_for_each_lexical(&mut self, kind: LexKind, start: Span) -> Result<StmtId> {
        let mut decls = self.parse_lexical_decls(kind, true)?;
        let (Some(decl), true) = (decls.pop(), decls.is_empty()) else {
            return Err(self.error_at_token(&self.tok, "for each takes one variable"));
        };
        if decl.init.is_some() {
            return Err(self.error_at_token(&self.tok, "a for-in variable declared with let or const cannot have an initializer"));
        }
        self.parse_for_in(ForInTarget::Lexical(kind, decl.name), start, true)
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
                match self.ctx.labels.get(&name) {
                    None => return Err(self.error_at_token(&token, format!("undefined label '{name}'"))),
                    Some(&is_loop) if !is_break && !is_loop => {
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
        self.push_scope(false, std::iter::empty());
        let cases = self.parse_cases();
        self.pop_scope();
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
            } else if self.at_keyword(Keyword::Default) && !self.at_default_xml_namespace() {
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
            while !(self.at_keyword(Keyword::Case)
                || (self.at_keyword(Keyword::Default) && !self.at_default_xml_namespace())
                || self.at(Punct::RBrace))
            {
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
            if self.es2015 && (self.at(Punct::LBracket) || self.at(Punct::LBrace)) {
                return Err(self.error_at_token(&self.tok, "destructuring catch parameters are not supported"));
            }
            let param = self.binding_identifier()?;
            self.expect(Punct::RParen)?;
            let body = self.parse_block_with(Some(&param.name))?;
            Some(CatchClause { param, body })
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
        let mut labels: Vec<(Name, Span)> = Vec::new();
        let mut chain = HashSet::new();
        while let TokenKind::Ident(name) = &self.tok.kind {
            if !self.peek().is_some_and(|t| t.is_punct(Punct::Colon)) {
                break;
            }
            let name = name.clone();
            self.check_identifier(&self.tok, &name)?;
            if self.ctx.labels.contains_key(&name) || !chain.insert(name.clone()) {
                return Err(self.error_at_token(&self.tok, format!("duplicate label '{name}'")));
            }
            let token = self.bump()?;
            self.bump()?;
            labels.push((Name { name, span: token.span }, token.span));
        }
        let is_loop = self.at_keyword(Keyword::For) || self.at_keyword(Keyword::While) || self.at_keyword(Keyword::Do);
        self.ctx.labels.extend(chain.into_iter().map(|name| (name, is_loop)));
        self.function_allowed = in_list && !self.ctx.strict;
        let body = self.parse_statement();
        for (label, _) in &labels {
            self.ctx.labels.remove(&label.name);
        }
        let mut body = body?;
        // A labelled function in a statement list is declared there as one without labels is.
        if let Some(Stmt::FunctionDecl(fid)) = self.ast.stmt(body).filter(|_| in_list) {
            self.declare_function(*fid)?;
        }
        for (label, label_start) in labels.into_iter().rev() {
            let span = Span::new(label_start.start, self.ast.stmt_span(body).end);
            body = self.ast.push_stmt(Stmt::Labeled { label, body }, span);
        }
        Ok(body)
    }

    // ---- functions ----

    fn parse_function_declaration(&mut self) -> Result<StmtId> {
        let is_async = self.at_async();
        let start = if is_async { self.bump()?.span } else { self.tok.span };
        self.expect_keyword(Keyword::Function)?;
        let name = self.function_name()?;
        let id = self.parse_function_rest(FunctionKind::Declaration, Some(name), start, is_async)?;
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

    /// The parameters and body of a function, from `(`; with `is_async`, an async function's.
    fn parse_function_rest(
        &mut self,
        kind: FunctionKind,
        name: Option<(Name, Token)>,
        start: Span,
        is_async: bool,
    ) -> Result<crate::ast::FunctionId> {
        self.enter()?;
        // The parameters are read with `await` as the function has it, then the code around
        // them goes on as it was.
        // An `await` inside the function does not count for an arrow function's head around it.
        let (in_async, async_params, awaits) = (self.ctx.in_async, self.ctx.async_params, self.awaits);
        self.ctx.in_async = false;
        self.ctx.async_params = is_async;
        let result = self.function_rest(kind, name, start, is_async);
        self.ctx.in_async = in_async;
        self.ctx.async_params = async_params;
        self.awaits = awaits;
        self.leave();
        result
    }

    fn function_rest(&mut self, kind: FunctionKind, name: Option<(Name, Token)>, start: Span, is_async: bool) -> Result<crate::ast::FunctionId> {
        self.expect(Punct::LParen)?;
        let mut params: Vec<Name> = Vec::new();
        let mut defaults: Vec<Option<ExprId>> = Vec::new();
        let mut rest = None;
        if !self.at(Punct::RParen) {
            loop {
                if self.es2015 && self.at(Punct::Ellipsis) {
                    self.bump()?;
                    rest = Some(self.function_name_or_param()?.0);
                    if !self.at(Punct::RParen) {
                        return Err(self.error_at_token(&self.tok, "a rest parameter must be the last parameter"));
                    }
                    break;
                }
                let (param, _) = self.function_name_or_param()?;
                let default = if self.es2015 && self.eat(Punct::Eq)? { Some(self.parse_assignment(false)?) } else { None };
                params.push(param);
                defaults.push(default);
                // A comma may follow the last parameter (ECMAScript 2017, section 14.1), except in
                // a setter, which takes one parameter and no comma (section 14.3).
                if !self.eat(Punct::Comma)? {
                    break;
                }
                if self.at(Punct::RParen) {
                    if kind == FunctionKind::Setter {
                        return Err(self.error_at_token(&self.tok, "a setter must have exactly one parameter"));
                    }
                    break;
                }
            }
        }
        let close = self.expect(Punct::RParen)?;
        match kind {
            FunctionKind::Getter if !params.is_empty() || rest.is_some() => {
                return Err(self.error_at_token(&close, "a getter must not have parameters"));
            }
            FunctionKind::Setter if params.len() != 1 || rest.is_some() => {
                return Err(self.error_at_token(&close, "a setter must have exactly one parameter"));
            }
            _ => {}
        }
        if defaults.iter().all(Option::is_none) {
            defaults.clear();
        }
        self.expect(Punct::LBrace)?;
        let names: Vec<String> = params.iter().chain(&rest).map(|p| p.name.clone()).collect();
        let outer_strict = self.ctx.strict;
        let inner = Context { strict: outer_strict, in_function: true, in_async: is_async, ..Context::default() };
        let outer = std::mem::replace(&mut self.ctx, inner);
        self.push_scope(true, names);
        let body = self.parse_body(true);
        self.pop_scope();
        let (strict, use_strict) = (self.ctx.strict, self.ctx.use_strict);
        self.ctx = outer;
        let body = body?;
        self.expect(Punct::RBrace)?;
        let span = self.span_from(start);
        let function = Function { kind, name: name.map(|(n, _)| n), params, defaults, rest, body, strict, is_async, span };
        // An async declaration or expression takes FormalParameters as a plain function does
        // (ECMA-262 2017 section 14.6): repeated simple names are an error only in strict code.
        // Arrow functions and async methods reject every repeat.
        self.check_params(&function, use_strict)?;
        Ok(self.ast.push_function(function))
    }

    /// An arrow function's body and the function, from `=>`, with its parameters read; with
    /// `is_async`, an async arrow function's.
    fn parse_arrow(&mut self, start: Span, (params, defaults, rest): Params, no_in: bool, is_async: bool) -> Result<ExprId> {
        if self.tok.newline_before {
            return Err(self.error_at_token(&self.tok, "a line break cannot come before '=>'"));
        }
        self.expect(Punct::Arrow)?;
        self.enter()?;
        let names: Vec<String> = params.iter().chain(&rest).map(|p| p.name.clone()).collect();
        let outer_strict = self.ctx.strict;
        let inner = Context { strict: outer_strict, in_function: true, in_async: is_async, ..Context::default() };
        let outer = std::mem::replace(&mut self.ctx, inner);
        // An `await` in the body does not count for an async arrow function's head around it.
        let awaits = self.awaits;
        self.push_scope(true, names);
        let block = self.at(Punct::LBrace);
        let body = if block {
            self.bump().and_then(|_| self.parse_body(true))
        } else {
            self.parse_assignment(no_in).map(|e| vec![self.ast.push_stmt(Stmt::Return(Some(e)), self.ast.expr_span(e))])
        };
        self.pop_scope();
        self.awaits = awaits;
        let (strict, use_strict) = (self.ctx.strict, self.ctx.use_strict);
        self.ctx = outer;
        self.leave();
        let body = body?;
        if block {
            self.expect(Punct::RBrace)?;
        }
        let defaults = if defaults.iter().all(Option::is_none) { Vec::new() } else { defaults };
        let function =
            Function { kind: FunctionKind::Arrow, name: None, params, defaults, rest, body, strict, is_async, span: self.span_from(start) };
        self.check_params(&function, use_strict)?;
        let id = self.ast.push_function(function);
        Ok(self.ast.push_expr(Expr::Arrow(id), self.span_from(start)))
    }

    /// The parameters an arrow function's parenthesized head `expr` gives (ECMA-262 2015 section
    /// 14.2.1): names, names with `= default`, and a last `...name`, each written bare.
    fn arrow_params(&self, expr: ExprId) -> Result<Params> {
        let invalid = |at: ExprId| self.error_at_offset(self.ast.expr_span(at).start, "invalid arrow function parameters");
        let Some(Expr::Paren(inner)) = self.ast.expr(expr) else { return Err(invalid(expr)) };
        let items = match self.ast.expr(*inner) {
            Some(Expr::Sequence(list)) => list.clone(),
            _ => vec![*inner],
        };
        self.arrow_items(items)
    }

    /// The parameters the items of an arrow function's head give: names, names with `= default`,
    /// and a last `...name`.
    fn arrow_items(&self, items: Vec<ExprId>) -> Result<Params> {
        let invalid = |at: ExprId| self.error_at_offset(self.ast.expr_span(at).start, "invalid arrow function parameters");
        let (mut params, mut defaults, mut rest) = (Vec::new(), Vec::new(), None);
        let name_of = |id: ExprId| match self.ast.expr(id) {
            Some(Expr::Ident(n)) => Some(Name { name: n.clone(), span: self.ast.expr_span(id) }),
            _ => None,
        };
        let count = items.len();
        for (i, item) in items.into_iter().enumerate() {
            match self.ast.expr(item) {
                Some(Expr::Ident(_)) => {
                    params.push(name_of(item).ok_or_else(|| invalid(item))?);
                    defaults.push(None);
                }
                Some(Expr::Assign { op: None, target, value }) => {
                    params.push(name_of(*target).ok_or_else(|| invalid(item))?);
                    defaults.push(Some(*value));
                }
                Some(Expr::Spread(arg)) if i + 1 == count => rest = Some(name_of(*arg).ok_or_else(|| invalid(item))?),
                _ => return Err(invalid(item)),
            }
        }
        Ok((params, defaults, rest))
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

    /// The rules for a function's name and parameters. Strict mode code (Annex C): neither `eval`,
    /// `arguments` nor a strict-mode reserved word, and no parameter name twice. An arrow function
    /// or a function with a default or rest parameter has no parameter name twice either (ECMA-262
    /// 2015 sections 14.1.2 and 14.2.1), and its body cannot start with `"use strict"`
    /// (`body_strict`; ECMA-262 2016 section 14.1.2).
    fn check_params(&self, function: &Function, body_strict: bool) -> Result<()> {
        let params = || function.params.iter().chain(&function.rest);
        let simple = function.simple_params() && function.kind != FunctionKind::Arrow;
        if body_strict && !function.simple_params() {
            let at = function.body.first().map_or(function.span.start, |&s| self.ast.stmt_span(s).start);
            return Err(self.error_at_offset(at, "\"use strict\" is not allowed in a function with default or rest parameters"));
        }
        if function.strict {
            for n in function.name.iter().chain(params()) {
                if is_eval_or_arguments(&n.name) {
                    return Err(self.error_at_offset(n.span.start, format!("'{}' cannot be declared in strict mode", n.name)));
                }
                if strict_reserved(&n.name) {
                    return Err(self.error_at_offset(n.span.start, format!("'{}' is a reserved word in strict mode", n.name)));
                }
            }
        }
        if function.strict || !simple {
            let mut seen = HashSet::new();
            for p in params() {
                if !seen.insert(p.name.as_str()) {
                    let message = if function.strict {
                        format!("duplicate parameter name '{}' is not allowed in strict mode", p.name)
                    } else {
                        format!("duplicate parameter name '{}'", p.name)
                    };
                    return Err(self.error_at_offset(p.span.start, message));
                }
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
            // `async` followed on its line by a name starts an async arrow function.
            let async_name = self.at_async() && self.peek().is_some_and(|t| matches!(t.kind, TokenKind::Ident(_)) && !t.newline_before);
            let mut expr = if async_name { self.parse_async_name_arrow(no_in)? } else { self.parse_conditional(no_in)? };
            if self.es2015 && self.at(Punct::Arrow) {
                expr = self.arrow_from(expr, no_in)?;
            }
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

    /// The arrow function whose head is `expr`, read before the current `=>`: a name standing
    /// alone, or parameters in parentheses.
    fn arrow_from(&mut self, expr: ExprId, no_in: bool) -> Result<ExprId> {
        let start = self.ast.expr_span(expr);
        let head = self.heads.get(&expr).copied();
        let (params, is_async) = match self.ast.expr(expr) {
            Some(Expr::Ident(name)) => ((vec![Name { name: name.clone(), span: start }], vec![None], None), false),
            Some(Expr::Paren(_)) => (self.arrow_params(expr)?, false),
            // `async (…) =>` was read as a call to `async` (ECMA-262 2017 section 14.7).
            Some(Expr::Call { args, .. }) if head.is_some() => {
                if head.is_some_and(|h| h.line_break) {
                    return Err(self.error_at_offset(start.start, "a line break cannot come after 'async' in an async arrow function"));
                }
                // No comma may follow a rest parameter (ECMA-262 2017 section 14.1).
                if let Some(&last) = args.last()
                    && head.is_some_and(|h| h.trailing_comma)
                    && matches!(self.ast.expr(last), Some(Expr::Spread(_)))
                {
                    return Err(self.error_at_offset(self.ast.expr_span(last).start, "a rest parameter must be the last parameter"));
                }
                (self.arrow_items(args.clone())?, true)
            }
            _ => return Err(self.unexpected()),
        };
        // `await` cannot be an arrow function's parameter, or stand in one's default value,
        // where it is an operator or in an async arrow function's head.
        if head.is_some_and(|h| h.awaits) && (is_async || self.ctx.in_async) {
            return Err(self.error_at_offset(start.start, "'await' cannot stand in an arrow function's parameters here"));
        }
        self.parse_arrow(start, params, no_in, is_async)
    }

    /// `async name => …` from `async` (ECMA-262 2017 section 14.7): the one parameter is read as
    /// an async function's is.
    fn parse_async_name_arrow(&mut self, no_in: bool) -> Result<ExprId> {
        let start = self.bump()?.span;
        let async_params = std::mem::replace(&mut self.ctx.async_params, true);
        let param = self.binding_identifier();
        self.ctx.async_params = async_params;
        let param = param?;
        if !self.at(Punct::Arrow) {
            return Err(self.error_at_token(&self.tok, format!("expected '=>' but found {}", describe(&self.tok))));
        }
        self.parse_arrow(start, (vec![param], vec![None], None), no_in, true)
    }

    /// An assignment's, `++`'s, `--`'s or `for-in`'s target: an identifier (in strict code
    /// neither `eval` nor `arguments`), a property, an E4X name standing alone (`@a`, `ns::a`;
    /// ECMA-357 section 11.1), or outside strict mode code a call (assigning to its result fails
    /// at run time, as ECMA-262 2015 Annex B and web browsers have it).
    fn check_target(&self, expr: ExprId, message: &str) -> Result<()> {
        let inner = self.ast.unparenthesized(expr);
        match self.ast.expr(inner) {
            Some(Expr::Ident(name)) => {
                if self.ctx.strict && is_eval_or_arguments(name) {
                    return Err(self.error_at_offset(self.ast.expr_span(inner).start, format!("cannot assign to '{name}' in strict mode")));
                }
                Ok(())
            }
            Some(Expr::Member { .. } | Expr::Index { .. } | Expr::XmlScopeName(_)) => Ok(()),
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
                self.reduce(&mut operands, &mut ops)?;
            }
            self.bump()?;
            ops.push((op, precedence));
            operands.push(self.parse_unary()?);
        }
        while !ops.is_empty() {
            self.reduce(&mut operands, &mut ops)?;
        }
        operands.pop().ok_or_else(|| self.unexpected())
    }

    /// Joins the top two operands with the top operator.
    fn reduce(&mut self, operands: &mut Vec<ExprId>, ops: &mut Vec<(BinKind, u8)>) -> Result<()> {
        let (Some((op, _)), Some(right), Some(left)) = (ops.pop(), operands.pop(), operands.pop()) else {
            return Ok(());
        };
        let span = self.ast.expr_span(left).to(self.ast.expr_span(right));
        let expr = match op {
            BinKind::Binary(op) => Expr::Binary { op, left, right },
            BinKind::Logical(op) => {
                self.check_coalesce_mix(op, left, right)?;
                Expr::Logical { op, left, right }
            }
        };
        operands.push(self.ast.push_expr(expr, span));
        Ok(())
    }

    /// `??` and `&&` or `||` written together without parentheses around one of them, which
    /// ECMA-262 2020 section 12.13 does not allow (`a ?? b || c`, `a && b ?? c`).
    fn check_coalesce_mix(&self, op: LogicalOp, left: ExprId, right: ExprId) -> Result<()> {
        let coalesce = op == LogicalOp::Coalesce;
        for operand in [left, right] {
            if let Some(Expr::Logical { op: inner, .. }) = self.ast.expr(operand)
                && (*inner == LogicalOp::Coalesce) != coalesce
            {
                return Err(self.error_at_offset(
                    self.ast.expr_span(operand).start,
                    "'??' cannot be used with '&&' or '||' without parentheses around one of them",
                ));
            }
        }
        Ok(())
    }

    /// Prefix operators, read with a loop and applied innermost first.
    fn parse_unary(&mut self) -> Result<ExprId> {
        let mut prefixes = Vec::new();
        loop {
            // In an async function's body, `await` is a prefix operator (ECMA-262 2017 section 14.6).
            let prefix = if self.ctx.in_async && self.tok.ident() == Some("await") {
                if self.tok.escaped {
                    return Err(self.error_at_token(&self.tok, "keyword 'await' must not contain escaped characters"));
                }
                Prefix::Await
            } else {
                let Some(prefix) = prefix_op(&self.tok) else { break };
                prefix
            };
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
                Prefix::Await => {
                    self.awaits = self.awaits.saturating_add(1);
                    self.ast.push_expr(Expr::Await(expr), span)
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
                TokenKind::Punct(Punct::DotDot) => expr = self.parse_xml_descendants(expr)?,
                TokenKind::Punct(Punct::Dot) => {
                    self.bump()?;
                    if self.e4x && self.at_e4x_member() {
                        expr = self.parse_xml_member(expr)?;
                        continue;
                    }
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
                TokenKind::Template { .. } => {
                    let (quasis, exprs) = self.parse_template()?;
                    let span = Span::new(self.ast.expr_span(expr).start, self.prev_end);
                    expr = self.ast.push_expr(Expr::TaggedTemplate { tag: expr, quasis, exprs }, span);
                }
                TokenKind::Punct(Punct::LParen) => {
                    let async_head = news.is_empty() && self.async_ident.is_some_and(|a| a == expr);
                    let line_break = self.tok.newline_before;
                    let awaits = self.awaits;
                    let (args, trailing_comma) = self.arguments()?;
                    let span = Span::new(self.ast.expr_span(expr).start, self.prev_end);
                    expr = match news.pop() {
                        Some(new) => self.ast.push_expr(Expr::New { callee: expr, args }, Span::new(new.start, self.prev_end)),
                        None => self.ast.push_expr(Expr::Call { callee: expr, args }, span),
                    };
                    if async_head && self.at(Punct::Arrow) {
                        self.heads.insert(expr, Head { line_break, awaits: self.awaits != awaits, trailing_comma });
                    }
                }
                TokenKind::Punct(Punct::QuestionDot) => {
                    if !news.is_empty() {
                        return Err(self.error_at_token(&self.tok, "an optional chain cannot follow 'new' without parentheses around it"));
                    }
                    expr = self.parse_optional_chain(expr)?;
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

    /// An optional chain (ECMA-262 2020 section 12.3.9), from the `?.` after `base` to the last
    /// access or call that follows it. A `?.` inside it is a step of the same chain; a tagged
    /// template and the E4X accesses (`..`, `.@`, `.(`) cannot follow one.
    fn parse_optional_chain(&mut self, base: ExprId) -> Result<ExprId> {
        let start = self.ast.expr_span(base).start;
        let mut expr = base;
        loop {
            match &self.tok.kind {
                TokenKind::Punct(Punct::QuestionDot) => {
                    self.bump()?;
                    let object = self.ast.push_expr(Expr::Optional(expr), self.ast.expr_span(expr));
                    expr = match &self.tok.kind {
                        TokenKind::Punct(Punct::LParen) => {
                            let args = self.parse_arguments()?;
                            self.ast.push_expr(Expr::Call { callee: object, args }, Span::new(start, self.prev_end))
                        }
                        TokenKind::Punct(Punct::LBracket) => self.optional_index(object, start)?,
                        _ => self.optional_member(object, start)?,
                    };
                }
                TokenKind::Punct(Punct::Dot) => {
                    self.bump()?;
                    if self.e4x && self.at_e4x_member() {
                        return Err(self.error_at_token(&self.tok, "an E4X access cannot follow an optional chain"));
                    }
                    expr = self.optional_member(expr, start)?;
                }
                TokenKind::Punct(Punct::LBracket) => expr = self.optional_index(expr, start)?,
                TokenKind::Punct(Punct::LParen) => {
                    let args = self.parse_arguments()?;
                    expr = self.ast.push_expr(Expr::Call { callee: expr, args }, Span::new(start, self.prev_end));
                }
                TokenKind::Template { .. } => return Err(self.error_at_token(&self.tok, "a tagged template cannot follow an optional chain")),
                TokenKind::Punct(Punct::DotDot) => return Err(self.error_at_token(&self.tok, "an E4X access cannot follow an optional chain")),
                _ => break,
            }
        }
        Ok(self.ast.push_expr(Expr::OptionalChain(expr), Span::new(start, self.prev_end)))
    }

    /// `.name` in an optional chain, with the `.` (or `?.`) already read.
    fn optional_member(&mut self, object: ExprId, start: u32) -> Result<ExprId> {
        let Some(name) = self.tok.identifier_name().map(str::to_string) else {
            return Err(self.error_at_token(&self.tok, format!("expected a property name but found {}", describe(&self.tok))));
        };
        let token = self.bump()?;
        Ok(self.ast.push_expr(Expr::Member { object, property: Name { name, span: token.span } }, Span::new(start, token.span.end)))
    }

    /// `[index]` in an optional chain.
    fn optional_index(&mut self, object: ExprId, start: u32) -> Result<ExprId> {
        self.expect(Punct::LBracket)?;
        let index = self.parse_expression(false)?;
        let close = self.expect(Punct::RBracket)?;
        Ok(self.ast.push_expr(Expr::Index { object, index }, Span::new(start, close.span.end)))
    }

    /// An element of an array literal or an argument list: an assignment expression, or with
    /// [`crate::Options::es2015`] a spread (`...expression`).
    fn parse_element(&mut self) -> Result<ExprId> {
        if self.es2015 && self.at(Punct::Ellipsis) {
            let start = self.bump()?.span;
            let arg = self.parse_assignment(false)?;
            return Ok(self.ast.push_expr(Expr::Spread(arg), self.span_from(start)));
        }
        self.parse_assignment(false)
    }

    fn parse_arguments(&mut self) -> Result<Vec<ExprId>> {
        Ok(self.arguments()?.0)
    }

    /// An argument list, and whether a comma follows its last argument.
    fn arguments(&mut self) -> Result<(Vec<ExprId>, bool)> {
        self.expect(Punct::LParen)?;
        let mut args = Vec::new();
        let mut trailing_comma = false;
        if !self.at(Punct::RParen) {
            loop {
                args.push(self.parse_element()?);
                // A comma may follow the last argument (ECMAScript 2017, section 12.3).
                if !self.eat(Punct::Comma)? {
                    break;
                }
                if self.at(Punct::RParen) {
                    trailing_comma = true;
                    break;
                }
            }
        }
        self.expect(Punct::RParen)?;
        Ok((args, trailing_comma))
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
            TokenKind::Ident(_) if self.at_async_function() => return self.parse_function_expression(),
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.check_identifier(&self.tok, &name)?;
                if name == "await" {
                    self.awaits = self.awaits.saturating_add(1);
                }
                // With E4X, `async::name` is a qualified name.
                if self.at_async() && !(self.e4x && self.peek().is_some_and(|t| t.is_punct(Punct::ColonColon))) {
                    let token = self.bump()?;
                    let id = self.ast.push_expr(Expr::Ident(name), token.span);
                    self.async_ident = Some(id);
                    return Ok(id);
                }
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
            TokenKind::Punct(Punct::LParen) if self.es2015 => return self.parse_paren_or_arrow_head(),
            TokenKind::Punct(Punct::LParen) => {
                self.bump()?;
                let inner = self.parse_expression(false)?;
                self.expect(Punct::RParen)?;
                return Ok(self.ast.push_expr(Expr::Paren(inner), self.span_from(start)));
            }
            TokenKind::Template { .. } => {
                let (quasis, exprs) = self.parse_template()?;
                return Ok(self.ast.push_expr(Expr::Template { quasis, exprs }, self.span_from(start)));
            }
            TokenKind::Punct(Punct::Lt) if self.e4x => return self.parse_xml_literal(),
            TokenKind::Punct(Punct::At | Punct::Star) if self.e4x => return self.parse_xml_scope_name(),
            _ => return Err(self.unexpected()),
        };
        let token = self.bump()?;
        let id = self.ast.push_expr(expr, token.span);
        if self.e4x && self.at(Punct::ColonColon) && matches!(token.kind, TokenKind::Ident(_)) {
            return self.parse_xml_qualified_primary(id);
        }
        Ok(id)
    }

    /// `( … )` with [`crate::Options::es2015`]: a parenthesized expression, or the head of an arrow
    /// function, which may also be `()`, end with `...name` or end with a comma; those three must
    /// come before `=>`.
    fn parse_paren_or_arrow_head(&mut self) -> Result<ExprId> {
        let start = self.expect(Punct::LParen)?.span;
        let awaits = self.awaits;
        let mut items = Vec::new();
        let mut head_only = self.at(Punct::RParen);
        while !self.at(Punct::RParen) {
            if self.at(Punct::Ellipsis) {
                let spread_start = self.bump()?.span;
                let name = self.binding_identifier()?;
                let arg = self.ast.push_expr(Expr::Ident(name.name), name.span);
                items.push(self.ast.push_expr(Expr::Spread(arg), self.span_from(spread_start)));
                head_only = true;
                break;
            }
            items.push(self.parse_assignment(false)?);
            if !self.eat(Punct::Comma)? {
                break;
            }
            // A comma may follow the last parameter (ECMA-262 2017 section 14.2), but not the
            // last item of a parenthesized expression.
            if self.at(Punct::RParen) {
                head_only = true;
                break;
            }
        }
        self.expect(Punct::RParen)?;
        if head_only && !self.at(Punct::Arrow) {
            return Err(self.error_at_token(&self.tok, format!("expected '=>' but found {}", describe(&self.tok))));
        }
        let inner = match items.as_slice() {
            [one] => *one,
            _ => {
                let span = self.span_from(start);
                self.ast.push_expr(Expr::Sequence(items), span)
            }
        };
        let paren = self.ast.push_expr(Expr::Paren(inner), self.span_from(start));
        if self.at(Punct::Arrow) {
            self.heads.insert(paren, Head { line_break: false, awaits: self.awaits != awaits, trailing_comma: false });
        }
        Ok(paren)
    }

    /// A template literal from its first piece (the current token): its pieces and substitutions.
    fn parse_template(&mut self) -> Result<(Vec<TemplatePiece>, Vec<ExprId>)> {
        let mut quasis = Vec::new();
        let mut exprs = Vec::new();
        loop {
            let TokenKind::Template { cooked, raw, tail } = &self.tok.kind else { return Err(self.unexpected()) };
            let tail = *tail;
            quasis.push(TemplatePiece { cooked: cooked.clone(), raw: raw.clone() });
            if tail {
                self.bump()?;
                return Ok((quasis, exprs));
            }
            self.bump()?;
            self.enter()?;
            let expr = self.parse_expression(false);
            self.leave();
            exprs.push(expr?);
            if !self.at(Punct::RBrace) {
                return Err(self.error_at_token(&self.tok, format!("expected '}}' but found {}", describe(&self.tok))));
            }
            let piece = self.lex.rescan_template(&self.tok)?;
            self.tok = piece;
        }
    }

    fn parse_function_expression(&mut self) -> Result<ExprId> {
        let is_async = self.at_async();
        let start = if is_async { self.bump()?.span } else { self.tok.span };
        self.expect_keyword(Keyword::Function)?;
        // A function expression's name is read as its own body reads names (ECMA-262 2017
        // section 14.6): `await` is a name for an ordinary one, and not for an async one.
        // Nor does the name count for an arrow function's head around it.
        let (in_async, async_params, awaits) = (self.ctx.in_async, self.ctx.async_params, self.awaits);
        self.ctx.in_async = is_async;
        self.ctx.async_params = false;
        let name = if matches!(self.tok.kind, TokenKind::Ident(_)) { self.function_name().map(Some) } else { Ok(None) };
        self.ctx.in_async = in_async;
        self.ctx.async_params = async_params;
        self.awaits = awaits;
        let id = self.parse_function_rest(FunctionKind::Expression, name?, start, is_async)?;
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
            elements.push(Some(self.parse_element()?));
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
                let function = self.parse_function_rest(kind, None, start, false)?;
                let value = if kind == FunctionKind::Getter { PropertyValue::Get(function) } else { PropertyValue::Set(function) };
                return Ok(Property { key, value, span: self.span_from(start) });
            }
            let key = PropertyKey::Ident(word.ident().unwrap_or_default().to_string());
            return self.property_value(key, start);
        }
        // `async name(…) {…}` (ECMA-262 2017 section 14.6): an async method, whose value is an
        // async function that does not bind its own name.
        if self.at_async() && self.peek().is_some_and(|t| !t.newline_before && property_key(&t).is_some()) {
            self.bump()?;
            let (key, _) = self.parse_property_key()?;
            let function = self.parse_function_rest(FunctionKind::Expression, None, start, true)?;
            // A method's parameters have distinct names (UniqueFormalParameters).
            let mut seen = HashSet::new();
            for p in self.ast.function(function).into_iter().flat_map(|f| f.params.iter().chain(&f.rest)) {
                if !seen.insert(p.name.as_str()) {
                    return Err(self.error_at_offset(p.span.start, format!("duplicate parameter name '{}'", p.name)));
                }
            }
            let span = self.span_from(start);
            let value = self.ast.push_expr(Expr::Function(function), span);
            return Ok(Property { key, value: PropertyValue::Init(value), span });
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
            Punct::QuestionQuestion => (L(LogicalOp::Coalesce), 1),
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
        TokenKind::XmlLiteral => "XML literal".into(),
        TokenKind::Template { .. } => "template literal".into(),
    }
}
