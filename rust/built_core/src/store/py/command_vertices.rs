// SPDX-License-Identifier: GPL-3.0-or-later
//! Keep the move, its landing and its texture changes private until the
//! candidate result passes the same local defect check as the drawing command.
use super::*;
use crate::store::{
    command_facade::VertexResult,
    commands::{cell, supplied_cell},
    landing,
};
use std::collections::HashSet;
type P = [f64; 3];

struct AttrChanges<'py> {
    own: Bound<'py, PyDict>,
    table: Bound<'py, PyAny>,
    rows: Bound<'py, PyDict>,
    textures: Bound<'py, PyDict>,
}
impl<'py> AttrChanges<'py> {
    fn new(owner: &Bound<'py, PyDict>) -> PyResult<Self> {
        Ok(Self {
            own: owner.as_any().get_item("_attr_own")?.cast_into()?,
            table: owner.as_any().get_item("_attr_table")?,
            rows: PyDict::new(owner.py()),
            textures: PyDict::new(owner.py()),
        })
    }

    fn face(&self, store: &PyMeshStore, key: u128) -> PyResult<Bound<'py, PyAny>> {
        if let Some(value) = self.rows.get_item(key)?.or(self.own.get_item(key)?) {
            return Ok(value);
        }
        let py = self.own.py();
        let (index, _) = store.face_info(key)?;
        let value = if index >= 0 {
            crate::kernel::store_views::properties::copy_attrs(&self.table.get_item(index)?)?.into_any()
        } else {
            PyDict::new(py).into_any()
        };
        self.rows.set_item(key, &value)?;
        Ok(value)
    }

    fn carry(
        &self,
        store: &PyMeshStore,
        moving: &[u128],
        matrix: [f64; 16],
        hypot: &str,
    ) -> PyResult<()> {
        let py = self.own.py();
        let matrix = glam::DMat4::from_cols_array(&matrix);
        if matrix.determinant() == 0. {
            return Ok(());
        }
        let inverse = matrix.inverse();
        let minv = crate::math::Matrix4::from(inverse).into_pyobject(py)?;
        let mut candidates = Vec::new();
        for slot in store.inner.live_slots(Kind::Face) {
            let key = store.inner.key(Kind::Face, slot);
            let row = if let Some(own) = self.own.get_item(key)? {
                own
            } else {
                let index = *store.inner.state.fattr.get(slot as usize);
                if index == 0 {
                    continue;
                }
                self.table.get_item(index - 1)?
            };
            if row.call_method1("get", ("texture",))?.is_truthy()? {
                candidates.push(key);
            }
        }
        for key in store.carried_faces(candidates, moving.to_vec())? {
            let attrs = self.face(store, key)?;
            let tex = attrs.get_item("texture")?;
            let mut uvw = tex.call_method1("get", ("uvw",))?;
            if !uvw.is_truthy()? || uvw.len()? != 8 {
                let Some(n) =
                    store.pretransform_face_normal(key, inverse.to_cols_array(), hypot)?
                else {
                    continue;
                };
                let (u, cu, v, cv): (Bound<'_, PyAny>, f64, Bound<'_, PyAny>, f64) = py
                    .import("core.texture")?
                    .call_method1(
                        "face_uv_axes",
                        (&tex, crate::math::Vec3::from(glam::DVec3::from_array(n))),
                    )?
                    .extract()?;
                let [a, b, c] = crate::math::xyz(&u)?.to_array();
                let [d, e, f] = crate::math::xyz(&v)?.to_array();
                uvw = vec![a, b, c, cu, d, e, f, cv].into_pyobject(py)?.into_any();
            }
            let mapped = py
                .import("built_core")?
                .call_method1("group_remapped_uvw", (&uvw, &minv))?;
            let copy = py.get_type::<PyDict>().call1((&tex,))?;
            copy.set_item("uvw", mapped)?;
            self.textures.set_item(key, copy)?;
        }
        Ok(())
    }

    fn copy_to(&self, store: &PyMeshStore, source: u128, targets: &[u128]) -> PyResult<()> {
        let py = self.own.py();
        let attrs = self.face(store, source)?;
        let texture = self.textures.get_item(source)?;
        for &key in targets {
            let copied = py.get_type::<PyDict>().call1((&attrs,))?;
            if let Some(tex) = &texture {
                copied.set_item("texture", tex)?;
            }
            self.rows.set_item(key, copied)?;
        }
        Ok(())
    }

    fn publish(self) -> PyResult<()> {
        self.own.update(self.rows.as_mapping())?;
        for (key, tex) in self.textures.iter() {
            self.own.as_any().get_item(key)?.set_item("texture", tex)?;
        }
        Ok(())
    }
}

impl PyMeshStore {
    pub(super) fn vertex_command(
        &mut self,
        py: Python<'_>,
        source: Vec<P>,
        landing: Vec<P>,
        delta: Option<P>,
        matrix: [f64; 16],
        carry: bool,
        hypot: &str,
        owner: &Bound<'_, PyDict>,
    ) -> PyResult<Option<VertexResult>> {
        let mut work = Self::wrap(self.inner.clone());
        let paint = AttrChanges::new(owner)?;
        let mut landed = Vec::new();
        let mut folded = Vec::new();
        // This scan compares the actual post-rounding positions. If it succeeds,
        // no receiver exists and the general weld scan would repeat its work.
        let free_move = delta
            .filter(|_| !carry)
            .and_then(|d| work.inner.move_unjoined_slots(&source, d));
        let may_join = delta.is_some() && free_move.is_none();
        let (mut dirty, moved, before) = if let Some(slots) = free_move {
            let keys = slots
                .iter()
                .map(|&v| work.inner.key(Kind::Vertex, v))
                .collect();
            (slots.len(), keys, None)
        } else if let Some(delta) = delta {
            let Some(start) =
                work.begin_vertex_move(py, source, landing.clone(), delta, true, hypot)?
            else {
                return Ok(None);
            };
            start
        } else {
            let (n, moved) = work.inner.transform_vertices(&source, matrix, true);
            (n, moved, None)
        };
        if carry && !moved.is_empty() {
            paint.carry(&work, &moved, matrix, hypot)?;
        }
        if delta.is_some() {
            let ((joined, bumps, trims), targets) = if may_join {
                let moving: HashSet<_> = moved
                    .iter()
                    .filter_map(|&k| work.inner.slot(Kind::Vertex, k))
                    .collect();
                let moved_cells: HashSet<_> = moving
                    .iter()
                    .map(|&v| cell(*work.inner.state.vpos.get(v as usize)))
                    .collect();
                let targets: HashSet<_> = work
                    .inner
                    .live_slots(Kind::Vertex)
                    .into_iter()
                    .filter(|v| !moving.contains(v))
                    .map(|v| cell(*work.inner.state.vpos.get(v as usize)))
                    .filter(|p| moved_cells.contains(p))
                    .collect();
                let Some(weld) = work.inner.weld_landing(&moving).map_err(op_err)? else {
                    return Ok(None);
                };
                (weld, targets)
            } else {
                ((false, 0, Vec::new()), HashSet::new())
            };
            let Some((fold_bumps, folds)) =
                work.fold_nonplanar(py, landing.clone(), 0.001, hypot)?
            else {
                return Ok(None);
            };
            dirty += bumps + fold_bumps;
            if joined {
                let all = work.inner.live_slots(Kind::Vertex);
                landed = all
                    .iter()
                    .copied()
                    .filter(|&v| targets.contains(&cell(*work.inner.state.vpos.get(v as usize))))
                    .map(|v| work.inner.key(Kind::Vertex, v))
                    .collect();
                if let Some(before) = before {
                    let cells: HashSet<_> = landing.into_iter().map(supplied_cell).collect();
                    let near = all
                        .into_iter()
                        .filter(|&v| cells.contains(&cell(*work.inner.state.vpos.get(v as usize))))
                        .map(|v| work.inner.key(Kind::Vertex, v))
                        .collect();
                    let counts = work.landing_defects(py, near, 1e-10, hypot)?;
                    let defects: Vec<_> = landing::increased(before, counts)
                        .map(str::to_owned)
                        .collect();
                    if !defects.is_empty() {
                        return Ok(Some((vec![], vec![], vec![], defects, 0)));
                    }
                }
            }
            for (source, target) in trims {
                paint.copy_to(&work, source, &[target])?;
            }
            for (source, targets) in folds {
                paint.copy_to(&work, source, &targets)?;
                folded.push(source);
            }
        }
        let dirty = u32::try_from(dirty).map_err(|_| {
            pyo3::exceptions::PyOverflowError::new_err("vertex edit exceeds dirty count")
        })?;
        paint.publish()?;
        self.inner = work.inner;
        Ok(Some((moved, landed, folded, vec![], dirty)))
    }
}
