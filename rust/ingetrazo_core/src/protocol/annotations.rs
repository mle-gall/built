// SPDX-License-Identifier: GPL-3.0-or-later
//! Prepare annotation, guide and geographic steps in the active document frame.
use super::*;
use pyo3::types::PyTuple;

pub(super) fn vec3<'py>(p: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = p.py();
    let args = py
        .get_type::<PyTuple>()
        .call1((p,))?
        .cast_into::<PyTuple>()?;
    py.import("ingetrazo_core")?
        .getattr("Vec3")?
        .call(args, None)
}
fn attachment_pair<'py>(v: &Obj<'py>) -> PyResult<(Option<Obj<'py>>, Option<Obj<'py>>)> {
    let mut it = v.try_iter()?;
    let a = it
        .next()
        .transpose()?
        .ok_or_else(|| PyValueError::new_err("expected two dimension attachments"))?;
    let b = it
        .next()
        .transpose()?
        .ok_or_else(|| PyValueError::new_err("expected two dimension attachments"))?;
    if it.next().transpose()?.is_some() {
        return Err(PyValueError::new_err("expected two dimension attachments"));
    }
    Ok((Some(a), Some(b)))
}
fn annotation<'py>(h: &Obj<'py>, kind: &str, object: Obj<'py>) -> Reply<'py> {
    execute(h, &history_command(h.py(), kind, (object,))?)
}
#[pyfunction]
#[pyo3(signature=(history,points,closed=false))]
fn add_geo_path<'py>(
    history: &Obj<'py>,
    points: &Obj<'py>,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] closed: bool,
) -> Reply<'py> {
    if points.len()? < 2 {
        return Err(PyValueError::new_err("expected at least two path nodes"));
    }
    let py = history.py();
    let nodes = PyList::empty(py);
    for p in points.try_iter()? {
        nodes.append(vec3(&p?)?)?;
    }
    let kw = PyDict::new(py);
    kw.set_item("closed", closed)?;
    annotation(
        history,
        "AddGeoPathCommand",
        py.import("georef.geopath")?
            .call_method("GeoPath", (nodes,), Some(&kw))?,
    )
}
#[pyfunction]
fn move_geo_path_node<'py>(
    history: &Obj<'py>,
    path: isize,
    node: isize,
    point: &Obj<'py>,
) -> Reply<'py> {
    let paths = history.getattr("scene")?.getattr("geo_paths")?;
    if path < 0 || path as usize >= paths.len()? {
        return Err(PyKeyError::new_err("unknown path node"));
    }
    let path = paths.get_item(path)?;
    if node < 0 || node as usize >= path.getattr("points")?.len()? {
        return Err(PyKeyError::new_err("unknown path node"));
    }
    execute(
        history,
        &history_command(
            history.py(),
            "MoveGeoPathNodeCommand",
            (path, node, vec3(point)?),
        )?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,anchor,offset,text,attachment=None))]
fn add_text_label<'py>(
    history: &Obj<'py>,
    anchor: &Obj<'py>,
    offset: &Obj<'py>,
    text: &Obj<'py>,
    attachment: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let point = vec3(anchor)?;
    let kw = PyDict::new(py);
    let attach = py
        .import("ingetrazo_core.annotation_anchor")?
        .getattr("Attachment")?
        .call_method1("restore", (history.getattr("scene")?, attachment, &point))?;
    kw.set_item("attachment", attach)?;
    annotation(
        history,
        "AddTextLabelCommand",
        py.import("ingetrazo_core.textlabel")?.call_method(
            "TextLabel",
            (point, vec3(offset)?, text),
            Some(&kw),
        )?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,a,b,offset,axis=None,attachments=(None,None)))]
fn add_dimension<'py>(
    history: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    offset: &Obj<'py>,
    axis: Option<&Obj<'py>>,
    #[pyo3(from_py_with=attachment_pair)] attachments: (Option<Obj<'py>>, Option<Obj<'py>>),
) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let kw = PyDict::new(py);
    kw.set_item("axis", axis)?;
    let dim = py.import("core.dimension")?.call_method(
        "Dimension",
        (vec3(a)?, vec3(b)?, vec3(offset)?),
        Some(&kw),
    )?;
    let factory = py.import("ingetrazo_core.annotation_anchor")?.getattr("Attachment")?;
    for (key, raw, point) in [
        ("attachment_a", attachments.0, "a"),
        ("attachment_b", attachments.1, "b"),
    ] {
        dim.setattr(
            key,
            factory.call_method1("restore", (&scene, raw, dim.getattr(point)?))?,
        )?;
    }
    annotation(history, "AddDimensionCommand", dim)
}
#[pyfunction]
#[pyo3(signature=(history,point,normal,name=None,symbol=None))]
fn place_section_plane<'py>(
    history: &Obj<'py>,
    point: &Obj<'py>,
    normal: &Obj<'py>,
    name: Option<&Obj<'py>>,
    symbol: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let planes = history.getattr("scene")?.getattr("section_planes")?;
    let kw = PyDict::new(py);
    let name = if let Some(n) = name {
        n.clone()
    } else {
        let args = PyDict::new(py);
        args.set_item("n", planes.len()? + 1)?;
        py.import("core.i18n")?
            .call_method("tr", ("Section {n}",), Some(&args))?
    };
    kw.set_item("name", name)?;
    kw.set_item(
        "symbol",
        match symbol {
            Some(s) => s.clone(),
            None => py
                .import("ingetrazo_core")?
                .call_method1("section_next_symbol", (planes,))?,
        },
    )?;
    annotation(
        history,
        "PlaceSectionPlaneCommand",
        py.import("core.section")?.call_method(
            "SectionPlane",
            (vec3(point)?, vec3(normal)?),
            Some(&kw),
        )?,
    )
}
#[pyfunction]
fn set_north<'py>(history: &Obj<'py>, degrees: f64) -> Reply<'py> {
    if !degrees.is_finite() {
        return Err(PyValueError::new_err("expected a finite north angle"));
    }
    let sun = history.py().import("ingetrazo_core.sun")?;
    let deg = sun
        .call_method1("wrap_north_deg", (degrees,))?
        .extract::<f64>()?;
    let before = history.getattr("revision")?;
    let current = sun
        .call_method1("scene_north_deg", (history.getattr("scene")?,))?
        .extract::<f64>()?;
    if (deg - current).abs() > 1e-9 {
        history.call_method1(
            "execute",
            (history_command(history.py(), "SetNorthCommand", (deg,))?,),
        )?;
    }
    answer(history, &before, true)
}
#[pyfunction]
#[pyo3(signature=(history,path,origin,u,v,aspect=1.0,name=""))]
fn add_image_plane<'py>(
    history: &Obj<'py>,
    path: &Obj<'py>,
    origin: &Obj<'py>,
    u: &Obj<'py>,
    v: &Obj<'py>,
    aspect: f64,
    name: &str,
) -> Reply<'py> {
    let py = history.py();
    let before = history.getattr("revision")?;
    let kw = PyDict::new(py);
    kw.set_item("aspect", aspect)?;
    kw.set_item("name", name)?;
    let plane = py.import("ingetrazo_core.image_plane")?.call_method(
        "ImagePlane",
        (path, vec3(origin)?, vec3(u)?, vec3(v)?),
        Some(&kw),
    )?;
    history.call_method1(
        "execute",
        (history_command(py, "AddImagePlaneCommand", (plane,))?,),
    )?;
    answer(history, &before, true)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(add_geo_path, m)?)?;
    m.add_function(wrap_pyfunction!(move_geo_path_node, m)?)?;
    m.add_function(wrap_pyfunction!(add_text_label, m)?)?;
    m.add_function(wrap_pyfunction!(add_dimension, m)?)?;
    m.add_function(wrap_pyfunction!(place_section_plane, m)?)?;
    m.add_function(wrap_pyfunction!(set_north, m)?)?;
    m.add_function(wrap_pyfunction!(add_image_plane, m)?)?;
    Ok(())
}
