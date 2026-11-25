// SPDX-License-Identifier: GPL-3.0-or-later
//! Mesh scans indexed by store identity and revision. Cache entries hold a
//! weak reference so eviction never determines a document's lifetime.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
fn module(py: Python) -> PyResult<Bound<PyModule>> {
    py.import("built_core.scan_index")
}
#[pyfunction]
fn enabled(py: Python) -> PyResult<bool> {
    Ok(
        std::env::var_os("BUILT_SCANS").as_deref() != Some(std::ffi::OsStr::new("0"))
            && !module(py)?.getattr("_forced_off")?.is_truthy()?,
    )
}
#[pyfunction]
fn store_backed(mesh: &Obj) -> PyResult<bool> {
    mesh.is_instance(
        &mesh
            .py()
            .import("built_core.store_mesh")?
            .getattr("StoreMesh")?,
    )
}
fn list<'p>(iter: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    PyList::new(iter.py(), iter.try_iter()?.collect::<PyResult<Vec<_>>>()?)
}
#[pyclass(module = "built_core.scan_index")]
struct Scan {
    #[pyo3(get)]
    native: Py<PyAny>,
    mesh: Option<Py<PyAny>>,
    #[pyo3(get)]
    store_backed: bool,
    vectors: Py<PyDict>,
    keys: Py<PyDict>,
}
impl Scan {
    fn wrap(native: Obj, mesh: Option<Py<PyAny>>, store_backed: bool) -> Self {
        let py = native.py();
        Self {
            native: native.unbind(),
            mesh,
            store_backed,
            vectors: PyDict::new(py).unbind(),
            keys: PyDict::new(py).unbind(),
        }
    }
    fn elements<'p>(&self, kind: &str, class: &str, ids: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = ids.py();
        let ids = list(ids)?;
        if ids.is_empty() {
            return Ok(ids.into_any());
        }
        let got = self.native.bind(py).call_method1("keys_at", (kind, ids))?;
        if !self.store_backed {
            return Ok(got);
        }
        let mesh = self.mesh.as_ref().unwrap().bind(py).call0()?;
        let cls = py.import("built_core.store_mesh")?.getattr(class)?;
        let out = PyList::empty(py);
        for key in got.try_iter()? {
            out.append(cls.call1((&mesh, key?))?)?;
        }
        Ok(out.into_any())
    }
    fn query<'p>(&self, method: &str, b: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.native
            .bind(b.py())
            .call_method1(method, (b.get_item(0)?, b.get_item(1)?))
    }
}
#[pymethods]
impl Scan {
    #[new]
    fn new(mesh: &Obj) -> PyResult<Self> {
        let py = mesh.py();
        let cls = py.import("built_core")?.getattr("MeshScan")?;
        let backed = store_backed(mesh)?;
        let native = if backed {
            cls.call_method1(
                "from_store",
                (mesh.getattr("__dict__")?.get_item("_store")?,),
            )?
        } else {
            cls.call_method1(
                "from_objects",
                (
                    list(&mesh.getattr("vertices")?)?,
                    list(&mesh.getattr("edges")?)?,
                    list(&mesh.getattr("faces")?)?,
                ),
            )?
        };
        Ok(Self::wrap(
            native,
            Some(py.import("weakref")?.call_method1("ref", (mesh,))?.unbind()),
            backed,
        ))
    }
    #[staticmethod]
    fn of_faces(module: &Obj, faces: &Obj) -> PyResult<Self> {
        let empty = PyList::empty(faces.py());
        Ok(Self::wrap(
            module
                .getattr("MeshScan")?
                .call_method1("from_objects", (&empty, &empty, list(faces)?))?,
            None,
            false,
        ))
    }
    fn faces<'p>(&self, idx: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.elements("face", "FaceView", idx)
    }
    fn edges<'p>(&self, idx: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.elements("edge", "EdgeView", idx)
    }
    fn vertices<'p>(&self, idx: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.elements("vertex", "VertexView", idx)
    }
    fn face_indices<'p>(&self, faces: &Obj<'p>) -> PyResult<Obj<'p>> {
        faces.py().import("built_core")?.call_method1(
            "scan_face_indices",
            (self.native.bind(faces.py()), list(faces)?),
        )
    }
    fn faces_meeting<'p>(&self, b: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.query("faces_meeting", b)
    }
    fn faces_within<'p>(&self, b: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.query("faces_within", b)
    }
    fn edges_meeting<'p>(&self, b: &Obj<'p>) -> PyResult<Obj<'p>> {
        self.query("edges_meeting", b)
    }
    fn faces_holding<'p>(&self, b: &Obj<'p>, pad: f64) -> PyResult<Obj<'p>> {
        self.native
            .bind(b.py())
            .call_method1("faces_holding", (b.get_item(0)?, b.get_item(1)?, pad))
    }
    fn vec<'p>(&self, py: Python<'p>, v: usize) -> PyResult<Obj<'p>> {
        let dict = self.vectors.bind(py);
        if let Some(hit) = dict.get_item(v)? {
            return Ok(hit);
        }
        let pos = self.native.bind(py).call_method1("position", (v,))?;
        let hit = py
            .import("built_core")?
            .getattr("Vec3")?
            .call1(pos.cast::<PyTuple>()?)?;
        dict.set_item(v, &hit)?;
        Ok(hit)
    }
    fn tkey<'p>(&self, py: Python<'p>, v: usize) -> PyResult<Obj<'p>> {
        let dict = self.keys.bind(py);
        if let Some(hit) = dict.get_item(v)? {
            return Ok(hit);
        }
        let hit = py
            .import("core.topology")?
            .call_method1("_key", (self.vec(py, v)?,))?;
        dict.set_item(v, &hit)?;
        Ok(hit)
    }
    fn edges_ending_at<'p>(&self, py: Python<'p>, key: [f64; 3]) -> PyResult<Obj<'p>> {
        self.native.bind(py).call_method1(
            "edges_at_cell",
            (PyTuple::new(
                py,
                key.map(|v| (v * 1e4).round_ties_even() as i64),
            )?,),
        )
    }
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.native)?;
        visit.call(&self.mesh)?;
        visit.call(&self.keys)?;
        visit.call(&self.vectors)
    }
}
#[pyfunction]
#[pyo3(signature=(mesh,fresh=false))]
fn scan<'p>(mesh: &Obj<'p>, fresh: bool) -> PyResult<Option<Obj<'p>>> {
    let py = mesh.py();
    if mesh.is_none() || !enabled(py)? {
        return Ok(None);
    }
    if mesh
        .get_type()
        .is(py.import("built_core.mesh")?.getattr("DeferredMesh")?)
    {
        mesh.getattr("faces")?.len()?;
    }
    let token = if store_backed(mesh)? {
        let st = mesh.getattr("__dict__")?.get_item("_store")?;
        (
            st.as_ptr() as usize,
            st.getattr("lineage")?,
            st.getattr("revision")?,
        )
            .into_pyobject(py)?
            .into_any()
    } else {
        (
            mesh.get_type(),
            mesh.getattr("revision")?,
            mesh.getattr("vertices")?.len()?,
            mesh.getattr("edges")?.len()?,
            mesh.getattr("faces")?.len()?,
        )
            .into_pyobject(py)?
            .into_any()
    };
    let cache = module(py)?.getattr("_cache")?;
    let id = mesh.as_ptr() as usize;
    let hit = cache.call_method1("get", (id,))?;
    if !fresh
        && !hit.is_none()
        && hit.get_item(0)?.call0()?.is(mesh)
        && hit.get_item(1)?.eq(&token)?
    {
        cache.call_method1("move_to_end", (id,))?;
        return Ok(Some(hit.get_item(2)?));
    }
    let scan = Bound::new(py, Scan::new(mesh)?)?;
    cache.set_item(
        id,
        (scan.borrow().mesh.as_ref().unwrap().bind(py), token, &scan),
    )?;
    cache.call_method1("move_to_end", (id,))?;
    if cache.len()? > 4 {
        cache.call_method1("popitem", (false,))?;
    }
    Ok(Some(scan.into_any()))
}
#[pyfunction]
#[pyo3(signature=(mesh=None))]
fn forget(py: Python, mesh: Option<&Obj>) -> PyResult<()> {
    let cache = module(py)?.getattr("_cache")?;
    match mesh {
        None => cache.call_method0("clear")?,
        Some(m) => cache.call_method1("pop", (m.as_ptr() as usize, py.None()))?,
    };
    Ok(())
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let name = "built_core.scan_index";
    let py = m.py();
    let sub = PyModule::new(py, name)?;
    sub.add("_forced_off", false)?;
    sub.add(
        "_cache",
        py.import("collections")?.call_method0("OrderedDict")?,
    )?;
    sub.add_class::<Scan>()?;
    for f in [
        wrap_pyfunction!(scan, &sub)?,
        wrap_pyfunction!(enabled, &sub)?,
        wrap_pyfunction!(store_backed, &sub)?,
        wrap_pyfunction!(forget, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    for key in ["box_of", "raw_box"] {
        sub.add(key, m.getattr("push_queries")?.getattr(key)?)?;
    }
    py.import("sys")?.getattr("modules")?.set_item(name, &sub)?;
    m.add("scan_index", sub)
}
