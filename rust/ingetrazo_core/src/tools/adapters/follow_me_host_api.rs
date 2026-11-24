// SPDX-License-Identifier: GPL-3.0-or-later
//! FollowMeHost callbacks on the live document.
use super::*;

fn keep<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let obj = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let key = py
        .get_type::<pyo3::types::PyString>()
        .call1((obj.getattr("key")?,))?;
    tool.getattr("_entities")?.set_item(&key, obj.clone())?;
    Ok(key.clone())
}
fn profile<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_entities")?
        .call_method1("get", (tool.call_method1("_state", ("profile",))?,))
}
fn hovered_face<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_entities")?
        .call_method1("get", (tool.call_method1("_state", ("hovered",))?,))
}
fn path<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.followme_host")?;
    let tool = a.get_item(0)?;
    let path = tool.call_method1("_state", ("path",))?;
    return Ok(if !path.is_none() {
        {
            let items = PyList::empty(py);
            for item in path.try_iter()? {
                let p = item?;
                items.append(module.call_method1("Vec3", positional(&p)?)?)?;
            }
            items.into_any()
        }
    } else {
        none(py)
    });
}
fn chain<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut _unused: Obj<'py>;
    let mut k: Obj<'py>;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in ({
            let value = tool.call_method1("_state", ("chain",))?;
            if value.is_truthy()? {
                value
            } else {
                PyTuple::empty(py).into_any()
            }
        })
        .try_iter()?
        {
            [k, _unused] = unpack::<2>(&item?)?;
            items.append(tool.getattr("_entities")?.get_item(&k)?)?;
        }
        items.into_any()
    });
}
fn path_points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.followme_host")?;
    let tool = a.get_item(0)?;
    return Ok((
        ({
            let items = PyList::empty(py);
            for item in tool.call_method1("_state", ("points",))?.try_iter()? {
                let p = item?;
                items.append(module.call_method1("Vec3", positional(&p)?)?)?;
            }
            items.into_any()
        }),
        tool.call_method1("_state", ("closed",))?,
    )
        .into_bound_py_any(py)?);
}
fn host_sweep_pick_face<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let px = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let face = vp.call_method1("pick_face", positional(&px)?)?;
    Ok(if !face.is_none() {
        tool.call_method1("_keep", (&face,))?
    } else {
        none(py)
    })
}
fn host_sweep_highlight<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    vp.call_method1(
        "set_hover",
        (tool.getattr("_entities")?.call_method1("get", (&key,))?,),
    )?;
    Ok(none(py))
}
fn host_sweep_selection<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.followme_host")?;
    let ordered: Obj<'py>;
    let mut p: Obj<'py>;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.getattr("_entities")?.call_method0("clear")?;
    let edges = {
        let items = PyList::empty(py);
        for item in vp.getattr("scene")?.getattr("selection")?.try_iter()? {
            let e = item?;
            if !(e.is_instance(&module.getattr("Edge")?)?) {
                continue;
            }
            items.append(&e)?;
        }
        items.into_any()
    };
    if edges.is_truthy()? {
        ordered = module.call_method1("order_path_edges", (&edges,))?;
        return Ok(if ordered.is_truthy()? {
            (
                ({
                    let items = PyList::empty(py);
                    for item in ordered.get_item(0)?.try_iter()? {
                        p = item?;
                        items.append(p.call_method0("toTuple")?)?;
                    }
                    items.into_any()
                }),
                ordered.get_item(1)?,
            )
                .into_bound_py_any(py)?
        } else {
            false.into_bound_py_any(py)?
        });
    }
    let faces = {
        let items = PyList::empty(py);
        for item in vp.getattr("scene")?.getattr("selection")?.try_iter()? {
            let f = item?;
            if !(f.is_instance(&module.getattr("Face")?)?) {
                continue;
            }
            items.append(&f)?;
        }
        items.into_any()
    };
    if faces.len()? == 1 {
        return Ok((
            ({
                let items = PyList::empty(py);
                for item in faces.get_item(0)?.getattr("vertices")?.try_iter()? {
                    p = item?;
                    items.append(p.call_method0("toTuple")?)?;
                }
                items.into_any()
            }),
            true,
        )
            .into_bound_py_any(py)?);
    }
    Ok(none(py))
}
fn host_sweep_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let face = tool.getattr("_entities")?.get_item(&key)?;
    (
        face.call_method0("centroid")?.call_method0("toTuple")?,
        face.call_method0("normal")?
            .call_method0("normalized")?
            .call_method0("toTuple")?,
    )
        .into_bound_py_any(py)
}
fn host_sweep_ring<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .getattr("_entities")?
            .get_item(&key)?
            .getattr("vertices")?
            .try_iter()?
        {
            let p = item?;
            items.append(p.call_method0("toTuple")?)?;
        }
        items.into_any()
    });
}
fn host_sweep_message<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.followme_host")?;
    let code = a.get_item(2)?;
    let vp = a.get_item(1)?;
    vp.call_method1(
        "flash_status",
        (
            module.call_method1("tr", (module.getattr("_MESSAGES")?.get_item(&code)?,))?,
            (if ("intro", "drag").into_bound_py_any(py)?.contains(&code)? {
                6000.into_bound_py_any(py)?
            } else {
                2500.into_bound_py_any(py)?
            }),
        ),
    )?;
    Ok(none(py))
}
fn host_sweep_done<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.followme_host")?;
    let ok = a.get_item(2)?;
    let vp = a.get_item(1)?;
    vp.call_method1("set_hover", (py.None(),))?;
    if ok.is_truthy()? {
        vp.getattr("scene")?
            .getattr("selection")?
            .call_method0("clear")?;
    } else {
        vp.call_method1(
            "flash_status",
            (({
                let value = vp.getattr("history")?.getattr("last_rejected")?;
                if value.is_truthy()? {
                    value
                } else {
                    module.call_method1("tr", (module.getattr("_FAILED")?,))?
                }
            }),),
        )?;
    }
    Ok(none(py))
}
fn commit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let closed = a.get_item(4)?;
    let face = a.get_item(2)?;
    let path = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let run = py.import("core.protocol.commands")?.getattr("run")?;
    let answer = run.call((vp.getattr("history")?, "follow_me",), Some(&fields!(py; "face" => py.get_type::<pyo3::types::PyString>().call1((face.getattr("key")?,))?, "path" => {let items=PyList::empty(py); for item in path.try_iter()? { let p = item?;
items.append(p.call_method0("toTuple")?)?;
} items.into_any()}, "closed" => &closed)))?;
    tool.call_method1("host_sweep_done", (&vp, answer.get_item("ok")?))?;
    vp.call_method0("update")?;
    Ok(none(py))
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    if tool
        .call_method1("_send", (&vp, fields!(py; "type" => "release").into_any()))?
        .get_item("redraw")?
        .is_truthy()?
    {
        vp.call_method0("update")?;
    }
    Ok(none(py))
}

fn preview_faces<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.followme_host")?;
    let attrs: Obj<'py>;
    let mut face: Obj<'py>;
    let tool = a.get_item(0)?;
    let faces = py
        .import("builtins")?
        .getattr("super")?
        .call1((module.getattr("FollowMeHost")?, &tool))?
        .call_method0("preview_faces")?;
    if faces.is_truthy()? && tool.getattr("_profile")?.getattr("attrs")?.is_truthy()? {
        attrs = py
            .get_type::<pyo3::types::PyDict>()
            .call1((tool.getattr("_profile")?.getattr("attrs")?,))?;
        for item in faces.try_iter()? {
            face = item?;
            face.setattr("attrs", &attrs)?;
        }
    }
    Ok(faces.clone())
}
fn host_sweep_profile<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let face = tool.getattr("_entities")?.get_item(&key)?;
    Ok({
        let values = PyList::empty(py);
        values.append(face.getattr("vertices")?)?;
        values.call_method1("extend", (face.getattr("holes")?,))?;
        positional(values.as_any())?.into_any()
    })
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.followme_host")?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("FollowMeHost")?, &tool))?
        .call_method1("on_deactivate", (&vp,))?;
    tool.getattr("_entities")?.call_method0("clear")?;
    Ok(none(py))
}

#[pyclass(frozen)]
struct ProfilePlane {
    origin: glam::DVec3,
    normal: glam::DVec3,
}
#[pymethods]
impl ProfilePlane {
    fn __call__(&self, edge: &Obj<'_>) -> PyResult<bool> {
        for end in ["a", "b"] {
            let point = crate::math::xyz(&edge.getattr(end)?)?;
            if !((point - self.origin).dot(self.normal).abs() < 1e-6) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
fn host_sweep_chain<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, vp, event] = unpack::<3>(a.as_any())?;
    let edge = vp.call_method1("pick_edge", positional(&event.get_item("px")?)?)?;
    if edge.is_none() {
        return Ok(none(py));
    }
    let entities = tool.getattr("_entities")?;
    let face = entities.get_item(event.get_item("profile")?)?;
    let plane = ProfilePlane {
        origin: crate::math::xyz(&face.call_method0("centroid")?)?,
        normal: crate::math::xyz(&face.call_method0("normal")?.call_method0("normalized")?)?,
    };
    let keys = event.get_item("chain")?;
    let chain = PyList::empty(py);
    for key in keys.try_iter()? {
        chain.append(entities.get_item(key?)?)?;
    }
    let module = py.import("tools.followme_host")?;
    let (crossing, chain) = if chain.is_empty() {
        if plane.__call__(&edge)? {
            return Ok(none(py));
        }
        let world = event.get_item("world")?;
        let toward = if world.is_none() {
            none(py)
        } else {
            module.call_method1("Vec3", positional(&world)?)?
        };
        let [crossing, chain] =
            unpack::<2>(&module.call_method1("manual_path_start", (face, edge, toward))?)?;
        (crossing, chain)
    } else {
        let skip = Bound::new(py, plane)?;
        if !module
            .call_method(
                "manual_path_extend",
                (&chain, edge),
                Some(&fields!(py; "skip"=>skip)),
            )?
            .is_truthy()?
        {
            return Ok(none(py));
        }
        (none(py), chain.into_any())
    };
    let points = PyList::empty(py);
    for vertex in chain.try_iter()? {
        let vertex = vertex?;
        points.append((
            tool.call_method1("_keep", (&vertex,))?,
            vertex.getattr("position")?.call_method0("toTuple")?,
        ))?;
    }
    let crossing = if crossing.is_none() {
        none(py)
    } else {
        crossing.call_method0("toTuple")?
    };
    (crossing, points).into_bound_py_any(py)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("host_sweep_chain", Mode::Method, host_sweep_chain, &["self", "vp", "args"], "", ""),
    ("_keep", Mode::Method, keep, &["self", "obj"], "", ""),
    ("_profile", Mode::Property, profile, &["self"], "", ""),
    ("hovered_face", Mode::Property, hovered_face, &["self"], "", ""),
    ("_path", Mode::Property, path, &["self"], "", ""),
    ("_chain", Mode::Property, chain, &["self"], "", ""),
    ("path_points", Mode::Method, path_points, &["self"], "", ""),
    ("preview_faces", Mode::Method, preview_faces, &["self"], "", ""),
    ("host_sweep_pick_face", Mode::Method, host_sweep_pick_face, &["self", "vp", "px"], "", ""),
    ("host_sweep_highlight", Mode::Method, host_sweep_highlight, &["self", "vp", "key"], "", ""),
    ("host_sweep_selection", Mode::Method, host_sweep_selection, &["self", "vp", "_"], "", ""),
    ("host_sweep_plane", Mode::Method, host_sweep_plane, &["self", "vp", "key"], "", ""),
    ("host_sweep_ring", Mode::Method, host_sweep_ring, &["self", "vp", "key"], "", ""),
    ("host_sweep_profile", Mode::Method, host_sweep_profile, &["self", "vp", "key"], "", ""),
    ("host_sweep_message", Mode::Method, host_sweep_message, &["self", "vp", "code"], "", ""),
    ("host_sweep_done", Mode::Method, host_sweep_done, &["self", "vp", "ok"], "", ""),
    ("_commit", Mode::Method, commit, &["self", "vp", "face", "path", "closed"], "", ""),
    ("on_release", Mode::Method, on_release, &["self", "vp"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "vp"], "", ""),
];
