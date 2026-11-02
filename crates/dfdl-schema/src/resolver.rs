//! Schema resolution abstractions and implementations for XSD `include` and `import`.
//!
//! Provides the [`SchemaResolver`] trait, allowing custom, in-memory,
//! or cached resolution of external schemas referenced by `xs:include` and `xs:import`.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::string::String;

/// Abstraction for resolving external XML/XSD schema documents referenced by
/// `xs:include` (`schemaLocation`) or `xs:import` (`namespace`, `schemaLocation`).
pub trait SchemaResolver {
    /// Resolves an included or imported XSD schema file given its location and optional base URI.
    ///
    /// # Arguments
    /// * `location` - The target location or URI specified in `schemaLocation` or `namespace`.
    /// * `base_uri` - Optional base URI of the enclosing schema document.
    ///
    /// Returns `Some(xml_content)` if resolved, or `None` if not found.
    fn resolve_schema(&mut self, location: &str, base_uri: Option<&str>) -> Option<String>;
}

/// Blanket implementation of [`SchemaResolver`] for any closure `FnMut(&str) -> Option<String>`.
///
/// Ensures full backward compatibility with existing closure-based resolver callbacks.
impl<F> SchemaResolver for F
where
    F: FnMut(&str) -> Option<String>,
{
    #[inline]
    fn resolve_schema(&mut self, location: &str, _base_uri: Option<&str>) -> Option<String> {
        (self)(location)
    }
}

/// A no-op schema resolver that always returns `None`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullResolver;

impl SchemaResolver for NullResolver {
    #[inline]
    fn resolve_schema(&mut self, _location: &str, _base_uri: Option<&str>) -> Option<String> {
        None
    }
}

/// In-memory map-based schema resolver.
///
/// Useful for testing, embedded schemas, or pre-loaded multi-file schemas.
#[derive(Debug, Clone, Default)]
pub struct MapResolver {
    schemas: BTreeMap<String, String>,
}

impl MapResolver {
    /// Creates a new empty `MapResolver`.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            schemas: BTreeMap::new(),
        }
    }

    /// Registers a schema XML string for a specific location URI.
    pub fn add(&mut self, location: impl Into<String>, content: impl Into<String>) {
        self.schemas.insert(location.into(), content.into());
    }

    /// Returns the number of registered schemas.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.schemas.len()
    }

    /// Returns `true` if no schemas are registered.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.schemas.is_empty()
    }
}

impl SchemaResolver for MapResolver {
    #[inline]
    fn resolve_schema(&mut self, location: &str, _base_uri: Option<&str>) -> Option<String> {
        self.schemas.get(location).cloned()
    }
}

/// Caching wrapper for any [`SchemaResolver`].
///
/// Memorizes previous resolution results (including cache misses) to avoid redundant resolution lookups.
#[derive(Debug, Clone, Default)]
pub struct CachingResolver<R> {
    inner: R,
    cache: BTreeMap<String, Option<String>>,
}

impl<R: SchemaResolver> CachingResolver<R> {
    /// Wraps an existing resolver with an in-memory resolution cache.
    #[inline]
    pub const fn new(inner: R) -> Self {
        Self {
            inner,
            cache: BTreeMap::new(),
        }
    }

    /// Clears the cached schema documents.
    #[inline]
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Returns the number of cached lookups.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns `true` if no lookups have been cached.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl<R: SchemaResolver> SchemaResolver for CachingResolver<R> {
    fn resolve_schema(&mut self, location: &str, base_uri: Option<&str>) -> Option<String> {
        if let Some(cached) = self.cache.get(location) {
            return cached.clone();
        }
        let resolved = self.inner.resolve_schema(location, base_uri);
        self.cache.insert(String::from(location), resolved.clone());
        resolved
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_map_and_caching_resolver() {
        let mut map_resolver = MapResolver::new();
        assert!(map_resolver.is_empty());
        map_resolver.add("foo.xsd", "<xs:schema/>");
        assert_eq!(map_resolver.len(), 1);
        assert!(!map_resolver.is_empty());

        let mut caching = CachingResolver::new(map_resolver);
        assert!(caching.is_empty());

        let res = caching.resolve_schema("foo.xsd", None);
        assert_eq!(res, Some(String::from("<xs:schema/>")));
        assert_eq!(caching.len(), 1);

        let res2 = caching.resolve_schema("foo.xsd", None);
        assert_eq!(res2, Some(String::from("<xs:schema/>")));

        let res_miss = caching.resolve_schema("unknown.xsd", None);
        assert_eq!(res_miss, None);
        assert_eq!(caching.len(), 2);

        caching.clear();
        assert!(caching.is_empty());
    }

    #[test]
    fn test_compiler_with_map_resolver() {
        use crate::compiler::SchemaCompiler;

        let main_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:include schemaLocation="sub.xsd"/>
    <xs:element name="Root" type="xs:int" dfdl:lengthKind="explicit" dfdl:length="4"/>
</xs:schema>"#;

        let sub_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:dfdl="http://www.dfdl.org/7793">
    <xs:annotation>
        <xs:appinfo source="http://www.dfdl.org/7793">
            <dfdl:format byteOrder="bigEndian"/>
        </xs:appinfo>
    </xs:annotation>
</xs:schema>"#;

        let mut map = MapResolver::new();
        map.add("sub.xsd", sub_xml);

        let compiler = SchemaCompiler::new();
        let schema = compiler.compile_str_with_resolver(main_xml, CachingResolver::new(map)).unwrap();
        assert_eq!(schema.root_term().unwrap().name.local_name, "Root");
    }
}
