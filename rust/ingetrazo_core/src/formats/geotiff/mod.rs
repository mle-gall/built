// SPDX-License-Identifier: GPL-3.0-or-later
//! TIFF/BigTIFF directory and GeoKey decoding. Pixel reads are band-streamed.
mod pixels;
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyTuple},
};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
};
type Obj<'p> = Bound<'p, PyAny>;
fn error(py: Python<'_>, msg: impl Into<String>) -> PyErr {
    let msg = msg.into();
    py.import("georef.geotiff")
        .and_then(|m| m.call_method1("GeoTiffError", (msg,)))
        .map(PyErr::from_value)
        .unwrap_or_else(|e| e)
}
fn uint(bytes: &[u8], little: bool) -> u64 {
    if little {
        bytes.iter().rev().fold(0, |n, &b| n << 8 | b as u64)
    } else {
        bytes.iter().fold(0, |n, &b| n << 8 | b as u64)
    }
}
struct Tag {
    kind: u64,
    data: Vec<u8>,
    little: bool,
}
impl Tag {
    fn ints(&self) -> Vec<u64> {
        let n = match self.kind {
            1 | 7 => 1,
            3 => 2,
            4 => 4,
            16 | 18 => 8,
            _ => return vec![],
        };
        self.data
            .chunks_exact(n)
            .map(|s| uint(s, self.little))
            .collect()
    }
    fn doubles(&self) -> Vec<f64> {
        match self.kind {
            12 => self
                .data
                .chunks_exact(8)
                .map(|b| f64::from_bits(uint(b, self.little)))
                .collect(),
            11 => self
                .data
                .chunks_exact(4)
                .map(|b| f32::from_bits(uint(b, self.little) as u32) as f64)
                .collect(),
            _ => self.ints().into_iter().map(|v| v as f64).collect(),
        }
    }
    fn string(&self) -> String {
        self.data
            .iter()
            .map(|&b| char::from(b))
            .collect::<String>()
            .trim_end_matches('\0')
            .to_owned()
    }
}
fn tags(f: &mut File) -> io::Result<(bool, bool, HashMap<u64, Tag>)> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "invalid TIFF directory");
    let length = f.metadata()?.len();
    let mut h = [0; 8];
    f.read_exact(&mut h)?;
    let little = match &h[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(bad()),
    };
    let (big, offset) = match uint(&h[2..4], little) {
        42 => (false, uint(&h[4..], little)),
        43 if uint(&h[4..6], little) == 8 => {
            f.read_exact(&mut h)?;
            (true, uint(&h, little))
        }
        _ => return Err(bad()),
    };
    f.seek(SeekFrom::Start(offset))?;
    let nsize = if big { 8 } else { 2 };
    f.read_exact(&mut h[..nsize])?;
    let n = uint(&h[..nsize], little);
    let size = if big { 20 } else { 12 };
    let width = if big { 8 } else { 4 };
    if n > length.saturating_sub(f.stream_position()?) / size as u64 {
        return Err(bad());
    }
    let mut tags = HashMap::new();
    let mut entry = [0; 20];
    for _ in 0..n {
        f.read_exact(&mut entry[..size])?;
        let id = uint(&entry[..2], little);
        if !matches!(id,256..=259|262|273|277..=279|284|317|322..=325|338..=339|347|33550|33922|34264|34735|34737|42113)
        {
            continue;
        }
        let kind = uint(&entry[2..4], little);
        let elem = match kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 | 16..=18 => 8,
            _ => continue,
        };
        let count = uint(&entry[4..size - width], little);
        let bytes = count
            .checked_mul(elem)
            .filter(|&v| v <= length)
            .ok_or_else(bad)?;
        let value = &entry[size - width..size];
        let data = if bytes <= width as u64 {
            value[..bytes as usize].to_vec()
        } else {
            let off = uint(value, little);
            if bytes > length.saturating_sub(off) {
                return Err(bad());
            }
            let pos = f.stream_position()?;
            f.seek(SeekFrom::Start(off))?;
            let mut buf = vec![0; usize::try_from(bytes).map_err(|_| bad())?];
            f.read_exact(&mut buf)?;
            f.seek(SeekFrom::Start(pos))?;
            buf
        };
        tags.insert(id, Tag { kind, data, little });
    }
    Ok((big, little, tags))
}
#[pyfunction]
fn geotiff_info<'p>(py: Python<'p>, path: PathBuf) -> PyResult<Obj<'p>> {
    let mut f = File::open(&path)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let (big, little, t) = py
        .detach(|| tags(&mut f))
        .map_err(|_| error(py, format!("{name}: not a TIFF file")))?;
    let nums = |id| t.get(&id).map(Tag::ints).unwrap_or_default();
    let one = |id, default| nums(id).first().copied().unwrap_or(default);
    let doubles = |id| t.get(&id).map(Tag::doubles).unwrap_or_default();
    let w = one(256, 0);
    let h = one(257, 0);
    if w == 0 || h == 0 {
        return Err(error(py, format!("{name}: the TIFF has no image size")));
    }
    let samples = one(277, 1);
    if !(1..=u16::MAX as u64).contains(&samples) {
        return Err(error(py, "invalid TIFF sample count"));
    }
    let mut bits = nums(258);
    bits.resize(bits.len().max(samples as usize), *bits.last().unwrap_or(&1));
    let tiled = t.contains_key(&324);
    let offsets = nums(if tiled { 324 } else { 273 });
    let counts = nums(if tiled { 325 } else { 279 });
    if offsets.is_empty() || counts.is_empty() {
        return Err(error(py, "the TIFF has no image data or byte counts"));
    }
    let kw = PyDict::new(py);
    let compression = one(259, 1);
    kw.set_item(
        "path",
        py.import("pathlib")?.call_method1("Path", (&path,))?,
    )?;
    for (k, v) in [
        ("width", w),
        ("height", h),
        ("samples", samples),
        ("compression", compression),
        ("photometric", one(262, if samples >= 3 { 2 } else { 1 })),
        ("planar", one(284, 1)),
        ("predictor", one(317, 1)),
        ("tile_w", if tiled { one(322, 0) } else { w }),
        (
            "tile_h",
            if tiled {
                one(323, 0)
            } else {
                let n = one(278, h);
                if n == 0 {
                    h
                } else {
                    n.min(h)
                }
            },
        ),
    ] {
        kw.set_item(k, v)?;
    }
    for (k, v) in [
        ("bits", bits),
        ("offsets", offsets),
        ("bytecounts", counts),
        ("extra_samples", nums(338)),
        ("sample_format", nums(339)),
    ] {
        kw.set_item(k, PyTuple::new(py, v)?)?
    }
    kw.set_item("tiled", tiled)?;
    kw.set_item("big", big)?;
    kw.set_item("byte_order", if little { "<" } else { ">" })?;
    if compression == 7 {
        if let Some(tag) = t.get(&347) {
            kw.set_item("jpeg_tables", PyBytes::new(py, &tag.data))?
        }
    }
    if let Some(tag) = t.get(&42113) {
        kw.set_item("nodata", tag.string())?
    }
    let matrix = doubles(34264);
    let scale = doubles(33550);
    let tie = doubles(33922);
    let mut affine = if matrix.len() >= 16 {
        Some([
            matrix[0], matrix[1], matrix[3], matrix[4], matrix[5], matrix[7],
        ])
    } else if scale.len() >= 2 && tie.len() >= 6 {
        Some([
            scale[0],
            0.,
            tie[3] - tie[0] * scale[0],
            0.,
            -scale[1],
            tie[4] + tie[1] * scale[1],
        ])
    } else {
        None
    };
    let ascii = t.get(&34737).map(Tag::string).unwrap_or_default();
    let keys = nums(34735);
    let mut model = 0;
    let mut point = false;
    let mut epsg = 0;
    let mut geo = false;
    let mut citation = vec![];
    if keys.len() >= 4 {
        for key in keys[4..].chunks_exact(4).take(keys[3] as usize) {
            match key[0] {
                1024 => model = key[3],
                1025 => point = key[3] == 2,
                3072 => epsg = key[3],
                2048 => geo = true,
                1026 | 3073 if key[1] == 34737 => citation.push(
                    ascii
                        .chars()
                        .skip(key[3] as usize)
                        .take(key[2] as usize)
                        .collect::<String>()
                        .trim_end_matches('|')
                        .to_owned(),
                ),
                _ => (),
            }
        }
    }
    kw.set_item("citation", citation.join("|").trim_matches('|'))?;
    if epsg != 0 && epsg != 32767 {
        kw.set_item("epsg", epsg)?
    }
    if let Some(a) = &mut affine {
        if point {
            a[2] -= 0.5 * (a[0] + a[1]);
            a[5] -= 0.5 * (a[3] + a[4])
        }
        kw.set_item("transform", PyTuple::new(py, *a)?)?
    }
    let zone = match epsg {
        32601..=32660 => Some((epsg - 32600, true)),
        32701..=32760 => Some((epsg - 32700, false)),
        _ => {
            static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r"(?i)UTM\s*zone\s*(\d{1,2})\s*([NS])").unwrap()
            });
            RE.captures(&ascii)
                .and_then(|m| Some((m[1].parse().ok()?, m[2].eq_ignore_ascii_case("N"))))
        }
    };
    if let Some((z, n)) = zone {
        kw.set_item("zone", z)?;
        kw.set_item("northern", n)?
    } else {
        kw.set_item(
            "geographic",
            model == 2
                || (epsg == 0
                    && geo
                    && affine.is_some_and(|a| a[2].abs() <= 180. && a[5].abs() <= 90.)),
        )?
    }
    py.import("georef.geotiff")?
        .getattr("GeoTiffInfo")?
        .call((), Some(&kw))
}
#[pyfunction]
fn pixel_to_map(info: &Obj<'_>, col: f64, row: f64) -> PyResult<(f64, f64)> {
    let [a, b, c, d, e, f] = info.getattr("transform")?.extract::<[f64; 6]>()?;
    Ok((a * col + b * row + c, d * col + e * row + f))
}
#[pyfunction]
fn corners_geodetic(info: &Obj<'_>) -> PyResult<Vec<(f64, f64)>> {
    let w = info.getattr("width")?.extract()?;
    let h = info.getattr("height")?.extract()?;
    let geo = info.getattr("geographic")?.is_truthy()?;
    [(0., h), (w, h), (w, 0.), (0., 0.)]
        .into_iter()
        .map(|(x, y)| {
            let (e, n) = pixel_to_map(info, x, y)?;
            Ok(if geo {
                (n, e)
            } else {
                crate::location::utm_inverse(
                    e,
                    n,
                    info.getattr("zone")?.extract()?,
                    info.getattr("northern")?.is_truthy()?,
                )
            })
        })
        .collect()
}
#[pyfunction]
fn ground_size(info: &Obj<'_>) -> PyResult<(f64, f64)> {
    let [a, b, _, d, e, f] = info.getattr("transform")?.extract::<[f64; 6]>()?;
    let h = info.getattr("height")?.extract::<f64>()?;
    let mut w = a.hypot(d) * info.getattr("width")?.extract::<f64>()?;
    let mut hh = b.hypot(e) * h;
    if info.getattr("geographic")?.is_truthy()? {
        w *= 111320. * (f + e * h / 2.).to_radians().cos();
        hh *= 110540.;
    }
    Ok((w, hh))
}
#[pyfunction]
fn georeferenced(info: &Obj<'_>) -> PyResult<bool> {
    Ok(!info.getattr("transform")?.is_none()
        && (info.getattr("geographic")?.is_truthy()? || !info.getattr("zone")?.is_none()))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(geotiff_info, m)?)?;
    m.add_function(wrap_pyfunction!(pixel_to_map, m)?)?;
    m.add_function(wrap_pyfunction!(corners_geodetic, m)?)?;
    m.add_function(wrap_pyfunction!(ground_size, m)?)?;
    m.add_function(wrap_pyfunction!(georeferenced, m)?)?;
    pixels::register(m)
}
