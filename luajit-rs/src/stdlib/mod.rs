//! Lua standard library implementation.
//!
//! This module provides the standard Lua library functions.

mod base;
mod math;
mod string;
mod table;
mod io;
mod os;

pub use base::register_base;
pub use math::register_math;
pub use string::register_string;
pub use table::register_table;
pub use io::register_io;
pub use os::register_os;

use crate::vm::State;

/// Register all standard libraries
pub fn register_all(state: &mut State) {
    register_base(state);
    register_math(state);
    register_string(state);
    register_table(state);
    register_io(state);
    register_os(state);
}
