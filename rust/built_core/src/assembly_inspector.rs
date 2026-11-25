// SPDX-License-Identifier: GPL-3.0-or-later
//! Assembly inspector projection. Measuring an expanded assembly shares the
//! cut-list reader's definition cache across all occurrences in the report.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

type Obj<'py> = Bound<'py, PyAny>;

/// Resolve locks and source ownership on the persisted occurrence. The
/// viewport's transient owner is only populated for some display proxies.
pub(crate) fn writable(scene: &Obj<'_>, group: &Obj<'_>) -> PyResult<bool> {
    let group_type = scene.py().import("core.group")?.getattr("Group")?;
    let mut at = group.clone();
    while at.is_instance(&group_type)? {
        if scene.call_method1("entity_locked", (&at,))?.is_truthy()?
            || scene
                .call_method1("_layer_state", (&at,))?
                .get_item(1)?
                .is_truthy()?
            || crate::model::links::linked_ancestor(&at)?.is_some()
        {
            return Ok(false);
        }
        at = at.getattr("_parent")?;
    }
    Ok(true)
}

/// Provider payload shared by the Project selection and Problems. The view
/// supplies its occurrence address, which is also returned with each verb.
#[pyfunction]
pub(crate) fn assembly_selection<'py>(
    scene: &Obj<'py>,
    group: &Obj<'py>,
    address: &str,
) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let row = PyDict::new(py);
    let editable = writable(scene, group)?;
    row.set_item("uid", address)?;
    row.set_item("revision", scene.getattr("content_version")?)?;
    row.set_item("can_split", !group.getattr("billboard")?.is_truthy()?)?;
    let name: String = group.getattr("name")?.extract()?;
    row.set_item("name", if name.is_empty() { "Assembly" } else { &name })?;
    row.set_item("editable", editable)?;
    let exploded = group.getattr("exploded")?;
    let exploded = exploded.cast::<PyDict>().ok();
    for (key, default) in [
        ("mode", "outward".into_pyobject(py)?.into_any()),
        ("factor", 0_i32.into_pyobject(py)?.into_any()),
    ] {
        let value = match exploded {
            Some(d) => d.get_item(key)?.unwrap_or(default),
            None => default,
        };
        row.set_item(key, value)?;
    }
    let children = PyList::empty(py);
    let selection = scene.getattr("selection")?;
    for child in group.getattr("children")?.try_iter()? {
        let child = child?;
        let entry = PyDict::new(py);
        entry.set_item("uid", child.getattr("uid")?)?;
        let name: String = child.getattr("name")?.extract()?;
        entry.set_item("name", if name.is_empty() { "Part" } else { &name })?;
        entry.set_item("hidden", child.getattr("hidden")?)?;
        entry.set_item("selected", selection.contains(&child)?)?;
        entry.set_item("editable", editable && writable(scene, &child)?)?;
        children.append(entry)?;
    }
    row.set_item("children", children)?;
    Ok(row)
}

#[pyfunction]
pub(crate) fn part_measures<'py>(group: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = group.py();
    let reader = py.import("built_core.parts")?;
    let units = py.import("core.units")?;
    let out = PyDict::new(py);
    for row in reader.call_method1("part_rows", (group,))?.try_iter()? {
        let row = row?;
        let size = row.get_item("size")?;
        let (l, w, t): (f64, f64, f64) = size.extract()?;
        let size = if reader.call_method1("is_surface", (size,))?.is_truthy()? {
            format!(
                "{} × {} — surface",
                units.call_method1("fmt_len_fine", (l,))?,
                units.call_method1("fmt_len_fine", (w,))?
            )
        } else {
            units
                .call_method1("fmt_triple", (l, w, t, true))?
                .extract()?
        };
        out.set_item(
            row.get_item("part")?.getattr("uid")?,
            (size, row.get_item("material")?),
        )?;
    }
    Ok(out)
}

/// One history step for showing or hiding the eligible children of the
/// selected assembly. Single-child edits resolve under that same parent.
#[pyfunction]
pub(crate) fn plan_part_edit<'py>(
    scene: &Obj<'py>,
    parent: &Obj<'py>,
    child_uid: Option<&str>,
    visible: Option<bool>,
    name: Option<&str>,
) -> PyResult<Option<Obj<'py>>> {
    if !writable(scene, parent)? {
        return Ok(None);
    }
    let history = scene.py().import("built_core.history")?;
    let mut changed = Vec::new();
    for child in parent.getattr("children")?.try_iter()? {
        let child = child?;
        if let Some(uid) = child_uid {
            if child.getattr("uid")?.extract::<String>()? != uid {
                continue;
            }
        }
        if !writable(scene, &child)? {
            continue;
        }
        if let (Some(_), Some(name)) = (child_uid, name) {
            let name = name.trim();
            if !name.is_empty() && !child.getattr("name")?.eq(name)? {
                let name = pyo3::types::PyString::new(scene.py(), name);
                let step = crate::doc::commands::NativeCommand::rename(&child, name.as_any())?;
                return Ok(Some(Bound::new(scene.py(), step)?.into_any()));
            }
        }
        if visible == Some(child.getattr("hidden")?.is_truthy()?) {
            changed.push(child);
        }
    }
    if changed.is_empty() {
        return Ok(None);
    }
    Ok(Some(history.call_method1(
        "HideCommand",
        (changed, !visible.unwrap()),
    )?))
}

/// A slider drag only previews resident rows. The document, archives and
/// history continue to see the committed transforms until release.
#[pyclass(module = "built_core")]
pub(crate) struct AssemblyPreview {
    rows: Vec<(usize, glam::DVec3, glam::DVec3)>,
}

#[pymethods]
impl AssemblyPreview {
    #[new]
    fn new(
        scene: &Obj<'_>,
        group: &Obj<'_>,
        packed: &Obj<'_>,
        index: &Obj<'_>,
        mode: &str,
    ) -> PyResult<Self> {
        use crate::math::{xyz, Vec3};
        use crate::placement_transform::{assembled_centres, explode_offsets, offset};
        let py = scene.py();
        let centres = assembled_centres(group)?
            .into_iter()
            .map(|v| Ok(Bound::new(py, v)?.into_any()))
            .collect::<PyResult<Vec<_>>>()?;
        let offsets = explode_offsets(centres, 1., mode)?;
        let frame = py
            .import("core.outliner")?
            .call_method1("world_matrix", (scene.getattr("groups")?, group))?;
        let world = |v: glam::DVec3| -> PyResult<glam::DVec3> {
            if frame.is_none() {
                Ok(v)
            } else {
                xyz(&frame.call_method1("mapVector", (Vec3::from(v),))?)
            }
        };
        let mut rows = Vec::new();
        for (child, step) in group.getattr("children")?.try_iter()?.zip(offsets) {
            let child = child?;
            let previous = world(offset(&child)?)?;
            let step = world(glam::DVec3::from_array(step.coords()))?;
            let mut stack = vec![child];
            while let Some(g) = stack.pop() {
                let key = packed.call_method1("row_key", (&g,))?;
                let ids = index.call_method1("get", (key,))?;
                if !ids.is_none() {
                    for row in ids.try_iter()? {
                        rows.push((row?.extract()?, step, previous));
                    }
                }
                for nested in g.getattr("children")?.try_iter()? {
                    stack.push(nested?);
                }
            }
        }
        Ok(Self { rows })
    }

    fn offsets(&self, factor: f64) -> Vec<(usize, [f32; 3])> {
        self.rows
            .iter()
            .map(|(id, unit, previous)| (*id, (*unit * factor - *previous).as_vec3().to_array()))
            .collect()
    }
}
