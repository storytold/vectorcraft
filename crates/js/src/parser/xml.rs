//! E4X syntax (ECMA-357 sections 11.1 and 12.1): literals, names and the operators on them.
//!
//! A literal is read by scanning the source units with an explicit stack of open tags, so tag
//! nesting uses no native recursion and is held to [`crate::Options::max_depth`]. Only an embedded
//! `{expression}` re-enters the expression parser, and that recursion is counted as any other.

use super::{Parser, Result, describe};
use crate::ast::{Expr, ExprId, Stmt, StmtId, XmlNamePart, XmlPart, XmlQualifier};
use crate::error::TOO_DEEP;
use crate::token::{Keyword, Punct, Span, TokenKind};
use crate::xml::{is_name_part, is_name_start, is_xml_space, scan_reference, valid_xml_units};

const LT: u16 = b'<' as u16;
const GT: u16 = b'>' as u16;
const SLASH: u16 = b'/' as u16;
const AMP: u16 = b'&' as u16;
const LBRACE: u16 = b'{' as u16;
const EQ: u16 = b'=' as u16;
const DQUOTE: u16 = b'"' as u16;
const SQUOTE: u16 = b'\'' as u16;

/// A literal being read: the markup since the last embedded expression, and the finished parts.
#[derive(Default)]
struct Scan {
    text: Vec<u16>,
    parts: Vec<XmlPart>,
}

impl Scan {
    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.parts.push(XmlPart::Text(std::mem::take(&mut self.text)));
        }
    }
}

impl Parser<'_> {
    pub(super) fn at_default_xml_namespace(&self) -> bool {
        self.e4x && self.peek().is_some_and(|t| t.ident() == Some("xml"))
    }

    /// `default xml namespace = expression;`, from `default`.
    pub(super) fn parse_default_xml_namespace(&mut self, start: Span) -> Result<StmtId> {
        self.expect_keyword(Keyword::Default)?;
        self.bump()?;
        if self.tok.ident() != Some("namespace") {
            return Err(self.error_at_token(&self.tok, format!("expected 'namespace' but found {}", describe(&self.tok))));
        }
        self.bump()?;
        self.expect(Punct::Eq)?;
        let value = self.parse_expression(false)?;
        self.semicolon()?;
        Ok(self.ast.push_stmt(Stmt::DefaultXmlNamespace(value), self.span_from(start)))
    }

    /// After a `.`: the next token starts an E4X name or a filter rather than a property name.
    pub(super) fn at_e4x_member(&self) -> bool {
        match &self.tok.kind {
            TokenKind::Punct(Punct::At | Punct::Star | Punct::LParen) => true,
            _ => self.tok.identifier_name().is_some() && self.peek().is_some_and(|t| t.is_punct(Punct::ColonColon)),
        }
    }

    /// `object.@name`, `object.*`, `object.ns::name` or `object.(predicate)`, from the token after
    /// the `.`.
    pub(super) fn parse_xml_member(&mut self, object: ExprId) -> Result<ExprId> {
        let start = self.ast.expr_span(object);
        if self.at(Punct::LParen) {
            self.bump()?;
            let predicate = self.parse_expression(false)?;
            self.expect(Punct::RParen)?;
            return Ok(self.ast.push_expr(Expr::XmlFilter { object, predicate }, self.span_from(start)));
        }
        let index = self.parse_xml_name()?;
        Ok(self.ast.push_expr(Expr::Index { object, index }, self.span_from(start)))
    }

    /// `object..name`, from the `..`.
    pub(super) fn parse_xml_descendants(&mut self, object: ExprId) -> Result<ExprId> {
        let start = self.ast.expr_span(object);
        self.bump()?;
        let name = self.parse_xml_name()?;
        Ok(self.ast.push_expr(Expr::XmlDescendants { object, name }, self.span_from(start)))
    }

    /// `@name`, `*` or `@*` standing alone as a primary expression.
    pub(super) fn parse_xml_scope_name(&mut self) -> Result<ExprId> {
        let start = self.tok.span;
        let name = self.parse_xml_name()?;
        Ok(self.ast.push_expr(Expr::XmlScopeName(name), self.span_from(start)))
    }

    /// `qualifier::name` as a primary expression, from the `::`; `qualifier` is already read.
    pub(super) fn parse_xml_qualified_primary(&mut self, qualifier: ExprId) -> Result<ExprId> {
        let start = self.ast.expr_span(qualifier);
        self.bump()?;
        let name = self.parse_xml_name_part()?;
        let qualifier = Some(XmlQualifier::Expr(qualifier));
        let id = self.ast.push_expr(Expr::XmlName { attribute: false, qualifier, name }, self.span_from(start));
        Ok(self.ast.push_expr(Expr::XmlScopeName(id), self.span_from(start)))
    }

    /// An E4X name: `@`, then `*`, a name or `[expression]`, with an optional `qualifier::`.
    fn parse_xml_name(&mut self) -> Result<ExprId> {
        let start = self.tok.span;
        let attribute = self.eat(Punct::At)?;
        let first_is_star = self.at(Punct::Star);
        let qualifier_ident = self.tok.identifier_name().map(str::to_string).filter(|_| matches!(self.tok.kind, TokenKind::Ident(_)));
        let first = self.parse_xml_name_part()?;
        let (qualifier, name) = if self.at(Punct::ColonColon) {
            let qualifier = if first_is_star {
                XmlQualifier::Any
            } else if let Some(name) = qualifier_ident {
                let span = self.span_from(start);
                XmlQualifier::Expr(self.ast.push_expr(Expr::Ident(name), span))
            } else {
                return Err(self.error_at_token(&self.tok, "expected a namespace name before '::'"));
            };
            self.bump()?;
            (Some(qualifier), self.parse_xml_name_part()?)
        } else {
            (None, first)
        };
        Ok(self.ast.push_expr(Expr::XmlName { attribute, qualifier, name }, self.span_from(start)))
    }

    fn parse_xml_name_part(&mut self) -> Result<XmlNamePart> {
        if self.eat(Punct::Star)? {
            return Ok(XmlNamePart::Star);
        }
        if self.eat(Punct::LBracket)? {
            let e = self.parse_expression(false)?;
            self.expect(Punct::RBracket)?;
            return Ok(XmlNamePart::Computed(e));
        }
        let Some(name) = self.tok.identifier_name().map(str::to_string) else {
            return Err(self.error_at_token(&self.tok, format!("expected an XML name but found {}", describe(&self.tok))));
        };
        self.bump()?;
        Ok(XmlNamePart::Ident(name))
    }

    /// An XML or XMLList literal, from its `<` (the current token).
    pub(super) fn parse_xml_literal(&mut self) -> Result<ExprId> {
        let start = self.tok.span;
        let token_slot = self.tokens.as_ref().map(Vec::len);
        if let Some(tokens) = &mut self.tokens {
            tokens.push(self.tok.clone());
        }
        let mut pos = start.start as usize;
        let list = self.unit(pos + 1) == Some(GT);
        if list {
            pos += 2;
        }
        let mut scan = Scan::default();
        // The name of each open element; `None` when it is computed.
        let mut open: Vec<Option<Vec<u16>>> = Vec::new();
        loop {
            if !list && open.is_empty() && !scan.text.is_empty() | !scan.parts.is_empty() {
                break;
            }
            let Some(c) = self.unit(pos) else { return Err(self.xml_error(pos, "unterminated XML literal")) };
            if c == LT {
                if self.unit(pos + 1) == Some(SLASH) {
                    if open.is_empty() {
                        if list && self.unit(pos + 2) == Some(GT) {
                            pos += 3;
                            break;
                        }
                        return Err(self.xml_error(pos, "a closing tag has no matching open tag"));
                    }
                    pos = self.xml_close_tag(pos, &mut open, &mut scan)?;
                } else if self.starts_with(pos, "<!--") {
                    pos = self.xml_comment(pos, &mut scan)?;
                } else if self.starts_with(pos, "<![CDATA[") {
                    pos = self.xml_through(pos, "]]>", &mut scan, "unterminated CDATA section")?;
                } else if self.starts_with(pos, "<?") {
                    pos = self.xml_instruction(pos, &mut scan)?;
                } else {
                    pos = self.xml_open_tag(pos, &mut open, &mut scan)?;
                }
            } else if c == LBRACE {
                if open.is_empty() && !list {
                    return Err(self.xml_error(pos, "expected an XML tag"));
                }
                scan.flush();
                let (e, after) = self.xml_embedded(pos)?;
                scan.parts.push(XmlPart::Content(e));
                pos = after;
            } else {
                if open.is_empty() && !list {
                    return Err(self.xml_error(pos, "expected an XML tag"));
                }
                pos = self.xml_text(pos, &mut scan)?;
            }
        }
        scan.flush();
        // Reads the token after the literal for the division goal.
        let (line, column) = self.lines.line_column(pos as u32);
        self.lex.seek(pos, line, column);
        self.prev_end = pos as u32;
        self.tok = self.next_significant()?;
        if let (Some(slot), Some(tokens)) = (token_slot, &mut self.tokens)
            && let Some(token) = tokens.get_mut(slot)
        {
            token.kind = TokenKind::XmlLiteral;
            token.span = Span::new(start.start, pos as u32);
        }
        Ok(self.ast.push_expr(Expr::XmlLiteral { parts: scan.parts, list }, Span::new(start.start, pos as u32)))
    }

    fn unit(&self, pos: usize) -> Option<u16> {
        self.src.get(pos).copied()
    }

    fn starts_with(&self, pos: usize, s: &str) -> bool {
        self.src.get(pos..).is_some_and(|rest| rest.iter().copied().take(s.len()).eq(s.encode_utf16()))
    }

    fn xml_error(&self, pos: usize, message: &str) -> crate::SyntaxError {
        self.error_at_offset(pos as u32, message)
    }

    /// Reads `{expression}` with `pos` at its `{`; returns the expression and the offset after `}`.
    fn xml_embedded(&mut self, pos: usize) -> Result<(ExprId, usize)> {
        let (line, column) = self.lines.line_column(pos as u32 + 1);
        self.lex.seek(pos + 1, line, column);
        self.tok = self.next_significant()?;
        let e = self.parse_expression(false)?;
        if !self.at(Punct::RBrace) {
            return Err(self.error_at_token(&self.tok, format!("expected '}}' but found {}", describe(&self.tok))));
        }
        Ok((e, self.tok.span.end as usize))
    }

    fn xml_copy(&self, from: usize, to: usize, scan: &mut Scan) {
        if let Some(units) = self.src.get(from..to) {
            scan.text.extend_from_slice(units);
        }
    }

    /// Copies from `pos` through the first `end`, which must be there.
    fn xml_through(&self, pos: usize, end: &str, scan: &mut Scan, message: &str) -> Result<usize> {
        let mut at = pos;
        while at < self.src.len() {
            if self.starts_with(at, end) {
                let after = at + end.len();
                self.xml_copy(pos, after, scan);
                return Ok(after);
            }
            at += 1;
        }
        Err(self.xml_error(pos, message))
    }

    fn xml_comment(&self, pos: usize, scan: &mut Scan) -> Result<usize> {
        let mut at = pos + 4;
        while at < self.src.len() {
            if self.starts_with(at, "--") {
                if self.unit(at + 2) != Some(GT) {
                    return Err(self.xml_error(at, "'--' cannot appear in an XML comment"));
                }
                self.xml_copy(pos, at + 3, scan);
                return Ok(at + 3);
            }
            at += 1;
        }
        Err(self.xml_error(pos, "unterminated XML comment"))
    }

    fn xml_instruction(&self, pos: usize, scan: &mut Scan) -> Result<usize> {
        if !self.unit(pos + 2).is_some_and(is_name_start) {
            return Err(self.xml_error(pos + 2, "invalid processing instruction name"));
        }
        self.xml_through(pos, "?>", scan, "unterminated processing instruction")
    }

    /// Reads a static name at `pos`, which must be one, and returns the offset after it.
    fn xml_static_name(&self, pos: usize) -> Result<usize> {
        if !self.unit(pos).is_some_and(is_name_start) {
            return Err(self.xml_error(pos, "invalid XML name"));
        }
        let mut at = pos + 1;
        while self.unit(at).is_some_and(is_name_part) {
            at += 1;
        }
        if !valid_xml_units(self.src.get(pos..at).unwrap_or_default()) {
            return Err(self.xml_error(pos, "invalid XML name"));
        }
        Ok(at)
    }

    fn xml_skip_space(&self, mut pos: usize, scan: &mut Scan) -> usize {
        let from = pos;
        while self.unit(pos).is_some_and(is_xml_space) {
            pos += 1;
        }
        self.xml_copy(from, pos, scan);
        pos
    }

    fn xml_open_tag(&mut self, pos: usize, open: &mut Vec<Option<Vec<u16>>>, scan: &mut Scan) -> Result<usize> {
        if open.len() >= self.max_depth {
            return Err(self.xml_error(pos, TOO_DEEP));
        }
        scan.text.push(LT);
        let mut at = pos + 1;
        let name = if self.unit(at) == Some(LBRACE) {
            scan.flush();
            let (e, after) = self.xml_embedded(at)?;
            scan.parts.push(XmlPart::Raw(e));
            at = after;
            None
        } else {
            let end = self.xml_static_name(at)?;
            self.xml_copy(at, end, scan);
            let name = self.src.get(at..end).map(<[u16]>::to_vec);
            at = end;
            name
        };
        loop {
            let before = at;
            at = self.xml_skip_space(at, scan);
            match self.unit(at) {
                None => return Err(self.xml_error(at, "unterminated XML tag")),
                Some(SLASH) if self.unit(at + 1) == Some(GT) => {
                    scan.text.extend_from_slice(&[SLASH, GT]);
                    return Ok(at + 2);
                }
                Some(GT) => {
                    scan.text.push(GT);
                    open.push(name);
                    return Ok(at + 1);
                }
                Some(LBRACE) => {
                    if at == before {
                        return Err(self.xml_error(at, "expected white space before attributes"));
                    }
                    scan.flush();
                    let (e, after) = self.xml_embedded(at)?;
                    scan.parts.push(XmlPart::Raw(e));
                    at = after;
                }
                Some(_) if at == before => return Err(self.xml_error(at, "expected white space before an attribute")),
                Some(_) => at = self.xml_attribute(at, scan)?,
            }
        }
    }

    /// Reads an attribute at `pos` and returns the offset after it.
    fn xml_attribute(&mut self, pos: usize, scan: &mut Scan) -> Result<usize> {
        let end = self.xml_static_name(pos)?;
        self.xml_copy(pos, end, scan);
        let at = self.xml_skip_space(end, scan);
        if self.unit(at) != Some(EQ) {
            return Err(self.xml_error(at, "expected '=' after an attribute name"));
        }
        scan.text.push(EQ);
        let at = self.xml_skip_space(at + 1, scan);
        match self.unit(at) {
            Some(LBRACE) => {
                scan.text.push(DQUOTE);
                scan.flush();
                let (e, after) = self.xml_embedded(at)?;
                scan.parts.push(XmlPart::Attribute(e));
                scan.text.push(DQUOTE);
                Ok(after)
            }
            Some(q @ (DQUOTE | SQUOTE)) => {
                let mut end = at + 1;
                loop {
                    match self.unit(end) {
                        None => return Err(self.xml_error(at, "unterminated attribute value")),
                        Some(c) if c == q => break,
                        Some(LT) => return Err(self.xml_error(end, "'<' cannot appear in an attribute value")),
                        Some(AMP) => end = self.xml_reference(end)?,
                        Some(_) => end += 1,
                    }
                }
                self.xml_copy(at, end + 1, scan);
                Ok(end + 1)
            }
            _ => Err(self.xml_error(at, "expected an attribute value")),
        }
    }

    fn xml_reference(&self, pos: usize) -> Result<usize> {
        match scan_reference(self.src, pos) {
            Some((after, _)) => Ok(after),
            None => Err(self.xml_error(pos, "invalid XML character or entity reference")),
        }
    }

    fn xml_close_tag(&mut self, pos: usize, open: &mut Vec<Option<Vec<u16>>>, scan: &mut Scan) -> Result<usize> {
        scan.text.extend_from_slice(&[LT, SLASH]);
        let mut at = pos + 2;
        let name = if self.unit(at) == Some(LBRACE) {
            scan.flush();
            let (e, after) = self.xml_embedded(at)?;
            scan.parts.push(XmlPart::Raw(e));
            at = after;
            None
        } else {
            let end = self.xml_static_name(at)?;
            self.xml_copy(at, end, scan);
            let name = self.src.get(at..end).map(<[u16]>::to_vec);
            at = end;
            name
        };
        at = self.xml_skip_space(at, scan);
        if self.unit(at) != Some(GT) {
            return Err(self.xml_error(at, "expected '>' at the end of a closing tag"));
        }
        scan.text.push(GT);
        if let (Some(Some(opened)), Some(closed)) = (open.last(), &name)
            && opened != closed
        {
            return Err(self.xml_error(pos, "the closing tag does not match the open tag"));
        }
        open.pop();
        Ok(at + 1)
    }

    fn xml_text(&self, pos: usize, scan: &mut Scan) -> Result<usize> {
        let mut at = pos;
        while let Some(c) = self.unit(at) {
            if c == LT || c == LBRACE {
                break;
            }
            at = if c == AMP { self.xml_reference(at)? } else { at + 1 };
        }
        self.xml_copy(pos, at, scan);
        Ok(at)
    }
}
