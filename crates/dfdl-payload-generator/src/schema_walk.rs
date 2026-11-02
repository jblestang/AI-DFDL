//! Schema introspection and graph traversal for exhaustive payload generation.
//!
//! Analyzes compiled DFDL schema terms to discover choice branches, array bounds,
//! facet constraints, framing delimiters, and explicit length bindings without
//! making any assumptions about specific protocols or field names.

use std::collections::{BTreeMap, BTreeSet};
use dfdl_core::infoset::value::DfdlSimpleType;
use dfdl_core::schema::ir::{
    CompiledElement, CompiledSchema, CompiledTerm, CompiledType, LengthKind,
    NodeId, TermKind,
};

/// Extracts the target element local name from an XPath or DFDL expression without schema assumptions.
///
/// Handles expressions such as:
/// - `"{ xs:string(../messageType) }"` -> `"messageType"`
/// - `"{ tdlType }"` -> `"tdlType"`
/// - `"{ /header/type }"` -> `"type"`
/// - `"{ ./flag }"` -> `"flag"`
pub fn extract_target_element_from_expr(expr: &str) -> Option<String> {
    let trimmed = expr.trim();
    let inner = if trimmed.starts_with('{') && trimmed.ends_with('}') {
        &trimmed[1..trimmed.len().saturating_sub(1)]
    } else {
        trimmed
    };

    let unwrapped = if let Some(open_paren) = inner.find('(') {
        if let Some(close_paren) = inner.rfind(')') {
            if close_paren > open_paren {
                &inner[open_paren + 1..close_paren]
            } else {
                inner
            }
        } else {
            inner
        }
    } else {
        inner
    };

    let last_segment = unwrapped
        .split('/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .last()?;

    let ident: String = last_segment
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();

    if ident.is_empty() {
        None
    } else {
        Some(ident)
    }
}

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
    /// Estimated byte offset from payload start.
    pub estimated_offset: usize,
    /// Estimated byte width.
    pub estimated_width: usize,
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
    /// Branch element or sequence name.
    pub branch_name: String,
    /// Dispatch branch key if specified (`dfdl:choiceBranchKey`).
    pub branch_key: Option<String>,
    /// Choice dispatch key expression on the parent choice (`dfdl:choiceDispatchKey`).
    pub parent_dispatch_key_expr: Option<String>,
    /// Generic extracted discriminator element name from dispatch key expression.
    pub discriminator_name: Option<String>,
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
    /// All discovered elements in schema traversal order.
    pub all_elements: Vec<ElementFieldInfo>,
}

impl SchemaAnalysis {
    /// Analyzes a compiled schema starting from its root element.
    pub fn analyze(schema: &CompiledSchema) -> Self {
        let mut analysis = Self::default();
        let mut visited = BTreeSet::new();
        let mut current_offset = 0usize;

        if let Some(root_term) = schema.get_term(schema.root_element_id) {
            analysis.traverse_term(schema, root_term, &mut visited, &mut current_offset);
        }

        analysis
    }

    /// Finds the best candidate for an explicit length field in the schema.
    ///
    /// Checks for explicit length fields or elements with names suggesting length, size, or count.
    pub fn find_length_field(&self) -> Option<&ElementFieldInfo> {
        if let Some(explicit) = self.explicit_length_fields.first() {
            return Some(explicit);
        }
        self.all_elements.iter().find(|e| {
            let lower = e.name.to_lowercase();
            lower.contains("length") || lower.contains("len") || lower.contains("size") || lower.contains("count")
        })
    }

    /// Discovers all unique choice discriminators and their valid branch keys across all choices in the schema.
    pub fn find_choice_discriminators(&self) -> Vec<(String, Vec<String>)> {
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for branch in &self.choice_branches {
            if let Some(ref disc) = branch.discriminator_name {
                let entry = map.entry(disc.clone()).or_default();
                if let Some(ref key) = branch.branch_key {
                    if !entry.contains(key) {
                        entry.push(key.clone());
                    }
                }
            }
        }
        map.into_iter().collect()
    }

    /// Recursively traverses schema terms to collect constraints, offsets, and branches.
    fn traverse_term(
        &mut self,
        schema: &CompiledSchema,
        term: &CompiledTerm,
        visited: &mut BTreeSet<NodeId>,
        current_offset: &mut usize,
    ) {
        if !visited.insert(term.id) {
            return;
        }

        match &term.kind {
            TermKind::Element(elem) => {
                let width = Self::estimate_element_width(term, elem);
                let offset = *current_offset;
                if width > 0 {
                    *current_offset = current_offset.saturating_add(width);
                }

                let info = Self::extract_element_info(term, elem, offset, width);
                self.all_elements.push(info.clone());

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
                        self.traverse_term(schema, child_term, visited, current_offset);
                    }
                }
            }
            TermKind::Sequence(seq) => {
                for &member_id in &seq.members {
                    if let Some(member) = schema.get_term(member_id) {
                        self.traverse_term(schema, member, visited, current_offset);
                    }
                }
            }
            TermKind::Choice(choice) => {
                let dispatch_expr = term.properties.choice_dispatch_key.clone();
                let disc_name = dispatch_expr
                    .as_deref()
                    .and_then(extract_target_element_from_expr);

                for (idx, &branch_id) in choice.branches.iter().enumerate() {
                    let (branch_key, branch_name) = if let Some(b_term) = schema.get_term(branch_id) {
                        let name = match b_term.kind {
                            TermKind::Element(ref e) => e.name.local_name.clone(),
                            _ => format!("branch_{}", idx),
                        };
                        (b_term.properties.choice_branch_key.clone(), name)
                    } else {
                        (None, format!("branch_{}", idx))
                    };

                    self.choice_branches.push(ChoiceBranchTarget {
                        choice_id: term.id,
                        branch_index: idx,
                        branch_term_id: branch_id,
                        branch_name,
                        branch_key,
                        parent_dispatch_key_expr: dispatch_expr.clone(),
                        discriminator_name: disc_name.clone(),
                    });

                    if let Some(branch_term) = schema.get_term(branch_id) {
                        self.traverse_term(schema, branch_term, visited, current_offset);
                    }
                }
            }
            TermKind::GroupRef(target) => {
                if let Some(target_term) = schema.get_term(*target) {
                    self.traverse_term(schema, target_term, visited, current_offset);
                }
            }
        }
    }

    /// Estimates element byte width from simple type or explicit length.
    fn estimate_element_width(term: &CompiledTerm, elem: &CompiledElement) -> usize {
        if let Some(len) = term.properties.length {
            return len;
        }
        match elem.type_ir {
            CompiledType::Simple(simple_type) => match simple_type {
                DfdlSimpleType::Boolean | DfdlSimpleType::Byte | DfdlSimpleType::UnsignedByte => 1,
                DfdlSimpleType::Short | DfdlSimpleType::UnsignedShort => 2,
                DfdlSimpleType::Int | DfdlSimpleType::UnsignedInt | DfdlSimpleType::Float => 4,
                DfdlSimpleType::Long | DfdlSimpleType::UnsignedLong | DfdlSimpleType::Double => 8,
                DfdlSimpleType::Decimal => 8,
                DfdlSimpleType::String | DfdlSimpleType::HexBinary => 4,
                DfdlSimpleType::Time | DfdlSimpleType::Date | DfdlSimpleType::DateTime => 8,
            },
            CompiledType::Complex(_) => 0,
        }
    }

    /// Extracts facet and framing constraints for an element term.
    fn extract_element_info(
        term: &CompiledTerm,
        elem: &CompiledElement,
        estimated_offset: usize,
        estimated_width: usize,
    ) -> ElementFieldInfo {
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
            estimated_offset,
            estimated_width,
        }
    }
}
