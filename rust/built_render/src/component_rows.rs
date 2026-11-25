//! Library definitions grouped by prototype mesh. The retired tray's deferred
//! refresher can consume the same walk in bounded slices.
use pyo3::{prelude::*, types::PyIterator};
use std::{cmp::Reverse, collections::HashMap};

type Row<'py> = (String, usize, Vec<Py<PyAny>>, Option<Bound<'py, PyAny>>);

#[pyclass]
pub struct ComponentRows {
    walk: Py<PyIterator>,
    scene: Py<PyAny>,
    by_mesh: HashMap<usize, usize>,
    placed: Vec<Vec<Py<PyAny>>>,
}

#[pymethods]
impl ComponentRows {
    #[new]
    fn new(scene: &Bound<'_, PyAny>) -> PyResult<Self> {
        let walk = scene
            .py()
            .import("built_core")?
            .call_method1("purge_iter_groups", (scene.getattr("groups")?,))?
            .try_iter()?;
        Ok(Self {
            walk: walk.unbind(),
            scene: scene.clone().unbind(),
            by_mesh: HashMap::new(),
            placed: vec![],
        })
    }

    fn advance(&mut self, py: Python<'_>, limit: usize) -> PyResult<bool> {
        let mut walk = self.walk.bind(py).clone();
        for _ in 0..limit {
            let Some(group) = walk.next() else {
                return Ok(false);
            };
            let group = group?;
            if !group.call_method0("is_component")?.is_truthy()?
                || group.getattr("billboard")?.is_truthy()?
            {
                continue;
            }
            let mesh = group.getattr("mesh")?.as_ptr() as usize;
            let i = *self.by_mesh.entry(mesh).or_insert_with(|| {
                self.placed.push(vec![]);
                self.placed.len() - 1
            });
            self.placed[i].push(group.unbind());
        }
        Ok(true)
    }

    fn finish<'py>(&mut self, py: Python<'py>) -> PyResult<Vec<Row<'py>>> {
        self.advance(py, usize::MAX)?;
        let mut definitions = vec![None; self.placed.len()];
        for template in self.scene.bind(py).getattr("kept_components")?.try_iter()? {
            let template = template?;
            let key = template.getattr("mesh")?.as_ptr() as usize;
            if let Some(&i) = self.by_mesh.get(&key) {
                definitions[i].get_or_insert(template);
            } else {
                definitions.push(Some(template));
                self.placed.push(vec![]);
            }
        }
        let faces = py.import("built_core.mesh")?.getattr("face_count")?;
        let mut rows = Vec::with_capacity(definitions.len());
        for (groups, definition) in self.placed.drain(..).zip(definitions) {
            let source = groups
                .first()
                .map(|g| g.bind(py))
                .or(definition.as_ref())
                .unwrap();
            let name = source
                .getattr("name")?
                .extract::<Option<String>>()?
                .unwrap_or_default();
            let count = faces.call1((source.getattr("mesh")?,))?.extract()?;
            rows.push((name, count, groups, definition));
        }
        self.by_mesh.clear();
        rows.sort_by_cached_key(|(name, _, groups, _)| {
            (Reverse(groups.len()), name.to_lowercase())
        });
        Ok(rows)
    }
}

/// Selection lives in one edit context. Prefer the parent holding most
/// instances, then the current context, then the first in document order.
#[pyfunction]
pub fn select_component_instances(
    win: &Bound<'_, PyAny>,
    groups: Vec<Bound<'_, PyAny>>,
) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let roots = scene.getattr("groups")?;
    let outliner = py.import("built_core.outliner")?;
    let parents = outliner
        .call_method1("parents", (&roots,))?
        .cast_into::<pyo3::types::PyDict>()?;
    let current = scene.getattr("edit_group")?.as_ptr();
    let total = groups.len();
    let mut by_parent = HashMap::<_, usize>::new();
    let mut buckets: Vec<(bool, Vec<Bound<'_, PyAny>>)> = Vec::new();
    for group in groups {
        let Some(parent) = parents.get_item(group.as_ptr() as usize)? else {
            continue;
        };
        let index = *by_parent.entry(parent.as_ptr()).or_insert_with(|| {
            buckets.push((parent.as_ptr() == current, vec![]));
            buckets.len() - 1
        });
        buckets[index].1.push(group);
    }
    let Some((_, picks)) = buckets
        .into_iter()
        .rev()
        .max_by_key(|(current, picks)| (picks.len(), *current))
    else {
        return Ok(());
    };
    let path = outliner.call_method1("path_to", (roots, &picks[0]))?;
    let tr = py.import("built_core.i18n")?.getattr("tr")?;
    if !py
        .import("built_render.tree_navigation")?
        .call_method1("open_to", (&vp, path))?
        .is_truthy()?
    {
        let text = tr.call1((
            "A group around it is hidden or locked: show or unlock it to select inside",
        ))?;
        vp.call_method1("flash_status", (text, 3500))?;
        return Ok(());
    }
    let others = total - picks.len();
    let selection = pyo3::types::PyList::empty(py);
    for group in picks {
        if scene
            .call_method1("entity_selectable", (&group,))?
            .is_truthy()?
        {
            selection.append(group)?;
        }
    }
    scene.call_method1("select", (&selection,))?;
    vp.call_method0("notify_scene_changed")?;
    let counts = pyo3::types::PyDict::new(py);
    counts.set_item("n", selection.len())?;
    let label = if others == 0 {
        "{n} entities selected"
    } else {
        counts.set_item("m", others)?;
        "{n} copies selected; {m} more are inside other groups"
    };
    win.call_method0("statusBar")?
        .call_method1("showMessage", (tr.call((label,), Some(&counts))?, 4000))?;
    Ok(())
}
