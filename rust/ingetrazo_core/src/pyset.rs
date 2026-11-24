// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! CPython's `set` of non-negative ints, slot for slot (`Objects/setobject.c`,
//! 3.13): for native passes whose result depends on the order a Python set
//! of element keys iterates in (the plane rebuild picks each plane's origin
//! from the first fresh face on it). A view of a store element hashes and
//! compares as its key, so a set of views has this table.
//!
//! `tests/test_pyset_order.py` replays random operation sequences on real
//! sets and on this one (`ingetrazo_core.py_set_replay`).
use pyo3::prelude::*;

const LINEAR_PROBES: usize = 9;
const PERTURB_SHIFT: u32 = 5;
const MINSIZE: usize = 8;
/// `sys.hash_info.modulus`.
const MODULUS: u128 = (1 << 61) - 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Empty,
    Dummy,
    Key(u128, usize),
}

/// `hash(k)` for an int `k >= 0`, as the `size_t` the table uses.
pub fn hash(k: u128) -> usize {
    (k % MODULUS) as usize
}

#[derive(Clone, Debug)]
pub struct PySet {
    table: Vec<Slot>,
    fill: usize,
    used: usize,
}

impl Default for PySet {
    fn default() -> Self {
        Self::new()
    }
}

impl PySet {
    pub fn new() -> Self {
        Self { table: vec![Slot::Empty; MINSIZE], fill: 0, used: 0 }
    }

    fn mask(&self) -> usize {
        self.table.len() - 1
    }

    pub fn len(&self) -> usize {
        self.used
    }

    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// `set(keys)` from a list or any iterator that is not a set.
    pub fn from_keys(keys: impl IntoIterator<Item = u128>) -> Self {
        let mut s = Self::new();
        for k in keys {
            s.add(k);
        }
        s
    }

    /// Iteration order: the table's slots.
    pub fn iter(&self) -> impl Iterator<Item = u128> + '_ {
        self.table.iter().filter_map(|s| match *s {
            Slot::Key(k, _) => Some(k),
            _ => None,
        })
    }

    fn entries(&self) -> impl Iterator<Item = (u128, usize)> + '_ {
        self.table.iter().filter_map(|s| match *s {
            Slot::Key(k, h) => Some((k, h)),
            _ => None,
        })
    }

    /// `set_lookkey`: the slot holding `k`, or None.
    fn find(&self, k: u128, h: usize) -> Option<usize> {
        let mask = self.mask();
        let mut i = h & mask;
        let mut perturb = h;
        loop {
            let probes = if i + LINEAR_PROBES <= mask { LINEAR_PROBES } else { 0 };
            for j in i..=i + probes {
                match self.table[j] {
                    Slot::Empty => return None,
                    Slot::Key(key, hh) if hh == h && key == k => return Some(j),
                    _ => {}
                }
            }
            perturb >>= PERTURB_SHIFT;
            i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb)) & mask;
        }
    }

    pub fn contains(&self, k: u128) -> bool {
        self.find(k, hash(k)).is_some()
    }

    /// `set_add_entry`.
    pub fn add(&mut self, k: u128) {
        self.add_entry(k, hash(k));
    }

    fn add_entry(&mut self, k: u128, h: usize) {
        let mask = self.mask();
        let mut i = h & mask;
        let mut perturb = h;
        let mut freeslot = None;
        loop {
            let probes = if i + LINEAR_PROBES <= mask { LINEAR_PROBES } else { 0 };
            for j in i..=i + probes {
                match self.table[j] {
                    Slot::Empty => {
                        if let Some(free) = freeslot {
                            self.used += 1;
                            self.table[free] = Slot::Key(k, h);
                            return;
                        }
                        self.fill += 1;
                        self.used += 1;
                        self.table[j] = Slot::Key(k, h);
                        if self.fill * 5 < mask * 3 {
                            return;
                        }
                        let n = if self.used > 50000 { self.used * 2 } else { self.used * 4 };
                        self.resize(n);
                        return;
                    }
                    Slot::Key(key, hh) if hh == h && key == k => return,
                    // The LAST dummy on the way is the one reused (3.13:
                    // `freeslot = entry`, no first-only test).
                    Slot::Dummy => freeslot = Some(j),
                    Slot::Key(..) => {}
                }
            }
            perturb >>= PERTURB_SHIFT;
            i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb)) & mask;
        }
    }

    /// `set_insert_clean`: into a table with no dummies and no `k`.
    fn insert_clean(table: &mut [Slot], k: u128, h: usize) {
        let mask = table.len() - 1;
        let mut i = h & mask;
        let mut perturb = h;
        loop {
            if table[i] == Slot::Empty {
                table[i] = Slot::Key(k, h);
                return;
            }
            if i + LINEAR_PROBES <= mask {
                for j in i + 1..=i + LINEAR_PROBES {
                    if table[j] == Slot::Empty {
                        table[j] = Slot::Key(k, h);
                        return;
                    }
                }
            }
            perturb >>= PERTURB_SHIFT;
            i = (i.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb)) & mask;
        }
    }

    /// `set_table_resize`.
    fn resize(&mut self, minused: usize) {
        let mut size = MINSIZE;
        while size <= minused {
            size <<= 1;
        }
        if size == MINSIZE && self.table.len() == MINSIZE && self.fill == self.used {
            // The small table stays: nothing to purge.
            return;
        }
        let old = std::mem::replace(&mut self.table, vec![Slot::Empty; size]);
        self.fill = self.used;
        for s in old {
            if let Slot::Key(k, h) = s {
                Self::insert_clean(&mut self.table, k, h);
            }
        }
    }

    /// `set_discard_entry`.
    pub fn discard(&mut self, k: u128) -> bool {
        match self.find(k, hash(k)) {
            Some(j) => {
                self.table[j] = Slot::Dummy;
                self.used -= 1;
                true
            }
            None => false,
        }
    }

    /// `s.update(other)` / `set(other)` for a set `other` (`set_merge`).
    pub fn merge(&mut self, other: &PySet) {
        if other.used == 0 {
            return;
        }
        if (self.fill + other.used) * 5 >= self.mask() * 3 {
            self.resize((self.used + other.used) * 2);
        }
        if self.fill == 0 && self.mask() == other.mask() && other.fill == other.used {
            self.table.clone_from(&other.table);
            self.fill = other.fill;
            self.used = other.used;
            return;
        }
        if self.fill == 0 {
            self.fill = other.used;
            self.used = other.used;
            for (k, h) in other.entries() {
                Self::insert_clean(&mut self.table, k, h);
            }
            return;
        }
        for (k, h) in other.entries() {
            self.add_entry(k, h);
        }
    }

    /// `set(self)`.
    pub fn copy(&self) -> PySet {
        let mut s = PySet::new();
        s.merge(self);
        s
    }

    /// `self - other` with `other` a set (`set_difference`): `other` is
    /// asked only whether it holds a key.
    pub fn difference(&self, other_len: usize, holds: impl Fn(u128) -> bool) -> PySet {
        if (self.used >> 2) > other_len {
            // set_copy_and_difference: a copy, the shared keys discarded
            // (dummies), the dummies resized away when they are more than
            // a quarter of the table (set_difference_update_internal).
            let mut s = self.copy();
            let keys: Vec<u128> = s.iter().filter(|&k| holds(k)).collect();
            for k in keys {
                s.discard(k);
            }
            if s.fill - s.used > s.mask() / 4 {
                let n = if s.used > 50000 { s.used * 2 } else { s.used * 4 };
                s.resize(n);
            }
            return s;
        }
        let mut s = PySet::new();
        for (k, h) in self.entries() {
            if !holds(k) {
                s.add_entry(k, h);
            }
        }
        s
    }
}

/// Test face: replay `ops` and return the iteration order after each.
/// Ops: `("add", k)`, `("discard", k)`, `("new", [keys])` (a fresh set from
/// a list, made current), `("copy", 0)` (`set(current)`), `("update", i)`
/// (current `|=` saved set i), `("save", 0)`, `("minus", i)`
/// (`current - saved[i]`).
#[pyfunction]
pub fn py_set_replay(ops: Vec<(String, Vec<u128>)>) -> PyResult<Vec<Vec<u128>>> {
    let mut cur = PySet::new();
    let mut saved: Vec<PySet> = Vec::new();
    let mut out = Vec::new();
    for (op, arg) in ops {
        match op.as_str() {
            "add" => cur.add(arg[0]),
            "discard" => {
                cur.discard(arg[0]);
            }
            "new" => cur = PySet::from_keys(arg),
            "copy" => cur = cur.copy(),
            "save" => saved.push(cur.copy()),
            "update" => {
                let other = saved[arg[0] as usize].clone();
                cur.merge(&other);
            }
            "minus" => {
                let other = &saved[arg[0] as usize];
                cur = cur.difference(other.len(), |k| other.contains(k));
            }
            "table" => {
                // The table's size, fill and used, in place of the order.
                out.push(vec![cur.table.len() as u128, cur.fill as u128, cur.used as u128]);
                continue;
            }
            _ => return Err(pyo3::exceptions::PyValueError::new_err(op)),
        }
        out.push(cur.iter().collect());
    }
    Ok(out)
}
