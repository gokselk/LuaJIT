//! LuaJIT-RS: A Rust port of LuaJIT using Cranelift for JIT compilation.
//!
//! This crate provides a Lua 5.1 compatible interpreter with JIT compilation
//! using the Cranelift code generator as a replacement for DynASM.
//!
//! # Architecture
//!
//! The implementation follows a similar structure to LuaJIT:
//!
//! - **Value representation**: NaN-boxing for efficient 64-bit tagged values
//! - **Bytecode**: Register-based instruction set similar to LuaJIT
//! - **Parser**: Single-pass compiler generating bytecode directly
//! - **Interpreter**: Efficient bytecode interpreter
//! - **JIT**: Trace-based compiler using Cranelift for code generation
//! - **GC**: Mark-and-sweep garbage collector
//!
//! # Example
//!
//! ```no_run
//! use luajit_rs::vm::State;
//! use luajit_rs::stdlib;
//!
//! let mut state = State::new();
//! stdlib::register_all(&mut state);
//!
//! // Load and run a Lua script
//! let func = state.load_string("print('Hello, World!')", "example").unwrap();
//! state.push(luajit_rs::value::Value::function(func)).unwrap();
//! state.call(0, 0).unwrap();
//! ```

#![allow(clippy::new_without_default)]
#![allow(clippy::should_implement_trait)]

pub mod value;
pub mod bytecode;
pub mod parser;
pub mod vm;
pub mod jit;
pub mod gc;
pub mod stdlib;

// Re-exports for convenience
pub use value::{Value, LuaError, LuaResult, LuaType};
pub use vm::State;
pub use parser::parse;

/// Version string
pub const VERSION: &str = "LuaJIT-RS 0.1.0 (Lua 5.1 compatible)";

/// Lua version number
pub const LUA_VERSION_NUM: i32 = 501;

/// Create a new Lua state with standard libraries loaded
pub fn new_state() -> State {
    let mut state = State::new();
    stdlib::register_all(&mut state);
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(VERSION.contains("LuaJIT-RS"));
    }

    #[test]
    fn test_new_state() {
        let state = new_state();
        assert_eq!(state.get_top(), 0);
    }

    #[test]
    fn test_parse_simple() {
        let proto = parse("local x = 1 + 2", "test").unwrap();
        assert!(!proto.code.is_empty());
    }
}
