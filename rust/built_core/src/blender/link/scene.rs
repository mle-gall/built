// SPDX-License-Identifier: GPL-3.0-or-later
//! The Built document seen by the Blender link: which top-level placements
//! Blender must receive again (fingerprints, narrowed by the change feed),
//! their GLB export in local coordinates, the current values of the kinds
//! Blender may send back, and the single History step that applies them.
use crate::math::{mat, Vec3};
use glam::{DMat3, DMat4, DQuat, DVec3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PyString},
};
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;

type Obj<'py> = Bound<'py, PyAny>;

/// The chunk name of the loose geometry (not a placement).
pub const LOOSE: &str = "loose";

fn matrix_of(g: &Obj<'_>) -> PyResult<DMat4> {
    let m = g.getattr("xform")?;
    if m.is_none() {
        Ok(DMat4::IDENTITY)
    } else {
        mat(&m)
    }
}

fn mesh_revision(mesh: &Obj<'_>) -> PyResult<u64> {
    if let Some(d) = mesh.getattr_opt("__dict__")? {
        if let Ok(d) = d.cast::<PyDict>() {
            if let Some(store) = d.get_item("_store")?.filter(|s| !s.is_none()) {
                if let Some(r) = store.getattr_opt("revision")? {
                    return Ok(r.extract::<u64>().unwrap_or(0));
                }
            }
        }
    }
    Ok(mesh.getattr_opt("_mut_serial")?.and_then(|s| s.extract::<u64>().ok()).unwrap_or(0))
}

fn hash_str(h: &mut DefaultHasher, o: &Obj<'_>) -> PyResult<()> {
    if o.is_none() {
        0u8.hash(h);
    } else {
        o.str()?.to_cow()?.hash(h);
    }
    Ok(())
}

/// Which top-level placements Blender must receive again. Fingerprints
/// cover a placement's geometry (meshes, their revisions and the feed's
/// mesh notices, paint, nested placements and their matrices), never its
/// own matrix, which travels on its own (`transform`).
#[pyclass(module = "built_core.render_blender")]
pub struct LinkTracker {
    session: u64,
    bumps: HashMap<usize, u64>,
    sent: HashMap<String, (String, Option<[f64; 16]>)>,
    top_of: HashMap<String, String>,
    dirty: HashSet<String>,
    full: bool,
}

impl LinkTracker {
    fn fingerprint(&self, g: &Obj<'_>, h: &mut DefaultHasher, depth: usize) -> PyResult<()> {
        if depth > 512 {
            return Err(PyValueError::new_err("group hierarchy too deep"));
        }
        hash_str(h, &g.getattr("name")?)?;
        g.getattr("hidden")?.is_truthy()?.hash(h);
        hash_str(h, &g.getattr("material")?)?;
        hash_str(h, &g.getattr("billboard")?)?;
        let mesh = g.getattr("mesh")?;
        let ptr = mesh.as_ptr() as usize;
        ptr.hash(h);
        mesh_revision(&mesh)?.hash(h);
        self.bumps.get(&ptr).copied().unwrap_or(0).hash(h);
        for child in g.getattr("children")?.try_iter()? {
            let child = child?;
            for v in matrix_of(&child)?.to_cols_array() {
                v.to_bits().hash(h);
            }
            self.fingerprint(&child, h, depth + 1)?;
        }
        Ok(())
    }

    fn print(&self, g: &Obj<'_>) -> PyResult<String> {
        let mut h = DefaultHasher::new();
        self.session.hash(&mut h);
        self.fingerprint(g, &mut h, 0)?;
        Ok(format!("{:016x}", h.finish()))
    }

    fn loose_print(&self, mesh: &Obj<'_>) -> PyResult<String> {
        let mut h = DefaultHasher::new();
        self.session.hash(&mut h);
        let ptr = mesh.as_ptr() as usize;
        ptr.hash(&mut h);
        mesh_revision(mesh)?.hash(&mut h);
        self.bumps.get(&ptr).copied().unwrap_or(0).hash(&mut h);
        Ok(format!("{:016x}", h.finish()))
    }

    fn map_tree(&mut self, top: &str, g: &Obj<'_>, depth: usize) -> PyResult<()> {
        if depth > 512 {
            return Ok(());
        }
        for child in g.getattr("children")?.try_iter()? {
            let child = child?;
            self.top_of.insert(child.getattr("uid")?.extract()?, top.to_owned());
            self.map_tree(top, &child, depth + 1)?;
        }
        Ok(())
    }
}

fn uids(o: Option<Obj<'_>>) -> PyResult<Vec<String>> {
    let Some(o) = o.filter(|o| !o.is_none()) else { return Ok(Vec::new()) };
    o.try_iter()?.map(|u| u?.extract::<String>()).collect()
}

fn field<'py>(o: &Obj<'py>, k: &str) -> PyResult<Option<Obj<'py>>> {
    if let Ok(d) = o.cast::<PyDict>() {
        return d.get_item(k);
    }
    o.getattr_opt(k)
}

#[pymethods]
impl LinkTracker {
    #[new]
    fn new() -> Self {
        // One salt per Built process: a Blender reconnecting in the same
        // session keeps what it holds; after a restart (mesh revisions
        // start again) it receives everything once.
        static SESSION: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let session = *SESSION.get_or_init(|| {
            let mut h = DefaultHasher::new();
            std::time::SystemTime::now().hash(&mut h);
            std::process::id().hash(&mut h);
            h.finish()
        });
        Self { session, bumps: HashMap::new(), sent: HashMap::new(), top_of: HashMap::new(), dirty: HashSet::new(), full: true }
    }

    /// A (re)connected add-on: what it already holds, by uid → fingerprint.
    fn reset(&mut self, have: HashMap<String, String>) {
        self.sent = have.into_iter().map(|(k, v)| (k, (v, None))).collect();
        self.full = true;
        self.dirty.clear();
    }

    /// Look at every placement on the next plan (feed steps were missed).
    fn widen(&mut self) {
        self.full = true;
    }

    /// Forget a chunk (its export failed): the next plan sends it again.
    fn forget(&mut self, uid: &str) {
        self.sent.remove(uid);
    }

    /// Placements Blender holds.
    fn held(&self) -> Vec<String> {
        let mut v: Vec<_> = self.sent.keys().cloned().collect();
        v.sort();
        v
    }

    /// Read one change-feed step: the placements it names narrow the next
    /// plan; meshes, new or removed placements and incomplete steps widen
    /// it to a fingerprint pass over every top-level placement.
    fn observe(&mut self, step: &Obj<'_>) -> PyResult<()> {
        if !step.getattr_opt("complete")?.map_or(Ok(false), |c| c.is_truthy())? {
            self.full = true;
        }
        if let Some(meshes) = step.getattr_opt("meshes")? {
            for change in meshes.try_iter()? {
                let change = change?;
                let mut mesh = change.getattr("mesh")?;
                if mesh.is_callable() {
                    mesh = mesh.call0()?;
                }
                if !mesh.is_none() {
                    *self.bumps.entry(mesh.as_ptr() as usize).or_default() += 1;
                }
                self.full = true;
            }
        }
        if let Some(p) = step.getattr_opt("placements")? {
            if !uids(field(&p, "created")?)?.is_empty() || !uids(field(&p, "removed")?)?.is_empty() {
                self.full = true;
            }
            for uid in uids(field(&p, "modified")?)? {
                match self.top_of.get(&uid) {
                    Some(top) => {
                        self.dirty.insert(top.clone());
                    }
                    None => {
                        self.dirty.insert(uid);
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether the next plan has anything to look at.
    fn pending(&self) -> bool {
        self.full || !self.dirty.is_empty()
    }

    /// What to send: `chunk` (geometry, with the matrix), `transform` (the
    /// matrix alone) or `remove`, per top-level placement and the loose
    /// geometry. Marks them sent.
    fn plan<'py>(&mut self, scene: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
        let py = scene.py();
        let out = PyList::empty(py);
        let full = std::mem::take(&mut self.full);
        let dirty = std::mem::take(&mut self.dirty);
        if full {
            self.top_of.clear();
        }
        let mut seen = HashSet::new();
        for g in scene.getattr("groups")?.try_iter()? {
            let g = g?;
            let uid: String = g.getattr("uid")?.extract()?;
            if full {
                self.top_of.insert(uid.clone(), uid.clone());
                self.map_tree(&uid, &g, 0)?;
            } else if !dirty.contains(&uid) {
                continue;
            }
            seen.insert(uid.clone());
            if !scene.call_method1("entity_visible", (&g,))?.is_truthy()? {
                if self.sent.remove(&uid).is_some() {
                    out.append(action(py, "remove", &uid, None, None, None)?)?;
                }
                continue;
            }
            let fp = self.print(&g)?;
            let m = matrix_of(&g)?.to_cols_array();
            let name: String = g.getattr("name")?.str()?.extract()?;
            match self.sent.get(&uid) {
                Some((old, _)) if *old != fp => out.append(action(py, "chunk", &uid, Some(&name), Some(m), Some(&fp))?)?,
                None => out.append(action(py, "chunk", &uid, Some(&name), Some(m), Some(&fp))?)?,
                Some((_, sent_m)) if sent_m.is_none_or(|s| s.iter().zip(m).any(|(a, b)| (a - b).abs() > 1e-9)) => {
                    out.append(action(py, "transform", &uid, Some(&name), Some(m), None)?)?
                }
                _ => {}
            }
            self.sent.insert(uid, (fp, Some(m)));
        }
        if full {
            let loose = scene.getattr("loose_mesh")?;
            let has_faces = loose.getattr("faces")?.len()? > 0;
            if has_faces {
                let fp = self.loose_print(&loose)?;
                if self.sent.get(LOOSE).is_none_or(|(old, _)| *old != fp) {
                    out.append(action(py, "chunk", LOOSE, Some("Loose geometry"), Some(DMat4::IDENTITY.to_cols_array()), Some(&fp))?)?;
                }
                self.sent.insert(LOOSE.into(), (fp, Some(DMat4::IDENTITY.to_cols_array())));
            }
            seen.insert(LOOSE.to_owned());
            let gone: Vec<String> = self.sent.keys().filter(|k| !seen.contains(*k)).cloned().collect();
            for uid in gone {
                self.sent.remove(&uid);
                out.append(action(py, "remove", &uid, None, None, None)?)?;
            }
        } else {
            // A dirty uid that is no longer top level: removed or nested now.
            for uid in dirty.into_iter().filter(|u| !seen.contains(u)) {
                if self.sent.contains_key(&uid) {
                    self.full = true;
                }
            }
        }
        Ok(out)
    }
}

fn action<'py>(py: Python<'py>, op: &str, uid: &str, name: Option<&str>, m: Option<[f64; 16]>, fp: Option<&str>) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("op", op)?;
    d.set_item("uid", uid)?;
    if let Some(n) = name {
        d.set_item("name", n)?;
    }
    if let Some(m) = m {
        d.set_item("matrix", m.to_vec())?;
    }
    if let Some(fp) = fp {
        d.set_item("fp", fp)?;
    }
    Ok(d)
}

/// Write the named top-level placements (and `"loose"`) as one GLB in their
/// local coordinates, each root node carrying `built_uid` in its extras.
/// Returns the uids written.
#[pyfunction]
pub fn export_chunks(scene: &Obj<'_>, uids: Vec<String>, path: PathBuf) -> PyResult<Vec<String>> {
    let py = scene.py();
    let wanted: HashSet<&str> = uids.iter().map(String::as_str).collect();
    let parts = PyList::empty(py);
    let mut written = Vec::new();
    if wanted.contains(LOOSE) && scene.getattr("loose_mesh")?.getattr("faces")?.len()? > 0 {
        parts.append(LOOSE)?;
        written.push(LOOSE.to_owned());
    }
    for g in scene.getattr("groups")?.try_iter()? {
        let g = g?;
        let uid: String = g.getattr("uid")?.extract()?;
        if wanted.contains(uid.as_str()) {
            parts.append(&g)?;
            written.push(uid);
        }
    }
    if written.is_empty() {
        return Ok(written);
    }
    let kw = PyDict::new(py);
    kw.set_item("link", parts)?;
    py.import("built_core.formats.gltf")?.call_method("save_glb", (scene, path), Some(&kw))?;
    Ok(written)
}

// ------------------------------------------------------------ values

static CRC: Mutex<Option<HashMap<(PathBuf, u64, u128), String>>> = Mutex::new(None);

/// A file's CRC-32 as eight hex digits, cached by path, size and mtime.
pub fn file_crc(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let key = (path.to_path_buf(), meta.len(), mtime);
    if let Some(v) = CRC.lock().unwrap().get_or_insert_with(HashMap::new).get(&key) {
        return Some(v.clone());
    }
    let crc = format!("{:08x}", crc32fast::hash(&std::fs::read(path).ok()?));
    CRC.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, crc.clone());
    Some(crc)
}

fn get_f64(o: &Obj<'_>, k: &str, default: f64) -> PyResult<f64> {
    match o.call_method1("get", (k,))? {
        v if v.is_none() => Ok(default),
        v => v.extract(),
    }
}

/// A material as Blender sees it after import: base colour (null over an
/// image), roughness, metallic and the image's name and CRC.
fn material_value<'py>(py: Python<'py>, m: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let tex = m.getattr("texture")?;
    let image = if tex.is_truthy()? { tex.call_method1("get", ("path",))? } else { py.None().into_bound(py) };
    let color = m.getattr("color")?;
    if image.is_truthy()? || color.is_none() {
        out.set_item("color", py.None())?;
    } else {
        let c: Vec<f64> = color.try_iter()?.take(3).map(|v| v?.extract()).collect::<PyResult<_>>()?;
        out.set_item("color", c)?;
    }
    let pbr = m.getattr("pbr")?;
    let finish = m.getattr_opt("finish")?.filter(|f| f.is_truthy().unwrap_or(false));
    let (rough, metal) = if pbr.is_truthy()? {
        (get_f64(&pbr, "roughness", 1.)?, get_f64(&pbr, "metallic", 0.)?)
    } else if let Some(f) = finish {
        let id: String = f.extract()?;
        match crate::material_finish::lookup(&id) {
            Some(x) => (x.pbr[0], x.pbr[1]),
            None => (1., 0.),
        }
    } else {
        (1., 0.)
    };
    out.set_item("roughness", rough)?;
    out.set_item("metallic", metal)?;
    if image.is_truthy()? {
        let p: PathBuf = image.extract()?;
        let t = PyDict::new(py);
        t.set_item("name", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())?;
        t.set_item("crc", file_crc(&p))?;
        t.set_item("path", p.to_string_lossy())?;
        out.set_item("texture", t)?;
    } else {
        out.set_item("texture", py.None())?;
    }
    Ok(out)
}

fn materials<'py>(scene: &Obj<'py>) -> PyResult<Vec<(String, Obj<'py>)>> {
    let mats = scene.getattr("materials")?;
    let mut out = Vec::new();
    if mats.is_none() {
        return Ok(out);
    }
    let names = if mats.hasattr("keys")? { mats.call_method0("keys")? } else { mats.clone() };
    for name in names.try_iter()? {
        let name = name?;
        let m = mats.call_method1("get", (&name,))?;
        if !m.is_none() {
            out.push((name.extract()?, m));
        }
    }
    Ok(out)
}

fn lights<'py>(scene: &Obj<'py>, key: &str) -> PyResult<(Bound<'py, PyDict>, Bound<'py, PyList>)> {
    let py = scene.py();
    let data = scene.getattr_opt("plugin_data")?;
    let raw = match data.filter(|d| d.is_truthy().unwrap_or(false)) {
        Some(d) if d.contains(key)? => d.get_item(key)?,
        _ => PyDict::new(py).into_any(),
    };
    // A private copy: applying edits it before the History step stores it.
    let json = py.import("json")?;
    let state = json.call_method1("loads", (json.call_method1("dumps", (raw,))?,))?;
    let state = state.cast_into::<PyDict>().unwrap_or_else(|_| PyDict::new(py));
    let list = state.get_item("lights")?.unwrap_or_else(|| PyList::empty(py).into_any());
    let cleaned = super::super::lights::clean_lights(&list)?;
    Ok((state, cleaned))
}

fn light_id(lt: &Obj<'_>, i: usize) -> PyResult<String> {
    Ok(match lt.call_method1("get", ("id",))? {
        v if v.is_truthy()? => v.extract()?,
        _ => format!("index{i}"),
    })
}

fn light_value<'py>(py: Python<'py>, lt: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for k in ["kind", "pos", "dir", "color", "power", "angle", "name", "on"] {
        out.set_item(k, lt.get_item(k)?)?;
    }
    Ok(out)
}

fn camera_value<'py>(py: Python<'py>, view: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let cam = py.import("built_core.camera")?.getattr("OrbitCamera")?.call0()?;
    view.call_method1("apply_camera", (&cam,))?;
    let d = super::super::camera_dict(&cam)?;
    let out = PyDict::new(py);
    for k in ["eye", "target", "up", "fov_deg", "perspective"] {
        out.set_item(k, d.get_item(k)?)?;
    }
    Ok(out)
}

/// Built's current value for every key of the four kinds Blender may send
/// back (`transform:`, `material:`, `light:`, `camera:`), or only `keys`.
#[pyfunction]
#[pyo3(signature=(scene, plugin, keys=None))]
pub fn link_values<'py>(scene: &Obj<'py>, plugin: &str, keys: Option<Vec<String>>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let out = PyDict::new(py);
    let wanted: Option<HashSet<String>> = keys.map(|k| k.into_iter().collect());
    let want_kind = |kind: &str| wanted.as_ref().is_none_or(|w| w.iter().any(|k| k.starts_with(kind)));
    let want = |key: &str| wanted.as_ref().is_none_or(|w| w.contains(key));
    if want_kind("transform:") {
        for g in scene.getattr("groups")?.try_iter()? {
            let g = g?;
            let key = format!("transform:{}", g.getattr("uid")?.extract::<String>()?);
            if want(&key) {
                let v = PyDict::new(py);
                v.set_item("matrix", matrix_of(&g)?.to_cols_array().to_vec())?;
                out.set_item(key, v)?;
            }
        }
    }
    if want_kind("material:") {
        for (name, m) in materials(scene)? {
            let key = format!("material:{name}");
            if want(&key) {
                out.set_item(key, material_value(py, &m)?)?;
            }
        }
    }
    if want_kind("light:") {
        let (_, list) = lights(scene, plugin)?;
        for (i, lt) in list.iter().enumerate() {
            let key = format!("light:{}", light_id(&lt, i)?);
            if want(&key) {
                out.set_item(key, light_value(py, &lt)?)?;
            }
        }
    }
    if want_kind("camera:") {
        for view in scene.getattr("saved_views")?.try_iter()? {
            let view = view?;
            let key = format!("camera:{}", view.getattr("name")?.extract::<String>()?);
            if want(&key) && !out.contains(&key)? {
                out.set_item(key, camera_value(py, &view)?)?;
            }
        }
    }
    if let Some(w) = wanted {
        for k in w {
            if !out.contains(&k)? {
                out.set_item(k, py.None())?;
            }
        }
    }
    Ok(out)
}

// ------------------------------------------------------------ apply

fn vec3(py: Python<'_>, v: DVec3) -> PyResult<Obj<'_>> {
    Ok(Bound::new(py, Vec3::from(v))?.into_any())
}

fn history<'py>(py: Python<'py>, name: &str, args: impl pyo3::call::PyCallArgs<'py>) -> PyResult<Obj<'py>> {
    py.import("built_core.history")?.call_method1(name, args)
}

fn floats(v: &Obj<'_>) -> PyResult<Vec<f64>> {
    v.try_iter()?.map(|x| x?.extract::<f64>()).collect()
}

/// Scale (along the old axes, about the old origin), rotate (about the old
/// origin) and move: the existing placement steps that take `old` to `new`.
fn transform_steps<'py>(py: Python<'py>, g: &Obj<'py>, old: DMat4, new: DMat4) -> PyResult<Vec<Obj<'py>>> {
    let (s0, r0, t0) = old.to_scale_rotation_translation();
    let (s1, r1, t1) = new.to_scale_rotation_translation();
    if old.determinant() * new.determinant() <= 0. {
        return Err(PyValueError::new_err("A mirrored transform stays in Blender"));
    }
    let mut steps = Vec::new();
    let k = s1 / s0;
    if (k - DVec3::ONE).abs().max_element() > 1e-9 {
        let axes = DMat3::from_quat(r0);
        let frame = PyList::empty(py);
        for c in 0..3 {
            frame.append(vec3(py, axes.col(c))?)?;
        }
        steps.push(history(py, "ScaleGroupCommand", (g, vec3(py, t0)?, (k.x, k.y, k.z), frame))?);
    }
    let q: DQuat = (r1 * r0.inverse()).normalize();
    let (axis, angle) = q.to_axis_angle();
    if angle.abs() > 1e-9 && axis.length() > 0.5 {
        steps.push(history(py, "RotateGroupCommand", (g, vec3(py, t0)?, vec3(py, axis)?, angle.to_degrees()))?);
    }
    if (t1 - t0).length() > 1e-12 {
        steps.push(history(py, "MoveGroupCommand", (g, vec3(py, t1 - t0)?))?);
    }
    Ok(steps)
}

fn value_of<'py>(item: &Obj<'py>, k: &str) -> PyResult<Obj<'py>> {
    item.get_item("value")?.call_method1("get", (k,))
}

/// Build the History step that applies accepted Blender changes: one
/// compound command (one undo entry). Returns `(command or None, applied
/// keys, {key: why it could not apply})`.
#[pyfunction]
#[pyo3(signature=(scene, items, plugin, textures=None))]
pub fn link_apply<'py>(
    scene: &Obj<'py>,
    items: &Obj<'py>,
    plugin: &str,
    textures: Option<HashMap<String, PathBuf>>,
) -> PyResult<(Obj<'py>, Vec<String>, Bound<'py, PyDict>)> {
    let py = scene.py();
    let textures = textures.unwrap_or_default();
    let failed = PyDict::new(py);
    let mut applied = Vec::new();
    let mut steps: Vec<Obj<'py>> = Vec::new();
    let groups: HashMap<String, Obj<'py>> = scene
        .getattr("groups")?
        .try_iter()?
        .map(|g| {
            let g = g?;
            Ok((g.getattr("uid")?.extract::<String>()?, g))
        })
        .collect::<PyResult<_>>()?;
    let mats = scene.getattr("materials")?;
    let (mut state, mut light_list) = (None, None);
    let mut lights_changed = false;
    let views: Vec<Obj<'py>> = scene.getattr("saved_views")?.try_iter()?.collect::<PyResult<_>>()?;
    for item in items.try_iter()? {
        let item = item?;
        let key: String = item.get_item("key")?.extract()?;
        let (kind, name) = key.split_once(':').unwrap_or(("", ""));
        let result: PyResult<()> = (|| {
            match kind {
                "transform" => {
                    let g = groups.get(name).ok_or_else(|| PyValueError::new_err("The placement is no longer in Built"))?;
                    if g.getattr_opt("locked")?.is_some_and(|l| l.is_truthy().unwrap_or(false)) {
                        return Err(PyValueError::new_err("The placement is locked in Built"));
                    }
                    let m = floats(&value_of(&item, "matrix")?)?;
                    let new = DMat4::from_cols_slice(m.get(..16).ok_or_else(|| PyValueError::new_err("bad matrix"))?);
                    if !new.is_finite() {
                        return Err(PyValueError::new_err("bad matrix"));
                    }
                    steps.extend(transform_steps(py, g, matrix_of(g)?, new)?);
                }
                "material" => {
                    let m = if mats.is_none() { py.None().into_bound(py) } else { mats.call_method1("get", (name,))? };
                    if m.is_none() {
                        return Err(PyValueError::new_err("Built has no material by this name"));
                    }
                    let changes = PyDict::new(py);
                    let pbr = match m.getattr("pbr")? {
                        p if p.is_truthy()? => py.get_type::<PyDict>().call1((p,))?.cast_into::<PyDict>()?,
                        _ => PyDict::new(py),
                    };
                    let mut pbr_changed = false;
                    for k in ["roughness", "metallic"] {
                        let v = value_of(&item, k)?;
                        if !v.is_none() {
                            pbr.set_item(k, v.extract::<f64>()?.clamp(0., 1.))?;
                            pbr_changed = true;
                        }
                    }
                    if pbr_changed {
                        changes.set_item("pbr", &pbr)?;
                    }
                    let tex = value_of(&item, "texture")?;
                    let has_new_image = !tex.is_none() && textures.contains_key(&key);
                    if has_new_image {
                        let old = m.getattr("texture")?;
                        let size = |k: &str| -> PyResult<f64> {
                            if old.is_truthy()? { get_f64(&old, k, 1.) } else { Ok(1.) }
                        };
                        let t = PyDict::new(py);
                        t.set_item("path", textures[&key].to_string_lossy())?;
                        t.set_item("sw", size("sw")?)?;
                        t.set_item("sh", size("sh")?)?;
                        changes.set_item("texture", t)?;
                        if !pbr_changed && m.getattr("pbr")?.is_none() {
                            changes.set_item("color", py.None())?;
                        }
                    }
                    let color = value_of(&item, "color")?;
                    if !color.is_none() && !has_new_image {
                        let c = floats(&color)?;
                        if c.len() < 3 {
                            return Err(PyValueError::new_err("bad colour"));
                        }
                        changes.set_item("color", (c[0].clamp(0., 1.), c[1].clamp(0., 1.), c[2].clamp(0., 1.)))?;
                    }
                    if changes.is_empty() {
                        return Ok(());
                    }
                    let new = py.import("dataclasses")?.call_method("replace", (&m,), Some(&changes))?;
                    steps.push(history(py, "RestampMaterialCommand", (name, new))?);
                }
                "light" => {
                    if state.is_none() {
                        let (s, l) = lights(scene, plugin)?;
                        state = Some(s);
                        light_list = Some(l);
                    }
                    let list = light_list.as_ref().unwrap();
                    let v = item.get_item("value")?;
                    let kind = v.call_method1("get", ("kind",))?;
                    if !(kind.eq("point")? || kind.eq("spot")?) {
                        return Err(PyValueError::new_err("Only point and spot lights come to Built"));
                    }
                    let mut at = None;
                    for (i, lt) in list.iter().enumerate() {
                        if light_id(&lt, i)? == name {
                            at = Some(i);
                        }
                    }
                    let fresh = PyDict::new(py);
                    if let Some(i) = at {
                        fresh.call_method1("update", (list.get_item(i)?,))?;
                    }
                    for k in ["kind", "pos", "dir", "color", "power", "angle", "name", "on"] {
                        let x = v.call_method1("get", (k,))?;
                        if !x.is_none() {
                            fresh.set_item(k, x)?;
                        }
                    }
                    if !v.call_method1("get", ("color",))?.is_none() {
                        fresh.set_item("kelvin", py.None())?;
                    }
                    if !name.starts_with("index") {
                        fresh.set_item("id", name)?;
                    }
                    let cleaned = super::super::lights::clean_lights(&PyList::new(py, [&fresh])?.into_any())?;
                    let one = cleaned.get_item(0).map_err(|_| PyValueError::new_err("bad light"))?;
                    match at {
                        Some(i) => list.set_item(i, one)?,
                        None => list.append(one)?,
                    }
                    lights_changed = true;
                }
                "camera" => {
                    let v = item.get_item("value")?;
                    let kw = PyDict::new(py);
                    kw.set_item("fov_deg", v.call_method1("get", ("fov_deg",))?.extract::<Option<f64>>()?.unwrap_or(45.))?;
                    kw.set_item("perspective", v.call_method1("get", ("perspective",))?.is_truthy()? || v.call_method1("get", ("perspective",))?.is_none())?;
                    let eye: [f64; 3] = floats(&v.get_item("eye")?)?.try_into().map_err(|_| PyValueError::new_err("bad eye"))?;
                    let target = PyList::new(py, floats(&v.get_item("target")?)?)?;
                    let up = PyList::new(py, floats(&v.get_item("up")?)?)?;
                    let fresh = py
                        .import("built_core.saved_views")?
                        .call_method("from_lookat", (name, eye, &target, &up), Some(&kw))?;
                    match views.iter().find(|w| w.getattr("name").and_then(|n| n.eq(name)).unwrap_or(false)) {
                        Some(view) => {
                            let cam = py.import("built_core.camera")?.getattr("OrbitCamera")?.call0()?;
                            fresh.call_method1("apply_camera", (&cam,))?;
                            steps.push(history(py, "UpdateSceneCommand", (view, cam))?);
                        }
                        None => steps.push(history(py, "AddSceneCommand", (fresh,))?),
                    }
                }
                _ => return Err(PyValueError::new_err("Built does not take this kind of change")),
            }
            Ok(())
        })();
        match result {
            Ok(()) => applied.push(key),
            Err(e) => failed.set_item(key, e.value(py).str()?)?,
        }
    }
    if lights_changed {
        let state = state.unwrap();
        state.set_item("lights", light_list.unwrap())?;
        steps.push(history(py, "SetPluginDataCommand", (plugin, state))?);
    }
    let command = match steps.len() {
        0 => py.None().into_bound(py),
        1 => steps.pop().unwrap(),
        _ => history(py, "CompoundCommand", (PyList::new(py, steps)?,))?,
    };
    Ok((command, applied, failed))
}

#[pyfunction]
pub fn link_new_id(py: Python<'_>) -> Bound<'_, PyString> {
    let t = super::wire::new_token().replace('-', "").to_lowercase();
    PyString::new(py, &t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decomposition_matches_the_target_pose() {
        // Built's steps compose as: scale about t0 along R0, rotate about t0, move.
        let old = DMat4::from_scale_rotation_translation(DVec3::new(1., 2., 1.), DQuat::from_rotation_z(0.3), DVec3::new(1., 2., 3.));
        let new = DMat4::from_scale_rotation_translation(DVec3::new(2., 2., 0.5), DQuat::from_rotation_x(0.2) * DQuat::from_rotation_z(0.9), DVec3::new(-1., 0.5, 4.));
        let (s0, r0, t0) = old.to_scale_rotation_translation();
        let (s1, r1, t1) = new.to_scale_rotation_translation();
        let k = s1 / s0;
        let axes = DMat3::from_quat(r0);
        let about = |m: DMat4| DMat4::from_translation(t0) * m * DMat4::from_translation(-t0);
        let scale = about(DMat4::from_mat3(axes * DMat3::from_diagonal(k) * axes.transpose()));
        let rot = about(DMat4::from_quat(r1 * r0.inverse()));
        let moved = DMat4::from_translation(t1 - t0);
        let got = moved * rot * scale * old;
        assert!(got.abs_diff_eq(new, 1e-9), "{got:?} vs {new:?}");
    }
}
