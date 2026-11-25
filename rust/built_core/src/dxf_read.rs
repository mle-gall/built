// SPDX-License-Identifier: GPL-3.0-or-later
//! DXF model-space geometry, blocks, units and layer installation.
//! The document adapter accepts ezdxf drawings, including recovered and binary files.
use crate::file_model::{group, new_mesh, verts};
use crate::file_open::{checkpoint, DocumentOpen};
use glam::{DMat4, DVec3};
use numpy::{ndarray::Array2, IntoPyArray};
use pyo3::{
    exceptions::PyZeroDivisionError,
    prelude::*,
    types::{PyDict, PyFrozenSet, PyList, PySet, PyTuple},
};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};
#[path = "dxf_ascii.rs"]
mod ascii;
#[path = "dwg_bridge.rs"]
mod dwg;
#[path = "dxf_live.rs"]
mod live;
#[path = "dxf_wire.rs"]
mod wire;

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
    sample: Vec<f64>,
    skipped: BTreeMap<String, usize>,
}
/// Explicit script access preserves the third-party CAD object when installed.
#[pyfunction]
fn dxf_open_document(py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
    DocumentOpen::stage(py, None, move |py, _| {
        match py.import("ezdxf") {
            Ok(cad) => match cad.call_method1("readfile", (&path,)) {
                Ok(doc) => return Ok(doc.unbind()),
                Err(e) if !e.is_instance_of::<pyo3::exceptions::PyException>(py) => return Err(e),
                _ => (),
            },
            Err(e) if !e.is_instance_of::<pyo3::exceptions::PyImportError>(py) => return Err(e),
            _ => (),
        }
        let source = py.detach(|| NativeDocument::open(&path, true))?;
        Ok(Py::new(
            py,
            (
                ScriptDrawing {},
                PreparedDrawing {
                    source: std::sync::Arc::new(source),
                },
            ),
        )?
        .into_any())
    })
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
    if let Ok(doc) = path.cast::<PreparedDrawing>() {
        let code = doc.borrow().source.units();
        return Ok((unit(code), code));
    }
    let doc = if path.hasattr("header")? {
        path.clone()
    } else {
        let path = path.extract::<PathBuf>()?;
        let result = DocumentOpen::stage(py, None, move |py, _| {
            let code = py.detach(|| NativeDocument::open(&path, false))?.units();
            Ok((unit(code), code).into_pyobject(py)?.into_any().unbind())
        });
        return match result {
            Ok(value) => value.extract(py),
            Err(_) => Ok((None, 0)),
        };
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
fn dxf_suggest_unit_scale(py: Python<'_>, doc: Py<PyAny>) -> PyResult<Py<PyAny>> {
    DocumentOpen::stage(py, None, move |py, _| {
        let suggestion = if let Ok(d) = doc.bind(py).cast::<PreparedDrawing>() {
            let d = d.borrow();
            (d.source.suggestion(), d.source.units())
        } else {
            suggest_unit(doc.bind(py))?
        };
        Ok(suggestion.into_pyobject(py)?.into_any().unbind())
    })
}
fn suggest_unit(doc: &Obj<'_>) -> PyResult<(Option<f64>, i64)> {
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
    Ok((infer_unit(declared, sizes), code))
}
fn infer_unit(declared: Option<f64>, mut sizes: Vec<f64>) -> Option<f64> {
    sizes.retain(|v| *v > 1e-9);
    if sizes.is_empty() {
        return declared;
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
    scale
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
fn segments(e: &Obj<'_>, kind: &str, tolerance: f64) -> PyResult<Option<(Ring, bool)>> {
    if kind == "CIRCLE" || kind == "ARC" {
        let d = e.getattr("dxf")?;
        if v(&d.getattr("extrusion")?)? == DVec3::Z {
            let (start, span) = if kind == "CIRCLE" {
                (0., -std::f64::consts::TAU)
            } else {
                let a = d.getattr("start_angle")?.extract::<f64>()?;
                let b = d.getattr("end_angle")?.extract::<f64>()?;
                if a == b {
                    return Ok(None);
                }
                let sweep = (b - a).rem_euclid(360.);
                (
                    a.to_radians(),
                    if sweep == 0. {
                        -std::f64::consts::TAU
                    } else {
                        sweep.to_radians()
                    },
                )
            };
            if d.getattr("radius")?.extract::<f64>()? == 0. {
                return Ok(None);
            }
            return Ok(ascii::arc(
                v(&d.getattr("center")?)?,
                d.getattr("radius")?.extract()?,
                start,
                span,
                tolerance,
            )
            .map(|r| (r, true)));
        }
    }
    Ok(live::geometry(e, kind, tolerance)?.runs.pop())
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
        let part = live::geometry(e, k, sagitta)?
            .parts
            .pop()
            .unwrap_or_default();
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
    inherited: Option<&str>,
    depth: usize,
    flat: bool,
    model: &mut Drawing,
    sagitta: f64,
) -> PyResult<()> {
    let k = kind(e)?;
    let own = layer(&e.getattr("dxf")?);
    // A component prototype combines its internal layers in one mesh.
    let actual = inherited.map(|parent| if own == "0" { parent } else { own.as_str() });
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
                walk(&c?, actual, depth + 1, true, model, sagitta)?;
            }
        }
        return Ok(());
    }
    let b = model
        .layers
        .entry(actual.unwrap_or("0").into())
        .or_default();
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
        walk(&e?, Some("0"), 0, false, &mut m, sagitta)?;
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
            walk(&e?, None, 0, true, &mut local, sagitta)?;
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
    let mut entities = false;
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
                entities |= section == "ENTITIES";
                if !matches!(section, "ENTITIES" | "TABLES") {
                    return None;
                }
            }
            "ENDSEC" if !section.is_empty() && fields.is_empty() => section = "",
            "EOF" if entities && section.is_empty() && next.is_none() && fields.is_empty() => {
                end = true;
                break;
            }
            _ if section == "TABLES" => (),
            "TEXT" if section == "ENTITIES" => {
                if fields.iter().any(|(code, _)| {
                    !matches!(
                        *code,
                        "8" | "10"
                            | "20"
                            | "30"
                            | "40"
                            | "1"
                            | "50"
                            | "72"
                            | "11"
                            | "21"
                            | "31"
                            | "73"
                    )
                }) {
                    return None;
                }
                *m.skipped.entry("TEXT".into()).or_default() += 1;
            }
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
                    let (start, end) = (p(0)?, p(1)?);
                    if m.sample.len() < 4000 {
                        m.sample.push(start.distance(end));
                    }
                    b.runs.push((vec![start, end], false));
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
enum NativeDocument {
    Sheet(std::sync::Arc<Drawing>),
    Cad(ascii::Document),
}
impl NativeDocument {
    fn open(path: &Path, inspect: bool) -> PyResult<Self> {
        let bytes = std::fs::read(path)?;
        let invalid = || {
            pyo3::exceptions::PyValueError::new_err(format!(
                "Unreadable DXF drawing: {}",
                path.display()
            ))
        };
        let decoded = wire::text(&bytes).ok_or_else(invalid)?;
        let text = decoded.as_ref();
        if !inspect && text.starts_with("0\nSECTION\n2\nTABLES\n") {
            if let Some(sheet) = sheet_records(text) {
                return Ok(Self::Sheet(std::sync::Arc::new(sheet)));
            }
        }
        let parsed = if inspect {
            ascii::decode_mode(text, false, true)
        } else {
            ascii::decode(text)
        };
        parsed
            .or_else(|| ascii::decode_mode(text, true, inspect))
            .map(Self::Cad)
            .ok_or_else(invalid)
    }
    fn units(&self) -> i64 {
        match self {
            Self::Sheet(_) => 6,
            Self::Cad(doc) => doc.units,
        }
    }
    fn suggestion(&self) -> Option<f64> {
        match self {
            Self::Sheet(sheet) => infer_unit(Some(1.), sheet.sample.clone()),
            Self::Cad(doc) => doc.suggestion(),
        }
    }
    fn model(&self, tolerance: f64) -> Option<std::sync::Arc<Drawing>> {
        match self {
            Self::Sheet(sheet) => Some(sheet.clone()),
            Self::Cad(doc) => doc.drawing(tolerance).map(std::sync::Arc::new),
        }
    }
}
/// An isolated, strictly convex CAD facet has no welds, crossings, shared
/// edges or straight corners for the stitching pipeline to repair. Use a
/// generous angular and model-space margin around the kernel tolerances.
fn isolated_facet(points: &[DVec3]) -> bool {
    if !(3..=4).contains(&points.len()) {
        return false;
    }
    let mut turns = Vec::with_capacity(points.len());
    for i in 0..points.len() {
        let p = points[i];
        let ab = points[(i + 1) % points.len()] - p;
        let ac = points[(i + points.len() - 1) % points.len()] - p;
        let cross = ab.cross(ac);
        if ab.length() < 0.001
            || ac.length() < 0.001
            || cross.length() <= 0.001 * ab.length().max(ac.length())
            || ab.normalize().dot(ac.normalize()).abs() >= 0.99
        {
            return false;
        }
        turns.push(cross.normalize());
    }
    turns.iter().all(|n| n.dot(turns[0]) > 0.999999)
}
fn stitch(mesh: &Obj<'_>, faces: &[Ring], scale: f64, offset: DVec3) -> PyResult<()> {
    let py = mesh.py();
    if let [ring] = faces {
        if mesh.getattr("vertices")?.len()? == 0 {
            let points = ring.iter().map(|p| *p * scale - offset).collect::<Vec<_>>();
            if isolated_facet(&points) {
                let _ = mesh.call_method1("add_face", (verts(py, &points)?,));
                return Ok(());
            }
        }
    }
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
            py.import("built_core")?
                .call_method("run_stitch", (mesh, keys, added), Some(&kw));
    }
    Ok(())
}
fn build<'p>(py: Python<'p>, b: &Bucket, scale: f64, offset: DVec3) -> PyResult<Obj<'p>> {
    let mesh = new_mesh(py)?;
    let mut pts = vec![];
    let mut endpoints = Vec::new();
    let mut curves = Vec::new();
    for (i, (run, curved)) in b.runs.iter().enumerate() {
        if i % 512 == 0 {
            checkpoint(py)?;
        }
        let curve: i64 = if *curved && run.len() > 2 {
            mesh.call_method0("next_curve_id")?.extract()?
        } else {
            -1
        };
        let base = (pts.len() / 3) as i64;
        for p in run {
            pts.extend((*p * scale - offset).to_array());
        }
        for k in 1..run.len() as i64 {
            endpoints.extend([base + k - 1, base + k]);
            curves.push(curve);
        }
    }
    if !pts.is_empty() {
        let positions = Array2::from_shape_vec((pts.len() / 3, 3), pts)
            .unwrap()
            .into_pyarray(py);
        let edges = Array2::from_shape_vec((curves.len(), 2), endpoints)
            .unwrap()
            .into_pyarray(py);
        let store_api = py.import("built_core.store_mesh")?;
        if mesh.get_type().is(&store_api.getattr("StoreMesh")?)
            && mesh
                .getattr("_store")?
                .getattr("_model_frame")?
                .is_truthy()?
        {
            // This mesh is new. Weld points and insert edges in a single store
            // construction, without allocating a temporary Python vertex view
            // for every point and rebuilding the store to append its edges.
            let live = mesh.getattr("_store")?;
            let empty = Vec::<i64>::new().into_pyarray(py);
            let curves = curves.into_pyarray(py);
            let kw = store_api
                .call_method0("_weld_args")?
                .cast_into::<PyDict>()?;
            let block = live.get_type().call_method(
                "from_block",
                (
                    live.getattr("namespace")?,
                    positions,
                    edges,
                    py.None(),
                    curves,
                    py.None(),
                    PyList::empty(py),
                    &empty,
                    &empty,
                    &empty,
                ),
                Some(&kw),
            )?;
            live.call_method1("_extend_bulk", (block,))?;
            mesh.setattr("_chunk_dirty", true)?;
            mesh.setattr("_mut_serial", mesh.getattr("_mut_serial")?.add(1)?)?;
        } else {
            let (v, ids): (Obj<'p>, Obj<'p>) =
                mesh.call_method1("bulk_weld", (positions,))?.extract()?;
            let ids = py.import("numpy")?.call_method1("asarray", (ids,))?;
            let ends = ids.get_item(edges)?.getattr("T")?;
            let flags = PyList::empty(py);
            for curve in curves {
                flags.append((false, (curve >= 0).then_some(curve), py.None()))?;
            }
            mesh.call_method1(
                "add_edges_welded",
                (v, ends.get_item(0)?, ends.get_item(1)?, flags),
            )?;
        }
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
    m: &Drawing,
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
        crate::file_bounds::survey_offset(py, lo.to_array(), hi.to_array())?
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
    for (layer, b) in &m.layers {
        let mesh = build(py, b, scale, shift)?;
        if mesh.getattr("edges")?.len()? == 0 && mesh.getattr("faces")?.len()? == 0 {
            continue;
        }
        made += outlines(&mesh, make);
        edges += mesh.getattr("edges")?.len()?;
        faces += mesh.getattr("faces")?.len()?;
        let g = group(py, &mesh, if layer == "0" { name } else { layer })?;
        tag(&g, layer)?;
        groups.append(g)?;
    }
    cb.call1((0.75, "Placing components…"))?;
    let mut protos = HashMap::new();
    let mut counts = BTreeMap::new();
    for (name, b) in &m.blocks {
        let mesh = build(py, b, scale, DVec3::ZERO)?;
        if mesh.getattr("edges")?.len()? > 0 || mesh.getattr("faces")?.len()? > 0 {
            made += outlines(&mesh, make);
            protos.insert(name.as_str(), mesh);
        }
    }
    for p in &m.placements {
        let Some(mesh) = protos.get(p.name.as_str()) else {
            continue;
        };
        let mut mat = p.m;
        mat.w_axis = (mat.w_axis.truncate() * scale - shift).extend(1.);
        let g = group(py, mesh, &p.name)?;
        g.setattr("xform", Py::new(py, crate::math::Matrix4::from(mat))?)?;
        tag(&g, &p.layer)?;
        groups.append(g)?;
        let n = counts.entry(p.name.as_str()).or_insert(0usize);
        if *n == 0 {
            edges += mesh.getattr("edges")?.len()?;
        }
        *n += 1;
    }
    if groups.is_empty() {
        return Err(PyErr::from_value(
            py.import("built_core.formats.dxf_in")?
                .call_method1("NothingToImport", (&m.skipped,))?,
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
    stats.set_item("skipped", &m.skipped)?;
    stats.set_item("offset", offset)?;
    cb.call1((1., "Done"))?;
    Ok((groups, tags, stats).into_pyobject(py)?.into_any())
}
#[pyclass(subclass)]
struct PreparedDrawing {
    source: std::sync::Arc<NativeDocument>,
}

#[pyclass]
struct CadEntity {
    kind: String,
}
#[pymethods]
impl CadEntity {
    fn dxftype(&self) -> &str {
        &self.kind
    }
}

#[pyclass]
struct CadLayouts {
    names: Vec<String>,
}
#[pymethods]
impl CadLayouts {
    fn names(&self) -> Vec<String> {
        self.names.clone()
    }
}

#[pyclass(extends=PreparedDrawing)]
struct ScriptDrawing {}
#[pymethods]
impl ScriptDrawing {
    fn modelspace(slf: PyRef<'_, Self>) -> Vec<CadEntity> {
        match slf.into_super().source.as_ref() {
            NativeDocument::Cad(doc) => doc
                .kinds
                .iter()
                .cloned()
                .map(|kind| CadEntity { kind })
                .collect(),
            _ => vec![],
        }
    }
    #[getter]
    fn layouts(slf: PyRef<'_, Self>) -> CadLayouts {
        let names = match slf.into_super().source.as_ref() {
            NativeDocument::Cad(doc) if !doc.layouts.is_empty() => doc.layouts.clone(),
            _ => vec!["Model".into(), "Layout1".into()],
        };
        CadLayouts { names }
    }
    #[getter]
    fn header<'p>(slf: PyRef<'p, Self>, py: Python<'p>) -> PyResult<Bound<'p, PyDict>> {
        let h = PyDict::new(py);
        h.set_item("$INSUNITS", slf.into_super().source.units())?;
        Ok(h)
    }
}

/// The options dialog and the import command share the same decoded drawing.
/// Keep open_document's public ezdxf API for scripts that inspect arbitrary CAD records.
#[pyfunction]
fn dxf_prepare_import(py: Python<'_>, path: PathBuf) -> PyResult<Py<PyAny>> {
    DocumentOpen::stage(py, None, move |py, _| {
        let decoded = py.detach(|| NativeDocument::open(&path, false))?;
        let suggestion = decoded.suggestion();
        let source = Py::new(
            py,
            PreparedDrawing {
                source: std::sync::Arc::new(decoded),
            },
        )?;
        Ok((source, suggestion).into_pyobject(py)?.into_any().unbind())
    })
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
        let source = doc.map(|d| d.into_bound(py));
        let tolerance = (0.01 / scale).max(1e-6);
        let model = if let Some(d) = source
            .as_ref()
            .filter(|s| s.cast::<PreparedDrawing>().is_err())
        {
            // Callers may supply a live CAD object with unsaved edits.
            cb.call1((0.2, "Collecting entities…"))?;
            std::sync::Arc::new(drawing(d, tolerance)?)
        } else {
            let cached = source
                .as_ref()
                .and_then(|s| s.cast::<PreparedDrawing>().ok())
                .map(|s| s.borrow().source.clone());
            let result = py.detach(|| -> PyResult<_> {
                let doc = match cached {
                    Some(doc) => doc,
                    None => std::sync::Arc::new(NativeDocument::open(&path, false)?),
                };
                Ok(doc.model(tolerance))
            })?;
            checkpoint(py)?;
            result.ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("Could not construct DXF geometry")
            })?
        };
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        Ok(prepare(
            py,
            &model,
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
#[pyfunction(signature=(owner, skipped=None))]
fn dxf_empty_error(owner: &Obj<'_>, skipped: Option<&Obj<'_>>) -> PyResult<()> {
    let message = concat!(
        "no importable geometry (lines, polylines, arcs, ",
        "circles, ellipses, splines or faces) in the model space"
    );
    owner
        .py()
        .get_type::<pyo3::exceptions::PyValueError>()
        .call_method1("__init__", (owner, message))?;
    let py = owner.py();
    let counts = match skipped {
        Some(value) if value.is_truthy()? => py.get_type::<PyDict>().call1((value,))?,
        _ => PyDict::new(py).into_any(),
    };
    owner.setattr("skipped", counts)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dxf_empty_error, m)?)?;
    dwg::register(m)?;
    m.add_function(wrap_pyfunction!(dxf_prepare_import, m)?)?;
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
