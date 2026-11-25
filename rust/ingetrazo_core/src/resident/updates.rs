// SPDX-License-Identifier: GPL-3.0-or-later
//! Diff resident rows and mesh serials without baking record-only edits
//! (`InstanceUpdates`).
//!
//! Removed rows are tombstones: selections and GPU pick IDs never shift. A
//! batch of mutated prototypes uses one bake; transform previews only write
//! rows. A prototype is (mesh, baked paint, billboard): a changed mesh
//! rebakes every variant it has, each with its own paint, hidden faces and
//! edges included, so an edit in a nested, painted or hidden-geometry model
//! never repacks the scene.
//!
//! Entering a component instance (or a transformed container) swaps the
//! group's mesh for a world-space copy; leaving puts the shared definition
//! back. Both are *swaps*: the group keeps its row, paint and flags, only
//! its prototype and matrix change, so they are written from the open edit
//! path instead of walking every placement of the model.
//!
//! The tracker's state is the document's objects (meshes, groups, History
//! commands), kept as the instance's attributes.
use super::packed_scene::{dtype, COMMAND_BYTES, INSTANCE_BYTES};
use numpy::{PyArray1, PyArrayMethods, PyReadonlyArray1};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyCFunction, PyDict, PyList, PySet, PyTuple},
    IntoPyObjectExt,
};

type Obj<'py> = Bound<'py, PyAny>;
type Reply<'py> = PyResult<Obj<'py>>;

const REBASE_DISTANCE: f32 = 512.;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
/// `getattr(o, name, None)`.
fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}
fn id(object: &Obj<'_>) -> usize {
    object.as_ptr() as usize
}
fn core<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("ingetrazo_core")
}
fn dict<'py>(s: &Obj<'py>, name: &str) -> PyResult<Bound<'py, PyDict>> {
    Ok(s.getattr(name)?.cast_into::<PyDict>()?)
}
fn set<'py>(s: &Obj<'py>, name: &str) -> PyResult<Bound<'py, PySet>> {
    Ok(s.getattr(name)?.cast_into::<PySet>()?)
}
fn is_a(object: &Obj<'_>, classes: &[&Obj<'_>]) -> PyResult<bool> {
    for class in classes {
        if object.is_instance(class)? {
            return Ok(true);
        }
    }
    Ok(false)
}
/// The array's bytes (C-contiguous arrays only: the packed buffers are).
fn raw<'py>(py: Python<'py>, array: &Obj<'py>) -> PyResult<Vec<u8>> {
    let np = py.import("numpy")?;
    let flat: PyReadonlyArray1<'_, u8> = np.call_method1("ascontiguousarray", (array,))?
        .call_method1("view", (np.getattr("uint8")?,))?.call_method1("reshape", (-1,))?.extract()?;
    Ok(flat.as_slice()?.to_vec())
}
fn bytes_view<'py>(py: Python<'py>, array: &Obj<'py>) -> Reply<'py> {
    let np = py.import("numpy")?;
    np.call_method1("ascontiguousarray", (array,))?.call_method1("view", (np.getattr("uint8")?,))?.call_method1("reshape", (-1,))
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

/// The content version less the edit-context bumps: entering or leaving a
/// group swaps meshes without moving anything in the world (the view's
/// content token reads the same), so the journal stays usable across it.
fn resident_version<'py>(scene: &Obj<'py>) -> Reply<'py> {
    let bumps = match scene.getattr_opt("_placement_context_bumps")? {
        Some(bumps) => bumps,
        None => 0.into_bound_py_any(scene.py())?,
    };
    scene.getattr("content_version")?.sub(bumps)
}

fn has_vertices(mesh: &Obj<'_>) -> PyResult<bool> {
    let source = mesh.getattr("__dict__")?.call_method1("get", ("_deferred_source",))?;
    Ok(if source.is_none() { mesh.getattr("vertices")?.len()? } else { source.get_item("pos")?.len()? } != 0)
}

/// The History command classes the journal tells apart.
struct Commands<'py> {
    history: Bound<'py, PyModule>,
}
impl<'py> Commands<'py> {
    fn new(py: Python<'py>) -> PyResult<Self> {
        Ok(Self { history: py.import("core.history")? })
    }
    fn any(&self, command: &Obj<'py>, names: &[&str]) -> PyResult<bool> {
        for name in names {
            if command.is_instance(&self.history.getattr(*name)?)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
const MESH_EDITS: [&str; 6] = ["SnapshotMutation", "SnapshotCompound", "MoveVerticesCommand", "AddEdgeCommand", "AddFaceCommand", "EraseSelectionCommand"];

/// Commands that only change the mesh they ran on: inside a session folded
/// into a share-back, that is the world copy, drawn by no row.
fn edits_world_copy_only<'py>(classes: &Commands<'py>, command: &Obj<'py>) -> PyResult<bool> {
    if classes.any(command, &["CompoundCommand"])? {
        for inner in command.getattr("commands")?.try_iter()? {
            if !edits_world_copy_only(classes, &inner?)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    classes.any(command, &MESH_EDITS)
}

fn row_key<'py>(s: &Obj<'py>, group: &Obj<'py>) -> Reply<'py> {
    s.getattr("packed")?.call_method1("row_key", (group,))
}
fn children_hold(parent: &Obj<'_>, group: &Obj<'_>) -> PyResult<bool> {
    let children = attr(parent, "children")?;
    Ok(children.is_truthy()? && children.contains(group)?)
}

/// The journal's reading of one command: whether the pending sets now say
/// all it changed (else the next diff walks the whole model).
fn visit<'py>(s: &Obj<'py>, command: &Obj<'py>, phase: &Obj<'py>, step: &Obj<'py>, targets: &Obj<'py>) -> PyResult<bool> {
    let py = s.py();
    let classes = Commands::new(py)?;
    let scene = s.getattr("scene")?;
    let meshes = dict(s, "meshes")?;
    let roots = set(s, "roots")?;
    if classes.any(command, &["SetFaceTextureCommand", "SetFaceBackCommand", "SetFaceColorCommand", "SetFaceOpacityCommand"])? {
        // Attribute commands deliberately leave geometry serials alone.
        let mut target = targets.call_method1("get", (command,))?;
        if target.is_none() {
            let faces = py.get_type::<PySet>().call1((command.getattr("_faces")?,))?;
            let mesh = scene.getattr("mesh")?;
            let definition = match mesh.getattr_opt("_instance_definition")? {
                Some(definition) => definition,
                None => mesh.clone(),
            };
            let all = py.get_type::<PySet>().call1((mesh.getattr("faces")?,))?;
            if !faces.call_method1("issubset", (all,))?.is_truthy()? || !meshes.contains(id(&definition))? {
                return Ok(false);
            }
            target = id(&definition).into_bound_py_any(py)?;
            targets.set_item(command, &target)?;
        }
        // Undo may occur after leaving the group's edit context.
        set(s, "pending_materials")?.add(target)?;
        return Ok(true);
    }
    if classes.any(command, &["HideCommand", "AssignLayerCommand"])? {
        let entities = command.getattr("_entities")?;
        let group = py.import("core.group")?.getattr("Group")?;
        for entity in entities.try_iter()? {
            if !entity?.is_instance(&group)? {
                return Ok(false);
            }
        }
        // These commands also dirty all GL chunk revisions. The command
        // journal tells us only placements changed.
        let pending = set(s, "pending")?;
        if roots.call_method1("issuperset", (&entities,))?.is_truthy()? {
            pending.call_method1("update", (&entities,))?;
        } else {
            pending.call_method1("update", (&roots,))?;
        }
        let revisions = PyDict::new(py);
        for (key, entry) in meshes.iter() {
            revisions.set_item(key, entry.get_item(0)?.getattr("revision")?)?;
        }
        s.setattr("mesh_revisions", revisions)?;
        return Ok(true);
    }
    if classes.any(command, &["CompoundCommand"])? {
        for inner in command.getattr("commands")?.try_iter()? {
            if !visit(s, &inner?, phase, step, targets)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let edited = |mesh: &Obj<'py>| -> PyResult<bool> {
        let Some(old) = meshes.get_item(id(mesh))? else {
            // The open group's world copy, first edited.
            let chain = s.call_method1("_stack_chain", (mesh,))?;
            if chain.is_none() {
                return Ok(false);
            }
            let last = chain.get_item(chain.len()? - 1)?;
            dict(s, "pending_swaps")?.set_item(row_key(s, &last)?, chain)?;
            return Ok(true);
        };
        if old.get_item(2)?.extract::<i64>()? < 0 {
            // diff already checks the loose mesh for its first row.
            return Ok(mesh.is(&scene.getattr("loose_mesh")?));
        }
        set(s, "pending_meshes")?.add(id(mesh))?;
        Ok(true)
    };
    if classes.any(command, &MESH_EDITS)? {
        if !step.is_none() {
            // The meshes the step changed, as the feed names them.
            for change in step.getattr("meshes")?.try_iter()? {
                let mesh = change?.getattr("mesh")?;
                if !mesh.is_none() && !edited(&mesh)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        let mesh = if classes.any(command, &["SnapshotMutation"])? {
            command.call_method1("_target", (&scene,))?
        } else {
            scene.getattr("mesh")?
        };
        return edited(&mesh);
    }
    if command.is_instance(&py.import("ingetrazo_core.soften")?.getattr("SoftenByAngleCommand")?)? {
        // Soft flags only: each mesh's prototypes rebake, smooth.
        for mesh in command.call_method0("meshes")?.try_iter()? {
            if !edited(&mesh?)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let rebake_definition = || -> PyResult<()> {
        let proto = command.getattr("proto")?;
        if let Some(old) = meshes.get_item(id(&proto))? {
            if old.get_item(2)?.extract::<i64>()? >= 0 {
                set(s, "pending_meshes")?.add(id(&proto))?;
            }
        }
        Ok(())
    };
    if attr(command, "reference_instance")?.is_truthy()? {
        // Leaving an edited instance: back on the definition, which every
        // copy shows (all its variants rebake).
        if !s.call_method1("queue_swap", (command.getattr("group")?,))?.is_truthy()? {
            return Ok(false);
        }
        rebake_definition()?;
        return Ok(true);
    }
    if attr(command, "reference_assembly")?.is_truthy()? {
        // Leaving an edited assembly: its row goes back on the definition
        // and its parts' matrices come back up, which leaves every world
        // matrix where it was. What ran inside drew on the world copy,
        // which no row shows any more; a part edited inside shares back as
        // its own swap. Any other change to the parts (one deleted, moved,
        // exploded) rescans the placements.
        if !s.call_method1("queue_swap", (command.getattr("group")?,))?.is_truthy()? {
            return Ok(false);
        }
        for inner in command.getattr("inner")?.try_iter()? {
            let inner = inner?;
            if attr(&inner, "reference_instance")?.is_truthy()? || attr(&inner, "reference_assembly")?.is_truthy()? {
                if !visit(s, &inner, phase, step, targets)? {
                    return Ok(false);
                }
            } else if !edits_world_copy_only(&classes, &inner)? {
                return Ok(false);
            }
        }
        rebake_definition()?;
        return Ok(true);
    }
    if classes.any(command, &["InsertGroupCommand", "DeleteGroupCommand"])? && command.getattr("_owner")?.is(&scene.getattr("groups")?) {
        let adding = classes.any(command, &["InsertGroupCommand"])? != phase.eq("undo")?;
        let group = command.getattr("group")?;
        let (pending, removed) = (set(s, "pending")?, set(s, "pending_removed")?);
        if adding {
            roots.add(&group)?;
            pending.add(&group)?;
            removed.discard(&group)?;
        } else {
            roots.discard(&group)?;
            pending.discard(&group)?;
            removed.add(&group)?;
        }
        return Ok(true);
    }
    if classes.any(command, &["MoveGroupCommand"])? && roots.contains(command.getattr("group")?)? {
        set(s, "pending")?.add(command.getattr("group")?)?;
        return Ok(true);
    }
    if classes.any(command, &["UnlinkCommand"])? {
        // Make Unique of a link: the same rows (the parts keep their uids)
        // on meshes of their own — new prototypes for those, baked once;
        // undo goes back to the shared ones, still resident. Only the roots
        // holding them.
        let tops = command.getattr("tops")?;
        if !tops.is_truthy()? || !roots.call_method1("issuperset", (&tops,))?.is_truthy()? {
            return Ok(false);
        }
        set(s, "pending")?.call_method1("update", (tops,))?;
        return Ok(true);
    }
    classes.any(command, &["SetRecordCommand", "SetRegistryCommand", "AddGuideCommand", "ChangeGuideCommand", "DeleteGuidesCommand",
        "MoveGuidesCommand", "RotateGuidesCommand", "AddDimensionCommand", "AddTextLabelCommand"])
}

/// One row the diff writes: its index and its 80 bytes.
struct Written {
    ids: Vec<i64>,
    rows: Vec<u8>,
}
impl Written {
    fn push(&mut self, row: usize, value: &[u8; INSTANCE_BYTES]) {
        self.ids.push(row as i64);
        self.rows.extend_from_slice(value);
    }
}

fn world_bytes(py: Python<'_>, matrix: &Obj<'_>) -> PyResult<[u8; 64]> {
    let mut out = [0u8; 64];
    if matrix.is_none() {
        for k in 0..4 {
            out[k * 20..k * 20 + 4].copy_from_slice(&1f32.to_le_bytes());
        }
        return Ok(out);
    }
    let np = py.import("numpy")?;
    let values: Vec<f32> = np.call_method1("asarray", (matrix, np.getattr("float32")?))?.call_method1("reshape", (-1,))?.extract()?;
    if values.len() != 16 {
        return Err(PyValueError::new_err("a placement matrix is 4×4"));
    }
    for (k, value) in values.iter().enumerate() {
        out[k * 4..k * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

#[pyclass(module = "ingetrazo_core", dict, weakref)]
pub struct InstanceUpdates;

#[pymethods]
impl InstanceUpdates {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }

    fn __init__<'py>(slf: &Bound<'py, Self>, scene: &Obj<'py>, packed: &Obj<'py>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let np = py.import("numpy")?;
        s.setattr("scene", scene)?;
        s.setattr("packed", packed)?;
        // id(mesh) -> {code: (baked paint, billboard)}: every resident
        // prototype a mesh has. One mesh may be drawn plain, under an
        // inherited texture/opacity/tag effect and as a face-me sprite.
        let variants = PyDict::new(py);
        let paints = match packed.getattr("prototype_paints")? {
            paints if paints.is_truthy()? => paints.cast_into::<PyDict>()?,
            _ => PyDict::new(py),
        };
        let lookup = packed.getattr("prototype_lookup")?;
        if lookup.is_truthy()? {
            for (key, code) in lookup.cast_into::<PyDict>()?.iter() {
                if code.extract::<i64>()? >= 0 {
                    let of_mesh = variants.call_method1("setdefault", (key.get_item(0)?, PyDict::new(py)))?;
                    of_mesh.set_item(&code, (paints.call_method1("get", (&code,))?, key.get_item(2)?))?;
                }
            }
        }
        s.setattr("variants", variants)?;
        // The rows' keys, prototypes, live keys and world boxes
        // (resident_rows.rs); also the view's uid_rows mapping.
        let instances = packed.getattr("instances")?;
        let rows = py.import("ingetrazo_render")?.getattr("ResidentRows")?.call1((
            py.get_type::<PyList>().call1((packed.getattr("uids")?,))?,
            bytes_view(py, &instances)?,
            bytes_view(py, &packed.getattr("bounds")?)?,
        ))?;
        s.setattr("resident", rows)?;
        s.setattr("changed_code_rows", PyDict::new(py))?;
        let meshes = PyDict::new(py);
        let known = packed.getattr("mesh_codes")?;
        if known.is_truthy()? {
            // What the pack met: each mesh once (empty containers' with -1,
            // a serial sentinel without a GPU row).
            for (key, entry) in known.cast_into::<PyDict>()?.iter() {
                let mesh = entry.get_item(0)?;
                meshes.set_item(key, (&mesh, mesh.getattr("_mut_serial")?, entry.get_item(1)?))?;
            }
        } else {
            let codes = instances.get_item("info")?.get_item((pyo3::types::PySlice::full(py), 0))?.call_method0("tolist")?;
            let loose = scene.getattr("loose_mesh")?;
            for (row, group) in packed.getattr("groups")?.try_iter()?.enumerate() {
                let group = group?;
                let mesh = if group.is_none() { loose.clone() } else { group.getattr("mesh")? };
                meshes.set_item(id(&mesh), (&mesh, mesh.getattr("_mut_serial")?, codes.get_item(row)?))?;
            }
            let mut every = vec![loose];
            for group in py.import("ingetrazo_core")?.call_method1("purge_iter_groups", (scene.getattr("groups")?,))?.try_iter()? {
                every.push(group?.getattr("mesh")?);
            }
            for mesh in every {
                meshes.call_method1("setdefault", (id(&mesh), (&mesh, mesh.getattr("_mut_serial")?, -1)))?;
            }
        }
        let revisions = PyDict::new(py);
        for (key, entry) in meshes.iter() {
            revisions.set_item(key, entry.get_item(0)?.getattr("revision")?)?;
        }
        s.setattr("meshes", meshes)?;
        s.setattr("mesh_revisions", revisions)?;
        s.setattr("changed", PyList::empty(py))?;
        s.setattr("removed", PyList::empty(py))?;
        s.setattr("version", resident_version(scene)?)?;
        // uid -> [root, ..., group]: groups of the open edit path that draw
        // another mesh than the one resident (see the module docstring).
        s.setattr("pending_swaps", PyDict::new(py))?;
        s.setattr("chains", PyDict::new(py))?;
        s.setattr("last_local", false)?;
        s.setattr("added", 0)?;
        for name in ["pending", "pending_meshes", "pending_materials", "pending_removed"] {
            s.setattr(name, PySet::empty(py)?)?;
        }
        s.setattr("journal_valid", true)?;
        s.setattr("unsubscribe", py.None())?;
        // The change feed's records (core/protocol) of the meshes pending a
        // rebake, per id(mesh), and after diff() those of the meshes it
        // rebakes: the element keys a partial rebake would start from.
        s.setattr("mesh_changes", PyDict::new(py))?;
        s.setattr("changed_records", PyDict::new(py))?;
        s.setattr("roots", py.get_type::<PySet>().call1((scene.getattr("groups")?,))?)?;
        let _ = np;
        Ok(())
    }

    /// key -> its rows (the view's mapping).
    #[getter]
    fn uid_rows<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        slf.as_any().getattr("resident")
    }

    /// Use History's existing journal callbacks to avoid scanning known edits.
    fn observe<'py>(slf: &Bound<'py, Self>, history: &Obj<'py>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        if !s.getattr("unsubscribe")?.is_none() || history.is_none() {
            return Ok(());
        }
        let weakref = py.import("weakref")?;
        let targets = weakref.getattr("WeakKeyDictionary")?.call0()?.unbind();
        // The listeners live as long as History does and are not seen by the
        // collector: they hold this tracker weakly, and it holds the History
        // it listens to, so a closed document is not kept through them.
        s.setattr("_history", history)?;
        let tracker = weakref.call_method1("ref", (s,))?.unbind();
        let watcher = tracker.clone_ref(py);
        let before = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
            let s = watcher.bind(args.py()).call0()?;
            if s.is_none() {
                return Ok(());
            }
            if !resident_version(&s.getattr("scene")?)?.eq(s.getattr("version")?)? {
                s.setattr("journal_valid", false)?;
            }
            Ok(())
        })?;
        let after = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
            let py = args.py();
            let s = &tracker.bind(py).call0()?;
            if s.is_none() {
                return Ok(());
            }
            let (command, phase, ran) = (args.get_item(0)?, args.get_item(1)?, args.get_item(2)?.is_truthy()?);
            let journal = s.getattr("_history")?;
            let feed = attr(&journal, "feed")?;
            // The feed published this step before any other listener ran.
            let step = if ran && !feed.is_none() {
                feed.call_method1("step", (journal.getattr("revision")?,))?
            } else {
                none(py)
            };
            if !step.is_none() {
                let meshes = dict(s, "meshes")?;
                let changes = dict(s, "mesh_changes")?;
                for change in step.getattr("meshes")?.try_iter()? {
                    let change = change?;
                    let mesh = change.getattr("mesh")?;
                    if !mesh.is_none() && meshes.contains(id(&mesh))? {
                        changes.call_method1("setdefault", (id(&mesh), PyList::empty(py)))?.call_method1("append", (change,))?;
                    }
                }
            }
            let valid = s.getattr("journal_valid")?.is_truthy()? && ran && visit(s, &command, &phase, &step, targets.bind(py))?;
            s.setattr("journal_valid", valid)?;
            s.setattr("version", resident_version(&s.getattr("scene")?)?)
        })?;
        let kw = PyDict::new(py);
        kw.set_item("before", before)?;
        kw.set_item("after", after)?;
        s.setattr("unsubscribe", history.call_method("add_listener", (), Some(&kw))?)
    }

    /// Record `[root, ..., group]` for every resident group on the open edit
    /// path, so leaving it (the definition back on the instance) can be a
    /// row swap even after the path is closed.
    fn remember_edit_path(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let chain = PyList::empty(py);
        let roots = set(s, "roots")?;
        let (resident, chains) = (s.getattr("resident")?, dict(s, "chains")?);
        for group in py.import("core.outliner")?.call_method1("open_path", (s.getattr("scene")?,))?.try_iter()? {
            let group = group?;
            if chain.is_empty() {
                if !roots.contains(&group)? {
                    return Ok(());
                }
            } else if !children_hold(&chain.get_item(chain.len() - 1)?, &group)? {
                return Ok(());
            }
            chain.append(&group)?;
            let key = row_key(s, &group)?;
            if !resident.call_method1("row", (&key,))?.is_none() {
                chains.set_item(key, py.get_type::<PyList>().call1((&chain,))?)?;
            }
        }
        Ok(())
    }

    /// The recorded path to the open group that now draws `mesh`, or None
    /// (not a context swap, or not resident).
    fn _stack_chain<'py>(slf: &Bound<'py, Self>, mesh: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        s.call_method0("remember_edit_path")?;
        let chains = dict(s, "chains")?;
        for group in py.import("core.outliner")?.call_method1("open_path", (s.getattr("scene")?,))?.try_iter()? {
            let group = group?;
            if group.getattr("mesh")?.is(mesh) {
                return Ok(match chains.get_item(row_key(s, &group)?)? {
                    Some(chain) if chain.get_item(chain.len()? - 1)?.is(&group) => chain,
                    _ => none(py),
                });
            }
        }
        Ok(none(py))
    }

    /// Rewrite `group`'s row from its current mesh and the edit path it was
    /// opened on. False when that path is unknown (full diff).
    fn queue_swap<'py>(slf: &Bound<'py, Self>, group: &Obj<'py>) -> PyResult<bool> {
        let s = slf.as_any();
        let key = row_key(s, group)?;
        match dict(s, "chains")?.get_item(&key)? {
            Some(chain) if chain.get_item(chain.len()? - 1)?.is(group) => {
                dict(s, "pending_swaps")?.set_item(key, chain)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// `(row, group, matrix, paint)` per pending swap, or None when one no
    /// longer stands (the path was restructured meanwhile).
    fn _resolve_swaps<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let np = py.import("numpy")?;
        let packed = s.getattr("packed")?;
        let (resident, roots, variants) = (s.getattr("resident")?, set(s, "roots")?, dict(s, "variants")?);
        let known = packed.getattr("groups")?.len()?;
        let info = packed.getattr("instances")?.get_item("info")?;
        let out = PyList::empty(py);
        for (uid, chain) in dict(s, "pending_swaps")?.iter() {
            let row = resident.call_method1("row", (uid,))?;
            if row.is_none() || !roots.contains(chain.get_item(0)?)? || row.extract::<usize>()? >= known {
                return Ok(none(py));
            }
            let mut matrix = none(py);
            let mut parent = none(py);
            for group in chain.try_iter()? {
                let group = group?;
                if !parent.is_none() && !children_hold(&parent, &group)? {
                    return Ok(none(py));
                }
                let xform = attr(&group, "xform")?;
                matrix = if matrix.is_none() { xform } else if xform.is_none() { matrix } else { matrix.mul(xform)? };
                parent = group;
            }
            let code = info.get_item((&row, 0))?.call_method0("__int__")?;
            let mut variant = None;
            for of_mesh in variants.values().iter() {
                if of_mesh.contains(&code)? {
                    variant = Some(of_mesh.get_item(&code)?);
                    break;
                }
            }
            let Some(variant) = variant else { return Ok(none(py)) };
            let float32 = np.getattr("float32")?;
            let world = if matrix.is_none() {
                let kw = PyDict::new(py);
                kw.set_item("dtype", float32)?;
                np.call_method("eye", (4,), Some(&kw))?
            } else {
                np.call_method1("asarray", (matrix.call_method0("data")?, float32))?.call_method1("reshape", (4, 4))?
            };
            out.append((row, chain.get_item(chain.len()? - 1)?, world, variant.get_item(0)?))?;
        }
        Ok(out.into_any())
    }

    fn _link_swaps<'py>(slf: &Bound<'py, Self>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let scene = s.getattr("scene")?;
        let change = attr(&scene, "_link_mesh_changes")?;
        let busy = |name: &str| -> PyResult<bool> { s.getattr(name)?.is_truthy() };
        if !change.is_truthy()? || change.get_item(2)?.is_none() || !busy("journal_valid")?
            || busy("pending")? || busy("pending_meshes")? || busy("pending_materials")? || busy("pending_removed")?
            || busy("pending_swaps")? || scene.getattr("_edit_stack")?.is_truthy()?
        {
            return Ok(none(py));
        }
        let expected = (s.getattr("version")?.add(scene.getattr("_placement_context_bumps")?)?, scene.getattr("content_version")?);
        if !change.get_item(pyo3::types::PySlice::new(py, 0, 2, 1))?.eq(expected)? {
            return Ok(none(py));
        }
        let packed = s.getattr("packed")?;
        let (resident, variants) = (s.getattr("resident")?, dict(s, "variants")?);
        let instances = packed.getattr("instances")?;
        let swaps = PyList::empty(py);
        for entry in change.get_item(2)?.try_iter()? {
            let entry = entry?;
            let (group, old_mesh) = (entry.get_item(0)?, entry.get_item(1)?);
            let key = row_key(s, &group)?;
            let row = resident.call_method1("row", (&key,))?;
            if row.is_none() || !resident.call_method1("is_active", (&key,))?.is_truthy()? || !has_vertices(&group.getattr("mesh")?)? {
                return Ok(none(py));
            }
            let code = instances.get_item("info")?.get_item((&row, 0))?.call_method0("__int__")?;
            let variant = match variants.get_item(&old_mesh)? {
                Some(of_mesh) => of_mesh.call_method1("get", (&code,))?,
                None => none(py),
            };
            if variant.is_none() {
                return Ok(none(py));
            }
            swaps.append((&row, group, instances.get_item("world")?.get_item(&row)?, variant.get_item(0)?))?;
        }
        Ok(swaps.into_any())
    }

    /// Allocate a prototype code for a variant never drawn before.
    fn _new_prototype<'py>(slf: &Bound<'py, Self>, mesh: &Obj<'py>, key: &Obj<'py>, paint: &Obj<'py>, mode: &Obj<'py>, old: &Obj<'py>, fresh: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let packed = s.getattr("packed")?;
        let code = packed.getattr("bounds")?.len()?;
        let mut old = old.clone();
        if old.is_none() || old.get_item(2)?.extract::<i64>()? < 0 {
            old = (mesh, -1, code).into_bound_py_any(py)?;
            dict(s, "meshes")?.set_item(id(mesh), &old)?;
        }
        packed.getattr("prototype_lookup")?.set_item(key, code)?;
        dict(s, "variants")?.call_method1("setdefault", (id(mesh), PyDict::new(py)))?.set_item(code, (paint, mode))?;
        fresh.set_item(code, mesh)?;
        if packed.call_method0("append_prototype")?.extract::<usize>()? != code {
            return Err(PyValueError::new_err("the prototype tables are out of step"));
        }
        (code, old).into_bound_py_any(py)
    }

    /// Most prototypes one edit rebakes in place; past it, a full pack.
    fn rebake_limit(slf: &Bound<'_, Self>) -> PyResult<usize> {
        Ok((slf.as_any().getattr("packed")?.getattr("commands")?.len()? / 8).max(256))
    }

    /// Return changed resident rows, retaining prototype and placement IDs.
    fn diff<'py>(slf: &Bound<'py, Self>, scene: &Obj<'py>) -> Reply<'py> {
        let py = slf.py();
        let s = slf.as_any();
        let nothing = || Ok(none(py));
        let packed = s.getattr("packed")?;
        if !packed.getattr("model_translations")?.is_none() || !scene.is(&s.getattr("scene")?) {
            return nothing();
        }
        let meshes = dict(s, "meshes")?;
        let (pending_meshes, pending_materials) = (set(s, "pending_meshes")?, set(s, "pending_materials")?);
        let mut local = !s.getattr("unsubscribe")?.is_none()
            && s.getattr("journal_valid")?.is_truthy()?
            && resident_version(scene)?.eq(s.getattr("version")?)?;
        let mut swaps = if local { s.call_method0("_resolve_swaps")? } else { s.call_method0("_link_swaps")? };
        local = !swaps.is_none();
        if local {
            for key in pending_meshes.iter() {
                if !has_vertices(&meshes.get_item(&key)?.ok_or_else(|| PyValueError::new_err("an unknown mesh is pending"))?.get_item(0)?)? {
                    local = false;
                    break;
                }
            }
        }
        if !local {
            swaps = none(py);
        }
        dict(s, "pending_swaps")?.clear();
        s.setattr("last_local", local)?;
        let blank = || (py.None(), py.None(), py.None(), 0, py.None());
        let placements = PyList::empty(py);
        if !local {
            placements.append(blank())?;
        }
        let state = core(py)?.getattr("PlacementState")?.call1((scene,))?;
        let loose_mesh = scene.getattr("loose_mesh")?;
        let serial = |mesh: &Obj<'py>| mesh.getattr("_mut_serial");
        if local {
            let stale = match meshes.get_item(id(&loose_mesh))? {
                Some(loose) => !loose.get_item(1)?.eq(serial(&loose_mesh)?)?,
                None => true,
            };
            if stale {
                placements.append(blank())?;
            }
        }
        let interaction = py.import("views.document_interaction")?;
        let kw = PyDict::new(py);
        kw.set_item("scene", scene)?;
        let vp = py.import("types")?.getattr("SimpleNamespace")?.call((), Some(&kw))?;
        let roots = if local { s.getattr("pending")? } else { scene.getattr("groups")? };
        placements.call_method1("extend", (core(py)?.getattr("placement_records")?.call1((scene, roots, &vp, &state))?,))?;
        // A local diff adds to the live keys; a full one starts from none.
        let resident = s.getattr("resident")?;
        resident.call_method1("begin", (local,))?;
        let changed = PyDict::new(py);
        if local {
            for key in pending_meshes.iter() {
                let entry = meshes.get_item(&key)?.expect("checked above");
                if !serial(&entry.get_item(0)?)?.eq(entry.get_item(1)?)? {
                    changed.set_item(key, entry)?;
                }
            }
        }
        for key in pending_materials.iter() {
            changed.set_item(&key, meshes.get_item(&key)?.ok_or_else(|| PyValueError::new_err("an unknown mesh is pending"))?)?;
        }
        if !local {
            // Attribute-only edits (paint, hidden faces) leave the geometry
            // serial alone but move the revision: rebake those meshes too.
            let revisions = dict(s, "mesh_revisions")?;
            for (key, entry) in meshes.iter() {
                if entry.get_item(2)?.extract::<i64>()? >= 0
                    && !entry.get_item(0)?.getattr("revision")?.eq(revisions.call_method1("get", (&key,))?)?
                {
                    changed.set_item(key, entry)?;
                }
            }
        }
        pending_materials.clear();
        pending_meshes.clear();
        let additions = PyList::empty(py);
        let new_keys = PySet::empty(py)?;
        let fresh = PyDict::new(py); // code -> mesh: variants that exist from this edit on
        let resolved = PyDict::new(py);
        let builtins = py.import("builtins")?;
        let lookup = packed.getattr("prototype_lookup")?;
        let occurrence_keys = packed.getattr("occurrence_keys")?.cast_into::<PyDict>()?;
        let groups = packed.getattr("groups")?;
        let mut current = raw(py, &packed.getattr("instances")?)?;
        let resident_rows = current.len() / INSTANCE_BYTES;
        let known_groups = groups.len()?;
        let mut written = Written { ids: Vec::new(), rows: Vec::new() };
        let paint_of = |group: &Obj<'py>| -> Reply<'py> {
            core(py)?.getattr("baked_paint")?.call1((interaction.call_method1("_paint_of", (&vp, group))?,))
        };
        let prototype = |mesh: &Obj<'py>, paint: &Obj<'py>, mode: &Obj<'py>, changed: &Bound<'py, PyDict>| -> PyResult<(u32, Obj<'py>)> {
            let key = (id(mesh), builtins.call_method1("repr", (paint,))?, mode).into_bound_py_any(py)?;
            let mut old = match meshes.get_item(id(mesh))? {
                Some(old) => old,
                None => none(py),
            };
            let mut code = lookup.call_method1("get", (&key,))?;
            if code.is_none() || code.extract::<i64>()? < 0 {
                // A new mesh, or a known mesh under a paint or billboard
                // mode it was never drawn with: a prototype of its own.
                let made = s.call_method1("_new_prototype", (mesh, &key, paint, mode, &old, &fresh))?;
                code = made.get_item(0)?;
                old = made.get_item(1)?;
            }
            if !old.get_item(1)?.eq(serial(mesh)?)? {
                changed.set_item(id(mesh), &old)?;
            }
            Ok((code.extract()?, old))
        };
        for placement in placements.iter() {
            let (group, matrix, proxy, flags, columns) = (placement.get_item(0)?, placement.get_item(1)?, placement.get_item(2)?, placement.get_item(3)?, placement.get_item(4)?);
            let (mesh, mode) = if !columns.is_none() {
                (columns.get_item(0)?, columns.get_item(1)?)
            } else if group.is_none() {
                (loose_mesh.clone(), false.into_bound_py_any(py)?)
            } else {
                (group.getattr("mesh")?, match group.getattr_opt("billboard")? {
                    Some(mode) => mode,
                    None => false.into_bound_py_any(py)?,
                })
            };
            let got = match resolved.get_item(id(&proxy))? {
                Some(got) => got,
                None => {
                    let got = (paint_of(&proxy)?, state.call1((&proxy,))?.get_item(1)?).into_bound_py_any(py)?;
                    resolved.set_item(id(&proxy), &got)?;
                    got
                }
            };
            let paint = got.get_item(0)?;
            let uid = packed.call_method1("row_key", (&group,))?;
            let old = meshes.get_item(id(&mesh))?;
            let moved = match &old {
                Some(old) => !old.get_item(1)?.eq(serial(&mesh)?)?,
                None => true,
            };
            if let Some(old) = &old {
                if old.get_item(2)?.extract::<i64>()? < 0 && !moved {
                    continue; // empty when packed, under any paint: no row
                }
            }
            if moved && !has_vertices(&mesh)? {
                if old.is_none() {
                    meshes.set_item(id(&mesh), (&mesh, serial(&mesh)?, -1))?;
                }
                continue;
            }
            let (code, _) = prototype(&mesh, &paint, &mode, &changed)?;
            let mut row = resident.call_method1("row", (&uid,))?;
            if occurrence_keys.contains(&group)? {
                let path = py.import("core.link_structure")?.call_method1("row_id", (scene.call_method1("occurrence_path", (&group,))?,))?;
                if !path.eq(&uid)? {
                    return nothing();
                }
            }
            if !row.is_none() && resident.call_method1("is_active", (&uid,))?.is_truthy()? {
                let previous = groups.get_item(&row)?;
                if !previous.is(&group) && (previous.is_none() || !scene.call_method1("occurrence_path", (&previous,))?.is_none()) {
                    return nothing();
                }
            }
            if row.is_none() {
                // A new collision changes how the whole set of equal UIDs is
                // addressed. A new pack assigns qualified keys to all of them.
                let mut collides = new_keys.contains(&uid)?;
                for other in occurrence_keys.keys().iter() {
                    collides |= other.getattr("uid")?.eq(&uid)?;
                }
                if collides {
                    return nothing();
                }
                new_keys.add(&uid)?;
                row = (resident_rows + additions.len()).into_bound_py_any(py)?;
                additions.append((&uid, &group, &row))?;
            }
            let at: usize = row.extract()?;
            if at < known_groups {
                groups.set_item(at, &group)?;
            }
            resident.call_method1("see", (&uid,))?;
            let mut value = [0u8; INSTANCE_BYTES];
            value[..64].copy_from_slice(&world_bytes(py, &matrix)?);
            let sprite = if mode.is_truthy()? { 8 } else { 0 };
            let mut info = [code, got.get_item(1)?.extract::<u32>()?, flags.extract::<u32>()? | sprite, 0];
            let existing = (at < resident_rows).then(|| &current[at * INSTANCE_BYTES..(at + 1) * INSTANCE_BYTES]);
            if let Some(existing) = existing {
                info[3] = u32_at(existing, 76);
                info[2] |= u32_at(existing, 72) & 192;
            }
            for (k, word) in info.iter().enumerate() {
                value[64 + k * 4..68 + k * 4].copy_from_slice(&word.to_le_bytes());
            }
            if existing.is_none_or(|existing| existing != value) {
                written.push(at, &value);
            }
        }
        if !swaps.is_none() {
            for swap in swaps.try_iter()? {
                let swap = swap?;
                // Same group, same paint and flags: its prototype and matrix.
                let (row, group, world, paint): (usize, Obj<'py>, Obj<'py>, Obj<'py>) = (swap.get_item(0)?.extract()?, swap.get_item(1)?, swap.get_item(2)?, swap.get_item(3)?);
                let mesh = group.getattr("mesh")?;
                let mode = match group.getattr_opt("billboard")? {
                    Some(mode) => mode,
                    None => false.into_bound_py_any(py)?,
                };
                let (code, _) = prototype(&mesh, &paint, &mode, &changed)?;
                groups.set_item(row, &group)?;
                let existing = current.get(row * INSTANCE_BYTES..(row + 1) * INSTANCE_BYTES).ok_or_else(|| PyValueError::new_err("a swapped row is resident"))?;
                let mut value = [0u8; INSTANCE_BYTES];
                value.copy_from_slice(existing);
                value[..64].copy_from_slice(&world_bytes(py, &world)?);
                value[64..68].copy_from_slice(&code.to_le_bytes());
                if existing != value {
                    written.push(row, &value);
                }
            }
        }
        current.clear();
        let removed = PyList::empty(py);
        let pending_removed = set(s, "pending_removed")?;
        if local {
            let walk = py.import("ingetrazo_core")?.getattr("purge_iter_groups")?;
            for root in pending_removed.iter() {
                for group in walk.call1(((root,),))?.try_iter()? {
                    let key = packed.call_method1("row_key", (group?,))?;
                    let row = resident.call_method1("row", (&key,))?;
                    if !row.is_none() {
                        resident.call_method1("unsee", (&key,))?;
                        removed.append(row)?;
                    }
                }
            }
        }
        for key in occurrence_keys.values().iter() {
            if !resident.call_method1("has_seen", (&key,))?.is_truthy()?
                && (resident.call_method1("is_active", (&key,))?.is_truthy()? || removed.contains(resident.call_method1("row", (&key,))?)?)
            {
                return nothing();
            }
        }
        set(s, "pending")?.clear();
        pending_removed.clear();
        s.setattr("version", resident_version(scene)?)?;
        s.setattr("journal_valid", true)?;
        if !local {
            s.setattr("roots", py.get_type::<PySet>().call1((scene.getattr("groups")?,))?)?;
        }
        // Every variant of a changed mesh, and each new variant, rebakes.
        let bake = py.get_type::<PyDict>().call1((&fresh,))?.cast_into::<PyDict>()?;
        let variants = dict(s, "variants")?;
        for (key, entry) in changed.iter() {
            if let Some(of_mesh) = variants.get_item(&key)? {
                for code in of_mesh.try_iter()? {
                    bake.set_item(code?, entry.get_item(0)?)?;
                }
            }
        }
        // Incremental placement writes use unshifted matrices. A new remote
        // placement or remote prototype needs bounds and a new full pack.
        for row in written.rows.chunks_exact(INSTANCE_BYTES) {
            let far = (0..3).any(|axis| f32::from_le_bytes(row[48 + axis * 4..52 + axis * 4].try_into().expect("four bytes")).abs() > REBASE_DISTANCE);
            if far {
                return nothing();
            }
        }
        let rebaked = PyDict::new(py);
        for mesh in bake.values().iter() {
            rebaked.set_item(id(&mesh), mesh)?;
        }
        let np = py.import("numpy")?;
        for mesh in rebaked.values().iter() {
            let store = mesh.getattr("__dict__")?.call_method1("get", ("_store",))?;
            if store.is_none() {
                continue;
            }
            let positions = store.call_method0("positions")?;
            if positions.len()? == 0 {
                continue;
            }
            let axis0 = PyDict::new(py);
            axis0.set_item("axis", 0)?;
            let low: [f64; 3] = positions.call_method("min", (), Some(&axis0))?.extract()?;
            let high: [f64; 3] = positions.call_method("max", (), Some(&axis0))?.extract()?;
            let frame: [f64; 3] = store.getattr("frame_origin")?.extract()?;
            if (0..3).any(|k| ((low[k] + high[k]) * 0.5 + frame[k]).abs() > f64::from(REBASE_DISTANCE)) {
                return nothing();
            }
        }
        let _ = np;
        let records = dict(s, "mesh_changes")?;
        s.setattr("mesh_changes", PyDict::new(py))?;
        let changed_records = PyDict::new(py);
        for key in rebaked.keys().iter() {
            if let Some(record) = records.get_item(&key)? {
                changed_records.set_item(key, record)?;
            }
        }
        s.setattr("changed_records", changed_records)?;
        if bake.len() > s.call_method0("rebake_limit")?.extract::<usize>()? {
            // Revisions and serials also move without an edit (a lazy mesh
            // built, commands that flag every group mesh), and a batch the
            // size of the scene costs more than one full pack. It also broke
            // the resident GPU scene (4 432 prototypes replaced: a blank
            // frame, no error), so the full pack it is.
            return nothing();
        }
        let to_bake = PyList::empty(py);
        for (code, mesh) in bake.iter() {
            let entry = meshes.get_item(id(&mesh))?.ok_or_else(|| PyValueError::new_err("a rebaked mesh is known"))?;
            to_bake.append((&mesh, entry.get_item(1)?, code))?;
        }
        s.setattr("changed", to_bake)?;
        let revisions = dict(s, "mesh_revisions")?;
        for mesh in rebaked.values().iter() {
            revisions.set_item(id(&mesh), mesh.getattr("revision")?)?;
        }
        let gone = resident.call_method0("finish")?;
        s.setattr("removed", if local { removed.into_any() } else { gone })?;
        s.setattr("added", additions.len())?;
        if !additions.is_empty() {
            packed.call_method1("append_rows", (additions.len(),))?;
            let uids = packed.getattr("uids")?;
            for addition in additions.iter() {
                let (uid, group, row) = (addition.get_item(0)?, addition.get_item(1)?, addition.get_item(2)?);
                resident.call_method1("add", (&uid, row))?;
                uids.call_method1("append", (uid,))?;
                groups.call_method1("append", (group,))?;
            }
        }
        let rows = PyArray1::from_vec(py, written.rows).call_method1("view", (dtype(py, "INSTANCE")?,))?;
        (PyArray1::from_vec(py, written.ids), rows).into_bound_py_any(py)
    }

    /// Give the resident vertices and indices spare room, copied on
    /// `executor` (`PackedScene.reserve`).
    fn reserve<'py>(slf: &Bound<'py, Self>, executor: &Obj<'py>) -> PyResult<()> {
        slf.as_any().getattr("packed")?.call_method1("reserve", (executor,))?;
        Ok(())
    }
    #[pyo3(signature = (wait = false))]
    fn adopt_reserved<'py>(slf: &Bound<'py, Self>, wait: bool) -> Reply<'py> {
        slf.as_any().getattr("packed")?.call_method1("adopt_reserved", (wait,))
    }

    #[pyo3(signature = (ids, rows, renderer = None))]
    fn apply<'py>(slf: &Bound<'py, Self>, ids: &Obj<'py>, rows: &Obj<'py>, renderer: Option<Obj<'py>>) -> PyResult<()> {
        let py = slf.py();
        let s = slf.as_any();
        let np = py.import("numpy")?;
        let renderer = renderer.filter(|r| !r.is_none());
        let packed = s.getattr("packed")?;
        let resident = s.getattr("resident")?;
        let changed = s.getattr("changed")?;
        let any_changed = changed.is_truthy()?;
        let uint8 = np.getattr("uint8")?;
        let flat = |array: &Obj<'py>| -> Reply<'py> { array.call_method1("view", (&uint8,))?.call_method1("reshape", (-1,)) };
        if any_changed {
            // A single bake for the changed set, including shared meshes.
            let group_type = py.import("core.group")?.getattr("Group")?;
            let variants = dict(s, "variants")?;
            let meshes = dict(s, "meshes")?;
            let prototypes = PyList::empty(py);
            let paints = PyDict::new(py);
            for entry in changed.try_iter()? {
                let entry = entry?;
                let (mesh, code) = (entry.get_item(0)?, entry.get_item(2)?);
                let variant = variants.get_item(id(&mesh))?.ok_or_else(|| PyValueError::new_err("a rebaked mesh has variants"))?.get_item(&code)?;
                let kw = PyDict::new(py);
                kw.set_item("name", "Resident prototype")?;
                let prototype = group_type.call((&mesh,), Some(&kw))?;
                prototype.setattr("billboard", variant.get_item(1)?)?;
                paints.set_item(id(&prototype), variant.get_item(0)?)?;
                prototypes.append(prototype)?;
            }
            let kw = PyDict::new(py);
            kw.set_item("groups", &prototypes)?;
            let stage = py.import("core.scene")?.getattr("Scene")?.call((), Some(&kw))?;
            let kw = PyDict::new(py);
            kw.set_item("baked_paints", &paints)?;
            let baked = core(py)?.getattr("pack_scene")?.call((stage,), Some(&kw))?;
            let baked_row = PyDict::new(py);
            for (row, group) in baked.getattr("groups")?.try_iter()?.enumerate() {
                baked_row.set_item(id(&group?), row)?;
            }
            let parity = py.import("views.wgpu_parity")?;
            if let Some(renderer) = &renderer {
                if renderer.hasattr("set_material_transparency")? && parity.call_method1("has_transparency", (&baked,))?.is_truthy()? {
                    renderer.call_method1("set_material_transparency", (true,))?;
                }
            }
            let exclusions = py.get_type::<PyDict>().call1((packed.getattr("profile_exclusions")?,))?.cast_into::<PyDict>()?;
            let before = py.get_type::<PyDict>().call1((&exclusions,))?.cast_into::<PyDict>()?;
            let paths = py.get_type::<PyList>().call1((packed.getattr("texture_paths")?,))?.cast_into::<PyList>()?;
            let baked_paths = baked.getattr("texture_paths")?;
            for path in baked_paths.try_iter()? {
                let path = path?;
                if !paths.contains(&path)? {
                    paths.append(path)?;
                }
            }
            let mut texture_map = vec![0u32];
            for path in baked_paths.try_iter()? {
                texture_map.push(paths.call_method1("index", (path?,))?.extract::<u32>()? + 1);
            }
            let all_paths = py.get_type::<PyTuple>().call1((&paths,))?;
            if !all_paths.eq(packed.getattr("texture_paths")?)? {
                packed.setattr("texture_paths", &all_paths)?;
                let textures = py.import("ingetrazo_render")?.call_method1("material_texture_array", (&paths, 8192, 256 * 1024 * 1024))?;
                packed.setattr("textures", &textures)?;
                if let Some(renderer) = &renderer {
                    renderer.getattr("set_textures")?.call1(py.get_type::<PyTuple>().call1((textures,))?.cast_into::<PyTuple>()?)?;
                }
            }
            let baked_exclusions = py.get_type::<PyDict>().call1((baked.getattr("profile_exclusions")?,))?.cast_into::<PyDict>()?;
            let baked_instances = raw(py, &baked.getattr("instances")?)?;
            let tables = ["commands", "edge_commands", "hidden_commands", "hidden_edge_commands"];
            let baked_tables: Vec<Vec<u8>> = tables.iter().map(|name| raw(py, &baked.getattr(*name)?)).collect::<PyResult<_>>()?;
            let baked_materials = raw(py, &baked.getattr("materials")?)?;
            let (baked_vertices, baked_indices, baked_bounds) = (baked.getattr("vertices")?, baked.getattr("indices")?, baked.getattr("bounds")?);
            for (i, entry) in changed.try_iter()?.enumerate() {
                let entry = entry?;
                let (mesh, code) = (entry.get_item(0)?, entry.get_item(2)?);
                let at: usize = code.extract()?;
                let row = baked_row.get_item(id(&prototypes.get_item(i)?))?;
                let prototype = match &row {
                    Some(row) => Some(u32_at(&baked_instances, row.extract::<usize>()? * INSTANCE_BYTES + 64) as usize),
                    None => None,
                };
                // count, first, base of the four draws; zeros when nothing bakes.
                let entry_of = |table: usize| -> [u32; 3] {
                    match prototype {
                        Some(p) => {
                            let o = p * COMMAND_BYTES;
                            [u32_at(&baked_tables[table], o), u32_at(&baked_tables[table], o + 8), u32_at(&baked_tables[table], o + 12)]
                        }
                        None => [0; 3],
                    }
                };
                let draws = [entry_of(0), entry_of(1), entry_of(2), entry_of(3)];
                let bounds = match prototype {
                    Some(p) => baked_bounds.get_item(p)?,
                    None => np.call_method1("zeros", ((2, 4), np.getattr("float32")?))?,
                };
                match prototype {
                    Some(p) => exclusions.set_item(&code, baked_exclusions.call_method1("get", (p, PyList::empty(py)))?)?,
                    None => exclusions.set_item(&code, PyList::empty(py))?,
                }
                let start = draws[0][2] as usize;
                let faces = draws[0][0] as usize;
                let count = faces + draws[1][0] as usize;
                // Hidden faces and edges follow the visible ones in the bake;
                // the renderer takes the visible range, the resident arrays
                // keep all four for Show hidden geometry. Billboards suppress
                // the visible edge draw, but its endpoint vertices still
                // precede the hidden ranges in the bake.
                let total = (draws[3][1] as usize + draws[3][0] as usize).saturating_sub(start);
                let slice = |from: usize, to: usize| pyo3::types::PySlice::new(py, from as isize, to as isize, 1);
                let vertices = baked_vertices.get_item(slice(start, start + total))?;
                let indices = baked_indices.get_item(slice(start, start + total))?;
                if let Some(renderer) = &renderer {
                    renderer.call_method1(
                        "replace_prototype",
                        (&code, flat(&vertices.get_item(slice(0, count))?)?, flat(&indices.get_item(slice(0, count))?)?, flat(&bounds)?, faces),
                    )?;
                }
                let mut material: Vec<u8> = Vec::new();
                let mut flags = 0u32;
                if let Some(p) = prototype {
                    let header = p * 16;
                    let (first, length) = (u32_at(&baked_materials, header + 4) as usize, u32_at(&baked_materials, header + 8) as usize);
                    flags = u32_at(&baked_materials, header + 12);
                    if length != 0 {
                        material = baked_materials.get(first * 16..first * 16 + length * 32).ok_or_else(|| PyValueError::new_err("material records out of range"))?.to_vec();
                        // Each record's two texture layers, in this pack's numbering.
                        for record in material.chunks_exact_mut(32) {
                            for word in [5usize, 6] {
                                let layer = u32_at(record, word * 4) as usize;
                                let mapped = *texture_map.get(layer).ok_or_else(|| PyValueError::new_err("texture layer out of range"))?;
                                record[word * 4..word * 4 + 4].copy_from_slice(&mapped.to_le_bytes());
                            }
                        }
                    }
                }
                let stored = PyArray1::from_vec(py, material.clone());
                packed.getattr("material_updates")?.set_item(&code, (flags, &stored))?;
                if let Some(renderer) = &renderer {
                    let live = if flags & 1 != 0 {
                        // The live API takes explicit corner records. A full
                        // upload retains the compact matching-back header.
                        let colours = raw(py, &vertices.get_item("color")?.get_item(slice(0, faces))?)?;
                        let mut records = vec![0u8; faces * 32];
                        for (record, colour) in records.chunks_exact_mut(32).zip(colours.chunks_exact(4)) {
                            record[16..20].copy_from_slice(colour);
                        }
                        PyArray1::from_vec(py, records)
                    } else {
                        stored
                    };
                    renderer.call_method1("update_prototype_materials", (&code, live))?;
                }
                let offset: usize = packed.call_method1("place", (&code, &vertices, &indices))?.extract()?;
                packed.getattr("bounds")?.set_item(&code, &bounds)?;
                for (table, draw) in tables.iter().zip(draws) {
                    let target = packed.getattr(*table)?.get_item(&code)?;
                    target.set_item("count", draw[0])?;
                    target.set_item("base", offset)?;
                    target.set_item("first", offset + draw[1] as usize - start)?;
                }
                meshes.set_item(id(&mesh), (&mesh, mesh.getattr("_mut_serial")?, if prototype.is_some() { at as i64 } else { -1 }))?;
            }
            let kept = PyList::empty(py);
            for (code, points) in exclusions.iter() {
                if points.is_truthy()? {
                    kept.append((code, points))?;
                }
            }
            packed.setattr("profile_exclusions", kept.to_tuple())?;
            if let Some(renderer) = renderer.as_ref().filter(|r| r.hasattr("set_profile_exclusions").unwrap_or(false)) {
                let own = |table: &Bound<'py, PyDict>, code: &Obj<'py>| -> Reply<'py> {
                    let points = table.call_method1("get", (code,))?;
                    Ok(if points.is_truthy()? { points } else { PyList::empty(py).into_any() })
                };
                if attr(renderer, "incremental_profile_exclusions")?.is_truthy()? {
                    // Only the rebaked prototypes: each came back on a fresh,
                    // unexcluded range; the others keep theirs natively.
                    let pairs = PyList::empty(py);
                    for entry in changed.try_iter()? {
                        let code = entry?.get_item(2)?;
                        let points = exclusions.call_method1("get", (&code,))?;
                        if points.is_truthy()? {
                            pairs.append((code, points))?;
                        }
                    }
                    if !pairs.is_empty() {
                        renderer.call_method1("set_profile_exclusions", (pairs,))?;
                    }
                } else {
                    // An older module rebuilds every prototype from the whole
                    // list: send it only when an edited prototype's own
                    // exclusions moved.
                    let mut moved = false;
                    for entry in changed.try_iter()? {
                        let code = entry?.get_item(2)?;
                        moved |= !own(&before, &code)?.eq(own(&exclusions, &code)?)?;
                    }
                    if moved {
                        renderer.call_method1("set_profile_exclusions", (packed.getattr("profile_exclusions")?,))?;
                    }
                }
            }
        }
        let instances = packed.getattr("instances")?;
        let mut membership_changed = instances.len()? != resident.getattr("count")?.extract::<usize>()?;
        let written = ids.len()? != 0;
        if written {
            let codes = rows.get_item("info")?.get_item((pyo3::types::PySlice::full(py), 0))?.call_method0("tolist")?;
            membership_changed |= resident.call_method1("set_codes", (ids.call_method0("tolist")?, codes))?.is_truthy()?;
            if let Some(renderer) = &renderer {
                renderer.call_method1("update_instances", (ids.call_method0("tolist")?, flat(rows)?))?;
            }
            instances.set_item(ids, rows)?;
        }
        let removed = s.getattr("removed")?;
        if removed.is_truthy()? {
            if let Some(renderer) = &renderer {
                renderer.call_method1("remove_instances", (&removed,))?;
            }
            instances.get_item("info")?.set_item((&removed, 2), 1)?;
        }
        if !written && !any_changed && !removed.is_truthy()? {
            return Ok(());
        }
        // A rebake keeps membership but re-anchors its prototypes' ranges:
        // recompute the starts then too.
        let commands = packed.getattr("commands")?;
        if membership_changed || any_changed {
            let starts = resident.call_method1("starts", (commands.len()?,))?;
            for table in ["commands", "edge_commands", "hidden_commands", "hidden_edge_commands"] {
                packed.getattr(table)?.set_item("start", &starts)?;
            }
        }
        let code_rows = PyDict::new(py);
        for entry in changed.try_iter()? {
            let code = entry?.get_item(2)?;
            code_rows.set_item(&code, resident.call_method1("members", (&code,))?)?;
        }
        s.setattr("changed_code_rows", &code_rows)?;
        let parts = PyList::new(py, [ids.clone(), np.call_method1("asarray", (&removed, np.getattr("int64")?))?])?;
        for members in code_rows.values().iter() {
            parts.append(members)?;
        }
        let touched = np.call_method1("unique", (np.call_method1("concatenate", (parts,))?,))?;
        if touched.len()? != 0 {
            let float32 = np.getattr("float32")?;
            let bounds = np.call_method1("asarray", (packed.getattr("world_bounds")?, &float32))?;
            let got = resident.call_method1(
                "refresh_boxes",
                (
                    touched.call_method0("tolist")?,
                    flat(&packed.getattr("instances")?)?,
                    flat(&np.call_method1("ascontiguousarray", (packed.getattr("bounds")?,))?)?,
                    bounds.get_item(0)?.call_method0("tolist")?,
                    bounds.get_item(1)?.call_method0("tolist")?,
                ),
            )?;
            let world_bounds = if got.is_none() { np.call_method1("zeros", ((2, 3),))? } else { np.call_method1("array", (got, float32))? };
            packed.setattr("world_bounds", world_bounds)?;
        }
        Ok(())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<InstanceUpdates>()
}
