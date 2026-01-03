//! Math library

use crate::value::{Value, LuaError, LuaResult};
use crate::vm::State;
use std::f64::consts::{PI, E};

/// Register math library
pub fn register_math(state: &mut State) {
    // Create math table
    let math = state.create_table(0, 32);

    // Helper to add a function to the math table
    let add_func = |state: &mut State, math: crate::value::GcRef<crate::value::Table>, name: &str, func: crate::value::NativeFn| {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*math.as_ptr()).set(key, Value::function(func_ref)); }
    };

    // Constants
    unsafe {
        (*math.as_ptr()).set(state.intern_string("pi"), Value::number(PI));
        (*math.as_ptr()).set(state.intern_string("huge"), Value::number(f64::INFINITY));
        (*math.as_ptr()).set(state.intern_string("maxinteger"), Value::number(i64::MAX as f64));
        (*math.as_ptr()).set(state.intern_string("mininteger"), Value::number(i64::MIN as f64));
    }

    // Add functions to math table (not as globals)
    add_func(state, math, "abs", math_abs);
    add_func(state, math, "acos", math_acos);
    add_func(state, math, "asin", math_asin);
    add_func(state, math, "atan", math_atan);
    add_func(state, math, "ceil", math_ceil);
    add_func(state, math, "cos", math_cos);
    add_func(state, math, "deg", math_deg);
    add_func(state, math, "exp", math_exp);
    add_func(state, math, "floor", math_floor);
    add_func(state, math, "fmod", math_fmod);
    add_func(state, math, "log", math_log);
    add_func(state, math, "max", math_max);
    add_func(state, math, "min", math_min);
    add_func(state, math, "modf", math_modf);
    add_func(state, math, "rad", math_rad);
    add_func(state, math, "random", math_random);
    add_func(state, math, "randomseed", math_randomseed);
    add_func(state, math, "sin", math_sin);
    add_func(state, math, "sqrt", math_sqrt);
    add_func(state, math, "tan", math_tan);
    add_func(state, math, "tointeger", math_tointeger);
    add_func(state, math, "type", math_type);
    add_func(state, math, "ult", math_ult);

    state.set_global("math", Value::table(math));
}

fn get_num(state: &State, idx: i32) -> LuaResult<f64> {
    state.to_number(idx).ok_or_else(|| LuaError::ArgumentError {
        func: "math".to_string(),
        arg: idx as usize,
        msg: "number expected".to_string(),
    })
}

fn math_abs(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.abs()))?;
    Ok(1)
}

fn math_acos(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.acos()))?;
    Ok(1)
}

fn math_asin(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.asin()))?;
    Ok(1)
}

fn math_atan(state: &mut State) -> LuaResult<usize> {
    let y = get_num(state, 1)?;
    if state.get_top() >= 2 {
        let x = get_num(state, 2)?;
        state.push(Value::number(y.atan2(x)))?;
    } else {
        state.push(Value::number(y.atan()))?;
    }
    Ok(1)
}

fn math_ceil(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.ceil()))?;
    Ok(1)
}

fn math_cos(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.cos()))?;
    Ok(1)
}

fn math_deg(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.to_degrees()))?;
    Ok(1)
}

fn math_exp(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.exp()))?;
    Ok(1)
}

fn math_floor(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.floor()))?;
    Ok(1)
}

fn math_fmod(state: &mut State) -> LuaResult<usize> {
    let x = get_num(state, 1)?;
    let y = get_num(state, 2)?;
    state.push(Value::number(x % y))?;
    Ok(1)
}

fn math_log(state: &mut State) -> LuaResult<usize> {
    let x = get_num(state, 1)?;
    let result = if state.get_top() >= 2 {
        let base = get_num(state, 2)?;
        x.log(base)
    } else {
        x.ln()
    };
    state.push(Value::number(result))?;
    Ok(1)
}

fn math_max(state: &mut State) -> LuaResult<usize> {
    let n = state.get_top();
    if n == 0 {
        return Err(LuaError::ArgumentError {
            func: "math.max".to_string(),
            arg: 1,
            msg: "value expected".to_string(),
        });
    }

    let mut max = get_num(state, 1)?;
    for i in 2..=n as i32 {
        let v = get_num(state, i)?;
        if v > max {
            max = v;
        }
    }
    state.push(Value::number(max))?;
    Ok(1)
}

fn math_min(state: &mut State) -> LuaResult<usize> {
    let n = state.get_top();
    if n == 0 {
        return Err(LuaError::ArgumentError {
            func: "math.min".to_string(),
            arg: 1,
            msg: "value expected".to_string(),
        });
    }

    let mut min = get_num(state, 1)?;
    for i in 2..=n as i32 {
        let v = get_num(state, i)?;
        if v < min {
            min = v;
        }
    }
    state.push(Value::number(min))?;
    Ok(1)
}

fn math_modf(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    let int_part = n.trunc();
    let frac_part = n.fract();
    state.push(Value::number(int_part))?;
    state.push(Value::number(frac_part))?;
    Ok(2)
}

fn math_rad(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.to_radians()))?;
    Ok(1)
}

fn math_random(state: &mut State) -> LuaResult<usize> {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Simple LCG random for now
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;

    let n = state.get_top();
    if n == 0 {
        // Return [0, 1)
        let r = (seed as f64) / (u64::MAX as f64);
        state.push(Value::number(r))?;
    } else if n == 1 {
        // Return [1, m]
        let m = get_num(state, 1)? as u64;
        let r = (seed % m) + 1;
        state.push(Value::number(r as f64))?;
    } else {
        // Return [m, n]
        let m = get_num(state, 1)? as i64;
        let n = get_num(state, 2)? as i64;
        let range = (n - m + 1) as u64;
        let r = m + (seed % range) as i64;
        state.push(Value::number(r as f64))?;
    }
    Ok(1)
}

fn math_randomseed(state: &mut State) -> LuaResult<usize> {
    // Seed is ignored in this simple implementation
    let _seed = get_num(state, 1)?;
    Ok(0)
}

fn math_sin(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.sin()))?;
    Ok(1)
}

fn math_sqrt(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.sqrt()))?;
    Ok(1)
}

fn math_tan(state: &mut State) -> LuaResult<usize> {
    let n = get_num(state, 1)?;
    state.push(Value::number(n.tan()))?;
    Ok(1)
}

fn math_tointeger(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);
    if let Some(i) = val.as_integer() {
        state.push(Value::integer(i))?;
    } else if let Some(n) = val.as_number() {
        let i = n as i64;
        if (i as f64) == n {
            state.push(Value::integer(i as i32))?;
        } else {
            state.push(Value::nil())?;
        }
    } else {
        state.push(Value::nil())?;
    }
    Ok(1)
}

fn math_type(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);
    if val.is_integer() {
        let s = state.intern_string("integer");
        state.push(s)?;
    } else if val.is_number() {
        let s = state.intern_string("float");
        state.push(s)?;
    } else {
        state.push(Value::nil())?;
    }
    Ok(1)
}

fn math_ult(state: &mut State) -> LuaResult<usize> {
    let m = get_num(state, 1)? as u64;
    let n = get_num(state, 2)? as u64;
    state.push(Value::boolean(m < n))?;
    Ok(1)
}
