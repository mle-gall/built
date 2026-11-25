// SPDX-License-Identifier: GPL-3.0-or-later
//! The keyed AddFace command works on copy-on-write columns until its topology
//! and paint are representable. A refusal leaves the original allocator intact.
use super::*;
use crate::store::command_facade::FaceResult;

impl PyMeshStore {
    pub(super) fn divide_face_slots(
        &mut self,
        f: u32,
        holding: &[u32],
        inside: &[u32],
        hypot: &str,
    ) -> PyResult<Option<(usize, Option<u128>, Vec<u128>, bool)>> {
        use crate::store::drawing_face::Division;
        let plan = self
            .inner
            .face_division(f, holding, inside, crate::hypot_of(hypot)?);
        let mut dirty = 0;
        let mut remainder = Vec::new();
        let mut swapped = false;
        let paint_source = match plan {
            None => return Ok(None),
            Some(Division::Holes {
                mother,
                swap,
                enclosed,
            }) => {
                if let Some(m) = mother {
                    let ring = if let Some((i, new)) = swap {
                        let old = &self.inner.face_rings(m)[i + 1];
                        self.op(|s, ch| {
                            s.remove_hole(m, old, ch);
                            Ok(())
                        })?;
                        swapped = true;
                        new
                    } else {
                        self.inner
                            .outer_ring(f)
                            .iter()
                            .map(|&v| *self.inner.state.vpos.get(v as usize))
                            .collect()
                    };
                    dirty += ring.len();
                    self.op(|s, ch| s.add_hole(m, &ring, ch))?;
                }
                for ring in enclosed {
                    dirty += ring.len();
                    self.op(|s, ch| s.add_hole(f, &ring, ch))?;
                }
                mother.map(|m| self.inner.key(Kind::Face, m))
            }
            Some(Division::Carve { mother, pieces }) => {
                let key = self.inner.key(Kind::Face, mother);
                self.remove_face(key)?;
                dirty += 1;
                for mut rings in pieces {
                    if rings.len() > 2 {
                        let keep = Self::maximal_holes(rings[1..].to_vec(), hypot)?;
                        let mut mask = std::iter::once(true).chain(keep);
                        rings.retain(|_| mask.next() == Some(true));
                    }
                    dirty += 1 + rings.iter().map(Vec::len).sum::<usize>();
                    let (new, _) = self.op(|s, ch| s.add_face(&rings, ch))?;
                    remainder.push(self.inner.key(Kind::Face, new));
                }
                Some(key)
            }
        };
        Ok(Some((dirty, paint_source, remainder, swapped)))
    }

    pub(in crate::store) fn add_face_in_frame(
        &mut self,
        py: Python<'_>,
        rings: Vec<Vec<[f64; 3]>>,
        attrs: Option<&Bound<'_, PyDict>>,
        interior: bool,
        divide: bool,
        scan: bool,
        hypot: &str,
        owner: &Bound<'_, PyDict>,
    ) -> PyResult<Option<FaceResult>> {
        let mut dirty = 1 + rings.iter().map(Vec::len).sum::<usize>();
        // A plain insertion can publish directly once every possible allocation
        // and quantised corner has been checked. Near the slot limit use the
        // staged path, which also handles generation retirement after undo.
        if !divide && attrs.is_none_or(|a| a.is_empty()) {
            let corners = dirty - 1;
            let fits = [corners, corners, 1].iter().enumerate().all(|(k, n)| {
                self.inner.hwm[k].len.max(self.inner.state.slots[k].len())
                    .saturating_add(*n) <= super::super::keys::MAX_SLOTS as usize
            });
            if fits && rings.iter().flatten().all(|&p| self.inner.quantize(p).into_iter().all(f64::is_finite)) {
                let dirty = u32::try_from(dirty).map_err(|_| {
                    pyo3::exceptions::PyOverflowError::new_err("face edit exceeds dirty count")
                })?;
                let (slot, _) = self.op(|s, ch| s.add_face(&rings, ch))?;
                self.inner.set_face_flags(slot, u8::from(interior));
                return Ok(Some((self.inner.key(Kind::Face, slot), dirty, None, Vec::new(), false)));
            }
        }
        let mut work = Self::wrap(self.inner.clone());
        let (slot, _) = work.op(|s, ch| s.add_face(&rings, ch))?;
        let face = work.inner.key(Kind::Face, slot);
        work.inner.set_face_flags(slot, u8::from(interior));
        let paint = PyDict::new(py);
        if let Some(attrs) = attrs {
            paint.update(attrs.as_mapping())?;
        }
        let mut mother = None;
        let mut remainder = Vec::new();
        let mut swaps = false;
        if divide {
            let (holding, inside) = if scan {
                crate::scans::MeshScan::from_state(work.inner.state.clone(), 0)
                    .drawn_face_candidates(py, slot)?
            } else {
                let all = work.inner.live_slots(Kind::Face);
                (all.clone(), all)
            };
            let Some((extra, source, pieces, exchanged)) =
                work.divide_face_slots(slot, &holding, &inside, hypot)?
            else {
                return Ok(None);
            };
            dirty += extra;
            mother = source;
            remainder = pieces;
            swaps = exchanged;
        }
        let dirty = u32::try_from(dirty).map_err(|_| {
            pyo3::exceptions::PyOverflowError::new_err("face edit exceeds dirty count")
        })?;
        let own = owner
            .as_any()
            .get_item("_attr_own")?
            .cast_into::<PyDict>()?;
        let updates = PyDict::new(py);
        if let Some(key) = mother {
            let src = match own.get_item(key)? {
                Some(value) => value,
                None => {
                    let (row, _) = work.face_info(key)?;
                    let value = if row < 0 {
                        PyDict::new(py).into_any()
                    } else {
                        let entry = owner.as_any().get_item("_attr_table")?.get_item(row)?;
                        crate::kernel::store_views::properties::copy_attrs(&entry)?.into_any()
                    };
                    updates.set_item(key, &value)?;
                    value
                }
            };
            for key in &remainder {
                updates.set_item(key, py.get_type::<PyDict>().call1((&src,))?)?;
            }
            for name in py.import("ingetrazo_core.mesh")?.getattr("PAINT_KEYS")?.try_iter()? {
                let name = name?;
                if src.contains(&name)? && !paint.contains(&name)? {
                    let v = src.get_item(&name)?;
                    let v = if v.is_instance_of::<PyDict>() {
                        py.get_type::<PyDict>().call1((v,))?
                    } else {
                        v
                    };
                    paint.set_item(name, v)?;
                }
            }
        }
        if !paint.is_empty() || attrs.is_some_and(|a| !a.is_empty()) {
            updates.set_item(face, paint)?;
        }
        own.update(updates.as_mapping())?;
        self.inner = work.inner;
        Ok(Some((face, dirty, mother, remainder, swaps)))
    }
}
