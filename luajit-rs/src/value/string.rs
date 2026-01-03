//! Lua string implementation.
//!
//! Lua strings are immutable, interned byte sequences. They can contain
//! any bytes including NUL. This implementation uses string interning
//! for efficient comparison and hashing.

use super::GcHeader;
use std::hash::{Hash, Hasher};
use std::fmt;
use rustc_hash::FxHasher;

/// An interned Lua string.
pub struct LuaString {
    /// GC header
    pub gc: GcHeader,
    /// Cached hash value
    hash: u64,
    /// String length
    len: usize,
    /// The actual string data (stored inline after the struct)
    /// Using a flexible array member pattern
    data: [u8; 0],
}

impl LuaString {
    /// Get the string data as bytes
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(self.data.as_ptr(), self.len)
        }
    }

    /// Get the string as a str (may fail if not valid UTF-8)
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(self.as_bytes()).ok()
    }

    /// Get the string length in bytes
    pub fn len(&self) -> usize {
        self.len
    }

    /// Check if string is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the cached hash
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Compute hash for a byte slice
    pub fn compute_hash(bytes: &[u8]) -> u64 {
        let mut hasher = FxHasher::default();
        bytes.hash(&mut hasher);
        hasher.finish()
    }
}

impl PartialEq for LuaString {
    fn eq(&self, other: &Self) -> bool {
        // Fast path: if hashes differ, strings differ
        if self.hash != other.hash || self.len != other.len {
            return false;
        }
        // Slow path: compare bytes
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for LuaString {}

impl Hash for LuaString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Use the cached hash
        state.write_u64(self.hash);
    }
}

impl fmt::Debug for LuaString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_str() {
            Some(s) => write!(f, "{:?}", s),
            None => write!(f, "<binary string len={}>", self.len),
        }
    }
}

impl fmt::Display for LuaString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_str() {
            Some(s) => write!(f, "{}", s),
            None => {
                // Escape non-printable characters
                for &byte in self.as_bytes() {
                    if byte.is_ascii_graphic() || byte == b' ' {
                        write!(f, "{}", byte as char)?;
                    } else {
                        write!(f, "\\{:03}", byte)?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// String interner for efficient string storage and comparison.
pub struct StringInterner {
    /// Interned strings, indexed by hash
    strings: hashbrown::HashMap<u64, Vec<*mut LuaString>, rustc_hash::FxBuildHasher>,
    /// Total memory used by strings
    memory_used: usize,
}

impl StringInterner {
    /// Create a new string interner
    pub fn new() -> Self {
        Self {
            strings: hashbrown::HashMap::with_hasher(rustc_hash::FxBuildHasher),
            memory_used: 0,
        }
    }

    /// Intern a string, returning a pointer to the interned copy
    pub fn intern(&mut self, bytes: &[u8]) -> *mut LuaString {
        let hash = LuaString::compute_hash(bytes);

        // Check if already interned
        if let Some(bucket) = self.strings.get(&hash) {
            for &ptr in bucket {
                let s = unsafe { &*ptr };
                if s.as_bytes() == bytes {
                    return ptr;
                }
            }
        }

        // Allocate new string
        let ptr = self.allocate_string(bytes, hash);

        // Add to intern table
        self.strings
            .entry(hash)
            .or_insert_with(Vec::new)
            .push(ptr);

        ptr
    }

    /// Allocate a new string (internal)
    fn allocate_string(&mut self, bytes: &[u8], hash: u64) -> *mut LuaString {
        // Calculate size: header + string data + null terminator
        let header_size = std::mem::size_of::<LuaString>();
        let total_size = header_size + bytes.len() + 1;

        // Allocate memory
        let layout = std::alloc::Layout::from_size_align(total_size, 8).unwrap();
        let ptr = unsafe { std::alloc::alloc(layout) as *mut LuaString };

        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        // Initialize the string
        unsafe {
            (*ptr).gc = GcHeader::new(4); // LuaType::String
            (*ptr).hash = hash;
            (*ptr).len = bytes.len();

            // Copy string data
            let data_ptr = (ptr as *mut u8).add(header_size);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr, bytes.len());
            // Add null terminator for C compatibility
            *data_ptr.add(bytes.len()) = 0;
        }

        self.memory_used += total_size;
        ptr
    }

    /// Intern a string from a Rust &str
    pub fn intern_str(&mut self, s: &str) -> *mut LuaString {
        self.intern(s.as_bytes())
    }

    /// Get total memory used by interned strings
    pub fn memory_used(&self) -> usize {
        self.memory_used
    }

    /// Get number of unique strings
    pub fn len(&self) -> usize {
        self.strings.values().map(|v| v.len()).sum()
    }

    /// Check if interner is empty
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StringInterner {
    fn drop(&mut self) {
        // Free all interned strings
        for bucket in self.strings.values() {
            for &ptr in bucket {
                unsafe {
                    let s = &*ptr;
                    let header_size = std::mem::size_of::<LuaString>();
                    let total_size = header_size + s.len + 1;
                    let layout = std::alloc::Layout::from_size_align(total_size, 8).unwrap();
                    std::alloc::dealloc(ptr as *mut u8, layout);
                }
            }
        }
    }
}

// Safety: StringInterner manages its own memory and is not shared
unsafe impl Send for StringInterner {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_same_string() {
        let mut interner = StringInterner::new();

        let s1 = interner.intern(b"hello");
        let s2 = interner.intern(b"hello");

        // Same string should return same pointer
        assert_eq!(s1, s2);
    }

    #[test]
    fn test_intern_different_strings() {
        let mut interner = StringInterner::new();

        let s1 = interner.intern(b"hello");
        let s2 = interner.intern(b"world");

        // Different strings should return different pointers
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_string_content() {
        let mut interner = StringInterner::new();

        let ptr = interner.intern(b"test string");
        let s = unsafe { &*ptr };

        assert_eq!(s.as_bytes(), b"test string");
        assert_eq!(s.as_str(), Some("test string"));
        assert_eq!(s.len(), 11);
    }
}
