// SPDX-License-Identifier: GPL-3.0-or-later
//! Construct an offset command from the surviving regions of a planar arrangement.
use crate::{
    math::{xyz, Vec3},
    store::{
        geometry::{Hypot, V3},
        offset::{self, Region, Ring},
    },
};
use glam::DVec3;
use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
    types::{PyDict, PyInt, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
fn convention(py: Python<'_>) -> PyResult<Hypot> {
    crate::hypot_of(
        &py.import("core._native")?
            .call_method0("qt_hypot")?
            .extract::<String>()?,
    )
}
fn points(ring: &Obj) -> PyResult<Ring> {
    ring.try_iter()?.map(|v| Ok(xyz(&v?)?.to_array())).collect()
}
fn vec3(v: [f64; 3]) -> Vec3 {
    DVec3::from_array(v).into()
}
#[pyfunction]
fn offset_regions<'p>(r#loop: &Obj<'p>, normal: &Obj<'p>, d: f64) -> PyResult<Bound<'p, PyList>> {
    let py = normal.py();
    let out = PyList::empty(py);
    let ring = points(r#loop)?;
    let n = xyz(normal)?;
    let unit = n.normalize_or_zero();
    if ring.len() < 3 || d.abs() < 1e-9 || unit.length_squared() < 0.5 {
        return Ok(out);
    }
    let h = convention(py)?;
    let segments = offset::outline(&ring, n.to_array(), d, h, true);
    let lines = PyList::new(py, segments.into_iter().map(|(a, b)| (vec3(a), vec3(b))))?;
    let unit = Vec3::from(unit).into_pyobject(py)?;
    let regions = crate::store::planar_api::planar_arrangement(
        lines.as_any(),
        &r#loop.get_item(0)?,
        unit.as_any(),
    )?;
    let regions = PyList::new(py, regions)?;
    let outlines = regions
        .iter()
        .map(|row| points(&row.get_item(0)?))
        .collect::<PyResult<_>>()?;
    let boundary = ring
        .into_iter()
        .map(|[x, y, z]| V3(x, y, z))
        .collect::<Vec<_>>();
    for (region, keep) in regions.iter().zip(offset::keep_outline(
        &boundary,
        outlines,
        n.to_array(),
        d,
        h,
    )) {
        if keep {
            out.append(region)?;
        }
    }
    Ok(out)
}
#[pyfunction]
fn offset_chain(points_: &Obj, normal: &Obj, d: f64) -> PyResult<Option<Vec<Vec3>>> {
    let ring = points(points_)?;
    let segments = offset::outline(
        &ring,
        xyz(normal)?.to_array(),
        d,
        convention(normal.py())?,
        false,
    );
    Ok(segments.first().map(|(first, _)| {
        std::iter::once(vec3(*first))
            .chain(segments.iter().map(|(_, b)| vec3(*b)))
            .collect()
    }))
}
pyo3::create_exception!(offset, OffsetHoleCollision, PyValueError);
#[pyclass]
struct OffsetFace {
    face: Py<PyAny>,
    attrs: Py<PyDict>,
    regions: Vec<Region>,
}
#[pymethods]
impl OffsetFace {
    #[pyo3(name = "do")]
    fn apply(&self, scene: &Obj) -> PyResult<()> {
        let py = scene.py();
        let mesh = scene.getattr("mesh")?;
        let face = self.face.bind(py);
        let result = mesh
            .getattr("_store")?
            .call_method1("offset_face", (face.getattr("key")?, &self.regions))?;
        mesh.call_method0("_dirty")?;
        let view = py.import("ingetrazo_core.store_mesh")?.getattr("FaceView")?;
        for key in result.get_item(0)?.try_iter()? {
            view.call1((&mesh, key?))?
                .getattr("attrs")?
                .call_method1("update", (self.attrs.bind(py),))?;
        }
        scene
            .getattr("selection")?
            .call_method1("discard", (face,))?;
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)
    }
}
#[pyfunction]
fn build_offset_face<'p>(
    mesh: &Obj<'p>,
    face: &Obj<'p>,
    distance: f64,
) -> PyResult<Option<Obj<'p>>> {
    let py = mesh.py();
    let store = mesh.getattr("_store")?;
    let face = if face.is_instance_of::<PyInt>() {
        if !store.call_method1("is_live", (face,))?.is_truthy()? {
            return Err(PyKeyError::new_err(face.clone().unbind()));
        }
        py.import("ingetrazo_core.store_mesh")?
            .call_method1("FaceView", (mesh, face))?
    } else {
        face.clone()
    };
    let outline = face.getattr("vertices")?;
    let normal = face.call_method0("normal")?;
    let regions = offset_regions(&outline, &normal, distance)?;
    let mut outlines = regions
        .iter()
        .map(|p| points(&p.get_item(0)?))
        .collect::<PyResult<Vec<_>>>()?;
    if outlines.is_empty() {
        let fallback = py
            .import("core.topology")?
            .call_method1("offset_loop", (&outline, &normal, distance))?;
        if fallback.is_none() {
            return Ok(None);
        }
        outlines.push(points(&fallback)?);
    }
    let h = convention(py)?;
    let holes = face
        .getattr("holes")?
        .try_iter()?
        .map(|v| points(&v?))
        .collect::<PyResult<_>>()?;
    let source = points(&outline)?;
    let mut layout = offset::layout(
        source.clone(),
        holes,
        outlines.clone(),
        xyz(&normal)?.to_array(),
        distance,
        h,
    )
    .ok_or_else(|| OffsetHoleCollision::new_err(()))?;
    for region in &mut layout {
        if region.len() > 2 {
            let holes = region[1..]
                .iter()
                .map(|ring| ring.iter().map(|&[x, y, z]| V3(x, y, z)).collect())
                .collect::<Vec<_>>();
            let keep = crate::store::fillet::maximal_holes(&holes, h);
            let mut flags = std::iter::once(true).chain(keep);
            region.retain(|_| flags.next().unwrap_or(false));
        }
    }
    let attrs = PyDict::new(py);
    attrs.call_method1("update", (face.getattr("attrs")?,))?;
    let cmds = PyList::empty(py);
    cmds.append(Py::new(
        py,
        OffsetFace {
            face: face.unbind(),
            attrs: attrs.unbind(),
            regions: layout,
        },
    )?)?;
    if outlines.len() == 1 && source.len() == outlines[0].len() {
        let history = py.import("ingetrazo_core.history")?;
        let mut ids = Vec::new();
        for i in 0..source.len() {
            let a = mesh.call_method1("vertex_at", (outline.get_item(i)?,))?;
            let b = mesh.call_method1("vertex_at", (outline.get_item((i + 1) % source.len())?,))?;
            let edge = mesh.call_method1("find_edge", (a, b))?;
            ids.push(if edge.is_none() {
                None
            } else {
                edge.getattr("curve")?.extract()?
            });
        }
        for (indices, closed) in crate::push_command::outline_curve_runs(ids, false) {
            let kw = PyDict::new(py);
            kw.set_item("closed", closed)?;
            cmds.append(
                history.getattr("TagCurveCommand")?.call(
                    (indices
                        .iter()
                        .map(|&i| vec3(outlines[0][i]))
                        .collect::<Vec<_>>(),),
                    Some(&kw),
                )?,
            )?;
        }
    }
    Ok(Some(
        py.import("ingetrazo_core")?
            .call_method1("resident_face_compound", (cmds,))?,
    ))
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add(
        "OffsetHoleCollision",
        m.py().get_type::<OffsetHoleCollision>(),
    )?;
    m.add_function(wrap_pyfunction!(crate::object_outline::interior_point, m)?)?;
    for f in [
        wrap_pyfunction!(offset_regions, m)?,
        wrap_pyfunction!(build_offset_face, m)?,
        wrap_pyfunction!(offset_chain, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
