// SPDX-License-Identifier: GPL-3.0-or-later
//! Plan a drawn segment batch against the evolving edge/face lists. The
//! scan supplies local references; history owns the resulting command objects.
use crate::{
    math::xyz,
    store::{
        drawing_face::inside,
        geometry::{plane_axes, Hypot, V3},
    },
};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
type Obj<'py> = Bound<'py, PyAny>;
type Point = [f64; 2];

fn vertices(it: &Obj<'_>) -> PyResult<Vec<DVec3>> {
    it.try_iter()?.map(|p| xyz(&p?)).collect()
}
pub(crate) struct Region {
    origin: DVec3,
    normal: DVec3,
    axes: [DVec3; 2],
    outer: Vec<Point>,
    holes: Vec<(Vec<DVec3>, Vec<Point>)>,
}
impl Region {
    pub(crate) fn new(f: &Obj<'_>) -> PyResult<Self> {
        let outer = vertices(&f.getattr("vertices")?)?;
        let normal = xyz(&f.call_method0("normal")?)?;
        let [x, y, z] = normal.to_array();
        let (u, v) = plane_axes(V3(x, y, z), Hypot::LibStdCxx);
        let mut face = Self {
            origin: outer.first().copied().unwrap_or_default(),
            normal,
            axes: [u, v].map(|V3(x, y, z)| DVec3::new(x, y, z)),
            outer: vec![],
            holes: vec![],
        };
        face.outer = outer.into_iter().map(|p| face.project(p)).collect();
        for hole in f.getattr("holes")?.try_iter()? {
            let ring = vertices(&hole?)?;
            let poly = ring.iter().map(|&p| face.project(p)).collect();
            face.holes.push((ring, poly));
        }
        Ok(face)
    }
    fn project(&self, p: DVec3) -> Point {
        self.axes.map(|u| (p - self.origin).dot(u))
    }
    fn solid(&self, p: DVec3) -> bool {
        let p = self.project(p);
        inside(p, &self.outer, false) && !self.holes.iter().any(|(_, hole)| inside(p, hole, false))
    }
    pub(crate) fn contacts(&self, a: DVec3, b: DVec3) -> bool {
        self.normal.length_squared() > 0.5
            && (a - self.origin).dot(self.normal).abs() < 1e-4
            && (b - self.origin).dot(self.normal).abs() < 1e-4
            && [a, b, (a + b) * 0.5].iter().any(|&p| self.solid(p))
    }
    fn holds(&self, ring: &[Point], cycle: &[DVec3]) -> bool {
        cycle.iter().all(|&p| {
            if (p - self.origin).dot(self.normal).abs() > 1e-3 {
                return false;
            }
            let q = self.project(p);
            inside(q, ring, true) || boundary(q, ring)
        })
    }
    fn touches(&self, cycle: &[DVec3]) -> bool {
        cycle.iter().any(|&p| {
            let q = self.project(p);
            boundary(q, &self.outer)
                || self
                    .outer
                    .iter()
                    .any(|r| (0..2).all(|i| rounded6(q[i]) == rounded6(r[i])))
        })
    }
}
fn rounded6(x: f64) -> f64 {
    let scaled = x * 1e6;
    if scaled.fract().abs() == 0.5 || scaled.abs() >= (1u64 << 52) as f64 {
        format!("{x:.6}").parse::<f64>().unwrap_or(x)
    } else {
        scaled.round_ties_even() / 1e6
    }
}
fn boundary(p: Point, ring: &[Point]) -> bool {
    ring.iter().zip(ring.iter().cycle().skip(1)).any(|(a, b)| {
        let (x, y) = (b[0] - a[0], b[1] - a[1]);
        let (u, v) = (p[0] - a[0], p[1] - a[1]);
        let d = u * x + v * y;
        (x * v - y * u).abs() <= 1e-7 && d >= -1e-7 && d <= x * x + y * y + 1e-7
    })
}

struct Plan<'py> {
    api: Bound<'py, PyModule>,
    edges: Obj<'py>,
    faces: Obj<'py>,
    near: Obj<'py>,
    commands: Bound<'py, PyList>,
    scanned: bool,
    close: bool,
    eye: Obj<'py>,
}
impl<'py> Plan<'py> {
    fn near_edges(&self, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<Obj<'py>> {
        if self.near.is_none() {
            return self.edges.call_method1("pick", ((a, b),));
        }
        self.near.call_method1("edges", (&self.edges, a, b))
    }
    fn near_faces(&self, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<Obj<'py>> {
        if self.near.is_none() {
            return self.faces.call_method1("pick", ((a, b),));
        }
        self.near.call_method1("faces", (&self.faces, a, b))
    }
    fn faces_around(&self, ring: &Obj<'py>) -> PyResult<Obj<'py>> {
        if self.near.is_none() {
            return self.faces.call_method1("pick", (ring,));
        }
        self.near.call_method1("faces_around", (&self.faces, ring))
    }
    fn duplicate(&self, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<bool> {
        Ok(!self
            .api
            .call_method1("find_duplicate_edge", (self.near_edges(a, b)?, a, b))?
            .is_none())
    }
    fn add_simulated(&self, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<()> {
        self.edges
            .call_method1("append", (self.api.call_method1("Edge", (a, b))?,))?;
        Ok(())
    }
    fn delete_face(&self, f: &Obj<'py>) -> PyResult<()> {
        self.commands
            .append(self.api.call_method1("DeleteFaceCommand", (f,))?)
    }
    fn attrs(&self, f: &Obj<'py>) -> PyResult<Obj<'py>> {
        let a = f.getattr("attrs")?;
        let py = a.py();
        if a.is_truthy()? {
            py.get_type::<PyDict>().call1((a,))
        } else {
            Ok(py.None().into_bound(py))
        }
    }
    fn cut(&self, e: &Obj<'py>, p: &Obj<'py>, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<()> {
        let py = e.py();
        let kw = PyDict::new(py);
        kw.set_item("cascade_faces", false)?;
        self.commands.append(
            self.api
                .getattr("DeleteEdgesCommand")?
                .call((vec![e],), Some(&kw))?,
        )?;
        if self.edges.contains(e)? {
            self.edges.call_method1("remove", (e,))?;
        }
        let ea = e.getattr("a")?;
        let eb = e.getattr("b")?;
        let kw = PyDict::new(py);
        let soft = e.getattr_opt("soft")?;
        kw.set_item("soft", soft.filter(|s| s.is_truthy().unwrap_or(false)))?;
        kw.set_item("curve", e.getattr_opt("curve")?)?;
        for (sa, sb) in [(&ea, p), (p, &eb)] {
            if !self.duplicate(sa, sb)? {
                self.commands.append(
                    self.api
                        .getattr("AddEdgeCommand")?
                        .call((sa, sb), Some(&kw))?,
                )?;
                self.add_simulated(sa, sb)?;
            }
        }
        let updates = self.api.call_method1(
            "split_edge_in_faces",
            (self.near_faces(&ea, &eb)?, &ea, &eb, p, (a, b)),
        )?;
        for update in updates.try_iter()? {
            let update = update?;
            let f = update.get_item(0)?;
            let verts = update.get_item(1)?;
            let holes = f.getattr("holes")?;
            let attrs = self.attrs(&f)?;
            let kw = PyDict::new(py);
            kw.set_item("auto", false)?;
            kw.set_item(
                "holes",
                if holes.is_truthy()? {
                    Some(&holes)
                } else {
                    None
                },
            )?;
            kw.set_item("attrs", &attrs)?;
            self.delete_face(&f)?;
            self.commands.append(
                self.api
                    .getattr("AddFaceCommand")?
                    .call((&verts,), Some(&kw))?,
            )?;
            let list = py.get_type::<PyList>();
            let holes = holes
                .try_iter()?
                .map(|h| list.call1((h?,)))
                .collect::<PyResult<Vec<_>>>()?;
            let new = self
                .api
                .call_method1("Face", (list.call1((&verts,))?, holes, &attrs))?;
            let i = self.faces.call_method1("index", (&f,))?;
            self.faces.set_item(i, new)?;
        }
        Ok(())
    }
    fn add_chord(&self, mother: &Obj<'py>, parts: [Obj<'py>; 2]) -> PyResult<()> {
        self.delete_face(mother)?;
        self.faces.call_method1("remove", (mother,))?;
        let py = mother.py();
        let keep = self.attrs(mother)?;
        let kw = PyDict::new(py);
        kw.set_item("auto", self.close)?;
        kw.set_item("attrs", &keep)?;
        for part in parts {
            if !self.api.call_method1("is_planar", (&part,))?.is_truthy()? {
                continue;
            }
            let local = self.faces_around(&part)?;
            if self
                .api
                .call_method1("face_exists", (local, &part))?
                .is_truthy()?
            {
                continue;
            }
            self.commands.append(
                self.api
                    .getattr("AddFaceCommand")?
                    .call((&part,), Some(&kw))?,
            )?;
            let attrs = PyDict::new(py);
            attrs.set_item("attrs", &keep)?;
            self.faces.call_method1(
                "append",
                (self.api.getattr("Face")?.call((part,), Some(&attrs))?,),
            )?;
        }
        Ok(())
    }
    fn cycle_available(
        &self,
        local: Obj<'py>,
        cycle: &Obj<'py>,
        claimed: &Bound<'py, PySet>,
    ) -> PyResult<bool> {
        let points = vertices(cycle)?;
        if points.is_empty() {
            return Ok(false);
        }
        let centre = points.iter().copied().sum::<DVec3>() / points.len() as f64;
        let mut regions = Vec::new();
        for f in local.try_iter()? {
            let f = f?;
            regions.push((
                self.api.call_method1("element_identity", (&f,))?,
                Region::new(&f)?,
            ));
        }
        if let Some((id, _)) = regions
            .iter()
            .find(|(_, r)| r.holes.iter().any(|(_, hole)| r.holds(hole, &points)))
        {
            let mut filled = false;
            for (other, r) in &regions {
                if !id.eq(other)? && r.solid(centre) {
                    filled = true;
                    break;
                }
            }
            if !filled {
                return Ok(false);
            }
        }
        if regions.iter().any(|(_, r)| !r.holes.is_empty()) {
            let shell = Region::new(&self.api.call_method1("Face", (cycle,))?)?;
            for (_, r) in &regions {
                for (hole, _) in &r.holes {
                    if hole
                        .iter()
                        .any(|&p| inside(shell.project(p), &shell.outer, true))
                        && !shell.holds(&shell.outer, hole)
                    {
                        return Ok(false);
                    }
                }
            }
        }
        if let Some((id, r)) = regions
            .iter()
            .find(|(_, r)| r.holds(&r.outer, &points) && r.solid(centre))
        {
            if r.touches(&points) {
                if claimed.contains(id)? {
                    return Ok(false);
                }
                claimed.add(id)?;
            }
        }
        Ok(true)
    }
    fn face_segment(&self, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<()> {
        let split = self
            .api
            .call_method1("find_chord_split", (self.near_faces(a, b)?, a, b))?;
        if !split.is_none() {
            return self.add_chord(
                &split.get_item(0)?,
                [split.get_item(1)?, split.get_item(2)?],
            );
        }
        if !self.close {
            return Ok(());
        }
        let py = a.py();
        let kw = PyDict::new(py);
        let key_owner = if self.near.is_none() { &self.edges } else { &self.near };
        kw.set_item("key", key_owner.getattr("key")?)?;
        if !self.scanned {
            kw.set_item("graph", self.edges.call_method0("graph")?)?;
        }
        let cycles = self
            .api
            .getattr("find_cycles_through")?
            .call((&self.edges, a, b), Some(&kw))?;
        let claimed = PySet::empty(py)?;
        for cycle in cycles.try_iter()? {
            let cycle = cycle?;
            if !self.api.call_method1("is_planar", (&cycle,))?.is_truthy()? {
                continue;
            }
            let local = self.faces_around(&cycle)?;
            if self
                .api
                .call_method1("face_exists", (&local, &cycle))?
                .is_truthy()?
                || !self.cycle_available(local, &cycle, &claimed)?
            {
                continue;
            }
            let oriented = self
                .api
                .call_method1("face_up_or_toward", (&cycle, &self.eye))?;
            self.commands
                .append(self.api.call_method1("AddFaceCommand", (&oriented,))?)?;
            self.faces
                .call_method1("append", (self.api.call_method1("Face", (&oriented,))?,))?;
        }
        Ok(())
    }
}

#[pyfunction]
#[pyo3(signature=(edges,faces,near,segments,scanned,detect,close,eye,soft,tag))]
pub(crate) fn plan_segments<'py>(
    edges: Obj<'py>,
    faces: Obj<'py>,
    near: Obj<'py>,
    segments: &Obj<'py>,
    scanned: bool,
    detect: bool,
    close: bool,
    eye: Obj<'py>,
    soft: Obj<'py>,
    tag: Obj<'py>,
) -> PyResult<Bound<'py, PyList>> {
    let py = segments.py();
    let api = py.import("core.edits")?;
    let plan = Plan {
        api,
        edges,
        faces,
        near,
        commands: PyList::empty(py),
        scanned,
        close,
        eye,
    };
    let kw = PyDict::new(py);
    kw.set_item("soft", soft)?;
    kw.set_item("layer", tag)?;
    for segment in segments.try_iter()? {
        let segment = segment?;
        let (a, b) = (segment.get_item(0)?, segment.get_item(1)?);
        if !xyz(&a)?.is_finite() || !xyz(&b)?.is_finite() {
            continue;
        }
        let split = if scanned {
            plan.edges.call_method1("split_plan", (&a, &b))?
        } else {
            plan.api
                .call_method1("plan_edge_split", (plan.near_edges(&a, &b)?, &a, &b))?
        };
        for (e, p) in split.get_item(1)?.cast::<PyDict>()? {
            plan.cut(&e, &p, &a, &b)?;
        }
        for part in split.get_item(0)?.try_iter()? {
            let part = part?;
            let (sa, sb) = (part.get_item(0)?, part.get_item(1)?);
            let existing = plan.duplicate(&sa, &sb)?;
            plan.commands.append(
                plan.api
                    .getattr("AddEdgeCommand")?
                    .call((&sa, &sb), Some(&kw))?,
            )?;
            if !existing {
                plan.add_simulated(&sa, &sb)?
            }
            if detect {
                plan.face_segment(&sa, &sb)?
            }
        }
    }
    Ok(plan.commands)
}

/// Graph of tool-supplied keys and vertices. The public API permits Python
/// key functions and adjacency adapters. Preserve the original vertex handles.
#[pyfunction]
pub(crate) fn graph_insert(adj: &Bound<'_, PyDict>, key: &Obj<'_>, edge: &Obj<'_>) -> PyResult<()> {
    graph_link(adj, key, edge, None)
}

#[pyfunction]
pub(crate) fn graph_adjacency<'py>(
    edges: &Obj<'py>,
    ka: &Obj<'py>,
    kb: &Obj<'py>,
    key: &Obj<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let adj = PyDict::new(edges.py());
    for edge in edges.try_iter()? {
        graph_link(&adj, key, &edge?, Some((ka, kb)))?;
    }
    Ok(adj)
}

fn graph_link(
    adj: &Bound<'_, PyDict>,
    key: &Obj<'_>,
    edge: &Obj<'_>,
    skip: Option<(&Obj<'_>, &Obj<'_>)>,
) -> PyResult<()> {
    let py = edge.py();
    let a = edge.getattr("a")?;
    let b = edge.getattr("b")?;
    let ka = key.call1((&a,))?;
    let kb = key.call1((&b,))?;
    if let Some((u, v)) = skip {
        if (ka.eq(u)? && kb.eq(v)?) || (ka.eq(v)? && kb.eq(u)?) {
            return Ok(());
        }
    }
    if !ka.eq(&kb)? {
        for (k, v, p) in [(&ka, &kb, &b), (&kb, &ka, &a)] {
            let row = if let Some(row) = adj.get_item(k)? {
                row.cast_into::<PyList>()?
            } else {
                let row = PyList::empty(py);
                adj.set_item(k, &row)?;
                row
            };
            row.append((v, p))?;
        }
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn graph_remove(
    adj: &Bound<'_, PyDict>,
    key: &Obj<'_>,
    edge: &Obj<'_>,
) -> PyResult<bool> {
    let a = edge.getattr("a")?;
    let b = edge.getattr("b")?;
    let ka = key.call1((&a,))?;
    let kb = key.call1((&b,))?;
    if ka.eq(&kb)? {
        return Ok(true);
    }
    for (k, v, p) in [(&ka, &kb, &b), (&kb, &ka, &a)] {
        let Some(row) = adj.get_item(k)? else {
            return Ok(false);
        };
        let mut index = None;
        for (i, item) in row.try_iter()?.enumerate() {
            let item = item?;
            if item.get_item(1)?.is(p) && item.get_item(0)?.eq(v)? {
                index = Some(i);
                break;
            }
        }
        let Some(i) = index else { return Ok(false) };
        row.del_item(i)?;
    }
    Ok(true)
}
#[pyfunction]
#[pyo3(signature=(adj,a,b,ka,kb,max_len=32,skip=None,direct=false))]
pub(crate) fn smallest_cycle<'py>(
    adj: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    ka: &Obj<'py>,
    kb: &Obj<'py>,
    max_len: isize,
    skip: Option<&Obj<'py>>,
    direct: bool,
) -> PyResult<Option<Vec<Obj<'py>>>> {
    if max_len < 3 || !adj.contains(ka)? || !adj.contains(kb)? {
        return Ok(None);
    }
    let parents = PyDict::new(a.py());
    parents.set_item(kb, (a.py().None(), b))?;
    let mut queue = std::collections::VecDeque::from([(kb.clone(), 1isize)]);
    while let Some((u, depth)) = queue.pop_front() {
        if depth >= max_len {
            continue;
        }
        let row = adj.call_method1("get", (&u, ()))?;
        for link in row.try_iter()? {
            let link = link?;
            let v = link.get_item(0)?;
            if (direct && u.eq(kb)? && v.eq(ka)?) || parents.contains(&v)? {
                continue;
            }
            if let Some(skip) = skip {
                if skip.contains(&v)? {
                    continue;
                }
            }
            let pos = link.get_item(1)?;
            parents.set_item(&v, (&u, pos))?;
            if v.eq(ka)? {
                let mut out = Vec::new();
                let mut at = v;
                while !at.is_none() {
                    let prev = parents
                        .get_item(&at)?
                        .expect("each visited vertex has a parent");
                    out.push(prev.get_item(1)?);
                    at = prev.get_item(0)?;
                }
                return Ok((out.len() >= 3).then_some(out));
            }
            queue.push_back((v, depth + 1));
        }
    }
    Ok(None)
}
