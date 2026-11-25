// SPDX-License-Identifier: GPL-3.0-or-later
//! The Browser's batch rename question and its single document edit.
use pyo3::{prelude::*, types::PyList};
use regex::{NoExpand, RegexBuilder};
use std::collections::{HashMap, HashSet};

type Field = (
    String,
    String,
    String,
    String,
    f64,
    f64,
    f64,
    Vec<String>,
    u32,
    String,
);
fn field(id: &str, kind: &str, label: &str, value: &str) -> Field {
    (
        id.into(),
        kind.into(),
        label.into(),
        value.into(),
        0.,
        0.,
        0.,
        vec![],
        0,
        String::new(),
    )
}

struct Target {
    group: Py<PyAny>,
    old: String,
    row: String,
}

#[pyclass]
pub(crate) struct BrowserRename {
    scene: Py<PyAny>,
    targets: Vec<Target>,
    mode: usize,
    base: String,
    start: String,
    digits: String,
    find: String,
    replacement: String,
    match_case: bool,
    prefix: String,
    suffix: String,
}

fn common_base(targets: &[Target]) -> String {
    let mut prefix = targets.first().map_or("", |t| t.old.as_str());
    for target in targets.iter().skip(1) {
        let bytes = prefix
            .chars()
            .zip(target.old.chars())
            .take_while(|(a, b)| a == b)
            .map(|(c, _)| c.len_utf8())
            .sum::<usize>();
        prefix = &prefix[..bytes];
    }
    let base = prefix
        .trim_end_matches(|c: char| c.is_ascii_digit() || " _-#.".contains(c))
        .trim();
    if base.is_empty() {
        "W".into()
    } else {
        base.into()
    }
}

#[pymethods]
impl BrowserRename {
    fn receive(&mut self, id: &str, value: &str) {
        match id {
            "mode" => {
                if let Ok(i @ 0..=2) = value.parse() {
                    self.mode = i;
                }
            }
            "base" => self.base = value.into(),
            "start" => self.start = value.into(),
            "digits" => self.digits = value.into(),
            "find" => self.find = value.into(),
            "replace" => self.replacement = value.into(),
            "case" => self.match_case = matches!(value, "1" | "true"),
            "prefix" => self.prefix = value.into(),
            "suffix" => self.suffix = value.into(),
            _ => {}
        }
    }

    fn single_row(&self) -> Option<&str> {
        (self.targets.len() == 1).then(|| self.targets[0].row.as_str())
    }

    fn preview(&self) -> Vec<(String, String)> {
        let pattern = (self.mode == 1 && !self.find.is_empty())
            .then(|| {
                RegexBuilder::new(&regex::escape(&self.find))
                    .case_insensitive(!self.match_case)
                    .build()
            })
            .and_then(Result::ok);
        let numbers = self.numbers();
        self.targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let old = &target.old;
                let new = match self.mode {
                    0 => numbers.map_or_else(
                        || old.clone(),
                        |(start, digits)| format!("{}{:0digits$}", self.base, start + index),
                    ),
                    1 => pattern.as_ref().map_or_else(
                        || old.clone(),
                        |p| p.replace_all(old, NoExpand(&self.replacement)).into_owned(),
                    ),
                    _ => format!("{}{old}{}", self.prefix, self.suffix),
                };
                (old.clone(), new)
            })
            .collect()
    }

    fn rows(&self, py: Python<'_>) -> PyResult<Vec<Field>> {
        let mut mode = field("mode", "choice", "Method", "");
        mode.4 = self.mode as f64;
        mode.7 = ["Name and number", "Find and replace", "Prefix and suffix"]
            .map(String::from)
            .to_vec();
        mode.3 = mode.7[self.mode].clone();
        let mut rows = vec![mode];
        match self.mode {
            0 => rows.extend([
                field("base", "edit", "Name:", &self.base),
                field("start", "edit", "Start at:", &self.start),
                field("digits", "edit", "Digits:", &self.digits),
            ]),
            1 => {
                let mut case = field("case", "check", "Match case", "");
                case.4 = u8::from(self.match_case) as f64;
                rows.extend([
                    field("find", "edit", "Find:", &self.find),
                    field("replace", "edit", "Replace with:", &self.replacement),
                    case,
                ]);
            }
            _ => rows.extend([
                field("prefix", "edit", "Prefix:", &self.prefix),
                field("suffix", "edit", "Suffix:", &self.suffix),
            ]),
        }
        let mut pairs = field("preview", "rename_preview", "Before", "After");
        pairs.7 = self
            .preview()
            .into_iter()
            .flat_map(|(a, b)| [a, b])
            .collect();
        rows.push(pairs);
        let tr = py.import("built_core.i18n")?.getattr("tr")?;
        for field in &mut rows {
            field.2 = tr.call1((&field.2,))?.extract()?;
            if field.1 == "choice" {
                for option in &mut field.7 {
                    *option = tr.call1((&*option,))?.extract()?;
                }
                field.3 = field.7[self.mode].clone();
            } else if field.1 == "rename_preview" {
                field.3 = tr.call1((&field.3,))?.extract()?;
            }
        }
        Ok(rows)
    }

    fn error(&self) -> Option<&'static str> {
        if self.mode == 0 && self.numbers().is_none() {
            return Some("Use a start from 0 to 99999 and 1 to 6 digits.");
        }
        let names = self.preview();
        if names.iter().any(|(_, n)| n.trim().is_empty()) {
            return Some("Enter a name for every row.");
        }
        if names.iter().all(|(a, b)| a == b) {
            return Some("No names would change.");
        }
        None
    }

    fn apply(&self, win: &Bound<'_, PyAny>) -> PyResult<bool> {
        let py = win.py();
        let vp = win.getattr("viewport")?;
        let scene = vp.getattr("scene")?;
        if !scene.is(self.scene.bind(py)) || !writable(&scene)? || self.error().is_some() {
            return Ok(false);
        }
        let root = scene.getattr("_root")?;
        let groups = PyList::empty(py);
        let names = PyList::empty(py);
        for (t, (_, new)) in self.targets.iter().zip(self.preview()) {
            let group = t.group.bind(py);
            if root.call_method1("_occurrence_path", (group,))?.is_none()
                || crate::model::links::linked_ancestor(group)?.is_some()
                || name(group)? != t.old
            {
                vp.call_method1(
                    "flash_status",
                    ("The Browser rows changed. Open Rename again.",),
                )?;
                return Ok(false);
            }
            if new != t.old {
                groups.append(group)?;
                names.append(new)?;
            }
        }
        if groups.is_empty() {
            return Ok(true);
        }
        // One native step through the protocol's batch rename verb.
        py.import("built_core.command_protocol")?
            .call_method1("rename_placements", (vp.getattr("history")?, groups, names))?;
        vp.call_method0("refresh_after_edit")?;
        Ok(true)
    }
}
impl BrowserRename {
    fn numbers(&self) -> Option<(usize, usize)> {
        let start = self.start.trim().parse().ok()?;
        let digits = self.digits.trim().parse().ok()?;
        (start <= 99999 && (1..=6).contains(&digits)).then_some((start, digits))
    }
}
fn writable(scene: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(!scene.hasattr("worksession_read_only")?
        || !scene.getattr("worksession_read_only")?.is_truthy()?)
}
fn name(group: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(group
        .getattr("name")?
        .extract::<Option<String>>()?
        .unwrap_or_default())
}

/// The displayed order wins. Collapsed or paginated targets follow in document order.
#[pyfunction]
pub(crate) fn browser_rename(
    scene: &Bound<'_, PyAny>,
    payload: &str,
) -> PyResult<Option<BrowserRename>> {
    let (owner, selected, query, order): (usize, bool, String, Vec<(String, bool)>) =
        serde_json::from_str(payload)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    if scene.as_ptr() as usize != owner || !writable(scene)? {
        return Ok(None);
    }
    let py = scene.py();
    let root = scene.getattr("_root")?;
    let selected_ids: HashSet<usize> = py
        .import("built_render")?
        .call_method1("browser_selection", (scene,))?
        .extract()?;
    let selected = selected || !selected_ids.is_empty();
    let ranks: HashMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(i, (k, problem))| (k.as_str(), (i, *problem)))
        .collect();
    let walk = py
        .import("built_core.structure")?
        .call_method1("_placements", (scene,))?;
    let row_id = py.import("built_core.link_structure")?.getattr("row_id")?;
    let repeated: HashSet<String> = root.call_method0("_repeated_uids")?.extract()?;
    let qualified_keys = ranks.keys().any(|key| key.starts_with("@["));
    let mut targets = Vec::new();
    for entry in walk.try_iter()? {
        let entry = entry?;
        if !entry.get_item(1)?.is_none() {
            continue;
        }
        let group = entry.get_item(0)?;
        if selected && !selected_ids.contains(&(group.as_ptr() as usize)) {
            continue;
        }
        let uid: String = group.getattr("uid")?.extract()?;
        let mut key = uid.clone();
        if repeated.contains(&uid)
            || uid.starts_with("@[")
            || (qualified_keys && !ranks.contains_key(uid.as_str()))
        {
            let qualified: String = row_id
                .call1((root.call_method1("_occurrence_path", (&group,))?,))?
                .extract()?;
            if repeated.contains(&uid)
                || uid.starts_with("@[")
                || ranks.contains_key(qualified.as_str())
            {
                key = qualified;
            }
        }
        let (rank, problem) = ranks
            .get(key.as_str())
            .copied()
            .unwrap_or((usize::MAX, false));
        if !selected && !query.is_empty() {
            let kind = if group.call_method0("is_component")?.is_truthy()? {
                "component"
            } else {
                "group"
            };
            if !crate::project_tree::project_item_matches(scene, &group, kind, &query, problem)? {
                continue;
            }
        }
        targets.push((
            rank,
            Target {
                old: name(&group)?,
                group: group.unbind(),
                row: key,
            },
        ));
    }
    if targets.is_empty() {
        return Ok(None);
    }
    targets.sort_by_key(|(rank, _)| *rank);
    let targets: Vec<_> = targets.into_iter().map(|(_, t)| t).collect();
    Ok(Some(BrowserRename {
        scene: scene.clone().unbind(),
        base: common_base(&targets),
        targets,
        mode: 0,
        start: "1".into(),
        digits: "2".into(),
        find: String::new(),
        replacement: String::new(),
        match_case: false,
        prefix: String::new(),
        suffix: String::new(),
    }))
}
