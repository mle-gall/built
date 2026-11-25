//! The resident scene's rows (architecture review 12): which placement key
//! each GPU row draws, which rows each prototype has, which keys the last
//! diff saw, and every row's world box and the scene's bounds.
//!
//! views/wgpu_updates.py kept these as Python dicts and sets keyed by uid
//! strings (about 500 bytes a row) and recomputed boxes, members and
//! command starts in NumPy per edit. A row keeps its index for life
//! (removed rows are tombstones), so a selection, a GPU pick id and a key
//! stay stable across edits.
//!
//! The object is also the view's ``uid_rows`` mapping (key → rows), which
//! highlighting, picking and the tools read.
use numpy::{IntoPyArray, PyArray1, PyReadonlyArray1};
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use std::collections::{BTreeSet, HashMap};

const INSTANCE: usize = 80;
const BOUNDS: usize = 32;

type Box3 = [f32; 6];
const EMPTY: Box3 = [f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];

const NONE: u32 = u32::MAX;

/// Placement keys, each stored once: rows, the mapping and the live set
/// refer to them by index.
#[derive(Default)]
struct Keys {
    names: Vec<Box<str>>,
    /// hash → key index; keys whose hash is taken go to ``collided``.
    by_hash: HashMap<u64, u32>,
    collided: HashMap<Box<str>, u32>,
}

fn hash(key: &str) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault};
    BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default().hash_one(key)
}

impl Keys {
    fn find(&self, key: &str) -> Option<u32> {
        match self.by_hash.get(&hash(key)) {
            Some(&i) if &*self.names[i as usize] == key => Some(i),
            _ => self.collided.get(key).copied(),
        }
    }

    fn intern(&mut self, key: &str) -> u32 {
        if let Some(i) = self.find(key) {
            return i;
        }
        let i = self.names.len() as u32;
        self.names.push(key.into());
        match self.by_hash.entry(hash(key)) {
            std::collections::hash_map::Entry::Vacant(v) => { v.insert(i); }
            std::collections::hash_map::Entry::Occupied(_) => { self.collided.insert(key.into(), i); }
        }
        i
    }

    fn bytes(&self) -> usize {
        self.names.iter().map(|n| n.len().next_multiple_of(16) + 16).sum::<usize>()
            + self.by_hash.capacity() * 13 + self.collided.len() * 48
    }
}

#[pyclass(mapping, module = "built_render")]
pub struct ResidentRows {
    keys: Keys,
    /// row → its key (``packed.uids``).
    row_key: Vec<u32>,
    /// row → prototype code.
    codes: Vec<u32>,
    /// key → its first row in the mapping (``uid_rows``), NONE when absent;
    /// further rows of one key in ``extra``.
    mapped: Vec<u32>,
    extra: HashMap<u32, Vec<u32>>,
    /// key → the row it was last given, NONE when none.
    last: Vec<u32>,
    /// key → whether the last diff saw it (a live row).
    active: Vec<bool>,
    /// While a full diff runs: the keys it has seen.
    seen: Option<Vec<bool>>,
    /// code → its rows.
    members: HashMap<u32, BTreeSet<u32>>,
    /// row → world box (lo xyz, hi xyz); hidden rows are empty.
    boxes: Vec<Box3>,
}

fn f32_at(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
}

/// The world box of one instance row (``world`` row-major, translation in
/// its last row) over its prototype's bounds; hidden rows (info[2] bit 0)
/// are empty.
fn row_box(row: &[u8], bounds: &[u8]) -> Box3 {
    let info2 = u32_at(row, 18);
    let code = u32_at(row, 16) as usize;
    if info2 & 1 != 0 || (code + 1) * BOUNDS > bounds.len() {
        return EMPTY;
    }
    let b = &bounds[code * BOUNDS..(code + 1) * BOUNDS];
    let (lo, hi) = ([f32_at(b, 0), f32_at(b, 1), f32_at(b, 2)], [f32_at(b, 4), f32_at(b, 5), f32_at(b, 6)]);
    let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    let extent = [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5, (hi[2] - lo[2]) * 0.5];
    let w = |r: usize, c: usize| f32_at(row, r * 4 + c);
    let mut out = [0f32; 6];
    for i in 0..3 {
        let c = (0..3).map(|j| w(j, i) * center[j]).sum::<f32>() + w(3, i);
        let e = (0..3).map(|j| w(j, i).abs() * extent[j]).sum::<f32>();
        out[i] = c - e;
        out[i + 3] = c + e;
    }
    out
}

impl ResidentRows {
    fn empty() -> Self {
        ResidentRows { keys: Keys::default(), row_key: vec![], codes: vec![], mapped: vec![], extra: HashMap::new(),
            last: vec![], active: vec![], seen: None, members: HashMap::new(), boxes: vec![] }
    }

    /// The key's index, made when new (with room in every key column).
    fn key_id(&mut self, key: &str) -> u32 {
        let i = self.keys.intern(key);
        if i as usize == self.mapped.len() {
            self.mapped.push(NONE);
            self.last.push(NONE);
            self.active.push(false);
            if let Some(seen) = &mut self.seen {
                seen.push(false);
            }
        }
        i
    }

    fn rows_of(&self, key: &str) -> Option<Vec<u32>> {
        let i = self.keys.find(key)?;
        let first = self.mapped[i as usize];
        if first == NONE {
            return None;
        }
        let mut out = vec![first];
        if let Some(more) = self.extra.get(&i) {
            out.extend(more);
        }
        Some(out)
    }

    fn map(&mut self, key: &str, rows: Vec<u32>) {
        let i = self.key_id(key);
        self.extra.remove(&i);
        match rows.split_first() {
            None => {
                // An empty list is still an entry: keep it as no rows.
                self.mapped[i as usize] = NONE;
                self.extra.insert(i, vec![]);
            }
            Some((&first, rest)) => {
                self.mapped[i as usize] = first;
                if !rest.is_empty() {
                    self.extra.insert(i, rest.to_vec());
                }
            }
        }
    }

    fn unmap(&mut self, key: &str) -> Option<Vec<u32>> {
        let i = self.keys.find(key)?;
        let empty = self.mapped[i as usize] == NONE && self.extra.get(&i).is_some_and(Vec::is_empty);
        let rows = if empty { Some(vec![]) } else { self.rows_of(key) }?;
        self.mapped[i as usize] = NONE;
        self.extra.remove(&i);
        Some(rows)
    }

    fn set_code(&mut self, row: u32, code: u32) -> bool {
        let r = row as usize;
        while self.codes.len() <= r {
            self.codes.push(NONE);
            self.boxes.push(EMPTY);
        }
        let old = self.codes[r];
        if old == code {
            return false;
        }
        if let Some(m) = self.members.get_mut(&old) {
            m.remove(&row);
        }
        self.members.entry(code).or_default().insert(row);
        self.codes[r] = code;
        true
    }
}

#[pymethods]
impl ResidentRows {
    /// From a pack: each row's key, the instance rows (80 bytes each) and
    /// the prototype bounds (32 bytes each).
    #[new]
    fn new(keys: Vec<String>, instances: PyReadonlyArray1<'_, u8>, bounds: PyReadonlyArray1<'_, u8>) -> PyResult<Self> {
        let raw = instances.as_slice()?;
        let bounds = bounds.as_slice()?;
        if raw.len() != keys.len() * INSTANCE {
            return Err(PyValueError::new_err("one key per instance row"));
        }
        let mut out = Self::empty();
        out.row_key.reserve(keys.len());
        out.codes.reserve(keys.len());
        out.boxes.reserve(keys.len());
        for (row, (key, inst)) in keys.iter().zip(raw.chunks_exact(INSTANCE)).enumerate() {
            let row = row as u32;
            let i = out.key_id(key);
            let k = i as usize;
            if out.mapped[k] == NONE {
                out.mapped[k] = row;
            } else {
                out.extra.entry(i).or_default().push(row);
            }
            out.last[k] = row;
            out.active[k] = true;
            out.row_key.push(i);
            let code = u32_at(inst, 16);
            out.codes.push(code);
            out.members.entry(code).or_default().insert(row);
            out.boxes.push(row_box(inst, bounds));
        }
        Ok(out)
    }

    // ---- the view's uid_rows mapping ---------------------------------------------------
    fn __len__(&self) -> usize {
        (0..self.mapped.len() as u32).filter(|i| self.mapped[*i as usize] != NONE || self.extra.contains_key(i)).count()
    }

    fn __contains__(&self, key: &str) -> bool {
        self.keys.find(key).is_some_and(|i| self.mapped[i as usize] != NONE || self.extra.contains_key(&i))
    }

    fn __getitem__(&self, key: &str) -> PyResult<Vec<u32>> {
        if !self.__contains__(key) {
            return Err(PyKeyError::new_err(key.to_owned()));
        }
        Ok(self.rows_of(key).unwrap_or_default())
    }

    fn __setitem__(&mut self, key: &str, rows: Vec<u32>) {
        self.map(key, rows);
    }

    fn __delitem__(&mut self, key: &str) -> PyResult<()> {
        if !self.__contains__(key) {
            return Err(PyKeyError::new_err(key.to_owned()));
        }
        self.unmap(key);
        Ok(())
    }

    /// ``uid_rows.get(key, default)``.
    #[pyo3(signature = (key, default=None))]
    fn get<'py>(&self, py: Python<'py>, key: &str, default: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
        if self.__contains__(key) {
            return Ok(self.rows_of(key).unwrap_or_default().into_pyobject(py)?.into_any());
        }
        Ok(default.unwrap_or_else(|| py.None().into_bound(py)))
    }

    #[pyo3(signature = (key, default=None))]
    fn pop<'py>(&mut self, py: Python<'py>, key: &str, default: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
        if self.__contains__(key) {
            return Ok(self.unmap(key).unwrap_or_default().into_pyobject(py)?.into_any());
        }
        default.ok_or_else(|| PyKeyError::new_err(key.to_owned()))
    }

    fn keys(&self) -> Vec<String> {
        self.keys.names.iter().map(|n| n.to_string()).filter(|n| self.__contains__(n)).collect()
    }

    // ---- rows and keys -----------------------------------------------------------------
    /// How many rows (tombstones included).
    #[getter]
    fn count(&self) -> usize {
        self.codes.len()
    }

    /// The row ``key`` was last given, or None.
    fn row(&self, key: &str) -> Option<u32> {
        self.keys.find(key).map(|i| self.last[i as usize]).filter(|&r| r != NONE)
    }

    /// The key row ``row`` draws.
    fn key(&self, row: usize) -> PyResult<String> {
        self.row_key.get(row).map(|&i| self.keys.names[i as usize].to_string())
            .ok_or_else(|| PyValueError::new_err("no such row"))
    }

    /// A new row for ``key`` (the next index).
    fn add(&mut self, key: &str, row: u32) -> PyResult<()> {
        if row as usize != self.row_key.len() {
            return Err(PyValueError::new_err("rows are added in order"));
        }
        let i = self.key_id(key);
        self.row_key.push(i);
        self.last[i as usize] = row;
        Ok(())
    }

    // ---- which keys a diff saw ---------------------------------------------------------
    fn is_active(&self, key: &str) -> bool {
        self.keys.find(key).is_some_and(|i| self.active[i as usize])
    }

    #[getter]
    fn active_count(&self) -> usize {
        self.active.iter().filter(|a| **a).count()
    }

    /// Start a diff: a local one adds to the live keys, a full one starts
    /// from none.
    fn begin(&mut self, local: bool) {
        self.seen = if local { None } else { Some(vec![false; self.active.len()]) };
    }

    fn see(&mut self, key: &str) {
        let i = self.key_id(key) as usize;
        match &mut self.seen {
            Some(seen) => seen[i] = true,
            None => self.active[i] = true,
        }
    }

    fn unsee(&mut self, key: &str) {
        let Some(i) = self.keys.find(key) else { return };
        match &mut self.seen {
            Some(seen) => seen[i as usize] = false,
            None => self.active[i as usize] = false,
        }
    }

    fn has_seen(&self, key: &str) -> bool {
        self.keys.find(key).is_some_and(|i| match &self.seen {
            Some(seen) => seen[i as usize],
            None => self.active[i as usize],
        })
    }

    /// End a diff: the seen keys are the live ones. A full diff returns
    /// the rows of the keys it no longer saw.
    fn finish(&mut self) -> Vec<u32> {
        let Some(seen) = self.seen.take() else { return vec![] };
        let gone = (0..seen.len()).filter(|&i| self.active[i] && !seen[i] && self.last[i] != NONE)
            .map(|i| self.last[i]).collect();
        self.active = seen;
        gone
    }

    // ---- prototypes, boxes, bounds -----------------------------------------------------
    /// The rows ``ids`` now draw ``codes``: True when a row changed
    /// prototype or a row is new.
    fn set_codes(&mut self, ids: Vec<u32>, codes: Vec<u32>) -> PyResult<bool> {
        if ids.len() != codes.len() {
            return Err(PyValueError::new_err("one code per row"));
        }
        let mut changed = false;
        for (row, code) in ids.into_iter().zip(codes) {
            changed |= self.set_code(row, code);
        }
        Ok(changed)
    }

    /// The rows of prototype ``code``, in order.
    fn members<'py>(&self, py: Python<'py>, code: u32) -> Bound<'py, PyArray1<i64>> {
        let rows: Vec<i64> = self.members.get(&code).map_or_else(Vec::new, |m| m.iter().map(|&r| r as i64).collect());
        rows.into_pyarray(py)
    }

    /// Each prototype's first instance index (the indirect commands'
    /// ``start``): its rows counted, in code order.
    fn starts<'py>(&self, py: Python<'py>, prototypes: usize) -> Bound<'py, PyArray1<i64>> {
        let mut out = vec![0i64; prototypes];
        let mut at = 0i64;
        for (code, start) in out.iter_mut().enumerate() {
            *start = at;
            at += self.members.get(&(code as u32)).map_or(0, |m| m.len() as i64);
        }
        out.into_pyarray(py)
    }

    /// Recompute the boxes of ``touched`` rows from the instance rows and
    /// prototype bounds; the scene's bounds ``(lo, hi)`` after it, shrunk
    /// only when a touched row held one of its faces.
    fn refresh_boxes(&mut self, touched: Vec<u32>, instances: PyReadonlyArray1<'_, u8>,
                     bounds: PyReadonlyArray1<'_, u8>, lo: [f32; 3], hi: [f32; 3]) -> PyResult<Option<([f32; 3], [f32; 3])>> {
        let raw = instances.as_slice()?;
        let bounds = bounds.as_slice()?;
        let rows = raw.len() / INSTANCE;
        while self.boxes.len() < rows {
            self.boxes.push(EMPTY);
            self.codes.push(u32::MAX);
        }
        let mut reduce = false;
        let (mut nlo, mut nhi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for &row in &touched {
            let r = row as usize;
            if r >= rows {
                return Err(PyValueError::new_err("row out of range"));
            }
            let old = self.boxes[r];
            let new = row_box(&raw[r * INSTANCE..(r + 1) * INSTANCE], bounds);
            for i in 0..3 {
                reduce |= old[i] == lo[i] && new[i] > old[i];
                reduce |= old[i + 3] == hi[i] && new[i + 3] < old[i + 3];
                nlo[i] = nlo[i].min(new[i]);
                nhi[i] = nhi[i].max(new[i + 3]);
            }
            self.boxes[r] = new;
        }
        if touched.is_empty() {
            return Ok(Some((lo, hi)));
        }
        if reduce {
            (nlo, nhi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
            for b in &self.boxes {
                for i in 0..3 {
                    nlo[i] = nlo[i].min(b[i]);
                    nhi[i] = nhi[i].max(b[i + 3]);
                }
            }
        } else {
            for i in 0..3 {
                nlo[i] = nlo[i].min(lo[i]);
                nhi[i] = nhi[i].max(hi[i]);
            }
        }
        Ok(nlo.iter().chain(&nhi).all(|v| v.is_finite()).then_some((nlo, nhi)))
    }

    /// The box of ``row`` (lo xyz, hi xyz).
    fn world_box(&self, row: usize) -> PyResult<Box3> {
        self.boxes.get(row).copied().ok_or_else(|| PyValueError::new_err("no such row"))
    }

    /// Bytes held (tests, the benchmark).
    fn held_bytes(&self) -> usize {
        self.keys.bytes() + (self.row_key.capacity() + self.codes.capacity() + self.mapped.capacity()
            + self.last.capacity()) * 4 + self.active.capacity() + self.boxes.capacity() * 24
            + self.extra.values().map(|v| v.capacity() * 4 + 40).sum::<usize>()
            + self.members.values().map(|m| m.len() * 12 + 48).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(code: u32, x: f32, hidden: bool) -> Vec<u8> {
        let mut w = [0f32; 16];
        w[0] = 1.0; w[5] = 1.0; w[10] = 1.0; w[15] = 1.0;
        w[12] = x;
        let mut out: Vec<u8> = w.iter().flat_map(|f| f.to_le_bytes()).collect();
        for v in [code, 0, u32::from(hidden), 0] {
            out.extend(v.to_le_bytes());
        }
        out
    }

    fn bounds() -> Vec<u8> {
        [0f32, 0., 0., 0., 1., 2., 3., 0.].iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    #[test]
    fn a_row_box_follows_its_matrix_and_hidden_rows_are_empty() {
        let b = row_box(&instance(0, 5.0, false), &bounds());
        assert_eq!(b, [5., 0., 0., 6., 2., 3.]);
        assert_eq!(row_box(&instance(0, 5.0, true), &bounds()), EMPTY);
    }

    #[test]
    fn membership_and_starts() {
        let mut r = ResidentRows::empty();
        for (row, key) in ["a", "b"].iter().enumerate() {
            let i = r.key_id(key);
            r.row_key.push(i);
            r.last[i as usize] = row as u32;
            r.active[i as usize] = true;
            r.set_code(row as u32, row as u32);
        }
        assert!(!r.set_code(0, 0));
        assert!(r.set_code(0, 1));
        assert!(r.set_code(2, 0));
        assert_eq!(r.members[&1].iter().copied().collect::<Vec<_>>(), [0, 1]);
        assert_eq!(r.codes, [1, 1, 0]);
        r.begin(false);
        r.see("b");
        assert_eq!(r.finish(), [0]);
        assert!(!r.is_active("a") && r.is_active("b"));
    }

    #[test]
    fn the_mapping_keeps_lists_and_keys_once() {
        let mut r = ResidentRows::empty();
        r.map("a", vec![3, 4]);
        r.map("b", vec![]);
        assert_eq!(r.rows_of("a"), Some(vec![3, 4]));
        assert!(r.__contains__("b") && !r.__contains__("c"));
        assert_eq!(r.unmap("a"), Some(vec![3, 4]));
        assert!(!r.__contains__("a"));
        assert_eq!(r.keys.intern("a"), 0);
        assert_eq!(r.keys.names.len(), 2);
    }
}
