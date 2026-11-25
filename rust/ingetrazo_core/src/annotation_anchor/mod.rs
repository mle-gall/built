// SPDX-License-Identifier: GPL-3.0-or-later
//! Bind native element coordinates to document occurrences and archive them.
use crate::{
    math::{xyz, Vec3},
    protocol::keys::{element, kind_of},
};
use pyo3::{
    exceptions::{PyAttributeError, PyKeyError, PyOverflowError, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::collections::HashSet;
mod pick;
mod placement;
type Obj<'p> = Bound<'p, PyAny>;
type Hit<'p> = (Option<Obj<'p>>, Option<Obj<'p>>);
const MODULE: &str = "ingetrazo_core.annotation_anchor";
fn matrix<'p>(scene: &Obj<'p>, owner: Option<&Obj<'p>>) -> PyResult<Option<Obj<'p>>> {
    let Some(owner) = owner.filter(|o| !o.is_none()) else {
        return Ok(None);
    };
    let m = if let Some(root) = scene.getattr_opt("_root")?.filter(|o| !o.is_none()) {
        root.call_method1("_anchor_matrix", (owner,))?
    } else {
        scene
            .py()
            .import("core.outliner")?
            .call_method1("world_matrix", (scene.getattr("groups")?, owner))?
    };
    Ok((!m.is_none()).then_some(m))
}
fn vertices<'p>(entity: &Obj<'p>, kind: &str) -> PyResult<Vec<Obj<'p>>> {
    match kind {
        "vertex" => Ok(vec![entity.clone()]),
        "edge" => Ok(vec![entity.getattr("v0")?, entity.getattr("v1")?]),
        _ => entity.getattr("loop")?.try_iter()?.collect(),
    }
}
fn coords(v: &[Obj]) -> PyResult<Vec<(u128, [f64; 3])>> {
    v.iter()
        .map(|p| {
            Ok((
                p.getattr("key")?.extract()?,
                xyz(&p.getattr("position")?)?.to_array(),
            ))
        })
        .collect()
}
fn mesh<'p>(scene: &Obj<'p>, owner: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    match owner.filter(|p| !p.is_none()) {
        Some(o) => o.getattr("mesh"),
        None => scene.getattr("loose_mesh"),
    }
}
fn occurrence<'p>(scene: &Obj<'p>, path: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    if !path.is_truthy()? {
        return Ok(None);
    }
    let o = scene.call_method1("resolve_occurrence", (path,))?;
    Ok((!o.is_none()).then_some(o))
}
fn inverse<'p>(m: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let pair = m.call_method0("inverted")?;
    Ok(pair.get_item(1)?.is_truthy()?.then_some(pair.get_item(0)?))
}
fn malformed(py: Python, e: &PyErr) -> bool {
    e.is_instance_of::<PyValueError>(py)
        || e.is_instance_of::<PyTypeError>(py)
        || e.is_instance_of::<PyKeyError>(py)
        || e.is_instance_of::<PyOverflowError>(py)
}
#[pyclass(module = "ingetrazo_core.annotation_anchor", get_all)]
struct Attachment {
    scene: Py<PyAny>,
    binding: Py<PyAny>,
}
impl Attachment {
    fn resolve<'p>(&self, py: Python<'p>) -> PyResult<Hit<'p>> {
        let s = self.scene.bind(py).call0()?;
        if s.is_none() {
            return Ok((None, None));
        }
        let b = self.binding.bind(py);
        let path = b.getattr("path")?;
        let owner = occurrence(&s, &path)?;
        if path.is_truthy()? && owner.is_none() {
            return Ok((None, None));
        }
        let e = element(
            &mesh(&s, owner.as_ref())?,
            &b.getattr("kind")?.extract::<String>()?,
            &b.getattr("key")?,
        )?;
        Ok((e, owner))
    }
    fn support<'p>(&self, py: Python<'p>) -> PyResult<Option<(Vec<Obj<'p>>, Option<Obj<'p>>)>> {
        let (e, owner) = self.resolve(py)?;
        let Some(e) = e else {
            return Ok(None);
        };
        let verts = vertices(
            &e,
            &self.binding.bind(py).getattr("kind")?.extract::<String>()?,
        )?;
        Ok(Some((
            verts,
            matrix(&self.scene.bind(py).call0()?, owner.as_ref())?,
        )))
    }
    fn restore_inner(scene: &Obj, raw: &Obj, world: &Obj) -> PyResult<Option<Self>> {
        let py = scene.py();
        if !raw.is_instance_of::<PyDict>() {
            return Ok(None);
        }
        let kind = raw.get_item("kind")?.extract::<String>()?;
        let path = raw.get_item("path")?;
        let owner = occurrence(scene, &path)?;
        if path.is_truthy()? && owner.is_none() {
            return Ok(None);
        }
        let mesh = mesh(scene, owner.as_ref())?;
        let mut vs = Vec::new();
        for p in raw.get_item("vertices")?.try_iter()? {
            let v = mesh.call_method1("vertex_at", (Vec3::from(xyz(&p?)?),))?;
            if v.is_none() {
                return Ok(None);
            }
            vs.push(v);
        }
        if vs.is_empty() {
            return Ok(None);
        }
        let keys = |v: &[Obj]| -> PyResult<Vec<u128>> {
            v.iter().map(|v| v.getattr("key")?.extract()).collect()
        };
        let e = match kind.as_str() {
            "vertex" if vs.len() == 1 => vs[0].clone(),
            "edge" if vs.len() == 2 => mesh.call_method1("find_edge", (&vs[0], &vs[1]))?,
            "face" => {
                let wanted: HashSet<_> = keys(&vs)?.into_iter().collect();
                let mut found = None;
                for f in mesh.call_method1("faces_at", (&vs[0],))?.try_iter()? {
                    let f = f?;
                    if keys(&vertices(&f, "face")?)?
                        .into_iter()
                        .collect::<HashSet<_>>()
                        == wanted
                    {
                        found = Some(f);
                        break;
                    }
                }
                let Some(f) = found else {
                    return Ok(None);
                };
                f
            }
            _ => return Ok(None),
        };
        if e.is_none() {
            return Ok(None);
        }
        let w = raw.call_method1("get", ("weights",))?;
        if w.is_none() {
            return Self::capture(scene, &e, owner.as_ref(), world);
        }
        let weights = w
            .try_iter()?
            .map(|p| {
                let p = p?;
                Ok((
                    p.get_item(0)?.extract::<usize>()?,
                    p.get_item(1)?.extract::<f64>()?,
                ))
            })
            .collect::<PyResult<Vec<_>>>()?;
        let b = py
            .import("ingetrazo_core")?
            .getattr("ElementAnchor")?
            .call_method1(
                "restore",
                (&kind, e.getattr("key")?, path, keys(&vs)?, weights),
            )?;
        if b.is_none() {
            Ok(None)
        } else {
            Self::new(scene, b).map(Some)
        }
    }
}
#[pymethods]
impl Attachment {
    #[new]
    fn new(scene: &Obj, binding: Obj) -> PyResult<Self> {
        Ok(Self {
            scene: scene
                .py()
                .import("weakref")?
                .call_method1("ref", (scene,))?
                .unbind(),
            binding: binding.unbind(),
        })
    }
    fn __deepcopy__<'p>(slf: &Bound<'p, Self>, memo: &Obj<'p>) -> PyResult<Obj<'p>> {
        let py = slf.py();
        let this = slf.borrow();
        let scene = this.scene.bind(py).call0()?;
        if scene.is_none() {
            return Ok(slf.as_any().clone());
        }
        let target = memo.call_method1("get", (scene.as_ptr() as usize, &scene))?;
        if target.is(&scene) {
            return Ok(
                Bound::new(py, Self::new(&scene, this.binding.bind(py).clone())?)?.into_any(),
            );
        }
        let out = Self::restore(
            &target,
            &this.archive(py)?.into_pyobject(py)?.into_any(),
            &this.position(py)?.into_pyobject(py)?.into_any(),
        )?;
        Ok(out.into_pyobject(py)?.into_any())
    }
    #[staticmethod]
    #[pyo3(signature=(scene,entity,owner,world))]
    fn capture(
        scene: &Obj,
        entity: &Obj,
        owner: Option<&Obj>,
        world: &Obj,
    ) -> PyResult<Option<Self>> {
        let Some(kind) = kind_of(entity)? else {
            return Ok(None);
        };
        let Some(e) = element(&mesh(scene, owner)?, kind, &entity.getattr("key")?)? else {
            return Ok(None);
        };
        let py = scene.py();
        let path = if let Some(o) = owner {
            let p = scene.call_method1("occurrence_path", (o,))?;
            if !p.is_truthy()? {
                return Ok(None);
            }
            p
        } else {
            PyTuple::empty(py).into_any()
        };
        let point = if let Some(m) = matrix(scene, owner)? {
            let Some(inv) = inverse(&m)? else {
                return Ok(None);
            };
            inv.call_method1("map", (world,))?
        } else {
            world.clone()
        };
        let b = py
            .import("ingetrazo_core")?
            .getattr("ElementAnchor")?
            .call_method1(
                "capture",
                (
                    kind,
                    e.getattr("key")?,
                    path,
                    coords(&vertices(&e, kind)?)?,
                    xyz(&point)?.to_array(),
                ),
            )?;
        if b.is_none() {
            Ok(None)
        } else {
            Self::new(scene, b).map(Some)
        }
    }
    fn position<'p>(&self, py: Python<'p>) -> PyResult<Option<Obj<'p>>> {
        let Some((v, m)) = self.support(py)? else {
            return Ok(None);
        };
        let b = self.binding.bind(py);
        let p = b.call_method1("position", (coords(&v)?,))?;
        if p.is_none() {
            return Ok(None);
        }
        let point = Bound::new(py, Vec3::from(xyz(&p)?))?.into_any();
        Ok(Some(if let Some(m) = m {
            m.call_method1("map", (point,))?
        } else {
            point
        }))
    }
    fn placement_frame<'p>(
        &self,
        py: Python<'p>,
        parent_matrix: Option<&Obj<'p>>,
    ) -> PyResult<Option<Obj<'p>>> {
        let Some((v, m)) = self.support(py)? else {
            return Ok(None);
        };
        let mut m = match m {
            Some(m) => m,
            None => py.import("ingetrazo_core")?.call_method0("Matrix4")?,
        };
        if let Some(p) = parent_matrix {
            let Some(inv) = inverse(p)? else {
                return Ok(None);
            };
            m = inv.mul(m)?;
        }
        Ok(Some(self.binding.bind(py).call_method1(
            "placement_frame",
            (coords(&v)?, m.call_method0("data")?),
        )?))
    }
    fn before_mesh_swap<'p>(
        &self,
        py: Python<'p>,
    ) -> PyResult<Option<(Option<Obj<'p>>, Vec<Obj<'p>>)>> {
        let Some((v, m)) = self.support(py)? else {
            return Ok(None);
        };
        let mut points = Vec::new();
        for vertex in v {
            let p = vertex.getattr("position")?;
            points.push(if let Some(m) = &m {
                m.call_method1("map", (p,))?
            } else {
                p
            });
        }
        Ok(Some((self.position(py)?, points)))
    }
    fn after_mesh_swap<'p>(slf: &Bound<'p, Self>, snapshot: &Obj<'p>) -> PyResult<Bound<'p, Self>> {
        let py = slf.py();
        let this = slf.borrow();
        if snapshot.is_none() || this.position(py)?.is_some() {
            return Ok(slf.clone());
        }
        let scene = this.scene.bind(py).call0()?;
        let b = this.binding.bind(py);
        let path = b.getattr("path")?;
        let owner = occurrence(&scene, &path)?;
        let inv = if let Some(m) = matrix(&scene, owner.as_ref())? {
            let Some(inv) = inverse(&m)? else {
                return Ok(slf.clone());
            };
            Some(inv)
        } else {
            None
        };
        let mut vs = Vec::new();
        for p in snapshot.get_item(1)?.try_iter()? {
            let p = p?;
            let p = if let Some(i) = &inv {
                i.call_method1("map", (p,))?
            } else {
                p
            };
            vs.push(p.call_method0("toTuple")?);
        }
        let d = PyDict::new(py);
        d.set_item("kind", b.getattr("kind")?)?;
        d.set_item("path", path)?;
        d.set_item("vertices", vs)?;
        match Self::restore(&scene, &d, &snapshot.get_item(0)?)? {
            Some(a) => Bound::new(py, a),
            None => Ok(slf.clone()),
        }
    }
    fn archive<'p>(&self, py: Python<'p>) -> PyResult<Option<Bound<'p, PyDict>>> {
        let (e, _) = self.resolve(py)?;
        let Some(e) = e else {
            return Ok(None);
        };
        let b = self.binding.bind(py);
        let kind = b.getattr("kind")?.extract::<String>()?;
        let vs = vertices(&e, &kind)?;
        let mut keys = Vec::new();
        let mut points = Vec::new();
        for v in vs {
            keys.push(v.getattr("key")?);
            points.push(v.getattr("position")?.call_method0("toTuple")?);
        }
        let out = PyDict::new(py);
        out.set_item("path", b.getattr("path")?)?;
        out.set_item("kind", kind)?;
        out.set_item("vertices", points)?;
        out.set_item("weights", b.call_method1("indexed_weights", (keys,))?)?;
        Ok(Some(out))
    }
    #[staticmethod]
    fn restore(scene: &Obj, raw: &Obj, world: &Obj) -> PyResult<Option<Self>> {
        match Self::restore_inner(scene, raw, world) {
            Err(e) if malformed(scene.py(), &e) => Ok(None),
            r => r,
        }
    }
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, MODULE)?;
    sub.add_class::<Attachment>()?;
    placement::register(&sub)?;
    pick::register(&sub)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item(MODULE, &sub)?;
    m.add("annotation_anchor", sub)
}
