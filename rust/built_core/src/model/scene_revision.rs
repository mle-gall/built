// SPDX-License-Identifier: GPL-3.0-or-later
//! `Scene.revision` (`core/scene.py`): a token that moves when anything a
//! viewer shows of the document moved, read from the Rust document.
//!
//! The walk compares, from one read to the next: `content_version`; for
//! every placement under the scene's groups (the open context's, as
//! `scene.groups` gives them) and its kept definitions, its node, its
//! parent, its mesh and the mesh's revision, its uid, name, tag, custom
//! attributes (`ifc`, `ext`, `text3d`, `exploded`, `explode_offset`),
//! paint, Hide, lock, billboard, component flag, axes and matrix; the tags'
//! values and the folder tables. The Python made a `repr` of each
//! placement's fields; this writes the document's values as bytes (floats
//! by their bits, as `repr` tells `-0.0` from `0.0`, tuples from lists and
//! `True` from `1`) and compares them with the last read's.
//! `tests/test_scene_revision_golden.py`.

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use super::module;
use crate::doc::{self, py::handle_key, value::Value, Data, Key, Xform};

type Obj<'py> = Bound<'py, PyAny>;

const ATTRS: [&str; 5] = ["ifc", "ext", "text3d", "exploded", "explode_offset"];

fn put_value(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::None => out.push(0),
        Value::Bool(b) => out.extend([1, *b as u8]),
        Value::Int(i) => {
            out.push(2);
            out.extend(i.to_le_bytes());
        }
        Value::ElementKey(k) => {
            out.push(3);
            out.extend(k.to_le_bytes());
        }
        Value::Vector(x) => {
            out.push(4);
            for c in x {
                out.extend(c.to_bits().to_le_bytes());
            }
        }
        Value::Float(f) => {
            out.push(5);
            out.extend(f.to_bits().to_le_bytes());
        }
        Value::Str(s) => {
            out.push(6);
            out.extend((s.len() as u64).to_le_bytes());
            out.extend(s.as_bytes());
        }
        Value::List(l) | Value::Tuple(l) => {
            out.push(if matches!(v, Value::List(_)) { 7 } else { 8 });
            out.extend((l.len() as u64).to_le_bytes());
            for x in l {
                put_value(out, x);
            }
        }
        Value::Dict(d) => {
            out.push(9);
            out.extend((d.len() as u64).to_le_bytes());
            for (k, x) in d {
                put_value(out, k);
                put_value(out, x);
            }
        }
        Value::Opaque(o) => {
            out.push(10);
            out.extend((o.0.as_ptr() as usize as u64).to_le_bytes());
        }
    }
}

fn put_str(out: &mut Vec<u8>, s: &Option<String>) {
    match s {
        None => out.push(0),
        Some(s) => put_value(out, &Value::Str(s.clone())),
    }
}

fn put_xform(out: &mut Vec<u8>, x: &Option<Xform>) {
    match x {
        None => out.push(0),
        Some(x) => {
            out.push(1);
            for c in x.doubles() {
                out.extend(c.to_bits().to_le_bytes());
            }
        }
    }
}

/// The revision: the last one when nothing moved, else one more.
#[pyfunction]
pub fn scene_revision(py: Python<'_>, scene: &Obj<'_>) -> PyResult<i64> {
    doc::py::fresh(py)?;
    let mut out: Vec<u8> = Vec::with_capacity(4096);
    let cv = scene.getattr(pyo3::intern!(py, "content_version"))?;
    out.extend(cv.extract::<i64>().unwrap_or(i64::MIN).to_le_bytes());
    let loose = scene.getattr(pyo3::intern!(py, "loose_mesh"))?;
    out.extend((loose.as_ptr() as usize as u64).to_le_bytes());
    put_mesh_revision(&mut out, &loose)?;
    let mut tops: Vec<Key> = Vec::new();
    for name in [pyo3::intern!(py, "groups"), pyo3::intern!(py, "kept_components")] {
        for g in scene.getattr(name)?.try_iter()? {
            tops.push(handle_key(&g?)?);
        }
    }
    let attrs: Vec<u16> = ATTRS.iter().map(|a| doc::attr_id(a)).collect::<Result<_, _>>().map_err(pyo3::exceptions::PyValueError::new_err)?;
    // The rows, and the meshes whose revision is asked (once each).
    let mut meshes: Vec<Py<PyAny>> = Vec::new();
    let mut slots: HashMap<usize, usize> = HashMap::new();
    let mut mesh_rows: Vec<(usize, usize)> = Vec::new();
    // References taken under the lock are released after it.
    let mut trash: Vec<Py<PyAny>> = Vec::new();
    {
        let d = doc::lock();
        let mut stack: Vec<(Key, Key)> = tops.iter().rev().map(|k| (*k, 0)).collect();
        while let Some((k, parent)) = stack.pop() {
            let Some(n) = d.node(k) else { continue };
            let Data::Placement(p) = &n.data else { continue };
            out.extend(k.to_le_bytes());
            out.extend(parent.to_le_bytes());
            let geometry = p.definition.as_ref().map(|def| def.0.get().geometry.clone_ref(py));
            let ptr = geometry.as_ref().map_or(0, |g| g.as_ptr() as usize);
            out.extend((ptr as u64).to_le_bytes());
            if let Some(g) = geometry {
                let next = slots.len();
                let slot = *slots.entry(ptr).or_insert(next);
                if slot == meshes.len() {
                    meshes.push(g);
                } else {
                    trash.push(g);
                }
                mesh_rows.push((out.len(), slot));
            }
            put_str(&mut out, &p.uid);
            put_str(&mut out, &p.name);
            put_str(&mut out, &p.tag);
            for a in &attrs {
                match n.attr(*a) {
                    None => out.push(0xff),
                    Some(v) => put_value(&mut out, v),
                }
            }
            match &p.material {
                None => out.push(0xff),
                Some(v) => put_value(&mut out, v),
            }
            out.extend([p.hidden as u8, p.locked as u8, p.component as u8]);
            match &p.billboard {
                None => out.push(0xff),
                Some(v) => put_value(&mut out, v),
            }
            put_xform(&mut out, &p.axes);
            put_xform(&mut out, &p.xform);
            stack.extend(n.children.iter().rev().map(|c| (*c, k)));
        }
    }
    drop(trash);
    // Each mesh's revision, after the rows (read through Python, unlocked).
    let mut revs: Vec<[u8; 8]> = Vec::with_capacity(meshes.len());
    for m in &meshes {
        let r = m.bind(py).getattr(pyo3::intern!(py, "revision"))?;
        revs.push(r.extract::<i64>().unwrap_or_else(|_| r.hash().unwrap_or(0) as i64).to_le_bytes());
    }
    let mut keyed = Vec::with_capacity(out.len() + mesh_rows.len() * 8);
    let mut at = 0;
    for (pos, slot) in &mesh_rows {
        keyed.extend_from_slice(&out[at..*pos]);
        keyed.extend_from_slice(&revs[*slot]);
        at = *pos;
    }
    keyed.extend_from_slice(&out[at..]);
    // The tags and the folder tables, as the Python compared them.
    let layer_values = module(py, "core.layers")?.getattr("layer_values")?;
    let tags = pyo3::types::PyList::empty(py);
    for ly in scene.getattr(pyo3::intern!(py, "layers"))?.try_iter()? {
        tags.append(layer_values.call1((ly?,))?)?;
    }
    let mut tail = tags.repr()?.to_string();
    for name in ["layer_folders", "folder_parents", "layer_folder_names", "folder_visibility"] {
        tail.push('\u{1}');
        tail.push_str(&scene.getattr(name)?.repr()?.to_string());
    }
    keyed.extend(tail.as_bytes());

    let state = scene.getattr(pyo3::intern!(py, "__dict__"))?.cast_into::<PyDict>()?;
    let same = match state.get_item("_revision_key")? {
        Some(old) => old.cast::<PyBytes>().is_ok_and(|b| b.as_bytes() == keyed.as_slice()),
        None => false,
    };
    let mut revision: i64 = match state.get_item("_revision")? {
        Some(r) => r.extract()?,
        None => 0,
    };
    if !same {
        revision += 1;
        state.set_item("_revision_key", PyBytes::new(py, &keyed))?;
        state.set_item("_revision", revision)?;
    }
    Ok(revision)
}

fn put_mesh_revision(out: &mut Vec<u8>, mesh: &Obj<'_>) -> PyResult<()> {
    let r = mesh.getattr(pyo3::intern!(mesh.py(), "revision"))?;
    out.extend(r.extract::<i64>().unwrap_or_else(|_| r.hash().unwrap_or(0) as i64).to_le_bytes());
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scene_revision, m)?)
}
