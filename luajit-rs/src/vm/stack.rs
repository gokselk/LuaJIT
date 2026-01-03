//! Lua stack implementation.
//!
//! The stack holds values during execution. It supports both the main stack
//! and per-function stack frames.

use crate::value::{Value, LuaError, LuaResult};

/// Default stack size
const DEFAULT_STACK_SIZE: usize = 1024;
/// Maximum stack size
const MAX_STACK_SIZE: usize = 1_000_000;

/// The Lua stack
pub struct Stack {
    /// Stack storage
    values: Vec<Value>,
    /// Current top of stack
    top: usize,
    /// Base for current frame
    base: usize,
}

impl Stack {
    /// Create a new stack
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_STACK_SIZE)
    }

    /// Create a stack with given capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let mut values = Vec::with_capacity(capacity);
        values.resize(capacity, Value::nil());
        Self {
            values,
            top: 0,
            base: 0,
        }
    }

    /// Get current top index
    #[inline]
    pub fn top(&self) -> usize {
        self.top
    }

    /// Set top index
    #[inline]
    pub fn set_top(&mut self, top: usize) {
        if top > self.values.len() {
            self.grow(top);
        }
        // Fill with nil if expanding
        for i in self.top..top {
            self.values[i] = Value::nil();
        }
        self.top = top;
    }

    /// Get current base index
    #[inline]
    pub fn base(&self) -> usize {
        self.base
    }

    /// Set base index
    #[inline]
    pub fn set_base(&mut self, base: usize) {
        self.base = base;
    }

    /// Number of values on stack above base
    #[inline]
    pub fn len(&self) -> usize {
        self.top.saturating_sub(self.base)
    }

    /// Check if stack is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Push a value onto the stack
    #[inline]
    pub fn push(&mut self, value: Value) -> LuaResult<()> {
        if self.top >= self.values.len() {
            if self.top >= MAX_STACK_SIZE {
                return Err(LuaError::StackOverflow);
            }
            self.grow(self.top + 1);
        }
        self.values[self.top] = value;
        self.top += 1;
        Ok(())
    }

    /// Pop a value from the stack
    #[inline]
    pub fn pop(&mut self) -> Value {
        if self.top > self.base {
            self.top -= 1;
            let val = self.values[self.top];
            self.values[self.top] = Value::nil();
            val
        } else {
            Value::nil()
        }
    }

    /// Pop n values from the stack
    pub fn pop_n(&mut self, n: usize) -> Vec<Value> {
        let mut result = Vec::with_capacity(n);
        for _ in 0..n {
            result.push(self.pop());
        }
        result.reverse();
        result
    }

    /// Get value at absolute index
    #[inline]
    pub fn get(&self, index: usize) -> Value {
        if index < self.values.len() {
            self.values[index]
        } else {
            Value::nil()
        }
    }

    /// Get value at index relative to base
    #[inline]
    pub fn get_rel(&self, offset: usize) -> Value {
        self.get(self.base + offset)
    }

    /// Set value at absolute index
    #[inline]
    pub fn set(&mut self, index: usize, value: Value) {
        if index >= self.values.len() {
            self.grow(index + 1);
        }
        self.values[index] = value;
        if index >= self.top {
            self.top = index + 1;
        }
    }

    /// Set value at index relative to base
    #[inline]
    pub fn set_rel(&mut self, offset: usize, value: Value) {
        self.set(self.base + offset, value);
    }

    /// Get a mutable pointer to a slot (for upvalue capture)
    #[inline]
    pub fn slot_ptr(&mut self, index: usize) -> *mut Value {
        if index >= self.values.len() {
            self.grow(index + 1);
        }
        &mut self.values[index] as *mut Value
    }

    /// Get slice of values from base to top
    pub fn frame_values(&self) -> &[Value] {
        &self.values[self.base..self.top]
    }

    /// Ensure stack has enough space
    pub fn ensure(&mut self, n: usize) -> LuaResult<()> {
        let needed = self.top + n;
        if needed > self.values.len() {
            if needed > MAX_STACK_SIZE {
                return Err(LuaError::StackOverflow);
            }
            self.grow(needed);
        }
        Ok(())
    }

    /// Grow the stack
    fn grow(&mut self, min_size: usize) {
        let new_size = (min_size * 2).min(MAX_STACK_SIZE);
        self.values.resize(new_size, Value::nil());
    }

    /// Copy values within the stack
    pub fn copy_range(&mut self, src: usize, dst: usize, count: usize) {
        if src < dst {
            for i in (0..count).rev() {
                self.values[dst + i] = self.values[src + i];
            }
        } else {
            for i in 0..count {
                self.values[dst + i] = self.values[src + i];
            }
        }
    }

    /// Clear values in range (set to nil)
    pub fn clear_range(&mut self, start: usize, end: usize) {
        for i in start..end.min(self.values.len()) {
            self.values[i] = Value::nil();
        }
    }

    /// Get the raw pointer to stack start (for C interop)
    pub fn as_ptr(&self) -> *const Value {
        self.values.as_ptr()
    }

    /// Get mutable raw pointer to stack start
    pub fn as_mut_ptr(&mut self) -> *mut Value {
        self.values.as_mut_ptr()
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Stack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stack")
            .field("base", &self.base)
            .field("top", &self.top)
            .field("len", &self.len())
            .field("values", &self.frame_values())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop() {
        let mut stack = Stack::new();

        stack.push(Value::integer(1)).unwrap();
        stack.push(Value::integer(2)).unwrap();
        stack.push(Value::integer(3)).unwrap();

        assert_eq!(stack.len(), 3);
        assert_eq!(stack.pop().as_integer(), Some(3));
        assert_eq!(stack.pop().as_integer(), Some(2));
        assert_eq!(stack.pop().as_integer(), Some(1));
        assert_eq!(stack.len(), 0);
    }

    #[test]
    fn test_get_set() {
        let mut stack = Stack::new();

        stack.set(5, Value::integer(42));
        assert_eq!(stack.get(5).as_integer(), Some(42));
        assert!(stack.get(4).is_nil());
    }

    #[test]
    fn test_frame() {
        let mut stack = Stack::new();

        stack.push(Value::integer(1)).unwrap();
        stack.push(Value::integer(2)).unwrap();
        stack.set_base(2);
        stack.push(Value::integer(3)).unwrap();

        assert_eq!(stack.len(), 1);
        assert_eq!(stack.get_rel(0).as_integer(), Some(3));
    }
}
