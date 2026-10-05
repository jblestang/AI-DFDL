# DEPENDENCY_AUDIT.md — Dependency Audit & `no_std` Compliance Policy

## 1. Policy Overview

To guarantee zero-panic behavior, `#![no_std]` compatibility, and predictable allocation characteristics, every external dependency introduced into production crates MUST undergo a strict dependency audit.

## 2. Dependency Audit Criteria

Each dependency entry must document:
1. **Name & Version**: Exact crate name and pinned version.
2. **Purpose**: Rationale for inclusion.
3. **`no_std` Support**: Is `#![no_std]` natively supported without default features?
4. **Allocation Behavior**: Does it allocate memory dynamically? Does it support fallible allocation?
5. **Panic Safety**: Does it contain internal panics, unwraps, or assertions on malformed input?
6. **Untrusted Input Handling**: Does it directly process raw untrusted input?

## 3. Current Workspace Dependency Audit

| Crate | Target Crates | `no_std` | Dynamic Alloc | Direct Untrusted Input | Panic Safety Status | Audit Notes |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `core` (Rust stdlib) | All | Yes | No | Yes | Audited / Language Builtin | Safe checked APIs used exclusively |
| `alloc` (Rust stdlib) | All | Yes | Yes (Fallible) | No | Audited / Language Builtin | `try_reserve` used for growth |
| `xmlparser` (v0.13.6) | `dfdl-xml` | Yes (`#![no_std]`) | No (Zero Alloc) | Yes (Raw XML) | Audited / Panic-free | `default-features = false`, zero allocation tokenizer |

## 4. Third-Party Dependency Rule

Production core crates (`dfdl-core`, `dfdl-xml`, `dfdl-schema`) MUST NOT depend on external third-party crates unless:
- The crate is `#![no_std]`.
- Default features supporting `std` are disabled (`default-features = false`).
- The crate source code has been audited for absence of `panic!`, `unwrap()`, and indexing panics on untrusted input.
