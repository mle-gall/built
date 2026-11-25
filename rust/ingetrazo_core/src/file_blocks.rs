// SPDX-License-Identifier: GPL-3.0-or-later
//! Turn streamed legacy JSON mesh blocks into the same deferred columns
//! as binary members, preserving the first use of each layer and paint.
use crate::math::Vec3;
use glam::DVec3;
use numpy::{IntoPyArray, PyReadonlyArray2};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySlice},
};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.formats.igz")
}
fn slice(py: Python<'_>, lo: usize, hi: usize) -> Bound<'_, PySlice> {
    PySlice::new(py, lo as isize, hi.min(isize::MAX as usize) as isize, 1)
}
#[pyfunction]
fn igz_native_blocks<'p>(meta: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = meta.py();
    let codec = api(py)?;
    let np = py.import("numpy")?;
    let json = py.import("json")?;
    let mut pos = meta.get_item(0)?;
    let flags = meta.get_item(1)?;
    let curves = meta.get_item(2)?;
    let mut layers = meta.get_item(3)?;
    let mut sizes = meta.get_item(5)?;
    let mut counts = meta.get_item(6)?;
    let face_attr = meta.get_item(7)?;
    let multi = meta.get_item(9)?;
    let ne = flags.len()?;
    let first_point = 2 * ne;
    let small = !codec
        .call_method1("_bulk_sized", (ne, counts.len()?))?
        .is_truthy()?;
    let names = PyDict::new(py);
    let mut lut = vec![];
    for text in meta.get_item(4)?.try_iter()? {
        let l = json.call_method1("loads", (text?,))?;
        lut.push(if small && !l.is_truthy()? {
            -1i32
        } else {
            names
                .call_method1("setdefault", (l, names.len()))?
                .extract()?
        })
    }
    if !names.is_empty() {
        lut.push(-1);
        layers = lut.into_pyarray(py).get_item(layers)?;
    }
    let table = PyList::empty(py);
    let kw = PyDict::new(py);
    kw.set_item("parse_constant", codec.getattr("_note_constant")?)?;
    for text in meta.get_item(8)?.try_iter()? {
        table.append(json.call_method("loads", (text?,), Some(&kw))?)?
    }
    if multi.len()? > 0 {
        let n = sizes.call_method0("tolist")?.extract::<Vec<usize>>()?;
        let f = counts.call_method0("tolist")?.extract::<Vec<usize>>()?;
        let index = |lengths: &[usize], start: usize| -> PyResult<Vec<usize>> {
            let mut offsets = vec![start];
            let mut at = start;
            for &n in lengths {
                at = at.checked_add(n).ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err("invalid mesh block sizes")
                })?;
                offsets.push(at)
            }
            Ok(offsets)
        };
        let corner = index(&n, first_point)?;
        let face = index(&f, 0)?;
        if *face.last().unwrap() != n.len() || *corner.last().unwrap() != pos.len()? {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "inconsistent mesh block sizes",
            ));
        }
        let mut keep = vec![true; n.len()];
        let next_counts = counts.call_method0("copy")?;
        let next_sizes = sizes.call_method0("copy")?;
        let pieces = PyList::empty(py);
        let mut at = 0usize;
        for i in multi.call_method0("tolist")?.extract::<Vec<usize>>()? {
            if i >= f.len() || f[i] < 2 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "invalid mesh block hole index",
                ));
            }
            let start = face[i] + 1;
            let end = face[i + 1];
            let holes = PyList::empty(py);
            for j in start..end {
                let h = PyList::empty(py);
                let coords = pos
                    .get_item(slice(py, corner[j], corner[j + 1]))?
                    .call_method0("tolist")?;
                for p in coords.try_iter()? {
                    h.append(Vec3::from(DVec3::from_array(p?.extract::<[f64; 3]>()?)))?
                }
                holes.append(h)?;
            }
            let kept = py
                .import("core.topology")?
                .call_method1("_maximal_holes", (&holes,))?;
            pieces.append(pos.get_item(slice(py, at, corner[start]))?)?;
            let mut pts = vec![];
            let mut lengths = vec![];
            for h in kept.try_iter()? {
                let h = h?;
                lengths.push(h.len()?);
                for p in h.try_iter()? {
                    pts.extend(crate::math::xyz(&p?)?.to_array())
                }
            }
            if !pts.is_empty() {
                pieces.append(
                    numpy::ndarray::Array2::from_shape_vec((pts.len() / 3, 3), pts)
                        .unwrap()
                        .into_pyarray(py),
                )?
            }
            let nr = lengths.len();
            keep[start + nr..end].fill(false);
            next_sizes.set_item(slice(py, start, start + nr), lengths)?;
            next_counts.set_item(i, 1 + nr)?;
            at = corner[end];
        }
        pieces.append(pos.get_item(slice(py, at, pos.len()?))?)?;
        pos = np.call_method1("concatenate", (pieces,))?;
        sizes = next_sizes.get_item(keep.into_pyarray(py))?;
        counts = next_counts;
    }
    // Only allocate a float32 column when every binary64 point survives
    // narrowing, including signed zeros and NaN bit patterns.
    let compact = if let Ok(view) = pos.extract::<PyReadonlyArray2<'_, f64>>() {
        view.as_array()
            .iter()
            .all(|v| (*v as f32 as f64).to_bits() == v.to_bits())
    } else {
        false
    };
    if compact {
        pos = pos.call_method1("astype", ("float32",))?
    }
    let out = PyDict::new(py);
    let metadata = PyDict::new(py);
    metadata.set_item("face_attrs", table)?;
    if !names.is_empty() {
        metadata.set_item("edge_layers", names.keys())?
    }
    let index = if pos.len()? < 1usize << 31 {
        "int32"
    } else {
        "int64"
    };
    let options = PyDict::new(py);
    options.set_item("dtype", index)?;
    out.set_item("meta", metadata)?;
    out.set_item(
        "edges",
        np.call_method("arange", (first_point,), Some(&options))?
            .call_method1("reshape", (-1, 2))?,
    )?;
    out.set_item("flags", flags)?;
    out.set_item(
        "curves",
        curves
            .call_method1("__ge__", (0,))?
            .call_method0("any")?
            .is_truthy()?
            .then_some(curves),
    )?;
    out.set_item("layers", (!names.is_empty()).then_some(layers))?;
    out.set_item(
        "rings",
        np.call_method("arange", (first_point, pos.len()?), Some(&options))?,
    )?;
    out.set_item("pos", pos)?;
    out.set_item("sizes", sizes.call_method1("astype", ("int32",))?)?;
    out.set_item("counts", counts.call_method1("astype", ("int32",))?)?;
    out.set_item("face_attr", face_attr)?;
    out.set_item("sequential", small)?;
    Ok(out)
}
#[pyfunction]
fn igz_merge_native_attrs(src: &Obj<'_>) -> PyResult<()> {
    let py = src.py();
    let codec = api(py)?;
    let json = py.import("json")?;
    let kw = PyDict::new(py);
    kw.set_item("sort_keys", true)?;
    kw.set_item("default", py.import("builtins")?.getattr("str")?)?;
    let table = PyList::empty(py);
    let ids = PyDict::new(py);
    let mut lut = vec![];
    let meta = src.get_item("meta")?;
    for a in meta.get_item("face_attrs")?.try_iter()? {
        let a = a?;
        let i = if codec
            .call_method1("_face_attrs_from_json", (&a,))?
            .is_truthy()?
        {
            let key = json.call_method("dumps", (&a,), Some(&kw))?;
            if let Some(i) = ids.get_item(&key)? {
                i.extract()?
            } else {
                let n = table.len() as i32;
                table.append(a)?;
                ids.set_item(key, n)?;
                n
            }
        } else {
            -1
        };
        lut.push(i)
    }
    lut.push(-1);
    src.set_item(
        "face_attr",
        lut.into_pyarray(py).get_item(src.get_item("face_attr")?)?,
    )?;
    meta.set_item("face_attrs", table)
}
#[pyfunction]
fn igz_adopt_native_blocks(data: &Obj<'_>, blocks: &Obj<'_>) -> PyResult<()> {
    let py = data.py();
    let codec = api(py)?;
    let scene = data.call_method1("get", ("scene",))?;
    if !scene.is_instance_of::<PyDict>() {
        return Ok(());
    }
    let adopt = |obj: &Obj<'_>| -> PyResult<()> {
        let n = obj.call_method1("pop", ("_njb", py.None()))?;
        if !n.is_none() {
            obj.set_item(
                "_arrays",
                codec.call_method1("_native_blocks", (blocks.get_item(n)?,))?,
            )?
        }
        Ok(())
    };
    adopt(&scene)?;
    let protos = scene.call_method1("get", ("protos",))?;
    if protos.is_truthy()? {
        for r in protos.try_iter()? {
            let r = r?;
            if r.is_instance_of::<PyDict>() {
                adopt(&r)?
            }
        }
    }
    for r in py
        .import("ingetrazo_core")?
        .getattr("FileGroups")?
        .call_method1("records", (scene,))?
        .try_iter()?
    {
        adopt(&r?)?
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_native_blocks, m)?)?;
    m.add_function(wrap_pyfunction!(igz_merge_native_attrs, m)?)?;
    m.add_function(wrap_pyfunction!(igz_adopt_native_blocks, m)?)
}
