// SPDX-License-Identifier: GPL-3.0-or-later
//! Intern the attribute rows used by plane rebuild and cap extrusion.
use super::Obj;
use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
fn frozen<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let tag;
    let values = PyList::empty(py);
    if let Ok(d) = v.cast::<PyDict>() {
        tag = "d";
        for (key, val) in d.iter() {
            values.append((key, frozen(&val)?))?;
        }
        values.sort()?;
    } else if v.is_instance_of::<PyList>() || v.is_instance_of::<PyTuple>() {
        tag = if v.is_instance_of::<PyList>() {
            "l"
        } else {
            "t"
        };
        for val in v.try_iter()? {
            values.append(frozen(&val?)?)?;
        }
    } else {
        v.hash()?;
        return Ok(v.clone());
    }
    Ok((tag, values.to_tuple()).into_pyobject(py)?.into_any())
}
struct Classes<'py> {
    index: Bound<'py, PyDict>,
    rows: Bound<'py, PyList>,
}
impl<'py> Classes<'py> {
    fn intern(&self, row: &Obj<'py>) -> PyResult<usize> {
        if !row.is_truthy()? {
            return Ok(0);
        }
        let n = self.rows.len();
        match frozen(row) {
            Ok(key) => {
                if let Some(found) = self.index.get_item(&key)? {
                    return found.extract();
                }
                self.index.set_item(key, n)?;
            }
            Err(e) if e.is_instance_of::<PyTypeError>(row.py()) => {
                for (i, v) in self.rows.iter().enumerate().skip(1) {
                    if v.eq(row)? {
                        return Ok(i);
                    }
                }
            }
            Err(e) => return Err(e),
        }
        self.rows.append(row)?;
        Ok(n)
    }
}
#[pyfunction]
#[pyo3(signature=(mesh,*,push=false))]
pub(super) fn paint_classes<'py>(mesh: &Obj<'py>, push: bool) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let owner = mesh
        .getattr("__dict__")?
        .call_method1("get", ("_instance_definition", mesh))?
        .getattr("__dict__")?;
    let rows = PyList::empty(py);
    rows.append(PyDict::new(py))?;
    let classes = Classes {
        index: PyDict::new(py),
        rows,
    };
    let mut table = vec![];
    for r in owner.get_item("_attr_table")?.try_iter()? {
        table.push(classes.intern(&r?)?);
    }
    let own = PyDict::new(py);
    for (k, v) in owner.get_item("_attr_own")?.cast::<PyDict>()?.iter() {
        own.set_item(k, classes.intern(&v)?)?;
    }
    if !push {
        return Ok((own, table, classes.rows).into_pyobject(py)?.into_any());
    }
    let mut swapped = vec![];
    for _ in 0..2 {
        for i in swapped.len()..classes.rows.len() {
            let v = classes.rows.get_item(i)?;
            let copy = py.get_type::<PyDict>().call1((v,))?;
            if copy
                .call_method1("get", ("back",))?
                .is_instance_of::<PyDict>()
            {
                super::geometry::exchange_paint(&copy)?;
            }
            swapped.push(classes.intern(&copy)?);
        }
    }
    let paint_keys = py.import("built_core.mesh")?.getattr("PAINT_KEYS")?;
    let mut surfaces = vec![];
    for i in 0..classes.rows.len() {
        let v = classes.rows.get_item(i)?;
        let surface = PyDict::new(py);
        for key in paint_keys.try_iter()? {
            let key = key?;
            if v.contains(&key)? {
                surface.set_item(&key, v.get_item(&key)?)?;
            }
        }
        surfaces.push(classes.intern(surface.as_any())?);
    }
    swapped.extend(swapped.len()..classes.rows.len());
    surfaces.extend(surfaces.len()..classes.rows.len());
    Ok((own, table, classes.rows, swapped, surfaces)
        .into_pyobject(py)?
        .into_any())
}
