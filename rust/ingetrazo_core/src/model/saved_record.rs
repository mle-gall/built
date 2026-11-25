// SPDX-License-Identifier: GPL-3.0-or-later
//! Journalled saved views share their camera, archive and presentation methods
//! with the document's existing native state machine.
use pyo3::{
    exceptions::PyAttributeError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
const MODULE: &str = "ingetrazo_core.saved_views";
const FIELDS: &[&str] = &[
    "name",
    "navigator_view",
    "folder",
    "target",
    "distance",
    "yaw",
    "pitch",
    "fov_deg",
    "perspective",
    "two_point",
    "hidden_layers",
    "hidden_folders",
    "style",
    "section",
    "georef",
    "shadows",
    "hidden_objects",
    "hidden_shown",
    "lens",
];

fn as_float(value: &Obj) -> PyResult<f64> {
    value
        .py()
        .get_type::<pyo3::types::PyFloat>()
        .call1((value,))?
        .extract()
}
fn as_tuple(value: &Obj) -> PyResult<Py<PyTuple>> {
    Ok(value
        .py()
        .get_type::<PyTuple>()
        .call1((value,))?
        .cast_into::<PyTuple>()?
        .unbind())
}
fn origin() -> Py<PyTuple> {
    Python::attach(|py| {
        (0., 0., 0.)
            .into_pyobject(py)
            .expect("fixed coordinate tuple")
            .unbind()
    })
}
#[pyfunction(name="__init__",signature=(view,name,target=origin(),distance=20.0,yaw=-std::f64::consts::FRAC_PI_4,pitch=std::f64::consts::FRAC_PI_6,fov_deg=45.,perspective=true,two_point=false,hidden_layers=None,style=None,section=None,georef=None,shadows=None,hidden_objects=None,hidden_shown=None,navigator_view=None,folder=None,lens=None,hidden_folders=None))]
#[allow(clippy::too_many_arguments)]
fn init(
    view: &Obj,
    name: &Obj,
    #[pyo3(from_py_with = as_tuple)] target: Py<PyTuple>,
    #[pyo3(from_py_with = as_float)] distance: f64,
    #[pyo3(from_py_with = as_float)] yaw: f64,
    #[pyo3(from_py_with = as_float)] pitch: f64,
    #[pyo3(from_py_with = as_float)] fov_deg: f64,
    #[pyo3(from_py_with = Bound::is_truthy)] perspective: bool,
    #[pyo3(from_py_with = Bound::is_truthy)] two_point: bool,
    hidden_layers: Option<&Obj>,
    style: Option<&Obj>,
    section: Option<&Obj>,
    georef: Option<&Obj>,
    shadows: Option<&Obj>,
    hidden_objects: Option<&Obj>,
    hidden_shown: Option<&Obj>,
    navigator_view: Option<&Obj>,
    folder: Option<&Obj>,
    lens: Option<&Obj>,
    hidden_folders: Option<&Obj>,
) -> PyResult<()> {
    let py = view.py();
    view.setattr("name", name)?;
    view.setattr("navigator_view", navigator_view)?;
    if folder.map(|v| v.is_truthy()).transpose()?.unwrap_or(false) {
        view.setattr("folder", folder)?;
    } else {
        view.setattr("folder", py.None())?;
    }
    view.setattr("target", target)?;
    for (k, v) in [
        ("distance", distance),
        ("yaw", yaw),
        ("pitch", pitch),
        ("fov_deg", fov_deg),
    ] {
        view.setattr(k, v)?;
    }
    view.setattr("perspective", perspective)?;
    view.setattr("two_point", two_point)?;
    for (key, v) in [
        ("hidden_layers", hidden_layers),
        ("hidden_folders", hidden_folders),
        ("hidden_objects", hidden_objects),
    ] {
        match v {
            Some(v) => view.setattr(key, py.get_type::<PyList>().call1((v,))?)?,
            None if key == "hidden_layers" => view.setattr(key, PyList::empty(py))?,
            None => view.setattr(key, py.None())?,
        }
    }
    // None means a file predating that setting. An empty lens explicitly resets it.
    for (k, v) in [
        ("style", style),
        ("section", section),
        ("georef", georef),
        ("shadows", shadows),
        ("hidden_shown", hidden_shown),
        ("lens", lens),
    ] {
        if let Some(d) = v {
            if d.is_truthy()? {
                view.setattr(k, py.get_type::<PyDict>().call1((d,))?)?;
                continue;
            }
        }
        if k == "lens" {
            view.setattr(k, PyDict::new(py))?;
        } else {
            view.setattr(k, py.None())?;
        }
    }
    Ok(())
}
#[pyfunction(signature=(name,eye,target,up,fov_deg=45.,perspective=true,ortho_height=None,hidden_layers=None))]
#[allow(clippy::too_many_arguments)]
fn from_lookat<'p>(
    py: Python<'p>,
    name: &Obj<'p>,
    eye: [f64; 3],
    target: &Obj<'p>,
    up: &Obj<'p>,
    fov_deg: f64,
    perspective: bool,
    ortho_height: Option<f64>,
    hidden_layers: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let down = [
        up.get_item(0)?.neg()?.extract()?,
        up.get_item(1)?.neg()?.extract()?,
    ];
    let (dist, yaw, pitch) = super::saved_view::lookat(
        eye,
        target.extract()?,
        down,
        fov_deg,
        perspective,
        ortho_height,
    );
    let kw = PyDict::new(py);
    kw.set_item("hidden_layers", hidden_layers)?;
    py.import(MODULE)?.getattr("SavedView")?.call(
        (name, target, dist, yaw, pitch, fov_deg, perspective),
        Some(&kw),
    )
}
#[pyfunction(name = "__getattr__")]
fn lazy<'p>(py: Python<'p>, name: &str) -> PyResult<Obj<'p>> {
    let m = py.import(MODULE)?;
    match name {
        "SavedView" => {
            let cls = crate::record_class::build(
                &m,
                name,
                FIELDS,
                false,
                vec![wrap_pyfunction!(init, &m)?],
                &[],
            )?;
            crate::record_class::bind_methods(
                &cls,
                &[
                    ("capture", "saved_view_capture", true),
                    ("take_camera", "saved_view_take_camera", false),
                    ("recapture", "saved_view_recapture", false),
                    ("apply", "saved_view_apply", false),
                    ("apply_camera", "saved_view_apply_camera", false),
                    ("apply_state", "saved_view_apply_state", false),
                    ("from_dict", "saved_view_from_dict", true),
                ],
            )?;
            cls.setattr("to_dict", cls.getattr("_file_view")?)?;
            cls.setattr("_photo_tick", 0)?;
            Ok(cls)
        }
        "MoveSceneCommand" | "RenameSceneCommand" => py.import("ingetrazo_core")?.getattr(name),
        "CommandRejected" => py.import("ingetrazo_core.history")?.getattr(name),
        _ => Err(PyAttributeError::new_err(name.to_owned())),
    }
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), MODULE)?;
    sub.add_function(wrap_pyfunction!(lazy, &sub)?)?;
    sub.add_function(wrap_pyfunction!(from_lookat, &sub)?)?;
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(MODULE, &sub)?;
    sub.setattr(
        "__all__",
        [
            "SavedView",
            "from_lookat",
            "CommandRejected",
            "MoveSceneCommand",
            "RenameSceneCommand",
        ],
    )?;
    m.add("saved_views", sub)
}
