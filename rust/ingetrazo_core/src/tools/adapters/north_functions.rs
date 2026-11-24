// SPDX-License-Identifier: GPL-3.0-or-later
//! Functions exported for legacy tool callers.
use super::*;

fn compass_segments<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.north_host")?;
    let mut first: Obj<'py>;
    let mut b: Obj<'py>;
    let centre = a.get_item(0)?;
    let north_deg = a.get_item(2)?;
    let radius = a.get_item(1)?;
    return Ok({
        let items = PyList::empty(py);
        for item in module
            .call_method1("ToolMachine", ("north",))?
            .call_method1(
                "query",
                (
                    "compass",
                    PyList::new(
                        py,
                        [
                            module.call_method1("_tuple", (&centre,))?,
                            radius.clone(),
                            north_deg.clone(),
                        ],
                    )?
                    .into_any(),
                ),
            )?
            .try_iter()?
        {
            {
                let [part0, part1] = unpack::<2>(&item?)?;
                first = part0;
                b = part1;
            }
            items.append(
                (
                    module.call_method1("Vec3", positional(&first)?)?,
                    module.call_method1("Vec3", positional(&b)?)?,
                )
                    .into_bound_py_any(py)?,
            )?;
        }
        items.into_any()
    });
}

fn north_deg_towards<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.north_host")?;
    let centre = a.get_item(0)?;
    let point = a.get_item(1)?;
    module
        .call_method1("ToolMachine", ("north",))?
        .call_method1(
            "query",
            (
                "angle",
                PyList::new(
                    py,
                    [
                        module.call_method1("_tuple", (&centre,))?,
                        module.call_method1("_tuple", (&point,))?,
                    ],
                )?
                .into_any(),
            ),
        )
}

fn refresh_north_display<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let changed: Obj<'py>;
    let reset: Obj<'py>;
    let viewport = a.get_item(0)?;
    let scene = optional(&viewport, "scene")?;
    if !optional(&scene, "georef")?.is_none() {
        reset = optional(&viewport, "reset_tiles")?;
        if reset.is_callable() {
            reset.call0()?;
        }
        changed = optional(&viewport, "tilesChanged")?;
        if !changed.is_none() {
            changed.call_method0("emit")?;
        }
    }
    let notify = optional(&viewport, "notify_scene_changed")?;
    if notify.is_callable() {
        notify.call0()?;
    }
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("refresh_north_display", Mode::Function, refresh_north_display, &["viewport"], "None", "After the north changed on a located model: the map is redrawn from\nthe new datum and the terrain rebuilt (``tilesChanged`` → the window's\nterrain build). Without a location only the sun moved: a repaint."),
    ("north_deg_towards", Mode::Function, north_deg_towards, &["centre", "point"], "", ""),
    ("compass_segments", Mode::Function, compass_segments, &["centre", "radius", "north_deg"], "", ""),
];
