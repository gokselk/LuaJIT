//! Lua userdata implementation.
//!
//! Userdata represents arbitrary C/Rust data in Lua. Full userdata is
//! garbage collected and can have a metatable. Light userdata is just
//! a raw pointer with no GC overhead.

use super::{GcHeader, GcRef, Table};
use std::any::{Any, TypeId};
use std::cell::Cell;

/// Full userdata - GC-managed arbitrary data with optional metatable.
pub struct Userdata {
    /// GC header
    pub gc: GcHeader,
    /// Metatable (if any)
    metatable: Cell<Option<GcRef<Table>>>,
    /// User values associated with this userdata
    user_values: Vec<super::Value>,
    /// Type ID for safe downcasting
    type_id: TypeId,
    /// Size of the data
    data_size: usize,
    /// The actual data (stored inline after the struct)
    data: [u8; 0],
}

impl Userdata {
    /// Create a new simple proxy userdata (no inline data)
    /// This is safe to use with gc.alloc
    pub fn new_proxy() -> Self {
        Self {
            gc: GcHeader::new(7), // LuaType::Userdata
            metatable: Cell::new(None),
            user_values: Vec::new(),
            type_id: TypeId::of::<()>(),
            data_size: 0,
            data: [],
        }
    }

    /// Get the metatable
    pub fn get_metatable(&self) -> Option<GcRef<Table>> {
        self.metatable.get()
    }

    /// Set the metatable
    pub fn set_metatable(&self, mt: Option<GcRef<Table>>) {
        self.metatable.set(mt);
    }

    /// Get a user value by index (1-based, Lua 5.4 style)
    pub fn get_user_value(&self, index: usize) -> super::Value {
        if index >= 1 && index <= self.user_values.len() {
            self.user_values[index - 1]
        } else {
            super::Value::nil()
        }
    }

    /// Set a user value by index (1-based)
    pub fn set_user_value(&mut self, index: usize, value: super::Value) {
        if index >= 1 && index <= self.user_values.len() {
            self.user_values[index - 1] = value;
        }
    }

    /// Get the data as a byte slice
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.data.as_ptr(), self.data_size) }
    }

    /// Get the data as a mutable byte slice
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.data.as_mut_ptr(), self.data_size) }
    }

    /// Get the data size
    pub fn size(&self) -> usize {
        self.data_size
    }

    /// Check if this userdata contains data of type T
    pub fn is<T: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<T>()
    }

    /// Try to get the data as type T
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        if self.is::<T>() && self.data_size == std::mem::size_of::<T>() {
            Some(unsafe { &*(self.data.as_ptr() as *const T) })
        } else {
            None
        }
    }

    /// Try to get the data as mutable type T
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        if self.is::<T>() && self.data_size == std::mem::size_of::<T>() {
            Some(unsafe { &mut *(self.data.as_mut_ptr() as *mut T) })
        } else {
            None
        }
    }
}

/// Userdata allocator
pub struct UserdataAllocator;

impl UserdataAllocator {
    /// Allocate a new userdata with the given data
    pub fn allocate<T: 'static>(data: T, num_user_values: usize) -> *mut Userdata {
        let data_size = std::mem::size_of::<T>();
        let header_size = std::mem::size_of::<Userdata>();
        let total_size = header_size + data_size;

        // Ensure proper alignment
        let align = std::cmp::max(std::mem::align_of::<Userdata>(), std::mem::align_of::<T>());
        let layout = std::alloc::Layout::from_size_align(total_size, align).unwrap();

        let ptr = unsafe { std::alloc::alloc(layout) as *mut Userdata };

        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        unsafe {
            // Initialize the userdata header
            std::ptr::write(
                &mut (*ptr).gc,
                GcHeader::new(7), // LuaType::Userdata
            );
            std::ptr::write(&mut (*ptr).metatable, Cell::new(None));
            std::ptr::write(
                &mut (*ptr).user_values,
                vec![super::Value::nil(); num_user_values],
            );
            std::ptr::write(&mut (*ptr).type_id, TypeId::of::<T>());
            std::ptr::write(&mut (*ptr).data_size, data_size);

            // Copy the data
            let data_ptr = (ptr as *mut u8).add(header_size) as *mut T;
            std::ptr::write(data_ptr, data);
        }

        ptr
    }

    /// Allocate a new userdata with raw bytes
    pub fn allocate_bytes(size: usize, num_user_values: usize) -> *mut Userdata {
        let header_size = std::mem::size_of::<Userdata>();
        let total_size = header_size + size;

        let layout = std::alloc::Layout::from_size_align(total_size, 8).unwrap();
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) as *mut Userdata };

        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        unsafe {
            std::ptr::write(
                &mut (*ptr).gc,
                GcHeader::new(7),
            );
            std::ptr::write(&mut (*ptr).metatable, Cell::new(None));
            std::ptr::write(
                &mut (*ptr).user_values,
                vec![super::Value::nil(); num_user_values],
            );
            std::ptr::write(&mut (*ptr).type_id, TypeId::of::<[u8]>());
            std::ptr::write(&mut (*ptr).data_size, size);
        }

        ptr
    }

    /// Free a userdata
    ///
    /// # Safety
    /// The pointer must have been allocated by this allocator and not already freed.
    pub unsafe fn free<T: 'static>(ptr: *mut Userdata) {
        let data_size = (*ptr).data_size;
        let header_size = std::mem::size_of::<Userdata>();
        let total_size = header_size + data_size;
        let align = std::cmp::max(std::mem::align_of::<Userdata>(), std::mem::align_of::<T>());

        // Drop the contained data if it's a typed userdata
        if (*ptr).is::<T>() {
            let data_ptr = (ptr as *mut u8).add(header_size) as *mut T;
            std::ptr::drop_in_place(data_ptr);
        }

        // Drop the user_values vector
        std::ptr::drop_in_place(&mut (*ptr).user_values);

        let layout = std::alloc::Layout::from_size_align(total_size, align).unwrap();
        std::alloc::dealloc(ptr as *mut u8, layout);
    }
}

/// A file handle userdata (example of typed userdata)
pub struct FileHandle {
    file: Option<std::fs::File>,
    mode: FileMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Read,
    Write,
    Append,
    ReadWrite,
    Closed,
}

impl FileHandle {
    pub fn new(file: std::fs::File, mode: FileMode) -> Self {
        Self {
            file: Some(file),
            mode,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.file.is_none()
    }

    pub fn close(&mut self) -> std::io::Result<()> {
        if let Some(file) = self.file.take() {
            drop(file);
            self.mode = FileMode::Closed;
        }
        Ok(())
    }

    pub fn file(&self) -> Option<&std::fs::File> {
        self.file.as_ref()
    }

    pub fn file_mut(&mut self) -> Option<&mut std::fs::File> {
        self.file.as_mut()
    }

    pub fn mode(&self) -> FileMode {
        self.mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_userdata() {
        let ptr = UserdataAllocator::allocate(42i32, 0);

        unsafe {
            assert!((*ptr).is::<i32>());
            assert!(!(*ptr).is::<i64>());

            let val = (*ptr).downcast_ref::<i32>().unwrap();
            assert_eq!(*val, 42);

            UserdataAllocator::free::<i32>(ptr);
        }
    }

    #[test]
    fn test_userdata_with_user_values() {
        let ptr = UserdataAllocator::allocate("hello".to_string(), 2);

        unsafe {
            let ud = &mut *ptr;

            ud.set_user_value(1, super::super::Value::integer(100));
            assert_eq!(ud.get_user_value(1).as_integer(), Some(100));
            assert!(ud.get_user_value(2).is_nil());

            // Cleanup
            std::ptr::drop_in_place(&mut (*ptr).user_values);
            let data_ptr = (ptr as *mut u8).add(std::mem::size_of::<Userdata>()) as *mut String;
            std::ptr::drop_in_place(data_ptr);
        }
    }
}
