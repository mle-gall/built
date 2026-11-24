// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve a Push/Pull target in order of precision. References are in world
//! space, while the moving cap and any staged risers are excluded.
use super::{
    host_geometry::Projection, push_preview::push_guide_reference, push_references::PushReferences,
};
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PySet},
};

struct Search<'py> {
    view: Bound<'py, PyAny>,
    hidden: Bound<'py, PyAny>,
    key: Bound<'py, PyAny>,
    excluded: Bound<'py, PySet>,
    staged: Bound<'py, PyAny>,
    camera: Projection<'py>,
    px: [f64; 2],
    radius: f64,
    anchor: DVec3,
    normal: DVec3,
    minimum: f64,
}
impl Search<'_> {
    fn excluded(&self, p: &Bound<'_, PyAny>) -> PyResult<bool> {
        self.excluded.contains(self.key.call1((p,))?)
    }
    fn is_staged(&self, item: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(!item.get_type().getattr("__hash__")?.is_none() && self.staged.contains(item)?)
    }
    fn visible(&self, p: DVec3) -> PyResult<bool> {
        Ok(!self
            .hidden
            .call1((&self.view, Vec3::from(p)))?
            .is_truthy()?)
    }
    fn distance2(&self, p: DVec3) -> PyResult<Option<f64>> {
        Ok(self
            .camera
            .pixel(p)?
            .map(|[x, y]| (x - self.px[0]).powi(2) + (y - self.px[1]).powi(2))
            .filter(|d| *d <= self.radius * self.radius))
    }
    fn delta(&self, p: DVec3) -> f64 {
        (p - self.anchor).dot(self.normal)
    }
    fn midpoint(&self) -> PyResult<Option<DVec3>> {
        let mut edges = vec![];
        let kw = PyDict::new(self.view.py());
        kw.set_item("radius_px", self.radius)?;
        for name in ["_nearby_loose_edges", "_nearby_group_edges"] {
            if let Some(near) = self.view.getattr_opt(name)?.filter(|m| m.is_callable()) {
                for e in near.call((self.px[0], self.px[1]), Some(&kw))?.try_iter()? {
                    edges.push(e?);
                }
            }
        }
        if let Some(e) = self
            .view
            .getattr_opt("_hover_edge")?
            .filter(|e| !e.is_none())
        {
            edges.push(e);
        }
        let mut best = None;
        for edge in edges {
            if edge
                .getattr_opt("guide")?
                .is_some_and(|v| v.is_truthy().unwrap_or(false))
                || self.is_staged(&edge)?
            {
                continue;
            }
            let (a, b) = (edge.getattr("a")?, edge.getattr("b")?);
            if self.excluded(&a)? || self.excluded(&b)? {
                continue;
            }
            let (a, b) = (xyz(&a)?, xyz(&b)?);
            if a.distance_squared(b) < 1e-18 {
                continue;
            }
            let p = (a + b) * 0.5;
            if let Some(d) = self.distance2(p)? {
                if best.is_none_or(|(old, _)| d < old) && self.visible(p)? {
                    best = Some((d, p));
                }
            }
        }
        Ok(best.map(|(_, p)| p))
    }

    fn on_edge(&self) -> PyResult<Option<DVec3>> {
        let Some(e) = self
            .view
            .getattr_opt("_hover_edge")?
            .filter(|e| !e.is_none())
        else {
            return Ok(None);
        };
        let Some(project) = self
            .view
            .getattr_opt("_project_to_lock_line")?
            .filter(|p| p.is_callable())
        else {
            return Ok(None);
        };
        if self.is_staged(&e)? {
            return Ok(None);
        }
        let (a, b) = (e.getattr("a")?, e.getattr("b")?);
        if self.excluded(&a)? && self.excluded(&b)? {
            return Ok(None);
        }
        let a = xyz(&a)?;
        let line = xyz(&b)? - a;
        if line.length() <= 1e-9 || line.normalize().dot(self.normal).abs() > 0.999 {
            return Ok(None);
        }
        let on = xyz(&project.call1((Vec3::from(a), Vec3::from(line), self.px[0], self.px[1]))?)?;
        let p = a + line * ((on - a).dot(line) / line.length_squared()).clamp(0., 1.);
        Ok((self.delta(p).abs() >= self.minimum && self.visible(p)?).then_some(p))
    }

    fn borders_cap(&self, face: &Bound<'_, PyAny>) -> PyResult<bool> {
        let mut loops = vec![face.getattr("loop")?];
        for ring in face.getattr("hole_loops")?.try_iter()? {
            loops.push(ring?);
        }
        for ring in loops {
            let mut prev = false;
            let mut first = false;
            for (i, v) in ring.try_iter()?.enumerate() {
                let excluded = self.excluded(&v?.getattr("position")?)?;
                if prev && excluded {
                    return Ok(true);
                }
                if i == 0 {
                    first = excluded;
                }
                prev = excluded;
            }
            if first && prev {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn on_face(&self, tool: &Bound<'_, PyAny>) -> PyResult<Option<DVec3>> {
        let (face, group): (Bound<'_, PyAny>, Bound<'_, PyAny>) = tool
            .call_method1("_reference_face", (&self.view, self.px[0], self.px[1]))?
            .extract()?;
        if face.is_none()
            || face.eq(tool.getattr("base_face")?)?
            || self.is_staged(&face)?
            || (group.is_none() && self.borders_cap(&face)?)
        {
            return Ok(None);
        }
        let ray = self
            .view
            .call_method1("_pixel_to_ray", (self.px[0], self.px[1]))?;
        let (o, d) = (ray.get_item(0)?, ray.get_item(1)?);
        if o.is_none() || d.is_none() {
            return Ok(None);
        }
        let (o, d) = (xyz(&o)?, xyz(&d)?);
        let plane = self
            .view
            .py()
            .import("core.snap")?
            .call_method1("face_plane_world", (face, group.getattr_opt("xform")?))?;
        let (point, n) = (
            xyz(&plane.get_item(0)?)?,
            xyz(&plane.get_item(1)?)?.normalize_or_zero(),
        );
        let denom = n.dot(d);
        if denom.abs() < 1e-6 {
            return Ok(None);
        }
        let t = n.dot(point - o) / denom;
        let p = o + d * t;
        Ok((t > 0. && self.delta(p).abs() >= self.minimum).then_some(p))
    }
}

#[pyfunction]
pub fn push_reference_target(
    tool: &Bound<'_, PyAny>,
    view: &Bound<'_, PyAny>,
    px: [f64; 2],
    minimum: f64,
) -> PyResult<Option<(Vec3, &'static str, f64)>> {
    let py = view.py();
    let radius = view
        .getattr_opt("snap_threshold_px")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(9.);
    let key = py.import("core.topology")?.getattr("_key")?;
    let excluded = PySet::empty(py)?;
    for p in tool.getattr("_cap_positions")?.try_iter()? {
        excluded.add(key.call1((p?,))?)?;
    }
    let anchor = tool.getattr("_anchor")?;
    let normal = tool.getattr("_normal")?;
    let s = Search {
        view: view.clone(),
        hidden: tool.getattr("_reference_hidden")?,
        key,
        excluded,
        staged: tool.getattr("_stage_new")?,
        camera: Projection::from_view(view)?,
        px,
        radius,
        anchor: xyz(&anchor)?,
        normal: xyz(&normal)?,
        minimum,
    };
    let cache = tool.getattr("_infer_cache")?;
    let refs = match cache.cast::<PushReferences>() {
        Ok(r) => r.clone(),
        Err(_) => {
            let refs = Bound::new(
                py,
                PushReferences::new(view, (px[0], px[1]), (!cache.is_none()).then_some(&cache))?,
            )?;
            tool.setattr("_infer_cache", &refs)?;
            refs
        }
    };
    let mut best = refs
        .try_borrow_mut()?
        .nearest(view, px, radius, s.excluded.as_any(), &s.hidden)?
        .map(|(dd, p)| (dd, DVec3::from(p.coords()), "vertex"));
    let guides = py
        .import("views.document_interaction")?
        .call_method1("shown_guides", (view.getattr("scene")?,))?;
    let mut consider = |p: DVec3, kind: &'static str| -> PyResult<()> {
        if let Some(dd) = s.distance2(p)? {
            if best.is_none_or(|(old, _, _)| dd < old) {
                best = Some((dd, p, kind));
            }
        }
        Ok(())
    };
    for g in guides.try_iter()? {
        let g = g?;
        if g.getattr_opt("direction")?.is_none_or(|v| v.is_none()) {
            consider(xyz(&g.getattr("point")?)?, "guide_point")?;
        }
    }
    consider(DVec3::ZERO, "origin")?;
    let result = if let Some((_, p, kind)) = best {
        Some((p, kind))
    } else if let Some(p) = s.midpoint()? {
        Some((p, "midpoint"))
    } else if let Some((p, _)) =
        push_guide_reference(view, &guides, px, radius, &anchor, &normal, minimum)?
    {
        Some((DVec3::from(p.coords()), "guide_line"))
    } else if let Some(p) = s.on_edge()? {
        Some((p, "edge"))
    } else {
        s.on_face(tool)?.map(|p| (p, "face"))
    };
    Ok(result.map(|(p, k)| (p.into(), k, s.delta(p))))
}
