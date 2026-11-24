// SPDX-License-Identifier: GPL-3.0-or-later
//! DXF model-space geometry, blocks, units and layer installation.
//! The document adapter accepts ezdxf drawings, including recovered and binary files.
use crate::file_model::{group, new_mesh, verts};
use crate::file_open::{checkpoint, DocumentOpen};
use glam::{DMat4, DVec3};
use numpy::{ndarray::Array2, IntoPyArray};
use pyo3::{
    exceptions::{PyKeyError, PyZeroDivisionError},
    prelude::*,
    types::{PyDict, PyFrozenSet, PyList, PySet, PyTuple},
};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};
type Obj<'p> = Bound<'p, PyAny>;
type Ring = Vec<DVec3>;
#[derive(Default)]
struct Bucket {
    runs: Vec<(Ring, bool)>,
    faces: Vec<Ring>,
    parts: Vec<Vec<Ring>>,
}
struct Placement {
    name: String,
    layer: String,
    m: DMat4,
}
#[derive(Default)]
struct Drawing {
    layers: BTreeMap<String, Bucket>,
    blocks: Vec<(String, Bucket)>,
    placements: Vec<Placement>,
    entities: usize,
    skipped: BTreeMap<String, usize>,
}
fn read<'p>(py: Python<'p>, path: &Path) -> PyResult<Obj<'p>> {
    let ez = py.import("ezdxf")?;
    match ez.call_method1("readfile", (path,)) {
        Err(e) if e.value(py).is_instance(&ez.getattr("DXFStructureError")?)? => py
            .import("ezdxf.recover")?
            .call_method1("readfile", (path,))?
            .get_item(0),
        other => other,
    }
}
#[pyfunction]
fn dxf_layout_lookup<'p>(
    py: Python<'p>,
    obj: &Obj<'p>,
    original: &Obj<'p>,
    key: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    match original.call1((obj, key)) {
        Err(e) if e.is_instance_of::<PyKeyError>(py) => {
            let ty = py.import("ezdxf.lldxf.const")?.getattr("DXFKeyError")?;
            if e.value(py).is_instance(&ty)? {
                return Err(e);
            }
            let err =
                PyErr::from_value(ty.call1((format!("Layout with key \"{key}\" is not listed"),))?);
            err.set_cause(py, Some(e));
            Err(err)
        }
        other => other,
    }
}
fn document<'p>(py: Python<'p>, path: &Path) -> PyResult<Obj<'p>> {
    let first = read(py, path);
    if !matches!(&first,Err(e) if e.is_instance_of::<PyKeyError>(py)) {
        return first;
    }
    // Serialize temporary overrides during recovery. Acquire the Python lock
    // so another worker can continue executing Python while this caller waits.
    static LOCK: pyo3::sync::PyOnceLock<Py<PyAny>> = pyo3::sync::PyOnceLock::new();
    let lock = LOCK
        .get_or_try_init(py, || -> PyResult<_> {
            Ok(py.import("threading")?.call_method0("RLock")?.unbind())
        })?
        .bind(py);
    lock.call_method0("acquire")?;
    let result = (|| {
        let class = py.import("ezdxf.layouts.layouts")?.getattr("Layouts")?;
        let method = "get_layout_by_key";
        let orig = class.getattr(method)?;
        let wrapper = py.import("functools")?.call_method1(
            "partialmethod",
            (wrap_pyfunction!(dxf_layout_lookup, py)?, &orig),
        )?;
        class.setattr(method, wrapper)?;
        let retry = read(py, path);
        class.setattr(method, orig)?;
        retry
    })();
    lock.call_method0("release")?;
    result
}
#[pyfunction]
fn dxf_open_document(py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
    DocumentOpen::stage(py, None, move |py, _| Ok(document(py, &path)?.unbind()))
}
fn unit(code: i64) -> Option<f64> {
    Some(match code {
        1 => 0.0254,
        2 => 0.3048,
        3 => 1609.344,
        4 => 0.001,
        5 => 0.01,
        6 => 1.,
        7 => 1000.,
        10 => 0.9144,
        _ => return None,
    })
}
#[pyfunction]
fn dxf_read_unit_scale(path: &Obj<'_>) -> PyResult<(Option<f64>, i64)> {
    let py = path.py();
    let doc = if path.hasattr("header")? {
        path.clone()
    } else {
        match dxf_open_document(py, path.extract()?) {
            Ok(d) => d.into_bound(py),
            Err(_) => return Ok((None, 0)),
        }
    };
    let code = doc
        .getattr("header")?
        .call_method1("get", ("$INSUNITS", 0))?;
    let code = if code.is_truthy()? {
        py.import("builtins")?
            .call_method1("int", (code,))?
            .extract()?
    } else {
        0
    };
    Ok((unit(code), code))
}
fn v(p: &Obj<'_>) -> PyResult<DVec3> {
    Ok(DVec3::new(
        p.getattr("x")?.extract()?,
        p.getattr("y")?.extract()?,
        p.getattr("z")?.extract()?,
    ))
}
fn sample_sizes(e: &Obj<'_>, sizes: &mut Vec<f64>) -> PyResult<()> {
    let kind = e.call_method0("dxftype")?.extract::<String>()?;
    let d = e.getattr("dxf")?;
    match kind.as_str() {
        "LINE" => sizes.push(v(&d.getattr("start")?)?.distance(v(&d.getattr("end")?)?)),
        "CIRCLE" | "ARC" => sizes.push(2. * d.getattr("radius")?.extract::<f64>()?),
        "LWPOLYLINE" => {
            let pts = e
                .call_method1("get_points", ("xy",))?
                .extract::<Vec<[f64; 2]>>()?;
            sizes.extend(
                pts.windows(2)
                    .map(|p| (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1])),
            );
        }
        _ => (),
    }
    Ok(())
}
#[pyfunction]
fn dxf_suggest_unit_scale(doc: &Obj<'_>) -> PyResult<(Option<f64>, i64)> {
    let (declared, code) = dxf_read_unit_scale(doc)?;
    let Ok(space) = doc.call_method0("modelspace") else {
        return Ok((declared, code));
    };
    let mut sizes = vec![];
    for e in space.try_iter()? {
        if sizes.len() >= 4000 {
            break;
        }
        let _ = sample_sizes(&e?, &mut sizes);
    }
    sizes.retain(|v| *v > 1e-9);
    if sizes.is_empty() {
        return Ok((declared, code));
    }
    let mid = sizes.len() / 2;
    let median = *sizes.select_nth_unstable_by(mid, f64::total_cmp).1;
    let sane = |s: f64| (0.05..=80.).contains(&(median * s));
    let scale = declared
        .filter(|s| sane(*s))
        .or_else(|| {
            [1., 0.01, 0.001, 0.0254, 0.3048]
                .into_iter()
                .find(|s| sane(*s))
        })
        .or(declared);
    Ok((scale, code))
}
fn layer(d: &Obj<'_>) -> String {
    d.getattr("layer")
        .and_then(|v| v.extract::<String>())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0".into())
}
fn matrix(e: &Obj<'_>) -> PyResult<DMat4> {
    let m = e.call_method0("matrix44")?;
    let mut col = [[0.; 4]; 4];
    for (i, c) in col.iter_mut().enumerate() {
        *c = m.call_method1("get_row", (i,))?.extract()?;
    }
    Ok(DMat4::from_cols_array_2d(&col))
}
fn kind(e: &Obj<'_>) -> PyResult<String> {
    e.call_method0("dxftype")?.extract()
}
fn placement(e: &Obj<'_>) -> PyResult<Placement> {
    let d = e.getattr("dxf")?;
    Ok(Placement {
        name: d.getattr("name")?.extract()?,
        layer: layer(&d),
        m: matrix(e)?,
    })
}
fn expand(doc: &Obj<'_>, p: Placement, depth: usize, out: &mut Vec<Placement>) -> PyResult<()> {
    checkpoint(doc.py())?;
    if depth < 8 {
        if let Ok(block) = doc.getattr("blocks")?.get_item(&p.name) {
            let mut kids = vec![];
            let mut inserts = false;
            let mut mixed = false;
            for e in block.try_iter()? {
                let e = e?;
                match kind(&e)?.as_str() {
                    "INSERT" => {
                        inserts = true;
                        if let Ok(mut kid) = placement(&e) {
                            kid.m = p.m * kid.m;
                            if kid.layer == "0" {
                                kid.layer = p.layer.clone();
                            }
                            kids.push(kid);
                        }
                    }
                    "ATTDEF" => (),
                    _ => {
                        mixed = true;
                        break;
                    }
                }
            }
            if inserts && !mixed {
                for k in kids {
                    expand(doc, k, depth + 1, out)?;
                }
                return Ok(());
            }
        }
    }
    out.push(p);
    Ok(())
}
fn prune(points: impl IntoIterator<Item = DVec3>, close: bool) -> Ring {
    let mut out = Ring::new();
    for p in points {
        if out.last().is_none_or(|q| q.distance(p) > 1e-12) {
            out.push(p);
        }
    }
    if close && out.len() > 1 && out[0].distance(*out.last().unwrap()) <= 1e-12 {
        out.pop();
    }
    out
}
fn mesh_entity(e: &Obj<'_>, k: &str) -> PyResult<Vec<Ring>> {
    let builder = e.py().import("ezdxf.render")?.getattr("MeshBuilder")?;
    let model = builder.call_method1(
        if k == "MESH" {
            "from_mesh"
        } else {
            "from_polyface"
        },
        (e,),
    )?;
    let pts = model
        .getattr("vertices")?
        .try_iter()?
        .map(|p| v(&p?))
        .collect::<PyResult<Vec<_>>>()?;
    let mut loops = vec![];
    for f in model.getattr("faces")?.try_iter()? {
        let ids = f?.extract::<Vec<usize>>()?;
        if let Some(p) = ids
            .iter()
            .map(|&i| pts.get(i).copied())
            .collect::<Option<Vec<_>>>()
        {
            let ring = prune(p, true);
            if ring.len() >= 3 {
                loops.push(ring);
            }
        }
    }
    Ok(loops)
}
fn segments(e: &Obj<'_>, k: &str, sagitta: f64) -> PyResult<Option<(Ring, bool)>> {
    let path = e
        .py()
        .import("ezdxf.path")?
        .call_method1("make_path", (e,))?;
    let pts = path
        .call_method1("flattening", (sagitta, 8))?
        .try_iter()?
        .map(|p| v(&p?))
        .collect::<PyResult<Vec<_>>>()?;
    if pts.len() < 2 {
        return Ok(None);
    }
    let curved = match k {
        "POLYLINE" => e.getattr("vertices")?.try_iter()?.any(|v| {
            v.and_then(|v| v.getattr("dxf")?.getattr("bulge")?.extract::<f64>())
                .is_ok_and(|b| b.abs() > 1e-12)
        }),
        "LWPOLYLINE" => e
            .getattr("has_arc")
            .and_then(|v| v.is_truthy())
            .unwrap_or(false),
        _ => true,
    };
    Ok(Some((pts, curved)))
}
fn entity(
    b: &mut Bucket,
    e: &Obj<'_>,
    k: &str,
    sagitta: f64,
    skipped: &mut BTreeMap<String, usize>,
) -> PyResult<bool> {
    let d = e.getattr("dxf")?;
    if k == "MESH"
        || (k == "POLYLINE"
            && (e.getattr("is_poly_face_mesh")?.is_truthy()?
                || e.getattr("is_polygon_mesh")?.is_truthy()?))
    {
        let part = mesh_entity(e, k).unwrap_or_default();
        if part.is_empty() {
            return Ok(false);
        }
        b.parts.push(part);
        return Ok(true);
    }
    match k {
        "3DFACE" | "SOLID" | "TRACE" => {
            let p0 = v(&d.getattr("vtx0")?)?;
            let p1 = v(&d.getattr("vtx1")?)?;
            let p2 = v(&d.getattr("vtx2")?)?;
            let p3 = d.getattr("vtx3")?;
            let p3 = if p3.is_none() { p2 } else { v(&p3)? };
            let p = if k == "3DFACE" {
                vec![p0, p1, p2, p3]
            } else {
                vec![p0, p1, p3, p2]
            };
            let ring = prune(p, false);
            if ring.len() < 3 {
                return Ok(false);
            }
            b.faces.push(ring);
        }
        "LINE" => b.runs.push((
            vec![v(&d.getattr("start")?)?, v(&d.getattr("end")?)?],
            false,
        )),
        "CIRCLE" | "ARC" | "ELLIPSE" | "SPLINE" | "LWPOLYLINE" | "POLYLINE" => {
            let Ok(Some(run)) = segments(e, k, sagitta) else {
                return Ok(false);
            };
            b.runs.push(run);
        }
        _ => {
            if !matches!(k, "ATTDEF" | "VIEWPORT" | "XLINE" | "RAY") {
                *skipped.entry(k.into()).or_default() += 1;
            }
            return Ok(false);
        }
    }
    Ok(true)
}
fn walk(
    e: &Obj<'_>,
    inherited: &str,
    depth: usize,
    flat: bool,
    model: &mut Drawing,
    sagitta: f64,
) -> PyResult<()> {
    let k = kind(e)?;
    let own = layer(&e.getattr("dxf")?);
    let actual = if own == "0" { inherited } else { &own };
    if !flat && k == "INSERT" {
        if let Ok(p) = placement(e) {
            model.placements.push(p);
            model.entities += 1;
            return Ok(());
        }
    }
    if matches!(k.as_str(), "INSERT" | "MINSERT") && depth < 8 {
        if let Ok(kids) = e.call_method0("virtual_entities") {
            for c in kids.try_iter()? {
                walk(
                    &c?,
                    if inherited == "\0" { inherited } else { actual },
                    depth + 1,
                    true,
                    model,
                    sagitta,
                )?;
            }
        }
        return Ok(());
    }
    let name = if flat && inherited == "\0" {
        "\0"
    } else {
        actual
    };
    let b = model.layers.entry(name.into()).or_default();
    if entity(b, e, &k, sagitta, &mut model.skipped)? {
        model.entities += 1;
    }
    Ok(())
}
fn drawing(doc: &Obj<'_>, sagitta: f64) -> PyResult<Drawing> {
    let mut m = Drawing::default();
    for (i, e) in doc.call_method0("modelspace")?.try_iter()?.enumerate() {
        if i % 256 == 0 {
            checkpoint(doc.py())?;
        }
        walk(&e?, "0", 0, false, &mut m, sagitta)?;
    }
    let raw = std::mem::take(&mut m.placements);
    for p in raw {
        expand(doc, p, 0, &mut m.placements)?;
    }
    let mut seen = std::collections::HashSet::new();
    for p in &m.placements {
        if !seen.insert(p.name.clone()) {
            continue;
        }
        let Ok(block) = doc.getattr("blocks")?.get_item(&p.name) else {
            continue;
        };
        let mut local = Drawing::default();
        for e in block.try_iter()? {
            walk(&e?, "\0", 0, true, &mut local, sagitta)?;
        }
        let mut bucket = Bucket::default();
        for (_, mut b) in local.layers {
            bucket.runs.append(&mut b.runs);
            bucket.faces.append(&mut b.faces);
            bucket.parts.append(&mut b.parts);
        }
        for (k, n) in local.skipped {
            *m.skipped.entry(k).or_default() += n;
        }
        m.blocks.push((p.name.clone(), bucket))
    }
    Ok(m)
}
/// The headerless R12 model-space subset written by the Sheets exporter.
/// Unknown sections, codes, encodings or geometry use the CAD document reader.
fn sheet_records(text: &str) -> Option<Drawing> {
    // R12 without a code-page declaration is read as cp1252 by ezdxf.
    if !text.is_ascii() || !text.starts_with("0\nSECTION\n2\nTABLES\n") {
        return None;
    }
    let mut lines = text.lines();
    let mut m = Drawing::default();
    let mut section = "";
    let mut end = false;
    let mut pair =
        || -> Option<(&str, &str)> { Some((lines.next()?.trim(), lines.next()?.trim())) };
    let mut next = pair();
    while let Some((code, k)) = next {
        if code != "0" {
            return None;
        }
        let mut fields = Vec::new();
        loop {
            next = pair();
            match next {
                Some(("0", _)) | None => break,
                Some(v) => fields.push(v),
            }
        }
        match k {
            "SECTION" => {
                if !section.is_empty() || fields.len() != 1 || fields[0].0 != "2" {
                    return None;
                }
                section = fields[0].1;
                if !matches!(section, "ENTITIES" | "TABLES") {
                    return None;
                }
            }
            "ENDSEC" => section = "",
            "EOF" if section.is_empty() && next.is_none() => {
                end = true;
                break;
            }
            _ if section == "TABLES" => (),
            "TEXT" if section == "ENTITIES" => *m.skipped.entry("TEXT".into()).or_default() += 1,
            "LINE" | "SOLID" if section == "ENTITIES" => {
                let mut coords = [[None; 3]; 4];
                let mut layer = "0";
                for (code, val) in fields {
                    let n = code.parse::<usize>().ok()?;
                    match n {
                        8 => layer = val,
                        10..=13 | 20..=23 | 30..=33 => {
                            coords[n % 10][n / 10 - 1] = Some(val.parse::<f64>().ok()?)
                        }
                        _ => return None,
                    }
                }
                let p = |i: usize| {
                    Some(DVec3::new(
                        coords[i][0]?,
                        coords[i][1]?,
                        coords[i][2].unwrap_or(0.),
                    ))
                };
                let b = m.layers.entry(layer.into()).or_default();
                if k == "LINE" {
                    b.runs.push((vec![p(0)?, p(1)?], false));
                } else {
                    let r = prune([p(0)?, p(1)?, p(3)?, p(2)?], false);
                    if r.len() < 3 {
                        return None;
                    }
                    b.faces.push(r);
                }
                m.entities += 1;
            }
            _ => return None,
        }
    }
    end.then_some(m)
}
fn sheet_file(path: &Path) -> Option<Drawing> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut prefix = [0; 19];
    file.read_exact(&mut prefix).ok()?;
    if &prefix != b"0\nSECTION\n2\nTABLES\n" {
        return None;
    }
    let mut text = String::from_utf8(prefix.to_vec()).ok()?;
    file.read_to_string(&mut text).ok()?;
    sheet_records(&text)
}
fn stitch(mesh: &Obj<'_>, faces: &[Ring], scale: f64, offset: DVec3) -> PyResult<()> {
    let py = mesh.py();
    let added = PySet::empty(py)?;
    let keys = PySet::empty(py)?;
    let key = py.import("core.topology")?.getattr("_key")?;
    for (i, ring) in faces.iter().enumerate() {
        if i % 256 == 0 {
            checkpoint(py)?;
        }
        let points: Vec<_> = ring.iter().map(|p| *p * scale - offset).collect();
        let pts = verts(py, &points)?;
        if let Ok(f) = mesh.call_method1("add_face", (&pts,)) {
            if !f.is_none() {
                added.add(f)?;
                for p in pts.iter() {
                    keys.add(key.call1((p,))?)?;
                }
            }
        }
    }
    if !added.is_empty() {
        let kw = PyDict::new(py);
        kw.set_item("coplanar_merge", true)?;
        let _ =
            py.import("core.history")?
                .call_method("run_stitch", (mesh, keys, added), Some(&kw));
    }
    Ok(())
}
fn build<'p>(py: Python<'p>, b: &Bucket, scale: f64, offset: DVec3) -> PyResult<Obj<'p>> {
    let mesh = new_mesh(py)?;
    let mut pts = vec![];
    let mut left = vec![];
    let mut right = vec![];
    let flags = PyList::empty(py);
    for (i, (run, curved)) in b.runs.iter().enumerate() {
        if i % 512 == 0 {
            checkpoint(py)?;
        }
        let curve = if *curved && run.len() > 2 {
            Some(mesh.call_method0("next_curve_id")?)
        } else {
            None
        };
        let base = pts.len() / 3;
        for p in run {
            pts.extend((*p * scale - offset).to_array());
        }
        for k in 1..run.len() {
            left.push((base + k - 1) as i64);
            right.push((base + k) as i64);
            flags.append((false, &curve, py.None()))?;
        }
    }
    if !pts.is_empty() {
        let array = Array2::from_shape_vec((pts.len() / 3, 3), pts)
            .unwrap()
            .into_pyarray(py);
        let (v, ids): (Obj<'p>, Obj<'p>) = mesh.call_method1("bulk_weld", (array,))?.extract()?;
        let ids = py.import("numpy")?.call_method1("asarray", (ids,))?;
        mesh.call_method1(
            "add_edges_welded",
            (
                v,
                ids.get_item(left.into_pyarray(py))?,
                ids.get_item(right.into_pyarray(py))?,
                flags,
            ),
        )?;
    }
    if !b.parts.is_empty() {
        let key = py.import("core.topology")?.getattr("_key")?;
        let seen = PySet::empty(py)?;
        for rings in &b.parts {
            let part = new_mesh(py)?;
            stitch(&part, rings, scale, offset)?;
            for face in part.getattr("faces")?.try_iter()? {
                let face = face?;
                let corners = face.getattr("vertices")?;
                let keys = corners
                    .try_iter()?
                    .map(|p| key.call1((p?,)))
                    .collect::<PyResult<Vec<_>>>()?;
                let outline = PyFrozenSet::new(py, keys)?;
                if seen.contains(&outline)? {
                    continue;
                }
                seen.add(outline)?;
                let holes = face.getattr("holes")?;
                let _ =
                    mesh.call_method1("add_face", (corners, holes.is_truthy()?.then_some(holes)));
            }
        }
    }
    if !b.faces.is_empty() {
        stitch(&mesh, &b.faces, scale, offset)?;
    }
    Ok(mesh)
}
fn outlines(mesh: &Obj<'_>, make: bool) -> usize {
    if !make {
        return 0;
    }
    mesh.py()
        .import("core.make_faces")
        .and_then(|m| m.call_method1("faces_from_linework", (mesh,))?.extract())
        .unwrap_or(0)
}
fn prepare<'p>(
    py: Python<'p>,
    m: Drawing,
    name: &str,
    scale: f64,
    make: bool,
    cb: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    cb.call1((0.45, "Scaling…"))?;
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    let mut point = |p: DVec3| {
        let p = p * scale;
        lo = lo.min(p);
        hi = hi.max(p);
    };
    for b in m.layers.values() {
        for p in b
            .runs
            .iter()
            .flat_map(|r| r.0.iter())
            .chain(b.faces.iter().flatten())
            .chain(b.parts.iter().flatten().flatten())
        {
            point(*p);
        }
    }
    for p in &m.placements {
        point(p.m.w_axis.truncate());
    }
    let offset = if lo.x < hi.x {
        py.import("formats.recenter")?
            .call_method1("survey_offset", (lo.to_array(), hi.to_array()))?
    } else {
        py.None().into_bound(py)
    };
    let shift = if offset.is_none() {
        DVec3::ZERO
    } else {
        DVec3::from_array(offset.extract()?)
    };
    let groups = PyList::empty(py);
    let tags = PyList::empty(py);
    let tag = |g: &Obj<'p>, name: &str| -> PyResult<()> {
        if name != "0" {
            g.setattr("layer", name)?;
            tags.append(name)?
        }
        Ok(())
    };
    let (mut edges, mut faces, mut made) = (0, 0, 0);
    cb.call1((0.6, "Building geometry…"))?;
    for (layer, b) in m.layers {
        let mesh = build(py, &b, scale, shift)?;
        if mesh.getattr("edges")?.len()? == 0 && mesh.getattr("faces")?.len()? == 0 {
            continue;
        }
        made += outlines(&mesh, make);
        edges += mesh.getattr("edges")?.len()?;
        faces += mesh.getattr("faces")?.len()?;
        let g = group(py, &mesh, if layer == "0" { name } else { &layer })?;
        tag(&g, &layer)?;
        groups.append(g)?;
    }
    cb.call1((0.75, "Placing components…"))?;
    let mut protos = HashMap::new();
    let mut counts = BTreeMap::new();
    for (name, b) in m.blocks {
        let mesh = build(py, &b, scale, DVec3::ZERO)?;
        if mesh.getattr("edges")?.len()? > 0 || mesh.getattr("faces")?.len()? > 0 {
            made += outlines(&mesh, make);
            protos.insert(name, mesh);
        }
    }
    for p in m.placements {
        let Some(mesh) = protos.get(&p.name) else {
            continue;
        };
        let mut mat = p.m;
        mat.w_axis = (mat.w_axis.truncate() * scale - shift).extend(1.);
        let g = group(py, mesh, &p.name)?;
        g.setattr("xform", Py::new(py, crate::math::Matrix4::from(mat))?)?;
        tag(&g, &p.layer)?;
        groups.append(g)?;
        let n = counts.entry(p.name).or_insert(0usize);
        if *n == 0 {
            edges += mesh.getattr("edges")?.len()?;
        }
        *n += 1;
    }
    if groups.is_empty() {
        return Err(PyErr::from_value(
            py.import("formats.dxf_in")?
                .call_method1("NothingToImport", (m.skipped,))?,
        ));
    }
    let stats = PyDict::new(py);
    for (key, n) in [
        ("entities", m.entities),
        ("edges", edges),
        ("faces", faces),
        ("outline_faces", made),
        ("groups", groups.len()),
    ] {
        stats.set_item(key, n)?;
    }
    stats.set_item("components", counts)?;
    stats.set_item("skipped", m.skipped)?;
    stats.set_item("offset", offset)?;
    cb.call1((1., "Done"))?;
    Ok((groups, tags, stats).into_pyobject(py)?.into_any())
}
#[pyfunction]
#[pyo3(signature=(scene,path,progress=None,scale=1.,doc=None,name=None,make_faces=false))]
fn dxf_load<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: PathBuf,
    progress: Option<&Obj<'p>>,
    scale: f64,
    doc: Option<Py<PyAny>>,
    name: Option<String>,
    make_faces: bool,
) -> PyResult<Obj<'p>> {
    if scale == 0. {
        return Err(PyZeroDivisionError::new_err("DXF scale must be nonzero"));
    }
    let result = DocumentOpen::stage(py, progress, move |py, cb| {
        cb.call1((0.05, "Reading file…"))?;
        let native = if doc.is_none() {
            py.detach(|| sheet_file(&path))
        } else {
            None
        };
        let model = if let Some(m) = native {
            m
        } else {
            let d = match doc {
                Some(d) => d.into_bound(py),
                None => document(py, &path)?,
            };
            cb.call1((0.2, "Collecting entities…"))?;
            drawing(&d, (0.01 / scale).max(1e-6))?
        };
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        Ok(prepare(
            py,
            model,
            name.as_deref().filter(|s| !s.is_empty()).unwrap_or(&stem),
            scale,
            make_faces,
            &cb,
        )?
        .unbind())
    })?;
    let data = result.bind(py).cast::<PyTuple>()?;
    for t in data.get_item(1)?.try_iter()? {
        let t = t?;
        if scene.call_method1("layer", (&t,))?.is_none() {
            scene.getattr("layers")?.call_method1(
                "append",
                (py.import("core.layers")?.call_method1("Layer", (t,))?,),
            )?;
        }
    }
    scene
        .getattr("groups")?
        .call_method1("extend", (data.get_item(0)?,))?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    data.get_item(2)
}
fn collect<'p>(o: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    o.try_iter()?.collect()
}
#[pyfunction]
#[pyo3(signature=(path,groups,solids=None,texts=None))]
fn dxf_save_layers(
    path: PathBuf,
    groups: &Obj<'_>,
    solids: Option<&Obj<'_>>,
    texts: Option<&Obj<'_>>,
) -> PyResult<usize> {
    crate::sheets::dxf::sheet_dxf(
        path,
        collect(groups)?,
        solids.map(collect).transpose()?,
        texts.map(collect).transpose()?,
    )
}
#[pyfunction]
#[pyo3(signature=(path,segments,layer="VISTA"))]
fn dxf_save_lines(path: PathBuf, segments: &Obj<'_>, layer: &str) -> PyResult<usize> {
    let g = (layer, segments).into_pyobject(segments.py())?.into_any();
    crate::sheets::dxf::sheet_dxf(path, vec![g], None, None)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dxf_open_document, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_load, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_suggest_unit_scale, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_read_unit_scale, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_save_layers, m)?)?;
    m.add_function(wrap_pyfunction!(dxf_save_lines, m)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_layered_view() {
        let d = sheet_records(include_str!("../../../tests/fixtures/dxf/layered.dxf")).unwrap();
        assert_eq!(d.entities, 4);
        assert_eq!(d.layers["PLAN"].faces.len(), 1);
        assert_eq!(d.layers["AXES"].runs.len(), 1);
    }
}
