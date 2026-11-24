//! Pixels for the georeferenced orthomosaic importer.
use image::{codecs::webp::WebPEncoder, ExtendedColorType, ImageEncoder};
use numpy::{ndarray::Array3, IntoPyArray, PyArray3, PyReadonlyArray3, PyUntypedArrayMethods};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyBytes};
use zune_core::{bytestream::ZCursor, colorspace::ColorSpace};
use zune_jpeg::JpegDecoder;

fn jpeg_pixels(data: &[u8], tables: &[u8]) -> Option<(usize, usize, ColorSpace, Vec<u8>)> {
    // TIFF may store quantisation and Huffman tables once for all the tiles.
    let stream;
    let jpeg = if tables.ends_with(&[0xff, 0xd9]) && data.starts_with(&[0xff, 0xd8]) {
        stream = [&tables[..tables.len() - 2], &data[2..]].concat();
        &stream
    } else {
        data
    };
    let mut decoder = JpegDecoder::new(ZCursor::new(jpeg));
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    if u64::from(info.width) * u64::from(info.height) > 64 * 1024 * 1024 {
        return None;
    }
    let space = match decoder.input_colorspace()? {
        // GDAL uses four JPEG channels for R,G,B and no-data alpha. An RGB
        // conversion would multiply the scene colours by the fourth band.
        source @ (ColorSpace::CMYK | ColorSpace::YCCK) => source,
        _ => ColorSpace::RGB,
    };
    decoder.set_options(decoder.options().jpeg_set_out_colorspace(space));
    let pixels = decoder.decode().ok()?;
    Some((info.width as usize, info.height as usize, space, pixels))
}

#[pyfunction]
fn survey_jpeg_tile<'py>(
    py: Python<'py>,
    data: &[u8],
    tables: &[u8],
    width: usize,
    height: usize,
    samples: usize,
) -> PyResult<Bound<'py, PyArray3<u8>>> {
    if !(1..=4).contains(&samples)
        || width
            .checked_mul(height)
            .is_none_or(|n| n > 64 * 1024 * 1024)
    {
        return Err(PyValueError::new_err("Invalid survey tile dimensions"));
    }
    let block = py.detach(|| {
        // Damaged or short edge tiles retain the importer's zero padding.
        let mut out = Array3::<u8>::zeros((height, width, samples));
        if let Some((w, h, space, pixels)) = jpeg_pixels(data, tables) {
            let channels = space.num_components();
            let dest = out.as_slice_mut().unwrap();
            for y in 0..height.min(h) {
                for x in 0..width.min(w) {
                    let i = (y * w + x) * channels;
                    let rgb = &pixels[i..i + channels];
                    let mut rgba = [rgb[0], rgb[1], rgb[2], 255];
                    if channels == 4 {
                        rgba[3] = rgb[3];
                        if space == ColorSpace::YCCK {
                            let l = f64::from(rgb[0]);
                            let cb = f64::from(rgb[1]) - 128.;
                            let cr = f64::from(rgb[2]) - 128.;
                            let inv = |v: f64| 255 - v.round().clamp(0., 255.) as u8;
                            rgba[..3].copy_from_slice(&[
                                inv(l + 1.402 * cr),
                                inv(l - 0.344136 * cb - 0.714136 * cr),
                                inv(l + 1.772 * cb),
                            ]);
                        }
                    }
                    let j = (y * width + x) * samples;
                    let n = if samples == 2 && channels == 3 {
                        1
                    } else {
                        samples
                    };
                    dest[j..j + n].copy_from_slice(&rgba[..n]);
                }
            }
        }
        out
    });
    Ok(block.into_pyarray(py))
}

#[pyfunction]
fn survey_webp<'py>(
    py: Python<'py>,
    rgba: PyReadonlyArray3<'py, u8>,
) -> PyResult<Bound<'py, PyBytes>> {
    let [h, w, 4] = *rgba.shape() else {
        return Err(PyValueError::new_err("Expected H × W × 4 survey pixels"));
    };
    let (w, h) = (u32::try_from(w)?, u32::try_from(h)?);
    let pixels = rgba.as_slice()?;
    let mut bytes = Vec::new();
    WebPEncoder::new_lossless(&mut bytes)
        .write_image(pixels, w, h, ExtendedColorType::Rgba8)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyBytes::new(py, &bytes))
}

#[pyfunction]
fn survey_rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64, f64, f64) {
    let (w, h) = (x1 - x0, y1 - y0);
    let margin = 0.15 * w.max(h).max(200.);
    (
        w + 2. * margin,
        h + 2. * margin,
        (x0 + x1) / 2.,
        (y0 + y1) / 2.,
    )
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(survey_jpeg_tile, m)?)?;
    m.add_function(wrap_pyfunction!(survey_webp, m)?)?;
    m.add_function(wrap_pyfunction!(survey_rectangle, m)?)?;
    Ok(())
}
