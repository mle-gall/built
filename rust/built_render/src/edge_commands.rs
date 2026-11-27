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
    let selected = matching(selection, "built_core.mesh", "Edge")?;
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
    py.import("built_render.chrome_forms")?
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
        .import("built_core.command_protocol")?
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
    for group in matching(&scene.getattr("selection")?, "built_core.group", "Group")?.iter() {
        if !group.getattr("billboard")?.is_truthy()? {
            targets.push(group);
        }
    }
    let [group] = targets.as_slice() else {
        return flash("Select one group or component to split into pieces");
    };
    let pieces = busy(win, &translated(py, "Split into Pieces", None)?, || {
        py.import("built_core.pieces")?
            .call_method1("split_into_pieces", (group,))
    })?;
    if !pieces.is_truthy()? {
        return flash("Nothing to split: it is already in its pieces");
    }
    let kw = PyDict::new(py);
    kw.set_item("n", pieces.len()?)?;
    let command = py
        .import("built_core.history")?
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

pub(crate) fn placement<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let group = scene.getattr("edit_group")?;
    if group.is_none() {
        return Ok(None);
    }
    let iter = scene
        .py()
        .import("built_core.group")?
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

/// What Weld Edges does with the edges a selection holds (the .skp
/// command, with what Join Segments used to do folded in):
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WeldPlan {
    /// Nothing selected: the model's collinear pieces, after a preview.
    JoinModel,
    /// One connected chain that is straight end to end: one straight edge.
    Straighten,
    /// One connected chain with corners or bends: one curve.
    Curve,
    /// Straight pieces of several runs: each run becomes one edge.
    JoinRuns,
    /// Nothing Weld can do with these.
    Nothing,
}

/// `chain`: the selection is one connected edge chain (Store::weld_chain);
/// `joins`: points Join would take away. A chain of n edges is straight
/// when Join takes away all of its n - 1 inner points.
pub(crate) fn weld_plan(selected: usize, chain: bool, joins: usize) -> WeldPlan {
    match (selected, chain) {
        (0, _) => WeldPlan::JoinModel,
        (_, true) if joins > 0 && joins + 1 == selected => WeldPlan::Straighten,
        (_, true) => WeldPlan::Curve,
        _ if joins > 0 => WeldPlan::JoinRuns,
        _ => WeldPlan::Nothing,
    }
}

/// Weld Edges (Edit menu, right-click): a connected chain becomes one curve,
/// or one straight edge when the chain is straight; with several straight
/// runs selected each becomes one edge; with nothing selected, the model's
/// collinear pieces are joined after a preview. `join_segments` is kept as
/// an alias of this command for scripts.
#[pyfunction]
fn weld_edges(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let selection = scene.getattr("selection")?;
    if !selection.is_truthy()? {
        return join_segments(win);
    }
    let selected = live_edges(&mesh, &selection)?;
    if selected.is_empty() || selected.len() != selection.len()? {
        return status(win, "Select connected edges to weld.", 0);
    }
    let chain = crate::context_edits::can_weld(&scene)?;
    let verdicts = py
        .import("built_core.edits")?
        .call_method1("join_preview", (&mesh, &selected))?
        .cast_into::<PyDict>()?;
    let mut joins = 0;
    for (_, reason) in verdicts.iter() {
        if reason.extract::<String>()? == "join" {
            joins += 1;
        }
    }
    match weld_plan(selected.len(), chain, joins) {
        WeldPlan::Curve => {
            let keys = selection.call_method1("element_keys", ("edge",))?;
            let cmd = py
                .import("built_core.history")?
                .call_method1("WeldEdgesCommand", (keys,))?;
            vp.getattr("history")?.call_method1("execute", (&cmd,))?;
            vp.call_method0("update")?;
            Ok(())
        }
        WeldPlan::Straighten | WeldPlan::JoinRuns => join_answer(
            win, &mesh, scene.getattr("version")?.extract()?, selected.as_any(), false, joins, "ok",
            &py.None().into_bound(py),
        ),
        WeldPlan::JoinModel => join_segments(win),
        WeldPlan::Nothing => status(win, "Select connected edges to weld.", 0),
    }
}

/// Two of these edges continue one another in a straight line (a point
/// Weld would take away): they share an end and leave it in opposite
/// directions along one line.
pub(crate) fn straight_pair(ends: &[(glam::DVec3, glam::DVec3)]) -> bool {
    let tol = 1e-9;
    ends.iter().enumerate().any(|(i, &(a0, a1))| {
        ends[i + 1..].iter().any(|&(b0, b1)| {
            [(a0, a1, b0, b1), (a0, a1, b1, b0), (a1, a0, b0, b1), (a1, a0, b1, b0)]
                .into_iter()
                .any(|(p, u, q, v)| {
                    let (du, dv) = (u - p, v - q);
                    let scale = du.length() * dv.length();
                    (p - q).length() <= tol * (1. + p.length())
                        && scale > 0.
                        && du.cross(dv).length() <= 1e-9 * scale
                        && du.dot(dv) < 0.
                })
        })
    })
}

/// Weld Edges is offered for the selection: nothing selected (the model's
/// collinear pieces), one connected chain, or edges of which two line up.
/// Beyond 256 edges any edge-only selection (checking would cost a frame).
pub fn weld_available(scene: &Bound<'_, PyAny>) -> PyResult<bool> {
    let selection = scene.getattr("selection")?;
    if !selection.is_truthy()? {
        return Ok(true);
    }
    Ok(weld_offered(scene)?)
}

/// Weld Edges for a non-empty selection of edges only (the context menu).
pub(crate) fn weld_offered(scene: &Bound<'_, PyAny>) -> PyResult<bool> {
    let selection = scene.getattr("selection")?;
    let n = selection.len()?;
    let snap = selection.call_method0("snapshot")?;
    if n < 2 || snap.get_item(3)?.extract::<usize>()? != n {
        return Ok(false);
    }
    if crate::context_edits::can_weld(scene)? || n > 256 {
        return Ok(true);
    }
    let store = scene.getattr("mesh")?.getattr("_store")?;
    let stamp = (snap.get_item(6)?.extract::<u64>()?, store.getattr("revision")?.extract::<u64>()?, store.as_ptr() as usize);
    thread_local! { static LAST: std::cell::Cell<Option<((u64, u64, usize), bool)>> = const { std::cell::Cell::new(None) }; }
    if let Some((_, offered)) = LAST.get().filter(|(prior, _)| *prior == stamp) {
        return Ok(offered);
    }
    use crate::document_host::xyz;
    let mut ends = Vec::with_capacity(n);
    for edge in matching(&selection, "built_core.mesh", "Edge")?.iter() {
        ends.push((xyz(&edge.getattr("a")?)?, xyz(&edge.getattr("b")?)?));
    }
    let offered = straight_pair(&ends);
    LAST.set(Some((stamp, offered)));
    Ok(offered)
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
    let edits = py.import("built_core.edits")?;
    let verdicts = edits.call_method1("join_preview", (&mesh, &edges))?;
    if !verdicts.is_truthy()? {
        return status(win, "No collinear segments to join.", 0);
    }
    let forms = py.import("built_render.chrome_forms")?;
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
        buttons.append(("ok", translated(py, "Weld", None)?))?;
        kw.set_item(
            "note",
            translated(py, "{n} points will be taken away", Some(&vars))?,
        )?;
    } else {
        kw.set_item("note", translated(py, "No point can be taken away", None)?)?;
    }
    kw.set_item("key", "weld_edges")?;
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
        .call((translated(py, "Weld Edges", None)?, fields), Some(&kw))?;
    form.setattr("join_marks", Py::new(py, JoinMarks { points })?)?;
    form.setattr("placement", "drawer")?;
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
        .import("built_core.history")?
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
            .import("built_core.edits")?
            .call_method1("join_edges", (scene.getattr("mesh")?, edges))?,),
    )?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(divide_selected, m)?)?;
    m.add_function(wrap_pyfunction!(split_pieces, m)?)?;
    m.add_function(wrap_pyfunction!(join_segments, m)?)?;
    m.add_function(wrap_pyfunction!(weld_edges, m)?)?;
    Ok(())
}

#[cfg(test)]
mod weld_tests {
    use super::{weld_plan, WeldPlan::*};

    #[test]
    fn weld_takes_over_what_join_did() {
        // Nothing selected: the model's collinear pieces, after a preview.
        assert_eq!(weld_plan(0, false, 0), JoinModel);
        // A straight chain becomes one straight edge, not a curve.
        assert_eq!(weld_plan(4, true, 3), Straighten);
        // A chain with a corner or a bend is one curve, as .skp welds it,
        // even where some of its pieces line up.
        assert_eq!(weld_plan(4, true, 2), Curve);
        assert_eq!(weld_plan(3, true, 0), Curve);
        // Several straight runs: each one joined.
        assert_eq!(weld_plan(6, false, 4), JoinRuns);
        assert_eq!(weld_plan(2, false, 0), Nothing);
    }

    #[test]
    fn weld_is_offered_where_two_edges_line_up() {
        use glam::DVec3 as V;
        let x = |a: f64, b: f64| (V::new(a, 0., 0.), V::new(b, 0., 0.));
        assert!(super::straight_pair(&[x(0., 1.), x(2., 1.)]));
        assert!(super::straight_pair(&[x(1., 2.), x(0., 1.)]));
        // Opposite sides of a square, a corner, or a fold back: no.
        assert!(!super::straight_pair(&[x(0., 1.), (V::new(0., 1., 0.), V::new(1., 1., 0.))]));
        assert!(!super::straight_pair(&[x(0., 1.), (V::new(1., 0., 0.), V::new(1., 1., 0.))]));
        assert!(!super::straight_pair(&[x(0., 1.), x(1., 0.5)]));
    }
}
