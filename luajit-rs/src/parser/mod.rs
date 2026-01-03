//! Lua lexer and parser.
//!
//! This module implements a complete Lua 5.1/5.2 compatible parser that
//! generates bytecode directly (one-pass compilation like LuaJIT).

mod lexer;
mod compiler;

pub use lexer::{Lexer, Token, TokenKind};
pub use compiler::Compiler;

use crate::value::{LuaError, LuaResult, GcRef, Proto};

/// Parse Lua source code and compile to bytecode
pub fn parse(source: &str, chunk_name: &str) -> LuaResult<Proto> {
    let lexer = Lexer::new(source);
    let mut compiler = Compiler::new(lexer, chunk_name);
    compiler.compile()
}

/// Parse and compile a Lua expression
pub fn parse_expression(source: &str) -> LuaResult<Proto> {
    let wrapped = format!("return {}", source);
    parse(&wrapped, "=expr")
}
