use super::super::pyfloat::round;
use super::*;
use std::collections::HashSet;

fn point(v: &Obj<'_>) -> PyResult<(f64, f64, f64)> {
    let [x, y, z] = crate::math::xyz(v)?.to_array();
    Ok((x, y, z))
}
fn enabled(v: &Obj<'_>, field: &str) -> PyResult<bool> {
    v.getattr_opt(field)?
        .map(|v| v.is_truthy())
        .transpose()
        .map(|v| v.unwrap_or(false))
}
fn groups<'p>(scene: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let mut out = vec![];
    for g in scene.getattr("groups")?.try_iter()? {
        let g = g?;
        if scene.call_method1("entity_visible", (&g,))?.is_truthy()? && !enabled(&g, "billboard")? {
            out.push(g)
        }
    }
    Ok(out)
}
#[pyfunction(signature=(scene,layers=false))]
pub(super) fn collect_geometry<'p>(scene: &Obj<'p>, layers: bool) -> PyResult<Bound<'p, PyTuple>> {
    let py = scene.py();
    let l = py.import("built_core.layers")?;
    let layer = l.getattr("layer_of")?;
    let default = l.getattr("DEFAULT_LAYER")?;
    let [tris, hard, soft, hard_tags, soft_tags, tri_tags] =
        std::array::from_fn(|_| PyList::empty(py));
    let tick = py
        .import("built_core")?
        .getattr("progress")?
        .getattr("tick")?;
    let eat = |mesh: Obj<'p>, owner: &Obj<'p>, loose: bool| -> PyResult<()> {
        let visible = |e: &Obj<'p>| -> PyResult<bool> {
            if loose {
                scene.call_method1("entity_visible", (e,))?.is_truthy()
            } else {
                Ok(!scene.call_method1("entity_hidden", (e,))?.is_truthy()?)
            }
        };
        let tag = |e: &Obj<'p>| -> PyResult<Obj<'p>> {
            let t = layer.call1((e,))?;
            Ok(if t.eq(&default)? { owner.clone() } else { t })
        };
        for f in mesh.getattr("faces")?.try_iter()? {
            let f = f?;
            tick.call0()?;
            if !visible(&f)? {
                continue;
            }
            let t = tag(&f)?;
            for tri in f.call_method0("triangulate")?.try_iter()? {
                let tri = tri?;
                tris.append((
                    point(&tri.get_item(0)?)?,
                    point(&tri.get_item(1)?)?,
                    point(&tri.get_item(2)?)?,
                ))?;
                if layers {
                    tri_tags.append(&t)?
                }
            }
        }
        for e in mesh.getattr("edges")?.try_iter()? {
            let e = e?;
            if !visible(&e)? || enabled(&e, "hidden")? {
                continue;
            }
            let a = point(&e.getattr("v0")?.getattr("position")?)?;
            let b = point(&e.getattr("v1")?.getattr("position")?)?;
            if enabled(&e, "soft")? {
                let mut normals = vec![];
                for f in e.getattr("faces")?.try_iter()? {
                    let f = f?;
                    if visible(&f)? {
                        normals.push(point(&f.call_method0("normal")?)?);
                        if normals.len() == 2 {
                            break;
                        }
                    }
                }
                if let Some(first) = normals.first() {
                    soft.append((a, b, *first, normals.get(1).copied()))?;
                    if layers {
                        soft_tags.append(tag(&e)?)?
                    }
                }
            } else {
                hard.append((a, b))?;
                if layers {
                    hard_tags.append(tag(&e)?)?
                }
            }
        }
        Ok(())
    };
    eat(scene.getattr("loose_mesh")?, &default, true)?;
    for g in groups(scene)? {
        let mesh = if g.getattr("children")?.is_truthy()? {
            let placements = py
                .import("built_core")?
                .call_method1("group_visible_placements", (scene, &g))?;
            crate::model::group_mesh::world_mesh(&g, Some(&placements))?
        } else if !g.getattr("xform")?.is_none() {
            crate::model::group_mesh::world_mesh(&g, None)?
        } else {
            g.getattr("mesh")?
        };
        eat(mesh, &layer.call1((&g,))?, false)?;
    }
    let mut out = vec![tris, hard, soft];
    if layers {
        out.extend([hard_tags, soft_tags, tri_tags])
    }
    PyTuple::new(py, out)
}
#[pyfunction]
pub(super) fn collect_circles<'p>(scene: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = scene.py();
    let out = PyList::empty(py);
    let curves = py.import("built_core.snap")?.getattr("curve_centers_of_face")?;
    let mut seen = HashSet::new();
    let mut eat = |mesh: Obj<'p>, xf: Obj<'p>, loose: bool| -> PyResult<()> {
        for face in mesh.getattr("faces")?.try_iter()? {
            let f = face?;
            if loose && !scene.call_method1("entity_visible", (&f,))?.is_truthy()? {
                continue;
            }
            let mut n = f.call_method0("normal")?;
            if !xf.is_none() {
                n = xf.call_method1("mapVector", (&n,))?;
                if n.call_method0("length")?.extract::<f64>()? < 1e-9 {
                    continue;
                }
                n.call_method0("normalize")?;
            }
            let normal = crate::math::xyz(&n)?.to_array();
            for row in curves.call1((&f, &xf))?.try_iter()? {
                let row = row?;
                let c = row.get_item(0)?;
                let r = row.get_item(1)?;
                let (x, y, z) = point(&c)?;
                let key = [
                    round(x, 4),
                    round(y, 4),
                    round(z, 4),
                    round(r.extract()?, 4),
                    round(normal[0].abs(), 3),
                    round(normal[1].abs(), 3),
                    round(normal[2].abs(), 3),
                ]
                .map(|x| if x == 0. { 0 } else { x.to_bits() });
                if seen.insert(key) {
                    out.append((c, r, &n))?
                }
            }
        }
        Ok(())
    };
    eat(scene.getattr("loose_mesh")?, py.None().into_bound(py), true)?;
    for g in groups(scene)? {
        for p in py
            .import("built_core")?
            .call_method1("group_visible_placements", (scene, &g))?
            .try_iter()?
        {
            let p = p?;
            eat(p.get_item(0)?.getattr("mesh")?, p.get_item(1)?, false)?
        }
    }
    Ok(out)
}
