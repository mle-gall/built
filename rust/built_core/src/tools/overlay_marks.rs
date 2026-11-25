// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen-space display list for the modelling tools. Use the host projection
//! for scripted views and one camera matrix per overlay in the native view.
use super::{
    flip,
    host_geometry::{Projection, ToolBounds},
};
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::prelude::*;
type Pixel = [f64; 2];
type Mark = (&'static str, Vec<Pixel>, u32, f32, Option<u32>, Vec<f32>);
const WHITE: u32 = 0xffffffff;
const RED: u32 = 0xc31c2cff;

#[derive(Default)]
struct Marks(Vec<Mark>);
impl Marks {
    fn line(&mut self, p: [Pixel; 2], color: u32, width: f32, dash: &[f32]) {
        self.0
            .push(("line", p.into(), color, width, None, dash.into()));
    }
    fn at(
        &mut self,
        shape: &'static str,
        [x, y]: Pixel,
        r: f64,
        color: u32,
        width: f32,
        fill: Option<u32>,
    ) {
        self.0.push((
            shape,
            vec![[x - r, y - r], [x + r, y + r]],
            color,
            width,
            fill,
            vec![],
        ));
    }
    fn cross(&mut self, [x, y]: Pixel, r: f64, color: u32, width: f32) {
        self.line([[x - r, y], [x + r, y]], color, width, &[]);
        self.line([[x, y - r], [x, y + r]], color, width, &[]);
    }
    fn polygon(&mut self, p: Vec<Pixel>, color: u32, width: f32, fill: Option<u32>) {
        self.0.push(("polygon", p, color, width, fill, vec![]));
    }
}
fn hidden(view: &Bound<'_, PyAny>, p: DVec3) -> PyResult<bool> {
    match view.getattr_opt("_occluded_on_screen")? {
        Some(check) => check.call1((Vec3::from(p),))?.is_truthy(),
        None => Ok(false),
    }
}
fn offset(m: &mut Marks, tool: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>) -> PyResult<()> {
    let reference = tool.call_method1("_state", ("reference",))?;
    let Some([a, b, start]) = reference.extract::<Option<[[f64; 3]; 3]>>()? else {
        return Ok(());
    };
    let (a, b, start) = (DVec3::from(a), DVec3::from(b), DVec3::from(start));
    if hidden(view, start)? {
        return Ok(());
    }
    let camera = Projection::from_view(view)?;
    let Some(px) = camera.pixel(start)? else {
        return Ok(());
    };
    let segment = |a, b| -> PyResult<Option<[Pixel; 2]>> {
        view.call_method1("_segment_to_pixels", (Vec3::from(a), Vec3::from(b)))?
            .extract()
    };
    if !hidden(view, (a + b) * 0.5)? {
        if let Some(p) = segment(a, b)? {
            m.line(p, 0xffffffeb, 4., &[]);
            m.line(p, 0x0001ffff, 2., &[]);
        }
    }
    if tool.getattr("dragging")?.is_truthy()? {
        if let Some(inward) = tool
            .getattr("_ref_inward")?
            .extract::<Option<Bound<'_, PyAny>>>()?
        {
            let end = start + xyz(&inward)? * tool.getattr("distance")?.extract::<f64>()?;
            if let Some(p) = segment(start, end)? {
                m.line(p, RED, 1.25, &[4., 3.]);
                if let Some(px) = camera.pixel(end)? {
                    m.at("circle", px, 2.5, RED, 1.25, Some(WHITE));
                }
            }
        }
    }
    m.at("rect", px, 4., WHITE, 3.5, Some(RED));
    m.at("rect", px, 4., RED, 1.2, Some(RED));
    Ok(())
}
fn texture(m: &mut Marks, tool: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>) -> PyResult<()> {
    if !tool.getattr("active")?.is_truthy()? || !view.hasattr("_world_to_pixel")? {
        return Ok(());
    }
    let camera = Projection::from_view(view)?;
    let px = |p: &Bound<'_, PyAny>| camera.pixel(xyz(p)?);
    let line = |m: &mut Marks, p: &Bound<'_, PyAny>, color, width, dash: &[f32]| -> PyResult<()> {
        if let (Some(a), Some(b)) = (px(&p.get_item(0)?)?, px(&p.get_item(1)?)?) {
            m.line([a, b], color, width, dash)
        }
        Ok(())
    };
    for pair in tool.call_method0("grid_lines")?.try_iter()? {
        line(m, &pair?, 0xffffffaa, 1., &[1., 2.])?;
    }
    let placement = tool.getattr("_placement")?;
    if placement.getattr("draw_protractor")?.is_truthy()? {
        let disc = tool.call_method1("_protractor_segments", (view,))?;
        if !disc.is_none() {
            const BLUE: u32 = 0x3c3ce1eb;
            for name in ["current_arm", "base_stub"] {
                line(m, &disc.get_item(name)?, BLUE, 1.3, &[4., 2.])?;
            }
            let points = disc
                .get_item("wedge")?
                .try_iter()?
                .map(|p| px(&p?))
                .collect::<PyResult<Option<Vec<_>>>>()?;
            if let Some(points) = points {
                m.polygon(points, BLUE, 1., Some(0x3c3ce146));
            }
            for pair in disc.get_item("ring")?.try_iter()? {
                line(m, &pair?, BLUE, 1.3, &[])?;
            }
            line(m, &disc.get_item("diameter")?, BLUE, 1.3, &[])?;
            let squares = disc.get_item("squares")?;
            line(m, &squares, BLUE, 1., &[])?;
            for p in squares.try_iter()? {
                if let Some(p) = px(&p?)? {
                    m.at("rect", p, 2.5, BLUE, 1., Some(BLUE));
                }
            }
        }
    }
    let held = tool.getattr("_held")?;
    if !held.is_none() {
        if let Some([x, y]) = px(&held.get_item(0)?)? {
            if held.get_item(1)?.eq("middle")? {
                m.polygon(
                    vec![[x, y - 7.], [x + 7., y + 5.6], [x - 7., y + 5.6]],
                    0x28c8dcff,
                    2.,
                    None,
                );
            } else {
                m.at("rect", [x, y], 6., 0x28c83cff, 2., None);
            }
        }
    }
    let lifted = tool.getattr("_lifted")?.extract::<Option<usize>>()?;
    let hover = placement.getattr("hover_pin")?.extract::<Option<isize>>()?;
    let radius = tool.getattr("PIN_PX")?.extract()?;
    for (i, p) in tool.call_method0("pins")?.try_iter()?.enumerate() {
        if let Some(p) = px(&p?)? {
            let fill = [0xdb3845ff, 0x299e5cff, 0x3366c7ff, 0xf2c73378][i];
            let outline = if Some(i) == lifted || Some(i as isize) == hover {
                0x1e1e1eff
            } else {
                0xfffffff0
            };
            m.at("rect", p, radius, outline, 2., Some(fill));
            if Some(i) == lifted {
                m.at("circle", p, 2. * radius, fill, 1.5, None);
            }
        }
    }
    Ok(())
}
#[pyfunction]
pub fn tool_overlay_marks(
    kind: &str,
    tool: &Bound<'_, PyAny>,
    view: &Bound<'_, PyAny>,
) -> PyResult<Vec<Mark>> {
    let mut m = Marks::default();
    match kind {
        "walk" => {
            if let Some(p) = tool
                .call_method1("_state", ("anchor",))?
                .extract::<Option<Pixel>>()?
            {
                m.cross(p, 10., 0x1e1e1ee6, 1.5);
                m.at("circle", p, 4., 0xffffffc8, 1., None);
            }
        }
        "move" => {
            let grips = tool.getattr("_grips")?;
            if tool.getattr("_grip_rot")?.is_none() && grips.is_truthy()? {
                let hot = tool.getattr("_hot_grip")?.extract::<Option<usize>>()?;
                let camera = Projection::from_view(view)?;
                for (i, g) in grips.try_iter()?.enumerate() {
                    if let Some(p) = camera.pixel(xyz(&g?.get_item(0)?)?)? {
                        let (r, width) = if hot == Some(i) { (6., 3.) } else { (4., 2.) };
                        m.cross(p, r, 0xdc2828ff, width);
                    }
                }
            }
        }
        "tape" => {
            let start = tool.getattr("start_point")?;
            if !start.is_none() {
                let p = xyz(&start)?;
                if let Some(px) = Projection::from_view(view)?.pixel(p)? {
                    if !hidden(view, p)? {
                        m.at("rect", px, 3.5, WHITE, 1.5, Some(RED));
                    }
                }
            }
        }
        "flip" => {
            let bounds = ToolBounds::selection(&view.getattr("scene")?, None, false)?;
            if let (Some(lo), Some(hi)) = bounds.limits() {
                let (lo, hi) = (DVec3::from(lo.coords()), DVec3::from(hi.coords()));
                let camera = Projection::from_view(view)?;
                let active = tool
                    .getattr("_lock_axis")?
                    .extract::<Option<String>>()?
                    .or(tool.getattr("_hover_axis")?.extract()?);
                for (axis, color) in [("x", 0xd83844ff), ("y", 0x289e5aff), ("z", 0x3466c6ff)] {
                    let quad = flip::quad(
                        axis,
                        ((hi + lo) * 0.5).into(),
                        ((hi - lo) * 0.5).into(),
                        1.15,
                    );
                    let pixels = quad
                        .into_iter()
                        .map(|p| camera.pixel(p.into()))
                        .collect::<PyResult<Option<Vec<_>>>>()?;
                    if let Some(p) = pixels {
                        let (w, a) = if active.as_deref() == Some(axis) {
                            (2.2, 90)
                        } else {
                            (1.2, 36)
                        };
                        m.polygon(p, color, w, Some(color & 0xffffff00 | a));
                    }
                }
            }
        }
        "offset" => offset(&mut m, tool, view)?,
        "texture_position" => texture(&mut m, tool, view)?,
        _ => {}
    }
    Ok(m.0)
}
