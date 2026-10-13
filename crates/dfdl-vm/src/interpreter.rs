//! DFDL Virtual Machine Interpreter.
//!
//! Provides the core linear execution engine [`VmInterpreter`] which processes
//! [`BytecodeProgram`] instructions against an input [`BitReader`] and emits
//! structural events to a [`VmEventSink`].
//!
//! Backtracking at Points of Uncertainty (POU) is handled in $O(1)$ time by
//! saving and restoring bit positions and sink offsets on an internal stack,
//! with zero heap allocation during branch recovery.

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::io::bitstream::BitReader;
use dfdl_core::io::traits::{ByteSource, Checkpoint};
use dfdl_core::limits::WorkBudget;
use dfdl_core::util::try_push;

use crate::arena::{VmEvent, VmEventSink};
use crate::isa::{BlockFieldKind, BytecodeProgram, Instruction, Register};

/// Sign-extends a `bits`-wide unsigned value to a 64-bit signed integer.
///
/// In accordance with DFDL §13.7.1, 1-bit integers have no sign bit and
/// are interpreted as unsigned.
#[inline]
#[must_use]
pub const fn sign_extend(val: u64, bits: usize) -> i64 {
    if bits <= 1 || bits >= 64 {
        val as i64
    } else {
        let mask = 1u64 << (bits.saturating_sub(1));
        if (val & mask) != 0 {
            (val | (!0u64 << bits)) as i64
        } else {
            val as i64
        }
    }
}

/// Point of Uncertainty (POU) frame for zero-allocation choice backtracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PouFrame {
    /// Saved bitstream reader checkpoint.
    bit_checkpoint: Checkpoint,
    /// Saved event sink watermark.
    sink_mark: usize,
    /// Target instruction address to branch to on failure.
    rollback_ip: u32,
}

/// High-performance linear bytecode virtual machine interpreter.
#[derive(Debug, Clone)]
pub struct VmInterpreter {
    /// 8 general-purpose 64-bit registers.
    pub regs: [u64; 8],
    /// Contiguous stack of active POU choice checkpoints.
    pou_stack: Vec<PouFrame>,
    /// Maximum instruction steps permitted before aborting (guards against infinite loops).
    pub max_steps: usize,
}

impl VmInterpreter {
    /// Default execution instruction step limit (10 million steps).
    pub const DEFAULT_MAX_STEPS: usize = 10_000_000;

    /// Constructs a new [`VmInterpreter`] with initialized registers and default step limits.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: [0; 8],
            pou_stack: Vec::with_capacity(16),
            max_steps: Self::DEFAULT_MAX_STEPS,
        }
    }

    /// Resets all register states and empties the POU stack for reuse.
    #[inline]
    pub fn reset(&mut self) {
        self.regs = [0; 8];
        self.pou_stack.clear();
    }

    /// Sets the value of a general-purpose register.
    #[inline]
    pub fn set_reg(&mut self, reg: Register, val: u64) {
        if let Some(slot) = self.regs.get_mut(reg.index()) {
            *slot = val;
        }
    }

    /// Gets the current value of a general-purpose register.
    #[inline]
    #[must_use]
    pub fn get_reg(&self, reg: Register) -> u64 {
        *self.regs.get(reg.index()).unwrap_or(&0)
    }

    /// Executes a [`BytecodeProgram`] against the provided [`BitReader`] and emits events into [`VmEventSink`].
    ///
    /// # Errors
    /// Returns [`DFDLError`] on fatal bitstream depletion, instruction failure without
    /// an active choice checkpoint, or if `max_steps` is exceeded.
    pub fn execute<S: ByteSource, K: VmEventSink>(
        &mut self,
        program: &BytecodeProgram,
        reader: &mut BitReader<S>,
        sink: &mut K,
    ) -> DFDLResult<()> {
        let mut budget = WorkBudget::new(self.max_steps);
        self.execute_with_budget(program, reader, sink, &mut budget)
    }

    /// Executes a [`BytecodeProgram`] against the provided [`BitReader`] and emits events into [`VmEventSink`]
    /// while consuming execution units from the provided [`WorkBudget`].
    ///
    /// # Errors
    /// Returns [`DFDLError`] on fatal bitstream depletion, instruction failure without
    /// an active choice checkpoint, or if `budget` is exhausted.
    pub fn execute_with_budget<S: ByteSource, K: VmEventSink>(
        &mut self,
        program: &BytecodeProgram,
        reader: &mut BitReader<S>,
        sink: &mut K,
        budget: &mut WorkBudget,
    ) -> DFDLResult<()> {
        let mut ip: usize = 0;

        macro_rules! try_read {
            ($read_expr:expr) => {
                match $read_expr {
                    Ok(val) => val,
                    Err(err) => {
                        if err.is_fatal() {
                            return Err(err);
                        }
                        if let Some(frame) = self.pou_stack.pop() {
                            reader.rollback(frame.bit_checkpoint)?;
                            sink.rollback(frame.sink_mark);
                            ip = frame.rollback_ip as usize;
                            continue;
                        } else {
                            return Err(err);
                        }
                    }
                }
            };
        }

        while ip < program.instructions.len() {
            budget.consume(1)?;

            let Some(inst) = program.instructions.get(ip) else {
                break;
            };

            match inst {
                Instruction::ReadBitsU8 { dest, count } => {
                    let val = try_read!(reader.read_bits(*count as usize));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBitsU16 { dest, count } => {
                    let val = try_read!(reader.read_bits(*count as usize));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBitsU32 { dest, count } => {
                    let val = try_read!(reader.read_bits(*count as usize));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBitsU64 { dest, count } => {
                    let val = try_read!(reader.read_bits(*count as usize));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBitsSigned { dest, count } => {
                    let raw = try_read!(reader.read_bits(*count as usize));
                    let ext = sign_extend(raw, *count as usize) as u64;
                    self.set_reg(*dest, ext);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBeU16 { dest } => {
                    let val = try_read!(reader.read_bits(16));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadLeU16 { dest } => {
                    let val = try_read!(reader.read_bits(16));
                    let swapped = (val as u16).swap_bytes() as u64;
                    self.set_reg(*dest, swapped);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBeU32 { dest } => {
                    let val = try_read!(reader.read_bits(32));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadLeU32 { dest } => {
                    let val = try_read!(reader.read_bits(32));
                    let swapped = (val as u32).swap_bytes() as u64;
                    self.set_reg(*dest, swapped);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBeU64 { dest } => {
                    let val = try_read!(reader.read_bits(64));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadLeU64 { dest } => {
                    let val = try_read!(reader.read_bits(64));
                    let swapped = val.swap_bytes();
                    self.set_reg(*dest, swapped);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBeI16 { dest } => {
                    let val = try_read!(reader.read_bits(16));
                    let ext = (val as i16 as i64) as u64;
                    self.set_reg(*dest, ext);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadLeI16 { dest } => {
                    let val = try_read!(reader.read_bits(16));
                    let swapped = (val as u16).swap_bytes() as i16;
                    let ext = (swapped as i64) as u64;
                    self.set_reg(*dest, ext);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBeI32 { dest } => {
                    let val = try_read!(reader.read_bits(32));
                    let ext = (val as i32 as i64) as u64;
                    self.set_reg(*dest, ext);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadLeI32 { dest } => {
                    let val = try_read!(reader.read_bits(32));
                    let swapped = (val as u32).swap_bytes() as i32;
                    let ext = (swapped as i64) as u64;
                    self.set_reg(*dest, ext);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBeI64 { dest } => {
                    let val = try_read!(reader.read_bits(64));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadLeI64 { dest } => {
                    let val = try_read!(reader.read_bits(64));
                    self.set_reg(*dest, val.swap_bytes());
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBeF32 { dest } => {
                    let val = try_read!(reader.read_bits(32));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadLeF32 { dest } => {
                    let val = try_read!(reader.read_bits(32));
                    let swapped = (val as u32).swap_bytes() as u64;
                    self.set_reg(*dest, swapped);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBeF64 { dest } => {
                    let val = try_read!(reader.read_bits(64));
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadLeF64 { dest } => {
                    let val = try_read!(reader.read_bits(64));
                    let swapped = val.swap_bytes();
                    self.set_reg(*dest, swapped);
                    ip = ip.saturating_add(1);
                }
                Instruction::LoadImm { dest, imm } => {
                    self.set_reg(*dest, *imm as u64);
                    ip = ip.saturating_add(1);
                }
                Instruction::AddImm { dest, imm } => {
                    let cur = self.get_reg(*dest);
                    self.set_reg(*dest, cur.wrapping_add(*imm as u64));
                    ip = ip.saturating_add(1);
                }
                Instruction::SubImm { dest, imm } => {
                    let cur = self.get_reg(*dest);
                    self.set_reg(*dest, cur.wrapping_sub(*imm as u64));
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBytes { dest_const: _, len } => {
                    for _ in 0..*len {
                        let _ = try_read!(reader.read_bits(8));
                    }
                    ip = ip.saturating_add(1);
                }
                Instruction::AlignBits { alignment_bits } => {
                    let align = *alignment_bits as usize;
                    if align > 1 {
                        let cur_pos = reader.position().0;
                        if let Some(rem) = cur_pos.checked_rem(align) {
                            if rem > 0 {
                                let skip = align.saturating_sub(rem);
                                try_read!(reader.skip_bits(skip));
                            }
                        }
                    }
                    ip = ip.saturating_add(1);
                }
                Instruction::SkipBits { bit_count } => {
                    try_read!(reader.skip_bits(*bit_count as usize));
                    ip = ip.saturating_add(1);
                }
                Instruction::SavePou { rollback_ip } => {
                    let frame = PouFrame {
                        bit_checkpoint: reader.checkpoint(),
                        sink_mark: sink.checkpoint(),
                        rollback_ip: *rollback_ip,
                    };
                    try_push(&mut self.pou_stack, frame)?;
                    ip = ip.saturating_add(1);
                }
                Instruction::CommitPou => {
                    let _ = self.pou_stack.pop();
                    ip = ip.saturating_add(1);
                }
                Instruction::RollbackPou => {
                    if let Some(frame) = self.pou_stack.pop() {
                        reader.rollback(frame.bit_checkpoint)?;
                        sink.rollback(frame.sink_mark);
                        ip = frame.rollback_ip as usize;
                        continue;
                    } else {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "RollbackPou executed with empty POU stack",
                        ));
                    }
                }
                Instruction::Jump { target_ip } => {
                    ip = *target_ip as usize;
                    continue;
                }
                Instruction::JumpIfEq { reg, imm, target_ip } => {
                    if self.get_reg(*reg) == (*imm as u64) {
                        ip = *target_ip as usize;
                        continue;
                    }
                    ip = ip.saturating_add(1);
                }
                Instruction::JumpIfNe { reg, imm, target_ip } => {
                    if self.get_reg(*reg) != (*imm as u64) {
                        ip = *target_ip as usize;
                        continue;
                    }
                    ip = ip.saturating_add(1);
                }
                Instruction::StartElement { symbol_id } => {
                    sink.on_event(VmEvent::StartElement(*symbol_id))?;
                    ip = ip.saturating_add(1);
                }
                Instruction::EndElement { symbol_id } => {
                    sink.on_event(VmEvent::EndElement(*symbol_id))?;
                    ip = ip.saturating_add(1);
                }
                Instruction::EmitScalarInt { symbol_id, reg, is_signed } => {
                    let raw = self.get_reg(*reg);
                    let val = raw as i64;
                    sink.on_event(VmEvent::ScalarInt {
                        id: *symbol_id,
                        val,
                        is_signed: *is_signed,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::EmitScalarFloat { symbol_id, reg, is_f64 } => {
                    let raw = self.get_reg(*reg);
                    let val = if *is_f64 {
                        f64::from_bits(raw)
                    } else {
                        f64::from(f32::from_bits(raw as u32))
                    };
                    sink.on_event(VmEvent::ScalarFloat {
                        id: *symbol_id,
                        val,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::EmitScalarBool { symbol_id, val } => {
                    sink.on_event(VmEvent::ScalarBool {
                        id: *symbol_id,
                        val: *val,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::EmitScalarString { symbol_id, const_idx } => {
                    let s = program.constants.get_string(*const_idx).ok_or_else(|| {
                        DFDLError::new(DFDLErrorKind::Parse, "Invalid constant string index")
                    })?;
                    sink.on_event(VmEvent::ScalarString {
                        id: *symbol_id,
                        val: s,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadString { symbol_id, len, trim_nul } => {
                    const STACK_STRING_CAP: usize = 64;
                    let mut stack_buf = [0u8; STACK_STRING_CAP];
                    let mut heap_buf;
                    let target_len = *len as usize;
                    let slice: &[u8] = if target_len <= STACK_STRING_CAP {
                        let target = stack_buf.get_mut(..target_len).ok_or_else(|| {
                            DFDLError::new(DFDLErrorKind::Parse, "Invalid stack string range")
                        })?;
                        try_read!(reader.read_bytes(target));
                        target
                    } else {
                        heap_buf = alloc::vec![0u8; target_len];
                        try_read!(reader.read_bytes(&mut heap_buf));
                        heap_buf.as_slice()
                    };
                    let non_nul_len = if *trim_nul {
                        slice
                            .iter()
                            .rposition(|&b| b != 0)
                            .map_or(0, |p| p.saturating_add(1))
                    } else {
                        slice.len()
                    };
                    let valid_slice = slice.get(..non_nul_len).unwrap_or(&[]);
                    let s = core::str::from_utf8(valid_slice).map_err(|_| {
                        DFDLError::new(DFDLErrorKind::Parse, "Invalid UTF-8 string bytes")
                    })?;
                    sink.on_event(VmEvent::ScalarString {
                        id: *symbol_id,
                        val: s,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBcdTime { symbol_id } => {
                    let mut raw = [0u8; 3];
                    try_read!(reader.read_bytes(&mut raw));
                    let b1 = *raw.first().unwrap_or(&0);
                    let b2 = *raw.get(1).unwrap_or(&0);
                    let b3 = *raw.get(2).unwrap_or(&0);
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    let time_buf = [
                        *HEX.get((b1 >> 4) as usize).unwrap_or(&b'0'),
                        *HEX.get((b1 & 0x0F) as usize).unwrap_or(&b'0'),
                        b':',
                        *HEX.get((b2 >> 4) as usize).unwrap_or(&b'0'),
                        *HEX.get((b2 & 0x0F) as usize).unwrap_or(&b'0'),
                        b':',
                        *HEX.get((b3 >> 4) as usize).unwrap_or(&b'0'),
                        *HEX.get((b3 & 0x0F) as usize).unwrap_or(&b'0'),
                    ];
                    let s = match core::str::from_utf8(&time_buf) {
                        Ok(valid) => valid,
                        Err(_) => {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                "Invalid BCD time string",
                            ))
                        }
                    };
                    sink.on_event(VmEvent::ScalarString {
                        id: *symbol_id,
                        val: s,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::CheckEof { expected_eof } => {
                    if reader.is_eof() != *expected_eof {
                        if let Some(frame) = self.pou_stack.pop() {
                            reader.rollback(frame.bit_checkpoint)?;
                            sink.rollback(frame.sink_mark);
                            ip = frame.rollback_ip as usize;
                            continue;
                        } else {
                            return Err(DFDLError::new(
                                DFDLErrorKind::Parse,
                                "End of data stream expectation mismatch",
                            ));
                        }
                    }
                    ip = ip.saturating_add(1);
                }
                Instruction::JumpIfEof { target_ip } => {
                    if reader.is_eof() {
                        ip = *target_ip as usize;
                        continue;
                    }
                    ip = ip.saturating_add(1);
                }
                Instruction::MoveReg { dest, src } => {
                    let val = self.get_reg(*src);
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::TestBit { dest, src, bit } => {
                    // Extract single bit at 0-indexed position (bit 0 = LSB) safely within 64-bit bounds
                    let val = self.get_reg(*src);
                    let bit_val = if *bit < 64 {
                        (val >> (*bit as u64)) & 1
                    } else {
                        0
                    };
                    self.set_reg(*dest, bit_val);
                    ip = ip.saturating_add(1);
                }
                Instruction::PeekBitsU8 { dest, count } => {
                    let val = try_read!(reader.lookahead_bits(0, *count as usize)) as u64;
                    self.set_reg(*dest, val);
                    ip = ip.saturating_add(1);
                }
                Instruction::RecordPosition { dest } => {
                    self.set_reg(*dest, reader.position().0 as u64);
                    ip = ip.saturating_add(1);
                }
                Instruction::PadToLength {
                    start_reg,
                    total_bytes,
                } => {
                    let start_bit = self.get_reg(*start_reg);
                    let current_bit = reader.position().0 as u64;
                    let target_bit =
                        start_bit.saturating_add((*total_bytes as u64).saturating_mul(8));
                    if current_bit < target_bit {
                        let skip_bits = target_bit.saturating_sub(current_bit) as usize;
                        try_read!(reader.skip_bits(skip_bits));
                    }
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadEmitBeU16 { symbol_id, is_signed } => {
                    let raw = try_read!(reader.read_bits(16));
                    let val = if *is_signed {
                        raw as i16 as i64
                    } else {
                        raw as i64
                    };
                    self.set_reg(Register::R0, val as u64);
                    sink.on_event(VmEvent::ScalarInt {
                        id: *symbol_id,
                        val,
                        is_signed: *is_signed,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadEmitLeU16 { symbol_id, is_signed } => {
                    let raw = try_read!(reader.read_bits(16));
                    let swapped = (raw as u16).swap_bytes();
                    let val = if *is_signed {
                        swapped as i16 as i64
                    } else {
                        swapped as i64
                    };
                    self.set_reg(Register::R0, val as u64);
                    sink.on_event(VmEvent::ScalarInt {
                        id: *symbol_id,
                        val,
                        is_signed: *is_signed,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadEmitBeU32 { symbol_id, is_signed } => {
                    let raw = try_read!(reader.read_bits(32));
                    let val = if *is_signed {
                        raw as i32 as i64
                    } else {
                        raw as i64
                    };
                    self.set_reg(Register::R0, val as u64);
                    sink.on_event(VmEvent::ScalarInt {
                        id: *symbol_id,
                        val,
                        is_signed: *is_signed,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadEmitLeU32 { symbol_id, is_signed } => {
                    let raw = try_read!(reader.read_bits(32));
                    let swapped = (raw as u32).swap_bytes();
                    let val = if *is_signed {
                        swapped as i32 as i64
                    } else {
                        swapped as i64
                    };
                    self.set_reg(Register::R0, val as u64);
                    sink.on_event(VmEvent::ScalarInt {
                        id: *symbol_id,
                        val,
                        is_signed: *is_signed,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadEmitBitsU8 { symbol_id, count, is_signed } => {
                    let raw = try_read!(reader.read_bits(*count as usize));
                    let val = if *is_signed {
                        sign_extend(raw, *count as usize)
                    } else {
                        raw as i64
                    };
                    self.set_reg(Register::R0, val as u64);
                    sink.on_event(VmEvent::ScalarInt {
                        id: *symbol_id,
                        val,
                        is_signed: *is_signed,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadEmitBool { symbol_id, count, true_val } => {
                    let raw = try_read!(reader.read_bits(*count as usize)) as u8;
                    let val = raw == *true_val;
                    self.set_reg(Register::R0, if val { 1 } else { 0 });
                    sink.on_event(VmEvent::ScalarBool {
                        id: *symbol_id,
                        val,
                    })?;
                    ip = ip.saturating_add(1);
                }
                Instruction::ReadBlock { start_idx, field_count } => {
                    let start = *start_idx as usize;
                    let count = *field_count as usize;
                    let end = start.saturating_add(count);
                    let fields = program.block_fields.get(start..end).ok_or_else(|| {
                        DFDLError::new(DFDLErrorKind::Parse, "Invalid block fields slice range")
                    })?;

                    let mut rolled_back = false;
                    'block_loop: for field in fields {
                        macro_rules! try_read_field {
                            ($read_expr:expr) => {
                                match $read_expr {
                                    Ok(val) => val,
                                    Err(err) => {
                                        if err.is_fatal() {
                                            return Err(err);
                                        }
                                        if let Some(frame) = self.pou_stack.pop() {
                                            reader.rollback(frame.bit_checkpoint)?;
                                            sink.rollback(frame.sink_mark);
                                            ip = frame.rollback_ip as usize;
                                            rolled_back = true;
                                            break 'block_loop;
                                        } else {
                                            return Err(err);
                                        }
                                    }
                                }
                            };
                        }

                        match field.kind {
                            BlockFieldKind::BeU16 { is_signed } => {
                                let raw = try_read_field!(reader.read_bits(16));
                                let val = if is_signed {
                                    raw as i16 as i64
                                } else {
                                    raw as i64
                                };
                                self.set_reg(Register::R0, val as u64);
                                sink.on_event(VmEvent::ScalarInt {
                                    id: field.symbol_id,
                                    val,
                                    is_signed,
                                })?;
                            }
                            BlockFieldKind::LeU16 { is_signed } => {
                                let raw = try_read_field!(reader.read_bits(16));
                                let swapped = (raw as u16).swap_bytes();
                                let val = if is_signed {
                                    swapped as i16 as i64
                                } else {
                                    swapped as i64
                                };
                                self.set_reg(Register::R0, val as u64);
                                sink.on_event(VmEvent::ScalarInt {
                                    id: field.symbol_id,
                                    val,
                                    is_signed,
                                })?;
                            }
                            BlockFieldKind::BeU32 { is_signed } => {
                                let raw = try_read_field!(reader.read_bits(32));
                                let val = if is_signed {
                                    raw as i32 as i64
                                } else {
                                    raw as i64
                                };
                                self.set_reg(Register::R0, val as u64);
                                sink.on_event(VmEvent::ScalarInt {
                                    id: field.symbol_id,
                                    val,
                                    is_signed,
                                })?;
                            }
                            BlockFieldKind::LeU32 { is_signed } => {
                                let raw = try_read_field!(reader.read_bits(32));
                                let swapped = (raw as u32).swap_bytes();
                                let val = if is_signed {
                                    swapped as i32 as i64
                                } else {
                                    swapped as i64
                                };
                                self.set_reg(Register::R0, val as u64);
                                sink.on_event(VmEvent::ScalarInt {
                                    id: field.symbol_id,
                                    val,
                                    is_signed,
                                })?;
                            }
                            BlockFieldKind::BeU64 { is_signed } => {
                                let raw = try_read_field!(reader.read_bits(64));
                                let val = raw as i64;
                                self.set_reg(Register::R0, raw);
                                sink.on_event(VmEvent::ScalarInt {
                                    id: field.symbol_id,
                                    val,
                                    is_signed,
                                })?;
                            }
                            BlockFieldKind::LeU64 { is_signed } => {
                                let raw = try_read_field!(reader.read_bits(64));
                                let swapped = raw.swap_bytes();
                                let val = swapped as i64;
                                self.set_reg(Register::R0, swapped);
                                sink.on_event(VmEvent::ScalarInt {
                                    id: field.symbol_id,
                                    val,
                                    is_signed,
                                })?;
                            }
                            BlockFieldKind::BitsU8 { count, is_signed } => {
                                let raw = try_read_field!(reader.read_bits(count as usize));
                                let val = if is_signed {
                                    sign_extend(raw, count as usize)
                                } else {
                                    raw as i64
                                };
                                self.set_reg(Register::R0, val as u64);
                                sink.on_event(VmEvent::ScalarInt {
                                    id: field.symbol_id,
                                    val,
                                    is_signed,
                                })?;
                            }
                            BlockFieldKind::Bool { count, true_val } => {
                                let raw = try_read_field!(reader.read_bits(count as usize)) as u8;
                                let val = raw == true_val;
                                self.set_reg(Register::R0, if val { 1 } else { 0 });
                                sink.on_event(VmEvent::ScalarBool {
                                    id: field.symbol_id,
                                    val,
                                })?;
                            }
                            BlockFieldKind::FixedString { len, trim_nul } => {
                                const STACK_STRING_CAP: usize = 64;
                                let mut stack_buf = [0u8; STACK_STRING_CAP];
                                let target_len = len as usize;
                                let target = stack_buf.get_mut(..target_len).ok_or_else(|| {
                                    DFDLError::new(DFDLErrorKind::Parse, "Invalid stack string range")
                                })?;
                                try_read_field!(reader.read_bytes(target));
                                let non_nul_len = if trim_nul {
                                    target
                                        .iter()
                                        .rposition(|&b| b != 0)
                                        .map_or(0, |p| p.saturating_add(1))
                                } else {
                                    target.len()
                                };
                                let valid_slice = target.get(..non_nul_len).unwrap_or(&[]);
                                let s = core::str::from_utf8(valid_slice).map_err(|_| {
                                    DFDLError::new(DFDLErrorKind::Parse, "Invalid UTF-8 string bytes")
                                })?;
                                sink.on_event(VmEvent::ScalarString {
                                    id: field.symbol_id,
                                    val: s,
                                })?;
                            }
                            BlockFieldKind::BcdTime => {
                                let mut raw = [0u8; 3];
                                try_read_field!(reader.read_bytes(&mut raw));
                                let b1 = *raw.first().unwrap_or(&0);
                                let b2 = *raw.get(1).unwrap_or(&0);
                                let b3 = *raw.get(2).unwrap_or(&0);
                                const HEX: &[u8; 16] = b"0123456789abcdef";
                                let time_buf = [
                                    *HEX.get((b1 >> 4) as usize).unwrap_or(&b'0'),
                                    *HEX.get((b1 & 0x0F) as usize).unwrap_or(&b'0'),
                                    b':',
                                    *HEX.get((b2 >> 4) as usize).unwrap_or(&b'0'),
                                    *HEX.get((b2 & 0x0F) as usize).unwrap_or(&b'0'),
                                    b':',
                                    *HEX.get((b3 >> 4) as usize).unwrap_or(&b'0'),
                                    *HEX.get((b3 & 0x0F) as usize).unwrap_or(&b'0'),
                                ];
                                let s = match core::str::from_utf8(&time_buf) {
                                    Ok(valid) => valid,
                                    Err(_) => {
                                        return Err(DFDLError::new(
                                            DFDLErrorKind::Parse,
                                            "Invalid BCD time string",
                                        ))
                                    }
                                };
                                sink.on_event(VmEvent::ScalarString {
                                    id: field.symbol_id,
                                    val: s,
                                })?;
                            }
                        }
                    }
                    if rolled_back {
                        continue;
                    }
                    ip = ip.saturating_add(1);
                }
                Instruction::HaltSuccess => {
                    return Ok(());
                }
                Instruction::HaltError { error_code: _ } => {
                    if let Some(frame) = self.pou_stack.pop() {
                        reader.rollback(frame.bit_checkpoint)?;
                        sink.rollback(frame.sink_mark);
                        ip = frame.rollback_ip as usize;
                        continue;
                    } else {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Bytecode execution halted with error code",
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for VmInterpreter {
    fn default() -> Self {
        Self::new()
    }
}
