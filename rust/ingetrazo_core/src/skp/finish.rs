// SPDX-License-Identifier: GPL-3.0-or-later
use super::tree::{attr, flagged, none, values, Obj};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyTuple},
};
use std::{collections::HashMap, path::PathBuf, time::UNIX_EPOCH};

#[pyfunction]
pub fn skp_merge_protos<'p>(protos: &Bound<'p, PyList>) -> PyResult<Bound<'p, PyList>> {
    let py = protos.py();
    let mut protos = protos.clone();
    for _ in 0..8 {
        let mut buckets = HashMap::<(String, usize, usize, usize), Vec<usize>>::new();
        for (i, p) in protos.iter().enumerate() {
            buckets
                .entry((
                    p.get_item("name")?.extract()?,
                    p.get_item("faces")?.len()?,
                    p.get_item("soft_edges")?.len()?,
                    p.get_item("children")?.len()?,
                ))
                .or_default()
                .push(i);
        }
        let mut parent: Vec<_> = (0..protos.len()).collect();
        for bucket in buckets.values().filter(|b| b.len() > 1) {
            let mut seen = HashMap::new();
            for &i in bucket {
                let p = protos.get_item(i)?;
                let fields = [
                    "faces",
                    "soft_edges",
                    "children",
                    "loose_edges",
                    "curves",
                    "component",
                ];
                let items = fields
                    .into_iter()
                    .map(|k| p.call_method1("get", (k,)))
                    .collect::<PyResult<Vec<_>>>()?;
                let sig = PyTuple::new(py, items)?.repr()?.to_string();
                parent[i] = *seen.entry(sig).or_insert(i);
            }
        }
        if parent.iter().enumerate().all(|(i, &j)| i == j) {
            break;
        }
        let out = PyList::empty(py);
        let mut indices = vec![0; protos.len()];
        for (i, &j) in parent.iter().enumerate() {
            let src = protos.get_item(i)?;
            if i == j {
                indices[i] = out.len();
                out.append(src)?;
                continue;
            }
            let dst = protos.get_item(j)?;
            let src_instances = src.get_item("instances")?;
            if !src_instances.is_truthy()? {
                continue;
            }
            let instances = dst.get_item("instances")?;
            for k in ["instance_layers", "instance_names", "instance_identities"] {
                if !src.contains(k)? && !dst.contains(k)? {
                    continue;
                }
                let pad = |n: usize| PyList::new(py, (0..n).map(|_| py.None()));
                let list = match dst.cast::<PyDict>()?.get_item(k)? {
                    Some(list) => list,
                    None => {
                        let l = pad(instances.len()?)?;
                        dst.set_item(k, &l)?;
                        l.into_any()
                    }
                };
                let append = match src.cast::<PyDict>()?.get_item(k)? {
                    Some(a) => a,
                    None => pad(src_instances.len()?)?.into_any(),
                };
                list.call_method1("extend", (append,))?;
            }
            instances.call_method1("extend", (src_instances,))?;
        }
        for p in out.iter() {
            for k in p.get_item("children")?.try_iter()? {
                let k = k?;
                let idx = k.get_item("proto")?.extract::<usize>()?;
                k.set_item("proto", indices[parent[idx]])?;
            }
        }
        protos = out;
    }
    Ok(protos)
}
#[pyfunction]
pub fn skp_file_layers<'p>(model: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = model.py();
    let out = PyList::empty(py);
    for l in values(model, "layers")? {
        let name = attr(&l, "name")?.extract::<String>().unwrap_or_default();
        if name.is_empty() || name == "Layer0" || name == "Untagged" || flagged(&l, "default")? {
            continue;
        }
        let visible = if let Some(v) = l.getattr_opt("hidden")? {
            !v.is_truthy()?
        } else {
            l.getattr_opt("visible")?
                .map_or(Ok(true), |v| v.is_truthy())?
        };
        let row = PyDict::new(py);
        row.set_item("name", name)?;
        row.set_item("visible", visible)?;
        out.append(row)?;
    }
    Ok(out)
}
#[pyfunction]
pub fn skp_empty_model(
    model: &Obj<'_>,
    skp_path: Option<PathBuf>,
    legacy_era: bool,
) -> PyResult<bool> {
    if legacy_era || flagged(model, "definitions")? {
        return Ok(false);
    }
    let Some(path) = skp_path else {
        return Ok(false);
    };
    let root = attr(model, "root")?;
    if root.is_none() {
        return Ok(false);
    }
    for k in [
        "faces",
        "edges",
        "instances",
        "texts",
        "dimensions",
        "construction_lines",
        "construction_points",
    ] {
        if flagged(&root, k)? {
            return Ok(false);
        }
    }
    Ok(model.py().detach(move || {
        let Ok(bytes) = super::vff::thumbnail(&path) else {
            return false;
        };
        let Ok(img) = image::load_from_memory(&bytes) else {
            return false;
        };
        let rgba = img.to_rgba8();
        let mut pixels = rgba.pixels();
        pixels
            .next()
            .is_some_and(|first| pixels.all(|p| p == first))
    }))
}
#[pyfunction]
pub fn skp_texture_dir(py: Python<'_>, skp_path: PathBuf) -> PyResult<Obj<'_>> {
    let stamp = std::fs::metadata(&skp_path)
        .and_then(|m| Ok((skp_path.canonicalize()?, m.len(), m.modified()?)))
        .ok()
        .and_then(|(p, size, t)| {
            t.duration_since(UNIX_EPOCH)
                .ok()
                .map(|v| format!("{}|{}|{}", p.display(), size, v.as_nanos()))
        })
        .unwrap_or_else(|| skp_path.display().to_string());
    let key = py
        .import("hashlib")?
        .call_method1("sha1", (PyBytes::new(py, stamp.as_bytes()),))?
        .call_method0("hexdigest")?
        .extract::<String>()?;
    let root = py
        .import("core.texture")?
        .call_method0("texture_cache_root")?
        .extract::<PathBuf>()?;
    let stem: String = skp_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .filter(|c| c.is_alphanumeric() || " ._-".contains(*c))
        .collect();
    let stem = match stem.trim() {
        "" => "model",
        s => s,
    };
    let path = root.join("skp").join(format!("{}-{}", stem, &key[..10]));
    let class = py.import("pathlib")?.getattr("Path")?;
    if std::fs::create_dir_all(&path).is_ok() {
        class.call1((path,))
    } else {
        let kw = PyDict::new(py);
        kw.set_item("prefix", "ingetrazo-skp-tex-")?;
        class.call1((py
            .import("tempfile")?
            .getattr("mkdtemp")?
            .call((), Some(&kw))?,))
    }
}
#[pyfunction]
pub fn skp_cached_colorize<'p>(
    data: &Obj<'p>,
    color: &Obj<'p>,
    ctype: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let py = data.py();
    let glue = py.import("formats.skp_reader")?;
    let transform = || glue.call_method1("_colorize_image", (data, color, ctype));
    let Some(rgb) = color.extract::<Vec<i64>>().ok().filter(|v| v.len() >= 3) else {
        return transform();
    };
    let hash = py
        .import("hashlib")?
        .call_method1("sha256", (data,))?
        .call_method0("digest")?;
    let key = (hash, (rgb[0], rgb[1], rgb[2]), ctype).into_pyobject(py)?;
    let cache = glue.getattr("_colorize_cache")?;
    if let Some(v) = cache.cast::<PyDict>()?.get_item(&key)? {
        cache.call_method1("move_to_end", (&key,))?;
        return Ok(v);
    }
    let v = transform()?;
    let limit = glue.getattr("_COLORIZE_CACHE_LIMIT")?.extract::<usize>()?;
    if !v.is(data) && v.len()? <= limit {
        cache.set_item(&key, &v)?;
        cache.call_method1("move_to_end", (&key,))?;
        let mut bytes = 0;
        for value in cache.call_method0("values")?.try_iter()? {
            bytes += value?.len()?;
        }
        while bytes > limit || cache.len()? > 128 {
            bytes -= cache
                .call_method1("popitem", (false,))?
                .get_item(1)?
                .len()?;
        }
    }
    Ok(v)
}
#[pyfunction]
pub fn skp_needs_tint(data: &Obj<'_>, declared_rgb: &Obj<'_>) -> PyResult<bool> {
    let Some(rgb) = declared_rgb
        .extract::<Vec<f64>>()
        .ok()
        .filter(|v| v.len() >= 3)
    else {
        return Ok(false);
    };
    if !data.is_truthy()? {
        return Ok(false);
    }
    let class = data.py().import("PySide6.QtGui")?.getattr("QImage")?;
    let img = class.call_method1("fromData", (data,))?;
    if img.call_method0("isNull")?.is_truthy()? {
        return Ok(false);
    }
    let small = img
        .call_method1("convertToFormat", (class.getattr("Format_ARGB32")?,))?
        .call_method1("scaled", (16, 16))?;
    let mut sum = [0.; 3];
    let mut alpha = 0.;
    for y in 0..16 {
        for x in 0..16 {
            let c = small.call_method1("pixelColor", (x, y))?;
            let a = c.call_method0("alpha")?.extract::<f64>()?;
            alpha += a;
            for (i, k) in ["red", "green", "blue"].iter().enumerate() {
                sum[i] += c.call_method0(k)?.extract::<f64>()? * a;
            }
        }
    }
    Ok(alpha > 0. && (0..3).any(|i| (sum[i] / alpha - rgb[i]).abs() > 24.))
}
#[pyfunction]
pub fn skp_inverted_png<'p>(data: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = data.py();
    let qt = py.import("PySide6.QtCore")?;
    let img = py
        .import("PySide6.QtGui")?
        .getattr("QImage")?
        .call_method1("fromData", (data,))?;
    if img.call_method0("isNull")?.is_truthy()? {
        return Ok(none(py));
    }
    img.call_method0("invertPixels")?;
    let buf = qt.call_method0("QBuffer")?;
    buf.call_method1("open", (qt.getattr("QIODevice")?.getattr("WriteOnly")?,))?;
    if img.call_method1("save", (&buf, "PNG"))?.is_truthy()? {
        py.get_type::<PyBytes>().call1((buf.call_method0("data")?,))
    } else {
        Ok(none(py))
    }
}
