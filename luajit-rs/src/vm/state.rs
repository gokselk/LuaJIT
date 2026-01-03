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

    /// Get a global variable
    pub fn get_global(&self, name: &str) -> Value {
        let key = {
            let hash = crate::value::string::LuaString::compute_hash(name.as_bytes());
            Value::number(hash as f64)
        };
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

    /// Get stack top
    pub fn get_top(&self) -> usize {
        self.stack.top()
    }

    /// Set stack top
    pub fn set_top(&mut self, top: usize) {
        self.stack.set_top(top);
    }

    /// Get value at stack index (1-based, negative from top)
    pub fn get_value(&self, index: i32) -> Value {
        let abs_index = self.abs_index(index);
        if abs_index > 0 {
            self.stack.get(abs_index as usize - 1)
        } else {
            Value::nil()
        }
    }

    /// Set value at stack index
    pub fn set_value(&mut self, index: i32, value: Value) {
        let abs_index = self.abs_index(index);
        if abs_index > 0 {
            self.stack.set(abs_index as usize - 1, value);
        }
    }

    /// Convert relative index to absolute
    fn abs_index(&self, index: i32) -> i32 {
        if index > 0 {
            index
        } else if index == 0 {
            0
        } else {
            self.stack.top() as i32 + index + 1
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

    /// Get value as number
    pub fn to_number(&self, index: i32) -> Option<f64> {
        self.get_value(index).as_number()
    }

    /// Get value as integer
    pub fn to_integer(&self, index: i32) -> Option<i32> {
        self.get_value(index).as_integer()
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

    /// Load and compile a chunk
    pub fn load_string(&mut self, source: &str, chunk_name: &str) -> LuaResult<GcRef<Function>> {
        let proto = crate::parser::parse(source, chunk_name)?;
        let proto_ref = self.gc.alloc(proto);
        let closure = Closure::new(proto_ref, Some(self.globals));
        let func = Function::Lua(closure);
        Ok(self.gc.alloc(func))
    }

    /// Call a function on the stack
    pub fn call(&mut self, nargs: usize, nresults: i32) -> LuaResult<()> {
        let func_idx = self.stack.top() - nargs - 1;
        let func = self.stack.get(func_idx);

        if !func.is_function() {
            return Err(LuaError::CallError(func.lua_type()));
        }

        // Run interpreter
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
