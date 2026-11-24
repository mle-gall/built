// SPDX-License-Identifier: GPL-3.0-or-later
//! Apply file-level registries and world-space annotations using the editor's
//! existing document classes so plugins and both windows see the same records.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::{HashMap, HashSet};

fn rows<'py>(dict: &Bound<'py, PyAny>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    let val = dict.call_method1("get", (key,))?;
    Ok(if val.is_none() {
        PyList::empty(dict.py()).into_any()
    } else {
        val
    })
}
fn opt<'py>(dict: &Bound<'py, PyAny>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    dict.call_method1("get", (key,))
}

pub(super) fn material_names(
    py: Python<'_>,
    scene: &Bound<'_, PyAny>,
    payload: &Bound<'_, PyDict>,
) -> PyResult<HashMap<String, String>> {
    let mats = rows(payload, "materials")?;
    if !mats.is_truthy()? {
        return Ok(HashMap::new());
    }
    let module = py.import("core.materials")?;
    let decode = module.getattr("Material")?.getattr("from_dict")?;
    let register = module.getattr("register")?;
    let registry = match scene.getattr_opt("materials")?.filter(|v| !v.is_none()) {
        Some(reg) => reg,
        None => {
            let reg = PyDict::new(py).into_any();
            scene.setattr("materials", &reg)?;
            reg
        }
    };
    let mut remap = HashMap::new();
    for rec in mats.try_iter()? {
        let mat = decode.call1((rec?,))?;
        let name = mat.getattr("name")?.extract::<String>()?;
        if name.is_empty() {
            continue;
        }
        let actual = register.call1((&registry, mat))?.extract::<String>()?;
        if name != actual {
            remap.insert(name, actual);
        }
    }
    Ok(remap)
}

fn register_materials(
    py: Python<'_>,
    scene: &Bound<'_, PyAny>,
    payload: &Bound<'_, PyDict>,
) -> PyResult<()> {
    let remap = material_names(py, scene, payload)?;
    if remap.is_empty() {
        return Ok(());
    }
    let mut seen = HashSet::new();
    for kind in ["groups", "protos"] {
        for part in rows(payload, kind)?.try_iter()? {
            for face in rows(&part?, "faces")?.try_iter()? {
                let attr = face?.get_item(2)?;
                if attr.is_truthy()? && seen.insert(attr.as_ptr()) {
                    if let Some(name) = opt(&attr, "mat")?.extract::<Option<String>>()? {
                        if let Some(actual) = remap.get(&name) {
                            attr.set_item("mat", actual)?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[pyfunction]
pub fn skp_apply_folders<'py>(
    scene: &Bound<'py, PyAny>,
    records: &Bound<'py, PyAny>,
) -> PyResult<(Bound<'py, PyDict>, Bound<'py, PyDict>)> {
    let py = scene.py();
    let layers = py.import("core.layers")?;
    let names = PyDict::new(py);
    let visibility = PyDict::new(py);
    let folders = PyDict::new(py);
    let hidden = PyDict::new(py);
    for record in records.try_iter()? {
        let row = record?;
        let key = opt(&row, "key")?;
        let parent_key = opt(&row, "parent")?;
        let parent = names
            .get_item(&parent_key)?
            .unwrap_or_else(|| py.None().into_bound(py));
        let name = opt(&row, "name")?.extract::<Option<String>>()?;
        let name = name.as_deref().unwrap_or("").trim();
        let name = if name.is_empty() { "Folder" } else { name };
        let taken = layers.call_method1("folders_of", (scene,))?;
        let here = if !taken.contains(name)? {
            let list = PyList::new(py, taken.extract::<Vec<Py<PyAny>>>()?)?;
            list.append(name)?;
            scene.setattr("layer_folders", list)?;
            if parent.is_truthy()? {
                layers.call_method1("move_folder", (scene, name, &parent))?;
            }
            name.to_owned()
        } else if layers
            .call_method1("folder_parent", (scene, name))?
            .eq(&parent)?
        {
            name.to_owned()
        } else {
            layers
                .call_method1("add_folder", (scene, name, &parent))?
                .extract()?
        };
        let off = opt(&row, "hidden")?.is_truthy()?;
        scene.getattr("folder_visibility")?.set_item(&here, !off)?;
        let off = off
            || visibility
                .get_item(parent_key)?
                .is_some_and(|v| v.is_truthy().unwrap_or(false));
        names.set_item(&key, &here)?;
        visibility.set_item(key, off)?;
        for tag in rows(&row, "tags")?.try_iter()? {
            let tag = tag?;
            folders.set_item(&tag, &here)?;
            hidden.set_item(tag, off)?;
        }
    }
    Ok((folders, hidden))
}

fn names<'py>(items: &Bound<'py, PyAny>, attr: &str) -> PyResult<Bound<'py, PySet>> {
    let set = PySet::empty(items.py())?;
    for item in items.try_iter()? {
        set.add(item?.getattr(attr)?)?;
    }
    Ok(set)
}

#[pyfunction]
pub fn skp_apply_metadata(scene: &Bound<'_, PyAny>, payload: &Bound<'_, PyDict>) -> PyResult<()> {
    let py = scene.py();
    register_materials(py, scene, payload)?;
    let tags = rows(payload, "layers")?;
    if tags.is_truthy()? {
        let (folders, _) = skp_apply_folders(scene, &rows(payload, "layer_folders")?)?;
        let list = scene.getattr("layers")?;
        let existing = names(&list, "key")?;
        let class = py.import("core.layers")?.getattr("Layer")?;
        for row in tags.try_iter()? {
            let row = row?;
            let name = opt(&row, "name")?;
            if !name.is_truthy()? || existing.contains(&name)? {
                continue;
            }
            let kw = PyDict::new(py);
            kw.set_item("visible", row.call_method1("get", ("visible", true))?)?;
            kw.set_item("folder", folders.get_item(&name)?)?;
            list.call_method1("append", (class.call((&name,), Some(&kw))?,))?;
            existing.add(name)?;
        }
    }
    let views = rows(payload, "scenes")?;
    if views.is_truthy()? {
        let list = scene.getattr("saved_views")?;
        let existing = names(&list, "name")?;
        let camera = py.import("core.saved_views")?.getattr("from_lookat")?;
        for (i, row) in views.try_iter()?.enumerate() {
            if i % 128 == 0 && i > 0 {
                py.detach(std::thread::yield_now);
            }
            let row = row?;
            let name = row.get_item("name")?;
            if existing.contains(&name)? {
                continue;
            }
            let mut up = opt(&row, "up")?;
            if !up.is_truthy()? {
                up = (0., 0., 1.).into_pyobject(py)?.into_any();
            }
            let height = opt(&row, "ortho_height")?;
            let view = camera.call1((
                &name,
                row.get_item("eye")?,
                row.get_item("target")?,
                up,
                row.call_method1("get", ("fov", 45.))?,
                !opt(&row, "parallel")?.is_truthy()?,
                height.is_truthy()?.then_some(height),
                opt(&row, "hidden_layers")?,
            ))?;
            list.call_method1("append", (view,))?;
            existing.add(name)?;
        }
    }
    let dims = rows(payload, "dimensions")?;
    if dims.is_truthy()? {
        let class = py.import("core.dimension")?.getattr("Dimension")?;
        let list = scene.getattr("dimensions")?;
        for raw in dims.try_iter()? {
            let raw = raw?;
            let a = xyz(&raw.get_item("a")?)?;
            let b = xyz(&raw.get_item("b")?)?;
            let segment = b - a;
            if segment.length() < 1e-9 {
                continue;
            }
            let vector = opt(&raw, "offset_vector")?;
            let v = if vector.is_truthy()? {
                xyz(&vector)?
            } else {
                let n = opt(&raw, "normal")?;
                let normal = if n.is_truthy()? { xyz(&n)? } else { DVec3::Z };
                let mut perp = normal.cross(segment);
                if perp.length() < 1e-9 {
                    perp = DVec3::Z.cross(segment);
                }
                if perp.length() < 1e-9 {
                    perp = DVec3::X.cross(segment);
                }
                perp.normalize() * raw.call_method1("get", ("offset", 0.))?.extract::<f64>()?
            };
            list.call_method1(
                "append",
                (class.call1((Vec3::from(a), Vec3::from(b), Vec3::from(v)))?,),
            )?;
        }
    }
    let texts = rows(payload, "texts")?;
    if texts.is_truthy()? {
        let class = py.import("core.textlabel")?.getattr("TextLabel")?;
        let list = scene.getattr("text_labels")?;
        for raw in texts.try_iter()? {
            let raw = raw?;
            if opt(&raw, "hidden")?.is_truthy()? {
                continue;
            }
            let anchor = xyz(&raw.get_item("anchor")?)?;
            let label = opt(&raw, "label")?;
            let delta = if label.is_truthy()? {
                xyz(&label)? - anchor
            } else {
                DVec3::ZERO
            };
            list.call_method1(
                "append",
                (class.call1((Vec3::from(anchor), Vec3::from(delta), raw.get_item("text")?))?,),
            )?;
        }
    }
    Ok(())
}

struct Definitions<'py> {
    records: Vec<Bound<'py, PyAny>>,
    meshes: Vec<Bound<'py, PyAny>>,
    group: Bound<'py, PyAny>,
    ids: HashSet<String>,
    namespace: Bound<'py, PyAny>,
    uuid: Bound<'py, PyAny>,
}

impl<'py> Definitions<'py> {
    fn assign_uid(
        &mut self,
        group: &Bound<'py, PyAny>,
        identity: &Bound<'py, PyAny>,
    ) -> PyResult<()> {
        let mut uid = group.getattr("uid")?.extract::<String>()?;
        if !identity.is_none() && self.namespace.is_truthy()? {
            let repr = (&self.namespace, identity)
                .into_pyobject(group.py())?
                .repr()?;
            let candidate = self
                .uuid
                .call_method1("uuid5", (self.uuid.getattr("NAMESPACE_URL")?, repr))?
                .getattr("hex")?
                .extract::<String>()?;
            if !self.ids.contains(&candidate[..16]) {
                uid = candidate[..16].to_owned();
                group.setattr("uid", &uid)?;
            }
        }
        while !self.ids.insert(uid.clone()) {
            uid = self
                .uuid
                .call_method0("uuid4")?
                .getattr("hex")?
                .extract::<String>()?[..16]
                .to_owned();
            group.setattr("uid", &uid)?;
        }
        Ok(())
    }
    fn place(
        &mut self,
        index: usize,
        xf: &Bound<'py, PyAny>,
        layer: Bound<'py, PyAny>,
        mut name: Bound<'py, PyAny>,
        identity: Bound<'py, PyAny>,
        depth: u8,
    ) -> PyResult<Bound<'py, PyAny>> {
        let rec = self
            .records
            .get(index)
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("unknown SKP prototype"))?
            .clone();
        if !name.is_truthy()? {
            name = opt(&rec, "name")?;
        }
        let group = self.group.call1((&self.meshes[index], name))?;
        self.assign_uid(&group, &identity)?;
        group.setattr("xform", xf)?;
        group.setattr(
            "component",
            rec.call_method1("get", ("component", true))?.is_truthy()?,
        )?;
        if layer.is_truthy()? {
            group.setattr("layer", layer)?;
        }
        if depth < 32 {
            let children = PyList::empty(xf.py());
            for child in rows(&rec, "children")?.try_iter()? {
                let child = child?;
                let local = opt(&child, "identity")?;
                let id = if local.is_none() || identity.is_none() {
                    xf.py().None().into_bound(xf.py())
                } else {
                    let seq = xf.py().get_type::<pyo3::types::PyTuple>();
                    seq.call1((&identity,))?.add(seq.call1((local,))?)?
                };
                children.append(self.place(
                    child.get_item("proto")?.extract()?,
                    &child.get_item("xform")?,
                    opt(&child, "layer")?,
                    opt(&child, "name")?,
                    id,
                    depth + 1,
                )?)?;
            }
            if !children.is_empty() {
                group.call_method1("adopt", (children,))?;
            }
        }
        Ok(group)
    }
}

#[pyfunction]
pub fn skp_apply_geometry<'py>(
    scene: &Bound<'py, PyAny>,
    payload: &Bound<'py, PyDict>,
    make_mesh: &Bound<'py, PyAny>,
    release: &Bound<'py, PyAny>,
    tick: &Bound<'py, PyAny>,
) -> PyResult<()> {
    let build = |rec: &Bound<'py, PyAny>| -> PyResult<Bound<'py, PyAny>> {
        let mesh = make_mesh.call1((
            rec.get_item("faces")?,
            opt(rec, "soft_edges")?,
            opt(rec, "loose_edges")?,
            opt(rec, "curves")?,
        ))?;
        release.call1((rec,))?;
        Ok(mesh)
    };
    apply_geometry(scene, payload, build, |step| {
        tick.call1((step,))?;
        Ok(())
    })
}

fn apply_geometry<'py>(
    scene: &Bound<'py, PyAny>,
    payload: &Bound<'py, PyDict>,
    mut build: impl FnMut(&Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>>,
    mut report: impl FnMut(usize) -> PyResult<()>,
) -> PyResult<()> {
    let py = scene.py();
    let roots = scene.getattr("groups")?;
    let used = scene
        .call_method0("groups_by_uid")?
        .try_iter()?
        .map(|v| v?.extract::<String>())
        .collect::<PyResult<HashSet<_>>>()?;
    let mut defs = Definitions {
        records: rows(payload, "protos")?.extract()?,
        meshes: Vec::new(),
        group: py.import("core.group")?.getattr("Group")?,
        ids: used,
        namespace: opt(payload, "identity_namespace")?,
        uuid: py.import("uuid")?.into_any(),
    };
    for row in rows(payload, "groups")?.try_iter()? {
        let row = row?;
        report(1)?;
        let mesh = build(&row)?;
        if !mesh.getattr("faces")?.is_truthy()? && !mesh.getattr("edges")?.is_truthy()? {
            continue;
        }
        let group = defs.group.call1((mesh, opt(&row, "name")?))?;
        defs.assign_uid(&group, &opt(&row, "identity")?)?;
        let layer = opt(&row, "layer")?;
        if layer.is_truthy()? {
            group.setattr("layer", layer)?;
        }
        let axes = opt(&row, "axes")?;
        if axes.is_instance_of::<PyList>() && axes.len()? == 16 {
            let columns = axes.extract::<[f64; 16]>()?;
            let values = (0..16).map(|i| columns[(i % 4) * 4 + i / 4]);
            let matrix = py
                .get_type::<crate::math::Matrix4>()
                .call1(pyo3::types::PyTuple::new(py, values)?)?;
            if !matrix.call_method0("isIdentity")?.is_truthy()? {
                group.setattr("axes", matrix)?;
            }
        }
        if opt(&row, "billboard")?.is_truthy()? {
            group.setattr("billboard", "mesh")?;
        }
        roots.call_method1("append", (group,))?;
    }
    for row in &defs.records {
        report(1)?;
        defs.meshes.push(build(row)?);
    }
    for index in 0..defs.records.len() {
        let row = defs.records[index].clone();
        let mesh = &defs.meshes[index];
        if !mesh.getattr("faces")?.is_truthy()?
            && !mesh.getattr("edges")?.is_truthy()?
            && !rows(&row, "children")?.is_truthy()?
        {
            continue;
        }
        report(1)?;
        let layers = rows(&row, "instance_layers")?;
        let names = rows(&row, "instance_names")?;
        let identities = rows(&row, "instance_identities")?;
        let at = |items: &Bound<'py, PyAny>, i| -> PyResult<Bound<'py, PyAny>> {
            if i < items.len()? {
                items.get_item(i)
            } else {
                Ok(py.None().into_bound(py))
            }
        };
        for (i, xf) in rows(&row, "instances")?.try_iter()?.enumerate() {
            if i % 128 == 0 && i > 0 {
                py.detach(std::thread::yield_now);
                report(0)?;
            }
            let group = defs.place(
                index,
                &xf?,
                at(&layers, i)?,
                at(&names, i)?,
                at(&identities, i)?,
                0,
            )?;
            roots.call_method1("append", (group,))?;
        }
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(scene,payload,progress=None,*,consume=None))]
pub fn skp_apply<'py>(
    scene: &Bound<'py, PyAny>,
    payload: &Bound<'py, PyDict>,
    progress: Option<&Bound<'py, PyAny>>,
    consume: Option<bool>,
) -> PyResult<String> {
    let py = scene.py();
    let consume = match consume {
        Some(v) => v,
        None => opt(payload, "_consume_geometry")?.is_truthy()?,
    };
    let mut remaining = HashMap::new();
    let mut total = 0;
    for (key, weight) in [("groups", 1), ("protos", 2)] {
        let items = rows(payload, key)?;
        total += items.len()? * weight;
        if consume {
            for row in items.try_iter()? {
                *remaining.entry(row?.as_ptr()).or_insert(0usize) += 1;
            }
        }
    }
    let create = py.import("core.store_mesh")?.getattr("empty_store_mesh")?;
    let mut done = 0usize;
    let mut last = None;
    skp_apply_metadata(scene, payload)?;
    apply_geometry(
        scene,
        payload,
        |rec| {
            let mesh = super::skp_definition(
                py,
                create.call0()?,
                &rec.get_item("faces")?,
                &opt(rec, "soft_edges")?,
                &opt(rec, "loose_edges")?,
                &opt(rec, "curves")?,
            )?;
            if let Some(count) = remaining.get_mut(&rec.as_ptr()) {
                *count -= 1;
                if *count == 0 {
                    rec.set_item("faces", PyList::empty(py))?;
                    for key in ["soft_edges", "loose_edges", "curves"] {
                        rec.call_method1("pop", (key, py.None()))?;
                    }
                }
            }
            Ok(mesh)
        },
        |step| {
            done += step;
            if let Some(report) = progress {
                let now = std::time::Instant::now();
                if last
                    .is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_millis(100))
                {
                    report.call1((
                        0.75 + 0.21 * (done as f64 / total.max(1) as f64).min(1.),
                        "Building geometry…",
                    ))?;
                    last = Some(now);
                }
            }
            Ok(())
        },
    )?;
    if consume {
        payload.set_item("groups", PyList::empty(py))?;
        payload.set_item("protos", PyList::empty(py))?;
    }
    let back = opt(payload, "back_color")?;
    if back.is_truthy()? && scene.getattr("back_face_color")?.is_none() {
        scene.setattr(
            "back_face_color",
            py.get_type::<pyo3::types::PyTuple>().call1((back,))?,
        )?;
    }
    for dim in scene.getattr("dimensions")?.try_iter()? {
        let dim = dim?;
        if dim.hasattr("bind")? {
            dim.call_method1("bind", (scene,))?;
        }
    }
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    payload.call_method1("get", ("backend", "?"))?.extract()
}
