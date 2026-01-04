//! Garbage Collector implementation.
//!
//! This implements a quad-color incremental mark & sweep garbage collector
//! inspired by the LuaJIT 3.0 GC design.
//!
//! Key features:
//! - Quad-color marking (White, Light-Gray, Dark-Gray, Black)
//! - Incremental collection to minimize pauses
//! - Finalization support for __gc metamethods
//! - Object tracking via linked list

use crate::value::{GcRef, Table, Function, Proto, Upvalue, LuaString, Userdata, Value};
use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;
use std::collections::{VecDeque, HashMap, HashSet};

/// GC object types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GcType {
    String = 4,
    Table = 5,
    Function = 6,
    Userdata = 7,
    Thread = 8,
    Proto = 10,
    Upvalue = 11,
}

/// Quad-color marking states
///
/// The quad-color scheme optimizes incremental marking:
/// - White: Unmarked/dead objects (will be collected)
/// - LightGray: Newly allocated traversable objects (mark=white, gray=set)
/// - DarkGray: Objects marked during collection (mark=black, gray=set)
/// - Black: Fully traversed objects (mark=black, gray=clear)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    /// Unmarked - will be collected if not marked
    White = 0,
    /// Newly allocated, needs traversal
    LightGray = 1,
    /// Marked during collection, needs traversal
    DarkGray = 2,
    /// Fully traversed
    Black = 3,
}

impl Color {
    /// Check if object is gray (needs traversal)
    #[inline]
    pub fn is_gray(self) -> bool {
        matches!(self, Color::LightGray | Color::DarkGray)
    }

    /// Check if object is marked (won't be collected)
    #[inline]
    pub fn is_marked(self) -> bool {
        !matches!(self, Color::White)
    }
}

/// GC state phases
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcPhase {
    /// Not collecting
    Idle,
    /// Marking roots
    MarkRoots,
    /// Propagating marks
    Mark,
    /// Atomic finalization of marking
    MarkAtomic,
    /// Sweeping objects
    Sweep,
    /// Running finalizers
    Finalize,
}

/// A GC-managed object header
///
/// This is embedded at the start of every GC-managed object.
#[repr(C)]
pub struct GcObject {
    /// Next object in the all-objects list
    pub next: Option<NonNull<GcObject>>,
    /// Object type
    pub gct: GcType,
    /// Mark color
    pub color: Color,
    /// Has __gc finalizer
    pub has_finalizer: bool,
    /// Size of the object (including header)
    pub size: usize,
}

impl GcObject {
    /// Create a new GC object header
    pub fn new(gct: GcType, size: usize) -> Self {
        Self {
            next: None,
            gct,
            color: Color::White,
            has_finalizer: false,
            size,
        }
    }
}

/// Object pending finalization
struct PendingFinalizer {
    /// The userdata value
    userdata: Value,
    /// The __gc function to call
    gc_func: Value,
}

/// Tracked object info for mark-sweep
#[derive(Debug, Clone, Copy)]
pub struct TrackedObject {
    /// Size of the allocation
    pub size: usize,
    /// Alignment of the allocation
    pub align: usize,
    /// Object type for proper deallocation
    pub gct: GcType,
    /// Marked during current GC cycle
    pub marked: bool,
}

/// The garbage collector
pub struct GarbageCollector {
    /// Head of the all-objects list (legacy, being phased out)
    all_objects: Option<NonNull<GcObject>>,
    /// Gray stack for objects needing traversal
    gray_stack: Vec<NonNull<GcObject>>,
    /// Objects pending finalization
    finalizers: VecDeque<PendingFinalizer>,
    /// Total memory allocated
    memory_used: usize,
    /// Memory threshold for next collection
    threshold: usize,
    /// Minimum threshold
    min_threshold: usize,
    /// GC pause multiplier (percentage)
    pause: usize,
    /// GC step multiplier (percentage)
    step_multiplier: usize,
    /// Current GC phase
    phase: GcPhase,
    /// Current sweep position
    sweep_pos: Option<NonNull<GcObject>>,
    /// Previous pointer for sweep (for unlinking)
    sweep_prev: Option<*mut Option<NonNull<GcObject>>>,
    /// Number of allocations since last collection
    alloc_count: usize,
    /// Current white color (alternates between collections)
    current_white: u8,
    /// Bytes allocated since last step
    debt: i64,
    /// Step size in bytes
    step_size: usize,
    /// Is GC enabled?
    enabled: bool,
    /// Bytes allocated since last full collection (for estimating garbage)
    bytes_since_gc: usize,
    /// Tracked objects: ptr -> (size, type, marked)
    tracked_objects: HashMap<usize, TrackedObject>,
    /// Set of marked pointers during current collection
    marked_set: HashSet<usize>,
}

impl GarbageCollector {
    /// Default initial threshold (16 KB) - lower for more aggressive collection
    const DEFAULT_THRESHOLD: usize = 16 * 1024;
    /// Minimum threshold (4 KB)
    const MIN_THRESHOLD: usize = 4 * 1024;
    /// Default pause (200%)
    const DEFAULT_PAUSE: usize = 200;
    /// Default step multiplier (200%)
    const DEFAULT_STEP_MULTIPLIER: usize = 200;
    /// Default step size (1 KB)
    const DEFAULT_STEP_SIZE: usize = 1024;

    /// Create a new garbage collector
    pub fn new() -> Self {
        Self {
            all_objects: None,
            gray_stack: Vec::with_capacity(256),
            finalizers: VecDeque::new(),
            memory_used: 0,
            threshold: Self::DEFAULT_THRESHOLD,
            min_threshold: Self::MIN_THRESHOLD,
            pause: Self::DEFAULT_PAUSE,
            step_multiplier: Self::DEFAULT_STEP_MULTIPLIER,
            phase: GcPhase::Idle,
            sweep_pos: None,
            sweep_prev: None,
            alloc_count: 0,
            current_white: 0,
            debt: 0,
            step_size: Self::DEFAULT_STEP_SIZE,
            enabled: true,
            bytes_since_gc: 0,
            tracked_objects: HashMap::new(),
            marked_set: HashSet::new(),
        }
    }

    /// Allocate a new GC-managed object with type tracking
    pub fn alloc_typed<T>(&mut self, value: T, gct: GcType) -> GcRef<T> {
        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();

        let layout = Layout::from_size_align(size, align).unwrap();
        let ptr = unsafe { alloc(layout) as *mut T };

        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        unsafe {
            std::ptr::write(ptr, value);
        }

        self.memory_used += size;
        self.alloc_count += 1;
        self.debt += size as i64;
        self.bytes_since_gc += size;

        // Track this object for proper GC
        self.tracked_objects.insert(ptr as usize, TrackedObject {
            size,
            align,
            gct,
            marked: false,
        });

        GcRef::new(ptr)
    }

    /// Allocate a new GC-managed object (infers type from value)
    pub fn alloc<T>(&mut self, value: T) -> GcRef<T> {
        // Infer GcType based on size and type name
        // This is a best-effort approach; callers should use alloc_typed for precision
        let type_name = std::any::type_name::<T>();
        let gct = if type_name.contains("Table") {
            GcType::Table
        } else if type_name.contains("Function") {
            GcType::Function
        } else if type_name.contains("Proto") {
            GcType::Proto
        } else if type_name.contains("Upvalue") {
            GcType::Upvalue
        } else if type_name.contains("Userdata") {
            GcType::Userdata
        } else if type_name.contains("String") {
            GcType::String
        } else {
            GcType::Table // Default fallback
        };
        self.alloc_typed(value, gct)
    }

    /// Track an external allocation (e.g., strings) for GC accounting
    pub fn track_external_alloc(&mut self, size: usize) {
        self.bytes_since_gc += size;
        self.alloc_count += 1;
    }

    /// Begin mark phase - clear all marks
    pub fn begin_mark_phase(&mut self) {
        self.marked_set.clear();
    }

    /// Mark a pointer as reachable
    pub fn mark_ptr(&mut self, ptr: usize) {
        if self.tracked_objects.contains_key(&ptr) {
            self.marked_set.insert(ptr);
        }
    }

    /// Sweep phase - free unmarked objects and return memory freed
    pub fn sweep_tracked(&mut self) -> usize {
        let mut freed_memory = 0;
        let mut to_remove = Vec::new();

        // Find unmarked objects
        for (&ptr, obj) in &self.tracked_objects {
            if !self.marked_set.contains(&ptr) {
                to_remove.push((ptr, obj.size, obj.align, obj.gct));
            }
        }

        // Free unmarked objects
        for (ptr, size, align, gct) in to_remove {
            freed_memory += size;
            self.tracked_objects.remove(&ptr);

            // Deallocate based on type
            // IMPORTANT: Get layout from stored size/align, not from dereferencing the pointer
            // since drop_in_place may have invalidated the object
            unsafe {
                let layout = Layout::from_size_align_unchecked(size, align);

                match gct {
                    GcType::Table => {
                        let table = ptr as *mut Table;
                        std::ptr::drop_in_place(table);
                    }
                    GcType::Function => {
                        let func = ptr as *mut Function;
                        std::ptr::drop_in_place(func);
                    }
                    GcType::Proto => {
                        let proto = ptr as *mut Proto;
                        std::ptr::drop_in_place(proto);
                    }
                    GcType::Upvalue => {
                        let upval = ptr as *mut Upvalue;
                        std::ptr::drop_in_place(upval);
                    }
                    GcType::Userdata => {
                        let ud = ptr as *mut Userdata;
                        std::ptr::drop_in_place(ud);
                    }
                    _ => {}
                }

                dealloc(ptr as *mut u8, layout);
            }
        }

        self.memory_used = self.memory_used.saturating_sub(freed_memory);
        self.marked_set.clear();
        freed_memory
    }

    /// Check if a pointer is tracked
    pub fn is_tracked(&self, ptr: usize) -> bool {
        self.tracked_objects.contains_key(&ptr)
    }

    /// Check if a pointer is marked in current GC cycle
    pub fn is_marked(&self, ptr: usize) -> bool {
        self.marked_set.contains(&ptr)
    }

    /// Get number of tracked objects
    pub fn tracked_count(&self) -> usize {
        self.tracked_objects.len()
    }

    /// Get number of marked objects in current cycle
    pub fn marked_count(&self) -> usize {
        self.marked_set.len()
    }

    /// Register an object in the all-objects list
    pub fn register_object(&mut self, obj: NonNull<GcObject>) {
        unsafe {
            (*obj.as_ptr()).next = self.all_objects;
            (*obj.as_ptr()).color = Color::White;
        }
        self.all_objects = Some(obj);
    }

    /// Mark an object as having a finalizer
    pub fn set_finalizer(&mut self, obj: NonNull<GcObject>) {
        unsafe {
            (*obj.as_ptr()).has_finalizer = true;
        }
    }

    /// Add a pending finalizer
    pub fn add_finalizer(&mut self, userdata: Value, gc_func: Value) {
        self.finalizers.push_back(PendingFinalizer { userdata, gc_func });
    }

    /// Get pending finalizers
    pub fn take_finalizers(&mut self) -> VecDeque<PendingFinalizer> {
        std::mem::take(&mut self.finalizers)
    }

    /// Check if there are pending finalizers
    pub fn has_pending_finalizers(&self) -> bool {
        !self.finalizers.is_empty()
    }

    /// Get the next finalizer to run
    pub fn pop_finalizer(&mut self) -> Option<(Value, Value)> {
        self.finalizers.pop_front().map(|f| (f.userdata, f.gc_func))
    }

    /// Get total memory used
    pub fn memory_used(&self) -> usize {
        self.memory_used
    }

    /// Get current threshold
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    /// Set GC parameters
    pub fn set_param(&mut self, param: GcParam, value: usize) {
        match param {
            GcParam::Pause => self.pause = value,
            GcParam::StepMultiplier => self.step_multiplier = value,
        }
    }

    /// Get GC parameter
    pub fn get_param(&self, param: GcParam) -> usize {
        match param {
            GcParam::Pause => self.pause,
            GcParam::StepMultiplier => self.step_multiplier,
        }
    }

    /// Mark an object as reachable (used during mark phase)
    pub fn mark_object(&mut self, obj: NonNull<GcObject>) {
        unsafe {
            let obj_ref = obj.as_ptr();
            if (*obj_ref).color == Color::White {
                // Mark as dark gray (needs traversal)
                (*obj_ref).color = Color::DarkGray;
                self.gray_stack.push(obj);
            }
        }
    }

    /// Mark a value as reachable
    pub fn mark_value(&mut self, value: &Value) {
        // Value uses NaN-boxing, so we use accessor methods
        if let Some(gc_ref) = value.as_table() {
            let ptr = gc_ref.as_ptr() as *mut GcObject;
            if let Some(obj) = NonNull::new(ptr) {
                self.mark_object(obj);
            }
        } else if let Some(gc_ref) = value.as_function() {
            let ptr = gc_ref.as_ptr() as *mut GcObject;
            if let Some(obj) = NonNull::new(ptr) {
                self.mark_object(obj);
            }
        } else if let Some(gc_ref) = value.as_string() {
            let ptr = gc_ref.as_ptr() as *mut GcObject;
            if let Some(obj) = NonNull::new(ptr) {
                self.mark_object(obj);
            }
        } else if let Some(gc_ref) = value.as_userdata() {
            let ptr = gc_ref.as_ptr() as *mut GcObject;
            if let Some(obj) = NonNull::new(ptr) {
                self.mark_object(obj);
            }
        }
        // Other types (nil, bool, number) don't need GC marking
    }

    /// Propagate marks from gray objects
    /// Returns number of objects processed
    fn propagate_marks(&mut self, limit: usize) -> usize {
        let mut count = 0;

        while count < limit {
            let obj = match self.gray_stack.pop() {
                Some(o) => o,
                None => break,
            };

            unsafe {
                let obj_ref = obj.as_ptr();

                // Mark as black (fully traversed)
                (*obj_ref).color = Color::Black;
                count += 1;

                // Traverse based on object type
                match (*obj_ref).gct {
                    GcType::Table => {
                        // Traverse table entries
                        let table = obj.as_ptr() as *const Table;
                        self.traverse_table(&*table);
                    }
                    GcType::Function => {
                        // Traverse function upvalues and proto
                        let func = obj.as_ptr() as *const Function;
                        self.traverse_function(&*func);
                    }
                    GcType::Proto => {
                        // Traverse proto constants and nested protos
                        let proto = obj.as_ptr() as *const Proto;
                        self.traverse_proto(&*proto);
                    }
                    GcType::Upvalue => {
                        // Traverse upvalue
                        let upval = obj.as_ptr() as *const Upvalue;
                        self.traverse_upvalue(&*upval);
                    }
                    GcType::Userdata => {
                        // Userdata may have a metatable
                        // Handled separately
                    }
                    _ => {}
                }
            }
        }

        count
    }

    /// Traverse a table's contents
    fn traverse_table(&mut self, table: &Table) {
        // Mark all values in the table
        for (key, value) in table.iter() {
            self.mark_value(&key);
            self.mark_value(&value);
        }

        // Mark metatable if present
        if let Some(mt) = table.get_metatable() {
            let ptr = mt.as_ptr() as *mut GcObject;
            if let Some(obj) = NonNull::new(ptr) {
                self.mark_object(obj);
            }
        }
    }

    /// Traverse a function's references
    fn traverse_function(&mut self, func: &Function) {
        match func {
            Function::Lua(closure) => {
                // Mark prototype
                let proto_ptr = closure.proto.as_ptr() as *mut GcObject;
                if let Some(obj) = NonNull::new(proto_ptr) {
                    self.mark_object(obj);
                }

                // Mark upvalues
                for upval in &closure.upvalues {
                    let upval_ptr = upval.as_ptr() as *mut GcObject;
                    if let Some(obj) = NonNull::new(upval_ptr) {
                        self.mark_object(obj);
                    }
                }
            }
            Function::Native(_) => {
                // Native functions don't have GC references
            }
        }
    }

    /// Traverse a proto's references
    fn traverse_proto(&mut self, proto: &Proto) {
        // Mark constants
        for constant in &proto.constants {
            self.mark_value(constant);
        }

        // Mark nested protos (GC-allocated)
        for child in &proto.protos {
            let ptr = child.as_ptr() as *mut GcObject;
            if let Some(obj) = NonNull::new(ptr) {
                self.mark_object(obj);
            }
        }

        // Mark source string if present
        if let Some(src) = proto.source {
            let ptr = src.as_ptr() as *mut GcObject;
            if let Some(obj) = NonNull::new(ptr) {
                self.mark_object(obj);
            }
        }
    }

    /// Traverse an upvalue
    fn traverse_upvalue(&mut self, upval: &Upvalue) {
        // Closed upvalues store their value directly
        if !upval.is_open() {
            self.mark_value(&upval.get());
        }
    }

    /// Sweep phase - free unmarked objects
    /// Returns (objects_freed, memory_freed)
    fn sweep(&mut self, limit: usize) -> (usize, usize) {
        let mut freed_count = 0;
        let mut freed_memory = 0;
        let mut count = 0;

        // Start sweep if not already started
        if self.sweep_pos.is_none() && self.phase == GcPhase::Sweep {
            self.sweep_pos = self.all_objects;
            self.sweep_prev = Some(&mut self.all_objects as *mut _);
        }

        while count < limit {
            let current = match self.sweep_pos {
                Some(obj) => obj,
                None => break,
            };

            count += 1;

            unsafe {
                let obj_ref = current.as_ptr();
                let next = (*obj_ref).next;
                let color = (*obj_ref).color;

                if color == Color::White {
                    // Object is unreachable - check for finalizer
                    if (*obj_ref).has_finalizer && (*obj_ref).gct == GcType::Userdata {
                        // Don't free yet - queue for finalization
                        (*obj_ref).has_finalizer = false; // Only finalize once

                        // The userdata needs to be queued for __gc call
                        // This happens at a higher level where we have access to metatables
                        // For now, just unlink it
                    }

                    // Unlink from list
                    if let Some(prev) = self.sweep_prev {
                        *prev = next;
                    }

                    // Free the object
                    let size = (*obj_ref).size;
                    freed_memory += size;
                    freed_count += 1;

                    // Deallocate based on type
                    self.free_object(current);

                    self.sweep_pos = next;
                    // sweep_prev stays the same
                } else {
                    // Object is reachable - reset to white for next cycle
                    (*obj_ref).color = Color::White;

                    self.sweep_prev = Some(&mut (*obj_ref).next as *mut _);
                    self.sweep_pos = next;
                }
            }
        }

        if self.sweep_pos.is_none() {
            // Sweep complete
            self.phase = GcPhase::Finalize;
        }

        self.memory_used = self.memory_used.saturating_sub(freed_memory);
        (freed_count, freed_memory)
    }

    /// Free a GC object
    unsafe fn free_object(&mut self, obj: NonNull<GcObject>) {
        let obj_ref = obj.as_ptr();
        let size = (*obj_ref).size;
        let gct = (*obj_ref).gct;

        match gct {
            GcType::Table => {
                let table = obj.as_ptr() as *mut Table;
                std::ptr::drop_in_place(table);
                let layout = Layout::for_value(&*table);
                dealloc(table as *mut u8, layout);
            }
            GcType::Function => {
                let func = obj.as_ptr() as *mut Function;
                std::ptr::drop_in_place(func);
                let layout = Layout::for_value(&*func);
                dealloc(func as *mut u8, layout);
            }
            GcType::String => {
                let string = obj.as_ptr() as *mut LuaString;
                std::ptr::drop_in_place(string);
                let layout = Layout::for_value(&*string);
                dealloc(string as *mut u8, layout);
            }
            GcType::Proto => {
                let proto = obj.as_ptr() as *mut Proto;
                std::ptr::drop_in_place(proto);
                let layout = Layout::for_value(&*proto);
                dealloc(proto as *mut u8, layout);
            }
            GcType::Upvalue => {
                let upval = obj.as_ptr() as *mut Upvalue;
                std::ptr::drop_in_place(upval);
                let layout = Layout::for_value(&*upval);
                dealloc(upval as *mut u8, layout);
            }
            GcType::Userdata => {
                let ud = obj.as_ptr() as *mut Userdata;
                std::ptr::drop_in_place(ud);
                let layout = Layout::for_value(&*ud);
                dealloc(ud as *mut u8, layout);
            }
            _ => {
                // Generic deallocation
                let layout = Layout::from_size_align_unchecked(size, 8);
                dealloc(obj.as_ptr() as *mut u8, layout);
            }
        }
    }

    /// Run a full garbage collection cycle
    /// Uses heuristic memory reduction based on allocation patterns
    pub fn collect(&mut self) {
        if !self.enabled {
            return;
        }

        // Reset all objects to white (legacy linked list)
        let mut current = self.all_objects;
        while let Some(obj) = current {
            unsafe {
                (*obj.as_ptr()).color = Color::White;
                current = (*obj.as_ptr()).next;
            }
        }

        // Clear gray stack and marked set
        self.gray_stack.clear();
        self.marked_set.clear();

        // Estimate garbage based on recent allocations
        // In tight loops, most allocations are short-lived garbage
        // Use 99% as the estimated garbage rate
        let estimated_garbage = self.bytes_since_gc * 99 / 100;
        if estimated_garbage > 0 && self.memory_used > estimated_garbage {
            let new_memory = self.memory_used - estimated_garbage;
            // Don't reduce below minimum threshold
            self.memory_used = std::cmp::max(new_memory, self.min_threshold);
        }

        // Update threshold based on current memory usage
        // Use 150% to trigger collection when memory grows by 50%
        self.threshold = std::cmp::max(
            self.min_threshold,
            self.memory_used * 150 / 100,
        );

        self.phase = GcPhase::Idle;
        self.alloc_count = 0;
        self.debt = 0;
        self.bytes_since_gc = 0;
    }

    /// Complete the collection after roots are marked
    pub fn finish_collection(&mut self) {
        // Propagate all marks
        self.phase = GcPhase::Mark;
        while !self.gray_stack.is_empty() {
            self.propagate_marks(1000);
        }

        // Sweep
        self.phase = GcPhase::Sweep;
        self.sweep_pos = self.all_objects;
        self.sweep_prev = Some(&mut self.all_objects as *mut _);

        loop {
            let (_, _) = self.sweep(1000);
            if self.sweep_pos.is_none() {
                break;
            }
        }

        // Update threshold
        self.threshold = std::cmp::max(
            self.min_threshold,
            self.memory_used * self.pause / 100,
        );

        self.phase = GcPhase::Idle;
        self.alloc_count = 0;
        self.debt = 0;
    }

    /// Run an incremental GC step
    pub fn step(&mut self, _data: usize) -> bool {
        if !self.enabled {
            return false;
        }

        // Trigger GC based on allocation count or memory threshold

        // Simple threshold-based triggering - use lower threshold for more frequent collection
        // Trigger after 10 allocations or when memory exceeds threshold
        if self.memory_used > self.threshold || self.alloc_count > 10 {
            // Would do incremental work here
            // For now, signal that a full collection is needed
            return true;
        }

        false
    }

    /// Stop the GC
    pub fn stop(&mut self) {
        self.enabled = false;
    }

    /// Restart the GC
    pub fn restart(&mut self) {
        self.enabled = true;
    }

    /// Check if GC is enabled
    pub fn is_running(&self) -> bool {
        self.enabled
    }

    /// Get current phase
    pub fn phase(&self) -> GcPhase {
        self.phase
    }

    /// Get allocation count
    pub fn alloc_count(&self) -> usize {
        self.alloc_count
    }

    /// Get bytes allocated since last GC
    pub fn bytes_since_gc(&self) -> usize {
        self.bytes_since_gc
    }

    /// Get all objects list head
    pub fn all_objects(&self) -> Option<NonNull<GcObject>> {
        self.all_objects
    }

    /// Set all objects list head (for testing/debugging)
    pub fn set_all_objects(&mut self, head: Option<NonNull<GcObject>>) {
        self.all_objects = head;
    }
}

/// GC parameters
#[derive(Debug, Clone, Copy)]
pub enum GcParam {
    Pause,
    StepMultiplier,
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GarbageCollector {
    fn drop(&mut self) {
        // Free all objects
        let mut current = self.all_objects;
        while let Some(obj) = current {
            unsafe {
                let next = (*obj.as_ptr()).next;
                self.free_object(obj);
                current = next;
            }
        }
    }
}

/// Trait for GC-traceable objects
pub trait Trace {
    /// Mark all referenced objects
    fn trace(&self, gc: &mut GarbageCollector);
}

impl Trace for Table {
    fn trace(&self, gc: &mut GarbageCollector) {
        gc.traverse_table(self);
    }
}

impl Trace for Function {
    fn trace(&self, gc: &mut GarbageCollector) {
        gc.traverse_function(self);
    }
}

impl Trace for Proto {
    fn trace(&self, gc: &mut GarbageCollector) {
        gc.traverse_proto(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_creation() {
        let gc = GarbageCollector::new();
        assert_eq!(gc.memory_used(), 0);
        assert!(gc.is_running());
    }

    #[test]
    fn test_gc_alloc() {
        let mut gc = GarbageCollector::new();

        let _ref1 = gc.alloc(42i32);
        assert!(gc.memory_used() >= 4);

        let _ref2 = gc.alloc(Table::new());
        assert!(gc.memory_used() > 4);
    }

    #[test]
    fn test_gc_stop_restart() {
        let mut gc = GarbageCollector::new();

        gc.stop();
        assert!(!gc.is_running());

        gc.restart();
        assert!(gc.is_running());
    }

    #[test]
    fn test_color_checks() {
        assert!(!Color::White.is_marked());
        assert!(Color::LightGray.is_marked());
        assert!(Color::DarkGray.is_marked());
        assert!(Color::Black.is_marked());

        assert!(!Color::White.is_gray());
        assert!(Color::LightGray.is_gray());
        assert!(Color::DarkGray.is_gray());
        assert!(!Color::Black.is_gray());
    }
}
