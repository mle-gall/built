// SPDX-License-Identifier: GPL-3.0-or-later
//! Paint edits use the live per-face attribute dictionaries enlisted by the
//! geometry journal. Direct command execution saves only the edited fields.
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

type PaintRows = Vec<Vec<Py<PyAny>>>;

fn write(attrs: &Bound<'_, PyAny>, field: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = attrs.py();
    if value.is_none() {
        attrs.call_method1("pop", (field, py.None()))?;
    } else {
        let value = match field {
            "color" => py.get_type::<PyList>().call1((value,))?,
            "texture" => py.get_type::<PyDict>().call1((value,))?,
            "back" if value.is_instance_of::<PyDict>() => {
                py.get_type::<PyDict>().call1((value,))?
            }
            _ => value.clone(),
        };
        attrs.set_item(field, value)?;
    }
    Ok(())
}

#[pyfunction]
pub(super) fn apply_face_paint(
    py: Python<'_>,
    faces: Vec<Py<PyAny>>,
    fields: Vec<(String, Py<PyAny>)>,
    capture: bool,
) -> PyResult<Option<PaintRows>> {
    // A selection may name one face twice or faces may share the same dict.
    // Read every original field before the first write.
    let before = if capture {
        Some(
            faces
                .iter()
                .map(|f| {
                    let a = f.bind(py).getattr("attrs")?;
                    fields
                        .iter()
                        .map(|(key, _)| Ok(a.call_method1("get", (key,))?.unbind()))
                        .collect()
                })
                .collect::<PyResult<PaintRows>>()?,
        )
    } else {
        None
    };
    for (i, face) in faces.iter().enumerate() {
        for (field, value) in &fields {
            let source = value.bind(py);
            let value = match field.as_str() {
                "back" if i >= source.len()? => continue,
                "back" => source.get_item(i)?,
                // A material can expose the recipe as a document-backed property.
                "pbr" if !source.is_none() => source.getattr("pbr")?,
                _ => source.clone(),
            };
            write(&face.bind(py).getattr("attrs")?, field, &value)?;
        }
    }
    Ok(before)
}

#[pyfunction]
pub(super) fn restore_face_paint(
    py: Python<'_>,
    faces: Vec<Py<PyAny>>,
    fields: Vec<String>,
    rows: PaintRows,
) -> PyResult<()> {
    for (face, row) in faces.iter().zip(rows) {
        let a = face.bind(py).getattr("attrs")?;
        for (key, value) in fields.iter().zip(row) {
            write(&a, key, value.bind(py))?;
        }
    }
    Ok(())
}
