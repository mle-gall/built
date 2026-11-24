// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered drawing marks in paper millimetres. Grid coordinates are signed so
//! adjacent frames generate exactly the same pattern across the paper origin.
use pyo3::{prelude::*, types::PyDict};
use std::{f64::consts::PI, ops::Range};

type Point = (f64, f64);
type Path = Vec<Point>;
type Box2 = [f64; 4];
#[derive(Default)]
struct Marks {
    lines: Vec<(f64, f64, f64, f64)>,
    dots: Vec<Point>,
    circles: Vec<(f64, f64, f64)>,
    polys: Vec<Path>,
    closed: Vec<Path>,
}
fn corners([x, y, r, b]: Box2) -> [Point; 4] {
    [(x, y), (r, y), (x, b), (r, b)]
}
fn limits(values: impl IntoIterator<Item = f64>) -> Point {
    values
        .into_iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
            (l.min(v), h.max(v))
        })
}
fn grid(lo: f64, hi: f64, step: f64) -> Range<i64> {
    ((lo / step).floor() as i64).saturating_sub(1)..((hi / step).ceil() as i64).saturating_add(1)
}
fn cells(b: Box2, w: f64, h: f64) -> impl Iterator<Item = (i64, i64)> {
    let rows = grid(b[1], b[3], h);
    grid(b[0], b[2], w).flat_map(move |x| rows.clone().map(move |y| (x, y)))
}
#[pyfunction]
pub(crate) fn hatch_random(ix: i64, iy: i64, n: i64) -> f64 {
    let h = ix.wrapping_mul(73856093) ^ iy.wrapping_mul(19349663) ^ n.wrapping_mul(83492791);
    let h = ((h ^ (h >> 13)) as u32).wrapping_mul(1274126177);
    ((h ^ (h >> 16)) & 0xffffff) as f64 / 16777216.
}
fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Path {
    vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
}

impl Marks {
    fn family(
        &mut self,
        b: Box2,
        angle: f64,
        ox: f64,
        oy: f64,
        mut along: f64,
        mut across: f64,
        dashes: &[f64],
        compensated: bool,
    ) {
        if across.abs() <= 1e-9 {
            return;
        }
        if across < 0. {
            across = -across;
            along = -along
        }
        let (uy, ux) = angle.to_radians().sin_cos();
        let (nx, ny) = (-uy, ux);
        let corners = corners(b);
        let (lo, hi) = limits(corners.map(|(x, y)| (x - ox) * nx + (y - oy) * ny));
        let rows = grid(lo, hi, across);
        if rows.end.saturating_sub(rows.start) > 20000 {
            return;
        }
        let period = crate::facegeom::py_sum(dashes.iter().map(|x| x.abs()), compensated);
        for k in rows.start..=rows.end {
            let bx = ox + k as f64 * (along * ux + across * nx);
            let by = oy + k as f64 * (along * uy + across * ny);
            let (t0, t1) = limits(corners.map(|(x, y)| (x - bx) * ux + (y - by) * uy));
            if dashes.is_empty() || period <= 1e-9 {
                self.lines
                    .push((bx + ux * t0, by + uy * t0, bx + ux * t1, by + uy * t1));
                continue;
            }
            let mut t = (t0 / period).floor() * period;
            if (t1 - t) / period * dashes.len() as f64 > 20000. {
                continue;
            }
            while t < t1 {
                let start = t;
                for &d in dashes {
                    if d > 0. {
                        let (a, b) = (t.max(t0), (t + d).min(t1));
                        if b > a {
                            self.lines
                                .push((bx + ux * a, by + uy * a, bx + ux * b, by + uy * b));
                        }
                    } else if d == 0. && t >= t0 && t <= t1 {
                        self.dots.push((bx + ux * t, by + uy * t));
                    }
                    t += d.abs();
                }
                if t == start {
                    break;
                }
            }
        }
    }

    fn scatter(&mut self, b: Box2, kind: &str, cell: f64) {
        if kind.starts_with("grain") {
            self.grain(b, cell, kind);
            return;
        }
        if let Some(ratio) = kind
            .strip_prefix("herring")
            .and_then(|s| s.parse::<f64>().ok())
        {
            let long = cell * ratio;
            let c = corners(b);
            let (u0, u1) = limits(c.map(|(x, y)| (x + y) / (2. * cell)));
            let (v0, v1) = limits(c.map(|(x, y)| (x - y) / (2. * long)));
            for i in (u0.floor() as i64 - 2)..(u1.ceil() as i64 + 2) {
                for j in (v0.floor() as i64 - 2)..(v1.ceil() as i64 + 2) {
                    let (x, y) = (
                        i as f64 * cell + j as f64 * long,
                        i as f64 * cell - j as f64 * long,
                    );
                    self.closed.push(rectangle(x, y, long, cell));
                    self.closed
                        .push(rectangle(x + long, y + cell - long, cell, long));
                }
            }
            return;
        }
        let width = if kind == "ticks" { cell * 5. } else { cell };
        let height = cell
            * match kind {
                "rubble" => 0.62,
                "scales" => 0.6,
                "studs" | "rings" | "canopy" => 0.87,
                _ => 1.,
            };
        if kind == "ticks" {
            let rows = grid(b[1], b[3], cell);
            for k in rows.start..=rows.end {
                let y = k as f64 * cell;
                self.lines.push((b[0] - cell, y, b[2] + cell, y));
            }
        }
        for (ix, iy) in cells(b, width, height) {
            let (x, y) = (ix as f64, iy as f64);
            let rand = |n| hatch_random(ix, iy, n);
            let (px, py) = ((x + rand(1)) * cell, (y + rand(2)) * cell);
            let stagger = 0.5 * iy.rem_euclid(2) as f64;
            match kind {
                "pores" if rand(3) >= 0.45 => self.circles.push((
                    (x + 0.2 + 0.6 * rand(5)) * cell,
                    (y + 0.2 + 0.6 * rand(9)) * cell,
                    cell * (0.08 + 0.1 * rand(4)),
                )),
                "ticks" => {
                    let x = (x + stagger + 0.2 * (rand(91) - 0.5)) * width;
                    let y = y * cell;
                    self.lines
                        .push((x, y + cell * 0.15, x + cell * 0.55, y + cell * 0.85));
                }
                "sand" | "concrete" => {
                    self.dots.push((px, py));
                    if kind == "concrete" || rand(3) < 0.5 {
                        self.dots.push(((x + rand(4)) * cell, (y + rand(5)) * cell))
                    }
                    if kind == "concrete" && rand(6) < 0.5 {
                        let r = cell * (0.22 + 0.14 * rand(7));
                        let spin = rand(8) * 2. * PI;
                        self.closed.push(
                            (0..3)
                                .map(|j| {
                                    let a = spin + j as f64 * 2.0944;
                                    (
                                        (x + 0.5) * cell + r * a.cos(),
                                        (y + 0.5) * cell + r * a.sin(),
                                    )
                                })
                                .collect(),
                        );
                    }
                }
                "gravel" => self.circles.push((px, py, cell * (0.16 + 0.14 * rand(3)))),
                "plaster" if rand(3) < 0.6 => self.dots.push((px, py)),
                "asphalt" => {
                    self.dots.push((px, py));
                    if rand(3) < 0.12 {
                        self.circles.push((px, py, cell * 0.22));
                    }
                }
                "strands" => {
                    for n in 0..2 {
                        let (s, c) = (rand(10 + n) * PI).sin_cos();
                        let (cx, cy) = ((x + rand(20 + n)) * cell, (y + rand(30 + n)) * cell);
                        let r = cell * 0.3;
                        self.lines
                            .push((cx - r * c, cy - r * s, cx + r * c, cy + r * s));
                    }
                }
                "chequer" => {
                    let (cx, cy) = ((x + 0.5) * cell, (y + 0.5) * cell);
                    let r = cell * 0.28;
                    let dy = if (ix ^ iy) & 1 != 0 { r } else { -r };
                    self.lines.push((cx - r, cy - dy, cx + r, cy + dy));
                }
                "shrubs" if rand(3) < 0.8 => {
                    for n in 0..4 {
                        let a = n as f64 * PI / 2. + rand(40 + n);
                        self.circles.push((
                            px + cell * 0.18 * a.cos(),
                            py + cell * 0.18 * a.sin(),
                            cell * (0.14 + 0.06 * rand(50 + n)),
                        ));
                    }
                }
                "grass" if rand(3) < 0.7 => {
                    for a in [-0.45f64, 0., 0.45] {
                        self.lines.push((
                            px,
                            py,
                            px + cell * 0.45 * a.sin(),
                            py - cell * 0.45 * a.cos(),
                        ));
                    }
                }
                "stone" | "rock" => {
                    let (f, n) = if kind == "rock" { (0.5, 13) } else { (0.45, 9) };
                    let [a, b, c, d] = [(ix, iy), (ix + 1, iy), (ix + 1, iy + 1), (ix, iy + 1)]
                        .map(|(i, j)| {
                            (
                                (i as f64 + f * (hatch_random(i, j, n) - 0.5)) * cell,
                                (j as f64 + f * (hatch_random(i, j, n + 1) - 0.5)) * cell,
                            )
                        });
                    if kind == "stone" {
                        self.closed.push(vec![a, b, c, d])
                    } else if rand(15) < 0.5 {
                        self.closed.extend([vec![a, b, c], vec![a, c, d]])
                    } else {
                        self.closed.extend([vec![a, b, d], vec![b, c, d]])
                    }
                }
                "studs" | "canopy" | "rings" => {
                    let (cx, cy) = ((x + stagger) * cell, y * height);
                    let sizes: &[f64] = match kind {
                        "studs" => &[0.2],
                        "canopy" => &[0.32],
                        _ => &[0.12, 0.24, 0.36],
                    };
                    self.circles
                        .extend(sizes.iter().map(|r| (cx, cy, cell * r)));
                    if kind == "canopy" {
                        self.dots.push((cx, cy));
                    }
                }
                "scales" => self.polys.push(
                    (0..11)
                        .map(|j| {
                            let a = j as f64 * PI / 10.;
                            (
                                (x + 0.5 + stagger) * cell + cell / 2. * a.cos(),
                                y * height + cell / 2. * 0.8 * a.sin(),
                            )
                        })
                        .collect(),
                ),
                "rubble" => {
                    let n = 6 + (rand(11) * 2.) as i64;
                    self.closed.push(
                        (0..n)
                            .map(|j| {
                                let a = rand(12) * PI + j as f64 * 2. * PI / n as f64;
                                (
                                    (x + 0.5 + stagger) * cell
                                        + cell * 0.42 * (0.78 + 0.22 * rand(20 + j)) * a.cos(),
                                    (y + 0.5) * height
                                        + height * 0.40 * (0.78 + 0.22 * rand(40 + j)) * a.sin(),
                                )
                            })
                            .collect(),
                    );
                }
                "cobbles" => self.closed.push(
                    (0..8)
                        .map(|j| {
                            let a = j as f64 * PI / 4.;
                            (
                                (x + 0.5 + 0.3 * (rand(16) - 0.5)) * cell
                                    + cell * 0.36 * (0.85 + 0.15 * rand(60 + j)) * a.cos(),
                                (y + 0.5 + 0.3 * (rand(17) - 0.5)) * cell
                                    + cell * 0.36 * (0.85 + 0.15 * rand(70 + j)) * a.sin(),
                            )
                        })
                        .collect(),
                ),
                _ => {}
            }
        }
    }

    fn grain(&mut self, b: Box2, gap: f64, kind: &str) {
        let (chance, wave, period) = match kind {
            "grain_open" => (0., 2.2, 14.),
            "grain_flame" => (0.3, 2.5, 6.),
            _ => (0.35, 1., 9.),
        };
        let eyes = kind == "grain_flame";
        let (kw, kh) = (gap * 18., gap * 9.);
        let mut knots = Vec::new();
        for (x, y) in cells(b, kw, kh) {
            let rand = |n| hatch_random(x, y, n);
            if rand(81) < chance {
                let cx = (x as f64 + rand(82)) * kw;
                let cy = (y as f64 + rand(83)) * kh;
                let r = gap * (0.9 + 0.7 * rand(84)) * if eyes { 1.6 } else { 1. };
                knots.push((cx, cy, r));
                let scales: &[f64] = if eyes {
                    &[0.3, 0.55, 0.8, 1.]
                } else {
                    &[0.55, 1.]
                };
                for &s in scales {
                    if eyes {
                        self.closed.push(
                            (0..24)
                                .map(|j| {
                                    let a = j as f64 * PI / 12.;
                                    (
                                        cx + r * 2. * s * a.cos(),
                                        cy + r * 0.6 * s * a.sin() * (1. - 0.35 * a.cos().powi(2)),
                                    )
                                })
                                .collect(),
                        );
                    } else {
                        self.closed.push(
                            (0..20)
                                .map(|j| {
                                    let a = j as f64 * PI / 10.;
                                    (cx + r * 1.6 * s * a.cos(), cy + r * 0.8 * s * a.sin())
                                })
                                .collect(),
                        );
                    }
                }
            }
        }
        if kind == "grain_open" {
            for (x, y) in cells(b, gap * 12., gap) {
                if hatch_random(x, y, 88) < 0.05 {
                    let cx = (x as f64 + hatch_random(x, y, 89)) * gap * 12.;
                    let cy = (y as f64 + 0.5) * gap;
                    let half = gap * (2. + 2. * hatch_random(x, y, 90));
                    self.closed.push(
                        (0..20)
                            .map(|j| {
                                let a = j as f64 * PI / 10.;
                                (cx + half * a.cos(), cy + gap * 0.28 * a.sin())
                            })
                            .collect(),
                    );
                }
            }
        }
        let step = gap * 0.4;
        let x0 = (b[0] / step).floor() * step - 2. * kw;
        for k in (b[1] / gap).floor() as i64 - 3..(b[3] / gap).ceil() as i64 + 4 {
            let base = k as f64 * gap + gap * 0.3 * (hatch_random(k, 0, 85) - 0.5);
            let phase = hatch_random(k, 0, 86) * 2. * PI;
            let sway = gap * wave * (0.15 + 0.15 * hatch_random(k, 0, 87));
            let mut pts = Path::new();
            let mut x = x0;
            while x <= b[2] + 2. * kw {
                let mut y = base + sway * (x / (gap * period) + phase).sin();
                for &(kx, ky, kr) in &knots {
                    let d = base - ky;
                    if d.abs() < kr * 4. && (x - kx).abs() < kr * 6. {
                        let push = kr
                            * 1.15
                            * (-((x - kx) / (kr * 2.2)).powi(2)).exp()
                            * (-(d / (kr * 2.2)).powi(2)).exp();
                        y += if d >= 0. { push } else { -push };
                    }
                }
                pts.push((x, y));
                if x + step == x {
                    break;
                }
                x += step;
            }
            self.polys.push(pts);
        }
    }

    fn waves(&mut self, b: Box2, band: f64, period: f64, amp: f64) {
        let start = (b[0] / period).floor() * period;
        let count = ((b[2] - start) / period).ceil().max(1.) as usize * 12;
        for k in grid(b[1], b[3], band) {
            let mid = (k as f64 + 0.5) * band;
            self.polys.push(
                (0..=count)
                    .map(|i| {
                        let x = start + i as f64 * period / 12.;
                        (x, mid + amp * (2. * PI * (x / period)).sin())
                    })
                    .collect(),
            );
        }
    }

    fn transformed(
        mut self,
        py: Python<'_>,
        angle: f64,
        (dx, dy): Point,
    ) -> PyResult<Bound<'_, PyDict>> {
        let (s, c) = angle.to_radians().sin_cos();
        let place = |x: f64, y: f64| (x * c - y * s + dx, x * s + y * c + dy);
        if angle != 0. || dx != 0. || dy != 0. {
            for t in &mut self.lines {
                let a = place(t.0, t.1);
                let b = place(t.2, t.3);
                *t = (a.0, a.1, b.0, b.1);
            }
            for p in self
                .dots
                .iter_mut()
                .chain(self.polys.iter_mut().flatten())
                .chain(self.closed.iter_mut().flatten())
            {
                *p = place(p.0, p.1);
            }
            for (x, y, _) in &mut self.circles {
                (*x, *y) = place(*x, *y);
            }
        }
        let d = PyDict::new(py);
        d.set_item("lines", self.lines)?;
        d.set_item("dots", self.dots)?;
        d.set_item("circles", self.circles)?;
        d.set_item("polys", self.polys)?;
        d.set_item("closed", self.closed)?;
        Ok(d)
    }
}

#[pyfunction]
#[pyo3(signature=(key,origin,along,across,length,thick,density=1.0))]
pub(crate) fn fitted_marks(
    key: &str,
    origin: [f64; 2],
    along: [f64; 2],
    across: [f64; 2],
    length: f64,
    thick: f64,
    density: f64,
) -> Vec<Path> {
    let mut polys = Vec::new();
    if thick <= 1e-6 || length <= 1e-6 {
        return polys;
    }
    let at = |a: f64, b: f64| {
        (
            origin[0] + along[0] * a + across[0] * b,
            origin[1] + along[1] * a + across[1] * b,
        )
    };
    match key {
        "wood_cross" => {
            polys.push(vec![at(0., 0.), at(length, thick)]);
            polys.push(vec![at(0., thick), at(length, 0.)]);
        }
        "insulation_zigzag" => {
            let step = thick / density.max(0.2) / 2.;
            let n = (length / step).trunc() + 3.;
            if n <= 40000. {
                polys.push(
                    (-1..n as i64)
                        .map(|k| at(k as f64 * step, if k % 2 != 0 { thick } else { 0. }))
                        .collect(),
                );
            }
        }
        _ => {
            let r = (thick * 0.22 / density.max(0.2)).min(thick * 0.45);
            let n = (length / (2. * r)).trunc() + 3.;
            let far = thick - r;
            if n <= 20000. {
                for k in -1..n as i64 {
                    let near_c = 2. * r * k as f64;
                    let far_c = near_c + r;
                    polys.push(
                        (0..=18)
                            .map(|j| {
                                let a = PI * j as f64 / 18.;
                                at(near_c + r * a.cos(), r - r * a.sin())
                            })
                            .collect(),
                    );
                    polys.push(
                        (0..=18)
                            .map(|j| {
                                let a = PI * j as f64 / 18.;
                                at(far_c + r * a.cos(), far + r * a.sin())
                            })
                            .collect(),
                    );
                    polys.push(vec![at(near_c - r, r), at(near_c, far), at(near_c + r, r)]);
                }
            }
        }
    }
    polys
}

#[pyfunction]
#[pyo3(signature=(key,bounds,scale,angle,families,offset,scatter,waves,fitted))]
pub(crate) fn generate_hatch<'py>(
    py: Python<'py>,
    key: &str,
    bounds: Box2,
    scale: f64,
    angle: f64,
    families: &Bound<'_, PyAny>,
    offset: Option<[f64; 2]>,
    scatter: Option<(&str, f64)>,
    waves: Option<(f64, f64, f64)>,
    fitted: Option<f64>,
) -> PyResult<Bound<'py, PyDict>> {
    let mut out = Marks::default();
    let scale = scale.max(0.05);
    let [dx, dy] = offset.unwrap_or_default();
    let mut b = [
        bounds[0] - dx,
        bounds[1] - dy,
        bounds[2] - dx,
        bounds[3] - dy,
    ];
    let turn = angle.rem_euclid(360.);
    let turn = if turn.abs() < 1e-9 { 0. } else { turn };
    if turn != 0. {
        let (s, c) = turn.to_radians().sin_cos();
        let back = corners(b).map(|(x, y)| (x * c + y * s, -x * s + y * c));
        let (x0, x1) = limits(back.map(|p| p.0));
        let (y0, y1) = limits(back.map(|p| p.1));
        b = [x0, y0, x1, y1];
    }
    for item in families.try_iter()? {
        let item = item?;
        let number = |i: usize| item.get_item(i)?.extract::<f64>();
        let angle = number(0)?;
        let mut x = number(1)?;
        let mut y = number(2)?;
        if item.len()? > 6 {
            let off = number(6)?;
            let (s, c) = angle.to_radians().sin_cos();
            x -= s * off;
            y += c * off;
        }
        let mut dashes = item.get_item(5)?.extract::<Vec<f64>>()?;
        for d in &mut dashes {
            *d *= scale;
        }
        out.family(
            b,
            angle,
            x * scale,
            y * scale,
            number(3)? * scale,
            number(4)? * scale,
            &dashes,
            py.version_info().minor >= 12,
        );
    }
    if let Some((kind, cell)) = scatter {
        out.scatter(b, kind, cell * scale);
    }
    if let Some((band, period, amp)) = waves {
        out.waves(b, band * scale, period * scale, amp * scale);
    }
    if let Some(band) = fitted {
        let h = band * scale;
        let w = h * 1.5;
        let rows = grid(b[1], b[3], h);
        let cols = grid(b[0], b[2], w);
        if key == "wood_cross" {
            if (rows.end - rows.start) as f64 * (cols.end - cols.start) as f64 <= 20000. {
                for k in rows {
                    for i in cols.clone() {
                        let x = i as f64 * w + k.rem_euclid(2) as f64 * w / 2.;
                        let y = k as f64 * h;
                        let mut board = rectangle(x, y, w, h);
                        board.push((x, y));
                        out.polys.push(board);
                        out.polys
                            .extend(fitted_marks(key, [x, y], [1., 0.], [0., 1.], w, h, 1.));
                    }
                }
            }
        } else {
            for k in rows {
                out.polys.extend(fitted_marks(
                    key,
                    [b[0] - h, k as f64 * h],
                    [1., 0.],
                    [0., 1.],
                    b[2] - b[0] + 2. * h,
                    h,
                    1.,
                ));
            }
        }
    }
    out.transformed(py, turn, (dx, dy))
}

/// Minimum-area bounding rectangle. The first hull edge wins equal-area ties.
#[pyfunction]
pub(crate) fn fit_band(
    ring: &Bound<'_, PyAny>,
) -> PyResult<Option<(Point, Point, Point, f64, f64)>> {
    let mut points = Vec::new();
    for row in ring.try_iter()? {
        let [x, y]: [f64; 2] = row?.extract()?;
        points.push((x, y));
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points.dedup();
    let half = |pts: &[Point]| {
        let mut h: Path = Vec::new();
        for &(x, y) in pts {
            while h.len() >= 2 {
                let (a, b) = h[h.len() - 2];
                let (c, d) = h[h.len() - 1];
                if (c - a) * (y - b) - (d - b) * (x - a) > 0. {
                    break;
                }
                h.pop();
            }
            h.push((x, y));
        }
        h.pop();
        h
    };
    let mut hull = half(&points);
    points.reverse();
    hull.extend(half(&points));
    if hull.len() < 3 {
        return Ok(None);
    }
    let mut best = None;
    let mut least = f64::INFINITY;
    for (&(x, y), &(r, b)) in hull.iter().zip(hull.iter().cycle().skip(1)) {
        let (ex, ey) = (r - x, b - y);
        let n = ex.hypot(ey);
        if n < 1e-9 {
            continue;
        }
        let (u, v) = (ex / n, ey / n);
        let (a0, a1) = limits(hull.iter().map(|&(x, y)| x * u + y * v));
        let (b0, b1) = limits(hull.iter().map(|&(x, y)| -x * v + y * u));
        let (w, h) = (a1 - a0, b1 - b0);
        let area = w * h;
        if area < least {
            let origin = (u * a0 - v * b0, v * a0 + u * b0);
            best = Some(if w >= h {
                (origin, (u, v), (-v, u), w, h)
            } else {
                (origin, (-v, u), (u, v), h, w)
            });
            least = area;
        }
    }
    Ok(best)
}
