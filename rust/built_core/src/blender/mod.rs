// SPDX-License-Identifier: GPL-3.0-or-later
//! Public Blender render and live-preview API. A job consists of a glTF snapshot,
//! the camera and lighting parameters, and the script beside the JSON file.
use crate::math::xyz;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{IntoPyDict, PyCFunction, PyDict, PyList, PyTuple},
};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
mod discovery;
mod lights;
type Obj<'p> = Bound<'p, PyAny>;
const FLAT_ID: &str = "org.blender.Blender";
const PERMISSION: &str =
    "flatpak override --user --talk-name=org.freedesktop.Flatpak com.built.Built";
const SAMPLES: [(&str, [i32; 3]); 2] = [("eevee", [16, 64, 128]), ("cycles", [32, 128, 512])];
const EXPOSURE: [(&str, f64); 3] = [("day", -0.6), ("night", 0.), ("overcast", -0.3)];
fn path_obj<'p>(py: Python<'p>, path: &Path) -> PyResult<Obj<'p>> {
    py.import("pathlib")?.call_method1("Path", (path,))
}
#[pyfunction]
fn render_script(py: Python<'_>) -> PyResult<Obj<'_>> {
    let root = py
        .import("built_core.paths")?
        .call_method0("app_root")?
        .extract::<PathBuf>()?;
    path_obj(py, &root.join("resources/blender/render_scene.py"))
}
#[pyfunction(pass_module)]
fn work_base<'p>(m: &Bound<'p, PyModule>) -> PyResult<Obj<'p>> {
    let py = m.py();
    let base = if m.call_method0("in_flatpak")?.is_truthy()? {
        py.import("pathlib")?
            .getattr("Path")?
            .call_method0("home")?
            .extract::<PathBuf>()?
            .join(".cache")
    } else {
        py.import("built_core.paths")?
            .call_method0("cache_directory")?
            .extract()?
    };
    path_obj(py, &base.join("Built/render"))
}
#[pyfunction]
fn command(found: &Obj, job: PathBuf) -> PyResult<Vec<String>> {
    let src = render_script(found.py())?.extract::<PathBuf>()?;
    let dest = job.with_file_name("render_scene.py");
    std::fs::copy(src, &dest)?;
    let mut argv = found.getattr("command")?.extract::<Vec<String>>()?;
    argv.extend(
        [
            "-b",
            "--factory-startup",
            "--python",
            &dest.to_string_lossy(),
            "--",
            &job.to_string_lossy(),
        ]
        .map(str::to_owned),
    );
    Ok(argv)
}
#[pyfunction]
fn sun_toward<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let sun = py.import("built_core.sun")?;
    let sh = match scene.getattr_opt("shadows")?.filter(|s| !s.is_none()) {
        Some(s) => s,
        None => sun.call_method0("ShadowSettings")?,
    };
    let datum = scene.getattr_opt("georef")?;
    let loc = datum
        .as_ref()
        .and_then(|d| Some((d.getattr("lat").ok()?, d.getattr("lon").ok()?)))
        .filter(|(a, b)| !a.is_none() && !b.is_none());
    let (lat, lon) = match loc {
        Some(pair) => pair,
        None => (sun.getattr("DEFAULT_LAT")?, sun.getattr("DEFAULT_LON")?),
    };
    let kw = PyDict::new(py);
    kw.set_item(
        "year",
        py.import("datetime")?
            .getattr("date")?
            .call_method0("today")?
            .getattr("year")?,
    )?;
    let when = match sh.call_method("when_utc", (&lon,), Some(&kw)) {
        Ok(v) => v,
        Err(e) if e.is_instance_of::<PyValueError>(py) => return Ok(py.None().into_bound(py)),
        Err(e) => return Err(e),
    };
    let dir = sun.call_method1(
        "turn_sun_to_model",
        (scene, sun.call_method1("sun_direction", (lat, lon, when))?),
    )?;
    if dir.is_none() {
        Ok(dir)
    } else {
        let v = xyz(&dir)?;
        Ok((v.x, v.y, v.z).into_pyobject(py)?.into_any())
    }
}
#[pyfunction]
fn camera_dict<'p>(camera: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(camera.py());
    for (k, v) in [
        ("eye", camera.call_method0("eye")?),
        ("target", camera.getattr("target")?),
        ("up", camera.call_method0("up_vector")?),
    ] {
        out.set_item(k, xyz(&v)?.to_array().to_vec())?;
    }
    let dist = camera.getattr("distance")?.extract::<f64>()?;
    let fov = camera.getattr("fov_deg")?.extract::<f64>()?;
    out.set_item("fov_deg", fov)?;
    out.set_item("perspective", camera.getattr("perspective")?.is_truthy()?)?;
    out.set_item("half_height", dist * (fov.to_radians() * 0.5).tan())?;
    let clip = |k: &str, default: f64| {
        camera
            .getattr_opt(k)?
            .map_or(Ok(default), |v| v.extract::<f64>())
    };
    out.set_item("near", clip("znear", 0.1)?.min((dist * 0.02).max(1e-4)))?;
    out.set_item("far", clip("zfar", 1e5)?)?;
    Ok(out)
}
#[pyfunction]
fn ground_dict<'p>(scene: &Obj<'p>) -> PyResult<Option<Bound<'p, PyDict>>> {
    let bounds = scene.call_method0("bounds")?;
    let lo = bounds.get_item(0)?;
    if lo.is_none() {
        return Ok(None);
    }
    let lo = xyz(&lo)?;
    let span = xyz(&bounds.get_item(1)?)? - lo;
    let out = PyDict::new(scene.py());
    out.set_item("z", lo.z.min(0.) - 0.002)?;
    out.set_item("size", (span.x.max(span.y) * 200.).max(4000.))?;
    out.set_item("color", vec![0.42, 0.42, 0.40])?;
    Ok(Some(out))
}
#[pyfunction(signature=(scene,camera,work,*,engine="eevee",quality=1,width=1600,height=900,ground=true,keep_blend=false,ambience="day",lights=None,sun_scale=1.0,serve=false))]
#[allow(clippy::too_many_arguments)]
fn write_job<'p>(
    scene: &Obj<'p>,
    camera: &Obj<'p>,
    work: PathBuf,
    engine: &str,
    quality: i32,
    width: i32,
    height: i32,
    ground: bool,
    keep_blend: bool,
    ambience: &str,
    lights: Option<&Obj<'p>>,
    sun_scale: f64,
    serve: bool,
) -> PyResult<Obj<'p>> {
    let samples = SAMPLES
        .iter()
        .find(|(e, _)| *e == engine)
        .ok_or_else(|| PyValueError::new_err(format!("unknown engine '{engine}'")))?
        .1[quality.clamp(0, 2) as usize];
    let exposure = EXPOSURE
        .iter()
        .find(|(name, _)| *name == ambience)
        .ok_or_else(|| PyValueError::new_err(format!("unknown ambience '{ambience}'")))?
        .1;
    let py = scene.py();
    std::fs::create_dir_all(&work)?;
    let glb = work.join("model.glb");
    let cam = camera.clone().unbind();
    let toward = PyCFunction::new_closure(
        py,
        None,
        None,
        move |args: &Bound<PyTuple>, _: Option<&Bound<PyDict>>| -> PyResult<Py<PyAny>> {
            let c = cam.bind(args.py());
            let target = if c.getattr("perspective")?.is_truthy()? {
                args.get_item(0)?
            } else {
                c.getattr("target")?
            };
            Ok(c.call_method0("eye")?.sub(target)?.unbind())
        },
    )?;
    let kw = PyDict::new(py);
    kw.set_item("face_me", toward)?;
    py.import("built_core.formats.gltf")?
        .call_method("save_glb", (scene, path_obj(py, &glb)?), Some(&kw))?;
    let data = PyDict::new(py);
    data.set_item("glb", glb.to_string_lossy())?;
    data.set_item("output", work.join("render.png").to_string_lossy())?;
    data.set_item(
        "blend",
        (keep_blend && !serve).then(|| work.join("render.blend").to_string_lossy().into_owned()),
    )?;
    data.set_item("width", width)?;
    data.set_item("height", height)?;
    data.set_item("engine", engine)?;
    data.set_item("samples", samples)?;
    data.set_item("camera", camera_dict(camera)?)?;
    data.set_item("ambience", ambience)?;
    data.set_item("exposure", exposure)?;
    data.set_item(
        "sun",
        if ambience == "day" {
            Some(sun_toward(scene)?)
        } else {
            None
        },
    )?;
    data.set_item("sun_strength", 3. * sun_scale.clamp(0., 4.))?;
    data.set_item("ground", if ground { ground_dict(scene)? } else { None })?;
    let enabled = PyList::empty(py);
    if let Some(raw) = lights {
        let raw = py.import("builtins")?.call_method1("list", (raw,))?;
        for light in lights::clean_lights(&raw)?.iter() {
            if light.get_item("on")?.is_truthy()? {
                enabled.append(light)?;
            }
        }
    }
    data.set_item("lights", enabled)?;
    if serve {
        data.set_item("serve", true)?;
    }
    let kw = PyDict::new(py);
    kw.set_item("indent", 2)?;
    let json = py
        .import("json")?
        .call_method("dumps", (data,), Some(&kw))?
        .extract::<String>()?;
    let path = work.join("job.json");
    std::fs::write(&path, json)?;
    path_obj(py, &path)
}
#[pyfunction(signature=(camera,output,width,height,engine="eevee"))]
fn sync_request(
    camera: &Obj,
    output: &Obj,
    width: i32,
    height: i32,
    engine: &str,
) -> PyResult<String> {
    let py = camera.py();
    let d = PyDict::new(py);
    d.set_item("camera", camera_dict(camera)?)?;
    d.set_item("output", output.str()?)?;
    d.set_item("width", width)?;
    d.set_item("height", height)?;
    d.set_item("samples", if engine == "cycles" { 16 } else { 8 })?;
    Ok(format!(
        "{}\n",
        py.import("json")?
            .call_method1("dumps", (d,))?
            .extract::<String>()?
    ))
}
#[pyfunction]
fn camera_key<'p>(camera: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = camera.py();
    let d = camera_dict(camera)?;
    let round = py.import("builtins")?.getattr("round")?;
    let mut coords = Vec::new();
    for key in ["eye", "target", "up"] {
        for val in d.as_any().get_item(key)?.try_iter()? {
            coords.push(round.call1((val?, 3))?);
        }
    }
    Ok((
        PyTuple::new(py, coords)?,
        d.as_any().get_item("perspective")?,
        round.call1((d.as_any().get_item("fov_deg")?, 2))?,
        round.call1((d.as_any().get_item("half_height")?, 3))?,
    )
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn progress_of(line: &str) -> Option<f64> {
    static PATTERNS: LazyLock<[regex::Regex; 2]> = LazyLock::new(|| {
        [
            r"Sample (\d+)\s*/\s*(\d+)",
            r"Rendering (\d+)\s*/\s*(\d+) samples",
        ]
        .map(|p| regex::Regex::new(p).unwrap())
    });
    PATTERNS.iter().find_map(|rx| {
        let caps = rx.captures(line)?;
        let done = caps[1].parse::<f64>().ok()?;
        let total = caps[2].parse::<f64>().ok()?;
        (total > 0.).then(|| (done / total).clamp(0., 1.))
    })
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("SETTINGS_KEY", "render/blender_path")?;
    m.add("BLENDER_FLATPAK_ID", FLAT_ID)?;
    m.add("FLATPAK_PERMISSION", PERMISSION)?;
    m.add("DOWNLOAD_URL", "https://www.blender.org/download/")?;
    m.add("ENGINES", ("eevee", "cycles"))?;
    let samples = PyDict::new(py);
    for (k, [a, b, c]) in SAMPLES {
        samples.set_item(k, (a, b, c))?;
    }
    m.add("SAMPLES", samples)?;
    m.add("AMBIENCES", ("day", "night", "overcast"))?;
    m.add("EXPOSURE", EXPOSURE.into_py_dict(py)?)?;
    m.add(
        "SYNC_SAMPLES",
        [("eevee", 8), ("cycles", 16)].into_py_dict(py)?,
    )?;
    m.add("SYNC_WIDTH", 960)?;
    m.add("SYNC_REST_MS", 700)?;
    for f in [
        wrap_pyfunction!(render_script, m)?,
        wrap_pyfunction!(sun_toward, m)?,
        wrap_pyfunction!(camera_dict, m)?,
        wrap_pyfunction!(ground_dict, m)?,
        wrap_pyfunction!(write_job, m)?,
        wrap_pyfunction!(command, m)?,
        wrap_pyfunction!(work_base, m)?,
        wrap_pyfunction!(sync_request, m)?,
        wrap_pyfunction!(camera_key, m)?,
        wrap_pyfunction!(progress_of, m)?,
    ] {
        m.add_function(f)?;
    }
    discovery::install(m)?;
    lights::install(m)
}
