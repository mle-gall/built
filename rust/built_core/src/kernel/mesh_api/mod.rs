// SPDX-License-Identifier: GPL-3.0-or-later
//! Public mesh classes. Heap classes retain Python subclass layouts and archive names.
use pyo3::{
    exceptions::{PyKeyError, PyOverflowError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyModule, PySet, PyTuple},
};
type O<'p> = Bound<'p, PyAny>;
type M<'p> = Bound<'p, PyModule>;
type D<'p> = Bound<'p, PyDict>;
mod elements;
mod mesh;
mod records;
mod views;

fn module<'p>(py: Python<'p>, name: &str) -> PyResult<O<'p>> {
    crate::model::module(py, name)
}
fn api(py: Python<'_>) -> PyResult<O<'_>> {
    module(py, "built_core.mesh")
}
fn store_api(py: Python<'_>) -> PyResult<O<'_>> {
    module(py, "built_core.store_mesh")
}
fn kernel(py: Python<'_>) -> PyResult<O<'_>> {
    module(py, "built_core")?.getattr("store_views")
}
fn builtin<'p>(py: Python<'p>, name: &str) -> PyResult<O<'p>> {
    module(py, "builtins")?.getattr(name)
}
fn list<'p>(o: &O<'p>) -> PyResult<O<'p>> {
    o.py().get_type::<PyList>().call1((o,))
}
fn none(py: Python<'_>) -> O<'_> {
    py.None().into_bound(py)
}
fn slots(ns: &D<'_>, names: &str) -> PyResult<()> {
    ns.set_item(
        "__slots__",
        PyTuple::new(ns.py(), names.split_whitespace().collect::<Vec<_>>())?,
    )
}
fn class<'p>(m: &M<'p>, name: &str, bases: &[O<'p>], ns: &D<'p>) -> PyResult<O<'p>> {
    ns.set_item("__module__", m.name()?)?;
    let ty = builtin(m.py(), "type")?.call1((name, PyTuple::new(m.py(), bases)?, ns))?;
    m.add(name, &ty)?;
    Ok(ty)
}
/// Bind native callables to heap classes while keeping the plugin's subclass layout.
#[pyclass(frozen)]
struct MeshMethod {
    function: Py<PyAny>,
    method_type: Py<PyAny>,
}
#[pymethods]
impl MeshMethod {
    #[pyo3(signature=(*args,**kwargs))]
    fn __call__<'p>(&self, args: &Bound<'p, PyTuple>, kwargs: Option<&D<'p>>) -> PyResult<O<'p>> {
        self.function.bind(args.py()).call(args, kwargs)
    }
    fn __get__<'p>(
        &self,
        py: Python<'p>,
        instance: Option<&O<'p>>,
        _owner: Option<&O<'p>>,
    ) -> PyResult<O<'p>> {
        let func = self.function.bind(py);
        match instance {
            Some(obj) => self.method_type.bind(py).call1((func, obj)),
            None => Ok(func.clone()),
        }
    }
}
fn bind(ns: &D<'_>, name: &str, func: &O<'_>) -> PyResult<()> {
    let method_type = module(ns.py(), "types")?.getattr("MethodType")?.unbind();
    ns.set_item(
        name,
        Bound::new(
            ns.py(),
            MeshMethod {
                function: func.clone().unbind(),
                method_type,
            },
        )?,
    )
}
fn property(ns: &D<'_>, name: &str, get: &O<'_>, set: Option<&O<'_>>) -> PyResult<()> {
    ns.set_item(name, builtin(ns.py(), "property")?.call1((get, set))?)
}
fn partial<'p>(func: &O<'p>, arg: impl IntoPyObject<'p>) -> PyResult<O<'p>> {
    module(func.py(), "functools")?.call_method1("partial", (func, arg))
}
macro_rules! method {
    ($ns:expr,$m:expr,$name:literal,$f:ident) => {
        bind(&$ns, $name, wrap_pyfunction!($f, $m)?.as_any())?
    };
}
macro_rules! getter {
    ($ns:expr,$m:expr,$name:literal,$f:ident) => {
        property(&$ns, $name, wrap_pyfunction!($f, $m)?.as_any(), None)?
    };
}
use {getter, method};
fn views_list<'p>(mesh: &O<'p>, kind: &str, keys: &O<'p>) -> PyResult<O<'p>> {
    let cls = store_api(mesh.py())?.getattr(kind)?;
    let result = PyList::empty(mesh.py());
    for key in keys.try_iter()? {
        result.append(cls.call1((mesh, key?))?)?;
    }
    Ok(result.into_any())
}
pub(crate) fn register(core: &M<'_>) -> PyResult<()> {
    let py = core.py();
    let m = PyModule::new(py, "built_core.mesh")?;
    let s = PyModule::new(py, "built_core.store_mesh")?;
    let k = core.getattr("store_views")?;
    elements::install(&m, core, &k)?;
    views::install(&s, &m, core, &k)?;
    records::install(&s, core, &k)?;
    mesh::install(&s, &m, core, &k)?;
    m.add("Mesh", s.getattr("StoreMesh")?)?;
    m.add("DeferredMesh", s.getattr("DeferredMesh")?)?;
    let modules = py.import("sys")?.getattr("modules")?;
    // Existing plugin archives name the original mesh/view classes and rebuild functions.
    let archive = PyModule::new(py, "core.store_mesh")?;
    for name in [
        "StoreMesh",
        "DeferredMesh",
        "VertexView",
        "EdgeView",
        "FaceView",
        "instance_view",
        "_unpickle_mesh",
        "_unpickle_view",
    ] {
        archive.add(name, s.getattr(name)?)?;
    }
    modules.set_item("core.store_mesh", archive)?;
    for (name, v) in [("mesh", m), ("store_mesh", s)] {
        modules.set_item(v.name()?, &v)?;
        core.add(name, v)?;
    }
    Ok(())
}
