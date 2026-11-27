// SPDX-License-Identifier: GPL-3.0-or-later
//! Format-2 plane bindings with implicit straight trims. Other exact trim
//! encodings must be implemented before their blocks can be opened.
use super::Frame;
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
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaneRow {
    r#type: String,
    frame: Frame,
}

pub(crate) fn read_header(data: &Obj<'_>) -> PyResult<Option<Vec<Frame>>> {
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
    if h.version != 1 || h.units != "model" || h.surfaces.is_empty() {
        return Err(bad("invalid pool header"));
    }
    h.surfaces
        .into_iter()
        .map(|p| {
            if p.r#type != "plane" {
                return Err(unsupported());
            }
            p.frame.validate().map_err(bad)?;
            Ok(p.frame)
        })
        .collect::<PyResult<Vec<_>>>()
        .map(Some)
}

/// Snapshot these rows together with the element id columns. Keep ids as
/// u64 throughout (never coerce them to floating point in a mixed array).
pub(crate) fn capture(st: &crate::store::Store, arrays: &Bound<'_, PyDict>) -> PyResult<()> {
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
    let Some(pool) = sink.getattr_opt("_plane_pool")? else {
        return Ok(());
    };
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
    let json = serde_json::to_string(&Header {
        version: 1,
        units: "model".into(),
        surfaces,
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
    let ids: PyReadonlyArray1<u64> = arrays
        .get_item("face_surface_id")
        .and_then(|x| Ok(x.extract()?))
        .map_err(|_| bad("face_surface_id needs a u64 column"))?;
    let refs: PyReadonlyArray1<u32> = arrays
        .get_item("face_surface_ref")
        .and_then(|x| Ok(x.extract()?))
        .map_err(|_| bad("face_surface_ref needs a u32 column"))?;
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
                frames
                    .get(r as usize)
                    .ok_or_else(|| bad("missing plane pool row"))?
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
pub(crate) struct PlaneTable(pub Vec<Frame>);

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
