//! Place parsed survey records in the current document coordinate system.
//! The file-format backends and history objects are shared with plugins.
use pyo3::{exceptions::PyValueError, prelude::*, types::PyList};
type Obj<'py> = Bound<'py, PyAny>;

struct Bounds {
    lo: [f64; 3],
    hi: [f64; 3],
}
impl Bounds {
    fn new() -> Self {
        Self {
            lo: [f64::INFINITY; 3],
            hi: [f64::NEG_INFINITY; 3],
        }
    }
    fn add(&mut self, p: &Obj<'_>) -> PyResult<()> {
        let xyz: [f64; 3] = p.call_method0("toTuple")?.extract()?;
        for (i, x) in xyz.into_iter().enumerate() {
            if !x.is_finite() {
                return Err(PyValueError::new_err("Invalid survey coordinate"));
            }
            self.lo[i] = self.lo[i].min(x);
            self.hi[i] = self.hi[i].max(x);
        }
        Ok(())
    }
    fn vectors<'py>(&self, py: Python<'py>, pad: f64) -> PyResult<(Obj<'py>, Obj<'py>)> {
        let vec = py.import("ingetrazo_core")?.getattr("Vec3")?;
        let [x, y, z] = self.lo;
        let [u, v, w] = self.hi;
        Ok((
            vec.call1((x - pad, y - pad, z - pad))?,
            vec.call1((u + pad, v + pad, w + pad))?,
        ))
    }
}

#[pyfunction]
pub fn place_alignment(window: &Obj<'_>, features: &Obj<'_>) -> PyResult<usize> {
    let py = window.py();
    let vp = window.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let mut lines = Vec::new();
    let (mut lat, mut lon, mut count) = (0., 0., 0usize);
    for feature in features.try_iter()? {
        let feature = feature?;
        let points = feature.getattr("points")?.extract::<Vec<(f64, f64)>>()?;
        if points.len() < 2 {
            continue;
        }
        for &(y, x) in &points {
            if !y.is_finite() || !x.is_finite() || y.abs() > 90. || x.abs() > 180. {
                return Err(PyValueError::new_err("Invalid survey coordinate"));
            }
            lat += y;
            lon += x;
            count += 1;
        }
        lines.push((feature, points));
    }
    if lines.is_empty() {
        return Ok(0);
    }
    let old = scene.getattr("georef")?;
    let datum = if old.is_none() {
        py.import("georef.datum")?
            .getattr("SceneDatum")?
            .call1((lat / count as f64, lon / count as f64))?
    } else {
        old.clone()
    };
    let history = py.import("core.history")?;
    let path_type = py.import("georef.geopath")?.getattr("GeoPath")?;
    let commands = PyList::empty(py);
    let mut bounds = Bounds::new();
    for (feat, points) in lines {
        let local = PyList::empty(py);
        for coord in points {
            let point = datum.call_method1("geodetic_to_local", coord)?;
            bounds.add(&point)?;
            local.append(point)?;
        }
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("name", feat.getattr("name")?)?;
        kwargs.set_item("closed", feat.getattr("closed")?)?;
        let path = path_type.call((local,), Some(&kwargs))?;
        commands.append(history.call_method1("AddGeoPathCommand", (path,))?)?;
    }
    let total = commands.len();
    // An existing project's capture includes the traces it already contains.
    for path in scene.getattr("geo_paths")?.try_iter()? {
        for point in path?.getattr("points")?.try_iter()? {
            bounds.add(&point?)?
        }
    }
    let (lo, hi) = bounds.vectors(py, 0.)?;
    let command = if total == 1 {
        commands.get_item(0)?
    } else {
        history.call_method1("CompoundCommand", (commands,))?
    };
    vp.getattr("history")?.call_method1("execute", (command,))?;
    if old.is_none() {
        scene.setattr("georef", &datum)?
    }
    window.call_method1("_set_survey_capture", (datum, &lo, &hi))?;
    let camera = vp.getattr("camera")?;
    camera.call_method1("set_view", ("top",))?;
    camera.call_method1("fit_to", (lo, hi))?;
    vp.call_method0("update")?;
    Ok(total)
}

#[pyfunction]
#[pyo3(signature=(viewport,rows,zone=18,northern=false))]
pub fn place_survey_points(
    viewport: &Obj<'_>,
    rows: &Obj<'_>,
    zone: u32,
    northern: bool,
) -> PyResult<usize> {
    let py = viewport.py();
    if !(1..=60).contains(&zone) {
        return Err(PyValueError::new_err("UTM zone must be between 1 and 60"));
    }
    let backend = py.import("georef.points")?;
    let old = viewport.getattr("scene")?.getattr("georef")?;
    let datum = if old.is_none() {
        backend.call_method1("datum_for_rows", (rows, zone, northern))?
    } else {
        old.clone()
    };
    let points = backend.call_method1("points_from_rows", (rows, &datum))?;
    let n = points.len()?;
    if n == 0 {
        return Ok(0);
    }
    let mut bounds = Bounds::new();
    for pt in points.try_iter()? {
        bounds.add(&pt?.getattr("position")?)?
    }
    let command = py.import("core.history")?.call_method1(
        "AddGeoPointsCommand",
        (points, old.is_none().then_some(datum)),
    )?;
    viewport
        .getattr("history")?
        .call_method1("execute", (command,))?;
    viewport
        .getattr("camera")?
        .call_method1("fit_to", bounds.vectors(py, 5.)?)?;
    viewport.call_method0("update")?;
    Ok(n)
}

/// Runs on the import worker. The survey parser and texture budget are the
/// same backends used for saved ODM archives.
#[pyfunction]
pub fn read_survey_mesh<'py>(
    py: Python<'py>,
    path: std::path::PathBuf,
    datum: &Obj<'py>,
    limit: u32,
    progress: &Obj<'py>,
) -> PyResult<(Obj<'py>, Bound<'py, pyo3::types::PyDict>)> {
    let backend = py.import("georef.photomesh")?;
    progress.call1((0.05, "Reading mesh…"))?;
    let mesh = backend.call_method1("load_odm_obj", (path, datum))?;
    let mut sizes = Vec::new();
    let mut files = Vec::new();
    for (index, material) in mesh.getattr("materials")?.try_iter()?.enumerate() {
        let texture = material?.getattr("texture")?;
        if let Some(path) = texture.extract::<Option<std::path::PathBuf>>()? {
            if let Some(size) = crate::image_assets::raster_dimensions(py, path.clone())? {
                sizes.push(size);
                files.push((index, path));
            }
        }
    }
    let targets = backend.call_method1("plan_texture_sizes", (sizes, limit))?;
    let images = pyo3::types::PyDict::new(py);
    let count = files.len() as f64;
    for (i, ((index, path), target)) in files.into_iter().zip(targets.try_iter()?).enumerate() {
        progress.call1((0.15 + 0.85 * i as f64 / count, "Loading textures…"))?;
        let image = backend.call_method1("load_atlas", (path, target?))?;
        if !image.is_none() {
            images.set_item(index, image)?
        }
    }
    Ok((mesh, images))
}

#[pyfunction]
pub fn survey_mesh_datum<'py>(
    py: Python<'py>,
    path: std::path::PathBuf,
    current: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    if !current.is_none() {
        return Ok(Some(current.clone()));
    }
    let anchor = py
        .import("georef.photomesh")?
        .call_method1("find_anchor", (path,))?;
    if anchor.is_none() {
        return Ok(None);
    }
    let geo = py.import("georef.datum")?;
    let (lat, lon): (f64, f64) = geo
        .call_method1(
            "utm_inverse",
            (
                anchor.getattr("east")?,
                anchor.getattr("north")?,
                anchor.getattr("zone")?,
                anchor.getattr("northern")?,
            ),
        )?
        .extract()?;
    Ok(Some(geo.call_method1("SceneDatum", (lat, lon))?))
}

#[pyfunction]
pub fn place_survey_mesh(
    window: &Obj<'_>,
    mesh: &Obj<'_>,
    images: &Obj<'_>,
    datum: &Obj<'_>,
) -> PyResult<()> {
    let py = window.py();
    let vp = window.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let altitude: f64 = datum.getattr("alt")?.extract()?;
    let origin = if altitude != 0. {
        altitude
    } else {
        py.import("georef.photomesh")?
            .call_method1("vertical_origin", (mesh,))?
            .extract::<Option<f64>>()?
            .unwrap_or(0.)
    };
    if origin != 0. {
        mesh.getattr("vertices")?
            .call_method1("__isub__", ((0., 0., origin),))?;
        mesh.call_method0("invalidate_index")?;
    }
    let layers = py.import("core.layers")?;
    let layer = layers.getattr("SURVEY_LAYER")?;
    if scene.call_method1("layer", (&layer,))?.is_none() {
        scene
            .getattr("layers")?
            .call_method1("append", (layers.call_method1("Layer", (&layer,))?,))?;
    }
    mesh.setattr("visible", true)?;
    mesh.setattr("layer", layer)?;
    mesh.setattr("images", images)?;
    datum.setattr("alt", origin)?;
    scene.setattr("georef", datum)?;
    scene.setattr("photo_mesh", mesh)?;
    // Reference meshes are document content even when the survey layer already exists.
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    vp.call_method1("upload_photo_mesh", (mesh, images))?;
    let (lo, hi): (Obj<'_>, Obj<'_>) = mesh.call_method0("bounds")?.extract()?;
    if !lo.is_none() {
        window.call_method1("_set_survey_capture", (datum, &lo, &hi))?;
        let camera = vp.getattr("camera")?;
        camera.call_method1("set_view", ("iso",))?;
        camera.call_method1("fit_to", (lo, hi))?;
    }
    vp.call_method0("update")?;
    Ok(())
}
