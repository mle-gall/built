// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixed-pin texture placement. The renderer and exporters consume the same world-to-UV map.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::prelude::*;
use pyo3::types::PyDict;

type Uv = (f64, f64);
type Pins = [Uv; 4];

fn dual(u: DVec3, v: DVec3) -> Option<(DVec3, DVec3)> {
    let (uu, uv, vv) = (u.dot(u), u.dot(v), v.dot(v));
    let det = uu * vv - uv * uv;
    (det.abs() >= 1e-24).then(|| ((u * vv - v * uv) / det, (v * uu - u * uv) / det))
}

fn rotated(v: DVec3, n: DVec3, angle: f64) -> DVec3 {
    let (s, c) = angle.sin_cos();
    v * c + n.cross(v) * s + n * (n.dot(v) * (1.0 - c))
}

#[pyclass(module = "ingetrazo_core", subclass, skip_from_py_object)]
#[derive(Clone)]
pub struct TextureMap {
    anchor: DVec3,
    #[pyo3(get, set)]
    uv: Uv,
    u: DVec3,
    v: DVec3,
}

impl TextureMap {
    fn point(&self, uv: Uv) -> DVec3 {
        self.anchor + self.u * (uv.0 - self.uv.0) + self.v * (uv.1 - self.uv.1)
    }

    fn at(&self, p: DVec3) -> Uv {
        let d = p - self.anchor;
        dual(self.u, self.v).map_or(self.uv, |(gu, gv)| {
            (self.uv.0 + gu.dot(d), self.uv.1 + gv.dot(d))
        })
    }

    fn corners(&self) -> Pins {
        let (u, v) = self.uv;
        [(u, v), (u + 1., v), (u, v + 1.), (u + 1., v + 1.)]
    }

    fn about(&self, uv: Uv) -> Self {
        Self {
            anchor: self.point(uv),
            uv,
            ..self.clone()
        }
    }
}

#[pymethods]
impl TextureMap {
    #[new]
    #[allow(non_snake_case)]
    fn new(
        R: &Bound<'_, PyAny>,
        uv: Uv,
        e_u: &Bound<'_, PyAny>,
        e_v: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        Ok(Self {
            anchor: xyz(R)?,
            uv,
            u: xyz(e_u)?,
            v: xyz(e_v)?,
        })
    }

    #[getter(R)]
    fn anchor(&self) -> Vec3 {
        self.anchor.into()
    }
    #[setter(R)]
    fn set_anchor(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.anchor = xyz(value)?;
        Ok(())
    }
    #[getter]
    fn e_u(&self) -> Vec3 {
        self.u.into()
    }
    #[setter]
    fn set_e_u(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.u = xyz(value)?;
        Ok(())
    }
    #[getter]
    fn e_v(&self) -> Vec3 {
        self.v.into()
    }
    #[setter]
    fn set_e_v(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.v = xyz(value)?;
        Ok(())
    }
    fn copy(&self) -> Self {
        self.clone()
    }
    fn world(&self, u: f64, v: f64) -> Vec3 {
        self.point((u, v)).into()
    }
    fn uv_of(&self, point: &Bound<'_, PyAny>) -> PyResult<Uv> {
        Ok(self.at(xyz(point)?))
    }
    fn uvw(&self) -> Option<Vec<f64>> {
        let (gu, gv) = dual(self.u, self.v)?;
        Some(vec![
            gu.x,
            gu.y,
            gu.z,
            self.uv.0 - gu.dot(self.anchor),
            gv.x,
            gv.y,
            gv.z,
            self.uv.1 - gv.dot(self.anchor),
        ])
    }

    #[staticmethod]
    #[pyo3(signature=(face, tex, at=None))]
    fn from_face(
        py: Python<'_>,
        face: &Bound<'_, PyAny>,
        tex: &Bound<'_, PyAny>,
        at: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<Self>> {
        let normal = face.call_method0("normal")?;
        let n = xyz(&normal)?;
        let axes = py
            .import("core.texture")?
            .call_method1("face_uv_axes", (tex, normal))?;
        let (gu, cu, gv, cv): (PyRef<'_, Vec3>, f64, PyRef<'_, Vec3>, f64) = axes.extract()?;
        if n.length_squared() < 1e-18 {
            return Ok(None);
        }
        let n = n.normalize();
        let (gu, gv) = (DVec3::from(gu.coords()), DVec3::from(gv.coords()));
        let Some((u, v)) = dual(gu - n * gu.dot(n), gv - n * gv.dot(n)) else {
            return Ok(None);
        };
        let at = if let Some(p) = at {
            xyz(p)?
        } else {
            let mut sum = DVec3::ZERO;
            let mut count = 0;
            for p in face.getattr("vertices")?.try_iter()? {
                sum += xyz(&p?)?;
                count += 1;
            }
            sum / count.max(1) as f64
        };
        let (a, b) = (gu.dot(at) + cu, gv.dot(at) + cv);
        let uv = (a.floor(), b.floor());
        Ok(Some(Self {
            anchor: at + u * (uv.0 - a) + v * (uv.1 - b),
            uv,
            u,
            v,
        }))
    }
}

#[derive(Clone)]
struct Saved {
    map: TextureMap,
    pins: Pins,
}

struct Drag {
    pin: Option<usize>,
    start: Option<DVec3>,
    before: Saved,
}

#[pyclass(module = "ingetrazo_core")]
pub struct TexturePlacement {
    #[pyo3(get)]
    map: Py<TextureMap>,
    entry: TextureMap,
    pins: Pins,
    normal: DVec3,
    history: Vec<Saved>,
    drag: Option<Drag>,
    press: Option<Uv>,
    moved: bool,
    #[pyo3(get)]
    lifted: Option<usize>,
    floating: Option<DVec3>,
    #[pyo3(get)]
    hover_pin: Option<usize>,
    held: Option<(DVec3, &'static str)>,
    #[pyo3(get)]
    sweep_deg: Option<f64>,
    #[pyo3(get)]
    on_tick: bool,
    esc_armed: bool,
}

fn pixel(project: &Bound<'_, PyAny>, p: DVec3) -> PyResult<Option<Uv>> {
    if project.is_none() {
        return Ok(None);
    }
    project.call1((Vec3::from(p),))?.extract()
}

fn nearest(
    project: &Bound<'_, PyAny>,
    points: &[DVec3],
    screen: Uv,
    radius: f64,
) -> PyResult<Option<usize>> {
    let mut best = None;
    let mut limit = radius;
    for (i, p) in points.iter().enumerate() {
        if let Some((x, y)) = pixel(project, *p)? {
            let d = (x - screen.0).hypot(y - screen.1);
            if d < limit || (best.is_none() && d == limit) {
                limit = d;
                best = Some(i);
            }
        }
    }
    Ok(best)
}

impl TexturePlacement {
    fn save(&self, py: Python<'_>) -> Saved {
        Saved {
            map: self.map.borrow(py).clone(),
            pins: self.pins,
        }
    }
    fn points(&self, py: Python<'_>) -> [DVec3; 4] {
        let m = self.map.borrow(py);
        let mut points = self.pins.map(|uv| m.point(uv));
        if let (Some(i), Some(p)) = (self.lifted, self.floating) {
            points[i] = p;
        }
        points
    }
    fn hold(
        &mut self,
        want: DVec3,
        free: bool,
        project: &Bound<'_, PyAny>,
        face: &Bound<'_, PyAny>,
        radius: f64,
    ) -> PyResult<DVec3> {
        self.held = None;
        if free {
            return Ok(want);
        }
        let Some(screen) = pixel(project, want)? else {
            return Ok(want);
        };
        let mut corners = Vec::new();
        let mut middles = Vec::new();
        let mut rings = vec![face.getattr("vertices")?];
        if let Some(holes) = face.getattr_opt("holes")? {
            for ring in holes.try_iter()? {
                rings.push(ring?);
            }
        }
        for ring in rings {
            let pts: Vec<_> = ring
                .try_iter()?
                .map(|p| xyz(&p?))
                .collect::<PyResult<_>>()?;
            for (a, b) in pts.iter().zip(pts.iter().cycle().skip(1)) {
                corners.push(*a);
                middles.push((*a + *b) * 0.5);
            }
        }
        for (kind, points) in [("corner", corners), ("middle", middles)] {
            if let Some(i) = nearest(project, &points, screen, radius)? {
                self.held = Some((points[i], kind));
                return Ok(points[i]);
            }
        }
        Ok(want)
    }

    fn apply(&mut self, py: Python<'_>, point: DVec3, free: bool, tick_deg: f64) -> PyResult<()> {
        let Some(drag) = &self.drag else {
            return Ok(());
        };
        let Some(start) = drag.start else {
            return Ok(());
        };
        let m = &drag.before.map;
        let [red, green, blue, _] = drag.before.pins;
        let mut next = m.about(red);
        let g = m.point(green) - next.anchor;
        let b = point - next.anchor;
        match drag.pin {
            Some(1) => {
                let lengths = (g.length(), b.length());
                if lengths.0 < 1e-9 || lengths.1 < 1e-9 {
                    next = m.clone();
                } else {
                    let product = lengths.0 * lengths.1;
                    let angle = (self.normal.dot(g.cross(b)) / product)
                        .atan2((g.dot(b) / product).clamp(-1., 1.));
                    let deg = angle.to_degrees();
                    let snapped = !free && self.held.is_none();
                    let mut measured = if snapped {
                        (deg / tick_deg).round_ties_even() * tick_deg
                    } else {
                        (deg * 10.).round_ties_even() / 10.
                    };
                    if self.held.is_none() && measured <= -180. {
                        measured += 360.;
                    }
                    self.sweep_deg = Some(measured);
                    self.on_tick = snapped;
                    let turn = if self.held.is_some() {
                        angle
                    } else {
                        measured.to_radians()
                    };
                    let scale = lengths.1 / lengths.0;
                    next.u = rotated(m.u, self.normal, turn) * scale;
                    next.v = rotated(m.v, self.normal, turn) * scale;
                }
            }
            Some(2) => {
                let (ug, vg, ub, vb) = (
                    green.0 - red.0,
                    green.1 - red.1,
                    blue.0 - red.0,
                    blue.1 - red.1,
                );
                let det = ug * vb - vg * ub;
                if det.abs() >= 1e-12 {
                    next.u = (g * vb - b * vg) / det;
                    next.v = (b * ug - g * ub) / det;
                }
                if det.abs() < 1e-12 || dual(next.u, next.v).is_none() {
                    next = m.clone();
                }
            }
            _ => {
                next = m.clone();
                next.anchor += point - start;
            }
        }
        self.map = Py::new(py, next)?;
        Ok(())
    }
}

#[pymethods]
impl TexturePlacement {
    #[new]
    fn new(py: Python<'_>, map: Py<TextureMap>, normal: &Vec3) -> Self {
        let entry = map.borrow(py).clone();
        let pins = entry.corners();
        Self {
            map,
            entry,
            pins,
            normal: normal.coords().into(),
            history: vec![],
            drag: None,
            press: None,
            moved: false,
            lifted: None,
            floating: None,
            hover_pin: None,
            held: None,
            sweep_deg: None,
            on_tick: false,
            esc_armed: false,
        }
    }
    #[getter]
    fn pin_uv(&self) -> Vec<Uv> {
        self.pins.to_vec()
    }
    fn pins(&self, py: Python<'_>) -> Vec<Vec3> {
        self.points(py).into_iter().map(Vec3::from).collect()
    }
    #[getter]
    fn lifted_pos(&self) -> Option<Vec3> {
        self.floating.map(Vec3::from)
    }
    #[getter]
    fn held(&self) -> Option<(Vec3, &'static str)> {
        self.held.map(|(p, s)| (p.into(), s))
    }
    #[getter]
    fn gesture_active(&self) -> bool {
        self.drag.is_some()
    }
    #[getter]
    fn draw_protractor(&self) -> bool {
        self.moved && self.drag.as_ref().is_some_and(|drag| drag.pin == Some(1))
    }
    #[getter]
    fn undo_depth(&self) -> usize {
        self.history.len()
    }
    /// The same operations used by the pin editor and its local undo stack.
    fn menu_rows(&self) -> Vec<(&'static str, Vec<(&'static str, &'static str, bool)>)> {
        vec![
            ("", vec![("done", "Done", true)]),
            ("", vec![("reset", "Reset", true)]),
            ("Flip", vec![("horizontal", "Left / Right", true), ("vertical", "Up / Down", true)]),
            ("Rotate", vec![("90", "90°", true), ("180", "180°", true), ("270", "270°", true)]),
            ("", vec![("undo", "Undo", !self.history.is_empty())]),
        ]
    }
    fn menu_action(&mut self, py: Python<'_>, action: &str) -> PyResult<bool> {
        match action {
            "reset" => self.reset(py)?,
            "horizontal" | "vertical" => self.flip(py, action == "horizontal")?,
            "90" | "180" | "270" => self.rotate(py, action.parse().unwrap())?,
            "undo" => return self.undo_step(py),
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn changed(&self, py: Python<'_>) -> bool {
        match (self.map.borrow(py).uvw(), self.entry.uvw()) {
            (Some(a), Some(b)) => a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-9),
            (None, None) => false,
            _ => true,
        }
    }

    #[pyo3(signature=(screen,point,free,project,face,grab_px,snap_px))]
    fn click(
        &mut self,
        py: Python<'_>,
        screen: Option<Uv>,
        point: Option<&Vec3>,
        free: bool,
        project: &Bound<'_, PyAny>,
        face: &Bound<'_, PyAny>,
        grab_px: f64,
        snap_px: f64,
    ) -> PyResult<&'static str> {
        self.press = screen;
        self.moved = false;
        self.esc_armed = false;
        let point = point.map(|p| DVec3::from(p.coords()));
        if let Some(i) = self.lifted {
            if let Some(p) = point {
                let p = self.hold(p, free, project, face, snap_px)?;
                self.pins[i] = self.map.borrow(py).at(p);
            }
            self.held = None;
            self.lifted = None;
            self.floating = None;
            self.drag = None;
            return Ok("update");
        }
        let pin = match screen {
            Some(s) => nearest(project, &self.points(py), s, grab_px)?,
            None => None,
        };
        if pin == Some(3) {
            self.drag = None;
            return Ok("distort");
        }
        self.drag = Some(Drag {
            pin,
            start: point,
            before: self.save(py),
        });
        Ok(if pin.is_none() { "pick_face" } else { "pin" })
    }
    #[pyo3(signature=(screen,point,free,project,face,grab_px,snap_px,click_px,tick_deg))]
    fn hover(
        &mut self,
        py: Python<'_>,
        screen: Option<Uv>,
        point: Option<&Vec3>,
        free: bool,
        project: &Bound<'_, PyAny>,
        face: &Bound<'_, PyAny>,
        grab_px: f64,
        snap_px: f64,
        click_px: f64,
        tick_deg: f64,
    ) -> PyResult<()> {
        let point = point.map(|p| DVec3::from(p.coords()));
        if let (Some((x, y)), Some((a, b))) = (screen, self.press) {
            self.moved |= (x - a).hypot(y - b) > click_px;
        }
        if self.lifted.is_some() {
            if let Some(p) = point {
                self.floating = Some(self.hold(p, free, project, face, snap_px)?);
            }
        } else if let (true, Some(drag), Some(p), Some(start)) = (
            self.moved,
            &self.drag,
            point,
            self.drag.as_ref().and_then(|d| d.start),
        ) {
            let p = match drag.pin {
                Some(0) => {
                    let spot = drag.before.map.point(drag.before.pins[0]) + (p - start);
                    p + (self.hold(spot, free, project, face, snap_px)? - spot)
                }
                Some(_) => self.hold(p, free, project, face, snap_px)?,
                None => p,
            };
            self.apply(py, p, free, tick_deg)?;
        } else {
            self.hover_pin = match screen {
                Some(s) => nearest(project, &self.points(py), s, grab_px)?,
                None => None,
            };
        }
        Ok(())
    }
    fn release(&mut self, py: Python<'_>) -> &'static str {
        self.press = None;
        let Some(drag) = self.drag.take() else {
            return "idle";
        };
        self.sweep_deg = None;
        self.held = None;
        if self.moved {
            self.history.push(drag.before);
            return "update";
        }
        if let Some(pin) = drag.pin {
            self.floating = Some(self.points(py)[pin]);
            self.lifted = Some(pin);
            return "lifted";
        }
        "idle"
    }
    fn rotate(&mut self, py: Python<'_>, degrees: f64) -> PyResult<()> {
        self.history.push(self.save(py));
        let mut m = self.map.borrow(py).about(self.pins[0]);
        let a = degrees.to_radians();
        m.u = rotated(m.u, self.normal, a);
        m.v = rotated(m.v, self.normal, a);
        self.map = Py::new(py, m)?;
        Ok(())
    }
    fn flip(&mut self, py: Python<'_>, horizontal: bool) -> PyResult<()> {
        self.history.push(self.save(py));
        let mut m = self.map.borrow(py).about(self.pins[0]);
        if horizontal {
            m.u = -m.u;
        } else {
            m.v = -m.v;
        }
        self.map = Py::new(py, m)?;
        Ok(())
    }
    fn undo_step(&mut self, py: Python<'_>) -> PyResult<bool> {
        let Some(state) = self.history.pop() else {
            return Ok(false);
        };
        self.map = Py::new(py, state.map)?;
        self.pins = state.pins;
        self.lifted = None;
        Ok(true)
    }
    fn reset(&mut self, py: Python<'_>) -> PyResult<()> {
        self.history.push(self.save(py));
        self.map = Py::new(py, self.entry.clone())?;
        self.pins = self.entry.corners();
        self.lifted = None;
        Ok(())
    }
    fn escape(&mut self, py: Python<'_>) -> PyResult<&'static str> {
        if self.lifted.take().is_some() {
            self.floating = None;
            return Ok("update");
        }
        if !self.esc_armed && self.changed(py) {
            self.reset(py)?;
            self.esc_armed = true;
            return Ok("restored");
        }
        Ok("exit")
    }
    fn grid_lines(
        &self,
        py: Python<'_>,
        vertices: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<(Vec3, Vec3)>> {
        let m = self.map.borrow(py);
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for p in vertices.try_iter()? {
            let (u, v) = m.at(xyz(&p?)?);
            for (i, k) in [u, v].into_iter().enumerate() {
                lo[i] = lo[i].min(k);
                hi[i] = hi[i].max(k);
            }
        }
        if !lo[0].is_finite() {
            return Ok(vec![]);
        }
        let [u0, v0] = lo.map(|v| (v + 1e-4).floor());
        let [u1, v1] = hi.map(|v| (v - 1e-4).ceil());
        let (w, h) = (u1 - u0, v1 - v0);
        if w * h > 2500. {
            return Ok(vec![]);
        }
        let mut lines = Vec::new();
        for k in 0..=w as usize {
            let u = u0 + k as f64;
            lines.push((m.world(u, v0), m.world(u, v1)));
        }
        for k in 0..=h as usize {
            let v = v0 + k as f64;
            lines.push((m.world(u0, v), m.world(u1, v)));
        }
        Ok(lines)
    }
    fn protractor<'py>(
        &self,
        py: Python<'py>,
        project: &Bound<'py, PyAny>,
        disc_px: f64,
    ) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(drag) = &self.drag else {
            return Ok(None);
        };
        if drag.pin != Some(1) {
            return Ok(None);
        }
        let base = &drag.before.map;
        let r0 = base.point(self.pins[0]);
        let axis = base.point(self.pins[1]) - r0;
        if axis.length_squared() < 1e-18 {
            return Ok(None);
        }
        let u = axis.normalize();
        let v = self.normal.cross(u).normalize_or_zero();
        let (Some((x, y)), Some((a, b))) = (pixel(project, r0)?, pixel(project, r0 + u)?) else {
            return Ok(None);
        };
        let scale = (x - a).hypot(y - b);
        if scale < 1e-6 {
            return Ok(None);
        }
        let radius = disc_px / scale;
        let direction = |t: f64| u * t.cos() + v * t.sin();
        let rim = |t: f64| Vec3::from(r0 + direction(t) * radius);
        let segment = |a: DVec3, b: DVec3| (Vec3::from(a), Vec3::from(b));
        let mut ring = Vec::new();
        let step = std::f64::consts::TAU / 48.;
        for i in 0..48 {
            ring.push((rim(step * i as f64), rim(step * ((i + 1) % 48) as f64)));
        }
        let degrees = self.sweep_deg.unwrap_or(0.);
        let sweep = degrees.to_radians();
        let steps = 2.max((degrees.abs() / 5.) as usize + 1);
        let mut wedge = vec![Vec3::from(r0)];
        wedge.extend((0..=steps).map(|k| rim(sweep * k as f64 / steps as f64)));
        let current = direction(sweep);
        let reach =
            ((self.map.borrow(py).point(self.pins[1]) - r0).length() * 1.3).max(radius * 1.7);
        let squares = segment(r0 + u * radius * 1.7, r0 + current * radius * 1.7);
        let disc = PyDict::new(py);
        disc.set_item("ring", ring)?;
        disc.set_item("diameter", segment(r0 - u * radius, r0 + u * radius))?;
        disc.set_item("wedge", wedge)?;
        disc.set_item("base_stub", segment(r0, r0 + u * radius * 1.7))?;
        disc.set_item("squares", squares)?;
        disc.set_item("current_arm", segment(r0, r0 + current * reach))?;
        disc.set_item(
            "frame",
            (Vec3::from(r0), Vec3::from(u), Vec3::from(v), radius),
        )?;
        Ok(Some(disc))
    }
}
