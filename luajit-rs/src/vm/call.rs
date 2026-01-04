//! Call frame management for the VM.

use crate::value::{Value, GcRef, Proto, Function, Closure};
use crate::bytecode::Instruction;
use smallvec::SmallVec;

/// Information about a function call
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// Function being called
    pub func: GcRef<Function>,
    /// Base stack index for this call
    pub base: usize,
    /// Saved program counter (for return)
    pub saved_pc: usize,
    /// Number of expected results
    pub num_results: i32,
    /// Is this a tailcall?
    pub is_tailcall: bool,
}

/// A call frame represents an active function call
#[derive(Debug)]
pub struct CallFrame {
    /// The closure being executed
    pub closure: Option<GcRef<Closure>>,
    /// Program counter (bytecode index)
    pub pc: usize,
    /// Base stack slot for this frame
    pub base: usize,
    /// Top of stack for this frame
    pub top: usize,
    /// Number of expected results (-1 = variable)
    pub num_results: i32,
    /// Is this a C/native call?
    pub is_native: bool,
    /// Saved vararg base (for vararg functions)
    pub vararg_base: Option<usize>,
    /// Number of varargs passed
    pub vararg_count: usize,
    /// Stack slot of the function itself (where results go)
    pub func_idx: usize,
    /// Function name (for debug info)
    pub name: Option<String>,
    /// How the function was called: "metamethod", "local", "global", "field", etc.
    pub name_what: Option<String>,
}

impl CallFrame {
    /// Create a new Lua call frame
    pub fn new_lua(closure: GcRef<Closure>, base: usize, func_idx: usize, num_results: i32) -> Self {
        Self {
            closure: Some(closure),
            pc: 0,
            base,
            top: base,
            num_results,
            is_native: false,
            vararg_base: None,
            vararg_count: 0,
            func_idx,
            name: None,
            name_what: None,
        }
    }

    /// Create a new Lua call frame with name info
    pub fn new_lua_named(closure: GcRef<Closure>, base: usize, func_idx: usize, num_results: i32, name: Option<String>, name_what: Option<String>) -> Self {
        Self {
            closure: Some(closure),
            pc: 0,
            base,
            top: base,
            num_results,
            is_native: false,
            vararg_base: None,
            vararg_count: 0,
            func_idx,
            name,
            name_what,
        }
    }

    /// Create a new native call frame
    pub fn new_native(base: usize, func_idx: usize, num_results: i32) -> Self {
        Self {
            closure: None,
            pc: 0,
            base,
            top: base,
            num_results,
            is_native: true,
            vararg_base: None,
            vararg_count: 0,
            func_idx,
            name: None,
            name_what: None,
        }
    }

    /// Create a new native call frame with name info
    pub fn new_native_named(base: usize, func_idx: usize, num_results: i32, name: Option<String>, name_what: Option<String>) -> Self {
        Self {
            closure: None,
            pc: 0,
            base,
            top: base,
            num_results,
            is_native: true,
            vararg_base: None,
            vararg_count: 0,
            func_idx,
            name,
            name_what,
        }
    }

    /// Get the prototype for this frame
    pub fn proto(&self) -> Option<&Proto> {
        self.closure.map(|c| unsafe { &(*(*c.as_ptr()).proto.as_ptr()) })
    }

    /// Get the current instruction
    pub fn current_instruction(&self) -> Option<Instruction> {
        self.proto().and_then(|p| p.code.get(self.pc).copied())
    }

    /// Advance PC and return the instruction
    pub fn fetch(&mut self) -> Option<Instruction> {
        let instr = self.current_instruction()?;
        self.pc += 1;
        Some(instr)
    }

    /// Jump relative to current PC
    pub fn jump(&mut self, offset: i16) {
        self.pc = ((self.pc as i32) + (offset as i32)) as usize;
    }

    /// Get a constant from the prototype
    pub fn get_constant(&self, index: usize) -> Value {
        self.proto()
            .and_then(|p| p.constants.get(index).copied())
            .unwrap_or(Value::nil())
    }

    /// Get a string constant (as bytes) from the prototype
    pub fn get_string_constant(&self, index: usize) -> Option<&[u8]> {
        self.proto()
            .and_then(|p| p.string_constants.get(index).map(|s| s.as_slice()))
    }
}

/// Maximum call stack depth to prevent stack overflow
const MAX_CALL_DEPTH: usize = 1000;

/// Call stack for the VM
pub struct CallStack {
    frames: SmallVec<[CallFrame; 16]>,
}

impl CallStack {
    pub fn new() -> Self {
        Self {
            frames: SmallVec::new(),
        }
    }

    /// Push a new frame (with stack overflow check)
    pub fn push(&mut self, frame: CallFrame) -> crate::value::LuaResult<()> {
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(crate::value::LuaError::StackOverflow);
        }
        self.frames.push(frame);
        Ok(())
    }

    /// Pop the current frame
    pub fn pop(&mut self) -> Option<CallFrame> {
        self.frames.pop()
    }

    /// Get the current frame
    pub fn current(&self) -> Option<&CallFrame> {
        self.frames.last()
    }

    /// Get the current frame mutably
    pub fn current_mut(&mut self) -> Option<&mut CallFrame> {
        self.frames.last_mut()
    }

    /// Get frame at depth (0 = current, 1 = caller, etc.)
    pub fn at_depth(&self, depth: usize) -> Option<&CallFrame> {
        if depth < self.frames.len() {
            Some(&self.frames[self.frames.len() - 1 - depth])
        } else {
            None
        }
    }

    /// Number of active frames
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Get all frames as a slice
    pub fn frames(&self) -> &[CallFrame] {
        &self.frames
    }

    /// Check if call stack is empty
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Clear all frames
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

impl Default for CallStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_stack() {
        let mut stack = CallStack::new();
        assert!(stack.is_empty());

        stack.push(CallFrame::new_native(0, 0, 1));
        assert_eq!(stack.depth(), 1);

        stack.push(CallFrame::new_native(10, 9, 0));
        assert_eq!(stack.depth(), 2);

        let frame = stack.pop().unwrap();
        assert_eq!(frame.base, 10);
        assert_eq!(stack.depth(), 1);
    }
}
