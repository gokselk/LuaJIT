//! Bytecode opcodes matching LuaJIT's instruction set.
//!
//! LuaJIT uses a register-based bytecode with two main formats:
//! - ABC: opcode(8) + A(8) + C(8) + B(8)
//! - AD:  opcode(8) + A(8) + D(16)

/// Opcode mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMode {
    /// ABC format (three 8-bit operands)
    ABC,
    /// AD format (one 8-bit and one 16-bit operand)
    AD,
    /// AJ format (8-bit operand and 16-bit signed jump offset)
    AJ,
}

/// Bytecode opcodes (matching LuaJIT structure)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    // Comparison ops (A = result slot, B/C = operands)
    ISLT = 0,    // if B < C
    ISGE = 1,    // if B >= C
    ISLE = 2,    // if B <= C
    ISGT = 3,    // if B > C
    ISEQV = 4,   // if B == C (any type)
    ISNEV = 5,   // if B != C (any type)
    ISEQS = 6,   // if B == C (string)
    ISNES = 7,   // if B != C (string)
    ISEQN = 8,   // if B == C (number)
    ISNEN = 9,   // if B != C (number)
    ISEQP = 10,  // if B == C (primitive)
    ISNEP = 11,  // if B != C (primitive)

    // Unary test and copy ops
    ISTC = 12,   // if C: A = C (copy if true)
    ISFC = 13,   // if not C: A = C (copy if false)
    IST = 14,    // if C (test)
    ISF = 15,    // if not C (test)
    ISTYPE = 16, // if type(B) == C
    ISNUM = 17,  // if isnumber(B)

    // Unary ops
    MOV = 18,    // A = D (move)
    NOT = 19,    // A = not D
    UNM = 20,    // A = -D (unary minus)
    LEN = 21,    // A = #D (length)

    // Binary ops (result in A, operands in B and C)
    ADDVN = 22,  // A = B + C (var + num)
    SUBVN = 23,  // A = B - C
    MULVN = 24,  // A = B * C
    DIVVN = 25,  // A = B / C
    MODVN = 26,  // A = B % C

    ADDNV = 27,  // A = B + C (num + var)
    SUBNV = 28,  // A = B - C
    MULNV = 29,  // A = B * C
    DIVNV = 30,  // A = B / C
    MODNV = 31,  // A = B % C

    ADDVV = 32,  // A = B + C (var + var)
    SUBVV = 33,  // A = B - C
    MULVV = 34,  // A = B * C
    DIVVV = 35,  // A = B / C
    MODVV = 36,  // A = B % C

    POW = 37,    // A = B ^ C
    CAT = 38,    // A = B .. ... .. C (concatenate)

    // Constant ops
    KSTR = 39,   // A = constant string D
    KCDATA = 40, // A = constant cdata D
    KSHORT = 41, // A = signed short D
    KNUM = 42,   // A = constant number D
    KPRI = 43,   // A = primitive D (nil/false/true)
    KNIL = 44,   // A, A+1, ..., A+D = nil

    // Upvalue and function ops
    UGET = 45,   // A = upvalue D
    USETV = 46,  // upvalue A = D
    USETS = 47,  // upvalue A = string constant D
    USETN = 48,  // upvalue A = number constant D
    USETP = 49,  // upvalue A = primitive D
    UCLO = 50,   // close upvalues >= A; jump D

    // Function ops
    FNEW = 51,   // A = new closure from prototype D

    // Table ops
    TNEW = 52,   // A = new table with array size B and hash size C
    TDUP = 53,   // A = duplicate table template D
    GGET = 54,   // A = _G[D] (global get)
    GSET = 55,   // _G[D] = A (global set)
    TGETV = 56,  // A = B[C] (table index)
    TGETS = 57,  // A = B[C] (string key)
    TGETB = 58,  // A = B[C] (byte key)
    TGETR = 59,  // A = B[C] (raw)
    TSETV = 60,  // A[B] = C
    TSETS = 61,  // A[B] = C (string key)
    TSETB = 62,  // A[B] = C (byte key)
    TSETR = 63,  // A[B] = C (raw)
    TSETM = 64,  // A[D], A[D+1], ... = A, A+1, ... (vararg table init)

    // Call/return ops
    CALLM = 65,  // call A with B-1 args, C-1 results, vararg
    CALL = 66,   // call A with B-1 args, C-1 results
    CALLMT = 67, // tailcall A with B-1 args, vararg
    CALLT = 68,  // tailcall A with B-1 args
    ITERC = 69,  // call iterator
    ITERN = 70,  // specialized next() call
    VARG = 71,   // A, A+1, ..., A+B-2 = vararg
    ISNEXT = 72, // verify next() for ITERN

    // Return ops
    RETM = 73,   // return A, A+1, ..., A+D-2, vararg
    RET = 74,    // return A, A+1, ..., A+D-2
    RET0 = 75,   // return (no values)
    RET1 = 76,   // return A

    // Loop ops
    FORI = 77,   // numeric for loop init
    JFORI = 78,  // numeric for loop init (JIT)
    FORL = 79,   // numeric for loop step
    IFORL = 80,  // numeric for loop step (interpreter)
    JFORL = 81,  // numeric for loop step (JIT)
    ITERL = 82,  // iterator for loop step
    IITERL = 83, // iterator for loop step (interpreter)
    JITERL = 84, // iterator for loop step (JIT)
    LOOP = 85,   // generic loop marker
    ILOOP = 86,  // generic loop (interpreter)
    JLOOP = 87,  // generic loop (JIT trace)

    // Jump ops
    JMP = 88,    // unconditional jump

    // Function headers
    FUNCF = 89,  // function header (fixed args)
    IFUNCF = 90, // function header (interpreter, fixed)
    JFUNCF = 91, // function header (JIT, fixed)
    FUNCV = 92,  // function header (vararg)
    IFUNCV = 93, // function header (interpreter, vararg)
    JFUNCV = 94, // function header (JIT, vararg)
    FUNCC = 95,  // C function call
    FUNCCW = 96, // C function call (with wrapper)
}

impl Opcode {
    /// Get the name of this opcode
    pub const fn name(self) -> &'static str {
        match self {
            Self::ISLT => "ISLT",
            Self::ISGE => "ISGE",
            Self::ISLE => "ISLE",
            Self::ISGT => "ISGT",
            Self::ISEQV => "ISEQV",
            Self::ISNEV => "ISNEV",
            Self::ISEQS => "ISEQS",
            Self::ISNES => "ISNES",
            Self::ISEQN => "ISEQN",
            Self::ISNEN => "ISNEN",
            Self::ISEQP => "ISEQP",
            Self::ISNEP => "ISNEP",
            Self::ISTC => "ISTC",
            Self::ISFC => "ISFC",
            Self::IST => "IST",
            Self::ISF => "ISF",
            Self::ISTYPE => "ISTYPE",
            Self::ISNUM => "ISNUM",
            Self::MOV => "MOV",
            Self::NOT => "NOT",
            Self::UNM => "UNM",
            Self::LEN => "LEN",
            Self::ADDVN => "ADDVN",
            Self::SUBVN => "SUBVN",
            Self::MULVN => "MULVN",
            Self::DIVVN => "DIVVN",
            Self::MODVN => "MODVN",
            Self::ADDNV => "ADDNV",
            Self::SUBNV => "SUBNV",
            Self::MULNV => "MULNV",
            Self::DIVNV => "DIVNV",
            Self::MODNV => "MODNV",
            Self::ADDVV => "ADDVV",
            Self::SUBVV => "SUBVV",
            Self::MULVV => "MULVV",
            Self::DIVVV => "DIVVV",
            Self::MODVV => "MODVV",
            Self::POW => "POW",
            Self::CAT => "CAT",
            Self::KSTR => "KSTR",
            Self::KCDATA => "KCDATA",
            Self::KSHORT => "KSHORT",
            Self::KNUM => "KNUM",
            Self::KPRI => "KPRI",
            Self::KNIL => "KNIL",
            Self::UGET => "UGET",
            Self::USETV => "USETV",
            Self::USETS => "USETS",
            Self::USETN => "USETN",
            Self::USETP => "USETP",
            Self::UCLO => "UCLO",
            Self::FNEW => "FNEW",
            Self::TNEW => "TNEW",
            Self::TDUP => "TDUP",
            Self::GGET => "GGET",
            Self::GSET => "GSET",
            Self::TGETV => "TGETV",
            Self::TGETS => "TGETS",
            Self::TGETB => "TGETB",
            Self::TGETR => "TGETR",
            Self::TSETV => "TSETV",
            Self::TSETS => "TSETS",
            Self::TSETB => "TSETB",
            Self::TSETR => "TSETR",
            Self::TSETM => "TSETM",
            Self::CALLM => "CALLM",
            Self::CALL => "CALL",
            Self::CALLMT => "CALLMT",
            Self::CALLT => "CALLT",
            Self::ITERC => "ITERC",
            Self::ITERN => "ITERN",
            Self::VARG => "VARG",
            Self::ISNEXT => "ISNEXT",
            Self::RETM => "RETM",
            Self::RET => "RET",
            Self::RET0 => "RET0",
            Self::RET1 => "RET1",
            Self::FORI => "FORI",
            Self::JFORI => "JFORI",
            Self::FORL => "FORL",
            Self::IFORL => "IFORL",
            Self::JFORL => "JFORL",
            Self::ITERL => "ITERL",
            Self::IITERL => "IITERL",
            Self::JITERL => "JITERL",
            Self::LOOP => "LOOP",
            Self::ILOOP => "ILOOP",
            Self::JLOOP => "JLOOP",
            Self::JMP => "JMP",
            Self::FUNCF => "FUNCF",
            Self::IFUNCF => "IFUNCF",
            Self::JFUNCF => "JFUNCF",
            Self::FUNCV => "FUNCV",
            Self::IFUNCV => "IFUNCV",
            Self::JFUNCV => "JFUNCV",
            Self::FUNCC => "FUNCC",
            Self::FUNCCW => "FUNCCW",
        }
    }

    /// Get the instruction mode for this opcode
    pub const fn mode(self) -> OpMode {
        match self {
            // Comparison ops use AD format
            Self::ISLT | Self::ISGE | Self::ISLE | Self::ISGT |
            Self::ISEQV | Self::ISNEV | Self::ISEQS | Self::ISNES |
            Self::ISEQN | Self::ISNEN | Self::ISEQP | Self::ISNEP => OpMode::AD,

            // Unary test/copy ops
            Self::ISTC | Self::ISFC => OpMode::AD,
            Self::IST | Self::ISF => OpMode::AD,
            Self::ISTYPE | Self::ISNUM => OpMode::AD,

            // Unary ops
            Self::MOV | Self::NOT | Self::UNM | Self::LEN => OpMode::AD,

            // Binary ops use ABC
            Self::ADDVN | Self::SUBVN | Self::MULVN | Self::DIVVN | Self::MODVN |
            Self::ADDNV | Self::SUBNV | Self::MULNV | Self::DIVNV | Self::MODNV |
            Self::ADDVV | Self::SUBVV | Self::MULVV | Self::DIVVV | Self::MODVV |
            Self::POW | Self::CAT => OpMode::ABC,

            // Constant ops
            Self::KSTR | Self::KCDATA | Self::KSHORT | Self::KNUM |
            Self::KPRI | Self::KNIL => OpMode::AD,

            // Upvalue ops
            Self::UGET | Self::USETV | Self::USETS | Self::USETN |
            Self::USETP | Self::UCLO => OpMode::AD,

            // Function ops
            Self::FNEW => OpMode::AD,

            // Table ops
            Self::TNEW | Self::TDUP => OpMode::AD,
            Self::GGET | Self::GSET => OpMode::AD,
            Self::TGETV | Self::TGETS | Self::TGETB | Self::TGETR |
            Self::TSETV | Self::TSETS | Self::TSETB | Self::TSETR => OpMode::ABC,
            Self::TSETM => OpMode::AD,

            // Call ops
            Self::CALLM | Self::CALL | Self::CALLMT | Self::CALLT |
            Self::ITERC | Self::ITERN => OpMode::ABC,
            Self::VARG => OpMode::ABC,
            Self::ISNEXT => OpMode::AD,

            // Return ops
            Self::RETM | Self::RET | Self::RET0 | Self::RET1 => OpMode::AD,

            // Loop ops use AJ (jump)
            Self::FORI | Self::JFORI | Self::FORL | Self::IFORL | Self::JFORL |
            Self::ITERL | Self::IITERL | Self::JITERL |
            Self::LOOP | Self::ILOOP | Self::JLOOP => OpMode::AJ,

            // Jump
            Self::JMP => OpMode::AJ,

            // Function headers
            Self::FUNCF | Self::IFUNCF | Self::JFUNCF |
            Self::FUNCV | Self::IFUNCV | Self::JFUNCV |
            Self::FUNCC | Self::FUNCCW => OpMode::AD,
        }
    }

    /// Check if this opcode is a jump
    pub const fn is_jump(self) -> bool {
        matches!(
            self,
            Self::JMP | Self::FORI | Self::JFORI |
            Self::FORL | Self::IFORL | Self::JFORL |
            Self::ITERL | Self::IITERL | Self::JITERL |
            Self::LOOP | Self::ILOOP | Self::JLOOP |
            Self::UCLO
        )
    }

    /// Check if this opcode is a conditional jump
    pub const fn is_conditional(self) -> bool {
        matches!(
            self,
            Self::ISLT | Self::ISGE | Self::ISLE | Self::ISGT |
            Self::ISEQV | Self::ISNEV | Self::ISEQS | Self::ISNES |
            Self::ISEQN | Self::ISNEN | Self::ISEQP | Self::ISNEP |
            Self::ISTC | Self::ISFC | Self::IST | Self::ISF |
            Self::FORL | Self::IFORL | Self::JFORL |
            Self::ITERL | Self::IITERL | Self::JITERL
        )
    }

    /// Check if this opcode is a return
    pub const fn is_return(self) -> bool {
        matches!(
            self,
            Self::RETM | Self::RET | Self::RET0 | Self::RET1 |
            Self::CALLMT | Self::CALLT
        )
    }

    /// Check if this opcode is a call
    pub const fn is_call(self) -> bool {
        matches!(
            self,
            Self::CALLM | Self::CALL | Self::CALLMT | Self::CALLT |
            Self::ITERC | Self::ITERN
        )
    }

    /// Check if this is a loop opcode (for JIT hot detection)
    pub const fn is_loop(self) -> bool {
        matches!(
            self,
            Self::FORL | Self::IFORL | Self::JFORL |
            Self::ITERL | Self::IITERL | Self::JITERL |
            Self::LOOP | Self::ILOOP | Self::JLOOP
        )
    }

    /// Get the number of stack slots this instruction uses
    pub const fn stack_use(self) -> i8 {
        match self {
            Self::CALL | Self::CALLM => -1, // Variable
            Self::RET | Self::RETM => -1,   // Variable
            _ => 0,
        }
    }
}

impl std::fmt::Display for Opcode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_modes() {
        assert_eq!(Opcode::ADDVV.mode(), OpMode::ABC);
        assert_eq!(Opcode::JMP.mode(), OpMode::AJ);
        assert_eq!(Opcode::MOV.mode(), OpMode::AD);
    }

    #[test]
    fn test_jump_detection() {
        assert!(Opcode::JMP.is_jump());
        assert!(Opcode::FORL.is_jump());
        assert!(!Opcode::MOV.is_jump());
    }

    #[test]
    fn test_return_detection() {
        assert!(Opcode::RET.is_return());
        assert!(Opcode::RET0.is_return());
        assert!(Opcode::CALLT.is_return());
        assert!(!Opcode::CALL.is_return());
    }
}
