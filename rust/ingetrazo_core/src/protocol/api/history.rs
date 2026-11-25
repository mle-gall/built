//! Public command classes and the host for the native document journal.
use super::*;
use pyo3::types::{PyInt, PySet, PyString};

fn zero() -> Py<PyAny> {
    Python::attach(|py| 0i32.into_pyobject(py).unwrap().into_any().unbind())
}
fn yes() -> Py<PyAny> {
    Python::attach(|py| {
        pyo3::types::PyBool::new(py, true)
            .to_owned()
            .into_any()
            .unbind()
    })
}
fn empty_name() -> Py<PyAny> {
    Python::attach(|py| PyString::new(py, "").into_any().unbind())
}
fn empty_tuple() -> Py<PyAny> {
    Python::attach(|py| PyTuple::empty(py).into_any().unbind())
}

#[pyfunction]
fn history_init(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let py = this.py();
    this.setattr("scene", scene)?;
    this.setattr("_listeners", list(py))?;
    this.setattr("_running", false)?;
    let journal = py
        .import("ingetrazo_core")?
        .call_method0("DocumentHistory")?;
    this.setattr("undo_stack", journal.getattr("undo")?)?;
    this.setattr("redo_stack", journal.getattr("redo")?)?;
    this.setattr("_native", journal)?;
    for key in ["last_error", "last_rejected", "last_txn"] {
        this.setattr(key, py.None())?;
    }
    this.setattr(
        "feed",
        py.import("core.protocol.feed")?
            .call_method1("ChangeFeed", (this,))?,
    )
}

#[pyfunction]
fn default_error_log(py: Python<'_>) -> PyResult<Obj<'_>> {
    Ok(py
        .import("core.paths")?
        .call_method0("user_log_dir")?
        .call_method1("__truediv__", ("ingetrazo-errors.log",))?
        .str()?
        .into_any())
}
#[pyfunction]
fn unsubscribe(history: &Obj<'_>, entry: &Obj<'_>) -> PyResult<()> {
    let listeners = history.getattr("_listeners")?;
    for (i, item) in listeners.try_iter()?.enumerate() {
        if item?.is(entry) {
            listeners.del_item(i)?;
            break;
        }
    }
    Ok(())
}
#[pyfunction(signature=(this, *, before=None, after=None))]
fn add_listener<'p>(
    this: &Obj<'p>,
    before: Option<&Obj<'p>>,
    after: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = this.py();
    let entry = (before, after).into_pyobject(py)?;
    this.getattr("_listeners")?
        .call_method1("append", (&entry,))?;
    py.import("functools")?
        .call_method1("partial", (wrap_pyfunction!(unsubscribe, py)?, this, entry))
}
#[pyfunction]
fn revision<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.getattr("_native")?.getattr("revision")
}
#[pyfunction(signature=(this, revision=zero()))]
fn changes_since<'p>(this: &Obj<'p>, revision: Py<PyAny>) -> PyResult<Obj<'p>> {
    let revision = revision.bind(this.py());
    if !revision.is_exact_instance_of::<PyInt>()
        || revision.lt(0)?
        || revision.gt(this.getattr("revision")?)?
    {
        return Err(PyValueError::new_err(
            "revision must be between zero and the current revision",
        ));
    }
    this.getattr("_native")?
        .call_method1("changes_since", (revision,))
}
#[pyfunction]
fn execute<'p>(this: &Obj<'p>, cmd: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.getattr("_native")?
        .call_method1("run", (this, cmd, "execute"))
}
fn replay<'p>(this: &Obj<'p>, stack: &str, phase: &str) -> PyResult<Obj<'p>> {
    let cmds = this.getattr(stack)?;
    if !cmds.is_truthy()? {
        return Ok(false.into_pyobject(this.py())?.to_owned().into_any());
    }
    this.getattr("_native")?
        .call_method1("run", (this, cmds.get_item(-1)?, phase))
}
#[pyfunction]
fn undo<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    replay(this, "undo_stack", "undo")
}
#[pyfunction]
fn redo<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    replay(this, "redo_stack", "redo")
}
#[pyfunction]
fn clear(this: &Obj<'_>) -> PyResult<()> {
    this.getattr("_native")?.call_method0("clear_stacks")?;
    Ok(())
}
#[pyfunction]
fn context_changed(this: &Obj<'_>, before: &Obj<'_>) -> PyResult<()> {
    if let Ok(t) = before.cast::<PyTuple>() {
        if t.len() != 2 {
            return Err(PyValueError::new_err("expected a version and serial"));
        }
        if !t.get_item(0)?.is_none() {
            this.getattr("feed")?.getattr("readers")?.call_method1(
                "context_changed",
                (
                    t.get_item(0)?,
                    t.get_item(1)?,
                    this.getattr("scene")?.getattr("content_version")?,
                ),
            )?;
        }
    }
    Ok(())
}
#[pyfunction]
fn command_state<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.py()
        .import("builtins")?
        .getattr("object")?
        .call_method1("__getstate__", (this,))
}
#[pyfunction]
fn direct_snapshot<'p>(this: &Obj<'p>, mesh: &Obj<'p>) -> PyResult<Obj<'p>> {
    if this.getattr("_journal_geometry")?.is_truthy()? {
        Ok(none(this.py()))
    } else {
        mesh.call_method0("capture_state")
    }
}
#[pyfunction]
fn bind_transaction(this: &Obj<'_>, txn: &Obj<'_>) -> PyResult<()> {
    this.setattr("_doc_txn", txn)
}
fn bump(scene: &Obj<'_>) -> PyResult<()> {
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}
#[pyfunction]
fn doc_after(_this: &Obj<'_>, scene: &Obj<'_>, phase: &Obj<'_>) -> PyResult<()> {
    let _ = phase;
    bump(scene)
}
#[pyfunction]
fn abstract_action(_this: &Obj<'_>, _scene: &Obj<'_>) {}
#[pyfunction]
fn meshes<'p>(_this: &Obj<'p>, scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    Ok((scene.getattr("mesh")?,)
        .into_pyobject(scene.py())?
        .into_any())
}
#[pyfunction]
fn face_meshes<'p>(this: &Obj<'p>, scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let _ = scene;
    let s = PySet::empty(this.py())?;
    for f in this.getattr("_faces")?.try_iter()? {
        s.add(f?.getattr_opt("_m")?)?
    }
    Ok(s.into_any())
}
#[pyfunction]
fn plugin_init(this: &Obj<'_>, key: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    let py = this.py();
    let json = py.import("json")?;
    this.setattr("key", key.str()?)?;
    this.setattr(
        "value",
        json.call_method1("loads", (json.call_method1("dumps", (value,))?,))?,
    )?;
    this.setattr("_had", false)?;
    this.setattr("_before", py.None())
}
fn snapshots(this: &Obj<'_>) -> PyResult<()> {
    this.setattr("before", this.py().None())?;
    this.setattr("after", this.py().None())
}
#[pyfunction(signature=(this,mutate,mesh=None))]
fn mutation_init(this: &Obj<'_>, mutate: &Obj<'_>, mesh: Option<&Obj<'_>>) -> PyResult<()> {
    this.setattr("mutate", mutate)?;
    this.setattr("_mesh", mesh)?;
    snapshots(this)
}
#[pyfunction(signature=(cls,before,after,mesh=None))]
fn from_states<'p>(
    cls: &Obj<'p>,
    before: &Obj<'p>,
    after: &Obj<'p>,
    mesh: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let kwargs = PyDict::new(cls.py());
    kwargs.set_item("mesh", mesh)?;
    let cmd = cls.call((cls.py().None(),), Some(&kwargs))?;
    cmd.setattr("before", before)?;
    cmd.setattr("after", after)?;
    Ok(cmd)
}
#[pyfunction]
fn import_init(this: &Obj<'_>, mutate: &Obj<'_>) -> PyResult<()> {
    this.setattr("mutate", mutate)?;
    snapshots(this)?;
    for t in this.getattr("_TRACKED")?.try_iter()? {
        let t = t?;
        for i in [1, 2] {
            this.setattr(t.get_item(i)?.cast::<PyString>()?, list(this.py()))?
        }
    }
    Ok(())
}
fn collect<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    v.py().get_type::<PyList>().call1((v,))
}
#[pyfunction(signature=(this,inner,*,heal=yes()))]
fn compound_snapshot_init(this: &Obj<'_>, inner: &Obj<'_>, heal: Py<PyAny>) -> PyResult<()> {
    this.setattr("inner", collect(inner)?)?;
    this.setattr("_heal", heal)?;
    snapshots(this)
}
#[pyfunction]
fn stitch_init(this: &Obj<'_>, inner: &Obj<'_>, stitch_seed: &Obj<'_>) -> PyResult<()> {
    this.setattr("inner", collect(inner)?)?;
    let py = this.py();
    let vec = py.import("ingetrazo_core")?.getattr("Vec3")?;
    let seed = PyList::empty(py);
    for p in stitch_seed.try_iter()? {
        seed.append(vec.call1((p?,))?)?
    }
    this.setattr("seed", seed)?;
    this.setattr("snapshot", py.None())
}
#[pyfunction]
fn compound_init(this: &Obj<'_>, commands: &Obj<'_>) -> PyResult<()> {
    this.setattr("commands", collect(commands)?)
}
#[pyfunction]
fn group_start<'p>(history: &Obj<'p>, session: &Obj<'p>) -> PyResult<Obj<'p>> {
    history
        .getattr("_native")?
        .call_method1("group_start", (session,))
}
#[pyfunction]
fn fold<'p>(history: &Obj<'p>, session: &Obj<'p>) -> PyResult<Obj<'p>> {
    history.getattr("_native")?.call_method1(
        "fold_occurrence",
        (
            session,
            history
                .py()
                .import("ingetrazo_core.history")?
                .getattr("OccurrenceCommands")?,
        ),
    )
}
fn group(this: &Obj<'_>, scene: &Obj<'_>, undo: bool, after: bool) -> PyResult<()> {
    this.py()
        .import("ingetrazo_core")?
        .getattr("DocumentHistory")?
        .call_method1("replay_group", (this, scene, undo, after))?;
    Ok(())
}
#[pyfunction]
fn group_after(this: &Obj<'_>, scene: &Obj<'_>, phase: &Obj<'_>) -> PyResult<()> {
    group(this, scene, phase.eq("undo")?, true)
}
#[pyfunction]
fn group_do(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    group(this, scene, false, false)
}
#[pyfunction]
fn group_undo(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    group(this, scene, true, false)
}
#[pyfunction(signature=(this,targets,patch,*,normalize_values=yes()))]
fn record_init(
    this: &Obj<'_>,
    targets: &Obj<'_>,
    patch: &Obj<'_>,
    normalize_values: Py<PyAny>,
) -> PyResult<()> {
    this.setattr("normalize_values", normalize_values)?;
    this.setattr("targets", collect(targets)?)?;
    this.setattr("patch", copied(patch)?)?;
    previous(this)
}
fn previous(this: &Obj<'_>) -> PyResult<()> {
    this.setattr("_before", this.py().None())?;
    this.setattr("_after", this.py().None())
}
#[pyfunction(signature=(this,targets,ifc_class,name=empty_name()))]
fn tag_init(
    this: &Obj<'_>,
    targets: &Obj<'_>,
    ifc_class: &Obj<'_>,
    name: Py<PyAny>,
) -> PyResult<()> {
    let d = dict(this.py());
    d.set_item("class", ifc_class)?;
    d.set_item("name", name)?;
    record_init(this, targets, &d, yes())
}
#[pyfunction]
fn untag_init(this: &Obj<'_>, targets: &Obj<'_>) -> PyResult<()> {
    record_init(this, targets, &none(this.py()), yes())
}
#[pyfunction(signature=(this,value,*,refiled_uids=empty_tuple()))]
fn registry_init(this: &Obj<'_>, value: &Obj<'_>, refiled_uids: Py<PyAny>) -> PyResult<()> {
    let py = this.py();
    let frozen = py.import("builtins")?.getattr("frozenset")?;
    this.setattr("refiled_uids", frozen.call1((refiled_uids,))?)?;
    let errors = py
        .import("core.bim_registry")?
        .call_method1("validate", (value,))?;
    if errors.is_truthy()? {
        return Err(PyValueError::new_err(
            PyString::new(py, "; ")
                .call_method1("join", (errors,))?
                .extract::<String>()?,
        ));
    }
    plugin_init(this, PyString::new(py, "bim_registry").as_any(), value)
}
fn registry(this: &Obj<'_>, scene: &Obj<'_>, undo: bool) -> PyResult<()> {
    this.py()
        .import("ingetrazo_core")?
        .call_method1("registry_apply", (this, scene, undo))?;
    Ok(())
}
#[pyfunction]
fn registry_do(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    registry(this, scene, false)
}
#[pyfunction]
fn registry_undo(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    registry(this, scene, true)
}
#[pyfunction]
fn registry_doc(_this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<bool> {
    Ok(scene
        .py()
        .import("core.project")?
        .call_method1("path_for", (scene,))?
        .is_none())
}
#[pyfunction]
fn registry_after(_this: &Obj<'_>, scene: &Obj<'_>, phase: &Obj<'_>) -> PyResult<()> {
    let _ = phase;
    scene.getattr("mesh")?.call_method0("bump_revision")?;
    bump(scene)
}
#[pyfunction(signature=(this,faces,ifc_class=None,name=empty_name()))]
fn face_init(
    this: &Obj<'_>,
    faces: &Obj<'_>,
    ifc_class: Option<&Obj<'_>>,
    name: Py<PyAny>,
) -> PyResult<()> {
    this.setattr("faces", collect(faces)?)?;
    this.setattr("ifc_class", ifc_class)?;
    this.setattr("name", name)?;
    previous(this)
}
pub(crate) fn register(core: &Module<'_>) -> PyResult<()> {
    let py = core.py();
    let m = PyModule::new(py, "ingetrazo_core.history")?;
    let m = &m;
    let builtins = py.import("builtins")?;
    let abc = py.import("abc")?;
    exports(
        m,
        core.as_any(),
        "placement_is_identity placement_rotation_deg",
    )?;
    exports(m, core.as_any(), "NativeCommand")?;
    exports(
        m,
        core.as_any(),
        "AddEdgeCommand AddFaceCommand DeleteEdgesCommand DeleteFaceCommand EraseSelectionCommand",
    )?;
    exports(
        m,
        core.as_any(),
        "HealOverlapsCommand RebuildPlanarFacesCommand RebuildPlaneFacesCommand",
    )?;
    exports(m, core.as_any(), "ExplodeGroupCommand UnlinkCommand")?;
    exports(m,core.as_any(),"AutoTagDrawnFacesCommand CoplanarMergeCommand ExplodeCurvesCommand FlipFacesCommand PruneOrphanEdgesCommand SimplifyMeshCommand StitchSolidCommand TagCurveCommand")?;
    exports(
        m,
        core.as_any(),
        "DeleteMaterialCommand EditTexturedMaterialCommand RestampMaterialCommand",
    )?;
    exports(
        m,
        core.as_any(),
        "invert_scale_factor mirror_matrix rotation_matrix scale_matrix",
    )?;
    exports(
        m,
        core.as_any(),
        "FlipVerticesCommand MoveVerticesCommand RotateVerticesCommand ScaleVerticesCommand",
    )?;
    exports(
        m,
        core.as_any(),
        "AddGeoPointsCommand SetNorthCommand StraightenModelCommand",
    )?;
    exports(
        m,
        core.as_any(),
        "ChangeAxesCommand FlipGroupsCommand MoveGroupCommand RotateGroupCommand ScaleGroupCommand",
    )?;
    exports(
        m,
        core.as_any(),
        "DeleteLayerCommand EnsureLayersCommand PurgeUnusedCommand",
    )?;
    exports(
        m,
        core.as_any(),
        "AssignLayerCommand HideCommand HideEdgesCommand",
    )?;
    exports(m,core.as_any(),"SetFaceBackCommand SetFaceColorCommand SetFaceMaterialTagCommand SetFaceOpacityCommand SetFaceTextureCommand")?;
    exports(m,core.as_any(),"ReplaceComponentCommand KeepComponentCommand AddKeptComponentsCommand GroupToComponentCommand EditText3DCommand ReparentGroupsCommand MergeGroupsCommand MakeUniqueCommand InsertGroupCommand DeleteGroupCommand SetStructureNodeCommand SplitIntoPiecesCommand SetMaterialFinishCommand MakeGroupCommand MakeNestedGroupCommand MakeComponentOfCommand GroupInContextCommand ExplodeViewCommand")?;
    exports(m,core.as_any(),"RecordStep AddDimensionCommand DeleteDimensionsCommand AddTextLabelCommand MoveTextLabelsCommand MoveGuidesCommand RotateGuidesCommand MoveDimensionsCommand EditTextLabelCommand EditDimensionTextCommand DeleteTextLabelsCommand AddGuideCommand PlaceSectionPlaneCommand DeleteSectionPlanesCommand ReverseSectionPlaneCommand SetActiveSectionCommand SetSectionModeCommand SetSectionFillCommand RenameSectionPlaneCommand AddSheetCommand PlaceViewCommand SheetCommand InsertSheetCommand RemoveSheetCommand MoveSheetCommand CreatePlanCommand AddSceneCommand DeleteScenesCommand UpdateSceneCommand SceneTreeCommand MoveSectionPlanesCommand RotateSectionPlanesCommand ChangeGuideCommand DeleteGuidesCommand AddImagePlaneCommand DeleteImagePlanesCommand TransformImagePlaneCommand SetImagePlaneOpacityCommand MoveImagePlanesCommand RotateImagePlanesCommand ScaleImagePlanesCommand AddGeoPathCommand DeleteGeoPointsCommand DeleteGeoPathsCommand ToggleGeoPathClosedCommand SetGeoPathSurfaceCommand MoveGeoPathNodeCommand")?;
    m.add(
        "group_owner_list",
        core.getattr("definition_edits")?.getattr("group_owner")?,
    )?;
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"A command declining to happen, on purpose: it has put the scene back
the way it found it and says why in plain words. :class:`History` takes
it as a no-op — nothing on the undo stack, nothing in the error log —
and keeps the reason in ``last_rejected`` for the tool to show."#,
        )?;
        class(
            m,
            "CommandRejected",
            vec![builtins.getattr("Exception")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Abstract reversible operation against a :class:`Scene`."#,
        )?;
        meth!(ns, m, "__getstate__", command_state);
        meth!(ns, m, "_direct_snapshot", direct_snapshot);
        meth!(ns, m, "bind_transaction", bind_transaction);
        meth!(ns, m, "doc_after", doc_after);
        ns.set_item("keeps_paint", false)?;
        ns.set_item("keeps_groups", false)?;
        ns.set_item("keeps_rows", false)?;
        ns.set_item("doc_undo", false)?;
        ns.set_item("_journal_geometry", false)?;
        for name in ["do", "undo"] {
            let abstract_fn = py
                .import("functools")?
                .call_method1("partial", (wrap_pyfunction!(abstract_action, m)?,))?;
            abc.call_method1("abstractmethod", (&abstract_fn,))?;
            method(&ns, name, abstract_fn)?;
        }
        class(m, "Command", vec![abc.getattr("ABC")?], &ns)?
            .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Commands whose persistent changes are covered by these geometry stores
and the document field journal."#,
        )?;
        meth!(ns, m, "geometry_meshes", meshes);
        class(m, "MeshCommand", vec![m.getattr("Command")?], &ns)?
            .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        meth!(ns, m, "geometry_meshes", face_meshes);
        class(
            m,
            "FaceAttributeCommand",
            vec![m.getattr("MeshCommand")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Replace one extension's document data (``scene.plugin_data[key]``),
undoably. ``value`` None removes the key. Values are copied through JSON
both ways, so neither the caller nor the history can alias them.

With the Rust document ``scene.plugin_data`` is a document value: the
step's transaction undoes it."#,
        )?;
        meth!(ns, m, "__init__", plugin_init);
        ns.set_item("doc_undo", true)?;
        class(
            m,
            "SetPluginDataCommand",
            vec![core.getattr("PluginDataStep")?, m.getattr("Command")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Wrap an arbitrary mesh mutation with snapshot undo. The push/pull live
preview applies the *same* mutation each drag frame (then reverts via its own
snapshot), so the forming solid renders exactly as it will commit — clean,
already stitched — instead of flashing the pre-stitch seams.

Redo restores the captured *result* rather than re-running the mutation: the
closure usually closes over tool state (``base_face`` etc.) that is reset
right after commit, so re-running it on redo would crash — and re-running the
deterministic plane rebuild on stale state would be wasteful besides.

``mesh`` (when given) is the mesh to snapshot instead of the scene's loose
one — a push/pull aimed at a Group edits that group's isolated mesh."#,
        )?;
        meth!(ns, m, "__init__", mutation_init);
        ns.set_item("keeps_rows", true)?;
        ns.set_item(
            "from_states",
            builtins.call_method1("classmethod", (wrap_pyfunction!(from_states, m)?,))?,
        )?;
        class(
            m,
            "SnapshotMutation",
            vec![core.getattr("MutationStep")?, m.getattr("MeshCommand")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Wrap a file import that may add loose geometry AND/OR reference groups
(big DAE/OBJ models land as a Group). SnapshotMutation only snapshots the
mesh, so a group added by the loader would survive undo — this also
remembers and reverts the groups the import appended.

Sheets too: an agent asked for "a new sheet" appends a composition
through ``run_python``, and untracked it was no step at all — the
bridge said nothing changed, and its undo took the user's previous
edit instead (#694).

And what the mutation removed: an extension that replaces its own
container group, or a console line that deletes one, lost that group
for good on undo, which only took the new one away."#,
        )?;
        meth!(ns, m, "__init__", import_init);
        ns.set_item(
            "_TRACKED",
            (
                ("groups", "added_groups", "removed_groups"),
                ("layers", "added_layers", "removed_layers"),
                ("saved_views", "added_views", "removed_views"),
                ("dimensions", "added_dims", "removed_dims"),
                ("text_labels", "added_texts", "removed_texts"),
                ("compositions", "added_sheets", "removed_sheets"),
            ),
        )?;
        class(
            m,
            "SnapshotImport",
            vec![core.getattr("ImportStep")?, m.getattr("Command")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Run a list of sub-commands under one identity-preserving snapshot, undone
by restoring it.

The line-draw plan splits and welds edges (and punches holes in coplanar
faces); the per-command inverses don't compose into a clean whole — undoing
them piecemeal leaves orphan split edges and stray vertices behind. One
snapshot of the entire edit reverses it exactly, and because identity is
preserved, earlier history entries keep working across this undo.
``heal=False`` preserves supplied copy topology instead of applying
drawing-specific global face cleanup."#,
        )?;
        meth!(ns, m, "__init__", compound_snapshot_init);
        ns.set_item("keeps_rows", true)?;
        class(
            m,
            "SnapshotCompound",
            vec![
                core.getattr("DrawingSnapshotStep")?,
                m.getattr("MeshCommand")?,
            ],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Run a list of sub-commands plus the stitch pass under a single
identity-preserving snapshot, undone by restoring that snapshot.

Push/pull builds its edit as delta commands and then stitches the result
watertight; the stitch's splits/merges restructure the very edges those
commands own, so composing their individual undos leaves orphan edges. One
snapshot of the whole push is exact and robust — and because it preserves
object identity, *other* history entries (a drawn line, an earlier push) keep
working across this undo."#,
        )?;
        meth!(ns, m, "__init__", stitch_init);
        class(
            m,
            "MeshSnapshotCommand",
            vec![
                core.getattr("StitchSnapshotStep")?,
                m.getattr("MeshCommand")?,
            ],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"A list of commands executed and reverted as one atomic step."#,
        )?;
        meth!(ns, m, "__init__", compound_init);
        ns.set_item("journal_parts", false)?;
        class(
            m,
            "CompoundCommand",
            vec![core.getattr("CompoundStep")?, m.getattr("Command")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Replay committed commands in their original occurrence coordinate frames."#,
        )?;
        meth!(ns, m, "do", group_do);
        meth!(ns, m, "undo", group_undo);
        meth!(ns, m, "doc_after", group_after);
        ns.set_item(
            "group_start",
            builtins.call_method1("staticmethod", (wrap_pyfunction!(group_start, m)?,))?,
        )?;
        ns.set_item(
            "fold",
            builtins.call_method1("staticmethod", (wrap_pyfunction!(fold, m)?,))?,
        )?;
        class(
            m,
            "OccurrenceCommands",
            vec![m.getattr("CompoundCommand")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Top-level patch of group records; None patch removes the record.

Nested values replace whole fields. GUIDs cannot be changed by a patch.
Snapshots are detached and retained across redo, including new identities."#,
        )?;
        meth!(ns, m, "__init__", record_init);
        ns.set_item("keeps_paint", false)?;
        ns.set_item("doc_undo", true)?;
        class(
            m,
            "SetRecordCommand",
            vec![core.getattr("RecordPatchStep")?, m.getattr("Command")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        meth!(ns, m, "__init__", tag_init);
        class(
            m,
            "TagGroupsCommand",
            vec![m.getattr("SetRecordCommand")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        meth!(ns, m, "__init__", untag_init);
        class(
            m,
            "UntagGroupsCommand",
            vec![m.getattr("SetRecordCommand")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        meth!(ns, m, "__init__", registry_init);
        meth!(ns, m, "do", registry_do);
        meth!(ns, m, "undo", registry_undo);
        meth!(ns, m, "doc_undo", registry_doc);
        meth!(ns, m, "doc_after", registry_after);
        class(
            m,
            "SetRegistryCommand",
            vec![m.getattr("SetPluginDataCommand")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Legacy loose-face tags, with stable identities across redo."#,
        )?;
        meth!(ns, m, "__init__", face_init);
        ns.set_item("keeps_paint", false)?;
        class(
            m,
            "SetFaceTagsCommand",
            vec![core.getattr("FaceTagsStep")?, m.getattr("Command")?],
            &ns,
        )?
        .setattr("__module__", "core.history")?;
    }
    {
        let ns = PyDict::new(py);
        ns.set_item(
            "__doc__",
            r#"Undo/redo stacks. ``execute`` is TRANSACTIONAL: if a command throws
mid-mutation, the mesh is restored to its pre-command state, the failure
is logged (stderr + ``error_log`` file), and nothing lands on the undo
stack — a failed operation must be a no-op, never a half-committed mess
(an aas.igz-style aborted draw left a quarter circle, an unsplit face and
a duplicated edge behind, with the traceback swallowed by the Qt event
loop). Same fail-safe doctrine as the BIM push guard, one level up."#,
        )?;
        meth!(ns, m, "__init__", history_init);
        meth!(ns, m, "add_listener", add_listener);
        meth!(ns, m, "changes_since", changes_since);
        meth!(ns, m, "execute", execute);
        meth!(ns, m, "undo", undo);
        meth!(ns, m, "redo", redo);
        meth!(ns, m, "clear", clear);
        meth!(ns, m, "context_changed", context_changed);
        ns.set_item("error_log", "ingetrazo-errors.log")?;
        ns.set_item("max_steps", 200)?;
        prop(&ns, "revision", wrap_pyfunction!(revision, m)?.into_any())?;
        ns.set_item(
            "default_error_log",
            builtins.call_method1("staticmethod", (wrap_pyfunction!(default_error_log, m)?,))?,
        )?;
        class(m, "History", vec![], &ns)?.setattr("__module__", "core.history")?;
    }
    for name in "MakeGroupCommand HideCommand HideEdgesCommand AssignLayerCommand MoveGroupCommand RotateGroupCommand ScaleGroupCommand FlipGroupsCommand ChangeAxesCommand AddEdgeCommand AddFaceCommand DeleteEdgesCommand DeleteFaceCommand EraseSelectionCommand MoveVerticesCommand FlipVerticesCommand RotateVerticesCommand ScaleVerticesCommand HealOverlapsCommand RebuildPlanarFacesCommand RebuildPlaneFacesCommand ExplodeGroupCommand TagCurveCommand ExplodeCurvesCommand AutoTagDrawnFacesCommand PruneOrphanEdgesCommand CoplanarMergeCommand StitchSolidCommand SimplifyMeshCommand RestampMaterialCommand EditTexturedMaterialCommand DeleteMaterialCommand".split_whitespace() {m.getattr("MeshCommand")?.call_method1("register",(m.getattr(name)?,))?;}
    for name in "SetFaceColorCommand SetFaceOpacityCommand SetFaceTextureCommand SetFaceBackCommand SetFaceMaterialTagCommand FlipFacesCommand".split_whitespace() {m.getattr("FaceAttributeCommand")?.call_method1("register",(m.getattr(name)?,))?;}
    for name in ["NativeCommand", "RecordStep"] {
        m.getattr("Command")?
            .call_method1("register", (m.getattr(name)?,))?;
    }
    core.add("history", m)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("ingetrazo_core.history", m)?;
    Ok(())
}
