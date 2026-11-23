// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered sheet records and typed drawing items. Their editor handles
//! initialize current defaults, including a title block's legacy rows.
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyList, PyString, PyTuple};

pub(crate) const ITEMS: &[(&str, &str)] = &[
    ("frames", "MarcoVista"),
    ("texts", "TextoItem"),
    ("images", "ImagenItem"),
    ("scalebars", "BarraEscala"),
    ("nortes", "FlechaNorte"),
    ("leyendas", "Leyenda"),
    ("shapes", "FormaItem"),
    ("cotas", "CotaItem"),
    ("cotas_ang", "CotaAngularItem"),
    ("cotas_rad", "CotaRadialItem"),
    ("etiquetas", "EtiquetaItem"),
    ("perfiles", "PerfilTerreno"),
    ("niveles", "NivelItem"),
    ("llamadas", "LlamadaItem"),
];
const BORDER: &[(&str, &str)] = &[
    ("mm", "border_mm"),
    ("color", "border_color"),
    ("radius_mm", "border_radius_mm"),
    ("style", "border_style"),
];

fn floats<'py>(py: Python<'py>, source: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    for value in source.try_iter()? {
        out.append(py.get_type::<PyFloat>().call1((value?,))?)?;
    }
    Ok(out)
}

fn item<'py>(class: &Bound<'py, PyAny>, raw: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyAny>> {
    let py = raw.py();
    let fields = class.getattr("__dataclass_fields__")?;
    let kwargs = PyDict::new(py);
    for (k, v) in raw {
        if fields.contains(&k)? {
            kwargs.set_item(k, v)?
        }
    }
    let record = class.call((), Some(&kwargs))?;
    if let Some(anchors) = raw
        .get_item("geometry_anchors")?
        .and_then(|v| v.cast_into::<PyDict>().ok())
    {
        for name in ["a", "b"] {
            let hold = format!("hold_{name}");
            if !record.getattr("_record_fields")?.contains(&hold)? {
                continue;
            }
            if let Some(source) = anchors
                .get_item(name)?
                .filter(|v| v.is_instance_of::<PyDict>())
            {
                let point = record.getattr(format!("{name}_world"))?;
                if point.is_truthy()? {
                    let tuple = py.get_type::<PyTuple>().call1((point,))?;
                    record.setattr(hold, (tuple, source))?;
                }
            }
        }
    }
    Ok(record)
}

#[pyfunction]
pub fn igz_load_sheet<'py>(
    py: Python<'py>,
    raw: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyAny>> {
    let types = py.import("core.composition")?;
    let kwargs = PyDict::new(py);
    for key in ["name", "paper", "landscape", "margin_mm"] {
        if let Some(v) = raw.get_item(key)? {
            match key {
                "landscape" => kwargs.set_item(key, v.is_truthy()?)?,
                "margin_mm" => kwargs.set_item(key, py.get_type::<PyFloat>().call1((v,))?)?,
                _ => kwargs.set_item(key, v)?,
            }
        }
    }
    if let Some(border) = raw
        .get_item("border")?
        .and_then(|v| v.cast_into::<PyDict>().ok())
    {
        if border
            .get_item("on")?
            .map(|v| v.is_truthy())
            .transpose()?
            .unwrap_or(false)
        {
            kwargs.set_item("border", true)?;
            for &(key, field) in BORDER {
                if let Some(v) = border.get_item(key)? {
                    let value = if key.ends_with("mm") {
                        py.get_type::<PyFloat>().call1((v,))?
                    } else {
                        v.str()?.into_any()
                    };
                    kwargs.set_item(field, value)?;
                }
            }
        }
    }
    for &(key, ty) in ITEMS {
        if let Some(list) = raw
            .get_item(key)?
            .and_then(|v| v.cast_into::<PyList>().ok())
        {
            let items = PyList::empty(py);
            let class = types.getattr(ty)?;
            for entry in list {
                if let Ok(raw) = entry.cast::<PyDict>() {
                    items.append(item(&class, raw)?)?;
                }
            }
            kwargs.set_item(key, items)?;
        }
    }
    if let Some(block) = raw
        .get_item("cajetin")?
        .and_then(|v| v.cast_into::<PyDict>().ok())
    {
        kwargs.set_item("cajetin", item(&types.getattr("Cajetin")?, &block)?)?;
    }
    if let Some(guides) = raw
        .get_item("guides")?
        .and_then(|v| v.cast_into::<PyDict>().ok())
    {
        for (k, field) in [("v", "guides_v"), ("h", "guides_h")] {
            if let Some(v) = guides.get_item(k)? {
                if v.is_truthy()? {
                    kwargs.set_item(field, floats(py, &v)?)?;
                }
            }
        }
    }
    let sheet = types.getattr("Composicion")?.call((), Some(&kwargs))?;
    types.call_method1("_migrate_fixed_scale_labels", (&sheet,))?;
    Ok(sheet)
}

fn archive_item<'py>(item: &Bound<'py, PyAny>, anchors: bool) -> PyResult<Bound<'py, PyDict>> {
    let py = item.py();
    let out = PyDict::new(py);
    for name in item.getattr("__dataclass_fields__")?.try_iter()? {
        let key = name?.cast_into::<PyString>()?;
        let value = item.getattr(&key)?;
        // Detach containers from the document before texture rewriting or
        // asynchronous compression can observe a later edit.
        out.set_item(key, crate::doc::value::Value::from_py(&value)?.to_py(py)?)?;
    }
    if anchors {
        let types = py.import("core.composition")?;
        let holds = PyDict::new(py);
        for bound in types.call_method1("sheet_holds", (item,))?.try_iter()? {
            let (name, (_, hold)): (String, (Bound<'_, PyAny>, Bound<'_, PyAny>)) =
                bound?.extract()?;
            let saved = if hold.is_instance_of::<PyDict>() {
                hold.clone()
            } else {
                hold.call_method0("archive")?
            };
            if saved.is_none() {
                continue;
            }
            holds.set_item(&name, saved)?;
            if !hold.is_instance_of::<PyDict>() {
                let point = hold.call_method0("position")?;
                if !point.is_none() {
                    out.set_item(
                        format!("{name}_world"),
                        py.get_type::<PyList>()
                            .call1((point.call_method0("toTuple")?,))?,
                    )?;
                }
            }
        }
        if !holds.is_empty() {
            out.set_item("geometry_anchors", holds)?;
        }
    }
    Ok(out)
}

#[pyfunction]
pub fn igz_archive_sheet<'py>(
    py: Python<'py>,
    sheet: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    for key in ["name", "paper", "landscape", "margin_mm"] {
        out.set_item(key, sheet.getattr(key)?)?;
    }
    for (index, &(key, _)) in ITEMS.iter().enumerate() {
        if index == 1 && sheet.getattr("border")?.is_truthy()? {
            let border = PyDict::new(py);
            border.set_item("on", true)?;
            for &(k, attr) in BORDER {
                border.set_item(k, sheet.getattr(attr)?)?;
            }
            out.set_item("border", border)?;
        }
        let items = sheet.getattr(key)?;
        if index == 0 || items.is_truthy()? {
            let records = PyList::empty(py);
            for item in items.try_iter()? {
                records.append(archive_item(&item?, index >= 4)?)?;
            }
            out.set_item(key, records)?;
        }
    }
    let block = sheet.getattr("cajetin")?;
    if !block.is_none() {
        out.set_item("cajetin", archive_item(&block, false)?)?;
    }
    let (v, h) = (sheet.getattr("guides_v")?, sheet.getattr("guides_h")?);
    if v.is_truthy()? || h.is_truthy()? {
        let guides = PyDict::new(py);
        guides.set_item("v", floats(py, &v)?)?;
        guides.set_item("h", floats(py, &h)?)?;
        out.set_item("guides", guides)?;
    }
    Ok(out)
}
