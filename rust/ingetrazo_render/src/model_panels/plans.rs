// SPDX-License-Identifier: GPL-3.0-or-later
use crate::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
type Point = [f64; 3];

fn unique<'p>(base: &Obj<'p>, collection: &Obj<'p>) -> PyResult<Obj<'p>> {
    let names = PyList::empty(base.py());
    for entry in collection.try_iter()? {
        names.append(entry?.getattr("name")?)?;
    }
    base.py()
        .import("core.outliner")?
        .call_method1("unique_name", (base, names))
}

fn point(v: &Obj<'_>) -> PyResult<Point> {
    v.call_method0("toTuple")?.extract()
}

#[pyfunction]
fn finish(
    win: &Obj<'_>,
    vp: &Obj<'_>,
    target: &Obj<'_>,
    group: bool,
    answer: &str,
    values: &Obj<'_>,
) -> PyResult<()> {
    if !win.getattr("viewport")?.is(vp) || answer != "ok" {
        return Ok(());
    }
    let scene = vp.getattr("scene")?;
    if scene
        .getattr_opt("worksession_read_only")?
        .is_some_and(|s| s.is_truthy().unwrap_or(false))
    {
        return Ok(());
    }
    let py = vp.py();
    let vector = crate::host_context::core_module(py)?.getattr("Vec3")?;
    let vec = |p: Point| vector.call1((p[0], p[1], p[2]));
    let title_args = PyDict::new(py);
    let sections = scene.getattr("section_planes")?;
    let cam = py.import("core.camera")?.call_method0("OrbitCamera")?;
    let mut outside = false;
    let (base, plane, centre, span) = if group {
        if !scene
            .call_method1("occurrence_path", (target,))?
            .is_truthy()?
        {
            return Ok(());
        }
        let bounds = py
            .import("core.outliner")?
            .call_method1("world_bounds", (scene.getattr("groups")?, target))?;
        let Some((lo, hi)) = bounds.extract::<Option<(Point, Point)>>()? else {
            vp.call_method1(
                "flash_status",
                (translated(py, "No geometry to make a plan of", None)?, 3500),
            )?;
            return Ok(());
        };
        let centre = [
            (lo[0] + hi[0]) / 2.,
            (lo[1] + hi[1]) / 2.,
            lo[2] + values.get_item("height")?.extract::<f64>()?,
        ];
        outside = centre[2] < lo[2] || centre[2] > hi[2];
        title_args.set_item("name", target.getattr("name")?)?;
        let title = unique(
            &translated(py, "Plan {name}", Some(&title_args))?,
            &scene.getattr("saved_views")?,
        )?;
        let module = py.import("core.section")?;
        let kw = PyDict::new(py);
        kw.set_item("name", &title)?;
        kw.set_item("symbol", crate::host_context::core_module(module.py())?.call_method1("section_next_symbol", (&sections,))?)?;
        let plane = module
            .getattr("SectionPlane")?
            .call((vec(centre)?, vec([0., 0., 1.])?), Some(&kw))?;
        cam.call_method1("set_view", ("top",))?;
        (
            title,
            plane,
            centre,
            (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.),
        )
    } else {
        if !sections.contains(target)? {
            return Ok(());
        }
        let p = point(&target.getattr("point")?)?;
        let n = point(&target.getattr("normal")?.call_method0("normalized")?)?;
        let bounds = scene.call_method0("bounds")?;
        let (mut centre, span) = if bounds.get_item(0)?.is_none() {
            (p, 10.)
        } else {
            let lo = point(&bounds.get_item(0)?)?;
            let hi = point(&bounds.get_item(1)?)?;
            (
                std::array::from_fn(|i| (lo[i] + hi[i]) / 2.),
                (0..3).map(|i| hi[i] - lo[i]).fold(1., f64::max),
            )
        };
        let distance: f64 = (0..3).map(|i| n[i] * (centre[i] - p[i])).sum();
        for i in 0..3 {
            centre[i] -= distance * n[i];
        }
        cam.setattr("yaw", n[1].atan2(n[0]))?;
        cam.setattr("pitch", n[2].clamp(-1., 1.).asin())?;
        let mut name = target.getattr("name")?;
        if !name.is_truthy()? {
            title_args.set_item("symbol", target.getattr("symbol")?)?;
            name = translated(py, "Section {symbol}", Some(&title_args))?;
        }
        (
            unique(&name, &scene.getattr("saved_views")?)?,
            target.clone(),
            centre,
            span,
        )
    };
    cam.setattr("target", vec(centre)?)?;
    cam.setattr("perspective", false)?;
    let fov = cam.getattr("fov_deg")?.extract::<f64>()?.to_radians();
    cam.setattr("distance", span * 1.2 / (2. * (fov / 2.).tan()))?;
    let view = py
        .import("core.saved_views")?
        .getattr("SavedView")?
        .call_method1("capture", (&base, &scene, cam))?;
    let state = PyDict::new(py);
    state.set_item("active", plane.getattr("uid")?)?;
    state.set_item("planes_shown", false)?;
    state.set_item("cuts_shown", true)?;
    view.setattr("section", state)?;
    let page = if values.get_item("sheet")?.is_truthy()? {
        let kwargs = PyDict::new(py);
        kwargs.set_item("name", unique(&base, &scene.getattr("compositions")?)?)?;
        kwargs.set_item("paper", "A3")?;
        let page = py
            .import("core.composition")?
            .getattr("Composicion")?
            .call((), Some(&kwargs))?;
        let frame = page.call_method0("default_frame")?;
        frame.setattr("view_key", format!("scene:{}", base.str()?))?;
        frame.setattr("scale_n", values.get_item("scale")?)?;
        frame.setattr("show_title", true)?;
        page.getattr("frames")?.call_method1("append", (frame,))?;
        page
    } else {
        py.None().into_bound(py)
    };
    vp.call_method0("end_group_edit")?;
    let kwargs = [("add_plane", group)].into_py_dict(py)?;
    let command = py
        .import("core.history")?
        .getattr("CreatePlanCommand")?
        .call((plane, view, &page), Some(&kwargs))?;
    vp.getattr("history")?.call_method1("execute", (command,))?;
    vp.call_method0("notify_scene_changed")?;
    if outside {
        vp.call_method1(
            "flash_status",
            (
                translated(
                    py,
                    "The cut is outside «{name}»: the plan shows it whole",
                    Some(&title_args),
                )?,
                4000,
            ),
        )?;
    } else {
        vp.call_method1("flash_status", (base, 4000))?;
    }
    if !page.is_none() && values.get_item("open_sheet")?.is_truthy()? {
        let index = scene
            .getattr("compositions")?
            .call_method1("index", (&page,))?;
        py.import("ingetrazo_render.sheet_workspace")?
            .call_method1("to_sheets", (win, index))?;
    }
    Ok(())
}

#[pyfunction]
fn with_sheet(form: &Obj<'_>) -> PyResult<bool> {
    form.call_method1("value", ("sheet",))?.is_truthy()
}

#[pyfunction]
fn validate<'p>(form: &Obj<'p>, _answer: &str) -> PyResult<Option<Obj<'p>>> {
    let texts = form.getattr("_texts")?;
    for key in ["height", "scale"] {
        if key == "scale" && !with_sheet(form)? {
            continue;
        }
        let Some(raw) = texts.cast::<PyDict>()?.get_item(key)? else {
            continue;
        };
        let value = raw
            .str()?
            .to_string()
            .trim()
            .replace(',', ".")
            .parse::<f64>()
            .ok();
        if !value
            .is_some_and(|v| v.is_finite() && (key != "scale" || (1.0..=1_000_000.).contains(&v)))
        {
            return Ok(Some(translated(
                form.py(),
                if key == "scale" {
                    "Enter a positive scale denominator."
                } else {
                    "Enter a finite cut height."
                },
                None,
            )?));
        }
    }
    Ok(None)
}

#[pyfunction]
pub(super) fn plan_options(win: &Obj<'_>, target: &Obj<'_>, group: bool) -> PyResult<()> {
    if target.is_none() {
        return Ok(());
    }
    let py = win.py();
    let forms = py.import("ingetrazo_render.chrome_forms")?;
    let fields = PyList::empty(py);
    let field = forms.getattr("Field")?;
    if group {
        let kw = [("decimals", 3)].into_py_dict(py)?;
        fields.append(field.call(
            (
                "height",
                "number",
                translated(py, "Cut height (m)", None)?,
                1.2,
            ),
            Some(&kw),
        )?)?;
    }
    fields.append(field.call1((
        "sheet",
        "check",
        translated(py, "Also place it on a new sheet", None)?,
        true,
    ))?)?;
    let kw = [("lo", 1), ("hi", 1_000_000), ("decimals", 3)].into_py_dict(py)?;
    kw.set_item("enabled", wrap_pyfunction!(with_sheet, py)?)?;
    fields.append(field.call(
        (
            "scale",
            "number",
            translated(py, "Scale denominator", None)?,
            if group { 100 } else { 50 },
        ),
        Some(&kw),
    )?)?;
    let kw = PyDict::new(py);
    kw.set_item("enabled", wrap_pyfunction!(with_sheet, py)?)?;
    fields.append(field.call(
        (
            "open_sheet",
            "check",
            translated(py, "Open the sheet when done", None)?,
            true,
        ),
        Some(&kw),
    )?)?;
    let opts = PyDict::new(py);
    opts.set_item("validate", wrap_pyfunction!(validate, py)?)?;
    opts.set_item(
        "on_done",
        py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(finish, py)?,
                win,
                win.getattr("viewport")?,
                target,
                group,
            ),
        )?,
    )?;
    let form = forms.getattr("Form")?.call(
        (
            translated(
                py,
                if group {
                    "Plan of a group"
                } else {
                    "View of the cut"
                },
                None,
            )?,
            fields,
        ),
        Some(&opts),
    )?;
    forms.call_method1("show", (win, form))?;
    Ok(())
}
