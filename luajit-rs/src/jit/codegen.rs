//! Code generation using Cranelift.
//!
//! This module translates our JIT IR to Cranelift IR and compiles it to machine code.

use super::ir::{IrInstruction, IrConst, IrType, IrRef};
use super::TraceExit;
use crate::value::{LuaResult, LuaError, Value};

use cranelift::prelude::*;
use cranelift_codegen::ir::{Function, InstBuilder, MemFlags};
use cranelift_codegen::isa::TargetIsa;
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{Module, Linkage};
use cranelift_jit::{JITModule, JITBuilder};
use std::collections::HashMap;
use std::sync::Arc;

/// Value representation in generated code
/// We use 64-bit NaN-boxing matching our Value type
const VALUE_TYPE: Type = types::I64;

/// Code generator
pub struct CodeGenerator {
    /// Target ISA
    isa: Arc<dyn TargetIsa>,
    /// JIT module
    module: JITModule,
    /// Function builder context
    builder_ctx: FunctionBuilderContext,
    /// IR ref to Cranelift value mapping
    value_map: HashMap<IrRef, Value>,
    /// Exit points
    exits: Vec<TraceExit>,
}

impl CodeGenerator {
    /// Create a new code generator
    pub fn new(isa: Arc<dyn TargetIsa>) -> Self {
        let builder = JITBuilder::with_isa(
            isa.clone(),
            cranelift_module::default_libcall_names(),
        );
        let module = JITModule::new(builder);

        Self {
            isa,
            module,
            builder_ctx: FunctionBuilderContext::new(),
            value_map: HashMap::new(),
            exits: Vec::new(),
        }
    }

    /// Compile IR to native code
    pub fn compile(&mut self, ir: &[IrInstruction]) -> LuaResult<(Box<[u8]>, Vec<TraceExit>)> {
        self.value_map.clear();
        let mut exits = Vec::new();

        // Create function signature
        // fn trace(stack: *mut Value) -> i32 (exit number or -1 for normal return)
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(self.module.target_config().pointer_type()));
        sig.returns.push(AbiParam::new(types::I32));

        // Declare function
        let func_id = self.module
            .declare_function("trace", Linkage::Local, &sig)
            .map_err(|e| LuaError::RuntimeError(format!("Cranelift declare error: {}", e)))?;

        // Create function
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        // Build function body
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut self.builder_ctx);

            // Create entry block
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            // Get stack pointer parameter
            let stack_ptr = builder.block_params(entry_block)[0];

            // Compile IR instructions
            Self::compile_instructions(&mut builder, ir, stack_ptr, &mut exits)?;

            // If we reach here without exiting, return -1
            let neg_one = builder.ins().iconst(types::I32, -1);
            builder.ins().return_(&[neg_one]);

            builder.finalize();
        }

        // Compile the function
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| LuaError::RuntimeError(format!("Cranelift define error: {}", e)))?;

        self.module.clear_context(&mut ctx);

        // Finalize and get code
        self.module
            .finalize_definitions()
            .map_err(|e| LuaError::RuntimeError(format!("Cranelift finalize error: {}", e)))?;

        let code = self.module.get_finalized_function(func_id);

        // Copy code to boxed slice
        let code_ptr = code as *const u8;
        let code_len = 0; // Would need to track actual size
        let code_slice = unsafe { std::slice::from_raw_parts(code_ptr, code_len) };
        let code_box = code_slice.to_vec().into_boxed_slice();

        Ok((code_box, exits))
    }

    /// Compile IR instructions
    fn compile_instructions(
        builder: &mut FunctionBuilder,
        ir: &[IrInstruction],
        stack_ptr: cranelift::prelude::Value,
        exits: &mut Vec<TraceExit>,
    ) -> LuaResult<()> {
        let mut ir_values: HashMap<IrRef, cranelift::prelude::Value> = HashMap::new();

        for instr in ir {
            match instr {
                IrInstruction::Const(r, c) => {
                    let val = match c {
                        IrConst::Nil => {
                            // NaN-box nil value
                            let nil_bits = Value::nil().raw_bits();
                            builder.ins().iconst(VALUE_TYPE, nil_bits as i64)
                        }
                        IrConst::Bool(b) => {
                            let val = Value::boolean(*b);
                            builder.ins().iconst(VALUE_TYPE, val.raw_bits() as i64)
                        }
                        IrConst::Int(i) => {
                            let val = Value::integer(*i);
                            builder.ins().iconst(VALUE_TYPE, val.raw_bits() as i64)
                        }
                        IrConst::Num(n) => {
                            // Convert to bits
                            let bits = n.to_bits();
                            builder.ins().iconst(VALUE_TYPE, bits as i64)
                        }
                        IrConst::Str(_s) => {
                            // String constant - would need string interning
                            builder.ins().iconst(VALUE_TYPE, 0)
                        }
                    };
                    ir_values.insert(*r, val);
                }

                IrInstruction::Load(r, slot) => {
                    // Load from stack: stack_ptr + slot * 8
                    let offset = (*slot as i32) * 8;
                    let val = builder.ins().load(
                        VALUE_TYPE,
                        MemFlags::trusted(),
                        stack_ptr,
                        offset,
                    );
                    ir_values.insert(*r, val);
                }

                IrInstruction::Store(slot, src_ref) => {
                    if let Some(&src) = ir_values.get(src_ref) {
                        let offset = (*slot as i32) * 8;
                        builder.ins().store(MemFlags::trusted(), src, stack_ptr, offset);
                    }
                }

                IrInstruction::Add(r, a, b) => {
                    if let (Some(&va), Some(&vb)) = (ir_values.get(a), ir_values.get(b)) {
                        // Unbox numbers, add, rebox
                        // For simplicity, assume both are floats
                        let fa = builder.ins().bitcast(types::F64, MemFlags::new(), va);
                        let fb = builder.ins().bitcast(types::F64, MemFlags::new(), vb);
                        let result = builder.ins().fadd(fa, fb);
                        let boxed = builder.ins().bitcast(VALUE_TYPE, MemFlags::new(), result);
                        ir_values.insert(*r, boxed);
                    }
                }

                IrInstruction::Sub(r, a, b) => {
                    if let (Some(&va), Some(&vb)) = (ir_values.get(a), ir_values.get(b)) {
                        let fa = builder.ins().bitcast(types::F64, MemFlags::new(), va);
                        let fb = builder.ins().bitcast(types::F64, MemFlags::new(), vb);
                        let result = builder.ins().fsub(fa, fb);
                        let boxed = builder.ins().bitcast(VALUE_TYPE, MemFlags::new(), result);
                        ir_values.insert(*r, boxed);
                    }
                }

                IrInstruction::Mul(r, a, b) => {
                    if let (Some(&va), Some(&vb)) = (ir_values.get(a), ir_values.get(b)) {
                        let fa = builder.ins().bitcast(types::F64, MemFlags::new(), va);
                        let fb = builder.ins().bitcast(types::F64, MemFlags::new(), vb);
                        let result = builder.ins().fmul(fa, fb);
                        let boxed = builder.ins().bitcast(VALUE_TYPE, MemFlags::new(), result);
                        ir_values.insert(*r, boxed);
                    }
                }

                IrInstruction::Div(r, a, b) => {
                    if let (Some(&va), Some(&vb)) = (ir_values.get(a), ir_values.get(b)) {
                        let fa = builder.ins().bitcast(types::F64, MemFlags::new(), va);
                        let fb = builder.ins().bitcast(types::F64, MemFlags::new(), vb);
                        let result = builder.ins().fdiv(fa, fb);
                        let boxed = builder.ins().bitcast(VALUE_TYPE, MemFlags::new(), result);
                        ir_values.insert(*r, boxed);
                    }
                }

                IrInstruction::Neg(r, a) => {
                    if let Some(&va) = ir_values.get(a) {
                        let fa = builder.ins().bitcast(types::F64, MemFlags::new(), va);
                        let result = builder.ins().fneg(fa);
                        let boxed = builder.ins().bitcast(VALUE_TYPE, MemFlags::new(), result);
                        ir_values.insert(*r, boxed);
                    }
                }

                IrInstruction::Lt(r, a, b) => {
                    if let (Some(&va), Some(&vb)) = (ir_values.get(a), ir_values.get(b)) {
                        let fa = builder.ins().bitcast(types::F64, MemFlags::new(), va);
                        let fb = builder.ins().bitcast(types::F64, MemFlags::new(), vb);
                        let cmp = builder.ins().fcmp(FloatCC::LessThan, fa, fb);
                        // Convert to Lua boolean
                        let true_val = builder.ins().iconst(VALUE_TYPE, Value::boolean(true).raw_bits() as i64);
                        let false_val = builder.ins().iconst(VALUE_TYPE, Value::boolean(false).raw_bits() as i64);
                        let result = builder.ins().select(cmp, true_val, false_val);
                        ir_values.insert(*r, result);
                    }
                }

                IrInstruction::Le(r, a, b) => {
                    if let (Some(&va), Some(&vb)) = (ir_values.get(a), ir_values.get(b)) {
                        let fa = builder.ins().bitcast(types::F64, MemFlags::new(), va);
                        let fb = builder.ins().bitcast(types::F64, MemFlags::new(), vb);
                        let cmp = builder.ins().fcmp(FloatCC::LessThanOrEqual, fa, fb);
                        let true_val = builder.ins().iconst(VALUE_TYPE, Value::boolean(true).raw_bits() as i64);
                        let false_val = builder.ins().iconst(VALUE_TYPE, Value::boolean(false).raw_bits() as i64);
                        let result = builder.ins().select(cmp, true_val, false_val);
                        ir_values.insert(*r, result);
                    }
                }

                IrInstruction::GuardNum(r, exit_id) => {
                    if let Some(&v) = ir_values.get(r) {
                        // Check if value is a number (NaN-box check)
                        // A value is NOT a number if bits match QNAN_BASE | TAG pattern
                        let qnan_base = 0x7FF8_0000_0000_0000u64;
                        let check_mask = builder.ins().iconst(VALUE_TYPE, qnan_base as i64);
                        let masked = builder.ins().band(v, check_mask);
                        let is_tagged = builder.ins().icmp(IntCC::Equal, masked, check_mask);

                        // Create exit block
                        let exit_block = builder.create_block();
                        let continue_block = builder.create_block();

                        builder.ins().brif(is_tagged, exit_block, &[], continue_block, &[]);

                        // Exit block
                        builder.switch_to_block(exit_block);
                        builder.seal_block(exit_block);
                        let exit_num = builder.ins().iconst(types::I32, *exit_id as i64);
                        builder.ins().return_(&[exit_num]);

                        // Continue block
                        builder.switch_to_block(continue_block);
                        builder.seal_block(continue_block);

                        // Track exit
                        exits.push(TraceExit {
                            id: *exit_id,
                            target_pc: 0, // Would be filled in from snapshot
                            snapshot: Vec::new(),
                        });
                    }
                }

                IrInstruction::GuardType(r, ty, exit_id) => {
                    // Type-specific guard
                    // Similar to GuardNum but check specific type tag
                    if let Some(&v) = ir_values.get(r) {
                        let exit_block = builder.create_block();
                        let continue_block = builder.create_block();

                        // For now, just always continue (placeholder)
                        builder.ins().jump(continue_block, &[]);

                        builder.switch_to_block(exit_block);
                        builder.seal_block(exit_block);
                        let exit_num = builder.ins().iconst(types::I32, *exit_id as i64);
                        builder.ins().return_(&[exit_num]);

                        builder.switch_to_block(continue_block);
                        builder.seal_block(continue_block);
                    }
                }

                IrInstruction::ForCheck(idx, limit, step, exit_id) => {
                    if let (Some(&vidx), Some(&vlimit), Some(&vstep)) =
                        (ir_values.get(idx), ir_values.get(limit), ir_values.get(step))
                    {
                        // Unbox as floats
                        let fidx = builder.ins().bitcast(types::F64, MemFlags::new(), vidx);
                        let flimit = builder.ins().bitcast(types::F64, MemFlags::new(), vlimit);
                        let fstep = builder.ins().bitcast(types::F64, MemFlags::new(), vstep);

                        // Check if step >= 0
                        let zero = builder.ins().f64const(0.0);
                        let step_positive = builder.ins().fcmp(FloatCC::GreaterThanOrEqual, fstep, zero);

                        // If step >= 0: continue if idx <= limit
                        // If step < 0: continue if idx >= limit
                        let cmp_le = builder.ins().fcmp(FloatCC::LessThanOrEqual, fidx, flimit);
                        let cmp_ge = builder.ins().fcmp(FloatCC::GreaterThanOrEqual, fidx, flimit);
                        let should_continue = builder.ins().select(step_positive, cmp_le, cmp_ge);

                        let exit_block = builder.create_block();
                        let continue_block = builder.create_block();

                        builder.ins().brif(should_continue, continue_block, &[], exit_block, &[]);

                        builder.switch_to_block(exit_block);
                        builder.seal_block(exit_block);
                        let exit_num = builder.ins().iconst(types::I32, *exit_id as i64);
                        builder.ins().return_(&[exit_num]);

                        builder.switch_to_block(continue_block);
                        builder.seal_block(continue_block);
                    }
                }

                IrInstruction::Loop => {
                    // Loop header - create a new block for the loop
                    let loop_block = builder.create_block();
                    builder.ins().jump(loop_block, &[]);
                    builder.switch_to_block(loop_block);
                    // Don't seal - back edge will come later
                }

                IrInstruction::Jump(offset) => {
                    // For now, just continue (actual jump handling would need block structure)
                }

                IrInstruction::Return(val) => {
                    let ret_val = builder.ins().iconst(types::I32, -1);
                    builder.ins().return_(&[ret_val]);
                }

                IrInstruction::Exit(exit_id) => {
                    let exit_num = builder.ins().iconst(types::I32, *exit_id as i64);
                    builder.ins().return_(&[exit_num]);
                }

                IrInstruction::Fallback(_instr) => {
                    // Emit exit to interpreter
                    let exit_num = builder.ins().iconst(types::I32, -2);
                    builder.ins().return_(&[exit_num]);
                }

                // Other instructions not yet implemented
                _ => {}
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codegen_creation() {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed").unwrap();
        let flags = settings::Flags::new(flag_builder);

        let isa = cranelift_native::builder()
            .unwrap()
            .finish(flags)
            .unwrap();

        let _codegen = CodeGenerator::new(isa);
    }
}
