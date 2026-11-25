//! Platform dialogs and pasteboard. The platform module is built into the launcher.
use super::*;
use pyo3::exceptions::PyModuleNotFoundError;
use pyo3::types::{PyBytes, PyList, PyString};
pub(crate) fn platform(py: Python<'_>) -> PyResult<Option<Bound<'_, PyModule>>> {
    match py.import("built_shell") {
        Ok(m) => Ok(Some(m)),
        Err(e)
            if e.is_instance_of::<PyModuleNotFoundError>(py)
                && e.value(py).getattr("name")?.eq("built_shell")? =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn available(py: Python<'_>) -> PyResult<bool> {
    Ok(platform(py)?.is_some())
}
type Filters = Vec<(String, Vec<String>)>;
fn filters(value: Option<&Obj<'_>>) -> PyResult<Filters> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    if value.is_instance_of::<PyString>() {
        let text = value.extract::<String>()?;
        let mut filters = Vec::new();
        for group in text.split(";;") {
            let Some((title, patterns)) = group.split_once('(') else {
                continue;
            };
            let extensions = patterns
                .trim_end_matches(')')
                .split_whitespace()
                .map(|p| p.trim_start_matches(['*', '.']))
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if !extensions.is_empty() {
                filters.push((
                    if title.trim().is_empty() {
                        "Files"
                    } else {
                        title.trim()
                    }
                    .to_owned(),
                    extensions,
                ));
            }
        }
        Ok(filters)
    } else {
        let mut filters: Filters = value.extract()?;
        for (_, exts) in &mut filters {
            for ext in exts {
                *ext = ext.trim_start_matches(['*', '.']).to_owned();
            }
        }
        Ok(filters)
    }
}
#[pyfunction(signature=(title,filters=None,directory=None,multiple=false))]
fn open_files<'p>(
    py: Python<'p>,
    title: &str,
    filters: Option<&Obj<'p>>,
    directory: Option<&str>,
    multiple: bool,
) -> PyResult<Obj<'p>> {
    match platform(py)? {
        Some(m) => m.call_method1(
            "open_files",
            (
                title,
                self::filters(filters)?,
                directory.filter(|p| !p.is_empty()),
                multiple,
            ),
        ),
        None => Ok(PyList::empty(py).into_any()),
    }
}
#[pyfunction(signature=(title,name="",filters=None,directory=None))]
fn save_file<'p>(
    py: Python<'p>,
    title: &str,
    name: &str,
    filters: Option<&Obj<'p>>,
    directory: Option<&str>,
) -> PyResult<Obj<'p>> {
    match platform(py)? {
        Some(m) => m.call_method1(
            "save_file",
            (
                title,
                name,
                self::filters(filters)?,
                directory.filter(|p| !p.is_empty()),
            ),
        ),
        None => Ok(none(py)),
    }
}
#[pyfunction(signature=(title,directory=None))]
fn pick_folder<'p>(py: Python<'p>, title: &str, directory: Option<&str>) -> PyResult<Obj<'p>> {
    match platform(py)? {
        Some(m) => m.call_method1("pick_folder", (title, directory.filter(|p| !p.is_empty()))),
        None => Ok(none(py)),
    }
}
#[pyfunction(signature=(title,text,buttons,tone="info"))]
fn alert(
    py: Python<'_>,
    title: &str,
    text: &str,
    buttons: Vec<String>,
    tone: &str,
) -> PyResult<usize> {
    match platform(py)? {
        Some(m) => m
            .call_method1("alert", (title, text, buttons, tone))?
            .extract(),
        None => Ok(buttons.len().saturating_sub(1)),
    }
}
#[pyfunction(signature=(view,timeout_ms=16.))]
fn pump(py: Python<'_>, view: usize, timeout_ms: f64) -> PyResult<Obj<'_>> {
    if view != 0 {
        if let Some(m) = platform(py)? {
            return m.call_method1("pump", (view, timeout_ms));
        }
    }
    Ok(PyList::empty(py).into_any())
}
#[pyfunction(signature=(view,key_code,chars,command=false))]
fn post_key(
    py: Python<'_>,
    view: usize,
    key_code: u16,
    chars: &str,
    command: bool,
) -> PyResult<bool> {
    if view != 0 {
        if let Some(m) = platform(py)? {
            return m
                .call_method1("post_key", (view, key_code, chars, command))?
                .extract();
        }
    }
    Ok(false)
}
#[pyfunction]
fn pasteboard_read<'p>(py: Python<'p>, kind: &str) -> PyResult<Obj<'p>> {
    let Some(m) = platform(py)? else {
        return Ok(none(py));
    };
    let data = m.call_method1("pasteboard_read", (kind,))?;
    if data.is_none() {
        Ok(data)
    } else {
        py.get_type::<PyBytes>().call1((data,))
    }
}
#[pyfunction]
fn pasteboard_write(py: Python<'_>, items: &Obj<'_>) -> PyResult<bool> {
    let Some(m) = platform(py)? else {
        return Ok(false);
    };
    let buffer = PyList::empty(py);
    for row in items.try_iter()? {
        let row = row?;
        buffer.append((
            row.get_item(0)?,
            py.get_type::<PyBytes>().call1((row.get_item(1)?,))?,
        ))?;
    }
    m.call_method1("pasteboard_write", (buffer,))?.extract()
}
#[pyfunction]
fn pasteboard_types(py: Python<'_>) -> PyResult<(Vec<String>, isize)> {
    match platform(py)? {
        Some(m) => m.call_method0("pasteboard_types")?.extract(),
        None => Ok((vec![], 0)),
    }
}
pub(super) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = &create(parent, "shell_services")?;
    for f in [
        wrap_pyfunction!(available, m)?,
        wrap_pyfunction!(open_files, m)?,
        wrap_pyfunction!(save_file, m)?,
        wrap_pyfunction!(pick_folder, m)?,
        wrap_pyfunction!(alert, m)?,
        wrap_pyfunction!(pump, m)?,
        wrap_pyfunction!(post_key, m)?,
        wrap_pyfunction!(pasteboard_read, m)?,
        wrap_pyfunction!(pasteboard_write, m)?,
        wrap_pyfunction!(pasteboard_types, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
