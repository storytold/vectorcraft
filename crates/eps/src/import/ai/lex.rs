//! The scanner of the editing data: PostScript-like tokens, read line by line.
//!
//! Backed by `craft_eps::ai_lex`.

pub(super) use craft_eps::ai_lex::{Lexer, Tok, Token, number};
