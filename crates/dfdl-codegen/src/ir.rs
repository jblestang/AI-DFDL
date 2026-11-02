//! Intermediate Representation (IR) for DFDL Ahead-of-Time Code Generation.
//!
//! Provides clean, normalized AST representations for generated Rust structs,
//! enums, fields, and codecs extracted from [`dfdl_core::schema::ir::CompiledSchema`].

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

/// Scalar or compound Rust type generated for a schema element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustType {
    /// Standard 8-bit unsigned integer.
    U8,
    /// Standard 8-bit signed integer.
    I8,
    /// Standard 16-bit unsigned integer.
    U16,
    /// Standard 16-bit signed integer.
    I16,
    /// Standard 32-bit unsigned integer.
    U32,
    /// Standard 32-bit signed integer.
    I32,
    /// Standard 64-bit unsigned integer.
    U64,
    /// Standard 64-bit signed integer.
    I64,
    /// Standard 32-bit IEEE 754 floating point.
    F32,
    /// Standard 64-bit IEEE 754 floating point.
    F64,
    /// Standard boolean flag.
    Bool,
    /// Fixed-size raw byte array `[u8; N]`.
    FixedByteArray(usize),
    /// Fixed-size ASCII/UTF-8 string.
    FixedString(usize),
    /// 3-byte Binary Coded Decimal (BCD) `hhmmss` time of day.
    TimeOfDay,
    /// Named enumeration type.
    Enum(String),
    /// Named nested struct type.
    Struct(String),
    /// Vector of items `alloc::vec::Vec<T>`.
    Vec(alloc::boxed::Box<RustType>),
}

/// Encoding and decoding logic strategy for a struct field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldCodec {
    /// Binary integer with byte size, signedness, and endianness.
    BinaryInt {
        /// Number of bytes on wire (1, 2, 4, 8).
        size: usize,
        /// True if signed two's complement.
        signed: bool,
        /// True if big-endian byte order.
        is_be: bool,
    },
    /// Binary floating-point number.
    BinaryFloat {
        /// Number of bytes on wire (4 for f32, 8 for f64).
        size: usize,
        /// True if big-endian byte order.
        is_be: bool,
    },
    /// Binary boolean flag (0 = false, non-zero = true).
    Bool {
        /// Number of bytes on wire (usually 1).
        size: usize,
    },
    /// Fixed-length byte slice copied directly.
    FixedByteArray {
        /// Number of bytes on wire.
        size: usize,
    },
    /// Fixed-length ASCII string trimmed or padded.
    FixedString {
        /// Number of bytes on wire.
        size: usize,
    },
    /// 3-byte BCD timestamp (`hhmmss`).
    BcdTimeOfDay,
    /// Enum field validated and mapped from a single wire byte.
    Enum {
        /// Target enum type name.
        enum_name: String,
    },
    /// Nested child struct decoded in-place.
    NestedStruct {
        /// Target struct type name.
        type_name: String,
    },
    /// Dynamic or fixed array of child items.
    Array {
        /// Inner item type.
        item_type: alloc::boxed::Box<RustType>,
        /// Array decoding mode.
        mode: ArrayMode,
    },
}

/// Array occurrence decoding strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArrayMode {
    /// Dynamic count read from a preceding sibling integer field.
    CountField {
        /// Name of the sibling field containing the item count.
        count_field: String,
        /// Maximum allowed items.
        max_items: usize,
    },
    /// Unbounded array consuming messages until end-of-input.
    UntilEof,
}

/// Definition of an individual field in a generated Rust struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    /// Rust field identifier (snake_case).
    pub rust_name: String,
    /// Original DFDL element name from schema.
    pub dfdl_name: String,
    /// Rust data type of the field.
    pub rust_type: RustType,
    /// Binary codec strategy for parsing and unparsing.
    pub codec: FieldCodec,
    /// Documentation comment for the field.
    pub doc: String,
}

/// Definition of a generated Rust struct corresponding to a DFDL sequence or complex element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    /// Rust struct name (PascalCase).
    pub name: String,
    /// Original DFDL element/type name.
    pub dfdl_name: String,
    /// Documentation comment for the struct.
    pub doc: String,
    /// Optional fixed total byte length on wire (e.g. 64 bytes).
    pub explicit_len: Option<usize>,
    /// Optional fill byte used when padding to explicit length (e.g. `b'X'`).
    pub fill_byte: Option<u8>,
    /// Optional initiator byte expected at the start of this struct (e.g. `'T'`).
    pub initiator: Option<u8>,
    /// Ordered list of member fields.
    pub fields: Vec<FieldDef>,
}

/// An individual branch of a choice sum-type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceBranchDef {
    /// Variant identifier in the enum (e.g. `Track`).
    pub variant_name: String,
    /// Inner payload struct name (e.g. `TrackMessage`).
    pub payload_type: String,
    /// 1-byte wire key or initiator character for dispatch (e.g. `b'T'`).
    pub dispatch_key: u8,
    /// Documentation comment for this variant.
    pub doc: String,
}

/// Definition of a generated Rust enum representing a DFDL choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceDef {
    /// Rust enum name (PascalCase).
    pub name: String,
    /// Original DFDL choice or element name.
    pub dfdl_name: String,
    /// Documentation comment for the choice.
    pub doc: String,
    /// Alternative branches.
    pub branches: Vec<ChoiceBranchDef>,
}

/// An individual variant in a generated simple enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariantDef {
    /// Variant identifier (PascalCase).
    pub name: String,
    /// Wire byte representation (e.g. `b'S'`).
    pub byte_val: u8,
    /// Documentation comment for the variant.
    pub doc: String,
}

/// Definition of a generated simple enum (e.g. `TrackPointType`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDef {
    /// Rust enum name (PascalCase).
    pub name: String,
    /// Documentation comment for the enum.
    pub doc: String,
    /// List of valid variants and their wire values.
    pub variants: Vec<EnumVariantDef>,
}

/// Complete Intermediate Representation model for an entire DFDL schema.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodeGenIr {
    /// Simple value enumerations.
    pub enums: Vec<EnumDef>,
    /// Complex element structs.
    pub structs: Vec<StructDef>,
    /// Choice sum-types.
    pub choices: Vec<ChoiceDef>,
    /// Primary root type name (struct or choice).
    pub root_type: Option<String>,
}
