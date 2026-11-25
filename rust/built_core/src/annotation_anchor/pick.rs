// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
fn find_edge<'p>(mesh: &Obj<'p>, a: &Obj<'p>, b: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let a = mesh.call_method1("vertex_at", (a,))?;
    let b = mesh.call_method1("vertex_at", (b,))?;
    if a.is_none() || b.is_none() {
        return Ok(None);
    }
    let e = mesh.call_method1("find_edge", (a, b))?;
    Ok((!e.is_none()).then_some(e))
}
#[pyfunction(signature=(scene,start,end,*,include_loose=true,owner=None))]
fn edge_by_endpoints<'p>(
    scene: &Obj<'p>,
    start: &Obj<'p>,
    end: &Obj<'p>,
    include_loose: bool,
    owner: Option<&Obj<'p>>,
) -> PyResult<Hit<'p>> {
    if include_loose {
        if let Some(e) = find_edge(&mesh(scene, None)?, start, end)? {
            return Ok((Some(e), None));
        }
    }
    for row in scene
        .py()
        .import("built_core.dimension")?
        .call_method1("_anchor_candidates", (scene, start, 1e-4))?
        .try_iter()?
    {
        let row = row?;
        let placement = row.get_item(1)?;
        if let Some(o) = owner {
            if !o.eq(&placement)? && !o.eq(row.get_item(0)?)? {
                continue;
            }
        }
        let inv = row.get_item(2)?;
        let (a, b) = if inv.is_none() {
            (start.clone(), end.clone())
        } else {
            (
                inv.call_method1("map", (start,))?,
                inv.call_method1("map", (end,))?,
            )
        };
        if let Some(e) = find_edge(&placement.getattr("mesh")?, &a, &b)? {
            return Ok((Some(e), Some(placement)));
        }
    }
    Ok((None, None))
}
fn flag(s: &Obj, name: &str) -> PyResult<bool> {
    s.getattr_opt(name)?
        .map(|v| v.is_truthy())
        .transpose()
        .map(|v| v.unwrap_or(false))
}
#[pyfunction]
fn picked_edge<'p>(scene: &Obj<'p>, edge: &Obj<'p>) -> PyResult<Hit<'p>> {
    if edge.is_none() {
        return Ok((None, None));
    }
    if flag(edge, "in_group")? {
        return edge_by_endpoints(
            scene,
            &edge.getattr("a")?,
            &edge.getattr("b")?,
            false,
            edge.getattr_opt("group")?.filter(|o| !o.is_none()).as_ref(),
        );
    }
    let owner = scene.getattr_opt("edit_group")?.filter(|o| !o.is_none());
    if kind_of(edge)? == Some("edge") {
        return Ok((Some(edge.clone()), owner));
    }
    let (Some(a), Some(b)) = (edge.getattr_opt("a")?, edge.getattr_opt("b")?) else {
        return Ok((None, None));
    };
    if flag(edge, "guide")? {
        return Ok((None, None));
    }
    let mesh = scene.getattr("mesh")?;
    let a = mesh.call_method1("vertex_at", (a,))?;
    let b = mesh.call_method1("vertex_at", (b,))?;
    if a.is_none() || b.is_none() {
        return Ok((None, None));
    }
    Ok((Some(mesh.call_method1("find_edge", (a, b))?), owner))
}
fn face<'p>(ctx: &Obj<'p>) -> PyResult<Hit<'p>> {
    let view = ctx.getattr("viewport")?;
    let Some(screen) = ctx.getattr_opt("screen")?.filter(|s| !s.is_none()) else {
        return Ok((None, None));
    };
    for name in [
        "pick_face_inference",
        "pick_face_placement",
        "pick_face_any",
    ] {
        let Some(pick) = view.getattr_opt(name)?.filter(|p| !p.is_none()) else {
            continue;
        };
        let result = (|| pick.call1((screen.call_method0("x")?, screen.call_method0("y")?)))();
        let hit = match result {
            Err(e)
                if e.is_instance_of::<PyAttributeError>(ctx.py())
                    || e.is_instance_of::<pyo3::exceptions::PyRuntimeError>(ctx.py()) =>
            {
                return Ok((None, None))
            }
            v => v?,
        };
        let (f, mut owner) = if hit.is_instance_of::<PyTuple>() {
            (
                hit.get_item(0)?,
                Some(hit.get_item(1)?).filter(|v| !v.is_none()),
            )
        } else {
            (hit, None)
        };
        if let Some(o) = owner.take() {
            owner = Some(o.getattr_opt("source_occurrence")?.unwrap_or(o)).filter(|p| !p.is_none());
        }
        if !f.is_none() && owner.is_none() {
            if let Some(g) = view
                .getattr("scene")?
                .getattr_opt("edit_group")?
                .filter(|v| !v.is_none())
            {
                if element(&g.getattr("mesh")?, "face", &f.getattr("key")?)?.is_some() {
                    owner = Some(g);
                }
            }
        }
        return Ok(((!f.is_none()).then_some(f), owner));
    }
    Ok((None, None))
}
#[pyfunction]
fn picked_attachment(ctx: &Obj) -> PyResult<Option<Attachment>> {
    let Some(scene) = ctx
        .getattr("viewport")?
        .getattr_opt("scene")?
        .filter(|v| !v.is_none())
    else {
        return Ok(None);
    };
    let Some(snap) = ctx.getattr_opt("snap")?.filter(|v| !v.is_none()) else {
        return Ok(None);
    };
    let Some(kind) = snap
        .getattr_opt("kind")?
        .and_then(|s| s.extract::<String>().ok())
    else {
        return Ok(None);
    };
    match kind.as_str() {
        "endpoint" => {
            let world = ctx.getattr("world")?;
            if flag(&snap, "context")? {
                let (f, o) = face(ctx)?;
                if let Some(f) = f {
                    if let Some(f) =
                        element(&mesh(&scene, o.as_ref())?, "face", &f.getattr("key")?)?
                    {
                        let m = matrix(&scene, o.as_ref())?;
                        for v in vertices(&f, "face")? {
                            let p = v.getattr("position")?;
                            let p = if let Some(m) = &m {
                                m.call_method1("map", (p,))?
                            } else {
                                p
                            };
                            if xyz(&p)?.distance(xyz(&world)?) < 1e-4 {
                                return Attachment::capture(&scene, &v, o.as_ref(), &world);
                            }
                        }
                    }
                }
            }
            let h = ctx
                .py()
                .import("built_core.dimension")?
                .call_method1("resolve_vertex_anchor", (&scene, &world))?;
            if !h.is_none() {
                let chain = h.getattr("chain")?;
                let owner = if chain.is_truthy()? {
                    Some(chain.get_item(-1)?)
                } else {
                    None
                };
                return Attachment::capture(&scene, &h.getattr("vertex")?, owner.as_ref(), &world);
            }
        }
        "on_edge" | "midpoint" => {
            if let Some(edge) = snap.getattr_opt("edge")? {
                let (e, o) = picked_edge(&scene, &edge)?;
                if let Some(e) = e {
                    return Attachment::capture(&scene, &e, o.as_ref(), &ctx.getattr("world")?);
                }
            }
        }
        "on_face" | "face_center" => {
            let (f, o) = face(ctx)?;
            if let Some(f) = f {
                return Attachment::capture(&scene, &f, o.as_ref(), &ctx.getattr("world")?);
            }
        }
        _ => {}
    }
    Ok(None)
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(edge_by_endpoints, m)?,
        wrap_pyfunction!(picked_edge, m)?,
        wrap_pyfunction!(picked_attachment, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
