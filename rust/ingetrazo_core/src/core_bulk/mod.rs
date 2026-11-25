// SPDX-License-Identifier: GPL-3.0-or-later
//! Core services shared by the native editor and Python plugins.
use pyo3::{
    prelude::*,
    types::{PyModule, PyTuple},
};
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
    for name in ["version", "triangulate", "settings", "timers"] {
        let m = PyModule::new(py, &format!("ingetrazo_core.{name}"))?;
        match name {
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
