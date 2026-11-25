// SPDX-License-Identifier: GPL-3.0-or-later
//! The resident scene as it is packed for the GPU (`PackedScene`), from
//! `views/wgpu_scene.py`.
//!
//! The ABI is little endian: 28-byte vertices, 32-byte prototype bounds,
//! 80-byte instances and 20-byte indexed indirect commands. Alpha zero
//! marks unpainted faces so container tint never overwrites explicit face
//! colour; alpha 1..255 is rounded effective opacity. Instance `info[2]`:
//! bit 0 suppresses ordinary drawing, bit 3 is billboard, bit 4 marks an
//! object hidden by Hide (descendants too), bit 5 means tag, navigator or
//! edit filtering also hides it. Materials are 16-byte prototype headers
//! followed by 32-byte corner records.
//!
//! The buffers are NumPy arrays (the document readers still index them by
//! field); this side reads and writes their bytes in place.
use numpy::{PyArray1, PyArrayDescrMethods, PyReadonlyArray2, PyUntypedArray, PyUntypedArrayMethods};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyTuple},
};

type Obj<'py> = Bound<'py, PyAny>;

/// Bytes of one instance row: a 4×4 f32 world matrix and four u32 of info.
pub const INSTANCE_BYTES: usize = 80;
/// Bytes of one indexed indirect command: count, instances, first, base, start.
pub const COMMAND_BYTES: usize = 20;
/// Bytes of one material header (base, first, length, flags) and of the
/// unit `first` counts in.
const MATERIAL_UNIT: usize = 16;
const MATERIAL_RECORD: usize = 32;
pub const OBJECT_HIDDEN: u32 = 1 << 4;
pub const VISIBILITY_FILTERED: u32 = 1 << 5;

/// The array's bytes: it must be C-contiguous (every packed buffer is).
fn bytes<'a>(array: &'a Obj<'_>) -> PyResult<&'a [u8]> {
    let untyped = array.cast::<PyUntypedArray>()?;
    if !untyped.is_c_contiguous() {
        return Err(PyValueError::new_err("packed buffers are contiguous"));
    }
    let len = untyped.len() * untyped.dtype().itemsize();
    // SAFETY: a contiguous array's data pointer spans its whole extent; the
    // array outlives the borrow, and the GIL is held for it.
    Ok(unsafe { std::slice::from_raw_parts((*untyped.as_array_ptr()).data.cast::<u8>(), len) })
}

/// The array's bytes, to write in place.
#[allow(clippy::mut_from_ref)]
fn bytes_mut<'a>(array: &'a Obj<'_>) -> PyResult<&'a mut [u8]> {
    let untyped = array.cast::<PyUntypedArray>()?;
    if !untyped.is_c_contiguous() {
        return Err(PyValueError::new_err("packed buffers are contiguous"));
    }
    let len = untyped.len() * untyped.dtype().itemsize();
    // SAFETY: as `bytes`; nothing else reads the buffer while the GIL is
    // held by this call.
    Ok(unsafe { std::slice::from_raw_parts_mut((*untyped.as_array_ptr()).data.cast::<u8>(), len) })
}

fn u32_at(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(raw[at..at + 4].try_into().expect("four bytes"))
}

fn empty_dict(py: Python<'_>, value: Option<Py<PyAny>>) -> Py<PyAny> {
    value.filter(|v| !v.is_none(py)).unwrap_or_else(|| PyDict::new(py).into_any().unbind())
}

#[pyclass(module = "ingetrazo_core", dict, get_all, set_all)]
pub struct PackedScene {
    vertices: Py<PyAny>,
    indices: Py<PyAny>,
    bounds: Py<PyAny>,
    commands: Py<PyAny>,
    edge_commands: Py<PyAny>,
    instances: Py<PyAny>,
    uids: Py<PyAny>,
    world_bounds: Py<PyAny>,
    timings: Py<PyAny>,
    groups: Py<PyAny>,
    materials: Py<PyAny>,
    textures: Py<PyAny>,
    texture_paths: Py<PyAny>,
    hidden_commands: Py<PyAny>,
    hidden_edge_commands: Py<PyAny>,
    profile_exclusions: Py<PyAny>,
    reveal_hidden: Py<PyAny>,
    prototype_lookup: Py<PyAny>,
    /// Code -> the baked paint its vertices carry (None: plain). An edit
    /// rebakes each variant of a changed mesh with its own paint.
    prototype_paints: Py<PyAny>,
    /// Sparse replacements since the last whole-table handoff. Rebuilding
    /// this stream on each edit copies every unrelated material record.
    material_updates: Py<PyAny>,
    origin: Py<PyAny>,
    prototype_origins: Py<PyAny>,
    model_translations: Py<PyAny>,
    bake_focus: Py<PyAny>,
    occurrence_keys: Py<PyAny>,
    /// id(mesh) -> (mesh, a prototype code it is drawn with, -1: nothing to
    /// draw): every mesh the pack met, the loose one and empty ones included.
    mesh_codes: Py<PyAny>,
}

impl PackedScene {
    /// The record of a finished pack: `field` hands over what the assembly
    /// computed, the rest is what the walk gathered.
    #[allow(clippy::too_many_arguments)]
    pub fn assembled(
        py: Python<'_>,
        field: &dyn Fn(&str) -> PyResult<Py<PyAny>>,
        uids: Py<PyAny>,
        groups: Py<PyAny>,
        textures: Py<PyAny>,
        texture_paths: Py<PyAny>,
        reveal_hidden: Py<PyAny>,
        prototype_lookup: Py<PyAny>,
        prototype_paints: Py<PyAny>,
        bake_focus: Py<PyAny>,
        occurrence_keys: Py<PyAny>,
        mesh_codes: Py<PyAny>,
    ) -> PyResult<Self> {
        Ok(Self {
            vertices: field("vertices")?,
            indices: field("indices")?,
            bounds: field("bounds")?,
            commands: field("commands")?,
            edge_commands: field("edge_commands")?,
            instances: field("instances")?,
            uids,
            world_bounds: field("world_bounds")?,
            timings: py.None(),
            groups,
            materials: field("materials")?,
            textures,
            texture_paths,
            hidden_commands: field("hidden_commands")?,
            hidden_edge_commands: field("hidden_edge_commands")?,
            profile_exclusions: field("profile_exclusions")?,
            reveal_hidden,
            prototype_lookup,
            prototype_paints,
            material_updates: PyDict::new(py).into_any().unbind(),
            origin: field("origin")?,
            prototype_origins: field("prototype_origins")?,
            model_translations: field("model_translations")?,
            bake_focus,
            occurrence_keys,
            mesh_codes,
        })
    }

    fn upload_arrays<'py>(&self, py: Python<'py>) -> PyResult<(Bound<'py, PyTuple>, Obj<'py>)> {
        let np = py.import("numpy")?;
        let raw = |array: &Obj<'py>| -> PyResult<Obj<'py>> {
            array.call_method1("view", (np.getattr("uint8")?,))?.call_method1("reshape", (-1,))
        };
        let commands = np.call_method1("concatenate", ((self.commands.bind(py), self.edge_commands.bind(py)),))?;
        let prototypes = PyTuple::new(
            py,
            [raw(self.vertices.bind(py))?, raw(self.indices.bind(py))?, raw(self.bounds.bind(py))?, raw(&commands)?],
        )?;
        Ok((prototypes, raw(self.instances.bind(py))?))
    }
}

#[pymethods]
impl PackedScene {
    #[new]
    #[pyo3(signature = (vertices, indices, bounds, commands, edge_commands, instances, uids, world_bounds,
        timings = None, groups = None, materials = None, textures = None, texture_paths = None,
        hidden_commands = None, hidden_edge_commands = None, profile_exclusions = None, reveal_hidden = None,
        prototype_lookup = None, prototype_paints = None, material_updates = None, origin = None,
        prototype_origins = None, model_translations = None, bake_focus = None, occurrence_keys = None,
        mesh_codes = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        vertices: Py<PyAny>,
        indices: Py<PyAny>,
        bounds: Py<PyAny>,
        commands: Py<PyAny>,
        edge_commands: Py<PyAny>,
        instances: Py<PyAny>,
        uids: Py<PyAny>,
        world_bounds: Py<PyAny>,
        timings: Option<Py<PyAny>>,
        groups: Option<Py<PyAny>>,
        materials: Option<Py<PyAny>>,
        textures: Option<Py<PyAny>>,
        texture_paths: Option<Py<PyAny>>,
        hidden_commands: Option<Py<PyAny>>,
        hidden_edge_commands: Option<Py<PyAny>>,
        profile_exclusions: Option<Py<PyAny>>,
        reveal_hidden: Option<Py<PyAny>>,
        prototype_lookup: Option<Py<PyAny>>,
        prototype_paints: Option<Py<PyAny>>,
        material_updates: Option<Py<PyAny>>,
        origin: Option<Py<PyAny>>,
        prototype_origins: Option<Py<PyAny>>,
        model_translations: Option<Py<PyAny>>,
        bake_focus: Option<Py<PyAny>>,
        occurrence_keys: Option<Py<PyAny>>,
        mesh_codes: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let none = || py.None();
        let empty = || PyTuple::empty(py).into_any().unbind();
        Ok(Self {
            vertices,
            indices,
            bounds,
            commands,
            edge_commands,
            instances,
            uids,
            world_bounds,
            timings: timings.unwrap_or_else(none),
            groups: groups.unwrap_or_else(none),
            materials: materials.unwrap_or_else(none),
            textures: textures.unwrap_or_else(none),
            texture_paths: texture_paths.unwrap_or_else(empty),
            hidden_commands: hidden_commands.unwrap_or_else(none),
            hidden_edge_commands: hidden_edge_commands.unwrap_or_else(none),
            profile_exclusions: profile_exclusions.unwrap_or_else(empty),
            reveal_hidden: reveal_hidden.unwrap_or_else(|| (false, false).into_pyobject(py).expect("tuple").into_any().unbind()),
            prototype_lookup: prototype_lookup.unwrap_or_else(none),
            prototype_paints: prototype_paints.unwrap_or_else(none),
            material_updates: empty_dict(py, material_updates),
            origin: origin.unwrap_or_else(|| (0., 0., 0.).into_pyobject(py).expect("tuple").into_any().unbind()),
            prototype_origins: prototype_origins.unwrap_or_else(empty),
            model_translations: model_translations.unwrap_or_else(none),
            bake_focus: bake_focus.unwrap_or_else(none),
            occurrence_keys: empty_dict(py, occurrence_keys),
            mesh_codes: empty_dict(py, mesh_codes),
        })
    }

    /// A copy of the record with `fields` replaced: the buffers it does not
    /// name are shared with this one.
    #[pyo3(signature = (**fields))]
    fn replace<'py>(slf: &Bound<'py, Self>, fields: Option<&Bound<'py, PyDict>>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let copy = {
            let s = slf.borrow();
            let c = |value: &Py<PyAny>| value.clone_ref(py);
            Bound::new(py, Self {
                vertices: c(&s.vertices), indices: c(&s.indices), bounds: c(&s.bounds), commands: c(&s.commands),
                edge_commands: c(&s.edge_commands), instances: c(&s.instances), uids: c(&s.uids),
                world_bounds: c(&s.world_bounds), timings: c(&s.timings), groups: c(&s.groups),
                materials: c(&s.materials), textures: c(&s.textures), texture_paths: c(&s.texture_paths),
                hidden_commands: c(&s.hidden_commands), hidden_edge_commands: c(&s.hidden_edge_commands),
                profile_exclusions: c(&s.profile_exclusions), reveal_hidden: c(&s.reveal_hidden),
                prototype_lookup: c(&s.prototype_lookup), prototype_paints: c(&s.prototype_paints),
                material_updates: c(&s.material_updates), origin: c(&s.origin),
                prototype_origins: c(&s.prototype_origins), model_translations: c(&s.model_translations),
                bake_focus: c(&s.bake_focus), occurrence_keys: c(&s.occurrence_keys), mesh_codes: c(&s.mesh_codes),
            })?
        };
        if let Some(fields) = fields {
            for (name, value) in fields.iter() {
                copy.as_any().setattr(name.extract::<String>()?.as_str(), value)?;
            }
        }
        Ok(copy)
    }

    /// `copy.copy`: the same buffers under a record of its own, with what
    /// was noted on this one (a staged upload, the snap sources).
    fn __copy__<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let copy = Self::replace(slf, None)?;
        copy.as_any().getattr("__dict__")?.call_method1("update", (slf.as_any().getattr("__dict__")?,))?;
        Ok(copy)
    }

    /// The key of `group`'s resident row: its path where a uid repeats.
    fn row_key<'py>(&self, py: Python<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
        let own = match group.getattr_opt("uid")? {
            Some(uid) => uid,
            None => "loose".into_pyobject(py)?.into_any(),
        };
        self.occurrence_keys.bind(py).call_method1("get", (group, own))
    }

    /// Project the original placement translations into this view frame.
    fn rebase(&mut self, py: Python<'_>, origin: &Obj<'_>) -> PyResult<bool> {
        let current = self.origin.bind(py);
        if current.eq(origin)? {
            return Ok(false);
        }
        let to: [f64; 3] = origin.extract()?;
        let from: [f64; 3] = current.extract()?;
        {
            let translations: PyReadonlyArray2<'_, f64> = self.model_translations.bind(py).extract()?;
            let translations = translations.as_array();
            let rows = bytes_mut(self.instances.bind(py))?;
            if translations.nrows() * INSTANCE_BYTES != rows.len() || translations.ncols() != 3 {
                return Err(PyValueError::new_err("one translation per resident row"));
            }
            for (row, translation) in rows.chunks_exact_mut(INSTANCE_BYTES).zip(translations.outer_iter()) {
                for axis in 0..3 {
                    // world[3, axis]: the fourth row of the 4×4 f32 matrix.
                    let at = 48 + axis * 4;
                    row[at..at + 4].copy_from_slice(&((translation[axis] - to[axis]) as f32).to_le_bytes());
                }
            }
        }
        let shift = [from[0] - to[0], from[1] - to[1], from[2] - to[2]];
        self.world_bounds = self.world_bounds.bind(py).add(PyArray1::from_slice(py, &shift))?.unbind();
        self.origin = origin.clone().unbind();
        Ok(true)
    }

    /// The whole material table: the last one with the sparse replacements
    /// since (`material_updates`) folded in.
    fn material_stream<'py>(&mut self, py: Python<'py>) -> PyResult<Obj<'py>> {
        let updates = self.material_updates.bind(py).clone().cast_into::<PyDict>()?;
        if updates.is_empty() {
            return Ok(self.materials.bind(py).clone());
        }
        let commands = bytes(self.commands.bind(py))?;
        let count = commands.len() / COMMAND_BYTES;
        let materials = self.materials.bind(py).clone();
        let old = bytes(&materials)?;
        let mut headers = vec![0u8; count * MATERIAL_UNIT];
        let mut records = Vec::new();
        let mut at = count;
        for code in 0..count {
            let header = code * MATERIAL_UNIT;
            let replaced = updates.get_item(code)?;
            let (flags, chunk): (u32, std::borrow::Cow<'_, [u8]>) = match &replaced {
                Some(update) => {
                    let data = update.get_item(1)?;
                    (update.get_item(0)?.extract()?, std::borrow::Cow::Owned(bytes(&data)?.to_vec()))
                }
                None => {
                    let first = u32_at(old, header + 4) as usize * MATERIAL_UNIT;
                    let length = u32_at(old, header + 8) as usize * MATERIAL_RECORD;
                    let chunk = old.get(first..first + length).ok_or_else(|| PyValueError::new_err("material table out of range"))?;
                    (u32_at(old, header + 12), std::borrow::Cow::Borrowed(chunk))
                }
            };
            headers[header + 12..header + 16].copy_from_slice(&flags.to_le_bytes());
            if !chunk.is_empty() {
                // base, where the records start, how many.
                let base = &commands[code * COMMAND_BYTES + 12..code * COMMAND_BYTES + 16];
                headers[header..header + 4].copy_from_slice(base);
                headers[header + 4..header + 8].copy_from_slice(&(at as u32).to_le_bytes());
                headers[header + 8..header + 12].copy_from_slice(&((chunk.len() / MATERIAL_RECORD) as u32).to_le_bytes());
                records.extend_from_slice(&chunk);
                at += chunk.len() / MATERIAL_UNIT;
            }
        }
        headers.extend_from_slice(&records);
        let stream = PyArray1::from_vec(py, headers).into_any();
        self.materials = stream.clone().unbind();
        self.material_updates = PyDict::new(py).into_any().unbind();
        Ok(stream)
    }

    fn _upload_arrays<'py>(&self, py: Python<'py>) -> PyResult<(Bound<'py, PyTuple>, Obj<'py>)> {
        self.upload_arrays(py)
    }

    /// Validate and stage the geometry off the frame that installs it.
    fn prepare_upload(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let (prototypes, instances) = {
            let this = slf.borrow();
            if this.instances.bind(py).len()? == 0 {
                return Ok(());
            }
            this.upload_arrays(py)?
        };
        let prepared = py.import("ingetrazo_render")?.getattr("SceneUpload")?.call1((prototypes, instances))?;
        slf.as_any().setattr("_prepared_upload", prepared)
    }

    fn upload<'py>(slf: &Bound<'py, Self>, renderer: &Obj<'py>) -> PyResult<()> {
        let py = slf.py();
        let own = slf.as_any().getattr("__dict__")?;
        let prepared = own.call_method1("pop", ("_prepared_upload", py.None()))?;
        let (prototypes, instances) = slf.borrow().upload_arrays(py)?;
        if prepared.is_none() {
            renderer.call_method1("set_scene", (prototypes, instances))?;
        } else {
            renderer.call_method1("set_prepared_scene", (prepared, instances))?;
        }
        let stream = slf.borrow_mut().material_stream(py)?;
        renderer.call_method1("set_materials", (stream,))?;
        let textures = py.get_type::<PyTuple>().call1((slf.borrow().textures.bind(py),))?.cast_into::<PyTuple>()?;
        renderer.getattr("set_textures")?.call1(textures)?;
        if renderer.hasattr("set_profile_exclusions")? {
            renderer.call_method1("set_profile_exclusions", (slf.borrow().profile_exclusions.bind(py),))?;
        }
        let parity = py.import("views.wgpu_parity")?;
        if renderer.hasattr("set_material_transparency")? {
            renderer.call_method1("set_material_transparency", (parity.call_method1("has_transparency", (slf,))?,))?;
        }
        let kw = PyDict::new(py);
        kw.set_item("renderer", renderer)?;
        kw.set_item("packed", slf)?;
        parity.call_method1("hidden_geometry", (py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&kw))?,))?;
        Ok(())
    }
}

/// The buffers' record layouts, as NumPy dtypes (`VERTEX`, `INSTANCE`,
/// `COMMAND` on the module).
pub fn dtype<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("ingetrazo_core")?.getattr(name)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let np = py.import("numpy")?;
    let layout = |fields: Vec<(&str, &str, Option<Vec<usize>>)>| -> PyResult<Obj<'_>> {
        let list = pyo3::types::PyList::empty(py);
        for (name, kind, shape) in fields {
            match shape {
                Some(shape) if shape.len() == 1 => list.append((name, kind, shape[0]))?,
                Some(shape) => list.append((name, kind, PyTuple::new(py, shape)?))?,
                None => list.append((name, kind))?,
            }
        }
        np.call_method1("dtype", (list,))
    };
    m.add("VERTEX", layout(vec![("pos", "<f4", Some(vec![3])), ("normal", "<f4", Some(vec![3])), ("color", "u1", Some(vec![4]))])?)?;
    m.add("INSTANCE", layout(vec![("world", "<f4", Some(vec![4, 4])), ("info", "<u4", Some(vec![4]))])?)?;
    m.add("COMMAND", layout(vec![("count", "<u4", None), ("instances", "<u4", None), ("first", "<u4", None), ("base", "<i4", None), ("start", "<u4", None)])?)?;
    m.add("OBJECT_HIDDEN", OBJECT_HIDDEN)?;
    m.add("VISIBILITY_FILTERED", VISIBILITY_FILTERED)?;
    m.add_class::<PackedScene>()
}
