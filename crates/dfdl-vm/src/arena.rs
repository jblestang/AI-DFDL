//! Zero-Allocation Event Sinks and Infoset Output Adapters.
//!
//! Provides the [`VmEventSink`] trait and optimized sinks, including contiguous
//! event buffers with $O(1)$ zero-allocation backtracking for choice points,
//! and adapters to traditional [`InfosetDocument`] representations.

extern crate alloc;
use alloc::string::ToString;
use alloc::vec::Vec;

use dfdl_core::error::{DFDLError, DFDLErrorKind, DFDLResult};
use dfdl_core::infoset::state::ElementState;
use dfdl_core::infoset::tree::{InfosetDocument, InfosetElement, InfosetNode};
use dfdl_core::infoset::value::DfdlValue;
use dfdl_core::util::try_push;

use crate::isa::{SymbolId, SymbolTable};

/// Individual event emitted by the DFDL Virtual Machine during bitstream decoding.
#[derive(Debug, Clone, PartialEq)]
pub enum VmEvent<'a> {
    /// Start of a complex element container.
    StartElement(SymbolId),
    /// End of a complex element container.
    EndElement(SymbolId),
    /// Emitted 64-bit integer scalar value.
    ScalarInt {
        /// Associated symbol identifier.
        id: SymbolId,
        /// Integer value.
        val: i64,
        /// True if signed two's complement, false if unsigned.
        is_signed: bool,
    },
    /// Emitted IEEE 754 floating-point scalar value.
    ScalarFloat {
        /// Associated symbol identifier.
        id: SymbolId,
        /// Float value.
        val: f64,
    },
    /// Emitted boolean scalar value.
    ScalarBool {
        /// Associated symbol identifier.
        id: SymbolId,
        /// Boolean value.
        val: bool,
    },
    /// Emitted string scalar value.
    ScalarString {
        /// Associated symbol identifier.
        id: SymbolId,
        /// Borrowed string slice.
        val: &'a str,
    },
}

/// Abstract event receiver for the DFDL Virtual Machine.
///
/// Enables streaming event output, contiguous arena storage, or direct
/// conversion into owned DOM trees.
pub trait VmEventSink {
    /// Handles an event emitted by the VM.
    ///
    /// # Errors
    /// Returns [`DFDLError`] if the event violates sink limits or constraints.
    fn on_event(&mut self, event: VmEvent<'_>) -> DFDLResult<()>;

    /// Returns a lightweight marker of current sink state for backtracking.
    fn checkpoint(&self) -> usize;

    /// Restores sink state to a previous marker without deallocating memory.
    fn rollback(&mut self, mark: usize);
}

/// Contiguous zero-allocation event buffer.
///
/// Stores events sequentially in a pre-allocated vector. Choice backtracking
/// executes in $O(1)$ by truncating the event vector without invoking the
/// system memory allocator or dropping recursive structures.
#[derive(Debug, Clone, Default)]
pub struct ArenaEventSink {
    events: Vec<StoredEvent>,
}

/// Concrete stored event representation in [`ArenaEventSink`].
#[derive(Debug, Clone, PartialEq)]
pub enum StoredEvent {
    /// Start of a complex element container.
    StartElement(SymbolId),
    /// End of a complex element container.
    EndElement(SymbolId),
    /// Emitted 64-bit integer scalar value.
    ScalarInt {
        /// Associated symbol identifier.
        id: SymbolId,
        /// Integer value.
        val: i64,
        /// True if signed two's complement, false if unsigned.
        is_signed: bool,
    },
    /// Emitted IEEE 754 floating-point scalar value.
    ScalarFloat {
        /// Associated symbol identifier.
        id: SymbolId,
        /// Float value.
        val: f64,
    },
    /// Emitted boolean scalar value.
    ScalarBool {
        /// Associated symbol identifier.
        id: SymbolId,
        /// Boolean value.
        val: bool,
    },
    /// Emitted string scalar value.
    ScalarString {
        /// Associated symbol identifier.
        id: SymbolId,
        /// Stored string (inlined if <= 16 bytes).
        val: StoredString,
    },
}

/// Inlined small-string storage avoiding heap allocations for strings up to 16 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredString {
    /// Inlined UTF-8 buffer for short strings.
    Inline {
        /// Number of valid bytes in `data` (0..=16).
        len: u8,
        /// Inlined byte storage.
        data: [u8; 16],
    },
    /// Heap-allocated fallback for strings longer than 16 bytes.
    Heap(alloc::string::String),
}

impl StoredString {
    /// Creates a [`StoredString`] from a string slice without heap allocation if `s.len() <= 16`.
    #[must_use]
    pub fn new(s: &str) -> Self {
        let bytes = s.as_bytes();
        if bytes.len() <= 16 {
            let mut data = [0u8; 16];
            if let Some(target) = data.get_mut(..bytes.len()) {
                target.copy_from_slice(bytes);
            }
            Self::Inline {
                len: bytes.len() as u8,
                data,
            }
        } else {
            Self::Heap(alloc::string::String::from(s))
        }
    }

    /// Returns the string slice.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Inline { len, data } => {
                let slice = data.get(..*len as usize).unwrap_or(&[]);
                core::str::from_utf8(slice).unwrap_or("")
            }
            Self::Heap(s) => s.as_str(),
        }
    }
}

impl core::fmt::Display for StoredString {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::ops::Deref for StoredString {
    type Target = str;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl ArenaEventSink {
    /// Creates a new empty arena event sink with default capacity.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(32)
    }

    /// Creates an arena with initial capacity.
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
        }
    }

    /// Returns a slice of all stored events.
    #[inline]
    #[must_use]
    pub fn events(&self) -> &[StoredEvent] {
        &self.events
    }

    /// Returns a slice of all stored events.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns true if no events have been stored.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Clears the arena for reuse without reallocating underlying capacity.
    #[inline]
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

impl VmEventSink for ArenaEventSink {
    fn on_event(&mut self, event: VmEvent<'_>) -> DFDLResult<()> {
        let stored = match event {
            VmEvent::StartElement(id) => StoredEvent::StartElement(id),
            VmEvent::EndElement(id) => StoredEvent::EndElement(id),
            VmEvent::ScalarInt { id, val, is_signed } => {
                StoredEvent::ScalarInt { id, val, is_signed }
            }
            VmEvent::ScalarFloat { id, val } => StoredEvent::ScalarFloat { id, val },
            VmEvent::ScalarBool { id, val } => StoredEvent::ScalarBool { id, val },
            VmEvent::ScalarString { id, val } => StoredEvent::ScalarString {
                id,
                val: StoredString::new(val),
            },
        };
        try_push(&mut self.events, stored)
    }

    #[inline]
    fn checkpoint(&self) -> usize {
        self.events.len()
    }

    #[inline]
    fn rollback(&mut self, mark: usize) {
        self.events.truncate(mark);
    }
}

/// Discarding event sink used for pure bitstream verification and maximum throughput.
#[derive(Debug, Default)]
pub struct NullEventSink {
    count: usize,
}

impl NullEventSink {
    /// Creates a new null sink.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self { count: 0 }
    }
}

impl VmEventSink for NullEventSink {
    #[inline]
    fn on_event(&mut self, _event: VmEvent<'_>) -> DFDLResult<()> {
        self.count = self.count.saturating_add(1);
        Ok(())
    }

    #[inline]
    fn checkpoint(&self) -> usize {
        self.count
    }

    #[inline]
    fn rollback(&mut self, mark: usize) {
        self.count = mark;
    }
}

/// Bridge adapter constructing a full standard [`InfosetDocument`] from VM events.
#[derive(Debug, Clone)]
pub struct DomBuilderSink<'a> {
    symbols: &'a SymbolTable,
    stack: Vec<InfosetElement>,
    root: Option<InfosetElement>,
}

impl<'a> DomBuilderSink<'a> {
    /// Creates a new DOM builder associated with a program's symbol table.
    #[inline]
    #[must_use]
    pub fn new(symbols: &'a SymbolTable) -> Self {
        Self {
            symbols,
            stack: Vec::new(),
            root: None,
        }
    }

    /// Consumes the sink and returns the completed [`InfosetDocument`].
    ///
    /// # Errors
    /// Returns [`DFDLError`] if the document has no root or unbalanced elements.
    pub fn into_document(mut self) -> DFDLResult<InfosetDocument> {
        let root = self.root.or_else(|| self.stack.pop()).ok_or_else(|| {
            DFDLError::new(DFDLErrorKind::Parse, "VM emitted empty infoset document")
        })?;
        Ok(InfosetDocument::with_root(root))
    }
}

impl<'a> VmEventSink for DomBuilderSink<'a> {
    fn on_event(&mut self, event: VmEvent<'_>) -> DFDLResult<()> {
        match event {
            VmEvent::StartElement(id) => {
                let qname = self.symbols.resolve(id).cloned().ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unresolved SymbolId in StartElement")
                })?;
                let elem = InfosetElement::complex(qname);
                try_push(&mut self.stack, elem)?;
                Ok(())
            }
            VmEvent::EndElement(id) => {
                let elem = self.stack.pop().ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unexpected EndElement in VM sink")
                })?;
                if let Some(expected_qname) = self.symbols.resolve(id) {
                    if &elem.name != expected_qname {
                        return Err(DFDLError::new(
                            DFDLErrorKind::Parse,
                            "Mismatched EndElement QName in VM sink",
                        ));
                    }
                }
                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))?;
                } else {
                    self.root = Some(elem);
                }
                Ok(())
            }
            VmEvent::ScalarInt { id, val, is_signed } => {
                let qname = self.symbols.resolve(id).cloned().ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unresolved SymbolId in ScalarInt")
                })?;
                let dfdl_val = if is_signed {
                    DfdlValue::Long(val)
                } else {
                    DfdlValue::UnsignedLong(val as u64)
                };
                if let Some(top) = self.stack.last_mut() {
                    if top.name == qname
                        && matches!(top.state, ElementState::NoValue)
                        && top.children.is_empty()
                    {
                        top.state = ElementState::Value(dfdl_val);
                        return Ok(());
                    }
                }
                let elem = InfosetElement::simple(qname, ElementState::Value(dfdl_val));
                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))?;
                } else {
                    self.root = Some(elem);
                }
                Ok(())
            }
            VmEvent::ScalarFloat { id, val } => {
                let qname = self.symbols.resolve(id).cloned().ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unresolved SymbolId in ScalarFloat")
                })?;
                let dfdl_val = DfdlValue::Double(val);
                if let Some(top) = self.stack.last_mut() {
                    if top.name == qname
                        && matches!(top.state, ElementState::NoValue)
                        && top.children.is_empty()
                    {
                        top.state = ElementState::Value(dfdl_val);
                        return Ok(());
                    }
                }
                let elem = InfosetElement::simple(qname, ElementState::Value(dfdl_val));
                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))?;
                } else {
                    self.root = Some(elem);
                }
                Ok(())
            }
            VmEvent::ScalarBool { id, val } => {
                let qname = self.symbols.resolve(id).cloned().ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unresolved SymbolId in ScalarBool")
                })?;
                let dfdl_val = DfdlValue::Boolean(val);
                if let Some(top) = self.stack.last_mut() {
                    if top.name == qname
                        && matches!(top.state, ElementState::NoValue)
                        && top.children.is_empty()
                    {
                        top.state = ElementState::Value(dfdl_val);
                        return Ok(());
                    }
                }
                let elem = InfosetElement::simple(qname, ElementState::Value(dfdl_val));
                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))?;
                } else {
                    self.root = Some(elem);
                }
                Ok(())
            }
            VmEvent::ScalarString { id, val } => {
                let qname = self.symbols.resolve(id).cloned().ok_or_else(|| {
                    DFDLError::new(DFDLErrorKind::Parse, "Unresolved SymbolId in ScalarString")
                })?;
                let dfdl_val = DfdlValue::String(val.to_string());
                if let Some(top) = self.stack.last_mut() {
                    if top.name == qname
                        && matches!(top.state, ElementState::NoValue)
                        && top.children.is_empty()
                    {
                        top.state = ElementState::Value(dfdl_val);
                        return Ok(());
                    }
                }
                let elem = InfosetElement::simple(qname, ElementState::Value(dfdl_val));
                if let Some(parent) = self.stack.last_mut() {
                    parent.try_add_child(InfosetNode::Element(elem))?;
                } else {
                    self.root = Some(elem);
                }
                Ok(())
            }
        }
    }

    #[inline]
    fn checkpoint(&self) -> usize {
        self.stack.len()
    }

    #[inline]
    fn rollback(&mut self, mark: usize) {
        self.stack.truncate(mark);
    }
}
