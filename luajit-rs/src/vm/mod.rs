//! Lua Virtual Machine.
//!
//! This module implements the bytecode interpreter that executes compiled Lua code.

mod state;
mod interpreter;
mod stack;
mod call;

pub use state::State;
pub use interpreter::Interpreter;
pub use stack::Stack;
pub use call::{CallInfo, CallFrame, CallStack};
