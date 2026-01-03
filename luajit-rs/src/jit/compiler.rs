//! JIT compiler - coordinates trace recording and code generation.

use super::{IrInstruction, TraceExit};
use crate::value::{LuaResult, LuaError};

use cranelift_codegen::isa::TargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use std::sync::Arc;

/// The JIT compiler
pub struct JitCompiler {
    /// Cranelift target ISA
    isa: Arc<dyn TargetIsa>,
}

impl JitCompiler {
    /// Create a new JIT compiler
    pub fn new() -> LuaResult<Self> {
        // Configure Cranelift for the host target
        let mut flag_builder = settings::builder();

        // Enable optimizations
        flag_builder.set("opt_level", "speed").map_err(|e| {
            LuaError::RuntimeError(format!("Cranelift config error: {}", e))
        })?;

        // Enable SIMD if available
        flag_builder.set("enable_simd", "true").ok();

        let flags = settings::Flags::new(flag_builder);

        // Get the native target
        let isa = cranelift_native::builder()
            .map_err(|e| LuaError::RuntimeError(format!("Cranelift native error: {}", e)))?
            .finish(flags)
            .map_err(|e| LuaError::RuntimeError(format!("Cranelift ISA error: {}", e)))?;

        Ok(Self { isa })
    }

    /// Compile IR to native code
    pub fn compile(&mut self, ir: &[IrInstruction]) -> LuaResult<(Box<[u8]>, Vec<TraceExit>)> {
        use super::CodeGenerator;

        let mut codegen = CodeGenerator::new(self.isa.clone());
        codegen.compile(ir)
    }

    /// Get the target ISA
    pub fn isa(&self) -> &Arc<dyn TargetIsa> {
        &self.isa
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compiler_creation() {
        let result = JitCompiler::new();
        assert!(result.is_ok());
    }
}
