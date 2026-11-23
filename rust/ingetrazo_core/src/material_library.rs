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

/// The material editor's fields and actions belong to the Project inspector.
type Sections = Vec<(String, Vec<(String, String)>)>;

#[pyfunction]
pub(crate) fn material_inspector(
    mat: &Bound<'_, PyAny>,
    uses: usize,
) -> PyResult<(String, Sections)> {
    let name: String = mat.getattr("name")?.extract()?;
    let key = format!("m:{name}");
    let attr = |field| mat.getattr(field);
    let data: Value = serde_json::from_str(
        &mat.py()
            .import("json")?
            .call_method1("dumps", (mat.call_method0("to_dict")?,))?
            .extract::<String>()?,
    )
    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    let field = |kind: &str, id: &str, value: &str| {
        format!("\u{1d}{kind}\u{1d}libmat:{id}:{name}\u{1d}{value}")
    };
    let act = |verb: &str, label: &str| (format!("lib:{verb}:{key}"), label.to_owned());
    let tex = &data["texture"];
    let image = tex["path"].as_str().filter(|s| !s.is_empty());
    let switch = |id, labels: &str, on: bool| {
        field("choice", id, &format!("{labels}\u{1d}{}", u8::from(on)))
    };
    let mut base = vec![
        ("Name".into(), field("edit", "name", &name)),
        (
            "Base".into(),
            switch("base", "Colour|Image", image.is_some()),
        ),
    ];
    let mut actions = vec![act("apply", "Paint with it"), act("select", "Select uses")];
    if let Some(path) = image {
        base.push((
            "Image".into(),
            path.rsplit(['/', '\\']).next().unwrap_or(path).into(),
        ));
        let w = tex["sw"].as_f64().unwrap_or(1.);
        base.push((
            "Tile (m)".into(),
            field(
                "edit",
                "tile",
                &format!("{w} × {}", tex["sh"].as_f64().unwrap_or(w)),
            ),
        ));
        actions.push(act("image", "Other image…"));
    }
    let color: Option<(f64, f64, f64)> = attr("color")?.extract()?;
    let (r, g, b) = color.unwrap_or(if image.is_some() {
        (1., 1., 1.)
    } else {
        (0.8, 0.8, 0.8)
    });
    let hex = format!(
        "#{:02x}{:02x}{:02x}",
        (r * 255.) as u8,
        (g * 255.) as u8,
        (b * 255.) as u8
    );
    base.push((
        if image.is_some() { "Tint" } else { "Colour" }.into(),
        field("color", "color", &hex),
    ));
    let pbr = &data["pbr"];
    let enabled = pbr.as_object().is_some_and(|m| !m.is_empty());
    let mut surface = vec![("PBR surface".into(), switch("pbr", "Off|On", enabled))];
    let mut maps = vec![];
    if enabled {
        for (id, caption, map, limit) in [
            ("metallic", "Metallic", "metallic_map", 100),
            ("roughness", "Roughness", "roughness_map", 100),
            ("normal_scale", "Normal", "normal_map", 200),
            ("ao_strength", "Occlusion", "ao_map", 100),
        ] {
            let val = (pbr[id].as_f64().unwrap_or(0.) * 100.).round();
            surface.push((
                caption.into(),
                field(
                    "slider",
                    &format!("percent_{id}"),
                    &format!("{val}\u{1d}{limit}"),
                ),
            ));
            let has_map = pbr[map]["path"].is_string();
            let verb = if has_map { "unmap" } else { "map" };
            let label = if has_map {
                format!("Clear {} map", caption.to_lowercase())
            } else {
                format!("{caption} map…")
            };
            maps.push((format!("lib:{verb}:{key}:{map}"), label));
            if let Some(path) = pbr[map]["path"].as_str() {
                surface.push((
                    format!("{caption} map"),
                    path.rsplit(['/', '\\']).next().unwrap_or(path).into(),
                ));
            }
        }
    }
    let opacity = (data["opacity"].as_f64().unwrap_or(1.) * 100.).round();
    let mut sections = vec![
        ("Material".into(), base),
        ("Actions".into(), actions),
        ("Surface".into(), surface),
    ];
    if !maps.is_empty() {
        sections.push(("Actions".into(), maps));
    }
    sections.push((
        "Look".into(),
        vec![
            (
                "Opacity".into(),
                field("slider", "opacity", &format!("{opacity}\u{1d}100")),
            ),
            (
                "Finish".into(),
                data["finish"].as_str().unwrap_or("from the name").into(),
            ),
            (
                "Used by".into(),
                format!("{uses} object{}", if uses == 1 { "" } else { "s" }),
            ),
        ],
    ));
    sections.push((
        "Actions".into(),
        vec![
            act("duplicate", "Duplicate"),
            act("keep", "Add to My library"),
            act("delete", "Delete…"),
        ],
    ));
    Ok((name, sections))
}

#[pyfunction]
pub(crate) fn material_surface_input<'py>(
    py: Python<'py>,
    field: &str,
    text: &str,
) -> PyResult<Bound<'py, PyDict>> {
    let fields = PyDict::new(py);
    if field == "pbr" {
        if text == "On" {
            fields.set_item("metallic", 0.)?;
            fields.set_item("roughness", 0.5)?;
        } else if text == "Off" {
            for key in py
                .import("core.material_commands")?
                .getattr("PBR_KEYS")?
                .try_iter()?
            {
                fields.set_item(key?, py.None())?;
            }
        } else {
            return Err(pyo3::exceptions::PyValueError::new_err("Choose On or Off"));
        }
    } else if let Some(key) = field.strip_prefix("percent_") {
        let max = match key {
            "metallic" | "roughness" | "ao_strength" => 100.,
            "normal_scale" => 200.,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Unknown surface control",
                ))
            }
        };
        let val = text
            .parse::<f64>()
            .ok()
            .filter(|v| (0.0..=max).contains(v))
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("Invalid surface percentage"))?;
        fields.set_item(key, val / 100.)?;
    }
    Ok(fields)
}
