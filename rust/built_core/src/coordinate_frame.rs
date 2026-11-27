// SPDX-License-Identifier: GPL-3.0-or-later
//! Temporary local coordinate systems for precision-sensitive mesh edits.
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
const MODULE: &str = "built_core.coordinate_frame";
const ZERO: (f64, f64, f64) = (0., 0., 0.);
fn tuple<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    v.py().get_type::<PyTuple>().call1((v,))
}
fn matrix_tuple<'p>(v: Obj<'p>) -> PyResult<Obj<'p>> {
    if v.is_none() {
        Ok(v)
    } else {
        tuple(&v)
    }
}
fn active(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import(MODULE)?.getattr("_active")
}
#[pyfunction(signature=(center,previous=[0.;3]))]
fn choose_origin(center: [f64; 3], previous: [f64; 3]) -> (f64, f64, f64) {
    crate::resident::assemble::choose_origin(center, previous).into()
}
#[pyfunction]
fn point_in_frame(point: &Obj, origin: &Obj) -> PyResult<crate::math::Vec3> {
    Ok((crate::math::xyz(point)? - crate::math::xyz(origin)?).into())
}
#[pyfunction]
fn drawing_point<'p>(mesh: &Obj<'p>, point: &Obj<'p>) -> PyResult<Obj<'p>> {
    let v = mesh.call_method1("vertex_at", (point,))?;
    let coords = if v.is_none() {
        point.call_method0("toTuple")?
    } else {
        v.call_method0("position_tuple")?
    };
    mesh.py().import("numpy")?.call_method1("array", (coords,))
}
fn matches(store: &Obj, origin: &Obj, matrix: &Obj) -> PyResult<bool> {
    Ok(tuple(&store.getattr("frame_origin")?)?.eq(origin)?
        && matrix_tuple(store.getattr("frame_matrix")?)?.eq(matrix)?)
}
struct Entered {
    store: Py<PyAny>,
    token: Py<PyAny>,
    frame: Py<PyAny>,
    parent: Option<Py<PyAny>>,
    revision: Option<Py<PyAny>>,
}
#[pyclass(unsendable)]
struct MeshFrame {
    mesh: Py<PyAny>,
    origin: Py<PyAny>,
    matrix: Option<Py<PyAny>>,
    entered: Option<Entered>,
}
#[pymethods]
impl MeshFrame {
    fn __enter__(&mut self, py: Python) -> PyResult<()> {
        let mesh = self.mesh.bind(py);
        let d = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
        let Some(st) = d.get_item("_store")?.filter(|s| !s.is_none()) else {
            return Ok(());
        };
        let origin = self.origin.bind(py);
        let matrix = matrix_tuple(match &self.matrix {
            Some(m) => m.bind(py).clone(),
            None => st.getattr("frame_matrix")?,
        })?;
        if matches(&st, origin, &matrix)? {
            return Ok(());
        }
        let definition = d.get_item("_instance_definition")?.filter(|v| !v.is_none());
        let parent = d.get_item("_frame_parent")?.filter(|p| !p.is_none());
        let frame = (origin, &matrix).into_pyobject(py)?;
        let local = if let Some(def) = &definition {
            def.getattr("_store")?.call_method1(
                "_instance_view",
                (def, &matrix, origin, d.contains("_instance_edit")?),
            )?
        } else if let Some(p) = parent.as_ref() {
            if matches(p, origin, &matrix)? {
                if !p.getattr("revision")?.eq(st.getattr("revision")?)? {
                    p.call_method1("_publish_frame", (&st,))?;
                }
                p.clone()
            } else {
                local_store(&d, &st, origin, &matrix, &frame)?
            }
        } else {
            local_store(&d, &st, origin, &matrix, &frame)?
        };
        let ctx = active(py)?;
        let mut meshes = Vec::new();
        for m in ctx.call_method0("get")?.try_iter()? {
            let m = m?;
            if !m.is(mesh) {
                meshes.push(m);
            }
        }
        if origin.extract::<[f64; 3]>()?.iter().any(|n| *n != 0.)
            || (definition.is_none() && !matrix.is_none())
        {
            meshes.push(mesh.clone());
        }
        let revision = if definition.is_none() {
            Some(local.getattr("revision")?.unbind())
        } else {
            None
        };
        let token = ctx.call_method1("set", (PyTuple::new(py, meshes)?,))?;
        if revision.is_some() {
            d.set_item("_frame_parent", &st)?;
        }
        d.set_item("_store", local)?;
        self.entered = Some(Entered {
            store: st.unbind(),
            token: token.unbind(),
            revision,
            parent: parent.map(Bound::unbind),
            frame: frame.into_any().unbind(),
        });
        Ok(())
    }
    fn __exit__(&mut self, py: Python, _ty: &Obj, _value: &Obj, _tb: &Obj) -> PyResult<()> {
        let Some(state) = self.entered.take() else {
            return Ok(());
        };
        active(py)?.call_method1("reset", (state.token,))?;
        let d = self.mesh.bind(py).getattr("__dict__")?;
        let st = state.store.bind(py);
        let local = d.get_item("_store")?;
        d.set_item("_store", st)?;
        if let Some(start) = state.revision {
            if let Some(p) = state.parent {
                d.set_item("_frame_parent", p)?;
            } else {
                d.call_method1("pop", ("_frame_parent", py.None()))?;
            }
            let rev = local.getattr("revision")?;
            if !rev.eq(start)? && !rev.eq(st.getattr("revision")?)? {
                st.call_method1("_publish_frame", (&local,))?;
            }
            d.set_item(
                "_tool_frame_cache",
                (st, st.getattr("revision")?, state.frame, local),
            )?;
        }
        Ok(())
    }
}
fn local_store<'p>(
    d: &Bound<'p, PyDict>,
    st: &Obj<'p>,
    origin: &Obj<'p>,
    matrix: &Obj<'p>,
    frame: &Bound<'p, PyTuple>,
) -> PyResult<Obj<'p>> {
    if let Some(c) = d.get_item("_tool_frame_cache")?.filter(|c| !c.is_none()) {
        if c.get_item(0)?.is(st)
            && c.get_item(1)?.eq(st.getattr("revision")?)?
            && c.get_item(2)?.eq(frame)?
        {
            return c.get_item(3);
        }
    }
    if matrix.is_none() {
        st.call_method1("_in_model_frame", (origin,))
    } else {
        st.call_method1("_in_instance_frame", (matrix, origin))
    }
}
#[pyfunction(signature=(mesh,origin,*,matrix=Python::attach(|py|py.Ellipsis())))]
fn mesh_frame(py: Python, mesh: &Obj, origin: &Obj, matrix: Py<PyAny>) -> PyResult<MeshFrame> {
    Ok(MeshFrame {
        mesh: mesh.clone().unbind(),
        origin: tuple(origin)?.unbind(),
        matrix: (!matrix.bind(py).is(&py.Ellipsis())).then_some(matrix),
        entered: None,
    })
}
#[pyclass(unsendable)]
struct ModelFrame {
    stack: Option<Py<PyAny>>,
}
#[pymethods]
impl ModelFrame {
    fn __enter__(&mut self, py: Python) -> PyResult<()> {
        let stack = py.import("contextlib")?.call_method0("ExitStack")?;
        let result = (|| -> PyResult<()> {
            for mesh in active(py)?.call_method0("get")?.try_iter()? {
                let frame = MeshFrame {
                    mesh: mesh?.unbind(),
                    origin: ZERO.into_pyobject(py)?.into_any().unbind(),
                    matrix: None,
                    entered: None,
                };
                stack.call_method1("enter_context", (frame,))?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            stack.call_method0("close")?;
            return Err(e);
        }
        self.stack = Some(stack.unbind());
        Ok(())
    }
    fn __exit__(&mut self, py: Python, t: &Obj, v: &Obj, tb: &Obj) -> PyResult<()> {
        if let Some(s) = self.stack.take() {
            s.call_method1(py, "__exit__", (t, v, tb))?;
        }
        Ok(())
    }
}
#[pyfunction]
fn model_frame() -> ModelFrame {
    ModelFrame { stack: None }
}
#[pyclass(unsendable)]
struct PlannedPoints {
    mesh: Py<PyAny>,
    points: Py<PyAny>,
    before: Option<Py<PyAny>>,
    changed: bool,
}
#[pymethods]
impl PlannedPoints {
    fn __enter__(&mut self, py: Python) -> PyResult<()> {
        let mesh = self.mesh.bind(py);
        if self.points.bind(py).is_truthy()? {
            self.before = mesh
                .getattr("__dict__")?
                .cast_into::<PyDict>()?
                .get_item("_planned_points")?
                .filter(|p| !p.is_none())
                .map(Bound::unbind);
            mesh.setattr("_planned_points", self.points.bind(py))?;
            self.changed = true;
        }
        Ok(())
    }
    fn __exit__(&mut self, py: Python, _t: &Obj, _v: &Obj, _tb: &Obj) -> PyResult<()> {
        if self.changed {
            self.changed = false;
            let mesh = self.mesh.bind(py);
            if let Some(p) = self.before.take() {
                mesh.setattr("_planned_points", p)?;
            } else {
                mesh.getattr("__dict__")?
                    .call_method1("pop", ("_planned_points", py.None()))?;
            }
        }
        Ok(())
    }
}
#[pyfunction]
fn planned_points(mesh: Py<PyAny>, points: Py<PyAny>) -> PlannedPoints {
    PlannedPoints {
        mesh,
        points,
        before: None,
        changed: false,
    }
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, MODULE)?;
    let kw = PyDict::new(py);
    kw.set_item("default", PyTuple::empty(py))?;
    sub.add(
        "_active",
        py.import("contextvars")?
            .call_method("ContextVar", ("geometry_frames",), Some(&kw))?,
    )?;
    sub.add("ZERO", ZERO)?;
    sub.add(
        "REBASE_DISTANCE",
        crate::resident::assemble::REBASE_DISTANCE,
    )?;
    for f in [
        wrap_pyfunction!(choose_origin, &sub)?,
        wrap_pyfunction!(mesh_frame, &sub)?,
        wrap_pyfunction!(model_frame, &sub)?,
        wrap_pyfunction!(planned_points, &sub)?,
        wrap_pyfunction!(drawing_point, &sub)?,
        wrap_pyfunction!(point_in_frame, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    py.import("sys")?
        .getattr("modules")?
        .set_item(MODULE, &sub)?;
    m.add("coordinate_frame", sub)
}
