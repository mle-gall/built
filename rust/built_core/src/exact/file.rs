// SPDX-License-Identifier: GPL-3.0-or-later
//! Format-2 support pools, sparse bindings and ordered native trim uses.
use super::{patch::Topology, Frame, Rows};
use crate::{
    element_ids as eid,
    file_ids::{EXACT_GEOMETRY, EXACT_NAMESPACE},
    store::{keys::Kind, py::PyMeshStore},
};
use numpy::{IntoPyArray, PyReadonlyArray1};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use serde::{Deserialize, Serialize};
use crate::store::source_arcs::ArcBinding;
type Obj<'p> = Bound<'p, PyAny>;
fn bad(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(format!("damaged exact geometry ({e})"))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    units: String,
    surfaces: Vec<PlaneRow>,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    source_arcs: Vec<ArcBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    patches: Vec<Rows>,
    /// Curves and curved surfaces bound to ordinary elements, one JSON row
    /// per geometry block (element ids in file space).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    curved: Vec<String>,
}

/// One block's curved bindings, keyed by persistent element id.
#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct CurvedRows {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split_runs: Vec<([u64;3],bool)>,
    pub edges: Vec<(u64, super::bound::EdgeBind)>,
    pub faces: Vec<(u64, super::bound::Support)>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaneRow {
    r#type: String,
    frame: Frame,
}

pub(crate) fn read_header(data: &Obj<'_>) -> PyResult<Option<PlaneTable>> {
    let req = data.call_method1("get", ("requires",))?;
    let critical = !req.is_none() && req.contains(EXACT_GEOMETRY)?;
    let ext = data.call_method1("get", ("ext",))?;
    let value = ext
        .cast::<PyDict>()
        .ok()
        .map(|d| d.get_item(EXACT_NAMESPACE))
        .transpose()?
        .flatten();
    if !critical && value.is_none() {
        return Ok(None);
    }
    let unsupported = || {
        PyValueError::new_err(format!("This document needs features this build of Built lacks: {EXACT_GEOMETRY} (unsupported or incomplete exact header)"))
    };
    let value = value.filter(|_| critical).ok_or_else(unsupported)?;
    if !data.get_item("built_format")?.eq(2)? {
        return Err(bad("exact geometry requires format 2"));
    }
    let json = data
        .py()
        .import("json")?
        .call_method1("dumps", (value,))?
        .extract::<String>()?;
    let h: Header = serde_json::from_str(&json).map_err(|_| unsupported())?;
    if h.version != 1
        || h.units != "model"
        || (h.surfaces.is_empty() && h.patches.is_empty() && h.source_arcs.is_empty() && h.curved.is_empty())
    {
        return Err(bad("invalid pool header"));
    }
    let mut curved = Vec::with_capacity(h.curved.len());
    for row in &h.curved {
        let rows: CurvedRows = serde_json::from_str(row).map_err(bad)?;
        if !rows.edges.iter().all(|(_, b)| b.curve.validate().is_ok() && b.interval.iter().all(|t| t.is_finite()))
            || !rows.faces.iter().all(|(_, s)| s.validate())
        {
            return Err(bad("invalid curved binding"));
        }
        curved.push(rows);
    }
    for pool in &h.patches {
        if pool.topology.is_some() {
            return Err(bad("topology in support pool"));
        }
        pool.validate_pool().map_err(bad)?;
    }
    let planes = h
        .surfaces
        .into_iter()
        .map(|p| {
            if p.r#type != "plane" {
                return Err(unsupported());
            }
            p.frame.validate().map_err(bad)?;
            Ok(p.frame)
        })
        .collect::<PyResult<Vec<_>>>()?;
    for arc in &h.source_arcs {arc.validate().map_err(bad)?}
    Ok(Some(PlaneTable(planes, h.patches, h.source_arcs, curved)))
}

/// Snapshot these rows together with the element id columns. Keep ids as
/// u64 throughout (never coerce them to floating point in a mixed array).
pub(crate) fn capture(st: &crate::store::Store, arrays: &Bound<'_, PyDict>) -> PyResult<()> {
    if let Some(patch) = &st.state.exact.native {
        let mut pool = patch.rows.clone();
        let topology = pool.topology.take().unwrap();
        arrays.set_item("_native_pool", serde_json::to_string(&pool).map_err(bad)?)?;
        arrays.set_item(
            "coedge_topology",
            serde_json::to_vec(&topology)
                .map_err(bad)?
                .into_pyarray(arrays.py()),
        )?;
        let uses = topology.faces.iter().flat_map(|f| f.loops.iter().flatten());
        for (key, ids) in [
            (
                "coedge_edge_id",
                uses.clone()
                    .filter_map(|u| u.edge)
                    .map(|e| st.pid(Kind::Edge, e as u32))
                    .collect::<Vec<_>>(),
            ),
            (
                "coedge_vertex_id",
                uses.filter_map(|u| u.pole)
                    .map(|v| st.pid(Kind::Vertex, v as u32))
                    .collect(),
            ),
        ] {
            arrays.set_item(key, ids.into_pyarray(arrays.py()))?;
        }
        for (kind, rows, key) in [
            (
                Kind::Edge,
                topology
                    .edges
                    .iter()
                    .map(|e| (e.id, e.curve))
                    .collect::<Vec<_>>(),
                "edge_exact_curve",
            ),
            (
                Kind::Face,
                topology.faces.iter().map(|f| (f.id, f.surface)).collect(),
                "face_surface",
            ),
        ] {
            let (ids, refs): (Vec<_>, Vec<_>) = rows
                .into_iter()
                .map(|(slot, r)| (st.pid(kind, slot as u32), r))
                .unzip();
            arrays.set_item(format!("{key}_id"), ids.into_pyarray(arrays.py()))?;
            arrays.set_item(format!("{key}_ref"), refs.into_pyarray(arrays.py()))?;
        }
    }
    if let Some(arcs)=&st.state.exact.arcs{
        let mut ids=Vec::new();
        let mut values=Vec::new();
        for (&slot,arc) in arcs.iter().filter(|(s,a)|st.arc_matches(**s,a)){
            ids.push(st.pid(Kind::Edge,slot));
            values.push(serde_json::to_string(arc).map_err(bad)?);
        }
        if !ids.is_empty(){arrays.set_item("_arc_ids",ids.into_pyarray(arrays.py()))?;arrays.set_item("_arc_rows",values)?;}
    }
    if st.state.exact.curved.is_some() || !st.state.split_runs.is_empty() {
        let empty = crate::store::exact::Curved::default();
        let c = st.state.exact.curved.as_deref().unwrap_or(&empty);
        let rows = CurvedRows {
            split_runs: st.split_run_ids(),
            edges: c
                .edges
                .iter()
                .filter(|(s, _)| st.alive(Kind::Edge, **s))
                .map(|(s, b)| (st.pid(Kind::Edge, *s), b.clone()))
                .collect(),
            faces: c
                .faces
                .iter()
                .filter(|(s, _)| st.alive(Kind::Face, **s))
                .map(|(s, b)| (st.pid(Kind::Face, *s), b.clone()))
                .collect(),
        };
        if !rows.edges.is_empty() || !rows.faces.is_empty() || !rows.split_runs.is_empty() {
            arrays.set_item("_curved_rows", serde_json::to_string(&rows).map_err(bad)?)?;
        }
    }
    if st.state.exact.planes.is_none() {
        return Ok(());
    }
    let mut slots = Vec::new();
    let mut frames = Vec::new();
    for (slot, f) in st.bound_planes() {
        slots.push(st.pid(Kind::Face, slot));
        frames.extend(f.origin.to_array());
        frames.extend(f.axis.to_array());
        frames.extend(f.reference.to_array());
    }
    if !slots.is_empty() {
        arrays.set_item("_plane_ids", slots.into_pyarray(arrays.py()))?;
        arrays.set_item("_plane_frames", frames.into_pyarray(arrays.py()))?;
    }
    Ok(())
}

pub(crate) fn encode(
    arrays: &Bound<'_, PyDict>,
    sink: &Obj<'_>,
    index: Option<&[u32]>,
) -> PyResult<()> {
    if let Some(raw) = arrays.get_item("_curved_rows")? {
        arrays.del_item("_curved_rows")?;
        // A compatibility facet copy has no format-2 ids: it drops them.
        if let Some(index) = index {
            let mut rows: CurvedRows = serde_json::from_str(&raw.extract::<String>()?).map_err(bad)?;
            let file_id = |id: u64| -> PyResult<u64> {
                let origin = index
                    .iter()
                    .position(|p| *p == eid::origin_index(id))
                    .ok_or_else(|| bad("missing origin"))?;
                Ok(eid::pack(origin as u32, eid::counter(id)))
            };
            for id in rows.split_runs.iter_mut().flat_map(|(ids, _)| ids.iter_mut()) { *id = file_id(*id)?; }
            for r in &mut rows.edges {
                r.0 = file_id(r.0)?;
            }
            for r in &mut rows.faces {
                r.0 = file_id(r.0)?;
            }
            let json = serde_json::to_string(&rows).map_err(bad)?;
            let pool = match sink.getattr_opt("_curved_pool")? {
                Some(p) => p.cast_into::<PyDict>()?,
                None => {
                    let p = PyDict::new(sink.py());
                    sink.setattr("_curved_pool", &p)?;
                    p
                }
            };
            let entry = match pool.get_item(&json)? {
                Some(r) => r.extract::<u32>()?,
                None => {
                    let r = u32::try_from(pool.len()).map_err(bad)?;
                    pool.set_item(&json, r)?;
                    r
                }
            };
            arrays.set_item("face_surface_curved", vec![entry].into_pyarray(sink.py()))?;
        }
    }
    if let Some(raw) = arrays.get_item("_native_pool")? {
        let index = index.ok_or_else(|| bad("native patch requires a format-2 geometry block"))?;
        let pool = match sink.getattr_opt("_patch_pool")? {
            Some(p) => p.cast_into::<PyDict>()?,
            None => {
                let p = PyDict::new(sink.py());
                sink.setattr("_patch_pool", &p)?;
                p
            }
        };
        let entry = match pool.get_item(&raw)? {
            Some(r) => r.extract::<u32>()?,
            None => {
                let r = u32::try_from(pool.len()).map_err(bad)?;
                pool.set_item(&raw, r)?;
                r
            }
        };
        arrays.del_item("_native_pool")?;
        arrays.set_item("face_loop_patch", vec![entry].into_pyarray(sink.py()))?;
        let file_id = |id| {
            let origin = index
                .iter()
                .position(|p| *p == eid::origin_index(id))
                .ok_or_else(|| bad("missing origin"))?;
            Ok(eid::pack(origin as u32, eid::counter(id)))
        };
        for key in ["coedge_edge_id", "coedge_vertex_id"] {
            let uses = arrays
                .as_any()
                .get_item(key)?
                .extract::<PyReadonlyArray1<u64>>()?;
            let ids = uses
                .as_slice()?
                .iter()
                .copied()
                .map(file_id)
                .collect::<PyResult<Vec<_>>>()?;
            arrays.set_item(key, ids.into_pyarray(sink.py()))?;
        }
        for key in ["edge_exact_curve", "face_surface"] {
            let ids = format!("{key}_id");
            let refs = format!("{key}_ref");
            let a = arrays
                .as_any()
                .get_item(&ids)?
                .extract::<PyReadonlyArray1<u64>>()?;
            let r = arrays
                .as_any()
                .get_item(&refs)?
                .extract::<PyReadonlyArray1<u32>>()?;
            let mut rows = Vec::new();
            for (&id, &reference) in a.as_slice()?.iter().zip(r.as_slice()?) {
                rows.push((file_id(id)?, reference));
            }
            rows.sort_unstable();
            let (a, r): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
            arrays.set_item(ids, a.into_pyarray(sink.py()))?;
            arrays.set_item(refs, r.into_pyarray(sink.py()))?;
        }
        return Ok(());
    }
    if let Some(ids)=arrays.get_item("_arc_ids")?{
        let rows=arrays.as_any().get_item("_arc_rows")?.extract::<Vec<String>>()?;
        arrays.del_item("_arc_rows")?;arrays.del_item("_arc_ids")?;
        if let Some(index)=index{
            let pool=match sink.getattr_opt("_arc_pool")?{
                Some(v)=>v.cast_into::<PyDict>()?,None=>{let p=PyDict::new(sink.py());sink.setattr("_arc_pool",&p)?;p}
            };
            let ids=ids.extract::<PyReadonlyArray1<u64>>()?;
            let mut pairs=Vec::new();
            for (&id,v) in ids.as_slice()?.iter().zip(rows){
                let origin=index.iter().position(|&o|o==eid::origin_index(id)).ok_or_else(||bad("missing arc origin"))?;
                let r=pool.call_method1("setdefault",(v,pool.len()))?.extract::<u32>()?;
                pairs.push((eid::pack(origin as u32,eid::counter(id)),r));
            }
            pairs.sort_unstable();let (ids,refs):(Vec<_>,Vec<_>)=pairs.into_iter().unzip();
            arrays.set_item("edge_exact_curve_id",ids.into_pyarray(sink.py()))?;
            arrays.set_item("edge_exact_curve_ref",refs.into_pyarray(sink.py()))?;
        }
    }
    let Some(ids) = arrays.get_item("_plane_ids")? else {
        return Ok(());
    };
    let frames = arrays
        .get_item("_plane_frames")?
        .ok_or_else(|| bad("missing planes"))?;
    arrays.del_item("_plane_ids")?;
    arrays.del_item("_plane_frames")?;
    let Some(index) = index else { return Ok(()) }; // compatibility facet copy
    let ids: PyReadonlyArray1<u64> = ids.extract()?;
    let frames: PyReadonlyArray1<f64> = frames.extract()?;
    let pool = match sink.getattr_opt("_plane_pool")? {
        Some(v) => v.cast_into::<PyDict>()?,
        None => {
            let d = PyDict::new(sink.py());
            sink.setattr("_plane_pool", &d)?;
            d
        }
    };
    let mut rows = Vec::new();
    for (&id, f) in ids
        .as_slice()?
        .iter()
        .zip(frames.as_slice()?.chunks_exact(9))
    {
        let file_origin = index
            .iter()
            .position(|p| *p == eid::origin_index(id))
            .ok_or_else(|| bad("missing face origin"))?;
        let key = pyo3::types::PyTuple::new(sink.py(), f)?;
        let r = match pool.get_item(&key)? {
            Some(v) => v.extract::<u32>()?,
            None => {
                let r = u32::try_from(pool.len()).map_err(bad)?;
                pool.set_item(key, r)?;
                r
            }
        };
        rows.push((eid::pack(file_origin as u32, eid::counter(id)), r));
    }
    rows.sort_unstable();
    let (ids, refs): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    arrays.set_item("face_surface_id", ids.into_pyarray(sink.py()))?;
    arrays.set_item("face_surface_ref", refs.into_pyarray(sink.py()))?;
    Ok(())
}

pub(crate) fn write_header(data: &Bound<'_, PyDict>, sink: &Obj<'_>) -> PyResult<()> {
    let plane_pool = sink.getattr_opt("_plane_pool")?;
    let patch_pool = sink.getattr_opt("_patch_pool")?;
    let arc_pool=sink.getattr_opt("_arc_pool")?;
    let curved_pool = sink.getattr_opt("_curved_pool")?;
    if plane_pool.is_none() && patch_pool.is_none() && arc_pool.is_none() && curved_pool.is_none() {
        return Ok(());
    }
    let mut curved = Vec::new();
    if let Some(pool) = curved_pool {
        for row in pool.try_iter()? {
            curved.push(row?.extract::<String>()?);
        }
    }
    let pool = plane_pool.unwrap_or_else(|| PyDict::new(sink.py()).into_any());
    let mut patches = Vec::new();
    if let Some(pool) = patch_pool {
        for row in pool.try_iter()? {
            patches.push(serde_json::from_str(&row?.extract::<String>()?).map_err(bad)?);
        }
    }

    let mut surfaces = Vec::new();
    for k in pool.try_iter()? {
        let f: [f64; 9] = k?.extract()?;
        surfaces.push(PlaneRow {
            r#type: "plane".into(),
            frame: Frame {
                origin: glam::DVec3::new(f[0], f[1], f[2]),
                axis: glam::DVec3::new(f[3], f[4], f[5]),
                reference: glam::DVec3::new(f[6], f[7], f[8]),
            },
        });
    }
    let mut source_arcs=Vec::new();
    if let Some(pool)=arc_pool{for row in pool.try_iter()? {source_arcs.push(serde_json::from_str(&row?.extract::<String>()?).map_err(bad)?);}}
    let json = serde_json::to_string(&Header {
        source_arcs,
        version: 1,
        units: "model".into(),
        surfaces,
        patches,
        curved,
    })
    .map_err(bad)?;
    let ext = match data.get_item("ext")? {
        Some(ext) => ext.cast::<PyDict>()?.copy()?,
        None => PyDict::new(data.py()),
    };
    ext.set_item(
        EXACT_NAMESPACE,
        data.py().import("json")?.call_method1("loads", (json,))?,
    )?;
    data.set_item("ext", ext)?;
    data.set_item("requires", PyList::new(data.py(), [EXACT_GEOMETRY])?)
}

pub(crate) fn decode(
    arrays: &Obj<'_>,
    frames: &[Frame],
    map: Option<&[u32]>,
) -> PyResult<Vec<(u64, Frame)>> {
    decode_supports(arrays,frames,map,"face_surface_id","face_surface_ref")
}
pub(crate) fn decode_supports<T:Clone>(arrays:&Obj<'_>,pool:&[T],map:Option<&[u32]>,id_name:&str,ref_name:&str)->PyResult<Vec<(u64,T)>>{
    let ids: PyReadonlyArray1<u64> = arrays
        .get_item(id_name)
        .and_then(|x| Ok(x.extract()?))
        .map_err(|_| bad(format!("{id_name} needs a u64 column")))?;
    let refs: PyReadonlyArray1<u32> = arrays
        .get_item(ref_name)
        .and_then(|x| Ok(x.extract()?))
        .map_err(|_| bad(format!("{ref_name} needs a u32 column")))?;
    let ids = ids.as_slice()?;
    let refs = refs.as_slice()?;
    if ids.len() != refs.len() || ids.is_empty() || !ids.windows(2).all(|w| w[0] < w[1]) {
        return Err(bad("unordered or unpaired surface bindings"));
    }
    ids.iter()
        .zip(refs)
        .map(|(&id, &r)| {
            let id = match map {
                Some(m) => eid::remap(id, m).map_err(bad)?,
                None => id,
            };
            Ok((
                id,
                pool
                    .get(r as usize)
                    .ok_or_else(|| bad(if ref_name == "face_surface_ref" { "missing plane pool row" } else { "missing arc pool row" }))?
                    .clone(),
            ))
        })
        .collect()
}
pub(crate) fn bind(mesh: &Obj<'_>, rows: Vec<(u64, Frame)>) -> PyResult<()> {
    let st = mesh.getattr("_store")?;
    let mut st = st.cast::<PyMeshStore>()?.try_borrow_mut()?;
    let index = (rows.len() > 8 && !st.inner.state.ids[Kind::Face as usize].is_implicit())
        .then(|| st.inner.pid_index(Kind::Face));
    for (id, frame) in rows {
        let f = match &index {
            Some(index) => index.get(&id).copied(),
            None => st.inner.slot_of_pid(Kind::Face, id),
        }
        .ok_or_else(|| bad("surface references missing face"))?;
        st.inner.bind_plane(f, frame).map_err(bad)?;
    }
    Ok(())
}

#[pyclass(frozen)]
#[derive(Serialize)]
pub(crate) struct PlaneTable(pub Vec<Frame>, pub Vec<Rows>, pub Vec<ArcBinding>, pub Vec<CurvedRows>);

/// Reconstruct a definition only after its document pool passed validation.
pub(crate) fn native_rows(a: &Obj<'_>, table: &PlaneTable) -> PyResult<Rows> {
    let refs = a
        .get_item("face_loop_patch")?
        .extract::<PyReadonlyArray1<u32>>()?;
    let &[r] = refs.as_slice()? else {
        return Err(bad("invalid native patch reference"));
    };
    let mut rows = table
        .1
        .get(r as usize)
        .ok_or_else(|| bad("missing native patch pool row"))?
        .clone();
    let bytes = a
        .get_item("coedge_topology")?
        .extract::<PyReadonlyArray1<u8>>()?;
    let t: Topology = serde_json::from_slice(bytes.as_slice()?).map_err(bad)?;
    rows.topology = Some(t);
    Ok(rows)
}

#[pyclass(frozen)]
pub(crate) struct BindPlane {
    pub face: u128,
    pub frame: Frame,
}
#[pymethods]
impl BindPlane {
    fn __call__(&self, scene: &Obj<'_>) -> PyResult<()> {
        let mesh = scene.getattr("mesh")?;
        let store = mesh.getattr("_store")?;
        store.call_method1("bind_exact_plane", (self.face, self.frame.array()))?;
        Ok(())
    }
}

pub(crate) fn build_native(
    mesh: &Obj<'_>,
    source: &Obj<'_>,
    arrays: &Obj<'_>,
    table: &PlaneTable,
    map: Option<&[u32]>,
) -> PyResult<()> {
    use crate::store::{
        ids_py::{sources, Owned},
        keys::NONE,
        native::NativePatch,
    };
    use std::{collections::HashSet, sync::Arc};
    let py = mesh.py();
    let patch = NativePatch::new(native_rows(arrays, table)?).map_err(bad)?;
    let object = mesh.getattr("_store")?;
    let mut store = object.cast::<PyMeshStore>()?.try_borrow_mut()?;
    let mut staged = store.inner.clone();
    staged.install_patch(Arc::new(patch)).map_err(bad)?;
    let dense = staged.dense();
    let list = |name: &str| source.get_item(name)?.call_method0("tolist");
    if list("pos")?.extract::<Vec<[f64; 3]>>()? != dense.pos
        || list("edges")?.extract::<Vec<[i64; 2]>>()? != dense.edges
        || list("rings")?.extract::<Vec<i64>>()? != dense.corners
        || list("sizes")?.extract::<Vec<i64>>()? != dense.ring_sizes
        || list("counts")?.extract::<Vec<i64>>()? != dense.ring_counts
    {
        return Err(bad("native topology and geometry block disagree"));
    }
    let ids = source.get_item("ids")?;
    for (kind, column) in Kind::ALL.into_iter().zip(sources(Some(&ids))?) {
        let n = staged.n(kind);
        let values = match column {
            Owned::First(first) => (0..n)
                .map(|i| {
                    first
                        .checked_add(i as u64)
                        .ok_or_else(|| bad("id overflow"))
                })
                .collect::<PyResult<Vec<_>>>()?,
            Owned::Array(values) if values.len() == n => values,
            _ => return Err(bad("missing native element ids")),
        };
        if values.iter().collect::<HashSet<_>>().len() != n {
            return Err(bad("duplicate native element ids"));
        }
        for (slot, id) in values.into_iter().enumerate() {
            staged.set_pid(kind, slot as u32, id);
        }
    }
    let topo = staged.state.exact.native.as_ref().unwrap().topology();
    let uses = topo.faces.iter().flat_map(|f| f.loops.iter().flatten());
    for (key, expected) in [
        (
            "coedge_edge_id",
            uses.clone()
                .filter_map(|u| u.edge)
                .map(|e| staged.pid(Kind::Edge, e as u32))
                .collect::<Vec<_>>(),
        ),
        (
            "coedge_vertex_id",
            uses.filter_map(|u| u.pole)
                .map(|v| staged.pid(Kind::Vertex, v as u32))
                .collect(),
        ),
    ] {
        let ids = arrays.get_item(key)?.extract::<PyReadonlyArray1<u64>>()?;
        let values = ids.as_slice()?;
        if values.len() != expected.len() {
            return Err(bad("coedge id count"));
        }
        for (&id, expected) in values.iter().zip(expected) {
            let actual = match map {
                Some(m) => eid::remap(id, m).map_err(bad)?,
                None => id,
            };
            if actual != expected {
                return Err(bad("coedge refers to a different logical element"));
            }
        }
    }
    for (kind, refs, prefix) in [
        (
            Kind::Edge,
            topo.edges.iter().map(|e| e.curve).collect::<Vec<_>>(),
            "edge_exact_curve",
        ),
        (
            Kind::Face,
            topo.faces.iter().map(|f| f.surface).collect(),
            "face_surface",
        ),
    ] {
        let a = arrays
            .get_item(format!("{prefix}_id"))?
            .extract::<PyReadonlyArray1<u64>>()?;
        let r = arrays
            .get_item(format!("{prefix}_ref"))?
            .extract::<PyReadonlyArray1<u32>>()?;
        let a = a.as_slice()?;
        let r = r.as_slice()?;
        if a.len() != refs.len() || r.len() != refs.len() || !a.windows(2).all(|w| w[0] < w[1]) {
            return Err(bad("invalid native binding column"));
        }
        let slots = staged.pid_index(kind);
        for (&id, &reference) in a.iter().zip(r) {
            let id = match map {
                Some(m) => eid::remap(id, m).map_err(bad)?,
                None => id,
            };
            if slots.get(&id).and_then(|&slot| refs.get(slot as usize)) != Some(&reference) {
                return Err(bad("native support binding disagrees with trim"));
            }
        }
    }
    let meta = source.get_item("meta")?;
    let table = PyList::empty(py);
    let decode = py
        .import("built_core.formats.igz")?
        .getattr("_face_attrs_from_json")?;
    if let Some(attrs) = meta.cast::<PyDict>()?.get_item("face_attrs")? {
        for row in attrs.try_iter()? {
            table.append(decode.call1((row?,))?)?;
        }
    }
    if let Some(names) = meta.cast::<PyDict>()?.get_item("edge_layers")? {
        staged.state.layers = Arc::new(names.extract()?);
    }
    for (key, kind) in [
        ("flags", Kind::Edge),
        ("layers", Kind::Edge),
        ("curves", Kind::Edge),
        ("face_attr", Kind::Face),
    ] {
        let array = source.get_item(key)?;
        if array.is_none() {
            continue;
        }
        let values = array.call_method0("tolist")?.extract::<Vec<i128>>()?;
        if values.len() != staged.n(kind) {
            return Err(bad("wrong attribute column length"));
        }
        for (i, v) in values.into_iter().enumerate() {
            match key {
                "flags" => staged.state.eflags.set(i, u8::try_from(v).map_err(bad)?),
                "layers" if v == -1 => staged.state.elayer.set(i, NONE),
                "layers" if v >= 0 && v < staged.state.layers.len() as i128 => {
                    staged.state.elayer.set(i, v as u32)
                }
                "curves" => staged.state.ecurve.set(i, v),
                "face_attr" if v >= -1 && v < table.len() as i128 => {
                    staged.state.fattr.set(i, (v + 1) as u32)
                }
                _ => return Err(bad("invalid attribute reference")),
            }
        }
    }
    store.inner = staged;
    drop(store);
    crate::kernel::store_views::lifecycle::become_store(mesh, &object, table.as_any(), None)
}

/// The block's curved rows, ids remapped to this process.
pub(crate) fn curved_rows(arrays: &Obj<'_>, table: &PlaneTable, map: Option<&[u32]>) -> PyResult<Option<CurvedRows>> {
    let Ok(col) = arrays.get_item("face_surface_curved") else {
        return Ok(None);
    };
    let refs = col.extract::<PyReadonlyArray1<u32>>().map_err(|_| bad("face_surface_curved needs a u32 column"))?;
    let &[r] = refs.as_slice()? else {
        return Err(bad("invalid curved pool reference"));
    };
    let rows = table.3.get(r as usize).ok_or_else(|| bad("missing curved pool row"))?;
    let remap = |id: u64| -> PyResult<u64> {
        match map {
            Some(m) => eid::remap(id, m).map_err(bad),
            None => Ok(id),
        }
    };
    let mut split_runs = rows.split_runs.clone();
    for id in split_runs.iter_mut().flat_map(|(ids, _)| ids.iter_mut()) { *id = remap(*id)?; }
    Ok(Some(CurvedRows {
        split_runs,
        edges: rows.edges.iter().map(|(id, b)| Ok((remap(*id)?, b.clone()))).collect::<PyResult<_>>()?,
        faces: rows.faces.iter().map(|(id, b)| Ok((remap(*id)?, b.clone()))).collect::<PyResult<_>>()?,
    }))
}

/// Install a block's curved bindings. A binding that no longer holds on
/// its element (a damaged or foreign file) is dropped, never forced.
pub(crate) fn bind_curved(mesh: &Obj<'_>, rows: &CurvedRows) -> PyResult<()> {
    let st = mesh.getattr("_store")?;
    let mut st = st.cast::<PyMeshStore>()?.try_borrow_mut()?;
    st.inner.install_curved(rows);
    Ok(())
}

impl Clone for CurvedRows {
    fn clone(&self) -> Self {
        Self {
            split_runs: self.split_runs.clone(),
            edges: self.edges.clone(),
            faces: self.faces.clone(),
        }
    }
}
