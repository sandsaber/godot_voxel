//! Byte-level serialization (endianness-aware readers/writers).
//!
//! Ported from `util/io/serialization.h`. Provides:
//! - [`Endianness`] enum + platform detection.
//! - [`MemoryWriter`] / [`MemoryWriterExistingBuffer`] — append bytes (u8/u16/
//!   u32/u64/float) with a chosen byte order.
//! - [`MemoryReader`] — read them back.
//!
//! The C++ `MemoryWriterTemplate<Container_T>` is generic over the backing store
//! (a growing `StdVector<uint8_t>` vs. a fixed `ByteSpanWithPosition`); in Rust we
//! express that with a small [`ByteSink`] trait so the same writer logic serves
//! both. The default [`MemoryWriter`] wraps a `Vec<u8>`.

/// Byte order for serialized integers. Matches `Endianness`.
///
/// Note: the C++ default is **big-endian** ("network byte order") for historical
/// reasons. [`MemoryWriter`] / [`MemoryReader`] keep that default so existing
/// on-disk/stream formats stay binary-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endianness {
    BigEndian,
    LittleEndian,
}

/// Detect the host byte order. Matches `get_platform_endianness`.
#[inline]
pub fn get_platform_endianness() -> Endianness {
    // Mirrors the C++ `*(char*)&1 == 1` trick. `cfg!(target_endian = "little")`
    // is the compile-time equivalent and is what std::endian would give in C++20.
    if cfg!(target_endian = "little") {
        Endianness::LittleEndian
    } else {
        Endianness::BigEndian
    }
}

/// A growable or fixed byte container that the writer appends to.
///
/// Implemented for `Vec<u8>` (the default, growable [`MemoryWriter`]) and for
/// [`ExistingBuffer`] (a fixed slice with a cursor, matching C++'s
/// `ByteSpanWithPosition` / `MemoryWriterExistingBuffer`).
pub trait ByteSink {
    /// Append a single byte. Bounds checking is the implementor's responsibility
    /// (the fixed-buffer variant panics in debug if full).
    fn push_byte(&mut self, v: u8);
    /// Append `bytes` verbatim.
    fn extend_from_slice(&mut self, bytes: &[u8]);
}

impl ByteSink for Vec<u8> {
    #[inline]
    fn push_byte(&mut self, v: u8) {
        self.push(v);
    }
    #[inline]
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        Vec::extend_from_slice(self, bytes);
    }
}

/// A fixed-capacity byte buffer with a write cursor. Matches `ByteSpanWithPosition`.
/// Writes past the end panic in debug (matching the C++ `ZN_ASSERT`).
pub struct ExistingBuffer<'a> {
    data: &'a mut [u8],
    pos: usize,
}

impl<'a> ExistingBuffer<'a> {
    pub fn new(data: &'a mut [u8], initial_pos: usize) -> Self {
        debug_assert!(initial_pos <= data.len());
        Self {
            data,
            pos: initial_pos,
        }
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Move the cursor (no allocation). Matches `resize`. Must be `<= capacity`.
    pub fn set_pos(&mut self, new_pos: usize) {
        debug_assert!(new_pos <= self.data.len());
        self.pos = new_pos;
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.data.len()
    }
}

impl ByteSink for ExistingBuffer<'_> {
    #[inline]
    fn push_byte(&mut self, v: u8) {
        debug_assert!(self.pos < self.data.len(), "ExistingBuffer overflow");
        self.data[self.pos] = v;
        self.pos += 1;
    }
    #[inline]
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        let end = self.pos + bytes.len();
        debug_assert!(end <= self.data.len(), "ExistingBuffer overflow on extend");
        self.data[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
    }
}

/// Writes typed values into a [`ByteSink`] with a chosen byte order. Matches
/// `MemoryWriterTemplate<Container_T>` / `MemoryWriter`.
pub struct MemoryWriter<'a, S: ByteSink> {
    sink: &'a mut S,
    endianness: Endianness,
}

impl<'a, S: ByteSink> MemoryWriter<'a, S> {
    pub fn new(sink: &'a mut S, endianness: Endianness) -> Self {
        Self { sink, endianness }
    }

    /// Big-endian (network order) writer — the C++ default.
    pub fn big(sink: &'a mut S) -> Self {
        Self::new(sink, Endianness::BigEndian)
    }

    /// Little-endian writer.
    pub fn little(sink: &'a mut S) -> Self {
        Self::new(sink, Endianness::LittleEndian)
    }

    #[inline]
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    #[inline]
    pub fn store_8(&mut self, v: u8) {
        self.sink.push_byte(v);
    }

    #[inline]
    pub fn store_16(&mut self, v: u16) {
        let [b0, b1] = match self.endianness {
            Endianness::BigEndian => [(v >> 8) as u8, (v & 0xff) as u8],
            Endianness::LittleEndian => [(v & 0xff) as u8, (v >> 8) as u8],
        };
        self.sink.push_byte(b0);
        self.sink.push_byte(b1);
    }

    #[inline]
    pub fn store_32(&mut self, v: u32) {
        let bytes = match self.endianness {
            Endianness::BigEndian => [
                (v >> 24) as u8,
                (v >> 16) as u8,
                (v >> 8) as u8,
                (v & 0xff) as u8,
            ],
            Endianness::LittleEndian => [
                (v & 0xff) as u8,
                (v >> 8) as u8,
                (v >> 16) as u8,
                (v >> 24) as u8,
            ],
        };
        for b in bytes {
            self.sink.push_byte(b);
        }
    }

    #[inline]
    pub fn store_64(&mut self, v: u64) {
        let bytes = match self.endianness {
            Endianness::BigEndian => [
                (v >> 56) as u8,
                (v >> 48) as u8,
                (v >> 40) as u8,
                (v >> 32) as u8,
                (v >> 24) as u8,
                (v >> 16) as u8,
                (v >> 8) as u8,
                (v & 0xff) as u8,
            ],
            Endianness::LittleEndian => [
                (v & 0xff) as u8,
                (v >> 8) as u8,
                (v >> 16) as u8,
                (v >> 24) as u8,
                (v >> 32) as u8,
                (v >> 40) as u8,
                (v >> 48) as u8,
                (v >> 56) as u8,
            ],
        };
        for b in bytes {
            self.sink.push_byte(b);
        }
    }

    /// Store an `f32` by reinterpreting its bits. Matches `store_float`.
    #[inline]
    pub fn store_float(&mut self, v: f32) {
        self.store_32(v.to_bits());
    }

    /// Store a raw byte slice verbatim. Matches `store_buffer`.
    #[inline]
    pub fn store_buffer(&mut self, data: &[u8]) {
        self.sink.extend_from_slice(data);
    }
}

/// Reads typed values from a byte slice with a cursor and chosen byte order.
/// Matches `MemoryReader`.
pub struct MemoryReader<'a> {
    data: &'a [u8],
    pos: usize,
    endianness: Endianness,
}

impl<'a> MemoryReader<'a> {
    pub fn new(data: &'a [u8], endianness: Endianness) -> Self {
        Self {
            data,
            pos: 0,
            endianness,
        }
    }

    /// Big-endian reader (the C++ default).
    pub fn big(data: &'a [u8]) -> Self {
        Self::new(data, Endianness::BigEndian)
    }

    /// Little-endian reader.
    pub fn little(data: &'a [u8]) -> Self {
        Self::new(data, Endianness::LittleEndian)
    }

    #[inline]
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    /// Switch the byte order mid-stream. The C++ `MemoryReader` exposes
    /// `endianness` as a public mutable field; the only in-tree use is
    /// `instance_data` toggling to big-endian when it sees a legacy version-0
    /// header. Mirrored here as an explicit setter.
    #[inline]
    pub fn set_endianness(&mut self, endianness: Endianness) {
        self.endianness = endianness;
    }

    /// Bytes consumed so far. Matches `get_position`.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bytes still available.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> &'a [u8] {
        // Bounds check: the C++ versions omit ERR_FAIL_COND but read OOB otherwise;
        // we panic to surface corruption early rather than silently reading zeros.
        let end = self.pos + n;
        assert!(
            end <= self.data.len(),
            "MemoryReader: read past end (pos={}, need {n}, len={})",
            self.pos,
            self.data.len()
        );
        let slice = &self.data[self.pos..end];
        self.pos = end;
        slice
    }

    #[inline]
    pub fn get_8(&mut self) -> u8 {
        self.take(1)[0]
    }

    #[inline]
    pub fn get_16(&mut self) -> u16 {
        let b = self.take(2);
        match self.endianness {
            Endianness::BigEndian => ((b[0] as u16) << 8) | b[1] as u16,
            Endianness::LittleEndian => b[0] as u16 | ((b[1] as u16) << 8),
        }
    }

    #[inline]
    pub fn get_32(&mut self) -> u32 {
        let b = self.take(4);
        match self.endianness {
            Endianness::BigEndian => {
                ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | b[3] as u32
            }
            Endianness::LittleEndian => {
                b[0] as u32 | ((b[1] as u32) << 8) | ((b[2] as u32) << 16) | ((b[3] as u32) << 24)
            }
        }
    }

    #[inline]
    pub fn get_64(&mut self) -> u64 {
        let b = self.take(8);
        match self.endianness {
            Endianness::BigEndian => {
                ((b[0] as u64) << 56)
                    | ((b[1] as u64) << 48)
                    | ((b[2] as u64) << 40)
                    | ((b[3] as u64) << 32)
                    | ((b[4] as u64) << 24)
                    | ((b[5] as u64) << 16)
                    | ((b[6] as u64) << 8)
                    | b[7] as u64
            }
            Endianness::LittleEndian => {
                b[0] as u64
                    | ((b[1] as u64) << 8)
                    | ((b[2] as u64) << 16)
                    | ((b[3] as u64) << 24)
                    | ((b[4] as u64) << 32)
                    | ((b[5] as u64) << 40)
                    | ((b[6] as u64) << 48)
                    | ((b[7] as u64) << 56)
            }
        }
    }

    /// Reinterpret the next 4 bytes as `f32`. Matches `get_float`.
    #[inline]
    pub fn get_float(&mut self) -> f32 {
        f32::from_bits(self.get_32())
    }

    /// Copy up to `dst.len()` bytes into `dst`, returning how many were copied
    /// (fewer if the source runs out). Matches `get_buffer`.
    #[inline]
    pub fn get_buffer(&mut self, dst: &mut [u8]) -> usize {
        let end = (self.pos + dst.len()).min(self.data.len());
        let len = end - self.pos;
        dst[..len].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        len
    }

    // ---- fallible variants -------------------------------------------------
    //
    // The `get_*` methods above panic on a short buffer, mirroring the C++
    // assumption that callers pre-validate lengths. Several on-disk formats
    // (e.g. `instance_data`) come from untrusted sources and need to bail out
    // cleanly on truncation instead of aborting. The `try_*` family returns
    // `Option`, leaving the cursor untouched on failure.

    /// Fallibly pull `n` bytes without panicking. Returns `None` if fewer than
    /// `n` bytes remain; the cursor is not advanced in that case.
    pub fn try_take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.data.len() {
            return None;
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Some(slice)
    }

    #[inline]
    pub fn try_get_8(&mut self) -> Option<u8> {
        self.try_take(1).map(|s| s[0])
    }

    #[inline]
    pub fn try_get_16(&mut self) -> Option<u16> {
        let b = self.try_take(2)?;
        Some(match self.endianness {
            Endianness::BigEndian => ((b[0] as u16) << 8) | b[1] as u16,
            Endianness::LittleEndian => b[0] as u16 | ((b[1] as u16) << 8),
        })
    }

    #[inline]
    pub fn try_get_32(&mut self) -> Option<u32> {
        let b = self.try_take(4)?;
        Some(match self.endianness {
            Endianness::BigEndian => {
                ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | b[3] as u32
            }
            Endianness::LittleEndian => {
                b[0] as u32 | ((b[1] as u32) << 8) | ((b[2] as u32) << 16) | ((b[3] as u32) << 24)
            }
        })
    }

    /// Reinterpret the next 4 bytes as `f32`, or `None` on truncation.
    #[inline]
    pub fn try_get_float(&mut self) -> Option<f32> {
        self.try_get_32().map(f32::from_bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_endian_is_little_on_x86() {
        // The dev/CI host is x86-64 (little). This just documents the assumption.
        assert_eq!(get_platform_endianness(), Endianness::LittleEndian);
    }

    #[test]
    fn writer_reader_roundtrip_big() {
        let mut buf = Vec::new();
        {
            let mut w = MemoryWriter::big(&mut buf);
            w.store_8(0xab);
            w.store_16(0x1234);
            w.store_32(0xdead_beef);
            w.store_64(0x0123_4567_89ab_cdef);
            w.store_float(1.5);
            w.store_buffer(&[9, 9, 9]);
        }
        // Big-endian: most-significant byte first.
        assert_eq!(buf[0], 0xab);
        assert_eq!(&buf[1..3], &[0x12, 0x34]);
        assert_eq!(&buf[3..7], &[0xde, 0xad, 0xbe, 0xef]);

        let mut r = MemoryReader::big(&buf);
        assert_eq!(r.get_8(), 0xab);
        assert_eq!(r.get_16(), 0x1234);
        assert_eq!(r.get_32(), 0xdead_beef);
        assert_eq!(r.get_64(), 0x0123_4567_89ab_cdef);
        assert_eq!(r.get_float(), 1.5);
        let mut tail = [0u8; 3];
        assert_eq!(r.get_buffer(&mut tail), 3);
        assert_eq!(tail, [9, 9, 9]);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn writer_reader_roundtrip_little() {
        let mut buf = Vec::new();
        {
            let mut w = MemoryWriter::little(&mut buf);
            w.store_16(0x1234);
            w.store_32(0xdead_beef);
        }
        // Little-endian: least-significant byte first.
        assert_eq!(&buf[0..2], &[0x34, 0x12]);
        assert_eq!(&buf[2..6], &[0xef, 0xbe, 0xad, 0xde]);

        let mut r = MemoryReader::little(&buf);
        assert_eq!(r.get_16(), 0x1234);
        assert_eq!(r.get_32(), 0xdead_beef);
    }

    #[test]
    fn reader_panics_past_end() {
        let buf = [1u8, 2, 3];
        let mut r = MemoryReader::big(&buf);
        r.get_8();
        r.get_16();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = r.get_8();
        }))
        .is_err());
    }

    #[test]
    fn get_buffer_truncates_at_end() {
        let buf = [1u8, 2, 3];
        let mut r = MemoryReader::big(&buf);
        r.get_8(); // pos = 1
        let mut dst = [0u8; 10];
        let n = r.get_buffer(&mut dst);
        assert_eq!(n, 2);
        assert_eq!(&dst[..n], &[2, 3]);
    }

    #[test]
    fn existing_buffer_writes_in_place() {
        let mut storage = [0u8; 8];
        {
            let mut buf = ExistingBuffer::new(&mut storage, 0);
            let mut w = MemoryWriter::big(&mut buf);
            w.store_32(0x11223344);
        }
        assert_eq!(&storage[..4], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(&storage[4..], &[0, 0, 0, 0]); // rest untouched
    }

    #[test]
    fn float_bits_roundtrip() {
        let mut buf = Vec::new();
        {
            let mut w = MemoryWriter::little(&mut buf);
            w.store_float(f32::NEG_INFINITY);
            w.store_float(0.0);
            w.store_float(-0.0);
        }
        let mut r = MemoryReader::little(&buf);
        let a = r.get_float();
        let b = r.get_float();
        let c = r.get_float();
        assert!(a.is_infinite() && a.is_sign_negative());
        assert_eq!(b, 0.0);
        assert!(c == 0.0 && c.is_sign_negative());
    }
}
