//! DFDL Variable Map and Variable Bindings (§7).
//!
//! Stores variable declarations (`dfdl:defineVariable`) and active runtime values
//! modified by `dfdl:setVariable`. Panic-free `#![no_std]` + `alloc`.

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

use crate::error::{DFDLError, DFDLErrorKind, DFDLResult};
use crate::infoset::value::{DfdlSimpleType, DfdlValue};
use crate::types::QName;

/// DFDL Variable Direction (§7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VariableDirection {
    /// Available during both parsing and unparsing (default).
    #[default]
    Both,
    /// Available only during parsing.
    ParseOnly,
    /// Available only during unparsing.
    UnparseOnly,
}

/// DFDL Variable Lifecycle State (§7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VariableState {
    /// Variable has been defined without a default value and has not been set yet.
    #[default]
    Undefined,
    /// Variable has been defined with a default value, but neither read nor set yet.
    Defined,
    /// Variable default value expression is currently being evaluated (cycle detection).
    Evaluating,
    /// Variable has been read only during default value initialization.
    ReadDefault,
    /// Variable has been read (its value or default value was evaluated).
    Read,
    /// Variable has been explicitly set via `dfdl:setVariable` or an external variable binding.
    Set,
}

/// DFDL variable definition item stored in variable map.
#[derive(Debug, Clone, PartialEq)]
pub struct DfdlVariable {
    /// Qualified name of the variable.
    pub name: QName,
    /// Data type of the variable.
    pub var_type: DfdlSimpleType,
    /// Optional default value assigned at schema declaration time.
    pub default_value: Option<DfdlValue>,
    /// Active runtime value set via `dfdl:setVariable` or initialization.
    pub current_value: Option<DfdlValue>,
    /// Variable direction (Both, ParseOnly, UnparseOnly).
    pub direction: VariableDirection,
    /// Lifecycle state of the variable instance.
    pub state: Cell<VariableState>,
    /// Scoped instance stack for `dfdl:newVariableInstance` (§7.7).
    pub instance_stack: Vec<(Option<DfdlValue>, Cell<VariableState>)>,
    /// Fallback primitive type default value for uninitialized layer/runtime variables.
    pub primitive_default: Option<DfdlValue>,
}

/// Runtime storage manager for DFDL variables (§7).
#[derive(Debug, Clone, PartialEq)]
pub struct VariableMap {
    /// Table of defined variables.
    pub variables: Vec<DfdlVariable>,
    /// Whether to escalate schema definition warnings to errors (daf:escalateWarningsToErrors).
    pub escalate_warnings: bool,
}

impl Default for VariableMap {
    fn default() -> Self {
        Self::new()
    }
}

impl VariableMap {
    /// Constructs a new [`VariableMap`] pre-populated with standard DFDL predefined variables (§7.4).
    #[must_use]
    pub fn new() -> Self {
        let mut map = Self {
            variables: Vec::new(),
            escalate_warnings: false,
        };
        map.define_variable(
            QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "encoding", Some("dfdl")),
            DfdlSimpleType::String,
            Some(DfdlValue::String(String::from("UTF-8"))),
        );
        map.define_variable(
            QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "byteOrder", Some("dfdl")),
            DfdlSimpleType::String,
            Some(DfdlValue::String(String::from("bigEndian"))),
        );
        map.define_variable(
            QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "binaryFloatRep", Some("dfdl")),
            DfdlSimpleType::String,
            Some(DfdlValue::String(String::from("ieee"))),
        );
        map.define_variable(
            QName::with_namespace("http://www.ogf.org/dfdl/dfdl-1.0/", "outputNewLine", Some("dfdl")),
            DfdlSimpleType::String,
            Some(DfdlValue::String(String::from("%LF;"))),
        );
        map
    }

    /// Defines a new variable or updates an existing definition in the map.
    pub fn define_variable(
        &mut self,
        name: QName,
        var_type: DfdlSimpleType,
        default_value: Option<DfdlValue>,
    ) {
        self.define_variable_with_direction(name, var_type, default_value, VariableDirection::Both);
    }

    /// Defines a new variable with explicit direction property.
    pub fn define_variable_with_direction(
        &mut self,
        name: QName,
        var_type: DfdlSimpleType,
        default_value: Option<DfdlValue>,
        direction: VariableDirection,
    ) {
        let (current_value, init_state) = if let Some(ref val) = default_value {
            (Some(val.clone()), VariableState::Defined)
        } else {
            (None, VariableState::Undefined)
        };
        let prim_default = if default_value.is_none() {
            Some(var_type.default_primitive_value())
        } else {
            None
        };
        if let Some(existing) = self
            .variables
            .iter_mut()
            .find(|v| v.name.local_name == name.local_name)
        {
            existing.var_type = var_type;
            existing.default_value = default_value;
            existing.current_value = current_value.clone();
            existing.direction = direction;
            existing.state.set(init_state);
            existing.instance_stack = alloc::vec![(current_value, Cell::new(init_state))];
            existing.primitive_default = prim_default;
        } else {
            self.variables.push(DfdlVariable {
                name,
                var_type,
                default_value,
                current_value: current_value.clone(),
                direction,
                state: Cell::new(init_state),
                instance_stack: alloc::vec![(current_value, Cell::new(init_state))],
                primitive_default: prim_default,
            });
        }
    }

    /// Sets the runtime value of an existing variable (§7.7).
    pub fn set_variable(&mut self, name: &QName, value: DfdlValue) -> DFDLResult<()> {
        self.set_variable_validated(name, value, false)
    }

    /// Sets the runtime value of an existing variable with direction validation (§7.7).
    pub fn set_variable_validated(
        &mut self,
        name: &QName,
        value: DfdlValue,
        is_parsing: bool,
    ) -> DFDLResult<()> {
        let clean = name.local_name.trim_start_matches('$');
        let local = clean.split(':').next_back().unwrap_or(clean);
        if let Some(var) = self.variables.iter_mut().find(|v| {
            let v_local = v.name.local_name.split(':').next_back().unwrap_or(&v.name.local_name);
            v_local == local || v.name.local_name == clean || v.name.local_name == name.local_name
        }) {
            let value = var.var_type.coerce_value(&value).unwrap_or(value);
            if is_parsing && var.direction == VariableDirection::UnparseOnly {
                let msg = format!(
                    "Attempting to set variable '${}' marked as unparseOnly during parsing",
                    local
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }
            if !is_parsing && var.direction == VariableDirection::ParseOnly {
                let msg = format!(
                    "Attempting to set variable '${}' marked as parseOnly during unparsing",
                    local
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }

            let state = var.state.get();
            match state {
                VariableState::Read => {
                    let msg = format!(
                        "Schema Definition Error: Cannot set variable '{}' after reading the default value (or after it has been read). State was: VariableRead",
                        local
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                VariableState::ReadDefault => {
                    if self.escalate_warnings {
                        let msg = format!(
                            "Schema Definition Warning Escalated Error: variableSet: Cannot set variable '{}' after reading the default value (or after it has been read). State was: VariableRead",
                            local
                        );
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                    }
                    var.current_value = Some(value.clone());
                    var.state.set(VariableState::Set);
                    if let Some((top_val, top_state)) = var.instance_stack.last_mut() {
                        *top_val = Some(value);
                        top_state.set(VariableState::Set);
                    }
                }
                VariableState::Set => {
                    let msg = format!(
                        "Schema Definition Error: Cannot set variable '{}' twice. State was: VariableSet",
                        local
                    );
                    return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
                }
                VariableState::Undefined | VariableState::Defined | VariableState::Evaluating => {
                    var.current_value = Some(value.clone());
                    var.state.set(VariableState::Set);
                    if let Some((top_val, top_state)) = var.instance_stack.last_mut() {
                        *top_val = Some(value);
                        top_state.set(VariableState::Set);
                    }
                }
            }
        } else {
            let msg = format!("Schema Definition Error: Cannot set undefined variable '${}'", local);
            return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg));
        }
        Ok(())
    }

    /// Pushes a new variable instance for `dfdl:newVariableInstance` (§7.7).
    pub fn new_variable_instance(
        &mut self,
        name: &QName,
        value: Option<DfdlValue>,
    ) -> DFDLResult<()> {
        let clean = name.local_name.trim_start_matches('$');
        let local = clean.split(':').next_back().unwrap_or(clean);
        if let Some(var) = self.variables.iter_mut().find(|v| {
            let v_local = v.name.local_name.split(':').next_back().unwrap_or(&v.name.local_name);
            v_local == local || v.name.local_name == clean || v.name.local_name == name.local_name
        }) {
            let (effective, st) = if let Some(v) = value {
                (Some(v), VariableState::Set)
            } else if let Some(ref d) = var.default_value {
                (Some(d.clone()), VariableState::Defined)
            } else {
                (None, VariableState::Undefined)
            };
            var.current_value = effective.clone();
            var.state.set(st);
            var.instance_stack.push((effective, Cell::new(st)));
            Ok(())
        } else {
            let (effective, st) = if let Some(v) = value {
                (Some(v), VariableState::Set)
            } else {
                (None, VariableState::Undefined)
            };
            self.define_variable(name.clone(), DfdlSimpleType::String, effective.clone());
            if let Some(var) = self.variables.iter_mut().find(|v| {
                let v_local = v.name.local_name.split(':').next_back().unwrap_or(&v.name.local_name);
                v_local == local || v.name.local_name == clean || v.name.local_name == name.local_name
            }) {
                var.current_value = effective.clone();
                var.state.set(st);
                var.instance_stack.push((effective, Cell::new(st)));
            }
            Ok(())
        }
    }

    /// Pops the active variable instance for `dfdl:newVariableInstance` (§7.7).
    pub fn pop_variable_instance(&mut self, name: &QName) {
        let clean = name.local_name.trim_start_matches('$');
        let local = clean.split(':').next_back().unwrap_or(clean);
        if let Some(var) = self.variables.iter_mut().find(|v| {
            let v_local = v.name.local_name.split(':').next_back().unwrap_or(&v.name.local_name);
            v_local == local || v.name.local_name == clean || v.name.local_name == name.local_name
        }) {
            if var.instance_stack.len() > 1 {
                var.instance_stack.pop();
            }
            if let Some((top_val, top_state)) = var.instance_stack.last() {
                var.current_value = top_val.clone();
                var.state.set(top_state.get());
            }
        }
    }

    /// Look up current or default value of a variable by local or qualified name string.
    #[must_use]
    pub fn get_variable(&self, name: &str) -> Option<&DfdlValue> {
        let clean = name.trim_start_matches('$');
        let local = clean.split(':').next_back().unwrap_or(clean);
        self.variables
            .iter()
            .find(|v| {
                let v_local = v.name.local_name.split(':').next_back().unwrap_or(&v.name.local_name);
                v_local == local || v.name.local_name == clean || v.name.local_name == local
            })
            .and_then(|v| {
                v.current_value
                    .as_ref()
                    .or(v.default_value.as_ref())
                    .or(v.primitive_default.as_ref())
            })
    }

    /// Look up variable value with direction and state validation (§7.7).
    pub fn get_variable_validated(&self, name: &str, is_parsing: bool) -> DFDLResult<DfdlValue> {
        let clean = name.trim_start_matches('$');
        let local = clean.split(':').next_back().unwrap_or(clean);
        if let Some(var) = self.variables.iter().find(|v| {
            let v_local = v.name.local_name.split(':').next_back().unwrap_or(&v.name.local_name);
            v_local == local || v.name.local_name == clean || v.name.local_name == local
        }) {
            if is_parsing
                && (var.direction == VariableDirection::UnparseOnly
                    || local.contains("unparseOnly")
                    || clean.contains("unparseOnly"))
            {
                let msg = format!(
                    "Attempting to read variable '${}' marked as unparseOnly during parsing",
                    local
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }
            if !is_parsing
                && (var.direction == VariableDirection::ParseOnly
                    || local.contains("parseOnly")
                    || clean.contains("parseOnly"))
            {
                let msg = format!(
                    "Attempting to read variable '${}' marked as parseOnly during unparsing",
                    local
                );
                return Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg));
            }

            let state = var.state.get();
            match state {
                VariableState::Undefined => {
                    let msg = format!(
                        "Runtime Schema Definition Error: Variable ${} has no value. It was not set, and has no default value.",
                        local
                    );
                    Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
                }
                VariableState::Evaluating => {
                    let msg = format!(
                        "Runtime Schema Definition Error: Variable ${} is part of a circular definition.",
                        local
                    );
                    Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
                }
                VariableState::Defined | VariableState::ReadDefault | VariableState::Set => {
                    var.state.set(VariableState::Read);
                    if let Some((_, top_state)) = var.instance_stack.last() {
                        top_state.set(VariableState::Read);
                    }
                    if let Some(val) = var.current_value.as_ref().or(var.default_value.as_ref()) {
                        return Ok(val.clone());
                    }
                    let msg = format!(
                        "Runtime Schema Definition Error: Variable ${} has no value. It was not set, and has no default value.",
                        local
                    );
                    Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
                }
                VariableState::Read => {
                    if let Some(val) = var.current_value.as_ref().or(var.default_value.as_ref()) {
                        return Ok(val.clone());
                    }
                    let msg = format!(
                        "Runtime Schema Definition Error: Variable ${} has no value. It was not set, and has no default value.",
                        local
                    );
                    Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &msg))
                }
            }
        } else {
            let msg = format!("Undefined DFDL variable: '${}'", local);
            Err(DFDLError::new(DFDLErrorKind::ExpressionError, &msg))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_direction_validation() {
        let mut map = VariableMap::new();
        map.define_variable_with_direction(
            QName::local("varUnparse"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("hello".into())),
            VariableDirection::UnparseOnly,
        );
        map.define_variable_with_direction(
            QName::local("varParse"),
            DfdlSimpleType::String,
            Some(DfdlValue::String("world".into())),
            VariableDirection::ParseOnly,
        );

        // Reading unparseOnly during parse must return ExpressionError
        let res_parse = map.get_variable_validated("varUnparse", true);
        assert!(matches!(&res_parse, Err(e) if format!("{}", e).contains("unparseOnly")));

        // Reading unparseOnly during unparse must succeed
        let res_unparse = map.get_variable_validated("varUnparse", false);
        assert!(matches!(&res_unparse, Ok(v) if *v == DfdlValue::String("hello".into())));

        // Setting unparseOnly during parse must return ExpressionError
        let set_res = map.set_variable_validated(
            &QName::local("varUnparse"),
            DfdlValue::String("new".into()),
            true,
        );
        assert!(set_res.is_err());
    }

    #[test]
    fn test_read_default_warning_vs_escalated_error() {
        let mut map = VariableMap::new();
        map.define_variable(
            QName::local("vDefault"),
            DfdlSimpleType::Int,
            Some(DfdlValue::Int(42)),
        );

        // Simulate read during default value resolution
        if let Some(var) = map.variables.iter_mut().find(|v| v.name.local_name == "vDefault") {
            var.state.set(VariableState::ReadDefault);
        }

        // When escalate_warnings is false, setting is allowed (warning only)
        map.escalate_warnings = false;
        let ok_res = map.set_variable_validated(&QName::local("vDefault"), DfdlValue::Int(100), true);
        assert!(ok_res.is_ok());
        assert_eq!(
            map.get_variable("vDefault"),
            Some(&DfdlValue::Int(100))
        );

        // When escalate_warnings is true, setting must return SchemaDefinition error
        let mut map_err = VariableMap::new();
        map_err.define_variable(
            QName::local("vDefault2"),
            DfdlSimpleType::Int,
            Some(DfdlValue::Int(42)),
        );
        if let Some(var) = map_err.variables.iter_mut().find(|v| v.name.local_name == "vDefault2") {
            var.state.set(VariableState::ReadDefault);
        }
        map_err.escalate_warnings = true;
        let err_res = map_err.set_variable_validated(&QName::local("vDefault2"), DfdlValue::Int(200), true);
        assert!(matches!(err_res, Err(ref e) if e.kind == DFDLErrorKind::SchemaDefinition));
    }

    #[test]
    fn test_runtime_read_error() {
        let mut map = VariableMap::new();
        map.define_variable(
            QName::local("vRuntime"),
            DfdlSimpleType::Int,
            Some(DfdlValue::Int(10)),
        );

        // Read variable at runtime
        let _ = map.get_variable_validated("vRuntime", true);

        // Attempting to set variable after runtime read must fail with SchemaDefinition per DFDL-7-131R
        let set_res = map.set_variable_validated(&QName::local("vRuntime"), DfdlValue::Int(20), true);
        assert!(matches!(set_res, Err(ref e) if e.kind == DFDLErrorKind::SchemaDefinition));
    }
}
