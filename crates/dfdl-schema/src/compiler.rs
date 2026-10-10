//! Main Schema Compiler implementation transforming XSD XML into CompiledSchema IR.
//!
//! Conforms strictly to DFDL 1.0 Specification §4, §5, §6, §7. Panic-free  + .

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::schema::ir::CompiledSchema;
use dfdl_core::types::UnqualifiedPathStepPolicy;
use dfdl_xml::limits::XmlReaderLimits;
use dfdl_xml::XmlReader;


pub mod annotations;
pub mod elements;
pub mod groups;
pub mod lower;
pub mod schema_doc;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

pub(crate) use self::elements::{
    parse_and_validate_enumeration_rep_attributes, validate_simple_type_facets, RepRangeOrVal,
};

/// Policy for handling invalid facet restrictions during schema compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum InvalidRestrictionPolicy {
    /// Allow invalid restrictions and validate them against the infoset.
    #[default]
    Validate,
    /// Treat invalid facet restrictions as schema definition errors.
    Error,
    /// Ignore invalid facet restrictions without applying them during validation.
    Ignore,
}

/// Validates facet applicability and values against an XSD simple type (DFDL 1.0 & XSD Part 2 §4.3).

#[derive(Debug, Default)]
pub struct SchemaCompiler {
    limits: XmlReaderLimits,
    /// Inverse of the `allowSignedIntegerLength1Bit` tunable (default `false` = allowed).
    disallow_signed_integer_length_1bit: bool,
    /// `maxOccursBounds` tunable (`None` = unlimited).
    max_occurs_bounds: Option<usize>,
    /// `requireEncodingErrorPolicyProperty` tunable: text elements must define the property.
    require_encoding_error_policy: bool,
    /// `requireTextBidiProperty` tunable: text elements must define `dfdl:textBidi`.
    require_text_bidi: bool,
    /// `requireFloatingProperty` tunable: text elements must define `dfdl:floating`.
    require_floating: bool,
    /// Whether to escalate schema definition warnings to errors (`daf:escalateWarningsToErrors`).
    pub escalate_warnings: bool,
    /// Policy for resolving unqualified path steps in expressions (§23).
    pub unqualified_path_step_policy: UnqualifiedPathStepPolicy,
    /// Maximum allowed byte length for xs:hexBinary values.
    pub max_hex_binary_length_in_bytes: Option<usize>,
    /// Policy for handling invalid facet restrictions (`daf:invalidRestrictionPolicy`).
    pub invalid_restriction_policy: InvalidRestrictionPolicy,
    /// Inverse of the `allowExpressionResultCoercion` tunable (default `false` = allowed).
    pub disallow_expression_result_coercion: bool,
    /// Inverse of the `check_delimiter_encoding` flag (default `false` = checked).
    pub disallow_delimiter_encoding_check: bool,
}

impl SchemaCompiler {
    /// Constructs a new [`SchemaCompiler`] with default limits.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: XmlReaderLimits::default(),
            disallow_signed_integer_length_1bit: false,
            max_occurs_bounds: None,
            require_encoding_error_policy: false,
            require_text_bidi: false,
            require_floating: false,
            escalate_warnings: false,
            unqualified_path_step_policy: Default::default(),
            max_hex_binary_length_in_bytes: None,
            invalid_restriction_policy: InvalidRestrictionPolicy::Validate,
            disallow_expression_result_coercion: false,
            disallow_delimiter_encoding_check: false,
        }
    }

    /// Constructs a [`SchemaCompiler`] with custom XML reader limits.
    #[inline]
    #[must_use]
    pub const fn with_limits(limits: XmlReaderLimits) -> Self {
        Self {
            limits,
            disallow_signed_integer_length_1bit: false,
            max_occurs_bounds: None,
            require_encoding_error_policy: false,
            require_text_bidi: false,
            require_floating: false,
            escalate_warnings: false,
            unqualified_path_step_policy: UnqualifiedPathStepPolicy::NoNamespace,
            max_hex_binary_length_in_bytes: None,
            invalid_restriction_policy: InvalidRestrictionPolicy::Validate,
            disallow_expression_result_coercion: false,
            disallow_delimiter_encoding_check: false,
        }
    }

    /// Sets whether to escalate warnings to errors (`daf:escalateWarningsToErrors`).
    #[inline]
    #[must_use]
    pub const fn with_escalate_warnings(mut self, escalate: bool) -> Self {
        self.escalate_warnings = escalate;
        self
    }

    /// Sets the policy for resolving unqualified path steps in DFDL expressions (§23).
    #[inline]
    #[must_use]
    pub const fn with_unqualified_path_step_policy(mut self, policy: UnqualifiedPathStepPolicy) -> Self {
        self.unqualified_path_step_policy = policy;
        self
    }

    /// Sets the `maxHexBinaryLengthInBytes` tunable: maximum allowed byte length for xs:hexBinary values.
    #[inline]
    #[must_use]
    pub const fn with_max_hex_binary_length_in_bytes(mut self, limit: Option<usize>) -> Self {
        self.max_hex_binary_length_in_bytes = limit;
        self
    }

    /// Sets the `invalidRestrictionPolicy` tunable.
    #[inline]
    #[must_use]
    pub const fn with_invalid_restriction_policy(mut self, policy: InvalidRestrictionPolicy) -> Self {
        self.invalid_restriction_policy = policy;
        self
    }

    /// Sets the `allowSignedIntegerLength1Bit` tunable. When `allow` is `false`, signed binary
    /// integers of length 1 bit are rejected with a schema definition error.
    #[inline]
    #[must_use]
    pub const fn with_allow_signed_integer_length_1bit(mut self, allow: bool) -> Self {
        self.disallow_signed_integer_length_1bit = !allow;
        self
    }

    /// Sets the `allowExpressionResultCoercion` tunable. When `false`, automatic type
    /// coercion in DFDL expression results is disallowed.
    #[inline]
    #[must_use]
    pub const fn with_allow_expression_result_coercion(mut self, allow: bool) -> Self {
        self.disallow_expression_result_coercion = !allow;
        self
    }

    /// Returns whether expression result coercion is allowed (`daf:allowExpressionResultCoercion`).
    #[inline]
    #[must_use]
    pub const fn allow_expression_result_coercion(&self) -> bool {
        !self.disallow_expression_result_coercion
    }

    /// Sets whether to validate delimiter encoding compatibility (§11.1).
    ///
    /// DFDL §11.1 requires delimiter and element content encodings to match for parser delimiter
    /// scanning. During unparsing, elements format their own content directly and delimiters
    /// are emitted independently, so this check may be bypassed when compiling schemas strictly
    /// for unparsing.
    #[inline]
    #[must_use]
    pub const fn with_check_delimiter_encoding(mut self, check: bool) -> Self {
        self.disallow_delimiter_encoding_check = !check;
        self
    }

    /// Sets the `maxOccursBounds` tunable: parsing more array occurrences than `bound` is an error.
    #[inline]
    #[must_use]
    pub const fn with_max_occurs_bounds(mut self, bound: Option<usize>) -> Self {
        self.max_occurs_bounds = bound;
        self
    }

    /// Sets the `requireEncodingErrorPolicyProperty` tunable: a text simple element whose
    /// `dfdl:encodingErrorPolicy` is not defined is a schema definition error.
    #[inline]
    #[must_use]
    pub const fn with_require_encoding_error_policy(mut self, require: bool) -> Self {
        self.require_encoding_error_policy = require;
        self
    }

    /// Sets the `requireTextBidiProperty` tunable: a text simple element whose
    /// `dfdl:textBidi` is not defined is a schema definition error.
    #[inline]
    #[must_use]
    pub const fn with_require_text_bidi(mut self, require: bool) -> Self {
        self.require_text_bidi = require;
        self
    }

    /// Sets the `requireFloatingProperty` tunable: a text simple element whose
    /// `dfdl:floating` is not defined is a schema definition error.
    #[inline]
    #[must_use]
    pub const fn with_require_floating(mut self, require: bool) -> Self {
        self.require_floating = require;
        self
    }

    /// Compiles an XSD schema document XML string into a [`CompiledSchema`].
    pub fn compile_str(&self, schema_xml: &str) -> DFDLResult<CompiledSchema> {
        self.compile_str_with_resolver(schema_xml, |_| None)
    }

    /// Compiles an XSD schema document with a custom import/include schema resolver callback.
    pub fn compile_str_with_resolver<F>(
        &self,
        schema_xml: &str,
        resolver: F,
    ) -> DFDLResult<CompiledSchema>
    where
        F: FnMut(&str) -> Option<String>,
    {
        self.compile_str_with_resolver_and_root(schema_xml, resolver, None)
    }

    /// Compiles an XSD schema document with a custom resolver and specific target root element name.
    pub fn compile_str_with_resolver_and_root<F>(
        &self,
        schema_xml: &str,
        mut resolver: F,
        target_root: Option<&str>,
    ) -> DFDLResult<CompiledSchema>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let mut reader = XmlReader::with_limits(schema_xml, self.limits);
        reader.set_permissive_namespaces(true);
        let mut visited = Vec::new();
        let xsd_schema = self.parse_schema_document_internal(
            &mut reader,
            &mut resolver,
            &mut visited,
            Some(schema_xml),
            None,
            None,
        )?;
        if xsd_schema.top_level_elements.is_empty() {
            return Err(DFDLError::new_static(
                DFDLErrorKind::SchemaDefinition,
                "No top-level element found in XSD schema (parse)",
            ));
        }
        self.lower_schema_to_ir_with_root(&xsd_schema, target_root)
    }


}
