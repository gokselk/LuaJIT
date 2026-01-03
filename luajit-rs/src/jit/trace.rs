//! Trace recording for the JIT compiler.
//!
//! Records a sequence of bytecode operations with type information
//! to build an SSA-form IR for compilation.

use super::{IrBuilder, IrInstruction, IrType, SnapshotEntry, MAX_TRACE_LENGTH};
use crate::value::{Value, LuaResult, LuaError, GcRef, Proto};
use crate::bytecode::{Instruction, Opcode};
use smallvec::SmallVec;

/// State of trace recording
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceState {
    /// Not currently recording
    NotRecording,
    /// Recording in progress
    Recording,
    /// Trace completed (looped back)
    Completed,
    /// Trace aborted
    Aborted,
    /// Maximum length reached
    MaxLength,
}

/// A recorded trace entry
#[derive(Debug, Clone)]
pub struct TraceEntry {
    /// The bytecode instruction
    pub instr: Instruction,
    /// Type information for operands
    pub operand_types: SmallVec<[IrType; 4]>,
    /// Bytecode PC
    pub pc: usize,
}

/// A trace represents a recorded sequence of bytecode
#[derive(Debug)]
pub struct Trace {
    /// Trace entries
    pub entries: Vec<TraceEntry>,
    /// Starting PC
    pub start_pc: usize,
    /// Loop target (if this trace loops)
    pub loop_target: Option<usize>,
}

/// The trace recorder
pub struct TraceRecorder {
    /// Trace ID
    pub id: usize,
    /// Starting bytecode PC
    pub start_pc: usize,
    /// Current PC
    pub current_pc: usize,
    /// The prototype being traced
    pub proto: GcRef<Proto>,
    /// Recorded entries
    pub entries: Vec<TraceEntry>,
    /// Stack slot types (for type guards)
    pub slot_types: Vec<IrType>,
    /// Has the trace looped?
    pub looped: bool,
    /// Link to another trace
    pub link_trace: Option<usize>,
    /// Abort reason (if any)
    pub abort_reason: Option<String>,
    /// Snapshots for exit points
    pub snapshots: Vec<Vec<SnapshotEntry>>,
    /// IR builder
    ir_builder: IrBuilder,
}

impl TraceRecorder {
    /// Create a new trace recorder
    pub fn new(id: usize, start_pc: usize, proto: GcRef<Proto>) -> Self {
        Self {
            id,
            start_pc,
            current_pc: start_pc,
            proto,
            entries: Vec::with_capacity(256),
            slot_types: vec![IrType::Unknown; 256],
            looped: false,
            link_trace: None,
            abort_reason: None,
            snapshots: Vec::new(),
            ir_builder: IrBuilder::new(),
        }
    }

    /// Record a bytecode instruction
    pub fn record(&mut self, instr: Instruction, stack: &[Value]) -> TraceState {
        // Check for abort conditions
        if self.entries.len() >= MAX_TRACE_LENGTH {
            self.abort_reason = Some("trace too long".to_string());
            return TraceState::MaxLength;
        }

        // Analyze the instruction and record type info
        let op = instr.opcode();

        // Check for loop completion
        if self.current_pc == self.start_pc && !self.entries.is_empty() {
            self.looped = true;
            return TraceState::Completed;
        }

        // Check for blacklisted operations
        if self.should_abort(op) {
            self.abort_reason = Some(format!("cannot trace {:?}", op));
            return TraceState::Aborted;
        }

        // Record operand types from stack
        let operand_types = self.record_types(instr, stack);

        // Add entry
        self.entries.push(TraceEntry {
            instr,
            operand_types,
            pc: self.current_pc,
        });

        // Update current PC
        self.current_pc += 1;
        if op.is_jump() {
            self.current_pc = ((self.current_pc as i32) + (instr.jump() as i32)) as usize;
        }

        // Build IR for this instruction
        self.build_ir_for(instr, stack);

        TraceState::Recording
    }

    /// Check if we should abort on this opcode
    fn should_abort(&self, op: Opcode) -> bool {
        matches!(
            op,
            // Cannot trace these (require interpreter features)
            Opcode::FUNCC | Opcode::FUNCCW |
            // Complex control flow
            Opcode::CALLT | Opcode::CALLMT |
            // Iterator operations (need special handling)
            Opcode::ITERN | Opcode::ISNEXT
        )
    }

    /// Record type information for instruction operands
    fn record_types(&mut self, instr: Instruction, stack: &[Value]) -> SmallVec<[IrType; 4]> {
        let mut types = SmallVec::new();
        let op = instr.opcode();

        match op {
            // Arithmetic ops: check operand types
            Opcode::ADDVV | Opcode::SUBVV | Opcode::MULVV |
            Opcode::DIVVV | Opcode::MODVV | Opcode::POW => {
                let b = instr.b() as usize;
                let c = instr.c() as usize;
                if b < stack.len() {
                    types.push(IrType::from_value(&stack[b]));
                }
                if c < stack.len() {
                    types.push(IrType::from_value(&stack[c]));
                }
            }

            // Unary ops
            Opcode::UNM | Opcode::NOT | Opcode::LEN => {
                let d = instr.d() as usize;
                if d < stack.len() {
                    types.push(IrType::from_value(&stack[d]));
                }
            }

            // Comparisons
            Opcode::ISLT | Opcode::ISGE | Opcode::ISLE | Opcode::ISGT |
            Opcode::ISEQV | Opcode::ISNEV => {
                let a = instr.a() as usize;
                let d = instr.d() as usize;
                if a < stack.len() {
                    types.push(IrType::from_value(&stack[a]));
                }
                if d < stack.len() {
                    types.push(IrType::from_value(&stack[d]));
                }
            }

            // Table ops
            Opcode::TGETV | Opcode::TGETS | Opcode::TGETB |
            Opcode::TSETV | Opcode::TSETS | Opcode::TSETB => {
                let b = instr.b() as usize;
                if b < stack.len() {
                    types.push(IrType::from_value(&stack[b]));
                }
            }

            _ => {}
        }

        types
    }

    /// Build IR for an instruction
    fn build_ir_for(&mut self, instr: Instruction, stack: &[Value]) {
        let op = instr.opcode();
        let a = instr.a() as usize;

        match op {
            Opcode::MOV => {
                let d = instr.d() as usize;
                let src = self.ir_builder.slot(d);
                self.ir_builder.emit_move(a, src);
            }

            Opcode::KSHORT => {
                let d = instr.d() as i16 as i32;
                self.ir_builder.emit_const_int(a, d);
            }

            Opcode::KNUM => {
                let d = instr.d() as usize;
                // Would load constant from prototype
                self.ir_builder.emit_const_num(a, 0.0);
            }

            Opcode::KPRI => {
                let d = instr.d();
                self.ir_builder.emit_const_pri(a, d as u8);
            }

            Opcode::ADDVV => {
                let b = instr.b() as usize;
                let c = instr.c() as usize;
                let lhs = self.ir_builder.slot(b);
                let rhs = self.ir_builder.slot(c);

                // Emit type guards
                self.ir_builder.emit_guard_num(lhs);
                self.ir_builder.emit_guard_num(rhs);

                self.ir_builder.emit_add(a, lhs, rhs);
            }

            Opcode::SUBVV => {
                let b = instr.b() as usize;
                let c = instr.c() as usize;
                let lhs = self.ir_builder.slot(b);
                let rhs = self.ir_builder.slot(c);

                self.ir_builder.emit_guard_num(lhs);
                self.ir_builder.emit_guard_num(rhs);

                self.ir_builder.emit_sub(a, lhs, rhs);
            }

            Opcode::MULVV => {
                let b = instr.b() as usize;
                let c = instr.c() as usize;
                let lhs = self.ir_builder.slot(b);
                let rhs = self.ir_builder.slot(c);

                self.ir_builder.emit_guard_num(lhs);
                self.ir_builder.emit_guard_num(rhs);

                self.ir_builder.emit_mul(a, lhs, rhs);
            }

            Opcode::DIVVV => {
                let b = instr.b() as usize;
                let c = instr.c() as usize;
                let lhs = self.ir_builder.slot(b);
                let rhs = self.ir_builder.slot(c);

                self.ir_builder.emit_guard_num(lhs);
                self.ir_builder.emit_guard_num(rhs);

                self.ir_builder.emit_div(a, lhs, rhs);
            }

            Opcode::UNM => {
                let d = instr.d() as usize;
                let src = self.ir_builder.slot(d);
                self.ir_builder.emit_guard_num(src);
                self.ir_builder.emit_neg(a, src);
            }

            Opcode::ISLT | Opcode::ISGE | Opcode::ISLE | Opcode::ISGT => {
                let d = instr.d() as usize;
                let lhs = self.ir_builder.slot(a);
                let rhs = self.ir_builder.slot(d);

                self.ir_builder.emit_guard_num(lhs);
                self.ir_builder.emit_guard_num(rhs);

                let cmp_op = match op {
                    Opcode::ISLT => IrCmp::Lt,
                    Opcode::ISGE => IrCmp::Ge,
                    Opcode::ISLE => IrCmp::Le,
                    Opcode::ISGT => IrCmp::Gt,
                    _ => unreachable!(),
                };

                self.ir_builder.emit_compare(lhs, rhs, cmp_op);
            }

            Opcode::JMP => {
                let offset = instr.jump();
                self.ir_builder.emit_jump(offset);
            }

            Opcode::FORL | Opcode::IFORL => {
                // For loop step
                let idx = self.ir_builder.slot(a);
                let limit = self.ir_builder.slot(a + 1);
                let step = self.ir_builder.slot(a + 2);

                // idx = idx + step
                self.ir_builder.emit_add(a, idx, step);

                // Check loop condition
                self.ir_builder.emit_for_check(a, a + 1, a + 2);
            }

            Opcode::LOOP | Opcode::ILOOP => {
                // Loop marker - emit loop header
                self.ir_builder.emit_loop();
            }

            Opcode::RET | Opcode::RET0 | Opcode::RET1 => {
                // Return - emit trace exit
                self.ir_builder.emit_return();
            }

            _ => {
                // Unhandled instruction - emit fallback
                self.ir_builder.emit_fallback(instr);
            }
        }
    }

    /// Build the final IR for compilation
    pub fn build_ir(self) -> LuaResult<Vec<IrInstruction>> {
        Ok(self.ir_builder.finish())
    }

    /// Take a snapshot of current state
    pub fn snapshot(&mut self, slots: &[Value]) {
        let mut entries = Vec::new();
        for (i, val) in slots.iter().enumerate() {
            if i >= 256 {
                break;
            }
            entries.push(SnapshotEntry {
                slot: i as u8,
                value_type: IrType::from_value(val),
                ir_ref: self.ir_builder.current_slot(i),
            });
        }
        self.snapshots.push(entries);
    }
}

/// Comparison operation
#[derive(Debug, Clone, Copy)]
pub enum IrCmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::GarbageCollector;

    #[test]
    fn test_recorder_creation() {
        let mut gc = GarbageCollector::new();
        let proto = gc.alloc(Proto::new());
        let recorder = TraceRecorder::new(0, 0, proto);
        assert_eq!(recorder.start_pc, 0);
        assert!(recorder.entries.is_empty());
    }
}
