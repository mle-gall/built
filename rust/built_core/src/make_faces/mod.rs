// SPDX-License-Identifier: GPL-3.0-or-later
//! Build faces from closed planar linework. Keep edge identities when welding
//! the arrangement, and respect the winding of an existing connected solid.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
    IntoPyObjectExt,
};
use std::collections::{BTreeSet, HashMap, HashSet};
type Obj<'p> = Bound<'p, PyAny>;
type Ring = Vec<DVec3>;
type Region = (Ring, Vec<Ring>);
const TOL: f64 = 1e-3;
const WELD: f64 = 1e-4;
pyo3::create_exception!(make_faces, MakeFacesError, PyValueError);
mod topology;
fn id(x: &Obj) -> PyResult<u128> {
    x.getattr("key")?.extract()
}
fn ends(x: &Obj) -> PyResult<[DVec3; 2]> {
    Ok([xyz(&x.getattr("a")?)?, xyz(&x.getattr("b")?)?])
}
fn points(x: &Obj) -> PyResult<Ring> {
    x.try_iter()?.map(|p| xyz(&p?)).collect()
}
fn vectors(r: &Ring) -> Vec<Vec3> {
    r.iter().copied().map(Vec3::from).collect()
}
struct Cluster<'p> {
    edges: Vec<Obj<'p>>,
    o: DVec3,
    n: DVec3,
}
fn flat<'p>(edges: Vec<Obj<'p>>) -> PyResult<Cluster<'p>> {
    let mut zs = Vec::with_capacity(2 * edges.len());
    for e in &edges {
        zs.extend(ends(e)?.map(|p| p.z));
    }
    zs.sort_by(f64::total_cmp);
    Ok(Cluster {
        edges,
        o: DVec3::Z * zs[zs.len() / 2],
        n: DVec3::Z,
    })
}
fn plane(edges: &[Obj]) -> PyResult<Option<(DVec3, DVec3)>> {
    let py = edges[0].py();
    let pts = PyList::empty(py);
    for e in edges {
        for p in ends(e)? {
            pts.append(Vec3::from(p))?;
        }
    }
    crate::object_planes::coplanar_plane(pts.as_any(), TOL, None)?
        .map(|(o, n)| Ok((xyz(&o)?, DVec3::from_array(n.coords()))))
        .transpose()
}
fn clusters(edges: Vec<Obj>) -> PyResult<Vec<Cluster>> {
    if edges.is_empty() {
        return Err(MakeFacesError::new_err(
            "Select the edges of the loops to turn into faces.",
        ));
    }
    let mut leveled = vec![];
    let (mut low, mut high, mut horizontal) = (f64::INFINITY, f64::NEG_INFINITY, true);
    for e in &edges {
        let [a, b] = ends(e)?;
        low = low.min(a.z.min(b.z));
        high = high.max(a.z.max(b.z));
        horizontal &= (a.z - b.z).abs() <= TOL;
        leveled.push(((a.z + b.z) * 0.5, e.clone()));
    }
    if high - low <= TOL {
        return Ok(vec![flat(edges)?]);
    }
    if horizontal {
        leveled.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut result = vec![];
        let mut band = vec![];
        let mut last = leveled[0].0;
        for (z, e) in leveled {
            if z - last > TOL {
                result.push(flat(std::mem::take(&mut band))?);
            }
            band.push(e);
            last = z;
        }
        result.push(flat(band)?);
        return Ok(result);
    }
    let Some((o, n)) = plane(&edges)? else {
        return Err(MakeFacesError::new_err("The selected edges are not on one plane (nor on horizontal planes at different heights): Make Faces only builds flat faces."));
    };
    Ok(vec![Cluster { edges, o, n }])
}
fn regions(c: &Cluster) -> PyResult<Vec<Region>> {
    if c.edges.is_empty() {
        return Ok(vec![]);
    }
    let py = c.edges[0].py();
    let lines = PyList::empty(py);
    for e in &c.edges {
        let [a, b] = ends(e)?;
        lines.append((Vec3::from(a), Vec3::from(b)))?;
    }
    let raw = crate::store::planar_api::planar_rebuild(
        lines.as_any(),
        &Vec3::from(c.o).into_bound_py_any(py)?,
        &Vec3::from(c.n).into_bound_py_any(py)?,
    )?
    .1;
    let ring = |r: Vec<Vec3>| {
        r.into_iter()
            .map(|v| DVec3::from_array(v.coords()))
            .collect::<Ring>()
    };
    Ok(raw
        .into_iter()
        .filter(|(o, _)| o.len() >= 3)
        .map(|(o, h)| {
            (
                ring(o),
                h.into_iter().filter(|h| h.len() >= 3).map(ring).collect(),
            )
        })
        .collect())
}
#[pyfunction]
fn plan(edges: &Obj) -> PyResult<Vec<(Vec<Vec3>, Vec<Vec<Vec3>>)>> {
    let mut out = vec![];
    for c in clusters(edges.try_iter()?.collect::<PyResult<_>>()?)? {
        for (o, h) in regions(&c)? {
            out.push((vectors(&o), h.iter().map(vectors).collect()));
        }
    }
    if out.is_empty() {
        return Err(MakeFacesError::new_err("The selected edges close no loop."));
    }
    Ok(out)
}
#[pyfunction]
fn selected_edges<'p>(mesh: &Obj<'p>, selection: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let live = mesh
        .getattr("edges")?
        .try_iter()?
        .map(|e| id(&e?))
        .collect::<PyResult<HashSet<_>>>()?;
    let class = mesh.py().import("built_core.mesh")?.getattr("Edge")?;
    let mut out = Vec::new();
    for e in selection.try_iter()? {
        let e = e?;
        if e.is_instance(&class)? && live.contains(&id(&e)?) {
            out.push(e);
        }
    }
    Ok(out)
}
#[derive(Default)]
struct Report<'p> {
    faces: Vec<Obj<'p>>,
    new: usize,
    turned: usize,
    kept: usize,
    covered: usize,
}
fn face<'p>(mesh: &Obj<'p>, o: &Ring, h: &[Ring]) -> PyResult<Obj<'p>> {
    mesh.call_method1(
        "add_face",
        (
            vectors(o),
            (!h.is_empty()).then(|| h.iter().map(vectors).collect::<Vec<_>>()),
        ),
    )
}
fn orient<'p>(py: Python<'p>, o: &Ring, eye: Option<&Obj<'p>>) -> PyResult<Ring> {
    let ring = vectors(o).into_bound_py_any(py)?;
    crate::kernel::object_boundary::face_winding(&ring, eye)?
        .iter()
        .map(xyz)
        .collect()
}
fn make<'p>(
    mesh: &Obj<'p>,
    clusters: Vec<Cluster<'p>>,
    eye: Option<&Obj<'p>>,
) -> PyResult<Report<'p>> {
    let py = mesh.py();
    let mut out = Report::default();
    for mut c in clusters {
        topology::project_vertices(mesh, &c)?;
        let live = mesh
            .getattr("edges")?
            .try_iter()?
            .map(|e| id(&e?))
            .collect::<PyResult<HashSet<_>>>()?;
        let mut edges = vec![];
        for e in c.edges {
            if live.contains(&id(&e)?) {
                edges.push(e);
            }
        }
        c.edges = edges;
        let mut rebuilt = regions(&c)?;
        if rebuilt.is_empty() {
            continue;
        }
        topology::snap_and_split(mesh, &c, &mut rebuilt)?;
        let existing = topology::existing(mesh, c.o, c.n)?;
        let mut by_ring = HashMap::<_, Vec<usize>>::new();
        for (i, f) in existing.iter().enumerate() {
            by_ring
                .entry(topology::ring_key(&f.rim))
                .or_default()
                .push(i);
        }
        let mut todo = Vec::new();
        for (o, h) in rebuilt {
            let holes = topology::holes_key(&h);
            let mut same = None;
            for i in by_ring.get(&topology::ring_key(&o)).into_iter().flatten() {
                let f = &existing[*i];
                if topology::holes_key(&f.holes) == holes {
                    same = Some(f);
                    break;
                }
            }
            if let Some(f) = same {
                let want = orient(py, &o, eye)?;
                if xyz(&f.face.call_method0("normal")?)?.dot(crate::polygon::raw_newell(&want)) < 0.
                    && !topology::has_neighbours(mesh, &f.face)?
                {
                    py.import("built_core.mesh")?
                        .call_method1("turn_face_over", (&f.face,))?;
                    out.turned += 1;
                }
                out.kept += 1;
                out.faces.push(f.face.clone());
                continue;
            }
            if let Some(p) = topology::probe(py, &o, &h, c.n)? {
                if topology::covered(&existing, p)? {
                    out.covered += 1;
                } else {
                    todo.push((o, h));
                }
            }
        }
        for (o, mut h) in todo {
            let oriented = orient(py, &o, eye)?;
            if oriented != o {
                for r in &mut h {
                    r.reverse();
                }
            }
            out.faces.push(face(mesh, &oriented, &h)?);
            out.new += 1;
        }
    }
    Ok(out)
}
#[pyfunction(signature=(mesh,edges,eye=None))]
fn make_faces<'p>(
    mesh: &Obj<'p>,
    edges: &Obj<'p>,
    eye: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyDict>> {
    let r = make(
        mesh,
        clusters(edges.try_iter()?.collect::<PyResult<_>>()?)?,
        eye,
    )?;
    mesh.call_method0("resplit_curves")?;
    let d = PyDict::new(mesh.py());
    d.set_item("faces", r.faces)?;
    for (k, n) in [
        ("new", r.new),
        ("turned", r.turned),
        ("kept", r.kept),
        ("covered", r.covered),
    ] {
        d.set_item(k, n)?;
    }
    Ok(d)
}
#[pyfunction]
fn faces_from_linework(mesh: &Obj) -> PyResult<usize> {
    let mut edges = vec![];
    let mut adj = HashMap::<u128, Vec<usize>>::new();
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        if e.getattr("faces")?.is_truthy()? {
            continue;
        }
        let ids = [id(&e.getattr("v0")?)?, id(&e.getattr("v1")?)?];
        for v in ids {
            adj.entry(v).or_default().push(edges.len());
        }
        edges.push((e, ids));
    }
    let mut seen = vec![false; edges.len()];
    let mut horizontal = vec![];
    let mut tilted = vec![];
    for i in 0..edges.len() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let mut stack = vec![i];
        let mut comp = vec![];
        let mut level = vec![];
        while let Some(i) = stack.pop() {
            let (e, vs) = &edges[i];
            comp.push(e.clone());
            let [a, b] = ends(e)?;
            if (a.z - b.z).abs() <= TOL {
                level.push(e.clone());
            }
            for v in vs {
                for &j in &adj[v] {
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if comp.len() != level.len() {
            if let Some((o, n)) = plane(&comp)? {
                tilted.push(Cluster { edges: comp, o, n });
                continue;
            }
        }
        horizontal.extend(level);
    }
    let mut groups = if horizontal.is_empty() {
        vec![]
    } else {
        clusters(horizontal)?
    };
    groups.extend(tilted);
    if groups.is_empty() {
        return Ok(0);
    }
    let result = make(mesh, groups, None)?;
    mesh.call_method0("resplit_curves")?;
    Ok(result.new)
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let ex = m.py().get_type::<MakeFacesError>();
    ex.setattr("__module__", "core.make_faces")?;
    m.add("MakeFacesError", ex)?;
    m.add("PLANE_TOL", TOL)?;
    for f in [
        wrap_pyfunction!(make_faces, m)?,
        wrap_pyfunction!(plan, m)?,
        wrap_pyfunction!(selected_edges, m)?,
        wrap_pyfunction!(faces_from_linework, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
