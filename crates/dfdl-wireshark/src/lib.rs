//! `dfdl-wireshark`: Generic Wireshark Dissector and PCAP Decoder powered by DFDL Schemas.
//!
//! Provides zero-hardcoding protocol dissection for Wireshark, TShark, and libpcap captures.
//! Introspects DFDL XML Schema documents (`.dfdl.xsd`) to extract:
//! - Field hierarchies and nested complex structures.
//! - Measurement units (`<ast:unit>`, `<unit>`).
//! - Numeric quantum and resolution scales (`<ast:resolution>`, `<ast:lsb>`).
//! - Field documentation tooltips (`<xs:documentation>`).
//!
//! # Architecture
//! - [`schema_meta`]: Purely generic schema introspection for documentation and engineering units.
//! - [`converter`]: Mathematical scaling and engineering value formatting.
//! - [`dissector`]: Wireshark protocol tree construction, ASCII rendering, PDML, and JSON serialization.
//! - [`engine`]: High-level [`DfdlDissectorEngine`] managing compiled DFDL execution graphs.
//! - [`pcap_runner`]: Generic PCAP file packet decoder without protocol assumptions.
//! - [`lua_dissector`]: Wireshark Lua dissector generator for live GUI network capture dissection.
//! - [`ffi`]: C-ABI foreign function interface for Wireshark dynamic library integration.

#![warn(missing_docs)]

pub mod converter;
pub mod dissector;
pub mod engine;
pub mod ffi;
pub mod lua_dissector;
pub mod pcap_runner;
pub mod schema_meta;

pub use converter::{convert_value, ConvertedValue};
pub use dissector::{DissectionNode, DissectionTree};
pub use engine::DfdlDissectorEngine;
pub use lua_dissector::{generate_lua_dissector, LuaDissectorConfig};
pub use pcap_runner::{decode_pcap_with_engine, DissectedPacket};
pub use schema_meta::{FieldMetadata, SchemaMetadataRegistry};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const SAMPLE_SCHEMA: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           xmlns:dfdl="http://www.dfdl.org/7793"
           xmlns:ast="urn:eurocontrol:asterix"
           targetNamespace="urn:eurocontrol:asterix">

  <xs:annotation>
    <xs:appinfo source="http://www.dfdl.org/7793">
      <dfdl:format byteOrder="bigEndian" alignment="1" representation="binary"/>
    </xs:appinfo>
  </xs:annotation>

  <xs:simpleType name="TimeOfDayType">
    <xs:annotation>
      <xs:documentation>Time of Day expressed in UTC elapsed seconds since midnight.</xs:documentation>
      <xs:appinfo source="urn:eurocontrol:asterix">
        <ast:item>I048/140</ast:item>
        <ast:description>Time of Day</ast:description>
        <ast:unit>seconds</ast:unit>
        <ast:resolution>1/128</ast:resolution>
        <ast:lsb>1/128 s = ~7.8125 ms</ast:lsb>
      </xs:appinfo>
    </xs:annotation>
    <xs:restriction base="xs:unsignedInt"/>
  </xs:simpleType>

  <xs:simpleType name="RhoType">
    <xs:annotation>
      <xs:documentation>Measured slant distance in Nautical Miles.</xs:documentation>
      <xs:appinfo source="urn:eurocontrol:asterix">
        <ast:item>I048/040</ast:item>
        <ast:description>Measured Distance Rho</ast:description>
        <ast:unit>NM</ast:unit>
        <ast:resolution>1/256</ast:resolution>
      </xs:appinfo>
    </xs:annotation>
    <xs:restriction base="xs:unsignedShort"/>
  </xs:simpleType>

  <xs:simpleType name="ThetaType">
    <xs:annotation>
      <xs:documentation>Measured azimuth angle in degrees.</xs:documentation>
      <xs:appinfo source="urn:eurocontrol:asterix">
        <ast:item>I048/040</ast:item>
        <ast:description>Measured Azimuth Theta</ast:description>
        <ast:unit>degrees</ast:unit>
        <ast:resolution>360/65536</ast:resolution>
      </xs:appinfo>
    </xs:annotation>
    <xs:restriction base="xs:unsignedShort"/>
  </xs:simpleType>

  <xs:element name="RadarReport" dfdl:lengthKind="implicit">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="timeOfDay" type="ast:TimeOfDayType"
                    dfdl:representation="binary" dfdl:lengthKind="explicit" dfdl:length="3"/>
        <xs:element name="rho" type="ast:RhoType"
                    dfdl:representation="binary" dfdl:lengthKind="explicit" dfdl:length="2"/>
        <xs:element name="theta" type="ast:ThetaType"
                    dfdl:representation="binary" dfdl:lengthKind="explicit" dfdl:length="2"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>

</xs:schema>"#;

    #[test]
    fn test_generic_schema_metadata_extraction() {
        let registry = SchemaMetadataRegistry::from_schema_xml(SAMPLE_SCHEMA);
        assert!(!registry.is_empty());

        let time_meta = registry.get("timeOfDay").expect("timeOfDay metadata found");
        assert_eq!(time_meta.unit, "seconds");
        assert_eq!(time_meta.resolution_str, "1/128");
        assert!((time_meta.scale_factor - (1.0 / 128.0)).abs() < 1e-9);
        assert!(time_meta.documentation.contains("UTC elapsed seconds"));

        let rho_meta = registry.get("rho").expect("rho metadata found");
        assert_eq!(rho_meta.unit, "NM");
        assert!((rho_meta.scale_factor - (1.0 / 256.0)).abs() < 1e-9);

        let theta_meta = registry.get("theta").expect("theta metadata found");
        assert_eq!(theta_meta.unit, "degrees");
        assert!((theta_meta.scale_factor - (360.0 / 65536.0)).abs() < 1e-9);
    }

    #[test]
    fn test_generic_dissector_engine_execution() {
        let engine = DfdlDissectorEngine::from_schema_str(SAMPLE_SCHEMA, None)
            .expect("Schema compilation must succeed");

        // 3 bytes time (0x01, 0x00, 0x00 = 65536 -> 65536 / 128 = 512.0 s)
        // 2 bytes rho (0x01, 0x00 = 256 -> 256 / 256 = 1.0 NM)
        // 2 bytes theta (0x40, 0x00 = 16384 -> 16384 * 360 / 65536 = 90.0 deg)
        let payload = [0x01, 0x00, 0x00, 0x01, 0x00, 0x40, 0x00];

        let tree = engine
            .dissect_payload(&payload)
            .expect("Dissection must succeed");

        assert!(tree.root.is_some());
        let root = tree.root.as_ref().unwrap();
        assert_eq!(root.name, "RadarReport");
        assert_eq!(root.children.len(), 3);

        let time_child = &root.children[0];
        assert_eq!(time_child.name, "timeOfDay");
        assert_eq!(time_child.description, "Time of Day");
        assert!(time_child.label.starts_with("Time of Day (timeOfDay)"));
        assert_eq!(time_child.raw_value, "65536");
        assert!(time_child.converted_value.contains("512"));
        assert_eq!(time_child.unit, "seconds");

        let rho_child = &root.children[1];
        assert_eq!(rho_child.name, "rho");
        assert_eq!(rho_child.description, "Measured Distance Rho");
        assert!(rho_child.label.starts_with("Measured Distance Rho (rho)"));
        assert_eq!(rho_child.raw_value, "256");
        assert!(rho_child.converted_value.contains("1"));
        assert_eq!(rho_child.unit, "NM");

        let theta_child = &root.children[2];
        assert_eq!(theta_child.name, "theta");
        assert_eq!(theta_child.description, "Measured Azimuth Theta");
        assert!(theta_child.label.starts_with("Measured Azimuth Theta (theta)"));
        assert_eq!(theta_child.raw_value, "16384");
        assert!(theta_child.converted_value.contains("90"));
        assert_eq!(theta_child.unit, "degrees");

        // Test ASCII and PDML and JSON rendering
        let ascii = tree.render_ascii();
        assert!(ascii.contains("Time of Day (timeOfDay)"));
        assert!(ascii.contains("Measured Distance Rho (rho)"));

        let pdml = tree.render_pdml(1);
        assert!(pdml.contains("<packet num=\"1\">"));
        assert!(pdml.contains("unit=\"NM\""));

        let json = tree.to_json();
        assert!(json.contains("\"name\":\"RadarReport\""));
        assert!(json.contains("\"description\":\"Time of Day\""));
    }

    #[test]
    fn test_generic_lua_generator() {
        let config = LuaDissectorConfig::new("RadarReport", "/path/to/schema.dfdl.xsd");
        let lua = generate_lua_dissector(&config);

        assert!(lua.contains("Proto(\"dfdl_radarreport\""));
        assert!(lua.contains("proto.prefs.schema_path"));
        assert!(lua.contains("/path/to/schema.dfdl.xsd"));
        assert!(lua.contains("function proto.dissector(tvb, pinfo, tree)"));
    }

    #[test]
    fn test_enum_formatting_main_string_secondary_numeric() {
        let schema_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           xmlns:dfdl="http://www.ogf.org/dfdl/dfdl-1.0/"
           xmlns:ast="urn:eurocontrol:asterix"
           targetNamespace="urn:eurocontrol:asterix">
  <xs:simpleType name="StatusType">
    <xs:annotation>
      <xs:documentation>
        Status code:
        - 0: Nominal
        - 1: Degraded
        - 2: Failure
      </xs:documentation>
      <xs:appinfo source="urn:eurocontrol:asterix">
        <ast:enum value="0" label="Nominal"/>
        <ast:enum value="1" label="Degraded"/>
        <ast:enum value="2" label="Failure"/>
      </xs:appinfo>
    </xs:annotation>
    <xs:restriction base="xs:unsignedByte"/>
  </xs:simpleType>

  <xs:element name="StatusReport">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="status" type="ast:StatusType"
                    dfdl:representation="binary" dfdl:lengthKind="explicit" dfdl:length="1"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;

        let registry = SchemaMetadataRegistry::from_schema_xml(schema_xml);
        let status_meta = registry.get("status").expect("status metadata should exist");
        assert_eq!(status_meta.get_enum_label(0), Some("Nominal"));
        assert_eq!(status_meta.get_enum_label(1), Some("Degraded"));
        assert_eq!(status_meta.get_enum_label(2), Some("Failure"));

        // Test conversion: formatted should be "{label} ({numeric})"
        let conv0 = converter::convert_value("status", "0", Some(status_meta));
        assert_eq!(conv0.formatted, "Nominal (0)");
        assert_eq!(conv0.raw, "0");

        let conv1 = converter::convert_value("status", "1", Some(status_meta));
        assert_eq!(conv1.formatted, "Degraded (1)");

        let conv2 = converter::convert_value("status", "2", Some(status_meta));
        assert_eq!(conv2.formatted, "Failure (2)");

        let conv_unknown = converter::convert_value("status", "7", Some(status_meta));
        assert_eq!(conv_unknown.formatted, "Unknown (7)");
    }

    #[test]
    fn test_schema_hierarchical_fields() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        let candidates = [
            std::path::PathBuf::from(&manifest_dir).join("../../crates/dfdl-example/schemas/asterix.dfdl.xsd"),
            std::path::PathBuf::from(&manifest_dir).join("../dfdl-example/schemas/asterix.dfdl.xsd"),
            std::path::PathBuf::from("crates/dfdl-example/schemas/asterix.dfdl.xsd"),
        ];
        let path = candidates
            .iter()
            .find(|p| p.exists())
            .expect("asterix.dfdl.xsd not found in candidates");
        let xsd = std::fs::read_to_string(path).expect("asterix.dfdl.xsd must exist");
        let reg = SchemaMetadataRegistry::from_schema_xml(&xsd);
        println!("RECORD_ELEMENTS: {:?}", reg.record_elements);
        println!("CONTAINMENT_KEYS: {:?}", reg.containment.keys());
        println!("HIERARCHICAL_FIELDS count: {}", reg.hierarchical_fields.len());
        for (p, n) in reg.hierarchical_fields.iter().take(20) {
            println!("FIELD: {p} (leaf: {n})");
        }
        assert!(!reg.hierarchical_fields.is_empty());
    }
}

