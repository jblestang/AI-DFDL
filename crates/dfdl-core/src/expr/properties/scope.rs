//! Hierarchical property scoping and delegation.
//!
//! Follows DFDL 1.0 Specification §13 for property inheritance along the lexical scope chain.

use super::keys::PropertyKey;
use super::store::PropertyStore;

/// Hierarchical lexical property scope maintaining parent delegation without full store cloning.
///
/// Implements RUST-SCH-02 to prevent deep cloning of `PropertyStore` and strings during tree descent.
#[derive(Debug, Clone, Copy)]
pub struct PropertyScope<'a> {
    local: &'a PropertyStore,
    parent: Option<&'a PropertyScope<'a>>,
}

impl<'a> PropertyScope<'a> {
    /// Constructs a root property scope from a base property store.
    #[inline]
    #[must_use]
    pub const fn root(local: &'a PropertyStore) -> Self {
        Self { local, parent: None }
    }

    /// Creates a child scope inheriting from this scope.
    #[inline]
    #[must_use]
    pub const fn child(&'a self, local: &'a PropertyStore) -> Self {
        Self {
            local,
            parent: Some(self),
        }
    }

    /// Returns the local property store for this scope level.
    #[inline]
    #[must_use]
    pub const fn local(&self) -> &'a PropertyStore {
        self.local
    }

    /// Retrieves a property value by string key, querying local bindings first,
    /// then delegating up the parent scope chain for inheritable properties (§13).
    #[must_use]
    pub fn get_property(&self, key: &str) -> Option<&'a str> {
        if let Some(val) = self.local.get_property(key) {
            return Some(val);
        }
        if let Some(parent) = self.parent {
            let is_inheritable = PropertyKey::parse(key)
                .map(|pk| pk.is_inheritable())
                .unwrap_or(true);
            if is_inheritable {
                return parent.get_property(key);
            }
        }
        None
    }

    /// Retrieves a property value by strongly typed [`PropertyKey`], querying local bindings
    /// first, then delegating up the parent scope chain if inheritable.
    #[must_use]
    pub fn get_property_key(&self, key: PropertyKey) -> Option<&'a str> {
        if let Some(val) = self.local.get_property_key(key) {
            return Some(val);
        }
        if key.is_inheritable() {
            if let Some(parent) = self.parent {
                return parent.get_property_key(key);
            }
        }
        None
    }

    /// Resolves and materializes all effective properties into an owned [`PropertyStore`].
    #[must_use]
    pub fn materialize(&self) -> PropertyStore {
        if let Some(parent) = self.parent {
            let parent_store = parent.materialize();
            let mut result = self.local.clone();
            result.merge_parent(&parent_store);
            result
        } else {
            self.local.clone()
        }
    }
}
