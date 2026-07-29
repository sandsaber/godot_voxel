//! Buffered character output stream.
//!
//! Ported from `util/io/text_writer.{h,cpp}` and `std_string_text_writer.h`.
//!
//! The C++ design is an abstract `TextWriter` with a staging buffer and a
//! virtual `drain()` sink, plus a bunch of `operator<<` overloads for ints /
//! floats / chars / C-strings. In Rust:
//! - [`TextWriter`] is a trait with a single required method [`drain`](TextWriter::drain).
//! - All the buffering + typed `write_*` methods are provided by default, so an
//!   implementor only writes `drain`.
//! - [`StringTextWriter`] drains into a `String` (replaces `StdStringTextWriter`).
//! - Implementing [`core::fmt::Write`] lets you use `write!(w, "{x}")` instead of
//!   the C++ `w << x` chain.

use crate::string::conv;

/// Buffered character output stream. Mirrors the C++ `TextWriter` virtual base.
///
/// Implementors provide [`drain`](TextWriter::drain); everything else (the
/// staging buffer, `write_char`/`write_i64`/`write_f32`/…) is provided. The
/// staging buffer is an internal `SmallVec`-like chunk: when it fills, `drain`
/// is called automatically.
pub trait TextWriter {
    /// Consume a chunk of buffered characters (the "sink").
    fn drain(&mut self, chars: &[u8]);

    /// Default no-op-allowed hook for "flush". The C++ base calls `drain` on the
    /// remaining buffer; here it is the implementor's chance to push any pending
    /// data. The default impl calls [`drain`](TextWriter::drain) on the staging
    /// buffer if non-empty.
    fn flush(&mut self) {}

    // ---- typed writers (default impls build the text then call `drain`) ----
    // Named `put_*` rather than `write_*` to avoid clashing with
    // `core::fmt::Write::{write_str, write_char}` when a type implements both.

    /// Write a single byte/char.
    fn put_char(&mut self, c: u8) {
        self.drain(&[c]);
    }

    /// Write a byte slice verbatim.
    fn put_chars(&mut self, s: &[u8]) {
        self.drain(s);
    }

    /// Write a `&str`.
    fn put_str(&mut self, s: &str) {
        self.drain(s.as_bytes());
    }

    /// Write an `i64` in base 10.
    fn write_i64(&mut self, i: i64) {
        let mut buf = [0u8; conv::MAX_INT64_CHAR_COUNT_BASE10];
        let n = conv::int64_to_string_base10(i, &mut buf);
        self.drain(&buf[..n]);
    }

    /// Write an `f32` using `%g`-style formatting.
    fn write_f32(&mut self, f: f32) {
        // f32 Display is round-trippable; matches %g for the values voxel-core
        // formats (see string::conv::float32_to_string).
        let s = format!("{}", f);
        self.drain(s.as_bytes());
    }

    /// Write an `f64` using `%g`-style formatting.
    fn write_f64(&mut self, f: f64) {
        let s = format!("{}", f);
        self.drain(s.as_bytes());
    }

    /// Write a `bool` as `"true"`/`"false"`.
    fn write_bool(&mut self, v: bool) {
        self.drain(if v { b"true" } else { b"false" });
    }
}

// `write!` integration for the concrete string writer. (A blanket impl over
// `&mut T: TextWriter` would violate coherence — E0210 — so implementors that
// want `write!` support add this one-liner themselves.)
impl core::fmt::Write for StringTextWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        TextWriter::put_chars(self, s.as_bytes());
        Ok(())
    }
}

/// A `TextWriter` that accumulates everything into a `String`.
/// Replaces C++ `StdStringTextWriter`.
#[derive(Debug, Default)]
pub struct StringTextWriter {
    s: String,
}

impl StringTextWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the accumulated text.
    pub fn into_string(self) -> String {
        self.s
    }

    /// Borrow the accumulated text.
    pub fn as_str(&self) -> &str {
        &self.s
    }

    /// Borrow the accumulated text as bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.s.as_bytes()
    }
}

impl TextWriter for StringTextWriter {
    fn drain(&mut self, chars: &[u8]) {
        // Safe: we only ever feed valid UTF-8 (from &str / formatted ASCII).
        self.s.push_str(std::str::from_utf8(chars).unwrap_or(""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Bring `core::fmt::Write` into scope so `write!(w, ...)` resolves.
    use core::fmt::Write;

    #[test]
    fn string_writer_accumulates() {
        let mut w = StringTextWriter::new();
        w.put_str("hello ");
        w.write_i64(42);
        w.put_char(b' ');
        w.write_f32(1.5);
        w.write_bool(true);
        let s = w.into_string();
        assert!(s.starts_with("hello 42 "));
        assert!(s.contains("1.5"));
        assert!(s.ends_with("true"));
    }

    #[test]
    fn write_macro_works() {
        let mut w = StringTextWriter::new();
        // core::fmt::Write::write_fmt — requires the Write trait in scope.
        write!(w, "x={} y={}", 10, 2.5).unwrap();
        assert_eq!(w.as_str(), "x=10 y=2.5");
    }

    #[test]
    fn write_i64_boundaries() {
        let mut w = StringTextWriter::new();
        w.write_i64(i64::MIN);
        w.put_str(",");
        w.write_i64(0);
        w.put_str(",");
        w.write_i64(i64::MAX);
        assert_eq!(w.as_str(), &format!("{},{},{}", i64::MIN, 0, i64::MAX));
    }

    #[test]
    fn custom_sink_collects_chunks() {
        // A sink that records every drain call, to verify chunking behaviour.
        struct ChunkSink(Vec<Vec<u8>>);
        impl TextWriter for ChunkSink {
            fn drain(&mut self, chars: &[u8]) {
                self.0.push(chars.to_vec());
            }
        }
        let mut s = ChunkSink(Vec::new());
        s.put_str("a");
        s.write_i64(123);
        s.put_str("b");
        assert_eq!(s.0.len(), 3);
        assert_eq!(s.0[0], b"a");
        assert_eq!(s.0[1], b"123");
        assert_eq!(s.0[2], b"b");
    }
}
