// SPDX-License-Identifier: GPL-3.0-or-later
//! Wavefront OBJ and MTL records, with detached preparation of editable meshes.
use crate::file_model::{dae_add_fused, group, new_mesh, place_mesh, verts};
use crate::file_open::DocumentOpen;
use glam::DVec3;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
use regex::Regex;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
type Obj<'p> = Bound<'p, PyAny>;
const CREAM: [f64; 3] = [0.96, 0.95, 0.925];

fn decode(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Ok(s.strip_prefix('\u{feff}').unwrap_or(s).to_owned());
    }
    // Undefined cp1252 slots have the replacement character, as in the legacy reader.
    const EXT: [char; 32] = [
        '€', '�', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '�', 'Ž', '�', '�', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '�', 'ž', 'Ÿ',
    ];
    Ok(bytes
        .into_iter()
        .map(|b| {
            if (128..160).contains(&b) {
                EXT[(b - 128) as usize]
            } else {
                char::from(b)
            }
        })
        .collect())
}
#[pyfunction]
fn read_obj_text(py: Python<'_>, path: PathBuf) -> PyResult<String> {
    Ok(py.detach(|| decode(&path))?)
}

fn number(token: Option<&str>) -> PyResult<f64> {
    token
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| PyValueError::new_err("Invalid OBJ coordinate"))
}
fn point(mut fields: std::str::SplitWhitespace<'_>) -> PyResult<DVec3> {
    Ok(DVec3::new(
        number(fields.next())?,
        number(fields.next())?,
        number(fields.next())?,
    ))
}
fn unit(path: &Path) -> Option<&'static str> {
    let file = std::fs::File::open(path).ok()?;
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    let mut buf = Vec::new();
    let mut stream = BufReader::new(file);
    while {
        buf.clear();
        stream.read_until(b'\n', &mut buf).ok()? != 0
    } {
        if !buf.starts_with(b"v ") {
            continue;
        }
        let mut fields = std::str::from_utf8(&buf).ok()?.split_whitespace().skip(1);
        let Some([x, y, z]) = fields
            .next()
            .zip(fields.next())
            .zip(fields.next())
            .map(|((x, y), z)| [x, y, z])
        else {
            continue;
        };
        let p = DVec3::new(x.parse().ok()?, y.parse().ok()?, z.parse().ok()?);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let span = if hi.x > lo.x {
        (hi - lo).max_element()
    } else {
        0.
    };
    Some(if span > 20000. {
        "mm"
    } else if span > 200. {
        "cm"
    } else {
        "m"
    })
}
#[pyfunction]
fn obj_suggest_unit(py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
    DocumentOpen::stage(py, None, move |py, _| {
        let unit = py.detach(|| unit(&path)).unwrap_or("m");
        Ok(unit.into_pyobject(py)?.into_any().unbind())
    })
}
fn map_path(
    path: &Path,
    mut fields: std::iter::Peekable<std::str::SplitWhitespace<'_>>,
) -> Option<PathBuf> {
    while fields.peek().is_some_and(|s| s.starts_with('-')) {
        fields.next();
        while fields.peek().is_some_and(|s| s.parse::<f64>().is_ok()) {
            fields.next();
        }
    }
    let tokens = fields.collect::<Vec<_>>().join(" ");
    let value = tokens.trim_matches('"');
    if value.is_empty() {
        return None;
    }
    let dir = path.parent()?;
    let portable = value.replace('\\', "/");
    [dir.join(value), dir.join(Path::new(&portable).file_name()?)]
        .into_iter()
        .find(|p| p.is_file())
}
#[derive(Default, Debug)]
struct Mat {
    color: Option<[f64; 3]>,
    image: Option<PathBuf>,
}
fn read_mtl(path: &Path) -> PyResult<HashMap<String, Arc<Mat>>> {
    let mut mats = HashMap::new();
    if !path.is_file() {
        return Ok(mats);
    }
    let input = decode(path)?;
    let mut entry = None;
    for line in input.lines() {
        let mut f = line.split_whitespace();
        match f.next() {
            Some("newmtl") => {
                entry = f.next().map(str::to_owned);
                if let Some(k) = entry.as_ref() {
                    mats.insert(k.clone(), Arc::new(Mat::default()));
                }
            }
            Some(tag) => {
                if let Some(m) = entry
                    .as_ref()
                    .and_then(|k| mats.get_mut(k))
                    .and_then(Arc::get_mut)
                {
                    match tag {
                        "Kd" if f.clone().count() >= 3 => m.color = Some(point(f)?.to_array()),
                        "map_Kd" => m.image = map_path(path, f.peekable()),
                        _ => (),
                    }
                }
            }
            None => (),
        }
    }
    Ok(mats)
}
#[derive(Default)]
struct Poly {
    points: Vec<DVec3>,
    uv: Option<Vec<[f64; 2]>>,
    mat: Option<Arc<Mat>>,
    smooth: u64,
}
#[derive(Default)]
struct Piece {
    name: String,
    faces: Vec<Poly>,
}
struct Model {
    pieces: Vec<Piece>,
    lo: DVec3,
    hi: DVec3,
}
fn index(raw: i64, len: usize) -> Option<usize> {
    let i = if raw > 0 {
        raw - 1
    } else {
        (len as i64).checked_add(raw)?
    };
    usize::try_from(i).ok().filter(|i| *i < len)
}
fn polygon(
    fields: std::str::SplitWhitespace<'_>,
    verts: &[DVec3],
    uvs: &[[f64; 2]],
) -> Option<Poly> {
    let mut face = Poly {
        uv: Some(Vec::new()),
        ..Default::default()
    };
    for token in fields {
        let mut parts = token.split('/');
        face.points
            .push(verts[index(parts.next()?.parse().ok()?, verts.len())?]);
        let uv = match parts.next() {
            None | Some("") => None,
            Some(s) => index(s.parse().ok()?, uvs.len()),
        };
        if let (Some(v), Some(i)) = (&mut face.uv, uv) {
            v.push(uvs[i]);
        } else {
            face.uv = None;
        }
    }
    (face.points.len() >= 3).then_some(face)
}
fn parse(path: &Path, scale: f64, matrix: Option<[f64; 9]>) -> PyResult<Model> {
    let mut model = Model {
        pieces: Vec::new(),
        lo: DVec3::splat(f64::INFINITY),
        hi: DVec3::splat(f64::NEG_INFINITY),
    };
    let mut ids = HashMap::new();
    let (mut verts, mut uvs) = (Vec::new(), Vec::new());
    let mut materials = HashMap::new();
    let mut material = None;
    let mut smooth = 0;
    let (mut object, mut group) = (String::new(), String::new());
    let mut current = None;
    for line in decode(path)?.lines() {
        let mut f = line.split_whitespace();
        match f.next() {
            Some("v") => {
                let mut p = point(f)? * scale;
                if let Some(m) = matrix {
                    p = DVec3::new(
                        DVec3::from_slice(&m).dot(p),
                        DVec3::from_slice(&m[3..]).dot(p),
                        DVec3::from_slice(&m[6..]).dot(p),
                    );
                }
                if line.starts_with("v ") {
                    model.lo = model.lo.min(p);
                    model.hi = model.hi.max(p);
                }
                verts.push(p);
            }
            Some("vt") => {
                let tuple =
                    number(f.next()).and_then(|x| Ok([x, number(Some(f.next().unwrap_or("0")))?]));
                uvs.push(tuple.unwrap_or([0.; 2]));
            }
            Some("mtllib") => {
                if let Some(name) = f.next() {
                    materials = read_mtl(&path.with_file_name(name))?
                }
            }
            Some("usemtl") => material = f.next().and_then(|k| materials.get(k)).cloned(),
            Some("s") => smooth = f.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            Some("o") => {
                object = f.collect::<Vec<_>>().join(" ");
                group.clear();
                current = None;
            }
            Some("g") => {
                group = f.collect::<Vec<_>>().join(" ");
                current = None;
            }
            Some("f") => {
                if let Some(mut poly) = polygon(f, &verts, &uvs) {
                    let id = *current.get_or_insert_with(|| {
                        *ids.entry((object.clone(), group.clone()))
                            .or_insert_with(|| {
                                let id = model.pieces.len();
                                model.pieces.push(Piece {
                                    name: if group.is_empty() {
                                        object.clone()
                                    } else {
                                        group.clone()
                                    },
                                    faces: vec![],
                                });
                                id
                            })
                    });
                    poly.mat = material.clone();
                    poly.smooth = smooth;
                    model.pieces[id].faces.push(poly);
                }
            }
            _ => (),
        }
    }
    Ok(model)
}
#[pyfunction]
fn obj_tidy_name(name: &str) -> String {
    static MESH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Z][A-Za-z]*(\.\d+)?$").unwrap());
    static COPY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\.\d{3})+$").unwrap());
    let name = name
        .rsplit_once('_')
        .filter(|(h, t)| !h.is_empty() && MESH.is_match(t))
        .map_or(name, |(h, _)| h);
    let simple = COPY.replace(name, "");
    let simple = simple.trim();
    if simple.is_empty() {
        name.into()
    } else {
        simple.into()
    }
}
fn piece_name(py: Python<'_>, s: &str, i: usize) -> PyResult<String> {
    let n = obj_tidy_name(s.trim());
    if n.chars().any(char::is_alphabetic) {
        return Ok(n);
    }
    let kw = PyDict::new(py);
    if n.is_empty() {
        kw.set_item("n", i)?
    } else {
        kw.set_item("n", n)?
    }
    py.import("built_core.i18n")?
        .call_method("tr", ("Part {n}",), Some(&kw))?
        .extract()
}
fn raw<'p>(py: Python<'p>, faces: &[Poly], shift: DVec3) -> PyResult<Bound<'p, PyList>> {
    let rows = PyList::empty(py);
    for (i, f) in faces.iter().enumerate() {
        if i % 256 == 0 {
            crate::file_open::checkpoint(py)?;
        }
        let local: Vec<_> = f.points.iter().map(|p| *p - shift).collect();
        let pts = verts(py, &local)?;
        let attrs = PyDict::new(py);
        if f.smooth != 0 {
            attrs.set_item("_s", f.smooth)?
        }
        if let Some(m) = &f.mat {
            if let Some(path) = &m.image {
                let t = PyDict::new(py);
                t.set_item("path", path.to_string_lossy())?;
                t.set_item("sw", 1.)?;
                t.set_item("sh", 1.)?;
                if let Some(uv) = &f.uv {
                    let map = crate::model::module(py, "built_core")?
                        .call_method1("texture_fit_uv_affine", (&pts, uv))?;
                    if !map.is_none() {
                        let a: [f64; 8] = map.extract()?;
                        t.set_item("uvw", map)?;
                        for (k, at) in [("sw", 0), ("sh", 4)] {
                            let len = DVec3::from_slice(&a[at..]).length();
                            if len > 1e-9 {
                                t.set_item(k, 1. / len)?
                            }
                        }
                    }
                }
                attrs.set_item("texture", t)?;
                attrs.set_item("back", true)?;
            } else if let Some(c) = m.color {
                if !c
                    .iter()
                    .zip(CREAM)
                    .all(|(v, b)| format!("{v:.4}") == format!("{b:.4}"))
                {
                    attrs.set_item("color", c.to_vec())?;
                    attrs.set_item("back", true)?;
                }
            }
        }
        rows.append((pts, (!attrs.is_empty()).then_some(attrs)))?;
    }
    Ok(rows)
}
fn editable(mesh: &Obj<'_>, rows: &Bound<'_, PyList>, error: Option<&str>) -> PyResult<()> {
    let py = mesh.py();
    let seed = PySet::empty(py)?;
    let added = PySet::empty(py)?;
    let key = py.import("built_core.topology")?.getattr("_key")?;
    for row in rows.iter() {
        let (pts, attrs): (Bound<'_, PyList>, Obj<'_>) = row.extract()?;
        if let Ok(f) = mesh.call_method1("add_face", (&pts,)) {
            if !attrs.is_none() {
                f.getattr("attrs")?.call_method1("update", (attrs,))?;
            }
            added.add(f)?;
            for p in pts.iter() {
                seed.add(key.call1((p,))?)?;
            }
        }
    }
    if let Some(error) = error.filter(|_| added.is_empty()) {
        return Err(PyValueError::new_err(error.to_owned()));
    }
    let kw = PyDict::new(py);
    kw.set_item("coplanar_merge", true)?;
    py.import("built_core")?
        .call_method("run_stitch", (mesh, seed, added), Some(&kw))?;
    py.import("built_core.orient")?
        .call_method1("orient_outward", (mesh,))?;
    let fuse = py.import("built_core.formats.fuse")?;
    fuse.call_method1("soften_by_smoothing_group", (mesh,))?;
    fuse.call_method1("drop_smoothing_groups", (mesh,))?;
    Ok(())
}
fn area(p: &Poly) -> bool {
    let mut sum = DVec3::ZERO;
    for (&a, &b) in p.points.iter().zip(p.points.iter().cycle().skip(1)) {
        sum += DVec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    sum.length_squared() > 1e-28
}
#[pyfunction]
#[pyo3(signature=(scene,path,progress=None,scale=1.,matrix=None,require_geometry=false))]
fn obj_load<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: PathBuf,
    progress: Option<&Obj<'p>>,
    scale: f64,
    matrix: Option<[f64; 9]>,
    require_geometry: bool,
) -> PyResult<Obj<'p>> {
    let dest = scene.getattr("mesh")?;
    let empty = dest.getattr("vertices")?.len()? == 0;
    let limit = py
        .import("built_core.formats.dae")?
        .getattr("_MAX_FUSE_LOOPS")?
        .extract::<usize>()?;
    let error = if require_geometry {
        let kw = PyDict::new(py);
        kw.set_item(
            "name",
            path.file_name().unwrap_or_default().to_string_lossy(),
        )?;
        Some(
            py.import("built_core.i18n")?
                .call_method("tr", ("“{name}” has no geometry.",), Some(&kw))?
                .extract::<String>()?,
        )
    } else {
        None
    };
    let message = error.clone();
    let result = DocumentOpen::stage(py, progress, move |py, cb| {
        cb.call1((0.05, "Reading file…"))?;
        let model = py.detach(|| parse(&path, scale, matrix))?;
        if let Some(e) = message
            .as_ref()
            .filter(|_| !model.pieces.iter().any(|p| p.faces.iter().any(area)))
        {
            return Err(PyValueError::new_err(e.clone()));
        }
        let offset = crate::file_bounds::survey_offset(py, model.lo.to_array(), model.hi.to_array())?;
        let shift = if offset.is_none() {
            DVec3::ZERO
        } else {
            DVec3::from_array(offset.extract()?)
        };
        let multi = model.pieces.len() > 1;
        let mut group_out = None;
        let mut mesh_out = None;
        let mut rows_out = None;
        if !multi && model.pieces.first().map_or(0, |p| p.faces.len()) <= limit {
            let rows = raw(
                py,
                model.pieces.first().map_or(&[], |p| p.faces.as_slice()),
                shift,
            )?;
            if empty {
                let m = new_mesh(py)?;
                editable(&m, &rows, message.as_deref())?;
                mesh_out = Some(m);
            } else {
                rows_out = Some(rows);
            }
        } else {
            let kids = PyList::empty(py);
            let fuse = py.import("built_core.formats.fuse")?;
            for (i, p) in model.pieces.iter().enumerate() {
                cb.call1((
                    0.5 + 0.45 * i as f64 / model.pieces.len() as f64,
                    "Building the model…",
                ))?;
                let m = new_mesh(py)?;
                let rows = raw(py, &p.faces, shift)?;
                dae_add_fused(&m, &fuse.call_method1("fuse_coplanar_loops", (rows,))?)?;
                let has_smoothing_groups = p.faces.iter().any(|f| f.smooth != 0);
                if has_smoothing_groups {
                    fuse.call_method1("soften_by_smoothing_group", (&m,))?;
                }
                fuse.call_method1("soften_smooth_edges", (&m,))?;
                if has_smoothing_groups {
                    fuse.call_method1("drop_smoothing_groups", (&m,))?;
                }
                if multi {
                    if m.getattr("faces")?.len()? == 0 {
                        continue;
                    }
                    let g = group(py, &m, &piece_name(py, &p.name, i + 1)?)?;
                    g.setattr(
                        "xform",
                        Py::new(py, crate::math::Matrix4::from(glam::DMat4::IDENTITY))?,
                    )?;
                    g.setattr("component", false)?;
                    kids.append(g)?;
                } else {
                    group_out = Some(py.import("built_core.group")?.call_method1("Group", (m,))?);
                }
            }
            if multi {
                let g = group(
                    py,
                    &new_mesh(py)?,
                    &path.file_stem().unwrap_or_default().to_string_lossy(),
                )?;
                g.call_method1("adopt", (kids,))?;
                group_out = Some(g);
            }
        }
        cb.call1((1., "Done"))?;
        Ok((offset, group_out, mesh_out, rows_out)
            .into_pyobject(py)?
            .into_any()
            .unbind())
    })?;
    let result = result.bind(py).cast::<PyTuple>()?;
    let g = result.get_item(1)?;
    let m = result.get_item(2)?;
    let r = result.get_item(3)?;
    if !g.is_none() {
        scene.getattr("groups")?.call_method1("append", (g,))?;
    }
    if !m.is_none() {
        place_mesh(py, &dest, &m)?;
    }
    if !r.is_none() {
        editable(&dest, r.cast()?, error.as_deref())?;
    }
    scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    result.get_item(0)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(obj_load, m)?)?;
    m.add_function(wrap_pyfunction!(obj_tidy_name, m)?)?;
    m.add_function(wrap_pyfunction!(obj_suggest_unit, m)?)?;
    m.add_function(wrap_pyfunction!(read_obj_text, m)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_obj_and_sidecar_retain_colors_and_face_count() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/obj/structured.obj");
        let model = parse(&path, 1., None).unwrap();
        assert_eq!(model.pieces.len(), 1);
        let faces = &model.pieces[0].faces;
        assert_eq!(faces.len(), 72);
        assert_eq!(faces[0].mat.as_ref().unwrap().color, Some([0.6; 3]));
        assert!(faces.iter().all(|f| f.points.len() == 3 && area(f)));
    }
}
