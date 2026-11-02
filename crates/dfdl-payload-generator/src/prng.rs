//! Deterministic, zero-dependency pseudo-random generator for reproducible payload generation.
//!
//! Implements a 64-bit SplitMix state initializer and XorShift64* generator conforming to
//! strict no-panic, saturating integer operations.

/// Deterministic pseudo-random number generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastPrng {
    state: u64,
}

impl Default for FastPrng {
    fn default() -> Self {
        Self::new(0x853C_49E6_748F_EA9Bu64)
    }
}

impl FastPrng {
    /// Creates a new PRNG with a 64-bit seed.
    pub fn new(seed: u64) -> Self {
        let initial = if seed == 0 {
            0x853C_49E6_748F_EA9Bu64
        } else {
            seed
        };
        let mut prng = Self { state: initial };
        let _ = prng.next_u64();
        prng
    }

    /// Generates the next pseudorandom 64-bit unsigned integer using SplitMix64 step.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15u64);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9u64);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EBu64);
        z ^ (z >> 31)
    }

    /// Generates the next pseudorandom 32-bit unsigned integer.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Generates an integer in the closed interval `[min, max]`.
    pub fn next_range_u64(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        let range = max.saturating_sub(min).saturating_add(1);
        let rem = self.next_u64() % range;
        min.saturating_add(rem)
    }

    /// Generates an integer in the closed interval `[min, max]`.
    pub fn next_range_i64(&mut self, min: i64, max: i64) -> i64 {
        if min >= max {
            return min;
        }
        let diff = (max as i128).saturating_sub(min as i128).saturating_add(1) as u64;
        let rem = (self.next_u64() % diff) as i64;
        min.saturating_add(rem)
    }

    /// Fills buffer with pseudorandom bytes.
    pub fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut chunks = dest.chunks_exact_mut(8);
        for chunk in chunks.by_ref() {
            let val = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&val);
        }
        let remainder = chunks.into_remainder();
        if !remainder.is_empty() {
            let val = self.next_u64().to_le_bytes();
            for (dest_byte, src_byte) in remainder.iter_mut().zip(val.iter()) {
                *dest_byte = *src_byte;
            }
        }
    }

    /// Returns a boolean with `p` chance of being true (0.0 to 1.0).
    pub fn gen_bool(&mut self, p: f64) -> bool {
        if p <= 0.0 {
            false
        } else if p >= 1.0 {
            true
        } else {
            let threshold = (p * (u32::MAX as f64)) as u32;
            self.next_u32() < threshold
        }
    }

    /// Selects an element randomly from a non-empty slice.
    pub fn choose<'a, T>(&mut self, slice: &'a [T]) -> Option<&'a T> {
        if slice.is_empty() {
            None
        } else {
            let idx = (self.next_u64() % (slice.len() as u64)) as usize;
            slice.get(idx)
        }
    }
}
