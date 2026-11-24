// SPDX-License-Identifier: GPL-3.0-or-later
//! Hidden-line geometry for the resident view and synchronous sheet capture.
use crate::document_host::*;
use glam::{DMat3, DMat4, DVec3};
use numpy::{IntoPyArray, PyArrayMethods, PyReadonlyArray2};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
fn get<'py>(o: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    o.call_method1("get", (name,))
}
fn visible(scene: &Obj, object: &Obj) -> PyResult<bool> {
    scene.call_method1("entity_visible", (object,))?.is_truthy()
}
fn shown(scene: &Obj, g: &Obj) -> PyResult<bool> {
    Ok(!opt(g, "billboard")?.is_truthy()? && visible(scene, g)?)
}
fn rows(o: &Obj) -> PyResult<Vec<DVec3>> {
    let arr = o
        .py()
        .import("numpy")?
        .call_method1("asarray", (o, "float64"))?
        .call_method1("reshape", (-1, 3))?
        .extract::<PyReadonlyArray2<f64>>()?;
    Ok(arr
        .as_array()
        .rows()
        .into_iter()
        .map(|r| DVec3::new(r[0], r[1], r[2]))
        .collect())
}
fn block(py: Python, points: Vec<DVec3>, stride: usize) -> PyResult<Obj> {
    let n = points.len() / stride;
    let data: Vec<_> = points.into_iter().flat_map(|v| v.to_array()).collect();
    Ok(data.into_pyarray(py).reshape([n, stride, 3])?.into_any())
}
struct Parts<'py> {
    tri: Bound<'py, PyList>,
    hard: Bound<'py, PyList>,
    soft: Bound<'py, PyList>,
    norm: Bound<'py, PyList>,
    hard_tags: Bound<'py, PyList>,
    soft_tags: Bound<'py, PyList>,
}
impl<'py> Parts<'py> {
    fn new(py: Python<'py>) -> Self {
        Self {
            tri: PyList::empty(py),
            hard: PyList::empty(py),
            soft: PyList::empty(py),
            norm: PyList::empty(py),
            hard_tags: PyList::empty(py),
            soft_tags: PyList::empty(py),
        }
    }
    fn loose(&self, scene: &Obj<'py>, tags: bool) -> PyResult<()> {
        let py = scene.py();
        let mut hard = Vec::new();
        let mut soft = Vec::new();
        let mut normals = Vec::new();
        let mut htags = Vec::new();
        let mut stags = Vec::new();
        let layer = py.import("core.layers")?.getattr("layer_of")?;
        for e in scene.getattr("loose_mesh")?.getattr("edges")?.try_iter()? {
            let e = e?;
            if opt(&e, "hidden")?.is_truthy()? || !visible(scene, &e)? {
                continue;
            }
            let ends = [
                xyz(&e.getattr("v0")?.getattr("position")?)?,
                xyz(&e.getattr("v1")?.getattr("position")?)?,
            ];
            if opt(&e, "soft")?.is_truthy()? {
                let mut faces = Vec::new();
                for f in e.getattr("faces")?.try_iter()? {
                    let f = f?;
                    if visible(scene, &f)? {
                        faces.push(f);
                        if faces.len() == 2 {
                            break;
                        }
                    }
                }
                if faces.is_empty() {
                    continue;
                }
                soft.extend(ends);
                normals.push(xyz(&faces[0].call_method0("normal")?)?);
                normals.push(if faces.len() == 2 {
                    xyz(&faces[1].call_method0("normal")?)?
                } else {
                    DVec3::NAN
                });
                if tags {
                    stags.push(layer.call1((&e,))?)
                }
            } else {
                hard.extend(ends);
                if tags {
                    htags.push(layer.call1((&e,))?)
                }
            }
        }
        if !hard.is_empty() {
            self.hard.append(block(py, hard, 2)?)?;
            if tags {
                self.hard_tags.append(objects(py, htags)?)?
            }
        }
        if !soft.is_empty() {
            self.soft.append(block(py, soft, 2)?)?;
            self.norm.append(block(py, normals, 2)?)?;
            if tags {
                self.soft_tags.append(objects(py, stags)?)?
            }
        }
        Ok(())
    }
    fn finish(&self, tri_tags: Option<Obj<'py>>) -> PyResult<Obj<'py>> {
        let py = self.tri.py();
        let np = py.import("numpy")?;
        let mut out = Vec::new();
        for (parts, n) in [
            (&self.tri, 3),
            (&self.hard, 2),
            (&self.soft, 2),
            (&self.norm, 2),
        ] {
            out.push(if parts.is_empty() {
                np.call_method1("empty", ((0, n, 3),))?
            } else {
                np.call_method1("concatenate", (parts,))?
            });
        }
        if let Some(tags) = tri_tags {
            out.push(tags);
            for parts in [&self.hard_tags, &self.soft_tags] {
                out.push(if parts.is_empty() {
                    np.call_method1("empty", (0, "object"))?
                } else {
                    np.call_method1("concatenate", (parts,))?
                });
            }
        }
        Ok(PyTuple::new(py, out)?.into_any())
    }
}
fn objects<'py>(py: Python<'py>, tags: Vec<Obj<'py>>) -> PyResult<Obj<'py>> {
    py.import("numpy")?
        .call_method1("asarray", (tags, "object"))
}
fn surfaces<'py>(
    base: &Obj<'py>,
    parts: &Parts<'py>,
    xs: &[Option<DMat4>],
    solid: bool,
) -> PyResult<usize> {
    let py = base.py();
    if solid {
        let origins = get(base, "v0")?;
        if !origins.is_none() && origins.len()? > 0 {
            let origin = rows(&origins)?;
            let e1 = rows(&base.get_item("e1")?)?;
            let e2 = rows(&base.get_item("e2")?)?;
            let mut out = Vec::new();
            for xf in xs {
                for ((&o, &u), &v) in origin.iter().zip(&e1).zip(&e2) {
                    let (a, b, c) = match xf {
                        Some(m) => (
                            m.transform_point3(o),
                            m.transform_vector3(u),
                            m.transform_vector3(v),
                        ),
                        None => (o, u, v),
                    };
                    out.extend([a, a + b, a + c])
                }
            }
            parts.tri.append(block(py, out, 3)?)?
        }
        let edges = get(base, "edges")?;
        if edges.is_truthy()? {
            let np = py.import("numpy")?;
            let pts = rows(&np.call_method1("frombuffer", (edges, "float32"))?)?;
            let mut out = Vec::new();
            for xf in xs {
                out.extend(
                    pts.iter()
                        .map(|p| xf.map_or(*p, |m| m.transform_point3(*p).as_vec3().as_dvec3())),
                )
            }
            parts.hard.append(block(py, out, 2)?)?
        }
    }
    let soft = get(base, "soft_pts")?;
    if soft.is_none() || soft.len()? == 0 {
        return Ok(0);
    }
    let pts = rows(&soft)?;
    let n0 = rows(&base.get_item("soft_n0")?)?;
    let n1 = rows(&base.get_item("soft_n1")?)?;
    let one = base
        .get_item("soft_single")?
        .call_method0("tolist")
        .or_else(|_| base.get_item("soft_single"))?
        .extract::<Vec<bool>>()?;
    let mut out = Vec::new();
    let mut normals = Vec::new();
    for xf in xs {
        let normal = xf.map(|m| {
            let l = DMat3::from_mat4(m);
            if l.determinant() == 0. {
                DMat3::IDENTITY
            } else {
                l.inverse().transpose()
            }
        });
        let direction = |p: DVec3| {
            normal.map_or(p, |m| {
                let a = m * p;
                let l = a.length();
                if l > 1e-12 {
                    a / l
                } else {
                    a
                }
            })
        };
        out.extend(
            pts.iter()
                .map(|p| xf.map_or(*p, |m| m.transform_point3(*p).as_vec3().as_dvec3())),
        );
        for ((&a, &b), &single) in n0.iter().zip(&n1).zip(&one) {
            normals.extend([direction(a), if single { DVec3::NAN } else { direction(b) }]);
        }
    }
    let n = out.len() / 2;
    parts.soft.append(block(py, out, 2)?)?;
    parts.norm.append(block(py, normals, 2)?)?;
    Ok(n)
}
#[pyfunction]
fn batch_hlr_instances(
    base: &Obj,
    xfs: &Obj,
    tri: &Bound<PyList>,
    hard: &Bound<PyList>,
    soft: &Bound<PyList>,
    norm: &Bound<PyList>,
) -> PyResult<()> {
    let parts = Parts {
        tri: tri.clone(),
        hard: hard.clone(),
        soft: soft.clone(),
        norm: norm.clone(),
        ..Parts::new(base.py())
    };
    let mut matrices = Vec::new();
    for xf in xfs.try_iter()? {
        matrices.push(Some(DMat4::from_cols_array(
            &xf?.call_method0("data")?.extract()?,
        )))
    }
    surfaces(base, &parts, &matrices, true)?;
    Ok(())
}
#[pyfunction]
fn hlr_geometry_now<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    let pieces = Parts::new(py);
    let triangulate = opt(v, "_tris_of")?;
    let mut tris = Vec::new();
    for face in scene.getattr("faces")?.try_iter()? {
        let f = face?;
        if !visible(&scene, &f)? {
            continue;
        }
        let ts = if triangulate.is_truthy()? {
            triangulate.call1((&f,))?
        } else {
            f.call_method0("triangulate")?
        };
        for t in ts.try_iter()? {
            let pts = t?.extract::<[Obj; 3]>()?;
            for p in pts {
                tris.push(xyz(&p)?)
            }
        }
    }
    if !tris.is_empty() {
        pieces.tri.append(block(py, tris, 3)?)?
    }
    pieces.loose(&scene, false)?;
    let batches = PyDict::new(py);
    let cache = opt(v, "_inst_chunks")?;
    let sheet = py.import("types")?.call_method0("SimpleNamespace")?;
    sheet.setattr("scene", &scene)?;
    let h = host(py)?;
    let class = h.getattr("DocumentInteraction")?;
    for root in scene.getattr("groups")?.try_iter()? {
        let placements = class.call_method1("_expand_placements", (&sheet, root?))?;
        for g in placements.try_iter()? {
            let mut g = g?;
            if !shown(&scene, &g)? {
                continue;
            }
            let xf = opt(&g, "xform")?;
            let wrap = |g: &Obj<'py>| {
                h.call_method1(
                    "_proto_wrapper",
                    (v, g.getattr("mesh")?, h.call_method1("_paint_of", (v, g))?),
                )
            };
            if !xf.is_none() && !opt(&g, "mesh")?.is_none() {
                let base = v.call_method1("_group_chunk", (wrap(&g)?,))?;
                let cached = if cache.is_none() {
                    none(py)
                } else {
                    cache.call_method1("get", (id(&g),))?
                };
                if cached.is_none()
                    || !cached.get_item("ikey")?.eq((
                        base.get_item("uid")?,
                        py.import("builtins")?
                            .call_method1("tuple", (xf.call_method0("data")?,))?,
                        get(&base, "rev")?,
                    ))?
                {
                    let key = id(&base);
                    let list = if let Some(pair) = batches.get_item(key)? {
                        pair.get_item(1)?
                    } else {
                        let l = PyList::empty(py);
                        batches.set_item(key, (&base, &l))?;
                        l.into_any()
                    };
                    list.call_method1("append", (xf,))?;
                    continue;
                }
            } else if xf.is_none() && !opt(&g, "owner")?.is_none() {
                g = wrap(&g)?
            }
            surfaces(
                &v.call_method1("_group_chunk", (g,))?,
                &pieces,
                &[None],
                true,
            )?;
        }
    }
    for pair in batches.values() {
        h.call_method1(
            "_batch_instances",
            (
                pair.get_item(0)?,
                pair.get_item(1)?,
                &pieces.tri,
                &pieces.hard,
                &pieces.soft,
                &pieces.norm,
            ),
        )?;
    }
    pieces.finish(None)
}
#[pyfunction]
fn hlr_geometry<'py>(v: &Obj<'py>, tags: bool) -> PyResult<Obj<'py>> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    let kw = PyDict::new(py);
    kw.set_item("near", "all")?;
    let idx = v.call_method1(
        "_pick_flat",
        (v.call_method("_pick_index", (), Some(&kw))?,),
    )?;
    let p = Parts::new(py);
    let np = py.import("numpy")?;
    let v0 = opt(&idx, "tri_v0")?;
    let mut tri_tags = None;
    if !v0.is_none() && v0.len()? > 0 {
        let ent = idx.getattr("tri_ent")?;
        let keep = idx.getattr("ent_vis")?.get_item(&ent)?;
        let a = v0.get_item(&keep)?;
        p.tri.append(
            np.call_method1(
                "stack",
                (
                    (
                        &a,
                        a.add(idx.getattr("tri_e1")?.get_item(&keep)?)?,
                        a.add(idx.getattr("tri_e2")?.get_item(&keep)?)?,
                    ),
                    1,
                ),
            )?
            .call_method1("astype", ("float64",))?,
        )?;
        if tags {
            let flags = PyDict::new(py);
            flags.set_item("return_inverse", true)?;
            let (ids, back): (Obj, Obj) = np
                .call_method("unique", (ent.get_item(keep)?,), Some(&flags))?
                .extract()?;
            tri_tags = Some(
                host(py)?
                    .getattr("DocumentInteraction")?
                    .call_method1("_cut_layer_tags", (&idx, ids))?
                    .get_item(back)?,
            )
        }
    } else if tags {
        tri_tags = Some(np.call_method1("empty", (0, "object"))?)
    }
    let ga = opt(&idx, "gedge_a")?;
    let layer = py.import("core.layers")?.getattr("layer_of")?;
    if !ga.is_none() && ga.len()? > 0 {
        let groups = idx.getattr("gedge_groups")?;
        let gi = idx.getattr("gedge_gi")?;
        let mut visible = Vec::new();
        for g in groups.try_iter()? {
            visible.push(shown(&scene, &g?)?)
        }
        let sel = if visible.is_empty() {
            np.call_method1("zeros", (ga.len()?, "bool"))?
        } else {
            visible.into_pyarray(py).get_item(&gi)?
        };
        p.hard.append(
            np.call_method1(
                "stack",
                (
                    (ga.get_item(&sel)?, idx.getattr("gedge_b")?.get_item(&sel)?),
                    1,
                ),
            )?
            .call_method1("astype", ("float64",))?,
        )?;
        if tags {
            let mut owners = Vec::new();
            for g in groups.try_iter()? {
                owners.push(layer.call1((g?,))?)
            }
            owners.push(py.import("core.layers")?.getattr("DEFAULT_LAYER")?);
            let t = objects(py, owners)?.get_item(gi.get_item(&sel)?)?;
            let own = opt(&idx, "gedge_layer")?;
            if !own.is_none() {
                for (i, o) in own.get_item(&sel)?.try_iter()?.enumerate() {
                    let o = o?;
                    if !o.is_none() {
                        t.set_item(i, o)?
                    }
                }
            }
            p.hard_tags.append(t)?
        }
    }
    p.loose(&scene, tags)?;
    for g in v.call_method0("_placements")?.try_iter()? {
        let g = g?;
        if !shown(&scene, &g)? {
            continue;
        }
        let ch = v.call_method1("_group_chunk", (&g,))?;
        let n = surfaces(&ch, &p, &[None], false)?;
        if tags && n > 0 {
            let owner = layer.call1((&g,))?;
            let own = get(&ch, "soft_layer")?;
            let mut t = vec![owner; n];
            if !own.is_none() && own.len()? == n {
                for (i, x) in own.try_iter()?.enumerate() {
                    let x = x?;
                    if x.is_truthy()? {
                        t[i] = x
                    }
                }
            }
            p.soft_tags.append(objects(py, t)?)?
        }
    }
    p.finish(tri_tags)
}

pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(batch_hlr_instances, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_geometry_now, m)?)?;
    Ok(())
}
