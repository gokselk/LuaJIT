//! Lua state - the main VM context.

use super::{Stack, CallStack, CallFrame};
use crate::value::{
    Value, LuaResult, LuaError, GcRef, LuaString, Table, Function, Closure, Proto, Upvalue,
};
use crate::value::string::StringInterner;
use crate::gc::GarbageCollector;
use std::collections::HashMap;

/// The main Lua state
pub struct State {
    /// Value stack
    pub stack: Stack,
    /// Call stack
    pub call_stack: CallStack,
    /// Global environment
    pub globals: GcRef<Table>,
    /// Registry (internal Lua table)
    pub registry: GcRef<Table>,
    /// String interner
    pub strings: StringInterner,
    /// Garbage collector
    pub gc: GarbageCollector,
    /// Metatables for basic types
    pub metatables: [Option<GcRef<Table>>; 8],
    /// Error handler
    pub error_handler: Option<GcRef<Function>>,
    /// Current error (if any)
    pub current_error: Option<LuaError>,
    /// Hook mask (for debugging)
    pub hook_mask: u8,
    /// Hook function
    pub hook: Option<GcRef<Function>>,
    /// Base count for hook
    pub base_hook_count: u32,
    /// Current hook count
    pub hook_count: u32,
    /// Allow hooks
    pub allow_hook: bool,
    /// Status
    pub status: ThreadStatus,
    /// Head of the open upvalue list (sorted by stack slot address, highest first)
    pub open_upvalues: Option<GcRef<Upvalue>>,
    /// Whether the current call is a method call (for error formatting)
    pub is_method_call: bool,
}

/// Thread/coroutine status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatus {
    Ok,
    Yielded,
    RuntimeError,
    SyntaxError,
    MemoryError,
    ErrorInError,
}

impl State {
    /// Create a new Lua state
    pub fn new() -> Self {
        let mut gc = GarbageCollector::new();

        // Create global table
        let globals = gc.alloc(Table::new());
        let registry = gc.alloc(Table::new());

        let mut state = Self {
            stack: Stack::new(),
            call_stack: CallStack::new(),
            globals,
            registry,
            strings: StringInterner::new(),
            gc,
            metatables: [None; 8],
            error_handler: None,
            current_error: None,
            hook_mask: 0,
            hook: None,
            base_hook_count: 0,
            hook_count: 0,
            allow_hook: true,
            status: ThreadStatus::Ok,
            open_upvalues: None,
            is_method_call: false,
        };

        // Initialize standard globals
        state.init_globals();

        state
    }

    /// Initialize standard global variables
    fn init_globals(&mut self) {
        // Set _G to point to globals
        unsafe {
            let globals_val = Value::table(self.globals);
            (*self.globals.as_ptr()).set(
                Value::from_raw_bits(self.intern_string("_G").raw_bits()),
                globals_val,
            );

            // Set _VERSION
            let version = self.intern_string("Lua 5.1");
            (*self.globals.as_ptr()).set(
                Value::from_raw_bits(self.intern_string("_VERSION").raw_bits()),
                version,
            );
        }
    }

    /// Intern a string
    pub fn intern_string(&mut self, s: &str) -> Value {
        let ptr = self.strings.intern_str(s);
        Value::string(GcRef::new(ptr))
    }

    /// Intern raw bytes as a Lua string
    pub fn intern_bytes(&mut self, bytes: &[u8]) -> Value {
        let ptr = self.strings.intern(bytes);
        Value::string(GcRef::new(ptr))
    }

    /// Get a global variable
    pub fn get_global(&mut self, name: &str) -> Value {
        let key = self.intern_string(name);
        unsafe { (*self.globals.as_ptr()).get(&key) }
    }

    /// Set a global variable
    pub fn set_global(&mut self, name: &str, value: Value) {
        let key = self.intern_string(name);
        unsafe {
            (*self.globals.as_ptr()).set(key, value);
        }
    }

    /// Push a value onto the stack
    pub fn push(&mut self, value: Value) -> LuaResult<()> {
        self.stack.push(value)
    }

    /// Pop a value from the stack
    pub fn pop(&mut self) -> Value {
        self.stack.pop()
    }

    /// Get stack top (number of elements in current frame)
    pub fn get_top(&self) -> usize {
        let base = self.stack.base();
        let top = self.stack.top();
        if top > base { top - base } else { 0 }
    }

    /// Set stack top (relative to base)
    pub fn set_top(&mut self, n: usize) {
        let base = self.stack.base();
        self.stack.set_top(base + n);
    }

    /// Get value at stack index (1-based relative to base, negative from top)
    pub fn get_value(&self, index: i32) -> Value {
        let base = self.stack.base();
        let top = self.stack.top();
        let abs_idx = if index > 0 {
            // Positive: 1-based from base
            base + (index as usize) - 1
        } else if index < 0 {
            // Negative: from top
            (top as i32 + index) as usize
        } else {
            return Value::nil();
        };
        if abs_idx < top {
            self.stack.get(abs_idx)
        } else {
            Value::nil()
        }
    }

    /// Set value at stack index (1-based relative to base, negative from top)
    pub fn set_value(&mut self, index: i32, value: Value) {
        let base = self.stack.base();
        let top = self.stack.top();
        let abs_idx = if index > 0 {
            base + (index as usize) - 1
        } else if index < 0 {
            (top as i32 + index) as usize
        } else {
            return;
        };
        if abs_idx < top {
            self.stack.set(abs_idx, value);
        }
    }

    /// Convert relative index to absolute (1-based result for compatibility)
    fn abs_index(&self, index: i32) -> i32 {
        let base = self.stack.base();
        let top = self.stack.top();
        if index > 0 {
            base as i32 + index
        } else if index == 0 {
            0
        } else {
            top as i32 + index + 1
        }
    }

    /// Type check helpers
    pub fn is_nil(&self, index: i32) -> bool {
        self.get_value(index).is_nil()
    }

    pub fn is_boolean(&self, index: i32) -> bool {
        self.get_value(index).is_boolean()
    }

    pub fn is_number(&self, index: i32) -> bool {
        self.get_value(index).is_number()
    }

    pub fn is_string(&self, index: i32) -> bool {
        self.get_value(index).is_string()
    }

    pub fn is_table(&self, index: i32) -> bool {
        self.get_value(index).is_table()
    }

    pub fn is_function(&self, index: i32) -> bool {
        self.get_value(index).is_function()
    }

    /// Get value as boolean (with Lua truthiness)
    pub fn to_boolean(&self, index: i32) -> bool {
        self.get_value(index).is_truthy()
    }

    /// Get value as number (with string coercion)
    pub fn to_number(&self, index: i32) -> Option<f64> {
        self.get_value(index).coerce_to_number()
    }

    /// Get value as integer (with string coercion)
    pub fn to_integer(&self, index: i32) -> Option<i32> {
        self.get_value(index).coerce_to_integer()
    }

    /// Create an argument error, checking if this is a method call for arg 1
    /// For method calls, arg 1 becomes a "self" error, and other args are renumbered
    pub fn arg_error(&self, func: &str, arg: usize, msg: &str) -> LuaError {
        if self.is_method_call {
            if arg == 1 {
                LuaError::SelfError {
                    func: func.to_string(),
                    msg: msg.to_string(),
                }
            } else {
                // For method calls, arguments are shifted by -1
                LuaError::ArgumentError {
                    func: func.to_string(),
                    arg: arg - 1,
                    msg: msg.to_string(),
                }
            }
        } else {
            LuaError::ArgumentError {
                func: func.to_string(),
                arg,
                msg: msg.to_string(),
            }
        }
    }

    /// Get value as string (with number-to-string coercion)
    /// Returns the string content if available
    pub fn to_lua_string(&mut self, index: i32) -> Option<String> {
        let val = self.get_value(index);
        if let Some(s) = val.as_string() {
            let lua_str = unsafe { &*s.as_ptr() };
            return lua_str.as_str().map(|s| s.to_string());
        }
        // Coerce number to string
        if let Some(n) = val.as_number() {
            // Format number as Lua does
            let i = n as i64;
            if (i as f64) == n && n.is_finite() {
                return Some(format!("{}", i));
            } else {
                return Some(format!("{}", n));
            }
        }
        None
    }

    /// Create a new table
    pub fn create_table(&mut self, narr: usize, nrec: usize) -> GcRef<Table> {
        self.gc.alloc(Table::with_capacity(narr, nrec))
    }

    /// Push a new table onto the stack
    pub fn push_table(&mut self, narr: usize, nrec: usize) -> LuaResult<()> {
        let table = self.create_table(narr, nrec);
        self.push(Value::table(table))
    }

    /// Load and compile a chunk (source code or bytecode)
    pub fn load_string(&mut self, source: &str, chunk_name: &str) -> LuaResult<GcRef<Function>> {
        // Check if this is bytecode (starts with our magic)
        let bytes = source.as_bytes();
        if bytes.starts_with(crate::stdlib::string::BYTECODE_MAGIC) {
            return self.load_bytecode(bytes);
        }

        let proto = crate::parser::parse(source, chunk_name)?;
        let source_str = self.intern_string(chunk_name);
        let source_ref = source_str.as_string().unwrap();
        let proto_ref = self.allocate_proto_tree_with_source(proto, Some(source_ref));
        let closure = Closure::new(proto_ref, Some(self.globals));
        let func = Function::Lua(closure);
        Ok(self.gc.alloc(func))
    }

    /// Load bytecode from a binary buffer
    pub fn load_bytecode(&mut self, bytes: &[u8]) -> LuaResult<GcRef<Function>> {
        let proto = crate::stdlib::string::load_proto(bytes)?;
        let proto_ref = self.allocate_proto_tree_with_source(proto, None);
        let closure = Closure::new(proto_ref, Some(self.globals));
        let func = Function::Lua(closure);
        Ok(self.gc.alloc(func))
    }

    /// Recursively allocate a prototype and all its child protos with source set on main proto
    fn allocate_proto_tree_with_source(&mut self, mut proto: Proto, source: Option<GcRef<LuaString>>) -> GcRef<Proto> {
        // Set source on the main proto
        if let Some(src) = source {
            proto.source = Some(src);
        }
        // Recursively allocate child protos first (bottom-up), propagating source
        for child_proto in proto.child_protos.drain(..) {
            let child_ref = self.allocate_proto_tree_with_source(*child_proto, proto.source);
            proto.protos.push(child_ref);
        }
        // Now allocate this proto
        self.gc.alloc(proto)
    }

    /// Call a function on the stack
    /// Note: The value at func position doesn't need to be a function -
    /// if it has a __call metamethod, that will be invoked instead.
    pub fn call(&mut self, nargs: usize, nresults: i32) -> LuaResult<()> {
        let func_idx = self.stack.top() - nargs - 1;
        // Note: Don't check is_function() here - the interpreter handles
        // __call metamethods for tables and userdata

        // Run interpreter (it will handle metamethods for non-function values)
        let mut interp = super::Interpreter::new(self);
        interp.call(func_idx, nargs, nresults)
    }

    /// Protected call (pcall)
    pub fn pcall(&mut self, nargs: usize, nresults: i32) -> LuaResult<bool> {
        match self.call(nargs, nresults) {
            Ok(()) => Ok(true),
            Err(e) => {
                self.current_error = Some(e);
                Ok(false)
            }
        }
    }

    /// Get current error
    pub fn get_error(&self) -> Option<&LuaError> {
        self.current_error.as_ref()
    }

    /// Clear current error
    pub fn clear_error(&mut self) {
        self.current_error = None;
        self.status = ThreadStatus::Ok;
    }

    /// Run garbage collection
    pub fn collect_garbage(&mut self) {
        self.gc.collect();
    }

    /// Get memory used
    pub fn memory_used(&self) -> usize {
        self.gc.memory_used() + self.strings.memory_used()
    }

    /// Register a native function
    pub fn register_function(&mut self, name: &str, func: crate::value::NativeFn) {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = self.gc.alloc(Function::Native(native));
        self.set_global(name, Value::function(func_ref));
    }

    /// Find or create an open upvalue for a stack index.
    /// Reuses existing upvalue if one exists for this index.
    pub fn find_or_create_upvalue(&mut self, slot_index: usize) -> GcRef<Upvalue> {
        // Walk the list (sorted by slot index, highest first)
        let mut prev: Option<GcRef<Upvalue>> = None;
        let mut current = self.open_upvalues;

        while let Some(uv_ref) = current {
            let uv = unsafe { &*uv_ref.as_ptr() };
            if let Some(uv_index) = uv.stack_index() {
                if uv_index == slot_index {
                    // Found existing upvalue for this slot
                    return uv_ref;
                }
                if uv_index < slot_index {
                    // Insert before this one
                    break;
                }
            }
            prev = Some(uv_ref);
            current = uv.next.get();
        }

        // Create new upvalue
        let new_uv = Upvalue::new_open(slot_index);
        let new_uv_ref = self.gc.alloc(new_uv);

        // Link into list
        unsafe {
            (*new_uv_ref.as_ptr()).next.set(current);
        }

        if let Some(prev_ref) = prev {
            unsafe {
                (*prev_ref.as_ptr()).next.set(Some(new_uv_ref));
            }
        } else {
            self.open_upvalues = Some(new_uv_ref);
        }

        new_uv_ref
    }

    /// Close all upvalues for stack slots >= level
    pub fn close_upvalues(&mut self, level: usize) {
        while let Some(uv_ref) = self.open_upvalues {
            let uv = unsafe { &*uv_ref.as_ptr() };
            if let Some(slot_index) = uv.stack_index() {
                if slot_index >= level {
                    // Close this upvalue, passing the stack for value capture
                    uv.close_with_stack(self.stack.all_values());
                    self.open_upvalues = uv.next.get();
                } else {
                    break;
                }
            } else {
                // Already closed, remove from list
                self.open_upvalues = uv.next.get();
            }
        }
    }

    /// Get error location string (source:line:) for current call position
    /// Returns empty string if no Lua call is active
    pub fn get_error_location(&self) -> String {
        // Walk the call stack to find the first Lua frame
        for frame in self.call_stack.frames().iter().rev() {
            if !frame.is_native {
                if let Some(closure_ref) = &frame.closure {
                    let closure = unsafe { &*closure_ref.as_ptr() };
                    let proto = unsafe { &*closure.proto.as_ptr() };

                    // Get source name
                    let source = if let Some(src_ref) = &proto.source {
                        let src = unsafe { &*src_ref.as_ptr() };
                        src.as_str().unwrap_or("?").to_string()
                    } else {
                        "?".to_string()
                    };

                    // Get line number from PC (PC points to next instruction, so use pc-1)
                    let line = if frame.pc > 0 && frame.pc <= proto.lineinfo.len() {
                        proto.lineinfo[frame.pc - 1]
                    } else if !proto.lineinfo.is_empty() {
                        proto.lineinfo[0]
                    } else {
                        proto.line_defined
                    };

                    return format!("{}:{}: ", source, line);
                }
            }
        }
        String::new()
    }

    /// Format an error with location info
    pub fn format_error(&self, error: &LuaError) -> String {
        let location = self.get_error_location();
        format!("{}{}", location, error)
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("stack_top", &self.stack.top())
            .field("call_depth", &self.call_stack.depth())
            .field("status", &self.status)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_creation() {
        let state = State::new();
        assert_eq!(state.stack.top(), 0);
        assert!(state.call_stack.is_empty());
    }

    #[test]
    fn test_push_pop() {
        let mut state = State::new();

        state.push(Value::integer(42)).unwrap();
        assert_eq!(state.get_top(), 1);

        let val = state.pop();
        assert_eq!(val.as_integer(), Some(42));
        assert_eq!(state.get_top(), 0);
    }

    #[test]
    fn test_globals() {
        let mut state = State::new();

        state.set_global("test", Value::integer(123));
        // Note: get_global uses hash-based lookup in this implementation
    }
}
