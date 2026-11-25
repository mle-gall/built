//! Context actions on a placed reference image.
use super::model_edit::{matching, translated};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

fn bump(scene: &Bound<'_, PyAny>) -> PyResult<()> {
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)
}

#[pyfunction]
fn image_lock(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let images = matching(
        &scene.getattr("selection")?,
        "core.image_plane",
        "ImagePlane",
    )?;
    if !images.is_empty() {
        for image in images.iter() {
            image.setattr("locked", !image.getattr("locked")?.is_truthy()?)?;
        }
        bump(&scene)?;
        vp.call_method0("update")?;
    }
    Ok(())
}

#[pyfunction]
fn image_unlock(win: &Bound<'_, PyAny>, image: &Bound<'_, PyAny>) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    image.setattr("locked", false)?;
    scene.call_method1("select", (PyList::new(win.py(), [image])?,))?;
    bump(&scene)?;
    vp.call_method0("update")?;
    let vars = PyDict::new(win.py());
    vars.set_item("name", image.getattr("name")?)?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (
            translated(win.py(), "Image unlocked: {name}", Some(&vars))?,
            3000,
        ),
    )?;
    Ok(())
}

#[pyfunction]
fn image_delete(vp: &Bound<'_, PyAny>, image: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = vp.py();
    let cmd = py
        .import("core.history")?
        .call_method1("DeleteImagePlanesCommand", (PyList::new(py, [image])?,))?;
    vp.getattr("history")?.call_method1("execute", (cmd,))?;
    vp.getattr("scene")?
        .getattr("selection")?
        .call_method1("discard", (image,))?;
    vp.call_method0("update")?;
    Ok(())
}

#[pyfunction]
fn image_measure(win: &Bound<'_, PyAny>, width: bool) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let images = matching(
        &scene.getattr("selection")?,
        "core.image_plane",
        "ImagePlane",
    )?;
    if images.is_empty() {
        return Ok(());
    }
    let image = images.get_item(0)?;
    let (title, label, value, lo, hi, decimals, suffix) = if width {
        (
            "Image size",
            "Width (m):",
            image.call_method0("width")?.extract()?,
            0.0001,
            1_000_000.,
            4,
            "m",
        )
    } else {
        (
            "Image opacity",
            "Opacity (%):",
            (100. * image.getattr("opacity")?.extract::<f64>()?).round_ties_even(),
            5.,
            100.,
            0,
            "%",
        )
    };
    let kw = PyDict::new(py);
    kw.set_item("suffix", suffix)?;
    if !width {
        kw.set_item("step", 5)?;
    }
    kw.set_item(
        "done",
        py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(image_answer, py)?,
                win,
                &scene,
                image,
                width,
            ),
        )?,
    )?;
    py.import("ingetrazo_render.chrome_forms")?
        .getattr("ask_number")?
        .call(
            (
                win,
                translated(py, title, None)?,
                translated(py, label, None)?,
                value,
                lo,
                hi,
                decimals,
            ),
            Some(&kw),
        )?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(win,scene,image,width,answer=None))]
fn image_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    image: &Bound<'_, PyAny>,
    width: bool,
    answer: Option<f64>,
) -> PyResult<()> {
    let Some(value) = answer else { return Ok(()) };
    let vp = win.getattr("viewport")?;
    if !vp.getattr("scene")?.is(scene)
        || !scene.getattr("image_planes")?.contains(image)?
        || !value.is_finite()
    {
        return Ok(());
    }
    let py = win.py();
    let history = py.import("core.history")?;
    if width {
        if value <= 0. {
            return Ok(());
        }
        let (u, v): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
            image.call_method1("scaled", (value, true))?.extract()?;
        let kw = PyDict::new(py);
        kw.set_item("u", &u)?;
        kw.set_item("v", &v)?;
        let vars = PyDict::new(py);
        vars.set_item(
            "size",
            py.import("core.units")?.call_method1(
                "fmt_pair",
                (u.call_method0("length")?, v.call_method0("length")?),
            )?,
        )?;
        let cmd = history
            .getattr("TransformImagePlaneCommand")?
            .call((image,), Some(&kw))?;
        vp.getattr("history")?.call_method1("execute", (&cmd,))?;
        win.call_method0("statusBar")?.call_method1(
            "showMessage",
            (
                translated(py, "Image resized to {size}", Some(&vars))?,
                4000,
            ),
        )?;
    } else {
        let cmd = history.call_method1(
            "SetImagePlaneOpacityCommand",
            (image, value.clamp(5., 100.) / 100.),
        )?;
        vp.getattr("history")?.call_method1("execute", (cmd,))?;
    }
    vp.call_method0("update")?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(image_lock, m)?)?;
    m.add_function(wrap_pyfunction!(image_unlock, m)?)?;
    m.add_function(wrap_pyfunction!(image_delete, m)?)?;
    m.add_function(wrap_pyfunction!(image_measure, m)?)?;
    Ok(())
}
