// SPDX-License-Identifier: GPL-3.0-or-later
use super::{error, Obj};
use crate::file_open::DocumentOpen;
use numpy::{ndarray::Array3, IntoPyArray, PyArray3};
use pyo3::prelude::*;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};
#[derive(FromPyObject)]
struct Layout {
    path: PathBuf,
    width: usize,
    height: usize,
    samples: usize,
    bits: Vec<u16>,
    sample_format: Vec<u16>,
    compression: u32,
    photometric: u16,
    planar: u16,
    predictor: u16,
    tile_w: usize,
    tile_h: usize,
    offsets: Vec<u64>,
    bytecounts: Vec<u64>,
    byte_order: String,
    jpeg_tables: Vec<u8>,
}
impl Layout {
    fn reason(&self) -> Option<String> {
        let bit = *self.bits.first().unwrap_or(&0);
        if !matches!(self.compression, 1 | 5 | 7 | 8 | 32773 | 32946) {
            return Some(format!("compression scheme {}", self.compression));
        }
        if self.bits.is_empty()
            || self
                .bits
                .iter()
                .take(self.samples)
                .any(|b| !matches!(b, 8 | 16))
        {
            return Some(format!("{bit}-bit samples"));
        }
        if self
            .sample_format
            .iter()
            .take(self.samples)
            .any(|b| !matches!(b, 1 | 4))
        {
            return Some("floating-point samples (an elevation model, not a picture)".into());
        }
        if !matches!(self.photometric, 0 | 1 | 2 | 6) {
            return Some(format!("photometric interpretation {}", self.photometric));
        }
        if self.samples == 0
            || (!(1..=4).contains(&self.samples) && self.photometric != 2)
            || (self.samples == 2 && self.photometric > 1)
        {
            return Some(format!("{} samples per pixel", self.samples));
        }
        if self.compression == 7 && (bit != 8 || self.planar != 1 || self.samples > 4) {
            return Some("JPEG-compressed samples in this layout".into());
        }
        None
    }
    fn block(&self, src: &[u8], channels: usize) -> Result<Vec<u8>, String> {
        let bytes = if self.bits[0] == 16 { 2 } else { 1 };
        let need = product(&[self.tile_h, self.tile_w, channels, bytes])?;
        if self.compression == 7 {
            return Ok(crate::survey_pixels::jpeg_block(
                src,
                &self.jpeg_tables,
                self.tile_w,
                self.tile_h,
                channels,
            )
            .into_raw_vec_and_offset()
            .0);
        }
        let mut buf = zeroed(need)?;
        match self.compression {
            5 => {
                let mut decoder =
                    weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
                let (mut used, mut written) = (0, 0);
                while written < need {
                    let step = decoder.decode_bytes(&src[used..], &mut buf[written..]);
                    used += step.consumed_in;
                    written += step.consumed_out;
                    if !matches!(step.status, Ok(weezl::LzwStatus::Ok))
                        || step.consumed_in + step.consumed_out == 0
                    {
                        break;
                    }
                }
            }
            8 | 32946 => {
                let mut z = flate2::read::ZlibDecoder::new(src);
                let mut n = 0;
                while n < need {
                    let len = z.read(&mut buf[n..]).map_err(|e| e.to_string())?;
                    if len == 0 {
                        break;
                    }
                    n += len;
                }
            }
            32773 => {
                let mut i = 0;
                let mut j = 0;
                while i < src.len() && j < need {
                    let v = src[i];
                    i += 1;
                    if v < 128 {
                        let n = (usize::from(v) + 1).min(src.len() - i).min(need - j);
                        buf[j..j + n].copy_from_slice(&src[i..i + n]);
                        i += n;
                        j += n;
                    } else if v > 128 && i < src.len() {
                        let n = (257 - usize::from(v)).min(need - j);
                        buf[j..j + n].fill(src[i]);
                        i += 1;
                        j += n;
                    }
                }
            }
            _ => {
                let n = need.min(src.len());
                buf[..n].copy_from_slice(&src[..n]);
            }
        }
        let stride = self.tile_w * channels;
        if bytes == 1 {
            if self.predictor == 2 {
                for row in buf.chunks_exact_mut(stride) {
                    for j in channels..row.len() {
                        row[j] = row[j].wrapping_add(row[j - channels])
                    }
                }
            }
            return Ok(buf);
        }
        let little = self.byte_order == "<";
        let mut out = Vec::with_capacity(need / 2);
        let mut previous = vec![0u16; channels];
        for row in buf.chunks_exact(stride * 2) {
            previous.fill(0);
            for (j, p) in row.chunks_exact(2).enumerate() {
                let a = [p[0], p[1]];
                let mut v = if little {
                    u16::from_le_bytes(a)
                } else {
                    u16::from_be_bytes(a)
                };
                if self.predictor == 2 {
                    let k = j % channels;
                    v = v.wrapping_add(previous[k]);
                    previous[k] = v;
                }
                out.push((v >> 8) as u8);
            }
        }
        Ok(out)
    }
    fn band(&self, file: &mut File, index: usize, rows: usize) -> Result<Vec<u8>, String> {
        let plane = if self.planar == 2 { self.samples } else { 1 };
        let channels = self.samples / plane;
        let mut raw = zeroed(product(&[rows, self.width, self.samples])?)?;
        let across = self.width.div_ceil(self.tile_w);
        let down = self.height.div_ceil(self.tile_h);
        for p in 0..plane {
            for x in 0..across {
                if crate::file_open::cancelled() {
                    return Err("cancelled".into());
                }
                let id = (p * down + index) * across + x;
                let offset = *self.offsets.get(id).ok_or("missing TIFF tile")?;
                let count = *self.bytecounts.get(id).ok_or("missing TIFF byte count")?;
                file.seek(SeekFrom::Start(offset))
                    .map_err(|e| e.to_string())?;
                let mut compressed = vec![];
                file.take(count)
                    .read_to_end(&mut compressed)
                    .map_err(|e| e.to_string())?;
                if compressed.len() as u64 != count {
                    return Err("truncated TIFF tile".into());
                }
                let block = self.block(&compressed, channels)?;
                let start = x * self.tile_w;
                let cols = self.tile_w.min(self.width - start);
                for y in 0..rows {
                    let target = (y * self.width + start) * self.samples;
                    let source = y * self.tile_w * channels;
                    if plane == 1 {
                        raw[target..target + cols * channels]
                            .copy_from_slice(&block[source..source + cols * channels]);
                    } else {
                        for xx in 0..cols {
                            raw[target + xx * self.samples + p] = block[source + xx];
                        }
                    }
                }
            }
        }
        Ok(raw)
    }
}
fn zeroed(n: usize) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    data.try_reserve_exact(n)
        .map_err(|_| "TIFF buffer exceeds available memory".to_string())?;
    data.resize(n, 0);
    Ok(data)
}
fn product(v: &[usize]) -> Result<usize, String> {
    v.iter()
        .try_fold(1usize, |n, &m| n.checked_mul(m))
        .filter(|&n| n > 0 && n <= isize::MAX as usize)
        .ok_or_else(|| "invalid TIFF dimensions".into())
}
#[pyfunction]
fn unsupported_reason(info: Layout) -> Option<String> {
    info.reason()
}
// Progress callbacks belong to the UI thread. Replay every completed band even
// when several short strips finish between two polls of the document worker.
#[pyclass]
struct BandProgress {
    callback: Py<PyAny>,
    count: usize,
    delivered: usize,
}
#[pymethods]
impl BandProgress {
    fn __call__(&mut self, py: Python<'_>, fraction: f64, _phase: &str) -> PyResult<()> {
        let done = ((fraction * self.count as f64).round() as usize).min(self.count);
        while self.delivered < done {
            self.delivered += 1;
            if !self
                .callback
                .bind(py)
                .call1((self.delivered as f64 / self.count as f64,))?
                .is_truthy()?
            {
                return Err(error(py, "cancelled"));
            }
        }
        Ok(())
    }
}
#[pyfunction(signature=(info,max_px=8192,progress=None))]
fn read_rgba(
    py: Python<'_>,
    info: Layout,
    max_px: usize,
    progress: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    let count = info.height.div_ceil(info.tile_h.max(1));
    let callback = progress
        .map(|callback| {
            Py::new(
                py,
                BandProgress {
                    callback,
                    count,
                    delivered: 0,
                },
            )
        })
        .transpose()?;
    let nested = DocumentOpen::staging();
    let caller = callback.as_ref().map(|c| c.bind(py).as_any().clone());
    DocumentOpen::stage(py, caller.as_ref(), move |py, worker| {
        let notify = |fraction| -> PyResult<()> {
            if nested {
                if let Some(cb) = &callback {
                    cb.call1(py, (fraction, "Reading orthomosaic"))?;
                }
            } else if callback.is_some() {
                worker.call1((fraction, "Reading orthomosaic"))?;
            }
            Ok(())
        };
        Ok(decode(py, info, max_px, notify)?
            .into_pyobject(py)?
            .into_any()
            .unbind())
    })
}
fn decode<'p>(
    py: Python<'p>,
    info: Layout,
    max_px: usize,
    progress: impl Fn(f64) -> PyResult<()>,
) -> PyResult<(Bound<'p, PyArray3<u8>>, usize)> {
    if let Some(r) = info.reason() {
        return Err(error(
            py,
            format!("{}: {r} is not supported", info.path.display()),
        ));
    }
    if max_px == 0 || info.tile_w == 0 || info.tile_h == 0 {
        return Err(error(py, "invalid TIFF dimensions"));
    }
    let k = info
        .width
        .max(info.height)
        .div_ceil(max_px)
        .checked_next_power_of_two()
        .ok_or_else(|| error(py, "invalid TIFF dimensions"))?;
    let w = info.width.div_ceil(k);
    let h = info.height.div_ceil(k);
    let n = product(&[w, h, 4]).map_err(|e| error(py, e))?;
    let mut out = zeroed(n).map_err(|e| error(py, e))?;
    let mut sums = vec![[0u64; 4]; w];
    let denom = (k as u64)
        .checked_mul(k as u64)
        .filter(|&n| n <= u64::MAX / 255)
        .ok_or_else(|| error(py, "invalid TIFF reduction"))?;
    let bands = info.height.div_ceil(info.tile_h);
    let mut f = File::open(&info.path)?;
    for i in 0..bands {
        py.detach(|| -> Result<(), String> {
            let rows = info.tile_h.min(info.height - i * info.tile_h);
            let raw = info.band(&mut f, i, rows)?;
            for (j, row) in raw.chunks_exact(info.width * info.samples).enumerate() {
                let y = i * info.tile_h + j;
                let ny = if y + 1 == info.height { k - y % k } else { 1 };
                for (x, p) in row.chunks_exact(info.samples).enumerate() {
                    let rgba = if info.samples <= 2 {
                        let gray = if info.photometric == 0 {
                            255 - p[0]
                        } else {
                            p[0]
                        };
                        [gray, gray, gray, *p.get(1).unwrap_or(&255)]
                    } else {
                        [p[0], p[1], p[2], *p.get(3).unwrap_or(&255)]
                    };
                    if k == 1 {
                        let d = (y * w + x) * 4;
                        out[d..d + 4].copy_from_slice(&rgba);
                    } else {
                        let nx = if x + 1 == info.width { k - x % k } else { 1 };
                        for (sum, v) in sums[x / k].iter_mut().zip(rgba) {
                            *sum += v as u64 * nx as u64 * ny as u64;
                        }
                    }
                }
                if k > 1 && ((y + 1) % k == 0 || y + 1 == info.height) {
                    let dest = &mut out[(y / k) * w * 4..][..w * 4];
                    for (px, acc) in dest.chunks_exact_mut(4).zip(&mut sums) {
                        for c in 0..4 {
                            px[c] = (acc[c] / denom) as u8
                        }
                        *acc = [0; 4];
                    }
                }
            }
            Ok(())
        })
        .map_err(|e| error(py, e))?;
        progress((i + 1) as f64 / bands as f64)?;
        crate::file_open::checkpoint(py)?;
    }
    Ok((
        Array3::from_shape_vec((h, w, 4), out)
            .unwrap()
            .into_pyarray(py),
        k,
    ))
}
#[pyfunction]
fn describe(info: &Obj<'_>) -> PyResult<String> {
    let h: Layout = info.extract()?;
    let code = h.compression.to_string();
    let comp = match h.compression {
        1 => "raw",
        5 => "LZW",
        7 => "JPEG",
        8 | 32946 => "Deflate",
        32773 => "PackBits",
        _ => &code,
    };
    let place = if let Some(z) = info.getattr("zone")?.extract::<Option<usize>>()? {
        format!(
            "UTM {z}{}",
            if info.getattr("northern")?.is_truthy()? {
                "N"
            } else {
                "S"
            }
        )
    } else if info.getattr("geographic")?.is_truthy()? {
        "lat/lon".into()
    } else {
        "no CRS".into()
    };
    Ok(format!(
        "{}×{} px, {}×{} bit, {comp}, {}, {place}",
        h.width,
        h.height,
        h.samples,
        h.bits.first().unwrap_or(&0),
        if info.getattr("tiled")?.is_truthy()? {
            "tiled"
        } else {
            "strips"
        }
    ))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(unsupported_reason, m)?)?;
    m.add_function(wrap_pyfunction!(read_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(describe, m)?)?;
    Ok(())
}
