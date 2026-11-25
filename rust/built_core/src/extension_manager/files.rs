// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use serde_json::Value;
fn metadata(p: &Path) -> Option<Value> {
    let file = p.join("extension.json");
    if file.is_symlink() {
        return None;
    }
    serde_json::from_slice(&fs::read(file).ok()?).ok()
}
pub(super) fn target(py: Python, id: &str) -> PyResult<(PathBuf, bool)> {
    require_id(id)?;
    let root = root(py)?;
    let dir = root.join(id);
    if dir.symlink_metadata().is_ok() {
        let owned = !dir.is_symlink()
            && dir.is_dir()
            && metadata(&dir)
                .and_then(|v| v["id"].as_str().map(|s| s == id))
                .unwrap_or(false);
        return Ok((dir, owned));
    }
    let flat = root.join(format!("{id}.py"));
    if flat.symlink_metadata().is_ok() {
        return Ok((flat, false));
    }
    Ok((dir, true))
}
#[pyfunction]
fn installed_info<'p>(py: Python<'p>, ident: &str) -> PyResult<Option<Obj<'p>>> {
    let (p, owned) = target(py, ident)?;
    if !owned {
        return Ok(None);
    }
    Ok(metadata(&p)
        .map(|v| py.import("json")?.call_method1("loads", (v.to_string(),)))
        .transpose()?)
}
#[pyfunction]
fn installation_conflict(py: Python, ident: &str) -> PyResult<bool> {
    Ok(!target(py, ident)?.1)
}
#[pyfunction]
fn extension_startup_state(py: Python) -> PyResult<Bound<PyDict>> {
    let result = PyDict::new(py);
    for dir in entries(&root(py)?) {
        if dir.is_symlink() || name(&dir).starts_with('.') || !dir.is_dir() {
            continue;
        }
        if let Some(data) = metadata(&dir) {
            if let Some(id) = data["id"].as_str().filter(|s| valid(s)) {
                let row = PyDict::new(py);
                row.set_item("sha256", data["sha256"].as_str())?;
                row.set_item("enabled", !dir.join(".disabled").is_file())?;
                result.set_item(id, row)?;
            }
        }
    }
    Ok(result)
}
#[pyfunction]
fn is_extension_enabled(py: Python, ident: &str) -> PyResult<bool> {
    let (p, owned) = target(py, ident)?;
    Ok(owned && p.is_dir() && !p.join(".disabled").exists())
}
#[pyfunction]
fn restart_required(ident: &str, startup_state: &Obj) -> PyResult<bool> {
    let py = startup_state.py();
    let old = key(startup_state, ident)?;
    let Some(info) = installed_info(py, ident)? else {
        return Ok(!old.is_none());
    };
    let enabled = is_extension_enabled(py, ident)?;
    let was = !old.is_none() && key(&old, "enabled")?.is_truthy()?;
    Ok(enabled != was
        || enabled && (old.is_none() || !key(&info, "sha256")?.eq(key(&old, "sha256")?)?))
}
#[pyfunction]
fn set_extension_enabled(py: Python, ident: &str, enabled: bool) -> PyResult<()> {
    let _guard = edit_lock(py)?;
    let (p, owned) = target(py, ident)?;
    if !owned || !p.is_dir() {
        return Err(err(format!(
            "'{ident}' is not installed by the extension manager."
        )));
    }
    let marker = p.join(".disabled");
    if enabled {
        clear_file(&marker)?
    } else if !marker.is_file() {
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(marker)?;
    }
    Ok(())
}
#[pyfunction]
fn uninstall_extension(py: Python, ident: &str) -> PyResult<()> {
    let _guard = edit_lock(py)?;
    let (p, owned) = target(py, ident)?;
    if !owned || !p.is_dir() {
        return Err(err(format!(
            "'{ident}' is not an installed extension managed by Built."
        )));
    }
    fs::remove_dir_all(p)?;
    Ok(())
}
#[pyfunction]
fn safe_mode(py: Python) -> PyResult<bool> {
    Ok(root(py)?.join(".safe-mode").is_file())
}
#[pyfunction]
fn set_safe_mode(py: Python, enabled: bool) -> PyResult<()> {
    let dir = root(py)?;
    let p = dir.join(".safe-mode");
    if enabled {
        fs::create_dir_all(dir)?;
        if !p.is_file() {
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(p)?;
        }
    } else {
        clear_file(&p)?
    }
    Ok(())
}
#[pyfunction]
fn disabled_overrides(py: Python) -> PyResult<BTreeSet<String>> {
    let s = py
        .import("built_core.general_settings")?
        .call_method1("value", ("extensions/disabled", "[]"))?;
    let json = s
        .extract::<String>()
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    Ok(json
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect())
}
fn save_overrides(py: Python, ids: &BTreeSet<String>) -> PyResult<()> {
    py.import("built_core.general_settings")?.call_method1(
        "set_value",
        ("extensions/disabled", serde_json::to_string(ids).unwrap()),
    )?;
    Ok(())
}
#[pyfunction]
fn set_override(py: Python, ident: &str, disabled: bool) -> PyResult<()> {
    let mut overrides = disabled_overrides(py)?;
    if disabled {
        overrides.insert(ident.into());
    } else {
        overrides.remove(ident);
    }
    save_overrides(py, &overrides)
}
#[pyfunction]
fn reset_overrides(py: Python) -> PyResult<()> {
    save_overrides(py, &BTreeSet::new())?;
    for (id, _) in extension_startup_state(py)? {
        set_extension_enabled(py, &id.extract::<String>()?, true)?;
    }
    set_safe_mode(py, false)
}
pub(super) fn hand(root: &Path) -> Vec<PathBuf> {
    entries(root)
        .into_iter()
        .filter(|p| {
            visible(p)
                && if p.is_dir() {
                    fs::read_dir(p).is_err()
                        || !p.join("extension.json").is_file() && p.join("__init__.py").is_file()
                } else {
                    p.extension().is_some_and(|v| v == "py")
                }
        })
        .collect()
}
#[pyfunction]
fn hand_installed_plugins(py: Python) -> PyResult<Vec<Obj>> {
    paths(py, hand(&root(py)?))
}
pub(super) fn ident(p: &Path) -> &str {
    if p.extension().is_some_and(|s| s == "py") {
        p.file_stem().and_then(|s| s.to_str()).unwrap_or("")
    } else {
        name(p)
    }
}
#[pyfunction]
fn hand_installed_ident(path: PathBuf) -> String {
    ident(&path).into()
}
pub(super) fn aside(p: &Path) -> PyResult<PathBuf> {
    let dst = p.with_file_name(format!(
        ".{}.manual-{}",
        name(p),
        crate::builtproject::manifest::new_id()
    ));
    fs::rename(p, &dst)?;
    Ok(dst)
}
#[pyfunction]
fn remove_hand_installed<'p>(py: Python<'p>, path: PathBuf) -> PyResult<Obj<'p>> {
    let _guard = edit_lock(py)?;
    if !hand(&root(py)?).contains(&path) {
        return Err(err(format!(
            "'{}' is not a plugin installed by hand.",
            name(&path)
        )));
    }
    super::path(py, &aside(&path)?)
}
pub(super) fn descendants(dir: &Path) -> Vec<PathBuf> {
    let mut result = vec![];
    let mut dirs = vec![dir.to_owned()];
    while let Some(dir) = dirs.pop() {
        for p in entries(&dir) {
            if name(&p) == "__pycache__" || p.is_symlink() {
                continue;
            }
            if p.is_dir() {
                dirs.push(p)
            } else if p.is_file() {
                result.push(p)
            }
        }
    }
    result.sort();
    result
}
fn fingerprint(py: Python, p: &Path) -> PyResult<(Option<String>, HashSet<PathBuf>)> {
    let modules = if p.is_dir() {
        descendants(p)
            .into_iter()
            .filter(|p| name(p) != "__init__.py" && p.extension().is_some_and(|s| s == "py"))
            .map(|m| m.strip_prefix(p).unwrap().into())
            .collect()
    } else {
        HashSet::new()
    };
    let Some(bytes) = fs::read(if p.is_dir() {
        p.join("__init__.py")
    } else {
        p.to_owned()
    })
    .ok() else {
        return Ok((None, modules));
    };
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut bytes = bytes.iter().copied().peekable();
    while let Some(c) = bytes.next() {
        if c == b'\r' {
            if bytes.peek() == Some(&b'\n') {
                bytes.next();
            }
            normalized.push(b'\n')
        } else {
            normalized.push(c)
        }
    }
    Ok((Some(digest(py, &normalized)?), modules))
}
pub(super) fn copies(py: Python, id: &str, package: &Path) -> PyResult<Vec<PathBuf>> {
    require_id(id)?;
    let (hash, modules) = fingerprint(py, package)?;

    let mut found = vec![];
    for p in entries(&root(py)?) {
        if !visible(&p) || p.is_symlink() || ident(&p) == id {
            continue;
        }
        if p.is_dir() {
            if !p.join("__init__.py").is_file() || p.join("extension.json").is_file() {
                continue;
            }
        } else if p.extension().is_none_or(|s| s != "py") {
            continue;
        }
        let (other, mods) = fingerprint(py, &p)?;
        if hash.is_some() && hash == other
            || modules.len() >= 3
                && mods.len() >= 3
                && 5 * modules.intersection(&mods).count() >= 3 * modules.union(&mods).count()
        {
            found.push(p)
        }
    }
    Ok(found)
}
#[pyfunction(signature=(ident,package))]
fn manual_copies<'p>(py: Python<'p>, ident: &str, package: PathBuf) -> PyResult<Vec<Obj<'p>>> {
    paths(py, copies(py, ident, &package)?)
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(installed_info, m)?,
        wrap_pyfunction!(installation_conflict, m)?,
        wrap_pyfunction!(extension_startup_state, m)?,
        wrap_pyfunction!(is_extension_enabled, m)?,
        wrap_pyfunction!(restart_required, m)?,
        wrap_pyfunction!(set_extension_enabled, m)?,
        wrap_pyfunction!(uninstall_extension, m)?,
        wrap_pyfunction!(safe_mode, m)?,
        wrap_pyfunction!(set_safe_mode, m)?,
        wrap_pyfunction!(disabled_overrides, m)?,
        wrap_pyfunction!(set_override, m)?,
        wrap_pyfunction!(reset_overrides, m)?,
        wrap_pyfunction!(hand_installed_plugins, m)?,
        wrap_pyfunction!(hand_installed_ident, m)?,
        wrap_pyfunction!(remove_hand_installed, m)?,
        wrap_pyfunction!(manual_copies, m)?,
    ] {
        m.add_function(f)?
    }
    Ok(())
}
