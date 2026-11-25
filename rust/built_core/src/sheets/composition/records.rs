// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::file_sheet::{igz_archive_sheet, load_sheet, ITEMS};

pub(super) fn install_methods(name: &str, ns: &Bound<PyDict>) -> PyResult<()> {
    let py = ns.py();
    match name {
        "Composicion" => {
            methods!(ns,py;"page_size_mm"=>page_size_mm,"all_items"=>all_items,"default_frame"=>default_frame,"default_cajetin"=>default_cajetin,"to_dict"=>igz_archive_sheet);
            ns.set_item(
                "from_dict",
                py.import("builtins")?
                    .call_method1("classmethod", (wrap_pyfunction!(from_dict, py)?,))?,
            )?;
        }
        "MarcoVista" => {
            methods!(ns,py;"scale_label"=>scale_label,"model_height_m"=>model_height_m,"render_px"=>render_px)
        }
        "Cajetin" => {
            methods!(ns,py;"__post_init__"=>init_rows,"set_field"=>set_field,"look"=>look,"design_key"=>design_key,"template_dict"=>template_dict)
        }
        "EtiquetaItem" => methods!(ns,py;"spots"=>spots,"add_leader"=>add_leader),
        _ => {}
    }
    Ok(())
}
#[pyfunction]
fn from_dict<'p>(cls: &Obj<'p>, d: &Bound<'p, PyDict>) -> PyResult<Obj<'p>> {
    load_sheet(cls, d)
}
#[pyfunction]
fn page_size_mm(sheet: &Obj) -> PyResult<(f64, f64)> {
    let paper = string(sheet, "paper")?;
    let (w, h) = api(sheet.py())?
        .getattr("PAPER_SIZES_MM")?
        .get_item(paper)?
        .extract()?;
    Ok(if flag(sheet, "landscape")? {
        (h, w)
    } else {
        (w, h)
    })
}
#[pyfunction]
fn default_frame<'p>(sheet: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = sheet.py();
    let (w, h) = page_size_mm(sheet)?;
    let m = num(sheet, "margin_mm")?;
    let kw = PyDict::new(py);
    kw.set_item("style", "style:Architectural")?;
    api(py)?
        .getattr("MarcoVista")?
        .call((m, m, w - 2. * m, h - 2. * m), Some(&kw))
}
#[pyfunction]
fn default_cajetin<'p>(sheet: &Obj<'p>) -> PyResult<Obj<'p>> {
    let (pw, ph) = page_size_mm(sheet)?;
    let m = num(sheet, "margin_mm")?;
    let w = 180f64.min(pw - m * 2.);
    api(sheet.py())?.call_method1("Cajetin", (pw - m - w, ph - m - 33., w, 33.))
}
#[pyfunction]
fn all_items<'p>(sheet: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let list = PyList::empty(sheet.py());
    for &(key, _) in ITEMS {
        list.call_method1("extend", (sheet.getattr(key)?,))?;
    }
    let block = sheet.getattr("cajetin")?;
    if !block.is_none() {
        list.append(block)?;
    }
    Ok(list)
}
#[pyfunction]
fn scale_label(item: &Obj) -> PyResult<String> {
    if flag(item, "perspective")? {
        return tr(item.py(), "NO SCALE");
    }
    let n = num(item, "scale_n")?;
    let scale = model::format_scale(n);
    let template = string(item, "scale_text").unwrap_or_default();
    let template = if template.is_empty() {
        "ESC. 1:{n}"
    } else {
        &template
    };
    Ok(if template.contains("1:{n}") {
        template.replace("1:{n}", &scale)
    } else {
        template.replace("{n}", &crate::sheets::svg::fmt_g(n))
    })
}
#[pyfunction]
fn model_height_m(item: &Obj) -> PyResult<f64> {
    item.py()
        .import("built_core")?
        .call_method1(
            "sheet_model_height",
            (num(item, "h_mm")?, num(item, "scale_n")?),
        )?
        .extract()
}
#[pyfunction(signature=(item,dpi=300))]
fn render_px(item: &Obj, dpi: i32) -> PyResult<(i64, i64)> {
    Ok((
        camera::mm_to_px(num(item, "w_mm")?, dpi),
        camera::mm_to_px(num(item, "h_mm")?, dpi),
    ))
}
#[pyfunction]
fn init_rows(item: &Obj) -> PyResult<()> {
    if !flag(item, "campos")? {
        let rows = PyList::empty(item.py());
        for pair in item.getattr("FIELDS")?.try_iter()? {
            let (label, key): (Obj, String) = pair?.extract()?;
            rows.append(PyList::new(item.py(), [label, item.getattr(key)?])?)?;
        }
        item.setattr("campos", rows)?;
    }
    Ok(())
}
#[pyfunction]
fn set_field(item: &Obj, label: &str, value: &Obj) -> PyResult<()> {
    let rows = item.getattr("campos")?;
    let wanted = label.trim().to_uppercase();
    for row in rows.try_iter()? {
        let row = row?;
        if row.get_item(0)?.str()?.to_string().trim().to_uppercase() == wanted {
            return row.set_item(1, value);
        }
    }
    rows.call_method1(
        "append",
        (PyList::new(
            item.py(),
            [label.into_bound_py_any(item.py())?, value.clone()],
        )?,),
    )?;
    Ok(())
}
#[pyfunction]
fn look<'p>(item: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(item.py());
    for key in item.getattr("LOOK_FIELDS")?.try_iter()? {
        let k = key?.extract::<String>()?;
        out.set_item(&k, item.getattr(&k)?)?;
    }
    Ok(out)
}
#[pyfunction]
fn design_key(item: &Obj) -> PyResult<String> {
    let m = api(item.py())?;
    let base = m.getattr("CAJETIN_DESIGN_BASE")?.cast_into::<PyDict>()?;
    for row in m.getattr("CAJETIN_DESIGNS")?.try_iter()? {
        let row = row?;
        let changes = row.get_item(2)?;
        let mut matches = true;
        for (k, v) in base.iter() {
            let expected = changes.call_method1("get", (&k, v))?;
            let actual = item.getattr(k.extract::<String>()?)?;
            matches = if expected.is_instance_of::<pyo3::types::PyFloat>()
                || actual.is_instance_of::<pyo3::types::PyFloat>()
            {
                actual
                    .extract::<f64>()
                    .ok()
                    .zip(expected.extract::<f64>().ok())
                    .is_some_and(|(a, b)| (a - b).abs() < 1e-6)
            } else {
                actual.eq(expected)?
            };
            if !matches {
                break;
            }
        }
        if matches {
            return row.get_item(0)?.extract();
        }
    }
    Ok(String::new())
}
#[pyfunction]
fn template_dict<'p>(item: &Obj<'p>) -> PyResult<Obj<'p>> {
    let d = item
        .py()
        .import("dataclasses")?
        .call_method1("asdict", (item,))?;
    for k in [
        "x_mm", "y_mm", "z", "locked", "group_id", "proyecto", "autor", "fecha", "escala", "lamina",
    ] {
        d.del_item(k)?;
    }
    item.py()
        .import("core.doc_model")?
        .call_method1("plain", (d,))
}
#[pyfunction]
fn spots(item: &Obj) -> PyResult<Vec<(f64, f64)>> {
    let mut spots = vec![(num(item, "ax_mm")?, num(item, "ay_mm")?)];
    let leaders = item.getattr("leaders")?;
    if leaders.is_truthy()? {
        for l in leaders.try_iter()? {
            let l = l?;
            spots.push((
                l.call_method1("get", ("ax_mm", 0.))?.extract()?,
                l.call_method1("get", ("ay_mm", 0.))?.extract()?,
            ));
        }
    }
    Ok(spots)
}
#[pyfunction(signature=(item,ax_mm,ay_mm,anchor_uid="",a_world=None))]
fn add_leader<'p>(
    item: &Obj<'p>,
    ax_mm: f64,
    ay_mm: f64,
    anchor_uid: &str,
    a_world: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyDict>> {
    let py = item.py();
    let leader = PyDict::new(py);
    leader.set_item("ax_mm", ax_mm)?;
    leader.set_item("ay_mm", ay_mm)?;
    leader.set_item("anchor_uid", anchor_uid)?;
    let world = match a_world {
        Some(p) if p.is_truthy()? => py.get_type::<PyList>().call1((p,))?,
        _ => py.None().into_bound(py),
    };
    leader.set_item("a_world", world)?;
    let old = item.getattr("leaders")?;
    let list = if old.is_truthy()? {
        py.get_type::<PyList>().call1((old,))?.cast_into()?
    } else {
        PyList::empty(py)
    };
    list.append(&leader)?;
    item.setattr("leaders", list)?;
    Ok(leader)
}
#[pyfunction]
pub(super) fn sheet_holds<'p>(item: &Obj<'p>) -> PyResult<Vec<(&'static str, Obj<'p>)>> {
    let mut out = Vec::new();
    if !item
        .getattr("anchor_uid")
        .is_ok_and(|v| v.is_truthy().unwrap_or(false))
    {
        return Ok(out);
    }
    for name in ["a", "b"] {
        let Ok(hold) = item.getattr(format!("hold_{name}")) else {
            continue;
        };
        let point = item.getattr(format!("{name}_world"))?;
        let tuple = item.py().get_type::<PyTuple>();
        if !hold.is_none()
            && point.is_truthy()?
            && tuple
                .call1((point,))?
                .eq(tuple.call1((hold.get_item(0)?,))?)?
        {
            out.push((name, hold));
        }
    }
    Ok(out)
}
#[pyfunction]
pub(super) fn bind_sheet_anchors(scene: &Obj) -> PyResult<()> {
    let py = scene.py();
    let restore = py
        .import("built_core.annotation_anchor")?
        .getattr("Attachment")?
        .getattr("restore")?;
    let vec = py.import("built_core")?.getattr("Vec3")?;
    for sheet in scene.getattr("compositions")?.try_iter()? {
        let sheet = sheet?;
        for key in ["cotas", "etiquetas", "niveles"] {
            for item in sheet.getattr(key)?.try_iter()? {
                let item = item?;
                for (name, hold) in sheet_holds(&item)? {
                    let (point, raw): (Bound<PyTuple>, Obj) = hold.extract()?;
                    if raw.is_instance_of::<PyDict>() {
                        let anchor = restore.call1((scene, raw, vec.call(&point, None)?))?;
                        let value = if anchor.is_none() {
                            anchor
                        } else {
                            (point, anchor).into_bound_py_any(py)?
                        };
                        item.setattr(format!("hold_{name}"), value)?;
                    }
                }
            }
        }
    }
    Ok(())
}
#[pyfunction]
pub(super) fn _migrate_fixed_scale_labels(sheet: &Obj) -> PyResult<()> {
    let py = sheet.py();
    let mut z = all_items(sheet)?
        .iter()
        .map(|i| num(&i, "z"))
        .collect::<PyResult<Vec<_>>>()?
        .into_iter()
        .reduce(f64::max)
        .map_or(0., |v| v + 1.);
    for f in sheet.getattr("frames")?.try_iter()? {
        let f = f?;
        if !flag(&f, "show_scale")? {
            continue;
        }
        f.setattr("show_scale", false)?;
        if !flag(&f, "uid")? {
            f.setattr(
                "uid",
                py.import("uuid")?.call_method0("uuid4")?.getattr("hex")?,
            )?;
        }
        let template = string(&f, "scale_text")?;
        let text = if template.is_empty() {
            "ESC. 1:{n}"
        } else {
            &template
        };
        let size = num(&f, "scale_mm")?;
        let size = if size == 0. { 3. } else { size.max(1.5) };
        let pos = string(&f, "scale_pos")?;
        let inside = matches!(pos.as_str(), "inside-bl" | "inside-br");
        let left = matches!(pos.as_str(), "inside-bl" | "under-left");
        let x = num(&f, "x_mm")?
            + if left { 0. } else { num(&f, "w_mm")? - 40. }
            + if inside {
                if left {
                    1.
                } else {
                    -1.
                }
            } else {
                0.
            };
        let y = num(&f, "y_mm")? + num(&f, "h_mm")? + if inside { -size * 1.4 - 1. } else { 1. };
        let kw = PyDict::new(py);
        kw.set_item("x_mm", x)?;
        kw.set_item("y_mm", y)?;
        kw.set_item("w_mm", 40.)?;
        kw.set_item(
            "text",
            text.replace("1:{n}", "{escala}").replace("{n}", "{escala}"),
        )?;
        kw.set_item("size_pt", size / (25.4 / 72.))?;
        kw.set_item("bold", true)?;
        kw.set_item("align", if left { "left" } else { "right" })?;
        kw.set_item("bg_color", if inside { "#ffffff" } else { "" })?;
        kw.set_item("bg_opacity", if inside { 0.86 } else { 1. })?;
        kw.set_item("frame_uid", f.getattr("uid")?)?;
        kw.set_item("z", z)?;
        sheet.getattr("texts")?.call_method1(
            "append",
            (api(py)?.getattr("TextoItem")?.call((), Some(&kw))?,),
        )?;
        z += 1.;
    }
    Ok(())
}
