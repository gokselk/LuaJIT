//! NaN-boxing implementation for Lua values.
//!
//! This implements efficient value representation using NaN-boxing, where
//! non-number values are encoded as NaN bit patterns with type tags.
//!
//! Layout (64-bit):
//! - Numbers: IEEE 754 double-precision float (NaN excluded for tagging)
//! - Other types: 0xFFF8_XXXX_PPPP_PPPP where XXXX is the type tag and P is payload

use super::{LuaType, LuaError, LuaResult, GcRef, Table, LuaString, Function, Userdata};
use std::fmt;
use std::hash::{Hash, Hasher};
use ordered_float::OrderedFloat;

/// Tag bits for NaN-boxing (upper 16 bits of NaN pattern)
const NAN_TAG_MASK: u64 = 0xFFFF_0000_0000_0000;
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Quiet NaN with tag space
const QNAN_BASE: u64 = 0x7FF8_0000_0000_0000;

/// Type tags (added to QNAN_BASE)
const TAG_NIL: u64      = 0x0001_0000_0000_0000;
const TAG_FALSE: u64    = 0x0002_0000_0000_0000;
const TAG_TRUE: u64     = 0x0003_0000_0000_0000;
const TAG_LIGHTUD: u64  = 0x0004_0000_0000_0000;
const TAG_STRING: u64   = 0x0005_0000_0000_0000;
const TAG_TABLE: u64    = 0x0006_0000_0000_0000;
const TAG_FUNCTION: u64 = 0x0007_0000_0000_0000;
const TAG_USERDATA: u64 = 0x0008_0000_0000_0000;
const TAG_THREAD: u64   = 0x0009_0000_0000_0000;
const TAG_INTEGER: u64  = 0x000A_0000_0000_0000;

/// A Lua value using NaN-boxing representation.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Value {
    pub(crate) bits: u64,
}

impl Value {
    // ==================== Constructors ====================

    /// Create a nil value
    #[inline]
    pub const fn nil() -> Self {
        Self { bits: QNAN_BASE | TAG_NIL }
    }

    /// Create a boolean value
    #[inline]
    pub const fn boolean(b: bool) -> Self {
        Self {
            bits: QNAN_BASE | if b { TAG_TRUE } else { TAG_FALSE },
        }
    }

    /// Create a number value
    #[inline]
    pub fn number(n: f64) -> Self {
        Self { bits: n.to_bits() }
    }

    /// Create an integer value
    #[inline]
    pub fn integer(i: i32) -> Self {
        Self {
            bits: QNAN_BASE | TAG_INTEGER | (i as u32 as u64),
        }
    }

    /// Create a string value from a GC reference
    #[inline]
    pub fn string(s: GcRef<LuaString>) -> Self {
        Self {
            bits: QNAN_BASE | TAG_STRING | (s.as_ptr() as u64 & PAYLOAD_MASK),
        }
    }

    /// Create a table value from a GC reference
    #[inline]
    pub fn table(t: GcRef<Table>) -> Self {
        Self {
            bits: QNAN_BASE | TAG_TABLE | (t.as_ptr() as u64 & PAYLOAD_MASK),
        }
    }

    /// Create a function value from a GC reference
    #[inline]
    pub fn function(f: GcRef<Function>) -> Self {
        Self {
            bits: QNAN_BASE | TAG_FUNCTION | (f.as_ptr() as u64 & PAYLOAD_MASK),
        }
    }

    /// Create a userdata value from a GC reference
    #[inline]
    pub fn userdata(u: GcRef<Userdata>) -> Self {
        Self {
            bits: QNAN_BASE | TAG_USERDATA | (u.as_ptr() as u64 & PAYLOAD_MASK),
        }
    }

    /// Create a light userdata value (raw pointer)
    #[inline]
    pub fn light_userdata(ptr: *mut ()) -> Self {
        Self {
            bits: QNAN_BASE | TAG_LIGHTUD | (ptr as u64 & PAYLOAD_MASK),
        }
    }

    // ==================== Type Checking ====================

    /// Check if value is a number (float or integer)
    #[inline]
    pub fn is_number(&self) -> bool {
        self.is_float() || self.is_integer()
    }

    /// Check if value is a float
    #[inline]
    pub fn is_float(&self) -> bool {
        // A value is a float if it's NOT a tagged value
        // (not a quiet NaN with our tag pattern)
        (self.bits & QNAN_BASE) != QNAN_BASE
    }

    /// Check if value is an integer
    #[inline]
    pub fn is_integer(&self) -> bool {
        (self.bits & (QNAN_BASE | 0xFFFF_0000_0000_0000)) == (QNAN_BASE | TAG_INTEGER)
    }

    /// Check if value is nil
    #[inline]
    pub fn is_nil(&self) -> bool {
        self.bits == (QNAN_BASE | TAG_NIL)
    }

    /// Check if value is a boolean
    #[inline]
    pub fn is_boolean(&self) -> bool {
        let tag = self.bits & !1; // Ignore the true/false bit
        tag == (QNAN_BASE | TAG_FALSE)
    }

    /// Check if value is a string
    #[inline]
    pub fn is_string(&self) -> bool {
        (self.bits & (QNAN_BASE | 0xFFFF_0000_0000_0000)) == (QNAN_BASE | TAG_STRING)
    }

    /// Check if value is a table
    #[inline]
    pub fn is_table(&self) -> bool {
        (self.bits & (QNAN_BASE | 0xFFFF_0000_0000_0000)) == (QNAN_BASE | TAG_TABLE)
    }

    /// Check if value is a function
    #[inline]
    pub fn is_function(&self) -> bool {
        (self.bits & (QNAN_BASE | 0xFFFF_0000_0000_0000)) == (QNAN_BASE | TAG_FUNCTION)
    }

    /// Check if value is userdata
    #[inline]
    pub fn is_userdata(&self) -> bool {
        (self.bits & (QNAN_BASE | 0xFFFF_0000_0000_0000)) == (QNAN_BASE | TAG_USERDATA)
    }

    /// Check if value is light userdata
    #[inline]
    pub fn is_light_userdata(&self) -> bool {
        (self.bits & (QNAN_BASE | 0xFFFF_0000_0000_0000)) == (QNAN_BASE | TAG_LIGHTUD)
    }

    // ==================== Value Extraction ====================

    /// Get the Lua type of this value
    #[inline]
    pub fn lua_type(&self) -> LuaType {
        if self.is_float() {
            LuaType::Number
        } else {
            let tag = (self.bits >> 48) & 0xFFFF;
            match tag & 0x000F {
                0x1 => LuaType::Nil,
                0x2 | 0x3 => LuaType::Boolean,
                0x4 => LuaType::LightUserdata,
                0x5 => LuaType::String,
                0x6 => LuaType::Table,
                0x7 => LuaType::Function,
                0x8 => LuaType::Userdata,
                0x9 => LuaType::Thread,
                0xA => LuaType::Number, // Integer
                _ => LuaType::Nil,
            }
        }
    }

    /// Get as a number (float or integer converted to float)
    #[inline]
    pub fn as_number(&self) -> Option<f64> {
        if self.is_float() {
            Some(f64::from_bits(self.bits))
        } else if self.is_integer() {
            Some((self.bits as u32 as i32) as f64)
        } else {
            None
        }
    }

    /// Get as an integer
    #[inline]
    pub fn as_integer(&self) -> Option<i32> {
        if self.is_integer() {
            Some(self.bits as u32 as i32)
        } else if self.is_float() {
            let n = f64::from_bits(self.bits);
            let i = n as i32;
            if (i as f64) == n {
                Some(i)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Get as a boolean
    #[inline]
    pub fn as_boolean(&self) -> Option<bool> {
        if self.bits == (QNAN_BASE | TAG_TRUE) {
            Some(true)
        } else if self.bits == (QNAN_BASE | TAG_FALSE) {
            Some(false)
        } else {
            None
        }
    }

    /// Get pointer payload
    #[inline]
    fn get_pointer<T>(&self) -> *mut T {
        (self.bits & PAYLOAD_MASK) as *mut T
    }

    /// Get as a string reference
    #[inline]
    pub fn as_string(&self) -> Option<GcRef<LuaString>> {
        if self.is_string() {
            Some(GcRef::new(self.get_pointer()))
        } else {
            None
        }
    }

    /// Get as a table reference
    #[inline]
    pub fn as_table(&self) -> Option<GcRef<Table>> {
        if self.is_table() {
            Some(GcRef::new(self.get_pointer()))
        } else {
            None
        }
    }

    /// Get as a function reference
    #[inline]
    pub fn as_function(&self) -> Option<GcRef<Function>> {
        if self.is_function() {
            Some(GcRef::new(self.get_pointer()))
        } else {
            None
        }
    }

    /// Get as userdata reference
    #[inline]
    pub fn as_userdata(&self) -> Option<GcRef<Userdata>> {
        if self.is_userdata() {
            Some(GcRef::new(self.get_pointer()))
        } else {
            None
        }
    }

    /// Get as light userdata (raw pointer)
    #[inline]
    pub fn as_light_userdata(&self) -> Option<*mut ()> {
        if self.is_light_userdata() {
            Some(self.get_pointer())
        } else {
            None
        }
    }

    // ==================== Truthiness ====================

    /// Check if value is truthy (not nil and not false)
    #[inline]
    pub fn is_truthy(&self) -> bool {
        !self.is_nil() && self.bits != (QNAN_BASE | TAG_FALSE)
    }

    /// Check if value is falsy (nil or false)
    #[inline]
    pub fn is_falsy(&self) -> bool {
        self.is_nil() || self.bits == (QNAN_BASE | TAG_FALSE)
    }

    // ==================== Arithmetic Operations ====================

    /// Add two values
    pub fn add(&self, other: &Value) -> LuaResult<Value> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a + b)),
            _ => {
                let ty = if self.as_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Subtract two values
    pub fn sub(&self, other: &Value) -> LuaResult<Value> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a - b)),
            _ => {
                let ty = if self.as_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Multiply two values
    pub fn mul(&self, other: &Value) -> LuaResult<Value> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a * b)),
            _ => {
                let ty = if self.as_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Divide two values
    pub fn div(&self, other: &Value) -> LuaResult<Value> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a / b)),
            _ => {
                let ty = if self.as_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Modulo operation
    pub fn modulo(&self, other: &Value) -> LuaResult<Value> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => {
                // Lua's modulo: a - floor(a/b)*b
                let result = a - (a / b).floor() * b;
                Ok(Value::number(result))
            }
            _ => {
                let ty = if self.as_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Power operation
    pub fn pow(&self, other: &Value) -> LuaResult<Value> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a.powf(b))),
            _ => {
                let ty = if self.as_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Unary minus
    pub fn unm(&self) -> LuaResult<Value> {
        match self.as_number() {
            Some(n) => Ok(Value::number(-n)),
            None => Err(LuaError::ArithmeticError(self.lua_type())),
        }
    }

    /// Integer division (Lua 5.3+)
    pub fn idiv(&self, other: &Value) -> LuaResult<Value> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => Ok(Value::number((a / b).floor())),
            _ => {
                let ty = if self.as_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    // ==================== Comparison Operations ====================

    /// Less than comparison
    pub fn lt(&self, other: &Value) -> LuaResult<bool> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => Ok(a < b),
            _ => Err(LuaError::CompareError(self.lua_type(), other.lua_type())),
        }
    }

    /// Less than or equal comparison
    pub fn le(&self, other: &Value) -> LuaResult<bool> {
        match (self.as_number(), other.as_number()) {
            (Some(a), Some(b)) => Ok(a <= b),
            _ => Err(LuaError::CompareError(self.lua_type(), other.lua_type())),
        }
    }

    /// Equality comparison (raw equality, no metamethods)
    pub fn raw_eq(&self, other: &Value) -> bool {
        if self.bits == other.bits {
            return true;
        }

        // Handle NaN: NaN != NaN
        if self.is_float() && other.is_float() {
            let a = f64::from_bits(self.bits);
            let b = f64::from_bits(other.bits);
            return a == b;
        }

        // Handle integer vs float comparison
        if let (Some(a), Some(b)) = (self.as_number(), other.as_number()) {
            return a == b;
        }

        false
    }

    /// Get the raw bits (for debugging/serialization)
    #[inline]
    pub fn raw_bits(&self) -> u64 {
        self.bits
    }
}

impl Default for Value {
    fn default() -> Self {
        Self::nil()
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.raw_eq(other)
    }
}

impl Eq for Value {}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // For numbers, we need to handle the case where integer and float
        // representations of the same value should hash the same
        if let Some(n) = self.as_number() {
            let i = n as i64;
            if (i as f64) == n {
                i.hash(state);
            } else {
                OrderedFloat(n).hash(state);
            }
        } else {
            self.bits.hash(state);
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.lua_type() {
            LuaType::Nil => write!(f, "nil"),
            LuaType::Boolean => write!(f, "{}", self.as_boolean().unwrap()),
            LuaType::Number => {
                if let Some(n) = self.as_number() {
                    if let Some(i) = self.as_integer() {
                        write!(f, "{}", i)
                    } else {
                        write!(f, "{}", n)
                    }
                } else {
                    write!(f, "NaN")
                }
            }
            LuaType::String => write!(f, "string: {:p}", self.get_pointer::<LuaString>()),
            LuaType::Table => write!(f, "table: {:p}", self.get_pointer::<Table>()),
            LuaType::Function => write!(f, "function: {:p}", self.get_pointer::<Function>()),
            LuaType::Userdata => write!(f, "userdata: {:p}", self.get_pointer::<Userdata>()),
            LuaType::LightUserdata => write!(f, "userdata: {:p}", self.get_pointer::<()>()),
            LuaType::Thread => write!(f, "thread: {:p}", self.get_pointer::<()>()),
            _ => write!(f, "unknown"),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.lua_type() {
            LuaType::Nil => write!(f, "nil"),
            LuaType::Boolean => write!(f, "{}", self.as_boolean().unwrap()),
            LuaType::Number => {
                if let Some(n) = self.as_number() {
                    // Use Lua-style number formatting
                    if n.is_infinite() {
                        if n.is_sign_positive() {
                            write!(f, "inf")
                        } else {
                            write!(f, "-inf")
                        }
                    } else if n.is_nan() {
                        write!(f, "nan")
                    } else if let Some(i) = self.as_integer() {
                        write!(f, "{}", i)
                    } else {
                        // Use ryu for fast, accurate float formatting
                        let mut buffer = ryu::Buffer::new();
                        write!(f, "{}", buffer.format(n))
                    }
                } else {
                    write!(f, "nan")
                }
            }
            LuaType::String => write!(f, "string: {:p}", self.get_pointer::<LuaString>()),
            LuaType::Table => write!(f, "table: {:p}", self.get_pointer::<Table>()),
            LuaType::Function => write!(f, "function: {:p}", self.get_pointer::<Function>()),
            LuaType::Userdata | LuaType::LightUserdata => {
                write!(f, "userdata: {:p}", self.get_pointer::<()>())
            }
            LuaType::Thread => write!(f, "thread: {:p}", self.get_pointer::<()>()),
            _ => write!(f, "unknown"),
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::boolean(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::number(n)
    }
}

impl From<i32> for Value {
    fn from(i: i32) -> Self {
        Value::integer(i)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
            Value::integer(i as i32)
        } else {
            Value::number(i as f64)
        }
    }
}

impl From<()> for Value {
    fn from(_: ()) -> Self {
        Value::nil()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nil() {
        let v = Value::nil();
        assert!(v.is_nil());
        assert!(!v.is_truthy());
        assert!(v.is_falsy());
        assert_eq!(v.lua_type(), LuaType::Nil);
    }

    #[test]
    fn test_boolean() {
        let t = Value::boolean(true);
        let f = Value::boolean(false);

        assert!(t.is_boolean());
        assert!(f.is_boolean());
        assert!(t.is_truthy());
        assert!(f.is_falsy());
        assert_eq!(t.as_boolean(), Some(true));
        assert_eq!(f.as_boolean(), Some(false));
    }

    #[test]
    fn test_number() {
        let n = Value::number(3.14);
        assert!(n.is_number());
        assert!(n.is_float());
        assert!(!n.is_integer());
        assert_eq!(n.as_number(), Some(3.14));

        let i = Value::integer(42);
        assert!(i.is_number());
        assert!(i.is_integer());
        assert_eq!(i.as_integer(), Some(42));
        assert_eq!(i.as_number(), Some(42.0));
    }

    #[test]
    fn test_arithmetic() {
        let a = Value::number(10.0);
        let b = Value::number(3.0);

        assert_eq!(a.add(&b).unwrap().as_number(), Some(13.0));
        assert_eq!(a.sub(&b).unwrap().as_number(), Some(7.0));
        assert_eq!(a.mul(&b).unwrap().as_number(), Some(30.0));
        assert!((a.div(&b).unwrap().as_number().unwrap() - 3.333333).abs() < 0.001);
        assert_eq!(a.modulo(&b).unwrap().as_number(), Some(1.0));
    }

    #[test]
    fn test_equality() {
        let a = Value::number(42.0);
        let b = Value::integer(42);
        assert!(a.raw_eq(&b));

        let nil1 = Value::nil();
        let nil2 = Value::nil();
        assert!(nil1.raw_eq(&nil2));
    }
}
