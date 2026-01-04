//! String library

use crate::value::{Value, LuaError, LuaResult};
use crate::vm::State;

pub fn register_string(state: &mut State) {
    // Create string table
    let string_table = state.create_table(0, 16);

    // Helper to add a function to the string table
    let add_func = |state: &mut State, tbl: crate::value::GcRef<crate::value::Table>, name: &str, func: crate::value::NativeFn| {
        let native = crate::value::NativeFunction::new(func);
        let func_ref = state.gc.alloc(crate::value::Function::Native(native));
        let key = state.intern_string(name);
        unsafe { (*tbl.as_ptr()).set(key, Value::function(func_ref)); }
    };

    // Add functions to string table
    add_func(state, string_table, "byte", string_byte);
    add_func(state, string_table, "char", string_char);
    add_func(state, string_table, "len", string_len);
    add_func(state, string_table, "lower", string_lower);
    add_func(state, string_table, "upper", string_upper);
    add_func(state, string_table, "rep", string_rep);
    add_func(state, string_table, "reverse", string_reverse);
    add_func(state, string_table, "sub", string_sub);
    add_func(state, string_table, "format", string_format);
    add_func(state, string_table, "find", string_find);
    add_func(state, string_table, "match", string_match);
    add_func(state, string_table, "gsub", string_gsub);
    add_func(state, string_table, "gmatch", string_gmatch);
    add_func(state, string_table, "dump", string_dump);

    state.set_global("string", Value::table(string_table));

    // Create string metatable with __index pointing to the string table
    // This allows string methods to be called on strings: ("hello"):upper()
    let string_mt = state.create_table(0, 2);
    let index_key = state.intern_string("__index");
    unsafe {
        (*string_mt.as_ptr()).set(index_key, Value::table(string_table));
    }

    // Set the string type metatable (LuaType::String = 4)
    state.metatables[4] = Some(string_mt);
}

fn string_byte(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let i = state.to_integer(2).unwrap_or(1);
    let j = state.to_integer(3).unwrap_or(i);

    if let Some(str_ref) = s.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        let bytes = str_val.as_bytes();
        let len = bytes.len() as i32;

        let start = if i >= 0 { i - 1 } else { len + i }.max(0) as usize;
        // Clamp end to 0 before casting to usize to avoid wrapping
        let end_i32 = if j >= 0 { j } else { len + j + 1 }.min(len).max(0);
        let end = end_i32 as usize;

        if start < end && start < bytes.len() {
            for byte in &bytes[start..end.min(bytes.len())] {
                state.push(Value::integer(*byte as i32))?;
            }
            Ok(end - start)
        } else {
            Ok(0)
        }
    } else {
        Err(LuaError::ArgumentError {
            func: "string.byte".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        })
    }
}

fn string_char(state: &mut State) -> LuaResult<usize> {
    let n = state.get_top();
    let mut result: Vec<u8> = Vec::with_capacity(n);

    for i in 1..=n as i32 {
        let c = state.to_integer(i).ok_or_else(|| LuaError::ArgumentError {
            func: "string.char".to_string(),
            arg: i as usize,
            msg: "number expected".to_string(),
        })?;
        if c < 0 || c > 255 {
            return Err(LuaError::ArgumentError {
                func: "string.char".to_string(),
                arg: i as usize,
                msg: "value out of range".to_string(),
            });
        }
        result.push(c as u8);
    }

    let val = state.intern_bytes(&result);
    state.push(val)?;
    Ok(1)
}

fn string_len(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    // Try number-to-string coercion first
    if let Some(n) = s.as_number() {
        let str_val = if n == (n as i64 as f64) && n.is_finite() {
            format!("{}", n as i64)
        } else {
            format!("{}", n)
        };
        state.push(Value::integer(str_val.len() as i32))?;
        return Ok(1);
    }
    if let Some(str_ref) = s.as_string() {
        let len = unsafe { (*str_ref.as_ptr()).len() };
        state.push(Value::integer(len as i32))?;
        Ok(1)
    } else {
        Err(state.arg_error("len", 1, &format!("string expected, got {}", s.lua_type())))
    }
}

fn string_lower(state: &mut State) -> LuaResult<usize> {
    let v = state.get_value(1);
    if let Some(str_ref) = v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        // Convert to lowercase byte by byte (ASCII only, non-ASCII bytes unchanged)
        let result: Vec<u8> = lua_str.as_bytes().iter().map(|&b| b.to_ascii_lowercase()).collect();
        let val = state.intern_bytes(&result);
        state.push(val)?;
        return Ok(1);
    }
    // Try number-to-string coercion
    if let Some(s) = state.to_lua_string(1) {
        let result: Vec<u8> = s.bytes().map(|b| b.to_ascii_lowercase()).collect();
        let val = state.intern_bytes(&result);
        state.push(val)?;
        return Ok(1);
    }
    Err(LuaError::ArgumentError {
        func: "string.lower".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

fn string_upper(state: &mut State) -> LuaResult<usize> {
    let v = state.get_value(1);
    if let Some(str_ref) = v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        // Convert to uppercase byte by byte (ASCII only, non-ASCII bytes unchanged)
        let result: Vec<u8> = lua_str.as_bytes().iter().map(|&b| b.to_ascii_uppercase()).collect();
        let val = state.intern_bytes(&result);
        state.push(val)?;
        return Ok(1);
    }
    // Try number-to-string coercion
    if let Some(s) = state.to_lua_string(1) {
        let result: Vec<u8> = s.bytes().map(|b| b.to_ascii_uppercase()).collect();
        let val = state.intern_bytes(&result);
        state.push(val)?;
        return Ok(1);
    }
    Err(LuaError::ArgumentError {
        func: "string.upper".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

fn string_rep(state: &mut State) -> LuaResult<usize> {
    // Get string bytes (either raw string or coerced number)
    let v = state.get_value(1);
    let bytes: Vec<u8> = if let Some(str_ref) = v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        lua_str.as_bytes().to_vec()
    } else if let Some(s) = state.to_lua_string(1) {
        s.into_bytes()
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.rep".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let n = state.to_integer(2).unwrap_or(0);

    if n <= 0 {
        let val = state.intern_bytes(&[]);
        state.push(val)?;
        return Ok(1);
    }

    // Get separator bytes
    let sep_v = state.get_value(3);
    let sep_bytes: Option<Vec<u8>> = if let Some(str_ref) = sep_v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        Some(lua_str.as_bytes().to_vec())
    } else if let Some(s) = state.to_lua_string(3) {
        Some(s.into_bytes())
    } else if !sep_v.is_nil() {
        return Err(LuaError::ArgumentError {
            func: "string.rep".to_string(),
            arg: 3,
            msg: "string expected".to_string(),
        });
    } else {
        None
    };

    let result = if let Some(sep) = sep_bytes {
        // With separator: repeat with separator between copies
        let mut result = Vec::with_capacity(bytes.len() * n as usize + sep.len() * (n as usize - 1));
        for i in 0..n {
            if i > 0 {
                result.extend_from_slice(&sep);
            }
            result.extend_from_slice(&bytes);
        }
        result
    } else {
        // No separator: simple repeat
        bytes.repeat(n as usize)
    };

    let val = state.intern_bytes(&result);
    state.push(val)?;
    Ok(1)
}

fn string_reverse(state: &mut State) -> LuaResult<usize> {
    let v = state.get_value(1);
    if let Some(str_ref) = v.as_string() {
        let lua_str = unsafe { &*str_ref.as_ptr() };
        let bytes = lua_str.as_bytes();
        let mut reversed: Vec<u8> = bytes.iter().copied().rev().collect();
        let val = state.intern_bytes(&reversed);
        state.push(val)?;
        return Ok(1);
    }
    // Try number-to-string coercion
    if let Some(s) = state.to_lua_string(1) {
        let reversed: Vec<u8> = s.bytes().rev().collect();
        let val = state.intern_bytes(&reversed);
        state.push(val)?;
        return Ok(1);
    }
    Err(LuaError::ArgumentError {
        func: "string.reverse".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

fn string_sub(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);

    // Get string, with number-to-string coercion
    let str_bytes = if let Some(str_ref) = s.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        str_val.as_bytes().to_vec()
    } else if let Some(n) = s.as_number() {
        // Number-to-string coercion
        let str_val = if n == (n as i64 as f64) && n.is_finite() {
            format!("{}", n as i64)
        } else {
            format!("{}", n)
        };
        str_val.into_bytes()
    } else {
        return Err(state.arg_error("sub", 1, &format!("string expected, got {}", s.lua_type())));
    };

    // Get start index, with proper type checking
    let i_val = state.get_value(2);
    let i = if state.get_top() < 2 || i_val.is_nil() {
        1
    } else if let Some(n) = i_val.coerce_to_integer() {
        n
    } else {
        return Err(state.arg_error("sub", 2, &format!("number expected, got {}", i_val.lua_type())));
    };

    // Get end index with coercion
    let j = state.to_integer(3).unwrap_or(-1);

    let len = str_bytes.len() as i32;
    let start = if i >= 0 { i - 1 } else { (len + i).max(0) } as usize;
    let end = if j >= 0 { j } else { len + j + 1 } as usize;

    let result = if start < end && start < str_bytes.len() {
        &str_bytes[start..end.min(str_bytes.len())]
    } else {
        &[]
    };
    let val = state.intern_bytes(result);
    state.push(val)?;
    Ok(1)
}

fn string_format(state: &mut State) -> LuaResult<usize> {
    let fmt = state.get_value(1);
    if let Some(str_ref) = fmt.as_string() {
        let str_val = unsafe { &*str_ref.as_ptr() };
        if let Some(fmt_str) = str_val.as_str() {
            let mut result = String::new();
            let mut chars = fmt_str.chars().peekable();
            let mut arg_idx = 2i32;

            while let Some(c) = chars.next() {
                if c == '%' {
                    match chars.peek() {
                        Some('%') => {
                            chars.next();
                            result.push('%');
                        }
                        Some('s') => {
                            chars.next();
                            if let Some(s) = state.to_lua_string(arg_idx) {
                                result.push_str(&s);
                            }
                            arg_idx += 1;
                        }
                        Some('d') | Some('i') => {
                            chars.next();
                            if let Some(n) = state.to_integer(arg_idx) {
                                result.push_str(&n.to_string());
                            }
                            arg_idx += 1;
                        }
                        Some('f') | Some('g') | Some('e') => {
                            chars.next();
                            if let Some(n) = state.to_number(arg_idx) {
                                result.push_str(&format!("{}", n));
                            }
                            arg_idx += 1;
                        }
                        Some('x') => {
                            chars.next();
                            if let Some(n) = state.to_integer(arg_idx) {
                                result.push_str(&format!("{:x}", n as u32));
                            }
                            arg_idx += 1;
                        }
                        Some('X') => {
                            chars.next();
                            if let Some(n) = state.to_integer(arg_idx) {
                                result.push_str(&format!("{:X}", n as u32));
                            }
                            arg_idx += 1;
                        }
                        Some('c') => {
                            chars.next();
                            if let Some(n) = state.to_integer(arg_idx) {
                                result.push(char::from_u32(n as u32).unwrap_or('?'));
                            }
                            arg_idx += 1;
                        }
                        Some('q') => {
                            chars.next();
                            if let Some(s) = state.to_lua_string(arg_idx) {
                                result.push('"');
                                for c in s.chars() {
                                    match c {
                                        '"' | '\\' | '\n' => {
                                            result.push('\\');
                                            result.push(c);
                                        }
                                        _ => result.push(c),
                                    }
                                }
                                result.push('"');
                            }
                            arg_idx += 1;
                        }
                        _ => result.push(c),
                    }
                } else {
                    result.push(c);
                }
            }

            let val = state.intern_string(&result);
            state.push(val)?;
            return Ok(1);
        }
    }
    Err(LuaError::ArgumentError {
        func: "string.format".to_string(),
        arg: 1,
        msg: "string expected".to_string(),
    })
}

/// Convert Lua pattern to a simple regex-compatible form
/// This is a simplified implementation that handles common cases
fn lua_pattern_to_regex(pattern: &str) -> String {
    let mut result = String::new();
    let mut chars = pattern.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '%' => {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    match next {
                        'd' => result.push_str("[0-9]"),
                        'a' => result.push_str("[a-zA-Z]"),
                        'l' => result.push_str("[a-z]"),
                        'u' => result.push_str("[A-Z]"),
                        'w' => result.push_str("[a-zA-Z0-9]"),
                        's' => result.push_str("[ \\t\\n\\r\\f\\v]"),
                        'p' => result.push_str("[!-/:-@\\[-`{-~]"),
                        'c' => result.push_str("[\\x00-\\x1f\\x7f]"),
                        'x' => result.push_str("[0-9a-fA-F]"),
                        'z' => result.push_str("\\x00"),
                        // Character class complements
                        'D' => result.push_str("[^0-9]"),
                        'A' => result.push_str("[^a-zA-Z]"),
                        'L' => result.push_str("[^a-z]"),
                        'U' => result.push_str("[^A-Z]"),
                        'W' => result.push_str("[^a-zA-Z0-9]"),
                        'S' => result.push_str("[^ \\t\\n\\r\\f\\v]"),
                        // Escaped special characters
                        _ => {
                            if "^$()%.[]*+-?".contains(next) {
                                result.push('\\');
                            }
                            result.push(next);
                        }
                    }
                }
            }
            // Escape regex special characters
            '^' | '$' | '(' | ')' | '.' | '[' | ']' | '+' | '?' | '{' | '}' | '|' | '\\' => {
                // Lua uses different anchors
                if c == '^' && result.is_empty() {
                    result.push('^'); // Start anchor
                } else if c == '$' {
                    result.push('$'); // End anchor
                } else {
                    result.push('\\');
                    result.push(c);
                }
            }
            '*' => result.push_str("*?"), // Lua * is non-greedy by default
            '-' => result.push_str("*?"), // Lua - is non-greedy *
            _ => result.push(c),
        }
    }

    result
}

fn string_find(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let pattern = state.get_value(2);
    let init = state.to_integer(3).unwrap_or(1);
    let plain = state.get_value(4).as_boolean().unwrap_or(false);

    let s_str = if let Some(str_ref) = s.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.find".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let pat_str = if let Some(str_ref) = pattern.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.find".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    // Handle negative indices
    let start_idx = if init >= 1 {
        (init - 1) as usize
    } else {
        (s_str.len() as i32 + init).max(0) as usize
    };

    if start_idx >= s_str.len() {
        return Ok(0); // Not found
    }

    let search_str = &s_str[start_idx..];

    if plain {
        // Plain text search
        if let Some(pos) = search_str.find(&pat_str) {
            let found_start = start_idx + pos + 1; // 1-based
            let found_end = found_start + pat_str.len() - 1;
            state.push(Value::integer(found_start as i32))?;
            state.push(Value::integer(found_end as i32))?;
            return Ok(2);
        }
    } else {
        // Pattern search - try simple literal match first
        if let Some(pos) = search_str.find(&pat_str) {
            let found_start = start_idx + pos + 1;
            let found_end = found_start + pat_str.len() - 1;
            state.push(Value::integer(found_start as i32))?;
            state.push(Value::integer(found_end as i32))?;
            return Ok(2);
        }
    }

    Ok(0) // Not found
}

/// Lua pattern matcher
struct PatternMatcher<'a> {
    pattern: &'a [u8],
    subject: &'a [u8],
    captures: Vec<(usize, usize)>, // (start, end) indices into subject
}

impl<'a> PatternMatcher<'a> {
    fn new(pattern: &'a [u8], subject: &'a [u8]) -> Self {
        Self {
            pattern,
            subject,
            captures: Vec::new(),
        }
    }

    /// Check if a character matches a character class
    fn match_class(&self, c: u8, class: u8) -> bool {
        match class.to_ascii_lowercase() {
            b'a' => c.is_ascii_alphabetic(),
            b'c' => c.is_ascii_control(),
            b'd' => c.is_ascii_digit(),
            b'g' => c.is_ascii_graphic(),
            b'l' => c.is_ascii_lowercase(),
            b'p' => c.is_ascii_punctuation(),
            b's' => c.is_ascii_whitespace(),
            b'u' => c.is_ascii_uppercase(),
            b'w' => c.is_ascii_alphanumeric(),
            b'x' => c.is_ascii_hexdigit(),
            b'z' => c == 0,
            _ => c == class,
        }
    }

    /// Match a single pattern element at position
    fn match_single(&self, p_pos: usize, s_pos: usize) -> bool {
        if s_pos >= self.subject.len() {
            return false;
        }
        let c = self.subject[s_pos];

        if p_pos >= self.pattern.len() {
            return false;
        }

        match self.pattern[p_pos] {
            b'.' => true, // Match any character
            b'%' if p_pos + 1 < self.pattern.len() => {
                let class = self.pattern[p_pos + 1];
                let result = self.match_class(c, class);
                // Uppercase means negation
                if class.is_ascii_uppercase() { !result } else { result }
            }
            b'[' => self.match_bracket(p_pos, c),
            ch => c == ch,
        }
    }

    /// Match a bracket expression [...]
    fn match_bracket(&self, p_pos: usize, c: u8) -> bool {
        let mut i = p_pos + 1;
        let mut negate = false;

        if i < self.pattern.len() && self.pattern[i] == b'^' {
            negate = true;
            i += 1;
        }

        let mut matched = false;
        while i < self.pattern.len() && self.pattern[i] != b']' {
            if i + 2 < self.pattern.len() && self.pattern[i + 1] == b'-' && self.pattern[i + 2] != b']' {
                // Range: a-z
                if c >= self.pattern[i] && c <= self.pattern[i + 2] {
                    matched = true;
                }
                i += 3;
            } else if self.pattern[i] == b'%' && i + 1 < self.pattern.len() {
                // Character class in bracket
                if self.match_class(c, self.pattern[i + 1]) {
                    matched = true;
                }
                i += 2;
            } else {
                if c == self.pattern[i] {
                    matched = true;
                }
                i += 1;
            }
        }

        if negate { !matched } else { matched }
    }

    /// Get the length of a pattern element
    fn pattern_element_len(&self, p_pos: usize) -> usize {
        if p_pos >= self.pattern.len() {
            return 0;
        }
        match self.pattern[p_pos] {
            b'%' => 2, // %x
            b'[' => {
                // Find closing ]
                let mut i = p_pos + 1;
                if i < self.pattern.len() && self.pattern[i] == b'^' {
                    i += 1;
                }
                if i < self.pattern.len() && self.pattern[i] == b']' {
                    i += 1; // ] at start is literal
                }
                while i < self.pattern.len() && self.pattern[i] != b']' {
                    i += 1;
                }
                i - p_pos + 1
            }
            _ => 1,
        }
    }

    /// Match pattern starting at given positions
    fn match_here(&mut self, mut p_pos: usize, mut s_pos: usize) -> Option<usize> {
        while p_pos < self.pattern.len() {
            // Check for end anchor $
            if self.pattern[p_pos] == b'$' && p_pos + 1 == self.pattern.len() {
                // $ at the end means must be at end of subject
                return if s_pos == self.subject.len() { Some(s_pos) } else { None };
            }

            // Check for capture start
            if self.pattern[p_pos] == b'(' {
                let capture_start = s_pos;
                p_pos += 1;

                // Find matching close paren and match contents
                if let Some(end_pos) = self.match_capture(p_pos, s_pos) {
                    // Find the closing paren position in pattern
                    let close_pos = self.find_capture_end(p_pos);
                    self.captures.push((capture_start, end_pos));
                    s_pos = end_pos;
                    p_pos = close_pos + 1;
                } else {
                    return None;
                }
                continue;
            }

            // Skip closing paren (handled by capture matching)
            if self.pattern[p_pos] == b')' {
                p_pos += 1;
                continue;
            }

            let elem_len = self.pattern_element_len(p_pos);

            // Check for quantifier after element
            let next_pos = p_pos + elem_len;
            if next_pos < self.pattern.len() {
                match self.pattern[next_pos] {
                    b'*' => {
                        // Zero or more (greedy)
                        let mut count = 0;
                        while self.match_single(p_pos, s_pos + count) {
                            count += 1;
                        }
                        // Try matching rest with decreasing counts
                        while count >= 0 {
                            if let Some(end) = self.match_here(next_pos + 1, s_pos + count as usize) {
                                return Some(end);
                            }
                            if count == 0 { break; }
                            count -= 1;
                        }
                        return None;
                    }
                    b'+' => {
                        // One or more (greedy)
                        if !self.match_single(p_pos, s_pos) {
                            return None;
                        }
                        let mut count = 1;
                        while self.match_single(p_pos, s_pos + count) {
                            count += 1;
                        }
                        // Try matching rest with decreasing counts
                        while count >= 1 {
                            if let Some(end) = self.match_here(next_pos + 1, s_pos + count) {
                                return Some(end);
                            }
                            count -= 1;
                        }
                        return None;
                    }
                    b'?' => {
                        // Zero or one
                        if self.match_single(p_pos, s_pos) {
                            if let Some(end) = self.match_here(next_pos + 1, s_pos + 1) {
                                return Some(end);
                            }
                        }
                        return self.match_here(next_pos + 1, s_pos);
                    }
                    b'-' => {
                        // Zero or more (non-greedy)
                        let mut count = 0;
                        loop {
                            if let Some(end) = self.match_here(next_pos + 1, s_pos + count) {
                                return Some(end);
                            }
                            if !self.match_single(p_pos, s_pos + count) {
                                return None;
                            }
                            count += 1;
                        }
                    }
                    _ => {
                        // No quantifier, match single element
                        if !self.match_single(p_pos, s_pos) {
                            return None;
                        }
                        s_pos += 1;
                        p_pos = next_pos;
                    }
                }
            } else {
                // Last element, no quantifier
                if !self.match_single(p_pos, s_pos) {
                    return None;
                }
                s_pos += 1;
                p_pos = next_pos;
            }
        }

        Some(s_pos)
    }

    /// Match a capture group, returning end position in subject
    fn match_capture(&mut self, p_pos: usize, s_pos: usize) -> Option<usize> {
        let close_pos = self.find_capture_end(p_pos);
        // Match the content between ( and )
        let inner_pattern = &self.pattern[p_pos..close_pos];
        let saved_pattern = self.pattern;
        self.pattern = inner_pattern;
        let result = self.match_here(0, s_pos);
        self.pattern = saved_pattern;
        result
    }

    /// Find the position of the closing paren for a capture
    fn find_capture_end(&self, start: usize) -> usize {
        let mut depth = 1;
        let mut i = start;
        while i < self.pattern.len() && depth > 0 {
            match self.pattern[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'%' => i += 1, // Skip escaped char
                _ => {}
            }
            i += 1;
        }
        i - 1 // Position of closing )
    }

    /// Try to match the pattern at any position in the subject
    fn find_match(&mut self, start: usize) -> Option<(usize, usize)> {
        // Check for anchor
        let (p_start, anchored) = if !self.pattern.is_empty() && self.pattern[0] == b'^' {
            (1, true)
        } else {
            (0, false)
        };

        let saved_pattern = self.pattern;
        self.pattern = &saved_pattern[p_start..];

        if anchored {
            self.captures.clear();
            if let Some(end) = self.match_here(0, start) {
                self.pattern = saved_pattern;
                return Some((start, end));
            }
        } else {
            for i in start..=self.subject.len() {
                self.captures.clear();
                if let Some(end) = self.match_here(0, i) {
                    self.pattern = saved_pattern;
                    return Some((i, end));
                }
            }
        }

        self.pattern = saved_pattern;
        None
    }
}

fn string_match(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let pattern = state.get_value(2);
    let init = state.to_integer(3).unwrap_or(1);

    let s_bytes = if let Some(str_ref) = s.as_string() {
        unsafe { (*str_ref.as_ptr()).as_bytes().to_vec() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "match".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let pat_bytes = if let Some(str_ref) = pattern.as_string() {
        unsafe { (*str_ref.as_ptr()).as_bytes().to_vec() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "match".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    // Handle negative indices
    let start_idx = if init >= 1 {
        (init - 1) as usize
    } else {
        (s_bytes.len() as i32 + init).max(0) as usize
    };

    if start_idx >= s_bytes.len() && !s_bytes.is_empty() {
        return Ok(0); // Not found
    }

    let mut matcher = PatternMatcher::new(&pat_bytes, &s_bytes);

    if let Some((match_start, match_end)) = matcher.find_match(start_idx) {
        if matcher.captures.is_empty() {
            // No captures, return the whole match
            let matched = &s_bytes[match_start..match_end];
            let val = state.intern_string(std::str::from_utf8(matched).unwrap_or(""));
            state.push(val)?;
            Ok(1)
        } else {
            // Return captures
            for (cap_start, cap_end) in &matcher.captures {
                let captured = &s_bytes[*cap_start..*cap_end];
                let val = state.intern_string(std::str::from_utf8(captured).unwrap_or(""));
                state.push(val)?;
            }
            Ok(matcher.captures.len())
        }
    } else {
        Ok(0) // Not found
    }
}

fn string_gsub(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let pattern = state.get_value(2);
    let repl = state.get_value(3);
    let max_n = state.to_integer(4);

    let s_str = if let Some(str_ref) = s.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gsub".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let pat_str = if let Some(str_ref) = pattern.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gsub".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    let repl_str = if let Some(str_ref) = repl.as_string() {
        unsafe { (*str_ref.as_ptr()).as_str().unwrap_or("").to_string() }
    } else if repl.is_function() {
        // Function replacement not fully supported yet
        String::new()
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gsub".to_string(),
            arg: 3,
            msg: "string/function/table expected".to_string(),
        });
    };

    let max_replacements = max_n.unwrap_or(i32::MAX) as usize;

    // Simple string replacement
    let mut result = s_str.clone();
    let mut count = 0;

    if !pat_str.is_empty() {
        while let Some(pos) = result.find(&pat_str) {
            if count >= max_replacements {
                break;
            }
            result = format!("{}{}{}", &result[..pos], repl_str, &result[pos + pat_str.len()..]);
            count += 1;
        }
    }

    let val = state.intern_string(&result);
    state.push(val)?;
    state.push(Value::integer(count as i32))?;
    Ok(2)
}

fn string_gmatch(state: &mut State) -> LuaResult<usize> {
    let s = state.get_value(1);
    let pattern = state.get_value(2);

    let s_bytes = if let Some(str_ref) = s.as_string() {
        unsafe { (*str_ref.as_ptr()).as_bytes().to_vec() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gmatch".to_string(),
            arg: 1,
            msg: "string expected".to_string(),
        });
    };

    let pat_bytes = if let Some(str_ref) = pattern.as_string() {
        unsafe { (*str_ref.as_ptr()).as_bytes().to_vec() }
    } else {
        return Err(LuaError::ArgumentError {
            func: "string.gmatch".to_string(),
            arg: 2,
            msg: "string expected".to_string(),
        });
    };

    // Create a table to store the iterator state (s, pattern, position)
    let iter_state = state.create_table(0, 4);

    // Store s
    let s_key = state.intern_string("s");
    let s_val = state.intern_bytes(&s_bytes);
    unsafe { (*iter_state.as_ptr()).set(s_key, s_val); }

    // Store pattern
    let pat_key = state.intern_string("p");
    let pat_val = state.intern_bytes(&pat_bytes);
    unsafe { (*iter_state.as_ptr()).set(pat_key, pat_val); }

    // Store position (1-indexed)
    let pos_key = state.intern_string("i");
    unsafe { (*iter_state.as_ptr()).set(pos_key, Value::integer(1)); }

    // Create the iterator function
    let native = crate::value::NativeFunction::new(gmatch_iter);
    let func_ref = state.gc.alloc(crate::value::Function::Native(native));

    // Return iterator function, state table, nil (initial control variable)
    state.push(Value::function(func_ref))?;
    state.push(Value::table(iter_state))?;
    state.push(Value::nil())?;
    Ok(3)
}

/// Iterator function for string.gmatch
fn gmatch_iter(state: &mut State) -> LuaResult<usize> {
    // Get the state table (first argument)
    let iter_state = state.get_value(1);

    let state_tbl = iter_state.as_table().ok_or_else(|| {
        LuaError::RuntimeError("gmatch: invalid iterator state".to_string())
    })?;

    let state_tbl_ptr = unsafe { &mut *state_tbl.as_ptr() };

    // Get s, pattern, position from state
    let s_key = state.intern_string("s");
    let s_val = state_tbl_ptr.get(&s_key);
    let s_bytes = if let Some(str_ref) = s_val.as_string() {
        unsafe { (*str_ref.as_ptr()).as_bytes().to_vec() }
    } else {
        return Ok(0); // End of iteration
    };

    let pat_key = state.intern_string("p");
    let pat_val = state_tbl_ptr.get(&pat_key);
    let pat_bytes = if let Some(str_ref) = pat_val.as_string() {
        unsafe { (*str_ref.as_ptr()).as_bytes().to_vec() }
    } else {
        return Ok(0);
    };

    let pos_key = state.intern_string("i");
    let pos_val = state_tbl_ptr.get(&pos_key);
    let pos = pos_val.as_integer().unwrap_or(1) as usize;

    if pos > s_bytes.len() {
        return Ok(0); // End of iteration
    }

    let start_idx = if pos >= 1 { pos - 1 } else { 0 };

    let mut matcher = PatternMatcher::new(&pat_bytes, &s_bytes);

    if let Some((match_start, match_end)) = matcher.find_match(start_idx) {
        // Update position for next iteration (skip at least 1 character to avoid infinite loops)
        let new_pos = (match_end.max(match_start + 1)) + 1;
        state_tbl_ptr.set(pos_key, Value::integer(new_pos as i32));

        if matcher.captures.is_empty() {
            // No captures, return the whole match
            let matched = &s_bytes[match_start..match_end];
            let val = state.intern_string(std::str::from_utf8(matched).unwrap_or(""));
            state.push(val)?;
            Ok(1)
        } else {
            // Return captures
            for (cap_start, cap_end) in &matcher.captures {
                let captured = &s_bytes[*cap_start..*cap_end];
                let val = state.intern_string(std::str::from_utf8(captured).unwrap_or(""));
                state.push(val)?;
            }
            Ok(matcher.captures.len())
        }
    } else {
        Ok(0) // No more matches
    }
}

/// string.dump(function [, strip]) -> binary string
/// Serializes a function's bytecode to a binary string that can be loaded with loadstring.
fn string_dump(state: &mut State) -> LuaResult<usize> {
    use crate::value::Function;

    let func_val = state.get_value(1);
    let _strip = state.get_value(2).as_boolean().unwrap_or(false);

    let func_ref = func_val.as_function().ok_or_else(|| LuaError::ArgumentError {
        func: "string.dump".to_string(),
        arg: 1,
        msg: "function expected".to_string(),
    })?;

    let func = unsafe { &*func_ref.as_ptr() };

    match func {
        Function::Native(_) => {
            return Err(LuaError::RuntimeError(
                "unable to dump given function".to_string()
            ));
        }
        Function::Lua(closure) => {
            let proto = unsafe { &*closure.proto.as_ptr() };
            let bytecode = dump_proto(proto);
            let result = state.intern_bytes(&bytecode);
            state.push(result)?;
            Ok(1)
        }
    }
}

/// Magic bytes for our bytecode format
pub const BYTECODE_MAGIC: &[u8] = b"\x1bLJR"; // "ESC LJR" - LuaJit Rust

/// Serialize a Proto to binary bytecode
pub fn dump_proto(proto: &crate::value::Proto) -> Vec<u8> {
    let mut buf = Vec::new();

    // Write header
    buf.extend_from_slice(BYTECODE_MAGIC);
    buf.push(1); // version

    // Write the proto recursively
    write_proto(&mut buf, proto);

    buf
}

fn write_u32(buf: &mut Vec<u8>, val: u32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_i32(buf: &mut Vec<u8>, val: i32) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_f64(buf: &mut Vec<u8>, val: f64) {
    buf.extend_from_slice(&val.to_le_bytes());
}

fn write_string(buf: &mut Vec<u8>, s: &[u8]) {
    write_u32(buf, s.len() as u32);
    buf.extend_from_slice(s);
}

fn write_proto(buf: &mut Vec<u8>, proto: &crate::value::Proto) {
    // Flags
    let mut flags: u8 = 0;
    if proto.is_vararg { flags |= 0x01; }
    buf.push(flags);

    // Basic info
    buf.push(proto.num_params);
    buf.push(proto.max_stack_size);
    buf.push(proto.num_upvalues);

    // Instructions
    write_u32(buf, proto.code.len() as u32);
    for instr in &proto.code {
        write_u32(buf, instr.raw());
    }

    // Constants (Value type)
    write_u32(buf, proto.constants.len() as u32);
    for constant in &proto.constants {
        if constant.is_nil() {
            buf.push(0); // nil
        } else if let Some(b) = constant.as_boolean() {
            buf.push(if b { 2 } else { 1 }); // bool
        } else if let Some(i) = constant.as_integer() {
            buf.push(3); // integer
            write_i32(buf, i);
        } else if let Some(n) = constant.as_number() {
            buf.push(4); // number
            write_f64(buf, n);
        } else {
            // Treat other types as nil for now
            buf.push(0);
        }
    }

    // String constants
    write_u32(buf, proto.string_constants.len() as u32);
    for s in &proto.string_constants {
        write_string(buf, s);
    }

    // Nested protos
    write_u32(buf, proto.protos.len() as u32);
    for child in &proto.protos {
        let child_proto = unsafe { &*child.as_ptr() };
        write_proto(buf, child_proto);
    }

    // Upvalue descriptors
    write_u32(buf, proto.upvalues.len() as u32);
    for uv in &proto.upvalues {
        buf.push(if uv.in_stack { 1 } else { 0 });
        buf.push(uv.index);
    }

    // Line info
    write_u32(buf, proto.line_defined);
    write_u32(buf, proto.last_line_defined);

    // Source name
    if let Some(src) = proto.source {
        let src_str = unsafe { &*src.as_ptr() };
        write_string(buf, src_str.as_bytes());
    } else {
        write_u32(buf, 0); // empty source
    }

    // Lineinfo (for debug)
    write_u32(buf, proto.lineinfo.len() as u32);
    for &line in &proto.lineinfo {
        write_u32(buf, line);
    }
}

/// Load a Proto from bytecode
pub fn load_proto(bytes: &[u8]) -> LuaResult<crate::value::Proto> {
    use crate::value::Proto;
    use crate::bytecode::Instruction;

    let mut cursor = 0;

    // Check magic
    if bytes.len() < BYTECODE_MAGIC.len() + 1 {
        return Err(LuaError::RuntimeError("invalid bytecode".to_string()));
    }
    if &bytes[..BYTECODE_MAGIC.len()] != BYTECODE_MAGIC {
        return Err(LuaError::RuntimeError("invalid bytecode magic".to_string()));
    }
    cursor += BYTECODE_MAGIC.len();

    // Check version
    if bytes[cursor] != 1 {
        return Err(LuaError::RuntimeError("unsupported bytecode version".to_string()));
    }
    cursor += 1;

    read_proto(bytes, &mut cursor)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> LuaResult<u32> {
    if *cursor + 4 > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let val = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(val)
}

fn read_i32(bytes: &[u8], cursor: &mut usize) -> LuaResult<i32> {
    if *cursor + 4 > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let val = i32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(val)
}

fn read_f64(bytes: &[u8], cursor: &mut usize) -> LuaResult<f64> {
    if *cursor + 8 > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let val = f64::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
        bytes[*cursor + 4],
        bytes[*cursor + 5],
        bytes[*cursor + 6],
        bytes[*cursor + 7],
    ]);
    *cursor += 8;
    Ok(val)
}

fn read_bytes(bytes: &[u8], cursor: &mut usize) -> LuaResult<Vec<u8>> {
    let len = read_u32(bytes, cursor)? as usize;
    if *cursor + len > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let data = bytes[*cursor..*cursor + len].to_vec();
    *cursor += len;
    Ok(data)
}

fn read_proto(bytes: &[u8], cursor: &mut usize) -> LuaResult<crate::value::Proto> {
    use crate::value::{Proto, UpvalueDesc, Value};
    use crate::bytecode::Instruction;

    if *cursor >= bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }

    // Flags
    let flags = bytes[*cursor];
    *cursor += 1;
    let is_vararg = flags & 0x01 != 0;

    // Basic info
    if *cursor + 3 > bytes.len() {
        return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
    }
    let num_params = bytes[*cursor];
    let max_stack_size = bytes[*cursor + 1];
    let num_upvalues = bytes[*cursor + 2];
    *cursor += 3;

    // Instructions
    let num_instructions = read_u32(bytes, cursor)? as usize;
    let mut code = Vec::with_capacity(num_instructions);
    for _ in 0..num_instructions {
        let raw = read_u32(bytes, cursor)?;
        code.push(Instruction(raw));
    }

    // Constants
    let num_constants = read_u32(bytes, cursor)? as usize;
    let mut constants = Vec::with_capacity(num_constants);
    for _ in 0..num_constants {
        if *cursor >= bytes.len() {
            return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
        }
        let tag = bytes[*cursor];
        *cursor += 1;
        let val = match tag {
            0 => Value::nil(),
            1 => Value::boolean(false),
            2 => Value::boolean(true),
            3 => Value::integer(read_i32(bytes, cursor)?),
            4 => Value::number(read_f64(bytes, cursor)?),
            _ => Value::nil(),
        };
        constants.push(val);
    }

    // String constants
    let num_string_constants = read_u32(bytes, cursor)? as usize;
    let mut string_constants = Vec::with_capacity(num_string_constants);
    for _ in 0..num_string_constants {
        string_constants.push(read_bytes(bytes, cursor)?);
    }

    // Nested protos (as child_protos for allocation later)
    let num_protos = read_u32(bytes, cursor)? as usize;
    let mut child_protos = Vec::with_capacity(num_protos);
    for _ in 0..num_protos {
        let child = read_proto(bytes, cursor)?;
        child_protos.push(Box::new(child));
    }

    // Upvalue descriptors
    let num_uv_desc = read_u32(bytes, cursor)? as usize;
    let mut upvalues = Vec::with_capacity(num_uv_desc);
    for _ in 0..num_uv_desc {
        if *cursor + 2 > bytes.len() {
            return Err(LuaError::RuntimeError("truncated bytecode".to_string()));
        }
        let in_stack = bytes[*cursor] != 0;
        let index = bytes[*cursor + 1];
        *cursor += 2;
        upvalues.push(UpvalueDesc {
            in_stack,
            index,
            name: None,
        });
    }

    // Line info
    let line_defined = read_u32(bytes, cursor)?;
    let last_line_defined = read_u32(bytes, cursor)?;

    // Source name (we don't intern it here - just store raw bytes)
    let source_bytes = read_bytes(bytes, cursor)?;

    // Lineinfo array
    let num_lineinfo = read_u32(bytes, cursor)? as usize;
    let mut lineinfo = Vec::with_capacity(num_lineinfo);
    for _ in 0..num_lineinfo {
        lineinfo.push(read_u32(bytes, cursor)?);
    }

    // Build the proto - note: source is None, will be set by allocate_proto_tree if needed
    let mut proto = Proto::new();
    proto.code = code;
    proto.constants = constants;
    proto.string_constants = string_constants;
    proto.child_protos = child_protos;
    proto.upvalues = upvalues;
    proto.lineinfo = lineinfo;
    proto.num_params = num_params;
    proto.is_vararg = is_vararg;
    proto.max_stack_size = max_stack_size;
    proto.num_upvalues = num_upvalues;
    proto.line_defined = line_defined;
    proto.last_line_defined = last_line_defined;
    // Store source bytes in string_constants if not empty, for potential re-serialization
    // For now, we leave source as None since we don't have access to the GC here

    Ok(proto)
}
