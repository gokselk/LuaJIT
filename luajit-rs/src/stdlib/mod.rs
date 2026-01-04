//! Lua standard library implementation.
//!
//! This module provides the standard Lua library functions.

mod base;
mod math;
pub mod string;
mod table;
mod io;
mod os;
pub mod debug;
mod package;
mod coroutine;

pub use base::register_base;
pub use math::register_math;
pub use string::register_string;
pub use string::{BYTECODE_MAGIC, load_proto, dump_proto};
pub use table::register_table;
pub use io::register_io;
pub use os::register_os;
pub use debug::register_debug;
pub use package::register_package;
pub use coroutine::register_coroutine;

use crate::vm::State;

/// Register all standard libraries
pub fn register_all(state: &mut State) {
    // Disable GC during library setup to prevent tables from being collected
    // before they are rooted in globals/registry
    let gc_was_enabled = state.gc.is_running();
    state.gc.stop();

    register_base(state);
    register_math(state);
    register_string(state);
    register_table(state);
    register_io(state);
    register_os(state);
    register_debug(state);
    register_package(state);
    register_coroutine(state);

    // Populate package.loaded with built-in modules
    populate_package_loaded(state);

    // Re-enable GC if it was enabled before
    if gc_was_enabled {
        state.gc.restart();
    }
}

/// Populate package.loaded with references to standard library modules
fn populate_package_loaded(state: &mut State) {
    use crate::value::Value;

    // Get package.loaded from registry (_LOADED)
    let loaded_key = state.intern_string("_LOADED");
    let loaded = unsafe {
        (*state.registry.as_ptr()).get(&loaded_key)
    };

    if let Some(loaded_ref) = loaded.as_table() {
        let loaded_tbl = unsafe { &mut *loaded_ref.as_ptr() };

        // Add _G (globals table)
        let g_key = state.intern_string("_G");
        loaded_tbl.set(g_key, Value::table(state.globals));

        // Add standard library modules
        let modules = ["coroutine", "debug", "io", "math", "os", "package", "string", "table"];

        for name in modules {
            let key = state.intern_string(name);
            let module = state.get_global(name);
            if !module.is_nil() {
                loaded_tbl.set(key, module);
            }
        }
    }
}
