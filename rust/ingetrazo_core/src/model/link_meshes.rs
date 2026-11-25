// SPDX-License-Identifier: GPL-3.0-or-later
//! Reuse revision-matched geometry on linked-file reload. The stamp includes
//! material renames and a stable prototype identity independent of write order.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyList, PySet, PyTuple},
};
use std::collections::{BTreeSet, HashMap, HashSet};
type Obj<'p> = Bound<'p, PyAny>;

fn json<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = v.py();
    let kw = PyDict::new(py);
    kw.set_item("sort_keys", true)?;
    py.import("json")?
        .call_method("dumps", (v,), Some(&kw))?
        .call_method0("encode")
}
fn digest<'p>(
    block: &Obj<'p>,
    geom: &Obj<'p>,
    compressed: &HashMap<String, Vec<u8>>,
) -> PyResult<Obj<'p>> {
    let py = block.py();
    let hash = py.import("hashlib")?.call_method0("sha256")?;
    if block.contains("geom")? {
        let meta = py.get_type::<PyDict>().call1((block.get_item("geom")?,))?;
        let member = meta.call_method1("pop", ("member",))?.extract::<String>()?;
        if let Some(bytes) = compressed.get(&member) {
            hash.call_method1("update", (PyBytes::new(py, bytes),))?;
        } else {
            hash.call_method1("update", (geom.get_item(member)?,))?;
        }
        hash.call_method1("update", (json(&meta)?,))?;
    } else if block.contains("_arrays")? {
        let np = py.import("numpy")?;
        let arrays = block.get_item("_arrays")?;
        let names = arrays
            .try_iter()?
            .map(|n| n?.extract())
            .collect::<PyResult<BTreeSet<String>>>()?;
        for name in names {
            hash.call_method1("update", (PyBytes::new(py, name.as_bytes()),))?;
            let v = arrays.get_item(name)?;
            if v.is_instance(&np.getattr("ndarray")?)? {
                let dims =
                    (v.getattr("dtype")?.getattr("str")?, v.getattr("shape")?).into_pyobject(py)?;
                hash.call_method1("update", (dims.str()?.call_method0("encode")?,))?;
                if v.getattr("size")?.is_truthy()? {
                    hash.call_method1("update", (np.call_method1("ascontiguousarray", (v,))?,))?;
                }
            } else {
                hash.call_method1("update", (json(&v)?,))?;
            }
        }
    } else {
        let empty = PyList::empty(py);
        let pair = (
            block.call_method1("get", ("edges", &empty))?,
            block.call_method1("get", ("faces", empty))?,
        )
            .into_pyobject(py)?;
        hash.call_method1("update", (json(pair.as_any())?,))?;
    }
    hash.call_method0("digest")
}

fn compressed_stamp(archive: &Obj, member: &str) -> PyResult<Vec<u8>> {
    let py = archive.py();
    let info = archive.call_method1("getinfo", (member,))?;
    // Opening validates the local ZIP header and positions the archive at the compressed payload.
    let handle = archive.call_method1("open", (&info,))?;
    let result = (|| -> PyResult<Vec<u8>> {
        let fp = archive.getattr("fp")?;
        let hash = py.import("hashlib")?.call_method0("sha256")?;
        let mut remaining = info.getattr("compress_size")?.extract::<usize>()?;
        while remaining > 0 {
            let chunk = fp.call_method1("read", (remaining.min(1 << 20),))?;
            let n = chunk.len()?;
            if n == 0 || n > remaining {
                return Err(PyValueError::new_err("truncated geometry member"));
            }
            hash.call_method1("update", (chunk,))?;
            remaining -= n;
        }
        let mut bytes = hash.call_method0("digest")?.extract::<Vec<u8>>()?;
        let metadata = (
            info.getattr("compress_type")?,
            info.getattr("file_size")?,
            info.getattr("CRC")?,
        )
            .into_pyobject(py)?;
        bytes.extend_from_slice(metadata.str()?.to_str()?.as_bytes());
        Ok(bytes)
    })();
    handle.call_method0("close")?;
    result
}
#[pyclass(module = "ingetrazo_core.link_meshes")]
pub struct LinkMeshes {
    path: String,
    registry: Py<PyAny>,
    previous: Py<PyDict>,
    #[pyo3(get)]
    reused: Py<PySet>,
    filtered: bool,
    #[pyo3(get)]
    proto_uids: HashMap<i64, String>,
    #[pyo3(get)]
    renames: HashMap<String, String>,
    compressed: HashMap<String, Vec<u8>>,
}
fn entries<'p>(d: &Obj<'p>, name: &str) -> PyResult<Vec<Obj<'p>>> {
    let rows = d.call_method1("get", (name,))?;
    if rows.is_none() {
        Ok(vec![])
    } else {
        rows.try_iter()?.collect()
    }
}
#[pymethods]
impl LinkMeshes {
    #[new]
    #[pyo3(signature=(path,registry,previous=None,*,filtered=false))]
    fn new(
        path: &Obj,
        registry: Py<PyAny>,
        previous: Option<&Obj>,
        filtered: bool,
    ) -> PyResult<Self> {
        let py = path.py();
        let cache = PyDict::new(py);
        if let Some(root) = previous {
            for g in py
                .import("ingetrazo_core")?
                .call_method1("purge_iter_groups", (vec![root],))?
                .try_iter()?
            {
                let mesh = g?.getattr("mesh")?;
                let stamp = mesh
                    .getattr("__dict__")?
                    .call_method1("get", ("_link_geometry",))?;
                if stamp.is_truthy()? && stamp.get_item(1)?.eq(mesh.getattr("revision")?)? {
                    cache.set_item(stamp.get_item(0)?, mesh)?;
                }
            }
        }
        Ok(Self {
            path: path.str()?.to_string(),
            registry,
            previous: cache.unbind(),
            filtered,
            reused: PySet::empty(py)?.unbind(),
            proto_uids: HashMap::new(),
            renames: HashMap::new(),
            compressed: HashMap::new(),
        })
    }
    fn prepare(&mut self, materials: &Obj, payload: &Obj) -> PyResult<()> {
        let py = payload.py();
        self.proto_uids.clear();
        self.compressed.clear();
        self.renames.clear();
        let mut stack = entries(payload, "groups")?;
        stack.extend(entries(payload, "kept_components")?);
        while let Some(g) = stack.pop() {
            if !g.is_instance_of::<PyDict>() {
                continue;
            }
            let uid = g.call_method1("get", ("uid",))?;
            if uid.is_truthy()?
                && g.contains("proto")?
                && !g.call_method1("get", ("xform",))?.is_none()
            {
                let code = py
                    .get_type::<pyo3::types::PyInt>()
                    .call1((g.get_item("proto")?,))?
                    .extract()?;
                let uid = uid.str()?.to_string();
                let stored = self.proto_uids.entry(code).or_insert_with(|| uid.clone());
                if uid < *stored {
                    *stored = uid;
                }
            }
            stack.extend(entries(&g, "children")?);
        }
        let registry = py.get_type::<PyDict>().call1((self.registry.bind(py),))?;
        let register = py.import("core.materials")?.getattr("register")?;
        for pair in materials.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let name = pair.get_item(0)?.extract::<String>()?;
            let final_name = register
                .call1((&registry, pair.get_item(1)?))?
                .extract::<String>()?;
            if name != final_name {
                self.renames.insert(name, final_name);
            }
        }
        Ok(())
    }
    fn key<'p>(
        &self,
        block: &Obj<'p>,
        geom: &Obj<'p>,
        identity: &Obj<'p>,
    ) -> PyResult<Bound<'p, PyTuple>> {
        let py = block.py();
        let mut identity = identity.clone();
        if identity.get_item(0)?.eq("proto")? {
            let i = identity.get_item(1)?;
            if let Ok(i) = i.extract::<i64>() {
                if let Some(uid) = self.proto_uids.get(&i) {
                    identity = ("proto", uid).into_pyobject(py)?.into_any();
                }
            }
        }
        let mut renamed = BTreeSet::new();
        if !self.renames.is_empty() {
            let mut meta = block.call_method1("get", ("geom",))?;
            if !meta.is_truthy()? {
                meta = block
                    .call_method1("get", ("_arrays", PyDict::new(py)))?
                    .call_method1("get", ("meta",))?;
            }
            let attrs = if meta.is_truthy()? {
                entries(&meta, "face_attrs")?
            } else {
                entries(block, "faces")?
            };
            for row in attrs {
                let back = row.call_method1("get", ("back",))?;
                for face in
                    std::iter::once(&row).chain(back.is_instance_of::<PyDict>().then_some(&back))
                {
                    if let Ok(name) = face.call_method1("get", ("mat",))?.extract::<String>() {
                        if let Some(to) = self.renames.get(&name) {
                            renamed.insert((name, to));
                        }
                    }
                }
            }
        }
        (
            self.path.as_str(),
            PyTuple::new(py, renamed)?,
            identity,
            digest(block, geom, &self.compressed)?,
        )
            .into_pyobject(py)
    }
    fn unchanged_members<'p>(&mut self, archive: &Obj<'p>, payload: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = payload.py();
        let protos = entries(payload, "protos")?
            .into_iter()
            .enumerate()
            .map(|(i, p)| (p.as_ptr(), i))
            .collect::<HashMap<_, _>>();
        let mut unchanged = HashSet::new();
        let mut needed = HashSet::new();
        let empty = PyDict::new(py);
        for b in py
            .import("formats.igz")?
            .call_method1("_mesh_blocks", (payload,))?
            .try_iter()?
        {
            let b = b?;
            if !b.contains("geom")? {
                for e in entries(&b, "edges")? {
                    if e.call_method1("get", ("curve",))?
                        .is_instance_of::<pyo3::types::PyString>()
                    {
                        return Ok(PyTuple::empty(py).into_any());
                    }
                }
                continue;
            }
            let member = b
                .get_item("geom")?
                .get_item("member")?
                .extract::<String>()?;
            if !self.compressed.contains_key(&member) {
                self.compressed
                    .insert(member.clone(), compressed_stamp(archive, &member)?);
            }
            let identity = if b.is(payload) {
                ("loose",).into_pyobject(py)?
            } else if let Some(i) = protos.get(&b.as_ptr()) {
                ("proto", i).into_pyobject(py)?
            } else {
                ("group", b.call_method1("get", ("uid", ""))?.str()?).into_pyobject(py)?
            };
            if !self.filtered
                && self
                    .previous
                    .bind(py)
                    .contains(self.key(&b, &empty, &identity)?)?
            {
                unchanged.insert(member);
            } else {
                needed.insert(member);
            }
        }
        Ok(PySet::new(py, unchanged.difference(&needed))?.into_any())
    }
    fn read<'p>(
        &self,
        mesh: &Obj<'p>,
        block: &Obj<'p>,
        geom: &Obj<'p>,
        identity: &Obj<'p>,
    ) -> PyResult<Obj<'p>> {
        let py = mesh.py();
        let key = self.key(block, geom, identity)?;
        if !self.filtered {
            if let Some(old) = self.previous.bind(py).get_item(&key)? {
                self.reused.bind(py).add(&old)?;
                return Ok(old);
            }
        }
        py.import("formats.igz")?
            .call_method1("_load_mesh", (mesh, block, geom))?;
        mesh.getattr("__dict__")?
            .set_item("_link_geometry", (key, mesh.getattr("revision")?))?;
        Ok(mesh.clone())
    }
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "ingetrazo_core.link_meshes")?;
    sub.add_class::<LinkMeshes>()?;
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(sub.name()?, &sub)?;
    m.add("link_meshes", sub)
}
