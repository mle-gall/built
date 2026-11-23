// SPDX-License-Identifier: GPL-3.0-or-later
//! A named material affects face dictionaries, the document registry and
//! placement paint. Capture direct-command inverses before the first write.
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::collections::HashSet;

const FIELDS: [&str; 5] = ["mat", "color", "texture", "opacity", "pbr"];
type Obj = Py<PyAny>;
type Row = (Obj, Obj);

#[derive(PartialEq)]
enum Edit {
    Stamp,
    Texture,
    Delete,
}

#[pyclass(module = "ingetrazo_core", name = "MaterialRestamp", frozen)]
pub(super) struct Restamp {
    kind: Edit,
    captured: bool,
    name: String,
    destination: String,
    original: Obj,
    previous_target: Obj,
    fronts: Vec<Row>,
    backs: Vec<Row>,
    groups: Vec<Row>,
    meshes: Vec<Obj>,
}

fn get<'py>(mapping: &Bound<'py, PyAny>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    mapping.call_method1("get", (key,))
}
fn put(mapping: &Bound<'_, PyAny>, key: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
    if value.is_none() {
        mapping.call_method1("pop", (key, mapping.py().None()))?;
    } else {
        mapping.set_item(key, value)?;
    }
    Ok(())
}

#[pymethods]
impl Restamp {
    #[new]
    fn new(
        loose: &Bound<'_, PyAny>,
        placements: &Bound<'_, PyAny>,
        registry: &Bound<'_, PyAny>,
        name: String,
        destination: String,
        kind: &str,
        capture: bool,
    ) -> PyResult<Self> {
        let py = loose.py();
        let kind = match kind {
            "stamp" => Edit::Stamp,
            "texture" => Edit::Texture,
            "delete" => Edit::Delete,
            _ => return Err(PyValueError::new_err("unknown material edit")),
        };
        let original = get(registry, &name)?.unbind();
        let previous_target = get(registry, &destination)?.unbind();
        let mut meshes = vec![loose.clone()];
        let mut seen = HashSet::new();
        let mut groups = Vec::new();
        for placement in placements.try_iter()? {
            let g = placement?;
            let mesh = g.getattr("mesh")?;
            if seen.insert(mesh.as_ptr()) {
                meshes.push(mesh);
            }
            let Some(paint) = g.getattr_opt("material")? else {
                continue;
            };
            if paint.is_instance_of::<PyDict>() && get(&paint, "mat")?.eq(&name)? {
                groups.push((g.unbind(), paint.unbind()));
            }
        }
        let mut fronts = Vec::new();
        let mut backs = Vec::new();
        let mut affected = Vec::new();
        seen.clear();
        for mesh in meshes {
            if mesh.is_none() {
                continue;
            }
            let mut worn = false;
            for face in mesh.getattr("faces")?.try_iter()? {
                let f = face?;
                let attrs = f.getattr("attrs")?;
                if kind != Edit::Texture {
                    let back = get(&attrs, "back")?;
                    if back.is_instance_of::<PyDict>() && get(&back, "mat")?.eq(&name)? {
                        backs.push((f.clone().unbind(), back.unbind()));
                        worn = true;
                    }
                }
                if get(&attrs, "mat")?.eq(&name)? {
                    let old = if capture {
                        let row = PyDict::new(py);
                        for key in FIELDS {
                            row.set_item(key, get(&attrs, key)?)?;
                        }
                        row.into_any()
                    } else if kind == Edit::Texture {
                        let row = PyDict::new(py);
                        row.set_item("texture", get(&attrs, "texture")?)?;
                        row.into_any()
                    } else {
                        py.None().into_bound(py)
                    };
                    fronts.push((f.unbind(), old.unbind()));
                    worn = true;
                }
            }
            if worn && seen.insert(mesh.as_ptr()) {
                affected.push(mesh.unbind());
            }
        }
        Ok(Self {
            kind,
            captured: capture,
            name,
            destination,
            original,
            previous_target,
            fronts,
            backs,
            groups,
            meshes: affected,
        })
    }

    #[getter]
    fn meshes(&self, py: Python<'_>) -> Vec<Obj> {
        self.meshes.iter().map(|x| x.clone_ref(py)).collect()
    }

    #[pyo3(signature=(registry, replacement, edit_texture=None))]
    fn apply(
        &self,
        registry: &Bound<'_, PyAny>,
        replacement: &Bound<'_, PyAny>,
        edit_texture: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let py = registry.py();
        let none = py.None().into_bound(py);
        let stamp = if self.kind == Edit::Stamp {
            replacement.call_method0("face_attrs")?
        } else {
            PyDict::new(py).into_any()
        };
        if self.kind == Edit::Delete || self.destination != self.name {
            registry.call_method1("pop", (&self.name, &none))?;
        }
        if self.kind != Edit::Delete {
            registry.set_item(&self.destination, replacement)?;
        }
        if self.kind == Edit::Texture {
            let edit =
                edit_texture.ok_or_else(|| PyValueError::new_err("missing texture editor"))?;
            for (f, old) in &self.fronts {
                let tex = old.bind(py).get_item("texture")?;
                if tex.is_truthy()? {
                    f.bind(py)
                        .getattr("attrs")?
                        .set_item("texture", edit.call1((tex,))?)?;
                }
            }
            for (g, old) in &self.groups {
                let tex = get(old.bind(py), "texture")?;
                if tex.is_truthy()? {
                    let paint = py.get_type::<PyDict>().call1((old,))?;
                    paint.set_item("texture", edit.call1((tex,))?)?;
                    g.bind(py).setattr("material", paint)?;
                }
            }
            return Ok(());
        }

        let original = self.original.bind(py);
        let keep_mapping = self.kind == Edit::Stamp
            && !original.is_none()
            && original
                .getattr("texture")?
                .eq(replacement.getattr("texture")?)?;
        let write = |attrs: &Bound<'_, PyAny>| -> PyResult<()> {
            for key in FIELDS {
                if key == "texture" && keep_mapping {
                    continue;
                }
                if stamp.contains(key)? {
                    attrs.set_item(key, stamp.get_item(key)?)?;
                } else {
                    attrs.call_method1("pop", (key, &none))?;
                }
            }
            Ok(())
        };
        for (face, _) in &self.fronts {
            write(&face.bind(py).getattr("attrs")?)?;
        }
        for (face, old) in &self.backs {
            let back = py.get_type::<PyDict>().call1((old,))?;
            write(&back)?;
            let attrs = face.bind(py).getattr("attrs")?;
            if self.kind == Edit::Delete && back.is_empty()? {
                put(&attrs, "back", &none)?;
            } else {
                attrs.set_item("back", back)?;
            }
        }
        for (g, _) in &self.groups {
            let paint = if self.kind == Edit::Delete {
                none.clone()
            } else {
                py.get_type::<PyDict>().call1((&stamp,))?
            };
            g.bind(py).setattr("material", paint)?;
        }
        Ok(())
    }

    fn restore(&self, registry: &Bound<'_, PyAny>) -> PyResult<()> {
        if !self.captured {
            return Err(PyValueError::new_err(
                "restamp is covered by a document transaction",
            ));
        }
        let py = registry.py();
        if self.name != self.destination {
            put(registry, &self.destination, self.previous_target.bind(py))?;
        }
        put(registry, &self.name, self.original.bind(py))?;
        for (face, old) in &self.fronts {
            let attrs = face.bind(py).getattr("attrs")?;
            for key in FIELDS {
                put(&attrs, key, &old.bind(py).get_item(key)?)?;
            }
        }
        for (face, old) in &self.backs {
            face.bind(py).getattr("attrs")?.set_item("back", old)?;
        }
        for (group, old) in &self.groups {
            group.bind(py).setattr("material", old)?;
        }
        Ok(())
    }
}
