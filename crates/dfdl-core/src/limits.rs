//! Resource limits and execution budgets for DFDL parsing, unparsing, and schema compilation.
//!
//! To enforce panic-freedom and prevent infinite recursion or unbounded memory growth,
//! all engine operations evaluate operations against configured [`ResourceLimits`].

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};

/// Execution and memory limits for the DFDL engine.
///
/// Prevents denial-of-service, stack overflow, or memory exhaustion from untrusted inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum nesting depth for schema components and Infoset nodes.
    pub max_nesting_depth: usize,
    /// Maximum number of array occurrences during parsing or unparsing.
    pub max_array_occurrences: usize,
    /// Maximum lookahead or backtracking window size in bytes.
    pub max_backtrack_bytes: usize,
    /// Maximum work cycles allowed during expression/regex evaluation.
    pub max_expression_work: usize,
    /// Maximum number of total Infoset nodes allowed.
    pub max_infoset_nodes: usize,
    /// Maximum schema byte size accepted by the schema compiler.
    pub max_schema_bytes: usize,
}

impl Default for ResourceLimits {
    /// Returns safe default resource limits suitable for constrained environments.
    #[inline]
    fn default() -> Self {
        Self {
            max_nesting_depth: 256,
            max_array_occurrences: 65_536,
            max_backtrack_bytes: 1_048_576, // 1 MiB
            max_expression_work: 100_000,
            max_infoset_nodes: 500_000,
            max_schema_bytes: 10_485_760, // 10 MiB
        }
    }
}

impl ResourceLimits {
    /// Checks if a proposed nesting depth exceeds configured limits.
    #[inline]
    #[must_use]
    pub const fn check_nesting(&self, depth: usize) -> bool {
        depth <= self.max_nesting_depth
    }

    /// Checks if a count of array occurrences exceeds configured limits.
    #[inline]
    #[must_use]
    pub const fn check_array_occurrences(&self, count: usize) -> bool {
        count <= self.max_array_occurrences
    }
}

/// Dynamic work budget counter to prevent CPU resource exhaustion or infinite loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkBudget {
    remaining_work: usize,
}

impl WorkBudget {
    /// Creates a new work budget initialized to `limit`.
    #[inline]
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self {
            remaining_work: limit,
        }
    }

    /// Consumes `cost` work units from the budget, returning an error if depleted.
    pub fn consume(&mut self, cost: usize) -> DFDLResult<()> {
        match self.remaining_work.checked_sub(cost) {
            Some(rem) => {
                self.remaining_work = rem;
                Ok(())
            }
            None => Err(DFDLError::new_static(
                DFDLErrorKind::WorkBudgetExhausted,
                "Work budget exceeded during execution",
            )),
        }
    }

    /// Returns the remaining work units.
    #[inline]
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.remaining_work
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_limits_defaults() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.max_nesting_depth, 256);
        assert_eq!(limits.max_array_occurrences, 65_536);
        assert!(limits.check_nesting(256));
        assert!(!limits.check_nesting(257));
        assert!(limits.check_array_occurrences(65_536));
        assert!(!limits.check_array_occurrences(65_537));
    }

    #[test]
    fn test_work_budget_consumption() {
        let mut budget = WorkBudget::new(100);
        assert_eq!(budget.remaining(), 100);
        budget.consume(40).unwrap();
        assert_eq!(budget.remaining(), 60);
        budget.consume(60).unwrap();
        assert_eq!(budget.remaining(), 0);
        assert!(budget.consume(1).is_err());
    }
}
