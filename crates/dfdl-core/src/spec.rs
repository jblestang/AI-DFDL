//! DFDL Specification baseline definitions and feature profile metadata.
//!
//! This module pins the normative DFDL specification baseline (OGF GFD-R-P.240)
//! along with incorporated errata, conformance levels, and feature profiles.

/// Normative specification reference metadata.
///
/// Holds metadata regarding the target DFDL specification version and document ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecBaseline {
    /// Document identifier string (e.g. "OGF GFD-R-P.240").
    pub doc_id: &'static str,
    /// Major specification version number.
    pub version_major: u8,
    /// Minor specification version number.
    pub version_minor: u8,
    /// Whether incorporated errata are included.
    pub errata_included: bool,
}

/// Baseline specification constant representing OGF GFD-R-P.240 DFDL 1.0.
pub const DFDL_1_0_BASELINE: SpecBaseline = SpecBaseline {
    doc_id: "OGF GFD-R-P.240",
    version_major: 1,
    version_minor: 0,
    errata_included: true,
};

/// Engine conformance profile level.
///
/// Defines the feature set supported by a specific engine build configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConformanceLevel {
    /// Minimal profile: basic binary/text scalars, fixed/implicit lengths, sequence groups.
    Minimal,
    /// Extended profile: adds variable arrays, discriminators, assertions, escape schemes.
    Extended,
    /// Full profile: complete DFDL 1.0 standard including calendar, packed decimals.
    Full,
}

/// Feature profile configuration for the DFDL engine.
///
/// Expresses explicit feature flags and limits enabled for schema processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureProfile {
    /// Active conformance level.
    pub level: ConformanceLevel,
    /// Supports binary floating-point primitives.
    pub enable_binary_floats: bool,
    /// Supports text regular expressions.
    pub enable_regex: bool,
    /// Supports dynamic expression evaluation.
    pub enable_expressions: bool,
    /// Supports element default values and nils.
    pub enable_defaults_and_nils: bool,
}

impl FeatureProfile {
    /// Constructs the standard initial feature profile (Extended level).
    ///
    /// # Examples
    /// ```rust
    /// use dfdl_core::spec::{FeatureProfile, ConformanceLevel};
    /// let profile = FeatureProfile::extended();
    /// assert_eq!(profile.level, ConformanceLevel::Extended);
    /// assert!(profile.enable_regex);
    /// ```
    #[inline]
    #[must_use]
    pub const fn extended() -> Self {
        Self {
            level: ConformanceLevel::Extended,
            enable_binary_floats: true,
            enable_regex: true,
            enable_expressions: true,
            enable_defaults_and_nils: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_spec_baseline_values() {
        assert_eq!(DFDL_1_0_BASELINE.doc_id, "OGF GFD-R-P.240");
        assert_eq!(DFDL_1_0_BASELINE.version_major, 1);
        assert_eq!(DFDL_1_0_BASELINE.version_minor, 0);
        assert!(DFDL_1_0_BASELINE.errata_included);
    }

    #[test]
    fn test_feature_profile_extended() {
        let profile = FeatureProfile::extended();
        assert_eq!(profile.level, ConformanceLevel::Extended);
        assert!(profile.enable_binary_floats);
        assert!(profile.enable_regex);
        assert!(profile.enable_expressions);
        assert!(profile.enable_defaults_and_nils);
    }
}
