// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Reading the `geom/` members of a format-3 `.igz` (`formats/igz.py`).
//!
//! A large document holds thousands of small raw members (11,604 on the
//! ride model). Python's `zipfile` inflates them one by one on the main
//! thread, then `_geometry_arrays` scans each block's index arrays with a
//! handful of NumPy reductions — a fixed cost per call that adds up over
//! thousands of meshes. Here the members are read in one pass over the
//! file, inflated and CRC-checked in parallel off the GIL, and each block
//! is checked against the layout its JSON declares, all in one call.
//!
//! Nothing here decides what a damaged file does. A member this code cannot
//! read exactly as `zipfile` would (another method, encryption, a CRC or
//! size mismatch, a local header that disagrees) comes back as `None`, and a
//! block it cannot vouch for comes back unchecked: Python then runs its own
//! reader and its own checks, and raises its own errors. So the fast path
//! only ever says "Python would accept this, and see these bytes".

use pyo3::exceptions::PyValueError;
use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};
use rayon::prelude::*;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// One `geom/` member, as the central directory describes it (Python's
/// `ZipInfo`): where its local header is, how it is stored, its sizes and
/// CRC, its name as stored in the local header, and its flag bits.
type Entry = (u64, u32, u64, u64, u32, Vec<u8>, u32);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dtype {
    F4,
    I4,
    I8,
    U1,
}

impl Dtype {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "<f4" => Some(Dtype::F4),
            "<i4" => Some(Dtype::I4),
            "<i8" => Some(Dtype::I8),
            "|u1" => Some(Dtype::U1),
            _ => None,
        }
    }

    fn size(self) -> usize {
        match self {
            Dtype::F4 | Dtype::I4 => 4,
            Dtype::I8 => 8,
            Dtype::U1 => 1,
        }
    }
}

/// One array of a block's layout: `[dtype, shape, offset]`.
struct Spec {
    name: String,
    dtype: Dtype,
    shape: Vec<usize>,
    offset: usize,
}

impl Spec {
    fn count(&self) -> Option<usize> {
        self.shape.iter().try_fold(1usize, |acc, &n| acc.checked_mul(n))
    }

    fn len(&self) -> usize {
        self.shape.first().copied().unwrap_or(0)
    }
}

/// What a block must satisfy: its layout and the sizes of the two tables
/// its JSON side carries (face attributes, edge tag names).
struct Check {
    specs: Vec<Spec>,
    n_attrs: i64,
    n_layers: i64,
}

/// `[dtype, shape, offset]` from the JSON as Python ints and a str, or
/// `None` for anything else (Python's `int(...)` would coerce more: it is
/// left to decide those).
fn parse_spec(name: String, spec: &Bound<'_, PyAny>) -> Option<Spec> {
    let items: Vec<Bound<'_, PyAny>> = if let Ok(l) = spec.cast::<PyList>() {
        l.iter().collect()
    } else if let Ok(t) = spec.cast::<PyTuple>() {
        t.iter().collect()
    } else {
        return None;
    };
    if items.len() != 3 {
        return None;
    }
    let dtype = Dtype::parse(&items[0].extract::<String>().ok()?)?;
    let shape_list = items[1].cast::<PyList>().ok()?;
    let mut shape = Vec::with_capacity(shape_list.len());
    for n in shape_list.iter() {
        let n: i64 = n.extract().ok()?;
        shape.push(usize::try_from(n).ok()?);
    }
    let offset: i64 = items[2].extract().ok()?;
    let offset = usize::try_from(offset).ok()?;
    Some(Spec { name, dtype, shape, offset })
}

fn parse_check(layout: &Bound<'_, PyAny>, n_attrs: i64, n_layers: i64) -> Option<Check> {
    let layout = layout.cast::<PyDict>().ok()?;
    let mut specs = Vec::with_capacity(layout.len());
    for (k, v) in layout.iter() {
        let name: String = k.extract().ok()?;
        specs.push(parse_spec(name, &v)?);
    }
    Some(Check { specs, n_attrs, n_layers })
}

/// Integer values of a 1-D (or any) index array, as i64.
fn ints<'a>(blob: &'a [u8], s: &Spec, count: usize) -> Option<Box<dyn Iterator<Item = i64> + 'a>> {
    let w = s.dtype.size();
    let bytes = &blob[s.offset..s.offset + count * w];
    match s.dtype {
        Dtype::I4 => Some(Box::new(
            bytes.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as i64),
        )),
        Dtype::I8 => Some(Box::new(bytes.chunks_exact(8).map(|c| {
            i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
        }))),
        _ => None,
    }
}

/// (min, max, sum) of an integer array; `None` when it is not integer.
fn stats(blob: &[u8], s: &Spec) -> Option<(i64, i64, i128, usize)> {
    let count = s.count()?;
    let mut lo = i64::MAX;
    let mut hi = i64::MIN;
    let mut sum: i128 = 0;
    for v in ints(blob, s, count)? {
        lo = lo.min(v);
        hi = hi.max(v);
        sum += v as i128;
    }
    Some((lo, hi, sum, count))
}

/// Whether `formats.igz._geometry_arrays` accepts `blob` under `check`:
/// the same tests, in a form that only ever answers yes where Python does.
/// Shapes the writer never produces (index arrays of other dtypes or
/// dimensions) answer no, and Python looks at them itself.
fn block_ok(blob: &[u8], check: &Check) -> bool {
    let mut by_name = std::collections::HashMap::with_capacity(check.specs.len());
    for s in &check.specs {
        let Some(count) = s.count() else { return false };
        let Some(size) = count.checked_mul(s.dtype.size()) else { return false };
        match s.offset.checked_add(size) {
            Some(end) if end <= blob.len() => {}
            _ => return false,
        }
        by_name.insert(s.name.as_str(), s);
    }
    let get = |n: &str| by_name.get(n).copied();
    let (Some(pos), Some(edges), Some(rings), Some(sizes), Some(counts), Some(fa), Some(_flags)) = (
        get("pos"),
        get("edges"),
        get("rings"),
        get("ring_sizes"),
        get("ring_counts"),
        get("face_attr"),
        get("edge_flags"),
    ) else {
        return false;
    };
    if pos.shape.len() != 2 || pos.shape[1] != 3 {
        return false;
    }
    if edges.shape.len() != 2 || edges.shape[1] != 2 {
        return false;
    }
    for s in [rings, sizes, counts, fa] {
        if s.shape.len() != 1 {
            return false;
        }
    }
    let nv = pos.len() as i64;
    let ne = edges.len();
    for s in [edges, rings] {
        let Some((lo, hi, _, n)) = stats(blob, s) else { return false };
        if n > 0 && (lo < 0 || hi >= nv) {
            return false;
        }
    }
    let Some((slo, _, ssum, sn)) = stats(blob, sizes) else { return false };
    let Some((clo, _, csum, cn)) = stats(blob, counts) else { return false };
    if (sn > 0 && slo < 1) || (cn > 0 && clo < 1) {
        return false;
    }
    if ssum != rings.len() as i128 || csum != sizes.len() as i128 {
        return false;
    }
    let Some((flo, fhi, _, fnn)) = stats(blob, fa) else { return false };
    if fa.len() != counts.len() || (fnn > 0 && (flo < -1 || fhi >= check.n_attrs)) {
        return false;
    }
    for name in ["edge_flags", "edge_curve", "edge_layer"] {
        if let Some(s) = get(name) {
            if s.shape.len() != 1 || s.len() != ne {
                return false;
            }
        }
    }
    if let Some(el) = get("edge_layer") {
        let Some((lo, hi, _, n)) = stats(blob, el) else { return false };
        if n > 0 && (lo < -1 || hi >= check.n_layers) {
            return false;
        }
    }
    true
}

/// The stored bytes of one member: its local header is checked the way
/// `zipfile` checks it (signature, the same name), then the data is read.
fn read_raw(file: &mut File, e: &Entry, file_len: u64) -> Option<Vec<u8>> {
    let (offset, _method, csize, _size, _crc, name, flags) = e;
    if flags & 0x1 != 0 {
        return None; // encrypted: zipfile asks for a password
    }
    let mut head = [0u8; 30];
    file.seek(SeekFrom::Start(*offset)).ok()?;
    file.read_exact(&mut head).ok()?;
    if head[0..4] != [0x50, 0x4b, 0x03, 0x04] {
        return None;
    }
    let n = u16::from_le_bytes([head[26], head[27]]) as u64;
    let x = u16::from_le_bytes([head[28], head[29]]) as u64;
    let mut stored_name = vec![0u8; n as usize];
    file.read_exact(&mut stored_name).ok()?;
    if &stored_name != name {
        return None;
    }
    let start = offset.checked_add(30 + n + x)?;
    if start.checked_add(*csize)? > file_len {
        return None;
    }
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut data = vec![0u8; usize::try_from(*csize).ok()?];
    file.read_exact(&mut data).ok()?;
    Some(data)
}

/// Inflate (or copy) `raw` into `out`, which is exactly the member's size;
/// true when it filled `out` exactly and the CRC matches.
fn inflate_into(raw: &[u8], method: u32, crc: u32, out: &mut [u8]) -> bool {
    match method {
        0 => {
            if raw.len() != out.len() {
                return false;
            }
            out.copy_from_slice(raw);
        }
        8 => {
            let mut d = flate2::Decompress::new(false);
            match d.decompress(raw, out, flate2::FlushDecompress::Finish) {
                Ok(flate2::Status::StreamEnd) => {}
                Ok(_) => {
                    // ``out`` is full: the stream must have nothing left.
                    // A member that inflates to more than its size is not
                    // what the directory says; Python decides.
                    let mut probe = [0u8; 1];
                    match d.decompress(&raw[d.total_in() as usize..], &mut probe,
                                       flate2::FlushDecompress::Finish) {
                        Ok(flate2::Status::StreamEnd) if d.total_out() as usize == out.len() => {}
                        _ => return false,
                    }
                }
                Err(_) => return false,
            }
            if d.total_out() as usize != out.len() {
                return false;
            }
        }
        _ => return false,
    }
    crc32fast::hash(out) == crc
}

/// The `geom/` members of the `.igz` at `path`, read and inflated in
/// parallel, each checked against its block's layout when one is given.
///
/// ``entries``: per member, ``(header_offset, compress_type, compress_size,
/// file_size, CRC, local name bytes, flag_bits)`` from Python's `ZipInfo`.
/// ``checks``: per member, ``(layout, n_face_attrs, n_edge_layers)`` of the
/// one block that references it, or ``None``.
///
/// Returns, per member, ``(bytes or None, checked)``: ``None`` where Python
/// must read the member itself, and ``checked`` true only when
/// `_geometry_arrays` would accept the block as it stands.
#[pyfunction]
pub fn igz_read_geometry<'py>(
    py: Python<'py>,
    path: std::path::PathBuf,
    entries: Vec<Entry>,
    checks: Vec<Option<(Bound<'py, PyAny>, i64, i64)>>,
) -> PyResult<Vec<(Option<Bound<'py, PyBytes>>, bool)>> {
    if checks.len() != entries.len() {
        return Err(PyValueError::new_err("entries and checks differ in length"));
    }
    let parsed: Vec<Option<Check>> = checks
        .iter()
        .map(|c| c.as_ref().and_then(|(l, a, n)| parse_check(l, *a, *n)))
        .collect();

    // The output buffers are Python bytes objects, allocated here and
    // filled below without the GIL: no second copy of 200 MB of geometry.
    // Nothing else can see them before this function returns.
    let mut outs: Vec<Option<(Bound<'py, PyBytes>, usize)>> = Vec::with_capacity(entries.len());
    for e in &entries {
        let size = usize::try_from(e.3).ok().filter(|&s| s <= isize::MAX as usize);
        outs.push(match size {
            Some(size) => unsafe {
                let ptr = ffi::PyBytes_FromStringAndSize(std::ptr::null(), size as ffi::Py_ssize_t);
                if ptr.is_null() {
                    return Err(PyErr::fetch(py));
                }
                let b = Bound::from_owned_ptr(py, ptr).cast_into_unchecked::<PyBytes>();
                let data = ffi::PyBytes_AsString(b.as_ptr()) as usize;
                Some((b, data))
            },
            None => None,
        });
    }
    let targets: Vec<Option<(usize, usize)>> = outs
        .iter()
        .zip(&entries)
        .map(|(o, e)| o.as_ref().map(|(_, p)| (*p, e.3 as usize)))
        .collect();

    let results: Vec<(bool, bool)> = py.detach(|| {
        // One pass over the file in offset order (cheap, sequential I/O),
        // then the CPU work spread over the cores.
        let mut raws: Vec<Option<Vec<u8>>> = vec![None; entries.len()];
        if let Ok(mut file) = File::open(&path) {
            let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
            let mut order: Vec<usize> = (0..entries.len()).collect();
            order.sort_by_key(|&i| entries[i].0);
            for i in order {
                if targets[i].is_some() {
                    raws[i] = read_raw(&mut file, &entries[i], file_len);
                }
            }
        }
        raws.into_par_iter()
            .enumerate()
            .map(|(i, raw)| {
                let (Some(raw), Some((ptr, size))) = (raw, targets[i]) else {
                    return (false, false);
                };
                // SAFETY: ``ptr`` is the buffer of a bytes object of exactly
                // ``size`` bytes created above, owned by ``outs`` (alive
                // until we return) and reachable from no other thread or
                // reference; each index is written by one task only.
                let out = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, size) };
                let e = &entries[i];
                if !inflate_into(&raw, e.1, e.4, out) {
                    return (false, false);
                }
                let ok = parsed[i].as_ref().is_some_and(|c| block_ok(out, c));
                (true, ok)
            })
            .collect()
    });

    Ok(outs
        .into_iter()
        .zip(results)
        .map(|(o, (read, ok))| match (o, read) {
            (Some((b, _)), true) => (Some(b), ok),
            _ => (None, false),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, dtype: Dtype, shape: &[usize], offset: usize) -> Spec {
        Spec { name: name.into(), dtype, shape: shape.to_vec(), offset }
    }

    /// A one-triangle block: 3 points, 3 edges, 1 ring of 3.
    fn triangle() -> (Vec<u8>, Check) {
        let mut b = Vec::new();
        for v in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let edges_at = b.len();
        for v in [0i32, 1, 1, 2, 2, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let rings_at = b.len();
        for v in [0i32, 1, 2] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let sizes_at = b.len();
        b.extend_from_slice(&3i32.to_le_bytes());
        let counts_at = b.len();
        b.extend_from_slice(&1i32.to_le_bytes());
        let fa_at = b.len();
        b.extend_from_slice(&(-1i32).to_le_bytes());
        let flags_at = b.len();
        b.extend_from_slice(&[0u8, 0, 1]);
        let specs = vec![
            spec("pos", Dtype::F4, &[3, 3], 0),
            spec("edges", Dtype::I4, &[3, 2], edges_at),
            spec("rings", Dtype::I4, &[3], rings_at),
            spec("ring_sizes", Dtype::I4, &[1], sizes_at),
            spec("ring_counts", Dtype::I4, &[1], counts_at),
            spec("face_attr", Dtype::I4, &[1], fa_at),
            spec("edge_flags", Dtype::U1, &[3], flags_at),
        ];
        (b, Check { specs, n_attrs: 0, n_layers: 0 })
    }

    #[test]
    fn accepts_a_written_block() {
        let (b, c) = triangle();
        assert!(block_ok(&b, &c));
    }

    #[test]
    fn refuses_an_index_out_of_range() {
        let (mut b, c) = triangle();
        let at = c.specs[2].offset;
        b[at..at + 4].copy_from_slice(&3i32.to_le_bytes());
        assert!(!block_ok(&b, &c));
    }

    #[test]
    fn refuses_a_truncated_block_and_a_missing_array() {
        let (b, c) = triangle();
        assert!(!block_ok(&b[..b.len() - 1], &c));
        let (b, mut c) = triangle();
        c.specs.retain(|s| s.name != "edge_flags");
        assert!(!block_ok(&b, &c));
    }

    #[test]
    fn refuses_rings_that_do_not_add_up() {
        let (mut b, c) = triangle();
        let at = c.specs[3].offset;
        b[at..at + 4].copy_from_slice(&2i32.to_le_bytes());
        assert!(!block_ok(&b, &c));
    }

    #[test]
    fn face_attr_must_point_into_the_table() {
        let (mut b, mut c) = triangle();
        let at = c.specs[5].offset;
        b[at..at + 4].copy_from_slice(&0i32.to_le_bytes());
        assert!(!block_ok(&b, &c));
        c.n_attrs = 1;
        assert!(block_ok(&b, &c));
    }

    #[test]
    fn inflates_and_checks_the_crc() {
        use std::io::Write;
        let data: Vec<u8> = (0..10_000u32).flat_map(|v| (v % 251).to_le_bytes()).collect();
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&data).unwrap();
        let raw = enc.finish().unwrap();
        let crc = crc32fast::hash(&data);
        let mut out = vec![0u8; data.len()];
        assert!(inflate_into(&raw, 8, crc, &mut out));
        assert_eq!(out, data);
        assert!(!inflate_into(&raw, 8, crc ^ 1, &mut out));
        let mut short = vec![0u8; data.len() - 1];
        assert!(!inflate_into(&raw, 8, crc, &mut short));
        let mut long = vec![0u8; data.len() + 1];
        assert!(!inflate_into(&raw, 8, crc, &mut long));
        let mut stored = vec![0u8; data.len()];
        assert!(inflate_into(&data, 0, crc, &mut stored));
        assert!(!inflate_into(&raw, 12, crc, &mut stored));
    }
}
