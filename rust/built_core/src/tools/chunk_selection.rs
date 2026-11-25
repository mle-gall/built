// SPDX-License-Identifier: GPL-3.0-or-later
//! Box selection for viewports that expose prototype chunks. Borrow chunk
//! arrays and project bounded batches, including both ends of crossing wires.
use super::P3;
use crate::pick::{inside, seg_rect, Projector, Rect};
use numpy::{ndarray::Array2, IntoPyArray, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::{
    exceptions::{PyAttributeError, PyValueError},
    prelude::*,
    types::{PyDict, PyTuple},
};

const BATCH: usize = 8192;
enum Rows<'py> {
    F32(PyReadonlyArray2<'py, f32>),
    F64(PyReadonlyArray2<'py, f64>),
}
impl<'py> Rows<'py> {
    fn read(arr: &Bound<'py, PyAny>) -> PyResult<Self> {
        let out = if let Ok(a) = arr.extract::<PyReadonlyArray2<'_, f64>>() {
            Self::F64(a)
        } else {
            Self::F32(arr.extract()?)
        };
        let cols = match &out {
            Self::F32(a) => a.as_array().ncols(),
            Self::F64(a) => a.as_array().ncols(),
        };
        if cols != 3 {
            return Err(PyValueError::new_err("expected three coordinates"));
        }
        Ok(out)
    }
    fn len(&self) -> usize {
        match self {
            Self::F32(a) => a.as_array().nrows(),
            Self::F64(a) => a.as_array().nrows(),
        }
    }
    fn at(&self, i: usize) -> P3 {
        match self {
            Self::F32(a) => std::array::from_fn(|j| a.as_array()[[i, j]] as f64),
            Self::F64(a) => std::array::from_fn(|j| a.as_array()[[i, j]]),
        }
    }
}
fn map(v: P3, m: &[f64; 16], point: bool) -> P3 {
    std::array::from_fn(|j| {
        ((v[0] * m[j] + v[1] * m[4 + j]) + v[2] * m[8 + j]) + if point { m[12 + j] } else { 0. }
    })
}

struct Chunk<'py> {
    tri: Option<[Rows<'py>; 3]>,
    edges: PyReadonlyArray1<'py, u8>,
}
impl<'py> Chunk<'py> {
    fn new(ch: &Bound<'py, PyAny>) -> PyResult<Self> {
        let a = ch.get_item("v0")?;
        let tri = if a.is_none() {
            None
        } else {
            let r = [
                Rows::read(&a)?,
                Rows::read(&ch.get_item("e1")?)?,
                Rows::read(&ch.get_item("e2")?)?,
            ];
            if r[1].len() != r[0].len() || r[2].len() != r[0].len() {
                return Err(PyValueError::new_err("mismatched triangle rows"));
            }
            Some(r)
        };
        let edges = ch
            .py()
            .import("numpy")?
            .call_method1("frombuffer", (ch.get_item("edges")?, "uint8"))?
            .extract::<PyReadonlyArray1<u8>>()?;
        if edges.as_slice()?.len() % 24 != 0 {
            return Err(PyValueError::new_err("expected pairs of edge endpoints"));
        }
        Ok(Self { tri, edges })
    }
    // Return as soon as the consumer has a decisive result.
    fn visit(
        &self,
        m: Option<&[f64; 16]>,
        mut batch: impl FnMut(&[P3], bool) -> PyResult<bool>,
    ) -> PyResult<bool> {
        let mut rows = Vec::new();
        if let Some(tri) = &self.tri {
            for start in (0..tri[0].len()).step_by(BATCH) {
                let end = (start + BATCH).min(tri[0].len());
                for k in 0..3 {
                    rows.clear();
                    for i in start..end {
                        let a = tri[0].at(i);
                        let a = m.map_or(a, |m| map(a, m, true));
                        let p = if k == 0 {
                            a
                        } else {
                            let v = tri[k].at(i);
                            let d = m.map_or(v, |m| map(v, m, false));
                            let float = m.is_none()
                                && matches!(tri[0], Rows::F32(_))
                                && matches!(tri[k], Rows::F32(_));
                            std::array::from_fn(|j| {
                                if float {
                                    (a[j] + d[j]) as f32 as f64
                                } else {
                                    a[j] + d[j]
                                }
                            })
                        };
                        rows.push(p);
                    }
                    if batch(&rows, false)? {
                        return Ok(true);
                    }
                }
            }
        }
        for ends in self.edges.as_slice()?.chunks(BATCH * 12) {
            rows.clear();
            for point in ends.chunks_exact(12) {
                let p = std::array::from_fn(|j| {
                    f32::from_ne_bytes(point[j * 4..j * 4 + 4].try_into().unwrap()) as f64
                });
                rows.push(m.map_or(p, |m| map(p, m, true).map(|x| x as f32 as f64)));
            }
            if batch(&rows, true)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[pyfunction]
pub fn chunk_bounds<'py>(ch: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = ch.py();
    let arrays = [ch.get_item("v0")?, ch.get_item("e1")?, ch.get_item("e2")?];
    let edges = ch.get_item("edges")?;
    if let Some(hit) = ch.cast::<PyDict>()?.get_item("_box_bounds")? {
        if let Ok(t) = hit.cast::<PyTuple>() {
            if t.len() == 3 && t.get_item(1)?.is(&edges) {
                if let Ok(refs) = t.get_item(0)?.cast_into::<PyTuple>() {
                    let mut valid = refs.len() == 3;
                    for (r, a) in refs.iter().zip(&arrays) {
                        valid &= if r.is_none() {
                            a.is_none()
                        } else {
                            !a.is_none() && r.call0()?.is(a)
                        };
                    }
                    if valid {
                        return t.get_item(2);
                    }
                }
            }
        }
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    Chunk::new(ch)?.visit(None, |pts, _| {
        for p in pts {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        Ok(false)
    })?;
    let bounds = if lo[0] > hi[0] {
        py.None().into_bound(py)
    } else {
        (lo.to_vec().into_pyarray(py), hi.to_vec().into_pyarray(py))
            .into_pyobject(py)?
            .into_any()
    };
    let weak = py.import("weakref")?.getattr("ref")?;
    let refs = arrays
        .iter()
        .map(|a| {
            if a.is_none() {
                Ok(a.clone())
            } else {
                weak.call1((a,))
            }
        })
        .collect::<PyResult<Vec<_>>>()?;
    ch.set_item("_box_bounds", (PyTuple::new(py, refs)?, edges, &bounds))?;
    Ok(bounds)
}

enum Camera<'py> {
    Native(Projector),
    Batch(Bound<'py, PyAny>),
}
impl<'py> Camera<'py> {
    fn new(view: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Some(f) = view.getattr_opt("_camera_px")? {
            match f.call0() {
                Ok(v) => {
                    let (m, w, h): ([f64; 16], f64, f64) = v.extract()?;
                    return Ok(Self::Native(Projector::new(&m, w, h)));
                }
                Err(e) if e.is_instance_of::<PyAttributeError>(view.py()) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(Self::Batch(view.getattr("_project_px")?))
    }
    fn project(&self, pts: &[P3]) -> PyResult<Vec<(f64, f64, bool)>> {
        if let Self::Native(cam) = self {
            return Ok(pts.iter().map(|&p| cam.px(p)).collect());
        }
        let Self::Batch(f) = self else { unreachable!() };
        let rows = Array2::from_shape_vec((pts.len(), 3), pts.iter().flatten().copied().collect())
            .unwrap()
            .into_pyarray(f.py());
        let output = f.call1((rows,))?;
        let cast = f.py().import("numpy")?.getattr("asarray")?;
        let x = cast
            .call1((output.get_item(0)?, "float64"))?
            .extract::<PyReadonlyArray1<f64>>()?;
        let y = cast
            .call1((output.get_item(1)?, "float64"))?
            .extract::<PyReadonlyArray1<f64>>()?;
        let ok = cast
            .call1((output.get_item(2)?, "bool"))?
            .extract::<PyReadonlyArray1<bool>>()?;
        if [x.as_array().len(), y.as_array().len(), ok.as_array().len()] != [pts.len(); 3] {
            return Err(PyValueError::new_err(
                "projector returned a different number of points",
            ));
        }
        Ok(x.as_array()
            .iter()
            .zip(y.as_array().iter())
            .zip(ok.as_array().iter())
            .map(|((&x, &y), &ok)| (x, y, ok))
            .collect())
    }
}

#[pyfunction]
pub fn box_chunk_group(
    view: &Bound<'_, PyAny>,
    group: &Bound<'_, PyAny>,
    rect: Rect,
    crossing: bool,
) -> PyResult<Option<bool>> {
    if group
        .getattr_opt("billboard")?
        .is_some_and(|x| x.is_truthy().unwrap_or(false))
    {
        return Ok(None);
    }
    let Some(chunk) = view.getattr_opt("_group_chunk")? else {
        return Ok(None);
    };
    let cam = Camera::new(view)?;
    let places = if let Some(expand) = view.getattr_opt("_expand_placements")? {
        expand
            .call1((group,))?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?
    } else {
        vec![group.clone()]
    };
    let mut seen = false;
    for place in places {
        let xf = place.getattr_opt("xform")?.filter(|v| !v.is_none());
        let mesh = place.getattr_opt("mesh")?.filter(|v| !v.is_none());
        let (source, m) = if let Some((xf, mesh)) = xf.zip(mesh) {
            let api = view.py().import("views.document_interaction")?;
            let paint = api.call_method1("_paint_of", (view, &place))?;
            let proto = api.call_method1("_proto_wrapper", (view, mesh, paint))?;
            (
                chunk.call1((proto,))?,
                Some(xf.call_method0("data")?.extract::<[f64; 16]>()?),
            )
        } else {
            (chunk.call1((&place,))?, None)
        };
        let limits = chunk_bounds(&source)?;
        if limits.is_none() {
            continue;
        }
        seen = true;
        let (lo, hi): (P3, P3) = limits.extract()?;
        let mut box_: [P3; 8] = std::array::from_fn(|i| {
            let corner = std::array::from_fn(|j| if i & (1 << j) == 0 { lo[j] } else { hi[j] });
            m.as_ref().map_or(corner, |m| map(corner, m, true))
        });
        if m.is_some() && !source.get_item("edges")?.is_empty()? {
            // Packed edge endpoints round after placement. Their rounded
            // coordinate can lie just outside the transformed prototype box.
            let low: P3 = std::array::from_fn(|j| {
                let v = box_.iter().map(|p| p[j]).fold(f64::INFINITY, f64::min);
                v.min(v as f32 as f64)
            });
            let high: P3 = std::array::from_fn(|j| {
                let v = box_.iter().map(|p| p[j]).fold(f64::NEG_INFINITY, f64::max);
                v.max(v as f32 as f64)
            });
            box_ = std::array::from_fn(|i| {
                std::array::from_fn(|j| if i & (1 << j) == 0 { low[j] } else { high[j] })
            });
        }
        let corners = cam.project(&box_)?;
        if corners.iter().all(|p| p.2) {
            if corners.iter().all(|p| inside(&rect, p.0, p.1)) {
                if crossing {
                    return Ok(Some(true));
                }
                continue;
            }
            if corners.iter().all(|p| p.0 < rect[0])
                || corners.iter().all(|p| p.1 < rect[1])
                || corners.iter().all(|p| p.0 > rect[2])
                || corners.iter().all(|p| p.1 > rect[3])
            {
                if !crossing {
                    return Ok(Some(false));
                }
                continue;
            }
        }
        let stop = Chunk::new(&source)?.visit(m.as_ref(), |pts, paired| {
            let pixels = cam.project(pts)?;
            if pixels
                .iter()
                .any(|&(x, y, ok)| (ok && inside(&rect, x, y)) == crossing)
            {
                return Ok(true);
            }
            Ok(crossing
                && paired
                && pixels.chunks_exact(2).any(|p| {
                    seg_rect(
                        (p[0].0, p[0].1),
                        (p[1].0, p[1].1),
                        p[0].2 && p[1].2,
                        &rect,
                        true,
                    )
                }))
        })?;
        if stop {
            return Ok(Some(crossing));
        }
    }
    Ok(seen.then_some(!crossing))
}
