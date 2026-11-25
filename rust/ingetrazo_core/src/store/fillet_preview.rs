// SPDX-License-Identifier: GPL-3.0-or-later
//! Carry one store transaction from a tool preview to the history command.
//! Material dictionaries belong to the document and are captured when planning.
use super::{PyChange, PyMeshStore, PyRoundPlan};
use crate::{
    math::Vec3,
    store::{
        fillet::{valid_controls, xyz},
        geometry::V3,
        Kind,
    },
};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::collections::HashSet;
type Obj<'p> = Bound<'p, PyAny>;

#[pyclass(frozen, module = "ingetrazo_core")]
pub(crate) struct BevelPreview {
    #[pyo3(get)]
    corners: String,
    #[pyo3(get)]
    shape: Option<Py<PyAny>>,
    geometry: PyRoundPlan,
    rebuilt_materials: Vec<Py<PyDict>>,
    strip_material: Py<PyDict>,
}

fn vectors(rings: &[Vec<V3>]) -> Vec<Vec<Vec3>> {
    rings
        .iter()
        .map(|r| {
            r.iter()
                .map(|p| Vec3::from(glam::DVec3::from_array(xyz(*p))))
                .collect()
        })
        .collect()
}

#[pymethods]
impl BevelPreview {
    #[getter]
    fn strips(&self) -> Vec<Vec<Vec3>> {
        vectors(&self.geometry.inner.strips)
    }
    #[getter]
    fn patches(&self) -> Vec<Vec<Vec3>> {
        vectors(&self.geometry.inner.patches)
    }
    #[getter]
    fn curves(&self) -> Vec<Vec<Vec3>> {
        vectors(&self.geometry.inner.curves)
    }
    #[getter]
    fn new_faces(&self) -> Vec<Vec<Vec3>> {
        let mut faces = self.strips();
        faces.extend(self.patches());
        faces
    }
    #[getter]
    fn removed_faces(&self) -> Vec<u128> {
        self.geometry.faces.clone()
    }
    #[getter]
    fn removed_edges(&self) -> Vec<u128> {
        self.geometry.edges.clone()
    }

    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        for attrs in &self.rebuilt_materials {
            visit.call(attrs)?;
        }
        visit.call(&self.strip_material)
    }
}

fn preview<'py>(
    mesh: &Obj<'py>,
    result: PyResult<PyRoundPlan>,
    vertex: bool,
    corners: &str,
    shape: Option<Py<PyAny>>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let geometry = match result {
        Ok(p) => p,
        Err(e) if e.is_instance_of::<PyValueError>(py) => {
            let Ok((message, point)) = e
                .value(py)
                .getattr("args")?
                .extract::<(String, Option<[f64; 3]>)>()
            else {
                return Err(e);
            };
            let fields = PyDict::new(py);
            if let Some([x, y, z]) = point {
                fields.set_item("p", format!("({x:.2}, {y:.2}, {z:.2})"))?;
            }
            return py
                .import("ingetrazo_core.i18n")?
                .call_method("tr", (message,), Some(&fields));
        }
        Err(e) => return Err(e),
    };
    let view = py.import("ingetrazo_core.store_mesh")?.getattr("FaceView")?;
    let attrs = |key: u128| {
        view.call1((mesh, key))?
            .getattr("attrs")?
            .cast_into::<PyDict>()
            .map_err(PyErr::from)
    };
    let rebuilt_materials = geometry
        .faces
        .iter()
        .map(|&k| Ok(attrs(k)?.copy()?.unbind()))
        .collect::<PyResult<_>>()?;
    let material = PyDict::new(py);
    let mut sources = geometry.materials.iter();
    if let Some(&key) = sources.next() {
        let first = attrs(key)?;
        const FIELDS: [&str; 5] = ["color", "texture", "mat", "opacity", "pbr"];
        for k in FIELDS {
            if let Some(v) = first.get_item(k)? {
                material.set_item(k, v)?;
            }
        }
        'compare: for &key in sources {
            let other = attrs(key)?;
            for field in FIELDS {
                let a = first.get_item(field)?;
                let b = other.get_item(field)?;
                let same = match (a, b) {
                    (Some(a), Some(b)) => a.eq(b)?,
                    (None, None) => true,
                    (Some(v), None) | (None, Some(v)) => !vertex && v.is_none(),
                };
                if !same {
                    material.clear();
                    break 'compare;
                }
            }
        }
    }
    Ok(Bound::new(
        py,
        BevelPreview {
            corners: corners.to_owned(),
            shape,
            geometry,
            rebuilt_materials,
            strip_material: material.unbind(),
        },
    )?
    .into_any())
}

fn selected(elements: &Obj<'_>) -> PyResult<Vec<u128>> {
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    for elem in elements.try_iter()? {
        let key = elem?.getattr("key")?.extract()?;
        if seen.insert(key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

fn refusal(keys: &[u128], size: f64, profile: f64, empty: &'static str) -> Option<&'static str> {
    if keys.is_empty() {
        Some(empty)
    } else if size <= 0. {
        Some("The radius must be positive.")
    } else if !(0. < profile && profile < 1.) {
        Some("The profile must be between 0 and 1.")
    } else {
        None
    }
}

fn hypot(py: Python<'_>) -> PyResult<String> {
    py.import("core._native")?
        .call_method0("qt_hypot")?
        .extract()
}

#[pyfunction]
#[pyo3(signature=(mesh, edges, radius, segments=8, profile=0.5, measure="radius", corners="round", shape=None))]
pub(crate) fn plan_fillet<'p>(
    mesh: &Obj<'p>,
    edges: &Obj<'p>,
    radius: f64,
    segments: isize,
    profile: f64,
    measure: &str,
    corners: &str,
    shape: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    let keys = selected(edges)?;
    if let Some(msg) = refusal(&keys, radius, profile, "Pick an edge to round.") {
        return py.import("ingetrazo_core.i18n")?.call_method1("tr", (msg,));
    }
    let measure = match measure {
        "offset" | "width" | "depth" => measure,
        _ => "radius",
    };
    let corners = if corners == "cut" { "cut" } else { "round" };
    let mut controls = Vec::new();
    let mut name = String::new();
    if let Some(shape) = shape {
        if shape.is_instance_of::<PyTuple>() || shape.is_instance_of::<PyList>() {
            controls = crate::bevel_editor::read(shape).unwrap_or_default();
            if !valid_controls(&controls) {
                return py.import("ingetrazo_core.i18n")?.call_method1(
                    "tr",
                    ("The drawn profile must run from one face to the other.",),
                );
            }
            name.push_str("custom");
        } else if let Ok(s) = shape.extract::<String>() {
            if matches!(s.as_str(), "steps" | "cornice" | "crown" | "supports") {
                name = s;
            }
        }
    }
    let chosen_shape = match name.as_str() {
        "" => None,
        "custom" => Some(PyTuple::new(py, &controls)?.into_any().unbind()),
        _ => Some(name.clone().into_pyobject(py)?.into_any().unbind()),
    };
    let store = mesh.getattr("_store")?;
    let plan = store.extract::<PyRef<PyMeshStore>>()?.plan_fillet(
        keys,
        radius,
        segments.max(1) as usize,
        profile,
        measure,
        corners,
        &name,
        controls,
        &hypot(py)?,
    );
    preview(mesh, plan, false, corners, chosen_shape)
}

#[pyfunction]
#[pyo3(signature=(mesh,vertices,size,segments=8,profile=0.5))]
pub(crate) fn plan_vertex_bevel<'p>(
    mesh: &Obj<'p>,
    vertices: &Obj<'p>,
    size: f64,
    segments: isize,
    profile: f64,
) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    let keys = selected(vertices)?;
    if let Some(msg) = refusal(&keys, size, profile, "Pick a corner to bevel.") {
        return py.import("ingetrazo_core.i18n")?.call_method1("tr", (msg,));
    }
    let store = mesh.getattr("_store")?;
    let plan = store.extract::<PyRef<PyMeshStore>>()?.plan_vertex_bevel(
        keys,
        size,
        segments.max(1) as usize,
        profile,
        &hypot(py)?,
    );
    preview(mesh, plan, true, "round", None)
}

#[pyfunction]
pub(crate) fn apply_fillet(mesh: &Obj<'_>, plan: &BevelPreview) -> PyResult<PyChange> {
    let py = mesh.py();
    let native = &plan.geometry;
    let curves = (0..native.inner.curves.len())
        .map(|_| mesh.call_method0("next_curve_id")?.extract())
        .collect::<PyResult<_>>()?;
    let store = mesh.getattr("_store")?;
    let change = store
        .extract::<PyRefMut<PyMeshStore>>()?
        .apply_fillet(native, curves, None)?;
    let created = &change.inner.created[Kind::Face as usize];
    let own = mesh.getattr("__dict__")?.get_item("_attr_own")?;
    for (&key, attrs) in created.iter().zip(&plan.rebuilt_materials) {
        own.set_item(key, attrs.bind(py).copy()?)?;
    }
    if !plan.strip_material.bind(py).is_empty() {
        for &key in &created[plan.rebuilt_materials.len()..] {
            own.set_item(key, plan.strip_material.bind(py).copy()?)?;
        }
    }
    let rings = native
        .inner
        .rebuilt
        .iter()
        .flat_map(|(_, rs)| rs.iter())
        .chain(&native.inner.strips)
        .chain(&native.inner.patches);
    let dirty = rings.map(Vec::len).sum::<usize>()
        + created.len()
        + native.inner.curves.len()
        + change.inner.removed[Kind::Face as usize].len()
        + change.inner.removed[Kind::Edge as usize].len();
    mesh.call_method1("_dirty", (dirty,))?;
    Ok(change)
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<BevelPreview>()?;
    m.add_function(wrap_pyfunction!(plan_fillet, m)?)?;
    m.add_function(wrap_pyfunction!(plan_vertex_bevel, m)?)?;
    m.add_function(wrap_pyfunction!(apply_fillet, m)?)?;
    Ok(())
}
