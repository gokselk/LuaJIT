//! Intermediate Representation for the JIT.
//!
//! This defines an SSA-form IR that is generated from bytecode traces
//! and then lowered to Cranelift IR for code generation.

use crate::value::Value;
use crate::bytecode::Instruction;
use super::trace::IrCmp;

/// IR types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrType {
    Unknown,
    Nil,
    Boolean,
    Integer,
    Number,
    String,
    Table,
    Function,
    Userdata,
    Thread,
}

impl IrType {
    /// Infer type from a Lua value
    pub fn from_value(value: &Value) -> Self {
        if value.is_nil() {
            IrType::Nil
        } else if value.is_boolean() {
            IrType::Boolean
        } else if value.is_integer() {
            IrType::Integer
        } else if value.is_number() {
            IrType::Number
        } else if value.is_string() {
            IrType::String
        } else if value.is_table() {
            IrType::Table
        } else if value.is_function() {
            IrType::Function
        } else if value.is_userdata() {
            IrType::Userdata
        } else {
            IrType::Unknown
        }
    }

    /// Check if this type is numeric
    pub fn is_numeric(&self) -> bool {
        matches!(self, IrType::Integer | IrType::Number)
    }
}

/// IR reference (index into instruction list)
pub type IrRef = usize;

/// Constant pool entry
#[derive(Debug, Clone)]
pub enum IrConst {
    Nil,
    Bool(bool),
    Int(i32),
    Num(f64),
    Str(String),
}

/// IR instruction
#[derive(Debug, Clone)]
pub enum IrInstruction {
    // Constants
    Const(IrRef, IrConst),

    // Stack slot operations
    Load(IrRef, usize),           // Load from stack slot
    Store(usize, IrRef),          // Store to stack slot

    // Arithmetic
    Add(IrRef, IrRef, IrRef),     // result = a + b
    Sub(IrRef, IrRef, IrRef),     // result = a - b
    Mul(IrRef, IrRef, IrRef),     // result = a * b
    Div(IrRef, IrRef, IrRef),     // result = a / b
    Mod(IrRef, IrRef, IrRef),     // result = a % b
    Pow(IrRef, IrRef, IrRef),     // result = a ^ b
    Neg(IrRef, IrRef),            // result = -a

    // Comparisons
    Lt(IrRef, IrRef, IrRef),      // result = a < b
    Le(IrRef, IrRef, IrRef),      // result = a <= b
    Eq(IrRef, IrRef, IrRef),      // result = a == b

    // Logic
    Not(IrRef, IrRef),            // result = not a
    And(IrRef, IrRef, IrRef),     // result = a and b
    Or(IrRef, IrRef, IrRef),      // result = a or b

    // Type operations
    IsNil(IrRef, IrRef),          // result = a == nil
    IsNum(IrRef, IrRef),          // result = isnumber(a)
    IsStr(IrRef, IrRef),          // result = isstring(a)
    ToNum(IrRef, IrRef),          // result = tonumber(a)
    ToStr(IrRef, IrRef),          // result = tostring(a)

    // Guards (emit side exit if fails)
    GuardType(IrRef, IrType, usize),  // Guard that ref has type, exit to snapshot
    GuardTrue(IrRef, usize),          // Guard that ref is truthy
    GuardFalse(IrRef, usize),         // Guard that ref is falsy
    GuardNotNil(IrRef, usize),        // Guard that ref is not nil
    GuardNum(IrRef, usize),           // Guard that ref is a number
    GuardInt(IrRef, usize),           // Guard that ref is an integer
    GuardEq(IrRef, IrRef, usize),     // Guard that refs are equal

    // Table operations
    TableNew(IrRef, usize, usize),    // result = new table(asize, hsize)
    TableGet(IrRef, IrRef, IrRef),    // result = table[key]
    TableSet(IrRef, IrRef, IrRef),    // table[key] = value
    TableLen(IrRef, IrRef),           // result = #table

    // String operations
    StrLen(IrRef, IrRef),             // result = #string
    StrConcat(IrRef, IrRef, IrRef),   // result = a .. b

    // Control flow
    Jump(i16),                        // Unconditional jump
    Branch(IrRef, i16, i16),          // if ref then jmp1 else jmp2
    Loop,                             // Loop header marker
    Phi(IrRef, Vec<(usize, IrRef)>),  // SSA phi node

    // For loop
    ForCheck(IrRef, IrRef, IrRef, usize), // Check for loop condition

    // Calls
    Call(IrRef, IrRef, usize, usize), // result = func(args...), nargs, nrets
    TailCall(IrRef, usize),           // return func(args...)
    Return(Option<IrRef>),            // Return from trace

    // Side exits
    Snapshot(usize, Vec<IrRef>),      // Snapshot for exit
    Exit(usize),                      // Exit to interpreter at snapshot

    // Fallback to interpreter
    Fallback(Instruction),            // Execute instruction in interpreter
}

/// IR builder - constructs SSA-form IR
pub struct IrBuilder {
    /// Generated instructions
    instructions: Vec<IrInstruction>,
    /// Current stack slot to IR ref mapping
    slot_map: Vec<Option<IrRef>>,
    /// Next IR reference
    next_ref: IrRef,
    /// Snapshot counter
    snapshot_counter: usize,
}

impl IrBuilder {
    /// Create a new IR builder
    pub fn new() -> Self {
        Self {
            instructions: Vec::with_capacity(256),
            slot_map: vec![None; 256],
            next_ref: 0,
            snapshot_counter: 0,
        }
    }

    /// Allocate a new IR reference
    fn alloc_ref(&mut self) -> IrRef {
        let r = self.next_ref;
        self.next_ref += 1;
        r
    }

    /// Get IR ref for a stack slot
    pub fn slot(&mut self, slot: usize) -> IrRef {
        if let Some(r) = self.slot_map.get(slot).and_then(|r| *r) {
            r
        } else {
            // Emit a load
            let r = self.alloc_ref();
            self.instructions.push(IrInstruction::Load(r, slot));
            if slot < self.slot_map.len() {
                self.slot_map[slot] = Some(r);
            }
            r
        }
    }

    /// Get current IR ref for a slot (without emitting load)
    pub fn current_slot(&self, slot: usize) -> usize {
        self.slot_map.get(slot).and_then(|r| *r).unwrap_or(0)
    }

    /// Set the IR ref for a slot
    fn set_slot(&mut self, slot: usize, r: IrRef) {
        if slot < self.slot_map.len() {
            self.slot_map[slot] = Some(r);
        }
    }

    /// Emit a move instruction
    pub fn emit_move(&mut self, dst: usize, src: IrRef) {
        self.set_slot(dst, src);
        self.instructions.push(IrInstruction::Store(dst, src));
    }

    /// Emit an integer constant
    pub fn emit_const_int(&mut self, dst: usize, value: i32) {
        let r = self.alloc_ref();
        self.instructions.push(IrInstruction::Const(r, IrConst::Int(value)));
        self.set_slot(dst, r);
    }

    /// Emit a number constant
    pub fn emit_const_num(&mut self, dst: usize, value: f64) {
        let r = self.alloc_ref();
        self.instructions.push(IrInstruction::Const(r, IrConst::Num(value)));
        self.set_slot(dst, r);
    }

    /// Emit a primitive constant (nil/false/true)
    pub fn emit_const_pri(&mut self, dst: usize, pri: u8) {
        let r = self.alloc_ref();
        let c = match pri {
            0 => IrConst::Nil,
            1 => IrConst::Bool(false),
            2 => IrConst::Bool(true),
            _ => IrConst::Nil,
        };
        self.instructions.push(IrInstruction::Const(r, c));
        self.set_slot(dst, r);
    }

    /// Emit a number type guard
    pub fn emit_guard_num(&mut self, r: IrRef) {
        let snap = self.snapshot_counter;
        self.instructions.push(IrInstruction::GuardNum(r, snap));
    }

    /// Emit add instruction
    pub fn emit_add(&mut self, dst: usize, a: IrRef, b: IrRef) {
        let r = self.alloc_ref();
        self.instructions.push(IrInstruction::Add(r, a, b));
        self.set_slot(dst, r);
    }

    /// Emit sub instruction
    pub fn emit_sub(&mut self, dst: usize, a: IrRef, b: IrRef) {
        let r = self.alloc_ref();
        self.instructions.push(IrInstruction::Sub(r, a, b));
        self.set_slot(dst, r);
    }

    /// Emit mul instruction
    pub fn emit_mul(&mut self, dst: usize, a: IrRef, b: IrRef) {
        let r = self.alloc_ref();
        self.instructions.push(IrInstruction::Mul(r, a, b));
        self.set_slot(dst, r);
    }

    /// Emit div instruction
    pub fn emit_div(&mut self, dst: usize, a: IrRef, b: IrRef) {
        let r = self.alloc_ref();
        self.instructions.push(IrInstruction::Div(r, a, b));
        self.set_slot(dst, r);
    }

    /// Emit negation
    pub fn emit_neg(&mut self, dst: usize, src: IrRef) {
        let r = self.alloc_ref();
        self.instructions.push(IrInstruction::Neg(r, src));
        self.set_slot(dst, r);
    }

    /// Emit comparison
    pub fn emit_compare(&mut self, a: IrRef, b: IrRef, cmp: IrCmp) {
        let r = self.alloc_ref();
        let instr = match cmp {
            IrCmp::Lt | IrCmp::Gt => IrInstruction::Lt(r, a, b),
            IrCmp::Le | IrCmp::Ge => IrInstruction::Le(r, a, b),
            IrCmp::Eq => IrInstruction::Eq(r, a, b),
            IrCmp::Ne => {
                self.instructions.push(IrInstruction::Eq(r, a, b));
                let r2 = self.alloc_ref();
                IrInstruction::Not(r2, r)
            }
        };
        self.instructions.push(instr);
    }

    /// Emit jump
    pub fn emit_jump(&mut self, offset: i16) {
        self.instructions.push(IrInstruction::Jump(offset));
    }

    /// Emit for loop check
    pub fn emit_for_check(&mut self, idx: usize, limit: usize, step: usize) {
        let idx_ref = self.slot(idx);
        let limit_ref = self.slot(limit);
        let step_ref = self.slot(step);
        let snap = self.snapshot_counter;
        self.instructions.push(IrInstruction::ForCheck(idx_ref, limit_ref, step_ref, snap));
    }

    /// Emit loop header
    pub fn emit_loop(&mut self) {
        self.instructions.push(IrInstruction::Loop);
    }

    /// Emit return
    pub fn emit_return(&mut self) {
        self.instructions.push(IrInstruction::Return(None));
    }

    /// Emit fallback
    pub fn emit_fallback(&mut self, instr: Instruction) {
        self.instructions.push(IrInstruction::Fallback(instr));
    }

    /// Take a snapshot
    pub fn take_snapshot(&mut self, slots: &[IrRef]) -> usize {
        let snap_id = self.snapshot_counter;
        self.instructions.push(IrInstruction::Snapshot(snap_id, slots.to_vec()));
        self.snapshot_counter += 1;
        snap_id
    }

    /// Finish building and return instructions
    pub fn finish(self) -> Vec<IrInstruction> {
        self.instructions
    }
}

impl Default for IrBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ir_builder() {
        let mut builder = IrBuilder::new();

        builder.emit_const_int(0, 42);
        builder.emit_const_int(1, 10);

        let a = builder.slot(0);
        let b = builder.slot(1);
        builder.emit_add(2, a, b);

        let ir = builder.finish();
        assert!(!ir.is_empty());
    }

    #[test]
    fn test_type_inference() {
        assert_eq!(IrType::from_value(&Value::nil()), IrType::Nil);
        assert_eq!(IrType::from_value(&Value::boolean(true)), IrType::Boolean);
        assert_eq!(IrType::from_value(&Value::integer(42)), IrType::Integer);
        assert_eq!(IrType::from_value(&Value::number(3.14)), IrType::Number);
    }
}
