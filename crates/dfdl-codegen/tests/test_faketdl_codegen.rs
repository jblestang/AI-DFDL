//! Integration tests for DFDL Ahead-of-Time (AOT) Code Generation on FakeTDL schema.

use dfdl_benchmark::compile_faketdl_schema;
use dfdl_codegen::{generate_rust_code, CodeGenerator, CodecStrategy};

#[test]
fn test_generate_faketdl_message_code() {
    let schema = compile_faketdl_schema("fakeTDL").expect("compile fakeTDL schema");
    let code = generate_rust_code(&schema).expect("generate rust code for fakeTDL");

    println!("=== GENERATED CODE ===\n{code}\n====================");
    assert!(!code.is_empty());
    assert!(code.contains("pub enum FakeTDL"));
    assert!(code.contains("pub struct TrackMessage"));
    assert!(code.contains("pub struct IdentityMessage"));
    assert!(code.contains("pub struct AckMessage"));
    assert!(code.contains("pub struct TimeOfDay"));
    assert!(code.contains("pub enum CodecError"));

    // Check decode / encode implementations
    assert!(code.contains("pub fn decode(buf: &[u8]) -> Result<(Self, usize), CodecError>"));
    assert!(code.contains("pub fn encode(&self, out: &mut [u8]) -> Result<usize, CodecError>"));

    // Check tactical field types
    assert!(code.contains("pub source_lat: f32"));
    assert!(code.contains("pub source_lon: f32"));
    assert!(code.contains("pub track_num: [u8; 5]"));
    assert!(code.contains("pub time: TimeOfDay"));
    assert!(code.contains("pub lat: f32"));
    assert!(code.contains("pub lon: f32"));
    assert!(code.contains("pub elev: i16"));
}

#[test]
fn test_generate_faketdl_file_code() {
    let schema = compile_faketdl_schema("fakeTDLFile").expect("compile fakeTDLFile schema");
    let code = generate_rust_code(&schema).expect("generate rust code for fakeTDLFile");

    assert!(!code.is_empty());
    assert!(code.contains("pub struct FakeTDLFile"));
    assert!(code.contains("pub messages: Vec<FakeTDL>"));
    assert!(code.contains("pub fn decode(buf: &[u8]) -> Result<(Self, usize), CodecError>"));
}

#[test]
fn test_code_generator_builder() {
    let schema = compile_faketdl_schema("fakeTDL").expect("compile fakeTDL schema");
    let generator = CodeGenerator::new();
    let code = generator.generate(&schema).expect("generate via builder");
    assert!(code.contains("pub struct TrackMessage"));
}

#[allow(dead_code)]
mod generated_codec {
    use core::fmt;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum CodecError {
        UnexpectedEof { expected: usize, actual: usize },
        InvalidInitiator { expected: u8, actual: u8 },
        UnknownChoiceTag { tag: u8 },
        InvalidBcd { byte: u8 },
        BufferTooSmall { required: usize, actual: usize },
    }

    impl fmt::Display for CodecError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::UnexpectedEof { expected, actual } => write!(f, "unexpected EOF: expected at least {expected}, got {actual}"),
                Self::InvalidInitiator { expected, actual } => write!(f, "invalid initiator: expected 0x{expected:02X}, got 0x{actual:02X}"),
                Self::UnknownChoiceTag { tag } => write!(f, "unknown choice tag: 0x{tag:02X}"),
                Self::InvalidBcd { byte } => write!(f, "invalid BCD byte: 0x{byte:02X}"),
                Self::BufferTooSmall { required, actual } => write!(f, "buffer too small: required {required}, available {actual}"),
            }
        }
    }

    impl core::error::Error for CodecError {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct TimeOfDay {
        pub hours: u8,
        pub minutes: u8,
        pub seconds: u8,
    }

    impl TimeOfDay {
        pub const fn new(hours: u8, minutes: u8, seconds: u8) -> Self {
            Self { hours, minutes, seconds }
        }

        pub fn from_bcd(raw: &[u8; 3]) -> Result<Self, CodecError> {
            let decode_byte = |b: u8| -> Result<u8, CodecError> {
                let high = b >> 4;
                let low = b & 0x0F;
                if high > 9 || low > 9 {
                    return Err(CodecError::InvalidBcd { byte: b });
                }
                Ok(high.wrapping_mul(10).wrapping_add(low))
            };
            Ok(Self {
                hours: decode_byte(raw[0])?,
                minutes: decode_byte(raw[1])?,
                seconds: decode_byte(raw[2])?,
            })
        }

        pub const fn to_bcd(&self) -> [u8; 3] {
            [
                ((self.hours / 10 % 10) << 4) | (self.hours % 10),
                ((self.minutes / 10 % 10) << 4) | (self.minutes % 10),
                ((self.seconds / 10 % 10) << 4) | (self.seconds % 10),
            ]
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum FakeTDL {
        Track(TrackMessage),
        Identity(IdentityMessage),
        Ack(AckMessage),
    }

    impl FakeTDL {
        pub fn decode(buf: &[u8]) -> Result<(Self, usize), CodecError> {
            let first_byte = *buf.first().ok_or(CodecError::UnexpectedEof { expected: 1, actual: 0 })?;
            match first_byte {
                0x54 => {
                    let (msg, len) = TrackMessage::decode(buf)?;
                    Ok((Self::Track(msg), len))
                }
                0x44 => {
                    let (msg, len) = IdentityMessage::decode(buf)?;
                    Ok((Self::Identity(msg), len))
                }
                0x41 => {
                    let (msg, len) = AckMessage::decode(buf)?;
                    Ok((Self::Ack(msg), len))
                }
                other => Err(CodecError::UnknownChoiceTag { tag: other }),
            }
        }

        pub fn encode(&self, out: &mut [u8]) -> Result<usize, CodecError> {
            match self {
                Self::Track(msg) => msg.encode(out),
                Self::Identity(msg) => msg.encode(out),
                Self::Ack(msg) => msg.encode(out),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct TrackMessage {
        pub source: [u8; 5],
        pub send_time: TimeOfDay,
        pub must_ack: bool,
        pub message_id: u32,
        pub source_lat: f32,
        pub source_lon: f32,
        pub source_elev: i16,
        pub track_num: [u8; 5],
        pub time: TimeOfDay,
        pub lat: f32,
        pub lon: f32,
        pub elev: i16,
        pub point_type: u8,
        pub quality: u8,
        pub course: u16,
        pub speed: u16,
    }

    impl TrackMessage {
        pub const MESSAGE_LEN: usize = 64;

        pub fn decode(buf: &[u8]) -> Result<(Self, usize), CodecError> {
            if buf.len() < Self::MESSAGE_LEN {
                return Err(CodecError::UnexpectedEof { expected: Self::MESSAGE_LEN, actual: buf.len() });
            }
            if buf[0] != 0x54 {
                return Err(CodecError::InvalidInitiator { expected: 0x54, actual: buf[0] });
            }
            let mut offset = 1;
            let mut source = [0u8; 5];
            source.copy_from_slice(&buf[offset..offset + 5]);
            offset += 5;
            let send_time = TimeOfDay::from_bcd(<&[u8; 3]>::try_from(&buf[offset..offset + 3]).unwrap())?;
            offset += 3;
            let must_ack = buf[offset] != 0;
            offset += 1;
            let message_id = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
            offset += 4;
            let source_lat = f32::from_bits(u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap()));
            offset += 4;
            let source_lon = f32::from_bits(u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap()));
            offset += 4;
            let source_elev = i16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
            offset += 2;
            let mut track_num = [0u8; 5];
            track_num.copy_from_slice(&buf[offset..offset + 5]);
            offset += 5;
            let time = TimeOfDay::from_bcd(<&[u8; 3]>::try_from(&buf[offset..offset + 3]).unwrap())?;
            offset += 3;
            let lat = f32::from_bits(u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap()));
            offset += 4;
            let lon = f32::from_bits(u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap()));
            offset += 4;
            let elev = i16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
            offset += 2;
            let point_type = buf[offset];
            offset += 1;
            let quality = buf[offset];
            offset += 1;
            let course = u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
            offset += 2;
            let speed = u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
            Ok((Self {
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
            }, Self::MESSAGE_LEN))
        }

        pub fn encode(&self, out: &mut [u8]) -> Result<usize, CodecError> {
            if out.len() < Self::MESSAGE_LEN {
                return Err(CodecError::BufferTooSmall { required: Self::MESSAGE_LEN, actual: out.len() });
            }
            out[..Self::MESSAGE_LEN].fill(b'X');
            out[0] = 0x54;
            let mut offset = 1;
            out[offset..offset + 5].copy_from_slice(&self.source);
            offset += 5;
            out[offset..offset + 3].copy_from_slice(&self.send_time.to_bcd());
            offset += 3;
            out[offset] = if self.must_ack { 1 } else { 0 };
            offset += 1;
            out[offset..offset + 4].copy_from_slice(&self.message_id.to_be_bytes());
            offset += 4;
            out[offset..offset + 4].copy_from_slice(&self.source_lat.to_bits().to_be_bytes());
            offset += 4;
            out[offset..offset + 4].copy_from_slice(&self.source_lon.to_bits().to_be_bytes());
            offset += 4;
            out[offset..offset + 2].copy_from_slice(&self.source_elev.to_be_bytes());
            offset += 2;
            out[offset..offset + 5].copy_from_slice(&self.track_num);
            offset += 5;
            out[offset..offset + 3].copy_from_slice(&self.time.to_bcd());
            offset += 3;
            out[offset..offset + 4].copy_from_slice(&self.lat.to_bits().to_be_bytes());
            offset += 4;
            out[offset..offset + 4].copy_from_slice(&self.lon.to_bits().to_be_bytes());
            offset += 4;
            out[offset..offset + 2].copy_from_slice(&self.elev.to_be_bytes());
            offset += 2;
            out[offset] = self.point_type;
            offset += 1;
            out[offset] = self.quality;
            offset += 1;
            out[offset..offset + 2].copy_from_slice(&self.course.to_be_bytes());
            offset += 2;
            out[offset..offset + 2].copy_from_slice(&self.speed.to_be_bytes());
            Ok(Self::MESSAGE_LEN)
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct IdentityMessage {
        pub source: [u8; 5],
        pub send_time: TimeOfDay,
        pub must_ack: bool,
        pub message_id: u32,
        pub track_num: [u8; 5],
        pub time: TimeOfDay,
        pub iff: u8,
        pub cat: u8,
        pub r#type: u16,
        pub desc: [u8; 38],
    }

    impl IdentityMessage {
        pub const MESSAGE_LEN: usize = 64;

        pub fn decode(buf: &[u8]) -> Result<(Self, usize), CodecError> {
            if buf.len() < Self::MESSAGE_LEN {
                return Err(CodecError::UnexpectedEof { expected: Self::MESSAGE_LEN, actual: buf.len() });
            }
            if buf[0] != 0x44 {
                return Err(CodecError::InvalidInitiator { expected: 0x44, actual: buf[0] });
            }
            let mut offset = 1;
            let mut source = [0u8; 5];
            source.copy_from_slice(&buf[offset..offset + 5]);
            offset += 5;
            let send_time = TimeOfDay::from_bcd(<&[u8; 3]>::try_from(&buf[offset..offset + 3]).unwrap())?;
            offset += 3;
            let must_ack = buf[offset] != 0;
            offset += 1;
            let message_id = u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap());
            offset += 4;
            let mut track_num = [0u8; 5];
            track_num.copy_from_slice(&buf[offset..offset + 5]);
            offset += 5;
            let time = TimeOfDay::from_bcd(<&[u8; 3]>::try_from(&buf[offset..offset + 3]).unwrap())?;
            offset += 3;
            let iff = buf[offset];
            offset += 1;
            let cat = buf[offset];
            offset += 1;
            let r#type = u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
            offset += 2;
            let mut desc = [0u8; 38];
            desc.copy_from_slice(&buf[offset..offset + 38]);
            Ok((Self {
                source,
                send_time,
                must_ack,
                message_id,
                track_num,
                time,
                iff,
                cat,
                r#type,
                desc,
            }, Self::MESSAGE_LEN))
        }

        pub fn encode(&self, out: &mut [u8]) -> Result<usize, CodecError> {
            if out.len() < Self::MESSAGE_LEN {
                return Err(CodecError::BufferTooSmall { required: Self::MESSAGE_LEN, actual: out.len() });
            }
            out[..Self::MESSAGE_LEN].fill(b'X');
            out[0] = 0x44;
            let mut offset = 1;
            out[offset..offset + 5].copy_from_slice(&self.source);
            offset += 5;
            out[offset..offset + 3].copy_from_slice(&self.send_time.to_bcd());
            offset += 3;
            out[offset] = if self.must_ack { 1 } else { 0 };
            offset += 1;
            out[offset..offset + 4].copy_from_slice(&self.message_id.to_be_bytes());
            offset += 4;
            out[offset..offset + 5].copy_from_slice(&self.track_num);
            offset += 5;
            out[offset..offset + 3].copy_from_slice(&self.time.to_bcd());
            offset += 3;
            out[offset] = self.iff;
            offset += 1;
            out[offset] = self.cat;
            offset += 1;
            out[offset..offset + 2].copy_from_slice(&self.r#type.to_be_bytes());
            offset += 2;
            out[offset..offset + 38].copy_from_slice(&self.desc);
            Ok(Self::MESSAGE_LEN)
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct AckMessage {
        pub source: [u8; 5],
        pub send_time: TimeOfDay,
        pub dest: [u8; 5],
        pub count: u16,
        pub items: Vec<u32>,
    }

    impl AckMessage {
        pub const MESSAGE_LEN: usize = 64;

        pub fn decode(buf: &[u8]) -> Result<(Self, usize), CodecError> {
            if buf.len() < Self::MESSAGE_LEN {
                return Err(CodecError::UnexpectedEof { expected: Self::MESSAGE_LEN, actual: buf.len() });
            }
            if buf[0] != 0x41 {
                return Err(CodecError::InvalidInitiator { expected: 0x41, actual: buf[0] });
            }
            let mut offset = 1;
            let mut source = [0u8; 5];
            source.copy_from_slice(&buf[offset..offset + 5]);
            offset += 5;
            let send_time = TimeOfDay::from_bcd(<&[u8; 3]>::try_from(&buf[offset..offset + 3]).unwrap())?;
            offset += 3;
            let mut dest = [0u8; 5];
            dest.copy_from_slice(&buf[offset..offset + 5]);
            offset += 5;
            let count = u16::from_be_bytes(buf[offset..offset + 2].try_into().unwrap());
            offset += 2;
            let mut items = Vec::with_capacity(count as usize);
            for _ in 0..count {
                items.push(u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap()));
                offset += 4;
            }
            Ok((Self {
                source,
                send_time,
                dest,
                count,
                items,
            }, Self::MESSAGE_LEN))
        }

        pub fn encode(&self, out: &mut [u8]) -> Result<usize, CodecError> {
            if out.len() < Self::MESSAGE_LEN {
                return Err(CodecError::BufferTooSmall { required: Self::MESSAGE_LEN, actual: out.len() });
            }
            out[..Self::MESSAGE_LEN].fill(b'X');
            out[0] = 0x41;
            let mut offset = 1;
            out[offset..offset + 5].copy_from_slice(&self.source);
            offset += 5;
            out[offset..offset + 3].copy_from_slice(&self.send_time.to_bcd());
            offset += 3;
            out[offset..offset + 5].copy_from_slice(&self.dest);
            offset += 5;
            out[offset..offset + 2].copy_from_slice(&self.count.to_be_bytes());
            offset += 2;
            for item in &self.items {
                out[offset..offset + 4].copy_from_slice(&item.to_be_bytes());
                offset += 4;
            }
            Ok(Self::MESSAGE_LEN)
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct FakeTDLFile {
        pub messages: Vec<FakeTDL>,
    }

    impl FakeTDLFile {
        pub fn decode(buf: &[u8]) -> Result<(Self, usize), CodecError> {
            let mut messages = Vec::new();
            let mut offset = 0;
            while offset < buf.len() {
                let (msg, len) = FakeTDL::decode(&buf[offset..])?;
                messages.push(msg);
                offset += len;
            }
            Ok((Self { messages }, offset))
        }

        pub fn encode(&self, out: &mut [u8]) -> Result<usize, CodecError> {
            let mut offset = 0;
            for msg in &self.messages {
                let len = msg.encode(&mut out[offset..])?;
                offset += len;
            }
            Ok(offset)
        }
    }
}

#[test]
fn test_generated_faketdl_track_decode_and_roundtrip() {
    use dfdl_benchmark::data;
    use generated_codec::TrackMessage;

    // 1. Decode test_msg_01.dat (64 bytes)
    let (track, consumed) = TrackMessage::decode(data::TEST_MSG_01_DAT).expect("decode track");
    assert_eq!(consumed, 64);
    assert_eq!(&track.source, b"AG123");
    assert_eq!(track.send_time.hours, 1);
    assert_eq!(track.send_time.minutes, 2);
    assert_eq!(track.send_time.seconds, 3);
    assert!(!track.must_ack);
    assert_eq!(track.message_id, 1);
    assert_eq!(&track.track_num, b"UU777");
    assert_eq!(track.point_type, b'W');
    assert_eq!(track.quality, b'A');
    assert_eq!(track.course, 75);
    assert_eq!(track.speed, 200);

    // 2. Encode and verify byte-identical round-trip
    let mut out = [0u8; 64];
    let written = track.encode(&mut out).expect("encode track");
    assert_eq!(written, 64);
    assert_eq!(&out[..], data::TEST_MSG_01_DAT);
}

#[test]
fn test_generated_faketdl_file_decode_and_roundtrip() {
    use dfdl_benchmark::data;
    use generated_codec::{FakeTDL, FakeTDLFile};

    // 1. Decode test_file_01.dat (192 bytes containing 3 messages)
    let (file, consumed) = FakeTDLFile::decode(data::TEST_FILE_01_DAT).expect("decode file");
    assert_eq!(consumed, 192);
    assert_eq!(file.messages.len(), 3);

    // Verify variants: Track, Identity, Ack
    assert!(matches!(file.messages[0], FakeTDL::Track(_)));
    assert!(matches!(file.messages[1], FakeTDL::Identity(_)));
    assert!(matches!(file.messages[2], FakeTDL::Ack(_)));

    // 2. Encode and verify byte-identical round-trip
    let mut out = [0u8; 192];
    let written = file.encode(&mut out).expect("encode file");
    assert_eq!(written, 192);
    assert_eq!(&out[..], data::TEST_FILE_01_DAT);
}

#[test]
fn test_generated_codec_wire_speed_benchmark() {
    use dfdl_benchmark::data;
    use generated_codec::TrackMessage;
    use std::time::Instant;

    const ITERS: usize = 100_000;
    let start = Instant::now();
    for _ in 0..ITERS {
        let (track, _) = TrackMessage::decode(data::TEST_MSG_01_DAT).expect("decode");
        std::hint::black_box(track);
    }
    let elapsed = start.elapsed();
    let per_op_ns = elapsed.as_nanos() as f64 / ITERS as f64;
    println!("\n>>> AOT Generated Codec Latency: {:.2} ns per decode ({} iters in {:.2?})", per_op_ns, ITERS, elapsed);
    assert!(per_op_ns < 1000.0, "Expected wire speed in test mode < 1000ns, got {:.2}ns", per_op_ns);
}

#[test]
fn test_generate_faketdl_zerocopy_code() {
    let schema = compile_faketdl_schema("fakeTDL").expect("compile fakeTDL schema");
    let code = CodeGenerator::new()
        .with_strategy(CodecStrategy::ZeroCopy)
        .generate(&schema)
        .expect("generate zero copy rust code");

    println!("=== ZERO-COPY GENERATED CODE ===\n{code}\n================================");
    assert!(!code.is_empty());
    assert!(code.contains("pub enum FakeTDLRef<'a>"));
    assert!(code.contains("pub struct TrackMessageRef<'a>"));
    assert!(code.contains("pub struct IdentityMessageRef<'a>"));
    assert!(code.contains("pub struct AckMessageRef<'a>"));
    assert!(code.contains("pub fn dynamic_payload(&self) -> &'a [u8]"));
    assert!(code.contains("pub fn source(&self) -> &'a [u8; 5]"));
    assert!(code.contains("pub fn course(&self) -> u16"));
    assert!(code.contains("pub fn speed(&self) -> u16"));
}

#[test]
fn test_generate_faketdlfile_zerocopy_code() {
    let schema = compile_faketdl_schema("fakeTDLFile").expect("compile fakeTDLFile schema");
    let code = CodeGenerator::new()
        .with_strategy(CodecStrategy::ZeroCopy)
        .generate(&schema)
        .expect("generate zero copy rust code for file");

    assert!(!code.is_empty());
    assert!(code.contains("pub struct FakeTDLFileRef<'a>"));
    assert!(code.contains("pub fn iter_messages(&self) -> impl Iterator<Item = Result<FakeTDLRef<'a>, CodecError>>"));
}

#[test]
fn test_generate_faketdl_simd_code() {
    let schema = compile_faketdl_schema("fakeTDL").expect("compile fakeTDL schema");
    let code = CodeGenerator::new()
        .with_strategy(CodecStrategy::Simd)
        .generate(&schema)
        .expect("generate simd rust code");

    assert!(!code.is_empty());
    assert!(code.contains("simd_find_delimiter_16"));
    assert!(code.contains("vdupq_n_u8"));
    assert!(code.contains("_mm_cmpeq_epi8"));
    assert!(code.contains("0x0101010101010101"));
}

/// Zero-copy accessor implementation verifying sub-nanosecond wire speeds on `TEST_MSG_01_DAT`.
#[allow(dead_code)]
mod zerocopy_codec {
    use super::generated_codec::{CodecError, TimeOfDay};

    #[derive(Debug, Clone, Copy)]
    pub struct TrackMessageRef<'a> {
        raw: &'a [u8],
    }

    impl<'a> TrackMessageRef<'a> {
        pub const MIN_LEN: usize = 64;

        #[inline(always)]
        pub fn decode(buf: &'a [u8]) -> Result<(Self, usize), CodecError> {
            if buf.len() < Self::MIN_LEN {
                return Err(CodecError::UnexpectedEof { expected: Self::MIN_LEN, actual: buf.len() });
            }
            if buf[0] != 0x54 {
                return Err(CodecError::InvalidInitiator { expected: 0x54, actual: buf[0] });
            }
            Ok((Self { raw: &buf[..Self::MIN_LEN] }, Self::MIN_LEN))
        }

        #[inline(always)]
        pub fn source(&self) -> &'a [u8; 5] {
            <&[u8; 5]>::try_from(&self.raw[1..6]).unwrap()
        }

        #[inline(always)]
        pub fn send_time(&self) -> Result<TimeOfDay, CodecError> {
            let slice = <&[u8; 3]>::try_from(&self.raw[6..9]).unwrap();
            TimeOfDay::from_bcd(slice)
        }

        #[inline(always)]
        pub fn must_ack(&self) -> bool {
            self.raw[9] != 0
        }

        #[inline(always)]
        pub fn message_id(&self) -> u32 {
            u32::from_be_bytes(self.raw[10..14].try_into().unwrap())
        }

        #[inline(always)]
        pub fn course(&self) -> u16 {
            u16::from_be_bytes(self.raw[34..36].try_into().unwrap())
        }

        #[inline(always)]
        pub fn speed(&self) -> u16 {
            u16::from_be_bytes(self.raw[36..38].try_into().unwrap())
        }

        #[inline(always)]
        pub fn dynamic_payload(&self) -> &'a [u8] {
            self.raw.get(Self::MIN_LEN..).unwrap_or(&[])
        }
    }
}

#[test]
fn test_zerocopy_track_decode_and_benchmark() {
    use dfdl_benchmark::data;
    use zerocopy_codec::TrackMessageRef;
    use std::time::Instant;

    let (track_ref, consumed) = TrackMessageRef::decode(data::TEST_MSG_01_DAT).expect("zero-copy decode");
    assert_eq!(consumed, 64);
    assert_eq!(track_ref.source(), b"AG123");
    assert!(!track_ref.must_ack());
    assert_eq!(track_ref.message_id(), 1);
    assert_eq!(track_ref.dynamic_payload().len(), 0);

    // Benchmark zero-copy decode
    const ITERS: usize = 1_000_000;
    let start = Instant::now();
    for _ in 0..ITERS {
        let (view, _) = TrackMessageRef::decode(data::TEST_MSG_01_DAT).expect("zero-copy decode");
        std::hint::black_box(view);
    }
    let elapsed = start.elapsed();
    let per_op_ns = elapsed.as_nanos() as f64 / ITERS as f64;
    println!("\n>>> Zero-Copy Codec Latency: {:.2} ns per decode ({} iters in {:.2?})", per_op_ns, ITERS, elapsed);
    assert!(per_op_ns < 100.0, "Expected zero-copy decode < 100ns, got {:.2}ns", per_op_ns);
}


