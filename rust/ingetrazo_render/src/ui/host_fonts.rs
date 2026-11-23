//! On Windows and Linux use the same font database and shaper as text geometry.
use pyo3::prelude::*;

fn with_gui<T>(f: impl FnOnce(&Bound<'_, PyModule>) -> PyResult<T>) -> Option<T> {
    Python::try_attach(|py| {
        let gui = py.import("PySide6.QtGui").ok()?;
        let app = gui
            .getattr("QGuiApplication")
            .ok()?
            .call_method0("instance")
            .ok()?;
        if app.is_none() {
            return None;
        }
        f(&gui).ok()
    })
    .flatten()
}

pub fn families() -> Vec<String> {
    with_gui(|gui| {
        gui.getattr("QFontDatabase")?
            .call_method0("families")?
            .extract()
    })
    .unwrap_or_default()
}

pub fn sample(
    name: &str,
    text: &str,
    bold: bool,
    italic: bool,
    size: f32,
    scale: f32,
    extent: egui::Vec2,
) -> Option<egui::FontImage> {
    let width = (extent.x * scale).ceil().clamp(1., 4096.) as usize;
    let height = (extent.y * scale).ceil().clamp(1., 512.) as usize;
    with_gui(|gui| {
        let font = gui.getattr("QFont")?.call1((name,))?;
        font.call_method1("setPixelSize", ((size * scale).round().max(1.) as i32,))?;
        font.call_method1("setBold", (bold,))?;
        font.call_method1("setItalic", (italic,))?;
        let image_type = gui.getattr("QImage")?;
        let image = image_type.call1((width, height, image_type.getattr("Format_Grayscale8")?))?;
        image.call_method1("fill", (0,))?;
        let painter = gui.getattr("QPainter")?.call1((&image,))?;
        painter.call_method1("setFont", (&font,))?;
        painter.call_method1("setPen", (gui.getattr("QColor")?.call1(("white",))?,))?;
        let metrics = gui.getattr("QFontMetrics")?.call1((&font,))?;
        let ascent: i32 = metrics.call_method0("ascent")?.extract()?;
        let line_height: i32 = metrics.call_method0("height")?.extract()?;
        painter.call_method1(
            "drawText",
            (0, (height as i32 - line_height) / 2 + ascent, text),
        )?;
        painter.call_method0("end")?;
        let stride: usize = image.call_method0("bytesPerLine")?.extract()?;
        let bytes: Vec<u8> = image
            .call_method0("constBits")?
            .call_method0("tobytes")?
            .extract()?;
        Ok(egui::FontImage {
            size: [width, height],
            pixels: bytes
                .chunks_exact(stride)
                .flat_map(|row| row[..width].iter().map(|p| f32::from(*p) / 255.))
                .collect(),
        })
    })
}
