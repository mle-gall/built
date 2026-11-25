// SPDX-License-Identifier: GPL-3.0-or-later
//! Arc inference reads the mesh's disk/radial incidence. Only candidates
//! handed to the machine need a document handle.
use super::{view_queries::ViewportToolHost, P3};
use crate::{
    math::{xyz, Vec3},
    store::{keys::Kind, py::PyMeshStore, Store},
};
use glam::{DMat4, DVec3};
use pyo3::{prelude::*, types::PyDict};
use serde_json::{json, Value};
use std::collections::HashSet;

fn point(o: &Bound<'_, PyAny>, attr: &str) -> PyResult<DVec3> {
    xyz(&o.getattr(attr)?)
}
fn interior(p: DVec3, a: DVec3, b: DVec3) -> bool {
    let tol = 1e-4;
    if p.distance(a) <= tol || p.distance(b) <= tol {
        return false;
    }
    let d = b - a;
    if d.length() < tol {
        return false;
    }
    let t = (p - a).dot(d) / d.length_squared();
    (-tol..=1. + tol).contains(&t) && p.distance(a + t * d) < tol
}
fn position(s: &Store, slot: u32) -> DVec3 {
    DVec3::from(*s.state.vpos.get(slot as usize))
}
fn ends(s: &Store, edge: u32) -> [DVec3; 2] {
    s.state.ends.get(edge as usize).map(|v| position(s, v))
}

fn tangent(mesh: &Bound<'_, PyAny>, p: DVec3) -> PyResult<Option<DVec3>> {
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        if e.getattr("curve")?.is_none()
            || (point(&e, "a")?.distance(p) > 1e-6 && point(&e, "b")?.distance(p) > 1e-6)
        {
            continue;
        }
        let mut points = Vec::new();
        let mut seen = HashSet::new();
        let mut incident = Vec::new();
        for c in mesh.call_method1("curve_edges", (&e,))?.try_iter()? {
            let c = c?;
            let [a, b] = [point(&c, "a")?, point(&c, "b")?];
            if a.distance(p) <= 1e-6 {
                incident.push(b)
            } else if b.distance(p) <= 1e-6 {
                incident.push(a)
            }
            for (v, pos) in [("v0", a), ("v1", b)] {
                if seen.insert(c.getattr(v)?.getattr("key")?.extract::<i128>()?) {
                    points.push(pos.to_array())
                }
            }
        }
        let [q] = incident[..] else { continue };
        let Some((center, _)) = crate::snap_curve::snap_circle(mesh.py(), points, 0.01) else {
            continue;
        };
        let radial = p - DVec3::from(center);
        if radial.length() < 1e-9 {
            continue;
        }
        let n = radial.normalize();
        let d = p - q;
        let t = d - n * d.dot(n);
        if t.length() >= 1e-9 {
            return Ok(Some(t.normalize()));
        }
    }
    Ok(None)
}
impl ViewportToolHost {
    fn arc_edge(&self, e: &Bound<'_, PyAny>) -> PyResult<Value> {
        Ok(
            json!({"id":self.id(e)?,"a":point(e,"a")?.to_array(),"b":point(e,"b")?.to_array(),"faces":e.getattr("faces")?.is_truthy()?}),
        )
    }
    pub(super) fn arc_service(&self, py: Python<'_>, verb: &str, arg: &Value) -> PyResult<Value> {
        let view = self.vp.bind(py);
        let mesh = view
            .getattr_opt("scene")?
            .map(|s| s.getattr_opt("mesh"))
            .transpose()?
            .flatten();
        let Some(mesh) = mesh.filter(|m| !m.is_none()) else {
            return Ok(match verb {
                "two_edge_corners" | "corner_edges" | "rounded_corners" => json!([]),
                _ => Value::Null,
            });
        };
        let p = || -> PyResult<DVec3> {
            Ok(serde_json::from_value::<P3>(arg["point"].clone())
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?
                .into())
        };
        let store = mesh
            .getattr("__dict__")?
            .cast::<PyDict>()?
            .get_item("_store")?
            .filter(|s| !s.is_none());
        let edge_view = |key: u128| {
            py.import("built_core.store_mesh")?
                .getattr("EdgeView")?
                .call1((&mesh, key))
        };
        let mut rows = Vec::new();
        match verb {
            "edge_under" => {
                let p = p()?;
                if let Some(e) = view.getattr_opt("_hover_edge")?.filter(|e| !e.is_none()) {
                    if let Some(v) = e.getattr_opt("v0")? {
                        if v.getattr("edges")?.contains(&e)?
                            && interior(p, point(&e, "a")?, point(&e, "b")?)
                        {
                            return self.arc_edge(&e);
                        }
                    }
                }
                if let Some(s) = store {
                    let s = PyMeshStore::read(&s)?;
                    for e in s.inner.live_slots(Kind::Edge) {
                        let [a, b] = ends(&s.inner, e);
                        if interior(p, a, b) {
                            return self.arc_edge(&edge_view(s.inner.key(Kind::Edge, e))?);
                        }
                    }
                } else {
                    for e in mesh.getattr("edges")?.try_iter()? {
                        let e = e?;
                        if interior(p, point(&e, "a")?, point(&e, "b")?) {
                            return self.arc_edge(&e);
                        }
                    }
                }
                return Ok(Value::Null);
            }
            "vertex_degree" => {
                let v = mesh.call_method1("vertex_at", (Vec3::from(p()?),))?;
                return Ok(if v.is_none() {
                    Value::Null
                } else {
                    json!(v.getattr("edges")?.len()?)
                });
            }
            "two_edge_corners" => {
                let min = arg["min_length"].as_f64().unwrap_or(0.);
                if let Some(s) = store {
                    let s = PyMeshStore::read(&s)?;
                    for v in s.inner.live_slots(Kind::Vertex) {
                        let mut adj = s.inner.edges_at(v);
                        let (Some(a), Some(b), None) = (adj.next(), adj.next(), adj.next()) else {
                            continue;
                        };
                        if [a, b]
                            .iter()
                            .all(|&e| s.inner.state.efaces.get(e as usize).len == 0)
                        {
                            continue;
                        }
                        let p = position(&s.inner, v);
                        let far = |e: u32| {
                            let [x, y] = *s.inner.state.ends.get(e as usize);
                            position(&s.inner, if x == v { y } else { x })
                        };
                        let (af, bf) = (far(a), far(b));
                        if af.distance(p) < min || bf.distance(p) < min {
                            continue;
                        }
                        rows.push(json!({"v":p.to_array(),"a":self.arc_edge(&edge_view(s.inner.key(Kind::Edge,a))?)?,"b":self.arc_edge(&edge_view(s.inner.key(Kind::Edge,b))?)?,"a_far":af.to_array(),"b_far":bf.to_array()}));
                    }
                } else {
                    for v in mesh.getattr("vertices")?.try_iter()? {
                        let v = v?;
                        let adj = v.getattr("edges")?;
                        if adj.len()? != 2 {
                            continue;
                        }
                        let edges = adj.try_iter()?.collect::<PyResult<Vec<_>>>()?;
                        let (a, b) = (&edges[0], &edges[1]);
                        if !a.getattr("faces")?.is_truthy()? && !b.getattr("faces")?.is_truthy()? {
                            continue;
                        }
                        if a.call_method0("length")?.extract::<f64>()? < min
                            || b.call_method0("length")?.extract::<f64>()? < min
                        {
                            continue;
                        }
                        rows.push(json!({"v":point(&v,"position")?.to_array(),"a":self.arc_edge(a)?,"b":self.arc_edge(b)?,"a_far":point(&a.call_method1("other",(&v,))?,"position")?.to_array(),"b_far":point(&b.call_method1("other",(&v,))?,"position")?.to_array()}));
                    }
                }
            }
            "tangent_at" => {
                let p = p()?;
                if let Some(f) = view
                    .getattr_opt("reference_tangent")?
                    .filter(|f| f.is_callable())
                {
                    let t = f.call1((Vec3::from(p),))?;
                    return Ok(if t.is_none() {
                        Value::Null
                    } else {
                        json!(xyz(&t)?.to_array())
                    });
                }
                if let Some(t) = tangent(&mesh, p)? {
                    return Ok(json!(t.to_array()));
                }
                if let Some(f) = view.getattr_opt("_placements")?.filter(|f| f.is_callable()) {
                    for g in f.call0()?.try_iter()? {
                        let g = g?;
                        if g.getattr_opt("billboard")?
                            .is_some_and(|b| b.is_truthy().unwrap_or(false))
                        {
                            continue;
                        }
                        let Some(gm) = g.getattr_opt("mesh")?.filter(|m| !m.is_none()) else {
                            continue;
                        };
                        let xf = g
                            .getattr_opt("xform")?
                            .filter(|m| !m.is_none())
                            .map(|m| m.call_method0("data")?.extract::<[f64; 16]>())
                            .transpose()?
                            .map(|v| DMat4::from_cols_array(&v))
                            .unwrap_or(DMat4::IDENTITY);
                        if xf.determinant() == 0. {
                            continue;
                        }
                        if let Some(t) = tangent(&gm, xf.inverse().transform_point3(p))? {
                            let t = xf.transform_vector3(t);
                            if t.length() >= 1e-9 {
                                return Ok(json!(t.normalize().to_array()));
                            }
                        }
                    }
                }
                return Ok(Value::Null);
            }
            "corner_edges" | "rounded_corners" => {
                let edge = self.object(py, arg["edge"].as_u64().unwrap_or_default())?;
                if edge.is_none() {
                    return Ok(json!([]));
                }
                for name in ["v0", "v1"] {
                    let v = edge.getattr(name)?;
                    for e in v.getattr("edges")?.try_iter()? {
                        let e = e?;
                        if e.eq(&edge)? {
                            continue;
                        }
                        if verb == "corner_edges" {
                            rows.push(json!({"v":point(&v,"position")?.to_array(),"b":self.arc_edge(&e)?,"far":point(&e.call_method1("other",(&v,))?,"position")?.to_array()}));
                            continue;
                        }
                        if e.getattr("curve")?.is_none() {
                            continue;
                        }
                        let chain = mesh.call_method1("curve_edges", (&e,))?;
                        let degrees = PyDict::new(py);
                        for c in chain.try_iter()? {
                            let c = c?;
                            for name in ["v0", "v1"] {
                                let end = c.getattr(name)?;
                                let n = degrees
                                    .get_item(&end)?
                                    .map(|v| v.extract::<usize>())
                                    .transpose()?
                                    .unwrap_or(0);
                                degrees.set_item(end, n + 1)?;
                            }
                        }
                        for (end, n) in degrees.iter() {
                            if n.extract::<usize>()? != 1 || end.eq(&v)? {
                                continue;
                            }
                            for b in end.getattr("edges")?.try_iter()? {
                                let b = b?;
                                if chain.contains(&b)?
                                    || !b.getattr("curve")?.is_none()
                                    || b.getattr("soft")?.is_truthy()?
                                {
                                    continue;
                                }
                                rows.push(json!({"v":point(&v,"position")?.to_array(),"edge_far":point(&edge.call_method1("other",(&v,))?,"position")?.to_array(),"v2":point(&end,"position")?.to_array(),"far":point(&b.call_method1("other",(&end,))?,"position")?.to_array(),"b":self.arc_edge(&b)?}));
                            }
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(Value::Array(rows))
    }
}
