use super::*;
use pyo3::types::{IntoPyDict, PyList, PySet};

pub(super) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("IN_M", 0.0254)?;
    m.add("FT_M", 0.3048)?;
    m.add(
        "UNIT_CHOICES",
        (
            "m",
            "cm",
            "mm",
            "in",
            "ft",
            "ft-in",
            "in-frac",
            "ft-in-frac",
        ),
    )?;
    m.add(
        "_FRAC_DENOMS",
        [(0, 1), (1, 4), (2, 16), (3, 32), (4, 64)].into_py_dict(py)?,
    )?;
    m.add(
        "BASE_UNIT",
        [
            ("length", "m"),
            ("area", "m2"),
            ("volume", "m3"),
            ("angle", "rad"),
        ]
        .into_py_dict(py)?,
    )?;
    let defaults = PyDict::new(py);
    defaults.set_item("length", "m")?;
    defaults.set_item("precision", 2)?;
    m.add("DEFAULT_MODEL_UNITS", defaults)?;
    m.add(
        "UNIT_LABELS",
        [
            ("m", "Metres (m)"),
            ("cm", "Centimetres (cm)"),
            ("mm", "Millimetres (mm)"),
            ("in", "Decimal inches (in)"),
            ("ft", "Decimal feet (ft)"),
            ("ft-in", "Feet and inches (1'6\")"),
            ("in-frac", "Fractional inches"),
            ("ft-in-frac", "Fractional feet and inches"),
        ]
        .into_py_dict(py)?,
    )?;
    m.add(
        "_BARE_SCALE",
        [
            ("m", 1.0),
            ("cm", 0.01),
            ("mm", 0.001),
            ("in", 0.0254),
            ("ft", 0.3048),
            ("ft-in", 0.0254),
            ("in-frac", 0.0254),
            ("ft-in-frac", 0.0254),
        ]
        .into_py_dict(py)?,
    )?;
    m.add("NEW_DOCUMENT_KEY", "units/new_document")?;
    m.add(
        "KIND_UNIT_NAMES",
        [
            ("mm2", "Square millimetres"),
            ("cm2", "Square centimetres"),
            ("m2", "Square metres"),
            ("ha", "Hectares"),
            ("in2", "Square inches"),
            ("ft2", "Square feet"),
            ("yd2", "Square yards"),
            ("acre", "Acres"),
            ("cm3", "Cubic centimetres"),
            ("l", "Litres"),
            ("m3", "Cubic metres"),
            ("in3", "Cubic inches"),
            ("ft3", "Cubic feet"),
            ("yd3", "Cubic yards"),
            ("gal", "US gallons"),
            ("deg", "Degrees"),
            ("rad", "Radians"),
            ("grad", "Gradians"),
        ]
        .into_py_dict(py)?,
    )?;
    m.add(
        "AREA_CHOICES",
        ("mm2", "cm2", "m2", "ha", "in2", "ft2", "yd2", "acre"),
    )?;
    m.add(
        "VOLUME_CHOICES",
        ("cm3", "l", "m3", "in3", "ft3", "yd3", "gal"),
    )?;
    m.add("ANGLE_CHOICES", ("deg", "rad", "grad"))?;
    m.add(
        "FRACTION_LABELS",
        ("1\"", "1/4\"", "1/16\"", "1/32\"", "1/64\""),
    )?;
    m.add(
        "PROFILE_LABELS",
        [
            ("metric", "Metric (m)"),
            ("centimetric", "Centimetric (cm)"),
            ("millimetric", "Millimetric (mm)"),
            ("imperial", "Imperial (ft-in)"),
            ("custom", "Custom"),
        ]
        .into_py_dict(py)?,
    )?;
    m.add("MAX_TYPED", 1000000000.0)?;
    m.add(
        "_IMPERIAL_FORMS",
        PySet::new(
            py,
            ["ft", "ft-in", "ft-in-frac", "in", "in-frac", "mi", "yd"],
        )?,
    )?;
    m.add(
        "_EXPECTED",
        [
            ("length", "Expected a length"),
            ("area", "Expected an area"),
            ("volume", "Expected a volume"),
            ("angle", "Expected an angle"),
        ]
        .into_py_dict(py)?,
    )?;

    m.add("_SCENE", py.None())?;
    m.add("_UNITLESS", false)?;
    let dc = py.import("dataclasses")?;
    let field = |default: Obj<'_>| -> PyResult<_> {
        let kw = PyDict::new(py);
        kw.set_item("default", default)?;
        dc.call_method("field", (), Some(&kw))
    };
    let options = PyDict::new(py);
    options.set_item("frozen", true)?;
    let unit_fields = PyList::empty(py);
    for (name, annotation) in [
        ("code", "str"),
        ("kind", "str"),
        ("factor", "float"),
        ("symbol", "str"),
    ] {
        unit_fields.append((name, annotation))?;
    }
    unit_fields.append(("aliases", "tuple", field(PyTuple::empty(py).into_any())?))?;
    let class = dc.call_method("make_dataclass", ("Unit", unit_fields), Some(&options))?;
    class.setattr("__module__", "core.units")?;
    m.add("Unit", &class)?;
    let rows = PyDict::new(py);
    let aliases = PyDict::new(py);
    for (code, kind, factor, symbol, words) in parser::units_table() {
        let row = class.call1((code, kind, factor, symbol, PyTuple::new(py, &words)?))?;
        rows.set_item(code, &row)?;
        for word in words {
            aliases.set_item(word.to_lowercase(), &row)?;
        }
    }
    m.add("UNITS", rows)?;
    m.add("_ALIASES", aliases)?;
    let fields = PyList::empty(py);
    fields.append(("value", "float | None", field(py.None().into_bound(py))?))?;
    for (name, ty, default) in [
        ("error", "str", "".into_pyobject(py)?.into_any()),
        ("position", "int", 0_i64.into_pyobject(py)?.into_any()),
        ("word", "str", "".into_pyobject(py)?.into_any()),
        (
            "explicit_measurement",
            "bool",
            false.into_pyobject(py)?.to_owned().into_any(),
        ),
    ] {
        fields.append((name, ty, field(default)?))?;
    }
    let parsed = dc.call_method("make_dataclass", ("Parsed", fields), Some(&options))?;
    parsed.setattr("__module__", "core.units")?;
    parsed.setattr(
        "ok",
        py.import("builtins")?
            .call_method1("property", (wrap_pyfunction!(parsed_ok, m)?,))?,
    )?;
    parsed.setattr(
        "message",
        py.import("functools")?
            .call_method1("partialmethod", (wrap_pyfunction!(message, m)?,))?,
    )?;
    m.add("Parsed", parsed)?;
    let profiles = PyDict::new(py);
    for (name, length, precision) in format::py_profiles() {
        let d = PyDict::new(py);
        d.set_item("length", length)?;
        d.set_item("precision", precision)?;
        profiles.set_item(name, d)?;
    }
    m.add("PROFILES", profiles)?;
    m.add(
        "VULGAR_FRACTIONS",
        parser::vulgar_fractions().into_py_dict(py)?,
    )?;
    Ok(())
}
