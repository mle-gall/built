// SPDX-License-Identifier: GPL-3.0-or-later
//! Walk visible, painted placements one mesh at a time, including camera-facing figures.
use super::*;
use crate::math::{xyz, Matrix4, Vec3};
use glam::{DMat4, DVec3};
use pyo3::types::PyIterator;

fn bounds(mesh: &Obj<'_>) -> PyResult<Option<(DVec3, DVec3)>> {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    for v in mesh.getattr("vertices")?.try_iter()? {
        let p = xyz(&v?.getattr("position")?)?;
        lo = lo.min(p);
        hi = hi.max(p)
    }
    Ok((lo.x <= hi.x).then_some((lo, hi)))
}
fn anchor(lo: DVec3, hi: DVec3) -> DVec3 {
    DVec3::new((lo.x + hi.x) / 2., (lo.y + hi.y) / 2., lo.z)
}
#[pyfunction]
fn _feet(mesh: &Obj<'_>) -> PyResult<Option<Vec3>> {
    Ok(bounds(mesh)?.map(|(lo, hi)| Vec3::from(anchor(lo, hi))))
}
#[pyfunction]
fn _turned_toward<'p>(mesh: &Obj<'p>, toward: &Obj<'p>) -> PyResult<Obj<'p>> {
    let mut big = None;
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        let area = f.call_method0("area")?.extract::<f64>()?;
        if big.as_ref().is_none_or(|(a, _)| area > *a) {
            big = Some((area, f))
        }
    }
    let (Some((_, f)), Some((lo, hi))) = (big, bounds(mesh)?) else {
        return Ok(mesh.clone());
    };
    let n = xyz(&f.call_method0("normal")?)?.truncate();
    if n.length() < 1e-9 {
        return Ok(mesh.clone());
    }
    let a = anchor(lo, hi);
    let d = xyz(&toward.call1((Vec3::from(a),))?)?.truncate();
    if d.length() < 1e-9 {
        return Ok(mesh.clone());
    }
    let n = n.normalize();
    let d = d.normalize();
    let yaw = n.perp_dot(d).atan2(n.dot(d));
    let matrix =
        DMat4::from_translation(a) * DMat4::from_rotation_z(yaw) * DMat4::from_translation(-a);
    mesh.py()
        .import("core.group")?
        .call_method1("transformed_mesh", (mesh, Matrix4::from(matrix)))
}
#[pyfunction]
fn _facing_quad<'p>(mesh: &Obj<'p>, toward: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    let mut tex = None;
    for f in mesh.getattr("faces")?.try_iter()? {
        let texture = get(&f?.getattr("attrs")?, "texture")?;
        if texture.is_truthy()? && get(&texture, "path")?.is_truthy()? {
            tex = Some(texture.cast::<PyDict>()?.copy()?);
            break;
        }
    }
    let (Some(tex), Some((lo, hi))) = (tex, bounds(mesh)?) else {
        return _turned_toward(mesh, toward);
    };
    let a = anchor(lo, hi);
    let size = hi - lo;
    let width = size.truncate().length();
    let d = xyz(&toward.call1((Vec3::from(a),))?)?.truncate();
    if width < 1e-9 || size.z < 1e-9 || d.length() < 1e-6 {
        return Ok(mesh.clone());
    }
    let right = d.normalize().perp().extend(0.) * width * 0.5;
    let up = DVec3::Z * size.z;
    let pts =
        crate::file_model::verts(py, &[a - right, a + right, a + right + up, a - right + up])?;
    let uv = [(0., 0.), (1., 0.), (1., 1.), (0., 1.)];
    tex.set_item(
        "uvw",
        crate::model::module(py, "ingetrazo_core")?
            .call_method1("texture_fit_uv_affine", (&pts, uv.to_vec()))?,
    )?;
    let out = crate::file_model::new_mesh(py)?;
    let attrs = out.call_method1("add_face", (pts,))?.getattr("attrs")?;
    attrs.call_method1(
        "update",
        (mesh.getattr("faces")?.get_item(0)?.getattr("attrs")?,),
    )?;
    attrs.set_item("texture", tex)?;
    Ok(out)
}
#[pyfunction]
fn _shown<'p>(face: &Obj<'p>, paint: &Obj<'p>) -> PyResult<Obj<'p>> {
    let attrs = face
        .py()
        .import("core.materials")?
        .call_method1("effective_attrs", (face.getattr("attrs")?, paint))?;
    Ok(if attrs.is_truthy()? {
        attrs
    } else {
        PyDict::new(face.py()).into_any()
    })
}
fn billboard<'p>(group: &Obj<'p>) -> PyResult<Obj<'p>> {
    attr(group, "billboard")
}
fn has_billboard(group: &Obj<'_>) -> PyResult<bool> {
    for pair in group
        .py()
        .import("core.group")?
        .call_method1("iter_placements", (group,))?
        .try_iter()?
    {
        if billboard(&pair?.get_item(0)?)?.is_truthy()? {
            return Ok(true);
        }
    }
    Ok(false)
}
fn placed_mesh<'p>(group: &Obj<'p>, matrix: &Obj<'p>) -> PyResult<Obj<'p>> {
    let mesh = group.getattr("mesh")?;
    if matrix.is_none() {
        Ok(mesh)
    } else {
        group
            .py()
            .import("core.group")?
            .call_method1("transformed_mesh", (mesh, matrix))
    }
}
fn turned<'p>(mesh: &Obj<'p>, kind: &Obj<'p>, toward: &Obj<'p>) -> PyResult<Obj<'p>> {
    if kind.eq("mesh")? {
        _turned_toward(mesh, toward)
    } else if kind.is_truthy()? {
        _facing_quad(mesh, toward)
    } else {
        Ok(mesh.clone())
    }
}
#[pyfunction]
pub fn faceme_figures<'p>(scene: &Obj<'p>, toward: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = scene.py();
    let out = PyList::empty(py);
    let groups = attr(scene, "groups")?;
    if groups.is_none() {
        return Ok(out);
    }
    for g in groups.try_iter()? {
        let g = g?;
        if !scene.call_method1("entity_visible", (&g,))?.is_truthy()? || !has_billboard(&g)? {
            continue;
        }
        for p in py
            .import("core.group")?
            .call_method1("iter_placements", (&g,))?
            .try_iter()?
        {
            let p = p?;
            let item = p.get_item(0)?;
            let kind = billboard(&item)?;
            if !kind.is_truthy()? {
                continue;
            }
            let mesh = placed_mesh(&item, &p.get_item(1)?)?;
            if let Some(feet) = _feet(&mesh)? {
                let feet = Py::new(py, feet)?;
                let d = xyz(&toward.call1((&feet,))?)?;
                let faces = turned(&mesh, &kind, toward)?
                    .getattr("faces")?
                    .try_iter()?
                    .collect::<PyResult<Vec<_>>>()?;
                out.append((faces, feet, d.y.atan2(d.x)))?;
            }
        }
    }
    Ok(out)
}
#[pyclass]
pub struct Faces {
    scene: Py<PyAny>,
    faces: Option<Py<PyIterator>>,
    groups: Option<Py<PyIterator>>,
    pieces: Option<Py<PyIterator>>,
    paint: Option<Py<PyAny>>,
    camera: Option<Py<PyAny>>,
    figures: bool,
    orient: bool,
    loose: bool,
    paired: bool,
}
impl Faces {
    fn new(
        scene: &Obj<'_>,
        camera: Option<&Obj<'_>>,
        figures: bool,
        paired: bool,
    ) -> PyResult<Self> {
        let loose = scene.hasattr("render_faces")?;
        let mesh = if loose {
            scene.getattr("loose_mesh")?
        } else if scene.hasattr("mesh")? {
            scene.getattr("mesh")?
        } else {
            scene.clone()
        };
        let groups = if loose {
            let g = attr(scene, "groups")?;
            if g.is_none() {
                None
            } else {
                Some(g.try_iter()?.unbind())
            }
        } else {
            None
        };
        Ok(Self {
            scene: scene.clone().unbind(),
            faces: Some(mesh.getattr("faces")?.try_iter()?.unbind()),
            groups,
            pieces: None,
            paint: None,
            camera: camera.map(|c| c.clone().unbind()),
            figures,
            orient: false,
            loose,
            paired,
        })
    }
    fn start(&mut self, mesh: &Obj<'_>, paint: Obj<'_>) -> PyResult<()> {
        self.faces = Some(mesh.getattr("faces")?.try_iter()?.unbind());
        self.paint = Some(paint.unbind());
        Ok(())
    }
    fn next_item<'p>(&mut self, py: Python<'p>) -> PyResult<Option<Obj<'p>>> {
        loop {
            if let Some(f) = step(py, &self.faces)? {
                if self.loose
                    && !self
                        .scene
                        .bind(py)
                        .call_method1("entity_visible", (&f,))?
                        .is_truthy()?
                {
                    continue;
                }
                if !self.paired {
                    return Ok(Some(f));
                }
                let attrs = if let Some(p) = &self.paint {
                    _shown(&f, p.bind(py))?
                } else {
                    f.getattr("attrs")?
                };
                return Ok(Some((f, attrs).into_pyobject(py)?.into_any()));
            }
            self.faces = None;
            self.loose = false;
            if let Some(p) = step(py, &self.pieces)? {
                let g = p.get_item(0)?;
                let kind = billboard(&g)?;
                if self.orient && kind.is_truthy()? && !self.figures {
                    continue;
                }
                let mut mesh = placed_mesh(&g, &p.get_item(1)?)?;
                if self.orient {
                    mesh = turned(&mesh, &kind, self.camera.as_ref().unwrap().bind(py))?;
                }
                self.start(&mesh, p.get_item(2)?)?;
                continue;
            }
            self.pieces = None;
            let Some(group) = step(py, &self.groups)? else {
                return Ok(None);
            };
            if !self
                .scene
                .bind(py)
                .call_method1("entity_visible", (&group,))?
                .is_truthy()?
            {
                continue;
            }
            let api = py.import("core.group")?;
            self.orient = self.camera.is_some() && has_billboard(&group)?;
            if self.orient {
                self.pieces = Some(
                    api.call_method1("iter_painted_placements", (&group,))?
                        .try_iter()?
                        .unbind(),
                );
                continue;
            }
            if self.camera.is_none() && billboard(&group)?.is_truthy()? {
                continue;
            }
            let pieces = api
                .call_method1("iter_painted_placements", (&group,))?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?;
            let mut painted = false;
            for p in pieces.iter().skip(1) {
                if attr(&p.get_item(0)?, "material")?.is_truthy()? {
                    painted = true;
                    break;
                }
            }
            if painted {
                self.pieces = Some(PyList::new(py, pieces)?.try_iter()?.unbind())
            } else {
                self.start(
                    &api.call_method1("world_mesh", (&group,))?,
                    attr(&group, "material")?,
                )?;
            }
        }
    }
}
fn step<'p>(py: Python<'p>, it: &Option<Py<PyIterator>>) -> PyResult<Option<Obj<'p>>> {
    it.as_ref()
        .and_then(|it| it.bind(py).clone().next())
        .transpose()
}
#[pymethods]
impl Faces {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'p>(&mut self, py: Python<'p>) -> PyResult<Option<Obj<'p>>> {
        self.next_item(py)
    }
}
#[pyfunction]
#[pyo3(signature=(scene,face_me=None,figures=true))]
pub fn world_faces_painted(
    scene: &Obj<'_>,
    face_me: Option<&Obj<'_>>,
    figures: bool,
) -> PyResult<Faces> {
    Faces::new(scene, face_me, figures, true)
}
#[pyfunction]
#[pyo3(signature=(scene,face_me=None,figures=true))]
fn world_faces(scene: &Obj<'_>, face_me: Option<&Obj<'_>>, figures: bool) -> PyResult<Faces> {
    Faces::new(scene, face_me, figures, false)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(_feet, m)?)?;
    m.add_function(wrap_pyfunction!(_turned_toward, m)?)?;
    m.add_function(wrap_pyfunction!(_facing_quad, m)?)?;
    m.add_function(wrap_pyfunction!(_shown, m)?)?;
    m.add_function(wrap_pyfunction!(faceme_figures, m)?)?;
    m.add_function(wrap_pyfunction!(world_faces, m)?)?;
    m.add_function(wrap_pyfunction!(world_faces_painted, m)?)
}
