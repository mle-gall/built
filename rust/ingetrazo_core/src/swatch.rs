// SPDX-License-Identifier: GPL-3.0-or-later
//! Active paint is a draft until its first stroke. Editing a registered
//! swatch restamps uses and records one history step instead.
use super::material_library::pick_library_paint;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use serde_json::Value;
use std::sync::LazyLock;
type Obj<'py> = Bound<'py, PyAny>;
type Sections = Vec<(String, Vec<(String, String)>)>;

static RAL: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../resources/colors/ral.json"))
        .expect("bundled RAL palette")
});

#[pyfunction]
pub fn ral_rgb(name: &str) -> Option<[f64; 3]> {
    if !name.starts_with("RAL ") {
        return None;
    }
    RAL["families"]
        .as_array()?
        .iter()
        .filter_map(|f| f["colors"].as_array())
        .flatten()
        .find_map(|row| {
            let code = row["code"].as_str()?;
            let exact = ["name", "name_es"]
                .iter()
                .filter_map(|key| row[key].as_str())
                .any(|label| name == format!("{code} {label}"));
            exact
                .then(|| serde_json::from_value(row["rgb"].clone()).ok())
                .flatten()
        })
}

#[pyfunction]
pub fn recoloured_name(scene: &Obj<'_>, name: &str, rgb: [f64; 3]) -> PyResult<String> {
    if !ral_rgb(name).is_some_and(|v| v.iter().zip(rgb).any(|(a, b)| (a - b).abs() >= 0.002)) {
        return Ok(name.into());
    }
    let py = scene.py();
    let suffix: String = py
        .import("core.i18n")?
        .call_method1("tr", ("(modified)",))?
        .extract()?;
    py.import("core.materials")?
        .call_method1(
            "free_name",
            (format!("{name} {suffix}"), scene.getattr("materials")?),
        )?
        .extract()
}

fn decode_rgb(raw: &str) -> PyResult<[f64; 3]> {
    if raw.len() == 7 && raw.starts_with('#') {
        if let Ok(n) = u32::from_str_radix(&raw[1..], 16) {
            return Ok([16, 8, 0].map(|s| f64::from((n >> s) & 255) / 255.));
        }
    }
    serde_json::from_str::<[f64; 3]>(raw)
        .ok()
        .filter(|v| v.iter().all(|n| (0.0..=1.).contains(n)))
        .ok_or_else(|| PyValueError::new_err("Choose an RGB colour"))
}

#[pyfunction]
pub fn brush_inspector(brush: &Obj<'_>) -> PyResult<(String, Sections)> {
    let mat = brush.getattr("current_material")?;
    let name = if mat.is_none() {
        String::new()
    } else {
        mat.getattr("name")?.extract()?
    };
    let field = |kind: &str, id: &str, value: &str| {
        format!("\u{1d}{kind}\u{1d}libmat:brush_{id}:{name}\u{1d}{value}")
    };
    let mut rows = vec![("Name".into(), field("edit", "name", &name))];
    let texture = brush.getattr("current_texture")?;
    if !texture.is_truthy()? {
        let rgb: [f64; 3] = brush.getattr("current_color")?.extract()?;
        let [r, g, b] = rgb.map(|v| (v * 255.).round() as u8);
        rows.push((
            "Colour".into(),
            field("color", "color", &format!("#{r:02x}{g:02x}{b:02x}")),
        ));
    } else {
        for (label, key, fallback) in [
            ("Tile width (m)", "sw", 1.),
            ("Tile height (m)", "sh", 1.),
            ("Rotation (°)", "rot", 0.),
        ] {
            let value: f64 = texture.call_method1("get", (key, fallback))?.extract()?;
            rows.push((label.into(), field("edit", key, &value.to_string())));
        }
    }
    Ok((
        "Active paint".into(),
        vec![("Next paint stroke".into(), rows)],
    ))
}

#[pyfunction]
pub fn edit_swatch(
    history: &Obj<'_>,
    brush: &Obj<'_>,
    name: &str,
    field: &str,
    raw: &str,
) -> PyResult<String> {
    let scene = history.getattr("scene")?;
    let py = scene.py();
    let mats = scene.getattr("materials")?;
    let active = brush.getattr("current_material")?;
    let live = mats.call_method1("get", (name,))?;
    let pending = field.starts_with("brush_");
    let active_name = if active.is_none() {
        String::new()
    } else {
        active.getattr("name")?.extract()?
    };
    if pending && (brush.getattr("current_is_default")?.is_truthy()? || active_name != name) {
        return Err(PyValueError::new_err("The active swatch has changed"));
    }
    if !pending && live.is_none() {
        return Err(PyValueError::new_err("This material no longer exists"));
    }
    if field == "finish" {
        let finish = if raw == "auto" {
            None
        } else if crate::material_finish::lookup(raw).is_some() {
            Some(raw)
        } else {
            return Err(PyValueError::new_err("Unknown render finish"));
        };
        if live.getattr("finish")?.extract::<Option<String>>()?.as_deref() != finish {
            let step = py.import("ingetrazo_core")?.call_method1("SetMaterialFinishCommand", (name, finish))?;
            history.call_method1("execute", (step,))?;
        }
        return Ok(name.to_owned());
    }
    let base = if !live.is_none() {
        live.clone()
    } else if !active.is_none() {
        active.clone()
    } else {
        py.import("core.material_commands")?.call_method1(
            "material_for_paint",
            (
                &scene,
                brush.getattr("current_color")?,
                brush.getattr("current_texture")?,
                brush.getattr("current_opacity")?,
            ),
        )?
    };
    let data = base.call_method0("to_dict")?.cast_into::<PyDict>()?;
    let dest = match field.trim_start_matches("brush_") {
        "name" => {
            let dest = raw.trim();
            if dest.is_empty() {
                return Err(PyValueError::new_err("Enter a material name"));
            }
            if dest != name && mats.contains(dest)? {
                if !live.is_none() {
                    return Err(PyValueError::new_err(
                        "Another material already uses that name",
                    ));
                }
                pick_library_paint(&scene, brush, dest, None)?;
                return Ok(dest.into());
            }
            dest.to_owned()
        }
        "color" => {
            let rgb = decode_rgb(raw)?;
            data.set_item("color", rgb)?;
            recoloured_name(
                &scene,
                &data.get_item("name")?.unwrap().extract::<String>()?,
                rgb,
            )?
        }
        key @ ("sw" | "sh" | "rot") if pending && live.is_none() => {
            let value: f64 = raw
                .trim()
                .replace(',', ".")
                .parse()
                .map_err(|_| PyValueError::new_err("Enter a number"))?;
            if !value.is_finite() || (key != "rot" && value <= 0.) {
                return Err(PyValueError::new_err(
                    "Enter a finite angle or a positive tile size",
                ));
            }
            let tex = brush
                .getattr("current_texture")?
                .cast_into::<PyDict>()?
                .copy()?;
            tex.set_item(key, value)?;
            data.set_item("texture", tex)?;
            data.get_item("name")?.unwrap().extract()?
        }
        _ => return Err(PyValueError::new_err("Unknown swatch field")),
    };
    data.set_item("name", &dest)?;
    if live.is_none() {
        pick_library_paint(&scene, brush, "", Some(&data))?;
    } else {
        let new = py
            .import("core.materials")?
            .getattr("Material")?
            .call_method1("from_dict", (&data,))?;
        if !new.eq(&live)? {
            let cmd = py
                .import("core.history")?
                .call_method1("RestampMaterialCommand", (name, &new))?;
            let before: u64 = history.getattr("revision")?.extract()?;
            history.call_method1("execute", (cmd,))?;
            if history.getattr("revision")?.extract::<u64>()? == before {
                return Err(PyValueError::new_err("Material edit was rejected"));
            }
            let recent = brush.getattr("recent")?;
            let updated = py.import("tools.paint")?.call_method1(
                "rename_recent",
                (&recent, name, new.call_method0("face_attrs")?),
            )?;
            if updated.ne(recent)? {
                brush.setattr("recent", &updated)?;
                save_recent_materials(&updated)?;
            }
        }
        if active_name == name {
            pick_library_paint(&scene, brush, &dest, None)?;
        }
    }
    Ok(dest)
}

fn settings(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("core.settings")?.getattr("settings")?.call0()
}
const KEY: &str = "paint/recent_materials";
#[pyfunction]
pub fn load_recent_materials(py: Python<'_>) -> PyResult<Obj<'_>> {
    let text = settings(py)?
        .call_method1("value", (KEY, ""))?
        .extract::<String>()
        .unwrap_or_default();
    let entries: Vec<Value> = serde_json::from_str(&text).unwrap_or_default();
    let valid: Vec<_> = entries
        .into_iter()
        .filter(|v| {
            if let Some(texture) = v
                .get("texture")
                .filter(|t| !t.is_null() && t.as_object().is_none_or(|o| !o.is_empty()))
            {
                return texture["path"]
                    .as_str()
                    .is_some_and(|p| std::path::Path::new(p).is_file());
            }
            v["color"].as_array().is_some_and(|c| c.len() >= 3)
        })
        .take(6)
        .collect();
    py.import("json")?
        .call_method1("loads", (serde_json::json!(valid).to_string(),))
}
#[pyfunction]
pub fn save_recent_materials(entries: &Obj<'_>) -> PyResult<()> {
    let py = entries.py();
    let slice = entries.get_item(pyo3::types::PySlice::new(py, 0, 6, 1))?;
    if let Ok(text) = py.import("json")?.call_method1("dumps", (slice,)) {
        settings(py)?.call_method1("setValue", (KEY, text))?;
    }
    Ok(())
}
#[pyclass]
pub struct PaintRecents {
    brush: Py<PyAny>,
}
#[pymethods]
impl PaintRecents {
    #[new]
    fn new(brush: Obj<'_>) -> PyResult<Self> {
        if !brush.getattr("recent")?.is_truthy()? {
            brush.setattr("recent", load_recent_materials(brush.py())?)?;
        }
        Ok(Self {
            brush: brush.unbind(),
        })
    }
    fn recent_materials_changed(&self, py: Python<'_>) -> PyResult<()> {
        save_recent_materials(&self.brush.bind(py).getattr("recent")?)
    }
}

/// Keep the complete stamp in the row key: a stroke can reorder the MRU
/// between painting the strip and dispatching a click.
#[pyfunction]
pub fn recent_swatches(brush: &Obj<'_>) -> PyResult<Vec<(String, String, String, String)>> {
    let py = brush.py();
    let dumps = py.import("json")?.getattr("dumps")?;
    let mut rows = vec![];
    for item in brush.getattr("recent")?.try_iter()?.take(6) {
        let item = item?;
        let raw: String = dumps.call1((&item,))?.extract()?;
        let Ok(data) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let path = data["texture"]["path"].as_str().unwrap_or("");
        let rgb = serde_json::from_value::<[f64; 3]>(data["color"].clone()).ok();
        if path.is_empty() && rgb.is_none() {
            continue;
        }
        let hex = rgb
            .map(|v| {
                let [r, g, b] = v.map(|n| (n * 255.).round() as u8);
                format!("#{r:02x}{g:02x}{b:02x}")
            })
            .unwrap_or_default();
        let label = data["mat"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| std::path::Path::new(path).file_stem()?.to_str())
            .unwrap_or(&hex);
        rows.push((format!("stamp:{raw}"), label.into(), hex, path.into()));
    }
    Ok(rows)
}

#[pyfunction]
pub fn pick_recent(scene: &Obj<'_>, brush: &Obj<'_>, stamp: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let recipe = stamp.cast::<PyDict>()?.copy()?;
    let name: Option<String> = recipe.get_item("mat")?.map(|v| v.extract()).transpose()?;
    if let Some(ref n) = name {
        if pick_library_paint(scene, brush, n, None)? {
            return Ok(());
        }
        recipe.set_item("name", n)?;
    }
    pick_library_paint(scene, brush, "", Some(&recipe))?;
    if name.is_none() {
        brush.setattr("current_material", py.None())?;
        brush.setattr("_library_pick", py.None())?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ral_rgb, m)?)?;
    m.add_function(wrap_pyfunction!(recoloured_name, m)?)?;
    m.add_function(wrap_pyfunction!(edit_swatch, m)?)?;
    m.add_function(wrap_pyfunction!(brush_inspector, m)?)?;
    m.add_function(wrap_pyfunction!(load_recent_materials, m)?)?;
    m.add_function(wrap_pyfunction!(save_recent_materials, m)?)?;
    m.add_function(wrap_pyfunction!(recent_swatches, m)?)?;
    m.add_function(wrap_pyfunction!(pick_recent, m)?)?;
    m.add_class::<PaintRecents>()
}
