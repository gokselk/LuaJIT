//! String library

use crate::value::{Value, LuaError, LuaResult};
use crate::vm::State;

pub fn register_string(state: &mut State) {
    // Create string table
    let string_table = state.create_table(0, 16);

    // Helper to add a function to the string table
    let add_func = |state: &mut State, tbl: crate::value::GcRef<crate::value::Table>, name: &str, func: crate::value::NativeFn| {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    // Add functions to string table
    add_func(state, string_table, "byte", string_byte);
    add_func(state, string_table, "char", string_char);
    add_func(state, string_table, "len", string_len);
    add_func(state, string_table, "lower", string_lower);
    add_func(state, string_table, "upper", string_upper);
    add_func(state, string_table, "rep", string_rep);
    add_func(state, string_table, "reverse", string_reverse);
    add_func(state, string_table, "sub", string_sub);
    add_func(state, string_table, "format", string_format);

    state.set_global("string", Value::table(string_table));
}

fn string_byte(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let i = state.to_integer(2).unwrap_or(1);
    let j = state.to_integer(3).unwrap_or(i);

    if let Some(str_ref) = s.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        let bytes = str_val.as_bytes();
        let len = bytes.len() as i32;

        let start = if i >= 0 { i - 1 } else { len + i }.max(0) as usize;
        let end = if j >= 0 { j } else { len + j + 1 }.min(len) as usize;

        if start < end && start < bytes.len() {
            for byte in &bytes[start..end.min(bytes.len())] {
                state.push(Value::integer(*byte as i32))?;
            }
            Ok(end - start)
        } else {
            Ok(0)
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "string.byte".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        })
    }
}

fn string_char(state: &mut State) -> LuaResult<usize> {
    let n = state.get_top();
    let mut result: Vec<u8> = Vec::with_capacity(n);

    for i in 1..=n as i32 {
        let c = state.to_integer(i).ok_or_else(|| LuaError::ArgumentError {
            func: "string.char".to_string(),
            arg: i as usize,
            msg: "number expected".to_string(),
        })?;
        if c < 0 || c > 255 {
            return Err(LuaError::ArgumentError {
                func: "string.char".to_string(),
                arg: i as usize,
                msg: "value out of range".to_string(),
            });
        }
        result.push(c as u8);
    }

    let val = state.intern_bytes(&result);
    state.push(val)?;
    Ok(1)
}

fn string_len(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    if let Some(str_ref) = s.as_string() {
        let len = unsafe { (*str_ref.as_ptr()).len() };
        state.push(Value::integer(len as i32))?;
        Ok(1)
    } else {
        Err(LuaError::ArgumentError {
            func: "string.len".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        })
    }
}

fn string_lower(state: &mut State) -> LuaResult<usize> {
    let v = state.get_value(1);
    if let Some(str_ref) = v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        // Convert to lowercase byte by byte (ASCII only, non-ASCII bytes unchanged)
        let result: Vec<u8> = lua_str.as_bytes().iter().map(|&b| b.to_ascii_lowercase()).collect();
        let val = state.intern_bytes(&result);
        state.push(val)?;
        return Ok(1);
    }
    // Try number-to-string coercion
    if let Some(s) = state.to_lua_string(1) {
        let result: Vec<u8> = s.bytes().map(|b| b.to_ascii_lowercase()).collect();
        let val = state.intern_bytes(&result);
        state.push(val)?;
        return Ok(1);
    }
    Err(LuaError::ArgumentError {
        func: "string.lower".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

fn string_upper(state: &mut State) -> LuaResult<usize> {
    let v = state.get_value(1);
    if let Some(str_ref) = v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        // Convert to uppercase byte by byte (ASCII only, non-ASCII bytes unchanged)
        let result: Vec<u8> = lua_str.as_bytes().iter().map(|&b| b.to_ascii_uppercase()).collect();
        let val = state.intern_bytes(&result);
        state.push(val)?;
        return Ok(1);
    }
    // Try number-to-string coercion
    if let Some(s) = state.to_lua_string(1) {
        let result: Vec<u8> = s.bytes().map(|b| b.to_ascii_uppercase()).collect();
        let val = state.intern_bytes(&result);
        state.push(val)?;
        return Ok(1);
    }
    Err(LuaError::ArgumentError {
        func: "string.upper".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

fn string_rep(state: &mut State) -> LuaResult<usize> {
    // Get string bytes (either raw string or coerced number)
    let v = state.get_value(1);
    let bytes: Vec<u8> = if let Some(str_ref) = v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        lua_str.as_bytes().to_vec()
    } else if let Some(s) = state.to_lua_string(1) {
        s.into_bytes()
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.rep".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let n = state.to_integer(2).unwrap_or(0);

    if n <= 0 {
        let val = state.intern_bytes(&[]);
        state.push(val)?;
        return Ok(1);
    }

    // Get separator bytes
    let sep_v = state.get_value(3);
    let sep_bytes: Option<Vec<u8>> = if let Some(str_ref) = sep_v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        Some(lua_str.as_bytes().to_vec())
    } else if let Some(s) = state.to_lua_string(3) {
        Some(s.into_bytes())
    } else if !sep_v.is_nil() {
        return Err(LuaError::ArgumentError {
            func: "string.rep".to_string(),
            arg: 3,
            msg: "string expected".to_string(),
        });
    } else {
        None
    };

    let result = if let Some(sep) = sep_bytes {
        // With separator: repeat with separator between copies
        let mut result = Vec::with_capacity(bytes.len() * n as usize + sep.len() * (n as usize - 1));
        for i in 0..n {
            if i > 0 {
                result.extend_from_slice(&sep);
            }
            result.extend_from_slice(&bytes);
        }
        result
    } else {
        // No separator: simple repeat
        bytes.repeat(n as usize)
    };

    let val = state.intern_bytes(&result);
    state.push(val)?;
    Ok(1)
}

fn string_reverse(state: &mut State) -> LuaResult<usize> {
    let v = state.get_value(1);
    if let Some(str_ref) = v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        let bytes = lua_str.as_bytes();
        let mut reversed: Vec<u8> = bytes.iter().copied().rev().collect();
        let val = state.intern_bytes(&reversed);
        state.push(val)?;
        return Ok(1);
    }
    // Try number-to-string coercion
    if let Some(s) = state.to_lua_string(1) {
        let reversed: Vec<u8> = s.bytes().rev().collect();
        let val = state.intern_bytes(&reversed);
        state.push(val)?;
        return Ok(1);
    }
    Err(LuaError::ArgumentError {
        func: "string.reverse".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

fn string_sub(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let i = state.to_integer(2).unwrap_or(1);
    let j = state.to_integer(3).unwrap_or(-1);

    if let Some(str_ref) = s.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        if let Some(s) = str_val.as_str() {
            let len = s.len() as i32;
            let start = if i >= 0 { i - 1 } else { (len + i).max(0) } as usize;
            let end = if j >= 0 { j } else { len + j + 1 } as usize;

            let result = if start < end && start < s.len() {
                s[start..end.min(s.len())].to_string()
            } else {
                String::new()
            };
            let val = state.intern_string(&result);
            state.push(val)?;
            return Ok(1);
        }
    }
    Err(LuaError::ArgumentError {
        func: "string.sub".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

fn string_format(state: &mut State) -> LuaResult<usize> {
    // Simplified format - only handles basic cases
    let fmt = state.get_value(1);
    if let Some(str_ref) = fmt.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        if let Some(fmt_str) = str_val.as_str() {
            // Very simplified - just return the format string
            let result = fmt_str.to_string();
            let val = state.intern_string(&result);
            state.push(val)?;
            return Ok(1);
        }
    }
    Err(LuaError::ArgumentError {
        func: "string.format".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}
