// SPDX-License-Identifier: GPL-3.0-or-later
//! COLLADA source arrays, effects, placement graphs and survey-coordinate frames.
use crate::file_model::{assign, dae_add_fused, group, new_mesh, place_mesh, verts};
use glam::{DMat3, DMat4, DVec3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use roxmltree::{Document, Node, NodeId};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
type N<'a> = Node<'a, 'a>;
type Obj<'p> = Bound<'p, PyAny>;
const NS: &str = "http://www.collada.org/2005/11/COLLADASchema";
fn bad(s: impl ToString) -> PyErr {
    PyValueError::new_err(s.to_string())
}
fn tag(n: N<'_>) -> &str {
    n.tag_name().name()
}
fn kids<'a, 'k>(n: N<'a>, k: &'k str) -> impl Iterator<Item = N<'a>> + 'k
where
    'a: 'k,
{
    n.children().filter(move |c| c.has_tag_name((NS, k)))
}
fn child<'a>(n: N<'a>, k: &str) -> Option<N<'a>> {
    kids(n, k).next()
}
fn under<'a>(n: N<'a>, path: &[&str]) -> Option<N<'a>> {
    path.iter().try_fold(n, |el, k| child(el, k))
}
fn val(n: Option<N<'_>>) -> &str {
    n.and_then(|n| n.text()).unwrap_or("").trim()
}
fn label(n: N<'_>) -> &str {
    n.attribute("name")
        .filter(|s| !s.is_empty())
        .or(n.attribute("id"))
        .unwrap_or("node")
}
fn own<'a>(n: N<'a>, key: &str) -> Option<&'a str> {
    kids(n, "extra")
        .flat_map(|e| kids(e, "technique"))
        .filter(|e| e.attribute("profile") == Some("IngeTrazo"))
        .find_map(|t| child(t, key).map(|e| e.text().unwrap_or("")))
}
fn numbers<T: std::str::FromStr>(s: &str) -> PyResult<Vec<T>> {
    s.split_whitespace()
        .map(|v| v.parse().map_err(|_| bad("invalid COLLADA numeric array")))
        .collect()
}
fn matrix(n: N<'_>) -> PyResult<DMat4> {
    let mut m = DMat4::IDENTITY;
    for e in n.children().filter(Node::is_element) {
        let t = tag(e);
        if !matches!(t, "matrix" | "translate" | "rotate" | "scale") {
            continue;
        }
        let v = numbers::<f64>(val(Some(e)))?;
        m *= match (t, v.as_slice()) {
            ("matrix", v) if v.len() == 16 => {
                DMat4::from_cols_array(v.try_into().unwrap()).transpose()
            }
            ("translate", [x, y, z, ..]) => DMat4::from_translation(DVec3::new(*x, *y, *z)),
            ("scale", [x, y, z, ..]) => DMat4::from_scale(DVec3::new(*x, *y, *z)),
            ("rotate", [x, y, z, a, ..]) => {
                let axis = DVec3::new(*x, *y, *z).normalize_or_zero();
                if axis == DVec3::ZERO {
                    DMat4::IDENTITY
                } else {
                    DMat4::from_axis_angle(axis, a.to_radians())
                }
            }
            _ => DMat4::IDENTITY,
        };
    }
    Ok(m)
}
#[derive(Clone, Copy)]
struct Frame {
    world: DMat4,
    local: DMat4,
    shift: DVec3,
}
impl Frame {
    fn root(shift: DVec3) -> Self {
        Self {
            world: DMat4::IDENTITY,
            local: DMat4::IDENTITY,
            shift,
        }
    }
    fn enter(self, mut local: DMat4, far: f64) -> Self {
        let mut shift = self.shift;
        if shift != DVec3::ZERO {
            if local.w_axis.truncate().abs().max_element() > far {
                local.w_axis -= shift.extend(0.);
                shift = DVec3::ZERO
            } else {
                let basis = DMat3::from_mat4(local);
                shift = if basis.determinant() == 0. {
                    DVec3::ZERO
                } else {
                    basis.inverse() * shift
                };
            }
        }
        Self {
            world: self.world * local,
            local,
            shift,
        }
    }
}
#[derive(Clone, Default)]
struct Paint {
    color: Option<Vec<f64>>,
    image: Option<String>,
    name: Option<String>,
}
struct Polygon {
    verts: Vec<usize>,
    uv: Option<Vec<usize>>,
}
struct Primitive {
    symbol: String,
    kind: String,
    loops: Vec<Polygon>,
    uv: Arc<[[f64; 2]]>,
    lines: Vec<(usize, usize)>,
    runs: Vec<usize>,
}
struct Geometry {
    points: Vec<DVec3>,
    prims: Vec<Primitive>,
    soft: Vec<(usize, usize)>,
}
#[derive(Clone)]
struct Face {
    points: Vec<DVec3>,
    uv: Option<Vec<[f64; 2]>>,
    paint: Paint,
}
struct Dae<'a> {
    root: N<'a>,
    ids: HashMap<&'a str, N<'a>>,
    tops: Vec<N<'a>>,
    zup: DMat4,
    far: f64,
    counts: HashMap<(NodeId, usize), usize>,
    uses: HashMap<&'a str, usize>,
    matrices: HashMap<NodeId, DMat4>,
    geometry: HashMap<NodeId, Arc<Geometry>>,
    paint: HashMap<NodeId, Paint>,
    alpha: HashMap<String, bool>,
    base: PathBuf,
}
impl<'a> Dae<'a> {
    fn new(xml: &'a Document<'a>, path: &Path) -> PyResult<Self> {
        let root = xml.root_element();
        let mut ids = HashMap::new();
        let mut uses = HashMap::new();
        for e in root.descendants().filter(Node::is_element) {
            if let Some(i) = e.attribute("id") {
                ids.insert(i, e);
            }
            if e.has_tag_name((NS, "instance_node")) {
                *uses.entry(e.attribute("url").unwrap_or("")).or_default() += 1;
            }
        }
        let asset = child(root, "asset");
        let scale = asset
            .and_then(|e| child(e, "unit"))
            .and_then(|e| e.attribute("meter"))
            .filter(|s| !s.is_empty())
            .unwrap_or("1")
            .parse::<f64>()
            .map_err(bad)?;
        let basis = match val(asset.and_then(|e| child(e, "up_axis"))) {
            "Y_UP" => DMat3::from_cols(DVec3::X, DVec3::Z, DVec3::NEG_Y),
            "X_UP" => DMat3::from_cols(DVec3::Y, DVec3::X, DVec3::Z),
            _ => DMat3::IDENTITY,
        };
        let tops = root
            .descendants()
            .filter(|n| n.has_tag_name((NS, "visual_scene")))
            .flat_map(|n| kids(n, "node"))
            .collect();
        Ok(Self {
            root,
            ids,
            tops,
            zup: DMat4::from_mat3(basis * scale),
            far: 50000. / scale.abs().max(1e-12),
            counts: HashMap::new(),
            uses,
            matrices: HashMap::new(),
            geometry: HashMap::new(),
            paint: HashMap::new(),
            alpha: HashMap::new(),
            base: path.parent().unwrap_or(Path::new("")).to_owned(),
        })
    }
    fn lookup(&self, url: Option<&str>) -> Option<N<'a>> {
        self.ids
            .get(url.unwrap_or("").trim_start_matches('#'))
            .copied()
    }
    fn target(&self, el: N<'a>) -> Option<N<'a>> {
        self.lookup(el.attribute("url"))
    }
    fn frame(&mut self, n: N<'a>, f: Frame) -> PyResult<Frame> {
        if !self.matrices.contains_key(&n.id()) {
            self.matrices.insert(n.id(), matrix(n)?);
        }
        Ok(f.enter(self.matrices[&n.id()], self.far))
    }
    fn placements(&self, limit: usize) -> PyResult<usize> {
        fn count<'a>(
            d: &Dae<'a>,
            n: N<'a>,
            inst: usize,
            nested: usize,
            limit: usize,
            memo: &mut HashMap<(NodeId, usize), usize>,
        ) -> PyResult<usize> {
            if inst > 32 {
                return Ok(0);
            }
            if nested > 512 {
                return Err(bad("COLLADA node hierarchy looks malformed"));
            }
            let key = (n.id(), inst);
            if let Some(v) = memo.get(&key) {
                return Ok(*v);
            }
            let mut sum = 1usize;
            for e in n.children().filter(Node::is_element) {
                let v = match tag(e) {
                    "node" => count(d, e, inst, nested + 1, limit, memo)?,
                    "instance_node" => match d.target(e) {
                        Some(t) => count(d, t, inst + 1, nested + 1, limit, memo)?,
                        None => 0,
                    },
                    _ => 0,
                };
                sum = sum.saturating_add(v);
                if sum > limit {
                    break;
                }
            }
            memo.insert(key, sum);
            Ok(sum)
        }
        let mut memo = HashMap::new();
        let mut total = 0usize;
        for &n in &self.tops {
            total = total.saturating_add(count(self, n, 0, 0, limit, &mut memo)?);
            if total > limit {
                break;
            }
        }
        Ok(total)
    }
    fn direct_count(&self, n: N<'a>) -> usize {
        kids(n, "instance_geometry")
            .filter_map(|e| self.target(e).and_then(|g| child(g, "mesh")))
            .flat_map(|m| m.children())
            .filter(Node::is_element)
            .map(|p| match tag(p) {
                "triangles" | "polylist" => p
                    .attribute("count")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                "polygons" => kids(p, "p").count(),
                _ => 0,
            })
            .fold(0, usize::saturating_add)
    }
    fn count(&mut self, n: N<'a>, depth: usize) -> usize {
        if depth > 32 {
            return 0;
        }
        let key = (n.id(), depth);
        if let Some(v) = self.counts.get(&key) {
            return *v;
        }
        let mut sum = self.direct_count(n);
        for e in n.children().filter(Node::is_element) {
            match tag(e) {
                "node" => sum = sum.saturating_add(self.count(e, depth)),
                "instance_node" => {
                    if let Some(t) = self.target(e) {
                        sum = sum.saturating_add(self.count(t, depth + 1))
                    }
                }
                _ => (),
            }
        }
        self.counts.insert(key, sum);
        sum
    }
    fn source(&self, source: N<'a>) -> PyResult<(Vec<f64>, usize)> {
        let data = numbers(val(child(source, "float_array")))?;
        let stride = under(source, &["technique_common", "accessor"])
            .and_then(|a| a.attribute("stride"))
            .filter(|v| !v.is_empty())
            .unwrap_or("3")
            .parse::<usize>()
            .map_err(bad)?;
        if stride == 0 {
            return Err(bad("invalid zero COLLADA source stride"));
        }
        Ok((data, stride))
    }
    fn geom(&mut self, g: N<'a>) -> PyResult<Arc<Geometry>> {
        if let Some(v) = self.geometry.get(&g.id()) {
            return Ok(v.clone());
        }
        let mut geom = Geometry {
            points: vec![],
            prims: vec![],
            soft: vec![],
        };
        if let Some(mesh) = child(g, "mesh") {
            if let Some(src) = child(mesh, "vertices")
                .and_then(|v| {
                    kids(v, "input").find(|i| i.attribute("semantic") == Some("POSITION"))
                })
                .and_then(|i| self.lookup(i.attribute("source")))
            {
                let (data, stride) = self.source(src)?;
                geom.points.extend(
                    (0..data.len().saturating_sub(2))
                        .step_by(stride)
                        .map(|i| DVec3::from_slice(&data[i..i + 3])),
                );
            }
            let mut runs = HashMap::new();
            for t in kids(mesh, "extra")
                .flat_map(|e| kids(e, "technique"))
                .filter(|t| t.attribute("profile") == Some("IngeTrazo"))
            {
                for e in kids(t, "faces") {
                    runs.insert(
                        e.attribute("material").unwrap_or(""),
                        numbers(val(Some(e)))?,
                    );
                }
            }
            let mut uv_cache = HashMap::new();
            for prim in mesh.children().filter(Node::is_element) {
                let kind = tag(prim);
                if !matches!(
                    kind,
                    "triangles" | "polylist" | "polygons" | "lines" | "linestrips"
                ) {
                    continue;
                }
                let symbol = prim.attribute("material").unwrap_or("");
                let (mut stride, mut v_off, mut uv_off, mut uv_src) = (1usize, 0usize, None, None);
                for i in kids(prim, "input") {
                    let off = i
                        .attribute("offset")
                        .unwrap_or("0")
                        .parse::<usize>()
                        .map_err(bad)?;
                    stride = stride.max(
                        off.checked_add(1)
                            .ok_or_else(|| bad("invalid COLLADA input offset"))?,
                    );
                    match i.attribute("semantic") {
                        Some("VERTEX") => v_off = off,
                        Some("TEXCOORD") if uv_off.is_none() => {
                            uv_off = Some(off);
                            uv_src = self.lookup(i.attribute("source"));
                        }
                        _ => (),
                    }
                }
                let uv = if let Some(src) = uv_src {
                    if !uv_cache.contains_key(&src.id()) {
                        let (data, step) = self.source(src)?;
                        uv_cache.insert(
                            src.id(),
                            data.chunks_exact(step.max(2))
                                .map(|p| [p[0], p[1]])
                                .collect::<Arc<[_]>>(),
                        );
                    }
                    uv_cache[&src.id()].clone()
                } else {
                    Arc::default()
                };
                let mut p = Primitive {
                    symbol: symbol.into(),
                    kind: kind.into(),
                    loops: vec![],
                    lines: vec![],
                    runs: runs.get(symbol).cloned().unwrap_or_default(),
                    uv,
                };
                for pe in kids(prim, "p") {
                    let idx = numbers::<i64>(val(Some(pe)))?;
                    let mut emit = |stream: &[i64]| {
                        let mut verts = Vec::with_capacity(stream.len() / stride);
                        let mut coords = uv_off
                            .filter(|_| !p.uv.is_empty())
                            .map(|_| Vec::with_capacity(verts.capacity()));
                        for row in stream.chunks_exact(stride) {
                            let i = row[v_off] as usize;
                            if i >= geom.points.len() {
                                return;
                            }
                            verts.push(i);
                            if let (Some(v), Some(k)) = (&mut coords, uv_off) {
                                let j = row[k] as usize;
                                if j >= p.uv.len() {
                                    coords = None
                                } else {
                                    v.push(j);
                                }
                            }
                        }
                        if verts.len() >= 3 {
                            p.loops.push(Polygon { verts, uv: coords })
                        }
                    };
                    match kind {
                        "lines" | "linestrips" => {
                            let v: Vec<_> = idx
                                .chunks_exact(stride)
                                .map(|r| r[v_off] as usize)
                                .collect();
                            let step = if kind == "lines" { 2 } else { 1 };
                            for pair in v.windows(2).step_by(step) {
                                if pair[0] != pair[1]
                                    && pair[0] < geom.points.len()
                                    && pair[1] < geom.points.len()
                                {
                                    p.lines.push((pair[0], pair[1]));
                                }
                            }
                        }
                        "triangles" => {
                            let width = stride
                                .checked_mul(3)
                                .ok_or_else(|| bad("invalid COLLADA stride"))?;
                            for chunk in idx.chunks_exact(width) {
                                emit(chunk)
                            }
                        }
                        "polylist" => {
                            let mut rem = idx.as_slice();
                            for n in numbers::<usize>(val(child(prim, "vcount")))? {
                                let size = n
                                    .checked_mul(stride)
                                    .ok_or_else(|| bad("invalid COLLADA polygon length"))?;
                                if size > rem.len() {
                                    break;
                                }
                                emit(&rem[..size]);
                                rem = &rem[size..];
                            }
                        }
                        _ => emit(&idx),
                    }
                    if kind != "polygons" && kind != "linestrips" {
                        break;
                    }
                }
                geom.prims.push(p);
            }
            let soft = numbers::<usize>(own(mesh, "soft").unwrap_or(""))?;
            geom.soft = soft
                .chunks_exact(2)
                .filter(|p| p.iter().all(|&i| i < geom.points.len()))
                .map(|p| (p[0], p[1]))
                .collect();
        }
        let geom = Arc::new(geom);
        self.geometry.insert(g.id(), geom.clone());
        Ok(geom)
    }
    fn material(&mut self, py: Python<'_>, m: N<'a>) -> PyResult<Paint> {
        if let Some(v) = self.paint.get(&m.id()) {
            return Ok(v.clone());
        }
        let mut paint = Paint {
            name: own(m, "name").filter(|s| !s.is_empty()).map(str::to_owned),
            ..Default::default()
        };
        if let Some(effect) = child(m, "instance_effect").and_then(|e| self.target(e)) {
            'search: for shader in ["lambert", "phong", "blinn", "constant"] {
                for d in effect
                    .descendants()
                    .filter(|e| e.has_tag_name((NS, shader)))
                    .filter_map(|e| child(e, "diffuse"))
                {
                    if let Some(c) = child(d, "color") {
                        let v = numbers::<f64>(val(Some(c)))?;
                        if v.len() >= 3 {
                            paint.color = Some(v[..3].to_vec());
                            break 'search;
                        }
                    }
                    let Some(sid) = child(d, "texture").and_then(|t| t.attribute("texture")) else {
                        continue;
                    };
                    let param = |s: &str| {
                        effect.descendants().find(|e| {
                            e.has_tag_name((NS, "newparam")) && e.attribute("sid") == Some(s)
                        })
                    };
                    let surf = val(param(sid).and_then(|e| under(e, &["sampler2D", "source"])));
                    let image =
                        self.ids
                            .get(sid)
                            .filter(|e| tag(**e) == "image")
                            .copied()
                            .or_else(|| {
                                self.ids
                                    .get(val(param(surf)
                                        .and_then(|e| under(e, &["surface", "init_from"]))))
                                    .copied()
                                    .filter(|e| tag(*e) == "image")
                            });
                    if let Some(image) = image {
                        let r = val(child(image, "init_from"));
                        if !r.is_empty() {
                            let path = self.base.join(
                                py.import("urllib.parse")?
                                    .call_method1("unquote", (r,))?
                                    .extract::<String>()?,
                            );
                            if path.is_file() {
                                paint.image = Some(path.to_string_lossy().into_owned());
                                let avg = py
                                    .import("core.vista_caras")?
                                    .call_method1("texture_average", (path,))?;
                                paint.color = avg.extract()?;
                            }
                            if paint.color.is_none() {
                                let h = r.into_pyobject(py)?.hash()?;
                                paint.color = Some(
                                    [0, 4, 8]
                                        .map(|s| 0.78 + ((h >> s) & 15) as f64 / 100.)
                                        .to_vec(),
                                );
                            }
                        }
                        break 'search;
                    }
                }
            }
        }
        self.paint.insert(m.id(), paint.clone());
        Ok(paint)
    }
    fn bindings(&mut self, py: Python<'_>, inst: N<'a>) -> PyResult<HashMap<String, Paint>> {
        let mut mats = HashMap::new();
        for im in inst
            .descendants()
            .filter(|e| e.has_tag_name((NS, "instance_material")))
        {
            if let Some(m) = self.lookup(im.attribute("target")) {
                mats.insert(
                    im.attribute("symbol").unwrap_or("").to_string(),
                    self.material(py, m)?,
                );
            }
        }
        Ok(mats)
    }
}

impl<'a> Dae<'a> {
    fn world_bounds(&mut self) -> PyResult<(DVec3, DVec3)> {
        let mut boxes = HashMap::new();
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        let mut nodes: Vec<_> = self.tops.iter().map(|&n| (n, DMat4::IDENTITY, 0)).collect();
        while let Some((n, parent, depth)) = nodes.pop() {
            if depth > 32 {
                continue;
            }
            let world = self
                .frame(
                    n,
                    Frame {
                        world: parent,
                        ..Frame::root(DVec3::ZERO)
                    },
                )?
                .world;
            for e in n.children().filter(Node::is_element) {
                match tag(e) {
                    "node" => nodes.push((e, world, depth)),
                    "instance_node" => {
                        if let Some(t) = self.target(e) {
                            nodes.push((t, world, depth + 1))
                        }
                    }
                    "instance_geometry" => {
                        if let Some(g) = self.target(e) {
                            if !boxes.contains_key(&g.id()) {
                                let geom = self.geom(g)?;
                                let a = geom.points.iter().copied().reduce(DVec3::min);
                                let b = geom.points.iter().copied().reduce(DVec3::max);
                                boxes.insert(g.id(), a.zip(b));
                            }
                            if let Some((a, b)) = boxes[&g.id()] {
                                for x in [a.x, b.x] {
                                    for y in [a.y, b.y] {
                                        for z in [a.z, b.z] {
                                            let p = self.zup.transform_point3(
                                                world.transform_point3(DVec3::new(x, y, z)),
                                            );
                                            lo = lo.min(p);
                                            hi = hi.max(p);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => (),
                }
            }
        }
        Ok((lo, hi))
    }
    fn survey<'p>(&mut self, py: Python<'p>) -> PyResult<(Obj<'p>, DVec3)> {
        let (lo, hi) = py.detach(|| self.world_bounds())?;
        let offset = crate::file_bounds::survey_offset(py, lo.to_array(), hi.to_array())?;
        let shift = if offset.is_none() {
            DVec3::ZERO
        } else {
            self.zup
                .inverse()
                .transform_vector3(DVec3::from_array(offset.extract()?))
        };
        Ok((offset, shift))
    }
    fn points(&self, geom: &Geometry, f: Frame) -> Vec<DVec3> {
        geom.points
            .iter()
            .map(|&p| {
                self.zup
                    .transform_point3(f.world.transform_point3(p - f.shift))
            })
            .collect()
    }
    fn direct(
        &mut self,
        py: Python<'_>,
        n: N<'a>,
        f: Frame,
        faces: &mut Vec<Face>,
    ) -> PyResult<()> {
        for inst in kids(n, "instance_geometry") {
            let Some(g) = self.target(inst) else { continue };
            let mats = self.bindings(py, inst)?;
            let geom = self.geom(g)?;
            py.detach(|| {
                let points = self.points(&geom, f);
                for prim in &geom.prims {
                    let paint = mats.get(&prim.symbol).cloned().unwrap_or_default();
                    for poly in &prim.loops {
                        faces.push(face(poly, &points, prim, &paint));
                    }
                }
            });
        }
        Ok(())
    }
    fn sprite(&mut self, py: Python<'_>, faces: &[Face]) -> PyResult<bool> {
        if faces.is_empty() || faces.len() > 400 {
            return Ok(false);
        }
        let Some(path) = faces[0].paint.image.as_deref() else {
            return Ok(false);
        };
        let mut corners = HashSet::new();
        let mut first = None;
        for f in faces {
            if f.uv.is_none() || f.paint.image.as_deref() != Some(path) {
                return Ok(false);
            }
            let n = normal(&f.points);
            if n.length() < 1e-12 {
                continue;
            }
            let n = n.normalize();
            if n.z.abs() > 0.08 || first.is_some_and(|v| n.dot(v).abs() < 0.99) {
                return Ok(false);
            }
            first.get_or_insert(n);
            corners.extend(f.points.iter().map(|&p| key(p, 4)));
        }
        if first.is_none() {
            return Ok(false);
        }
        if corners.len() > 4 {
            return Ok(true);
        }
        if !self.alpha.contains_key(path) {
            let a = py
                .import("ingetrazo_render")?
                .call_method1("texture_cutout", (path,))?
                .is_truthy()?;
            self.alpha.insert(path.to_owned(), a);
        }
        Ok(self.alpha[path])
    }
    fn collect(
        &mut self,
        py: Python<'_>,
        n: N<'a>,
        pre: Frame,
        depth: usize,
        children: bool,
        out: &mut Vec<Face>,
        pieces: &mut Pieces<'a>,
        opts: Collect,
    ) -> PyResult<()> {
        if depth > 32 {
            return Ok(());
        }
        let f = self.frame(n, pre)?;
        self.direct(py, n, f, out)?;
        for e in n.children().filter(Node::is_element) {
            let (t, d) = match tag(e) {
                "node" if children => (e, depth),
                "instance_node" => {
                    let Some(t) = self.target(e) else { continue };
                    if opts.instances {
                        let copies = *self
                            .uses
                            .get(e.attribute("url").unwrap_or(""))
                            .unwrap_or(&1);
                        let polys = self.count(t, depth + 1);
                        if copies >= 2
                            && polys >= opts.min_polys
                            && polys.saturating_mul(copies - 1) >= opts.min_saved
                        {
                            pieces.inst.push((
                                e.attribute("name").unwrap_or(label(t)).into(),
                                t,
                                f,
                                depth + 1,
                            ));
                            continue;
                        }
                    }
                    (t, depth + 1)
                }
                _ => continue,
            };
            if opts.sprites {
                let mut sub = vec![];
                self.collect(py, t, f, d, true, &mut sub, pieces, opts)?;
                if self.sprite(py, &sub)? {
                    pieces
                        .sprites
                        .push((e.attribute("name").unwrap_or(label(t)).into(), sub))
                } else {
                    out.append(&mut sub)
                }
            } else {
                self.collect(py, t, f, d, true, out, pieces, opts)?
            }
        }
        Ok(())
    }
    fn lines(
        &mut self,
        n: N<'a>,
        pre: Frame,
        depth: usize,
        children: bool,
        out: &mut Vec<(DVec3, DVec3)>,
    ) -> PyResult<()> {
        if depth > 32 {
            return Ok(());
        }
        let f = self.frame(n, pre)?;
        for e in n.children().filter(Node::is_element) {
            match tag(e) {
                "instance_geometry" => {
                    if let Some(g) = self.target(e) {
                        let geom = self.geom(g)?;
                        if geom.prims.iter().any(|p| !p.lines.is_empty()) {
                            let pts = self.points(&geom, f);
                            for p in &geom.prims {
                                out.extend(p.lines.iter().map(|&(a, b)| (pts[a], pts[b])));
                            }
                        }
                    }
                }
                "node" if children => self.lines(e, f, depth, true, out)?,
                "instance_node" => {
                    if let Some(t) = self.target(e) {
                        self.lines(t, f, depth + 1, true, out)?
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }
}
fn face(poly: &Polygon, points: &[DVec3], prim: &Primitive, paint: &Paint) -> Face {
    Face {
        points: poly.verts.iter().map(|&i| points[i]).collect(),
        uv: poly
            .uv
            .as_ref()
            .filter(|_| paint.image.is_some())
            .map(|u| u.iter().map(|&i| prim.uv[i]).collect()),
        paint: paint.clone(),
    }
}
fn normal(pts: &[DVec3]) -> DVec3 {
    pts.iter()
        .zip(pts.iter().cycle().skip(1))
        .map(|(&a, &b)| a.cross(b))
        .sum()
}
fn key(p: DVec3, d: usize) -> [u64; 3] {
    p.to_array().map(|v| {
        let n = if v.fract() == 0. {
            v
        } else {
            format!("{v:.d$}").parse().unwrap_or(v)
        };
        if n == 0. {
            0
        } else {
            n.to_bits()
        }
    })
}
fn attr<'p>(py: Python<'p>, f: &Face, points: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let attrs = PyDict::new(py);
    if let (Some(path), Some(uv)) = (&f.paint.image, &f.uv) {
        let map = py
            .import("core.texture")?
            .call_method1("fit_uv_affine", (points, uv))?;
        if !map.is_none() {
            let a: [f64; 8] = map.extract()?;
            let t = PyDict::new(py);
            t.set_item("path", path)?;
            t.set_item("uvw", map)?;
            for (k, start) in [("sw", 0), ("sh", 4)] {
                let len = DVec3::from_slice(&a[start..]).length();
                t.set_item(k, if len > 1e-9 { 1. / len } else { 1. })?
            }
            attrs.set_item("texture", t)?;
            attrs.set_item("back", true)?;
            return Ok(attrs);
        }
    }
    if let Some(color) = &f.paint.color {
        attrs.set_item("color", color)?;
        attrs.set_item("back", true)?
    }
    Ok(attrs)
}
fn mapped<'p>(py: Python<'p>, m: DMat4) -> PyResult<Obj<'p>> {
    Ok(Py::new(py, crate::math::Matrix4::from(m))?
        .into_bound(py)
        .into_any())
}
fn fused<'p>(py: Python<'p>, faces: &[Face]) -> PyResult<Obj<'p>> {
    let mesh = new_mesh(py)?;
    let raw = PyList::empty(py);
    for f in faces {
        let p = verts(py, &f.points)?;
        let attrs = attr(py, f, &p)?;
        raw.append((p, (!attrs.is_empty()).then_some(attrs)))?
    }
    let api = py.import("formats.fuse")?;
    dae_add_fused(&mesh, &api.call_method1("fuse_coplanar_loops", (raw,))?)?;
    api.call_method1("soften_smooth_edges", (&mesh,))?;
    Ok(mesh)
}
fn editable(mesh: &Obj<'_>, faces: &[Face], lines: &[(DVec3, DVec3)]) -> PyResult<()> {
    use pyo3::types::PySet;
    let py = mesh.py();
    let seed = PySet::empty(py)?;
    let added = PySet::empty(py)?;
    let key = py.import("core.topology")?.getattr("_key")?;
    for face in faces {
        let pts = verts(py, &face.points)?;
        let Ok(f) = mesh.call_method1("add_face", (&pts,)) else {
            continue;
        };
        added.add(&f)?;
        assign(&f, attr(py, face, &pts)?.as_any())?;
        for p in pts.iter() {
            seed.add(key.call1((p,))?)?
        }
    }
    if !faces.is_empty() {
        let kw = PyDict::new(py);
        kw.set_item("coplanar_merge", true)?;
        py.import("core.history")?
            .call_method("run_stitch", (mesh, seed, added), Some(&kw))?;
        py.import("core.orient")?
            .call_method1("orient_outward", (mesh,))?;
    }
    for &(a, b) in lines {
        if a.distance(b) >= 1e-6 {
            let points = verts(py, &[a, b])?;
            let _ = mesh.call_method1("add_edge", (points.get_item(0)?, points.get_item(1)?));
        }
    }
    Ok(())
}
#[derive(Clone, Copy)]
struct Collect {
    sprites: bool,
    instances: bool,
    min_polys: usize,
    min_saved: usize,
}
#[derive(Default)]
struct Pieces<'a> {
    sprites: Vec<(String, Vec<Face>)>,
    inst: Vec<(String, N<'a>, Frame, usize)>,
}
#[derive(Clone, Copy)]
struct Limits {
    fuse: usize,
    split: usize,
    max_groups: usize,
    placements: usize,
    collect: Collect,
}
impl Limits {
    fn get(py: Python<'_>) -> PyResult<Self> {
        let m = py.import("formats.dae")?;
        let k = |s| m.getattr(s)?.extract::<usize>();
        Ok(Self {
            fuse: k("_MAX_FUSE_LOOPS")?,
            split: k("_SPLIT_MIN")?,
            max_groups: k("_MAX_GROUPS")?,
            placements: k("_MAX_PLACEMENTS")?,
            collect: Collect {
                sprites: true,
                instances: true,
                min_polys: k("_INST_MIN_POLYS")?,
                min_saved: k("_INST_MIN_SAVED")?,
            },
        })
    }
}
impl<'a> Dae<'a> {
    fn small_group<'p>(
        &mut self,
        py: Python<'p>,
        n: N<'a>,
        pre: Frame,
        depth: usize,
        pieces: &mut Pieces<'a>,
        opts: Collect,
    ) -> PyResult<Option<Obj<'p>>> {
        if depth > 32 {
            return Ok(None);
        }
        let f = self.frame(n, pre)?;
        let mut faces = vec![];
        let mut lines = vec![];
        self.collect(py, n, pre, 0, false, &mut faces, pieces, opts)?;
        self.lines(n, pre, 0, false, &mut lines)?;
        if self.sprite(py, &faces)? {
            pieces
                .sprites
                .push((label(n).into(), std::mem::take(&mut faces)))
        }
        let children = PyList::empty(py);
        for c in kids(n, "node") {
            if let Some(k) = self.small_group(py, c, f, depth + 1, pieces, opts)? {
                k.setattr("xform", mapped(py, DMat4::IDENTITY)?)?;
                k.setattr("component", false)?;
                children.append(k)?
            }
        }
        let mesh = new_mesh(py)?;
        editable(&mesh, &faces, &lines)?;
        if children.is_empty()
            && !mesh.getattr("faces")?.is_truthy()?
            && !mesh.getattr("edges")?.is_truthy()?
        {
            return Ok(None);
        }
        let g = group(py, &mesh, label(n))?;
        let axes = self.zup * f.world * self.zup.inverse();
        if axes != DMat4::IDENTITY {
            g.setattr("axes", mapped(py, axes)?)?
        }
        g.call_method1("adopt", (children,))?;
        g.setattr("component", false)?;
        Ok(Some(g))
    }
}
#[derive(Clone)]
struct Bucket<'a> {
    n: N<'a>,
    size: usize,
    order: usize,
    frame: Frame,
    depth: usize,
    name: String,
    direct: bool,
    instance: bool,
}
impl<'a> Dae<'a> {
    fn reference<'p>(
        &mut self,
        py: Python<'p>,
        root: Frame,
        limits: Limits,
        groups: &Bound<'p, PyList>,
        progress: Option<&Obj<'p>>,
    ) -> PyResult<()> {
        let mut heap = vec![];
        let mut seq = 0;
        for n in self.tops.clone() {
            let size = self.count(n, 0);
            if size != 0 {
                heap.push(Bucket {
                    n,
                    size,
                    order: seq,
                    frame: root,
                    depth: 0,
                    name: label(n).into(),
                    direct: false,
                    instance: false,
                });
                seq += 1
            }
        }
        let mut done = vec![];
        while !heap.is_empty() && heap.len() + done.len() < limits.max_groups {
            let i = (0..heap.len())
                .max_by_key(|&i| (heap[i].size, std::cmp::Reverse(heap[i].order)))
                .unwrap();
            if heap[i].size <= limits.split {
                break;
            }
            let entry = heap.swap_remove(i);
            let n = entry.n;
            if entry.direct
                || entry.depth > 32
                || !n
                    .children()
                    .any(|c| c.is_element() && matches!(tag(c), "node" | "instance_node"))
            {
                done.push(entry);
                continue;
            }
            let f = self.frame(n, entry.frame)?;
            let direct = self.direct_count(n);
            if direct > 0 {
                heap.push(Bucket {
                    direct: true,
                    instance: false,
                    frame: f,
                    size: direct,
                    order: seq,
                    ..entry.clone()
                });
                seq += 1
            }
            for (k, via) in [("node", false), ("instance_node", true)] {
                for el in kids(n, k) {
                    let t = if via {
                        let Some(t) = self.target(el) else { continue };
                        t
                    } else {
                        el
                    };
                    let depth = entry.depth + usize::from(via);
                    let size = self.count(t, depth);
                    if size != 0 {
                        heap.push(Bucket {
                            n: t,
                            size,
                            order: seq,
                            frame: f,
                            depth,
                            name: el
                                .attribute("name")
                                .filter(|s| !s.is_empty())
                                .unwrap_or(label(t))
                                .into(),
                            direct: false,
                            instance: via,
                        });
                        seq += 1
                    }
                }
            }
        }
        done.extend(heap);
        done.sort_by_key(|b| (std::cmp::Reverse(b.size), b.order));
        let mut seen = HashMap::new();
        for b in &done {
            if b.instance && !b.direct {
                *seen.entry(b.n.id()).or_insert(0usize) += 1
            }
        }
        let mut pieces = Pieces::default();
        let mut buckets = vec![];
        for b in done {
            if b.instance && !b.direct && seen.get(&b.n.id()).copied().unwrap_or(0) >= 2 {
                pieces.inst.push((b.name, b.n, b.frame, b.depth));
                continue;
            }
            let mut loops = vec![];
            if b.direct {
                self.direct(py, b.n, b.frame, &mut loops)?
            } else {
                self.collect(
                    py,
                    b.n,
                    b.frame,
                    b.depth,
                    true,
                    &mut loops,
                    &mut pieces,
                    limits.collect,
                )?;
                if self.sprite(py, &loops)? {
                    pieces.sprites.push((b.name, loops));
                    continue;
                }
            }
            if !loops.is_empty() {
                buckets.push((b.name, loops))
            }
        }
        if buckets.is_empty() && pieces.sprites.is_empty() && pieces.inst.is_empty() {
            return Err(bad("No geometry found in the COLLADA file"));
        }
        let total = buckets.len() + pieces.inst.len() + pieces.sprites.len();
        let mut step = 0;
        let mut tick = |name: &str| -> PyResult<()> {
            if let Some(cb) = progress {
                cb.call1((
                    0.15 + 0.8 * step as f64 / total.max(1) as f64,
                    format!("Building {name}…"),
                ))?;
            }
            step += 1;
            Ok(())
        };
        for (name, faces) in buckets {
            tick(&name)?;
            let m = fused(py, &faces)?;
            if m.getattr("faces")?.is_truthy()? {
                groups.append(group(py, &m, &name)?)?
            }
        }
        let mut prototypes = HashMap::new();
        let opts = Collect {
            instances: false,
            ..limits.collect
        };
        for (name, n, f, depth) in std::mem::take(&mut pieces.inst) {
            tick(&name)?;
            let key = (n.id(), f.shift.to_array().map(f64::to_bits));
            if !prototypes.contains_key(&key) {
                let mut probe = Pieces::default();
                let mut raw = vec![];
                self.collect(
                    py,
                    n,
                    Frame::root(f.shift),
                    depth,
                    true,
                    &mut raw,
                    &mut probe,
                    opts,
                )?;
                let m = if probe.sprites.is_empty() {
                    let m = fused(py, &raw)?;
                    m.getattr("faces")?.is_truthy()?.then_some(m)
                } else {
                    None
                };
                prototypes.insert(key, m);
            }
            if let Some(m) = &prototypes[&key] {
                let g = group(py, m, &name)?;
                g.setattr(
                    "xform",
                    mapped(py, self.zup * f.world * self.zup.inverse())?,
                )?;
                groups.append(g)?
            } else {
                let mut raw = vec![];
                self.collect(py, n, f, depth, true, &mut raw, &mut pieces, opts)?;
                if self.sprite(py, &raw)? {
                    pieces.sprites.push((name, raw))
                } else {
                    let m = fused(py, &raw)?;
                    if m.getattr("faces")?.is_truthy()? {
                        groups.append(group(py, &m, &name)?)?
                    }
                }
            }
        }
        for (name, raw) in pieces.sprites {
            tick(&name)?;
            let m = fused(py, &raw)?;
            if m.getattr("faces")?.is_truthy()? {
                let g = group(py, &m, &name)?;
                g.setattr("billboard", "mesh")?;
                groups.append(g)?
            }
        }
        Ok(())
    }
}

fn boundary(points: &[Vec<DVec3>]) -> Option<Vec<Vec<DVec3>>> {
    type Key = [u64; 3];
    let mut pos = HashMap::<Key, DVec3>::new();
    let mut edges = HashSet::new();
    let mut order = vec![];
    for tri in points {
        if tri.len() != 3 {
            return None;
        }
        for (&a, &b) in tri.iter().zip(tri.iter().cycle().skip(1)) {
            let ka = key(a, 5);
            let kb = key(b, 5);
            pos.entry(ka).or_insert(a);
            if ka != kb && edges.insert((ka, kb)) {
                order.push((ka, kb))
            }
        }
    }
    let mut next = HashMap::new();
    for (a, b) in order.iter().copied() {
        if !edges.contains(&(b, a)) && next.insert(a, b).is_some() {
            return None;
        }
    }
    let mut rings = vec![];
    for (start, _) in order {
        if !next.contains_key(&start) {
            continue;
        }
        let mut r = vec![];
        let mut k = start;
        loop {
            r.push(pos[&k]);
            k = next.remove(&k)?;
            if k == start {
                break;
            }
            if r.len() > 100000 {
                return None;
            }
        }
        rings.push(r)
    }
    rings.sort_by(|a, b| {
        normal(b)
            .length_squared()
            .total_cmp(&normal(a).length_squared())
    });
    (!rings.is_empty()).then_some(rings)
}
struct Structured<'p> {
    registry: Bound<'p, PyDict>,
    mat_names: HashMap<(String, String), String>,
    meshes: HashMap<(NodeId, [u64; 3]), Obj<'p>>,
    names: Vec<String>,
    layers: Vec<String>,
}
impl<'p> Structured<'p> {
    fn material(
        &mut self,
        py: Python<'p>,
        paint: &Paint,
        material_id: &str,
    ) -> PyResult<Option<String>> {
        let Some(n) = &paint.name else {
            return Ok(None);
        };
        let k = (material_id.to_owned(), n.clone());
        if let Some(name) = self.mat_names.get(&k) {
            return Ok(Some(name.clone()));
        }
        let color = if paint.image.is_none() {
            paint
                .color
                .as_ref()
                .map(|c| pyo3::types::PyTuple::new(py, c))
                .transpose()?
        } else {
            None
        };
        let texture = if let Some(path) = &paint.image {
            let d = PyDict::new(py);
            d.set_item("path", path)?;
            Some(d)
        } else {
            None
        };
        let module = py.import("core.materials")?;
        let material = module.getattr("Material")?.call1((n, color, texture))?;
        let name: String = module
            .call_method1("register", (&self.registry, material))?
            .extract()?;
        self.mat_names.insert(k, name.clone());
        Ok(Some(name))
    }
    fn mesh<'a>(
        &mut self,
        py: Python<'p>,
        dae: &mut Dae<'a>,
        n: N<'a>,
        f: Frame,
    ) -> PyResult<Obj<'p>> {
        let Some((inst, g)) =
            kids(n, "instance_geometry").find_map(|e| dae.target(e).map(|g| (e, g)))
        else {
            return new_mesh(py);
        };
        let k = (g.id(), f.shift.to_array().map(f64::to_bits));
        if let Some(m) = self.meshes.get(&k) {
            return Ok(m.clone());
        }
        let geom = py.detach(|| dae.geom(g))?;
        let mesh = new_mesh(py)?;
        let mats = dae.bindings(py, inst)?;
        let targets: HashMap<_, _> = inst
            .descendants()
            .filter(|e| e.has_tag_name((NS, "instance_material")))
            .map(|m| {
                (
                    m.attribute("symbol").unwrap_or(""),
                    m.attribute("target").unwrap_or(""),
                )
            })
            .collect();
        let points = dae.points(&geom, Frame::root(f.shift));
        let tris = PyList::empty(py);
        for prim in &geom.prims {
            crate::file_open::checkpoint(py)?;
            if prim.kind == "lines" {
                for &(a, b) in &prim.lines {
                    let ab = verts(py, &[points[a], points[b]])?;
                    let _ = mesh.call_method1("add_edge", (ab.get_item(0)?, ab.get_item(1)?));
                }
                continue;
            }
            let paint = mats.get(&prim.symbol).cloned().unwrap_or_default();
            let mname = self.material(
                py,
                &paint,
                targets.get(prim.symbol.as_str()).copied().unwrap_or(""),
            )?;
            let attrs = |face: &Face, pts: &Obj<'p>| -> PyResult<_> {
                let d = attr(py, face, pts)?;
                if d.contains("back")? {
                    d.del_item("back")?
                }
                if let Some(name) = &mname {
                    d.set_item("mat", name)?
                }
                Ok(d)
            };
            if prim.kind == "triangles"
                && !prim.runs.is_empty()
                && prim.runs.iter().try_fold(0usize, |v, &x| v.checked_add(x))
                    == Some(prim.loops.len())
            {
                let mut idx = 0;
                for &count in &prim.runs {
                    let part = &prim.loops[idx..idx + count];
                    idx += count;
                    let fs = part
                        .iter()
                        .map(|p| face(p, &points, prim, &paint))
                        .collect::<Vec<_>>();
                    let loops: Vec<_> = fs.iter().map(|f| f.points.clone()).collect();
                    let all = Face {
                        points: loops.iter().flatten().copied().collect(),
                        uv: fs
                            .iter()
                            .map(|f| f.uv.as_ref())
                            .collect::<Option<Vec<_>>>()
                            .map(|v| v.into_iter().flatten().copied().collect()),
                        paint: paint.clone(),
                    };
                    let style = attrs(&all, verts(py, &all.points)?.as_any())?;
                    let mut joined = false;
                    if let Some(r) = boundary(&loops) {
                        let holes = PyList::empty(py);
                        for h in &r[1..] {
                            holes.append(verts(py, h)?)?
                        }
                        if let Ok(f) = mesh.call_method1(
                            "add_face",
                            (verts(py, &r[0])?, (!holes.is_empty()).then_some(holes)),
                        ) {
                            assign(&f, &style)?;
                            joined = true;
                        }
                    }
                    if !joined {
                        for lp in &loops {
                            tris.append((verts(py, lp)?, style.copy()?))?
                        }
                    }
                }
                continue;
            }
            for (i, poly) in prim.loops.iter().enumerate() {
                if i % 512 == 0 {
                    crate::file_open::checkpoint(py)?;
                }
                let f = face(poly, &points, prim, &paint);
                let pts = verts(py, &f.points)?;
                let style = attrs(&f, &pts)?;
                if prim.kind == "triangles" {
                    tris.append((pts, style))?
                } else if let Ok(f) = mesh.call_method1("add_face", (pts,)) {
                    assign(&f, &style)?
                }
            }
        }
        if !tris.is_empty() {
            let fused = py
                .import("formats.fuse")?
                .call_method1("fuse_coplanar_loops", (tris,))?;
            dae_add_fused(&mesh, &fused)?
        }
        if !geom.soft.is_empty() {
            let pair = |x: DVec3, y: DVec3| {
                let mut k = [key(x, 5), key(y, 5)];
                k.sort_unstable();
                k
            };
            let wanted: HashSet<_> = geom
                .soft
                .iter()
                .map(|&(a, b)| pair(points[a], points[b]))
                .collect();
            for e in mesh.getattr("edges")?.try_iter()? {
                let e = e?;
                if wanted.contains(&pair(
                    crate::math::xyz(&e.getattr("a")?)?,
                    crate::math::xyz(&e.getattr("b")?)?,
                )) {
                    e.setattr("soft", true)?
                }
            }
        }
        self.meshes.insert(k, mesh.clone());
        Ok(mesh)
    }
    fn placed<'a>(
        &mut self,
        py: Python<'p>,
        dae: &mut Dae<'a>,
        n: N<'a>,
        pre: Frame,
        depth: usize,
    ) -> PyResult<Obj<'p>> {
        let name = own(n, "name").filter(|s| !s.is_empty()).unwrap_or(label(n));
        self.names.push(name.into());
        let f = dae.frame(n, pre)?;
        let target = child(n, "instance_node").and_then(|e| dae.target(e));
        let body = target.unwrap_or(n);
        let bf = if target.is_some() {
            dae.frame(body, f)?
        } else {
            f
        };
        let g = group(py, &self.mesh(py, dae, body, bf)?, name)?;
        let transform =
            if child(n, "matrix").is_some() || pre.shift != DVec3::ZERO && f.shift == DVec3::ZERO {
                Some(mapped(py, dae.zup * f.local * dae.zup.inverse())?)
            } else {
                None
            };
        g.setattr("xform", transform)?;
        if let Some(layer) = own(n, "layer").filter(|s| !s.is_empty()) {
            g.setattr("layer", layer)?;
            if !self.layers.iter().any(|s| s == layer) {
                self.layers.push(layer.into())
            }
        }
        if depth < 32 {
            let children = PyList::empty(py);
            for c in kids(body, "node") {
                let kid = self.placed(py, dae, c, bf, depth + 1)?;
                if kid.getattr("xform")?.is_none() {
                    kid.setattr("xform", mapped(py, DMat4::IDENTITY)?)?
                }
                children.append(kid)?;
            }
            if !children.is_empty() {
                g.call_method1("adopt", (children,))?;
            }
        }
        g.setattr("component", target.is_some())?;
        if let Some(bb) = own(n, "billboard").filter(|s| !s.is_empty()) {
            if bb == "mesh" {
                g.setattr("billboard", "mesh")?
            } else {
                g.setattr("billboard", true)?
            }
        }
        Ok(g)
    }
}
#[pyclass]
struct DaeImport {
    groups: Py<PyList>,
    registry: Py<PyDict>,
    layers: Vec<String>,
    names: Vec<String>,
    offset: Py<PyAny>,
    datum: Option<[f64; 3]>,
    loose: Vec<Py<PyAny>>,
    editable: Option<(Vec<Face>, Vec<(DVec3, DVec3)>)>,
}
fn tick(cb: Option<&Obj<'_>>, f: f64, text: &str) -> PyResult<()> {
    if let Some(cb) = cb {
        cb.call1((f, text))?;
    }
    Ok(())
}
fn geo(n: N<'_>) -> Option<[f64; 3]> {
    let location = under(n, &["asset", "coverage", "geographic_location"])?;
    let lat = val(child(location, "latitude")).parse().ok()?;
    let lon = val(child(location, "longitude")).parse().ok()?;
    let alt = val(child(location, "altitude"));
    Some([
        lat,
        lon,
        if alt.is_empty() {
            0.
        } else {
            alt.parse().ok()?
        },
    ])
}
fn read_xml(py: Python<'_>, path: &Path) -> PyResult<String> {
    let bytes = py.detach(|| std::fs::read(path))?;
    if let Ok(text) = std::str::from_utf8(&bytes) {
        return Ok(text.trim_start_matches('\u{feff}').into());
    }
    let enc = if bytes.starts_with(&[255, 254]) || bytes.starts_with(&[254, 255]) {
        "utf-16".to_owned()
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
        let r = regex::Regex::new(r#"encoding\s*=\s*["']([^"']+)["']"#).unwrap();
        r.captures(&head)
            .map(|c| c[1].to_string())
            .unwrap_or("utf-8".into())
    };
    pyo3::types::PyBytes::new(py, &bytes)
        .call_method1("decode", (enc,))?
        .extract()
}
fn stage<'p>(
    py: Python<'p>,
    path: &Path,
    registry: &Bound<'p, PyDict>,
    limits: Limits,
    empty: bool,
    cb: Option<&Obj<'p>>,
) -> PyResult<DaeImport> {
    tick(cb, 0.02, "Reading file…")?;
    let input = read_xml(py, path)?;
    let xml = py
        .detach(|| Document::parse(&input))
        .map_err(|e| bad(format!("Invalid COLLADA XML: {e}")))?;
    let mut dae = py.detach(|| Dae::new(&xml, path))?;
    if py.detach(|| dae.placements(limits.placements))? > limits.placements {
        return Err(bad(format!("The COLLADA file places its components more than {} times: it looks malformed, so it is not imported.",limits.placements)));
    }
    let (offset, shift) = dae.survey(py)?;
    let frame = Frame::root(shift);
    let groups = PyList::empty(py);
    let mut staged = DaeImport {
        groups: groups.clone().unbind(),
        registry: registry.clone().unbind(),
        layers: vec![],
        names: vec![],
        offset: offset.unbind(),
        datum: geo(dae.root),
        loose: vec![],
        editable: None,
    };
    if own(dae.root, "structure").is_some() {
        let mut builder = Structured {
            registry: registry.clone(),
            mat_names: HashMap::new(),
            meshes: HashMap::new(),
            names: vec![],
            layers: vec![],
        };
        let tops = dae.tops.clone();
        for (i, n) in tops.iter().copied().enumerate() {
            tick(
                cb,
                0.1 + 0.85 * i as f64 / tops.len() as f64,
                "Building the model…",
            )?;
            if n.attribute("id") == Some("loose") {
                let f = dae.frame(n, frame)?;
                staged
                    .loose
                    .push(builder.mesh(py, &mut dae, n, f)?.unbind());
            } else {
                groups.append(builder.placed(py, &mut dae, n, frame, 0)?)?
            }
        }
        staged.layers = builder.layers;
        staged.names = builder.names;
    } else {
        let tops = dae.tops.clone();
        let total = tops
            .iter()
            .fold(0usize, |v, &n| v.saturating_add(dae.count(n, 0)));
        if total > limits.fuse {
            tick(cb, 0.1, "Collecting geometry…")?;
            dae.reference(py, frame, limits, &groups, cb)?
        } else {
            let mut pieces = Pieces::default();
            let mut pending = vec![];
            let mut lines = vec![];
            let opts = Collect {
                instances: false,
                ..limits.collect
            };
            for n in tops {
                dae.lines(n, frame, 0, false, &mut lines)?;
                let mut direct = vec![];
                dae.collect(py, n, frame, 0, false, &mut direct, &mut pieces, opts)?;
                if dae.sprite(py, &direct)? {
                    pieces.sprites.push((label(n).into(), direct))
                } else {
                    pending.extend(direct)
                }
                let f = dae.frame(n, frame)?;
                for c in kids(n, "node") {
                    if let Some(g) = dae.small_group(py, c, f, 0, &mut pieces, opts)? {
                        groups.append(g)?
                    }
                }
            }
            let sprites = !pieces.sprites.is_empty();
            for (name, faces) in pieces.sprites {
                let m = fused(py, &faces)?;
                if m.getattr("faces")?.is_truthy()? {
                    let g = group(py, &m, &name)?;
                    g.setattr("billboard", "mesh")?;
                    groups.append(g)?
                }
            }
            if pending.is_empty() && !sprites && groups.is_empty() {
                for g in dae
                    .root
                    .descendants()
                    .filter(|e| e.has_tag_name((NS, "geometry")))
                {
                    let geom = py.detach(|| dae.geom(g))?;
                    let pts = dae.points(&geom, Frame::root(DVec3::ZERO));
                    for prim in &geom.prims {
                        for p in &prim.loops {
                            pending.push(face(p, &pts, prim, &Paint::default()));
                        }
                    }
                }
            }
            if pending.is_empty() && groups.is_empty() && lines.is_empty() && !sprites {
                return Err(bad("No geometry found in the COLLADA file"));
            }
            if pending.len() > limits.fuse {
                let m = fused(py, &pending)?;
                groups.append(group(
                    py,
                    &m,
                    &path.file_stem().unwrap_or_default().to_string_lossy(),
                )?)?;
            } else if !pending.is_empty() || !lines.is_empty() {
                if empty {
                    let m = new_mesh(py)?;
                    editable(&m, &pending, &lines)?;
                    staged.loose.push(m.unbind())
                } else {
                    staged.editable = Some((pending, lines));
                }
            }
        }
    }
    tick(cb, 1., "Done")?;
    Ok(staged)
}
#[pyfunction]
#[pyo3(signature=(scene,path,progress=None))]
fn dae_load<'p>(
    py: Python<'p>,
    scene: &Obj<'p>,
    path: PathBuf,
    progress: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let registry = scene.getattr("materials")?;
    let registry = if registry.is_none() {
        PyDict::new(py)
    } else {
        py.get_type::<PyDict>().call1((&registry,))?.cast_into()?
    };
    let registry = registry.unbind();
    let dst = scene.getattr("mesh")?;
    let empty = dst.getattr("vertices")?.len()? == 0;
    let limits = Limits::get(py)?;
    let report = progress.is_some();
    let result = crate::file_open::DocumentOpen::stage(py, progress, move |py, cb| {
        let staged = stage(
            py,
            &path,
            registry.bind(py),
            limits,
            empty,
            report.then_some(&cb),
        )?;
        Ok(Py::new(py, staged)?.into_any())
    })?;
    let staged = result.bind(py).cast::<DaeImport>()?.borrow();
    for m in &staged.loose {
        place_mesh(py, &dst, m.bind(py))?
    }
    if let Some((faces, lines)) = &staged.editable {
        editable(&dst, faces, lines)?
    }
    let materials = scene.getattr("materials")?;
    if materials.is_none() {
        scene.setattr("materials", staged.registry.bind(py))?
    } else {
        materials.call_method1("update", (staged.registry.bind(py),))?;
    }
    let layers = scene.getattr("layers")?;
    let mut known = HashSet::new();
    for l in layers.try_iter()? {
        known.insert(l?.getattr("key")?.extract::<String>()?);
    }
    for l in &staged.layers {
        if known.insert(l.clone()) {
            layers.call_method1(
                "append",
                (py.import("core.layers")?.call_method1("Layer", (l,))?,),
            )?;
        }
    }
    if !staged.names.is_empty() {
        py.import("core.group")?
            .call_method1("reserve_group_names", (&staged.names,))?;
    }
    if let Some([lat, lon, alt]) = staged.datum {
        if scene.getattr("georef")?.is_none() {
            scene.setattr(
                "georef",
                py.import("georef.datum")?
                    .call_method1("SceneDatum", (lat, lon, alt))?,
            )?
        }
    }
    scene
        .getattr("groups")?
        .call_method1("extend", (staged.groups.bind(py),))?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)?;
    Ok(staged.offset.bind(py).clone())
}
#[pyfunction]
fn dae_placements(py: Python<'_>, root: &Obj<'_>) -> PyResult<usize> {
    let s = py
        .import("xml.etree.ElementTree")?
        .call_method1("tostring", (root, "unicode"))?
        .extract::<String>()?;
    let xml = Document::parse(&s).map_err(bad)?;
    Dae::new(&xml, Path::new(""))?.placements(Limits::get(py)?.placements)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dae_load, m)?)?;
    m.add_function(wrap_pyfunction!(dae_placements, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_structured_scene_has_polygons_runs_and_shared_placements() {
        let xml =
            Document::parse(include_str!("../../../tests/fixtures/dae/structured.dae")).unwrap();
        let mut d = Dae::new(&xml, Path::new("scene.dae")).unwrap();
        assert_eq!(own(d.root, "structure"), Some("1"));
        assert_eq!(d.tops.len(), 5);
        assert!(d.placements(100).unwrap() > 5);
        let g = d.geom(d.ids["geom0"]).unwrap();
        assert!(g.prims.iter().any(|p| p.runs.iter().any(|&n| n > 1)));
        assert!(g.prims.iter().any(|p| !p.lines.is_empty()));
        assert!(d.uses.values().any(|&n| n > 1));
    }
}
