//! Table library

use crate::value::{Value, LuaError, LuaResult};
use crate::vm::State;

pub fn register_table(state: &mut State) {
    state.register_function("table.concat", table_concat);
    state.register_function("table.insert", table_insert);
    state.register_function("table.remove", table_remove);
    state.register_function("table.sort", table_sort);
    state.register_function("table.unpack", table_unpack);
    state.register_function("table.pack", table_pack);
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
    // Simplified sort - not implemented
    Ok(0)
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
