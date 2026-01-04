//! Base library - core Lua functions.

use crate::value::{Value, LuaError, LuaResult, LuaType, Userdata, Table};
use crate::vm::State;

/// Register base library functions
pub fn register_base(state: &mut State) {
    state.register_function("print", lua_print);
    state.register_function("type", lua_type);
    state.register_function("tostring", lua_tostring);
    state.register_function("tonumber", lua_tonumber);
    state.register_function("assert", lua_assert);
    state.register_function("error", lua_error);
    state.register_function("pcall", lua_pcall);
    state.register_function("xpcall", lua_xpcall);
    state.register_function("pairs", lua_pairs);
    state.register_function("ipairs", lua_ipairs);
    state.register_function("next", lua_next);
    state.register_function("select", lua_select);
    state.register_function("rawequal", lua_rawequal);
    state.register_function("rawget", lua_rawget);
    state.register_function("rawset", lua_rawset);
    state.register_function("gcinfo", lua_gcinfo);

    // Store ipairs_iter in registry for internal use
    let ipairs_iter = crate::value::NativeFunction::new(lua_ipairs_iter);
    let ipairs_iter_func = state.gc.alloc(crate::value::Function::Native(ipairs_iter));
    let key = state.intern_string("__ipairs_iter");
    unsafe { (*state.registry.as_ptr()).set(key, Value::function(ipairs_iter_func)); }
    state.register_function("setmetatable", lua_setmetatable);
    state.register_function("getmetatable", lua_getmetatable);
    state.register_function("collectgarbage", lua_collectgarbage);
    state.register_function("load", lua_load);
    state.register_function("loadstring", lua_loadstring);
    state.register_function("loadfile", lua_loadfile);
    state.register_function("dofile", lua_dofile);
    state.register_function("getfenv", lua_getfenv);
    state.register_function("setfenv", lua_setfenv);
    state.register_function("newproxy", lua_newproxy);
    state.register_function("module", lua_module);

    // Add _G (self-reference to globals table)
    let globals = state.globals;
    state.set_global("_G", Value::table(globals));

    // Add _VERSION
    let version = state.intern_string("Lua 5.1");
    state.set_global("_VERSION", version);
}

/// print(...)
fn lua_print(state: &mut State) -> LuaResult<usize> {
    let n = state.get_top();
    let mut output = String::new();

    for i in 1..=n as i32 {
        if i > 1 {
            output.push('\t');
        }
        let val = state.get_value(i);
        output.push_str(&format_value(&val));
    }

    println!("{}", output);
    Ok(0)
}

/// type(v) -> string
fn lua_type(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);
    let type_name = val.lua_type().name();
    let str_val = state.intern_string(type_name);
    state.push(str_val)?;
    Ok(1)
}

/// tostring(v) -> string
fn lua_tostring(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);
    let s = format_value(&val);
    let str_val = state.intern_string(&s);
    state.push(str_val)?;
    Ok(1)
}

/// tonumber(v [, base]) -> number | nil
fn lua_tonumber(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);
    let base = state.to_integer(2).unwrap_or(10);

    let result = if let Some(n) = val.as_number() {
        Value::number(n)
    } else if let Some(s) = val.as_string() {
        // Try to parse string as number
        let s = unsafe { &*s.as_ptr() };
        if let Some(str_val) = s.as_str() {
            if base == 10 {
                str_val.trim().parse::<f64>()
                    .map(Value::number)
                    .unwrap_or(Value::nil())
            } else {
                // Parse with base
                i64::from_str_radix(str_val.trim(), base as u32)
                    .map(|n| Value::number(n as f64))
                    .unwrap_or(Value::nil())
            }
        } else {
            Value::nil()
        }
    } else {
        Value::nil()
    };

    state.push(result)?;
    Ok(1)
}

/// assert(v [, message]) -> v | error
fn lua_assert(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);

    if val.is_falsy() {
        let msg = if state.get_top() >= 2 {
            let msg_val = state.get_value(2);
            format_value(&msg_val)
        } else {
            "assertion failed!".to_string()
        };
        return Err(LuaError::RuntimeError(msg));
    }

    // Return all arguments
    Ok(state.get_top())
}

/// error(message [, level])
fn lua_error(state: &mut State) -> LuaResult<usize> {
    let msg = state.get_value(1);
    Err(LuaError::RuntimeError(format_value(&msg)))
}

/// pcall(f, ...) -> status, result...
fn lua_pcall(state: &mut State) -> LuaResult<usize> {
    let nargs = state.get_top() - 1;

    match state.call(nargs, -1) {
        Ok(()) => {
            // Success - prepend true
            let nresults = state.get_top();
            // Extend stack to make room for the prepended 'true'
            state.set_top(nresults + 1);
            // Shift results right by 1 and add true at the beginning
            for i in (1..=nresults as i32).rev() {
                let val = state.get_value(i);
                state.set_value(i + 1, val);
            }
            state.set_value(1, Value::boolean(true));
            Ok(nresults + 1)
        }
        Err(e) => {
            // Error - return false, error message with location
            state.set_top(0);
            state.push(Value::boolean(false))?;
            let formatted = state.format_error(&e);
            let err_msg = state.intern_string(&formatted);
            state.push(err_msg)?;
            Ok(2)
        }
    }
}

/// xpcall(f, err, ...) -> status, result...
fn lua_xpcall(state: &mut State) -> LuaResult<usize> {
    let err_handler = state.get_value(2);
    let top = state.get_top();
    let nargs = top - 2; // Number of args to pass to f (excluding f and err)

    // Stack layout: [f, err, arg1, arg2, ...]
    // We need to rearrange to: [f, arg1, arg2, ...]
    // Shift arguments left by 1 to remove the error handler
    for i in 3..=top as i32 {
        let val = state.get_value(i);
        state.set_value(i - 1, val);
    }
    state.set_top(top - 1); // Remove the extra slot

    // Now stack is [f, arg1, arg2, ...] with nargs arguments
    match state.call(nargs, -1) {
        Ok(()) => {
            let nresults = state.get_top();
            // Extend stack to make room for the prepended 'true'
            state.set_top(nresults + 1);
            // Shift results right by 1 and add true at the beginning
            for i in (1..=nresults as i32).rev() {
                let val = state.get_value(i);
                state.set_value(i + 1, val);
            }
            state.set_value(1, Value::boolean(true));
            Ok(nresults + 1)
        }
        Err(e) => {
            // Call the error handler with the error message (with location)
            state.set_top(0);
            let formatted = state.format_error(&e);
            let err_msg = state.intern_string(&formatted);

            if err_handler.is_function() {
                state.push(err_handler)?;
                state.push(err_msg)?;
                match state.call(1, 1) {
                    Ok(()) => {
                        // Get the transformed error message
                        let transformed = state.get_value(1);
                        state.set_top(0);
                        state.push(Value::boolean(false))?;
                        state.push(transformed)?;
                        Ok(2)
                    }
                    Err(_) => {
                        // Error in error handler - return original error
                        state.set_top(0);
                        state.push(Value::boolean(false))?;
                        state.push(err_msg)?;
                        Ok(2)
                    }
                }
            } else {
                state.push(Value::boolean(false))?;
                state.push(err_msg)?;
                Ok(2)
            }
        }
    }
}

/// pairs(t) -> next, t, nil
fn lua_pairs(state: &mut State) -> LuaResult<usize> {
    let table = state.get_value(1);
    if !table.is_table() {
        return Err(LuaError::ArgumentError {
            func: "pairs".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        });
    }

    // Return next, table, nil
    let next_fn = state.get_global("next");
    state.push(next_fn)?;
    state.push(table)?;
    state.push(Value::nil())?;
    Ok(3)
}

/// ipairs iterator function - increments index and returns table[index]
fn lua_ipairs_iter(state: &mut State) -> LuaResult<usize> {
    let table = state.get_value(1);
    let index = state.get_value(2);

    if let (Some(t), Some(i)) = (table.as_table(), index.as_integer()) {
        let next_i = i + 1;
        let t = unsafe { &*t.as_ptr() };
        let val = t.get(&Value::integer(next_i));
        if !val.is_nil() {
            state.push(Value::integer(next_i))?;
            state.push(val)?;
            Ok(2)
        } else {
            Ok(0) // End iteration
        }
    } else {
        Ok(0)
    }
}

/// ipairs(t) -> iterator, t, 0
fn lua_ipairs(state: &mut State) -> LuaResult<usize> {
    let table = state.get_value(1);
    if !table.is_table() {
        return Err(LuaError::ArgumentError {
            func: "ipairs".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        });
    }

    // Get ipairs iterator from registry
    let key = state.intern_string("__ipairs_iter");
    let registry = unsafe { &*state.registry.as_ptr() };
    let iter = registry.get(&key);
    state.push(iter)?;
    state.push(table)?;
    state.push(Value::integer(0))?;
    Ok(3)
}

/// next(table [, index]) -> key, value
fn lua_next(state: &mut State) -> LuaResult<usize> {
    let table = state.get_value(1);
    let key = state.get_value(2);

    if let Some(t) = table.as_table() {
        let t = unsafe { &*t.as_ptr() };
        match t.next_checked(&key) {
            Ok(Some((next_key, next_val))) => {
                state.push(next_key)?;
                state.push(next_val)?;
                Ok(2)
            }
            Ok(None) => {
                // End of iteration
                state.push(Value::nil())?;
                Ok(1)
            }
            Err(()) => {
                // Invalid key
                Err(LuaError::RuntimeError("invalid key to 'next'".to_string()))
            }
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "next".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

/// select(index, ...) -> value(s)
fn lua_select(state: &mut State) -> LuaResult<usize> {
    let n = state.get_top();
    let index = state.get_value(1);

    if let Some(s) = index.as_string() {
        let s = unsafe { &*s.as_ptr() };
        if s.as_str() == Some("#") {
            // Clear stack and push just the count
            state.set_top(0);
            state.push(Value::integer((n - 1) as i32))?;
            return Ok(1);
        }
    }

    if let Some(i) = index.as_integer() {
        let i = if i >= 0 {
            i as usize
        } else {
            // Negative index counts from end
            let pos = (n as i32 + i) as usize;
            if pos == 0 { 1 } else { pos }
        };

        if i > 0 && i <= n - 1 {
            // Return all values from index onwards
            // Values are at positions 2, 3, ..., n (1 is the index itself)
            // We want values starting from position i+1
            let start = i + 1;
            let count = n - i;

            // Move values from position start..n to 1..count
            for j in 0..count {
                let val = state.get_value((start + j) as i32);
                state.set_value((1 + j) as i32, val);
            }
            state.set_top(count);
            Ok(count)
        } else if i > n - 1 {
            // Index beyond available args - return nothing (not an error)
            state.set_top(0);
            Ok(0)
        } else {
            // i <= 0 is an error (except for negative indices handled above)
            Err(LuaError::ArgumentError {
                func: "select".to_string(),
                arg: 1,
                msg: "index out of range".to_string(),
            })
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "select".to_string(),
            arg: 1,
            msg: "number expected".to_string(),
        })
    }
}

/// rawequal(v1, v2) -> boolean
fn lua_rawequal(state: &mut State) -> LuaResult<usize> {
    let v1 = state.get_value(1);
    let v2 = state.get_value(2);
    state.push(Value::boolean(v1.raw_eq(&v2)))?;
    Ok(1)
}

/// rawget(table, index) -> value
fn lua_rawget(state: &mut State) -> LuaResult<usize> {
    let table = state.get_value(1);
    let key = state.get_value(2);

    if let Some(t) = table.as_table() {
        let t = unsafe { &*t.as_ptr() };
        let val = t.raw_get(&key);
        state.push(val)?;
        Ok(1)
    } else {
        Err(LuaError::ArgumentError {
            func: "rawget".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

/// rawset(table, index, value) -> table
fn lua_rawset(state: &mut State) -> LuaResult<usize> {
    let table = state.get_value(1);
    let key = state.get_value(2);
    let val = state.get_value(3);

    if let Some(t) = table.as_table() {
        let t = unsafe { &*t.as_ptr() };
        t.raw_set(key, val);
        state.push(table)?;
        Ok(1)
    } else {
        Err(LuaError::ArgumentError {
            func: "rawset".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

/// rawlen(v) -> number
fn lua_rawlen(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);

    let len = if let Some(t) = val.as_table() {
        unsafe { (*t.as_ptr()).len() }
    } else if let Some(s) = val.as_string() {
        unsafe { (*s.as_ptr()).len() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "rawlen".to_string(),
            arg: 1,
            msg: "table or string expected".to_string(),
        });
    };

    state.push(Value::integer(len as i32))?;
    Ok(1)
}

/// setmetatable(table, metatable) -> table
fn lua_setmetatable(state: &mut State) -> LuaResult<usize> {
    let table = state.get_value(1);
    let mt = state.get_value(2);

    if let Some(t) = table.as_table() {
        let t = unsafe { &*t.as_ptr() };

        // Check for __metatable protection
        if let Some(existing_mt) = t.get_metatable() {
            let existing_mt_table = unsafe { &*existing_mt.as_ptr() };
            let key = state.intern_string("__metatable");
            let protected = existing_mt_table.get(&key);
            if !protected.is_nil() {
                return Err(LuaError::RuntimeError(
                    "cannot change a protected metatable".to_string(),
                ));
            }
        }

        if mt.is_nil() {
            t.set_metatable(None);
        } else if let Some(mt_table) = mt.as_table() {
            t.set_metatable(Some(mt_table));
        } else {
            return Err(LuaError::ArgumentError {
                func: "setmetatable".to_string(),
                arg: 2,
                msg: "nil or table expected".to_string(),
            });
        }
        state.push(table)?;
        Ok(1)
    } else {
        Err(LuaError::ArgumentError {
            func: "setmetatable".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

/// getmetatable(object) -> table | nil | __metatable value
fn lua_getmetatable(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);

    // Get metatable from table or userdata
    let mt_opt = if let Some(t) = val.as_table() {
        let t = unsafe { &*t.as_ptr() };
        t.get_metatable()
    } else if let Some(u) = val.as_userdata() {
        let u = unsafe { &*u.as_ptr() };
        u.get_metatable()
    } else {
        // Check type metatable for other types
        None
    };

    if let Some(mt) = mt_opt {
        let mt_table = unsafe { &*mt.as_ptr() };
        // Check for __metatable field (protection)
        let key = state.intern_string("__metatable");
        let protected = mt_table.get(&key);
        if !protected.is_nil() {
            state.push(protected)?;
        } else {
            state.push(Value::table(mt))?;
        }
    } else {
        state.push(Value::nil())?;
    }
    Ok(1)
}

/// collectgarbage([opt [, arg]]) -> varies
fn lua_collectgarbage(state: &mut State) -> LuaResult<usize> {
    let opt = state.get_value(1);

    let opt_str = if let Some(s) = opt.as_string() {
        let s = unsafe { &*s.as_ptr() };
        s.as_str().unwrap_or("collect")
    } else {
        "collect"
    };

    match opt_str {
        "collect" => {
            state.collect_garbage();
            state.push(Value::integer(0))?;
        }
        "count" => {
            let kb = state.memory_used() as f64 / 1024.0;
            state.push(Value::number(kb))?;
        }
        "stop" => {
            state.gc.stop();
            state.push(Value::integer(0))?;
        }
        "restart" => {
            state.gc.restart();
            state.push(Value::integer(0))?;
        }
        "isrunning" => {
            state.push(Value::boolean(state.gc.is_running()))?;
        }
        _ => {
            state.push(Value::integer(0))?;
        }
    }

    Ok(1)
}

/// gcinfo() -> memory_in_use_kb
/// Deprecated in Lua 5.1, but still available for compatibility
fn lua_gcinfo(state: &mut State) -> LuaResult<usize> {
    let kb = state.memory_used() / 1024;
    state.push(Value::integer(kb as i32))?;
    Ok(1)
}

/// Format a value for printing
fn format_value(val: &Value) -> String {
    if val.is_nil() {
        "nil".to_string()
    } else if let Some(b) = val.as_boolean() {
        b.to_string()
    } else if let Some(n) = val.as_number() {
        if let Some(i) = val.as_integer() {
            i.to_string()
        } else {
            format!("{}", n)
        }
    } else if let Some(s) = val.as_string() {
        let s = unsafe { &*s.as_ptr() };
        s.as_str().unwrap_or("<binary>").to_string()
    } else {
        format!("{}: {:p}", val.lua_type().name(), val as *const _)
    }
}

/// load(chunk [, chunkname [, mode [, env]]]) -> function | nil, error
fn lua_load(state: &mut State) -> LuaResult<usize> {
    let chunk_val = state.get_value(1);

    // Get the source - can be a string or a function that returns strings
    let source = if let Some(s) = chunk_val.as_string() {
        let s = unsafe { &*s.as_ptr() };
        if let Some(str_val) = s.as_str() {
            str_val.to_string()
        } else {
            String::from_utf8_lossy(s.as_bytes()).to_string()
        }
    } else if chunk_val.is_function() {
        // Call the function repeatedly to get source chunks
        let mut source = String::new();
        loop {
            let base = state.stack.top();
            state.stack.set(base, chunk_val.clone());
            state.stack.set_top(base + 1);
            if state.call(0, 1).is_err() {
                break;
            }
            let result = state.get_value(1);
            if result.is_nil() {
                break;
            }
            if let Some(s) = result.as_string() {
                let s = unsafe { &*s.as_ptr() };
                if let Some(str_val) = s.as_str() {
                    if str_val.is_empty() {
                        break;
                    }
                    source.push_str(str_val);
                } else {
                    break;
                }
            } else {
                break;
            }
            state.set_top(0);
        }
        source
    } else {
        return Err(LuaError::ArgumentError {
            func: "load".to_string(),
            arg: 1,
            msg: "string or function expected".to_string(),
        });
    };

    let chunk_name = if state.get_top() >= 2 {
        let name_val = state.get_value(2);
        if let Some(s) = name_val.as_string() {
            let s = unsafe { &*s.as_ptr() };
            s.as_str().map(|s| s.to_string())
        } else {
            None
        }
    } else {
        None
    }.unwrap_or_else(|| "=(load)".to_string());

    // Note: mode and env parameters are currently ignored

    match state.load_string(&source, &chunk_name) {
        Ok(func) => {
            state.push(Value::function(func))?;
            Ok(1)
        }
        Err(e) => {
            state.push(Value::nil())?;
            let err_msg = state.intern_string(&e.to_string());
            state.push(err_msg)?;
            Ok(2)
        }
    }
}

/// loadstring(string [, chunkname]) -> function | nil, error
fn lua_loadstring(state: &mut State) -> LuaResult<usize> {
    let source_val = state.get_value(1);

    let str_ref = source_val.as_string().ok_or_else(|| LuaError::ArgumentError {
        func: "loadstring".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })?;

    let lua_str = unsafe { &*str_ref.as_ptr() };
    let bytes = lua_str.as_bytes();

    // Check if this is bytecode (starts with our magic) - handle before UTF-8 conversion
    if bytes.starts_with(crate::stdlib::string::BYTECODE_MAGIC) {
        match state.load_bytecode(bytes) {
            Ok(func) => {
                state.push(Value::function(func))?;
                return Ok(1);
            }
            Err(e) => {
                state.push(Value::nil())?;
                let err_msg = state.intern_string(&e.to_string());
                state.push(err_msg)?;
                return Ok(2);
            }
        }
    }

    // Regular source code - convert to string
    let source = if let Some(str_val) = lua_str.as_str() {
        str_val.to_string()
    } else {
        // Binary string - try to interpret as UTF-8 lossy
        String::from_utf8_lossy(bytes).to_string()
    };

    let chunk_name = if state.get_top() >= 2 {
        let name_val = state.get_value(2);
        if let Some(s) = name_val.as_string() {
            let s = unsafe { &*s.as_ptr() };
            s.as_str().map(|s| s.to_string())
        } else {
            None
        }
    } else {
        None
    }.unwrap_or_else(|| "=(loadstring)".to_string());

    match state.load_string(&source, &chunk_name) {
        Ok(func) => {
            state.push(Value::function(func))?;
            Ok(1)
        }
        Err(e) => {
            // On error, return nil and error message
            state.push(Value::nil())?;
            let err_msg = state.intern_string(&e.to_string());
            state.push(err_msg)?;
            Ok(2)
        }
    }
}

/// loadfile([filename [, mode [, env]]]) -> function | nil, error
fn lua_loadfile(state: &mut State) -> LuaResult<usize> {
    let filename_val = state.get_value(1);

    let filename = if filename_val.is_nil() {
        // Read from stdin
        return Err(LuaError::RuntimeError(
            "loadfile from stdin not yet implemented".to_string(),
        ));
    } else if let Some(s) = filename_val.as_string() {
        let s = unsafe { &*s.as_ptr() };
        s.as_str()
            .ok_or_else(|| LuaError::ArgumentError {
                func: "loadfile".to_string(),
                arg: 1,
                msg: "string expected".to_string(),
            })?
            .to_string()
    } else {
        return Err(LuaError::ArgumentError {
            func: "loadfile".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    // Read file contents
    let source = match std::fs::read_to_string(&filename) {
        Ok(s) => s,
        Err(e) => {
            state.push(Value::nil())?;
            let err_msg = state.intern_string(&format!("cannot open {}: {}", filename, e));
            state.push(err_msg)?;
            return Ok(2);
        }
    };

    let chunk_name = format!("@{}", filename);

    match state.load_string(&source, &chunk_name) {
        Ok(func) => {
            state.push(Value::function(func))?;
            Ok(1)
        }
        Err(e) => {
            state.push(Value::nil())?;
            let err_msg = state.intern_string(&e.to_string());
            state.push(err_msg)?;
            Ok(2)
        }
    }
}

/// dofile([filename]) -> results...
fn lua_dofile(state: &mut State) -> LuaResult<usize> {
    let filename_val = state.get_value(1);

    let filename = if filename_val.is_nil() {
        return Err(LuaError::RuntimeError(
            "dofile from stdin not yet implemented".to_string(),
        ));
    } else if let Some(s) = filename_val.as_string() {
        let s = unsafe { &*s.as_ptr() };
        s.as_str()
            .ok_or_else(|| LuaError::ArgumentError {
                func: "dofile".to_string(),
                arg: 1,
                msg: "string expected".to_string(),
            })?
            .to_string()
    } else {
        return Err(LuaError::ArgumentError {
            func: "dofile".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    // Read file contents
    let source = std::fs::read_to_string(&filename)
        .map_err(|e| LuaError::RuntimeError(format!("cannot open {}: {}", filename, e)))?;

    let chunk_name = format!("@{}", filename);
    let func = state.load_string(&source, &chunk_name)?;

    // Clear stack and push the function
    state.set_top(0);
    state.push(Value::function(func))?;

    // Call with 0 args, variable results
    state.call(0, -1)?;

    // Return all results
    Ok(state.get_top())
}

/// getfenv([f]) - stub
fn lua_getfenv(state: &mut State) -> LuaResult<usize> {
    // Return globals table as a stub
    state.push(Value::table(state.globals))?;
    Ok(1)
}

/// setfenv(f, table) - stub
fn lua_setfenv(_state: &mut State) -> LuaResult<usize> {
    // Stub - just return first arg
    Ok(1)
}

/// newproxy([boolean|proxy]) - create a new proxy userdata
/// If called with no arguments or false, creates a proxy without metatable.
/// If called with true, creates a proxy with an empty metatable.
/// If called with another proxy, creates a proxy sharing the same metatable.
fn lua_newproxy(state: &mut State) -> LuaResult<usize> {
    // Allocate an empty proxy userdata using GC
    let proxy = state.gc.alloc(Userdata::new_proxy());

    // Check argument for metatable
    if state.get_top() >= 1 {
        let arg = state.get_value(1);

        if let Some(true) = arg.as_boolean() {
            // Create metatable FIRST, set it, THEN register for finalization
            // This ensures the metatable exists when GC checks for __gc
            let mt = state.gc.alloc(Table::with_capacity(0, 4));
            unsafe { (*proxy.as_ptr()).set_metatable(Some(mt)); }
            state.register_finalizable(proxy);
        } else if let Some(other_proxy) = arg.as_userdata() {
            // Share metatable with another proxy
            let mt = unsafe { (*other_proxy.as_ptr()).get_metatable() };
            unsafe { (*proxy.as_ptr()).set_metatable(mt); }
            if mt.is_some() {
                state.register_finalizable(proxy);
            }
        }
    }

    state.push(Value::userdata(proxy))?;
    Ok(1)
}

/// module(name, ...) - stub
fn lua_module(_state: &mut State) -> LuaResult<usize> {
    Err(LuaError::RuntimeError("module not yet implemented".to_string()))
}
