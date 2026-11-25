// SPDX-License-Identifier: GPL-3.0-or-later
//! Convert a paint stroke into undoable attribute changes.
use crate::math::xyz;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
#[pyfunction]
fn paint_face_plane<'p>(face: &Obj<'p>) -> PyResult<(Obj<'p>, f64)> {
    let n = face.call_method0("normal")?;
    let vertices = face.getattr("vertices")?;
    let offset = if vertices.len()? == 0 {
        0.
    } else {
        xyz(&n)?.dot(xyz(&vertices.get_item(0)?)?)
    };
    Ok((n, offset))
}
fn same_plane(face: &Obj, plane: &Obj) -> PyResult<bool> {
    let n = xyz(&plane.get_item(0)?)?;
    if (n.dot(xyz(&face.call_method0("normal")?)?).abs() - 1.).abs() > 1e-3 {
        return Ok(false);
    }
    let vertices = face.getattr("vertices")?;
    let d = if vertices.len()? == 0 {
        0.
    } else {
        n.dot(xyz(&vertices.get_item(0)?)?)
    };
    Ok((d - plane.get_item(1)?.extract::<f64>()?).abs() <= 1e-4)
}
fn flattened<'p>(texture: &Obj<'p>, plane: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    texture.py().import("built_core")?.call_method1(
        "texture_flattened",
        (texture, plane.map(|p| p.get_item(0)).transpose()?),
    )
}
#[pyfunction]
fn texture_commands<'p>(
    faces: &Obj<'p>,
    tex: &Obj<'p>,
    plane: Option<&Obj<'p>>,
) -> PyResult<Vec<Obj<'p>>> {
    let py = faces.py();
    let set = py
        .import("built_core.history")?
        .getattr("SetFaceTextureCommand")?;
    let Some(plane) = plane.filter(|p| !p.is_none()) else {
        return Ok(vec![set.call1((faces, tex))?]);
    };
    if !tex.call_method1("get", ("uvw",))?.is_truthy()? {
        return Ok(vec![set.call1((faces, tex))?]);
    }
    let same = PyList::empty(py);
    let other = PyList::empty(py);
    for face in faces.try_iter()? {
        let face = face?;
        if same_plane(&face, plane)? {
            same.append(face)?
        } else {
            other.append(face)?
        }
    }
    let mut cmds = Vec::new();
    if !same.is_empty() {
        cmds.push(set.call1((same, tex))?);
    }
    if !other.is_empty() {
        cmds.push(set.call1((other, flattened(tex, Some(plane))?))?);
    }
    Ok(cmds)
}
#[pyfunction]
fn surface_commands<'p>(
    mesh: &Obj<'p>,
    faces: &Obj<'p>,
    seed: &Obj<'p>,
    tex: &Obj<'p>,
    plane: Option<&Obj<'p>>,
) -> PyResult<Option<Vec<Obj<'p>>>> {
    if mesh.is_none() || seed.is_none() || faces.len()? < 2 {
        return Ok(None);
    }
    let py = mesh.py();
    let look = if tex.call_method1("get", ("uvw",))?.is_truthy()?
        && !plane
            .map(|p| same_plane(seed, p))
            .transpose()?
            .unwrap_or(false)
    {
        flattened(tex, plane)?
    } else {
        tex.clone()
    };
    let maps = py
        .import("built_core")?
        .call_method1("texture_continuous_maps", (mesh, faces, seed, look))?;
    if maps.len()? < 2 {
        return Ok(None);
    }
    let mut cmds = Vec::new();
    let rest = PyList::empty(py);
    let set = py
        .import("built_core.history")?
        .getattr("SetFaceTextureCommand")?;
    for face in faces.try_iter()? {
        let f = face?;
        let key = f.getattr("key")?;
        if maps.contains(&key)? {
            cmds.push(set.call1((PyList::new(py, [f])?, maps.get_item(key)?))?);
        } else {
            rest.append(f)?;
        }
    }
    if !rest.is_empty() {
        cmds.extend(texture_commands(rest.as_any(), tex, plane)?);
    }
    Ok(Some(cmds))
}
fn list<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    v.py().import("builtins")?.call_method1("list", (v,))
}
fn material_color<'p>(m: &Obj<'p>) -> PyResult<Obj<'p>> {
    let c = m.getattr("color")?;
    if c.is_truthy()? {
        Ok(c)
    } else {
        Ok((1., 1., 1.).into_pyobject(m.py())?.into_any())
    }
}
#[pyfunction(signature=(color,texture,opacity,material,plane=None,face=None))]
fn paint_stamp<'p>(
    color: &Obj<'p>,
    texture: &Obj<'p>,
    opacity: &Obj<'p>,
    material: &Obj<'p>,
    plane: Option<&Obj<'p>>,
    face: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyDict>> {
    let py = color.py();
    let stamp = PyDict::new(py);
    if texture.is_none() {
        stamp.set_item("color", list(color)?)?;
    } else {
        let mut tex = py.import("builtins")?.call_method1("dict", (texture,))?;
        if let Some(f) = face {
            if tex.call_method1("get", ("uvw",))?.is_truthy()?
                && !plane
                    .map(|p| same_plane(f, p))
                    .transpose()?
                    .unwrap_or(false)
            {
                tex = flattened(&tex, plane)?;
            }
        } else {
            tex.call_method1("pop", ("uvw", py.None()))?;
        }
        stamp.set_item("texture", tex)?;
    }
    if !opacity.is_none() {
        stamp.set_item("opacity", opacity.extract::<f64>()?)?;
    }
    if !material.is_none() {
        stamp.set_item("mat", material.getattr("name")?)?;
        let recipe = material.getattr("pbr")?;
        if !recipe.is_none() {
            stamp.set_item("pbr", recipe)?;
            if texture.is_truthy()? {
                stamp.set_item("color", list(&material_color(material)?)?)?;
            }
        }
    }
    Ok(stamp)
}
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn build_paint<'p>(
    mesh: &Obj<'p>,
    faces: &Obj<'p>,
    seed: &Obj<'p>,
    group: &Obj<'p>,
    back: bool,
    default: bool,
    color: &Obj<'p>,
    texture: &Obj<'p>,
    opacity: &Obj<'p>,
    material: &Obj<'p>,
    plane: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    let history = py.import("built_core.history")?;
    let none = py.None().into_bound(py);
    let name = if material.is_none() {
        none.clone()
    } else {
        material.getattr("name")?
    };
    let register = || {
        history.call_method1(
            "SetFaceMaterialTagCommand",
            (PyList::empty(py), &name, material),
        )
    };
    let compound = |cmds: Vec<Obj<'p>>| history.call_method1("CompoundCommand", (cmds,));
    if !group.is_none() {
        let stamp = if default {
            none
        } else {
            paint_stamp(color, texture, opacity, material, None, None)?.into_any()
        };
        return compound(vec![
            register()?,
            py.import("built_core")?
                .getattr("NativeCommand")?
                .call_method1("paint", (PyList::new(py, [group])?, stamp))?,
        ]);
    }
    if back {
        if default {
            return history.call_method1("SetFaceBackCommand", (faces, none));
        }
        let stamps = PyList::empty(py);
        for f in faces.try_iter()? {
            stamps.append(paint_stamp(
                color,
                texture,
                opacity,
                material,
                plane,
                Some(&f?),
            )?)?;
        }
        return compound(vec![
            register()?,
            history.call_method1("SetFaceBackCommand", (faces, stamps))?,
        ]);
    }
    if default {
        return compound(vec![
            history.call_method1("SetFaceColorCommand", (faces, &none))?,
            history.call_method1("SetFaceTextureCommand", (faces, &none))?,
            history.call_method1("SetFaceOpacityCommand", (faces, &none))?,
            history.call_method1("SetFaceMaterialTagCommand", (faces, &none, &none))?,
        ]);
    }
    let mut cmds = if texture.is_none() {
        vec![
            history.call_method1("SetFaceColorCommand", (faces, color))?,
            history.call_method1("SetFaceTextureCommand", (faces, &none))?,
        ]
    } else {
        let mut c = match surface_commands(mesh, faces, seed, texture, plane)? {
            Some(c) => c,
            None => texture_commands(faces, texture, plane)?,
        };
        if !material.is_none() && !material.getattr("pbr")?.is_none() {
            c.push(
                history.call_method1("SetFaceColorCommand", (faces, material_color(material)?))?,
            );
        }
        c
    };
    cmds.push(history.call_method1("SetFaceOpacityCommand", (faces, opacity))?);
    cmds.push(history.call_method1("SetFaceMaterialTagCommand", (faces, name, material))?);
    compound(cmds)
}
pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(paint_face_plane, m)?,
        wrap_pyfunction!(texture_commands, m)?,
        wrap_pyfunction!(surface_commands, m)?,
        wrap_pyfunction!(paint_stamp, m)?,
        wrap_pyfunction!(build_paint, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
