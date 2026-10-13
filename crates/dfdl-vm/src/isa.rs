//! DFDL Virtual Machine Instruction Set Architecture (DFDL-ISA).
//!
//! Defines virtual registers, opcodes, symbol descriptors, constant pools,
//! and linear bytecode programs for deterministic binary bitstream parsing.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::types::QName;

/// Virtual general-purpose register identifier.
///
/// The DFDL virtual machine provides 8 general-purpose 64-bit registers
/// (`R0` through `R7`) used for intermediate scalar values, bit buffers,
/// loop counters, and branch discriminators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Register {
    /// General purpose register 0.
    R0 = 0,
    /// General purpose register 1.
    R1 = 1,
    /// General purpose register 2.
    R2 = 2,
    /// General purpose register 3.
    R3 = 3,
    /// General purpose register 4.
    R4 = 4,
    /// General purpose register 5.
    R5 = 5,
    /// General purpose register 6.
    R6 = 6,
    /// General purpose register 7.
    R7 = 7,
}

impl Register {
    /// Returns the numerical register index in the range `0..8`.
    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Strongly-typed index into the program's symbol table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

/// Strongly-typed index into the program's constant pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConstantPoolIndex(pub u32);

/// Individual instruction in the linear DFDL bytecode program.
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    /// Reads `count` bits (1..=8) into the destination register as unsigned integer.
    ReadBitsU8 {
        /// Destination register.
        dest: Register,
        /// Number of bits to read (1..=8).
        count: u8,
    },
    /// Reads `count` bits (1..=16) into the destination register as unsigned integer.
    ReadBitsU16 {
        /// Destination register.
        dest: Register,
        /// Number of bits to read (1..=16).
        count: u8,
    },
    /// Reads `count` bits (1..=32) into the destination register as unsigned integer.
    ReadBitsU32 {
        /// Destination register.
        dest: Register,
        /// Number of bits to read (1..=32).
        count: u8,
    },
    /// Reads `count` bits (1..=64) into the destination register as unsigned integer.
    ReadBitsU64 {
        /// Destination register.
        dest: Register,
        /// Number of bits to read (1..=64).
        count: u8,
    },
    /// Reads `count` bits (1..=64) into the destination register and sign-extends to 64-bit.
    ReadBitsSigned {
        /// Destination register.
        dest: Register,
        /// Number of bits to read (1..=64).
        count: u8,
    },
    /// Reads 16 bits in Big-Endian format directly into the destination register.
    ReadBeU16 {
        /// Destination register.
        dest: Register,
    },
    /// Reads 16 bits in Little-Endian format directly into the destination register.
    ReadLeU16 {
        /// Destination register.
        dest: Register,
    },
    /// Reads 32 bits in Big-Endian format directly into the destination register.
    ReadBeU32 {
        /// Destination register.
        dest: Register,
    },
    /// Reads 32 bits in Little-Endian format directly into the destination register.
    ReadLeU32 {
        /// Destination register.
        dest: Register,
    },
    /// Reads 64 bits in Big-Endian format directly into the destination register.
    ReadBeU64 {
        /// Destination register.
        dest: Register,
    },
    /// Reads 64 bits in Little-Endian format directly into the destination register.
    ReadLeU64 {
        /// Destination register.
        dest: Register,
    },
    /// Reads a 16-bit signed integer in Big-Endian format.
    ReadBeI16 {
        /// Destination register.
        dest: Register,
    },
    /// Reads a 16-bit signed integer in Little-Endian format.
    ReadLeI16 {
        /// Destination register.
        dest: Register,
    },
    /// Reads a 32-bit signed integer in Big-Endian format.
    ReadBeI32 {
        /// Destination register.
        dest: Register,
    },
    /// Reads a 32-bit signed integer in Little-Endian format.
    ReadLeI32 {
        /// Destination register.
        dest: Register,
    },
    /// Reads a 64-bit signed integer in Big-Endian format.
    ReadBeI64 {
        /// Destination register.
        dest: Register,
    },
    /// Reads a 64-bit signed integer in Little-Endian format.
    ReadLeI64 {
        /// Destination register.
        dest: Register,
    },
    /// Reads an IEEE 754 32-bit single-precision float in Big-Endian into register bitwise.
    ReadBeF32 {
        /// Destination register holding the raw IEEE 754 bits.
        dest: Register,
    },
    /// Reads an IEEE 754 32-bit single-precision float in Little-Endian into register bitwise.
    ReadLeF32 {
        /// Destination register holding the raw IEEE 754 bits.
        dest: Register,
    },
    /// Reads an IEEE 754 64-bit double-precision float in Big-Endian into register bitwise.
    ReadBeF64 {
        /// Destination register holding the raw IEEE 754 bits.
        dest: Register,
    },
    /// Reads an IEEE 754 64-bit double-precision float in Little-Endian into register bitwise.
    ReadLeF64 {
        /// Destination register holding the raw IEEE 754 bits.
        dest: Register,
    },
    /// Loads an immediate 32-bit value into a register.
    LoadImm {
        /// Destination register.
        dest: Register,
        /// Immediate value.
        imm: u32,
    },
    /// Adds an immediate value to a register using wrapping arithmetic.
    AddImm {
        /// Target register.
        dest: Register,
        /// Immediate value.
        imm: u32,
    },
    /// Subtracts an immediate value from a register using wrapping arithmetic.
    SubImm {
        /// Target register.
        dest: Register,
        /// Immediate value.
        imm: u32,
    },
    /// Reads a fixed sequence of bytes into the constant pool buffer.
    ReadBytes {
        /// Destination constant pool buffer index.
        dest_const: ConstantPoolIndex,
        /// Number of bytes to read.
        len: u32,
    },
    /// Aligns the bitstream to the specified alignment boundary in bits.
    AlignBits {
        /// Alignment in bits (e.g., 8 for byte alignment, 32 for word).
        alignment_bits: u16,
    },
    /// Advances the bitstream by `bit_count` bits without capturing.
    SkipBits {
        /// Number of bits to skip.
        bit_count: u32,
    },
    /// Saves a Point of Uncertainty (POU) checkpoint with rollback address.
    SavePou {
        /// Instruction pointer to jump to upon choice branch failure.
        rollback_ip: u32,
    },
    /// Commits and discards the most recent POU checkpoint.
    CommitPou,
    /// Rolls back the bitstream and infoset to the most recent POU checkpoint.
    RollbackPou,
    /// Unconditional jump to `target_ip`.
    Jump {
        /// Destination instruction address.
        target_ip: u32,
    },
    /// Conditional branch: jumps to `target_ip` if `reg == imm`.
    JumpIfEq {
        /// Register to compare.
        reg: Register,
        /// Immediate value to test against.
        imm: u32,
        /// Target address if equal.
        target_ip: u32,
    },
    /// Conditional branch: jumps to `target_ip` if `reg != imm`.
    JumpIfNe {
        /// Register to compare.
        reg: Register,
        /// Immediate value to test against.
        imm: u32,
        /// Target address if not equal.
        target_ip: u32,
    },
    /// Signals the start of a complex element container.
    StartElement {
        /// Symbol identifier of the element.
        symbol_id: SymbolId,
    },
    /// Signals the end of a complex element container.
    EndElement {
        /// Symbol identifier of the element.
        symbol_id: SymbolId,
    },
    /// Emits a simple scalar integer value from register.
    EmitScalarInt {
        /// Symbol identifier.
        symbol_id: SymbolId,
        /// Register containing the integer value.
        reg: Register,
        /// Indicates if the value is signed.
        is_signed: bool,
    },
    /// Emits a simple scalar float value from register.
    EmitScalarFloat {
        /// Symbol identifier.
        symbol_id: SymbolId,
        /// Register containing the raw IEEE 754 bits.
        reg: Register,
        /// True if 64-bit double, false if 32-bit single.
        is_f64: bool,
    },
    /// Emits a simple boolean scalar value.
    EmitScalarBool {
        /// Symbol identifier.
        symbol_id: SymbolId,
        /// Boolean value.
        val: bool,
    },
    /// Emits a simple string scalar value from constant pool or byte slice.
    EmitScalarString {
        /// Symbol identifier.
        symbol_id: SymbolId,
        /// Constant pool index containing the string.
        const_idx: ConstantPoolIndex,
    },
    /// Reads a fixed-length string from the bitstream and emits a string scalar event.
    ReadString {
        /// Associated symbol identifier.
        symbol_id: SymbolId,
        /// Number of bytes to read.
        len: u32,
        /// If true, trims trailing NUL (0) padding bytes.
        trim_nul: bool,
    },
    /// Reads a 3-byte BCD timestamp (hhmmss) and emits as "hh:mm:ss" string scalar event.
    ReadBcdTime {
        /// Associated symbol identifier.
        symbol_id: SymbolId,
    },
    /// Checks EOF condition on the input bitstream.
    CheckEof {
        /// If true, succeeds if EOF is reached. If false, fails if EOF is reached.
        expected_eof: bool,
    },
    /// Jumps to `target_ip` if the input bitstream has reached EOF.
    JumpIfEof {
        /// Target instruction address.
        target_ip: u32,
    },
    /// Copies 64-bit value from `src` register to `dest` register.
    MoveReg {
        /// Target destination register.
        dest: Register,
        /// Source register.
        src: Register,
    },
    /// Extracts a single bit from `src` register at `bit` position (0..=63) and sets `dest` to 0 or 1.
    TestBit {
        /// Destination register receiving the extracted single-bit value (0 or 1).
        dest: Register,
        /// Source register containing the integer or bitmask value.
        src: Register,
        /// 0-indexed bit position to extract (0 = LSB, up to 63).
        bit: u8,
    },
    /// Peeks `count` bits (1..=8) into destination register without advancing the bitstream.
    PeekBitsU8 {
        /// Destination register.
        dest: Register,
        /// Number of bits to peek (1..=8).
        count: u8,
    },
    /// Records the current bitstream bit position into the destination register.
    RecordPosition {
        /// Destination register.
        dest: Register,
    },
    /// Advances the bitstream to ensure a total of `total_bytes` have been consumed since `start_reg`.
    PadToLength {
        /// Register containing the starting bit position.
        start_reg: Register,
        /// Expected total length in bytes.
        total_bytes: u32,
    },
    /// Fused: Reads a 16-bit Big-Endian integer and directly emits a scalar int event.
    ReadEmitBeU16 {
        /// Associated symbol identifier.
        symbol_id: SymbolId,
        /// Indicates if the value is signed.
        is_signed: bool,
    },
    /// Fused: Reads a 16-bit Little-Endian integer and directly emits a scalar int event.
    ReadEmitLeU16 {
        /// Associated symbol identifier.
        symbol_id: SymbolId,
        /// Indicates if the value is signed.
        is_signed: bool,
    },
    /// Fused: Reads a 32-bit Big-Endian integer and directly emits a scalar int event.
    ReadEmitBeU32 {
        /// Associated symbol identifier.
        symbol_id: SymbolId,
        /// Indicates if the value is signed.
        is_signed: bool,
    },
    /// Fused: Reads a 32-bit Little-Endian integer and directly emits a scalar int event.
    ReadEmitLeU32 {
        /// Associated symbol identifier.
        symbol_id: SymbolId,
        /// Indicates if the value is signed.
        is_signed: bool,
    },
    /// Fused: Reads `count` bits (1..=8) and directly emits a scalar int event.
    ReadEmitBitsU8 {
        /// Associated symbol identifier.
        symbol_id: SymbolId,
        /// Number of bits to read (1..=8).
        count: u8,
        /// Indicates if the value is signed.
        is_signed: bool,
    },
    /// Fused: Reads `count` bits (1..=8), compares equality to `true_val`, and emits boolean scalar event.
    ReadEmitBool {
        /// Associated symbol identifier.
        symbol_id: SymbolId,
        /// Number of bits to read (1..=8).
        count: u8,
        /// Wire byte value representing boolean true.
        true_val: u8,
    },
    /// Fused contiguous struct/block reader: reads and emits multiple fields in a tight loop.
    ReadBlock {
        /// Starting index into `program.block_fields`.
        start_idx: u32,
        /// Number of contiguous fields in this block.
        field_count: u16,
    },
    /// Halts program execution with success.
    HaltSuccess,
    /// Halts program execution with error code.
    HaltError {
        /// Numerical error code.
        error_code: u32,
    },
}

/// Primitive scalar field descriptor within a fused contiguous block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockFieldKind {
    /// Sub-byte or 8-bit integer with bit width `count`.
    BitsU8 {
        /// Bit count (1..=8).
        count: u8,
        /// True if signed two's complement.
        is_signed: bool,
    },
    /// 16-bit big-endian integer.
    BeU16 {
        /// True if signed two's complement.
        is_signed: bool,
    },
    /// 16-bit little-endian integer.
    LeU16 {
        /// True if signed two's complement.
        is_signed: bool,
    },
    /// 32-bit big-endian integer.
    BeU32 {
        /// True if signed two's complement.
        is_signed: bool,
    },
    /// 32-bit little-endian integer.
    LeU32 {
        /// True if signed two's complement.
        is_signed: bool,
    },
    /// 64-bit big-endian integer.
    BeU64 {
        /// True if signed two's complement.
        is_signed: bool,
    },
    /// 64-bit little-endian integer.
    LeU64 {
        /// True if signed two's complement.
        is_signed: bool,
    },
    /// Boolean scalar represented by bit pattern.
    Bool {
        /// Bit count (1..=8).
        count: u8,
        /// Value representing true.
        true_val: u8,
    },
    /// Fixed-length string.
    FixedString {
        /// Byte length.
        len: u8,
        /// Trim trailing null padding.
        trim_nul: bool,
    },
    /// Binary Coded Decimal (BCD) 3-byte HH:MM:SS time string.
    BcdTime,
}

/// Field descriptor in a fused contiguous block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockField {
    /// Symbol ID for event emission.
    pub symbol_id: SymbolId,
    /// Field type and physical representation.
    pub kind: BlockFieldKind,
}

/// Symbol table mapping numerical [`SymbolId`] to qualified element names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolTable {
    names: Vec<QName>,
}

impl SymbolTable {
    /// Creates an empty symbol table.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self { names: Vec::new() }
    }

    /// Registers a QName and returns its assigned [`SymbolId`].
    pub fn intern(&mut self, name: QName) -> SymbolId {
        if let Some(pos) = self.names.iter().position(|n| n == &name) {
            SymbolId(pos as u32)
        } else {
            let id = self.names.len() as u32;
            self.names.push(name);
            SymbolId(id)
        }
    }

    /// Resolves a [`SymbolId`] to its registered [`QName`].
    #[must_use]
    pub fn resolve(&self, id: SymbolId) -> Option<&QName> {
        self.names.get(id.0 as usize)
    }

    /// Returns the total number of registered symbols.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Returns true if no symbols are registered.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Constant pool storing string literals, binary signatures, and lookup tables.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConstantPool {
    strings: Vec<String>,
    bytes: Vec<Vec<u8>>,
}

impl ConstantPool {
    /// Creates an empty constant pool.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            strings: Vec::new(),
            bytes: Vec::new(),
        }
    }

    /// Interns a string into the pool, returning its index.
    pub fn add_string(&mut self, s: String) -> ConstantPoolIndex {
        if let Some(pos) = self.strings.iter().position(|existing| existing == &s) {
            ConstantPoolIndex(pos as u32)
        } else {
            let idx = self.strings.len() as u32;
            self.strings.push(s);
            ConstantPoolIndex(idx)
        }
    }

    /// Retrieves a string constant by index.
    #[must_use]
    pub fn get_string(&self, idx: ConstantPoolIndex) -> Option<&str> {
        self.strings.get(idx.0 as usize).map(|s| s.as_str())
    }

    /// Adds raw byte data into the pool, returning its index.
    pub fn add_bytes(&mut self, b: Vec<u8>) -> ConstantPoolIndex {
        let idx = self.bytes.len() as u32;
        self.bytes.push(b);
        ConstantPoolIndex(idx)
    }

    /// Retrieves raw byte data by index.
    #[must_use]
    pub fn get_bytes(&self, idx: ConstantPoolIndex) -> Option<&[u8]> {
        self.bytes.get(idx.0 as usize).map(|b| b.as_slice())
    }
}

/// Complete compiled DFDL linear bytecode program.
#[derive(Debug, Clone, PartialEq)]
pub struct BytecodeProgram {
    /// Sequence of executable instructions.
    pub instructions: Vec<Instruction>,
    /// Symbol table.
    pub symbols: SymbolTable,
    /// Constant pool.
    pub constants: ConstantPool,
    /// Descriptors of fields for fused contiguous block reads.
    pub block_fields: Vec<BlockField>,
}

impl BytecodeProgram {
    /// Constructs a new empty bytecode program.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            instructions: Vec::new(),
            symbols: SymbolTable::new(),
            constants: ConstantPool::new(),
            block_fields: Vec::new(),
        }
    }

    /// Appends an instruction to the program and returns its address.
    pub fn emit(&mut self, inst: Instruction) -> usize {
        let addr = self.instructions.len();
        self.instructions.push(inst);
        addr
    }

    /// Updates the target IP of an existing instruction (backpatching).
    pub fn patch_target(&mut self, addr: usize, target: u32) {
        if let Some(inst) = self.instructions.get_mut(addr) {
            match inst {
                Instruction::Jump { ref mut target_ip } => *target_ip = target,
                Instruction::JumpIfEq { ref mut target_ip, .. } => *target_ip = target,
                Instruction::JumpIfNe { ref mut target_ip, .. } => *target_ip = target,
                Instruction::JumpIfEof { ref mut target_ip } => *target_ip = target,
                Instruction::SavePou { ref mut rollback_ip } => *rollback_ip = target,
                _ => {}
            }
        }
    }
}

impl Default for BytecodeProgram {
    fn default() -> Self {
        Self::new()
    }
}
