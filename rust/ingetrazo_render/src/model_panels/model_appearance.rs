// SPDX-License-Identifier: GPL-3.0-or-later
use crate::settings_rows::*;
use pyo3::{prelude::*, types::PyList};

type Obj<'p> = Bound<'p, PyAny>;
const COLORS: &[(&str, &str)] = &[
    ("edge_color", "Edge color"),
    ("front_color", "Front color"),
    ("back_color", "Back color"),
    ("sky_color", "Sky color"),
    ("ground_color", "Ground color"),
    ("background", "Background"),
    ("section_fill_color", "Section fill color"),
];
fn active<'p>(env: &Obj<'p>) -> PyResult<Obj<'p>> {
    let win = env.getattr("win")?;
    if win.hasattr("active_style")? {
        win.call_method0("active_style")
    } else {
        env.getattr("scene")?.getattr("display_style")
    }
}
fn hint(style: &Obj<'_>, field: &str) -> PyResult<&'static str> {
    let mode: String = style.getattr("face_mode")?.extract()?;
    Ok(match field {
        "front_color" if mode=="wireframe" => "Front color saved — Wireframe draws no faces.",
        "back_color" if matches!(mode.as_str(),"hidden_line"|"wireframe") => "Back color saved — it paints back faces in Textures, Shaded, Monochrome and X-ray modes.",
        "background" if style.getattr("sky")?.is_truthy()? => "Background saved — it shows with the sky off.",
        "sky_color"|"ground_color" if !style.getattr("sky")?.is_truthy()? => "Sky and ground colors show with the sky on.",
        "edge_color" if mode!="wireframe" && !style.getattr("edges")?.is_truthy()? => "Edge color saved — this style has edges off.",
        "section_fill_color" if !style.getattr("section_fill")?.is_truthy()? => "Section fill color saved — section fill is off in this style.",
        _=>""
    })
}
fn write(py: Python<'_>, key: &'static str, color: bool) -> Res {
    func(py, move |py, a| {
        let (env, raw) = (a.get_item(0)?, a.get_item(1)?);
        let win = env.getattr("win")?;
        let scene = env.getattr("scene")?;
        let value = if color {
            let hex = raw.extract::<String>()?;
            let hex = hex.trim_start_matches('#');
            if hex.len() != 6 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(pyo3::exceptions::PyValueError::new_err("Use #rrggbb"));
            }
            let [r, g, b] =
                [0, 2, 4].map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64 / 255.);
            obj(py, (r, g, b))?.into_bound(py)
        } else {
            raw
        };
        if active(&env)?.getattr(key)?.eq(&value)? {
            return Ok(none(py));
        }
        let style = win.call_method0("own_style")?;
        style.setattr(key, value)?;
        if style.is(&scene.getattr("display_style")?) {
            scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
        }
        win.call_method0("repaint_styled")?;
        Ok(none(py))
    })
}
pub(crate) fn append<'p>(
    py: Python<'p>,
    env: &Obj<'p>,
    groups: &Bound<'p, PyList>,
) -> PyResult<()> {
    let scene = env.getattr("scene")?;
    if !scene.hasattr("display_style")? {
        return Ok(());
    }
    let style = active(env)?;
    let module = py.import("ingetrazo_core.style")?;
    let rows = PyList::empty(py);
    let names = PyList::empty(py);
    for coll in [
        module.getattr("BUILTIN_STYLES")?,
        module.call_method0("user_styles")?,
    ] {
        for item in coll.try_iter()? {
            let name: String = item?.getattr("name")?.extract()?;
            names.append((&name, tr(py, &name)?))?;
        }
    }
    let read = |key: &'static str| {
        func(py, move |_, a| {
            Ok(active(&a.get_item(0)?)?.getattr(key)?.unbind())
        })
    };
    rows.append(row(
        py,
        "style_preset",
        "choice",
        &tr(py, "Style")?,
        Some(read("name")?),
        Some(func(py, |py, a| {
            let env = a.get_item(0)?;
            let style = py
                .import("ingetrazo_core.style")?
                .call_method1("style_by_name", (a.get_item(1)?,))?;
            if !style.is_none() {
                let win = env.getattr("win")?;
                win.call_method1("set_active_style", (&style,))?;
                let scene = env.getattr("scene")?;
                if style.is(&scene.getattr("display_style")?) {
                    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
                }
            }
            Ok(none(py))
        })?),
        &[("options", names.into_any().unbind())],
    )?)?;
    let modes = PyList::empty(py);
    for (k, t) in [
        ("textures", "Textures"),
        ("shaded", "Shaded"),
        ("monochrome", "Monochrome"),
        ("xray", "X-ray"),
        ("wireframe", "Wireframe"),
        ("hidden_line", "Hidden line"),
    ] {
        modes.append((k, tr(py, t)?))?;
    }
    rows.append(row(
        py,
        "face_mode",
        "choice",
        &tr(py, "Face style")?,
        Some(read("face_mode")?),
        Some(write(py, "face_mode", false)?),
        &[("options", modes.into_any().unbind())],
    )?)?;
    for (k, t) in [
        ("edges", "Edges"),
        ("profiles", "Profiles"),
        ("sky", "Sky"),
        ("section_fill", "Section fill"),
    ] {
        rows.append(row(
            py,
            k,
            "check",
            &tr(py, t)?,
            Some(read(k)?),
            Some(write(py, k, false)?),
            &[],
        )?)?;
    }
    for &(k, t) in COLORS {
        let get = func(py, move |py, a| {
            let env = a.get_item(0)?;
            let style = active(&env)?;
            let rgb = if k == "back_color" {
                py.import("ingetrazo_core.style")?
                    .call_method1("effective_back_color", (style, env.getattr("scene")?))?
            } else {
                style.getattr(k)?
            };
            let [r, g, b] = rgb
                .extract::<[f64; 3]>()?
                .map(|v| (255. * v).round_ties_even() as u8);
            obj(py, format!("#{r:02x}{g:02x}{b:02x}"))
        })?;
        let tip = hint(&style, k)?;
        rows.append(row(
            py,
            k,
            "color",
            &tr(py, t)?,
            Some(get),
            Some(write(py, k, true)?),
            &[(
                "tip",
                obj(
                    py,
                    if tip.is_empty() {
                        String::new()
                    } else {
                        tr(py, tip)?
                    },
                )?,
            )],
        )?)?;
    }
    rows.append(row(
        py,
        "save_style",
        "button",
        &tr(py, "Save style…")?,
        None,
        None,
        &[(
            "press",
            func(py, |py, a| {
                let env = a.get_item(0)?;
                let style = active(&env)?.call_method0("copy")?.unbind();
                let owner = env.getattr("win")?.unbind();
                let done = func(py, move |py, a| {
                    let raw = a.get_item(0)?;
                    if let Some(name) = raw
                        .extract::<Option<String>>()?
                        .filter(|s| !s.trim().is_empty())
                    {
                        style.bind(py).setattr("name", name.trim())?;
                        let library = py.import("ingetrazo_core.style")?;
                        if let Err(error) =
                            library.call_method1("save_user_style", (style.bind(py),))
                        {
                            if !error.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
                                return Err(error);
                            }
                            owner
                                .bind(py)
                                .getattr("viewport")?
                                .call_method1("flash_status", (error.value(py).str()?,))?;
                        }
                        crate::sheet_runtime::refresh(owner.bind(py))?;
                    }
                    Ok(none(py))
                })?;
                py.import("ingetrazo_render.chrome_forms")?.call_method1(
                    "ask_text",
                    (
                        env.getattr("win")?,
                        tr(py, "Save style…")?,
                        tr(py, "Name:")?,
                        "",
                        done,
                    ),
                )?;
                Ok(none(py))
            })?,
        )],
    )?)?;
    rows.append(row(
        py,
        "delete_style",
        "button",
        &tr(py, "Delete saved style")?,
        None,
        None,
        &[
            (
                "enabled",
                obj(
                    py,
                    module
                        .call_method0("user_styles")?
                        .try_iter()?
                        .filter_map(Result::ok)
                        .any(|s| {
                            s.getattr("name")
                                .and_then(|n| n.eq(style.getattr("name")?))
                                .unwrap_or(false)
                        }),
                )?,
            ),
            (
                "press",
                func(py, |py, a| {
                    let env = a.get_item(0)?;
                    let lib = py.import("ingetrazo_core.style")?;
                    if lib
                        .call_method1("delete_user_style", (active(&env)?.getattr("name")?,))?
                        .is_truthy()?
                    {
                        env.getattr("win")?.call_method1(
                            "set_active_style",
                            (lib.call_method1("style_by_name", ("Default",))?,),
                        )?;
                    }
                    Ok(none(py))
                })?,
            ),
        ],
    )?)?;
    groups.insert(0, ("style", tr(py, "Style")?, rows))?;
    Ok(())
}
