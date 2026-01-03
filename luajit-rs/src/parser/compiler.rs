//! Lua bytecode compiler.
//!
//! Single-pass compiler that generates bytecode directly from source.

use super::lexer::{Lexer, Token, TokenKind};
use crate::bytecode::{Instruction, Opcode};
use crate::value::{LuaError, LuaResult, Proto, UpvalueDesc, LocVar, Value};

/// Expression description - tracks where an expression value is located
#[derive(Debug, Clone)]
enum ExprDesc {
    /// Nil value
    Nil,
    /// Boolean constant
    Bool(bool),
    /// Numeric constant
    Number(f64),
    /// String constant (index in constants)
    String(usize),
    /// Value in register
    Register(u8),
    /// Value in upvalue
    Upvalue(u8),
    /// Global variable (constant index for name)
    Global(usize),
    /// Table index: table in reg, key in reg or constant
    Index { table: u8, key: u8, key_is_const: bool },
    /// Relocatable expression (instruction index to patch)
    Relocate(usize),
    /// Jump expression (instruction index)
    Jump(usize),
    /// Call expression (base register, number of returns, instruction PC)
    Call(u8, u8, usize),
    /// Vararg expression
    Vararg,
    /// No value (void)
    Void,
}

/// Local variable info during compilation
#[derive(Debug, Clone)]
struct LocalVar {
    name: String,
    slot: u8,
    start_pc: usize,
    is_captured: bool,
}

/// Jump patch info
#[derive(Debug)]
struct JumpPatch {
    pc: usize,
    target: Option<usize>,
}

/// Function state during compilation
struct FunctionState {
    proto: Proto,
    locals: Vec<LocalVar>,
    upvalues: Vec<(String, bool, u8)>, // (name, in_stack, index)
    free_reg: u8,
    num_params: u8,
    is_vararg: bool,
    /// Pending jumps to patch
    pending_jumps: Vec<JumpPatch>,
    /// Loop break jumps to patch
    break_jumps: Vec<usize>,
    /// Loop continue target
    loop_start: Option<usize>,
    /// Block nesting level
    block_level: usize,
}

impl FunctionState {
    fn new() -> Self {
        Self {
            proto: Proto::new(),
            locals: Vec::new(),
            upvalues: Vec::new(),
            free_reg: 0,
            num_params: 0,
            is_vararg: false,
            pending_jumps: Vec::new(),
            break_jumps: Vec::new(),
            loop_start: None,
            block_level: 0,
        }
    }

    fn reserve_reg(&mut self) -> u8 {
        let reg = self.free_reg;
        self.free_reg += 1;
        if self.free_reg > self.proto.max_stack_size {
            self.proto.max_stack_size = self.free_reg;
        }
        reg
    }

    fn free_regs(&mut self, count: u8) {
        self.free_reg = self.free_reg.saturating_sub(count);
    }

    fn emit(&mut self, instr: Instruction, line: u32) -> usize {
        self.proto.add_instruction(instr, line)
    }

    fn add_constant(&mut self, value: Value) -> usize {
        self.proto.add_constant(value)
    }

    fn add_string_constant(&mut self, s: &str) -> usize {
        // Store the actual string in proto.string_constants
        // The interpreter will intern it at runtime
        self.proto.add_string_constant(s)
    }

    fn current_pc(&self) -> usize {
        self.proto.code.len()
    }

    fn find_local(&self, name: &str) -> Option<u8> {
        for local in self.locals.iter().rev() {
            if local.name == name {
                return Some(local.slot);
            }
        }
        None
    }

    fn find_upvalue(&mut self, name: &str, parent: Option<&mut FunctionState>) -> Option<u8> {
        // Check if already captured
        for (i, (uv_name, _, _)) in self.upvalues.iter().enumerate() {
            if uv_name == name {
                return Some(i as u8);
            }
        }

        // Try to capture from parent
        if let Some(parent) = parent {
            // Check parent locals
            if let Some(slot) = parent.find_local(name) {
                // Mark as captured
                for local in &mut parent.locals {
                    if local.name == name {
                        local.is_captured = true;
                    }
                }
                let idx = self.upvalues.len() as u8;
                self.upvalues.push((name.to_string(), true, slot));
                self.proto.upvalues.push(UpvalueDesc {
                    in_stack: true,
                    index: slot,
                    name: None,
                });
                return Some(idx);
            }

            // Check parent upvalues
            if let Some(uv_idx) = parent.find_upvalue(name, None) {
                let idx = self.upvalues.len() as u8;
                self.upvalues.push((name.to_string(), false, uv_idx));
                self.proto.upvalues.push(UpvalueDesc {
                    in_stack: false,
                    index: uv_idx,
                    name: None,
                });
                return Some(idx);
            }
        }

        None
    }
}

/// Bytecode compiler
pub struct Compiler<'a> {
    lexer: Lexer<'a>,
    /// Stack of function states (for nested functions)
    functions: Vec<FunctionState>,
    chunk_name: String,
}

impl<'a> Compiler<'a> {
    pub fn new(lexer: Lexer<'a>, chunk_name: &str) -> Self {
        Self {
            lexer,
            functions: vec![FunctionState::new()],
            chunk_name: chunk_name.to_string(),
        }
    }

    fn fs(&self) -> &FunctionState {
        self.functions.last().unwrap()
    }

    fn fs_mut(&mut self) -> &mut FunctionState {
        self.functions.last_mut().unwrap()
    }

    fn current_line(&self) -> u32 {
        self.lexer.line()
    }

    /// Compile the chunk
    pub fn compile(mut self) -> LuaResult<Proto> {
        // Add implicit _ENV upvalue
        self.fs_mut().upvalues.push(("_ENV".to_string(), true, 0));
        self.fs_mut().proto.upvalues.push(UpvalueDesc {
            in_stack: true,
            index: 0,
            name: None,
        });
        self.fs_mut().proto.num_upvalues = 1;
        self.fs_mut().proto.is_vararg = true;

        // Emit function header
        let line = self.current_line();
        self.fs_mut().emit(Instruction::ad(Opcode::FUNCV, 0, 0), line);

        // Parse statements
        self.parse_block()?;

        // Emit return
        let line = self.current_line();
        self.fs_mut().emit(Instruction::ad(Opcode::RET0, 0, 1), line);

        // Finalize
        let mut fs = self.functions.pop().unwrap();
        fs.proto.num_upvalues = fs.upvalues.len() as u8;

        Ok(fs.proto)
    }

    /// Parse a block of statements
    fn parse_block(&mut self) -> LuaResult<()> {
        self.fs_mut().block_level += 1;
        let initial_locals = self.fs().locals.len();

        loop {
            if self.check_block_end()? {
                break;
            }
            self.parse_statement()?;
        }

        // Close locals in this block
        let locals_to_close = self.fs().locals.len() - initial_locals;
        if locals_to_close > 0 {
            self.close_locals(initial_locals);
        }

        self.fs_mut().block_level -= 1;
        Ok(())
    }

    fn check_block_end(&mut self) -> LuaResult<bool> {
        let kind = &self.lexer.peek()?.kind;
        Ok(matches!(
            kind,
            TokenKind::Else | TokenKind::Elseif | TokenKind::End |
            TokenKind::Until | TokenKind::Eof
        ))
    }

    fn close_locals(&mut self, from: usize) {
        let locals_to_remove: Vec<_> = self.fs().locals[from..].to_vec();
        let has_captured = locals_to_remove.iter().any(|l| l.is_captured);

        if has_captured {
            let first_slot = locals_to_remove.first().map(|l| l.slot).unwrap_or(0);
            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::adj(Opcode::UCLO, first_slot, 0),
                line,
            );
        }

        self.fs_mut().locals.truncate(from);
        if let Some(last) = self.fs().locals.last() {
            self.fs_mut().free_reg = last.slot + 1;
        } else {
            self.fs_mut().free_reg = self.fs().num_params;
        }
    }

    /// Parse a statement
    fn parse_statement(&mut self) -> LuaResult<()> {
        match &self.lexer.peek()?.kind {
            TokenKind::Semicolon => {
                self.lexer.next()?;
            }
            TokenKind::If => self.parse_if()?,
            TokenKind::While => self.parse_while()?,
            TokenKind::Do => self.parse_do()?,
            TokenKind::For => self.parse_for()?,
            TokenKind::Repeat => self.parse_repeat()?,
            TokenKind::Function => self.parse_function_stat()?,
            TokenKind::Local => self.parse_local()?,
            TokenKind::Return => self.parse_return()?,
            TokenKind::Break => self.parse_break()?,
            TokenKind::Goto => self.parse_goto()?,
            TokenKind::ColonColon => self.parse_label()?,
            _ => self.parse_expr_stat()?,
        }
        Ok(())
    }

    /// Parse if statement
    fn parse_if(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'if'
        let mut end_jumps = Vec::new();

        loop {
            // Parse condition
            let cond = self.parse_expression()?;
            let cond_reg = self.expr_to_register(cond)?;

            // Emit test and jump: IST skips JMP if true (fall through to then block)
            let line = self.current_line();
            self.fs_mut().emit(Instruction::ad(Opcode::IST, 0, cond_reg as u16), line);
            let false_jump = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);

            self.lexer.expect(TokenKind::Then)?;
            self.parse_block()?;

            // Check for else/elseif
            let next_token = &self.lexer.peek()?.kind;
            if matches!(next_token, TokenKind::Else | TokenKind::Elseif) {
                // Jump to end after this block
                let line = self.current_line();
                let end_jump = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);
                end_jumps.push(end_jump);
            }

            // Patch false jump to here
            let target = self.fs().current_pc();
            self.patch_jump(false_jump, target)?;

            match &self.lexer.peek()?.kind {
                TokenKind::Elseif => {
                    self.lexer.next()?;
                    continue;
                }
                TokenKind::Else => {
                    self.lexer.next()?;
                    self.parse_block()?;
                    break;
                }
                _ => break,
            }
        }

        self.lexer.expect(TokenKind::End)?;

        // Patch all end jumps
        let target = self.fs().current_pc();
        for jump in end_jumps {
            self.patch_jump(jump, target)?;
        }

        Ok(())
    }

    /// Parse while statement
    fn parse_while(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'while'

        let loop_start = self.fs().current_pc();
        let saved_break = std::mem::take(&mut self.fs_mut().break_jumps);
        let saved_loop = self.fs_mut().loop_start;
        self.fs_mut().loop_start = Some(loop_start);

        // Parse condition
        let cond = self.parse_expression()?;
        let cond_reg = self.expr_to_register(cond)?;

        // Emit test and jump if false
        let line = self.current_line();
        // IST skips JMP if true (continue loop), else JMP exits
        self.fs_mut().emit(Instruction::ad(Opcode::IST, 0, cond_reg as u16), line);
        let exit_jump = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);

        self.lexer.expect(TokenKind::Do)?;
        self.parse_block()?;
        self.lexer.expect(TokenKind::End)?;

        // Jump back to start
        let line = self.current_line();
        let pc = self.fs().current_pc();
        let offset = loop_start as i16 - pc as i16 - 1;
        self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, offset), line);

        // Patch exit jump
        let target = self.fs().current_pc();
        self.patch_jump(exit_jump, target)?;

        // Patch break jumps
        let breaks = std::mem::replace(&mut self.fs_mut().break_jumps, saved_break);
        for jump in breaks {
            self.patch_jump(jump, target)?;
        }
        self.fs_mut().loop_start = saved_loop;

        Ok(())
    }

    /// Parse do...end block
    fn parse_do(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'do'
        self.parse_block()?;
        self.lexer.expect(TokenKind::End)?;
        Ok(())
    }

    /// Parse for statement
    fn parse_for(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'for'

        let name = match self.lexer.next()?.kind {
            TokenKind::Name(n) => n,
            _ => return Err(LuaError::SyntaxError("expected name".to_string())),
        };

        match &self.lexer.peek()?.kind {
            TokenKind::Eq => self.parse_numeric_for(name),
            TokenKind::Comma | TokenKind::In => self.parse_generic_for(name),
            _ => Err(LuaError::SyntaxError("'=' or 'in' expected".to_string())),
        }
    }

    fn parse_numeric_for(&mut self, name: String) -> LuaResult<()> {
        self.lexer.next()?; // consume '='

        let base = self.fs().free_reg;

        // Parse init, limit, step - must be in consecutive registers starting at base
        // Reset free_reg to base to ensure proper placement
        let init = self.parse_expression()?;
        self.fs_mut().free_reg = base;
        self.expr_to_reg(init, base)?;
        self.fs_mut().free_reg = base + 1;

        self.lexer.expect(TokenKind::Comma)?;
        let limit = self.parse_expression()?;
        self.fs_mut().free_reg = base + 1;
        self.expr_to_reg(limit, base + 1)?;
        self.fs_mut().free_reg = base + 2;

        if self.lexer.match_token(&TokenKind::Comma)? {
            let s = self.parse_expression()?;
            self.fs_mut().free_reg = base + 2;
            self.expr_to_reg(s, base + 2)?;
        } else {
            // Default step = 1
            let line = self.current_line();
            self.fs_mut().emit(Instruction::ad(Opcode::KSHORT, base + 2, 1), line);
        }
        self.fs_mut().free_reg = base + 3;

        // Reserve slot for loop variable
        let loop_var = self.fs_mut().reserve_reg();
        let start_pc = self.fs().current_pc();
        self.fs_mut().locals.push(LocalVar {
            name,
            slot: loop_var,
            start_pc,
            is_captured: false,
        });

        self.lexer.expect(TokenKind::Do)?;

        // Emit FORI
        let line = self.current_line();
        let loop_start = self.fs_mut().emit(Instruction::adj(Opcode::FORI, base, 0), line);

        let saved_break = std::mem::take(&mut self.fs_mut().break_jumps);
        let saved_loop = self.fs_mut().loop_start;
        self.fs_mut().loop_start = Some(loop_start);

        self.parse_block()?;
        self.lexer.expect(TokenKind::End)?;

        // Emit UCLO to close upvalues for the loop variable (slot base+3)
        // The visible loop variable is at base+3; base, base+1, base+2 are control vars
        let line = self.current_line();
        self.fs_mut().emit(Instruction::adj(Opcode::UCLO, base + 3, 0), line);

        // Emit FORL
        let pc = self.fs().current_pc();
        let offset = loop_start as i16 - pc as i16 - 1;
        self.fs_mut().emit(Instruction::adj(Opcode::FORL, base, offset), line);

        // Patch FORI to jump past FORL
        let target = self.fs().current_pc();
        self.patch_jump(loop_start, target)?;

        // Patch breaks
        let breaks = std::mem::replace(&mut self.fs_mut().break_jumps, saved_break);
        for jump in breaks {
            self.patch_jump(jump, target)?;
        }
        self.fs_mut().loop_start = saved_loop;

        // Remove loop variable
        self.fs_mut().locals.pop();
        self.fs_mut().free_reg = base;

        Ok(())
    }

    fn parse_generic_for(&mut self, first_name: String) -> LuaResult<()> {
        let mut names = vec![first_name];

        while self.lexer.match_token(&TokenKind::Comma)? {
            match self.lexer.next()?.kind {
                TokenKind::Name(n) => names.push(n),
                _ => return Err(LuaError::SyntaxError("expected name".to_string())),
            }
        }

        self.lexer.expect(TokenKind::In)?;

        let base = self.fs().free_reg;

        // Parse iterator expressions (generator, state, control)
        let mut num_exprs = 0;
        let mut last_call_info: Option<(u8, usize)> = None; // (call_base, call_pc)
        loop {
            let expr = self.parse_expression()?;
            // For calls, keep results in place (don't move them)
            if let ExprDesc::Call(call_base, _, pc) = &expr {
                last_call_info = Some((*call_base, *pc));
                // Don't use expr_to_next_reg for calls - keep results at call_base
                self.fs_mut().free_reg = *call_base + 1;
            } else {
                last_call_info = None;
                self.expr_to_next_reg(expr)?;
            }
            num_exprs += 1;
            if !self.lexer.match_token(&TokenKind::Comma)? {
                break;
            }
        }

        // If there's only 1 expression and it's a call, patch it to return 3 values
        // (e.g., pairs(t) returns next, t, nil)
        if num_exprs == 1 {
            if let Some((call_base, call_pc)) = last_call_info {
                // Patch the CALL instruction to request 3 results instead of 1
                let mut call_instr = self.fs().proto.code[call_pc];
                // C field: 2 = 1 result, 4 = 3 results
                call_instr.set_c(4);
                self.fs_mut().proto.code[call_pc] = call_instr;
                // Reserve the additional 2 slots for the extra return values
                self.fs_mut().free_reg = call_base + 3;
                num_exprs = 3;
            }
        }

        // Adjust to 3 values
        while num_exprs < 3 {
            let line = self.current_line();
            let reg = self.fs_mut().reserve_reg();
            self.fs_mut().emit(Instruction::ad(Opcode::KPRI, reg, 0), line); // nil
            num_exprs += 1;
        }

        // Reserve slots for loop variables
        for name in &names {
            let slot = self.fs_mut().reserve_reg();
            let start_pc = self.fs().current_pc();
            self.fs_mut().locals.push(LocalVar {
                name: name.clone(),
                slot,
                start_pc,
                is_captured: false,
            });
        }

        self.lexer.expect(TokenKind::Do)?;

        // Emit JMP to loop test
        let line = self.current_line();
        let jmp_to_test = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);

        let loop_start = self.fs().current_pc();
        let saved_break = std::mem::take(&mut self.fs_mut().break_jumps);
        let saved_loop = self.fs_mut().loop_start;
        self.fs_mut().loop_start = Some(loop_start);

        self.parse_block()?;
        self.lexer.expect(TokenKind::End)?;

        // Emit UCLO to close upvalues for loop variables before continuing
        let line = self.current_line();
        self.fs_mut().emit(Instruction::adj(Opcode::UCLO, base + 3, 0), line);

        // Patch JMP to here (loop test)
        let test_pc = self.fs().current_pc();
        self.patch_jump(jmp_to_test, test_pc)?;

        // Emit ITERC (call iterator)
        self.fs_mut().emit(
            Instruction::abc(Opcode::ITERC, base + 3, base, names.len() as u8 + 1),
            line,
        );

        // Emit ITERL (loop if not nil)
        let pc = self.fs().current_pc();
        let offset = loop_start as i16 - pc as i16 - 1;
        self.fs_mut().emit(Instruction::adj(Opcode::ITERL, base + 3, offset), line);

        // Patch breaks
        let target = self.fs().current_pc();
        let breaks = std::mem::replace(&mut self.fs_mut().break_jumps, saved_break);
        for jump in breaks {
            self.patch_jump(jump, target)?;
        }
        self.fs_mut().loop_start = saved_loop;

        // Remove loop variables
        for _ in &names {
            self.fs_mut().locals.pop();
        }
        self.fs_mut().free_reg = base;

        Ok(())
    }

    /// Parse repeat...until
    fn parse_repeat(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'repeat'

        let loop_start = self.fs().current_pc();
        let saved_break = std::mem::take(&mut self.fs_mut().break_jumps);
        let saved_loop = self.fs_mut().loop_start;
        self.fs_mut().loop_start = Some(loop_start);

        self.parse_block()?;
        self.lexer.expect(TokenKind::Until)?;

        // Parse condition
        let cond = self.parse_expression()?;
        let cond_reg = self.expr_to_register(cond)?;

        // IST skips JMP if true (exit loop), else JMP continues loop
        let line = self.current_line();
        self.fs_mut().emit(Instruction::ad(Opcode::IST, 0, cond_reg as u16), line);
        let pc = self.fs().current_pc();
        let offset = loop_start as i16 - pc as i16 - 1;  // -1 accounts for PC increment after JMP
        self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, offset), line);

        // Patch breaks
        let target = self.fs().current_pc();
        let breaks = std::mem::replace(&mut self.fs_mut().break_jumps, saved_break);
        for jump in breaks {
            self.patch_jump(jump, target)?;
        }
        self.fs_mut().loop_start = saved_loop;

        Ok(())
    }

    /// Parse function statement
    fn parse_function_stat(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'function'

        // Parse name (may be table field access like t.foo or t:method)
        let first_name = match self.lexer.next()?.kind {
            TokenKind::Name(n) => n,
            _ => return Err(LuaError::SyntaxError("expected function name".to_string())),
        };

        // Check for field access (t.foo) or method (t:method)
        let mut is_method = false;
        let mut table_expr: Option<ExprDesc> = None;
        let mut field_name: Option<String> = None;

        loop {
            if self.lexer.match_token(&TokenKind::Dot)? {
                // t.foo.bar...
                let name = match self.lexer.next()?.kind {
                    TokenKind::Name(n) => n,
                    _ => return Err(LuaError::SyntaxError("expected name after '.'".to_string())),
                };
                if table_expr.is_none() {
                    // First dot - resolve the base name
                    table_expr = Some(self.resolve_name(&first_name)?);
                }
                // Access the field
                let table_reg = self.expr_to_register(table_expr.take().unwrap())?;
                let key_idx = self.fs_mut().add_string_constant(&name);
                table_expr = Some(ExprDesc::Index {
                    table: table_reg,
                    key: key_idx as u8,
                    key_is_const: true,
                });
                field_name = Some(name);
            } else if self.lexer.match_token(&TokenKind::Colon)? {
                // t:method (method definition)
                let name = match self.lexer.next()?.kind {
                    TokenKind::Name(n) => n,
                    _ => return Err(LuaError::SyntaxError("expected method name after ':'".to_string())),
                };
                if table_expr.is_none() {
                    table_expr = Some(self.resolve_name(&first_name)?);
                }
                field_name = Some(name);
                is_method = true;
                break; // Method is always last
            } else {
                break;
            }
        }

        // Parse function body
        let proto = self.parse_function_body(is_method)?;

        // Add child proto to parent and get index
        let proto_idx = self.fs().proto.child_protos.len();
        self.fs_mut().proto.child_protos.push(Box::new(proto));

        // Create closure
        let line = self.current_line();
        let func_reg = self.fs_mut().reserve_reg();
        self.fs_mut().emit(Instruction::ad(Opcode::FNEW, func_reg, proto_idx as u16), line);

        if let Some(fname) = field_name {
            // Assign to table field: t.foo = function or t.method = function
            let table_reg = self.expr_to_register(table_expr.unwrap())?;
            let key_idx = self.fs_mut().add_string_constant(&fname);
            self.fs_mut().emit(
                Instruction::abc(Opcode::TSETS, table_reg, func_reg, key_idx as u8),
                line,
            );
        } else {
            // Assign to global
            let name_idx = self.fs_mut().add_string_constant(&first_name);
            self.fs_mut().emit(Instruction::ad(Opcode::GSET, func_reg, name_idx as u16), line);
        }
        self.fs_mut().free_regs(1);

        Ok(())
    }

    /// Parse local statement
    fn parse_local(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'local'

        if self.lexer.match_token(&TokenKind::Function)? {
            // local function name ...
            let name = match self.lexer.next()?.kind {
                TokenKind::Name(n) => n,
                _ => return Err(LuaError::SyntaxError("expected name".to_string())),
            };

            // Reserve slot first (for recursive calls)
            let slot = self.fs_mut().reserve_reg();
            let start_pc = self.fs().current_pc();
            self.fs_mut().locals.push(LocalVar {
                name,
                slot,
                start_pc,
                is_captured: false,
            });

            let proto = self.parse_function_body(false)?;

            // Add child proto to parent and get index
            let proto_idx = self.fs().proto.child_protos.len();
            self.fs_mut().proto.child_protos.push(Box::new(proto));

            let line = self.current_line();
            self.fs_mut().emit(Instruction::ad(Opcode::FNEW, slot, proto_idx as u16), line);
        } else {
            // local name1, name2, ... = expr1, expr2, ...
            let mut names = Vec::new();
            loop {
                match self.lexer.next()?.kind {
                    TokenKind::Name(n) => names.push(n),
                    _ => return Err(LuaError::SyntaxError("expected name".to_string())),
                }
                if !self.lexer.match_token(&TokenKind::Comma)? {
                    break;
                }
            }

            let first_slot = self.fs().free_reg;

            // Reserve slots
            for name in &names {
                let slot = self.fs_mut().reserve_reg();
                let start_pc = self.fs().current_pc();
                self.fs_mut().locals.push(LocalVar {
                    name: name.clone(),
                    slot,
                    start_pc,
                    is_captured: false,
                });
            }

            // Parse initializers
            if self.lexer.match_token(&TokenKind::Eq)? {
                let mut num_exprs = 0u8;
                let num_names = names.len() as u8;
                let mut last_call_pc: Option<usize> = None;

                loop {
                    let expr = self.parse_expression()?;
                    let target = first_slot + num_exprs;

                    // Track if last expression is a call and its PC
                    if let ExprDesc::Call(_, _, pc) = expr {
                        last_call_pc = Some(pc);
                    } else {
                        last_call_pc = None;
                    }

                    self.expr_to_reg(expr, target)?;
                    num_exprs += 1;

                    if !self.lexer.match_token(&TokenKind::Comma)? {
                        break;
                    }
                }

                // If last expression was a call and we need more values, patch it
                if let Some(call_pc) = last_call_pc {
                    if num_exprs < num_names {
                        let needed = num_names - num_exprs + 1; // +1 because we already counted the first result
                        let mut call_instr = self.fs().proto.code[call_pc];
                        let call_base = call_instr.a();
                        // C field: needed + 1 (since C=0 means MULTRET, C=1 means 0 results, C=2 means 1 result, etc.)
                        call_instr.set_c(needed + 1);
                        self.fs_mut().proto.code[call_pc] = call_instr;

                        // If the call results aren't at the right position, we need to move them
                        // The first result was already moved by expr_to_reg (via MOV)
                        // But additional results are at call_base+1, call_base+2, etc.
                        // They need to be at first_slot+1, first_slot+2, etc.
                        let target_base = first_slot + num_exprs - 1; // -1 because first was already moved
                        let line = self.current_line();
                        for i in 1..needed {
                            let src = call_base + i;
                            let dst = target_base + i;
                            if src != dst {
                                self.fs_mut().emit(Instruction::ad(Opcode::MOV, dst, src as u16), line);
                            }
                        }

                        // Update free_reg to account for additional results
                        self.fs_mut().free_reg = first_slot + num_names;
                        num_exprs = num_names; // All slots filled by call
                    }
                }

                // Fill remaining with nil
                while num_exprs < num_names {
                    let line = self.current_line();
                    let reg = first_slot + num_exprs;
                    self.fs_mut().emit(Instruction::ad(Opcode::KPRI, reg, 0), line);
                    num_exprs += 1;
                }
            } else {
                // No initializers, set all to nil
                let line = self.current_line();
                self.fs_mut().emit(
                    Instruction::ad(Opcode::KNIL, first_slot, names.len() as u16 - 1),
                    line,
                );
            }
        }

        Ok(())
    }

    /// Parse return statement
    fn parse_return(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'return'

        let line = self.current_line();

        // Check for empty return
        if self.check_block_end()? || self.lexer.check(&TokenKind::Semicolon)? {
            self.emit_uclo_if_needed();
            self.fs_mut().emit(Instruction::ad(Opcode::RET0, 0, 1), line);
            return Ok(());
        }

        // Parse first expression
        let first_expr = self.parse_expression()?;

        // Check if there's only one return value
        if !self.lexer.check(&TokenKind::Comma)? {
            // Single return value - figure out the register first
            let ret_reg = match first_expr {
                ExprDesc::Register(r) => r,
                ExprDesc::Call(base, _, _) => base,
                _ => self.expr_to_next_reg(first_expr)?,
            };
            // Emit UCLO before return if needed (AFTER all code generation)
            self.emit_uclo_if_needed();
            self.fs_mut().emit(Instruction::ad(Opcode::RET1, ret_reg, 2), line);
            return Ok(());
        }

        // Multiple return values - need consecutive registers
        let base = self.fs().free_reg;
        let mut value_regs = Vec::new();

        // Put first expression into next register
        let reg = self.expr_to_next_reg(first_expr)?;
        value_regs.push(reg);

        // Parse remaining expressions
        while self.lexer.match_token(&TokenKind::Comma)? {
            let expr = self.parse_expression()?;
            let reg = self.expr_to_next_reg(expr)?;
            value_regs.push(reg);
        }

        let count = value_regs.len() as u8;

        // Move values to consecutive slots if needed
        for (i, &reg) in value_regs.iter().enumerate() {
            let target = base + i as u8;
            if reg != target {
                self.fs_mut().emit(
                    Instruction::ad(Opcode::MOV, target, reg as u16),
                    line,
                );
            }
        }

        // Emit UCLO before return if needed (AFTER parsing all expressions)
        self.emit_uclo_if_needed();
        self.fs_mut().emit(Instruction::ad(Opcode::RET, base, count as u16 + 1), line);

        Ok(())
    }

    /// Emit UCLO instruction if any locals in current function are captured
    fn emit_uclo_if_needed(&mut self) {
        let has_captured = self.fs().locals.iter().any(|l| l.is_captured);
        if has_captured {
            let first_captured_slot = self.fs().locals.iter()
                .filter(|l| l.is_captured)
                .map(|l| l.slot)
                .min()
                .unwrap_or(0);
            let line = self.current_line();
            self.fs_mut().emit(Instruction::adj(Opcode::UCLO, first_captured_slot, 0), line);
        }
    }

    /// Parse break statement
    fn parse_break(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'break'

        if self.fs().loop_start.is_none() {
            return Err(LuaError::SyntaxError("break outside loop".to_string()));
        }

        let line = self.current_line();
        let jump = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);
        self.fs_mut().break_jumps.push(jump);

        Ok(())
    }

    /// Parse goto (simplified - just skip)
    fn parse_goto(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume 'goto'
        self.lexer.expect(TokenKind::Name("".to_string()))?;
        // Labels not fully implemented
        Ok(())
    }

    /// Parse label ::name::
    fn parse_label(&mut self) -> LuaResult<()> {
        self.lexer.next()?; // consume '::'
        self.lexer.next()?; // consume name
        self.lexer.expect(TokenKind::ColonColon)?;
        // Labels not fully implemented
        Ok(())
    }

    /// Parse expression statement (assignment or function call)
    fn parse_expr_stat(&mut self) -> LuaResult<()> {
        let expr = self.parse_suffixed_expr()?;

        if self.lexer.check(&TokenKind::Eq)? || self.lexer.check(&TokenKind::Comma)? {
            // Assignment
            self.parse_assignment(expr)?;
        } else {
            // Function call (must be a call expression)
            match expr {
                ExprDesc::Call(_, _, _) => {} // Already emitted
                _ => return Err(LuaError::SyntaxError("syntax error".to_string())),
            }
        }

        Ok(())
    }

    /// Parse assignment
    fn parse_assignment(&mut self, first: ExprDesc) -> LuaResult<()> {
        let mut targets = vec![first];

        while self.lexer.match_token(&TokenKind::Comma)? {
            targets.push(self.parse_suffixed_expr()?);
        }

        self.lexer.expect(TokenKind::Eq)?;

        // Save free_reg to restore after assignment (temporaries not needed)
        let saved_free_reg = self.fs().free_reg;

        // Parse values - track where each value ends up and if last is a call
        let mut value_regs = Vec::new();
        let mut last_call_info: Option<(u8, usize)> = None; // (base_reg, call_pc)

        loop {
            let expr = self.parse_expression()?;
            // Track if this expression is a call
            if let ExprDesc::Call(base, _, pc) = &expr {
                last_call_info = Some((*base, *pc));
                // For calls, don't move the result - keep it at call_base
                // This is important for multi-return handling
                value_regs.push(*base);
                self.fs_mut().free_reg = *base + 1;
            } else {
                last_call_info = None;
                let reg = self.expr_to_next_reg(expr)?;
                value_regs.push(reg);
            }
            if !self.lexer.match_token(&TokenKind::Comma)? {
                break;
            }
        }

        let num_targets = targets.len();
        let num_values = value_regs.len();

        // If we have more targets than values and last value is a call,
        // patch the call to return more results
        if num_targets > num_values {
            if let Some((call_base, call_pc)) = last_call_info {
                let extra_needed = num_targets - num_values + 1;
                // Patch CALL to return more results (C = nresults + 1)
                let mut call_instr = self.fs().proto.code[call_pc];
                call_instr.set_c((extra_needed + 1) as u8);
                self.fs_mut().proto.code[call_pc] = call_instr;
                // Add the additional result registers to value_regs
                for i in 1..extra_needed {
                    value_regs.push(call_base + i as u8);
                }
                self.fs_mut().free_reg = call_base + extra_needed as u8;
            }
        }

        // Assign values to targets
        for (i, target) in targets.iter().enumerate() {
            // If we don't have a value for this target, use nil
            let val_reg = if i < value_regs.len() {
                value_regs[i]
            } else {
                // Emit nil for missing values
                let line = self.current_line();
                let reg = self.fs_mut().reserve_reg();
                self.fs_mut().emit(Instruction::ad(Opcode::KPRI, reg, 0), line);
                reg
            };
            let line = self.current_line();

            match target {
                ExprDesc::Register(r) => {
                    if *r != val_reg {
                        self.fs_mut().emit(
                            Instruction::ad(Opcode::MOV, *r, val_reg as u16),
                            line,
                        );
                    }
                }
                ExprDesc::Upvalue(uv) => {
                    self.fs_mut().emit(
                        Instruction::ad(Opcode::USETV, *uv, val_reg as u16),
                        line,
                    );
                }
                ExprDesc::Global(idx) => {
                    self.fs_mut().emit(
                        Instruction::ad(Opcode::GSET, val_reg, *idx as u16),
                        line,
                    );
                }
                ExprDesc::Index { table, key, key_is_const } => {
                    if *key_is_const {
                        self.fs_mut().emit(
                            Instruction::abc(Opcode::TSETS, *table, val_reg, *key),
                            line,
                        );
                    } else {
                        self.fs_mut().emit(
                            Instruction::abc(Opcode::TSETV, *table, val_reg, *key),
                            line,
                        );
                    }
                }
                _ => {
                    return Err(LuaError::SyntaxError("invalid assignment target".to_string()));
                }
            }
        }

        self.fs_mut().free_reg = saved_free_reg;
        Ok(())
    }

    /// Parse expression
    fn parse_expression(&mut self) -> LuaResult<ExprDesc> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> LuaResult<ExprDesc> {
        let mut left = self.parse_and_expr()?;

        while self.lexer.match_token(&TokenKind::Or)? {
            let left_reg = self.expr_to_register(left)?;
            let line = self.current_line();

            // Short-circuit: if truthy, skip right side (keep left)
            // ISF skips next instruction if falsy, so if NOT falsy (truthy), we execute JMP
            self.fs_mut().emit(Instruction::ad(Opcode::ISF, 0, left_reg as u16), line);
            let skip_jump = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);

            let right = self.parse_and_expr()?;
            let right_reg = self.expr_to_register(right)?;

            // Move result to left_reg
            if right_reg != left_reg {
                self.fs_mut().emit(
                    Instruction::ad(Opcode::MOV, left_reg, right_reg as u16),
                    line,
                );
            }

            let target = self.fs().current_pc();
            self.patch_jump(skip_jump, target)?;

            left = ExprDesc::Register(left_reg);
        }

        Ok(left)
    }

    fn parse_and_expr(&mut self) -> LuaResult<ExprDesc> {
        let mut left = self.parse_compare_expr()?;

        while self.lexer.match_token(&TokenKind::And)? {
            let left_reg = self.expr_to_register(left)?;
            let line = self.current_line();

            // Short-circuit: if falsy, skip right side (keep left)
            // IST skips next instruction if truthy, so if NOT truthy (falsy), we execute JMP
            self.fs_mut().emit(Instruction::ad(Opcode::IST, 0, left_reg as u16), line);
            let skip_jump = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);

            let right = self.parse_compare_expr()?;
            let right_reg = self.expr_to_register(right)?;

            if right_reg != left_reg {
                self.fs_mut().emit(
                    Instruction::ad(Opcode::MOV, left_reg, right_reg as u16),
                    line,
                );
            }

            let target = self.fs().current_pc();
            self.patch_jump(skip_jump, target)?;

            left = ExprDesc::Register(left_reg);
        }

        Ok(left)
    }

    fn parse_compare_expr(&mut self) -> LuaResult<ExprDesc> {
        let mut left = self.parse_concat_expr()?;

        loop {
            let op = match &self.lexer.peek()?.kind {
                TokenKind::Lt => Opcode::ISLT,
                TokenKind::Gt => Opcode::ISGT,
                TokenKind::LtEq => Opcode::ISLE,
                TokenKind::GtEq => Opcode::ISGE,
                TokenKind::EqEq => Opcode::ISEQV,
                TokenKind::TildeEq => Opcode::ISNEV,
                _ => break,
            };
            self.lexer.next()?;

            let left_reg = self.expr_to_register(left)?;
            let right = self.parse_concat_expr()?;
            let right_reg = self.expr_to_register(right)?;

            let line = self.current_line();

            // Materialize boolean result:
            // 1. Set true first (KPRI D=2)
            // 2. Emit comparison (IS* skips next if true)
            // 3. Set false (KPRI D=1) - only reached if comparison was false
            let result = self.fs_mut().reserve_reg();
            self.fs_mut().emit(Instruction::ad(Opcode::KPRI, result, 2), line); // true
            self.fs_mut().emit(
                Instruction::ad(op, left_reg, right_reg as u16),
                line,
            );
            self.fs_mut().emit(Instruction::ad(Opcode::KPRI, result, 1), line); // false

            left = ExprDesc::Register(result);
        }

        Ok(left)
    }

    fn parse_concat_expr(&mut self) -> LuaResult<ExprDesc> {
        let mut left = self.parse_add_expr()?;

        if self.lexer.match_token(&TokenKind::DotDot)? {
            let left_reg = self.expr_to_register(left)?;
            let mut right_count = 0u8;

            loop {
                let right = self.parse_add_expr()?;
                self.expr_to_next_reg(right)?;
                right_count += 1;

                if !self.lexer.match_token(&TokenKind::DotDot)? {
                    break;
                }
            }

            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::abc(Opcode::CAT, left_reg, left_reg, left_reg + right_count),
                line,
            );
            self.fs_mut().free_reg = left_reg + 1;

            left = ExprDesc::Register(left_reg);
        }

        Ok(left)
    }

    fn parse_add_expr(&mut self) -> LuaResult<ExprDesc> {
        let mut left = self.parse_mul_expr()?;

        loop {
            let op = match &self.lexer.peek()?.kind {
                TokenKind::Plus => Opcode::ADDVV,
                TokenKind::Minus => Opcode::SUBVV,
                _ => break,
            };
            self.lexer.next()?;

            let left_reg = self.expr_to_register(left)?;
            let right = self.parse_mul_expr()?;
            let right_reg = self.expr_to_register(right)?;

            // Allocate a new register for the result to avoid clobbering source
            let result_reg = self.fs_mut().reserve_reg();
            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::abc(op, result_reg, left_reg, right_reg),
                line,
            );
            // Free the operand registers if they were temporary
            self.fs_mut().free_reg = result_reg + 1;

            left = ExprDesc::Register(result_reg);
        }

        Ok(left)
    }

    fn parse_mul_expr(&mut self) -> LuaResult<ExprDesc> {
        let mut left = self.parse_unary_expr()?;

        loop {
            let op = match &self.lexer.peek()?.kind {
                TokenKind::Star => Opcode::MULVV,
                TokenKind::Slash => Opcode::DIVVV,
                TokenKind::Percent => Opcode::MODVV,
                TokenKind::SlashSlash => Opcode::DIVVV, // floor div (simplified)
                _ => break,
            };
            self.lexer.next()?;

            let left_reg = self.expr_to_register(left)?;
            let right = self.parse_unary_expr()?;
            let right_reg = self.expr_to_register(right)?;

            // Allocate a new register for the result to avoid clobbering source
            let result_reg = self.fs_mut().reserve_reg();
            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::abc(op, result_reg, left_reg, right_reg),
                line,
            );
            self.fs_mut().free_reg = result_reg + 1;

            left = ExprDesc::Register(result_reg);
        }

        Ok(left)
    }

    /// Continue parsing an expression given an already-parsed primary in a register.
    /// This handles the case in table constructors where we've already consumed a name
    /// but need to continue parsing operators like + - * / etc.
    fn parse_subexpr_with_primary(&mut self, primary_reg: u8, _min_prec: u8) -> LuaResult<ExprDesc> {
        let mut left = ExprDesc::Register(primary_reg);

        // Handle power operator (highest precedence among binary ops after primary)
        if self.lexer.match_token(&TokenKind::Caret)? {
            let left_reg = self.expr_to_register(left)?;
            let right = self.parse_unary_expr()?;
            let right_reg = self.expr_to_register(right)?;
            let result_reg = self.fs_mut().reserve_reg();
            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::abc(Opcode::POW, result_reg, left_reg, right_reg),
                line,
            );
            self.fs_mut().free_reg = result_reg + 1;
            left = ExprDesc::Register(result_reg);
        }

        // Handle mul/div operators
        loop {
            let op = match &self.lexer.peek()?.kind {
                TokenKind::Star => Opcode::MULVV,
                TokenKind::Slash => Opcode::DIVVV,
                TokenKind::Percent => Opcode::MODVV,
                TokenKind::SlashSlash => Opcode::DIVVV,
                _ => break,
            };
            self.lexer.next()?;

            let left_reg = self.expr_to_register(left)?;
            let right = self.parse_unary_expr()?;
            let right_reg = self.expr_to_register(right)?;
            let result_reg = self.fs_mut().reserve_reg();
            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::abc(op, result_reg, left_reg, right_reg),
                line,
            );
            self.fs_mut().free_reg = result_reg + 1;
            left = ExprDesc::Register(result_reg);
        }

        // Handle add/sub operators
        loop {
            let op = match &self.lexer.peek()?.kind {
                TokenKind::Plus => Opcode::ADDVV,
                TokenKind::Minus => Opcode::SUBVV,
                _ => break,
            };
            self.lexer.next()?;

            let left_reg = self.expr_to_register(left)?;
            let right = self.parse_mul_expr()?;
            let right_reg = self.expr_to_register(right)?;
            let result_reg = self.fs_mut().reserve_reg();
            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::abc(op, result_reg, left_reg, right_reg),
                line,
            );
            self.fs_mut().free_reg = result_reg + 1;
            left = ExprDesc::Register(result_reg);
        }

        // Handle concat operator
        if self.lexer.match_token(&TokenKind::DotDot)? {
            let left_reg = self.expr_to_register(left)?;
            let mut right_count = 0u8;

            loop {
                let right = self.parse_add_expr()?;
                self.expr_to_next_reg(right)?;
                right_count += 1;

                if !self.lexer.match_token(&TokenKind::DotDot)? {
                    break;
                }
            }

            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::abc(Opcode::CAT, left_reg, left_reg, left_reg + right_count),
                line,
            );
            self.fs_mut().free_reg = left_reg + 1;
            left = ExprDesc::Register(left_reg);
        }

        // Handle comparison operators
        loop {
            let op = match &self.lexer.peek()?.kind {
                TokenKind::Lt => Opcode::ISLT,
                TokenKind::Gt => Opcode::ISGT,
                TokenKind::LtEq => Opcode::ISLE,
                TokenKind::GtEq => Opcode::ISGE,
                TokenKind::EqEq => Opcode::ISEQV,
                TokenKind::TildeEq => Opcode::ISNEV,
                _ => break,
            };
            self.lexer.next()?;

            let left_reg = self.expr_to_register(left)?;
            let right = self.parse_concat_expr()?;
            let right_reg = self.expr_to_register(right)?;

            let line = self.current_line();
            let result = self.fs_mut().reserve_reg();
            self.fs_mut().emit(Instruction::ad(Opcode::KPRI, result, 2), line); // true
            self.fs_mut().emit(
                Instruction::ad(op, left_reg, right_reg as u16),
                line,
            );
            self.fs_mut().emit(Instruction::ad(Opcode::KPRI, result, 1), line); // false

            left = ExprDesc::Register(result);
        }

        // Handle 'and' operator
        while self.lexer.match_token(&TokenKind::And)? {
            let left_reg = self.expr_to_register(left)?;
            let line = self.current_line();

            // Short-circuit: if falsy, skip right side
            self.fs_mut().emit(Instruction::ad(Opcode::IST, 0, left_reg as u16), line);
            let skip_jump = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);

            let right = self.parse_compare_expr()?;
            let right_reg = self.expr_to_register(right)?;

            if right_reg != left_reg {
                self.fs_mut().emit(
                    Instruction::ad(Opcode::MOV, left_reg, right_reg as u16),
                    line,
                );
            }

            let target = self.fs().current_pc();
            self.patch_jump(skip_jump, target)?;

            left = ExprDesc::Register(left_reg);
        }

        // Handle 'or' operator
        while self.lexer.match_token(&TokenKind::Or)? {
            let left_reg = self.expr_to_register(left)?;
            let line = self.current_line();

            // Short-circuit: if truthy, skip right side
            self.fs_mut().emit(Instruction::ad(Opcode::ISF, 0, left_reg as u16), line);
            let skip_jump = self.fs_mut().emit(Instruction::adj(Opcode::JMP, 0, 0), line);

            let right = self.parse_and_expr()?;
            let right_reg = self.expr_to_register(right)?;

            if right_reg != left_reg {
                self.fs_mut().emit(
                    Instruction::ad(Opcode::MOV, left_reg, right_reg as u16),
                    line,
                );
            }

            let target = self.fs().current_pc();
            self.patch_jump(skip_jump, target)?;

            left = ExprDesc::Register(left_reg);
        }

        Ok(left)
    }

    fn parse_unary_expr(&mut self) -> LuaResult<ExprDesc> {
        match &self.lexer.peek()?.kind {
            TokenKind::Not => {
                self.lexer.next()?;
                let expr = self.parse_unary_expr()?;
                let src_reg = self.expr_to_register(expr)?;
                let result_reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                self.fs_mut().emit(Instruction::ad(Opcode::NOT, result_reg, src_reg as u16), line);
                Ok(ExprDesc::Register(result_reg))
            }
            TokenKind::Minus => {
                self.lexer.next()?;
                let expr = self.parse_unary_expr()?;
                let src_reg = self.expr_to_register(expr)?;
                let result_reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                self.fs_mut().emit(Instruction::ad(Opcode::UNM, result_reg, src_reg as u16), line);
                Ok(ExprDesc::Register(result_reg))
            }
            TokenKind::Hash => {
                self.lexer.next()?;
                let expr = self.parse_unary_expr()?;
                let src_reg = self.expr_to_register(expr)?;
                let result_reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                self.fs_mut().emit(Instruction::ad(Opcode::LEN, result_reg, src_reg as u16), line);
                Ok(ExprDesc::Register(result_reg))
            }
            _ => self.parse_pow_expr(),
        }
    }

    fn parse_pow_expr(&mut self) -> LuaResult<ExprDesc> {
        let mut left = self.parse_suffixed_expr()?;

        if self.lexer.match_token(&TokenKind::Caret)? {
            let left_reg = self.expr_to_register(left)?;
            let right = self.parse_unary_expr()?; // Right associative
            let right_reg = self.expr_to_register(right)?;

            // Allocate a new register for the result to avoid clobbering source
            let result_reg = self.fs_mut().reserve_reg();
            let line = self.current_line();
            self.fs_mut().emit(
                Instruction::abc(Opcode::POW, result_reg, left_reg, right_reg),
                line,
            );
            self.fs_mut().free_reg = result_reg + 1;

            left = ExprDesc::Register(result_reg);
        }

        Ok(left)
    }

    fn parse_suffixed_expr(&mut self) -> LuaResult<ExprDesc> {
        let mut expr = self.parse_primary_expr()?;

        loop {
            match &self.lexer.peek()?.kind {
                TokenKind::Dot => {
                    self.lexer.next()?;
                    let name = match self.lexer.next()?.kind {
                        TokenKind::Name(n) => n,
                        _ => return Err(LuaError::SyntaxError("expected name after '.'".to_string())),
                    };
                    let table_reg = self.expr_to_register(expr)?;
                    let key_idx = self.fs_mut().add_string_constant(&name);
                    expr = ExprDesc::Index {
                        table: table_reg,
                        key: key_idx as u8,
                        key_is_const: true,
                    };
                }
                TokenKind::LBracket => {
                    self.lexer.next()?;
                    let table_reg = self.expr_to_register(expr)?;
                    let key_expr = self.parse_expression()?;
                    let key_reg = self.expr_to_register(key_expr)?;
                    self.lexer.expect(TokenKind::RBracket)?;
                    expr = ExprDesc::Index {
                        table: table_reg,
                        key: key_reg,
                        key_is_const: false,
                    };
                }
                TokenKind::Colon => {
                    self.lexer.next()?;
                    let method_name = match self.lexer.next()?.kind {
                        TokenKind::Name(n) => n,
                        _ => return Err(LuaError::SyntaxError("expected name after ':'".to_string())),
                    };
                    // Method call: obj:method(args) -> obj.method(obj, args)
                    let table_reg = self.expr_to_register(expr)?;
                    let method_idx = self.fs_mut().add_string_constant(&method_name);
                    let method_expr = ExprDesc::Index {
                        table: table_reg,
                        key: method_idx as u8,
                        key_is_const: true,
                    };
                    expr = self.parse_method_call_expr(method_expr, table_reg)?;
                }
                TokenKind::LParen | TokenKind::LBrace | TokenKind::String(_) => {
                    expr = self.parse_call_expr(expr, false)?;
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_call_expr(&mut self, func: ExprDesc, _is_method: bool) -> LuaResult<ExprDesc> {
        let line = self.current_line();

        // Move function to a contiguous register block at free_reg.
        // This ensures arguments can be placed immediately after the function.
        let base = self.fs().free_reg;
        self.expr_to_reg(func, base)?;
        // Set free_reg to base + 1 so arguments go immediately after function
        self.fs_mut().free_reg = base + 1;

        // Track if last arg is a call or vararg (for variable args)
        let mut last_call_pc: Option<usize> = None;
        let mut last_is_vararg = false;
        let mut last_vararg_pc: Option<usize> = None;
        let num_args = match &self.lexer.peek()?.kind {
            TokenKind::LParen => {
                self.lexer.next()?;
                let mut count = 0u8;
                if !self.lexer.check(&TokenKind::RParen)? {
                    loop {
                        // Target register for this argument
                        let target = self.fs().free_reg;
                        let arg = self.parse_expression()?;

                        // Check if this is a call or vararg expression (might be last arg)
                        let call_base = match &arg {
                            ExprDesc::Call(base, _, pc) => {
                                last_call_pc = Some(*pc);
                                last_is_vararg = false;
                                last_vararg_pc = None;
                                Some(*base)
                            }
                            ExprDesc::Vararg => {
                                last_is_vararg = true;
                                last_call_pc = None;
                                // We'll get the VARG pc after expr_to_reg emits it
                                last_vararg_pc = Some(self.fs().current_pc());
                                None
                            }
                            _ => {
                                last_call_pc = None;
                                last_is_vararg = false;
                                last_vararg_pc = None;
                                None
                            }
                        };

                        self.expr_to_reg(arg, target)?;

                        // If last was vararg, get the actual pc of VARG instruction
                        if last_is_vararg {
                            last_vararg_pc = Some(self.fs().current_pc() - 1);
                        }

                        // Track if a MOV was emitted for the last call (call_base != target)
                        // If MOV was needed, the call doesn't land at target, so we can't use
                        // variable result passing (would have gaps in args)
                        let call_at_target = call_base.map(|b| b == target).unwrap_or(true);
                        if !call_at_target {
                            // Clear last_call_pc so we don't patch this call to C=0
                            last_call_pc = None;
                        }

                        // Ensure next arg goes to next slot
                        self.fs_mut().free_reg = target + 1;
                        count += 1;
                        if !self.lexer.match_token(&TokenKind::Comma)? {
                            break;
                        }
                    }
                }
                self.lexer.expect(TokenKind::RParen)?;
                count
            }
            TokenKind::LBrace => {
                // Table constructor as single argument
                let arg = self.parse_table_constructor()?;
                self.expr_to_next_reg(arg)?;
                1
            }
            TokenKind::String(s) => {
                let s = s.clone();
                self.lexer.next()?;
                let idx = self.fs_mut().add_string_constant(&s);
                let reg = self.fs_mut().reserve_reg();
                self.fs_mut().emit(Instruction::ad(Opcode::KSTR, reg, idx as u16), line);
                1
            }
            _ => return Err(LuaError::SyntaxError("expected function arguments".to_string())),
        };

        // If last arg was a call or vararg, patch it and use B=0 for variable args
        let b_field = if let Some(call_pc) = last_call_pc {
            // Patch inner call to return variable results
            let mut inner_call = self.fs().proto.code[call_pc];
            inner_call.set_c(0); // C=0 means return all results
            self.fs_mut().proto.code[call_pc] = inner_call;
            0u8 // B=0 means variable args from previous call
        } else if last_is_vararg {
            // Patch VARG instruction to return all varargs (B=0)
            if let Some(vararg_pc) = last_vararg_pc {
                let mut varg_instr = self.fs().proto.code[vararg_pc];
                varg_instr.set_b(0); // B=0 means return all varargs
                self.fs_mut().proto.code[vararg_pc] = varg_instr;
            }
            0u8 // B=0 means variable args from vararg
        } else {
            num_args + 1 // Normal: B = fixed args + 1
        };

        // Emit CALL
        let call_pc = self.fs().current_pc();
        self.fs_mut().emit(
            Instruction::abc(Opcode::CALL, base, b_field, 2), // 2 = 1 result + 1
            line,
        );
        self.fs_mut().free_reg = base + 1;

        Ok(ExprDesc::Call(base, 1, call_pc))
    }

    /// Parse method call: obj:method(args) - obj is passed as first argument
    fn parse_method_call_expr(&mut self, method: ExprDesc, self_reg: u8) -> LuaResult<ExprDesc> {
        let line = self.current_line();

        // Layout: [func, self, arg1, arg2, ...]
        let base = self.fs().free_reg;
        self.expr_to_reg(method, base)?;
        self.fs_mut().free_reg = base + 1;

        // Copy self to be first argument
        self.fs_mut().emit(
            Instruction::ad(Opcode::MOV, base + 1, self_reg as u16),
            line,
        );
        self.fs_mut().free_reg = base + 2;

        // Parse arguments (starting at base + 2)
        // Track if last arg is a call or vararg (for variable args)
        let mut last_call_pc: Option<usize> = None;
        let mut last_is_vararg = false;
        let mut last_vararg_pc: Option<usize> = None;
        let num_args = match &self.lexer.peek()?.kind {
            TokenKind::LParen => {
                self.lexer.next()?;
                let mut count = 1u8; // Start at 1 for self
                if !self.lexer.check(&TokenKind::RParen)? {
                    loop {
                        let target = self.fs().free_reg;
                        let arg = self.parse_expression()?;

                        // Check if this is a call or vararg expression (might be last arg)
                        match &arg {
                            ExprDesc::Call(_, _, pc) => {
                                last_call_pc = Some(*pc);
                                last_is_vararg = false;
                                last_vararg_pc = None;
                            }
                            ExprDesc::Vararg => {
                                last_is_vararg = true;
                                last_call_pc = None;
                                last_vararg_pc = Some(self.fs().current_pc());
                            }
                            _ => {
                                last_call_pc = None;
                                last_is_vararg = false;
                                last_vararg_pc = None;
                            }
                        }

                        self.expr_to_reg(arg, target)?;

                        // If last was vararg, get the actual pc of VARG instruction
                        if last_is_vararg {
                            last_vararg_pc = Some(self.fs().current_pc() - 1);
                        }

                        self.fs_mut().free_reg = target + 1;
                        count += 1;
                        if !self.lexer.match_token(&TokenKind::Comma)? {
                            break;
                        }
                    }
                }
                self.lexer.expect(TokenKind::RParen)?;
                count
            }
            TokenKind::LBrace => {
                let arg = self.parse_table_constructor()?;
                self.expr_to_next_reg(arg)?;
                2 // self + table
            }
            TokenKind::String(s) => {
                let s = s.clone();
                self.lexer.next()?;
                let idx = self.fs_mut().add_string_constant(&s);
                let reg = self.fs_mut().reserve_reg();
                self.fs_mut().emit(Instruction::ad(Opcode::KSTR, reg, idx as u16), line);
                2 // self + string
            }
            _ => return Err(LuaError::SyntaxError("expected method arguments".to_string())),
        };

        // If last arg was a call or vararg, patch it and use B=0 for variable args
        let b_field = if let Some(call_pc) = last_call_pc {
            // Patch inner call to return variable results
            let mut inner_call = self.fs().proto.code[call_pc];
            inner_call.set_c(0); // C=0 means return all results
            self.fs_mut().proto.code[call_pc] = inner_call;
            0u8 // B=0 means variable args from previous call
        } else if last_is_vararg {
            // Patch VARG instruction to return all varargs (B=0)
            if let Some(vararg_pc) = last_vararg_pc {
                let mut varg_instr = self.fs().proto.code[vararg_pc];
                varg_instr.set_b(0); // B=0 means return all varargs
                self.fs_mut().proto.code[vararg_pc] = varg_instr;
            }
            0u8 // B=0 means variable args from vararg
        } else {
            num_args + 1 // Normal: B = fixed args + 1
        };

        // Emit CALL
        let call_pc = self.fs().current_pc();
        self.fs_mut().emit(
            Instruction::abc(Opcode::CALL, base, b_field, 2),
            line,
        );
        self.fs_mut().free_reg = base + 1;

        Ok(ExprDesc::Call(base, 1, call_pc))
    }

    fn parse_primary_expr(&mut self) -> LuaResult<ExprDesc> {
        match &self.lexer.peek()?.kind.clone() {
            TokenKind::LParen => {
                self.lexer.next()?;
                let expr = self.parse_expression()?;
                self.lexer.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            TokenKind::Name(name) => {
                let name = name.clone();
                self.lexer.next()?;
                self.resolve_name(&name)
            }
            TokenKind::Nil => {
                self.lexer.next()?;
                Ok(ExprDesc::Nil)
            }
            TokenKind::True => {
                self.lexer.next()?;
                Ok(ExprDesc::Bool(true))
            }
            TokenKind::False => {
                self.lexer.next()?;
                Ok(ExprDesc::Bool(false))
            }
            TokenKind::Number(n) => {
                let n = *n;
                self.lexer.next()?;
                Ok(ExprDesc::Number(n))
            }
            TokenKind::String(s) => {
                let s = s.clone();
                self.lexer.next()?;
                let idx = self.fs_mut().add_string_constant(&s);
                Ok(ExprDesc::String(idx))
            }
            TokenKind::DotDotDot => {
                self.lexer.next()?;
                Ok(ExprDesc::Vararg)
            }
            TokenKind::LBrace => {
                self.parse_table_constructor()
            }
            TokenKind::Function => {
                self.lexer.next()?;
                let proto = self.parse_function_body(false)?;

                // Add child proto to parent and get index
                let proto_idx = self.fs().proto.child_protos.len();
                self.fs_mut().proto.child_protos.push(Box::new(proto));

                let line = self.current_line();
                let reg = self.fs_mut().reserve_reg();
                self.fs_mut().emit(Instruction::ad(Opcode::FNEW, reg, proto_idx as u16), line);
                Ok(ExprDesc::Register(reg))
            }
            _ => Err(LuaError::SyntaxError("unexpected symbol".to_string())),
        }
    }

    fn resolve_name(&mut self, name: &str) -> LuaResult<ExprDesc> {
        // Check locals in current function
        if let Some(slot) = self.fs().find_local(name) {
            return Ok(ExprDesc::Register(slot));
        }

        // Check if already captured as upvalue
        for (i, (uv_name, _, _)) in self.fs().upvalues.iter().enumerate() {
            if uv_name == name {
                return Ok(ExprDesc::Upvalue(i as u8));
            }
        }

        // Try to capture from parent scopes
        if let Some(uv_idx) = self.capture_upvalue(name) {
            return Ok(ExprDesc::Upvalue(uv_idx));
        }

        // Global
        let idx = self.fs_mut().add_string_constant(name);
        Ok(ExprDesc::Global(idx))
    }

    /// Try to capture a variable from parent scopes as an upvalue
    fn capture_upvalue(&mut self, name: &str) -> Option<u8> {
        let num_functions = self.functions.len();
        if num_functions < 2 {
            return None;
        }

        // Search from innermost parent outward
        // First, find which level has the variable
        let mut found_level = None;
        let mut in_stack = false;
        let mut index = 0u8;

        for level in (0..num_functions - 1).rev() {
            let fs = &self.functions[level];

            // Check if it's a local at this level
            if let Some(slot) = fs.find_local(name) {
                found_level = Some(level);
                in_stack = true;
                index = slot;
                break;
            }

            // Check if it's already an upvalue at this level
            for (i, (uv_name, _, _)) in fs.upvalues.iter().enumerate() {
                if uv_name == name {
                    found_level = Some(level);
                    in_stack = false;
                    index = i as u8;
                    break;
                }
            }
            if found_level.is_some() {
                break;
            }
        }

        let found_level = found_level?;

        // Now propagate the upvalue through all intermediate levels
        // If found at level N, we need to add upvalues from N+1 to current
        let mut current_in_stack = in_stack;
        let mut current_index = index;

        // Mark the local as captured if it's a local
        if in_stack {
            for local in &mut self.functions[found_level].locals {
                if local.name == name {
                    local.is_captured = true;
                    break;
                }
            }
        }

        // Propagate through intermediate levels
        for level in (found_level + 1)..num_functions {
            let fs = &mut self.functions[level];

            // Add upvalue to this level
            let uv_idx = fs.upvalues.len() as u8;
            fs.upvalues.push((name.to_string(), current_in_stack, current_index));
            fs.proto.upvalues.push(UpvalueDesc {
                in_stack: current_in_stack,
                index: current_index,
                name: None,
            });

            // For next level, reference this level's upvalue
            current_in_stack = false;
            current_index = uv_idx;
        }

        Some(current_index)
    }

    fn parse_table_constructor(&mut self) -> LuaResult<ExprDesc> {
        self.lexer.expect(TokenKind::LBrace)?;

        let line = self.current_line();
        let reg = self.fs_mut().reserve_reg();

        // Emit TNEW (we'll patch array/hash sizes later)
        self.fs_mut().emit(Instruction::ad(Opcode::TNEW, reg, 0), line);

        let mut array_count = 0u32;
        let mut hash_count = 0u32;

        loop {
            if self.lexer.check(&TokenKind::RBrace)? {
                break;
            }

            match &self.lexer.peek()?.kind.clone() {
                TokenKind::LBracket => {
                    // [key] = value
                    self.lexer.next()?;
                    let key = self.parse_expression()?;
                    let key_reg = self.expr_to_register(key)?;
                    self.lexer.expect(TokenKind::RBracket)?;
                    self.lexer.expect(TokenKind::Eq)?;
                    let val = self.parse_expression()?;
                    let val_reg = self.expr_to_register(val)?;

                    let line = self.current_line();
                    self.fs_mut().emit(
                        Instruction::abc(Opcode::TSETV, reg, val_reg, key_reg),
                        line,
                    );
                    hash_count += 1;
                }
                TokenKind::Name(name) => {
                    let name = name.clone();
                    // Peek ahead to see if this is `name = value` pattern
                    // We need to save position in case we need to parse as expression
                    self.lexer.next()?;
                    if self.lexer.check(&TokenKind::Eq)? {
                        // name = value (hash field)
                        self.lexer.next()?;
                        let val = self.parse_expression()?;
                        let val_reg = self.expr_to_register(val)?;
                        let key_idx = self.fs_mut().add_string_constant(&name);

                        let line = self.current_line();
                        self.fs_mut().emit(
                            Instruction::abc(Opcode::TSETS, reg, val_reg, key_idx as u8),
                            line,
                        );
                        hash_count += 1;
                    } else {
                        // Array element - name is start of a suffixed expression
                        // Resolve name first to get primary
                        let mut expr = self.resolve_name(&name)?;

                        // Handle suffixes (indexing, calls) just like parse_suffixed_expr
                        loop {
                            match &self.lexer.peek()?.kind {
                                TokenKind::Dot => {
                                    self.lexer.next()?;
                                    let field = match self.lexer.next()?.kind {
                                        TokenKind::Name(n) => n,
                                        _ => return Err(LuaError::SyntaxError("expected name after '.'".to_string())),
                                    };
                                    let table_reg = self.expr_to_register(expr)?;
                                    let key_idx = self.fs_mut().add_string_constant(&field);
                                    expr = ExprDesc::Index {
                                        table: table_reg,
                                        key: key_idx as u8,
                                        key_is_const: true,
                                    };
                                }
                                TokenKind::LBracket => {
                                    self.lexer.next()?;
                                    let table_reg = self.expr_to_register(expr)?;
                                    let key_expr = self.parse_expression()?;
                                    let key_reg = self.expr_to_register(key_expr)?;
                                    self.lexer.expect(TokenKind::RBracket)?;
                                    expr = ExprDesc::Index {
                                        table: table_reg,
                                        key: key_reg,
                                        key_is_const: false,
                                    };
                                }
                                TokenKind::LParen | TokenKind::LBrace | TokenKind::String(_) => {
                                    expr = self.parse_call_expr(expr, false)?;
                                }
                                TokenKind::Colon => {
                                    self.lexer.next()?;
                                    let method_name = match self.lexer.next()?.kind {
                                        TokenKind::Name(n) => n,
                                        _ => return Err(LuaError::SyntaxError("expected method name".to_string())),
                                    };
                                    let table_reg = self.expr_to_register(expr)?;
                                    let method_idx = self.fs_mut().add_string_constant(&method_name);
                                    let method_expr = ExprDesc::Index {
                                        table: table_reg,
                                        key: method_idx as u8,
                                        key_is_const: true,
                                    };
                                    expr = self.parse_method_call_expr(method_expr, table_reg)?;
                                }
                                _ => break,
                            }
                        }

                        // Continue with binary operations if any
                        let primary_reg = self.expr_to_register(expr)?;
                        let val = self.parse_subexpr_with_primary(primary_reg, 0)?;
                        let val_reg = self.expr_to_register(val)?;
                        array_count += 1;

                        let line = self.current_line();
                        self.fs_mut().emit(
                            Instruction::abc(Opcode::TSETB, reg, val_reg, array_count as u8),
                            line,
                        );
                    }
                }
                _ => {
                    // Array element
                    let val = self.parse_expression()?;
                    let line = self.current_line();

                    // Check if this is vararg (...) - handle specially for multi-value expansion
                    match val {
                        ExprDesc::Vararg => {
                            // Check if this is the last element (no comma/semicolon before })
                            let is_last = !self.lexer.check(&TokenKind::Comma)? &&
                                         !self.lexer.check(&TokenKind::Semicolon)?;
                            if is_last {
                                // Use VARG with B=0 (variable count) starting at reg+1
                                self.fs_mut().emit(
                                    Instruction::abc(Opcode::VARG, reg + 1, 0, 0),
                                    line,
                                );
                                // Use TSETM to set multiple values from reg+1 to top
                                // D = array_count + 1 (next index, 1-based)
                                self.fs_mut().emit(
                                    Instruction::ad(Opcode::TSETM, reg, (array_count + 1) as u16),
                                    line,
                                );
                                // Don't increment array_count as we don't know how many were added
                            } else {
                                // Not last element, just take first vararg value
                                let val_reg = self.fs_mut().reserve_reg();
                                self.fs_mut().emit(
                                    Instruction::abc(Opcode::VARG, val_reg, 2, 0),
                                    line,
                                );
                                array_count += 1;
                                self.fs_mut().emit(
                                    Instruction::abc(Opcode::TSETB, reg, val_reg, array_count as u8),
                                    line,
                                );
                            }
                        }
                        _ => {
                            let val_reg = self.expr_to_register(val)?;
                            array_count += 1;
                            self.fs_mut().emit(
                                Instruction::abc(Opcode::TSETB, reg, val_reg, array_count as u8),
                                line,
                            );
                        }
                    }
                }
            }

            // Optional separator
            if !self.lexer.match_token(&TokenKind::Comma)? &&
               !self.lexer.match_token(&TokenKind::Semicolon)? {
                break;
            }
        }

        self.lexer.expect(TokenKind::RBrace)?;
        Ok(ExprDesc::Register(reg))
    }

    fn parse_function_body(&mut self, is_method: bool) -> LuaResult<Proto> {
        self.lexer.expect(TokenKind::LParen)?;

        // Parse parameters
        let mut params = Vec::new();
        let mut is_vararg = false;

        // For methods, implicitly add 'self' as first parameter
        if is_method {
            params.push("self".to_string());
        }

        if !self.lexer.check(&TokenKind::RParen)? {
            loop {
                match &self.lexer.peek()?.kind.clone() {
                    TokenKind::Name(name) => {
                        params.push(name.clone());
                        self.lexer.next()?;
                    }
                    TokenKind::DotDotDot => {
                        self.lexer.next()?;
                        is_vararg = true;
                        break;
                    }
                    _ => return Err(LuaError::SyntaxError("expected parameter name".to_string())),
                }
                if !self.lexer.match_token(&TokenKind::Comma)? {
                    break;
                }
            }
        }

        self.lexer.expect(TokenKind::RParen)?;

        // Create new function state
        let mut new_fs = FunctionState::new();
        new_fs.num_params = params.len() as u8;
        new_fs.is_vararg = is_vararg;

        // Add parameters as locals
        for (i, name) in params.iter().enumerate() {
            new_fs.locals.push(LocalVar {
                name: name.clone(),
                slot: i as u8,
                start_pc: 0,
                is_captured: false,
            });
        }
        new_fs.free_reg = params.len() as u8;

        // Emit function header
        let header_op = if is_vararg { Opcode::FUNCV } else { Opcode::FUNCF };
        new_fs.emit(Instruction::ad(header_op, new_fs.free_reg, 0), 1);

        // Swap to new function
        self.functions.push(new_fs);

        // Parse body
        self.parse_block()?;
        self.lexer.expect(TokenKind::End)?;

        // Emit return if not present
        let line = self.current_line();
        self.fs_mut().emit(Instruction::ad(Opcode::RET0, 0, 1), line);

        // Pop function state and copy metadata to proto
        let mut fs = self.functions.pop().unwrap();
        fs.proto.num_params = fs.num_params;
        fs.proto.is_vararg = fs.is_vararg;
        Ok(fs.proto)
    }

    /// Expression to register helpers
    fn expr_to_register(&mut self, expr: ExprDesc) -> LuaResult<u8> {
        match expr {
            ExprDesc::Register(r) => Ok(r),
            ExprDesc::Nil => {
                let reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                self.fs_mut().emit(Instruction::ad(Opcode::KPRI, reg, 0), line);
                Ok(reg)
            }
            ExprDesc::Bool(b) => {
                let reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                let pri = if b { 2 } else { 1 };
                self.fs_mut().emit(Instruction::ad(Opcode::KPRI, reg, pri), line);
                Ok(reg)
            }
            ExprDesc::Number(n) => {
                let reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                if n >= i16::MIN as f64 && n <= i16::MAX as f64 && n.fract() == 0.0 {
                    self.fs_mut().emit(Instruction::ad(Opcode::KSHORT, reg, n as i16 as u16), line);
                } else {
                    let idx = self.fs_mut().add_constant(Value::number(n));
                    self.fs_mut().emit(Instruction::ad(Opcode::KNUM, reg, idx as u16), line);
                }
                Ok(reg)
            }
            ExprDesc::String(idx) => {
                let reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                self.fs_mut().emit(Instruction::ad(Opcode::KSTR, reg, idx as u16), line);
                Ok(reg)
            }
            ExprDesc::Upvalue(uv) => {
                let reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                self.fs_mut().emit(Instruction::ad(Opcode::UGET, reg, uv as u16), line);
                Ok(reg)
            }
            ExprDesc::Global(idx) => {
                let reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                self.fs_mut().emit(Instruction::ad(Opcode::GGET, reg, idx as u16), line);
                Ok(reg)
            }
            ExprDesc::Index { table, key, key_is_const } => {
                let reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                if key_is_const {
                    self.fs_mut().emit(Instruction::abc(Opcode::TGETS, reg, table, key), line);
                } else {
                    self.fs_mut().emit(Instruction::abc(Opcode::TGETV, reg, table, key), line);
                }
                Ok(reg)
            }
            ExprDesc::Call(base, _, _) => Ok(base),
            ExprDesc::Vararg => {
                let reg = self.fs_mut().reserve_reg();
                let line = self.current_line();
                self.fs_mut().emit(Instruction::abc(Opcode::VARG, reg, 2, 0), line);
                Ok(reg)
            }
            _ => Err(LuaError::SyntaxError("cannot convert to register".to_string())),
        }
    }

    fn expr_to_next_reg(&mut self, expr: ExprDesc) -> LuaResult<u8> {
        let target = self.fs().free_reg;
        let result = self.expr_to_reg(expr, target)?;
        // Ensure free_reg is advanced past the target
        if self.fs().free_reg <= target {
            self.fs_mut().free_reg = target + 1;
        }
        Ok(result)
    }

    fn expr_to_reg(&mut self, expr: ExprDesc, target: u8) -> LuaResult<u8> {
        let line = self.current_line();

        // For simple expressions, emit directly to target register
        // This avoids unnecessary temp register + MOV sequences
        match expr {
            ExprDesc::Register(r) => {
                if r != target {
                    self.fs_mut().emit(Instruction::ad(Opcode::MOV, target, r as u16), line);
                }
                Ok(target)
            }
            ExprDesc::Nil => {
                self.fs_mut().emit(Instruction::ad(Opcode::KPRI, target, 0), line);
                Ok(target)
            }
            ExprDesc::Bool(b) => {
                let pri = if b { 2 } else { 1 };
                self.fs_mut().emit(Instruction::ad(Opcode::KPRI, target, pri), line);
                Ok(target)
            }
            ExprDesc::Number(n) => {
                if n >= i16::MIN as f64 && n <= i16::MAX as f64 && n.fract() == 0.0 {
                    self.fs_mut().emit(Instruction::ad(Opcode::KSHORT, target, n as i16 as u16), line);
                } else {
                    let idx = self.fs_mut().add_constant(Value::number(n));
                    self.fs_mut().emit(Instruction::ad(Opcode::KNUM, target, idx as u16), line);
                }
                Ok(target)
            }
            ExprDesc::String(idx) => {
                self.fs_mut().emit(Instruction::ad(Opcode::KSTR, target, idx as u16), line);
                Ok(target)
            }
            ExprDesc::Upvalue(uv) => {
                self.fs_mut().emit(Instruction::ad(Opcode::UGET, target, uv as u16), line);
                Ok(target)
            }
            ExprDesc::Global(idx) => {
                self.fs_mut().emit(Instruction::ad(Opcode::GGET, target, idx as u16), line);
                Ok(target)
            }
            ExprDesc::Index { table, key, key_is_const } => {
                if key_is_const {
                    self.fs_mut().emit(Instruction::abc(Opcode::TGETS, target, table, key), line);
                } else {
                    self.fs_mut().emit(Instruction::abc(Opcode::TGETV, target, table, key), line);
                }
                Ok(target)
            }
            ExprDesc::Call(base, _, _) => {
                if base != target {
                    self.fs_mut().emit(Instruction::ad(Opcode::MOV, target, base as u16), line);
                }
                Ok(target)
            }
            ExprDesc::Vararg => {
                self.fs_mut().emit(Instruction::abc(Opcode::VARG, target, 2, 0), line);
                Ok(target)
            }
            _ => Err(LuaError::SyntaxError("cannot convert to register".to_string())),
        }
    }

    fn patch_jump(&mut self, pc: usize, target: usize) -> LuaResult<()> {
        let offset = target as i32 - pc as i32 - 1;
        if offset < i16::MIN as i32 || offset > i16::MAX as i32 {
            return Err(LuaError::SyntaxError("jump too large".to_string()));
        }
        self.fs_mut().proto.code[pc].set_d(offset as u16);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn test_compile_simple() {
        let proto = parse("local x = 1 + 2", "test").unwrap();
        assert!(!proto.code.is_empty());
    }

    #[test]
    fn test_compile_function() {
        let proto = parse("function foo() return 1 end", "test").unwrap();
        assert!(!proto.code.is_empty());
    }

    #[test]
    fn test_compile_if() {
        let proto = parse("if true then x = 1 else x = 2 end", "test").unwrap();
        assert!(!proto.code.is_empty());
    }

    #[test]
    fn test_compile_while() {
        let proto = parse("while true do break end", "test").unwrap();
        assert!(!proto.code.is_empty());
    }
}
