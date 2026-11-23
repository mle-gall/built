// SPDX-License-Identifier: GPL-3.0-or-later
//! Assembly inspector projection. Measuring an expanded assembly shares the
//! cut-list reader's definition cache across all occurrences in the report.
use pyo3::{prelude::*, types::PyDict};

type Obj<'py> = Bound<'py, PyAny>;

fn writable(scene: &Obj<'_>, group: &Obj<'_>) -> PyResult<bool> {
    Ok(!scene.call_method1("entity_locked", (group,))?.is_truthy()?
        && scene
            .py()
            .import("core.links")?
            .call_method1("linked_ancestor", (group,))?
            .is_none())
}

#[pyfunction]
pub(crate) fn part_measures<'py>(group: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = group.py();
    let reader = py.import("core.parts")?;
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
    let history = scene.py().import("core.history")?;
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
                return Ok(Some(
                    history.call_method1("RenameGroupCommand", (child, name))?,
                ));
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
#[pyclass(module = "ingetrazo_core")]
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
