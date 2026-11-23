// SPDX-License-Identifier: GPL-3.0-or-later
//! Bundled library catalogue and selection of a recipe for the live brush.
use pyo3::{prelude::*, types::PyDict};
use serde_json::{json, Value};
use std::path::Path;

fn category(id: &str) -> &str {
    match id {
        "roof" => "Roofing",
        "floor" => "Flooring",
        "misc" => "Miscellaneous",
        "brick" => "Brick",
        "concrete" => "Concrete",
        "stone" => "Stone",
        "wood" => "Wood",
        "metal" => "Metal",
        "ground" => "Ground",
        "glass" => "Glass",
        "water" => "Water",
        "wall" => "Wall",
        "wallpaper" => "Wallpaper",
        "fabric" => "Fabric",
        "rug" => "Rug",
        "sky" => "Sky",
        other => other,
    }
}

fn manifest(path: &Path) -> PyResult<Value> {
    match std::fs::read(path) {
        Ok(data) => serde_json::from_slice(&data).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("{}: {e}", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(e) => Err(e.into()),
    }
}

fn rows<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value[key].as_array().into_iter().flatten()
}

#[pyfunction]
pub(crate) fn bundled_materials(resources: std::path::PathBuf, language: &str) -> PyResult<String> {
    let mut items = Vec::new();
    let textures = resources.join("textures");
    for cat in rows(&manifest(&textures.join("library.json"))?, "categories") {
        let Some(id) = cat["id"].as_str() else {
            continue;
        };
        for (i, item) in rows(cat, "items").enumerate() {
            let (Some(file), Some(name)) = (item["file"].as_str(), item["name"].as_str()) else {
                continue;
            };
            let path = textures.join("library").join(file);
            let text = name.replace(['_', '-'], " ");
            let mut letters = text.trim().chars();
            let label: String = letters
                .next()
                .into_iter()
                .flat_map(char::to_uppercase)
                .chain(letters)
                .collect();
            let recipe = json!({"name": name, "opacity": item["opacity"], "texture": {
                "path": path, "sw": item["sw"].as_f64().unwrap_or(1.0), "sh": item["sh"].as_f64().unwrap_or(1.0), "rot": 0.0}});
            items.push(json!({"key":format!("lib:{id}:{i}"),"name":name,"label":label,"category":category(id),
                "source":"bundled","base":"image","uses":0,"hex":"","thumb":path,"verbs":["apply"],"recipe":recipe}));
        }
    }
    for family in rows(&manifest(&resources.join("colors/ral.json"))?, "families") {
        for entry in rows(family, "colors") {
            let Some(code) = entry["code"].as_str() else {
                continue;
            };
            let local = if language.starts_with("es") {
                entry["name_es"].as_str()
            } else {
                None
            };
            let name = format!(
                "{code} {}",
                local.or_else(|| entry["name"].as_str()).unwrap_or(code)
            );
            let Some(rgb) = entry["rgb"].as_array().filter(|v| v.len() == 3) else {
                continue;
            };
            let scale = if rgb.iter().any(|c| c.as_f64().unwrap_or(0.) > 1.) {
                255.
            } else {
                1.
            };
            let rgb: Vec<_> = rgb
                .iter()
                .map(|c| (c.as_f64().unwrap_or_default() / scale).clamp(0., 1.))
                .collect();
            let hex = format!(
                "#{:02x}{:02x}{:02x}",
                (rgb[0] * 255.) as u8,
                (rgb[1] * 255.) as u8,
                (rgb[2] * 255.) as u8
            );
            items.push(json!({"key":format!("ral:{code}"),"name":name,"short":code,"category":"Colours (RAL)",
                "source":"bundled","base":"colour","uses":0,"hex":hex,"thumb":null,"verbs":["apply"],
                "recipe":{"name":name,"color":rgb}}));
        }
    }
    Ok(Value::Array(items).to_string())
}

/// Pick paint without creating a document edit. Registration belongs to the
/// first paint stroke. Preserve the recipe's alpha, PBR, rotation and tint.
#[pyfunction]
#[pyo3(signature=(scene, brush, *, name="", recipe=None))]
pub(crate) fn pick_library_paint(
    scene: &Bound<'_, PyAny>,
    brush: &Bound<'_, PyAny>,
    name: &str,
    recipe: Option<&Bound<'_, PyDict>>,
) -> PyResult<bool> {
    let py = scene.py();
    let mat = if let Some(recipe) = recipe {
        if recipe
            .get_item("name")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false))
        {
            py.import("core.materials")?
                .getattr("Material")?
                .call_method1("from_dict", (recipe,))?
        } else {
            py.import("core.material_commands")?.call_method1(
                "material_for_paint",
                (
                    scene,
                    recipe.get_item("color")?,
                    recipe.get_item("texture")?,
                    recipe.get_item("opacity")?,
                ),
            )?
        }
    } else {
        let mat = scene.getattr("materials")?.call_method1("get", (name,))?;
        if mat.is_none() {
            return Ok(false);
        }
        mat
    };
    let color = mat.getattr("color")?;
    let texture = mat.getattr("texture")?;
    brush.setattr(
        "current_color",
        if color.is_none() {
            (1., 1., 1.).into_pyobject(py)?.into_any()
        } else {
            color
        },
    )?;
    brush.setattr(
        "current_texture",
        if texture.is_none() {
            None
        } else {
            Some(texture.cast::<PyDict>()?.copy()?)
        },
    )?;
    brush.setattr("current_texture_plane", py.None())?;
    brush.setattr("current_opacity", mat.getattr("opacity")?)?;
    brush.setattr("current_material", mat)?;
    brush.setattr("current_is_default", false)?;
    Ok(true)
}
