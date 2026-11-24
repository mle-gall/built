// SPDX-License-Identifier: GPL-3.0-or-later
//! Project document annotations onto the live or export painter.
use crate::document_host::*;
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
type Point = (f64, f64);
type Segment = (Point, Point);
struct Paint<'py> {
    v: Obj<'py>,
    p: Obj<'py>,
    styles: Bound<'py, PyModule>,
}
impl<'py> Paint<'py> {
    fn new(v: &Obj<'py>, p: &Obj<'py>) -> PyResult<Self> {
        Ok(Self {
            v: v.clone(),
            p: p.clone(),
            styles: host(v.py())?,
        })
    }
    fn project(&self, p: &Obj) -> PyResult<Option<Point>> {
        self.v.call_method1("_world_to_pixel", (p,))?.extract()
    }
    fn segment(&self, a: &Obj, b: &Obj) -> PyResult<Option<Segment>> {
        let s = self.v.call_method1("_clip_segment_front", (a, b))?;
        if s.is_none() {
            return Ok(None);
        }
        Ok(self
            .project(&s.get_item(0)?)?
            .zip(self.project(&s.get_item(1)?)?))
    }
    fn px(&self, p: Point) -> PyResult<Obj<'py>> {
        self.styles.call_method1("QPointF", p)
    }
    fn rect(&self, x: f64, y: f64, w: f64, h: f64) -> PyResult<Obj<'py>> {
        self.styles.call_method1("QRectF", (x, y, w, h))
    }
    fn rgb(&self, c: [i32; 3]) -> PyResult<Obj<'py>> {
        self.styles.call_method1("QColor", (c[0], c[1], c[2]))
    }
    fn rgba(&self, c: [i32; 4]) -> PyResult<Obj<'py>> {
        self.styles.call_method1("QColor", (c[0], c[1], c[2], c[3]))
    }
    fn tint(&self, c: [f64; 3], a: f64) -> PyResult<Obj<'py>> {
        self.styles
            .getattr("QColor")?
            .call_method1("fromRgbF", (c[0], c[1], c[2], a))
    }
    fn flag(&self, name: &str) -> PyResult<Obj<'py>> {
        self.styles.getattr("Qt")?.getattr(name)
    }
    fn make_pen(&self, color: &Obj<'py>, width: f64, style: Option<&str>) -> PyResult<Obj<'py>> {
        match style {
            Some(s) => self
                .styles
                .call_method1("QPen", (color, width, self.flag(s)?)),
            None => self.styles.call_method1("QPen", (color, width)),
        }
    }
    fn pen(&self, color: &Obj<'py>, width: f64) -> PyResult<()> {
        self.set_pen(&self.make_pen(color, width, None)?)
    }
    fn set_pen(&self, pen: &Obj) -> PyResult<()> {
        self.p.call_method1("setPen", (pen,))?;
        Ok(())
    }
    fn brush(&self, color: &Obj) -> PyResult<()> {
        self.p.call_method1("setBrush", (color,))?;
        Ok(())
    }
    fn no_brush(&self) -> PyResult<()> {
        self.brush(&self.flag("NoBrush")?)
    }
    fn line(&self, a: Point, b: Point) -> PyResult<()> {
        self.p
            .call_method1("drawLine", (self.px(a)?, self.px(b)?))?;
        Ok(())
    }
    fn circle(&self, at: Point, r: f64) -> PyResult<()> {
        self.p.call_method1("drawEllipse", (self.px(at)?, r, r))?;
        Ok(())
    }
    fn cross(&self, (x, y): Point, r: f64) -> PyResult<()> {
        self.line((x - r, y), (x + r, y))?;
        self.line((x, y - r), (x, y + r))
    }
    fn polygon(&self, points: &[Point]) -> PyResult<()> {
        let list = PyList::new(
            self.v.py(),
            points
                .iter()
                .map(|&p| self.px(p))
                .collect::<PyResult<Vec<_>>>()?,
        )?;
        self.p.call_method1(
            "drawPolygon",
            (self.styles.call_method1("QPolygonF", (list,))?,),
        )?;
        Ok(())
    }
    fn text(&self, p: Point, s: &str) -> PyResult<()> {
        self.p.call_method1("drawText", (self.px(p)?, s))?;
        Ok(())
    }
    fn font(&self, size: i32, bold: bool, italic: bool) -> PyResult<()> {
        let font = self.styles.call_method0("QFont")?;
        font.call_method1("setPointSize", (size,))?;
        if bold {
            font.call_method1("setBold", (true,))?;
        }
        if italic {
            font.call_method1("setItalic", (true,))?;
        }
        self.p.call_method1("setFont", (font,))?;
        Ok(())
    }
    fn shadow_text(&self, (x, y): Point, s: &str, ink: &Obj, shadow: &Obj) -> PyResult<()> {
        self.pen(shadow, 1.)?;
        self.text((x + 1., y + 1.), s)?;
        self.pen(ink, 1.)?;
        self.text((x, y), s)
    }
}
fn truth(o: &Obj, name: &str) -> PyResult<bool> {
    opt(o, name)?.is_truthy()
}
fn tr(py: Python, word: &str) -> PyResult<String> {
    host(py)?.call_method1("tr", (word,))?.extract()
}
mod geo;
mod labels;
mod marks;
mod selection;
mod segments;
mod widgets;
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    labels::register(m)?;
    widgets::register(m)?;
    geo::register(m)?;
    selection::register(m)?;
    segments::register(m)?;
    marks::register(m)
}
