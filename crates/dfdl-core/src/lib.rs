//! `dfdl-core`: Core vocabulary, types, diagnostics, limits, and panic-free utilities.
//!
//! Designed for `#![no_std]` + `alloc` environments.

#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

extern crate alloc;

pub mod encoding;
pub mod error;
pub mod expr;
pub mod infoset;
pub mod io;
pub mod kernel;
pub mod limits;
pub mod pattern;
pub mod schema;
pub mod spec;
pub mod types;
pub mod util;

pub use error::{DFDLError, DFDLErrorKind, DFDLResult, ErrorMessage};
pub use kernel::{ParserEngine, UnparserEngine};
pub use limits::{ResourceLimits, WorkBudget};
pub use spec::{ConformanceLevel, FeatureProfile, SpecBaseline, DFDL_1_0_BASELINE};
pub use types::{BitOffset, ByteOffset, InfosetPath, Namespace, QName, SourceLocation, SymbolId};
pub use util::{
    checked_add_usize, checked_div_usize, checked_mul_usize, get_checked, get_slice_checked,
    try_extend_from_slice, try_push,
};
