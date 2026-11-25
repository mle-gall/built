// SPDX-License-Identifier: GPL-3.0-or-later
//! Binary and ASCII STL. Positions stay binary64 until survey recentering.
use glam::DVec3;
use numpy::{ndarray::Array2, IntoPyArray};
use pyo3::types::{PyDict, PyList, PySet};
use pyo3::{exceptions::PyValueError, prelude::*};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

fn scaled(value: f64, scale: f64) -> PyResult<f64> {
    let value = value * scale;
    if value.abs() <= f32::MAX as f64 {
        Ok(value)
    } else {
        Err(PyValueError::new_err(
            "STL contains a coordinate outside the supported range",
        ))
    }
}

fn append(vertices: &mut Vec<f64>, tri: [[f64; 3]; 3]) {
    let [a, b, c] = tri.map(DVec3::from_array);
    if (b - a).cross(c - a).length() > 1e-12 {
        vertices.extend(tri.into_iter().flatten());
    }
}

fn progress(cb: Option<&Py<PyAny>>, fraction: f64) -> PyResult<()> {
    if let Some(cb) = cb {
        Python::attach(|py| cb.call1(py, (0.05 + 0.65 * fraction, "Reading file…")))?;
    }
    Ok(())
}

fn ascii(
    mut src: BufReader<File>,
    size: u64,
    scale: f64,
    cb: Option<&Py<PyAny>>,
) -> PyResult<Vec<f64>> {
    let mut rows = Vec::new();
    let mut line = Vec::new();
    let mut step = 0;
    let mut tri = [[0.; 3]; 3];
    let mut pos = 0;
    for number in 1.. {
        line.clear();
        let bytes = src.read_until(b'\n', &mut line)?;
        if bytes == 0 {
            break;
        }
        pos += bytes;
        if number % 4096 == 0 {
            progress(cb, pos as f64 / size as f64)?;
        }
        let text = String::from_utf8_lossy(&line);
        let text = if number == 1 {
            text.trim_start_matches('\u{feff}')
        } else {
            &text
        };
        let mut words = text.split_whitespace();
        let Some(keyword) = words.next() else {
            continue;
        };
        if keyword.starts_with('#') {
            continue;
        }
        let expected = |msg| PyValueError::new_err(format!("{msg} on STL line {number}"));
        let is = |s: &str| keyword.eq_ignore_ascii_case(s);
        match step {
            0 => {
                if !is("facet") {
                    continue;
                }
                if !words
                    .next()
                    .is_some_and(|s| s.eq_ignore_ascii_case("normal"))
                {
                    return Err(expected("Invalid STL facet normal"));
                }
                for _ in 0..3 {
                    let n = words
                        .next()
                        .and_then(|s| s.parse::<f64>().ok())
                        .ok_or_else(|| expected("Invalid STL facet normal"))?;
                    if !n.is_finite() {
                        return Err(expected("Non-finite STL facet normal"));
                    }
                }
            }
            1 if is("outer") && words.next().is_some_and(|s| s.eq_ignore_ascii_case("loop")) => {}
            1 => return Err(expected("Expected 'outer loop'")),
            2..=4 => {
                if !is("vertex") {
                    return Err(expected("Expected an STL vertex"));
                }
                for c in &mut tri[step - 2] {
                    let v = words
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| expected("Invalid STL vertex"))?;
                    *c = scaled(v, scale)?;
                }
            }
            5 if !is("endloop") => return Err(expected("Expected 'endloop'")),
            6 if !is("endfacet") => return Err(expected("Expected 'endfacet'")),
            6 => append(&mut rows, tri),
            _ => {}
        }
        if words.next().is_some() {
            return Err(expected("Unexpected text"));
        }
        step = (step + 1) % 7;
    }
    if step != 0 {
        return Err(PyValueError::new_err("STL ASCII facet is incomplete"));
    }
    Ok(rows)
}

fn read(path: PathBuf, scale: f64, cb: Option<&Py<PyAny>>) -> PyResult<Vec<f64>> {
    let mut src = BufReader::new(File::open(path)?);
    let size = src.get_ref().metadata()?.len();
    if size >= 84 {
        let mut header = [0; 84];
        src.read_exact(&mut header)?;
        let count = u32::from_le_bytes(header[80..84].try_into().unwrap());
        let expected = 84 + u64::from(count) * 50;
        if size == expected || (size > expected && !header[..5].eq_ignore_ascii_case(b"solid")) {
            let mut rows = Vec::new();
            let mut record = [0; 50];
            for i in 0..count {
                src.read_exact(&mut record)?;
                let mut tri = [[0.; 3]; 3];
                for (out, bytes) in tri.iter_mut().flatten().zip(record[12..48].chunks_exact(4)) {
                    *out = scaled(f32::from_le_bytes(bytes.try_into().unwrap()) as f64, scale)?;
                }
                append(&mut rows, tri);
                if i % (count / 100).max(1) == 0 {
                    progress(cb, f64::from(i + 1) / f64::from(count))?;
                }
            }
            return Ok(rows);
        }
        src.rewind()?;
    }
    ascii(src, size, scale, cb)
}

fn write(path: PathBuf, triangles: &Bound<'_, PyAny>, scale: f64) -> PyResult<()> {
    let mut output = crate::atomic_document::AtomicDocument::create(path)?;
    let mut dst = BufWriter::new(output.output()?);
    let mut header = [0; 84];
    let label = b"IngeTrazo STL export";
    header[..label.len()].copy_from_slice(label);
    dst.write_all(&header)?;
    let mut count = 0u32;
    for tri in triangles.try_iter()? {
        let tri = tri?;
        let a = crate::math::xyz(&tri.get_item(0)?)?;
        let b = crate::math::xyz(&tri.get_item(1)?)?;
        let c = crate::math::xyz(&tri.get_item(2)?)?;
        let normal = (b - a).cross(c - a);
        let len = normal.length();
        let n = if len > 1e-12 {
            normal / len
        } else {
            DVec3::ZERO
        };
        for v in n.to_array() {
            dst.write_all(&(v as f32).to_le_bytes())?;
        }
        for v in [a, b, c].into_iter().flat_map(|p| p.to_array()) {
            dst.write_all(&(scaled(v, scale)? as f32).to_le_bytes())?;
        }
        dst.write_all(&[0; 2])?;
        count = count
            .checked_add(1)
            .ok_or_else(|| PyValueError::new_err("Too many STL triangles"))?;
    }
    dst.seek(SeekFrom::Start(80))?;
    dst.write_all(&count.to_le_bytes())?;
    dst.flush()?;
    drop(dst);
    output.commit()?;
    Ok(())
}

fn threshold(py: Python<'_>) -> PyResult<usize> {
    py.import("formats.dae")?
        .getattr("_MAX_FUSE_LOOPS")?
        .extract()
}

#[pyfunction]
#[pyo3(signature=(path,progress=None,scale=1.0,simplify_mode="none",report=None))]
fn parse_stl<'py>(
    py: Python<'py>,
    path: PathBuf,
    progress: Option<&Bound<'py, PyAny>>,
    scale: f64,
    simplify_mode: &str,
    report: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyAny>> {
    if !scale.is_finite() || scale <= 0. {
        return Err(PyValueError::new_err(
            "STL unit scale must be a positive finite number",
        ));
    }
    if !matches!(simplify_mode, "none" | "principal" | "all") {
        return Err(PyValueError::new_err(
            "STL simplification mode must be none, principal, or all",
        ));
    }
    let tick = |f, message: &str| -> PyResult<()> {
        if let Some(cb) = progress {
            cb.call1((f, message))?;
        }
        Ok(())
    };
    tick(0.02, "Reading file…")?;
    let cb = progress.cloned().map(Bound::unbind);
    let (mut rows, lo, hi) = py.detach(move || -> PyResult<_> {
        let rows = read(path, scale, cb.as_ref())?;
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for point in rows.chunks_exact(3) {
            for i in 0..3 {
                lo[i] = lo[i].min(point[i]);
                hi[i] = hi[i].max(point[i]);
            }
        }
        Ok((rows, lo, hi))
    })?;
    let offset = crate::file_bounds::survey_offset(py, lo, hi)?;
    if let Some(report) = report {
        report.set_item("offset", &offset)?;
    }
    if !offset.is_none() {
        let shift: [f64; 3] = offset.extract()?;
        py.detach(|| {
            for xyz in rows.chunks_exact_mut(3) {
                for i in 0..3 {
                    xyz[i] -= shift[i];
                }
            }
        });
    }
    let mesh = py.import("core.mesh")?.getattr("Mesh")?.call0()?;
    for chunk in rows.chunks(4096 * 9) {
        let triangles = chunk.len() / 9;
        let vertices = Array2::from_shape_vec((triangles * 3, 3), chunk.to_vec())
            .unwrap()
            .into_pyarray(py);
        mesh.call_method1(
            "add_faces_bulk",
            (
                vertices,
                vec![3i64; triangles].into_pyarray(py),
                vec![1i64; triangles].into_pyarray(py),
            ),
        )?;
        py.check_signals()?;
    }
    drop(rows);
    if mesh.getattr("faces")?.len()? == 0 {
        return Err(PyValueError::new_err(
            "STL file contains no usable triangles",
        ));
    }
    let kwargs = PyDict::new(py);
    if simplify_mode != "none" {
        tick(0.75, "Merging flat surfaces…")?;
        kwargs.set_item("principal_planes", simplify_mode == "principal")?;
        py.import("formats.fuse")?
            .call_method("simplify_mesh", (&mesh,), Some(&kwargs))?;
        kwargs.clear();
    }
    let faces = mesh.getattr("faces")?;
    if faces.len()? <= threshold(py)? {
        tick(0.85, "Joining triangles…")?;
        if simplify_mode == "none" {
            let key = py.import("core.topology")?.getattr("_key")?;
            let seed = PySet::empty(py)?;
            let new_faces = PySet::empty(py)?;
            for face in faces.try_iter()? {
                let face = face?;
                for point in face.getattr("vertices")?.try_iter()? {
                    seed.add(key.call1((point?,))?)?;
                }
                new_faces.add(face)?;
            }
            kwargs.set_item("coplanar_merge", true)?;
            py.import("ingetrazo_core")?.call_method(
                "run_stitch",
                (&mesh, seed, &new_faces),
                Some(&kwargs),
            )?;
            kwargs.clear();
            kwargs.set_item("only", new_faces)?;
        }
        py.import("core.orient")?
            .call_method("orient_outward", (&mesh,), Some(&kwargs))?;
    }
    tick(1.0, "Done")?;
    Ok(mesh)
}

#[pyfunction]
fn add_stl_mesh(
    scene: &Bound<'_, PyAny>,
    path: PathBuf,
    target: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = scene.py();
    let faces = target.getattr("faces")?;
    if faces.len()? > threshold(py)? {
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        let group = py
            .import("core.group")?
            .getattr("Group")?
            .call1((target, name.as_ref()))?;
        scene.getattr("groups")?.call_method1("append", (group,))?;
    } else {
        let mesh = scene.getattr("mesh")?;
        for face in faces.try_iter()? {
            let face = face?;
            mesh.call_method1(
                "add_face",
                (face.getattr("vertices")?, face.getattr("holes")?),
            )?;
        }
    }
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)
}

#[pyfunction]
#[pyo3(signature=(scene,path,progress=None,scale=1.0,simplify_mode="none"))]
fn load_stl<'py>(
    scene: &Bound<'py, PyAny>,
    path: PathBuf,
    progress: Option<&Bound<'py, PyAny>>,
    scale: f64,
    simplify_mode: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    let report = PyDict::new(py);
    let target = parse_stl(
        py,
        path.clone(),
        progress,
        scale,
        simplify_mode,
        Some(&report),
    )?;
    add_stl_mesh(scene, path, &target)?;
    report
        .get_item("offset")?
        .ok_or_else(|| PyValueError::new_err("Missing STL bounds"))
}

fn faces<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    if !scene.hasattr("render_faces")? {
        return scene
            .getattr("mesh")
            .unwrap_or_else(|_| scene.clone())
            .getattr("faces");
    }
    let groups = scene
        .getattr("groups")
        .unwrap_or_else(|_| PyList::empty(py).into_any());
    let mut transformed = false;
    for group in groups.try_iter()? {
        if group?.getattr("xform").is_ok_and(|m| !m.is_none()) {
            transformed = true;
            break;
        }
    }
    if !transformed {
        return scene.call_method0("render_faces");
    }
    let batches = PyList::empty(py);
    batches.append(py.import("builtins")?.call_method1(
        "filter",
        (
            scene.getattr("entity_visible")?,
            scene.getattr("loose_mesh")?.getattr("faces")?,
        ),
    )?)?;
    let world_mesh = py.import("core.group")?.getattr("world_mesh")?;
    for group in groups.try_iter()? {
        let group = group?;
        if scene
            .call_method1("entity_visible", (&group,))?
            .is_truthy()?
            && !group
                .getattr("billboard")
                .map_or(Ok(false), |v| v.is_truthy())?
        {
            batches.append(world_mesh.call1((group,))?.getattr("faces")?)?;
        }
    }
    py.import("itertools")?
        .getattr("chain")?
        .call_method1("from_iterable", (batches,))
}

#[pyfunction]
fn iter_triangles<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    let triangles = py
        .import("formats.meshexport")?
        .getattr("export_triangles")?;
    let mapped = py
        .import("builtins")?
        .call_method1("map", (triangles, faces(scene)?))?;
    py.import("itertools")?
        .getattr("chain")?
        .call_method1("from_iterable", (mapped,))
}

const UNITS: [(&str, f64); 5] = [
    ("m", 1.),
    ("cm", 0.01),
    ("mm", 0.001),
    ("in", 0.0254),
    ("ft", 0.3048),
];

#[pyfunction]
#[pyo3(signature=(scene,path,unit="m"))]
fn save_stl(scene: &Bound<'_, PyAny>, path: PathBuf, unit: &str) -> PyResult<()> {
    let factor = UNITS
        .iter()
        .find(|(u, _)| *u == unit)
        .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(unit.to_owned()))?
        .1;
    write(path, &iter_triangles(scene)?, 1. / factor)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let units = PyDict::new(m.py());
    for (unit, value) in UNITS {
        units.set_item(unit, value)?;
    }
    m.add("STL_UNITS", units)?;
    m.add_function(wrap_pyfunction!(parse_stl, m)?)?;
    m.add_function(wrap_pyfunction!(parse_stl_threaded, m)?)?;
    m.add_function(wrap_pyfunction!(load_stl, m)?)?;
    m.add_function(wrap_pyfunction!(add_stl_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(iter_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(save_stl, m)?)?;
    Ok(())
}

/// The native shell polls this worker through its modal event pump. Only the
/// worker owns the mesh until the shell accepts the completed result.
#[pyclass(unsendable)]
struct PollStl {
    worker: crate::file_open::DocumentOpen,
    progress: Py<PyAny>,
    last: Option<(f64, String)>,
}

#[pymethods]
impl PollStl {
    fn __call__(&mut self, py: Python<'_>) -> PyResult<bool> {
        let value = self.worker.progress();
        if !value.1.is_empty() && self.last.as_ref() != Some(&value) {
            self.progress.call1(py, value.clone())?;
            self.last = Some(value);
        }
        Ok(self.worker.ready())
    }
}

#[pyfunction]
#[pyo3(signature=(path,scale,simplify_mode,cb,report=None))]
fn parse_stl_threaded(
    py: Python<'_>,
    path: PathBuf,
    scale: f64,
    simplify_mode: String,
    cb: Py<PyAny>,
    report: Option<Py<PyDict>>,
) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
    let worker = crate::file_open::DocumentOpen::parse(move |py, progress| {
        Ok(parse_stl(
            py,
            path,
            Some(&progress),
            scale,
            &simplify_mode,
            report.as_ref().map(|r| r.bind(py)),
        )?
        .unbind())
    })?;
    let poll = Py::new(
        py,
        PollStl {
            worker,
            progress: cb,
            last: None,
        },
    )?;
    py.import("ingetrazo_render.shell_modal")?
        .call_method1("run_until", (&poll,))?;
    let result = poll.borrow(py).worker.wait(py);
    poll.borrow_mut(py).__call__(py)?;
    match result {
        Ok(mesh) => Ok((mesh, py.None())),
        Err(error) => Ok((py.None(), error.into_value(py).into_any())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn angled_triangle_matches_reference_writer() {
        const GOLDEN: &[u8] = include_bytes!("../../../tests/fixtures/stl/angled-mm.stl");
        let path =
            std::env::temp_dir().join(format!("built-stl-golden-{}.stl", std::process::id()));
        let tri = [[-1.125, 0.01, 4.0], [3.0, -2.25, 3.0], [1.0, 3.0, 0.125]];
        Python::initialize();
        Python::attach(|py| {
            let triangles = PyList::new(py, [tri]).unwrap();
            write(path.clone(), &triangles, 1000.).unwrap();
        });
        let actual = std::fs::read(&path).unwrap();
        let points = read(path.clone(), 0.001, None).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(actual, GOLDEN);
        assert_eq!(points, tri.concat());
    }
}
