//! Table library

use crate::value::{Value, LuaError, LuaResult};
use crate::vm::State;

pub fn register_table(state: &mut State) {
    // Create table table
    let table_lib = state.create_table(0, 8);

    // Helper to add a function to the table
    let add_func = |state: &mut State, tbl: crate::value::GcRef<crate::value::Table>, name: &str, func: crate::value::NativeFn| {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    // Add functions to table (Lua 5.1 compatible)
    add_func(state, table_lib, "concat", table_concat);
    add_func(state, table_lib, "insert", table_insert);
    add_func(state, table_lib, "remove", table_remove);
    add_func(state, table_lib, "sort", table_sort);
    // Lua 5.1 deprecated functions (but still present in LuaJIT)
    add_func(state, table_lib, "foreach", table_foreach);
    add_func(state, table_lib, "foreachi", table_foreachi);
    add_func(state, table_lib, "getn", table_getn);
    add_func(state, table_lib, "maxn", table_maxn);
    // Lua 5.3+ functions (not for 5.1 compatibility, but needed for move)
    add_func(state, table_lib, "move", table_move);

    state.set_global("table", Value::table(table_lib));

    // Also register unpack as a global (Lua 5.1 compatibility)
    let native = crate::value::NativeFunction::new(table_unpack);
    let func_ref = state.gc.alloc(crate::value::Function::Native(native));
    state.set_global("unpack", Value::function(func_ref));
}

fn table_concat(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);
    let sep = if state.get_top() >= 2 {
        let s = state.get_value(2);
        if let Some(str_ref) = s.as_string() {
            let str_val = unsafe { &*str_ref.as_ptr() };
            str_val.as_str().unwrap_or("").to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let i = state.to_integer(3).unwrap_or(1) as usize;
    let j = state.to_integer(4);

    if let Some(t_ref) = t.as_table() {
        let table = unsafe { &*t_ref.as_ptr() };
        let len = table.len();
        let end = j.map(|v| v as usize).unwrap_or(len);

        let mut result = String::new();
        for idx in i..=end {
            if idx > i {
                result.push_str(&sep);
            }
            // Use get() to check both array and hash parts
            let val = table.get(&Value::integer(idx as i32));
            if let Some(str_ref) = val.as_string() {
                let str_val = unsafe { &*str_ref.as_ptr() };
                if let Some(s) = str_val.as_str() {
                    result.push_str(s);
                }
            } else if let Some(n) = val.as_number() {
                // Format number: use integer format if it's a whole number
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    result.push_str(&format!("{}", n as i64));
                } else {
                    result.push_str(&format!("{}", n));
                }
            } else {
                // nil or non-string/number value - error
                return Err(LuaError::RuntimeError(
                    format!("invalid value (nil) at index {} in table for 'concat'", idx)
                ));
            }
        }

        let val = state.intern_string(&result);
        state.push(val)?;
        Ok(1)
    } else {
        Err(LuaError::ArgumentError {
            func: "table.concat".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

fn table_insert(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);

    if let Some(t_ref) = t.as_table() {
        let table = unsafe { &*t_ref.as_ptr() };
        let n = state.get_top();

        if n == 2 {
            // insert at end
            let val = state.get_value(2);
            let len = table.len();
            table.set_array(len + 1, val);
        } else if n >= 3 {
            // insert at position
            let pos = state.to_integer(2).unwrap_or(1) as usize;
            let val = state.get_value(3);

            // Shift elements
            let len = table.len();
            for i in (pos..=len).rev() {
                let v = table.get_array(i);
                table.set_array(i + 1, v);
            }
            table.set_array(pos, val);
        }
        Ok(0)
    } else {
        Err(LuaError::ArgumentError {
            func: "table.insert".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

fn table_remove(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);

    if let Some(t_ref) = t.as_table() {
        let table = unsafe { &*t_ref.as_ptr() };
        let len = table.len();
        let pos = state.to_integer(2).unwrap_or(len as i32) as usize;

        if pos >= 1 && pos <= len {
            let removed = table.get_array(pos);

            // Shift elements
            for i in pos..len {
                let v = table.get_array(i + 1);
                table.set_array(i, v);
            }
            table.set_array(len, Value::nil());

            state.push(removed)?;
            Ok(1)
        } else {
            Ok(0)
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "table.remove".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

fn table_sort(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);
    let comp = if state.get_top() >= 2 {
        let c = state.get_value(2);
        if c.is_function() { Some(c) } else { None }
    } else {
        None
    };

    if let Some(t_ref) = t.as_table() {
        let table = unsafe { &mut *t_ref.as_ptr() };
        let len = table.len();

        if len <= 1 {
            return Ok(0);
        }

        // Extract array elements into a Vec for sorting
        let mut arr: Vec<Value> = Vec::with_capacity(len);
        for i in 1..=len {
            arr.push(table.get_array(i));
        }

        // Implement insertion sort (simpler and works with the callback mechanism)
        for i in 1..arr.len() {
            let mut j = i;
            while j > 0 {
                let should_swap = compare_with_metamethod(state, &arr[j], &arr[j - 1], comp.as_ref())?;

                if should_swap {
                    arr.swap(j, j - 1);
                    j -= 1;
                } else {
                    break;
                }
            }
        }

        // Write sorted elements back
        for (i, val) in arr.into_iter().enumerate() {
            table.set_array(i + 1, val);
        }

        Ok(0)
    } else {
        Err(LuaError::ArgumentError {
            func: "table.sort".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

/// Compare two values for sorting, using metamethods if available
/// Returns true if a < b
fn compare_with_metamethod(state: &mut State, a: &Value, b: &Value, comp: Option<&Value>) -> LuaResult<bool> {
    // If a custom comparator is provided, use it
    if let Some(comp_fn) = comp {
        return call_compare_function(state, comp_fn, a, b);
    }

    // Check for __lt metamethod on either value
    if let Some(lt_mm) = get_lt_metamethod(state, a, b) {
        return call_compare_function(state, &lt_mm, a, b);
    }

    // Fall back to raw comparison
    Ok(compare_values_raw(a, b))
}

/// Get the __lt metamethod from either value's metatable
fn get_lt_metamethod(state: &mut State, a: &Value, b: &Value) -> Option<Value> {
    // Check a's metatable first
    if let Some(t) = a.as_table() {
        let table = unsafe { &*t.as_ptr() };
        if let Some(mt) = table.get_metatable() {
            let mt_table = unsafe { &*mt.as_ptr() };
            let key = state.intern_string("__lt");
            let mm = mt_table.get(&key);
            if mm.is_function() {
                return Some(mm);
            }
        }
    }

    // Check b's metatable
    if let Some(t) = b.as_table() {
        let table = unsafe { &*t.as_ptr() };
        if let Some(mt) = table.get_metatable() {
            let mt_table = unsafe { &*mt.as_ptr() };
            let key = state.intern_string("__lt");
            let mm = mt_table.get(&key);
            if mm.is_function() {
                return Some(mm);
            }
        }
    }

    None
}

/// Call a comparison function and return its boolean result
fn call_compare_function(state: &mut State, func: &Value, a: &Value, b: &Value) -> LuaResult<bool> {
    // Save current stack state
    let saved_top = state.stack.top();

    // Push function and arguments
    state.stack.set(saved_top, func.clone());
    state.stack.set(saved_top + 1, a.clone());
    state.stack.set(saved_top + 2, b.clone());
    state.stack.set_top(saved_top + 3);

    // Call the function
    state.call(2, 1)?;

    // Get the result
    let result = state.get_value(1);
    let is_true = !result.is_falsy();

    // Restore stack
    state.set_top(0);

    Ok(is_true)
}

/// Compare two values for sorting without metamethods (returns true if a < b)
fn compare_values_raw(a: &Value, b: &Value) -> bool {
    // Number comparison
    if let (Some(na), Some(nb)) = (a.as_number(), b.as_number()) {
        return na < nb;
    }

    // String comparison
    if let (Some(sa), Some(sb)) = (a.as_string(), b.as_string()) {
        let sa = unsafe { &*sa.as_ptr() };
        let sb = unsafe { &*sb.as_ptr() };
        return sa.as_bytes() < sb.as_bytes();
    }

    // Mixed types - numbers come before strings
    if a.is_number() && b.is_string() {
        return true;
    }
    if a.is_string() && b.is_number() {
        return false;
    }

    false
}

fn table_unpack(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);
    let i = state.to_integer(2).unwrap_or(1) as usize;
    let j = state.to_integer(3);

    if let Some(t_ref) = t.as_table() {
        let table = unsafe { &*t_ref.as_ptr() };
        let len = table.len();
        let end = j.map(|v| v as usize).unwrap_or(len);

        let mut count = 0;
        for idx in i..=end {
            let val = table.get_array(idx);
            state.push(val)?;
            count += 1;
        }
        Ok(count)
    } else {
        Err(LuaError::ArgumentError {
            func: "table.unpack".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

fn table_pack(state: &mut State) -> LuaResult<usize> {
    let n = state.get_top();
    let t = state.create_table(n, 1);

    unsafe {
        for i in 1..=n as i32 {
            let val = state.get_value(i);
            (*t.as_ptr()).set_array(i as usize, val);
        }
        (*t.as_ptr()).set(state.intern_string("n"), Value::integer(n as i32));
    }

    state.push(Value::table(t))?;
    Ok(1)
}

/// table.foreach(t, f) - call f(k, v) for each element, stop if f returns non-nil
/// Deprecated in Lua 5.1, but still available
fn table_foreach(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);
    let f = state.get_value(2);

    if !t.is_table() {
        return Err(LuaError::ArgumentError {
            func: "foreach".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        });
    }
    if !f.is_function() {
        return Err(LuaError::ArgumentError {
            func: "foreach".to_string(),
            arg: 2,
            msg: "function expected".to_string(),
        });
    }

    let table = unsafe { &*t.as_table().unwrap().as_ptr() };
    let mut key = Value::nil();

    loop {
        match table.next_checked(&key) {
            Ok(Some((k, v))) => {
                // Call f(k, v)
                state.push(f)?;
                state.push(k)?;
                state.push(v)?;
                state.call(2, 1)?;
                let result = state.get_value(1);
                state.set_top(0);

                if !result.is_nil() {
                    state.push(result)?;
                    return Ok(1);
                }
                key = k;
            }
            Ok(None) => break,
            Err(_) => return Err(LuaError::RuntimeError("invalid key in next".to_string())),
        }
    }

    Ok(0)
}

/// table.foreachi(t, f) - call f(i, v) for i=1 to #t, stop if f returns non-nil
/// Deprecated in Lua 5.1, but still available
fn table_foreachi(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);
    let f = state.get_value(2);

    if !t.is_table() {
        return Err(LuaError::ArgumentError {
            func: "foreachi".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        });
    }
    if !f.is_function() {
        return Err(LuaError::ArgumentError {
            func: "foreachi".to_string(),
            arg: 2,
            msg: "function expected".to_string(),
        });
    }

    let table = unsafe { &*t.as_table().unwrap().as_ptr() };
    let len = table.len();

    for i in 1..=len {
        let v = table.get_array(i);
        // Call f(i, v)
        state.push(f)?;
        state.push(Value::integer(i as i32))?;
        state.push(v)?;
        state.call(2, 1)?;
        let result = state.get_value(1);
        state.set_top(0);

        if !result.is_nil() {
            state.push(result)?;
            return Ok(1);
        }
    }

    Ok(0)
}

/// table.getn(t) - return length of table (same as #t)
/// Deprecated in Lua 5.1, but still available
fn table_getn(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);

    if let Some(t_ref) = t.as_table() {
        let table = unsafe { &*t_ref.as_ptr() };
        state.push(Value::integer(table.len() as i32))?;
        Ok(1)
    } else {
        Err(LuaError::ArgumentError {
            func: "getn".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

/// table.maxn(t) - return largest positive integer key in table
/// Deprecated in Lua 5.2, but still available in LuaJIT
fn table_maxn(state: &mut State) -> LuaResult<usize> {
    let t = state.get_value(1);

    if let Some(t_ref) = t.as_table() {
        let table = unsafe { &*t_ref.as_ptr() };
        let mut maxn = 0.0f64;

        let mut key = Value::nil();
        loop {
            match table.next_checked(&key) {
                Ok(Some((k, _))) => {
                    if let Some(n) = k.as_number() {
                        if n > 0.0 && n == (n as i64) as f64 && n > maxn {
                            maxn = n;
                        }
                    }
                    key = k;
                }
                Ok(None) => break,
                Err(_) => return Err(LuaError::RuntimeError("invalid key in next".to_string())),
            }
        }

        state.push(Value::number(maxn))?;
        Ok(1)
    } else {
        Err(LuaError::ArgumentError {
            func: "maxn".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        })
    }
}

/// table.move(a1, f, e, t [, a2]) - move elements from a1 to a2
fn table_move(state: &mut State) -> LuaResult<usize> {
    let a1 = state.get_value(1);
    let f = state.to_integer(2).ok_or_else(|| LuaError::ArgumentError {
        func: "move".to_string(),
        arg: 2,
        msg: "number expected".to_string(),
    })?;
    let e = state.to_integer(3).ok_or_else(|| LuaError::ArgumentError {
        func: "move".to_string(),
        arg: 3,
        msg: "number expected".to_string(),
    })?;
    let t_idx = state.to_integer(4).ok_or_else(|| LuaError::ArgumentError {
        func: "move".to_string(),
        arg: 4,
        msg: "number expected".to_string(),
    })?;
    let a2 = if state.get_top() >= 5 {
        state.get_value(5)
    } else {
        a1
    };

    if !a1.is_table() {
        return Err(LuaError::ArgumentError {
            func: "move".to_string(),
            arg: 1,
            msg: "table expected".to_string(),
        });
    }
    if !a2.is_table() {
        return Err(LuaError::ArgumentError {
            func: "move".to_string(),
            arg: 5,
            msg: "table expected".to_string(),
        });
    }

    let src = unsafe { &*a1.as_table().unwrap().as_ptr() };
    let dst = unsafe { &mut *a2.as_table().unwrap().as_ptr() };

    // Copy elements
    let count = e - f + 1;
    if count > 0 {
        for i in 0..count {
            let val = src.get_array((f + i) as usize);
            dst.set_array((t_idx + i) as usize, val);
        }
    }

    state.push(a2)?;
    Ok(1)
}
