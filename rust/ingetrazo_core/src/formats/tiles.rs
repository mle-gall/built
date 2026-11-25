// SPDX-License-Identifier: GPL-3.0-or-later
use super::{path_obj, Obj};
use crate::math::Vec3;
use glam::DVec3;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyTuple},
};
use std::{
    collections::BTreeSet,
    f64::consts::PI,
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
#[pyfunction]
pub(super) fn deg2num(lat_deg: f64, lon_deg: f64, zoom: i32) -> (f64, f64) {
    let n = 2f64.powi(zoom);
    (
        (lon_deg + 180.) / 360. * n,
        (1. - lat_deg
            .clamp(-85.05112878, 85.05112878)
            .to_radians()
            .tan()
            .asinh()
            / PI)
            / 2.
            * n,
    )
}
#[pyfunction]
fn num2deg(xtile: f64, ytile: f64, zoom: i32) -> (f64, f64) {
    let n = 2f64.powi(zoom);
    (
        (PI * (1. - 2. * ytile / n)).sinh().atan().to_degrees(),
        xtile / n * 360. - 180.,
    )
}
#[pyfunction]
fn tile_bbox(x: i64, y: i64, zoom: i32) -> (f64, f64, f64, f64) {
    let (n, w) = num2deg(x as f64, y as f64, zoom);
    let (s, e) = num2deg(x as f64 + 1., y as f64 + 1., zoom);
    (s, w, n, e)
}
fn ranges(
    s: f64,
    w: f64,
    n: f64,
    e: f64,
    z: i32,
) -> (std::ops::RangeInclusive<i64>, std::ops::RangeInclusive<i64>) {
    let (x0, y0) = deg2num(n, w, z);
    let (x1, y1) = deg2num(s, e, z);
    let max = (2f64.powi(z).ceil() as i64).saturating_sub(1);
    let bounds =
        |a: f64, b: f64| (a.min(b).floor() as i64).max(0)..=(a.max(b).floor() as i64).min(max);
    (bounds(x0, x1), bounds(y0, y1))
}
#[pyfunction]
pub(super) fn tiles_covering(
    lat_s: f64,
    lon_w: f64,
    lat_n: f64,
    lon_e: f64,
    zoom: i32,
) -> Vec<(i64, i64)> {
    let (x, y) = ranges(lat_s, lon_w, lat_n, lon_e, zoom);
    x.flat_map(|x| y.clone().map(move |y| (x, y))).collect()
}
#[pyfunction]
fn tile_url(source: &Obj<'_>, x: i64, y: i64, z: i64) -> PyResult<String> {
    let subs = source.getattr_opt("subdomains")?;
    let s = if let Some(v) = subs.filter(|v| v.len().is_ok_and(|n| n > 0)) {
        v.get_item((x as i128 + y as i128).rem_euclid(v.len()? as i128) as usize)?
            .extract::<String>()?
    } else {
        String::new()
    };
    Ok(source
        .getattr("url_template")?
        .extract::<String>()?
        .replace("{s}", &s)
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string()))
}
#[pyfunction]
fn source_slug(name: &str) -> String {
    let slug = name
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "fuente".into()
    } else {
        slug
    }
}
#[pyfunction(signature=(url_template,max_zoom=19,name=None))]
fn custom_source<'p>(
    py: Python<'p>,
    url_template: &str,
    max_zoom: i32,
    name: Option<&str>,
) -> PyResult<Obj<'p>> {
    let name = name.filter(|s| !s.is_empty());
    py.import("georef.tiles")?.call_method1(
        "TileSource",
        (
            name.map(|s| format!("custom-{}", source_slug(s)))
                .unwrap_or_else(|| "custom".into()),
            name.unwrap_or("XYZ personalizado"),
            url_template,
            max_zoom,
            "Fuente personalizada (definida por el usuario)",
        ),
    )
}
#[pyfunction(signature=(layer,source,zoom=16,radius_m=1200.))]
fn tile_init(layer: &Obj<'_>, source: &Obj<'_>, zoom: i32, radius_m: f64) -> PyResult<()> {
    let py = layer.py();
    layer.setattr("source", source)?;
    layer.setattr("zoom", zoom)?;
    layer.setattr("radius_m", radius_m)?;
    layer.setattr("visible", true)?;
    layer.setattr("opacity", 1.)?;
    layer.setattr("terrain3d", false)?;
    layer.setattr("images", PyDict::new(py))?;
    layer.setattr("patches", vec![(0., 0., radius_m, radius_m)])
}
#[pyfunction]
fn tile_to_dict<'p>(py: Python<'p>, layer: &Obj<'p>) -> PyResult<Obj<'p>> {
    let source = layer.getattr("source")?;
    let d = PyDict::new(py);
    d.set_item("source", source.getattr("id")?)?;
    for k in ["url_template", "name", "attribution"] {
        d.set_item(k, source.getattr(k)?)?
    }
    for k in ["zoom", "radius_m", "visible", "opacity", "terrain3d"] {
        d.set_item(k, layer.getattr(k)?)?
    }
    let p = PyList::empty(py);
    for v in layer.getattr("patches")?.try_iter()? {
        p.append(py.get_type::<PyList>().call1((v?,))?)?
    }
    d.set_item("patches", p)?;
    Ok(d.into_any())
}
#[pyfunction]
fn tile_from_dict<'p>(py: Python<'p>, cls: &Obj<'p>, raw: &Obj<'p>) -> PyResult<Obj<'p>> {
    let m = py.import("georef.tiles")?;
    let id = raw.call_method1("get", ("source", "custom"))?;
    let mut source = m.getattr("PRESETS")?.call_method1("get", (&id,))?;
    if source.is_none() {
        let kw = PyDict::new(py);
        kw.set_item("attribution", raw.call_method1("get", ("attribution", ""))?)?;
        source = m.call_method(
            "TileSource",
            (
                &id,
                raw.call_method1("get", ("name", &id))?,
                raw.call_method1("get", ("url_template", ""))?,
            ),
            Some(&kw),
        )?;
    }
    let num = |k, d| -> PyResult<f64> {
        py.get_type::<pyo3::types::PyFloat>()
            .call1((raw.call_method1("get", (k, d))?,))?
            .extract()
    };
    let o = cls.call1((source, num("zoom", 16.)? as i32, num("radius_m", 1200.)?))?;
    let patches = raw.call_method1("get", ("patches",))?;
    if patches.is_truthy()? {
        let p = PyList::empty(py);
        for v in patches.try_iter()? {
            let vals = v?
                .try_iter()?
                .map(|v| py.get_type::<pyo3::types::PyFloat>().call1((v?,)))
                .collect::<PyResult<Vec<_>>>()?;
            p.append(PyTuple::new(py, vals)?)?
        }
        o.setattr("patches", p)?
    }
    o.setattr(
        "visible",
        raw.call_method1("get", ("visible", true))?.is_truthy()?,
    )?;
    o.setattr("opacity", num("opacity", 1.)?.clamp(0., 1.))?;
    o.setattr(
        "terrain3d",
        raw.call_method1("get", ("terrain3d", false))?.is_truthy()?,
    )?;
    Ok(o)
}
#[pyfunction(signature=(layer,width_m,length_m,cx=0.,cy=0.))]
fn set_rectangle(layer: &Obj<'_>, width_m: f64, length_m: f64, cx: f64, cy: f64) -> PyResult<()> {
    layer.setattr("patches", vec![(cx, cy, width_m / 2., length_m / 2.)])
}
#[pyfunction]
fn add_patch(layer: &Obj<'_>, cx: f64, cy: f64, hw: f64, hh: f64) -> PyResult<()> {
    layer
        .getattr("patches")?
        .call_method1("append", ((cx, cy, hw, hh),))?;
    Ok(())
}
#[pyfunction(name = "terrain_geodetic_bounds")]
pub(super) fn bbox(datum: &Obj<'_>, bounds: [f64; 4]) -> PyResult<(f64, f64, f64, f64)> {
    let [x0, y0, x1, y1] = bounds;
    let (mut s, mut w, mut n, mut e) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        let (a, b, _) = datum
            .call_method1("local_to_geodetic", (Vec3::from(DVec3::new(x, y, 0.)),))?
            .extract::<(f64, f64, f64)>()?;
        s = s.min(a);
        w = w.min(b);
        n = n.max(a);
        e = e.max(b);
    }
    Ok((s, w, n, e))
}
#[pyfunction]
fn patch_tiles(
    layer: &Obj<'_>,
    datum: &Obj<'_>,
    cx: f64,
    cy: f64,
    hw: f64,
    hh: f64,
) -> PyResult<Vec<(i64, i64)>> {
    let (s, w, n, e) = bbox(datum, [cx - hw, cy - hh, cx + hw, cy + hh])?;
    Ok(tiles_covering(
        s,
        w,
        n,
        e,
        layer.getattr("zoom")?.extract()?,
    ))
}
#[pyfunction]
fn visible_tiles(layer: &Obj<'_>, datum: &Obj<'_>) -> PyResult<Vec<(i64, i64)>> {
    let r = layer.getattr("radius_m")?.extract()?;
    patch_tiles(layer, datum, 0., 0., r, r)
}
fn coverage(
    layer: &Obj<'_>,
    datum: &Obj<'_>,
    limit: Option<usize>,
) -> PyResult<(bool, BTreeSet<(i64, i64)>)> {
    let zoom = layer.getattr("zoom")?.extract()?;
    let mut seen = BTreeSet::new();
    for p in layer.getattr("patches")?.try_iter()? {
        let (cx, cy, hw, hh) = p?.extract::<(f64, f64, f64, f64)>()?;
        let (s, w, n, e) = bbox(datum, [cx - hw, cy - hh, cx + hw, cy + hh])?;
        let (xs, ys) = ranges(s, w, n, e, zoom);
        for x in xs {
            for y in ys.clone() {
                seen.insert((x, y));
                if limit.is_some_and(|n| seen.len() > n) {
                    return Ok((true, seen));
                }
            }
        }
    }
    Ok((false, seen))
}
#[pyfunction]
fn flat_tiles(layer: &Obj<'_>, datum: &Obj<'_>) -> PyResult<Vec<(i64, i64)>> {
    Ok(coverage(layer, datum, None)?.1.into_iter().collect())
}
#[pyfunction(signature=(layer,datum,max_tiles=500,min_zoom=8))]
fn cap_detail(layer: &Obj<'_>, datum: &Obj<'_>, max_tiles: usize, min_zoom: i32) -> PyResult<bool> {
    let mut z = layer.getattr("zoom")?.extract::<i32>()?;
    let old = z;
    while z > min_zoom && coverage(layer, datum, Some(max_tiles))?.0 {
        z -= 1;
        layer.setattr("zoom", z)?
    }
    Ok(z != old)
}
#[pyfunction]
fn capture_bounds(
    layer: &Obj<'_>,
    datum: &Obj<'_>,
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
) -> PyResult<(f64, f64)> {
    let (w, h, cx, cy) = crate::survey_pixels::survey_rectangle(minx, miny, maxx, maxy);
    set_rectangle(layer, w, h, cx, cy)?;
    cap_detail(layer, datum, 500, 8)?;
    Ok((w, h))
}
#[pyfunction]
fn quad_local<'p>(
    layer: &Obj<'p>,
    datum: &Obj<'p>,
    x: i64,
    y: i64,
) -> PyResult<Vec<(Obj<'p>, (f64, f64))>> {
    let (s, w, n, e) = tile_bbox(x, y, layer.getattr("zoom")?.extract()?);
    let local = |a, b| datum.call_method1("geodetic_to_local", (a, b));
    let nw = local(n, w)?;
    let se = local(s, e)?;
    Ok(vec![
        (nw.clone(), (0., 0.)),
        (local(n, e)?, (1., 0.)),
        (se.clone(), (1., 1.)),
        (nw, (0., 0.)),
        (se, (1., 1.)),
        (local(s, w)?, (0., 1.)),
    ])
}
#[pyclass(module = "georef.tiles", dict)]
struct TileCache {
    root: PathBuf,
    #[pyo3(get, set)]
    max_bytes: u64,
}
impl TileCache {
    fn file(&self, src: &str, x: i64, y: i64, z: i64) -> PyResult<PathBuf> {
        let mut c = Path::new(src).components();
        if !matches!(c.next(), Some(std::path::Component::Normal(_))) || c.next().is_some() {
            return Err(PyValueError::new_err("invalid tile source id"));
        }
        Ok(self
            .root
            .join(src)
            .join(z.to_string())
            .join(x.to_string())
            .join(y.to_string()))
    }
    fn files(&self) -> std::io::Result<Vec<(SystemTime, u64, PathBuf)>> {
        let mut dirs = vec![self.root.clone()];
        let mut files = vec![];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(dir)? {
                let e = entry?;
                let t = e.file_type()?;
                if t.is_dir() {
                    dirs.push(e.path())
                } else if t.is_file() {
                    let m = e.metadata()?;
                    files.push((m.modified()?, m.len(), e.path()))
                }
            }
        }
        Ok(files)
    }
}
#[pymethods]
impl TileCache {
    #[new]
    #[pyo3(signature=(root,max_bytes=268435456))]
    fn new(root: PathBuf, max_bytes: u64) -> PyResult<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self { root, max_bytes })
    }
    #[getter]
    fn root<'p>(&self, py: Python<'p>) -> PyResult<Obj<'p>> {
        path_obj(py, &self.root)
    }
    fn path_for<'p>(
        &self,
        py: Python<'p>,
        source_id: &str,
        x: i64,
        y: i64,
        z: i64,
    ) -> PyResult<Obj<'p>> {
        path_obj(py, &self.file(source_id, x, y, z)?)
    }
    fn get<'p>(
        &self,
        py: Python<'p>,
        source_id: &str,
        x: i64,
        y: i64,
        z: i64,
    ) -> PyResult<Option<Bound<'p, PyBytes>>> {
        let p = self.file(source_id, x, y, z)?;
        let bytes = py.detach(|| {
            fs::read(&p).ok().inspect(|_| {
                if let Ok(f) = File::open(&p) {
                    if let Ok(t) = f.metadata().and_then(|m| m.modified()) {
                        let _ = f.set_modified(t + Duration::from_secs(1));
                    }
                }
            })
        });
        Ok(bytes.map(|b| PyBytes::new(py, &b)))
    }
    fn put(
        &self,
        py: Python<'_>,
        source_id: &str,
        x: i64,
        y: i64,
        z: i64,
        data: &[u8],
    ) -> PyResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        let p = self.file(source_id, x, y, z)?;
        py.detach(|| -> std::io::Result<()> {
            static CACHE: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _lock = CACHE.lock().unwrap_or_else(|e| e.into_inner());
            fs::create_dir_all(p.parent().unwrap())?;
            let mut stage = crate::atomic_document::AtomicDocument::create(p.clone())?;
            std::io::Write::write_all(stage.output()?, data)?;
            stage.commit()?;
            let mut files = self.files()?;
            let newest = files.iter().filter(|(_, _, f)| f != &p).map(|x| x.0).max();
            if let Some((t, _, _)) = files.iter_mut().find(|(_, _, f)| f == &p) {
                if let Some(new) = newest.filter(|n| *n >= *t) {
                    *t = new + Duration::from_secs(1);
                    let _ = File::open(&p)?.set_modified(*t);
                }
            }
            let mut total = files.iter().map(|v| v.1).sum::<u64>();
            if total > self.max_bytes {
                files.sort_by_key(|v| v.0);
                for (_, n, f) in files {
                    if total <= self.max_bytes {
                        break;
                    }
                    if fs::remove_file(f).is_ok() {
                        total -= n
                    }
                }
            }
            Ok(())
        })?;
        Ok(())
    }
}
#[pyfunction]
fn default_cache_dir(py: Python<'_>) -> PyResult<Obj<'_>> {
    let profile = std::env::var_os("BUILT_SETTINGS_DIR").map(PathBuf::from);
    let renderer = py.import("ingetrazo_render")?;
    let base = if let Some(directory) = renderer.getattr_opt("local_data_directory")? {
        directory.call1((profile,))?.extract::<Option<PathBuf>>()?
    } else {
        // Older standalone renderer wheels predate the native path binding.
        profile.map(|p| p.join("data")).or_else(|| {
            #[cfg(target_os = "macos")]
            {
                std::env::var_os("HOME")
                    .map(|p| PathBuf::from(p).join("Library/Application Support"))
            }
            #[cfg(windows)]
            {
                std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            }
            #[cfg(not(any(target_os = "macos", windows)))]
            {
                std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .or_else(|| {
                        std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share"))
                    })
            }
        })
    }
    .ok_or_else(|| pyo3::exceptions::PyOSError::new_err("Missing user data directory"))?;
    path_obj(py, &base.join("IngeTrazo/IngeTrazo/tiles"))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    functions!(m;deg2num,num2deg,tile_bbox,bbox,tiles_covering,tile_url,source_slug,custom_source,tile_init,tile_to_dict,tile_from_dict,set_rectangle,add_patch,patch_tiles,flat_tiles,visible_tiles,quad_local,capture_bounds,cap_detail,default_cache_dir);
    m.add_class::<TileCache>()
}
