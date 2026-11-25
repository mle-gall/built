//! Bridge and Make Faces validate the current profiles before creating one
//! undo entry. Redo restores the history snapshot without rerunning geometry.
use super::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

struct Operation {
    function: Py<PyAny>,
    profiles: Py<PyAny>,
    eye: Option<Py<PyAny>>,
}

#[pyclass(unsendable)]
struct ProfileMutation {
    pending: Option<Operation>,
    faces: Option<Py<PyAny>>,
    added: usize,
}
#[pymethods]
impl ProfileMutation {
    fn __call__(&mut self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = scene.py();
        let op = self.pending.take().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err("profile mutation already executed")
        })?;
        let mesh = scene.getattr("mesh")?;
        let (faces, added) = if let Some(eye) = op.eye {
            let result = op.function.bind(py).call1((mesh, op.profiles, eye))?;
            (
                result.get_item("faces")?,
                result.get_item("new")?.extract()?,
            )
        } else {
            let faces = op.function.bind(py).call1((mesh, op.profiles))?;
            let added = faces.len()?;
            (faces, added)
        };
        self.faces = Some(faces.unbind());
        self.added = added;
        Ok(())
    }
}

#[pyfunction]
fn create_profile_faces(win: &Bound<'_, PyAny>, bridge: bool) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let selection = scene.getattr("selection")?;
    let mesh = scene.getattr("mesh")?;
    let (module, select, verb, error) = if bridge {
        (
            "core.bridge",
            "profiles_from_selection",
            "bridge",
            "BridgeError",
        )
    } else {
        (
            "core.make_faces",
            "selected_edges",
            "make_faces",
            "MakeFacesError",
        )
    };
    let backend = py.import(module)?;
    let planned = backend
        .call_method1(select, (&mesh, &selection))
        .and_then(|v| {
            backend.call_method1("plan", (&v,))?;
            Ok(v)
        });
    let profiles = match planned {
        Ok(v) => v,
        Err(err) if err.matches(py, backend.getattr(error)?)? => {
            let message = err.value(py).str()?;
            vp.call_method1(
                "flash_status",
                (translated(py, &message.to_string(), None)?, 4000),
            )?;
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    let eye = if bridge {
        None
    } else {
        let eye = if let Some(camera) = vp
            .getattr("camera")
            .ok()
            .filter(|v| !v.is_none() && v.hasattr("eye").unwrap_or(false))
        {
            camera.call_method0("eye")?.unbind()
        } else {
            py.None()
        };
        Some(eye)
    };
    let mutation = Py::new(
        py,
        ProfileMutation {
            pending: Some(Operation {
                function: backend.getattr(verb)?.unbind(),
                profiles: profiles.unbind(),
                eye,
            }),
            faces: None,
            added: 0,
        },
    )?;
    let cmd = py
        .import("ingetrazo_core.history")?
        .call_method1("SnapshotMutation", (mutation.bind(py),))?;
    let history = vp.getattr("history")?;
    history.call_method1("execute", (cmd,))?;
    let mut result = mutation.borrow_mut(py);
    let Some(created) = result.faces.take() else {
        return Ok(());
    };
    let live = mesh.getattr("faces")?;
    let targets = PyList::empty(py);
    for face in created.bind(py).try_iter()? {
        let face = face?;
        if live.contains(&face)? {
            targets.append(face)?;
        }
    }
    selection.call_method0("clear")?;
    selection.call_method1("update", (&targets,))?;
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    vp.call_method0("update")?;
    let vars = PyDict::new(py);
    vars.set_item("n", result.added)?;
    let message = if bridge {
        "Bridge: {n} faces added"
    } else if result.added > 0 {
        "Make Faces: {n} faces made"
    } else if targets.is_empty() {
        "Make Faces: an existing face already covers those loops"
    } else {
        "Make Faces: those loops already had their faces"
    };
    vp.call_method1(
        "flash_status",
        (translated(py, message, Some(&vars))?, 3000),
    )?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(create_profile_faces, m)?)?;
    Ok(())
}
