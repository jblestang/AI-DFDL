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
        properties: crate::schema::ir::ResolvedProperties,
    ) -> DFDLResult<NodeId> {
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
#[allow(clippy::unwrap_used)]
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
}
