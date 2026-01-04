//! Bytecode interpreter.
//!
//! This module implements the main interpretation loop that executes bytecode.

use super::{State, CallFrame};
use crate::value::{Value, LuaResult, LuaError, GcRef, Function, Closure, Table, Upvalue};
use crate::bytecode::{Instruction, Opcode};

/// The bytecode interpreter
pub struct Interpreter<'a> {
    state: &'a mut State,
}

impl<'a> Interpreter<'a> {
    pub fn new(state: &'a mut State) -> Self {
        Self { state }
    }

    /// Call a function
    pub fn call(&mut self, func_idx: usize, nargs: usize, nresults: i32) -> LuaResult<()> {
        let func = self.state.stack.get(func_idx);

        match func.as_function() {
            Some(func_ref) => unsafe {
                match &*func_ref.as_ptr() {
                    Function::Lua(_closure) => {
                        self.call_lua(func_idx, nargs, nresults)
                    }
                    Function::Native(_native) => {
                        self.call_native(func_idx, nargs, nresults)
                    }
                }
            },
            None => {
                // Try __call metamethod
                if let Some(mm) = self.get_metamethod(&func, "__call") {
                    // Get the name of the called value before shifting
                    let (name, name_what) = self.get_value_name(func_idx);

                    // Shift arguments right to make room for the object as first arg
                    // Stack layout before: [obj, arg1, arg2, ...]
                    // Stack layout after: [mm, obj, arg1, arg2, ...]
                    for i in (0..=nargs).rev() {
                        let val = self.state.stack.get(func_idx + i);
                        self.state.stack.set(func_idx + i + 1, val);
                    }
                    self.state.stack.set(func_idx, mm);

                    // Track the call name for debug.getinfo
                    self.state.call_name = name;
                    self.state.call_name_what = name_what;

                    // Now call with nargs+1 arguments (the original object + original args)
                    let result = self.call(func_idx, nargs + 1, nresults);

                    // Clear the call name
                    self.state.call_name = None;
                    self.state.call_name_what = None;

                    return result;
                }
                Err(LuaError::CallError(func.lua_type()))
            },
        }
    }

    /// Call a Lua function
    fn call_lua(&mut self, func_idx: usize, nargs: usize, nresults: i32) -> LuaResult<()> {
        let func = self.state.stack.get(func_idx);
        let func_ref = func.as_function().unwrap();

        let closure = unsafe {
            match &*func_ref.as_ptr() {
                Function::Lua(c) => GcRef::new(c as *const Closure as *mut Closure),
                _ => return Err(LuaError::CallError(func.lua_type())),
            }
        };

        let proto = unsafe { &*(*closure.as_ptr()).proto.as_ptr() };
        let arg_base = func_idx + 1; // Where args start

        // Save caller's stack state
        let saved_base = self.state.stack.base();
        let saved_top = self.state.stack.top();

        // Adjust arguments to match parameters
        let num_params = proto.num_params as usize;
        let current_args = nargs;

        // Handle varargs - varargs are stored BEFORE the frame's base
        // so they don't conflict with local registers
        let (base, vararg_base, vararg_count) = if proto.is_vararg {
            let vcount = if current_args > num_params { current_args - num_params } else { 0 };
            // Varargs start after fixed params: arg_base + num_params
            let vbase = arg_base + num_params;
            // Frame base starts after varargs
            let frame_base = arg_base + current_args.max(num_params);

            // Copy fixed params to frame base (R0, R1, ...)
            for i in 0..num_params.min(current_args) {
                let val = self.state.stack.get(arg_base + i);
                self.state.stack.set(frame_base + i, val);
            }
            // Fill missing params with nil
            for i in current_args..num_params {
                self.state.stack.set(frame_base + i, Value::nil());
            }

            (frame_base, Some(vbase), vcount)
        } else {
            // Non-vararg: base is where args start, fill missing with nil
            for i in current_args..num_params {
                self.state.stack.set(arg_base + i, Value::nil());
            }
            (arg_base, None, 0)
        };

        // Create call frame
        let mut frame = CallFrame::new_lua(closure, base, func_idx, nresults);
        frame.vararg_base = vararg_base;
        frame.vararg_count = vararg_count;
        frame.top = base + proto.max_stack_size as usize;

        // Set up stack
        self.state.stack.set_base(base);
        self.state.stack.set_top(frame.top);
        if let Err(e) = self.state.call_stack.push(frame) {
            // Capture error location before unwinding - use the caller's frame
            self.state.error_location = Some(self.state.get_error_location());
            // Restore stack state before returning error
            self.state.stack.set_base(saved_base);
            self.state.stack.set_top(saved_top);
            return Err(e);
        }

        // Execute
        let result = self.execute();

        // Capture error location before popping the frame
        if result.is_err() && self.state.error_location.is_none() {
            self.state.error_location = Some(self.state.get_error_location());
        }

        // Clean up - restore caller's stack state
        self.state.call_stack.pop();

        // Results are placed starting at func_idx by do_return
        // Current top = func_idx + actual_num_results
        if nresults >= 0 {
            // Fixed number of results requested - pad with nil if needed
            let wanted = nresults as usize;
            let current_top = self.state.stack.top();
            let actual = current_top.saturating_sub(func_idx);
            for i in actual..wanted {
                self.state.stack.set(func_idx + i, Value::nil());
            }
            self.state.stack.set_top(func_idx + wanted);
        }
        // When nresults == -1, keep the top set by do_return (which includes all actual results)

        self.state.stack.set_base(saved_base);

        result
    }

    /// Call a native function
    fn call_native(&mut self, func_idx: usize, nargs: usize, nresults: i32) -> LuaResult<()> {
        let func = self.state.stack.get(func_idx);
        let func_ref = func.as_function().unwrap();

        let native = unsafe {
            match &*func_ref.as_ptr() {
                Function::Native(n) => n,
                _ => return Err(LuaError::CallError(func.lua_type())),
            }
        };

        let base = func_idx + 1;
        let old_base = self.state.stack.base();
        let old_top = self.state.stack.top();

        // Set base and top for the native call
        self.state.stack.set_base(base);
        self.state.stack.set_top(base + nargs);

        // Create native frame
        let frame = CallFrame::new_native(base, func_idx, nresults);
        self.state.call_stack.push(frame)?;

        // Call the function
        let result = (native.func)(self.state);

        self.state.call_stack.pop();

        match result {
            Ok(num_returns) => {
                // Adjust results
                let results_start = self.state.stack.top() - num_returns;
                let target_start = func_idx;

                // Move results
                for i in 0..num_returns {
                    let val = self.state.stack.get(results_start + i);
                    self.state.stack.set(target_start + i, val);
                }

                // Fill remaining expected slots with nil (important for ITERC when next() returns 0)
                if nresults > 0 {
                    for i in num_returns..(nresults as usize) {
                        self.state.stack.set(target_start + i, Value::nil());
                    }
                }

                // Adjust top
                let new_top = if nresults < 0 {
                    target_start + num_returns
                } else {
                    target_start + nresults as usize
                };
                self.state.stack.set_top(new_top);
                self.state.stack.set_base(old_base);

                Ok(())
            }
            Err(e) => {
                self.state.stack.set_base(old_base);
                self.state.stack.set_top(old_top);
                Err(e)
            }
        }
    }

    /// Look up a metamethod in a value's metatable
    fn get_metamethod(&mut self, val: &Value, method: &str) -> Option<Value> {
        // Get metatable from table, userdata, or type metatable
        let metatable = if let Some(t) = val.as_table() {
            let table = unsafe { &*t.as_ptr() };
            table.get_metatable()
        } else if let Some(u) = val.as_userdata() {
            let userdata = unsafe { &*u.as_ptr() };
            userdata.get_metatable()
        } else {
            // Check type metatable for primitive types (number, string, boolean, etc.)
            let type_index = val.lua_type() as usize;
            if type_index < self.state.metatables.len() {
                self.state.metatables[type_index]
            } else {
                None
            }
        };

        // Look up the metamethod in the metatable
        if let Some(mt) = metatable {
            let mt = unsafe { &*mt.as_ptr() };
            let key = self.state.intern_string(method);
            let result = mt.get(&key);
            if !result.is_nil() {
                return Some(result);
            }
        }
        None
    }

    /// Get the name and namewhat for a value at a given slot based on bytecode analysis
    /// Returns (name, namewhat) if found, or (None, None) if not determinable
    fn get_value_name(&self, slot: usize) -> (Option<String>, Option<String>) {
        // Get the current frame to analyze the bytecode
        let frame = match self.state.call_stack.current() {
            Some(f) => f,
            None => return (None, None),
        };

        let closure = match frame.closure {
            Some(c) => c,
            None => return (None, None),
        };

        let proto = unsafe { &*(*closure.as_ptr()).proto.as_ptr() };
        let pc = frame.pc;

        // Adjust slot relative to frame base
        let frame_base = self.state.stack.base();
        let relative_slot = if slot >= frame_base {
            (slot - frame_base) as u8
        } else {
            return (None, None);
        };

        // First, check if the slot is a named local variable at the current PC
        for locvar in &proto.locvars {
            if locvar.slot == relative_slot {
                let start = locvar.start_pc as usize;
                let end = locvar.end_pc as usize;
                if start <= pc && pc <= end {
                    eprintln!("DEBUG FOUND: {} at slot {}", locvar.name, locvar.slot);
                    return (Some(locvar.name.clone()), Some("local".to_string()));
                }
            }
        }

        // Look for the instruction that put the value in this slot
        // Search backward from current PC
        for i in (0..pc).rev() {
            let instr = proto.code[i];
            let a = instr.a();

            // Check if this instruction writes to our slot
            if a != relative_slot {
                continue;
            }

            match instr.opcode() {
                // MOV from another register - check if that register is a known local
                Opcode::MOV => {
                    let d = instr.d() as u8;
                    // Check if source is a named local at this PC
                    for locvar in &proto.locvars {
                        if locvar.slot == d {
                            let start = locvar.start_pc as usize;
                            let end = locvar.end_pc as usize;
                            if start <= i && i <= end {
                                return (Some(locvar.name.clone()), Some("local".to_string()));
                            }
                        }
                    }
                    // MOV but source not a named local - stop searching
                    break;
                }
                // GGET - global get
                Opcode::GGET => {
                    let d = instr.d() as usize;
                    if let Some(bytes) = frame.get_string_constant(d) {
                        if let Ok(name) = std::str::from_utf8(bytes) {
                            return (Some(name.to_string()), Some("global".to_string()));
                        }
                    }
                    break;
                }
                // TGETS - field access
                Opcode::TGETS => {
                    let c = instr.c() as usize;
                    if let Some(bytes) = frame.get_string_constant(c) {
                        if let Ok(name) = std::str::from_utf8(bytes) {
                            return (Some(name.to_string()), Some("field".to_string()));
                        }
                    }
                    break;
                }
                // Skip over CALL/CALLT - the value was there before the call
                Opcode::CALL | Opcode::CALLT | Opcode::CALLM | Opcode::CALLMT => {
                    // Continue searching - the slot held a value before this call
                    continue;
                }
                // For other opcodes that write to the slot, stop searching
                _ => {
                    break;
                }
            }
        }

        (None, None)
    }

    /// Call a binary metamethod and return the result
    fn call_binary_metamethod(&mut self, mm: Value, a: Value, b: Value, result_slot: usize, mm_name: &str) -> LuaResult<()> {
        // Set up the call: func, arg1, arg2
        let call_base = self.state.stack.top();
        self.state.stack.set(call_base, mm);
        self.state.stack.set(call_base + 1, a);
        self.state.stack.set(call_base + 2, b);
        self.state.stack.set_top(call_base + 3);

        // Track metamethod for debug info
        self.state.current_metamethod = Some(mm_name.to_string());

        // Call the metamethod
        let result = self.call(call_base, 2, 1);

        // Clear metamethod tracking
        self.state.current_metamethod = None;

        result?;

        // Get the result and move it to the target slot
        let result = self.state.stack.get(call_base);
        self.state.stack.set(result_slot, result);

        Ok(())
    }

    /// Call a unary metamethod and return the result
    fn call_unary_metamethod(&mut self, mm: Value, a: Value, result_slot: usize, mm_name: &str) -> LuaResult<()> {
        // Set up the call: func, arg, arg (Lua passes operand twice for consistency with binary ops)
        let call_base = self.state.stack.top();
        self.state.stack.set(call_base, mm);
        self.state.stack.set(call_base + 1, a.clone());
        self.state.stack.set(call_base + 2, a);
        self.state.stack.set_top(call_base + 3);

        // Track metamethod for debug info
        self.state.current_metamethod = Some(mm_name.to_string());

        // Call the metamethod
        let result = self.call(call_base, 2, 1);

        // Clear metamethod tracking
        self.state.current_metamethod = None;

        result?;

        // Get the result and move it to the target slot
        let result = self.state.stack.get(call_base);
        self.state.stack.set(result_slot, result);

        Ok(())
    }

    /// Perform binary arithmetic with metamethod fallback
    fn arith_binary(&mut self, vb: Value, vc: Value, base_a: usize, mm_name: &str,
                    op: fn(&Value, &Value) -> LuaResult<Value>) -> LuaResult<()> {
        // Try the normal operation first
        match op(&vb, &vc) {
            Ok(result) => {
                self.state.stack.set(base_a, result);
                Ok(())
            }
            Err(LuaError::ArithmeticError(_)) => {
                // Try metamethod from first operand, then second
                if let Some(mm) = self.get_metamethod(&vb, mm_name) {
                    return self.call_binary_metamethod(mm, vb, vc, base_a, mm_name);
                }
                if let Some(mm) = self.get_metamethod(&vc, mm_name) {
                    return self.call_binary_metamethod(mm, vb, vc, base_a, mm_name);
                }
                // No metamethod found, return the original error
                let ty = if vb.coerce_to_number().is_none() {
                    vb.lua_type()
                } else {
                    vc.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
            Err(e) => Err(e),
        }
    }

    /// Perform unary arithmetic with metamethod fallback
    fn arith_unary(&mut self, v: Value, base_a: usize, mm_name: &str,
                   op: fn(&Value) -> LuaResult<Value>) -> LuaResult<()> {
        // Try the normal operation first
        match op(&v) {
            Ok(result) => {
                self.state.stack.set(base_a, result);
                Ok(())
            }
            Err(LuaError::ArithmeticError(_)) => {
                // Try metamethod
                if let Some(mm) = self.get_metamethod(&v, mm_name) {
                    return self.call_unary_metamethod(mm, v.clone(), base_a, mm_name);
                }
                Err(LuaError::ArithmeticError(v.lua_type()))
            }
            Err(e) => Err(e),
        }
    }

    /// Call a comparison metamethod and return the boolean result
    fn call_compare_metamethod(&mut self, mm: Value, a: Value, b: Value, mm_name: &str) -> LuaResult<bool> {
        let call_base = self.state.stack.top();
        self.state.stack.set(call_base, mm);
        self.state.stack.set(call_base + 1, a);
        self.state.stack.set(call_base + 2, b);
        self.state.stack.set_top(call_base + 3);

        // Track metamethod for debug info
        self.state.current_metamethod = Some(mm_name.to_string());

        let result = self.call(call_base, 2, 1);

        // Clear metamethod tracking
        self.state.current_metamethod = None;

        result?;

        let result = self.state.stack.get(call_base);
        Ok(result.is_truthy())
    }

    /// Compare with __lt metamethod fallback
    /// Returns Ok(Some(bool)) if comparison succeeded, Ok(None) if no metamethod
    /// In Lua 5.1/LuaJIT mode, both operands must have the SAME __lt metamethod
    fn compare_lt(&mut self, a: Value, b: Value) -> LuaResult<Option<bool>> {
        let mm_a = self.get_metamethod(&a, "__lt");
        let mm_b = self.get_metamethod(&b, "__lt");

        match (mm_a, mm_b) {
            (Some(mma), Some(mmb)) if mma.raw_eq(&mmb) => {
                // Both have the same __lt metamethod
                Ok(Some(self.call_compare_metamethod(mma, a, b, "__lt")?))
            }
            (Some(_), Some(_)) => {
                // Different metamethods - error in Lua 5.1 mode
                Err(LuaError::CompareError(a.lua_type(), b.lua_type()))
            }
            (Some(_), None) | (None, Some(_)) => {
                // Only one has metamethod - check if same metatable
                // In Lua 5.1, this should error, but LuaJIT is more lenient when
                // both operands are the same type with same metatable
                // For now, follow strict Lua 5.1 semantics: both must have same mm
                Err(LuaError::CompareError(a.lua_type(), b.lua_type()))
            }
            (None, None) => Ok(None),
        }
    }

    /// Compare with __le metamethod fallback
    /// Returns Ok(Some(bool)) if comparison succeeded, Ok(None) if no metamethod
    /// In Lua 5.1/LuaJIT mode, both operands must have the SAME __le metamethod
    fn compare_le(&mut self, a: Value, b: Value) -> LuaResult<Option<bool>> {
        let mm_a = self.get_metamethod(&a, "__le");
        let mm_b = self.get_metamethod(&b, "__le");

        match (mm_a, mm_b) {
            (Some(mma), Some(mmb)) if mma.raw_eq(&mmb) => {
                // Both have the same __le metamethod
                Ok(Some(self.call_compare_metamethod(mma, a.clone(), b.clone(), "__le")?))
            }
            (Some(_), Some(_)) => {
                // Different metamethods - fall back to not(b < a)
                if let Some(result) = self.compare_lt(b, a)? {
                    return Ok(Some(!result));
                }
                Err(LuaError::CompareError(a.lua_type(), b.lua_type()))
            }
            (Some(_), None) | (None, Some(_)) => {
                // Only one has __le - fall back to not(b < a)
                if let Some(result) = self.compare_lt(b, a)? {
                    return Ok(Some(!result));
                }
                Err(LuaError::CompareError(a.lua_type(), b.lua_type()))
            }
            (None, None) => {
                // No __le - fall back to not(b < a) if __lt is available
                if let Some(result) = self.compare_lt(b, a)? {
                    return Ok(Some(!result));
                }
                Ok(None)
            }
        }
    }

    /// Handle __index metamethod for table/userdata access
    /// Returns the value (possibly from metamethod) or nil
    fn table_index(&mut self, table: Value, key: Value, result_slot: usize) -> LuaResult<()> {
        if let Some(t) = table.as_table() {
            let val = unsafe { (*t.as_ptr()).get(&key) };
            if !val.is_nil() {
                self.state.stack.set(result_slot, val);
                return Ok(());
            }
            // Key not found, try __index
            if let Some(mm) = self.get_metamethod(&table, "__index") {
                return self.call_index_metamethod(mm, table, key, result_slot);
            }
            self.state.stack.set(result_slot, Value::nil());
            Ok(())
        } else if table.is_userdata() {
            // Userdata always needs __index metamethod
            if let Some(mm) = self.get_metamethod(&table, "__index") {
                return self.call_index_metamethod(mm, table, key, result_slot);
            }
            self.state.stack.set(result_slot, Value::nil());
            Ok(())
        } else {
            Err(LuaError::IndexError(table.lua_type()))
        }
    }

    /// Call the __index metamethod
    fn call_index_metamethod(&mut self, mm: Value, table: Value, key: Value, result_slot: usize) -> LuaResult<()> {
        if let Some(idx_table) = mm.as_table() {
            // __index is a table, look up in it
            let val = unsafe { (*idx_table.as_ptr()).get(&key) };
            self.state.stack.set(result_slot, val);
        } else if mm.as_function().is_some() {
            // __index is a function, call it
            let call_base = self.state.stack.top();
            self.state.stack.set(call_base, mm);
            self.state.stack.set(call_base + 1, table);
            self.state.stack.set(call_base + 2, key);
            self.state.stack.set_top(call_base + 3);
            // Track metamethod for debug info
            self.state.current_metamethod = Some("__index".to_string());
            let result = self.call(call_base, 2, 1);
            self.state.current_metamethod = None;
            result?;
            let val = self.state.stack.get(call_base);
            self.state.stack.set(result_slot, val);
        } else {
            self.state.stack.set(result_slot, Value::nil());
        }
        Ok(())
    }

    /// Handle __newindex metamethod for table assignment
    fn table_newindex(&mut self, table: Value, key: Value, val: Value) -> LuaResult<()> {
        if let Some(t) = table.as_table() {
            // Check if key already exists (rawget)
            let existing = unsafe { (*t.as_ptr()).get(&key) };
            if !existing.is_nil() {
                // Key exists, just set it (no metamethod)
                unsafe { (*t.as_ptr()).set(key, val) };
                return Ok(());
            }
            // Key doesn't exist, try __newindex
            if let Some(mm) = self.get_metamethod(&table, "__newindex") {
                if let Some(idx_table) = mm.as_table() {
                    // __newindex is a table, set in it
                    unsafe { (*idx_table.as_ptr()).set(key, val) };
                } else if mm.as_function().is_some() {
                    // __newindex is a function, call it
                    let call_base = self.state.stack.top();
                    self.state.stack.set(call_base, mm);
                    self.state.stack.set(call_base + 1, table);
                    self.state.stack.set(call_base + 2, key);
                    self.state.stack.set(call_base + 3, val);
                    self.state.stack.set_top(call_base + 4);
                    // Track metamethod for debug info
                    self.state.current_metamethod = Some("__newindex".to_string());
                    let result = self.call(call_base, 3, 0);
                    self.state.current_metamethod = None;
                    result?;
                }
                return Ok(());
            }
            // No metamethod, just set
            unsafe { (*t.as_ptr()).set(key, val) };
            Ok(())
        } else if table.is_userdata() {
            // Userdata always needs __newindex metamethod
            if let Some(mm) = self.get_metamethod(&table, "__newindex") {
                if let Some(idx_table) = mm.as_table() {
                    // __newindex is a table, set in it
                    unsafe { (*idx_table.as_ptr()).set(key, val) };
                } else if mm.as_function().is_some() {
                    // __newindex is a function, call it
                    let call_base = self.state.stack.top();
                    self.state.stack.set(call_base, mm);
                    self.state.stack.set(call_base + 1, table);
                    self.state.stack.set(call_base + 2, key);
                    self.state.stack.set(call_base + 3, val);
                    self.state.stack.set_top(call_base + 4);
                    // Track metamethod for debug info
                    self.state.current_metamethod = Some("__newindex".to_string());
                    let result = self.call(call_base, 3, 0);
                    self.state.current_metamethod = None;
                    result?;
                }
                return Ok(());
            }
            // No metamethod - error
            Err(LuaError::IndexError(table.lua_type()))
        } else {
            Err(LuaError::IndexError(table.lua_type()))
        }
    }

    /// Main execution loop
    fn execute(&mut self) -> LuaResult<()> {
        loop {
            let frame = match self.state.call_stack.current_mut() {
                Some(f) => f,
                None => return Ok(()),
            };

            let instr = match frame.fetch() {
                Some(i) => i,
                None => return Err(LuaError::RuntimeError("instruction fetch failed".to_string())),
            };

            let base = frame.base;
            let func_idx = frame.func_idx;
            let op = instr.opcode();
            let a = instr.a() as usize;

            match op {
                // Move and load operations
                Opcode::MOV => {
                    let d = instr.d() as usize;
                    let val = self.state.stack.get(base + d);
                    self.state.stack.set(base + a, val);
                }

                Opcode::KSHORT => {
                    let d = instr.d() as i16;
                    self.state.stack.set(base + a, Value::integer(d as i32));
                }

                Opcode::KNUM => {
                    let d = instr.d() as usize;
                    let frame = self.state.call_stack.current().unwrap();
                    let val = frame.get_constant(d);
                    self.state.stack.set(base + a, val);
                }

                Opcode::KPRI => {
                    let d = instr.d();
                    let val = match d {
                        0 => Value::nil(),
                        1 => Value::boolean(false),
                        2 => Value::boolean(true),
                        _ => Value::nil(),
                    };
                    self.state.stack.set(base + a, val);
                }

                Opcode::KNIL => {
                    let d = instr.d() as usize;
                    for i in a..=a + d {
                        self.state.stack.set(base + i, Value::nil());
                    }
                }

                Opcode::KSTR => {
                    let d = instr.d() as usize;
                    // Get string constant and intern it at runtime
                    let frame = self.state.call_stack.current().unwrap();
                    let string_val = if let Some(bytes) = frame.get_string_constant(d) {
                        // Need to copy to avoid borrow issues
                        let bytes = bytes.to_vec();
                        drop(frame);
                        self.state.intern_bytes(&bytes)
                    } else {
                        Value::nil()
                    };
                    self.state.stack.set(base + a, string_val);
                }

                // Arithmetic operations (with metamethod support)
                Opcode::ADDVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    self.arith_binary(vb, vc, base + a, "__add", Value::add)?;
                }

                Opcode::SUBVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    self.arith_binary(vb, vc, base + a, "__sub", Value::sub)?;
                }

                Opcode::MULVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    self.arith_binary(vb, vc, base + a, "__mul", Value::mul)?;
                }

                Opcode::DIVVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    self.arith_binary(vb, vc, base + a, "__div", Value::div)?;
                }

                Opcode::MODVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    self.arith_binary(vb, vc, base + a, "__mod", Value::modulo)?;
                }

                Opcode::POW => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    self.arith_binary(vb, vc, base + a, "__pow", Value::pow)?;
                }

                Opcode::UNM => {
                    let d = instr.d() as usize;
                    let vd = self.state.stack.get(base + d);
                    self.arith_unary(vd, base + a, "__unm", Value::unm)?;
                }

                Opcode::NOT => {
                    let d = instr.d() as usize;
                    let vd = self.state.stack.get(base + d);
                    let result = Value::boolean(!vd.is_truthy());
                    self.state.stack.set(base + a, result);
                }

                Opcode::LEN => {
                    let d = instr.d() as usize;
                    let vd = self.state.stack.get(base + d);

                    // In LuaJIT 5.1 mode, __len metamethod is NOT called for tables
                    // It's only called for userdata and other types with debug metatables
                    if let Some(t) = vd.as_table() {
                        // For tables, always use raw length (no __len in 5.1 mode)
                        let len = unsafe { (*t.as_ptr()).len() as f64 };
                        self.state.stack.set(base + a, Value::number(len));
                    } else if let Some(s) = vd.as_string() {
                        let len = unsafe { (*s.as_ptr()).len() as f64 };
                        self.state.stack.set(base + a, Value::number(len));
                    } else if let Some(mm) = self.get_metamethod(&vd, "__len") {
                        // For userdata and other types, check __len metamethod
                        let call_base = self.state.stack.top();
                        self.state.stack.set(call_base, mm);
                        self.state.stack.set(call_base + 1, vd);
                        self.state.stack.set_top(call_base + 2);
                        // Track metamethod for debug info
                        self.state.current_metamethod = Some("__len".to_string());
                        let result = self.call(call_base, 1, 1);
                        self.state.current_metamethod = None;
                        result?;
                        let val = self.state.stack.get(call_base);
                        self.state.stack.set(base + a, val);
                    } else {
                        return Err(LuaError::LengthError(vd.lua_type()));
                    }
                }

                // Comparison operations
                Opcode::ISLT | Opcode::ISGE | Opcode::ISLE | Opcode::ISGT => {
                    let d = instr.d() as usize;
                    let va = self.state.stack.get(base + a);
                    let vd = self.state.stack.get(base + d);

                    let cmp = if let (Some(na), Some(nb)) = (va.as_number(), vd.as_number()) {
                        // Number comparison
                        match op {
                            Opcode::ISLT => na < nb,
                            Opcode::ISGE => na >= nb,
                            Opcode::ISLE => na <= nb,
                            Opcode::ISGT => na > nb,
                            _ => unreachable!(),
                        }
                    } else if let (Some(sa), Some(sb)) = (va.as_string(), vd.as_string()) {
                        // String comparison
                        let sa = unsafe { &*sa.as_ptr() };
                        let sb = unsafe { &*sb.as_ptr() };
                        let a_bytes = sa.as_bytes();
                        let b_bytes = sb.as_bytes();
                        match op {
                            Opcode::ISLT => a_bytes < b_bytes,
                            Opcode::ISGE => a_bytes >= b_bytes,
                            Opcode::ISLE => a_bytes <= b_bytes,
                            Opcode::ISGT => a_bytes > b_bytes,
                            _ => unreachable!(),
                        }
                    } else {
                        // Try metamethods
                        let result = match op {
                            Opcode::ISLT => self.compare_lt(va.clone(), vd.clone())?,
                            Opcode::ISGT => self.compare_lt(vd.clone(), va.clone())?,
                            Opcode::ISLE => self.compare_le(va.clone(), vd.clone())?,
                            Opcode::ISGE => self.compare_le(vd.clone(), va.clone())?,
                            _ => unreachable!(),
                        };
                        match result {
                            Some(r) => r,
                            None => return Err(LuaError::CompareError(va.lua_type(), vd.lua_type())),
                        }
                    };

                    if cmp {
                        // Skip next instruction (the JMP)
                        let frame = self.state.call_stack.current_mut().unwrap();
                        frame.pc += 1;
                    }
                }

                Opcode::ISEQV | Opcode::ISNEV => {
                    let d = instr.d() as usize;
                    let va = self.state.stack.get(base + a);
                    let vd = self.state.stack.get(base + d);

                    let eq = if va.raw_eq(&vd) {
                        // Raw equality always means equal
                        true
                    } else {
                        // Try __eq metamethod for tables
                        // __eq is called only if both have the same __eq metamethod
                        let mm_a = self.get_metamethod(&va, "__eq");
                        let mm_d = self.get_metamethod(&vd, "__eq");

                        match (mm_a, mm_d) {
                            (Some(mma), Some(mmd)) if mma.raw_eq(&mmd) => {
                                // Both have the same __eq metamethod
                                self.call_compare_metamethod(mma, va, vd, "__eq")?
                            }
                            _ => false,
                        }
                    };

                    let cmp = if op == Opcode::ISEQV { eq } else { !eq };

                    if cmp {
                        let frame = self.state.call_stack.current_mut().unwrap();
                        frame.pc += 1;
                    }
                }

                Opcode::IST => {
                    let d = instr.d() as usize;
                    let vd = self.state.stack.get(base + d);
                    if vd.is_truthy() {
                        let frame = self.state.call_stack.current_mut().unwrap();
                        frame.pc += 1;
                    }
                }

                Opcode::ISF => {
                    let d = instr.d() as usize;
                    let vd = self.state.stack.get(base + d);
                    if vd.is_falsy() {
                        let frame = self.state.call_stack.current_mut().unwrap();
                        frame.pc += 1;
                    }
                }

                // Jump
                Opcode::JMP => {
                    let offset = instr.jump();
                    let frame = self.state.call_stack.current_mut().unwrap();
                    frame.jump(offset);
                }

                // Table operations
                Opcode::TNEW => {
                    let d = instr.d() as usize;
                    let asize = d & 0x7FF;
                    let hsize = d >> 11;
                    let table = self.state.gc.alloc(Table::with_capacity(asize, hsize));
                    self.state.stack.set(base + a, Value::table(table));
                    // Check if GC should run after allocation
                    self.state.check_gc();
                }

                Opcode::TGETV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + b);
                    let key = self.state.stack.get(base + c);

                    if table.is_table() {
                        self.table_index(table, key, base + a)?;
                    } else if table.is_string() {
                        // String indexing: check if numeric (byte access) or string (method)
                        if key.is_string() {
                            // Method access: s["sub"] -> string.sub
                            let string_lib = self.state.get_global("string");
                            if let Some(t) = string_lib.as_table() {
                                let val = unsafe { (*t.as_ptr()).get(&key) };
                                self.state.stack.set(base + a, val);
                            } else {
                                self.state.stack.set(base + a, Value::nil());
                            }
                        } else {
                            // Numeric indexing not typically supported
                            self.state.stack.set(base + a, Value::nil());
                        }
                    } else {
                        return Err(LuaError::IndexError(table.lua_type()));
                    }
                }

                Opcode::TGETS => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + b);
                    let frame = self.state.call_stack.current().unwrap();
                    // Get string constant and intern it for key
                    let key = if let Some(bytes) = frame.get_string_constant(c) {
                        let bytes = bytes.to_vec();
                        drop(frame);
                        self.state.intern_bytes(&bytes)
                    } else {
                        frame.get_constant(c)
                    };

                    if table.is_table() || table.is_userdata() {
                        self.table_index(table, key, base + a)?;
                    } else if table.is_string() {
                        // String method access: s.sub -> string.sub
                        let string_lib = self.state.get_global("string");
                        if let Some(t) = string_lib.as_table() {
                            let val = unsafe { (*t.as_ptr()).get(&key) };
                            self.state.stack.set(base + a, val);
                        } else {
                            self.state.stack.set(base + a, Value::nil());
                        }
                    } else {
                        return Err(LuaError::IndexError(table.lua_type()));
                    }
                }

                Opcode::TGETB => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + b);

                    if table.is_table() {
                        // Use integer key for table_index
                        let key = Value::integer(c as i32);
                        self.table_index(table, key, base + a)?;
                    } else {
                        return Err(LuaError::IndexError(table.lua_type()));
                    }
                }

                Opcode::TSETV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + a);
                    let val = self.state.stack.get(base + b);
                    let key = self.state.stack.get(base + c);

                    self.table_newindex(table, key, val)?;
                }

                Opcode::TSETS => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + a);
                    let val = self.state.stack.get(base + b);
                    let frame = self.state.call_stack.current().unwrap();
                    // Get string constant and intern it for key
                    let key = if let Some(bytes) = frame.get_string_constant(c) {
                        let bytes = bytes.to_vec();
                        drop(frame);
                        self.state.intern_bytes(&bytes)
                    } else {
                        frame.get_constant(c)
                    };

                    self.table_newindex(table, key, val)?;
                }

                Opcode::TSETB => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + a);
                    let val = self.state.stack.get(base + b);
                    let key = Value::integer(c as i32);

                    self.table_newindex(table, key, val)?;
                }

                // Multi-value table set (for varargs and calls in table constructors)
                Opcode::TSETM => {
                    let d = instr.d() as usize;  // Starting index (1-based)
                    let table = self.state.stack.get(base + a);
                    let top = self.state.stack.top();

                    if let Some(t) = table.as_table() {
                        // Set values from A+1 to top into table starting at index D
                        let num_values = top.saturating_sub(base + a + 1);
                        for i in 0..num_values {
                            let val = self.state.stack.get(base + a + 1 + i);
                            unsafe { (*t.as_ptr()).set_array(d + i, val) };
                        }
                    } else {
                        return Err(LuaError::IndexError(table.lua_type()));
                    }
                }

                // Global operations
                Opcode::GGET => {
                    let d = instr.d() as usize;
                    // Get string constant, intern it, and look up in globals
                    let frame = self.state.call_stack.current().unwrap();
                    let val = if let Some(bytes) = frame.get_string_constant(d) {
                        let bytes = bytes.to_vec();
                        drop(frame);
                        let key = self.state.intern_bytes(&bytes);
                        unsafe { (*self.state.globals.as_ptr()).get(&key) }
                    } else {
                        Value::nil()
                    };
                    self.state.stack.set(base + a, val);
                }

                Opcode::GSET => {
                    let d = instr.d() as usize;
                    let val = self.state.stack.get(base + a);
                    // Get string constant and intern it for global key
                    let frame = self.state.call_stack.current().unwrap();
                    if let Some(bytes) = frame.get_string_constant(d) {
                        let bytes = bytes.to_vec();
                        drop(frame);
                        let key = self.state.intern_bytes(&bytes);
                        unsafe { (*self.state.globals.as_ptr()).set(key, val) };
                    }
                }

                // Upvalue operations
                Opcode::UGET => {
                    let d = instr.d() as usize;
                    let frame = self.state.call_stack.current().unwrap();
                    if let Some(closure) = frame.closure {
                        let val = unsafe {
                            let c = &*closure.as_ptr();
                            if let Some(uv) = c.upvalues.get(d) {
                                let uv_ref = &*uv.as_ptr();
                                // Use stack-based access for open upvalues
                                uv_ref.get_from_stack(self.state.stack.all_values())
                            } else {
                                Value::nil()
                            }
                        };
                        self.state.stack.set(base + a, val);
                    }
                }

                Opcode::USETV => {
                    let d = instr.d() as usize;
                    let val = self.state.stack.get(base + d);
                    let frame = self.state.call_stack.current().unwrap();
                    if let Some(closure) = frame.closure {
                        unsafe {
                            let c = &*closure.as_ptr();
                            if let Some(uv) = c.upvalues.get(a) {
                                let uv_ref = &*uv.as_ptr();
                                // Use stack-based access for open upvalues
                                uv_ref.set_in_stack(self.state.stack.all_values_mut(), val);
                            }
                        }
                    }
                }

                // Function calls
                Opcode::CALL => {
                    let b = instr.b() as usize; // nargs + 1
                    let c = instr.c() as i32;   // nresults + 1

                    let nargs = if b == 0 {
                        // Variable args from previous call: calculate based on stack top
                        self.state.stack.top().saturating_sub(base + a + 1)
                    } else {
                        b - 1
                    };

                    // Detect method call pattern: look for MOV copying self in the instructions
                    // before CALL. Pattern: TGETS/TGETV; MOV(a+1, self); [args]; CALL(a, ...)
                    let is_method = if nargs >= 1 {
                        let frame = self.state.call_stack.current().unwrap();
                        let pc = frame.pc;
                        if let Some(closure) = frame.closure {
                            let proto = unsafe { &*(*closure.as_ptr()).proto.as_ptr() };
                            // Look back through previous instructions for the MOV pattern
                            // The MOV for self should be within the last few instructions
                            let search_limit = (pc.saturating_sub(1)).min(10);
                            let mut found = false;
                            for i in 1..=search_limit {
                                if pc < i + 1 { break; }
                                let check_instr = proto.code[pc - 1 - i];
                                if check_instr.opcode() == Opcode::MOV &&
                                   check_instr.a() == (a + 1) as u8 {
                                    found = true;
                                    break;
                                }
                            }
                            found
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    self.state.is_method_call = is_method;

                    let nresults = c - 1;
                    let result = self.call(base + a, nargs, nresults);
                    self.state.is_method_call = false;
                    result?;
                }

                Opcode::CALLT => {
                    // Tail call
                    let b = instr.b() as usize;
                    let nargs = if b == 0 {
                        self.state.stack.top() - base - a - 1
                    } else {
                        b - 1
                    };

                    // Move function and args to base - 1
                    let dst = base - 1;
                    for i in 0..=nargs {
                        let val = self.state.stack.get(base + a + i);
                        self.state.stack.set(dst + i, val);
                    }

                    // Pop current frame and call
                    self.state.call_stack.pop();
                    let frame = self.state.call_stack.current().unwrap();
                    let nresults = frame.num_results;

                    return self.call(dst, nargs, nresults);
                }

                // Return operations
                Opcode::RET0 => {
                    // No results - set top to func_idx to indicate 0 results
                    return self.do_return(func_idx, 0);
                }

                Opcode::RET1 => {
                    let val = self.state.stack.get(base + a);
                    self.state.stack.set(func_idx, val);
                    return self.do_return(func_idx, 1);
                }

                Opcode::RET => {
                    let d = instr.d() as usize;
                    let nrets = d - 1;

                    // Move results to func_idx (where caller expects them)
                    for i in 0..nrets {
                        let val = self.state.stack.get(base + a + i);
                        self.state.stack.set(func_idx + i, val);
                    }

                    return self.do_return(func_idx, nrets);
                }

                // Loop operations
                Opcode::FORI => {
                    // For loop init: base+a = idx, base+a+1 = limit, base+a+2 = step
                    // Use coerce_to_number to handle string-to-number conversion
                    let idx = self.state.stack.get(base + a).coerce_to_number();
                    let limit = self.state.stack.get(base + a + 1).coerce_to_number();
                    let step = self.state.stack.get(base + a + 2).coerce_to_number();

                    match (idx, limit, step) {
                        (Some(i), Some(l), Some(s)) => {
                            // Store converted numbers back to stack for FORL
                            self.state.stack.set(base + a, Value::number(i));
                            self.state.stack.set(base + a + 1, Value::number(l));
                            self.state.stack.set(base + a + 2, Value::number(s));

                            // Copy to loop variable
                            self.state.stack.set(base + a + 3, Value::number(i));

                            // Check if loop should run
                            let should_run = if s >= 0.0 { i <= l } else { i >= l };
                            if !should_run {
                                let offset = instr.jump();
                                let frame = self.state.call_stack.current_mut().unwrap();
                                frame.jump(offset);
                            }
                        }
                        _ => {
                            return Err(LuaError::RuntimeError(
                                "'for' limit must be a number".to_string(),
                            ));
                        }
                    }
                }

                Opcode::FORL | Opcode::IFORL => {
                    let idx = self.state.stack.get(base + a).as_number().unwrap();
                    let limit = self.state.stack.get(base + a + 1).as_number().unwrap();
                    let step = self.state.stack.get(base + a + 2).as_number().unwrap();

                    let new_idx = idx + step;
                    self.state.stack.set(base + a, Value::number(new_idx));
                    self.state.stack.set(base + a + 3, Value::number(new_idx));

                    let continue_loop = if step >= 0.0 {
                        new_idx <= limit
                    } else {
                        new_idx >= limit
                    };

                    if continue_loop {
                        let offset = instr.jump();
                        let frame = self.state.call_stack.current_mut().unwrap();
                        frame.jump(offset);
                    }
                }

                Opcode::ITERL | Opcode::IITERL => {
                    // Iterator loop
                    let val = self.state.stack.get(base + a);
                    if !val.is_nil() {
                        // Copy control variable
                        self.state.stack.set(base + a - 1, val);
                        // Jump back to loop body
                        let offset = instr.jump();
                        let frame = self.state.call_stack.current_mut().unwrap();
                        frame.jump(offset);
                    }
                }

                Opcode::ITERC => {
                    // Call iterator: A = where to put results, B = iterator base (func, state, var)
                    let b = instr.b() as usize;
                    let iter_base = base + b;
                    let func = self.state.stack.get(iter_base);
                    let state_val = self.state.stack.get(iter_base + 1);
                    let var = self.state.stack.get(iter_base + 2);

                    // Set up call: base+a = func, base+a+1 = state, base+a+2 = var
                    self.state.stack.set(base + a, func);
                    self.state.stack.set(base + a + 1, state_val);
                    self.state.stack.set(base + a + 2, var);

                    let c = instr.c() as i32;
                    self.call(base + a, 2, c - 1)?;
                }

                Opcode::LOOP | Opcode::ILOOP => {
                    // Generic loop marker, used for JIT hot detection
                    // In interpreter, just continue
                }

                // Concatenation
                Opcode::CAT => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;

                    // Helper to convert a value to string if possible
                    fn to_concat_string(val: &Value, state: &State) -> Option<String> {
                        if let Some(n) = val.as_number() {
                            if let Some(i) = val.as_integer() {
                                Some(format!("{}", i))
                            } else {
                                Some(format!("{}", n))
                            }
                        } else if let Some(s) = val.as_string() {
                            let s = unsafe { &*s.as_ptr() };
                            s.as_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    }

                    // Start with the rightmost value and work left
                    // This handles metamethods correctly: a..b..c = a..(b..c)
                    // Always keep intermediate result at base+c on the stack to protect from GC
                    for i in (b..c).rev() {
                        // Always read from stack to get latest values
                        let left = self.state.stack.get(base + i);
                        let right = self.state.stack.get(base + c);

                        // Try direct string conversion first
                        match (to_concat_string(&left, &self.state), to_concat_string(&right, &self.state)) {
                            (Some(l), Some(r)) => {
                                let result_str = format!("{}{}", l, r);
                                // Track string allocation for GC
                                self.state.gc.track_external_alloc(result_str.len() + 32);
                                let result = self.state.intern_string(&result_str);
                                // Store on stack immediately to protect from GC
                                self.state.stack.set(base + c, result);
                            }
                            _ => {
                                // Try __concat metamethod from left, then right
                                if let Some(mm) = self.get_metamethod(&left, "__concat") {
                                    self.call_binary_metamethod(mm, left, right, base + c, "__concat")?;
                                    // Result is already at base + c
                                } else if let Some(mm) = self.get_metamethod(&right, "__concat") {
                                    self.call_binary_metamethod(mm, left, right, base + c, "__concat")?;
                                    // Result is already at base + c
                                } else {
                                    // No metamethod - error
                                    return Err(LuaError::ConcatError(
                                        if to_concat_string(&left, &self.state).is_none() {
                                            left.lua_type()
                                        } else {
                                            right.lua_type()
                                        }
                                    ));
                                }
                            }
                        }
                    }
                    // Copy final result from base+c to base+a
                    let result = self.state.stack.get(base + c);
                    self.state.stack.set(base + a, result);
                    // Check if GC should run after string allocation
                    // Set context so __gc debug info shows "__concat"
                    self.state.gc_trigger_context = Some("__concat".to_string());
                    self.state.check_gc();
                    self.state.gc_trigger_context = None;
                }

                // Vararg
                Opcode::VARG => {
                    let b = instr.b() as usize; // Number of varargs wanted + 1
                    let frame = self.state.call_stack.current().unwrap();
                    let num_varargs = frame.vararg_count;

                    if let Some(vbase) = frame.vararg_base {
                        let wanted = if b == 0 {
                            num_varargs
                        } else {
                            b - 1
                        };

                        for i in 0..wanted {
                            let val = if i < num_varargs {
                                self.state.stack.get(vbase + i)
                            } else {
                                Value::nil()
                            };
                            self.state.stack.set(base + a + i, val);
                        }

                        if b == 0 {
                            self.state.stack.set_top(base + a + wanted);
                        }
                    } else {
                        // No varargs, fill with nil
                        let wanted = if b == 0 { 0 } else { b - 1 };
                        for i in 0..wanted {
                            self.state.stack.set(base + a + i, Value::nil());
                        }
                    }
                }

                // Function header (no-op in interpreter)
                Opcode::FUNCF | Opcode::IFUNCF | Opcode::FUNCV | Opcode::IFUNCV => {
                    // Function headers are handled at call time
                }

                // Create closure
                Opcode::FNEW => {
                    let d = instr.d() as usize;
                    let frame = self.state.call_stack.current().unwrap();
                    if let Some(parent_closure) = frame.closure {
                        let parent_proto = unsafe { &*(*parent_closure.as_ptr()).proto.as_ptr() };
                        if let Some(child_proto_ref) = parent_proto.protos.get(d) {
                            let child_proto = unsafe { &*child_proto_ref.as_ptr() };
                            let mut new_closure = Closure::new(*child_proto_ref, Some(self.state.globals));

                            // Set up upvalues based on the child proto's upvalue descriptors
                            for uv_desc in &child_proto.upvalues {
                                let upvalue = if uv_desc.in_stack {
                                    // Capture from current stack frame - use index, not pointer
                                    let abs_slot = base + uv_desc.index as usize;
                                    // Find or create an upvalue for this slot index
                                    self.state.find_or_create_upvalue(abs_slot)
                                } else {
                                    // Get from parent closure's upvalues
                                    unsafe {
                                        let parent = &*parent_closure.as_ptr();
                                        if let Some(uv) = parent.upvalues.get(uv_desc.index as usize) {
                                            *uv
                                        } else {
                                            self.state.gc.alloc(Upvalue::new_closed(Value::nil()))
                                        }
                                    }
                                };
                                new_closure.upvalues.push(upvalue);
                            }

                            let func = Function::Lua(new_closure);
                            let func_ref = self.state.gc.alloc(func);
                            self.state.stack.set(base + a, Value::function(func_ref));
                            // Check if GC should run after allocation
                            self.state.check_gc();
                        }
                    }
                }

                // Close upvalues
                Opcode::UCLO => {
                    // Close upvalues for slots >= A (using index, not pointer)
                    let close_slot = base + a;
                    self.state.close_upvalues(close_slot);

                    // Handle jump if present
                    let offset = instr.jump();
                    if offset != 0 {
                        let frame = self.state.call_stack.current_mut().unwrap();
                        frame.jump(offset);
                    }
                }

                _ => {
                    return Err(LuaError::RuntimeError(format!(
                        "unimplemented opcode: {:?}",
                        op
                    )));
                }
            }
        }
    }

    fn do_return(&mut self, results_base: usize, nresults: usize) -> LuaResult<()> {
        // Return handled by caller
        self.state.stack.set_top(results_base + nresults);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpreter_creation() {
        let mut state = State::new();
        let _interp = Interpreter::new(&mut state);
    }
}
