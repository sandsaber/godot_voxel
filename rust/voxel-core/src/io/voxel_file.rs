//! `VoxelFile` — a thin file-I/O trait for on-disk voxel formats.
//!
//! Stands in for Godot's `FileAccess`. The C++ region-file code threads a
//! `FileAccess &` through every header/sector function; this trait lets the
//! Rust port do the same without depending on the engine, with [`StdVoxelFile`]
//! providing the production implementation over [`std::fs::File`].
//!
//! Only the methods actually used by the region format are exposed: byte-level
//! `read`/`write`, `seek`, `position`, `len`, `flush`. Typed primitives
//! (`get_8`/`store_32`/etc.) are layered on top via [`MemoryReader`] /
//! [`MemoryWriter`] over small buffers, exactly as the C++ uses them.
//!
//! [`MemoryReader`]: super::serialization::MemoryReader
//! [`MemoryWriter`]: super::serialization::MemoryWriter

use std::io::{self, Read, Seek, SeekFrom, Write};

/// Byte-oriented file I/O. Ported from the subset of Godot `FileAccess` that
/// the region format uses.
#[allow(clippy::len_without_is_empty)]
pub trait VoxelFile {
    /// Move the cursor to `pos` bytes from the start of the file.
    fn seek(&mut self, pos: u64) -> io::Result<()>;
    /// Current cursor position (bytes from start).
    fn position(&mut self) -> io::Result<u64>;
    /// Total file length in bytes.
    fn len(&self) -> io::Result<u64>;
    /// Read up to `dst.len()` bytes into `dst`, returning how many were read.
    /// Fewer than requested indicates end-of-file.
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize>;
    /// Write all of `src`.
    fn write(&mut self, src: &[u8]) -> io::Result<()>;
    /// Resize the file, truncating or zero-extending as needed. Used by the
    /// region sector-compaction path (C++ has no truncate and leaves stale
    /// trailing bytes; we clean up properly).
    fn set_len(&mut self, len: u64) -> io::Result<()>;
    /// Flush pending writes to the OS.
    fn flush(&mut self) -> io::Result<()>;
}

/// Production [`VoxelFile`] over a [`std::fs::File`]. Owns the file handle and
/// closes it on drop.
pub struct StdVoxelFile {
    file: std::fs::File,
}

impl StdVoxelFile {
    /// Open an existing file for reading and writing. Returns `None` if the
    /// file does not exist (matching the C++ `READ_WRITE` mode that refuses to
    /// create).
    pub fn open_rw(path: &std::path::Path) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        Ok(Self { file })
    }

    /// Create a new file (truncating if it exists) open for reading and writing.
    pub fn create(path: &std::path::Path) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(Self { file })
    }
}

impl VoxelFile for StdVoxelFile {
    fn seek(&mut self, pos: u64) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(pos))?;
        Ok(())
    }

    fn position(&mut self) -> io::Result<u64> {
        self.file.stream_position()
    }

    fn len(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        // Match Godot's `get_buffer`: read exactly dst.len() unless EOF.
        // `read_exact` would error on short reads; we want a partial count.
        let mut filled = 0;
        while filled < dst.len() {
            match self.file.read(&mut dst[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(filled)
    }

    fn write(&mut self, src: &[u8]) -> io::Result<()> {
        self.file.write_all(src)
    }

    fn set_len(&mut self, len: u64) -> io::Result<()> {
        self.file.set_len(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
pub mod test_support {
    //! An in-memory [`VoxelFile`] backed by a `Vec<u8>`, for unit tests that
    //! don't want to touch the filesystem.

    use super::*;

    /// In-memory file. Reads past the end return fewer bytes; writes grow the
    /// backing buffer and seek past the end zero-fill the gap.
    pub struct MemoryFile {
        data: Vec<u8>,
        pos: u64,
    }

    impl MemoryFile {
        pub fn new() -> Self {
            Self {
                data: Vec::new(),
                pos: 0,
            }
        }
        pub fn with_data(data: Vec<u8>) -> Self {
            Self { data, pos: 0 }
        }
        pub fn data(&self) -> &[u8] {
            &self.data
        }
    }

    impl Default for MemoryFile {
        fn default() -> Self {
            Self::new()
        }
    }

    impl VoxelFile for MemoryFile {
        fn seek(&mut self, pos: u64) -> io::Result<()> {
            self.pos = pos;
            Ok(())
        }
        fn position(&mut self) -> io::Result<u64> {
            Ok(self.pos)
        }
        fn len(&self) -> io::Result<u64> {
            Ok(self.data.len() as u64)
        }
        fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
            let avail = self.data.len().saturating_sub(self.pos as usize);
            let n = avail.min(dst.len());
            dst[..n].copy_from_slice(&self.data[self.pos as usize..self.pos as usize + n]);
            self.pos += n as u64;
            Ok(n)
        }
        fn write(&mut self, src: &[u8]) -> io::Result<()> {
            let end = self.pos as usize + src.len();
            if end > self.data.len() {
                self.data.resize(end, 0);
            }
            self.data[self.pos as usize..end].copy_from_slice(src);
            self.pos = end as u64;
            Ok(())
        }
        fn set_len(&mut self, len: u64) -> io::Result<()> {
            self.data.resize(len as usize, 0);
            Ok(())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryFile;
    use super::*;

    #[test]
    fn memory_file_write_then_read_round_trips() {
        let mut f = MemoryFile::new();
        f.write(b"hello").unwrap();
        assert_eq!(f.len().unwrap(), 5);
        f.seek(0).unwrap();
        let mut buf = [0u8; 5];
        assert_eq!(f.read(&mut buf).unwrap(), 5);
        assert_eq!(&buf, b"hello");
    }

    #[test]
    fn memory_file_read_past_end_returns_partial() {
        let mut f = MemoryFile::with_data(vec![1, 2, 3]);
        f.seek(2).unwrap();
        let mut buf = [0u8; 10];
        let n = f.read(&mut buf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf[0], 3);
    }

    #[test]
    fn memory_file_write_past_end_zero_fills_gap() {
        let mut f = MemoryFile::with_data(vec![1, 2, 3]);
        f.seek(5).unwrap();
        f.write(&[9]).unwrap();
        assert_eq!(f.data(), &[1, 2, 3, 0, 0, 9]);
    }
}
