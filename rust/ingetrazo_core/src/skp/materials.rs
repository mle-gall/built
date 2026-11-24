// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve named materials, tiled images and metallic-roughness channels.
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

fn attr<'py>(row: &Bound<'py, PyAny>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    Ok(row
        .getattr_opt(name)?
        .unwrap_or_else(|| row.py().None().into_bound(row.py())))
}
fn flag(row: &Bound<'_, PyAny>, name: &str) -> PyResult<bool> {
    attr(row, name)?.is_truthy()
}
fn name(row: &Bound<'_, PyAny>, field: &str) -> PyResult<String> {
    Ok(attr(row, field)?
        .extract::<Option<String>>()?
        .unwrap_or_default())
}
fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or_default()
}

struct Images<'py> {
    adapter: Bound<'py, PyAny>,
    source: Bound<'py, PyAny>,
    directory: Option<PathBuf>,
}

impl Images<'_> {
    fn write(&mut self, filename: &str, id: &str, data: &[u8]) -> PyResult<Option<String>> {
        if self.directory.is_none() {
            self.directory = Some(
                self.adapter
                    .call_method1("_texture_dir", (&self.source,))?
                    .extract()?,
            );
        }
        let dir = self.directory.as_ref().unwrap();
        let safe = basename(filename);
        if safe.is_empty()
            || safe == "."
            || safe == ".."
            || Path::new(safe).components().count() != 1
        {
            return Ok(None);
        }
        let path = self.adapter.py().detach(|| -> std::io::Result<PathBuf> {
            let mut path = dir.join(safe);
            if path.is_file() && fs::read(&path)? != data {
                path = dir.join(format!("{}_{}", basename(id), safe));
            }
            if !path.is_file() || fs::read(&path)? != data {
                fs::write(&path, data)?;
            }
            Ok(path)
        });
        Ok(path.ok().map(|p| p.to_string_lossy().into_owned()))
    }

    fn pbr<'py>(
        &mut self,
        recipe: &Bound<'py, PyAny>,
        id: &str,
    ) -> PyResult<Option<Bound<'py, PyDict>>> {
        if recipe.is_none() {
            return Ok(None);
        }
        let py = recipe.py();
        let out = PyDict::new(py);
        for (gate, channel, factor, slot) in [
            (
                "metalness_enabled",
                "metallic",
                "metallic_factor",
                "metallic",
            ),
            (
                "roughness_enabled",
                "roughness",
                "roughness_factor",
                "roughness",
            ),
            ("normal_enabled", "normal", "normal_scale", "normal_scale"),
            ("ao_enabled", "ao", "ao_strength", "ao_strength"),
        ] {
            if !flag(recipe, gate)? {
                continue;
            }
            let tex = attr(recipe, &format!("{channel}_texture"))?;
            let mut has_map = false;
            if !tex.is_none() {
                let mut data = attr(&tex, "data")?;
                if data.is_truthy()? {
                    let mut filename = name(&tex, "filename")?;
                    if channel == "roughness" && flag(recipe, "roughness_texture_inverted")? {
                        data = self.adapter.call_method1("_inverted_png", (data,))?;
                        let base = basename(&filename);
                        let stem = base.rsplit_once('.').map_or(base, |(stem, _)| stem);
                        filename = format!(
                            "{}_inverted.png",
                            if stem.is_empty() { channel } else { stem }
                        );
                    }
                    if filename.is_empty() {
                        filename = format!("map_{id}.png");
                    }
                    if !data.is_none() {
                        if let Some(path) =
                            self.write(&filename, id, data.cast::<PyBytes>()?.as_bytes())?
                        {
                            let map = PyDict::new(py);
                            map.set_item("path", path)?;
                            if channel == "normal"
                                && attr(recipe, "normal_style")?.extract::<Option<i64>>()?
                                    == Some(1)
                            {
                                map.set_item("flip_y", true)?;
                            }
                            out.set_item(format!("{channel}_map"), map)?;
                            has_map = true;
                        }
                    }
                }
            }
            if has_map || matches!(channel, "metallic" | "roughness") {
                let n = recipe.getattr(factor)?.extract::<f64>()?;
                out.set_item(
                    slot,
                    if channel == "normal" {
                        n.max(0.)
                    } else {
                        n.clamp(0., 1.)
                    },
                )?;
            }
        }
        Ok((!out.is_empty()).then_some(out))
    }
}

fn size(tex: &Bound<'_, PyAny>, field: &str) -> Option<f64> {
    attr(tex, field)
        .ok()?
        .extract::<f64>()
        .ok()
        .filter(|&v| v > 1e-6 && v < 1e5)
}

#[pyfunction]
pub fn skp_materials<'py>(
    model: &Bound<'py, PyAny>,
    source: Bound<'py, PyAny>,
) -> PyResult<(Bound<'py, PyDict>, Bound<'py, PyList>)> {
    let py = model.py();
    let attrs = PyDict::new(py);
    let registry = PyDict::new(py);
    let list = PyList::empty(py);
    let ids = attr(model, "materials_by_id")?;
    if ids.is_none() {
        return Ok((attrs, list));
    }
    let ids = ids.cast::<PyDict>()?;
    let module = py.import("core.materials")?;
    let mut images = Images {
        adapter: py.import("formats.skp_openskp")?.into_any(),
        source,
        directory: None,
    };
    let copy = py.import("copy")?.getattr("deepcopy")?;
    let mut cached: HashMap<usize, Option<Bound<PyDict>>> = HashMap::new();
    for (key, mat) in ids {
        let id = key.str()?.to_string();
        let tex = attr(&mat, "texture")?;
        let color = attr(&mat, "color")?;
        let entry = PyDict::new(py);
        if !tex.is_none() {
            let mut data = attr(&tex, "data")?;
            if data.is_truthy()? {
                let filename = name(&tex, "filename")?;
                let mut file = basename(&filename).to_owned();
                if flag(&mat, "colorized")?
                    && images
                        .adapter
                        .call_method1("_needs_tint", (&data, &color))?
                        .is_truthy()?
                {
                    let mode = attr(&mat, "colorize_type")?
                        .extract::<Option<i64>>()?
                        .unwrap_or(0);
                    data = images
                        .adapter
                        .call_method1("_cached_colorize", (data, &color, mode))?;
                    file = format!(
                        "{id}_{}",
                        if file.is_empty() {
                            "material.png"
                        } else {
                            &file
                        }
                    );
                }
                if file.is_empty() {
                    file = format!("material_{id}.png");
                }
                if let Some(path) = images.write(&file, &id, data.cast::<PyBytes>()?.as_bytes())? {
                    let w = size(&tex, "width");
                    let h = size(&tex, "height");
                    let t = PyDict::new(py);
                    t.set_item("path", path)?;
                    t.set_item("sw", w.or(h).map_or(1., |n| n * 0.0254))?;
                    t.set_item("sh", h.or(w).map_or(1., |n| n * 0.0254))?;
                    entry.set_item("texture", t)?;
                }
            }
        }
        if entry.is_empty() {
            if color.is_none() || color.len()? < 3 {
                continue;
            }
            let rgb = [0, 1, 2].map(|i| color.get_item(i)?.extract::<f64>().map(|v| v / 255.));
            entry.set_item("color", rgb.into_iter().collect::<PyResult<Vec<_>>>()?)?;
        }
        let opacity = attr(&mat, "transparency")?
            .extract::<Option<f64>>()?
            .unwrap_or(1.);
        if opacity < 0.999 {
            entry.set_item("opacity", opacity)?;
        }
        let k = mat.as_ptr() as usize;
        if let std::collections::hash_map::Entry::Vacant(slot) = cached.entry(k) {
            slot.insert(images.pbr(&attr(&mat, "pbr")?, &id)?);
        }
        if let Some(pbr) = &cached[&k] {
            entry.set_item("pbr", copy.call1((pbr,))?)?;
        }
        attrs.set_item(key, entry)?;
    }
    for (key, mat) in ids {
        let Some(entry) = attrs.get_item(key)? else {
            continue;
        };
        let label = name(&mat, "name")?;
        if label.trim().is_empty() {
            continue;
        }
        let recipe = entry.cast::<PyDict>()?.copy()?;
        recipe.set_item("name", label.trim())?;
        let record = module
            .getattr("Material")?
            .call_method1("from_dict", (recipe,))?;
        entry.set_item("mat", module.call_method1("register", (&registry, record))?)?;
    }
    for (_, mat) in &registry {
        list.append(mat.call_method0("to_dict")?)?;
    }
    Ok((attrs, list))
}

#[pyfunction]
pub fn skp_read_pbr<'py>(
    zip: &Bound<'py, PyAny>,
    xml_name: &str,
    material: &Bound<'py, PyAny>,
    ns: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = zip.py();
    let workflow = material
        .call_method1("get", ("workflow", "0"))?
        .extract::<String>()?
        .trim()
        .parse::<u32>()
        .unwrap_or(0);
    if workflow != 1 {
        return Ok(None);
    }
    let block = material.call_method1("find", ("mat:pbrMR", ns))?;
    if block.is_none() {
        return Ok(None);
    }
    let fields = PyDict::new(py);
    fields.set_item("workflow", workflow)?;
    let child = |tag: &str| block.call_method1("find", (format!("mat:{tag}"), ns));
    let text = |tag: &str| -> PyResult<String> {
        let el = child(tag)?;
        if el.is_none() {
            Ok(String::new())
        } else {
            name(&el, "text")
        }
    };
    let number = |s: &str, default: f64| {
        s.trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .unwrap_or(default)
    };
    for (tag, key) in [
        ("enable_metalness", "metalness_enabled"),
        ("enable_roughness", "roughness_enabled"),
        ("roughness_texture_invert", "roughness_texture_inverted"),
        ("enable_normal", "normal_enabled"),
        ("enable_occlusion", "ao_enabled"),
    ] {
        fields.set_item(key, text(tag)?.trim() == "1")?;
    }
    let round = py.import("builtins")?.getattr("round")?;
    for (tag, key) in [
        ("metallicFactor", "metallic_factor"),
        ("roughnessFactor", "roughness_factor"),
        ("normalScale", "normal_scale"),
        ("occlusionStrength", "ao_strength"),
    ] {
        fields.set_item(key, round.call1((number(&text(tag)?, 1.), 6))?)?;
    }
    fields.set_item(
        "normal_style",
        round.call1((number(&text("normalMapStyle")?, 0.),))?,
    )?;
    let scale = |axis| -> PyResult<f64> {
        Ok(number(
            &block.call_method1("get", (axis, ""))?.extract::<String>()?,
            0.,
        ))
    };
    let (x, y) = (scale("xScale")?, scale("yScale")?);
    let dir = xml_name.rsplit_once('/').map_or(xml_name, |(p, _)| p);
    let names = py
        .get_type::<pyo3::types::PySet>()
        .call1((zip.call_method0("namelist")?,))?;
    for (tag, key) in [
        ("metallic_texture", "metallic_texture"),
        ("roughness_texture", "roughness_texture"),
        ("normal_texture", "normal_texture"),
        ("occlusion_texture", "ao_texture"),
    ] {
        fields.set_item(key, py.None())?;
        let el = child(tag)?;
        if el.is_none() {
            continue;
        }
        let path = el
            .call_method1("get", ("path", ""))?
            .extract::<String>()?
            .replace('\\', "/");
        let path = path.trim_start_matches("./");
        let filename = el
            .call_method1("get", ("file_name", ""))?
            .extract::<String>()?;
        for candidate in [
            (!path.is_empty()).then(|| format!("{dir}/{path}")),
            (!path.is_empty()).then(|| path.to_owned()),
            (!filename.is_empty()).then(|| format!("{dir}/{filename}")),
        ]
        .into_iter()
        .flatten()
        {
            if !names.contains(&candidate)? {
                continue;
            }
            py.import("openskp._core")?
                .call_method1("_validate_zip_entry_size", (zip, &candidate))?;
            let kw = PyDict::new(py);
            kw.set_item(
                "filename",
                if filename.is_empty() {
                    basename(&candidate)
                } else {
                    &filename
                },
            )?;
            kw.set_item("width", x)?;
            kw.set_item("height", y)?;
            kw.set_item("data", zip.call_method1("read", (candidate,))?)?;
            fields.set_item(
                key,
                py.import("openskp.model")?
                    .getattr("Texture")?
                    .call((), Some(&kw))?,
            )?;
            break;
        }
    }
    Ok(Some(
        py.import("types")?
            .getattr("SimpleNamespace")?
            .call((), Some(&fields))?,
    ))
}
