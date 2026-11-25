// SPDX-License-Identifier: GPL-3.0-or-later
//! The interval over which the topology of a staged extrusion stays valid.
//! Each moving vertex advances on the intersection of its fixed wall planes.
use crate::{
    math::xyz,
    store::{
        geometry::Hypot,
        keys::Kind,
        py::{PyMeshStore, PyStoreSnapshot},
        Store,
    },
};
use glam::DVec3;
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

fn slide_rate(normals: &[DVec3], n: DVec3) -> Option<DVec3> {
    let nn = n.normalize_or_zero();
    let mut planes: Vec<DVec3> = vec![];
    for &m in normals {
        if planes.iter().all(|o| m.cross(*o).length() > 1e-3) {
            planes.push(m);
        }
    }
    if planes.iter().all(|m| m.dot(nn).abs() < 1e-3) {
        return Some(n);
    }
    let m = planes[0];
    let line = if planes.len() == 1 {
        nn - m * nn.dot(m)
    } else {
        m.cross(planes[1])
    };
    let along = line.dot(nn);
    if line.length() < 1e-9 || along.abs() < 1e-3 * line.length() {
        return None;
    }
    let rate = line * (n.dot(nn) / along);
    planes
        .iter()
        .all(|m| m.dot(rate).abs() <= 1e-6 * rate.length())
        .then_some(rate)
}
struct Mover {
    key: u128,
    point: DVec3,
    rate: DVec3,
}

#[pyclass(module = "built_core")]
pub struct PushSlide {
    movers: Vec<Mover>,
    #[pyo3(get)]
    interval: (f64, f64),
    #[pyo3(get)]
    faces: Vec<u128>,
    #[pyo3(get)]
    edges: Vec<u128>,
}
#[pymethods]
impl PushSlide {
    #[staticmethod]
    #[allow(clippy::too_many_arguments)]
    fn resolve(
        store: &Bound<'_, PyAny>,
        before: PyRef<'_, PyStoreSnapshot>,
        anchor: &Bound<'_, PyAny>,
        normal: &Bound<'_, PyAny>,
        distance: f64,
        mut levels: Vec<f64>,
        cap: &Bound<'_, PyAny>,
        tolerance: f64,
    ) -> PyResult<Option<Self>> {
        let st = PyMeshStore::read(store)?;
        let s = &st.inner;
        let (anchor, n) = (xyz(anchor)?, xyz(normal)?);
        let pos = |v: u32| DVec3::from(*s.state.vpos.get(v as usize));
        let height = |p: DVec3| (p - anchor).dot(n);
        let made = |k| s.new_slots_since(&before.inner, k);
        let created = made(Kind::Vertex);
        let moving: Vec<_> = created
            .iter()
            .copied()
            .filter(|&v| (height(pos(v)) - distance).abs() < tolerance)
            .collect();
        let moving_set: HashSet<_> = moving.iter().copied().collect();
        let mut walls: HashMap<u32, Vec<DVec3>> = HashMap::new();
        for f in s.faces_around(&moving) {
            let vertices: Vec<_> = s.face_rings(f).into_iter().flatten().collect();
            if vertices.iter().all(|v| moving_set.contains(v)) {
                continue;
            }
            let normal: DVec3 = <[f64; 3]>::from(s.face_normal(f, Hypot::LibStdCxx)).into();
            for v in vertices {
                if moving_set.contains(&v) {
                    walls.entry(v).or_default().push(normal);
                } else {
                    levels.push(height(pos(v)));
                }
            }
        }
        let mut result = Self {
            movers: vec![],
            interval: (distance, distance),
            faces: vec![],
            edges: vec![],
        };
        if !levels.iter().any(|&d| (d - distance).abs() <= tolerance) {
            if moving.is_empty() {
                let mut spread = 0_f64;
                for p in cap.try_iter()? {
                    spread = spread.max(height(xyz(&p?)?).abs());
                }
                if created
                    .iter()
                    .any(|&v| (height(pos(v)) - distance).abs() <= spread + tolerance)
                {
                    return Ok(None);
                }
            }
            for v in moving {
                let Some(rate) =
                    slide_rate(walls.get(&v).map(Vec::as_slice).unwrap_or_default(), n)
                else {
                    return Ok(None);
                };
                result.movers.push(Mover {
                    key: s.key(Kind::Vertex, v),
                    point: pos(v),
                    rate,
                });
            }
            result.interval = (
                levels
                    .iter()
                    .copied()
                    .filter(|&d| d < distance)
                    .fold(f64::NEG_INFINITY, f64::max),
                levels
                    .iter()
                    .copied()
                    .filter(|&d| d > distance)
                    .fold(f64::INFINITY, f64::min),
            );
        }
        let keys = |k| made(k).into_iter().map(|i| s.key(k, i)).collect();
        result.faces = keys(Kind::Face);
        result.edges = keys(Kind::Edge);
        Ok(Some(result))
    }
    #[getter]
    fn vertices(&self) -> Vec<u128> {
        self.movers.iter().map(|m| m.key).collect()
    }

    fn apply(&self, mesh: &Bound<'_, PyAny>, offset: f64) -> PyResult<()> {
        let store = mesh.getattr("_store")?;
        let positions: Vec<_> = self
            .movers
            .iter()
            .map(|m| (m.key, (m.point + m.rate * offset).to_array()))
            .collect();
        PyMeshStore::place_tool_batch(&store, positions)?;
        mesh.call_method0("_bump")?;
        Ok(())
    }
}

impl Store {
    pub(crate) fn new_slots_since(&self, before: &crate::store::Snapshot, kind: Kind) -> Vec<u32> {
        let old = &before.state.slots[kind as usize];
        let current = &self.state.slots[kind as usize];
        self.live_slots(kind)
            .into_iter()
            .filter(|&v| {
                before.state.ns != self.state.ns || !old.alive(v) || old.gen(v) != current.gen(v)
            })
            .collect()
    }
}

/// A stage and its parked copy share the immutable geometry. Only the live
/// distance changes while dragging inside the open interval.
struct Stage {
    mesh: Py<PyAny>,
    d: f64,
    slide: PushSlide,
    made: Py<PyAny>,
}
impl Stage {
    fn holds(&self, d: f64, tol: f64) -> bool {
        let (lo, hi) = self.slide.interval;
        (self.d - d).abs() <= 1e-9 || (lo + tol < d && d < hi - tol)
    }
}

struct Capture {
    mesh: Py<PyAny>,
    selection: Py<PyAny>,
}
impl Capture {
    fn take(mesh: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>) -> PyResult<Self> {
        let selection = mesh
            .py()
            .import("built_core")?
            .call_method1("ElementSet", (scene.getattr("selection")?,))?;
        Ok(Self {
            mesh: mesh.call_method0("capture_state")?.unbind(),
            selection: selection.unbind(),
        })
    }

    fn restore(&self, mesh: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        mesh.call_method1("restore_state", (&self.mesh,))?;
        let selection = scene.getattr("selection")?;
        selection.call_method0("clear")?;
        selection.call_method1("update", (&self.selection,))?;
        Ok(())
    }
}

struct Parked {
    geometry: std::sync::Arc<Stage>,
    applied: f64,
    model: Capture,
}

#[pyclass(module = "built_core")]
#[derive(Default)]
pub struct PushStages {
    clean: Option<Capture>,
    active: Option<std::sync::Arc<Stage>>,
    applied: f64,
    parked: [Option<Parked>; 2],
    #[pyo3(get)]
    refusal: Option<(f64, f64)>,
    #[pyo3(get)]
    slow: bool,
    serial: Option<(usize, u64)>,
}
fn serial(mesh: &Bound<'_, PyAny>) -> PyResult<(usize, u64)> {
    Ok((
        mesh.as_ptr() as usize,
        mesh.getattr("_mut_serial")?.extract()?,
    ))
}
fn bump_scene(scene: &Bound<'_, PyAny>) -> PyResult<()> {
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)
}
impl PushStages {
    fn resolve(
        &self,
        tool: &Bound<'_, PyAny>,
        mesh: &Bound<'_, PyAny>,
        d: f64,
        tol: f64,
    ) -> PyResult<Option<Stage>> {
        let py = mesh.py();
        let mut levels: Vec<f64> = tool.getattr("_plane_levels")?.extract()?;
        levels.push(0.);
        if let Some(limit) = tool.getattr("_limit_in")?.extract::<Option<f64>>()? {
            levels.push(-limit);
        }
        let before = self
            .clean
            .as_ref()
            .expect("captured before mutation")
            .mesh
            .bind(py)
            .getattr("snap")?;
        let Some(slide) = PushSlide::resolve(
            &mesh.getattr("_store")?,
            before.extract()?,
            &tool.getattr("_anchor")?,
            &tool.getattr("_normal")?,
            d,
            levels,
            &tool.getattr("_cap_positions")?,
            tol,
        )?
        else {
            return Ok(None);
        };
        let views = py.import("core.push_scans")?.getattr("views")?;
        let faces = views.call1((mesh, "face", &slide.faces))?;
        let made = py
            .import("built_core")?
            .call_method1("ElementSet", (faces,))?;
        made.call_method1("update", (views.call1((mesh, "edge", &slide.edges))?,))?;
        Ok(Some(Stage {
            mesh: mesh.clone().unbind(),
            d,
            slide,
            made: made.unbind(),
        }))
    }
}
#[pymethods]
impl PushStages {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    #[getter]
    fn snapshot(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        self.clean.as_ref().map(|c| c.mesh.clone_ref(py))
    }
    #[getter]
    fn distance(&self) -> Option<f64> {
        self.active.as_ref().map(|s| s.d)
    }
    #[getter]
    fn vertices(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(stage) = &self.active {
            return Ok(py
                .import("core.push_scans")?
                .call_method1("views", (&stage.mesh, "vertex", stage.slide.vertices()))?
                .unbind());
        }
        Ok(pyo3::types::PyList::empty(py).into_any().unbind())
    }
    #[getter]
    fn made(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if let Some(s) = &self.active {
            return Ok(s.made.clone_ref(py));
        }
        Ok(pyo3::types::PySet::empty(py)?.into_any().unbind())
    }
    #[getter]
    fn cached_sides(&self) -> Vec<f64> {
        [-1., 1.]
            .into_iter()
            .zip(&self.parked)
            .filter_map(|(side, p)| p.as_ref().map(|_| side))
            .collect()
    }
    fn stale(&self, mesh: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.clean.is_some() && self.serial != Some(serial(mesh)?))
    }
    fn holds(&self, d: f64, tol: f64) -> bool {
        self.active.as_ref().is_some_and(|s| s.holds(d, tol))
    }
    fn restore(&mut self, mesh: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some(capture) = &self.clean {
            capture.restore(mesh, scene)?;
            self.active = None;
            self.serial = Some(serial(mesh)?);
            bump_scene(scene)?;
        }
        Ok(())
    }
    fn park(&mut self, mesh: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let Some(stage) = &self.active else {
            return Ok(());
        };
        let slot = &mut self.parked[usize::from(stage.d > 0.)];
        if slot
            .as_ref()
            .is_none_or(|p| !std::sync::Arc::ptr_eq(&p.geometry, stage))
        {
            *slot = Some(Parked {
                geometry: stage.clone(),
                applied: self.applied,
                model: Capture::take(mesh, scene)?,
            });
        }
        Ok(())
    }
    fn slide(&mut self, mesh: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>, d: f64) -> PyResult<()> {
        if let Some(stage) = &self.active {
            if !stage.slide.movers.is_empty() && (d - self.applied).abs() > 1e-12 {
                stage.slide.apply(mesh, d - stage.d)?;
                self.applied = d;
                self.serial = Some(serial(mesh)?);
                bump_scene(scene)?;
            }
        }
        Ok(())
    }
    /// Run the guarded document operation only on entry into a new interval.
    /// Release the borrow across the operation so observers can inspect the tool.
    fn frame(
        slf: &Bound<'_, Self>,
        tool: &Bound<'_, PyAny>,
        mesh: &Bound<'_, PyAny>,
        scene: &Bound<'_, PyAny>,
        d: f64,
        min: f64,
        tol: f64,
    ) -> PyResult<bool> {
        let sign = d.signum();
        {
            let mut state = slf.borrow_mut();
            if d.abs() < min {
                state.park(mesh, scene)?;
                if state.active.is_some() {
                    state.restore(mesh, scene)?;
                }
                return Ok(true);
            }
            if state.refusal.is_some_and(|(s, _)| s != sign) {
                state.refusal = None;
            }
            if state.holds(d, tol) {
                state.slide(mesh, scene, d)?;
                return Ok(true);
            }
            state.park(mesh, scene)?;
            if let Some(p) = state.parked[usize::from(d > 0.)]
                .as_ref()
                .filter(|p| p.geometry.holds(d, tol))
            {
                p.model.restore(mesh, scene)?;
                let (active, applied) = (p.geometry.clone(), p.applied);
                state.active = Some(active);
                state.applied = applied;
                state.serial = Some(serial(mesh)?);
                bump_scene(scene)?;
                state.slide(mesh, scene, d)?;
                return Ok(true);
            }
            if state.slow || state.refusal.is_some_and(|(_, last)| d.abs() < 2. * last) {
                return Ok(false);
            }
        }
        let start = std::time::Instant::now();
        let staged = (|| -> PyResult<Option<Stage>> {
            {
                let mut state = slf.borrow_mut();
                if state.clean.is_none() {
                    state.clean = Some(Capture::take(mesh, scene)?);
                } else if state.active.is_some() {
                    state.restore(mesh, scene)?;
                }
            }
            tool.call_method1("_mutate", (scene,))?;
            if tool.getattr("_refused")?.is_truthy()? {
                return Ok(None);
            }
            slf.borrow().resolve(tool, mesh, d, tol)
        })();
        let mut state = slf.borrow_mut();
        state.slow |= start.elapsed().as_secs_f64() > tool.getattr("_SLOW_STAGE_S")?.extract()?;
        tool.setattr("_refused", false)?;
        let stage = match staged {
            Ok(s) => s,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(mesh.py()) => None,
            Err(e) => return Err(e),
        };
        if let Some(stage) = stage {
            state.active = Some(std::sync::Arc::new(stage));
            state.applied = d;
            state.serial = Some(serial(mesh)?);
            bump_scene(scene)?;
            return Ok(true);
        }
        state.restore(mesh, scene)?;
        state.refusal = Some((sign, d.abs()));
        Ok(false)
    }
}

/// Choose the preview the host can display. The resident Metal host requests
/// transient surfaces; snapshot-capable editing hosts can display real stages.
#[pyfunction]
pub fn present_push_preview(
    tool: &Bound<'_, PyAny>,
    view: &Bound<'_, PyAny>,
    requested: &str,
    minimum: f64,
    tolerance: f64,
) -> PyResult<()> {
    use pyo3::types::{PyList, PySet};
    let py = tool.py();
    let scene = view.getattr("scene")?;
    let mesh = tool.call_method1("_target_mesh", (&scene,))?;
    let stages = tool.getattr("_stages")?.cast_into::<PushStages>()?;
    let d = tool.getattr("extrusion")?.extract::<f64>()?;
    let mode = if requested == "auto" {
        let transient = view
            .getattr_opt("mesh_outline_preview")?
            .map(|f| f.is_truthy())
            .transpose()?
            .unwrap_or(false);
        if transient
            || tool.getattr("_extra_faces")?.is_truthy()?
            || tool.getattr("_keep_base")?.is_truthy()?
        {
            "overlay"
        } else if tool.getattr("_prism_cap")?.is_truthy()? {
            "prism"
        } else if tool.getattr("_attached")?.is_truthy()? {
            "stage"
        } else {
            "overlay"
        }
    } else {
        requested
    };
    let suppressed = PySet::empty(py)?;
    tool.setattr("_light_faces", PyList::empty(py))?;
    tool.setattr("_light_wires", PyList::empty(py))?;
    match mode {
        "prism" => {
            let delta = d - tool.getattr("_prism_applied")?.extract::<f64>()?;
            if delta.abs() > 1e-12 {
                tool.call_method1("_move_cap", (view, delta, d))?;
            }
        }
        "stage" if PushStages::frame(&stages, tool, &mesh, &scene, d, minimum, tolerance)? => {}
        "stage" | "overlay" => {
            stages.borrow_mut().park(&mesh, &scene)?;
            tool.call_method1("_revert_preview", (view,))?;
            let faces = super::push_preview::push_light_faces(tool, minimum)?;
            if !faces.is_empty()
                && tool.getattr("_attached")?.is_truthy()?
                && !tool.getattr("_keep_base")?.is_truthy()?
                && super::push_preview::recessed(tool, view, d)?
            {
                suppressed.add(tool.getattr("base_face")?)?;
            }
            tool.setattr("_light_faces", faces)?;
        }
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "unknown extrusion preview",
            ))
        }
    }
    view.call_method1("set_suppressed_faces", (suppressed,))?;
    view.call_method0("update")?;
    Ok(())
}
