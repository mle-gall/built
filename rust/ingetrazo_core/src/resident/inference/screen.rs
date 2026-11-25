// SPDX-License-Identifier: GPL-3.0-or-later
//! Camera-independent world bounds and a camera-keyed grid for placement picking.
use numpy::{
    IntoPyArray, PyArray2, PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2, PyReadonlyArray3,
};
use pyo3::{
    exceptions::{PyIndexError, PyValueError},
    prelude::*,
    types::PySet,
};
use std::collections::{HashMap, HashSet};

type Obj<'p> = Bound<'p, PyAny>;
const REACH: f64 = 128.;
const CELL: f64 = 32.;

#[pyclass(module = "ingetrazo_core")]
struct PlacementGrid {
    cells: HashMap<(i64, i64), Vec<i64>>,
    long: Vec<i64>,
}
fn cell(p: f64) -> i64 {
    (p / CELL).floor() as i64
}
impl PlacementGrid {
    fn insert(&mut self, row: usize, low: [f64; 2], high: [f64; 2], width: f64, height: f64) {
        let mut ranges = [(0, 0); 2];
        for (k, size) in [width, height].into_iter().enumerate() {
            if low[k].is_nan() || high[k].is_nan() || low[k] > size + REACH || high[k] < -REACH {
                return;
            }
            ranges[k] = (cell(low[k].max(-REACH)), cell(high[k].min(size + REACH)));
        }
        let [(x0, x1), (y0, y1)] = ranges;
        if (x1 as i128 - x0 as i128 + 1) * (y1 as i128 - y0 as i128 + 1) > 16 {
            self.long.push(row as i64);
        } else {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    self.cells.entry((x, y)).or_default().push(row as i64);
                }
            }
        }
    }
    fn near(&self, x: f64, y: f64, r: f64) -> Vec<i64> {
        let mut ids = self.long.clone();
        for cy in cell(y - r)..=cell(y + r) {
            for cx in cell(x - r)..=cell(x + r) {
                if let Some(rows) = self.cells.get(&(cx, cy)) {
                    ids.extend_from_slice(rows);
                }
            }
        }
        ids
    }
}

fn refresh_world<'p>(
    index: &Obj<'p>,
    packed: &Obj<'p>,
    n: usize,
    dirty: &[usize],
) -> PyResult<Obj<'p>> {
    let py = index.py();
    let cached = index.getattr_opt("world_box_cache")?;
    let valid = match &cached {
        Some(c) if !c.is_none() => c.get_item(0)?.len()? == n,
        _ => false,
    };
    let cache = if valid {
        cached.unwrap()
    } else {
        (
            PyArray2::<f64>::zeros(py, [n, 3], false),
            PyArray2::<f64>::zeros(py, [n, 3], false),
        )
            .into_pyobject(py)?
            .into_any()
    };
    if !valid || !dirty.is_empty() {
        let instances = packed.getattr("instances")?;
        let info = instances
            .get_item("info")?
            .extract::<PyReadonlyArray2<u32>>()?;
        let world = instances
            .get_item("world")?
            .extract::<PyReadonlyArray3<f32>>()?;
        let bounds = packed
            .getattr("bounds")?
            .extract::<PyReadonlyArray3<f32>>()?;
        let (info, world, bounds) = (info.as_array(), world.as_array(), bounds.as_array());
        let (wc, we) = (cache.get_item(0)?, cache.get_item(1)?);
        let mut wc = wc.cast::<PyArray2<f64>>()?.try_readwrite()?;
        let mut we = we.cast::<PyArray2<f64>>()?.try_readwrite()?;
        let (mut wc, mut we) = (wc.as_array_mut(), we.as_array_mut());
        if world.dim() != (n, 4, 4)
            || info.nrows() != n
            || info.ncols() < 1
            || bounds.shape()[1] < 2
            || bounds.shape()[2] < 3
            || wc.dim() != (n, 3)
            || we.dim() != (n, 3)
        {
            return Err(PyValueError::new_err(
                "invalid placement bounds or transform dimensions",
            ));
        }
        let mut update = |i: usize| -> PyResult<()> {
            if i >= n {
                return Err(PyIndexError::new_err(i));
            }
            let code = info[[i, 0]] as usize;
            if code >= bounds.shape()[0] {
                return Err(PyIndexError::new_err(code));
            }
            for j in 0..3 {
                let (mut c, mut e) = (0., 0.);
                for k in 0..3 {
                    let (lo, hi) = (bounds[[code, 0, k]] as f64, bounds[[code, 1, k]] as f64);
                    let v = world[[i, k, j]] as f64;
                    c += v * ((lo + hi) * 0.5);
                    e += v.abs() * ((hi - lo) * 0.5);
                }
                wc[[i, j]] = c + world[[i, 3, j]] as f64;
                we[[i, j]] = e;
            }
            Ok(())
        };
        if valid {
            for &i in dirty {
                update(i)?;
            }
        } else {
            for i in 0..n {
                update(i)?;
            }
        }
    }
    if !valid {
        index.setattr("world_box_cache", &cache)?;
    }
    Ok(cache)
}

#[pyfunction]
pub(super) fn screen_boxes<'p>(
    index: &Obj<'p>,
    packed: &Obj<'p>,
    mvp: &Obj<'p>,
    width: f64,
    height: f64,
) -> PyResult<Obj<'p>> {
    let py = index.py();
    let n = packed.getattr("instances")?.len()?;
    let m: [f64; 16] = mvp.call_method0("data")?.extract()?;
    let key = (m, width, height, n).into_pyobject(py)?;
    let old = index.getattr("screen_cache")?;
    let same = !old.is_none() && old.get_item(0)?.eq(&key)?;
    let pending = index.getattr("screen_dirty")?;
    if same && pending.is_empty()? {
        return Ok(old.get_item(1)?);
    }
    let dirty: Vec<usize> = pending
        .try_iter()?
        .map(|r| r?.extract())
        .collect::<PyResult<_>>()?;
    let world = refresh_world(index, packed, n, &dirty)?;
    let (wc, we) = (
        world.get_item(0)?.extract::<PyReadonlyArray2<f64>>()?,
        world.get_item(1)?.extract::<PyReadonlyArray2<f64>>()?,
    );
    let (wc, we) = (wc.as_array(), we.as_array());
    let previous = if same { Some(old.get_item(1)?) } else { None };
    let (lo, hi, moved) = if let Some(p) = &previous {
        (
            p.get_item(0)?.cast_into::<PyArray2<f64>>()?,
            p.get_item(1)?.cast_into::<PyArray2<f64>>()?,
            p.get_item(3)?.cast_into::<PySet>()?,
        )
    } else {
        (
            PyArray2::zeros(py, [n, 2], false),
            PyArray2::zeros(py, [n, 2], false),
            PySet::empty(py)?,
        )
    };
    let mut grid = PlacementGrid {
        cells: HashMap::new(),
        long: Vec::new(),
    };
    {
        let (mut l, mut h) = (lo.try_readwrite()?, hi.try_readwrite()?);
        let (mut l, mut h) = (l.as_array_mut(), h.as_array_mut());
        let mut update = |i: usize| -> PyResult<()> {
            if i >= n {
                return Err(PyIndexError::new_err(i));
            }
            let (low, high) = super::project_box(
                std::array::from_fn(|k| wc[[i, k]]),
                std::array::from_fn(|k| we[[i, k]]),
                &m,
                width,
                height,
            );
            for k in 0..2 {
                l[[i, k]] = low[k];
                h[[i, k]] = high[k];
            }
            if same {
                moved.add(i)?;
            } else {
                grid.insert(i, low, high, width, height);
            }
            Ok(())
        };
        if same {
            for i in dirty {
                update(i)?;
            }
        } else {
            for i in 0..n {
                update(i)?;
            }
        }
    }
    pending.call_method0("clear")?;
    if let Some(p) = previous {
        return Ok(p);
    }
    let boxes = (lo, hi, Py::new(py, grid)?, moved)
        .into_pyobject(py)?
        .into_any();
    index.setattr("screen_cache", (key, &boxes))?;
    Ok(boxes)
}

#[pyfunction(signature=(index,packed=None))]
pub(super) fn eligible_rows<'p>(index: &Obj<'p>, packed: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let packed = match packed {
        Some(p) => p.clone(),
        None => index.getattr("packed")?,
    };
    let info = packed
        .getattr("instances")?
        .get_item("info")?
        .extract::<PyReadonlyArray2<u32>>()?;
    let info = info.as_array();
    let rows: Vec<_> = info
        .column(2)
        .iter()
        .enumerate()
        .filter_map(|(i, f)| (f & 129 == 0).then_some(i as i64))
        .collect();
    Ok(rows.into_pyarray(index.py()).into_any())
}

#[pyfunction(signature=(index,packed,mvp,width,height,x,y,radius,rows))]
pub(super) fn screen_rows<'p>(
    index: &Obj<'p>,
    packed: &Obj<'p>,
    mvp: &Obj<'p>,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
    radius: f64,
    rows: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let boxes = screen_boxes(index, packed, mvp, width, height)?;
    let (lo, hi) = (
        boxes.get_item(0)?.extract::<PyReadonlyArray2<f64>>()?,
        boxes.get_item(1)?.extract::<PyReadonlyArray2<f64>>()?,
    );
    let (lo, hi) = (lo.as_array(), hi.as_array());
    let info = packed
        .getattr("instances")?
        .get_item("info")?
        .extract::<PyReadonlyArray2<u32>>()?;
    let info = info.as_array();
    let subset = rows
        .map(|r| match r.extract::<PyReadonlyArray1<i64>>() {
            Ok(values) => Ok(values.as_array().to_vec()),
            Err(_) => r.extract::<Vec<i64>>(),
        })
        .transpose()?;
    let candidates = if (0.0..=width).contains(&x)
        && (0.0..=height).contains(&y)
        && (0.0..=REACH).contains(&radius)
    {
        let grid = boxes.get_item(2)?;
        let mut ids = grid.extract::<PyRef<PlacementGrid>>()?.near(x, y, radius);
        for r in boxes.get_item(3)?.try_iter()? {
            ids.push(r?.extract()?);
        }
        ids.sort_unstable();
        ids.dedup();
        if let Some(subset) = subset {
            let allowed: HashSet<_> = subset.into_iter().collect();
            ids.retain(|r| allowed.contains(r));
        }
        ids
    } else {
        subset.unwrap_or_else(|| (0..lo.nrows() as i64).collect())
    };
    let mut result = Vec::new();
    for r in candidates {
        let i = if r < 0 { lo.nrows() as i64 + r } else { r } as usize;
        if i >= info.nrows() {
            return Err(PyIndexError::new_err(r));
        }
        if info[[i, 2]] & 129 != 0 {
            continue;
        }
        let gap = |k: usize, p: f64| {
            let (a, b) = (lo[[i, k]] - p, p - hi[[i, k]]);
            if a.is_nan() || b.is_nan() {
                f64::NAN
            } else {
                a.max(b).max(0.)
            }
        };
        let (dx, dy) = (gap(0, x), gap(1, y));
        if dx * dx + dy * dy < radius * radius {
            result.push(r);
        }
    }
    Ok(result.into_pyarray(index.py()).into_any())
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(screen_rows, m)?)?;
    m.add_function(wrap_pyfunction!(eligible_rows, m)?)?;
    m.add_function(wrap_pyfunction!(screen_boxes, m)?)
}
