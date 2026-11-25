// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
#[pyo3(signature=(vp,fast=true))]
pub(super) fn view_line_groups<'p>(vp: &Obj<'p>, fast: bool) -> ResultObj<'p> {
    let py = vp.py();
    let hlr = py.import("core.hlr")?;
    let cam = vp.getattr("camera")?;
    let scene = vp.getattr("scene")?;
    let kw = PyDict::new(py);
    kw.set_item(
        "geometry",
        if fast {
            vp.call_method0("hlr_geometry")?
        } else {
            none(py)
        },
    )?;
    if cam.getattr("perspective")?.is_truthy()? {
        let lines = hlr
            .getattr("hlr_perspective")?
            .call((scene, cam), Some(&kw))?;
        return Ok(PyList::new(py, [("VISTA", lines)])?.into_any());
    }
    let layered = scene.getattr("layers")?.len()? > 1;
    if layered {
        kw.set_item("layers", true)?;
        if fast {
            kw.set_item(
                "geometry",
                vp.call_method(
                    "hlr_geometry",
                    (),
                    Some(&[("tags", true)].into_py_dict(py)?),
                )?,
            )?;
        }
    }
    let drawing = hlr.getattr("hlr_drawing")?.call((&scene, cam), Some(&kw))?;
    if layered {
        drawing.setattr(
            "styles",
            py.import("ingetrazo_core")?
                .getattr("tag_styles")?
                .call_method1("line_styles", (scene, true))?,
        )?;
        return py
            .import("formats.dxf_views")?
            .getattr("dxf_groups_by_layer")?
            .call(
                (drawing, "VISTA"),
                Some(&[("hidden_lines", false)].into_py_dict(py)?),
            );
    }
    let kinds = drawing.getattr("kinds")?;
    let equal = |kind: &str| kinds.call_method1("__eq__", (hlr.getattr(kind)?,));
    let cut = equal("KIND_CUT")?;
    let profile = equal("KIND_PROFILE")?;
    let other = cut
        .call_method1("__or__", (&profile,))?
        .call_method0("__invert__")?;
    let segs = drawing.getattr("segs")?;
    Ok(PyList::new(
        py,
        [
            ("VISTA", segs.get_item(other)?),
            ("VISTA-PERFIL", segs.get_item(profile)?),
            ("VISTA-CORTE", segs.get_item(cut)?),
        ],
    )?
    .into_any())
}
#[pyclass]
struct ActiveDrawing {
    win: Py<PyAny>,
    vp: Py<PyAny>,
}
#[pymethods]
impl ActiveDrawing {
    #[pyo3(signature=(fast=true))]
    fn lines<'p>(&self, py: Python<'p>, fast: bool) -> ResultObj<'p> {
        view_line_groups(self.vp.bind(py), fast)
    }
    #[pyo3(signature=(**options))]
    fn faces<'p>(&self, py: Python<'p>, options: Option<&Bound<'p, PyDict>>) -> ResultObj<'p> {
        let vp = self.vp.bind(py);
        py.import("core.vista_caras")?
            .getattr("view_faces")?
            .call((vp.getattr("scene")?, vp.getattr("camera")?), options)
    }
    fn window(&self, py: Python) -> PyResult<(f64, f64, f64, f64)> {
        rect(&self.vp.bind(py).getattr("camera")?)
    }
    fn isolate(&self, py: Python) -> PyResult<scopes::SelectionScope> {
        Ok(scopes::selection_only(
            self.vp.bind(py).getattr("scene")?.unbind(),
        ))
    }
    fn isolate_picture(&self, py: Python) -> PyResult<scopes::PictureScope> {
        scopes::PictureScope::new(self.vp.bind(py))
    }
    fn widen(&self, py: Python, aspect: f64) -> PyResult<scopes::CameraScope> {
        Ok(scopes::widened(
            self.vp.bind(py).getattr("camera")?.unbind(),
            aspect,
        ))
    }
    fn frame(&self, py: Python, frame: (f64, f64, f64)) -> PyResult<scopes::CameraScope> {
        Ok(scopes::framed(
            self.vp.bind(py).getattr("camera")?.unbind(),
            frame.into(),
        ))
    }
    fn fit(&self, py: Python, selection: bool) -> PyResult<scopes::CameraScope> {
        let vp = self.vp.bind(py);
        let mut bounds = if selection {
            vp.getattr("scene")?.call_method0("selection_bounds")?
        } else {
            self.win.bind(py).call_method0("_extents_bounds")?
        };
        if bounds.is_none() {
            bounds = (py.None(), py.None()).into_pyobject(py)?.into_any()
        }
        Ok(scopes::fitted(
            vp.getattr("camera")?.unbind(),
            bounds.unbind(),
        ))
    }
    #[pyo3(signature=(width,axes=true,aspect=None))]
    fn picture<'p>(
        &self,
        py: Python<'p>,
        width: usize,
        axes: bool,
        aspect: Option<f64>,
    ) -> ResultObj<'p> {
        let vp = self.vp.bind(py);
        let scene = vp.getattr("scene")?;
        let shown = scene.getattr("show_axes")?;
        let height = aspect.map(|a| ((width as f64 / a).round_ties_even() as usize).max(64));
        let display = vp.getattr("keys_stand_in")?;
        let kw = [("overlays", false), ("sharp_shadows", true)].into_py_dict(py)?;
        scene.setattr("show_axes", shown.is_truthy()? && axes)?;
        let pixels = display.call_method("render_pixels", (width, height), Some(&kw));
        scene.setattr("show_axes", shown)?;
        let pixels = pixels?;
        let image = if pixels.is_truthy()? {
            render(py)?
                .getattr("SurveyRaster")?
                .call1(pixels.cast_into::<PyTuple>()?)?
        } else {
            none(py)
        };
        Ok((image, self.window(py)?).into_pyobject(py)?.into_any())
    }
}
#[pyfunction]
pub(super) fn active_view_builder(win: &Obj) -> PyResult<builder::SvgBuilder> {
    let py = win.py();
    let vp = win.call_method0("_active_view")?;
    let mm = |dim: &str| -> PyResult<f64> {
        Ok(vp.call_method0(dim)?.extract::<f64>()?.max(1.) * 25.4 / 96.)
    };
    let size = Some((mm("width")?, mm("height")?));
    let host = Bound::new(
        py,
        ActiveDrawing {
            win: win.clone().unbind(),
            vp: vp.unbind(),
        },
    )?;
    let callback = |name: &str| host.getattr(name).map(Bound::unbind);
    Ok(builder::SvgBuilder::new(
        py,
        callback("lines")?,
        callback("faces")?,
        Some(callback("picture")?),
        100.,
        None,
        Some(callback("window")?),
        size,
        Some(callback("isolate")?),
        Some(callback("widen")?),
        Some(callback("frame")?),
        Some(callback("fit")?),
        Some(callback("isolate_picture")?),
    ))
}
