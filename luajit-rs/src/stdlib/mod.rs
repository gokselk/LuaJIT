//! Lua standard library implementation.
//!
//! This module provides the standard Lua library functions.

mod base;
mod math;
pub mod string;
mod table;
mod io;
mod os;
mod debug;
mod package;

pub use base::register_base;
pub use math::register_math;
pub use string::register_string;
pub use string::{BYTECODE_MAGIC, load_proto, dump_proto};
pub use table::register_table;
pub use io::register_io;
pub use os::register_os;
pub use debug::register_debug;
pub use package::register_package;

use crate::vm::State;

/// Register all standard libraries
pub fn register_all(state: &mut State) {
    register_base(state);
    register_math(state);
    register_string(state);
    register_table(state);
    register_io(state);
    register_os(state);
    register_debug(state);
    register_package(state);
}
