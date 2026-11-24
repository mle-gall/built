// SPDX-License-Identifier: GPL-3.0-or-later
//! History command entry points. The caller enlists the facade in the document
//! transaction. Going through its store also publishes edits to an open definition.
mod erase;
mod vertices;
mod plane;
pub(crate) use plane::{command_rebuild_plane, command_rebuild_planar, RebuiltPlane, RebuiltPlanar};
pub(crate) use vertices::{command_move_vertices, command_transform_vertices, MovedVertices, VertexResult};
pub(crate) use erase::{command_erase_cleanup, command_erase_selection, ErasedCleanup, ErasedSelection};
use crate::math::Vec3;
use pyo3::{prelude::*, types::PyDict};

pub(crate) struct DrawnEdge {
    pub(crate) key: Option<u128>,
    pub(crate) flags_changed: bool,
    pub(crate) dirty: u32,
}

fn store_hypot(mesh: &Bound<'_, PyAny>) -> PyResult<Option<String>> {
    let api = mesh.py().import("core.store_mesh")?;
    if !mesh.get_type().is(&api.getattr("StoreMesh")?)
        || mesh
            .getattr("__dict__")?
            .cast::<PyDict>()?
            .contains("_planned_points")?
    {
        return Ok(None);
    }
    api.call_method0("_hypot")?.extract()
}

fn input_point(mesh: &Bound<'_, PyAny>, point: [f64; 3]) -> PyResult<[f64; 3]> {
    mesh.call_method1(
        "_input_point",
        (Vec3::from(glam::DVec3::from_array(point)),),
    )?
    .extract()
}

pub(crate) fn command_draw_edge(
    mesh: &Bound<'_, PyAny>,
    a: [f64; 3],
    b: [f64; 3],
    soft: Option<bool>,
    curve: Option<i64>,
    layer: Option<&str>,
) -> PyResult<Option<DrawnEdge>> {
    if store_hypot(mesh)?.is_none() {
        return Ok(None);
    }
    let default_layer = mesh.py().import("core.layers")?.getattr("DEFAULT_LAYER")?;
    let a = input_point(mesh, a)?;
    let b = input_point(mesh, b)?;
    let key: Option<u128> = mesh
        .getattr("_store")?
        .call_method1(
            "draw_command_edge",
            (a, b, soft, curve, layer, default_layer),
        )?
        .extract()?;
    let dirty = u32::from(key.is_some());
    let flags_changed = key.is_none() && (soft.is_some() || curve.is_some() || layer.is_some());
    if dirty != 0 {
        mesh.call_method1("_dirty", (dirty,))?;
    } else if flags_changed {
        mesh.call_method0("mark_modified")?;
    }
    Ok(Some(DrawnEdge {
        key,
        flags_changed,
        dirty,
    }))
}

pub(crate) struct AddedFace {
    pub(crate) face: u128,
    pub(crate) dirty: u32,
    pub(crate) mother: Option<u128>,
    pub(crate) remainder: Vec<u128>,
    pub(crate) hole_swaps: bool,
}

pub(super) type FaceResult = (u128, u32, Option<u128>, Vec<u128>, bool);

impl AddedFace {
    pub(super) fn fields(self) -> FaceResult {
        (
            self.face,
            self.dirty,
            self.mother,
            self.remainder,
            self.hole_swaps,
        )
    }
}

pub(crate) fn command_add_face(
    mesh: &Bound<'_, PyAny>,
    outer: Vec<[f64; 3]>,
    holes: Option<Vec<Vec<[f64; 3]>>>,
    attrs: Option<&Bound<'_, PyDict>>,
    interior: bool,
    divide: bool,
) -> PyResult<Option<AddedFace>> {
    let Some(hypot) = store_hypot(mesh)? else {
        return Ok(None);
    };
    let py = mesh.py();
    let holes = holes.unwrap_or_default();
    let mask = super::py::PyMeshStore::maximal_holes(holes.clone(), &hypot)?;
    let mut rings = Vec::with_capacity(1 + holes.len());
    for ring in std::iter::once(outer).chain(
        holes
            .into_iter()
            .zip(mask)
            .filter_map(|(ring, keep)| keep.then_some(ring)),
    ) {
        rings.push(
            ring.into_iter()
                .map(|p| input_point(mesh, p))
                .collect::<PyResult<Vec<_>>>()?,
        );
    }
    let owner = py
        .import("core.store_mesh")?
        .call_method1("_attribute_owner", (mesh,))?;
    let scan = divide
        && py
            .import("core.scan_index")?
            .call_method0("enabled")?
            .is_truthy()?;
    let result: Option<FaceResult> = mesh
        .getattr("_store")?
        .call_method1(
            "_add_command_face",
            (rings, attrs, interior, divide, scan, hypot, owner),
        )?
        .extract()?;
    let Some((face, dirty, mother, remainder, hole_swaps)) = result else {
        return Ok(None);
    };
    mesh.call_method1("_dirty", (dirty,))?;
    Ok(Some(AddedFace {
        face,
        dirty,
        mother,
        remainder,
        hole_swaps,
    }))
}

type Point = [f64; 3];
type Segment = [Point; 2];

fn key_window(points: &[Point]) -> ((f64, f64, f64), (f64, f64, f64)) {
    let mut lo = points[0];
    let mut hi = lo;
    for p in &points[1..] {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let size = lo.into_iter().chain(hi).map(f64::abs).fold(0.0, f64::max);
    let pad = 2e-4 + size * 1e-6;
    (lo.map(|n| n - pad).into(), hi.map(|n| n + pad).into())
}

enum Candidates<'a> {
    Loop(&'a [Point]),
    Segments(&'a [Segment]),
}

fn candidate_keys(mesh: &Bound<'_, PyAny>, selection: Candidates<'_>) -> PyResult<Vec<u128>> {
    let use_scan = match selection {
        Candidates::Loop(points) => !points.is_empty(),
        Candidates::Segments(segments) => (1..=256).contains(&segments.len()),
    };
    if use_scan {
        let scan = mesh
            .py()
            .import("core.scan_index")?
            .call_method1("scan", (mesh,))?;
        if !scan.is_none() {
            let native = scan.getattr("native")?;
            let rows: Vec<u32> = match selection {
                Candidates::Loop(points) => native
                    .call_method1("faces_within", key_window(points))?
                    .extract()?,
                Candidates::Segments(segments) => {
                    let mut found = std::collections::BTreeSet::new();
                    for segment in segments {
                        let hits: Vec<u32> = native
                            .call_method1("faces_meeting", key_window(segment))?
                            .extract()?;
                        found.extend(hits);
                    }
                    found.into_iter().collect()
                }
            };
            return native.call_method1("keys_at", ("face", rows))?.extract();
        }
    }
    mesh.getattr("_store")?
        .call_method1("keys", ("face",))?
        .extract()
}

pub(crate) struct RemovedEdges {
    pub(crate) edges: Vec<u128>,
    pub(crate) faces: Vec<u128>,
    pub(crate) dirty: u32,
}

pub(crate) fn command_delete_edges(
    mesh: &Bound<'_, PyAny>,
    segments: Vec<Segment>,
    cascade_faces: bool,
) -> PyResult<Option<RemovedEdges>> {
    if store_hypot(mesh)?.is_none() {
        return Ok(None);
    }
    let segments = segments
        .into_iter()
        .map(|[a, b]| Ok([input_point(mesh, a)?, input_point(mesh, b)?]))
        .collect::<PyResult<Vec<_>>>()?;
    let faces = if cascade_faces {
        candidate_keys(mesh, Candidates::Segments(&segments))?
    } else {
        Vec::new()
    };
    let result: Option<(Vec<u128>, Vec<u128>)> = mesh
        .getattr("_store")?
        .call_method1("delete_edge_segments", (segments, faces))?
        .extract()?;
    let Some((edges, faces)) = result else {
        return Ok(None);
    };
    let dirty = u32::try_from(edges.len() + faces.len())
        .map_err(|_| pyo3::exceptions::PyOverflowError::new_err("erase exceeds dirty count"))?;
    mesh.call_method1("_dirty", (dirty,))?;
    Ok(Some(RemovedEdges {
        edges,
        faces,
        dirty,
    }))
}

pub(crate) struct RemovedFace {
    pub(crate) face: Option<u128>,
    pub(crate) dirty: u32,
}

pub(crate) fn command_delete_face(
    mesh: &Bound<'_, PyAny>,
    outer: Vec<Point>,
) -> PyResult<Option<RemovedFace>> {
    if store_hypot(mesh)?.is_none() {
        return Ok(None);
    }
    let outer = outer
        .into_iter()
        .map(|p| input_point(mesh, p))
        .collect::<PyResult<Vec<_>>>()?;
    let faces = candidate_keys(mesh, Candidates::Loop(&outer))?;
    let (handled, face): (bool, Option<u128>) = mesh
        .getattr("_store")?
        .call_method1("delete_face_loop", (outer, faces))?
        .extract()?;
    if !handled {
        return Ok(None);
    }
    let dirty = u32::from(face.is_some());
    if dirty != 0 {
        mesh.call_method1("_dirty", (dirty,))?;
    }
    Ok(Some(RemovedFace { face, dirty }))
}
