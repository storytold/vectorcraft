//! JavaScript for VectorCraft's scripts: an ECMAScript 5.1 lexer and parser
//! (<https://262.ecma-international.org/5.1/>), with the legacy forms of Annex B that older scripts
//! use (octal literals and escapes outside strict mode, function declarations in blocks, the
//! lenient regular expression patterns).
//!
//! [`parse`] reads a script into a [`Program`]: a syntax tree whose expressions, statements and
//! functions live in arenas and carry their source spans. A script that breaks the grammar or one
//! of its early-error rules (section 16: strict mode's restrictions, duplicate parameters in strict
//! code, invalid assignment targets, `break` and `continue` without a target, regular expression
//! literals that are not valid) gives a [`SyntaxError`] with its line and column.
//!
//! With [`Options::preprocessor_lines`], the lexer also reads the preprocessor lines scripts begin
//! with (`#include`, `#includepath`, `#script`, `#strict`, `#target`, `#targetengine`) into
//! [`Directive`]s. They are kept in [`Program::directives`] and never run.
//!
//! The lexer reads the source as UTF-16 code units, as JavaScript does: columns and spans count
//! code units. Nesting is counted: input nested deeper than [`Options::max_depth`] gives a
//! `SyntaxError` ("nesting too deep") instead of exhausting the stack, binary operators, `if` …
//! `else if` chains, labels and prefix operators are read with loops, and the syntax tree is
//! dropped without recursion.
//!
//! A [`Realm`] runs scripts. It compiles the syntax tree to bytecode, checks the bytecode with a
//! verifier, and runs it on an interpreter that keeps JavaScript calls on its own frame stack, so
//! deep recursion in a script does not use the native stack. Objects and environments live in an
//! arena with a mark-and-sweep collector; strings built by repeated `+` are flattened and dropped
//! with loops. The realm provides the language (sections 10 to 14 of ECMA-262 5.1) and a minimal
//! set of built-ins: `Object` and `Object.prototype`'s basic methods, `Function` with
//! `Function.prototype.call` and `apply`, `eval`, and the error constructors. Every run is bounded
//! by [`Limits`]: instructions run (fuel), heap bytes, frames, nested host calls, and an
//! [`Interrupt`] flag another thread can set. A run that reaches a limit ends with
//! [`RunErrorKind::Limit`], which a script's `try` cannot catch.
#![forbid(unsafe_code)]

pub mod ast;
mod builtins;
mod bytecode;
mod chars;
mod compiler;
mod error;
mod heap;
mod interp;
mod lexer;
mod number;
mod parser;
mod realm;
mod regexp;
mod token;
mod value;

pub use ast::{
    AssignOp, Ast, BinaryOp, CatchClause, Expr, ExprId, ForInTarget, ForInit, Function, FunctionId, FunctionKind, LogicalOp, Name, Program, Property,
    PropertyKey, PropertyValue, Stmt, StmtId, SwitchCase, UnaryOp, UpdateOp, VarDecl,
};
pub use error::{SyntaxError, TOO_DEEP, TOO_LONG};
pub use lexer::{Goal, Lexer};
pub use number::to_string as number_to_string;
pub use realm::{Interrupt, JsValue, LimitKind, Limits, Realm, RunError, RunErrorKind};
pub use token::{Directive, DirectiveName, Keyword, Punct, Span, Token, TokenKind};

/// How a script is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The script is strict mode code from its start, as code passed to a strict `eval` is.
    pub strict: bool,
    /// Read preprocessor lines (`#target …` and the other directives) into [`Directive`]s. Off,
    /// a `#` is an unexpected character, as ECMA-262 has it.
    pub preprocessor_lines: bool,
    /// How deeply statements, expressions, functions and regular expression groups may nest.
    pub max_depth: usize,
    /// The most UTF-16 code units a source may hold.
    pub max_source_units: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { strict: false, preprocessor_lines: false, max_depth: DEFAULT_MAX_DEPTH, max_source_units: 64 << 20 }
    }
}

/// The default [`Options::max_depth`]: deeper than scripts nest, and shallow enough for the stack
/// of a wasm build or a secondary thread.
pub const DEFAULT_MAX_DEPTH: usize = 200;

/// Parses `source` as a script.
pub fn parse(source: &str, options: &Options) -> Result<Program, SyntaxError> {
    let units = to_utf16(source, options)?;
    parse_utf16(&units, options)
}

/// Parses `source`, in UTF-16 code units, as a script.
pub fn parse_utf16(source: &[u16], options: &Options) -> Result<Program, SyntaxError> {
    check_length(source.len(), options)?;
    parser::Parser::new(source, options, false)?.parse_program().map(|(program, _)| program)
}

/// The tokens of `source`, in order, as the parser reads them: preprocessor lines included, and
/// each `/` read as division or as the start of a regular expression literal as the grammar
/// decides. The source must be a valid script.
pub fn tokenize(source: &str, options: &Options) -> Result<Vec<Token>, SyntaxError> {
    let units = to_utf16(source, options)?;
    check_length(units.len(), options)?;
    parser::Parser::new(&units, options, true)?.parse_program().map(|(_, tokens)| tokens)
}

fn to_utf16(source: &str, options: &Options) -> Result<Vec<u16>, SyntaxError> {
    // A source's UTF-16 length is at most its UTF-8 length; count only a source that may be too
    // long.
    if source.len() > options.max_source_units {
        check_length(source.encode_utf16().count(), options)?;
    }
    Ok(source.encode_utf16().collect())
}

fn check_length(units: usize, options: &Options) -> Result<(), SyntaxError> {
    if units > options.max_source_units || u32::try_from(units).is_err() {
        return Err(SyntaxError { message: TOO_LONG.into(), line: 1, column: 1, offset: 0 });
    }
    Ok(())
}

/// Where each line of a source starts, for turning offsets into lines and columns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineTable {
    starts: Vec<u32>,
    /// The source ends with a line terminator.
    ends_with_break: bool,
    empty: bool,
}

impl LineTable {
    pub(crate) fn new(src: &[u16]) -> Self {
        let mut starts = vec![0];
        let mut i = 0;
        while let Some(&u) = src.get(i) {
            i += 1;
            let terminator = match u {
                0x0D if src.get(i) == Some(&0x0A) => {
                    i += 1;
                    true
                }
                0x0A | 0x0D | 0x2028 | 0x2029 => true,
                _ => false,
            };
            if terminator {
                starts.push(u32::try_from(i).unwrap_or(u32::MAX));
            }
        }
        let ends_with_break = src.last().is_some_and(|&u| matches!(u, 0x0A | 0x0D | 0x2028 | 0x2029));
        Self { starts, ends_with_break, empty: src.is_empty() }
    }

    pub(crate) fn line_column(&self, offset: u32) -> (u32, u32) {
        let line = self.starts.partition_point(|&s| s <= offset).max(1);
        let start = self.starts.get(line - 1).copied().unwrap_or(0);
        (u32::try_from(line).unwrap_or(u32::MAX), offset.saturating_sub(start) + 1)
    }

    pub(crate) fn line_count(&self) -> u32 {
        if self.empty {
            return 0;
        }
        let breaks = self.starts.len() - 1;
        u32::try_from(breaks + usize::from(!self.ends_with_break)).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod interp_tests;
#[cfg(test)]
mod tests;
