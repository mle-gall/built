//! Per-document ownership of cameras, edit targets and tab suspension.
//! Python pane objects are handles into the current document controller.
use super::ModelViews;
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
    PyTraverseError, PyVisit,
};
use std::{
    cell::{Cell, RefCell},
    time::Instant,
};

type Slots = [Option<Py<PyAny>>; 4];

#[pyclass(unsendable)]
pub struct DocumentViews {
    model: RefCell<ModelViews>,
    displays: RefCell<Slots>,
    saved: RefCell<Slots>,
    visible: Cell<bool>,
    tolerance: Cell<f32>,
}

#[pymethods]
impl DocumentViews {
    #[new]
    fn new() -> Self {
        Self {
            model: RefCell::new(ModelViews::new()),
            displays: Default::default(),
            saved: Default::default(),
            visible: Cell::new(false),
            tolerance: Cell::new(16.),
        }
    }
    #[getter]
    fn layout(&self) -> &'static str {
        self.model.borrow().layout()
    }
    #[getter]
    fn visible(&self) -> bool {
        self.visible.get()
    }
    #[getter]
    fn active(&self) -> usize {
        self.model.borrow().active
    }
    #[getter]
    fn shares(&self) -> [f32; 3] {
        self.model.borrow().shares()
    }

    fn command_view<'py>(&self, doc: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        if self.visible() {
            if let Some(pane) = self.display(doc.py(), self.active()) {
                return pane.getattr("source_viewport");
            }
        }
        let front = doc.getattr("front_view")?;
        if front.is_none() { doc.getattr("viewport") } else { Ok(front) }
    }

    fn choose(&self, doc: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
        self.model.borrow_mut().choose(key)?;
        if key == "single" {
            self.suspend(doc)?;
        }
        self.read_tolerance(doc.py())?;
        doc.getattr("stand_in")?.call_method0("update")?;
        Ok(())
    }
    fn cancel(&self) {
        self.model.borrow_mut().cancel();
    }
    fn cancel_divider(&self) -> bool {
        self.model.borrow_mut().cancel_divider()
    }
    fn maximize(&self, doc: &Bound<'_, PyAny>) -> PyResult<()> {
        self.model.borrow_mut().toggle_maximized();
        doc.getattr("stand_in")?.call_method0("update")?;
        Ok(())
    }
    fn archive<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        self.model.borrow().archive(py)
    }
    fn restore(&self, doc: &Bound<'_, PyAny>) -> PyResult<()> {
        self.suspend(doc)?;
        self.saved.replace(Default::default());
        let home = doc
            .getattr("viewport")?
            .getattr("scene")?
            .getattr("views_home")?;
        self.model.borrow_mut().restore(&home);
        if self.layout() != "single" {
            self.read_tolerance(doc.py())?;
        }
        Ok(())
    }

    fn suspend(&self, doc: &Bound<'_, PyAny>) -> PyResult<()> {
        self.visible.set(false);
        self.cancel();
        if self.displays.borrow().iter().all(Option::is_none) {
            return Ok(());
        }
        let main = doc.getattr("viewport")?;
        let primary = doc.getattr("stand_in")?;
        self.activate(doc, &main)?;
        main.setattr("keys_stand_in", &primary)?;
        main.setattr("_host", primary.getattr("_host")?)?;
        doc.setattr("front_view", doc.py().None())?;
        primary.getattr("tools")?.setattr("view", &primary)?;
        main.setattr("tool_preview", true)?;
        for (index, slot) in self
            .displays
            .replace(Default::default())
            .into_iter()
            .enumerate()
        {
            if let Some(display) = slot {
                let display = display.bind(doc.py());
                if index != 1 {
                    let vp = display.getattr("source_viewport")?;
                    let pose = (vp.getattr("camera")?, vp.getattr("view_style")?)
                        .into_pyobject(doc.py())?;
                    self.saved.borrow_mut()[index] = Some(pose.into_any().unbind());
                }
                display.call_method0("dispose")?;
            }
        }
        Ok(())
    }

    fn frame<'py>(&self, doc: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        self.visible.set(false);
        let py = doc.py();
        let primary = doc.getattr("stand_in")?;
        if self.layout() == "single"
            || primary.getattr("renderer")?.is_none()
            || primary.getattr("packed")?.is_none()
            || !primary
                .getattr("_ui_attached")
                .is_ok_and(|v| v.is_truthy().unwrap_or(false))
        {
            return primary.call_method0("draw_frame");
        }
        let ui = py.import("views.wgpu_ui")?;
        if crate::document_registration::view::calls::chrome_only(&primary)?
            || py
                .import("ingetrazo_render.sheet_workspace")?
                .call_method1("shown", (&primary,))?
                .is_truthy()?
        {
            return primary.call_method0("draw_frame");
        }
        let renderer = primary.getattr("renderer")?;
        let Some(area) = renderer
            .call_method0("ui_free_ndc")?
            .extract::<Option<[f64; 4]>>()?
        else {
            return primary.call_method0("draw_frame");
        };
        let started = Instant::now();
        let width = primary.call_method0("width")?.extract()?;
        let height = primary.call_method0("height")?.extract()?;
        let panes = self.model.borrow_mut().arrange(width, height, area);
        if panes.is_empty() {
            return primary.call_method0("draw_frame");
        }
        if primary.getattr("_scene_dirty")?.is_truthy()? {
            primary.call_method0("_upload")?;
        }
        for index in 0..4 {
            if !panes.iter().any(|p| p.0 == index) {
                if let Some(display) = self.display(py, index) {
                    if !display.getattr("renderer")?.is_none() {
                        display.call_method0("release")?;
                    }
                }
            }
        }
        let active = self.active();
        let preview = self.model.borrow().hover.unwrap_or(active);
        let mut tiles = Vec::with_capacity(panes.len());
        for (id, x, y, w, h) in panes {
            let display = self.ensure_pane(doc, id)?;
            let host = display.getattr("_host")?;
            host.setattr("x", x)?;
            host.setattr("y", y)?;
            host.setattr("width", w.max(1))?;
            host.setattr("height", h.max(1))?;
            let vp = display.getattr("source_viewport")?;
            if id == preview {
                self.activate(doc, &vp)?;
            }
            vp.setattr("tool_preview", id == preview)?;
            tiles.push((display, vp, [x, y, w, h], id == active));
        }
        let tr = py.import("core.i18n")?.getattr("tr")?;
        let mut batch = Vec::with_capacity(tiles.len());
        for (display, vp, bounds, selected) in tiles {
            display.call_method0("draw_frame")?;
            let camera = vp.getattr("camera")?;
            let forward = camera
                .call_method0("forward")?
                .call_method0("toTuple")?
                .extract()?;
            let projection = camera
                .call_method0("projection_mode")?
                .extract::<String>()?;
            let title = self.model.borrow().title(forward, &projection);
            batch.push((
                display.getattr("_target")?,
                bounds,
                selected,
                tr.call1((title,))?,
            ));
        }
        self.visible.set(true);
        ui.call_method1("before_frame", (&primary,))?;
        let guides = self.model.borrow().guides();
        let (cpu, gpu): (f64, Option<f64>) = primary
            .getattr("renderer")?
            .call_method1("present_model_views", (batch, guides))?
            .extract()?;
        ui.call_method1("after_frame", (&primary,))?;
        let result = (cpu, gpu, started.elapsed().as_secs_f64() * 1000.);
        primary
            .getattr("frameReady")?
            .call_method1("emit", result)?;
        Ok(result.into_pyobject(py)?.into_any())
    }

    /// Chrome gets first refusal until a pane or divider captures the pointer.
    fn route<'py>(
        &self,
        doc: &Bound<'py, PyAny>,
        kind: &str,
        method: &str,
        event: &Bound<'py, PyAny>,
        held: u32,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        let primary = doc.getattr("stand_in")?;
        if !self.visible() {
            return Ok(Some(primary));
        }
        let host = primary.getattr("_host")?;
        if host.call_method0("_modal")?.is_truthy()? {
            self.cancel();
            return Ok(Some(primary));
        }
        let captured = self.model.borrow().captured();
        if !captured
            && doc
                .py()
                .import("views.wgpu_view")?
                .call_method1("_ui_claims", (&primary, method, event))?
                .is_truthy()?
        {
            self.model.borrow_mut().hover = None;
            if let Some(pane) = self.display(doc.py(), self.active()) {
                self.activate(doc, &pane.getattr("source_viewport")?)?;
            }
            return Ok(None);
        }
        let pos = event.call_method0("position")?;
        let x: f64 = pos.call_method0("x")?.extract()?;
        let y: f64 = pos.call_method0("y")?.extract()?;
        let left = event.hasattr("button")?
            && event
                .call_method0("button")?
                .getattr("value")?
                .extract::<u32>()?
                == 1;
        let cursor =
            self.model
                .borrow_mut()
                .divider(kind, x as f32, y as f32, left, self.tolerance.get());
        if let Some(cursor) = cursor {
            let crossing_blocks_tool = if cursor == "move" && !captured {
                let front = doc.getattr("front_view")?;
                let vp = if front.is_none() { doc.getattr("viewport")? } else { front };
                let tool = vp.getattr("active_tool")?;
                !vp.getattr("nav_mode")?.is_none()
                    || (!tool.is_none() && !tool.getattr("name")?.eq("Select")?)
            } else {
                false
            };
            if !crossing_blocks_tool {
                host.call_method1("cursor_changed", (cursor,))?;
                return Ok(None);
            }
            self.cancel_divider();
        }
        let hit = self.model.borrow_mut().pointer(kind, x, y, held != 0);
        let Some((id, local_x, local_y)) = hit else {
            return Ok(None);
        };
        let Some(display) = self.display(doc.py(), id) else {
            return Ok(None);
        };
        self.activate(doc, &display.getattr("source_viewport")?)?;
        event.call_method1("set_local_position", (local_x, local_y))?;
        Ok(Some(display))
    }

    fn key_view<'py>(
        &self,
        doc: &Bound<'py, PyAny>,
        method: &str,
        event: &Bound<'py, PyAny>,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        let primary = doc.getattr("stand_in")?;
        if !self.visible() {
            return Ok(Some(primary));
        }
        let py = doc.py();
        let claimed = if method == "event" {
            crate::document_registration::view::calls::claims_key(&primary, event)?
        } else {
            py.import("views.wgpu_view")?
                .call_method1("_ui_claims", (&primary, method, event))?
        };
        if claimed.is_truthy()? {
            event.call_method0("accept")?;
            Ok(None)
        } else {
            Ok(self.display(py, self.model.borrow().hover.unwrap_or(self.active())))
        }
    }

    fn fit_bounds(&self, win: &Bound<'_, PyAny>, bounds: &Bound<'_, PyAny>) -> PyResult<()> {
        let args = PyTuple::new(win.py(), bounds.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        if self.visible() {
            for id in self.model.borrow().panes.iter().map(|p| p.id) {
                if let Some(pane) = self.display(win.py(), id) {
                    pane.getattr("source_viewport")?
                        .getattr("camera")?
                        .call_method1("fit_box", &args)?;
                }
            }
        } else {
            let kwargs = PyDict::new(win.py());
            kwargs.set_item(
                "free",
                crate::document_registration::view::calls::free_area(win)?,
            )?;
            win.getattr("viewport")?.getattr("camera")?.call_method(
                "fit_box",
                &args,
                Some(&kwargs),
            )?;
        }
        win.getattr("_host")?.call_method0("request_frame")?;
        Ok(())
    }
    fn preserve_peer_styles(&self, doc: &Bound<'_, PyAny>) -> PyResult<()> {
        let style = doc
            .getattr("viewport")?
            .getattr("scene")?
            .getattr("display_style")?;
        if !style.is_none() {
            for id in [0, 2, 3] {
                if let Some(p) = self.display(doc.py(), id) {
                    let vp = p.getattr("source_viewport")?;
                    if vp.getattr("view_style")?.is_none() {
                        vp.setattr("view_style", style.call_method0("copy")?)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for slots in [&self.displays, &self.saved] {
            if let Ok(slots) = slots.try_borrow() {
                for obj in slots.iter().flatten() {
                    visit.call(obj)?;
                }
            }
        }
        Ok(())
    }
    fn __clear__(&mut self) {
        self.displays.replace(Default::default());
        self.saved.replace(Default::default());
    }
}

impl DocumentViews {
    fn read_tolerance(&self, py: Python<'_>) -> PyResult<()> {
        let value = py
            .import("core.settings")?
            .call_method0("settings")?
            .call_method1("value", ("views/divider_snap_px", 16))?;
        let n = py
            .import("builtins")?
            .call_method1("int", (value,))
            .and_then(|v| v.extract::<f32>())
            .unwrap_or(16.);
        self.tolerance.set(n.max(0.));
        Ok(())
    }
    fn display<'py>(&self, py: Python<'py>, index: usize) -> Option<Bound<'py, PyAny>> {
        self.displays.borrow()[index]
            .as_ref()
            .map(|v| v.bind(py).clone())
    }
    fn ensure_pane<'py>(
        &self,
        doc: &Bound<'py, PyAny>,
        index: usize,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = doc.py();
        if let Some(p) = self.display(py, index) {
            return Ok(p);
        }
        let initial = self.model.borrow().initial_view(index);
        let saved = self.saved.borrow()[index]
            .as_ref()
            .map(|v| v.bind(py).clone());
        let display = py
            .import("ingetrazo_render.shell_views")?
            .call_method1("make_pane", (doc, index == 1, initial, saved))?;
        self.displays.borrow_mut()[index] = Some(display.clone().unbind());
        Ok(display)
    }
    fn activate(&self, doc: &Bound<'_, PyAny>, vp: &Bound<'_, PyAny>) -> PyResult<()> {
        let front = doc.getattr("front_view")?;
        let previous = if front.is_none() {
            doc.getattr("viewport")?
        } else {
            front
        };
        let tools = doc.getattr("stand_in")?.getattr("tools")?;
        let display = vp.getattr("keys_stand_in")?;
        if !previous.is(vp) {
            for field in ["active_tool", "suspended_tool", "nav_mode"] {
                vp.setattr(field, previous.getattr(field)?)?;
                previous.setattr(field, doc.py().None())?;
            }
            vp.setattr("_value_buffer", previous.getattr("_value_buffer")?)?;
            previous.setattr("tool_preview", false)?;
        }
        let old = tools.getattr("view")?;
        if !old.is(&display) {
            if old.hasattr("idle_tools")? {
                old.setattr("tools", old.getattr("idle_tools")?)?;
            }
            tools.setattr("view", &display)?;
            display.setattr("tools", &tools)?;
            display.setattr("_preview_key", doc.py().None())?;
            tools.call_method0("claim")?;
        }
        vp.setattr("tool_preview", true)?;
        doc.setattr("front_view", vp)
    }
}
