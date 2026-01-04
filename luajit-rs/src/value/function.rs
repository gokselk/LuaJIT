//! Lua function types: closures, prototypes, and upvalues.
//!
//! A Proto (prototype) contains the compiled bytecode for a function.
//! A Closure combines a Proto with captured upvalues.
//! Upvalues are variables from enclosing scopes.

use super::{GcHeader, GcRef, Value, LuaString, Table};
use crate::bytecode::Instruction;
use smallvec::SmallVec;
use std::cell::{Cell, RefCell};

/// A function prototype (compiled function).
pub struct Proto {
    /// GC header
    pub gc: GcHeader,
    /// Bytecode instructions
    pub code: Vec<Instruction>,
    /// Constants used by the function
    pub constants: Vec<Value>,
    /// String constants (stored as raw bytes, interned at runtime)
    pub string_constants: Vec<Vec<u8>>,
    /// Nested function prototypes (GC-allocated at load time)
    pub protos: Vec<GcRef<Proto>>,
    /// Child prototypes (compile-time storage, moved to protos at load time)
    pub child_protos: Vec<Box<Proto>>,
    /// Upvalue descriptors
    pub upvalues: Vec<UpvalueDesc>,
    /// Line number info (bytecode index -> line number)
    pub lineinfo: Vec<u32>,
    /// Local variable debug info
    pub locvars: Vec<LocVar>,
    /// Upvalue names (debug info)
    pub upvalue_names: Vec<Option<GcRef<LuaString>>>,

    /// Number of fixed parameters
    pub num_params: u8,
    /// Is this function vararg?
    pub is_vararg: bool,
    /// Maximum stack size needed
    pub max_stack_size: u8,
    /// Number of upvalues
    pub num_upvalues: u8,

    /// Source file name
    pub source: Option<GcRef<LuaString>>,
    /// First line of definition
    pub line_defined: u32,
    /// Last line of definition
    pub last_line_defined: u32,
}

/// Upvalue descriptor - describes how to find an upvalue
#[derive(Debug, Clone, Copy)]
pub struct UpvalueDesc {
    /// Is this upvalue in the enclosing function's stack (vs another upvalue)?
    pub in_stack: bool,
    /// Index: if in_stack, the stack slot; otherwise, the upvalue index
    pub index: u8,
    /// Name (optional, for debug)
    pub name: Option<u32>, // Index into upvalue_names
}

/// Local variable debug info
#[derive(Debug, Clone)]
pub struct LocVar {
    /// Variable name
    pub name: Option<GcRef<LuaString>>,
    /// First bytecode where variable is active
    pub start_pc: u32,
    /// Last bytecode where variable is active
    pub end_pc: u32,
}

impl Proto {
    /// Create a new empty prototype
    pub fn new() -> Self {
        Self {
            gc: GcHeader::new(10), // LuaType::Proto
            code: Vec::new(),
            constants: Vec::new(),
            string_constants: Vec::new(),
            protos: Vec::new(),
            child_protos: Vec::new(),
            upvalues: Vec::new(),
            lineinfo: Vec::new(),
            locvars: Vec::new(),
            upvalue_names: Vec::new(),
            num_params: 0,
            is_vararg: false,
            max_stack_size: 2, // Minimum for proper operation
            num_upvalues: 0,
            source: None,
            line_defined: 0,
            last_line_defined: 0,
        }
    }

    /// Add a string constant (from UTF-8 str) and return its index
    pub fn add_string_constant(&mut self, s: &str) -> usize {
        self.add_string_constant_bytes(s.as_bytes())
    }

    /// Add a string constant (from raw bytes) and return its index
    pub fn add_string_constant_bytes(&mut self, bytes: &[u8]) -> usize {
        // Check if string already exists
        for (i, existing) in self.string_constants.iter().enumerate() {
            if existing == bytes {
                return i;
            }
        }
        let idx = self.string_constants.len();
        self.string_constants.push(bytes.to_vec());
        idx
    }

    /// Get the line number for a given bytecode index
    pub fn get_line(&self, pc: usize) -> Option<u32> {
        self.lineinfo.get(pc).copied()
    }

    /// Add a constant and return its index
    pub fn add_constant(&mut self, value: Value) -> usize {
        // Check if constant already exists
        for (i, k) in self.constants.iter().enumerate() {
            if k.raw_eq(&value) {
                return i;
            }
        }
        let idx = self.constants.len();
        self.constants.push(value);
        idx
    }

    /// Add bytecode and return its index
    pub fn add_instruction(&mut self, instr: Instruction, line: u32) -> usize {
        let idx = self.code.len();
        self.code.push(instr);
        self.lineinfo.push(line);
        idx
    }
}

impl Default for Proto {
    fn default() -> Self {
        Self::new()
    }
}

/// An upvalue - a captured variable from an enclosing scope.
pub struct Upvalue {
    /// GC header
    pub gc: GcHeader,
    /// The value storage (used when closed)
    value: Cell<Value>,
    /// If open, contains the stack index; if closed, this is None
    stack_index: Cell<Option<usize>>,
    /// Link to next upvalue in the open upvalue list
    pub next: Cell<Option<GcRef<Upvalue>>>,
}

impl Upvalue {
    /// Create a new open upvalue pointing to a stack index
    pub fn new_open(index: usize) -> Self {
        Self {
            gc: GcHeader::new(11), // LuaType::Upvalue
            value: Cell::new(Value::nil()),
            stack_index: Cell::new(Some(index)),
            next: Cell::new(None),
        }
    }

    /// Create a closed upvalue with a value
    pub fn new_closed(value: Value) -> Self {
        Self {
            gc: GcHeader::new(11),
            value: Cell::new(value),
            stack_index: Cell::new(None),
            next: Cell::new(None),
        }
    }

    /// Check if upvalue is open (still on stack)
    pub fn is_open(&self) -> bool {
        self.stack_index.get().is_some()
    }

    /// Get the upvalue's current value (for closed upvalues only)
    /// For open upvalues, use get_from_stack instead
    pub fn get(&self) -> Value {
        self.value.get()
    }

    /// Get value from stack if open, otherwise from internal storage
    pub fn get_from_stack(&self, stack: &[Value]) -> Value {
        if let Some(index) = self.stack_index.get() {
            if index < stack.len() {
                stack[index]
            } else {
                Value::nil()
            }
        } else {
            self.value.get()
        }
    }

    /// Set the upvalue's value
    pub fn set(&self, value: Value) {
        self.value.set(value);
    }

    /// Set value in stack if open, otherwise in internal storage
    pub fn set_in_stack(&self, stack: &mut [Value], value: Value) {
        if let Some(index) = self.stack_index.get() {
            if index < stack.len() {
                stack[index] = value;
            }
        } else {
            self.value.set(value);
        }
    }

    /// Close the upvalue (copy value from stack to internal storage)
    pub fn close_with_stack(&self, stack: &[Value]) {
        if let Some(index) = self.stack_index.get() {
            if index < stack.len() {
                self.value.set(stack[index]);
            }
            self.stack_index.set(None);
        }
    }

    /// Get the stack index (if open)
    pub fn get_stack_index(&self) -> Option<usize> {
        self.stack_index.get()
    }

    /// Alias for get_stack_index
    pub fn stack_index(&self) -> Option<usize> {
        self.stack_index.get()
    }
}

/// A Lua closure - a function with its captured upvalues.
pub struct Closure {
    /// GC header
    pub gc: GcHeader,
    /// The function prototype
    pub proto: GcRef<Proto>,
    /// Captured upvalues
    pub upvalues: SmallVec<[GcRef<Upvalue>; 4]>,
    /// Environment table (for _ENV)
    pub env: Option<GcRef<Table>>,
}

impl std::fmt::Debug for Closure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Closure")
            .field("proto", &"<proto>")
            .field("upvalues_count", &self.upvalues.len())
            .field("has_env", &self.env.is_some())
            .finish()
    }
}

impl Closure {
    /// Create a new closure from a prototype
    pub fn new(proto: GcRef<Proto>, env: Option<GcRef<Table>>) -> Self {
        let num_upvalues = unsafe { (*proto.as_ptr()).num_upvalues as usize };
        Self {
            gc: GcHeader::new(6), // LuaType::Function
            proto,
            upvalues: SmallVec::with_capacity(num_upvalues),
            env,
        }
    }

    /// Get an upvalue by index
    pub fn get_upvalue(&self, index: usize) -> Option<Value> {
        self.upvalues.get(index).map(|uv| unsafe { (*uv.as_ptr()).get() })
    }

    /// Set an upvalue by index
    pub fn set_upvalue(&self, index: usize, value: Value) {
        if let Some(uv) = self.upvalues.get(index) {
            unsafe { (*uv.as_ptr()).set(value) }
        }
    }
}

/// Native (Rust) function signature
pub type NativeFn = fn(&mut crate::vm::State) -> Result<usize, super::LuaError>;

/// A native (Rust) function wrapper
pub struct NativeFunction {
    /// GC header
    pub gc: GcHeader,
    /// The native function pointer
    pub func: NativeFn,
    /// Number of upvalues
    pub num_upvalues: u8,
    /// Upvalues for this native function
    pub upvalues: SmallVec<[Value; 2]>,
}

impl NativeFunction {
    /// Create a new native function
    pub fn new(func: NativeFn) -> Self {
        Self {
            gc: GcHeader::new(6),
            func,
            num_upvalues: 0,
            upvalues: SmallVec::new(),
        }
    }

    /// Create a native function with upvalues
    pub fn with_upvalues(func: NativeFn, upvalues: SmallVec<[Value; 2]>) -> Self {
        Self {
            gc: GcHeader::new(6),
            func,
            num_upvalues: upvalues.len() as u8,
            upvalues,
        }
    }
}

/// A Lua function - either a closure or a native function.
pub enum Function {
    Lua(Closure),
    Native(NativeFunction),
}

impl Function {
    /// Create a new Lua closure
    pub fn lua(proto: GcRef<Proto>, env: Option<GcRef<Table>>) -> Self {
        Function::Lua(Closure::new(proto, env))
    }

    /// Create a new native function
    pub fn native(func: NativeFn) -> Self {
        Function::Native(NativeFunction::new(func))
    }

    /// Check if this is a Lua function
    pub fn is_lua(&self) -> bool {
        matches!(self, Function::Lua(_))
    }

    /// Check if this is a native function
    pub fn is_native(&self) -> bool {
        matches!(self, Function::Native(_))
    }

    /// Get as Lua closure
    pub fn as_lua(&self) -> Option<&Closure> {
        match self {
            Function::Lua(c) => Some(c),
            _ => None,
        }
    }

    /// Get as Lua closure (mutable)
    pub fn as_lua_mut(&mut self) -> Option<&mut Closure> {
        match self {
            Function::Lua(c) => Some(c),
            _ => None,
        }
    }

    /// Get as native function
    pub fn as_native(&self) -> Option<&NativeFunction> {
        match self {
            Function::Native(n) => Some(n),
            _ => None,
        }
    }
}

impl std::fmt::Debug for Function {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Function::Lua(_) => write!(f, "<lua function>"),
            Function::Native(_) => write!(f, "<native function>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proto_creation() {
        let proto = Proto::new();
        assert!(proto.code.is_empty());
        assert!(proto.constants.is_empty());
        assert_eq!(proto.num_params, 0);
        assert!(!proto.is_vararg);
    }

    #[test]
    fn test_upvalue_open_close() {
        let mut stack = vec![Value::integer(42)];
        let uv = Upvalue::new_open(0);

        assert!(uv.is_open());
        assert_eq!(uv.get_from_stack(&stack).as_integer(), Some(42));

        uv.set_in_stack(&mut stack, Value::integer(100));
        assert_eq!(stack[0].as_integer(), Some(100));

        uv.close_with_stack(&stack);
        assert!(!uv.is_open());
        assert_eq!(uv.get().as_integer(), Some(100));
    }
}
