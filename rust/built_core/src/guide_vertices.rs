// SPDX-License-Identifier: GPL-3.0-or-later
//! Construction guide dash geometry, in model coordinates until upload.
use crate::math::xyz;
use crate::tools::plane::{axes_of, plane_axes, Axes};
use glam::DVec3;
use numpy::{IntoPyArray, PyArray1};
use pyo3::prelude::*;
use std::collections::HashSet;
use std::f64::consts::{PI, TAU};

pub const MIN_CIRCLE_DASHES: usize = 12;

struct Dashes {
    coords: Vec<f32>,
    spacing: f64,
    reach: f64,
    centre: DVec3,
    eye: Option<DVec3>,
    base_dist: f64,
}

impl Dashes {
    fn vertex(&mut self, point: DVec3) {
        self.coords.extend(point.as_vec3().to_array());
    }

    fn distance_scale(&self, point: DVec3) -> f64 {
        match self.eye {
            Some(eye) if self.base_dist > 0. => (point.distance(eye) / self.base_dist).max(1.),
            _ => 1.,
        }
    }

    fn line(&mut self, point: DVec3, direction: DVec3) {
        let foot = (self.centre - point).dot(direction);
        let scale = self.distance_scale(point + direction * foot);
        let start = foot - self.reach * scale;
        let spacing = self.spacing * scale;
        let count = (2. * self.reach / self.spacing).clamp(1., 4000.) as usize;
        for i in 0..count {
            let t = start + i as f64 * spacing;
            self.vertex(point + direction * t);
            self.vertex(point + direction * (t + spacing * 0.5));
        }
    }

    fn segment(&mut self, origin: DVec3, point: DVec3) {
        let delta = point - origin;
        let length = delta.length();
        if length < 1e-9 {
            return;
        }
        let direction = delta / length;
        let mut t = 0.;
        while t < length {
            self.vertex(origin + direction * t);
            self.vertex(origin + direction * (t + self.spacing * 0.5).min(length));
            t += self.spacing;
        }
    }

    fn circle(&mut self, centre: DVec3, radius: f64, normal: DVec3, axes: &Axes) {
        if radius <= 0. {
            return;
        }
        let spacing = self.spacing * self.distance_scale(centre);
        let n = ((TAU * radius / spacing) as usize).clamp(MIN_CIRCLE_DASHES, 4000);
        let angle = PI / n as f64;
        let chords = (angle / 0.15).ceil() as usize;
        let step = angle / chords as f64;
        let (u, v) = plane_axes(normal, axes);
        for i in 0..n {
            let start = TAU * i as f64 / n as f64;
            for j in 0..chords {
                let a = start + step * j as f64;
                for t in [a, a + step] {
                    self.vertex(centre + u * (radius * t.cos()) + v * (radius * t.sin()));
                }
            }
        }
    }
}

fn flag(obj: &Bound<'_, PyAny>, name: &str) -> PyResult<bool> {
    Ok(obj.hasattr(name)? && obj.getattr(name)?.is_truthy()?)
}

fn optional_point(obj: &Bound<'_, PyAny>, name: &str) -> PyResult<Option<DVec3>> {
    if !obj.hasattr(name)? {
        return Ok(None);
    }
    let point = obj.getattr(name)?;
    if point.is_none() {
        Ok(None)
    } else {
        xyz(&point).map(Some)
    }
}

#[pyfunction]
pub fn guide_vertices<'py>(
    py: Python<'py>,
    guides: &Bound<'_, PyAny>,
    spacing: f64,
    selection: &Bound<'_, PyAny>,
    centre: Option<&Bound<'_, PyAny>>,
    reach: f64,
    eye: Option<&Bound<'_, PyAny>>,
    axes: Option<&Bound<'_, PyAny>>,
) -> PyResult<(Bound<'py, PyArray1<f32>>, Vec<(usize, usize, bool)>)> {
    let centre = centre.map(xyz).transpose()?.unwrap_or(DVec3::ZERO);
    let eye = eye.map(xyz).transpose()?;
    let spacing = spacing.max(1e-6);
    let axes = axes_of(axes)?;
    let mut dashes = Dashes {
        coords: Vec::new(),
        spacing,
        reach: if reach > 0. { reach } else { spacing * 500. },
        centre,
        eye,
        base_dist: eye.map_or(0., |e| e.distance(centre)),
    };
    let selected = selection
        .try_iter()?
        .map(|v| v.map(|v| v.as_ptr()))
        .collect::<PyResult<HashSet<_>>>()?;
    let mut spans = Vec::new();
    for g in guides.try_iter()? {
        let g = g?;
        let start = dashes.coords.len() / 3;
        if flag(&g, "is_circle")? {
            dashes.circle(
                xyz(&g.getattr("center")?)?,
                g.getattr("radius")?.extract()?,
                xyz(&g.getattr("normal")?)?,
                &axes,
            );
        } else if let Some(point) = optional_point(&g, "point")? {
            if flag(&g, "is_line")? {
                if let Some(dir) = optional_point(&g, "direction")? {
                    dashes.line(point, dir);
                }
            } else if let Some(origin) = optional_point(&g, "origin")? {
                dashes.segment(origin, point);
            }
        }
        let count = dashes.coords.len() / 3 - start;
        if count > 0 {
            spans.push((start, count, selected.contains(&g.as_ptr())));
        }
    }
    Ok((dashes.coords.into_pyarray(py), spans))
}
