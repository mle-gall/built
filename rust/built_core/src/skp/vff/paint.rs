// SPDX-License-Identifier: GPL-3.0-or-later
//! Read material and style XML while the VFF member directory is available.
use super::{archive::Archive, namespace};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};
use roxmltree::{Document, Node};

const MATERIAL: &str = "http://sketchup.google.com/schemas/sketchup/1.0/material";
const STYLE: &str = "http://sketchup.google.com/schemas/sketchup/1.0/style";
const TYPES: &str = "http://sketchup.google.com/schemas/1.0/types";
fn child<'a, 'd>(el: Node<'a, 'd>, name: &str) -> Option<Node<'a, 'd>> {
    el.children().find(|n| n.has_tag_name((MATERIAL, name)))
}
fn number(val: Option<&str>, default: f64) -> f64 {
    val.and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|n| n.is_finite())
        .unwrap_or(default)
}
fn texture<'py>(
    py: Python<'py>,
    zip: &mut Archive,
    filename: &str,
    member: Option<&str>,
    x: f64,
    y: f64,
) -> PyResult<Bound<'py, PyAny>> {
    let out = namespace(py)?;
    out.setattr("filename", filename)?;
    out.setattr("width", x)?;
    out.setattr("height", y)?;
    let bytes = member
        .map(|name| py.detach(|| zip.read(name)))
        .transpose()?;
    out.setattr("data", bytes.as_deref().map(|b| PyBytes::new(py, b)))?;
    Ok(out)
}
fn pbr<'py>(
    py: Python<'py>,
    zip: &mut Archive,
    mat: Node,
    folder: &str,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    if mat
        .attribute("workflow")
        .and_then(|s| s.trim().parse::<u32>().ok())
        != Some(1)
    {
        return Ok(None);
    }
    let Some(m) = child(mat, "pbrMR") else {
        return Ok(None);
    };
    let recipe = namespace(py)?;
    let value = |tag: &str| child(m, tag).and_then(|n| n.text());
    for (tag, key) in [
        ("enable_metalness", "metalness_enabled"),
        ("enable_roughness", "roughness_enabled"),
        ("roughness_texture_invert", "roughness_texture_inverted"),
        ("enable_normal", "normal_enabled"),
        ("enable_occlusion", "ao_enabled"),
    ] {
        recipe.setattr(key, value(tag).is_some_and(|s| s.trim() == "1"))?;
    }
    let round = py.import("builtins")?.getattr("round")?;
    for (tag, key) in [
        ("metallicFactor", "metallic_factor"),
        ("roughnessFactor", "roughness_factor"),
        ("normalScale", "normal_scale"),
        ("occlusionStrength", "ao_strength"),
    ] {
        recipe.setattr(key, round.call1((number(value(tag), 1.), 6))?)?;
    }
    recipe.setattr(
        "normal_style",
        round.call1((number(value("normalMapStyle"), 0.),))?,
    )?;
    for (tag, key) in [
        ("metallic_texture", "metallic_texture"),
        ("roughness_texture", "roughness_texture"),
        ("normal_texture", "normal_texture"),
        ("occlusion_texture", "ao_texture"),
    ] {
        recipe.setattr(key, py.None())?;
        if let Some(t) = child(m, tag) {
            let path = t.attribute("path").unwrap_or("").replace('\\', "/");
            let path = path.trim_start_matches("./");
            let filename = t.attribute("file_name").unwrap_or("");
            let candidates = [
                format!("{folder}/{path}"),
                path.to_owned(),
                format!("{folder}/{filename}"),
            ];
            if let Some(name) = candidates
                .iter()
                .find(|n| !n.ends_with('/') && zip.contains(n))
            {
                recipe.setattr(
                    key,
                    texture(
                        py,
                        zip,
                        if filename.is_empty() {
                            name.rsplit('/').next().unwrap_or(name)
                        } else {
                            filename
                        },
                        Some(name),
                        number(m.attribute("xScale"), 0.),
                        number(m.attribute("yScale"), 0.),
                    )?,
                )?;
            }
        }
    }
    Ok(Some(recipe))
}

pub(super) fn read<'py>(
    py: Python<'py>,
    zip: &mut Archive,
    material_names: &Bound<'py, PyDict>,
    layer_colors: &Bound<'py, PyDict>,
    model: &Bound<'py, PyAny>,
) -> PyResult<()> {
    let named = PyDict::new(py);
    let by_folder = PyDict::new(py);
    let names: Vec<_> = zip
        .names
        .iter()
        .filter(|n| {
            (n.starts_with("materials/") && n.ends_with("material.xml"))
                || (n.starts_with("styles/") && n.ends_with("style.xml"))
        })
        .cloned()
        .collect();
    let styles = PyList::empty(py);
    for name in names {
        let xml = py.detach(|| zip.read(&name))?;
        let Ok(text) = std::str::from_utf8(&xml) else {
            continue;
        };
        let Ok(doc) = Document::parse(text.trim_start_matches('\u{feff}')) else {
            continue;
        };
        if name.starts_with("styles/") {
            let Some(style) = doc.descendants().find(|n| n.has_tag_name((STYLE, "style"))) else {
                continue;
            };
            let s = namespace(py)?;
            s.setattr("name", style.attribute("name").unwrap_or(""))?;
            for item in style.children().filter(|n| n.has_tag_name((STYLE, "item"))) {
                let attr = match item.attribute("id") {
                    Some("2002") => "front_color",
                    Some("2003") => "back_color",
                    _ => continue,
                };
                if let Some(c) = item
                    .children()
                    .find(|n| n.has_tag_name((TYPES, "variant")))
                    .and_then(|n| n.text())
                    .and_then(|s| s.trim().parse::<i64>().ok())
                {
                    s.setattr(
                        attr,
                        (
                            (c & 255) as u8,
                            ((c >> 8) & 255) as u8,
                            ((c >> 16) & 255) as u8,
                        ),
                    )?;
                }
            }
            styles.append(s)?;
            continue;
        }
        let Some(mat) = doc
            .descendants()
            .find(|n| n.has_tag_name((MATERIAL, "material")))
        else {
            continue;
        };
        let label = mat.attribute("name").unwrap_or("unknown");
        let channel = |k| mat.attribute(k).unwrap_or("128").parse::<i32>().ok();
        let (Some(r), Some(g), Some(b)) = (
            channel("colorRed"),
            channel("colorGreen"),
            channel("colorBlue"),
        ) else {
            continue;
        };
        if let Some(layer) = label.strip_prefix("Layer_") {
            layer_colors.set_item(layer, (r, g, b))?;
        }
        let folder = name.rsplit_once('/').unwrap().0;
        let m = namespace(py)?;
        m.setattr("name", label)?;
        m.setattr("color", (r, g, b, 255))?;
        m.setattr(
            "transparency",
            if mat.attribute("useTrans") == Some("1") {
                (1. - number(mat.attribute("trans"), 0.)).clamp(0., 1.)
            } else {
                1.
            },
        )?;
        m.setattr("colorized", mat.attribute("type") == Some("2"))?;
        m.setattr(
            "colorize_type",
            mat.attribute("colorizeType")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0),
        )?;
        m.setattr("texture", py.None())?;
        if let Some(t) = child(mat, "texture") {
            let file = t.attribute("textureFilename").unwrap_or("");
            let first = format!("{folder}/{file}");
            let image = child(t, "images")
                .and_then(|n| child(n, "image"))
                .and_then(|n| n.attribute("path"))
                .unwrap_or("")
                .trim_start_matches(['.', '/']);
            let source = if !file.is_empty() && zip.contains(&first) {
                Some(first)
            } else {
                let prefix = format!("{folder}/");
                zip.names
                    .iter()
                    .find(|n| n.starts_with(&prefix) && !n.to_ascii_lowercase().ends_with(".xml"))
                    .cloned()
                    .or_else(|| {
                        [image.to_string(), format!("{folder}/{image}")]
                            .into_iter()
                            .find(|s| !s.ends_with('/') && zip.contains(s))
                    })
            };
            let filename = if file.is_empty() {
                source
                    .as_deref()
                    .and_then(|s| s.rsplit('/').next())
                    .unwrap_or("")
            } else {
                file
            };
            m.setattr(
                "texture",
                texture(
                    py,
                    zip,
                    filename,
                    source.as_deref(),
                    number(t.attribute("xScale"), 0.),
                    number(t.attribute("yScale"), 0.),
                )?,
            )?;
        }
        m.setattr("pbr", pbr(py, zip, mat, folder)?)?;
        named.set_item(label, &m)?;
        by_folder.set_item(name.split('/').nth(1).unwrap_or(""), m)?;
    }
    let ids = PyDict::new(py);
    for (id, name) in material_names {
        if let Some(mat) = named.get_item(&name)?.or(by_folder.get_item(&name)?) {
            ids.set_item(id, mat)?;
        }
    }
    model.setattr("styles", styles)?;
    model.setattr("materials", named.values())?;
    model.setattr("materials_by_id", ids)
}
