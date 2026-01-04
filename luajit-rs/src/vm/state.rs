//! Lua state - the main VM context.

use super::{Stack, CallStack, CallFrame};
use crate::value::{
    Value, LuaResult, LuaError, GcRef, LuaString, Table, Function, Closure, Proto, Upvalue, Userdata,
};
use crate::value::string::StringInterner;
use crate::gc::GarbageCollector;
use std::collections::{HashMap, HashSet};

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
    /// Cached error location (captured before stack unwinding)
    pub error_location: Option<String>,
    /// Current metamethod being called (for debug info)
    pub current_metamethod: Option<String>,
    /// Name of the value being called (for __call metamethod debug info)
    pub call_name: Option<String>,
    /// How the called value was accessed (for __call metamethod debug info)
    pub call_name_what: Option<String>,
    /// List of userdata with __gc finalizers (tracked as raw pointers for weak ref semantics)
    pub finalizable_userdata: Vec<*mut Userdata>,
    /// Current GC trigger context (e.g., "__concat" when GC is triggered during CAT)
    pub gc_trigger_context: Option<String>,
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
            error_location: None,
            current_metamethod: None,
            call_name: None,
            call_name_what: None,
            finalizable_userdata: Vec::new(),
            gc_trigger_context: None,
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
        let table = self.gc.alloc(Table::with_capacity(narr, nrec));
        self.check_gc();
        table
    }

    /// Check if GC should run and run it if needed
    pub fn check_gc(&mut self) {
        if self.gc.step(0) {
            self.collect_garbage();
        }
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
        // === PHASE 1: Mark and sweep strings FIRST ===
        // This must happen before finalizers run, as finalizers may create strings
        self.strings.begin_gc();
        let mut visited_tables: HashSet<usize> = HashSet::new();

        // Mark strings on the stack (use top() for absolute index)
        for i in 0..self.stack.top() {
            let val = self.stack.get(i);
            self.mark_value_strings(val, &mut visited_tables);
        }

        // Mark strings in globals and registry
        self.mark_table_strings(self.globals, &mut visited_tables);
        self.mark_table_strings(self.registry, &mut visited_tables);

        // Collect upvalue values first (to avoid borrow issues)
        let mut upvalue_values: Vec<Value> = Vec::new();
        for frame in self.call_stack.frames() {
            if let Some(ref closure) = frame.closure {
                let closure_obj = unsafe { &*closure.as_ptr() };
                for uv in &closure_obj.upvalues {
                    let upval = unsafe { &*uv.as_ptr() };
                    if !upval.is_open() {
                        upvalue_values.push(upval.get());
                    }
                }
                // Also mark proto constant strings
                let proto = unsafe { &*closure_obj.proto.as_ptr() };
                for constant in &proto.constants {
                    if let Some(s) = constant.as_string() {
                        self.strings.mark(s.as_ptr());
                    }
                }
            }
        }
        for val in upvalue_values {
            self.mark_value_strings(val, &mut visited_tables);
        }

        // Mark strings in open upvalues
        let mut uv_opt = self.open_upvalues;
        while let Some(uv_ref) = uv_opt {
            let uv = unsafe { &*uv_ref.as_ptr() };
            if !uv.is_open() {
                if let Some(s) = uv.get().as_string() {
                    self.strings.mark(s.as_ptr());
                }
            }
            uv_opt = uv.next.get();
        }

        // Mark strings in type metatables (metamethod names like "__gc", "__add", etc.)
        // Collect first to avoid borrow issues
        let mts: Vec<GcRef<Table>> = self.metatables.iter().filter_map(|x| *x).collect();
        for mt in mts {
            self.mark_table_strings(mt, &mut visited_tables);
        }

        // Mark strings in error handler
        let error_handler = self.error_handler;
        if let Some(eh) = error_handler {
            self.mark_function_strings(eh, &mut visited_tables);
        }

        // Mark strings in hook function
        let hook = self.hook;
        if let Some(h) = hook {
            self.mark_function_strings(h, &mut visited_tables);
        }

        // Sweep unreachable strings
        self.strings.sweep();

        // === PHASE 2: Handle userdata finalization ===
        let mut reachable_ud: HashSet<*mut Userdata> = HashSet::new();

        // Scan entire stack for userdata
        for i in 0..self.stack.top() {
            let val = self.stack.get(i);
            if let Some(ud) = val.as_userdata() {
                reachable_ud.insert(ud.as_ptr());
            }
        }
        self.mark_table_userdata(self.globals, &mut reachable_ud);
        self.mark_table_userdata(self.registry, &mut reachable_ud);

        // Find unreachable finalizable userdata
        let gc_key = self.intern_string("__gc");
        let mut to_finalize: Vec<(*mut Userdata, Value)> = Vec::new();
        let mut keep: Vec<*mut Userdata> = Vec::new();

        for &ptr in &self.finalizable_userdata {
            if reachable_ud.contains(&ptr) {
                keep.push(ptr);
            } else {
                let ud = unsafe { &*ptr };
                if let Some(mt) = ud.get_metatable() {
                    let gc_fn = unsafe { (*mt.as_ptr()).get(&gc_key) };
                    if gc_fn.is_function() {
                        to_finalize.push((ptr, gc_fn));
                    }
                }
            }
        }
        self.finalizable_userdata = keep;

        // Run finalizers (after string sweep, so new strings aren't immediately collected)
        for (ud_ptr, gc_fn) in to_finalize {
            let saved_top = self.get_top();
            // Use the GC trigger context if set (e.g., "__concat"), otherwise "__gc"
            let mm_name = self.gc_trigger_context.clone().unwrap_or_else(|| "__gc".to_string());
            self.current_metamethod = Some(mm_name);

            if self.push(gc_fn).is_ok() {
                if self.push(Value::userdata(GcRef::new(ud_ptr))).is_ok() {
                    let _ = self.pcall(1, 0);
                }
            }

            self.current_metamethod = None;
            self.set_top(saved_top);
        }

        // === PHASE 3: GC collection for other objects ===
        self.gc.collect();
    }

    /// Mark strings reachable from a value
    fn mark_value_strings(&mut self, value: Value, visited_tables: &mut HashSet<usize>) {
        if let Some(s) = value.as_string() {
            self.strings.mark(s.as_ptr());
        } else if let Some(t) = value.as_table() {
            self.mark_table_strings(t, visited_tables);
        } else if let Some(f) = value.as_function() {
            self.mark_function_strings(f, visited_tables);
        } else if let Some(ud) = value.as_userdata() {
            // Mark strings in userdata metatable
            let ud_ref = unsafe { &*ud.as_ptr() };
            if let Some(mt) = ud_ref.get_metatable() {
                self.mark_table_strings(mt, visited_tables);
            }
        }
    }

    /// Mark strings reachable from a function and its proto
    fn mark_function_strings(&mut self, func_ref: GcRef<Function>, visited_tables: &mut HashSet<usize>) {
        let func = unsafe { &*func_ref.as_ptr() };

        // Collect data first to avoid borrow conflicts
        let (proto_ref, env_ref, upvalue_values, native_values): (
            Option<GcRef<Proto>>,
            Option<GcRef<Table>>,
            Vec<Value>,
            Vec<Value>
        ) = match func {
            Function::Lua(closure) => {
                let mut uv_vals = Vec::new();
                for uv in &closure.upvalues {
                    let upval = unsafe { &*uv.as_ptr() };
                    if !upval.is_open() {
                        uv_vals.push(upval.get());
                    }
                }
                (Some(closure.proto), closure.env, uv_vals, Vec::new())
            }
            Function::Native(native) => {
                (None, None, Vec::new(), native.upvalues.to_vec())
            }
        };

        // Now mark using collected data
        if let Some(proto) = proto_ref {
            self.mark_proto_strings(proto, visited_tables);
        }

        if let Some(env) = env_ref {
            self.mark_table_strings(env, visited_tables);
        }

        for val in upvalue_values {
            self.mark_value_strings(val, visited_tables);
        }

        for val in native_values {
            self.mark_value_strings(val, visited_tables);
        }
    }

    /// Mark all strings in a Proto and its nested protos
    fn mark_proto_strings(&mut self, proto_ref: GcRef<Proto>, visited_tables: &mut HashSet<usize>) {
        let proto = unsafe { &*proto_ref.as_ptr() };

        // Mark source string
        if let Some(src) = proto.source {
            self.strings.mark(src.as_ptr());
        }

        // Mark upvalue names
        for name_opt in &proto.upvalue_names {
            if let Some(name) = name_opt {
                self.strings.mark(name.as_ptr());
            }
        }

        // Mark strings in constants
        for constant in &proto.constants {
            if let Some(s) = constant.as_string() {
                self.strings.mark(s.as_ptr());
            } else if let Some(t) = constant.as_table() {
                self.mark_table_strings(t, visited_tables);
            } else if let Some(f) = constant.as_function() {
                self.mark_function_strings(f, visited_tables);
            }
        }

        // Mark nested protos recursively
        for nested in &proto.protos {
            self.mark_proto_strings(*nested, visited_tables);
        }
    }

    /// Mark strings reachable from a table
    fn mark_table_strings(&mut self, table: GcRef<Table>, visited_tables: &mut HashSet<usize>) {
        let ptr = table.as_ptr() as usize;
        if !visited_tables.insert(ptr) {
            return; // Already visited
        }

        let t = unsafe { &*table.as_ptr() };
        // Use iter_all to include both array and hash parts
        for (key, value) in t.iter_all() {
            self.mark_value_strings(key, visited_tables);
            self.mark_value_strings(value, visited_tables);
        }
        if let Some(mt) = t.get_metatable() {
            self.mark_table_strings(mt, visited_tables);
        }
    }

    /// Mark userdata reachable from a table (recursive)
    fn mark_table_userdata(&self, table: GcRef<Table>, reachable: &mut HashSet<*mut Userdata>) {
        let t = unsafe { &*table.as_ptr() };
        for (key, value) in t.iter() {
            self.mark_value_userdata(&key, reachable);
            self.mark_value_userdata(&value, reachable);
        }
        if let Some(mt) = t.get_metatable() {
            self.mark_table_userdata(mt, reachable);
        }
    }

    /// Mark userdata reachable from a value
    fn mark_value_userdata(&self, value: &Value, reachable: &mut HashSet<*mut Userdata>) {
        if let Some(ud) = value.as_userdata() {
            reachable.insert(ud.as_ptr());
        } else if let Some(t) = value.as_table() {
            let table = unsafe { &*t.as_ptr() };
            for (k, v) in table.iter() {
                if let Some(ud) = k.as_userdata() {
                    reachable.insert(ud.as_ptr());
                }
                if let Some(ud) = v.as_userdata() {
                    reachable.insert(ud.as_ptr());
                }
            }
        }
    }

    /// Register a userdata for finalization (called when __gc is set)
    pub fn register_finalizable(&mut self, ud: GcRef<Userdata>) {
        let ptr = ud.as_ptr();
        if !self.finalizable_userdata.contains(&ptr) {
            self.finalizable_userdata.push(ptr);
        }
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
    pub fn format_error(&mut self, error: &LuaError) -> String {
        // RuntimeErrorNoLocation: don't add location info (error level=0)
        if let LuaError::RuntimeErrorNoLocation(msg) = error {
            self.error_location = None;
            return msg.clone();
        }

        // Use cached error location if available (captured before stack unwinding)
        let location = if let Some(ref loc) = self.error_location {
            loc.clone()
        } else {
            self.get_error_location()
        };
        // Clear the cached location after use
        self.error_location = None;
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
