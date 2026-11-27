//! Settings › Library: My library's folder and the shared libraries
//! (library_host/destinations.rs keeps them in the preferences).
use super::*;
use crate::library_host::destinations as dest;

fn verb<'py>(py: Python<'py>, id: &str, label: &str) -> PyResult<Bound<'py, PyAny>> {
    chrome(py)?.getattr("verb")?.call1((id, tr(py, label)?))
}

fn left<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
    [("align", "left")].into_py_dict(py)
}

/// The pane's blocks.
pub(super) fn pane(c: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = c.py();
    let cfg = dest::config(py)?;
    c.call_method1("caption", (tr(py, "My library")?,))?;
    let default = dest::default_mine().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let (path, which) = match &cfg.mine {
        Some(p) => (p.to_string_lossy().into_owned(), tr(py, "Chosen folder")?),
        None => (default.clone(), tr(py, "Default folder")?),
    };
    c.call_method1("kv", (PyList::new(py, [(which, path)])?,))?;
    c.call_method1("text", (tr(py, "Saved items for every model on this machine. Point it at a synced folder to have them everywhere.")?, "faint"))?;
    let mut buttons = vec![verb(py, "lib_root", "Change folder…")?];
    if cfg.mine.is_some() {
        buttons.push(verb(py, "lib_root_default", "Use the default")?);
    }
    buttons.push(verb(py, "lib_root_reveal", crate::library_view::REVEAL)?);
    c.call_method("buttons", (buttons,), Some(&left(py)?))?;
    c.call_method1("caption", (tr(py, "Shared libraries")?,))?;
    if cfg.shared.is_empty() {
        c.call_method1("text", (tr(py, "None yet. Add a team drive, a NAS or a synced folder: its items show in the Library, and it is offered when saving unless it is read-only.")?, "faint"))?;
    }
    for (i, s) in cfg.shared.iter().enumerate() {
        let state = if s.read_only { tr(py, "Read-only")? } else { tr(py, "Saves allowed")? };
        let missing = if s.path.is_dir() { String::new() } else { format!(" · {}", tr(py, "folder not found")?) };
        c.call_method1("kv", (PyList::new(py, [(s.name.clone(), format!("{} · {state}{missing}", s.path.to_string_lossy()))])?,))?;
        c.call_method("buttons", (vec![verb(py, &format!("lib_edit:{i}"), "Edit…")?, verb(py, &format!("lib_remove:{i}"), "Remove")?],), Some(&left(py)?))?;
    }
    c.call_method("buttons", (vec![verb(py, "lib_add", "Add shared library…")?],), Some(&left(py)?))?;
    Ok(())
}

fn show(win: &Bound<'_, PyAny>, title: &str, fields: Vec<Bound<'_, PyAny>>, ok: &str, done: Py<PyAny>) -> PyResult<()> {
    let py = win.py();
    let forms = cf(py)?;
    let kw = PyDict::new(py);
    kw.set_item("buttons", vec![("cancel", tr(py, "Cancel")?), ("ok", tr(py, ok)?)])?;
    kw.set_item("on_done", done)?;
    let form = forms.getattr("Form")?.call((tr(py, title)?, fields), Some(&kw))?;
    form.setattr("placement", "drawer")?;
    forms.getattr("show")?.call1((win, form))?;
    Ok(())
}

/// A shared library's drawer: name, folder, read-only. `at`: the one edited.
fn shared_form(slf: &Slf<'_>, at: Option<usize>) -> PyResult<()> {
    let py = slf.py();
    let cfg = dest::config(py)?;
    let old = at.and_then(|i| cfg.shared.get(i).cloned());
    let field = cf(py)?.getattr("Field")?;
    let fields = vec![
        field.call(("name", "edit", tr(py, "Name")?, old.as_ref().map_or(String::new(), |s| s.name.clone())), Some(&[("focus", true)].into_py_dict(py)?))?,
        field.call(("folder", "path", tr(py, "Folder")?, old.as_ref().map_or(String::new(), |s| s.path.to_string_lossy().into_owned())), Some(&[("pick", "folder")].into_py_dict(py)?))?,
        field.call1(("read_only", "check", tr(py, "Read-only (never saved to)")?, old.as_ref().is_some_and(|s| s.read_only)))?,
    ];
    let me = slf.clone().unbind();
    let old_name = old.map(|s| s.name);
    let done = func(py, move |py, a| {
        if !a.get_item(0)?.eq("ok")? {
            return Ok(none(py));
        }
        let v = a.get_item(1)?;
        let me = me.bind(py);
        let name: String = v.get_item("name")?.extract()?;
        let folder: String = v.get_item("folder")?.str()?.to_string();
        let read_only = v.get_item("read_only")?.is_truthy()?;
        let set = py.import("built_render.wgpu_library")?.getattr("set_shared")?;
        let kw = [("read_only", read_only.into_pyobject(py)?.to_owned().into_any()), ("old_name", obj(py, old_name.clone())?.into_bound(py))].into_py_dict(py)?;
        match set.call((&name, &folder), Some(&kw)) {
            Ok(_) => note(me, &trf(py, "{name} is a shared library", &[("name", s(py, name.trim()))])?, "ok")?,
            Err(e) => note(me, &e.value(py).str()?.to_string(), "error")?,
        }
        refresh(me)?;
        Ok(none(py))
    })?;
    show(&win(slf)?, if at.is_some() { "Edit shared library" } else { "Add shared library" }, fields, if at.is_some() { "Save" } else { "Add" }, done)
}

/// A Library verb of the pane; false when `arg` is not one.
pub(super) fn act(slf: &Slf<'_>, arg: &str) -> PyResult<bool> {
    let py = slf.py();
    if !arg.starts_with("lib_") {
        return Ok(false);
    }
    let lib = py.import("built_render.wgpu_library")?;
    match arg {
        "lib_root" => {
            let current = dest::mine_root(py)?.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let field = cf(py)?.getattr("Field")?.call(("folder", "path", tr(py, "Folder")?, current), Some(&[("pick", "folder")].into_py_dict(py)?))?;
            let me = slf.clone().unbind();
            let done = func(py, move |py, a| {
                if a.get_item(0)?.eq("ok")? {
                    let me = me.bind(py);
                    let folder: String = a.get_item(1)?.get_item("folder")?.str()?.to_string();
                    match py.import("built_render.wgpu_library")?.call_method1("set_mine", (folder,)) {
                        Ok(_) => note(me, &tr(py, "My library moved: items in the old folder stay there")?, "ok")?,
                        Err(e) => note(me, &e.value(py).str()?.to_string(), "error")?,
                    }
                    refresh(me)?;
                }
                Ok(none(py))
            })?;
            show(&win(slf)?, "My library's folder", vec![field], "Use this folder", done)?;
        }
        "lib_root_default" => {
            lib.call_method1("set_mine", ("",))?;
            refresh(slf)?;
        }
        "lib_root_reveal" => {
            if let Some(root) = dest::mine_root(py)? {
                std::fs::create_dir_all(&root)?;
                py.import("built_core.desktop_open")?.call_method1("reveal", (root.to_string_lossy().as_ref(),))?;
            }
        }
        "lib_add" => shared_form(slf, None)?,
        _ => {
            if let Some(i) = arg.strip_prefix("lib_edit:").and_then(|i| i.parse().ok()) {
                shared_form(slf, Some(i))?;
            } else if let Some(i) = arg.strip_prefix("lib_remove:").and_then(|i| i.parse::<usize>().ok()) {
                if let Some(s) = dest::config(py)?.shared.get(i) {
                    lib.call_method1("remove_shared", (&s.name,))?;
                    note(slf, &trf(py, "{name} is no longer a shared library (its folder is untouched)", &[("name", super::s(py, &s.name))])?, "ok")?;
                }
                refresh(slf)?;
            }
        }
    }
    Ok(true)
}
