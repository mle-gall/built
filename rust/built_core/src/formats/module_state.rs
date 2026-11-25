// SPDX-License-Identifier: GPL-3.0-or-later
use pyo3::exceptions::{PyAttributeError, PyRecursionError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyFrozenSet, PyList, PyTuple};
type Any<'p> = Bound<'p, PyAny>;
type Module<'p> = Bound<'p, PyModule>;

#[pyfunction]
fn _group_ifc_record<'p>(group: &Any<'p>) -> PyResult<Option<Any<'p>>> {
    let py = group.py();
    let Some(record) = group.getattr_opt("ifc")? else {
        return Ok(None);
    };
    if record.is_none() {
        return Ok(None);
    }
    let out = PyDict::new(py);
    if record.is_instance_of::<pyo3::types::PyString>() {
        let name = record.call_method0("strip")?;
        if !name.is_truthy()? {
            return Ok(None);
        }
        out.set_item("class", name)?;
        return Ok(Some(out.into_any()));
    }
    let log = py
        .import("logging")?
        .call_method1("getLogger", ("formats.igz",))?;
    if let Ok(fields) = record.cast::<PyDict>() {
        let kw = PyDict::new(py);
        kw.set_item("allow_nan", false)?;
        let dumps = py.import("json")?.getattr("dumps")?;
        for (key, v) in fields.iter() {
            let one = PyDict::new(py);
            one.set_item(&key, &v)?;
            match dumps.call((one,), Some(&kw)) {
                Ok(_) => out.set_item(key, v)?,
                Err(e)
                    if e.is_instance_of::<PyTypeError>(py)
                        || e.is_instance_of::<PyValueError>(py)
                        || e.is_instance_of::<PyRecursionError>(py) =>
                {
                    log.call_method1(
                        "warning",
                        (
                            "Skipping invalid IFC field %r on group %r",
                            key,
                            group.getattr("name")?,
                        ),
                    )?;
                }
                Err(e) => return Err(e),
            }
        }
        return Ok(Some(out.into_any()));
    }
    log.call_method1(
        "warning",
        (
            "Skipping invalid IFC record on group %r",
            group.getattr("name")?,
        ),
    )?;
    Ok(None)
}
#[pyfunction]
fn _property<'p>(parameter: &Any<'p>) -> PyResult<Any<'p>> {
    let py = parameter.py();
    let value = parameter.call_method1("get", ("value", ""))?;
    let out = PyDict::new(py);
    if parameter.call_method1("get", ("kind",))?.eq("Length")?
        && parameter.call_method1("get", ("units",))?.eq("mm")?
    {
        match py.import("builtins")?.call_method1("float", (&value,)) {
            Ok(n) => {
                let number = n.extract::<f64>()? / 1000.;
                if number.is_finite() {
                    out.set_item("kind", "length")?;
                    out.set_item("value", number)?;
                    return Ok(out.into_any());
                }
            }
            Err(e)
                if e.is_instance_of::<PyValueError>(py) || e.is_instance_of::<PyTypeError>(py) =>
            {
                ()
            }
            Err(e) => return Err(e),
        }
    }
    out.set_item("kind", "text")?;
    out.set_item("value", value.str()?)?;
    Ok(out.into_any())
}
#[pyfunction]
fn _family_record<'p>(
    typ: &Any<'p>,
    family: &Any<'p>,
    source: &Any<'p>,
    parameters: &Any<'p>,
) -> PyResult<Any<'p>> {
    let py = typ.py();
    let record = PyDict::new(py);
    let psets = PyDict::new(py);
    let descriptor = PyDict::new(py);
    let params = PyDict::new(py);
    for (n, src, k) in [
        ("Family", family, "name"),
        ("Category", family, "category"),
        ("Source file", source, "file_name"),
    ] {
        descriptor.set_item(n, src.call_method1("get", (k, ""))?)?;
    }
    for row in parameters.try_iter()? {
        let row = row?;
        params.set_item(row.get_item("name")?, _property(&row)?)?;
    }
    psets.set_item("Revit family", descriptor)?;
    psets.set_item("Revit parameters", params)?;
    record.set_item("v", 1)?;
    record.set_item("class", "IfcBuildingElementProxy")?;
    record.set_item("name", typ.get_item("name")?)?;
    record.set_item("psets", psets)?;
    let class = family.call_method1("get", ("omniclass",))?;
    if class.is_truthy()? {
        let item = PyDict::new(py);
        item.set_item("system", "omniclass")?;
        item.set_item("code", class)?;
        record.set_item("classifications", vec![item])?;
    }
    Ok(record.into_any())
}
fn rfa(m: &Module<'_>, core: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    let hist = core.getattr("history")?;
    let base = hist.getattr("SnapshotImport")?;
    let ns = PyDict::new(py);
    ns.set_item("__module__", "formats.rfa")?;
    ns.set_item(
        "_TRACKED",
        base.getattr("_TRACKED")?.add(((
            "kept_components",
            "added_templates",
            "removed_templates",
        ),))?,
    )?;
    for n in ["do", "undo"] {
        ns.set_item(
            n,
            py.import("functools")?.call_method1(
                "partialmethod",
                (core.getattr(format!("rfa_history_{n}"))?,),
            )?,
        )?;
    }
    m.add(
        "RfaImport",
        py.import("builtins")?
            .call_method1("type", ("RfaImport", (base,), ns))?,
    )?;
    m.add("ZONE_TAG_KEY", "rfa-clearance-zones")?;
    m.add_function(wrap_pyfunction!(_property, m)?)?;
    m.add_function(wrap_pyfunction!(_family_record, m)?)?;
    Ok(())
}
fn igz(m: &Module<'_>, core: &Module<'_>, package: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    m.add(
        "__version__",
        core.getattr("version")?.getattr("__version__")?,
    )?;
    m.add("built", package.getattr("built")?)?;
    m.add("Mesh", core.getattr("mesh")?.getattr("Mesh")?)?;
    for (n, v) in [
        ("PLAIN_FORMAT", 1),
        ("CONTAINER_FORMAT", 2),
        ("GEOMETRY_FORMAT", 3),
        ("CURRENT_FORMAT", 3),
        ("_LARGE_DOCUMENT", 20_000),
        ("_BULK_MIN", 1024),
        ("_NATIVE_JSON_MIN", 32 << 20),
        ("_NATIVE_ZIP_FLAGS", 0x80e),
    ] {
        m.add(n, v)?;
    }
    m.add("_ZIP_MAGIC", PyBytes::new(py, b"PK\x03\x04"))?;
    m.add("_DOC_ENTRY", "document.json")?;
    m.add("_TEX_PREFIX", "textures/")?;
    m.add("_GEOM_PREFIX", "geom/")?;
    m.add("_ZIP_EPOCH", (1980, 1, 1, 0, 0, 0))?;
    m.add(
        "_TEXTURE_WALK_SKIP",
        PyFrozenSet::new(py, ["vertices", "holes", "a", "b", "edges", "arrays"])?,
    )?;
    let kw = PyDict::new(py);
    kw.set_item("sort_keys", true)?;
    kw.set_item("separators", (",", ":"))?;
    let json = py.import("json")?;
    m.add(
        "_attrs_key",
        json.getattr("JSONEncoder")?
            .call((), Some(&kw))?
            .getattr("encode")?,
    )?;
    m.add("_EMPTY", PyList::empty(py))?;
    m.add("_saw_nonfinite", vec![false])?;
    m.add("_dropped_nonfinite", vec![0])?;
    for (n, id, default) in [
        ("_finite_flag", "file_nonfinite", "_saw_nonfinite"),
        ("_repair_count", "file_repairs", "_dropped_nonfinite"),
    ] {
        let kw = PyDict::new(py);
        kw.set_item("default", m.getattr(default)?)?;
        m.add(
            n,
            py.import("contextvars")?
                .getattr("ContextVar")?
                .call((id,), Some(&kw))?,
        )?;
    }
    m.add_function(wrap_pyfunction!(_group_ifc_record, m)?)?;
    Ok(())
}
#[pyfunction(pass_module, name = "__getattr__")]
fn deferred<'p>(m: &Module<'p>, name: &str) -> PyResult<Any<'p>> {
    let py = m.py();
    let v = match (m.name()?.to_str()?, name) {
        ("formats.dwg_bridge", "_VENDOR_BIN" | "_SIBLING_BIN") => {
            let root = py
                .import("built_core.paths")?
                .call_method0("app_root")?;
            if name == "_VENDOR_BIN" {
                root.call_method1("joinpath", ("vendor", "libredwg", "bin"))?
            } else {
                root.getattr("parent")?
                    .getattr("parent")?
                    .call_method1("joinpath", ("ingecad", "vendor", "libredwg", "bin"))?
            }
        }
        ("formats.dwg_bridge", "DwgConverter") => {
            py.import("built_render")?.getattr("DwgConverter")?
        }
        ("formats.skp_reader", "_colorize_image" | "_image_has_cutout" | "_plane_basis") => {
            py.import("core.texture")?.getattr(match name {
                "_colorize_image" => "colorize_image",
                "_image_has_cutout" => "image_has_cutout",
                _ => "projection_basis",
            })?
        }
        _ => return Err(PyAttributeError::new_err(name.to_owned())),
    };
    m.add(name, &v)?;
    Ok(v)
}
pub(super) fn configure(
    name: &str,
    m: &Module<'_>,
    core: &Module<'_>,
    package: &Module<'_>,
) -> PyResult<()> {
    let py = m.py();
    match name {
        "built" => {
            m.add("BUILT_SUFFIX", ".built")?;
            m.add("DOCUMENT_SUFFIXES", (".built", ".igz"))?;
            m.add("DOCUMENT_FILTER", "Documents (*.built *.igz)")?;
            m.add("BUILT_FILTER", "Built document (*.built)")?;
            m.add("WARNING_MM", 0.0254)?;
        }
        "dae" => {
            for (k, v) in [
                ("_MAX_FUSE_LOOPS", 400),
                ("_MAX_PLACEMENTS", 1_000_000),
                ("_INST_MIN_SAVED", 400),
                ("_INST_MIN_POLYS", 60),
                ("_MAX_GROUPS", 250),
                ("_SPLIT_MIN", 4000),
            ] {
                m.add(k, v)?;
            }
        }
        "dwg_bridge" => {
            for n in ["shutil", "sys", "tempfile"] {
                m.add(n, py.import(n)?)?;
            }
            m.add("_TIMEOUT", 300)?;
            m.add_function(wrap_pyfunction!(deferred, m)?)?;
        }
        "dxf_out" => {
            m.add(
                "_LTYPES",
                py.get_type::<PyTuple>()
                    .call1((core.call_method0("sheet_dxf_ltypes")?,))?,
            )?;
        }
        "dxf_views" => m.add("DXF_VIEW_SCALE", 50.)?,
        "fuse" => m.add("SMOOTH_KEY", "_s")?,
        "igz" => igz(m, core, package)?,
        "meshexport" => m.add("_DEFAULT_COLOR", (0.96, 0.95, 0.925))?,
        "obj" => {
            let d = PyDict::new(py);
            for (k, v) in [
                ("m", 1.),
                ("cm", 0.01),
                ("mm", 0.001),
                ("in", 0.0254),
                ("ft", 0.3048),
            ] {
                d.set_item(k, v)?;
            }
            m.add("OBJ_UNITS", d)?;
            m.add("Y_UP_TO_Z_UP", (1., 0., 0., 0., 0., -1., 0., 1., 0.))?;
        }
        "project" => {
            m.add("SUFFIX", ".builtproject")?;
            m.add("LEGACY_SUFFIX", ".igzproj")?;
            m.add("SUFFIXES", (".builtproject", ".igzproj"))?;
        }
        "rfa" => rfa(m, core)?,
        "skp" => m.add("_BACKENDS", vec![core.call_method0("ModelImportBackend")?])?,
        "skp_reader" => {
            m.add("built_core", core)?;
            m.add("_INCH", 0.0254)?;
            m.add("EXACT_TREE", true)?;
            m.add("_GEOMETRY_CACHE_BYTES", 64 << 20)?;
            m.add("_COLORIZE_CACHE_LIMIT", 32 * 1024 * 1024)?;
            m.add(
                "_colorize_cache",
                py.import("collections")?.call_method0("OrderedDict")?,
            )?;
            let kw = PyDict::new(py);
            kw.set_item("default", py.None())?;
            m.add(
                "_geometry_cache",
                py.import("contextvars")?
                    .getattr("ContextVar")?
                    .call(("skp_geometry_cache",), Some(&kw))?,
            )?;
            m.add_function(wrap_pyfunction!(deferred, m)?)?;
        }
        "svg_out" => {
            let pens = PyDict::new(py);
            for (k, v) in [("cut", 0.5), ("profile", 0.35), ("edge", 0.18)] {
                pens.set_item(k, v)?;
            }
            m.add("DEFAULT_PENS", pens)?;
            m.add("MAX_TEXTURE_PX", 1024)?;
            m.add("BAKE_PX_PER_MM", 8.)?;
            m.add("MAX_BAKE_PX", 2048)?;
            m.add("_SEAM_MM", 0.15)?;
        }
        _ => {}
    }
    Ok(())
}
