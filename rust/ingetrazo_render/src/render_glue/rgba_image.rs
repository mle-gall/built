//! A straight-alpha RGBA image owned by Rust: what the renderer's captures
//! and the overlay raster hand Python instead of a QImage (the Qt import
//! inventory's "Rust image buffers and codecs"). It saves and encodes PNG
//! and JPEG and answers the pixel reads tests and plugins make.
use image::{codecs::jpeg::JpegEncoder, ImageFormat, RgbaImage as Pixels};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use std::{io::Cursor, path::PathBuf};

#[pyclass(name = "RgbaImage", module = "ingetrazo_render")]
#[derive(Clone)]
pub struct RgbaImage {
    pixels: Pixels,
    dpr: f64,
}

impl RgbaImage {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        Pixels::from_raw(width, height, rgba).map(|pixels| Self { pixels, dpr: 1. })
    }

    fn format(name: &str) -> PyResult<ImageFormat> {
        match name.to_ascii_lowercase().trim_start_matches('.') {
            "png" => Ok(ImageFormat::Png),
            "jpg" | "jpeg" => Ok(ImageFormat::Jpeg),
            other => Err(PyValueError::new_err(format!("unsupported image format {other:?}"))),
        }
    }

    fn encoded(&self, format: ImageFormat, quality: i32) -> PyResult<Vec<u8>> {
        let mut out = Cursor::new(Vec::new());
        let result = if format == ImageFormat::Jpeg {
            // JPEG has no alpha: the picture over white, as Qt wrote it.
            let rgb = image::DynamicImage::ImageRgba8(self.over_white()).to_rgb8();
            let q = if (1..=100).contains(&quality) { quality as u8 } else { 75 };
            JpegEncoder::new_with_quality(&mut out, q).encode_image(&rgb)
        } else {
            self.pixels.write_to(&mut out, format)
        };
        result.map_err(|e| PyValueError::new_err(format!("cannot encode the image: {e}")))?;
        Ok(out.into_inner())
    }

    fn over_white(&self) -> Pixels {
        let mut flat = self.pixels.clone();
        for p in flat.pixels_mut() {
            let a = u32::from(p[3]);
            for c in 0..3 {
                p[c] = ((u32::from(p[c]) * a + 255 * (255 - a) + 127) / 255) as u8;
            }
            p[3] = 255;
        }
        flat
    }
}

#[pymethods]
#[allow(non_snake_case)]
impl RgbaImage {
    /// ``RgbaImage(width, height, rgba)``: straight RGBA bytes, row by row.
    #[new]
    fn py_new(width: u32, height: u32, rgba: Vec<u8>) -> PyResult<Self> {
        Self::new(width, height, rgba).ok_or_else(|| PyValueError::new_err("pixels do not match the size"))
    }

    /// Read a PNG or JPEG file.
    #[staticmethod]
    fn load(path: PathBuf) -> PyResult<Self> {
        let pixels = image::open(&path)
            .map_err(|e| PyValueError::new_err(format!("cannot read {}: {e}", path.display())))?
            .into_rgba8();
        Ok(Self { pixels, dpr: 1. })
    }

    /// An SVG file drawn into a ``size`` × ``size`` square (resvg), centred
    /// and fitted, transparent around it.
    #[staticmethod]
    fn from_svg(path: PathBuf, size: u32) -> PyResult<Self> {
        use resvg::tiny_skia::{Pixmap, Transform};
        let data = std::fs::read(&path).map_err(|e| PyValueError::new_err(format!("cannot read {}: {e}", path.display())))?;
        let tree = crate::image_assets::svg_tree(&data, path.parent())
            .ok_or_else(|| PyValueError::new_err(format!("{} is not a valid SVG", path.display())))?;
        let mut pixmap = Pixmap::new(size, size).ok_or_else(|| PyValueError::new_err("empty size"))?;
        let (w, h) = (tree.size().width(), tree.size().height());
        let k = size as f32 / w.max(h);
        let t = Transform::from_row(k, 0., 0., k, (size as f32 - w * k) / 2., (size as f32 - h * k) / 2.);
        resvg::render(&tree, t, &mut pixmap.as_mut());
        let mut pixels = Pixels::from_raw(size, size, pixmap.take()).ok_or_else(|| PyValueError::new_err("bad raster"))?;
        crate::image_assets::unpremultiply(&mut pixels);
        Ok(Self { pixels, dpr: 1. })
    }

    fn width(&self) -> u32 {
        self.pixels.width()
    }
    fn height(&self) -> u32 {
        self.pixels.height()
    }
    fn isNull(&self) -> bool {
        self.pixels.width() == 0 || self.pixels.height() == 0
    }
    fn devicePixelRatio(&self) -> f64 {
        self.dpr
    }
    fn setDevicePixelRatio(&mut self, dpr: f64) {
        self.dpr = dpr;
    }
    fn copy(&self) -> Self {
        self.clone()
    }

    /// Whether any pixel is less than opaque.
    fn hasAlphaChannel(&self) -> bool {
        self.pixels.pixels().any(|p| p[3] < 255)
    }

    /// A copy resampled to ``width`` × ``height`` (smooth: Catmull-Rom on
    /// premultiplied colour, so transparent texels do not darken edges).
    fn scaled(&self, width: u32, height: u32) -> PyResult<Self> {
        if width == 0 || height == 0 {
            return Err(PyValueError::new_err("empty size"));
        }
        let mut pre = self.pixels.clone();
        crate::image_assets::premultiply(&mut pre);
        let mut out = image::imageops::resize(&pre, width, height, image::imageops::FilterType::CatmullRom);
        crate::image_assets::unpremultiply(&mut out);
        Ok(Self { pixels: out, dpr: self.dpr })
    }

    /// ``(r, g, b, a)`` at ``(x, y)``.
    fn pixel(&self, x: u32, y: u32) -> PyResult<(u8, u8, u8, u8)> {
        self.pixels
            .get_pixel_checked(x, y)
            .map(|p| (p[0], p[1], p[2], p[3]))
            .ok_or_else(|| PyValueError::new_err("pixel outside the image"))
    }

    /// The straight RGBA bytes, row by row.
    fn rgba<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.pixels.as_raw())
    }

    /// The image as PNG or JPEG bytes (``quality`` 1-100 for JPEG).
    #[pyo3(signature = (format="PNG", quality=-1))]
    fn encode<'py>(&self, py: Python<'py>, format: &str, quality: i32) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &self.encoded(Self::format(format)?, quality)?))
    }

    /// Write to ``path``; the format from ``format`` or the extension.
    /// False when it cannot be written, as QImage.save answered.
    #[pyo3(signature = (path, format=None, quality=-1))]
    fn save(&self, path: PathBuf, format: Option<&str>, quality: i32) -> bool {
        let format = match format {
            Some(f) => Self::format(f),
            None => Self::format(path.extension().and_then(|e| e.to_str()).unwrap_or("png")),
        };
        let Ok(format) = format else { return false };
        self.encoded(format, quality).is_ok_and(|bytes| std::fs::write(&path, bytes).is_ok())
    }

    fn __repr__(&self) -> String {
        format!("RgbaImage({} x {})", self.pixels.width(), self.pixels.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_flattens_jpeg_over_white() {
        let img = RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 0, 0, 0]).unwrap();
        assert_eq!(img.over_white().get_pixel(1, 0).0, [255, 255, 255, 255]);
        let png = img.encoded(ImageFormat::Png, -1).unwrap();
        let back = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(back.as_raw(), img.pixels.as_raw());
        assert!(!img.encoded(ImageFormat::Jpeg, 72).unwrap().is_empty());
        assert!(RgbaImage::new(2, 2, vec![0; 3]).is_none());
    }
}
