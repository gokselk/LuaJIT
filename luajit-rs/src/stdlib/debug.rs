//! Debug library - debugging and introspection functions.

use crate::value::{Value, LuaError, LuaResult, LuaType, Table, Function};
use crate::vm::State;

pub fn register_debug(state: &mut State) {
    // Create debug table
    let debug_lib = state.create_table(0, 8);

    // Helper to add a function to the table
    let add_func = |state: &mut State, tbl: crate::value::GcRef<Table>, name: &str, func: crate::value::NativeFn| {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    // Add functions to debug table
    add_func(state, debug_lib, "debug", debug_debug);
    add_func(state, debug_lib, "getfenv", debug_getfenv);
    add_func(state, debug_lib, "gethook", debug_gethook);
    add_func(state, debug_lib, "getinfo", debug_getinfo);
    add_func(state, debug_lib, "getlocal", debug_getlocal);
    add_func(state, debug_lib, "getmetatable", debug_getmetatable);
    add_func(state, debug_lib, "getregistry", debug_getregistry);
    add_func(state, debug_lib, "getupvalue", debug_getupvalue);
    add_func(state, debug_lib, "setfenv", debug_setfenv);
    add_func(state, debug_lib, "sethook", debug_sethook);
    add_func(state, debug_lib, "setlocal", debug_setlocal);
    add_func(state, debug_lib, "setmetatable", debug_setmetatable);
    add_func(state, debug_lib, "setupvalue", debug_setupvalue);
    add_func(state, debug_lib, "traceback", debug_traceback);
    add_func(state, debug_lib, "upvalueid", debug_upvalueid);
    add_func(state, debug_lib, "upvaluejoin", debug_upvaluejoin);

    state.set_global("debug", Value::table(debug_lib));
}

/// debug.getmetatable(value) -> metatable | nil
/// Unlike the base getmetatable, this returns the raw metatable without checking __metatable.
fn debug_getmetatable(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);

    // Get metatable based on type
    let mt = if let Some(t) = val.as_table() {
        let table = unsafe { &*t.as_ptr() };
        table.get_metatable()
    } else if let Some(u) = val.as_userdata() {
        let userdata = unsafe { &*u.as_ptr() };
        userdata.get_metatable()
    } else {
        // Get type metatable for primitive types
        let type_index = val.lua_type() as usize;
        if type_index < state.metatables.len() {
            state.metatables[type_index]
        } else {
            None
        }
    };

    if let Some(mt) = mt {
        state.push(Value::table(mt))?;
    } else {
        state.push(Value::nil())?;
    }
    Ok(1)
}

/// debug.setmetatable(value, table) -> value
/// Sets the metatable for any value. For tables and userdata, sets the instance metatable.
/// For other types, sets the type metatable.
fn debug_setmetatable(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);
    let mt_val = state.get_value(2);

    // Get the metatable (or nil)
    let new_mt = if mt_val.is_nil() {
        None
    } else if let Some(t) = mt_val.as_table() {
        Some(t)
    } else {
        return Err(LuaError::ArgumentError {
            func: "debug.setmetatable".to_string(),
            arg: 2,
            msg: "nil or table expected".to_string(),
        });
    };

    // Set metatable based on type
    if let Some(t) = val.as_table() {
        let table = unsafe { &*t.as_ptr() };
        table.set_metatable(new_mt);
    } else if let Some(u) = val.as_userdata() {
        let userdata = unsafe { &*u.as_ptr() };
        userdata.set_metatable(new_mt);
    } else {
        // Set type metatable for primitive types
        let type_index = val.lua_type() as usize;
        if type_index < state.metatables.len() {
            state.metatables[type_index] = new_mt;
        }
    }

    // Return the original value
    state.push(val)?;
    Ok(1)
}

/// debug.traceback([thread,] [message [, level]]) -> string
/// Returns a string with a traceback of the call stack.
fn debug_traceback(state: &mut State) -> LuaResult<usize> {
    let msg = if state.get_top() >= 1 {
        let v = state.get_value(1);
        if let Some(s) = v.as_string() {
            let s = unsafe { &*s.as_ptr() };
            s.as_str().map(|s| s.to_string()).unwrap_or_default()
        } else {
            "".to_string()
        }
    } else {
        "".to_string()
    };

    let level = state.to_integer(2).unwrap_or(1) as usize;

    // Build a real stack traceback
    // Get stack frames from call stack
    let frames = state.call_stack.frames();
    let num_frames = frames.len();

    // First, count how many Lua frames we can show
    let mut lua_frame_count = 0;
    for i in (0..num_frames).rev() {
        if i < level {
            continue;
        }
        let frame = &frames[i];
        if frame.closure.is_some() {
            lua_frame_count += 1;
        }
    }

    // If no Lua frames to show, just return the message without traceback
    if lua_frame_count == 0 {
        let result_val = state.intern_string(&msg);
        state.push(result_val)?;
        return Ok(1);
    }

    let mut result = if msg.is_empty() {
        "stack traceback:".to_string()
    } else {
        format!("{}\nstack traceback:", msg)
    };

    // Skip 'level' frames and show up to 11 entries
    let mut shown = 0;
    let max_show = 11;

    for i in (0..num_frames).rev() {
        if i < level {
            continue;
        }
        if shown >= max_show {
            result.push_str("\n\t...");
            break;
        }

        let frame = &frames[i];
        if let Some(closure) = frame.closure {
            let proto = unsafe { &*(*closure.as_ptr()).proto.as_ptr() };
            let source = if let Some(src_ref) = proto.source {
                let src = unsafe { &*src_ref.as_ptr() };
                src.as_str().unwrap_or("?").to_string()
            } else {
                "?".to_string()
            };
            let line = if frame.pc > 0 && frame.pc <= proto.lineinfo.len() {
                proto.lineinfo[frame.pc - 1] as i32
            } else {
                proto.line_defined as i32
            };
            result.push_str(&format!("\n\t{}:{}: in function", source, line));
        } else if frame.is_native {
            result.push_str("\n\t[C]: in function");
        }
        shown += 1;
    }

    let result_val = state.intern_string(&result);
    state.push(result_val)?;
    Ok(1)
}

/// debug.getinfo(f [, what]) -> table
/// Returns a table with information about a function or stack level.
fn debug_getinfo(state: &mut State) -> LuaResult<usize> {
    let f = state.get_value(1);
    let what = if state.get_top() >= 2 {
        let w = state.get_value(2);
        if let Some(s) = w.as_string() {
            unsafe { (*s.as_ptr()).as_str().unwrap_or("flnSu").to_string() }
        } else {
            "flnSu".to_string()
        }
    } else {
        "flnSu".to_string()
    };

    let info = state.create_table(0, 8);

    // Handle function vs stack level
    if let Some(func_ref) = f.as_function() {
        let func = unsafe { &*func_ref.as_ptr() };

        match func {
            Function::Lua(closure) => {
                let proto = unsafe { &*closure.proto.as_ptr() };

                if what.contains('S') {
                    // Source info
                    let source = if let Some(src_ref) = proto.source {
                        let src = unsafe { &*src_ref.as_ptr() };
                        src.as_str().unwrap_or("?").to_string()
                    } else {
                        "?".to_string()
                    };
                    let source_key = state.intern_string("source");
                    let source_val = state.intern_string(&source);
                    unsafe { (*info.as_ptr()).set(source_key, source_val); }

                    let short_src_key = state.intern_string("short_src");
                    let short_src = if source.len() > 60 {
                        format!("{}...", &source[..57])
                    } else {
                        source.clone()
                    };
                    let short_src_val = state.intern_string(&short_src);
                    unsafe { (*info.as_ptr()).set(short_src_key, short_src_val); }

                    let linedefined_key = state.intern_string("linedefined");
                    unsafe { (*info.as_ptr()).set(linedefined_key, Value::integer(proto.line_defined as i32)); }

                    let lastlinedefined_key = state.intern_string("lastlinedefined");
                    unsafe { (*info.as_ptr()).set(lastlinedefined_key, Value::integer(proto.last_line_defined as i32)); }

                    let what_key = state.intern_string("what");
                    let what_val = state.intern_string("Lua");
                    unsafe { (*info.as_ptr()).set(what_key, what_val); }
                }

                if what.contains('l') {
                    let currentline_key = state.intern_string("currentline");
                    unsafe { (*info.as_ptr()).set(currentline_key, Value::integer(-1)); }
                }

                if what.contains('u') {
                    let nups_key = state.intern_string("nups");
                    unsafe { (*info.as_ptr()).set(nups_key, Value::integer(closure.upvalues.len() as i32)); }

                    let nparams_key = state.intern_string("nparams");
                    unsafe { (*info.as_ptr()).set(nparams_key, Value::integer(proto.num_params as i32)); }

                    let isvararg_key = state.intern_string("isvararg");
                    unsafe { (*info.as_ptr()).set(isvararg_key, Value::boolean(proto.is_vararg)); }
                }

                if what.contains('n') {
                    let name_key = state.intern_string("name");
                    unsafe { (*info.as_ptr()).set(name_key, Value::nil()); }

                    let namewhat_key = state.intern_string("namewhat");
                    let namewhat_val = state.intern_string("");
                    unsafe { (*info.as_ptr()).set(namewhat_key, namewhat_val); }
                }

                if what.contains('f') {
                    let func_key = state.intern_string("func");
                    unsafe { (*info.as_ptr()).set(func_key, f); }
                }
            }
            Function::Native(_) => {
                if what.contains('S') {
                    let source_key = state.intern_string("source");
                    let source_val = state.intern_string("=[C]");
                    unsafe { (*info.as_ptr()).set(source_key, source_val); }

                    let short_src_key = state.intern_string("short_src");
                    let short_src_val = state.intern_string("[C]");
                    unsafe { (*info.as_ptr()).set(short_src_key, short_src_val); }

                    let linedefined_key = state.intern_string("linedefined");
                    unsafe { (*info.as_ptr()).set(linedefined_key, Value::integer(-1)); }

                    let lastlinedefined_key = state.intern_string("lastlinedefined");
                    unsafe { (*info.as_ptr()).set(lastlinedefined_key, Value::integer(-1)); }

                    let what_key = state.intern_string("what");
                    let what_val = state.intern_string("C");
                    unsafe { (*info.as_ptr()).set(what_key, what_val); }
                }

                if what.contains('l') {
                    let currentline_key = state.intern_string("currentline");
                    unsafe { (*info.as_ptr()).set(currentline_key, Value::integer(-1)); }
                }

                if what.contains('u') {
                    let nups_key = state.intern_string("nups");
                    unsafe { (*info.as_ptr()).set(nups_key, Value::integer(0)); }
                }

                if what.contains('f') {
                    let func_key = state.intern_string("func");
                    unsafe { (*info.as_ptr()).set(func_key, f); }
                }
            }
        }
    } else if let Some(level) = f.as_number() {
        // Stack level - get info about the function at that level
        let level = level as usize;

        // Check level bounds first
        let num_frames = state.call_stack.frames().len();
        if level >= num_frames {
            state.push(Value::nil())?;
            return Ok(1);
        }

        // Extract frame info without holding borrow of state
        let frame_info = {
            let frames = state.call_stack.frames();
            let frame = &frames[frames.len() - 1 - level];
            let name = frame.name.clone();
            let name_what = frame.name_what.clone();
            frame.closure.map(|closure| {
                let proto = unsafe { &*(*closure.as_ptr()).proto.as_ptr() };
                let source = if let Some(src_ref) = proto.source {
                    let src = unsafe { &*src_ref.as_ptr() };
                    src.as_str().unwrap_or("?").to_string()
                } else {
                    "?".to_string()
                };
                let line_defined = proto.line_defined as i32;
                let current_line = if frame.pc > 0 && frame.pc <= proto.lineinfo.len() {
                    proto.lineinfo[frame.pc - 1] as i32
                } else {
                    -1
                };
                (source, line_defined, current_line, name, name_what)
            })
        };

        // Check if we're in a metamethod or have call name info (for __call)
        let current_mm = state.current_metamethod.clone();
        let call_name = state.call_name.clone();
        let call_name_what = state.call_name_what.clone();

        if let Some((source, line_defined, current_line, name, name_what)) = frame_info {
            if what.contains('S') {
                let source_key = state.intern_string("source");
                let source_val = state.intern_string(&source);
                unsafe { (*info.as_ptr()).set(source_key, source_val); }

                let linedefined_key = state.intern_string("linedefined");
                unsafe { (*info.as_ptr()).set(linedefined_key, Value::integer(line_defined)); }

                let what_key = state.intern_string("what");
                let what_val = state.intern_string("Lua");
                unsafe { (*info.as_ptr()).set(what_key, what_val); }
            }

            if what.contains('l') {
                let currentline_key = state.intern_string("currentline");
                unsafe { (*info.as_ptr()).set(currentline_key, Value::integer(current_line)); }
            }

            if what.contains('n') {
                // Check if we're in a metamethod first
                if let Some(ref mm_name) = current_mm {
                    let name_key = state.intern_string("name");
                    let name_val = state.intern_string(mm_name);
                    unsafe { (*info.as_ptr()).set(name_key, name_val); }

                    let namewhat_key = state.intern_string("namewhat");
                    let namewhat_val = state.intern_string("metamethod");
                    unsafe { (*info.as_ptr()).set(namewhat_key, namewhat_val); }
                } else if let Some(ref cn) = call_name {
                    // __call: report how the called object was accessed
                    let name_key = state.intern_string("name");
                    let name_val = state.intern_string(cn);
                    unsafe { (*info.as_ptr()).set(name_key, name_val); }

                    let namewhat_key = state.intern_string("namewhat");
                    let namewhat_val = state.intern_string(call_name_what.as_deref().unwrap_or(""));
                    unsafe { (*info.as_ptr()).set(namewhat_key, namewhat_val); }
                } else if let Some(ref n) = name {
                    let name_key = state.intern_string("name");
                    let name_val = state.intern_string(n);
                    unsafe { (*info.as_ptr()).set(name_key, name_val); }

                    let namewhat_key = state.intern_string("namewhat");
                    let namewhat_val = state.intern_string(name_what.as_deref().unwrap_or(""));
                    unsafe { (*info.as_ptr()).set(namewhat_key, namewhat_val); }
                } else {
                    let name_key = state.intern_string("name");
                    unsafe { (*info.as_ptr()).set(name_key, Value::nil()); }

                    let namewhat_key = state.intern_string("namewhat");
                    let namewhat_val = state.intern_string("");
                    unsafe { (*info.as_ptr()).set(namewhat_key, namewhat_val); }
                }
            }
        }
    }

    state.push(Value::table(info))?;
    Ok(1)
}

/// debug.sethook([thread,] hook, mask [, count]) -> ()
/// Sets the hook function for the current thread.
fn debug_sethook(state: &mut State) -> LuaResult<usize> {
    // For now, we just accept and ignore hooks
    // Full hook implementation requires interpreter modifications
    Ok(0)
}

/// debug.gethook([thread]) -> hook, mask, count
/// Returns the current hook settings.
fn debug_gethook(state: &mut State) -> LuaResult<usize> {
    // Return nil, empty mask, 0 count (no hook set)
    state.push(Value::nil())?;
    let empty = state.intern_string("");
    state.push(empty)?;
    state.push(Value::integer(0))?;
    Ok(3)
}

/// debug.getlocal([thread,] level, index) -> name, value
/// Returns the name and value of a local variable.
fn debug_getlocal(state: &mut State) -> LuaResult<usize> {
    // Simplified implementation - just return nil for now
    state.push(Value::nil())?;
    Ok(1)
}

/// debug.setlocal([thread,] level, index, value) -> name
/// Sets the value of a local variable.
fn debug_setlocal(state: &mut State) -> LuaResult<usize> {
    // Simplified implementation - just return nil for now
    state.push(Value::nil())?;
    Ok(1)
}

/// debug.debug() -> nil
/// Enters interactive debug mode (simplified - just returns immediately)
fn debug_debug(_state: &mut State) -> LuaResult<usize> {
    // In a real implementation, this would enter an interactive debugging REPL
    // For now, we just return immediately
    Ok(0)
}

/// debug.getfenv(o) -> table
/// Returns the environment table of a function or thread.
fn debug_getfenv(state: &mut State) -> LuaResult<usize> {
    let o = state.get_value(1);

    // In Lua 5.1/LuaJIT, functions have environments
    // For now, return the global environment
    if o.is_function() || o.is_nil() || o.as_integer() == Some(0) {
        // Return globals table for functions or the main thread (0)
        state.push(Value::table(state.globals))?;
        Ok(1)
    } else if let Some(n) = o.as_integer() {
        // Stack level
        if n > 0 {
            // Return globals for now (simplified)
            state.push(Value::table(state.globals))?;
            Ok(1)
        } else {
            Err(LuaError::ArgumentError {
                func: "getfenv".to_string(),
                arg: 1,
                msg: "level must be non-negative".to_string(),
            })
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "getfenv".to_string(),
            arg: 1,
            msg: "function or level expected".to_string(),
        })
    }
}

/// debug.setfenv(object, table) -> object
/// Sets the environment table of a function.
fn debug_setfenv(state: &mut State) -> LuaResult<usize> {
    let o = state.get_value(1);
    let env = state.get_value(2);

    if !env.is_table() {
        return Err(LuaError::ArgumentError {
            func: "setfenv".to_string(),
            arg: 2,
            msg: "table expected".to_string(),
        });
    }

    // In a full implementation, this would set the function's _ENV
    // For now, just return the object
    state.push(o)?;
    Ok(1)
}

/// debug.getregistry() -> table
/// Returns the registry table.
fn debug_getregistry(state: &mut State) -> LuaResult<usize> {
    state.push(Value::table(state.registry))?;
    Ok(1)
}

/// debug.getupvalue(f, up) -> name, value
/// Returns the name and value of upvalue up of function f.
fn debug_getupvalue(state: &mut State) -> LuaResult<usize> {
    let f = state.get_value(1);
    let up = state.to_integer(2).unwrap_or(0) as usize;

    if up < 1 {
        state.push(Value::nil())?;
        return Ok(1);
    }

    if let Some(func_ref) = f.as_function() {
        let func = unsafe { &*func_ref.as_ptr() };

        match func {
            Function::Lua(closure) => {
                let upvalue_index = up - 1; // 1-based to 0-based
                if upvalue_index < closure.upvalues.len() {
                    // Get upvalue name from prototype if available
                    let proto = unsafe { &*closure.proto.as_ptr() };
                    let name = if upvalue_index < proto.upvalue_names.len() {
                        if let Some(name_ref) = proto.upvalue_names[upvalue_index] {
                            let name_str = unsafe { &*name_ref.as_ptr() };
                            name_str.as_str().unwrap_or("").to_string()
                        } else {
                            "".to_string()
                        }
                    } else {
                        "".to_string()
                    };

                    // Get upvalue value
                    let upval_ref = &closure.upvalues[upvalue_index];
                    let upval = unsafe { &*upval_ref.as_ptr() };
                    let value = upval.get();

                    let name_val = state.intern_string(&name);
                    state.push(name_val)?;
                    state.push(value)?;
                    return Ok(2);
                }
            }
            Function::Native(_) => {
                // Native functions don't have upvalues accessible this way
            }
        }
    }

    state.push(Value::nil())?;
    Ok(1)
}

/// debug.setupvalue(f, up, value) -> name
/// Sets the value of upvalue up of function f.
fn debug_setupvalue(state: &mut State) -> LuaResult<usize> {
    let f = state.get_value(1);
    let up = state.to_integer(2).unwrap_or(0) as usize;
    let value = state.get_value(3);

    if up < 1 {
        state.push(Value::nil())?;
        return Ok(1);
    }

    if let Some(func_ref) = f.as_function() {
        let func = unsafe { &mut *func_ref.as_ptr() };

        match func {
            Function::Lua(closure) => {
                let upvalue_index = up - 1;
                if upvalue_index < closure.upvalues.len() {
                    // Get upvalue name from prototype
                    let proto = unsafe { &*closure.proto.as_ptr() };
                    let name = if upvalue_index < proto.upvalue_names.len() {
                        if let Some(name_ref) = proto.upvalue_names[upvalue_index] {
                            let name_str = unsafe { &*name_ref.as_ptr() };
                            name_str.as_str().unwrap_or("").to_string()
                        } else {
                            "".to_string()
                        }
                    } else {
                        "".to_string()
                    };

                    // Set upvalue value
                    let upval = unsafe { &*closure.upvalues[upvalue_index].as_ptr() };
                    upval.set(value);

                    let name_val = state.intern_string(&name);
                    state.push(name_val)?;
                    return Ok(1);
                }
            }
            Function::Native(_) => {
                // Native functions don't have upvalues
            }
        }
    }

    state.push(Value::nil())?;
    Ok(1)
}

/// debug.upvalueid(f, n) -> id
/// Returns a unique identifier for upvalue n of function f.
fn debug_upvalueid(state: &mut State) -> LuaResult<usize> {
    let f = state.get_value(1);
    let n = state.to_integer(2).unwrap_or(0) as usize;

    if n < 1 {
        return Err(LuaError::ArgumentError {
            func: "upvalueid".to_string(),
            arg: 2,
            msg: "invalid upvalue index".to_string(),
        });
    }

    if let Some(func_ref) = f.as_function() {
        let func = unsafe { &*func_ref.as_ptr() };

        match func {
            Function::Lua(closure) => {
                let upvalue_index = n - 1;
                if upvalue_index < closure.upvalues.len() {
                    // Return the address of the upvalue as a light userdata
                    let upval_ptr = &closure.upvalues[upvalue_index] as *const _ as *mut ();
                    state.push(Value::light_userdata(upval_ptr))?;
                    return Ok(1);
                }
            }
            Function::Native(_) => {}
        }
    }

    Err(LuaError::ArgumentError {
        func: "upvalueid".to_string(),
        arg: 2,
        msg: "invalid upvalue index".to_string(),
    })
}

/// debug.upvaluejoin(f1, n1, f2, n2)
/// Makes the n1-th upvalue of f1 refer to the n2-th upvalue of f2.
fn debug_upvaluejoin(state: &mut State) -> LuaResult<usize> {
    let f1 = state.get_value(1);
    let n1 = state.to_integer(2).unwrap_or(0) as usize;
    let f2 = state.get_value(3);
    let n2 = state.to_integer(4).unwrap_or(0) as usize;

    if n1 < 1 || n2 < 1 {
        return Err(LuaError::ArgumentError {
            func: "upvaluejoin".to_string(),
            arg: if n1 < 1 { 2 } else { 4 },
            msg: "invalid upvalue index".to_string(),
        });
    }

    // Validate both functions are Lua closures
    let func1_ref = f1.as_function().ok_or_else(|| LuaError::ArgumentError {
        func: "upvaluejoin".to_string(),
        arg: 1,
        msg: "Lua function expected".to_string(),
    })?;

    let func2_ref = f2.as_function().ok_or_else(|| LuaError::ArgumentError {
        func: "upvaluejoin".to_string(),
        arg: 3,
        msg: "Lua function expected".to_string(),
    })?;

    let func1 = unsafe { &mut *func1_ref.as_ptr() };
    let func2 = unsafe { &*func2_ref.as_ptr() };

    match (func1, func2) {
        (Function::Lua(closure1), Function::Lua(closure2)) => {
            let idx1 = n1 - 1;
            let idx2 = n2 - 1;

            if idx1 >= closure1.upvalues.len() || idx2 >= closure2.upvalues.len() {
                return Err(LuaError::ArgumentError {
                    func: "upvaluejoin".to_string(),
                    arg: if idx1 >= closure1.upvalues.len() { 2 } else { 4 },
                    msg: "invalid upvalue index".to_string(),
                });
            }

            // Clone the upvalue reference from closure2 to closure1
            closure1.upvalues[idx1] = closure2.upvalues[idx2].clone();
            Ok(0)
        }
        _ => Err(LuaError::ArgumentError {
            func: "upvaluejoin".to_string(),
            arg: 1,
            msg: "Lua function expected".to_string(),
        }),
    }
}
