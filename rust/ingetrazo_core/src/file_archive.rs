// SPDX-License-Identifier: GPL-3.0-or-later
//! Deterministic, seekable ZIP output as produced by Python's ZipInfo.
//! The host's zlib is the codec used by the reference writer on macOS.
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedBytes;
use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::FileExt;

unsafe extern "C" {
    fn dup(fd: c_int) -> c_int;
}
fn file(fd: i32) -> io::Result<File> {
    // Own only the duplicate. The caller keeps its buffered file alive.
    let copy = unsafe { dup(fd) };
    if copy < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(copy) })
    }
}

// Public z_stream ABI from zlib.h. Null allocation functions select zlib's allocator.
#[repr(C)]
struct Stream {
    input: *const u8,
    available: c_uint,
    total_in: c_ulong,
    output: *mut u8,
    space: c_uint,
    total_out: c_ulong,
    msg: *const c_char,
    state: *mut c_void,
    alloc: Option<unsafe extern "C" fn(*mut c_void, c_uint, c_uint) -> *mut c_void>,
    free: Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>,
    opaque: *mut c_void,
    data_type: c_int,
    adler: c_ulong,
    reserved: c_ulong,
}
#[link(name = "z")]
unsafe extern "C" {
    fn zlibVersion() -> *const c_char;
    fn deflateInit2_(
        s: *mut Stream,
        level: c_int,
        method: c_int,
        bits: c_int,
        mem: c_int,
        strategy: c_int,
        version: *const c_char,
        size: c_int,
    ) -> c_int;
    fn deflate(s: *mut Stream, flush: c_int) -> c_int;
    fn deflateEnd(s: *mut Stream) -> c_int;
}
struct Deflater(Box<Stream>);
impl Deflater {
    fn new(level: i32) -> io::Result<Self> {
        // The zero representation is valid for z_stream's C fields. Keep
        // its address stable from init until end, including during errors.
        let mut s: Box<Stream> = Box::new(unsafe { std::mem::zeroed() });
        let result = unsafe {
            deflateInit2_(
                &mut *s,
                level,
                8,
                -15,
                8,
                0,
                zlibVersion(),
                size_of::<Stream>() as _,
            )
        };
        if result != 0 {
            return Err(io::Error::other("cannot initialize ZIP compression"));
        }
        Ok(Self(s))
    }
    fn push(
        &mut self,
        data: &[u8],
        finish: bool,
        buf: &mut [u8],
        dest: &mut File,
    ) -> io::Result<()> {
        self.0.input = data.as_ptr();
        self.0.available = data.len() as _;
        loop {
            self.0.output = buf.as_mut_ptr();
            self.0.space = buf.len() as _;
            let r = unsafe { deflate(&mut *self.0, if finish { 4 } else { 0 }) };
            if r != 0 && r != 1 {
                return Err(io::Error::other("ZIP compression failed"));
            }
            dest.write_all(&buf[..buf.len() - self.0.space as usize])?;
            if r == 1 || (!finish && self.0.available == 0) {
                return Ok(());
            }
        }
    }
}
impl Drop for Deflater {
    fn drop(&mut self) {
        unsafe {
            deflateEnd(&mut *self.0);
        }
    }
}

type Member = (String, bool, Option<PyBackedBytes>, Option<(i32, u64, u64)>);
struct Entry<'a> {
    name: &'a str,
    method: u16,
    crc: u32,
    packed: u64,
    size: u64,
    offset: u64,
    local_zip64: bool,
}
impl Entry<'_> {
    fn header(&self, central: bool, limit: u64) -> Vec<u8> {
        let large_sizes = if central {
            self.size > limit || self.packed > limit
        } else {
            self.local_zip64
        };
        let large_offset = central && self.offset > limit;
        let version: u16 = if self.local_zip64 || large_sizes || large_offset {
            45
        } else {
            20
        };
        let mut extra = Vec::new();
        if large_sizes {
            extra.extend(self.size.to_le_bytes());
            extra.extend(self.packed.to_le_bytes());
        }
        if large_offset {
            extra.extend(self.offset.to_le_bytes());
        }
        let extra_len = if extra.is_empty() {
            0
        } else {
            extra.len() as u16 + 4
        };
        let mut h = Vec::with_capacity(46 + self.name.len());
        if central {
            h.extend(0x02014b50u32.to_le_bytes());
            h.extend((0x0300u16 | version).to_le_bytes());
        } else {
            h.extend(0x04034b50u32.to_le_bytes());
        }
        for n in [
            version,
            if self.name.is_ascii() { 0 } else { 0x800 },
            self.method,
            0,
            33,
        ] {
            h.extend(n.to_le_bytes());
        }
        for n in [
            self.crc,
            if large_sizes {
                u32::MAX
            } else {
                self.packed as u32
            },
            if large_sizes {
                u32::MAX
            } else {
                self.size as u32
            },
        ] {
            h.extend(n.to_le_bytes());
        }
        h.extend((self.name.len() as u16).to_le_bytes());
        h.extend(extra_len.to_le_bytes());
        if central {
            h.extend([0; 6]); // comment, disk and internal attributes
            h.extend((0o600u32 << 16).to_le_bytes());
            h.extend(
                (if large_offset {
                    u32::MAX
                } else {
                    self.offset as u32
                })
                .to_le_bytes(),
            );
        }
        h.extend(self.name.bytes());
        if extra_len > 0 {
            h.extend(1u16.to_le_bytes());
            h.extend((extra_len - 4).to_le_bytes());
            h.extend(extra);
        }
        h
    }
}

#[pyclass(frozen)]
pub struct ArchiveProgress {
    total: std::sync::atomic::AtomicU64,
    written: std::sync::atomic::AtomicU64,
}

#[pymethods]
impl ArchiveProgress {
    #[new]
    fn new() -> Self {
        Self {
            total: 0.into(),
            written: 0.into(),
        }
    }

    fn fraction(&self) -> f64 {
        use std::sync::atomic::Ordering::Relaxed;
        (self.written.load(Relaxed) as f64 / self.total.load(Relaxed).max(1) as f64).min(1.0)
    }
}

#[pyfunction(signature = (fd, members, fast, limit=2_147_483_647, meter=None))]
pub fn igz_write_zip(
    py: Python<'_>,
    fd: i32,
    members: Vec<Member>,
    fast: bool,
    limit: u64,
    meter: Option<Py<ArchiveProgress>>,
) -> PyResult<bool> {
    let meter = meter.as_ref().map(Py::get);
    if let Some(meter) = meter {
        let size = members
            .iter()
            .map(|(_, _, b, r)| {
                b.as_ref()
                    .map_or_else(|| r.map_or(0, |(_, _, n)| n), |b| b.len() as u64)
            })
            .sum();
        meter
            .total
            .store(size, std::sync::atomic::Ordering::Relaxed);
    }
    py.detach(|| -> io::Result<bool> {
        if members
            .iter()
            .any(|(name, _, _, _)| name.len() > 65535 || name.contains(char::from(0)))
        {
            return Ok(false);
        }
        let mut dest = file(fd)?;
        let mut central = Vec::new();
        let mut input = vec![0; 1 << 20];
        let mut output = vec![0; 1 << 16];
        for (name, compressed, bytes, range) in &members {
            let size = match (bytes, range) {
                (Some(b), None) => b.len() as u64,
                (None, Some((_, _, n))) => *n,
                _ => return Err(io::Error::other("invalid ZIP member source")),
            };
            let offset = dest.stream_position()?;
            let mut e = Entry {
                name,
                method: if *compressed { 8 } else { 0 },
                crc: 0,
                packed: 0,
                size: 0,
                offset,
                local_zip64: (size as f64) * 1.05 > limit as f64,
            };
            dest.write_all(&e.header(false, limit))?;
            let start = dest.stream_position()?;
            let mut crc = crc32fast::Hasher::new();
            let mut codec = if *compressed {
                Some(Deflater::new(if fast { 1 } else { -1 })?)
            } else {
                None
            };
            let mut write = |chunk: &[u8]| -> io::Result<()> {
                crc.update(chunk);
                if let Some(meter) = meter {
                    meter
                        .written
                        .fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
                }
                e.size += chunk.len() as u64;
                if let Some(c) = codec.as_mut() {
                    c.push(chunk, false, &mut output, &mut dest)
                } else {
                    dest.write_all(chunk)
                }
            };
            if let Some(b) = bytes {
                for chunk in b.chunks(1 << 20) {
                    write(chunk)?;
                }
            } else if let Some((fd, off, size)) = range {
                let scratch = file(*fd)?;
                let mut at = 0;
                while at < *size {
                    let n = (size - at).min(input.len() as u64) as usize;
                    scratch.read_exact_at(
                        &mut input[..n],
                        off.checked_add(at)
                            .ok_or(io::Error::other("invalid scratch offset"))?,
                    )?;
                    write(&input[..n])?;
                    at += n as u64;
                }
            }
            if let Some(c) = codec.as_mut() {
                c.push(&[], true, &mut output, &mut dest)?;
            }
            let end = dest.stream_position()?;
            e.crc = crc.finalize();
            e.packed = end - start;
            dest.seek(SeekFrom::Start(offset))?;
            dest.write_all(&e.header(false, limit))?;
            dest.seek(SeekFrom::Start(end))?;
            central.extend(e.header(true, limit));
        }
        let offset = dest.stream_position()?;
        let count = members.len() as u64;
        let length = central.len() as u64;
        dest.write_all(&central)?;
        if count > 65535 || offset > limit || length > limit {
            let end = dest.stream_position()?;
            dest.write_all(&0x06064b50u32.to_le_bytes())?;
            dest.write_all(&44u64.to_le_bytes())?;
            dest.write_all(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
            for n in [count, count, length, offset] {
                dest.write_all(&n.to_le_bytes())?;
            }
            dest.write_all(&0x07064b50u32.to_le_bytes())?;
            dest.write_all(&[0; 4])?;
            dest.write_all(&end.to_le_bytes())?;
            dest.write_all(&1u32.to_le_bytes())?;
        }
        dest.write_all(&0x06054b50u32.to_le_bytes())?;
        dest.write_all(&[0; 4])?;
        for _ in 0..2 {
            dest.write_all(&(count.min(u16::MAX.into()) as u16).to_le_bytes())?;
        }
        dest.write_all(&(length.min(u32::MAX.into()) as u32).to_le_bytes())?;
        dest.write_all(&(offset.min(u32::MAX.into()) as u32).to_le_bytes())?;
        dest.write_all(&[0; 2])?;
        Ok(true)
    })
    .map_err(Into::into)
}

/// Read through the open archive's descriptor. Another save can rename the
/// path while we work. Positional reads cannot disturb another member reader.
struct Range {
    source: File,
    offset: u64,
    remaining: u64,
}
impl Read for Range {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let cap = self.remaining.min(buf.len() as u64) as usize;
        let n = self.source.read_at(&mut buf[..cap], self.offset)?;
        self.offset = self
            .offset
            .checked_add(n as u64)
            .ok_or(io::Error::other("invalid ZIP offset"))?;
        self.remaining -= n as u64;
        Ok(n)
    }
}
#[pyfunction]
pub fn igz_read_member<'py>(
    py: Python<'py>,
    fd: i32,
    offset: u64,
    packed: u64,
    size: usize,
    method: u16,
    crc: u32,
) -> PyResult<Option<Bound<'py, pyo3::types::PyBytes>>> {
    let range = Range {
        source: file(fd)?,
        offset,
        remaining: packed,
    };
    let mut source: Box<dyn Read + Send> = match method {
        0 if packed == size as u64 => Box::new(range),
        8 => Box::new(flate2::read::DeflateDecoder::new(range)),
        _ => return Ok(None),
    };
    let mut valid = false;
    let bytes = pyo3::types::PyBytes::new_with(py, size, |buf| {
        valid = py.detach(|| {
            source.read_exact(buf).is_ok()
                && matches!(source.read(&mut [0]), Ok(0))
                && crc32fast::hash(buf) == crc
        });
        Ok(())
    })?;
    Ok(valid.then_some(bytes))
}

#[pyclass(frozen)]
pub struct DocumentReader {
    consumed: std::sync::atomic::AtomicU64,
    size: u64,
}

struct JsonInput<'a> {
    meter: &'a DocumentReader,
    source: Box<dyn Read + Send>,
    crc: crc32fast::Hasher,
    tail: Vec<u8>,
}

impl Read for JsonInput<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use std::sync::atomic::Ordering::Relaxed;
        let n = self.source.read(buf)?;
        let bytes = &buf[..n];
        let pos = self.meter.consumed.fetch_add(n as u64, Relaxed) + n as u64;
        if pos > self.meter.size {
            return Err(io::Error::other("ZIP member exceeds declared size"));
        }
        // A reserved arbitrary-precision field is decoded by the reference
        // reader. Include partial Unicode escapes across buffer boundaries.
        self.tail.extend_from_slice(&bytes[..bytes.len().min(5)]);
        if bytes.contains(&b'$')
            || bytes
                .windows(6)
                .chain(self.tail.windows(6))
                .any(|w| w == br"\u0024")
        {
            return Err(io::Error::other("reserved JSON key"));
        }
        if n >= 5 {
            self.tail.clear();
            self.tail.extend_from_slice(&bytes[n - 5..]);
        } else if self.tail.len() > 5 {
            self.tail.drain(..self.tail.len() - 5);
        }
        self.crc.update(bytes);
        Ok(n)
    }
}

#[pymethods]
impl DocumentReader {
    #[new]
    fn new(size: u64) -> Self {
        Self {
            size,
            consumed: 0.into(),
        }
    }
    fn fraction(&self) -> f64 {
        (self.consumed.load(std::sync::atomic::Ordering::Relaxed) as f64 / self.size.max(1) as f64)
            .min(1.0)
    }

    /// Parse from the caller's open file, bounded by its checked directory
    /// entry. Returning None retains the application's historical errors,
    /// non-finite-coordinate recovery and unusual JSON spellings.
    fn parse<'py>(
        &self,
        py: Python<'py>,
        fd: i32,
        offset: u64,
        packed: u64,
        method: u16,
        crc: Option<u32>,
        placements: bool,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        let range = Range {
            source: file(fd)?,
            offset,
            remaining: packed,
        };
        let source: Box<dyn Read + Send> = match method {
            0 if packed == self.size => Box::new(range),
            8 => Box::new(flate2::read::DeflateDecoder::new(range)),
            _ => return Ok(None),
        };
        let parsed = py.detach(|| {
            let mut input = JsonInput {
                meter: self,
                source,
                crc: crc32fast::Hasher::new(),
                tail: Vec::with_capacity(10),
            };
            let buf = std::io::BufReader::with_capacity(128 << 10, &mut input);
            use serde::de::DeserializeSeed;
            let fields = if placements {
                crate::file_json::Fields::Document
            } else {
                crate::file_json::Fields::Plain
            };
            let mut parser = serde_json::Deserializer::from_reader(buf);
            let value = fields.deserialize(&mut parser).ok()?;
            parser.end().ok()?;
            (self.consumed.load(std::sync::atomic::Ordering::Relaxed) == self.size
                && crc.is_none_or(|expected| input.crc.finalize() == expected))
            .then_some(value)
        });
        parsed.map(|v| v.to_py(py)).transpose()
    }
}
