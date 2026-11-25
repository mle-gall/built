// SPDX-License-Identifier: GPL-3.0-or-later
//! A definition is drawn in its own coordinates, even if the selected instance
//! sits elsewhere. The pick plan supplies rings and paints for deferred meshes.
use super::Polygon;
use glam::{DMat4, DVec3};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    ops::Range,
};
type Obj<'p> = Bound<'p, PyAny>;
struct Piece<'p> {
    mesh: Obj<'p>,
    transform: DMat4,
    paint: Paint,
    count: usize,
}
#[derive(Default)]
struct Paint {
    color: Option<[f64; 3]>,
    texture: Option<String>,
}
impl Paint {
    fn read(attrs: &Obj) -> PyResult<Self> {
        let Ok(d) = attrs.cast::<PyDict>() else {
            return Ok(Self::default());
        };
        let color = d
            .get_item("color")?
            .filter(|v| !v.is_none())
            .map(|v| -> PyResult<_> {
                Ok([
                    v.get_item(0)?.extract()?,
                    v.get_item(1)?.extract()?,
                    v.get_item(2)?.extract()?,
                ])
            })
            .transpose()?;
        let texture = d
            .get_item("texture")?
            .filter(|v| v.is_truthy().unwrap_or(false))
            .map(|t| {
                t.get_item("path")
                    .and_then(|v| v.extract())
                    .unwrap_or_default()
            });
        Ok(Self { color, texture })
    }
    fn hash(&self, h: &mut DefaultHasher) {
        self.color
            .map(|c| c.map(|x| (x * 1000.).round_ties_even() as i64))
            .hash(h);
        self.texture.hash(h);
    }
    fn rgb(&self) -> Option<[f32; 3]> {
        self.color
            .map(|v| v.map(|f| f as f32))
            .or_else(|| self.texture.as_ref().map(|_| [0.72, 0.66, 0.58]))
    }
}
fn pieces<'p>(root: &Obj<'p>) -> PyResult<Vec<Piece<'p>>> {
    fn collect<'p>(
        g: &Obj<'p>,
        m: DMat4,
        depth: u8,
        count: &Obj<'p>,
        out: &mut Vec<Piece<'p>>,
    ) -> PyResult<()> {
        let mesh = g.getattr("mesh")?;
        let n = count.call1((&mesh,))?.extract()?;
        let paint = g
            .getattr_opt("material")?
            .map(|a| Paint::read(&a))
            .transpose()?
            .unwrap_or_default();
        out.push(Piece {
            mesh,
            transform: m,
            paint,
            count: n,
        });
        if depth <= 16 {
            if let Some(children) = g.getattr_opt("children")?.filter(|c| !c.is_none()) {
                for child in children.try_iter()? {
                    let c = child?;
                    let t = match c.getattr_opt("xform")?.filter(|v| !v.is_none()) {
                        Some(x) => m * DMat4::from_cols_array(&x.call_method0("data")?.extract()?),
                        None => m,
                    };
                    collect(&c, t, depth + 1, count, out)?;
                }
            }
        }
        Ok(())
    }
    let mut all = Vec::new();
    collect(
        root,
        DMat4::IDENTITY,
        0,
        &root.py().import("built_core.mesh")?.getattr("face_count")?,
        &mut all,
    )?;
    Ok(all)
}

struct Deferred<'p> {
    pos: Obj<'p>,
    corners: Obj<'p>,
    face_attr: Obj<'p>,
    paints: Vec<Paint>,
    rings: Vec<Range<usize>>,
}
impl<'p> Deferred<'p> {
    fn load(mesh: &Obj<'p>) -> PyResult<Option<Self>> {
        let py = mesh.py();
        let plan = py
            .import("built_core")?
            .call_method1("plan_of", (mesh,))?;
        if plan.is_none() {
            return Ok(None);
        }
        let source = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
        let mut paints = Vec::new();
        if let Some(table) = source
            .get_item("meta")?
            .cast_into::<PyDict>()?
            .get_item("face_attrs")?
            .filter(|t| !t.is_none())
        {
            let decode = py.import("built_core.formats.igz")?.getattr("_face_attrs_from_json")?;
            for a in table.try_iter()? {
                paints.push(Paint::read(&decode.call1((a?,))?)?);
            }
        }
        Ok(Some(Self {
            pos: plan.getattr("pos")?,
            corners: plan.getattr("corners")?,
            face_attr: source.get_item("face_attr")?,
            paints,
            rings: vec![],
        }))
    }
    fn index_rings(&mut self, mesh: &Obj) -> PyResult<()> {
        let plan = mesh
            .py()
            .import("built_core")?
            .call_method1("plan_of", (mesh,))?;
        let counts: Vec<usize> = plan.getattr("ring_counts")?.extract()?;
        let sizes: Vec<usize> = plan.getattr("ring_sizes")?.extract()?;
        let (mut ring, mut offset) = (0, 0usize);
        for count in counts {
            let group = sizes
                .get(ring..ring + count)
                .filter(|g| !g.is_empty())
                .ok_or_else(|| PyValueError::new_err("Invalid component face rings"))?;
            self.rings.push(offset..offset + group[0]);
            offset += group.iter().sum::<usize>();
            ring += count;
        }
        Ok(())
    }
    fn paint(&self, i: usize) -> PyResult<Option<&Paint>> {
        let k = self.face_attr.get_item(i)?.extract::<isize>()?;
        if k < 0 {
            return Ok(None);
        }
        self.paints
            .get(k as usize)
            .map(Some)
            .ok_or_else(|| PyValueError::new_err("Invalid component face paint"))
    }
}
fn vertex(v: &Obj) -> PyResult<DVec3> {
    Ok(DVec3::from_array(
        v.getattr("position")?.call_method0("toTuple")?.extract()?,
    ))
}

fn mesh_hash(piece: &Piece) -> PyResult<u64> {
    let mut h = DefaultHasher::new();
    let mesh = &piece.mesh;
    let plan = Deferred::load(mesh)?;
    piece.count.hash(&mut h);
    if plan.is_none()
        && mesh
            .get_type()
            .is(&mesh.py().import("built_core.mesh")?.getattr("DeferredMesh")?)
    {
        return Ok(h.finish());
    }
    let (vertices, faces) = match &plan {
        Some(p) => (p.pos.clone(), None),
        None => (mesh.getattr("vertices")?, Some(mesh.getattr("faces")?)),
    };
    let n = vertices.len()?;
    n.hash(&mut h);
    for i in (0..n).step_by((n / 2000).max(1)) {
        let v = vertices.get_item(i)?;
        let p = if plan.is_some() {
            DVec3::from_array(v.extract()?)
        } else {
            vertex(&v)?
        };
        // Source arrays may differ from rebuilt topology below a micrometre.
        p.to_array()
            .map(|v| (v * 1e6).round_ties_even() as i64)
            .hash(&mut h);
    }
    for i in (0..piece.count).step_by((piece.count / 2000).max(1)) {
        if let Some(p) = &plan {
            p.paint(i)?.unwrap_or(&Paint::default()).hash(&mut h);
        } else if let Some(f) = &faces {
            Paint::read(&f.get_item(i)?.getattr("attrs")?)?.hash(&mut h);
        }
    }
    Ok(h.finish())
}
#[pyfunction]
pub(crate) fn component_signature(definition: &Obj) -> PyResult<Vec<(usize, u64)>> {
    let mut measured = HashMap::new();
    let mut sig = Vec::new();
    for piece in pieces(definition)? {
        let id = piece.mesh.as_ptr() as usize;
        let value = match measured.entry(id) {
            std::collections::hash_map::Entry::Occupied(hit) => *hit.get(),
            std::collections::hash_map::Entry::Vacant(slot) => *slot.insert(mesh_hash(&piece)?),
        };
        let mut h = DefaultHasher::new();
        value.hash(&mut h);
        piece.paint.hash(&mut h);
        piece
            .transform
            .to_cols_array()
            .map(f64::to_bits)
            .hash(&mut h);
        sig.push((id, h.finish()));
    }
    Ok(sig)
}
pub(super) fn polygons(definition: &Obj, max_faces: usize) -> PyResult<Vec<Polygon>> {
    let pieces = pieces(definition)?;
    let stride = pieces
        .iter()
        .map(|p| p.count)
        .sum::<usize>()
        .div_ceil(max_faces.max(1))
        .max(1);
    let mut plans = HashMap::new();
    let mut polys = Vec::new();
    let mut offset = 0;
    let eye = DVec3::new(0.62, -0.74, 0.55).normalize();
    let right = DVec3::new(-eye.y, eye.x, 0.).normalize();
    let up = eye.cross(right).normalize();
    let light = DVec3::new(0.35, -0.55, 0.85).normalize();
    for p in pieces {
        let first = stride - 1 - offset % stride;
        offset += p.count;
        if first >= p.count {
            continue;
        }
        let id = p.mesh.as_ptr() as usize;
        if let std::collections::hash_map::Entry::Vacant(e) = plans.entry(id) {
            let mut lazy = Deferred::load(&p.mesh)?;
            if let Some(data) = &mut lazy {
                data.index_rings(&p.mesh)?;
            }
            e.insert(lazy);
        }
        let plan = plans.get(&id).and_then(Option::as_ref);
        let faces = if plan.is_none() {
            Some(p.mesh.getattr("faces")?)
        } else {
            None
        };
        for i in (first..p.count).step_by(stride) {
            let mut pts = Vec::new();
            let rgb = if let Some(plan) = plan {
                let range = plan
                    .rings
                    .get(i)
                    .ok_or_else(|| PyValueError::new_err("Missing face ring"))?;
                for index in range.clone() {
                    let v = plan.pos.get_item(plan.corners.get_item(index)?)?;
                    pts.push(DVec3::from_array(v.extract()?));
                }
                plan.paint(i)?.and_then(Paint::rgb)
            } else {
                let f = faces.as_ref().unwrap().get_item(i)?;
                for v in f.getattr("loop")?.try_iter()? {
                    pts.push(vertex(&v?)?);
                }
                Paint::read(&f.getattr("attrs")?)?.rgb()
            };
            if pts.len() < 3 {
                continue;
            }
            for pt in &mut pts {
                *pt = p.transform.project_point3(*pt);
            }
            let mut normal = DVec3::ZERO;
            for (a, b) in pts.iter().zip(pts.iter().cycle().skip(1)) {
                normal += a.cross(*b);
            }
            if normal.length() < 1e-12 {
                continue;
            }
            let depth = pts.iter().map(|p| p.dot(eye)).sum::<f64>() / pts.len() as f64;
            let xy = pts
                .iter()
                .map(|p| (p.dot(right) as f32, -p.dot(up) as f32))
                .collect();
            polys.push((
                depth as f32,
                xy,
                rgb.or_else(|| p.paint.rgb()).unwrap_or([0.96, 0.95, 0.925]),
                normal.normalize().dot(light).abs() as f32,
            ));
        }
    }
    Ok(polys)
}
