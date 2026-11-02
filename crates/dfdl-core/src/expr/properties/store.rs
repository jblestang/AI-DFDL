//! Scoped property storage and inheritance manipulation.
//!
//! Follows DFDL 1.0 Specification §6.3, §7, and §8.1.
//! Handles property bindings, assertions, variables, enumerations, and parent merging.

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::schema::ir::CompiledAssert;
use crate::util::try_push;

use super::entities::validate_dfdl_property_entities;
use super::keys::{PropertyBinding, PropertyKey, PropertyKeyName};

/// Scoped property store handling property binding and inheritance resolution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PropertyStore {
    pub(crate) bindings: Vec<PropertyBinding>,
    pub(crate) asserts: Vec<CompiledAssert>,
    pub(crate) set_variables: Vec<(crate::types::QName, String)>,
    pub(crate) new_variable_instances: Vec<(crate::types::QName, Option<String>)>,
    /// Number of discriminator statements attached locally.
    pub discriminator_count: usize,
    pub(crate) assert_errors: Vec<String>,
    pub(crate) in_scope_namespaces: Vec<(String, String)>,
}

impl PropertyStore {
    /// Constructs an empty [`PropertyStore`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            asserts: Vec::new(),
            set_variables: Vec::new(),
            new_variable_instances: Vec::new(),
            discriminator_count: 0,
            assert_errors: Vec::new(),
            in_scope_namespaces: Vec::new(),
        }
    }

    /// Returns the in-scope namespace bindings associated with this property store.
    #[inline]
    #[must_use]
    pub fn in_scope_namespaces(&self) -> &[(String, String)] {
        &self.in_scope_namespaces
    }

    /// Sets the in-scope namespace bindings associated with this property store.
    pub fn set_in_scope_namespaces(&mut self, ns: Vec<(String, String)>) {
        self.in_scope_namespaces = ns;
    }

    /// Adds namespace bindings to the in-scope namespaces of this property store.
    pub fn add_namespaces(&mut self, ns: &[(String, String)]) {
        for (prefix, uri) in ns {
            if !self.in_scope_namespaces.iter().any(|(p, _)| p == prefix) {
                let _ = try_push(&mut self.in_scope_namespaces, (prefix.clone(), uri.clone()));
            }
        }
    }

    /// Adds or updates namespace bindings in the in-scope namespaces of this property store.
    ///
    /// If a prefix is already bound, its URI is updated to reflect inner XML shadowing.
    pub fn update_namespaces(&mut self, ns: &[(String, String)]) {
        for (prefix, uri) in ns {
            if let Some(existing) = self.in_scope_namespaces.iter_mut().find(|(p, _)| p == prefix) {
                existing.1 = uri.clone();
            } else {
                let _ = try_push(&mut self.in_scope_namespaces, (prefix.clone(), uri.clone()));
            }
        }
    }

    /// Returns `true` if the property store contains no bindings, asserts, set_variables, or assert_errors.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
            && self.asserts.is_empty()
            && self.set_variables.is_empty()
            && self.new_variable_instances.is_empty()
            && self.assert_errors.is_empty()
    }

    /// Adds a DFDL setVariable declaration to this property store (§7.7).
    pub fn add_set_variable(&mut self, var_name: &str, value_expr: &str) {
        let clean = var_name
            .trim_start_matches("tns:")
            .trim_start_matches("ex:");
        let _ = try_push(
            &mut self.set_variables,
            (crate::types::QName::local(clean), value_expr.to_string()),
        );
    }

    /// Adds a DFDL newVariableInstance declaration to this property store (§7.7).
    pub fn add_new_variable_instance(&mut self, var_name: &str, default_val_expr: Option<&str>) {
        let clean = var_name
            .trim_start_matches("tns:")
            .trim_start_matches("ex:");
        let _ = try_push(
            &mut self.new_variable_instances,
            (
                crate::types::QName::local(clean),
                default_val_expr.map(alloc::string::ToString::to_string),
            ),
        );
    }

    /// Adds a recorded assertion definition error to be reported during term validation.
    pub fn add_assert_error(&mut self, err: &str) {
        let _ = try_push(&mut self.assert_errors, err.to_string());
    }

    /// Returns recorded assertion definition errors for this property store.
    #[inline]
    #[must_use]
    pub fn assert_errors(&self) -> &[String] {
        &self.assert_errors
    }

    /// Returns recorded newVariableInstances declarations on this property store.
    #[inline]
    #[must_use]
    pub fn new_variable_instances(&self) -> &[(crate::types::QName, Option<String>)] {
        &self.new_variable_instances
    }

    /// Returns recorded setVariable statements on this property store.
    #[inline]
    #[must_use]
    pub fn set_variables(&self) -> &[(crate::types::QName, String)] {
        &self.set_variables
    }

    /// Copies all local properties, assertions, and setVariables from `other` into `self`.
    pub fn extend(&mut self, other: &PropertyStore) {
        let self_has_enums = self.bindings.iter().any(|b| b.key == "enumeration");
        for binding in &other.bindings {
            if binding.key.starts_with("repValue:") {
                if !self.bindings.iter().any(|b| b.key == binding.key && b.value == binding.value) {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else if binding.key == "enumeration" {
                if !self_has_enums
                    && !self.bindings.iter().any(|b| b.key == "enumeration" && b.value == binding.value)
                {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else if self.get_property(&binding.key).is_none() {
                let _ = self.set_property(&binding.key, &binding.value);
            }
        }
        for assert in &other.asserts {
            let _ = try_push(&mut self.asserts, assert.clone());
        }
        for (k, v) in &other.set_variables {
            if !self.set_variables.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.set_variables, (k.clone(), v.clone()));
            }
        }
        for (k, v) in &other.new_variable_instances {
            if !self.new_variable_instances.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.new_variable_instances, (k.clone(), v.clone()));
            }
        }
        for err in &other.assert_errors {
            let _ = try_push(&mut self.assert_errors, err.clone());
        }
        self.add_namespaces(&other.in_scope_namespaces);
        self.discriminator_count = self
            .discriminator_count
            .saturating_add(other.discriminator_count);
    }

    /// Copies local properties, assertions, and setVariables from `other` into `self`,
    /// excluding any property keys already defined in `exclude`.
    pub fn extend_excluding(&mut self, other: &PropertyStore, exclude: &PropertyStore) {
        let self_has_enums = self.bindings.iter().any(|b| b.key == "enumeration");
        for binding in &other.bindings {
            if binding.key.starts_with("repValue:") {
                if !self.bindings.iter().any(|b| b.key == binding.key && b.value == binding.value) {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else if binding.key == "enumeration" {
                if !self_has_enums
                    && !self.bindings.iter().any(|b| b.key == "enumeration" && b.value == binding.value)
                {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else if exclude.get_property(&binding.key).is_none()
                && self.get_property(&binding.key).is_none()
            {
                let _ = self.set_property(&binding.key, &binding.value);
            }
        }
        for assert in &other.asserts {
            let _ = try_push(&mut self.asserts, assert.clone());
        }
        for (k, v) in &other.set_variables {
            if !self.set_variables.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.set_variables, (k.clone(), v.clone()));
            }
        }
        for (k, v) in &other.new_variable_instances {
            if !self.new_variable_instances.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.new_variable_instances, (k.clone(), v.clone()));
            }
        }
        for err in &other.assert_errors {
            let _ = try_push(&mut self.assert_errors, err.clone());
        }
        for (prefix, uri) in &other.in_scope_namespaces {
            if prefix.is_empty() {
                if !self.in_scope_namespaces.iter().any(|(p, _)| p.is_empty()) {
                    let _ = try_push(&mut self.in_scope_namespaces, (String::new(), uri.clone()));
                }
            } else if let Some(existing) = self.in_scope_namespaces.iter_mut().find(|(p, _)| p == prefix) {
                existing.1 = uri.clone();
            } else {
                let _ = try_push(&mut self.in_scope_namespaces, (prefix.clone(), uri.clone()));
            }
        }
        self.discriminator_count = self
            .discriminator_count
            .saturating_add(other.discriminator_count);
    }

    /// Overrides local properties, assertions, and setVariables with those from `other`.
    pub fn override_with(&mut self, other: &PropertyStore) {
        for binding in &other.bindings {
            if binding.key.starts_with("repValue:") || binding.key == "enumeration" {
                if !self.bindings.iter().any(|b| b.key == binding.key && b.value == binding.value) {
                    let _ = try_push(&mut self.bindings, binding.clone());
                }
            } else {
                let _ = self.set_property(&binding.key, &binding.value);
            }
        }
        for assert in &other.asserts {
            let _ = try_push(&mut self.asserts, assert.clone());
        }
        for (k, v) in &other.set_variables {
            if !self.set_variables.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.set_variables, (k.clone(), v.clone()));
            }
        }
        for (k, v) in &other.new_variable_instances {
            if !self.new_variable_instances.iter().any(|(name, _)| name == k) {
                let _ = try_push(&mut self.new_variable_instances, (k.clone(), v.clone()));
            }
        }
        for err in &other.assert_errors {
            let _ = try_push(&mut self.assert_errors, err.clone());
        }
        self.discriminator_count = self
            .discriminator_count
            .saturating_add(other.discriminator_count);
    }

    /// Returns reference to attached assert items.
    #[inline]
    #[must_use]
    pub fn asserts(&self) -> &[CompiledAssert] {
        &self.asserts
    }

    /// Returns true if any asserts are attached.
    #[inline]
    #[must_use]
    pub fn has_asserts(&self) -> bool {
        !self.asserts.is_empty()
    }

    /// Merges missing parent properties into this property store.
    pub fn merge(&mut self, parent: &PropertyStore) {
        self.merge_parent(parent);
    }

    /// Merges missing inheritable parent properties into this property store.
    pub fn merge_parent(&mut self, parent: &PropertyStore) {
        for binding in &parent.bindings {
            if binding.key == "inputValueCalc"
                || binding.key == "outputValueCalc"
                || binding.key == "initiator"
                || binding.key == "terminator"
                || binding.key == "separator"
                || binding.key == "separatorPosition"
                || binding.key == "separatorSuppressionPolicy"
                || binding.key == "sequenceKind"
                || binding.key == "initiatedContent"
                || binding.key == "hiddenGroupRef"
                || binding.key == "choiceDispatchKey"
                || binding.key == "choiceBranchKey"
                || binding.key == "choiceLengthKind"
                || binding.key == "choiceLength"
                || binding.key == "leadingSkip"
                || binding.key == "trailingSkip"
                || binding.key == "occursCount"
                || binding.key == "assert"
                || binding.key == "discriminator"
                || binding.key == "setVariable"
                || binding.key == "newVariableInstance"
                || binding.key == "testKind"
                || binding.key == "length"
                || binding.key == "lengthKind"
                || binding.key == "lengthUnits"
                || binding.key == "alignmentUnits"
                || binding.key == "occursCountKind"
                || binding.key == "fillByte"
                || binding.key == "prefixLengthType"
                || binding.key == "prefixIncludesPrefixLength"
                || binding.key == "representation"
                || binding.key == "layer"
                || binding.key == "layerTransform"
                || binding.key == "layerLength"
                || binding.key == "layerLengthUnits"
                || binding.key == "layerBoundaryMark"
                || binding.key == "repType"
                || binding.key == "repValues"
                || binding.key == "repValueRanges"
            {
                continue;
            }
            if self.get_property(&binding.key).is_none() {
                let _ = self.set_property(&binding.key, &binding.value);
            }
        }
    }

    /// Adds a DFDL assertion definition to this property store.
    pub fn add_assert(&mut self, test_expr: &str, message: Option<&str>) {
        self.add_assert_with_kind(crate::schema::ir::TestKind::Expression, test_expr, message);
    }

    /// Adds a DFDL assertion definition with explicit TestKind.
    pub fn add_assert_with_kind(
        &mut self,
        test_kind: crate::schema::ir::TestKind,
        test_expr: &str,
        message: Option<&str>,
    ) {
        self.add_assert_with_failure_type(
            test_kind,
            test_expr,
            message,
            crate::schema::ir::FailureType::ProcessingError,
        );
    }

    /// Adds a DFDL assertion definition with explicit TestKind and FailureType.
    pub fn add_assert_with_failure_type(
        &mut self,
        test_kind: crate::schema::ir::TestKind,
        test_expr: &str,
        message: Option<&str>,
        failure_type: crate::schema::ir::FailureType,
    ) {
        let test_ast = if test_kind == crate::schema::ir::TestKind::Expression {
            crate::expr::parse_expr(test_expr).ok()
        } else {
            None
        };
        let compiled_regex = if test_kind == crate::schema::ir::TestKind::Pattern {
            let anchored = if let Some(stripped) = test_expr.strip_prefix('^') {
                alloc::format!("^(?:{})", stripped)
            } else {
                alloc::format!("^(?:{})", test_expr)
            };
            crate::pattern::DfdlRegex::new(&anchored)
                .or_else(|_| crate::pattern::DfdlRegex::new(test_expr))
                .ok()
        } else {
            None
        };
        let _ = try_push(
            &mut self.asserts,
            CompiledAssert {
                test_kind,
                test_expr: test_expr.to_string(),
                test_ast,
                compiled_regex,
                message: message.map(String::from),
                failure_type,
            },
        );
    }

    /// Validates DFDL entity syntax and validity across all stored property bindings.
    pub fn validate_property_entities(&self) -> DFDLResult<()> {
        for binding in &self.bindings {
            validate_dfdl_property_entities(&binding.key, &binding.value)?;
        }
        Ok(())
    }

    /// Sets or overrides a property key-value binding.
    pub fn set_property(&mut self, key: &str, value: &str) -> DFDLResult<()> {
        let prop_name = PropertyKeyName::parse(key);
        for binding in &mut self.bindings {
            if binding.key == prop_name {
                binding.value = value.to_string();
                return Ok(());
            }
        }
        try_push(
            &mut self.bindings,
            PropertyBinding {
                key: prop_name,
                value: value.to_string(),
            },
        )
    }

    /// Sets or overrides a property using a strongly typed [`PropertyKey`].
    pub fn set_property_key(&mut self, key: PropertyKey, value: &str) -> DFDLResult<()> {
        for binding in &mut self.bindings {
            if let PropertyKeyName::Known(k) = binding.key {
                if k == key {
                    binding.value = value.to_string();
                    return Ok(());
                }
            }
        }
        try_push(
            &mut self.bindings,
            PropertyBinding {
                key: PropertyKeyName::Known(key),
                value: value.to_string(),
            },
        )
    }

    /// Removes a property binding by key (with or without `dfdl:` prefix).
    pub fn remove_property(&mut self, key: &str) {
        let prop_name = PropertyKeyName::parse(key);
        self.bindings.retain(|b| b.key != prop_name);
    }

    /// Removes a property binding by strongly typed [`PropertyKey`].
    pub fn remove_property_key(&mut self, key: PropertyKey) {
        self.bindings.retain(|b| b.key != PropertyKeyName::Known(key));
    }

    /// Adds an enumeration value facet to this property store.
    pub fn add_enumeration(&mut self, value: &str) -> DFDLResult<()> {
        for b in &self.bindings {
            if b.key == PropertyKeyName::Known(PropertyKey::Enumeration) && b.value == value {
                let msg = alloc::format!(
                    "Schema Definition Error: Enumerations must be unique: duplicate value '{}'",
                    value
                );
                return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
            }
        }
        try_push(
            &mut self.bindings,
            PropertyBinding {
                key: PropertyKeyName::Known(PropertyKey::Enumeration),
                value: value.to_string(),
            },
        )
    }

    /// Adds an enumeration representation value mapping for dfdlx:repType.
    pub fn add_enumeration_rep(&mut self, enum_val: &str, rep_val: &str) {
        let _ = try_push(
            &mut self.bindings,
            PropertyBinding {
                key: PropertyKeyName::Custom(alloc::format!("repValue:{}", enum_val)),
                value: rep_val.to_string(),
            },
        );
    }

    /// Retrieves an explicitly set local property value by key name.
    #[must_use]
    pub fn get_property(&self, key: &str) -> Option<&str> {
        let prop_name = PropertyKeyName::parse(key);
        for binding in &self.bindings {
            if binding.key == prop_name {
                return Some(&binding.value);
            }
        }
        None
    }

    /// Retrieves an explicitly set local property value by strongly typed [`PropertyKey`].
    #[must_use]
    pub fn get_property_key(&self, key: PropertyKey) -> Option<&str> {
        for binding in &self.bindings {
            if let PropertyKeyName::Known(k) = binding.key {
                if k == key {
                    return Some(&binding.value);
                }
            }
        }
        None
    }

    /// Returns a reference slice of all property bindings stored locally.
    #[must_use]
    pub fn bindings(&self) -> &[PropertyBinding] {
        &self.bindings
    }

    /// Resolves property value along inheritance chain (local -> parent -> default).
    #[must_use]
    pub fn resolve_property<'a>(
        &'a self,
        parent: Option<&'a PropertyStore>,
        key: &str,
        default_val: &'a str,
    ) -> &'a str {
        if let Some(val) = self.get_property(key) {
            return val;
        }
        if let Some(p) = parent {
            if let Some(val) = p.get_property(key) {
                return val;
            }
        }
        default_val
    }
}
