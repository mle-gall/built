// SPDX-License-Identifier: GPL-3.0-or-later
//! The resident scene as it is packed for the GPU (`PackedScene`).
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
//! The buffers are Rust allocations seen through NumPy arrays (the document
//! readers index them by field); this side reads and writes their bytes in
//! place. Vertices and indices can have spare room past their end (`Room`),
//! so a rebaked prototype that outgrows its range is appended without
//! copying the model.
use numpy::{
    ndarray::ArrayView1, PyArray1, PyArrayDescrMethods, PyArrayMethods, PyReadonlyArray2, PyUntypedArray,
    PyUntypedArrayMethods,
};
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
    fn fields(&self) -> [&Py<PyAny>; 26] {
        [&self.vertices, &self.indices, &self.bounds, &self.commands, &self.edge_commands, &self.instances, &self.uids,
         &self.world_bounds, &self.timings, &self.groups, &self.materials, &self.textures, &self.texture_paths,
         &self.hidden_commands, &self.hidden_edge_commands, &self.profile_exclusions, &self.reveal_hidden,
         &self.prototype_lookup, &self.prototype_paints, &self.material_updates, &self.origin, &self.prototype_origins,
         &self.model_translations, &self.bake_focus, &self.occurrence_keys, &self.mesh_codes]
    }

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

    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        for field in self.fields() {
            visit.call(field)?;
        }
        Ok(())
    }
    fn __clear__(&mut self, py: Python<'_>) {
        // The groups, meshes and paints are the document's: let them go.
        for field in [&mut self.uids, &mut self.groups, &mut self.prototype_lookup, &mut self.prototype_paints,
                      &mut self.material_updates, &mut self.occurrence_keys, &mut self.mesh_codes, &mut self.textures,
                      &mut self.texture_paths, &mut self.profile_exclusions] {
            *field = py.None();
        }
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
        let noted = copy.as_any().getattr("__dict__")?;
        noted.call_method1("update", (slf.as_any().getattr("__dict__")?,))?;
        // The spare room stays with the record that grows into it.
        for key in ["_room", "_reserving"] {
            noted.call_method1("pop", (key, slf.py().None()))?;
        }
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

    /// The spare room past the vertices and indices, or None.
    #[getter]
    fn room<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        noted(slf, "_room")
    }
    /// The pending reservation of that room (`reserve`), or None.
    #[getter]
    fn reserving<'py>(slf: &Bound<'py, Self>) -> PyResult<Obj<'py>> {
        noted(slf, "_reserving")
    }

    /// One more prototype, drawing nothing yet: its code.
    fn append_prototype(&mut self, py: Python<'_>) -> PyResult<usize> {
        let code = bytes(self.commands.bind(py))?.len() / COMMAND_BYTES;
        let mut bounds = bytes(self.bounds.bind(py))?.to_vec();
        bounds.extend_from_slice(&[0u8; 32]);
        let floats: Vec<f32> = bounds.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().expect("four bytes"))).collect();
        self.bounds = PyArray1::from_vec(py, floats).call_method1("reshape", (-1, 2, 4))?.unbind();
        for table in [&mut self.commands, &mut self.edge_commands, &mut self.hidden_commands, &mut self.hidden_edge_commands] {
            let mut rows = bytes(table.bind(py))?.to_vec();
            rows.extend_from_slice(&[0u8; COMMAND_BYTES]);
            *table = records(py, rows, "COMMAND")?.unbind();
        }
        Ok(code)
    }

    /// `count` more resident rows, zeroed.
    fn append_rows(&mut self, py: Python<'_>, count: usize) -> PyResult<()> {
        let mut rows = bytes(self.instances.bind(py))?.to_vec();
        rows.resize(rows.len() + count * INSTANCE_BYTES, 0);
        self.instances = records(py, rows, "INSTANCE")?.unbind();
        Ok(())
    }

    /// Copy the resident vertices and indices into ones with spare room on
    /// `executor`, ahead of the first edit that appends. The first append
    /// otherwise copies the whole model on the UI thread (65-200 ms on a
    /// 32 M vertex model). Called on entering a group: the intent to edit,
    /// and nothing is held twice for a model only looked at.
    fn reserve<'py>(slf: &Bound<'py, Self>, executor: &Obj<'py>) -> PyResult<()> {
        let py = slf.py();
        if !noted(slf, "_room")?.is_none() || !noted(slf, "_reserving")?.is_none() {
            return Ok(());
        }
        let (vertices, indices) = {
            let s = slf.borrow();
            (s.vertices.clone_ref(py), s.indices.clone_ref(py))
        };
        let copy = pyo3::types::PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<Py<PyAny>> {
            let py = args.py();
            let (source_v, source_i) = (bytes(vertices.bind(py))?, bytes(indices.bind(py))?);
            let count = source_v.len() / VERTEX_BYTES;
            if source_i.len() != count * INDEX_BYTES {
                return Err(PyValueError::new_err("one index per resident vertex"));
            }
            // As raw bytes, without the GIL: the arrays are kept alive by
            // this job, and every write to them adopts the room first.
            let (from_v, from_i) = (source_v.as_ptr() as usize, source_i.as_ptr() as usize);
            let room = py.detach(move || {
                let mut room = Room::with_capacity(count + (count / 32).max(4096));
                // SAFETY: both sources span `count` elements (checked above).
                unsafe {
                    std::ptr::copy_nonoverlapping(from_v as *const u8, room.vertices.as_mut_ptr(), count * VERTEX_BYTES);
                    std::ptr::copy_nonoverlapping(from_i as *const u8, room.indices.as_mut_ptr().cast::<u8>(), count * INDEX_BYTES);
                }
                room
            });
            Ok((vertices.clone_ref(py), indices.clone_ref(py), Py::new(py, room)?).into_pyobject(py)?.into_any().unbind())
        })?;
        note(slf, "_reserving", &executor.call_method1("submit", (copy,))?)
    }

    /// Swap a finished reservation in: the old `(vertices, indices)` when it
    /// was adopted, None when none is pending or it no longer matches,
    /// False while it is still copying (and not `wait`).
    #[pyo3(signature = (wait = false))]
    fn adopt_reserved<'py>(slf: &Bound<'py, Self>, wait: bool) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let future = noted(slf, "_reserving")?;
        if future.is_none() {
            return Ok(py.None().into_bound(py));
        }
        if !wait && !future.call_method0("done")?.is_truthy()? {
            return Ok(pyo3::types::PyBool::new(py, false).to_owned().into_any());
        }
        note(slf, "_reserving", &py.None().into_bound(py))?;
        let Ok(result) = future.call_method0("result") else {
            return Ok(py.None().into_bound(py));
        };
        let (source_v, source_i, room) = (result.get_item(0)?, result.get_item(1)?, result.get_item(2)?.cast_into::<Room>()?);
        // Every write to these arrays goes through `place`, which adopts
        // first: identical arrays mean identical contents.
        let same = {
            let s = slf.borrow();
            s.vertices.bind(py).is(&source_v) && s.indices.bind(py).is(&source_i)
        };
        if !noted(slf, "_room")?.is_none() || !same {
            return Ok(py.None().into_bound(py));
        }
        let count = bytes(&source_v)?.len() / VERTEX_BYTES;
        let (vertices, indices) = room_views(&room, count)?;
        {
            let mut s = slf.borrow_mut();
            s.vertices = vertices.unbind();
            s.indices = indices.unbind();
        }
        note(slf, "_room", room.as_any())?;
        Ok((source_v, source_i).into_pyobject(py)?.into_any())
    }

    /// Write a rebaked prototype into the resident vertices and indices:
    /// its base.
    ///
    /// A rebake that fits the range its code owns goes back there (a moved
    /// face keeps its counts); a longer one is appended into the spare
    /// room, grown by a margin when full. Concatenating copied the whole
    /// model per edit.
    fn place<'py>(slf: &Bound<'py, Self>, code: usize, vertices: &Obj<'py>, indices: &Obj<'py>) -> PyResult<usize> {
        let py = slf.py();
        Self::adopt_reserved(slf, true)?;
        let np = py.import("numpy")?;
        let flat = |array: &Obj<'py>| -> PyResult<Obj<'py>> {
            np.call_method1("ascontiguousarray", (array,))?.call_method1("view", (np.getattr("uint8")?,))?.call_method1("reshape", (-1,))
        };
        let (new_v, new_i) = (flat(vertices)?, flat(indices)?);
        let (new_v, new_i) = (bytes(&new_v)?, bytes(&new_i)?);
        let total = new_v.len() / VERTEX_BYTES;
        if new_v.len() != total * VERTEX_BYTES || new_i.len() != total * INDEX_BYTES {
            return Err(PyValueError::new_err("one index per rebaked vertex"));
        }
        let used = {
            let s = slf.borrow();
            let tables = [&s.commands, &s.edge_commands, &s.hidden_commands, &s.hidden_edge_commands].map(|t| bytes(t.bind(py)));
            let [faces, edges, hidden_faces, hidden_edges] = tables;
            let tables = [faces?, edges?, hidden_faces?, hidden_edges?];
            let count_of = |table: &[u8], code: usize| u32_at(table, code * COMMAND_BYTES) as usize;
            let base_of = |code: usize| i32::from_le_bytes(tables[0][code * COMMAND_BYTES + 12..code * COMMAND_BYTES + 16].try_into().expect("four bytes"));
            let prototypes = tables[0].len() / COMMAND_BYTES;
            if code < prototypes {
                let owned: usize = tables.iter().map(|t| count_of(t, code)).sum();
                let base = base_of(code);
                // Two codes never share a range in the bake; check anyway,
                // a shared one must not be overwritten for both.
                let sharing = (0..prototypes).filter(|c| base_of(*c) == base && tables.iter().any(|t| count_of(t, *c) > 0)).count();
                if total <= owned && sharing == 1 && base >= 0 {
                    let base = base as usize;
                    let (target_v, target_i) = (bytes_mut(s.vertices.bind(py))?, bytes_mut(s.indices.bind(py))?);
                    let range = |unit: usize| base * unit..(base + total) * unit;
                    target_v.get_mut(range(VERTEX_BYTES)).ok_or_else(|| PyValueError::new_err("prototype range out of bounds"))?.copy_from_slice(new_v);
                    target_i.get_mut(range(INDEX_BYTES)).ok_or_else(|| PyValueError::new_err("prototype range out of bounds"))?.copy_from_slice(new_i);
                    return Ok(base);
                }
            }
            bytes(s.vertices.bind(py))?.len() / VERTEX_BYTES
        };
        let current = bytes(slf.borrow().vertices.bind(py))?.as_ptr();
        let room = noted(slf, "_room")?;
        let room = match room.cast_into::<Room>() {
            Ok(room) if room.borrow().vertices.as_ptr() == current && used + total <= room.borrow().capacity() => room,
            _ => {
                let mut fresh = Room::with_capacity(used + total + (total * 4).max(used / 32).max(4096));
                {
                    let s = slf.borrow();
                    fresh.vertices[..used * VERTEX_BYTES].copy_from_slice(bytes(s.vertices.bind(py))?);
                    let indices = bytes(s.indices.bind(py))?;
                    // SAFETY: the room holds at least `used` indices.
                    unsafe { std::ptr::copy_nonoverlapping(indices.as_ptr(), fresh.indices.as_mut_ptr().cast::<u8>(), used * INDEX_BYTES) };
                }
                Bound::new(py, fresh)?
            }
        };
        {
            let mut r = room.borrow_mut();
            r.vertices[used * VERTEX_BYTES..(used + total) * VERTEX_BYTES].copy_from_slice(new_v);
            // SAFETY: the room holds at least `used + total` indices.
            unsafe { std::ptr::copy_nonoverlapping(new_i.as_ptr(), r.indices.as_mut_ptr().add(used).cast::<u8>(), total * INDEX_BYTES) };
        }
        let (vertices, indices) = room_views(&room, used + total)?;
        {
            let mut s = slf.borrow_mut();
            s.vertices = vertices.unbind();
            s.indices = indices.unbind();
        }
        note(slf, "_room", room.as_any())?;
        Ok(used)
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

const VERTEX_BYTES: usize = 28;
const INDEX_BYTES: usize = 4;

/// Vertices and indices with spare room past the resident ones. The
/// allocations never move: the record's arrays are views of their start.
#[pyclass(module = "ingetrazo_core")]
pub struct Room {
    vertices: Box<[u8]>,
    indices: Box<[u32]>,
}

impl Room {
    fn with_capacity(elements: usize) -> Self {
        Self { vertices: vec![0u8; elements * VERTEX_BYTES].into_boxed_slice(), indices: vec![0u32; elements].into_boxed_slice() }
    }
    fn capacity(&self) -> usize {
        self.indices.len()
    }
}

#[pymethods]
impl Room {
    /// Vertices (and indices) the room holds.
    #[getter(capacity)]
    fn py_capacity(&self) -> usize {
        self.capacity()
    }
}

/// The first `count` vertices and indices of `room`, as the record's arrays.
fn room_views<'py>(room: &Bound<'py, Room>, count: usize) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = room.py();
    let (vertices, indices) = {
        let r = room.borrow();
        if count > r.capacity() {
            return Err(PyValueError::new_err("more vertices than the room holds"));
        }
        (r.vertices.as_ptr(), r.indices.as_ptr())
    };
    // SAFETY: the room's allocations never move or shrink, each array
    // keeps the room alive as its base, and `count` is within capacity.
    let (vertices, indices) = unsafe {
        (
            PyArray1::<u8>::borrow_from_array(&ArrayView1::from_shape_ptr(count * VERTEX_BYTES, vertices), room.clone().into_any()),
            PyArray1::<u32>::borrow_from_array(&ArrayView1::from_shape_ptr(count, indices), room.clone().into_any()),
        )
    };
    Ok((vertices.call_method1("view", (dtype(py, "VERTEX")?,))?, indices.into_any()))
}

/// A structured array over `bytes` (`INSTANCE` or `COMMAND`).
fn records<'py>(py: Python<'py>, bytes: Vec<u8>, layout: &str) -> PyResult<Obj<'py>> {
    PyArray1::from_vec(py, bytes).call_method1("view", (dtype(py, layout)?,))
}

/// What is noted on the record beside its fields (its `__dict__`).
fn noted<'py>(scene: &Bound<'py, PackedScene>, key: &str) -> PyResult<Obj<'py>> {
    scene.as_any().getattr("__dict__")?.call_method1("get", (key,))
}
fn note(scene: &Bound<'_, PackedScene>, key: &str, value: &Obj<'_>) -> PyResult<()> {
    scene.as_any().getattr("__dict__")?.set_item(key, value)
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
    m.add_class::<Room>()?;
    m.add_class::<PackedScene>()
}
