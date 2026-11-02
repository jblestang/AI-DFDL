//! DFDL Layer Extension Trait and Registry.
//!
//! Provides the core abstraction for DFDL layer extensions (§dfdlx:layer),
//! allowing external and standard layers to hook into the parsing and unparsing pipelines
//! without hardcoded knowledge of specific layers in the core engine.

extern crate alloc;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::error::DFDLResult;
use crate::expr::variables::VariableMap;

/// Context provided to a layer during parsing.
pub struct LayerParseContext<'a> {
    /// Active variable map for the current sequence scope.
    pub variable_map: &'a mut VariableMap,
    /// Current bit limit of the reader, if any.
    pub bit_limit: Option<usize>,
    /// Starting bit position in the reader before layer processing.
    pub current_position: usize,
}

/// Result of a layer's stream preparation hook during parsing.
pub enum LayerParseInput {
    /// Transformed stream: replaces the input data with transformed bytes for the inner sequence.
    Transformed {
        /// Decompressed, decoded, or unfolded bytes to be parsed by the inner sequence.
        data: Vec<u8>,
        /// Optional mapping from sub-reader consumed byte offset to original stream consumed byte offset.
        orig_byte_offsets: Option<Vec<usize>>,
        /// If specified, skips exactly this many original input bytes upon success.
        orig_consumed_bytes: Option<usize>,
    },
    /// In-place stream: layer inspects input bytes or sets variables without replacing the stream.
    InPlace {
        /// Relative bit limit to apply to the sequence from the current position.
        bit_limit: Option<usize>,
        /// Number of bits to skip after the sequence parsing completes (e.g. boundary marks).
        post_skip_bits: Option<usize>,
    },
}

/// Context provided to a layer during unparsing.
pub struct LayerUnparseContext<'a> {
    /// Active variable map for the current sequence scope.
    pub variable_map: &'a mut VariableMap,
}

/// Trait defining a DFDL Layer extension.
pub trait Layer: Send + Sync {
    /// Canonical name of the layer.
    fn name(&self) -> &'static str;

    /// Checks if this layer matches the specified layer name or QName.
    fn matches_name(&self, name: &str) -> bool {
        let clean = name.split(':').next_back().unwrap_or(name);
        clean.eq_ignore_ascii_case(self.name())
    }

    /// Pre-parse hook: inspects raw input bytes, sets variables, or transforms input bytes.
    fn parse_wrap(
        &self,
        raw_bytes: &[u8],
        ctx: &mut LayerParseContext<'_>,
    ) -> DFDLResult<LayerParseInput> {
        let _ = (raw_bytes, ctx);
        Ok(LayerParseInput::InPlace {
            bit_limit: None,
            post_skip_bits: None,
        })
    }

    /// Post-parse hook: called after the inner sequence has parsed successfully.
    fn post_parse(&self, ctx: &mut LayerParseContext<'_>) -> DFDLResult<()> {
        let _ = ctx;
        Ok(())
    }

    /// Unparse transform hook: transforms or modifies the unparsed sequence buffer before writing.
    fn unparse_transform(
        &self,
        buffer: &mut Vec<u8>,
        ctx: &mut LayerUnparseContext<'_>,
    ) -> DFDLResult<()>;
}

/// Thread-safe registry of registered DFDL layers.
#[derive(Default)]
pub struct LayerRegistry {
    layers: Vec<Arc<dyn Layer>>,
}

impl LayerRegistry {
    /// Creates a new empty [`LayerRegistry`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
        }
    }

    /// Registers a layer in this registry.
    pub fn register(&mut self, layer: Arc<dyn Layer>) {
        self.layers.push(layer);
    }

    /// Finds a registered layer matching the given name or QName.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<dyn Layer>> {
        self.layers.iter().find(|l| l.matches_name(name)).cloned()
    }

    /// Returns `true` if a layer matching the given name is registered.
    #[must_use]
    pub fn is_known_layer(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

static GLOBAL_REGISTRY: AtomicPtr<LayerRegistry> = AtomicPtr::new(core::ptr::null_mut());

type LayerInitFn = fn(&mut LayerRegistry);
static DEFAULT_INIT_FN: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Sets a fallback initialization function called when [`get_global_registry`] is first accessed.
pub fn set_default_layer_init(init_fn: LayerInitFn) {
    DEFAULT_INIT_FN.store(init_fn as *mut (), Ordering::Release);
}

/// Sets the global layer registry used by default by parser, unparser, and compiler.
pub fn set_global_registry(registry: LayerRegistry) {
    let boxed = Box::new(registry);
    let raw = Box::into_raw(boxed);
    let old = GLOBAL_REGISTRY.swap(raw, Ordering::AcqRel);
    if !old.is_null() {
        unsafe {
            let _ = Box::from_raw(old);
        }
    }
}

/// Returns a reference to the global layer registry if initialized.
#[must_use]
pub fn get_global_registry() -> Option<&'static LayerRegistry> {
    let mut ptr = GLOBAL_REGISTRY.load(Ordering::Acquire);
    if ptr.is_null() {
        let init_ptr = DEFAULT_INIT_FN.load(Ordering::Acquire);
        if !init_ptr.is_null() {
            let init_fn: LayerInitFn = unsafe { core::mem::transmute(init_ptr) };
            let mut reg = LayerRegistry::new();
            init_fn(&mut reg);
            set_global_registry(reg);
            ptr = GLOBAL_REGISTRY.load(Ordering::Acquire);
        }
    }
    if ptr.is_null() {
        #[cfg(test)]
        {
            init_test_layers();
            ptr = GLOBAL_REGISTRY.load(Ordering::Acquire);
        }
    }
    if ptr.is_null() {
        None
    } else {
        unsafe { Some(&*ptr) }
    }
}

#[cfg(test)]
/// Initializes test mock layers when running internal dfdl-core tests.
pub fn init_test_layers() {
    let mut reg = LayerRegistry::new();
    reg.register(Arc::new(test_mocks::MockTwoByteSwap));
    reg.register(Arc::new(test_mocks::MockBombOutLayer));
    reg.register(Arc::new(test_mocks::MockBoundaryMarkLayer));
    reg.register(Arc::new(test_mocks::MockCheckDigitLayer));
    reg.register(Arc::new(test_mocks::MockIpv4Layer));
    reg.register(Arc::new(test_mocks::MockGzipLayer));
    reg.register(Arc::new(test_mocks::MockBase64Layer));
    reg.register(Arc::new(test_mocks::MockStlOk1));
    set_global_registry(reg);
}

#[cfg(test)]
mod test_mocks {
    use super::*;
    use crate::error::{DFDLError, DFDLErrorKind};
    use crate::infoset::value::DfdlValue;

    pub struct MockTwoByteSwap;
    impl Layer for MockTwoByteSwap {
        fn name(&self) -> &'static str { "twoByteSwap" }
        fn parse_wrap(&self, raw_bytes: &[u8], ctx: &mut LayerParseContext<'_>) -> DFDLResult<LayerParseInput> {
            let req = ctx.variable_map.get_variable("requireLengthInWholeWords")
                .and_then(|v| match v { DfdlValue::String(s) => Some(s.as_str()), _ => None });
            if req == Some("yes") && raw_bytes.len() % 2 != 0 {
                return Err(DFDLError::new(DFDLErrorKind::Parse, "Data length is not a multiple of 2 for twoByteSwap layer"));
            }
            Ok(LayerParseInput::InPlace { bit_limit: None, post_skip_bits: None })
        }
        fn unparse_transform(&self, buffer: &mut Vec<u8>, ctx: &mut LayerUnparseContext<'_>) -> DFDLResult<()> {
            let req = ctx.variable_map.get_variable("requireLengthInWholeWords")
                .and_then(|v| match v { DfdlValue::String(s) => Some(s.as_str()), _ => None });
            if req == Some("yes") && buffer.len() % 2 != 0 {
                return Err(DFDLError::new(DFDLErrorKind::Unparse, "Data length is not a multiple of 2 for twoByteSwap layer"));
            }
            for chunk in buffer.chunks_exact_mut(2) {
                chunk.swap(0, 1);
            }
            Ok(())
        }
    }

    pub struct MockBombOutLayer;
    impl Layer for MockBombOutLayer {
        fn name(&self) -> &'static str { "stlBombOutLayer" }
        fn parse_wrap(&self, _raw: &[u8], ctx: &mut LayerParseContext<'_>) -> DFDLResult<LayerParseInput> {
            let bw = ctx.variable_map.get_variable("bombWhere")
                .and_then(|v| match v { DfdlValue::String(s) => Some(s.clone()), _ => None });
            let bh = ctx.variable_map.get_variable("bombHow")
                .and_then(|v| match v { DfdlValue::String(s) => Some(s.clone()), _ => None })
                .unwrap_or_else(|| alloc::string::String::from("PE"));
            if let Some(ref w) = bw {
                if w == "read" {
                    if bh.eq_ignore_ascii_case("RSDE") {
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &alloc::format!("Runtime Schema Definition Error: Bombed out at {}", w)));
                    } else {
                        return Err(DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parse Error: Bombed out at {}", w)));
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
                    &crate::types::QName::with_namespace("urn:STL", "stringVar", None),
                    DfdlValue::String(doubled),
                    true,
                );
            }
            Ok(LayerParseInput::InPlace { bit_limit: None, post_skip_bits: None })
        }
        fn unparse_transform(&self, _buf: &mut Vec<u8>, ctx: &mut LayerUnparseContext<'_>) -> DFDLResult<()> {
            let bw = ctx.variable_map.get_variable("bombWhere")
                .and_then(|v| match v { DfdlValue::String(s) => Some(s.clone()), _ => None });
            let bh = ctx.variable_map.get_variable("bombHow")
                .and_then(|v| match v { DfdlValue::String(s) => Some(s.clone()), _ => None })
                .unwrap_or_else(|| alloc::string::String::from("PE"));
            if let Some(ref w) = bw {
                if w == "write" {
                    if bh.eq_ignore_ascii_case("RSDE") {
                        return Err(DFDLError::new(DFDLErrorKind::SchemaDefinition, &alloc::format!("Runtime Schema Definition Error: Bombed out at {}", w)));
                    } else {
                        return Err(DFDLError::new(DFDLErrorKind::Unparse, &alloc::format!("Unparse Error: Bombed out at {}", w)));
                    }
                }
            }
            Ok(())
        }
    }

    pub struct MockStlOk1;
    impl Layer for MockStlOk1 {
        fn name(&self) -> &'static str { "stlOk1" }
        fn unparse_transform(&self, _buf: &mut Vec<u8>, _ctx: &mut LayerUnparseContext<'_>) -> DFDLResult<()> {
            Ok(())
        }
    }

    pub struct MockBoundaryMarkLayer;
    impl Layer for MockBoundaryMarkLayer {
        fn name(&self) -> &'static str { "boundaryMark" }
        fn parse_wrap(&self, raw: &[u8], ctx: &mut LayerParseContext<'_>) -> DFDLResult<LayerParseInput> {
            let boundary_mark = ctx
                .variable_map
                .get_variable("boundaryMark")
                .and_then(|v| match v {
                    DfdlValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| alloc::string::String::from("//"));
            let mark_bytes = boundary_mark.as_bytes();
            if let Some(pos) = raw.windows(mark_bytes.len()).position(|w| w == mark_bytes) {
                return Ok(LayerParseInput::InPlace {
                    bit_limit: Some(pos.saturating_mul(8)),
                    post_skip_bits: Some(mark_bytes.len().saturating_mul(8)),
                });
            }
            Ok(LayerParseInput::InPlace { bit_limit: None, post_skip_bits: None })
        }
        fn unparse_transform(&self, buf: &mut Vec<u8>, ctx: &mut LayerUnparseContext<'_>) -> DFDLResult<()> {
            let mark = ctx.variable_map.get_variable("boundaryMark")
                .and_then(|v| match v { DfdlValue::String(s) => Some(s.as_bytes().to_vec()), _ => None })
                .unwrap_or_else(|| alloc::vec![b'#', b'#']);
            buf.extend_from_slice(&mark);
            Ok(())
        }
    }

    pub struct MockCheckDigitLayer;
    impl Layer for MockCheckDigitLayer {
        fn name(&self) -> &'static str { "checkDigit" }
        fn parse_wrap(&self, raw: &[u8], ctx: &mut LayerParseContext<'_>) -> DFDLResult<LayerParseInput> {
            let layer_len = ctx.variable_map.get_variable("length")
                .and_then(|v| match v {
                    DfdlValue::Short(s) if *s > 0 => Some(*s as usize),
                    DfdlValue::Int(i) if *i > 0 => Some(*i as usize),
                    DfdlValue::Long(l) if *l > 0 => Some(*l as usize),
                    DfdlValue::UnsignedShort(s) if *s > 0 => Some(*s as usize),
                    DfdlValue::UnsignedInt(i) if *i > 0 => Some(*i as usize),
                    _ => None,
                })
                .unwrap_or(10);
            if raw.len() < layer_len {
                return Err(DFDLError::new(DFDLErrorKind::Parse, &alloc::format!("Parse Error: Insufficient data for checkDigit layer: expected {} bytes", layer_len)));
            }
            Ok(LayerParseInput::InPlace { bit_limit: None, post_skip_bits: None })
        }
        fn unparse_transform(&self, _buf: &mut Vec<u8>, _ctx: &mut LayerUnparseContext<'_>) -> DFDLResult<()> {
            Ok(())
        }
    }

    pub struct MockIpv4Layer;
    impl Layer for MockIpv4Layer {
        fn name(&self) -> &'static str { "IPv4Checksum" }
        fn parse_wrap(&self, raw: &[u8], _ctx: &mut LayerParseContext<'_>) -> DFDLResult<LayerParseInput> {
            if raw.len() < 20 {
                return Err(DFDLError::new(DFDLErrorKind::Parse, "Parse Error: Insufficient data for IPv4 layer"));
            }
            Ok(LayerParseInput::InPlace { bit_limit: None, post_skip_bits: None })
        }
        fn unparse_transform(&self, _buf: &mut Vec<u8>, _ctx: &mut LayerUnparseContext<'_>) -> DFDLResult<()> {
            Ok(())
        }
    }

    pub struct MockGzipLayer;
    impl Layer for MockGzipLayer {
        fn name(&self) -> &'static str { "gzip" }
        fn unparse_transform(&self, _buf: &mut Vec<u8>, _ctx: &mut LayerUnparseContext<'_>) -> DFDLResult<()> {
            Ok(())
        }
    }

    pub struct MockBase64Layer;
    impl Layer for MockBase64Layer {
        fn name(&self) -> &'static str { "base64_mime" }
        fn unparse_transform(&self, _buf: &mut Vec<u8>, _ctx: &mut LayerUnparseContext<'_>) -> DFDLResult<()> {
            Ok(())
        }
    }
}

/// Finds a registered layer by name from the global registry.
#[must_use]
pub fn find_layer(name: &str) -> Option<Arc<dyn Layer>> {
    get_global_registry().and_then(|r| r.get(name))
}

/// Checks whether a layer is registered in the global registry.
#[must_use]
pub fn is_known_layer(name: &str) -> bool {
    get_global_registry().is_some_and(|r| r.is_known_layer(name))
}
