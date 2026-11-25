// SPDX-License-Identifier: GPL-3.0-or-later
//! A saved view's capture and recall (`built_core.saved_views`): the camera and
//! the presentation state a scene remembers (tags and folders, style,
//! section, geographic layers, shadows, hidden objects, the hidden-things
//! switches), read from and put back onto the live scene and camera, and
//! the state a recall can change as one value to compare (`shown_state`,
//! `visibility_sig`). The scene, its style, shadows and display objects
//! stay Python objects: they are read and written as the Python did,
//! attribute for attribute and in the same order
//! (`tests/test_saved_view_state_parity.py`).

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFrozenSet, PyList, PySet, PyTuple};

fn attr_or<'py>(obj: &Bound<'py, PyAny>, name: &str, default: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    Ok(obj.getattr_opt(name)?.unwrap_or(default))
}

fn none(py: Python<'_>) -> Bound<'_, PyAny> {
    py.None().into_bound(py)
}

fn pybool(py: Python<'_>, v: bool) -> Bound<'_, PyAny> {
    pyo3::types::PyBool::new(py, v).to_owned().into_any()
}

fn truthy_attr(obj: &Bound<'_, PyAny>, name: &str, default: bool) -> PyResult<bool> {
    match obj.getattr_opt(name)? {
        Some(v) => v.is_truthy(),
        None => Ok(default),
    }
}

fn float<'py>(v: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    v.py().get_type::<pyo3::types::PyFloat>().call1((v,))
}

fn target_tuple<'py>(t: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(t.py(), [t.call_method0("x")?, t.call_method0("y")?, t.call_method0("z")?])
}

/// `georef_objects(scene)`: `[("map", tile_layer), ("terrain", terrain),
/// ("survey", photo_mesh)]`, None where the scene has none.
#[pyfunction]
fn saved_view_georef_objects<'py>(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let rows = [("map", "tile_layer"), ("terrain", "terrain"), ("survey", "photo_mesh")]
        .into_iter()
        .map(|(k, a)| Ok(PyTuple::new(py, [k.into_pyobject(py)?.into_any(), attr_or(scene, a, none(py))?])?))
        .collect::<PyResult<Vec<_>>>()?;
    PyList::new(py, rows)
}

/// `georef_state(scene)`: each geographic layer's visibility, a missing
/// one hidden.
#[pyfunction]
fn saved_view_georef_state<'py>(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for row in saved_view_georef_objects(py, scene)?.iter() {
        let (key, obj): (Bound<'py, PyAny>, Bound<'py, PyAny>) = row.extract()?;
        let on = !obj.is_none() && truthy_attr(&obj, "visible", false)?;
        out.set_item(key, on)?;
    }
    Ok(out)
}

fn active_uid<'py>(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let sec = scene.call_method0("active_section")?;
    if sec.is_truthy()? { sec.getattr("uid") } else { Ok(none(py)) }
}

fn hidden_uids<'py>(scene: &Bound<'py, PyAny>, default_hidden: bool) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut out = Vec::new();
    for item in scene.call_method0("groups_by_uid")?.call_method0("items")?.try_iter()? {
        let (uid, g): (Bound<'py, PyAny>, Bound<'py, PyAny>) = item?.extract()?;
        let hidden = if default_hidden { truthy_attr(&g, "hidden", false)? } else { g.getattr("hidden")?.is_truthy()? };
        if hidden {
            out.push(uid);
        }
    }
    Ok(out)
}

/// `SavedView.capture(name, scene, camera, style=None)` for the class
/// `cls`: the live camera and presentation state as a new view.
#[pyfunction]
#[pyo3(signature = (cls, name, scene, camera, style=None))]
fn saved_view_capture<'py>(py: Python<'py>, cls: &Bound<'py, PyAny>, name: &Bound<'py, PyAny>, scene: &Bound<'py, PyAny>,
                           camera: &Bound<'py, PyAny>, style: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    let style = match style.filter(|s| !s.is_none()) {
        Some(s) => s,
        None => attr_or(scene, "display_style", none(py))?,
    };
    let kw = PyDict::new(py);
    kw.set_item("navigator_view", attr_or(scene, "_navigator_view_key", none(py))?)?;
    kw.set_item("target", target_tuple(&camera.getattr("target")?)?)?;
    for (k, a) in [("distance", "distance"), ("yaw", "yaw"), ("pitch", "pitch"), ("fov_deg", "fov_deg"),
                   ("perspective", "perspective")] {
        kw.set_item(k, camera.getattr(a)?)?;
    }
    kw.set_item("two_point", truthy_attr(camera, "two_point", false)?)?;
    kw.set_item("lens", if camera.hasattr("lens_dict")? { camera.call_method0("lens_dict")? } else { none(py) })?;
    let hidden_layers = PyList::empty(py);
    for ly in scene.getattr("layers")?.try_iter()? {
        let ly = ly?;
        if !ly.getattr("visible")?.is_truthy()? {
            hidden_layers.append(ly.getattr("key")?)?;
        }
    }
    kw.set_item("hidden_layers", hidden_layers)?;
    let hidden_folders = PyList::empty(py);
    for item in attr_or(scene, "folder_visibility", PyDict::new(py).into_any())?.call_method0("items")?.try_iter()? {
        let (f, on): (Bound<'py, PyAny>, Bound<'py, PyAny>) = item?.extract()?;
        if !on.is_truthy()? {
            hidden_folders.append(f)?;
        }
    }
    kw.set_item("hidden_folders", hidden_folders)?;
    kw.set_item("style", if style.is_truthy()? { style.call_method0("to_dict")? } else { none(py) })?;
    let section = PyDict::new(py);
    section.set_item("active", active_uid(py, scene)?)?;
    section.set_item("planes_shown", attr_or(scene, "show_section_planes", pybool(py, true))?)?;
    section.set_item("cuts_shown", attr_or(scene, "show_section_cuts", pybool(py, true))?)?;
    kw.set_item("section", section)?;
    kw.set_item("georef", saved_view_georef_state(py, scene)?)?;
    let shadows = attr_or(scene, "shadows", none(py))?;
    kw.set_item("shadows", if shadows.is_none() { none(py) } else { scene.getattr("shadows")?.call_method0("to_dict")? })?;
    kw.set_item("hidden_objects", PyList::new(py, hidden_uids(scene, false)?)?)?;
    let shown = PyDict::new(py);
    shown.set_item("objects", truthy_attr(scene, "show_hidden_objects", false)?)?;
    shown.set_item("geometry", truthy_attr(scene, "show_hidden_geometry", false)?)?;
    kw.set_item("hidden_shown", shown)?;
    cls.call((name,), Some(&kw))
}

/// `SavedView.take_camera(camera)`: only the camera (and its lens).
#[pyfunction]
fn saved_view_take_camera(view: &Bound<'_, PyAny>, camera: &Bound<'_, PyAny>) -> PyResult<()> {
    view.setattr("target", target_tuple(&camera.getattr("target")?)?)?;
    for a in ["distance", "yaw", "pitch", "fov_deg"] {
        view.setattr(a, float(&camera.getattr(a)?)?)?;
    }
    view.setattr("perspective", camera.getattr("perspective")?.is_truthy()?)?;
    view.setattr("two_point", truthy_attr(camera, "two_point", false)?)?;
    if camera.hasattr("lens_dict")? {
        view.setattr("lens", camera.call_method0("lens_dict")?)?;
    }
    Ok(())
}

/// `SavedView.recapture(scene, camera, style=None)`: the view taken again
/// in place, its name and folder kept.
#[pyfunction]
#[pyo3(signature = (view, scene, camera, style=None))]
fn saved_view_recapture(py: Python<'_>, view: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>, camera: &Bound<'_, PyAny>,
                        style: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let cls = super::module(py, "built_core.saved_views")?.getattr("SavedView")?;
    let fresh = saved_view_capture(py, &cls, &view.getattr("name")?, scene, camera, style)?;
    fresh.setattr("folder", view.getattr("folder")?)?;
    for field in view.getattr("_record_fields")?.try_iter()? {
        let field = field?;
        view.setattr(field.cast::<pyo3::types::PyString>()?, fresh.getattr(field.cast::<pyo3::types::PyString>()?)?)?;
    }
    Ok(())
}

/// `SavedView.apply_camera(camera)`: the camera, lens included.
#[pyfunction]
fn saved_view_apply_camera(py: Python<'_>, view: &Bound<'_, PyAny>, camera: &Bound<'_, PyAny>) -> PyResult<()> {
    let point = super::module(py, "built_core")?.getattr("Vec3")?;
    camera.setattr("target", point.call1((view.getattr("target")?,))?)?;
    for a in ["distance", "yaw", "pitch", "fov_deg", "perspective", "two_point"] {
        camera.setattr(a, view.getattr(a)?)?;
    }
    if camera.hasattr("apply_lens")? {
        camera.call_method1("apply_lens", (view.getattr("lens")?,))?;
    }
    Ok(())
}

/// `apply_shadow_state(scene, raw)`: a shadow snapshot put back onto
/// `scene.shadows` in place (the panel, menu and viewport hold that one).
#[pyfunction]
fn saved_view_apply_shadows(py: Python<'_>, scene: &Bound<'_, PyAny>, raw: &Bound<'_, PyAny>) -> PyResult<()> {
    let fresh = super::module(py, "built_core.sun")?.getattr("ShadowSettings")?.call_method1("from_dict", (raw,))?;
    scene.getattr("shadows")?.getattr("__dict__")?.call_method1("update", (fresh.getattr("__dict__")?,))?;
    Ok(())
}

/// `SavedView.apply_state(scene, put_style=None)`: everything but the
/// camera; each part only when the view remembers it.
#[pyfunction]
#[pyo3(signature = (view, scene, put_style=None))]
fn saved_view_apply_state(py: Python<'_>, view: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>,
                          put_style: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let hidden = PySet::new(py, view.getattr("hidden_layers")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
    for ly in scene.getattr("layers")?.try_iter()? {
        let ly = ly?;
        ly.setattr("visible", !hidden.contains(ly.getattr("key")?)?)?;
    }
    let folders = view.getattr("hidden_folders")?;
    if !folders.is_none() {
        let off = PySet::new(py, folders.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        let all = super::module(py, "core.layers")?.getattr("folders_of")?.call1((scene,))?;
        let vis = PyDict::new(py);
        for f in all.try_iter()? {
            let f = f?;
            let on = !off.contains(&f)?;
            vis.set_item(f, on)?;
        }
        scene.setattr("folder_visibility", vis)?;
    }
    let style = view.getattr("style")?;
    if style.is_truthy()? {
        let style = super::module(py, "built_core.style")?.getattr("Style")?.call_method1("from_dict", (style,))?;
        match put_style.filter(|p| !p.is_none()) {
            Some(put) => {
                put.call1((style,))?;
            }
            None => scene.setattr("display_style", style)?,
        }
    }
    let section = view.getattr("section")?;
    if !section.is_none() {
        let uid = section.call_method1("get", ("active",))?;
        let mut target = none(py);
        for sp in attr_or(scene, "section_planes", PyList::empty(py).into_any())?.try_iter()? {
            let sp = sp?;
            if sp.getattr("uid")?.eq(&uid)? {
                target = sp;
                break;
            }
        }
        scene.call_method1("set_active_section", (target,))?;
        scene.setattr("show_section_planes", section.call_method1("get", ("planes_shown", true))?.is_truthy()?)?;
        scene.setattr("show_section_cuts", section.call_method1("get", ("cuts_shown", true))?.is_truthy()?)?;
    }
    let georef = view.getattr("georef")?;
    if !georef.is_none() {
        for row in saved_view_georef_objects(py, scene)?.iter() {
            let (key, obj): (Bound<'_, PyAny>, Bound<'_, PyAny>) = row.extract()?;
            if !obj.is_none() && georef.contains(&key)? {
                obj.setattr("visible", georef.get_item(&key)?.is_truthy()?)?;
            }
        }
    }
    let shadows = view.getattr("shadows")?;
    if !shadows.is_none() && !attr_or(scene, "shadows", none(py))?.is_none() {
        saved_view_apply_shadows(py, scene, &shadows)?;
    }
    let objects = view.getattr("hidden_objects")?;
    if !objects.is_none() {
        let hidden = PySet::new(py, objects.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        for item in scene.call_method0("groups_by_uid")?.call_method0("items")?.try_iter()? {
            let (uid, g): (Bound<'_, PyAny>, Bound<'_, PyAny>) = item?.extract()?;
            g.setattr("hidden", hidden.contains(uid)?)?;
        }
    }
    let shown = view.getattr("hidden_shown")?;
    if !shown.is_none() {
        scene.setattr("show_hidden_objects", shown.call_method1("get", ("objects",))?.is_truthy()?)?;
        scene.setattr("show_hidden_geometry", shown.call_method1("get", ("geometry",))?.is_truthy()?)?;
    }
    Ok(())
}

/// `SavedView.apply(scene, camera, put_style=None)`: camera, then state.
#[pyfunction]
#[pyo3(signature = (view, scene, camera, put_style=None))]
fn saved_view_apply(py: Python<'_>, view: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>, camera: &Bound<'_, PyAny>,
                    put_style: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    saved_view_apply_camera(py, view, camera)?;
    saved_view_apply_state(py, view, scene, put_style)
}

fn sorted_items<'py>(py: Python<'py>, d: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let items = super::module(py, "builtins")?.getattr("sorted")?.call1((d.call_method0("items")?,))?;
    PyTuple::new(py, items.try_iter()?.collect::<PyResult<Vec<_>>>()?)
}

fn to_dict_or_none<'py>(py: Python<'py>, v: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if v.is_none() { Ok(none(py)) } else { v.call_method0("to_dict") }
}

/// `shown_state(scene)`: everything a recall can change besides the
/// camera, as one value to compare.
#[pyfunction]
fn saved_view_shown_state<'py>(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let style = attr_or(scene, "display_style", none(py))?;
    let shadows = attr_or(scene, "shadows", none(py))?;
    let section = if scene.hasattr("active_section")? { scene.call_method0("active_section")? } else { none(py) };
    let layers = scene.getattr("layers")?.try_iter()?
        .map(|ly| { let ly = ly?; Ok(PyTuple::new(py, [ly.getattr("key")?, ly.getattr("visible")?])?) })
        .collect::<PyResult<Vec<_>>>()?;
    let mut hidden = hidden_uids(scene, true)?;
    let sorted = super::module(py, "builtins")?.getattr("sorted")?.call1((PyList::new(py, hidden.drain(..))?,))?;
    PyTuple::new(py, [
        PyTuple::new(py, layers)?.into_any(),
        sorted_items(py, &attr_or(scene, "folder_visibility", PyDict::new(py).into_any())?)?.into_any(),
        to_dict_or_none(py, &style)?,
        attr_or(&section, "uid", none(py))?,
        attr_or(scene, "show_section_planes", none(py))?,
        attr_or(scene, "show_section_cuts", none(py))?,
        sorted_items(py, &saved_view_georef_state(py, scene)?.into_any())?.into_any(),
        to_dict_or_none(py, &shadows)?,
        PyTuple::new(py, sorted.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any(),
        attr_or(scene, "show_hidden_objects", none(py))?,
        attr_or(scene, "show_hidden_geometry", none(py))?,
    ])
}

/// `visibility_sig(scene)`: the geometry's visibility while a sheet
/// recalls a scene for a moment.
#[pyfunction]
fn saved_view_visibility_sig<'py>(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let effective = crate::model::module(py, "built_core")?.getattr("tag_edits")?.getattr("effective_visible")?;
    let allowed = attr_or(scene, "_navigator_allowed", none(py))?;
    let layers = scene.getattr("layers")?.try_iter()?
        .map(|ly| { let ly = ly?; Ok(PyTuple::new(py, [ly.getattr("key")?, effective.call1((scene, &ly))?])?) })
        .collect::<PyResult<Vec<_>>>()?;
    let mut geo = Vec::new();
    for row in saved_view_georef_objects(py, scene)?.iter() {
        let obj = row.get_item(1)?;
        if !obj.is_none() {
            geo.push(truthy_attr(&obj, "visible", true)?);
        }
    }
    let mut hidden = Vec::new();
    for item in scene.call_method0("groups_by_uid")?.call_method0("items")?.try_iter()? {
        let (uid, g): (Bound<'py, PyAny>, Bound<'py, PyAny>) = item?.extract()?;
        if g.getattr("hidden")?.is_truthy()? {
            hidden.push(uid);
        }
    }
    PyTuple::new(py, [
        PyTuple::new(py, layers)?.into_any(),
        PyTuple::new(py, geo)?.into_any(),
        PyFrozenSet::new(py, hidden)?.into_any(),
        if allowed.is_none() { none(py) } else { PyFrozenSet::new(py, allowed.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any() },
        scene.getattr("show_hidden_objects")?,
        scene.getattr("show_hidden_geometry")?,
    ])
}

/// `SavedView.from_dict(raw)` for the class `cls`: the .igz entry read
/// back, `parallel` the inverse of `perspective`.
#[pyfunction]
fn saved_view_from_dict<'py>(py: Python<'py>, cls: &Bound<'py, PyAny>, raw: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let get = |k: &str| raw.call_method1("get", (k,));
    let get_or = |k: &str, d: Bound<'py, PyAny>| raw.call_method1("get", (k, d));
    let mut t = get("target")?;
    if !t.is_truthy()? {
        t = PyList::new(py, [0.0, 0.0, 0.0])?.into_any();
    }
    let kw = PyDict::new(py);
    kw.set_item("target", PyTuple::new(py, [t.get_item(0)?, t.get_item(1)?, t.get_item(2)?])?)?;
    kw.set_item("distance", get_or("distance", 20.0f64.into_pyobject(py)?.into_any())?)?;
    kw.set_item("yaw", get_or("yaw", (-0.7853981633974483f64).into_pyobject(py)?.into_any())?)?;
    kw.set_item("pitch", get_or("pitch", 0.5235987755982988f64.into_pyobject(py)?.into_any())?)?;
    kw.set_item("fov_deg", get_or("fov", 45.0f64.into_pyobject(py)?.into_any())?)?;
    kw.set_item("perspective", !get_or("parallel", pybool(py, false))?.is_truthy()?)?;
    kw.set_item("two_point", get_or("two_point", pybool(py, false))?.is_truthy()?)?;
    for k in ["hidden_layers", "hidden_folders", "style", "section", "georef", "shadows", "hidden_objects",
              "hidden_shown", "navigator_view"] {
        kw.set_item(k, get(k)?)?;
    }
    let folder = get("folder")?;
    kw.set_item("folder", if folder.is_instance_of::<pyo3::types::PyString>() { folder } else { none(py) })?;
    let lens = get("lens")?;
    kw.set_item("lens", if lens.is_instance_of::<PyDict>() { lens } else { none(py) })?;
    cls.call((get_or("name", "Scene".into_pyobject(py)?.into_any())?,), Some(&kw))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(saved_view_georef_objects, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_georef_state, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_capture, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_take_camera, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_recapture, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_apply_camera, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_apply_shadows, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_apply_state, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_apply, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_shown_state, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_visibility_sig, m)?)?;
    m.add_function(wrap_pyfunction!(saved_view_from_dict, m)?)?;
    Ok(())
}
