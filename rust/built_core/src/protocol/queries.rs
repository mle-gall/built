// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry queries produce ordered JSON records and preserve duplicate keys.
use super::keys::*;
use super::*;
fn list<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.py().get_type::<PyList>().call1((v,))
}
fn json_ids<'py>(v: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(v.py());
    for row in v.try_iter()? {
        out.append(to_json_key(&row?)?)?;
    }
    Ok(out)
}
fn base<'py>(m: &Obj<'py>) -> Reply<'py> {
    let d = PyDict::new(m.py());
    d.set_item("mesh", mesh_key(m)?)?;
    Ok(d)
}
#[pyfunction]
#[pyo3(signature=(scene,mesh=None))]
fn mesh_info<'py>(scene: &Obj<'py>, mesh: Option<&Obj<'py>>) -> Reply<'py> {
    let m = resolve_mesh(scene, mesh)?;
    let out = base(&m)?;
    let st = store_of(&m)?;
    let counts = if let Some(st) = &st {
        st.call_method0("counts")?
            .extract::<(usize, usize, usize)>()?
    } else {
        (
            m.getattr("vertices")?.len()?,
            m.getattr("edges")?.len()?,
            m.getattr("faces")?.len()?,
        )
    };
    out.set_item("vertices", counts.0)?;
    out.set_item("edges", counts.1)?;
    out.set_item("faces", counts.2)?;
    out.set_item("active", m.is(scene.getattr("mesh")?))?;
    out.set_item("backing", if st.is_some() { "store" } else { "python" })?;
    out.set_item(
        "revision",
        if let Some(st) = st {
            st.getattr("revision")?
        } else {
            m.getattr("_mut_serial")?
        },
    )?;
    Ok(out)
}
fn parse<'py>(seq: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let v = PyList::empty(seq.py());
    for k in seq.try_iter()? {
        v.append(from_json_key(&k?)?)?;
    }
    Ok(v)
}
fn xyz<'py>(p: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    PyList::new(
        p.py(),
        [
            p.call_method0("x")?,
            p.call_method0("y")?,
            p.call_method0("z")?,
        ],
    )
}
fn geometry<'py>(
    scene: &Obj<'py>,
    keys: &Obj<'py>,
    mesh: Option<&Obj<'py>>,
    kind: &str,
    field: &str,
) -> Reply<'py> {
    let py = scene.py();
    let m = resolve_mesh(scene, mesh)?;
    let keys = parse(keys)?;
    let found = elements(&m, kind, keys.as_any())?;
    let st = store_of(&m)?;
    let rows = PyList::empty(py);
    let missing = PyList::empty(py);
    for k in keys.iter() {
        let key = to_json_key(&k)?;
        let Some(element) = found.get_item(&k)? else {
            missing.append(key)?;
            continue;
        };
        let r = PyDict::new(py);
        r.set_item("key", key)?;
        match kind {
            "face" => {
                let rings = PyList::empty(py);
                let corners = PyList::empty(py);
                if let Some(st) = &st {
                    for ring in st.call_method1("face_positions", (&k,))?.try_iter()? {
                        let pts = PyList::empty(py);
                        for p in ring?.try_iter()? {
                            pts.append(list(&p?)?)?;
                        }
                        rings.append(pts)?;
                    }
                    for ring in st.call_method1("face_rings", (&k,))?.try_iter()? {
                        corners.append(json_ids(&ring?)?)?;
                    }
                } else {
                    let loops = PyList::new(py, [element.getattr("loop")?])?;
                    let holes = element.getattr("hole_loops")?;
                    if holes.is_truthy()? {
                        loops.call_method1("extend", (holes,))?;
                    }
                    for ring in loops.iter() {
                        let pts = PyList::empty(py);
                        let ids = PyList::empty(py);
                        for v in ring.try_iter()? {
                            let v = v?;
                            pts.append(xyz(&v.getattr("position")?)?)?;
                            ids.append(to_json_key(&v.getattr("key")?)?)?;
                        }
                        rings.append(pts)?;
                        corners.append(ids)?;
                    }
                }
                r.set_item("rings", rings)?;
                r.set_item("corners", corners)?;
                r.set_item("normal", xyz(&element.call_method0("normal")?)?)?;
            }
            "edge" => {
                let ends = PyList::empty(py);
                let pos = PyList::empty(py);
                for n in ["v0", "v1"] {
                    let v = element.getattr(n)?;
                    ends.append(to_json_key(&v.getattr("key")?)?)?;
                    pos.append(list(&v.call_method0("position_tuple")?)?)?;
                }
                let fs = PyList::empty(py);
                for f in element.getattr("faces")?.try_iter()? {
                    fs.append(to_json_key(&f?.getattr("key")?)?)?;
                }
                r.set_item("ends", ends)?;
                r.set_item("positions", pos)?;
                r.set_item("soft", element.getattr("soft")?.is_truthy()?)?;
                r.set_item(
                    "hidden",
                    element
                        .getattr_opt("hidden")?
                        .map(|v| v.is_truthy())
                        .transpose()?
                        .unwrap_or(false),
                )?;
                r.set_item("faces", fs)?;
            }
            _ => r.set_item("position", list(&element.call_method0("position_tuple")?)?)?,
        }
        rows.append(r)?;
    }
    let out = base(&m)?;
    out.set_item(field, rows)?;
    out.set_item("missing", missing)?;
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(scene,keys,mesh=None))]
fn faces<'py>(scene: &Obj<'py>, keys: &Obj<'py>, mesh: Option<&Obj<'py>>) -> Reply<'py> {
    geometry(scene, keys, mesh, "face", "faces")
}
#[pyfunction]
#[pyo3(signature=(scene,keys,mesh=None))]
fn edges<'py>(scene: &Obj<'py>, keys: &Obj<'py>, mesh: Option<&Obj<'py>>) -> Reply<'py> {
    geometry(scene, keys, mesh, "edge", "edges")
}
#[pyfunction]
#[pyo3(signature=(scene,keys,mesh=None))]
fn vertices<'py>(scene: &Obj<'py>, keys: &Obj<'py>, mesh: Option<&Obj<'py>>) -> Reply<'py> {
    geometry(scene, keys, mesh, "vertex", "vertices")
}
#[pyfunction]
fn selection<'py>(scene: &Obj<'py>) -> Reply<'py> {
    let py = scene.py();
    let out = base(&scene.getattr("mesh")?)?;
    let placements = PyList::empty(py);
    let kinds = PyDict::new(py);
    for k in ["vertex", "edge", "face"] {
        kinds.set_item(k, PyList::empty(py))?;
    }
    let group = py.import("built_core.group")?.getattr("Group")?;
    for x in scene.getattr("selection")?.try_iter()? {
        let x = x?;
        if x.is_instance(&group)? {
            placements.append(x.getattr("uid")?)?;
        } else if let Some(k) = kind_of(&x)? {
            kinds
                .as_any()
                .get_item(k)?
                .call_method1("append", (x.getattr("key")?,))?;
        }
    }
    placements.sort()?;
    out.set_item("placements", placements)?;
    for (k, v) in kinds.iter() {
        v.call_method0("sort")?;
        out.set_item(k, json_ids(&v)?)?;
    }
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(scene,*,mesh=None,faces=None,edges=None,vertices=None,mode="replace"))]
fn select<'py>(
    scene: &Obj<'py>,
    mesh: Option<&Obj<'py>>,
    faces: Option<&Obj<'py>>,
    edges: Option<&Obj<'py>>,
    vertices: Option<&Obj<'py>>,
    mode: &str,
) -> Reply<'py> {
    if !matches!(mode, "replace" | "add" | "remove" | "clear") {
        return Err(PyValueError::new_err(
            "mode must be replace, add, remove or clear",
        ));
    }
    let m = resolve_mesh(scene, mesh)?;
    if !m.is(scene.getattr("mesh")?) {
        return Err(PyValueError::new_err(
            "elements can be selected in the open mesh only",
        ));
    }
    let py = scene.py();
    let picked = PyList::empty(py);
    let missing = PyList::empty(py);
    for (kind, ks) in [("face", faces), ("edge", edges), ("vertex", vertices)] {
        let Some(ks) = ks else { continue };
        let keys = parse(ks)?;
        let found = elements(&m, kind, keys.as_any())?;
        for k in keys.iter() {
            if let Some(v) = found.get_item(&k)? {
                picked.append(v)?;
            } else {
                missing.append(to_json_key(&k)?)?;
            }
        }
    }
    if mode == "clear" {
        scene.call_method0("clear_selection")?;
    } else {
        let kw = PyDict::new(py);
        if mode == "remove" {
            kw.set_item("mode", mode)?;
        } else {
            kw.set_item("additive", mode == "add")?;
        }
        scene.call_method("select", (picked,), Some(&kw))?;
    }
    let out = selection(scene)?;
    out.set_item("missing", missing)?;
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(history,since_revision=Python::attach(|py| 0_i64.into_pyobject(py).unwrap().into_any().unbind())))]
fn feed<'py>(history: &Obj<'py>, since_revision: Py<PyAny>) -> Reply<'py> {
    let got = history
        .getattr("feed")?
        .call_method1("since", (since_revision,))?;
    let out = PyDict::new(history.py());
    for field in ["revision", "complete"] {
        out.set_item(field, got.get_item(field)?)?;
    }
    let steps = PyList::empty(history.py());
    for s in got.get_item("steps")?.try_iter()? {
        steps.append(s?.call_method0("to_json")?)?;
    }
    out.set_item("steps", steps)?;
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(scene,mesh=None))]
fn solid_report<'py>(scene: &Obj<'py>, mesh: Option<&Obj<'py>>) -> Reply<'py> {
    let m = resolve_mesh(scene, mesh)?;
    let pair = scene
        .py()
        .import("built_core.solids")?
        .call_method1("solid_report", (&m,))?;
    let out = base(&m)?;
    out.set_item("solid", pair.get_item(0)?)?;
    out.set_item("volume", pair.get_item(1)?)?;
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(scene,keys,mesh=None))]
fn face_set_volume<'py>(scene: &Obj<'py>, keys: &Obj<'py>, mesh: Option<&Obj<'py>>) -> Reply<'py> {
    let py = scene.py();
    let m = resolve_mesh(scene, mesh)?;
    let keys = parse(keys)?;
    let found = elements(&m, "face", keys.as_any())?;
    let missing = PyList::empty(py);
    let faces = PyList::empty(py);
    for k in keys.iter() {
        if let Some(f) = found.get_item(&k)? {
            faces.append(f)?
        } else {
            missing.append(to_json_key(&k)?)?;
        }
    }
    let out = base(&m)?;
    out.set_item(
        "volume",
        if missing.is_empty() {
            Some(
                py.import("core.bim")?
                    .call_method1("face_set_volume", (faces,))?,
            )
        } else {
            None
        },
    )?;
    out.set_item("missing", missing)?;
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(scene,keys,mesh=None))]
fn triangles<'py>(scene: &Obj<'py>, keys: &Obj<'py>, mesh: Option<&Obj<'py>>) -> Reply<'py> {
    let py = scene.py();
    let m = resolve_mesh(scene, mesh)?;
    let ks = parse(keys)?;
    let rows = PyList::empty(py);
    let missing = PyList::empty(py);
    let append = |k: Obj<'py>, v: Option<Obj<'py>>| -> PyResult<()> {
        let k = to_json_key(&k)?;
        if let Some(v) = v {
            let r = PyDict::new(py);
            r.set_item("key", k)?;
            r.set_item("triangles", v)?;
            rows.append(r)?;
        } else {
            missing.append(k)?;
        }
        Ok(())
    };
    if let Some(st) = store_of(&m)? {
        let ns = st.getattr("namespace")?.extract::<u128>()?;
        let valid = ks
            .iter()
            .map(|k| {
                k.extract::<u128>()
                    .ok()
                    .filter(|v| v >> 40 == ns)
                    .unwrap_or(0)
            })
            .collect::<Vec<_>>();
        let hypot = py.import("core._native")?.call_method0("qt_hypot")?;
        let found = st
            .call_method0("snapshot")?
            .call_method1("triangles_by_face", (valid, hypot))?;
        for (k, v) in ks.iter().zip(found.try_iter()?) {
            let v = v?;
            append(k, Some(v).filter(|v| !v.is_none()))?;
        }
    } else {
        let found = elements(&m, "face", ks.as_any())?;
        for k in ks.iter() {
            let v = if let Some(f) = found.get_item(&k)? {
                let tris = PyList::empty(py);
                for t in f.call_method0("triangulate")?.try_iter()? {
                    let pts = PyList::empty(py);
                    for p in t?.try_iter()? {
                        pts.append(xyz(&p?)?)?;
                    }
                    tris.append(pts)?;
                }
                Some(tris.into_any())
            } else {
                None
            };
            append(k, v)?;
        }
    }
    let out = base(&m)?;
    out.set_item("faces", rows)?;
    out.set_item("missing", missing)?;
    Ok(out)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(mesh_info, m)?)?;
    m.add_function(wrap_pyfunction!(faces, m)?)?;
    m.add_function(wrap_pyfunction!(edges, m)?)?;
    m.add_function(wrap_pyfunction!(vertices, m)?)?;
    m.add_function(wrap_pyfunction!(selection, m)?)?;
    m.add_function(wrap_pyfunction!(select, m)?)?;
    m.add_function(wrap_pyfunction!(feed, m)?)?;
    m.add_function(wrap_pyfunction!(solid_report, m)?)?;
    m.add_function(wrap_pyfunction!(triangles, m)?)?;
    m.add_function(wrap_pyfunction!(face_set_volume, m)?)?;
    Ok(())
}
