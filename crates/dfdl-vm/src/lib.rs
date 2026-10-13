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
    use dfdl_core::infoset::value::DfdlValue;
    use dfdl_core::io::bitstream::BitReader;
    use dfdl_core::io::source::SliceByteSource;
    use dfdl_core::io::traits::{BitOrder, ByteOrder};
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
}

