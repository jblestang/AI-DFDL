//! Schema IR Builder and Schema Graph Validation.
//!
//! Provides [`SchemaBuilder`] for programmatically building compiled schemas without XML,
//! and [`validate_schema_ir`] to detect invalid references, property conflicts, and cycle loops.

extern crate alloc;
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::schema::ir::{CompiledSchema, CompiledTerm, LengthKind, NodeId, TermKind};
use crate::types::QName;
use crate::util::try_push;

/// Programmatic builder for [`CompiledSchema`] intermediate representations.
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    next_id: u32,
    terms: Vec<CompiledTerm>,
    root_id: Option<NodeId>,
    /// Schema variable map.
    pub variable_map: crate::expr::variables::VariableMap,
    /// Policy for resolving unqualified path steps in expressions (§23).
    pub unqualified_path_step_policy: crate::types::UnqualifiedPathStepPolicy,
    /// Tunable maxHexBinaryLengthInBytes limit.
    pub max_hex_binary_length_in_bytes: Option<usize>,
}

impl SchemaBuilder {
    /// Constructs a new [`SchemaBuilder`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 0,
            terms: Vec::new(),
            root_id: None,
            variable_map: crate::expr::variables::VariableMap::new(),
            unqualified_path_step_policy: Default::default(),
            max_hex_binary_length_in_bytes: None,
        }
    }

    /// Defines a DFDL variable in the schema builder (§7).
    pub fn define_variable(
        &mut self,
        name: QName,
        var_type: crate::infoset::value::DfdlSimpleType,
        default_value: Option<crate::infoset::value::DfdlValue>,
    ) {
        self.variable_map
            .define_variable(name, var_type, default_value);
    }

    /// Defines a variable with direction in the variable map.
    pub fn define_variable_with_direction(
        &mut self,
        name: QName,
        var_type: crate::infoset::value::DfdlSimpleType,
        default_value: Option<crate::infoset::value::DfdlValue>,
        direction: crate::expr::variables::VariableDirection,
    ) {
        self.variable_map
            .define_variable_with_direction(name, var_type, default_value, direction);
    }

    /// Allocates a new unique [`NodeId`].
    fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Adds a compiled term to the schema graph builder.
    pub fn add_term(&mut self, name: QName, kind: TermKind) -> DFDLResult<NodeId> {
        let id = self.alloc_id();
        let term = CompiledTerm {
            id,
            name,
            kind,
            properties: Default::default(),
        };

        if self.root_id.is_none() {
            self.root_id = Some(id);
        }

        try_push(&mut self.terms, term)?;
        Ok(id)
    }

    /// Adds a compiled term with explicit resolved properties to the schema graph builder.
    pub fn add_term_with_props(
        &mut self,
        name: QName,
        kind: TermKind,
        mut properties: crate::schema::ir::ResolvedProperties,
    ) -> DFDLResult<NodeId> {
        if properties.encoding_prop.is_constant() && properties.encoding.starts_with('{') {
            if let Ok(p) = crate::schema::ir::DfdlProp::parse_str(&properties.encoding) {
                properties.encoding_prop = p;
            }
        }
        if properties.byte_order_prop.is_constant() {
            if let Some(ref bo_expr) = properties.byte_order_expr {
                if let Ok(p) = crate::schema::ir::DfdlProp::parse_with(bo_expr, crate::expr::properties::parse_byte_order) {
                    properties.byte_order_prop = p;
                }
            }
        }
        if properties.initiator_prop.is_none() {
            if let Some(ref init) = properties.initiator {
                if !init.is_empty() {
                    properties.initiator_prop = crate::schema::ir::DfdlProp::parse_str(init).ok();
                }
            }
        }
        if properties.terminator_prop.is_none() {
            if let Some(ref term) = properties.terminator {
                if !term.is_empty() {
                    properties.terminator_prop = crate::schema::ir::DfdlProp::parse_str(term).ok();
                }
            }
        }
        if properties.separator_prop.is_none() {
            if let Some(ref sep) = properties.separator {
                if !sep.is_empty() {
                    properties.separator_prop = crate::schema::ir::DfdlProp::parse_str(sep).ok();
                }
            }
        }
        if properties.length_prop.is_none() {
            if let Some(ref expr) = properties.length_expr {
                properties.length_prop = crate::schema::ir::DfdlProp::parse_with(expr, |raw| {
                    raw.trim().parse::<usize>().map_err(|e| {
                        crate::error::DFDLError::new(
                            crate::error::DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Invalid length constant '{}': {}", raw, e),
                        )
                    })
                }).ok();
            } else if let Some(len) = properties.length {
                properties.length_prop = Some(crate::schema::ir::DfdlProp::constant(len));
            }
        }
        if properties.occurs_count_prop.is_none() {
            if let Some(ref expr) = properties.occurs_count_expr {
                properties.occurs_count_prop = crate::schema::ir::DfdlProp::parse_with(expr, |raw| {
                    raw.trim().parse::<usize>().map_err(|e| {
                        crate::error::DFDLError::new(
                            crate::error::DFDLErrorKind::SchemaDefinition,
                            &alloc::format!("Invalid occursCount constant '{}': {}", raw, e),
                        )
                    })
                }).ok();
            }
        }
        if properties.nil_value_prop.is_none() {
            if let Some(ref nv) = properties.nil_value {
                if !nv.is_empty() {
                    properties.nil_value_prop = crate::schema::ir::DfdlProp::parse_str(nv).ok();
                }
            }
        }
        if properties.text_standard_decimal_separator_prop.is_constant()
            && properties.text_standard_decimal_separator.starts_with('{')
        {
            if let Ok(p) = crate::schema::ir::DfdlProp::parse_str(&properties.text_standard_decimal_separator) {
                properties.text_standard_decimal_separator_prop = p;
            }
        }
        if properties.text_standard_grouping_separator_prop.is_constant()
            && properties.text_standard_grouping_separator.starts_with('{')
        {
            if let Ok(p) = crate::schema::ir::DfdlProp::parse_str(&properties.text_standard_grouping_separator) {
                properties.text_standard_grouping_separator_prop = p;
            }
        }
        if properties.calendar_pattern_prop.is_none() {
            if let Some(ref cp) = properties.calendar_pattern {
                if !cp.is_empty() {
                    properties.calendar_pattern_prop = crate::schema::ir::DfdlProp::parse_str(cp).ok();
                }
            }
        }
        if properties.calendar_language_prop.is_none() {
            if let Some(ref cl) = properties.calendar_language {
                if !cl.is_empty() {
                    properties.calendar_language_prop = crate::schema::ir::DfdlProp::parse_str(cl).ok();
                }
            }
        }
        if properties.calendar_time_zone_prop.is_none() {
            if let Some(ref ctz) = properties.calendar_time_zone {
                if !ctz.is_empty() {
                    properties.calendar_time_zone_prop = crate::schema::ir::DfdlProp::parse_str(ctz).ok();
                }
            }
        }
        if properties.output_new_line_prop.is_none() {
            if let Some(ref onl) = properties.output_new_line {
                if !onl.is_empty() {
                    properties.output_new_line_prop = crate::schema::ir::DfdlProp::parse_str(onl).ok();
                }
            }
        }
        if properties.text_standard_exponent_rep_prop.is_none() {
            if let Some(ref exp) = properties.text_standard_exponent_rep {
                if !exp.is_empty() {
                    properties.text_standard_exponent_rep_prop = crate::schema::ir::DfdlProp::parse_str(exp).ok();
                }
            }
        }
        if properties.text_boolean_true_rep_prop.is_none() {
            if let Some(ref tr) = properties.text_boolean_true_rep {
                if !tr.is_empty() {
                    properties.text_boolean_true_rep_prop = crate::schema::ir::DfdlProp::parse_str(tr).ok();
                }
            }
        }
        if properties.text_boolean_false_rep_prop.is_none() {
            if let Some(ref fr) = properties.text_boolean_false_rep {
                if !fr.is_empty() {
                    properties.text_boolean_false_rep_prop = crate::schema::ir::DfdlProp::parse_str(fr).ok();
                }
            }
        }
        let id = self.alloc_id();
        let term = CompiledTerm {
            id,
            name,
            kind,
            properties,
        };

        if self.root_id.is_none() {
            self.root_id = Some(id);
        }

        try_push(&mut self.terms, term)?;
        Ok(id)
    }

    /// Returns a reference to the resolved properties of a term by ID.
    #[must_use]
    pub fn get_term_props(&self, id: NodeId) -> Option<&crate::schema::ir::ResolvedProperties> {
        self.terms
            .iter()
            .find(|t| t.id == id)
            .map(|t| &t.properties)
    }

    /// Returns a reference to a compiled term by ID.
    #[must_use]
    pub fn get_term(&self, id: NodeId) -> Option<&CompiledTerm> {
        self.terms.iter().find(|t| t.id == id)
    }

    /// Sets the distinguished root element [`NodeId`].
    #[inline]
    pub fn set_root(&mut self, root_id: NodeId) {
        self.root_id = Some(root_id);
    }

    /// Builds and validates the [`CompiledSchema`].
    pub fn build(self) -> DFDLResult<CompiledSchema> {
        let root_element_id = self.root_id.ok_or_else(|| {
            DFDLError::new(
                DFDLErrorKind::SchemaDefinition,
                "Cannot build schema IR without a root element",
            )
        })?;

        let schema = CompiledSchema {
            root_element_id,
            terms: self.terms,
            variable_map: self.variable_map,
            disallow_signed_integer_length_1bit: false,
            max_occurs_bounds: None,
            unqualified_path_step_policy: self.unqualified_path_step_policy,
            max_hex_binary_length_in_bytes: self.max_hex_binary_length_in_bytes,
        };

        validate_schema_ir(&schema)?;
        Ok(schema)
    }
}

/// Validates a [`CompiledSchema`] graph for missing references, invalid property sets, and cycle loops.
pub fn validate_schema_ir(schema: &CompiledSchema) -> DFDLResult<()> {
    // 1. Verify root element exists
    if schema.get_term(schema.root_element_id).is_none() {
        return Err(DFDLError::new(
            DFDLErrorKind::SchemaDefinition,
            "Schema root_element_id refers to missing term in IR graph",
        ));
    }

    // 2. Validate all term references and properties
    for term in &schema.terms {
        // Validate explicit length property consistency
        if term.properties.length_kind == LengthKind::Explicit && term.properties.length.is_none() {
            // Checked during execution or compilation if dynamically defined
        }

        match term.kind {
            TermKind::Element(ref elem) => {
                if elem.min_occurs > elem.max_occurs.unwrap_or(usize::MAX) {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "minOccurs cannot be greater than maxOccurs",
                    ));
                }
                if term.properties.length_kind == LengthKind::Pattern {
                    if let crate::schema::ir::CompiledType::Complex(complex_id) = elem.type_ir {
                        let mut visited_descendants = Vec::new();
                        check_pattern_complex_descendants(schema, complex_id, &mut visited_descendants)?;
                    }
                }
            }
            TermKind::Sequence(ref seq) => {
                for member_id in &seq.members {
                    if schema.get_term(*member_id).is_none() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            "Sequence member NodeId reference does not exist in schema graph",
                        ));
                    }
                }
            }
            TermKind::Choice(ref choice) => {
                for branch_id in &choice.branches {
                    if schema.get_term(*branch_id).is_none() {
                        return Err(DFDLError::new(
                            DFDLErrorKind::SchemaDefinition,
                            "Choice branch NodeId reference does not exist in schema graph",
                        ));
                    }
                }
            }
            TermKind::GroupRef(target_id) => {
                if schema.get_term(target_id).is_none() {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        "GroupRef target NodeId reference does not exist in schema graph",
                    ));
                }
            }
        }
    }

    // 3. Cycle detection in group references
    let mut visited = Vec::new();
    detect_reference_cycles(schema, schema.root_element_id, &mut visited)?;

    Ok(())
}

fn detect_reference_cycles(
    schema: &CompiledSchema,
    current_id: NodeId,
    visited: &mut Vec<NodeId>,
) -> DFDLResult<()> {
    if visited.contains(&current_id) {
        return Err(DFDLError::new(
            DFDLErrorKind::SchemaDefinition,
            "Recursive cycle detected in schema graph references",
        ));
    }

    try_push(visited, current_id)?;

    if let Some(term) = schema.get_term(current_id) {
        match term.kind {
            TermKind::Sequence(ref seq) => {
                for &child_id in &seq.members {
                    detect_reference_cycles(schema, child_id, visited)?;
                }
            }
            TermKind::Choice(ref choice) => {
                for &branch_id in &choice.branches {
                    detect_reference_cycles(schema, branch_id, visited)?;
                }
            }
            TermKind::GroupRef(target_id) => {
                detect_reference_cycles(schema, target_id, visited)?;
            }
            TermKind::Element(ref elem) => {
                if let crate::schema::ir::CompiledType::Complex(complex_id) = elem.type_ir {
                    detect_reference_cycles(schema, complex_id, visited)?;
                }
            }
        }
    }

    visited.pop();
    Ok(())
}

fn check_pattern_complex_descendants(
    schema: &CompiledSchema,
    current_id: NodeId,
    visited: &mut Vec<NodeId>,
) -> DFDLResult<()> {
    if visited.contains(&current_id) {
        return Ok(());
    }
    try_push(visited, current_id)?;
    if let Some(term) = schema.get_term(current_id) {
        match term.kind {
            TermKind::Element(ref elem) => {
                if term.properties.representation != crate::schema::ir::Representation::Text {
                    let msg = alloc::format!(
                        "Schema Definition Error: Elements of complex type with dfdl:lengthKind='pattern' must have child content with representation 'text' (DFDL-12-088R), but element '{}' has representation='binary'",
                        elem.name.local_name
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                if let crate::schema::ir::CompiledType::Complex(child_complex_id) = elem.type_ir {
                    check_pattern_complex_descendants(schema, child_complex_id, visited)?;
                }
            }
            TermKind::Sequence(ref seq) => {
                for &child_id in &seq.members {
                    check_pattern_complex_descendants(schema, child_id, visited)?;
                }
            }
            TermKind::Choice(ref choice) => {
                for &branch_id in &choice.branches {
                    check_pattern_complex_descendants(schema, branch_id, visited)?;
                }
            }
            TermKind::GroupRef(target_id) => {
                check_pattern_complex_descendants(schema, target_id, visited)?;
            }
        }
    }
    visited.pop();
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::field_reassign_with_default,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::infoset::value::DfdlSimpleType;
    use crate::schema::ir::{CompiledElement, CompiledSequence, CompiledType};

    #[test]
    fn test_schema_builder_and_validation() {
        let mut builder = SchemaBuilder::new();
        let elem = CompiledElement {
            name: QName::local("root"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };

        let id = builder
            .add_term(QName::local("root"), TermKind::Element(elem))
            .unwrap();
        builder.set_root(id);

        let schema = builder.build().unwrap();
        assert_eq!(schema.root_element_id, id);
    }

    #[test]
    fn test_invalid_reference_rejection() {
        let mut builder = SchemaBuilder::new();
        let seq = CompiledSequence {
            members: Vec::from([NodeId(999)]), // Non-existent ID
        };

        let id = builder
            .add_term(QName::local("seq"), TermKind::Sequence(seq))
            .unwrap();
        builder.set_root(id);

        assert!(builder.build().is_err());
    }

    /// Verifies variable declarations, accessors, and graph validation constraints.
    #[test]
    fn test_schema_builder_and_validation_error_branches() {
        use crate::schema::ir::{CompiledChoice, Representation, ResolvedProperties};

        let mut b = SchemaBuilder::new();
        b.define_variable(
            QName::local("v1"),
            DfdlSimpleType::String,
            Some(crate::infoset::value::DfdlValue::String("val".into())),
        );
        b.define_variable_with_direction(
            QName::local("v2"),
            DfdlSimpleType::Int,
            None,
            crate::expr::variables::VariableDirection::ParseOnly,
        );

        let elem_valid = CompiledElement {
            name: QName::local("e1"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let e1_id = b.add_term_with_props(
            QName::local("e1"),
            TermKind::Element(elem_valid),
            ResolvedProperties::default(),
        ).unwrap();

        assert!(b.get_term(e1_id).is_some());
        assert!(b.get_term_props(e1_id).is_some());

        // Test minOccurs > maxOccurs rejection
        let mut b_min_max = SchemaBuilder::new();
        let elem_invalid = CompiledElement {
            name: QName::local("badOccurs"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 5,
            max_occurs: Some(2),
            is_nillable: false,
            default_value: None,
        };
        let bad_id = b_min_max.add_term(QName::local("badOccurs"), TermKind::Element(elem_invalid)).unwrap();
        b_min_max.set_root(bad_id);
        assert!(b_min_max.build().is_err());

        // Test non-existent choice branch
        let mut b_choice = SchemaBuilder::new();
        let choice = CompiledChoice {
            branches: alloc::vec![NodeId(888)],
        };
        let c_id = b_choice.add_term(QName::local("choice"), TermKind::Choice(choice)).unwrap();
        b_choice.set_root(c_id);
        assert!(b_choice.build().is_err());

        // Test non-existent group ref target
        let mut b_grp = SchemaBuilder::new();
        let g_id = b_grp.add_term(QName::local("grp"), TermKind::GroupRef(NodeId(777))).unwrap();
        b_grp.set_root(g_id);
        assert!(b_grp.build().is_err());

        // Test pattern complex with binary child rejection (DFDL-12-088R)
        let mut b_pat = SchemaBuilder::new();
        let mut bin_props = ResolvedProperties::default();
        bin_props.representation = Representation::Binary;
        let bin_elem = CompiledElement {
            name: QName::local("binChild"),
            type_ir: CompiledType::Simple(DfdlSimpleType::Int),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let bin_id = b_pat.add_term_with_props(QName::local("binChild"), TermKind::Element(bin_elem), bin_props).unwrap();

        let seq_pat = CompiledSequence {
            members: alloc::vec![bin_id],
        };
        let seq_id = b_pat.add_term(QName::local("patSeq"), TermKind::Sequence(seq_pat)).unwrap();

        let mut pat_props = ResolvedProperties::default();
        pat_props.length_kind = LengthKind::Pattern;
        let pat_elem = CompiledElement {
            name: QName::local("patRoot"),
            type_ir: CompiledType::Complex(seq_id),
            min_occurs: 1,
            max_occurs: Some(1),
            is_nillable: false,
            default_value: None,
        };
        let pat_root_id = b_pat.add_term_with_props(QName::local("patRoot"), TermKind::Element(pat_elem), pat_props).unwrap();
        b_pat.set_root(pat_root_id);
        assert!(b_pat.build().is_err());

        // Test build without root element
        let b_no_root = SchemaBuilder::new();
        assert!(b_no_root.build().is_err());

        // Test root ID referring to missing term
        let mut b_missing_root = SchemaBuilder::new();
        b_missing_root.set_root(NodeId(9999));
        assert!(b_missing_root.build().is_err());

        // Test cycle detection in schema graph
        let mut b_cycle = SchemaBuilder::new();
        let s1_id = b_cycle.add_term(QName::local("s1"), TermKind::Sequence(CompiledSequence { members: alloc::vec![NodeId(1)] })).unwrap();
        let _s2_id = b_cycle.add_term(QName::local("s2"), TermKind::Sequence(CompiledSequence { members: alloc::vec![s1_id] })).unwrap();
        b_cycle.set_root(s1_id);
        assert!(b_cycle.build().is_err());
    }
}
