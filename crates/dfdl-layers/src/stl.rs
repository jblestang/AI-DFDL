//! Simple Test Layers (STL) and Daffodil conformance test suite extensions.
//!
//! Includes `stlBombOutLayer`, `stlOk1`, `stlOk2`, `stlOk3`, `stlOk4`, and `allTypesLayer`.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::kernel::layer::{Layer, LayerParseContext, LayerParseInput, LayerUnparseContext};
use dfdl_core::types::QName;

/// Test layer that intentionally raises errors at specified lifecycle hook locations.
#[derive(Default)]
pub struct StlBombOutLayer;

impl Layer for StlBombOutLayer {
    fn name(&self) -> &'static str {
        "stlBombOutLayer"
    }

    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case("stlbomboutlayer")
            || clean.eq_ignore_ascii_case("stlBombOutLayer")
    }

    fn parse_wrap(
        &self,
        _raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let bomb_where = ctx
            .variable_map
            .get_variable("bombWhere")
            .and_then(|v| match v {
                DfdlValue::String(s) => Some(s.clone()),
                _ => None,
            });
        let bomb_how = ctx
            .variable_map
            .get_variable("bombHow")
            .and_then(|v| match v {
                DfdlValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| String::from("PE"));

        if let Some(ref bw) = bomb_where {
            if bw == "setter" || bw == "getter" || bw == "read" || bw == "closeInput" || bw == "wrapInput" {
                if bomb_how.eq_ignore_ascii_case("RSDE") {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!("Runtime Schema Definition Error: Bombed out at {}", bw),
                    ));
                } else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Parse,
                        &alloc::format!("Parse Error: Bombed out at {}", bw),
                    ));
                }
            }
        }

        if let Some(DfdlValue::String(s)) = ctx
            .variable_map
            .get_variable("stringVar")
            .or_else(|| ctx.variable_map.get_variable("stringVarIn"))
        {
            let doubled = alloc::format!("{} {}", s, s);
            let _ = ctx.variable_map.set_variable_validated(
                &QName::with_namespace("urn:STL", "stringVar", None),
                DfdlValue::String(doubled),
                true,
            );
        }

        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        let bomb_where = ctx
            .variable_map
            .get_variable("bombWhere")
            .and_then(|v| match v {
                DfdlValue::String(s) => Some(s.clone()),
                _ => None,
            });
        let bomb_how = ctx
            .variable_map
            .get_variable("bombHow")
            .and_then(|v| match v {
                DfdlValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_else(|| String::from("PE"));

        if let Some(ref bw) = bomb_where {
            if bw == "closeOutput" || bw == "write" || bw == "wrapOutput" || bw == "getter" || bw == "setter" {
                if bomb_how.eq_ignore_ascii_case("RSDE") {
                    return Err(DFDLError::new(
                        DFDLErrorKind::SchemaDefinition,
                        &alloc::format!("Runtime Schema Definition Error: Bombed out at {}", bw),
                    ));
                } else {
                    return Err(DFDLError::new(
                        DFDLErrorKind::Unparse,
                        &alloc::format!("Unparse Error: Bombed out at {}", bw),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Simple test layer 1.
#[derive(Default)]
pub struct StlOk1Layer;

impl Layer for StlOk1Layer {
    fn name(&self) -> &'static str {
        "stlOk1"
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        Ok(())
    }
}

/// Simple test layer 2.
#[derive(Default)]
pub struct StlOk2Layer;

impl Layer for StlOk2Layer {
    fn name(&self) -> &'static str {
        "stlOk2"
    }

    fn parse_wrap(
        &self,
        _raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let _ = ctx.variable_map.set_variable_validated(
            &QName::with_namespace("urn:STL", "intVar", None),
            DfdlValue::Int(84),
            true,
        );
        let _ = ctx.variable_map.set_variable_validated(
            &QName::with_namespace("urn:STL", "stringVar", None),
            DfdlValue::String(String::from("forty two forty two")),
            true,
        );
        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        Ok(())
    }
}

/// Simple test layer 3.
#[derive(Default)]
pub struct StlOk3Layer;

impl Layer for StlOk3Layer {
    fn name(&self) -> &'static str {
        "stlOk3"
    }

    fn parse_wrap(
        &self,
        _raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let _ = ctx.variable_map.set_variable_validated(
            &QName::with_namespace("urn:STL", "stringVar", None),
            DfdlValue::String(String::from("forty two forty two")),
            true,
        );
        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        Ok(())
    }
}

/// Simple test layer 4.
#[derive(Default)]
pub struct StlOk4Layer;

impl Layer for StlOk4Layer {
    fn name(&self) -> &'static str {
        "stlOk4"
    }

    fn parse_wrap(
        &self,
        _raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let _ = ctx.variable_map.set_variable_validated(
            &QName::with_namespace("urn:STL", "intVar", None),
            DfdlValue::Int(84),
            true,
        );
        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        Ok(())
    }
}

/// Test layer setting variables of all primitive types.
#[derive(Default)]
pub struct AllTypesLayer;

impl Layer for AllTypesLayer {
    fn name(&self) -> &'static str {
        "allTypesLayer"
    }

    fn parse_wrap(
        &self,
        _raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let ns = "urn:org.apache.daffodil.layers.xsd.AllTypesLayer";
        let vm = &mut ctx.variable_map;
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "b2", None), DfdlValue::Byte(84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "ub2", None), DfdlValue::UnsignedByte(84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "s2", None), DfdlValue::Short(-84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "us2", None), DfdlValue::UnsignedShort(84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "i2", None), DfdlValue::Int(84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "ui2", None), DfdlValue::UnsignedInt(84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "l2", None), DfdlValue::Long(-84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "ul2", None), DfdlValue::UnsignedLong(84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "integer2", None), DfdlValue::Long(-84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "nni2", None), DfdlValue::UnsignedLong(84), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "dec2", None), DfdlValue::Decimal(String::from("-84.84")), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "f2", None), DfdlValue::Float(0.0084), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "d2", None), DfdlValue::Double(-8.4E143), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "string2", None), DfdlValue::String(String::from("fortyTwo fortyTwo")), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "hex2", None), DfdlValue::HexBinary(alloc::vec![0x2a, 0x2a]), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "date2", None), DfdlValue::Date(String::from("1942-04-03")), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "time2", None), DfdlValue::Time(String::from("05:42:42")), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "dt2", None), DfdlValue::DateTime(String::from("1942-04-03T04:42:42")), true);
        let _ = vm.set_variable_validated(&QName::with_namespace(ns, "bool2", None), DfdlValue::Boolean(false), true);

        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    fn unparse_transform(
        &self,
        _buffer: &mut Vec<u8>,
        _ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()> {
        Ok(())
    }
}
