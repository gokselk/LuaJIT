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
                    Function::Lua(closure) => {
                        self.call_lua(func_idx, nargs, nresults)
                    }
                    Function::Native(native) => {
                        self.call_native(func_idx, nargs, nresults)
                    }
                }
            },
            None => Err(LuaError::CallError(func.lua_type())),
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
        let base = func_idx + 1;

        // Save caller's stack state
        let saved_base = self.state.stack.base();
        let saved_top = self.state.stack.top();

        // Adjust arguments to match parameters
        let num_params = proto.num_params as usize;
        let current_args = nargs;

        if current_args < num_params {
            // Fill missing args with nil
            for i in current_args..num_params {
                self.state.stack.set(base + i, Value::nil());
            }
        }

        // Handle varargs
        let vararg_base = if proto.is_vararg && current_args > num_params {
            let vbase = base + num_params;
            Some(vbase)
        } else {
            None
        };

        // Create call frame
        let mut frame = CallFrame::new_lua(closure, base, nresults);
        frame.vararg_base = vararg_base;
        frame.top = base + proto.max_stack_size as usize;

        // Set up stack
        self.state.stack.set_base(base);
        self.state.stack.set_top(frame.top);
        self.state.call_stack.push(frame);

        // Execute
        let result = self.execute();

        // Clean up - restore caller's stack state
        self.state.call_stack.pop();
        self.state.stack.set_base(saved_base);
        // The return value is at func_idx; set top to include it
        self.state.stack.set_top(func_idx + 1.max(saved_top));

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
        let frame = CallFrame::new_native(base, nresults);
        self.state.call_stack.push(frame);

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
                    let string_val = if let Some(s) = frame.get_string_constant(d) {
                        // Need to copy to avoid borrow issues
                        let s = s.to_string();
                        drop(frame);
                        self.state.intern_string(&s)
                    } else {
                        Value::nil()
                    };
                    self.state.stack.set(base + a, string_val);
                }

                // Arithmetic operations
                Opcode::ADDVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    let result = vb.add(&vc)?;
                    self.state.stack.set(base + a, result);
                }

                Opcode::SUBVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    let result = vb.sub(&vc)?;
                    self.state.stack.set(base + a, result);
                }

                Opcode::MULVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    let result = vb.mul(&vc)?;
                    self.state.stack.set(base + a, result);
                }

                Opcode::DIVVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    let result = vb.div(&vc)?;
                    self.state.stack.set(base + a, result);
                }

                Opcode::MODVV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    let result = vb.modulo(&vc)?;
                    self.state.stack.set(base + a, result);
                }

                Opcode::POW => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let vb = self.state.stack.get(base + b);
                    let vc = self.state.stack.get(base + c);
                    let result = vb.pow(&vc)?;
                    self.state.stack.set(base + a, result);
                }

                Opcode::UNM => {
                    let d = instr.d() as usize;
                    let vd = self.state.stack.get(base + d);
                    let result = vd.unm()?;
                    self.state.stack.set(base + a, result);
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
                    let len = if let Some(t) = vd.as_table() {
                        unsafe { (*t.as_ptr()).len() as f64 }
                    } else if let Some(s) = vd.as_string() {
                        unsafe { (*s.as_ptr()).len() as f64 }
                    } else {
                        return Err(LuaError::LengthError(vd.lua_type()));
                    };
                    self.state.stack.set(base + a, Value::number(len));
                }

                // Comparison operations
                Opcode::ISLT | Opcode::ISGE | Opcode::ISLE | Opcode::ISGT => {
                    let d = instr.d() as usize;
                    let va = self.state.stack.get(base + a);
                    let vd = self.state.stack.get(base + d);

                    let cmp = match (va.as_number(), vd.as_number()) {
                        (Some(a), Some(b)) => match op {
                            Opcode::ISLT => a < b,
                            Opcode::ISGE => a >= b,
                            Opcode::ISLE => a <= b,
                            Opcode::ISGT => a > b,
                            _ => unreachable!(),
                        },
                        _ => return Err(LuaError::CompareError(va.lua_type(), vd.lua_type())),
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
                    let eq = va.raw_eq(&vd);
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
                }

                Opcode::TGETV => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + b);
                    let key = self.state.stack.get(base + c);

                    if let Some(t) = table.as_table() {
                        let val = unsafe { (*t.as_ptr()).get(&key) };
                        self.state.stack.set(base + a, val);
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
                    let key = if let Some(s) = frame.get_string_constant(c) {
                        let s = s.to_string();
                        drop(frame);
                        self.state.intern_string(&s)
                    } else {
                        frame.get_constant(c)
                    };

                    if let Some(t) = table.as_table() {
                        let val = unsafe { (*t.as_ptr()).get(&key) };
                        self.state.stack.set(base + a, val);
                    } else {
                        return Err(LuaError::IndexError(table.lua_type()));
                    }
                }

                Opcode::TGETB => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + b);

                    if let Some(t) = table.as_table() {
                        let val = unsafe { (*t.as_ptr()).get_array(c) };
                        self.state.stack.set(base + a, val);
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

                    if let Some(t) = table.as_table() {
                        unsafe { (*t.as_ptr()).set(key, val) };
                    } else {
                        return Err(LuaError::IndexError(table.lua_type()));
                    }
                }

                Opcode::TSETS => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + a);
                    let val = self.state.stack.get(base + b);
                    let frame = self.state.call_stack.current().unwrap();
                    // Get string constant and intern it for key
                    let key = if let Some(s) = frame.get_string_constant(c) {
                        let s = s.to_string();
                        drop(frame);
                        self.state.intern_string(&s)
                    } else {
                        frame.get_constant(c)
                    };

                    if let Some(t) = table.as_table() {
                        unsafe { (*t.as_ptr()).set(key, val) };
                    } else {
                        return Err(LuaError::IndexError(table.lua_type()));
                    }
                }

                Opcode::TSETB => {
                    let b = instr.b() as usize;
                    let c = instr.c() as usize;
                    let table = self.state.stack.get(base + a);
                    let val = self.state.stack.get(base + b);

                    if let Some(t) = table.as_table() {
                        unsafe { (*t.as_ptr()).set_array(c, val) };
                    } else {
                        return Err(LuaError::IndexError(table.lua_type()));
                    }
                }

                // Global operations
                Opcode::GGET => {
                    let d = instr.d() as usize;
                    // Get string constant, intern it, and look up in globals
                    let frame = self.state.call_stack.current().unwrap();
                    let val = if let Some(s) = frame.get_string_constant(d) {
                        let s = s.to_string();
                        drop(frame);
                        let key = self.state.intern_string(&s);
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
                    if let Some(s) = frame.get_string_constant(d) {
                        let s = s.to_string();
                        drop(frame);
                        let key = self.state.intern_string(&s);
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
                                (*uv.as_ptr()).get()
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
                                (*uv.as_ptr()).set(val);
                            }
                        }
                    }
                }

                // Function calls
                Opcode::CALL => {
                    let b = instr.b() as usize; // nargs + 1
                    let c = instr.c() as i32;   // nresults + 1

                    let nargs = if b == 0 {
                        self.state.stack.top() - base - a - 1
                    } else {
                        b - 1
                    };

                    let nresults = c - 1;
                    self.call(base + a, nargs, nresults)?;
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
                    return self.do_return(base, 0);
                }

                Opcode::RET1 => {
                    let val = self.state.stack.get(base + a);
                    self.state.stack.set(base - 1, val);
                    return self.do_return(base - 1, 1);
                }

                Opcode::RET => {
                    let d = instr.d() as usize;
                    let nrets = d - 1;

                    // Move results
                    let dst = base - 1;
                    for i in 0..nrets {
                        let val = self.state.stack.get(base + a + i);
                        self.state.stack.set(dst + i, val);
                    }

                    return self.do_return(dst, nrets);
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

                    let mut result = String::new();
                    for i in b..=c {
                        let val = self.state.stack.get(base + i);
                        if let Some(n) = val.as_number() {
                            result.push_str(&format!("{}", n));
                        } else if let Some(s) = val.as_string() {
                            let s = unsafe { &*s.as_ptr() };
                            if let Some(str_val) = s.as_str() {
                                result.push_str(str_val);
                            }
                        } else {
                            return Err(LuaError::ConcatError(val.lua_type()));
                        }
                    }

                    let str_val = self.state.intern_string(&result);
                    self.state.stack.set(base + a, str_val);
                }

                // Vararg
                Opcode::VARG => {
                    let b = instr.b() as usize; // Number of varargs wanted + 1
                    let frame = self.state.call_stack.current().unwrap();

                    if let Some(vbase) = frame.vararg_base {
                        let num_varargs = base - vbase;
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
                                    // Capture from current stack frame
                                    let abs_slot = base + uv_desc.index as usize;
                                    let slot_ptr = self.state.stack.slot_ptr(abs_slot);
                                    let uv = Upvalue::new_open(slot_ptr);
                                    self.state.gc.alloc(uv)
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
                        }
                    }
                }

                // Close upvalues
                Opcode::UCLO => {
                    // Close upvalues (simplified - would need upvalue list)
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
