// SPDX-License-Identifier: GPL-3.0-or-later
//! Edit the UV columns of the selected face's resident material records.
//! Corner order and texture IDs come from the same pack the renderer uploaded.
use glam::{DMat4, DVec3};
use numpy::{IntoPyArray, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};

struct Span {
    code: usize,
    start: usize,
    points: Vec<DVec3>,
    origin: DVec3,
    records: Vec<u8>,
    full: Option<Vec<u8>>,
}
#[pyclass(module = "ingetrazo_core")]
pub struct TextureToolRows {
    spans: Vec<Span>,
    front: bool,
    back: bool,
}

#[pymethods]
impl TextureToolRows {
    #[staticmethod]
    fn prepare(view: &Bound<'_, PyAny>, face: &Bound<'_, PyAny>, side: &str) -> PyResult<Self> {
        let py = view.py();
        let packed = view.getattr("packed")?;
        let mesh = view.getattr("scene")?.getattr("mesh")?;
        let store = mesh.getattr_opt("_store")?.filter(|s| !s.is_none());
        let origin = store
            .as_ref()
            .map(|s| s.getattr("frame_origin")?.extract::<[f64; 3]>())
            .transpose()?
            .unwrap_or([0.; 3]);
        let placement = store
            .as_ref()
            .map(|s| s.getattr("frame_matrix")?.extract::<Option<[f64; 16]>>())
            .transpose()?
            .flatten()
            .map_or(DMat4::IDENTITY, |m| DMat4::from_cols_array(&m));
        let attrs = face.getattr("attrs")?;
        let front = side == "front";
        let back = !front
            || (attrs
                .call_method1("get", ("back",))?
                .cast::<PyDict>()
                .is_err()
                && !py
                    .import("core.materials")?
                    .call_method1("back_is_default", (&attrs,))?
                    .is_truthy()?);
        let ranges: Vec<(usize, usize, usize)> = py
            .import("views.wgpu_parity")?
            .call_method1("face_ranges", (view, vec![face.clone()]))?
            .extract()?;
        let sparse = view
            .getattr("renderer")?
            .hasattr("set_tool_material_range")?;
        let mut spans = Vec::new();
        let points = packed
            .getattr("vertices")?
            .get_item("pos")?
            .extract::<PyReadonlyArray2<f32>>()?;
        let points = points.as_array();
        let cmds = packed.getattr("commands")?;
        let indices = packed
            .getattr("indices")?
            .extract::<PyReadonlyArray1<u32>>()?;
        let indices = indices.as_array();
        let headers = packed
            .getattr("materials")?
            .extract::<PyReadonlyArray1<u8>>()?;
        let table = headers.as_slice()?;
        for (code, triangle, count) in ranges {
            let start = triangle * 3;
            let size = count * 3;
            let cmd = cmds.get_item(code)?;
            let base = cmd.get_item("base")?.extract::<i64>()?;
            let first = cmd.get_item("first")?.extract::<usize>()?;
            let local_origin = DVec3::from(
                packed
                    .getattr("prototype_origins")?
                    .get_item(code)?
                    .extract::<[f64; 3]>()?,
            );
            let zero = placement.transform_point3(local_origin) - DVec3::from(origin);
            let overrides = packed.getattr("material_updates")?;
            let replacement = overrides
                .cast::<PyDict>()?
                .get_item(code)?
                .map(|v| -> PyResult<_> { Ok(v.get_item(1)?.extract::<PyReadonlyArray1<u8>>()?) })
                .transpose()?;
            let all = if let Some(r) = &replacement {
                r.as_slice()?
            } else {
                let h = table
                    .get(code * 16..code * 16 + 16)
                    .ok_or_else(|| PyValueError::new_err("missing material header"))?;
                let off = u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize * 16;
                let len = u32::from_le_bytes(h[8..12].try_into().unwrap()) as usize * 32;
                table
                    .get(off..off + len)
                    .ok_or_else(|| PyValueError::new_err("invalid material header"))?
            };
            let Some(records) = all.get(start * 32..(start + size) * 32) else {
                continue;
            };
            let mut corners = Vec::with_capacity(size);
            for i in start..start + size {
                let j = *indices
                    .get(first + i)
                    .ok_or_else(|| PyValueError::new_err("missing texture corner"))?
                    as usize;
                // Material rows are per emitted corner, as in scene_bake.
                if j != i {
                    return Err(PyValueError::new_err(
                        "editable corners out of material order",
                    ));
                }
                let at = usize::try_from(base + j as i64)
                    .map_err(|_| PyValueError::new_err("invalid texture vertex"))?;
                if at >= points.nrows() {
                    return Err(PyValueError::new_err("invalid texture vertex"));
                }
                let p = DVec3::new(
                    points[[at, 0]] as f64,
                    points[[at, 1]] as f64,
                    points[[at, 2]] as f64,
                );
                corners.push(placement.transform_vector3(p));
            }
            spans.push(Span {
                code,
                start,
                points: corners,
                origin: zero,
                records: records.to_vec(),
                full: (!sparse).then(|| all.to_vec()),
            });
        }
        Ok(Self { spans, front, back })
    }

    fn upload(&self, renderer: &Bound<'_, PyAny>, uvw: Option<[f64; 8]>) -> PyResult<()> {
        for span in &self.spans {
            let mut rows = span.records.clone();
            if let Some(m) = uvw {
                for (k, axis) in m.chunks_exact(4).enumerate() {
                    let grad = DVec3::new(axis[0], axis[1], axis[2]);
                    let phase = grad.dot(span.origin) + axis[3];
                    // Remove whole tiles near a distant model origin before packing f32.
                    let phase = if span.origin == DVec3::ZERO {
                        phase
                    } else {
                        phase.rem_euclid(1.)
                    };
                    for (row, p) in rows.chunks_exact_mut(32).zip(&span.points) {
                        let value = ((grad.dot(*p) + phase) as f32).to_le_bytes();
                        if self.front {
                            row[k * 4..k * 4 + 4].copy_from_slice(&value);
                        }
                        if self.back {
                            row[8 + k * 4..12 + k * 4].copy_from_slice(&value);
                        }
                    }
                }
            }
            if let Some(all) = &span.full {
                let mut data = all.clone();
                data[span.start * 32..span.start * 32 + rows.len()].copy_from_slice(&rows);
                renderer.call_method1(
                    "update_prototype_materials",
                    (span.code, data.into_pyarray(renderer.py())),
                )?;
            } else {
                renderer.call_method1(
                    "set_tool_material_range",
                    (
                        span.code,
                        span.start,
                        rows.into_pyarray(renderer.py()),
                        uvw.is_none(),
                    ),
                )?;
            }
        }
        Ok(())
    }
}
