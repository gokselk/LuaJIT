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
    add_func(state, string_table, "find", string_find);
    add_func(state, string_table, "match", string_match);
    add_func(state, string_table, "gsub", string_gsub);
    add_func(state, string_table, "gmatch", string_gmatch);
    add_func(state, string_table, "dump", string_dump);

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
    let fmt = state.get_value(1);
    if let Some(str_ref) = fmt.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        if let Some(fmt_str) = str_val.as_str() {
            let mut result = String::new();
            let mut chars = fmt_str.chars().peekable();
            let mut arg_idx = 2i32;

            while let Some(c) = chars.next() {
                if c == '%' {
                    match chars.peek() {
                        Some('%') => {
                            chars.next();
                            result.push('%');
                        }
                        Some('s') => {
                            chars.next();
                            if let Some(s) = state.to_lua_string(arg_idx) {
                                result.push_str(&s);
                            }
                            arg_idx += 1;
                        }
                        Some('d') | Some('i') => {
                            chars.next();
                            if let Some(n) = state.to_integer(arg_idx) {
                                result.push_str(&n.to_string());
                            }
                            arg_idx += 1;
                        }
                        Some('f') | Some('g') | Some('e') => {
                            chars.next();
                            if let Some(n) = state.to_number(arg_idx) {
                                result.push_str(&format!("{}", n));
                            }
                            arg_idx += 1;
                        }
                        Some('x') => {
                            chars.next();
                            if let Some(n) = state.to_integer(arg_idx) {
                                result.push_str(&format!("{:x}", n as u32));
                            }
                            arg_idx += 1;
                        }
                        Some('X') => {
                            chars.next();
                            if let Some(n) = state.to_integer(arg_idx) {
                                result.push_str(&format!("{:X}", n as u32));
                            }
                            arg_idx += 1;
                        }
                        Some('c') => {
                            chars.next();
                            if let Some(n) = state.to_integer(arg_idx) {
                                result.push(char::from_u32(n as u32).unwrap_or('?'));
                            }
                            arg_idx += 1;
                        }
                        Some('q') => {
                            chars.next();
                            if let Some(s) = state.to_lua_string(arg_idx) {
                                result.push('"');
                                for c in s.chars() {
                                    match c {
                                        '"' | '\\' | '\n' => {
                                            result.push('\\');
                                            result.push(c);
                                        }
                                        _ => result.push(c),
                                    }
                                }
                                result.push('"');
                            }
                            arg_idx += 1;
                        }
                        _ => result.push(c),
                    }
                } else {
                    result.push(c);
                }
            }

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

/// Convert Lua pattern to a simple regex-compatible form
/// This is a simplified implementation that handles common cases
fn lua_pattern_to_regex(pattern: &str) -> String {
    let mut result = String::new();
    let mut chars = pattern.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '%' => {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    match next {
                        'd' => result.push_str("[0-9]"),
                        'a' => result.push_str("[a-zA-Z]"),
                        'l' => result.push_str("[a-z]"),
                        'u' => result.push_str("[A-Z]"),
                        'w' => result.push_str("[a-zA-Z0-9]"),
                        's' => result.push_str("[ \\t\\n\\r\\f\\v]"),
                        'p' => result.push_str("[!-/:-@\\[-`{-~]"),
                        'c' => result.push_str("[\\x00-\\x1f\\x7f]"),
                        'x' => result.push_str("[0-9a-fA-F]"),
                        'z' => result.push_str("\\x00"),
                        // Character class complements
                        'D' => result.push_str("[^0-9]"),
                        'A' => result.push_str("[^a-zA-Z]"),
                        'L' => result.push_str("[^a-z]"),
                        'U' => result.push_str("[^A-Z]"),
                        'W' => result.push_str("[^a-zA-Z0-9]"),
                        'S' => result.push_str("[^ \\t\\n\\r\\f\\v]"),
                        // Escaped special characters
                        _ => {
                            if "^$()%.[]*+-?".contains(next) {
                                result.push('\\');
                            }
                            result.push(next);
                        }
                    }
                }
            }
            // Escape regex special characters
            '^' | '$' | '(' | ')' | '.' | '[' | ']' | '+' | '?' | '{' | '}' | '|' | '\\' => {
                // Lua uses different anchors
                if c == '^' && result.is_empty() {
                    result.push('^'); // Start anchor
                } else if c == '$' {
                    result.push('$'); // End anchor
                } else {
                    result.push('\\');
                    result.push(c);
                }
            }
            '*' => result.push_str("*?"), // Lua * is non-greedy by default
            '-' => result.push_str("*?"), // Lua - is non-greedy *
            _ => result.push(c),
        }
    }

    result
}

fn string_find(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let pattern = state.get_value(2);
    let init = state.to_integer(3).unwrap_or(1);
    let plain = state.get_value(4).as_boolean().unwrap_or(false);

    let s_str = if let Some(str_ref) = s.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.find".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let pat_str = if let Some(str_ref) = pattern.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.find".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    // Handle negative indices
    let start_idx = if init >= 1 {
        (init - 1) as usize
    } else {
        (s_str.len() as i32 + init).max(0) as usize
    };

    if start_idx >= s_str.len() {
        return Ok(0); // Not found
    }

    let search_str = &s_str[start_idx..];

    if plain {
        // Plain text search
        if let Some(pos) = search_str.find(&pat_str) {
            let found_start = start_idx + pos + 1; // 1-based
            let found_end = found_start + pat_str.len() - 1;
            state.push(Value::integer(found_start as i32))?;
            state.push(Value::integer(found_end as i32))?;
            return Ok(2);
        }
    } else {
        // Pattern search - try simple literal match first
        if let Some(pos) = search_str.find(&pat_str) {
            let found_start = start_idx + pos + 1;
            let found_end = found_start + pat_str.len() - 1;
            state.push(Value::integer(found_start as i32))?;
            state.push(Value::integer(found_end as i32))?;
            return Ok(2);
        }
    }

    Ok(0) // Not found
}

fn string_match(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let pattern = state.get_value(2);
    let init = state.to_integer(3).unwrap_or(1);

    let s_str = if let Some(str_ref) = s.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.match".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let pat_str = if let Some(str_ref) = pattern.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.match".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    // Handle negative indices
    let start_idx = if init >= 1 {
        (init - 1) as usize
    } else {
        (s_str.len() as i32 + init).max(0) as usize
    };

    if start_idx >= s_str.len() {
        return Ok(0); // Not found
    }

    let search_str = &s_str[start_idx..];

    // Simple literal match
    if let Some(pos) = search_str.find(&pat_str) {
        let matched = &search_str[pos..pos + pat_str.len()];
        let val = state.intern_string(matched);
        state.push(val)?;
        return Ok(1);
    }

    Ok(0) // Not found
}

fn string_gsub(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let pattern = state.get_value(2);
    let repl = state.get_value(3);
    let max_n = state.to_integer(4);

    let s_str = if let Some(str_ref) = s.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gsub".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let pat_str = if let Some(str_ref) = pattern.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gsub".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    let repl_str = if let Some(str_ref) = repl.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else if repl.is_function() {
        // Function replacement not fully supported yet
        String::new()
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gsub".to_string(),
            arg: 3,
            msg: "string/function/table expected".to_string(),
        });
    };

    let max_replacements = max_n.unwrap_or(i32::MAX) as usize;

    // Simple string replacement
    let mut result = s_str.clone();
    let mut count = 0;

    if !pat_str.is_empty() {
        while let Some(pos) = result.find(&pat_str) {
            if count >= max_replacements {
                break;
            }
            result = format!("{}{}{}", &result[..pos], repl_str, &result[pos + pat_str.len()..]);
            count += 1;
        }
    }

    let val = state.intern_string(&result);
    state.push(val)?;
    state.push(Value::integer(count as i32))?;
    Ok(2)
}

fn string_gmatch(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let pattern = state.get_value(2);

    let s_str = if let Some(str_ref) = s.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gmatch".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let _pat_str = if let Some(str_ref) = pattern.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gmatch".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    // For now, return an iterator that returns nil (ends immediately)
    // This is a stub that needs proper implementation with closures
    let native = crate::value::NativeFunction::new(|_state| Ok(0));
    let func_ref = state.gc.alloc(crate::value::Function::Native(native));
    state.push(Value::function(func_ref))?;
    Ok(1)
}

/// string.dump(function [, strip]) -> binary string
/// Serializes a function's bytecode to a binary string that can be loaded with loadstring.
fn string_dump(state: &mut State) -> LuaResult<usize> {
    use crate::value::Function;

    let func_val = state.get_value(1);
    let _strip = state.get_value(2).as_boolean().unwrap_or(false);

    let func_ref = func_val.as_function().ok_or_else(|| LuaError::ArgumentError {
        func: "string.dump".to_string(),
        arg: 1,
        msg: "function expected".to_string(),
    })?;

    let func = unsafe { &*func_ref.as_ptr() };

    match func {
        Function::Native(_) => {
            return Err(LuaError::RuntimeError(
                "unable to dump given function".to_string()
            ));
        }
        Function::Lua(closure) => {
            let proto = unsafe { &*closure.proto.as_ptr() };
            let bytecode = dump_proto(proto);
            let result = state.intern_bytes(&bytecode);
            state.push(result)?;
            Ok(1)
        }
    }
}

/// Magic bytes for our bytecode format
pub const BYTECODE_MAGIC: &[u8] = b"\x1bLJR"; // "ESC LJR" - LuaJit Rust

/// Serialize a Proto to binary bytecode
pub fn dump_proto(proto: &crate::value::Proto) -> Vec<u8> {
    let mut buf = Vec::new();

    // Write header
    buf.extend_from_slice(BYTECODE_MAGIC);
    buf.push(1); // version

    // Write the proto recursively
    write_proto(&mut buf, proto);

    buf
}

fn write_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_i32(buf: &mut Vec<u8>, val: i32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_f64(buf: &mut Vec<u8>, val: f64) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_string(buf: &mut Vec<u8>, s: &[u8]) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s);
}

fn write_proto(buf: &mut Vec<u8>, proto: &crate::value::Proto) {
    // Flags
    let mut flags: u8 = 0;
    if proto.is_vararg { flags |= 0x01; }
    buf.push(flags);

    // Basic info
    buf.push(proto.num_params);
    buf.push(proto.max_stack_size);
    buf.push(proto.num_upvalues);

    // Instructions
    write_u32(buf, proto.code.len() as u32);
    for instr in &proto.code {
        write_u32(buf, instr.raw());
    }

    // Constants (Value type)
    write_u32(buf, proto.constants.len() as u32);
    for constant in &proto.constants {
        if constant.is_nil() {
            buf.push(0); // nil
        } else if let Some(b) = constant.as_boolean() {
            buf.push(if b { 2 } else { 1 }); // bool
        } else if let Some(i) = constant.as_integer() {
            buf.push(3); // integer
            write_i32(buf, i);
        } else if let Some(n) = constant.as_number() {
            buf.push(4); // number
            write_f64(buf, n);
        } else {
            // Treat other types as nil for now
            buf.push(0);
        }
    }

    // String constants
    write_u32(buf, proto.string_constants.len() as u32);
    for s in &proto.string_constants {
        write_string(buf, s);
    }

    // Nested protos
    write_u32(buf, proto.protos.len() as u32);
    for child in &proto.protos {
        let child_proto = unsafe { &*child.as_ptr() };
        write_proto(buf, child_proto);
    }

    // Upvalue descriptors
    write_u32(buf, proto.upvalues.len() as u32);
    for uv in &proto.upvalues {
        buf.push(if uv.in_stack { 1 } else { 0 });
        buf.push(uv.index);
    }

    // Line info
    write_u32(buf, proto.line_defined);
    write_u32(buf, proto.last_line_defined);

    // Source name
    if let Some(src) = proto.source {
        let src_str = unsafe { &*src.as_ptr() };
        write_string(buf, src_str.as_bytes());
    } else {
        write_u32(buf, 0); // empty source
    }

    // Lineinfo (for debug)
    write_u32(buf, proto.lineinfo.len() as u32);
    for &line in &proto.lineinfo {
        write_u32(buf, line);
    }
}

/// Load a Proto from bytecode
pub fn load_proto(bytes: &[u8]) -> LuaResult<crate::value::Proto> {
    use crate::value::Proto;
    use crate::bytecode::Instruction;

    let mut cursor = 0;

    // Check magic
    if bytes.len() < BYTECODE_MAGIC.len() + 1 {
        return Err(LuaError::RuntimeError("invalid bytecode".to_string()));
    }
    if &bytes[..BYTECODE_MAGIC.len()] != BYTECODE_MAGIC {
        return Err(LuaError::RuntimeError("invalid bytecode magic".to_string()));
    }
    cursor += BYTECODE_MAGIC.len();

    // Check version
    if bytes[cursor] != 1 {
        return Err(LuaError::RuntimeError("unsupported bytecode version".to_string()));
    }
    cursor += 1;

    read_proto(bytes, &mut cursor)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> LuaResult<u32> {
    if *cursor + 4 > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let val = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(val)
}

fn read_i32(bytes: &[u8], cursor: &mut usize) -> LuaResult<i32> {
    if *cursor + 4 > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let val = i32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(val)
}

fn read_f64(bytes: &[u8], cursor: &mut usize) -> LuaResult<f64> {
    if *cursor + 8 > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let val = f64::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
        bytes[*cursor + 4],
        bytes[*cursor + 5],
        bytes[*cursor + 6],
        bytes[*cursor + 7],
    ]);
    *cursor += 8;
    Ok(val)
}

fn read_bytes(bytes: &[u8], cursor: &mut usize) -> LuaResult<Vec<u8>> {
    let len = read_u32(bytes, cursor)? as usize;
    if *cursor + len > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let data = bytes[*cursor..*cursor + len].to_vec();
    *cursor += len;
    Ok(data)
}

fn read_proto(bytes: &[u8], cursor: &mut usize) -> LuaResult<crate::value::Proto> {
    use crate::value::{Proto, UpvalueDesc, Value};
    use crate::bytecode::Instruction;

    if *cursor >= bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }

    // Flags
    let flags = bytes[*cursor];
    *cursor += 1;
    let is_vararg = flags & 0x01 != 0;

    // Basic info
    if *cursor + 3 > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let num_params = bytes[*cursor];
    let max_stack_size = bytes[*cursor + 1];
    let num_upvalues = bytes[*cursor + 2];
    *cursor += 3;

    // Instructions
    let num_instructions = read_u32(bytes, cursor)? as usize;
    let mut code = Vec::with_capacity(num_instructions);
    for _ in 0..num_instructions {
        let raw = read_u32(bytes, cursor)?;
        code.push(Instruction(raw));
    }

    // Constants
    let num_constants = read_u32(bytes, cursor)? as usize;
    let mut constants = Vec::with_capacity(num_constants);
    for _ in 0..num_constants {
        if *cursor >= bytes.len() {
            return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
        }
        let tag = bytes[*cursor];
        *cursor += 1;
        let val = match tag {
            0 => Value::nil(),
            1 => Value::boolean(false),
            2 => Value::boolean(true),
            3 => Value::integer(read_i32(bytes, cursor)?),
            4 => Value::number(read_f64(bytes, cursor)?),
            _ => Value::nil(),
        };
        constants.push(val);
    }

    // String constants
    let num_string_constants = read_u32(bytes, cursor)? as usize;
    let mut string_constants = Vec::with_capacity(num_string_constants);
    for _ in 0..num_string_constants {
        string_constants.push(read_bytes(bytes, cursor)?);
    }

    // Nested protos (as child_protos for allocation later)
    let num_protos = read_u32(bytes, cursor)? as usize;
    let mut child_protos = Vec::with_capacity(num_protos);
    for _ in 0..num_protos {
        let child = read_proto(bytes, cursor)?;
        child_protos.push(Box::new(child));
    }

    // Upvalue descriptors
    let num_uv_desc = read_u32(bytes, cursor)? as usize;
    let mut upvalues = Vec::with_capacity(num_uv_desc);
    for _ in 0..num_uv_desc {
        if *cursor + 2 > bytes.len() {
            return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
        }
        let in_stack = bytes[*cursor] != 0;
        let index = bytes[*cursor + 1];
        *cursor += 2;
        upvalues.push(UpvalueDesc {
            in_stack,
            index,
            name: None,
        });
    }

    // Line info
    let line_defined = read_u32(bytes, cursor)?;
    let last_line_defined = read_u32(bytes, cursor)?;

    // Source name (we don't intern it here - just store raw bytes)
    let source_bytes = read_bytes(bytes, cursor)?;

    // Lineinfo array
    let num_lineinfo = read_u32(bytes, cursor)? as usize;
    let mut lineinfo = Vec::with_capacity(num_lineinfo);
    for _ in 0..num_lineinfo {
        lineinfo.push(read_u32(bytes, cursor)?);
    }

    // Build the proto - note: source is None, will be set by allocate_proto_tree if needed
    let mut proto = Proto::new();
    proto.code = code;
    proto.constants = constants;
    proto.string_constants = string_constants;
    proto.child_protos = child_protos;
    proto.upvalues = upvalues;
    proto.lineinfo = lineinfo;
    proto.num_params = num_params;
    proto.is_vararg = is_vararg;
    proto.max_stack_size = max_stack_size;
    proto.num_upvalues = num_upvalues;
    proto.line_defined = line_defined;
    proto.last_line_defined = last_line_defined;
    // Store source bytes in string_constants if not empty, for potential re-serialization
    // For now, we leave source as None since we don't have access to the GC here

    Ok(proto)
}
