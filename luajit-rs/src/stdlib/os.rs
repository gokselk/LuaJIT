//! OS library (stub implementation)

use crate::value::{Value, LuaResult};
use crate::vm::State;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn register_os(state: &mut State) {
    // Create os table
    let os_table = state.create_table(0, 8);

    // Helper to add a function to the table
    let add_func = |state: &mut State, tbl: crate::value::GcRef<crate::value::Table>, name: &str, func: crate::value::NativeFn| {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    // Add functions to os table
    add_func(state, os_table, "clock", os_clock);
    add_func(state, os_table, "date", os_date);
    add_func(state, os_table, "difftime", os_difftime);
    add_func(state, os_table, "exit", os_exit);
    add_func(state, os_table, "getenv", os_getenv);
    add_func(state, os_table, "time", os_time);

    state.set_global("os", Value::table(os_table));
}

fn os_clock(state: &mut State) -> LuaResult<usize> {
    // Return CPU time (simplified - just return elapsed time)
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    state.push(Value::number(elapsed % 1000000.0))?;
    Ok(1)
}

fn os_date(state: &mut State) -> LuaResult<usize> {
    // Simplified date - just return a basic timestamp string
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let s = format!("{}", now);
    let val = state.intern_string(&s);
    state.push(val)?;
    Ok(1)
}

fn os_difftime(state: &mut State) -> LuaResult<usize> {
    let t2 = state.to_number(1).unwrap_or(0.0);
    let t1 = state.to_number(2).unwrap_or(0.0);
    state.push(Value::number(t2 - t1))?;
    Ok(1)
}

fn os_exit(state: &mut State) -> LuaResult<usize> {
    let code = state.to_integer(1).unwrap_or(0);
    std::process::exit(code);
}

fn os_getenv(state: &mut State) -> LuaResult<usize> {
    let name = state.get_value(1);
    if let Some(str_ref) = name.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        if let Some(name_str) = str_val.as_str() {
            if let Ok(env_val) = std::env::var(name_str) {
                let val = state.intern_string(&env_val);
                state.push(val)?;
                return Ok(1);
            }
        }
    }
    state.push(Value::nil())?;
    Ok(1)
}

fn os_time(state: &mut State) -> LuaResult<usize> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    state.push(Value::number(now as f64))?;
    Ok(1)
}
