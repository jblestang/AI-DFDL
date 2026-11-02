//! `dfdl-payload-generator` — Schema-driven DFDL payload and PCAP capture generator.
//!
//! Generates authentic binary payloads conforming to any DFDL schema, with exhaustive coverage
//! of:
//! - Valid payloads across all choice branches and facet boundaries
//! - Syntactically invalid payloads (truncations, framing errors, length mismatches, bit misalignment)
//! - Semantically invalid payloads (out-of-bounds facets, wrong enums, unmatched dispatch keys, failed asserts)
//! - Direct export to PCAP capture format with Ethernet, IPv4, and UDP encapsulation.

pub mod generator;
pub mod pcap;
pub mod prng;
pub mod schema_walk;
pub mod semantic;
pub mod syntax;
pub mod valid;

pub use generator::{
    GeneratedPayload, GenerationLimits, GenerationReport, PayloadGenerator, PayloadMode,
};
pub use pcap::{LinkType, NetworkConfig, PcapFile, PcapPacket, PcapWriter, TransportProtocol};
pub use prng::FastPrng;
pub use schema_walk::{ChoiceBranchTarget, ElementFieldInfo, SchemaAnalysis};
pub use semantic::{SemanticAnomaly, SemanticAnomalyInjector};
pub use syntax::{SyntaxAnomaly, SyntaxAnomalyInjector};
pub use valid::{GenerationConfig, ValidPayloadBuilder};
