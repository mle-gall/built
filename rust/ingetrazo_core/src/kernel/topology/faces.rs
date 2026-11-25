// SPDX-License-Identifier: GPL-3.0-or-later
//! Address attached faces by store key, and detached faces by an ordered outline.
use super::{store_api, Obj};
use crate::{
    math::{xyz, Vec3},
    object_outline::{self, positions, vectors, FaceOutline},
    store::py::PyMeshStore,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Pieces = Option<(Vec<Vec3>, Vec<Vec3>)>;

pub(super) fn exact_mesh(mesh: &Obj<'_>) -> PyResult<bool> {
    Ok(mesh
        .get_type()
        .is(&store_api(mesh.py())?.getattr("StoreMesh")?))
}
pub(super) fn hypot(py: Python<'_>) -> PyResult<Option<String>> {
    store_api(py)?.call_method0("_hypot")?.extract()
}

struct Stored<'py> {
    mesh: Obj<'py>,
    store: Bound<'py, PyMeshStore>,
    key: u128,
    h: String,
}
impl<'py> Stored<'py> {
    fn of(face: &Obj<'py>) -> PyResult<Option<Self>> {
        let Some(mesh) = face.getattr_opt("_m")? else {
            return Ok(None);
        };
        if !exact_mesh(&mesh)? || !face.is_instance(&store_api(face.py())?.getattr("FaceView")?)? {
            return Ok(None);
        }
        let Some(h) = hypot(face.py())? else {
            return Ok(None);
        };
        Ok(Some(Self {
            store: mesh.getattr("_store")?.cast_into()?,
            key: face.getattr("key")?.extract()?,
            mesh,
            h,
        }))
    }
    fn peer(&self, key: Option<u128>) -> PyResult<Option<Obj<'py>>> {
        key.map(|k| store_api(self.mesh.py())?.call_method1("FaceView", (&self.mesh, k)))
            .transpose()
    }
}
#[pyfunction]
fn _drawing_store<'py>(face: &Obj<'py>) -> PyResult<Option<Bound<'py, PyMeshStore>>> {
    Ok(Stored::of(face)?.map(|v| v.store))
}
#[pyfunction]
#[pyo3(signature=(mother,r#loop,outside_holes=false))]
fn loop_inside_face(mother: &Obj<'_>, r#loop: &Obj<'_>, outside_holes: bool) -> PyResult<bool> {
    match Stored::of(mother)? {
        Some(s) => {
            s.store
                .try_borrow()?
                .loop_inside_face(s.key, positions(r#loop)?, outside_holes, &s.h)
        }
        None => FaceOutline::new(mother)?.contains(r#loop, outside_holes),
    }
}
#[pyfunction]
fn hole_merged_with_loop(face: &Obj<'_>, r#loop: &Obj<'_>) -> PyResult<Option<(usize, Vec<Vec3>)>> {
    match Stored::of(face)? {
        Some(s) => s
            .store
            .try_borrow()?
            .merged_hole(face.py(), s.key, positions(r#loop)?, &s.h),
        None => FaceOutline::new(face)?.merged_hole(r#loop),
    }
}
fn seq_mesh<'py>(seq: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let Some(mesh) = seq.getattr_opt("owner")? else {
        return Ok(None);
    };
    Ok(
        (exact_mesh(&mesh)? && seq.is_instance(&seq.py().import("ingetrazo_core")?.getattr("MeshElements")?)?)
            .then_some(mesh),
    )
}
fn keys(faces: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<Option<Vec<u128>>> {
    let cls = store_api(mesh.py())?.getattr("FaceView")?;
    let mut out = Vec::new();
    for f in faces.try_iter()? {
        let f = f?;
        if !f.is_instance(&cls)? || !f.getattr("_m")?.is(mesh) {
            return Ok(None);
        }
        out.push(f.getattr("key")?.extract()?);
    }
    Ok(Some(out))
}
#[pyfunction]
#[pyo3(signature=(faces,r#loop,exclude=None))]
fn find_containing_face<'py>(
    faces: &Obj<'py>,
    r#loop: &Obj<'py>,
    exclude: Option<&Obj<'py>>,
) -> PyResult<Option<Obj<'py>>> {
    let py = faces.py();
    let group = if let Some(m) = seq_mesh(faces)? {
        Some((m, None))
    } else if (faces.is_instance_of::<PyList>() || faces.is_instance_of::<PyTuple>())
        && faces.len()? > 0
    {
        if let Some(s) = Stored::of(&faces.get_item(0)?)? {
            keys(faces, &s.mesh)?.map(|k| (s.mesh, Some(k)))
        } else {
            None
        }
    } else {
        None
    };
    if let (Some((mesh, candidates)), Some(h)) = (group, hypot(py)?) {
        let st = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
        let excluded = match exclude {
            Some(e) => e.getattr_opt("key")?.map(|v| v.extract()).transpose()?,
            None => None,
        };
        let k = st
            .try_borrow()?
            .containing_face(positions(r#loop)?, candidates, excluded, &h)?;
        return k
            .map(|k| store_api(py)?.call_method1("FaceView", (mesh, k)))
            .transpose();
    }
    object_outline::containing_object(faces, r#loop, exclude.unwrap_or(&py.None().into_bound(py)))
}
#[pyfunction]
fn orphaned_edges_at<'py>(
    edges: &Obj<'py>,
    faces: &Obj<'py>,
    vertices: &Obj<'py>,
) -> PyResult<Vec<Obj<'py>>> {
    if let (Some(m), Some(other)) = (seq_mesh(edges)?, seq_mesh(faces)?) {
        if m.is(&other) {
            let st = m.getattr("_store")?.cast_into::<PyMeshStore>()?;
            let keys = st
                .try_borrow()?
                .orphaned_edges_at(positions(vertices)?, None, None)?;
            let view = store_api(m.py())?.getattr("EdgeView")?;
            return keys.into_iter().map(|k| view.call1((&m, k))).collect();
        }
    }
    object_outline::orphan_objects(edges, faces, vertices)
}
#[pyfunction]
#[pyo3(signature=(r#loop,points,tol=1e-4,*,mesh=None))]
fn refine_loop_with_points(
    r#loop: &Obj<'_>,
    points: &Obj<'_>,
    tol: f64,
    mesh: Option<&Obj<'_>>,
) -> PyResult<Vec<Vec3>> {
    if let Some(mesh) = mesh {
        if exact_mesh(mesh)? {
            if let Some(h) = hypot(mesh.py())? {
                let st = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                return Ok(vectors(st.try_borrow()?.refine_ring(
                    positions(r#loop)?,
                    positions(points)?,
                    tol,
                    &h,
                )?));
            }
        }
    }
    object_outline::refined_ring(r#loop, points, tol)
}
#[pyfunction]
#[pyo3(signature=(face,a,b,faces,memo=None))]
fn classify_push_edge<'py>(
    face: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    faces: &Obj<'py>,
    memo: Option<&Bound<'py, PyDict>>,
) -> PyResult<(&'static str, Option<Obj<'py>>)> {
    let mut list = None;
    if let Some(s) = Stored::of(face)? {
        let candidates = if seq_mesh(faces)?.is_some_and(|m| m.is(&s.mesh)) {
            Some(None)
        } else {
            let rows = face.py().get_type::<PyList>().call1((faces,))?;
            let candidates = keys(&rows, &s.mesh)?;
            list = Some(rows);
            candidates.map(Some)
        };
        if let Some(c) = candidates {
            let (kind, k) = s.store.try_borrow()?.classify_segment(
                s.key,
                xyz(a)?.to_array(),
                xyz(b)?.to_array(),
                c,
                &s.h,
            )?;
            return Ok((kind, s.peer(k)?));
        }
    }
    crate::object_push_edges::classify_object_edge(face, a, b, list.as_ref().unwrap_or(faces), memo)
}
fn split(face: &Obj<'_>, chain: &Obj<'_>, chord: bool) -> PyResult<Pieces> {
    if let Some(s) = Stored::of(face)? {
        let result =
            s.store
                .try_borrow()?
                .split_face_loops(s.key, positions(chain)?, chord, &s.h)?;
        return Ok(result.map(|mut r| {
            let b = r.pop().unwrap();
            (vectors(r.pop().unwrap()), vectors(b))
        }));
    }
    FaceOutline::new(face)?.split(chain, chord)
}
#[pyfunction]
fn split_face_by_chord(face: &Obj<'_>, a: &Obj<'_>, b: &Obj<'_>) -> PyResult<Pieces> {
    split(face, PyTuple::new(face.py(), [a, b])?.as_any(), true)
}
#[pyfunction]
fn find_chord_split<'py>(
    faces: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
) -> PyResult<Option<(Obj<'py>, Vec<Vec3>, Vec<Vec3>)>> {
    let chain = PyTuple::new(faces.py(), [a, b])?;
    for f in faces.try_iter()? {
        let f = f?;
        if let Some((l, r)) = split(&f, &chain, true)? {
            return Ok(Some((f, l, r)));
        }
    }
    Ok(None)
}
#[pyfunction]
fn split_face_by_chain(face: &Obj<'_>, chain: &Obj<'_>) -> PyResult<Pieces> {
    split(face, chain, false)
}
#[pyfunction]
fn find_subdividing_chain(face: &Obj<'_>, r#loop: &Obj<'_>) -> PyResult<Option<Vec<Vec3>>> {
    match Stored::of(face)? {
        Some(s) => Ok(s
            .store
            .try_borrow()?
            .subdividing_chain(s.key, positions(r#loop)?, &s.h)?
            .map(vectors)),
        None => FaceOutline::new(face)?.chain(r#loop),
    }
}
#[pyfunction]
fn subtract_loop_from_face(face: &Obj<'_>, r#loop: &Obj<'_>) -> PyResult<Option<Vec<Vec3>>> {
    let Some(chain) = find_subdividing_chain(face, r#loop)? else {
        return Ok(None);
    };
    let Some((a, b)) = split(face, PyList::new(face.py(), chain)?.as_any(), false)? else {
        return Ok(None);
    };
    let key = crate::object_drawing::outline_key(r#loop)?;
    let matches = |r: &[Vec3]| {
        let mut k = r
            .iter()
            .map(|v| crate::store::commands::cell(v.coords()))
            .collect::<Vec<_>>();
        k.sort_unstable();
        k.dedup();
        k == key
    };
    Ok(if matches(&a) {
        Some(b)
    } else if matches(&b) {
        Some(a)
    } else {
        None
    })
}
#[pyfunction]
fn carve_loop_by_chords(face: &Obj<'_>, r#loop: &Obj<'_>) -> PyResult<Option<Vec<Vec<Vec3>>>> {
    match Stored::of(face)? {
        Some(s) => Ok(s
            .store
            .try_borrow()?
            .carve_face(s.key, positions(r#loop)?, &s.h)?
            .map(|r| r.into_iter().map(vectors).collect())),
        None => FaceOutline::new(face)?.carve(r#loop),
    }
}

pub(super) fn register(s: &Bound<'_, PyModule>) -> PyResult<()> {
    s.add_function(wrap_pyfunction!(_drawing_store, s)?)?;
    s.add_function(wrap_pyfunction!(loop_inside_face, s)?)?;
    s.add_function(wrap_pyfunction!(hole_merged_with_loop, s)?)?;
    s.add_function(wrap_pyfunction!(find_containing_face, s)?)?;
    s.add_function(wrap_pyfunction!(orphaned_edges_at, s)?)?;
    s.add_function(wrap_pyfunction!(refine_loop_with_points, s)?)?;
    s.add_function(wrap_pyfunction!(classify_push_edge, s)?)?;
    s.add_function(wrap_pyfunction!(split_face_by_chord, s)?)?;
    s.add_function(wrap_pyfunction!(find_chord_split, s)?)?;
    s.add_function(wrap_pyfunction!(split_face_by_chain, s)?)?;
    s.add_function(wrap_pyfunction!(find_subdividing_chain, s)?)?;
    s.add_function(wrap_pyfunction!(subtract_loop_from_face, s)?)?;
    s.add_function(wrap_pyfunction!(carve_loop_by_chords, s)?)?;
    Ok(())
}
