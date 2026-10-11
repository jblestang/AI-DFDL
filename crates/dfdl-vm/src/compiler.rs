//! DFDL Bytecode Compiler.
//!
//! Lowers high-level [`CompiledSchema`] intermediate representations into linear
//! [`BytecodeProgram`] instructions suitable for direct execution by [`crate::interpreter::VmInterpreter`].

extern crate alloc;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::infoset::value::DfdlSimpleType;
use dfdl_core::io::traits::ByteOrder;
use dfdl_core::schema::ir::{
    AlignmentUnits, BinaryCalendarRep, CompiledElement, CompiledSchema, CompiledSequence,
    CompiledType, LengthKind, LengthUnits, NodeId, OccursCountKind, Representation,
    ResolvedProperties, TermKind, TextTrimKind,
};
use dfdl_core::util::try_push;

use crate::isa::{
    BlockField, BlockFieldKind, BytecodeProgram, Instruction, Register, SymbolId,
};

/// Compiler lowering [`CompiledSchema`] graph models to linear bytecode programs.
#[derive(Debug)]
pub struct BytecodeCompiler<'a> {
    schema: &'a CompiledSchema,
    program: BytecodeProgram,
    length_depth: usize,
}

impl<'a> BytecodeCompiler<'a> {
    /// Compiles a schema graph into a linear [`BytecodeProgram`].
    ///
    /// # Errors
    /// Returns [`DFDLError`] if unresolved references or unsupported construct combinations exist.
    pub fn compile(schema: &'a CompiledSchema) -> DFDLResult<BytecodeProgram> {
        let mut compiler = Self {
            schema,
            program: BytecodeProgram::new(),
            length_depth: 0,
        };

        compiler.lower_term(schema.root_element_id)?;
        compiler.program.emit(Instruction::CheckEof {
            expected_eof: true,
        });
        compiler.program.emit(Instruction::HaltSuccess);

        Ok(compiler.program)
    }

    /// Lowers an individual schema term by its [`NodeId`].
    fn lower_term(&mut self, term_id: NodeId) -> DFDLResult<()> {
        let term = self.schema.get_term(term_id).ok_or_else(|| {
            DFDLError::new(
                DFDLErrorKind::SchemaDefinition,
                "NodeId not found in compiled schema graph",
            )
        })?;

        self.lower_framing(&term.properties);

        match &term.kind {
            TermKind::Element(elem) => {
                self.lower_element_term(elem, &term.properties)?;
            }
            TermKind::Sequence(seq) => {
                self.lower_sequence(seq)?;
            }
            TermKind::Choice(choice) => {
                self.lower_choice(choice)?;
            }
            TermKind::GroupRef(target_id) => {
                self.lower_term(*target_id)?;
            }
        }

        self.lower_trailing_framing(&term.properties);

        Ok(())
    }

    /// Lowers a sequence with contiguous struct / block fusion.
    fn lower_sequence(&mut self, seq: &CompiledSequence) -> DFDLResult<()> {
        let mut flattened = Vec::new();
        for &member_id in &seq.members {
            self.flatten_sequence_terms(member_id, &mut flattened);
        }

        let mut i = 0;
        while i < flattened.len() {
            let mut block_run = Vec::new();
            let mut j = i;
            while j < flattened.len() {
                if let Some(term_id) = flattened.get(j).copied() {
                    if let Some(bf) = self.try_as_block_field(term_id) {
                        let _ = try_push(&mut block_run, bf);
                        j = j.saturating_add(1);
                        continue;
                    }
                }
                break;
            }

            if block_run.len() >= 2 {
                let start_idx = self.program.block_fields.len() as u32;
                let field_count = block_run.len() as u16;
                self.program.block_fields.extend(block_run);
                self.program.emit(Instruction::ReadBlock {
                    start_idx,
                    field_count,
                });
                i = j;
            } else {
                if let Some(term_id) = flattened.get(i).copied() {
                    self.lower_term(term_id)?;
                }
                i = i.saturating_add(1);
            }
        }

        Ok(())
    }

    /// Recursively flattens nested group references and sequences without framing into a linear term list.
    fn flatten_sequence_terms(&self, term_id: NodeId, out: &mut Vec<NodeId>) {
        if let Some(term) = self.schema.get_term(term_id) {
            let has_framing = term.properties.alignment > 1
                || term.properties.leading_skip > 0
                || term.properties.trailing_skip > 0
                || term.properties.initiator.as_ref().is_some_and(|s| !s.is_empty())
                || term.properties.terminator.as_ref().is_some_and(|s| !s.is_empty());

            if !has_framing {
                match &term.kind {
                    TermKind::Sequence(seq) => {
                        for &member_id in &seq.members {
                            self.flatten_sequence_terms(member_id, out);
                        }
                        return;
                    }
                    TermKind::GroupRef(target_id) => {
                        self.flatten_sequence_terms(*target_id, out);
                        return;
                    }
                    _ => {}
                }
            }
        }
        let _ = try_push(out, term_id);
    }

    /// Checks if a schema term represents a contiguous scalar field that can be fused into a block.
    fn try_as_block_field(&mut self, term_id: NodeId) -> Option<BlockField> {
        let term = self.schema.get_term(term_id)?;
        let TermKind::Element(elem) = &term.kind else {
            return None;
        };

        if elem.min_occurs != 1 || elem.max_occurs != Some(1) {
            return None;
        }
        if term.properties.occurs_count_kind == OccursCountKind::Expression {
            return None;
        }
        if term.properties.alignment > 1
            || term.properties.leading_skip > 0
            || term.properties.trailing_skip > 0
        {
            return None;
        }
        if let Some(ref init) = term.properties.initiator {
            if !init.is_empty() {
                return None;
            }
        }
        if let Some(ref term_del) = term.properties.terminator {
            if !term_del.is_empty() {
                return None;
            }
        }

        let CompiledType::Simple(simple_type) = elem.type_ir else {
            return None;
        };

        let props = &term.properties;
        if !matches!(simple_type, DfdlSimpleType::String)
            && props.representation != Representation::Binary
        {
            return None;
        }

        let symbol_id = self.program.symbols.intern(elem.name.clone());
        let is_be = props.byte_order == ByteOrder::BigEndian;

        if let Some(bits) = props.length {
            if props.length_units == LengthUnits::Bits && bits <= 8 {
                let is_signed = matches!(
                    simple_type,
                    DfdlSimpleType::Byte
                        | DfdlSimpleType::Short
                        | DfdlSimpleType::Int
                        | DfdlSimpleType::Long
                );
                return Some(BlockField {
                    symbol_id,
                    kind: BlockFieldKind::BitsU8 {
                        count: bits as u8,
                        is_signed,
                    },
                });
            }
        }

        match simple_type {
            DfdlSimpleType::Byte => Some(BlockField {
                symbol_id,
                kind: BlockFieldKind::BitsU8 {
                    count: 8,
                    is_signed: true,
                },
            }),
            DfdlSimpleType::UnsignedByte => Some(BlockField {
                symbol_id,
                kind: BlockFieldKind::BitsU8 {
                    count: 8,
                    is_signed: false,
                },
            }),
            DfdlSimpleType::Short => Some(BlockField {
                symbol_id,
                kind: if is_be {
                    BlockFieldKind::BeU16 { is_signed: true }
                } else {
                    BlockFieldKind::LeU16 { is_signed: true }
                },
            }),
            DfdlSimpleType::UnsignedShort => Some(BlockField {
                symbol_id,
                kind: if is_be {
                    BlockFieldKind::BeU16 { is_signed: false }
                } else {
                    BlockFieldKind::LeU16 { is_signed: false }
                },
            }),
            DfdlSimpleType::Int => Some(BlockField {
                symbol_id,
                kind: if is_be {
                    BlockFieldKind::BeU32 { is_signed: true }
                } else {
                    BlockFieldKind::LeU32 { is_signed: true }
                },
            }),
            DfdlSimpleType::UnsignedInt => Some(BlockField {
                symbol_id,
                kind: if is_be {
                    BlockFieldKind::BeU32 { is_signed: false }
                } else {
                    BlockFieldKind::LeU32 { is_signed: false }
                },
            }),
            DfdlSimpleType::Long => Some(BlockField {
                symbol_id,
                kind: if is_be {
                    BlockFieldKind::BeU64 { is_signed: true }
                } else {
                    BlockFieldKind::LeU64 { is_signed: true }
                },
            }),
            DfdlSimpleType::UnsignedLong => Some(BlockField {
                symbol_id,
                kind: if is_be {
                    BlockFieldKind::BeU64 { is_signed: false }
                } else {
                    BlockFieldKind::LeU64 { is_signed: false }
                },
            }),
            DfdlSimpleType::Boolean => {
                let bit_count = match props.length_units {
                    LengthUnits::Bits => props.length.unwrap_or(8) as u8,
                    _ => (props.length.unwrap_or(1).saturating_mul(8)) as u8,
                };
                if bit_count <= 8 {
                    Some(BlockField {
                        symbol_id,
                        kind: BlockFieldKind::Bool {
                            count: bit_count,
                            true_val: 1,
                        },
                    })
                } else {
                    None
                }
            }
            DfdlSimpleType::String => {
                let str_len = props
                    .length
                    .or(props.facets.max_length)
                    .or(props.facets.min_length)
                    .unwrap_or(0);
                if str_len > 0 && str_len <= 64 {
                    let trim_nul = props.text_trim_kind != TextTrimKind::None
                        && (props.text_pad_char.is_empty()
                            || props.text_pad_char == "\0"
                            || props.text_pad_char == "%#r00;");
                    Some(BlockField {
                        symbol_id,
                        kind: BlockFieldKind::FixedString {
                            len: str_len as u8,
                            trim_nul,
                        },
                    })
                } else {
                    None
                }
            }
            DfdlSimpleType::Time | DfdlSimpleType::DateTime | DfdlSimpleType::Date => {
                if props.binary_calendar_rep == BinaryCalendarRep::Bcd {
                    Some(BlockField {
                        symbol_id,
                        kind: BlockFieldKind::BcdTime,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Emits alignment and leading skip instructions for a term.
    fn lower_framing(&mut self, props: &ResolvedProperties) {
        if props.alignment > 1 {
            let align_bits = match props.alignment_units {
                AlignmentUnits::Bytes => props.alignment.saturating_mul(8),
                AlignmentUnits::Bits => props.alignment,
            };
            if align_bits > 1 {
                self.program.emit(Instruction::AlignBits {
                    alignment_bits: align_bits as u16,
                });
            }
        }

        if props.leading_skip > 0 {
            let skip_bits = match props.alignment_units {
                AlignmentUnits::Bytes => props.leading_skip.saturating_mul(8),
                AlignmentUnits::Bits => props.leading_skip,
            };
            if skip_bits > 0 {
                self.program.emit(Instruction::SkipBits {
                    bit_count: skip_bits as u32,
                });
            }
        }
    }

    /// Emits trailing skip padding instructions for a term.
    fn lower_trailing_framing(&mut self, props: &ResolvedProperties) {
        if props.trailing_skip > 0 {
            let skip_bits = match props.alignment_units {
                AlignmentUnits::Bytes => props.trailing_skip.saturating_mul(8),
                AlignmentUnits::Bits => props.trailing_skip,
            };
            if skip_bits > 0 {
                self.program.emit(Instruction::SkipBits {
                    bit_count: skip_bits as u32,
                });
            }
        }
    }

    /// Lowers initiator delimiter verification if present.
    fn lower_initiator(&mut self, props: &ResolvedProperties) {
        if let Some(ref init) = props.initiator {
            if !init.is_empty() {
                for &b in init.as_bytes() {
                    self.program.emit(Instruction::ReadBitsU8 {
                        dest: Register::R0,
                        count: 8,
                    });
                    let ok_jump = self.program.emit(Instruction::JumpIfEq {
                        reg: Register::R0,
                        imm: b as u32,
                        target_ip: 0,
                    });
                    self.program.emit(Instruction::HaltError { error_code: 1 });
                    let ok_target = self.program.instructions.len() as u32;
                    self.program.patch_target(ok_jump, ok_target);
                }
            }
        }
    }

    /// Lowers an element term and handles fixed or dynamic repetition.
    fn lower_element_term(
        &mut self,
        elem: &CompiledElement,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let symbol_id = self.program.symbols.intern(elem.name.clone());

        match props.occurs_count_kind {
            OccursCountKind::Expression => {
                // Loop counter evaluated dynamically from preceding sibling (in R0)
                self.program.emit(Instruction::MoveReg {
                    dest: Register::R2,
                    src: Register::R0,
                });
                let loop_top = self.program.instructions.len() as u32;
                let exit_jump = self.program.emit(Instruction::JumpIfEq {
                    reg: Register::R2,
                    imm: 0,
                    target_ip: 0,
                });
                self.lower_single_element(symbol_id, elem, props)?;
                self.program.emit(Instruction::SubImm {
                    dest: Register::R2,
                    imm: 1,
                });
                self.program.emit(Instruction::Jump {
                    target_ip: loop_top,
                });
                let loop_exit = self.program.instructions.len() as u32;
                self.program.patch_target(exit_jump, loop_exit);
            }
            OccursCountKind::Implicit => {
                // Mandatory occurrences first
                for _ in 0..elem.min_occurs {
                    self.lower_single_element(symbol_id, elem, props)?;
                }

                // Variable occurrences up to EOF or max_occurs
                let max_reps = elem.max_occurs.unwrap_or(usize::MAX);
                if max_reps > elem.min_occurs {
                    let loop_top = self.program.instructions.len() as u32;
                    let exit_jump = self.program.emit(Instruction::JumpIfEof { target_ip: 0 });
                    let save_addr = self.program.emit(Instruction::SavePou { rollback_ip: 0 });
                    self.lower_single_element(symbol_id, elem, props)?;
                    self.program.emit(Instruction::CommitPou);
                    self.program.emit(Instruction::Jump {
                        target_ip: loop_top,
                    });
                    let loop_exit = self.program.instructions.len() as u32;
                    self.program.patch_target(exit_jump, loop_exit);
                    self.program.patch_target(save_addr, loop_exit);
                }
            }
            _ => {
                let count =
                    if elem.min_occurs == elem.max_occurs.unwrap_or(0) && elem.min_occurs > 0 {
                        elem.min_occurs
                    } else {
                        1
                    };

                for _ in 0..count {
                    self.lower_single_element(symbol_id, elem, props)?;
                }
            }
        }

        Ok(())
    }

    /// Lowers a single occurrence of an element.
    fn lower_single_element(
        &mut self,
        symbol_id: SymbolId,
        elem: &CompiledElement,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let is_explicit_complex = props.length_kind == LengthKind::Explicit
            && props.length.is_some()
            && matches!(elem.type_ir, CompiledType::Complex(_));

        let reg = if self.length_depth == 0 {
            Register::R7
        } else {
            Register::R6
        };

        if is_explicit_complex {
            self.program.emit(Instruction::RecordPosition { dest: reg });
            self.length_depth = self.length_depth.saturating_add(1);
        }

        self.program.emit(Instruction::StartElement { symbol_id });

        self.lower_initiator(props);

        match &elem.type_ir {
            CompiledType::Simple(simple_type) => {
                self.lower_simple_scalar(symbol_id, *simple_type, props)?;
            }
            CompiledType::Complex(child_id) => {
                self.lower_term(*child_id)?;
            }
        }

        self.lower_trailing_framing(props);

        if is_explicit_complex {
            self.length_depth = self.length_depth.saturating_sub(1);
            if let Some(total_bytes) = props.length {
                self.program.emit(Instruction::PadToLength {
                    start_reg: reg,
                    total_bytes: total_bytes as u32,
                });
            }
        }

        self.program.emit(Instruction::EndElement { symbol_id });
        Ok(())
    }

    /// Lowers simple scalar fields based on physical representation and type.
    fn lower_simple_scalar(
        &mut self,
        symbol_id: SymbolId,
        simple_type: DfdlSimpleType,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        if matches!(simple_type, DfdlSimpleType::String) {
            let str_len = props
                .length
                .or(props.facets.max_length)
                .or(props.facets.min_length)
                .unwrap_or(0);
            let trim_nul = props.text_trim_kind != TextTrimKind::None
                && (props.text_pad_char.is_empty()
                    || props.text_pad_char == "\0"
                    || props.text_pad_char == "%#r00;");
            self.program.emit(Instruction::ReadString {
                symbol_id,
                len: str_len as u32,
                trim_nul,
            });
            return Ok(());
        }

        match props.representation {
            Representation::Binary => {
                self.lower_binary_scalar(symbol_id, simple_type, props)?;
            }
            Representation::Text => {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Text representation lowering in dfdl-vm is not yet implemented",
                ));
            }
        }
        Ok(())
    }

    /// Emits optimal instructions for binary primitive reading.
    fn lower_binary_scalar(
        &mut self,
        symbol_id: SymbolId,
        simple_type: DfdlSimpleType,
        props: &ResolvedProperties,
    ) -> DFDLResult<()> {
        let is_be = props.byte_order == ByteOrder::BigEndian;

        if let Some(bits) = props.length {
            if props.length_units == LengthUnits::Bits && bits < 64 {
                let is_signed = matches!(
                    simple_type,
                    DfdlSimpleType::Byte
                        | DfdlSimpleType::Short
                        | DfdlSimpleType::Int
                        | DfdlSimpleType::Long
                );
                if is_signed {
                    self.program.emit(Instruction::ReadBitsSigned {
                        dest: Register::R0,
                        count: bits as u8,
                    });
                } else {
                    self.program.emit(Instruction::ReadBitsU64 {
                        dest: Register::R0,
                        count: bits as u8,
                    });
                }
                self.program.emit(Instruction::EmitScalarInt {
                    symbol_id,
                    reg: Register::R0,
                    is_signed,
                });
                return Ok(());
            }
        }

        match simple_type {
            DfdlSimpleType::Byte => {
                self.program.emit(Instruction::ReadEmitBitsU8 {
                    symbol_id,
                    count: 8,
                    is_signed: true,
                });
            }
            DfdlSimpleType::UnsignedByte => {
                self.program.emit(Instruction::ReadEmitBitsU8 {
                    symbol_id,
                    count: 8,
                    is_signed: false,
                });
            }
            DfdlSimpleType::Short => {
                if is_be {
                    self.program.emit(Instruction::ReadEmitBeU16 {
                        symbol_id,
                        is_signed: true,
                    });
                } else {
                    self.program.emit(Instruction::ReadEmitLeU16 {
                        symbol_id,
                        is_signed: true,
                    });
                }
            }
            DfdlSimpleType::UnsignedShort => {
                if is_be {
                    self.program.emit(Instruction::ReadEmitBeU16 {
                        symbol_id,
                        is_signed: false,
                    });
                } else {
                    self.program.emit(Instruction::ReadEmitLeU16 {
                        symbol_id,
                        is_signed: false,
                    });
                }
            }
            DfdlSimpleType::Int => {
                if is_be {
                    self.program.emit(Instruction::ReadEmitBeU32 {
                        symbol_id,
                        is_signed: true,
                    });
                } else {
                    self.program.emit(Instruction::ReadEmitLeU32 {
                        symbol_id,
                        is_signed: true,
                    });
                }
            }
            DfdlSimpleType::UnsignedInt => {
                if is_be {
                    self.program.emit(Instruction::ReadEmitBeU32 {
                        symbol_id,
                        is_signed: false,
                    });
                } else {
                    self.program.emit(Instruction::ReadEmitLeU32 {
                        symbol_id,
                        is_signed: false,
                    });
                }
            }
            DfdlSimpleType::Long => {
                if is_be {
                    self.program.emit(Instruction::ReadBeI64 {
                        dest: Register::R0,
                    });
                } else {
                    self.program.emit(Instruction::ReadLeI64 {
                        dest: Register::R0,
                    });
                }
                self.program.emit(Instruction::EmitScalarInt {
                    symbol_id,
                    reg: Register::R0,
                    is_signed: true,
                });
            }
            DfdlSimpleType::UnsignedLong => {
                if is_be {
                    self.program.emit(Instruction::ReadBeU64 {
                        dest: Register::R0,
                    });
                } else {
                    self.program.emit(Instruction::ReadLeU64 {
                        dest: Register::R0,
                    });
                }
                self.program.emit(Instruction::EmitScalarInt {
                    symbol_id,
                    reg: Register::R0,
                    is_signed: false,
                });
            }
            DfdlSimpleType::Float => {
                if is_be {
                    self.program.emit(Instruction::ReadBeF32 {
                        dest: Register::R0,
                    });
                } else {
                    self.program.emit(Instruction::ReadLeF32 {
                        dest: Register::R0,
                    });
                }
                self.program.emit(Instruction::EmitScalarFloat {
                    symbol_id,
                    reg: Register::R0,
                    is_f64: false,
                });
            }
            DfdlSimpleType::Double => {
                if is_be {
                    self.program.emit(Instruction::ReadBeF64 {
                        dest: Register::R0,
                    });
                } else {
                    self.program.emit(Instruction::ReadLeF64 {
                        dest: Register::R0,
                    });
                }
                self.program.emit(Instruction::EmitScalarFloat {
                    symbol_id,
                    reg: Register::R0,
                    is_f64: true,
                });
            }
            DfdlSimpleType::Boolean => {
                let bit_count = match props.length_units {
                    LengthUnits::Bits => props.length.unwrap_or(8) as u8,
                    _ => (props.length.unwrap_or(1).saturating_mul(8)) as u8,
                };
                if bit_count <= 8 {
                    self.program.emit(Instruction::ReadEmitBool {
                        symbol_id,
                        count: bit_count,
                        true_val: 1,
                    });
                } else {
                    self.program.emit(Instruction::ReadBitsU8 {
                        dest: Register::R0,
                        count: bit_count,
                    });
                    let cond_addr = self.program.emit(Instruction::JumpIfNe {
                        reg: Register::R0,
                        imm: 0,
                        target_ip: 0,
                    });
                    self.program.emit(Instruction::EmitScalarBool {
                        symbol_id,
                        val: false,
                    });
                    let end_jump = self.program.emit(Instruction::Jump { target_ip: 0 });

                    let true_target = self.program.instructions.len() as u32;
                    self.program.patch_target(cond_addr, true_target);
                    self.program.emit(Instruction::EmitScalarBool {
                        symbol_id,
                        val: true,
                    });

                    let end_target = self.program.instructions.len() as u32;
                    self.program.patch_target(end_jump, end_target);
                }
            }
            DfdlSimpleType::Time | DfdlSimpleType::DateTime | DfdlSimpleType::Date => {
                if props.binary_calendar_rep == BinaryCalendarRep::Bcd {
                    self.program.emit(Instruction::ReadBcdTime { symbol_id });
                } else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "Unsupported calendar representation in dfdl-vm",
                    ));
                }
            }
            _ => {
                return Err(DFDLError::new(
                    DFDLErrorKind::SchemaDefinition,
                    "Unsupported binary simple type in dfdl-vm",
                ));
            }
        }

        Ok(())
    }

    /// Emits zero-allocation choice branching bytecode with automatic POU management or direct dispatch.
    fn lower_choice(
        &mut self,
        choice: &dfdl_core::schema::ir::CompiledChoice,
    ) -> DFDLResult<()> {
        if choice.branches.is_empty() {
            return Ok(());
        }

        // Check if all branches have a distinct 1-byte branch key or initiator
        let mut branch_keys = Vec::new();
        let mut all_have_keys = true;

        for &branch_id in &choice.branches {
            if let Some(key) = self.get_branch_key(branch_id) {
                if branch_keys.iter().any(|(_, k)| *k == key) {
                    all_have_keys = false;
                    break;
                }
                let _ = try_push(&mut branch_keys, (branch_id, key));
            } else {
                all_have_keys = false;
                break;
            }
        }

        if all_have_keys && !branch_keys.is_empty() {
            // Fast path: deterministic choice dispatch via 1-byte peek
            self.program.emit(Instruction::PeekBitsU8 {
                dest: Register::R0,
                count: 8,
            });

            let mut jump_table = Vec::new();
            for &(branch_id, key) in &branch_keys {
                let j = self.program.emit(Instruction::JumpIfEq {
                    reg: Register::R0,
                    imm: key,
                    target_ip: 0,
                });
                let _ = try_push(&mut jump_table, (j, branch_id));
            }

            self.program.emit(Instruction::HaltError { error_code: 1 });

            let mut exit_jumps = Vec::new();
            for (jump_addr, branch_id) in jump_table {
                let branch_ip = self.program.instructions.len() as u32;
                self.program.patch_target(jump_addr, branch_ip);

                self.lower_term(branch_id)?;

                let exit_j = self.program.emit(Instruction::Jump { target_ip: 0 });
                let _ = try_push(&mut exit_jumps, exit_j);
            }

            let choice_end_ip = self.program.instructions.len() as u32;
            for exit_j in exit_jumps {
                self.program.patch_target(exit_j, choice_end_ip);
            }
        } else {
            // General speculative choice with POU backtracking
            let mut branch_exit_jumps = Vec::new();
            let total_branches = choice.branches.len();

            for (idx, &branch_id) in choice.branches.iter().enumerate() {
                let is_last = idx.saturating_add(1) == total_branches;

                if !is_last {
                    let save_addr = self.program.emit(Instruction::SavePou { rollback_ip: 0 });
                    self.lower_term(branch_id)?;
                    self.program.emit(Instruction::CommitPou);
                    let jump_addr = self.program.emit(Instruction::Jump { target_ip: 0 });
                    try_push(&mut branch_exit_jumps, jump_addr)?;

                    let next_branch_ip = self.program.instructions.len() as u32;
                    self.program.patch_target(save_addr, next_branch_ip);
                } else {
                    self.lower_term(branch_id)?;
                }
            }

            let choice_end_ip = self.program.instructions.len() as u32;
            for jump_addr in branch_exit_jumps {
                self.program.patch_target(jump_addr, choice_end_ip);
            }
        }

        Ok(())
    }

    /// Resolves an integer or character discriminator key for a choice branch.
    fn get_branch_key(&self, branch_term_id: NodeId) -> Option<u32> {
        let term = self.schema.get_term(branch_term_id)?;
        if let Some(ref key_str) = term.properties.choice_branch_key {
            let trimmed = key_str.trim();
            if let Ok(num) = trimmed.parse::<u32>() {
                return Some(num);
            }
            if trimmed.len() == 1 {
                return trimmed.as_bytes().first().map(|&b| b as u32);
            }
        }
        if let Some(ref init) = term.properties.initiator {
            if init.len() == 1 {
                return init.as_bytes().first().map(|&b| b as u32);
            }
        }
        None
    }
}
