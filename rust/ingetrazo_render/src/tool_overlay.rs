// SPDX-License-Identifier: GPL-3.0-or-later
//! Rasterize just the dirty part of the tool overlay at the surface DPR. The
//! tile contains premultiplied RGBA for direct source-over composition.
use pyo3::{prelude::*, types::PyBytes};
use resvg::tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, StrokeDash,
    Transform,
};

type Mark = (String, Vec<[f32; 2]>, u32, f32, Option<u32>, Vec<f32>);
type Tile<'py> = (u32, u32, u32, u32, Bound<'py, PyBytes>);
fn path(kind: &str, points: &[[f32; 2]]) -> Option<Path> {
    let [first, second, ..] = points else {
        return None;
    };
    let mut p = PathBuilder::new();
    match kind {
        "rect" | "circle" => {
            let rect = Rect::from_ltrb(first[0], first[1], second[0], second[1])?;
            if kind == "rect" {
                p.push_rect(rect)
            } else {
                p.push_oval(rect)
            }
        }
        "line" | "polygon" => {
            p.move_to(first[0], first[1]);
            for &[x, y] in &points[1..] {
                p.line_to(x, y)
            }
            if kind == "polygon" {
                p.close();
            }
        }
        _ => return None,
    }
    p.finish()
}
fn paint(color: u32) -> Paint<'static> {
    let [r, g, b, a] = color.to_be_bytes();
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p
}
fn tile(marks: Vec<Mark>, width: u32, height: u32, dpr: f32) -> Option<(u32, u32, Pixmap)> {
    if marks.is_empty() || !dpr.is_finite() || dpr <= 0. {
        return None;
    }
    let (mut left, mut top, mut right, mut bottom) = (width as f32, height as f32, 0f32, 0f32);
    for (_, points, _, stroke, _, _) in &marks {
        let pad = stroke * dpr + 2.;
        for &[x, y] in points {
            left = left.min(x * dpr - pad);
            top = top.min(y * dpr - pad);
            right = right.max(x * dpr + pad);
            bottom = bottom.max(y * dpr + pad);
        }
    }
    let (x, y) = (left.floor() as u32, top.floor() as u32);
    let w = (right.ceil() as u32).min(width).saturating_sub(x);
    let h = (bottom.ceil() as u32).min(height).saturating_sub(y);
    let mut tile = Pixmap::new(w, h)?;
    let transform = Transform::from_row(dpr, 0., 0., dpr, -(x as f32), -(y as f32));
    for (kind, points, ink, width, fill, dash) in marks {
        let Some(p) = path(&kind, &points) else {
            continue;
        };
        if let Some(fill) = fill {
            tile.fill_path(&p, &paint(fill), FillRule::EvenOdd, transform, None);
        }
        let stroke = Stroke {
            width,
            line_cap: LineCap::Square,
            line_join: LineJoin::Bevel,
            dash: StrokeDash::new(dash.into_iter().map(|n| n * width).collect(), 0.),
            ..Default::default()
        };
        tile.stroke_path(&p, &paint(ink), &stroke, transform, None);
    }
    Some((x, y, tile))
}
pub(crate) fn render_tile(
    kind: &str,
    tool: &Bound<'_, PyAny>,
    view: &Bound<'_, PyAny>,
    width: u32,
    height: u32,
    dpr: f32,
) -> PyResult<Option<(u32, u32, Pixmap)>> {
    let marks = tool
        .py()
        .import("ingetrazo_core")?
        .call_method1("tool_overlay_marks", (kind, tool, view))?
        .extract()?;
    Ok(tile(marks, width, height, dpr))
}
#[pyfunction]
pub fn tool_overlay_tile<'py>(
    py: Python<'py>,
    kind: &str,
    tool: &Bound<'py, PyAny>,
    view: &Bound<'py, PyAny>,
    width: u32,
    height: u32,
    dpr: f32,
) -> PyResult<Option<Tile<'py>>> {
    Ok(render_tile(kind, tool, view, width, height, dpr)?
        .map(|(x, y, t)| (x, y, t.width(), t.height(), PyBytes::new(py, t.data()))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_clips_and_preserves_physical_pixel_origin() {
        let mark = || {
            (
                "rect".into(),
                vec![[8., 5.], [12., 9.]],
                0xffffffff,
                1.,
                Some(0xff0000ff),
                vec![],
            )
        };
        let (x, y, p) = tile(vec![mark()], 20, 30, 2.).unwrap();
        assert_eq!((x, y, p.width()), (12, 6, 8));
        assert_eq!(
            &p.data()[((14 - y) * p.width() + 18 - x) as usize * 4..][..4],
            &[255, 0, 0, 255]
        );
        assert!(tile(vec![mark()], 2, 2, 2.).is_none());
    }
}
