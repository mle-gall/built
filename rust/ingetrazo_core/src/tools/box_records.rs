// SPDX-License-Identifier: GPL-3.0-or-later
//! Box selection over scripted mesh and annotation records. Native cameras
//! project once per batch. Scripted views may supply a world-to-pixel callback.
use super::host_geometry::Projection;
use crate::{math::xyz, pick::inside};
use glam::{DMat4, DVec3};
use pyo3::prelude::*;

type Pixel = [f64; 2];

#[pyfunction]
pub fn segment_overlaps_box(a: Pixel, b: Pixel, rect: [f64; 4]) -> bool {
    let [x0, y0, x1, y1] = rect;
    if inside(&rect, a[0], a[1]) || inside(&rect, b[0], b[1]) {
        return true;
    }
    let ccw = |p: Pixel, q: Pixel, r: Pixel| {
        (r[1] - p[1]) * (q[0] - p[0]) > (q[1] - p[1]) * (r[0] - p[0])
    };
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .any(|(&p, &q)| ccw(p, q, a) != ccw(p, q, b) && ccw(a, b, p) != ccw(a, b, q))
}

struct Query<'py> {
    camera: Projection<'py>,
    rect: [f64; 4],
    crossing: bool,
}

impl Query<'_> {
    fn point(&self, p: &Bound<'_, PyAny>) -> PyResult<Option<Pixel>> {
        self.camera.pixel(xyz(p)?)
    }
    fn inside(&self, p: Option<Pixel>) -> bool {
        p.is_some_and(|[x, y]| inside(&self.rect, x, y))
    }
    fn segment(&self, a: Option<Pixel>, b: Option<Pixel>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) if self.crossing => segment_overlaps_box(a, b, self.rect),
            _ => self.inside(a) && self.inside(b),
        }
    }
    fn ring(&self, points: &Bound<'_, PyAny>, sides: bool) -> PyResult<bool> {
        let mut pixels = Vec::new();
        for p in points.try_iter()? {
            let Some(q) = self.point(&p?)? else {
                return Ok(false);
            };
            pixels.push(q);
        }
        Ok(if self.crossing {
            pixels.iter().any(|&p| self.inside(Some(p)))
                || (sides
                    && pixels
                        .iter()
                        .zip(pixels.iter().cycle().skip(1))
                        .any(|(&a, &b)| self.segment(Some(a), Some(b))))
        } else {
            pixels.iter().all(|&p| self.inside(Some(p)))
        })
    }
    fn record(
        &self,
        kind: &str,
        item: &Bound<'_, PyAny>,
        view: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        let segment = |pair: &Bound<'_, PyAny>| -> PyResult<bool> {
            Ok(self.segment(
                self.point(&pair.get_item(0)?)?,
                self.point(&pair.get_item(1)?)?,
            ))
        };
        Ok(match kind {
            "edges" => self.segment(
                self.point(&item.getattr("a")?)?,
                self.point(&item.getattr("b")?)?,
            ),
            "faces" => self.ring(&item.getattr("vertices")?, true)?,
            "dimensions" => segment(&item.call_method0("line_points")?)?,
            "text_labels" => {
                let end = self.point(&item.call_method0("position")?)?;
                let anchor = self.point(&item.getattr("anchor")?)?;
                if anchor.is_none() {
                    self.inside(end)
                } else {
                    self.segment(anchor, end)
                }
            }
            "guides" if item.getattr("is_line")?.is_truthy()? => {
                if !self.crossing {
                    return Ok(false);
                }
                let mut pair = item.call_method0("segment")?;
                if let Some(clip) = view.getattr_opt("_clip_segment_front")? {
                    pair = clip.call1((pair.get_item(0)?, pair.get_item(1)?))?;
                }
                !pair.is_none() && segment(&pair)?
            }
            "guides"
                if item
                    .getattr_opt("is_circle")?
                    .is_some_and(|v| v.is_truthy().unwrap_or(false)) =>
            {
                self.ring(&item.call_method1("sample_points", (24,))?, false)?
            }
            "guides" => self.inside(self.point(&item.getattr("point")?)?),
            _ => false,
        })
    }
}

/// Native record filtering also serves the Python materialised mesh fallback.
#[pyfunction]
pub fn box_records(
    view: &Bound<'_, PyAny>,
    rect: [f64; 4],
    crossing: bool,
    loose: bool,
) -> PyResult<Vec<Py<PyAny>>> {
    let query = Query {
        camera: Projection::from_view(view)?,
        rect,
        crossing,
    };
    let names: &[&str] = if loose {
        &["edges", "faces"]
    } else {
        &["guides", "dimensions", "text_labels"]
    };
    let scene = view.getattr("scene")?;
    let selectable = scene.getattr_opt("entity_selectable")?;
    let mut selected = Vec::new();
    for &kind in names {
        let Some(items) = scene.getattr_opt(kind)? else {
            continue;
        };
        for item in items.try_iter()? {
            let item = item?;
            if let Some(f) = &selectable {
                if !f.call1((&item,))?.is_truthy()? {
                    continue;
                }
            }
            if query.record(kind, &item, view)? {
                selected.push(item.unbind());
            }
        }
    }
    Ok(selected)
}

#[pyfunction]
pub fn box_placements(
    view: &Bound<'_, PyAny>,
    rect: [f64; 4],
    crossing: bool,
    placements: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    let query = Query {
        camera: Projection::from_view(view)?,
        rect,
        crossing,
    };
    let mut nonempty = false;
    for placement in placements.try_iter()? {
        let placement = placement?;
        let mesh = placement.get_item(0)?.getattr("mesh")?;
        let matrix = placement.get_item(1)?;
        let m = if matrix.is_none() {
            DMat4::IDENTITY
        } else {
            DMat4::from_cols_array(&matrix.call_method0("data")?.extract()?)
        };
        let project =
            |p: &Bound<'_, PyAny>| query.camera.pixel(m.transform_point3(DVec3::from(xyz(p)?)));
        for v in mesh.getattr("vertices")?.try_iter()? {
            nonempty = true;
            if query.inside(project(&v?.getattr("position")?)?) == crossing {
                return Ok(crossing);
            }
        }
        if crossing {
            for e in mesh.getattr("edges")?.try_iter()? {
                let e = e?;
                if query.segment(project(&e.getattr("a")?)?, project(&e.getattr("b")?)?) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(nonempty && !crossing)
}
