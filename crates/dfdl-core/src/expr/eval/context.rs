//! Evaluation context for DFDL expressions.

extern crate alloc;
use alloc::string::String;

use crate::error::DFDLResult;
use crate::infoset::tree::{InfosetDocument, InfosetElement};
use crate::infoset::value::DfdlValue;
use crate::limits::WorkBudget;
use crate::types::InfosetPath;

/// Maximum evaluation recursion depth to prevent stack overflow.
pub const MAX_EVAL_DEPTH: usize = 64;

/// Evaluation context containing variables, target document, and work budget limits.
pub struct ExprContext<'a> {
    /// Optional reference to active Infoset document.
    pub doc: Option<&'a InfosetDocument>,
    /// Current path context in document.
    pub current_path: &'a InfosetPath,
    /// Variable bindings available in scope (`(variable_name, value)`).
    pub variables: &'a [(&'a str, DfdlValue)],
    /// Optional DFDL schema variable map (`VariableMap`).
    pub variable_map: Option<&'a crate::expr::variables::VariableMap>,
    /// Optional compiled schema for property lookups.
    pub schema: Option<&'a crate::schema::ir::CompiledSchema>,
    /// Remaining work budget.
    pub budget: &'a mut WorkBudget,
    /// Current recursion depth.
    pub depth: usize,
    /// Current 1-based array iteration index (DFDL §23.1 dfdl:occursIndex()).
    pub occurs_index: usize,
    /// Indicates whether evaluation is occurring during parsing (`true`) or unparsing (`false`).
    pub is_parsing: bool,
    /// Optional lookahead function to read bits from the bitstream without advancing position.
    pub lookahead_fn: Option<&'a (dyn Fn(usize, usize) -> DFDLResult<u128> + 'a)>,
    /// Enclosing complex elements and their known lengths, length units, and encodings during parsing.
    pub enclosing_lengths: &'a [(String, usize, crate::schema::ir::LengthUnits, crate::encoding::EncodingKind)],
    /// Policy for resolving unqualified path steps in expressions (§23).
    pub unqualified_path_step_policy: crate::types::UnqualifiedPathStepPolicy,
    /// In-scope namespaces at the expression evaluation point (`[(prefix, uri)]`).
    pub in_scope_namespaces: &'a [(alloc::string::String, alloc::string::String)],
    /// Computed outputValueCalc values for elements during unparsing.
    pub ovc_values: Option<&'a alloc::collections::BTreeMap<alloc::string::String, DfdlValue>>,
}

impl<'a> ExprContext<'a> {
    /// Creates a new [`ExprContext`].
    pub fn new(
        doc: Option<&'a InfosetDocument>,
        current_path: &'a InfosetPath,
        variables: &'a [(&'a str, DfdlValue)],
        budget: &'a mut WorkBudget,
    ) -> Self {
        Self {
            doc,
            current_path,
            variables,
            variable_map: None,
            schema: None,
            budget,
            depth: 0,
            occurs_index: 1,
            is_parsing: true,
            lookahead_fn: None,
            enclosing_lengths: &[],
            unqualified_path_step_policy: Default::default(),
            in_scope_namespaces: &[],
            ovc_values: None,
        }
    }

    /// Creates a new [`ExprContext`] with an active schema [`VariableMap`].
    pub fn with_variable_map(
        doc: Option<&'a InfosetDocument>,
        current_path: &'a InfosetPath,
        variables: &'a [(&'a str, DfdlValue)],
        variable_map: Option<&'a crate::expr::variables::VariableMap>,
        budget: &'a mut WorkBudget,
    ) -> Self {
        Self {
            doc,
            current_path,
            variables,
            variable_map,
            schema: None,
            budget,
            depth: 0,
            occurs_index: 1,
            is_parsing: true,
            lookahead_fn: None,
            enclosing_lengths: &[],
            unqualified_path_step_policy: Default::default(),
            in_scope_namespaces: &[],
            ovc_values: None,
        }
    }

    /// Attaches precomputed OVC values for unparsing.
    #[must_use]
    pub fn with_ovc_values(
        mut self,
        ovc_values: Option<&'a alloc::collections::BTreeMap<alloc::string::String, DfdlValue>>,
    ) -> Self {
        self.ovc_values = ovc_values;
        self
    }

    /// Attaches known enclosing element lengths during parsing.
    #[must_use]
    pub fn with_enclosing_lengths(
        mut self,
        enclosing_lengths: &'a [(String, usize, crate::schema::ir::LengthUnits, crate::encoding::EncodingKind)],
    ) -> Self {
        self.enclosing_lengths = enclosing_lengths;
        self
    }

    /// Attaches an active compiled schema for element property lookups and inherits its policy.
    #[must_use]
    pub fn with_schema(mut self, schema: &'a crate::schema::ir::CompiledSchema) -> Self {
        self.unqualified_path_step_policy = schema.unqualified_path_step_policy;
        self.schema = Some(schema);
        self
    }

    /// Attaches in-scope namespaces for prefix and default-namespace resolution.
    #[must_use]
    pub fn with_namespaces(
        mut self,
        namespaces: &'a [(alloc::string::String, alloc::string::String)],
    ) -> Self {
        self.in_scope_namespaces = namespaces;
        self
    }

    /// Overrides the policy for resolving unqualified path steps in expressions.
    #[must_use]
    pub fn with_unqualified_path_step_policy(
        mut self,
        policy: crate::types::UnqualifiedPathStepPolicy,
    ) -> Self {
        self.unqualified_path_step_policy = policy;
        self
    }

    /// Sets the active 1-based array iteration index for `dfdl:occursIndex()`.
    #[must_use]
    pub fn with_occurs_index(mut self, idx: usize) -> Self {
        self.occurs_index = idx;
        self
    }

    /// Configures the context for unparser evaluation (`is_parsing = false`).
    #[must_use]
    pub fn for_unparsing(mut self) -> Self {
        self.is_parsing = false;
        self
    }

    /// Attaches an optional lookahead function for reading bits from the bitstream.
    #[must_use]
    pub fn with_lookahead(
        mut self,
        la_fn: Option<&'a (dyn Fn(usize, usize) -> DFDLResult<u128> + 'a)>,
    ) -> Self {
        self.lookahead_fn = la_fn;
        self
    }

    /// Resolves an element in the active document respecting policy and namespaces.
    #[inline]
    #[must_use]
    pub fn find_element(&self, path: &InfosetPath) -> Option<&'a InfosetElement> {
        let doc = self.doc?;
        doc.find_element_with_policy_checked(
            path,
            self.occurs_index,
            false,
            self.unqualified_path_step_policy,
            self.in_scope_namespaces,
        )
        .ok()
        .flatten()
    }
}
