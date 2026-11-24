// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve a definition's rings and flagged segments once in local metres.
//! A transformed placement borrows a read-only span for each resulting ring.
use super::tree::{attr, either, flagged, none, values, Obj};
use glam::{DMat4, DVec3};
use numpy::{ndarray::Array2, IntoPyArray, PyReadonlyArray2};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
use std::collections::HashSet;
fn array<'p>(py: Python<'p>, coords: Vec<f64>) -> Obj<'p> {
    Array2::from_shape_vec((coords.len() / 3, 3), coords)
        .expect("complete coordinate triples")
        .into_pyarray(py)
        .into_any()
}
fn cache<'p>(d: &Obj<'p>, key: &str, v: &Obj<'p>, bytes: usize) -> PyResult<()> {
    d.py()
        .import("formats.skp_consumption")?
        .call_method1("_remember_geometry", (d, key, v, bytes))?;
    Ok(())
}
#[pyfunction]
pub fn skp_defn_geom<'p>(d: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = d.py();
    if let Some(v) = d.getattr_opt("_geom")? {
        return Ok(v);
    }
    let flat = PyList::empty(py);
    let meta = PyList::empty(py);
    let mut coords = Vec::new();
    let mut holes = 0;
    for (i, f) in d
        .getattr("faces")?
        .call_method0("values")?
        .try_iter()?
        .enumerate()
    {
        if i % 256 == 0 {
            crate::file_open::skp_checkpoint(py)?;
        }
        let f = f?;
        let loops = values(&f, "loops")?;
        let mut spans = Vec::new();
        for (i, l) in loops.iter().enumerate() {
            let raw = super::skp_ring(d, l)?;
            let Some(raw) = raw.filter(|r| r.len() >= 3) else {
                if i == 0 {
                    break;
                }
                continue;
            };
            let start = flat.len();
            for p in raw {
                let xyz = p.extract::<[f64; 3]>()?;
                coords.extend(xyz.map(|x| x * 0.0254));
                flat.append(p)?;
            }
            spans.push((start, flat.len()));
        }
        if let Some((outer, inner)) = spans.split_first() {
            holes += inner.len();
            meta.append((f, outer.0, outer.1, inner.to_vec()))?;
        }
    }
    let bytes = coords.len() * 8 + flat.len() * 96 + meta.len() * 256 + holes * 128;
    let geom = (array(py, coords), flat, meta)
        .into_pyobject(py)?
        .into_any();
    cache(d, "_geom", &geom, bytes)?;
    Ok(geom)
}
fn mapped<'p>(arr: &Obj<'p>, xf: &Obj<'p>) -> PyResult<Obj<'p>> {
    if xf.call_method0("isIdentity")?.is_truthy()? {
        return Ok(arr.clone());
    }
    let m = DMat4::from_cols_array(&xf.call_method0("data")?.extract()?);
    let a = arr.extract::<PyReadonlyArray2<f64>>()?;
    let points = a
        .as_array()
        .outer_iter()
        .map(|p| DVec3::new(p[0], p[1], p[2]))
        .collect::<Vec<_>>();
    let coords = arr.py().detach(|| {
        points
            .into_iter()
            .flat_map(|p| m.transform_point3(p).to_array())
            .collect()
    });
    Ok(array(arr.py(), coords))
}
#[pyfunction]
pub fn skp_world_points<'p>(arr: &Obj<'p>, xf: &Obj<'p>) -> PyResult<Obj<'p>> {
    arr.py()
        .import("formats.skp_coordinates")?
        .call_method1("PointRows", (mapped(arr, xf)?,))
}
fn ends(edge: &Obj<'_>, verts: &Bound<'_, PyDict>) -> PyResult<Option<[[f64; 3]; 2]>> {
    let mut p = [[0.; 3]; 2];
    for (i, key) in ["v1_id", "v2_id"].iter().enumerate() {
        let Some(v) = verts.get_item(edge.getattr(key)?)? else {
            return Ok(None);
        };
        for (j, k) in ["x", "y", "z"].iter().enumerate() {
            p[i][j] = v.getattr(k)?.extract::<f64>()? * 0.0254;
        }
    }
    Ok(Some(p))
}
fn append_pairs(arr: &Obj<'_>, xf: &Obj<'_>, out: &Obj<'_>, soft: bool) -> PyResult<()> {
    if arr.len()? == 0 {
        return Ok(());
    }
    let world = mapped(arr, xf)?;
    if soft && out.hasattr("add")? {
        out.call_method1("add", (world,))?;
        return Ok(());
    }
    let coords = world.extract::<PyReadonlyArray2<f64>>()?;
    let a = coords.as_array();
    for i in (0..a.nrows()).step_by(2) {
        let p = |i: usize| (a[[i, 0]], a[[i, 1]], a[[i, 2]]);
        out.call_method1("append", ((p(i), p(i + 1)),))?;
    }
    Ok(())
}
fn segments(d: &Obj<'_>, xf: &Obj<'_>, out: &Obj<'_>, soft: bool) -> PyResult<()> {
    let py = d.py();
    let key = if soft { "_soft_arr" } else { "_loose_arr" };
    let a = if let Some(a) = d.getattr_opt(key)? {
        a
    } else {
        let mut coords = vec![];
        let used = PySet::empty(py)?;
        let mut hidden = none(py);
        if !soft {
            let cache = py
                .import("formats.skp_consumption")?
                .getattr("_geometry_cache")?
                .call_method0("get")?;
            hidden = attr(&cache, "hidden_tags")?;
            if let Some(faces) = d.getattr_opt("faces")? {
                for f in faces.call_method0("values")?.try_iter()? {
                    for l in values(&f?, "loops")? {
                        for u in l.try_iter()? {
                            used.add(u?.get_item(0)?)?;
                        }
                    }
                }
            }
        }
        if let Some(edges) = d.getattr_opt("edges")? {
            for (i, (k, e)) in edges.cast::<PyDict>()?.iter().enumerate() {
                if i % 256 == 0 {
                    crate::file_open::skp_checkpoint(py)?;
                }
                if soft {
                    if !(flagged(&e, "soft")? || flagged(&e, "smooth")? || flagged(&e, "hidden")?) {
                        continue;
                    }
                } else if used.contains(k)?
                    || flagged(&e, "hidden")?
                    || (!hidden.is_none() && hidden.contains(attr(&e, "layer")?)?)
                {
                    continue;
                }
                if let Some(points) = ends(&e, d.getattr("vertices")?.cast()?)? {
                    coords.extend(points.into_iter().flatten());
                }
            }
        }
        let bytes = coords.len() * 8;
        let a = array(py, coords);
        cache(d, key, &a, bytes)?;
        a
    };
    append_pairs(&a, xf, out, soft)
}
#[pyfunction]
pub fn skp_soft_segments(d: &Obj<'_>, xf: &Obj<'_>, out: &Obj<'_>) -> PyResult<()> {
    segments(d, xf, out, true)
}
#[pyfunction]
pub fn skp_loose_segments(d: &Obj<'_>, xf: &Obj<'_>, out: &Obj<'_>) -> PyResult<()> {
    segments(d, xf, out, false)
}
#[pyfunction]
pub fn skp_curve_segments(d: &Obj<'_>, xf: &Obj<'_>, out: &Obj<'_>) -> PyResult<()> {
    let py = d.py();
    let mut curves: Vec<Vec<f64>> = vec![];
    let ids = PyDict::new(py);
    if let Some(edges) = d.getattr_opt("edges")? {
        for (_, e) in edges.cast::<PyDict>()? {
            let cid = attr(&e, "curve_id")?;
            if cid.is_none() {
                continue;
            }
            if let Some(p) = ends(&e, d.getattr("vertices")?.cast()?)? {
                let i = match ids.get_item(&cid)? {
                    Some(i) => i.extract()?,
                    None => {
                        let i = curves.len();
                        ids.set_item(cid, i)?;
                        curves.push(vec![]);
                        i
                    }
                };
                curves[i].extend(p.into_iter().flatten());
            }
        }
    }
    for coords in curves {
        let pairs = PyList::empty(py);
        let world = mapped(&array(py, coords), xf)?;
        let a = world.extract::<PyReadonlyArray2<f64>>()?;
        let a = a.as_array();
        for i in (0..a.nrows()).step_by(2) {
            pairs.append((a.row(i).to_vec(), a.row(i + 1).to_vec()))?;
        }
        out.call_method1("append", (pairs,))?;
    }
    Ok(())
}
#[pyfunction]
pub fn skp_instance_identity<'p>(parent: &Obj<'p>, instance: &Obj<'p>) -> PyResult<Obj<'p>> {
    let pid = attr(instance, "persistent_id")?;
    if parent.is_none() || !pid.is_truthy()? {
        return Ok(none(parent.py()));
    }
    parent.add((pid.str()?,))
}
struct Collect<'a, 'p> {
    by_id: &'a Bound<'p, PyDict>,
    attrs: &'a Bound<'p, PyDict>,
    out: &'a Obj<'p>,
    ids: Option<&'a Obj<'p>>,
    uses: Option<&'a Obj<'p>>,
    images: Option<&'a Obj<'p>>,
    edges: Option<&'a Obj<'p>>,
    layers: Option<&'a Obj<'p>>,
    group_layer: &'a Obj<'p>,
    lines: Option<&'a Obj<'p>>,
    curves: Option<&'a Obj<'p>>,
    release: Option<&'a Obj<'p>>,
}
impl<'p> Collect<'_, 'p> {
    fn walk(
        &self,
        d: &Obj<'p>,
        xf: &Obj<'p>,
        depth: usize,
        path: &mut HashSet<usize>,
        paint: &Obj<'p>,
        layer: &Obj<'p>,
        id: &Obj<'p>,
    ) -> PyResult<()> {
        let py = d.py();
        let ptr = d.as_ptr() as usize;
        if depth > 32 || !path.insert(ptr) {
            return Ok(());
        }
        {
            let geom = skp_defn_geom(d)?;
            let rows = geom.get_item(2)?;
            if rows.is_truthy()? {
                let wl = skp_world_points(&geom.get_item(0)?, xf)?;
                let mirrored = xf.call_method0("determinant")?.extract::<f64>()? < 0.;
                for (i, row) in rows.try_iter()?.enumerate() {
                    if i % 256 == 0 {
                        crate::file_open::skp_checkpoint(py)?;
                    }
                    let r = row?;
                    let f = super::skp_face_entry(
                        &r.get_item(0)?,
                        &wl,
                        &geom.get_item(1)?,
                        r.get_item(1)?.extract()?,
                        r.get_item(2)?.extract()?,
                        r.get_item(3)?.extract()?,
                        self.attrs,
                        Some(paint),
                        Some(layer),
                        mirrored,
                    )?;
                    if !f.is_none() {
                        self.out.call_method1("append", (f,))?;
                    }
                }
            }
        }
        if let Some(out) = self.curves {
            skp_curve_segments(d, xf, out)?;
        }
        if let Some(out) = self.edges {
            segments(d, xf, out, true)?;
        }
        if let Some(out) = self.lines {
            segments(d, xf, out, false)?;
        }
        if let Some(release) = self.release {
            release.call1((d,))?;
        }
        for ins in values(d, "instances")? {
            let Some(child) = self.by_id.get_item(attr(&ins, "ref_idx")?)? else {
                continue;
            };
            let placed = xf.mul(super::skp_instance_matrix(
                ins.getattr("matrix")?.extract()?,
            )?)?;
            let ident = skp_instance_identity(id, &ins)?;
            let mat = either(attr(&ins, "material_id")?, paint)?;
            let child_layer = either(attr(&ins, "layer")?, layer)?;
            let tag = either(child_layer.clone(), self.group_layer)?;
            let image = flagged(&child, "is_image")?;
            if image || flagged(&child, "always_faces_camera")? {
                if let Some(images) = self.images {
                    images.call_method1("append", ((&child, placed, mat, tag, ident),))?;
                    continue;
                }
                if image {
                    self.out.call_method1(
                        "extend",
                        (super::skp_image_faces(&child, &placed, self.attrs, &mat)?,),
                    )?;
                    continue;
                }
            }
            let cid = attr(&child, "id")?;
            if let (Some(ids), Some(uses)) = (self.ids, self.uses) {
                if ids.contains(&cid)? {
                    let p = uses.call_method1("setdefault", ((&cid, &mat), PyList::empty(py)))?;
                    p.call_method1(
                        "append",
                        ((placed, tag, either(attr(&ins, "name")?, &none(py))?, ident),),
                    )?;
                    continue;
                }
            }
            if let Some(layers) = self.layers {
                if flagged(&ins, "layer")? {
                    layers.call_method1("append", ((&child, placed, mat, child_layer, ident),))?;
                    continue;
                }
            }
            self.walk(&child, &placed, depth + 1, path, &mat, &child_layer, &ident)?;
        }
        path.remove(&ptr);
        Ok(())
    }
}
#[pyfunction(signature=(defn,xform,by_id,attr_map,out,depth,stack,proto_ids=None,proto_uses=None,inherited=None,image_uses=None,edges_out=None,layer=None,layer_uses=None,group_layer=None,loose_out=None,curves_out=None,identity=None,release=None))]
pub fn skp_collect<'p>(
    defn: &Obj<'p>,
    xform: &Obj<'p>,
    by_id: &Bound<'p, PyDict>,
    attr_map: &Bound<'p, PyDict>,
    out: &Obj<'p>,
    depth: usize,
    stack: &Obj<'p>,
    proto_ids: Option<&Obj<'p>>,
    proto_uses: Option<&Obj<'p>>,
    inherited: Option<&Obj<'p>>,
    image_uses: Option<&Obj<'p>>,
    edges_out: Option<&Obj<'p>>,
    layer: Option<&Obj<'p>>,
    layer_uses: Option<&Obj<'p>>,
    group_layer: Option<&Obj<'p>>,
    loose_out: Option<&Obj<'p>>,
    curves_out: Option<&Obj<'p>>,
    identity: Option<&Obj<'p>>,
    release: Option<&Obj<'p>>,
) -> PyResult<()> {
    let py = defn.py();
    let n = none(py);
    let mut path = stack.extract::<HashSet<usize>>()?;
    let identity = identity
        .cloned()
        .unwrap_or_else(|| PyTuple::empty(py).into_any());
    Collect {
        by_id,
        attrs: attr_map,
        out,
        ids: proto_ids,
        uses: proto_uses,
        images: image_uses,
        edges: edges_out,
        layers: layer_uses,
        group_layer: group_layer.unwrap_or(&n),
        lines: loose_out,
        curves: curves_out,
        release,
    }
    .walk(
        defn,
        xform,
        depth,
        &mut path,
        inherited.unwrap_or(&n),
        layer.unwrap_or(&n),
        &identity,
    )
}
