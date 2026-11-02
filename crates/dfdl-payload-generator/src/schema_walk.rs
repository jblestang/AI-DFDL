//! Schema introspection and graph traversal for exhaustive payload generation.
//!
//! Analyzes compiled DFDL schema terms to discover choice branches, array bounds,
//! facet constraints, framing delimiters, and explicit length bindings.

use std::collections::BTreeSet;
use dfdl_core::schema::ir::{
    CompiledElement, CompiledSchema, CompiledTerm, CompiledType, LengthKind,
    NodeId, TermKind,
};

/// Summary of a schema element's properties and constraints.
#[derive(Debug, Clone)]
pub struct ElementFieldInfo {
    /// Schema graph NodeId.
    pub node_id: NodeId,
    /// Local XML element name.
    pub name: String,
    /// Element simple or complex type.
    pub element_type: CompiledType,
    /// Minimum allowed occurrences.
    pub min_occurs: usize,
    /// Maximum allowed occurrences.
    pub max_occurs: Option<usize>,
    /// Minimum numerical or lexical facet bound.
    pub min_inclusive: Option<String>,
    /// Maximum numerical or lexical facet bound.
    pub max_inclusive: Option<String>,
    /// Discrete enumeration facet values.
    pub enumerations: Vec<String>,
    /// Static explicit length if known.
    pub length: Option<usize>,
    /// Dynamic length expression if present.
    pub length_expr: Option<String>,
    /// Length specification kind.
    pub length_kind: LengthKind,
    /// Framing initiator sequence.
    pub initiator: Option<String>,
    /// Framing terminator sequence.
    pub terminator: Option<String>,
}

/// Metadata about a choice group branch for targeted dispatch testing.
#[derive(Debug, Clone)]
pub struct ChoiceBranchTarget {
    /// NodeId of the choice group.
    pub choice_id: NodeId,
    /// 0-based branch index within the choice.
    pub branch_index: usize,
    /// NodeId of the branch term.
    pub branch_term_id: NodeId,
    /// Dispatch branch key if specified (`dfdl:choiceBranchKey`).
    pub branch_key: Option<String>,
    /// Choice dispatch key expression on the parent choice (`dfdl:choiceDispatchKey`).
    pub parent_dispatch_key_expr: Option<String>,
}

/// Discovered metadata across a compiled DFDL schema for exhaustive coverage.
#[derive(Debug, Clone, Default)]
pub struct SchemaAnalysis {
    /// All discovered choice branches in the schema.
    pub choice_branches: Vec<ChoiceBranchTarget>,
    /// All elements with enumeration facets.
    pub enum_fields: Vec<ElementFieldInfo>,
    /// All elements with min/max range constraints.
    pub bounded_fields: Vec<ElementFieldInfo>,
    /// All elements with explicit length constraints.
    pub explicit_length_fields: Vec<ElementFieldInfo>,
}

impl SchemaAnalysis {
    /// Analyzes a compiled schema starting from its root element.
    pub fn analyze(schema: &CompiledSchema) -> Self {
        let mut analysis = Self::default();
        let mut visited = BTreeSet::new();

        if let Some(root_term) = schema.get_term(schema.root_element_id) {
            analysis.traverse_term(schema, root_term, &mut visited);
        }

        analysis
    }

    /// Recursively traverses schema terms to collect constraints and branches.
    fn traverse_term(
        &mut self,
        schema: &CompiledSchema,
        term: &CompiledTerm,
        visited: &mut BTreeSet<NodeId>,
    ) {
        if !visited.insert(term.id) {
            return;
        }

        match &term.kind {
            TermKind::Element(elem) => {
                let info = Self::extract_element_info(term, elem);
                if !info.enumerations.is_empty() {
                    self.enum_fields.push(info.clone());
                }
                if info.min_inclusive.is_some() || info.max_inclusive.is_some() {
                    self.bounded_fields.push(info.clone());
                }
                if info.length_kind == LengthKind::Explicit {
                    self.explicit_length_fields.push(info.clone());
                }

                if let CompiledType::Complex(child_id) = elem.type_ir {
                    if let Some(child_term) = schema.get_term(child_id) {
                        self.traverse_term(schema, child_term, visited);
                    }
                }
            }
            TermKind::Sequence(seq) => {
                for &member_id in &seq.members {
                    if let Some(member) = schema.get_term(member_id) {
                        self.traverse_term(schema, member, visited);
                    }
                }
            }
            TermKind::Choice(choice) => {
                for (idx, &branch_id) in choice.branches.iter().enumerate() {
                    let branch_key = if let Some(b_term) = schema.get_term(branch_id) {
                        b_term.properties.choice_branch_key.clone()
                    } else {
                        None
                    };

                    self.choice_branches.push(ChoiceBranchTarget {
                        choice_id: term.id,
                        branch_index: idx,
                        branch_term_id: branch_id,
                        branch_key,
                        parent_dispatch_key_expr: term.properties.choice_dispatch_key.clone(),
                    });

                    if let Some(branch_term) = schema.get_term(branch_id) {
                        self.traverse_term(schema, branch_term, visited);
                    }
                }
            }
            TermKind::GroupRef(target) => {
                if let Some(target_term) = schema.get_term(*target) {
                    self.traverse_term(schema, target_term, visited);
                }
            }
        }
    }

    /// Extracts facet and framing constraints for an element term.
    fn extract_element_info(term: &CompiledTerm, elem: &CompiledElement) -> ElementFieldInfo {
        ElementFieldInfo {
            node_id: term.id,
            name: elem.name.local_name.clone(),
            element_type: elem.type_ir.clone(),
            min_occurs: elem.min_occurs,
            max_occurs: elem.max_occurs,
            min_inclusive: term.properties.facets.min_inclusive.clone(),
            max_inclusive: term.properties.facets.max_inclusive.clone(),
            enumerations: term.properties.facets.enumeration.clone(),
            length: term.properties.length,
            length_expr: term.properties.length_expr.clone(),
            length_kind: term.properties.length_kind,
            initiator: term.properties.initiator.clone(),
            terminator: term.properties.terminator.clone(),
        }
    }
}
