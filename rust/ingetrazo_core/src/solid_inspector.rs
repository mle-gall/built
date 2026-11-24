// SPDX-License-Identifier: GPL-3.0-or-later
//! Project inspector adapter for the solid analysis plugin. A repair addresses
//! the same connected defect even if another hole is repaired first.
use pyo3::{prelude::*, types::PyDict};

type Obj<'py> = Bound<'py, PyAny>;
type Sections = Vec<(String, Vec<(String, String)>)>;

fn fault_key(scene: &Obj<'_>, group: &Obj<'_>, item: &Obj<'_>) -> PyResult<String> {
    let mut keys = item
        .get_item("ents")?
        .try_iter()?
        .map(|e| e?.getattr("key")?.extract::<u128>())
        .collect::<PyResult<Vec<_>>>()?;
    keys.sort_unstable();
    let keys: Vec<_> = keys.iter().map(u128::to_string).collect();
    let kind: String = item.get_item("kind")?.extract()?;
    let uid: String = if group.is_none() {
        "active".into()
    } else {
        group.getattr("uid")?.extract()?
    };
    let mesh = if group.is_none() {
        scene.getattr("mesh")?
    } else {
        group.getattr("mesh")?
    };
    Ok(format!(
        "{uid}/{:p}/{:p}/{kind}/{}",
        scene.as_ptr(),
        mesh.as_ptr(),
        keys.join(",")
    ))
}

fn inspect<'py>(
    scene: &Obj<'py>,
    group: &Obj<'py>,
    explicit: bool,
) -> PyResult<Option<(Obj<'py>, Obj<'py>)>> {
    let py = scene.py();
    let owner = if group.is_none() {
        scene.getattr("edit_group")?
    } else {
        group.clone()
    };
    if !owner.is_none()
        && (owner.getattr("billboard")?.is_truthy()?
            || !py
                .import("core.links")?
                .call_method1("linked_ancestor", (&owner,))?
                .is_none())
    {
        return Ok(None);
    }
    let mesh = if group.is_none() {
        scene.getattr("mesh")?
    } else {
        group.getattr("mesh")?
    };
    if !explicit
        && !py
            .import("core.lazy_mesh")?
            .call_method1("plan_of", (&mesh,))?
            .is_none()
    {
        return Ok(None);
    }
    if mesh.getattr("edges")?.len()? == 0 || (!explicit && mesh.getattr("faces")?.len()? > 20_000) {
        return Ok(None);
    }
    let plugin = py.import("core.solid_analysis")?;
    let report = plugin.call_method1("inspect_mesh", (&mesh,))?;
    Ok(Some((mesh, report)))
}

#[pyfunction]
#[pyo3(signature=(scene, group, explicit=false))]
pub(crate) fn solid_inspector_sections(
    scene: &Obj<'_>,
    group: &Obj<'_>,
    explicit: bool,
) -> PyResult<Sections> {
    let Some((mesh, report)) = inspect(scene, group, explicit)? else {
        return Ok(Vec::new());
    };
    let plugin = scene.py().import("core.solid_analysis")?;
    if plugin.call_method1("is_sound", (&report,))?.is_truthy()? {
        let volume: Option<f64> = report.get_item("volume")?.extract()?;
        let value = volume
            .map(|v| format!("{v:.3} m³"))
            .unwrap_or_else(|| "yes".into());
        return Ok(vec![("Solid".into(), vec![("Watertight".into(), value)])]);
    }
    let status = if report.get_item("watertight")?.is_truthy()? {
        "Faces reversed"
    } else {
        "Not watertight"
    };
    let mut sections = vec![(
        "Solid".into(),
        vec![(
            status.into(),
            plugin
                .call_method1("_problem_text", (&report,))?
                .extract()?,
        )],
    )];
    let mut actions = Vec::new();
    let mut reasons = Vec::new();
    let writable = !locked(scene, group)?;
    if !explicit {
        if writable {
            sections.push((
                "Actions".into(),
                vec![("solidrepair".into(), "Repair".into())],
            ));
        }
        return Ok(sections);
    }
    let mut count = 0;
    for fault in plugin
        .call_method1("problem_items", (&report,))?
        .try_iter()?
    {
        let fault = fault?;
        count += 1;
        let kind = fault.get_item("kind")?.extract::<String>()?;
        let title: String = plugin
            .getattr("KINDS")?
            .get_item(&kind)?
            .get_item(0)?
            .extract()?;
        let reason: Option<String> = plugin
            .call_method1("why_not_repairable", (&mesh, &fault))?
            .extract()?;
        if let Some(reason) = reason {
            reasons.push((format!("{count}. {title}"), reason));
        } else if writable {
            actions.push((
                format!("solidfault:{}", fault_key(scene, group, &fault)?),
                format!("Repair {count}. {title}"),
            ));
        }
    }
    if !reasons.is_empty() {
        sections.push(("Manual repairs".into(), reasons));
    }
    if !actions.is_empty() {
        if count > 1 && !group.is_none() {
            actions.push(("solidrepair".into(), "Repair all in this object".into()));
        }
        sections.push(("Actions".into(), actions));
    }
    Ok(sections)
}

/// Re-diagnose before editing. An old inspector click cannot repair the next
/// defect in the list or write through a linked or locked occurrence.
#[pyfunction]
pub(crate) fn plan_solid_fault<'py>(
    scene: &Obj<'py>,
    group: &Obj<'py>,
    key: Option<&str>,
) -> PyResult<Option<Obj<'py>>> {
    if locked(scene, group)? {
        return Ok(None);
    }
    let Some((mesh, report)) = inspect(scene, group, true)? else {
        return Ok(None);
    };
    let py = scene.py();
    let plugin = py.import("core.solid_analysis")?;
    for item in plugin
        .call_method1("problem_items", (&report,))?
        .try_iter()?
    {
        let item = item?;
        if let Some(key) = key {
            if fault_key(scene, group, &item)? != key {
                continue;
            }
        }
        if !plugin
            .call_method1("why_not_repairable", (&mesh, &item))?
            .is_none()
        {
            continue;
        }
        if key.is_none() {
            return Ok(Some(
                plugin.call_method1("RepairSolidsCommand", (vec![mesh],))?,
            ));
        }
        let args = PyDict::new(py);
        args.set_item("item", item)?;
        let fix = py
            .import("functools")?
            .getattr("partial")?
            .call((plugin.getattr("repair_item")?,), Some(&args))?;
        return Ok(Some(
            plugin.call_method1("RepairSolidsCommand", (vec![mesh], fix))?,
        ));
    }
    Ok(None)
}

fn locked(scene: &Obj<'_>, group: &Obj<'_>) -> PyResult<bool> {
    if group.is_none()
        && scene
            .getattr("_root")?
            .call_method0("_loose_locked")?
            .is_truthy()?
    {
        return Ok(true);
    }
    let owner = if group.is_none() {
        scene.getattr("edit_group")?
    } else {
        group.clone()
    };
    if owner.is_none() {
        return Ok(false);
    }
    Ok(!crate::assembly_inspector::writable(scene, &owner)?)
}

/// Scan local definitions once. Group meshes are often shared by thousands
/// of occurrences. Read deferred definitions from their load plans.
#[pyfunction]
#[pyo3(signature=(scene, budget=1.5))]
pub(crate) fn solid_problem_targets<'py>(
    scene: &Obj<'py>,
    budget: f64,
) -> PyResult<(Vec<Obj<'py>>, usize, usize)> {
    use std::{collections::HashSet, time::Instant};
    let py = scene.py();
    let plugin = py.import("core.solid_analysis")?;
    let lazy = py.import("core.lazy_mesh")?;
    let solids = py.import("core.solids")?;
    let links = py.import("core.links")?;
    let active = scene.getattr("mesh")?;
    let mut seen = HashSet::from([active.as_ptr()]);
    let mut candidates = vec![(py.None().into_bound(py), active)];
    let mut stack: Vec<_> = scene
        .getattr("groups")?
        .try_iter()?
        .collect::<PyResult<_>>()?;
    stack.reverse();
    while let Some(g) = stack.pop() {
        if g.getattr("billboard")?.is_truthy()? || !links.call_method1("link_of", (&g,))?.is_none()
        {
            continue;
        }
        let children: Vec<_> = g
            .getattr("children")?
            .try_iter()?
            .collect::<PyResult<_>>()?;
        stack.extend(children.into_iter().rev());
        let mesh = g.getattr("mesh")?;
        if seen.insert(mesh.as_ptr()) {
            candidates.push((g, mesh));
        }
    }
    let mut broken = Vec::new();
    let total = candidates.len();
    let start = Instant::now();
    for (index, (g, mesh)) in candidates.into_iter().enumerate() {
        if index > 0 && start.elapsed().as_secs_f64() >= budget {
            return Ok((broken, index, total));
        }
        let bad = if !lazy.call_method1("plan_of", (&mesh,))?.is_none() {
            solids.call_method1("_lazily_open", (&mesh,))?.is_truthy()?
        } else if mesh.getattr("edges")?.len()? == 0 {
            false
        } else {
            let report = plugin.call_method1("inspect_mesh", (&mesh,))?;
            !plugin.call_method1("is_sound", (report,))?.is_truthy()?
        };
        if bad {
            broken.push(g);
        }
    }
    Ok((broken, total, total))
}
