//! Resource limits for the `dfdl-xml` streaming XML parser.
//!
//! Enforces limits on XML element nesting depth, attribute counts, token lengths,
//! entity expansion size, and decoded character counts to protect against entity expansion
//! attacks (e.g. Billion Laughs) and stack overflow.

/// Resource limits for XML parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmlReaderLimits {
    /// Maximum element nesting depth.
    pub max_depth: usize,
    /// Maximum number of attributes per element.
    pub max_attributes: usize,
    /// Maximum byte length for a single token, element name, or attribute value.
    pub max_token_length: usize,
    /// Maximum allowed entity expansion buffer size in bytes.
    pub max_entity_expansion_bytes: usize,
    /// Whether undeclared XML namespace prefixes raise an error (true) or fallback to prefix string (false).
    pub strict_namespaces: bool,
}

impl Default for XmlReaderLimits {
    /// Returns safe default resource limits for XML parsing.
    #[inline]
    fn default() -> Self {
        Self {
            max_depth: 128,
            max_attributes: 128,
            max_token_length: 65_536,             // 64 KiB
            max_entity_expansion_bytes: 104_8576, // 1 MiB
            strict_namespaces: true,
        }
    }
}

impl XmlReaderLimits {
    /// Checks if a proposed nesting depth is within limits.
    #[inline]
    #[must_use]
    pub const fn check_depth(&self, depth: usize) -> bool {
        depth <= self.max_depth
    }

    /// Checks if attribute count is within limits.
    #[inline]
    #[must_use]
    pub const fn check_attribute_count(&self, count: usize) -> bool {
        count <= self.max_attributes
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_limits_defaults() {
        let limits = XmlReaderLimits::default();
        assert_eq!(limits.max_depth, 128);
        assert_eq!(limits.max_attributes, 128);
        assert!(limits.check_depth(100));
        assert!(!limits.check_depth(200));
    }
}
