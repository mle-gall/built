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
            return parse_color(&s);
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
/// A colour as Qt names one: `#rrggbb`, `#aarrggbb` or a CSS name.
fn parse_color(s: &str) -> PyResult<Color> {
    if let Some(hex) = s.strip_prefix('#').filter(|v| v.len() == 8) {
        if let Ok(argb) = u32::from_str_radix(hex, 16) {
            let [a, r, g, b] = argb.to_be_bytes();
            return Ok(Color([r, g, b, a]));
        }
    }
    let c = s.parse::<svgtypes::Color>().map_err(|_| PyValueError::new_err("invalid paint color"))?;
    Ok(Color([c.red, c.green, c.blue, c.alpha]))
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

/// A line of text laid out at the origin, kept by its font, words, anchor
/// and ink: a marker's label is the same from frame to frame and only its
/// place changes, and the layout (font lookup, shaping, outlines) was most
/// of a label's cost. Bounded; a full cache starts again.
fn text_run(font: &AnnotationFont, text: &str, anchor: &str, ink: Color) -> Option<std::rc::Rc<resvg::usvg::Tree>> {
    use std::{cell::RefCell, collections::HashMap, rc::Rc};
    type Key = (String, String, u64, bool, bool, String, [u8; 4]);
    thread_local! {
        static RUNS: RefCell<HashMap<Key, Rc<resvg::usvg::Tree>>> = RefCell::new(HashMap::new());
    }
    let key: Key = (text.to_owned(), font.family.clone(), font.size.to_bits(), font.bold, font.italic,
                    anchor.to_owned(), ink.0);
    if let Some(tree) = RUNS.with(|runs| runs.borrow().get(&key).cloned()) {
        return Some(tree);
    }
    // The extent only sizes the fragment's viewport; it does not clip the run.
    let tree = Rc::new(font.text_tree(text, (0., 0.), (1., 1.), anchor, ink)?);
    RUNS.with(|runs| {
        let mut runs = runs.borrow_mut();
        if runs.len() >= 512 {
            runs.clear();
        }
        runs.insert(key, tree.clone());
    });
    Some(tree)
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

    fn text(&mut self, text: &str, at: (f64, f64), anchor: &str, ink: Color) -> PyResult<()> {
        let tree = text_run(&self.state.font, text, anchor, ink)
            .ok_or_else(|| PyValueError::new_err("invalid annotation text"))?;
        // Placed in device pixels, after the scale: a whole-pixel move then
        // draws the same pixels.
        let scale = self.scale();
        let transform = scale.post_translate(at.0 as f32 * scale.sx, at.1 as f32 * scale.sy);
        let b = tree.root().abs_stroke_bounding_box();
        let (x, y) = (at.0 as f32, at.1 as f32);
        if let Some(bounds) = Rect::from_ltrb(b.left() + x, b.top() + y, b.right() + x, b.bottom() + y) {
            self.mark_logical(bounds, 2.);
        }
        resvg::render(&tree, transform, &mut self.target()?.as_mut());
        Ok(())
    }

    /// A solid or dashed pen as `QPen(color, width[, Qt.DashLine])` sets it.
    fn pen(&mut self, color: Color, width: f32, dashed: bool) {
        self.state.pen = Some(color);
        self.state.stroke = Stroke {
            width,
            line_cap: LineCap::Square,
            line_join: LineJoin::Bevel,
            dash: if dashed { StrokeDash::new(vec![4. * width, 2. * width], 0.) } else { None },
            ..Default::default()
        };
    }

    fn line(&mut self, a: (f64, f64), b: (f64, f64)) -> PyResult<()> {
        let mut path = PathBuilder::new();
        path.move_to(a.0 as f32, a.1 as f32);
        path.line_to(b.0 as f32, b.1 as f32);
        self.path(path.finish(), false)
    }

    fn circle(&mut self, (x, y): (f64, f64), r: f64) -> PyResult<()> {
        let rect = Rect::from_xywh((x - r) as f32, (y - r) as f32, (2. * r) as f32, (2. * r) as f32);
        self.path(rect.and_then(PathBuilder::from_oval), true)
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
        self.text(text, (x, y), anchor, ink)
    }
    /// The tool overlay's logical-pixel primitives (formerly the loop in
    /// views/wgpu_overlay.paint_layer), in order. Each is ``(kind, points,
    /// color, width, text, fill, dash, pen_style, snap_kind)``: kind is
    /// polyline, polygon, rectangle (two corners), point (a disc of radius
    /// ``width``), text or snap (a marker without its label); ``dash`` is
    /// a custom pattern in pen widths, drawn with flat caps; ``pen_style``
    /// the Qt pen style code.
    #[allow(clippy::type_complexity)]
    fn draw_primitives(
        &mut self,
        items: Vec<(String, Vec<(f64, f64)>, String, f64, String, Option<String>, Vec<f32>, u32, String)>,
    ) -> PyResult<()> {
        for (kind, points, color, width, text, fill, dash, style, snap_kind) in items {
            let ink = parse_color(&color)?;
            if kind == "snap" {
                let [r, g, b, _] = ink.0.map(|v| f64::from(v) / 255.);
                let at = *points.first().ok_or_else(|| PyValueError::new_err("a snap needs a point"))?;
                self.draw_snap(&snap_kind, at, (r, g, b), Vec::new(), None)?;
                continue;
            }
            let pattern: Vec<f32> = if dash.is_empty() {
                match style {
                    2 => vec![4., 2.],
                    3 => vec![1., 2.],
                    4 => vec![4., 2., 1., 2.],
                    5 => vec![4., 2., 1., 2., 1., 2.],
                    _ => Vec::new(),
                }
            } else {
                dash.clone()
            };
            self.state.pen = Some(ink);
            let pen_width = width as f32;
            self.state.stroke = Stroke {
                width: pen_width,
                line_cap: if dash.is_empty() { LineCap::Square } else { LineCap::Butt },
                line_join: LineJoin::Bevel,
                dash: StrokeDash::new(pattern.iter().map(|n| n * pen_width).collect(), 0.),
                ..Default::default()
            };
            self.state.brush = fill.as_deref().map(parse_color).transpose()?;
            let line = |closed: bool| {
                let mut path = PathBuilder::new();
                for (i, &(x, y)) in points.iter().enumerate() {
                    if i == 0 {
                        path.move_to(x as f32, y as f32);
                    } else {
                        path.line_to(x as f32, y as f32);
                    }
                }
                if closed {
                    path.close();
                }
                path.finish()
            };
            match kind.as_str() {
                "polyline" => self.path(line(false), false)?,
                "polygon" => self.path(line(true), true)?,
                "rectangle" => {
                    let (&(x0, y0), &(x1, y1)) = (points.first().unwrap_or(&(0., 0.)), points.get(1).unwrap_or(&(0., 0.)));
                    // As QRectF(a, b).normalized(): the corner, then the extent.
                    let rect = Rect::from_xywh(x0.min(x1) as f32, y0.min(y1) as f32, (x1 - x0).abs() as f32, (y1 - y0).abs() as f32);
                    self.path(rect.map(PathBuilder::from_rect), true)?
                }
                "point" => self.circle(*points.first().unwrap_or(&(0., 0.)), width)?,
                "text" => self.text(&text, *points.first().unwrap_or(&(0., 0.)), "start", ink)?,
                other => return Err(PyValueError::new_err(format!("Unknown overlay primitive: {other}"))),
            }
        }
        Ok(())
    }
    /// A snap's marker (formerly views/wgpu_overlay.draw_snap_indicator):
    /// the dashed guides (projected and clipped, logical pixels, with their
    /// colours), a white halo under the kind's glyph in ``rgb`` at ``at``,
    /// and the label beside it, haloed too. Axis snaps draw nothing.
    #[pyo3(signature=(kind, at, rgb, guides=Vec::new(), label=None))]
    fn draw_snap(
        &mut self,
        kind: &str,
        at: (f64, f64),
        rgb: (f64, f64, f64),
        guides: Vec<((f64, f64), (f64, f64), (f64, f64, f64))>,
        label: Option<&str>,
    ) -> PyResult<()> {
        if matches!(kind, "axis_inference" | "axis") {
            return Ok(());
        }
        let (r, g, b) = rgb;
        let color = Color::fromRgbF(r, g, b, 1.);
        for (a, z, (gr, gg, gb)) in guides {
            self.pen(Color::fromRgbF(gr, gg, gb, 0.9), 2., true);
            self.line(a, z)?;
        }
        let halo = Color([255, 255, 255, 230]);
        let tint = |a| Some(Color::fromRgbF(r, g, b, a));
        let (px, py) = at;
        self.state.brush = tint(0.30);
        // Halo, then the marker: (pen colour, width, brush) per pass.
        let passes = |inner: Option<Color>| [(halo, 4.5, None), (color, 2.6, inner)];
        match kind {
            "intersection" => {
                for (pen, width, _) in passes(None) {
                    self.pen(pen, width, false);
                    self.line((px - 8., py - 8.), (px + 8., py + 8.))?;
                    self.line((px - 8., py + 8.), (px + 8., py - 8.))?;
                }
            }
            "center" => {
                for (pen, width, brush) in passes(tint(0.85)) {
                    self.pen(pen, width, false);
                    self.state.brush = brush;
                    self.circle((px, py), 6.5)?;
                }
            }
            "endpoint" | "origin" | "component_origin" | "on_edge" | "on_axis" | "intersection_edge"
            | "intersection_face" | "on_line" | "extension" | "from_point" | "aligned" | "tangent" => {
                let rect = Rect::from_xywh((px - 7.) as f32, (py - 7.) as f32, 14., 14.).map(PathBuilder::from_rect);
                for (pen, width, brush) in passes(tint(0.30)) {
                    self.pen(pen, width, false);
                    self.state.brush = brush;
                    self.path(rect.clone(), true)?;
                }
            }
            "midpoint" | "arc_midpoint" => {
                let mut diamond = PathBuilder::new();
                diamond.move_to(px as f32, (py - 9.) as f32);
                diamond.line_to((px + 9.) as f32, py as f32);
                diamond.line_to(px as f32, (py + 9.) as f32);
                diamond.line_to((px - 9.) as f32, py as f32);
                diamond.close();
                let diamond = diamond.finish();
                for (pen, width, brush) in passes(tint(0.30)) {
                    self.pen(pen, width, false);
                    self.state.brush = brush;
                    self.path(diamond.clone(), true)?;
                }
            }
            "on_face" | "close" | "reference" | "through_point" | "perp_face" => {
                let radius = match kind {
                    "on_face" => 5.5,
                    "close" => 9.,
                    _ => 6.5,
                };
                // The brush stays the translucent tint under both rings.
                for (pen, width, _) in passes(None) {
                    self.pen(pen, width, false);
                    self.circle((px, py), radius)?;
                }
            }
            _ => {}
        }
        if let Some(label) = label.filter(|l| !l.is_empty()) {
            let mut font = AnnotationFont::default();
            font.size = 9. * 96. / 72.;
            self.state.font = font;
            self.pen(Color([255, 255, 255, 220]), 1., false);
            self.text(label, (px + 11., py + 17.), "start", Color([255, 255, 255, 220]))?;
            self.pen(color, 1., false);
            self.text(label, (px + 10., py + 16.), "start", color)?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drawn_region_covers_the_ink_and_nothing_outside_it_is_inked() {
        // 2x: a 200 x 100 logical canvas.
        let mut canvas = AnnotationCanvas::new(400, 200, 200.).unwrap();
        assert_eq!(canvas.drawn_region(), None);
        canvas.pen(Color([255, 0, 0, 255]), 2., false);
        canvas.line((20., 30.), (60., 30.)).unwrap();
        canvas.circle((150., 70.), 5.).unwrap();
        let r = canvas.drawn_region().unwrap();
        // The line reaches x 40..120, y 60 physical; the ring 290..310, 130..150.
        assert!(r[0] <= 38 && r[1] <= 58 && r[2] >= 312 && r[3] >= 152, "{r:?}");
        assert!(r[2] <= 400 && r[3] <= 200);
        let layer = canvas.overlay.as_ref().unwrap();
        for y in 0..200u32 {
            for x in 0..400u32 {
                let inside = x >= r[0] && x < r[2] && y >= r[1] && y < r[3];
                if !inside {
                    assert_eq!(layer.pixel(x, y).unwrap().alpha(), 0, "ink at {x},{y} outside {r:?}");
                }
            }
        }
        // The region's straight RGBA is the layer's, demultiplied.
        let rgba = canvas.region_rgba(r);
        let w = (r[2] - r[0]) as usize;
        let (x, y) = (60usize, 60usize);
        let at = ((y - r[1] as usize) * w + (x - r[0] as usize)) * 4;
        assert_eq!(&rgba[at..at + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn a_label_laid_out_once_draws_the_same_ink_wherever_it_goes() {
        let ink = Color([20, 160, 90, 255]);
        let draw = |at: (f64, f64)| {
            let mut canvas = AnnotationCanvas::new(400, 200, 200.).unwrap();
            canvas.text("On Edge 3", at, "start", ink).unwrap();
            (canvas.drawn_region().unwrap(), canvas.overlay.unwrap())
        };
        let (ra, a) = draw((10., 30.));
        let (rb, b) = draw((40., 50.)); // 60 and 40 physical pixels on
        assert_eq!((rb[0] - ra[0], rb[1] - ra[1]), (60, 40));
        assert!(a.data().iter().any(|&v| v > 0));
        for y in ra[1]..ra[3] {
            for x in ra[0]..ra[2] {
                assert_eq!(a.pixel(x, y), b.pixel(x + 60, y + 40), "at {x},{y}");
            }
        }
    }

    #[test]
    fn off_canvas_drawing_leaves_no_region() {
        let mut canvas = AnnotationCanvas::new(100, 100, 100.).unwrap();
        canvas.pen(Color([0, 0, 0, 255]), 1., false);
        canvas.line((-50., -50.), (-20., -40.)).unwrap();
        assert_eq!(canvas.drawn_region(), None);
    }
}
