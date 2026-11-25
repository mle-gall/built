use super::*;

pub(super) type P = [f64; 3];
pub(super) fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(super) fn cross(a: P, b: P) -> P {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(super) fn sub(a: P, b: P) -> P {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(super) fn unit(a: P) -> P {
    let l = dot(a, a).sqrt().max(1e-12);
    a.map(|v| v / l)
}
pub(super) fn xyz(v: &Obj<'_>) -> PyResult<P> {
    if let Ok(vector) = v.cast::<crate::math::Vec3>() {
        return Ok(vector.borrow().coords());
    }
    Ok([
        v.call_method0("x")?.extract()?,
        v.call_method0("y")?.extract()?,
        v.call_method0("z")?.extract()?,
    ])
}
pub(super) fn matrix(obj: &Obj<'_>) -> PyResult<[[f64; 4]; 4]> {
    let vals: [f64; 16] = obj.call_method0("data")?.extract()?;
    Ok(std::array::from_fn(|i| {
        std::array::from_fn(|j| vals[j * 4 + i])
    }))
}
fn view_matrix(c: &Obj<'_>) -> PyResult<[[f64; 4]; 4]> {
    matrix(
        &c.call_method0("projection_matrix")?
            .mul(c.call_method0("view_matrix")?)?,
    )
}
fn clip_row(m: [f64; 4], p: P) -> f64 {
    dot([m[0], m[1], m[2]], p) + m[3]
}
#[pyclass(module = "core.vista_caras")]
pub(super) struct Projection {
    pub cam: Camera,
    pub ok: bool,
}
#[pymethods]
impl Projection {
    fn point<'p>(&self, py: Python<'p>, p: P) -> PyResult<Option<Obj<'p>>> {
        self.xy(p).map(|v| array1(py, &v)).transpose()
    }
}
impl Projection {
    pub fn xy(&self, p: P) -> Option<[f64; 2]> {
        match &self.cam {
            Camera::Parallel { eye, right, up, .. } => {
                let d = sub(p, *eye);
                Some([dot(d, *right), dot(d, *up)])
            }
            Camera::Perspective { m, scale, .. } => {
                let w = clip_row(m[3], p);
                (w > 1e-9).then(|| {
                    [
                        clip_row(m[0], p) / w * scale[0],
                        clip_row(m[1], p) / w * scale[1],
                    ]
                })
            }
        }
    }
    pub fn new(c: &Obj<'_>) -> PyResult<Self> {
        if c.getattr("perspective")?.is_truthy()? {
            perspective(c)
        } else {
            parallel(c)
        }
    }
}
#[pyfunction(name = "_Parallel")]
pub(super) fn parallel(camera: &Obj<'_>) -> PyResult<Projection> {
    let up = match camera.getattr_opt("up_vector")? {
        Some(f) => f.call0()?,
        None => camera.getattr("up")?,
    };
    let (eye, right, up, fwd) = super::super::hlr_view::basis(
        xyz(&camera.call_method0("eye")?)?,
        xyz(&camera.getattr("target")?)?,
        xyz(&up)?,
    );
    Ok(Projection {
        cam: Camera::Parallel {
            eye,
            right,
            up,
            fwd,
        },
        ok: true,
    })
}
#[pyfunction(name = "_Perspective")]
pub(super) fn perspective(camera: &Obj<'_>) -> PyResult<Projection> {
    let m = view_matrix(camera)?;
    let w = clip_row(m[3], xyz(&camera.getattr("target")?)?);
    let h = w * (camera.getattr("fov_deg")?.extract::<f64>()?.to_radians() / 2.).tan();
    Ok(Projection {
        cam: Camera::Perspective {
            m,
            eye: xyz(&camera.call_method0("eye")?)?,
            scale: [h * camera.getattr("aspect")?.extract::<f64>()?, h],
        },
        ok: w > 1e-9,
    })
}
#[pyfunction]
pub(super) fn view_rect(camera: &Obj<'_>) -> PyResult<(f64, f64, f64, f64)> {
    let proj = Projection::new(camera)?;
    if let Camera::Perspective { scale: [x, y], .. } = proj.cam {
        return Ok((-x, -y, x, y));
    }
    let inverse: [[f64; 4]; 4] = camera
        .py()
        .import("numpy.linalg")?
        .call_method1("inv", (view_matrix(camera)?,))?
        .extract()?;
    let corner = |s: f64| {
        let w = inverse.map(|r| r[0] * s + r[1] * s + r[3]);
        proj.xy([w[0] / w[3], w[1] / w[3], w[2] / w[3]]).unwrap()
    };
    let [a, b] = corner(-1.);
    let [c, d] = corner(1.);
    Ok((a, b, c, d))
}
pub(super) fn pins(
    r: &[P],
    normal: P,
    proj: &Projection,
    axes: &Obj<'_>,
) -> PyResult<Option<Vec<((f64, f64), (f64, f64))>>> {
    let c = std::array::from_fn(|i| r.iter().map(|p| p[i]).sum::<f64>() / r.len() as f64);
    // NumPy argmax keeps the first corner at a tied distance.
    let mut a = [0.; 3];
    let mut max = -1.;
    for &p in r {
        let v = sub(p, c);
        let d = dot(v, v);
        if d > max {
            a = v;
            max = d;
        }
    }
    let mut radius = max.sqrt();
    if radius < 1e-12 {
        return Ok(None);
    }
    a = a.map(|v| v / radius);
    let b = unit(cross(normal, a));
    let u = xyz(&axes.get_item(0)?)?;
    let v = xyz(&axes.get_item(2)?)?;
    let cu = axes.get_item(1)?.extract::<f64>()?;
    let cv = axes.get_item(3)?.extract::<f64>()?;
    for _ in 0..12 {
        let mut pins = Vec::new();
        let mut xy = Vec::new();
        for dir in [a, b, a.map(|v| -v), b.map(|v| -v)] {
            let p = std::array::from_fn(|i| c[i] + dir[i] * radius);
            let Some(q) = proj.xy(p) else { break };
            xy.push(q);
            pins.push(((q[0], q[1]), (dot(u, p) + cu, dot(v, p) + cv)));
        }
        if pins.len() == 4 {
            return Ok((signed_area(&xy).abs() > 1e-14).then_some(pins));
        }
        radius *= 0.5;
    }
    Ok(None)
}
