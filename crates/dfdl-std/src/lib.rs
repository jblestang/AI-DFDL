//! `dfdl-std`: Optional host `std::io` adapters and filesystem integrations.

#![warn(missing_docs)]
#![allow(ambiguous_glob_reexports)]

pub use dfdl_core::*;
pub use dfdl_schema::*;
pub use dfdl_xml::*;

/// Helper function to describe std integration status.
#[must_use]
pub fn std_adapter_info() -> &'static str {
    "dfdl-std adapters ready"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_std_adapter_info() {
        assert_eq!(std_adapter_info(), "dfdl-std adapters ready");
    }
}
