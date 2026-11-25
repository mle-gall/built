// SPDX-License-Identifier: GPL-3.0-or-later
//! Erase returns keys for the command's selection and plane-rebuild steps.
//! The winding snapshot spans both phases, including the caller's rebuilds.
use super::{input_point, store_hypot, Point, Segment};
use pyo3::{prelude::*, types::PyDict};

pub(crate) struct ErasedSelection {
    pub(crate) edges: Vec<u128>,
    pub(crate) faces: Vec<u128>,
    pub(crate) slit_planes: Vec<(Point, Point)>,
    pub(crate) across: Vec<u128>,
    pub(crate) dirty: u32,
    pub(crate) footprint: Py<PyAny>,
}
pub(crate) struct ErasedCleanup {
    pub(crate) seams: u32,
    pub(crate) healed_faces: Vec<u128>,
    pub(crate) orphan_vertices: u32,
    pub(crate) dirty: u32,
}

type Merges = Vec<(u128, Vec<u128>, bool)>;
type StoreErase = (
    u32,
    Vec<u128>,
    Vec<u128>,
    Merges,
    Vec<(Point, Point)>,
    Vec<u128>,
);

fn face_attrs<'py>(
    store: &Bound<'py, PyAny>,
    owner: &Bound<'py, PyAny>,
    own: &Bound<'py, PyDict>,
    key: u128,
) -> PyResult<Bound<'py, PyAny>> {
    if let Some(value) = own.get_item(key)? {
        return Ok(value);
    }
    let py = store.py();
    let (row, _): (i64, u8) = store.call_method1("face_info", (key,))?.extract()?;
    let value = if row < 0 {
        PyDict::new(py).into_any()
    } else {
        let entry = owner.get_item("_attr_table")?.get_item(row)?;
        py.import("core.store_mesh")?
            .call_method1("_copy_attrs", (entry,))?
    };
    own.set_item(key, &value)?;
    Ok(value)
}

fn merge_attrs(mesh: &Bound<'_, PyAny>, store: &Bound<'_, PyAny>, merges: Merges) -> PyResult<()> {
    if merges.is_empty() {
        return Ok(());
    }
    let py = mesh.py();
    let owner = py
        .import("core.store_mesh")?
        .call_method1("_attribute_owner", (mesh,))?;
    let own = owner.get_item("_attr_own")?.cast_into::<PyDict>()?;
    for (target, sources, stacked) in merges {
        if stacked && face_attrs(store, &owner, &own, target)?.is_truthy()? {
            continue;
        }
        let mut best = PyDict::new(py).into_any();
        let mut largest = -1.;
        for source in sources.into_iter().skip(usize::from(stacked)) {
            let attrs = face_attrs(store, &owner, &own, source)?;
            if attrs.is_truthy()? {
                let rings = store.call_method1("face_positions", (source,))?;
                let points: Vec<Point> = rings.get_item(0)?.extract()?;
                let points: Vec<_> = points.into_iter().map(glam::DVec3::from_array).collect();
                let area = 0.5 * crate::polygon::raw_newell(&points).length();
                if area > largest {
                    largest = area;
                    best = attrs;
                }
            }
        }
        own.set_item(target, py.get_type::<PyDict>().call1((best,))?)?;
    }
    Ok(())
}

pub(crate) fn command_erase_selection(
    mesh: &Bound<'_, PyAny>,
    loops: Vec<Vec<Point>>,
    segments: Vec<Segment>,
) -> PyResult<Option<ErasedSelection>> {
    let hypot = store_hypot(mesh.py())?;
    let py = mesh.py();
    let footprint = py
        .import("core.topology")?
        .call_method1("winding_footprint", (mesh,))?;
    let loops = loops
        .into_iter()
        .map(|ring| {
            ring.into_iter()
                .map(|p| input_point(mesh, p))
                .collect::<PyResult<Vec<_>>>()
        })
        .collect::<PyResult<Vec<_>>>()?;
    let segments = segments
        .into_iter()
        .map(|[a, b]| Ok([input_point(mesh, a)?, input_point(mesh, b)?]))
        .collect::<PyResult<Vec<_>>>()?;
    let store = mesh.getattr("_store")?;
    let result: Option<StoreErase> = store
        .call_method1("erase_selection", (loops, segments, hypot))?
        .extract()?;
    let Some((dirty, edges, faces, merges, slit_planes, across)) = result else {
        return Ok(None);
    };
    if dirty != 0 {
        mesh.call_method1("_dirty", (dirty,))?;
    }
    merge_attrs(mesh, &store, merges)?;
    Ok(Some(ErasedSelection {
        edges,
        faces,
        slit_planes,
        across,
        dirty,
        footprint: footprint.unbind(),
    }))
}

fn heal_keys(mesh: &Bound<'_, PyAny>, footprint: &Bound<'_, PyAny>) -> PyResult<Vec<u128>> {
    let py = mesh.py();
    let kw = PyDict::new(py);
    kw.set_item("footprint", footprint)?;
    let healed = py
        .import("core.topology")?
        .getattr("heal_overlapping_faces")?
        .call((mesh,), Some(&kw))?;
    healed
        .try_iter()?
        .map(|face| face?.getattr("key")?.extract())
        .collect()
}

pub(crate) struct Healed {
    pub(crate) faces: Vec<u128>,
    pub(crate) dirty: usize,
}

pub(crate) fn command_heal_overlaps(mesh: &Bound<'_, PyAny>) -> PyResult<Option<Healed>> {
    let before: usize = mesh.getattr("_mut_serial")?.extract()?;
    let footprint = mesh
        .py()
        .import("core.topology")?
        .call_method1("winding_footprint", (mesh,))?;
    let faces = heal_keys(mesh, &footprint)?;
    let after: usize = mesh.getattr("_mut_serial")?.extract()?;
    Ok(Some(Healed {
        faces,
        dirty: after - before,
    }))
}

pub(crate) fn command_erase_cleanup(
    mesh: &Bound<'_, PyAny>,
    footprint: &Bound<'_, PyAny>,
) -> PyResult<Option<ErasedCleanup>> {
    let serial: u64 = mesh.getattr("_mut_serial")?.extract()?;
    let seams: u32 = mesh
        .getattr("_store")?
        .call_method0("erase_soft_seams")?
        .extract()?;
    if seams != 0 {
        mesh.call_method1("_dirty", (seams,))?;
    }
    let healed_faces = heal_keys(mesh, footprint)?;
    let orphan_vertices = mesh.call_method0("prune_orphan_vertices")?.extract()?;
    let after: u64 = mesh.getattr("_mut_serial")?.extract()?;
    let dirty = u32::try_from(after - serial)
        .map_err(|_| pyo3::exceptions::PyOverflowError::new_err("erase exceeds dirty count"))?;
    Ok(Some(ErasedCleanup {
        seams,
        healed_faces,
        orphan_vertices,
        dirty,
    }))
}
