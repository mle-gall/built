// SPDX-License-Identifier: GPL-3.0-or-later
//! Retain a component definition independently of any placed instances.
use pyo3::{prelude::*, types::PySet};
use std::collections::HashSet;
type Obj<'p> = Bound<'p, PyAny>;
fn members<'p>(scene: &Obj<'p>, key: &str) -> PyResult<Vec<Obj<'p>>> {
    match scene.getattr_opt(key)?.filter(|v| !v.is_none()) {
        Some(v) => v.try_iter()?.collect(),
        None => Ok(vec![]),
    }
}
fn placements<'p>(scene: &Obj<'p>) -> PyResult<Bound<'p, pyo3::types::PyIterator>> {
    scene
        .py()
        .import("ingetrazo_core")?
        .call_method1("purge_iter_groups", (scene.getattr_opt("groups")?,))?
        .try_iter()
}
#[pyfunction]
fn kept_template<'p>(scene: &Obj<'p>, mesh: &Obj) -> PyResult<Option<Obj<'p>>> {
    for t in members(scene, "kept_components")? {
        if t.getattr("mesh")?.is(mesh) {
            return Ok(Some(t));
        }
    }
    Ok(None)
}
#[pyfunction]
fn make_template<'p>(instance: &Obj<'p>) -> PyResult<Obj<'p>> {
    let t = place_copy(instance, None)?;
    for key in ["layer", "material", "explode_offset"] {
        t.setattr(key, instance.py().None())?;
    }
    t.setattr("hidden", false)?;
    t.setattr("component", true)?;
    Ok(t)
}
#[pyfunction(signature=(template,matrix=None))]
fn place_copy<'p>(template: &Obj<'p>, matrix: Option<&Obj>) -> PyResult<Obj<'p>> {
    let py = template.py();
    let g = py
        .import("core.group")?
        .call_method1("copy_group", (template,))?;
    let cls = py.import("ingetrazo_core")?.getattr("Matrix4")?;
    let m = match matrix {
        Some(m) => cls.call1((m,))?,
        None => cls.call0()?,
    };
    g.setattr("xform", m)?;
    Ok(g)
}
#[pyfunction]
fn copies_of<'p>(scene: &Obj<'p>, mesh: &Obj) -> PyResult<Vec<Obj<'p>>> {
    let mut out = vec![];
    for g in placements(scene)? {
        let g = g?;
        if g.getattr("mesh")?.is(mesh) && g.call_method0("is_component")?.is_truthy()? {
            out.push(g);
        }
    }
    Ok(out)
}
#[pyfunction]
fn orphaned_by<'p>(scene: &Obj<'p>, targets: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let targets = targets.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let leaving: HashSet<_> = targets.iter().map(|g| g.as_ptr()).collect();
    let mut seen = HashSet::new();
    let mut out = vec![];
    for g in targets {
        let m = g.getattr("mesh")?;
        if g.call_method0("is_component")?.is_truthy()?
            && seen.insert(m.as_ptr())
            && kept_template(scene, &m)?.is_none()
            && copies_of(scene, &m)?
                .iter()
                .all(|c| leaving.contains(&c.as_ptr()))
        {
            out.push(g);
        }
    }
    Ok(out)
}
#[pyfunction]
fn definitions_of<'p>(scene: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    for g in members(scene, "groups")?
        .into_iter()
        .chain(members(scene, "kept_components")?)
    {
        if !g.call_method0("is_component")?.is_truthy()?
            || g.getattr_opt("billboard")?
                .map(|v| v.is_truthy())
                .transpose()?
                .unwrap_or(false)
            || crate::document_insert::is_scale_figure(&g)?
        {
            continue;
        }
        if seen.insert(g.getattr("mesh")?.as_ptr()) {
            out.push(make_template(&g)?);
        }
    }
    Ok(out)
}
#[pyfunction(signature=(scene,skip=None))]
fn names_in_use<'p>(scene: &Obj<'p>, skip: Option<&Obj>) -> PyResult<Bound<'p, PySet>> {
    let names = PySet::empty(scene.py())?;
    for g in placements(scene)?.chain(members(scene, "kept_components")?.into_iter().map(Ok)) {
        let g = g?;
        if skip.is_some_and(|s| s.is(&g)) {
            continue;
        }
        let n = g.getattr("name")?;
        if n.is_truthy()? {
            names.add(n)?;
        }
    }
    Ok(names)
}
#[pyfunction]
fn numbered_name(name: &str, taken: &Obj) -> PyResult<String> {
    static SUFFIX: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^(.*?)\s*#(\d+)\n?$").unwrap());
    let py = taken.py();
    let (base, mut n) = if let Some(c) = SUFFIX.captures(name) {
        (
            c.get(1).unwrap().as_str(),
            py.get_type::<pyo3::types::PyInt>()
                .call1((&c[2],))?
                .add(1)?,
        )
    } else {
        (name, (2_i32).into_pyobject(py)?.into_any())
    };
    let used = py.get_type::<PySet>().call1((taken,))?;
    loop {
        let name = format!("{base} #{n}");
        if !used.contains(&name)? {
            return Ok(name);
        }
        n = n.add(1)?;
    }
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let name = "ingetrazo_core.kept_components";
    let sub = PyModule::new(m.py(), name)?;
    for f in [
        wrap_pyfunction!(kept_template, &sub)?,
        wrap_pyfunction!(make_template, &sub)?,
        wrap_pyfunction!(place_copy, &sub)?,
        wrap_pyfunction!(copies_of, &sub)?,
        wrap_pyfunction!(orphaned_by, &sub)?,
        wrap_pyfunction!(definitions_of, &sub)?,
        wrap_pyfunction!(names_in_use, &sub)?,
        wrap_pyfunction!(numbered_name, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(name, &sub)?;
    m.add("kept_components", sub)
}
