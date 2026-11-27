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
        .import("built_core.outliner")?
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
            .import("built_core.history")?
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
    let faces = matching(&sel, "built_core.mesh", "Face")?;
    let edges = matching(&sel, "built_core.mesh", "Edge")?;
    let groups = matching(&sel, "built_core.group", "Group")?;
    let loose = !edges.is_empty() || !faces.is_empty();
    let hist = py.import("built_core.history")?;
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
    let groups = matching(&sel, "built_core.group", "Group")?;
    if groups.is_empty() {
        if !matching(&sel, "built_core.image_plane", "ImagePlane")?.is_empty() {
            win.call_method0("_on_explode_image")?;
        }
    } else {
        let cmd = win
            .py()
            .import("built_core.history")?
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
    for im in matching(&sel, "built_core.image_plane", "ImagePlane")?.iter() {
        let group = im.call_method0("as_group")?;
        if !group.is_none() {
            images.append(im)?;
            groups.append(group)?;
        }
    }
    if groups.is_empty() {
        return Ok(());
    }
    let history = py.import("built_core.history")?;
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
struct ComponentForm {
    mode: Component,
    win: Py<PyAny>,
    scene: Py<PyAny>,
    context: Py<PyAny>,
    faces: Py<PyList>,
    edges: Py<PyList>,
    groups: Py<PyList>,
    fallback: String,
}
// A transformed ordinary group may share its mesh with its copy. A kept
// definition must remain independent when that original group is edited.
fn detach_group_meshes(source: &Bound<'_, PyAny>, copy: &Bound<'_, PyAny>) -> PyResult<()> {
    if source.call_method0("is_component")?.is_truthy()? {
        return Ok(());
    }
    let mesh = source.getattr("mesh")?;
    if copy.getattr("mesh")?.is(&mesh) {
        let own_mesh = source.py().import("built_core.group")?.call_method1("_copy_definition_mesh", (mesh,))?;
        copy.setattr("mesh", own_mesh)?;
    }
    let children = source.getattr("children")?;
    if children.is_truthy()? {
        for (child, duplicate) in children.try_iter()?.zip(copy.getattr("children")?.try_iter()?) {
            detach_group_meshes(&child?, &duplicate?)?;
        }
    }
    Ok(())
}

#[pymethods]
impl ComponentForm {
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.win)?;
        visit.call(&self.scene)?;
        visit.call(&self.context)?;
        visit.call(&self.faces)?;
        visit.call(&self.edges)?;
        visit.call(&self.groups)
    }

    fn __call__(&self, py: Python<'_>, answer: &str, values: &Bound<'_, PyAny>) -> PyResult<()> {
        if answer != "ok" { return Ok(()); }
        let name: String = values.get_item("text")?.extract()?;
        let win = self.win.bind(py);
        let vp = win.getattr("viewport")?;
        let scene = vp.getattr("scene")?;
        if !scene.is(self.scene.bind(py)) || !scene.getattr("edit_group")?.is(self.context.bind(py))
        {
            return Ok(());
        }
        let name = name.trim();
        let name = if name.is_empty() { self.fallback.as_str() } else { name };
        let history = py.import("built_core.history")?;
        if !values.get_item("replace")?.is_truthy()? {
            let copy = py.import("built_core.group")?.getattr("copy_group")?;
            let copies = PyList::empty(py);
            for source in self.groups.bind(py).iter() {
                let duplicate = copy.call1((&source,))?;
                detach_group_meshes(&source, &duplicate)?;
                copies.append(duplicate)?;
            }
            let template = if matches!(self.mode, Component::Group) {
                copies.get_item(0)?
            } else {
                let geometry = py.import("built_core")?.call_method1(
                    "GroupGeometry", (self.faces.bind(py), self.edges.bind(py)))?;
                let group = geometry.call_method1("build", (&scene, copies.is_empty(), name))?;
                geometry.call_method1("adopt_exact", (scene.getattr("mesh")?, group.getattr("mesh")?))?;
                if !copies.is_empty() { group.call_method1("adopt", (copies,))?; }
                group
            };
            template.setattr("name", name)?;
            template.setattr("component", true)?;
            if template.getattr("xform")?.is_none() {
                template.setattr("xform", py.import("built_core")?.call_method0("Matrix4")?)?;
            }
            let cmd = history.call_method1("KeepComponentCommand", (template,))?;
            vp.getattr("history")?.call_method1("execute", (cmd,))?;
            vp.call_method0("update")?;
            return Ok(());
        }
        let kw = PyDict::new(py);
        let cmd = match self.mode {
            Component::Group => history.call_method1(
                "GroupToComponentCommand",
                (
                    self.groups.bind(py).get_item(0)?,
                    name,
                ),
            )?,
            Component::Assembly | Component::Geometry => {
                kw.set_item("name", name)?;
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
        if values.get_item("axes")?.is_truthy()? {
            let groups = matching(&scene.getattr("selection")?, "built_core.group", "Group")?;
            if groups.len() == 1 {
                win.call_method1("_on_change_axes", (groups.get_item(0)?,))?;
            }
        }
        Ok(())
    }
}
#[pyfunction]
fn placing_component(form: &Bound<'_, PyAny>) -> PyResult<bool> {
    form.call_method1("value", ("replace",))?.is_truthy()
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
    let faces = matching(&sel, "built_core.mesh", "Face")?;
    let edges = matching(&sel, "built_core.mesh", "Edge")?;
    let groups = PyList::empty(py);
    for g in matching(&sel, "built_core.group", "Group")?.iter() {
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
    let existing_name = if matches!(mode, Component::Group) {
        let g = groups.get_item(0)?;
        if g.call_method0("is_component")?.is_truthy()? {
            vp.call_method1(
                "flash_status",
                (translated(py, "It is a component already", None)?,),
            )?;
            return Ok(());
        }
        g.getattr("name")?.extract::<Option<String>>()?.filter(|name|
            !name.trim().is_empty() && !name.strip_prefix("Group ").is_some_and(|number|
                !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit())))
    } else { None };
    let suggestion = if let Some(name) = existing_name { name } else {
        let taken = py
            .import("built_core.kept_components")?
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
    let forms = py.import("built_render.chrome_forms")?;
    let field = forms.getattr("Field")?;
    let fields = PyList::empty(py);
    let focus = PyDict::new(py);
    focus.set_item("focus", true)?;
    fields.append(field.call(("text", "edit", translated(py, "Component name:", None)?, &suggestion), Some(&focus))?)?;
    fields.append(field.call1(("replace", "check", translated(py, "Replace selection with component", None)?, true))?)?;
    let options = PyDict::new(py);
    options.set_item("enabled", wrap_pyfunction!(placing_component, py)?)?;
    fields.append(field.call(("axes", "check", translated(py, "Set axes after creation", None)?, false), Some(&options))?)?;
    let done = ComponentForm {
        mode,
        win: win.clone().unbind(),
        context: scene.getattr("edit_group")?.unbind(),
        scene: scene.unbind(),
        faces: faces.unbind(),
        edges: edges.unbind(),
        groups: groups.unbind(),
        fallback: suggestion,
    };
    let kw = PyDict::new(py);
    kw.set_item("buttons", [("cancel", translated(py, "Cancel", None)?), ("ok", translated(py, "Create", None)?)])?;
    kw.set_item("on_done", Py::new(py, done)?)?;
    let form = forms.getattr("Form")?.call((translated(py, "Make component", None)?, fields), Some(&kw))?;
    form.setattr("placement", "drawer")?;
    forms.call_method1("show", (win, form))?;
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
