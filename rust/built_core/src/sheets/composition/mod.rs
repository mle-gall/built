// SPDX-License-Identifier: GPL-3.0-or-later
//! Script-visible, journalled sheet records. Dataclass reflection describes the
//! archive schema; the record fields themselves live in the document store.
use crate::sheets::{items, items_b, model};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use pyo3::IntoPyObjectExt;
type Obj<'p> = Bound<'p, PyAny>;
mod camera;
mod dimensions;
mod fields;
mod history;
mod records;

fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_core.composition")
}
fn num(record: &Obj, key: &str) -> PyResult<f64> {
    record.getattr(key)?.extract()
}
fn string(record: &Obj, key: &str) -> PyResult<String> {
    record.getattr(key)?.extract()
}
fn flag(record: &Obj, key: &str) -> PyResult<bool> {
    record.getattr(key)?.is_truthy()
}
fn fields(record: &Obj) -> PyResult<model::Fields> {
    model::read_fields(crate::doc::py::handle_key(record)?)
}
fn tr(py: Python<'_>, key: &str) -> PyResult<String> {
    py.import("built_core.i18n")?
        .call_method1("tr", (key,))?
        .extract()
}
fn method<'p>(f: &Obj<'p>) -> PyResult<Obj<'p>> {
    f.py()
        .import("functools")?
        .call_method1("partialmethod", (f,))
}
macro_rules! methods {
    ($ns:expr, $py:expr; $($key:literal => $f:path),* $(,)?)=>{{
        $( $ns.set_item($key, method(wrap_pyfunction!($f,$py)?.as_any())?)?; )*
    }}
}
macro_rules! properties {
    ($ns:expr, $py:expr; $($key:literal => $f:path),* $(,)?)=>{{
        $( $ns.set_item($key,$py.import("builtins")?.call_method1("property",(wrap_pyfunction!($f,$py)?,))?)?; )*
    }}
}
use {methods, properties};

pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let builtin = py.import("builtins")?;
    let dc = py.import("dataclasses")?;
    let doc = py.import("built_core.doc_model")?;
    let definitions = py
        .import("json")?
        .call_method1("loads", (include_str!("schema.json"),))?
        .cast_into::<PyDict>()?;
    camera::install(m)?;
    fields::install(m)?;
    for f in [
        wrap_pyfunction!(records::sheet_holds, m)?,
        wrap_pyfunction!(records::bind_sheet_anchors, m)?,
        wrap_pyfunction!(records::_migrate_fixed_scale_labels, m)?,
        wrap_pyfunction!(dimensions::cota_line_deg, m)?,
    ] {
        m.add_function(f)?;
    }
    let title_fields = [
        "corner",
        "radius_mm",
        "layout",
        "double_border",
        "fill_color",
        "label_color",
        "text_color",
        "line_color",
        "label_mm",
        "border_mm",
        "line_mm",
    ];
    for (name, fields) in definitions.iter() {
        let name: String = name.extract()?;
        let ns = PyDict::new(py);
        let annotations = PyDict::new(py);
        ns.set_item("__module__", "built_core.composition")?;
        ns.set_item("__annotations__", &annotations)?;
        for entry in fields.try_iter()? {
            let [k, t, v]: [Obj; 3] = entry?.extract()?;
            let k = k.extract::<String>()?;
            let t = t.extract::<String>()?;
            annotations.set_item(&k, t)?;
            let value = if v.is_instance_of::<PyList>() || v.is_instance_of::<PyDict>() {
                let kw = PyDict::new(py);
                kw.set_item("default_factory", v.get_type())?;
                dc.getattr("field")?.call((), Some(&kw))?
            } else {
                v
            };
            ns.set_item(k, value)?;
        }
        records::install_methods(&name, &ns)?;
        dimensions::install_methods(&name, &ns)?;
        if name == "Cajetin" {
            ns.set_item("LOOK_FIELDS", PyTuple::new(py, title_fields)?)?;
            ns.set_item(
                "FIELDS",
                PyTuple::new(
                    py,
                    [
                        ("PROYECTO", "proyecto"),
                        ("AUTOR", "autor"),
                        ("FECHA", "fecha"),
                        ("ESCALA", "escala"),
                        ("LÁMINA", "lamina"),
                    ],
                )?,
            )?;
            let base = PyDict::new(py);
            for f in title_fields {
                base.set_item(f, ns.get_item(f)?.unwrap())?;
            }
            m.add("CAJETIN_DESIGN_BASE", base)?;
            let designs = py
                .import("json")?
                .call_method1("loads", (include_str!("title_designs.json"),))?;
            let rows = designs
                .try_iter()?
                .map(|r| py.get_type::<PyTuple>().call1((r?,)))
                .collect::<PyResult<Vec<_>>>()?;
            m.add("CAJETIN_DESIGNS", PyTuple::new(py, rows)?)?;
        }
        let class = builtin.call_method1("type", (&name, (doc.getattr("Record")?,), &ns))?;
        let kw = PyDict::new(py);
        kw.set_item(
            "eq",
            !matches!(
                name.as_str(),
                "EtiquetaItem" | "NivelItem" | "LlamadaItem" | "CotaAngularItem"
            ),
        )?;
        let class = dc.getattr("dataclass")?.call((class,), Some(&kw))?;
        let extra: &[&str] = match name.as_str() {
            "EtiquetaItem" | "NivelItem" => &["hold_a"],
            "CotaItem" => &["hold_a", "hold_b"],
            _ => &[],
        };
        let kw = PyDict::new(py);
        kw.set_item("extra", PyTuple::new(py, extra)?)?;
        let class = doc
            .getattr("record_fields")?
            .call((), Some(&kw))?
            .call1((class,))?;
        m.add(name, class)?;
    }
    history::install(m)
}
pub(crate) use history::{edit_signature, edits_of};
