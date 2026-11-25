//! Sample a repeated texture through its projective UV pins, clipped to
//! the page-space face. Evaluate the inverse map at pixel centres; a tile
//! may cross the projection horizon outside the face's outline.
use crate::{image_assets, survey_raster::SurveyRaster};
use glam::{DMat3, DVec2, DVec3};
use image::{Rgba, RgbaImage};
use pyo3::prelude::*;
use resvg::tiny_skia::{FillRule, Mask, PathBuilder, Transform};
use std::path::PathBuf;

type Point = (f64, f64);
fn basis(p: [Point; 4]) -> Option<DMat3> {
    let [a, b, c, d] = p.map(|(x, y)| DVec3::new(x, y, 1.));
    let basis = DMat3::from_cols(a, b, c);
    let weights = basis.inverse() * d;
    let result = basis * DMat3::from_diagonal(weights);
    (result.is_finite() && result.determinant() != 0.).then_some(result)
}
fn project(m: DMat3, p: DVec2) -> DVec2 {
    let v = m * p.extend(1.);
    v.truncate() / v.z
}
#[pyclass(frozen)]
pub struct UvProjection(DMat3);
#[pymethods]
impl UvProjection {
    #[staticmethod]
    fn pins(source: [Point; 4], page: [Point; 4]) -> Option<Self> {
        Some(Self(basis(page)? * basis(source)?.inverse()))
    }
    fn map<'py>(&self, p: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let x = p.call_method0("x")?.extract()?;
        let y = p.call_method0("y")?.extract()?;
        let v = project(self.0, DVec2::new(x, y));
        p.get_type().call1((v.x, v.y))
    }
}
#[pyclass(frozen)]
pub struct SvgTexture {
    pixels: RgbaImage,
    #[pyo3(get)]
    alpha: bool,
}
#[pymethods]
impl SvgTexture {
    #[staticmethod]
    fn open(py: Python<'_>, path: PathBuf, shade: f64, limit: u32) -> PyResult<Option<Self>> {
        let Some(mut pixels) = image_assets::decode(py, &path)? else {
            return Ok(None);
        };
        let alpha = image_assets::texture_has_alpha(py, path)?;
        py.detach(|| {
            let side = pixels.width().max(pixels.height());
            if side > limit {
                let w =
                    (u64::from(pixels.width()) * u64::from(limit) / u64::from(side)).max(1) as u32;
                let h =
                    (u64::from(pixels.height()) * u64::from(limit) / u64::from(side)).max(1) as u32;
                pixels = image_assets::resize(pixels, w, h);
            }
            if shade < 0.999 {
                for pix in pixels.pixels_mut() {
                    for c in &mut pix.0[..3] {
                        *c = (f64::from(*c) * shade).round().clamp(0., 255.) as u8;
                    }
                }
            }
            Ok(Some(Self { pixels, alpha }))
        })
    }
    fn raster(&self) -> SurveyRaster {
        SurveyRaster {
            pixels: self.pixels.clone(),
        }
    }
    fn bake(
        &self,
        py: Python<'_>,
        projection: &UvProjection,
        rings: Vec<Vec<Point>>,
        resolution: f64,
        limit: u32,
    ) -> Option<(Point, Point, SurveyRaster)> {
        py.detach(|| self.projected(projection.0, &rings, resolution, limit.min(4096)))
    }
}
impl SvgTexture {
    fn projected(
        &self,
        m: DMat3,
        rings: &[Vec<Point>],
        resolution: f64,
        limit: u32,
    ) -> Option<(Point, Point, SurveyRaster)> {
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = -lo;
        for &(x, y) in rings.iter().flatten() {
            let p = DVec2::new(x, y);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let size = hi - lo;
        if !size.is_finite() || size.min_element() <= 1e-6 || resolution <= 0. || limit == 0 {
            return None;
        }
        let s = resolution.min(f64::from(limit) / size.max_element());
        let (w, h) = ((size.x * s).ceil() as u32, (size.y * s).ceil() as u32);
        let mut mask = Mask::new(w, h)?;
        let mut path = PathBuilder::new();
        for ring in rings.iter().filter(|r| r.len() >= 3) {
            let center =
                ring.iter().map(|&(x, y)| DVec2::new(x, y)).sum::<DVec2>() / ring.len() as f64;
            for (i, &(x, y)) in ring.iter().enumerate() {
                let p = DVec2::new(x, y);
                let p = (p - lo) * s + (p - center).normalize_or_zero();
                if i == 0 {
                    path.move_to(p.x as f32, p.y as f32);
                } else {
                    path.line_to(p.x as f32, p.y as f32);
                }
            }
            path.close();
        }
        mask.fill_path(
            &path.finish()?,
            FillRule::EvenOdd,
            true,
            Transform::identity(),
        );
        let origin = project(m, DVec2::ZERO);
        let mip_size = |axis, max: u32| {
            ((project(m, axis) - origin).length() * s * 2.)
                .ceil()
                .clamp(4., f64::from(max.max(4))) as u32
        };
        let tw = mip_size(DVec2::X, self.pixels.width())
            .min(self.pixels.width())
            .max(1);
        let th = mip_size(DVec2::Y, self.pixels.height())
            .min(self.pixels.height())
            .max(1);
        let resized;
        let src = if (tw, th) != self.pixels.dimensions() {
            resized = image_assets::resize(self.pixels.clone(), tw, th);
            &resized
        } else {
            &self.pixels
        };
        let inverse = m.inverse();
        let background = if self.alpha {
            Rgba([0; 4])
        } else {
            Rgba([255; 4])
        };
        let pixels = RgbaImage::from_fn(w, h, |x, y| {
            let coverage = f64::from(mask.data()[(y * w + x) as usize]) / 255.;
            if coverage == 0. {
                return background;
            }
            let uv = project(
                inverse,
                lo + (DVec2::new(f64::from(x), f64::from(y)) + 0.5) / s,
            );
            if !uv.is_finite() {
                return background;
            }
            let at = DVec2::new(
                uv.x.rem_euclid(1.) * f64::from(tw),
                uv.y.rem_euclid(1.) * f64::from(th),
            ) - 0.5;
            let d = at - at.floor();
            let mut colour = [0.; 3];
            let mut a = 0.;
            for (dx, wx) in [(0, 1. - d.x), (1, d.x)] {
                for (dy, wy) in [(0, 1. - d.y), (1, d.y)] {
                    let ix = (at.x.floor() as i64 + dx).rem_euclid(i64::from(tw)) as u32;
                    let iy = (at.y.floor() as i64 + dy).rem_euclid(i64::from(th)) as u32;
                    let p = src.get_pixel(ix, iy).0;
                    let weight = f64::from(p[3]) / 255. * wx * wy * coverage;
                    a += weight;
                    for (sum, c) in colour.iter_mut().zip(p) {
                        *sum += weight * f64::from(c);
                    }
                }
            }
            if !self.alpha {
                for c in &mut colour {
                    *c += 255. * (1. - a);
                }
                a = 1.;
            }
            let mut rgba = [0u8; 4];
            if a > 0. {
                for (v, c) in rgba.iter_mut().zip(colour) {
                    *v = (c / a).round() as u8;
                }
            }
            rgba[3] = (a * 255.).round() as u8;
            Rgba(rgba)
        });
        Some((lo.into(), size.into(), SurveyRaster { pixels }))
    }
}
