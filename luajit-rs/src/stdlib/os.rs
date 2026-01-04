//! OS library implementation

use crate::value::{Value, LuaResult, LuaError};
use crate::vm::State;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn register_os(state: &mut State) {
    // Create os table
    let os_table = state.create_table(0, 16);

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
    add_func(state, os_table, "execute", os_execute);
    add_func(state, os_table, "exit", os_exit);
    add_func(state, os_table, "getenv", os_getenv);
    add_func(state, os_table, "remove", os_remove);
    add_func(state, os_table, "rename", os_rename);
    add_func(state, os_table, "setlocale", os_setlocale);
    add_func(state, os_table, "time", os_time);
    add_func(state, os_table, "tmpname", os_tmpname);

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

fn os_execute(state: &mut State) -> LuaResult<usize> {
    let cmd = state.get_value(1);

    if cmd.is_nil() {
        // os.execute() with no arguments returns true if shell is available
        state.push(Value::boolean(true))?;
        return Ok(1);
    }

    if let Some(str_ref) = cmd.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        if let Some(cmd_str) = str_val.as_str() {
            #[cfg(unix)]
            {
                use std::process::Command;
                match Command::new("sh").arg("-c").arg(cmd_str).status() {
                    Ok(status) => {
                        let code = status.code().unwrap_or(-1);
                        if code == 0 {
                            state.push(Value::boolean(true))?;
                        } else {
                            state.push(Value::nil())?;
                        }
                        let exit_str = state.intern_string("exit");
                        state.push(exit_str)?;
                        state.push(Value::number(code as f64))?;
                        return Ok(3);
                    }
                    Err(_) => {
                        state.push(Value::nil())?;
                        return Ok(1);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                state.push(Value::nil())?;
                return Ok(1);
            }
        }
    }

    state.push(Value::nil())?;
    Ok(1)
}

fn os_remove(state: &mut State) -> LuaResult<usize> {
    let path = state.get_value(1);

    if let Some(str_ref) = path.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        if let Some(path_str) = str_val.as_str() {
            match std::fs::remove_file(path_str) {
                Ok(()) => {
                    state.push(Value::boolean(true))?;
                    return Ok(1);
                }
                Err(e) => {
                    state.push(Value::nil())?;
                    let msg = state.intern_string(&e.to_string());
                    state.push(msg)?;
                    return Ok(2);
                }
            }
        }
    }

    Err(LuaError::ArgumentError {
        func: "remove".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

fn os_rename(state: &mut State) -> LuaResult<usize> {
    let old_name = state.get_value(1);
    let new_name = state.get_value(2);

    let old_path = if let Some(str_ref) = old_name.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        str_val.as_str().unwrap_or("").to_string()
    } else {
        return Err(LuaError::ArgumentError {
            func: "rename".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let new_path = if let Some(str_ref) = new_name.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        str_val.as_str().unwrap_or("").to_string()
    } else {
        return Err(LuaError::ArgumentError {
            func: "rename".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    match std::fs::rename(&old_path, &new_path) {
        Ok(()) => {
            state.push(Value::boolean(true))?;
            Ok(1)
        }
        Err(e) => {
            state.push(Value::nil())?;
            let msg = state.intern_string(&e.to_string());
            state.push(msg)?;
            Ok(2)
        }
    }
}

fn os_setlocale(state: &mut State) -> LuaResult<usize> {
    // Locale handling is complex - just return the current locale or nil
    let locale = state.get_value(1);

    if locale.is_nil() || locale.as_string().is_some() {
        // Return C locale as default
        let c_locale = state.intern_string("C");
        state.push(c_locale)?;
        Ok(1)
    } else {
        state.push(Value::nil())?;
        Ok(1)
    }
}

fn os_tmpname(state: &mut State) -> LuaResult<usize> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let tmp_dir = std::env::temp_dir();
    let filename = format!("lua_{}", timestamp);
    let path = tmp_dir.join(filename);

    let path_str = path.to_string_lossy().to_string();
    let val = state.intern_string(&path_str);
    state.push(val)?;
    Ok(1)
}
