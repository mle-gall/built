// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin discovery and the published core.extensions scripting module.
use super::path;
use pyo3::{
    exceptions::{PyException, PyImportError, PySystemExit, PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
type Obj<'p> = Bound<'p, PyAny>;
type Candidate = (String, PathBuf, Option<String>);

fn sorted_entries(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn name(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn misplaced(folder: &Path, children: &[PathBuf]) -> Option<String> {
    let visible = |p: &&PathBuf| !name(p).starts_with('.');
    if let Some(file) = children
        .iter()
        .filter(visible)
        .find(|p| p.extension().is_some_and(|e| e == "py") && p.is_file())
    {
        return Some(name(file));
    }
    for child in children.iter().filter(visible) {
        if child.join("__init__.py").is_file() {
            return Some(name(child));
        }
    }
    for child in children.iter().filter(visible) {
        if let Ok(nested) = sorted_entries(child) {
            if let Some(pkg) = nested
                .iter()
                .filter(visible)
                .find(|p| p.join("__init__.py").is_file())
            {
                return Some(
                    pkg.strip_prefix(folder)
                        .ok()?
                        .iter()
                        .map(|s| s.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }
    None
}

fn logger(py: Python) -> PyResult<Obj> {
    py.import("logging")?
        .call_method1("getLogger", ("ingetrazo.plugins",))
}

fn candidates(py: Python, dir: &Path) -> PyResult<Vec<Candidate>> {
    let mut out = Vec::new();
    let entries = match sorted_entries(dir) {
        Ok(entries) => entries,
        Err(e) => {
            logger(py)?.call_method1(
                "warning",
                (format!(
                    "cannot read the plugin folder {}: {e}",
                    dir.display()
                ),),
            )?;
            return Ok(out);
        }
    };
    for entry in entries {
        let id = name(&entry);
        if id.starts_with(['_', '.']) {
            continue;
        }
        if entry.is_dir() {
            let children = match sorted_entries(&entry) {
                Ok(children) => children,
                Err(e) => {
                    out.push((id, entry, Some(PyErr::from(e).to_string())));
                    continue;
                }
            };
            let init = entry.join("__init__.py");
            if init.is_file() {
                if !entry.join(".disabled").is_file() {
                    out.push((id, init, None));
                }
            } else if let Some(inner) = misplaced(&entry, &children) {
                out.push((id,entry,Some(format!("This folder is not an extension: Built needs __init__.py directly inside it. Move {inner} straight into the extensions folder."))));
            }
        } else if entry.extension().is_some_and(|e| e == "py") && entry.is_file() {
            out.push((stem(&entry), entry, None));
        }
    }
    Ok(out)
}

#[pyfunction]
fn installed_candidates(py: Python, directory: PathBuf) -> PyResult<Vec<Candidate>> {
    candidates(py, &directory)
}

fn bundled(py: Python) -> PyResult<PathBuf> {
    Ok(py
        .import("core.paths")?
        .call_method0("app_root")?
        .extract::<PathBuf>()?
        .join("plugins"))
}

#[pyfunction]
fn user_plugins_dir(py: Python) -> PyResult<PathBuf> {
    let (var, suffix) = if cfg!(windows) {
        ("APPDATA", "AppData/Roaming")
    } else {
        ("XDG_DATA_HOME", ".local/share")
    };
    let base = match std::env::var_os(var).filter(|v| !v.is_empty()) {
        Some(value) => PathBuf::from(value),
        None => py
            .import("pathlib")?
            .getattr("Path")?
            .call_method0("home")?
            .extract::<PathBuf>()?
            .join(suffix),
    };
    Ok(base.join("ingetrazo/plugins"))
}

#[pyfunction]
fn plugin_dirs(py: Python) -> PyResult<Vec<PathBuf>> {
    Ok(vec![bundled(py)?, super::root(py)?])
}

#[pyfunction]
#[pyo3(signature=(what, exc_info=None))]
fn note_failure(py: Python, what: &str, exc_info: Option<&Bound<'_, PyTuple>>) -> PyResult<()> {
    let info = match exc_info {
        Some(info) => info.clone(),
        None => py
            .import("sys")?
            .call_method0("exc_info")?
            .cast_into::<PyTuple>()?,
    };
    let traceback = py
        .import("traceback")?
        .getattr("format_exception")?
        .call(info, None)?
        .extract::<Vec<String>>()?
        .concat();
    let stamp: String = py
        .import("time")?
        .call_method1("strftime", ("%Y-%m-%d %H:%M:%S",))?
        .extract()?;
    let root = py
        .import("core.paths")?
        .call_method0("user_log_dir")?
        .extract::<PathBuf>()?;
    if let Ok(mut f) = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(root.join("ingetrazo-errors.log"))
    {
        let _ = writeln!(f, "\n[{stamp}] {what}\n{traceback}");
    }
    Ok(())
}

fn import_file<'p>(py: Python<'p>, id: &str, file: &Path) -> PyResult<Obj<'p>> {
    let util = py.import("importlib.util")?;
    let name = format!("ingetrazo_plugin_{id}");
    let spec = util.call_method1("spec_from_file_location", (&name, file))?;
    let loader = spec
        .getattr_opt("loader")?
        .filter(|v| !v.is_none())
        .ok_or_else(|| {
            PyImportError::new_err(format!(
                "cannot build an import spec for {}",
                file.display()
            ))
        })?;
    let module = util.call_method1("module_from_spec", (&spec,))?;
    let registry = py.import("sys")?.getattr("modules")?;
    registry.set_item(&name, &module)?;
    if let Err(e) = loader.call_method1("exec_module", (&module,)) {
        registry.call_method1("pop", (&name, py.None()))?;
        return Err(e);
    }
    Ok(module)
}

fn builds_bare(inspect: &Obj<'_>, cls: &Obj<'_>) -> PyResult<bool> {
    let py = cls.py();
    let sig = match inspect.call_method1("signature", (cls.getattr("__init__")?,)) {
        Ok(sig) => sig,
        Err(e) if e.is_instance_of::<PyTypeError>(py) || e.is_instance_of::<PyValueError>(py) => {
            return Ok(true)
        }
        Err(e) => return Err(e),
    };
    for p in sig
        .getattr("parameters")?
        .call_method0("values")?
        .try_iter()?
        .skip(1)
    {
        let p = p?;
        if !p.getattr("default")?.is(p.getattr("empty")?) {
            continue;
        }
        let kind = p.getattr("kind")?;
        if !kind.is(p.getattr("VAR_POSITIONAL")?) && !kind.is(p.getattr("VAR_KEYWORD")?) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn load<'p>(
    api: &Obj<'p>,
    id: &str,
    file: &Path,
    panels_only: bool,
    builtins: &Path,
    native_panels: Option<&Obj<'p>>,
) -> PyResult<Option<Obj<'p>>> {
    let py = api.py();
    if file.parent() == Some(builtins) {
        if let Some(native) = native_panels {
            if native.contains(id)? {
                let name = native.get_item(id)?;
                if !name.is_truthy()? {
                    return Ok(None);
                }
                let kw = PyDict::new(py);
                kw.set_item(
                    "panel",
                    py.import(name.cast::<pyo3::types::PyString>()?)?
                        .getattr("create_panel")?,
                )?;
                return Ok(Some(api.call_method(
                    "LoadedPlugin",
                    (id, file),
                    Some(&kw),
                )?));
            }
        }
    }
    let module = import_file(py, id, file)?;
    let panel = module
        .getattr_opt("create_panel")?
        .filter(|v| v.is_callable());
    if panels_only && panel.is_none() {
        return Ok(None);
    }
    let tools = PyList::empty(py);
    if !panels_only {
        let inspect = py.import("inspect")?;
        let tool = py
            .import("tools.base")?
            .getattr("Tool")?
            .cast_into::<pyo3::types::PyType>()?;
        for member in inspect
            .call_method1("getmembers", (&module, inspect.getattr("isclass")?))?
            .try_iter()?
        {
            let cls = member?.get_item(1)?.cast_into::<pyo3::types::PyType>()?;
            if cls.is(&tool)
                || !cls.is_subclass(&tool)?
                || !cls.getattr("__module__")?.eq(module.getattr("__name__")?)?
                || inspect.call_method1("isabstract", (&cls,))?.is_truthy()?
                || !builds_bare(inspect.as_any(), cls.as_any())?
            {
                continue;
            }
            tools.append(cls.call0()?)?;
        }
    }
    let setup = module.getattr_opt("setup")?.filter(|v| v.is_callable());
    if tools.is_empty() && setup.is_none() && panel.is_none() {
        return Ok(None);
    }
    Ok(Some(api.call_method1(
        "LoadedPlugin",
        (id, path(py, file)?, tools, setup, panel),
    )?))
}

#[pyfunction]
#[pyo3(signature=(dirs=None, *, panels_only=false, native_panels=None))]
fn discover_plugins<'p>(
    py: Python<'p>,
    dirs: Option<&Obj<'p>>,
    panels_only: bool,
    native_panels: Option<&Obj<'p>>,
) -> PyResult<(Bound<'p, PyList>, Bound<'p, PyList>)> {
    let api = py.import("core.extensions")?;
    let manager = super::module(py)?;
    let found = PyList::empty(py);
    let errors = PyList::empty(py);
    if manager.call_method0("safe_mode")?.is_truthy()? {
        return Ok((found, errors));
    }
    if native_panels.is_some() {
        let libs = super::root(py)?.with_file_name("plugin-libs");
        let path = py.import("sys")?.getattr("path")?;
        let value = libs.to_string_lossy();
        if libs.is_dir() && !path.contains(value.as_ref())? {
            path.call_method1("append", (value.as_ref(),))?;
        }
    }
    let disabled = manager.call_method0("disabled_overrides")?;
    let dirs = match dirs {
        Some(dirs) => dirs.clone(),
        None => api.call_method0("plugin_dirs")?,
    };
    let mut seen = HashSet::new();
    let bundled = bundled(py)?;
    let log = logger(py)?;
    for dir in dirs.try_iter()? {
        let dir = dir?.extract::<PathBuf>()?;
        if !dir.is_dir() {
            continue;
        }
        for (id, file, problem) in candidates(py, &dir)? {
            if disabled.contains(&id)? {
                continue;
            }
            if !seen.insert(id.clone()) {
                log.call_method1(
                    "warning",
                    (
                        "plugin %r at %s shadowed by an earlier one; skipped",
                        &id,
                        &file,
                    ),
                )?;
                continue;
            }
            let problem = if let Some(reason) = problem {
                log.call_method1("warning", ("plugin %r at %s: %s", &id, &file, &reason))?;
                reason
            } else {
                match load(
                    api.as_any(),
                    &id,
                    &file,
                    panels_only,
                    &bundled,
                    native_panels,
                ) {
                    Ok(plugin) => {
                        if let Some(p) = plugin {
                            found.append(p)?;
                        }
                        continue;
                    }
                    Err(e)
                        if e.is_instance_of::<PyException>(py)
                            || e.is_instance_of::<PySystemExit>(py) =>
                    {
                        let info =
                            (e.get_type(py), e.value(py), e.traceback(py)).into_pyobject(py)?;
                        let what = format!("failed to load plugin {id:?} from {}", file.display());
                        let kw = PyDict::new(py);
                        kw.set_item("exc_info", &info)?;
                        log.call_method("error", (&what,), Some(&kw))?;
                        note_failure(py, &what, Some(&info))?;
                        // Do not use PyErr::print: sys.exit(0) in an extension
                        // must produce a disabled row and let the host keep running.
                        format!("{}: {}", e.get_type(py).name()?, e.value(py).str()?)
                    }
                    Err(e) => return Err(e),
                }
            };
            errors.append(api.call_method1("PluginError", (id, path(py, &file)?, problem))?)?;
        }
    }
    Ok((found, errors))
}

pub(crate) fn install(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let dc = py.import("dataclasses")?;
    let types = py.import("builtins")?;
    let path_type = py.import("pathlib")?.getattr("Path")?;
    for name in ["LoadedPlugin", "PluginError"] {
        let fields = PyList::empty(py);
        fields.append(("stem", types.getattr("str")?))?;
        fields.append(("path", &path_type))?;
        if name == "PluginError" {
            fields.append(("error", types.getattr("str")?))?;
        } else {
            let kw = PyDict::new(py);
            kw.set_item("default_factory", types.getattr("list")?)?;
            fields.append((
                "tools",
                types.getattr("list")?,
                dc.call_method("field", (), Some(&kw))?,
            ))?;
            for key in ["setup", "panel"] {
                fields.append((key, types.getattr("object")?, py.None()))?;
            }
        }
        let cls = dc.call_method1("make_dataclass", (name, &fields))?;
        cls.setattr("__module__", "core.extensions")?;
        m.add(name, cls)?;
    }
    m.add_function(wrap_pyfunction!(user_plugins_dir, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_dirs, m)?)?;
    m.add_function(wrap_pyfunction!(discover_plugins, m)?)?;
    m.add_function(wrap_pyfunction!(installed_candidates, m)?)?;
    m.add_function(wrap_pyfunction!(note_failure, m)?)
}
