// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use measures::{measurements, sum, Totals};
pub(super) struct Affine {
    linear: DMat3,
    translation: DVec3,
}
pub(super) fn affine(matrix: &Obj) -> PyResult<Affine> {
    let m = crate::math::mat(matrix)?;
    Ok(Affine {
        linear: DMat3::from_mat4(m),
        translation: m.w_axis.truncate(),
    })
}
pub(super) fn transformed<'p>(p: &Obj<'p>, a: &Affine, double: bool) -> PyResult<Obj<'p>> {
    let mut points = row_values(p)?;
    p.py().detach(|| {
        for v in &mut points {
            let q = a.linear * DVec3::from(*v) + a.translation;
            *v = if double { q } else { q.as_vec3().as_dvec3() }.to_array();
        }
    });
    rows(p.py(), points)
}
fn uniform(a: &Bundle, xf: &Affine, data: &Obj) -> PyResult<Option<f64>> {
    let rot = xf.linear;
    let gram = rot.transpose() * rot;
    let s = (gram.x_axis.x + gram.y_axis.y + gram.z_axis.z) / 3.;
    if s <= 0.
        || (gram - DMat3::IDENTITY * s)
            .to_cols_array()
            .iter()
            .any(|v| v.abs() > 2e-6 * s)
    {
        return Ok(None);
    }
    if !data.is_none() {
        let b = a.borrow();
        let Some((lo, hi)) = b._bounds else {
            return Ok(None);
        };
        let bounds = DVec3::from(lo).abs().max(DVec3::from(hi).abs());
        let absolute = DMat3::from_cols(rot.x_axis.abs(), rot.y_axis.abs(), rot.z_axis.abs());
        let error = (absolute * bounds + xf.translation.abs()).max_element() * f32::EPSILON as f64;
        let area = sum(data, 0)? * s;
        if error * b._perimeter * s.sqrt() > 1e-7f64.max(2e-5 * area) {
            return Ok(None);
        }
        if data.get_item(4)?.is_truthy()?
            && error * area
                > 1e-7f64.max(2e-5 * data.get_item(3)?.extract::<f64>()? * rot.determinant().abs())
        {
            return Ok(None);
        }
    }
    Ok(Some(s))
}
#[pyfunction]
pub(super) fn placed_arrays<'p>(local: &Bundle<'p>, matrix: Option<&Obj>) -> PyResult<Bundle<'p>> {
    let Some(m) = matrix else {
        return Ok(local.clone());
    };
    let xf = affine(m)?;
    let a = local.borrow();
    let py = local.py();
    let dst = packed(
        py,
        transformed(a.pos.bind(py), &xf, a.model_f64)?,
        a.corners.bind(py).clone(),
        a.ring_sizes.bind(py).clone(),
        a.ring_counts.bind(py).clone(),
        a.edge_face_counts.as_ref().map(|e| e.bind(py).clone()),
        a.model_f64,
    )?;
    drop(a);
    let data = measurements(local)?;
    if let Some(s) = uniform(local, &xf, &data)? {
        if !data.is_none() {
            let det = xf.linear.determinant();
            let normals = row_values(&data.get_item(2)?)?
                .into_iter()
                .map(|n| (xf.linear * DVec3::from(n) * (det.signum() / s.sqrt())).to_array())
                .collect();
            let volume = data
                .get_item(3)?
                .extract::<Option<f64>>()?
                .map(|v| v * det.abs());
            let closed = data.get_item(4)?.is_truthy()?;
            let d = (
                data.get_item(0)?.mul(s)?,
                data.get_item(1)?.mul(s)?,
                rows(py, normals)?,
                volume,
                closed,
                data.get_item(5)?,
            )
                .into_bound_py_any(py)?;
            let src = local.borrow();
            let mut out = dst.borrow_mut();
            out._measurements = Some(d.unbind());
            out._perimeter = src._perimeter * s.sqrt();
            out._signed_volume = src._signed_volume * det;
            out._balanced = src._balanced;
            // Keep conservative world bounds for a subsequent transformed query.
            out._bounds = src._bounds.map(|(l, h)| {
                let (l, h) = (DVec3::from(l), DVec3::from(h));
                let c = xf.linear * ((h + l) * 0.5) + xf.translation;
                let extent = (h - l) * 0.5;
                let span = xf.linear.x_axis.abs() * extent.x
                    + xf.linear.y_axis.abs() * extent.y
                    + xf.linear.z_axis.abs() * extent.z;
                ((c - span).to_array(), (c + span).to_array())
            });
        }
    }
    Ok(dst)
}
pub(super) fn join<'p>(py: Python<'p>, parts: &[Bundle<'p>], weld: bool) -> PyResult<Bundle<'p>> {
    if parts.is_empty() {
        return faces_arrays(PyList::empty(py).as_any());
    }
    let np = py.import("numpy")?;
    let mut columns: [Vec<Obj>; 5] = std::array::from_fn(|_| Vec::new());
    let mut offset = 0;
    let mut precise = false;
    for p in parts {
        let a = p.borrow();
        let pos = a.pos.bind(py);
        columns[0].push(pos.clone());
        columns[1].push(a.corners.bind(py).add(offset)?);
        offset += pos.len()?;
        columns[2].push(a.ring_sizes.bind(py).clone());
        columns[3].push(a.ring_counts.bind(py).clone());
        precise |= a.model_f64;
        if !weld {
            columns[4].push(
                a.edge_face_counts
                    .as_ref()
                    .ok_or_else(|| PyValueError::new_err("missing edge incidences"))?
                    .bind(py)
                    .clone(),
            );
        }
    }
    let concat = |xs| np.call_method1("concatenate", (xs,));
    let [p, c, s, n, e] = columns;
    let (p, c) = (concat(p)?, concat(c)?);
    let (p, c) = if weld {
        let kw = PyDict::new(py);
        kw.set_item("precise", precise)?;
        let (p, inv): (Obj, Obj) = py
            .import("core.lazy_mesh")?
            .call_method("weld", (p,), Some(&kw))?
            .extract()?;
        (p, inv.get_item(c)?)
    } else {
        (p, c)
    };
    packed(
        py,
        p,
        c,
        concat(s)?,
        concat(n)?,
        if weld { None } else { Some(concat(e)?) },
        precise,
    )
}
fn parts<'p>(a: &Obj<'p>) -> PyResult<Vec<Bundle<'p>>> {
    a.try_iter()?
        .map(|v| Ok(v?.cast_into::<Arrays>()?))
        .collect()
}
#[pyfunction]
pub(super) fn merged<'p>(arrays: &Obj<'p>) -> PyResult<Bundle<'p>> {
    join(arrays.py(), &parts(arrays)?, true)
}
#[pyfunction]
pub(super) fn assembled<'p>(arrays: &Obj<'p>) -> PyResult<Bundle<'p>> {
    let parts = parts(arrays)?;
    if let [a] = &parts[..] {
        return Ok(a.clone());
    }
    for a in &parts {
        let es = a.getattr("edge_face_counts")?;
        if es.is_none() || indices(&es)?.into_iter().any(|v| v != 0 && v != 2) {
            return join(arrays.py(), &parts, true);
        }
    }
    join(arrays.py(), &parts, false)
}
// Yield on demand: asking for the first placement must not open a sibling's mesh.
#[pyclass]
pub(super) struct Owned {
    stack: Vec<(Py<PyAny>, Py<PyAny>)>,
    pending: Option<(Py<PyAny>, Py<PyAny>)>,
    excluded: Option<Py<PyAny>>,
}
#[pymethods]
impl Owned {
    fn __iter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }
    fn __next__<'p>(&mut self, py: Python<'p>) -> PyResult<Option<(Obj<'p>, Obj<'p>)>> {
        if let Some((g, world)) = self.pending.take() {
            let children = g.bind(py).getattr("children")?;
            if !children.is_none() {
                for i in (0..children.len()?).rev() {
                    let ch = children.get_item(i)?;
                    if ch.getattr("ifc")?.is_truthy()? {
                        continue;
                    }
                    if let Some(ex) = &self.excluded {
                        if ex.bind(py).contains(ch.getattr("uid")?)? {
                            continue;
                        }
                    }
                    self.stack.push((ch.unbind(), world.clone_ref(py)));
                }
            }
        }
        let Some((g, base)) = self.stack.pop() else {
            return Ok(None);
        };
        let g = g.into_bound(py);
        let base = base.into_bound(py);
        let own = g.getattr_opt("xform")?.unwrap_or_else(|| none(py));
        let world = if own.is_none() {
            base
        } else if base.is_none() {
            own
        } else {
            base.mul(own)?
        };
        self.pending = Some((g.clone().unbind(), world.clone().unbind()));
        Ok(Some((g, world)))
    }
}
#[pyfunction(signature=(group,base=None,excluded=None))]
pub(super) fn owned_placements(group: &Obj, base: Option<&Obj>, excluded: Option<&Obj>) -> Owned {
    Owned {
        stack: vec![(
            group.clone().unbind(),
            base.cloned().unwrap_or_else(|| none(group.py())).unbind(),
        )],
        pending: None,
        excluded: excluded.map(|x| x.clone().unbind()),
    }
}
#[pyclass]
pub(super) struct WorldArrays {
    it: Py<PyAny>,
}
#[pymethods]
impl WorldArrays {
    fn __iter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }
    fn __next__<'p>(&self, py: Python<'p>) -> PyResult<Option<Bundle<'p>>> {
        for row in self.it.bind(py).try_iter()? {
            let (g, m): (Obj, Obj) = row?.extract()?;
            let a = arrays_of(&g.getattr("mesh")?)?;
            if a.getattr("ring_counts")?.len()? > 0 {
                return Ok(Some(placed_arrays(&a, (!m.is_none()).then_some(&m))?));
            }
        }
        Ok(None)
    }
}
#[pyfunction(signature=(group,base=None,excluded=None,placements=None))]
pub(super) fn world_arrays(
    group: &Obj,
    base: Option<&Obj>,
    excluded: Option<&Obj>,
    placements: Option<&Obj>,
) -> PyResult<WorldArrays> {
    let py = group.py();
    let it = match placements {
        Some(x) => x.try_iter()?.into_any(),
        None => Bound::new(py, owned_placements(group, base, excluded))?.into_any(),
    };
    Ok(WorldArrays { it: it.unbind() })
}
#[pyfunction]
pub(super) fn placement_quantities(placements: &Obj) -> PyResult<Totals> {
    let py = placements.py();
    let mut work = vec![];
    for r in placements.try_iter()? {
        let (g, m): (Obj, Obj) = r?.extract()?;
        let a = arrays_of(&g.getattr("mesh")?)?;
        let d = measurements(&a)?;
        if !d.is_none() {
            work.push((a, m, d));
        }
    }
    if work.is_empty() {
        return Ok((0., 0., None, false, 0));
    }
    let fresh = || -> PyResult<Totals> {
        let ps = work
            .iter()
            .map(|(a, m, _)| placed_arrays(a, (!m.is_none()).then_some(m)))
            .collect::<PyResult<Vec<_>>>()?;
        measures::quantities(PyList::new(py, ps)?.as_any())
    };
    let (mut gross, mut net, mut vol, mut count, mut closed) = (0., 0., 0., 0, true);
    for (a, m, d) in &work {
        let is_closed = d.get_item(4)?.is_truthy()?;
        if work.len() > 1 && (!is_closed || !a.borrow()._balanced) {
            return fresh();
        }
        let (s, det) = if m.is_none() {
            (1., 1.)
        } else {
            let xf = affine(m)?;
            let Some(s) = uniform(a, &xf, d)? else {
                return fresh();
            };
            (s, xf.linear.determinant())
        };
        gross += sum(d, 0)? * s;
        net += sum(d, 1)? * s;
        vol += a.borrow()._signed_volume * det;
        count += d.get_item(0)?.len()?;
        closed &= is_closed;
    }
    Ok((gross, net, closed.then_some(vol.abs()), closed, count))
}
