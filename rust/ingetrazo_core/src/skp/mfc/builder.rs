// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve references only after the archive has registered every entity.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

fn val<'py>(dict: &Bound<'py, PyAny>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    dict.call_method1("get", (key,))
}
fn number_or_none<'py>(v: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    Ok(if v.is_truthy()? {
        v
    } else {
        v.py().None().into_bound(v.py())
    })
}

fn attribute_text(value: &Bound<'_, PyAny>) -> PyResult<String> {
    if value.is_none() {
        return Ok(String::new());
    }
    if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
        let values = value
            .try_iter()?
            .map(|v| attribute_text(&v?))
            .collect::<PyResult<Vec<_>>>()?;
        return Ok(values.join(","));
    }
    value.str()?.extract()
}

fn attributes<'py>(container: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(container.py());
    if let Ok(d) = container.cast::<PyDict>() {
        if let Some(children) = d.get_item("children")? {
            for child in children.try_iter()? {
                let Ok(row) = child?.get_item(1)?.cast_into::<PyDict>() else {
                    continue;
                };
                if let Some(name) = row.get_item("name")?.filter(|n| !n.is_none()) {
                    if name.is_truthy()? {
                        let entries = row
                            .get_item("entries")?
                            .map(|v| v.cast_into::<PyDict>())
                            .transpose()?;
                        result.set_item(
                            name,
                            match entries {
                                Some(items) => items.copy()?,
                                None => PyDict::new(container.py()),
                            },
                        )?;
                    }
                }
            }
        }
    }
    Ok(result)
}

fn builder<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
    let record = py.import("types")?.getattr("SimpleNamespace")?.call0()?;
    for name in [
        "vertices",
        "edges",
        "edge_flags",
        "edge_layers",
        "edge_curves",
        "arc_curves",
        "faces",
    ] {
        record.setattr(name, PyDict::new(py))?;
    }
    for name in [
        "instances",
        "section_planes",
        "texts",
        "dimensions",
        "construction_lines",
        "construction_points",
    ] {
        record.setattr(name, PyList::empty(py))?;
    }
    Ok(record)
}

fn edge(
    b: &Bound<'_, PyAny>,
    id: &Bound<'_, PyAny>,
    e: &Bound<'_, PyAny>,
    slots: &Bound<'_, PyDict>,
) -> PyResult<()> {
    let edges = b.getattr("edges")?;
    if edges.contains(id)? {
        return Ok(());
    }
    let a = e.get_item("v1")?;
    let z = e.get_item("v2")?;
    let vertices = b.getattr("vertices")?;
    for vid in [&a, &z] {
        if !vertices.contains(vid)? {
            if let Some(rec) = slots.get_item(vid)? {
                let v = rec.get_item(2)?;
                if !v.is_none() {
                    vertices.set_item(vid, v.get_item("xyz")?)?;
                }
            }
        }
    }
    edges.set_item(id, (a, z))?;
    let db = e.get_item("db")?;
    let mut flags = 0u8;
    for (field, mask) in [("hidden", 1), ("soft", 8), ("smooth", 16)] {
        if val(&db, field)?.is_truthy()? {
            flags |= mask;
        }
    }
    if flags != 0 {
        b.getattr("edge_flags")?.set_item(id, flags)?;
    }
    let layer = val(&db, "layer")?;
    if layer.is_truthy()? {
        b.getattr("edge_layers")?.set_item(id, layer)?;
    }
    let curve = val(e, "curve")?;
    if !curve.is_none() {
        b.getattr("edge_curves")?.set_item(id, &curve)?;
        if let Ok(cv) = val(e, "curve_value")?.cast_into::<PyDict>() {
            if val(&cv, "k")?.eq("arccurve")? {
                b.getattr("arc_curves")?
                    .set_item(curve, cv.get_item("params")?)?;
            }
        }
    }
    Ok(())
}

#[pyfunction]
pub(super) fn fill(
    b: &Bound<'_, PyAny>,
    ents: &Bound<'_, PyAny>,
    slots: &Bound<'_, PyDict>,
) -> PyResult<()> {
    let py = b.py();
    let list = py.get_type::<PyList>();
    for item in ents.try_iter()? {
        let item = item?;
        let slot = item.get_item(0)?;
        let Ok(v) = item.get_item(2)?.cast_into::<PyDict>() else {
            continue;
        };
        let Some(kind) = v.get_item("k")? else {
            continue;
        };
        let row = PyDict::new(py);
        let kind = kind.extract::<String>()?;
        let target = match kind.as_str() {
            "edge" => {
                edge(b, &slot, &v, slots)?;
                continue;
            }
            "arccurve" => {
                b.getattr("arc_curves")?
                    .set_item(slot, v.get_item("params")?)?;
                continue;
            }
            "face" => {
                let rings = PyList::empty(py);
                for ring in v.as_any().get_item("loops")?.try_iter()? {
                    let ring = ring?;
                    let coedges = PyList::empty(py);
                    for coedge in ring.get_item("uses")?.try_iter()? {
                        let coedge = coedge?;
                        let eid = coedge.get_item("edge")?;
                        if let Some(rec) = slots.get_item(&eid)? {
                            let e = rec.get_item(2)?;
                            if e.is_none() {
                                continue;
                            }
                            edge(b, &eid, &e, slots)?;
                            coedges.append((
                                eid,
                                if coedge.get_item("sense")?.is_truthy()? {
                                    -1
                                } else {
                                    1
                                },
                            ))?;
                        }
                    }
                    rings.append(coedges)?;
                }
                row.set_item("loops", rings)?;
                row.set_item(
                    "normal",
                    v.as_any()
                        .get_item("plane")?
                        .cast::<PyTuple>()?
                        .get_slice(0, 3),
                )?;
                let db = v.as_any().get_item("db")?;
                row.set_item("material_id", number_or_none(db.get_item("mat")?)?)?;
                row.set_item(
                    "back_material_id",
                    number_or_none(v.as_any().get_item("back_mat")?)?,
                )?;
                row.set_item("layer", val(&db, "layer")?)?;
                row.set_item("hidden", db.get_item("hidden")?.is_truthy()?)?;
                if let Some(attrs) = v
                    .get_item("attrs")?
                    .and_then(|v| v.cast_into::<PyDict>().ok())
                {
                    if let Some(children) = attrs.get_item("children")? {
                        for child in children.try_iter()? {
                            let ft = child?.get_item(1)?;
                            if ft.cast::<PyDict>().is_err() || !val(&ft, "k")?.eq("ftc")? {
                                continue;
                            }
                            for (dst, src) in
                                [("uv_transform", "front"), ("uv_transform_back", "back")]
                            {
                                row.set_item(dst, list.call1((ft.get_item(src)?,))?)?;
                            }
                            for (dst, src) in [
                                ("uv_projected", "front_projected"),
                                ("uv_projected_back", "back_projected"),
                            ] {
                                row.set_item(dst, val(&ft, src)?.is_truthy()?)?;
                            }
                        }
                    }
                }
                b.getattr("faces")?.set_item(slot, row)?;
                continue;
            }
            "instance" | "image" => {
                let image = kind == "image";
                let db = v.as_any().get_item("db")?;
                row.set_item("ref_idx", v.get_item("def")?)?;
                row.set_item(
                    "ref_guid",
                    if image {
                        val(&v, "guid")?
                    } else {
                        "".into_pyobject(py)?.into_any()
                    },
                )?;
                row.set_item(
                    "name",
                    if image {
                        "".into_pyobject(py)?.into_any()
                    } else {
                        v.as_any().get_item("name")?
                    },
                )?;
                row.set_item(
                    "matrix",
                    list.call1((v.as_any().get_item(if image { "xform" } else { "xf" })?,))?,
                )?;
                row.set_item("material_id", number_or_none(db.get_item("mat")?)?)?;
                row.set_item("layer_id", number_or_none(db.get_item("layer")?)?)?;
                row.set_item("hidden", db.get_item("hidden")?.is_truthy()?)?;
                row.set_item("children", PyList::empty(py))?;
                let properties = PyDict::new(py);
                if !image {
                    let attrs = attributes(&val(&v, "attrs")?)?;
                    if let Some(map) = attrs.get_item("dynamic_attributes")? {
                        for (k, value) in map.cast::<PyDict>()? {
                            properties.set_item(k, attribute_text(&value)?)?;
                        }
                    }
                    row.set_item("attribute_dictionaries", attrs)?;
                }
                row.set_item("properties", properties)?;
                row.set_item("persistent_id", val(&v, "persistent_id")?)?;
                "instances"
            }
            "dimension" | "text" | "sectionplane" => {
                row.set_item(
                    "hidden",
                    val(&v.as_any().get_item("db")?, "hidden")?.is_truthy()?,
                )?;
                if kind == "dimension" {
                    for key in ["text", "offset"] {
                        row.set_item(key, val(&v, key)?)?;
                    }
                    let refs = v.as_any().get_item("connect")?;
                    let endpoint = |i, field| -> PyResult<Bound<PyAny>> {
                        let p = val(&v, field)?;
                        if p.is_none() {
                            if let Some(rec) = slots.get_item(refs.get_item(i)?)? {
                                if let Ok(obj) = rec.get_item(2)?.cast_into::<PyDict>() {
                                    if let Some(xyz) = obj.get_item("xyz")? {
                                        return Ok(xyz);
                                    }
                                }
                            }
                        }
                        Ok(p)
                    };
                    let (a, z) = (endpoint(0, "a")?, endpoint(1, "b")?);
                    if !a.is_none() && !z.is_none() {
                        row.set_item("a", a)?;
                        row.set_item("b", z)?;
                    }
                    "dimensions"
                } else if kind == "text" {
                    for key in ["text", "p", "lp"] {
                        row.set_item(key, val(&v, key)?)?;
                    }
                    "texts"
                } else {
                    for key in ["name", "label", "plane"] {
                        row.set_item(key, val(&v, key)?)?;
                    }
                    "section_planes"
                }
            }
            "cline" => {
                for key in ["point", "direction", "start", "end"] {
                    row.set_item(key, val(&v, key)?)?;
                }
                "construction_lines"
            }
            "cpoint" => {
                row.set_item("position", val(&v, "pos")?)?;
                "construction_points"
            }
            _ => continue,
        };
        b.getattr(target)?.call_method1("append", (row,))?;
    }
    Ok(())
}

/// OpenSKP's model view uses the same field names in classic and modern files.
/// Resolve classic material, layer and definition slots before creating it.
#[pyfunction]
pub(super) fn read_legacy<'py>(
    py: Python<'py>,
    skp_path: std::path::PathBuf,
) -> PyResult<Bound<'py, PyDict>> {
    use std::collections::HashSet;
    let input = py.detach(|| std::fs::read(skp_path))?;
    let bytes = pyo3::types::PyBytes::new(py, &input);
    drop(input);
    let raw = bytes.as_bytes();
    let version = raw
        .windows(3)
        .skip(4)
        .position(|w| w == [0xff, 0xfe, 0xff])
        .and_then(|i| {
            let start = i + 8;
            let text = String::from_utf16_lossy(
                &raw.get(start..)?
                    .iter()
                    .take(96)
                    .copied()
                    .collect::<Vec<_>>()
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
            );
            Some(text.get(text.find('{')?..=text.find('}')?)?.to_owned())
        })
        .unwrap_or_else(|| "unknown".into());
    let result = super::archive::walk(&bytes)?;
    let ar = result.get_item(0)?;
    let root = result.get_item(1)?;
    let slots = ar.getattr("slots")?.cast_into::<PyDict>()?;
    let materials = PyDict::new(py);
    let mat_names = PyDict::new(py);
    for row in result.get_item(3)?.try_iter()? {
        let row = row?;
        let slot = row.get_item(0)?;
        let value = row.get_item(1)?;
        let mat = PyDict::new(py);
        let rgba = value.get_item("rgba")?;
        let color = PyDict::new(py);
        for (i, key) in ["r", "g", "b", "a"].into_iter().enumerate() {
            color.set_item(key, rgba.get_item(i)?)?;
        }
        mat.set_item("color", color)?;
        mat.set_item(
            "transparency",
            if val(&value, "use_opacity")?.is_truthy()? {
                (1.0 - val(&value, "opacity")?.extract::<f64>()?).clamp(0., 1.)
            } else {
                1.
            },
        )?;
        mat.set_item("colorized", val(&value, "colorized")?.is_truthy()?)?;
        mat.set_item(
            "colorize_type",
            val(&value, "colorize_type")?
                .extract::<Option<u32>>()?
                .unwrap_or(0),
        )?;
        let name = value.get_item("name")?;
        mat.set_item("name", &name)?;
        if value.contains("tex_dib")? {
            let dib = val(&value, "tex_dib")?;
            let pic = if let Some(src) = slots.get_item(dib)? {
                let rec = src.get_item(2)?;
                if rec.is_none() {
                    val(&value, "tex_data")?
                } else {
                    rec.get_item("data")?
                }
            } else {
                val(&value, "tex_data")?
            };
            let texture = PyDict::new(py);
            let mut filename = val(&value, "tex_file")?
                .extract::<Option<String>>()?
                .unwrap_or_default();
            if filename.is_empty() {
                let png = pic
                    .cast::<pyo3::types::PyBytes>()
                    .is_ok_and(|b| b.as_bytes().starts_with(b"\x89PNG"));
                filename = format!("{}.{}", name, if png { "png" } else { "jpg" });
            }
            texture.set_item("filename", filename)?;
            texture.set_item("x_scale", value.get_item("tex_w")?)?;
            texture.set_item("y_scale", value.get_item("tex_h")?)?;
            texture.set_item("data", pic)?;
            mat.set_item("texture", texture)?;
        }
        mat_names.set_item(slot, &name)?;
        materials.set_item(name, mat)?;
    }
    let names = PyDict::new(py);
    let colors = PyDict::new(py);
    let hidden = PyDict::new(py);
    for row in result.get_item(2)?.try_iter()? {
        let row = row?;
        let v = row.get_item(1)?;
        let name = v.get_item("name")?;
        let c = v.get_item("rgba")?;
        names.set_item(row.get_item(0)?, &name)?;
        colors.set_item(&name, (c.get_item(0)?, c.get_item(1)?, c.get_item(2)?))?;
        hidden.set_item(&name, val(&v, "hidden")?.is_truthy()?)?;
    }
    if !colors.contains("Layer0")? {
        colors.set_item("Layer0", (136, 136, 136))?;
        hidden.set_item("Layer0", false)?;
    }
    let mut images = HashSet::new();
    for (_, rec) in &slots {
        if rec.get_item(1)?.eq("CImage")? {
            let v = rec.get_item(2)?;
            if v.cast::<PyDict>().is_ok() {
                images.insert(v.get_item("def")?.extract::<u32>()?);
            }
        }
    }
    let defs = PyDict::new(py);
    for (slot, rec) in &slots {
        if !rec.get_item(1)?.eq("CComponentDefinition")? {
            continue;
        }
        let Ok(v) = rec.get_item(2)?.cast_into::<PyDict>() else {
            continue;
        };
        let b = builder(py)?;
        fill(&b, &v.as_any().get_item("ents")?, &slots)?;
        let d = PyDict::new(py);
        for field in ["guid", "name", "is_group", "shadows_face_sun"] {
            d.set_item(field, v.get_item(field)?)?;
        }
        d.set_item("always_faces_camera", v.get_item("faces_camera")?)?;
        d.set_item("is_image", images.contains(&slot.extract::<u32>()?))?;
        d.set_item("builder", b)?;
        defs.set_item(slot, d)?;
    }
    let b = builder(py)?;
    fill(&b, &root, &slots)?;
    let d = PyDict::new(py);
    d.set_item("guid", "ROOT")?;
    d.set_item("name", "ROOT_MODEL")?;
    d.set_item("builder", &b)?;
    defs.set_item("ROOT", d)?;
    let dims = PyList::empty(py);
    for d in b.getattr("dimensions")?.try_iter()? {
        let d = d?;
        if val(&d, "a")?.is_truthy()? && val(&d, "b")?.is_truthy()? {
            dims.append(d)?;
        }
    }
    let pages = match super::pages::page_list(&bytes, &ar)? {
        Some(pages) => pages,
        None => super::pages::scan(&bytes, &ar, &names)?,
    };
    let doc = PyDict::new(py);
    doc.set_item("version", version)?;
    doc.set_item("dimensions", dims)?;
    doc.set_item("layer_colors", colors)?;
    doc.set_item("layer_hidden", hidden)?;
    doc.set_item("layer_id_to_name", names)?;
    doc.set_item("material_id_to_name", mat_names)?;
    doc.set_item("materials", materials)?;
    doc.set_item("materials_by_folder", PyDict::new(py))?;
    doc.set_item("defs_dict", defs)?;
    doc.set_item("elements", PyList::empty(py))?;
    doc.set_item("pages", pages)?;
    doc.set_item("styles", PyList::empty(py))?;
    doc.set_item("units", py.None())?;
    doc.set_item("thumbnail_data", py.None())?;
    Ok(doc)
}
