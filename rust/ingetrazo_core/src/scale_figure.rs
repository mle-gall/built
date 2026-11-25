// SPDX-License-Identifier: GPL-3.0-or-later
//! Choose and retain the scale figure for new drawings and recognize it on insert.
use crate::atomic_document::AtomicDocument;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};
use std::{fs, io::Write, path::PathBuf};

const ON: &str = "new_document/scale_figure";
const PICK: &str = "new_document/scale_figure_pick";
const IMAGE: &str = "new_document/scale_figure_image";
const HEIGHT: &str = "new_document/scale_figure_height";
const NAME: &str = "new_document/scale_figure_name";
const DEFAULT: &str = "ingeniero";
const CUSTOM: &str = "custom";
const MARK: &str = "scale_figure";

#[pyclass(frozen, module = "ingetrazo_core.scale_figure")]
struct Figure {
    path: PathBuf,
    #[pyo3(get)]
    height: f64,
    #[pyo3(get)]
    name: String,
}
#[pymethods]
impl Figure {
    #[getter]
    fn image<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyAny>> {
        py.import("pathlib")?.call_method1("Path", (&self.path,))
    }
}
fn people_dir(py: Python<'_>) -> PyResult<PathBuf> {
    let root: PathBuf = py
        .import("core.paths")?
        .call_method0("app_root")?
        .extract()?;
    Ok(root.join("resources/components"))
}
#[pyfunction]
fn people(py: Python<'_>) -> PyResult<Bound<'_, PyList>> {
    let folder = people_dir(py)?;
    let list = PyList::empty(py);
    let Ok(data) = fs::read(folder.join("people.json")) else {
        return Ok(list);
    };
    let Ok(entries) = py
        .import("json")?
        .call_method1("loads", (PyBytes::new(py, &data),))
    else {
        return Ok(list);
    };
    let Ok(entries) = entries.cast::<PyList>() else {
        return Ok(list);
    };
    for entry in entries {
        let Ok(key) = entry.get_item("key").and_then(|v| v.extract::<String>()) else {
            return Ok(PyList::empty(py));
        };
        if folder.join(format!("{key}.png")).is_file() {
            list.append(entry)?;
        }
    }
    Ok(list)
}
fn figures_dir(py: Python<'_>) -> PyResult<PathBuf> {
    let base: PathBuf = py
        .import("core.autosave")?
        .call_method0("_app_data_base")?
        .extract()?;
    Ok(base.join("figures"))
}
#[pyfunction]
fn store_picture(py: Python<'_>, source: PathBuf) -> PyResult<Bound<'_, PyAny>> {
    let data = fs::read(&source)?;
    let bytes = PyBytes::new(py, &data);
    let decoder = py.import("ingetrazo_render")?.getattr("SurveyRaster")?;
    if decoder.call_method1("decode", (&bytes,))?.is_none() {
        // Platform image codecs can recognize formats outside the byte decoder.
        if decoder.call_method1("open", (&source,))?.is_none() || fs::read(&source)? != data {
            return Err(PyValueError::new_err("not an image"));
        }
    }
    let digest: String = py
        .import("hashlib")?
        .call_method1("sha256", (bytes,))?
        .call_method0("hexdigest")?
        .extract()?;
    let ext = source
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("png")
        .to_lowercase();
    let folder = figures_dir(py)?;
    let target = folder.join(format!("{}.{ext}", &digest[..16]));
    if !target.is_file() {
        fs::create_dir_all(&folder)?;
        let mut out = AtomicDocument::create(target.clone())?;
        out.output()?.write_all(&data)?;
        out.commit()?;
    }
    py.import("pathlib")?.call_method1("Path", (target,))
}
fn height(value: &Bound<PyAny>) -> f64 {
    value
        .py()
        .import("builtins")
        .and_then(|m| m.call_method1("float", (value,)))
        .and_then(|v| v.extract::<f64>())
        .unwrap_or(1.70)
        .clamp(0.30, 3.0)
}
#[pyfunction(signature=(settings=None))]
fn chosen(py: Python<'_>, settings: Option<&Bound<PyAny>>) -> PyResult<Option<Figure>> {
    let owned;
    let st = match settings {
        Some(st) => st,
        None => {
            owned = py.import("core.settings")?.call_method0("settings")?;
            &owned
        }
    };
    let setting = |key: &str, default: &str| -> PyResult<String> {
        let value = st.call_method1("value", (key, default))?;
        Ok(if value.is_none() {
            default.into()
        } else {
            value.str()?.to_string()
        })
    };
    if setting(ON, "1")? == "0" {
        return Ok(None);
    }
    let pick = setting(PICK, DEFAULT)?;
    let tr = py.import("core.i18n")?.getattr("tr")?;
    if pick == CUSTOM {
        let path = PathBuf::from(setting(IMAGE, "")?);
        if path.is_file() {
            let name = setting(NAME, "")?;
            let name = match name.trim() {
                "" => tr.call1(("Person",))?.extract()?,
                s => s.to_owned(),
            };
            return Ok(Some(Figure {
                path,
                name,
                height: height(&st.call_method1("value", (HEIGHT,))?),
            }));
        }
    }
    let entries = people(py)?;
    let mut selected = None;
    for entry in entries {
        let key = entry.get_item("key")?.extract::<String>()?;
        if key == pick {
            selected = Some((key, entry));
            break;
        }
        if key == DEFAULT {
            selected = Some((key, entry));
        }
    }
    let Some((key, entry)) = selected else {
        return Ok(None);
    };
    Ok(Some(Figure {
        path: people_dir(py)?.join(format!("{key}.png")),
        height: height(&entry.call_method1("get", ("height",))?),
        name: tr.call1((entry.get_item("name")?,))?.extract()?,
    }))
}
#[pyfunction]
fn mark(group: &Bound<PyAny>) -> PyResult<()> {
    let ext = group.getattr("ext")?;
    let attrs = if ext.is_truthy()? {
        group
            .py()
            .get_type::<PyDict>()
            .call1((ext,))?
            .cast_into::<PyDict>()?
    } else {
        PyDict::new(group.py())
    };
    attrs.set_item(MARK, true)?;
    group.setattr("ext", attrs)
}
#[pyfunction]
fn is_marked(group: &Bound<PyAny>) -> PyResult<bool> {
    let Some(ext) = group.getattr_opt("ext")? else {
        return Ok(false);
    };
    Ok(ext.is_truthy()? && ext.call_method1("get", (MARK,))?.is_truthy()?)
}
pub(crate) fn register(root: &Bound<PyModule>) -> PyResult<()> {
    let py = root.py();
    let name = "ingetrazo_core.scale_figure";
    let m = PyModule::new(py, name)?;
    for (key, value) in [
        ("KEY_ON", ON),
        ("KEY_PICK", PICK),
        ("KEY_IMAGE", IMAGE),
        ("KEY_HEIGHT", HEIGHT),
        ("KEY_NAME", NAME),
        ("DEFAULT_PICK", DEFAULT),
        ("CUSTOM", CUSTOM),
    ] {
        m.add(key, value)?;
    }
    m.add("MAX_HEIGHT", 3.)?;
    m.add("LEGACY_IMAGES", ("sumari.png", "ingeniero.png"))?;
    m.add_class::<Figure>()?;
    for f in [
        wrap_pyfunction!(people, &m)?,
        wrap_pyfunction!(store_picture, &m)?,
        wrap_pyfunction!(chosen, &m)?,
        wrap_pyfunction!(mark, &m)?,
        wrap_pyfunction!(is_marked, &m)?,
    ] {
        m.add_function(f)?;
    }
    py.import("sys")?.getattr("modules")?.set_item(name, &m)?;
    root.add("scale_figure", m)
}
