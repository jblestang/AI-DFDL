# PANIC_SAFETY.md — Panic-Free Architectural Policy & Verification Guidelines

## 1. Zero-Panic Mandate

In all production crates (`dfdl-core`, `dfdl-xml`, `dfdl-schema`), code MUST be panic-free by design.

### 1.1 Prohibited Language Constructs in Production
- `panic!`, `todo!`, `unimplemented!`, `unreachable!`
- `.unwrap()`, `.expect()`
- `assert!`, `assert_eq!`, `debug_assert!`
- Direct indexing (`slice[i]`, `vec[i]`)
- Unchecked integer arithmetic (`+`, `-`, `*`, `/`, `%`, `<<`, `>>`)
- Unchecked type casting (`as`)
- `unsafe` blocks or `core::hint::unreachable_unchecked()`
- Input-controlled recursion or stack growth

### 1.2 Required Idiomatic Constructs
- Safe lookup: `.get(i)` or `.get_mut(i)` returning `Option<&T>`
- Safe arithmetic: `.checked_add()`, `.checked_sub()`, `.checked_mul()`, `.checked_div()`, `.checked_shl()`
- Safe conversions: `TryFrom` / `TryInto`
- Safe growth: `vec.try_reserve(n)` followed by push/extend
- Safe recursion: State machines or explicit bounded stacks (`Vec` with size cap)

## 2. Clippy & Compiler Enforcement

All core crates mandate the following workspace lint configuration:

```toml
[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
unreachable = "deny"
indexing_slicing = "deny"
arithmetic_side_effects = "deny"
```

## 3. Section Review Checklist & Audit Template

Before declaring any section complete, the following checklist MUST be verified:

- [ ] All indexing uses `.get()` / `.get_mut()` with explicit `None` error paths.
- [ ] All arithmetic operations use `.checked_*()` or safe checked math helper routines.
- [ ] Collections are allocated or grown using fallible `try_reserve` or pre-checked limits.
- [ ] No `unwrap()`, `expect()`, `panic!()`, `todo!()`, or `unimplemented!()` exist in non-test production files.
- [ ] Recursive calls are eliminated or bounded by an explicit stack-depth counter.
- [ ] Iterators and loop increments have progress guarantees to prevent infinite loops.
- [ ] Binary builds successfully for `thumbv7m-none-eabi` (`#![no_std]`).
