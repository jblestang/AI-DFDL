//! Hand-written, zero-allocation native Rust encoder and decoder for FakeTDL tactical messaging.
//!
//! Provides direct, safe binary serialization and deserialization for:
//! - [`TrackMessage`]: 64-byte tactical track report (initiator `'T'`)
//! - [`IdentityMessage`]: 64-byte entity identification report (initiator `'D'`)
//! - [`AckMessage`]: 64-byte tactical acknowledgement report (initiator `'A'`)
//! - [`FakeTdlMessage`]: Sum type over the 3 message kinds
//! - [`FakeTdlFile`]: Sequence of 64-byte FakeTDL messages
//!
//! Benchmarking this native codec alongside the interpreted DFDL engine measures
//! the exact latency/throughput overhead introduced by schema interpretation and
//! generic infoset abstraction.

use std::fmt;

/// Length of every FakeTDL message in bytes.
pub const MESSAGE_LEN: usize = 64;

/// Fill byte character used for padding in FakeTDL binary format (`'X'`).
pub const FILL_BYTE: u8 = b'X';

/// Error type returned when native decoding fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCodecError {
    /// Input buffer is too short or length is not a multiple of 64 bytes.
    UnexpectedEof {
        /// Expected minimum number of bytes.
        expected: usize,
        /// Actual number of bytes provided.
        actual: usize,
    },
    /// Unknown message initiator byte (expected `'T'`, `'D'`, or `'A'`).
    UnknownMessageType {
        /// The byte initiator encountered.
        byte: u8,
    },
    /// Invalid Binary-Coded Decimal (BCD) nibble.
    InvalidBcd {
        /// The invalid BCD byte encountered.
        byte: u8,
    },
    /// Invalid track point type character.
    InvalidPointType {
        /// The byte encountered (expected `'S'`, `'W'`, or `'E'`).
        byte: u8,
    },
    /// Invalid track point quality character.
    InvalidQuality {
        /// The byte encountered (expected `'E'` or `'A'`).
        byte: u8,
    },
    /// Invalid IFF (Identification Friend or Foe) character.
    InvalidIff {
        /// The byte encountered (expected `'U'`, `'F'`, or `'E'`).
        byte: u8,
    },
    /// Invalid category character.
    InvalidCategory {
        /// The byte encountered (expected `'L'`, `'S'`, `'U'`, `'A'`, or `'W'`).
        byte: u8,
    },
    /// Invalid ack message ID count (must be between 1 and 12).
    InvalidAckCount {
        /// The count value encountered.
        count: u16,
    },
}

impl fmt::Display for NativeCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { expected, actual } => {
                write!(
                    f,
                    "unexpected EOF: expected at least {expected} bytes, got {actual}"
                )
            }
            Self::UnknownMessageType { byte } => {
                write!(
                    f,
                    "unknown message type initiator: 0x{byte:02X} ('{}')",
                    *byte as char
                )
            }
            Self::InvalidBcd { byte } => {
                write!(f, "invalid BCD byte: 0x{byte:02X}")
            }
            Self::InvalidPointType { byte } => {
                write!(
                    f,
                    "invalid point type: 0x{byte:02X} ('{}')",
                    *byte as char
                )
            }
            Self::InvalidQuality { byte } => {
                write!(f, "invalid quality: 0x{byte:02X} ('{}')", *byte as char)
            }
            Self::InvalidIff { byte } => {
                write!(f, "invalid IFF: 0x{byte:02X} ('{}')", *byte as char)
            }
            Self::InvalidCategory { byte } => {
                write!(f, "invalid category: 0x{byte:02X} ('{}')", *byte as char)
            }
            Self::InvalidAckCount { count } => {
                write!(f, "invalid ack item count: {count} (must be 1..=12)")
            }
        }
    }
}

impl std::error::Error for NativeCodecError {}

/// Represents a 24-hour time of day (`hhmmss`) stored in 3-byte Binary Coded Decimal (BCD).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TimeOfDay {
    /// Hours (0..=23).
    pub hours: u8,
    /// Minutes (0..=59).
    pub minutes: u8,
    /// Seconds (0..=59).
    pub seconds: u8,
}

impl TimeOfDay {
    /// Creates a new [`TimeOfDay`].
    #[must_use]
    pub const fn new(hours: u8, minutes: u8, seconds: u8) -> Self {
        Self {
            hours,
            minutes,
            seconds,
        }
    }

    /// Decodes a [`TimeOfDay`] from a 3-byte BCD slice.
    ///
    /// # Errors
    /// Returns [`NativeCodecError::InvalidBcd`] if any nibble is greater than 9.
    pub fn from_bcd(raw: &[u8; 3]) -> Result<Self, NativeCodecError> {
        let decode_byte = |b: u8| -> Result<u8, NativeCodecError> {
            let high = b >> 4;
            let low = b & 0x0F;
            if high > 9 || low > 9 {
                return Err(NativeCodecError::InvalidBcd { byte: b });
            }
            Ok(high.wrapping_mul(10).wrapping_add(low))
        };

        let hours = decode_byte(raw[0])?;
        let minutes = decode_byte(raw[1])?;
        let seconds = decode_byte(raw[2])?;
        Ok(Self {
            hours,
            minutes,
            seconds,
        })
    }

    /// Encodes this [`TimeOfDay`] into a 3-byte BCD array.
    #[must_use]
    pub const fn to_bcd(&self) -> [u8; 3] {
        [
            encode_bcd_byte(self.hours),
            encode_bcd_byte(self.minutes),
            encode_bcd_byte(self.seconds),
        ]
    }
}

const fn encode_bcd_byte(v: u8) -> u8 {
    let high = (v / 10) % 10;
    let low = v % 10;
    (high << 4) | low
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02}:{:02}:{:02}",
            self.hours, self.minutes, self.seconds
        )
    }
}

/// Track point type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackPointType {
    /// Start point (`'S'`).
    Start,
    /// Waypoint (`'W'`).
    Waypoint,
    /// End point (`'E'`).
    End,
}

impl TrackPointType {
    /// Converts a wire byte into a [`TrackPointType`].
    ///
    /// # Errors
    /// Returns [`NativeCodecError::InvalidPointType`] if byte is not `'S'`, `'W'`, or `'E'`.
    pub const fn from_byte(b: u8) -> Result<Self, NativeCodecError> {
        match b {
            b'S' => Ok(Self::Start),
            b'W' => Ok(Self::Waypoint),
            b'E' => Ok(Self::End),
            _ => Err(NativeCodecError::InvalidPointType { byte: b }),
        }
    }

    /// Returns the ASCII wire byte for this [`TrackPointType`].
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Start => b'S',
            Self::Waypoint => b'W',
            Self::End => b'E',
        }
    }
}

/// Track point quality enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackPointQuality {
    /// Estimated (`'E'`).
    Estimated,
    /// Actual / observed (`'A'`).
    Actual,
}

impl TrackPointQuality {
    /// Converts a wire byte into a [`TrackPointQuality`].
    ///
    /// # Errors
    /// Returns [`NativeCodecError::InvalidQuality`] if byte is not `'E'` or `'A'`.
    pub const fn from_byte(b: u8) -> Result<Self, NativeCodecError> {
        match b {
            b'E' => Ok(Self::Estimated),
            b'A' => Ok(Self::Actual),
            _ => Err(NativeCodecError::InvalidQuality { byte: b }),
        }
    }

    /// Returns the ASCII wire byte for this [`TrackPointQuality`].
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Estimated => b'E',
            Self::Actual => b'A',
        }
    }
}

/// Identification Friend or Foe (IFF) classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Iff {
    /// Unknown (`'U'`).
    Unknown,
    /// Friend (`'F'`).
    Friend,
    /// Enemy (`'E'`).
    Enemy,
}

impl Iff {
    /// Converts a wire byte into an [`Iff`].
    ///
    /// # Errors
    /// Returns [`NativeCodecError::InvalidIff`] if byte is not `'U'`, `'F'`, or `'E'`.
    pub const fn from_byte(b: u8) -> Result<Self, NativeCodecError> {
        match b {
            b'U' => Ok(Self::Unknown),
            b'F' => Ok(Self::Friend),
            b'E' => Ok(Self::Enemy),
            _ => Err(NativeCodecError::InvalidIff { byte: b }),
        }
    }

    /// Returns the ASCII wire byte for this [`Iff`].
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Unknown => b'U',
            Self::Friend => b'F',
            Self::Enemy => b'E',
        }
    }
}

/// Tactical entity category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Land (`'L'`).
    Land,
    /// Sea surface (`'S'`).
    Sea,
    /// Subsurface (`'U'`).
    Subsurface,
    /// Air (`'A'`).
    Air,
    /// Weapon (`'W'`).
    Weapon,
}

impl Category {
    /// Converts a wire byte into a [`Category`].
    ///
    /// # Errors
    /// Returns [`NativeCodecError::InvalidCategory`] if byte is not `'L'`, `'S'`, `'U'`, `'A'`, or `'W'`.
    pub const fn from_byte(b: u8) -> Result<Self, NativeCodecError> {
        match b {
            b'L' => Ok(Self::Land),
            b'S' => Ok(Self::Sea),
            b'U' => Ok(Self::Subsurface),
            b'A' => Ok(Self::Air),
            b'W' => Ok(Self::Weapon),
            _ => Err(NativeCodecError::InvalidCategory { byte: b }),
        }
    }

    /// Returns the ASCII wire byte for this [`Category`].
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Land => b'L',
            Self::Sea => b'S',
            Self::Subsurface => b'U',
            Self::Air => b'A',
            Self::Weapon => b'W',
        }
    }
}

/// FakeTDL Track Message (initiator `'T'`). Fixed 64 bytes on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackMessage {
    /// 5-character originating unit number.
    pub source: [u8; 5],
    /// Transmission timestamp (BCD `hhmmss`).
    pub send_time: TimeOfDay,
    /// Acknowledgment requested flag.
    pub must_ack: bool,
    /// Originating message sequence identifier.
    pub message_id: u32,
    /// Latitude of message originator in degrees (-90.0 ..= +90.0).
    pub source_lat: f32,
    /// Longitude of message originator in degrees (-180.0 ..= +180.0).
    pub source_lon: f32,
    /// Elevation of message originator (units of 25 feet MSL).
    pub source_elev: i16,
    /// 5-character track identifier.
    pub track_num: [u8; 5],
    /// Track observation timestamp (BCD `hhmmss`).
    pub time: TimeOfDay,
    /// Latitude of track in degrees (-90.0 ..= +90.0).
    pub lat: f32,
    /// Longitude of track in degrees (-180.0 ..= +180.0).
    pub lon: f32,
    /// Elevation of track (units of 25 feet MSL).
    pub elev: i16,
    /// Track point geometry classification.
    pub point_type: TrackPointType,
    /// Track measurement confidence/quality.
    pub quality: TrackPointQuality,
    /// Track heading in integer degrees (0..=359).
    pub course: u16,
    /// Track speed in data-miles per hour (0..=2000).
    pub speed: u16,
}

impl TrackMessage {
    /// Decodes a [`TrackMessage`] from a 64-byte slice.
    ///
    /// # Errors
    /// Returns [`NativeCodecError`] if buffer is malformed or invalid enum/BCD value.
    pub fn decode(bytes: &[u8; MESSAGE_LEN]) -> Result<Self, NativeCodecError> {
        if bytes[0] != b'T' {
            return Err(NativeCodecError::UnknownMessageType { byte: bytes[0] });
        }

        let mut source = [0u8; 5];
        source.copy_from_slice(&bytes[1..6]);

        let send_time = TimeOfDay::from_bcd(&[bytes[6], bytes[7], bytes[8]])?;
        let must_ack = bytes[9] != 0;

        let mut b4 = [0u8; 4];
        let mut b2 = [0u8; 2];

        b4.copy_from_slice(&bytes[10..14]);
        let message_id = u32::from_be_bytes(b4);

        b4.copy_from_slice(&bytes[14..18]);
        let source_lat = f32::from_bits(u32::from_be_bytes(b4));

        b4.copy_from_slice(&bytes[18..22]);
        let source_lon = f32::from_bits(u32::from_be_bytes(b4));

        b2.copy_from_slice(&bytes[22..24]);
        let source_elev = i16::from_be_bytes(b2);

        let mut track_num = [0u8; 5];
        track_num.copy_from_slice(&bytes[24..29]);

        let time = TimeOfDay::from_bcd(&[bytes[29], bytes[30], bytes[31]])?;

        b4.copy_from_slice(&bytes[32..36]);
        let lat = f32::from_bits(u32::from_be_bytes(b4));

        b4.copy_from_slice(&bytes[36..40]);
        let lon = f32::from_bits(u32::from_be_bytes(b4));

        b2.copy_from_slice(&bytes[40..42]);
        let elev = i16::from_be_bytes(b2);

        let point_type = TrackPointType::from_byte(bytes[42])?;
        let quality = TrackPointQuality::from_byte(bytes[43])?;

        b2.copy_from_slice(&bytes[44..46]);
        let course = u16::from_be_bytes(b2);

        b2.copy_from_slice(&bytes[46..48]);
        let speed = u16::from_be_bytes(b2);

        Ok(Self {
            source,
            send_time,
            must_ack,
            message_id,
            source_lat,
            source_lon,
            source_elev,
            track_num,
            time,
            lat,
            lon,
            elev,
            point_type,
            quality,
            course,
            speed,
        })
    }

    /// Serializes this [`TrackMessage`] directly into a 64-byte array.
    #[must_use]
    pub fn encode(&self) -> [u8; MESSAGE_LEN] {
        let mut buf = [FILL_BYTE; MESSAGE_LEN];
        self.encode_into(&mut buf);
        buf
    }

    /// Serializes this [`TrackMessage`] directly into the provided 64-byte destination buffer.
    pub fn encode_into(&self, buf: &mut [u8; MESSAGE_LEN]) {
        buf[0] = b'T';
        buf[1..6].copy_from_slice(&self.source);
        let st_bcd = self.send_time.to_bcd();
        buf[6..9].copy_from_slice(&st_bcd);
        buf[9] = u8::from(self.must_ack);
        buf[10..14].copy_from_slice(&self.message_id.to_be_bytes());
        buf[14..18].copy_from_slice(&self.source_lat.to_bits().to_be_bytes());
        buf[18..22].copy_from_slice(&self.source_lon.to_bits().to_be_bytes());
        buf[22..24].copy_from_slice(&self.source_elev.to_be_bytes());
        buf[24..29].copy_from_slice(&self.track_num);
        let t_bcd = self.time.to_bcd();
        buf[29..32].copy_from_slice(&t_bcd);
        buf[32..36].copy_from_slice(&self.lat.to_bits().to_be_bytes());
        buf[36..40].copy_from_slice(&self.lon.to_bits().to_be_bytes());
        buf[40..42].copy_from_slice(&self.elev.to_be_bytes());
        buf[42] = self.point_type.to_byte();
        buf[43] = self.quality.to_byte();
        buf[44..46].copy_from_slice(&self.course.to_be_bytes());
        buf[46..48].copy_from_slice(&self.speed.to_be_bytes());
        // Remaining 16 bytes (48..64) are filled with FILL_BYTE ('X')
        buf[48..64].fill(FILL_BYTE);
    }
}

/// FakeTDL Identity Message (initiator `'D'`). Fixed 64 bytes on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityMessage {
    /// 5-character originating unit number.
    pub source: [u8; 5],
    /// Transmission timestamp (BCD `hhmmss`).
    pub send_time: TimeOfDay,
    /// Acknowledgment requested flag.
    pub must_ack: bool,
    /// Originating message sequence identifier.
    pub message_id: u32,
    /// 5-character track identifier.
    pub track_num: [u8; 5],
    /// Track observation timestamp (BCD `hhmmss`).
    pub time: TimeOfDay,
    /// Identification Friend or Foe classification.
    pub iff: Iff,
    /// Entity category.
    pub cat: Category,
    /// Entity type detail code.
    pub entity_type: u16,
    /// 38-byte ASCII description, NUL-padded at end.
    pub desc: [u8; 38],
}

impl IdentityMessage {
    /// Decodes an [`IdentityMessage`] from a 64-byte slice.
    ///
    /// # Errors
    /// Returns [`NativeCodecError`] if buffer is malformed or invalid enum/BCD value.
    pub fn decode(bytes: &[u8; MESSAGE_LEN]) -> Result<Self, NativeCodecError> {
        if bytes[0] != b'D' {
            return Err(NativeCodecError::UnknownMessageType { byte: bytes[0] });
        }

        let mut source = [0u8; 5];
        source.copy_from_slice(&bytes[1..6]);

        let send_time = TimeOfDay::from_bcd(&[bytes[6], bytes[7], bytes[8]])?;
        let must_ack = bytes[9] != 0;

        let mut b4 = [0u8; 4];
        let mut b2 = [0u8; 2];

        b4.copy_from_slice(&bytes[10..14]);
        let message_id = u32::from_be_bytes(b4);

        let mut track_num = [0u8; 5];
        track_num.copy_from_slice(&bytes[14..19]);

        let time = TimeOfDay::from_bcd(&[bytes[19], bytes[20], bytes[21]])?;

        let iff = Iff::from_byte(bytes[22])?;
        let cat = Category::from_byte(bytes[23])?;

        b2.copy_from_slice(&bytes[24..26]);
        let entity_type = u16::from_be_bytes(b2);

        let mut desc = [0u8; 38];
        desc.copy_from_slice(&bytes[26..64]);

        Ok(Self {
            source,
            send_time,
            must_ack,
            message_id,
            track_num,
            time,
            iff,
            cat,
            entity_type,
            desc,
        })
    }

    /// Serializes this [`IdentityMessage`] directly into a 64-byte array.
    #[must_use]
    pub fn encode(&self) -> [u8; MESSAGE_LEN] {
        let mut buf = [0u8; MESSAGE_LEN];
        self.encode_into(&mut buf);
        buf
    }

    /// Serializes this [`IdentityMessage`] directly into the provided 64-byte destination buffer.
    pub fn encode_into(&self, buf: &mut [u8; MESSAGE_LEN]) {
        buf[0] = b'D';
        buf[1..6].copy_from_slice(&self.source);
        let st_bcd = self.send_time.to_bcd();
        buf[6..9].copy_from_slice(&st_bcd);
        buf[9] = u8::from(self.must_ack);
        buf[10..14].copy_from_slice(&self.message_id.to_be_bytes());
        buf[14..19].copy_from_slice(&self.track_num);
        let t_bcd = self.time.to_bcd();
        buf[19..22].copy_from_slice(&t_bcd);
        buf[22] = self.iff.to_byte();
        buf[23] = self.cat.to_byte();
        buf[24..26].copy_from_slice(&self.entity_type.to_be_bytes());
        buf[26..64].copy_from_slice(&self.desc);
    }
}

/// FakeTDL Acknowledgment Message (initiator `'A'`). Fixed 64 bytes on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckMessage {
    /// 5-character originating unit number.
    pub source: [u8; 5],
    /// Transmission timestamp (BCD `hhmmss`).
    pub send_time: TimeOfDay,
    /// 5-character destination unit number.
    pub dest: [u8; 5],
    /// List of acknowledged message sequence identifiers (1..=12 items).
    pub items: Vec<u32>,
}

impl AckMessage {
    /// Decodes an [`AckMessage`] from a 64-byte slice.
    ///
    /// # Errors
    /// Returns [`NativeCodecError`] if buffer is malformed or item count is outside 1..=12.
    pub fn decode(bytes: &[u8; MESSAGE_LEN]) -> Result<Self, NativeCodecError> {
        if bytes[0] != b'A' {
            return Err(NativeCodecError::UnknownMessageType { byte: bytes[0] });
        }

        let mut source = [0u8; 5];
        source.copy_from_slice(&bytes[1..6]);

        let send_time = TimeOfDay::from_bcd(&[bytes[6], bytes[7], bytes[8]])?;

        let mut dest = [0u8; 5];
        dest.copy_from_slice(&bytes[9..14]);

        let mut b2 = [0u8; 2];
        b2.copy_from_slice(&bytes[14..16]);
        let count = u16::from_be_bytes(b2);

        if !(1..=12).contains(&count) {
            return Err(NativeCodecError::InvalidAckCount { count });
        }

        let mut items = Vec::with_capacity(count as usize);
        let mut offset = 16usize;
        let mut b4 = [0u8; 4];
        for _ in 0..count {
            let end = offset.saturating_add(4);
            let item_slice = bytes.get(offset..end).ok_or(NativeCodecError::UnexpectedEof {
                expected: end,
                actual: bytes.len(),
            })?;
            b4.copy_from_slice(item_slice);
            items.push(u32::from_be_bytes(b4));
            offset = end;
        }

        Ok(Self {
            source,
            send_time,
            dest,
            items,
        })
    }

    /// Serializes this [`AckMessage`] directly into a 64-byte array.
    #[must_use]
    pub fn encode(&self) -> [u8; MESSAGE_LEN] {
        let mut buf = [FILL_BYTE; MESSAGE_LEN];
        self.encode_into(&mut buf);
        buf
    }

    /// Serializes this [`AckMessage`] directly into the provided 64-byte destination buffer.
    pub fn encode_into(&self, buf: &mut [u8; MESSAGE_LEN]) {
        buf[0] = b'A';
        buf[1..6].copy_from_slice(&self.source);
        let st_bcd = self.send_time.to_bcd();
        buf[6..9].copy_from_slice(&st_bcd);
        buf[9..14].copy_from_slice(&self.dest);

        let count = self.items.len().min(12) as u16;
        buf[14..16].copy_from_slice(&count.to_be_bytes());

        let mut offset = 16usize;
        for &item in self.items.iter().take(12) {
            let end = offset.saturating_add(4);
            if let Some(slot) = buf.get_mut(offset..end) {
                slot.copy_from_slice(&item.to_be_bytes());
            }
            offset = end;
        }

        // Remaining bytes up to 64 are filled with FILL_BYTE ('X')
        if let Some(padding) = buf.get_mut(offset..64) {
            padding.fill(FILL_BYTE);
        }
    }
}

/// Sum type over the 3 FakeTDL message kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum FakeTdlMessage {
    /// Tactical track message (`'T'`).
    Track(TrackMessage),
    /// Tactical entity identification message (`'D'`).
    Identity(IdentityMessage),
    /// Tactical acknowledgement message (`'A'`).
    Ack(AckMessage),
}

impl FakeTdlMessage {
    /// Decodes a single 64-byte FakeTDL message from a byte slice.
    ///
    /// # Errors
    /// Returns [`NativeCodecError`] if the slice is shorter than 64 bytes or invalid content.
    pub fn decode(bytes: &[u8]) -> Result<Self, NativeCodecError> {
        if bytes.len() < MESSAGE_LEN {
            return Err(NativeCodecError::UnexpectedEof {
                expected: MESSAGE_LEN,
                actual: bytes.len(),
            });
        }

        let chunk_slice = bytes.get(..MESSAGE_LEN).ok_or(NativeCodecError::UnexpectedEof {
            expected: MESSAGE_LEN,
            actual: bytes.len(),
        })?;

        let mut chunk = [0u8; MESSAGE_LEN];
        chunk.copy_from_slice(chunk_slice);

        match chunk[0] {
            b'T' => Ok(Self::Track(TrackMessage::decode(&chunk)?)),
            b'D' => Ok(Self::Identity(IdentityMessage::decode(&chunk)?)),
            b'A' => Ok(Self::Ack(AckMessage::decode(&chunk)?)),
            other => Err(NativeCodecError::UnknownMessageType { byte: other }),
        }
    }

    /// Serializes this message into a 64-byte array.
    #[must_use]
    pub fn encode(&self) -> [u8; MESSAGE_LEN] {
        match self {
            Self::Track(msg) => msg.encode(),
            Self::Identity(msg) => msg.encode(),
            Self::Ack(msg) => msg.encode(),
        }
    }

    /// Serializes this message into the provided 64-byte destination buffer.
    pub fn encode_into(&self, buf: &mut [u8; MESSAGE_LEN]) {
        match self {
            Self::Track(msg) => msg.encode_into(buf),
            Self::Identity(msg) => msg.encode_into(buf),
            Self::Ack(msg) => msg.encode_into(buf),
        }
    }
}

/// Represents a FakeTDL multi-message file (`fakeTDLFile`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FakeTdlFile {
    /// Collection of FakeTDL messages contained in the file.
    pub messages: Vec<FakeTdlMessage>,
}

impl FakeTdlFile {
    /// Creates a new empty [`FakeTdlFile`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    /// Decodes all FakeTDL messages from a byte slice.
    ///
    /// # Errors
    /// Returns [`NativeCodecError`] if any message is invalid or input is not a multiple of 64 bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, NativeCodecError> {
        if bytes.is_empty() {
            return Ok(Self::new());
        }

        let (chunks, remainder) = bytes.as_chunks::<MESSAGE_LEN>();
        if !remainder.is_empty() {
            return Err(NativeCodecError::UnexpectedEof {
                expected: MESSAGE_LEN,
                actual: remainder.len(),
            });
        }

        let mut messages = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let msg = match chunk[0] {
                b'T' => FakeTdlMessage::Track(TrackMessage::decode(chunk)?),
                b'D' => FakeTdlMessage::Identity(IdentityMessage::decode(chunk)?),
                b'A' => FakeTdlMessage::Ack(AckMessage::decode(chunk)?),
                other => return Err(NativeCodecError::UnknownMessageType { byte: other }),
            };
            messages.push(msg);
        }

        Ok(Self { messages })
    }

    /// Serializes all messages into a binary byte vector.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let capacity = self.messages.len().saturating_mul(MESSAGE_LEN);
        let mut out = Vec::with_capacity(capacity);
        for msg in &self.messages {
            out.extend_from_slice(&msg.encode());
        }
        out
    }
}
