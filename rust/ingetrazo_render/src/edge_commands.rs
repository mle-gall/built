//! Edge commands are bound to the document and edit mesh that opened the form.
use super::model_edit::{busy, matching, translated};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};

fn live_edges<'py>(
    mesh: &Bound<'py, PyAny>,
    selection: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let selected = matching(selection, "core.mesh", "Edge")?;
    if selected.is_empty() {
        return Ok(selected);
    }
    let live = mesh
        .py()
        .import("builtins")?
        .call_method1("set", (mesh.getattr("edges")?,))?
        .cast_into::<PySet>()?;
    let edges = PyList::empty(mesh.py());
    for edge in selected.iter() {
        if live.contains(&edge)? {
            edges.append(edge)?;
        }
    }
    Ok(edges)
}

fn status(win: &Bound<'_, PyAny>, text: &str, n: usize) -> PyResult<()> {
    let args = PyDict::new(win.py());
    args.set_item("n", n)?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (translated(win.py(), text, Some(&args))?, 3000),
    )?;
    Ok(())
}

#[pyfunction]
fn divide_selected(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let edges = live_edges(&mesh, &scene.getattr("selection")?)?;
    if edges.is_empty() {
        return Ok(());
    }
    let kw = PyDict::new(py);
    kw.set_item(
        "done",
        py.import("functools")?.call_method1(
            "partial",
            (wrap_pyfunction!(divide_answer, py)?, win, mesh, edges),
        )?,
    )?;
    py.import("ingetrazo_render.chrome_forms")?
        .getattr("ask_number")?
        .call(
            (
                win,
                translated(py, "Divide", None)?,
                translated(py, "Number of segments:", None)?,
                2,
                2,
                500,
            ),
            Some(&kw),
        )?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(win, mesh, edges, count=None))]
fn divide_answer(
    win: &Bound<'_, PyAny>,
    mesh: &Bound<'_, PyAny>,
    edges: &Bound<'_, PyList>,
    count: Option<f64>,
) -> PyResult<()> {
    let Some(n) = count else { return Ok(()) };
    if n.fract() != 0.0 || !(2.0..=500.0).contains(&n) {
        return Ok(());
    }
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if !scene.getattr("mesh")?.is(mesh) {
        return Ok(());
    }
    let keys = PyList::empty(win.py());
    for edge in live_edges(mesh, edges)?.iter() {
        keys.append(edge.getattr("key")?)?;
    }
    if keys.is_empty() {
        return Ok(());
    }
    let result = win
        .py()
        .import("core.protocol.commands")?
        .call_method1("divide_edges", (vp.getattr("history")?, keys, n as usize))?;
    if !result.get_item("ok")?.is_truthy()? {
        return Ok(());
    }
    scene.getattr("selection")?.call_method0("clear")?;
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    vp.call_method0("update")?;
    status(win, "Divided into {n} segments", n as usize)
}

#[pyfunction]
fn split_pieces(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let flash = |text| -> PyResult<()> {
        vp.call_method1("flash_status", (translated(py, text, None)?,))?;
        Ok(())
    };
    if !scene.getattr("edit_group")?.is_none() {
        return flash("Leave the group first (Esc) to split it into pieces");
    }
    let mut targets = Vec::new();
    for group in matching(&scene.getattr("selection")?, "core.group", "Group")?.iter() {
        if !group.getattr("billboard")?.is_truthy()? {
            targets.push(group);
        }
    }
    let [group] = targets.as_slice() else {
        return flash("Select one group or component to split into pieces");
    };
    let pieces = busy(win, &translated(py, "Split into Pieces", None)?, || {
        py.import("core.pieces")?
            .call_method1("split_into_pieces", (group,))
    })?;
    if !pieces.is_truthy()? {
        return flash("Nothing to split: it is already in its pieces");
    }
    let kw = PyDict::new(py);
    kw.set_item("n", pieces.len()?)?;
    let command = py
        .import("core.history")?
        .call_method1("SplitIntoPiecesCommand", (group, pieces))?;
    let history = vp.getattr("history")?;
    history.call_method1("execute", (command,))?;
    if history.getattr("last_error")?.is_truthy()?
        || history.getattr("last_rejected")?.is_truthy()?
    {
        return Ok(());
    }
    vp.call_method1(
        "flash_status",
        (
            translated(
                py,
                "Split into {n} pieces — Explode Group sets them free",
                Some(&kw),
            )?,
            5000,
        ),
    )?;
    vp.call_method0("update")?;
    Ok(())
}

const REASONS: [(&str, &str, &str); 5] = [
    ("join", "Taken away", "Points taken away"),
    ("kink", "Kink", "Kept: almost straight, but a real corner"),
    (
        "branch",
        "Another edge",
        "Kept: another edge meets the line here",
    ),
    (
        "flags",
        "Soft, hidden or layer",
        "Kept: the pieces differ in soft, hidden or layer",
    ),
    ("curve", "Curve", "Kept: part of a curve"),
];

fn placement<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let group = scene.getattr("edit_group")?;
    if group.is_none() {
        return Ok(None);
    }
    let iter = scene
        .py()
        .import("core.group")?
        .getattr("iter_placements")?;
    for root in scene.getattr("groups")?.try_iter()? {
        for pair in iter.call1((root?,))?.try_iter()? {
            let pair = pair?;
            if pair.get_item(0)?.is(&group) {
                return Ok(Some(pair.get_item(1)?));
            }
        }
    }
    Ok(group.getattr("xform")?.extract()?)
}

#[pyclass]
pub(crate) struct JoinMarks {
    points: Vec<(u8, [f64; 3])>,
}

#[pymethods]
impl JoinMarks {
    #[pyo3(signature=(matrix, width, height, origin=(0.0, 0.0)))]
    pub(crate) fn project(
        &self,
        matrix: [f64; 16],
        width: f64,
        height: f64,
        origin: (f64, f64),
    ) -> Vec<(u8, f32, f32)> {
        let mvp = glam::DMat4::from_cols_array(&matrix);
        self.points
            .iter()
            .filter_map(|(kind, point)| {
                let clip = mvp * glam::DVec3::from_array(*point).extend(1.0);
                if clip.w <= 0.0 || clip.x.abs() > clip.w || clip.y.abs() > clip.w {
                    return None;
                }
                let x = (clip.x / clip.w + 1.0) * width / 2.0;
                let y = (1.0 - clip.y / clip.w) * height / 2.0;
                Some((*kind, (x + origin.0) as f32, (y + origin.1) as f32))
            })
            .collect()
    }
}

#[pyfunction]
fn join_segments(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let selected = live_edges(&mesh, &scene.getattr("selection")?)?;
    let whole = selected.is_empty();
    let edges = if whole {
        py.import("builtins")?
            .call_method1("list", (mesh.getattr("edges")?,))?
    } else {
        selected.into_any()
    };
    let edits = py.import("core.edits")?;
    let verdicts = edits.call_method1("join_preview", (&mesh, &edges))?;
    if !verdicts.is_truthy()? {
        return status(win, "No collinear segments to join.", 0);
    }
    let forms = py.import("ingetrazo_render.chrome_forms")?;
    let matrix = placement(&scene)?;
    let mut points = Vec::new();
    let mut counts = [0usize; 5];
    for (v, reason) in verdicts.cast::<PyDict>()?.iter() {
        let reason: String = reason.extract()?;
        let i = REASONS
            .iter()
            .position(|r| r.0 == reason)
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("unknown join verdict"))?;
        let mut p = v.getattr("position")?;
        if let Some(m) = &matrix {
            p = m.call_method1("map", (p,))?;
        }
        points.push((i as u8, p.call_method0("toTuple")?.extract()?));
        counts[i] += 1;
    }
    points.sort_by_key(|p| p.0);
    let fields = PyList::empty(py);
    let field = forms.getattr("Field")?;
    for (i, (key, label, tip)) in REASONS.iter().enumerate() {
        if counts[i] == 0 {
            continue;
        }
        let kw = PyDict::new(py);
        kw.set_item("tip", translated(py, tip, None)?)?;
        let name = translated(py, label, None)?.extract::<String>()?;
        fields.append(field.call(
            (
                format!("join_reason_{key}"),
                "text",
                format!("{}  {name}", counts[i]),
            ),
            Some(&kw),
        )?)?;
    }
    let kw = PyDict::new(py);
    let vars = PyDict::new(py);
    vars.set_item("n", counts[0])?;
    let buttons = PyList::empty(py);
    buttons.append(("cancel", translated(py, "Cancel", None)?))?;
    if counts[0] > 0 {
        buttons.append(("ok", translated(py, "Join", None)?))?;
        kw.set_item(
            "note",
            translated(py, "{n} points will be taken away", Some(&vars))?,
        )?;
    } else {
        kw.set_item("note", translated(py, "No point can be taken away", None)?)?;
    }
    kw.set_item("key", "join_segments")?;
    kw.set_item("width", 288)?;
    kw.set_item("buttons", buttons)?;
    kw.set_item(
        "on_done",
        py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(join_answer, py)?,
                win,
                &mesh,
                scene.getattr("version")?,
                edges,
                whole,
                counts[0],
            ),
        )?,
    )?;
    let form = forms
        .getattr("Form")?
        .call((translated(py, "Join Segments", None)?, fields), Some(&kw))?;
    form.setattr("join_marks", Py::new(py, JoinMarks { points })?)?;
    forms.call_method1("show", (win, form))?;
    Ok(())
}

#[pyfunction]
fn join_answer(
    win: &Bound<'_, PyAny>,
    mesh: &Bound<'_, PyAny>,
    version: u64,
    edges: &Bound<'_, PyAny>,
    whole: bool,
    count: usize,
    answer: &str,
    _values: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if answer != "ok" || count == 0 {
        return Ok(());
    }
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if !scene.getattr("mesh")?.is(mesh) || scene.getattr("version")?.extract::<u64>()? != version {
        return Ok(());
    }
    let result = PyList::empty(py);
    let mutation = py.import("functools")?.call_method1(
        "partial",
        (wrap_pyfunction!(join_mutation, py)?, edges, &result),
    )?;
    let cmd = py
        .import("core.history")?
        .call_method1("SnapshotMutation", (mutation,))?;
    vp.getattr("history")?.call_method1("execute", (&cmd,))?;
    if cmd.getattr("after")?.is_none() {
        return Ok(());
    }
    if !whole {
        let sel = scene.getattr("selection")?;
        sel.call_method0("clear")?;
        sel.call_method1("update", (&result,))?;
    }
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    vp.call_method0("update")?;
    status(
        win,
        "Removed {n} intermediate points",
        edges.len()?.saturating_sub(result.len()),
    )
}
#[pyfunction]
fn join_mutation(
    edges: &Bound<'_, PyAny>,
    result: &Bound<'_, PyList>,
    scene: &Bound<'_, PyAny>,
) -> PyResult<()> {
    result.call_method1(
        "extend",
        (scene
            .py()
            .import("core.edits")?
            .call_method1("join_edges", (scene.getattr("mesh")?, edges))?,),
    )?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(divide_selected, m)?)?;
    m.add_function(wrap_pyfunction!(split_pieces, m)?)?;
    m.add_function(wrap_pyfunction!(join_segments, m)?)?;
    Ok(())
}
