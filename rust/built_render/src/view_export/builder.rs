// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
type Callback = Option<Py<PyAny>>;
type Frame = (f64, f64, f64);
#[derive(Clone, Copy, Default)]
struct View {
    selection: bool,
    whole: bool,
    aspect: Option<f64>,
    frame: Option<Frame>,
}

#[pyclass(module = "built_render.view_export")]
pub(super) struct SvgBuilder {
    lines: Py<PyAny>,
    faces: Py<PyAny>,
    picture: Callback,
    window: Callback,
    isolate: Callback,
    widen: Callback,
    frame: Callback,
    fit: Callback,
    isolate_picture: Callback,
    #[pyo3(get, set)]
    scale_n: f64,
    #[pyo3(get, set)]
    pens: Callback,
    #[pyo3(get)]
    screen_mm: Option<(f64, f64)>,
    cache: Py<PyDict>,
}
impl SvgBuilder {
    fn cached<'p>(
        &self,
        py: Python<'p>,
        layer: &str,
        options: Obj<'p>,
        view: View,
        work: impl FnOnce() -> ResultObj<'p>,
    ) -> ResultObj<'p> {
        let key = (
            layer,
            options,
            view.selection,
            view.whole,
            view.aspect,
            view.frame,
        )
            .into_pyobject(py)?;
        let cache = self.cache.bind(py);
        if let Some(hit) = cache.get_item(&key)? {
            return Ok(hit);
        }
        let stack = py.import("contextlib")?.call_method0("ExitStack")?;
        let value = scoped(&stack, || {
            let enter = |ctx: Obj| stack.call_method1("enter_context", (ctx,)).map(|_| ());
            if view.selection {
                if let Some(f) = &self.isolate {
                    enter(f.bind(py).call0()?)?;
                }
            }
            if view.whole {
                if let Some(f) = &self.fit {
                    enter(f.bind(py).call1((view.selection,))?)?;
                }
            }
            if let (Some(a), Some(f)) = (view.aspect, &self.widen) {
                enter(f.bind(py).call1((a,))?)?;
            }
            if let (Some(p), Some(f)) = (view.frame, &self.frame) {
                enter(f.bind(py).call1((p,))?)?;
            }
            work()
        })?;
        cache.set_item(key, &value)?;
        Ok(value)
    }
    fn draw_picture<'p>(
        &self,
        py: Python<'p>,
        preview: bool,
        axes: bool,
        view: View,
    ) -> ResultObj<'p> {
        let Some(image) = &self.picture else {
            return Ok(none(py));
        };
        self.cached(
            py,
            "image",
            (preview, axes).into_pyobject(py)?.into_any(),
            view,
            || {
                let width = if preview { PREVIEW_WIDTH } else { FILE_WIDTH };
                let draw = || match view.aspect {
                    Some(a) => image.bind(py).call1((width, axes, a)),
                    None => image.bind(py).call1((width, axes)),
                };
                if view.selection {
                    if let Some(f) = &self.isolate_picture {
                        return scoped(&f.bind(py).call0()?, draw);
                    }
                }
                draw()
            },
        )
    }
}
#[pymethods]
impl SvgBuilder {
    #[new]
    #[pyo3(signature=(lines,faces,image=None,scale_n=100.,pens=None,window=None,screen_mm=None,isolate=None,widen=None,frame=None,fit=None,isolate_picture=None))]
    pub(super) fn new(
        py: Python,
        lines: Py<PyAny>,
        faces: Py<PyAny>,
        image: Callback,
        scale_n: f64,
        pens: Callback,
        window: Callback,
        screen_mm: Option<(f64, f64)>,
        isolate: Callback,
        widen: Callback,
        frame: Callback,
        fit: Callback,
        isolate_picture: Callback,
    ) -> Self {
        Self {
            lines,
            faces,
            picture: image,
            scale_n,
            pens,
            window,
            screen_mm,
            isolate,
            widen,
            frame,
            fit,
            isolate_picture,
            cache: PyDict::new(py).unbind(),
        }
    }
    fn can_frame(&self) -> bool {
        self.frame.is_some()
    }
    #[pyo3(signature=(preview,axes=true,aspect=None,frame=None,whole=false,selection=false))]
    fn image<'p>(
        &self,
        py: Python<'p>,
        preview: bool,
        axes: bool,
        aspect: Option<f64>,
        frame: Option<Frame>,
        whole: bool,
        selection: bool,
    ) -> ResultObj<'p> {
        self.draw_picture(
            py,
            preview,
            axes,
            View {
                aspect,
                frame,
                whole: whole && self.fit.is_some(),
                selection,
            },
        )
    }
    #[pyo3(signature=(options=None,paper=None,preview=false))]
    fn build<'p>(
        &self,
        py: Python<'p>,
        options: Option<&Bound<'p, PyDict>>,
        paper: Option<&Obj<'p>>,
        preview: bool,
    ) -> ResultObj<'p> {
        let paper = paper.cloned().unwrap_or_else(|| none(py));
        let mut area = String::new();
        let mut framing = None;
        let size = if let Ok(d) = paper.cast::<PyDict>() {
            area = d
                .get_item("area")?
                .map(|v| v.extract())
                .transpose()?
                .unwrap_or_default();
            if area == "view" && self.frame.is_some() {
                if let Some(value) = d.get_item("frame")?.filter(|v| !v.is_none()) {
                    let [x, y, z]: [f64; 3] = value.extract()?;
                    let round = |v: f64| (v * 1e6).round_ties_even() / 1e6;
                    if x.abs() >= 1e-9 || y.abs() >= 1e-9 || (z - 1.).abs() >= 1e-9 {
                        framing = Some((round(x), round(y), round(z)));
                    }
                }
            }
            d.get_item("size")?.unwrap_or_else(|| none(py))
        } else {
            if paper.extract::<String>().is_ok_and(|s| s == "screen") {
                area = "view".into();
            }
            paper
        };
        let screen = size.extract::<String>().is_ok_and(|s| s == "screen");
        let size = if screen {
            self.screen_mm
        } else if size.is_none() {
            None
        } else {
            let [w, h]: [f64; 2] = size.extract()?;
            Some((w, h))
        };
        let mut view = View {
            selection: area == "selection" && self.isolate.is_some(),
            whole: matches!(area.as_str(), "model" | "selection") && self.fit.is_some(),
            frame: framing,
            ..View::default()
        };
        if area == "view" && self.widen.is_some() {
            if let (Some((w, h)), Some((sw, sh))) = (size, self.screen_mm) {
                let want = w / h.max(1e-9);
                if (want / (sw / sh.max(1e-9)) - 1.).abs() >= 1e-3 {
                    view.aspect = Some((want * 1e4).round_ties_even() / 1e4);
                }
            }
        }
        let mut image = none(py);
        let mut faces = none(py);
        let mut line_colours = true;
        if let Some(opts) = options.filter(|o| !o.is_empty()) {
            let flag = |name: &str, def: bool| -> PyResult<bool> {
                opts.get_item(name)?
                    .map(|v| v.is_truthy())
                    .transpose()
                    .map(|v| v.unwrap_or(def))
            };
            line_colours = flag("materials", true)?;
            if flag("image", false)? {
                image = self.draw_picture(py, preview, flag("axes", true)?, view)?
            } else {
                let key = py
                    .import("builtins")?
                    .call_method1("frozenset", (opts.items(),))?;
                faces = self.cached(py, "faces", key, view, || {
                    self.faces.bind(py).call((), Some(opts))
                })?;
            }
        }
        let mut window = None;
        if let Some(f) = self.window.as_ref().filter(|_| area == "view") {
            let value = self.cached(py, "window", none(py), view, || f.bind(py).call0())?;
            if !value.is_none() {
                let [x0, y0, x1, y1]: [f64; 4] = value.extract()?;
                window = Some(if let Some((sw, sh)) = size {
                    let (mut w, mut h) = ((x1 - x0).max(1e-9), (y1 - y0).max(1e-9));
                    let ratio = sw / sh.max(1e-9);
                    if ratio > w / h {
                        w = h * ratio
                    } else {
                        h = w / ratio
                    }
                    (
                        (x0 + x1 - w) / 2.,
                        (y0 + y1 - h) / 2.,
                        (x0 + x1 + w) / 2.,
                        (y0 + y1 + h) / 2.,
                    )
                } else {
                    (x0, y0, x1, y1)
                });
            }
        }
        let lines = self.cached(py, "lines", none(py), view, || {
            if view.selection || view.whole || view.frame.is_some() || view.aspect.is_some() {
                self.lines
                    .bind(py)
                    .call((), Some(&[("fast", !view.selection)].into_py_dict(py)?))
            } else {
                self.lines.bind(py).call0()
            }
        })?;
        let kw = PyDict::new(py);
        kw.set_item("window", window)?;
        kw.set_item("margin", window.map(|_| 0.))?;
        if preview {
            kw.set_item("bake_px_per_mm", 3.)?;
            kw.set_item("seam_mm", 0.6)?;
        }
        py.import("built_core")?.getattr("view_svg_text")?.call(
            (
                lines,
                self.scale_n,
                &self.pens,
                faces,
                line_colours,
                image,
                size,
            ),
            Some(&kw),
        )
    }
}
