//! Document interaction queries over a viewport. The viewport supplies the
//! resident hit-test surface and Python record objects; machines see handles.
use super::handles::ToolHandles;
use pyo3::{
    prelude::*,
    types::{PyDict, PyFrozenSet, PyTuple},
};

fn method<'py>(view: &Bound<'py, PyAny>, name: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    Ok(view.getattr_opt(name)?.filter(|f| !f.is_none()))
}
pub(super) fn hit<'py>(
    view: &Bound<'py, PyAny>,
    name: &str,
    px: (f64, f64),
) -> PyResult<Option<Bound<'py, PyAny>>> {
    Ok(match method(view, name)? {
        Some(f) => Some(f.call1(px)?).filter(|v| !v.is_none()),
        None => None,
    })
}
pub(super) fn classify(entity: &Bound<'_, PyAny>) -> PyResult<&'static str> {
    let py = entity.py();
    type Classes = Vec<(Py<PyAny>, &'static str)>;
    static CLASSES: pyo3::sync::PyOnceLock<Classes> = pyo3::sync::PyOnceLock::new();
    let classes = CLASSES.get_or_try_init(py, || {
        [
            ("core.mesh", "Face", "face"),
            ("core.mesh", "Edge", "edge"),
            ("core.group", "Group", "group"),
            ("built_core.section", "SectionPlane", "section_plane"),
            ("built_core.textlabel", "TextLabel", "text_label"),
            ("core.dimension", "Dimension", "dimension"),
            ("built_core.image_plane", "ImagePlane", "image_plane"),
            ("built_core.guide", "Guide", "guide"),
            ("built_core.guide", "GuideCircle", "guide"),
            ("built_core.georef.geopath", "GeoPath", "geopath"),
        ]
        .into_iter()
        .map(|(module, class, kind)| Ok((py.import(module)?.getattr(class)?.unbind(), kind)))
        .collect::<PyResult<Classes>>()
    })?;
    for (class, kind) in classes {
        if entity.is_instance(class.bind(py))? {
            return Ok(kind);
        }
    }
    Ok("other")
}

#[pyclass(module = "built_core", subclass)]
pub struct ViewportToolHost {
    #[pyo3(get)]
    pub(super) vp: Py<PyAny>,
    #[pyo3(get)]
    handles: Option<Py<ToolHandles>>,
    #[pyo3(get)]
    pub(super) tool: Option<Py<PyAny>>,
}
impl ViewportToolHost {
    pub(super) fn id(&self, entity: &Bound<'_, PyAny>) -> PyResult<u64> {
        self.table(entity.py())?.try_borrow_mut()?.id(entity)
    }
    fn table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, ToolHandles>> {
        self.handles
            .as_ref()
            .map(|h| h.bind(py).clone())
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("pick without a handle table"))
    }
    pub(super) fn object<'py>(&self, py: Python<'py>, id: u64) -> PyResult<Bound<'py, PyAny>> {
        Ok(self
            .table(py)?
            .try_borrow()?
            .get(py, Some(id))
            .unwrap_or_else(|| py.None().into_bound(py)))
    }
}
#[pymethods]
impl ViewportToolHost {
    #[new]
    #[pyo3(signature=(viewport, handles=None, tool=None))]
    fn new(viewport: Py<PyAny>, handles: Option<Py<ToolHandles>>, tool: Option<Py<PyAny>>) -> Self {
        Self {
            vp: viewport,
            handles,
            tool,
        }
    }
    fn arc_tangent(&self, py: Python<'_>, point: [f64; 3]) -> PyResult<Option<[f64; 3]>> {
        let value = self.arc_service(py, "tangent_at", &serde_json::json!({"point":point}))?;
        serde_json::from_value(value)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }
    fn revision(&self, py: Python<'_>) -> PyResult<u64> {
        self.history_token(py)
    }
    fn run(
        &self,
        py: Python<'_>,
        name: &str,
        args: &str,
    ) -> PyResult<(bool, u64, i64, Option<String>)> {
        let arguments = serde_json::from_str(args)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let a = self.command(py, name, &arguments)?;
        Ok((a.ok, a.revision, a.face_delta, a.error))
    }
    fn call<'py>(
        &self,
        py: Python<'py>,
        verb: &str,
        args: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let tool = self
            .tool
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("callback without a tool"))?;
        tool.bind(py)
            .call_method1(format!("host_{verb}"), (self.vp.bind(py), args))
    }
    fn eye(&self, py: Python<'_>) -> PyResult<Option<[f64; 3]>> {
        let Some(camera) = method(self.vp.bind(py), "camera")? else {
            return Ok(None);
        };
        method(&camera, "eye")?
            .map(|f| crate::math::xyz(&f.call0()?).map(|p| p.to_array()))
            .transpose()
    }
    fn query(
        &self,
        py: Python<'_>,
        what: &str,
        x: f64,
        y: f64,
    ) -> PyResult<Vec<(u64, &'static str, f64)>> {
        let view = self.vp.bind(py);
        let px = (x, y);
        let mut rows = Vec::new();
        let mut push = |entity: Bound<'_, PyAny>, dist: f64| -> PyResult<()> {
            if !entity.is_none() {
                rows.push((self.id(&entity)?, classify(&entity)?, dist));
            }
            Ok(())
        };
        match what {
            "linework_edge" => {
                if let Some(v) = hit(view, "pick_linework_edge", px)? {
                    let (entity, dist) = v.extract()?;
                    push(entity, dist)?;
                }
            }
            "linework_groups" | "groups_around" => {
                let name = if what == "linework_groups" {
                    "pick_linework_groups"
                } else {
                    "linework_groups_around"
                };
                if let Some(items) = hit(view, name, px)? {
                    for item in items.try_iter()? {
                        let item = item?;
                        if item.is_instance_of::<PyTuple>() {
                            let (entity, dist) = item.extract()?;
                            push(entity, dist)?;
                        } else {
                            push(item, 0.)?;
                        }
                    }
                }
            }
            "label_block" => {
                if let Some(f) = method(view, "pick_text_label")? {
                    let kw = PyDict::new(py);
                    kw.set_item("rect_only", true)?;
                    push(f.call(px, Some(&kw))?, 0.)?;
                }
            }
            _ => {
                let name = match what {
                    "visible_edge" if method(view, "pick_visible_edge")?.is_some() => {
                        "pick_visible_edge"
                    }
                    "visible_edge" => "pick_edge",
                    "face" => "pick_face",
                    "group" => "pick_group",
                    "dimension" => "pick_dimension",
                    "text_label" => "pick_text_label",
                    "geopath" => "pick_geopath",
                    "section_plane" => "pick_section_plane",
                    "guide" => "pick_guide",
                    "image_plane" => "pick_image_plane",
                    _ => return Ok(rows),
                };
                if let Some(obj) = hit(view, name, px)? {
                    push(obj, 0.)?;
                }
            }
        }
        Ok(rows)
    }
    fn has(&self, py: Python<'_>, what: &str) -> PyResult<bool> {
        Ok(what == "linework" && self.vp.bind(py).hasattr("pick_linework_edge")?)
    }
    fn edge_clear(&self, py: Python<'_>, id: u64, x: f64, y: f64) -> PyResult<bool> {
        let view = self.vp.bind(py);
        let (Some(point), Some(occluded)) = (
            method(view, "edge_point_under_cursor")?,
            method(view, "_is_occluded")?,
        ) else {
            return Ok(false);
        };
        let p = point.call1((self.object(py, id)?, x, y))?;
        Ok(!p.is_none() && !occluded.call1((p,))?.is_truthy()?)
    }
    fn image_in_front(
        &self,
        py: Python<'_>,
        id: u64,
        x: f64,
        y: f64,
    ) -> PyResult<Option<(u64, &'static str)>> {
        let Some(f) = method(self.vp.bind(py), "image_in_front_of")? else {
            return Ok(None);
        };
        let entity = f.call1((self.object(py, id)?, x, y))?;
        if entity.is_none() {
            Ok(None)
        } else {
            Ok(Some((self.id(&entity)?, classify(&entity)?)))
        }
    }
    fn extension_at(&self, py: Python<'_>, x: f64, y: f64) -> PyResult<Option<u64>> {
        hit(self.vp.bind(py), "pick_extension_item", (x, y))?
            .map(|e| self.id(&e))
            .transpose()
    }
    fn clear_selection(&self, py: Python<'_>) -> PyResult<()> {
        self.vp
            .bind(py)
            .getattr("scene")?
            .call_method0("clear_selection")?;
        Ok(())
    }
    fn selection_signature(&self, py: Python<'_>) -> PyResult<u64> {
        let selection = self.vp.bind(py).getattr("scene")?.getattr("selection")?;
        let value = py.get_type::<PyFrozenSet>().call1((selection,))?;
        Ok(value.hash()? as u64 & i64::MAX as u64)
    }
    fn selection_empty(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(!self
            .vp
            .bind(py)
            .getattr("scene")?
            .getattr("selection")?
            .is_truthy()?)
    }
    #[pyo3(signature=(ident))]
    fn set_hover(&self, py: Python<'_>, ident: Option<u64>) -> PyResult<()> {
        self.vp.bind(py).call_method1(
            "set_hover",
            (ident.map(|id| self.object(py, id)).transpose()?,),
        )?;
        Ok(())
    }
    #[pyo3(signature=(ident))]
    fn set_extension_pick(&self, py: Python<'_>, ident: Option<u64>) -> PyResult<()> {
        let view = self.vp.bind(py);
        if let Some(id) = ident {
            view.call_method1("set_extension_pick", (self.object(py, id)?,))?;
        } else {
            view.call_method0("clear_extension_pick")?;
        }
        Ok(())
    }
    fn has_extension_pick(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(method(self.vp.bind(py), "extension_pick")?.is_some())
    }
    fn editing(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(!self
            .vp
            .bind(py)
            .getattr("scene")?
            .getattr("edit_group")?
            .is_none())
    }
    fn edit_box_contains(&self, py: Python<'_>, x: f64, y: f64) -> PyResult<bool> {
        Ok(
            hit(self.vp.bind(py), "edit_group_box_contains_pixel", (x, y))?
                .is_some_and(|v| v.is_truthy().unwrap_or(false)),
        )
    }
    fn open_placement(&self, py: Python<'_>, id: u64) -> PyResult<()> {
        self.vp
            .bind(py)
            .call_method1("begin_group_edit", (self.object(py, id)?,))?;
        Ok(())
    }
    fn close_placement(&self, py: Python<'_>) -> PyResult<()> {
        self.vp.bind(py).call_method0("end_one_group_edit")?;
        Ok(())
    }
}
