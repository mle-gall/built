// SPDX-License-Identifier: GPL-3.0-or-later
//! Section rows, applicable inspector fields and validated history commands.
use crate::project_tree::{project_item_matches, row, Row};
use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
    types::PyDict,
};

const FOLDER: &str = "sections:all";

fn field(plane: &Bound<'_, PyAny>, key: &str) -> PyResult<String> {
    Ok(plane
        .getattr(key)?
        .extract::<Option<String>>()?
        .unwrap_or_default())
}

fn label(plane: &Bound<'_, PyAny>) -> PyResult<String> {
    let name = field(plane, "name")?;
    let symbol = field(plane, "symbol")?;
    Ok(match (symbol.is_empty(), name.is_empty()) {
        (true, true) => format!("#{}", plane.getattr("uid")?),
        (true, false) => name,
        (false, true) => symbol,
        _ => format!("{symbol} — {name}"),
    })
}

fn mode(plane: &Bound<'_, PyAny>) -> PyResult<&'static str> {
    Ok(
        match (
            plane.getattr("active")?.is_truthy()?,
            plane.getattr("hidden")?.is_truthy()?,
        ) {
            (true, false) => "Cut and frame",
            (true, true) => "Cut only",
            (false, false) => "Frame only",
            (false, true) => "Off",
        },
    )
}

pub(crate) fn rows(scene: &Bound<'_, PyAny>, query: &str, open: bool) -> PyResult<Vec<Row>> {
    let planes = scene.getattr("section_planes")?;
    let filter = !query.trim().is_empty();
    if planes.len()? == 0 {
        return Ok(vec![]);
    }
    let mut root = row(
        FOLDER.into(),
        "Section planes".into(),
        "",
        "folder",
        0,
        7,
        planes.len()?,
        0,
    );
    root.1[7] = 1;
    root.1[8] = i64::from(open || filter);
    root.1[5] = i64::from(scene.getattr("show_section_planes")?.is_truthy()?);
    let mut rows = vec![root];
    if open || filter {
        let selection = scene.getattr("selection")?;
        for plane in planes.try_iter()? {
            let p = plane?;
            if filter && !project_item_matches(scene, &p, "section", query, false)? {
                continue;
            }
            let key = format!("section:{}", p.getattr("uid")?);
            let mut r = row(
                key,
                label(&p)?,
                mode(&p)?,
                "section",
                1,
                7,
                0,
                usize::from(selection.contains(&p)?),
            );
            r.1[5] = i64::from(!p.getattr("hidden")?.is_truthy()?);
            rows.push(r);
        }
        if filter && rows.len() == 1 {
            rows.clear();
        }
    }
    Ok(rows)
}

fn plane<'py>(scene: &Bound<'py, PyAny>, uid: &str) -> PyResult<Bound<'py, PyAny>> {
    for plane in scene.getattr("section_planes")?.try_iter()? {
        let p = plane?;
        if p.getattr("uid")?.str()?.to_str()? == uid {
            return Ok(p);
        }
    }
    Err(PyKeyError::new_err("Section plane no longer exists"))
}

type Section = (String, Vec<(String, String)>);

#[pyfunction]
pub(crate) fn section_inspector(
    scene: &Bound<'_, PyAny>,
    key: &str,
) -> PyResult<Option<(String, Vec<Section>)>> {
    if key == FOLDER {
        let switches = [
            ("Frames", scene.getattr("show_section_planes")?.is_truthy()?),
            ("Cuts", scene.getattr("show_section_cuts")?.is_truthy()?),
            (
                "Fill",
                scene
                    .getattr("display_style")?
                    .getattr("section_fill")?
                    .is_truthy()?,
            ),
        ]
        .map(|(name, on)| (name.into(), if on { "On" } else { "Off" }.into()))
        .to_vec();
        let acts = [
            ("section_planes", "Show / hide frames"),
            ("section_cuts", "Switch cuts"),
            ("section_fill", "Switch fill"),
        ]
        .map(|(id, label)| (format!("sectionview:{id}"), label.to_owned()))
        .to_vec();
        return Ok(Some((
            "Section planes".into(),
            vec![("Display".into(), switches), ("Actions".into(), acts)],
        )));
    }
    let Some(uid) = key.strip_prefix("section:") else {
        return Ok(None);
    };
    let Ok(p) = plane(scene, uid) else {
        return Ok(None);
    };
    let edit =
        |field: &str, value: String| format!("\u{1d}edit\u{1d}section:{uid}:{field}\u{1d}{value}");
    let mut fields = vec![
        ("Name".into(), edit("name", field(&p, "name")?)),
        ("Symbol".into(), edit("symbol", field(&p, "symbol")?)),
        ("Mode".into(), mode(&p)?.into()),
    ];
    let mut acts = vec![
        (format!("section:{uid}:eye"), "Switch frame".into()),
        (format!("section:{uid}:cut"), "Switch cut".into()),
    ];
    if p.getattr("active")?.is_truthy()? {
        let values = crate::model::module(scene.py(), "ingetrazo_core")?
            .call_method1("section_effective_fill", (&p, scene.getattr("display_style")?))?;
        let colour: Vec<f64> = values.get_item(1)?.extract()?;
        let rgb = colour
            .iter()
            .take(3)
            .map(|v| format!("{:02x}", (v * 255.).round() as u8))
            .collect::<String>();
        fields.push(("Pattern".into(), values.get_item(0)?.extract()?));
        fields.push(("Colour".into(), edit("color", format!("#{rgb}"))));
        fields.push((
            "Hatch spacing".into(),
            edit("spacing", format!("{} m", values.get_item(2)?)),
        ));
        for (pattern, label) in [
            ("solid", "Solid"),
            ("hatch", "Hatch"),
            ("cross", "Crosshatch"),
            ("layers", "By tag"),
            ("none", "No fill"),
            ("inherit", "As the style"),
        ] {
            acts.push((format!("section:{uid}:pattern:{pattern}"), label.into()));
        }
        acts.push(("sectionview:section_fill".into(), "Switch fill".into()));
    }
    Ok(Some((
        label(&p)?,
        vec![("Section".into(), fields), ("Actions".into(), acts)],
    )))
}

#[pyfunction]
pub(crate) fn section_edit<'py>(
    scene: &Bound<'py, PyAny>,
    uid: &str,
    field_name: &str,
    value: &str,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let p = plane(scene, uid)?;
    let py = p.py();
    let history = py.import("ingetrazo_core.history")?;
    let active = p.getattr("active")?.is_truthy()?;
    let hidden = p.getattr("hidden")?.is_truthy()?;
    match field_name {
        "eye" | "cut" => Ok(Some(history.call_method1(
            "SetSectionModeCommand",
            (
                &p,
                active ^ (field_name == "cut"),
                !hidden ^ (field_name == "eye"),
            ),
        )?)),
        "name" | "symbol" => {
            let mut name = field(&p, "name")?;
            let mut symbol = field(&p, "symbol")?;
            let target = if field_name == "name" {
                &mut name
            } else {
                &mut symbol
            };
            if target == value.trim() {
                return Ok(None);
            }
            *target = value.trim().into();
            if name.is_empty() && symbol.is_empty() {
                return Err(PyValueError::new_err("Give the plane a name or symbol"));
            }
            Ok(Some(history.call_method1(
                "RenameSectionPlaneCommand",
                (&p, name, symbol),
            )?))
        }
        "pattern" | "color" | "spacing" if active => {
            let old = p.getattr("fill")?;
            let fill = if old.is_none() {
                PyDict::new(py).into_any()
            } else {
                py.get_type::<PyDict>().call1((&old,))?
            };
            let value = value.trim();
            if value.is_empty() || value == "inherit" {
                fill.call_method1("pop", (field_name, py.None()))?;
            } else if field_name == "pattern" {
                if !py
                    .import("ingetrazo_core.section")?
                    .getattr("FILL_PATTERNS")?
                    .contains(value)?
                {
                    return Err(PyValueError::new_err("Unknown section pattern"));
                }
                fill.set_item(field_name, value)?;
            } else if field_name == "spacing" {
                let length = py
                    .import("core.units")?
                    .call_method1("parse_length_field", (value,))?
                    .extract::<Option<f64>>()?;
                let Some(length) = length.filter(|v| v.is_finite() && *v > 0.) else {
                    return Err(PyValueError::new_err("Enter a positive hatch spacing"));
                };
                fill.set_item(field_name, length)?;
            } else {
                let rgb = value.strip_prefix('#').unwrap_or(value);
                let n = u32::from_str_radix(rgb, 16)
                    .ok()
                    .filter(|_| rgb.len() == 6)
                    .ok_or_else(|| PyValueError::new_err("Enter a six-digit RGB colour"))?;
                let channel = |s| ((n >> s) & 255u32) as f64 / 255.;
                fill.set_item(field_name, (channel(16), channel(8), channel(0)))?;
            }
            if old.eq(&fill)? || (!fill.is_truthy()? && old.is_none()) {
                return Ok(None);
            }
            Ok(Some(
                history.call_method1("SetSectionFillCommand", (&p, &fill))?,
            ))
        }
        _ => Err(PyValueError::new_err(
            "This edit does not apply to the section plane",
        )),
    }
}
