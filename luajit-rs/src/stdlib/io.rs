//! IO library (stub implementation)

use crate::value::{Value, LuaResult};
use crate::vm::State;
use std::io::{self, Write, BufRead};

pub fn register_io(state: &mut State) {
    state.register_function("io.write", io_write);
    state.register_function("io.read", io_read);
    state.register_function("io.flush", io_flush);
}

fn io_write(state: &mut State) -> LuaResult<usize> {
    let n = state.get_top();
    let mut stdout = io::stdout();

    for i in 1..=n as i32 {
        let val = state.get_value(i);
        if let Some(str_ref) = val.as_string() {
            let str_val = unsafe { &*str_ref.as_ptr() };
            if let Some(s) = str_val.as_str() {
                stdout.write_all(s.as_bytes()).ok();
            }
        } else if let Some(n) = val.as_number() {
            write!(stdout, "{}", n).ok();
        }
    }

    // Return file handle (stub - just return true)
    state.push(Value::boolean(true))?;
    Ok(1)
}

fn io_read(state: &mut State) -> LuaResult<usize> {
    let fmt = state.get_value(1);

    let result = if let Some(str_ref) = fmt.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        match str_val.as_str() {
            Some("*l") | Some("l") | Some("*L") | Some("L") => {
                // Read line
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line).ok();
                if line.ends_with('\n') {
                    line.pop();
                }
                state.intern_string(&line)
            }
            Some("*a") | Some("a") => {
                // Read all
                let mut content = String::new();
                io::stdin().lock().read_to_string(&mut content).ok();
                state.intern_string(&content)
            }
            Some("*n") | Some("n") => {
                // Read number
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line).ok();
                if let Ok(n) = line.trim().parse::<f64>() {
                    Value::number(n)
                } else {
                    Value::nil()
                }
            }
            _ => Value::nil(),
        }
    } else if let Some(n) = fmt.as_number() {
        // Read n bytes
        let n = n as usize;
        let mut buf = vec![0u8; n];
        let read = io::stdin().lock().read(&mut buf).unwrap_or(0);
        buf.truncate(read);
        state.intern_string(&String::from_utf8_lossy(&buf))
    } else {
        // Default: read line
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line).ok();
        if line.ends_with('\n') {
            line.pop();
        }
        state.intern_string(&line)
    };

    state.push(result)?;
    Ok(1)
}

fn io_flush(state: &mut State) -> LuaResult<usize> {
    io::stdout().flush().ok();
    state.push(Value::boolean(true))?;
    Ok(1)
}

use std::io::Read;
