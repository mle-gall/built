// SPDX-License-Identifier: GPL-3.0-or-later
//! Install planar arrangements through the document's active mesh facade.
//! Scoped edits retain surviving face identity, winding, paint and curve flags.
use crate::math::{xyz, Vec3};
use crate::store::commands::{cell, Cell};
use crate::store::drawing_plane::covers;
use crate::store::geometry::{Hypot, V3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{HashMap, HashSet};
type Obj<'py> = Bound<'py, PyAny>;
type Region<'py> = (Obj<'py>, Obj<'py>, Obj<'py>);
fn rows<'py>(v: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    v.try_iter()?.collect()
}
fn points(v: &Obj<'_>) -> PyResult<Vec<DVec3>> {
    v.try_iter()?.map(|p| xyz(&p?)).collect()
}
fn v3(v: DVec3) -> V3 {
    V3(v.x, v.y, v.z)
}
fn triangles(v: &Obj<'_>) -> PyResult<Vec<[V3; 3]>> {
    v.try_iter()?
        .map(|r| {
            let r = r?;
            Ok([
                v3(xyz(&r.get_item(0)?)?),
                v3(xyz(&r.get_item(1)?)?),
                v3(xyz(&r.get_item(2)?)?),
            ])
        })
        .collect()
}
const H: Hypot = Hypot::LibStdCxx; // Drawing coverage uses binary64 vector lengths on every platform.
fn tri_probe(t: [V3; 3]) -> V3 {
    t[0].add(t[1]).add(t[2]).div(3.)
}
fn face_loops<'py>(f: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut v = vec![f.getattr("loop")?];
    v.extend(rows(&f.getattr("hole_loops")?)?);
    Ok(v)
}
fn cycle(r: &Obj<'_>, both: bool) -> PyResult<Vec<Cell>> {
    // Order signed binary64 keys numerically before choosing the first minimum.
    let mut keys: Vec<_> = points(r)?
        .into_iter()
        .map(|p| {
            cell(p.to_array()).map(|bits| {
                if bits >> 63 == 0 {
                    bits ^ (1 << 63)
                } else {
                    !bits
                }
            })
        })
        .collect();
    let rotate = |ks: &mut Vec<Cell>| {
        if let Some((i, _)) = ks.iter().enumerate().min_by_key(|(_, p)| *p) {
            ks.rotate_left(i);
        }
    };
    if both {
        let mut rev = keys.iter().rev().copied().collect();
        rotate(&mut rev);
        rotate(&mut keys);
        Ok(rev.min(keys))
    } else {
        rotate(&mut keys);
        Ok(keys)
    }
}
fn signature(r: &Obj<'_>) -> PyResult<Vec<Cell>> {
    let mut keys: Vec<_> = points(r)?.iter().map(|p| cell(p.to_array())).collect();
    keys.sort_unstable();
    keys.dedup();
    Ok(keys)
}
fn hole_cycles(r: &Obj<'_>) -> PyResult<Vec<Vec<Cell>>> {
    let mut c = r
        .try_iter()?
        .map(|h| cycle(&h?, true))
        .collect::<PyResult<Vec<_>>>()?;
    c.sort();
    Ok(c)
}
fn keep_region<'py>(
    keep: &mut Vec<Region<'py>>,
    same: &mut HashSet<u128>,
    owner: &Obj<'py>,
    normal: DVec3,
    region: &Obj<'py>,
) -> PyResult<()> {
    let mut outer = region.get_item(0)?;
    let mut holes = region.get_item(1)?;
    let r = points(&outer)?;
    let mut n = DVec3::ZERO;
    for (a, b) in r.iter().zip(r.iter().cycle().skip(1)) {
        n += DVec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    let winding = if n.length() < 1e-9 {
        DVec3::Z
    } else {
        n.normalize_or_zero()
    };
    if winding.dot(normal) < 0. {
        outer = PyList::new(outer.py(), rows(&outer)?.into_iter().rev())?.into_any();
        let rev = PyList::empty(holes.py());
        for h in holes.try_iter()? {
            rev.append(PyList::new(holes.py(), rows(&h?)?.into_iter().rev())?)?;
        }
        holes = rev.into_any();
    }
    let k = owner.getattr("key")?.extract()?;
    if !same.contains(&k)
        && !owner.getattr("interior")?.is_truthy()?
        && cycle(&owner.getattr("vertices")?, false)? == cycle(&outer, false)?
        && hole_cycles(&owner.getattr("holes")?)? == hole_cycles(&holes)?
    {
        same.insert(k);
        return Ok(());
    }
    let py = owner.py();
    keep.push((
        outer,
        if holes.is_empty()? {
            py.None().into_bound(py)
        } else {
            holes
        },
        py.get_type::<PyDict>().call1((owner.getattr("attrs")?,))?,
    ));
    Ok(())
}

#[pyfunction]
fn point_in_tri(p: &Obj<'_>, a: &Obj<'_>, b: &Obj<'_>, c: &Obj<'_>) -> PyResult<bool> {
    Ok(covers(
        [v3(xyz(a)?), v3(xyz(b)?), v3(xyz(c)?)],
        v3(xyz(p)?),
        H,
    ))
}

#[pyclass]
struct SegmentIndex(crate::instances::PointIndex);

impl SegmentIndex {
    fn build(segments: impl Iterator<Item = (DVec3, DVec3)>) -> Self {
        let (lo, hi): (Vec<_>, Vec<_>) = segments
            .map(|(a, b)| {
                let pad = DVec3::splat(4e-4 * (1. + (b - a).length()) + 4e-4);
                ((a.min(b) - pad).to_array(), (a.max(b) + pad).to_array())
            })
            .unzip();
        Self(crate::instances::PointIndex::new(&lo, &hi))
    }
}

#[pymethods]
impl SegmentIndex {
    #[new]
    fn new(segments: Vec<(Obj<'_>, Obj<'_>)>) -> PyResult<Self> {
        let segments = segments
            .iter()
            .map(|(a, b)| Ok((xyz(a)?, xyz(b)?)))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self::build(segments.into_iter()))
    }
    fn __call__(&self, point: &Obj<'_>) -> PyResult<Vec<u32>> {
        Ok(self.0.candidates(xyz(point)?.to_array()))
    }
}

struct Flags<'py> {
    entries: Vec<(DVec3, DVec3, Obj<'py>)>,
    near: SegmentIndex,
    stamp: Obj<'py>,
}
impl<'py> Flags<'py> {
    fn new(py: Python<'py>, flags: &Obj<'py>) -> PyResult<Self> {
        let entries = flags
            .extract::<Vec<(Obj, Obj, Obj)>>()?
            .into_iter()
            .map(|(a, b, f)| Ok((xyz(&a)?, xyz(&b)?, f)))
            .collect::<PyResult<Vec<_>>>()?;
        let near = SegmentIndex::build(entries.iter().map(|(a, b, _)| (*a, *b)));
        Ok(Self {
            entries,
            near,
            stamp: py.import("built_core.mesh")?.getattr("stamp_edge_flags")?,
        })
    }
    fn apply(&self, e: &Obj<'_>) -> PyResult<()> {
        let a = e.getattr("a")?;
        let b = xyz(&e.getattr("b")?)?;
        let p = xyz(&a)?;
        for i in self.near.0.candidates(p.to_array()) {
            let (v, w, ref flags) = self.entries[i as usize];
            let on = |p: DVec3| {
                let line = w - v;
                let len = line.length();
                if len < 1e-4 {
                    return cell(p.to_array()) == cell(v.to_array());
                }
                let t = (p - v).dot(line) / (len * len);
                (-1e-4..=1.0001).contains(&t) && (p - (v + line * t)).length() < 1e-4
            };
            if on(p) && on(b) {
                self.stamp.call1((e, flags))?;
                break;
            }
        }
        Ok(())
    }
}
fn install(
    mesh: &Obj<'_>,
    keep: Vec<Region<'_>>,
    flagged: &Obj<'_>,
    same: usize,
) -> PyResult<usize> {
    let flags = Flags::new(mesh.py(), flagged)?;
    let count = keep.len() + same;
    let mut restored = HashSet::<u128>::new();
    for (outer, holes, attrs) in keep {
        let f = mesh.call_method1("add_face", (outer, holes))?;
        if attrs.is_truthy()? {
            f.getattr("attrs")?.call_method1("update", (attrs,))?;
        }
        if !flags.entries.is_empty() {
            for r in face_loops(&f)? {
                let r = rows(&r)?;
                for (a, b) in r.iter().zip(r.iter().cycle().skip(1)) {
                    let e = mesh.call_method1("find_edge", (a, b))?;
                    if !e.is_none() && restored.insert(e.getattr("key")?.extract()?) {
                        flags.apply(&e)?;
                    }
                }
            }
        }
    }
    mesh.py()
        .import("built_core.topology")?
        .call_method1("prune_collinear_orphan_edges", (mesh,))?;
    mesh.call_method0("resplit_curves")?;
    Ok(count)
}

#[pyfunction]
pub(crate) fn store_regions(
    mesh: &Obj<'_>,
    regions: &Obj<'_>,
    keys: Vec<u128>,
    owners: &Obj<'_>,
    flags: &Obj<'_>,
) -> PyResult<usize> {
    let view = mesh.py().import("built_core.store_mesh")?.getattr("FaceView")?;
    let mut same = HashSet::new();
    let mut keep = Vec::new();
    for row in owners.try_iter()? {
        let row = row?;
        let face = view.call1((mesh, row.get_item(1)?))?;
        keep_region(
            &mut keep,
            &mut same,
            &face,
            xyz(&row.get_item(2)?)?,
            &regions.get_item(row.get_item(0)?)?,
        )?;
    }
    let remove: Vec<_> = keys.into_iter().filter(|k| !same.contains(k)).collect();
    let count = remove.len();
    mesh.getattr("_store")?
        .call_method1("remove_drawing_faces", (remove,))?;
    mesh.call_method1("_dirty", (count,))?;
    install(mesh, keep, flags, same.len())
}
pub(crate) fn rebuild_selected<'py>(
    mesh: &Obj<'py>, origin: &Obj<'py>, normal: &Obj<'py>,
    faces: Vec<Obj<'py>>, edges: Vec<Obj<'py>>,
) -> PyResult<Option<usize>> {
    let py = mesh.py();
    if edges.is_empty() {
        for f in faces {
            mesh.call_method1("remove_face", (f,))?;
        }
        return Ok(None);
    }
    let mesh_api = py.import("built_core.mesh")?;
    let mut flags = Vec::new();
    let mut segs = Vec::new();
    for e in edges {
        let a = Vec3::from(xyz(&e.getattr("a")?)?);
        let b = Vec3::from(xyz(&e.getattr("b")?)?);
        if !mesh_api.call_method1("edge_is_plain", (&e,))?.is_truthy()? {
            flags.push((
                a.clone(),
                b.clone(),
                mesh_api.call_method1("edge_flags", (&e,))?,
            ));
        }
        segs.push((a, b));
    }
    let result = py
        .import("built_core.arrangement")?
        .call_method1("planar_rebuild", (segs, origin, normal))?;
    let triangulate = py.import("built_core.triangulate")?.getattr("triangulate")?;
    let mut outlines = HashMap::new();
    let mut openings = HashMap::<Vec<Cell>, Vec<Vec<Cell>>>::new();
    let mut old = Vec::new();
    for f in &faces {
        let k = signature(&f.getattr("vertices")?)?;
        *outlines.entry(k.clone()).or_insert(0) += 1;
        for r in f.getattr("holes")?.try_iter()? {
            openings.entry(signature(&r?)?).or_default().push(k.clone());
        }
        old.push((
            f,
            xyz(&f.call_method0("normal")?)?,
            triangles(&f.call_method0("triangulate")?)?,
        ));
    }
    let mut same = HashSet::new();
    let mut keep = Vec::new();
    for region in result.get_item(1)?.try_iter()? {
        let region = region?;
        let outer = region.get_item(0)?;
        let holes = region.get_item(1)?;
        if holes.is_empty()? {
            let k = signature(&outer)?;
            if !outlines.contains_key(&k)
                && openings
                    .get(&k)
                    .is_some_and(|owners| !owners.iter().any(|s| outlines[s] > 1))
            {
                continue;
            }
        }
        let mut largest = -1.;
        let mut probe = None;
        for t in triangles(&triangulate.call1((&outer, &holes, normal))?)? {
            let normal = t[1].sub(t[0]).cross(t[2].sub(t[0]), H);
            let size = normal.dot(normal, H);
            if size > largest {
                largest = size;
                probe = Some(tri_probe(t));
            }
        }
        if let Some(p) = probe {
            if let Some((f, n, _)) = old
                .iter()
                .find(|(_, _, ts)| ts.iter().any(|&t| covers(t, p, H)))
            {
                keep_region(&mut keep, &mut same, f, *n, &region)?;
            }
        }
    }
    for f in faces {
        if !same.contains(&f.getattr("key")?.extract::<u128>()?) {
            mesh.call_method1("remove_face", (f,))?;
        }
    }
    Ok(Some(install(
        mesh,
        keep,
        PyList::new(py, flags)?.as_any(),
        same.len(),
    )?))
}
fn welded<'py>(mesh: &Obj<'py>, ring: &Obj<'py>) -> PyResult<Option<Vec<Obj<'py>>>> {
    let mut vertices: Vec<Obj> = Vec::new();
    for pt in ring.try_iter()? {
        let v = mesh.call_method1("vertex", (pt?,))?;
        if let Some(prev) = vertices.last() {
            if v.eq(prev)? {
                continue;
            }
        }
        vertices.push(v);
    }
    if vertices.len() > 1 && vertices[0].eq(vertices.last().unwrap())? {
        vertices.pop();
    }
    if vertices.len() < 3 {
        return Ok(None);
    }
    Ok(Some(
        vertices
            .iter()
            .map(|v| v.getattr("position"))
            .collect::<PyResult<_>>()?,
    ))
}
pub(crate) fn install_flat(
    mesh: &Obj<'_>,
    edges: &Obj<'_>,
    faces: &Obj<'_>,
    flags: &Obj<'_>,
) -> PyResult<()> {
    let flags = Flags::new(mesh.py(), flags)?;
    mesh.getattr("_store")?.call_method0("drawing_clear")?;
    mesh.getattr("_attr_own")?.call_method0("clear")?;
    mesh.call_method0("_bump")?;
    for row in edges.try_iter()? {
        let row = row?;
        let a = row.get_item(0)?;
        let b = row.get_item(1)?;
        if mesh
            .call_method1("vertex", (&a,))?
            .eq(mesh.call_method1("vertex", (&b,))?)?
        {
            continue;
        }
        let e = mesh.call_method1("add_edge", (a, b))?;
        flags.apply(&e)?;
    }
    for r in faces.try_iter()? {
        let r = r?;
        let Some(outer) = welded(mesh, &r.get_item(0)?)? else {
            continue;
        };
        let holes = PyList::empty(mesh.py());
        for h in r.get_item(1)?.try_iter()? {
            if let Some(r) = welded(mesh, &h?)? {
                holes.append(r)?;
            }
        }
        mesh.call_method1(
            "add_face",
            (
                outer,
                if holes.is_empty() {
                    mesh.py().None().into_bound(mesh.py())
                } else {
                    holes.into_any()
                },
            ),
        )?;
    }
    Ok(())
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "planar_edits")?;
    sub.add_class::<SegmentIndex>()?;
    sub.add_function(wrap_pyfunction!(point_in_tri, &sub)?)?;
    sub.add_function(wrap_pyfunction!(store_regions, &sub)?)?;
    m.add_submodule(&sub)
}
