// SPDX-License-Identifier: GPL-3.0-or-later
//! Plane commands reuse the same arrangement installer as the editing facade.
//! A face scope follows only its remaining boundary edges, in ring order.
use super::{store_hypot, Point};
use crate::{
    math::Vec3,
    planar_edits,
    store::{keys::Kind, py::PyMeshStore},
};
use pyo3::{prelude::*, types::PyList};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;

pub(crate) struct RebuiltPlane {
    pub(crate) ran: bool,
    pub(crate) rebuilt: usize,
    pub(crate) dirty: usize,
}

fn serial(mesh: &Obj<'_>) -> PyResult<usize> {
    mesh.getattr("_mut_serial")?.extract()
}

fn vector(py: Python<'_>, p: Point) -> PyResult<Obj<'_>> {
    Ok(Vec3::from(glam::DVec3::from_array(p))
        .into_pyobject(py)?
        .into_any())
}

fn flags<'py>(mesh: &Obj<'py>, keys: Vec<u128>) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let view = py.import("ingetrazo_core.store_mesh")?.getattr("EdgeView")?;
    let read = py.import("ingetrazo_core.mesh")?.getattr("edge_flags")?;
    let entries = PyList::empty(py);
    for key in keys {
        let edge = view.call1((mesh, key))?;
        entries.append((
            edge.getattr("a")?,
            edge.getattr("b")?,
            read.call1((&edge,))?,
        ))?;
    }
    Ok(entries)
}

fn scoped(
    mesh: &Obj<'_>,
    store: &Obj<'_>,
    origin: &Obj<'_>,
    normal: &Obj<'_>,
    keys: &[u128],
) -> PyResult<Option<usize>> {
    let (faces, edges) = {
        let read = PyMeshStore::read(store)?;
        let s = &read.inner;
        let mut faces = Vec::new();
        let mut edges = Vec::new();
        let mut seen = HashSet::new();
        for &key in keys {
            let Some(face) = s.slot(Kind::Face, key) else {
                continue;
            };
            faces.push(key);
            for ring in s.face_rings(face) {
                for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    if let Some(e) = s.find_edge(a, b) {
                        if seen.insert(e) {
                            edges.push(s.key(Kind::Edge, e));
                        }
                    }
                }
            }
        }
        (faces, edges)
    };
    let api = mesh.py().import("ingetrazo_core.store_mesh")?;
    let views = |name: &str, keys: Vec<u128>| -> PyResult<Vec<Obj<'_>>> {
        let ctor = api.getattr(name)?;
        keys.into_iter().map(|k| ctor.call1((mesh, k))).collect()
    };
    planar_edits::rebuild_selected(
        mesh,
        origin,
        normal,
        views("FaceView", faces)?,
        views("EdgeView", edges)?,
    )
}

pub(super) fn rebuild_face_keys(
    mesh: &Obj<'_>,
    store: &Obj<'_>,
    origin: Point,
    normal: Point,
    keys: &[u128],
) -> PyResult<Option<usize>> {
    scoped(
        mesh,
        store,
        &vector(mesh.py(), origin)?,
        &vector(mesh.py(), normal)?,
        keys,
    )
}

fn regions_on_plane(
    mesh: &Obj<'_>,
    store: &Obj<'_>,
    origin: &Obj<'_>,
    normal: &Obj<'_>,
    hypot: &str,
) -> PyResult<Option<usize>> {
    let o = crate::math::xyz(origin)?.to_array();
    let n = crate::math::xyz(normal)?.to_array();
    let h = crate::hypot_of(hypot)?;
    let py = mesh.py();
    let (segs, flagged) = PyMeshStore::read(store)?
        .inner
        .drawing_plane_edges(o, n, h, false);
    if segs.is_empty() {
        return Ok(None);
    }
    let flags = flags(mesh, flagged)?;
    let (_, regions) =
        crate::store::planar_api::planar_rebuild(segs.into_pyobject(py)?.as_any(), origin, normal)?;
    let rings = regions
        .iter()
        .map(|(outer, holes)| {
            std::iter::once(outer)
                .chain(holes)
                .map(|ring| ring.iter().map(Vec3::coords).collect())
                .collect()
        })
        .collect::<Vec<_>>();
    let (keys, owners) = PyMeshStore::read(store)?
        .inner
        .drawing_coverage(o, n, &rings, h);
    Ok(Some(planar_edits::store_regions(
        mesh,
        regions.into_pyobject(py)?.as_any(),
        keys,
        owners.into_pyobject(py)?.as_any(),
        flags.as_any(),
    )?))
}

pub(crate) fn command_rebuild_plane(
    mesh: &Obj<'_>,
    origin: Point,
    normal: Point,
    faces: Option<Vec<u128>>,
) -> PyResult<Option<RebuiltPlane>> {
    let hypot = store_hypot(mesh.py())?;
    let start = serial(mesh)?;
    let store = mesh.getattr("_store")?;
    let count = match faces {
        Some(keys) => rebuild_face_keys(mesh, &store, origin, normal, &keys)?,
        None => regions_on_plane(
            mesh,
            &store,
            &vector(mesh.py(), origin)?,
            &vector(mesh.py(), normal)?,
            &hypot,
        )?,
    };
    Ok(Some(RebuiltPlane {
        ran: count.is_some(),
        rebuilt: count.unwrap_or(0),
        dirty: serial(mesh)? - start,
    }))
}

pub(crate) struct RebuiltPlanar {
    pub(crate) flat: bool,
    pub(crate) rebuilt: usize,
    pub(crate) dirty: usize,
}

fn inherit_flat_paint(
    mesh: &Obj<'_>,
    store: &Obj<'_>,
    snapshot: &Obj<'_>,
    hypot: &str,
) -> PyResult<()> {
    let own = snapshot
        .getattr("own")?
        .cast_into::<pyo3::types::PyDict>()?;
    let table = snapshot.getattr("table")?;
    if own.is_empty() && table.is_empty()? {
        return Ok(());
    }
    let mut painted = std::collections::HashMap::new();
    for (k, v) in own.iter() {
        painted.insert(k.extract::<u128>()?, v.is_truthy()?);
    }
    let mut rows = Vec::new();
    for (i, attrs) in table.try_iter()?.enumerate() {
        if attrs?.is_truthy()? {
            rows.push(i + 1);
        }
    }
    let owners: Vec<(u128, u128, i64)> = store
        .call_method1(
            "drawing_paint_owners",
            (snapshot.getattr("snap")?, painted, rows, hypot),
        )?
        .extract()?;
    let view = mesh.py().import("ingetrazo_core.store_mesh")?.getattr("FaceView")?;
    for (face, source, row) in owners {
        let attrs = crate::kernel::store_views::snapshot_attr(
            snapshot,
            source,
            &row.into_pyobject(mesh.py())?.into_any(),
        )?;
        view.call1((mesh, face))?
            .getattr("attrs")?
            .call_method1("update", (attrs,))?;
    }
    Ok(())
}

pub(crate) fn command_rebuild_planar(mesh: &Obj<'_>) -> PyResult<Option<RebuiltPlanar>> {
    let hypot = store_hypot(mesh.py())?;
    let py = mesh.py();
    let h = crate::hypot_of(&hypot)?;
    let store = mesh.getattr("_store")?;
    let plane = {
        let read = PyMeshStore::read(&store)?;
        let s = &read.inner;
        if s.state.order[1].live(&s.state.slots[1]).is_empty() {
            None
        } else {
            s.drawing_plane(&[], h)
        }
    };
    let Some([mut origin, mut normal]) = plane else {
        return Ok(Some(RebuiltPlanar {
            flat: false,
            rebuilt: 0,
            dirty: 0,
        }));
    };
    let start = serial(mesh)?;
    let snapshot = mesh.call_method0("capture_state")?;
    let topo = py.import("core.topology")?;
    let winding = topo.call_method1("winding_footprint", (mesh,))?;
    let faces = mesh.getattr("faces")?;
    if !faces.is_empty()? {
        let first = faces.get_item(0)?;
        origin = crate::math::xyz(&first.getattr("vertices")?.get_item(0)?)?.to_array();
        normal = crate::math::xyz(&first.call_method0("normal")?)?.to_array();
    }
    let (segments, flagged) = PyMeshStore::read(&store)?
        .inner
        .drawing_plane_edges(origin, normal, h, true);
    let flags = flags(mesh, flagged)?;
    let (edges, regions) = crate::store::planar_api::planar_rebuild(
        segments.into_pyobject(py)?.as_any(),
        &vector(py, origin)?,
        &vector(py, normal)?,
    )?;
    let rebuilt = regions.len();
    planar_edits::install_flat(
        mesh,
        edges.into_pyobject(py)?.as_any(),
        regions.into_pyobject(py)?.as_any(),
        flags.as_any(),
    )?;
    inherit_flat_paint(mesh, &store, &snapshot, &hypot)?;
    if !winding.is_none() {
        topo.call_method1("orient_coplanar_faces", (mesh, winding))?;
    }
    mesh.call_method0("resplit_curves")?;
    Ok(Some(RebuiltPlanar {
        flat: true,
        rebuilt,
        dirty: serial(mesh)? - start,
    }))
}
