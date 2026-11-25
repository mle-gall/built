// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use glam::{DMat3, DVec3};
use pyo3::types::PyTuple;
use std::io::{BufRead, BufReader};
fn numbers<const N: usize>(entry: &Obj, key: &str) -> Option<[f64; N]> {
    let seq = entry.get_item(key).ok()?;
    let float = entry.py().get_type::<pyo3::types::PyFloat>();
    seq.try_iter()
        .ok()?
        .map(|v| float.call1((v?,))?.extract::<f64>())
        .collect::<PyResult<Vec<_>>>()
        .ok()?
        .try_into()
        .ok()
}
fn bounds(obj: &Path, rotation: DMat3) -> Option<DVec3> {
    let file = fs::File::open(obj).ok()?;
    let mut positions = vec![];
    let mut used = HashSet::new();
    for row in BufReader::new(file).split(b'\n') {
        let row = row.ok()?;
        let line = String::from_utf8_lossy(&row);
        let mut tokens = line.split_whitespace();
        match tokens.next() {
            Some("v") => {
                if let (Some(x), Some(y), Some(z)) = (tokens.next(), tokens.next(), tokens.next()) {
                    let point = || {
                        Some(
                            rotation
                                * DVec3::new(x.parse().ok()?, y.parse().ok()?, z.parse().ok()?),
                        )
                    };
                    positions.push(point());
                }
            }
            Some("f") => {
                for field in tokens {
                    if let Some(i) = field.split('/').next().and_then(|v| v.parse::<i64>().ok()) {
                        if i != 0 {
                            let index = if i > 0 {
                                i - 1
                            } else {
                                positions.len() as i64 + i
                            };
                            if let Ok(i) = usize::try_from(index) {
                                used.insert(i);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for (i, p) in positions.into_iter().enumerate() {
        if used.is_empty() || used.contains(&i) {
            if let Some(p) = p {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
    }
    Some(hi - lo)
}
#[pyfunction]
pub(super) fn model_matrix<'p>(entry: &Obj<'p>, obj: PathBuf) -> PyResult<Bound<'p, PyTuple>> {
    let py = entry.py();
    let rot = numbers::<9>(entry, "rot")
        .map(|v| DMat3::from_cols_array(&v).transpose())
        .unwrap_or(DMat3::IDENTITY);
    let [w, d, h] = numbers(entry, "cm").unwrap_or([0.; 3]);
    let wanted = DVec3::new(w, h, d) / 100.;
    let ext = py.detach(|| bounds(&obj, rot)).unwrap_or(DVec3::ZERO);
    let mut valid = vec![];
    let mut scale = DVec3::ONE;
    for k in 0..3 {
        if wanted[k] > 0. && ext[k] > 1e-9 {
            scale[k] = wanted[k] / ext[k];
            valid.push(scale[k]);
        }
    }
    let fallback = if valid.is_empty() {
        0.01
    } else {
        valid.iter().sum::<f64>() / valid.len() as f64
    };
    for k in 0..3 {
        if !(wanted[k] > 0. && ext[k] > 1e-9) {
            scale[k] = fallback;
        }
    }
    let up = DMat3::from_cols(DVec3::X, DVec3::Z, -DVec3::Y);
    PyTuple::new(
        py,
        (up * DMat3::from_diagonal(scale) * rot)
            .transpose()
            .to_cols_array(),
    )
}
