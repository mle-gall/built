// SPDX-License-Identifier: GPL-3.0-or-later
use crate::settings_rows::*;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use serde::{Deserialize, Serialize};
type Obj<'p> = Bound<'p, PyAny>;
const SOURCES: &str = "basemap/custom_sources";
const SELECTED: &str = "basemap/source";
#[derive(Deserialize, Serialize)]
struct Source {
    name: String,
    url: String,
}
impl Source {
    fn id(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "custom-{}",
            py.import("built_core.georef.tiles")?
                .call_method1("source_slug", (&self.name,))?
        ))
    }
}
fn save(st: &Obj<'_>, sources: &[Source]) -> PyResult<()> {
    st.call_method1(
        "setValue",
        (
            SOURCES,
            serde_json::to_string(sources).expect("strings serialize"),
        ),
    )?;
    st.call_method0("sync")?;
    Ok(())
}
fn read(st: &Obj<'_>) -> PyResult<Vec<Source>> {
    let raw = st.call_method1("value", (SOURCES, ""))?;
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&raw.str()?.to_string()).unwrap_or_default();
    let mut rows: Vec<Source> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_value::<Source>(row).ok())
        .filter(|s| !s.name.is_empty() && !s.url.is_empty())
        .collect();
    let legacy = st
        .call_method1("value", ("basemap/custom_url", ""))?
        .str()?
        .to_string();
    if !legacy.is_empty() {
        if !rows.iter().any(|s| s.url == legacy) {
            rows.push(Source {
                name: "XYZ personalizado".into(),
                url: legacy,
            });
            save(st, &rows)?;
        }
        st.call_method1("remove", ("basemap/custom_url",))?;
        st.call_method0("sync")?;
    }
    Ok(rows)
}
fn sources<'p>(st: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let py = st.py();
    let tiles = py.import("built_core.georef.tiles")?;
    let options = tiles.getattr("PRESETS")?.cast_into::<PyDict>()?.copy()?;
    for src in read(st)? {
        let entry = tiles.call_method1("custom_source", (&src.url, 19, &src.name))?;
        options.set_item(entry.getattr("id")?, entry)?;
    }
    Ok(options)
}
pub(crate) fn current<'p>(st: &Obj<'p>) -> PyResult<Obj<'p>> {
    let options = sources(st)?;
    let default = st
        .py()
        .import("built_core.georef.tiles")?
        .getattr("DEFAULT_SOURCE_ID")?;
    let id = st.call_method1("value", (SELECTED, &default))?;
    Ok(options
        .get_item(id)?
        .unwrap_or(options.get_item(&default)?.expect("default source")))
}
fn select(env: &Obj<'_>, id: &str) -> PyResult<()> {
    let st = env.getattr("st")?;
    let options = sources(&st)?;
    let source = options
        .get_item(id)?
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("Unknown tile source"))?;
    st.call_method1("setValue", (SELECTED, id))?;
    st.call_method0("sync")?;
    let scene = env.getattr("scene")?;
    if let Some(layer) = scene.getattr_opt("tile_layer")?.filter(|l| !l.is_none()) {
        layer.setattr("source", source)?;
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
        env.getattr("vp")?.call_method0("reset_tiles")?;
    }
    crate::sheet_runtime::refresh(&env.getattr("win")?)?;
    Ok(())
}
fn source_input(v: &Obj<'_>) -> PyResult<Source> {
    Ok(Source {
        name: v.get_item("name")?.str()?.to_string().trim().into(),
        url: v.get_item("url")?.str()?.to_string().trim().into(),
    })
}
fn add(env: &Obj<'_>) -> PyResult<()> {
    let py = env.py();
    let forms = py.import("built_render.chrome_forms")?;
    let field = forms.getattr("Field")?;
    let fields = PyList::new(
        py,
        [
            field.call1(("name", "edit", tr(py, "Source name:")?, ""))?,
            field.call1(("url", "edit", tr(py, "Tile URL (with {z}/{x}/{y}):")?, ""))?,
        ],
    )?;
    let kw = PyDict::new(py);
    kw.set_item(
        "validate",
        func(py, |py, a| {
            let src = source_input(&a.get_item(0)?.call_method0("values")?)?;
            if src.name.is_empty() {
                obj(py, tr(py, "Enter a source name.")?)
            } else if !["{x}", "{y}", "{z}"].iter().all(|p| src.url.contains(p)) {
                obj(
                    py,
                    tr(py, "The URL must contain the {z}, {x} and {y} placeholders")?,
                )
            } else {
                Ok(none(py))
            }
        })?,
    )?;
    let owner = env.clone().unbind();
    let scene = env.getattr("scene")?.unbind();
    kw.set_item(
        "on_done",
        func(py, move |py, a| {
            if a.get_item(0)?.eq("ok")? {
                let env = owner.bind(py);
                if !env
                    .getattr("win")?
                    .getattr("viewport")?
                    .getattr("scene")?
                    .is(scene.bind(py))
                {
                    return Ok(none(py));
                }
                let st = env.getattr("st")?;
                let src = source_input(&a.get_item(1)?)?;
                let id = src.id(py)?;
                let mut rows = read(&st)?;
                rows.retain(|r| r.id(py).as_deref().ok() != Some(id.as_str()));
                rows.push(src);
                save(&st, &rows)?;
                select(env, &id)?;
            }
            Ok(none(py))
        })?,
    )?;
    let form = forms
        .getattr("Form")?
        .call((tr(py, "New XYZ source")?, fields), Some(&kw))?;
    form.setattr("placement", "drawer")?;
    forms.call_method1("show", (env.getattr("win")?, form))?;
    Ok(())
}
pub(crate) fn append<'p>(
    py: Python<'p>,
    env: &Obj<'p>,
    groups: &Bound<'p, PyList>,
) -> PyResult<()> {
    let st = env.getattr("st")?;
    let opts = PyList::empty(py);
    for (key, src) in sources(&st)? {
        opts.append((key, src.getattr("name")?))?;
    }
    opts.append(("add-source", tr(py, "Add XYZ source…")?))?;
    if current(&st)?
        .getattr("id")?
        .str()?
        .to_str()?
        .starts_with("custom-")
    {
        opts.append(("remove-source", tr(py, "Remove source")?))?;
    }
    let rows = PyList::new(
        py,
        [row(
            py,
            "basemap_source",
            "choice",
            &tr(py, "Base map")?,
            Some(func(py, |_, a| {
                Ok(current(&a.get_item(0)?.getattr("st")?)?
                    .getattr("id")?
                    .unbind())
            })?),
            Some(func(py, |py, a| {
                let env = a.get_item(0)?;
                let id = a.get_item(1)?.extract::<String>()?;
                match id.as_str() {
                    "add-source" => add(&env)?,
                    "remove-source" => {
                        let st = env.getattr("st")?;
                        let old = current(&st)?.getattr("id")?.str()?.to_string();
                        let mut rows = read(&st)?;
                        rows.retain(|r| r.id(py).ok().as_deref() != Some(&old));
                        save(&st, &rows)?;
                        let default = py
                            .import("built_core.georef.tiles")?
                            .getattr("DEFAULT_SOURCE_ID")?
                            .extract::<String>()?;
                        select(&env, &default)?;
                    }
                    _ => select(&env, &id)?,
                }
                Ok(none(py))
            })?),
            &[("options", opts.into_any().unbind())],
        )?],
    )?;
    groups.append(("basemap", tr(py, "Base map")?, rows))?;
    Ok(())
}

#[pyfunction]
pub(crate) fn selected_tile_source(py: Python<'_>) -> PyResult<Obj<'_>> {
    current(&py.import("built_core.settings")?.call_method0("settings")?)
}
