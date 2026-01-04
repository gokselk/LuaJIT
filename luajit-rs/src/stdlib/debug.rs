//! Debug library - debugging and introspection functions.

use crate::value::{Value, LuaError, LuaResult, LuaType, Table};
use crate::vm::State;

pub fn register_debug(state: &mut State) {
    // Create debug table
    let debug_lib = state.create_table(0, 4);

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

    // For now, just return the message with "stack traceback:" prefix
    let result = if msg.is_empty() {
        "stack traceback:".to_string()
    } else {
        format!("{}\nstack traceback:", msg)
    };

    let result_val = state.intern_string(&result);
    state.push(result_val)?;
    Ok(1)
}
