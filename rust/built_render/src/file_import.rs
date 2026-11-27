//! Import transactions preserve the receiving document and its undo baseline
//! when a format reader fails midway. Format readers remain Python backends.
use super::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

/// Ask once for both import options. The header only suggests the drawing unit.
#[pyfunction]
fn dxf_options(win: &Bound<'_, PyAny>, header_scale: Option<f64>) -> PyResult<Option<(f64, bool)>> {
    const UNITS: [(&str, &str, f64); 5] = [
        ("mm", "Millimetres", 0.001), ("cm", "Centimetres", 0.01),
        ("m", "Metres", 1.0), ("in", "Inches", 0.0254), ("ft", "Feet", 0.3048),
    ];
    let py = win.py();
    let store = py.import("built_core.settings")?.call_method0("settings")?;
    let saved = store.call_method1("value", ("import/dxf_unit", "mm"))?;
    let saved_key = saved.extract::<&str>().ok();
    let default = UNITS.iter().find(|u| Some(u.2) == header_scale)
        .or_else(|| UNITS.iter().find(|u| Some(u.0) == saved_key))
        .unwrap_or(&UNITS[0]);
    let kwargs = PyDict::new(py);
    kwargs.set_item("type", py.get_type::<pyo3::types::PyBool>())?;
    let faces = store.call_method("value", ("import/dxf_make_faces", false), Some(&kwargs))?;
    let checklist = PyList::new(py, [(translated(py, "Create faces from closed planar outlines", None)?, faces)])?;
    let buttons = PyList::new(py, [("cancel", translated(py, "Cancel", None)?)])?;
    for (key, label, _) in UNITS {
        buttons.append((key, translated(py, label, None)?))?;
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("default", default.0)?;
    kwargs.set_item("cancel", "cancel")?;
    kwargs.set_item("checklist", &checklist)?;
    let answer = py.import("built_render.chrome_dialogs")?.call_method("ask", (
        win, translated(py, "What unit is this drawing in?", None)?,
        translated(py, "What unit is this drawing in? (CAD headers often lie)", None)?, buttons,
    ), Some(&kwargs))?;
    let choice = answer.extract::<Option<&str>>()?.unwrap_or(default.0);
    let Some((key, _, factor)) = UNITS.iter().find(|u| u.0 == choice) else {
        return Ok(None);
    };
    let faces = checklist.get_item(0)?.get_item(1)?.extract::<bool>()?;
    store.call_method1("setValue", ("import/dxf_unit", key))?;
    store.call_method1("setValue", ("import/dxf_make_faces", faces))?;
    Ok(Some((*factor, faces)))
}

#[pyfunction]
fn guarded_load(
    load: &Bound<'_, PyAny>,
    errors: &Bound<'_, PyList>,
    scene: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if let Err(err) = load.call1((scene,)) {
        errors.append(err.value(scene.py()))?;
        return Err(err);
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(win,load,bar,title,describe=None,command_type=None))]
fn run_import<'py>(
    win: &Bound<'py, PyAny>,
    load: &Bound<'py, PyAny>,
    bar: &Bound<'py, PyAny>,
    title: &str,
    describe: Option<&Bound<'py, PyAny>>,
    command_type: Option<Bound<'py, PyAny>>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = win.py();
    let errors = PyList::empty(py);
    let guarded = py.import("functools")?.call_method1(
        "partial",
        (wrap_pyfunction!(guarded_load, py)?, load, &errors),
    )?;
    let ty = match command_type {
        Some(c) => c,
        None => py.import("built_core.history")?.getattr("SnapshotImport")?,
    };
    let command = ty.call1((guarded,))?;
    let clean = !win.call_method0("_is_dirty")?.is_truthy()?;
    let vp = win.getattr("viewport")?;
    let hist = vp.getattr("history")?;
    if let Err(err) = hist.call_method1("execute", (&command,)) {
        errors.append(err.value(py))?;
    }
    // History can reject a transaction before invoking its loader.
    let error = if !errors.is_empty() {
        errors.get_item(0)?
    } else {
        let e = hist.getattr("last_error")?;
        let e = if e.is_truthy()? {
            e
        } else {
            hist.getattr("last_rejected")?
        };
        if !e.is_truthy()? {
            return Ok(Some(command));
        }
        py.import("builtins")?.call_method1("RuntimeError", (e,))?
    };
    let msg = match describe {
        Some(f) => f.call1((&error,))?,
        None => error.str()?.into_any(),
    };
    let dialogs = py.import("built_render.chrome_dialogs")?;
    dialogs.call_method1("close_failed", (bar, &msg))?;
    if clean {
        win.setattr("_saved_version", vp.getattr("scene")?.getattr("version")?)?;
    }
    win.call_method0("_update_title")?;
    dialogs.call_method1("critical", (win, title, msg))?;
    Ok(None)
}
#[pyfunction]
fn prepare_import(
    vp: &Bound<'_, PyAny>,
    cmd: &Bound<'_, PyAny>,
    progress: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if let Ok(groups) = cmd.getattr("added_groups") {
        for g in groups.try_iter()? {
            progress.call1((0.97, "Preparing display…"))?;
            let _ = vp.call_method1("_group_chunk", (g?,));
        }
    }
    progress.call1((1.0, "Done"))?;
    Ok(())
}
fn message(py: Python<'_>, key: &str, args: &Bound<'_, PyDict>) -> PyResult<String> {
    Ok(translated(py, key, Some(args))?.extract()?)
}
#[pyfunction]
#[pyo3(signature=(offset=None))]
fn survey_note(py: Python<'_>, offset: Option<&Bound<'_, PyAny>>) -> PyResult<String> {
    let Some(offset) = offset else {
        return Ok(String::new());
    };
    if !offset.is_truthy()? {
        return Ok(String::new());
    }
    let args = PyDict::new(py);
    args.set_item("x", offset.get_item(0)?)?;
    args.set_item("y", offset.get_item(1)?)?;
    Ok(format!(
        "  ·  {}",
        message(
            py,
            "moved near the origin (survey coordinates); offset {x:,.0f}, {y:,.0f} m",
            &args
        )?
    ))
}
#[pyfunction]
#[pyo3(signature=(win,name,offset=None))]
fn report_import(
    win: &Bound<'_, PyAny>,
    name: &str,
    offset: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let py = win.py();
    let note = survey_note(py, offset)?;
    if !note.is_empty() {
        win.call_method0("_on_zoom_extents")?;
    }
    let vars = PyDict::new(py);
    vars.set_item("name", name)?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (
            message(py, "Imported {name}", &vars)? + &note,
            if note.is_empty() { 3000 } else { 8000 },
        ),
    )?;
    Ok(())
}
#[pyfunction]
fn load_file(
    kind: &str,
    path: &Bound<'_, PyAny>,
    params: &Bound<'_, PyDict>,
    result: &Bound<'_, PyDict>,
    scene: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = scene.py();
    match kind {
        "rfa" => {
            let backend = py.import("built_core.formats.rfa")?;
            let payload = backend.call_method1("read_rfa", (path,))?;
            result.set_item(
                "report",
                backend.call_method1("apply_payload", (scene, payload))?,
            )?;
        }
        "step" => {
            py.import("built_core.formats.step")?.call_method1("apply_step", (scene, params.get_item("prepared")?, params.get_item("accept_partial")?))?;
        }
        "stl" => {
            py.import("built_core.formats.stl")?
                .call_method1("add_stl_mesh", (scene, path, params.get_item("target")?))?;
        }
        "dxf" => {
            let stats = py
                .import("built_core.formats.dxf_in")?
                .getattr("load_dxf")?
                .call((scene, path), Some(params))?;
            result.call_method1("update", (stats,))?;
        }
        kind => {
            let (module, function) = match kind {
                "glb" => ("built_core.formats.glb", "load_glb"),
                "dae" => ("built_core.formats.dae", "load_dae"),
                "obj" => ("built_core.formats.obj", "load_obj"),
                _ => {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "unknown import format",
                    ))
                }
            };
            let offset = py
                .import(module)?
                .getattr(function)?
                .call((scene, path), Some(params))?;
            result.set_item("offset", offset)?;
        }
    }
    Ok(())
}
#[pyfunction]
fn dxf_failure(err: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = err.py();
    if !err.is_instance(&py.import("built_core.formats.dxf_in")?.getattr("NothingToImport")?)? {
        return Ok(err.str()?.to_string());
    }
    let args = PyDict::new(py);
    args.set_item("name", path.getattr("name")?)?;
    let mut text=message(py,"“{name}” was read, but it has nothing that can be imported: no lines, polylines, arcs, circles, ellipses, splines or faces in the model space.",&args)?;
    let skipped: std::collections::BTreeMap<String, usize> = err.getattr("skipped")?.extract()?;
    if !skipped.is_empty() {
        args.set_item(
            "kinds",
            skipped
                .into_iter()
                .map(|(k, n)| format!("{k} ×{n}"))
                .collect::<Vec<_>>()
                .join(", "),
        )?;
        text.push_str("\n\n");
        text.push_str(&message(py, "Left out: {kinds}", &args)?);
    }
    Ok(text)
}
#[pyfunction]
#[pyo3(signature=(win,path,kind,scale=1.0,simplify="principal",label=None))]
fn import_file(
    win: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    kind: &str,
    scale: f64,
    simplify: &str,
    label: Option<&Bound<'_, PyAny>>,
) -> PyResult<bool> {
    import_prepared(win,path,kind,scale,simplify,label,None)
}

#[pyfunction]
fn confirm_step(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    shown: &Bound<'_, PyAny>,
    prepared: &Bound<'_, PyAny>,
    answer: &str,
) -> PyResult<bool> {
    if answer != "import" || !win.getattr("viewport")?.getattr("scene")?.is(scene) {
        return Ok(false);
    }
    import_prepared(
        win,
        path,
        "step",
        1.,
        "principal",
        Some(shown),
        Some(prepared),
    )
}

fn import_prepared(
    win: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    kind: &str,
    scale: f64,
    simplify: &str,
    label: Option<&Bound<'_, PyAny>>,
    accepted: Option<&Bound<'_, PyAny>>,
) -> PyResult<bool> {
    let py = win.py();
    let shown = label.unwrap_or(path);
    let name = shown.getattr("name")?.extract::<String>()?;
    let args = PyDict::new(py);
    args.set_item("name", &name)?;
    let error_title = match kind {
        "rfa" => "Import .rfa family failed",
        "obj" => "Import OBJ failed",
        "stl" => "Import STL failed",
        "step" => "Import STEP failed",
        "dae" => "Import DAE failed",
        "glb" => "Import glTF/GLB failed",
        "dxf" if label.is_some() => "Import DWG failed",
        "dxf" => "Import DXF failed",
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "unknown import format",
            ))
        }
    };
    let title = translated(py, error_title, None)?.extract::<String>()?;
    let dialogs = py.import("built_render.chrome_dialogs")?;
    let params = PyDict::new(py);
    let mut make_faces = false;
    if kind == "step" {
        let prepared = if let Some(p)=accepted{p.clone()}else{match py.import("built_core.formats.step")?.call_method1("prepare_step", (path,)) {
            Ok(p) => p,
            Err(e) => {
                dialogs.call_method1("critical", (win, &title, e.value(py).str()?))?;
                return Ok(false);
            }
        }};
        let problems = prepared.get_item("problems")?.extract::<Vec<String>>()?;
        if !problems.is_empty() && accepted.is_none() {
            let buttons = PyList::new(py, [("cancel", translated(py, "Cancel", None)?), ("import", translated(py,"Import available bodies", None)?)])?;
            let kw = PyDict::new(py);
            kw.set_item("cancel", "cancel")?;
            kw.set_item("default", "cancel")?;
            let done=py.import("functools")?.call_method1("partial",(wrap_pyfunction!(confirm_step,py)?,win,win.getattr("viewport")?.getattr("scene")?,path,shown,&prepared))?;
            dialogs.call_method("ask_later", (win, translated(py,"Import partial STEP model?",None)?, problems.join("\n"), buttons,done),Some(&kw))?;
            return Ok(false);
        }
        params.set_item("accept_partial", !problems.is_empty())?;
        params.set_item("prepared", prepared)?;
    }
    if kind == "dxf" {
        let backend = py.import("built_core.formats.dxf_in")?;
        let source = match backend.call_method1("prepare_import", (path,)) {
            Ok(doc) => doc,
            Err(e) => {
                dialogs.call_method1(
                    "critical",
                    (
                        win,
                        translated(py, "Import DXF failed", None)?,
                        e.value(py).str()?,
                    ),
                )?;
                return Ok(false);
            }
        };
        let (doc, suggested): (Bound<'_, PyAny>, Bound<'_, PyAny>) = source.extract()?;
        let opts = win.call_method1("_dxf_options", (shown, suggested))?;
        let Some((scale, faces)) = opts.extract::<Option<(f64, bool)>>()? else {
            return Ok(false);
        };
        make_faces = faces;
        params.set_item("doc", doc)?;
        params.set_item("name", shown.getattr("stem")?)?;
        params.set_item("scale", scale)?;
        params.set_item("make_faces", faces)?;
    }
    let (bar, progress): (Bound<'_, PyAny>, Bound<'_, PyAny>) = win
        .call_method1(
            "_import_progress",
            (message(py, "Importing {name}…", &args)?,),
        )?
        .extract()?;
    let result = PyDict::new(py);
    params.set_item("progress", &progress)?;
    if kind == "obj" {
        params.set_item("scale", scale)?;
        params.set_item("require_geometry", true)?;
    }
    if kind == "stl" {
        let kw = PyDict::new(py);
        kw.set_item("report", &result)?;
        let (target, error): (Bound<'_, PyAny>, Bound<'_, PyAny>) = win
            .call_method(
                "_parse_stl_threaded",
                (path, scale, simplify, &progress),
                Some(&kw),
            )?
            .extract()?;
        if !error.is_none() {
            dialogs.call_method1("close_failed", (&bar, &error))?;
            dialogs.call_method1("critical", (win, &title, error.str()?))?;
            return Ok(false);
        }
        params.set_item("target", target)?;
    }
    let partial = py.import("functools")?.getattr("partial")?;
    let load = partial.call1((
        wrap_pyfunction!(load_file, py)?,
        kind,
        path,
        &params,
        &result,
    ))?;
    let kw = PyDict::new(py);
    if kind == "rfa" {
        kw.set_item(
            "command_type",
            py.import("built_core.formats.rfa")?.getattr("RfaImport")?,
        )?;
    }
    if kind == "dxf" {
        let bound = PyDict::new(py);
        bound.set_item("path", shown)?;
        kw.set_item(
            "describe",
            partial.call((wrap_pyfunction!(dxf_failure, py)?,), Some(&bound))?,
        )?;
    }
    let cmd = win.call_method("_run_import", (load, &bar, &title), Some(&kw))?;
    if cmd.is_none() {
        return Ok(false);
    }
    win.call_method1("_prepare_import_display", (&cmd, &progress))?;
    bar.call_method0("close")?;
    if kind == "dxf" {
        win.call_method0("_on_zoom_extents")?;
    }
    win.getattr("viewport")?.call_method0("update")?;
    win.setattr("_import_name", &name)?;
    win.call_method0("_update_title")?;
    let offset = result.get_item("offset")?;
    if kind == "dxf" {
        for k in ["edges", "groups"] {
            args.set_item(k, result.call_method1("get", (k, 0))?)?;
        }
        let mut msg = message(
            py,
            "Imported {name}: {edges} edges in {groups} tagged groups",
            &args,
        )?;
        if make_faces {
            args.set_item("n", result.call_method1("get", ("outline_faces", 0))?)?;
            msg += "  ·  ";
            msg += &message(py, "{n} faces made from closed outlines", &args)?;
        }
        msg += &survey_note(py, offset.as_ref())?;
        win.call_method0("statusBar")?
            .call_method1("showMessage", (msg, 8000))?;
    } else {
        if kind == "step" {
            let p = params.get_item("prepared")?.unwrap();
            for k in ["exact","faceted","rejected","missing"] { args.set_item(k,p.get_item(k)?)?; }
            let mut text=message(py,"Imported {name}: {exact} exact, {faceted} faceted, {rejected} rejected, {missing} missing",&args)?;
            if p.get_item("synthetic_file_name")?.is_truthy()? {
                text.push_str(" · ");
                text.push_str(&translated(py,"Supplied missing STEP file name",None)?.extract::<String>()?);
            }
            win.call_method0("statusBar")?.call_method1("showMessage",(text,8000))?;
        } else {
            report_import(win, &name, offset.as_ref())?;
        }
    }
    if let Some(lines) = result.get_item("report")? {
        let lines = lines.extract::<Vec<String>>()?;
        if !lines.is_empty() {
            dialogs.call_method1(
                "information",
                (
                    win,
                    translated(py, ".rfa family import report", None)?,
                    lines.join("\n"),
                ),
            )?;
        }
    }
    Ok(true)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(recover_selection,m)?)?;
    m.add_function(wrap_pyfunction!(run_import, m)?)?;
    m.add_function(wrap_pyfunction!(prepare_import, m)?)?;
    m.add_function(wrap_pyfunction!(import_file, m)?)?;
    m.add_function(wrap_pyfunction!(report_import, m)?)?;
    m.add_function(wrap_pyfunction!(survey_note, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_failure, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_options, m)?)?;
    Ok(())
}

#[pyfunction]
fn recover_selection(win: &Bound<'_, PyAny>, faceted: bool) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let history = vp.getattr("history")?;
    let api = py.import("built_core.command_protocol")?;
    let targets = selection_targets(&scene)?;
    if targets.is_empty() {
        vp.call_method1(
            "flash_status",
            (
                translated(py, "Select faces or groups to process", None)?,
                4000,
            ),
        )?;
        return Ok(());
    }
    if faceted {
        let cmds = PyList::empty(py);
        for (_, mesh, keys) in &targets {
            cmds.append(api.call_method1("_plan_facets", (mesh, keys))?)?;
        }
        let result = api.call_method1("execute", (&history, compound(py, &cmds)?))?;
        if !result.get_item("ok")?.is_truthy()? {
            vp.call_method1("flash_status", (result.get_item("error")?.str()?, 6000))?;
        }
    } else {
        let title = translated(py, "Recover exact geometry", None)?;
        let done = py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(preview_recovery, py)?,
                win,
                &history,
                &scene,
                targets,
            ),
        )?;
        py.import("built_render.chrome_forms")?.call_method1(
            "ask_number",
            (
                win,
                title,
                translated(py, "Fit tolerance (mm)", None)?,
                0.1,
                0.000001,
                1e9,
                6,
                done,
            ),
        )?;
    }
    vp.call_method0("update")?;
    Ok(())
}

type RecoveryTargets<'p> = Vec<(String, Bound<'p, PyAny>, Bound<'p, PyList>)>;
fn selection_targets<'p>(scene: &Bound<'p, PyAny>) -> PyResult<RecoveryTargets<'p>> {
    let py = scene.py();
    let face = py.import("built_core.store_mesh")?.getattr("FaceView")?;
    let group = py.import("built_core.group")?.getattr("Group")?;
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut groups = Vec::new();
    let loose = PyList::empty(py);
    for item in scene.getattr("selection")?.try_iter()? {
        let item = item?;
        if item.is_instance(&face)? {
            loose.append(item.getattr("key")?)?;
        } else if item.is_instance(&group)? {
            groups.push(item);
        }
    }
    while let Some(g) = groups.pop() {
        let mesh = g.getattr("mesh")?;
        if seen.insert(mesh.as_ptr() as usize) {
            let keys = PyList::empty(py);
            for f in mesh.getattr("faces")?.try_iter()? {
                keys.append(f?.getattr("key")?)?;
            }
            if !keys.is_empty() {
                targets.push((g.getattr("name")?.extract()?, mesh, keys));
            }
        }
        for child in g.getattr("children")?.try_iter()? {
            groups.push(child?)
        }
    }
    let mesh = scene.getattr("mesh")?;
    if !loose.is_empty() && !seen.contains(&(mesh.as_ptr() as usize)) {
        targets.push((String::new(), mesh, loose));
    }
    Ok(targets)
}
fn compound<'p>(py: Python<'p>, commands: &Bound<'p, PyList>) -> PyResult<Bound<'p, PyAny>> {
    if commands.len() == 1 {
        return commands.get_item(0);
    }
    py.import("built_core.history")?
        .call_method1("CompoundCommand", (commands,))
}

#[pyfunction]
fn preview_recovery<'p>(
    win: &Bound<'p, PyAny>,
    history: &Bound<'p, PyAny>,
    scene: &Bound<'p, PyAny>,
    targets: RecoveryTargets<'p>,
    millimetres: Option<f64>,
) -> PyResult<()> {
    let Some(mm) = millimetres else { return Ok(()) };
    let py = win.py();
    let vp = win.getattr("viewport")?;
    if !vp.getattr("scene")?.is(scene) {
        return Ok(());
    }
    let api = py.import("built_core.command_protocol")?;
    let dialogs = py.import("built_render.chrome_dialogs")?;
    let title = translated(py, "Recover exact geometry", None)?;
    let mut detail = Vec::new();
    let cmds = PyList::empty(py);
    let revisions = PyList::empty(py);
    for (name, mesh, keys) in targets {
        let (report, cmd): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
            match api.call_method1("_plan_recovery", (&mesh, keys, mm / 1000.)) {
                Ok(r) => r.extract()?,
                Err(e) => {
                    dialogs.call_method1("information", (win, title, e.value(py).str()?))?;
                    return Ok(());
                }
            };
        if report.get_item("faces")?.is_truthy()? {
            cmds.append(cmd)?;
        }
        revisions.append((&mesh, mesh.getattr("_store")?.getattr("revision")?))?;
        if !name.is_empty() {
            detail.push(name);
        }
        for row in report.get_item("parts")?.try_iter()? {
            let row = row?;
            let vars = PyDict::new(py);
            for k in [
                "first_face",
                "method",
                "faces",
                "planar_faces",
                "curved_supports",
                "max_residual_m",
                "input_volume_m3",
                "output_volume_m3",
            ] {
                let v = row.get_item(k)?;
                vars.set_item(
                    k,
                    if v.is_none() {
                        translated(py, "Open selection", None)?
                    } else {
                        v
                    },
                )?;
            }
            detail.push(message(py,"Face {first_face} · {method} · {faces} faces\nPlanar faces: {planar_faces} · Curved supports: {curved_supports}\nMaximum residual: {max_residual_m} m\nInput volume (m³): {input_volume_m3}\nOutput volume (m³): {output_volume_m3}",&vars)?);
            detail.extend(row.get_item("untouched")?.extract::<Vec<String>>()?);
        }
    }
    if cmds.is_empty() {
        dialogs.call_method1("information", (win, title, detail.join("\n")))?;
        return Ok(());
    }
    let buttons = PyList::new(
        py,
        [
            ("cancel", translated(py, "Cancel", None)?),
            ("apply", translated(py, "Apply", None)?),
        ],
    )?;
    let kw = PyDict::new(py);
    kw.set_item("cancel", "cancel")?;
    kw.set_item("default", "cancel")?;
    let done = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(apply_recovery, py)?,
            win,
            history,
            scene,
            revisions,
            compound(py, &cmds)?,
        ),
    )?;
    dialogs.call_method(
        "ask_later",
        (win, title, detail.join("\n\n"), buttons, done),
        Some(&kw),
    )?;
    Ok(())
}

#[pyfunction]
fn apply_recovery(
    win: &Bound<'_, PyAny>,
    history: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    revisions: Vec<(Bound<'_, PyAny>, u64)>,
    command: &Bound<'_, PyAny>,
    answer: &str,
) -> PyResult<()> {
    if answer != "apply" {
        return Ok(());
    }
    let py = win.py();
    let vp = win.getattr("viewport")?;
    if !vp.getattr("scene")?.is(scene) {
        return Ok(());
    }
    for (mesh, revision) in revisions {
        if mesh
            .getattr("_store")?
            .getattr("revision")?
            .extract::<u64>()?
            != revision
        {
            vp.call_method1(
                "flash_status",
                (
                    translated(py, "Selection changed after recovery preview", None)?,
                    6000,
                ),
            )?;
            return Ok(());
        }
    }
    let result = py
        .import("built_core.command_protocol")?
        .call_method1("execute", (history, command))?;
    if !result.get_item("ok")?.is_truthy()? {
        vp.call_method1("flash_status", (result.get_item("error")?.str()?, 6000))?;
    }
    vp.call_method0("update")?;
    Ok(())
}
