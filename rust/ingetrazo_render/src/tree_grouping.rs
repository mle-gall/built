//! Document facts and commands for an occurrence's context menu.
use super::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{HashMap, HashSet};

const LOCKED: &str = "Locked or hidden rows cannot be grouped: unlock or show them first";
const DIFFERENT_PARENT: &str = "Choose rows inside the same group to group them";
const SHARED: &str = "Components keep their parts: Make Unique first to change what is inside one";
const READ_ONLY: &str = "This project file is read-only here: make it the file you edit first.";

struct Row<'py> {
    group: Bound<'py, PyAny>,
    parent: Option<usize>,
}
struct Occurrences<'py> {
    rows: Vec<Row<'py>>,
    targets: Vec<usize>,
}

impl<'py> Occurrences<'py> {
    fn read(scene: &Bound<'py, PyAny>, clicked: &Bound<'py, PyAny>) -> PyResult<Self> {
        let mut out = Self {
            rows: vec![],
            targets: vec![],
        };
        if clicked.is_none() {
            return Ok(out);
        }
        let selected = scene.getattr("selection")?;
        // A collapsed definition row supplies its displayed placements explicitly.
        let definition = clicked.cast::<PyList>().ok().map(|copies| {
            copies
                .iter()
                .map(|group| group.as_ptr())
                .collect::<HashSet<_>>()
        });
        let multi = definition.is_none() && selected.contains(clicked)?;
        let mut path = Vec::new();
        let mut found = false;
        let walk = scene
            .py()
            .import("core.outliner")?
            .call_method1("walk", (scene.getattr("groups")?,))?;
        for entry in walk.try_iter()? {
            let entry = entry?;
            let group = entry.get_item(0)?;
            let depth: usize = entry.get_item(2)?.extract()?;
            path.truncate(depth);
            let index = out.rows.len();
            let direct = definition.as_ref().map_or_else(
                || group.is(clicked),
                |copies| copies.contains(&group.as_ptr()),
            );
            found |= direct;
            if direct || (multi && selected.contains(&group)?) {
                out.targets.push(index);
            }
            out.rows.push(Row {
                group,
                parent: path.last().copied(),
            });
            path.push(index);
        }
        if !found {
            out.targets.clear();
        }
        Ok(out)
    }
    fn path(&self, mut parent: Option<usize>) -> Vec<&Bound<'py, PyAny>> {
        let mut path = vec![];
        while let Some(i) = parent {
            path.push(&self.rows[i].group);
            parent = self.rows[i].parent;
        }
        path.reverse();
        path
    }
    fn refusal(
        &self,
        scene: &Bound<'py, PyAny>,
        indices: &[usize],
        siblings: bool,
    ) -> PyResult<&'static str> {
        let Some(&first) = indices.first() else {
            return Ok("");
        };
        if scene.hasattr("worksession_read_only")?
            && scene.getattr("worksession_read_only")?.is_truthy()?
        {
            return Ok(READ_ONLY);
        }
        let mut checked = HashSet::new();
        for &i in indices {
            if siblings && self.rows[i].parent != self.rows[first].parent {
                return Ok(DIFFERENT_PARENT);
            }
            if siblings
                && scene
                    .py()
                    .import("core.links")?
                    .call_method1("is_linked", (&self.rows[i].group,))?
                    .is_truthy()?
            {
                return Ok("Kept in the source file");
            }
            let mut index = Some(i);
            while let Some(j) = index {
                let row = &self.rows[j];
                if i != j
                    && (row.group.call_method0("is_component")?.is_truthy()?
                        || scene
                            .py()
                            .import("core.links")?
                            .call_method1("is_linked", (&row.group,))?
                            .is_truthy()?)
                {
                    return Ok(SHARED);
                }
                if !checked.insert(j) {
                    break;
                }
                if row.group.getattr("hidden")?.is_truthy()?
                    || !scene
                        .call_method1("entity_selectable", (&row.group,))?
                        .is_truthy()?
                {
                    return Ok(LOCKED);
                }
                index = row.parent;
            }
        }
        Ok("")
    }
    fn components(&self) -> PyResult<Vec<usize>> {
        let mut comps = vec![];
        for &i in &self.targets {
            if self.rows[i]
                .group
                .call_method0("is_component")?
                .is_truthy()?
            {
                comps.push(i);
            }
        }
        Ok(comps)
    }
    fn copies(&self, components: &[usize]) -> PyResult<Vec<usize>> {
        if components.is_empty() {
            return Ok(vec![]);
        }
        let mut definitions = HashSet::new();
        for &i in components {
            definitions.insert(self.rows[i].group.getattr("mesh")?.as_ptr());
        }
        let mut copies = vec![];
        let mut seen = HashSet::new();
        for (i, row) in self.rows.iter().enumerate() {
            if definitions.contains(&row.group.getattr("mesh")?.as_ptr())
                && row.group.call_method0("is_component")?.is_truthy()?
                && seen.insert(row.group.as_ptr())
            {
                copies.push(i);
            }
        }
        Ok(copies)
    }
    fn select_copies(&self, vp: &Bound<'py, PyAny>, components: &[usize]) -> PyResult<()> {
        let copies = self.copies(components)?;
        if copies.is_empty() {
            return Ok(());
        }
        let py = vp.py();
        let scene = vp.getattr("scene")?;
        let here = scene.getattr("edit_group")?;
        let mut counts = HashMap::new();
        for &i in &copies {
            *counts.entry(self.rows[i].parent).or_insert(0usize) += 1;
        }
        // First in model order wins a tie after the open branch.
        let best = copies
            .iter()
            .rev()
            .max_by_key(|&&i| {
                let parent = self.rows[i].parent;
                (
                    counts[&parent],
                    parent.map_or(here.is_none(), |j| here.is(&self.rows[j].group)),
                )
            })
            .unwrap();
        let parent = self.rows[*best].parent;
        if !py
            .import("views.tree_navigation")?
            .call_method1("open_to", (vp, self.path(parent)))?
            .is_truthy()?
        {
            vp.call_method1(
                "flash_status",
                (
                    translated(
                        py,
                        "A group around it is hidden or locked: show or unlock it to select inside",
                        None,
                    )?,
                    3500,
                ),
            )?;
            return Ok(());
        }
        let picks = PyList::empty(py);
        for &i in &copies {
            let row = &self.rows[i];
            if row.parent == parent
                && scene
                    .call_method1("entity_selectable", (&row.group,))?
                    .is_truthy()?
            {
                picks.append(&row.group)?;
            }
        }
        scene.call_method1("select", (&picks,))?;
        let elsewhere = copies.len() - counts[&parent];
        let kw = PyDict::new(py);
        kw.set_item("n", picks.len())?;
        let msg = if elsewhere == 0 {
            "{n} entities selected"
        } else {
            kw.set_item("m", elsewhere)?;
            "{n} copies selected; {m} more are inside other groups"
        };
        vp.call_method1("flash_status", (translated(py, msg, Some(&kw))?, 4000))?;
        vp.call_method0("notify_scene_changed")?;
        Ok(())
    }
}

#[pyfunction]
pub fn tree_group_rows(
    scene: &Bound<'_, PyAny>,
    clicked: &Bound<'_, PyAny>,
) -> PyResult<Vec<(String, String, String)>> {
    let tree = Occurrences::read(scene, clicked)?;
    if tree.targets.is_empty() {
        return Ok(vec![]);
    }
    let py = scene.py();
    let mut rows = vec![];
    let mut push =
        |id: &str, label: &str, vars: Option<&Bound<'_, PyDict>>, why: &str| -> PyResult<()> {
            let reason = if why.is_empty() {
                String::new()
            } else {
                translated(py, why, None)?.extract()?
            };
            rows.push((id.into(), translated(py, label, vars)?.extract()?, reason));
            Ok(())
        };
    if !clicked.is_instance_of::<PyList>() {
        let why = tree.refusal(scene, &tree.targets, true)?;
        if tree.targets.len() > 1 {
            push("group", "Make Group", None, why)?;
        }
        push("component", "Make Component", None, why)?;
        push("explode", "Explode Group", None, why)?;
    }
    let comps = tree.components()?;
    if !comps.is_empty() {
        push(
            "unique",
            "Make Unique",
            None,
            tree.refusal(scene, &comps, false)?,
        )?;
        let vars = PyDict::new(py);
        vars.set_item("n", tree.copies(&comps)?.len())?;
        push("instances", "Select Instances ({n})", Some(&vars), "")?;
    }
    Ok(rows)
}

#[pyfunction]
pub fn tree_group_action(
    win: &Bound<'_, PyAny>,
    clicked: &Bound<'_, PyAny>,
    verb: &str,
) -> PyResult<()> {
    if !matches!(
        verb,
        "group" | "component" | "explode" | "toggleitem" | "unique" | "instances"
    ) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Unknown tree action",
        ));
    }
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if verb == "toggleitem" {
        if !clicked.is_none()
            && scene
                .call_method1("entity_selectable", (clicked,))?
                .is_truthy()?
        {
            scene.call_method1("select", ([clicked], false, "toggle"))?;
            vp.call_method0("notify_scene_changed")?;
        }
        return Ok(());
    }
    let tree = Occurrences::read(&scene, clicked)?;
    let comps = if matches!(verb, "instances" | "unique") {
        tree.components()?
    } else {
        vec![]
    };
    if verb == "instances" {
        return tree.select_copies(&vp, &comps);
    }
    let indices = if verb == "unique" {
        &comps
    } else {
        &tree.targets
    };
    if indices.is_empty() || (verb == "group" && indices.len() < 2) {
        return Ok(());
    }
    let why = tree.refusal(&scene, indices, verb != "unique")?;
    if !why.is_empty() {
        vp.call_method1("flash_status", (translated(win.py(), why, None)?, 5000))?;
        return Ok(());
    }
    let groups = PyList::new(win.py(), indices.iter().map(|&i| &tree.rows[i].group))?;
    if verb == "unique" {
        return super::context_edits::unique_rows(win, &groups, true);
    }
    let parents = tree.path(tree.rows[indices[0]].parent);
    if !win
        .py()
        .import("views.tree_navigation")?
        .call_method1("open_to", (&vp, parents))?
        .is_truthy()?
    {
        return Ok(());
    }
    scene.call_method1("select", (&groups,))?;
    match verb {
        "group" => {
            win.call_method0("_on_make_group")?;
        }
        "component" => {
            win.call_method0("_on_make_component")?;
        }
        "explode" => {
            let history = win.py().import("core.history")?;
            let steps = PyList::empty(win.py());
            for group in groups.iter() {
                steps.append(history.call_method1("ExplodeGroupCommand", (group,))?)?;
            }
            win.call_method1(
                "_execute_make",
                (history.call_method1("CompoundCommand", (steps,))?,),
            )?;
        }
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "Unknown tree action",
            ))
        }
    }
    vp.call_method0("update")?;
    Ok(())
}
