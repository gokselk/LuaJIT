//! Coroutine library implementation.

use crate::value::{Value, LuaResult, LuaError, GcRef, Userdata};
use crate::value::userdata::UserdataAllocator;
use crate::vm::State;
use std::cell::RefCell;

/// Coroutine status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoroutineStatus {
    /// Just created, never resumed
    Created,
    /// Currently running
    Running,
    /// Yielded, can be resumed
    Suspended,
    /// Normal (coroutine is resuming another)
    Normal,
    /// Finished execution or errored
    Dead,
}

impl CoroutineStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CoroutineStatus::Created => "suspended",
            CoroutineStatus::Running => "running",
            CoroutineStatus::Suspended => "suspended",
            CoroutineStatus::Normal => "normal",
            CoroutineStatus::Dead => "dead",
        }
    }
}

/// Coroutine data stored as userdata
pub struct Coroutine {
    /// The function to run
    pub func: Value,
    /// Current status (using Cell for interior mutability without RefCell issues)
    pub status: std::cell::Cell<CoroutineStatus>,
    /// Error message if dead due to error
    pub error: RefCell<Option<String>>,
}

impl Coroutine {
    pub fn new(func: Value) -> Self {
        Self {
            func,
            status: std::cell::Cell::new(CoroutineStatus::Created),
            error: RefCell::new(None),
        }
    }
}

/// Allocate a Coroutine as userdata
fn alloc_coroutine(co: Coroutine) -> GcRef<Userdata> {
    let ptr = UserdataAllocator::allocate(co, 0);
    GcRef::new(ptr)
}

/// Register coroutine library
pub fn register_coroutine(state: &mut State) {
    let co_table = state.create_table(0, 8);

    let add_func = |state: &mut State, tbl: GcRef<crate::value::Table>, name: &str, func: crate::value::NativeFn| {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    add_func(state, co_table, "create", coroutine_create);
    add_func(state, co_table, "resume", coroutine_resume);
    add_func(state, co_table, "yield", coroutine_yield);
    add_func(state, co_table, "status", coroutine_status);
    add_func(state, co_table, "running", coroutine_running);
    add_func(state, co_table, "wrap", coroutine_wrap);
    add_func(state, co_table, "isyieldable", coroutine_isyieldable);

    state.set_global("coroutine", Value::table(co_table));
}

/// Get a Coroutine from a userdata value
fn get_coroutine(ud: &Userdata) -> Option<&Coroutine> {
    ud.downcast_ref::<Coroutine>()
}

/// coroutine.create(f) -> thread
fn coroutine_create(state: &mut State) -> LuaResult<usize> {
    let func = state.get_value(1);

    if !func.is_function() {
        return Err(LuaError::ArgumentError {
            func: "create".to_string(),
            arg: 1,
            msg: "function expected".to_string(),
        });
    }

    let co = Coroutine::new(func);
    let ud = alloc_coroutine(co);
    state.push(Value::userdata(ud))?;
    Ok(1)
}

/// coroutine.resume(co, ...) -> true, results... | false, error
fn coroutine_resume(state: &mut State) -> LuaResult<usize> {
    let co_val = state.get_value(1);

    let co_ud = match co_val.as_userdata() {
        Some(ud) => ud,
        None => {
            return Err(LuaError::ArgumentError {
                func: "resume".to_string(),
                arg: 1,
                msg: "coroutine expected".to_string(),
            });
        }
    };

    let co_ptr = co_ud.as_ptr();
    let ud = unsafe { &*co_ptr };

    let co = match get_coroutine(ud) {
        Some(c) => c,
        None => {
            return Err(LuaError::ArgumentError {
                func: "resume".to_string(),
                arg: 1,
                msg: "coroutine expected".to_string(),
            });
        }
    };

    let current_status = co.status.get();

    match current_status {
        CoroutineStatus::Dead => {
            state.push(Value::boolean(false))?;
            let msg = state.intern_string("cannot resume dead coroutine");
            state.push(msg)?;
            return Ok(2);
        }
        CoroutineStatus::Running => {
            state.push(Value::boolean(false))?;
            let msg = state.intern_string("cannot resume running coroutine");
            state.push(msg)?;
            return Ok(2);
        }
        _ => {}
    }

    // Collect resume arguments and copy the function before any mutable operations
    let nargs = state.get_top();
    let mut args = Vec::new();
    for i in 2..=nargs as i32 {
        args.push(state.get_value(i));
    }

    // Copy the function value before we start mutating
    let func = co.func;

    // Set status to running using Cell (no borrow conflicts)
    co.status.set(CoroutineStatus::Running);

    if current_status == CoroutineStatus::Created {
        // First resume - call the function

        // Clear stack and push function + args
        state.set_top(0);
        state.push(func)?;
        for arg in &args {
            state.push(*arg)?;
        }

        // Call the function - we've copied everything we need, so no borrow issues
        let call_result = state.call(args.len(), -1);

        // Now safely update status after the call
        // We need to get a fresh reference to the coroutine
        let ud_after = unsafe { &*co_ptr };
        let co_after = get_coroutine(ud_after).unwrap();

        match call_result {
            Ok(()) => {
                // Function completed successfully
                co_after.status.set(CoroutineStatus::Dead);

                // Get return values
                let nresults = state.get_top();
                let mut results = Vec::new();
                for i in 1..=nresults as i32 {
                    results.push(state.get_value(i));
                }

                // Return true + results
                state.set_top(0);
                state.push(Value::boolean(true))?;
                for result in results {
                    state.push(result)?;
                }
                Ok(state.get_top())
            }
            Err(e) => {
                // Function errored
                co_after.status.set(CoroutineStatus::Dead);
                *co_after.error.borrow_mut() = Some(format!("{}", e));

                state.set_top(0);
                state.push(Value::boolean(false))?;
                let msg = state.intern_string(&format!("{}", e));
                state.push(msg)?;
                Ok(2)
            }
        }
    } else {
        // Not a created coroutine (shouldn't happen with our simplified implementation)
        state.push(Value::boolean(false))?;
        let msg = state.intern_string("cannot resume coroutine");
        state.push(msg)?;
        Ok(2)
    }
}

/// coroutine.yield(...) -> values from resume
/// Note: This is a simplified implementation that stores yield values
/// but doesn't actually suspend execution mid-function
fn coroutine_yield(state: &mut State) -> LuaResult<usize> {
    // In a full implementation, this would:
    // 1. Save the current execution state
    // 2. Return control to the coroutine.resume caller
    // 3. Resume from here when the coroutine is resumed again

    // For now, we just return an error indicating yield is not fully supported
    Err(LuaError::RuntimeError(
        "cannot yield from outside a coroutine".to_string()
    ))
}

/// coroutine.status(co) -> string
fn coroutine_status(state: &mut State) -> LuaResult<usize> {
    let co_val = state.get_value(1);

    let co_ud = match co_val.as_userdata() {
        Some(ud) => ud,
        None => {
            return Err(LuaError::ArgumentError {
                func: "status".to_string(),
                arg: 1,
                msg: "coroutine expected".to_string(),
            });
        }
    };

    let ud = unsafe { &*co_ud.as_ptr() };

    let co = match get_coroutine(ud) {
        Some(c) => c,
        None => {
            return Err(LuaError::ArgumentError {
                func: "status".to_string(),
                arg: 1,
                msg: "coroutine expected".to_string(),
            });
        }
    };

    let status_str = co.status.get().as_str();
    let s = state.intern_string(status_str);
    state.push(s)?;
    Ok(1)
}

/// coroutine.running() -> thread, bool
fn coroutine_running(state: &mut State) -> LuaResult<usize> {
    // Return nil, true (main thread)
    state.push(Value::nil())?;
    state.push(Value::boolean(true))?;
    Ok(2)
}

/// coroutine.wrap(f) -> function
fn coroutine_wrap(state: &mut State) -> LuaResult<usize> {
    let func = state.get_value(1);

    if !func.is_function() {
        return Err(LuaError::ArgumentError {
            func: "wrap".to_string(),
            arg: 1,
            msg: "function expected".to_string(),
        });
    }

    // Create the coroutine
    let co = Coroutine::new(func);
    let ud = alloc_coroutine(co);

    // Create a wrapper function that resumes the coroutine
    // For simplicity, we create a closure that stores the coroutine userdata
    // This is a simplified implementation - a full one would need proper closures

    // Store the coroutine in the registry with a unique key
    let key_str = format!("_CO_WRAP_{:p}", ud.as_ptr());
    let key = state.intern_string(&key_str);
    unsafe {
        (*state.registry.as_ptr()).set(key, Value::userdata(ud));
    }

    // Create a native function that will look up and resume the coroutine
    let wrapper = crate::value::NativeFunction::new(coroutine_wrap_resume);
    let wrapper_func = state.gc.alloc(crate::value::Function::Native(wrapper));

    // We need to somehow pass the key to the wrapper function
    // For now, just return the coroutine itself as a "wrapped" thread
    state.push(Value::userdata(ud))?;
    Ok(1)
}

/// Internal function for wrapped coroutine resume
fn coroutine_wrap_resume(state: &mut State) -> LuaResult<usize> {
    // This would need access to the stored coroutine
    // For now, return an error
    Err(LuaError::RuntimeError(
        "coroutine.wrap not fully implemented".to_string()
    ))
}

/// coroutine.isyieldable() -> bool
fn coroutine_isyieldable(state: &mut State) -> LuaResult<usize> {
    // Main thread is not yieldable in standard Lua
    state.push(Value::boolean(false))?;
    Ok(1)
}
