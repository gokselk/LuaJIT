//! Core Lua value types and representation.
//!
//! This module implements the tagged value representation similar to LuaJIT's TValue.
//! We use NaN-boxing for efficient value representation on 64-bit systems.

mod nanbox;
pub mod table;
pub mod string;
mod function;
mod userdata;

pub use nanbox::Value;
pub use table::{Table, GcHeader};
pub use string::{LuaString, StringInterner};
pub use function::{Function, Closure, Upvalue, Proto, UpvalueDesc, LocVar, NativeFn, NativeFunction};
pub use userdata::Userdata;

use std::fmt;
use std::hash::{Hash, Hasher};
use ordered_float::OrderedFloat;

/// Lua value types (matches LuaJIT's type tags)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LuaType {
    Nil = 0,
    Boolean = 1,
    LightUserdata = 2,
    Number = 3,
    String = 4,
    Table = 5,
    Function = 6,
    Userdata = 7,
    Thread = 8,
    // Internal types
    Cdata = 9,
    Proto = 10,
    Upvalue = 11,
}

impl LuaType {
    pub fn name(self) -> &'static str {
        match self {
            LuaType::Nil => "nil",
            LuaType::Boolean => "boolean",
            LuaType::LightUserdata => "userdata",
            LuaType::Number => "number",
            LuaType::String => "string",
            LuaType::Table => "table",
            LuaType::Function => "function",
            LuaType::Userdata => "userdata",
            LuaType::Thread => "thread",
            LuaType::Cdata => "cdata",
            LuaType::Proto => "proto",
            LuaType::Upvalue => "upvalue",
        }
    }
}

impl fmt::Display for LuaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Result type for Lua operations
pub type LuaResult<T> = Result<T, LuaError>;

/// Lua runtime errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum LuaError {
    #[error("attempt to perform arithmetic on a {0} value")]
    ArithmeticError(LuaType),

    #[error("attempt to compare {0} with {1}")]
    CompareError(LuaType, LuaType),

    #[error("attempt to concatenate a {0} value")]
    ConcatError(LuaType),

    #[error("attempt to get length of a {0} value")]
    LengthError(LuaType),

    #[error("attempt to index a {0} value")]
    IndexError(LuaType),

    #[error("attempt to call a {0} value")]
    CallError(LuaType),

    #[error("{0}")]
    RuntimeError(String),

    #[error("syntax error: {0}")]
    SyntaxError(String),

    #[error("memory allocation error")]
    MemoryError,

    #[error("stack overflow")]
    StackOverflow,

    #[error("type error: expected {expected}, got {got}")]
    TypeError { expected: LuaType, got: LuaType },

    #[error("bad argument #{arg} to '{func}' ({msg})")]
    ArgumentError { func: String, arg: usize, msg: String },
}

/// A garbage-collected reference to a Lua object
#[derive(Debug)]
pub struct GcRef<T> {
    ptr: *mut T,
}

// Manual Clone/Copy impl without T: Copy bound since we're just wrapping a pointer
impl<T> Clone for GcRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for GcRef<T> {}

impl<T> GcRef<T> {
    pub fn new(ptr: *mut T) -> Self {
        Self { ptr }
    }

    pub fn as_ptr(&self) -> *mut T {
        self.ptr
    }

    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    /// # Safety
    /// The pointer must be valid and properly aligned
    pub unsafe fn as_ref(&self) -> &T {
        &*self.ptr
    }

    /// # Safety
    /// The pointer must be valid and properly aligned
    pub unsafe fn as_mut(&mut self) -> &mut T {
        &mut *self.ptr
    }
}

impl<T> PartialEq for GcRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl<T> Eq for GcRef<T> {}

impl<T> Hash for GcRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.ptr.hash(state);
    }
}

// Safety: GcRef is essentially a wrapper around a raw pointer
// The actual thread safety is managed by the GC
unsafe impl<T: Send> Send for GcRef<T> {}
unsafe impl<T: Sync> Sync for GcRef<T> {}
