use super::super::paint::Cap;
use super::*;
use pyo3::types::PyBytes;
use std::{fmt::Write, path::Path};

struct Svg<'p> {
    py: Python<'p>,
    data: String,
    clip_id: usize,
    clipped: bool,
}
fn path_data(rings: &[Vec<Point>], close: bool) -> String {
    let mut data = String::new();
    for ring in rings {
        for (i, [x, y]) in ring.iter().enumerate() {
            write!(data, "{}{} {}", if i == 0 { "M" } else { "L" }, x, y).unwrap();
        }
        if close {
            data.push('Z');
        }
    }
    data
}
impl Writer for Svg<'_> {
    fn clip(&mut self, rings: Option<Rings>) -> PyResult<()> {
        if self.clipped {
            self.data.push_str("</g>");
        }
        self.clipped = rings.is_some();
        if let Some(rings) = rings {
            self.clip_id += 1;
            let id = self.clip_id;
            write!(self.data,r#"<defs><clipPath id="c{id}"><path clip-rule="evenodd" d="{}"/></clipPath></defs><g clip-path="url(#c{id})">"#,path_data(&rings,true)).unwrap();
        }
        Ok(())
    }
    fn path(
        &mut self,
        rings: &[Vec<Point>],
        closed: bool,
        pen: Option<&Pen>,
        fill: Option<Rgba>,
    ) -> PyResult<()> {
        if rings.is_empty() {
            return Ok(());
        }
        if !closed && rings.len() == 1 && rings[0].len() == 2 {
            let [x, y] = rings[0][0];
            let [u, v] = rings[0][1];
            write!(self.data, r#"<line x1="{x}" y1="{y}" x2="{u}" y2="{v}""#).unwrap();
        } else {
            write!(
                self.data,
                r#"<path fill-rule="evenodd" d="{}""#,
                path_data(rings, closed)
            )
            .unwrap();
        }
        if let Some(p) = pen {
            let cap = match p.cap {
                Cap::Round => "round",
                Cap::Flat => "butt",
                Cap::Square => "square",
            };
            write!(
                self.data,
                r#" stroke="{}" stroke-opacity="{}" stroke-width="{}" stroke-linecap="{cap}""#,
                p.color.hex(),
                p.color.3 as f64 / 255.,
                p.width.max(0.01)
            )
            .unwrap();
            if !p.dash.is_empty() {
                let dash = p
                    .dash
                    .iter()
                    .map(|v| (v * p.width).to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(self.data, r#" stroke-dasharray="{dash}""#).unwrap();
            }
        }
        if let Some(c) = fill {
            write!(
                self.data,
                r#" fill="{}" fill-opacity="{}""#,
                c.hex(),
                c.3 as f64 / 255.
            )
            .unwrap();
        } else {
            self.data.push_str(" fill=\"none\"");
        }
        self.data.push_str("/>");
        Ok(())
    }
    fn text(
        &mut self,
        [x, y]: Point,
        [u, v]: Point,
        value: &str,
        size: f64,
        bold: bool,
        italic: bool,
        c: Rgba,
    ) -> PyResult<()> {
        let value = value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        write!(self.data,r#"<text xml:space="preserve" transform="matrix({u} {v} {} {u} {x} {y})" font-family="Inter, sans-serif" font-size="{size}" font-weight="{}" font-style="{}" fill="{}" fill-opacity="{}">{value}</text>"#,-v,if bold{"bold"}else{"normal"},if italic{"italic"}else{"normal"},c.hex(),c.3 as f64/255.).unwrap();
        Ok(())
    }
    fn image(&mut self, r: [Point; 4], pic: &Picture) -> PyResult<()> {
        let encoded: String = self
            .py
            .import("base64")?
            .call_method1("b64encode", (PyBytes::new(self.py, &png(pic)?),))?
            .call_method1("decode", ("ascii",))?
            .extract()?;
        let [x, y] = r[0];
        write!(self.data,r#"<image width="1" height="1" preserveAspectRatio="none" transform="matrix({} {} {} {} {x} {y})" xlink:href="data:image/png;base64,{encoded}"/>"#,r[1][0]-x,r[1][1]-y,r[3][0]-x,r[3][1]-y).unwrap();
        Ok(())
    }
}
pub(super) fn write(
    py: Python,
    path: &Path,
    page: &Page,
    pics: &mut HashMap<String, Picture>,
) -> PyResult<()> {
    let [w, h] = page.size;
    let data = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}">"#
    );
    let mut svg = Svg {
        py,
        data,
        clip_id: 0,
        clipped: false,
    };
    draw(page, pics, &mut svg)?;
    svg.data.push_str("</svg>");
    std::fs::write(path, svg.data)?;
    Ok(())
}
