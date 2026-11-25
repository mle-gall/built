// SPDX-License-Identifier: GPL-3.0-or-later
//! Grab-time vertices for Push/Pull. Staging can replace the document topology
//! during a drag, so inference uses this immutable copy until the next grab.
use super::{host_geometry::Projection, view_queries::hit, P3};
use crate::{
    math::{xyz, Vec3},
    pick::Projector,
    store::{keys::Kind, py::PyMeshStore},
};
use glam::DVec3;
use numpy::{ndarray::Array2, IntoPyArray, PyArray2, PyReadonlyArray2};
use pyo3::{exceptions::PyValueError, prelude::*};
use std::collections::{HashMap, HashSet};

type Frame = ([f64; 16], f64, f64);
type Candidate = (f64, usize, usize); // squared pixel distance, mesh, vertex in mesh order
const CELL: f64 = 32.0;
fn cell(px: f64) -> i64 {
    (px / CELL).floor() as i64
}

fn matrix<'py>(
    py: Python<'py>,
    rows: impl Iterator<Item = P3>,
    len: usize,
) -> Bound<'py, PyArray2<f64>> {
    Array2::from_shape_vec((len, 3), rows.flatten().collect())
        .unwrap()
        .into_pyarray(py)
}
fn read_rows(a: &Bound<'_, PyAny>) -> PyResult<Vec<P3>> {
    macro_rules! read {
        ($t:ty) => {
            if let Ok(a) = a.extract::<PyReadonlyArray2<$t>>() {
                let rows = a.as_array();
                if rows.ncols() == 3 {
                    return Ok(rows
                        .rows()
                        .into_iter()
                        .map(|r| [r[0] as f64, r[1] as f64, r[2] as f64])
                        .collect());
                }
            }
        };
    }
    read!(f64);
    read!(f32);
    Err(PyValueError::new_err("expected an N by 3 position array"))
}
fn mesh_points(mesh: &Bound<'_, PyAny>) -> PyResult<Vec<P3>> {
    let plan = mesh
        .py()
        .import("built_core")?
        .call_method1("plan_of", (mesh,))?;
    if !plan.is_none() {
        return read_rows(&plan.getattr("pos")?);
    }
    if let Some(store) = mesh
        .getattr_opt("_store")?
        .filter(|s| s.is_instance_of::<PyMeshStore>())
    {
        let store = PyMeshStore::read(&store)?;
        return Ok(store
            .inner
            .live_slots(Kind::Vertex)
            .into_iter()
            .map(|v| *store.inner.state.vpos.get(v as usize))
            .collect());
    }
    mesh.getattr("vertices")?
        .try_iter()?
        .map(|v| Ok(xyz(&v?.getattr("position")?)?.to_array()))
        .collect()
}

#[pyclass(module = "built_core")]
pub struct PushReferences {
    points: Vec<Vec<P3>>,
    frame: Option<Frame>,
    cells: HashMap<(i64, i64), Vec<(usize, usize)>>,
    custom_frame: Option<(Vec<u8>, usize, usize)>,
    bounds: Vec<Option<[f64; 4]>>,
}
impl PushReferences {
    fn native_hits(&mut self, frame: Frame, px: [f64; 2], radius: f64) -> Vec<Candidate> {
        let cam = Projector::new(&frame.0, frame.1, frame.2);
        if self.frame != Some(frame) {
            self.cells.clear();
            for (m, mesh) in self.points.iter().enumerate() {
                for (i, &p) in mesh.iter().enumerate() {
                    let (x, y, ok) = cam.px(p);
                    if ok {
                        self.cells
                            .entry((cell(x), cell(y)))
                            .or_default()
                            .push((m, i));
                    }
                }
            }
            self.frame = Some(frame);
        }
        let mut hits = vec![];
        let mut scan = |rows: &[(usize, usize)]| {
            for &(m, i) in rows {
                let (x, y, _) = cam.px(self.points[m][i]);
                let dd = (x - px[0]).powi(2) + (y - px[1]).powi(2);
                if dd <= radius * radius {
                    hits.push((dd, m, i));
                }
            }
        };
        if radius > 512. {
            for rows in self.cells.values() {
                scan(rows);
            }
        } else {
            for x in cell(px[0] - radius)..=cell(px[0] + radius) {
                for y in cell(px[1] - radius)..=cell(px[1] + radius) {
                    if let Some(rows) = self.cells.get(&(x, y)) {
                        scan(rows);
                    }
                }
            }
        }
        hits
    }

    // Extension viewports may supply a batch projector or only a scalar
    // callback. Cache rejection bounds only when they identify their camera.
    fn custom_hits(
        &mut self,
        view: &Bound<'_, PyAny>,
        px: [f64; 2],
        r: f64,
    ) -> PyResult<Vec<Candidate>> {
        let batch = view.getattr_opt("_project_px")?;
        let proj = Projection::from_view(view)?;
        let key = if let Some(mvp) = view.getattr_opt("_np_mvp")? {
            Some((
                mvp.call0()?.call_method0("tobytes")?.extract()?,
                view.call_method0("width")?.extract()?,
                view.call_method0("height")?.extract()?,
            ))
        } else {
            None
        };
        if key.is_none() || self.custom_frame != key {
            self.bounds.fill(None);
            self.custom_frame = key;
        }
        let mut hits = vec![];
        for (m, mesh) in self.points.iter().enumerate() {
            if self.bounds[m].is_some_and(|[lx, ly, hx, hy]| {
                px[0] + r + 1. < lx
                    || px[1] + r + 1. < ly
                    || px[0] - r - 1. > hx
                    || px[1] - r - 1. > hy
            }) {
                continue;
            }
            let mut b = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            let mut visit = |i: usize, x: f64, y: f64| {
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
                let dd = (x - px[0]).powi(2) + (y - px[1]).powi(2);
                if dd <= r * r {
                    hits.push((dd, m, i));
                }
            };
            if let Some(batch) = &batch {
                let (x, y, ok): (Vec<f64>, Vec<f64>, Vec<bool>) = batch
                    .call1((matrix(view.py(), mesh.iter().copied(), mesh.len()),))?
                    .extract()?;
                for (i, ((x, y), ok)) in x.into_iter().zip(y).zip(ok).take(mesh.len()).enumerate() {
                    if ok {
                        visit(i, x, y)
                    }
                }
            } else {
                for (i, &p) in mesh.iter().enumerate() {
                    if let Some([x, y]) = proj.pixel(p.into())? {
                        visit(i, x, y)
                    }
                }
            }
            self.bounds[m] = Some(b);
        }
        Ok(hits)
    }
}
#[pymethods]
impl PushReferences {
    #[new]
    #[pyo3(signature=(view, px, cached=None))]
    pub(super) fn new(
        view: &Bound<'_, PyAny>,
        px: (f64, f64),
        cached: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let mut points = vec![];
        if let Some(rows) = cached {
            for pair in rows.try_iter()? {
                points.push(read_rows(&pair?.get_item(1)?)?);
            }
        } else {
            let scene = view.getattr("scene")?;
            let meshes = if let Some(meshes) = hit(view, "inference_meshes", px)? {
                meshes.try_iter()?.collect::<PyResult<Vec<_>>>()?
            } else {
                let mut out = vec![scene.getattr("mesh")?];
                if let Some(groups) = scene.getattr_opt("groups")? {
                    for g in groups.try_iter()? {
                        out.push(g?.getattr("mesh")?);
                    }
                }
                out
            };
            let mut seen = HashSet::new();
            for mesh in meshes {
                if seen.insert(mesh.as_ptr()) {
                    let p = mesh_points(&mesh)?;
                    if !p.is_empty() {
                        points.push(p);
                    }
                }
            }
        }
        Ok(Self {
            bounds: vec![None; points.len()],
            points,
            frame: None,
            cells: HashMap::new(),
            custom_frame: None,
        })
    }
    pub(super) fn nearest(
        &mut self,
        view: &Bound<'_, PyAny>,
        px: [f64; 2],
        radius: f64,
        excluded: &Bound<'_, PyAny>,
        hidden: &Bound<'_, PyAny>,
    ) -> PyResult<Option<(f64, Vec3)>> {
        let key = super::adapters::imported(view.py(), "built_core.topology")?.getattr("_key")?;
        self.nearest_where(view, px, radius, hidden, |point| {
            excluded.contains(key.call1((point,))?)
        })
    }
}

impl PushReferences {
    pub(super) fn nearest_where(
        &mut self,
        view: &Bound<'_, PyAny>,
        px: [f64; 2],
        radius: f64,
        hidden: &Bound<'_, PyAny>,
        excluded: impl Fn(&Bound<'_, PyAny>) -> PyResult<bool>,
    ) -> PyResult<Option<(f64, Vec3)>> {
        let mut frame = None;
        if let Some(f) = view.getattr_opt("_camera_px")? {
            match f.call0() {
                Ok(v) => frame = Some(v.extract()?),
                Err(e) if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(view.py()) => {}
                Err(e) => return Err(e),
            }
        }
        let mut hits = match frame {
            Some(f) => self.native_hits(f, px, radius),
            None => self.custom_hits(view, px, radius)?,
        };
        if hits.is_empty() {
            return Ok(None);
        }
        // Ordered distance, then original mesh and vertex order for ties.
        hits.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let p = |&(_, m, i): &Candidate| self.points[m][i];
        let eligible = |h: &Candidate| -> PyResult<bool> {
            Ok(!excluded(
                Bound::new(view.py(), Vec3::from(DVec3::from(p(h))))?.as_any(),
            )?)
        };
        let mut remaining = &hits[..];
        if let Some(clear) = view.getattr_opt("_first_clear")? {
            // The occlusion engine tests the coordinates represented by its
            // float32 scene, while the accepted measurement stays in float64.
            let all = matrix(
                view.py(),
                hits.iter().map(|h| p(h).map(|x| x as f32 as f64)),
                hits.len(),
            );
            while !remaining.is_empty() {
                let slice = pyo3::types::PySlice::new(
                    view.py(),
                    (hits.len() - remaining.len()) as isize,
                    hits.len() as isize,
                    1,
                );
                let pts = all.get_item(slice)?;
                let Some(i) = clear.call1((pts,))?.extract::<Option<isize>>()? else {
                    break;
                };
                if i < 0 {
                    return Ok(None);
                }
                let h = remaining
                    .get(i as usize)
                    .ok_or_else(|| PyValueError::new_err("invalid visible vertex index"))?;
                if eligible(h)? {
                    return Ok(Some((h.0, DVec3::from(p(h)).into())));
                }
                remaining = &remaining[i as usize + 1..];
            }
        }
        for h in remaining {
            if eligible(h)?
                && !hidden
                    .call1((view, Vec3::from(DVec3::from(p(h)))))?
                    .is_truthy()?
            {
                return Ok(Some((h.0, DVec3::from(p(h)).into())));
            }
        }
        Ok(None)
    }
}
