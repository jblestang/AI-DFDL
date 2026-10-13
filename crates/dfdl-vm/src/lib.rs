//! # DFDL Virtual Machine (`dfdl-vm`)
//!
//! A high-performance, `#![no_std]` capable virtual machine and bytecode compiler
//! for Data Format Description Language (DFDL).
//!
//! ## Architecture Overview
//!
//! `dfdl-vm` replaces tree-walking recursive AST parsing with a flat linear
//! instruction stream executed over a register-based virtual machine:
//!
//! - **DFDL-ISA ([`isa`])**: Linear bytecode instruction set featuring specialized
//!   opcodes for sub-byte bit extraction, endianness swapping, bitstream alignment,
//!   and Points of Uncertainty (POU).
//! - **Zero-Allocation Sinks ([`arena`])**: High-throughput event sinks including
//!   contiguous arena buffers where choice backtracking operates in $O(1)$ time
//!   via vector truncation without invoking heap allocators or dropping recursive trees.
//! - **Linear Interpreter ([`interpreter`])**: Register-based engine (`R0`..`R7`)
//!   with contiguous POU stack executing without recursive stack frames.
//! - **Bytecode Compiler ([`compiler`])**: Lowers [`dfdl_core::schema::ir::CompiledSchema`]
//!   directly into linear [`BytecodeProgram`].
//! - **Validation Mode**: Designed for maximum wire-speed and line-rate decoding;
//!   operates under equivalent of [`dfdl_core::kernel::ValidationMode::Off`]
//!   (facet constraint checks are skipped during bytecode execution).

#![no_std]
#![deny(missing_docs)]

extern crate alloc;

pub mod arena;
pub mod compiler;
pub mod interpreter;
pub mod isa;

pub use arena::{ArenaEventSink, DomBuilderSink, NullEventSink, VmEvent, VmEventSink};
pub use compiler::BytecodeCompiler;
pub use interpreter::{sign_extend, VmInterpreter};
pub use isa::{
    BlockField, BlockFieldKind, BytecodeProgram, ConstantPool, ConstantPoolIndex, Instruction,
    Register, SymbolId, SymbolTable,
};

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;
    use dfdl_core::infoset::state::ElementState;
    use dfdl_core::infoset::tree::InfosetNode;
    use dfdl_core::infoset::value::{DfdlSimpleType, DfdlValue};
    use dfdl_core::io::bitstream::BitReader;
    use dfdl_core::io::source::SliceByteSource;
    use dfdl_core::io::traits::{BitOrder, ByteOrder};
    use dfdl_core::schema::builder::SchemaBuilder;
    use dfdl_core::schema::ir::*;
    use dfdl_core::types::QName;

    #[test]
    fn test_linear_binary_execution() {
        let mut prog = BytecodeProgram::new();
        let sym_root = prog.symbols.intern(QName::local("Telemetry"));
        let sym_hdr = prog.symbols.intern(QName::local("SyncWord"));
        let sym_val = prog.symbols.intern(QName::local("PayloadLength"));

        prog.emit(Instruction::StartElement {
            symbol_id: sym_root,
        });
        prog.emit(Instruction::ReadBeU16 { dest: Register::R0 });
        prog.emit(Instruction::EmitScalarInt {
            symbol_id: sym_hdr,
            reg: Register::R0,
            is_signed: false,
        });
        prog.emit(Instruction::ReadBeU16 { dest: Register::R1 });
        prog.emit(Instruction::EmitScalarInt {
            symbol_id: sym_val,
            reg: Register::R1,
            is_signed: false,
        });
        prog.emit(Instruction::EndElement {
            symbol_id: sym_root,
        });
        prog.emit(Instruction::CheckEof { expected_eof: true });
        prog.emit(Instruction::HaltSuccess);

        let data = [0xEB, 0x90, 0x00, 0x42];
        let mut reader = BitReader::new(
            SliceByteSource::new(&data),
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );

        let mut vm = VmInterpreter::new();
        let mut sink = ArenaEventSink::new();

        vm.execute(&prog, &mut reader, &mut sink)
            .expect("Execution should succeed");

        assert_eq!(sink.len(), 4);
        assert_eq!(
            sink.events()[1],
            arena::StoredEvent::ScalarInt {
                id: sym_hdr,
                val: 0xEB90,
                is_signed: false
            }
        );
        assert_eq!(
            sink.events()[2],
            arena::StoredEvent::ScalarInt {
                id: sym_val,
                val: 0x0042,
                is_signed: false
            }
        );
    }

    #[test]
    fn test_zero_alloc_choice_backtracking() {
        let mut prog = BytecodeProgram::new();
        let sym_choice = prog.symbols.intern(QName::local("ChoiceRoot"));
        let sym_b1 = prog.symbols.intern(QName::local("Branch1_U32"));
        let sym_b2 = prog.symbols.intern(QName::local("Branch2_U16"));

        prog.emit(Instruction::StartElement {
            symbol_id: sym_choice,
        });

        // Branch 1: requires 4 bytes (U32)
        let save_addr = prog.emit(Instruction::SavePou { rollback_ip: 0 });
        prog.emit(Instruction::ReadBeU32 { dest: Register::R0 });
        prog.emit(Instruction::EmitScalarInt {
            symbol_id: sym_b1,
            reg: Register::R0,
            is_signed: false,
        });
        prog.emit(Instruction::CommitPou);
        let jump_addr = prog.emit(Instruction::Jump { target_ip: 0 });

        // Branch 2 target address
        let b2_target = prog.instructions.len() as u32;
        prog.patch_target(save_addr, b2_target);

        // Branch 2: requires only 2 bytes (U16)
        prog.emit(Instruction::ReadBeU16 { dest: Register::R1 });
        prog.emit(Instruction::EmitScalarInt {
            symbol_id: sym_b2,
            reg: Register::R1,
            is_signed: false,
        });

        // Choice End
        let end_target = prog.instructions.len() as u32;
        prog.patch_target(jump_addr, end_target);

        prog.emit(Instruction::EndElement {
            symbol_id: sym_choice,
        });
        prog.emit(Instruction::HaltSuccess);

        // Feed only 2 bytes of data (so Branch 1 fails on EOF and backtracks to Branch 2)
        let data = [0x12, 0x34];
        let mut reader = BitReader::new(
            SliceByteSource::new(&data),
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );

        let mut vm = VmInterpreter::new();
        let mut sink = ArenaEventSink::new();

        vm.execute(&prog, &mut reader, &mut sink)
            .expect("Execution should succeed by backtracking to Branch 2");

        assert_eq!(sink.len(), 3);
        assert_eq!(
            sink.events()[1],
            arena::StoredEvent::ScalarInt {
                id: sym_b2,
                val: 0x1234,
                is_signed: false
            }
        );
    }

    #[test]
    fn test_dom_builder_sink() {
        let mut prog = BytecodeProgram::new();
        let sym_root = prog.symbols.intern(QName::local("Root"));
        let sym_leaf = prog.symbols.intern(QName::local("Leaf"));

        prog.emit(Instruction::StartElement {
            symbol_id: sym_root,
        });
        prog.emit(Instruction::ReadBeU16 { dest: Register::R0 });
        prog.emit(Instruction::EmitScalarInt {
            symbol_id: sym_leaf,
            reg: Register::R0,
            is_signed: false,
        });
        prog.emit(Instruction::EndElement {
            symbol_id: sym_root,
        });
        prog.emit(Instruction::HaltSuccess);

        let data = [0xCA, 0xFE];
        let mut reader = BitReader::new(
            SliceByteSource::new(&data),
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );

        let mut vm = VmInterpreter::new();
        let mut dom_sink = DomBuilderSink::new(&prog.symbols);

        vm.execute(&prog, &mut reader, &mut dom_sink)
            .expect("DOM build execution should succeed");

        let doc = dom_sink.into_document().expect("DOM doc valid");
        let root = doc.root.expect("Root exists");
        assert_eq!(root.name.local_name, "Root");
        let child = match root.children.first() {
            Some(InfosetNode::Element(ref elem)) => elem,
            _ => panic!("Expected child element"),
        };
        assert_eq!(child.name.local_name, "Leaf");
        assert_eq!(
            child.state,
            ElementState::Value(DfdlValue::UnsignedLong(0xCAFE))
        );
    }

    #[test]
    fn test_compiler_end_to_end() {
        use dfdl_core::infoset::value::DfdlSimpleType;
        use dfdl_core::schema::builder::SchemaBuilder;
        use dfdl_core::schema::ir::{
            CompiledElement, CompiledSequence, CompiledType, Representation, ResolvedProperties,
            TermKind,
        };

        let mut builder = SchemaBuilder::new();

        let mut bin_props = ResolvedProperties::default();
        bin_props.representation = Representation::Binary;
        bin_props.byte_order = ByteOrder::BigEndian;

        let elem_a = CompiledElement {
            name: QName::local("fieldA"),
            type_ir: CompiledType::Simple(DfdlSimpleType::UnsignedShort),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let id_a = builder
            .add_term_with_props(
                QName::local("fieldA"),
                TermKind::Element(elem_a),
                bin_props.clone(),
            )
            .expect("Valid term A");

        let elem_b = CompiledElement {
            name: QName::local("fieldB"),
            type_ir: CompiledType::Simple(DfdlSimpleType::UnsignedInt),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let id_b = builder
            .add_term_with_props(
                QName::local("fieldB"),
                TermKind::Element(elem_b),
                bin_props.clone(),
            )
            .expect("Valid term B");

        let seq = CompiledSequence {
            members: alloc::vec![id_a, id_b],
        };
        let seq_id = builder
            .add_term_with_props(
                QName::local("seq"),
                TermKind::Sequence(seq),
                bin_props.clone(),
            )
            .expect("Valid seq");

        let root_elem = CompiledElement {
            name: QName::local("Message"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let root_id = builder
            .add_term_with_props(
                QName::local("Message"),
                TermKind::Element(root_elem),
                bin_props,
            )
            .expect("Valid root");
        builder.set_root(root_id);

        let schema = builder.build().expect("Schema should build cleanly");

        let program = BytecodeCompiler::compile(&schema).expect("Compilation must succeed");

        // fieldA (u16: 0x1234) + fieldB (u32: 0x56789ABC)
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        let mut reader = BitReader::new(
            SliceByteSource::new(&data),
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );

        let mut vm = VmInterpreter::new();
        let mut dom_sink = DomBuilderSink::new(&program.symbols);

        vm.execute(&program, &mut reader, &mut dom_sink)
            .expect("Execution should succeed");

        let doc = dom_sink.into_document().expect("Valid doc");
        let root = doc.root.expect("Root exists");
        assert_eq!(root.name.local_name, "Message");
        assert_eq!(root.children.len(), 2);

        let child_a = match root.children.first() {
            Some(InfosetNode::Element(ref e)) => e,
            _ => panic!("Expected child A"),
        };
        assert_eq!(child_a.name.local_name, "fieldA");
        assert_eq!(
            child_a.state,
            ElementState::Value(DfdlValue::UnsignedLong(0x1234))
        );

        let child_b = match root.children.get(1) {
            Some(InfosetNode::Element(ref e)) => e,
            _ => panic!("Expected child B"),
        };
        assert_eq!(child_b.name.local_name, "fieldB");
        assert_eq!(
            child_b.state,
            ElementState::Value(DfdlValue::UnsignedLong(0x5678_9ABC))
        );
    }

    #[test]
    fn test_instruction_compression_and_fusion() {
        assert!(
            core::mem::size_of::<Instruction>() <= 16,
            "Instruction footprint is {} bytes (expected <= 16 bytes for cache line density)",
            core::mem::size_of::<Instruction>()
        );
    }

    #[test]
    fn test_execute_with_budget_exhaustion() {
        use dfdl_core::limits::WorkBudget;

        let mut prog = BytecodeProgram::new();
        let sym_a = prog.symbols.intern(QName::local("fieldA"));
        let sym_b = prog.symbols.intern(QName::local("fieldB"));

        prog.emit(Instruction::ReadBeU16 { dest: Register::R0 });
        prog.emit(Instruction::EmitScalarInt {
            symbol_id: sym_a,
            reg: Register::R0,
            is_signed: false,
        });
        prog.emit(Instruction::ReadBeU32 { dest: Register::R1 });
        prog.emit(Instruction::EmitScalarInt {
            symbol_id: sym_b,
            reg: Register::R1,
            is_signed: false,
        });

        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        let src = SliceByteSource::new(&data);
        let mut reader = BitReader::new(
            src,
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );

        let mut vm = VmInterpreter::new();
        let mut sink = ArenaEventSink::new();
        // Program has 4 instructions. Budget of 2 must abort with WorkBudgetExhausted.
        let mut budget = WorkBudget::new(2);
        let res = vm.execute_with_budget(&prog, &mut reader, &mut sink, &mut budget);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind, dfdl_core::error::DFDLErrorKind::WorkBudgetExhausted);
    }

    #[test]
    fn test_isa_symbols_and_constant_pool() {
        assert_eq!(Register::R0.index(), 0);
        assert_eq!(Register::R7.index(), 7);

        let mut syms = SymbolTable::new();
        assert!(syms.is_empty());
        let s0 = syms.intern(QName::local("foo"));
        assert_eq!(syms.len(), 1);
        let s0_again = syms.intern(QName::local("foo"));
        assert_eq!(s0, s0_again);
        let s1 = syms.intern(QName::local("bar"));
        assert_eq!(syms.len(), 2);
        assert_eq!(syms.resolve(s0).unwrap().local_name, "foo");
        assert_eq!(syms.resolve(s1).unwrap().local_name, "bar");
        assert!(syms.resolve(SymbolId(999)).is_none());

        let mut pool = ConstantPool::new();
        let c0 = pool.add_string(alloc::string::String::from("hello"));
        let c0_again = pool.add_string(alloc::string::String::from("hello"));
        assert_eq!(c0, c0_again);
        assert_eq!(pool.get_string(c0), Some("hello"));
        assert_eq!(pool.get_string(ConstantPoolIndex(99)), None);

        let b0 = pool.add_bytes(alloc::vec![1, 2, 3]);
        assert_eq!(pool.get_bytes(b0), Some(&[1u8, 2, 3][..]));
        assert_eq!(pool.get_bytes(ConstantPoolIndex(99)), None);

        let mut prog = BytecodeProgram::default();
        let addr0 = prog.emit(Instruction::Jump { target_ip: 0 });
        let addr1 = prog.emit(Instruction::JumpIfEq { reg: Register::R0, imm: 1, target_ip: 0 });
        let addr2 = prog.emit(Instruction::JumpIfNe { reg: Register::R0, imm: 2, target_ip: 0 });
        let addr3 = prog.emit(Instruction::JumpIfEof { target_ip: 0 });
        let addr4 = prog.emit(Instruction::SavePou { rollback_ip: 0 });
        let addr5 = prog.emit(Instruction::HaltSuccess);

        prog.patch_target(addr0, 100);
        prog.patch_target(addr1, 101);
        prog.patch_target(addr2, 102);
        prog.patch_target(addr3, 103);
        prog.patch_target(addr4, 104);
        prog.patch_target(addr5, 105);

        assert_eq!(prog.instructions[addr0], Instruction::Jump { target_ip: 100 });
        assert_eq!(prog.instructions[addr1], Instruction::JumpIfEq { reg: Register::R0, imm: 1, target_ip: 101 });
        assert_eq!(prog.instructions[addr2], Instruction::JumpIfNe { reg: Register::R0, imm: 2, target_ip: 102 });
        assert_eq!(prog.instructions[addr3], Instruction::JumpIfEof { target_ip: 103 });
        assert_eq!(prog.instructions[addr4], Instruction::SavePou { rollback_ip: 104 });
        assert_eq!(prog.instructions[addr5], Instruction::HaltSuccess);
    }

    #[test]
    fn test_arena_event_sink_and_stored_string() {
        use alloc::string::ToString;
        use crate::arena::StoredString;

        let s_inline = StoredString::new("short");
        assert_eq!(s_inline.as_str(), "short");
        assert_eq!(s_inline.to_string(), "short");
        assert_eq!(&*s_inline, "short");

        let s_heap = StoredString::new("this is a string that exceeds 16 bytes easily");
        assert_eq!(s_heap.as_str(), "this is a string that exceeds 16 bytes easily");
        assert_eq!(&*s_heap, "this is a string that exceeds 16 bytes easily");

        let mut sink = ArenaEventSink::with_capacity(4);
        assert!(sink.is_empty());
        assert_eq!(sink.len(), 0);

        let sym = SymbolId(0);
        sink.on_event(VmEvent::StartElement(sym)).unwrap();
        sink.on_event(VmEvent::ScalarInt { id: sym, val: 42, is_signed: false }).unwrap();
        sink.on_event(VmEvent::ScalarFloat { id: sym, val: core::f64::consts::PI }).unwrap();
        sink.on_event(VmEvent::ScalarBool { id: sym, val: true }).unwrap();
        sink.on_event(VmEvent::ScalarString { id: sym, val: "val" }).unwrap();
        sink.on_event(VmEvent::EndElement(sym)).unwrap();

        assert_eq!(sink.len(), 6);
        assert_eq!(sink.events().len(), 6);
        let cp = sink.checkpoint();
        assert_eq!(cp, 6);
        sink.rollback(3);
        assert_eq!(sink.len(), 3);
        sink.clear();
        assert!(sink.is_empty());

        let mut null_sink = NullEventSink::new();
        null_sink.on_event(VmEvent::StartElement(sym)).unwrap();
        assert_eq!(null_sink.checkpoint(), 1);
        null_sink.rollback(0);
        assert_eq!(null_sink.checkpoint(), 0);
    }

    #[test]
    fn test_dom_builder_sink_edge_cases() {
        let mut syms = SymbolTable::new();
        let sym_root = syms.intern(QName::local("Root"));
        let sym_child = syms.intern(QName::local("Child"));

        let mut dom = DomBuilderSink::new(&syms);
        assert!(dom.clone().into_document().is_err());

        // Error: unresolved symbol ID in StartElement
        assert!(dom.on_event(VmEvent::StartElement(SymbolId(999))).is_err());
        // Error: unexpected EndElement with empty stack
        assert!(dom.on_event(VmEvent::EndElement(sym_root)).is_err());

        // Successful build with float, bool, string, int
        dom.on_event(VmEvent::StartElement(sym_root)).unwrap();
        dom.on_event(VmEvent::ScalarFloat { id: sym_child, val: 1.5 }).unwrap();
        dom.on_event(VmEvent::ScalarBool { id: sym_child, val: false }).unwrap();
        dom.on_event(VmEvent::ScalarString { id: sym_child, val: "text" }).unwrap();
        dom.on_event(VmEvent::ScalarInt { id: sym_child, val: -10, is_signed: true }).unwrap();
        // Mismatched EndElement
        let mut bad_dom = dom.clone();
        assert!(bad_dom.on_event(VmEvent::EndElement(sym_child)).is_err());

        dom.on_event(VmEvent::EndElement(sym_root)).unwrap();
        let doc = dom.into_document().unwrap();
        assert_eq!(doc.root.unwrap().name.local_name, "Root");

        // Simple element in-place value update
        let mut dom2 = DomBuilderSink::new(&syms);
        dom2.on_event(VmEvent::StartElement(sym_child)).unwrap();
        dom2.on_event(VmEvent::ScalarInt { id: sym_child, val: 100, is_signed: false }).unwrap();
        dom2.on_event(VmEvent::EndElement(sym_child)).unwrap();
        let doc2 = dom2.into_document().unwrap();
        assert_eq!(doc2.root.unwrap().state, ElementState::Value(DfdlValue::UnsignedLong(100)));

        // Unresolved scalar errors
        let mut dom3 = DomBuilderSink::new(&syms);
        assert!(dom3.on_event(VmEvent::ScalarInt { id: SymbolId(999), val: 1, is_signed: false }).is_err());
        assert!(dom3.on_event(VmEvent::ScalarFloat { id: SymbolId(999), val: 1.0 }).is_err());
        assert!(dom3.on_event(VmEvent::ScalarBool { id: SymbolId(999), val: true }).is_err());
        assert!(dom3.on_event(VmEvent::ScalarString { id: SymbolId(999), val: "a" }).is_err());
    }

    #[test]
    fn test_interpreter_sign_extend_and_registers() {
        assert_eq!(sign_extend(0, 0), 0);
        assert_eq!(sign_extend(1, 1), 1);
        assert_eq!(sign_extend(0x7F, 8), 127);
        assert_eq!(sign_extend(0x80, 8), -128);
        assert_eq!(sign_extend(0xFF, 8), -1);
        assert_eq!(sign_extend(0x7FFF, 16), 32767);
        assert_eq!(sign_extend(0x8000, 16), -32768);
        assert_eq!(sign_extend(0xFFFF_FFFF_FFFF_FFFF, 64), -1);

        let mut vm = VmInterpreter::default();
        vm.set_reg(Register::R3, 0x1234);
        assert_eq!(vm.get_reg(Register::R3), 0x1234);
        vm.reset();
        assert_eq!(vm.get_reg(Register::R3), 0);
    }

    #[test]
    fn test_interpreter_all_scalar_and_endian_opcodes() {
        let mut prog = BytecodeProgram::new();
        let sym_a = prog.symbols.intern(QName::local("A"));

        // ReadBitsU8, U16, U32, U64, Signed
        prog.emit(Instruction::ReadBitsU8 { dest: Register::R0, count: 4 });
        prog.emit(Instruction::ReadBitsU16 { dest: Register::R1, count: 12 });
        prog.emit(Instruction::ReadBitsU32 { dest: Register::R2, count: 20 });
        prog.emit(Instruction::ReadBitsU64 { dest: Register::R3, count: 28 });
        prog.emit(Instruction::ReadBitsSigned { dest: Register::R4, count: 8 });

        // Little Endian reads
        prog.emit(Instruction::ReadLeU16 { dest: Register::R5 });
        prog.emit(Instruction::ReadLeU32 { dest: Register::R6 });
        prog.emit(Instruction::ReadLeU64 { dest: Register::R7 });
        prog.emit(Instruction::ReadLeI16 { dest: Register::R0 });
        prog.emit(Instruction::ReadLeI32 { dest: Register::R1 });
        prog.emit(Instruction::ReadLeI64 { dest: Register::R2 });
        prog.emit(Instruction::ReadLeF32 { dest: Register::R3 });
        prog.emit(Instruction::ReadLeF64 { dest: Register::R4 });

        // Big Endian signed & floats
        prog.emit(Instruction::ReadBeI16 { dest: Register::R5 });
        prog.emit(Instruction::ReadBeI32 { dest: Register::R6 });
        prog.emit(Instruction::ReadBeI64 { dest: Register::R7 });
        prog.emit(Instruction::ReadBeF32 { dest: Register::R0 });
        prog.emit(Instruction::ReadBeF64 { dest: Register::R1 });

        // Arithmetic
        prog.emit(Instruction::LoadImm { dest: Register::R0, imm: 10 });
        prog.emit(Instruction::AddImm { dest: Register::R0, imm: 5 });
        prog.emit(Instruction::SubImm { dest: Register::R0, imm: 3 });

        // ReadBytes & AlignBits & SkipBits
        let const_idx = prog.constants.add_bytes(alloc::vec![]);
        prog.emit(Instruction::ReadBytes { dest_const: const_idx, len: 2 });
        prog.emit(Instruction::AlignBits { alignment_bits: 8 });
        prog.emit(Instruction::SkipBits { bit_count: 8 });

        // MoveReg, PeekBitsU8, RecordPosition, PadToLength
        prog.emit(Instruction::RecordPosition { dest: Register::R6 });
        prog.emit(Instruction::PeekBitsU8 { dest: Register::R5, count: 8 });
        prog.emit(Instruction::MoveReg { dest: Register::R4, src: Register::R0 });
        prog.emit(Instruction::PadToLength { start_reg: Register::R6, total_bytes: 2 });

        // Emit scalar variants
        let s_idx = prog.constants.add_string(alloc::string::String::from("test_const"));
        prog.emit(Instruction::EmitScalarFloat { symbol_id: sym_a, reg: Register::R0, is_f64: false });
        prog.emit(Instruction::EmitScalarFloat { symbol_id: sym_a, reg: Register::R1, is_f64: true });
        prog.emit(Instruction::EmitScalarBool { symbol_id: sym_a, val: true });
        prog.emit(Instruction::EmitScalarString { symbol_id: sym_a, const_idx: s_idx });

        // Strings & BCD
        prog.emit(Instruction::ReadString { symbol_id: sym_a, len: 4, trim_nul: true });
        prog.emit(Instruction::ReadString { symbol_id: sym_a, len: 70, trim_nul: false });
        prog.emit(Instruction::ReadBcdTime { symbol_id: sym_a });

        // Jumps and loops
        prog.emit(Instruction::JumpIfEq { reg: Register::R0, imm: 999, target_ip: 0 });
        prog.emit(Instruction::JumpIfNe { reg: Register::R0, imm: 12, target_ip: 0 });
        prog.emit(Instruction::Jump { target_ip: 0 }); // target will be patched

        let jump_pos = prog.instructions.len() - 1;
        let next_ip = prog.instructions.len() as u32;
        prog.patch_target(jump_pos, next_ip);

        // Fused instructions
        prog.emit(Instruction::ReadEmitBeU16 { symbol_id: sym_a, is_signed: true });
        prog.emit(Instruction::ReadEmitLeU16 { symbol_id: sym_a, is_signed: true });
        prog.emit(Instruction::ReadEmitBeU32 { symbol_id: sym_a, is_signed: true });
        prog.emit(Instruction::ReadEmitLeU32 { symbol_id: sym_a, is_signed: true });
        prog.emit(Instruction::ReadEmitBitsU8 { symbol_id: sym_a, count: 8, is_signed: true });
        prog.emit(Instruction::ReadEmitBool { symbol_id: sym_a, count: 8, true_val: 1 });

        // Fused ReadBlock
        let b_start = prog.block_fields.len() as u32;
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BeU16 { is_signed: true } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::LeU16 { is_signed: false } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BeU32 { is_signed: true } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::LeU32 { is_signed: false } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BeU64 { is_signed: true } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::LeU64 { is_signed: false } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BitsU8 { count: 8, is_signed: false } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::Bool { count: 8, true_val: 1 } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::FixedString { len: 4, trim_nul: true } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BcdTime });
        let b_count = (prog.block_fields.len() as u32 - b_start) as u16;
        prog.emit(Instruction::ReadBlock { start_idx: b_start, field_count: b_count });

        prog.emit(Instruction::CheckEof { expected_eof: false });
        prog.emit(Instruction::HaltSuccess);

        // Prepare large enough dummy data stream
        let mut data = alloc::vec![0u8; 300];
        // Populate BCD time fields with valid 0x12, 0x34, 0x56
        data[250] = 0x12;
        data[251] = 0x34;
        data[252] = 0x56;

        let mut reader = BitReader::new(
            SliceByteSource::new(&data),
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );

        let mut vm = VmInterpreter::new();
        let mut sink = ArenaEventSink::new();
        vm.execute(&prog, &mut reader, &mut sink).expect("Program should execute cleanly");
    }

    #[test]
    fn test_interpreter_control_flow_and_backtracking() {
        let mut prog = BytecodeProgram::new();
        let sym_a = prog.symbols.intern(QName::local("A"));

        // Test POU backtracking on CheckEof mismatch
        let save_ip = prog.emit(Instruction::SavePou { rollback_ip: 0 });
        prog.emit(Instruction::ReadBeU16 { dest: Register::R0 });
        prog.emit(Instruction::CheckEof { expected_eof: true }); // Fails since data has more bytes!
        prog.emit(Instruction::HaltSuccess);

        let branch2_ip = prog.instructions.len() as u32;
        prog.patch_target(save_ip, branch2_ip);
        prog.emit(Instruction::ReadBeU16 { dest: Register::R1 });
        prog.emit(Instruction::EmitScalarInt { symbol_id: sym_a, reg: Register::R1, is_signed: false });
        prog.emit(Instruction::HaltSuccess);

        let data = [0x11, 0x22, 0x33, 0x44];
        let mut reader = BitReader::new(
            SliceByteSource::new(&data),
            BitOrder::MostSignificantBitFirst,
            ByteOrder::BigEndian,
        );
        let mut vm = VmInterpreter::new();
        let mut sink = ArenaEventSink::new();
        vm.execute(&prog, &mut reader, &mut sink).expect("POU rollback should branch to branch2");
        assert_eq!(vm.get_reg(Register::R1), 0x1122);

        // Test RollbackPou without POU frame -> error
        let mut bad_prog = BytecodeProgram::new();
        bad_prog.emit(Instruction::RollbackPou);
        let mut r2 = BitReader::new(SliceByteSource::new(&data), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        assert!(vm.execute(&bad_prog, &mut r2, &mut sink).is_err());

        // Test HaltError with and without POU
        let mut halt_prog = BytecodeProgram::new();
        halt_prog.emit(Instruction::HaltError { error_code: 42 });
        let mut r3 = BitReader::new(SliceByteSource::new(&data), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        assert!(vm.execute(&halt_prog, &mut r3, &mut sink).is_err());
    }

    #[test]
    fn test_compiler_lowerings() {
        // Test compiler with various primitive types and framing
        let mut builder = SchemaBuilder::new();
        let mut props = ResolvedProperties::default();
        props.representation = Representation::Binary;
        props.byte_order = ByteOrder::LittleEndian;
        props.alignment = 2;
        props.leading_skip = 1;
        props.trailing_skip = 1;

        let root_elem = CompiledElement {
            name: QName::local("TestRoot"),
            type_ir: CompiledType::Simple(DfdlSimpleType::UnsignedShort),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let r_id = builder.add_term_with_props(QName::local("TestRoot"), TermKind::Element(root_elem), props).unwrap();
        builder.set_root(r_id);
        let schema = builder.build().unwrap();
        let prog = BytecodeCompiler::compile(&schema).unwrap();
        assert!(!prog.instructions.is_empty());

        // Test choice with branch key
        let mut b2 = SchemaBuilder::new();
        let mut bprops = ResolvedProperties::default();
        bprops.representation = Representation::Binary;
        bprops.choice_branch_key = Some(alloc::string::String::from("65")); // 'A'
        let elem_a = CompiledElement {
            name: QName::local("A"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let ea_id = b2.add_term_with_props(QName::local("A"), TermKind::Element(elem_a), bprops).unwrap();

        let mut bprops2 = ResolvedProperties::default();
        bprops2.representation = Representation::Binary;
        bprops2.choice_branch_key = Some(alloc::string::String::from("66")); // 'B'
        let elem_b = CompiledElement {
            name: QName::local("B"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let eb_id = b2.add_term_with_props(QName::local("B"), TermKind::Element(elem_b), bprops2).unwrap();

        let choice = CompiledChoice {
            branches: alloc::vec![ea_id, eb_id],
        };
        let choice_id = b2.add_term(QName::local("Choice"), TermKind::Choice(choice)).unwrap();
        let root2 = CompiledElement {
            name: QName::local("RootChoice"),
            type_ir: CompiledType::Complex(choice_id),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let mut rprops = ResolvedProperties::default();
        rprops.representation = Representation::Binary;
        let r2_id = b2.add_term_with_props(QName::local("RootChoice"), TermKind::Element(root2), rprops).unwrap();
        b2.set_root(r2_id);
        let schema2 = b2.build().unwrap();
        let prog2 = BytecodeCompiler::compile(&schema2).unwrap();
        assert!(!prog2.instructions.is_empty());

        // Test OccursCountKind::Implicit repetition with EOF
        let mut b3 = SchemaBuilder::new();
        let mut iprops = ResolvedProperties::default();
        iprops.representation = Representation::Binary;
        iprops.occurs_count_kind = OccursCountKind::Implicit;
        let var_elem = CompiledElement {
            name: QName::local("VarElem"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(5),
            default_value: None,
            is_nillable: false,
        };
        let v_id = b3.add_term_with_props(QName::local("VarElem"), TermKind::Element(var_elem), iprops).unwrap();
        b3.set_root(v_id);
        let schema3 = b3.build().unwrap();
        let prog3 = BytecodeCompiler::compile(&schema3).unwrap();
        assert!(!prog3.instructions.is_empty());
    }

    #[test]
    fn test_compiler_block_fusion_and_all_types() {
        let mut builder = SchemaBuilder::new();
        let mut seq_members = alloc::vec![];

        let types_and_props = [
            (DfdlSimpleType::Byte, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::UnsignedByte, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::Short, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::Short, ByteOrder::LittleEndian, None, None),
            (DfdlSimpleType::UnsignedShort, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::UnsignedShort, ByteOrder::LittleEndian, None, None),
            (DfdlSimpleType::Int, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::Int, ByteOrder::LittleEndian, None, None),
            (DfdlSimpleType::UnsignedInt, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::UnsignedInt, ByteOrder::LittleEndian, None, None),
            (DfdlSimpleType::Long, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::Long, ByteOrder::LittleEndian, None, None),
            (DfdlSimpleType::UnsignedLong, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::UnsignedLong, ByteOrder::LittleEndian, None, None),
            (DfdlSimpleType::Boolean, ByteOrder::BigEndian, Some(LengthUnits::Bits), Some(4)),
            (DfdlSimpleType::String, ByteOrder::BigEndian, None, Some(8)),
            (DfdlSimpleType::Time, ByteOrder::BigEndian, None, None),
            (DfdlSimpleType::Byte, ByteOrder::BigEndian, Some(LengthUnits::Bits), Some(6)),
        ];

        for (i, (ty, bo, lu, len)) in types_and_props.iter().enumerate() {
            let mut p = ResolvedProperties::default();
            p.representation = Representation::Binary;
            p.byte_order = *bo;
            if let Some(u) = lu {
                p.length_units = *u;
            }
            if let Some(l) = len {
                p.length = Some(*l);
            }
            if *ty == DfdlSimpleType::Time {
                p.binary_calendar_rep = BinaryCalendarRep::Bcd;
            }
            let name = alloc::format!("f{}", i);
            let elem = CompiledElement {
                name: QName::local(&name),
                type_ir: CompiledType::Simple(*ty),
                min_occurs: 1,
                max_occurs: Some(1),
                default_value: None,
                is_nillable: false,
            };
            let tid = builder.add_term_with_props(QName::local(&name), TermKind::Element(elem), p).unwrap();
            seq_members.push(tid);
        }

        let seq_id = builder.add_term(QName::local("Seq"), TermKind::Sequence(CompiledSequence { members: seq_members })).unwrap();
        let root = CompiledElement {
            name: QName::local("RootSeq"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let mut rp = ResolvedProperties::default();
        rp.representation = Representation::Binary;
        let r_id = builder.add_term_with_props(QName::local("RootSeq"), TermKind::Element(root), rp).unwrap();
        builder.set_root(r_id);

        let schema = builder.build().unwrap();
        let prog = BytecodeCompiler::compile(&schema).unwrap();
        assert!(!prog.instructions.is_empty());
        assert!(!prog.block_fields.is_empty());
    }

    #[test]
    fn test_compiler_floats_delimiters_and_choices() {
        let mut b = SchemaBuilder::new();
        // Element with initiator delimiter
        let mut p_init = ResolvedProperties::default();
        p_init.representation = Representation::Binary;
        p_init.initiator = Some(alloc::string::String::from("X"));
        let e_init = CompiledElement {
            name: QName::local("WithInit"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let init_id = b.add_term_with_props(QName::local("WithInit"), TermKind::Element(e_init), p_init).unwrap();

        // Floats and Doubles (BE and LE)
        let float_types = [
            (DfdlSimpleType::Float, ByteOrder::BigEndian),
            (DfdlSimpleType::Float, ByteOrder::LittleEndian),
            (DfdlSimpleType::Double, ByteOrder::BigEndian),
            (DfdlSimpleType::Double, ByteOrder::LittleEndian),
        ];
        let mut float_ids = alloc::vec![init_id];
        for (ty, bo) in float_types {
            let mut p = ResolvedProperties::default();
            p.representation = Representation::Binary;
            p.byte_order = bo;
            let el = CompiledElement {
                name: QName::local("flt"),
                type_ir: CompiledType::Simple(ty),
                min_occurs: 1,
                max_occurs: Some(1),
                default_value: None,
                is_nillable: false,
            };
            let tid = b.add_term_with_props(QName::local("flt"), TermKind::Element(el), p).unwrap();
            float_ids.push(tid);
        }

        // Boolean > 8 bits
        let mut p_bool = ResolvedProperties::default();
        p_bool.representation = Representation::Binary;
        p_bool.length_units = LengthUnits::Bits;
        p_bool.length = Some(16);
        let el_bool = CompiledElement {
            name: QName::local("big_bool"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Boolean),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let bool_id = b.add_term_with_props(QName::local("big_bool"), TermKind::Element(el_bool), p_bool).unwrap();
        float_ids.push(bool_id);

        // Sub-byte bits < 64 (signed and unsigned)
        let mut p_sub_s = ResolvedProperties::default();
        p_sub_s.representation = Representation::Binary;
        p_sub_s.length_units = LengthUnits::Bits;
        p_sub_s.length = Some(12);
        let el_sub_s = CompiledElement {
            name: QName::local("sub_s"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Short),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let sub_s_id = b.add_term_with_props(QName::local("sub_s"), TermKind::Element(el_sub_s), p_sub_s).unwrap();
        float_ids.push(sub_s_id);

        let mut p_sub_u = ResolvedProperties::default();
        p_sub_u.representation = Representation::Binary;
        p_sub_u.length_units = LengthUnits::Bits;
        p_sub_u.length = Some(12);
        let el_sub_u = CompiledElement {
            name: QName::local("sub_u"),
            type_ir: CompiledType::Simple(DfdlSimpleType::UnsignedShort),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let sub_u_id = b.add_term_with_props(QName::local("sub_u"), TermKind::Element(el_sub_u), p_sub_u).unwrap();
        float_ids.push(sub_u_id);

        // Repetition with OccursCountKind::Expression
        let mut p_expr = ResolvedProperties::default();
        p_expr.representation = Representation::Binary;
        p_expr.occurs_count_kind = OccursCountKind::Expression;
        let el_expr = CompiledElement {
            name: QName::local("expr_rep"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(10),
            default_value: None,
            is_nillable: false,
        };
        let expr_id = b.add_term_with_props(QName::local("expr_rep"), TermKind::Element(el_expr), p_expr).unwrap();
        float_ids.push(expr_id);

        // Fixed repetition count > 1
        let mut p_fixed = ResolvedProperties::default();
        p_fixed.representation = Representation::Binary;
        let el_fixed = CompiledElement {
            name: QName::local("fixed_rep"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 3,
            max_occurs: Some(3),
            default_value: None,
            is_nillable: false,
        };
        let fixed_id = b.add_term_with_props(QName::local("fixed_rep"), TermKind::Element(el_fixed), p_fixed).unwrap();
        float_ids.push(fixed_id);

        // Speculative choice (no branch keys)
        let mut p_branch = ResolvedProperties::default();
        p_branch.representation = Representation::Binary;
        let branch1 = b.add_term_with_props(QName::local("Br1"), TermKind::Element(CompiledElement {
            name: QName::local("Br1"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        }), p_branch.clone()).unwrap();
        let branch2 = b.add_term_with_props(QName::local("Br2"), TermKind::Element(CompiledElement {
            name: QName::local("Br2"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        }), p_branch).unwrap();
        let spec_choice = b.add_term(QName::local("SpecChoice"), TermKind::Choice(CompiledChoice {
            branches: alloc::vec![branch1, branch2],
        })).unwrap();
        float_ids.push(spec_choice);

        // Complex element with explicit length padding
        let sub_seq = b.add_term(QName::local("SubSeq"), TermKind::Sequence(CompiledSequence { members: alloc::vec![branch1] })).unwrap();
        let mut p_complex = ResolvedProperties::default();
        p_complex.representation = Representation::Binary;
        p_complex.length_kind = LengthKind::Explicit;
        p_complex.length = Some(8);
        let el_complex = CompiledElement {
            name: QName::local("ComplexPadded"),
            type_ir: CompiledType::Complex(sub_seq),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let complex_id = b.add_term_with_props(QName::local("ComplexPadded"), TermKind::Element(el_complex), p_complex).unwrap();
        float_ids.push(complex_id);

        // Nested sequence without framing (flattening)
        let inner_seq = b.add_term(QName::local("InnerSeq"), TermKind::Sequence(CompiledSequence { members: alloc::vec![branch2] })).unwrap();
        let group_ref = b.add_term(QName::local("GRef"), TermKind::GroupRef(inner_seq)).unwrap();
        float_ids.push(group_ref);

        let main_seq = b.add_term(QName::local("MainSeq"), TermKind::Sequence(CompiledSequence { members: float_ids })).unwrap();
        let root = CompiledElement {
            name: QName::local("AllRoot"),
            type_ir: CompiledType::Complex(main_seq),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let mut r_props = ResolvedProperties::default();
        r_props.representation = Representation::Binary;
        let r_id = b.add_term_with_props(QName::local("AllRoot"), TermKind::Element(root), r_props).unwrap();
        b.set_root(r_id);

        let schema = b.build().unwrap();
        let prog = BytecodeCompiler::compile(&schema).unwrap();
        assert!(!prog.instructions.is_empty());
    }

    #[test]
    fn test_compiler_error_cases() {
        // Text representation error
        let mut b = SchemaBuilder::new();
        let mut p = ResolvedProperties::default();
        p.representation = Representation::Text;
        let elem = CompiledElement {
            name: QName::local("TextElem"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let tid = b.add_term_with_props(QName::local("TextElem"), TermKind::Element(elem), p).unwrap();
        b.set_root(tid);
        let s = b.build().unwrap();
        assert!(BytecodeCompiler::compile(&s).is_err());

        // Empty choice branches
        let mut b2 = SchemaBuilder::new();
        let choice = CompiledChoice { branches: alloc::vec![] };
        let cid = b2.add_term(QName::local("EmptyChoice"), TermKind::Choice(choice)).unwrap();
        let root = CompiledElement {
            name: QName::local("R"),
            type_ir: CompiledType::Complex(cid),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let rid = b2.add_term_with_props(QName::local("R"), TermKind::Element(root), ResolvedProperties::default()).unwrap();
        b2.set_root(rid);
        let s2 = b2.build().unwrap();
        assert!(BytecodeCompiler::compile(&s2).is_ok());

        // Unsupported calendar error
        let mut b3 = SchemaBuilder::new();
        let mut p3 = ResolvedProperties::default();
        p3.representation = Representation::Binary;
        p3.binary_calendar_rep = BinaryCalendarRep::Packed;
        let elem3 = CompiledElement {
            name: QName::local("DateElem"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Date),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let tid3 = b3.add_term_with_props(QName::local("DateElem"), TermKind::Element(elem3), p3).unwrap();
        b3.set_root(tid3);
        let s3 = b3.build().unwrap();
        assert!(BytecodeCompiler::compile(&s3).is_err());
    }

    #[test]
    fn test_dom_builder_sink_in_place_all_types() {
        let mut syms = SymbolTable::new();
        let s_flt = syms.intern(QName::local("Flt"));
        let s_bool = syms.intern(QName::local("Bool"));
        let s_str = syms.intern(QName::local("Str"));

        // Float in place
        let mut d1 = DomBuilderSink::new(&syms);
        d1.on_event(VmEvent::StartElement(s_flt)).unwrap();
        assert_eq!(d1.checkpoint(), 1);
        d1.on_event(VmEvent::ScalarFloat { id: s_flt, val: core::f64::consts::E }).unwrap();
        d1.on_event(VmEvent::EndElement(s_flt)).unwrap();
        let doc1 = d1.into_document().unwrap();
        assert_eq!(doc1.root.unwrap().state, ElementState::Value(DfdlValue::Double(core::f64::consts::E)));

        // Bool in place
        let mut d2 = DomBuilderSink::new(&syms);
        d2.on_event(VmEvent::StartElement(s_bool)).unwrap();
        d2.on_event(VmEvent::ScalarBool { id: s_bool, val: true }).unwrap();
        d2.on_event(VmEvent::EndElement(s_bool)).unwrap();
        let doc2 = d2.into_document().unwrap();
        assert_eq!(doc2.root.unwrap().state, ElementState::Value(DfdlValue::Boolean(true)));

        // String in place
        let mut d3 = DomBuilderSink::new(&syms);
        d3.on_event(VmEvent::StartElement(s_str)).unwrap();
        d3.on_event(VmEvent::ScalarString { id: s_str, val: "val3" }).unwrap();
        d3.on_event(VmEvent::EndElement(s_str)).unwrap();
        let doc3 = d3.into_document().unwrap();
        assert_eq!(doc3.root.unwrap().state, ElementState::Value(DfdlValue::String(alloc::string::String::from("val3"))));

        // DomBuilderSink rollback
        let mut d4 = DomBuilderSink::new(&syms);
        d4.on_event(VmEvent::StartElement(s_str)).unwrap();
        assert_eq!(d4.checkpoint(), 1);
        d4.rollback(0);
        assert_eq!(d4.checkpoint(), 0);
    }

    #[test]
    fn test_interpreter_extra_branches() {
        let mut prog = BytecodeProgram::new();
        let sym_a = prog.symbols.intern(QName::local("Extra"));

        // Test CommitPou
        prog.emit(Instruction::SavePou { rollback_ip: 0 });
        prog.emit(Instruction::CommitPou);

        // Test HaltError with active POU frame
        let save_halt = prog.emit(Instruction::SavePou { rollback_ip: 0 });
        prog.emit(Instruction::HaltError { error_code: 99 });
        let after_halt = prog.instructions.len() as u32;
        prog.patch_target(save_halt, after_halt);

        // Fused unsigned variants
        prog.emit(Instruction::ReadEmitBeU16 { symbol_id: sym_a, is_signed: false });
        prog.emit(Instruction::ReadEmitLeU16 { symbol_id: sym_a, is_signed: false });
        prog.emit(Instruction::ReadEmitBeU32 { symbol_id: sym_a, is_signed: false });
        prog.emit(Instruction::ReadEmitLeU32 { symbol_id: sym_a, is_signed: false });
        prog.emit(Instruction::ReadEmitBitsU8 { symbol_id: sym_a, count: 8, is_signed: false });

        // Fused Block with opposite sign variants
        let b_start = prog.block_fields.len() as u32;
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BeU16 { is_signed: false } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::LeU16 { is_signed: true } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BeU32 { is_signed: false } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::LeU32 { is_signed: true } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BeU64 { is_signed: false } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::LeU64 { is_signed: true } });
        prog.block_fields.push(BlockField { symbol_id: sym_a, kind: BlockFieldKind::BitsU8 { count: 8, is_signed: true } });
        let b_count = (prog.block_fields.len() as u32 - b_start) as u16;
        prog.emit(Instruction::ReadBlock { start_idx: b_start, field_count: b_count });

        // JumpIfEq taken, JumpIfNe taken
        prog.emit(Instruction::LoadImm { dest: Register::R0, imm: 77 });
        let j_eq = prog.emit(Instruction::JumpIfEq { reg: Register::R0, imm: 77, target_ip: 0 });
        prog.emit(Instruction::HaltError { error_code: 1 });
        let target_eq = prog.instructions.len() as u32;
        prog.patch_target(j_eq, target_eq);

        let j_ne = prog.emit(Instruction::JumpIfNe { reg: Register::R0, imm: 88, target_ip: 0 });
        prog.emit(Instruction::HaltError { error_code: 2 });
        let target_ne = prog.instructions.len() as u32;
        prog.patch_target(j_ne, target_ne);

        prog.emit(Instruction::HaltSuccess);

        let data = [0x55; 128];
        let mut reader = BitReader::new(SliceByteSource::new(&data), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut vm = VmInterpreter::new();
        let mut sink = ArenaEventSink::new();
        vm.execute(&prog, &mut reader, &mut sink).expect("Extra branches should execute cleanly");
    }

    #[test]
    fn test_compiler_unsupported_type_and_missing_node() {
        // Unsupported simple type (HexBinary)
        let mut b = SchemaBuilder::new();
        let mut p = ResolvedProperties::default();
        p.representation = Representation::Binary;
        let elem = CompiledElement {
            name: QName::local("HexElem"),
            type_ir: CompiledType::Simple(DfdlSimpleType::HexBinary),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let tid = b.add_term_with_props(QName::local("HexElem"), TermKind::Element(elem), p).unwrap();
        b.set_root(tid);
        let s = b.build().unwrap();
        assert!(BytecodeCompiler::compile(&s).is_err());

        // Choice branch with char key
        let mut b2 = SchemaBuilder::new();
        let mut p_c1 = ResolvedProperties::default();
        p_c1.representation = Representation::Binary;
        p_c1.choice_branch_key = Some(alloc::string::String::from("Z"));
        let e1 = CompiledElement {
            name: QName::local("E1"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let t1 = b2.add_term_with_props(QName::local("E1"), TermKind::Element(e1), p_c1).unwrap();

        let mut p_c2 = ResolvedProperties::default();
        p_c2.representation = Representation::Binary;
        p_c2.initiator = Some(alloc::string::String::from("W"));
        let e2 = CompiledElement {
            name: QName::local("E2"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let t2 = b2.add_term_with_props(QName::local("E2"), TermKind::Element(e2), p_c2).unwrap();

        let choice = CompiledChoice { branches: alloc::vec![t1, t2] };
        let cid = b2.add_term(QName::local("C"), TermKind::Choice(choice)).unwrap();
        let root = CompiledElement {
            name: QName::local("RootC"),
            type_ir: CompiledType::Complex(cid),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let mut pr = ResolvedProperties::default();
        pr.representation = Representation::Binary;
        let rid = b2.add_term_with_props(QName::local("RootC"), TermKind::Element(root), pr).unwrap();
        b2.set_root(rid);
        let s2 = b2.build().unwrap();
        let prog2 = BytecodeCompiler::compile(&s2).unwrap();
        assert!(!prog2.instructions.is_empty());
    }

    #[test]
    fn test_interpreter_eof_align_pad_and_missing_node() {
        let mut prog = BytecodeProgram::new();
        let sym_a = prog.symbols.intern(QName::local("A"));

        // Read 3 bits to make stream unaligned
        prog.emit(Instruction::ReadBitsU8 { dest: Register::R0, count: 3 });
        // Align to 8 bits
        prog.emit(Instruction::AlignBits { alignment_bits: 8 });
        // Record position
        prog.emit(Instruction::RecordPosition { dest: Register::R1 });
        // Read 1 byte
        prog.emit(Instruction::ReadBitsU8 { dest: Register::R2, count: 8 });
        // PadToLength with target > current
        prog.emit(Instruction::PadToLength { start_reg: Register::R1, total_bytes: 4 });
        // Read string with trailing NUL
        prog.emit(Instruction::ReadString { symbol_id: sym_a, len: 4, trim_nul: true });
        // JumpIfEof when NOT eof
        prog.emit(Instruction::JumpIfEof { target_ip: 999 });
        // Skip remaining
        prog.emit(Instruction::SkipBits { bit_count: 16 });
        // JumpIfEof when EOF
        let j_eof = prog.emit(Instruction::JumpIfEof { target_ip: 0 });
        prog.emit(Instruction::HaltError { error_code: 3 });
        let after_eof = prog.instructions.len() as u32;
        prog.patch_target(j_eof, after_eof);
        prog.emit(Instruction::CheckEof { expected_eof: true });
        prog.emit(Instruction::HaltSuccess);

        let data = [0xFF, 0x11, 0x00, 0x00, 0x00, b'o', b'k', 0, 0, 0xAA, 0xBB];
        let mut reader = BitReader::new(SliceByteSource::new(&data), BitOrder::MostSignificantBitFirst, ByteOrder::BigEndian);
        let mut vm = VmInterpreter::new();
        let mut sink = ArenaEventSink::new();
        vm.execute(&prog, &mut reader, &mut sink).expect("Should execute to EOF cleanly");

        // Missing term compiler error
        let mut b = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("R"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Byte),
            min_occurs: 1,
            max_occurs: Some(1),
            default_value: None,
            is_nillable: false,
        };
        let mut pr = ResolvedProperties::default();
        pr.representation = Representation::Binary;
        let rid = b.add_term_with_props(QName::local("R"), TermKind::Element(elem), pr).unwrap();
        b.set_root(rid);
        let mut s = b.build().unwrap();
        s.root_element_id = NodeId(9999);
        assert!(BytecodeCompiler::compile(&s).is_err());
    }
}


