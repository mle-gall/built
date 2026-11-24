//! The capture target for projected document annotations. Live tools share
//! their projection and occlusion code with export; only the capture's pens,
//! shapes, text and source-over composition are handled here.
use crate::image_assets::{fonts, svg_tree};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use resvg::{
    tiny_skia::{
        FillRule, IntSize, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect,
        Stroke, StrokeDash, Transform,
    },
    usvg::fontdb,
};
type Obj<'a> = Bound<'a, PyAny>;

fn scalar(obj: &Obj<'_>, field: &str) -> PyResult<f64> {
    obj.call_method0(field)?.extract()
}
fn point(obj: &Obj<'_>) -> PyResult<(f64, f64)> {
    Ok((scalar(obj, "x")?, scalar(obj, "y")?))
}

fn rectangle(rect: &Obj<'_>) -> PyResult<Option<Rect>> {
    Ok(Rect::from_xywh(
        scalar(rect, "x")? as f32,
        scalar(rect, "y")? as f32,
        scalar(rect, "width")? as f32,
        scalar(rect, "height")? as f32,
    ))
}

fn polygon(points: &Obj<'_>) -> PyResult<PathBuilder> {
    let mut path = PathBuilder::new();
    for (i, p) in points.try_iter()?.enumerate() {
        let (x, y) = point(&p?)?;
        if i == 0 {
            path.move_to(x as f32, y as f32);
        } else {
            path.line_to(x as f32, y as f32);
        }
    }
    Ok(path)
}

fn xml(s: &str) -> String {
    s.chars()
        .filter(|&c| c >= ' ' || matches!(c, '\r' | '\n' | '\t'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[pyclass]
#[derive(Clone)]
pub struct AnnotationFont {
    family: String,
    size: f64,
    bold: bool,
    italic: bool,
}

impl Default for AnnotationFont {
    fn default() -> Self {
        Self {
            family: String::new(),
            size: 12.,
            bold: false,
            italic: false,
        }
    }
}

impl AnnotationFont {
    pub(super) fn outline_face(family: &str, bold: bool, italic: bool) -> Option<fontdb::ID> {
        Self { family: family.to_owned(), bold, italic, ..Self::default() }.face()
    }
    fn face(&self) -> Option<fontdb::ID> {
        let families = [
            fontdb::Family::Name(&self.family),
            fontdb::Family::Name(if cfg!(target_os = "macos") {
                "Helvetica Neue"
            } else if cfg!(windows) {
                "Segoe UI"
            } else {
                "DejaVu Sans"
            }),
            fontdb::Family::SansSerif,
        ];
        fonts()
            .query(&fontdb::Query {
                families: &families,
                weight: if self.bold {
                    fontdb::Weight::BOLD
                } else {
                    fontdb::Weight::NORMAL
                },
                style: if self.italic {
                    fontdb::Style::Italic
                } else {
                    fontdb::Style::Normal
                },
                ..Default::default()
            })
            .or_else(|| fonts().faces().next().map(|f| f.id))
    }
    fn text_tree(
        &self,
        text: &str,
        at: (f64, f64),
        extent: (f64, f64),
        anchor: &str,
        ink: Color,
    ) -> Option<resvg::usvg::Tree> {
        let (x, y) = at;
        let family = self
            .face()
            .and_then(|id| fonts().face(id))
            .and_then(|f| f.families.first())
            .map_or("sans-serif", |f| f.0.as_str());
        let fragment = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}"><text xml:space="preserve" x="{x}" y="{y}" text-anchor="{anchor}" font-family="{}" font-size="{}" font-weight="{}" font-style="{}" {}>{}</text></svg>"#,
            extent.0,
            extent.1,
            xml(family),
            self.size,
            if self.bold { "bold" } else { "normal" },
            if self.italic { "italic" } else { "normal" },
            ink.style("fill"),
            xml(text)
        );
        svg_tree(fragment.as_bytes(), None)
    }

    #[cfg(any(not(target_os = "macos"), test))]
    pub fn sample(
        name: &str,
        text: &str,
        bold: bool,
        italic: bool,
        size: f32,
        scale: f32,
        extent: egui::Vec2,
    ) -> Option<egui::FontImage> {
        let w = (extent.x * scale).ceil().clamp(1., 4096.) as u32;
        let h = (extent.y * scale).ceil().clamp(1., 512.) as u32;
        let font = Self {
            family: name.into(),
            size: (size * scale).into(),
            bold,
            italic,
        };
        let (_, a, d) = font.measure(None);
        let tree = font.text_tree(
            text,
            (0., (f64::from(h) + a - d) / 2.),
            (w.into(), h.into()),
            "start",
            Color([255; 4]),
        )?;
        let mut mask = Pixmap::new(w, h)?;
        resvg::render(&tree, Transform::identity(), &mut mask.as_mut());
        Some(egui::FontImage {
            size: [w as usize, h as usize],
            pixels: mask
                .data()
                .chunks_exact(4)
                .map(|p| f32::from(p[3]) / 255.)
                .collect(),
        })
    }
    fn measure(&self, text: Option<&str>) -> (f64, f64, f64) {
        let metrics = self.face().and_then(|id| {
            fonts()
                .with_face_data(id, |data, index| {
                    let face = rustybuzz::Face::from_slice(data, index)?;
                    let scale = self.size / f64::from(face.units_per_em());
                    let advance = text.map_or(0., |text| {
                        let mut buf = rustybuzz::UnicodeBuffer::new();
                        buf.push_str(text);
                        buf.guess_segment_properties();
                        rustybuzz::shape(&face, &[], buf)
                            .glyph_positions()
                            .iter()
                            .map(|p| f64::from(p.x_advance))
                            .sum::<f64>()
                            * scale
                    });
                    Some((
                        advance,
                        f64::from(face.ascender()) * scale,
                        -f64::from(face.descender()) * scale,
                    ))
                })
                .flatten()
        });
        metrics.unwrap_or((0., self.size * 0.8, self.size * 0.2))
    }
}

#[pymethods]
#[allow(non_snake_case)]
impl AnnotationFont {
    #[new]
    #[pyo3(signature=(family=""))]
    fn new(family: &str) -> Self {
        Self {
            family: family.into(),
            ..Self::default()
        }
    }
    fn setPointSize(&mut self, points: i32) {
        self.setPointSizeF(points.into());
    }
    fn setItalic(&mut self, italic: bool) {
        self.italic = italic;
    }
    fn descent(&self) -> i32 {
        self.measure(None).2.ceil() as i32
    }
    fn setPointSizeF(&mut self, points: f64) {
        self.size = points * 96. / 72.;
    }
    fn setBold(&mut self, bold: bool) {
        self.bold = bold;
    }
    fn horizontalAdvance(&self, text: &str) -> i32 {
        self.measure(Some(text)).0.round() as i32
    }
    fn ascent(&self) -> i32 {
        self.measure(None).1.ceil() as i32
    }
    fn height(&self) -> i32 {
        let (_, a, d) = self.measure(None);
        (a.ceil() + d.ceil()) as i32
    }
}

#[pyclass(name = "PaintColor", frozen, skip_from_py_object)]
#[derive(Clone, Copy)]
pub struct Color([u8; 4]);

#[pymethods]
#[allow(non_snake_case)]
impl Color {
    #[new]
    #[pyo3(signature=(r,g=None,b=None,a=255))]
    fn new(r: &Obj<'_>, g: Option<u8>, b: Option<u8>, a: u8) -> PyResult<Self> {
        if let (Some(g), Some(b)) = (g, b) {
            return Ok(Self([r.extract()?, g, b, a]));
        }
        if let Ok(s) = r.extract::<String>() {
            if let Some(hex) = s.strip_prefix('#').filter(|v| v.len() == 8) {
                if let Ok(argb) = u32::from_str_radix(hex, 16) {
                    let [a, r, g, b] = argb.to_be_bytes();
                    return Ok(Self([r, g, b, a]));
                }
            }
            let c = s
                .parse::<svgtypes::Color>()
                .map_err(|_| PyValueError::new_err("invalid paint color"))?;
            return Ok(Self([c.red, c.green, c.blue, c.alpha]));
        }
        Self::read(r)
    }
    #[staticmethod]
    #[pyo3(signature=(r,g,b,a=1.0))]
    fn fromRgbF(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self([r, g, b, a].map(|v| (v.clamp(0., 1.) * 255.).round() as u8))
    }
    fn getRgb(&self) -> (u8, u8, u8, u8) {
        self.0.into()
    }
    fn getRgbF(&self) -> (f64, f64, f64, f64) {
        self.0.map(|v| f64::from(v) / 255.).into()
    }
}

#[pyclass(name = "PaintPen")]
pub struct NativePen {
    color: Color,
    width: f32,
    cap: LineCap,
    join: LineJoin,
    dash: Vec<f32>,
}
#[pymethods]
#[allow(non_snake_case)]
impl NativePen {
    #[new]
    #[pyo3(signature=(color,width=1.,style=1))]
    fn new(color: &Obj<'_>, width: f32, style: u32) -> PyResult<Self> {
        let dash = match style {
            2 => vec![4., 2.],
            3 => vec![1., 2.],
            4 => vec![4., 2., 1., 2.],
            5 => vec![4., 2., 1., 2., 1., 2.],
            _ => Vec::new(),
        };
        Ok(Self {
            color: Color::read(color)?,
            width,
            cap: LineCap::Square,
            join: LineJoin::Bevel,
            dash,
        })
    }
    fn setColor(&mut self, color: &Obj<'_>) -> PyResult<()> {
        self.color = Color::read(color)?;
        Ok(())
    }
    fn setDashPattern(&mut self, dash: Vec<f32>) {
        self.dash = dash;
    }
    fn setCapStyle(&mut self, code: u32) {
        self.cap = match code {
            0 => LineCap::Butt,
            32 => LineCap::Round,
            _ => LineCap::Square,
        };
    }
    fn setJoinStyle(&mut self, code: u32) {
        self.join = match code {
            0 => LineJoin::Miter,
            128 => LineJoin::Round,
            _ => LineJoin::Bevel,
        };
    }
}
impl Color {
    fn read(obj: &Obj<'_>) -> PyResult<Self> {
        if let Ok(color) = obj.cast::<Self>() {
            return Ok(*color.borrow());
        }
        Ok(Self(obj.call_method0("getRgb")?.extract()?))
    }
    fn paint(self) -> Paint<'static> {
        let [r, g, b, a] = self.0;
        let mut paint = Paint::default();
        paint.set_color_rgba8(r, g, b, a);
        paint
    }
    fn style(self, attr: &str) -> String {
        let [r, g, b, a] = self.0;
        format!(
            r##"{attr}="#{r:02x}{g:02x}{b:02x}" {attr}-opacity="{}""##,
            f64::from(a) / 255.
        )
    }
}
#[derive(Clone)]
struct Style {
    pen: Option<Color>,
    stroke: Stroke,
    brush: Option<Color>,
    font: AnnotationFont,
}
impl Default for Style {
    fn default() -> Self {
        let ink = Color([0, 0, 0, 255]);
        Self {
            pen: Some(ink),
            stroke: Stroke::default(),
            brush: None,
            font: Default::default(),
        }
    }
}

fn demultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let a = u32::from(pixel[3]);
        if a != 0 && a != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

#[pyclass]
pub struct AnnotationCanvas {
    size: IntSize,
    width: f64,
    height: f64,
    state: Style,
    stack: Vec<Style>,
    overlay: Option<Pixmap>,
    /// Physical bounds of everything drawn, `[x0, y0, x1, y1]`, padded for
    /// anti-aliasing: the part of the layer the renderer uploads.
    drawn: Option<[f32; 4]>,
}

impl AnnotationCanvas {
    pub(crate) fn size(&self) -> (u32, u32) {
        (self.size.width(), self.size.height())
    }

    fn mark(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if !(x0 <= x1 && y0 <= y1) {
            return;
        }
        let r = [x0.floor() - 1., y0.floor() - 1., x1.ceil() + 1., y1.ceil() + 1.];
        self.drawn = Some(match self.drawn {
            Some(d) => [d[0].min(r[0]), d[1].min(r[1]), d[2].max(r[2]), d[3].max(r[3])],
            None => r,
        });
    }

    fn mark_logical(&mut self, rect: Rect, pad: f32) {
        let k = self.size.width() as f32 / self.width as f32;
        self.mark(rect.left() * k - pad, rect.top() * k - pad, rect.right() * k + pad, rect.bottom() * k + pad);
    }

    /// The drawn part of the layer in whole physical pixels, `[x0, y0, x1, y1)`.
    pub(crate) fn drawn_region(&self) -> Option<[u32; 4]> {
        self.overlay.as_ref()?;
        let (w, h) = (self.size.width() as f32, self.size.height() as f32);
        let d = self.drawn?;
        let r = [d[0].clamp(0., w) as u32, d[1].clamp(0., h) as u32, d[2].clamp(0., w) as u32, d[3].clamp(0., h) as u32];
        (r[0] < r[2] && r[1] < r[3]).then_some(r)
    }

    /// Straight-alpha RGBA of `[x0, y0, x1, y1)`, zero where nothing was drawn.
    pub(crate) fn region_rgba(&self, r: [u32; 4]) -> Vec<u8> {
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        let mut out = vec![0u8; (w * h * 4) as usize];
        let Some(layer) = &self.overlay else { return out };
        let stride = self.size.width() as usize * 4;
        for (row, y) in (r[1]..r[3]).enumerate() {
            let at = y as usize * stride + r[0] as usize * 4;
            let dst = &mut out[row * w as usize * 4..(row + 1) * w as usize * 4];
            dst.copy_from_slice(&layer.data()[at..at + w as usize * 4]);
            demultiply(dst);
        }
        out
    }

    fn scale(&self) -> Transform {
        let factor = self.size.width() as f32 / self.width as f32;
        Transform::from_scale(factor, factor)
    }

    fn target(&mut self) -> PyResult<&mut Pixmap> {
        if self.overlay.is_none() {
            self.overlay = Pixmap::new(self.size.width(), self.size.height());
        }
        self.overlay
            .as_mut()
            .ok_or_else(|| PyValueError::new_err("invalid annotation target"))
    }

    fn path(&mut self, path: Option<Path>, fill: bool) -> PyResult<()> {
        let Some(path) = path else {
            return Ok(());
        };
        let transform = self.scale();
        let pen = self.state.pen;
        let brush = self.state.brush;
        let stroke = self.state.stroke.clone();
        if pen.is_some() || (fill && brush.is_some()) {
            // A stroke reaches half its width out, a miter join up to twice it.
            let pad = if pen.is_some() { 2. * stroke.width.max(1.) * transform.sx } else { 0. };
            self.mark_logical(path.bounds(), pad);
        }
        let target = self.target()?;
        if let Some(color) = brush.filter(|_| fill) {
            target.fill_path(&path, &color.paint(), FillRule::EvenOdd, transform, None);
        }
        if let Some(color) = pen {
            target.stroke_path(&path, &color.paint(), &stroke, transform, None);
        }
        Ok(())
    }
}

#[pymethods]
#[allow(non_snake_case)]
impl AnnotationCanvas {
    #[new]
    fn new(w: u32, h: u32, logical_width: f64) -> PyResult<Self> {
        let size = IntSize::from_wh(w, h)
            .ok_or_else(|| PyValueError::new_err("invalid capture dimensions"))?;
        if !logical_width.is_finite() || logical_width <= 0. {
            return Err(PyValueError::new_err("invalid logical width"));
        }
        Ok(Self {
            size,
            width: logical_width,
            height: logical_width * f64::from(h) / f64::from(w),
            state: Default::default(),
            stack: vec![],
            overlay: None,
            drawn: None,
        })
    }
    #[getter]
    fn logical_size(&self) -> (f64, f64) {
        (self.width, self.height)
    }
    fn device(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn width(&self) -> u32 {
        self.size.width()
    }
    fn height(&self) -> u32 {
        self.size.height()
    }
    fn devicePixelRatioF(&self) -> f64 {
        f64::from(self.size.width()) / self.width
    }
    fn save(&mut self) {
        self.stack.push(self.state.clone());
    }
    fn restore(&mut self) {
        if let Some(style) = self.stack.pop() {
            self.state = style;
        }
    }
    fn font(&self) -> AnnotationFont {
        self.state.font.clone()
    }
    fn fontMetrics(&self) -> AnnotationFont {
        self.state.font.clone()
    }
    fn setFont(&mut self, font: &Obj<'_>) -> PyResult<()> {
        if let Ok(f) = font.extract::<PyRef<'_, AnnotationFont>>() {
            self.state.font = (*f).clone();
        } else {
            let px = scalar(font, "pixelSize")?;
            self.state.font = AnnotationFont {
                family: font.call_method0("family")?.extract()?,
                size: if px > 0. {
                    px
                } else {
                    (scalar(font, "pointSizeF")? * 96. / 72.).max(1.)
                },
                bold: font.call_method0("bold")?.extract()?,
                italic: font.call_method0("italic")?.extract()?,
            };
        }
        Ok(())
    }
    fn setBrush(&mut self, brush: &Obj<'_>) -> PyResult<()> {
        self.state.brush = if brush.hasattr("getRgb")? {
            Some(Color::read(brush)?)
        } else {
            None
        };
        Ok(())
    }
    fn setPen(&mut self, pen: &Obj<'_>) -> PyResult<()> {
        if let Ok(pen) = pen.cast::<NativePen>() {
            let p = pen.borrow();
            self.state.pen = Some(p.color);
            self.state.stroke = Stroke {
                width: p.width,
                line_cap: p.cap,
                line_join: p.join,
                dash: StrokeDash::new(p.dash.iter().map(|n| n * p.width).collect(), 0.),
                ..Default::default()
            };
            return Ok(());
        }
        self.state.stroke = Stroke::default();
        if pen.hasattr("widthF")? {
            let width = scalar(pen, "widthF")?.max(0.01) as f32;
            let cap: String = pen.call_method0("capStyle")?.getattr("name")?.extract()?;
            let line_cap = match cap.as_str() {
                "FlatCap" => LineCap::Butt,
                "RoundCap" => LineCap::Round,
                _ => LineCap::Square,
            };
            let dash = pen
                .call_method0("dashPattern")?
                .extract::<Vec<f32>>()?
                .into_iter()
                .map(|n| n * width)
                .collect();
            self.state.stroke = Stroke {
                width,
                line_cap,
                line_join: LineJoin::Bevel,
                dash: StrokeDash::new(dash, 0.),
                ..Default::default()
            };
            self.state.pen = Some(Color::read(&pen.call_method0("color")?)?);
        } else {
            self.state.pen = if pen.hasattr("getRgb")? {
                Some(Color::read(pen)?)
            } else {
                None
            };
        }
        Ok(())
    }
    fn drawLine(&mut self, a: &Obj<'_>, b: &Obj<'_>) -> PyResult<()> {
        let (x1, y1) = point(a)?;
        let (x2, y2) = point(b)?;
        let mut path = PathBuilder::new();
        path.move_to(x1 as f32, y1 as f32);
        path.line_to(x2 as f32, y2 as f32);
        self.path(path.finish(), false)
    }
    fn drawPolygon(&mut self, points: &Obj<'_>) -> PyResult<()> {
        let mut path = polygon(points)?;
        path.close();
        self.path(path.finish(), true)
    }
    fn drawPolyline(&mut self, points: &Obj<'_>) -> PyResult<()> {
        self.path(polygon(points)?.finish(), false)
    }
    fn composite_plugin(&mut self, data: &Bound<'_, PyBytes>) -> PyResult<()> {
        let (w, h) = self.size();
        let image = resvg::tiny_skia::PixmapRef::from_bytes(data.as_bytes(), w, h)
            .ok_or_else(|| PyValueError::new_err("invalid plugin overlay pixels"))?;
        self.mark(0., 0., w as f32, h as f32);
        self.target()?.draw_pixmap(
            0,
            0,
            image,
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
        Ok(())
    }
    fn drawRect(&mut self, rect: &Obj<'_>) -> PyResult<()> {
        self.path(rectangle(rect)?.map(PathBuilder::from_rect), true)
    }
    fn draw_tool(&mut self, kind: &str, tool: &Obj<'_>, view: &Obj<'_>) -> PyResult<()> {
        let (w, h) = self.size();
        if let Some((x, y, tile)) = crate::tool_overlay::render_tile(
            kind,
            tool,
            view,
            w,
            h,
            self.devicePixelRatioF() as f32,
        )? {
            self.mark(x as f32, y as f32, x as f32 + tile.width() as f32, y as f32 + tile.height() as f32);
            self.target()?.draw_pixmap(
                x as i32,
                y as i32,
                tile.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
        Ok(())
    }
    fn drawEllipse(&mut self, center: &Obj<'_>, rx: f64, ry: f64) -> PyResult<()> {
        let (x, y) = point(center)?;
        let rect = Rect::from_xywh(
            (x - rx) as f32,
            (y - ry) as f32,
            (2. * rx) as f32,
            (2. * ry) as f32,
        );
        self.path(rect.and_then(PathBuilder::from_oval), true)
    }
    #[pyo3(signature=(*args))]
    fn fillRect(&mut self, args: &Bound<'_, pyo3::types::PyTuple>) -> PyResult<()> {
        let (rect, color) = match args.len() {
            2 => (rectangle(&args.get_item(0)?)?, args.get_item(1)?),
            5 => (
                Rect::from_xywh(
                    args.get_item(0)?.extract()?,
                    args.get_item(1)?.extract()?,
                    args.get_item(2)?.extract()?,
                    args.get_item(3)?.extract()?,
                ),
                args.get_item(4)?,
            ),
            _ => return Err(PyValueError::new_err("expected a rectangle and a color")),
        };
        if let Some(rect) = rect {
            let transform = self.scale();
            self.mark_logical(rect, 0.);
            self.target()?
                .fill_rect(rect, &Color::read(&color)?.paint(), transform, None);
        }
        Ok(())
    }
    #[pyo3(signature=(pos, value, centered_text=None))]
    fn drawText(
        &mut self,
        pos: &Obj<'_>,
        value: &Obj<'_>,
        centered_text: Option<&str>,
    ) -> PyResult<()> {
        let Some(ink) = self.state.pen else {
            return Ok(());
        };
        let font = &self.state.font;
        let (mut x, mut y) = point(pos)?;
        let (text, anchor) = if let Some(s) = centered_text {
            let (_, a, d) = font.measure(None);
            x += scalar(pos, "width")? / 2.;
            y += (scalar(pos, "height")? + a - d) / 2.;
            (s, "middle")
        } else {
            (value.cast::<pyo3::types::PyString>()?.to_str()?, "start")
        };
        let tree = font
            .text_tree(text, (x, y), (self.width, self.height), anchor, ink)
            .ok_or_else(|| PyValueError::new_err("invalid annotation text"))?;
        let transform = self.scale();
        let ink = tree.root().abs_stroke_bounding_box();
        self.mark_logical(ink, 2.);
        resvg::render(&tree, transform, &mut self.target()?.as_mut());
        Ok(())
    }
    fn composite<'py>(
        &self,
        py: Python<'py>,
        background: &Bound<'py, PyBytes>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let src = background.as_bytes();
        let pixels = u64::from(self.size.width()) * u64::from(self.size.height());
        if pixels.checked_mul(4) != Some(src.len() as u64) {
            return Err(PyValueError::new_err("invalid capture pixel length"));
        }
        let Some(layer) = &self.overlay else {
            return Ok(background.clone());
        };
        // The layer is premultiplied. Compose directly over straight RGBA,
        // preserving all background bytes where no annotation was drawn.
        PyBytes::new_with(py, src.len(), |out| {
            out.copy_from_slice(src);
            py.detach(|| {
                for (dst, mark) in out.chunks_exact_mut(4).zip(layer.data().chunks_exact(4)) {
                    let a = u32::from(mark[3]);
                    if a == 0 {
                        continue;
                    }
                    let retained = u32::from(dst[3]) * (255 - a);
                    let alpha = a * 255 + retained;
                    for k in 0..3 {
                        let c = u32::from(mark[k]) * 255 * 255 + u32::from(dst[k]) * retained;
                        dst[k] = ((c + alpha / 2) / alpha).min(255) as u8;
                    }
                    dst[3] = ((alpha + 127) / 255) as u8;
                }
            });
            Ok(())
        })
    }
}
