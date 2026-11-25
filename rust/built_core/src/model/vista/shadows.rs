//! Clip a caster to the light side of each receiver, slide it along the
//! sun direction, then intersect with the receiver's visible region.
use super::*;
type Rings = Vec<Vec<[f64; 2]>>;
pub(super) struct Caster {
    pub world: Vec<Vec<P>>,
    pub identity: Py<PyList>,
}
fn above(r: &[P], n: P, origin: P) -> Vec<P> {
    let h: Vec<_> = r.iter().map(|&p| dot(sub(p, origin), n) - 1e-6).collect();
    let mut out = Vec::new();
    for i in 0..r.len() {
        let j = (i + 1) % r.len();
        if h[i] > 0. {
            out.push(r[i]);
        }
        if (h[i] > 0.) != (h[j] > 0.) {
            let t = h[i] / (h[i] - h[j]);
            out.push(std::array::from_fn(|a| r[i][a] + (r[j][a] - r[i][a]) * t));
        }
    }
    out
}
fn slid(rings: &[Vec<P>], n: P, p0: P, d: P, proj: &Projection) -> Rings {
    let mut out = Vec::new();
    for ring in rings {
        let mut pts = above(ring, n, p0);
        if pts.len() < 3 {
            continue;
        }
        for p in &mut pts {
            let t = dot(sub(*p, p0), n) / dot(d, n);
            *p = std::array::from_fn(|i| p[i] - t * d[i]);
        }
        let (xy, _) = proj.cam.ring(&pts);
        if xy.len() >= 3 {
            out.push(xy)
        }
    }
    out
}
struct Regions<'p> {
    class: Obj<'p>,
    even: Obj<'p>,
    add: Obj<'p>,
}
impl<'p> Regions<'p> {
    fn new(py: Python<'p>) -> PyResult<Self> {
        let m = py.import("manifold3d")?;
        Ok(Self {
            class: m.getattr("CrossSection")?,
            even: m.getattr("FillRule")?.getattr("EvenOdd")?,
            add: m.getattr("OpType")?.getattr("Add")?,
        })
    }
    fn section(&self, rings: &Rings) -> PyResult<Obj<'p>> {
        let scaled: Rings = rings
            .iter()
            .map(|r| r.iter().map(|p| p.map(|x| x * 1000.)).collect())
            .collect();
        self.class
            .call1((arrays(self.class.py(), &scaled)?, &self.even))
    }
    fn union(&self, pieces: Vec<Obj<'p>>) -> PyResult<Obj<'p>> {
        self.class
            .call_method1("batch_boolean", (pieces, &self.add))
    }
    fn rings(&self, section: &Obj<'p>) -> PyResult<Rings> {
        let mut out = vec![];
        for p in section.call_method0("to_polygons")?.try_iter()? {
            let r: Vec<[f64; 2]> = p?.extract()?;
            let r: Vec<_> = r.iter().map(|p| p.map(|x| x / 1000.)).collect();
            if r.len() >= 3 && signed_area(&r).abs() >= MIN_AREA {
                out.push(r)
            }
        }
        Ok(out)
    }
}
fn bounds(r: &[P], e1: P, e2: P) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for &p in r {
        let (x, y) = (dot(p, e1), dot(p, e2));
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    b
}
pub(super) fn cast<'p>(
    py: Python<'p>,
    faces: &[Obj<'p>],
    casters: &[Caster],
    sun: P,
    proj: &Projection,
    alpha: f64,
    class: &Obj<'p>,
) -> PyResult<Option<Obj<'p>>> {
    let d = unit(sun);
    let side = cross(d, [0., 0., 1.]);
    let e1 = if dot(side, side).sqrt() < 1e-9 {
        [1., 0., 0.]
    } else {
        unit(side)
    };
    let e2 = cross(d, e1);
    let boxes: Vec<_> = casters
        .iter()
        .map(|c| bounds(&c.world[0], e1, e2))
        .collect();
    let regions = Regions::new(py)?;
    let tick = progress(py)?;
    let mut over = Vec::new();
    for f in faces {
        tick.call0()?;
        if f.getattr("opacity")?.extract::<f64>()? < 0.999 {
            continue;
        }
        let extra = f.getattr("extra")?;
        let world = extra.get_item("world")?;
        let outer: Vec<P> = world.get_item(0)?.extract()?;
        let n = extra.get_item("seen_normal")?.extract()?;
        let visible = regions.section(&read_rings(&f.getattr("rings")?)?)?;
        over.push(visible.clone());
        let dark = if dot(n, d) <= 1e-6 {
            visible
        } else {
            let b = bounds(&outer, e1, e2);
            let mut pieces = Vec::new();
            for (c, k) in casters.iter().zip(&boxes) {
                if c.identity.as_ptr() == world.as_ptr()
                    || k[0] >= b[2]
                    || k[2] <= b[0]
                    || k[1] >= b[3]
                    || k[3] <= b[1]
                {
                    continue;
                }
                let rings = slid(&c.world, n, outer[0], d, proj);
                if !rings.is_empty() {
                    pieces.push(regions.section(&rings)?);
                }
            }
            if pieces.is_empty() {
                continue;
            }
            visible.call_method1("__xor__", (regions.union(pieces)?,))?
        };
        let rings = regions.rings(&dark)?;
        if !rings.is_empty() {
            f.setattr("shadow", arrays(py, &rings)?)?;
            f.setattr("shadow_alpha", alpha)?;
        }
    }
    if d[2] <= 1e-6 {
        return Ok(None);
    }
    let mut pieces = Vec::new();
    for c in casters {
        tick.call0()?;
        let r = slid(&c.world, [0., 0., 1.], [0.; 3], d, proj);
        if !r.is_empty() {
            pieces.push(regions.section(&r)?)
        }
    }
    if pieces.is_empty() {
        return Ok(None);
    }
    let mut ground = regions.union(pieces)?;
    if !over.is_empty() {
        ground = ground.sub(regions.union(over)?)?
    }
    let r = regions.rings(&ground)?;
    if r.is_empty() {
        return Ok(None);
    }
    let extra = PyDict::new(py);
    extra.set_item("ground_shadow", true)?;
    let kw = PyDict::new(py);
    kw.set_item("extra", extra)?;
    Ok(Some(class.call(
        (arrays(py, &r)?, (0., 0., 0.), "SUELO", alpha),
        Some(&kw),
    )?))
}
