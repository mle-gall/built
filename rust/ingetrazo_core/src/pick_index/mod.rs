// SPDX-License-Identifier: GPL-3.0-or-later
//! Document pick index. The host supplies mesh records and cached placement
//! bakes; the arrays keep stable entity and placement offsets across queries.
pub mod blocks;
pub mod edges;
pub mod footprint;
pub mod hits;
pub mod lazy;
pub mod occluders;
pub mod prototypes;
pub mod rays;
pub mod visibility;
use crate::math::xyz;
use numpy::{IntoPyArray, PyArrayMethods};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::time::Instant;

type Obj<'py> = Bound<'py, PyAny>;

fn triple([x, y, z]: [f64; 3]) -> (f64, f64, f64) {
    (x, y, z)
}

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn attr<'py>(obj: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    Ok(obj.getattr_opt(name)?.unwrap_or_else(|| none(obj.py())))
}

fn namespace(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("types")?.getattr("SimpleNamespace")?.call0()
}

fn matrix(py: Python<'_>, values: Vec<f64>) -> PyResult<Obj<'_>> {
    let rows = values.len() / 3;
    Ok(values.into_pyarray(py).reshape([rows, 3])?.into_any())
}

fn outside_meshes<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    match view.getattr_opt("_outer_loose_meshes")? {
        Some(method) => method.call0(),
        None => Ok(PyTuple::empty(view.py()).into_any()),
    }
}

#[derive(Default)]
struct Faces {
    areas: Vec<f64>,
    vis: Vec<bool>,
    sel: Vec<bool>,
    infer: Vec<bool>,
    origins: Vec<f64>,
    edge1: Vec<f64>,
    edge2: Vec<f64>,
    triangle_entities: Vec<i64>,
}

impl Faces {
    fn collect(
        &mut self,
        view: &Obj,
        scene: &Obj,
        faces: &Obj,
        owner: &Obj,
        entities: &Bound<PyList>,
    ) -> PyResult<()> {
        let area_of = view.getattr_opt("_area_of")?;
        let tris_of = view.getattr_opt("_tris_of")?;
        for face in faces.try_iter()? {
            let face = face?;
            let index = entities.len() as i64;
            entities.append((&face, owner))?;
            let area = match &area_of {
                Some(f) => f.call1((&face,))?,
                None => face.call_method0("area")?,
            };
            self.areas.push(area.extract()?);
            self.vis.push(
                scene
                    .call_method1("entity_visible", (&face,))?
                    .is_truthy()?,
            );
            let selectable = scene
                .call_method1("entity_selectable", (&face,))?
                .is_truthy()?;
            self.sel.push(owner.is_none() && selectable);
            self.infer.push(selectable);
            let triangles = match &tris_of {
                Some(f) => f.call1((&face,))?,
                None => face.call_method0("triangulate")?,
            };
            for tri in triangles.try_iter()? {
                let [a, b, c]: [Obj; 3] = tri?.extract()?;
                let a = xyz(&a)?;
                self.origins.extend(a.to_array());
                self.edge1.extend((xyz(&b)? - a).to_array());
                self.edge2.extend((xyz(&c)? - a).to_array());
                self.triangle_entities.push(index);
            }
        }
        Ok(())
    }
}

fn loose_faces<'py>(view: &Obj<'py>, scene: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let memo = attr(view, "_pick_loose_memo")?;
    if !memo.is_none() && memo.get_item(0)?.eq(key)? {
        return memo.get_item(1);
    }
    let entities = PyList::empty(py);
    let mut faces = Faces::default();
    faces.collect(view, scene, &scene.getattr("faces")?, &none(py), &entities)?;
    for mesh in outside_meshes(view)?.try_iter()? {
        let mesh = mesh?;
        let owner = view.call_method1("_loose_mesh_wrapper", (&mesh,))?;
        faces.collect(view, scene, &mesh.getattr("faces")?, &owner, &entities)?;
    }
    let count = faces.triangle_entities.len();
    let triangles = if count == 0 {
        none(py)
    } else {
        (
            matrix(py, faces.origins)?,
            matrix(py, faces.edge1)?,
            matrix(py, faces.edge2)?,
            faces.triangle_entities.into_pyarray(py),
        )
            .into_pyobject(py)?
            .into_any()
    };
    let loose = vec![true; entities.len()].into_pyarray(py);
    let result = (
        entities,
        triangles,
        count,
        faces.areas.into_pyarray(py),
        faces.vis.into_pyarray(py),
        faces.sel.into_pyarray(py),
        faces.infer.into_pyarray(py),
        loose,
    )
        .into_pyobject(py)?
        .into_any();
    view.setattr("_pick_loose_memo", (key, &result))?;
    Ok(result)
}

#[derive(Default)]
struct Edges {
    a: Vec<f64>,
    b: Vec<f64>,
    selectable: Vec<bool>,
    snappable: Vec<bool>,
    soft: Vec<bool>,
}

impl Edges {
    fn collect(
        &mut self,
        scene: &Obj,
        list: &Bound<PyList>,
        edges: &Obj,
        active: bool,
    ) -> PyResult<()> {
        let reveal = attr(scene, "show_hidden_geometry")?.is_truthy()?;
        for e in edges.try_iter()? {
            let e = e?;
            list.append(&e)?;
            self.a.extend(xyz(&e.getattr("a")?)?.to_array());
            self.b.extend(xyz(&e.getattr("b")?)?.to_array());
            self.soft.push(attr(&e, "soft")?.is_truthy()?);
            self.snappable
                .push(scene.call_method1("edge_snappable", (&e,))?.is_truthy()?);
            self.selectable.push(
                active
                    && scene
                        .call_method1("entity_selectable", (&e,))?
                        .is_truthy()?
                    && (reveal || !attr(&e, "hidden")?.is_truthy()?),
            );
        }
        Ok(())
    }

    fn install(view: &Obj, scene: &Obj, idx: &Obj) -> PyResult<()> {
        let py = view.py();
        let list = PyList::empty(py);
        let mut edges = Self::default();
        edges.collect(scene, &list, &scene.getattr("edges")?, true)?;
        for mesh in outside_meshes(view)?.try_iter()? {
            edges.collect(scene, &list, &mesh?.getattr("edges")?, false)?;
        }
        idx.setattr("edges", &list)?;
        if list.is_empty() {
            for name in ["edge_a", "edge_b", "edge_sel", "edge_snap", "edge_soft"] {
                idx.setattr(name, py.None())?;
            }
        } else {
            idx.setattr("edge_a", matrix(py, edges.a)?)?;
            idx.setattr("edge_b", matrix(py, edges.b)?)?;
            idx.setattr("edge_sel", edges.selectable.into_pyarray(py))?;
            idx.setattr("edge_snap", edges.snappable.into_pyarray(py))?;
            idx.setattr("edge_soft", edges.soft.into_pyarray(py))?;
        }
        Ok(())
    }
}

fn placement_block<'py>(
    view: &Obj<'py>,
    scene: &Obj<'py>,
    host: &Obj<'py>,
    hidden: bool,
    generation: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let lazy = match view.getattr_opt("_pick_lazy_table")? {
        Some(f) => f.call0()?,
        None => none(py),
    };
    let context = scene.getattr("edit_group")?.as_ptr() as usize;
    let epoch = match view.getattr_opt("_placements_epoch")? {
        Some(f) => Some(f.call0()?),
        None => None,
    };
    let class = host.getattr("DocumentInteraction")?;
    let ckey = epoch
        .as_ref()
        .map(|e| (e, context, hidden, lazy.as_ptr() as usize).into_pyobject(py))
        .transpose()?;
    let memo = attr(view, "_pick_cands_memo")?;
    let (touchable, fixed, positions) =
        if let Some(key) = ckey.as_ref().filter(|_| !memo.is_none()) {
            if memo.get_item(0)?.eq(key)? {
                Some((memo.get_item(1)?, memo.get_item(2)?, memo.get_item(3)?))
            } else {
                None
            }
        } else {
            None
        }
        .map_or_else(
            || -> PyResult<_> {
                let (t, f, p): (Obj, Obj, Obj) = class
                    .call_method1("_pick_candidates", (view, hidden, &lazy))?
                    .extract()?;
                if let Some(key) = &ckey {
                    view.setattr("_pick_cands_memo", (key, &t, &f, &p))?;
                }
                Ok((t, f, p))
            },
            Ok,
        )?;
    let mut live = Vec::new();
    if lazy.is_none() {
        for row in positions.call_method0("values")?.try_iter()? {
            let row = row?;
            live.push((row.get_item(0)?.extract::<usize>()?, row.get_item(1)?));
        }
    } else {
        let members = attr(view, "_pick_live")?;
        if !members.is_none() {
            for id in members.try_iter()? {
                let record = positions.call_method1("get", (id?,))?;
                if !record.is_none() {
                    live.push((record.get_item(0)?.extract()?, record.get_item(1)?));
                }
            }
            live.sort_unstable_by_key(|r| r.0);
        }
    }
    let count = fixed.len()? + live.len();
    if count == 0 {
        return Ok(none(py));
    }
    let epoch = match epoch {
        Some(e) => e,
        None => host.call_method1("_cache_ver", (view,))?,
    };
    let sig = (&epoch, hidden, count, generation).into_pyobject(py)?;
    let cached = attr(view, "_pick_block")?;
    if !cached.is_none()
        && (cached.get_item(0)?.eq(&sig)? || !attr(view, "_frozen_cache_version")?.is_none())
    {
        return cached.get_item(1);
    }
    let fixed_key = (&epoch, hidden, fixed.len()?, context).into_pyobject(py)?;
    let cached = attr(view, "_pick_fixed_part")?;
    let ids = if lazy.is_none() {
        PyTuple::empty(py).into_any()
    } else {
        lazy.get_item(4)?
    };
    let fixed_part = if !cached.is_none() && cached.get_item(0)?.eq(&fixed_key)? {
        cached.get_item(1)?
    } else {
        let part = blocks::pick_block_part(view, &fixed, &touchable, &ids, &none(py))?;
        view.setattr("_pick_fixed_part", (&fixed_key, &part))?;
        part
    };
    let live = PyList::new(py, live.into_iter().map(|r| r.1))?;
    let lazy_part = blocks::pick_block_part(view, live.as_any(), &touchable, &ids, &fixed_part)?;
    let block = blocks::pick_join_parts(view, &fixed_part, &lazy_part)?;
    view.setattr("_pick_block", (sig, &block))?;
    Ok(block)
}

fn assemble<'py>(
    view: &Obj<'py>,
    scene: &Obj<'py>,
    host: &Obj<'py>,
    loose: &Obj<'py>,
    block: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let np = py.import("numpy")?;
    let idx = namespace(py)?;
    let mut entities = loose.get_item(0)?.call_method0("copy")?;
    let n_loose = entities.len()?;
    let count = if block.is_none() {
        0
    } else {
        block.get_item("entities")?.len()?
    };
    if count > 0 {
        let group_entities = block.get_item("entities")?;
        if !group_entities.is_instance_of::<PyList>() {
            entities = py
                .import("views.array_bake")?
                .getattr("PickEntities")?
                .call1((entities,))?;
        }
        entities.call_method1("extend", (group_entities,))?;
    }
    idx.setattr("entities", &entities)?;
    let loose_index = vec![-1i32; n_loose].into_pyarray(py);
    let rows = if count == 0 {
        loose_index.into_any()
    } else {
        let group_index = block.get_item("place_idx")?;
        np.call_method1("concatenate", ((loose_index, group_index),))?
    };
    idx.setattr("ent_place_idx", rows)?;
    idx.setattr(
        "ent_placements",
        if count == 0 {
            PyList::empty(py).into_any()
        } else {
            block.get_item("placements")?
        },
    )?;
    for (i, (name, field)) in [
        ("ent_area", "areas"),
        ("ent_vis", "vis"),
        ("ent_sel", "sel"),
        ("ent_infer", "infer"),
        ("ent_loose", ""),
    ]
    .iter()
    .enumerate()
    {
        let mut arr = loose.get_item(i + 3)?;
        if count > 0 {
            let extra = if field.is_empty() {
                vec![false; count].into_pyarray(py).into_any()
            } else {
                block.get_item(*field)?
            };
            arr = np.call_method1("concatenate", ((arr, extra),))?;
        }
        idx.setattr(*name, arr)?;
    }
    let triangles = loose.get_item(1)?;
    let n_tris: usize = loose.get_item(2)?.extract()?;
    let spans = PyList::empty(py);
    if n_tris > 0 {
        spans.append((py.None(), 0, n_tris))?;
    }
    let static_tris = count > 0 && !block.get_item("v0")?.is_none();
    for (i, slot) in ["v0", "e1", "e2", "te"].iter().enumerate() {
        let parts = PyList::empty(py);
        if !triangles.is_none() {
            parts.append(triangles.get_item(i)?)?;
        }
        if static_tris {
            let mut column = block.get_item(*slot)?;
            if i == 3 {
                column = host.call_method1("_offset_kept", (view, column, n_loose))?;
            }
            parts.append(column)?;
        }
        let result = if parts.is_empty() {
            none(py)
        } else {
            host.call_method1("_cat_kept", (view, *slot, parts))?
        };
        idx.setattr(["tri_v0", "tri_e1", "tri_e2", "tri_ent"][i], result)?;
    }
    let mut span_len = n_tris;
    let own = PyList::empty(py);
    if count > 0 {
        if static_tris {
            for row in block.get_item("spans")?.try_iter()? {
                let (bb, start, n): (Obj, usize, usize) = row?.extract()?;
                spans.append((bb, n_tris + start, n))?;
                span_len += n;
            }
        }
        for row in block.get_item("own")?.try_iter()? {
            let (bb, a, b, c, te, offset): (Obj, Obj, Obj, Obj, Obj, usize) = row?.extract()?;
            own.append((bb, a, b, c, te, offset + n_loose))?;
        }
    }
    let v0 = idx.getattr("tri_v0")?;
    idx.setattr(
        "tri_spans",
        if !spans.is_empty() && !v0.is_none() && span_len == v0.len()? {
            spans.into_any()
        } else {
            none(py)
        },
    )?;
    idx.setattr("own_spans", own)?;
    for name in [
        "gedge_a",
        "gedge_b",
        "gedge_gi",
        "gedge_layer",
        "gedge_sel",
        "gedge_fixed",
        "gedge_groups",
        "gedge_ctx",
        "gedge_wire",
    ] {
        let value = if !block.is_none() {
            block.get_item(name)?
        } else if matches!(name, "gedge_groups" | "gedge_ctx" | "gedge_wire") {
            PyList::empty(py).into_any()
        } else {
            none(py)
        };
        idx.setattr(name, value)?;
    }
    Edges::install(view, scene, &idx)?;
    Ok(idx)
}

#[pyfunction]
pub fn document_pick_index<'py>(view: &Obj<'py>, near: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let host = py.import("views.document_interaction")?.into_any();
    if view.hasattr("_pick_lazy_table")? {
        let pos = attr(view, "_last_mouse_pos")?;
        let signature = if near.is_none() && !pos.is_none() {
            (pos.call_method0("x")?, pos.call_method0("y")?)
                .into_pyobject(py)?
                .into_any()
        } else if near.is_instance_of::<PyTuple>() && near.get_item(0)?.eq("ray")? {
            none(py)
        } else {
            near.repr()?.into_any()
        };
        let tick = view
            .getattr_opt("_tick")?
            .unwrap_or(0usize.into_pyobject(py)?.into_any());
        let key = (
            tick,
            host.call_method1("_content_ver", (view,))?,
            &signature,
        )
            .into_pyobject(py)?;
        if signature.is_none() || !attr(view, "_pick_last_q")?.eq(&key)? {
            view.call_method1("_pick_materialize", (near,))?;
            view.setattr("_pick_last_q", key)?;
        }
    }
    let scene = view.getattr("scene")?;
    let hidden = match view.getattr_opt("_rest_is_hidden")? {
        Some(f) if f.is_callable() => f.call0()?.is_truthy()?,
        _ => false,
    };
    let mesh = scene.getattr("mesh")?.as_ptr() as usize;
    let group = scene.getattr("edit_group")?.as_ptr() as usize;
    let version = host.call_method1("_content_ver", (view,))?;
    let generation = view
        .getattr_opt("_pick_gen")?
        .unwrap_or(0usize.into_pyobject(py)?.into_any());
    let key = (&version, mesh, hidden, group, &generation).into_pyobject(py)?;
    let loose_key = (version, mesh, hidden, group).into_pyobject(py)?;
    let cache = attr(view, "_pick_index_cache")?;
    if !cache.is_none() {
        let old = cache.get_item(0)?;
        if old.eq(&key)? {
            return cache.get_item(1);
        }
        if cache.len()? > 2
            && old.cast::<PyTuple>()?.get_slice(0, 4).eq(&loose_key)?
            && !attr(view, "_frozen_cache_version")?.is_none()
        {
            let block = cache.get_item(2)?;
            if !block.is_none() && block.is(&attr(view, "_pick_block")?) {
                let idx = cache.get_item(1)?;
                view.setattr("_pick_index_cache", (key, &idx, block))?;
                return Ok(idx);
            }
        }
    }
    let start = Instant::now();
    let loose = loose_faces(view, &scene, &loose_key)?;
    let block = placement_block(view, &scene, &host, hidden, &generation)?;
    let idx = assemble(view, &scene, &host, &loose, &block)?;
    if host.getattr("_PERF")?.is_truthy()? {
        host.call_method1(
            "_plog",
            ("pick_index", start.elapsed().as_secs_f64() * 1000.),
        )?;
    }
    view.setattr("_pick_index_cache", (key, &idx, attr(view, "_pick_block")?))?;
    Ok(idx)
}
