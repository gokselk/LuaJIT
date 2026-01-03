//! JIT Compiler using Cranelift.
//!
//! This module implements a trace-based JIT compiler that compiles hot bytecode
//! paths to native machine code using Cranelift as the backend.
//!
//! The JIT follows a similar design to LuaJIT:
//! 1. Detect hot loops/functions via counters
//! 2. Record a trace (sequence of bytecode operations with type guards)
//! 3. Compile the trace to native code via Cranelift
//! 4. Link traces together for efficient execution

mod trace;
mod compiler;
mod ir;
mod codegen;

pub use trace::{Trace, TraceState, TraceRecorder};
pub use compiler::JitCompiler;
pub use ir::{IrBuilder, IrInstruction, IrType};
pub use codegen::CodeGenerator;

use crate::value::{Value, LuaResult, GcRef, Proto};
use crate::bytecode::Instruction;
use std::collections::HashMap;

/// Hot count threshold for trace recording
pub const HOT_THRESHOLD: u32 = 56;

/// Maximum trace length
pub const MAX_TRACE_LENGTH: usize = 4000;

/// Maximum number of traces
pub const MAX_TRACES: usize = 1000;

/// JIT state
pub struct JitState {
    /// Hot counters for loops
    pub hot_counters: HashMap<usize, u32>,
    /// Compiled traces
    pub traces: Vec<CompiledTrace>,
    /// The JIT compiler
    pub compiler: JitCompiler,
    /// Is JIT enabled?
    pub enabled: bool,
    /// Current trace being recorded (if any)
    pub recording: Option<TraceRecorder>,
}

/// A compiled trace
pub struct CompiledTrace {
    /// Trace ID
    pub id: usize,
    /// Starting bytecode PC
    pub start_pc: usize,
    /// Starting prototype
    pub proto: GcRef<Proto>,
    /// Compiled machine code
    pub code: Box<[u8]>,
    /// Entry point offset in code
    pub entry: usize,
    /// Exit stubs for side exits
    pub exits: Vec<TraceExit>,
    /// Link to another trace (for loops)
    pub link: Option<usize>,
    /// Side traces
    pub side_traces: Vec<usize>,
}

/// Exit point from a trace
#[derive(Debug, Clone)]
pub struct TraceExit {
    /// Exit ID
    pub id: usize,
    /// Target bytecode PC
    pub target_pc: usize,
    /// Snapshot of register values
    pub snapshot: Vec<SnapshotEntry>,
}

/// Entry in a trace snapshot
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// Stack slot
    pub slot: u8,
    /// Value type at this point
    pub value_type: IrType,
    /// IR reference (for reconstructing value)
    pub ir_ref: usize,
}

impl JitState {
    /// Create a new JIT state
    pub fn new() -> LuaResult<Self> {
        Ok(Self {
            hot_counters: HashMap::new(),
            traces: Vec::new(),
            compiler: JitCompiler::new()?,
            enabled: true,
            recording: None,
        })
    }

    /// Check if a bytecode location is hot
    pub fn is_hot(&mut self, pc: usize) -> bool {
        let counter = self.hot_counters.entry(pc).or_insert(0);
        *counter += 1;
        *counter >= HOT_THRESHOLD
    }

    /// Start recording a trace
    pub fn start_recording(&mut self, pc: usize, proto: GcRef<Proto>) {
        if self.recording.is_some() || self.traces.len() >= MAX_TRACES {
            return;
        }

        self.recording = Some(TraceRecorder::new(self.traces.len(), pc, proto));
    }

    /// Record a bytecode instruction
    pub fn record(&mut self, instr: Instruction, stack: &[Value]) -> TraceState {
        if let Some(ref mut recorder) = self.recording {
            recorder.record(instr, stack)
        } else {
            TraceState::NotRecording
        }
    }

    /// Finish recording and compile the trace
    pub fn finish_recording(&mut self) -> LuaResult<Option<usize>> {
        let recorder = match self.recording.take() {
            Some(r) => r,
            None => return Ok(None),
        };

        // Extract values before consuming recorder
        let start_pc = recorder.start_pc;
        let proto = recorder.proto;
        let link = recorder.link_trace;

        // Build IR from recorded trace
        let ir = recorder.build_ir()?;

        // Compile to native code
        let (code, exits) = self.compiler.compile(&ir)?;

        let trace_id = self.traces.len();
        let compiled = CompiledTrace {
            id: trace_id,
            start_pc,
            proto,
            code,
            entry: 0,
            exits,
            link,
            side_traces: Vec::new(),
        };

        self.traces.push(compiled);
        Ok(Some(trace_id))
    }

    /// Abort current recording
    pub fn abort_recording(&mut self) {
        self.recording = None;
    }

    /// Get a compiled trace
    pub fn get_trace(&self, id: usize) -> Option<&CompiledTrace> {
        self.traces.get(id)
    }

    /// Execute a trace (returns number of exits used)
    pub fn execute_trace(&self, _id: usize, _stack: &mut [Value]) -> LuaResult<usize> {
        // This would call into the compiled code
        // For now, return 0 to indicate no exit taken
        Ok(0)
    }

    /// Enable/disable JIT
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Flush all compiled traces
    pub fn flush(&mut self) {
        self.traces.clear();
        self.hot_counters.clear();
        self.recording = None;
    }

    /// Get JIT stats
    pub fn stats(&self) -> JitStats {
        JitStats {
            num_traces: self.traces.len(),
            code_size: self.traces.iter().map(|t| t.code.len()).sum(),
            hot_locations: self.hot_counters.len(),
        }
    }
}

impl Default for JitState {
    fn default() -> Self {
        Self::new().expect("failed to create JIT state")
    }
}

/// JIT statistics
#[derive(Debug, Clone)]
pub struct JitStats {
    pub num_traces: usize,
    pub code_size: usize,
    pub hot_locations: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jit_state_creation() {
        let state = JitState::new().unwrap();
        assert!(state.enabled);
        assert!(state.traces.is_empty());
    }

    #[test]
    fn test_hot_detection() {
        let mut state = JitState::new().unwrap();

        for _ in 0..HOT_THRESHOLD - 1 {
            assert!(!state.is_hot(100));
        }
        assert!(state.is_hot(100)); // Should be hot now
    }
}
