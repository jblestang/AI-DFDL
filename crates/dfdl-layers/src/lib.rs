//! DFDL standard and extension layer implementations.
//!
//! Provides isolated implementations for all known DFDL layer extensions:
//! - Byte swapping (`twoByteSwap`, `fourByteSwap`, `byteSwap`)
//! - RFC 1952 GZIP compression (`gzip`)
//! - RFC 2045 MIME Base64 (`base64_MIME`)
//! - RFC 2822 / 5322 Internet Message Format Line Folding (`lineFolded_IMF`)
//! - ITU-R M.1371 AIS payload armoring (`aisPayloadArmor`, `aisPayloadArmoring`)
//! - RFC 791 IPv4 header checksum (`IPv4Checksum`)
//! - Check digit verification (`checkDigit`)
//! - Boundary mark stream limiting (`boundaryMark`)
//! - Simple test layers (`stlBombOutLayer`, `stlOk1`, `stlOk2`, `stlOk3`, `stlOk4`, `allTypesLayer`)
//! - GPS-SPS navigation parity layers (`parityLayer`, `parityLayerHOW`)

#![no_std]
#![deny(missing_docs)]

extern crate alloc;
use alloc::sync::Arc;

pub mod ais;
pub mod base64;
pub mod boundary_mark;
pub mod byteswap;
pub mod check_digit;
pub mod gps_sps;
pub mod gzip;
pub mod ipv4_checksum;
pub mod linefold;
pub mod stl;

use dfdl_core::kernel::layer::LayerRegistry;

/// Registers all standard and extension layers into the given [`LayerRegistry`].
pub fn register_all(registry: &mut LayerRegistry) {
    // Byte swapping layers
    registry.register(Arc::new(byteswap::ByteSwapLayer { word_size: 2 }));
    registry.register(Arc::new(byteswap::ByteSwapLayer { word_size: 4 }));

    // Stream transformation layers
    registry.register(Arc::new(gzip::GzipLayer));
    registry.register(Arc::new(base64::Base64MimeLayer));
    registry.register(Arc::new(linefold::LineFoldedImfLayer));
    registry.register(Arc::new(ais::AisPayloadArmorLayer));

    // Checksum & verification layers
    registry.register(Arc::new(ipv4_checksum::IPv4ChecksumLayer));
    registry.register(Arc::new(check_digit::CheckDigitLayer));
    registry.register(Arc::new(boundary_mark::BoundaryMarkLayer));

    // Daffodil conformance STL layers
    registry.register(Arc::new(stl::StlBombOutLayer));
    registry.register(Arc::new(stl::StlOk1Layer));
    registry.register(Arc::new(stl::StlOk2Layer));
    registry.register(Arc::new(stl::StlOk3Layer));
    registry.register(Arc::new(stl::StlOk4Layer));
    registry.register(Arc::new(stl::AllTypesLayer));

    // GPS-SPS navigation telemetry parity layers
    registry.register(Arc::new(gps_sps::GpsSpsParityLayer));
    registry.register(Arc::new(gps_sps::GpsSpsParityHowLayer));
}

/// Initializes the global layer registry in `dfdl-core` with all registered layers.
pub fn init() {
    let mut registry = LayerRegistry::new();
    register_all(&mut registry);
    dfdl_core::kernel::layer::set_global_registry(registry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_registration_and_lookup() {
        let mut reg = LayerRegistry::new();
        register_all(&mut reg);

        assert!(reg.is_known_layer("fourByteSwap"));
        assert!(reg.is_known_layer("twoByteSwap"));
        assert!(reg.is_known_layer("byteSwap"));
        assert!(reg.is_known_layer("gzip"));
        assert!(reg.is_known_layer("base64_MIME"));
        assert!(reg.is_known_layer("lineFolded_IMF"));
        assert!(reg.is_known_layer("aisPayloadArmor"));
        assert!(reg.is_known_layer("IPv4Checksum"));
        assert!(reg.is_known_layer("checkDigit"));
        assert!(reg.is_known_layer("boundaryMark"));
        assert!(reg.is_known_layer("stlBombOutLayer"));
        assert!(reg.is_known_layer("stlOk1"));
        assert!(reg.is_known_layer("stlOk2"));
        assert!(reg.is_known_layer("stlOk3"));
        assert!(reg.is_known_layer("stlOk4"));
        assert!(reg.is_known_layer("allTypesLayer"));
        assert!(reg.is_known_layer("parityLayer"));
        assert!(reg.is_known_layer("parityLayerHOW"));

        assert!(!reg.is_known_layer("unknownCustomLayer"));
    }
}
