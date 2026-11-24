// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use glam::DMat4;
use pyo3::types::{PyDict, PyList};
fn matrix<'py>(cam: &Obj<'py>, pick: bool) -> PyResult<Obj<'py>> {
    let projection = if pick {
        cam.getattr_opt("picking_projection_matrix")?
    } else {
        None
    };
    let proj = match projection {
        Some(f) => f.call0()?,
        None => cam.call_method0("projection_matrix")?,
    };
    proj.call_method1("__mul__", (cam.call_method0("view_matrix")?,))
}
#[pyfunction]
fn view_ray<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let m = matrix(&v.getattr("camera")?, true)?;
    let (w, h) = size(v)?;
    m.call_method1("pixel_ray", (x, y, w, h))
}
#[pyfunction]
fn view_pixel<'py>(v: &Obj<'py>, world: &Obj<'py>, mvp: &Obj<'py>) -> PyResult<Obj<'py>> {
    let m = if mvp.is_none() {
        matrix(&v.getattr("camera")?, false)?
    } else {
        mvp.clone()
    };
    let (w, h) = size(v)?;
    m.call_method1("world_pixel", (world, w, h))
}
#[pyclass]
struct PixelProjector {
    matrix: DMat4,
    width: f64,
    height: f64,
    #[pyo3(get)]
    snap_projection: Py<PyAny>,
}
#[pymethods]
impl PixelProjector {
    fn __call__(&self, world: &Obj) -> PyResult<Option<(f64, f64)>> {
        let p = self.matrix * xyz(world)?.extend(1.);
        Ok((p.w > 0.).then(|| {
            (
                (p.x / p.w * 0.5 + 0.5) * self.width,
                (1. - (p.y / p.w * 0.5 + 0.5)) * self.height,
            )
        }))
    }
}
#[pyfunction]
fn pixel_projector<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    if v.getattr("__dict__")?.contains("_world_to_pixel")? {
        return v.getattr("_world_to_pixel");
    }
    let m = matrix(&v.getattr("camera")?, false)?.call_method0("data")?;
    let (w, h) = size(v)?;
    let matrix = DMat4::from_cols_array(&m.extract()?);
    let snap_projection = (m, (w, h)).into_pyobject(v.py())?.into_any().unbind();
    Ok(Bound::new(
        v.py(),
        PixelProjector {
            matrix,
            width: w,
            height: h,
            snap_projection,
        },
    )?
    .into_any())
}
#[pyfunction]
fn camera_transform<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let m = matrix(&v.getattr("camera")?, false)?.call_method0("data")?;
    let (w, h) = size(v)?;
    Ok((v.py().get_type::<PyList>().call1((m,))?, w, h)
        .into_pyobject(v.py())?
        .into_any())
}
#[pyfunction]
fn camera_matrix_array<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let m = matrix(&v.getattr("camera")?, false)?.call_method0("data")?;
    let np = v.py().import("numpy")?;
    let kw = PyDict::new(v.py());
    kw.set_item("order", "F")?;
    np.call_method1("asarray", (m, np.getattr("float64")?))?
        .call_method("reshape", (4, 4), Some(&kw))
}
#[pyfunction]
fn clip_distance(v: &Obj) -> PyResult<f64> {
    let cam = v.getattr("camera")?;
    let clip = number(v, "_CLIP_W")?;
    if cam
        .getattr_opt("perspective")?
        .is_some_and(|p| !p.is_truthy().unwrap_or(true))
    {
        return Ok(clip);
    }
    let near = opt(&cam, "near_plane")?;
    Ok(if near.is_callable() {
        clip.min(near.call0()?.extract()?)
    } else {
        clip
    })
}
#[pyfunction]
fn instance_pixel_mask<'py>(v: &Obj<'py>, pool: &Obj<'py>, scale: f64) -> PyResult<Obj<'py>> {
    let mut memo = opt(v, "_inst_sphere_memo")?;
    if memo.is_none() || !memo.get_item(0)?.is(pool) {
        let bounds = v.call_method1("_bounding_spheres", (pool.get_item(2)?, pool.get_item(3)?))?;
        memo = (pool, bounds.get_item(0)?, bounds.get_item(1)?)
            .into_pyobject(v.py())?
            .into_any();
        v.setattr("_inst_sphere_memo", &memo)?;
    }
    v.call_method1(
        "_spheres_below_a_pixel",
        (
            memo.get_item(1)?,
            memo.get_item(2)?,
            number(v, "_INST_MIN_PX")? * scale,
        ),
    )
}
#[pyfunction]
fn small_pixel_spheres<'py>(
    v: &Obj<'py>,
    ctr: &Obj<'py>,
    rad: &Obj<'py>,
    floor: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let mut sz = v.getattr("_export_size")?;
    if !sz.is_truthy()? {
        sz = v.call_method0("_fb_size")?;
    }
    let cam = v.getattr("camera")?;
    let scale = cam
        .call_method0("projection_matrix")?
        .call_method0("data")?
        .get_item(5)?
        .extract::<f64>()?
        .abs()
        * sz.get_item(1)?.extract::<f64>()?;
    let floor = if floor.is_none() {
        v.getattr("_INST_MIN_PX")?
    } else {
        floor.clone()
    };
    v.py().import("ingetrazo_core")?.call_method1(
        "small_spheres",
        (
            ctr,
            rad,
            v.call_method0("_np_mvp")?.get_item(3)?,
            scale,
            cam.getattr("perspective")?,
            floor,
        ),
    )
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(view_ray, m)?)?;
    m.add_function(wrap_pyfunction!(view_pixel, m)?)?;
    m.add_function(wrap_pyfunction!(pixel_projector, m)?)?;
    m.add_function(wrap_pyfunction!(camera_transform, m)?)?;
    m.add_function(wrap_pyfunction!(camera_matrix_array, m)?)?;
    m.add_function(wrap_pyfunction!(clip_distance, m)?)?;
    m.add_function(wrap_pyfunction!(instance_pixel_mask, m)?)?;
    m.add_function(wrap_pyfunction!(small_pixel_spheres, m)?)?;
    Ok(())
}
