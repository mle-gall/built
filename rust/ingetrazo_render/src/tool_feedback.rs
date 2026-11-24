// SPDX-License-Identifier: GPL-3.0-or-later
//! Compose the active machine's surfaces, depth-tested wires, and selection
//! boxes in the renderer's transient mesh slot. A failed upload is retried.
use numpy::{IntoPyArray, PyArray1, PyArrayMethods};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyList},
};

type ImageRun<'py> = (String, f32, f32, Bound<'py, PyArray1<f32>>);
type StoredRun = (String, f32, f32, Vec<f32>);

// The frame may outlive a view's GPU surface. Keep only its identity.
#[derive(PartialEq)]
enum Target {
    Resident(u64),
    Extension(usize),
}

#[pyclass(module = "ingetrazo_render")]
struct MeshFrame {
    faces: Vec<u8>,
    wires: Vec<u8>,
    runs: Vec<StoredRun>,
    boxes: Py<PyAny>,
    renderer: Target,
}

impl MeshFrame {
    fn same(
        &self,
        faces: &[u8],
        wires: &[u8],
        runs: &[ImageRun<'_>],
        boxes: &Bound<'_, PyAny>,
        renderer: &Target,
    ) -> PyResult<bool> {
        if self.renderer != *renderer
            || self.faces != faces
            || self.wires != wires
            || self.runs.len() != runs.len()
            || !self.boxes.bind(boxes.py()).eq(boxes)?
        {
            return Ok(false);
        }
        for ((p, s, o, v), (path, shade, opacity, vertices)) in self.runs.iter().zip(runs) {
            if p != path || s != shade || o != opacity || v != vertices.readonly().as_slice()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[pyfunction]
pub fn sync_tool_preview(access: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = access.py();
    let view = access.getattr("view")?;
    let tool = access.getattr("tool")?;
    let faces = PyList::empty(py);
    if let Some(preview) = tool.getattr_opt("preview_faces")? {
        for f in preview.call0()?.try_iter()? {
            faces.append(f?)?;
        }
    }
    view.setattr("preview_faces", &faces)?;
    let Some(renderer) = view.getattr_opt("renderer")?.filter(|r| !r.is_none()) else {
        return Ok(());
    };
    let target = if let Ok(native) = renderer.cast::<crate::Renderer>() {
        Target::Resident(native.try_borrow()?.id)
    } else {
        Target::Extension(renderer.as_ptr() as usize)
    };
    let Some(upload) = renderer.getattr_opt("set_preview_mesh")? else {
        return Ok(());
    };
    let wires = PyList::empty(py);
    if tool
        .getattr_opt("wireframe_depth_tested")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false))
    {
        let band = py
            .import("views.wgpu_annotations")?
            .call_method1("rubber_band", (access,))?;
        if !band.is_none() && band.get_item(3)?.is_truthy()? {
            let color = band.get_item(1)?;
            for edge in band.get_item(0)?.try_iter()? {
                let e = edge?;
                wires.append((e.get_item(0)?, e.get_item(1)?, &color))?;
            }
        }
    }
    let source = view
        .getattr_opt("source_viewport")?
        .unwrap_or_else(|| py.None().into_bound(py));
    let opacity = tool
        .getattr_opt("preview_opacity")?
        .map(|x| x.extract())
        .transpose()?
        .unwrap_or(1.);
    let (plain, materials) =
        crate::preview_texture::preview_materials(py, faces.as_any(), &source, opacity)?;
    let (tri, line): (Bound<'_, PyArray1<u8>>, Bound<'_, PyArray1<u8>>) = py
        .import("ingetrazo_core")?
        .call_method1("pack_tool_preview", (plain, wires))?
        .extract()?;
    let mut runs: Vec<ImageRun<'_>> = Vec::new();
    if !materials.is_empty() {
        let asarray = py.import("numpy")?.getattr("ascontiguousarray")?;
        for m in materials.iter() {
            runs.push((
                m.get_item(0)?.extract()?,
                m.get_item(1)?.extract()?,
                m.get_item(2)?.extract()?,
                asarray.call1((m.get_item(3)?, "float32"))?.cast_into()?,
            ));
        }
    }
    let (box_key, boxes): (Bound<'_, PyAny>, Bound<'_, PyAny>) =
        if let Some(get) = view.getattr_opt("_selection_box_vertices")? {
            get.call0()?.extract()?
        } else {
            (py.None().into_bound(py), py.None().into_bound(py))
        };
    let (tri_read, line_read) = (tri.readonly(), line.readonly());
    let (tri_bytes, line_bytes) = (tri_read.as_slice()?, line_read.as_slice()?);
    if let Some(prev) = view.getattr_opt("_preview_key")? {
        if let Ok(frame) = prev.cast::<MeshFrame>() {
            if frame
                .borrow()
                .same(tri_bytes, line_bytes, &runs, &box_key, &target)?
            {
                return Ok(());
            }
        }
    }
    let edges = if !boxes.is_none() && !boxes.is_empty()? {
        let b = boxes.call_method0("tobytes")?.cast_into::<PyBytes>()?;
        [line_bytes, b.as_bytes()].concat().into_pyarray(py)
    } else {
        line.clone()
    };
    let mut stored = vec![];
    let images = PyList::empty(py);
    for (p, s, o, v) in runs {
        stored.push((p.clone(), s, o, v.readonly().as_slice()?.to_vec()));
        images.append((p, s, o, v))?;
    }
    if images.is_empty() {
        upload.call1((&tri, edges))?;
    } else {
        upload.call1((&tri, edges, images))?;
    }
    view.setattr(
        "_preview_key",
        MeshFrame {
            faces: tri_bytes.to_vec(),
            wires: line_bytes.to_vec(),
            runs: stored,
            boxes: box_key.unbind(),
            renderer: target,
        },
    )
}
