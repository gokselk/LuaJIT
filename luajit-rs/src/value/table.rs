//! Lua table implementation.
//!
//! Tables in Lua are the primary data structure, serving as arrays, dictionaries,
//! objects, and more. This implementation uses a hybrid approach with an array
//! part for integer keys and a hash part for other keys.

use super::{Value, GcRef, LuaString};
use hashbrown::HashMap;
use smallvec::SmallVec;
use std::cell::{Cell, RefCell};

/// GC header for garbage-collected objects
#[derive(Debug)]
pub struct GcHeader {
    /// Marked flag for GC
    pub marked: Cell<u8>,
    /// GC type tag
    pub gct: u8,
}

impl GcHeader {
    pub fn new(gct: u8) -> Self {
        Self {
            marked: Cell::new(0),
            gct,
        }
    }
}

/// A Lua table with array and hash parts.
pub struct Table {
    /// GC header
    pub gc: GcHeader,
    /// Array part for integer indices 1..=array.len()
    array: RefCell<Vec<Value>>,
    /// Hash part for non-integer keys or sparse integer keys
    hash: RefCell<HashMap<TableKey, Value, rustc_hash::FxBuildHasher>>,
    /// Metatable (if any)
    metatable: Cell<Option<GcRef<Table>>>,
    /// Cached array length (for # operator optimization)
    cached_len: Cell<Option<usize>>,
}

/// A key for the hash part of a table.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum TableKey {
    Integer(i64),
    Float(ordered_float::OrderedFloat<f64>),
    String(u64), // String pointer as key
    Boolean(bool),
    Pointer(u64), // For table/function/userdata keys
}

impl TableKey {
    pub fn from_value(value: &Value) -> Option<Self> {
        if value.is_nil() {
            return None; // nil cannot be a table key
        }

        if let Some(i) = value.as_integer() {
            return Some(TableKey::Integer(i as i64));
        }

        if let Some(n) = value.as_number() {
            if n.is_nan() {
                return None; // NaN cannot be a table key
            }
            // Check if it's an integer value
            let i = n as i64;
            if (i as f64) == n {
                return Some(TableKey::Integer(i));
            }
            return Some(TableKey::Float(ordered_float::OrderedFloat(n)));
        }

        if let Some(b) = value.as_boolean() {
            return Some(TableKey::Boolean(b));
        }

        if value.is_string() {
            return Some(TableKey::String(value.raw_bits()));
        }

        if value.is_table() || value.is_function() || value.is_userdata() {
            return Some(TableKey::Pointer(value.raw_bits()));
        }

        None
    }
}

impl Table {
    /// Create a new empty table
    pub fn new() -> Self {
        Self {
            gc: GcHeader::new(5), // LuaType::Table
            array: RefCell::new(Vec::new()),
            hash: RefCell::new(HashMap::with_hasher(rustc_hash::FxBuildHasher)),
            metatable: Cell::new(None),
            cached_len: Cell::new(Some(0)),
        }
    }

    /// Create a new table with preallocated capacity
    pub fn with_capacity(array_size: usize, hash_size: usize) -> Self {
        Self {
            gc: GcHeader::new(5),
            array: RefCell::new(Vec::with_capacity(array_size)),
            hash: RefCell::new(HashMap::with_capacity_and_hasher(
                hash_size,
                rustc_hash::FxBuildHasher,
            )),
            metatable: Cell::new(None),
            cached_len: Cell::new(Some(0)),
        }
    }

    /// Get a value from the table
    pub fn get(&self, key: &Value) -> Value {
        // Fast path for integer keys in array range
        if let Some(i) = key.as_integer() {
            if i >= 1 {
                let array = self.array.borrow();
                let idx = (i - 1) as usize;
                if idx < array.len() {
                    let v = array[idx];
                    if !v.is_nil() {
                        return v;
                    }
                }
            }
        }

        // Fall back to hash lookup
        if let Some(table_key) = TableKey::from_value(key) {
            self.hash.borrow().get(&table_key).copied().unwrap_or(Value::nil())
        } else {
            Value::nil()
        }
    }

    /// Set a value in the table
    pub fn set(&self, key: Value, value: Value) {
        // Invalidate cached length
        self.cached_len.set(None);

        // Fast path for integer keys
        if let Some(i) = key.as_integer() {
            if i >= 1 {
                let idx = (i - 1) as usize;
                let mut array = self.array.borrow_mut();

                // If index is within reasonable array bounds, use array part
                if idx < array.len() {
                    array[idx] = value;
                    return;
                } else if idx == array.len() && idx < 1024 * 1024 {
                    // Extend array for consecutive indices
                    array.push(value);
                    return;
                } else if idx < array.len() + 16 && idx < 1024 * 1024 {
                    // Small gap, fill with nils and add
                    array.resize(idx + 1, Value::nil());
                    array[idx] = value;
                    return;
                }
            }
        }

        // Use hash part
        if let Some(table_key) = TableKey::from_value(&key) {
            let mut hash = self.hash.borrow_mut();
            if value.is_nil() {
                hash.remove(&table_key);
            } else {
                hash.insert(table_key, value);
            }
        }
        // Setting nil key is a no-op (and error in real Lua)
    }

    /// Raw get (no metamethods)
    pub fn raw_get(&self, key: &Value) -> Value {
        self.get(key)
    }

    /// Raw set (no metamethods)
    pub fn raw_set(&self, key: Value, value: Value) {
        self.set(key, value);
    }

    /// Get array element by index (1-based)
    pub fn get_array(&self, index: usize) -> Value {
        let array = self.array.borrow();
        if index >= 1 && index <= array.len() {
            array[index - 1]
        } else {
            Value::nil()
        }
    }

    /// Set array element by index (1-based)
    pub fn set_array(&self, index: usize, value: Value) {
        if index >= 1 {
            self.cached_len.set(None);
            let mut array = self.array.borrow_mut();
            let idx = index - 1;
            if idx < array.len() {
                array[idx] = value;
            } else if idx == array.len() {
                array.push(value);
            } else {
                // Need to use hash part or extend array
                drop(array);
                self.set(Value::integer(index as i32), value);
            }
        }
    }

    /// Get the length of the table (# operator)
    pub fn len(&self) -> usize {
        // Return cached length if available
        if let Some(len) = self.cached_len.get() {
            return len;
        }

        // Calculate length
        let array = self.array.borrow();

        // Find the boundary: an index where t[i] is not nil and t[i+1] is nil
        let mut len = 0;
        for (i, v) in array.iter().enumerate() {
            if v.is_nil() {
                break;
            }
            len = i + 1;
        }

        // Cache the result
        self.cached_len.set(Some(len));
        len
    }

    /// Get metatable
    pub fn get_metatable(&self) -> Option<GcRef<Table>> {
        self.metatable.get()
    }

    /// Set metatable
    pub fn set_metatable(&self, mt: Option<GcRef<Table>>) {
        self.metatable.set(mt);
    }

    /// Check if table is empty
    pub fn is_empty(&self) -> bool {
        self.array.borrow().iter().all(|v| v.is_nil()) && self.hash.borrow().is_empty()
    }

    /// Get the next key-value pair after the given key (for pairs())
    pub fn next(&self, key: &Value) -> Option<(Value, Value)> {
        self.next_checked(key).ok().flatten()
    }

    /// Like next(), but returns Err(()) if the key doesn't exist in the table
    /// Ok(Some(k, v)) - found next key-value pair
    /// Ok(None) - key exists but was the last element (or nil key with empty table)
    /// Err(()) - key doesn't exist in the table
    pub fn next_checked(&self, key: &Value) -> Result<Option<(Value, Value)>, ()> {
        if key.is_nil() {
            // Start iteration
            let array = self.array.borrow();
            for (i, v) in array.iter().enumerate() {
                if !v.is_nil() {
                    return Ok(Some((Value::integer((i + 1) as i32), *v)));
                }
            }
            // Check hash part
            let hash = self.hash.borrow();
            if let Some((k, v)) = hash.iter().next() {
                return Ok(Some((self.key_to_value(k), *v)));
            }
            return Ok(None);
        }

        // Continue iteration from the given key
        if let Some(i) = key.as_integer() {
            if i >= 1 {
                let idx = i as usize;
                let array = self.array.borrow();
                // First check if key exists in array (1-based index)
                let key_idx = (i - 1) as usize;
                if key_idx < array.len() && !array[key_idx].is_nil() {
                    // Key exists in array, look for next non-nil
                    for j in idx..array.len() {
                        if !array[j].is_nil() {
                            return Ok(Some((Value::integer((j + 1) as i32), array[j])));
                        }
                    }
                    // Move to hash part
                    drop(array);
                    let hash = self.hash.borrow();
                    if let Some((k, v)) = hash.iter().next() {
                        return Ok(Some((self.key_to_value(k), *v)));
                    }
                    return Ok(None);
                }
                // Integer key but not in array - check hash part
                drop(array);
            }
        }

        // Key is in hash part, find next entry
        let table_key = match TableKey::from_value(key) {
            Some(k) => k,
            None => return Err(()), // Invalid key type
        };
        let hash = self.hash.borrow();
        let mut found = false;
        for (k, v) in hash.iter() {
            if found {
                return Ok(Some((self.key_to_value(k), *v)));
            }
            if k == &table_key {
                found = true;
            }
        }
        if found {
            Ok(None) // Was the last key
        } else {
            Err(()) // Key doesn't exist
        }
    }

    /// Convert a TableKey back to a Value
    fn key_to_value(&self, key: &TableKey) -> Value {
        match key {
            TableKey::Integer(i) => {
                if *i >= i32::MIN as i64 && *i <= i32::MAX as i64 {
                    Value::integer(*i as i32)
                } else {
                    Value::number(*i as f64)
                }
            }
            TableKey::Float(f) => Value::number(f.0),
            TableKey::Boolean(b) => Value::boolean(*b),
            TableKey::String(bits) | TableKey::Pointer(bits) => {
                // Reconstruct the value from raw bits
                Value::from_raw_bits(*bits)
            }
        }
    }

    /// Iterator over array indices (for ipairs)
    pub fn ipairs(&self) -> impl Iterator<Item = (usize, Value)> + '_ {
        let array = self.array.borrow();
        let len = array.len();
        (0..len)
            .map(move |i| {
                let array = self.array.borrow();
                (i + 1, array[i])
            })
            .take_while(|(_, v)| !v.is_nil())
    }

    /// Get array part capacity
    pub fn array_capacity(&self) -> usize {
        self.array.borrow().capacity()
    }

    /// Get hash part capacity
    pub fn hash_capacity(&self) -> usize {
        self.hash.borrow().capacity()
    }

    /// Reserve space in array part
    pub fn reserve_array(&self, additional: usize) {
        self.array.borrow_mut().reserve(additional);
    }

    /// Reserve space in hash part
    pub fn reserve_hash(&self, additional: usize) {
        self.hash.borrow_mut().reserve(additional);
    }
}

impl Value {
    /// Reconstruct a Value from raw bits (used for table key recovery)
    pub(crate) fn from_raw_bits(bits: u64) -> Self {
        Self { bits }
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("array_len", &self.array.borrow().len())
            .field("hash_len", &self.hash.borrow().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_array_operations() {
        let t = Table::new();

        t.set_array(1, Value::integer(10));
        t.set_array(2, Value::integer(20));
        t.set_array(3, Value::integer(30));

        assert_eq!(t.get_array(1).as_integer(), Some(10));
        assert_eq!(t.get_array(2).as_integer(), Some(20));
        assert_eq!(t.get_array(3).as_integer(), Some(30));
        assert!(t.get_array(4).is_nil());
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn test_hash_operations() {
        let t = Table::new();

        t.set(Value::number(1.5), Value::integer(100));
        t.set(Value::boolean(true), Value::integer(200));

        assert_eq!(t.get(&Value::number(1.5)).as_integer(), Some(100));
        assert_eq!(t.get(&Value::boolean(true)).as_integer(), Some(200));
        assert!(t.get(&Value::boolean(false)).is_nil());
    }

    #[test]
    fn test_length() {
        let t = Table::new();

        // Empty table
        assert_eq!(t.len(), 0);

        // Add elements
        t.set_array(1, Value::integer(1));
        t.set_array(2, Value::integer(2));
        t.set_array(3, Value::integer(3));
        assert_eq!(t.len(), 3);

        // Remove middle element (creates hole)
        t.set_array(2, Value::nil());
        assert_eq!(t.len(), 1); // Length stops at first nil
    }
}
