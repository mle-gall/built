// SPDX-License-Identifier: GPL-3.0-or-later
//! Prepare drawing steps in the active frame, retaining exact supplied coordinates.
use super::*;
use crate::math::Vec3;
use glam::DVec3;
use pyo3::types::PyFloat;
type P = [f64; 3];

fn point(value: &Obj<'_>) -> PyResult<P> {
    let float = value.py().get_type::<PyFloat>();
    let mut coords = value.try_iter()?;
    let mut p = [0.; 3];
    for v in &mut p {
        let c = coords
            .next()
            .transpose()?
            .ok_or_else(|| PyValueError::new_err("expected three coordinates"))?;
        *v = float.call1((c,))?.extract()?;
    }
    Ok(p)
}
fn ring(value: &Obj<'_>) -> PyResult<Vec<P>> {
    value.try_iter()?.map(|p| point(&p?)).collect()
}
fn vector<'py>(py: Python<'py>, p: P) -> PyResult<Obj<'py>> {
    Ok(Bound::new(py, Vec3::from(DVec3::from_array(p)))?.into_any())
}
fn triple(p: P) -> (f64, f64, f64) {
    (p[0], p[1], p[2])
}
fn plus(a: P, b: P) -> P {
    std::array::from_fn(|i| a[i] + b[i])
}
fn minus(a: P, b: P) -> P {
    std::array::from_fn(|i| a[i] - b[i])
}

struct Drawing<'py> {
    scene: Obj<'py>,
    mesh: Obj<'py>,
    frames: Bound<'py, PyModule>,
    origin: P,
    stored: bool,
}
impl<'py> Drawing<'py> {
    fn new(h: &Obj<'py>, anchor: Option<P>) -> PyResult<Self> {
        let py = h.py();
        let scene = h.getattr("scene")?;
        let mesh = scene.getattr("mesh")?;
        let frames = py.import("ingetrazo_core.coordinate_frame")?;
        let stored = mesh
            .getattr("__dict__")?
            .cast::<PyDict>()?
            .get_item("_store")?
            .is_some_and(|v| !v.is_none());
        let origin = match anchor {
            Some(p) if stored => frames
                .call_method1("choose_origin", (triple(p),))?
                .extract()?,
            _ => [0.; 3],
        };
        Ok(Self {
            scene,
            mesh,
            frames,
            origin,
            stored,
        })
    }
    fn local(&self, p: P) -> PyResult<Obj<'py>> {
        vector(self.mesh.py(), minus(p, self.origin))
    }
    fn loop_points(&self, ring: &[P]) -> PyResult<Bound<'py, PyList>> {
        PyList::new(
            self.mesh.py(),
            ring.iter()
                .map(|p| self.local(*p))
                .collect::<PyResult<Vec<_>>>()?,
        )
    }
    fn in_frame<T>(&self, op: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
        crate::doc::history::in_frame(
            &self
                .frames
                .call_method1("mesh_frame", (&self.mesh, triple(self.origin)))?,
            op,
        )
    }
    fn execute(&self, h: &Obj<'py>, cmd: &Obj<'py>, exact: &Bound<'py, PyDict>) -> PyResult<()> {
        crate::doc::history::in_frame(
            &self
                .frames
                .call_method1("planned_points", (&self.mesh, exact))?,
            || {
                h.call_method1("execute", (cmd,))?;
                Ok(())
            },
        )
    }
    fn snap(&self, p: &Obj<'py>) -> PyResult<P> {
        self.frames
            .call_method1("drawing_point", (&self.mesh, p))?
            .extract()
    }
}
#[pyfunction]
#[pyo3(signature=(history,a,b=None,soft=None,tag=None,eye=None,offset=None))]
fn add_edge<'py>(
    history: &Obj<'py>,
    a: &Obj<'py>,
    b: Option<&Obj<'py>>,
    soft: Option<&Obj<'py>>,
    tag: Option<&Obj<'py>>,
    eye: Option<&Obj<'py>>,
    offset: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    if let Some(t) = tag {
        if history
            .getattr("scene")?
            .call_method1("layer_by_key", (t,))?
            .is_none()
        {
            return Err(PyKeyError::new_err(format!(
                "no tag with key {}",
                t.repr()?
            )));
        }
    }
    let raw = point(a)?;
    let displacement = offset.map(point).transpose()?;
    let end = match (b, displacement) {
        (Some(p), _) => point(p)?,
        (None, Some(d)) => plus(raw, d),
        _ => return Err(PyValueError::new_err("add_edge needs b or offset")),
    };
    let draw = Drawing::new(history, Some(raw))?;
    let a = draw.local(raw)?;
    let b = draw.local(end)?;
    let kw = PyDict::new(py);
    kw.set_item("soft", soft)?;
    kw.set_item("tag", tag)?;
    kw.set_item("eye", eye.map(|v| draw.local(point(v)?)).transpose()?)?;
    let before = history.getattr("revision")?;
    draw.in_frame(|| {
        let cmd = py.import("core.edits")?.call_method(
            "build_add_edge",
            (&draw.scene, &a, &b),
            Some(&kw),
        )?;
        let exact = PyDict::new(py);
        if draw.stored {
            let (start, end) = match displacement {
                Some(d) => {
                    let start = draw.snap(&a)?;
                    (start, plus(start, d))
                }
                None => (minus(raw, draw.origin), minus(end, draw.origin)),
            };
            exact.set_item(a.call_method0("toTuple")?, triple(start))?;
            exact.set_item(b.call_method0("toTuple")?, triple(end))?;
        }
        draw.execute(history, &cmd, &exact)
    })?;
    answer(history, &before, true)
}
#[pyfunction]
fn add_edges<'py>(history: &Obj<'py>, segments: &Obj<'py>) -> Reply<'py> {
    let py = history.py();
    let mut lines = Vec::new();
    for item in segments.try_iter()? {
        let pair = item?;
        lines.push((point(&pair.get_item(0)?)?, point(&pair.get_item(1)?)?));
    }
    let draw = Drawing::new(history, lines.first().map(|l| l.0))?;
    let local = PyList::empty(py);
    let exact = PyDict::new(py);
    for (a, b) in lines {
        let (a, b) = (draw.local(a)?, draw.local(b)?);
        if draw.stored {
            for p in [&a, &b] {
                let xyz = p.call_method0("toTuple")?;
                exact.set_item(&xyz, &xyz)?;
            }
        }
        local.append((a, b))?;
    }
    let before = history.getattr("revision")?;
    draw.in_frame(|| {
        let cmd = py
            .import("core.edits")?
            .call_method1("build_add_edges", (&draw.scene, local))?;
        draw.execute(history, &cmd, &exact)
    })?;
    answer(history, &before, true)
}
#[pyfunction]
#[pyo3(signature=(history,outer=None,holes=None,anchor=None,offsets=None,curve=false))]
fn add_face<'py>(
    history: &Obj<'py>,
    outer: Option<&Obj<'py>>,
    holes: Option<&Obj<'py>>,
    anchor: Option<&Obj<'py>>,
    offsets: Option<&Obj<'py>>,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] curve: bool,
) -> Reply<'py> {
    let py = history.py();
    let offsets = offsets.map(ring).transpose()?;
    let outer = if let Some(p) = outer {
        ring(p)?
    } else if let (Some(a), Some(offsets)) = (anchor, &offsets) {
        let a = point(a)?;
        offsets.iter().map(|d| plus(a, *d)).collect()
    } else {
        return Err(PyValueError::new_err(
            "add_face needs outer, or anchor and offsets",
        ));
    };
    if outer.len() < 3 {
        return Err(PyValueError::new_err("a face needs three points"));
    }
    let draw = Drawing::new(history, Some(outer[0]))?;
    let points = draw.loop_points(&outer)?;
    let inner = PyList::empty(py);
    if let Some(holes) = holes {
        for h in holes.try_iter()? {
            inner.append(draw.loop_points(&ring(&h?)?)?)?;
        }
    }
    let segments = PyList::empty(py);
    for r in std::iter::once(points.clone().into_any()).chain(inner.iter()) {
        let n = r.len()?;
        for i in 0..n {
            segments.append((r.get_item(i)?, r.get_item((i + 1) % n)?))?;
        }
    }
    let before = history.getattr("revision")?;
    draw.in_frame(|| {
        let exact = PyDict::new(py);
        if draw.stored {
            if let Some(offsets) = offsets {
                let a = anchor.ok_or_else(|| {
                    pyo3::exceptions::PyTypeError::new_err("expected anchor with offsets")
                })?;
                let start = draw.snap(&draw.local(point(a)?)?)?;
                for (p, d) in points.iter().zip(offsets) {
                    exact.set_item(p.call_method0("toTuple")?, triple(plus(start, d)))?;
                }
            } else {
                for p in &points {
                    let xyz = p.call_method0("toTuple")?;
                    exact.set_item(&xyz, &xyz)?;
                }
            }
        }
        let kw = PyDict::new(py);
        kw.set_item("detect_faces", false)?;
        if curve {
            kw.set_item(
                "extra",
                py.import("core.arrangement")?
                    .call_method1("curve_extras", (&draw.scene, &points))?,
            )?;
        } else {
            let opts = PyDict::new(py);
            opts.set_item("holes", if inner.is_empty() { None } else { Some(&inner) })?;
            let face = py.import("ingetrazo_core.history")?.call_method(
                "AddFaceCommand",
                (&points,),
                Some(&opts),
            )?;
            kw.set_item("extra", PyList::new(py, [face])?)?;
            kw.set_item("plane_rebuild", true)?;
        }
        let cmd = py.import("core.edits")?.call_method(
            "build_add_edges",
            (&draw.scene, segments),
            Some(&kw),
        )?;
        draw.execute(history, &cmd, &exact)
    })?;
    answer(history, &before, true)
}
#[pyfunction]
#[pyo3(signature=(history,points,center=None,trim=None))]
fn add_arc<'py>(
    history: &Obj<'py>,
    points: &Obj<'py>,
    center: Option<&Obj<'py>>,
    trim: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let raw = ring(points)?;
    if raw.len() < 2 {
        return Err(PyValueError::new_err("an arc needs two points"));
    }
    let draw = Drawing::new(history, Some(raw[0]))?;
    let pts = draw.loop_points(&raw)?;
    let close = center.map(|p| draw.local(point(p)?)).transpose()?;
    let cuts = PyList::empty(py);
    if let Some(trim) = trim {
        for row in trim.try_iter()? {
            let r = row?;
            cuts.append((
                draw.local(point(&r.get_item(0)?)?)?,
                draw.local(point(&r.get_item(1)?)?)?,
            ))?;
        }
    }
    let before = history.getattr("revision")?;
    draw.in_frame(|| {
        let cmd = py.import("core.arrangement")?.call_method1(
            "arc_command",
            (
                &draw.scene,
                &pts,
                close,
                if cuts.is_empty() { None } else { Some(cuts) },
            ),
        )?;
        let exact = PyDict::new(py);
        if draw.stored {
            for p in &pts {
                let xyz = p.call_method0("toTuple")?;
                exact.set_item(&xyz, &xyz)?;
            }
        }
        draw.execute(history, &cmd, &exact)
    })?;
    answer(history, &before, true)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(add_edge, m)?)?;
    m.add_function(wrap_pyfunction!(add_edges, m)?)?;
    m.add_function(wrap_pyfunction!(add_face, m)?)?;
    m.add_function(wrap_pyfunction!(add_arc, m)?)?;
    Ok(())
}
