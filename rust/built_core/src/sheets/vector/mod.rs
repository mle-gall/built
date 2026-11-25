//! SVG and DXF consume the same positioned paper primitives as PDF.
use super::paint::{Op, Pen, Prim, Rgba};
use super::pdf::{self, Page, Picture};
use pyo3::{prelude::*, types::PyDict};
use std::{collections::HashMap, path::PathBuf};
mod dxf;
mod svg;

type Point = [f64; 2];
type Rings = Vec<Vec<Point>>;

trait Writer {
    fn clip(&mut self, rings: Option<Rings>) -> PyResult<()>;
    fn path(
        &mut self,
        rings: &[Vec<Point>],
        closed: bool,
        pen: Option<&Pen>,
        fill: Option<Rgba>,
    ) -> PyResult<()>;
    fn text(
        &mut self,
        origin: Point,
        direction: Point,
        value: &str,
        size: f64,
        bold: bool,
        italic: bool,
        color: Rgba,
    ) -> PyResult<()>;
    fn image(&mut self, rect: [Point; 4], picture: &Picture) -> PyResult<()>;
}

fn moved([x, y]: Point, [dx, dy]: Point) -> Point {
    [x + dx, y + dy]
}
fn rings_at(rings: &[Vec<Point>], origin: Point) -> Rings {
    rings
        .iter()
        .map(|r| r.iter().map(|p| moved(*p, origin)).collect())
        .collect()
}
fn clip_at(clip: &Prim, origin: Point) -> Option<Rings> {
    if let Prim::Clip { rect, rings } = clip {
        Some(match rect {
            Some(r) => vec![r.map(|p| moved(p, origin)).to_vec()],
            None => rings_at(rings, origin),
        })
    } else {
        None
    }
}
fn draw(page: &Page, pics: &mut HashMap<String, Picture>, out: &mut impl Writer) -> PyResult<()> {
    for item in &page.items {
        let at = |p| moved(p, item.origin);
        for Op { prim, pen, brush } in &item.ops {
            let pen = pen.as_ref().filter(|p| !p.none);
            match prim {
                Prim::Clip { .. } => out.clip(clip_at(prim, item.origin))?,
                Prim::Unclip { clip } => {
                    out.clip(clip.as_deref().and_then(|c| clip_at(c, item.origin)))?
                }
                Prim::Line { pts: [x, y, u, v] } => {
                    out.path(&[vec![at([*x, *y]), at([*u, *v])]], false, pen, None)?
                }
                Prim::Poly { pts, closed } => out.path(
                    &[pts.iter().copied().map(at).collect()],
                    *closed,
                    pen,
                    *brush,
                )?,
                Prim::Path { rings } => {
                    out.path(&rings_at(rings, item.origin), true, pen, *brush)?
                }
                Prim::RRect { pts, rx, ry } => out.path(
                    &[pdf::rounded(pts, *rx, *ry).into_iter().map(at).collect()],
                    true,
                    pen,
                    *brush,
                )?,
                Prim::Ellipse { center, ax, ay } => {
                    let pts = (0..128)
                        .map(|i| {
                            let (s, c) = (i as f64 * std::f64::consts::TAU / 128.).sin_cos();
                            at([
                                center[0] + ax[0] * c + ay[0] * s,
                                center[1] + ax[1] * c + ay[1] * s,
                            ])
                        })
                        .collect();
                    out.path(&[pts], true, pen, *brush)?;
                }
                Prim::Text {
                    rect,
                    flags,
                    text,
                    size,
                    bold,
                    italic,
                    underline,
                    color,
                } => {
                    if let Some(layout) = pdf::text_lines(rect, *flags, text, *size, *bold) {
                        for line in &layout.lines {
                            out.text(
                                at(line.origin),
                                layout.u,
                                &line.text,
                                *size,
                                *bold,
                                *italic,
                                *color,
                            )?;
                            if *underline {
                                let (a, b, w) = pdf::underline_of(&layout, line, *size);
                                out.path(
                                    &[vec![at(a), at(b)]],
                                    false,
                                    Some(&Pen::new(*color, w)),
                                    None,
                                )?;
                            }
                        }
                    }
                }
                Prim::Image { rect, source } => {
                    if !pics.contains_key(source) {
                        if let Some(p) = pdf::load_picture(source) {
                            pics.insert(source.clone(), p);
                        }
                    }
                    if let Some(pic) = pics.get(source) {
                        out.image(rect.map(at), pic)?;
                    }
                }
            }
        }
        out.clip(None)?;
    }
    Ok(())
}

fn png(pic: &Picture) -> PyResult<Vec<u8>> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&pic.rgba, pic.w, pic.h, image::ExtendedColorType::Rgba8)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature=(kind,path,comps,contents,pictures,profiles=None,scene=None,doc_path=None))]
pub fn sheet_vector(
    py: Python,
    kind: &str,
    path: PathBuf,
    comps: Vec<Bound<PyAny>>,
    contents: &Bound<PyDict>,
    pictures: &Bound<PyDict>,
    profiles: Option<&Bound<PyAny>>,
    scene: Option<&Bound<PyAny>>,
    doc_path: Option<&Bound<PyAny>>,
) -> PyResult<()> {
    let pages = pdf::pages_of(py, &comps, Some(contents), profiles, scene, doc_path)?;
    let [page] = pages.as_slice() else {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Expected one sheet",
        ));
    };
    let mut pics = pdf::pictures_of(Some(pictures))?;
    match kind {
        "svg" => svg::write(py, &path, page, &mut pics),
        "dxf" => dxf::write(py, &path, page, &mut pics),
        _ => Err(pyo3::exceptions::PyValueError::new_err(
            "Unknown sheet format",
        )),
    }
}
