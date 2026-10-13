# Formal Audit: WorkBudget Implementation and Usage vs. DFDL v1.0 Standard

**Date:** 2026-10-13 07:25:00 UTC+2  
**Author:** Antigravity AI  
**Scope:** `WorkBudget` definition, lifecycle, consumption sites, and error handling across `crates/dfdl-core`, `crates/dfdl-vm`, `crates/dfdl-schema`, and `crates/dfdl-tests`.  
**Standard References:** OGF DFDL v1.0 (GFD-P-R.207) §3.1, §3.2, §9.3, §15, §16, and Appendix F.

---

## 1. Executive Summary

This audit assesses the [`WorkBudget`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/limits.rs#L58-L94) mechanism implemented in `AI-DFDL` against the requirements and execution model of the **DFDL v1.0 standard**.

### Key Assessment
* **Architectural Intent (Sound):** The use of a monotonically decreasing execution budget (`remaining_work: usize`) prevents infinite loops, recursive runaway, and algorithmic complexity Denial-of-Service (DoS) attacks.
* **Critical Compliance Defect (Vulnerability):** In both **Choice** resolution (§15) and **Array occurrence** loops (§16), the parser treats `WorkBudgetExhausted` as a standard recoverable **Processing Error (PE)** rather than a **Fatal Implementation Limit**. 
  - In choices, budget exhaustion triggers speculative backtracking across subsequent branches, converting a resource exhaustion error into `Parse Error: All Choice Alternatives Failed`.
  - In arrays with `minOccurs="0"` or satisfied `minOccurs`, budget exhaustion is silently swallowed as a normal loop termination, causing the parser to return a truncated infoset with a depleted budget.
* **Engine Disconnect:** While `dfdl-core` meters term parsing and expression AST nodes, `dfdl-vm` does not share `WorkBudget`, relying instead on an internal `max_steps` instruction counter. Delimiter scanning in large streams is currently unmetered.

---

## 2. DFDL v1.0 Specification Baseline

The DFDL v1.0 specification defines strict semantics for errors, implementation limits, and points of uncertainty:

### 2.1 Error Taxonomy (DFDL §3.1 & §3.2)
1. **Schema Definition Error (SDE):** Static or dynamic schema inconsistency. Strictly non-recoverable and fatal; no backtracking allowed.
2. **Processing Error (PE):** Occurs during parsing or unparsing when data does not match the schema. If encountered at a Point of Uncertainty (PoU), the processor must backtrack. If not at a PoU, it is fatal.
3. **Validation Error (VE):** Data facet/type violation; never stops processing and never causes backtracking.
4. **Implementation Limits (DFDL §3.1):**
   > *"A DFDL processor may provide an implementation limit, for example a limit on the number of elements in an array or the recursion depth of schemas. Exceeding an implementation limit is an implementation-defined error, but is usually treated as a fatal processing error or schema definition error."*

### 2.2 Points of Uncertainty (PoU) and Backtracking (DFDL §9.3, §15, Appendix F)
Speculative parsing is permitted **only** to resolve ambiguities (choice alternatives, optional elements, `occursCountKind="parsed"`). 
When an unrecoverable system or implementation limit (such as resource exhaustion, out of memory, or execution budget exhaustion) is reached, treating it as a normal data-mismatch processing error breaks the standard contract and masks critical resource depletion.

---

## 3. Structural Audit of `WorkBudget`

### 3.1 Implementation (`crates/dfdl-core/src/limits.rs:58-94`)
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkBudget {
    remaining_work: usize,
}
```
* **Arithmetic Safety:** `consume()` uses `self.remaining_work.checked_sub(cost)`, safely handling underflows without panic.
* **Error Return:** Returns `DFDLErrorKind::WorkBudgetExhausted` when `remaining_work < cost`.
* **State Invariance:** `WorkBudget` is strictly non-increasing. It is intentionally **not** restored during parser checkpoint rollbacks (`reader.rollback()`, `builder.rollback()`). This is the correct security posture: physical CPU work spent exploring failed speculative paths must not be refunded, otherwise an attacker could exploit exponential backtracking paths.

---

## 4. Consumption Mapping & Metering Coverage

### 4.1 Active Consumption Sites (`.budget.consume(1)`)
Currently, `consume(1)` is invoked at three locations:
1. **Parser Engine Term Entry:** [`crates/dfdl-core/src/kernel/parser/mod.rs:411`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/mod.rs#L411)
   - Every invocation of `parse_term_inner` decrements the budget by 1.
2. **Unparser Engine Term Entry:** [`crates/dfdl-core/src/kernel/unparser.rs:203`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/unparser.rs#L203)
   - Every invocation of `unparse_term_internal` decrements the budget by 1.
3. **Expression Evaluator AST Node:** [`crates/dfdl-core/src/expr/eval.rs:198`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/expr/eval.rs#L198)
   - Every AST node evaluated in `eval_expr` decrements the shared `budget` reference in `ExprContext` by 1.

### 4.2 Unmetered Blind Spots
1. **Delimiter & Stream Scanning:** Delimiter search routines (`match_literal_delimiter`, `peek_delimiter_match_length`, `peek_in_scope_delimiter`) scan input bitstreams without decrementing `WorkBudget`. An input stream of tens of megabytes with no matching delimiter incurs heavy byte/bit decoding without consuming work units until EOF or backtracking limit is hit.
2. **Subsystem Disconnect (`dfdl-vm`):** Bytecode execution in `crates/dfdl-vm` does not use `WorkBudget`. It defines an independent `self.max_steps = 10_000_000` instruction limit that triggers `DFDLErrorKind::ImplementationLimit`. As a result, the AST engine and the VM engine exhibit inconsistent resource limit behaviors.
3. **Schema Compiler Disconnect (`dfdl-schema`):** Schema compilation does not use `WorkBudget`. It relies solely on hardcoded recursion limits (`depth >= 32`).

---

## 5. Critical Findings & Vulnerabilities

### Finding 1 (Critical): Choice Backtracking Masks Budget Exhaustion
* **Location:** [`crates/dfdl-core/src/kernel/parser/mod.rs:1345-1372`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/mod.rs#L1345-L1372)
* **Code:**
  ```rust
  match self.parse_term(branch_id, builder) {
      Ok(()) => { ... break; }
      Err(e) => {
          if e.kind == DFDLErrorKind::SchemaDefinition {
              return Err(e);
          }
          ...
          last_choice_error = Some(e);
          self.reader.rollback(reader_cp)?;
          builder.rollback(builder_cp);
          // Continues to next branch!
      }
  }
  ```
* **Impact:** If Branch 1 exhausts the budget (`DFDLErrorKind::WorkBudgetExhausted`), the error kind is **not** `SchemaDefinition`. The engine rolls back and attempts Branch 2. Because `remaining_work == 0`, Branch 2 immediately fails with `WorkBudgetExhausted`, cascading across all branches. The parser then converts this into:
  `Err(DFDLError::new(DFDLErrorKind::Parse, "Parse Error: All Choice Alternatives Failed..."))`.
* **Standard Violation:** A fatal resource limit is misclassified as a recoverable data-mismatch processing error.

---

### Finding 2 (High): Array Speculation Swallows Budget Exhaustion (Silent Data Truncation)
* **Location:** [`crates/dfdl-core/src/kernel/parser/element.rs:790-800`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs#L790-L800) and [`element.rs:1058-1065`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/kernel/parser/element.rs#L1058-L1065)
* **Code:**
  ```rust
  if count >= elem.min_occurs && e.kind != DFDLErrorKind::SchemaDefinition {
      break;
  }
  return Err(e);
  ```
* **Impact:** In bounded occurrences or `occursCountKind="parsed"` arrays:
  - If `minOccurs` is 0, or if at least `minOccurs` elements have already been parsed, an error in parsing the subsequent occurrence causes the loop to check `e.kind != SchemaDefinition`.
  - When `WorkBudgetExhausted` occurs, the condition holds true, the loop **breaks gracefully**, and the function returns `Ok(count)`.
  - The budget exhaustion is completely suppressed. The parser then continues to the next schema component with a zeroed budget. If at EOF, the parse succeeds falsely, producing a truncated infoset.
* **Standard Violation:** Violates DFDL §16.1.4 and §3.1. Implementation limits must abort parsing and must not produce partial/truncated valid documents.

---

### Finding 3 (Medium): Disconnection Between `ResourceLimits` and `WorkBudget`
* **Location:** [`crates/dfdl-core/src/limits.rs:20,35`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/limits.rs#L20)
* **Code:** `ResourceLimits` contains `max_expression_work: usize = 100_000`, yet `ParserEngine::new` accepts an unconstrained `&mut WorkBudget`. Callers and tests instantiate `WorkBudget::new(100)` or `WorkBudget::new(1000)` with arbitrary magic numbers.
* **Recommendation:** Provide a constructor or builder linking `ParserEngine` configuration directly to `ResourceLimits`.

---

## 6. Detailed Remediation Recommendations

### 6.1 Introduce a Fatal Error Predicate
Add an explicit helper on [`DFDLError`](file:///Users/jean-baptiste/AI-DFDL/crates/dfdl-core/src/error.rs):
```rust
impl DFDLError {
    /// Returns true if the error represents a fatal, non-recoverable condition
    /// that MUST NOT trigger speculative backtracking at Points of Uncertainty.
    #[inline]
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        matches!(
            self.kind,
            DFDLErrorKind::SchemaDefinition
                | DFDLErrorKind::WorkBudgetExhausted
                | DFDLErrorKind::ImplementationLimit
                | DFDLErrorKind::InternalInvariant
        )
    }
}
```

### 6.2 Update PoU Error Handlers in Choice and Element Loops
In `crates/dfdl-core/src/kernel/parser/mod.rs:1346`:
```rust
- if e.kind == DFDLErrorKind::SchemaDefinition {
+ if e.is_fatal() {
      return Err(e);
  }
```
In `crates/dfdl-core/src/kernel/parser/element.rs:795` and `1058`:
```rust
- if count >= elem.min_occurs && e.kind != DFDLErrorKind::SchemaDefinition {
+ if count >= elem.min_occurs && !e.is_fatal() {
      break;
  }
```

### 6.3 Harmonize `dfdl-vm` with `WorkBudget`
Refactor `dfdl-vm::VmInterpreter` to accept `&mut WorkBudget` (or consume steps from it), ensuring both AST and Bytecode execution paths share unified DoS protection semantics.

### 6.4 Add Conformance Tests for Budget Exhaustion
Add integration tests in `crates/dfdl-tests`:
1. `test_choice_budget_exhaustion_does_not_backtrack`: Confirms that budget exhaustion in Branch 1 returns `WorkBudgetExhausted` immediately without attempting Branch 2 or reporting "All Choice Alternatives Failed".
2. `test_array_budget_exhaustion_does_not_truncate`: Confirms that budget exhaustion during array parsing aborts with `WorkBudgetExhausted` rather than returning a truncated infoset.
