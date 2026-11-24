// SPDX-License-Identifier: GPL-3.0-or-later
//! Recover planar polygon regions from imported triangle soups.
use crate::file_model::{assign, dae_add_fused, new_mesh, verts};
use crate::file_open::checkpoint;
use glam::DVec3;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
use std::collections::HashMap;
type Obj<'p> = Bound<'p, PyAny>;
type Vertex = [u64; 3];
type Edge = [Vertex; 2];
fn rounded(v: f64, digits: usize) -> f64 {
    if v.fract() == 0. {
        v
    } else {
        format!("{v:.digits$}").parse().unwrap_or(v)
    }
}
fn key(v: DVec3) -> Vertex {
    v.to_array().map(|c| {
        let n = rounded(c, 4);
        if n == 0. {
            0
        } else {
            n.to_bits()
        }
    })
}
fn edge(a: Vertex, b: Vertex) -> Edge {
    if a < b {
        [a, b]
    } else {
        [b, a]
    }
}
fn normal(pts: &[DVec3]) -> DVec3 {
    let mut n = DVec3::ZERO;
    for (&a, &b) in pts.iter().zip(pts.iter().cycle().skip(1)) {
        n += DVec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    n
}
fn get<'p>(o: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    o.call_method1("get", (k,))
}
fn tuple<'p>(o: &Obj<'p>) -> PyResult<Obj<'p>> {
    o.py().get_type::<PyTuple>().call1((o,))
}
fn freeze<'p>(o: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = o.py();
    if let Ok(d) = o.cast::<PyDict>() {
        let pairs = PyList::empty(py);
        for (k, v) in d.iter() {
            pairs.append((k, freeze(&v)?))?;
        }
        pairs.call_method0("sort")?;
        return tuple(pairs.as_any());
    }
    if o.is_instance_of::<PyList>() || o.is_instance_of::<PyTuple>() || o.is_instance_of::<PySet>()
    {
        let rows = PyList::empty(py);
        for v in o.try_iter()? {
            rows.append(freeze(&v?)?)?;
        }
        if o.is_instance_of::<PySet>() {
            let kw = PyDict::new(py);
            kw.set_item("key", py.import("builtins")?.getattr("repr")?)?;
            rows.call_method("sort", (), Some(&kw))?;
        }
        return tuple(rows.as_any());
    }
    if o.hash().is_err() {
        Ok(o.repr()?.into_any())
    } else {
        Ok(o.clone())
    }
}
struct Style<'p> {
    color: Obj<'p>,
    texture: Obj<'p>,
    extras: Obj<'p>,
    uv: Vec<f64>,
}
impl<'p> Style<'p> {
    fn from(attrs: &Obj<'p>) -> PyResult<Self> {
        let py = attrs.py();
        let none = || py.None().into_bound(py);
        let mut s = Self {
            color: none(),
            texture: none(),
            extras: PyTuple::empty(py).into_any(),
            uv: vec![],
        };
        if !attrs.is_truthy()? {
            return Ok(s);
        }
        let c = get(attrs, "color")?;
        if !c.is_none() {
            s.color = tuple(&c)?;
        }
        let tex = get(attrs, "texture")?;
        if tex.is_truthy()? {
            let uv = get(&tex, "uvw")?;
            let values = if uv.is_truthy()? {
                s.uv = uv
                    .extract::<Vec<f64>>()?
                    .into_iter()
                    .map(|n| rounded(n, 4))
                    .collect();
                Some(PyTuple::new(py, &s.uv)?)
            } else {
                None
            };
            s.texture = (
                get(&tex, "path")?,
                get(&tex, "sw")?,
                get(&tex, "sh")?,
                tex.call_method1("get", ("rot", 0))?,
                values,
            )
                .into_pyobject(py)?
                .into_any();
        }
        let d = attrs.cast::<PyDict>()?;
        let extra = PyList::empty(py);
        for (k, v) in d.iter() {
            if !k.eq("color")? && !k.eq("texture")? {
                extra.append((k, freeze(&v)?))?;
            }
        }
        extra.call_method0("sort")?;
        s.extras = tuple(extra.as_any())?;
        Ok(s)
    }
    fn compatible(&self, b: &Self) -> PyResult<bool> {
        if !self.color.eq(&b.color)? || !self.extras.eq(&b.extras)? {
            return Ok(false);
        }
        if self.texture.eq(&b.texture)? {
            return Ok(true);
        }
        Ok(!self.uv.is_empty()
            && !b.uv.is_empty()
            && self.texture.get_item(0)?.eq(b.texture.get_item(0)?)?
            && self
                .uv
                .iter()
                .zip(&b.uv)
                .all(|(x, y)| (x - y).abs() <= 0.02 + 0.01 * x.abs().max(y.abs())))
    }
}
struct Face<'p> {
    pts: Obj<'p>,
    attrs: Obj<'p>,
    style: Style<'p>,
    points: Vec<DVec3>,
    keys: Vec<Vertex>,
    normal: DVec3,
}
fn find(parent: &mut [usize], mut i: usize) -> usize {
    while i != parent[i] {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}
struct EdgeUse {
    key: Edge,
    faces: Vec<usize>,
}
fn edges(faces: &[Face<'_>], ids: impl IntoIterator<Item = usize>) -> Vec<EdgeUse> {
    let mut index = HashMap::new();
    let mut all: Vec<EdgeUse> = vec![];
    for i in ids {
        let keys = &faces[i].keys;
        for (&a, &b) in keys.iter().zip(keys.iter().cycle().skip(1)) {
            if a == b {
                continue;
            }
            let k = edge(a, b);
            let id = *index.entry(k).or_insert_with(|| {
                let id = all.len();
                all.push(EdgeUse {
                    key: k,
                    faces: vec![],
                });
                id
            });
            all[id].faces.push(i);
        }
    }
    all
}
// Trace a degree-two boundary. Keep first-encounter order for ties and
// orient the outer contour and holes against the region's original normal.
fn boundary(faces: &[Face<'_>], region: &[usize]) -> Option<Vec<Vec<DVec3>>> {
    let uses = edges(faces, region.iter().copied());
    let mut positions = HashMap::new();
    for &i in region {
        for (&key, &p) in faces[i].keys.iter().zip(&faces[i].points) {
            positions.entry(key).or_insert(p);
        }
    }
    let mut boundary = vec![];
    let mut adj = HashMap::<Vertex, Vec<(Vertex, usize)>>::new();
    for u in uses {
        if u.faces.len() > 2 {
            return None;
        }
        if u.faces.len() != 1 {
            continue;
        }
        let [a, b] = u.key;
        let id = boundary.len();
        boundary.push(u.key);
        adj.entry(a).or_default().push((b, id));
        adj.entry(b).or_default().push((a, id));
    }
    if boundary.is_empty() || adj.values().any(|ns| ns.len() != 2) {
        return None;
    }
    let mut closed = Vec::new();
    let mut used = vec![false; boundary.len()];
    for (i, &[a, b]) in boundary.iter().enumerate() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut ring = vec![positions[&a]];
        let mut at = b;
        while at != a {
            ring.push(positions[&at]);
            let &(next, edge) = adj[&at].iter().find(|(_, e)| !used[*e])?;
            used[edge] = true;
            at = next;
        }
        if ring.len() < 3 {
            return None;
        }
        closed.push((normal(&ring), ring));
    }
    closed.sort_by(|a, b| b.0.length_squared().total_cmp(&a.0.length_squared()));
    let reference = faces[region[0]].normal;
    Some(
        closed
            .into_iter()
            .enumerate()
            .map(|(i, (n, mut ring))| {
                if (i == 0 && n.dot(reference) < 0.) || (i != 0 && n.dot(reference) > 0.) {
                    ring.reverse();
                }
                ring
            })
            .collect(),
    )
}
#[pyfunction]
#[pyo3(signature=(loops,cos_tol=0.99999,principal_planes=false))]
fn fuse_coplanar_loops<'p>(
    loops: &Obj<'p>,
    cos_tol: f64,
    principal_planes: bool,
) -> PyResult<Bound<'p, PyList>> {
    let py = loops.py();
    let mut faces: Vec<Face> = vec![];
    let mut triangles = HashMap::<[Vertex; 3], usize>::new();
    for (i, row) in loops.try_iter()?.enumerate() {
        if i % 512 == 0 {
            checkpoint(py)?;
        }
        let (pts, attrs): (Obj<'p>, Obj<'p>) = row?.extract()?;
        if pts.len()? < 3 {
            continue;
        }
        let points = pts
            .try_iter()?
            .map(|p| crate::math::xyz(&p?))
            .collect::<PyResult<Vec<_>>>()?;
        let n = normal(&points);
        let len = n.length();
        if len < 1e-10 {
            continue;
        }
        let keys = points.iter().map(|p| key(*p)).collect();
        let style = Style::from(&attrs)?;
        let face = Face {
            pts,
            attrs,
            normal: n / len,
            points,
            keys,
            style,
        };
        if face.keys.len() == 3 {
            let mut tri = [face.keys[0], face.keys[1], face.keys[2]];
            tri.sort_unstable();
            if let Some(&i) = triangles.get(&tri) {
                if !face.style.texture.is_none() && faces[i].style.texture.is_none() {
                    faces[i] = face;
                }
                continue;
            }
            triangles.insert(tri, faces.len());
        }
        faces.push(face);
    }
    let mut parent = (0..faces.len()).collect::<Vec<_>>();
    for u in edges(&faces, 0..faces.len()) {
        let [i, j] = u.faces.as_slice() else { continue };
        let (i, j) = (*i, *j);
        let (a, b) = (&faces[i], &faces[j]);
        if principal_planes
            && [a.normal, b.normal]
                .iter()
                .any(|n| n.abs().max_element() < 1. - 1e-7)
        {
            continue;
        }
        if a.normal.dot(b.normal).abs() < cos_tol || !a.style.compatible(&b.style)? {
            continue;
        }
        let (i, j) = (find(&mut parent, i), find(&mut parent, j));
        if i != j
            && faces[i].normal.dot(faces[j].normal).abs() >= cos_tol
            && faces[i].style.compatible(&faces[j].style)?
        {
            parent[j] = i;
        }
    }
    let mut regions = Vec::<Vec<usize>>::new();
    let mut idx = HashMap::new();
    for i in 0..faces.len() {
        let root = find(&mut parent, i);
        let id = *idx.entry(root).or_insert_with(|| {
            regions.push(vec![]);
            regions.len() - 1
        });
        regions[id].push(i);
    }
    let out = PyList::empty(py);
    for (i, r) in regions.iter().enumerate() {
        if i % 512 == 0 {
            checkpoint(py)?;
        }
        if r.len() > 1 {
            if let Some(rings) = boundary(&faces, r) {
                let holes = PyList::empty(py);
                for h in &rings[1..] {
                    holes.append(verts(py, h)?)?;
                }
                let originals = PyList::new(py, r.iter().map(|&f| &faces[f].pts))?;
                out.append((verts(py, &rings[0])?, holes, &faces[r[0]].attrs, originals))?;
                continue;
            }
        }
        for &i in r {
            let f = &faces[i];
            out.append((
                &f.pts,
                PyList::empty(py),
                &f.attrs,
                PyList::new(py, [&f.pts])?,
            ))?;
        }
    }
    Ok(out)
}
#[pyfunction]
fn fuse_smooth_sig<'p>(attrs: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = attrs.py();
    if !attrs.is_truthy()? {
        return Ok(py.None().into_bound(py));
    }
    let c = get(attrs, "color")?;
    let t = get(attrs, "texture")?;
    Ok((
        if c.is_none() { c } else { tuple(&c)? },
        if t.is_truthy()? {
            get(&t, "path")?
        } else {
            py.None().into_bound(py)
        },
    )
        .into_pyobject(py)?
        .into_any())
}
fn soften(mesh: &Obj<'_>, cos: f64, groups: bool) -> PyResult<()> {
    let py = mesh.py();
    let cache = PyDict::new(py);
    for (i, e) in mesh.getattr("edges")?.try_iter()?.enumerate() {
        if i % 1024 == 0 {
            checkpoint(py)?;
        }
        let e = e?;
        if groups && e.getattr("soft")?.is_truthy()? {
            continue;
        }
        let fs = e.getattr("faces")?;
        if fs.len()? != 2 {
            continue;
        }
        let a = fs.get_item(0)?;
        let b = fs.get_item(1)?;
        if a.eq(&b)? {
            continue;
        }
        let x = a.getattr("attrs")?;
        let y = b.getattr("attrs")?;
        if groups {
            if !x.is_truthy()? || !y.is_truthy()? {
                continue;
            }
            let g = get(&x, "_s")?;
            if !g.is_truthy()? || !g.eq(get(&y, "_s")?)? {
                continue;
            }
        }
        if !fuse_smooth_sig(&x)?.eq(fuse_smooth_sig(&y)?)? {
            continue;
        }
        let normal = |f: &Obj<'_>| -> PyResult<_> {
            let k = f.getattr("key")?;
            let n = if let Some(n) = cache.get_item(&k)? {
                n
            } else {
                let n = f.call_method0("normal")?;
                cache.set_item(k, &n)?;
                n
            };
            crate::math::xyz(&n)
        };
        let (a, b) = (normal(&a)?, normal(&b)?);
        if a.dot(b).abs() > cos || groups && (a.length() < 1e-9 || b.length() < 1e-9) {
            e.setattr("soft", true)?;
        }
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(mesh,cos_threshold=0.85))]
fn soften_smooth_edges(mesh: &Obj<'_>, cos_threshold: f64) -> PyResult<()> {
    soften(mesh, cos_threshold, false)
}
#[pyfunction]
#[pyo3(signature=(mesh,crease_cos=0.17))]
fn soften_by_smoothing_group(mesh: &Obj<'_>, crease_cos: f64) -> PyResult<()> {
    soften(mesh, crease_cos, true)
}
#[pyfunction]
fn drop_smoothing_groups(mesh: &Obj<'_>) -> PyResult<()> {
    for f in mesh.getattr("faces")?.try_iter()? {
        let a = f?.getattr("attrs")?;
        if a.is_truthy()? {
            a.call_method1("pop", ("_s", mesh.py().None()))?;
        }
    }
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(mesh,max_angle_degrees=0.,principal_planes=false))]
fn simplify_mesh(
    mesh: &Obj<'_>,
    max_angle_degrees: f64,
    principal_planes: bool,
) -> PyResult<usize> {
    if !(0.0..=5.0).contains(&max_angle_degrees) {
        return Err(PyValueError::new_err(
            "Simplification angle must be between 0 and 5 degrees",
        ));
    }
    let py = mesh.py();
    let fs = mesh.getattr("faces")?;
    let count = fs.len()?;
    if count == 0 {
        return Ok(0);
    }
    let raw = PyList::empty(py);
    let mut keep = Vec::new();
    for f in fs.try_iter()? {
        let f = f?;
        if f.getattr("holes")?.is_truthy()? || f.getattr("interior")?.is_truthy()? {
            keep.push(f);
        } else {
            let a = f.getattr("attrs")?;
            raw.append((f.getattr("vertices")?, a.is_truthy()?.then_some(a)))?;
        }
    }
    let dst = new_mesh(py)?;
    let fused = fuse_coplanar_loops(
        &raw,
        max_angle_degrees.to_radians().max(0.00045).cos(),
        principal_planes,
    )?;
    dae_add_fused(&dst, &fused)?;
    for f in keep {
        let n = dst.call_method1("add_face", (f.getattr("vertices")?, f.getattr("holes")?))?;
        assign(&n, &f.getattr("attrs")?)?;
        n.setattr("interior", f.getattr("interior")?)?;
    }
    let api = py.import("core.mesh")?;
    let mut flags = HashMap::new();
    let k = |e: &Obj<'_>| -> PyResult<_> {
        Ok(edge(
            key(crate::math::xyz(&e.getattr("a")?)?),
            key(crate::math::xyz(&e.getattr("b")?)?),
        ))
    };
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        flags.insert(k(&e)?, api.call_method1("edge_flags", (e,))?);
    }
    for e in dst.getattr("edges")?.try_iter()? {
        let e = e?;
        if let Some(f) = flags.get(&k(&e)?) {
            api.call_method1("stamp_edge_flags", (e, f))?;
        }
    }
    loop {
        let mut collapse = None;
        for v in dst.getattr("vertices")?.try_iter()? {
            let v = v?;
            if dst.call_method1("collapsible_vertex", (&v,))?.is_truthy()? {
                collapse = Some(v);
                break;
            }
        }
        if let Some(v) = collapse {
            dst.call_method1("collapse_vertex", (v,))?;
        } else {
            break;
        }
    }
    soften_smooth_edges(&dst, 0.85)?;
    mesh.call_method1("restore_state", (dst.call_method0("capture_state")?,))?;
    Ok(count.saturating_sub(mesh.getattr("faces")?.len()?))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(fuse_coplanar_loops, m)?)?;
    m.add_function(wrap_pyfunction!(fuse_smooth_sig, m)?)?;
    m.add_function(wrap_pyfunction!(soften_by_smoothing_group, m)?)?;
    m.add_function(wrap_pyfunction!(soften_smooth_edges, m)?)?;
    m.add_function(wrap_pyfunction!(drop_smoothing_groups, m)?)?;
    m.add_function(wrap_pyfunction!(simplify_mesh, m)?)
}
