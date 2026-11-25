//! Bind the native ProjectSession to asynchronous document opens, cameras and history.
use super::*;
use crate::settings_rows::{tr, trf};
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::types::{PyList, PySet};
use std::collections::HashMap;
fn path<'p>(p: &Obj<'p>) -> PyResult<Obj<'p>> {
    p.py().import("pathlib")?.call_method1("Path", (p,))
}
fn dict<'p>(o: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    Ok(o.getattr("__dict__")?.cast_into()?)
}
fn docs<'p>(win: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    Ok(win.getattr("_documents")?.cast_into()?)
}
#[pyfunction]
fn session<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    Ok(dict(win)?
        .get_item("_project")?
        .unwrap_or_else(|| none(win.py())))
}
pub(super) fn document<'p>(win: &Obj<'p>, p: &Obj<'_>) -> PyResult<Option<Obj<'p>>> {
    for d in docs(win)?.iter() {
        if d.call_method1("same_file", (p,))?.is_truthy()? {
            return Ok(Some(d));
        }
    }
    Ok(None)
}
fn touch(win: &Obj<'_>) -> PyResult<()> {
    for d in docs(win)?.iter() {
        let v = attr(&d, "stand_in")?;
        if !v.is_none() {
            v.setattr("_ui_tree_sig", win.py().None())?;
            dict(&v)?.call_method1("pop", ("_ui_rules_cache", win.py().None()))?;
        }
    }
    win.getattr("_host")?.call_method0("request_frame")?;
    Ok(())
}
fn notify(win: &Obj<'_>, title: String, text: String, tone: &str) -> PyResult<()> {
    let kw = PyDict::new(win.py());
    kw.set_item("tone", tone)?;
    module(win.py(), "chrome_dialogs")?.call_method("notify", (win, title, text), Some(&kw))?;
    Ok(())
}
#[pyfunction(signature=(win,path))]
fn open_project(win: &Obj<'_>, path: &Obj<'_>) -> PyResult<bool> {
    let py = win.py();
    let h = win.getattr("_host")?;
    let profile = attr(&h, "settings_dir")?;
    let profile = if profile.is_truthy()? {
        Some(self::path(&profile)?)
    } else {
        None
    };
    let s = match py
        .import("ingetrazo_core")?
        .call_method1("ProjectSession", (self::path(path)?, profile))
    {
        Ok(s) => s,
        Err(e) if e.is_instance_of::<PyOSError>(py) || e.is_instance_of::<PyValueError>(py) => {
            notify(
                win,
                tr(py, "Open project")?,
                e.value(py).str()?.to_string(),
                "error",
            )?;
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    persist(win)?;
    win.setattr("_project", &s)?;
    let file = self::path(&s.getattr("path")?)?;
    win.setattr("_session_path", &file)?;
    let conversion = s.getattr("conversion")?;
    if !conversion.is_none() && conversion.get_item(1)?.eq("converted")? {
        let label = trf(
            py,
            "{old} is now {new}.",
            &[
                (
                    "old",
                    self::path(&conversion.get_item(0)?)?.getattr("name")?,
                ),
                ("new", file.getattr("name")?),
            ],
        )?;
        notify(win, tr(py, "Project converted")?, label, "info")?;
    }
    let kw = PyDict::new(py);
    kw.set_item(
        "replace",
        if conversion.is_truthy()? {
            Some(conversion.get_item(0)?.str()?)
        } else {
            None
        },
    )?;
    py.import("core.recent")?
        .call_method("remember", (s.getattr("path")?,), Some(&kw))?;
    h.call_method0("recent_changed")?;
    win.setattr("_opening_session", true)?;
    for row in s.call_method0("plan")?.try_iter()? {
        let r = row?;
        if r.get_item(4)?.is_truthy()? {
            let camera = r.get_item(2)?;
            let camera = if camera.is_truthy()? {
                py.import("json")?.call_method1("loads", (camera,))?
            } else {
                none(py)
            };
            open_member(
                win,
                &s,
                &self::path(&r.get_item(0)?)?,
                &camera,
                &r.get_item(3)?,
            )?;
        }
    }
    if !s.call_method0("pending")?.is_truthy()? {
        finish(win, &s)?;
    }
    touch(win)?;
    Ok(true)
}
fn open_member(
    win: &Obj<'_>,
    s: &Obj<'_>,
    p: &Obj<'_>,
    camera: &Obj<'_>,
    scene: &Obj<'_>,
) -> PyResult<()> {
    let py = win.py();
    if let Some(d) = document(win, p)? {
        return member_in(win, s, p, &none(py), scene, &d);
    }
    let ok = match module(py, "shell_files")?.call_method1("open_path", (win, p)) {
        Ok(v) => v.is_truthy()?,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => {
            return member_failed(win, s, p, e.value(py).as_any())
        }
        Err(e) => return Err(e),
    };
    let opens = py.import("ingetrazo_render.document_open")?;
    if opens.call_method1("pending", (win, p))?.is_truthy()? {
        let api = module(py, "shell_project")?;
        let kw = PyDict::new(py);
        kw.set_item(
            "failed",
            bind(
                api.getattr("member_failed")?,
                &[win.clone(), s.clone(), p.clone()],
            )?,
        )?;
        let loaded = bind(
            api.getattr("member_in")?,
            &[
                win.clone(),
                s.clone(),
                p.clone(),
                camera.clone(),
                scene.clone(),
            ],
        )?;
        opens.call_method("after_open", (win, p, loaded), Some(&kw))?;
        return Ok(());
    }
    if ok {
        if let Some(d) = document(win, p)? {
            return member_in(win, s, p, camera, scene, &d);
        }
    }
    member_failed(
        win,
        s,
        p,
        tr(py, "could not be opened")?.into_pyobject(py)?.as_any(),
    )
}
#[pyfunction]
fn member_in(
    win: &Obj<'_>,
    s: &Obj<'_>,
    p: &Obj<'_>,
    camera: &Obj<'_>,
    scene: &Obj<'_>,
    doc: &Obj<'_>,
) -> PyResult<()> {
    if !session(win)?.is(s) {
        return Ok(());
    }
    if doc.is(&win.getattr("_doc")?) {
        if camera.is_truthy()? {
            win.call_method1("_apply_camera_dict", (camera,))?;
        }
        let vp = doc.getattr("viewport")?;
        if scene.is_truthy()? && !vp.is_none() {
            for v in vp.getattr("scene")?.getattr("saved_views")?.try_iter()? {
                let v = v?;
                if v.getattr("name")?.eq(scene)? {
                    win.getattr("_scene_tabs")?
                        .call_method1("mark_view", (v,))?;
                    break;
                }
            }
        }
    }
    s.call_method1("loaded", (p,))?;
    settled(win, s)
}
#[pyfunction]
fn member_failed(win: &Obj<'_>, s: &Obj<'_>, p: &Obj<'_>, why: &Obj<'_>) -> PyResult<()> {
    if !session(win)?.is(s) {
        return Ok(());
    }
    s.call_method1("failed", (p, why.str()?))?;
    settled(win, s)
}
fn settled(win: &Obj<'_>, s: &Obj<'_>) -> PyResult<()> {
    refresh(win)?;
    if !s.call_method0("pending")?.is_truthy()? {
        finish(win, s)?
    }
    touch(win)
}
fn finish(win: &Obj<'_>, s: &Obj<'_>) -> PyResult<()> {
    let py = win.py();
    if !dict(win)?
        .call_method1("get", ("_opening_session",))?
        .is_truthy()?
    {
        return Ok(());
    }
    win.setattr("_opening_session", false)?;
    let mut rank = HashMap::new();
    for (i, r) in s.call_method0("plan")?.try_iter()?.enumerate() {
        rank.insert(r?.get_item(0)?.str()?.to_string(), i);
    }
    let docs = docs(win)?;
    let mut ordered = Vec::new();
    for d in docs.iter() {
        let p = d.getattr("path")?;
        let rank = if p.is_truthy()? {
            rank.get(&path(&p)?.call_method0("resolve")?.str()?.to_string())
                .copied()
        } else {
            None
        };
        ordered.push((rank.unwrap_or(usize::MAX), d));
    }
    ordered.sort_by_key(|(r, _)| *r);
    docs.set_slice(
        0,
        docs.len(),
        PyList::new(py, ordered.into_iter().map(|(_, d)| d))?.as_any(),
    )?;
    let editable = s.getattr("editable")?;
    if editable.is_truthy()? {
        if let Some(d) = document(win, &editable)? {
            if !d.is(&win.getattr("_doc")?) {
                win.call_method1("activate_document", (d,))?;
            }
        }
    }
    refresh(win)?;
    let problems = s.call_method0("problems")?;
    let n = problems.len()?;
    if n > 0 {
        let text = if n == 1 {
            problems.get_item(0)?.get_item(1)?.extract()?
        } else {
            trf(
                py,
                "{n} project files did not open",
                &[("n", n.into_pyobject(py)?.into_any())],
            )?
        };
        notify(
            win,
            trf(
                py,
                "Project {name}",
                &[("name", path(&s.getattr("path")?)?.getattr("stem")?)],
            )?,
            text,
            "warning",
        )?;
    }
    persist(win)?;
    win.call_method0("_update_title")?;
    Ok(())
}
#[pyfunction]
fn refresh(win: &Obj<'_>) -> PyResult<()> {
    let s = session(win)?;
    for d in docs(win)?.iter() {
        let vp = d.getattr("viewport")?;
        let p = d.getattr("path")?;
        let ro = !s.is_none()
            && !p.is_none()
            && s.call_method1("is_member", (&p,))?.is_truthy()?
            && s.call_method1("read_only", (&p,))?.is_truthy()?;
        d.setattr("read_only", ro)?;
        if !vp.is_none() {
            vp.getattr("scene")?.setattr("worksession_read_only", ro)?;
        }
    }
    win.call_method0("_update_tab")?;
    Ok(())
}
#[pyfunction]
fn persist(win: &Obj<'_>) -> PyResult<()> {
    let py = win.py();
    let s = session(win)?;
    if s.is_none()
        || flag(win, "_closed")?
        || dict(win)?
            .call_method1("get", ("_opening_session",))?
            .is_truthy()?
    {
        return Ok(());
    }
    let tabs = PyList::empty(py);
    let marked = win.getattr("_scene_tabs")?.call_method0("marked_view")?;
    let front = win.getattr("_doc")?;
    let json = py.import("json")?;
    for d in docs(win)?.iter() {
        let p = d.getattr("path")?;
        let vp = d.getattr("viewport")?;
        if p.is_none() || vp.is_none() {
            continue;
        }
        let camera = json.call_method1("dumps", (win.call_method1("_camera_dict", (vp,))?,))?;
        tabs.append((
            path(&p)?,
            camera,
            if d.is(&front) {
                attr(&marked, "name")?
            } else {
                none(py)
            },
        ))?;
    }
    let view = attr(&front, "stand_in")?;
    let layout = if view.is_none() {
        None
    } else {
        let d = PyDict::new(py);
        d.set_item("lens", dict(&view)?.call_method1("get", ("_ui_lens", 0))?)?;
        Some(json.call_method1("dumps", (d,))?)
    };
    match s.call_method1("record", (tabs, layout)) {
        Ok(_) => Ok(()),
        Err(e) if e.is_instance_of::<PyValueError>(py) => {
            status(win, &e)?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}
fn status(win: &Obj<'_>, e: &PyErr) -> PyResult<()> {
    win.call_method0("statusBar")?
        .call_method1("showMessage", (e.value(win.py()).str()?, 4000))?;
    Ok(())
}
#[pyfunction(signature=(win,path))]
fn set_active_file<'p>(win: &Obj<'p>, path: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = win.py();
    let s = session(win)?;
    if s.is_none() {
        return Err(PyValueError::new_err(tr(py, "Open a project first")?));
    }
    let p = self::path(path)?.call_method0("resolve")?;
    s.call_method1("set_editable", (&p,))?;
    if document(win, &p)?.is_none() {
        open_member(win, &s, &p, &none(py), &none(py))?;
    }
    if let Some(doc) = document(win, &p)? {
        win.call_method1("activate_document", (doc,))?;
    }
    refresh(win)?;
    persist(win)?;
    touch(win)?;
    query(win)
}
#[pyfunction]
fn query<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = win.py();
    let s = session(win)?;
    if !s.is_none() {
        return py
            .import("json")?
            .call_method1("loads", (s.call_method0("query")?,));
    }
    let files = PyList::empty(py);
    for d in docs(win)?.iter() {
        let p = d.getattr("path")?;
        if p.is_truthy()? {
            let row = PyDict::new(py);
            row.set_item("path", path(&p)?.call_method0("resolve")?.str()?)?;
            row.set_item("role", "host")?;
            row.set_item("read_only", false)?;
            files.append(row)?;
        }
    }
    let out = PyDict::new(py);
    out.set_item("project", py.None())?;
    out.set_item("active_file", py.None())?;
    out.set_item("files", files)?;
    Ok(out.into_any())
}
#[pyfunction]
fn closed(win: &Obj<'_>, doc: &Obj<'_>) -> PyResult<()> {
    let s = session(win)?;
    let p = doc.getattr("path")?;
    if !s.is_none() && !p.is_none() {
        s.call_method1("closed", (path(&p)?,))?;
        touch(win)?;
    }
    Ok(())
}
#[pyfunction(signature=(win,doc,path))]
fn before_save(win: &Obj<'_>, doc: &Obj<'_>, path: &Obj<'_>) -> PyResult<()> {
    let s = session(win)?;
    let v = doc.getattr("viewport")?;
    if s.is_none() || v.is_none() {
        return Ok(());
    }
    let api = win.py().import("core.project")?;
    let scene = v.getattr("scene")?;
    if api.call_method1("path_for", (&scene,))?.is_none() {
        api.call_method1("attach", (&scene, s.getattr("path")?, path))?;
    }
    Ok(())
}
#[pyfunction]
fn saved(win: &Obj<'_>, doc: &Obj<'_>) -> PyResult<()> {
    let s = session(win)?;
    let p = doc.getattr("path")?;
    if s.is_none() || p.is_none() {
        return Ok(());
    }
    let joined = !s.call_method1("is_member", (&p,))?.is_truthy()?;
    let p = path(&p)?;
    let update = (|| -> PyResult<()> {
        s.call_method0("reload")?;
        s.call_method1("loaded", (&p,))?;
        if joined || s.getattr("editable")?.is_none() {
            s.call_method1("set_editable", (&p,))?;
        }
        Ok(())
    })();
    match update {
        Err(e) if e.is_instance_of::<PyValueError>(win.py()) => return Ok(()),
        other => other?,
    }
    refresh(win)?;
    persist(win)?;
    touch(win)
}
#[pyfunction]
fn tree_rows<'p>(win: &Obj<'p>, query_text: &str, expanded: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = win.py();
    let s = session(win)?;
    if s.is_none() {
        Ok(PyList::empty(py).into_any())
    } else {
        s.call_method1(
            "tree_rows",
            (query_text, py.get_type::<PySet>().call1((expanded,))?),
        )
    }
}
#[pyfunction(signature=(win,verb,path))]
fn tree_action(win: &Obj<'_>, verb: &str, path: &Obj<'_>) -> PyResult<bool> {
    let py = win.py();
    let s = session(win)?;
    if s.is_none() || !path.is_truthy()? {
        return Ok(false);
    }
    match verb {
        "open" => {
            if let Some(d) = document(win, path)? {
                win.call_method1("activate_document", (d,))?;
            } else {
                let p = self::path(path)?;
                if p.call_method0("is_file")?.is_truthy()? {
                    open_member(win, &s, &p, &none(py), &none(py))?;
                }
            }
        }
        "edit" => {
            if let Err(e) = set_active_file(win, path) {
                if !e.is_instance_of::<PyValueError>(py) {
                    return Err(e);
                }
                status(win, &e)?;
            }
        }
        "locate" => {
            let p = self::path(path)?;
            let ext = p.getattr("suffix")?.extract::<String>()?;
            let filter = format!("{} (*{ext})", ext.trim_start_matches('.'));
            let label = trf(py, "Locate {name}", &[("name", p.getattr("name")?)])?;
            let found = py
                .import("ingetrazo_render.shell_files")?
                .getattr("ShellFileDialog")?
                .call_method1("getOpenFileName", (win, label, "", filter))?
                .get_item(0)?;
            if found.is_truthy()? {
                locate(win, path, &found)?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
#[pyfunction]
fn locate(win: &Obj<'_>, old: &Obj<'_>, new: &Obj<'_>) -> PyResult<()> {
    let py = win.py();
    let s = session(win)?;
    let p = path(new)?;
    py.import("ingetrazo_core")?.call_method1(
        "project_relocate",
        (path(&s.getattr("path")?)?, path(old)?, &p),
    )?;
    s.call_method0("reload")?;
    if matches!(
        p.getattr("suffix")?
            .extract::<String>()?
            .to_lowercase()
            .as_str(),
        ".built" | ".igz"
    ) {
        open_member(win, &s, &p.call_method0("resolve")?, &none(py), &none(py))?;
    }
    persist(win)?;
    touch(win)
}
#[pyfunction]
fn problems<'p>(win: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    let py = win.py();
    let s = session(win)?;
    let rows = PyList::empty(py);
    if !s.is_none() {
        for p in s.call_method0("problems")?.try_iter()? {
            let p = p?;
            rows.append((
                "Needs attention",
                format!("project:{}", p.get_item(0)?),
                p.get_item(1)?,
                PyList::empty(py),
            ))?;
        }
    }
    Ok(rows)
}
pub(super) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = &create(parent, "shell_project")?;
    for f in [
        wrap_pyfunction!(session, m)?,
        wrap_pyfunction!(open_project, m)?,
        wrap_pyfunction!(member_in, m)?,
        wrap_pyfunction!(member_failed, m)?,
        wrap_pyfunction!(refresh, m)?,
        wrap_pyfunction!(persist, m)?,
        wrap_pyfunction!(set_active_file, m)?,
        wrap_pyfunction!(query, m)?,
        wrap_pyfunction!(closed, m)?,
        wrap_pyfunction!(before_save, m)?,
        wrap_pyfunction!(saved, m)?,
        wrap_pyfunction!(tree_rows, m)?,
        wrap_pyfunction!(tree_action, m)?,
        wrap_pyfunction!(locate, m)?,
        wrap_pyfunction!(problems, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
