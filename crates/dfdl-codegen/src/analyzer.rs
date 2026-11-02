//! Schema Analyzer for DFDL Ahead-of-Time (AOT) Code Generation.
//!
//! Traverses [`dfdl_core::schema::ir::CompiledSchema`] models to extract
//! normalized [`crate::ir::CodeGenIr`] structures for Rust code emission.

extern crate alloc;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use dfdl_core::infoset::value::DfdlSimpleType;
use dfdl_core::io::traits::ByteOrder;
use dfdl_core::schema::ir::{
    BinaryCalendarRep, CompiledElement, CompiledSchema, CompiledSequence, CompiledType,
    LengthKind, NodeId, OccursCountKind, TermKind,
};

use crate::error::{CodegenError, CodegenResult};
use crate::ir::{
    ArrayMode, ChoiceBranchDef, ChoiceDef, CodeGenIr, FieldCodec, FieldDef, RustType, StructDef,
};

/// Traverses a compiled DFDL schema to construct the CodeGen Intermediate Representation.
#[derive(Debug)]
pub struct SchemaAnalyzer<'a> {
    schema: &'a CompiledSchema,
    ir: CodeGenIr,
}

impl<'a> SchemaAnalyzer<'a> {
    /// Creates a new [`SchemaAnalyzer`] for the specified compiled schema.
    #[must_use]
    pub fn new(schema: &'a CompiledSchema) -> Self {
        Self {
            schema,
            ir: CodeGenIr::default(),
        }
    }

    /// Analyzes the schema and extracts the full [`CodeGenIr`].
    ///
    /// # Errors
    /// Returns [`CodegenError`] if invalid references or unsupported schema graphs exist.
    pub fn analyze(mut self) -> CodegenResult<CodeGenIr> {
        let root_term = self
            .schema
            .get_term(self.schema.root_element_id)
            .ok_or_else(|| CodegenError::NodeNotFound {
                node_id: self.schema.root_element_id.0,
            })?;

        let TermKind::Element(root_elem) = &root_term.kind else {
            return Err(CodegenError::InvalidSchema {
                message: "root term must be an element".to_string(),
            });
        };

        let root_type_name = to_pascal_case(&root_elem.name.local_name);
        self.ir.root_type = Some(root_type_name.clone());

        match &root_elem.type_ir {
            CompiledType::Complex(child_id) => {
                self.analyze_complex_entry(&root_elem.name.local_name, *child_id, &root_term.properties)?;
            }
            CompiledType::Simple(_) => {
                return Err(CodegenError::UnsupportedConstruct {
                    feature: "RootSimpleType",
                    details: "AOT code generation requires a complex message type as root".to_string(),
                });
            }
        }

        Ok(self.ir)
    }

    /// Analyzes the top-level complex entry point (either a sequence, choice, or container).
    fn analyze_complex_entry(
        &mut self,
        name: &str,
        child_id: NodeId,
        props: &dfdl_core::schema::ir::ResolvedProperties,
    ) -> CodegenResult<()> {
        let child_term = self.schema.get_term(child_id).ok_or_else(|| CodegenError::NodeNotFound {
            node_id: child_id.0,
        })?;

        match &child_term.kind {
            TermKind::Choice(choice) => {
                let choice_name = to_pascal_case(name);
                self.analyze_choice(&choice_name, choice, props)?;
            }
            TermKind::Sequence(seq) => {
                // Check if this sequence is a wrapper around a single choice or repeated message
                if seq.members.len() == 1 {
                    if let Some(first_id) = seq.members.first() {
                        if let Some(first_term) = self.schema.get_term(*first_id) {
                            if let TermKind::Choice(inner_choice) = &first_term.kind {
                                let choice_name = to_pascal_case(name);
                                self.analyze_choice(&choice_name, inner_choice, props)?;
                                return Ok(());
                            }
                            if let TermKind::Element(inner_elem) = &first_term.kind {
                                if inner_elem.max_occurs.is_none() || inner_elem.max_occurs.unwrap_or(1) > 1 {
                                    // Multi-message container (e.g. fakeTDLFile)
                                    self.analyze_multi_message_container(name, inner_elem, *first_id)?;
                                    return Ok(());
                                }
                            }
                        }
                    }
                }

                // Standard struct sequence
                let struct_def = self.analyze_struct(name, seq, props)?;
                self.ir.structs.push(struct_def);
            }
            TermKind::Element(_) | TermKind::GroupRef(_) => {
                return Err(CodegenError::UnsupportedConstruct {
                    feature: "UnexpectedComplexChild",
                    details: name.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Analyzes a container containing a dynamic or unbounded array of messages (e.g. `fakeTDLFile`).
    fn analyze_multi_message_container(
        &mut self,
        container_name: &str,
        item_elem: &CompiledElement,
        item_term_id: NodeId,
    ) -> CodegenResult<()> {
        let item_term = self.schema.get_term(item_term_id).ok_or_else(|| CodegenError::NodeNotFound {
            node_id: item_term_id.0,
        })?;

        let item_type_name = to_pascal_case(&item_elem.name.local_name);

        // Analyze inner item (usually a choice or message struct)
        if let CompiledType::Complex(item_child_id) = &item_elem.type_ir {
            self.analyze_complex_entry(&item_elem.name.local_name, *item_child_id, &item_term.properties)?;
        }

        let container_struct = StructDef {
            name: to_pascal_case(container_name),
            dfdl_name: container_name.to_string(),
            doc: alloc::format!("Multi-message container for `{container_name}`."),
            explicit_len: None,
            fill_byte: None,
            initiator: None,
            fields: vec![FieldDef {
                rust_name: "messages".to_string(),
                dfdl_name: item_elem.name.local_name.clone(),
                rust_type: RustType::Vec(Box::new(RustType::Struct(item_type_name.clone()))),
                codec: FieldCodec::Array {
                    item_type: Box::new(RustType::Struct(item_type_name)),
                    mode: ArrayMode::UntilEof,
                },
                doc: "Sequence of contained messages.".to_string(),
            }],
        };

        self.ir.structs.push(container_struct);
        Ok(())
    }

    /// Analyzes a choice term and generates a corresponding sum-type enum and child branch structs.
    fn analyze_choice(
        &mut self,
        choice_name: &str,
        choice: &dfdl_core::schema::ir::CompiledChoice,
        parent_props: &dfdl_core::schema::ir::ResolvedProperties,
    ) -> CodegenResult<()> {
        let mut branches = Vec::new();

        for &branch_id in &choice.branches {
            let branch_term = self.schema.get_term(branch_id).ok_or_else(|| CodegenError::NodeNotFound {
                node_id: branch_id.0,
            })?;

            let TermKind::Element(branch_elem) = &branch_term.kind else {
                continue;
            };

            let key_byte = self.get_branch_dispatch_key(branch_term)?;
            let variant_name = to_pascal_case(&branch_elem.name.local_name);
            let payload_type = alloc::format!("{}Message", variant_name);

            // Analyze branch struct
            if let CompiledType::Complex(seq_id) = &branch_elem.type_ir {
                let seq_term = self.schema.get_term(*seq_id).ok_or_else(|| CodegenError::NodeNotFound {
                    node_id: seq_id.0,
                })?;
                if let TermKind::Sequence(seq) = &seq_term.kind {
                    let effective_props = if branch_term.properties.length.is_some() {
                        &branch_term.properties
                    } else if parent_props.length.is_some() {
                        parent_props
                    } else {
                        &branch_term.properties
                    };
                    let mut struct_def = self.analyze_struct(&payload_type, seq, effective_props)?;
                    struct_def.initiator = Some(key_byte);
                    self.ir.structs.push(struct_def);
                }
            }

            branches.push(ChoiceBranchDef {
                variant_name,
                payload_type,
                dispatch_key: key_byte,
                doc: alloc::format!("Branch for element `{}`.", branch_elem.name.local_name),
            });
        }

        self.ir.choices.push(ChoiceDef {
            name: choice_name.to_string(),
            dfdl_name: choice_name.to_string(),
            doc: alloc::format!("Choice sum-type representing `{choice_name}`."),
            branches,
        });

        Ok(())
    }

    /// Extracts the 1-byte dispatch key or initiator for a choice branch.
    fn get_branch_dispatch_key(&self, term: &dfdl_core::schema::ir::CompiledTerm) -> CodegenResult<u8> {
        if let Some(ref key_str) = term.properties.choice_branch_key {
            if let Ok(key) = key_str.parse::<u8>() {
                return Ok(key);
            }
        }
        if let Some(ref init_str) = term.properties.initiator {
            if let Some(first_char) = init_str.chars().next() {
                return Ok(first_char as u8);
            }
        }
        Err(CodegenError::AmbiguousChoiceBranch {
            branch_name: term.name.local_name.clone(),
        })
    }

    /// Analyzes a sequence into a strongly-typed [`StructDef`].
    fn analyze_struct(
        &mut self,
        struct_name: &str,
        seq: &CompiledSequence,
        props: &dfdl_core::schema::ir::ResolvedProperties,
    ) -> CodegenResult<StructDef> {
        let mut flattened_terms = Vec::new();
        for &member_id in &seq.members {
            self.flatten_sequence_terms(member_id, &mut flattened_terms);
        }

        let mut fields = Vec::new();
        for term_id in flattened_terms {
            if let Some(field) = self.analyze_field(term_id)? {
                fields.push(field);
            }
        }

        let explicit_len = if props.length_kind == LengthKind::Explicit {
            props.length
        } else {
            None
        };

        let fill_byte = if props.fill_byte_defined {
            Some(props.fill_byte)
        } else {
            None
        };

        let initiator = props.initiator.as_ref().and_then(|s| s.bytes().next());

        Ok(StructDef {
            name: to_pascal_case(struct_name),
            dfdl_name: struct_name.to_string(),
            doc: alloc::format!("Tactical message struct for `{struct_name}`."),
            explicit_len,
            fill_byte,
            initiator,
            fields,
        })
    }

    /// Recursively flattens sequence members, unwrapping `GroupRef` definitions.
    fn flatten_sequence_terms(&self, term_id: NodeId, out: &mut Vec<NodeId>) {
        let Some(term) = self.schema.get_term(term_id) else {
            return;
        };

        match &term.kind {
            TermKind::GroupRef(target_id) => {
                self.flatten_sequence_terms(*target_id, out);
            }
            TermKind::Sequence(seq) => {
                for &member_id in &seq.members {
                    self.flatten_sequence_terms(member_id, out);
                }
            }
            TermKind::Element(_) | TermKind::Choice(_) => {
                out.push(term_id);
            }
        }
    }

    /// Analyzes an individual schema element term into a [`FieldDef`].
    fn analyze_field(&mut self, term_id: NodeId) -> CodegenResult<Option<FieldDef>> {
        let term = self.schema.get_term(term_id).ok_or_else(|| CodegenError::NodeNotFound {
            node_id: term_id.0,
        })?;

        let TermKind::Element(elem) = &term.kind else {
            return Ok(None);
        };

        let dfdl_name = elem.name.local_name.clone();
        let rust_name = to_snake_case(&dfdl_name);
        let props = &term.properties;

        // Check for occurs / array count
        if props.occurs_count_kind == OccursCountKind::Expression {
            if let Some(ref expr) = props.occurs_count_expr {
                let sibling = extract_sibling_name(expr).unwrap_or("count");
                let item_type = match &elem.type_ir {
                    CompiledType::Simple(st) => map_simple_type_to_rust(*st, props)?,
                    CompiledType::Complex(_) => RustType::Struct(to_pascal_case(&dfdl_name)),
                };
                return Ok(Some(FieldDef {
                    rust_name: rust_name.clone(),
                    dfdl_name,
                    rust_type: RustType::Vec(Box::new(item_type.clone())),
                    codec: FieldCodec::Array {
                        item_type: Box::new(item_type),
                        mode: ArrayMode::CountField {
                            count_field: to_snake_case(sibling),
                            max_items: elem.max_occurs.unwrap_or(12),
                        },
                    },
                    doc: alloc::format!("Array of `{}` items.", elem.name.local_name),
                }));
            }
        }

        // Scalar types
        let (rust_type, codec) = match &elem.type_ir {
            CompiledType::Simple(simple_type) => self.analyze_simple_scalar(*simple_type, props)?,
            CompiledType::Complex(nested_id) => {
                let nested_term = self.schema.get_term(*nested_id).ok_or_else(|| CodegenError::NodeNotFound {
                    node_id: nested_id.0,
                })?;
                let type_name = to_pascal_case(&dfdl_name);
                if let TermKind::Sequence(seq) = &nested_term.kind {
                    let struct_def = self.analyze_struct(&type_name, seq, &term.properties)?;
                    self.ir.structs.push(struct_def);
                }
                (
                    RustType::Struct(type_name.clone()),
                    FieldCodec::NestedStruct { type_name },
                )
            }
        };

        Ok(Some(FieldDef {
            rust_name,
            dfdl_name,
            rust_type,
            codec,
            doc: alloc::format!("Field `{}`.", elem.name.local_name),
        }))
    }

    /// Maps simple scalar types and properties to Rust type and binary codec.
    fn analyze_simple_scalar(
        &mut self,
        st: DfdlSimpleType,
        props: &dfdl_core::schema::ir::ResolvedProperties,
    ) -> CodegenResult<(RustType, FieldCodec)> {
        let is_be = props.byte_order == ByteOrder::BigEndian;

        match st {
            DfdlSimpleType::Byte => Ok((
                RustType::I8,
                FieldCodec::BinaryInt {
                    size: 1,
                    signed: true,
                    is_be,
                },
            )),
            DfdlSimpleType::UnsignedByte => Ok((
                RustType::U8,
                FieldCodec::BinaryInt {
                    size: 1,
                    signed: false,
                    is_be,
                },
            )),
            DfdlSimpleType::Short => Ok((
                RustType::I16,
                FieldCodec::BinaryInt {
                    size: 2,
                    signed: true,
                    is_be,
                },
            )),
            DfdlSimpleType::UnsignedShort => Ok((
                RustType::U16,
                FieldCodec::BinaryInt {
                    size: 2,
                    signed: false,
                    is_be,
                },
            )),
            DfdlSimpleType::Int => Ok((
                RustType::I32,
                FieldCodec::BinaryInt {
                    size: 4,
                    signed: true,
                    is_be,
                },
            )),
            DfdlSimpleType::UnsignedInt => Ok((
                RustType::U32,
                FieldCodec::BinaryInt {
                    size: 4,
                    signed: false,
                    is_be,
                },
            )),
            DfdlSimpleType::Long => Ok((
                RustType::I64,
                FieldCodec::BinaryInt {
                    size: 8,
                    signed: true,
                    is_be,
                },
            )),
            DfdlSimpleType::UnsignedLong => Ok((
                RustType::U64,
                FieldCodec::BinaryInt {
                    size: 8,
                    signed: false,
                    is_be,
                },
            )),
            DfdlSimpleType::Float => Ok((
                RustType::F32,
                FieldCodec::BinaryFloat {
                    size: 4,
                    is_be,
                },
            )),
            DfdlSimpleType::Double => Ok((
                RustType::F64,
                FieldCodec::BinaryFloat {
                    size: 8,
                    is_be,
                },
            )),
            DfdlSimpleType::Boolean => Ok((RustType::Bool, FieldCodec::Bool { size: 1 })),
            DfdlSimpleType::String => {
                let len = props
                    .length
                    .or(props.facets.max_length)
                    .or(props.facets.min_length)
                    .unwrap_or(0);
                if len == 1 {
                    Ok((
                        RustType::U8,
                        FieldCodec::BinaryInt {
                            size: 1,
                            signed: false,
                            is_be: true,
                        },
                    ))
                } else if len > 1 && len <= 64 {
                    Ok((
                        RustType::FixedByteArray(len),
                        FieldCodec::FixedByteArray { size: len },
                    ))
                } else {
                    Ok((
                        RustType::FixedString(len),
                        FieldCodec::FixedString { size: len },
                    ))
                }
            }
            DfdlSimpleType::Time | DfdlSimpleType::DateTime | DfdlSimpleType::Date => {
                if props.binary_calendar_rep == BinaryCalendarRep::Bcd {
                    Ok((RustType::TimeOfDay, FieldCodec::BcdTimeOfDay))
                } else {
                    Ok((
                        RustType::U32,
                        FieldCodec::BinaryInt {
                            size: 4,
                            signed: false,
                            is_be,
                        },
                    ))
                }
            }
            _ => Err(CodegenError::UnsupportedConstruct {
                feature: "UnsupportedScalarType",
                details: alloc::format!("{:?}", st),
            }),
        }
    }
}

/// Maps DFDL simple type to basic RustType.
fn map_simple_type_to_rust(
    st: DfdlSimpleType,
    props: &dfdl_core::schema::ir::ResolvedProperties,
) -> CodegenResult<RustType> {
    match st {
        DfdlSimpleType::Byte => Ok(RustType::I8),
        DfdlSimpleType::UnsignedByte => Ok(RustType::U8),
        DfdlSimpleType::Short => Ok(RustType::I16),
        DfdlSimpleType::UnsignedShort => Ok(RustType::U16),
        DfdlSimpleType::Int => Ok(RustType::I32),
        DfdlSimpleType::UnsignedInt => Ok(RustType::U32),
        DfdlSimpleType::Long => Ok(RustType::I64),
        DfdlSimpleType::UnsignedLong => Ok(RustType::U64),
        DfdlSimpleType::Float => Ok(RustType::F32),
        DfdlSimpleType::Double => Ok(RustType::F64),
        DfdlSimpleType::Boolean => Ok(RustType::Bool),
        DfdlSimpleType::String => {
            let len = props.length.unwrap_or(0);
            if len > 0 {
                Ok(RustType::FixedByteArray(len))
            } else {
                Ok(RustType::FixedString(len))
            }
        }
        DfdlSimpleType::Time | DfdlSimpleType::DateTime => {
            if props.binary_calendar_rep == BinaryCalendarRep::Bcd {
                Ok(RustType::TimeOfDay)
            } else {
                Ok(RustType::U32)
            }
        }
        _ => Err(CodegenError::UnsupportedConstruct {
            feature: "SimpleTypeMapping",
            details: alloc::format!("{:?}", st),
        }),
    }
}

/// Converts an identifier to `snake_case`.
#[must_use]
pub fn to_snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(4));
    let mut prev_is_lower = false;
    for c in s.chars() {
        if c == '-' || c == '_' || c == '.' || c == ':' {
            out.push('_');
            prev_is_lower = false;
        } else if c.is_uppercase() {
            if prev_is_lower {
                out.push('_');
            }
            out.extend(c.to_lowercase());
            prev_is_lower = false;
        } else {
            out.push(c);
            prev_is_lower = true;
        }
    }
    // Prevent Rust reserved keywords
    match out.as_str() {
        "type" => "r#type".to_string(),
        "match" => "r#match".to_string(),
        "move" => "r#move".to_string(),
        "ref" => "r#ref".to_string(),
        "fn" => "r#fn".to_string(),
        "loop" => "r#loop".to_string(),
        _ => out,
    }
}

/// Converts an identifier to `PascalCase`.
#[must_use]
pub fn to_pascal_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '-' || c == '_' || c == '.' || c == ':' {
            capitalize_next = true;
        } else if capitalize_next {
            out.extend(c.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Extracts a sibling field name from an expression like `{ ../count }`.
fn extract_sibling_name(expr: &str) -> Option<&str> {
    let trimmed = expr.trim().trim_start_matches('{').trim_end_matches('}').trim();
    if let Some(pos) = trimmed.rfind('/') {
        let name = trimmed.get(pos.saturating_add(1)..)?.trim();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}
