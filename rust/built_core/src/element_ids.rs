// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent element ids (docs/persistent-ids.md §3).
//!
//! An element id is `origin : counter`. The origin is 64 random bits drawn
//! once per process the first time it mints (0 is the migration origin:
//! ids derived from file order), the counter counts from 0 and never
//! rewinds. In memory and in the file an id is packed into 64 bits,
//! `origin index << 40 | counter`, against an origin table. The table in
//! memory is this process's; a file carries its own, remapped on load and
//! on save. Only the logical text form (`f:<origin hex>:<counter hex>`)
//! ever leaves the process.
//!
//! Record uids (definitions, materials, scenes, …) are 64 random bits as
//! 16 lowercase hex, the form placement uids have.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub const COUNTER_BITS: u32 = 40;
pub const COUNTER_MASK: u64 = (1 << COUNTER_BITS) - 1;
pub const MAX_ORIGINS: usize = 1 << (64 - COUNTER_BITS);

/// The process origin table: index → 64-bit origin. Index 0 is the
/// migration origin 0.
static ORIGINS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
/// This process's origin index, once it minted.
static SESSION: OnceLock<u32> = OnceLock::new();
/// One counter per kind (vertex, edge, face): ids are unique per kind, and
/// a mesh drawn element by element gets consecutive ids per kind, which
/// stay implicit (`store/ids.rs`).
static COUNTERS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

#[inline]
pub fn pack(origin_index: u32, counter: u64) -> u64 {
    ((origin_index as u64) << COUNTER_BITS) | (counter & COUNTER_MASK)
}

#[inline]
pub fn origin_index(id: u64) -> u32 {
    (id >> COUNTER_BITS) as u32
}

#[inline]
pub fn counter(id: u64) -> u64 {
    id & COUNTER_MASK
}

/// `BUILT_IDS_SEED` (an integer) makes every draw below deterministic:
/// recorded test goldens only.
static SEEDED: OnceLock<Option<AtomicU64>> = OnceLock::new();

pub fn random_u64() -> u64 {
    if let Some(state) = SEEDED.get_or_init(|| {
        std::env::var("BUILT_IDS_SEED").ok().and_then(|v| v.parse::<u64>().ok()).map(AtomicU64::new)
    }) {
        // SplitMix64.
        let mut z = state.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed).wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        return z ^ (z >> 31);
    }
    let mut b = [0u8; 8];
    if getrandom::fill(&mut b).is_err() {
        // No entropy source: time and address bits still differ per run.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        return t ^ (&b as *const _ as u64).rotate_left(29) ^ (std::process::id() as u64) << 48;
    }
    u64::from_le_bytes(b)
}

fn table() -> std::sync::MutexGuard<'static, Vec<u64>> {
    let mut t = ORIGINS.lock().unwrap_or_else(|e| e.into_inner());
    if t.is_empty() {
        t.push(0);
    }
    t
}

/// The process index of a logical origin, added when new.
pub fn intern(origin: u64) -> Result<u32, String> {
    let mut t = table();
    if let Some(i) = t.iter().position(|&o| o == origin) {
        return Ok(i as u32);
    }
    if t.len() >= MAX_ORIGINS {
        return Err("too many id origins".into());
    }
    t.push(origin);
    Ok((t.len() - 1) as u32)
}

/// The logical origin of a process index.
pub fn origin_of(index: u32) -> Option<u64> {
    table().get(index as usize).copied()
}

/// This process's origin index (drawn on first use).
pub fn session_index() -> u32 {
    *SESSION.get_or_init(|| loop {
        let o = random_u64();
        if o != 0 {
            if let Ok(i) = intern(o) {
                break i;
            }
        }
    })
}

/// `n` consecutive fresh ids of kind `kind` (0 vertex, 1 edge, 2 face);
/// the first one. Never handed out again in this process (the counters are
/// outside every snapshot).
pub fn mint_range(kind: usize, n: u64) -> u64 {
    let s = session_index();
    let c = COUNTERS[kind.min(2)].fetch_add(n.max(1), Ordering::Relaxed);
    assert!(c + n <= COUNTER_MASK, "element id counter exhausted for this session");
    pack(s, c)
}

#[inline]
pub fn mint(kind: usize) -> u64 {
    mint_range(kind, 1)
}

/// Remap packed ids from a file's origin table into the process table:
/// the index map, `None` when it is the identity.
pub fn file_to_process(file_origins: &[u64]) -> Result<Option<Vec<u32>>, String> {
    let mut map = Vec::with_capacity(file_origins.len());
    for (i, &o) in file_origins.iter().enumerate() {
        if i == 0 && o != 0 {
            return Err("origin 0 of an id table must be the migration origin".into());
        }
        map.push(intern(o)?);
    }
    Ok(if map.iter().enumerate().all(|(i, &m)| i as u32 == m) { None } else { Some(map) })
}

pub fn remap(id: u64, map: &[u32]) -> Result<u64, String> {
    let i = origin_index(id) as usize;
    map.get(i)
        .map(|&m| pack(m, counter(id)))
        .ok_or_else(|| format!("element id names origin {i}, which the file does not list"))
}

// ---------------------------------------------------------------- text form

pub fn kind_letter(kind: &str) -> Option<char> {
    match kind {
        "vertex" | "v" => Some('v'),
        "edge" | "e" => Some('e'),
        "face" | "f" => Some('f'),
        _ => None,
    }
}

pub fn kind_name(letter: char) -> Option<&'static str> {
    match letter {
        'v' => Some("vertex"),
        'e' => Some("edge"),
        'f' => Some("face"),
        _ => None,
    }
}

/// `f:<origin hex>:<counter hex>` of a packed id.
pub fn to_text(kind: char, id: u64) -> String {
    let origin = origin_of(origin_index(id)).unwrap_or(u64::MAX);
    format!("{kind}:{origin:x}:{:x}", counter(id))
}

/// `(kind letter, packed id)` of a text id; the origin is interned.
pub fn from_text(text: &str) -> Result<(char, u64), String> {
    let bad = || format!("not an element id: {text:?}");
    let mut parts = text.split(':');
    let (Some(k), Some(o), Some(c), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(bad());
    };
    let kind = k.chars().next().filter(|_| k.len() == 1).and_then(|c| kind_name(c).map(|_| c)).ok_or_else(bad)?;
    let origin = u64::from_str_radix(o, 16).map_err(|_| bad())?;
    let count = u64::from_str_radix(c, 16).map_err(|_| bad())?;
    if count > COUNTER_MASK || o.is_empty() || c.is_empty() {
        return Err(bad());
    }
    Ok((kind, pack(intern(origin)?, count)))
}

/// Whether a string looks like an element id (`f:…:…`), not a key.
pub fn is_text_id(text: &str) -> bool {
    text.len() > 4 && text.as_bytes()[1] == b':' && matches!(text.as_bytes()[0], b'v' | b'e' | b'f')
}

// ---------------------------------------------------------------- record uids

/// A fresh 16-hex record uid.
pub fn new_uid() -> String {
    format!("{:016x}", random_u64())
}

/// A fresh 32-hex document id.
pub fn new_doc_id() -> String {
    format!("{:016x}{:016x}", random_u64(), random_u64())
}

/// 16 hex of SHA-256 over a name: the deterministic uids of migration and
/// of linked definitions (docs/persistent-ids.md §4, §5.3).
pub fn derived_uid(name: &str) -> String {
    let h = Sha256::digest(name.as_bytes());
    h[..8].iter().map(|b| format!("{b:02x}")).collect()
}

pub const ROOT_DEFINITION: &str = "0000000000000000";

// ---------------------------------------------------------------- definitions

/// Definition uids by geometry object (weak): a definition is the mesh a
/// placement shows, so a new mesh object (a copy, Make Unique) is a new
/// definition and the object that comes back on undo keeps its uid.
static DEFINITIONS: pyo3::sync::PyOnceLock<Py<PyAny>> = pyo3::sync::PyOnceLock::new();

fn definitions(py: Python<'_>) -> PyResult<&Bound<'_, PyAny>> {
    Ok(DEFINITIONS
        .get_or_try_init(py, || -> PyResult<_> {
            Ok(py.import("weakref")?.call_method0("WeakKeyDictionary")?.unbind())
        })?
        .bind(py))
}

fn owner<'p>(mesh: &Bound<'p, PyAny>) -> PyResult<Bound<'p, PyAny>> {
    Ok(mesh
        .getattr_opt("_instance_definition")?
        .filter(|o| !o.is_none())
        .unwrap_or_else(|| mesh.clone()))
}

/// A fresh 16-hex record uid. Its own random source, apart from the
/// placement uid pool: drawing ids never shifts what else a session draws.
pub fn fresh_uid(_py: Python<'_>) -> PyResult<String> {
    Ok(new_uid())
}

/// A fresh 32-hex document id.
pub fn fresh_doc_id(_py: Python<'_>) -> PyResult<String> {
    Ok(new_doc_id())
}

/// The persistent uid of the definition a mesh is (drawn on first use).
#[pyfunction]
pub fn definition_uid(mesh: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = mesh.py();
    let reg = definitions(py)?;
    let m = owner(mesh)?;
    if let Some(u) = reg.call_method1("get", (&m,))?.extract::<Option<String>>()? {
        return Ok(u);
    }
    let u = fresh_uid(py)?;
    reg.set_item(&m, &u)?;
    Ok(u)
}

/// Name a mesh's definition (a file's `uid`, a derived one).
#[pyfunction]
pub fn set_definition_uid(mesh: &Bound<'_, PyAny>, uid: &str) -> PyResult<()> {
    let py = mesh.py();
    definitions(py)?.set_item(owner(mesh)?, uid)
}

/// The uid if the mesh has one already (no drawing).
#[pyfunction]
pub fn known_definition_uid(mesh: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    let py = mesh.py();
    definitions(py)?.call_method1("get", (owner(mesh)?,))?.extract()
}

/// A material's persistent uid, drawn when it has none.
#[pyfunction]
pub fn material_uid(material: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Some(u) = material.getattr_opt("uid")?.and_then(|v| v.extract::<Option<String>>().ok()).flatten() {
        return Ok(u);
    }
    // Named after its name, as a file without uids names it on load (the
    // archive keeps uids unique if a renamed material left this one behind).
    let name = material.getattr("name")?.str()?.to_string();
    let u = derived_uid(&format!("built:legacy:v1:material:{name}"));
    material.setattr("uid", &u)?;
    Ok(u)
}

// ---------------------------------------------------------------- Python

#[pyfunction]
#[pyo3(name = "new_uid")]
fn py_new_uid() -> String {
    new_uid()
}

#[pyfunction]
#[pyo3(name = "new_doc_id")]
fn py_new_doc_id() -> String {
    new_doc_id()
}

#[pyfunction]
#[pyo3(name = "derived_uid")]
fn py_derived_uid(name: &str) -> String {
    derived_uid(name)
}

/// Text form of a packed id: `id_text("face", packed)`.
#[pyfunction]
fn id_text(kind: &str, id: u64) -> PyResult<String> {
    let k = kind_letter(kind).ok_or_else(|| PyValueError::new_err(format!("unknown kind {kind:?}")))?;
    Ok(to_text(k, id))
}

/// `(kind name, packed id)` of a text id.
#[pyfunction]
fn parse_id(text: &str) -> PyResult<(&'static str, u64)> {
    let (k, id) = from_text(text).map_err(PyValueError::new_err)?;
    Ok((kind_name(k).unwrap(), id))
}

#[pyfunction]
#[pyo3(name = "is_text_id")]
fn py_is_text_id(text: &str) -> bool {
    is_text_id(text)
}

/// The process origin table (index → origin), for the writer.
#[pyfunction]
fn origin_table() -> Vec<u64> {
    table().clone()
}

/// Intern a file's origins: the index map, or None for the identity.
#[pyfunction]
#[pyo3(name = "file_to_process")]
fn py_file_to_process(origins: Vec<u64>) -> PyResult<Option<Vec<u32>>> {
    file_to_process(&origins).map_err(PyValueError::new_err)
}

#[pyfunction]
#[pyo3(name = "mint_range", signature = (n, kind = "face"))]
fn py_mint_range(n: u64, kind: &str) -> PyResult<u64> {
    let k = ["vertex", "edge", "face"].iter().position(|x| *x == kind).ok_or_else(|| PyValueError::new_err(format!("unknown kind {kind:?}")))?;
    Ok(mint_range(k, n))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "built_core.element_ids")?;
    sub.add_function(wrap_pyfunction!(py_new_uid, &sub)?)?;
    sub.add_function(wrap_pyfunction!(py_new_doc_id, &sub)?)?;
    sub.add_function(wrap_pyfunction!(py_derived_uid, &sub)?)?;
    sub.add_function(wrap_pyfunction!(id_text, &sub)?)?;
    sub.add_function(wrap_pyfunction!(parse_id, &sub)?)?;
    sub.add_function(wrap_pyfunction!(py_is_text_id, &sub)?)?;
    sub.add_function(wrap_pyfunction!(origin_table, &sub)?)?;
    sub.add_function(wrap_pyfunction!(py_file_to_process, &sub)?)?;
    sub.add_function(wrap_pyfunction!(py_mint_range, &sub)?)?;
    sub.add_function(wrap_pyfunction!(definition_uid, &sub)?)?;
    sub.add_function(wrap_pyfunction!(material_uid, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_definition_uid, &sub)?)?;
    sub.add_function(wrap_pyfunction!(known_definition_uid, &sub)?)?;
    sub.add("ROOT_DEFINITION", ROOT_DEFINITION)?;
    sub.add("COUNTER_BITS", COUNTER_BITS)?;
    py.import("sys")?.getattr("modules")?.set_item("built_core.element_ids", &sub)?;
    m.add("element_ids", sub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip_and_migration_origin() {
        let id = pack(0, 0x4a2);
        assert_eq!(to_text('f', id), "f:0:4a2");
        assert_eq!(from_text("f:0:4a2").unwrap(), ('f', id));
        let m = mint(1);
        let t = to_text('e', m);
        assert_eq!(from_text(&t).unwrap(), ('e', m));
        assert!(from_text("x:0:1").is_err());
        assert!(from_text("f:0").is_err());
        assert!(is_text_id("v:0:0") && !is_text_id("12345"));
    }

    #[test]
    fn mints_are_unique_and_remapped_tables_keep_logical_ids() {
        let a = mint_range(2, 10);
        let b = mint(2);
        assert!(counter(b) >= counter(a) + 10);
        let o = random_u64() | 1;
        let map = file_to_process(&[0, o]).unwrap();
        let idx = intern(o).unwrap();
        match map {
            None => assert_eq!(idx, 1),
            Some(m) => assert_eq!(m[1], idx),
        }
        let file_id = pack(1, 7);
        let p = remap(file_id, &[0, idx]).unwrap();
        assert_eq!(origin_of(origin_index(p)), Some(o));
        assert!(file_to_process(&[5]).is_err());
    }

    #[test]
    fn derived_uids_are_stable_hex() {
        let a = derived_uid("built:legacy:v1:def:3");
        assert_eq!(a.len(), 16);
        assert_eq!(a, derived_uid("built:legacy:v1:def:3"));
        assert_ne!(a, derived_uid("built:legacy:v1:def:4"));
        assert_eq!(new_uid().len(), 16);
        assert_eq!(new_doc_id().len(), 32);
    }
}
