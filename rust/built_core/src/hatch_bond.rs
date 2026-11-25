// SPDX-License-Identifier: GPL-3.0-or-later
//! Masonry courses and their intersections with a sheet's section plane.
//! Course coordinates are real millimetres; only the last transform uses paper units.
use crate::hatch_pattern::hatch_random;
use pyo3::{prelude::*, types::PyDict};
use std::f64::consts::PI;
type Point = (f64, f64);
type Rect = (f64, f64, f64, f64, &'static str);
type Course = (f64, f64, Vec<Rect>);
type Band = ([f64; 2], [f64; 2], [f64; 2], f64, f64);
type Map<'a> = &'a dyn Fn(f64, f64) -> Point;

fn row(
    rects: &mut Vec<Rect>,
    b: f64,
    top: f64,
    stride: f64,
    off: f64,
    span: [f64; 2],
    kind: &'static str,
) {
    let lo = ((span[0] - off) / stride).floor() as i64 - 1;
    let hi = ((span[1] - off) / stride).ceil() as i64 + 1;
    if hi - lo > 4000 {
        return;
    }
    for i in lo..=hi {
        let a = off + i as f64 * stride;
        rects.push((a, a + stride, b, top, kind));
    }
}

#[pyfunction]
pub(crate) fn masonry_course(
    key: &str,
    k: i64,
    thick: f64,
    dims: [f64; 5],
    lo: f64,
    hi: f64,
) -> Course {
    let [length, width, height, bed, head] = dims;
    let (ls, ws, hs) = (length + head, width + head, height + bed);
    let mut pieces = Vec::new();
    let span = [lo, hi];
    let parity = k.rem_euclid(2) as f64;
    let mut n = ((thick + head) / ws).round_ties_even().max(1.) as i64;
    let key = if key == "brick_auto" {
        if thick < (width + height) / 2. {
            "brick_on_edge"
        } else if n == 1 {
            "brick"
        } else {
            "brick_bond"
        }
    } else {
        key
    };
    let mut single = |depth, course, stride, off, kind| {
        row(&mut pieces, 0., depth, stride, off, span, kind);
        (depth, course, std::mem::take(&mut pieces))
    };
    match key {
        "block" | "aac_block" => {
            return single(
                thick,
                hs,
                ls,
                parity * ls / 2.,
                if key == "block" { "block" } else { "aac" },
            )
        }
        "ashlar" => return single(thick, 400., 800., parity * 400., "brick"),
        "brick_on_edge" => {
            return single(height + head, width + bed, ls, parity * ls / 2., "brick")
        }
        "brick_rowlock" => return single(ls, width + bed, height + head, 0., "brick"),
        "brick_honeycomb" => {
            let gap = ls / 2.;
            let stride = ls + gap;
            let off = parity * stride / 2.;
            row(&mut pieces, 0., ws, stride, off, span, "gap");
            row(&mut pieces, 0., ws, stride, off + gap, span, "brick");
            for p in &mut pieces {
                p.1 = p.0 + if p.4 == "gap" { gap } else { ls };
            }
            return (ws, hs, pieces);
        }
        "brick_header" => {
            let m = ((thick + head) / ls).round_ties_even().max(1.) as i64;
            for w in 0..m {
                row(
                    &mut pieces,
                    w as f64 * ls,
                    (w + 1) as f64 * ls,
                    ws,
                    parity * ws / 2.,
                    span,
                    "brick",
                );
            }
            return (m as f64 * ls, hs, pieces);
        }
        "brick" | "hollow_brick" | "perforated_block" | "concrete_brick" => {
            let kind = match key {
                "hollow_brick" => "hollow",
                "perforated_block" => "perforated",
                "concrete_brick" => "cored",
                _ => "brick",
            };
            for w in 0..n {
                row(
                    &mut pieces,
                    w as f64 * ws,
                    (w + 1) as f64 * ws,
                    ls,
                    ((k.wrapping_add(w)) & 1) as f64 * ls / 2.,
                    span,
                    kind,
                );
            }
            return (n as f64 * ws, hs, pieces);
        }
        "brick_flemish" | "brick_dutch" if key == "brick_flemish" || parity == 0. => {
            let stride = ls + ws;
            let off = (if key == "brick_flemish" {
                k
            } else {
                k.div_euclid(2)
            })
            .rem_euclid(2) as f64
                * stride
                / 2.;
            let first = ((lo - off) / stride).floor() as i64 - 1;
            let last = ((hi - off) / stride).ceil() as i64 + 1;
            for i in first..=last.min(first + 4000) {
                let a = off + i as f64 * stride;
                pieces.extend([
                    (a, a + ls, 0., ws, "brick"),
                    (a, a + ls, ws, 2. * ws, "brick"),
                    (a + ls, a + stride, 0., 2. * ws, "brick"),
                ]);
            }
            return (2. * ws, hs, pieces);
        }
        "brick_dutch" => return single(ls, hs, ws, ws / 2., "brick"),
        "brick_one_half" => n = 3,
        _ => {}
    }
    let n = n.clamp(2, 8);
    let shift = if key == "brick_cross" {
        k.div_euclid(2).rem_euclid(2) as f64 * ls / 2.
    } else {
        0.
    };
    let heads = n / 2;
    let mut wythes = vec![false; heads as usize];
    if parity == 0. {
        if n % 2 != 0 {
            wythes.insert(0, true);
        }
    } else if n % 2 != 0 {
        wythes.push(true);
    } else {
        wythes[0] = true;
        wythes.push(true);
    }
    let mut b = 0.;
    for stretcher in wythes {
        let (depth, stride, off) = if stretcher {
            (ws, ls, shift)
        } else {
            (ls, ws, ws / 2.)
        };
        row(&mut pieces, b, b + depth, stride, off, span, "brick");
        b += depth;
    }
    (b, hs, pieces)
}

#[derive(Default)]
struct Marks {
    lines: Vec<(f64, f64, f64, f64)>,
    polys: Vec<Vec<Point>>,
    bricks: Vec<Vec<Point>>,
}
fn outline(at: Map<'_>, a: f64, right: f64, b: f64, top: f64) -> Vec<Point> {
    vec![at(a, b), at(right, b), at(right, top), at(a, top), at(a, b)]
}

impl Marks {
    fn line(&mut self, p: Point, q: Point) {
        self.lines.push((p.0, p.1, q.0, q.1));
    }
    fn gap(&mut self, at: Map<'_>, a: f64, r: f64, b: f64, t: f64) {
        self.line(at(a, b), at(r, t));
        self.line(at(a, t), at(r, b));
        self.line(at(a, b), at(a, t));
        self.line(at(r, b), at(r, t));
    }
    fn brick(&mut self, at: Map<'_>, [a, r, b, t]: [f64; 4], depth: f64, head: f64, ha: f64) {
        let b = b + if b > 1e-6 { head } else { 0. };
        let t = t - if t < depth - 1e-6 { head } else { 0. };
        if r - a > 2. * ha && t > b {
            self.bricks.push(outline(at, a + ha, r - ha, b, t));
        }
    }
    fn pores(&mut self, at: Map<'_>, [a, r, b, t]: [f64; 4]) {
        let step = 28.;
        let i0 = (a / step).floor() as i64;
        let i1 = (r / step).ceil() as i64;
        let j0 = (b / step).floor() as i64;
        let j1 = (t / step).ceil() as i64;
        if (i1 - i0) as f64 * (j1 - j0) as f64 > 3000. {
            return;
        }
        for i in i0..i1 {
            for j in j0..j1 {
                let x = (i as f64 + 0.15 + 0.7 * hatch_random(i, j, 7)) * step;
                let y = (j as f64 + 0.15 + 0.7 * hatch_random(i, j, 11)) * step;
                let rad = 2. + 3. * hatch_random(i, j, 13);
                if a + rad < x && x < r - rad && b + rad < y && y < t - rad {
                    self.polys.push(
                        (0..9)
                            .map(|k| {
                                let ang = k as f64 * PI / 4.;
                                at(x + rad * ang.cos(), y + rad * ang.sin())
                            })
                            .collect(),
                    );
                }
            }
        }
    }
    fn plan_cells(&mut self, at: Map<'_>, [mut a, mut r, b, t]: [f64; 4], kind: &str, half: f64) {
        match kind {
            "block" => {
                let shell = 32f64.min(0.25 * (t - b));
                let mid = (a + r) / 2.;
                for (lo, hi) in [(a + shell, mid - shell / 2.), (mid + shell / 2., r - shell)] {
                    if hi > lo {
                        self.polys.push(outline(at, lo, hi, b + shell, t - shell));
                    }
                }
            }
            "aac" => self.pores(at, [a, r, b, t]),
            "cored" | "perforated" => {
                a += half;
                r -= half;
                if a >= r || b >= t {
                    return;
                }
                if kind == "cored" {
                    let n = ((r - a) / 80.).round_ties_even().clamp(2., 4.) as i64;
                    let cell = (r - a) / n as f64;
                    for i in 0..n {
                        self.polys.push(outline(
                            at,
                            a + i as f64 * cell + 0.2 * cell,
                            a + (i + 1) as f64 * cell - 0.2 * cell,
                            b + 0.25 * (t - b),
                            t - 0.25 * (t - b),
                        ));
                    }
                } else {
                    let cols = ((r - a) / 32.).round_ties_even().max(3.) as i64;
                    let rows = ((t - b) / 32.).round_ties_even().max(2.) as i64;
                    if cols * rows > 400 {
                        return;
                    }
                    let (cw, ch) = ((r - a) / cols as f64, (t - b) / rows as f64);
                    for i in 0..cols {
                        for j in 0..rows {
                            self.polys.push(outline(
                                at,
                                a + i as f64 * cw + 0.22 * cw,
                                a + (i + 1) as f64 * cw - 0.22 * cw,
                                b + j as f64 * ch + 0.22 * ch,
                                b + (j + 1) as f64 * ch - 0.22 * ch,
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn section_cells(
        &mut self,
        at: Map<'_>,
        [a, r, b, t]: [f64; 4],
        kind: &str,
        dims: [f64; 5],
        double: bool,
    ) {
        let [_, width, height, bed, _] = dims;
        let top = a + (r - a) * height / (height + bed);
        match kind {
            "hollow" => {
                let a = a + if double { bed / 2. } else { 0. };
                let cols = ((t - b) / (width / 3.)).round_ties_even().max(2.) as i64;
                let (cw, ch) = ((t - b) / cols as f64, (top - a) / 2.);
                for row in 0..2 {
                    for col in 0..cols {
                        let x0 = b + col as f64 * cw + 0.2 * cw;
                        let x1 = b + (col + 1) as f64 * cw - 0.2 * cw;
                        let y0 = a + row as f64 * ch + 0.22 * ch;
                        let y1 = a + (row + 1) as f64 * ch - 0.22 * ch;
                        self.polys.push(vec![
                            at(y0, x0),
                            at(y0, x1),
                            at(y1, x1),
                            at(y1, x0),
                            at(y0, x0),
                        ]);
                    }
                }
            }
            "perforated" => {
                let n = ((t - b) / 30.).round_ties_even().max(2.) as i64;
                for i in 1..n {
                    let y = b + (t - b) * i as f64 / n as f64;
                    self.line(at(a, y), at(top, y));
                }
            }
            "cored" => {
                for f in [0.25, 0.75] {
                    let y = b + (t - b) * f;
                    self.line(at(a, y), at(top, y));
                }
            }
            "aac" => self.pores(at, [a, r, b, t]),
            "block" => {
                let shell = 32f64.min(0.25 * (t - b));
                for y in [b + shell, t - shell] {
                    self.line(at(a, y), at(r, y));
                }
            }
            _ => {}
        }
    }
    fn result(self, py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
        let out = PyDict::new(py);
        out.set_item("lines", self.lines)?;
        out.set_item("polys", self.polys)?;
        out.set_item("bricks", self.bricks)?;
        Ok(out)
    }
}

#[pyfunction]
#[pyo3(signature=(key,band,cut,scale,scale_n,dims,unit,joints,offset=None))]
pub(crate) fn masonry_marks<'py>(
    py: Python<'py>,
    key: &str,
    band: Band,
    cut: &str,
    scale: f64,
    scale_n: f64,
    dims: [f64; 5],
    unit: [f64; 2],
    joints: &str,
    offset: Option<[f64; 2]>,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let (mut origin, mut along, across, length, thick) = band;
    if length <= 1e-6 || thick <= 1e-6 {
        return Ok(None);
    }
    let [length_mm, width, height, mut bed, mut head] = dims;
    let per_mm = scale.max(1e-6) * unit[0] / (unit[1] * 1000.);
    let real = thick * scale_n.max(1e-6);
    let [ox, oy] = offset.unwrap_or_default();
    let vertical = cut == "vertical";
    let double = joints == "double";
    let flip = if vertical {
        along[1] > 1e-9 || (along[1].abs() <= 1e-9 && along[0] < 0.)
    } else {
        along[0] < -1e-9 || (along[0].abs() <= 1e-9 && along[1] < 0.)
    };
    if flip {
        for k in 0..2 {
            origin[k] += along[k] * length;
            along[k] = -along[k];
        }
    }
    let left = [-along[1], along[0]];
    if across[0] * left[0] + across[1] * left[1] < 0. {
        for i in 0..2 {
            origin[i] += across[i] * thick;
        }
    }
    let across = left;
    let start = if vertical { -oy } else { -ox };
    let span = length / per_mm;
    if key == "ashlar" {
        bed = 10.;
        head = 10.;
    }
    let (half_h, half_b) = (head / 2., bed / 2.);
    let mut out = Marks::default();
    if real > if vertical { 700. } else { 1500. } {
        let (u, v, ul, vl, corner) = if vertical {
            let end = [
                origin[0] + across[0] * thick + along[0] * length,
                origin[1] + across[1] * thick + along[1] * length,
            ];
            (
                [1., 0.],
                [0., -1.],
                (end[0] - origin[0]).abs(),
                (end[1] - origin[1]).abs(),
                [origin[0].min(end[0]), origin[1].max(end[1])],
            )
        } else {
            (along, across, length, thick, origin)
        };
        let u0 = (corner[0] * u[0] + corner[1] * u[1]) / per_mm - ox;
        let v0 = (corner[0] * v[0] + corner[1] * v[1]) / per_mm - oy;
        let at = |a: f64, b: f64| {
            (
                corner[0] + (u[0] * (a - u0) + v[0] * (b - v0)) * per_mm,
                corner[1] + (u[1] * (a - u0) + v[1] * (b - v0)) * per_mm,
            )
        };
        let (_, course, _) = masonry_course(key, 0, 0., dims, 0., 0.);
        let (ur, vr) = (ul / per_mm, vl / per_mm);
        let first = (v0 / course).floor() as i64;
        let last = ((v0 + vr) / course).ceil() as i64;
        if (last - first + 1) as f64 * (ur / width.max(1.)).max(1.) > 20000. {
            return Ok(None);
        }
        let face = match key {
            "brick" | "hollow_brick" | "brick_honeycomb" | "block" | "aac_block"
            | "perforated_block" | "concrete_brick" => width,
            "brick_on_edge" => height,
            "brick_one_half" => length_mm + width + head,
            _ => length_mm,
        };
        for k in first..=last {
            let (a, b) = (k as f64 * course, (k + 1) as f64 * course);
            if !double {
                out.line(at(u0, a), at(u0 + ur, a));
            }
            let (_, _, rects) = masonry_course(key, k, face, dims, u0, u0 + ur);
            for (x, r, y, _, kind) in rects {
                if y > 1e-6 {
                    continue;
                }
                if kind == "gap" {
                    out.gap(&at, x, r, a, b);
                } else if double {
                    out.bricks
                        .push(outline(&at, x + half_h, r - half_h, a + half_b, b - half_b));
                } else {
                    out.line(at(x, a), at(x, b));
                }
            }
        }
    } else {
        let place = |a: f64, b: f64, depth: f64| {
            let (aa, bb) = ((a - start) * per_mm, b * thick / depth);
            (
                origin[0] + along[0] * aa + across[0] * bb,
                origin[1] + along[1] * aa + across[1] * bb,
            )
        };
        if !vertical {
            let (depth, _, rects) = masonry_course(key, 0, real, dims, start, start + span);
            if rects.len() > 4000 {
                return Ok(None);
            }
            let at = |a, b| place(a, b, depth);
            for (a, r, b, t, kind) in rects {
                if kind == "gap" {
                    out.gap(&at, a, r, b, t);
                    continue;
                }
                if double {
                    out.brick(&at, [a, r, b, t], depth, half_h, half_h);
                } else {
                    out.line(at(a, b), at(a, t));
                    if b > 1e-6 {
                        out.line(at(a, b), at(r, b));
                    }
                }
                out.plan_cells(&at, [a, r, b, t], kind, half_h);
            }
        } else {
            let (_, course, _) = masonry_course(key, 0, real, dims, 0., 0.);
            let first = (start / course).floor() as i64;
            let last = ((start + span) / course).ceil() as i64;
            if last - first + 1 > 2000 {
                return Ok(None);
            }
            let probe = 0.6 * (width + head);
            for k in first..=last {
                let (a, r) = (k as f64 * course, (k + 1) as f64 * course);
                let (depth, _, rects) = masonry_course(key, k, real, dims, probe, probe);
                let at = |a, b| place(a, b, depth);
                if !double {
                    out.line(at(a, 0.), at(a, depth));
                }
                for (lo, hi, b, t, kind) in rects {
                    if !(lo < probe && probe < hi) {
                        continue;
                    }
                    if kind == "gap" {
                        out.gap(&at, a, r, b, t);
                        continue;
                    }
                    if double {
                        out.brick(&at, [a, r, b, t], depth, half_h, half_b);
                    } else if b > 1e-6 {
                        out.line(at(a, b), at(r, b));
                    }
                    out.section_cells(&at, [a, r, b, t], kind, dims, double);
                }
            }
        }
    }
    out.result(py).map(Some)
}
