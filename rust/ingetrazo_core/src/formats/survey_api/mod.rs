// SPDX-License-Identifier: GPL-3.0-or-later
//! Public survey record schemas and method bindings. The import path of a record
//! is also its pickle identity, so register each module under its public name.
use super::Obj;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

mod providers;
mod records;

struct Api<'p> {
    core: Bound<'p, PyModule>,
    module: Bound<'p, PyModule>,
}
impl<'p> Api<'p> {
    fn py(&self) -> Python<'p> {
        self.module.py()
    }
    fn exports(&self, names: &[(&str, &str)]) -> PyResult<()> {
        for (name, implementation) in names {
            self.module
                .add(*name, self.core.getattr(*implementation)?)?;
        }
        Ok(())
    }
    fn functions(&self, names: &[&str]) -> PyResult<()> {
        for name in names {
            self.module.add(*name, self.core.getattr(*name)?)?;
        }
        Ok(())
    }
    fn class(
        &self,
        name: &str,
        base: Option<Obj<'p>>,
        members: &Bound<'p, PyDict>,
    ) -> PyResult<Obj<'p>> {
        let py = self.py();
        members.set_item("__module__", self.module.name()?)?;
        let cls = py.import("builtins")?.call_method1(
            "type",
            (
                name,
                PyTuple::new(py, base.into_iter().collect::<Vec<_>>())?,
                members,
            ),
        )?;
        self.module.add(name, &cls)?;
        Ok(cls)
    }
    fn methods(
        &self,
        methods: &[(&str, &str)],
        properties: &[(&str, &str)],
    ) -> PyResult<Bound<'p, PyDict>> {
        let py = self.py();
        let ns = PyDict::new(py);
        let method = py.import("functools")?.getattr("partialmethod")?;
        for (name, f) in methods {
            ns.set_item(*name, method.call1((self.core.getattr(*f)?,))?)?;
        }
        let property = py.import("builtins")?.getattr("property")?;
        for (name, f) in properties {
            ns.set_item(*name, property.call1((self.core.getattr(*f)?,))?)?;
        }
        Ok(ns)
    }
}
enum DefaultValue {
    Required,
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(&'static str),
    Factory(&'static str),
    Box2d,
    EmptyTuple,
    EmptyBytes,
}
use DefaultValue::*;
struct Field(&'static str, &'static str, DefaultValue);
impl Api<'_> {
    fn record(
        &self,
        name: &str,
        fields: &[Field],
        members: &Bound<'_, PyDict>,
        frozen: bool,
        eq: bool,
        stored: &[&str],
    ) -> PyResult<()> {
        let py = self.py();
        let dc = py.import("dataclasses")?;
        let builtins = py.import("builtins")?;
        let schema = PyList::empty(py);
        for Field(key, ty, default) in fields {
            // Forward annotations refer to public records in this module.
            let annotation = builtins
                .getattr_opt(*ty)?
                .or(self.module.getattr_opt(*ty)?)
                .unwrap_or(ty.into_pyobject(py)?.into_any());
            let kw = PyDict::new(py);
            if stored.contains(key) {
                kw.set_item("init", false)?;
            }
            if key.starts_with('_') {
                kw.set_item("repr", false)?;
            }
            match default {
                Required => (),
                Null => kw.set_item("default", py.None())?,
                Bool(x) => kw.set_item("default", x)?,
                Int(x) => kw.set_item("default", x)?,
                Float(x) => kw.set_item("default", x)?,
                Text(x) => kw.set_item("default", x)?,
                Factory(t) => kw.set_item("default_factory", builtins.getattr(*t)?)?,
                Box2d => kw.set_item("default", (0., 0., 0., 0.))?,
                EmptyTuple => kw.set_item("default", PyTuple::empty(py))?,
                EmptyBytes => kw.set_item("default", pyo3::types::PyBytes::new(py, &[]))?,
            }
            schema.append((*key, annotation, dc.call_method("field", (), Some(&kw))?))?;
        }
        members.set_item("__module__", self.module.name()?)?;
        let args = PyDict::new(py);
        args.set_item("namespace", members)?;
        args.set_item("frozen", frozen)?;
        args.set_item("eq", eq)?;
        let cls = dc.call_method("make_dataclass", (name, schema), Some(&args))?;
        // make_dataclass chooses its caller as the module on Python 3.12+.
        cls.setattr("__module__", self.module.name()?)?;
        self.module.add(name, cls)
    }
}

#[pyfunction]
pub(super) fn register_survey(package: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = package.py();
    let core = py.import("ingetrazo_core")?;
    let modules = py.import("sys")?.getattr("modules")?.cast_into::<PyDict>()?;
    for (name, build) in [
        (
            "geoimport",
            records::geoimport as fn(&Api<'_>) -> PyResult<()>,
        ),
        ("datum", records::datum),
        ("points", records::points),
        ("geopath", records::geopath),
        ("geotiff", records::geotiff),
        ("photomesh", records::photomesh),
        ("profile", records::profile),
        ("terrain", records::terrain),
        ("tiles", providers::tiles),
        ("transport", providers::transport),
        ("tile_fetcher", providers::tile_fetcher),
        ("dem", providers::dem),
        ("geocode", providers::geocode),
    ] {
        let path = format!("georef.{name}");
        if let Some(registered) = modules.get_item(&path)? {
            package.add(name, registered)?;
            continue;
        }
        let module = PyModule::new(py, &path)?;
        let spec = py
            .import("importlib.machinery")?
            .call_method1("ModuleSpec", (&path, py.None()))?;
        module.setattr("__spec__", spec)?;
        module.setattr("__package__", "georef")?;
        modules.set_item(&path, &module)?;
        if let Err(error) = build(&Api {
            core: core.clone(),
            module: module.clone(),
        }) {
            modules.del_item(&path)?;
            return Err(error);
        }
        modules.set_item(format!("ingetrazo_core.{path}"), &module)?;
        package.add(name, module)?;
    }
    Ok(())
}
