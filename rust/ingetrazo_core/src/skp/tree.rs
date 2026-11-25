// SPDX-License-Identifier: GPL-3.0-or-later
//! Build the reachable occurrence tree. Prototype indices are reserved before
//! descent so that cyclic references cannot change indices or recurse forever.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
use std::collections::{HashMap, HashSet};

pub(super) type Obj<'p> = Bound<'p, PyAny>;
pub(super) fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
pub(super) fn attr<'p>(v: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    Ok(v.getattr_opt(k)?.unwrap_or_else(|| none(v.py())))
}
pub(super) fn either<'p>(a: Obj<'p>, b: &Obj<'p>) -> PyResult<Obj<'p>> {
    Ok(if a.is_truthy()? { a } else { b.clone() })
}
pub(super) fn values<'p>(obj: &Obj<'p>, field: &str) -> PyResult<Vec<Obj<'p>>> {
    let v = attr(obj, field)?;
    if v.is_none() {
        Ok(Vec::new())
    } else {
        v.try_iter()?.collect()
    }
}
pub(super) fn flagged(v: &Obj<'_>, flag: &str) -> PyResult<bool> {
    attr(v, flag)?.is_truthy()
}
pub(super) fn image(v: &Obj<'_>) -> PyResult<bool> {
    Ok(flagged(v, "is_image")? || flagged(v, "always_faces_camera")?)
}
fn tuple<'p>(py: Python<'p>, items: &[Obj<'p>]) -> PyResult<Obj<'p>> {
    Ok(PyTuple::new(py, items)?.into_any())
}
fn add_use<'p>(dict: &Bound<'p, PyDict>, key: &Obj<'p>, value: &Obj<'p>) -> PyResult<()> {
    dict.call_method1("setdefault", (key, PyList::empty(dict.py())))?
        .call_method1("append", (value,))?;
    Ok(())
}
/// Count paths in batches at each depth. Repeated placements of a repeated
/// definition should cost one addition per edge, not one walk per occurrence.
fn placement_counts(roots: &[usize], edges: &[Vec<usize>]) -> Vec<usize> {
    let mut remaining = vec![0usize; edges.len()];
    for children in edges {
        for &child in children {
            remaining[child] += 1;
        }
    }
    let mut ready: Vec<_> = (0..edges.len()).filter(|&i| remaining[i] == 0).collect();
    while let Some(i) = ready.pop() {
        for &j in &edges[i] {
            remaining[j] -= 1;
            if remaining[j] == 0 {
                ready.push(j);
            }
        }
    }
    let mut counts = vec![0usize; edges.len()];
    let mut paths = HashMap::<usize, usize>::new();
    for &i in roots {
        *paths.entry(i).or_default() += 1;
    }
    // The ordinary graph is acyclic. For the cyclic remainder preserve the
    // reader's rule: count the back edge, but never descend through it again.
    fn cyclic(
        i: usize,
        edges: &[Vec<usize>],
        counts: &mut [usize],
        path: &mut Vec<usize>,
        depth: usize,
        weight: usize,
    ) {
        if depth > 32 || path.contains(&i) {
            return;
        }
        path.push(i);
        for &j in &edges[i] {
            counts[j] = counts[j].saturating_add(weight);
            cyclic(j, edges, counts, path, depth + 1, weight);
        }
        path.pop();
    }
    for depth in 0..=32 {
        if paths.is_empty() {
            break;
        }
        let mut next = HashMap::<usize, usize>::new();
        for (i, weight) in paths {
            if remaining[i] > 0 {
                cyclic(i, edges, &mut counts, &mut Vec::new(), depth, weight);
                continue;
            }
            for &j in &edges[i] {
                counts[j] = counts[j].saturating_add(weight);
                let scale = next.entry(j).or_default();
                *scale = scale.saturating_add(weight);
            }
        }
        paths = next;
    }
    counts
}

struct Node<'p> {
    def: Obj<'p>,
    image: bool,
    edges: Vec<(usize, Obj<'p>)>,
    tagged: bool,
    faces: usize,
}
struct Plan<'p> {
    nodes: Vec<Node<'p>>,
    roots: Vec<usize>,
    by_id: Bound<'p, PyDict>,
    counts: Vec<usize>,
    protos: HashSet<usize>,
    index: HashMap<usize, usize>,
}
impl<'p> Plan<'p> {
    fn new(model: &Obj<'p>) -> PyResult<Self> {
        let py = model.py();
        let defs = attr(model, "definitions")?;
        let mut objects: Vec<Obj> = if defs.is_truthy()? {
            defs.call_method0("values")?
                .try_iter()?
                .collect::<PyResult<_>>()?
        } else {
            vec![]
        };
        let by_id = PyDict::new(py);
        let mut root = attr(model, "root")?;
        for d in &objects {
            by_id.set_item(attr(d, "id")?, d)?;
            if root.is_none()
                && attr(d, "name")?.extract::<String>().ok().as_deref() == Some("ROOT_MODEL")
            {
                root = d.clone();
            }
        }
        if !root.is_none() && !objects.iter().any(|o| o.is(&root)) {
            objects.push(root.clone());
        }
        let index: HashMap<_, _> = objects
            .iter()
            .enumerate()
            .map(|(i, v)| (v.as_ptr() as usize, i))
            .collect();
        let roots = if root.is_none() {
            (0..objects.len()).collect()
        } else {
            vec![index[&(root.as_ptr() as usize)]]
        };
        let mut nodes = Vec::new();
        for def in objects {
            let mut edges = vec![];
            let mut tagged = false;
            for (i, ins) in values(&def, "instances")?.into_iter().enumerate() {
                if i % 256 == 0 {
                    crate::file_open::skp_checkpoint(py)?;
                }
                let Some(child) = by_id.get_item(attr(&ins, "ref_idx")?)? else {
                    continue;
                };
                tagged |= flagged(&ins, "layer")?;
                edges.push((index[&(child.as_ptr() as usize)], ins));
            }
            let f = attr(&def, "faces")?;
            nodes.push(Node {
                image: image(&def)?,
                edges,
                tagged,
                faces: if f.is_none() { 0 } else { f.len()? },
                def,
            });
        }
        let edges: Vec<Vec<usize>> = nodes
            .iter()
            .map(|n| n.edges.iter().map(|(i, _)| *i).collect())
            .collect();
        let counts = py.detach(|| placement_counts(&roots, &edges));
        Ok(Self {
            counts,
            nodes,
            roots,
            by_id,
            protos: HashSet::new(),
            index,
        })
    }
    fn subtree(
        &self,
        i: usize,
        memo: &mut HashMap<usize, (usize, bool, bool)>,
        path: &mut Vec<usize>,
    ) -> (usize, bool, bool) {
        if path.contains(&i) || path.len() > 32 {
            return (0, false, false);
        }
        if let Some(v) = memo.get(&i) {
            return *v;
        }
        let n = &self.nodes[i];
        let mut row = (n.faces, n.image, n.tagged);
        path.push(i);
        for (k, _) in &n.edges {
            let s = self.subtree(*k, memo, path);
            row.0 = row.0.saturating_add(s.0);
            row.1 |= s.1;
            row.2 |= s.2;
        }
        path.pop();
        memo.insert(i, row);
        row
    }
    fn choose(
        &mut self,
        exact: bool,
        known: bool,
        groups: &Obj<'_>,
        limits: (usize, usize),
    ) -> PyResult<()> {
        if exact {
            self.protos.extend(
                (0..self.nodes.len()).filter(|&i| self.counts[i] > 0 && !self.nodes[i].image),
            );
            return Ok(());
        }
        let mut memo = HashMap::new();
        for i in 0..self.nodes.len() {
            self.subtree(i, &mut memo, &mut vec![]);
        }
        let mut top = vec![0; self.nodes.len()];
        for &r in &self.roots {
            for (k, _) in &self.nodes[r].edges {
                top[*k] += 1;
            }
        }
        for (i, &cnt) in self.counts.iter().enumerate() {
            let (polys, im, tag) = memo[&i];
            if cnt >= 2
                && !im
                && !tag
                && ((polys >= limits.0 && polys.saturating_mul(cnt - 1) >= limits.1)
                    || (known
                        && top[i] == cnt
                        && !groups.contains(attr(&self.nodes[i].def, "id")?)?))
            {
                self.protos.insert(i);
            }
        }
        let mut nested: Vec<_> = (0..top.len())
            .filter(|&i| top[i] > 0 && self.counts[i] == 1 && !self.nodes[i].image)
            .collect();
        self.protos.extend(&nested);
        let mut seen = HashSet::new();
        while let Some(i) = nested.pop() {
            if !seen.insert(i) {
                continue;
            }
            for (k, ins) in &self.nodes[i].edges {
                if self.nodes[*k].image {
                    continue;
                }
                if self.counts[*k] == 1 || ((flagged(ins, "layer")? || memo[k].2) && !memo[k].1) {
                    self.protos.insert(*k);
                }
                nested.push(*k);
            }
        }
        Ok(())
    }
}

struct Builder<'p> {
    py: Python<'p>,
    glue: Obj<'p>,
    plan: Plan<'p>,
    roots: Bound<'p, PyList>,
    proto_ids: Bound<'p, PySet>,
    attrs: Obj<'p>,
    name: Obj<'p>,
    groups: Bound<'p, PyList>,
    protos: Bound<'p, PyList>,
    images: Bound<'p, PyList>,
    singles: HashSet<usize>,
    slots: Bound<'p, PyDict>,
    flat: Obj<'p>,
    consumer: Obj<'p>,
}
impl<'p> Builder<'p> {
    fn mesh(&self, def: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
        let g = PyDict::new(self.py);
        g.set_item("name", either(attr(def, "name")?, &self.name)?)?;
        for k in ["faces", "loose_edges", "curves"] {
            g.set_item(k, PyList::empty(self.py))?;
        }
        g.set_item("soft_edges", self.glue.call_method0("SoftSegments")?)?;
        Ok(g)
    }
    fn collect(
        &self,
        d: &Obj<'p>,
        xf: &Obj<'p>,
        g: &Bound<'p, PyDict>,
        kw: &Bound<'p, PyDict>,
    ) -> PyResult<()> {
        for (key, field) in [
            ("edges_out", "soft_edges"),
            ("loose_out", "loose_edges"),
            ("curves_out", "curves"),
        ] {
            kw.set_item(key, g.as_any().get_item(field)?)?;
        }
        self.glue.getattr("_collect")?.call(
            (
                d,
                xf,
                &self.plan.by_id,
                &self.attrs,
                g.as_any().get_item("faces")?,
                0,
                PySet::empty(self.py)?,
            ),
            Some(kw),
        )?;
        Ok(())
    }
    fn publish_group(&self, g: Bound<'p, PyDict>) -> PyResult<()> {
        if g.as_any().get_item("faces")?.is_truthy()?
            || g.as_any().get_item("loose_edges")?.is_truthy()?
        {
            self.groups.append(g)?;
        }
        Ok(())
    }
    fn flatten(
        &self,
        i: usize,
        xf: &Obj<'p>,
        mat: &Obj<'p>,
        lay: &Obj<'p>,
        id: &Obj<'p>,
        name: Obj<'p>,
        uses: &Bound<'p, PyDict>,
        layers: &Bound<'p, PyList>,
    ) -> PyResult<()> {
        let d = &self.plan.nodes[i].def;
        let g = self.mesh(d)?;
        g.set_item("name", either(name, &g.as_any().get_item("name")?)?)?;
        g.set_item("axes", xf.call_method0("data")?)?;
        g.set_item("identity", id)?;
        if lay.is_truthy()? {
            g.set_item("layer", lay)?;
        }
        let kw = PyDict::new(self.py);
        for (k, v) in [
            ("proto_ids", self.proto_ids.as_any()),
            ("proto_uses", uses.as_any()),
            ("inherited", mat),
            ("image_uses", self.images.as_any()),
            ("layer_uses", layers.as_any()),
            ("group_layer", lay),
            ("identity", id),
            ("release", &self.flat),
        ] {
            kw.set_item(k, v)?;
        }
        self.collect(d, xf, &g, &kw)?;
        self.publish_group(g)
    }
    fn build(&self, first: &Obj<'p>) -> PyResult<usize> {
        enum Step<'p> {
            Build(Obj<'p>),
            Link(Bound<'p, PyDict>, Bound<'p, PyDict>),
        }
        let mut pending = vec![Step::Build(first.clone())];
        while let Some(step) = pending.pop() {
            match step {
                Step::Build(key) => {
                    if self.slots.contains(&key)? {
                        continue;
                    }
                    crate::file_open::skp_checkpoint(self.py)?;
                    let idx = self.protos.len();
                    self.slots.set_item(&key, idx)?;
                    self.protos.append(self.py.None())?;
                    let d = self.plan.by_id.as_any().get_item(key.get_item(0)?)?;
                    let g = self.mesh(&d)?;
                    let images = PyList::empty(self.py);
                    let uses = PyDict::new(self.py);
                    let kw = PyDict::new(self.py);
                    kw.set_item("proto_ids", &self.proto_ids)?;
                    kw.set_item("proto_uses", &uses)?;
                    kw.set_item("inherited", key.get_item(1)?)?;
                    kw.set_item("image_uses", &images)?;
                    self.collect(&d, &self.glue.call_method0("QMatrix4x4")?, &g, &kw)?;
                    if !self.consumer.is_none() {
                        self.consumer.call_method1("release", (&key,))?;
                    }
                    g.set_item("_images", images)?;
                    g.set_item("instances", PyList::empty(self.py))?;
                    g.set_item(
                        "component",
                        !self
                            .singles
                            .contains(&self.plan.index[&(d.as_ptr() as usize)]),
                    )?;
                    self.protos.set_item(idx, &g)?;
                    let keys = uses.keys();
                    pending.push(Step::Link(g, uses));
                    for child in keys.iter().rev() {
                        pending.push(Step::Build(child));
                    }
                }
                Step::Link(g, uses) => {
                    let children = PyList::empty(self.py);
                    for (key, places) in uses {
                        let ci = self.slots.as_any().get_item(&key)?;
                        for place in places.try_iter()? {
                            let p = place?;
                            let kid = PyDict::new(self.py);
                            kid.set_item("proto", &ci)?;
                            kid.set_item("xform", p.get_item(0)?)?;
                            kid.set_item("identity", p.get_item(3)?)?;
                            for (n, k) in [(1, "layer"), (2, "name")] {
                                let v = p.get_item(n)?;
                                if v.is_truthy()? {
                                    kid.set_item(k, v)?;
                                }
                            }
                            children.append(kid)?;
                        }
                    }
                    g.set_item("children", children)?;
                }
            }
        }
        self.slots.as_any().get_item(first)?.extract()
    }
    fn expand(
        &self,
        idx: usize,
        world: &Obj<'p>,
        layer: &Obj<'p>,
        id: &Obj<'p>,
        depth: usize,
    ) -> PyResult<()> {
        let g = self.protos.get_item(idx)?;
        let join = |other: Obj<'p>| -> PyResult<Obj<'p>> {
            if id.is_none() || other.is_none() {
                Ok(none(self.py))
            } else {
                id.add(other)
            }
        };
        for row in g.get_item("_images")?.try_iter()? {
            let r = row?;
            self.images.append((
                r.get_item(0)?,
                world.mul(r.get_item(1)?)?,
                r.get_item(2)?,
                either(r.get_item(3)?, layer)?,
                join(r.get_item(4)?)?,
            ))?;
        }
        if depth < 32 {
            for kid in g.get_item("children")?.try_iter()? {
                let k = kid?;
                self.expand(
                    k.get_item("proto")?.extract()?,
                    &world.mul(k.get_item("xform")?)?,
                    &either(k.call_method1("get", ("layer",))?, layer)?,
                    &join(k.get_item("identity")?)?,
                    depth + 1,
                )?;
            }
        }
        Ok(())
    }
    fn assemble(&mut self, consume: bool) -> PyResult<()> {
        let py = self.py;
        let glue = &self.glue;
        let uses = PyDict::new(py);
        let layers = PyList::empty(py);
        let empty = PyTuple::empty(py).into_any();
        let lifetime = py.import("ingetrazo_core")?;
        let keep = lifetime.call_method1("image_stage_geometry", (&self.plan.by_id,))?;
        if consume
            && !self
                .plan
                .nodes
                .iter()
                .any(|n| flagged(&n.def, "is_image").unwrap_or(false))
        {
            let counts = PyDict::new(py);
            for (i, &n) in self.plan.counts.iter().enumerate() {
                if n > 0 {
                    counts.set_item(attr(&self.plan.nodes[i].def, "id")?, n)?;
                }
            }
            let f = lifetime.call_method1(
                "FlatGeometry",
                (
                    &self.plan.by_id,
                    &self.proto_ids,
                    counts,
                    &self.roots,
                    32,
                    &keep,
                ),
            )?;
            if flagged(&f, "proved")? {
                self.flat = f.getattr("release")?;
            }
        }
        for &r in &self.plan.roots {
            let node = &self.plan.nodes[r];
            let d = &node.def;
            let g = self.mesh(d)?;
            let xf = glue.call_method0("QMatrix4x4")?;
            let geom = glue.call_method1("_defn_geom", (d,))?;
            let pts = glue.call_method1("_world_points", (geom.get_item(0)?, &xf))?;
            for row in geom.get_item(2)?.try_iter()? {
                let f = row?;
                let entry = glue.call_method1(
                    "_face_entry",
                    (
                        f.get_item(0)?,
                        &pts,
                        geom.get_item(1)?,
                        f.get_item(1)?,
                        f.get_item(2)?,
                        f.get_item(3)?,
                        &self.attrs,
                    ),
                )?;
                if !entry.is_none() {
                    g.as_any()
                        .get_item("faces")?
                        .call_method1("append", (entry,))?;
                }
            }
            for (func, key) in [
                ("_curve_segments", "curves"),
                ("_loose_edge_segments", "loose_edges"),
                ("_soft_edge_segments", "soft_edges"),
            ] {
                glue.call_method1(func, (d, &xf, g.as_any().get_item(key)?))?;
            }
            g.set_item("identity", ("root",))?;
            g.set_item("name", &self.name)?;
            self.publish_group(g)?;
            for (j, (k, ins)) in node.edges.iter().enumerate() {
                if j % 256 == 0 {
                    crate::file_open::skp_checkpoint(py)?;
                }
                let c = &self.plan.nodes[*k];
                let xf = glue.call_method1("_matrix", (ins.getattr("matrix")?,))?;
                let id = glue.call_method1("_instance_identity", (&empty, ins))?;
                let mat = attr(ins, "material_id")?;
                let lay = either(attr(ins, "layer")?, &none(py))?;
                let name = either(attr(ins, "name")?, &none(py))?;
                if c.image {
                    self.images.append((&c.def, xf, mat, lay, id))?;
                } else if self.plan.protos.contains(k) {
                    add_use(
                        &uses,
                        &tuple(py, &[attr(&c.def, "id")?, mat])?,
                        &tuple(py, &[xf, lay, name, id])?,
                    )?;
                } else {
                    self.flatten(*k, &xf, &mat, &lay, &id, name, &uses, &layers)?;
                }
            }
        }
        let mut i = 0;
        while i < layers.len() {
            let r = layers.get_item(i)?;
            i += 1;
            let d = r.get_item(0)?;
            self.flatten(
                self.plan.index[&(d.as_ptr() as usize)],
                &r.get_item(1)?,
                &r.get_item(2)?,
                &r.get_item(3)?,
                &r.get_item(4)?,
                none(py),
                &uses,
                &layers,
            )?;
        }
        if !self.flat.is_none() {
            self.consumer = lifetime.call_method1(
                "PrototypeGeometry",
                (
                    &self.plan.by_id,
                    &self.proto_ids,
                    &uses,
                    &self.roots,
                    32,
                    keep,
                ),
            )?;
        }
        for (key, places) in uses {
            crate::file_open::skp_checkpoint(py)?;
            let idx = self.build(&key)?;
            let g = self.protos.get_item(idx)?;
            for (j, key) in [
                "instances",
                "instance_layers",
                "instance_names",
                "instance_identities",
            ]
            .iter()
            .enumerate()
            {
                let col = PyList::empty(py);
                let mut any = false;
                for r in places.try_iter()? {
                    let v = r?.get_item(j)?;
                    any |= v.is_truthy()?;
                    col.append(v)?;
                }
                if any || j == 0 || j == 3 {
                    g.set_item(key, col)?;
                }
            }
        }
        let has_local = self.protos.iter().any(|p| {
            p.get_item("_images")
                .is_ok_and(|i| i.len().unwrap_or(0) > 0)
        });
        for (i, p) in self.protos.iter().enumerate() {
            if has_local {
                let lays = p.call_method1("get", ("instance_layers",))?;
                for (k, xf) in p.get_item("instances")?.try_iter()?.enumerate() {
                    self.expand(
                        i,
                        &xf?,
                        &if lays.is_none() {
                            none(py)
                        } else {
                            lays.get_item(k)?
                        },
                        &p.get_item("instance_identities")?.get_item(k)?,
                        0,
                    )?;
                }
            }
        }
        for p in self.protos.iter() {
            p.del_item("_images")?;
        }
        for row in self.images.iter() {
            let d = row.get_item(0)?;
            let xf = row.get_item(1)?;
            let mat = row.get_item(2)?;
            let g = self.mesh(&d)?;
            let mut billboard = true;
            if flagged(&d, "is_image")? {
                let faces = glue.call_method1("_image_quad_faces", (&d, &xf, &self.attrs, &mat))?;
                billboard = false;
                for f in faces.try_iter()? {
                    let a = f?.get_item(2)?;
                    if !a.is_none() && a.contains("texture")? {
                        let path = a.get_item("texture")?.get_item("path")?;
                        billboard = glue
                            .call_method1("_image_has_cutout", (path,))?
                            .is_truthy()?
                            && !glue
                                .call_method1(
                                    "_rests_on_a_face",
                                    (
                                        &self.roots,
                                        &self.plan.by_id,
                                        faces.get_item(0)?.get_item(0)?,
                                    ),
                                )?
                                .is_truthy()?;
                        break;
                    }
                }
                g.set_item("faces", faces)?;
            } else {
                let kw = PyDict::new(py);
                kw.set_item("inherited", &mat)?;
                self.collect(&d, &xf, &g, &kw)?;
            }
            g.set_item("billboard", billboard)?;
            g.set_item("identity", row.get_item(4)?)?;
            let lay = row.get_item(3)?;
            if lay.is_truthy()? {
                g.set_item("layer", lay)?;
            }
            self.publish_group(g)?;
        }
        Ok(())
    }
}
#[pyfunction(signature=(model,name,skp_path=None,*,consume=false))]
pub fn skp_adapt_inner<'p>(
    model: &Obj<'p>,
    name: Obj<'p>,
    skp_path: Option<&Obj<'p>>,
    consume: bool,
) -> PyResult<Obj<'p>> {
    let py = model.py();
    let glue = py.import("ingetrazo_core.formats.skp_reader")?.into_any();
    let compat = py.import("ingetrazo_core.reader_compat")?;
    let version = attr(model, "version")?.str()?.to_string();
    let major = version
        .trim_start_matches('{')
        .split('.')
        .next()
        .unwrap_or("")
        .parse::<u32>()
        .ok();
    let legacy = major.is_some_and(|n| n < 21);
    let path = skp_path.unwrap_or(&name);
    let paint = glue.call_method1("_material_attrs", (model, path))?;
    let attrs = paint.get_item(0)?;
    let layers = glue.call_method1("file_layer_records", (model,))?;
    let hidden = PyList::empty(py);
    for row in layers.try_iter()? {
        let r = row?;
        if !r.get_item("visible")?.is_truthy()? {
            hidden.append(r.get_item("name")?)?;
        }
    }
    glue.getattr("_geometry_cache")?
        .call_method0("get")?
        .setattr(
            "hidden_tags",
            compat.call_method1("hidden_tags", (hidden,))?,
        )?;
    let mut plan = Plan::new(model)?;
    if !legacy {
        for n in &plan.nodes {
            super::skp_mark_drapes(&n.def, attrs.cast()?)?;
        }
    }
    let groups = compat.call_method0("group_definitions")?;
    let dae = py.import("ingetrazo_core.formats.dae")?;
    plan.choose(
        glue.getattr("EXACT_TREE")?.is_truthy()?,
        compat.call_method1("kinds_known", (legacy,))?.is_truthy()?,
        &groups,
        (
            dae.getattr("_INST_MIN_POLYS")?.extract()?,
            dae.getattr("_INST_MIN_SAVED")?.extract()?,
        ),
    )?;
    let roots = PyList::new(py, plan.roots.iter().map(|&i| &plan.nodes[i].def))?;
    let ids = PySet::empty(py)?;
    let mut singles = HashSet::new();
    for &i in &plan.protos {
        let id = attr(&plan.nodes[i].def, "id")?;
        if plan.counts[i] == 1 && groups.contains(&id)? {
            singles.insert(i);
        }
        ids.add(id)?;
    }
    let mut b = Builder {
        py,
        glue: glue.clone(),
        plan,
        roots,
        proto_ids: ids,
        attrs,
        name,
        groups: PyList::empty(py),
        protos: PyList::empty(py),
        images: PyList::empty(py),
        singles,
        slots: PyDict::new(py),
        flat: none(py),
        consumer: none(py),
    };
    b.assemble(consume)?;
    let protos = glue.call_method1("_merge_equal_protos", (&b.protos,))?;
    let result = PyDict::new(py);
    result.set_item("groups", &b.groups)?;
    result.set_item("protos", &protos)?;
    result.set_item("backend", "openskp")?;
    let ns = if let Some(path) = skp_path {
        py.import("pathlib")?
            .call_method1("Path", (path,))?
            .getattr("stem")?
            .call_method0("casefold")?
    } else {
        let sig = PyList::empty(py);
        for (_, d) in &b.plan.by_id {
            sig.append((
                attr(&d, "name")?.str()?,
                d.getattr_opt("guid")?
                    .map_or(String::new(), |g| g.to_string()),
            ))?;
        }
        sig.call_method0("sort")?;
        py.import("hashlib")?
            .call_method1(
                "sha256",
                (py.import("json")?
                    .call_method1("dumps", (sig,))?
                    .call_method0("encode")?,),
            )?
            .call_method0("hexdigest")?
    };
    result.set_item("identity_namespace", ns)?;
    if b.groups.is_empty() {
        let mut geometry = false;
        for p in protos.try_iter()? {
            let p = p?;
            for k in ["faces", "loose_edges", "children"] {
                geometry |= p.get_item(k)?.is_truthy()?;
            }
        }
        if !geometry {
            if !glue
                .call_method1("_is_empty_model", (model, skp_path, legacy))?
                .is_truthy()?
            {
                return Ok(none(py));
            }
            result.set_item("empty", true)?;
        }
    }
    for (k, v) in [
        ("materials", paint.get_item(1)?),
        ("layers", layers),
        ("layer_folders", compat.call_method0("tag_folder_records")?),
    ] {
        if v.is_truthy()? {
            result.set_item(k, v)?;
        }
    }
    let scenes = PyList::empty(py);
    for (i, page) in values(model, "pages")?.into_iter().enumerate() {
        if i % 256 == 0 {
            crate::file_open::skp_checkpoint(py)?;
        }
        let r = PyDict::new(py);
        let eye = attr(&page, "eye")?;
        let target = attr(&page, "target")?;
        if eye.is_none() || target.is_none() {
            continue;
        }
        for (k, vec) in [("eye", eye), ("target", target)] {
            r.set_item(
                k,
                vec.extract::<Vec<f64>>()?
                    .iter()
                    .map(|x| x * 0.0254)
                    .collect::<Vec<_>>(),
            )?;
        }
        let fallback = format!("Scene {}", scenes.len() + 1)
            .into_pyobject(py)?
            .into_any();
        r.set_item("name", either(attr(&page, "name")?, &fallback)?)?;
        r.set_item(
            "up",
            attr(&page, "up")?
                .extract::<Vec<f64>>()
                .unwrap_or_else(|_| vec![0., 0., 1.]),
        )?;
        r.set_item("fov", attr(&page, "fov")?.extract::<f64>().unwrap_or(35.))?;
        r.set_item("parallel", flagged(&page, "parallel")?)?;
        r.set_item(
            "ortho_height",
            attr(&page, "ortho_height")?.extract::<f64>().unwrap_or(0.) * 0.0254,
        )?;
        r.set_item("hidden_layers", values(&page, "hidden_layers")?)?;
        scenes.append(r)?;
    }
    if !scenes.is_empty() {
        result.set_item("scenes", scenes)?;
    }
    super::skp_model_annotations(model, &result)?;
    for s in values(model, "styles")? {
        let color = attr(&s, "back_color")?;
        if color.is_truthy()? {
            result.set_item(
                "back_color",
                PyTuple::new(py, color.extract::<Vec<f64>>()?.iter().map(|c| c / 255.))?,
            )?;
            break;
        }
    }
    Ok(result.into_any())
}

/// Worker and scripting entry point. Classic files use the installed model
/// provider for the class registry. Modern files only need the native reader.
#[pyfunction(signature = (path, progress=None))]
pub fn skp_parse<'p>(path: &Obj<'p>, progress: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let py = path.py();
    let glue = py.import("ingetrazo_core.formats.skp_reader")?;
    let compat = py.import("ingetrazo_core.reader_compat")?;
    compat.call_method0("reset_group_definitions")?;
    if let Some(callback) = progress {
        callback.call1((0.1, "Reading .skp model…"))?;
    }
    let mut model = glue.call_method1("skp_vff_model", (path, compat.getattr("_parse_tables")?))?;
    if model.is_none() {
        let provider = py.import("openskp")?;
        compat.call_method0("apply")?;
        model = glue.call_method1("_parse_raw", (provider, &compat, path))?;
    }
    if let Some(callback) = progress {
        callback.call1((0.6, "Building geometry…"))?;
    }
    let name = py
        .import("pathlib")?
        .call_method1("Path", (path,))?
        .getattr("stem")?;
    let kw = PyDict::new(py);
    kw.set_item("skp_path", path)?;
    kw.set_item("consume", true)?;
    glue.getattr("_adapt")?.call((model, name), Some(&kw))
}
