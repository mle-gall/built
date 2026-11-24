//! The PyO3 face of the tool machines: `ToolMachine(kind)`.
//!
//! `event(ev, host)` takes one event as a dict and a host object whose
//! methods answer [`ToolHost`]; it returns `{consumed, redraw, committed}`.
//! Commands reach the host as `run(name, json_args)`. `preview()`, `state()`
//! and `info` are plain dicts and tuples. Used by `tools/rust_tools.py`
//! (the transition adapter, `docs/rust-tools.md` §10).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use super::{new_tool, ViewReading, Answer, Command, Grow, Mods, Pick, PickKind, Pointer, Reply, SelectMode, Snap, Tool,
            ToolEvent, ToolHost, Value, P3};

pub fn json_to_py<'py>(py: Python<'py>, v: &serde_json::Value) -> PyResult<Bound<'py, PyAny>> {
    json_values(py,v,true)
}
pub(super) fn json_values<'py>(py: Python<'py>, v: &serde_json::Value, tuples: bool) -> PyResult<Bound<'py, PyAny>> {
    use serde_json::Value as J;
    Ok(match v {
        J::Null => py.None().into_bound(py),
        J::Bool(b) => b.into_pyobject(py)?.to_owned().into_any(),
        J::Number(n) => match n.as_i64() {
            Some(i) => i.into_pyobject(py)?.into_any(),
            None => n.as_f64().unwrap_or(f64::NAN).into_pyobject(py)?.into_any(),
        },
        J::String(s) => s.into_pyobject(py)?.into_any(),
        J::Array(a) => {
            let items = a.iter().map(|x| json_values(py, x, tuples)).collect::<PyResult<Vec<_>>>()?;
            if tuples { PyTuple::new(py, items)?.into_any() } else { PyList::new(py,items)?.into_any() }
        }
        J::Object(o) => {
            let d = PyDict::new(py);
            for (k, x) in o {
                d.set_item(k, json_values(py, x, tuples)?)?;
            }
            d.into_any()
        }
    })
}

/// Plain Python values (and anything with `toTuple`) as JSON.
pub fn py_to_json(v: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    use serde_json::Value as J;
    if v.is_none() {
        return Ok(J::Null);
    }
    if let Ok(b) = v.cast::<pyo3::types::PyBool>() {
        return Ok(J::Bool(b.is_true()));
    }
    if let Ok(i) = v.extract::<i64>() {
        return Ok(J::from(i));
    }
    if let Ok(x) = v.extract::<f64>() {
        return Ok(serde_json::json!(x));
    }
    if let Ok(s) = v.extract::<String>() {
        return Ok(J::String(s));
    }
    if v.hasattr("toTuple")? {
        return py_to_json(&v.call_method0("toTuple")?);
    }
    if let Ok(d) = v.cast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, item) in d.iter() {
            map.insert(k.str()?.to_string(), py_to_json(&item)?);
        }
        return Ok(J::Object(map));
    }
    let mut out = Vec::new();
    for item in v.try_iter()? {
        out.push(py_to_json(&item?)?);
    }
    Ok(J::Array(out))
}

fn p3(v: &Bound<'_, PyAny>) -> PyResult<P3> {
    crate::math::xyz(v).map(|point| point.to_array())
}

fn opt<'py>(d: &Bound<'py, PyDict>, key: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    Ok(d.get_item(key)?.filter(|v| !v.is_none()))
}

fn pointer(d: &Bound<'_, PyDict>) -> PyResult<Pointer> {
    let px: (f64, f64) = opt(d, "px")?.map(|v| v.extract()).transpose()?.unwrap_or((0.0, 0.0));
    let mods: u32 = opt(d, "mods")?.map(|v| v.extract()).transpose()?.unwrap_or(0);
    let world = opt(d, "world")?.map(|v| p3(&v)).transpose()?;
    let kind: Option<String> = opt(d, "snap_kind")?.map(|v| v.extract()).transpose()?;
    let t_ms: f64 = opt(d, "t")?.map(|v| v.extract()).transpose()?.unwrap_or(0.0);
    let snap = match (world, kind) {
        (Some(point), Some(kind)) => Some(Snap { point, kind }),
        _ => None,
    };
    Ok(Pointer { px: [px.0, px.1], mods: Mods::from_qt(mods), world, snap, t_ms })
}

fn value(v: &Bound<'_, PyAny>) -> PyResult<Value> {
    if let Ok(x) = v.extract::<f64>() {
        return Ok(Value::Length(x));
    }
    let xs: Vec<f64> = v.extract()?;
    Ok(match xs.as_slice() {
        [a, b] => Value::Pair(*a, *b),
        [a, b, c] => Value::Triple(*a, *b, *c),
        _ => Value::List(xs),
    })
}

fn event(d: &Bound<'_, PyDict>) -> PyResult<ToolEvent> {
    let kind: String = d.get_item("type")?.ok_or_else(|| PyValueError::new_err("event without a type"))?.extract()?;
    let mods = || -> PyResult<Mods> {
        Ok(Mods::from_qt(opt(d, "mods")?.map(|v| v.extract()).transpose()?.unwrap_or(0)))
    };
    use super::walkthrough::Navigation;
    let field = |key: &str| d.get_item(key)?.ok_or_else(|| PyValueError::new_err(format!("missing {key}")));
    Ok(match kind.as_str() {
        "paste_at" => ToolEvent::PasteAt(p3(&field("offset")?)?),
        "flip_along" => ToolEvent::FlipAlong { axis: field("axis")?.extract()?, copy: field("copy")?.extract()? },
        "profile" => ToolEvent::Profile(field("value")?.extract()?),
        "protractor_frame" => ToolEvent::ProtractorFrame { basis: field("basis")?.extract()?, near: field("near")?.extract()?, exact: field("exact")?.extract()? },
        "place_camera" => ToolEvent::Navigate(Navigation::Place { base: p3(&field("base")?)?, look: opt(d,"look")?.map(|v| p3(&v)).transpose()? }),
        "look_delta" => ToolEvent::Navigate(Navigation::Look(field("delta")?.extract()?)),
        "camera_tick" => ToolEvent::Navigate(Navigation::Tick { dt: field("dt")?.extract()?, mods: mods()? }),
        "walk_step" => ToolEvent::Navigate(Navigation::Walk { delta: field("delta")?.extract()?, dt: field("dt")?.extract()?, mods: mods()? }),
        "first_person_step" => ToolEvent::Navigate(Navigation::Keys { keys: field("keys")?.extract()?, dt: field("dt")?.extract()?, mods: mods()? }),
        "activate" => ToolEvent::Activate,
        "deactivate" => ToolEvent::Deactivate,
        "cancel" => ToolEvent::Cancel,
        "undo" => ToolEvent::Undo,
        "hover" => ToolEvent::Hover(pointer(d)?),
        "press" => ToolEvent::Press(pointer(d)?),
        "drag" => ToolEvent::Drag(pointer(d)?),
        "release" => ToolEvent::Release(pointer(d)?),
        "click" => ToolEvent::Click(pointer(d)?),
        "double_click" => ToolEvent::DoubleClick(pointer(d)?),
        "triple_click" => ToolEvent::TripleClick(pointer(d)?),
        "box" => {
            let r: (f64, f64, f64, f64) = d.get_item("rect")?.ok_or_else(|| PyValueError::new_err("box without rect"))?.extract()?;
            let crossing: bool = opt(d, "crossing")?.map(|v| v.extract()).transpose()?.unwrap_or(false);
            ToolEvent::Box { rect: [r.0, r.1, r.2, r.3], crossing, mods: mods()? }
        }
        "key" => ToolEvent::Key {
            key: d.get_item("key")?.ok_or_else(|| PyValueError::new_err("key without key"))?.extract()?,
            mods: mods()?,
        },
        "array" => ToolEvent::Array {
            count: d.get_item("count")?.ok_or_else(|| PyValueError::new_err("array without count"))?.extract()?,
            mode: opt(d, "mode")?.map(|v| v.extract()).transpose()?.unwrap_or_else(|| "x".to_string()),
            step: opt(d, "step")?.map(|v| v.extract()).transpose()?,
        },
        "key_release" => ToolEvent::KeyRelease {
            key: d.get_item("key")?.ok_or_else(|| PyValueError::new_err("key_release without key"))?.extract()?,
            tapped: opt(d, "tapped")?.map(|v| v.extract()).transpose()?.unwrap_or(true),
        },
        "frame" => {
            let axes: [(f64, f64, f64); 4] = d.get_item("axes")?.ok_or_else(|| PyValueError::new_err("frame without axes"))?
                .extract::<Vec<Bound<PyAny>>>()?
                .iter().map(|v| p3(v).map(|p| (p[0], p[1], p[2]))).collect::<PyResult<Vec<_>>>()?
                .try_into().map_err(|_| PyValueError::new_err("axes are origin, x, y, z"))?;
            ToolEvent::Frame {
                axes: axes.map(|(x, y, z)| [x, y, z]),
                names_point: opt(d, "names_point")?.map(|v| v.extract()).transpose()?,
            }
        }
        "view" => {
            let pair = |k: &str| -> PyResult<Option<(P3, P3)>> {
                match opt(d, k)? {
                    None => Ok(None),
                    Some(v) => {
                        let (a, b): (Bound<PyAny>, Bound<PyAny>) = v.extract()?;
                        Ok(Some((p3(&a)?, p3(&b)?)))
                    }
                }
            };
            ToolEvent::View(ViewReading {
                ray: pair("ray")?,
                axis_lock: opt(d, "axis_lock")?.map(|v| p3(&v)).transpose()?,
                protractor: opt(d, "protractor")?.map(|v| v.extract::<(Option<f64>, bool)>()).transpose()?,
                plane_normal: opt(d, "plane_normal")?.map(|v| p3(&v)).transpose()?,
            })
        }
        "radius" => ToolEvent::Radius(
            d.get_item("value")?.ok_or_else(|| PyValueError::new_err("radius without value"))?.extract()?),
        "segments" => ToolEvent::Segments(
            d.get_item("count")?.ok_or_else(|| PyValueError::new_err("segments without count"))?.extract()?),
        "scale_factors" => ToolEvent::ScaleFactors(d.get_item("value")?.ok_or_else(|| PyValueError::new_err("missing factors"))?.extract()?),
        "absolute_value" => ToolEvent::AbsoluteValue(value(&d.get_item("value")?.ok_or_else(|| PyValueError::new_err("missing size"))?)?),
        "value" => ToolEvent::Value(value(&d.get_item("value")?.ok_or_else(|| PyValueError::new_err("value without value"))?)?),
        "work_plane" => ToolEvent::WorkPlane(match opt(d, "plane")? {
            None => None,
            Some(v) => {
                let (p, n): (Bound<PyAny>, Bound<PyAny>) = v.extract()?;
                Some((p3(&p)?, p3(&n)?))
            }
        }),
        other => return Err(PyValueError::new_err(format!("unknown tool event {other:?}"))),
    })
}

/// [`ToolHost`] over a Python object. The first exception a callback
/// raises is kept and raised once the event returns.
struct PyHost<'a, 'py> {
    obj: &'a Bound<'py, PyAny>,
    err: Option<PyErr>,
}

impl<'py> PyHost<'_, 'py> {
    fn py(&self) -> Python<'py> { self.obj.py() }

    fn call(&mut self, name: &str, args: Vec<Bound<'py, PyAny>>) -> Option<Bound<'py, PyAny>> {
        if self.err.is_some() {
            return None;
        }
        let result = PyTuple::new(self.py(), args).and_then(|t| self.obj.call_method1(name, t));
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                self.err = Some(e);
                None
            }
        }
    }

    fn take<T: pyo3::conversion::FromPyObjectOwned<'py>>(&mut self, v: Option<Bound<'py, PyAny>>) -> Option<T> {
        let v = v?;
        let r: Result<T, PyErr> = v.extract::<T>().map_err(Into::into);
        match r {
            Ok(x) => Some(x),
            Err(e) => {
                self.err.get_or_insert(e);
                None
            }
        }
    }

    fn f(&self, x: f64) -> Bound<'py, PyAny> { x.into_pyobject(self.py()).unwrap().into_any() }
    fn u(&self, x: u64) -> Bound<'py, PyAny> { x.into_pyobject(self.py()).unwrap().into_any() }
    fn s(&self, x: &str) -> Bound<'py, PyAny> { x.into_pyobject(self.py()).unwrap().into_any() }
    fn px(&self, px: [f64; 2]) -> Vec<Bound<'py, PyAny>> { vec![self.f(px[0]), self.f(px[1])] }

    fn pick_of(&mut self, v: Option<Bound<'py, PyAny>>) -> Option<Pick> {
        let (id, kind): (u64, String) = self.take(v.filter(|v| !v.is_none()))?;
        Some(Pick { id, kind: PickKind::parse(&kind) })
    }
}

impl ToolHost for PyHost<'_, '_> {
    fn run(&mut self, cmd: &Command) -> Answer {
        if let Ok(host) = self.obj.cast::<super::view_queries::ViewportToolHost>() {
            if self.err.is_some() { return Answer::default(); }
            return match host.borrow().command(self.py(),cmd.name,&cmd.args) {
                Ok(answer) => answer,
                Err(error) => { self.err=Some(error); Answer::default() }
            };
        }
        let args = vec![self.s(cmd.name), self.s(&cmd.args.to_string())];
        let v = self.call("run", args);
        let Some((ok, revision, face_delta, error)) = self.take::<(bool, u64, i64, Option<String>)>(v) else {
            return Answer::default();
        };
        Answer { ok, revision, face_delta, error }
    }
    fn revision(&mut self) -> u64 {
        let v = self.call("revision", vec![]);
        self.take(v).unwrap_or(0)
    }
    fn eye(&mut self) -> Option<P3> {
        let v = self.call("eye", vec![])?;
        if v.is_none() {
            return None;
        }
        match p3(&v) {
            Ok(p) => Some(p),
            Err(e) => {
                self.err.get_or_insert(e);
                None
            }
        }
    }
    fn query(&mut self, what: &str, px: [f64; 2]) -> Vec<(Pick, f64)> {
        let mut args = vec![self.s(what)];
        args.extend(self.px(px));
        let Some(list) = self.call("query", args) else { return vec![] };
        let rows: Vec<(u64, String, f64)> = self.take(Some(list)).unwrap_or_default();
        rows.into_iter().map(|(id, kind, d)| (Pick { id, kind: PickKind::parse(&kind) }, d)).collect()
    }
    fn has(&mut self, what: &str) -> bool {
        let args = vec![self.s(what)];
        let v = self.call("has", args);
        self.take(v).unwrap_or(false)
    }
    fn edge_clear(&mut self, edge: u64, px: [f64; 2]) -> bool {
        let mut args = vec![self.u(edge)];
        args.extend(self.px(px));
        let v = self.call("edge_clear", args);
        self.take(v).unwrap_or(false)
    }
    fn image_in_front(&mut self, face: u64, px: [f64; 2]) -> Option<Pick> {
        let mut args = vec![self.u(face)];
        args.extend(self.px(px));
        let v = self.call("image_in_front", args);
        self.pick_of(v)
    }
    fn extension_at(&mut self, px: [f64; 2]) -> Option<u64> {
        let v = self.call("extension_at", self.px(px));
        self.take::<Option<u64>>(v).flatten()
    }
    fn cycle_status(&mut self, index: usize, count: usize, id: u64) {
        let args = vec![self.u(index as u64), self.u(count as u64), self.u(id)];
        self.call("cycle_status", args);
    }
    fn select(&mut self, ids: &[u64], mode: SelectMode, grow: Grow) {
        let list = PyList::new(self.py(), ids).unwrap().into_any();
        let args = vec![list, self.s(mode.name()), self.s(grow.name())];
        self.call("select", args);
    }
    fn clear_selection(&mut self) { self.call("clear_selection", vec![]); }
    fn selection_signature(&mut self) -> u64 {
        let v = self.call("selection_signature", vec![]);
        self.take(v).unwrap_or(0)
    }
    fn selection_empty(&mut self) -> bool {
        let v = self.call("selection_empty", vec![]);
        self.take(v).unwrap_or(true)
    }
    fn box_select(&mut self, rect: [f64; 4], crossing: bool, mode: SelectMode) {
        let r = PyTuple::new(self.py(), rect).unwrap().into_any();
        let c = crossing.into_pyobject(self.py()).unwrap().to_owned().into_any();
        let args = vec![r, c, self.s(mode.name())];
        self.call("box_select", args);
    }
    fn set_hover(&mut self, id: Option<u64>) {
        let arg = match id { Some(i) => self.u(i), None => self.py().None().into_bound(self.py()) };
        self.call("set_hover", vec![arg]);
    }
    fn set_extension_pick(&mut self, id: Option<u64>) {
        let arg = match id { Some(i) => self.u(i), None => self.py().None().into_bound(self.py()) };
        self.call("set_extension_pick", vec![arg]);
    }
    fn has_extension_pick(&mut self) -> bool {
        let v = self.call("has_extension_pick", vec![]);
        self.take(v).unwrap_or(false)
    }
    fn erase(&mut self, selection_only: bool) -> bool {
        let arg = selection_only.into_pyobject(self.py()).unwrap().to_owned().into_any();
        let v = self.call("erase", vec![arg]);
        self.take(v).unwrap_or(false)
    }
    fn editing(&mut self) -> bool {
        let v = self.call("editing", vec![]);
        self.take(v).unwrap_or(false)
    }
    fn edit_box_contains(&mut self, px: [f64; 2]) -> bool {
        let v = self.call("edit_box_contains", self.px(px));
        self.take(v).unwrap_or(false)
    }
    fn open_placement(&mut self, id: u64) {
        let args = vec![self.u(id)];
        self.call("open_placement", args);
    }
    fn close_placement(&mut self) { self.call("close_placement", vec![]); }
    fn call(&mut self, verb: &str, args: serde_json::Value) -> serde_json::Value {
        if self.err.is_some() { return serde_json::Value::Null; }
        if let Ok(host) = self.obj.cast::<super::view_queries::ViewportToolHost>() {
            match host.borrow().service(self.py(), verb, &args) {
                Ok(Some(result)) => return result,
                Ok(None) => {}
                Err(error) => {
                    self.err = Some(error);
                    return serde_json::Value::Null;
                }
            }
        }
        let a = match json_to_py(self.py(), &args) {
            Ok(a) => a,
            Err(e) => {
                self.err.get_or_insert(e);
                return serde_json::Value::Null;
            }
        };
        let v = self.call("call", vec![self.s(verb), a]);
        match v.map(|v| py_to_json(&v)) {
            Some(Ok(j)) => j,
            Some(Err(e)) => {
                self.err.get_or_insert(e);
                serde_json::Value::Null
            }
            None => serde_json::Value::Null,
        }
    }
    fn message(&mut self, template: &str, args: &[(&str, String)]) {
        let d = PyDict::new(self.py());
        for (k, v) in args {
            if let Err(e) = d.set_item(*k, v) {
                self.err.get_or_insert(e);
            }
        }
        let args = vec![self.s(template), d.into_any()];
        self.call("message", args);
    }
    fn host_verb(&mut self, verb: &str, id: u64) {
        let args = vec![self.s(verb), self.u(id)];
        self.call("host_verb", args);
    }
}

fn preview_vectors(preview: super::Preview) -> Vec<(crate::math::Vec3, crate::math::Vec3)> {
    preview.lines.into_iter().map(|line| (super::plane::vec(line.a.into()), super::plane::vec(line.b.into()))).collect()
}

fn reply<'py>(py: Python<'py>, r: &Reply) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("consumed", r.consumed)?;
    d.set_item("redraw", r.redraw)?;
    d.set_item("committed", r.committed)?;
    Ok(d)
}

#[pyclass(unsendable, module = "ingetrazo_core")]
pub struct ToolMachine {
    tool: Box<dyn Tool>,
    fields: std::cell::OnceCell<serde_json::Value>,
    /// Moves on every event and set: what the adapter caches state by.
    #[pyo3(get)]
    serial: u64,
}

impl ToolMachine {
    fn lock(&mut self) -> PyResult<&mut super::plane::PlaneLock> {
        self.tool.plane_lock().ok_or_else(|| PyValueError::new_err("this tool has no plane lock"))
    }
}

#[pymethods]
impl ToolMachine {
    #[new]
    fn new(kind: &str) -> PyResult<Self> {
        new_tool(kind)
            .map(|tool| ToolMachine { tool, serial: 0, fields: Default::default() })
            .ok_or_else(|| PyValueError::new_err(format!("no Rust tool {kind:?}")))
    }

    #[staticmethod]
    fn group_axes_basis(origin: P3, red: P3, green: P3) -> Option<[P3; 3]> {
        super::change_axes::axes(origin, red, green)
    }

    #[staticmethod]
    fn flip_quad(axis: &str, center: P3, half: P3, margin: f64) -> [P3; 4] {
        super::flip::quad(axis, center, half, margin)
    }

    #[staticmethod]
    fn protractor_angle(basis: [P3; 2], arm: P3, other: P3) -> Option<f64> {
        super::protractor::angle(basis, arm, other)
    }
    #[staticmethod]
    fn protractor_display(degrees: f64, near: bool) -> f64 {
        super::protractor::display(degrees, near)
    }
    #[staticmethod]
    fn protractor_direction(basis: [P3; 2], arm: P3, degrees: f64) -> P3 {
        super::protractor::direction(basis, arm, degrees)
    }

    #[staticmethod]
    fn offset_loop(points: Vec<P3>, normal: P3, distance: f64) -> Option<Vec<P3>> {
        super::offset_tool::closed_offset(&points, normal, distance)
    }

    #[staticmethod]
    fn walk_blocked(host: &Bound<'_, PyAny>, eye: P3, delta: P3, height: f64, clearance: f64, knee: f64) -> PyResult<bool> {
        let mut bridge = PyHost { obj: host, err: None };
        let blocked = super::walkthrough::blocked(&mut bridge, eye, delta, height, clearance, knee);
        bridge.err.map_or(Ok(blocked), Err)
    }

    /// The tool's class attributes, as the inference and the host read them.
    #[getter]
    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let i = self.tool.info();
        let d = PyDict::new(py);
        d.set_item("kind", i.kind)?;
        d.set_item("name", i.name)?;
        d.set_item("shortcut", i.shortcut)?;
        d.set_item("description", i.description)?;
        d.set_item("uses_snap", i.uses_snap)?;
        d.set_item("box_select", i.box_select)?;
        d.set_item("vcb_label", i.vcb_label)?;
        d.set_item("magnetic_axis_deg", i.magnetic_axis_deg)?;
        d.set_item("screen_axis_px", i.screen_axis_px)?;
        d.set_item("axis_source", i.axis_source)?;
        d.set_item("axis_source_empty_only", i.axis_source_empty_only)?;
        Ok(d)
    }

    fn event<'py>(&mut self, py: Python<'py>, ev: &Bound<'py, PyDict>, host: &Bound<'py, PyAny>)
        -> PyResult<Bound<'py, PyDict>> {
        let ev = event(ev)?;
        self.fields.take();
        self.serial += 1;
        let mut h = PyHost { obj: host, err: None };
        let r = self.tool.event(&ev, &mut h);
        if let Some(e) = h.err {
            return Err(e);
        }
        reply(py, &r)
    }

    fn busy(&self) -> bool { self.tool.busy() }

    // ---- The plane lock of a planar machine (tools.base.PlaneLock's
    // state, one owner): the same calls as `ingetrazo_core.PlaneLock`.

    #[getter]
    fn plane_lock<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.lock_get(py, "plane_lock")
    }
    #[setter]
    fn set_plane_lock(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.lock_set("plane_lock", value)
    }
    #[getter]
    fn plane_ref<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.lock_get(py, "plane_ref")
    }
    #[setter]
    fn set_plane_ref(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.lock_set("plane_ref", value)
    }
    #[getter]
    fn hover_plane<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.lock_get(py, "hover_plane")
    }
    #[setter]
    fn set_hover_plane(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.lock_set("hover_plane", value)
    }

    fn lock_get<'py>(&mut self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        use super::plane::{vec, AXIS};
        let l = self.lock()?;
        Ok(match name {
            "plane_lock" => l.lock.map(|k| AXIS[k]).into_pyobject(py)?.into_any(),
            "plane_ref" => match l.reference.clone() {
                None => py.None().into_bound(py),
                Some((n, k)) => (vec(n), k).into_pyobject(py)?.into_any(),
            },
            "hover_plane" => match l.hover {
                None => py.None().into_bound(py),
                Some((p, n)) => (vec(p), vec(n)).into_pyobject(py)?.into_any(),
            },
            _ => return Err(PyValueError::new_err(format!("no plane lock field {name:?}"))),
        })
    }

    fn lock_set(&mut self, name: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        use super::plane::{plane_of, AXIS};
        use crate::math::xyz;
        self.fields.take();
        self.serial += 1;
        let l = self.lock()?;
        match name {
            "plane_lock" => {
                l.lock = if value.is_none() { None } else {
                    let a: String = value.extract()?;
                    Some(AXIS.iter().position(|x| *x == a).ok_or_else(|| PyValueError::new_err("x, y or z"))?)
                }
            }
            "plane_ref" => {
                l.reference = if value.is_none() { None } else {
                    let (n, k): (Bound<PyAny>, String) = value.extract()?;
                    Some((xyz(&n)?, k))
                }
            }
            "hover_plane" => l.hover = plane_of(value)?,
            _ => return Err(PyValueError::new_err(format!("no plane lock field {name:?}"))),
        }
        Ok(())
    }

    fn toggle(&mut self, axis: &str) -> PyResult<()> {
        self.fields.take();
        self.serial += 1;
        let k = super::plane::AXIS.iter().position(|x| *x == axis).ok_or_else(|| PyValueError::new_err("x, y or z"))?;
        self.lock()?.toggle(k);
        Ok(())
    }

    fn clear(&mut self) -> PyResult<()> {
        self.fields.take();
        self.serial += 1;
        self.lock()?.clear();
        Ok(())
    }

    #[pyo3(signature = (point, axes=None))]
    fn locked(&mut self, point: &Bound<'_, PyAny>, axes: Option<&Bound<'_, PyAny>>)
        -> PyResult<Option<(crate::math::Vec3, crate::math::Vec3)>> {
        use super::plane::{axes_of, vec};
        let (p, a) = (crate::math::xyz(point)?, axes_of(axes)?);
        Ok(self.lock()?.locked(p, &a).map(|(p, n)| (vec(p), vec(n))))
    }

    #[pyo3(signature = (work_plane, axes=None))]
    fn own_plane(&mut self, work_plane: &Bound<'_, PyAny>, axes: Option<&Bound<'_, PyAny>>)
        -> PyResult<(crate::math::Vec3, crate::math::Vec3)> {
        use super::plane::{axes_of, plane_of, vec};
        let (w, a) = (plane_of(work_plane)?, axes_of(axes)?);
        let (p, n) = self.lock()?.own_plane(w, &a);
        Ok((vec(p), vec(n)))
    }

    #[pyo3(signature = (plane, points, names_point, axes=None))]
    fn snapped(&mut self, plane: &Bound<'_, PyAny>, points: Vec<Bound<'_, PyAny>>, names_point: Option<bool>,
                    axes: Option<&Bound<'_, PyAny>>) -> PyResult<Option<(crate::math::Vec3, crate::math::Vec3)>> {
        use super::plane::{axes_of, plane_of, vec};
        let Some(plane) = plane_of(plane)? else { return Ok(None) };
        let pts = points.iter().filter(|p| !p.is_none()).map(crate::math::xyz).collect::<PyResult<Vec<_>>>()?;
        let a = axes_of(axes)?;
        Ok(self.lock()?.snapped(plane, &pts, names_point, &a).map(|(p, n)| (vec(p), vec(n))))
    }

    /// A read-only question about the tool's geometry (`Tool::query`).
    fn query<'py>(&self, py: Python<'py>, what: &str, arg: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        json_to_py(py, &self.tool.query(what, &py_to_json(arg)?))
    }

    /// Set a `state()` field; False when the tool has no such field.
    fn set(&mut self, key: &str, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        self.fields.take();
        self.serial += 1;
        Ok(self.tool.set(key, &py_to_json(value)?))
    }

    fn state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> { json_to_py(py, &self.tool.state()) }

    /// One `state()` field (None when absent): what the inference reads
    /// on every pointer move, without building the rest.
    fn get<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Bound<'py, PyAny>> {
        json_to_py(py, self.fields.get_or_init(|| self.tool.state()).get(key).unwrap_or(&serde_json::Value::Null))
    }

    fn set_paste_clipboard(&mut self, clip: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let geometry = super::paste_host::geometry(clip)?;
        self.fields.take();
        self.serial += 1;
        self.tool.set("clipboard", &geometry);
        Ok(())
    }
    fn paste_faces<'py>(&self, py: Python<'py>, rows: &Bound<'py, PyAny>, offset: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        super::paste_host::preview_faces(py, &self.tool.preview_surfaces(), rows, offset)
    }

    fn preview_surfaces(&self) -> Vec<Vec<Vec<crate::math::Vec3>>> {
        self.tool.preview_surfaces().iter().map(|face| face.iter().map(|ring|
            ring.iter().map(|point| super::plane::vec((*point).into())).collect()).collect()).collect()
    }

    fn preview_lines(&self) -> Vec<(crate::math::Vec3,crate::math::Vec3)> {
        preview_vectors(self.tool.preview())
    }

    #[pyo3(signature=(centre, axes, radius, hover=None, sweep=None))]
    fn cursor_lines(&self, centre: &Bound<'_, PyAny>, axes: [P3;2], radius: f64,
                    hover: Option<P3>, sweep: Option<f64>) -> PyResult<Vec<(crate::math::Vec3,crate::math::Vec3)>> {
        let plane = super::preview::CursorPlane { at: crate::math::xyz(centre)?, axes: axes.map(Into::into), radius, sweep };
        Ok(preview_vectors(self.tool.cursor_preview(&plane, hover)))
    }

    /// `{lines: [(a, b, style)], faces: [[p]], box: ((x0, y0, x1, y1), crossing) | None,
    /// value: (at, label) | None}`.
    fn preview<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let p = self.tool.preview();
        let d = PyDict::new(py);
        let lines = PyList::empty(py);
        for l in &p.lines {
            lines.append(((l.a[0], l.a[1], l.a[2]), (l.b[0], l.b[1], l.b[2]), l.style.name()))?;
        }
        d.set_item("lines", lines)?;
        let faces = PyList::empty(py);
        for f in &p.faces {
            faces.append(f.iter().map(|q| (q[0], q[1], q[2])).collect::<Vec<_>>())?;
        }
        d.set_item("faces", faces)?;
        d.set_item("box", p.box_rect.map(|(r, c)| ((r[0], r[1], r[2], r[3]), c)))?;
        d.set_item("value", p.value.map(|v| ((v.at[0], v.at[1], v.at[2]), v.label)))?;
        Ok(d)
    }
}

/// How a click with these Qt modifiers joins the selection:
/// `replace`, `add` (Ctrl), `toggle` (Shift) or `remove` (Shift+Ctrl).
#[pyfunction]
pub fn select_mode(modifiers: u32) -> &'static str {
    SelectMode::from_mods(Mods::from_qt(modifiers)).name()
}

/// What a click at `(x, y)` takes, by the Select machine's cascade, asking
/// `host` (the adapter's) for each kind: the picked handle or None.
#[pyfunction]
#[pyo3(signature = (host, x, y, lines=true))]
pub fn select_pick(host: &Bound<'_, PyAny>, x: f64, y: f64, lines: bool) -> PyResult<Option<u64>> {
    let mut h = PyHost { obj: host, err: None };
    let hit = super::select::pick(&mut h, [x, y], lines);
    if let Some(e) = h.err {
        return Err(e);
    }
    Ok(hit.map(|p| p.id))
}

/// The grip specification is also saved with the last edit for hot retype.
#[pyclass(module = "ingetrazo_core", skip_from_py_object)]
pub struct ScaleGripSpec(super::scale_grip::Grip);

#[pymethods]
impl ScaleGripSpec {
    #[new]
    fn new(extents: P3, axes: Vec<usize>, uniform: bool) -> PyResult<Self> {
        let mut mask = [false; 3];
        for i in axes {
            *mask.get_mut(i).ok_or_else(|| PyValueError::new_err("invalid scale axis"))? = true;
        }
        Ok(Self(super::scale_grip::Grip { extents, mask, uniform }))
    }

    fn typed(&self, values: Vec<f64>, absolute: bool) -> Option<(f64, f64, f64)> {
        self.0.typed(&values, absolute).map(Into::into)
    }

    fn point(&self, grip: P3, point: P3) -> Option<(f64, f64, f64)> {
        self.0.point(grip.into(), point.into()).map(Into::into)
    }

    fn ray(&self, grip: P3, origin: P3, direction: P3) -> Option<(f64, f64, f64)> {
        self.0.ray(grip.into(), origin.into(), direction.into()).map(Into::into)
    }
}

#[pyfunction]
pub fn scale_box_geometry(extents: P3) -> (Vec<(P3, Vec<usize>)>, Vec<(P3, P3)>) {
    let geometry = super::scale_grip::BoxGeometry::new(extents);
    (geometry.grips, geometry.edges)
}

#[pyfunction]
pub fn move_rotation_grips(axes: [P3; 3], lo: P3, hi: P3, eye: P3) -> Vec<(P3, P3, P3)> {
    super::move_grips::rotation_grips(axes, lo, hi, eye)
}

#[pyfunction]
pub fn move_grip_region(points: Vec<[f64; 2]>, cursor: [f64; 2], margin: f64) -> bool {
    super::move_grips::hover_hull(points, cursor, margin)
}
