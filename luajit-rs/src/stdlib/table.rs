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

    // Add functions to table
    add_func(state, table_lib, "concat", table_concat);
    add_func(state, table_lib, "insert", table_insert);
    add_func(state, table_lib, "remove", table_remove);
    add_func(state, table_lib, "sort", table_sort);
    add_func(state, table_lib, "unpack", table_unpack);
    add_func(state, table_lib, "pack", table_pack);

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
            let val = table.get_array(idx);
            if let Some(str_ref) = val.as_string() {
                let str_val = unsafe { &*str_ref.as_ptr() };
                if let Some(s) = str_val.as_str() {
                    result.push_str(s);
                }
            } else if let Some(n) = val.as_number() {
                result.push_str(&format!("{}", n));
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
                let should_swap = if let Some(ref comp_fn) = comp {
                    // Call comparator: comp(arr[j], arr[j-1])
                    // If true, arr[j] should come before arr[j-1]
                    let base = state.stack.top();
                    state.stack.set(base, comp_fn.clone());
                    state.stack.set(base + 1, arr[j].clone());
                    state.stack.set(base + 2, arr[j - 1].clone());
                    state.stack.set_top(base + 3);

                    // We need to call the function - but we can't easily call from here
                    // For now, fall back to default comparison when comp is provided
                    // This is a limitation we'll need to fix properly later
                    compare_values(&arr[j], &arr[j - 1])
                } else {
                    compare_values(&arr[j], &arr[j - 1])
                };

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

/// Compare two values for sorting (returns true if a < b)
fn compare_values(a: &Value, b: &Value) -> bool {
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
