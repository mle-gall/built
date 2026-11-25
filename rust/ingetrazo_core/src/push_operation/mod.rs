// SPDX-License-Identifier: GPL-3.0-or-later
//! The extrusion operation shared by the interactive tool and protocol.
//! A staged preview holds an identity-preserving store snapshot. Only the
//! geometry actually accepted by the kernel becomes the next history step.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
    IntoPyObjectExt,
};
type Obj<'p> = Bound<'p, PyAny>;
mod mutation;
pub(crate) use mutation::paint_of;
mod refusal;
const MIN_EXTRUDE: f64 = 2e-4;
fn flag(o: &Obj, k: &str) -> PyResult<bool> {
    o.getattr(k)?.is_truthy()
}
fn number(o: &Obj, k: &str) -> PyResult<f64> {
    o.getattr(k)?.extract()
}
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn scans(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("core.push_scans")
}
fn core(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core")
}
fn module(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("core.push_pull")
}
fn tol(py: Python<'_>) -> PyResult<f64> {
    py.import("core.mesh")?.getattr("_STITCH_TOL")?.extract()
}
fn bump(scene: &Obj) -> PyResult<()> {
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}
fn target_group_mesh<'p>(op: &Obj<'p>) -> PyResult<Obj<'p>> {
    let group = op.getattr("_group")?;
    if group.is_none() {
        Ok(group)
    } else {
        group.getattr("mesh")
    }
}
#[pyclass(module = "core.push_pull")]
struct GroupScene {
    scene: Py<PyAny>,
    #[pyo3(get)]
    mesh: Py<PyAny>,
    #[pyo3(get)]
    selection: Py<PyAny>,
}
#[pymethods]
impl GroupScene {
    #[getter]
    fn faces<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.mesh.bind(py).getattr("faces")
    }
    #[getter]
    fn edges<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.mesh.bind(py).getattr("edges")
    }
    #[getter]
    fn version<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        self.scene.bind(py).getattr("version")
    }
    #[setter]
    fn set_version(&self, v: &Obj) -> PyResult<()> {
        self.scene.bind(v.py()).setattr("version", v)
    }
}
#[pyfunction]
fn target_scene<'p>(op: &Obj<'p>, scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let mesh = target_group_mesh(op)?;
    if mesh.is_none() {
        return Ok(scene.clone());
    }
    Ok(Bound::new(
        op.py(),
        GroupScene {
            scene: scene.clone().unbind(),
            mesh: mesh.unbind(),
            selection: scene.getattr("selection")?.unbind(),
        },
    )?
    .into_any())
}
#[pyfunction]
fn target_mesh<'p>(op: &Obj<'p>, scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let mesh = target_group_mesh(op)?;
    if mesh.is_none() {
        scene.getattr("mesh")
    } else {
        Ok(mesh)
    }
}
#[pyfunction]
fn init(op: &Obj) -> PyResult<()> {
    reset(op)?;
    op.setattr("_keep_mode", false)?;
    op.setattr("_topped_out", false)
}
#[pyfunction]
fn reset(op: &Obj) -> PyResult<()> {
    let py = op.py();
    for field in [
        "hovered_face",
        "base_face",
        "_limit_in",
        "_group",
        "_hover_group",
        "_anchor",
        "_grab",
        "_normal",
        "_infer_cache",
        "_inference_point",
        "_inference_kind",
        "_last",
    ] {
        op.setattr(field, py.None())?;
    }
    for field in [
        "dragging",
        "_attached",
        "_prism_cap",
        "_keep_base",
        "_selected_arm",
        "_drag_pre_oriented",
        "_refused",
    ] {
        op.setattr(field, false)?;
    }
    for field in [
        "_prism_verts",
        "_extra_faces",
        "_extra_normals",
        "_cap_positions",
        "_plane_levels",
        "_light_faces",
        "_light_wires",
    ] {
        op.setattr(field, PyList::empty(py))?;
    }
    op.setattr("extrusion", 0.)?;
    op.setattr("_prism_applied", 0.)?;
    op.setattr("_stages", core(py)?.call_method0("PushStages")?)
}
#[pyfunction]
fn move_cap(op: &Obj, viewport: &Obj, delta: f64, applied: f64) -> PyResult<()> {
    let scene = viewport.getattr("scene")?;
    let mesh = target_mesh(op, &scene)?;
    let step = op.getattr("_normal")?.mul(delta)?;
    let verts = op.getattr("_prism_verts")?;
    let store = mesh.getattr_opt("_store")?.filter(|s| !s.is_none());
    if let Some(st) = store {
        let keys = verts
            .try_iter()?
            .map(|v| v?.getattr("key"))
            .collect::<PyResult<Vec<_>>>()?;
        st.call_method1("move_vertices", (keys, step.call_method0("toTuple")?))?;
        mesh.call_method0("_bump")?;
    } else {
        for v in verts.try_iter()? {
            mesh.call_method1("move_vertex", (v?, &step))?;
        }
    }
    op.setattr("_prism_applied", applied)?;
    bump(&scene)
}
#[pyfunction]
fn revert_prism_preview(op: &Obj, viewport: &Obj) -> PyResult<()> {
    let applied = number(op, "_prism_applied")?;
    if applied != 0. {
        move_cap(op, viewport, -applied, 0.)?;
    }
    Ok(())
}
#[pyfunction]
fn drop_stale_stage(op: &Obj, viewport: &Obj) -> PyResult<bool> {
    let stages = op.getattr("_stages")?;
    if stages.getattr("snapshot")?.is_none() {
        return Ok(false);
    }
    let mesh = target_mesh(op, &viewport.getattr("scene")?)?;
    if !stages.call_method1("stale", (mesh,))?.is_truthy()? {
        return Ok(false);
    }
    viewport.call_method1("set_suppressed_faces", (PySet::empty(op.py())?,))?;
    op.call_method0("_reset")?;
    viewport.call_method0("update")?;
    Ok(true)
}
#[pyfunction]
fn revert_preview(op: &Obj, viewport: &Obj) -> PyResult<()> {
    revert_prism_preview(op, viewport)?;
    if !drop_stale_stage(op, viewport)? {
        let stages = op.getattr("_stages")?;
        if !stages.getattr("distance")?.is_none() {
            let scene = viewport.getattr("scene")?;
            stages.call_method1("restore", (target_mesh(op, &scene)?, scene))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn staged_command<'p>(op: &Obj<'p>, viewport: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = op.py();
    let stages = op.getattr("_stages")?;
    let d = number(op, "extrusion")?;
    if stages.getattr("distance")?.is_none()
        || flag(op, "_keep_base")?
        || drop_stale_stage(op, viewport)?
        || d.abs() < MIN_EXTRUDE
        || !stages.call_method1("holds", (d, tol(py)?))?.is_truthy()?
    {
        return Ok(none(py));
    }
    let scene = viewport.getattr("scene")?;
    let mesh = target_mesh(op, &scene)?;
    stages.call_method1("slide", (&mesh, scene, d))?;
    let kw = PyDict::new(py);
    kw.set_item("mesh", target_group_mesh(op)?)?;
    let cmd = py
        .import("ingetrazo_core.history")?
        .getattr("SnapshotMutation")?
        .call_method(
            "from_states",
            (
                stages.getattr("snapshot")?,
                mesh.call_method0("capture_state")?,
            ),
            Some(&kw),
        )?;
    op.setattr("_stages", core(py)?.call_method0("PushStages")?)?;
    op.setattr("_refused", false)?;
    Ok(cmd)
}
#[pyfunction]
fn compute_inward_limit(op: &Obj, scene: &Obj) -> PyResult<()> {
    let py = op.py();
    op.setattr("_limit_in", py.None())?;
    let face = op.getattr("base_face")?;
    if !flag(op, "_attached")? || face.is_none() {
        return Ok(());
    }
    let n = or_face(op, "_normal", &face, "normal")?.call_method0("normalized")?;
    let anchor = or_face(op, "_anchor", &face, "centroid")?;
    op.setattr(
        "_limit_in",
        scans(py)?.call_method1(
            "inward_limit",
            (scene.getattr("mesh")?, face, n, anchor, tol(py)?),
        )?,
    )
}
fn or_face<'p>(op: &Obj<'p>, key: &str, face: &Obj<'p>, method: &str) -> PyResult<Obj<'p>> {
    let value = op.getattr(key)?;
    if value.is_none() {
        face.call_method0(method)
    } else {
        Ok(value)
    }
}
fn flash(vp: &Obj, msg: &str, ms: u32, args: &[(&str, Obj)]) -> PyResult<()> {
    let kw = PyDict::new(vp.py());
    for (k, v) in args {
        kw.set_item(k, v)?;
    }
    let text = vp
        .py()
        .import("core.i18n")?
        .call_method("tr", (msg,), Some(&kw))?;
    vp.call_method1("flash_status", (text, ms))?;
    Ok(())
}
#[pyfunction(signature=(op,viewport=None))]
fn clamp_extrusion(op: &Obj, viewport: Option<&Obj>) -> PyResult<()> {
    if let Some(limit) = op.getattr("_limit_in")?.extract::<Option<f64>>()? {
        if number(op, "extrusion")? < -limit {
            op.setattr("extrusion", -limit)?;
            if let Some(vp) = viewport {
                flash(
                    vp,
                    "Push limited to {value} — deeper would leave the solid",
                    5000,
                    &[(
                        "value",
                        vp.py()
                            .import("core.units")?
                            .call_method1("fmt_len", (limit,))?,
                    )],
                )?;
            }
        }
    }
    Ok(())
}
#[pyfunction]
fn cap_loop_positions<'p>(face: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let vec = core(face.py())?.getattr("Vec3")?;
    let mut points = Vec::new();
    for ring in
        std::iter::once(Ok(face.getattr("vertices")?)).chain(face.getattr("holes")?.try_iter()?)
    {
        for p in ring?.try_iter()? {
            points.push(vec.call1((p?,))?);
        }
    }
    Ok(points)
}
#[pyfunction(signature=(face,mesh=None))]
fn surface_normal<'p>(face: &Obj<'p>, mesh: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let mesh = match mesh {
        Some(m) => m.clone(),
        None => face.getattr("_m")?,
    };
    scans(face.py())?.call_method1("surface_normal", (mesh, face))
}
#[pyfunction]
fn outward_normal<'p>(cls: &Obj<'p>, face: &Obj<'p>, mesh: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = face.py();
    if py
        .import("core.orient")?
        .call_method1("is_closed", (mesh,))?
        .is_truthy()?
        || py
            .import("core.topology")?
            .call_method1("_mesh_is_flat", (mesh,))?
            .is_truthy()?
    {
        face.call_method0("normal")
    } else {
        cls.call_method1("_surface_normal", (face, mesh))
    }
}
#[pyfunction]
fn classify_base<'p>(op: &Obj<'p>, scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    scans(op.py())?.call_method1(
        "classify_base",
        (scene.getattr("mesh")?, op.getattr("base_face")?),
    )
}
#[pyfunction]
fn spend_keep_mode(op: &Obj, viewport: &Obj) -> PyResult<()> {
    if flag(op, "_keep_mode")? {
        op.setattr("_keep_mode", false)?;
        op.setattr("_keep_base", false)?;
        op.call_method1("refresh_hint", (viewport,))?;
    }
    Ok(())
}
#[pyfunction]
fn retype_state<'p>(op: &Obj<'p>, cmd: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = op.py();
    let out = PyDict::new(py);
    out.set_item("cmd", cmd)?;
    for (key, attr) in [
        ("face", "base_face"),
        ("group", "_group"),
        ("keep_base", "_keep_base"),
        ("prism_cap", "_prism_cap"),
        ("pre_oriented", "_drag_pre_oriented"),
        ("limit_in", "_limit_in"),
    ] {
        out.set_item(key, op.getattr(attr)?)?;
    }
    for (key, attr) in [
        ("cap_positions", "_cap_positions"),
        ("extras", "_extra_faces"),
        ("extra_normals", "_extra_normals"),
    ] {
        out.set_item(key, py.get_type::<PyList>().call1((op.getattr(attr)?,))?)?;
    }
    let n = op.getattr("_normal")?;
    out.set_item(
        "normal",
        if n.is_none() {
            n
        } else {
            core(py)?.call_method1("Vec3", (n,))?
        },
    )?;
    out.set_item(
        "sign",
        if number(op, "extrusion")? >= 0. {
            1.
        } else {
            -1.
        },
    )?;
    Ok(out)
}
#[pyfunction]
fn commit(op: &Obj, viewport: &Obj) -> PyResult<()> {
    let py = op.py();
    viewport.call_method1("set_hover", (py.None(),))?;
    op.setattr("_light_faces", PyList::empty(py))?;
    op.setattr("_light_wires", PyList::empty(py))?;
    viewport.call_method1("set_suppressed_faces", (PySet::empty(py)?,))?;
    let mut cmd = staged_command(op, viewport)?;
    if cmd.is_none() {
        revert_preview(op, viewport)?;
    }
    if op.getattr("base_face")?.is_none() || number(op, "extrusion")?.abs() < MIN_EXTRUDE {
        op.call_method0("_reset")?;
        spend_keep_mode(op, viewport)?;
        viewport.call_method0("update")?;
        return Ok(());
    }
    op.setattr("_topped_out", false)?;
    op.setattr("_multi_missed", 0)?;
    if cmd.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("mesh", target_group_mesh(op)?)?;
        let action = op.getattr(if flag(op, "_extra_faces")? {
            "_mutate_many"
        } else {
            "_mutate_or_top_out"
        })?;
        cmd = py
            .import("ingetrazo_core.history")?
            .getattr("SnapshotMutation")?
            .call((action,), Some(&kw))?;
    }
    viewport
        .getattr("history")?
        .call_method1("execute", (&cmd,))?;
    let mut last = none(py);
    let partial = flag(op, "_topped_out")?;
    if partial || !flag(op, "_refused")? {
        op.get_type()
            .setattr("last_distance", op.getattr("extrusion")?)?;
        last = retype_state(op, &cmd)?.into_any();
        if partial {
            flash(
                viewport,
                "Push stopped at {d}: going further would break the solid",
                4000,
                &[(
                    "d",
                    py.import("core.units")?
                        .call_method1("fmt_len", (number(op, "extrusion")?.abs(),))?,
                )],
            )?;
        } else if flag(op, "_multi_missed")? {
            let total = op.getattr("_extra_faces")?.len()? + 1;
            let missed = op.getattr("_multi_missed")?.extract::<usize>()?;
            flash(
                viewport,
                "Pushed {done} of {total} faces — the others would break the solid",
                6000,
                &[
                    ("done", (total - missed).into_bound_py_any(py)?),
                    ("total", total.into_bound_py_any(py)?),
                ],
            )?;
        }
    } else {
        flash(
            viewport,
            if op.getattr("_store_refusal")?.is_none() {
                "Push refused: would break the solid (left it unchanged)"
            } else {
                "Push refused: the geometry kernel could not finish it (left it unchanged; a report was saved)"
            },
            6000,
            &[],
        )?;
    }
    op.call_method0("_reset")?;
    spend_keep_mode(op, viewport)?;
    op.setattr("_last", last)?;
    viewport.call_method0("update")?;
    Ok(())
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let b = py.import("builtins")?;
    m.add("_REPROS", PySet::empty(py)?)?;
    m.add("_MIN_EXTRUDE", MIN_EXTRUDE)?;
    let ns = PyDict::new(py);
    ns.set_item("__module__", "core.push_pull")?;
    ns.set_item("last_distance", py.None())?;
    ns.set_item("_store_refusal", py.None())?;
    ns.set_item("_CURVE_FACET_COS", 0.85)?;
    ns.set_item("_CLAMP_TRIES", 6)?;
    let method = py.import("functools")?.getattr("partialmethod")?;
    macro_rules! methods {($($name:literal => $f:path),* $(,)?)=>{$(ns.set_item($name,method.call1((wrap_pyfunction!($f,m)?,))?)?;)*};}
    methods!("__init__"=>init,"_reset"=>reset,"_target_scene"=>target_scene,"_target_mesh"=>target_mesh,"_move_cap"=>move_cap,
        "_revert_prism_preview"=>revert_prism_preview,"_staged_command"=>staged_command,"_drop_stale_stage"=>drop_stale_stage,"_revert_preview"=>revert_preview,
        "_compute_inward_limit"=>compute_inward_limit,"_clamp_extrusion"=>clamp_extrusion,"_classify_base"=>classify_base,"_commit"=>commit,"_retype_state"=>retype_state,
        "_mutate_many"=>mutation::mutate_many,"_mutate_or_top_out"=>mutation::mutate_or_top_out,"_mutate"=>mutation::mutate,"_mutate_guarded"=>mutation::mutate_guarded,
        "_mutate_inner"=>mutation::mutate_inner,"_cap_passes_a_wall_vertex"=>mutation::passes_wall,"_spend_keep_mode"=>spend_keep_mode);
    for (name, kind, f) in [
        (
            "_outward_normal",
            "classmethod",
            wrap_pyfunction!(outward_normal, m)?,
        ),
        (
            "_surface_normal",
            "staticmethod",
            wrap_pyfunction!(surface_normal, m)?,
        ),
        (
            "_cap_loop_positions",
            "staticmethod",
            wrap_pyfunction!(cap_loop_positions, m)?,
        ),
    ] {
        ns.set_item(name, b.call_method1(kind, (f,))?)?;
    }
    let attr = py.import("operator")?.getattr("attrgetter")?;
    for (key, attr_name) in [
        ("_stage_d", "distance"),
        ("_stage_cache", "cached_sides"),
        ("_stage_new", "made"),
        ("_stage_slow", "slow"),
        ("_stage_refusal", "refusal"),
        ("_preview_snapshot", "snapshot"),
        ("_stage_verts", "vertices"),
    ] {
        ns.set_item(
            key,
            b.call_method1(
                "property",
                (attr.call1((format!("_stages.{attr_name}"),))?,),
            )?,
        )?;
    }
    m.add(
        "PushPullOperation",
        b.call_method1("type", ("PushPullOperation", (b.getattr("object")?,), ns))?,
    )?;
    Ok(())
}
