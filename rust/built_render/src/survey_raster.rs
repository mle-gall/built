//! Immutable RGBA tiles shared by the map, DEM sampler and terrain renderer.
use image::{codecs::jpeg::JpegEncoder, ExtendedColorType};
use image::{imageops, ImageDecoder, ImageReader, Rgba, RgbaImage};
use numpy::{ndarray::Array2, IntoPyArray, PyArray2};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict},
};
use std::{
    fs::File,
    io::{BufReader, Cursor},
    path::Path,
};

#[pyclass(frozen, module = "built_render")]
pub struct SurveyRaster {
    pub(crate) pixels: RgbaImage,
}

const MAX_PIXELS: u64 = 128 * 1024 * 1024;
fn fits(w: u32, h: u32) -> bool {
    w != 0 && h != 0 && u64::from(w) * u64::from(h) <= MAX_PIXELS
}
#[pymethods]
impl SurveyRaster {
    #[new]
    fn new(width: u32, height: u32, rgba: &[u8]) -> PyResult<Self> {
        if !fits(width, height) || u64::from(width) * u64::from(height) * 4 != rgba.len() as u64 {
            return Err(PyValueError::new_err("Invalid survey raster dimensions"));
        }
        Ok(Self {
            pixels: RgbaImage::from_raw(width, height, rgba.to_vec()).unwrap(),
        })
    }
    #[staticmethod]
    fn decode(py: Python<'_>, data: &[u8]) -> Option<Self> {
        py.detach(|| {
            let reader = ImageReader::new(Cursor::new(data))
                .with_guessed_format()
                .ok()?;
            let decoder = reader.into_decoder().ok()?;
            let (w, h) = decoder.dimensions();
            if !fits(w, h) {
                return None;
            }
            let pixels = image::DynamicImage::from_decoder(decoder)
                .ok()?
                .into_rgba8();
            Some(Self { pixels })
        })
    }
    #[staticmethod]
    fn atlas(
        py: Python<'_>,
        path: std::path::PathBuf,
        target: (u32, u32),
    ) -> PyResult<Option<Self>> {
        if !fits(target.0, target.1) {
            return Ok(None);
        }
        if let Some(image) = py.detach(|| scaled_png(&path, target)) {
            return Ok(Some(Self { pixels: image }));
        }
        Ok(crate::image_assets::decode(py, &path)?.map(|pixels| {
            py.detach(|| {
                let (w, h) = target_size(pixels.width(), pixels.height(), target);
                Self {
                    pixels: crate::image_assets::resize(pixels, w, h),
                }
            })
        }))
    }

    #[staticmethod]
    fn open(py: Python<'_>, path: std::path::PathBuf) -> PyResult<Option<Self>> {
        Ok(crate::image_assets::decode(py, &path)?.map(|pixels| Self { pixels }))
    }

    fn resized(&self, py: Python<'_>, width: u32, height: u32) -> PyResult<Self> {
        if !fits(width, height) {
            return Err(PyValueError::new_err("invalid raster dimensions"));
        }
        Ok(Self {
            pixels: py.detach(|| crate::image_assets::resize(self.pixels.clone(), width, height)),
        })
    }

    fn material_png<'py>(&self, py: Python<'py>, source: &[u8]) -> PyResult<Bound<'py, PyBytes>> {
        crate::material_png::encode(
            py,
            self.width(),
            self.height(),
            self.pixels.as_raw(),
            source,
        )
    }

    fn frame_region(
        &self,
        py: Python<'_>,
        region: (f64, f64, f64, f64),
        width: u32,
        height: u32,
    ) -> PyResult<Self> {
        if !fits(width, height) {
            return Err(PyValueError::new_err("invalid frame dimensions"));
        }
        let (x, y, w, h) = region;
        let crop = |origin: f64, extent: f64, pixels: u32| {
            let start = (origin * f64::from(pixels))
                .round()
                .clamp(0., f64::from(pixels - 1)) as u32;
            let length = (extent * f64::from(pixels))
                .round()
                .clamp(1., f64::from(pixels - start)) as u32;
            (start, length)
        };
        let (x, w) = crop(x, w, self.width());
        let (y, h) = crop(y, h, self.height());
        let pixels = py.detach(|| {
            let part = imageops::crop_imm(&self.pixels, x, y, w, h).to_image();
            if (width, height) == (w, h) {
                part
            } else {
                crate::image_assets::resize(part, width, height)
            }
        });
        Ok(Self { pixels })
    }

    fn save(&self, py: Python<'_>, path: std::path::PathBuf) -> bool {
        py.detach(|| {
            if image::ImageFormat::from_path(&path).ok() == Some(image::ImageFormat::Jpeg) {
                let rgb: image::RgbImage = image::buffer::ConvertBuffer::convert(&self.pixels);
                rgb.save(path).is_ok()
            } else {
                self.pixels.save(path).is_ok()
            }
        })
    }

    fn png<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let data = py
            .detach(|| {
                let mut out = Vec::new();
                crate::captured_frame::encode_png(
                    &mut out,
                    self.width(),
                    self.height(),
                    self.pixels.as_raw(),
                )?;
                Ok::<_, png::EncodingError>(out)
            })
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(PyBytes::new(py, &data))
    }

    fn mean_rgb(&self, py: Python<'_>) -> (f64, f64, f64) {
        py.detach(|| {
            let mut rgb = [0u64; 3];
            let mut weight = 0;
            for pixel in self.pixels.pixels() {
                let a = u64::from(pixel[3]);
                weight += a;
                for (sum, c) in rgb.iter_mut().zip(pixel.0) {
                    *sum += u64::from(c) * a;
                }
            }
            rgb.map(|v| v as f64 / (weight.max(1) as f64 * 255.)).into()
        })
    }
    fn jpeg<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = py
            .detach(|| {
                let rgb: image::RgbImage = image::buffer::ConvertBuffer::convert(&self.pixels);
                let mut out = Vec::new();
                JpegEncoder::new_with_quality(&mut out, 92).encode(
                    rgb.as_raw(),
                    self.width(),
                    self.height(),
                    ExtendedColorType::Rgb8,
                )?;
                image::ImageResult::Ok(out)
            })
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(PyBytes::new(py, &bytes))
    }

    #[getter]
    fn width(&self) -> u32 {
        self.pixels.width()
    }
    #[getter]
    fn height(&self) -> u32 {
        self.pixels.height()
    }
    #[pyo3(name = "pixelColor")]
    fn pixel_color(&self, x: i64, y: i64) -> PyResult<crate::annotation_canvas::Color> {
        let pixel = u32::try_from(x).ok().zip(u32::try_from(y).ok())
            .and_then(|(x, y)| self.pixels.get_pixel_checked(x, y))
            .ok_or_else(|| PyValueError::new_err("pixel outside raster"))?;
        Ok(crate::annotation_canvas::Color::from_pixel(pixel.0))
    }
    fn rgba<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.pixels.as_raw())
    }
    fn elevation<'py>(&self, py: Python<'py>, encoding: &str) -> Bound<'py, PyArray2<f32>> {
        py.detach(|| {
            Array2::from_shape_fn((self.height() as usize, self.width() as usize), |(y, x)| {
                let [r, g, b, _] = self.pixels.get_pixel(x as u32, y as u32).0.map(f32::from);
                if encoding == "mapbox" {
                    -10000. + (r * 65536. + g * 256. + b) * 0.1
                } else {
                    r * 256. + g + b / 256. - 32768.
                }
            })
        })
        .into_pyarray(py)
    }
    #[staticmethod]
    fn mosaic(
        tile_range: (i32, i32, u32, u32, i32),
        tiles: &Bound<'_, PyDict>,
        side: u32,
    ) -> PyResult<Option<Self>> {
        let (x0, y0, cols, rows, z0) = tile_range;
        let (Some(w), Some(h)) = (cols.checked_mul(side), rows.checked_mul(side)) else {
            return Ok(None);
        };
        if !fits(w, h) {
            return Ok(None);
        }
        let mut output = None;
        for (addr, value) in tiles.iter() {
            let (x, y, z) = addr.extract::<(i32, i32, i32)>()?;
            let dx = i64::from(x) - i64::from(x0);
            let dy = i64::from(y) - i64::from(y0);
            if z != z0 || !(0..i64::from(cols)).contains(&dx) || !(0..i64::from(rows)).contains(&dy)
            {
                continue;
            }
            let src = value.extract::<PyRef<'_, Self>>()?;
            let pixels = output
                .get_or_insert_with(|| RgbaImage::from_pixel(w, h, Rgba([138, 138, 138, 255])));
            imageops::overlay(
                pixels,
                &src.pixels,
                dx * i64::from(side),
                dy * i64::from(side),
            );
        }
        Ok(output.map(|pixels| Self { pixels }))
    }
}

fn target_size(w: u32, h: u32, bounds: (u32, u32)) -> (u32, u32) {
    let scale = (bounds.0 as f64 / w as f64)
        .min(bounds.1 as f64 / h as f64)
        .min(1.);
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

/// WebODM's non-interlaced PNGs can exceed 2 GiB uncompressed. Box-filter
/// directly from scanlines; only one source row and one accumulator row are live.
fn scaled_png(path: &Path, bounds: (u32, u32)) -> Option<RgbaImage> {
    let mut dec = png::Decoder::new(BufReader::new(File::open(path).ok()?));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    if reader.info().interlaced {
        return None;
    }
    let (w, h) = (reader.info().width, reader.info().height);
    let (tw, th) = target_size(w, h, bounds);
    if !fits(tw, th) {
        return None;
    }
    let (color, depth) = reader.output_color_type();
    if depth != png::BitDepth::Eight || color == png::ColorType::Indexed {
        return None;
    }
    let n = color.samples();
    let mut dest = RgbaImage::new(tw, th);
    let mut sums = vec![[0u64; 5]; tw as usize];
    for y in 0..h {
        let row = reader.next_row().ok()??;
        for (x, px) in row.data().chunks_exact(n).enumerate() {
            let (r, g, b, a) = match px {
                [v] => (*v, *v, *v, 255),
                [v, a] => (*v, *v, *v, *a),
                [r, g, b] => (*r, *g, *b, 255),
                [r, g, b, a] => (*r, *g, *b, *a),
                _ => return None,
            };
            let sum = &mut sums[(x as u64 * u64::from(tw) / u64::from(w)) as usize];
            let alpha = u64::from(a);
            sum[0] += u64::from(r) * alpha;
            sum[1] += u64::from(g) * alpha;
            sum[2] += u64::from(b) * alpha;
            sum[3] += alpha;
            sum[4] += 1;
        }
        let dy = u64::from(y) * u64::from(th) / u64::from(h);
        if u64::from(y + 1) * u64::from(th) / u64::from(h) != dy {
            for (x, sum) in sums.iter_mut().enumerate() {
                let [r, g, b, a, count] = *sum;
                let straight = |v| if a == 0 { 0 } else { ((v + a / 2) / a) as u8 };
                dest.put_pixel(
                    x as u32,
                    dy as u32,
                    Rgba([
                        straight(r),
                        straight(g),
                        straight(b),
                        ((a + count / 2) / count) as u8,
                    ]),
                );
                *sum = [0; 5];
            }
        }
    }
    reader.finish().ok()?;
    Some(dest)
}
