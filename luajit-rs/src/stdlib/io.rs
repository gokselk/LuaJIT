//! IO library

use crate::value::{Value, LuaResult, LuaError, GcRef, Userdata};
use crate::value::userdata::UserdataAllocator;
use crate::vm::State;
use std::io::{self, Write, BufRead, Read, Seek, SeekFrom};
use std::fs::{File, OpenOptions};
use std::cell::RefCell;

/// Allocate a FileHandle as userdata
fn alloc_file_userdata(fh: FileHandle) -> GcRef<Userdata> {
    let ptr = UserdataAllocator::allocate(fh, 0);
    GcRef::new(ptr)
}

/// File handle type for Lua
#[derive(Debug)]
pub struct FileHandle {
    inner: RefCell<FileInner>,
}

#[derive(Debug)]
enum FileInner {
    Stdin,
    Stdout,
    Stderr,
    File(File),
    Closed,
}

impl FileHandle {
    pub fn stdin() -> Self {
        Self { inner: RefCell::new(FileInner::Stdin) }
    }

    pub fn stdout() -> Self {
        Self { inner: RefCell::new(FileInner::Stdout) }
    }

    pub fn stderr() -> Self {
        Self { inner: RefCell::new(FileInner::Stderr) }
    }

    pub fn from_file(file: File) -> Self {
        Self { inner: RefCell::new(FileInner::File(file)) }
    }

    pub fn is_closed(&self) -> bool {
        matches!(*self.inner.borrow(), FileInner::Closed)
    }

    pub fn close(&self) -> io::Result<()> {
        let mut inner = self.inner.borrow_mut();
        match &*inner {
            FileInner::File(_) => {
                *inner = FileInner::Closed;
                Ok(())
            }
            FileInner::Closed => Err(io::Error::new(io::ErrorKind::Other, "file already closed")),
            _ => Err(io::Error::new(io::ErrorKind::Other, "cannot close standard file")),
        }
    }

    pub fn write(&self, data: &[u8]) -> io::Result<usize> {
        let mut inner = self.inner.borrow_mut();
        match &mut *inner {
            FileInner::Stdout => {
                io::stdout().write(data)
            }
            FileInner::Stderr => {
                io::stderr().write(data)
            }
            FileInner::File(f) => f.write(data),
            FileInner::Stdin => Err(io::Error::new(io::ErrorKind::Other, "cannot write to stdin")),
            FileInner::Closed => Err(io::Error::new(io::ErrorKind::Other, "file is closed")),
        }
    }

    pub fn read_line(&self) -> io::Result<String> {
        let inner = self.inner.borrow();
        match &*inner {
            FileInner::Stdin => {
                drop(inner);
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line)?;
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Ok(line)
            }
            FileInner::File(_) => {
                drop(inner);
                let mut inner = self.inner.borrow_mut();
                if let FileInner::File(f) = &mut *inner {
                    let mut buf_reader = io::BufReader::new(f);
                    let mut line = String::new();
                    buf_reader.read_line(&mut line)?;
                    if line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }
                    Ok(line)
                } else {
                    Err(io::Error::new(io::ErrorKind::Other, "unexpected state"))
                }
            }
            FileInner::Closed => Err(io::Error::new(io::ErrorKind::Other, "file is closed")),
            _ => Err(io::Error::new(io::ErrorKind::Other, "cannot read from this file")),
        }
    }

    pub fn read_all(&self) -> io::Result<String> {
        let inner = self.inner.borrow();
        match &*inner {
            FileInner::Stdin => {
                drop(inner);
                let mut content = String::new();
                io::stdin().lock().read_to_string(&mut content)?;
                Ok(content)
            }
            FileInner::File(_) => {
                drop(inner);
                let mut inner = self.inner.borrow_mut();
                if let FileInner::File(f) = &mut *inner {
                    let mut content = String::new();
                    f.read_to_string(&mut content)?;
                    Ok(content)
                } else {
                    Err(io::Error::new(io::ErrorKind::Other, "unexpected state"))
                }
            }
            FileInner::Closed => Err(io::Error::new(io::ErrorKind::Other, "file is closed")),
            _ => Err(io::Error::new(io::ErrorKind::Other, "cannot read from this file")),
        }
    }

    pub fn read_bytes(&self, n: usize) -> io::Result<Vec<u8>> {
        let inner = self.inner.borrow();
        match &*inner {
            FileInner::Stdin => {
                drop(inner);
                let mut buf = vec![0u8; n];
                let read = io::stdin().lock().read(&mut buf)?;
                buf.truncate(read);
                Ok(buf)
            }
            FileInner::File(_) => {
                drop(inner);
                let mut inner = self.inner.borrow_mut();
                if let FileInner::File(f) = &mut *inner {
                    let mut buf = vec![0u8; n];
                    let read = f.read(&mut buf)?;
                    buf.truncate(read);
                    Ok(buf)
                } else {
                    Err(io::Error::new(io::ErrorKind::Other, "unexpected state"))
                }
            }
            FileInner::Closed => Err(io::Error::new(io::ErrorKind::Other, "file is closed")),
            _ => Err(io::Error::new(io::ErrorKind::Other, "cannot read from this file")),
        }
    }

    pub fn flush(&self) -> io::Result<()> {
        let mut inner = self.inner.borrow_mut();
        match &mut *inner {
            FileInner::Stdout => io::stdout().flush(),
            FileInner::Stderr => io::stderr().flush(),
            FileInner::File(f) => f.flush(),
            FileInner::Closed => Err(io::Error::new(io::ErrorKind::Other, "file is closed")),
            _ => Ok(()),
        }
    }

    pub fn seek(&self, pos: SeekFrom) -> io::Result<u64> {
        let mut inner = self.inner.borrow_mut();
        match &mut *inner {
            FileInner::File(f) => f.seek(pos),
            FileInner::Closed => Err(io::Error::new(io::ErrorKind::Other, "file is closed")),
            _ => Err(io::Error::new(io::ErrorKind::Other, "cannot seek on this file")),
        }
    }
}


pub fn register_io(state: &mut State) {
    // Note: GC is disabled by register_all() before calling this function

    // Create io table
    let io_table = state.create_table(0, 16);

    // Helper to add a function to the table
    let add_func = |state: &mut State, tbl: crate::value::GcRef<crate::value::Table>, name: &str, func: crate::value::NativeFn| {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    // Add functions to io table
    add_func(state, io_table, "close", io_close);
    add_func(state, io_table, "flush", io_flush);
    add_func(state, io_table, "input", io_input);
    add_func(state, io_table, "lines", io_lines);
    add_func(state, io_table, "open", io_open);
    add_func(state, io_table, "output", io_output);
    add_func(state, io_table, "popen", io_popen);
    add_func(state, io_table, "read", io_read);
    add_func(state, io_table, "tmpfile", io_tmpfile);
    add_func(state, io_table, "type", io_type);
    add_func(state, io_table, "write", io_write);

    // Create file handle metatable
    let file_mt = state.create_table(0, 16);
    add_func(state, file_mt, "close", file_close);
    add_func(state, file_mt, "flush", file_flush);
    add_func(state, file_mt, "lines", file_lines);
    add_func(state, file_mt, "read", file_read_method);
    add_func(state, file_mt, "seek", file_seek);
    add_func(state, file_mt, "setvbuf", file_setvbuf);
    add_func(state, file_mt, "write", file_write_method);

    // Set __index to point to itself for method calls
    unsafe {
        let index_key = state.intern_string("__index");
        (*file_mt.as_ptr()).set(index_key, Value::table(file_mt));

        // Add __tostring
        let tostring_key = state.intern_string("__tostring");
        let native = crate::value::NativeFunction::new(file_tostring);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        (*file_mt.as_ptr()).set(tostring_key, Value::function(func_ref));

        // Add __gc (stub for now)
        let gc_key = state.intern_string("__gc");
        let gc_native = crate::value::NativeFunction::new(file_gc);
        let gc_func = state.gc.alloc(crate::value::Function::Native(gc_native));
        (*file_mt.as_ptr()).set(gc_key, Value::function(gc_func));
    }

    // Store the metatable in registry for later use
    let mt_key = state.intern_string("_FILE_MT");
    unsafe {
        (*state.registry.as_ptr()).set(mt_key, Value::table(file_mt));
    }

    // Create stdin, stdout, stderr file handles with metatable
    let stdin = alloc_file_userdata(FileHandle::stdin());
    let stdout = alloc_file_userdata(FileHandle::stdout());
    let stderr = alloc_file_userdata(FileHandle::stderr());

    unsafe {
        (*stdin.as_ptr()).set_metatable(Some(file_mt));
        (*stdout.as_ptr()).set_metatable(Some(file_mt));
        (*stderr.as_ptr()).set_metatable(Some(file_mt));
    }

    // Store file handles in io table
    let stdin_key = state.intern_string("stdin");
    let stdout_key = state.intern_string("stdout");
    let stderr_key = state.intern_string("stderr");
    unsafe {
        (*io_table.as_ptr()).set(stdin_key, Value::userdata(stdin));
        (*io_table.as_ptr()).set(stdout_key, Value::userdata(stdout));
        (*io_table.as_ptr()).set(stderr_key, Value::userdata(stderr));
    }

    // Store default input/output in registry
    let input_key = state.intern_string("_IO_input");
    let output_key = state.intern_string("_IO_output");
    unsafe {
        (*state.registry.as_ptr()).set(input_key, Value::userdata(stdin));
        (*state.registry.as_ptr()).set(output_key, Value::userdata(stdout));
    }

    state.set_global("io", Value::table(io_table));
}

fn get_file_handle<'a>(state: &'a State, idx: i32) -> Option<&'a FileHandle> {
    let val = state.get_value(idx);
    if let Some(ud) = val.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        ud.downcast_ref::<FileHandle>()
    } else {
        None
    }
}

fn io_open(state: &mut State) -> LuaResult<usize> {
    let filename = state.get_value(1);
    let mode = state.get_value(2);

    let filename = if let Some(s) = filename.as_string() {
        unsafe { (*s.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "open".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let mode_str = if let Some(s) = mode.as_string() {
        unsafe { (*s.as_ptr()).as_str().unwrap_or("r").to_string() }
    } else {
        "r".to_string()
    };

    let file = match mode_str.as_str() {
        "r" => OpenOptions::new().read(true).open(&filename),
        "w" => OpenOptions::new().write(true).create(true).truncate(true).open(&filename),
        "a" => OpenOptions::new().write(true).create(true).append(true).open(&filename),
        "r+" => OpenOptions::new().read(true).write(true).open(&filename),
        "w+" => OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&filename),
        "a+" => OpenOptions::new().read(true).write(true).create(true).append(true).open(&filename),
        "rb" => OpenOptions::new().read(true).open(&filename),
        "wb" => OpenOptions::new().write(true).create(true).truncate(true).open(&filename),
        "ab" => OpenOptions::new().write(true).create(true).append(true).open(&filename),
        "r+b" | "rb+" => OpenOptions::new().read(true).write(true).open(&filename),
        "w+b" | "wb+" => OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&filename),
        "a+b" | "ab+" => OpenOptions::new().read(true).write(true).create(true).append(true).open(&filename),
        _ => return Err(LuaError::ArgumentError {
            func: "open".to_string(),
            arg: 2,
            msg: format!("invalid mode '{}'", mode_str),
        }),
    };

    match file {
        Ok(f) => {
            let handle = alloc_file_userdata(FileHandle::from_file(f));
            state.push(Value::userdata(handle))?;
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

fn io_close(state: &mut State) -> LuaResult<usize> {
    let file = if state.get_top() >= 1 {
        state.get_value(1)
    } else {
        // Close default output
        let key = state.intern_string("_IO_output");
        let registry = unsafe { &*state.registry.as_ptr() };
        registry.get(&key)
    };

    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            match fh.close() {
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
        } else {
            Err(LuaError::ArgumentError {
                func: "close".to_string(),
                arg: 1,
                msg: "file expected".to_string(),
            })
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "close".to_string(),
            arg: 1,
            msg: "file expected".to_string(),
        })
    }
}

fn io_flush(state: &mut State) -> LuaResult<usize> {
    let file = if state.get_top() >= 1 {
        state.get_value(1)
    } else {
        // Flush default output
        let key = state.intern_string("_IO_output");
        let registry = unsafe { &*state.registry.as_ptr() };
        registry.get(&key)
    };

    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            match fh.flush() {
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
        } else {
            io::stdout().flush().ok();
            state.push(Value::boolean(true))?;
            Ok(1)
        }
    } else {
        io::stdout().flush().ok();
        state.push(Value::boolean(true))?;
        Ok(1)
    }
}

fn io_input(state: &mut State) -> LuaResult<usize> {
    if state.get_top() >= 1 {
        let arg = state.get_value(1);
        let new_input = if let Some(s) = arg.as_string() {
            // Open file for reading
            let filename = unsafe { (*s.as_ptr()).as_str().unwrap_or("").to_string() };
            match File::open(&filename) {
                Ok(f) => Value::userdata(alloc_file_userdata(FileHandle::from_file(f))),
                Err(e) => return Err(LuaError::RuntimeError(format!("cannot open file '{}': {}", filename, e))),
            }
        } else if arg.is_userdata() {
            arg
        } else {
            return Err(LuaError::ArgumentError {
                func: "input".to_string(),
                arg: 1,
                msg: "string or file expected".to_string(),
            });
        };

        let key = state.intern_string("_IO_input");
        unsafe { (*state.registry.as_ptr()).set(key, new_input); }
    }

    // Return current input
    let key = state.intern_string("_IO_input");
    let registry = unsafe { &*state.registry.as_ptr() };
    let input = registry.get(&key);
    state.push(input)?;
    Ok(1)
}

fn io_output(state: &mut State) -> LuaResult<usize> {
    if state.get_top() >= 1 {
        let arg = state.get_value(1);
        let new_output = if let Some(s) = arg.as_string() {
            // Open file for writing
            let filename = unsafe { (*s.as_ptr()).as_str().unwrap_or("").to_string() };
            match File::create(&filename) {
                Ok(f) => Value::userdata(alloc_file_userdata(FileHandle::from_file(f))),
                Err(e) => return Err(LuaError::RuntimeError(format!("cannot open file '{}': {}", filename, e))),
            }
        } else if arg.is_userdata() {
            arg
        } else {
            return Err(LuaError::ArgumentError {
                func: "output".to_string(),
                arg: 1,
                msg: "string or file expected".to_string(),
            });
        };

        let key = state.intern_string("_IO_output");
        unsafe { (*state.registry.as_ptr()).set(key, new_output); }
    }

    // Return current output
    let key = state.intern_string("_IO_output");
    let registry = unsafe { &*state.registry.as_ptr() };
    let output = registry.get(&key);
    state.push(output)?;
    Ok(1)
}

fn io_read(state: &mut State) -> LuaResult<usize> {
    // Get default input
    let key = state.intern_string("_IO_input");
    let registry = unsafe { &*state.registry.as_ptr() };
    let input = registry.get(&key);

    if let Some(ud) = input.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            return file_read(state, fh, 1);
        }
    }

    // Fall back to stdin
    let fmt = state.get_value(1);
    let result = read_from_stdin(state, &fmt)?;
    state.push(result)?;
    Ok(1)
}

fn file_read(state: &mut State, fh: &FileHandle, first_arg: i32) -> LuaResult<usize> {
    let n = state.get_top();
    if n < first_arg as usize {
        // Default: read line
        match fh.read_line() {
            Ok(line) => {
                let val = state.intern_string(&line);
                state.push(val)?;
                Ok(1)
            }
            Err(_) => {
                state.push(Value::nil())?;
                Ok(1)
            }
        }
    } else {
        let mut count = 0;
        for i in first_arg..=n as i32 {
            let fmt = state.get_value(i);
            let result = if let Some(str_ref) = fmt.as_string() {
                let str_val = unsafe { &*str_ref.as_ptr() };
                match str_val.as_str() {
                    Some("*l") | Some("l") => {
                        match fh.read_line() {
                            Ok(line) => state.intern_string(&line),
                            Err(_) => Value::nil(),
                        }
                    }
                    Some("*L") | Some("L") => {
                        // Read line with newline
                        match fh.read_line() {
                            Ok(mut line) => {
                                line.push('\n');
                                state.intern_string(&line)
                            }
                            Err(_) => Value::nil(),
                        }
                    }
                    Some("*a") | Some("a") => {
                        match fh.read_all() {
                            Ok(content) => state.intern_string(&content),
                            Err(_) => Value::nil(),
                        }
                    }
                    Some("*n") | Some("n") => {
                        match fh.read_line() {
                            Ok(line) => {
                                if let Ok(n) = line.trim().parse::<f64>() {
                                    Value::number(n)
                                } else {
                                    Value::nil()
                                }
                            }
                            Err(_) => Value::nil(),
                        }
                    }
                    _ => Value::nil(),
                }
            } else if let Some(num) = fmt.as_number() {
                let bytes = num as usize;
                match fh.read_bytes(bytes) {
                    Ok(data) => {
                        if data.is_empty() {
                            Value::nil()
                        } else {
                            state.intern_string(&String::from_utf8_lossy(&data))
                        }
                    }
                    Err(_) => Value::nil(),
                }
            } else {
                Value::nil()
            };
            state.push(result)?;
            count += 1;
        }
        Ok(count)
    }
}

fn read_from_stdin(state: &mut State, fmt: &Value) -> LuaResult<Value> {
    if let Some(str_ref) = fmt.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        match str_val.as_str() {
            Some("*l") | Some("l") | Some("*L") | Some("L") => {
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line).ok();
                if line.ends_with('\n') {
                    line.pop();
                }
                Ok(state.intern_string(&line))
            }
            Some("*a") | Some("a") => {
                let mut content = String::new();
                io::stdin().lock().read_to_string(&mut content).ok();
                Ok(state.intern_string(&content))
            }
            Some("*n") | Some("n") => {
                let mut line = String::new();
                io::stdin().lock().read_line(&mut line).ok();
                if let Ok(n) = line.trim().parse::<f64>() {
                    Ok(Value::number(n))
                } else {
                    Ok(Value::nil())
                }
            }
            _ => Ok(Value::nil()),
        }
    } else if let Some(n) = fmt.as_number() {
        let n = n as usize;
        let mut buf = vec![0u8; n];
        let read = io::stdin().lock().read(&mut buf).unwrap_or(0);
        buf.truncate(read);
        Ok(state.intern_string(&String::from_utf8_lossy(&buf)))
    } else {
        // Default: read line
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line).ok();
        if line.ends_with('\n') {
            line.pop();
        }
        Ok(state.intern_string(&line))
    }
}

fn io_write(state: &mut State) -> LuaResult<usize> {
    // Get default output
    let key = state.intern_string("_IO_output");
    let registry = unsafe { &*state.registry.as_ptr() };
    let output = registry.get(&key);

    let n = state.get_top();

    if let Some(ud) = output.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            for i in 1..=n as i32 {
                let val = state.get_value(i);
                if let Some(str_ref) = val.as_string() {
                    let str_val = unsafe { &*str_ref.as_ptr() };
                    if let Some(s) = str_val.as_str() {
                        fh.write(s.as_bytes()).ok();
                    }
                } else if let Some(num) = val.as_number() {
                    fh.write(format!("{}", num).as_bytes()).ok();
                }
            }
            state.push(output)?;
            return Ok(1);
        }
    }

    // Fall back to stdout
    let mut stdout = io::stdout();
    for i in 1..=n as i32 {
        let val = state.get_value(i);
        if let Some(str_ref) = val.as_string() {
            let str_val = unsafe { &*str_ref.as_ptr() };
            if let Some(s) = str_val.as_str() {
                stdout.write_all(s.as_bytes()).ok();
            }
        } else if let Some(num) = val.as_number() {
            write!(stdout, "{}", num).ok();
        }
    }

    state.push(Value::boolean(true))?;
    Ok(1)
}

fn io_lines(state: &mut State) -> LuaResult<usize> {
    // Create iterator function
    if state.get_top() >= 1 {
        let filename = state.get_value(1);
        if let Some(s) = filename.as_string() {
            let fname = unsafe { (*s.as_ptr()).as_str().unwrap_or("").to_string() };
            match File::open(&fname) {
                Ok(f) => {
                    let handle = alloc_file_userdata(FileHandle::from_file(f));
                    // Return iterator, file handle
                    let iter = crate::value::NativeFunction::new(lines_iterator);
                    let iter_func = state.gc.alloc(crate::value::Function::Native(iter));
                    state.push(Value::function(iter_func))?;
                    state.push(Value::userdata(handle))?;
                    state.push(Value::nil())?;
                    return Ok(3);
                }
                Err(e) => {
                    return Err(LuaError::RuntimeError(format!("cannot open file '{}': {}", fname, e)));
                }
            }
        }
    }

    // Use default input
    let key = state.intern_string("_IO_input");
    let registry = unsafe { &*state.registry.as_ptr() };
    let input = registry.get(&key);

    let iter = crate::value::NativeFunction::new(lines_iterator);
    let iter_func = state.gc.alloc(crate::value::Function::Native(iter));
    state.push(Value::function(iter_func))?;
    state.push(input)?;
    state.push(Value::nil())?;
    Ok(3)
}

fn lines_iterator(state: &mut State) -> LuaResult<usize> {
    let file = state.get_value(1);

    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            match fh.read_line() {
                Ok(line) if !line.is_empty() || !fh.is_closed() => {
                    // Check if we actually read something (not EOF)
                    let val = state.intern_string(&line);
                    state.push(val)?;
                    Ok(1)
                }
                _ => Ok(0), // EOF
            }
        } else {
            Ok(0)
        }
    } else {
        Ok(0)
    }
}

fn io_type(state: &mut State) -> LuaResult<usize> {
    let val = state.get_value(1);

    if let Some(ud) = val.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            if fh.is_closed() {
                let s = state.intern_string("closed file");
                state.push(s)?;
            } else {
                let s = state.intern_string("file");
                state.push(s)?;
            }
            return Ok(1);
        }
    }

    state.push(Value::nil())?;
    Ok(1)
}

fn io_tmpfile(state: &mut State) -> LuaResult<usize> {
    // Create a temporary file
    match tempfile() {
        Ok(f) => {
            let handle = alloc_file_userdata(FileHandle::from_file(f));
            state.push(Value::userdata(handle))?;
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

fn tempfile() -> io::Result<File> {
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    let tmp_dir = env::temp_dir();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!("lua_tmpfile_{}", timestamp);
    let path = tmp_dir.join(filename);

    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&path)
}

fn io_popen(state: &mut State) -> LuaResult<usize> {
    // popen is platform-specific and complex to implement properly
    // For now, return nil with an error message
    state.push(Value::nil())?;
    let msg = state.intern_string("popen not supported");
    state.push(msg)?;
    Ok(2)
}

// File method functions (called on file handle object)

/// file:close()
fn file_close(state: &mut State) -> LuaResult<usize> {
    let file = state.get_value(1);
    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            match fh.close() {
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
        } else {
            Err(LuaError::ArgumentError {
                func: "close".to_string(),
                arg: 1,
                msg: "file expected".to_string(),
            })
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "close".to_string(),
            arg: 1,
            msg: "file expected".to_string(),
        })
    }
}

/// file:flush()
fn file_flush(state: &mut State) -> LuaResult<usize> {
    let file = state.get_value(1);
    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            match fh.flush() {
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
        } else {
            Err(LuaError::ArgumentError {
                func: "flush".to_string(),
                arg: 1,
                msg: "file expected".to_string(),
            })
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "flush".to_string(),
            arg: 1,
            msg: "file expected".to_string(),
        })
    }
}

/// file:lines()
fn file_lines(state: &mut State) -> LuaResult<usize> {
    let file = state.get_value(1);

    let iter = crate::value::NativeFunction::new(lines_iterator);
    let iter_func = state.gc.alloc(crate::value::Function::Native(iter));
    state.push(Value::function(iter_func))?;
    state.push(file)?;
    state.push(Value::nil())?;
    Ok(3)
}

/// file:read(...)
fn file_read_method(state: &mut State) -> LuaResult<usize> {
    let file = state.get_value(1);

    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            return file_read(state, fh, 2);
        }
    }

    Err(LuaError::ArgumentError {
        func: "read".to_string(),
        arg: 1,
        msg: "file expected".to_string(),
    })
}

/// file:seek([whence [, offset]])
fn file_seek(state: &mut State) -> LuaResult<usize> {
    let file = state.get_value(1);

    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            let whence = if state.get_top() >= 2 {
                if let Some(s) = state.get_value(2).as_string() {
                    unsafe { (*s.as_ptr()).as_str().unwrap_or("cur").to_string() }
                } else {
                    "cur".to_string()
                }
            } else {
                "cur".to_string()
            };

            let offset = if state.get_top() >= 3 {
                state.to_number(3).unwrap_or(0.0) as i64
            } else {
                0
            };

            let seek_pos = match whence.as_str() {
                "set" => SeekFrom::Start(offset as u64),
                "cur" => SeekFrom::Current(offset),
                "end" => SeekFrom::End(offset),
                _ => {
                    return Err(LuaError::ArgumentError {
                        func: "seek".to_string(),
                        arg: 2,
                        msg: format!("invalid option '{}'", whence),
                    });
                }
            };

            match fh.seek(seek_pos) {
                Ok(pos) => {
                    state.push(Value::number(pos as f64))?;
                    Ok(1)
                }
                Err(e) => {
                    state.push(Value::nil())?;
                    let msg = state.intern_string(&e.to_string());
                    state.push(msg)?;
                    Ok(2)
                }
            }
        } else {
            Err(LuaError::ArgumentError {
                func: "seek".to_string(),
                arg: 1,
                msg: "file expected".to_string(),
            })
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "seek".to_string(),
            arg: 1,
            msg: "file expected".to_string(),
        })
    }
}

/// file:setvbuf(mode [, size])
fn file_setvbuf(state: &mut State) -> LuaResult<usize> {
    // Buffering is handled by the OS/libc, we just return success
    state.push(Value::boolean(true))?;
    Ok(1)
}

/// file:write(...)
fn file_write_method(state: &mut State) -> LuaResult<usize> {
    let file = state.get_value(1);

    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            let n = state.get_top();
            for i in 2..=n as i32 {
                let val = state.get_value(i);
                if let Some(str_ref) = val.as_string() {
                    let str_val = unsafe { &*str_ref.as_ptr() };
                    if let Some(s) = str_val.as_str() {
                        if let Err(e) = fh.write(s.as_bytes()) {
                            state.push(Value::nil())?;
                            let msg = state.intern_string(&e.to_string());
                            state.push(msg)?;
                            return Ok(2);
                        }
                    }
                } else if let Some(num) = val.as_number() {
                    if let Err(e) = fh.write(format!("{}", num).as_bytes()) {
                        state.push(Value::nil())?;
                        let msg = state.intern_string(&e.to_string());
                        state.push(msg)?;
                        return Ok(2);
                    }
                }
            }
            state.push(file)?;
            return Ok(1);
        }
    }

    Err(LuaError::ArgumentError {
        func: "write".to_string(),
        arg: 1,
        msg: "file expected".to_string(),
    })
}

/// __tostring metamethod for file handles
fn file_tostring(state: &mut State) -> LuaResult<usize> {
    let file = state.get_value(1);

    if let Some(ud) = file.as_userdata() {
        let ud_ptr = ud.as_ptr();
        let ud_ref = unsafe { &*ud_ptr };
        if let Some(fh) = ud_ref.downcast_ref::<FileHandle>() {
            let s = if fh.is_closed() {
                format!("file ({})", "closed")
            } else {
                format!("file ({:p})", ud_ptr)
            };
            let val = state.intern_string(&s);
            state.push(val)?;
            return Ok(1);
        }
    }

    let s = state.intern_string("file (?)");
    state.push(s)?;
    Ok(1)
}

/// __gc metamethod for file handles
fn file_gc(state: &mut State) -> LuaResult<usize> {
    // Close the file if it's open
    let file = state.get_value(1);

    if let Some(ud) = file.as_userdata() {
        let ud = unsafe { &*ud.as_ptr() };
        if let Some(fh) = ud.downcast_ref::<FileHandle>() {
            // Silently close the file
            let _ = fh.close();
        }
    }

    Ok(0)
}
