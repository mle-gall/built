// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
const KINDS: [(&str, &str, (u8, u8, u8)); 5] = [
    ("open", "Open border (hole)", (230, 40, 40)),
    ("stray", "Stray edges", (255, 140, 0)),
    ("over", "Overconnected edges", (150, 60, 220)),
    ("internal", "Internal face", (30, 130, 255)),
    ("reversed", "Reversed face", (230, 0, 160)),
];

#[pyfunction]
fn _problem_text(report: &Obj<'_>) -> PyResult<String> {
    let tr = report.py().import("built_core.i18n")?.getattr("tr")?;
    let mut parts = vec![];
    for (key, fmt) in [
        ("open", "{n} open borders"),
        ("stray", "{n} stray edges"),
        ("over", "{n} overconnected"),
        ("internal", "{n} internal faces"),
        ("reversed", "{n} reversed faces"),
    ] {
        let v = report.call_method1("get", (key,))?;
        if v.is_truthy()? {
            let kw = PyDict::new(v.py());
            kw.set_item("n", v.len()?)?;
            parts.push(tr.call((fmt,), Some(&kw))?.extract::<String>()?);
        }
    }
    Ok(parts.join(" · "))
}

#[pyfunction]
fn _edge_clusters<'p>(edges: &Obj<'p>) -> PyResult<Vec<Vec<Obj<'p>>>> {
    fn root(parents: &mut HashMap<Key, Key>, mut a: Key) -> Key {
        loop {
            let p = *parents.entry(a).or_insert(a);
            if p == a {
                return a;
            }
            let gp = parents[&p];
            parents.insert(a, gp);
            a = gp;
        }
    }
    let mut parent = HashMap::new();
    let mut seen = vec![];
    for e in edges.try_iter()? {
        let e = e?;
        let a = key(&e.getattr("v0")?)?;
        let left = root(&mut parent, a);
        let right = root(&mut parent, key(&e.getattr("v1")?)?);
        parent.insert(left, right);
        seen.push((a, e));
    }
    let mut sets = Vec::<Vec<Obj<'p>>>::new();
    let mut slots = HashMap::new();
    for (a, e) in seen {
        let i = *slots.entry(root(&mut parent, a)).or_insert_with(|| {
            sets.push(vec![]);
            sets.len() - 1
        });
        sets[i].push(e);
    }
    Ok(sets)
}
#[pyfunction]
fn problem_items<'p>(report: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = report.py();
    let out = PyList::empty(py);
    let append = |kind: &str, ents: Vec<Obj<'p>>| -> PyResult<()> {
        let row = PyDict::new(py);
        row.set_item("kind", kind)?;
        row.set_item("ents", ents)?;
        out.append(row)
    };
    for name in ["open", "stray"] {
        for run in _edge_clusters(&report.get_item(name)?)? {
            append(name, run)?;
        }
    }
    let empty = PyTuple::empty(py);
    let internal = report.call_method1("get", ("internal", &empty))?;
    let mut explained = HashSet::new();
    for f in internal.try_iter()? {
        explained.extend(face_pairs(&f?)?);
    }
    let over = PyList::empty(py);
    for edge in report.get_item("over")?.try_iter()? {
        let e = edge?;
        if !explained.contains(&edge_pair(&e.getattr("v0")?, &e.getattr("v1")?)?) {
            over.append(e)?;
        }
    }
    for run in _edge_clusters(over.as_any())? {
        append("over", run)?;
    }
    for (k, faces) in [
        ("internal", internal),
        ("reversed", report.call_method1("get", ("reversed", empty))?),
    ] {
        for f in faces.try_iter()? {
            append(k, vec![f?])?;
        }
    }
    Ok(out)
}
#[pyfunction]
fn _item_points<'p>(item: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let mut out = vec![];
    for ent in item.get_item("ents")?.try_iter()? {
        let ent = ent?;
        if let Some(ring) = ent.getattr_opt("loop")? {
            out.extend(positions(&rows(&ring)?)?);
        } else {
            for v in ["v0", "v1"] {
                out.push(ent.getattr(v)?.getattr("position")?);
            }
        }
    }
    Ok(out)
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(_problem_text, m)?,
        wrap_pyfunction!(_edge_clusters, m)?,
        wrap_pyfunction!(problem_items, m)?,
        wrap_pyfunction!(_item_points, m)?,
    ] {
        m.add_function(f)?;
    }
    let kinds = PyDict::new(m.py());
    for (name, label, color) in KINDS {
        kinds.set_item(name, (label, color))?;
    }
    m.add("KINDS", kinds)
}
