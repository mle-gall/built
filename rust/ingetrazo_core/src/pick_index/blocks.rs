// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement blocks keep shared prototypes lazy and fixed arrays reusable.
use super::*;
use pyo3::types::PySlice;

const COLUMNS: &[&str] = &[
    "entities",
    "pidx",
    "plist",
    "area",
    "vis",
    "sel",
    "infer",
    "own",
    "static_key",
    "v0",
    "e1",
    "e2",
    "te",
    "spans",
    "gea",
    "geb",
    "ggi",
    "gel",
    "gsel",
    "ggroups",
    "gctx",
    "gwire",
    "gbase",
];

fn append<'py>(part: &Obj<'py>, name: &str, item: impl IntoPyObject<'py>) -> PyResult<()> {
    part.get_item(name)?.cast::<PyList>()?.append(item)
}

struct Chunk<'py> {
    group: Obj<'py>,
    data: Obj<'py>,
    visible: bool,
    selectable: bool,
    snappable: bool,
}

fn prepare<'py>(
    view: &Obj<'py>,
    host: &Obj<'py>,
    candidates: &Obj<'py>,
    touchable: &Obj<'py>,
    lazy: &Obj<'py>,
) -> PyResult<Vec<Chunk<'py>>> {
    let py = view.py();
    let scene = view.getattr("scene")?;
    let mut chunks = Vec::new();
    let grids = PyList::empty(py);
    let floor: usize = host.getattr("_SPAN_GRID_MIN")?.extract()?;
    let candidates = PyList::new(py, candidates.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
    // The scene's rules for every placement at once (`scene_group_states`).
    let states = crate::model::scene_mesh_states::scene_group_states(py, &scene, candidates.as_any())?;
    for (group, (visible, snappable, selectable)) in candidates.iter().zip(states) {
        if attr(&group, "billboard")?.is_truthy()? {
            continue;
        }
        let selectable = selectable && touchable.contains(group.as_ptr() as usize)?;
        if !visible && !snappable {
            continue;
        }
        let data = view.call_method1("_group_chunk", (&group,))?;
        if !data.get_item("faces")?.is_truthy()? && !data.get_item("edges")?.is_truthy()? {
            continue;
        }
        chunks.push(Chunk {
            group,
            data,
            visible,
            selectable,
            snappable,
        });
    }
    host.call_method1("_pick_warm_take", (view,))?;
    if !host.getattr("_NO_SPAN_GRID")?.is_truthy()? {
        for chunk in &chunks {
            let v0 = chunk.data.get_item("v0")?;
            if !v0.is_none()
                && v0.len()? > floor
                && !lazy.contains(chunk.group.as_ptr() as usize)?
                && !chunk.data.contains("_span_grid")?
            {
                grids.append(&chunk.data)?;
            }
        }
        if grids.len() > 1 {
            let workers = py
                .import("os")?
                .call_method0("cpu_count")?
                .extract::<Option<usize>>()?
                .unwrap_or(2)
                .min(8);
            let pool = py
                .import("concurrent.futures")?
                .getattr("ThreadPoolExecutor")?
                .call1((workers,))?;
            let jobs = pool.call_method1("map", (host.getattr("_split_chunk_spans")?, grids));
            let result = jobs.and_then(|jobs| {
                for job in jobs.try_iter()? {
                    job?;
                }
                Ok(())
            });
            pool.call_method0("shutdown")?;
            result?;
        }
    }
    Ok(chunks)
}

fn pairs<'py>(chunk: &Obj<'py>, owner: &Obj<'py>, faces: &Obj<'py>) -> PyResult<Obj<'py>> {
    let memo = chunk.call_method1("get", ("_pick_ents",))?;
    if !memo.is_none() && memo.get_item(0)?.is(owner) {
        return memo.get_item(1);
    }
    let bake = chunk.py().import("views.array_bake")?;
    let pairs = if faces.is_instance(&bake.getattr("DeferredFaces")?)? {
        bake.getattr("LazyPairs")?.call1((faces, owner))?
    } else {
        let rows = PyList::empty(chunk.py());
        for face in faces.try_iter()? {
            rows.append((face?, owner))?;
        }
        rows.into_any()
    };
    chunk.set_item("_pick_ents", (owner, &pairs))?;
    Ok(pairs)
}

fn edges(
    view: &Obj,
    host: &Obj,
    np: &Obj,
    part: &Obj,
    chunk: &Chunk,
    owner: &Obj,
    group_index: usize,
    wire: bool,
) -> PyResult<()> {
    let py = view.py();
    let data = &chunk.data;
    let raw = data.get_item("edges")?;
    let memo = data.call_method1("get", ("_pick_ge",))?;
    let ge = if !memo.is_none() && memo.get_item(0)?.is(&raw) {
        memo.get_item(1)?
    } else {
        let ge = np
            .call_method1("frombuffer", (&raw, "float32"))?
            .call_method1("reshape", (-1, 2, 3))?
            .call_method1("astype", ("float64",))?;
        data.set_item("_pick_ge", (raw, &ge))?;
        ge
    };
    let n = ge.len()?;
    let all = PySlice::full(py);
    append(part, "gbase", &ge)?;
    append(part, "gea", ge.get_item((&all, 0))?)?;
    append(part, "geb", ge.get_item((&all, 1))?)?;
    append(part, "ggi", vec![group_index as i64; n].into_pyarray(py))?;
    let layers = np.call_method1("empty", (n, "object"))?;
    let own = data.call_method1("get", ("edge_layer",))?;
    if !own.is_none() && own.len()? == n {
        layers.set_item(&all, own)?;
    }
    append(part, "gel", layers)?;
    append(part, "gsel", vec![chunk.selectable; n].into_pyarray(py))?;
    append(part, "ggroups", owner)?;
    append(
        part,
        "gctx",
        host.call_method1("_snap_context_of", (view, &chunk.group))?,
    )?;
    append(part, "gwire", wire)
}

pub(super) fn pick_block_part<'py>(
    view: &Obj<'py>,
    candidates: &Obj<'py>,
    touchable: &Obj<'py>,
    lazy: &Obj<'py>,
    base: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let host = py.import("views.document_interaction")?.into_any();
    let np = py.import("numpy")?.into_any();
    let part = PyDict::new(py).into_any();
    for name in COLUMNS {
        part.set_item(*name, PyList::empty(py))?;
    }
    let initial = |field: &str| -> PyResult<usize> {
        if base.is_none() {
            Ok(0)
        } else {
            base.get_item(field)?.extract()
        }
    };
    let mut faces_count = initial("n_ent")?;
    let mut placement = initial("n_plist")?;
    let mut tri_offset = initial("n_tri")?;
    let mut group_index = initial("n_ggroups")?;
    for chunk in prepare(view, &host, candidates, touchable, lazy)? {
        let g = &chunk.group;
        let data = &chunk.data;
        let faces = data.get_item("faces")?;
        let n = faces.len()?;
        let owner = view.call_method1("_owner_of", (g,))?;
        append(&part, "entities", pairs(data, &owner, &faces)?)?;
        append(&part, "pidx", vec![placement as i32; n].into_pyarray(py))?;
        append(&part, "plist", g)?;
        append(&part, "area", data.get_item("areas")?)?;
        for (name, value) in [
            ("vis", chunk.visible),
            ("sel", chunk.selectable),
            ("infer", chunk.snappable),
        ] {
            append(&part, name, vec![value; n].into_pyarray(py))?;
        }
        if !data.get_item("v0")?.is_none() {
            let bb = data.call_method1("get", ("bbox",))?;
            if lazy.contains(g.as_ptr() as usize)? {
                append(
                    &part,
                    "own",
                    (
                        bb,
                        data.get_item("v0")?,
                        data.get_item("e1")?,
                        data.get_item("e2")?,
                        data.get_item("tri_ent")?,
                        faces_count,
                    ),
                )?;
            } else {
                append(
                    &part,
                    "static_key",
                    (
                        data.call_method1("get", ("uid", data.as_ptr() as usize))?,
                        data.call_method1("get", ("rev",))?,
                        faces_count,
                    ),
                )?;
                let (v0, e1, e2, te, cells): (Obj, Obj, Obj, Obj, Obj) = host
                    .call_method1("_split_chunk_spans", (data,))?
                    .extract()?;
                let rows = v0.len()?;
                for (k, v) in [
                    ("v0", v0),
                    ("e1", e1),
                    ("e2", e2),
                    ("te", te.add(faces_count)?),
                ] {
                    append(&part, k, v)?;
                }
                if cells.is_none() {
                    append(&part, "spans", (bb, tri_offset, rows))?;
                } else {
                    for cell in cells.try_iter()? {
                        let (bb, start, count): (Obj, usize, usize) = cell?.extract()?;
                        append(&part, "spans", (bb, tri_offset + start, count))?;
                    }
                }
                tri_offset += rows;
            }
        }
        if chunk.snappable && data.get_item("edges")?.is_truthy()? {
            edges(view, &host, &np, &part, &chunk, &owner, group_index, n == 0)?;
            group_index += 1;
        }
        faces_count += n;
        placement += 1;
    }
    for (name, n) in [
        ("n_ent", faces_count),
        ("n_plist", placement),
        ("n_tri", tri_offset),
        ("n_ggroups", group_index),
    ] {
        part.set_item(name, n)?;
    }
    if base.is_none() {
        for name in [
            "pidx", "area", "vis", "sel", "infer", "gea", "geb", "ggi", "gel", "gsel",
        ] {
            let items = part.get_item(name)?;
            if items.len()? > 0 {
                part.set_item(name, vec![np.call_method1("concatenate", (items,))?])?;
            }
        }
        let entities = part.get_item("entities")?;
        if let Some(flat) = flatten_lists(&entities, usize::MAX)? {
            part.set_item("entities", vec![flat])?;
        }
    }
    Ok(part)
}

fn flatten_lists<'py>(parts: &Obj<'py>, limit: usize) -> PyResult<Option<Obj<'py>>> {
    let mut n = 0;
    for item in parts.try_iter()? {
        let item = item?;
        if !item.is_instance_of::<PyList>() {
            return Ok(None);
        }
        n += item.len()?;
        if n >= limit {
            return Ok(None);
        }
    }
    let flat = PyList::empty(parts.py());
    for item in parts.try_iter()? {
        flat.call_method1("extend", (item?,))?;
    }
    Ok(Some(flat.into_any()))
}

pub(super) fn pick_join_parts<'py>(
    view: &Obj<'py>,
    fixed: &Obj<'py>,
    lazy: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let np = py.import("numpy")?;
    let both = |key: &str| fixed.get_item(key)?.add(lazy.get_item(key)?);
    let out = PyDict::new(py);
    let parts = both("entities")?;
    let entities = if lazy.get_item("entities")?.len()? == 0
        && parts.len()? == 1
        && parts.get_item(0)?.is_instance_of::<PyList>()
    {
        parts.get_item(0)?
    } else if let Some(list) = flatten_lists(&parts, 200_000)? {
        list
    } else {
        let entities = py
            .import("views.array_bake")?
            .getattr("PickEntities")?
            .call0()?;
        for part in parts.try_iter()? {
            entities.call_method1("extend", (part?,))?;
        }
        entities
    };
    out.set_item("entities", entities)?;
    for (name, key) in [
        ("placements", "plist"),
        ("own", "own"),
        ("gedge_groups", "ggroups"),
        ("gedge_ctx", "gctx"),
        ("gedge_wire", "gwire"),
        ("spans", "spans"),
    ] {
        out.set_item(name, both(key)?)?;
    }
    for (key, name, dtype) in [
        ("pidx", "place_idx", "int32"),
        ("area", "areas", "float64"),
        ("vis", "vis", "bool"),
        ("sel", "sel", "bool"),
        ("infer", "infer", "bool"),
        ("gea", "gedge_a", ""),
        ("geb", "gedge_b", ""),
        ("ggi", "gedge_gi", ""),
        ("gel", "gedge_layer", ""),
        ("gsel", "gedge_sel", ""),
    ] {
        let items = both(key)?;
        let column = if items.len()? > 0 {
            np.call_method1("concatenate", (items,))?
        } else if dtype.is_empty() {
            none(py)
        } else {
            np.call_method1("empty", (0, dtype))?
        };
        out.set_item(name, column)?;
    }
    let key = both("static_key")?.call_method0("__iter__")?;
    let key = py.import("builtins")?.getattr("tuple")?.call1((key,))?;
    let memo = attr(view, "_pick_static_memo")?;
    let triangles = if !memo.is_none() && memo.get_item(0)?.eq(&key)? {
        memo.get_item(1)?
    } else {
        let cols = PyDict::new(py);
        for name in ["v0", "e1", "e2", "te"] {
            let items = both(name)?;
            cols.set_item(
                name,
                if items.len()? > 0 {
                    np.call_method1("concatenate", (items,))?
                } else {
                    none(py)
                },
            )?;
        }
        view.setattr("_pick_static_memo", (key, &cols))?;
        cols.into_any()
    };
    out.call_method1("update", (triangles,))?;
    let mut n = 0;
    for array in fixed.get_item("gea")?.try_iter()? {
        n += array?.len()?;
    }
    out.set_item("gedge_fixed", (fixed, n, lazy.get_item("gbase")?))?;
    Ok(out.into_any())
}
