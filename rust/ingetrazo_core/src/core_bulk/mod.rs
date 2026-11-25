// SPDX-License-Identifier: GPL-3.0-or-later
//! Core services shared by the native editor and Python plugins.
use pyo3::{
    prelude::*,
    types::{PyModule, PyTuple},
};
mod annotations;
mod autosave;
mod axes;
mod geometry;
mod i18n;
mod materials;
mod paths;
mod structure;
mod style;
mod sun;
use pyo3::types::PyDict;

fn method(ns: &Bound<PyDict>, name: &str, func: &Obj) -> PyResult<()> {
    ns.set_item(
        name,
        ns.py()
            .import("functools")?
            .call_method1("partialmethod", (func,))?,
    )
}
fn class_method(ns: &Bound<PyDict>, name: &str, func: &Obj) -> PyResult<()> {
    ns.set_item(
        name,
        ns.py()
            .import("builtins")?
            .call_method1("classmethod", (func,))?,
    )
}
fn data_class<'p>(
    m: &Module<'p>,
    name: &str,
    fields: &[&str],
    ns: &Bound<'p, PyDict>,
    base: Option<Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = m.py();
    let ann = PyDict::new(py);
    for &key in fields {
        ann.set_item(
            key,
            match (name, key) {
                ("Material", "name") => "str",
                ("Material", "color") => "Optional[tuple]",
                ("Material", "texture") => "Optional[dict]",
                ("Material", "opacity") => "Optional[float]",
                ("Material", "finish") => "Optional[str]",
                ("Material", "pbr") => "Optional[dict]",
                ("ShadowSettings", "enabled") => "bool",
                ("ShadowSettings", "month") => "int",
                ("ShadowSettings", "day") => "int",
                ("ShadowSettings", "hour") => "int",
                ("ShadowSettings", "minute") => "int",
                ("ShadowSettings", "darkness") => "float",
                ("ShadowSettings", "utc_offset") => "int | None",
                ("Style", "name") => "str",
                ("Style", "face_mode") => "str",
                ("Style", "edges") => "bool",
                ("Style", "profiles") => "bool",
                ("Style", "back_edges") => "bool",
                ("Style", "edge_color") => "tuple",
                ("Style", "front_color") => "tuple",
                ("Style", "back_color") => "tuple | None",
                ("Style", "background") => "tuple",
                ("Style", "sky") => "bool",
                ("Style", "sky_color") => "tuple",
                ("Style", "ground_color") => "tuple",
                ("Style", "section_fill") => "bool",
                ("Style", "section_fill_color") => "tuple",
                ("Style", "ambient_occlusion") => "bool",
                ("Style", "ao_distance") => "float",
                ("Style", "ao_intensity") => "float",
                ("Style", "color_by_layer") => "bool",
                ("Style", "model_linetypes") => "bool",
                _ => "object",
            },
        )?;
    }
    ns.set_item("__annotations__", ann)?;
    ns.set_item("__module__", m.name()?)?;
    let cls = py.import("builtins")?.call_method1(
        "type",
        (
            name,
            PyTuple::new(py, base.into_iter().collect::<Vec<_>>())?,
            ns,
        ),
    )?;
    let cls = py
        .import("dataclasses")?
        .call_method1("dataclass", (cls,))?;
    cls.setattr(
        "__module__",
        m.name()?.to_string().replace("ingetrazo_core.", "core."),
    )?;
    m.add(name, &cls)?;
    Ok(cls)
}

type Obj<'p> = Bound<'p, PyAny>;
type Module<'p> = Bound<'p, PyModule>;

#[pyfunction(pass_module)]
fn use_native(m: &Module<'_>, profile: &Obj<'_>) -> PyResult<()> {
    m.setattr(
        "_store",
        m.py()
            .import("ingetrazo_core")?
            .call_method1("NativeSettings", (profile,))?,
    )
}

#[pyfunction(pass_module)]
fn settings<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    let chosen = m.getattr("_store")?;
    if !chosen.is_none() {
        return chosen.call_method0("fresh");
    }
    let py = m.py();
    let profile = py
        .import("os")?
        .getattr("environ")?
        .call_method1("get", ("BUILT_SETTINGS_DIR",))?;
    let profile = if profile.is_truthy()? {
        profile
    } else {
        py.None().into_bound(py)
    };
    let implicit = m.getattr("_implicit")?;
    let mut store = implicit.get_item(1)?;
    if store.is_none() || !implicit.get_item(0)?.eq(&profile)? {
        store = py
            .import("ingetrazo_core")?
            .call_method1("NativeSettings", (&profile,))?;
        m.setattr("_implicit", (profile, &store))?;
    }
    store.call_method0("fresh")
}

fn detached<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    let mut q = m.getattr("_detached")?;
    if q.is_none() {
        q = m
            .py()
            .import("ingetrazo_render")?
            .call_method0("WindowTimers")?;
        m.setattr("_detached", &q)?;
    }
    Ok(q)
}
fn queue<'p>(m: &Module<'p>, owner: &Obj<'p>) -> PyResult<Obj<'p>> {
    if let Some(host) = owner.getattr_opt("_host")? {
        if let Some(q) = host.getattr_opt("timers")?.filter(|q| !q.is_none()) {
            return Ok(q);
        }
    }
    detached(m)
}
#[pyfunction(pass_module,signature=(owner,callback,interval=0,repeat=false))]
fn timer<'p>(
    m: &Module<'p>,
    owner: &Obj<'p>,
    callback: &Obj<'p>,
    interval: i64,
    repeat: bool,
) -> PyResult<Obj<'p>> {
    queue(m, owner)?.call_method1("timer", (callback, interval, repeat, owner))
}
#[pyfunction(pass_module)]
fn later(m: &Module<'_>, owner: &Obj<'_>, delay: &Obj<'_>, callback: &Obj<'_>) -> PyResult<()> {
    queue(m, owner)?.call_method1("later", (owner, delay, callback))?;
    Ok(())
}
#[pyfunction(pass_module)]
fn run_due(m: &Module<'_>) -> PyResult<()> {
    detached(m)?.call_method0("run_due")?;
    Ok(())
}
#[pyfunction(pass_module)]
fn next_ms<'p>(m: &Module<'p>) -> PyResult<Obj<'p>> {
    detached(m)?.call_method0("next_ms")
}
#[pyfunction(pass_module)]
fn cancel_context(m: &Module<'_>, context: &Obj<'_>) -> PyResult<()> {
    detached(m)?.call_method1("cancel_context", (context,))?;
    Ok(())
}

pub(crate) fn register(core: &Module<'_>) -> PyResult<()> {
    let py = core.py();
    let modules = py.import("sys")?.getattr("modules")?;
    for name in [
        "version",
        "triangulate",
        "settings",
        "timers",
        "paths",
        "i18n",
        "autosave",
        "axes",
        "geometry",
        "guide",
        "textlabel",
        "sun",
        "style",
        "materials",
        "structure",
    ] {
        let m = PyModule::new(py, &format!("ingetrazo_core.{name}"))?;
        modules.set_item(m.name()?, &m)?;
        match name {
            "i18n" => i18n::install(&m)?,
            "paths" => paths::install(&m)?,
            "autosave" => autosave::install(&m)?,
            "axes" => axes::install(&m, core)?,
            "geometry" => geometry::install(&m, core)?,
            "guide" => annotations::install(&m, false)?,
            "textlabel" => annotations::install(&m, true)?,
            "sun" => sun::install(&m)?,
            "style" => style::install(&m)?,
            "materials" => materials::install(&m, core)?,
            "structure" => structure::install(&m, core)?,
            "version" => {
                let v = include_str!("../../VERSION").trim();
                m.add("__version__", v)?;
                m.add(
                    "USER_AGENT",
                    format!("IngeTrazo/{v} (+https://ingetrazo.com)"),
                )?;
            }
            "triangulate" => {
                let v = core.getattr("Vec3")?;
                m.add("QVector3D", &v)?;
                let tuple = py.import("builtins")?.getattr("tuple")?;
                let float = py.import("builtins")?.getattr("float")?;
                m.add("Point2", tuple.get_item((&float, &float))?)?;
                m.add("Tri3", tuple.get_item((&v, &v, &v))?)?;
                for (n, f) in [
                    ("plane_axes", "drawing_plane_axes"),
                    ("_newell", "polygon_normal"),
                    ("earcut", "polygon_earcut"),
                    ("is_convex", "polygon_is_convex"),
                    ("triangulate", "polygon_triangles"),
                ] {
                    m.add(n, core.getattr(f)?)?;
                }
            }
            "settings" => {
                m.add("_store", py.None())?;
                m.add("_implicit", PyTuple::new(py, [py.None(), py.None()])?)?;
                m.add_function(wrap_pyfunction!(use_native, &m)?)?;
                m.add_function(wrap_pyfunction!(settings, &m)?)?;
            }
            _ => {
                m.add("_detached", py.None())?;
                for f in [
                    wrap_pyfunction!(timer, &m)?,
                    wrap_pyfunction!(later, &m)?,
                    wrap_pyfunction!(run_due, &m)?,
                    wrap_pyfunction!(next_ms, &m)?,
                    wrap_pyfunction!(cancel_context, &m)?,
                ] {
                    m.add_function(f)?;
                }
            }
        }
        modules.set_item(m.name()?, &m)?;
        core.add(name, m)?;
    }
    Ok(())
}
