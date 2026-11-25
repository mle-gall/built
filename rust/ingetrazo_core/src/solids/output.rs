// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use surface::Connections;
type Ring = Vec<DVec3>;
struct Polygon<'p> {
    outer: Ring,
    holes: Vec<Ring>,
    attrs: Bound<'p, PyDict>,
    originals: Obj<'p>,
}
fn ring(seq: &Obj) -> PyResult<Ring> {
    seq.try_iter()?.map(|p| xyz(&p?)).collect()
}
fn vertices<'p>(py: Python<'p>, ring: &Ring) -> PyResult<Bound<'p, PyList>> {
    PyList::new(py, ring.iter().copied().map(Vec3::from))
}
fn rounded(p: DVec3) -> [u64; 3] {
    crate::store::commands::rounded_cell::<7>(p.to_array())
}
fn simplify(polygons: &mut [Polygon]) {
    let mut straight = HashMap::new();
    for p in polygons.iter() {
        for r in std::iter::once(&p.outer).chain(&p.holes) {
            for (i, &b) in r.iter().enumerate() {
                let n = r.len();
                let incoming = b - r[(i + n - 1) % n];
                let outgoing = r[(i + 1) % n] - b;
                let aligned = incoming.cross(outgoing).length()
                    <= 1e-6 * (incoming.length() * outgoing.length()).max(1e-12)
                    && incoming.dot(outgoing) > 0.;
                *straight.entry(rounded(b)).or_insert(true) &= aligned;
            }
        }
    }
    for p in polygons {
        for r in std::iter::once(&mut p.outer).chain(&mut p.holes) {
            let keep = |x: &DVec3| !straight[&rounded(*x)];
            if r.iter().filter(|v| keep(v)).count() >= 3 {
                r.retain(keep);
            }
        }
    }
}
fn propagate(mesh: &Obj, edges: &[EdgeFlag]) -> PyResult<()> {
    if edges.is_empty() {
        return Ok(());
    }
    let tol = 1e-7
        * edges
            .iter()
            .map(|e| e.a.abs().max(e.b.abs()).max_element())
            .fold(1., f64::max);
    let mut curves = Vec::<Vec<Obj>>::new();
    let mut curve_index = HashMap::new();
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        let p = xyz(&e.getattr("a")?)?;
        let q = xyz(&e.getattr("b")?)?;
        let flag = edges.iter().find(|f| {
            let lo = f.a.min(f.b) - tol;
            let hi = f.a.max(f.b) + tol;
            if !p.min(q).cmpge(lo).all() || !p.max(q).cmple(hi).all() {
                return false;
            }
            let d = f.b - f.a;
            [p, q].into_iter().all(|v| {
                let s = ((v - f.a).dot(d) / d.length_squared().max(1e-300)).clamp(0., 1.);
                v.distance_squared(f.a + s * d) <= tol * tol
            })
        });
        if let Some(f) = flag {
            e.setattr("hidden", f.hidden)?;
            e.setattr("layer", &f.layer)?;
            if let Some(key) = f.curve {
                let i = *curve_index.entry(key).or_insert_with(|| {
                    curves.push(vec![]);
                    curves.len() - 1
                });
                curves[i].push(e);
            }
        }
    }
    let py = mesh.py();
    let next = py
        .import("ingetrazo_core.mesh")?
        .getattr("Mesh")?
        .getattr("next_curve_id")?;
    for members in curves {
        let mut chain = Connections::new(members.len());
        let ends = PyDict::new(py);
        for (i, e) in members.iter().enumerate() {
            for k in ["v0", "v1"] {
                let vertex = e.getattr(k)?;
                ends.call_method1("setdefault", (vertex, PyList::empty(py)))?
                    .call_method1("append", (i,))?;
            }
        }
        for edges in ends.values() {
            if edges.len()? == 2 {
                chain.join(edges.get_item(0)?.extract()?, edges.get_item(1)?.extract()?);
            }
        }
        let mut ids = HashMap::new();
        for (i, e) in members.iter().enumerate() {
            use std::collections::hash_map::Entry;
            let id = match ids.entry(chain.root(i)) {
                Entry::Occupied(v) => v.into_mut(),
                Entry::Vacant(v) => v.insert(next.call0()?),
            };
            e.setattr("curve", &*id)?;
        }
    }
    Ok(())
}
pub(super) fn group<'p>(
    man: &Obj<'p>,
    sources: &Sources<'p>,
    name: &Obj<'p>,
    layer: &Obj<'p>,
    outer: bool,
) -> PyResult<Option<Obj<'p>>> {
    let py = man.py();
    let result = man.call_method0("to_mesh64")?;
    let surf = Surface::read(
        &result.getattr("vert_properties")?,
        &result.getattr("tri_verts")?,
        &result.getattr("face_id")?,
    )?;
    let mask = if outer {
        surf.outer()
    } else {
        vec![true; surf.triangles.len()]
    };
    let tagged = PyList::empty(py);
    for ((tri, &owner), show) in surf.triangles.iter().zip(&surf.owners).zip(mask) {
        if !show {
            continue;
        }
        let attrs = match sources.attrs.get(owner as usize) {
            Some(a) => a.copy()?,
            None => PyDict::new(py),
        };
        attrs.set_item("_solid_src", owner)?;
        tagged.append((
            PyList::new(py, tri.map(|i| Vec3::from(surf.points[i as usize])))?,
            attrs,
        ))?;
    }
    if tagged.is_empty() {
        return Ok(None);
    }
    let fused = crate::fuse::fuse_coplanar_loops(tagged.as_any(), 0.99999, false)?;
    let mut polys = Vec::new();
    for row in fused {
        let (boundary, holes, attrs, originals): (Obj, Obj, Bound<PyDict>, Obj) = row.extract()?;
        let holes = if holes.is_none() {
            vec![]
        } else {
            holes
                .try_iter()?
                .map(|r| ring(&r?))
                .collect::<PyResult<_>>()?
        };
        polys.push(Polygon {
            outer: ring(&boundary)?,
            holes,
            attrs,
            originals,
        });
    }
    simplify(&mut polys);
    let mesh = py.import("ingetrazo_core.mesh")?.call_method0("Mesh")?;
    let ids = PyDict::new(py);
    for p in polys {
        let attrs = p.attrs.copy()?;
        let src = attrs.call_method1("pop", ("_solid_src", py.None()))?;
        let holes = PyList::empty(py);
        for h in &p.holes {
            holes.append(vertices(py, h)?)?;
        }
        let holes = if holes.is_empty() {
            none(py)
        } else {
            holes.into_any()
        };
        let made = match mesh.call_method1("add_face", (vertices(py, &p.outer)?, holes)) {
            Ok(face) => vec![face],
            Err(_) => {
                let mut made = Vec::new();
                for r in p.originals.try_iter()? {
                    if let Ok(face) = mesh.call_method1("add_face", (r?,)) {
                        made.push(face);
                    }
                }
                made
            }
        };
        for face in made {
            face.getattr("attrs")?.call_method1("update", (&attrs,))?;
            ids.set_item(face.getattr("key")?, &src)?;
        }
    }
    for edge in mesh.getattr("edges")?.try_iter()? {
        let e = edge?;
        let fs = items(&e.getattr("faces")?)?;
        if let [a, b] = &fs[..] {
            let owner = |f: &Obj| -> PyResult<Option<usize>> {
                Ok(ids
                    .get_item(f.getattr("key")?)?
                    .map(|id| id.extract::<Option<usize>>())
                    .transpose()?
                    .flatten())
            };
            if let (Some(a), Some(b)) = (owner(a)?, owner(b)?) {
                if sources.soft.contains(&order(a, b)) {
                    e.setattr("soft", true)?;
                }
            }
        }
    }
    propagate(&mesh, &sources.edges)?;
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    let group = py
        .import("core.group")?
        .getattr("Group")?
        .call((mesh,), Some(&kw))?;
    if !layer.is_none() {
        group.setattr("layer", layer)?;
    }
    Ok(Some(group))
}
