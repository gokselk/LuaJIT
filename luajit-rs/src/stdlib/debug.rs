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
    add_func(state, debug_lib, "getmetatable", debug_getmetatable);
    add_func(state, debug_lib, "setmetatable", debug_setmetatable);
    add_func(state, debug_lib, "traceback", debug_traceback);
    add_func(state, debug_lib, "getinfo", debug_getinfo);
    add_func(state, debug_lib, "sethook", debug_sethook);
    add_func(state, debug_lib, "gethook", debug_gethook);
    add_func(state, debug_lib, "getlocal", debug_getlocal);
    add_func(state, debug_lib, "setlocal", debug_setlocal);

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
    let mut result = if msg.is_empty() {
        "stack traceback:".to_string()
    } else {
        format!("{}\nstack traceback:", msg)
    };

    // Get stack frames from call stack
    let frames = state.call_stack.frames();
    let num_frames = frames.len();

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
                (source, line_defined, current_line)
            })
        };

        if let Some((source, line_defined, current_line)) = frame_info {
            if what.contains('S') {
                let source_key = state.intern_string("source");
                let source_val = state.intern_string(&source);
                unsafe { (*info.as_ptr()).set(source_key, source_val); }

                let linedefined_key = state.intern_string("linedefined");
                unsafe { (*info.as_ptr()).set(linedefined_key, Value::integer(line_defined)); }
            }

            if what.contains('l') {
                let currentline_key = state.intern_string("currentline");
                unsafe { (*info.as_ptr()).set(currentline_key, Value::integer(current_line)); }
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
