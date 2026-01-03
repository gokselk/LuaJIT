//! Bytecode format and instruction encoding.
//!
//! This module defines the bytecode instruction format similar to LuaJIT.
//! Instructions are 32-bit values in either ABC or AD format.

mod opcodes;

pub use opcodes::{Opcode, OpMode};

/// A bytecode instruction (32-bit).
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Instruction(pub u32);

impl Instruction {
    // Bit positions and masks
    const OP_SHIFT: u32 = 0;
    const OP_MASK: u32 = 0xFF;
    const A_SHIFT: u32 = 8;
    const A_MASK: u32 = 0xFF;
    const B_SHIFT: u32 = 24;
    const B_MASK: u32 = 0xFF;
    const C_SHIFT: u32 = 16;
    const C_MASK: u32 = 0xFF;
    const D_SHIFT: u32 = 16;
    const D_MASK: u32 = 0xFFFF;

    /// Create an ABC-format instruction
    #[inline]
    pub const fn abc(op: Opcode, a: u8, b: u8, c: u8) -> Self {
        Self(
            (op as u32) |
            ((a as u32) << Self::A_SHIFT) |
            ((c as u32) << Self::C_SHIFT) |
            ((b as u32) << Self::B_SHIFT)
        )
    }

    /// Create an AD-format instruction
    #[inline]
    pub const fn ad(op: Opcode, a: u8, d: u16) -> Self {
        Self(
            (op as u32) |
            ((a as u32) << Self::A_SHIFT) |
            ((d as u32) << Self::D_SHIFT)
        )
    }

    /// Create an AD-format instruction with signed D
    #[inline]
    pub const fn adj(op: Opcode, a: u8, d: i16) -> Self {
        Self::ad(op, a, d as u16)
    }

    /// Get the opcode
    #[inline]
    pub const fn opcode(&self) -> Opcode {
        unsafe { std::mem::transmute((self.0 & Self::OP_MASK) as u8) }
    }

    /// Get the A field (8 bits)
    #[inline]
    pub const fn a(&self) -> u8 {
        ((self.0 >> Self::A_SHIFT) & Self::A_MASK) as u8
    }

    /// Get the B field (8 bits)
    #[inline]
    pub const fn b(&self) -> u8 {
        ((self.0 >> Self::B_SHIFT) & Self::B_MASK) as u8
    }

    /// Get the C field (8 bits)
    #[inline]
    pub const fn c(&self) -> u8 {
        ((self.0 >> Self::C_SHIFT) & Self::C_MASK) as u8
    }

    /// Get the D field (16 bits unsigned)
    #[inline]
    pub const fn d(&self) -> u16 {
        ((self.0 >> Self::D_SHIFT) & Self::D_MASK) as u16
    }

    /// Get the D field as signed jump offset
    #[inline]
    pub const fn jump(&self) -> i16 {
        self.d() as i16
    }

    /// Get raw instruction value
    #[inline]
    pub const fn raw(&self) -> u32 {
        self.0
    }

    /// Set the A field
    #[inline]
    pub fn set_a(&mut self, a: u8) {
        self.0 = (self.0 & !(Self::A_MASK << Self::A_SHIFT)) | ((a as u32) << Self::A_SHIFT);
    }

    /// Set the B field
    #[inline]
    pub fn set_b(&mut self, b: u8) {
        self.0 = (self.0 & !(Self::B_MASK << Self::B_SHIFT)) | ((b as u32) << Self::B_SHIFT);
    }

    /// Set the C field
    #[inline]
    pub fn set_c(&mut self, c: u8) {
        self.0 = (self.0 & !(Self::C_MASK << Self::C_SHIFT)) | ((c as u32) << Self::C_SHIFT);
    }

    /// Set the D field
    #[inline]
    pub fn set_d(&mut self, d: u16) {
        self.0 = (self.0 & !(Self::D_MASK << Self::D_SHIFT)) | ((d as u32) << Self::D_SHIFT);
    }

    /// Check if operand is a constant index (high bit set in B/C)
    #[inline]
    pub const fn is_k_b(&self) -> bool {
        (self.0 >> Self::B_SHIFT) & 0x80 != 0
    }

    #[inline]
    pub const fn is_k_c(&self) -> bool {
        (self.0 >> Self::C_SHIFT) & 0x80 != 0
    }

    /// Get constant index from B (strip high bit)
    #[inline]
    pub const fn kb(&self) -> u8 {
        self.b() & 0x7F
    }

    /// Get constant index from C (strip high bit)
    #[inline]
    pub const fn kc(&self) -> u8 {
        self.c() & 0x7F
    }
}

impl std::fmt::Debug for Instruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let op = self.opcode();
        match op.mode() {
            OpMode::ABC => {
                write!(f, "{:?} A={} B={} C={}", op, self.a(), self.b(), self.c())
            }
            OpMode::AD | OpMode::AJ => {
                write!(f, "{:?} A={} D={}", op, self.a(), self.d())
            }
        }
    }
}

impl std::fmt::Display for Instruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let op = self.opcode();
        write!(f, "{}", op.name())?;

        match op.mode() {
            OpMode::ABC => {
                let b = if self.is_k_b() {
                    format!("K{}", self.kb())
                } else {
                    format!("R{}", self.b())
                };
                let c = if self.is_k_c() {
                    format!("K{}", self.kc())
                } else {
                    format!("R{}", self.c())
                };
                write!(f, " R{} {} {}", self.a(), b, c)
            }
            OpMode::AD => {
                write!(f, " R{} {}", self.a(), self.d())
            }
            OpMode::AJ => {
                write!(f, " R{} => {}", self.a(), self.jump())
            }
        }
    }
}

/// A basic block of bytecode
#[derive(Debug, Clone)]
pub struct BasicBlock {
    /// Starting PC of this block
    pub start: usize,
    /// Instructions in this block
    pub instructions: Vec<Instruction>,
    /// Successor blocks (for control flow)
    pub successors: Vec<usize>,
    /// Is this block a loop header?
    pub is_loop_header: bool,
}

impl BasicBlock {
    pub fn new(start: usize) -> Self {
        Self {
            start,
            instructions: Vec::new(),
            successors: Vec::new(),
            is_loop_header: false,
        }
    }

    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    pub fn end(&self) -> usize {
        self.start + self.instructions.len()
    }
}

/// Control flow graph for a function
#[derive(Debug)]
pub struct ControlFlowGraph {
    pub blocks: Vec<BasicBlock>,
    pub entry: usize,
}

impl ControlFlowGraph {
    /// Build CFG from bytecode
    pub fn build(code: &[Instruction]) -> Self {
        if code.is_empty() {
            return Self {
                blocks: vec![BasicBlock::new(0)],
                entry: 0,
            };
        }

        // Find block boundaries
        let mut leaders = vec![false; code.len()];
        leaders[0] = true; // First instruction is a leader

        for (i, instr) in code.iter().enumerate() {
            let op = instr.opcode();
            if op.is_jump() {
                let target = (i as i32 + 1 + instr.jump() as i32) as usize;
                if target < code.len() {
                    leaders[target] = true;
                }
                // Instruction after jump is also a leader
                if i + 1 < code.len() {
                    leaders[i + 1] = true;
                }
            } else if op.is_return() && i + 1 < code.len() {
                leaders[i + 1] = true;
            }
        }

        // Build blocks
        let mut blocks = Vec::new();
        let mut current_start = 0;

        for i in 0..=code.len() {
            if i == code.len() || (i > 0 && leaders[i]) {
                if i > current_start {
                    let mut block = BasicBlock::new(current_start);
                    block.instructions = code[current_start..i].to_vec();

                    // Add successors
                    if let Some(last) = block.instructions.last() {
                        let last_op = last.opcode();
                        if last_op.is_jump() {
                            let target = (i as i32 - 1 + 1 + last.jump() as i32) as usize;
                            block.successors.push(target);
                            // Conditional jumps also fall through
                            if last_op.is_conditional() && i < code.len() {
                                block.successors.push(i);
                            }
                        } else if !last_op.is_return() && i < code.len() {
                            block.successors.push(i);
                        }
                    }

                    blocks.push(block);
                }
                current_start = i;
            }
        }

        // Mark loop headers (simplified detection)
        // First collect which blocks should be marked as loop headers
        let mut loop_headers: Vec<usize> = Vec::new();
        for i in 0..blocks.len() {
            for &succ in &blocks[i].successors {
                // Back edge: successor is before current block
                if succ <= blocks[i].start {
                    loop_headers.push(succ);
                }
            }
        }
        // Then mark them
        for header in loop_headers {
            for block in &mut blocks {
                if block.start == header {
                    block.is_loop_header = true;
                    break;
                }
            }
        }

        Self { blocks, entry: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instruction_encoding() {
        let instr = Instruction::abc(Opcode::ADDVV, 0, 1, 2);
        assert_eq!(instr.opcode(), Opcode::ADDVV);
        assert_eq!(instr.a(), 0);
        assert_eq!(instr.b(), 1);
        assert_eq!(instr.c(), 2);

        let instr2 = Instruction::ad(Opcode::JMP, 0, 10);
        assert_eq!(instr2.opcode(), Opcode::JMP);
        assert_eq!(instr2.a(), 0);
        assert_eq!(instr2.d(), 10);
    }

    #[test]
    fn test_jump_encoding() {
        let instr = Instruction::adj(Opcode::JMP, 0, -5);
        assert_eq!(instr.jump(), -5);

        let instr2 = Instruction::adj(Opcode::JMP, 0, 100);
        assert_eq!(instr2.jump(), 100);
    }

    #[test]
    fn test_constant_flag() {
        let instr = Instruction::abc(Opcode::ADDVN, 0, 1, 0x80); // C is constant
        assert!(!instr.is_k_b());
        assert!(instr.is_k_c());
        assert_eq!(instr.kc(), 0);
    }
}
