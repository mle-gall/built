//! Group and component commands use the history of the selected edit context.
use super::model_edit::{matching, translated};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
    PyTraverseError, PyVisit,
};

#[pyfunction]
fn can_make_here(vp: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = vp.py();
    if py
        .import("core.outliner")?
        .call_method1("editing_inside_component", (vp.getattr("scene")?,))?
        .is_truthy()?
    {
        vp.call_method1(
            "flash_status",
            (
                translated(
                    py,
                    "Components keep their parts: Make Unique first to change what is inside one",
                    None,
                )?,
                4000,
            ),
        )?;
        return Ok(false);
    }
    Ok(true)
}
#[pyfunction]
fn execute_make(vp: &Bound<'_, PyAny>, cmd: &Bound<'_, PyAny>) -> PyResult<()> {
    let group = vp.getattr("scene")?.getattr("edit_group")?;
    let cmd = if group.is_none() {
        cmd.clone()
    } else {
        vp.py()
            .import("ingetrazo_core.history")?
            .call_method1("GroupInContextCommand", (cmd, group))?
    };
    vp.getattr("history")?.call_method1("execute", (cmd,))?;
    Ok(())
}

#[pyfunction]
fn make_group(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    if !win.call_method0("_can_make_here")?.is_truthy()? {
        return Ok(());
    }
    let vp = win.getattr("viewport")?;
    let sel = vp.getattr("scene")?.getattr("selection")?;
    let faces = matching(&sel, "ingetrazo_core.mesh", "Face")?;
    let edges = matching(&sel, "ingetrazo_core.mesh", "Edge")?;
    let groups = matching(&sel, "core.group", "Group")?;
    let loose = !edges.is_empty() || !faces.is_empty();
    let hist = py.import("ingetrazo_core.history")?;
    let fail = if loose {
        None
    } else {
        match groups.len() {
            0 => Some("Select the geometry to group first"),
            1 => Some(
                "That is already a group — select something else with it to group them together",
            ),
            _ => None,
        }
    };
    if let Some(tip) = fail {
        vp.call_method1("flash_status", (translated(py, tip, None)?, 4000))?;
        return Ok(());
    }
    if groups.is_empty() {
        win.call_method1(
            "_execute_make",
            (hist.call_method1("MakeGroupCommand", (faces, edges))?,),
        )?;
    } else {
        let kw = PyDict::new(py);
        kw.set_item("n", groups.len())?;
        kw.set_item(
            "extra",
            if loose {
                translated(py, " and the loose geometry", None)?.extract::<String>()?
            } else {
                String::new()
            },
        )?;
        win.call_method1(
            "_execute_make",
            (hist.call_method1("MakeNestedGroupCommand", (faces, edges, groups))?,),
        )?;
        vp.call_method1(
            "flash_status",
            (
                translated(
                    py,
                    "Grouped {n} group(s){extra} — double-click to go inside",
                    Some(&kw),
                )?,
                4000,
            ),
        )?;
    }
    vp.call_method0("update")?;
    Ok(())
}
#[pyfunction]
fn explode_group(win: &Bound<'_, PyAny>) -> PyResult<()> {
    if !win.call_method0("_can_make_here")?.is_truthy()? {
        return Ok(());
    }
    let vp = win.getattr("viewport")?;
    let sel = vp.getattr("scene")?.getattr("selection")?;
    let groups = matching(&sel, "core.group", "Group")?;
    if groups.is_empty() {
        if !matching(&sel, "core.image_plane", "ImagePlane")?.is_empty() {
            win.call_method0("_on_explode_image")?;
        }
    } else {
        let cmd = win
            .py()
            .import("ingetrazo_core.history")?
            .getattr("ExplodeGroupCommand")?;
        for g in groups.iter() {
            win.call_method1("_execute_make", (cmd.call1((g,))?,))?;
        }
        vp.call_method0("update")?;
    }
    Ok(())
}
#[pyfunction]
fn explode_images(vp: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = vp.py();
    let scene = vp.getattr("scene")?;
    let sel = scene.getattr("selection")?;
    let images = PyList::empty(py);
    let groups = PyList::empty(py);
    for im in matching(&sel, "core.image_plane", "ImagePlane")?.iter() {
        let group = im.call_method0("as_group")?;
        if !group.is_none() {
            images.append(im)?;
            groups.append(group)?;
        }
    }
    if groups.is_empty() {
        return Ok(());
    }
    let history = py.import("ingetrazo_core.history")?;
    let commands = PyList::empty(py);
    commands.append(history.call_method1("DeleteImagePlanesCommand", (images,))?)?;
    for g in groups.iter() {
        commands.append(history.call_method1("InsertGroupCommand", (g,))?)?;
    }
    vp.getattr("history")?.call_method1(
        "execute",
        (history.call_method1("CompoundCommand", (commands,))?,),
    )?;
    sel.call_method1("update", (groups,))?;
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    vp.call_method0("update")?;
    Ok(())
}

enum Component {
    Geometry,
    Group,
    Assembly,
}
#[pyclass(unsendable)]
struct ComponentName {
    mode: Component,
    win: Py<PyAny>,
    scene: Py<PyAny>,
    context: Py<PyAny>,
    faces: Py<PyList>,
    edges: Py<PyList>,
    groups: Py<PyList>,
    fallback: String,
}
#[pymethods]
impl ComponentName {
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.win)?;
        visit.call(&self.scene)?;
        visit.call(&self.context)?;
        visit.call(&self.faces)?;
        visit.call(&self.edges)?;
        visit.call(&self.groups)
    }

    #[pyo3(signature=(name=None))]
    fn __call__(&self, py: Python<'_>, name: Option<String>) -> PyResult<()> {
        let Some(name) = name else { return Ok(()) };
        let win = self.win.bind(py);
        let vp = win.getattr("viewport")?;
        let scene = vp.getattr("scene")?;
        if !scene.is(self.scene.bind(py)) || !scene.getattr("edit_group")?.is(self.context.bind(py))
        {
            return Ok(());
        }
        let name = name.trim();
        let history = py.import("ingetrazo_core.history")?;
        let kw = PyDict::new(py);
        let cmd = match self.mode {
            Component::Group => history.call_method1(
                "GroupToComponentCommand",
                (
                    self.groups.bind(py).get_item(0)?,
                    (!name.is_empty()).then_some(name),
                ),
            )?,
            Component::Assembly | Component::Geometry => {
                kw.set_item(
                    "name",
                    if name.is_empty() {
                        self.fallback.as_str()
                    } else {
                        name
                    },
                )?;
                if matches!(self.mode, Component::Assembly) {
                    history.getattr("MakeComponentOfCommand")?.call(
                        (
                            self.faces.bind(py),
                            self.edges.bind(py),
                            self.groups.bind(py),
                        ),
                        Some(&kw),
                    )?
                } else {
                    kw.set_item("component", true)?;
                    history
                        .getattr("MakeGroupCommand")?
                        .call((self.faces.bind(py), self.edges.bind(py)), Some(&kw))?
                }
            }
        };
        win.call_method1("_execute_make", (cmd,))?;
        vp.call_method0("update")?;
        win.call_method0("statusBar")?.call_method1(
            "showMessage",
            (
                translated(
                    py,
                    "Component created — copies will share its definition",
                    None,
                )?,
                4000,
            ),
        )?;
        Ok(())
    }
}
#[pyfunction]
fn make_component(win: &Bound<'_, PyAny>) -> PyResult<()> {
    if !win.call_method0("_can_make_here")?.is_truthy()? {
        return Ok(());
    }
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let sel = scene.getattr("selection")?;
    let faces = matching(&sel, "ingetrazo_core.mesh", "Face")?;
    let edges = matching(&sel, "ingetrazo_core.mesh", "Edge")?;
    let groups = PyList::empty(py);
    for g in matching(&sel, "core.group", "Group")?.iter() {
        if !g.getattr("billboard")?.is_truthy()? {
            groups.append(g)?;
        }
    }
    let loose = !faces.is_empty() || !edges.is_empty();
    let mode = match (groups.len(), loose) {
        (0, false) => {
            vp.call_method1(
                "flash_status",
                (translated(
                    py,
                    "Select the geometry for the component first",
                    None,
                )?,),
            )?;
            return Ok(());
        }
        (0, true) => Component::Geometry,
        (1, false) => Component::Group,
        _ => Component::Assembly,
    };
    let suggestion: String = if matches!(mode, Component::Group) {
        let g = groups.get_item(0)?;
        if g.call_method0("is_component")?.is_truthy()? {
            vp.call_method1(
                "flash_status",
                (translated(py, "It is a component already", None)?,),
            )?;
            return Ok(());
        }
        let name = g.getattr("name")?;
        if name.is_truthy()? {
            name.extract()?
        } else {
            translated(py, "Component", None)?.extract()?
        }
    } else {
        let taken = py
            .import("core.kept_components")?
            .call_method1("names_in_use", (&scene,))?;
        let kw = PyDict::new(py);
        let mut n = 1;
        loop {
            kw.set_item("n", n)?;
            let name = translated(py, "Component #{n}", Some(&kw))?;
            if !taken.contains(&name)? {
                break name.extract()?;
            }
            n += 1;
        }
    };
    let kw = PyDict::new(py);
    kw.set_item("text", &suggestion)?;
    let done = ComponentName {
        mode,
        win: win.clone().unbind(),
        context: scene.getattr("edit_group")?.unbind(),
        scene: scene.unbind(),
        faces: faces.unbind(),
        edges: edges.unbind(),
        groups: groups.unbind(),
        fallback: suggestion,
    };
    kw.set_item("done", Py::new(py, done)?)?;
    py.import("ingetrazo_render.chrome_forms")?.getattr("ask_text")?.call(
        (
            win,
            translated(py, "Make Component", None)?,
            translated(py, "Component name:", None)?,
        ),
        Some(&kw),
    )?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(can_make_here, m)?)?;
    m.add_function(wrap_pyfunction!(execute_make, m)?)?;
    m.add_function(wrap_pyfunction!(make_group, m)?)?;
    m.add_function(wrap_pyfunction!(make_component, m)?)?;
    m.add_function(wrap_pyfunction!(explode_group, m)?)?;
    m.add_function(wrap_pyfunction!(explode_images, m)?)?;
    Ok(())
}
