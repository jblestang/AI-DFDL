-- ============================================================================
-- Generic DFDL Wireshark Dissector
-- Allows loading and changing DFDL schemas from within Wireshark GUI
-- Configure via: Edit -> Preferences -> Protocols -> DFDL (DFDL Engine)
-- ============================================================================

local proto = Proto("dfdl", "DFDL (DFDL Engine)")

-- User Preferences (Accessible inside Wireshark: Edit -> Preferences -> Protocols)
proto.prefs.schema_path = Pref.string("DFDL Schema Path (.dfdl.xsd)", "/Users/jean-baptiste/AI-DFDL/crates/dfdl-example/schemas/asterix.dfdl.xsd", "Absolute path to the DFDL XML schema file (.dfdl.xsd)")
proto.prefs.ports = Pref.range("UDP Port(s)", "21111-22135", "UDP ports to dissect with this DFDL schema", 65535)
proto.prefs.cli_path = Pref.string("dfdl-wireshark Binary", "/Users/jean-baptiste/AI-DFDL/target/release/dfdl-wireshark", "Command name or full path to dfdl-wireshark executable")

-- Dynamically registered fields with overlay tooltips
local proto_fields = {
    ["AircraftIdType"] = ProtoField.string("dfdl.AircraftIdType", "Target Identification (Callsign) (AircraftIdType)", base.NONE, nil, 0, "Target Identification (Callsign) | Item: I048/240 | Unit: string | Item 240: Target Identification (Callsign).         8 characters representing the aircraft callsign."),
    ["statusSsr"] = ProtoField.string("dfdl.statusSsr", "statusSsr", base.NONE, nil, 0, ""),
    ["trackNumber"] = ProtoField.string("dfdl.trackNumber", "Radar Track Number (trackNumber)", base.NONE, nil, 0, "Radar Track Number | Item: I048/161, I001/161 | Unit: track_id | Resolution: 1 | LSB: 1 | Item 161: Track Number.         Unique tracking identifier assigned to a track by the radar data processor.         Standard Eurocontrol definition uses bits 12-1 (0 to 4095) with bits 16-13 reserved (0).         Quantum: 1. Unit: dimensionless integer. Range: 0 to 65535."),
    ["messageType"] = ProtoField.string("dfdl.messageType", "Service Message Type (messageType)", base.NONE, nil, 0, "Service Message Type | Item: I034/000, I002/000 | Unit: type_id | Resolution: 1 | LSB: 1 | Item 000: Message Type.         Defines the service message type for Monoradar Service Messages:         - 1: North Marker Message         - 2: Sector Crossing Message         - 3: Geographical Filtering Message (Cat 034) / Station Status Message (Cat 002)         - 4: Jamming Strobe Message         - 5: Solar Storm Message         - 6: SSR Jamming Strobe Message         - 7: Mode S Jamming Strobe Message         - 8..255: Other service / reserved messages         Quantum: 1. Unit: message type ID. Range: 1 to 255."),
    ["Cat034NorthMarkerType"] = ProtoField.string("dfdl.Cat034NorthMarkerType", "Cat034NorthMarkerType", base.NONE, nil, 0, ""),
    ["truncatedTimeOfDay"] = ProtoField.string("dfdl.truncatedTimeOfDay", "Truncated Time of Day (truncatedTimeOfDay)", base.NONE, nil, 0, "Truncated Time of Day | Item: I001/141 | Unit: seconds | Resolution: 1/128 | LSB: 0.0078125 s (1/128 s) | Item 141: Truncated Time of Day.         Time of day truncated to the 16 least significant bits of the standard Time of Day.         Quantum (LSB): 1/128 s (~7.8125 milliseconds).         Unit: seconds.         Range: [0 s, 512 s) represented as [0, 65535] (wraps around every 512 seconds)."),
    ["stationProcessingMode"] = ProtoField.string("dfdl.stationProcessingMode", "stationProcessingMode", base.NONE, nil, 0, ""),
    ["PolarAzimuthType"] = ProtoField.string("dfdl.PolarAzimuthType", "Measured Azimuth Theta in Polar Coordinates (PolarAzimuthType)", base.NONE, nil, 0, "Measured Azimuth Theta in Polar Coordinates | Item: I048/040-THETA, I034/100-THETA, I001/040-THETA | Unit: degrees | Resolution: 360/65536 | LSB: 0.005493164 deg | Item 040 / 100: Measured Azimuth (Theta) in Polar Coordinates.         Azimuth angle measured clockwise from True North to the target.         Quantum (LSB): 360 / 2^16 degrees (= 360 / 65536 deg ~ 0.005493164 degrees ~ 19.77 arcseconds).         Unit: degrees.         Range: 0 to 359.9945068 degrees (0 to 65535 LSB)."),
    ["spare"] = ProtoField.string("dfdl.spare", "Spare bit (spare)", base.NONE, nil, 0, "Spare bit | Item: I048/230.SPARE | Unit: code | Resolution: 1 | Spare bit (set to 0)"),
    ["messageCount"] = ProtoField.string("dfdl.messageCount", "Message Count Values (messageCount)", base.NONE, nil, 0, "Message Count Values | Item: I034/070 | Unit: messages | Resolution: 1 | LSB: 1 | Item 070: Message Count Values.         Number of messages of the specified type transmitted during the reporting period.         Quantum (LSB): 1 message.         Unit: messages.         Range: 0 to 65535 messages."),
    ["gho"] = ProtoField.string("dfdl.gho", "Ghost Track Indicator (gho)", base.NONE, nil, 0, "Ghost Track Indicator | Item: I048/170.GHO | Unit: code | Resolution: 1 | GHO: Ghost Track Indicator (0=True target, 1=Ghost track)"),
    ["prl"] = ProtoField.string("dfdl.prl", "Primary Plot Runlength (prl)", base.NONE, nil, 0, "Primary Plot Runlength | Item: I048/130-PRL | Unit: degrees | Resolution: 360/8192 | LSB: 0.0439453125 deg | Item 130 Subfield 4: Primary Plot Runlength (PRL).         Angular width of the primary radar (PSR) plot.         Quantum (LSB): 360 / 2^13 degrees (= 360 / 8192 deg ~ 0.0439453125 degrees).         Unit: degrees.         Range: 0 to 11.2060547 degrees (0 to 255 LSB)."),
    ["antennaRotationPeriod"] = ProtoField.string("dfdl.antennaRotationPeriod", "Antenna Rotation Period (antennaRotationPeriod)", base.NONE, nil, 0, "Antenna Rotation Period | Item: I034/041, I002/041 | Unit: seconds | Resolution: 1/128 | LSB: 0.0078125 s (1/128 s) | Item 041: Antenna Rotation Period.         Measured duration of the previous full 360-degree antenna revolution.         Quantum (LSB): 1/128 s (~7.8125 milliseconds).         Unit: seconds.         Range: 0 to 511.9921875 seconds (0 to 65535 LSB)."),
    ["PamType"] = ProtoField.string("dfdl.PamType", "Primary Plot Amplitude (PamType)", base.NONE, nil, 0, "Primary Plot Amplitude | Item: I048/130-PAM | Unit: dBm | Resolution: 1 | LSB: 1 dBm | Item 130 Subfield 5: Primary Plot Amplitude (PAM).         Received signal strength of the primary radar (PSR) target plot.         Quantum (LSB): 1 dBm.         Unit: dBm.         Range: 0 to 255 dBm."),
    ["Fspec2Type"] = ProtoField.string("dfdl.Fspec2Type", "Field Specification (FSPEC 2) (Fspec2Type)", base.NONE, nil, 0, "Field Specification (FSPEC 2) | Unit: bitfield | Resolution: 1 | FSPEC Octet 2: First extension field specification."),
    ["ssrJammingStrobe"] = ProtoField.string("dfdl.ssrJammingStrobe", "ssrJammingStrobe", base.NONE, nil, 0, ""),
    ["stationConfig"] = ProtoField.string("dfdl.stationConfig", "stationConfig", base.NONE, nil, 0, ""),
    ["DataSourceHeightType"] = ProtoField.string("dfdl.DataSourceHeightType", "Antenna Height above Mean Sea Level (DataSourceHeightType)", base.NONE, nil, 0, "Antenna Height above Mean Sea Level | Item: I034/120-HEIGHT | Unit: meters | Resolution: 6.25 | LSB: 6.25 m | Item 120 (Height Subfield): 3D Position of Data Source - Antenna Height.         Height of the radar antenna phase center relative to mean sea level in two's complement.         Quantum (LSB): 6.25 meters (Eurocontrol standard) or 1 meter.         Unit: meters.         Range: -32768 to 32767."),
    ["sim"] = ProtoField.string("dfdl.sim", "Simulated Target (sim)", base.NONE, nil, 0, "Simulated Target | Item: I048/020.SIM | Unit: code | Resolution: 1 | SIM: Simulated Target Report (0=Actual, 1=Simulated)"),
    ["bdsRep"] = ProtoField.string("dfdl.bdsRep", "BDS Report Repetition Factor (bdsRep)", base.NONE, nil, 0, "BDS Report Repetition Factor | Item: I048/250-REP | Unit: count | Resolution: 1 | Item 250: BDS Report Repetition Factor.         Number of BDS reports following."),
    ["Cat002Record"] = ProtoField.string("dfdl.Cat002Record", "Cat002Record", base.NONE, nil, 0, ""),
    ["SicType"] = ProtoField.string("dfdl.SicType", "System Identification Code (SIC) (SicType)", base.NONE, nil, 0, "System Identification Code (SIC) | Item: I034/010-SIC, I048/010-SIC, I001/010-SIC | Unit: code | Resolution: 1 | LSB: 1 | Item 010: System Identification Code (SIC).         Identifies the radar sensor station within the SAC."),
    ["AircraftAddressType"] = ProtoField.string("dfdl.AircraftAddressType", "24-bit ICAO Aircraft Address (AircraftAddressType)", base.NONE, nil, 0, "24-bit ICAO Aircraft Address | Item: I048/220 | Unit: hex | Resolution: 1 | LSB: 1 | Item 220: Aircraft Address.         Unique 24-bit ICAO technical aircraft address assigned to aircraft transponder.         Quantum: 1. Unit: 24-bit hexadecimal code. Range: 0x000000 to 0xFFFFFF (0 to 16777215)."),
    ["SrrType"] = ProtoField.string("dfdl.SrrType", "Number of Received SSR Replies (SrrType)", base.NONE, nil, 0, "Number of Received SSR Replies | Item: I048/130-SRR | Unit: replies | Resolution: 1 | LSB: 1 | Item 130 Subfield 2: Number of Received SSR Replies (SRR).         Total number of valid SSR replies received for this target plot.         Quantum (LSB): 1 reply.         Unit: replies.         Range: 0 to 255 replies."),
    ["mssc"] = ProtoField.string("dfdl.mssc", "Mode S Specific Service Capability (mssc)", base.NONE, nil, 0, "Mode S Specific Service Capability | Item: I048/230.MSSC | Unit: code | Resolution: 1 | MSSC: Mode S Specific Service Capability (0=No, 1=Yes)"),
    ["category"] = ProtoField.string("dfdl.category", "ASTERIX Data Category number (category)", base.NONE, nil, 0, "ASTERIX Data Category number | Item: Data Category | Unit: dimensionless | Resolution: 1 | LSB: 1 | ASTERIX Data Category.         Identifies the message category according to Eurocontrol ASTERIX standards:         - Cat 034: Transmission of Monoradar Service Messages (Ed 1.28)         - Cat 048: Transmission of Monoradar Target Reports (Ed 1.21)         - Cat 001: Transmission of Monoradar Data (Ed 1.2)         - Cat 002: Transmission of Monoradar Service Messages (Ed 1.0)         Quantum: 1 (category number). Unit: dimensionless. Range: 0 to 255."),
    ["fspec3"] = ProtoField.string("dfdl.fspec3", "Field Specification (FSPEC 3) (fspec3)", base.NONE, nil, 0, "Field Specification (FSPEC 3) | Unit: bitfield | Resolution: 1 | FSPEC Octet 3: Second extension field specification."),
    ["arc"] = ProtoField.string("dfdl.arc", "Altitude Reporting Capability (arc)", base.NONE, nil, 0, "Altitude Reporting Capability | Item: I048/230.ARC | Unit: code | Resolution: 1 | ARC: Altitude Reporting Capability (0=100 ft, 1=25 ft)"),
    ["latitude"] = ProtoField.string("dfdl.latitude", "WGS-84 Geographic Latitude / Longitude (latitude)", base.NONE, nil, 0, "WGS-84 Geographic Latitude / Longitude | Item: I034/120-LATLON | Unit: degrees | Resolution: 180/8388608 | LSB: 0.00002145767 deg | Item 120 (Latitude / Longitude Subfields): 3D Position of Data Source in WGS-84.         Geographic coordinates in two's complement 24-bit binary representation.         Quantum (LSB): 180 / 2^23 degrees (= 180 / 8388608 deg ~ 0.00002145767 degrees ~ 2.146 cm at equator).         Unit: degrees.         Range: [-90.0°, +90.0°] for Latitude; [-180.0°, +180.0°] for Longitude."),
    ["modeFspec"] = ProtoField.string("dfdl.modeFspec", "modeFspec", base.NONE, nil, 0, ""),
    ["Cat034GeographicalFilteringType"] = ProtoField.string("dfdl.Cat034GeographicalFilteringType", "Cat034GeographicalFilteringType", base.NONE, nil, 0, ""),
    ["northMarker"] = ProtoField.string("dfdl.northMarker", "northMarker", base.NONE, nil, 0, ""),
    ["PrlType"] = ProtoField.string("dfdl.PrlType", "Primary Plot Runlength (PrlType)", base.NONE, nil, 0, "Primary Plot Runlength | Item: I048/130-PRL | Unit: degrees | Resolution: 360/8192 | LSB: 0.0439453125 deg | Item 130 Subfield 4: Primary Plot Runlength (PRL).         Angular width of the primary radar (PSR) plot.         Quantum (LSB): 360 / 2^13 degrees (= 360 / 8192 deg ~ 0.0439453125 degrees).         Unit: degrees.         Range: 0 to 11.2060547 degrees (0 to 255 LSB)."),
    ["targetDescriptorExt"] = ProtoField.string("dfdl.targetDescriptorExt", "targetDescriptorExt", base.NONE, nil, 0, ""),
    ["trackStatus"] = ProtoField.string("dfdl.trackStatus", "trackStatus", base.NONE, nil, 0, ""),
    ["SacType"] = ProtoField.string("dfdl.SacType", "System Area Code (SAC) (SacType)", base.NONE, nil, 0, "System Area Code (SAC) | Item: I034/010-SAC, I048/010-SAC, I001/010-SAC | Unit: code | Resolution: 1 | LSB: 1 | Item 010: System Area Code (SAC).         Identifies the nation, region, or airspace division."),
    ["sectorNumber"] = ProtoField.string("dfdl.sectorNumber", "Azimuth Sector Number (sectorNumber)", base.NONE, nil, 0, "Azimuth Sector Number | Item: I034/020, I002/020 | Unit: degrees | Resolution: 360/256 | LSB: 1.40625 deg | Item 020: Sector Number.         Azimuth sector crossing indicator, dividing 360 degrees into 256 equal angular sectors.         Quantum (LSB): 360 / 256 degrees = 1.40625 degrees (~1° 24' 22.5\").         Unit: degrees.         Range: 0 to 255 (= 0.0° to 358.59375° clockwise from True North)."),
    ["TrackNumberType"] = ProtoField.string("dfdl.TrackNumberType", "Radar Track Number (TrackNumberType)", base.NONE, nil, 0, "Radar Track Number | Item: I048/161, I001/161 | Unit: track_id | Resolution: 1 | LSB: 1 | Item 161: Track Number.         Unique tracking identifier assigned to a track by the radar data processor.         Standard Eurocontrol definition uses bits 12-1 (0 to 4095) with bits 16-13 reserved (0).         Quantum: 1. Unit: dimensionless integer. Range: 0 to 65535."),
    ["RpdType"] = ProtoField.string("dfdl.RpdType", "Range Difference (PSR - SSR) (RpdType)", base.NONE, nil, 0, "Range Difference (PSR - SSR) | Item: I048/130-RPD | Unit: NM | Resolution: 1/256 | LSB: 0.00390625 NM (~7.2297 m) | Item 130 Subfield 6: Difference in Range between PSR and SSR plots (RPD = PSR - SSR).         Quantum (LSB): 1/256 NM (~7.2297 meters). Encoded as two's complement.         Unit: Nautical Miles (NM).         Range: [-128/256 NM, +127/256 NM] = [-0.5 NM, +0.49609375 NM]."),
    ["groundSpeed"] = ProtoField.string("dfdl.groundSpeed", "Calculated Track Ground Speed (groundSpeed)", base.NONE, nil, 0, "Calculated Track Ground Speed | Item: I048/200-SPEED, I001/200-SPEED | Unit: NM/s | Resolution: 1/16384 | LSB: 0.00006103515625 NM/s (~0.2197 kt) | Item 200 (Ground Speed Subfield): Calculated Track Velocity in Polar Coordinates.         Horizontal speed of the target calculated by the tracker.         Quantum (LSB): 2^-14 NM/s (= 1/16384 NM/s ~ 0.2197265625 knots ~ 0.113037 m/s).         Unit: NM/s (multiply raw value by 3600 / 16384 to obtain speed in knots).         Range: 0 to 3.99993896 NM/s (~0 to 14399.78 knots)."),
    ["statusFspec"] = ProtoField.string("dfdl.statusFspec", "statusFspec", base.NONE, nil, 0, ""),
    ["height"] = ProtoField.string("dfdl.height", "Antenna Height above Mean Sea Level (height)", base.NONE, nil, 0, "Antenna Height above Mean Sea Level | Item: I034/120-HEIGHT | Unit: meters | Resolution: 6.25 | LSB: 6.25 m | Item 120 (Height Subfield): 3D Position of Data Source - Antenna Height.         Height of the radar antenna phase center relative to mean sea level in two's complement.         Quantum (LSB): 6.25 meters (Eurocontrol standard) or 1 meter.         Unit: meters.         Range: -32768 to 32767."),
    ["modeSsr"] = ProtoField.string("dfdl.modeSsr", "modeSsr", base.NONE, nil, 0, ""),
    ["AsterixPacket"] = ProtoField.string("dfdl.AsterixPacket", "AsterixPacket", base.NONE, nil, 0, ""),
    ["CollimationErrorType"] = ProtoField.string("dfdl.CollimationErrorType", "Collimation Error (CollimationErrorType)", base.NONE, nil, 0, "Collimation Error | Item: I034/090 | Unit: degrees | Resolution: 360/65536 | LSB: 0.005493164 deg | Item 090: Collimation Error.         Angular alignment error between the PSR axis and the SSR axis.         Quantum (LSB): 360 / 2^16 degrees (= 360 / 65536 deg ~ 0.005493164 degrees).         Unit: degrees.         Range: 0 to 359.9945 degrees."),
    ["MessageCountType"] = ProtoField.string("dfdl.MessageCountType", "Message Count Values (MessageCountType)", base.NONE, nil, 0, "Message Count Values | Item: I034/070 | Unit: messages | Resolution: 1 | LSB: 1 | Item 070: Message Count Values.         Number of messages of the specified type transmitted during the reporting period.         Quantum (LSB): 1 message.         Unit: messages.         Range: 0 to 65535 messages."),
    ["Wgs84CoordinateType"] = ProtoField.string("dfdl.Wgs84CoordinateType", "WGS-84 Geographic Latitude / Longitude (Wgs84CoordinateType)", base.NONE, nil, 0, "WGS-84 Geographic Latitude / Longitude | Item: I034/120-LATLON | Unit: degrees | Resolution: 180/8388608 | LSB: 0.00002145767 deg | Item 120 (Latitude / Longitude Subfields): 3D Position of Data Source in WGS-84.         Geographic coordinates in two's complement 24-bit binary representation.         Quantum (LSB): 180 / 2^23 degrees (= 180 / 8388608 deg ~ 0.00002145767 degrees ~ 2.146 cm at equator).         Unit: degrees.         Range: [-90.0°, +90.0°] for Latitude; [-180.0°, +180.0°] for Longitude."),
    ["Fspec1Type"] = ProtoField.string("dfdl.Fspec1Type", "Field Specification (FSPEC 1) (Fspec1Type)", base.NONE, nil, 0, "Field Specification (FSPEC 1) | Unit: bitfield | Resolution: 1 | FSPEC Octet 1: Primary field specification."),
    ["ApdType"] = ProtoField.string("dfdl.ApdType", "Azimuth Difference (PSR - SSR) (ApdType)", base.NONE, nil, 0, "Azimuth Difference (PSR - SSR) | Item: I048/130-APD | Unit: degrees | Resolution: 360/16384 | LSB: 0.02197265625 deg | Item 130 Subfield 7: Difference in Azimuth between PSR and SSR plots (APD = PSR - SSR).         Quantum (LSB): 360 / 2^14 degrees (= 360 / 16384 deg ~ 0.02197265625 degrees). Encoded as two's complement.         Unit: degrees.         Range: [-128 * (360/16384)°, +127 * (360/16384)°] = [-2.8125°, +2.79052734375°]."),
    ["commCapability"] = ProtoField.string("dfdl.commCapability", "commCapability", base.NONE, nil, 0, ""),
    ["TimeOfDayType"] = ProtoField.string("dfdl.TimeOfDayType", "Time of Day (UTC since midnight) (TimeOfDayType)", base.NONE, nil, 0, "Time of Day (UTC since midnight) | Item: I034/030, I048/140, I002/030 | Unit: seconds | Resolution: 1/128 | LSB: 0.0078125 s (1/128 s) | Item 030 / 140: Time of Day.         Absolute time stamping expressed as UTC elapsed time since midnight.         Quantum (LSB): 1/128 s (~7.8125 milliseconds).         Unit: seconds.         Range: [0 s, 24 h) = [0, 86400 s) represented as integer [0, 11059200].         Maximum 24-bit integer: 16777215 (~36.4 hours)."),
    ["cnf"] = ProtoField.string("dfdl.cnf", "Confirmed / Tentative Track (cnf)", base.NONE, nil, 0, "Confirmed / Tentative Track | Item: I048/170.CNF | Unit: code | Resolution: 1 | CNF: Confirmed vs Tentative Track (0=Confirmed, 1=Tentative)"),
    ["modeSJammingStrobe"] = ProtoField.string("dfdl.modeSJammingStrobe", "modeSJammingStrobe", base.NONE, nil, 0, ""),
    ["statusPsr"] = ProtoField.string("dfdl.statusPsr", "statusPsr", base.NONE, nil, 0, ""),
    ["plotCharFspec"] = ProtoField.string("dfdl.plotCharFspec", "plotCharFspec", base.NONE, nil, 0, ""),
    ["sam"] = ProtoField.string("dfdl.sam", "SSR Reply Amplitude (sam)", base.NONE, nil, 0, "SSR Reply Amplitude | Item: I048/130-SAM | Unit: dBm | Resolution: 1 | LSB: 1 dBm | Item 130 Subfield 3: SSR Reply Amplitude (SAM).         Average received signal strength of SSR replies referenced to receiver detection threshold.         Quantum (LSB): 1 dBm.         Unit: dBm.         Range: 0 to 255 dBm."),
    ["trackStatusExt"] = ProtoField.string("dfdl.trackStatusExt", "trackStatusExt", base.NONE, nil, 0, ""),
    ["tst"] = ProtoField.string("dfdl.tst", "Test Target Indicator (tst)", base.NONE, nil, 0, "Test Target Indicator | Item: I048/020.TST | Unit: code | Resolution: 1 | TST: Test Target Indicator (0=Real, 1=Test)"),
    ["modeCom"] = ProtoField.string("dfdl.modeCom", "modeCom", base.NONE, nil, 0, ""),
    ["fspec2"] = ProtoField.string("dfdl.fspec2", "Field Specification (FSPEC 2) (fspec2)", base.NONE, nil, 0, "Field Specification (FSPEC 2) | Unit: bitfield | Resolution: 1 | FSPEC Octet 2: First extension field specification."),
    ["Fspec3Type"] = ProtoField.string("dfdl.Fspec3Type", "Field Specification (FSPEC 3) (Fspec3Type)", base.NONE, nil, 0, "Field Specification (FSPEC 3) | Unit: bitfield | Resolution: 1 | FSPEC Octet 3: Second extension field specification."),
    ["rab"] = ProtoField.string("dfdl.rab", "Report from Aircraft Address Reply (rab)", base.NONE, nil, 0, "Report from Aircraft Address Reply | Item: I048/020.RAB | Unit: code | Resolution: 1 | RAB: Report from Target Report (0) vs Reply (1)"),
    ["fx"] = ProtoField.string("dfdl.fx", "Extension Bit (fx)", base.NONE, nil, 0, "Extension Bit | Item: I048/170.FX | Unit: code | Resolution: 1 | FX: Extension Indicator (0=End, 1=Extension follows)"),
    ["GroundSpeedType"] = ProtoField.string("dfdl.GroundSpeedType", "Calculated Track Ground Speed (GroundSpeedType)", base.NONE, nil, 0, "Calculated Track Ground Speed | Item: I048/200-SPEED, I001/200-SPEED | Unit: NM/s | Resolution: 1/16384 | LSB: 0.00006103515625 NM/s (~0.2197 kt) | Item 200 (Ground Speed Subfield): Calculated Track Velocity in Polar Coordinates.         Horizontal speed of the target calculated by the tracker.         Quantum (LSB): 2^-14 NM/s (= 1/16384 NM/s ~ 0.2197265625 knots ~ 0.113037 m/s).         Unit: NM/s (multiply raw value by 3600 / 16384 to obtain speed in knots).         Range: 0 to 3.99993896 NM/s (~0 to 14399.78 knots)."),
    ["aic"] = ProtoField.string("dfdl.aic", "Aircraft Identification Capability (aic)", base.NONE, nil, 0, "Aircraft Identification Capability | Item: I048/230.AIC | Unit: code | Resolution: 1 | AIC: Aircraft Identification Capability (0=No, 1=Yes)"),
    ["bdsData"] = ProtoField.string("dfdl.bdsData", "Mode S Comm B MB Data (56 bits) (bdsData)", base.NONE, nil, 0, "Mode S Comm B MB Data (56 bits) | Item: I048/250-DATA | Unit: bytes | Resolution: 1 | LSB: 1 bit | Item 250 (BDS Data Subfield): Mode S Comm B Message Data.         56-bit MB data payload extracted from Mode S Comm-B replies (e.g. BDS 4,0, 5,0, 6,0).         Quantum: 56 bits (7 octets). Unit: binary hex payload."),
    ["AsterixDatagram"] = ProtoField.string("dfdl.AsterixDatagram", "AsterixDatagram", base.NONE, nil, 0, ""),
    ["Cat048Record"] = ProtoField.string("dfdl.Cat048Record", "Cat048Record", base.NONE, nil, 0, ""),
    ["SectorNumberType"] = ProtoField.string("dfdl.SectorNumberType", "Azimuth Sector Number (SectorNumberType)", base.NONE, nil, 0, "Azimuth Sector Number | Item: I034/020, I002/020 | Unit: degrees | Resolution: 360/256 | LSB: 1.40625 deg | Item 020: Sector Number.         Azimuth sector crossing indicator, dividing 360 degrees into 256 equal angular sectors.         Quantum (LSB): 360 / 256 degrees = 1.40625 degrees (~1° 24' 22.5\").         Unit: degrees.         Range: 0 to 255 (= 0.0° to 358.59375° clockwise from True North)."),
    ["foeFri"] = ProtoField.string("dfdl.foeFri", "Friend / Foe Identification (foeFri)", base.NONE, nil, 0, "Friend / Foe Identification | Item: I048/020.FOE_FRI | Unit: code | Resolution: 1 | FOE_FRI: Friend / Foe Identification (0=No Mode 4, 1=Friendly, 2=Unknown, 3=No reply)"),
    ["Mode3ACodeType"] = ProtoField.string("dfdl.Mode3ACodeType", "Mode-3/A Code in Octal Representation (Mode3ACodeType)", base.NONE, nil, 0, "Mode-3/A Code in Octal Representation | Item: I048/070, I001/070 | Unit: octal | Resolution: 1 | LSB: 1 | Item 070: Mode-3/A Code in Octal Representation.         Mode-3/A transponder squawk reply code with status bits:         - Bit 16 (V): 0=Code validated, 1=Code not validated         - Bit 15 (G): 0=Default, 1=Garbled code         - Bit 14 (L): 0=Mode-3/A code derived from reply, 1=Smoothed by tracker         - Bit 13: Reserved (set to 0)         - Bits 12-1: 4-digit octal reply code (A4..A1, B4..B1, C4..C1, D4..D1), range 0000 to 7777.         Quantum: 1 octal code. Unit: octal squawk code. Range: 0000 to 7777 (raw 0 to 65535)."),
    ["srr"] = ProtoField.string("dfdl.srr", "Number of Received SSR Replies (srr)", base.NONE, nil, 0, "Number of Received SSR Replies | Item: I048/130-SRR | Unit: replies | Resolution: 1 | LSB: 1 | Item 130 Subfield 2: Number of Received SSR Replies (SRR).         Total number of valid SSR replies received for this target plot.         Quantum (LSB): 1 reply.         Unit: replies.         Range: 0 to 255 replies."),
    ["Cat034SolarStormType"] = ProtoField.string("dfdl.Cat034SolarStormType", "Cat034SolarStormType", base.NONE, nil, 0, ""),
    ["bdsReg"] = ProtoField.string("dfdl.bdsReg", "BDS Register Identifier (BDS 1 and BDS 2) (bdsReg)", base.NONE, nil, 0, "BDS Register Identifier (BDS 1 and BDS 2) | Item: I048/250-BDS | Unit: hex | Resolution: 1 | LSB: 1 | Item 250 (BDS Register Subfield): Comm B Data Buffer Identifier.         Bits 8-5 identify BDS 1, bits 4-1 identify BDS 2:         - 0x40: BDS 4,0 (Selected Vertical Intent)         - 0x50: BDS 5,0 (Track and Turn Report)         - 0x60: BDS 6,0 (Heading and Speed Report)         Quantum: 1. Unit: hexadecimal register identifier. Range: 0x00 to 0xFF (0 to 255)."),
    ["cdm"] = ProtoField.string("dfdl.cdm", "Climb / Descent Mode (cdm)", base.NONE, nil, 0, "Climb / Descent Mode | Item: I048/170.CDM | Unit: code | Resolution: 1 | CDM: Climb / Descent Mode (0=Maintaining, 1=Climbing, 2=Descending, 3=Unknown)"),
    ["statusCom"] = ProtoField.string("dfdl.statusCom", "statusCom", base.NONE, nil, 0, ""),
    ["com"] = ProtoField.string("dfdl.com", "Communications Capability (com)", base.NONE, nil, 0, "Communications Capability | Item: I048/230.COM | Unit: code | Resolution: 1 | COM: Communications Capability (0=No comm, 1=Comm A/B, 2=Up ELM, 3=Up/Down ELM, 4=Level 5)"),
    ["mi"] = ProtoField.string("dfdl.mi", "Military Identification (mi)", base.NONE, nil, 0, "Military Identification | Item: I048/020.MI | Unit: code | Resolution: 1 | MI: Military Identification (0=No, 1=Yes)"),
    ["stat"] = ProtoField.string("dfdl.stat", "Flight Status (stat)", base.NONE, nil, 0, "Flight Status | Item: I048/230.STAT | Unit: code | Resolution: 1 | STAT: Flight Status (0=Airborne, 1=Ground, 2=Alert/Airborne, 3=Alert/Ground, 4=Alert+SPI, 5=SPI)"),
    ["height3d"] = ProtoField.string("dfdl.height3d", "Height Measured by 3D Radar (height3d)", base.NONE, nil, 0, "Height Measured by 3D Radar | Item: I048/110 | Unit: feet | Resolution: 25 | LSB: 25 ft (1/4 FL) | Item 110: Height Measured by a 3D Radar.         Altitude/height of target above mean sea level measured directly by a 3D primary radar.         Quantum (LSB): 25 feet (= 1/4 FL ~ 7.62 meters).         Unit: feet.         Range: 0 to 819100 feet (0 to 2047.75 FL, 0 to 32764 LSB)."),
    ["PolarDistanceType"] = ProtoField.string("dfdl.PolarDistanceType", "Measured Distance Rho in Polar Coordinates (PolarDistanceType)", base.NONE, nil, 0, "Measured Distance Rho in Polar Coordinates | Item: I048/040-RHO, I034/100-RHO, I001/040-RHO | Unit: NM | Resolution: 1/256 | LSB: 0.00390625 NM (~7.2297 m) | Item 040 / 100: Measured Distance (Rho) in Polar Coordinates.         Slant distance between the radar antenna and the target.         Quantum (LSB): 1/256 NM (= 0.00390625 NM ~ 7.2297 meters).         Unit: Nautical Miles (NM).         Range: 0 to 255.99609375 NM (0 to 65535 LSB)."),
    ["heading"] = ProtoField.string("dfdl.heading", "Calculated Track Heading (heading)", base.NONE, nil, 0, "Calculated Track Heading | Item: I048/200-HEADING, I001/200-HEADING | Unit: degrees | Resolution: 360/65536 | LSB: 0.005493164 deg | Item 200 (Heading Subfield): Calculated Track Heading / Course in Polar Coordinates.         Calculated track heading angle measured clockwise from True North.         Quantum (LSB): 360 / 2^16 degrees (= 360 / 65536 deg ~ 0.005493164 degrees).         Unit: degrees.         Range: 0 to 359.9945068 degrees (0 to 65535 LSB)."),
    ["dou"] = ProtoField.string("dfdl.dou", "Doubtful Correlation (dou)", base.NONE, nil, 0, "Doubtful Correlation | Item: I048/170.DOU | Unit: code | Resolution: 1 | DOU: Low Confidence in Plot-to-Track Correlation (0=Normal, 1=Low)"),
    ["BlockLengthType"] = ProtoField.string("dfdl.BlockLengthType", "Total length of ASTERIX Data Block in bytes (BlockLengthType)", base.NONE, nil, 0, "Total length of ASTERIX Data Block in bytes | Item: Length Indicator | Unit: bytes | Resolution: 1 | LSB: 1 byte | ASTERIX Data Block Length.         Total length of the data block in octets, including the Category field (1 octet)         and the Length field itself (2 octets). Minimum valid length is 3 octets.         Quantum: 1 octet (byte). Unit: bytes. Range: 3 to 65535 bytes."),
    ["rpd"] = ProtoField.string("dfdl.rpd", "Range Difference (PSR - SSR) (rpd)", base.NONE, nil, 0, "Range Difference (PSR - SSR) | Item: I048/130-RPD | Unit: NM | Resolution: 1/256 | LSB: 0.00390625 NM (~7.2297 m) | Item 130 Subfield 6: Difference in Range between PSR and SSR plots (RPD = PSR - SSR).         Quantum (LSB): 1/256 NM (~7.2297 meters). Encoded as two's complement.         Unit: Nautical Miles (NM).         Range: [-128/256 NM, +127/256 NM] = [-0.5 NM, +0.49609375 NM]."),
    ["solarStorm"] = ProtoField.string("dfdl.solarStorm", "solarStorm", base.NONE, nil, 0, ""),
    ["modePsr"] = ProtoField.string("dfdl.modePsr", "modePsr", base.NONE, nil, 0, ""),
    ["srl"] = ProtoField.string("dfdl.srl", "SSR Plot Runlength (srl)", base.NONE, nil, 0, "SSR Plot Runlength | Item: I048/130-SRL | Unit: degrees | Resolution: 360/8192 | LSB: 0.0439453125 deg | Item 130 Subfield 1: SSR Plot Runlength (SRL).         Angular width of the SSR plot.         Quantum (LSB): 360 / 2^13 degrees (= 360 / 8192 deg ~ 0.0439453125 degrees).         Unit: degrees.         Range: 0 to 11.2060547 degrees (0 to 255 LSB)."),
    ["typ"] = ProtoField.string("dfdl.typ", "Target Report Type (typ)", base.NONE, nil, 0, "Target Report Type | Item: I048/020.TYP | Unit: code | Resolution: 1 | TYP: Target Report Type (0=No detection, 1=Single PSR, 2=Single SSR, 3=SSR+PSR, 4=Single Mode S, 5=Mode S+PSR, 6=Mode S+SSR, 7=Mode S+SSR+PSR)"),
    ["aircraftId"] = ProtoField.string("dfdl.aircraftId", "Target Identification (Callsign) (aircraftId)", base.NONE, nil, 0, "Target Identification (Callsign) | Item: I048/240 | Unit: string | Item 240: Target Identification (Callsign).         8 characters representing the aircraft callsign."),
    ["sectorCrossing"] = ProtoField.string("dfdl.sectorCrossing", "sectorCrossing", base.NONE, nil, 0, ""),
    ["si"] = ProtoField.string("dfdl.si", "SI / II Capability (si)", base.NONE, nil, 0, "SI / II Capability | Item: I048/230.SI | Unit: code | Resolution: 1 | SI: SI Code Capability (0=SI code capable, 1=II code capable)"),
    ["fspec4"] = ProtoField.string("dfdl.fspec4", "fspec4", base.NONE, nil, 0, ""),
    ["Cat034SsrJammingStrobeType"] = ProtoField.string("dfdl.Cat034SsrJammingStrobeType", "Cat034SsrJammingStrobeType", base.NONE, nil, 0, ""),
    ["stationStatus"] = ProtoField.string("dfdl.stationStatus", "stationStatus", base.NONE, nil, 0, ""),
    ["spi"] = ProtoField.string("dfdl.spi", "Special Position Identification (spi)", base.NONE, nil, 0, "Special Position Identification | Item: I048/020.SPI | Unit: code | Resolution: 1 | SPI: Special Position Identification (0=Default, 1=SPI)"),
    ["thetaEnd"] = ProtoField.string("dfdl.thetaEnd", "Measured Azimuth Theta in Polar Coordinates (thetaEnd)", base.NONE, nil, 0, "Measured Azimuth Theta in Polar Coordinates | Item: I048/040-THETA, I034/100-THETA, I001/040-THETA | Unit: degrees | Resolution: 360/65536 | LSB: 0.005493164 deg | Item 040 / 100: Measured Azimuth (Theta) in Polar Coordinates.         Azimuth angle measured clockwise from True North to the target.         Quantum (LSB): 360 / 2^16 degrees (= 360 / 65536 deg ~ 0.005493164 degrees ~ 19.77 arcseconds).         Unit: degrees.         Range: 0 to 359.9945068 degrees (0 to 65535 LSB)."),
    ["apd"] = ProtoField.string("dfdl.apd", "Azimuth Difference (PSR - SSR) (apd)", base.NONE, nil, 0, "Azimuth Difference (PSR - SSR) | Item: I048/130-APD | Unit: degrees | Resolution: 360/16384 | LSB: 0.02197265625 deg | Item 130 Subfield 7: Difference in Azimuth between PSR and SSR plots (APD = PSR - SSR).         Quantum (LSB): 360 / 2^14 degrees (= 360 / 16384 deg ~ 0.02197265625 degrees). Encoded as two's complement.         Unit: degrees.         Range: [-128 * (360/16384)°, +127 * (360/16384)°] = [-2.8125°, +2.79052734375°]."),
    ["collimationError"] = ProtoField.string("dfdl.collimationError", "Collimation Error (collimationError)", base.NONE, nil, 0, "Collimation Error | Item: I034/090 | Unit: degrees | Resolution: 360/65536 | LSB: 0.005493164 deg | Item 090: Collimation Error.         Angular alignment error between the PSR axis and the SSR axis.         Quantum (LSB): 360 / 2^16 degrees (= 360 / 65536 deg ~ 0.005493164 degrees).         Unit: degrees.         Range: 0 to 359.9945 degrees."),
    ["statusMds"] = ProtoField.string("dfdl.statusMds", "statusMds", base.NONE, nil, 0, ""),
    ["mode3a"] = ProtoField.string("dfdl.mode3a", "Mode-3/A Code in Octal Representation (mode3a)", base.NONE, nil, 0, "Mode-3/A Code in Octal Representation | Item: I048/070, I001/070 | Unit: octal | Resolution: 1 | LSB: 1 | Item 070: Mode-3/A Code in Octal Representation.         Mode-3/A transponder squawk reply code with status bits:         - Bit 16 (V): 0=Code validated, 1=Code not validated         - Bit 15 (G): 0=Default, 1=Garbled code         - Bit 14 (L): 0=Mode-3/A code derived from reply, 1=Smoothed by tracker         - Bit 13: Reserved (set to 0)         - Bits 12-1: 4-digit octal reply code (A4..A1, B4..B1, C4..C1, D4..D1), range 0000 to 7777.         Quantum: 1 octal code. Unit: octal squawk code. Range: 0000 to 7777 (raw 0 to 65535)."),
    ["Cat034ModeSJammingStrobeType"] = ProtoField.string("dfdl.Cat034ModeSJammingStrobeType", "Cat034ModeSJammingStrobeType", base.NONE, nil, 0, ""),
    ["Cat034SectorCrossingType"] = ProtoField.string("dfdl.Cat034SectorCrossingType", "Cat034SectorCrossingType", base.NONE, nil, 0, ""),
    ["targetDescriptor"] = ProtoField.string("dfdl.targetDescriptor", "targetDescriptor", base.NONE, nil, 0, ""),
    ["pam"] = ProtoField.string("dfdl.pam", "Primary Plot Amplitude (pam)", base.NONE, nil, 0, "Primary Plot Amplitude | Item: I048/130-PAM | Unit: dBm | Resolution: 1 | LSB: 1 dBm | Item 130 Subfield 5: Primary Plot Amplitude (PAM).         Received signal strength of the primary radar (PSR) target plot.         Quantum (LSB): 1 dBm.         Unit: dBm.         Range: 0 to 255 dBm."),
    ["BdsRegisterType"] = ProtoField.string("dfdl.BdsRegisterType", "BDS Register Identifier (BDS 1 and BDS 2) (BdsRegisterType)", base.NONE, nil, 0, "BDS Register Identifier (BDS 1 and BDS 2) | Item: I048/250-BDS | Unit: hex | Resolution: 1 | LSB: 1 | Item 250 (BDS Register Subfield): Comm B Data Buffer Identifier.         Bits 8-5 identify BDS 1, bits 4-1 identify BDS 2:         - 0x40: BDS 4,0 (Selected Vertical Intent)         - 0x50: BDS 5,0 (Track and Turn Report)         - 0x60: BDS 6,0 (Heading and Speed Report)         Quantum: 1. Unit: hexadecimal register identifier. Range: 0x00 to 0xFF (0 to 255)."),
    ["fspec1"] = ProtoField.string("dfdl.fspec1", "Field Specification (FSPEC 1) (fspec1)", base.NONE, nil, 0, "Field Specification (FSPEC 1) | Unit: bitfield | Resolution: 1 | FSPEC Octet 1: Primary field specification."),
    ["aircraftAddress"] = ProtoField.string("dfdl.aircraftAddress", "24-bit ICAO Aircraft Address (aircraftAddress)", base.NONE, nil, 0, "24-bit ICAO Aircraft Address | Item: I048/220 | Unit: hex | Resolution: 1 | LSB: 1 | Item 220: Aircraft Address.         Unique 24-bit ICAO technical aircraft address assigned to aircraft transponder.         Quantum: 1. Unit: 24-bit hexadecimal code. Range: 0x000000 to 0xFFFFFF (0 to 16777215)."),
    ["CartesianCoordinateType"] = ProtoField.string("dfdl.CartesianCoordinateType", "Calculated Position in Cartesian Coordinates (X / Y) (CartesianCoordinateType)", base.NONE, nil, 0, "Calculated Position in Cartesian Coordinates (X / Y) | Item: I048/042, I001/042 | Unit: NM | Resolution: 1/64 | LSB: 0.015625 NM (~28.918 m) | Item 042: Calculated Position in Cartesian Coordinates (X or Y).         Position coordinate relative to the radar sensor in signed two's complement.         Quantum (LSB): 1/64 NM (= 0.015625 NM ~ 28.918 meters) in Cat 048;                        1/16 NM (= 0.0625 NM ~ 115.67 meters) in Cat 001.         Unit: Nautical Miles (NM).         Range: -512.0 NM to +511.984375 NM (-32768 to 32767 LSB at 1/64 NM)."),
    ["AsterixRecord"] = ProtoField.string("dfdl.AsterixRecord", "AsterixRecord", base.NONE, nil, 0, ""),
    ["jammingStrobe"] = ProtoField.string("dfdl.jammingStrobe", "jammingStrobe", base.NONE, nil, 0, ""),
    ["Cat001Record"] = ProtoField.string("dfdl.Cat001Record", "Cat001Record", base.NONE, nil, 0, ""),
    ["TruncatedTimeOfDayType"] = ProtoField.string("dfdl.TruncatedTimeOfDayType", "Truncated Time of Day (TruncatedTimeOfDayType)", base.NONE, nil, 0, "Truncated Time of Day | Item: I001/141 | Unit: seconds | Resolution: 1/128 | LSB: 0.0078125 s (1/128 s) | Item 141: Truncated Time of Day.         Time of day truncated to the 16 least significant bits of the standard Time of Day.         Quantum (LSB): 1/128 s (~7.8125 milliseconds).         Unit: seconds.         Range: [0 s, 512 s) represented as [0, 65535] (wraps around every 512 seconds)."),
    ["me"] = ProtoField.string("dfdl.me", "Military Emergency (me)", base.NONE, nil, 0, "Military Emergency | Item: I048/020.ME | Unit: code | Resolution: 1 | ME: Military Emergency (0=No, 1=Yes)"),
    ["sic"] = ProtoField.string("dfdl.sic", "System Identification Code (SIC) (sic)", base.NONE, nil, 0, "System Identification Code (SIC) | Item: I034/010-SIC, I048/010-SIC, I001/010-SIC | Unit: code | Resolution: 1 | LSB: 1 | Item 010: System Identification Code (SIC).         Identifies the radar sensor station within the SAC."),
    ["longitude"] = ProtoField.string("dfdl.longitude", "WGS-84 Geographic Latitude / Longitude (longitude)", base.NONE, nil, 0, "WGS-84 Geographic Latitude / Longitude | Item: I034/120-LATLON | Unit: degrees | Resolution: 180/8388608 | LSB: 0.00002145767 deg | Item 120 (Latitude / Longitude Subfields): 3D Position of Data Source in WGS-84.         Geographic coordinates in two's complement 24-bit binary representation.         Quantum (LSB): 180 / 2^23 degrees (= 180 / 8388608 deg ~ 0.00002145767 degrees ~ 2.146 cm at equator).         Unit: degrees.         Range: [-90.0°, +90.0°] for Latitude; [-180.0°, +180.0°] for Longitude."),
    ["rho"] = ProtoField.string("dfdl.rho", "Measured Distance Rho in Polar Coordinates (rho)", base.NONE, nil, 0, "Measured Distance Rho in Polar Coordinates | Item: I048/040-RHO, I034/100-RHO, I001/040-RHO | Unit: NM | Resolution: 1/256 | LSB: 0.00390625 NM (~7.2297 m) | Item 040 / 100: Measured Distance (Rho) in Polar Coordinates.         Slant distance between the radar antenna and the target.         Quantum (LSB): 1/256 NM (= 0.00390625 NM ~ 7.2297 meters).         Unit: Nautical Miles (NM).         Range: 0 to 255.99609375 NM (0 to 65535 LSB)."),
    ["sup"] = ProtoField.string("dfdl.sup", "Correlation Suppression (sup)", base.NONE, nil, 0, "Correlation Suppression | Item: I048/170.SUP | Unit: code | Resolution: 1 | SUP: Track-to-Track Correlation Suppression (0=No, 1=Yes)"),
    ["BdsDataType"] = ProtoField.string("dfdl.BdsDataType", "Mode S Comm B MB Data (56 bits) (BdsDataType)", base.NONE, nil, 0, "Mode S Comm B MB Data (56 bits) | Item: I048/250-DATA | Unit: bytes | Resolution: 1 | LSB: 1 bit | Item 250 (BDS Data Subfield): Mode S Comm B Message Data.         56-bit MB data payload extracted from Mode S Comm-B replies (e.g. BDS 4,0, 5,0, 6,0).         Quantum: 56 bits (7 octets). Unit: binary hex payload."),
    ["theta"] = ProtoField.string("dfdl.theta", "Measured Azimuth Theta in Polar Coordinates (theta)", base.NONE, nil, 0, "Measured Azimuth Theta in Polar Coordinates | Item: I048/040-THETA, I034/100-THETA, I001/040-THETA | Unit: degrees | Resolution: 360/65536 | LSB: 0.005493164 deg | Item 040 / 100: Measured Azimuth (Theta) in Polar Coordinates.         Azimuth angle measured clockwise from True North to the target.         Quantum (LSB): 360 / 2^16 degrees (= 360 / 65536 deg ~ 0.005493164 degrees ~ 19.77 arcseconds).         Unit: degrees.         Range: 0 to 359.9945068 degrees (0 to 65535 LSB)."),
    ["PlotCharacteristicsType"] = ProtoField.string("dfdl.PlotCharacteristicsType", "Radar Plot Characteristics (PlotCharacteristicsType)", base.NONE, nil, 0, "Radar Plot Characteristics | Item: I048/130 | Unit: compound | Resolution: compound | LSB: compound | Item 130: Radar Plot Characteristics.         Compound data item containing SSR/PSR plot runlength, amplitude, and range/azimuth differences.         Quantum: Compound. Unit: compound structure."),
    ["flightLevel"] = ProtoField.string("dfdl.flightLevel", "Flight Level in Binary Representation (flightLevel)", base.NONE, nil, 0, "Flight Level in Binary Representation | Item: I048/090, I001/090 | Unit: FL | Resolution: 1/4 | LSB: 0.25 FL (25 ft ~ 7.62 m) | Item 090: Flight Level in Binary Representation.         Pressure altitude measured by aircraft barometric altimeter transponder:         - Bit 16 (V): 0=Code validated, 1=Code not validated         - Bit 15 (G): 0=Default, 1=Garbled code         - Bits 14-1: Flight Level encoded in signed two's complement.         Quantum (LSB): 1/4 FL (= 25 feet ~ 7.62 meters).         Unit: FL (Flight Level; 1 FL = 100 feet).         Range: -12 FL (-300 ft) to +1270 FL (+127000 ft). Raw range: 0 to 65535."),
    ["tre"] = ProtoField.string("dfdl.tre", "Track Termination Indicator (tre)", base.NONE, nil, 0, "Track Termination Indicator | Item: I048/170.TRE | Unit: code | Resolution: 1 | TRE: End of Track Lifetime (0=Track alive, 1=Track terminated)"),
    ["err"] = ProtoField.string("dfdl.err", "Error Condition (err)", base.NONE, nil, 0, "Error Condition | Item: I048/020.ERR | Unit: code | Resolution: 1 | ERR: Error Condition (0=No error, 1=Error)"),
    ["cartesianX"] = ProtoField.string("dfdl.cartesianX", "Calculated Position in Cartesian Coordinates (X / Y) (cartesianX)", base.NONE, nil, 0, "Calculated Position in Cartesian Coordinates (X / Y) | Item: I048/042, I001/042 | Unit: NM | Resolution: 1/64 | LSB: 0.015625 NM (~28.918 m) | Item 042: Calculated Position in Cartesian Coordinates (X or Y).         Position coordinate relative to the radar sensor in signed two's complement.         Quantum (LSB): 1/64 NM (= 0.015625 NM ~ 28.918 meters) in Cat 048;                        1/16 NM (= 0.0625 NM ~ 115.67 meters) in Cat 001.         Unit: Nautical Miles (NM).         Range: -512.0 NM to +511.984375 NM (-32768 to 32767 LSB at 1/64 NM)."),
    ["xpp"] = ProtoField.string("dfdl.xpp", "X-Pulse Present (xpp)", base.NONE, nil, 0, "X-Pulse Present | Item: I048/020.XPP | Unit: code | Resolution: 1 | XPP: X-Pulse Present (0=No, 1=Yes)"),
    ["Height3dType"] = ProtoField.string("dfdl.Height3dType", "Height Measured by 3D Radar (Height3dType)", base.NONE, nil, 0, "Height Measured by 3D Radar | Item: I048/110 | Unit: feet | Resolution: 25 | LSB: 25 ft (1/4 FL) | Item 110: Height Measured by a 3D Radar.         Altitude/height of target above mean sea level measured directly by a 3D primary radar.         Quantum (LSB): 25 feet (= 1/4 FL ~ 7.62 meters).         Unit: feet.         Range: 0 to 819100 feet (0 to 2047.75 FL, 0 to 32764 LSB)."),
    ["Cat034OtherServiceType"] = ProtoField.string("dfdl.Cat034OtherServiceType", "Cat034OtherServiceType", base.NONE, nil, 0, ""),
    ["mah"] = ProtoField.string("dfdl.mah", "Manoeuvre Detection (mah)", base.NONE, nil, 0, "Manoeuvre Detection | Item: I048/170.MAH | Unit: code | Resolution: 1 | MAH: Horizontal Manoeuvre Detection (0=No, 1=Yes)"),
    ["BdsRepType"] = ProtoField.string("dfdl.BdsRepType", "BDS Report Repetition Factor (BdsRepType)", base.NONE, nil, 0, "BDS Report Repetition Factor | Item: I048/250-REP | Unit: count | Resolution: 1 | Item 250: BDS Report Repetition Factor.         Number of BDS reports following."),
    ["HeadingType"] = ProtoField.string("dfdl.HeadingType", "Calculated Track Heading (HeadingType)", base.NONE, nil, 0, "Calculated Track Heading | Item: I048/200-HEADING, I001/200-HEADING | Unit: degrees | Resolution: 360/65536 | LSB: 0.005493164 deg | Item 200 (Heading Subfield): Calculated Track Heading / Course in Polar Coordinates.         Calculated track heading angle measured clockwise from True North.         Quantum (LSB): 360 / 2^16 degrees (= 360 / 65536 deg ~ 0.005493164 degrees).         Unit: degrees.         Range: 0 to 359.9945068 degrees (0 to 65535 LSB)."),
    ["DataSourceIdType"] = ProtoField.string("dfdl.DataSourceIdType", "Data Source Identifier (SAC / SIC) (DataSourceIdType)", base.NONE, nil, 0, "Data Source Identifier (SAC / SIC) | Item: I034/010, I048/010, I001/010, I002/010 | Unit: code | Resolution: 1 | LSB: 1 | Item 010: Data Source Identifier (SAC / SIC).         - SAC (System Area Code): Identifies the nation/region or airspace division.         - SIC (System Identification Code): Identifies the radar sensor system within the SAC.         Quantum: 1. Unit: dimensionless code. Range: 0 to 255."),
    ["dataFilter"] = ProtoField.string("dfdl.dataFilter", "dataFilter", base.NONE, nil, 0, ""),
    ["record"] = ProtoField.string("dfdl.record", "record", base.NONE, nil, 0, ""),
    ["cartesianY"] = ProtoField.string("dfdl.cartesianY", "Calculated Position in Cartesian Coordinates (X / Y) (cartesianY)", base.NONE, nil, 0, "Calculated Position in Cartesian Coordinates (X / Y) | Item: I048/042, I001/042 | Unit: NM | Resolution: 1/64 | LSB: 0.015625 NM (~28.918 m) | Item 042: Calculated Position in Cartesian Coordinates (X or Y).         Position coordinate relative to the radar sensor in signed two's complement.         Quantum (LSB): 1/64 NM (= 0.015625 NM ~ 28.918 meters) in Cat 048;                        1/16 NM (= 0.0625 NM ~ 115.67 meters) in Cat 001.         Unit: Nautical Miles (NM).         Range: -512.0 NM to +511.984375 NM (-32768 to 32767 LSB at 1/64 NM)."),
    ["b1b"] = ProtoField.string("dfdl.b1b", "BDS 0,5 bits 37-40 (b1b)", base.NONE, nil, 0, "BDS 0,5 bits 37-40 | Item: I048/230.B1B | Unit: code | Resolution: 1 | B1B: BDS 0,5 bits 37-40"),
    ["rhoStart"] = ProtoField.string("dfdl.rhoStart", "Measured Distance Rho in Polar Coordinates (rhoStart)", base.NONE, nil, 0, "Measured Distance Rho in Polar Coordinates | Item: I048/040-RHO, I034/100-RHO, I001/040-RHO | Unit: NM | Resolution: 1/256 | LSB: 0.00390625 NM (~7.2297 m) | Item 040 / 100: Measured Distance (Rho) in Polar Coordinates.         Slant distance between the radar antenna and the target.         Quantum (LSB): 1/256 NM (= 0.00390625 NM ~ 7.2297 meters).         Unit: Nautical Miles (NM).         Range: 0 to 255.99609375 NM (0 to 65535 LSB)."),
    ["SamType"] = ProtoField.string("dfdl.SamType", "SSR Reply Amplitude (SamType)", base.NONE, nil, 0, "SSR Reply Amplitude | Item: I048/130-SAM | Unit: dBm | Resolution: 1 | LSB: 1 dBm | Item 130 Subfield 3: SSR Reply Amplitude (SAM).         Average received signal strength of SSR replies referenced to receiver detection threshold.         Quantum (LSB): 1 dBm.         Unit: dBm.         Range: 0 to 255 dBm."),
    ["tcc"] = ProtoField.string("dfdl.tcc", "Coordinate Conversion Mode (tcc)", base.NONE, nil, 0, "Coordinate Conversion Mode | Item: I048/170.TCC | Unit: code | Resolution: 1 | TCC: Coordinate Conversion Mode (0=Radar plane, 1=Sensor to local)"),
    ["timeOfDay"] = ProtoField.string("dfdl.timeOfDay", "Time of Day (UTC since midnight) (timeOfDay)", base.NONE, nil, 0, "Time of Day (UTC since midnight) | Item: I034/030, I048/140, I002/030 | Unit: seconds | Resolution: 1/128 | LSB: 0.0078125 s (1/128 s) | Item 030 / 140: Time of Day.         Absolute time stamping expressed as UTC elapsed time since midnight.         Quantum (LSB): 1/128 s (~7.8125 milliseconds).         Unit: seconds.         Range: [0 s, 24 h) = [0, 86400 s) represented as integer [0, 11059200].         Maximum 24-bit integer: 16777215 (~36.4 hours)."),
    ["rhoEnd"] = ProtoField.string("dfdl.rhoEnd", "Measured Distance Rho in Polar Coordinates (rhoEnd)", base.NONE, nil, 0, "Measured Distance Rho in Polar Coordinates | Item: I048/040-RHO, I034/100-RHO, I001/040-RHO | Unit: NM | Resolution: 1/256 | LSB: 0.00390625 NM (~7.2297 m) | Item 040 / 100: Measured Distance (Rho) in Polar Coordinates.         Slant distance between the radar antenna and the target.         Quantum (LSB): 1/256 NM (= 0.00390625 NM ~ 7.2297 meters).         Unit: Nautical Miles (NM).         Range: 0 to 255.99609375 NM (0 to 65535 LSB)."),
    ["Cat034Record"] = ProtoField.string("dfdl.Cat034Record", "Cat034Record", base.NONE, nil, 0, ""),
    ["rad"] = ProtoField.string("dfdl.rad", "Maintaining Sensor Type (rad)", base.NONE, nil, 0, "Maintaining Sensor Type | Item: I048/170.RAD | Unit: code | Resolution: 1 | RAD: Type of Sensor(s) Maintaining Track (0=Combined, 1=PSR, 2=SSR/Mode S, 3=Invalid)"),
    ["FlightLevelType"] = ProtoField.string("dfdl.FlightLevelType", "Flight Level in Binary Representation (FlightLevelType)", base.NONE, nil, 0, "Flight Level in Binary Representation | Item: I048/090, I001/090 | Unit: FL | Resolution: 1/4 | LSB: 0.25 FL (25 ft ~ 7.62 m) | Item 090: Flight Level in Binary Representation.         Pressure altitude measured by aircraft barometric altimeter transponder:         - Bit 16 (V): 0=Code validated, 1=Code not validated         - Bit 15 (G): 0=Default, 1=Garbled code         - Bits 14-1: Flight Level encoded in signed two's complement.         Quantum (LSB): 1/4 FL (= 25 feet ~ 7.62 meters).         Unit: FL (Flight Level; 1 FL = 100 feet).         Range: -12 FL (-300 ft) to +1270 FL (+127000 ft). Raw range: 0 to 65535."),
    ["MessageTypeType"] = ProtoField.string("dfdl.MessageTypeType", "Service Message Type (MessageTypeType)", base.NONE, nil, 0, "Service Message Type | Item: I034/000, I002/000 | Unit: type_id | Resolution: 1 | LSB: 1 | Item 000: Message Type.         Defines the service message type for Monoradar Service Messages:         - 1: North Marker Message         - 2: Sector Crossing Message         - 3: Geographical Filtering Message (Cat 034) / Station Status Message (Cat 002)         - 4: Jamming Strobe Message         - 5: Solar Storm Message         - 6: SSR Jamming Strobe Message         - 7: Mode S Jamming Strobe Message         - 8..255: Other service / reserved messages         Quantum: 1. Unit: message type ID. Range: 1 to 255."),
    ["b1a"] = ProtoField.string("dfdl.b1a", "BDS 0,5 bit 16 status (b1a)", base.NONE, nil, 0, "BDS 0,5 bit 16 status | Item: I048/230.B1A | Unit: code | Resolution: 1 | B1A: BDS 0,5 bit 16 status"),
    ["geographicalFiltering"] = ProtoField.string("dfdl.geographicalFiltering", "geographicalFiltering", base.NONE, nil, 0, ""),
    ["sac"] = ProtoField.string("dfdl.sac", "System Area Code (SAC) (sac)", base.NONE, nil, 0, "System Area Code (SAC) | Item: I034/010-SAC, I048/010-SAC, I001/010-SAC | Unit: code | Resolution: 1 | LSB: 1 | Item 010: System Area Code (SAC).         Identifies the nation, region, or airspace division."),
    ["thetaStart"] = ProtoField.string("dfdl.thetaStart", "Measured Azimuth Theta in Polar Coordinates (thetaStart)", base.NONE, nil, 0, "Measured Azimuth Theta in Polar Coordinates | Item: I048/040-THETA, I034/100-THETA, I001/040-THETA | Unit: degrees | Resolution: 360/65536 | LSB: 0.005493164 deg | Item 040 / 100: Measured Azimuth (Theta) in Polar Coordinates.         Azimuth angle measured clockwise from True North to the target.         Quantum (LSB): 360 / 2^16 degrees (= 360 / 65536 deg ~ 0.005493164 degrees ~ 19.77 arcseconds).         Unit: degrees.         Range: 0 to 359.9945068 degrees (0 to 65535 LSB)."),
    ["AsterixPacketType"] = ProtoField.string("dfdl.AsterixPacketType", "AsterixPacketType", base.NONE, nil, 0, ""),
    ["DataCategoryType"] = ProtoField.string("dfdl.DataCategoryType", "ASTERIX Data Category number (DataCategoryType)", base.NONE, nil, 0, "ASTERIX Data Category number | Item: Data Category | Unit: dimensionless | Resolution: 1 | LSB: 1 | ASTERIX Data Category.         Identifies the message category according to Eurocontrol ASTERIX standards:         - Cat 034: Transmission of Monoradar Service Messages (Ed 1.28)         - Cat 048: Transmission of Monoradar Target Reports (Ed 1.21)         - Cat 001: Transmission of Monoradar Data (Ed 1.2)         - Cat 002: Transmission of Monoradar Service Messages (Ed 1.0)         Quantum: 1 (category number). Unit: dimensionless. Range: 0 to 255."),
    ["rdp"] = ProtoField.string("dfdl.rdp", "Report from RDP (rdp)", base.NONE, nil, 0, "Report from RDP | Item: I048/020.RDP | Unit: code | Resolution: 1 | RDP: Provided by Surveillance Chain (0) vs RDP (1)"),
    ["SrlType"] = ProtoField.string("dfdl.SrlType", "SSR Plot Runlength (SrlType)", base.NONE, nil, 0, "SSR Plot Runlength | Item: I048/130-SRL | Unit: degrees | Resolution: 360/8192 | LSB: 0.0439453125 deg | Item 130 Subfield 1: SSR Plot Runlength (SRL).         Angular width of the SSR plot.         Quantum (LSB): 360 / 2^13 degrees (= 360 / 8192 deg ~ 0.0439453125 degrees).         Unit: degrees.         Range: 0 to 11.2060547 degrees (0 to 255 LSB)."),
    ["modeMds"] = ProtoField.string("dfdl.modeMds", "modeMds", base.NONE, nil, 0, ""),
    ["AntennaRotationPeriodType"] = ProtoField.string("dfdl.AntennaRotationPeriodType", "Antenna Rotation Period (AntennaRotationPeriodType)", base.NONE, nil, 0, "Antenna Rotation Period | Item: I034/041, I002/041 | Unit: seconds | Resolution: 1/128 | LSB: 0.0078125 s (1/128 s) | Item 041: Antenna Rotation Period.         Measured duration of the previous full 360-degree antenna revolution.         Quantum (LSB): 1/128 s (~7.8125 milliseconds).         Unit: seconds.         Range: 0 to 511.9921875 seconds (0 to 65535 LSB)."),
    ["length"] = ProtoField.string("dfdl.length", "Total length of ASTERIX Data Block in bytes (length)", base.NONE, nil, 0, "Total length of ASTERIX Data Block in bytes | Item: Length Indicator | Unit: bytes | Resolution: 1 | LSB: 1 byte | ASTERIX Data Block Length.         Total length of the data block in octets, including the Category field (1 octet)         and the Length field itself (2 octets). Minimum valid length is 3 octets.         Quantum: 1 octet (byte). Unit: bytes. Range: 3 to 65535 bytes."),
    ["bdsReport"] = ProtoField.string("dfdl.bdsReport", "bdsReport", base.NONE, nil, 0, ""),
    ["Cat034JammingStrobeType"] = ProtoField.string("dfdl.Cat034JammingStrobeType", "Cat034JammingStrobeType", base.NONE, nil, 0, ""),
    ["otherService"] = ProtoField.string("dfdl.otherService", "otherService", base.NONE, nil, 0, ""),
}
proto.fields = { proto_fields["AircraftIdType"], proto_fields["statusSsr"], proto_fields["trackNumber"], proto_fields["messageType"], proto_fields["Cat034NorthMarkerType"], proto_fields["truncatedTimeOfDay"], proto_fields["stationProcessingMode"], proto_fields["PolarAzimuthType"], proto_fields["spare"], proto_fields["messageCount"], proto_fields["gho"], proto_fields["prl"], proto_fields["antennaRotationPeriod"], proto_fields["PamType"], proto_fields["Fspec2Type"], proto_fields["ssrJammingStrobe"], proto_fields["stationConfig"], proto_fields["DataSourceHeightType"], proto_fields["sim"], proto_fields["bdsRep"], proto_fields["Cat002Record"], proto_fields["SicType"], proto_fields["AircraftAddressType"], proto_fields["SrrType"], proto_fields["mssc"], proto_fields["category"], proto_fields["fspec3"], proto_fields["arc"], proto_fields["latitude"], proto_fields["modeFspec"], proto_fields["Cat034GeographicalFilteringType"], proto_fields["northMarker"], proto_fields["PrlType"], proto_fields["targetDescriptorExt"], proto_fields["trackStatus"], proto_fields["SacType"], proto_fields["sectorNumber"], proto_fields["TrackNumberType"], proto_fields["RpdType"], proto_fields["groundSpeed"], proto_fields["statusFspec"], proto_fields["height"], proto_fields["modeSsr"], proto_fields["AsterixPacket"], proto_fields["CollimationErrorType"], proto_fields["MessageCountType"], proto_fields["Wgs84CoordinateType"], proto_fields["Fspec1Type"], proto_fields["ApdType"], proto_fields["commCapability"], proto_fields["TimeOfDayType"], proto_fields["cnf"], proto_fields["modeSJammingStrobe"], proto_fields["statusPsr"], proto_fields["plotCharFspec"], proto_fields["sam"], proto_fields["trackStatusExt"], proto_fields["tst"], proto_fields["modeCom"], proto_fields["fspec2"], proto_fields["Fspec3Type"], proto_fields["rab"], proto_fields["fx"], proto_fields["GroundSpeedType"], proto_fields["aic"], proto_fields["bdsData"], proto_fields["AsterixDatagram"], proto_fields["Cat048Record"], proto_fields["SectorNumberType"], proto_fields["foeFri"], proto_fields["Mode3ACodeType"], proto_fields["srr"], proto_fields["Cat034SolarStormType"], proto_fields["bdsReg"], proto_fields["cdm"], proto_fields["statusCom"], proto_fields["com"], proto_fields["mi"], proto_fields["stat"], proto_fields["height3d"], proto_fields["PolarDistanceType"], proto_fields["heading"], proto_fields["dou"], proto_fields["BlockLengthType"], proto_fields["rpd"], proto_fields["solarStorm"], proto_fields["modePsr"], proto_fields["srl"], proto_fields["typ"], proto_fields["aircraftId"], proto_fields["sectorCrossing"], proto_fields["si"], proto_fields["fspec4"], proto_fields["Cat034SsrJammingStrobeType"], proto_fields["stationStatus"], proto_fields["spi"], proto_fields["thetaEnd"], proto_fields["apd"], proto_fields["collimationError"], proto_fields["statusMds"], proto_fields["mode3a"], proto_fields["Cat034ModeSJammingStrobeType"], proto_fields["Cat034SectorCrossingType"], proto_fields["targetDescriptor"], proto_fields["pam"], proto_fields["BdsRegisterType"], proto_fields["fspec1"], proto_fields["aircraftAddress"], proto_fields["CartesianCoordinateType"], proto_fields["AsterixRecord"], proto_fields["jammingStrobe"], proto_fields["Cat001Record"], proto_fields["TruncatedTimeOfDayType"], proto_fields["me"], proto_fields["sic"], proto_fields["longitude"], proto_fields["rho"], proto_fields["sup"], proto_fields["BdsDataType"], proto_fields["theta"], proto_fields["PlotCharacteristicsType"], proto_fields["flightLevel"], proto_fields["tre"], proto_fields["err"], proto_fields["cartesianX"], proto_fields["xpp"], proto_fields["Height3dType"], proto_fields["Cat034OtherServiceType"], proto_fields["mah"], proto_fields["BdsRepType"], proto_fields["HeadingType"], proto_fields["DataSourceIdType"], proto_fields["dataFilter"], proto_fields["record"], proto_fields["cartesianY"], proto_fields["b1b"], proto_fields["rhoStart"], proto_fields["SamType"], proto_fields["tcc"], proto_fields["timeOfDay"], proto_fields["rhoEnd"], proto_fields["Cat034Record"], proto_fields["rad"], proto_fields["FlightLevelType"], proto_fields["MessageTypeType"], proto_fields["b1a"], proto_fields["geographicalFiltering"], proto_fields["sac"], proto_fields["thetaStart"], proto_fields["AsterixPacketType"], proto_fields["DataCategoryType"], proto_fields["rdp"], proto_fields["SrlType"], proto_fields["modeMds"], proto_fields["AntennaRotationPeriodType"], proto_fields["length"], proto_fields["bdsReport"], proto_fields["Cat034JammingStrobeType"], proto_fields["otherService"] }

local native_dfdl = nil
local dylib_candidates = {
    "/Users/jean-baptiste/AI-DFDL/target/release/libdfdl_wireshark.dylib",
    "/Users/jean-baptiste/AI-DFDL/target/release/libdfdl_wireshark.dylib",
    "dfdl-wireshark.so",
    "libdfdl_wireshark.dylib",
    "libdfdl_wireshark.so"
}
for _, cand in ipairs(dylib_candidates) do
    local dylib_loader = package.loadlib(cand, "luaopen_dfdl_wireshark")
    if dylib_loader then
        local ok, mod = pcall(dylib_loader)
        if ok and mod then
            native_dfdl = mod
            break
        end
    end
end

-- Lightweight pure Lua JSON decoder (zero external dependencies)
local function parse_json_value(str, idx)
    idx = string.find(str, "%S", idx) or idx
    local c = string.sub(str, idx, idx)
    if c == '"' then
        local j = idx + 1
        local out = {}
        while j <= #str do
            local ch = string.sub(str, j, j)
            if ch == '\\' then
                local esc = string.sub(str, j + 1, j + 1)
                if esc == '"' or esc == '\\' or esc == '/' then
                    table.insert(out, esc)
                    j = j + 2
                elseif esc == 'n' then
                    table.insert(out, '\n')
                    j = j + 2
                elseif esc == 'r' then
                    table.insert(out, '\r')
                    j = j + 2
                elseif esc == 't' then
                    table.insert(out, '\t')
                    j = j + 2
                elseif esc == 'u' then
                    local hex_str = string.sub(str, j + 2, j + 5)
                    local cp = tonumber(hex_str, 16)
                    if cp then
                        if cp < 128 then
                            table.insert(out, string.char(cp))
                        else
                            table.insert(out, utf8.char(cp))
                        end
                    end
                    j = j + 6
                else
                    table.insert(out, esc)
                    j = j + 2
                end
            elseif ch == '"' then
                return table.concat(out), j + 1
            else
                table.insert(out, ch)
                j = j + 1
            end
        end
        return table.concat(out), j
    elseif c == '{' then
        local obj = {}
        idx = idx + 1
        while idx <= #str do
            idx = string.find(str, "%S", idx) or idx
            if string.sub(str, idx, idx) == '}' then return obj, idx + 1 end
            local key, next_i = parse_json_value(str, idx)
            idx = string.find(str, ":", next_i) + 1
            local val, after_val = parse_json_value(str, idx)
            obj[key] = val
            idx = string.find(str, "%S", after_val) or after_val
            if string.sub(str, idx, idx) == ',' then
                idx = idx + 1
            elseif string.sub(str, idx, idx) == '}' then
                return obj, idx + 1
            end
        end
        return obj, idx
    elseif c == '[' then
        local arr = {}
        idx = idx + 1
        while idx <= #str do
            idx = string.find(str, "%S", idx) or idx
            if string.sub(str, idx, idx) == ']' then return arr, idx + 1 end
            local val, next_i = parse_json_value(str, idx)
            table.insert(arr, val)
            idx = string.find(str, "%S", next_i) or next_i
            if string.sub(str, idx, idx) == ',' then
                idx = idx + 1
            elseif string.sub(str, idx, idx) == ']' then
                return arr, idx + 1
            end
        end
        return arr, idx
    elseif string.find(c, "[%d%-]") then
        local _, stop, num = string.find(str, "([%d%.%-%+eE]+)", idx)
        return tonumber(num) or num, stop + 1
    elseif string.sub(str, idx, idx + 3) == "true" then
        return true, idx + 4
    elseif string.sub(str, idx, idx + 4) == "false" then
        return false, idx + 5
    elseif string.sub(str, idx, idx + 3) == "null" then
        return nil, idx + 4
    end
    return nil, idx + 1
end

local function decode_json(s)
    if not s or s == "" then return nil end
    local val, _ = parse_json_value(s, 1)
    return val
end

-- Recursively populates Wireshark tree nodes with overlay tooltips
local function populate_node(tree_item, tvb, node)
    if not node then return end

    local sub_item
    local pf = proto_fields[node.name]

    if pf and (not node.children or #node.children == 0) then
        local display_val = node.converted
        if not display_val or display_val == "" then
            display_val = node.raw or ""
        end
        sub_item = tree_item:add(pf, tvb(0, 0), tostring(display_val))
        local item_label = node.label
        if not item_label or item_label == "" then
            local prefix = node.name
            if node.description and node.description ~= "" and node.description ~= node.name then
                prefix = string.format("%s (%s)", node.description, node.name)
            end
            item_label = string.format("%s: %s", prefix, tostring(display_val))
        end
        sub_item:set_text(item_label)
    else
        local label = node.label or node.name or "Item"
        if (not node.label or node.label == node.name) and node.description and node.description ~= "" and node.description ~= node.name then
            label = string.format("%s (%s)", node.description, node.name)
        end
        sub_item = tree_item:add(proto, tvb(), label)
    end

    if node.children and #node.children > 0 then
        for _, child in ipairs(node.children) do
            populate_node(sub_item, tvb, child)
        end
    end
end

-- Resolves the effective schema path from Preferences, environment, or hardcoded default
local function get_effective_schema()
    local p = proto.prefs.schema_path
    if p and p ~= "" then
        local f = io.open(p, "r")
        if f then
            f:close()
            return p
        end
    end
    local env_p = os.getenv("DFDL_SCHEMA")
    if env_p and env_p ~= "" then
        local f = io.open(env_p, "r")
        if f then
            f:close()
            return env_p
        end
    end
    return "/Users/jean-baptiste/AI-DFDL/crates/dfdl-example/schemas/asterix.dfdl.xsd"
end

-- Resolves the CLI binary executable path
local function find_cli_bin()
    local p = proto.prefs.cli_path
    if p and p ~= "" and p ~= "dfdl-wireshark" then
        local f = io.open(p, "r")
        if f then
            f:close()
            return p
        end
    end
    local candidates = {
        "/Users/jean-baptiste/AI-DFDL/target/release/dfdl-wireshark",
        "/Users/jean-baptiste/AI-DFDL/target/release/dfdl-wireshark",
        "/usr/local/bin/dfdl-wireshark",
        "/opt/homebrew/bin/dfdl-wireshark",
        "/Users/jean-baptiste/AI-DFDL/target/debug/dfdl-wireshark",
    }
    for _, cand in ipairs(candidates) do
        local f = io.open(cand, "r")
        if f then
            f:close()
            return cand
        end
    end
    return "dfdl-wireshark"
end

-- Main protocol dissector function
function proto.dissector(tvb, pinfo, tree)
    local length = tvb:len()
    if length == 0 then return end

    local payload_tvb = tvb
    if length > 42 then
        local b12 = tvb(12, 1):uint()
        local b13 = tvb(13, 1):uint()
        if b12 == 0x08 and b13 == 0x00 then
            local ip_proto = tvb(23, 1):uint()
            if ip_proto == 17 then
                local ihl = (tvb(14, 1):uint() % 16) * 4
                local udp_offset = 14 + ihl + 8
                if length >= udp_offset then
                    payload_tvb = tvb(udp_offset)
                end
            end
        end
    end

    pinfo.cols.protocol = "dfdl"
    local active_schema = get_effective_schema()
    if not active_schema then
        local root_tree = tree:add(proto, payload_tvb(), "DFDL (DFDL Engine): [No Schema Loaded]")
        root_tree:add(proto, payload_tvb(), "Open Edit -> Preferences -> Protocols -> DFDL (DFDL Engine) to select a .dfdl.xsd schema file")
        return
    end

    local hex_data = tostring(payload_tvb:bytes():tohex())
    local json_str = nil

    if native_dfdl and native_dfdl.dissect_hex then
        json_str = native_dfdl.dissect_hex(active_schema, hex_data)
    else
        local cli = find_cli_bin()
        local cmd = string.format("%s dissect-hex --schema '%s' %s 2>/dev/null", cli, active_schema, hex_data)
        local handle = io.popen(cmd)
        if handle then
            json_str = handle:read("*a")
            handle:close()
        end
    end

    local tree_data = decode_json(json_str)
    if tree_data and tree_data.root then
        local root_tree = tree:add(proto, payload_tvb(), string.format("DFDL (DFDL Engine) (%d bytes)", payload_tvb:len()))
        populate_node(root_tree, payload_tvb, tree_data.root)
    end
end

-- Dynamic port table binding based on Preferences
local registered_ports = {}

local function update_port_bindings()
    local udp_table = DissectorTable.get("udp.port")
    for _, p in ipairs(registered_ports) do
        udp_table:remove(p, proto)
    end
    registered_ports = {}

    local range_str = tostring(proto.prefs.ports or "21111-22135")
    for part in string.gmatch(range_str, "[^,]+") do
        local start_p, end_p = string.match(part, "(%d+)%s*-%s*(%d+)")
        if start_p and end_p then
            for p = tonumber(start_p), tonumber(end_p) do
                udp_table:add(p, proto)
                table.insert(registered_ports, p)
            end
        else
            local single = tonumber(string.match(part, "%d+"))
            if single then
                udp_table:add(single, proto)
                table.insert(registered_ports, single)
            end
        end
    end
end

function proto.prefs_changed()
    update_port_bindings()
end

update_port_bindings()
register_postdissector(proto)
