//! NaN-boxing implementation for Lua values.
//!
//! This implements efficient value representation using NaN-boxing, following
//! LuaJIT's approach. Non-number values are encoded as NaN bit patterns with
//! type tags in the upper bits.
//!
//! Layout (64-bit):
//! - Numbers: IEEE 754 double-precision float (raw bits, no transformation)
//! - Tagged values: upper 16 bits > 0xFFF8, lower 48 bits are payload
//!
//! IEEE 754 floats have upper 16 bits at most 0xFFF8 (negative quiet NaN).
//! Any value with upper 16 bits >= 0xFFF9 is a tagged non-number value.

use super::{LuaType, LuaError, LuaResult, GcRef, Table, LuaString, Function, Userdata};
use std::fmt;
use std::hash::{Hash, Hasher};
use ordered_float::OrderedFloat;

// ==================== Type Tags ====================
//
// Following LuaJIT's ordering (lj_obj.h):
// - Primitives (nil/false/true) have highest itypes
// - GC objects have lower itypes
// - Numbers have itype <= TAG_NUMBER_MAX
//
// We use upper 16 bits for type discrimination.
// All IEEE 754 floats have upper 16 bits <= 0xFFF8.
// Tags 0xFFF9-0xFFFF are safe for non-number values.

/// Maximum upper 16 bits for any IEEE 754 float (negative quiet NaN)
const TAG_NUMBER_MAX: u16 = 0xFFF8;

/// Type tag for nil (highest value, like LuaJIT's ~0u)
const TAG_NIL: u16 = 0xFFFF;

/// Type tag for false (like LuaJIT's ~1u)
const TAG_FALSE: u16 = 0xFFFE;

/// Type tag for true (like LuaJIT's ~2u)
const TAG_TRUE: u16 = 0xFFFD;

/// Type tag for light userdata (like LuaJIT's ~3u)
const TAG_LIGHTUD: u16 = 0xFFFC;

/// Type tag for strings (like LuaJIT's ~4u)
const TAG_STRING: u16 = 0xFFFB;

/// Type tag for tables (like LuaJIT's ~11u, but we compress into available range)
const TAG_TABLE: u16 = 0xFFFA;

/// Type tag for functions (like LuaJIT's ~8u)
const TAG_FUNCTION: u16 = 0xFFF9;

// For userdata/thread, we use TAG_FUNCTION with a marker bit in the payload
// Bit 47 (0x0000_8000_0000_0000) distinguishes:
// - Bit 47 = 0: Function
// - Bit 47 = 1: Userdata (or thread if we add thread support later)

/// Marker bit for userdata type in payload (bit 47)
const PAYLOAD_OTHER_BIT: u64 = 0x0000_8000_0000_0000;
/// Marker bit for thread - currently unused, reserved for future
const PAYLOAD_THREAD_BIT: u64 = 0x0000_4000_0000_0000;

/// Mask for 47-bit pointer payload (for userdata, preserves bits 0-46)
const PAYLOAD_MASK_47: u64 = 0x0000_7FFF_FFFF_FFFF;

/// Mask for 48-bit pointer payload
const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

// ==================== Helper Functions ====================

/// Extract the type tag (upper 16 bits)
#[inline]
const fn get_tag(bits: u64) -> u16 {
    (bits >> 48) as u16
}

/// Make a tagged value from tag and 48-bit payload
#[inline]
const fn make_tagged(tag: u16, payload: u64) -> u64 {
    ((tag as u64) << 48) | (payload & PAYLOAD_MASK)
}

/// Check if bits represent a number (tag <= TAG_NUMBER_MAX)
#[inline]
const fn is_number_bits(bits: u64) -> bool {
    get_tag(bits) <= TAG_NUMBER_MAX
}

/// Check if bits represent a tagged value (tag > TAG_NUMBER_MAX)
#[inline]
const fn is_tagged_bits(bits: u64) -> bool {
    get_tag(bits) > TAG_NUMBER_MAX
}

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
        Self { bits: make_tagged(TAG_NIL, 0) }
    }

    /// Create a boolean value
    #[inline]
    pub const fn boolean(b: bool) -> Self {
        Self {
            bits: make_tagged(if b { TAG_TRUE } else { TAG_FALSE }, 0),
        }
    }

    /// Create a number value (stored as raw IEEE 754 bits)
    #[inline]
    pub fn number(n: f64) -> Self {
        Self { bits: n.to_bits() }
    }

    /// Create an integer value (stored as a float)
    #[inline]
    pub fn integer(i: i32) -> Self {
        Self::number(i as f64)
    }

    /// Create a string value from a GC reference
    #[inline]
    pub fn string(s: GcRef<LuaString>) -> Self {
        Self {
            bits: make_tagged(TAG_STRING, s.as_ptr() as u64),
        }
    }

    /// Create a table value from a GC reference
    #[inline]
    pub fn table(t: GcRef<Table>) -> Self {
        Self {
            bits: make_tagged(TAG_TABLE, t.as_ptr() as u64),
        }
    }

    /// Create a function value from a GC reference
    #[inline]
    pub fn function(f: GcRef<Function>) -> Self {
        Self {
            bits: make_tagged(TAG_FUNCTION, f.as_ptr() as u64),
        }
    }

    /// Create a userdata value from a GC reference
    #[inline]
    pub fn userdata(u: GcRef<Userdata>) -> Self {
        // Use TAG_FUNCTION with PAYLOAD_OTHER_BIT set
        // Use 47-bit pointer mask to preserve full Linux user-space addresses
        let ptr = u.as_ptr() as u64 & PAYLOAD_MASK_47;
        Self {
            bits: make_tagged(TAG_FUNCTION, PAYLOAD_OTHER_BIT | ptr),
        }
    }

    /// Create a light userdata value (raw pointer)
    #[inline]
    pub fn light_userdata(ptr: *mut ()) -> Self {
        Self {
            bits: make_tagged(TAG_LIGHTUD, ptr as u64),
        }
    }

    // ==================== Type Checking ====================

    /// Check if value is a number
    #[inline]
    pub fn is_number(&self) -> bool {
        is_number_bits(self.bits)
    }

    /// Check if value is a float (same as is_number)
    #[inline]
    pub fn is_float(&self) -> bool {
        self.is_number()
    }

    /// Check if value is an integer (float that's a whole number)
    #[inline]
    pub fn is_integer(&self) -> bool {
        if !self.is_number() {
            return false;
        }
        let n = f64::from_bits(self.bits);
        n.is_finite() && n == (n as i32 as f64)
    }

    /// Check if value is nil
    #[inline]
    pub fn is_nil(&self) -> bool {
        get_tag(self.bits) == TAG_NIL
    }

    /// Check if value is a boolean
    #[inline]
    pub fn is_boolean(&self) -> bool {
        let tag = get_tag(self.bits);
        tag == TAG_FALSE || tag == TAG_TRUE
    }

    /// Check if value is a string
    #[inline]
    pub fn is_string(&self) -> bool {
        get_tag(self.bits) == TAG_STRING
    }

    /// Check if value is a table
    #[inline]
    pub fn is_table(&self) -> bool {
        get_tag(self.bits) == TAG_TABLE
    }

    /// Check if value is a function
    #[inline]
    pub fn is_function(&self) -> bool {
        get_tag(self.bits) == TAG_FUNCTION && (self.bits & PAYLOAD_OTHER_BIT) == 0
    }

    /// Check if value is userdata
    /// Note: With 47-bit pointer support, userdata is distinguished by PAYLOAD_OTHER_BIT only.
    /// Thread support requires a different encoding (not currently implemented).
    #[inline]
    pub fn is_userdata(&self) -> bool {
        get_tag(self.bits) == TAG_FUNCTION && (self.bits & PAYLOAD_OTHER_BIT) != 0
    }

    /// Check if value is a thread
    /// Note: Thread type is not currently supported with 47-bit pointer encoding.
    /// This always returns false for now.
    #[inline]
    pub fn is_thread(&self) -> bool {
        false  // Thread support disabled with 47-bit pointer encoding
    }

    /// Check if value is light userdata
    #[inline]
    pub fn is_light_userdata(&self) -> bool {
        get_tag(self.bits) == TAG_LIGHTUD
    }

    // ==================== Value Extraction ====================

    /// Get the Lua type of this value
    #[inline]
    pub fn lua_type(&self) -> LuaType {
        let tag = get_tag(self.bits);

        // Numbers have tag <= TAG_NUMBER_MAX
        if tag <= TAG_NUMBER_MAX {
            return LuaType::Number;
        }

        match tag {
            TAG_NIL => LuaType::Nil,
            TAG_FALSE | TAG_TRUE => LuaType::Boolean,
            TAG_LIGHTUD => LuaType::LightUserdata,
            TAG_STRING => LuaType::String,
            TAG_TABLE => LuaType::Table,
            TAG_FUNCTION => {
                // Check marker bit for function vs userdata
                // (Thread support disabled with 47-bit pointer encoding)
                if (self.bits & PAYLOAD_OTHER_BIT) == 0 {
                    LuaType::Function
                } else {
                    LuaType::Userdata
                }
            }
            _ => LuaType::Nil, // Shouldn't happen
        }
    }

    /// Get as a number (float or integer converted to float)
    #[inline]
    pub fn as_number(&self) -> Option<f64> {
        if self.is_number() {
            Some(f64::from_bits(self.bits))
        } else {
            None
        }
    }

    /// Coerce value to number (including string-to-number conversion)
    /// This is used for arithmetic operations and for loops in Lua
    pub fn coerce_to_number(&self) -> Option<f64> {
        // First try direct number conversion
        if let Some(n) = self.as_number() {
            return Some(n);
        }

        // Try string-to-number conversion
        if let Some(s) = self.as_string() {
            let s = unsafe { &*s.as_ptr() };
            if let Some(text) = s.as_str() {
                let text = text.trim();

                // Handle hex numbers
                if text.starts_with("0x") || text.starts_with("0X") {
                    if let Ok(n) = i64::from_str_radix(&text[2..], 16) {
                        return Some(n as f64);
                    }
                }

                // Parse as float
                if let Ok(n) = text.parse::<f64>() {
                    return Some(n);
                }
            }
        }

        None
    }

    /// Get as an integer
    #[inline]
    pub fn as_integer(&self) -> Option<i32> {
        if self.is_number() {
            let n = f64::from_bits(self.bits);
            if n.is_finite() {
                let i = n as i32;
                if (i as f64) == n {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Coerce value to integer (including string-to-number conversion)
    pub fn coerce_to_integer(&self) -> Option<i32> {
        if let Some(n) = self.coerce_to_number() {
            if n.is_finite() {
                let i = n as i32;
                if (i as f64) == n {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Get as a boolean
    #[inline]
    pub fn as_boolean(&self) -> Option<bool> {
        match get_tag(self.bits) {
            TAG_TRUE => Some(true),
            TAG_FALSE => Some(false),
            _ => None,
        }
    }

    /// Get pointer payload
    #[inline]
    fn get_pointer<T>(&self) -> *mut T {
        (self.bits & PAYLOAD_MASK) as *mut T
    }

    /// Get pointer payload with 47-bit mask (for userdata)
    #[inline]
    fn get_pointer_47<T>(&self) -> *mut T {
        (self.bits & PAYLOAD_MASK_47) as *mut T
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
            Some(GcRef::new(self.get_pointer_47()))
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
    /// Following LuaJIT: anything except nil and false is truthy
    #[inline]
    pub fn is_truthy(&self) -> bool {
        let tag = get_tag(self.bits);
        // Everything is truthy except nil (0xFFFF) and false (0xFFFE)
        tag < TAG_FALSE
    }

    /// Check if value is falsy (nil or false)
    #[inline]
    pub fn is_falsy(&self) -> bool {
        let tag = get_tag(self.bits);
        tag >= TAG_FALSE  // nil (0xFFFF) and false (0xFFFE)
    }

    // ==================== Arithmetic Operations ====================

    /// Add two values (with string coercion)
    pub fn add(&self, other: &Value) -> LuaResult<Value> {
        match (self.coerce_to_number(), other.coerce_to_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a + b)),
            _ => {
                let ty = if self.coerce_to_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Subtract two values (with string coercion)
    pub fn sub(&self, other: &Value) -> LuaResult<Value> {
        match (self.coerce_to_number(), other.coerce_to_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a - b)),
            _ => {
                let ty = if self.coerce_to_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Multiply two values (with string coercion)
    pub fn mul(&self, other: &Value) -> LuaResult<Value> {
        match (self.coerce_to_number(), other.coerce_to_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a * b)),
            _ => {
                let ty = if self.coerce_to_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Divide two values (with string coercion)
    pub fn div(&self, other: &Value) -> LuaResult<Value> {
        match (self.coerce_to_number(), other.coerce_to_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a / b)),
            _ => {
                let ty = if self.coerce_to_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Modulo operation (with string coercion)
    pub fn modulo(&self, other: &Value) -> LuaResult<Value> {
        match (self.coerce_to_number(), other.coerce_to_number()) {
            (Some(a), Some(b)) => {
                // Lua's modulo: a - floor(a/b)*b
                let result = a - (a / b).floor() * b;
                Ok(Value::number(result))
            }
            _ => {
                let ty = if self.coerce_to_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Power operation (with string coercion)
    pub fn pow(&self, other: &Value) -> LuaResult<Value> {
        match (self.coerce_to_number(), other.coerce_to_number()) {
            (Some(a), Some(b)) => Ok(Value::number(a.powf(b))),
            _ => {
                let ty = if self.coerce_to_number().is_none() {
                    self.lua_type()
                } else {
                    other.lua_type()
                };
                Err(LuaError::ArithmeticError(ty))
            }
        }
    }

    /// Unary minus (with string coercion)
    pub fn unm(&self) -> LuaResult<Value> {
        match self.coerce_to_number() {
            Some(n) => Ok(Value::number(-n)),
            None => Err(LuaError::ArithmeticError(self.lua_type())),
        }
    }

    /// Integer division (Lua 5.3+) (with string coercion)
    pub fn idiv(&self, other: &Value) -> LuaResult<Value> {
        match (self.coerce_to_number(), other.coerce_to_number()) {
            (Some(a), Some(b)) => Ok(Value::number((a / b).floor())),
            _ => {
                let ty = if self.coerce_to_number().is_none() {
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
        // For numbers, use f64 comparison which properly handles NaN (NaN != NaN)
        if self.is_number() && other.is_number() {
            let a = f64::from_bits(self.bits);
            let b = f64::from_bits(other.bits);
            return a == b;
        }

        // For non-numbers, bit equality is sufficient
        self.bits == other.bits
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
            LuaType::Userdata => write!(f, "userdata: {:p}", self.get_pointer_47::<Userdata>()),
            LuaType::LightUserdata => write!(f, "userdata: {:p}", self.get_pointer::<()>()),
            LuaType::Thread => write!(f, "thread: {:p}", self.get_pointer_47::<()>()),
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
    fn test_special_floats() {
        // Infinity should be a number
        let inf = Value::number(f64::INFINITY);
        assert!(inf.is_number());
        assert_eq!(inf.lua_type(), LuaType::Number);
        assert_eq!(inf.as_number(), Some(f64::INFINITY));

        let neg_inf = Value::number(f64::NEG_INFINITY);
        assert!(neg_inf.is_number());
        assert_eq!(neg_inf.lua_type(), LuaType::Number);
        assert_eq!(neg_inf.as_number(), Some(f64::NEG_INFINITY));

        // NaN should be a number
        let nan = Value::number(f64::NAN);
        assert!(nan.is_number());
        assert_eq!(nan.lua_type(), LuaType::Number);
        assert!(nan.as_number().unwrap().is_nan());
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
    fn test_division_by_zero() {
        let one = Value::number(1.0);
        let zero = Value::number(0.0);

        // Division by zero should produce infinity, not nil
        let result = one.div(&zero).unwrap();
        assert!(result.is_number());
        assert_eq!(result.as_number(), Some(f64::INFINITY));
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

    #[test]
    fn test_truthiness() {
        // Numbers are truthy
        assert!(Value::number(0.0).is_truthy());
        assert!(Value::number(1.0).is_truthy());
        assert!(Value::number(f64::INFINITY).is_truthy());

        // Only nil and false are falsy
        assert!(Value::nil().is_falsy());
        assert!(Value::boolean(false).is_falsy());
        assert!(Value::boolean(true).is_truthy());
    }
}
