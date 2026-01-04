//! Package library - module loading and require implementation.

use crate::value::{Value, LuaError, LuaResult, Table, GcRef, Function, NativeFunction};
use crate::vm::State;
use std::path::Path;

pub fn register_package(state: &mut State) {
    // Create package table
    let package = state.create_table(0, 8);

    // Create package.loaded table
    let loaded = state.create_table(0, 16);

    // Create package.preload table
    let preload = state.create_table(0, 8);

    // Add built-in modules to preload
    register_preload_modules(state, preload);

    // Register bit library as a global (LuaJIT compatibility)
    register_bit_library(state);

    // Register jit library as a global (LuaJIT compatibility)
    register_jit_library(state);

    // Set default path
    let default_path = "./?.lua;./?/init.lua;/usr/local/share/lua/5.1/?.lua";
    let path_val = state.intern_string(default_path);

    // Create package.loaders table (Lua 5.1 compatible)
    let loaders = state.create_table(4, 0);

    // Add native function for loadlib
    let native_loadlib = NativeFunction::new(package_loadlib);
    let loadlib_func = state.gc.alloc(Function::Native(native_loadlib));

    // Add native function for seeall
    let native_seeall = NativeFunction::new(package_seeall);
    let seeall_func = state.gc.alloc(Function::Native(native_seeall));

    // Store in package table
    let loaded_key = state.intern_string("loaded");
    let preload_key = state.intern_string("preload");
    let path_key = state.intern_string("path");
    let cpath_key = state.intern_string("cpath");
    let loaders_key = state.intern_string("loaders");
    let loadlib_key = state.intern_string("loadlib");
    let seeall_key = state.intern_string("seeall");
    let config_key = state.intern_string("config");

    // Package config string (directory separator, path separator, etc.)
    let config_val = state.intern_string("/\n;\n?\n!\n-");

    unsafe {
        let pkg = &mut *package.as_ptr();
        pkg.set(loaded_key, Value::table(loaded));
        pkg.set(preload_key, Value::table(preload));
        pkg.set(path_key, path_val);
        pkg.set(cpath_key, state.intern_string("")); // No C path support yet
        pkg.set(loaders_key, Value::table(loaders));
        pkg.set(loadlib_key, Value::function(loadlib_func));
        pkg.set(seeall_key, Value::function(seeall_func));
        pkg.set(config_key, config_val);

        // Add loaders (1-indexed as table array)
        let loaders_tbl = &mut *loaders.as_ptr();
        // Loader 1: preload
        let preload_loader = NativeFunction::new(loader_preload);
        let loader1 = state.gc.alloc(Function::Native(preload_loader));
        loaders_tbl.set_array(1, Value::function(loader1));
        // Loader 2: Lua loader (file-based)
        let lua_loader = NativeFunction::new(loader_lua);
        let loader2 = state.gc.alloc(Function::Native(lua_loader));
        loaders_tbl.set_array(2, Value::function(loader2));
        // Loader 3: C loader (stub)
        let c_loader = NativeFunction::new(loader_c);
        let loader3 = state.gc.alloc(Function::Native(c_loader));
        loaders_tbl.set_array(3, Value::function(loader3));
        // Loader 4: all-in-one C loader (stub)
        let all_loader = NativeFunction::new(loader_croot);
        let loader4 = state.gc.alloc(Function::Native(all_loader));
        loaders_tbl.set_array(4, Value::function(loader4));
    }

    // Store package.loaded reference in registry for quick access
    let loaded_reg_key = state.intern_string("_LOADED");
    unsafe {
        let reg = &mut *state.registry.as_ptr();
        reg.set(loaded_reg_key, Value::table(loaded));
    }

    // Store package.preload reference in registry
    let preload_reg_key = state.intern_string("_PRELOAD");
    unsafe {
        let reg = &mut *state.registry.as_ptr();
        reg.set(preload_reg_key, Value::table(preload));
    }

    // Set package global
    state.set_global("package", Value::table(package));

    // Register require as global
    state.register_function("require", lua_require);
}

/// Register built-in modules in package.preload
fn register_preload_modules(state: &mut State, preload: GcRef<Table>) {
    // table.new - LuaJIT extension for pre-allocated tables
    add_preload(state, preload, "table.new", table_new_loader);

    // table.clear - LuaJIT extension
    add_preload(state, preload, "table.clear", table_clear_loader);

    // bit - bit operations (LuaJIT)
    add_preload(state, preload, "bit", bit_loader);

    // jit - JIT control (stub)
    add_preload(state, preload, "jit", jit_loader);
}

fn add_preload(state: &mut State, preload: GcRef<Table>, name: &str, loader: crate::value::NativeFn) {
    let native = NativeFunction::new(loader);
    let func_ref = state.gc.alloc(Function::Native(native));
    let key = state.intern_string(name);
    unsafe { (*preload.as_ptr()).set(key, Value::function(func_ref)); }
}

/// require(modname) -> module
pub fn lua_require(state: &mut State) -> LuaResult<usize> {
    let modname_val = state.get_value(1);
    let modname = modname_val.as_string()
        .ok_or_else(|| LuaError::ArgumentError {
            func: "require".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        })?;
    let modname_str = unsafe { (*modname.as_ptr()).as_str() }
        .ok_or_else(|| LuaError::RuntimeError("invalid module name".to_string()))?
        .to_string();

    // Check package.loaded first
    let loaded_key = state.intern_string("_LOADED");
    let loaded = unsafe {
        let reg = &*state.registry.as_ptr();
        reg.get(&loaded_key)
    };

    if let Some(loaded_table) = loaded.as_table() {
        let cached = unsafe { (*loaded_table.as_ptr()).get(&modname_val) };
        if !cached.is_nil() {
            state.push(cached)?;
            return Ok(1);
        }
    }

    // Check package.preload
    let preload_key = state.intern_string("_PRELOAD");
    let preload = unsafe {
        let reg = &*state.registry.as_ptr();
        reg.get(&preload_key)
    };

    if let Some(preload_table) = preload.as_table() {
        let loader = unsafe { (*preload_table.as_ptr()).get(&modname_val) };
        if let Some(func) = loader.as_function() {
            // Call the loader
            let call_base = state.stack.top();
            state.push(Value::function(func))?;
            state.push(modname_val)?;
            state.call(1, 1)?;

            let result = state.stack.get(call_base);

            // Cache in package.loaded
            if let Some(loaded_table) = loaded.as_table() {
                let to_cache = if result.is_nil() { Value::boolean(true) } else { result };
                unsafe { (*loaded_table.as_ptr()).set(modname_val, to_cache); }
            }

            state.stack.set_top(call_base + 1);
            state.stack.set(call_base, result);
            return Ok(1);
        }
    }

    // Try to load from file using package.path
    let path = get_package_path(state);

    for template in path.split(';') {
        let filepath = template.replace("?", &modname_str.replace(".", "/"));

        if Path::new(&filepath).exists() {
            // Read and load the file
            let source = match std::fs::read_to_string(&filepath) {
                Ok(s) => s,
                Err(e) => {
                    return Err(LuaError::RuntimeError(
                        format!("cannot read '{}': {}", filepath, e)
                    ));
                }
            };

            match state.load_string(&source, &filepath) {
                Ok(func) => {
                    // Execute the chunk
                    let call_base = state.stack.top();
                    state.push(Value::function(func))?;
                    state.push(modname_val)?; // Pass module name as argument
                    state.call(1, 1)?;

                    let result = state.stack.get(call_base);

                    // Cache result (use true if nil returned)
                    if let Some(loaded_table) = loaded.as_table() {
                        let to_cache = if result.is_nil() { Value::boolean(true) } else { result };
                        unsafe { (*loaded_table.as_ptr()).set(modname_val, to_cache); }
                    }

                    state.stack.set_top(call_base + 1);
                    state.stack.set(call_base, result);
                    return Ok(1);
                }
                Err(e) => {
                    return Err(LuaError::RuntimeError(
                        format!("error loading module '{}' from file '{}':\n\t{}", modname_str, filepath, e)
                    ));
                }
            }
        }
    }

    Err(LuaError::RuntimeError(
        format!("module '{}' not found:\n\tno field package.preload['{}']\n\tno file in package.path",
                modname_str, modname_str)
    ))
}

fn get_package_path(state: &mut State) -> String {
    let package_key = state.intern_string("package");
    let package = unsafe {
        let globals = &*state.globals.as_ptr();
        globals.get(&package_key)
    };

    if let Some(pkg_table) = package.as_table() {
        let path_key = state.intern_string("path");
        let path = unsafe { (*pkg_table.as_ptr()).get(&path_key) };
        if let Some(s) = path.as_string() {
            if let Some(path_str) = unsafe { (*s.as_ptr()).as_str() } {
                return path_str.to_string();
            }
        }
    }

    "./?.lua".to_string()
}

// ============ Preload module loaders ============

/// table.new(narray, nhash) -> table
fn table_new_loader(state: &mut State) -> LuaResult<usize> {
    // Return a function that creates pre-sized tables
    state.register_function("__table_new_impl", table_new_impl);
    let key = state.intern_string("__table_new_impl");
    let func = unsafe {
        let globals = &*state.globals.as_ptr();
        globals.get(&key)
    };
    state.push(func)?;
    Ok(1)
}

fn table_new_impl(state: &mut State) -> LuaResult<usize> {
    let narray = state.get_value(1).as_number().unwrap_or(0.0) as usize;
    let nhash = state.get_value(2).as_number().unwrap_or(0.0) as usize;
    let table = state.create_table(narray, nhash);
    state.push(Value::table(table))?;
    Ok(1)
}

/// table.clear(t) -> clears a table
fn table_clear_loader(state: &mut State) -> LuaResult<usize> {
    state.register_function("__table_clear_impl", table_clear_impl);
    let key = state.intern_string("__table_clear_impl");
    let func = unsafe {
        let globals = &*state.globals.as_ptr();
        globals.get(&key)
    };
    state.push(func)?;
    Ok(1)
}

fn table_clear_impl(state: &mut State) -> LuaResult<usize> {
    let table = state.get_value(1);
    if let Some(t) = table.as_table() {
        unsafe { (*t.as_ptr()).clear(); }
    }
    Ok(0)
}

/// Register the bit library as a global
fn register_bit_library(state: &mut State) {
    let bit = state.create_table(0, 16);

    let add_func = |state: &mut State, tbl: GcRef<Table>, name: &str, func: crate::value::NativeFn| {
        let native = NativeFunction::new(func);
        let func_ref = state.gc.alloc(Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    add_func(state, bit, "tobit", bit_tobit);
    add_func(state, bit, "bnot", bit_bnot);
    add_func(state, bit, "band", bit_band);
    add_func(state, bit, "bor", bit_bor);
    add_func(state, bit, "bxor", bit_bxor);
    add_func(state, bit, "lshift", bit_lshift);
    add_func(state, bit, "rshift", bit_rshift);
    add_func(state, bit, "arshift", bit_arshift);
    add_func(state, bit, "rol", bit_rol);
    add_func(state, bit, "ror", bit_ror);
    add_func(state, bit, "bswap", bit_bswap);
    add_func(state, bit, "tohex", bit_tohex);

    state.set_global("bit", Value::table(bit));
}

/// bit library loader (for require compatibility)
fn bit_loader(state: &mut State) -> LuaResult<usize> {
    // Return the global bit table
    let key = state.intern_string("bit");
    let bit = unsafe {
        let globals = &*state.globals.as_ptr();
        globals.get(&key)
    };
    state.push(bit)?;
    Ok(1)
}

fn to_bit(v: f64) -> i32 {
    (v as i64 as i32)
}

fn bit_tobit(state: &mut State) -> LuaResult<usize> {
    let n = state.get_value(1).as_number().unwrap_or(0.0);
    state.push(Value::integer(to_bit(n)))?;
    Ok(1)
}

fn bit_bnot(state: &mut State) -> LuaResult<usize> {
    let n = state.get_value(1).as_number().unwrap_or(0.0);
    state.push(Value::integer(!to_bit(n)))?;
    Ok(1)
}

fn bit_band(state: &mut State) -> LuaResult<usize> {
    let mut result = to_bit(state.get_value(1).as_number().unwrap_or(0.0));
    for i in 2..=state.get_top() as i32 {
        result &= to_bit(state.get_value(i).as_number().unwrap_or(0.0));
    }
    state.push(Value::integer(result))?;
    Ok(1)
}

fn bit_bor(state: &mut State) -> LuaResult<usize> {
    let mut result = to_bit(state.get_value(1).as_number().unwrap_or(0.0));
    for i in 2..=state.get_top() as i32 {
        result |= to_bit(state.get_value(i).as_number().unwrap_or(0.0));
    }
    state.push(Value::integer(result))?;
    Ok(1)
}

fn bit_bxor(state: &mut State) -> LuaResult<usize> {
    let mut result = to_bit(state.get_value(1).as_number().unwrap_or(0.0));
    for i in 2..=state.get_top() as i32 {
        result ^= to_bit(state.get_value(i).as_number().unwrap_or(0.0));
    }
    state.push(Value::integer(result))?;
    Ok(1)
}

fn bit_lshift(state: &mut State) -> LuaResult<usize> {
    let n = to_bit(state.get_value(1).as_number().unwrap_or(0.0)) as u32;
    let shift = (state.get_value(2).as_number().unwrap_or(0.0) as u32) & 31;
    state.push(Value::integer((n << shift) as i32))?;
    Ok(1)
}

fn bit_rshift(state: &mut State) -> LuaResult<usize> {
    let n = to_bit(state.get_value(1).as_number().unwrap_or(0.0)) as u32;
    let shift = (state.get_value(2).as_number().unwrap_or(0.0) as u32) & 31;
    state.push(Value::integer((n >> shift) as i32))?;
    Ok(1)
}

fn bit_arshift(state: &mut State) -> LuaResult<usize> {
    let n = to_bit(state.get_value(1).as_number().unwrap_or(0.0));
    let shift = (state.get_value(2).as_number().unwrap_or(0.0) as u32) & 31;
    state.push(Value::integer(n >> shift))?;
    Ok(1)
}

fn bit_rol(state: &mut State) -> LuaResult<usize> {
    let n = to_bit(state.get_value(1).as_number().unwrap_or(0.0)) as u32;
    let shift = (state.get_value(2).as_number().unwrap_or(0.0) as u32) & 31;
    state.push(Value::integer(n.rotate_left(shift) as i32))?;
    Ok(1)
}

fn bit_ror(state: &mut State) -> LuaResult<usize> {
    let n = to_bit(state.get_value(1).as_number().unwrap_or(0.0)) as u32;
    let shift = (state.get_value(2).as_number().unwrap_or(0.0) as u32) & 31;
    state.push(Value::integer(n.rotate_right(shift) as i32))?;
    Ok(1)
}

fn bit_bswap(state: &mut State) -> LuaResult<usize> {
    let n = to_bit(state.get_value(1).as_number().unwrap_or(0.0)) as u32;
    state.push(Value::integer(n.swap_bytes() as i32))?;
    Ok(1)
}

fn bit_tohex(state: &mut State) -> LuaResult<usize> {
    let n = to_bit(state.get_value(1).as_number().unwrap_or(0.0)) as u32;
    let digits = state.get_value(2).as_number().map(|d| d as i32).unwrap_or(8);

    let hex = if digits < 0 {
        format!("{:0width$X}", n, width = (-digits) as usize)
    } else {
        format!("{:0width$x}", n, width = digits as usize)
    };

    let result = state.intern_string(&hex);
    state.push(result)?;
    Ok(1)
}

/// Register the jit library as a global
fn register_jit_library(state: &mut State) {
    let jit = state.create_table(0, 8);

    let add_func = |state: &mut State, tbl: GcRef<Table>, name: &str, func: crate::value::NativeFn| {
        let native = NativeFunction::new(func);
        let func_ref = state.gc.alloc(Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    add_func(state, jit, "on", jit_on);
    add_func(state, jit, "off", jit_off);
    add_func(state, jit, "flush", jit_flush);
    add_func(state, jit, "status", jit_status);

    // Set version info
    let version_key = state.intern_string("version");
    let version_val = state.intern_string("LuaJIT-RS 0.1.0");
    unsafe { (*jit.as_ptr()).set(version_key, version_val); }

    let version_num_key = state.intern_string("version_num");
    unsafe { (*jit.as_ptr()).set(version_num_key, Value::integer(20100)); }

    let os_key = state.intern_string("os");
    let os_val = state.intern_string(std::env::consts::OS);
    unsafe { (*jit.as_ptr()).set(os_key, os_val); }

    let arch_key = state.intern_string("arch");
    let arch_val = state.intern_string(std::env::consts::ARCH);
    unsafe { (*jit.as_ptr()).set(arch_key, arch_val); }

    state.set_global("jit", Value::table(jit));
}

/// jit library loader (for require compatibility)
fn jit_loader(state: &mut State) -> LuaResult<usize> {
    // Return the global jit table
    let key = state.intern_string("jit");
    let jit = unsafe {
        let globals = &*state.globals.as_ptr();
        globals.get(&key)
    };
    state.push(jit)?;
    Ok(1)
}

fn jit_on(_state: &mut State) -> LuaResult<usize> { Ok(0) }
fn jit_off(_state: &mut State) -> LuaResult<usize> { Ok(0) }
fn jit_flush(_state: &mut State) -> LuaResult<usize> { Ok(0) }

fn jit_status(state: &mut State) -> LuaResult<usize> {
    state.push(Value::boolean(false))?; // JIT is off
    Ok(1)
}

/// package.loadlib(libname, funcname) -> function | nil, error
/// Loads a C library - returns nil since we don't support C libraries
fn package_loadlib(state: &mut State) -> LuaResult<usize> {
    state.push(Value::nil())?;
    let msg = state.intern_string("C library loading not supported");
    state.push(msg)?;
    Ok(2)
}

/// package.seeall(module) - sets up module to see all globals
fn package_seeall(state: &mut State) -> LuaResult<usize> {
    let module = state.get_value(1);
    if let Some(t) = module.as_table() {
        // Create metatable with __index = _G
        let mt = state.create_table(0, 1);
        let index_key = state.intern_string("__index");
        unsafe {
            (*mt.as_ptr()).set(index_key, Value::table(state.globals));
            (*t.as_ptr()).set_metatable(Some(mt));
        }
    }
    Ok(0)
}

/// Loader 1: preload loader - looks in package.preload
fn loader_preload(state: &mut State) -> LuaResult<usize> {
    let modname = state.get_value(1);
    let modname_str = if let Some(s) = modname.as_string() {
        unsafe { (*s.as_ptr()).as_str() }.map(|s| s.to_string())
    } else {
        None
    };

    if let Some(name) = modname_str {
        // Get package.preload
        let package_key = state.intern_string("package");
        let package = unsafe { (*state.globals.as_ptr()).get(&package_key) };
        if let Some(pkg) = package.as_table() {
            let preload_key = state.intern_string("preload");
            let preload = unsafe { (*pkg.as_ptr()).get(&preload_key) };
            if let Some(preload_tbl) = preload.as_table() {
                let name_key = state.intern_string(&name);
                let loader = unsafe { (*preload_tbl.as_ptr()).get(&name_key) };
                if !loader.is_nil() {
                    state.push(loader)?;
                    return Ok(1);
                }
            }
        }
    }
    let msg = state.intern_string("\n\tno field package.preload");
    state.push(msg)?;
    Ok(1)
}

/// Loader 2: Lua file loader
fn loader_lua(state: &mut State) -> LuaResult<usize> {
    let modname = state.get_value(1);
    let modname_str = if let Some(s) = modname.as_string() {
        unsafe { (*s.as_ptr()).as_str() }.map(|s| s.to_string())
    } else {
        None
    };

    if modname_str.is_none() {
        let msg = state.intern_string("\n\t(invalid module name)");
        state.push(msg)?;
        return Ok(1);
    }
    let name = modname_str.unwrap();

    // Get package.path
    let path = get_package_path(state);
    let mut errors = String::new();

    for template in path.split(';') {
        let filepath = template.replace("?", &name.replace(".", "/"));
        if Path::new(&filepath).exists() {
            // Load the file
            let source = match std::fs::read_to_string(&filepath) {
                Ok(s) => s,
                Err(_) => continue,
            };
            match state.load_string(&source, &format!("@{}", filepath)) {
                Ok(func) => {
                    state.push(Value::function(func))?;
                    return Ok(1);
                }
                Err(_) => continue,
            }
        }
        errors.push_str(&format!("\n\tno file '{}'", filepath));
    }

    let msg = state.intern_string(&errors);
    state.push(msg)?;
    Ok(1)
}

/// Loader 3: C library loader (stub)
fn loader_c(_state: &mut State) -> LuaResult<usize> {
    Ok(0) // No C library support
}

/// Loader 4: all-in-one C loader (stub)
fn loader_croot(_state: &mut State) -> LuaResult<usize> {
    Ok(0) // No C library support
}
