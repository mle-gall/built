// SPDX-License-Identifier: GPL-3.0-or-later
//! Read a script-supplied CAD entity through its group-tag writer protocol.
use super::{ascii, Bucket, Obj};
use pyo3::{exceptions::PyValueError, prelude::*};
type Groups = Vec<(i32, String)>;
#[pyclass]
#[derive(Default)]
struct Tags {
    data: Groups,
}
#[pymethods]
impl Tags {
    #[getter]
    fn dxfversion(&self) -> &'static str {
        "AC1032"
    }
    #[getter]
    fn write_handles(&self) -> bool {
        false
    }
    #[getter]
    fn force_optional(&self) -> bool {
        false
    }
    fn write_tag2(&mut self, code: i32, value: &Obj<'_>) -> PyResult<()> {
        self.data.push((code, value.str()?.to_string()));
        Ok(())
    }
    fn write_vertex(&mut self, code: i32, vertex: &Obj<'_>) -> PyResult<()> {
        for (i, v) in vertex.try_iter()?.enumerate() {
            self.write_tag2(code + 10 * i as i32, &v?)?;
        }
        Ok(())
    }
    fn write_tag(&mut self, tag: &Obj<'_>) -> PyResult<()> {
        if let Some(expand) = tag.getattr_opt("dxftags")? {
            for t in expand.call0()?.try_iter()? {
                let t = t?;
                self.write_tag2(t.getattr("code")?.extract()?, &t.getattr("value")?)?;
            }
        } else {
            self.write_tag2(tag.getattr("code")?.extract()?, &tag.getattr("value")?)?;
        }
        Ok(())
    }
    fn write_tags(&mut self, tags: &Obj<'_>) -> PyResult<()> {
        for t in tags.try_iter()? {
            self.write_tag(&t?)?;
        }
        Ok(())
    }
    fn write_str(&mut self, s: &str) -> PyResult<()> {
        let mut lines = s.lines();
        while let Some(k) = lines.next() {
            self.data.push((
                k.trim()
                    .parse()
                    .map_err(|_| PyValueError::new_err("invalid DXF group code"))?,
                lines
                    .next()
                    .ok_or_else(|| PyValueError::new_err("missing DXF group value"))?
                    .to_owned(),
            ));
        }
        Ok(())
    }
}
fn groups(e: &Obj<'_>) -> PyResult<Groups> {
    let sink = Py::new(e.py(), Tags::default())?.into_bound(e.py());
    e.call_method1("export_dxf", (&sink,))?;
    let mut writer = sink.borrow_mut();
    Ok(std::mem::take(&mut writer.data))
}
pub(super) fn geometry(e: &Obj<'_>, kind: &str, tol: f64) -> PyResult<Bucket> {
    let header = groups(e)?;
    let vertices = if kind == "POLYLINE" {
        e.getattr("vertices")?
            .try_iter()?
            .map(|v| groups(&v?))
            .collect::<PyResult<Vec<_>>>()?
    } else {
        vec![]
    };
    ascii::from_object(kind, &header, &vertices, tol)
        .ok_or_else(|| PyValueError::new_err("invalid CAD entity"))
}
