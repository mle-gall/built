//! Transfer document ownership between live winit windows without reloading a model.
use super::*;
use pyo3::types::{PyList, PyWeakrefMethods, PyWeakrefReference};
use std::{
    collections::HashMap,
    thread::{self, ThreadId},
};

struct Pending {
    origin: usize,
    kind: String,
    payload: Option<Py<PyAny>>,
}

#[derive(Default)]
#[pyclass]
struct Windows {
    open: Vec<Py<PyWeakrefReference>>,
    front: Option<(ThreadId, Py<PyWeakrefReference>)>,
    next: u64,
    pending: HashMap<String, Pending>,
}

fn registry(py: Python<'_>) -> PyResult<Bound<'_, Windows>> {
    Ok(module(py, "shell_windows")?
        .getattr("_registry")?
        .cast_into()?)
}
pub(super) fn front(py: Python<'_>) -> PyResult<Obj<'_>> {
    Ok(registry(py)?
        .borrow()
        .front
        .as_ref()
        // Window events and drawing belong to the thread that activated the host.
        .filter(|(owner, _)| *owner == thread::current().id())
        .and_then(|(_, w)| w.bind(py).upgrade())
        .unwrap_or_else(|| none(py)))
}
#[pyfunction(name="set_host",signature=(host))]
pub(crate) fn activate(py: Python<'_>, host: Option<&Obj<'_>>) -> PyResult<()> {
    registry(py)?.borrow_mut().front = host
        .filter(|h| !h.is_none())
        .map(PyWeakrefReference::new)
        .transpose()?
        .map(|w| (thread::current().id(), w.unbind()));
    Ok(())
}
#[pyfunction(name = "register")]
fn register_host(host: &Obj<'_>) -> PyResult<()> {
    registry(host.py())?
        .borrow_mut()
        .open
        .push(PyWeakrefReference::new(host)?.unbind());
    Ok(())
}
#[pyfunction]
fn unregister(host: &Obj<'_>) -> PyResult<()> {
    let py = host.py();
    let reg = registry(py)?;
    {
        let mut state = reg.borrow_mut();
        state
            .open
            .retain(|w| w.bind(py).upgrade().is_some_and(|h| !h.is(host)));
        state
            .pending
            .retain(|_, pending| pending.origin != host.as_ptr() as usize);
    }
    if front(py)?.is(host) {
        activate(py, hosts(py)?.last())?;
    }
    Ok(())
}
#[pyfunction]
fn hosts(py: Python<'_>) -> PyResult<Vec<Obj<'_>>> {
    let all = registry(py)?
        .borrow()
        .open
        .iter()
        .filter_map(|w| w.bind(py).upgrade())
        .collect::<Vec<_>>();
    let mut active = Vec::new();
    for h in all {
        if !flag(&h.getattr("window")?, "_closed")? {
            active.push(h);
        }
    }
    Ok(active)
}
#[pyfunction]
fn host_of<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    for h in hosts(win.py())? {
        if h.getattr("window")?.is(win) {
            return Ok(h);
        }
    }
    Ok(none(win.py()))
}
#[pyfunction(signature=(path, exclude=None))]
fn find_document<'p>(path: &Obj<'p>, exclude: Option<&Obj<'p>>) -> PyResult<(Obj<'p>, Obj<'p>)> {
    for h in hosts(path.py())? {
        let win = h.getattr("window")?;
        if exclude.is_some_and(|e| win.is(e)) {
            continue;
        }
        if let Some(doc) = super::project::document(&win, path)? {
            return Ok((h, doc));
        }
    }
    Ok((none(path.py()), none(path.py())))
}
#[pyfunction(signature=(host,kind="new",payload=None))]
fn request(host: &Obj<'_>, kind: &str, payload: Option<Obj<'_>>) -> PyResult<String> {
    let reg = registry(host.py())?;
    let token = {
        let mut state = reg.borrow_mut();
        state.next += 1;
        let token = format!("w{}", state.next);
        state.pending.insert(
            token.clone(),
            Pending {
                origin: host.as_ptr() as usize,
                kind: kind.to_owned(),
                payload: payload.map(Bound::unbind),
            },
        );
        token
    };
    if let Err(e) = host
        .getattr("window_requests")
        .and_then(|q| q.call_method1("append", (&token,)))
        .and_then(|_| host.call_method0("request_frame"))
    {
        reg.borrow_mut().pending.remove(&token);
        return Err(e);
    }
    Ok(token)
}
#[pyfunction]
fn take(py: Python<'_>, token: &str) -> PyResult<Option<(String, Option<Py<PyAny>>)>> {
    Ok(registry(py)?
        .borrow_mut()
        .pending
        .remove(token)
        .map(|p| (p.kind, p.payload)))
}
#[pyfunction]
fn can_move(win: &Obj<'_>) -> PyResult<bool> {
    Ok(!host_of(win)?.is_none()
        && win.getattr("_workspace")?.is_none()
        && win.getattr("_documents")?.len()? > 1)
}
#[pyfunction]
fn move_to_new_window<'p>(win: &Obj<'p>, doc: &Obj<'p>) -> PyResult<Option<String>> {
    if !can_move(win)?
        || !win.getattr("_documents")?.contains(doc)?
        || module(win.py(), "chrome_dialogs")?
            .call_method1("modal", (win,))?
            .is_truthy()?
    {
        return Ok(None);
    }
    Ok(Some(request(
        &host_of(win)?,
        "move",
        Some((win, doc).into_pyobject(win.py())?.into_any()),
    )?))
}
#[pyfunction]
fn transfer(src_win: &Obj<'_>, doc: &Obj<'_>, dst_host: &Obj<'_>) -> PyResult<bool> {
    let py = doc.py();
    let src = src_win.getattr("_documents")?.cast_into::<PyList>()?;
    if !src.contains(doc)? {
        return Ok(false);
    }
    let dst = dst_host.getattr("window")?;
    let docs = dst.getattr("_documents")?.cast_into::<PyList>()?;
    let mut blanks = Vec::new();
    for d in docs.iter() {
        if dst.call_method1("_doc_is_blank", (&d,))?.is_truthy()? {
            blanks.push(d);
        }
    }
    if src_win.getattr("_doc")?.is(doc) {
        if src.len() < 2 {
            return Ok(false);
        }
        let i: usize = src.call_method1("index", (doc,))?.extract()?;
        let next = src.get_item(if i + 1 < src.len() { i + 1 } else { i - 1 })?;
        let kw = PyDict::new(py);
        kw.set_item("closing", true)?;
        src_win.call_method("activate_document", (next,), Some(&kw))?;
    }
    src.call_method1("remove", (doc,))?;
    let vp = doc.getattr("viewport")?;
    let view = doc.getattr("stand_in")?;
    for o in [&vp, &view] {
        if !o.is_none() && o.hasattr("_host")? {
            dst_host.getattr("timers")?.call_method1(
                "transfer_context",
                (src_win.getattr("_host")?.getattr("timers")?, o),
            )?;
            o.setattr("_host", dst_host)?;
        }
    }
    if !view.is_none() {
        view.call_method1("move_timers", (dst_host,))?;
        view.setattr("renderer", py.None())?;
    }
    core(py)?.call_method1("attach_viewport_extensions", (&dst, &vp))?;
    docs.append(doc)?;
    dst.call_method1("activate_document", (doc,))?;
    for d in blanks {
        if !d.is(doc) && docs.contains(&d)? {
            dst.call_method1("_retire_document", (d,))?;
        }
    }
    for w in [src_win, &dst] {
        let _ = w
            .call_method0("_link_watcher")
            .and_then(|watcher| watcher.call_method0("refresh"));
        w.call_method0("_update_title")?;
    }
    module(py, "wgpu_start")?.call_method1("hide", (&dst,))?;
    src_win.getattr("_host")?.call_method0("request_frame")?;
    dst_host.call_method0("request_frame")?;
    Ok(true)
}
#[pyfunction]
fn elsewhere(win: &Obj<'_>) -> PyResult<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    for (n, h) in hosts(win.py())?.into_iter().enumerate() {
        let other = h.getattr("window")?;
        if other.is(win) {
            continue;
        }
        let front = other.getattr("_doc")?;
        let mut label = format!("Window {}", n + 1);
        if !front.is_none() {
            label.push_str(&format!(" · {}", front.call_method0("display_name")?));
        }
        for (i, d) in other.getattr("_documents")?.try_iter()?.enumerate() {
            out.push((
                format!("{}:{i}", h.as_ptr() as usize),
                d?.call_method0("display_name")?.extract()?,
                label.clone(),
            ));
        }
    }
    Ok(out)
}
#[pyfunction]
fn goto(win: &Obj<'_>, ident: &str) -> PyResult<bool> {
    let Some((host, index)) = ident.split_once(':') else {
        return Ok(false);
    };
    let (Ok(id), Ok(index)) = (host.parse::<usize>(), index.parse::<usize>()) else {
        return Ok(false);
    };
    for h in hosts(win.py())? {
        if h.as_ptr() as usize == id {
            let other = h.getattr("window")?;
            let docs = other.getattr("_documents")?;
            if index >= docs.len()? {
                return Ok(false);
            }
            other.call_method1("activate_document", (docs.get_item(index)?,))?;
            h.setattr("focus_requested", true)?;
            h.call_method0("request_frame")?;
            return Ok(true);
        }
    }
    Ok(false)
}
pub(super) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = &create(parent, "shell_windows")?;
    m.add("_registry", Bound::new(m.py(), Windows::default())?)?;
    for f in [
        wrap_pyfunction!(register_host, m)?,
        wrap_pyfunction!(unregister, m)?,
        wrap_pyfunction!(hosts, m)?,
        wrap_pyfunction!(host_of, m)?,
        wrap_pyfunction!(find_document, m)?,
        wrap_pyfunction!(request, m)?,
        wrap_pyfunction!(take, m)?,
        wrap_pyfunction!(can_move, m)?,
        wrap_pyfunction!(move_to_new_window, m)?,
        wrap_pyfunction!(transfer, m)?,
        wrap_pyfunction!(elsewhere, m)?,
        wrap_pyfunction!(goto, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
