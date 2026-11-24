// SPDX-License-Identifier: GPL-3.0-or-later
//! Public push queries. The store supplies keyed views and its revision cache.
//! Detached meshes obtain a fresh scan because scripts can mutate a point in place.
use super::push_queries as bounds;
use crate::{
    math::{xyz, Vec3},
    pushscan as scan,
    scans::MeshScan,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
type P = (f64, f64, f64);

struct Scan<'py> {
    view: Obj<'py>,
    native: Bound<'py, MeshScan>,
}
impl<'py> Scan<'py> {
    fn open(mesh: &Obj<'py>, required: bool) -> PyResult<Option<Self>> {
        let py = mesh.py();
        if mesh.is_none() && !required {
            return Ok(None);
        }
        let api = py.import("core.scan_index")?;
        let stored = api.call_method1("store_backed", (mesh,))?.is_truthy()?;
        let mut view = api.call_method1("scan", (mesh, !stored))?;
        if view.is_none() {
            if !required {
                return Ok(None);
            }
            view = api.call_method1("Scan", (mesh,))?;
        }
        let native = view.getattr("native")?.cast_into::<MeshScan>()?;
        Ok(Some(Self { view, native }))
    }
    fn index(&self, face: &Obj<'py>) -> PyResult<Option<u32>> {
        self.view
            .call_method1("face_indices", (PyList::new(face.py(), [face])?,))?
            .get_item(0)?
            .extract()
    }
    fn for_face(mesh: &Obj<'py>, face: &Obj<'py>) -> PyResult<Option<(Self, u32)>> {
        let s = Self::open(mesh, true)?.unwrap();
        Ok(s.index(face)?.map(|i| (s, i)))
    }
    fn rows(&self, kind: &str, idx: Vec<u32>) -> PyResult<Option<Obj<'py>>> {
        Ok(Some(self.view.call_method1(kind, (idx,))?))
    }
}
fn hypot(py: Python<'_>) -> PyResult<String> {
    py.import("core._native")?
        .call_method0("qt_hypot")?
        .extract()
}
fn sum_mode(py: Python<'_>) -> bool {
    py.version_info().minor >= 12
}
fn p(v: &Obj<'_>) -> PyResult<P> {
    let v = xyz(v)?;
    Ok((v.x, v.y, v.z))
}
fn store_backed(mesh: &Obj<'_>) -> PyResult<bool> {
    mesh.py()
        .import("core.scan_index")?
        .call_method1("store_backed", (mesh,))?
        .is_truthy()
}
fn store_enabled(mesh: &Obj<'_>) -> PyResult<bool> {
    Ok(store_backed(mesh)?
        && mesh
            .py()
            .import("core.scan_index")?
            .call_method0("enabled")?
            .is_truthy()?)
}
fn points(input: &Obj<'_>) -> PyResult<Vec<[f64; 3]>> {
    input
        .try_iter()?
        .map(|v| Ok(xyz(&v?)?.to_array()))
        .collect()
}

#[pyfunction]
fn is_closed(mesh: &Obj<'_>) -> PyResult<Option<bool>> {
    if !store_backed(mesh)? {
        return Ok(None);
    }
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    Ok(Some(scan::scan_is_closed(
        mesh.py(),
        s.native.try_borrow_mut()?,
    )?))
}
#[pyfunction]
pub(crate) fn mesh_is_flat(mesh: &Obj<'_>, tol: f64) -> PyResult<Option<bool>> {
    if !store_enabled(mesh)? {
        return Ok(None);
    }
    mesh.getattr("_store")?
        .call_method1("faces_are_flat", (tol, hypot(mesh.py())?))?
        .extract()
}
#[pyfunction]
fn faces_on_segment<'py>(
    mesh: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    let idx = scan::scan_faces_holding_points(
        mesh.py(),
        s.native.try_borrow_mut()?,
        vec![p(a)?, p(b)?],
        4e-4,
        2e-4,
    )?;
    s.rows("faces", idx)
}
#[pyfunction]
#[pyo3(signature=(mesh,n,base_xy,dot_min=0.999))]
fn parallel_overlapping<'py>(
    mesh: &Obj<'py>,
    n: &Obj<'py>,
    base_xy: &Obj<'py>,
    dot_min: f64,
) -> PyResult<Option<Obj<'py>>> {
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    if !base_xy.is_truthy()? {
        return Ok(None);
    }
    let (u, v) = crate::store::planar_api::plane_basis(n)?;
    let (lo, hi) = bounds::bounds_xy(base_xy.extract()?)?;
    let py = mesh.py();
    let idx = scan::scan_parallel_overlapping(
        py,
        s.native.try_borrow_mut()?,
        p(n)?,
        u.coords().into(),
        v.coords().into(),
        lo,
        hi,
        dot_min,
        &hypot(py)?,
        sum_mode(py),
    )?;
    s.rows("faces", idx)
}
#[pyfunction]
fn lateral_exit(
    mesh: &Obj<'_>,
    base_face: &Obj<'_>,
    samples: &Obj<'_>,
    direction: &Obj<'_>,
    n: &Obj<'_>,
) -> PyResult<Option<(bool, Option<f64>)>> {
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    let py = mesh.py();
    let idx = s.index(base_face)?;
    let t = scan::scan_lateral_exit(
        py,
        s.native.try_borrow_mut()?,
        bounds::positions(samples)?,
        p(direction)?,
        p(n)?,
        idx,
        &hypot(py)?,
        sum_mode(py),
    )?;
    Ok(Some((true, t)))
}
#[pyfunction]
fn inward_limit(
    mesh: &Obj<'_>,
    base_face: &Obj<'_>,
    n: &Obj<'_>,
    anchor: &Obj<'_>,
    stitch_tol: f64,
) -> PyResult<Option<f64>> {
    let Some((s, i)) = Scan::for_face(mesh, base_face)? else {
        return Ok(None);
    };
    let py = mesh.py();
    scan::scan_inward_limit(
        py,
        s.native.try_borrow_mut()?,
        i,
        p(n)?,
        p(anchor)?,
        stitch_tol,
        &hypot(py)?,
        sum_mode(py),
    )
}

fn vertices_by_boxes<'py>(s: &Scan<'py>, boxes: Vec<[f64; 6]>) -> PyResult<Option<Obj<'py>>> {
    let idx = scan::scan_vertices_in_boxes(s.view.py(), s.native.try_borrow_mut()?, boxes)?;
    s.rows("vertices", idx)
}
#[pyfunction]
fn vertices_in_boxes<'py>(mesh: &Obj<'py>, boxes: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    let mut packed = Vec::new();
    for b in boxes.try_iter()? {
        let b = b?;
        let seq = mesh.py().get_type::<PyTuple>().call1((b,))?;
        packed.push(seq.extract()?);
    }
    vertices_by_boxes(&s, packed)
}
#[pyfunction]
fn vertices_at_keys<'py>(mesh: &Obj<'py>, points: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let boxes = bounds::key_boxes(points)?;
    if boxes.is_empty() {
        return Ok(Some(PyList::empty(mesh.py()).into_any()));
    }
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    vertices_by_boxes(&s, boxes)
}
#[pyfunction]
fn faces_meeting<'py>(mesh: &Obj<'py>, lo: P, hi: P) -> PyResult<Option<Obj<'py>>> {
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    let idx = s
        .native
        .try_borrow_mut()?
        .faces_meeting(mesh.py(), lo, hi)?;
    s.rows("faces", idx)
}
enum Neighborhood {
    Rings,
    Outer,
    Edges,
}
fn near<'py>(mesh: &Obj<'py>, points: &Obj<'py>, kind: Neighborhood) -> PyResult<Option<Obj<'py>>> {
    let Some((lo, hi)) = bounds::box_of(points, 2e-4)? else {
        return Ok(Some(PyList::empty(mesh.py()).into_any()));
    };
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    let py = mesh.py();
    let (field, idx) = match kind {
        Neighborhood::Rings => (
            "faces",
            scan::scan_faces_meeting_rings(py, s.native.try_borrow_mut()?, lo, hi)?,
        ),
        Neighborhood::Outer => (
            "faces",
            s.native.try_borrow_mut()?.faces_meeting(py, lo, hi)?,
        ),
        Neighborhood::Edges => (
            "edges",
            s.native.try_borrow_mut()?.edges_meeting(py, lo, hi)?,
        ),
    };
    s.rows(field, idx)
}
#[pyfunction]
fn faces_near_points_rings<'py>(mesh: &Obj<'py>, points: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    near(mesh, points, Neighborhood::Rings)
}
#[pyfunction]
fn faces_near_points<'py>(mesh: &Obj<'py>, points: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    near(mesh, points, Neighborhood::Outer)
}
#[pyfunction]
fn edges_near_points<'py>(mesh: &Obj<'py>, points: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    near(mesh, points, Neighborhood::Edges)
}
#[pyfunction]
#[pyo3(signature=(mesh,near=None))]
fn cracked_edges<'py>(mesh: &Obj<'py>, near: Option<&Obj<'py>>) -> PyResult<Option<Obj<'py>>> {
    let Some(s) = Scan::open(mesh, false)? else {
        return Ok(None);
    };
    let near = near
        .map(|n| mesh.py().get_type::<PyTuple>().call1((n,))?.extract())
        .transpose()?;
    let idx = scan::scan_cracked_edges(mesh.py(), s.native.try_borrow_mut()?, near)?;
    s.rows("edges", idx)
}
#[pyfunction]
fn live_keys<'py>(mesh: &Obj<'py>, kind: &str) -> PyResult<Option<Obj<'py>>> {
    if !store_enabled(mesh)? {
        return Ok(None);
    }
    let store = mesh.getattr("_store")?;
    let base = store.getattr("key_base")?.extract::<u128>()?;
    let result = if let Ok(base) = u64::try_from(base) {
        let keys = store.call_method1("local_keys", (kind,))?;
        keys.call_method1(
            "__or__",
            (mesh.py().import("numpy")?.call_method1("uint64", (base,))?,),
        )?
        .call_method0("tolist")?
    } else {
        store.call_method1("keys", (kind,))?
    };
    Ok(Some(result))
}
fn view_type<'py>(py: Python<'py>, kind: &str) -> PyResult<Obj<'py>> {
    let name = match kind {
        "face" => "FaceView",
        "edge" => "EdgeView",
        "vertex" => "VertexView",
        _ => return Err(pyo3::exceptions::PyKeyError::new_err(kind.to_owned())),
    };
    py.import("core.store_mesh")?.getattr(name)
}
#[pyfunction]
fn views<'py>(mesh: &Obj<'py>, kind: &str, keys: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    bounds::views(mesh, &view_type(mesh.py(), kind)?, keys)
}
#[pyfunction]
fn mark<'py>(mesh: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    if !store_enabled(mesh)? {
        return Ok(None);
    }
    Ok(Some(mesh.getattr("_store")?.call_method0("snapshot")?))
}
#[pyfunction]
fn made_since<'py>(
    mesh: &Obj<'py>,
    mark: &Obj<'py>,
    kind: &str,
) -> PyResult<Option<Vec<Obj<'py>>>> {
    if mark.is_none()
        || !store_backed(mesh)?
        || mesh
            .py()
            .import("core._native")?
            .getattr("native")?
            .is_none()
    {
        return Ok(None);
    }
    let keys = scan::store_keys_since(mesh.getattr("_store")?.extract()?, mark.extract()?, kind)?;
    let cls = view_type(mesh.py(), kind)?;
    Ok(Some(
        keys.into_iter()
            .map(|k| cls.call1((mesh, k)))
            .collect::<PyResult<_>>()?,
    ))
}
#[pyfunction]
fn set_minus<'py>(
    mesh: &Obj<'py>,
    kind: &str,
    after_keys: &Obj<'py>,
    before: &Obj<'py>,
) -> PyResult<Option<Bound<'py, pyo3::types::PySet>>> {
    let types = PyDict::new(mesh.py());
    types.set_item(kind, view_type(mesh.py(), kind)?)?;
    bounds::set_minus(mesh, &types, kind, after_keys, before)
}
#[pyfunction]
fn classify_base(mesh: &Obj<'_>, face: &Obj<'_>) -> PyResult<(bool, bool)> {
    let Some((s, i)) = Scan::for_face(mesh, face)? else {
        return Ok((false, false));
    };
    let py = mesh.py();
    scan::scan_classify_base(py, s.native.try_borrow_mut()?, i, &hypot(py)?, sum_mode(py))
}
#[pyfunction]
fn sweeps_free_sheet(mesh: &Obj<'_>, face: &Obj<'_>, loops: &Obj<'_>) -> PyResult<bool> {
    let Some((s, i)) = Scan::for_face(mesh, face)? else {
        return Ok(true);
    };
    let py = mesh.py();
    let loops = loops
        .try_iter()?
        .map(|r| points(&r?))
        .collect::<PyResult<_>>()?;
    scan::scan_sweeps_free_sheet(
        py,
        s.native.try_borrow_mut()?,
        i,
        loops,
        &hypot(py)?,
        sum_mode(py),
    )
}
#[pyfunction]
fn surface_normal<'py>(mesh: &Obj<'py>, face: &Obj<'py>) -> PyResult<Obj<'py>> {
    let Some((s, i)) = Scan::for_face(mesh, face)? else {
        return face.call_method0("normal")?.call_method0("normalized");
    };
    let py = mesh.py();
    let n =
        scan::scan_surface_normal(py, s.native.try_borrow_mut()?, i, &hypot(py)?, sum_mode(py))?;
    Ok(Vec3::from(glam::DVec3::new(n.0, n.1, n.2))
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn cap_passes_wall(
    mesh: &Obj<'_>,
    face: &Obj<'_>,
    cap: &Obj<'_>,
    normal: &Obj<'_>,
    d: f64,
) -> PyResult<bool> {
    let Some((s, i)) = Scan::for_face(mesh, face)? else {
        return Ok(false);
    };
    let py = mesh.py();
    scan::scan_cap_passes_wall(
        py,
        s.native.try_borrow_mut()?,
        i,
        points(cap)?,
        p(normal)?,
        d,
        &hypot(py)?,
        sum_mode(py),
    )
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "push_scans")?;
    sub.add_function(wrap_pyfunction!(is_closed, &sub)?)?;
    sub.add_function(wrap_pyfunction!(mesh_is_flat, &sub)?)?;
    sub.add_function(wrap_pyfunction!(faces_on_segment, &sub)?)?;
    sub.add_function(wrap_pyfunction!(parallel_overlapping, &sub)?)?;
    sub.add_function(wrap_pyfunction!(lateral_exit, &sub)?)?;
    sub.add_function(wrap_pyfunction!(inward_limit, &sub)?)?;
    sub.add_function(wrap_pyfunction!(vertices_in_boxes, &sub)?)?;
    sub.add_function(wrap_pyfunction!(vertices_at_keys, &sub)?)?;
    sub.add_function(wrap_pyfunction!(faces_meeting, &sub)?)?;
    sub.add_function(wrap_pyfunction!(faces_near_points_rings, &sub)?)?;
    sub.add_function(wrap_pyfunction!(faces_near_points, &sub)?)?;
    sub.add_function(wrap_pyfunction!(edges_near_points, &sub)?)?;
    sub.add_function(wrap_pyfunction!(cracked_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(live_keys, &sub)?)?;
    sub.add_function(wrap_pyfunction!(views, &sub)?)?;
    sub.add_function(wrap_pyfunction!(mark, &sub)?)?;
    sub.add_function(wrap_pyfunction!(made_since, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_minus, &sub)?)?;
    sub.add_function(wrap_pyfunction!(classify_base, &sub)?)?;
    sub.add_function(wrap_pyfunction!(sweeps_free_sheet, &sub)?)?;
    sub.add_function(wrap_pyfunction!(surface_normal, &sub)?)?;
    sub.add_function(wrap_pyfunction!(cap_passes_wall, &sub)?)?;
    sub.add("_grown_box", wrap_pyfunction!(bounds::grown_box, &sub)?)?;
    m.add_submodule(&sub)
}
