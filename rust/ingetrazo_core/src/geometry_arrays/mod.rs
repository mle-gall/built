// SPDX-License-Identifier: GPL-3.0-or-later
//! Indexed face bundles and placement quantities. Preserve saved point identity
//! across read-only queries, including coincident vertices in a deferred model.
use crate::math::xyz;
use glam::{DMat3, DVec3};
use numpy::{IntoPyArray, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
    IntoPyObjectExt,
};
use std::collections::{BTreeMap, HashMap};
type Obj<'p> = Bound<'p, PyAny>;
type Bundle<'p> = Bound<'p, Arrays>;
mod measures;
mod placements;
fn none(py: Python) -> Obj {
    py.None().into_bound(py)
}
fn numpy<'p>(py: Python<'p>, x: &Obj<'p>, ty: &str) -> PyResult<Obj<'p>> {
    py.import("numpy")?.call_method1("asarray", (x, ty))
}
fn col<'p>(py: Python<'p>, x: Vec<i64>) -> Obj<'p> {
    x.into_pyarray(py).into_any()
}
fn rows<'p>(py: Python<'p>, x: Vec<[f64; 3]>) -> PyResult<Obj<'p>> {
    let n = x.len();
    let flat = x.into_iter().flatten().collect::<Vec<_>>();
    Ok(flat.into_pyarray(py).reshape((n, 3))?.into_any())
}
fn row_values(x: &Obj) -> PyResult<Vec<[f64; 3]>> {
    let a = numpy(x.py(), x, "float64")?.extract::<PyReadonlyArray2<f64>>()?;
    let a = a.as_array();
    if a.ncols() != 3 {
        return Err(PyValueError::new_err(
            "expected three coordinates per point",
        ));
    }
    Ok(a.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect())
}
fn indices(x: &Obj) -> PyResult<Vec<i64>> {
    Ok(numpy(x.py(), x, "int64")?
        .extract::<PyReadonlyArray1<i64>>()?
        .as_array()
        .to_vec())
}
fn packed<'p>(
    py: Python<'p>,
    p: Obj<'p>,
    c: Obj<'p>,
    s: Obj<'p>,
    n: Obj<'p>,
    edges: Option<Obj<'p>>,
    double: bool,
) -> PyResult<Bundle<'p>> {
    Bound::new(
        py,
        Arrays::new(
            p.unbind(),
            c.unbind(),
            s.unbind(),
            n.unbind(),
            edges.map(Bound::unbind),
            double,
        ),
    )
}
#[pyclass(module = "core.geometry_arrays", get_all, set_all)]
struct Arrays {
    pos: Py<PyAny>,
    corners: Py<PyAny>,
    ring_sizes: Py<PyAny>,
    ring_counts: Py<PyAny>,
    edge_face_counts: Option<Py<PyAny>>,
    model_f64: bool,
    _measurements: Option<Py<PyAny>>,
    _perimeter: f64,
    _balanced: bool,
    _signed_volume: f64,
    _bounds: Option<([f64; 3], [f64; 3])>,
}
#[pymethods]
impl Arrays {
    #[new]
    #[pyo3(signature=(pos,corners,ring_sizes,ring_counts,edge_face_counts=None,model_f64=false))]
    fn new(
        pos: Py<PyAny>,
        corners: Py<PyAny>,
        ring_sizes: Py<PyAny>,
        ring_counts: Py<PyAny>,
        edge_face_counts: Option<Py<PyAny>>,
        model_f64: bool,
    ) -> Self {
        Self {
            pos,
            corners,
            ring_sizes,
            ring_counts,
            edge_face_counts,
            model_f64,
            _measurements: None,
            _perimeter: 0.,
            _balanced: true,
            _signed_volume: 0.,
            _bounds: None,
        }
    }
}
#[pyfunction]
fn faces_arrays<'p>(faces: &Obj<'p>) -> PyResult<Bundle<'p>> {
    let py = faces.py();
    let mut pos = vec![];
    let mut index = HashMap::<u128, i64>::new();
    let (mut cs, mut sizes, mut ns) = (vec![], vec![], vec![]);
    for f in faces.try_iter()? {
        let f = f?;
        let mut rs = vec![f.getattr("loop")?];
        rs.extend(
            f.getattr("hole_loops")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?,
        );
        ns.push(rs.len() as i64);
        for r in rs {
            sizes.push(r.len()? as i64);
            for v in r.try_iter()? {
                let v = v?;
                let key = v.getattr("key")?.extract()?;
                let i = match index.entry(key) {
                    std::collections::hash_map::Entry::Occupied(e) => *e.get(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let i = pos.len() as i64;
                        pos.push(xyz(&v.getattr("position")?)?.to_array());
                        e.insert(i);
                        i
                    }
                };
                cs.push(i);
            }
        }
    }
    packed(
        py,
        rows(py, pos)?,
        col(py, cs),
        col(py, sizes),
        col(py, ns),
        None,
        false,
    )
}
#[pyfunction]
fn arrays_of<'p>(mesh: &Obj<'p>) -> PyResult<Bundle<'p>> {
    let py = mesh.py();
    let d = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    let store = if mesh.get_type().name()? == "StoreMesh" {
        d.get_item("_store")?.unwrap_or_else(|| none(py))
    } else {
        none(py)
    };
    let src = d.get_item("_deferred_source")?.unwrap_or_else(|| none(py));
    let rev = if store.is_none() {
        None
    } else {
        Some(store.getattr("revision")?)
    };
    let serial = (
        d.get_item("_mut_serial")?,
        src.as_ptr() as usize,
        store.as_ptr() as usize,
        rev,
    )
        .into_bound_py_any(py)?;
    if let Some(cache) = d.get_item("_geometry_arrays")? {
        if !cache.is_none() && cache.get_item(0)?.eq(&serial)? {
            return Ok(cache.get_item(1)?.cast_into::<Arrays>()?);
        }
    }
    let a = if !src.is_none() {
        let np = py.import("numpy")?;
        let p = np.call_method1("asarray", (src.get_item("pos")?,))?;
        let double = !p.getattr("dtype")?.eq(np.getattr("float32")?)?;
        let c = numpy(py, &src.get_item("rings")?, "int64")?;
        let meta = src.call_method1("get", ("meta", PyDict::new(py)))?;
        let (p, c) = if src.contains("sequential")? && !meta.contains("arrays")? {
            let (p, inv): (Obj, Obj) = py
                .import("ingetrazo_core")?
                .call_method1("deferred_weld", (p, src.get_item("sequential")?))?
                .extract()?;
            (p, inv.get_item(c)?)
        } else {
            (numpy(py, &p, "float64")?, c)
        };
        let a = packed(
            py,
            p,
            c,
            numpy(py, &src.get_item("sizes")?, "int64")?,
            numpy(py, &src.get_item("counts")?, "int64")?,
            None,
            double,
        )?;
        let counts = measures::edge_counts(&a)?;
        a.borrow_mut().edge_face_counts = Some(counts.unbind());
        a
    } else if !store.is_none() {
        py.import("core.store_mesh")?
            .call_method1("faces_arrays", (mesh,))?
            .cast_into::<Arrays>()?
    } else {
        let a = faces_arrays(&mesh.getattr("faces")?)?;
        let counts = mesh
            .getattr("edges")?
            .try_iter()?
            .map(|e| Ok(e?.getattr("faces")?.len()? as i64))
            .collect::<PyResult<_>>()?;
        a.borrow_mut().edge_face_counts = Some(col(py, counts).unbind());
        a
    };
    d.set_item("_geometry_arrays", (serial, &a))?;
    Ok(a)
}
#[pyfunction]
fn face_attributes<'p>(mesh: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let py = mesh.py();
    let ty = mesh.get_type();
    let (table, keys) = if ty.is(&py.import("core.mesh")?.getattr("DeferredMesh")?) {
        let src = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
        let json = py.import("formats.igz")?.getattr("_face_attrs_from_json")?;
        let attrs = src.get_item("meta")?.call_method1("get", ("face_attrs",))?;
        let mut table = Vec::new();
        if !attrs.is_none() {
            for a in attrs.try_iter()? {
                let d = json.call1((a?,))?;
                table.push(if d.is_truthy()? {
                    d
                } else {
                    PyDict::new(py).into_any()
                });
            }
        }
        (table, indices(&src.get_item("face_attr")?)?)
    } else if ty.name()? == "StoreMesh" {
        let (table, keys) = crate::kernel::store_views::attributes::attr_columns(mesh, None, None)?;
        (table.extract()?, indices(&keys)?)
    } else {
        return mesh
            .getattr("faces")?
            .try_iter()?
            .map(|f| f?.getattr("attrs"))
            .collect();
    };
    keys.into_iter()
        .map(|i| {
            if i < 0 {
                Ok(PyDict::new(py).into_any())
            } else {
                table
                    .get(i as usize)
                    .cloned()
                    .ok_or_else(|| PyValueError::new_err("unknown face attribute row"))
            }
        })
        .collect()
}
#[pyfunction]
fn recorded_arrays<'p>(faces: &Obj<'p>) -> PyResult<Bundle<'p>> {
    let py = faces.py();
    let (mut pos, mut sizes, mut ns) = (vec![], vec![], vec![]);
    for f in faces.try_iter()? {
        let f = f?;
        let mut rings = vec![f.get_item("outer")?];
        if let Some(h) = f.cast::<PyDict>()?.get_item("holes")? {
            rings.extend(h.try_iter()?.collect::<PyResult<Vec<_>>>()?);
        }
        ns.push(rings.len() as i64);
        for r in rings {
            sizes.push(r.len()? as i64);
            for p in r.try_iter()? {
                pos.push(xyz(&p?)?.to_array());
            }
        }
    }
    let cs = col(py, (0..pos.len() as i64).collect());
    let a = packed(
        py,
        rows(py, pos)?,
        cs,
        col(py, sizes),
        col(py, ns),
        None,
        false,
    )?;
    placements::join(py, &[a], true)
}
#[pyfunction(signature=(mesh,indices,matrix=None))]
fn selected_arrays<'p>(
    mesh: &Obj<'p>,
    indices: &Obj<'p>,
    matrix: Option<&Obj>,
) -> PyResult<Bundle<'p>> {
    let py = mesh.py();
    let a = arrays_of(mesh)?;
    let double = a.borrow().model_f64;
    let np = py.import("numpy")?;
    let counts = a.getattr("ring_counts")?;
    let sizes = a.getattr("ring_sizes")?;
    let mask = np.call_method1("zeros", (counts.len()?, "bool"))?;
    mask.set_item(indices, true)?;
    let rings = np.call_method1("repeat", (&mask, &counts))?;
    let corners = np.call_method1("repeat", (&rings, &sizes))?;
    let pos = a
        .getattr("pos")?
        .get_item(a.getattr("corners")?.get_item(corners)?)?;
    let pos = if let Some(m) = matrix {
        placements::transformed(&pos, &placements::affine(m)?, double)?
    } else {
        pos
    };
    let c = col(py, (0..pos.len()? as i64).collect());
    let selected = packed(
        py,
        pos,
        c,
        sizes.get_item(rings)?,
        counts.get_item(mask)?,
        None,
        double,
    )?;
    placements::join(py, &[selected], true)
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<Arrays>()?;
    for f in [
        wrap_pyfunction!(faces_arrays, m)?,
        wrap_pyfunction!(arrays_of, m)?,
        wrap_pyfunction!(face_attributes, m)?,
        wrap_pyfunction!(recorded_arrays, m)?,
        wrap_pyfunction!(selected_arrays, m)?,
        wrap_pyfunction!(placements::owned_placements, m)?,
        wrap_pyfunction!(placements::world_arrays, m)?,
        wrap_pyfunction!(placements::placed_arrays, m)?,
        wrap_pyfunction!(placements::merged, m)?,
        wrap_pyfunction!(placements::assembled, m)?,
        wrap_pyfunction!(placements::placement_quantities, m)?,
        wrap_pyfunction!(measures::measurements, m)?,
        wrap_pyfunction!(measures::quantities, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
