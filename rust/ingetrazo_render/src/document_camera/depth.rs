// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::document_host::host;
use glam::DVec3;
use numpy::ndarray::Array2;
use numpy::{AllowTypeChange, IntoPyArray, PyArrayLikeDyn};
use pyo3::types::PyTuple;

#[derive(Default)]
struct Bounds(Option<(DVec3, DVec3)>);
impl Bounds {
    fn add(&mut self, p: DVec3) {
        self.0 = Some(self.0.map_or((p, p), |(a, b)| {
            // Array reductions propagate a NaN coordinate. The camera then
            // refuses the fitted range for the frame.
            let bad = p.is_nan_mask() | a.is_nan_mask();
            (
                DVec3::select(bad, DVec3::NAN, a.min(p)),
                DVec3::select(bad, DVec3::NAN, b.max(p)),
            )
        }));
    }
    fn vertices(verts: &Obj) -> PyResult<Self> {
        let mut bounds = Self::default();
        for v in verts.try_iter()? {
            bounds.add(xyz(&v?.getattr("position")?)?);
        }
        Ok(bounds)
    }
    fn image(im: &Obj) -> PyResult<Self> {
        let mut bounds = Self::default();
        for p in im.call_method0("corners")?.try_iter()? {
            bounds.add(xyz(&p?)?);
        }
        Ok(bounds)
    }
}
#[pyfunction]
fn sync_reach(v: &Obj) -> PyResult<()> {
    let cam = v.getattr("camera")?;
    if cam.getattr("perspective")?.is_truthy()? {
        return Ok(());
    }
    let sc = v.getattr("scene")?;
    for key in ["tile_layer", "photo_mesh"] {
        if opt(&opt(&sc, key)?, "visible")?.is_truthy()? {
            return cam.setattr("depth_reach", v.py().None());
        }
    }
    let mut bounds = Bounds::default();
    bounds.add(xyz(&v.py().import("core.axes")?.getattr("ORIGIN")?)?);
    let (a, b): (Obj, Obj) = sc.call_method0("bounds")?.extract()?;
    if !a.is_none() {
        bounds.add(xyz(&a)?);
        bounds.add(xyz(&b)?);
    }
    let images = opt(&sc, "image_planes")?;
    if images.is_truthy()? {
        for im in images.try_iter()? {
            let im = im?;
            if sc.call_method1("entity_visible", (&im,))?.is_truthy()? {
                if let Some((a, b)) = Bounds::image(&im)?.0 {
                    bounds.add(a);
                    bounds.add(b);
                }
            }
        }
    }
    for g in sc.getattr("groups")?.try_iter()? {
        let g = g?;
        if opt(&g, "billboard")?.is_truthy()?
            && sc.call_method1("entity_visible", (&g,))?.is_truthy()?
        {
            let xf = g.getattr("xform")?;
            let p = if xf.is_none() {
                DVec3::ZERO
            } else {
                let col = xf.call_method1("column", (3,))?;
                DVec3::new(
                    col.call_method0("x")?.extract()?,
                    col.call_method0("y")?.extract()?,
                    col.call_method0("z")?.extract()?,
                )
            };
            bounds.add(p);
            bounds.add(p + DVec3::Z * 3.);
        }
    }
    let (a, b) = bounds.0.unwrap();
    cam.setattr(
        "depth_reach",
        (vec3(v.py(), (a + b) * 0.5)?, (b - a).length() * 0.5),
    )
}

#[pyfunction]
fn refuse_depth(v: &Obj) -> PyResult<bool> {
    let sc = v.getattr("scene")?;
    if !v.getattr("camera")?.getattr("perspective")?.is_truthy()?
        || !sc.getattr("edit_group")?.is_none()
        || opt(v, "_preview_groups")?.is_truthy()?
        || !opt(v, "_frozen_cache_version")?.is_none()
    {
        return Ok(true);
    }
    for key in ["_base_map_showing", "_terrain_showing", "_photo_showing"] {
        if v.call_method0(key)?.is_truthy()? {
            return Ok(true);
        }
    }
    if opt(&sc, "guides")?.is_truthy()?
        || opt(&opt(&sc, "shadows")?, "enabled")?.is_truthy()?
        || opt(&sc, "show_hidden_objects")?.is_truthy()?
        || opt(&sc, "show_hidden_geometry")?.is_truthy()?
    {
        return Ok(true);
    }
    let tool = opt(v, "active_tool")?;
    if tool.is_truthy()? {
        let provider = opt(&tool, "preview_faces")?;
        if provider.is_callable() {
            match provider.call0().and_then(|r| r.is_truthy()) {
                Ok(false) => {}
                Ok(true) => return Ok(true),
                Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(v.py()) => {
                    return Ok(true)
                }
                Err(e) => return Err(e),
            }
        }
    }
    opt(&tool, "wireframe_depth_tested")?.is_truthy()
}
fn missing(py: Python<'_>) -> PyResult<Obj<'_>> {
    Ok((py.None(), py.None()).into_pyobject(py)?.into_any())
}
#[pyfunction]
fn depth_boxes<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let sc = v.getattr("scene")?;
    let lazy = py.import("ingetrazo_core")?;
    let mut boxes = Vec::new();
    for g in v.call_method0("_placements")?.try_iter()? {
        let g = g?;
        if !sc.call_method1("entity_visible", (&g,))?.is_truthy()? {
            continue;
        }
        if opt(&g, "billboard")?.is_truthy()? {
            let verts = v
                .call_method1("_billboard_mesh", (&g,))?
                .getattr("vertices")?;
            if let Some((a, b)) = Bounds::vertices(&verts)?.0 {
                let c = (a + b) * 0.5;
                let extent = b - a;
                let radius = (extent.x * extent.x + extent.y * extent.y).sqrt() * 0.5;
                boxes.push((
                    DVec3::new(c.x - radius, c.y - radius, a.z),
                    DVec3::new(c.x + radius, c.y + radius, b.z),
                ));
            }
            continue;
        }
        let item = if g.getattr("xform")?.is_none() {
            v.call_method1("_group_chunk", (&g,))?
                .call_method1("get", ("bbox",))?
        } else {
            v.call_method1("_placement_bbox", (&g,))?
        };
        if item.is_none() {
            if lazy
                .call_method1("entity_count", (g.getattr("mesh")?,))?
                .eq(0)?
            {
                continue;
            }
            return missing(py);
        }
        let (a, b): ([f64; 3], [f64; 3]) = item.extract()?;
        boxes.push((DVec3::from_array(a), DVec3::from_array(b)));
    }
    let mesh = sc.getattr("mesh")?;
    let plan = lazy.call_method1("plan_of", (&mesh,))?;
    let loose = if plan.is_none() {
        Bounds::vertices(&mesh.getattr("vertices")?)?
    } else {
        let points = plan.getattr("pos")?;
        let array = points.extract::<PyArrayLikeDyn<f64, AllowTypeChange>>()?;
        let mut out = Bounds::default();
        let data = array.as_array();
        let mut it = data.iter();
        while let Some(&x) = it.next() {
            let y = it
                .next()
                .zip(it.next())
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("expected XYZ positions"))?;
            out.add(DVec3::new(x, *y.0, *y.1));
        }
        out
    };
    boxes.extend(loose.0);
    let images = opt(&sc, "image_planes")?;
    if images.is_truthy()? {
        for im in images.try_iter()? {
            let im = im?;
            if sc.call_method1("entity_visible", (&im,))?.is_truthy()? {
                boxes.extend(Bounds::image(&im)?.0);
            }
        }
    }
    if boxes.is_empty() {
        return missing(py);
    }
    let arr = |side: bool| {
        Array2::from_shape_fn((boxes.len(), 3), |(i, j)| {
            if side {
                boxes[i].1[j]
            } else {
                boxes[i].0[j]
            }
        })
        .into_pyarray(py)
    };
    Ok((arr(false), arr(true)).into_pyobject(py)?.into_any())
}
#[pyfunction]
fn fit_scene(v: &Obj) -> PyResult<()> {
    let py = v.py();
    let cam = v.getattr("camera")?;
    cam.setattr("_depth_range", py.None())?;
    if v.call_method0("_depth_fit_refused")?.is_truthy()? {
        return Ok(());
    }
    let key = (
        v.call_method0("_placements_epoch")?,
        host(py)?.call_method1("_content_ver", (v,))?,
        v.getattr("scene")?.getattr("mesh")?.as_ptr() as usize,
    )
        .into_pyobject(py)?;
    let mut memo = opt(v, "_depth_boxes")?;
    if memo.is_none() || !memo.get_item(0)?.eq(&key)? {
        let boxes = v.call_method0("_depth_fit_boxes")?;
        memo = (key, boxes.get_item(0)?, boxes.get_item(1)?)
            .into_pyobject(py)?
            .into_any();
        v.setattr("_depth_boxes", &memo)?;
    }
    let lo = memo.get_item(1)?;
    if !lo.is_none() {
        cam.call_method1("fit_depth_range", (lo, memo.get_item(2)?))?;
    }
    Ok(())
}
#[pyfunction]
fn navigation_detail(v: &Obj, mvp: &Obj, mode: &str) -> PyResult<bool> {
    if !v.getattr("_export_size")?.is_none() {
        return Ok(false);
    }
    let py = v.py();
    let camera = py
        .get_type::<PyTuple>()
        .call1((mvp.call_method0("data")?,))?;
    let old = opt(v, "_nav_detail_camera")?;
    v.setattr("_nav_detail_camera", &camera)?;
    let pool = opt(v, "_inst_pool")?;
    if pool.is_none()
        || pool.get_item(1)?.len()? < 2000
        || !matches!(mode, "shaded" | "textures" | "monochrome")
        || !v.getattr("scene")?.getattr("edit_group")?.is_none()
        || v.getattr("scene")?.getattr("selection")?.is_truthy()?
    {
        v.setattr("_nav_detail_until", 0.)?;
        return Ok(false);
    }
    let h = host(py)?;
    let now: f64 = h
        .getattr("_time_mod")?
        .call_method0("monotonic")?
        .extract()?;
    if !old.is_none() && !old.eq(&camera)? {
        v.setattr("_nav_detail_until", now + 0.35)?;
        let mut timer = opt(v, "_nav_detail_timer")?;
        if timer.is_none() {
            timer = h.call_method1("window_timer", (v, v.getattr("update")?))?;
            v.setattr("_nav_detail_timer", &timer)?;
        }
        timer.call_method1("start", (370,))?;
    }
    Ok(now
        < v.getattr_opt("_nav_detail_until")?
            .map(|x| x.extract::<f64>())
            .transpose()?
            .unwrap_or(0.))
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sync_reach, m)?)?;
    m.add_function(wrap_pyfunction!(refuse_depth, m)?)?;
    m.add_function(wrap_pyfunction!(depth_boxes, m)?)?;
    m.add_function(wrap_pyfunction!(fit_scene, m)?)?;
    m.add_function(wrap_pyfunction!(navigation_detail, m)?)?;
    Ok(())
}
