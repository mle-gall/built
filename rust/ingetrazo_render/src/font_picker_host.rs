// SPDX-License-Identifier: GPL-3.0-or-later
//! Font picker requests belong to the display that opened them.
use crate::settings_rows::{func, tr};
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
fn view<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    win.py()
        .import("ingetrazo_render.chrome_dialogs")?
        .call_method1("view_of", (win,))
}
#[pyfunction]
#[pyo3(signature=(win,title,text,family,bold,italic,size,unit,done,*,depth=None,editable=false,bounds=(4.,96.),lengths=None))]
fn show(
    win: &Obj,
    title: String,
    text: String,
    family: String,
    bold: bool,
    italic: bool,
    size: f64,
    unit: String,
    done: Py<PyAny>,
    depth: Option<f64>,
    editable: bool,
    bounds: (f64, f64),
    lengths: Option<&Obj>,
) -> PyResult<bool> {
    let py = win.py();
    let view = view(win)?;
    if !view
        .getattr_opt("_ui_attached")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false))
    {
        return Ok(false);
    }
    let settings = py.import("ingetrazo_core.settings")?.call_method0("settings")?;
    let list = |key: &str| -> PyResult<Vec<String>> {
        settings
            .call_method(
                "value",
                (format!("fonts/{key}"), PyList::empty(py)),
                Some(&[("type", py.get_type::<PyList>())].into_py_dict(py)?),
            )?
            .extract()
    };
    let lengths = lengths
        .map(|v| -> PyResult<_> {
            Ok(crate::font_lengths::FontLengths::new(
                v.get_item("unit")?.extract()?,
                v.get_item("text")?.extract()?,
            ))
        })
        .transpose()?;
    let draft = crate::ui::kit::FontChoice {
        title,
        text,
        font: family,
        bold,
        italic,
        height: size,
        unit,
        editable,
        thickness: depth,
        lengths,
        minimum: bounds.0,
        maximum: bounds.1,
        recent: list("recent")?,
        favourites: list("favourites")?,
    };
    let json = serde_json::to_string(&draft)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    view.getattr("renderer")?
        .call_method1("ui_font_picker", (json,))?;
    view.setattr("_font_done", done)?;
    view.call_method0("update")?;
    Ok(true)
}
#[pyfunction]
#[pyo3(signature=(win,payload,source_view=None))]
fn answer(win: &Obj, payload: &str, source_view: Option<&Obj>) -> PyResult<()> {
    let py = win.py();
    let current = view(win)?;
    let source = source_view.unwrap_or(&current);
    if source.is_none() {
        return Ok(());
    }
    let done = source
        .getattr("__dict__")?
        .call_method1("pop", ("_font_done", py.None()))?;
    if !done.is_none() && !payload.is_empty() && current.is(source) {
        let choice = py.import("json")?.call_method1("loads", (payload,))?;
        let settings = py.import("ingetrazo_core.settings")?.call_method0("settings")?;
        for key in ["recent", "favourites"] {
            settings.call_method1("setValue", (format!("fonts/{key}"), choice.get_item(key)?))?;
        }
        done.call1((choice,))?;
    }
    Ok(())
}
#[pyfunction]
fn text3d(win: &Obj, params: &Obj, done: Py<PyAny>) -> PyResult<()> {
    let py = win.py();
    let units = py.import("core.units")?;
    let draft = py
        .import("core.text3d")?
        .call_method1("text_params", (tr(py, "Built")?,))?;
    if !params.is_none() {
        draft.call_method1("update", (params,))?;
    }
    let vp = win.getattr("viewport")?;
    let unit: String = units
        .call_method1("model_units_of", (vp.getattr("scene")?,))?
        .get_item("length")?
        .extract()?;
    let number = |v: f64| {
        units
            .call_method1("length_number", (v, &unit))?
            .extract::<f64>()
    };
    let done = {
        let unit = unit.clone();
        func(py, move |py, args| {
            let p = args.get_item(0)?;
            let units = py.import("core.units")?;
            let metres =
                |k: &str| units.call_method1("metres_from_number", (p.get_item(k)?, &unit));
            let value = py.import("core.text3d")?.call_method1(
                "text_params",
                (
                    p.get_item("text")?.call_method0("strip")?,
                    p.get_item("font")?,
                    p.get_item("bold")?,
                    p.get_item("italic")?,
                    metres("height")?,
                    metres("thickness")?,
                ),
            )?;
            done.call1(py, (value,))
        })?
    };
    let lengths = PyDict::new(py);
    lengths.set_item("unit", &unit)?;
    let prec = units.call_method0("fine_precision")?;
    let shown = ["height", "thickness"]
        .iter()
        .map(|k| {
            let s = units.call_method1("format_length", (draft.get_item(*k)?, &unit, &prec))?;
            units.call_method1("localized", (s,))
        })
        .collect::<PyResult<Vec<_>>>()?;
    lengths.set_item("text", shown)?;
    if !show(
        win,
        tr(py, "3D Text")?,
        draft.get_item("text")?.extract()?,
        draft.get_item("font")?.extract()?,
        draft.get_item("bold")?.extract()?,
        draft.get_item("italic")?.extract()?,
        number(draft.get_item("height")?.extract()?)?,
        units.call_method1("length_suffix", (&unit,))?.extract()?,
        done,
        Some(number(draft.get_item("thickness")?.extract()?)?),
        true,
        (number(0.001)?, number(1000.)?),
        Some(lengths.as_any()),
    )? {
        vp.call_method1(
            "flash_status",
            (tr(py, "3D Text requires the native view.")?, 3000),
        )?;
    }
    Ok(())
}
#[pyfunction]
fn annotation(win: &Obj) -> PyResult<()> {
    let py = win.py();
    let settings = py.import("ingetrazo_render.settings_sheet")?;
    let env = settings.call_method1("Env", (win,))?;
    let style = env
        .getattr("scene")?
        .getattr("dimension_style")?
        .cast_into::<PyDict>()?;
    let font = style
        .get_item("font_family")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or_default();
    let bold = style
        .get_item("font_bold")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(true);
    let italic = style
        .get_item("font_italic")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(false);
    let size = style
        .get_item("font_size")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(9.);
    let env = env.unbind();
    let apply = func(py, move |py, args| {
        let env = env.bind(py);
        let p = args.get_item(0)?;
        let settings = py.import("ingetrazo_render.settings_sheet")?;
        let history = settings.call_method1("document_history", (env.getattr("vp")?,))?;
        let result = py.import("ingetrazo_core.command_protocol")?.call_method1(
            "set_annotation_font",
            (
                history,
                p.get_item("font")?,
                p.get_item("bold")?,
                p.get_item("italic")?,
                p.get_item("height")?.extract::<f64>()?.round_ties_even() as i64,
            ),
        )?;
        if result.get_item("ok")?.is_truthy()? {
            settings.call_method1("_scene_changed", (env,))?;
        }
        Ok(py.None())
    })?;
    show(
        win,
        "Dimension and label font".into(),
        "2.40 m".into(),
        font,
        bold,
        italic,
        size,
        " pt".into(),
        apply,
        None,
        false,
        (6., 28.),
        None,
    )?;
    Ok(())
}
pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let name = "ingetrazo_render.font_picker";
    let m = PyModule::new(py, name)?;
    m.add_function(wrap_pyfunction!(show, &m)?)?;
    m.add_function(wrap_pyfunction!(answer, &m)?)?;
    m.add_function(wrap_pyfunction!(text3d, &m)?)?;
    m.add_function(wrap_pyfunction!(annotation, &m)?)?;
    parent.add("font_picker", &m)?;
    py.import("sys")?.getattr("modules")?.set_item(name, m)
}
