// SPDX-License-Identifier: GPL-3.0-or-later
//! Group projected geometry into CAD layers with line classes and section fills.
use numpy::{ndarray::Array2, IntoPyArray};
use pyo3::{
    basic::CompareOp,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::collections::HashMap;
type Obj<'p> = Bound<'p, PyAny>;
const SCALE: f64 = 50.;
struct Classes {
    edge: i64,
    profile: i64,
    cut: i64,
    hidden: i64,
}
impl Classes {
    fn get(py: Python<'_>) -> PyResult<Self> {
        let m = py.import("core.hlr")?;
        Ok(Self {
            edge: m.getattr("KIND_EDGE")?.extract()?,
            profile: m.getattr("KIND_PROFILE")?.extract()?,
            cut: m.getattr("KIND_CUT")?.extract()?,
            hidden: m.getattr("KIND_HIDDEN")?.extract()?,
        })
    }
}
#[pyfunction]
fn dxf_groups_by_class<'p>(
    drawing: &Obj<'p>,
    view_layer: &str,
    hidden_lines: bool,
) -> PyResult<Bound<'p, PyList>> {
    let py = drawing.py();
    let c = Classes::get(py)?;
    let k = drawing.getattr("kinds")?;
    let seg = drawing.getattr("segs")?;
    let out = PyList::empty(py);
    for (name, kind) in [("", c.edge), ("-PERFIL", c.profile), ("-CORTE", c.cut)] {
        out.append((
            format!("{view_layer}{name}"),
            seg.get_item(k.rich_compare(kind, CompareOp::Eq)?)?,
        ))?;
    }
    if hidden_lines {
        out.append((
            format!("{view_layer}-OCULTAS"),
            seg.get_item(k.rich_compare(c.hidden, CompareOp::Eq)?)?,
            "DASHED",
        ))?;
    }
    Ok(out)
}
fn custom_name(linetype: &str) -> String {
    let raw = linetype.split_once(':').map_or(linetype, |(_, r)| r);
    let mut name = String::new();
    let mut prev = false;
    for c in raw.chars() {
        if name.len() == 31 {
            break;
        }
        let ok = c.is_ascii_alphanumeric() || c == '_';
        if ok {
            name.push(c.to_ascii_uppercase());
        } else if !prev {
            name.push('_');
        }
        prev = !ok;
    }
    if name.is_empty() {
        "OWN".into()
    } else {
        name
    }
}
fn opt_dict<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    if let Ok(v) = o.getattr(k) {
        if v.is_truthy()? {
            return Ok(v);
        }
    }
    Ok(PyDict::new(o.py()).into_any())
}
#[pyfunction]
#[pyo3(signature=(drawing,view_layer,hidden_lines,texts=None))]
fn dxf_groups_by_layer<'p>(
    drawing: &Obj<'p>,
    view_layer: &str,
    hidden_lines: bool,
    texts: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyList>> {
    let py = drawing.py();
    let c = Classes::get(py)?;
    let api = py.import("core.layers")?;
    let kinds = api.getattr("DXF_LINETYPES")?;
    let styles = opt_dict(drawing, "styles")?;
    let labels = opt_dict(drawing, "labels")?;
    let k = drawing.getattr("kinds")?;
    let tags = drawing.getattr("layers")?;
    let seg = drawing.getattr("segs")?;
    let mut cells: Vec<(String, &str, Vec<usize>)> = vec![];
    let mut lookup = HashMap::new();
    let mut bare = vec![];
    for i in 0..k.len()? {
        let kind = k.get_item(i)?.extract::<i64>()?;
        let tag = tags.get_item(i)?;
        if kind == c.hidden {
            continue;
        }
        if kind == c.cut && !tag.is_truthy()? {
            bare.push(i);
            continue;
        }
        let tag = if tag.is_truthy()? {
            tag.extract::<String>()?
        } else {
            view_layer.into()
        };
        let suffix = if kind == c.cut {
            "-CORTE"
        } else if kind == c.profile {
            "-PERFIL"
        } else {
            ""
        };
        let j = *lookup.entry((tag.clone(), suffix)).or_insert_with(|| {
            cells.push((tag, suffix, vec![]));
            cells.len() - 1
        });
        cells[j].2.push(i);
    }
    let out = PyList::empty(py);
    out.append((format!("{view_layer}-CORTE"), seg.get_item(bare)?))?;
    if hidden_lines {
        out.append((
            format!("{view_layer}-OCULTAS"),
            seg.get_item(k.rich_compare(c.hidden, CompareOp::Eq)?)?,
            "DASHED",
        ))?;
    }
    for (tag, suffix, ids) in cells {
        let mut style = styles.call_method1("get", (&tag,))?;
        if !style.is_truthy()? {
            style = (py.None(), "continuous").into_pyobject(py)?.into_any();
        }
        let rgb = style.get_item(0)?;
        let linetype = if suffix == "-CORTE" {
            "continuous".to_owned()
        } else {
            style.get_item(1)?.extract()?
        };
        let label = labels
            .call_method1("get", (&tag, &tag))?
            .extract::<String>()?
            + suffix;
        let weight = if suffix.is_empty() && style.len()? > 2 {
            style.get_item(2)?
        } else {
            py.None().into_bound(py)
        };
        let selected = seg.get_item(ids)?;
        if kinds.contains(&linetype)? {
            out.append((
                label,
                selected,
                kinds.get_item(linetype)?,
                rgb,
                py.None(),
                weight,
            ))?;
            continue;
        }
        let pattern = if style.len()? > 4 {
            style.get_item(4)?
        } else {
            PyTuple::empty(py).into_any()
        };
        let dash = api
            .call_method1("file_dashes", (pattern,))?
            .extract::<Vec<f64>>()?;
        out.append((
            &label,
            &selected,
            custom_name(&linetype),
            rgb,
            PyTuple::new(py, dash.into_iter().map(|v| v * 0.1))?,
            weight,
        ))?;
        if let Some(texts) =
            texts.filter(|_| suffix != "-CORTE" && style.len().is_ok_and(|n| n > 5))
        {
            let words = style.get_item(5)?;
            if words.is_none() {
                continue;
            }
            let text = words.get_item(0)?;
            let height = 0.1 * words.get_item(1)?.extract::<f64>()?;
            let spec = (
                &text,
                height,
                0.1 * words.get_item(2)?.extract::<f64>()?,
                0.1 * words.get_item(3)?.extract::<f64>()?,
            )
                .into_pyobject(py)?;
            for row in selected.try_iter()? {
                let [x0, y0, x1, y1] = row?.extract::<[f64; 4]>()?;
                for mark in api
                    .call_method1("text_marks", ((x0, y0), (x1, y1), &spec))?
                    .try_iter()?
                {
                    let [x, y, ang] = mark?.extract::<[f64; 3]>()?;
                    texts.call_method1("append", ((&label, x, y, height, ang, &text),))?;
                }
            }
        }
    }
    Ok(out)
}
#[pyfunction]
#[pyo3(signature=(scene,camera,scale_n=Some(50.),k=None,frame_scale=1.,geometry=None))]
fn dxf_cut_layers<'p>(
    scene: &Obj<'p>,
    camera: &Obj<'p>,
    scale_n: Option<f64>,
    k: Option<f64>,
    frame_scale: f64,
    geometry: Option<&Obj<'p>>,
) -> PyResult<(Bound<'p, PyList>, Bound<'p, PyList>)> {
    let py = scene.py();
    let api = py.import("core.layers")?;
    let hatches = crate::model::module(api.py(), "built_core")?.getattr("tag_styles")?.call_method1("cut_hatches", (scene,))?;
    let groups = PyList::empty(py);
    let solids = PyList::empty(py);
    if !hatches.is_truthy()? {
        return Ok((groups, solids));
    }
    let kw = PyDict::new(py);
    for (name, v) in [("profiles", false), ("fills", true), ("layers", true)] {
        kw.set_item(name, v)?;
    }
    kw.set_item("depth_layers", api.call_method1("grain_layers", (scene,))?)?;
    kw.set_item("geometry", geometry)?;
    let drawing = py
        .import("core.hlr")?
        .call_method("hlr_drawing", (scene, camera), Some(&kw))?;
    let tags = drawing.getattr("loop_layers")?;
    if tags.is_none() {
        return Ok((groups, solids));
    }
    let scale = scale_n.filter(|v| *v != 0.).unwrap_or(SCALE);
    let k = k.filter(|v| *v != 0.).unwrap_or(1000. / scale);
    let forms = crate::model::module(api.py(), "built_core")?.getattr("tag_styles")?.call_method1("cut_hatch_forms", (scene, scale))?;
    let cut = api.call_method1("section_cut_kind", (scene,))?;
    let labels = crate::model::module(api.py(), "built_core")?.getattr("tag_edits")?.call_method1("export_labels", (scene,))?;
    let depth = drawing.getattr("loop_depths")?;
    let loops = drawing.getattr("loops")?;
    let by_tag = PyDict::new(py);
    for (i, (r, t)) in loops.try_iter()?.zip(tags.try_iter()?).enumerate() {
        let tag = t?;
        if !hatches.contains(&tag)? {
            continue;
        }
        let (rings, ds) = if let Some(p) = by_tag.get_item(&tag)? {
            p.extract::<(Bound<'p, PyList>, Bound<'p, PyList>)>()?
        } else {
            let pair = (PyList::empty(py), PyList::empty(py));
            by_tag.set_item(&tag, (&pair.0, &pair.1))?;
            pair
        };
        let points = r?.extract::<Vec<[f64; 2]>>()?;
        rings.append(
            points
                .into_iter()
                .map(|[x, y]| (x * k, -y * k))
                .collect::<Vec<_>>(),
        )?;
        let d = if depth.is_truthy()? {
            depth.get_item(i)?
        } else {
            py.None().into_bound(py)
        };
        ds.append(if d.is_none() { d } else { d.mul(k)? })?;
    }
    let hatch = py.import("core.hatches")?;
    for (tag, data) in by_tag.iter() {
        let (rings, ds): (Obj<'p>, Obj<'p>) = data.extract()?;
        let name = labels
            .call_method1("get", (&tag, &tag))?
            .extract::<String>()?;
        let layer = if scene.hasattr("layer")? {
            scene.call_method1("layer", (&tag,))?
        } else {
            py.None().into_bound(py)
        };
        let style = |op: &str| {
            if layer.is_none() {
                Ok(py.None().into_bound(py))
            } else {
                crate::model::module(py, "built_core")?.getattr("tag_styles")?.call_method1(op, (&layer,))
            }
        };
        let ink = style("hatch_ink_rgb")?;
        let ink = if ink.is_truthy()? {
            ink
        } else {
            style("layer_color")?
        };
        let ground = style("hatch_ground_rgb")?;
        let id = hatches.get_item(&tag)?;
        let filled = id.eq("solid")?;
        if !ground.is_none() || filled {
            let quads = PyList::empty(py);
            for q in hatch.call_method1("fill_quads", (&rings,))?.try_iter()? {
                let p = q?.extract::<[[f64; 2]; 4]>()?;
                let row = [2, 3, 0, 1].map(|i| (p[i][0] / k, -p[i][1] / k));
                quads.append(PyTuple::new(py, row)?)?;
            }
            if !ground.is_none() {
                solids.append((format!("{name}-FONDO"), &quads, ground))?;
            }
            if filled {
                solids.append((format!("{name}-TRAMA"), quads, ink))?;
                continue;
            }
        }
        let form = py
            .get_type::<PyDict>()
            .call1((forms.call_method1("get", (&tag, PyDict::new(py)))?,))?;
        form.set_item("cut", &cut)?;
        let lines = hatch.call_method1("hatch_lines", (id, rings, form, frame_scale, ds))?;
        let mut pts = vec![];
        for line in lines.try_iter()? {
            let [x0, y0, x1, y1] = line?.extract::<[f64; 4]>()?;
            pts.extend([x0 / k, -y0 / k, x1 / k, -y1 / k]);
        }
        if !pts.is_empty() {
            let array = Array2::from_shape_vec((pts.len() / 4, 4), pts)
                .unwrap()
                .into_pyarray(py);
            groups.append((format!("{name}-TRAMA"), array, "CONTINUOUS", ink))?;
        }
    }
    Ok((groups, solids))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dxf_groups_by_class, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_groups_by_layer, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_cut_layers, m)?)
}
